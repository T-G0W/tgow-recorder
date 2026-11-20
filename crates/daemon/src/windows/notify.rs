use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use tgow_common::ipc::ToastKind;
use tgow_common::paths;

#[derive(Clone)]
pub struct Notifier {
    current: Arc<Mutex<Option<Child>>>,
}

impl Notifier {
    pub fn start() -> Self {
        Self { current: Arc::new(Mutex::new(None)) }
    }

    pub fn send(&self, kind: ToastKind, text: &str, timeout_ms: i32) {
        let Ok(mut current) = self.current.lock() else { return };
        if let Some(mut old) = current.take() {
            if matches!(old.try_wait(), Ok(None)) {
                let _ = old.kill();
            }
            let _ = old.wait();
        }
        let child = Command::new(paths::sibling_binary("tgow-overlay"))
            .arg("--toast")
            .arg(kind.id())
            .arg(timeout_ms.max(500).to_string())
            .arg(text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn();
        *current = child.ok();
    }
}
