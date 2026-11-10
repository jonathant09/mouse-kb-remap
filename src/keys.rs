//! Keyboard layout (generic ANSI TKL) and key metadata.
//!
//! Every key has a stable `id` used in config files ("A", "F1", "LShift", ...),
//! a display `label`, a Windows virtual-key code and its position on the
//! diagram in key units (1u = one standard key).

pub struct KeyDef {
    pub id: &'static str,
    pub label: &'static str,
    pub vk: u16,
    pub x: f32,
    pub y: f32,
    pub w: f32,
}

const fn k(id: &'static str, label: &'static str, vk: u16, x: f32, y: f32, w: f32) -> KeyDef {
    KeyDef { id, label, vk, x, y, w }
}

/// Generic 87-key ANSI layout. Total size: 18.25u x 6.25u.
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
];

pub fn find(id: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.id == id)
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
        _ => find(id).map(|k| k.label.to_string()).unwrap_or_else(|| id.to_string()),
    }
}

/// Map the `text` of a Slint `KeyEvent` to one of our key ids.
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
