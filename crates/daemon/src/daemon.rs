use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tgow_common::config::{
    Action, CaptureOptions, Config, NotificationSpeed, ReplayAutostart, StreamService, YesNoGames,
};
use tgow_common::i18n::{self, duration_text};
use tgow_common::ipc::recorder::{Command as RecCommand, Output};
use tgow_common::ipc::{self, Event, ModeStatus, Request, Status, Stream, ToastKind};
use tgow_common::{paths, system, tr, trf};

use crate::args::{self, Source};
use crate::platform::hotkeys::Hotkeys;
use crate::platform::notify::Notifier;
use crate::platform::tray::Tray;
use crate::platform::{controller, desktop};
use crate::state::TrayState;
use crate::{games, server, update};

static TERMINATE: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_signals() {
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
}

#[cfg(windows)]
fn install_signals() {}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Record,
    Replay,
    Stream,
    Screenshot,
}

pub enum Msg {
    Request(Request, Sender<Event>),
    Subscribe(Stream),
    Action(Action),
    Recorder {
        id: u64,
        output: Output,
    },
    RecorderClosed {
        id: u64,
    },
    Region {
        purpose: Purpose,
        region: Option<String>,
    },
    Kill {
        id: u64,
    },
    Game(Option<String>),
    Update(update::Found),
    #[cfg(windows)]
    Notice(String),
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Replay,
    Record,
    Stream,
}

impl Mode {
    fn kind(self) -> ToastKind {
        match self {
            Mode::Replay => ToastKind::Replay,
            Mode::Record => ToastKind::Record,
            Mode::Stream => ToastKind::Stream,
        }
    }
}

struct Session {
    id: u64,
    child: Child,
    stdin: Option<ChildStdin>,
    source: String,
    started: Instant,
    paused_total: Duration,
    paused_since: Option<Instant>,
    running: bool,
    stopping: bool,
    saved: bool,
}

impl Session {
    fn elapsed(&self) -> f64 {
        let mut paused = self.paused_total;
        if let Some(since) = self.paused_since {
            paused += since.elapsed();
        }
        self.started.elapsed().saturating_sub(paused).as_secs_f64()
    }

    fn send(&mut self, cmd: &RecCommand) {
        if let Some(stdin) = self.stdin.as_mut() {
            if stdin.write_all(cmd.line().as_bytes()).and_then(|_| stdin.flush()).is_err() {
                self.stdin = None;
            }
        }
    }
}

struct Shot {
    id: u64,
    child: Child,
    path: PathBuf,
    source: String,
    to_disk: bool,
}

struct Countdown {
    remaining: u32,
    next: Instant,
    source: Source,
}

pub struct Daemon {
    tx: Sender<Msg>,
    cfg: Config,
    replay: Option<Session>,
    record: Option<Session>,
    stream: Option<Session>,
    shot: Option<Shot>,
    subscribers: Vec<Stream>,
    overlay: Option<Child>,
    notifier: Notifier,
    tray: Tray,
    hotkeys: Hotkeys,
    controller: Arc<AtomicBool>,
    next_id: u64,
    mic_muted: bool,
    countdown: Option<Countdown>,
    region_pending: bool,
    last_saved: Option<String>,
    free_space: Option<u64>,
    low_disk_warned: bool,
    last_disk_check: Instant,
    last_tick: Instant,
    last_status: Option<Status>,
    gpu_vendor: String,
    wayland: bool,
    game: Option<String>,
    replay_for_game: bool,
    updates: Arc<AtomicBool>,
    update_ready: Option<update::Ready>,
    update_noticed: Option<String>,
    apply_update: Option<update::Ready>,
    quit: bool,
}

pub fn run(show: bool) -> anyhow::Result<()> {
    if ipc::daemon_running() {
        let _ = ipc::send(&Request::Run { action: Action::ShowHide });
        return Ok(());
    }
    let mut cfg = Config::load();
    if cfg.main.auto_update {
        if let Some(ready) = update::pending() {
            if update::apply(&ready) {
                return Ok(());
            }
        }
    }
    let listener = server::bind()?;
    trim_log();
    install_signals();
    let (tx, rx) = mpsc::channel();
    server::serve(listener, tx.clone());
    let controller = Arc::new(AtomicBool::new(false));
    controller::spawn(controller.clone(), tx.clone());

    let autostart = desktop::autostart_enabled();
    if autostart != cfg.main.start_on_login {
        cfg.main.start_on_login = autostart;
        let _ = cfg.save();
    }
    let mut daemon = Daemon {
        tx: tx.clone(),
        cfg: cfg.clone(),
        replay: None,
        record: None,
        stream: None,
        shot: None,
        subscribers: Vec::new(),
        overlay: None,
        notifier: Notifier::start(),
        tray: Tray::new(),
        hotkeys: Hotkeys::new(),
        controller,
        next_id: 1,
        mic_muted: false,
        countdown: None,
        region_pending: false,
        last_saved: None,
        free_space: None,
        low_disk_warned: false,
        last_disk_check: Instant::now() - Duration::from_secs(60),
        last_tick: Instant::now(),
        last_status: None,
        gpu_vendor: system::gpu_vendor(),
        wayland: system::is_wayland(),
        game: None,
        replay_for_game: false,
        updates: Arc::new(AtomicBool::new(cfg.main.auto_update)),
        update_ready: None,
        update_noticed: None,
        apply_update: None,
        quit: false,
    };
    games::spawn(tx.clone());
    update::spawn(tx.clone(), daemon.updates.clone());
    daemon.apply_config(None);
    if let Some(version) = update::take_finished() {
        let text = trf("TGOW Recorder was updated to %s", &[&version]);
        daemon.toast(ToastKind::Info, &text, true, None);
    }
    daemon.autostart_replay();
    if show {
        daemon.show_hide();
    }
    daemon.main_loop(rx);
    daemon.shutdown();
    ipc::unbind();
    if let Some(ready) = daemon.apply_update.take() {
        if !update::apply(&ready) {
            log_line("tgow-recorder: could not install the update");
        }
    }
    Ok(())
}

