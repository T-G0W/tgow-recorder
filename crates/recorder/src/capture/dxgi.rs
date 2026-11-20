use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetCursorPos, GetForegroundWindow, GetIconInfo, CURSORINFO, CURSOR_SHOWING, HCURSOR, HICON, ICONINFO,
};

use crate::frame::{Frame, FrameSlot, PixFmt};

#[derive(Debug, Clone)]
pub enum Target {
    PrimaryMonitor,
    Monitor(String),
    Area(i32, i32, u32, u32),
    FocusedWindow,
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

struct Output {
    adapter: IDXGIAdapter1,
    output: IDXGIOutput1,
    info: MonitorInfo,
}

pub fn dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn outputs() -> Result<Vec<Output>> {
    dpi_aware();
    let mut out = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().context("DXGI is not available")?;
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                o += 1;
                let Ok(desc) = output.GetDesc() else { continue };
                if !desc.AttachedToDesktop.as_bool() {
                    continue;
                }
                let Ok(output1) = output.cast::<IDXGIOutput1>() else { continue };
                let r = desc.DesktopCoordinates;
                let len = desc.DeviceName.iter().position(|c| *c == 0).unwrap_or(desc.DeviceName.len());
                let raw = String::from_utf16_lossy(&desc.DeviceName[..len]);
                let name = raw.trim_start_matches("\\\\.\\").to_string();
                out.push(Output {
                    adapter: adapter.clone(),
                    output: output1,
                    info: MonitorInfo {
                        name,
                        x: r.left,
                        y: r.top,
                        width: (r.right - r.left) as u32,
                        height: (r.bottom - r.top) as u32,
                    },
                });
            }
            a += 1;
        }
    }
    Ok(out)
}

pub fn monitors() -> Result<Vec<MonitorInfo>> {
    Ok(outputs()?.into_iter().map(|o| o.info).collect())
}

fn contains(m: &MonitorInfo, x: i32, y: i32) -> bool {
    x >= m.x && y >= m.y && x < m.x + m.width as i32 && y < m.y + m.height as i32
}

fn foreground_rect() -> Option<RECT> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut r = RECT::default();
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .ok()?;
        Some(r)
    }
}

type Crop = Option<(i32, i32, u32, u32)>;

fn pick(list: Vec<Output>, target: &Target) -> Result<(Output, Crop)> {
    let cursor = unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        p
    };
    let at = |x: i32, y: i32, list: Vec<Output>| -> Result<Output> {
        let mut first = None;
        for o in list {
            if contains(&o.info, x, y) {
                return Ok(o);
            }
            if first.is_none() {
                first = Some(o);
            }
        }
        first.ok_or_else(|| anyhow!("no monitor found"))
    };
    match target {
        Target::PrimaryMonitor => Ok((at(0, 0, list)?, None)),
        Target::Monitor(name) => {
            let wanted = name.trim_start_matches("\\\\.\\").to_string();
            if list.iter().any(|o| o.info.name.eq_ignore_ascii_case(&wanted)) {
                let o = list.into_iter().find(|o| o.info.name.eq_ignore_ascii_case(&wanted)).unwrap();
                Ok((o, None))
            } else {
                crate::report::warning(&format!("monitor \"{name}\" was not found, using the main monitor"));
                Ok((at(0, 0, list)?, None))
            }
        }
        Target::Area(x, y, w, h) => {
            let o = at(*x, *y, list)?;
            if *w == 0 || *h == 0 {
                Ok((o, None))
            } else {
                let crop = (x - o.info.x, y - o.info.y, *w, *h);
                Ok((o, Some(crop)))
            }
        }
        Target::FocusedWindow => {
            let r = foreground_rect();
            let (cx, cy) = r.map(|r| ((r.left + r.right) / 2, (r.top + r.bottom) / 2)).unwrap_or((cursor.x, cursor.y));
            let o = at(cx, cy, list)?;
            let crop = r.map(|r| {
                (
                    r.left - o.info.x,
                    r.top - o.info.y,
                    (r.right - r.left).max(2) as u32,
                    (r.bottom - r.top).max(2) as u32,
                )
            });
            Ok((o, crop))
        }
    }
}

