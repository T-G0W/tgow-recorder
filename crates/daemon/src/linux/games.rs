use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tgow_common::paths;

use crate::games::{library_dirs, manifest_value};

const WINE_HELPERS: &[&str] = &[
    "wineserver",
    "services",
    "explorer",
    "winedevice",
    "plugplay",
    "svchost",
    "rpcss",
    "conhost",
    "start",
    "rundll32",
    "winemenubuilder",
    "tabtip",
    "steam",
    "steamwebhelper",
    "steamerrorreporter",
    "crashpad_handler",
    "unitycrashhandler64",
    "unitycrashhandler32",
    "eosoverlayrenderer-win64-shipping",
    "easyanticheat",
    "easyanticheat_eos_setup",
    "battleye",
    "beservice",
    "uninstall",
    "setup",
    "installer",
    "vc_redist.x64",
    "vc_redist.x86",
    "dxsetup",
];

fn steam_roots() -> Vec<PathBuf> {
    let home = paths::home();
    let mut roots = vec![
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    roots.retain(|p| p.join("steamapps").is_dir());
    roots
}

fn steam_name(app_id: &str, cache: &mut HashMap<String, String>) -> String {
    if let Some(n) = cache.get(app_id) {
        return n.clone();
    }
    let mut name = String::new();
    for dir in library_dirs(steam_roots()) {
        let Ok(text) = std::fs::read_to_string(dir.join(format!("appmanifest_{app_id}.acf"))) else { continue };
        if let Some(found) = manifest_value(&text, "name") {
            name = found;
            break;
        }
    }
    if name.is_empty() {
        name = format!("Steam {app_id}");
    }
    cache.insert(app_id.to_string(), name.clone());
    name
}

fn own_uid() -> u32 {
    unsafe { libc::getuid() }
}

fn owned_by_me(pid_dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(pid_dir).map(|m| m.uid() == own_uid()).unwrap_or(false)
}

fn exe_stem(arg0: &str) -> Option<String> {
    let base = arg0.rsplit(['\\', '/']).next()?;
    let lower = base.to_lowercase();
    let stem = lower.strip_suffix(".exe")?;
    if WINE_HELPERS.contains(&stem) {
        return None;
    }
    Some(base[..stem.len()].to_string())
}

pub fn detect(cache: &mut HashMap<String, String>) -> Option<String> {
    let entries = std::fs::read_dir("/proc").ok()?;
    let mut wine_game = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let dir = entry.path();
        if !owned_by_me(&dir) {
            continue;
        }
        let Ok(raw) = std::fs::read(dir.join("cmdline")) else { continue };
        if raw.is_empty() {
            continue;
        }
        let args: Vec<String> = raw.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        if args.iter().any(|a| a == "SteamLaunch") {
            if let Some(id) = args.iter().find_map(|a| a.strip_prefix("AppId=")) {
                if id != "0" && !id.is_empty() {
                    return Some(steam_name(id, cache));
                }
            }
        }
        if wine_game.is_none() {
            if let Some(stem) = args.first().and_then(|a| exe_stem(a)) {
                wine_game = Some(stem);
            }
        }
    }
    wine_game
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_names() {
        assert_eq!(exe_stem("Z:\\games\\Hollow\\HollowKnight.exe").as_deref(), Some("HollowKnight"));
        assert_eq!(exe_stem("C:\\windows\\system32\\services.exe"), None);
        assert_eq!(exe_stem("/usr/bin/firefox"), None);
    }
}
