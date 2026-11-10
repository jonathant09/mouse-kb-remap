//! Profiles, bindings and on-disk configuration (JSON).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    WheelUp,
    WheelDown,
}

impl MouseButton {
    pub const ALL: [MouseButton; 7] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::WheelUp,
        MouseButton::WheelDown,
        MouseButton::Back,
        MouseButton::Forward,
    ];

    pub fn id(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            MouseButton::Back => "back",
            MouseButton::Forward => "forward",
            MouseButton::WheelUp => "wheel_up",
            MouseButton::WheelDown => "wheel_down",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "Left click",
            MouseButton::Right => "Right click",
            MouseButton::Middle => "Wheel click",
            MouseButton::Back => "Back",
            MouseButton::Forward => "Forward",
            MouseButton::WheelUp => "Scroll up",
            MouseButton::WheelDown => "Scroll down",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            MouseButton::Left => "Button 1",
            MouseButton::Right => "Button 2",
            MouseButton::Middle => "Button 3",
            MouseButton::Back => "Mouse 4 · XButton1",
            MouseButton::Forward => "Mouse 5 · XButton2",
            MouseButton::WheelUp => "Wheel",
            MouseButton::WheelDown => "Wheel",
        }
    }
}

/// What an input does when it is remapped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Send a keyboard key (by key id, see `keys.rs`).
    Key { key: String },
    /// Send a mouse button / wheel step.
    Mouse { button: MouseButton },
    /// Swallow the input entirely.
    Disabled,
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Action::Key { key } => crate::keys::display_name(key),
            Action::Mouse { button } => button.name().to_string(),
            Action::Disabled => "Disabled".to_string(),
        }
    }
}

/// Input ids look like `key:A`, `key:F1`, `mouse:back`.
pub fn input_label(input: &str) -> String {
    if let Some(k) = input.strip_prefix("key:") {
        crate::keys::display_name(k)
    } else if let Some(m) = input.strip_prefix("mouse:") {
        MouseButton::from_id(m)
            .map(|b| b.name().to_string())
            .unwrap_or_else(|| m.to_string())
    } else {
        input.to_string()
    }
}

pub fn input_detail(input: &str) -> String {
    if input.starts_with("key:") {
        "Keyboard key".to_string()
    } else if let Some(m) = input.strip_prefix("mouse:") {
        MouseButton::from_id(m)
            .map(|b| b.detail().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Profile {
    pub name: String,
    /// Executable names (e.g. `GenshinImpact.exe`) that activate this profile
    /// while their window is focused. Empty for the Default profile.
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub bindings: BTreeMap<String, Action>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `profiles[0]` is always the Default profile: it applies when no other
    /// profile matches, and unbound inputs in other profiles fall back to it.
    pub profiles: Vec<Profile>,
}

fn default_version() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let mut genshin = BTreeMap::new();
        genshin.insert("mouse:back".to_string(), Action::Key { key: "F".into() });
        genshin.insert("mouse:forward".to_string(), Action::Key { key: "E".into() });
        Config {
            version: 1,
            theme: ThemeMode::Dark,
            enabled: true,
            profiles: vec![
                Profile {
                    name: "Default".into(),
                    apps: vec![],
                    bindings: BTreeMap::new(),
                },
                Profile {
                    name: "Genshin Impact".into(),
                    apps: vec!["GenshinImpact.exe".into()],
                    bindings: genshin,
                },
            ],
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mouse-kb-remap").map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Config {
        let Some(path) = Self::path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(mut cfg) => {
                    if cfg.profiles.is_empty() {
                        cfg.profiles = Config::default().profiles;
                    }
                    cfg
                }
                Err(err) => {
                    eprintln!("config: failed to parse {}: {err}; using defaults", path.display());
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                // Write to a temp file first so a crash never leaves a half-written config.
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, text).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(err) => eprintln!("config: failed to serialize: {err}"),
        }
    }

    /// Index of the profile that should be active for a focused executable.
    pub fn profile_for_exe(&self, exe: &str) -> usize {
        self.profiles
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, p)| p.apps.iter().any(|a| a.eq_ignore_ascii_case(exe)))
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}
