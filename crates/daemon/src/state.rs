use tgow_common::config::Action;
use tgow_common::tr;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct TrayState {
    pub recording: bool,
    pub paused: bool,
    pub replay: bool,
    pub streaming: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Run(Action),
    Quit,
}

pub enum MenuEntry {
    Item(String, MenuCommand),
    Separator,
}

impl TrayState {
    pub fn look(&self) -> ([u8; 3], bool) {
        if self.paused {
            ([240, 190, 30], true)
        } else if self.recording || self.streaming {
            ([225, 40, 40], true)
        } else if self.replay {
            ([118, 185, 0], true)
        } else {
            ([150, 150, 150], false)
        }
    }

    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.recording {
            lines.push(if self.paused { tr("Paused") } else { tr("Recording") });
        }
        if self.replay {
            lines.push(format!("{}: {}", tr("Instant Replay"), tr("On")));
        }
        if self.streaming {
            lines.push(tr("Streaming"));
        }
        lines
    }

    pub fn menu(&self) -> Vec<MenuEntry> {
        let item = |label: String, action: Action| MenuEntry::Item(label, MenuCommand::Run(action));
        let mut items = vec![
            item(tr("Show the UI"), Action::ShowHide),
            MenuEntry::Separator,
            item(tr("Start/stop recording"), Action::ToggleRecord),
        ];
        if self.recording {
            items.push(item(if self.paused { tr("Unpause") } else { tr("Pause") }, Action::TogglePause));
        }
        items.push(item(tr("Turn replay on/off"), Action::ToggleReplay));
        if self.replay {
            items.push(item(tr("Save replay"), Action::SaveReplay));
        }
        items.push(item(tr("Start/stop streaming"), Action::ToggleStream));
        items.push(item(tr("Take a screenshot"), Action::Screenshot));
        items.push(MenuEntry::Separator);
        items.push(MenuEntry::Item(tr("Quit"), MenuCommand::Quit));
        items
    }
}

fn coverage(x: f32) -> f32 {
    (x + 0.5).clamp(0.0, 1.0)
}

pub fn dot_rgba(size: i32, rgb: [u8; 3], ring: bool) -> Vec<[u8; 4]> {
    let mut data = Vec::with_capacity((size * size) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    let outer = size as f32 * 0.47;
    let thickness = (size as f32 * 0.09).max(1.5);
    let inner = if ring { size as f32 * 0.27 } else { size as f32 * 0.34 };
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let fill = coverage(inner - d);
            let band = if ring { coverage(outer - d) * coverage(d - (outer - thickness)) } else { 0.0 };
            data.push(if fill > 0.0 {
                [rgb[0], rgb[1], rgb[2], (fill * 255.0) as u8]
            } else {
                [235, 235, 235, (band * 255.0) as u8]
            });
        }
    }
    data
}
