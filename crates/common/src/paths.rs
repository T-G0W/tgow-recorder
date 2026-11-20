use std::path::{Path, PathBuf};

use crate::platform;

pub const APP_ID: &str = "tgow-recorder";

pub fn home() -> PathBuf {
    platform::home()
}

pub fn config_dir() -> PathBuf {
    platform::config_base().join(APP_ID)
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn restore_token_file() -> PathBuf {
    config_dir().join("restore_token")
}

pub fn runtime_dir() -> PathBuf {
    platform::runtime_base().join(APP_ID)
}

pub fn daemon_socket() -> PathBuf {
    runtime_dir().join("daemon.sock")
}

pub fn daemon_port_file() -> PathBuf {
    runtime_dir().join("daemon.port")
}

pub fn replay_cache_dir() -> PathBuf {
    platform::cache_base().join(APP_ID).join("replay")
}

pub fn log_file() -> PathBuf {
    platform::cache_base().join(APP_ID).join("tgow-recorder.log")
}

pub fn videos_dir() -> PathBuf {
    platform::videos_dir()
}

pub fn pictures_dir() -> PathBuf {
    platform::pictures_dir()
}

pub fn sibling_binary(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(format!("{name}{}", platform::EXE_SUFFIX));
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from(format!("{name}{}", platform::EXE_SUFFIX))
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home().join(rest)
    } else if path == "~" {
        home()
    } else {
        PathBuf::from(path)
    }
}

pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    platform::free_space_bytes(&probe)
}
