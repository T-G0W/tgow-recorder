use std::path::{Path, PathBuf};

use crate::system::LocalTime;

pub const EXE_SUFFIX: &str = "";

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

pub fn config_base() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn cache_base() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}

pub fn runtime_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn user_dir(key: &str, fallback: &str) -> PathBuf {
    if let Ok(text) = std::fs::read_to_string(config_base().join("user-dirs.dirs")) {
        let wanted = format!("XDG_{key}_DIR=");
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix(&wanted) {
                let value = rest.trim_matches('"').replace("$HOME", &home().to_string_lossy());
                if !value.is_empty() {
                    return PathBuf::from(value);
                }
            }
        }
    }
    home().join(fallback)
}

pub fn videos_dir() -> PathBuf {
    user_dir("VIDEOS", "Videos")
}

pub fn pictures_dir() -> PathBuf {
    user_dir("PICTURES", "Pictures")
}

pub fn free_space_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

pub fn now_local() -> LocalTime {
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    }
}

pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
}

pub fn gpu_vendors() -> Vec<&'static str> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        if let Ok(v) = std::fs::read_to_string(entry.path().join("device/vendor")) {
            found.push(crate::system::vendor_name(
                u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).unwrap_or(0),
            ));
        }
    }
    found
}

pub fn on_ac_power() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return true;
    };
    let mut has_battery = false;
    for entry in entries.flatten() {
        let p = entry.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "Mains" | "USB" => {
                if std::fs::read_to_string(p.join("online")).map(|v| v.trim() == "1").unwrap_or(false) {
                    return true;
                }
            }
            "Battery" => has_battery = true,
            _ => {}
        }
    }
    !has_battery
}

pub fn set_scroll_lock_led(on: bool) {
    let Ok(entries) = std::fs::read_dir("/sys/class/leds") else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().ends_with("::scrolllock") {
            let _ = std::fs::write(entry.path().join("brightness"), if on { "1" } else { "0" });
        }
    }
}

pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn reveal_path(path: &Path) {
    open_path(path.parent().unwrap_or(path));
}
