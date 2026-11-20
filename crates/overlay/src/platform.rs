#[derive(Clone, Copy)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

#[cfg(unix)]
pub fn local_stamp(secs: u64) -> Stamp {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    Stamp {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    }
}

#[cfg(windows)]
pub fn local_stamp(secs: u64) -> Stamp {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

    let ticks = (secs + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            local = utc;
        }
    }
    Stamp {
        year: local.wYear as i32,
        month: local.wMonth as u32,
        day: local.wDay as u32,
        hour: local.wHour as u32,
        minute: local.wMinute as u32,
    }
}

#[cfg(unix)]
pub fn capture_screen() -> Option<crate::backdrop::Shot> {
    None
}

#[cfg(windows)]
pub fn capture_screen() -> Option<crate::backdrop::Shot> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, MonitorFromPoint,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, MONITORINFO,
        MONITOR_DEFAULTTONEAREST, SRCCOPY,
    };
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            return None;
        }
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let mut shot = None;
        if let Ok(bitmap) = CreateDIBSection(Some(memory), &header, DIB_RGB_COLORS, &mut bits, None, 0) {
            let previous = SelectObject(memory, bitmap.into());
            if BitBlt(memory, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok() && !bits.is_null()
            {
                let len = (w * h * 4) as usize;
                let bgra = std::slice::from_raw_parts(bits as *const u8, len);
                let mut rgba = Vec::with_capacity(len);
                for px in bgra.as_chunks::<4>().0 {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
                shot = Some(crate::backdrop::Shot { width: w as usize, height: h as usize, rgba });
            }
            SelectObject(memory, previous);
            let _ = DeleteObject(bitmap.into());
        }
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        shot
    }
}

#[cfg(unix)]
pub fn bring_to_front(_frame: &eframe::Frame) {}

#[cfg(windows)]
pub fn bring_to_front(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetCursorPos, SetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_SHOWWINDOW,
    };

    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else { return };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcMonitor;
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_SHOWWINDOW,
            );
        }
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

#[cfg(unix)]
pub fn place_toast(_frame: &eframe::Frame, _hidden: f32, _first: bool) {}

#[cfg(windows)]
pub fn place_toast(frame: &eframe::Frame, hidden: f32, first: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateRoundRectRgn, GetMonitorInfoW, MonitorFromPoint, SetWindowRgn, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE,
        SWP_NOSIZE, SWP_SHOWWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    };

    let Ok(handle) = frame.window_handle() else { return };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else { return };
    let hwnd = HWND(raw.hwnd.get() as *mut _);
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        if first {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let extra = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | extra);
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let margin = (24.0 * scale).round() as i32;
                let x = info.rcWork.right - w - margin;
                let y = info.rcWork.top + margin;
                let _ =
                    SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
        }
        let left = ((hidden.clamp(0.0, 1.0)) * w as f32).round() as i32;
        let r = (crate::toast::RADIUS * 2.0 * scale).round() as i32;
        let region = CreateRoundRectRgn(left, 0, w + 1, h + 1, r, r);
        SetWindowRgn(hwnd, Some(region), true);
    }
}

pub fn hide_console(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}
