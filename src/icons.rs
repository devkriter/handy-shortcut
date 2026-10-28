//! Window and tray icons, one per state, drawn by art.rs at the size Windows
//! wants for this display.

use native_windows_gui as nwg;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, GetSystemMetrics, SM_CXICON, SM_CXSMICON,
};

use crate::art;

#[derive(Default)]
pub struct Icons {
    pub app: nwg::Icon,
    pub app_small: nwg::Icon,
    pub stopped: nwg::Icon,
    pub listening: nwg::Icon,
    pub holding: nwg::Icon,
    pub error: nwg::Icon,
}

fn make(size: i32, color: [u8; 3]) -> nwg::Icon {
    let bits = art::bmp_entry(size as u32, color);
    let handle = unsafe {
        CreateIconFromResourceEx(bits.as_ptr(), bits.len() as u32, 1, 0x0003_0000, size, size, 0)
    };
    let mut icon = nwg::Icon::default();
    icon.handle = handle as _;
    icon
}

pub fn load() -> Icons {
    let big = unsafe { GetSystemMetrics(SM_CXICON) }.max(32);
    let small = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    Icons {
        app: make(big, art::APP),
        app_small: make(small, art::APP),
        stopped: make(small, art::STOPPED),
        listening: make(small, art::LISTENING),
        holding: make(small, art::HOLDING),
        error: make(small, art::ERROR),
    }
}
