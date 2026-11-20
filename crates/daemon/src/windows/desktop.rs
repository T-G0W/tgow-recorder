use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GlobalFree, ERROR_FILE_NOT_FOUND, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("TGOW Recorder");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    unsafe {
        if !enabled {
            let rc = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            return if rc.is_ok() || rc == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(std::io::Error::from_raw_os_error(rc.0 as i32))
            };
        }
        let exe = std::env::current_exe()?;
        let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(std::iter::once(0)).collect();
        let rc = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(value.as_ptr().cast()),
            (value.len() * 2) as u32,
        );
        if rc.is_ok() {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(rc.0 as i32))
        }
    }
}

fn dib(img: &image::RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity(40 + (w * h * 4) as usize);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(w * h * 4).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = img.get_pixel(x, y).0;
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out
}

unsafe fn put(format: u32, bytes: &[u8]) -> bool {
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return false };
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(mem));
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem);
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(HGLOBAL(mem.0)));
            return false;
        }
        true
    }
}

pub fn copy_image(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else { return false };
    let Ok(decoded) = image::load_from_memory(&bytes) else { return false };
    let rgba = decoded.to_rgba8();
    let png = if path.extension().map(|e| e.eq_ignore_ascii_case("png")).unwrap_or(false) {
        bytes
    } else {
        let mut out = std::io::Cursor::new(Vec::new());
        if rgba.write_to(&mut out, image::ImageFormat::Png).is_err() {
            return false;
        }
        out.into_inner()
    };
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = put(CF_DIB.0 as u32, &dib(&rgba));
        let png_format = RegisterClipboardFormatW(w!("PNG"));
        if png_format != 0 {
            put(png_format, &png);
        }
        let _ = CloseClipboard();
        ok
    }
}

pub fn run_open_command(command: &str, path: &Path) {
    let command = command.trim();
    if command.is_empty() {
        return;
    }
    let line = format!("/C {command} \"{}\"", path.display());
    if let Ok(mut child) = Command::new("cmd")
        .raw_arg(line)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

pub fn blink_led() {
    std::thread::spawn(|| {
        tgow_common::system::set_scroll_lock_led(true);
        std::thread::sleep(std::time::Duration::from_millis(250));
        tgow_common::system::set_scroll_lock_led(false);
    });
}

pub fn allow_foreground() {
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}
