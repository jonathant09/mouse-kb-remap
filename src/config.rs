//! Profiles, bindings, macros and on-disk configuration (JSON).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Current on-disk format. v2 added macros, turbo, combos and the shift layer;
/// v3 added layers 2-3, tap / hold, combos as triggers, system actions,
/// hotkeys and app rules; v4 added controllers, gestures and per-device
/// bindings. Older files load unchanged.
pub const CONFIG_VERSION: u32 = 4;

/// Binding layers per profile: base, shift (layer 1), layer 2, layer 3.
pub const LAYER_COUNT: usize = 4;

/// Name of a binding layer, as shown in the UI.
pub fn layer_name(layer: usize) -> &'static str {
    match layer {
        0 => "Base",
        1 => "Shift",
        2 => "Layer 2",
        _ => "Layer 3",
    }
}

/// Environment variable that points the app at a different config file
/// (portable installs, testing).
pub const CONFIG_ENV: &str = "MOUSE_KB_REMAP_CONFIG";

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
    WheelLeft,
    WheelRight,
}

impl MouseButton {
    pub const ALL: [MouseButton; 9] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::WheelUp,
        MouseButton::WheelDown,
        MouseButton::Back,
        MouseButton::Forward,
        MouseButton::WheelLeft,
        MouseButton::WheelRight,
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
            MouseButton::WheelLeft => "wheel_left",
            MouseButton::WheelRight => "wheel_right",
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
            MouseButton::WheelLeft => "Tilt left",
            MouseButton::WheelRight => "Tilt right",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            MouseButton::Left => "LMB",
            MouseButton::Right => "RMB",
            MouseButton::Middle => "MMB",
            MouseButton::Back => "M4",
            MouseButton::Forward => "M5",
            MouseButton::WheelUp => "Wh↑",
            MouseButton::WheelDown => "Wh↓",
            MouseButton::WheelLeft => "Wh←",
            MouseButton::WheelRight => "Wh→",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            MouseButton::Left => "Button 1",
            MouseButton::Right => "Button 2",
            MouseButton::Middle => "Button 3",
            MouseButton::Back => "Mouse 4 · XButton1",
            MouseButton::Forward => "Mouse 5 · XButton2",
            MouseButton::WheelUp | MouseButton::WheelDown => "Wheel",
            MouseButton::WheelLeft | MouseButton::WheelRight => "Wheel tilt (horizontal scroll)",
        }
    }

    /// Wheel "buttons" have no up/down, they are single steps.
    pub fn is_wheel(self) -> bool {
        matches!(
            self,
            MouseButton::WheelUp | MouseButton::WheelDown | MouseButton::WheelLeft | MouseButton::WheelRight
        )
    }
}

/// A game controller (gamepad) button, or a stick pushed in one direction,
/// as an input. Named by position, with Xbox names (A is the bottom face
/// button on every controller: Cross on PlayStation, B on Nintendo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadButton {
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Lt,
    Rt,
    Back,
    Start,
    Ls,
    Rs,
    Up,
    Down,
    Left,
    Right,
    LsUp,
    LsDown,
    LsLeft,
    LsRight,
    RsUp,
    RsDown,
    RsLeft,
    RsRight,
    /// Xbox / PS / Home / Steam button.
    Guide,
    /// Xbox Share, Switch Capture, Steam "…" (quick access).
    Share,
    /// PlayStation touchpad click, Steam right trackpad click.
    Touchpad,
    /// DualSense microphone button.
    Mic,
    /// Extra buttons: back paddles, Joy-Con SL / SR, Steam L4 / R4 / L5 / R5.
    P1,
    P2,
    P3,
    P4,
    /// Steam left trackpad click.
    LeftPad,
    /// Numbered button of a controller the app doesn't know (1-32).
    Button(u8),
}

impl PadButton {
    /// Every named input, in the order of their bits (see `engine::pad::bit`).
    pub const ALL: [PadButton; 33] = [
        PadButton::A,
        PadButton::B,
        PadButton::X,
        PadButton::Y,
        PadButton::Lb,
        PadButton::Rb,
        PadButton::Lt,
        PadButton::Rt,
        PadButton::Back,
        PadButton::Start,
        PadButton::Ls,
        PadButton::Rs,
        PadButton::Up,
        PadButton::Down,
        PadButton::Left,
        PadButton::Right,
        PadButton::LsUp,
        PadButton::LsDown,
        PadButton::LsLeft,
        PadButton::LsRight,
        PadButton::RsUp,
        PadButton::RsDown,
        PadButton::RsLeft,
        PadButton::RsRight,
        PadButton::Guide,
        PadButton::Share,
        PadButton::Touchpad,
        PadButton::Mic,
        PadButton::P1,
        PadButton::P2,
        PadButton::P3,
        PadButton::P4,
        PadButton::LeftPad,
    ];

    /// Highest numbered button.
    pub const MAX_NUMBERED: u8 = 32;

    pub fn id(self) -> String {
        let s = match self {
            PadButton::A => "a",
            PadButton::B => "b",
            PadButton::X => "x",
            PadButton::Y => "y",
            PadButton::Lb => "lb",
            PadButton::Rb => "rb",
            PadButton::Lt => "lt",
            PadButton::Rt => "rt",
            PadButton::Back => "back",
            PadButton::Start => "start",
            PadButton::Ls => "ls",
            PadButton::Rs => "rs",
            PadButton::Up => "up",
            PadButton::Down => "down",
            PadButton::Left => "left",
            PadButton::Right => "right",
            PadButton::LsUp => "ls_up",
            PadButton::LsDown => "ls_down",
            PadButton::LsLeft => "ls_left",
            PadButton::LsRight => "ls_right",
            PadButton::RsUp => "rs_up",
            PadButton::RsDown => "rs_down",
            PadButton::RsLeft => "rs_left",
            PadButton::RsRight => "rs_right",
            PadButton::Guide => "guide",
            PadButton::Share => "share",
            PadButton::Touchpad => "touchpad",
            PadButton::Mic => "mic",
            PadButton::P1 => "p1",
            PadButton::P2 => "p2",
            PadButton::P3 => "p3",
            PadButton::P4 => "p4",
            PadButton::LeftPad => "left_pad",
            PadButton::Button(n) => return format!("button{n}"),
        };
        s.to_string()
    }

