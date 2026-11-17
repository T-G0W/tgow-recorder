use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tgow_common::config::{Action, Hotkey};
use tgow_common::trf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, KillTimer, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HICON, MF_SEPARATOR,
    MF_STRING, MSG, SM_CXSMICON, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE,
    WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

use crate::daemon::Msg;
use crate::state::{dot_rgba, MenuCommand, MenuEntry, TrayState};

const WM_COMMANDS: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const TRAY_ID: u32 = 1;
const BALLOON_TIMER: usize = 1;

pub enum Cmd {
    Bind(Sender<Msg>),
    TrayVisible(bool),
    TrayState(TrayState),
    Refresh,
    Hotkeys(Vec<(Action, Hotkey)>),
    Balloon { error: bool, text: String, timeout_ms: u32 },
}

struct Shell {
    tx: Mutex<Sender<Cmd>>,
    hwnd: isize,
}

static SHELL: OnceLock<Option<Shell>> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn post(cmd: Cmd) {
    let Some(shell) = SHELL.get_or_init(start) else { return };
    let sent = shell.tx.lock().map(|tx| tx.send(cmd).is_ok()).unwrap_or(false);
    if sent {
        unsafe {
            let _ = PostMessageW(Some(HWND(shell.hwnd as *mut _)), WM_COMMANDS, WPARAM(0), LPARAM(0));
        }
    }
}

fn start() -> Option<Shell> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    let (ready_tx, ready_rx) = mpsc::channel::<isize>();
    std::thread::Builder::new()
        .name("shell".into())
        .spawn(move || unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return };
            let class = w!("TgowRecorderShell");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: module.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("TGOW Recorder"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module.into()),
                None,
            ) else {
                return;
            };
            TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
            STATE.with(|s| *s.borrow_mut() = Some(State::new(hwnd, rx)));
            let _ = ready_tx.send(hwnd.0 as isize);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .ok()?;
    let hwnd = ready_rx.recv_timeout(Duration::from_secs(5)).ok()?;
    Some(Shell { tx: Mutex::new(tx), hwnd })
}

struct State {
    hwnd: HWND,
    rx: Receiver<Cmd>,
    msg_tx: Option<Sender<Msg>>,
    visible: bool,
    added: bool,
    temporary: bool,
    tray: TrayState,
    icon: Option<HICON>,
    hotkeys: Vec<(i32, Action)>,
}

fn copy_wide(dst: &mut [u16], text: &str) {
    let mut n = 0;
    for unit in text.encode_utf16().take(dst.len() - 1) {
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}

fn make_icon(state: TrayState) -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let (rgb, ring) = state.look();
    let pixels: Vec<u8> = dot_rgba(size, rgb, ring).into_iter().flat_map(|[r, g, b, a]| [b, g, r, a]).collect();
    let row = (size as usize).div_ceil(16) * 2;
    let mask = vec![0u8; row * size as usize];
    unsafe { CreateIcon(None, size, size, 1, 32, mask.as_ptr(), pixels.as_ptr()).ok() }
}

fn vk_for(key: &str) -> Option<u32> {
    let vk = match key {
        "Enter" | "Return" => 0x0d,
        "Space" => 0x20,
        "Escape" => 0x1b,
        "Tab" => 0x09,
        "Backspace" => 0x08,
        "Insert" => 0x2d,
        "Delete" => 0x2e,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ArrowLeft" | "Left" => 0x25,
        "ArrowUp" | "Up" => 0x26,
        "ArrowRight" | "Right" => 0x27,
        "ArrowDown" | "Down" => 0x28,
        "Print" | "PrintScreen" => 0x2c,
        "Pause" => 0x13,
        "ScrollLock" => 0x91,
        "Minus" => 0xbd,
        "Plus" | "Equals" => 0xbb,
        "Comma" => 0xbc,
        "Period" => 0xbe,
        "Slash" => 0xbf,
        "Semicolon" => 0xba,
        _ => {
            if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
                if (1..=24).contains(&n) {
                    return Some(0x70 + n - 1);
                }
            }
            let key = key.strip_prefix("Num").filter(|k| k.len() == 1).unwrap_or(key);
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase() as u32
            } else {
                return None;
            }
        }
    };
    Some(vk)
}

fn modifiers_for(h: &Hotkey) -> HOT_KEY_MODIFIERS {
    let mut mods = MOD_NOREPEAT;
    for m in &h.modifiers {
        mods |= match m.as_str() {
            "Alt" => MOD_ALT,
            "Ctrl" => MOD_CONTROL,
            "Shift" => MOD_SHIFT,
            "Super" => MOD_WIN,
            _ => HOT_KEY_MODIFIERS(0),
        };
    }
    mods
}

impl State {
    fn new(hwnd: HWND, rx: Receiver<Cmd>) -> Self {
        Self {
            hwnd,
            rx,
            msg_tx: None,
            visible: false,
            added: false,
            temporary: false,
            tray: TrayState::default(),
            icon: None,
            hotkeys: Vec::new(),
        }
    }

