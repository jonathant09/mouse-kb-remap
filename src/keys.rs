//! Keyboard layout (generic ANSI, TKL or full size) and key metadata.
//!
//! Every key has a stable `id` used in config files ("A", "F1", "LShift", ...),
//! a display `label`, a Windows virtual-key code and, for keys drawn on the
//! diagram, its position in key units (1u = one standard key).

use crate::config::Modifier;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// Drawn on the TKL (87-key) diagram.
    Main,
    /// Drawn only on the full-size diagram.
    Numpad,
    /// Not drawn: F13-F24, media and browser keys. Usable as targets and,
    /// via "Detect key", as inputs.
    Extra,
}

pub struct KeyDef {
    pub id: &'static str,
    pub label: &'static str,
    pub vk: u16,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub group: Group,
}

const fn k(id: &'static str, label: &'static str, vk: u16, x: f32, y: f32, w: f32) -> KeyDef {
    KeyDef {
        id,
        label,
        vk,
        x,
        y,
        w,
        h: 1.0,
        group: Group::Main,
    }
}

const fn np(id: &'static str, label: &'static str, vk: u16, x: f32, y: f32, w: f32, h: f32) -> KeyDef {
    KeyDef {
        id,
        label,
        vk,
        x,
        y,
        w,
        h,
        group: Group::Numpad,
    }
}

const fn ex(id: &'static str, label: &'static str, vk: u16) -> KeyDef {
    KeyDef {
        id,
        label,
        vk,
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
        group: Group::Extra,
    }
}

/// Windows has no separate virtual key for the number-pad Enter (it is
/// VK_RETURN with the "extended" flag). We use the unassigned code 0x0E for it.
pub const VK_NUMPAD_ENTER: u16 = 0x0E;

/// TKL width / full-size width / height of the diagram in key units (the UI
/// uses the same numbers in ui/profile-view.slint).
#[cfg(test)]
pub const TKL_WIDTH: f32 = 18.25;
#[cfg(test)]
pub const FULL_WIDTH: f32 = 22.5;
#[cfg(test)]
pub const HEIGHT: f32 = 6.25;