    pub fn from_id(id: &str) -> Option<Self> {
        if let Some(n) = id.strip_prefix("button") {
            return n
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=Self::MAX_NUMBERED).contains(n))
                .map(PadButton::Button);
        }
        Self::ALL.into_iter().find(|b| b.id() == id)
    }

    pub fn name(self) -> String {
        let s = match self {
            PadButton::A => "A",
            PadButton::B => "B",
            PadButton::X => "X",
            PadButton::Y => "Y",
            PadButton::Lb => "Left bumper",
            PadButton::Rb => "Right bumper",
            PadButton::Lt => "Left trigger",
            PadButton::Rt => "Right trigger",
            PadButton::Back => "View / Back",
            PadButton::Start => "Menu / Start",
            PadButton::Ls => "Left stick click",
            PadButton::Rs => "Right stick click",
            PadButton::Up => "D-pad up",
            PadButton::Down => "D-pad down",
            PadButton::Left => "D-pad left",
            PadButton::Right => "D-pad right",
            PadButton::LsUp => "Left stick up",
            PadButton::LsDown => "Left stick down",
            PadButton::LsLeft => "Left stick left",
            PadButton::LsRight => "Left stick right",
            PadButton::RsUp => "Right stick up",
            PadButton::RsDown => "Right stick down",
            PadButton::RsLeft => "Right stick left",
            PadButton::RsRight => "Right stick right",
            PadButton::Guide => "Guide / Home",
            PadButton::Share => "Share / Capture",
            PadButton::Touchpad => "Touchpad click",
            PadButton::Mic => "Mic button",
            PadButton::P1 => "Extra 1",
            PadButton::P2 => "Extra 2",
            PadButton::P3 => "Extra 3",
            PadButton::P4 => "Extra 4",
            PadButton::LeftPad => "Left trackpad click",
            PadButton::Button(n) => return format!("Button {n}"),
        };
        s.to_string()
    }

    pub fn detail(self) -> &'static str {
        match self {
            PadButton::Lt | PadButton::Rt => "Controller trigger (pressed past a third)",
            PadButton::LsUp
            | PadButton::LsDown
            | PadButton::LsLeft
            | PadButton::LsRight
            | PadButton::RsUp
            | PadButton::RsDown
            | PadButton::RsLeft
            | PadButton::RsRight => "Controller stick, pushed past half way",
            PadButton::Guide => "Xbox, PS, Home or Steam button",
            PadButton::Share => "Xbox Share, Switch Capture, Steam quick access (…)",
            PadButton::Touchpad => "PlayStation touchpad click, Steam right trackpad click",
            PadButton::Mic => "DualSense microphone button",
            PadButton::P1 => "Left back paddle · Joy-Con (L) SL · Steam L4",
            PadButton::P2 => "Right back paddle · Joy-Con (R) SR · Steam R4",
            PadButton::P3 => "DualSense Edge left Fn · Joy-Con (L) SR · Steam L5",
            PadButton::P4 => "DualSense Edge right Fn · Joy-Con (R) SL · Steam R5",
            PadButton::LeftPad => "Steam left trackpad click",
            PadButton::Button(_) => "Button of a controller without a known layout",
            _ => "Controller button",
        }
    }
}

/// Which button names the controller drawing shows.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PadLabels {
    #[default]
    Xbox,
    PlayStation,
    Nintendo,
}

/// Which controller stick moves the mouse pointer.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StickPointer {
    #[default]
    Off,
    Left,
    Right,
}

/// A mouse or controller the user named, for bindings that only apply to
/// it (`mouse:back@Trackball`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// How the backend recognises it: a Windows HID device path, `xinput:1`
    /// for a controller slot, or `evdev:vendor:product:name` on Linux.
    pub id: String,
}

impl Device {
    /// Short description of what kind of device the id is.
    pub fn kind(&self) -> &'static str {
        if self.id.starts_with("xinput:") {
            "Controller"
        } else if self.id.starts_with("evdev:") {
            "Input device"
        } else if self.id == "injected" {
            "Synthetic input (testing)"
        } else {
            "Mouse"
        }
    }

    pub fn is_controller(&self) -> bool {
        self.id.starts_with("xinput:")
    }
}

/// Split a binding id into the input part and the device it is limited to:
/// `mouse:back@Trackball` → (`mouse:back`, Some(`Trackball`)). For a combo
/// the device belongs to the last input (the trigger).
pub fn split_device(id: &str) -> (&str, Option<&str>) {
    match id.split_once('@') {
        Some((input, device)) if !device.is_empty() => (input, Some(device)),
        _ => (id, None),
    }
}

/// Characters a device name can't contain (they'd break binding ids).
pub fn clean_device_name(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '@' | '+'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Modifier keys that can be combined with a key ("Ctrl + C").
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Win,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [Modifier::Ctrl, Modifier::Shift, Modifier::Alt, Modifier::Win];

    pub fn bit(self) -> u8 {
        match self {
            Modifier::Ctrl => 1,
            Modifier::Shift => 2,
            Modifier::Alt => 4,
            Modifier::Win => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Modifier::Ctrl => "Ctrl",
            Modifier::Shift => "Shift",
            Modifier::Alt => "Alt",
            Modifier::Win => "Win",
        }
    }

    /// Virtual-key code of the left-hand key, used when sending combos.
    pub fn vk(self) -> u16 {
        match self {
            Modifier::Ctrl => 0xA2,
            Modifier::Shift => 0xA0,
            Modifier::Alt => 0xA4,
            Modifier::Win => 0x5B,
        }
    }

    pub fn from_bits(bits: u8) -> Vec<Modifier> {
        Self::ALL.into_iter().filter(|m| bits & m.bit() != 0).collect()
    }

    pub fn to_bits(mods: &[Modifier]) -> u8 {
        mods.iter().fold(0, |acc, m| acc | m.bit())
    }
}

/// "Ctrl + Shift + C" style label.
pub fn combo_label(key: &str, mods: &[Modifier]) -> String {
    let mut sorted = mods.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut parts: Vec<String> = sorted.iter().map(|m| m.name().to_string()).collect();
    parts.push(crate::keys::display_name(key));
    parts.join(" + ")
}

/// A key (optionally with modifiers) or a mouse button: what turbo presses.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mods: Vec<Modifier>,
    },
    Mouse {
        button: MouseButton,
    },
}

impl Output {
    pub fn label(&self) -> String {
        match self {
            Output::Key { key, mods } => combo_label(key, mods),
            Output::Mouse { button } => button.name().to_string(),
        }
    }
}

/// How a macro binding repeats.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MacroMode {
    /// Play once per press.
    #[default]
    Once,
    /// Play `count` times per press.
    Repeat,
    /// Loop while the input is held; stops on release.
    WhileHeld,
    /// First press starts looping, second press stops.
    Toggle,
}

impl MacroMode {
    pub const ALL: [MacroMode; 4] = [
        MacroMode::Once,
        MacroMode::Repeat,
        MacroMode::WhileHeld,
        MacroMode::Toggle,
    ];

    pub fn index(self) -> i32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0) as i32
    }

    pub fn name(self) -> &'static str {
        match self {
            MacroMode::Once => "once",
            MacroMode::Repeat => "repeat",
            MacroMode::WhileHeld => "while held",
            MacroMode::Toggle => "toggle",
        }
    }
}

fn one() -> u32 {
    1
}
fn d20() -> u32 {
    20
}
fn d50() -> u32 {
    50
}
fn d5() -> u32 {
    5
}
fn d200() -> u32 {
    200
}
fn d40() -> u32 {
    40
}
fn d250() -> u32 {
    250
}
fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// Things outside the input stream a button can do (`Action::System`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SystemCommand {
    LockPc,
    MuteMic,
    NextAudioOutput,
    MinimizeWindow,
    MaximizeWindow,
    AlwaysOnTop,
    NextMonitor,
    CloseWindow,
    NextProfile,
    AutoProfile,
}

