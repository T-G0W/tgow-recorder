use anyhow::{anyhow, bail, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use crate::frame::{Frame, FrameSlot, PixFmt};

#[derive(Debug, Clone)]
pub enum Target {
    FirstMonitor,
    FocusedMonitor,
    Monitor(String),
    Area(i32, i32, u32, u32),
    Window(u32),
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

pub fn monitors() -> Result<Vec<MonitorInfo>> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    list_monitors(&conn, root)
}

fn list_monitors(conn: &RustConnection, root: Window) -> Result<Vec<MonitorInfo>> {
    let reply = conn.randr_get_monitors(root, true)?.reply()?;
    let mut out = Vec::new();
    for m in reply.monitors {
        let name = conn
            .get_atom_name(m.name)?
            .reply()
            .map(|r| String::from_utf8_lossy(&r.name).into_owned())
            .unwrap_or_default();
        out.push(MonitorInfo { name, x: m.x as i32, y: m.y as i32, width: m.width as u32, height: m.height as u32 });
    }
    Ok(out)
}

struct Shm {
    id: i32,
    addr: *mut u8,
    size: usize,
    seg: u32,
}

impl Shm {
    fn new(conn: &RustConnection, size: usize) -> Result<Shm> {
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            bail!("shmget failed");
        }
        let addr = unsafe { libc::shmat(id, std::ptr::null(), 0) };
        if addr as isize == -1 {
            unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            bail!("shmat failed");
        }
        let seg = conn.generate_id()?;
        conn.shm_attach(seg, id as u32, false)?.check()?;
        unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
        Ok(Shm { id, addr: addr as *mut u8, size, seg })
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.addr, self.size) }
    }

    fn release(&self, conn: &RustConnection) {
        let _ = conn.shm_detach(self.seg);
        unsafe { libc::shmdt(self.addr as *const libc::c_void) };
        let _ = self.id;
    }
}

fn atom(conn: &RustConnection, name: &str) -> Result<u32> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

