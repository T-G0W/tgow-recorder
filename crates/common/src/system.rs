use std::path::{Path, PathBuf};

use crate::config::YesNoGames;
use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

pub fn now_local() -> LocalTime {
    platform::now_local()
}

impl LocalTime {
    pub fn file_stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    pub fn folder_stamp(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

pub fn is_wayland() -> bool {
    platform::is_wayland()
}

pub fn vendor_name(id: u32) -> &'static str {
    match id {
        0x10de => "nvidia",
        0x1002 | 0x1022 => "amd",
        0x8086 => "intel",
        _ => "unknown",
    }
}

pub fn gpu_vendor() -> String {
    let found = platform::gpu_vendors();
    for preferred in ["nvidia", "amd", "intel"] {
        if found.contains(&preferred) {
            return preferred.into();
        }
    }
    "unknown".into()
}

pub fn sanitize_file_part(s: &str) -> String {
    let cleaned: String =
        s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    let trimmed = cleaned.trim().trim_matches('_').to_string();
    if trimmed.is_empty() {
        "Video".into()
    } else {
        trimmed.chars().take(64).collect()
    }
}

pub fn output_path(
    base_dir: &Path,
    prefix: &str,
    extension: &str,
    app_name: Option<&str>,
    is_game: bool,
    folders: YesNoGames,
    name_after_app: YesNoGames,
) -> PathBuf {
    let now = now_local();
    let use_app = |choice: YesNoGames| match choice {
        YesNoGames::No => false,
        YesNoGames::Yes => app_name.is_some(),
        YesNoGames::GamesOnly => is_game && app_name.is_some(),
    };
    let mut dir = base_dir.to_path_buf();
    if use_app(folders) {
        dir = dir.join(sanitize_file_part(app_name.unwrap_or_default()));
    } else if folders != YesNoGames::No {
        dir = dir.join(now.folder_stamp());
    }
    let stem =
        if use_app(name_after_app) { sanitize_file_part(app_name.unwrap_or_default()) } else { prefix.to_string() };
    let mut path = dir.join(format!("{stem}_{}.{extension}", now.file_stamp()));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}_{}_{n}.{extension}", now.file_stamp()));
        n += 1;
    }
    path
}

pub fn on_ac_power() -> bool {
    platform::on_ac_power()
}

pub fn set_scroll_lock_led(on: bool) {
    platform::set_scroll_lock_led(on)
}

pub fn open_path(path: &Path) {
    platform::open_path(path)
}

pub fn reveal_path(path: &Path) {
    platform::reveal_path(path)
}

pub fn human_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.1} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}
