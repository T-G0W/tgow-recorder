use std::sync::mpsc::Sender;

use crate::daemon::Msg;
use crate::state::TrayState;
use crate::windows::shell::{self, Cmd};

pub struct Tray {
    state: TrayState,
}

impl Tray {
    pub fn new() -> Self {
        Self { state: TrayState::default() }
    }

    pub fn set_visible(&mut self, visible: bool, tx: &Sender<Msg>) {
        shell::post(Cmd::Bind(tx.clone()));
        shell::post(Cmd::TrayState(self.state));
        shell::post(Cmd::TrayVisible(visible));
    }

    pub fn update(&mut self, state: TrayState) {
        if state == self.state {
            return;
        }
        self.state = state;
        shell::post(Cmd::TrayState(state));
    }

    pub fn refresh(&self) {
        shell::post(Cmd::Refresh);
    }
}
