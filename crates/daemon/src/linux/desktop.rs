use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tgow_common::paths;

fn autostart_file() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| paths::home().join(".config"));
    base.join("autostart").join("tgow-recorder.desktop")
}

pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let file = autostart_file();
    if !enabled {
        if file.exists() {
            std::fs::remove_file(file)?;
        }
        return Ok(());
    }
    let exec = std::env::current_exe()
        .ok()
        .filter(|p| !p.to_string_lossy().contains("/target/"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tgow-recorder".into());
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=TGOW Recorder\nComment=Screen recorder overlay\nExec={exec}\nIcon=media-record\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-phase=2\n"
    );
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, entry)
}

fn tool_exists(name: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file())).unwrap_or(false)
}

pub fn copy_image(path: &Path) -> bool {
    let mime = if path.extension().map(|e| e == "png").unwrap_or(false) { "image/png" } else { "image/jpeg" };
    let Ok(bytes) = std::fs::read(path) else { return false };
    let mut cmd = if tgow_common::system::is_wayland() && tool_exists("wl-copy") {
        let mut c = Command::new("wl-copy");
        c.args(["--type", mime]);
        c
    } else if tool_exists("xclip") {
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard", "-t", mime, "-i"]);
        c
    } else {
        return false;
    };
    let Ok(mut child) = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let ok = child.stdin.take().map(|mut s| s.write_all(&bytes).is_ok()).unwrap_or(false);
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    ok
}

pub fn run_open_command(command: &str, path: &Path) {
    let command = command.trim();
    if command.is_empty() {
        return;
    }
    let script = format!("{command} \"$1\"");
    if let Ok(mut child) = Command::new("sh").arg("-c").arg(script).arg("sh").arg(path).stdin(Stdio::null()).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

pub fn blink_led() {
    std::thread::spawn(|| {
        tgow_common::system::set_scroll_lock_led(true);
        std::thread::sleep(std::time::Duration::from_millis(250));
        tgow_common::system::set_scroll_lock_led(false);
    });
}

pub fn allow_foreground() {}
