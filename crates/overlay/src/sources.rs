use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use eframe::egui;
use tgow_common::paths;

#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub description: String,
    pub output: bool,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub loaded: bool,
    pub monitors: Vec<Monitor>,
    pub devices: Vec<AudioDevice>,
    pub apps: Vec<String>,
    pub cameras: Vec<Camera>,
}

impl Sources {
    pub fn outputs(&self) -> impl Iterator<Item = &AudioDevice> {
        self.devices.iter().filter(|d| d.output)
    }

    pub fn inputs(&self) -> impl Iterator<Item = &AudioDevice> {
        self.devices.iter().filter(|d| !d.output)
    }

    pub fn device_label(&self, id: &str) -> String {
        match id {
            "default_output" => "Default output".to_string(),
            "default_input" => "Default input".to_string(),
            _ => self
                .devices
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.description.clone())
                .unwrap_or_else(|| id.to_string()),
        }
    }
}

pub type SharedSources = Arc<Mutex<Sources>>;

fn run_lines(args: &[&str]) -> Vec<String> {
    let bin = paths::sibling_binary("tgow-rec");
    let Ok(out) = crate::platform::hide_console(&mut Command::new(bin))
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('@'))
        .collect()
}

#[cfg(unix)]
fn cameras() -> Vec<Camera> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return found;
    };
    for entry in entries.flatten() {
        let dev = entry.file_name().to_string_lossy().into_owned();
        let name = std::fs::read_to_string(entry.path().join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| dev.clone());
        let index =
            std::fs::read_to_string(entry.path().join("index")).map(|s| s.trim().to_string()).unwrap_or_default();
        if index == "0" || index.is_empty() {
            found.push(Camera { path: format!("/dev/{dev}"), name });
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

#[cfg(windows)]
fn cameras() -> Vec<Camera> {
    run_lines(&["--list-cameras"])
        .into_iter()
        .filter_map(|line| {
            let (path, name) = line.split_once('|')?;
            Some(Camera { path: path.to_string(), name: name.to_string() })
        })
        .collect()
}

pub fn load(ctx: egui::Context) -> SharedSources {
    let shared: SharedSources = Arc::new(Mutex::new(Sources::default()));
    let target = shared.clone();
    std::thread::spawn(move || {
        let monitors = run_lines(&["--list-monitors"])
            .into_iter()
            .filter_map(|line| {
                let mut parts = line.split('|');
                let name = parts.next()?.to_string();
                let (w, h) = parts.next()?.split_once('x')?;
                Some(Monitor { name, width: w.parse().ok()?, height: h.parse().ok()? })
            })
            .collect();
        let devices = run_lines(&["--list-audio-devices"])
            .into_iter()
            .filter_map(|line| {
                let mut parts = line.split('|');
                let id = parts.next()?.to_string();
                let description = parts.next().unwrap_or(&id).to_string();
                let output = parts.next().map(|k| k.trim() == "output").unwrap_or(false);
                if id == "default_output" || id == "default_input" {
                    return None;
                }
                Some(AudioDevice { id, description, output })
            })
            .collect();
        let mut apps = run_lines(&["--list-application-audio"]);
        apps.sort_by_key(|a| a.to_lowercase());
        apps.dedup();
        let sources = Sources { loaded: true, monitors, devices, apps, cameras: cameras() };
        if let Ok(mut s) = target.lock() {
            *s = sources;
        }
        ctx.request_repaint();
    });
    shared
}
