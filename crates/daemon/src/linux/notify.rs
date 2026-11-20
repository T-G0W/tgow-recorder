use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};

use tgow_common::ipc::ToastKind;
use zbus::zvariant::Value;

pub struct Note {
    pub kind: ToastKind,
    pub text: String,
    pub timeout_ms: i32,
}

#[derive(Clone)]
pub struct Notifier {
    tx: Sender<Note>,
}

fn icon_for(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Record => "media-record",
        ToastKind::Replay => "media-playlist-repeat",
        ToastKind::Stream => "network-transmit",
        ToastKind::Screenshot => "camera-photo",
        ToastKind::Error => "dialog-error",
        ToastKind::Info => "dialog-information",
    }
}

impl Notifier {
    pub fn close(&self) {}

    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Note>();
        std::thread::Builder::new()
            .name("notify".into())
            .spawn(move || {
                let mut conn: Option<zbus::blocking::Connection> = None;
                let mut last_id: u32 = 0;
                for note in rx {
                    if conn.is_none() {
                        conn = zbus::blocking::Connection::session().ok();
                    }
                    let Some(c) = conn.as_ref() else { continue };
                    let mut hints: HashMap<&str, Value> = HashMap::new();
                    hints.insert("transient", Value::from(true));
                    let urgency: u8 = if note.kind == ToastKind::Error { 2 } else { 1 };
                    hints.insert("urgency", Value::from(urgency));
                    let actions: Vec<&str> = Vec::new();
                    let body = (
                        "TGOW Recorder",
                        last_id,
                        icon_for(note.kind),
                        "TGOW Recorder",
                        note.text.as_str(),
                        actions,
                        hints,
                        note.timeout_ms,
                    );
                    let reply = c.call_method(
                        Some("org.freedesktop.Notifications"),
                        "/org/freedesktop/Notifications",
                        Some("org.freedesktop.Notifications"),
                        "Notify",
                        &body,
                    );
                    match reply {
                        Ok(msg) => {
                            if let Ok(id) = msg.body().deserialize::<u32>() {
                                last_id = id;
                            }
                        }
                        Err(err) => {
                            eprintln!("tgow-recorder: notification failed: {err}");
                            conn = None;
                        }
                    }
                }
            })
            .expect("notification thread");
        Self { tx }
    }

    pub fn send(&self, kind: ToastKind, text: &str, timeout_ms: i32) {
        let _ = self.tx.send(Note { kind, text: text.to_string(), timeout_ms });
    }
}