impl Daemon {
    fn main_loop(&mut self, rx: Receiver<Msg>) {
        while !self.quit {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if TERMINATE.load(Ordering::SeqCst) {
                self.quit = true;
            }
            self.tick();
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Request(request, reply) => {
                let event = self.request(request);
                let _ = reply.send(event);
            }
            Msg::Subscribe(stream) => {
                self.subscribers.push(stream);
                let status = Event::Status { status: self.status() };
                self.broadcast(&status);
            }
            Msg::Action(action) => {
                if let Err(err) = self.action(action) {
                    self.toast(ToastKind::Error, &err, true, None);
                }
            }
            Msg::Recorder { id, output } => self.recorder_output(id, output),
            Msg::RecorderClosed { id } => self.recorder_closed(id),
            Msg::Region { purpose, region } => self.region_selected(purpose, region),
            Msg::Kill { id } => {
                for s in [&mut self.replay, &mut self.record, &mut self.stream].into_iter().flatten() {
                    if s.id == id {
                        let _ = s.child.kill();
                    }
                }
            }
            Msg::Game(game) => self.game_changed(game),
            Msg::Update(found) => self.update_found(found),
            #[cfg(windows)]
            Msg::Notice(text) => self.toast(ToastKind::Error, &text, true, None),
            Msg::Quit => self.quit = true,
        }
        self.push_status(false);
    }

    fn request(&mut self, request: Request) -> Event {
        let result = match request {
            Request::Run { action } => self.action(action),
            Request::SaveReplay { seconds } => self.save_replay(Some(seconds)),
            Request::StartReplay => {
                if self.replay.is_none() {
                    self.start_replay()
                } else {
                    Ok(())
                }
            }
            Request::StopReplay => {
                self.stop(Mode::Replay);
                Ok(())
            }
            Request::StartRecord => {
                if self.record.is_none() {
                    self.begin_record(None)
                } else {
                    Ok(())
                }
            }
            Request::StopRecord => {
                self.stop(Mode::Record);
                Ok(())
            }
            Request::StartStream => {
                if self.stream.is_none() {
                    self.start_stream()
                } else {
                    Ok(())
                }
            }
            Request::StopStream => {
                self.stop(Mode::Stream);
                Ok(())
            }
            Request::ReloadConfig => {
                let previous = self.cfg.clone();
                self.cfg = Config::load();
                self.apply_config(Some(&previous));
                Ok(())
            }
            Request::ResetPortal => {
                let _ = std::fs::remove_file(paths::restore_token_file());
                let text =
                    tr("Portal selection reset. The portal will ask what you want to record the next time you use it.");
                self.toast(ToastKind::Info, &text, true, None);
                Ok(())
            }
            Request::Status => return Event::Status { status: self.status() },
            Request::Subscribe | Request::OverlayClosed => Ok(()),
            Request::Quit => {
                self.quit = true;
                Ok(())
            }
        };
        match result {
            Ok(()) => Event::Ok,
            Err(message) => {
                self.toast(ToastKind::Error, &message, true, None);
                Event::Error { message }
            }
        }
    }

