use std::sync::mpsc::Sender;

use tgow_common::config::{Action, Hotkey, HotkeyMode, MainConfig};

use crate::daemon::Msg;
use crate::windows::shell::{self, Cmd};

pub struct Hotkeys;

impl Hotkeys {
    pub fn new() -> Self {
        Self
    }

    pub fn apply(&mut self, main: &MainConfig, tx: &Sender<Msg>) {
        shell::post(Cmd::Bind(tx.clone()));
        let bindings: Vec<(Action, Hotkey)> = if main.hotkey_mode == HotkeyMode::Disabled {
            Vec::new()
        } else {
            main.hotkeys.bindings.iter().filter(|(_, h)| h.is_set()).cloned().collect()
        };
        shell::post(Cmd::Hotkeys(bindings));
    }
}
