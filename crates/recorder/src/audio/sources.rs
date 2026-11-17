use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

pub struct SourceBuffer {
    pub samples: Mutex<VecDeque<f32>>,
    pub primed: AtomicBool,
}

impl SourceBuffer {
    pub fn new() -> Arc<SourceBuffer> {
        Arc::new(SourceBuffer {
            samples: Mutex::new(VecDeque::with_capacity(RATE as usize)),
            primed: AtomicBool::new(false),
        })
    }
}

#[derive(Default)]
pub struct TrackInputs {
    pub sources: Mutex<Vec<Arc<SourceBuffer>>>,
}

#[derive(Debug, Clone)]
pub enum Want {
    Device(String),
    Apps { names: Vec<String>, inverse: bool },
}

pub fn wants_for(sources: &[String]) -> Vec<Want> {
    let mut out = Vec::new();
    let mut apps = Vec::new();
    let mut inverse_apps = Vec::new();
    for s in sources {
        if let Some(a) = s.strip_prefix("app-inverse:") {
            inverse_apps.push(a.to_lowercase());
        } else if let Some(a) = s.strip_prefix("app:") {
            apps.push(a.to_lowercase());
        } else {
            out.push(Want::Device(s.strip_prefix("device:").unwrap_or(s).to_string()));
        }
    }
    if !apps.is_empty() {
        out.push(Want::Apps { names: apps, inverse: false });
    }
    if !inverse_apps.is_empty() {
        out.push(Want::Apps { names: inverse_apps, inverse: true });
    }
    out
}