impl SystemCommand {
    pub const ALL: [SystemCommand; 10] = [
        SystemCommand::LockPc,
        SystemCommand::MuteMic,
        SystemCommand::NextAudioOutput,
        SystemCommand::MinimizeWindow,
        SystemCommand::MaximizeWindow,
        SystemCommand::AlwaysOnTop,
        SystemCommand::NextMonitor,
        SystemCommand::CloseWindow,
        SystemCommand::NextProfile,
        SystemCommand::AutoProfile,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SystemCommand::LockPc => "Lock PC",
            SystemCommand::MuteMic => "Mute microphone",
            SystemCommand::NextAudioOutput => "Next audio output",
            SystemCommand::MinimizeWindow => "Minimize window",
            SystemCommand::MaximizeWindow => "Maximize / restore",
            SystemCommand::AlwaysOnTop => "Always on top",
            SystemCommand::NextMonitor => "Move to next monitor",
            SystemCommand::CloseWindow => "Close window",
            SystemCommand::NextProfile => "Next profile",
            SystemCommand::AutoProfile => "Automatic profile",
        }
    }
}

/// What an input does when it is remapped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Send a keyboard key (by key id, see `keys.rs`), optionally with
    /// modifiers held around it. Held for as long as the input is held.
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mods: Vec<Modifier>,
    },
    /// Send a mouse button / wheel step.
    Mouse { button: MouseButton },
    /// Swallow the input entirely.
    Disabled,
    /// Play a macro from the macro library.
    Macro {
        #[serde(rename = "macro")]
        name: String,
        #[serde(default)]
        mode: MacroMode,
        /// Number of plays for `repeat` mode.
        #[serde(default = "one")]
        count: u32,
        /// Pause between repetitions.
        #[serde(default)]
        gap_ms: u32,
    },
    /// Rapid-fire a key or button while held (or toggled on). With a
    /// `delay_ms` it behaves like keyboard auto-repeat: one press, a pause,
    /// then repeats.
    Turbo {
        output: Output,
        /// Time from one press to the next.
        #[serde(default = "d50")]
        interval_ms: u32,
        /// How long each press is held down.
        #[serde(default = "d20")]
        hold_ms: u32,
        /// Wait after the first press before repeating starts.
        #[serde(default)]
        delay_ms: u32,
        /// Press once to start, again to stop (instead of while held).
        #[serde(default)]
        toggle: bool,
    },
    /// Type a piece of text.
    Text {
        text: String,
        /// Pause between characters.
        #[serde(default = "d5")]
        delay_ms: u32,
    },
    /// Start a program, open a file or a URL.
    Launch {
        path: String,
        #[serde(default)]
        args: String,
    },
    /// Hold to switch to the profile's shift layer (like G-Shift / Hypershift).
    /// Older configs only: loaded as `Layer { layer: 1, toggle: false }`.
    Shift,
    /// Hold to lower (or raise) the pointer speed, a "sniper" button.
    /// `speed` is the Windows pointer speed, 1-20 (10 is the default).
    Sensitivity { speed: u32 },
    /// Use binding layer `layer` (1-3) while held, or toggle it on and off.
    Layer {
        layer: u8,
        #[serde(default, skip_serializing_if = "is_false")]
        toggle: bool,
    },
    /// One action on a tap, another when held (and optionally a third on a
    /// double tap). Pressing another input while this one is held also
    /// counts as holding it, so it works as a modifier.
    TapHold {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tap: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        double: Option<Box<Action>>,
        /// Held at least this long counts as a hold.
        #[serde(default = "d200")]
        hold_ms: u32,
        /// A second press within this time after a tap is a double tap.
        #[serde(default = "d250")]
        double_ms: u32,
    },
    /// Lock the PC, mute the microphone, move the focused window…
    System { command: SystemCommand },
    /// Mouse gesture: hold the input, move the pointer in a direction and
    /// let go. Letting go without moving does `tap`.
    Gesture {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tap: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        up: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        down: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        left: Option<Box<Action>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        right: Option<Box<Action>>,
        /// Pixels the pointer has to move to count as a swipe.
        #[serde(default = "d40")]
        distance: u32,
    },
    /// Controller gyro moves the pointer while held, or toggle it.
    Gyro {
        #[serde(default, skip_serializing_if = "is_false")]
        toggle: bool,
    },
}

impl Action {
    /// The nested action in slot `slot` of a tap / hold (1 tap, 2 hold,
    /// 3 double) or a gesture (1 click, 2 up, 3 down, 4 left, 5 right).
    pub fn slot(&self, slot: usize) -> Option<&Action> {
        self.slots().get(slot.wrapping_sub(1)).and_then(|s| s.as_deref())
    }

    pub fn slot_mut(&mut self, slot: usize) -> Option<&mut Option<Box<Action>>> {
        let mut slots = self.slots_mut();
        if slot == 0 || slot > slots.len() {
            return None;
        }
        Some(slots.swap_remove(slot - 1))
    }

    /// Nested action slots, in slot order (empty for plain actions).
    fn slots(&self) -> Vec<&Option<Box<Action>>> {
        match self {
            Action::TapHold { tap, hold, double, .. } => vec![tap, hold, double],
            Action::Gesture {
                tap,
                up,
                down,
                left,
                right,
                ..
            } => vec![tap, up, down, left, right],
            _ => vec![],
        }
    }

    fn slots_mut(&mut self) -> Vec<&mut Option<Box<Action>>> {
        match self {
            Action::TapHold { tap, hold, double, .. } => vec![tap, hold, double],
            Action::Gesture {
                tap,
                up,
                down,
                left,
                right,
                ..
            } => vec![tap, up, down, left, right],
            _ => vec![],
        }
    }

    /// Whether this action holds other actions (tap / hold, gesture).
    pub fn has_slots(&self) -> bool {
        matches!(self, Action::TapHold { .. } | Action::Gesture { .. })
    }

    /// Call `f` on this action and every action nested in it.
    pub fn walk(&self, f: &mut impl FnMut(&Action)) {
        f(self);
        for a in self.slots().into_iter().flatten() {
            a.walk(f);
        }
    }

