use tgow_common::ipc::ToastKind;

use crate::windows::shell::{self, Cmd};

#[derive(Clone)]
pub struct Notifier;

impl Notifier {
    pub fn start() -> Self {
        Self
    }

    pub fn send(&self, kind: ToastKind, text: &str, timeout_ms: i32) {
        shell::post(Cmd::Balloon {
            error: kind == ToastKind::Error,
            text: text.to_string(),
            timeout_ms: timeout_ms.max(500) as u32,
        });
    }
}