    fn send(&self, msg: Msg) {
        if let Some(tx) = &self.msg_tx {
            let _ = tx.send(msg);
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon.unwrap_or_default(),
            ..Default::default()
        };
        let mut tip = vec!["TGOW Recorder".to_string()];
        tip.extend(self.tray.summary());
        copy_wide(&mut data.szTip, &tip.join("\n"));
        data
    }

    fn refresh_icon(&mut self) {
        let fresh = make_icon(self.tray);
        if let Some(old) = std::mem::replace(&mut self.icon, fresh) {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
    }

    fn show_icon(&mut self) {
        if self.icon.is_none() {
            self.refresh_icon();
        }
        let data = self.data();
        unsafe {
            if self.added {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            } else {
                self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
            }
        }
    }

    fn hide_icon(&mut self) {
        if self.added {
            let data = self.data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
        self.temporary = false;
    }

    fn set_hotkeys(&mut self, bindings: Vec<(Action, Hotkey)>) {
        for (id, _) in self.hotkeys.drain(..) {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
        }
        let mut next = 1;
        for (action, hotkey) in bindings {
            let Some(vk) = vk_for(&hotkey.key) else { continue };
            let ok = unsafe { RegisterHotKey(Some(self.hwnd), next, modifiers_for(&hotkey), vk) }.is_ok();
            if ok {
                self.hotkeys.push((next, action));
                next += 1;
            } else {
                let text = trf("The hotkey %s is already used for something else", &[&hotkey.label()]);
                self.send(Msg::Notice(text));
            }
        }
    }

    fn balloon(&mut self, error: bool, text: &str, timeout_ms: u32) {
        if !self.added {
            self.show_icon();
            self.temporary = !self.visible;
        }
        if !self.added {
            return;
        }
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = if error { NIIF_ERROR } else { NIIF_INFO };
        data.Anonymous.uTimeout = timeout_ms;
        copy_wide(&mut data.szInfo, text);
        copy_wide(&mut data.szInfoTitle, "TGOW Recorder");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            if self.temporary {
                SetTimer(Some(self.hwnd), BALLOON_TIMER, timeout_ms + 4000, None);
            }
        }
    }

    fn drain(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Cmd::Bind(tx) => self.msg_tx = Some(tx),
                Cmd::TrayVisible(visible) => {
                    self.visible = visible;
                    if visible {
                        self.temporary = false;
                        self.show_icon();
                    } else {
                        self.hide_icon();
                    }
                }
                Cmd::TrayState(state) => {
                    self.tray = state;
                    self.refresh_icon();
                    if self.added {
                        self.show_icon();
                    }
                }
                Cmd::Refresh => {
                    if self.added {
                        self.show_icon();
                    }
                }
                Cmd::Hotkeys(bindings) => self.set_hotkeys(bindings),
                Cmd::Balloon { error, text, timeout_ms } => self.balloon(error, &text, timeout_ms),
            }
        }
    }

    fn timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), BALLOON_TIMER);
        }
        if self.temporary && !self.visible {
            self.hide_icon();
        }
    }

    fn taskbar_restarted(&mut self) {
        if self.added {
            self.added = false;
            self.show_icon();
        }
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn show_menu(hwnd: HWND) {
    let Some((entries, tx)) = with_state(|s| (s.tray.menu(), s.msg_tx.clone())) else { return };
    let mut commands: Vec<MenuCommand> = Vec::new();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for entry in &entries {
            match entry {
                MenuEntry::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                MenuEntry::Item(label, command) => {
                    commands.push(*command);
                    let text = wide(label);
                    let _ = AppendMenuW(menu, MF_STRING, commands.len(), PCWSTR(text.as_ptr()));
                }
            }
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if picked == 0 {
            return;
        }
        if let (Some(command), Some(tx)) = (commands.get(picked - 1), tx) {
            let _ = tx.send(match command {
                MenuCommand::Run(action) => Msg::Action(*action),
                MenuCommand::Quit => Msg::Quit,
            });
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMANDS => {
            with_state(|s| s.drain());
            LRESULT(0)
        }
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => {
                    with_state(|s| s.send(Msg::Action(Action::ShowHide)));
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            with_state(|s| {
                if let Some((_, action)) = s.hotkeys.iter().find(|(i, _)| *i == id) {
                    s.send(Msg::Action(*action));
                }
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == BALLOON_TIMER => {
            with_state(|s| s.timer());
            LRESULT(0)
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_state(|s| s.taskbar_restarted());
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes() {
        assert_eq!(vk_for("F9"), Some(0x78));
        assert_eq!(vk_for("Z"), Some(0x5a));
        assert_eq!(vk_for("Num1"), Some(0x31));
        assert_eq!(vk_for("Space"), Some(0x20));
        assert_eq!(vk_for("Nope"), None);
    }
}
