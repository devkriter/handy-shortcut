//! Small Windows helpers: wide strings, local time, run-at-login,
//! single-instance and session-end handling.

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, SYSTEMTIME, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows,
    FindWindowW, GetMessageW, GetWindowTextW, GetWindowThreadProcessId, LoadImageW, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SendMessageW, SetClassLongPtrW, SetForegroundWindow, ShowWindow,
    TranslateMessage, GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL, IMAGE_ICON, MSG, SM_CXICON, SM_CXSMICON, SW_RESTORE, WM_ENDSESSION,
    WM_QUERYENDSESSION, WM_SETICON, WNDCLASSW, WS_OVERLAPPED,
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

// ---- single instance and session end ---------------------------------------

/// Class of the hidden window that a second copy talks to.
const SESSION_CLASS: &str = "HandyShortcut.Session";

/// Message that asks the running copy to show its window.
fn show_message() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(wide("HandyShortcut.Show").as_ptr()) })
}

static ON_SHOW: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Start a hidden top-level window on its own thread. It lets go of the
/// shortcut at log-off/shutdown (only top-level windows are told about that)
/// and calls `on_show` when a second copy of the app is started.
pub fn spawn_session_window(on_show: impl Fn() + Send + Sync + 'static) {
    let _ = ON_SHOW.set(Box::new(on_show));
    std::thread::spawn(|| unsafe {
        let class = wide(SESSION_CLASS);
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(session_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let title = wide("Handy Shortcut session");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

unsafe extern "system" fn session_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_QUERYENDSESSION => {
            crate::keys::let_go();
            1
        }
        WM_ENDSESSION => {
            crate::keys::let_go();
            0
        }
        m if m == show_message() => {
            if let Some(f) = ON_SHOW.get() {
                f();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
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

/// This process's top-level window titled `title`, if any.
pub fn own_window(title: &str) -> Option<isize> {
    struct Search {
        title: Vec<u16>,
        found: HWND,
    }
    unsafe extern "system" fn check(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut buf = [0u16; 128];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) as usize;
        if pid == GetCurrentProcessId() && buf[..len] == search.title[..] {
            search.found = hwnd;
            return 0; // stop
        }
        1
    }
    let mut search = Search { title: title.encode_utf16().collect(), found: std::ptr::null_mut() };
    unsafe { EnumWindows(Some(check), &mut search as *mut Search as LPARAM) };
    (!search.found.is_null()).then_some(search.found as isize)
}

/// Give `hwnd` the .exe's own icon (embedded by build.rs) at the sizes its
/// monitor wants. It's set on the window class too: the UI toolkit resets
/// the window's own icon when it updates the window, and Windows then falls
/// back to the class icon.
pub fn set_window_icon(hwnd: isize) {
    let hwnd = hwnd as HWND;
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let dpi = GetDpiForWindow(hwnd).max(96);
        let sizes = [(ICON_BIG, GCLP_HICON, SM_CXICON), (ICON_SMALL, GCLP_HICONSM, SM_CXSMICON)];
        for (which, class_slot, metric) in sizes {
            let size = GetSystemMetricsForDpi(metric, dpi);
            // Resource id 1, which winresource gives the icon.
            let icon = LoadImageW(instance, 1 as _, IMAGE_ICON, size, size, 0);
            if !icon.is_null() {
                SetClassLongPtrW(hwnd, class_slot, icon as isize);
                SendMessageW(hwnd, WM_SETICON, which as WPARAM, icon as LPARAM);
            }
        }
    }
}

/// Restore `hwnd` if minimized and bring it to the front.
pub fn bring_to_front(hwnd: isize) {
    let hwnd = hwnd as HWND;
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

/// Ask the copy that's already running to show its window.
pub fn show_running_instance() {
    let class = wide(SESSION_CLASS);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        AllowSetForegroundWindow(pid);
        PostMessageW(hwnd, show_message(), 0, 0);
    }
}