pub const KEYS: &[KeyDef] = &[
    // Function row
    k("Esc", "Esc", 0x1B, 0.0, 0.0, 1.0),
    k("F1", "F1", 0x70, 2.0, 0.0, 1.0),
    k("F2", "F2", 0x71, 3.0, 0.0, 1.0),
    k("F3", "F3", 0x72, 4.0, 0.0, 1.0),
    k("F4", "F4", 0x73, 5.0, 0.0, 1.0),
    k("F5", "F5", 0x74, 6.5, 0.0, 1.0),
    k("F6", "F6", 0x75, 7.5, 0.0, 1.0),
    k("F7", "F7", 0x76, 8.5, 0.0, 1.0),
    k("F8", "F8", 0x77, 9.5, 0.0, 1.0),
    k("F9", "F9", 0x78, 11.0, 0.0, 1.0),
    k("F10", "F10", 0x79, 12.0, 0.0, 1.0),
    k("F11", "F11", 0x7A, 13.0, 0.0, 1.0),
    k("F12", "F12", 0x7B, 14.0, 0.0, 1.0),
    k("PrintScreen", "PrtSc", 0x2C, 15.25, 0.0, 1.0),
    k("ScrollLock", "ScrLk", 0x91, 16.25, 0.0, 1.0),
    k("Pause", "Pause", 0x13, 17.25, 0.0, 1.0),
    // Number row
    k("Grave", "`", 0xC0, 0.0, 1.25, 1.0),
    k("1", "1", 0x31, 1.0, 1.25, 1.0),
    k("2", "2", 0x32, 2.0, 1.25, 1.0),
    k("3", "3", 0x33, 3.0, 1.25, 1.0),
    k("4", "4", 0x34, 4.0, 1.25, 1.0),
    k("5", "5", 0x35, 5.0, 1.25, 1.0),
    k("6", "6", 0x36, 6.0, 1.25, 1.0),
    k("7", "7", 0x37, 7.0, 1.25, 1.0),
    k("8", "8", 0x38, 8.0, 1.25, 1.0),
    k("9", "9", 0x39, 9.0, 1.25, 1.0),
    k("0", "0", 0x30, 10.0, 1.25, 1.0),
    k("Minus", "-", 0xBD, 11.0, 1.25, 1.0),
    k("Equal", "=", 0xBB, 12.0, 1.25, 1.0),
    k("Backspace", "Backspace", 0x08, 13.0, 1.25, 2.0),
    k("Insert", "Ins", 0x2D, 15.25, 1.25, 1.0),
    k("Home", "Home", 0x24, 16.25, 1.25, 1.0),
    k("PageUp", "PgUp", 0x21, 17.25, 1.25, 1.0),
    // Top letter row
    k("Tab", "Tab", 0x09, 0.0, 2.25, 1.5),
    k("Q", "Q", 0x51, 1.5, 2.25, 1.0),
    k("W", "W", 0x57, 2.5, 2.25, 1.0),
    k("E", "E", 0x45, 3.5, 2.25, 1.0),
    k("R", "R", 0x52, 4.5, 2.25, 1.0),
    k("T", "T", 0x54, 5.5, 2.25, 1.0),
    k("Y", "Y", 0x59, 6.5, 2.25, 1.0),
    k("U", "U", 0x55, 7.5, 2.25, 1.0),
    k("I", "I", 0x49, 8.5, 2.25, 1.0),
    k("O", "O", 0x4F, 9.5, 2.25, 1.0),
    k("P", "P", 0x50, 10.5, 2.25, 1.0),
    k("LBracket", "[", 0xDB, 11.5, 2.25, 1.0),
    k("RBracket", "]", 0xDD, 12.5, 2.25, 1.0),
    k("Backslash", "\\", 0xDC, 13.5, 2.25, 1.5),
    k("Delete", "Del", 0x2E, 15.25, 2.25, 1.0),
    k("End", "End", 0x23, 16.25, 2.25, 1.0),
    k("PageDown", "PgDn", 0x22, 17.25, 2.25, 1.0),
    // Home row
    k("CapsLock", "Caps", 0x14, 0.0, 3.25, 1.75),
    k("A", "A", 0x41, 1.75, 3.25, 1.0),
    k("S", "S", 0x53, 2.75, 3.25, 1.0),
    k("D", "D", 0x44, 3.75, 3.25, 1.0),
    k("F", "F", 0x46, 4.75, 3.25, 1.0),
    k("G", "G", 0x47, 5.75, 3.25, 1.0),
    k("H", "H", 0x48, 6.75, 3.25, 1.0),
    k("J", "J", 0x4A, 7.75, 3.25, 1.0),
    k("K", "K", 0x4B, 8.75, 3.25, 1.0),
    k("L", "L", 0x4C, 9.75, 3.25, 1.0),
    k("Semicolon", ";", 0xBA, 10.75, 3.25, 1.0),
    k("Quote", "'", 0xDE, 11.75, 3.25, 1.0),
    k("Enter", "Enter", 0x0D, 12.75, 3.25, 2.25),
    // Bottom letter row
    k("LShift", "Shift", 0xA0, 0.0, 4.25, 2.25),
    k("Z", "Z", 0x5A, 2.25, 4.25, 1.0),
    k("X", "X", 0x58, 3.25, 4.25, 1.0),
    k("C", "C", 0x43, 4.25, 4.25, 1.0),
    k("V", "V", 0x56, 5.25, 4.25, 1.0),
    k("B", "B", 0x42, 6.25, 4.25, 1.0),
    k("N", "N", 0x4E, 7.25, 4.25, 1.0),
    k("M", "M", 0x4D, 8.25, 4.25, 1.0),
    k("Comma", ",", 0xBC, 9.25, 4.25, 1.0),
    k("Period", ".", 0xBE, 10.25, 4.25, 1.0),
    k("Slash", "/", 0xBF, 11.25, 4.25, 1.0),
    k("RShift", "Shift", 0xA1, 12.25, 4.25, 2.75),
    k("Up", "↑", 0x26, 16.25, 4.25, 1.0),
    // Space row
    k("LCtrl", "Ctrl", 0xA2, 0.0, 5.25, 1.25),
    k("LWin", "Win", 0x5B, 1.25, 5.25, 1.25),
    k("LAlt", "Alt", 0xA4, 2.5, 5.25, 1.25),
    k("Space", "", 0x20, 3.75, 5.25, 6.25),
    k("RAlt", "Alt", 0xA5, 10.0, 5.25, 1.25),
    k("RWin", "Win", 0x5C, 11.25, 5.25, 1.25),
    k("Menu", "Menu", 0x5D, 12.5, 5.25, 1.25),
    k("RCtrl", "Ctrl", 0xA3, 13.75, 5.25, 1.25),
    k("Left", "←", 0x25, 15.25, 5.25, 1.0),
    k("Down", "↓", 0x28, 16.25, 5.25, 1.0),
    k("Right", "→", 0x27, 17.25, 5.25, 1.0),
    // Number pad (full-size layout only)
    np("NumLock", "Num", 0x90, 18.5, 1.25, 1.0, 1.0),
    np("NumpadDivide", "/", 0x6F, 19.5, 1.25, 1.0, 1.0),
    np("NumpadMultiply", "*", 0x6A, 20.5, 1.25, 1.0, 1.0),
    np("NumpadSubtract", "-", 0x6D, 21.5, 1.25, 1.0, 1.0),
    np("Numpad7", "7", 0x67, 18.5, 2.25, 1.0, 1.0),
    np("Numpad8", "8", 0x68, 19.5, 2.25, 1.0, 1.0),
    np("Numpad9", "9", 0x69, 20.5, 2.25, 1.0, 1.0),
    np("NumpadAdd", "+", 0x6B, 21.5, 2.25, 1.0, 2.0),
    np("Numpad4", "4", 0x64, 18.5, 3.25, 1.0, 1.0),
    np("Numpad5", "5", 0x65, 19.5, 3.25, 1.0, 1.0),
    np("Numpad6", "6", 0x66, 20.5, 3.25, 1.0, 1.0),
    np("Numpad1", "1", 0x61, 18.5, 4.25, 1.0, 1.0),
    np("Numpad2", "2", 0x62, 19.5, 4.25, 1.0, 1.0),
    np("Numpad3", "3", 0x63, 20.5, 4.25, 1.0, 1.0),
    np("NumpadEnter", "Ent", VK_NUMPAD_ENTER, 21.5, 4.25, 1.0, 2.0),
    np("Numpad0", "0", 0x60, 18.5, 5.25, 2.0, 1.0),
    np("NumpadDecimal", ".", 0x6E, 20.5, 5.25, 1.0, 1.0),
    // Extra keys (not drawn)
    ex("F13", "F13", 0x7C),
    ex("F14", "F14", 0x7D),
    ex("F15", "F15", 0x7E),
    ex("F16", "F16", 0x7F),
    ex("F17", "F17", 0x80),
    ex("F18", "F18", 0x81),
    ex("F19", "F19", 0x82),
    ex("F20", "F20", 0x83),
    ex("F21", "F21", 0x84),
    ex("F22", "F22", 0x85),
    ex("F23", "F23", 0x86),
    ex("F24", "F24", 0x87),
    ex("MediaPlayPause", "Play/Pause", 0xB3),
    ex("MediaNext", "Next track", 0xB0),
    ex("MediaPrev", "Previous track", 0xB1),
    ex("MediaStop", "Stop media", 0xB2),
    ex("VolumeUp", "Volume up", 0xAF),
    ex("VolumeDown", "Volume down", 0xAE),
    ex("VolumeMute", "Mute", 0xAD),
    ex("BrowserBack", "Browser back", 0xA6),
    ex("BrowserForward", "Browser forward", 0xA7),
    ex("BrowserRefresh", "Browser refresh", 0xA8),
    ex("BrowserStop", "Browser stop", 0xA9),
    ex("BrowserSearch", "Browser search", 0xAA),
    ex("BrowserFavorites", "Favorites", 0xAB),
    ex("BrowserHome", "Browser home", 0xAC),
    ex("LaunchMail", "Mail", 0xB4),
    ex("LaunchMedia", "Media player", 0xB5),
    ex("LaunchApp1", "This PC", 0xB6),
    ex("LaunchApp2", "Calculator", 0xB7),
];