    /// Like `walk`, mutably.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Action)) {
        f(self);
        for a in self.slots_mut().into_iter().flatten() {
            a.walk_mut(f);
        }
    }

    pub fn label(&self) -> String {
        match self {
            Action::Key { key, mods } => combo_label(key, mods),
            Action::Mouse { button } => button.name().to_string(),
            Action::Disabled => "Disabled".to_string(),
            Action::Macro { name, mode, count, .. } => match mode {
                MacroMode::Repeat => format!("Macro: {name} ×{count}"),
                MacroMode::Once => format!("Macro: {name}"),
                m => format!("Macro: {name} ({})", m.name()),
            },
            Action::Turbo {
                output, interval_ms, ..
            } => {
                format!("Turbo {} · {}/s", output.label(), per_second(*interval_ms))
            }
            Action::Text { text, .. } => {
                let first: String = text.chars().take(18).collect();
                if text.chars().count() > 18 {
                    format!("Type \"{first}…\"")
                } else {
                    format!("Type \"{first}\"")
                }
            }
            Action::Launch { path, .. } => {
                let name = path.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or(path);
                format!("Launch {name}")
            }
            Action::Shift => "Shift layer (hold)".to_string(),
            Action::Sensitivity { speed } => format!("Sensitivity {speed}/20 (hold)"),
            Action::Layer { layer, toggle } => {
                let name = match layer {
                    1 => "Shift layer".to_string(),
                    n => format!("Layer {n}"),
                };
                format!("{name} ({})", if *toggle { "toggle" } else { "hold" })
            }
            Action::TapHold { tap, hold, double, .. } => {
                let part = |a: &Option<Box<Action>>| a.as_ref().map(|a| a.label()).unwrap_or_else(|| "nothing".into());
                let mut s = format!("Tap {} · hold {}", part(tap), part(hold));
                if double.is_some() {
                    s.push_str(&format!(" · double {}", part(double)));
                }
                s
            }
            Action::System { command } => command.name().to_string(),
            Action::Gyro { toggle } => format!("Gyro mouse ({})", if *toggle { "toggle" } else { "hold" }),
            Action::Gesture {
                tap,
                up,
                down,
                left,
                right,
                ..
            } => {
                let mut parts = Vec::new();
                for (arrow, a) in [("↑", up), ("↓", down), ("←", left), ("→", right)] {
                    if let Some(a) = a {
                        parts.push(format!("{arrow} {}", a.label()));
                    }
                }
                if let Some(t) = tap {
                    parts.push(format!("click {}", t.label()));
                }
                if parts.is_empty() {
                    "Gesture (nothing set)".to_string()
                } else {
                    format!("Gesture: {}", parts.join(" · "))
                }
            }
        }
    }

    /// Very short label used on keycaps.
    pub fn short_label(&self) -> String {
        match self {
            Action::Key { key, mods } => {
                let base = match crate::keys::find(key) {
                    Some(k) if !k.label.is_empty() => k.label.to_string(),
                    _ => key.clone(),
                };
                if mods.is_empty() {
                    base
                } else {
                    let prefix: String = Modifier::ALL
                        .iter()
                        .filter(|m| mods.contains(m))
                        .map(|m| match m {
                            Modifier::Ctrl => "^",
                            Modifier::Shift => "⇧",
                            Modifier::Alt => "⌥",
                            Modifier::Win => "⊞",
                        })
                        .collect();
                    format!("{prefix}{base}")
                }
            }
            Action::Mouse { button } => button.short().to_string(),
            Action::Disabled => "Off".to_string(),
            Action::Macro { .. } => "Mac".to_string(),
            Action::Turbo { .. } => "Tur".to_string(),
            Action::Text { .. } => "Txt".to_string(),
            Action::Launch { .. } => "Run".to_string(),
            Action::Shift => "Shft".to_string(),
            Action::Sensitivity { .. } => "Sns".to_string(),
            Action::Layer { layer: 1, .. } => "Shft".to_string(),
            Action::Layer { layer, .. } => format!("L{layer}"),
            Action::TapHold { .. } => "T/H".to_string(),
            Action::System { .. } => "Sys".to_string(),
            Action::Gesture { .. } => "Gst".to_string(),
            Action::Gyro { .. } => "Gyr".to_string(),
        }
    }
}

pub fn per_second(interval_ms: u32) -> String {
    let rate = 1000.0 / interval_ms.max(1) as f64;
    if rate >= 10.0 || (rate - rate.round()).abs() < 0.05 {
        format!("{}", rate.round() as u32)
    } else {
        format!("{rate:.1}")
    }
}

/// One step of a macro.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroStep {
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    /// Press and release, holding for `hold_ms`.
    KeyTap {
        key: String,
        #[serde(default = "d20")]
        hold_ms: u32,
    },
    MouseDown {
        button: MouseButton,
    },
    MouseUp {
        button: MouseButton,
    },
    /// Click (or one wheel step), holding for `hold_ms`.
    MouseClick {
        button: MouseButton,
        #[serde(default = "d20")]
        hold_ms: u32,
    },
    Delay {
        ms: u32,
    },
    Text {
        text: String,
    },
    /// Move the pointer by (x, y) counts like a mouse would, or to the
    /// screen position (x, y) in pixels when `absolute`.
    MouseMove {
        x: i32,
        y: i32,
        #[serde(default, skip_serializing_if = "is_false")]
        absolute: bool,
    },
}

impl MacroStep {
    /// (short kind label, value) for the macro editor.
    pub fn describe(&self) -> (&'static str, String) {
        use crate::keys::display_name;
        match self {
            MacroStep::KeyDown { key } => ("Key down", display_name(key)),
            MacroStep::KeyUp { key } => ("Key up", display_name(key)),
            MacroStep::KeyTap { key, hold_ms } => ("Key press", format!("{} · {hold_ms} ms", display_name(key))),
            MacroStep::MouseDown { button } => ("Mouse down", button.name().to_string()),
            MacroStep::MouseUp { button } => ("Mouse up", button.name().to_string()),
            MacroStep::MouseClick { button, hold_ms } => {
                if button.is_wheel() {
                    ("Mouse", button.name().to_string())
                } else {
                    ("Mouse click", format!("{} · {hold_ms} ms", button.name()))
                }
            }
            MacroStep::Delay { ms } => ("Delay", format!("{ms} ms")),
            MacroStep::Text { text } => ("Type text", format!("\"{text}\"")),
            MacroStep::MouseMove { x, y, absolute: false } => ("Move by", format!("{x:+}, {y:+}")),
            MacroStep::MouseMove { x, y, absolute: true } => ("Move to", format!("{x}, {y}")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Macro {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<MacroStep>,
    /// Make every delay up to this much shorter or longer, at random.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub jitter_ms: u32,
}

impl Macro {
    pub fn new(name: impl Into<String>, steps: Vec<MacroStep>) -> Self {
        Macro {
            name: name.into(),
            steps,
            jitter_ms: 0,
        }
    }
}

pub type BindingMap = BTreeMap<String, Action>;

static NO_BINDINGS: BindingMap = BTreeMap::new();

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    /// What activates this profile (see `AppRule`): executable names such as
    /// `GenshinImpact.exe`, full paths, or `title:` window titles. Empty for
    /// the Default profile.
    #[serde(default)]
    pub apps: Vec<String>,
    /// Only activate while the matching window covers its whole screen.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fullscreen_only: bool,
    #[serde(default)]
    pub bindings: BindingMap,
    /// Layer 1, used while an input bound to it is held (the shift layer).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shift_bindings: BindingMap,
    /// Layers 2 and 3.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<BindingMap>,
    /// Controller stick that moves the pointer. `None`: same as the Default
    /// profile (off for Default itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stick_pointer: Option<StickPointer>,
    /// Tilting the controller moves the pointer (gyro). `None`: same as Default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gyro_pointer: Option<bool>,
    /// The controller's touchpad moves the pointer. `None`: same as Default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touch_pointer: Option<bool>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Self {
        Profile {
            name: name.into(),
            apps: vec![],
            fullscreen_only: false,
            bindings: BTreeMap::new(),
            shift_bindings: BTreeMap::new(),
            layers: vec![],
            stick_pointer: None,
            gyro_pointer: None,
            touch_pointer: None,
        }
    }

    /// Bindings of a layer: 0 base, 1 shift, 2-3.
    pub fn layer(&self, layer: usize) -> &BindingMap {
        match layer {
            0 => &self.bindings,
            1 => &self.shift_bindings,
            n => self.layers.get(n - 2).unwrap_or(&NO_BINDINGS),
        }
    }

    pub fn layer_mut(&mut self, layer: usize) -> &mut BindingMap {
        match layer {
            0 => &mut self.bindings,
            1 => &mut self.shift_bindings,
            n => {
                let i = (n - 2).min(LAYER_COUNT - 3);
                while self.layers.len() <= i {
                    self.layers.push(BTreeMap::new());
                }
                &mut self.layers[i]
            }
        }
    }

    /// Drop empty trailing layers so they don't clutter the file.
    pub fn tidy(&mut self) {
        while self.layers.last().is_some_and(|l| l.is_empty()) {
            self.layers.pop();
        }
    }

    fn maps(&self) -> impl Iterator<Item = &BindingMap> {
        [&self.bindings, &self.shift_bindings]
            .into_iter()
            .chain(self.layers.iter())
    }

    fn maps_mut(&mut self) -> impl Iterator<Item = &mut BindingMap> {
        [&mut self.bindings, &mut self.shift_bindings]
            .into_iter()
            .chain(self.layers.iter_mut())
    }

    pub fn binding_count(&self) -> usize {
        self.maps().map(|m| m.len()).sum()
    }

    /// Every top-level binding action, all layers.
    pub fn all_actions(&self) -> impl Iterator<Item = &Action> {
        self.maps().flat_map(|m| m.values())
    }

    pub fn all_actions_mut(&mut self) -> impl Iterator<Item = &mut Action> {
        self.maps_mut().flat_map(|m| m.values_mut())
    }

    /// Whether any action, nested ones included, plays macro `name`.
    pub fn uses_macro(&self, name: &str) -> bool {
        let mut found = false;
        for a in self.all_actions() {
            a.walk(&mut |a| found |= matches!(a, Action::Macro { name: n, .. } if n == name));
        }
        found
    }
}

// ---------------------------------------------------------------------------
// Combos: an input id that needs other keys or buttons held
// ---------------------------------------------------------------------------

/// A binding id like `ctrl+mouse:back` or `mouse:right+mouse:left`: hold the
/// modifiers and inputs before the last part, then press the last one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    pub mods: Vec<Modifier>,
    /// Other inputs that must be held (`key:Q`, `mouse:right`).
    pub held: Vec<String>,
    pub trigger: String,
}

impl Combo {
    pub fn parse(id: &str) -> Option<Combo> {
        if !id.contains('+') {
            return None;
        }
        let mut parts: Vec<&str> = id.split('+').collect();
        let trigger = parts.pop()?.to_string();
        let mut mods = Vec::new();
        let mut held = Vec::new();
        for p in parts {
            match Modifier::ALL.iter().find(|m| m.name().eq_ignore_ascii_case(p)) {
                Some(m) => mods.push(*m),
                None if p.contains(':') => held.push(p.to_string()),
                None => return None,
            }
        }
        if (mods.is_empty() && held.is_empty()) || !trigger.contains(':') {
            return None;
        }
        Some(Combo { mods, held, trigger })
    }