fn active_window(conn: &RustConnection, root: Window) -> Option<Window> {
    let prop = atom(conn, "_NET_ACTIVE_WINDOW").ok()?;
    let reply = conn.get_property(false, root, prop, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
    let id = reply.value32()?.next().filter(|w| *w != 0);
    id
}

fn window_rect(conn: &RustConnection, root: Window, win: Window) -> Option<(i32, i32, u32, u32)> {
    let geo = conn.get_geometry(win).ok()?.reply().ok()?;
    let tr = conn.translate_coordinates(win, root, 0, 0).ok()?.reply().ok()?;
    Some((tr.dst_x as i32, tr.dst_y as i32, geo.width as u32, geo.height as u32))
}

fn resolve(conn: &RustConnection, root: Window, target: &Target, screen: (u32, u32)) -> Result<(i32, i32, u32, u32)> {
    let mons = list_monitors(conn, root).unwrap_or_default();
    let whole = (0, 0, screen.0, screen.1);
    let rect = match target {
        Target::FirstMonitor => mons.first().map(|m| (m.x, m.y, m.width, m.height)).unwrap_or(whole),
        Target::FocusedMonitor => {
            let p = conn.query_pointer(root)?.reply()?;
            let (px, py) = (p.root_x as i32, p.root_y as i32);
            mons.iter()
                .find(|m| px >= m.x && py >= m.y && px < m.x + m.width as i32 && py < m.y + m.height as i32)
                .or(mons.first())
                .map(|m| (m.x, m.y, m.width, m.height))
                .unwrap_or(whole)
        }
        Target::Monitor(name) => mons
            .iter()
            .find(|m| &m.name == name)
            .map(|m| (m.x, m.y, m.width, m.height))
            .ok_or_else(|| anyhow!("monitor \"{name}\" was not found"))?,
        Target::Area(x, y, w, h) => {
            if *w == 0 || *h == 0 {
                mons.iter()
                    .find(|m| *x >= m.x && *y >= m.y && *x < m.x + m.width as i32 && *y < m.y + m.height as i32)
                    .map(|m| (m.x, m.y, m.width, m.height))
                    .unwrap_or(whole)
            } else {
                (*x, *y, *w, *h)
            }
        }
        Target::Window(id) => window_rect(conn, root, *id).ok_or_else(|| anyhow!("window {id:#x} is gone"))?,
        Target::FocusedWindow => active_window(conn, root).and_then(|w| window_rect(conn, root, w)).unwrap_or(whole),
    };
    let x = rect.0.clamp(0, screen.0 as i32 - 1);
    let y = rect.1.clamp(0, screen.1 as i32 - 1);
    let w = rect.2.min(screen.0 - x as u32).max(2);
    let h = rect.3.min(screen.1 - y as u32).max(2);
    Ok((x, y, w & !1, h & !1))
}

fn draw_cursor(conn: &RustConnection, frame: &mut Frame, origin: (i32, i32)) {
    let Ok(cookie) = conn.xfixes_get_cursor_image() else { return };
    let Ok(img) = cookie.reply() else { return };
    let (w, h) = (img.width as u32, img.height as u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for px in &img.cursor_image {
        let a = (px >> 24) & 0xff;
        let un = |c: u32| (c * 255).checked_div(a).map(|v| v.min(255) as u8).unwrap_or(0);
        rgba.extend_from_slice(&[un((px >> 16) & 0xff), un((px >> 8) & 0xff), un(px & 0xff), a as u8]);
    }
    let at_x = img.x as i32 - img.xhot as i32 - origin.0;
    let at_y = img.y as i32 - img.yhot as i32 - origin.1;
    frame.blend_rgba(&rgba, w, h, at_x, at_y);
}

pub fn spawn(
    target: Target,
    cursor: bool,
    fps: u32,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    let (conn, screen_num) = RustConnection::connect(None).context("cannot connect to the X server")?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let screen_size = (screen.width_in_pixels as u32, screen.height_in_pixels as u32);
    conn.shm_query_version()?.reply().context("the X server has no MIT-SHM")?;
    if cursor {
        let _ = conn.xfixes_query_version(5, 0)?.reply();
    }
    let first = resolve(&conn, root, &target, screen_size)?;
    let follow = matches!(target, Target::FocusedWindow | Target::Window(_));

    let handle = std::thread::Builder::new().name("x11-capture".into()).spawn(move || {
        let max_bytes = screen_size.0 as usize * screen_size.1 as usize * 4;
        let shm = match Shm::new(&conn, max_bytes) {
            Ok(s) => s,
            Err(e) => {
                slot.close(format!("{e:#}"));
                return;
            }
        };
        let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
        let mut rect = first;
        let mut next = Instant::now();
        let mut last_follow = Instant::now();
        while !stop.load(Ordering::SeqCst) {
            if follow && last_follow.elapsed() > Duration::from_millis(250) {
                if let Ok(r) = resolve(&conn, root, &target, screen_size) {
                    rect = r;
                }
                last_follow = Instant::now();
            }
            let (x, y, w, h) = rect;
            let reply = conn
                .shm_get_image(
                    root,
                    x as i16,
                    y as i16,
                    w as u16,
                    h as u16,
                    !0,
                    ImageFormat::Z_PIXMAP.into(),
                    shm.seg,
                    0,
                )
                .ok()
                .and_then(|c| c.reply().ok());
            if reply.is_none() {
                slot.close("X11 capture failed");
                break;
            }
            let stride = w as usize * 4;
            let mut frame = Frame {
                data: shm.bytes()[..stride * h as usize].to_vec(),
                width: w,
                height: h,
                stride,
                format: PixFmt::Bgrx,
                captured: Instant::now(),
            };
            if cursor {
                draw_cursor(&conn, &mut frame, (x, y));
            }
            slot.publish(frame);
            next += period;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
        }
        shm.release(&conn);
        slot.close("capture stopped");
    })?;
    Ok(handle)
}