/// Keys offered under the "Media" action.
pub const MEDIA: &[&str] = &[
    "MediaPlayPause",
    "MediaNext",
    "MediaPrev",
    "MediaStop",
    "VolumeUp",
    "VolumeDown",
    "VolumeMute",
    "BrowserBack",
    "BrowserForward",
    "BrowserRefresh",
    "BrowserHome",
    "BrowserSearch",
    "LaunchMail",
    "LaunchApp2",
    "LaunchApp1",
];

/// Common shortcuts offered as one-click presets under the "Key" action.
pub const SHORTCUTS: &[(&str, &str, &[Modifier])] = &[
    ("Copy", "C", &[Modifier::Ctrl]),
    ("Paste", "V", &[Modifier::Ctrl]),
    ("Cut", "X", &[Modifier::Ctrl]),
    ("Undo", "Z", &[Modifier::Ctrl]),
    ("Redo", "Y", &[Modifier::Ctrl]),
    ("Select all", "A", &[Modifier::Ctrl]),
    ("Save", "S", &[Modifier::Ctrl]),
    ("Find", "F", &[Modifier::Ctrl]),
    ("Close tab", "W", &[Modifier::Ctrl]),
    ("Reopen tab", "T", &[Modifier::Ctrl, Modifier::Shift]),
    ("Switch app", "Tab", &[Modifier::Alt]),
    ("Close window", "F4", &[Modifier::Alt]),
    ("Show desktop", "D", &[Modifier::Win]),
    ("Task view", "Tab", &[Modifier::Win]),
    ("File Explorer", "E", &[Modifier::Win]),
    ("Screenshot", "S", &[Modifier::Win, Modifier::Shift]),
    ("Task Manager", "Esc", &[Modifier::Ctrl, Modifier::Shift]),
    ("Emoji panel", "Period", &[Modifier::Win]),
];

