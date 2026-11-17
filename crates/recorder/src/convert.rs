use crate::frame::{Frame, PixFmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Nv12,
    P010,
    Yuv420p,
    Yuv444p,
}

const SHIFT: i32 = 16;
const HALF: i32 = 1 << (SHIFT - 1);

#[derive(Clone, Copy)]
struct Coeffs {
    y: [i32; 3],
    u: [i32; 3],
    v: [i32; 3],
    y_off: i32,
}

fn coeffs(full: bool) -> Coeffs {
    let (kr, kb) = (0.2126f64, 0.0722f64);
    let kg = 1.0 - kr - kb;
    let (ys, cs) = if full { (1.0, 1.0) } else { (219.0 / 255.0, 224.0 / 255.0) };
    let q = |v: f64| (v * (1i64 << SHIFT) as f64).round() as i32;
    Coeffs {
        y: [q(kr * ys), q(kg * ys), q(kb * ys)],
        u: [q(-kr / (2.0 * (1.0 - kb)) * cs), q(-kg / (2.0 * (1.0 - kb)) * cs), q(0.5 * cs)],
        v: [q(0.5 * cs), q(-kg / (2.0 * (1.0 - kr)) * cs), q(-kb / (2.0 * (1.0 - kr)) * cs)],
        y_off: if full { 0 } else { 16 },
    }
}

impl Coeffs {
    #[inline(always)]
    fn luma(&self, r: i32, g: i32, b: i32) -> i32 {
        (((self.y[0] * r + self.y[1] * g + self.y[2] * b + HALF) >> SHIFT) + self.y_off).clamp(0, 255)
    }

    #[inline(always)]
    fn chroma(&self, r: i32, g: i32, b: i32) -> (i32, i32) {
        let u = ((self.u[0] * r + self.u[1] * g + self.u[2] * b + HALF) >> SHIFT) + 128;
        let v = ((self.v[0] * r + self.v[1] * g + self.v[2] * b + HALF) >> SHIFT) + 128;
        (u.clamp(0, 255), v.clamp(0, 255))
    }

    #[inline(always)]
    fn luma10(&self, r: i32, g: i32, b: i32) -> i32 {
        (((self.y[0] * r + self.y[1] * g + self.y[2] * b) >> (SHIFT - 2)) + self.y_off * 4).clamp(0, 1023)
    }

    #[inline(always)]
    fn chroma10(&self, r: i32, g: i32, b: i32) -> (i32, i32) {
        let u = ((self.u[0] * r + self.u[1] * g + self.u[2] * b) >> (SHIFT - 2)) + 512;
        let v = ((self.v[0] * r + self.v[1] * g + self.v[2] * b) >> (SHIFT - 2)) + 512;
        (u.clamp(0, 1023), v.clamp(0, 1023))
    }
}

#[derive(Clone, Copy)]
struct Src<'a> {
    data: &'a [u8],
    stride: usize,
    ri: usize,
    bi: usize,
}

impl Src<'_> {
    #[inline(always)]
    fn rgb(&self, x: usize, y: usize) -> (i32, i32, i32) {
        let p = y * self.stride + x * 4;
        (self.data[p + self.ri] as i32, self.data[p + 1] as i32, self.data[p + self.bi] as i32)
    }

    #[inline(always)]
    fn quad(&self, x: usize, y: usize) -> (i32, i32, i32) {
        let a = self.rgb(x, y);
        let b = self.rgb(x + 1, y);
        let c = self.rgb(x, y + 1);
        let d = self.rgb(x + 1, y + 1);
        ((a.0 + b.0 + c.0 + d.0 + 2) >> 2, (a.1 + b.1 + c.1 + d.1 + 2) >> 2, (a.2 + b.2 + c.2 + d.2 + 2) >> 2)
    }
}

pub struct Target<'a> {
    pub planes: Vec<&'a mut [u8]>,
    pub strides: Vec<usize>,
}

struct Job<'a> {
    first_pair: usize,
    pairs: usize,
    planes: Vec<&'a mut [u8]>,
}

