use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, GetKeyState, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_SCROLL,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos, SHGetKnownFolderPath, ShellExecuteW, KF_FLAG_DEFAULT,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::system::LocalTime;

pub const EXE_SUFFIX: &str = ".exe";

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let len = (0..).take_while(|&i| *p.0.add(i) != 0).count();
        let path = PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(p.0, len)));
        CoTaskMemFree(Some(p.0 as *const _));
        Some(path)
    }
}

pub fn home() -> PathBuf {
    known_folder(&FOLDERID_Profile)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
}

pub fn config_base() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData\\Roaming")).join("T-GOW")
}

pub fn cache_base() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData\\Local")).join("T-GOW")
}

pub fn runtime_base() -> PathBuf {
    cache_base()
}

pub fn videos_dir() -> PathBuf {
    known_folder(&FOLDERID_Videos).unwrap_or_else(|| home().join("Videos"))
}

pub fn pictures_dir() -> PathBuf {
    known_folder(&FOLDERID_Pictures).unwrap_or_else(|| home().join("Pictures"))
}

pub fn free_space_bytes(path: &Path) -> Option<u64> {
    let w = wide(path.as_os_str());
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut free), None, None).ok()? };
    Some(free)
}

pub fn now_local() -> LocalTime {
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    LocalTime {
        year: st.wYear as i32,
        month: st.wMonth as u32,
        day: st.wDay as u32,
        hour: st.wHour as u32,
        minute: st.wMinute as u32,
        second: st.wSecond as u32,
    }
}

pub fn is_wayland() -> bool {
    false
}

pub fn gpu_vendors() -> Vec<&'static str> {
    let mut found = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return found };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            if let Ok(desc) = adapter.GetDesc1() {
                found.push(crate::system::vendor_name(desc.VendorId));
            }
            i += 1;
        }
    }
    found
}

pub fn on_ac_power() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return true;
    }
    s.ACLineStatus != 0
}

pub fn set_scroll_lock_led(on: bool) {
    let lit = unsafe { GetKeyState(VK_SCROLL.0 as i32) } & 1 == 1;
    if lit != on {
        unsafe {
            keybd_event(VK_SCROLL.0 as u8, 0x46, KEYEVENTF_EXTENDEDKEY, 0);
            keybd_event(VK_SCROLL.0 as u8, 0x46, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP, 0);
        }
    }
}

fn shell(verb: &str, file: &std::ffi::OsStr, params: Option<&std::ffi::OsStr>) {
    let verb = wide(std::ffi::OsStr::new(verb));
    let file = wide(file);
    let params = params.map(wide);
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            params.as_ref().map(|p| PCWSTR(p.as_ptr())).unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn open_path(path: &Path) {
    shell("open", path.as_os_str(), None);
}

pub fn reveal_path(path: &Path) {
    let arg = format!("/select,\"{}\"", path.display());
    shell("open", std::ffi::OsStr::new("explorer.exe"), Some(std::ffi::OsStr::new(&arg)));
}