struct Duplicator {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

impl Duplicator {
    fn open(o: &Output) -> Result<Duplicator> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &o.adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("could not create a Direct3D 11 device")?;
            let device = device.ok_or_else(|| anyhow!("no Direct3D device"))?;
            let context = context.ok_or_else(|| anyhow!("no Direct3D context"))?;
            let dup = o.output.DuplicateOutput(&device).context(
                "desktop duplication is not available (is another program using it, or is this a remote session?)",
            )?;
            Ok(Duplicator { device, context, dup, staging: None })
        }
    }

    fn grab(&mut self, timeout_ms: u32) -> Result<Option<Frame>> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => bail!("access lost"),
                Err(e) => return Err(e.into()),
            }
            let result = (|| -> Result<Option<Frame>> {
                if info.LastPresentTime == 0 {
                    return Ok(None);
                }
                let texture: ID3D11Texture2D = resource.ok_or_else(|| anyhow!("empty frame"))?.cast()?;
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                let staging = match &self.staging {
                    Some((t, w, h)) if *w == desc.Width && *h == desc.Height => t.clone(),
                    _ => {
                        let sd = D3D11_TEXTURE2D_DESC {
                            Width: desc.Width,
                            Height: desc.Height,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: desc.Format,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Usage: D3D11_USAGE_STAGING,
                            BindFlags: 0,
                            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                            MiscFlags: 0,
                        };
                        let mut tex = None;
                        self.device.CreateTexture2D(&sd, None, Some(&mut tex))?;
                        let tex = tex.ok_or_else(|| anyhow!("staging texture failed"))?;
                        self.staging = Some((tex.clone(), desc.Width, desc.Height));
                        tex
                    }
                };
                self.context.CopyResource(&staging, &texture);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let row = desc.Width as usize * 4;
                let pitch = mapped.RowPitch as usize;
                let mut data = vec![0u8; row * desc.Height as usize];
                let src = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * desc.Height as usize);
                for y in 0..desc.Height as usize {
                    data[y * row..(y + 1) * row].copy_from_slice(&src[y * pitch..y * pitch + row]);
                }
                self.context.Unmap(&staging, 0);
                Ok(Some(Frame {
                    data,
                    width: desc.Width,
                    height: desc.Height,
                    stride: row,
                    format: PixFmt::Bgrx,
                    captured: Instant::now(),
                }))
            })();
            let _ = self.dup.ReleaseFrame();
            result
        }
    }
}

struct Gdi {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
}

impl Gdi {
    fn open(o: &Output) -> Result<Gdi> {
        let MonitorInfo { x, y, width, height, .. } = o.info;
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width as i32,
                    biHeight: -(height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .context("could not create a capture bitmap")?;
            SelectObject(memory, bitmap.into());
            Ok(Gdi { x, y, width, height, screen, memory, bitmap, bits: bits as *mut u8 })
        }
    }

    fn grab(&mut self, timeout_ms: u32) -> Result<Option<Frame>> {
        std::thread::sleep(Duration::from_millis(timeout_ms as u64));
        unsafe {
            BitBlt(
                self.memory,
                0,
                0,
                self.width as i32,
                self.height as i32,
                Some(self.screen),
                self.x,
                self.y,
                SRCCOPY | CAPTUREBLT,
            )
            .context("screen copy failed")?;
            let len = (self.width * self.height * 4) as usize;
            let data = std::slice::from_raw_parts(self.bits, len).to_vec();
            Ok(Some(Frame {
                data,
                width: self.width,
                height: self.height,
                stride: self.width as usize * 4,
                format: PixFmt::Bgrx,
                captured: Instant::now(),
            }))
        }
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

enum Grabber {
    Dxgi(Duplicator),
    Gdi(Gdi),
}

impl Grabber {
    fn open(o: &Output) -> Result<Grabber> {
        match Duplicator::open(o) {
            Ok(d) => Ok(Grabber::Dxgi(d)),
            Err(e) => {
                crate::report::info(&format!("{e:#}, using GDI capture instead"));
                Ok(Grabber::Gdi(Gdi::open(o)?))
            }
        }
    }

    fn grab(&mut self, timeout_ms: u32) -> Result<Option<Frame>> {
        match self {
            Grabber::Dxgi(d) => d.grab(timeout_ms),
            Grabber::Gdi(g) => g.grab(timeout_ms),
        }
    }
}

unsafe impl Send for Grabber {}

struct CursorImage {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    hot_x: i32,
    hot_y: i32,
}

fn bitmap_bits(bitmap: HBITMAP) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        Some((buf, w, h))
    }
}

fn cursor_image(handle: HCURSOR) -> Option<CursorImage> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(HICON(handle.0), &mut ii).ok()?;
        let color = if ii.hbmColor.is_invalid() { None } else { bitmap_bits(ii.hbmColor) };
        let mask = if ii.hbmMask.is_invalid() { None } else { bitmap_bits(ii.hbmMask) };
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        let (rgba, width, height) = match (color, mask) {
            (Some((c, w, h)), m) => {
                let has_alpha = c.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
                let mut out = Vec::with_capacity(c.len());
                for (i, p) in c.as_chunks::<4>().0.iter().enumerate() {
                    let a = if has_alpha {
                        p[3]
                    } else {
                        let masked =
                            m.as_ref().map(|(mb, _, _)| mb.get(i * 4).copied().unwrap_or(0) != 0).unwrap_or(false);
                        if masked {
                            0
                        } else {
                            255
                        }
                    };
                    out.extend_from_slice(&[p[2], p[1], p[0], a]);
                }
                (out, w, h)
            }
            (None, Some((m, w, h2))) => {
                let h = h2 / 2;
                let row = (w * 4) as usize;
                let mut out = Vec::with_capacity(row * h as usize);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let and = m[y * row + x * 4] != 0;
                        let xor = m[(y + h as usize) * row + x * 4] != 0;
                        let px = match (and, xor) {
                            (false, false) => [0, 0, 0, 255],
                            (false, true) => [255, 255, 255, 255],
                            (true, true) => [0, 0, 0, 200],
                            (true, false) => [0, 0, 0, 0],
                        };
                        out.extend_from_slice(&px);
                    }
                }
                (out, w, h)
            }
            (None, None) => return None,
        };
        Some(CursorImage { rgba, width, height, hot_x: ii.xHotspot as i32, hot_y: ii.yHotspot as i32 })
    }
}

