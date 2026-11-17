use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use tgow_common::config::Action;
use windows::core::{s, w, PCSTR};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::daemon::Msg;

const DPAD_UP: u16 = 0x0001;
const DPAD_DOWN: u16 = 0x0002;
const DPAD_LEFT: u16 = 0x0004;
const DPAD_RIGHT: u16 = 0x0008;
const START: u16 = 0x0010;
const BACK: u16 = 0x0020;
const GUIDE: u16 = 0x0400;
const Y: u16 = 0x8000;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Gamepad {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct XState {
    packet: u32,
    pad: Gamepad,
}

type GetState = unsafe extern "system" fn(u32, *mut XState) -> u32;
type RawProc = unsafe extern "system" fn() -> isize;

struct XInput {
    get: GetState,
    home: u16,
}

fn load() -> Option<XInput> {
    unsafe {
        let lib = LoadLibraryW(w!("xinput1_4.dll")).or_else(|_| LoadLibraryW(w!("xinput1_3.dll"))).ok()?;
        if let Some(f) = GetProcAddress(lib, PCSTR(100usize as *const u8)) {
            return Some(XInput { get: std::mem::transmute::<RawProc, GetState>(f), home: GUIDE });
        }
        let f = GetProcAddress(lib, s!("XInputGetState"))?;
        Some(XInput { get: std::mem::transmute::<RawProc, GetState>(f), home: BACK })
    }
}

fn combo(button: u16) -> Option<Action> {
    match button {
        START => Some(Action::ShowHide),
        Y => Some(Action::Screenshot),
        DPAD_UP => Some(Action::SaveReplay),
        DPAD_DOWN => Some(Action::ToggleRecord),
        DPAD_LEFT => Some(Action::ToggleReplay),
        DPAD_RIGHT => Some(Action::SaveReplay1Min),
        _ => None,
    }
}

pub fn spawn(enabled: Arc<AtomicBool>, tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("controller".into()).spawn(move || {
        let mut xinput: Option<XInput> = None;
        let mut previous = [0u16; 4];
        loop {
            if !enabled.load(Ordering::Relaxed) {
                previous = [0; 4];
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            if xinput.is_none() {
                xinput = load();
                if xinput.is_none() {
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
            let Some(x) = xinput.as_ref() else { continue };
            for (index, last) in previous.iter_mut().enumerate() {
                let mut state = XState::default();
                if unsafe { (x.get)(index as u32, &mut state) } != 0 {
                    *last = 0;
                    continue;
                }
                let now = state.pad.buttons;
                let fresh = now & !*last;
                *last = now;
                if now & x.home == 0 || fresh & !x.home == 0 {
                    continue;
                }
                for bit in [START, Y, DPAD_UP, DPAD_DOWN, DPAD_LEFT, DPAD_RIGHT] {
                    if fresh & bit != 0 {
                        if let Some(action) = combo(bit) {
                            let _ = tx.send(Msg::Action(action));
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}
