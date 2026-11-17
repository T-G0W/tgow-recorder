use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::frame::{Frame, FrameSlot, PixFmt};

pub fn spawn(
    width: u32,
    height: u32,
    fps: u32,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("pattern".into())
        .spawn(move || {
            let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
            let started = Instant::now();
            let mut n: u32 = 0;
            while !stop.load(Ordering::SeqCst) {
                let stride = width as usize * 4;
                let mut data = vec![0u8; stride * height as usize];
                let bar = (n * 8) % width.max(1);
                for y in 0..height {
                    for x in 0..width {
                        let i = y as usize * stride + x as usize * 4;
                        let lit = x >= bar && x < bar + 64;
                        data[i] = if lit { 255 } else { (x * 255 / width.max(1)) as u8 };
                        data[i + 1] = if lit { 255 } else { (y * 255 / height.max(1)) as u8 };
                        data[i + 2] = if lit { 255 } else { (n % 256) as u8 };
                    }
                }
                slot.publish(Frame { data, width, height, stride, format: PixFmt::Bgrx, captured: Instant::now() });
                n += 1;
                let next = started + period * n;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }
            slot.close("capture stopped");
        })
        .expect("spawn pattern")
}