    /// Canonical id; a combo with nothing held is just the trigger's id.
    pub fn id(&self) -> String {
        let mut mods = self.mods.clone();
        mods.sort();
        mods.dedup();
        let mut held = self.held.clone();
        held.sort();
        held.dedup();
        held.retain(|h| *h != self.trigger);
        let mut parts: Vec<String> = mods.iter().map(|m| m.name().to_ascii_lowercase()).collect();
        parts.extend(held);
        parts.push(self.trigger.clone());
        parts.join("+")
    }
}

// ---------------------------------------------------------------------------
// App rules: which windows activate a profile
// ---------------------------------------------------------------------------

/// One entry of `Profile::apps`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppRule {
    /// `game.exe`, `*launcher*.exe`: the executable's file name.
    Exe(String),
    /// `C:\Games\*\game.exe`: the executable's full path.
    Path(String),
    /// `title:YouTube`: the window title contains this (or matches it, with
    /// wildcards).
    Title(String),
}

impl AppRule {
    pub fn parse(text: &str) -> AppRule {
        let t = text.trim();
        if let Some(title) = t.strip_prefix("title:") {
            AppRule::Title(title.trim().to_lowercase())
        } else if t.contains('\\') || t.contains('/') {
            AppRule::Path(t.to_lowercase())
        } else {
            AppRule::Exe(t.to_lowercase())
        }
    }

    /// How specific a match is (higher wins), or None. `exe`, `path` and
    /// `title` must be lower-cased.
    pub fn matches(&self, exe: &str, path: &str, title: &str) -> Option<u8> {
        let has_wild = |p: &str| p.contains('*') || p.contains('?');
        match self {
            AppRule::Exe(p) if !p.is_empty() => (if has_wild(p) { glob_match(p, exe) } else { p == exe }).then_some(1),
            AppRule::Path(p) => (if has_wild(p) { glob_match(p, path) } else { p == path }).then_some(2),
            AppRule::Title(p) if !p.is_empty() => (if has_wild(p) {
                glob_match(p, title)
            } else {
                title.contains(p.as_str())
            })
            .then_some(3),
            _ => None,
        }
    }
}

/// Wildcard match: `*` any run of characters, `?` any one character.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

// ---------------------------------------------------------------------------
// App hotkeys
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hotkey {
    pub key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mods: Vec<Modifier>,
}

/// Keys that control the app itself. They work everywhere, even while
/// remapping is paused.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Hotkeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause: Option<Hotkey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_profile: Option<Hotkey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_profile: Option<Hotkey>,
}

impl Hotkeys {
    pub const NAMES: [&'static str; 3] = ["Pause / resume remapping", "Next profile", "Back to automatic profile"];

    pub fn get(&self, i: usize) -> Option<&Hotkey> {
        match i {
            0 => self.pause.as_ref(),
            1 => self.next_profile.as_ref(),
            _ => self.auto_profile.as_ref(),
        }
    }

    pub fn slot_mut(&mut self, i: usize) -> &mut Option<Hotkey> {
        match i {
            0 => &mut self.pause,
            1 => &mut self.next_profile,
            _ => &mut self.auto_profile,
        }
    }
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
    /// Closing the window keeps the app running in the tray.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Show the number pad on the keyboard diagram.
    #[serde(default)]
    pub full_keyboard: bool,
    /// `profiles[0]` is always the Default profile: it applies when no other
    /// profile matches, and unbound inputs in other profiles fall back to it.
    pub profiles: Vec<Profile>,
    /// Macro library, referenced by name from `Action::Macro`.
    #[serde(default)]
    pub macros: Vec<Macro>,
    #[serde(default)]
    pub hotkeys: Hotkeys,
    /// Show on-screen notifications (profile switches, toggles). Off unless
    /// turned on in Settings.
    #[serde(default)]
    pub notifications: bool,
    /// Look for a new version at start and once a day. Off unless turned on.
    #[serde(default)]
    pub check_updates: bool,
    /// Mice and controllers with names, for bindings limited to one device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
    /// Pointer speed when a controller stick moves it (1-20).
    #[serde(default = "d10")]
    pub stick_speed: u32,
    /// Controller settings: gyro and touchpad speeds, button names, which
    /// controllers are read.
    #[serde(default)]
    pub pad: PadSettings,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PadSettings {
    /// Pointer speed when the gyro moves it (1-20).
    pub gyro_speed: u32,
    pub gyro_invert_x: bool,
    pub gyro_invert_y: bool,
    /// Pointer speed when the touchpad moves it (1-20).
    pub touch_speed: u32,
    pub labels: PadLabels,
    /// Read Xbox-style controllers (XInput).
    pub xinput: bool,
    /// Read PlayStation, Switch, Steam and other controllers directly (HID).
    pub direct: bool,
    /// Skip Steam's virtual Xbox controllers (Steam Input), which copy a real
    /// controller the app may already be reading.
    pub skip_steam_virtual: bool,
    /// Turn off the Steam Deck's / Steam Controller's own mouse and keyboard
    /// mode while the app reads it.
    pub steam_lizard_off: bool,
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings {
            gyro_speed: 10,
            gyro_invert_x: false,
            gyro_invert_y: false,
            touch_speed: 10,
            labels: PadLabels::Xbox,
            xinput: true,
            direct: true,
            skip_steam_virtual: true,
            steam_lizard_off: false,
        }
    }
}

fn d10() -> u32 {
    10
}

fn default_version() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let mut genshin = Profile::new("Genshin Impact");
        genshin.apps.push("GenshinImpact.exe".into());
        genshin.bindings.insert(
            "mouse:back".to_string(),
            Action::Key {
                key: "F".into(),
                mods: vec![],
            },
        );
        genshin.bindings.insert(
            "mouse:forward".to_string(),
            Action::Key {
                key: "E".into(),
                mods: vec![],
            },
        );
        Config {
            version: CONFIG_VERSION,
            theme: ThemeMode::Dark,
            enabled: true,
            close_to_tray: true,
            full_keyboard: false,
            profiles: vec![Profile::new("Default"), genshin],
            macros: vec![],
            hotkeys: Hotkeys::default(),
            notifications: false,
            check_updates: false,
            devices: vec![],
            stick_speed: 10,
            pad: PadSettings::default(),
        }
    }
}

