//! Settings in %APPDATA%\Handy Shortcut\settings.json (same file and fields
//! as the original Python version, so existing settings carry over).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What the shortcut does while the button is pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Hold the shortcut down from button press to release.
    #[default]
    Hold,
    /// Press the shortcut once on button press and once on release (for
    /// apps that toggle).
    TapBoth,
    /// Press the shortcut once on button press only.
    TapPress,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Hold, Mode::TapBoth, Mode::TapPress];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Hold => "Hold while button is down",
            Mode::TapBoth => "Press on button down and up",
            Mode::TapPress => "Press on button down only",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Substring of the input device name.
    pub device: String,
    /// 1-based channel within that device.
    pub channel: u32,
    /// Must match Handy's Transcribe Shortcut.
    pub hotkey: String,
    pub mode: Mode,
    /// Treat downward thumps as presses.
    pub invert: bool,
    /// Press Enter once the button is released and the shortcut has gone
    /// out, e.g. to send the transcribed message in Claude Code.
    pub enter_after: bool,
    /// How long to wait before that Enter, so the transcript is in first.
    pub enter_delay_ms: u32,
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
            mode: Mode::Hold,
            invert: false,
            enter_after: false,
            enter_delay_ms: 1000,
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