    fn action(&mut self, action: Action) -> Result<(), String> {
        match action {
            Action::ShowHide => {
                self.show_hide();
                Ok(())
            }
            Action::ToggleReplay => {
                if self.replay.is_some() {
                    self.stop(Mode::Replay);
                    Ok(())
                } else {
                    self.start_replay()
                }
            }
            Action::SaveReplay => self.save_replay(None),
            Action::SaveReplayShort => self.save_replay(Some(self.cfg.replay.short_save_seconds.max(5))),
            Action::SaveReplay1Min => self.save_replay(Some(60)),
            Action::SaveReplay5Min => self.save_replay(Some(300)),
            Action::SaveReplay10Min => self.save_replay(Some(600)),
            Action::ToggleRecord => self.toggle_record(None),
            Action::ToggleRecordRegion => self.toggle_record(Some("region")),
            Action::ToggleRecordWindow => self.toggle_record(Some("window")),
            Action::ToggleRecordPortal => self.toggle_record(Some("portal")),
            Action::TogglePause => {
                if let Some(s) = self.record.as_mut() {
                    s.send(&RecCommand::TogglePause);
                }
                Ok(())
            }
            Action::ToggleStream => {
                if self.stream.is_some() {
                    self.stop(Mode::Stream);
                    Ok(())
                } else {
                    self.start_stream()
                }
            }
            Action::Screenshot => self.screenshot(None),
            Action::ScreenshotRegion => self.screenshot(Some("region")),
            Action::ScreenshotWindow => self.screenshot(Some("window")),
            Action::ScreenshotPortal => self.screenshot(Some("portal")),
            Action::ToggleMic => {
                self.mic_muted = !self.mic_muted;
                for mode in [Mode::Replay, Mode::Record, Mode::Stream] {
                    self.apply_mute(mode);
                }
                let text = if self.mic_muted { tr("Microphone muted") } else { tr("Microphone unmuted") };
                self.toast(ToastKind::Info, &text, true, None);
                Ok(())
            }
            Action::AddMarker => {
                if let Some(s) = self.record.as_mut() {
                    s.send(&RecCommand::Marker);
                    let text = tr("Chapter marker added");
                    self.toast(ToastKind::Record, &text, true, None);
                }
                Ok(())
            }
        }
    }