fn draw_cursor(frame: &mut Frame, origin: (i32, i32), cache: &mut HashMap<isize, Option<CursorImage>>) {
    unsafe {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_err() || ci.flags != CURSOR_SHOWING || ci.hCursor.is_invalid() {
            return;
        }
        let key = ci.hCursor.0 as isize;
        let img = cache.entry(key).or_insert_with(|| cursor_image(ci.hCursor));
        if let Some(img) = img {
            let x = ci.ptScreenPos.x - img.hot_x - origin.0;
            let y = ci.ptScreenPos.y - img.hot_y - origin.1;
            frame.blend_rgba(&img.rgba, img.width, img.height, x, y);
        }
    }
}

fn cursor_state() -> (i32, i32, isize) {
    unsafe {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        if GetCursorInfo(&mut ci).is_err() {
            return (0, 0, 0);
        }
        (ci.ptScreenPos.x, ci.ptScreenPos.y, if ci.flags == CURSOR_SHOWING { ci.hCursor.0 as isize } else { 0 })
    }
}

pub fn spawn(
    target: Target,
    cursor: bool,
    fps: u32,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    let (first, first_crop) = pick(outputs()?, &target)?;
    let mut dup = Grabber::open(&first)?;
    let follow = matches!(target, Target::FocusedWindow);
    let handle = std::thread::Builder::new().name("dxgi-capture".into()).spawn(move || {
        let mut out = first;
        let mut crop = first_crop;
        let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        let mut cursors = HashMap::new();
        let mut last_follow = Instant::now();
        let mut last_frame: Option<Frame> = None;
        let mut last_cursor = (0, 0, 0isize);
        let mut last_crop = crop;
        let mut pending = false;
        let mut next = Instant::now();
        let mut blank_since = Instant::now();
        while !stop.load(Ordering::SeqCst) {
            if follow && last_follow.elapsed() > Duration::from_millis(250) {
                last_follow = Instant::now();
                if let Ok(list) = outputs() {
                    if let Ok((o, c)) = pick(list, &target) {
                        if o.info.name != out.info.name {
                            if let Ok(d) = Grabber::open(&o) {
                                dup = d;
                                out = o;
                                last_frame = None;
                                blank_since = Instant::now();
                            }
                        }
                        crop = c;
                    }
                }
            }
            let wait = next.saturating_duration_since(Instant::now()).as_millis().clamp(1, 100) as u32;
            let mut fresh = false;
            match dup.grab(wait) {
                Ok(Some(f)) => {
                    last_frame = Some(f);
                    fresh = true;
                }
                Ok(None) => {}
                Err(e) => {
                    if e.to_string() == "access lost" {
                        std::thread::sleep(Duration::from_millis(100));
                        if let Ok((o, c, d)) = outputs()
                            .and_then(|l| pick(l, &target))
                            .and_then(|(o, c)| Grabber::open(&o).map(|d| (o, c, d)))
                        {
                            out = o;
                            crop = c;
                            dup = d;
                            last_frame = None;
                            blank_since = Instant::now();
                        }
                        continue;
                    }
                    slot.close(format!("screen capture failed: {e:#}"));
                    return;
                }
            }
            if last_frame.is_none() && blank_since.elapsed() > Duration::from_millis(300) {
                blank_since = Instant::now();
                if let Ok(Some(f)) = Gdi::open(&out).and_then(|mut g| g.grab(0)) {
                    last_frame = Some(f);
                    fresh = true;
                }
            }
            if cursor {
                let state = cursor_state();
                if state != last_cursor {
                    last_cursor = state;
                    fresh = true;
                }
            }
            if follow && crop != last_crop {
                last_crop = crop;
                fresh = true;
            }
            let now = Instant::now();
            pending |= fresh;
            if !pending || now < next {
                continue;
            }
            pending = false;
            next = if now > next + period * 4 { now + period } else { next + period };
            let Some(base) = last_frame.as_ref() else { continue };
            let mut frame = base.clone();
            frame.captured = now;
            if cursor {
                draw_cursor(&mut frame, (out.info.x, out.info.y), &mut cursors);
            }
            if let Some((x, y, w, h)) = crop {
                frame = frame.crop(x.max(0) as u32, y.max(0) as u32, w & !1, h & !1);
            }
            slot.publish(frame);
        }
        slot.close("capture stopped");
    })?;
    Ok(handle)
}
