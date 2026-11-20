use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tgow_common::{paths, VERSION};

use crate::daemon::Msg;

const LATEST: &str = "https://api.github.com/repos/T-G0W/tgow-recorder/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/T-G0W/tgow-recorder/releases/latest";
const FIRST_CHECK: Duration = Duration::from_secs(20);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Setup,
    Archive,
}

#[derive(Debug, Clone)]
pub struct Ready {
    pub version: String,
    pub file: PathBuf,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub enum Found {
    Available(String),
    Ready(Ready),
}

pub fn spawn(tx: Sender<Msg>, enabled: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new().name("update".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        cleanup();
        loop {
            if enabled.load(Ordering::Relaxed) {
                if let Some(found) = check() {
                    if tx.send(Msg::Update(found)).is_err() {
                        return;
                    }
                }
            }
            std::thread::sleep(INTERVAL);
        }
    });
}

fn dir() -> PathBuf {
    paths::log_file().with_file_name("update")
}

fn cleanup() {
    let Ok(entries) = std::fs::read_dir(dir()) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        let version = name.strip_prefix("tgow-recorder-").and_then(|rest| rest.split('-').next()).unwrap_or("");
        if name.ends_with(".part") || (!name.starts_with("updated-to") && !newer(version)) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn marker() -> PathBuf {
    dir().join("updated-to")
}

fn newer(version: &str) -> bool {
    match (semver::Version::parse(version), semver::Version::parse(VERSION)) {
        (Ok(remote), Ok(local)) => remote > local,
        _ => false,
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

fn install_kind() -> Option<Kind> {
    let dir = exe_dir()?;
    if cfg!(windows) {
        return dir.join("unins000.exe").is_file().then_some(Kind::Setup);
    }
    let text = dir.to_string_lossy();
    if text.starts_with("/usr/") || text.starts_with("/opt/") || text.contains("/target/") {
        return None;
    }
    let probe = dir.join(".tgow-update-probe");
    let writable = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    writable.then_some(Kind::Archive)
}

fn asset_suffix(kind: Kind) -> &'static str {
    match kind {
        Kind::Setup => "-windows-x64-setup.exe",
        Kind::Archive => "-linux-x86_64.tar.gz",
    }
}

fn curl() -> Command {
    let mut command = if cfg!(windows) {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        Command::new(system.join("System32").join("curl.exe"))
    } else {
        Command::new("curl")
    };
    command.args(["-fsSL", "--proto", "=https", "--tlsv1.2", "-A", "tgow-recorder"]);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn fetch_json(url: &str) -> Option<serde_json::Value> {
    let output = curl()
        .args(["--max-time", "30", "-H", "Accept: application/vnd.github+json", url])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    serde_json::from_slice(&output.stdout).ok()
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn verified(file: &Path) -> bool {
    let expected = std::fs::read_to_string(file.with_extension("sha256")).unwrap_or_default();
    !expected.trim().is_empty() && sha256_of(file).is_some_and(|actual| actual == expected.trim())
}

fn download(url: &str, name: &str, digest: &str) -> Option<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir).ok()?;
    let target = dir.join(name);
    if target.is_file() && verified(&target) {
        return Some(target);
    }
    let partial = dir.join(format!("{name}.part"));
    let ok =
        curl().args(["--max-time", "900", "-o"]).arg(&partial).arg(url).status().map(|s| s.success()).unwrap_or(false);
    if !ok || sha256_of(&partial).as_deref() != Some(digest) {
        let _ = std::fs::remove_file(&partial);
        crate::daemon::log_line("tgow-recorder: the downloaded update did not match its checksum");
        return None;
    }
    std::fs::rename(&partial, &target).ok()?;
    std::fs::write(target.with_extension("sha256"), digest).ok()?;
    Some(target)
}

pub fn check() -> Option<Found> {
    let release = fetch_json(LATEST)?;
    if release["draft"].as_bool() == Some(true) || release["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if !newer(&version) {
        return None;
    }
    let Some(kind) = install_kind() else { return Some(Found::Available(version)) };
    let suffix = asset_suffix(kind);
    let asset = release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str().is_some_and(|n| n.starts_with("tgow-recorder-") && n.ends_with(suffix)))?;
    let name = asset["name"].as_str()?;
    let url = asset["browser_download_url"].as_str()?;
    if !url.starts_with("https://github.com/") {
        return None;
    }
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?.to_ascii_lowercase();
    let file = download(url, name, &digest)?;
    Some(Found::Ready(Ready { version, file, kind }))
}

pub fn pending() -> Option<Ready> {
    let kind = install_kind()?;
    let suffix = asset_suffix(kind);
    let entries = std::fs::read_dir(dir()).ok()?;
    let mut best: Option<Ready> = None;
    for entry in entries.flatten() {
        let file = entry.path();
        let name = file.file_name()?.to_string_lossy().into_owned();
        let Some(version) = name.strip_prefix("tgow-recorder-").and_then(|n| n.strip_suffix(suffix)) else {
            continue;
        };
        if !newer(version) || !verified(&file) {
            continue;
        }
        let better = best
            .as_ref()
            .is_none_or(|b| semver::Version::parse(version).ok() > semver::Version::parse(&b.version).ok());
        if better {
            best = Some(Ready { version: version.to_string(), file, kind });
        }
    }
    best
}

pub fn take_finished() -> Option<String> {
    let path = marker();
    let version = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    (version.trim() == VERSION).then(|| VERSION.to_string())
}

pub fn apply(ready: &Ready) -> bool {
    let _ = std::fs::write(marker(), &ready.version);
    let ok = match ready.kind {
        Kind::Setup => run_setup(&ready.file),
        Kind::Archive => replace_files(&ready.file),
    };
    if !ok {
        let _ = std::fs::remove_file(marker());
        let _ = std::fs::remove_file(&ready.file);
        let _ = std::fs::remove_file(ready.file.with_extension("sha256"));
    }
    ok
}

#[cfg(windows)]
fn run_setup(file: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new(file)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/update=1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0000_0008 | 0x0000_0200)
        .spawn()
        .is_ok()
}

#[cfg(unix)]
fn run_setup(_file: &Path) -> bool {
    false
}

#[cfg(unix)]
fn replace_files(archive: &Path) -> bool {
    use std::os::unix::process::CommandExt;
    let Some(target) = exe_dir() else { return false };
    let work = dir().join("unpacked");
    let _ = std::fs::remove_dir_all(&work);
    if std::fs::create_dir_all(&work).is_err() {
        return false;
    }
    let unpacked = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(&work)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let source = work.join("tgow-recorder");
    let programs = ["tgow-recorder", "tgow-rec", "tgow-overlay"];
    if !unpacked || !programs.iter().all(|p| source.join(p).is_file()) {
        let _ = std::fs::remove_dir_all(&work);
        return false;
    }
    let extras = ["tgow-recorder.svg", "logo-512.png", "README.md", "THIRD-PARTY-LICENSES.txt", "install.sh"];
    for name in programs.iter().chain(extras.iter()) {
        let from = source.join(name);
        if !from.is_file() {
            continue;
        }
        let staged = target.join(format!(".{name}.new"));
        if std::fs::copy(&from, &staged).is_err() || std::fs::rename(&staged, target.join(name)).is_err() {
            let _ = std::fs::remove_file(&staged);
            let _ = std::fs::remove_dir_all(&work);
            return false;
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    Command::new("sh")
        .arg("-c")
        .arg("sleep 2; exec \"$0\"")
        .arg(target.join("tgow-recorder"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .is_ok()
}

#[cfg(windows)]
fn replace_files(_archive: &Path) -> bool {
    false
}
