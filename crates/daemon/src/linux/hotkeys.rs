use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures_lite::StreamExt;
use tgow_common::config::{Action, Hotkey, HotkeyMode, MainConfig};
use tgow_common::tr;

use crate::daemon::Msg;

#[derive(Default)]
struct StopInner {
    stopped: bool,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
pub struct StopSignal(Arc<Mutex<StopInner>>);

impl StopSignal {
    pub fn stop(&self) {
        let mut inner = self.0.lock().unwrap();
        inner.stopped = true;
        if let Some(w) = inner.waker.take() {
            w.wake();
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.0.lock().unwrap().stopped
    }

    fn wait(&self) -> StopWait {
        StopWait(self.clone())
    }
}

struct StopWait(StopSignal);

impl Future for StopWait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut inner = (self.0).0.lock().unwrap();
        if inner.stopped {
            Poll::Ready(())
        } else {
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

pub struct Hotkeys {
    stop: Option<StopSignal>,
}

impl Hotkeys {
    pub fn new() -> Self {
        Self { stop: None }
    }

    pub fn apply(&mut self, main: &MainConfig, tx: &Sender<Msg>) {
        if let Some(stop) = self.stop.take() {
            stop.stop();
        }
        if main.hotkey_mode == HotkeyMode::Disabled {
            return;
        }
        let bindings: Vec<(Action, Hotkey)> =
            main.hotkeys.bindings.iter().filter(|(_, h)| h.is_set()).cloned().collect();
        if bindings.is_empty() {
            return;
        }
        let stop = StopSignal::default();
        let tx = tx.clone();
        let thread_stop = stop.clone();
        let wayland = tgow_common::system::is_wayland();
        let spawned = std::thread::Builder::new().name("hotkeys".into()).spawn(move || {
            let result =
                if wayland { run_portal(bindings, tx, thread_stop) } else { run_x11(bindings, tx, thread_stop) };
            if let Err(err) = result {
                eprintln!("tgow-recorder: global hotkeys unavailable: {err}");
            }
        });
        if spawned.is_ok() {
            self.stop = Some(stop);
        }
    }
}

fn xkb_key_name(key: &str) -> String {
    match key {
        "Enter" | "Return" => "Return".into(),
        "Space" => "space".into(),
        "ArrowLeft" | "Left" => "Left".into(),
        "ArrowRight" | "Right" => "Right".into(),
        "ArrowUp" | "Up" => "Up".into(),
        "ArrowDown" | "Down" => "Down".into(),
        "PageUp" => "Prior".into(),
        "PageDown" => "Next".into(),
        "Backspace" => "BackSpace".into(),
        "Minus" => "minus".into(),
        "Plus" | "Equals" => "equal".into(),
        "Comma" => "comma".into(),
        "Period" => "period".into(),
        "Slash" => "slash".into(),
        "Semicolon" => "semicolon".into(),
        "Print" | "PrintScreen" => "Print".into(),
        k if k.len() == 4 && k.starts_with("Num") => k[3..].to_string(),
        k if k.chars().count() == 1 => k.to_lowercase(),
        k => k.to_string(),
    }
}

fn portal_trigger(h: &Hotkey) -> String {
    let mut parts: Vec<String> = h
        .modifiers
        .iter()
        .map(|m| match m.as_str() {
            "Ctrl" => "CTRL".to_string(),
            "Shift" => "SHIFT".to_string(),
            "Alt" => "ALT".to_string(),
            "Super" => "LOGO".to_string(),
            other => other.to_uppercase(),
        })
        .collect();
    parts.push(xkb_key_name(&h.key));
    parts.join("+")
}

fn run_portal(bindings: Vec<(Action, Hotkey)>, tx: Sender<Msg>, stop: StopSignal) -> anyhow::Result<()> {
    use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};

    futures_lite::future::block_on(async move {
        if let Ok(id) = ashpd::AppID::try_from("com.tgow.Recorder") {
            let _ = ashpd::register_host_app(id).await;
        }
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session(Default::default()).await?;
        let triggers: Vec<String> = bindings.iter().map(|(_, h)| portal_trigger(h)).collect();
        let shortcuts: Vec<NewShortcut> = bindings
            .iter()
            .zip(triggers.iter())
            .map(|((action, _), trigger)| {
                NewShortcut::new(action.id(), tr(action.label_key())).preferred_trigger(trigger.as_str())
            })
            .collect();
        let request = portal.bind_shortcuts(&session, &shortcuts, None, BindShortcutsOptions::default()).await?;
        request.response()?;
        let mut activated = portal.receive_activated().await?;
        loop {
            let next = futures_lite::future::or(async { activated.next().await.map(Some) }, async {
                stop.wait().await;
                None
            })
            .await;
            match next {
                Some(Some(event)) => {
                    if let Some(action) = Action::from_id(event.shortcut_id()) {
                        if tx.send(Msg::Action(action)).is_err() {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        let _ = session.close().await;
        Ok(())
    })
}

fn keysym_for(key: &str) -> Option<u32> {
    let k = match key {
        "Enter" | "Return" => 0xff0d,
        "Space" => 0x20,
        "Escape" => 0xff1b,
        "Tab" => 0xff09,
        "Backspace" => 0xff08,
        "Insert" => 0xff63,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "ArrowLeft" | "Left" => 0xff51,
        "ArrowUp" | "Up" => 0xff52,
        "ArrowRight" | "Right" => 0xff53,
        "ArrowDown" | "Down" => 0xff54,
        "Print" | "PrintScreen" => 0xff61,
        "Pause" => 0xff13,
        "ScrollLock" => 0xff14,
        "Minus" => 0x2d,
        "Comma" => 0x2c,
        "Period" => 0x2e,
        "Slash" => 0x2f,
        _ => {
            if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
                if (1..=35).contains(&n) {
                    return Some(0xffbe + n - 1);
                }
            }
            let key = key.strip_prefix("Num").filter(|k| k.len() == 1).unwrap_or(key);
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if c.is_ascii_alphabetic() {
                c.to_ascii_lowercase() as u32
            } else if c.is_ascii_graphic() {
                c as u32
            } else {
                return None;
            }
        }
    };
    Some(k)
}

fn modifier_mask(h: &Hotkey) -> u16 {
    let mut mask = 0u16;
    for m in &h.modifiers {
        mask |= match m.as_str() {
            "Shift" => 1,
            "Ctrl" => 4,
            "Alt" => 8,
            "Super" => 64,
            _ => 0,
        };
    }
    mask
}

fn run_x11(bindings: Vec<(Action, Hotkey)>, tx: Sender<Msg>, stop: StopSignal) -> anyhow::Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    use x11rb::protocol::Event;

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let min = conn.setup().min_keycode;
    let max = conn.setup().max_keycode;
    let mapping = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = mapping.keysyms_per_keycode as usize;
    let keycode_of = |sym: u32| -> Option<u8> {
        mapping.keysyms.chunks(per.max(1)).position(|chunk| chunk.contains(&sym)).map(|i| min + i as u8)
    };

    let mut grabs: Vec<(u8, u16, Action)> = Vec::new();
    for (action, hotkey) in &bindings {
        let Some(sym) = keysym_for(&hotkey.key) else { continue };
        let Some(code) = keycode_of(sym) else { continue };
        let mods = modifier_mask(hotkey);
        for extra in [0u16, 2, 16, 18] {
            let _ = conn.grab_key(false, root, ModMask::from(mods | extra), code, GrabMode::ASYNC, GrabMode::ASYNC);
        }
        grabs.push((code, mods, *action));
    }
    conn.flush()?;

    let relevant: u16 = 1 | 4 | 8 | 64;
    while !stop.is_stopped() {
        match conn.poll_for_event()? {
            Some(Event::KeyPress(ev)) => {
                let state = u16::from(ev.state) & relevant;
                if let Some((_, _, action)) = grabs.iter().find(|(code, mods, _)| *code == ev.detail && *mods == state)
                {
                    if tx.send(Msg::Action(*action)).is_err() {
                        break;
                    }
                }
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    for (code, mods, _) in &grabs {
        for extra in [0u16, 2, 16, 18] {
            let _ = conn.ungrab_key(*code, root, ModMask::from(mods | extra));
        }
    }
    let _ = conn.flush();
    Ok(())
}