pub fn find(id: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.id == id)
}

pub fn find_by_vk(vk: u16) -> Option<&'static KeyDef> {
    // Generic modifier codes map to the left-hand key.
    let vk = match vk {
        0x10 => 0xA0,
        0x11 => 0xA2,
        0x12 => 0xA4,
        v => v,
    };
    KEYS.iter().find(|k| k.vk == vk)
}

/// Keys drawn on the diagram for the chosen layout.
pub fn layout(full: bool) -> impl Iterator<Item = &'static KeyDef> {
    KEYS.iter()
        .filter(move |k| k.group == Group::Main || (full && k.group == Group::Numpad))
}

/// The modifier a virtual key belongs to, if any.
pub fn modifier_of(vk: u16) -> Option<Modifier> {
    match vk {
        0x10 | 0xA0 | 0xA1 => Some(Modifier::Shift),
        0x11 | 0xA2 | 0xA3 => Some(Modifier::Ctrl),
        0x12 | 0xA4 | 0xA5 => Some(Modifier::Alt),
        0x5B | 0x5C => Some(Modifier::Win),
        _ => None,
    }
}

/// Human-readable name for a key id (used in the inspector / callouts).
pub fn display_name(id: &str) -> String {
    match id {
        "Space" => "Space".into(),
        "LShift" => "Left Shift".into(),
        "RShift" => "Right Shift".into(),
        "LCtrl" => "Left Ctrl".into(),
        "RCtrl" => "Right Ctrl".into(),
        "LAlt" => "Left Alt".into(),
        "RAlt" => "Right Alt".into(),
        "LWin" => "Left Win".into(),
        "RWin" => "Right Win".into(),
        "Up" => "Up Arrow".into(),
        "Down" => "Down Arrow".into(),
        "Left" => "Left Arrow".into(),
        "Right" => "Right Arrow".into(),
        "NumLock" => "Num Lock".into(),
        "NumpadEnter" => "Num Enter".into(),
        _ => match find(id) {
            Some(k) if k.group == Group::Numpad => format!("Num {}", k.label),
            Some(k) => k.label.to_string(),
            None => id.to_string(),
        },
    }
}

