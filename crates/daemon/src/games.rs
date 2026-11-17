use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::daemon::Msg;
use crate::platform;

pub fn quoted(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

pub fn manifest_value(text: &str, key: &str) -> Option<String> {
    text.lines().map(quoted).find(|p| p.len() == 2 && p[0].eq_ignore_ascii_case(key)).map(|p| p[1].to_string())
}

pub fn library_dirs(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in roots {
        let steamapps = root.join("steamapps");
        dirs.push(steamapps.canonicalize().unwrap_or_else(|_| steamapps.clone()));
        if let Ok(text) = std::fs::read_to_string(steamapps.join("libraryfolders.vdf")) {
            for line in text.lines() {
                let parts = quoted(line);
                if parts.len() == 2 && parts[0] == "path" {
                    let lib = Path::new(&parts[1].replace("\\\\", "\\")).join("steamapps");
                    dirs.push(lib.canonicalize().unwrap_or(lib));
                }
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

pub fn spawn(tx: Sender<Msg>) {
    std::thread::Builder::new()
        .name("games".into())
        .spawn(move || {
            let mut cache = HashMap::new();
            let mut last: Option<String> = None;
            loop {
                let now = platform::games::detect(&mut cache);
                if now != last {
                    last = now.clone();
                    if tx.send(Msg::Game(now)).is_err() {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_pairs() {
        assert_eq!(quoted("\t\t\"name\"\t\t\"Some Game\""), vec!["name", "Some Game"]);
        assert_eq!(
            manifest_value("\t\"installdir\"\t\t\"Hollow Knight\"", "installdir").as_deref(),
            Some("Hollow Knight")
        );
    }
}
