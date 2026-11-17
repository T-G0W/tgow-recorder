use std::sync::mpsc::Sender;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip};
use tgow_common::config::Action;

use crate::daemon::Msg;
use crate::state::{dot_rgba, MenuCommand, MenuEntry, TrayState};

pub struct RecorderTray {
    tx: Sender<Msg>,
    state: TrayState,
}

fn dot(size: i32, rgb: [u8; 3], ring: bool) -> Icon {
    let data = dot_rgba(size, rgb, ring).into_iter().flat_map(|[r, g, b, a]| [a, r, g, b]).collect();
    Icon { width: size, height: size, data }
}

impl ksni::Tray for RecorderTray {
    fn id(&self) -> String {
        "tgow-recorder".into()
    }

    fn title(&self) -> String {
        "TGOW Recorder".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (rgb, ring) = self.state.look();
        vec![dot(22, rgb, ring), dot(32, rgb, ring), dot(48, rgb, ring)]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "TGOW Recorder".into(), description: self.state.summary().join("\n"), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Msg::Action(Action::ShowHide));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        self.state
            .menu()
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Separator => MenuItem::Separator,
                MenuEntry::Item(label, command) => StandardItem {
                    label,
                    icon_name: if command == MenuCommand::Quit { "application-exit".into() } else { String::new() },
                    activate: Box::new(move |this: &mut Self| {
                        let msg = match command {
                            MenuCommand::Run(action) => Msg::Action(action),
                            MenuCommand::Quit => Msg::Quit,
                        };
                        let _ = this.tx.send(msg);
                    }),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }
}

pub struct Tray {
    handle: Option<Handle<RecorderTray>>,
    state: TrayState,
}

impl Tray {
    pub fn new() -> Self {
        Self { handle: None, state: TrayState::default() }
    }

    pub fn set_visible(&mut self, visible: bool, tx: &Sender<Msg>) {
        if visible && self.handle.is_none() {
            let tray = RecorderTray { tx: tx.clone(), state: self.state };
            match tray.spawn() {
                Ok(handle) => self.handle = Some(handle),
                Err(err) => eprintln!("tgow-recorder: tray icon unavailable: {err}"),
            }
        } else if !visible {
            if let Some(handle) = self.handle.take() {
                handle.shutdown().wait();
            }
        }
    }

    pub fn update(&mut self, state: TrayState) {
        if state == self.state {
            return;
        }
        self.state = state;
        if let Some(handle) = &self.handle {
            handle.update(|t| t.state = state);
        }
    }

    pub fn refresh(&self) {
        if let Some(handle) = &self.handle {
            handle.update(|_| {});
        }
    }
}