/// Map the `text` of a Slint `KeyEvent` to one of our key ids. Only used when
/// the engine can't capture keys itself (UI preview on other platforms).
pub fn from_slint_text(text: &str) -> Option<&'static str> {
    use slint::platform::Key;

    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let special: &[(Key, &'static str)] = &[
        (Key::Escape, "Esc"),
        (Key::Tab, "Tab"),
        (Key::Backtab, "Tab"),
        (Key::Backspace, "Backspace"),
        (Key::Return, "Enter"),
        (Key::Delete, "Delete"),
        (Key::Insert, "Insert"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::UpArrow, "Up"),
        (Key::DownArrow, "Down"),
        (Key::LeftArrow, "Left"),
        (Key::RightArrow, "Right"),
        (Key::Shift, "LShift"),
        (Key::ShiftR, "RShift"),
        (Key::Control, "LCtrl"),
        (Key::ControlR, "RCtrl"),
        (Key::Alt, "LAlt"),
        (Key::AltGr, "RAlt"),
        (Key::Meta, "LWin"),
        (Key::MetaR, "RWin"),
        (Key::CapsLock, "CapsLock"),
        (Key::ScrollLock, "ScrollLock"),
        (Key::Pause, "Pause"),
        (Key::SysReq, "PrintScreen"),
        (Key::Menu, "Menu"),
        (Key::Space, "Space"),
        (Key::F1, "F1"),
        (Key::F2, "F2"),
        (Key::F3, "F3"),
        (Key::F4, "F4"),
        (Key::F5, "F5"),
        (Key::F6, "F6"),
        (Key::F7, "F7"),
        (Key::F8, "F8"),
        (Key::F9, "F9"),
        (Key::F10, "F10"),
        (Key::F11, "F11"),
        (Key::F12, "F12"),
        (Key::F13, "F13"),
        (Key::F14, "F14"),
        (Key::F15, "F15"),
        (Key::F16, "F16"),
        (Key::F17, "F17"),
        (Key::F18, "F18"),
        (Key::F19, "F19"),
        (Key::F20, "F20"),
        (Key::F21, "F21"),
        (Key::F22, "F22"),
        (Key::F23, "F23"),
        (Key::F24, "F24"),
    ];
    for (key, id) in special {
        if char::from(*key) == c {
            return Some(id);
        }
    }

    let id = match c.to_ascii_uppercase() {
        'A'..='Z' | '0'..='9' => return find(&c.to_ascii_uppercase().to_string()).map(|k| k.id),
        '`' | '~' => "Grave",
        '-' | '_' => "Minus",
        '=' | '+' => "Equal",
        '[' | '{' => "LBracket",
        ']' | '}' => "RBracket",
        '\\' | '|' => "Backslash",
        ';' | ':' => "Semicolon",
        '\'' | '"' => "Quote",
        ',' | '<' => "Comma",
        '.' | '>' => "Period",
        '/' | '?' => "Slash",
        '!' => "1",
        '@' => "2",
        '#' => "3",
        '$' => "4",
        '%' => "5",
        '^' => "6",
        '&' => "7",
        '*' => "8",
        '(' => "9",
        ')' => "0",
        _ => return None,
    };
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_vks_are_unique() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.vk, b.vk, "{} and {} share a VK", a.id, b.id);
            }
        }
    }

    #[test]
    fn presets_and_media_reference_real_keys() {
        for id in MEDIA {
            assert!(find(id).is_some(), "{id}");
        }
        for (_, key, _) in SHORTCUTS {
            assert!(find(key).is_some(), "{key}");
        }
    }

    #[test]
    fn layouts_fit_their_width() {
        for k in layout(true) {
            assert!(k.x + k.w <= FULL_WIDTH + 0.01, "{}", k.id);
            assert!(k.y + k.h <= HEIGHT + 0.01, "{}", k.id);
        }
        for k in layout(false) {
            assert!(k.x + k.w <= TKL_WIDTH + 0.01, "{}", k.id);
        }
        assert_eq!(layout(false).count(), 87);
        assert_eq!(layout(true).count(), 104);
    }

    #[test]
    fn find_by_vk_handles_generic_modifiers() {
        assert_eq!(find_by_vk(0x10).unwrap().id, "LShift");
        assert_eq!(find_by_vk(0xA3).unwrap().id, "RCtrl");
        assert_eq!(find_by_vk(VK_NUMPAD_ENTER).unwrap().id, "NumpadEnter");
        assert!(find_by_vk(0xFF).is_none());
    }
}
