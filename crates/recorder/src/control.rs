use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use tgow_common::ipc::recorder::Command;

static SIGNAL_HIT: AtomicI32 = AtomicI32::new(0);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
const STOP: i32 = 2;

#[cfg(unix)]
const SAVE: i32 = libc::SIGUSR1;
#[cfg(unix)]
const TOGGLE_PAUSE: i32 = libc::SIGUSR2;
#[cfg(windows)]
const SAVE: i32 = -10;
#[cfg(windows)]
const TOGGLE_PAUSE: i32 = -12;

#[cfg(unix)]
pub fn cancelled() -> bool {
    let hit = SIGNAL_HIT.load(Ordering::SeqCst);
    CANCELLED.load(Ordering::SeqCst) || (hit != 0 && hit != SAVE && hit != TOGGLE_PAUSE)
}

#[cfg(unix)]
extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL_HIT.store(sig, Ordering::SeqCst);
}

#[cfg(unix)]
pub fn install_signals() {
    unsafe {
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGUSR1, libc::SIGUSR2, libc::SIGHUP] {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(windows)]
unsafe extern "system" fn on_console(_kind: u32) -> windows::core::BOOL {
    SIGNAL_HIT.store(STOP, Ordering::SeqCst);
    true.into()
}

#[cfg(windows)]
pub fn install_signals() {
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(on_console), true);
    }
}

pub fn listen(stop: Arc<AtomicBool>) -> Receiver<Command> {
    let (tx, rx) = channel();
    let stdin_tx: Sender<Command> = tx.clone();
    std::thread::Builder::new()
        .name("stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if let Some(cmd) = Command::parse(&line) {
                    if cmd == Command::Stop {
                        CANCELLED.store(true, Ordering::SeqCst);
                    }
                    if stdin_tx.send(cmd).is_err() {
                        break;
                    }
                }
            }
        })
        .ok();
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || loop {
            let sig = SIGNAL_HIT.swap(0, Ordering::SeqCst);
            let cmd = match sig {
                0 => None,
                SAVE => Some(Command::Save),
                TOGGLE_PAUSE => Some(Command::TogglePause),
                _ => {
                    CANCELLED.store(true, Ordering::SeqCst);
                    Some(Command::Stop)
                }
            };
            if let Some(c) = cmd {
                if tx.send(c).is_err() {
                    break;
                }
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        })
        .ok();
    rx
}
