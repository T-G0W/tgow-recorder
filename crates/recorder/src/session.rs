use anyhow::{anyhow, bail, Result};
use ffmpeg_next::Rational;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tgow_common::ipc::recorder::Command;

use crate::args::{Codec, FpsMode, Options};
use crate::audio::{backend, sources, AudioEncoder, Mixer};
use crate::capture;
use crate::control;
use crate::frame::{Frame, Wait};
use crate::output::{self, Mode, Msg, MuxOptions, OutputJob, StreamDesc, Tagged};
use crate::report;
use crate::video::{self, VideoEncoder};

fn container_for(opts: &Options, target: &str, replay: bool) -> String {
    if let Some(c) = &opts.container {
        return c.clone();
    }
    if target.starts_with("srt://") || target.starts_with("udp://") {
        return "mpegts".into();
    }
    if target.starts_with("rtmp://") || target.starts_with("rtmps://") {
        return "flv".into();
    }
    if replay {
        return "mp4".into();
    }
    Path::new(target)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "mp4".into())
}

fn audio_codec_for(opts: &Options, container: &str) -> String {
    let format = output::format_name(container);
    match format {
        "mp3" => return "mp3".into(),
        "webm" => return "opus".into(),
        "flv" => return "aac".into(),
        _ => {}
    }
    match opts.audio_codec.as_deref() {
        Some("flac") if matches!(format, "matroska" | "mp4" | "mov") => "flac".into(),
        Some("mp3") => "mp3".into(),
        Some("aac") => "aac".into(),
        Some("opus") => {
            if matches!(format, "mpegts" | "hls") {
                "aac".into()
            } else {
                "opus".into()
            }
        }
        _ => {
            if matches!(format, "mpegts" | "hls" | "mov") {
                "aac".into()
            } else {
                "opus".into()
            }
        }
    }
}

fn check_codec_container(opts: &mut Options, container: &str) {
    let format = output::format_name(container);
    if format == "webm" && !matches!(opts.codec, Codec::Vp8 | Codec::Vp9 | Codec::Av1) {
        let gpu_av1 = !opts.cpu_encoder && tgow_common::system::gpu_vendor() != "unknown";
        opts.codec = if gpu_av1 { Codec::Av1 } else { Codec::Vp9 };
        report::info(&format!("webm only holds vp8, vp9 or av1, using {:?}", opts.codec));
    }
    if matches!(format, "mpegts" | "flv") && matches!(opts.codec, Codec::Av1 | Codec::Vp8 | Codec::Vp9) {
        report::warning("this container can't carry that codec, using h264");
        opts.codec = Codec::H264;
    }
    if format == "whip" && opts.codec != Codec::H264 {
        opts.codec = Codec::H264;
    }
}

fn wait_first_frame(cap: &capture::Capture) -> Result<Arc<Frame>> {
    match cap.slot.wait_newer(0, Duration::from_secs(20)) {
        Wait::Frame(f, _) => Ok(f),
        Wait::Timeout => bail!("no image arrived from the capture source"),
        Wait::Closed(reason) => bail!("capture failed: {reason}"),
    }
}

pub fn screenshot(opts: &Options, path: &Path) -> Result<()> {
    let cap = capture::start(opts)?;
    let first = match wait_first_frame(&cap) {
        Ok(f) => f,
        Err(e) => {
            cap.shutdown();
            return Err(e);
        }
    };
    let frame = match cap.slot.wait_newer(1, Duration::from_millis(150)) {
        Wait::Frame(newer, _) => newer,
        _ => first,
    };
    let mut owned = (*frame).clone();
    if let Some(feed) = &cap.webcam {
        std::thread::sleep(Duration::from_millis(300));
        feed.composite(&mut owned);
    }
    cap.shutdown();
    let rgba = owned.to_rgba();
    let mut img = image::RgbaImage::from_raw(owned.width, owned.height, rgba).ok_or_else(|| anyhow!("bad frame"))?;
    let (tw, th) = video::fit_size(owned.width, owned.height, opts.size_limit);
    if (tw, th) != (owned.width & !1, owned.height & !1) {
        img = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "jpg" || ext == "jpeg" {
        let quality = match opts.quality {
            crate::args::Quality::Preset(0) => 75,
            crate::args::Quality::Preset(1) => 85,
            crate::args::Quality::Preset(3) => 98,
            _ => 92,
        };
        let file = std::fs::File::create(path)?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), quality);
        enc.encode_image(&image::DynamicImage::ImageRgba8(img).to_rgb8())?;
    } else {
        img.save_with_format(path, image::ImageFormat::Png)?;
    }
    report::saved("screenshot", path, 0.0);
    if let Some(script) = &opts.script {
        let _ = std::process::Command::new(script).arg(path).arg("screenshot").spawn();
    }
    Ok(())
}

