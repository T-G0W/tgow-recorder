use std::collections::HashSet;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tgow_common::config::Action;

use crate::daemon::Msg;

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_GAMEPAD: usize = 0x130;
const BTN_NORTH: u16 = 0x133;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const KEY_MAX: usize = 0x2ff;

struct Pad {
    path: PathBuf,
    fd: OwnedFd,
    pressed: HashSet<u16>,
}

fn eviocgbit(ev: u32, len: usize) -> libc::c_ulong {
    let dir: libc::c_ulong = 2;
    (dir << 30) | ((len as libc::c_ulong) << 16) | ((b'E' as libc::c_ulong) << 8) | (0x20 + ev) as libc::c_ulong
}

fn is_gamepad(fd: &OwnedFd) -> bool {
    let mut bits = [0u8; KEY_MAX / 8 + 1];
    let rc = unsafe { libc::ioctl(fd.as_raw_fd(), eviocgbit(EV_KEY as u32, bits.len()), bits.as_mut_ptr()) };
    if rc < 0 {
        return false;
    }
    bits[BTN_GAMEPAD / 8] & (1 << (BTN_GAMEPAD % 8)) != 0
}

fn scan(pads: &mut Vec<Pad>) {
    let Ok(entries) = std::fs::read_dir("/dev/input") else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event") || pads.iter().any(|p| p.path == path) {
            continue;
        }
        let Ok(file) =
            std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path)
        else {
            continue;
        };
        let fd: OwnedFd = file.into();
        if is_gamepad(&fd) {
            pads.push(Pad { path, fd, pressed: HashSet::new() });
        }
    }
}

fn combo(button: u16) -> Option<Action> {
    match button {
        BTN_START => Some(Action::ShowHide),
        BTN_NORTH => Some(Action::Screenshot),
        BTN_DPAD_UP => Some(Action::SaveReplay),
        BTN_DPAD_DOWN => Some(Action::ToggleRecord),
        BTN_DPAD_LEFT => Some(Action::ToggleReplay),
        BTN_DPAD_RIGHT => Some(Action::SaveReplay1Min),
        _ => None,
    }
}

fn read_pad(pad: &mut Pad, tx: &Sender<Msg>) -> bool {
    let size = std::mem::size_of::<libc::input_event>();
    let mut buf = vec![0u8; size * 64];
    loop {
        let n = unsafe { libc::read(pad.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            return err.raw_os_error() == Some(libc::EAGAIN);
        }
        if n == 0 {
            return false;
        }
        for chunk in buf[..n as usize].chunks_exact(size) {
            let ev: libc::input_event = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
            let mut pressed_now: Option<u16> = None;
            match ev.type_ {
                EV_KEY => {
                    if ev.value == 1 {
                        pad.pressed.insert(ev.code);
                        pressed_now = Some(ev.code);
                    } else if ev.value == 0 {
                        pad.pressed.remove(&ev.code);
                    }
                }
                EV_ABS if ev.code == ABS_HAT0X || ev.code == ABS_HAT0Y => {
                    let (neg, pos) = if ev.code == ABS_HAT0X {
                        (BTN_DPAD_LEFT, BTN_DPAD_RIGHT)
                    } else {
                        (BTN_DPAD_UP, BTN_DPAD_DOWN)
                    };
                    pad.pressed.remove(&neg);
                    pad.pressed.remove(&pos);
                    if ev.value < 0 {
                        pad.pressed.insert(neg);
                        pressed_now = Some(neg);
                    } else if ev.value > 0 {
                        pad.pressed.insert(pos);
                        pressed_now = Some(pos);
                    }
                }
                _ => {}
            }
            if let Some(button) = pressed_now {
                if button != BTN_MODE && pad.pressed.contains(&BTN_MODE) {
                    if let Some(action) = combo(button) {
                        let _ = tx.send(Msg::Action(action));
                    }
                }
            }
        }
        if (n as usize) < buf.len() {
            return true;
        }
    }
}

pub fn spawn(enabled: Arc<AtomicBool>, tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("controller".into()).spawn(move || {
        let mut pads: Vec<Pad> = Vec::new();
        let mut last_scan = Instant::now() - Duration::from_secs(60);
        loop {
            if !enabled.load(Ordering::Relaxed) {
                pads.clear();
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            if last_scan.elapsed() > Duration::from_secs(3) {
                scan(&mut pads);
                last_scan = Instant::now();
            }
            if pads.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            let mut fds: Vec<libc::pollfd> =
                pads.iter().map(|p| libc::pollfd { fd: p.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
            let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 500) };
            if rc <= 0 {
                continue;
            }
            let mut dead = Vec::new();
            for (i, pfd) in fds.iter().enumerate() {
                let broken = pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0;
                if broken || (pfd.revents & libc::POLLIN != 0 && !read_pad(&mut pads[i], &tx)) {
                    dead.push(i);
                }
            }
            for i in dead.into_iter().rev() {
                pads.remove(i);
            }
        }
    });
}
