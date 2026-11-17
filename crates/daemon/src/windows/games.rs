use std::collections::HashMap;
use std::path::PathBuf;

use windows::core::{w, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::games::{library_dirs, manifest_value};

const HELPERS: &[&str] = &[
    "steam",
    "steamwebhelper",
    "steamerrorreporter",
    "steamservice",
    "gameoverlayui",
    "epicgameslauncher",
    "epicwebhelper",
    "epiconlineservices",
    "eosoverlayrenderer-win64-shipping",
    "galaxyclient",
    "galaxyclient helper",
    "gogsupportservice",
    "crashpad_handler",
    "crashreportclient",
    "unitycrashhandler64",
    "unitycrashhandler32",
    "easyanticheat",
    "easyanticheat_eos",
    "easyanticheat_eos_setup",
    "battleye",
    "beservice",
    "uninstall",
    "unins000",
    "setup",
    "installer",
    "vc_redist.x64",
    "vc_redist.x86",
    "dxsetup",
];

const STORES: &[&str] = &["\\steamapps\\common\\", "\\epic games\\", "\\gog galaxy\\games\\", "\\xboxgames\\"];

fn steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Valve\\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if rc.is_ok() {
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        roots.push(PathBuf::from(String::from_utf16_lossy(&buf[..len]).replace('/', "\\")));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(pf).join("Steam"));
    }
    roots.retain(|p| p.join("steamapps").is_dir());
    roots.dedup();
    roots
}

fn steam_name(installdir: &str) -> Option<String> {
    for dir in library_dirs(steam_roots()) {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            if manifest_value(&text, "installdir").map(|d| d.eq_ignore_ascii_case(installdir)).unwrap_or(false) {
                return manifest_value(&text, "name");
            }
        }
    }
    None
}

fn image_path(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(process);
        ok.then(|| String::from_utf16_lossy(&buf[..size as usize]))
    }
}

fn game_of(path: &str, cache: &mut HashMap<String, String>) -> Option<String> {
    let lower = path.to_lowercase();
    let stem = lower.rsplit('\\').next()?.strip_suffix(".exe")?;
    if HELPERS.contains(&stem) || stem.contains("crashhandler") || stem.contains("crashreport") {
        return None;
    }
    for store in STORES {
        let Some(at) = lower.find(store) else { continue };
        let start = at + store.len();
        let dir = path[start..].split('\\').next()?;
        if dir.is_empty() || dir.eq_ignore_ascii_case("launcher") {
            return None;
        }
        if let Some(name) = cache.get(dir) {
            return Some(name.clone());
        }
        let name = if *store == STORES[0] { steam_name(dir) } else { None }.unwrap_or_else(|| dir.to_string());
        cache.insert(dir.to_string(), name.clone());
        return Some(name);
    }
    None
}

pub fn detect(cache: &mut HashMap<String, String>) -> Option<String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            if let Some(path) = image_path(entry.th32ProcessID) {
                if let Some(game) = game_of(&path, cache) {
                    found = Some(game);
                    break;
                }
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_paths() {
        let mut cache = HashMap::new();
        assert_eq!(
            game_of(
                "C:\\Program Files\\Epic Games\\Fortnite\\FortniteGame\\Binaries\\Win64\\FortniteClient.exe",
                &mut cache
            )
            .as_deref(),
            Some("Fortnite")
        );
        assert_eq!(game_of("C:\\Program Files (x86)\\Steam\\steam.exe", &mut cache), None);
        assert_eq!(game_of("C:\\Windows\\explorer.exe", &mut cache), None);
    }
}
