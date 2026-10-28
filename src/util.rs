//! Small Windows helpers: wide strings, local time, run-at-login and
//! single-instance handling.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, SYSTEMTIME};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
    RegisterWindowMessageW,
};

use crate::APP_NAME;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// ("2026-10-27", "22:28:00") in local time.
pub fn now() -> (String, String) {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    (
        format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay),
        format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond),
    )
}

// ---- start with Windows ----------------------------------------------------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn startup_command() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("\"{}\" --tray", exe.display())
}

fn startup_value() -> Option<String> {
    let (key, name) = (wide(RUN_KEY), wide(APP_NAME));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate((len as usize / 2).saturating_sub(1));
    Some(String::from_utf16_lossy(&buf))
}

pub fn starts_with_windows() -> bool {
    startup_value().is_some()
}

pub fn set_start_with_windows(on: bool) -> Result<(), String> {
    let (key, name) = (wide(RUN_KEY), wide(APP_NAME));
    let rc = unsafe {
        if on {
            let cmd = wide(&startup_command());
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                cmd.as_ptr().cast(),
                (cmd.len() * 2) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr())
        }
    };
    match rc {
        0 => Ok(()),
        2 if !on => Ok(()), // wasn't set
        e => Err(format!("registry error {e}")),
    }
}

/// If start-with-Windows is on but points at an old copy of the exe, point it
/// at this one.
pub fn refresh_startup_path() {
    if startup_value().is_some_and(|v| v != startup_command()) {
        let _ = set_start_with_windows(true);
    }
}

// ---- single instance -------------------------------------------------------

/// Message that asks the running copy to show its window.
pub fn show_message() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(wide("HandyShortcut.Show").as_ptr()) })
}

/// True if this is the only copy running. The mutex is kept for the life of
/// the process.
pub fn claim_single_instance() -> bool {
    let name = wide("Local\\HandyShortcut.SingleInstance");
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        !(h.is_null() || GetLastError() == ERROR_ALREADY_EXISTS)
    }
}

/// Ask the copy that's already running to show its window.
pub fn show_running_instance() {
    let (class, title) = (wide("NativeWindowsGuiWindow"), wide(APP_NAME));
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), title.as_ptr());
        if hwnd.is_null() {
            return;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        AllowSetForegroundWindow(pid);
        PostMessageW(hwnd, show_message(), 0, 0);
    }
}
