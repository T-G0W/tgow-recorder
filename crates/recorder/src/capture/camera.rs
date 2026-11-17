use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::frame::{Frame, PixFmt};

#[cfg(windows)]
use super::mfcam as device;
#[cfg(unix)]
use super::v4l2 as device;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CamFormat {
    Auto,
    Yuyv,
    Mjpeg,
}

#[derive(Debug, Clone)]
pub struct Placement {
    pub x_percent: f32,
    pub y_percent: f32,
    pub width_percent: f32,
    pub height_percent: f32,
    pub flip: bool,
}

pub struct Image {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct Feed {
    pub placement: Placement,
    latest: Mutex<Option<Arc<Image>>>,
}

impl Feed {
    pub fn new(placement: Placement) -> Arc<Feed> {
        Arc::new(Feed { placement, latest: Mutex::new(None) })
    }

    pub fn set(&self, image: Image) {
        *self.latest.lock().unwrap() = Some(Arc::new(image));
    }

    pub fn composite(&self, frame: &mut Frame) {
        let Some(img) = self.latest.lock().unwrap().clone() else { return };
        let p = &self.placement;
        let fw = frame.width as f32;
        let fh = frame.height as f32;
        let mut tw = (fw * p.width_percent / 100.0).max(16.0);
        let mut th = (fh * p.height_percent / 100.0).max(16.0);
        let aspect = img.width as f32 / img.height.max(1) as f32;
        if tw / th > aspect {
            tw = th * aspect;
        } else {
            th = tw / aspect;
        }
        let tw = tw as u32;
        let th = th as u32;
        let ox = ((fw - tw as f32) * p.x_percent / 100.0).clamp(0.0, fw) as u32;
        let oy = ((fh - th as f32) * p.y_percent / 100.0).clamp(0.0, fh) as u32;
        let (ri, bi) = match frame.format {
            PixFmt::Bgrx => (2, 0),
            PixFmt::Rgbx => (0, 2),
        };
        for ty in 0..th {
            let fy = oy + ty;
            if fy >= frame.height {
                break;
            }
            let sy = (ty as u64 * img.height as u64 / th as u64) as u32;
            for tx in 0..tw {
                let fx = ox + tx;
                if fx >= frame.width {
                    break;
                }
                let mut sx = (tx as u64 * img.width as u64 / tw as u64) as u32;
                if p.flip {
                    sx = img.width - 1 - sx;
                }
                let s = ((sy * img.width + sx) * 4) as usize;
                let d = fy as usize * frame.stride + fx as usize * 4;
                frame.data[d + ri] = img.rgba[s];
                frame.data[d + 1] = img.rgba[s + 1];
                frame.data[d + bi] = img.rgba[s + 2];
            }
        }
    }
}

pub struct Spec {
    pub device: String,
    pub placement: Placement,
    pub format: CamFormat,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub fn parse_spec(spec: &str) -> Spec {
    let mut parts = spec.split(';');
    let device = parts.next().unwrap_or_default().trim().to_string();
    let mut s = Spec {
        device,
        placement: Placement {
            x_percent: 100.0,
            y_percent: 100.0,
            width_percent: 30.0,
            height_percent: 30.0,
            flip: false,
        },
        format: CamFormat::Auto,
        width: 0,
        height: 0,
        fps: 0,
    };
    for kv in parts {
        let Some((k, v)) = kv.split_once('=') else { continue };
        let num = || v.trim_end_matches('%').trim().parse::<f32>().unwrap_or(0.0);
        match k.trim() {
            "x" => s.placement.x_percent = num(),
            "y" => s.placement.y_percent = num(),
            "width" => s.placement.width_percent = num(),
            "height" => s.placement.height_percent = num(),
            "flip" | "hflip" => s.placement.flip = matches!(v.trim(), "yes" | "true" | "1"),
            "pixfmt" => {
                s.format = match v.trim() {
                    "yuyv" => CamFormat::Yuyv,
                    "mjpeg" => CamFormat::Mjpeg,
                    _ => CamFormat::Auto,
                }
            }
            "camera_width" => s.width = num() as u32,
            "camera_height" => s.height = num() as u32,
            "camera_fps" => s.fps = num() as u32,
            _ => {}
        }
    }
    s
}

#[cfg(unix)]
pub fn yuyv_to_rgba(src: &[u8], w: u32, h: u32, stride: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    for y in 0..h as usize {
        let row = &src[y * stride as usize..];
        for x in (0..w as usize).step_by(2) {
            let i = x * 2;
            if i + 3 >= row.len() {
                break;
            }
            let (y0, u, y1, v) = (row[i] as i32, row[i + 1] as i32 - 128, row[i + 2] as i32, row[i + 3] as i32 - 128);
            for (k, yy) in [y0, y1].into_iter().enumerate() {
                let c = (yy - 16).max(0) * 298;
                let o = (y * w as usize + x + k) * 4;
                if o + 3 >= out.len() {
                    continue;
                }
                out[o] = clamp((c + 409 * v + 128) >> 8);
                out[o + 1] = clamp((c - 100 * u - 208 * v + 128) >> 8);
                out[o + 2] = clamp((c + 516 * u + 128) >> 8);
                out[o + 3] = 255;
            }
        }
    }
    out
}

pub fn list() -> Vec<(String, String)> {
    device::list()
}

pub fn start(spec: &str, stop: Arc<AtomicBool>) -> Result<(std::thread::JoinHandle<()>, Arc<Feed>)> {
    let spec = parse_spec(spec);
    let feed = Feed::new(spec.placement.clone());
    let handle = device::start(&spec, feed.clone(), stop)?;
    Ok((handle, feed))
}