/// Older files hold the shift layer as `{ "type": "shift" }`.
fn migrate_action(a: &mut Action) {
    a.walk_mut(&mut |a| {
        if matches!(a, Action::Shift) {
            *a = Action::Layer {
                layer: 1,
                toggle: false,
            };
        }
    });
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os(CONFIG_ENV).filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        directories::ProjectDirs::from("", "", "mouse-kb-remap").map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Config {
        let Some(path) = Self::path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match Self::parse(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    // Keep the broken file so a typo in a hand edit never loses settings.
                    let backup = path.with_file_name("config.broken.json");
                    let _ = std::fs::copy(&path, &backup);
                    crate::log!(
                        "config: failed to parse {}: {err}; using defaults (saved a copy to {})",
                        path.display(),
                        backup.display()
                    );
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn parse(text: &str) -> Result<Config, serde_json::Error> {
        let mut cfg = serde_json::from_str::<Config>(text)?;
        if cfg.profiles.is_empty() {
            cfg.profiles = Config::default().profiles;
        }
        for p in &mut cfg.profiles {
            p.all_actions_mut().for_each(migrate_action);
            p.layers.truncate(LAYER_COUNT - 2);
        }
        cfg.version = CONFIG_VERSION;
        Ok(cfg)
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
                let result = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path));
                if let Err(e) = result {
                    crate::log!("config: failed to save {}: {e}", path.display());
                }
            }
            Err(err) => crate::log!("config: failed to serialize: {err}"),
        }
    }

    pub fn find_macro(&self, name: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.name == name)
    }

    /// A name based on `base` that no macro uses yet.
    pub fn unique_macro_name(&self, base: &str) -> String {
        unique_name(base, |n| self.macros.iter().any(|m| m.name == n))
    }

    /// A name based on `base` that no profile uses yet.
    pub fn unique_profile_name(&self, base: &str) -> String {
        unique_name(base, |n| self.profiles.iter().any(|p| p.name == n))
    }

    /// Point every binding that plays macro `old` at `new` (or remove them
    /// when `new` is `None`). Returns how many bindings changed.
    pub fn retarget_macro(&mut self, old: &str, new: Option<&str>) -> usize {
        let mut changed = 0;
        let plays_old = |a: &Action| matches!(a, Action::Macro { name, .. } if name == old);
        for p in &mut self.profiles {
            for layer in p.maps_mut() {
                if new.is_none() {
                    // Bindings that only play it go away entirely.
                    let before = layer.len();
                    layer.retain(|_, a| !plays_old(a));
                    changed += before - layer.len();
                }
                for action in layer.values_mut() {
                    action.walk_mut(&mut |a| {
                        if let Some(new) = new {
                            if let Action::Macro { name, .. } = a {
                                if name == old {
                                    *name = new.to_string();
                                    changed += 1;
                                }
                            }
                        } else if let Action::TapHold { tap, hold, double, .. } = a {
                            for slot in [tap, hold, double] {
                                if slot.as_deref().is_some_and(plays_old) {
                                    *slot = None;
                                    changed += 1;
                                }
                            }
                        }
                    });
                }
            }
        }
        changed
    }

    /// Names of profiles with a binding that plays macro `name`.
    pub fn macro_users(&self, name: &str) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|p| p.uses_macro(name))
            .map(|p| p.name.clone())
            .collect()
    }

    /// Rename a device, or forget it (`new` = None) together with the
    /// bindings limited to it. Returns how many bindings were changed.
    pub fn retarget_device(&mut self, old: &str, new: Option<&str>) -> usize {
        let mut changed = 0;
        for p in &mut self.profiles {
            for map in p.maps_mut() {
                let ids: Vec<String> = map
                    .keys()
                    .filter(|id| split_device(id).1 == Some(old))
                    .cloned()
                    .collect();
                for id in ids {
                    let Some(action) = map.remove(&id) else { continue };
                    changed += 1;
                    if let Some(new) = new {
                        map.insert(format!("{}@{new}", split_device(&id).0), action);
                    }
                }
            }
        }
        if let Some(d) = self.devices.iter_mut().find(|d| d.name == old) {
            match new {
                Some(n) => d.name = n.to_string(),
                None => d.name.clear(),
            }
        }
        self.devices.retain(|d| !d.name.is_empty());
        changed
    }

    /// A free device name based on `base`.
    pub fn unique_device_name(&self, base: &str) -> String {
        let base = clean_device_name(base);
        let base = if base.is_empty() { "Device".to_string() } else { base };
        unique_name(&base, |n| self.devices.iter().any(|d| d.name.eq_ignore_ascii_case(n)))
    }

    /// The stick that moves the pointer in profile `index`.
    /// Whether the gyro moves the pointer in profile `index`.
    pub fn gyro_pointer(&self, index: usize) -> bool {
        self.profiles
            .get(index)
            .and_then(|p| p.gyro_pointer)
            .or_else(|| self.profiles.first().and_then(|p| p.gyro_pointer))
            .unwrap_or(false)
    }

    /// Whether the touchpad moves the pointer in profile `index`.
    pub fn touch_pointer(&self, index: usize) -> bool {
        self.profiles
            .get(index)
            .and_then(|p| p.touch_pointer)
            .or_else(|| self.profiles.first().and_then(|p| p.touch_pointer))
            .unwrap_or(false)
    }

    pub fn stick_pointer(&self, index: usize) -> StickPointer {
        self.profiles
            .get(index)
            .and_then(|p| p.stick_pointer)
            .or_else(|| self.profiles.first().and_then(|p| p.stick_pointer))
            .unwrap_or_default()
    }
}

pub fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| !taken(n))
        .expect("infinite iterator")
}

// ---------------------------------------------------------------------------
// Import / export
// ---------------------------------------------------------------------------

/// File format for exported profiles: the profiles plus every macro they use.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProfileExport {
    #[serde(rename = "mouse-kb-remap")]
    pub kind: String,
    pub version: u32,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub macros: Vec<Macro>,
}

