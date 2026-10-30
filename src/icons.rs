//! Tray icons, one per state, drawn by art.rs. (The window uses the .exe's
//! own icon; see util::set_window_icon.)

use tray_icon::Icon;

use crate::art;

/// Tray icons are drawn at 32 px; Windows scales them to the tray size.
const TRAY_SIZE: u32 = 32;

pub struct Icons {
    pub stopped: Icon,
    pub listening: Icon,
    pub holding: Icon,
    pub error: Icon,
}

fn tray(color: [u8; 3]) -> Icon {
    Icon::from_rgba(art::render(TRAY_SIZE, color), TRAY_SIZE, TRAY_SIZE)
        .expect("tray icon pixels match their size")
}

pub fn load() -> Icons {
    Icons {
        stopped: tray(art::STOPPED),
        listening: tray(art::LISTENING),
        holding: tray(art::HOLDING),
        error: tray(art::ERROR),
    }
}