fn run_job(job: Job, s: Src, c: Coeffs, w: usize, layout: Layout, strides: &[usize]) {
    let mut planes = job.planes;
    for local in 0..job.pairs {
        let sy = (job.first_pair + local) * 2;
        match layout {
            Layout::Nv12 => {
                let (yp, rest) = planes.split_at_mut(1);
                let yrow0 = local * 2 * strides[0];
                let yrow1 = yrow0 + strides[0];
                let crow = local * strides[1];
                for x in (0..w).step_by(2) {
                    for (dx, dy, row) in [(0, 0, yrow0), (1, 0, yrow0), (0, 1, yrow1), (1, 1, yrow1)] {
                        let (r, g, b) = s.rgb(x + dx, sy + dy);
                        yp[0][row + x + dx] = c.luma(r, g, b) as u8;
                    }
                    let (r, g, b) = s.quad(x, sy);
                    let (u, v) = c.chroma(r, g, b);
                    rest[0][crow + x] = u as u8;
                    rest[0][crow + x + 1] = v as u8;
                }
            }
            Layout::Yuv420p => {
                let (yp, rest) = planes.split_at_mut(1);
                let (up, vp) = rest.split_at_mut(1);
                let yrow0 = local * 2 * strides[0];
                let yrow1 = yrow0 + strides[0];
                let urow = local * strides[1];
                let vrow = local * strides[2];
                for x in (0..w).step_by(2) {
                    for (dx, dy, row) in [(0, 0, yrow0), (1, 0, yrow0), (0, 1, yrow1), (1, 1, yrow1)] {
                        let (r, g, b) = s.rgb(x + dx, sy + dy);
                        yp[0][row + x + dx] = c.luma(r, g, b) as u8;
                    }
                    let (r, g, b) = s.quad(x, sy);
                    let (u, v) = c.chroma(r, g, b);
                    up[0][urow + x / 2] = u as u8;
                    vp[0][vrow + x / 2] = v as u8;
                }
            }
            Layout::Yuv444p => {
                let (yp, rest) = planes.split_at_mut(1);
                let (up, vp) = rest.split_at_mut(1);
                for dy in 0..2 {
                    let yrow = (local * 2 + dy) * strides[0];
                    let urow = (local * 2 + dy) * strides[1];
                    let vrow = (local * 2 + dy) * strides[2];
                    for x in 0..w {
                        let (r, g, b) = s.rgb(x, sy + dy);
                        yp[0][yrow + x] = c.luma(r, g, b) as u8;
                        let (u, v) = c.chroma(r, g, b);
                        up[0][urow + x] = u as u8;
                        vp[0][vrow + x] = v as u8;
                    }
                }
            }
            Layout::P010 => {
                let (yp, rest) = planes.split_at_mut(1);
                let yrow0 = local * 2 * strides[0];
                let yrow1 = yrow0 + strides[0];
                let crow = local * strides[1];
                let put = |buf: &mut [u8], at: usize, v: i32| {
                    let word = ((v as u16) << 6).to_le_bytes();
                    buf[at] = word[0];
                    buf[at + 1] = word[1];
                };
                for x in (0..w).step_by(2) {
                    for (dx, dy, row) in [(0, 0, yrow0), (1, 0, yrow0), (0, 1, yrow1), (1, 1, yrow1)] {
                        let (r, g, b) = s.rgb(x + dx, sy + dy);
                        put(yp[0], row + (x + dx) * 2, c.luma10(r, g, b));
                    }
                    let (r, g, b) = s.quad(x, sy);
                    let (u, v) = c.chroma10(r, g, b);
                    put(rest[0], crow + x * 2, u);
                    put(rest[0], crow + x * 2 + 2, v);
                }
            }
        }
    }
}

pub fn convert(frame: &Frame, layout: Layout, full_range: bool, target: Target) {
    let (src, src_stride, width, height) = (&frame.data[..], frame.stride, frame.width as usize, frame.height as usize);
    let (ri, bi) = match frame.format {
        PixFmt::Bgrx => (2, 0),
        PixFmt::Rgbx => (0, 2),
    };
    let s = Src { data: src, stride: src_stride, ri, bi };
    let c = coeffs(full_range);
    let w = width & !1;
    let pairs = height / 2;
    if pairs == 0 || w == 0 {
        return;
    }
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8).min(pairs);
    let per = pairs.div_ceil(workers);

    let rows_for = |plane: usize| -> usize {
        match (layout, plane) {
            (_, 0) => per * 2,
            (Layout::Yuv444p, _) => per * 2,
            _ => per,
        }
    };
    let strides = target.strides.clone();
    let mut split: Vec<std::vec::IntoIter<&mut [u8]>> = target
        .planes
        .into_iter()
        .enumerate()
        .map(|(i, p)| p.chunks_mut(strides[i] * rows_for(i)).collect::<Vec<_>>().into_iter())
        .collect();

    std::thread::scope(|scope| {
        for t in 0..workers {
            let first_pair = t * per;
            if first_pair >= pairs {
                break;
            }
            let count = per.min(pairs - first_pair);
            let planes: Vec<&mut [u8]> = split.iter_mut().filter_map(|it| it.next()).collect();
            let job = Job { first_pair, pairs: count, planes };
            let strides = &strides;
            scope.spawn(move || run_job(job, s, c, w, layout, strides));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(data: Vec<u8>, width: u32, height: u32) -> Frame {
        Frame {
            stride: width as usize * 4,
            data,
            width,
            height,
            format: PixFmt::Bgrx,
            captured: std::time::Instant::now(),
        }
    }

    #[test]
    fn white_and_black_limited() {
        let src = vec![255u8; 4 * 4 * 2];
        let mut y = vec![0u8; 4 * 2];
        let mut uv = vec![0u8; 4];
        convert(&frame(src, 4, 2), Layout::Nv12, false, Target { planes: vec![&mut y, &mut uv], strides: vec![4, 4] });
        assert!(y.iter().all(|v| *v == 235));
        assert!(uv.iter().all(|v| *v == 128));
    }

    #[test]
    fn red_full_range() {
        let mut src = vec![0u8; 4 * 2 * 2];
        for px in src.as_chunks_mut::<4>().0 {
            px[2] = 255;
        }
        let mut y = vec![0u8; 2 * 2];
        let mut uv = vec![0u8; 2];
        convert(&frame(src, 2, 2), Layout::Nv12, true, Target { planes: vec![&mut y, &mut uv], strides: vec![2, 2] });
        assert_eq!(y[0], 54);
        assert_eq!(uv[1], 255);
    }
}