struct Pacer {
    mode: FpsMode,
    fps: u32,
    start: Instant,
    paused_total: Duration,
    paused_since: Option<Instant>,
    tick: i64,
    last_pts: i64,
}

impl Pacer {
    fn active(&self, at: Instant) -> Duration {
        at.saturating_duration_since(self.start).saturating_sub(self.paused_total)
    }

    fn pause(&mut self, on: bool) {
        let now = Instant::now();
        if on && self.paused_since.is_none() {
            self.paused_since = Some(now);
        } else if !on {
            if let Some(s) = self.paused_since.take() {
                self.paused_total += now - s;
            }
        }
    }

    fn vfr_pts(&mut self, captured: Instant) -> i64 {
        let pts = (self.active(captured).as_secs_f64() * 90_000.0) as i64;
        let min_step = 90_000 / self.fps.max(1) as i64 / 2;
        let pts = if self.tick == 0 { pts.max(0) } else { pts.max(self.last_pts + min_step.max(1)) };
        self.tick += 1;
        self.last_pts = pts;
        pts
    }
}

fn send_packets(tx: &Sender<Msg>, stream: usize, packets: &mut Vec<ffmpeg_next::Packet>) {
    for p in packets.drain(..) {
        let _ = tx.send(Msg::Packet(Tagged { stream, packet: p }));
    }
}

