pub mod camera;
pub mod pattern;

#[cfg(unix)]
pub mod drm;
#[cfg(unix)]
pub mod portal;
#[cfg(unix)]
pub mod v4l2;
#[cfg(unix)]
pub mod x11;

#[cfg(windows)]
pub mod dxgi;
#[cfg(windows)]
pub mod mfcam;

use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::args::Options;
use crate::frame::FrameSlot;

pub struct Capture {
    pub slot: Arc<FrameSlot>,
    pub stop: Arc<AtomicBool>,
    pub label: String,
    pub webcam: Option<Arc<camera::Feed>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Capture {
    pub fn shutdown(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

#[cfg(unix)]
fn token_file(opts: &Options) -> std::path::PathBuf {
    opts.token_file.clone().unwrap_or_else(tgow_common::paths::restore_token_file)
}

pub fn start(opts: &Options) -> Result<Capture> {
    let source = opts.source.clone().unwrap_or_default();
    let slot = Arc::new(FrameSlot::default());
    let stop = Arc::new(AtomicBool::new(false));

    if let Some(size) = source.strip_prefix("testsrc") {
        let (w, h) = size
            .trim_start_matches(':')
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
            .unwrap_or((1280, 720));
        let h = pattern::spawn(w, h, opts.fps, slot.clone(), stop.clone());
        return Ok(finish(slot, stop, vec![h], "test pattern".into(), opts));
    }

    let (handle, label) = platform_start(&source, opts, slot.clone(), stop.clone())?;
    Ok(finish(slot, stop, vec![handle], label, opts))
}

type Started = (std::thread::JoinHandle<()>, String);

#[cfg(windows)]
fn platform_start(source: &str, opts: &Options, slot: Arc<FrameSlot>, stop: Arc<AtomicBool>) -> Result<Started> {
    let target = match source {
        "screen" | "screen-direct" => dxgi::Target::FirstMonitor,
        "focused_monitor" | "" | "portal" => dxgi::Target::FocusedMonitor,
        "focused" | "window" => dxgi::Target::FocusedWindow,
        "region" => match &opts.region {
            Some(r) => dxgi::Target::Area(r.x, r.y, r.width, r.height),
            None => anyhow::bail!("-w region needs -region WxH+X+Y"),
        },
        other => dxgi::Target::Monitor(other.to_string()),
    };
    let label = match target {
        dxgi::Target::Area(..) => "region",
        dxgi::Target::FocusedWindow => "window",
        _ => "monitor",
    };
    Ok((dxgi::spawn(target, opts.cursor, opts.fps, slot, stop)?, label.to_string()))
}

#[cfg(unix)]
fn platform_start(source: &str, opts: &Options, slot: Arc<FrameSlot>, stop: Arc<AtomicBool>) -> Result<Started> {
    let source = source.to_string();
    let started = if tgow_common::system::is_wayland() {
        let window = matches!(source.as_str(), "window" | "focused");
        let crop = if source == "region" {
            opts.region.as_ref().filter(|r| r.width > 0 && r.height > 0).map(|r| (r.x, r.y, r.width, r.height))
        } else {
            None
        };
        let restore = opts.restore_portal || source != "portal";
        let req = portal::PortalRequest {
            window,
            cursor: opts.cursor,
            restore,
            token_file: token_file(opts),
            fps: opts.fps,
            crop,
        };
        let label = match source.as_str() {
            "region" => "region",
            "window" | "focused" => "window",
            "portal" => "portal",
            _ => "monitor",
        };
        (portal::spawn(req, slot.clone(), stop.clone())?, label.to_string())
    } else {
        let target = match source.as_str() {
            "screen" | "screen-direct" => x11::Target::FirstMonitor,
            "focused_monitor" | "" => x11::Target::FocusedMonitor,
            "focused" => x11::Target::FocusedWindow,
            "region" => match &opts.region {
                Some(r) => x11::Target::Area(r.x, r.y, r.width, r.height),
                None => anyhow::bail!("-w region needs -region WxH+X+Y"),
            },
            "portal" => {
                let req = portal::PortalRequest {
                    window: false,
                    cursor: opts.cursor,
                    restore: opts.restore_portal,
                    token_file: token_file(opts),
                    fps: opts.fps,
                    crop: None,
                };
                return Ok((portal::spawn(req, slot, stop)?, "portal".into()));
            }
            "window" => x11::Target::FocusedWindow,
            other => {
                if let Some(id) = parse_window_id(other) {
                    x11::Target::Window(id)
                } else {
                    x11::Target::Monitor(other.to_string())
                }
            }
        };
        let label = match target {
            x11::Target::Area(..) => "region",
            x11::Target::Window(_) | x11::Target::FocusedWindow => "window",
            _ => "monitor",
        };
        (x11::spawn(target, opts.cursor, opts.fps, slot, stop)?, label.to_string())
    };
    Ok(started)
}

fn finish(
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    mut threads: Vec<std::thread::JoinHandle<()>>,
    label: String,
    opts: &Options,
) -> Capture {
    let mut feed = None;
    if let Some(spec) = opts.webcam.as_deref().filter(|s| !s.is_empty()) {
        match camera::start(spec, stop.clone()) {
            Ok((t, f)) => {
                threads.push(t);
                feed = Some(f);
            }
            Err(e) => crate::report::warning(&format!("webcam disabled: {e:#}")),
        }
    }
    Capture { slot, stop, label, webcam: feed, threads }
}

#[cfg(unix)]
fn parse_window_id(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        s.parse().ok()
    } else {
        None
    }
}
