//! Handy Shortcut - turn a microphone's hardware push-to-talk button into a
//! held keyboard shortcut (e.g. Handy's Transcribe Shortcut).
//!
//! The computer can't see the button, but every press and release puts a
//! loud "thump" on the mic signal; see detector.rs.
//!
//! `--tray` starts hidden in the tray (used when starting with Windows).

#![windows_subsystem = "windows"]

mod app;
mod art;
mod bridge;
mod detector;
mod icons;
mod keys;
mod settings;
mod util;

pub const APP_NAME: &str = "Handy Shortcut";

fn main() {
    // Never leave the shortcut stuck down system-wide.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        keys::let_go();
        let path = settings::data_dir().join("handy_shortcut.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let (date, time) = util::now();
            let _ = writeln!(f, "{date} [{time}] crashed: {info}");
        }
        default_hook(info);
    }));

    if !util::claim_single_instance() {
        util::show_running_instance();
        return;
    }

    let tray_only = std::env::args().any(|a| a == "--tray");
    nwg::init().expect("Failed to start the Windows UI");
    let app = app::App::build(tray_only).expect("Failed to build the window");
    nwg::dispatch_thread_events();
    app.shutdown();
}

use native_windows_gui as nwg;