impl Config {
    /// Bundle the given profiles (by index) with the macros they use.
    pub fn export(&self, indices: &[usize]) -> ProfileExport {
        let profiles: Vec<Profile> = indices.iter().filter_map(|&i| self.profiles.get(i).cloned()).collect();
        let mut macros: Vec<Macro> = Vec::new();
        for p in &profiles {
            for a in p.all_actions() {
                a.walk(&mut |a| {
                    if let Action::Macro { name, .. } = a {
                        if !macros.iter().any(|m| &m.name == name) {
                            if let Some(m) = self.find_macro(name) {
                                macros.push(m.clone());
                            }
                        }
                    }
                });
            }
        }
        ProfileExport {
            kind: "profiles".into(),
            version: CONFIG_VERSION,
            profiles,
            macros,
        }
    }

    /// Add profiles from an export file or a whole `config.json`. Imported
    /// profiles are appended (names made unique); a "Default" profile comes in
    /// as a normal profile. Macros with the same name and steps are reused,
    /// clashing ones are renamed. Returns the number of profiles added.
    pub fn import(&mut self, text: &str) -> Result<usize, String> {
        let (profiles, macros) = if let Ok(e) = serde_json::from_str::<ProfileExport>(text) {
            (e.profiles, e.macros)
        } else {
            let c = serde_json::from_str::<Config>(text).map_err(|e| format!("not a profile export or config: {e}"))?;
            (c.profiles, c.macros)
        };
        if profiles.is_empty() {
            return Err("the file contains no profiles".into());
        }

        // Macro name in the file -> name in our library.
        let mut renamed: Vec<(String, String)> = Vec::new();
        for m in macros {
            match self.find_macro(&m.name) {
                Some(existing) if existing.steps == m.steps => {}
                Some(_) => {
                    let new = self.unique_macro_name(&m.name);
                    renamed.push((m.name.clone(), new.clone()));
                    self.macros.push(Macro { name: new, ..m });
                }
                None => self.macros.push(m),
            }
        }

        let count = profiles.len();
        for mut p in profiles {
            for a in p.all_actions_mut() {
                migrate_action(a);
                a.walk_mut(&mut |a| {
                    if let Action::Macro { name, .. } = a {
                        if let Some((_, new)) = renamed.iter().find(|(old, _)| old == name) {
                            *name = new.clone();
                        }
                    }
                });
            }
            p.layers.truncate(LAYER_COUNT - 2);
            p.name = self.unique_profile_name(p.name.trim());
            self.profiles.push(p);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_bindings_follow_renames_and_removal() {
        assert_eq!(split_device("mouse:back@Left one"), ("mouse:back", Some("Left one")));
        assert_eq!(split_device("ctrl+mouse:back@Pad"), ("ctrl+mouse:back", Some("Pad")));
        assert_eq!(split_device("mouse:back"), ("mouse:back", None));
        assert_eq!(clean_device_name(" a@b+c "), "abc");

        let mut cfg = Config::default();
        cfg.devices.push(Device {
            name: "Trackball".into(),
            id: "x".into(),
        });
        let key = |k: &str| Action::Key {
            key: k.into(),
            mods: vec![],
        };
        cfg.profiles[0].bindings.insert("mouse:back@Trackball".into(), key("A"));
        cfg.profiles[1]
            .layer_mut(2)
            .insert("ctrl+mouse:left@Trackball".into(), key("B"));
        cfg.profiles[1].bindings.insert("mouse:left".into(), key("C"));
        assert_eq!(cfg.retarget_device("Trackball", Some("Ball")), 2);
        assert_eq!(cfg.devices[0].name, "Ball");
        assert!(cfg.profiles[0].bindings.contains_key("mouse:back@Ball"));
        assert!(cfg.profiles[1].layer(2).contains_key("ctrl+mouse:left@Ball"));
        assert_eq!(cfg.unique_device_name("ball"), "ball (2)");
        assert_eq!(cfg.retarget_device("Ball", None), 2);
        assert!(cfg.devices.is_empty());
        assert_eq!(cfg.profiles[1].binding_count(), 3); // its own two, plus mouse:left
    }

    #[test]
    fn gestures_and_controller_settings_round_trip() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert(
            "mouse:right".into(),
            Action::Gesture {
                tap: Some(Box::new(Action::Mouse {
                    button: MouseButton::Right,
                })),
                up: None,
                down: Some(Box::new(Action::System {
                    command: SystemCommand::MinimizeWindow,
                })),
                left: None,
                right: None,
                distance: 60,
            },
        );
        cfg.profiles[1].stick_pointer = Some(StickPointer::Right);
        let text = serde_json::to_string(&cfg).unwrap();
        assert!(text.contains(r#""type":"gesture""#) && text.contains(r#""stick_pointer":"right""#));
        let back = Config::parse(&text).unwrap();
        assert_eq!(back.profiles, cfg.profiles);
        assert_eq!(back.stick_pointer(0), StickPointer::Off);
        assert_eq!(back.stick_pointer(1), StickPointer::Right);
        let g = &back.profiles[0].bindings["mouse:right"];
        assert!(g.has_slots());
        assert_eq!(g.slot(3).map(|a| a.label()), Some("Minimize window".to_string()));
        assert!(g.label().starts_with("Gesture: ↓ Minimize window"));
        // A gesture missing its distance gets the default.
        let a: Action = serde_json::from_str(r#"{"type":"gesture"}"#).unwrap();
        assert!(matches!(a, Action::Gesture { distance: 40, .. }));
        assert_eq!(PadButton::from_id("rs_left"), Some(PadButton::RsLeft));
    }

    #[test]
    fn v1_config_still_loads() {
        let text = r#"{
            "version": 1, "theme": "light", "enabled": true,
            "profiles": [
                { "name": "Default", "apps": [], "bindings": { "mouse:forward": { "type": "key", "key": "LCtrl" } } },
                { "name": "Game", "apps": ["game.exe"], "bindings": { "key:CapsLock": { "type": "disabled" } } }
            ]
        }"#;
        let cfg = Config::parse(text).unwrap();
        assert_eq!(cfg.version, CONFIG_VERSION);
        assert_eq!(cfg.theme, ThemeMode::Light);
        assert!(cfg.close_to_tray);
        assert_eq!(
            cfg.profiles[0].bindings["mouse:forward"],
            Action::Key {
                key: "LCtrl".into(),
                mods: vec![]
            }
        );
        assert!(cfg.macros.is_empty());
    }

    #[test]
    fn actions_round_trip() {
        let actions = vec![
            Action::Key {
                key: "C".into(),
                mods: vec![Modifier::Ctrl, Modifier::Shift],
            },
            Action::Macro {
                name: "Combo".into(),
                mode: MacroMode::WhileHeld,
                count: 1,
                gap_ms: 10,
            },
            Action::Turbo {
                output: Output::Mouse {
                    button: MouseButton::Left,
                },
                interval_ms: 40,
                hold_ms: 15,
                delay_ms: 300,
                toggle: true,
            },
            Action::Text {
                text: "gg wp".into(),
                delay_ms: 5,
            },
            Action::Launch {
                path: "notepad.exe".into(),
                args: String::new(),
            },
            Action::Shift,
            Action::Sensitivity { speed: 4 },
            Action::Layer { layer: 2, toggle: true },
            Action::TapHold {
                tap: Some(Box::new(Action::Key {
                    key: "Esc".into(),
                    mods: vec![],
                })),
                hold: Some(Box::new(Action::Key {
                    key: "LCtrl".into(),
                    mods: vec![],
                })),
                double: None,
                hold_ms: 180,
                double_ms: 250,
            },
            Action::System {
                command: SystemCommand::NextMonitor,
            },
        ];
        for a in actions {
            let json = serde_json::to_string(&a).unwrap();
            let back: Action = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back, "{json}");
        }
        // The documented JSON shape for macros uses the key "macro".
        let a: Action = serde_json::from_str(r#"{ "type": "macro", "macro": "Reload" }"#).unwrap();
        assert_eq!(
            a,
            Action::Macro {
                name: "Reload".into(),
                mode: MacroMode::Once,
                count: 1,
                gap_ms: 0
            }
        );
    }

    #[test]
    fn combo_labels_are_ordered() {
        assert_eq!(combo_label("C", &[Modifier::Shift, Modifier::Ctrl]), "Ctrl + Shift + C");
        assert_eq!(combo_label("F5", &[]), "F5");
    }

    #[test]
    fn retarget_and_remove_macro_references() {
        let mut cfg = Config::default();
        cfg.macros.push(Macro::new("A", vec![]));
        let bind = Action::Macro {
            name: "A".into(),
            mode: MacroMode::Once,
            count: 1,
            gap_ms: 0,
        };
        cfg.profiles[0].bindings.insert("key:F1".into(), bind.clone());
        cfg.profiles[1].shift_bindings.insert("key:F2".into(), bind.clone());
        // Nested in a tap / hold on layer 2.
        cfg.profiles[1].layer_mut(2).insert(
            "key:F3".into(),
            Action::TapHold {
                tap: Some(Box::new(bind)),
                hold: Some(Box::new(Action::Disabled)),
                double: None,
                hold_ms: 200,
                double_ms: 250,
            },
        );
        assert_eq!(cfg.macro_users("A"), vec!["Default", "Genshin Impact"]);
        assert_eq!(cfg.retarget_macro("A", Some("B")), 3);
        assert_eq!(cfg.macro_users("B").len(), 2);
        assert_eq!(cfg.retarget_macro("B", None), 3);
        assert!(cfg.macro_users("B").is_empty());
        // The tap / hold binding stays, without its tap.
        assert!(matches!(
            &cfg.profiles[1].layer(2)["key:F3"],
            Action::TapHold {
                tap: None,
                hold: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn old_shift_bindings_become_layer_one() {
        let text = r#"{ "profiles": [ { "name": "Default", "bindings": {
            "mouse:back": { "type": "shift" },
            "key:F1": { "type": "tap_hold", "hold": { "type": "shift" } }
        } } ] }"#;
        let cfg = Config::parse(text).unwrap();
        let b = &cfg.profiles[0].bindings;
        assert_eq!(
            b["mouse:back"],
            Action::Layer {
                layer: 1,
                toggle: false
            }
        );
        assert_eq!(
            b["key:F1"].slot(2),
            Some(&Action::Layer {
                layer: 1,
                toggle: false
            })
        );
        // Defaults for the tap / hold timing.
        assert!(matches!(
            b["key:F1"],
            Action::TapHold {
                hold_ms: 200,
                double_ms: 250,
                ..
            }
        ));
    }

    #[test]
    fn layers_two_and_three_round_trip() {
        let mut p = Profile::new("P");
        p.layer_mut(3).insert("key:A".into(), Action::Disabled);
        assert_eq!(p.layers.len(), 2);
        assert!(p.layer(2).is_empty());
        assert_eq!(p.binding_count(), 1);
        let json = serde_json::to_string(&p).unwrap();
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layer(3)["key:A"], Action::Disabled);
        p.layer_mut(3).clear();
        p.tidy();
        assert!(p.layers.is_empty());
    }

    #[test]
    fn combo_ids_parse_and_normalise() {
        let c = Combo::parse("shift+ctrl+mouse:back").unwrap();
        assert_eq!(c.mods, vec![Modifier::Shift, Modifier::Ctrl]);
        assert_eq!(c.trigger, "mouse:back");
        assert_eq!(c.id(), "ctrl+shift+mouse:back");
        let c = Combo::parse("mouse:right+mouse:left").unwrap();
        assert_eq!(c.held, vec!["mouse:right"]);
        assert!(Combo::parse("key:A").is_none());
        assert!(Combo::parse("bogus+key:A").is_none());
        assert!(Combo::parse("ctrl+shift").is_none());
        // Nothing held: the plain input id.
        let plain = Combo {
            mods: vec![],
            held: vec![],
            trigger: "key:A".into(),
        };
        assert_eq!(plain.id(), "key:A");
    }

    #[test]
    fn app_rules_match_names_paths_and_titles() {
        let exe = "game.exe";
        let path = r"c:\games\game\game.exe";
        let title = "game – main menu";
        assert_eq!(AppRule::parse("Game.EXE").matches(exe, path, title), Some(1));
        assert_eq!(AppRule::parse("g*.exe").matches(exe, path, title), Some(1));
        assert_eq!(AppRule::parse("other.exe").matches(exe, path, title), None);
        assert_eq!(
            AppRule::parse(r"C:\Games\*\game.exe").matches(exe, path, title),
            Some(2)
        );
        assert_eq!(AppRule::parse(r"D:\game.exe").matches(exe, path, title), None);
        assert_eq!(AppRule::parse("title:Main Menu").matches(exe, path, title), Some(3));
        assert_eq!(AppRule::parse("title:game ? main*").matches(exe, path, title), Some(3));
        assert_eq!(AppRule::parse("title:").matches(exe, path, title), None);
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*", ""));
        assert!(glob_match("a*c", "abbbc"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*.exe", "x.y.exe"));
        assert!(!glob_match("*.exe", "x.exe.bak"));
        assert!(glob_match("**a", "bba"));
    }

    #[test]
    fn export_then_import_renames_clashes() {
        let mut cfg = Config::default();
        cfg.macros.push(Macro::new("Combo", vec![MacroStep::Delay { ms: 5 }]));
        cfg.profiles[1].bindings.insert(
            "key:F1".into(),
            Action::Macro {
                name: "Combo".into(),
                mode: MacroMode::Once,
                count: 1,
                gap_ms: 0,
            },
        );
        let text = serde_json::to_string(&cfg.export(&[1])).unwrap();

        // Same macro content: reused, profile name made unique.
        let mut same = cfg.clone();
        assert_eq!(same.import(&text), Ok(1));
        assert_eq!(same.macros.len(), 1);
        assert_eq!(same.profiles.last().unwrap().name, "Genshin Impact (2)");

        // Different macro with the same name: imported copy gets renamed.
        let mut other = cfg.clone();
        other.macros[0].steps = vec![];
        assert_eq!(other.import(&text), Ok(1));
        assert_eq!(other.macros.len(), 2);
        assert_eq!(other.macros[1].name, "Combo (2)");
        assert!(matches!(
            &other.profiles.last().unwrap().bindings["key:F1"],
            Action::Macro { name, .. } if name == "Combo (2)"
        ));

        assert!(other.import("{}").is_err());
    }
}
