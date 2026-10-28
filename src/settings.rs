//! Settings in %APPDATA%\Handy Shortcut\settings.json (same file and fields
//! as the original Python version, so existing settings carry over).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Substring of the input device name.
    pub device: String,
    /// 1-based channel within that device.
    pub channel: u32,
    /// Must match Handy's Transcribe Shortcut.
    pub hotkey: String,
    /// Treat downward thumps as presses.
    pub invert: bool,
    /// Start listening when the app opens.
    pub autostart: bool,
    pub click_level: f32,
    pub peak_min: f32,
    pub width_ms: f32,
    pub quiet_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            device: "Line 2 (Audient EVO4)".into(),
            channel: 1,
            hotkey: "ctrl+space".into(),
            invert: false,
            autostart: true,
            click_level: 0.3,
            peak_min: 0.9,
            width_ms: 2.3, // thumps are ~2.7 ms; speech spikes reach ~2.0 ms
            quiet_ms: 30,
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    base.join(crate::APP_NAME)
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir())?;
        let json = serde_json::to_string_pretty(self).expect("settings serialize");
        std::fs::write(data_dir().join("settings.json"), json)
    }

    /// Whether switching from `self` to `other` needs the listener restarted.
    pub fn listener_differs(&self, other: &Settings) -> bool {
        let strip = |s: &Settings| Settings { autostart: false, ..s.clone() };
        strip(self) != strip(other)
    }
}
