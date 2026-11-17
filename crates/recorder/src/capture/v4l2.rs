use anyhow::{anyhow, bail, Context, Result};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::camera::{yuyv_to_rgba, CamFormat, Feed, Image, Spec};

const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600;
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605;
const VIDIOC_REQBUFS: libc::c_ulong = 0xc014_5608;
const VIDIOC_QUERYBUF: libc::c_ulong = 0xc058_5609;
const VIDIOC_QBUF: libc::c_ulong = 0xc058_560f;
const VIDIOC_DQBUF: libc::c_ulong = 0xc058_5611;
const VIDIOC_STREAMON: libc::c_ulong = 0x4004_5612;
const VIDIOC_STREAMOFF: libc::c_ulong = 0x4004_5613;
const VIDIOC_S_PARM: libc::c_ulong = 0xc0cc_5616;

const BUF_TYPE_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const CAP_VIDEO_CAPTURE: u32 = 0x1;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

const fn fourcc(s: &[u8; 4]) -> u32 {
    (s[0] as u32) | ((s[1] as u32) << 8) | ((s[2] as u32) << 16) | ((s[3] as u32) << 24)
}
const FMT_YUYV: u32 = fourcc(b"YUYV");
const FMT_MJPG: u32 = fourcc(b"MJPG");

#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct Format {
    kind: u32,
    pad: u32,
    pix: [u32; 12],
    rest: [u8; 152],
}

#[repr(C)]
struct RequestBuffers {
    count: u32,
    kind: u32,
    memory: u32,
    capabilities: u32,
    flags: [u8; 4],
}

#[repr(C)]
struct Buffer {
    index: u32,
    kind: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    pad0: u32,
    timestamp: [i64; 2],
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    offset: u64,
    length: u32,
    reserved2: u32,
    request_fd: u32,
    pad1: u32,
}

#[repr(C)]
struct StreamParm {
    kind: u32,
    capability: u32,
    capturemode: u32,
    timeperframe: [u32; 2],
    extendedmode: u32,
    readbuffers: u32,
    reserved: [u32; 4],
    rest: [u8; 160],
}

fn ioctl<T>(fd: i32, req: libc::c_ulong, arg: &mut T) -> std::io::Result<()> {
    loop {
        let rc = unsafe { libc::ioctl(fd, req, arg as *mut T) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

pub fn list() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/dev") else { return out };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("video")).unwrap_or(false))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
        let mut cap: Capability = unsafe { std::mem::zeroed() };
        if ioctl(file.as_raw_fd(), VIDIOC_QUERYCAP, &mut cap).is_err() {
            continue;
        }
        let caps = if cap.capabilities & CAP_DEVICE_CAPS != 0 { cap.device_caps } else { cap.capabilities };
        if caps & CAP_VIDEO_CAPTURE == 0 {
            continue;
        }
        out.push((path.to_string_lossy().into_owned(), cstr(&cap.card)));
    }
    out
}

struct Mapped {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for Mapped {}

fn set_format(fd: i32, fmt_code: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let mut f: Format = unsafe { std::mem::zeroed() };
    f.kind = BUF_TYPE_CAPTURE;
    f.pix[0] = if w == 0 { 1280 } else { w };
    f.pix[1] = if h == 0 { 720 } else { h };
    f.pix[2] = fmt_code;
    f.pix[3] = 1;
    ioctl(fd, VIDIOC_S_FMT, &mut f).ok()?;
    if f.pix[2] != fmt_code {
        return None;
    }
    Some((f.pix[0], f.pix[1], f.pix[2], f.pix[4]))
}

pub fn start(spec: &Spec, feed: Arc<Feed>, stop: Arc<AtomicBool>) -> Result<std::thread::JoinHandle<()>> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&spec.device)
        .with_context(|| format!("cannot open {}", spec.device))?;
    let fd = file.as_raw_fd();

    let order: Vec<u32> = match spec.format {
        CamFormat::Yuyv => vec![FMT_YUYV],
        CamFormat::Mjpeg => vec![FMT_MJPG],
        CamFormat::Auto => vec![FMT_MJPG, FMT_YUYV],
    };
    let (width, height, pixfmt, stride) = order
        .iter()
        .find_map(|f| set_format(fd, *f, spec.width, spec.height))
        .ok_or_else(|| anyhow!("{} supports neither YUYV nor MJPEG", spec.device))?;

    if spec.fps > 0 {
        let mut parm: StreamParm = unsafe { std::mem::zeroed() };
        parm.kind = BUF_TYPE_CAPTURE;
        parm.timeperframe = [1, spec.fps];
        let _ = ioctl(fd, VIDIOC_S_PARM, &mut parm);
    }

    let mut req =
        RequestBuffers { count: 4, kind: BUF_TYPE_CAPTURE, memory: MEMORY_MMAP, capabilities: 0, flags: [0; 4] };
    ioctl(fd, VIDIOC_REQBUFS, &mut req).context("camera buffer request failed")?;
    let mut maps = Vec::new();
    for i in 0..req.count {
        let mut b: Buffer = unsafe { std::mem::zeroed() };
        b.index = i;
        b.kind = BUF_TYPE_CAPTURE;
        b.memory = MEMORY_MMAP;
        ioctl(fd, VIDIOC_QUERYBUF, &mut b)?;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                b.length as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                b.offset as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            bail!("camera mmap failed");
        }
        maps.push(Mapped { ptr: ptr as *mut u8, len: b.length as usize });
        ioctl(fd, VIDIOC_QBUF, &mut b)?;
    }
    let mut kind = BUF_TYPE_CAPTURE as i32;
    ioctl(fd, VIDIOC_STREAMON, &mut kind).context("camera stream start failed")?;

    let feed_thread = feed;
    let handle = std::thread::Builder::new().name("webcam".into()).spawn(move || {
        let fd = file.as_raw_fd();
        while !stop.load(Ordering::SeqCst) {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let rc = unsafe { libc::poll(&mut pfd, 1, 100) };
            if rc <= 0 {
                continue;
            }
            let mut b: Buffer = unsafe { std::mem::zeroed() };
            b.kind = BUF_TYPE_CAPTURE;
            b.memory = MEMORY_MMAP;
            if ioctl(fd, VIDIOC_DQBUF, &mut b).is_err() {
                continue;
            }
            if let Some(m) = maps.get(b.index as usize) {
                let used = (b.bytesused as usize).min(m.len);
                let data = unsafe { std::slice::from_raw_parts(m.ptr, used) };
                let image = if pixfmt == FMT_YUYV {
                    Some(Image { rgba: yuyv_to_rgba(data, width, height, stride.max(width * 2)), width, height })
                } else {
                    image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok().map(|img| {
                        let rgba = img.to_rgba8();
                        Image { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
                    })
                };
                if let Some(img) = image {
                    feed_thread.set(img);
                }
            }
            let _ = ioctl(fd, VIDIOC_QBUF, &mut b);
        }
        let mut kind = BUF_TYPE_CAPTURE as i32;
        let _ = ioctl(fd, VIDIOC_STREAMOFF, &mut kind);
        for m in maps {
            unsafe { libc::munmap(m.ptr as *mut libc::c_void, m.len) };
        }
    })?;
    Ok(handle)
}