    fn show_hide(&mut self) {
        if !self.subscribers.is_empty() {
            self.broadcast(&Event::Close);
            return;
        }
        if let Some(child) = self.overlay.as_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
                let _ = child.wait();
                self.overlay = None;
                return;
            }
        }
        desktop::allow_foreground();
        match quiet(&mut Command::new(paths::sibling_binary("tgow-overlay"))).stdin(Stdio::null()).spawn() {
            Ok(child) => self.overlay = Some(child),
            Err(err) => eprintln!("tgow-recorder: could not start the overlay: {err}"),
        }
    }

    fn conflict(&self, mode: Mode) -> Option<String> {
        let key = match mode {
            Mode::Record if self.replay.is_some() => {
                "Unable to start recording when replay is turned on. Turn off replay before starting recording."
            }
            Mode::Record if self.stream.is_some() => {
                "Unable to start recording when streaming. Stop streaming before starting recording."
            }
            Mode::Stream if self.replay.is_some() => {
                "Unable to start streaming when replay is turned on. Turn off replay before starting streaming."
            }
            Mode::Stream if self.record.is_some() => {
                "Unable to start streaming when recording. Stop recording before starting streaming."
            }
            Mode::Replay if self.record.is_some() => {
                "Unable to start replay when recording. Stop recording before starting replay."
            }
            Mode::Replay if self.stream.is_some() => {
                "Unable to start replay when streaming. Stop streaming before starting replay."
            }
            _ => return None,
        };
        Some(tr(key))
    }

    fn resolve_source(&self, name: &str) -> String {
        if name == "window" && self.wayland {
            "portal".into()
        } else {
            name.to_string()
        }
    }

    fn select_region(&mut self, purpose: Purpose) -> Result<(), String> {
        if self.region_pending {
            return Ok(());
        }
        self.broadcast(&Event::Close);
        desktop::allow_foreground();
        let child = quiet(&mut Command::new(paths::sibling_binary("tgow-overlay")))
            .arg("--select-region")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(_) => return Err(tr("Failed to start region capture")),
        };
        self.region_pending = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut line = String::new();
            if let Some(out) = child.stdout.take() {
                let _ = BufReader::new(out).read_line(&mut line);
            }
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let line = line.trim().to_string();
            let region = if ok && !line.is_empty() { Some(line) } else { None };
            let _ = tx.send(Msg::Region { purpose, region });
        });
        Ok(())
    }

    fn region_selected(&mut self, purpose: Purpose, region: Option<String>) {
        self.region_pending = false;
        let Some(region) = region else { return };
        let source = Source { name: "region".into(), region: Some(region) };
        let result = match purpose {
            Purpose::Record => self.record_with_countdown(source),
            Purpose::Replay => self.spawn_replay(source),
            Purpose::Stream => self.spawn_stream(source),
            Purpose::Screenshot => self.spawn_screenshot(source),
        };
        if let Err(err) = result {
            self.toast(ToastKind::Error, &err, true, None);
        }
    }

    fn toggle_record(&mut self, source: Option<&str>) -> Result<(), String> {
        if self.countdown.take().is_some() {
            return Ok(());
        }
        if self.record.is_some() {
            self.stop(Mode::Record);
            return Ok(());
        }
        self.begin_record(source)
    }

    fn begin_record(&mut self, source: Option<&str>) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Record) {
            return Err(err);
        }
        let name = self.resolve_source(source.unwrap_or(&self.cfg.record.capture.source));
        if name == "region" {
            return self.select_region(Purpose::Record);
        }
        self.record_with_countdown(Source::plain(&name))
    }

    fn record_with_countdown(&mut self, source: Source) -> Result<(), String> {
        let secs = self.cfg.main.countdown_seconds.min(60);
        if secs == 0 {
            return self.spawn_record(source);
        }
        self.countdown = Some(Countdown { remaining: secs, next: Instant::now(), source });
        Ok(())
    }

    fn spawn_record(&mut self, source: Source) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Record) {
            return Err(err);
        }
        let r = &self.cfg.record;
        let dir = paths::expand_tilde(&r.save_directory);
        let path = system::output_path(
            &dir,
            "Video",
            &r.container,
            self.game.as_deref(),
            self.game.is_some(),
            r.organize_into_folders,
            r.name_after_application,
        );
        make_parent(&path)?;
        let argv = args::record(&self.cfg, &source, &path);
        let session = self.spawn_session(&source.name, argv)?;
        self.record = Some(session);
        self.low_disk_warned = false;
        Ok(())
    }

    fn start_replay(&mut self) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Replay) {
            return Err(err);
        }
        let name = self.resolve_source(&self.cfg.replay.capture.source);
        if name == "region" {
            return self.select_region(Purpose::Replay);
        }
        self.spawn_replay(Source::plain(&name))
    }

    fn spawn_replay(&mut self, source: Source) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Replay) {
            return Err(err);
        }
        let dir = paths::expand_tilde(&self.cfg.replay.save_directory);
        if paths::ensure_dir(&dir).is_err() {
            return Err(tr("Failed to create the save directory"));
        }
        let argv = args::replay(&self.cfg, &source, &dir);
        let session = self.spawn_session(&source.name, argv)?;
        self.replay = Some(session);
        self.low_disk_warned = false;
        Ok(())
    }

    fn start_stream(&mut self) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Stream) {
            return Err(err);
        }
        let s = &self.cfg.stream;
        let required = match s.service {
            StreamService::Twitch => &s.twitch_key,
            StreamService::Youtube => &s.youtube_key,
            StreamService::Kick => {
                if s.kick_url.trim().is_empty() {
                    &s.kick_url
                } else {
                    &s.kick_key
                }
            }
            StreamService::Rumble => &s.rumble_key,
            StreamService::Whip => &s.whip_url,
            StreamService::Custom => &s.custom_url,
        };
        if required.trim().is_empty() {
            return Err(tr("Failed to start streaming, the stream URL is empty. Please set a stream URL in settings"));
        }
        if s.service == StreamService::Whip {
            let url = s.whip_url.trim();
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(tr("Failed to start streaming, the WHIP stream URL should start with http:// or https://"));
            }
        }
        let name = self.resolve_source(&s.capture.source);
        if name == "region" {
            return self.select_region(Purpose::Stream);
        }
        self.spawn_stream(Source::plain(&name))
    }

    fn spawn_stream(&mut self, source: Source) -> Result<(), String> {
        if let Some(err) = self.conflict(Mode::Stream) {
            return Err(err);
        }
        let argv = args::stream(&self.cfg, &source);
        let session = self.spawn_session(&source.name, argv)?;
        self.stream = Some(session);
        Ok(())
    }

    fn screenshot(&mut self, source: Option<&str>) -> Result<(), String> {
        if self.shot.is_some() {
            return Ok(());
        }
        let name = self.resolve_source(source.unwrap_or(&self.cfg.screenshot.source));
        if name == "region" {
            return self.select_region(Purpose::Screenshot);
        }
        self.spawn_screenshot(Source::plain(&name))
    }

    fn spawn_screenshot(&mut self, source: Source) -> Result<(), String> {
        let s = &self.cfg.screenshot;
        let to_disk = s.save_to_disk || !s.save_to_clipboard;
        let path = if to_disk {
            system::output_path(
                &paths::expand_tilde(&s.save_directory),
                "Screenshot",
                s.format.ext(),
                self.game.as_deref(),
                self.game.is_some(),
                s.organize_into_folders,
                s.name_after_application,
            )
        } else {
            paths::runtime_dir().join(format!("clipboard.{}", s.format.ext()))
        };
        make_parent(&path)?;
        let argv = args::screenshot(&self.cfg, &source, &path);
        let led = s.led_indicator;
        let (id, child, _) = self.spawn_recorder(argv).map_err(|_| tr("Failed to take a screenshot"))?;
        if led {
            desktop::blink_led();
        }
        self.shot = Some(Shot { id, child, path, source: source.name, to_disk });
        Ok(())
    }

    fn spawn_recorder(&mut self, argv: Vec<String>) -> std::io::Result<(u64, Child, Option<ChildStdin>)> {
        let mut child = quiet(&mut Command::new(paths::sibling_binary("tgow-rec")))
            .args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let id = self.next_id;
        self.next_id += 1;
        let stdout = child.stdout.take();
        let stdin = child.stdin.take();
        if let Some(err) = child.stderr.take() {
            std::thread::Builder::new().name(format!("rec-{id}-log")).spawn(move || {
                for line in BufReader::new(err).lines() {
                    let Ok(line) = line else { break };
                    log_line(&line);
                }
            })?;
        }
        let tx = self.tx.clone();
        std::thread::Builder::new().name(format!("rec-{id}")).spawn(move || {
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    match Output::parse(&line) {
                        Some(output) => {
                            if tx.send(Msg::Recorder { id, output }).is_err() {
                                return;
                            }
                        }
                        None => log_line(&line),
                    }
                }
            }
            let _ = tx.send(Msg::RecorderClosed { id });
        })?;
        Ok((id, child, stdin))
    }

    fn spawn_session(&mut self, source: &str, argv: Vec<String>) -> Result<Session, String> {
        let (id, child, stdin) = self.spawn_recorder(argv).map_err(|err| {
            log_line(&format!("tgow-recorder: could not start the recorder: {err}"));
            tr("Failed to launch the recorder")
        })?;
        Ok(Session {
            id,
            child,
            stdin,
            source: source.to_string(),
            started: Instant::now(),
            paused_total: Duration::ZERO,
            paused_since: None,
            running: false,
            stopping: false,
            saved: false,
        })
    }

    fn session_mut(&mut self, mode: Mode) -> &mut Option<Session> {
        match mode {
            Mode::Replay => &mut self.replay,
            Mode::Record => &mut self.record,
            Mode::Stream => &mut self.stream,
        }
    }

    fn find_mode(&self, id: u64) -> Option<Mode> {
        [(&self.replay, Mode::Replay), (&self.record, Mode::Record), (&self.stream, Mode::Stream)]
            .into_iter()
            .find(|(s, _)| s.as_ref().map(|s| s.id) == Some(id))
            .map(|(_, m)| m)
    }

    fn stop(&mut self, mode: Mode) {
        let tx = self.tx.clone();
        if let Some(s) = self.session_mut(mode).as_mut() {
            if s.stopping {
                return;
            }
            s.stopping = true;
            s.send(&RecCommand::Stop);
            s.stdin = None;
            let id = s.id;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(15));
                let _ = tx.send(Msg::Kill { id });
            });
        }
    }

    fn save_replay(&mut self, seconds: Option<u32>) -> Result<(), String> {
        let duration = self.cfg.replay.duration_seconds;
        let Some(s) = self.replay.as_mut() else { return Ok(()) };
        if !s.running || s.stopping {
            return Ok(());
        }
        match seconds {
            Some(secs) => s.send(&RecCommand::SaveSeconds(secs)),
            None => s.send(&RecCommand::Save),
        }
        if seconds.unwrap_or(duration) >= 300 {
            let text = tr("Saving replay, this might take some time");
            let show = self.cfg.replay.capture.show_notifications;
            self.toast(ToastKind::Replay, &text, show, None);
        }
        Ok(())
    }

    fn capture_for(&self, mode: Mode) -> &CaptureOptions {
        match mode {
            Mode::Replay => &self.cfg.replay.capture,
            Mode::Record => &self.cfg.record.capture,
            Mode::Stream => &self.cfg.stream.capture,
        }
    }

    fn apply_mute(&mut self, mode: Mode) {
        let tracks: Vec<usize> = self
            .capture_for(mode)
            .audio_tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.inputs.iter().any(|i| is_microphone(i)))
            .map(|(i, _)| i)
            .collect();
        let muted = self.mic_muted;
        if let Some(s) = self.session_mut(mode).as_mut() {
            for index in tracks {
                s.send(&RecCommand::Mute(index, muted));
            }
        }
    }

    fn recorder_output(&mut self, id: u64, output: Output) {
        if let Some(shot) = &self.shot {
            if shot.id == id {
                self.shot_output(output);
                return;
            }
        }
        let Some(mode) = self.find_mode(id) else { return };
        let show = self.capture_for(mode).show_notifications;
        let target =
            tr(args::target_name(&self.session_mut(mode).as_ref().map(|s| s.source.clone()).unwrap_or_default()));
        match output {
            Output::Started => {
                if let Some(s) = self.session_mut(mode).as_mut() {
                    s.running = true;
                    s.started = Instant::now();
                }
                let key = match mode {
                    Mode::Replay => "Started replaying %s",
                    Mode::Record => "Started recording %s",
                    Mode::Stream => "Started streaming %s",
                };
                self.toast(mode.kind(), &trf(key, &[&target]), show, None);
                if self.mic_muted {
                    self.apply_mute(mode);
                }
            }
            Output::Saved { kind, path, seconds } => {
                if let Some(s) = self.session_mut(mode).as_mut() {
                    s.saved = true;
                }
                let key = if kind == "replay" { "Saved a %s replay of %s" } else { "Saved a %s recording of %s" };
                let shown = self.game.clone().unwrap_or(target);
                let text = trf(key, &[&duration_text(seconds), &shown]);
                let path = if kind == "replay" { self.place_replay(&path) } else { path };
                let path = path.to_string_lossy().into_owned();
                self.last_saved = Some(path.clone());
                self.toast(mode.kind(), &text, show, Some(path));
            }
            Output::Paused(paused) => {
                if let Some(s) = self.session_mut(mode).as_mut() {
                    if paused && s.paused_since.is_none() {
                        s.paused_since = Some(Instant::now());
                    } else if !paused {
                        if let Some(since) = s.paused_since.take() {
                            s.paused_total += since.elapsed();
                        }
                    }
                }
                let text = if paused { tr("Recording has been paused") } else { tr("Recording has been unpaused") };
                self.toast(ToastKind::Record, &text, show, None);
            }
            Output::Error(message) => {
                let text = trf("%s. Verify if settings are correct", &[message.trim_end_matches('.')]);
                self.toast(ToastKind::Error, &text, true, None);
            }
            Output::Warning(message) => eprintln!("tgow-recorder: recorder warning: {message}"),
            Output::Finished => {}
        }
    }

    fn shot_output(&mut self, output: Output) {
        let show = self.cfg.screenshot.show_notifications;
        match output {
            Output::Saved { path, .. } => {
                let Some(shot) = self.shot.as_ref() else { return };
                let target = tr(args::target_name(&shot.source));
                let to_disk = shot.to_disk;
                let mut text = trf("Saved a screenshot of %s", &[&target]);
                if self.cfg.screenshot.save_to_clipboard && desktop::copy_image(&path) && !to_disk {
                    text = tr("Screenshot copied to clipboard");
                }
                if to_disk {
                    desktop::run_open_command(&self.cfg.screenshot.open_command, &path);
                    self.last_saved = Some(path.to_string_lossy().into_owned());
                }
                let shown_path = if to_disk { Some(path.to_string_lossy().into_owned()) } else { None };
                self.toast(ToastKind::Screenshot, &text, show, shown_path);
            }
            Output::Error(message) => eprintln!("tgow-recorder: screenshot failed: {message}"),
            _ => {}
        }
    }

    fn recorder_closed(&mut self, id: u64) {
        if self.shot.as_ref().map(|s| s.id) == Some(id) {
            let mut shot = self.shot.take().unwrap();
            let ok = shot.child.wait().map(|s| s.success()).unwrap_or(false);
            if !ok || !shot.path.exists() {
                let text = tr("Failed to take a screenshot");
                self.toast(ToastKind::Error, &text, true, None);
            } else if !shot.to_disk {
                let path = shot.path.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    let _ = std::fs::remove_file(path);
                });
            }
            return;
        }
        let Some(mode) = self.find_mode(id) else { return };
        let Some(mut session) = self.session_mut(mode).take() else { return };
        let _ = session.child.wait();
        let show = self.capture_for(mode).show_notifications;
        match mode {
            Mode::Record => {
                if !session.saved {
                    let text = tr("Failed to start/save recording");
                    self.toast(ToastKind::Error, &text, true, None);
                }
            }
            Mode::Replay => {
                if session.stopping {
                    let text = tr("Replay stopped");
                    self.toast(ToastKind::Replay, &text, show, None);
                } else {
                    let text = tr("Replay stopped because of an error");
                    self.toast(ToastKind::Error, &text, true, None);
                }
            }
            Mode::Stream => {
                if session.stopping {
                    let text = tr("Streaming has stopped");
                    self.toast(ToastKind::Stream, &text, show, None);
                } else {
                    let text = tr("Streaming stopped because of an error");
                    self.toast(ToastKind::Error, &text, true, None);
                }
            }
        }
    }

    fn toast(&mut self, kind: ToastKind, text: &str, notify: bool, path: Option<String>) {
        if notify && !self.overlay_covers_toasts() {
            let timeout = match self.cfg.main.notification_speed {
                NotificationSpeed::Normal => 3000,
                NotificationSpeed::Fast => 1500,
            };
            let timeout = if kind == ToastKind::Error { timeout * 2 } else { timeout };
            self.notifier.send(kind, text, timeout);
        }
        self.broadcast(&Event::Toast { kind, text: text.to_string(), path });
    }

    fn update_found(&mut self, found: update::Found) {
        match found {
            update::Found::Available(version) => {
                if self.update_noticed.as_deref() != Some(version.as_str()) {
                    let text = trf("TGOW Recorder %s is available at %s", &[&version, update::RELEASES_PAGE]);
                    self.toast(ToastKind::Info, &text, true, None);
                    self.update_noticed = Some(version);
                }
            }
            update::Found::Ready(ready) => self.update_ready = Some(ready),
        }
    }

    fn idle(&self) -> bool {
        self.replay.is_none()
            && self.record.is_none()
            && self.stream.is_none()
            && self.shot.is_none()
            && self.countdown.is_none()
            && self.overlay.is_none()
            && !self.region_pending
    }

    fn try_update(&mut self) {
        if self.update_ready.is_none() || !self.cfg.main.auto_update || !self.idle() {
            return;
        }
        let Some(ready) = self.update_ready.take() else { return };
        if cfg!(unix) {
            let text = trf("Updating TGOW Recorder to %s", &[&ready.version]);
            self.toast(ToastKind::Info, &text, true, None);
        }
        self.apply_update = Some(ready);
        self.quit = true;
    }

    fn overlay_covers_toasts(&self) -> bool {
        cfg!(windows) && self.overlay.is_some()
    }

    fn broadcast(&mut self, event: &Event) {
        if self.subscribers.is_empty() {
            return;
        }
        let line = ipc::encode(event);
        self.subscribers.retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
    }

    fn mode_status(&self, session: &Option<Session>) -> ModeStatus {
        match session {
            Some(s) => ModeStatus {
                active: true,
                elapsed_secs: if s.running { s.elapsed() } else { 0.0 },
                target: tr(args::target_name(&s.source)),
            },
            None => ModeStatus::default(),
        }
    }

    fn status(&self) -> Status {
        let replay = self.mode_status(&self.replay);
        let buffer = replay.elapsed_secs.min(self.cfg.replay.duration_seconds as f64);
        Status {
            replay,
            record: self.mode_status(&self.record),
            stream: self.mode_status(&self.stream),
            paused: self.record.as_ref().map(|s| s.paused_since.is_some()).unwrap_or(false),
            mic_muted: self.mic_muted,
            replay_buffer_secs: buffer,
            last_saved: self.last_saved.clone(),
            free_space_bytes: self.free_space,
            gpu_vendor: self.gpu_vendor.clone(),
            version: tgow_common::VERSION.to_string(),
            wayland: self.wayland,
        }
    }

    fn push_status(&mut self, force: bool) {
        let status = self.status();
        let tray_state = TrayState {
            recording: status.record.active,
            paused: status.paused,
            replay: status.replay.active,
            streaming: status.stream.active,
        };
        self.tray.update(tray_state);
        if !force && self.last_status.as_ref() == Some(&status) {
            return;
        }
        self.last_status = Some(status.clone());
        self.broadcast(&Event::Status { status });
    }

    fn any_active(&self) -> bool {
        self.replay.is_some() || self.record.is_some() || self.stream.is_some()
    }

    fn tick(&mut self) {
        self.try_update();
        if let Some(cd) = self.countdown.as_mut() {
            if Instant::now() >= cd.next {
                if cd.remaining == 0 {
                    let source = self.countdown.take().unwrap().source;
                    if let Err(err) = self.spawn_record(source) {
                        self.toast(ToastKind::Error, &err, true, None);
                    }
                } else {
                    let n = cd.remaining;
                    cd.remaining -= 1;
                    cd.next += Duration::from_secs(1);
                    let text = trf("Starting in %s", &[&n.to_string()]);
                    if !self.overlay_covers_toasts() {
                        self.notifier.send(ToastKind::Record, &text, 900);
                    }
                    self.broadcast(&Event::Toast { kind: ToastKind::Record, text, path: None });
                }
            }
        }

        if let Some(child) = self.overlay.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                self.overlay = None;
            }
        }

        if self.last_tick.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_tick = Instant::now();

        if self.last_disk_check.elapsed() >= Duration::from_secs(30) {
            self.last_disk_check = Instant::now();
            self.check_disk();
        }
        self.update_led();
        self.push_status(self.any_active());
    }

    fn check_disk(&mut self) {
        let dir = if self.replay.is_some() && self.record.is_none() {
            &self.cfg.replay.save_directory
        } else {
            &self.cfg.record.save_directory
        };
        self.free_space = paths::free_space_bytes(&paths::expand_tilde(dir));
        let limit = self.cfg.main.low_disk_warning_gb as u64 * 1_000_000_000;
        if limit == 0 || !(self.record.is_some() || self.replay.is_some()) {
            return;
        }
        match self.free_space {
            Some(free) if free < limit => {
                if !self.low_disk_warned {
                    self.low_disk_warned = true;
                    let gb = format!("{:.1}", free as f64 / 1e9);
                    let text = trf("Low disk space: %s GB left", &[&gb]);
                    self.toast(ToastKind::Error, &text, true, None);
                }
            }
            Some(_) => self.low_disk_warned = false,
            None => {}
        }
    }

    fn update_led(&self) {
        let wants = (self.replay.is_some() && self.cfg.replay.capture.led_indicator)
            || (self.record.is_some() && self.cfg.record.capture.led_indicator)
            || (self.stream.is_some() && self.cfg.stream.capture.led_indicator);
        let any_led = self.cfg.replay.capture.led_indicator
            || self.cfg.record.capture.led_indicator
            || self.cfg.stream.capture.led_indicator;
        if any_led {
            system::set_scroll_lock_led(wants);
        }
    }

    fn apply_config(&mut self, previous: Option<&Config>) {
        i18n::set_language(&self.cfg.main.language);
        let main_changed = previous.map(|p| p.main != self.cfg.main).unwrap_or(true);
        if main_changed {
            let hotkeys_changed = previous
                .map(|p| p.main.hotkeys != self.cfg.main.hotkeys || p.main.hotkey_mode != self.cfg.main.hotkey_mode)
                .unwrap_or(true);
            if hotkeys_changed {
                self.hotkeys.apply(&self.cfg.main, &self.tx);
            }
            self.controller.store(self.cfg.main.controller_hotkeys, Ordering::Relaxed);
            self.updates.store(self.cfg.main.auto_update, Ordering::Relaxed);
            self.tray.set_visible(self.cfg.main.show_tray_icon, &self.tx);
            self.tray.refresh();
        }
        let autostart_changed = previous.map(|p| p.main.start_on_login != self.cfg.main.start_on_login);
        if autostart_changed.unwrap_or(self.cfg.main.start_on_login)
            && desktop::set_autostart(self.cfg.main.start_on_login).is_err()
        {
            let text = tr("Failed to add TGOW Recorder to system startup");
            self.toast(ToastKind::Error, &text, true, None);
        }
        if let Some(prev) = previous {
            if self.replay.is_some() && prev.replay != self.cfg.replay {
                let text =
                    tr("Replay settings have been modified. You may need to restart replay to apply the changes.");
                self.toast(ToastKind::Replay, &text, true, None);
            }
            if self.stream.is_some() && prev.stream != self.cfg.stream {
                let text = tr(
                    "Streaming settings have been modified. You may need to restart streaming to apply the changes.",
                );
                self.toast(ToastKind::Stream, &text, true, None);
            }
        }
    }

    fn game_changed(&mut self, game: Option<String>) {
        let started = self.game.is_none() && game.is_some();
        let ended = self.game.is_some() && game.is_none();
        self.game = game;
        if self.cfg.replay.autostart != ReplayAutostart::OnGame {
            return;
        }
        if started && !self.any_active() {
            match self.start_replay() {
                Ok(()) => self.replay_for_game = true,
                Err(err) => self.toast(ToastKind::Error, &err, true, None),
            }
        } else if ended && self.replay_for_game {
            self.replay_for_game = false;
            if self.replay.is_some() {
                self.stop(Mode::Replay);
            }
        }
    }

    fn place_replay(&self, saved: &Path) -> PathBuf {
        let Some(game) = self.game.as_deref() else { return saved.to_path_buf() };
        let r = &self.cfg.replay;
        let uses_game = |c: YesNoGames| c != YesNoGames::No;
        if !uses_game(r.organize_into_folders) && !uses_game(r.name_after_application) {
            return saved.to_path_buf();
        }
        let base = paths::expand_tilde(&r.save_directory);
        let ext = saved.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| r.container.clone());
        let target = system::output_path(
            &base,
            "Replay",
            &ext,
            Some(game),
            true,
            r.organize_into_folders,
            r.name_after_application,
        );
        if make_parent(&target).is_err() || std::fs::rename(saved, &target).is_err() {
            return saved.to_path_buf();
        }
        if let Some(old_dir) = saved.parent() {
            if old_dir != base {
                let _ = std::fs::remove_dir(old_dir);
            }
        }
        target
    }

    fn autostart_replay(&mut self) {
        let start = match self.cfg.replay.autostart {
            ReplayAutostart::OnStartup => true,
            ReplayAutostart::OnStartupWithPower => system::on_ac_power(),
            ReplayAutostart::Off | ReplayAutostart::OnGame => false,
        };
        if start {
            if let Err(err) = self.start_replay() {
                self.toast(ToastKind::Error, &err, true, None);
            }
        }
    }

    fn shutdown(&mut self) {
        std::thread::sleep(Duration::from_millis(100));
        self.broadcast(&Event::Close);
        for mode in [Mode::Replay, Mode::Record, Mode::Stream] {
            self.stop(mode);
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        for slot in [&mut self.replay, &mut self.record, &mut self.stream] {
            if let Some(mut s) = slot.take() {
                loop {
                    match s.child.try_wait() {
                        Ok(Some(_)) | Err(_) => break,
                        Ok(None) if Instant::now() >= deadline => {
                            let _ = s.child.kill();
                            let _ = s.child.wait();
                            break;
                        }
                        Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
            }
        }
        if let Some(mut child) = self.overlay.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        system::set_scroll_lock_led(false);
        self.tray.set_visible(false, &self.tx);
        if self.apply_update.is_some() {
            self.notifier.close();
        }
    }
}

fn is_microphone(input: &str) -> bool {
    let name = input.strip_prefix("device:").unwrap_or(input);
    if input.starts_with("app:") || input.starts_with("app-inverse:") {
        return false;
    }
    name == "default_input" || (name != "default_output" && !name.ends_with(".monitor"))
}

fn make_parent(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(dir) => paths::ensure_dir(dir).map_err(|_| tr("Failed to create the save directory")),
        None => Ok(()),
    }
}

pub fn log_line(line: &str) {
    eprintln!("{line}");
    let path = paths::log_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {line}", system::now_local().file_stamp());
    }
}

pub fn trim_log() {
    let path = paths::log_file();
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
}
