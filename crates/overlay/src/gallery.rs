use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Ui, Vec2};
use tgow_common::config::Config;
use tgow_common::{paths, system, tr, trf};

use crate::theme::{self, Theme};
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Recording,
    Replay,
    Screenshot,
}

impl Kind {
    fn label(self) -> String {
        match self {
            Kind::Recording => tr("Recordings"),
            Kind::Replay => tr("Replays"),
            Kind::Screenshot => tr("Screenshots"),
        }
    }

    fn color(self) -> Color32 {
        match self {
            Kind::Recording => Color32::from_rgb(220, 70, 70),
            Kind::Replay => Color32::from_rgb(90, 160, 240),
            Kind::Screenshot => Color32::from_rgb(230, 190, 70),
        }
    }
}

pub struct Entry {
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: u64,
}

#[derive(Default)]
pub struct Gallery {
    pub entries: Vec<Entry>,
    pub filter: Option<Kind>,
    pub confirm_delete: Option<PathBuf>,
    pub loaded: bool,
}

const VIDEO_EXT: [&str; 7] = ["mp4", "mkv", "mov", "webm", "flv", "ts", "mp3"];
const IMAGE_EXT: [&str; 3] = ["png", "jpg", "jpeg"];

fn collect(dir: &Path, kind: Kind, depth: u32, out: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth > 0 {
                collect(&path, kind, depth - 1, out);
            }
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let wanted = match kind {
            Kind::Screenshot => IMAGE_EXT.contains(&ext.as_str()),
            _ => VIDEO_EXT.contains(&ext.as_str()),
        };
        if !wanted {
            continue;
        }
        let modified =
            meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(Entry { path, kind, size: meta.len(), modified });
    }
}

fn date_text(secs: u64) -> String {
    let t = crate::platform::local_stamp(secs);
    format!("{:02}.{:02}.{:04} {:02}:{:02}", t.day, t.month, t.year, t.hour, t.minute)
}

impl Gallery {
    pub fn reload(&mut self, config: &Config) {
        let mut entries = Vec::new();
        let dirs = [
            (config.record.save_directory.as_str(), Kind::Recording),
            (config.replay.save_directory.as_str(), Kind::Replay),
            (config.screenshot.save_directory.as_str(), Kind::Screenshot),
        ];
        let mut seen: Vec<PathBuf> = Vec::new();
        for (dir, kind) in dirs {
            let dir = paths::expand_tilde(dir);
            if seen.contains(&dir) {
                continue;
            }
            seen.push(dir.clone());
            collect(&dir, kind, 1, &mut entries);
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
        entries.truncate(60);
        self.entries = entries;
        self.loaded = true;
        self.confirm_delete = None;
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme, config: &Config, free_space: Option<u64>) -> bool {
        if !self.loaded {
            self.reload(config);
        }
        ui.horizontal(|ui| {
            for (filter, label) in [
                (None, tr("All")),
                (Some(Kind::Recording), Kind::Recording.label()),
                (Some(Kind::Replay), Kind::Replay.label()),
                (Some(Kind::Screenshot), Kind::Screenshot.label()),
            ] {
                let selected = self.filter == filter;
                let text_width = ui.painter().layout_no_wrap(label.clone(), theme::font(16.0), theme::TEXT).size().x;
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new((text_width + 36.0).max(96.0), 38.0), Sense::click());
                let fill = if selected {
                    theme.accent.gamma_multiply(0.85)
                } else if response.hovered() {
                    theme::FIELD_HOVER
                } else {
                    theme::FIELD_FILL
                };
                ui.painter().rect_filled(rect, theme.radius(6), fill);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, theme::font(16.0), theme::TEXT);
                if response.clicked() {
                    self.filter = filter;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::plain_button(ui, theme, &tr("Refresh"), 140.0).clicked() {
                    self.reload(config);
                }
                let free =
                    free_space.or_else(|| paths::free_space_bytes(&paths::expand_tilde(&config.record.save_directory)));
                if let Some(bytes) = free {
                    let text = trf("Free space: %s", &[&system::human_bytes(bytes)]);
                    let needed = ui.painter().layout_no_wrap(text.clone(), theme::font(14.0), theme::TEXT).size().x;
                    if ui.available_width() > needed + 12.0 {
                        ui.label(RichText::new(text).color(theme::TEXT_DIM).size(14.0));
                    }
                }
            });
        });
        ui.add_space(10.0);

        let visible: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.filter.is_none_or(|f| f == e.kind))
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(tr("No recordings yet")).color(theme::TEXT_DIM).size(20.0));
            });
            return false;
        }

        let mut delete = None;
        let mut close = false;
        for index in visible {
            let entry = &self.entries[index];
            let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
            let fill = if response.hovered() { theme::PANEL_HOVER } else { theme::SECTION_FILL };
            ui.painter().rect_filled(rect, theme.radius(8), fill);
            ui.painter().rect_filled(
                Rect::from_min_size(rect.left_top(), Vec2::new(5.0, rect.height())),
                theme.radius(2),
                entry.kind.color(),
            );
            let name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.painter().text(
                rect.left_top() + Vec2::new(22.0, 12.0),
                Align2::LEFT_TOP,
                &name,
                theme::font(17.0),
                theme::TEXT,
            );
            ui.painter().text(
                rect.left_bottom() + Vec2::new(22.0, -12.0),
                Align2::LEFT_BOTTOM,
                format!("{}   {}   {}", entry.kind.label(), date_text(entry.modified), system::human_bytes(entry.size)),
                theme::font(13.0),
                theme::TEXT_DIM,
            );

            let mut x = rect.right() - 12.0;
            let mut button = |text: String, width: f32, danger: bool| -> bool {
                let b = Rect::from_min_size(egui::pos2(x - width, rect.center().y - 18.0), Vec2::new(width, 36.0));
                x -= width + 8.0;
                let r = ui.interact(b, ui.scope_id().with((&entry.path, &text)), Sense::click());
                let fill = if r.hovered() {
                    if danger {
                        theme::DANGER
                    } else {
                        theme::FIELD_HOVER
                    }
                } else {
                    theme::FIELD_FILL
                };
                ui.painter().rect_filled(b, theme.radius(5), fill);
                ui.painter().text(b.center(), Align2::CENTER_CENTER, text, theme::font(15.0), theme::TEXT);
                r.clicked()
            };
            let confirming = self.confirm_delete.as_deref() == Some(entry.path.as_path());
            if confirming {
                if button(tr("Cancel"), 110.0, false) {
                    self.confirm_delete = None;
                }
                if button(tr("Delete"), 110.0, true) {
                    delete = Some(index);
                }
            } else {
                if button(tr("Delete"), 100.0, true) {
                    self.confirm_delete = Some(entry.path.clone());
                }
                if button(tr("Open folder"), 150.0, false) {
                    system::reveal_path(&entry.path);
                    close = true;
                }
                if button(tr("Open"), 100.0, false) {
                    system::open_path(&entry.path);
                    close = true;
                }
            }
            ui.add_space(6.0);
        }

        if let Some(i) = delete {
            let path = self.entries[i].path.clone();
            if std::fs::remove_file(&path).is_ok() {
                self.entries.remove(i);
            }
            self.confirm_delete = None;
        }
        close
    }
}
