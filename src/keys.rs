//! Parsing shortcuts like "ctrl+space" and holding them down system-wide.

use std::sync::Mutex;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Hotkey {
    /// Virtual-key codes, pressed in this order and released in reverse.
    keys: Vec<u16>,
}

/// The shortcut currently held down, so it can always be let go: on stop,
/// quit, log-off or a panic.
static HELD: Mutex<Option<Hotkey>> = Mutex::new(None);

// The first name for each code is the one shown when recording a shortcut.
const NAMES: &[(&str, u16)] = &[
    ("ctrl", 0x11), ("control", 0x11), ("left ctrl", 0xA2), ("right ctrl", 0xA3),
    ("shift", 0x10), ("left shift", 0xA0), ("right shift", 0xA1),
    ("alt", 0x12), ("left alt", 0xA4), ("right alt", 0xA5), ("alt gr", 0xA5),
    ("win", 0x5B), ("windows", 0x5B), ("left windows", 0x5B), ("right windows", 0x5C),
    ("space", 0x20), ("enter", 0x0D), ("return", 0x0D), ("tab", 0x09),
    ("esc", 0x1B), ("escape", 0x1B), ("backspace", 0x08),
    ("insert", 0x2D), ("delete", 0x2E), ("del", 0x2E), ("home", 0x24), ("end", 0x23),
    ("page up", 0x21), ("page down", 0x22),
    ("up", 0x26), ("down", 0x28), ("left", 0x25), ("right", 0x27),
    ("caps lock", 0x14), ("scroll lock", 0x91), ("num lock", 0x90), ("pause", 0x13),
    ("print screen", 0x2C), ("menu", 0x5D),
    ("num *", 0x6A), ("num +", 0x6B), ("num -", 0x6D), ("num .", 0x6E), ("num /", 0x6F),
    (";", 0xBA), ("=", 0xBB), (",", 0xBC), ("-", 0xBD), (".", 0xBE), ("/", 0xBF),
    ("`", 0xC0), ("[", 0xDB), ("\\", 0xDC), ("]", 0xDD), ("'", 0xDE),
];

fn vk_for(name: &str) -> Option<u16> {
    if let Some(&(_, vk)) = NAMES.iter().find(|(n, _)| *n == name) {
        return Some(vk);
    }
    let b = name.as_bytes();
    match b {
        [c @ b'a'..=b'z'] => Some(c.to_ascii_uppercase() as u16),
        [c @ b'0'..=b'9'] => Some(*c as u16),
        _ => {
            if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
                return (1..=24).contains(&n).then(|| 0x70 + n - 1);
            }
            let n = name.strip_prefix("num ")?.parse::<u16>().ok()?;
            (n <= 9).then(|| 0x60 + n)
        }
    }
}

fn name_for(vk: u16) -> String {
    match vk {
        0x41..=0x5A | 0x30..=0x39 => ((vk as u8) as char).to_ascii_lowercase().to_string(),
        0x70..=0x87 => format!("f{}", vk - 0x70 + 1),
        0x60..=0x69 => format!("num {}", vk - 0x60),
        _ => NAMES
            .iter()
            .find(|(_, v)| *v == vk)
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| format!("vk{vk}")),
    }
}

fn is_extended(vk: u16) -> bool {
    matches!(
        vk,
        0xA3 | 0xA5 | 0x5B | 0x5C | 0x5D | 0x2D | 0x2E | 0x24 | 0x23 | 0x21 | 0x22
            | 0x25..=0x28 | 0x6F | 0x90 | 0x2C
    )
}

pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

impl Hotkey {
    pub fn parse(s: &str) -> Result<Hotkey, String> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            return Err("it's empty".into());
        }
        let mut keys = Vec::new();
        for part in s.split('+') {
            let part = part.split_whitespace().collect::<Vec<_>>().join(" ");
            if part.is_empty() {
                return Err("it has an empty part".into());
            }
            keys.push(vk_for(&part).ok_or_else(|| format!("unknown key '{part}'"))?);
        }
        Ok(Hotkey { keys })
    }
}

/// The shortcut being pressed right now, ending in `vk`, as "ctrl+shift+f9".
pub fn combo_with(vk: u16) -> String {
    let down = |k: i32| unsafe { GetKeyState(k) } < 0;
    let mut parts = Vec::new();
    if down(0x11) { parts.push("ctrl".to_string()); }
    if down(0x10) { parts.push("shift".to_string()); }
    if down(0x12) { parts.push("alt".to_string()); }
    if down(0x5B) || down(0x5C) { parts.push("win".to_string()); }
    parts.push(name_for(vk));
    parts.join("+")
}

fn send(vks: impl Iterator<Item = u16>, up: bool) {
    let inputs: Vec<INPUT> = vks
        .map(|vk| {
            let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
            if is_extended(vk) {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        })
        .collect();
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

fn held() -> std::sync::MutexGuard<'static, Option<Hotkey>> {
    HELD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Press and keep holding `h`. Does nothing if a shortcut is already held.
pub fn hold(h: &Hotkey) {
    let mut held = held();
    if held.is_none() {
        send(h.keys.iter().copied(), false);
        *held = Some(h.clone());
    }
}

/// Press and release `h` once. The short gap lets apps that poll key state
/// see it down.
pub fn tap(h: &Hotkey) {
    send(h.keys.iter().copied(), false);
    std::thread::sleep(std::time::Duration::from_millis(20));
    send(h.keys.iter().rev().copied(), true);
}

/// Release the held shortcut, if any.
pub fn let_go() {
    if let Some(h) = held().take() {
        send(h.keys.iter().rev().copied(), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcuts() {
        assert_eq!(Hotkey::parse("ctrl+space").unwrap().keys, vec![0x11, 0x20]);
        assert_eq!(Hotkey::parse(" Ctrl + Shift + F9 ").unwrap().keys, vec![0x11, 0x10, 0x78]);
        assert_eq!(Hotkey::parse("right ctrl").unwrap().keys, vec![0xA3]);
        assert_eq!(Hotkey::parse("alt+num 5").unwrap().keys, vec![0x12, 0x65]);
        assert_eq!(Hotkey::parse("win+a").unwrap().keys, vec![0x5B, 0x41]);
        assert!(Hotkey::parse("").is_err());
        assert!(Hotkey::parse("ctrl+").is_err());
        assert!(Hotkey::parse("ctrl+banana").is_err());
        assert!(Hotkey::parse("f25").is_err());
    }

    #[test]
    fn names_round_trip() {
        for vk in [0x20, 0x41, 0x35, 0x70, 0x87, 0x63, 0xBA, 0x2E] {
            assert_eq!(vk_for(&name_for(vk)), Some(vk), "vk {vk:#x}");
        }
    }
}