pub fn run(mut opts: Options) -> Result<()> {
    ffmpeg_next::init().map_err(|e| anyhow!("FFmpeg init failed: {e}"))?;
    report::set_verbose(opts.verbose);
    let target = opts.output.clone().unwrap_or_default();
    let replay = opts.replay_seconds.is_some();
    let container = container_for(&opts, &target, replay);
    let streaming = !replay && (target.contains("://") || output::format_name(&container) == "whip");
    check_codec_container(&mut opts, &container);
    let global_header = output::wants_global_header(&container);

    let stop = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    control::install_signals();
    let commands = control::listen(stop.clone());

    let cap = if opts.source.is_some() { Some(capture::start(&opts)?) } else { None };

    let mut video_enc = None;
    let mut first_frame = None;
    if let Some(c) = &cap {
        let f = match wait_first_frame(c) {
            Ok(f) => f,
            Err(e) => {
                if let Some(c) = cap {
                    c.shutdown();
                }
                return Err(e);
            }
        };
        let enc = VideoEncoder::open(&video::Setup { opts: &opts, src_w: f.width, src_h: f.height, global_header })?;
        report::info(&format!(
            "capturing {} at {}x{}, encoding {}x{} with {} ({:?})",
            c.label, f.width, f.height, enc.width, enc.height, enc.codec_name, enc.backend
        ));
        video_enc = Some(enc);
        first_frame = Some(f);
    }

    let audio_codec = audio_codec_for(&opts, &container);
    let mut streams = Vec::new();
    if let Some(v) = &video_enc {
        streams.push(StreamDesc {
            params: v.parameters(),
            time_base: v.time_base,
            video: true,
            title: String::new(),
            frame_rate: Some(Rational::new(opts.fps as i32, 1)),
        });
    }
    let mut audio_encoders = Vec::new();
    let mut track_inputs = Vec::new();
    let mut track_wants = Vec::new();
    for (i, track) in opts.audio.iter().enumerate() {
        let enc = AudioEncoder::open(&audio_codec, opts.audio_bitrate, global_header, &opts.audio_opts)?;
        let title = if !track.title.is_empty() {
            track.title.clone()
        } else if opts.audio.len() > 1 {
            format!("Track {}", i + 1)
        } else {
            String::new()
        };
        streams.push(StreamDesc {
            params: enc.parameters(),
            time_base: enc.time_base,
            video: false,
            title,
            frame_rate: None,
        });
        audio_encoders.push(enc);
        track_inputs.push(Arc::new(sources::TrackInputs::default()));
        track_wants.push(sources::wants_for(&track.sources));
    }
    if streams.is_empty() {
        bail!("nothing to record");
    }

    let mode = if replay {
        Mode::Replay {
            seconds: opts.replay_seconds.unwrap_or(60),
            on_disk: opts.replay_on_disk,
            dir: PathBuf::from(&target),
            container: container.clone(),
            restart_on_save: opts.restart_replay_on_save,
            date_folders: opts.date_folders,
        }
    } else if streaming {
        Mode::Stream { url: target.clone(), container: container.clone() }
    } else {
        Mode::Record {
            dir_hint: Path::new(&target).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")),
            container: container.clone(),
            split_secs: (opts.split_minutes > 0).then_some(opts.split_minutes as f64 * 60.0),
            first_path: target.clone(),
        }
    };
    let job = OutputJob {
        mode,
        streams,
        mux: MuxOptions {
            auth: opts.auth.clone(),
            exclude_metadata: opts.exclude_metadata,
            extra: opts.muxer_opts.clone(),
        },
        script: opts.script.clone(),
    };

    let (out_tx, out_rx) = channel::<Msg>();
    let (setup_tx, setup_rx) = channel::<Result<()>>();
    let output_thread = std::thread::Builder::new().name("output".into()).spawn(move || {
        let r = output::run(job, out_rx);
        if let Err(e) = &r {
            report::error(&format!("{e:#}"));
        }
        let _ = setup_tx.send(r);
    })?;

    let audio_stop = Arc::new(AtomicBool::new(false));
    let muted: Arc<Vec<AtomicBool>> = Arc::new((0..audio_encoders.len()).map(|_| AtomicBool::new(false)).collect());
    let mut capture_threads = Vec::new();
    let start = Instant::now();
    let mut mixer_thread = None;
    if !audio_encoders.is_empty() {
        match backend::spawn(track_wants, track_inputs.clone(), audio_stop.clone()) {
            Ok(h) => capture_threads.push(h),
            Err(e) => report::warning(&format!("audio capture is off: {e:#}")),
        }
        let (atx, arx) = channel::<Tagged>();
        let base = if video_enc.is_some() { 1 } else { 0 };
        let mixer = Mixer {
            inputs: track_inputs,
            encoders: audio_encoders,
            stream_index: (0..opts.audio.len()).map(|i| base + i).collect(),
            muted: muted.clone(),
        };
        mixer_thread = Some(mixer.run(atx, paused.clone(), audio_stop.clone(), start));
        let fwd = out_tx.clone();
        std::thread::Builder::new()
            .name("audio-forward".into())
            .spawn(move || {
                for t in arx {
                    if fwd.send(Msg::Packet(t)).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }

    report::started();
    let mut pacer = Pacer {
        mode: opts.fps_mode,
        fps: opts.fps,
        start,
        paused_total: Duration::ZERO,
        paused_since: None,
        tick: 0,
        last_pts: -1,
    };
    let mut packets = Vec::new();
    let mut seen_serial = 0u64;
    let mut last_frame: Option<Arc<Frame>> = first_frame;
    let mut last_encoded = Instant::now();
    let period = Duration::from_secs_f64(1.0 / opts.fps.max(1) as f64);
    let mut next_tick = Instant::now();
    let mut failure: Option<String> = None;
    let mut stats_frames = 0u64;
    let mut stats_since = Instant::now();

    'main: loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) => break 'main,
                Ok(Command::Save) => {
                    let _ = out_tx.send(Msg::Save(None));
                }
                Ok(Command::SaveSeconds(s)) => {
                    let _ = out_tx.send(Msg::Save(Some(s)));
                }
                Ok(cmd @ (Command::Pause | Command::Resume | Command::TogglePause)) => {
                    if replay || streaming {
                        continue;
                    }
                    let want = match cmd {
                        Command::Pause => true,
                        Command::Resume => false,
                        _ => !paused.load(Ordering::SeqCst),
                    };
                    if want != paused.load(Ordering::SeqCst) {
                        paused.store(want, Ordering::SeqCst);
                        pacer.pause(want);
                        report::paused(want);
                    }
                }
                Ok(Command::Mute(track, on)) => {
                    if let Some(m) = muted.get(track) {
                        m.store(on, Ordering::SeqCst);
                    }
                }
                Ok(Command::Marker) => {
                    let _ = out_tx.send(Msg::Marker);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if let Ok(Err(_)) = setup_rx.try_recv() {
            failure = Some("the output stopped".into());
            break;
        }

        let (Some(c), Some(enc)) = (&cap, video_enc.as_mut()) else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };

        if paused.load(Ordering::SeqCst) {
            if let Wait::Closed(r) = c.slot.wait_newer(u64::MAX, Duration::from_millis(30)) {
                failure = Some(r);
                break;
            }
            next_tick = Instant::now();
            continue;
        }

        let (frame, pts) = match pacer.mode {
            FpsMode::Cfr => {
                let now = Instant::now();
                if next_tick > now {
                    std::thread::sleep(next_tick - now);
                }
                next_tick += period;
                if Instant::now() > next_tick + period * 4 {
                    next_tick = Instant::now();
                }
                if c.slot.is_closed() {
                    if let Wait::Closed(r) = c.slot.wait_newer(u64::MAX, Duration::ZERO) {
                        failure = Some(r);
                    }
                    break;
                }
                if let Some((f, s)) = c.slot.latest() {
                    seen_serial = s;
                    last_frame = Some(f);
                }
                let Some(f) = last_frame.clone() else { continue };
                let pts = pacer.tick;
                pacer.tick += 1;
                (f, pts)
            }
            FpsMode::Vfr | FpsMode::Content => {
                let wait = if pacer.mode == FpsMode::Content {
                    Duration::from_millis(1000)
                } else {
                    Duration::from_millis(250)
                };
                match c.slot.wait_newer(seen_serial, wait) {
                    Wait::Frame(f, s) => {
                        seen_serial = s;
                        last_frame = Some(f.clone());
                        let pts = pacer.vfr_pts(f.captured);
                        (f, pts)
                    }
                    Wait::Timeout => {
                        if last_encoded.elapsed() < Duration::from_millis(900) {
                            continue;
                        }
                        let Some(f) = last_frame.clone() else { continue };
                        let pts = pacer.vfr_pts(Instant::now());
                        (f, pts)
                    }
                    Wait::Closed(r) => {
                        failure = Some(r);
                        break;
                    }
                }
            }
        };

        let encoded = match &c.webcam {
            Some(feed) => {
                let mut owned = (*frame).clone();
                feed.composite(&mut owned);
                enc.encode(&owned, pts, &mut packets)
            }
            None => enc.encode(&frame, pts, &mut packets),
        };
        if let Err(e) = encoded {
            failure = Some(format!("{e:#}"));
            break;
        }
        last_encoded = Instant::now();
        send_packets(&out_tx, 0, &mut packets);
        stats_frames += 1;
        if report::verbose() && stats_since.elapsed() >= Duration::from_secs(1) {
            let secs = stats_since.elapsed().as_secs_f64();
            report::info(&format!(
                "fps {:.1}, convert {:.2} ms, encode {:.2} ms",
                stats_frames as f64 / secs,
                enc.convert_time.as_secs_f64() * 1000.0 / stats_frames.max(1) as f64,
                enc.encode_time.as_secs_f64() * 1000.0 / stats_frames.max(1) as f64
            ));
            enc.convert_time = Duration::ZERO;
            enc.encode_time = Duration::ZERO;
            stats_frames = 0;
            stats_since = Instant::now();
        }
    }

    stop.store(true, Ordering::SeqCst);
    audio_stop.store(true, Ordering::SeqCst);
    if let Some(m) = mixer_thread {
        let _ = m.join();
    }
    if let Some(enc) = video_enc.as_mut() {
        let _ = enc.finish(&mut packets);
        send_packets(&out_tx, 0, &mut packets);
    }
    if let Some(c) = cap {
        c.shutdown();
    }
    for t in capture_threads {
        let _ = t.join();
    }
    std::thread::sleep(Duration::from_millis(50));
    let _ = out_tx.send(Msg::Finish);
    drop(out_tx);
    let _ = output_thread.join();
    if let Some(reason) = failure {
        if reason != "capture stopped" {
            report::error(&reason);
            report::finished();
            bail!("{reason}");
        }
    }
    report::finished();
    Ok(())
}
