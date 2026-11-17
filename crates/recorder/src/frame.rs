use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub enum PixFmt {
    Bgrx,
    Rgbx,
}

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixFmt,
    pub captured: Instant,
}

impl Frame {
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Frame {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let w = w.min(self.width - x).max(1);
        let h = h.min(self.height - y).max(1);
        let row = w as usize * 4;
        let mut data = vec![0u8; row * h as usize];
        for r in 0..h as usize {
            let src = (y as usize + r) * self.stride + x as usize * 4;
            data[r * row..(r + 1) * row].copy_from_slice(&self.data[src..src + row]);
        }
        Frame { data, width: w, height: h, stride: row, format: self.format, captured: self.captured }
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for r in 0..self.height as usize {
            let line = &self.data[r * self.stride..r * self.stride + self.width as usize * 4];
            for px in line.as_chunks::<4>().0 {
                match self.format {
                    PixFmt::Bgrx => out.extend_from_slice(&[px[2], px[1], px[0], 255]),
                    PixFmt::Rgbx => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
                }
            }
        }
        out
    }

    pub fn blend_rgba(&mut self, img: &[u8], img_w: u32, img_h: u32, at_x: i32, at_y: i32) {
        for row in 0..img_h as i32 {
            let ty = at_y + row;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            for col in 0..img_w as i32 {
                let tx = at_x + col;
                if tx < 0 || tx >= self.width as i32 {
                    continue;
                }
                let s = ((row as u32 * img_w + col as u32) * 4) as usize;
                let a = img[s + 3] as u32;
                if a == 0 {
                    continue;
                }
                let d = ty as usize * self.stride + tx as usize * 4;
                let (r, g, b) = (img[s] as u32, img[s + 1] as u32, img[s + 2] as u32);
                let (ri, bi) = match self.format {
                    PixFmt::Bgrx => (2, 0),
                    PixFmt::Rgbx => (0, 2),
                };
                let mix = |dst: u8, src: u32| ((src * a + dst as u32 * (255 - a)) / 255) as u8;
                self.data[d + ri] = mix(self.data[d + ri], r);
                self.data[d + 1] = mix(self.data[d + 1], g);
                self.data[d + bi] = mix(self.data[d + bi], b);
            }
        }
    }
}

pub struct FrameSlot {
    inner: Mutex<SlotState>,
    ready: Condvar,
}

struct SlotState {
    frame: Option<Arc<Frame>>,
    serial: u64,
    closed: Option<String>,
}

impl Default for FrameSlot {
    fn default() -> Self {
        FrameSlot { inner: Mutex::new(SlotState { frame: None, serial: 0, closed: None }), ready: Condvar::new() }
    }
}

pub enum Wait {
    Frame(Arc<Frame>, u64),
    Timeout,
    Closed(String),
}

impl FrameSlot {
    pub fn publish(&self, frame: Frame) {
        let mut s = self.inner.lock().unwrap();
        s.frame = Some(Arc::new(frame));
        s.serial += 1;
        self.ready.notify_all();
    }

    pub fn close(&self, reason: impl Into<String>) {
        let mut s = self.inner.lock().unwrap();
        if s.closed.is_none() {
            s.closed = Some(reason.into());
        }
        self.ready.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed.is_some()
    }

    pub fn wait_newer(&self, seen: u64, timeout: Duration) -> Wait {
        let deadline = Instant::now() + timeout;
        let mut s = self.inner.lock().unwrap();
        loop {
            if let Some(reason) = &s.closed {
                return Wait::Closed(reason.clone());
            }
            if s.serial > seen {
                if let Some(f) = &s.frame {
                    return Wait::Frame(f.clone(), s.serial);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Wait::Timeout;
            }
            s = self.ready.wait_timeout(s, deadline - now).unwrap().0;
        }
    }

    pub fn latest(&self) -> Option<(Arc<Frame>, u64)> {
        let s = self.inner.lock().unwrap();
        s.frame.clone().map(|f| (f, s.serial))
    }
}
