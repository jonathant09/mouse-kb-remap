//! The remapping engine: watches the focused window, picks the matching
//! profile and rewrites input.
//!
//! * `mod.rs` (this file): platform-independent types, `compile()` and the
//!   `Engine` facade the UI talks to.
//! * `dispatch.rs`: what happens to one input event (shared by all backends).
//! * `player.rs`: timed playback of macros, turbo and text on a worker thread.
//! * `windows.rs` / `linux.rs`: OS hooks and input injection.
//! * `win_actions.rs`: system actions (audio, windows, lock) on Windows.
//! * `fallback.rs`: no-op engine so the UI runs elsewhere.

use crate::config::{
    split_device, Action, AppRule, BindingMap, Combo, Config, Device, MacroMode, MacroStep, Modifier, MouseButton,
    Output, PadButton, StickPointer, SystemCommand, LAYER_COUNT,
};
use std::collections::HashMap;
use std::sync::Arc;

pub mod dispatch;
pub mod pad;
// Read directly over HID on Windows; on Linux the kernel drivers do this.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod pad_hid;
pub mod player;
// The virtual controller, minus the driver (win_vpad.rs, linux_vpad.rs).
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub mod vpad;

#[cfg(windows)]
mod win_actions;
#[cfg(windows)]
mod win_devices;
#[cfg(windows)]
mod win_hid;
#[cfg(windows)]
mod win_hidhide;
#[cfg(windows)]
mod win_vpad;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_focus;
#[cfg(target_os = "linux")]
mod linux_vpad;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(windows, target_os = "linux")))]
mod fallback;
#[cfg(not(any(windows, target_os = "linux")))]
use fallback as imp;

pub use player::{LaunchSpec, Plan, Step};

/// Sent to the UI whenever the foreground window (or the active profile)
/// changes.
#[derive(Clone, Debug, Default)]
pub struct FocusInfo {
    /// Executable name of the focused window, e.g. `GenshinImpact.exe`.
    pub exe: String,
    /// True when the focused window belongs to this app (remaps are paused).
    pub is_self: bool,
    /// True when the focused process runs as admin but we don't, so Windows
    /// will block our hooks from seeing/sending input to it.
    pub elevation_blocked: bool,
    /// Index of the profile in use (`None` while our own window is focused).
    pub profile: Option<usize>,
    /// The profile was chosen by hand (hotkey, command line), not by the
    /// focused app.
    pub forced: bool,
}

/// Things the app itself does, asked for by a hotkey, a binding or the
/// command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    TogglePause,
    NextProfile,
    AutoProfile,
}

/// Events from the engine threads to the UI.
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Focus(FocusInfo),
    /// A key pressed while capture mode was on: virtual key + modifier bits
    /// (see `Modifier::bit`) that were held with it.
    Captured {
        vk: u16,
        mods: u8,
    },
    Command(AppCommand),
    /// Worth an on-screen notification ("Layer 2 on").
    Notice(String),
    /// A button was pressed on a device while device capture was on.
    DeviceCaptured {
        /// `Device::id`.
        id: String,
        /// What the device calls itself, for a first name.
        name: String,
    },
    /// A controller input was pressed while "Detect button" was on.
    PadDetected {
        /// Input id, e.g. `pad:button7`.
        input: String,
    },
    /// The virtual controller was plugged in or out, or can't be.
    #[cfg_attr(not(windows), allow(dead_code))]
    Vpad(VpadState),
    /// Hiding the real controllers (HidHide) started, stopped or failed.
    #[cfg_attr(not(windows), allow(dead_code))]
    Hide(HideState),
}

/// Hiding the real controllers from games (Windows, HidHide).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HideState {
    /// Not hiding anything.
    #[default]
    Off,
    /// Hiding this many controller devices (a controller can have two).
    Hiding(usize),
    /// Wanted, but HidHide isn't installed.
    NotInstalled,
    Failed(String),
}

/// The virtual controller (Windows, ViGEmBus).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum VpadState {
    /// Not plugged in: the active profile doesn't send controller output.
    #[default]
    Unplugged,
    /// Plugged in, as XInput controller 1-4 if known.
    Plugged { slot: Option<u8> },
    /// Wanted, but the ViGEmBus driver isn't installed.
    NoDriver,
    /// Wanted, but plugging it in failed.
    Failed(String),
}

/// Index into `Compiled::devices` (and `Config::devices`).
pub type DeviceIndex = u16;

/// A recorded physical input, for building macros.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecEvent {
    Key { vk: u16, down: bool },
    Button { button: MouseButton, down: bool },
    Wheel(MouseButton),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Input {
    /// Windows virtual-key code (also used as the key identity on Linux).
    Key(u16),
    Mouse(MouseButton),
    /// Game controller button or stick direction.
    Pad(PadButton),
}

/// What an input does, ready for the hot path.
#[derive(Clone, Debug)]
pub enum Target {
    /// A key, with modifier bits held around it.
    Key {
        vk: u16,
        mods: u8,
    },
    Mouse(MouseButton),
    Disabled,
    /// Use binding layer 1-3 while held, or toggle it.
    Layer {
        layer: u8,
        toggle: bool,
    },
    /// Timed playback on the worker thread (macro, turbo, text, launch,
    /// system actions).
    Play(Arc<Plan>),
    /// Hold to set the pointer speed (1-20).
    Sensitivity(u32),
    /// Decided on release or after a while: tap, hold or double tap.
    TapHold(Arc<TapHold>),
    /// Decided on release by how far the pointer moved.
    Gesture(Arc<Gesture>),
    /// Controller gyro moves the pointer while held, or toggle it.
    Gyro {
        toggle: bool,
    },
    /// Hold a button, trigger or stick direction of the virtual controller.
    Pad(PadButton),
}

impl Target {
    /// Whether this sends virtual controller output (also from a tap / hold
    /// or gesture part, or a turbo of a controller button).
    pub fn uses_vpad(&self) -> bool {
        match self {
            Target::Pad(_) => true,
            Target::Play(plan) => plan.steps.iter().any(|s| matches!(s, Step::PadDown(_))),
            Target::TapHold(th) => [&th.tap, &th.hold, &th.double]
                .iter()
                .any(|p| p.as_ref().is_some_and(Target::uses_vpad)),
            Target::Gesture(g) => g
                .swipes
                .iter()
                .chain(std::iter::once(&g.tap))
                .any(|p| p.as_ref().is_some_and(Target::uses_vpad)),
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct Gesture {
    pub tap: Option<Target>,
    /// Up, down, left, right.
    pub swipes: [Option<Target>; 4],
    pub distance: u32,
}

impl Gesture {
    /// What a release `dx`, `dy` pixels away from the press does.
    pub fn pick(&self, dx: i32, dy: i32) -> Option<&Target> {
        let d = self.distance as i64;
        let (x, y) = (dx as i64, dy as i64);
        if x * x + y * y < d * d {
            return self.tap.as_ref();
        }
        let i = if y.abs() >= x.abs() {
            if y < 0 {
                0
            } else {
                1
            }
        } else if x < 0 {
            2
        } else {
            3
        };
        self.swipes[i].as_ref()
    }
}

#[derive(Debug)]
pub struct TapHold {
    pub tap: Option<Target>,
    pub hold: Option<Target>,
    pub double: Option<Target>,
    pub hold_ms: u32,
    pub double_ms: u32,
}

/// A binding that needs other modifiers / inputs held when `trigger` is
/// pressed.
#[derive(Clone, Debug)]
pub struct ComboBinding {
    pub mods: u8,
    pub held: Vec<Input>,
    pub trigger: Input,
    /// Only when the trigger comes from this device.
    pub device: Option<DeviceIndex>,
    pub target: Target,
}

impl ComboBinding {
    fn specificity(&self) -> u32 {
        self.mods.count_ones() + self.held.len() as u32
    }

    fn same_keys(&self, other: &ComboBinding) -> bool {
        self.mods == other.mods
            && self.trigger == other.trigger
            && self.held == other.held
            && self.device == other.device
    }
}

pub type Bindings = HashMap<Input, Target>;
/// Bindings that only apply to one device's input.
pub type DeviceBindings = HashMap<(DeviceIndex, Input), Target>;

/// The binding layers of a profile, Default already merged underneath.
#[derive(Default, Debug)]
pub struct Layers {
    pub maps: [Bindings; LAYER_COUNT],
    /// Per layer: bindings for one device, which win over `maps`.
    pub device_maps: [DeviceBindings; LAYER_COUNT],
    /// Per layer, most specific first.
    pub combos: [Vec<ComboBinding>; LAYER_COUNT],
    /// Controller stick that moves the pointer.
    pub stick_pointer: StickPointer,
    /// The controller gyro / touchpad moves the pointer.
    pub gyro_pointer: bool,
    pub touch_pointer: bool,
    /// Some input is bound to the Gyro action.
    pub gyro_bound: bool,
    /// Some binding sends virtual controller output (or the mouse moves
    /// one of its sticks).
    pub uses_vpad: bool,
    /// Virtual controller stick that mouse movement moves.
    pub mouse_stick: StickPointer,
}

impl Layers {
    /// Every input that triggers or is held for a binding.
    fn inputs(&self) -> impl Iterator<Item = &Input> {
        self.maps
            .iter()
            .flat_map(|m| m.keys())
            .chain(self.device_maps.iter().flat_map(|m| m.keys().map(|(_, i)| i)))
            .chain(
                self.combos
                    .iter()
                    .flatten()
                    .flat_map(|c| std::iter::once(&c.trigger).chain(c.held.iter())),
            )
    }

    /// Whether any mouse button or wheel input is remapped or part of a combo.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn uses_mouse(&self) -> bool {
        self.inputs().any(|i| matches!(i, Input::Mouse(_)))
    }

    /// Whether any controller input is remapped (or the stick moves the
    /// pointer), so the controller has to be read.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn uses_pad(&self) -> bool {
        self.stick_pointer != StickPointer::Off
            || self.gyro_pointer
            || self.touch_pointer
            || self.gyro_bound
            || self.inputs().any(|i| matches!(i, Input::Pad(_)))
    }

    /// Controller inputs the virtual controller shouldn't copy from a real
    /// one (`pad::bit` bits): the ones with a binding on any layer, the
    /// stick that moves the pointer, and Guide (Windows already opens the
    /// Game Bar for the real one).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn pad_excluded(&self) -> u64 {
        let mut mask = pad::bit(PadButton::Guide);
        for i in self.inputs() {
            if let Input::Pad(b) = i {
                mask |= pad::bit(*b);
            }
        }
        let stick = match self.stick_pointer {
            StickPointer::Left => [
                PadButton::LsUp,
                PadButton::LsDown,
                PadButton::LsLeft,
                PadButton::LsRight,
            ],
            StickPointer::Right => [
                PadButton::RsUp,
                PadButton::RsDown,
                PadButton::RsLeft,
                PadButton::RsRight,
            ],
            StickPointer::Off => return mask,
        };
        stick.iter().fold(mask, |m, b| m | pad::bit(*b))
    }

    /// Whether `input` does anything on any layer.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn binds(&self, input: &Input) -> bool {
        self.maps.iter().any(|m| m.contains_key(input))
            || self.device_maps.iter().any(|m| m.keys().any(|(_, i)| i == input))
            || self.combos.iter().flatten().any(|c| c.trigger == *input)
    }

    /// Whether what `input` does depends on the device it came from.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn device_specific(&self, input: &Input) -> bool {
        self.device_maps.iter().any(|m| m.keys().any(|(_, i)| i == input))
            || self
                .combos
                .iter()
                .flatten()
                .any(|c| c.device.is_some() && c.trigger == *input)
    }

    /// Whether any mouse button or wheel binding is limited to a device.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn mouse_device_bindings(&self) -> bool {
        let mouse = |i: &Input| matches!(i, Input::Mouse(_));
        self.device_maps.iter().any(|m| m.keys().any(|(_, i)| mouse(i)))
            || self
                .combos
                .iter()
                .flatten()
                .any(|c| c.device.is_some() && mouse(&c.trigger))
    }

    /// What `input` from `device` does: the highest active layer wins, and
    /// within a layer a combo whose keys are held wins over the plain
    /// binding, and a binding for the device over one for all devices.
    /// `active` has bit n set for each active layer n (the base layer always
    /// is); `mods` are the modifier bits held. Returns the target and
    /// whether it came from a combo.
    pub fn lookup(
        &self,
        input: &Input,
        device: Option<DeviceIndex>,
        active: u8,
        mods: u8,
        is_down: impl Fn(&Input) -> bool,
    ) -> Option<(&Target, bool)> {
        for layer in (0..LAYER_COUNT).rev() {
            if layer > 0 && active & (1 << layer) == 0 {
                continue;
            }
            for c in &self.combos[layer] {
                if c.trigger == *input
                    && (c.device.is_none() || c.device == device)
                    && c.mods & mods == c.mods
                    && c.held.iter().all(&is_down)
                {
                    return Some((&c.target, true));
                }
            }
            if let Some(d) = device {
                if let Some(t) = self.device_maps[layer].get(&(d, *input)) {
                    return Some((t, false));
                }
            }
            if let Some(t) = self.maps[layer].get(input) {
                return Some((t, false));
            }
        }
        None
    }
}

/// The focused window, as profile rules see it (lower-cased).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowInfo {
    pub exe: String,
    pub path: String,
    pub title: String,
    pub fullscreen: bool,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct CompiledProfile {
    pub rules: Vec<AppRule>,
    pub fullscreen_only: bool,
    pub layers: Arc<Layers>,
}

/// An app hotkey: key + exact modifier bits.
#[derive(Clone, Debug, PartialEq)]
pub struct HotkeyBinding {
    pub vk: u16,
    pub mods: u8,
    pub command: AppCommand,
}

/// Everything the engine needs from the config.
pub struct Compiled {
    pub profiles: Vec<CompiledProfile>,
    pub hotkeys: Vec<HotkeyBinding>,
    /// Named devices; `DeviceIndex` values index this.
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    pub devices: Vec<Device>,
    /// Pointer speed for a controller stick (1-20).
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    pub stick_speed: u32,
    /// Gyro, touchpad and which controllers to read.
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    pub pad: crate::config::PadSettings,
}

/// Which named device a backend device id belongs to: an exact id match,
/// else the same hardware interface (plugged into another port since), else
/// the same vendor and product: one mouse often reports through several
/// interfaces (`MI_00`, `MI_01`), and its buttons may come from either.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn match_device(devices: &[Device], id: &str) -> Option<DeviceIndex> {
    if let Some(i) = devices.iter().position(|d| d.id.eq_ignore_ascii_case(id)) {
        return Some(i as DeviceIndex);
    }
    let hw = hardware_part(id)?;
    let find = |key: &dyn Fn(&str) -> String| {
        let want = key(hw);
        devices
            .iter()
            .position(|d| hardware_part(&d.id).is_some_and(|h| key(h) == want))
            .map(|i| i as DeviceIndex)
    };
    find(&|h| h.to_ascii_uppercase()).or_else(|| find(&|h| vendor_product(h)))
}

/// `VID_046D&PID_C539` out of a hardware part.
fn vendor_product(hw: &str) -> String {
    hw.to_ascii_uppercase()
        .split('&')
        .filter(|p| p.starts_with("VID_") || p.starts_with("PID_"))
        .collect::<Vec<_>>()
        .join("&")
}

/// `VID_046D&PID_C539&MI_01&COL01` out of a Windows device path
/// (`\\?\HID#VID_046D&PID_C539&MI_01&Col01#8&2d8e1a4&0&0000#{…}`).
fn hardware_part(id: &str) -> Option<&str> {
    let mut parts = id.split('#');
    parts.next()?;
    let hw = parts.next()?;
    parts.next()?; // only device paths have an instance part
    hw.to_ascii_uppercase().contains("VID_").then_some(hw)
}

/// The profile for a window: the most specific matching rule wins (title,
/// then path, then exe name), then the first in the list. Default (0) when
/// nothing matches.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn pick_profile(profiles: &[CompiledProfile], w: &WindowInfo) -> usize {
    let mut best: Option<(u8, usize)> = None;
    for (i, p) in profiles.iter().enumerate().skip(1) {
        if p.fullscreen_only && !w.fullscreen {
            continue;
        }
        let score = p
            .rules
            .iter()
            .filter_map(|r| r.matches(&w.exe, &w.path, &w.title))
            .max();
        if let Some(s) = score {
            if best.is_none_or(|(b, _)| s > b) {
                best = Some((s, i));
            }
        }
    }
    best.map(|(_, i)| i).unwrap_or(0)
}

/// Whether the focused window has to be re-checked now and then (window
/// titles and fullscreen change without a focus change).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn needs_window_poll(profiles: &[CompiledProfile]) -> bool {
    profiles
        .iter()
        .any(|p| p.fullscreen_only || p.rules.iter().any(|r| matches!(r, AppRule::Title(_))))
}

pub fn parse_input(id: &str) -> Option<Input> {
    if let Some(k) = id.strip_prefix("key:") {
        crate::keys::find(k).map(|k| Input::Key(k.vk))
    } else if let Some(m) = id.strip_prefix("mouse:") {
        MouseButton::from_id(m).map(Input::Mouse)
    } else if let Some(p) = id.strip_prefix("pad:") {
        PadButton::from_id(p).map(Input::Pad)
    } else {
        None
    }
}

fn key_vk(id: &str) -> Option<u16> {
    crate::keys::find(id).map(|k| k.vk)
}

/// Steps that press `output` (mods first), hold it for `hold_ms`, release it.
fn press_steps(output: &Output, hold_ms: u32) -> Option<Vec<Step>> {
    let mut steps = Vec::new();
    match output {
        Output::Key { key, mods } => {
            let vk = key_vk(key)?;
            let mods: Vec<u16> = Modifier::ALL
                .iter()
                .filter(|m| mods.contains(m))
                .map(|m| m.vk())
                .collect();
            steps.extend(mods.iter().map(|&m| Step::KeyDown(m)));
            steps.push(Step::KeyDown(vk));
            steps.push(Step::Delay(hold_ms));
            steps.push(Step::KeyUp(vk));
            steps.extend(mods.iter().rev().map(|&m| Step::KeyUp(m)));
        }
        Output::Mouse { button } if button.is_wheel() => steps.push(Step::Wheel(*button)),
        Output::Mouse { button } => {
            steps.push(Step::ButtonDown(*button));
            steps.push(Step::Delay(hold_ms));
            steps.push(Step::ButtonUp(*button));
        }
        Output::Pad { button } if button.is_output() => {
            steps.push(Step::PadDown(*button));
            steps.push(Step::Delay(hold_ms));
            steps.push(Step::PadUp(*button));
        }
        Output::Pad { .. } => return None,
    }
    Some(steps)
}

/// Turn macro steps from the config into player steps. Unknown keys are skipped.
pub fn macro_steps(steps: &[MacroStep]) -> Vec<Step> {
    let mut out = Vec::new();
    for s in steps {
        match s {
            MacroStep::KeyDown { key } => out.extend(key_vk(key).map(Step::KeyDown)),
            MacroStep::KeyUp { key } => out.extend(key_vk(key).map(Step::KeyUp)),
            MacroStep::KeyTap { key, hold_ms } => {
                if let Some(vk) = key_vk(key) {
                    out.extend([Step::KeyDown(vk), Step::Delay(*hold_ms), Step::KeyUp(vk)]);
                }
            }
            MacroStep::MouseDown { button } if button.is_wheel() => out.push(Step::Wheel(*button)),
            MacroStep::MouseDown { button } => out.push(Step::ButtonDown(*button)),
            MacroStep::MouseUp { button } if button.is_wheel() => {}
            MacroStep::MouseUp { button } => out.push(Step::ButtonUp(*button)),
            MacroStep::MouseClick { button, .. } if button.is_wheel() => out.push(Step::Wheel(*button)),
            MacroStep::MouseClick { button, hold_ms } => out.extend([
                Step::ButtonDown(*button),
                Step::Delay(*hold_ms),
                Step::ButtonUp(*button),
            ]),
            MacroStep::Delay { ms } => out.push(Step::Delay(*ms)),
            MacroStep::Text { text } => out.extend(text_steps(text, 0)),
            MacroStep::MouseMove { x, y, absolute } => out.push(Step::Move {
                x: *x,
                y: *y,
                absolute: *absolute,
            }),
            MacroStep::PadTap { button, hold_ms } if button.is_output() => {
                out.extend([Step::PadDown(*button), Step::Delay(*hold_ms), Step::PadUp(*button)])
            }
            MacroStep::PadDown { button } if button.is_output() => out.push(Step::PadDown(*button)),
            MacroStep::PadUp { button } if button.is_output() => out.push(Step::PadUp(*button)),
            MacroStep::PadTap { .. } | MacroStep::PadDown { .. } | MacroStep::PadUp { .. } => {}
        }
    }
    out
}

fn text_steps(text: &str, delay_ms: u32) -> Vec<Step> {
    let mut out = Vec::new();
    for (i, c) in text.chars().filter(|c| *c != '\r').enumerate() {
        if i > 0 && delay_ms > 0 {
            out.push(Step::Delay(delay_ms));
        }
        out.push(Step::Char(c));
    }
    out
}

/// Shortest allowed turbo interval. Faster than this tends to be dropped by
/// games and floods the input queue.
pub const MIN_TURBO_INTERVAL_MS: u32 = 5;

fn parse_action(action: &Action, cfg: &Config) -> Option<Target> {
    Some(match action {
        Action::Key { key, mods } => Target::Key {
            vk: key_vk(key)?,
            mods: Modifier::to_bits(mods),
        },
        Action::Mouse { button } => Target::Mouse(*button),
        Action::Disabled => Target::Disabled,
        Action::Shift => Target::Layer {
            layer: 1,
            toggle: false,
        },
        Action::Layer { layer, toggle } => Target::Layer {
            layer: (*layer).clamp(1, LAYER_COUNT as u8 - 1),
            toggle: *toggle,
        },
        Action::Sensitivity { speed } => Target::Sensitivity((*speed).clamp(1, 20)),
        Action::Macro {
            name,
            mode,
            count,
            gap_ms,
        } => {
            let m = cfg.find_macro(name)?;
            Target::Play(Arc::new(
                Plan::new(
                    macro_steps(&m.steps),
                    match mode {
                        MacroMode::Once => player::Mode::Once,
                        MacroMode::Repeat => player::Mode::Repeat((*count).max(1)),
                        MacroMode::WhileHeld => player::Mode::WhileHeld,
                        MacroMode::Toggle => player::Mode::Toggle,
                    },
                    *gap_ms,
                    *gap_ms,
                )
                .with_jitter(m.jitter_ms)
                .with_label(&m.name),
            ))
        }
        Action::Turbo {
            output,
            interval_ms,
            hold_ms,
            delay_ms,
            toggle,
        } => {
            let interval = (*interval_ms).max(MIN_TURBO_INTERVAL_MS);
            // Leave at least 1 ms released between presses.
            let hold = (*hold_ms).clamp(1, interval - 1);
            let mut steps = press_steps(output, hold)?;
            let is_wheel = matches!(output, Output::Mouse { button } if button.is_wheel());
            steps.push(Step::Delay(if is_wheel { interval } else { interval - hold }));
            Target::Play(Arc::new(
                Plan::new(
                    steps,
                    if *toggle {
                        player::Mode::Toggle
                    } else {
                        player::Mode::WhileHeld
                    },
                    0,
                    delay_ms.saturating_sub(interval),
                )
                .with_label(&format!("Turbo {}", output.label())),
            ))
        }
        Action::Text { text, delay_ms } => Target::Play(Arc::new(Plan::new(
            text_steps(text, *delay_ms),
            player::Mode::Once,
            0,
            0,
        ))),
        Action::Launch { path, args } => {
            if path.trim().is_empty() {
                return None;
            }
            Target::Play(Arc::new(Plan::new(
                vec![Step::Launch(Arc::new(LaunchSpec {
                    path: path.trim().to_string(),
                    args: args.clone(),
                }))],
                player::Mode::Once,
                0,
                0,
            )))
        }
        Action::System { command } => system_target(*command),
        Action::Gyro { toggle } => Target::Gyro { toggle: *toggle },
        Action::Pad { button } if button.is_output() => Target::Pad(*button),
        Action::Pad { .. } => return None,
        Action::TapHold {
            tap,
            hold,
            double,
            hold_ms,
            double_ms,
        } => {
            let part = |a: &Option<Box<Action>>| nested(a, cfg);
            let spec = TapHold {
                tap: part(tap),
                hold: part(hold),
                double: part(double),
                hold_ms: (*hold_ms).clamp(10, 5000),
                double_ms: (*double_ms).clamp(10, 5000),
            };
            if spec.tap.is_none() && spec.hold.is_none() && spec.double.is_none() {
                Target::Disabled
            } else {
                Target::TapHold(Arc::new(spec))
            }
        }
        Action::Gesture {
            tap,
            up,
            down,
            left,
            right,
            distance,
        } => Target::Gesture(Arc::new(Gesture {
            tap: nested(tap, cfg),
            swipes: [up, down, left, right].map(|a| nested(a, cfg)),
            distance: (*distance).clamp(5, 2000),
        })),
    })
}

/// A part of a tap / hold or gesture. One level only: a tap / hold or
/// gesture inside another does nothing.
fn nested(a: &Option<Box<Action>>, cfg: &Config) -> Option<Target> {
    a.as_deref()
        .filter(|a| !a.has_slots())
        .and_then(|a| parse_action(a, cfg))
}

fn system_target(command: SystemCommand) -> Target {
    Target::Play(Arc::new(Plan::new(
        vec![Step::System(command)],
        player::Mode::Once,
        0,
        0,
    )))
}

/// One compiled binding layer.
#[derive(Clone, Default)]
struct LayerParts {
    plain: Bindings,
    device: DeviceBindings,
    combos: Vec<ComboBinding>,
}

/// One binding layer: plain inputs, inputs of one device, and combos.
/// Bindings for a device that isn't in the device list are skipped.
fn compile_layer(map: &BindingMap, cfg: &Config) -> LayerParts {
    let mut out = LayerParts::default();
    for (id, action) in map {
        let (id, device_name) = split_device(id);
        let device = match device_name {
            Some(name) => match cfg.devices.iter().position(|d| d.name == name) {
                Some(i) => Some(i as DeviceIndex),
                None => continue,
            },
            None => None,
        };
        let Some(target) = parse_action(action, cfg) else {
            continue;
        };
        if let Some(c) = Combo::parse(id) {
            let (Some(trigger), Some(held)) = (
                parse_input(&c.trigger),
                c.held.iter().map(|h| parse_input(h)).collect::<Option<Vec<_>>>(),
            ) else {
                continue;
            };
            out.combos.push(ComboBinding {
                mods: Modifier::to_bits(&c.mods),
                held,
                trigger,
                device,
                target,
            });
        } else if let Some(input) = parse_input(id) {
            match device {
                Some(d) => out.device.insert((d, input), target),
                None => out.plain.insert(input, target),
            };
        }
    }
    out
}

fn compile_hotkeys(cfg: &Config) -> Vec<HotkeyBinding> {
    let commands = [
        AppCommand::TogglePause,
        AppCommand::NextProfile,
        AppCommand::AutoProfile,
    ];
    commands
        .iter()
        .enumerate()
        .filter_map(|(i, &command)| {
            let h = cfg.hotkeys.get(i)?;
            Some(HotkeyBinding {
                vk: key_vk(&h.key)?,
                mods: Modifier::to_bits(&h.mods),
                command,
            })
        })
        .collect()
}

/// Turn the user-facing config into lookup tables the input hook can use
/// without allocating.
pub fn compile(cfg: &Config) -> Compiled {
    let default: Vec<LayerParts> = (0..LAYER_COUNT)
        .map(|l| {
            cfg.profiles
                .first()
                .map(|p| compile_layer(p.layer(l), cfg))
                .unwrap_or_default()
        })
        .collect();
    let merged = |index: usize| -> Layers {
        let own = (index > 0).then(|| &cfg.profiles[index]);
        let mut layers = Layers {
            stick_pointer: cfg.stick_pointer(index),
            gyro_pointer: cfg.gyro_pointer(index),
            touch_pointer: cfg.touch_pointer(index),
            mouse_stick: cfg.mouse_stick(index),
            ..Layers::default()
        };
        for (l, inherited) in default.iter().enumerate() {
            let mut parts = inherited.clone();
            if let Some(p) = own {
                let mine = compile_layer(p.layer(l), cfg);
                parts.plain.extend(mine.plain);
                parts.device.extend(mine.device);
                // The profile's own combo replaces Default's for the same keys.
                parts.combos.retain(|c| !mine.combos.iter().any(|o| o.same_keys(c)));
                parts.combos.splice(0..0, mine.combos);
            }
            // Most specific first, a device's own before all devices'; stable,
            // so the profile's come before Default's.
            parts
                .combos
                .sort_by_key(|c| (std::cmp::Reverse(c.specificity()), c.device.is_none()));
            layers.maps[l] = parts.plain;
            layers.device_maps[l] = parts.device;
            layers.combos[l] = parts.combos;
        }
        let is_gyro = |t: &Target| match t {
            Target::Gyro { .. } => true,
            Target::TapHold(th) => [&th.tap, &th.hold, &th.double]
                .iter()
                .any(|p| matches!(p, Some(Target::Gyro { .. }))),
            _ => false,
        };
        layers.gyro_bound = layers.maps.iter().flat_map(|m| m.values()).any(is_gyro)
            || layers.device_maps.iter().flat_map(|m| m.values()).any(is_gyro)
            || layers.combos.iter().flatten().any(|c| is_gyro(&c.target));
        layers.uses_vpad = layers.maps.iter().flat_map(|m| m.values()).any(Target::uses_vpad)
            || layers
                .device_maps
                .iter()
                .flat_map(|m| m.values())
                .any(Target::uses_vpad)
            || layers.combos.iter().flatten().any(|c| c.target.uses_vpad());
        layers.uses_vpad |= layers.mouse_stick != StickPointer::Off;
        layers
    };
    let mut profiles: Vec<CompiledProfile> = cfg
        .profiles
        .iter()
        .enumerate()
        .map(|(i, p)| CompiledProfile {
            rules: if i == 0 {
                vec![]
            } else {
                p.apps.iter().map(|a| AppRule::parse(a)).collect()
            },
            fullscreen_only: i > 0 && p.fullscreen_only,
            layers: Arc::new(merged(i)),
        })
        .collect();
    if profiles.is_empty() {
        profiles.push(CompiledProfile {
            rules: vec![],
            fullscreen_only: false,
            layers: Arc::new(merged(0)),
        });
    }
    Compiled {
        profiles,
        hotkeys: compile_hotkeys(cfg),
        devices: cfg.devices.clone(),
        stick_speed: cfg.stick_speed.clamp(1, 20),
        pad: cfg.pad.clone(),
    }
}

/// Turn recorded input into macro steps, with the recorded pauses as delays.
/// Key auto-repeat is dropped.
pub fn recording_to_steps(events: &[(std::time::Duration, RecEvent)]) -> Vec<MacroStep> {
    let mut steps = Vec::new();
    let mut last: Option<std::time::Duration> = None;
    let mut down: Vec<u16> = Vec::new();
    for (t, ev) in events {
        let key_id = |vk: u16| crate::keys::find_by_vk(vk).map(|k| k.id.to_string());
        let step = match *ev {
            RecEvent::Key { vk, down: true } => {
                if down.contains(&vk) {
                    continue; // auto-repeat
                }
                down.push(vk);
                key_id(vk).map(|key| MacroStep::KeyDown { key })
            }
            RecEvent::Key { vk, down: false } => {
                down.retain(|v| *v != vk);
                key_id(vk).map(|key| MacroStep::KeyUp { key })
            }
            RecEvent::Button { button, down: true } => Some(MacroStep::MouseDown { button }),
            RecEvent::Button { button, down: false } => Some(MacroStep::MouseUp { button }),
            RecEvent::Wheel(button) => Some(MacroStep::MouseClick { button, hold_ms: 0 }),
        };
        let Some(step) = step else { continue };
        if let Some(prev) = last {
            let ms = t.saturating_sub(prev).as_millis() as u32;
            if ms > 0 {
                steps.push(MacroStep::Delay { ms });
            }
        }
        last = Some(*t);
        steps.push(step);
    }
    steps
}

/// Whether this system can have a virtual controller (Windows: ViGEmBus;
/// Linux: uinput).
pub const VPAD_SUPPORTED: bool = cfg!(any(windows, target_os = "linux"));

/// Whether the virtual controller's driver (ViGEmBus) is installed.
pub fn vpad_driver_present() -> bool {
    #[cfg(windows)]
    {
        win_vpad::driver_present()
    }
    #[cfg(target_os = "linux")]
    {
        linux_vpad::driver_present()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Look for the virtual controller's driver again (it was just installed).
pub fn vpad_retry() {
    vpad::retry();
}

/// Whether HidHide (for hiding the real controllers) is installed.
pub fn hidhide_present() -> bool {
    #[cfg(windows)]
    {
        win_hidhide::driver_present()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Look for HidHide again (it was just installed).
pub fn hidhide_retry() {
    #[cfg(windows)]
    win_hidhide::retry();
}

/// Hiding the real controllers now.
pub fn hide_status() -> HideState {
    #[cfg(windows)]
    {
        win_hidhide::status()
    }
    #[cfg(not(windows))]
    {
        HideState::Off
    }
}

/// The virtual controller now.
pub fn vpad_status() -> VpadState {
    vpad::status()
}

pub struct Engine {
    inner: imp::Engine,
}

impl Engine {
    /// Start the engine. `on_event` is called from the engine threads.
    pub fn start(cfg: &Config, on_event: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        Engine {
            inner: imp::Engine::start(compile(cfg), cfg.enabled, Box::new(on_event)),
        }
    }

    /// Push a new configuration to the running engine. Stops running macros.
    pub fn apply(&self, cfg: &Config) {
        self.inner.apply(compile(cfg), cfg.enabled);
    }

    /// Use this profile whatever app is focused, or `None` to go back to
    /// choosing by the focused app.
    pub fn force_profile(&self, index: Option<usize>) {
        self.inner.force_profile(index);
    }

    /// Whether this platform has a real input backend.
    pub fn supported(&self) -> bool {
        imp::SUPPORTED
    }

    /// Whether this process runs with admin rights.
    pub fn elevated(&self) -> bool {
        self.inner.elevated()
    }

    /// Report the next key press as `EngineEvent::Captured` instead of passing
    /// it on. Returns false if the backend can't capture (use the UI instead).
    pub fn start_capture(&self) -> bool {
        self.inner.start_capture()
    }

    pub fn cancel_capture(&self) {
        self.inner.cancel_capture();
    }

    /// Report the next button pressed on a mouse or controller as
    /// `EngineEvent::DeviceCaptured` (the press itself goes through).
    pub fn capture_device(&self, on: bool) {
        self.inner.capture_device(on);
    }

    /// Report the next controller input pressed as
    /// `EngineEvent::PadDetected` (instead of acting on it).
    pub fn detect_pad(&self, on: bool) {
        self.inner.detect_pad(on);
    }

    /// Start recording keys and mouse buttons for a macro.
    pub fn start_recording(&self) -> bool {
        self.inner.start_recording()
    }

    /// Stop recording; returns events with their time since the start.
    pub fn stop_recording(&self) -> Vec<(std::time::Duration, RecEvent)> {
        self.inner.stop_recording()
    }

    /// Stop all playback and restore the pointer speed (on quit).
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Hotkey, Macro, Profile};

    fn key(k: &str) -> Action {
        Action::Key {
            key: k.into(),
            mods: vec![],
        }
    }

    const NONE_DOWN: fn(&Input) -> bool = |_| false;

    #[test]
    fn profiles_inherit_default_per_layer() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert("key:F1".into(), key("A"));
        cfg.profiles[0].shift_bindings.insert("key:F2".into(), key("B"));
        cfg.profiles[0].layer_mut(3).insert("key:F3".into(), key("D"));
        let mut game = Profile::new("Game");
        game.apps.push("Game.EXE".into());
        game.bindings.insert("key:F1".into(), key("C"));
        cfg.profiles.push(game);

        let compiled = compile(&cfg);
        let game = compiled.profiles.last().unwrap();
        assert_eq!(game.rules, vec![AppRule::Exe("game.exe".into())]);
        let f1 = Input::Key(0x70);
        let f2 = Input::Key(0x71);
        let f3 = Input::Key(0x72);
        let get = |i: &Input, active: u8| game.layers.lookup(i, None, active, 0, NONE_DOWN).map(|(t, _)| t);
        assert!(matches!(get(&f1, 0), Some(Target::Key { vk: 0x43, .. })));
        // Layer lookups fall back to the base layer...
        assert!(matches!(get(&f1, 0b10), Some(Target::Key { vk: 0x43, .. })));
        // ...and see Default's layers.
        assert!(matches!(get(&f2, 0b10), Some(Target::Key { vk: 0x42, .. })));
        assert!(get(&f2, 0).is_none());
        assert!(matches!(get(&f3, 0b1000), Some(Target::Key { vk: 0x44, .. })));
        assert!(get(&f3, 0b10).is_none());
    }

    #[test]
    fn unknown_keys_and_missing_macros_are_skipped() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert("key:Nope".into(), key("A"));
        cfg.profiles[0].bindings.insert("key:F1".into(), key("Nope"));
        cfg.profiles[0].bindings.insert("ctrl+key:Nope".into(), key("A"));
        cfg.profiles[0].bindings.insert(
            "key:F2".into(),
            Action::Macro {
                name: "missing".into(),
                mode: MacroMode::Once,
                count: 1,
                gap_ms: 0,
            },
        );
        let c = compile(&cfg);
        assert!(c.profiles[0].layers.maps[0].is_empty());
        assert!(c.profiles[0].layers.combos[0].is_empty());
    }

    #[test]
    fn combos_compile_most_specific_first_and_profiles_override_default() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert("ctrl+key:F1".into(), key("A"));
        cfg.profiles[0].bindings.insert("ctrl+shift+key:F1".into(), key("B"));
        cfg.profiles[1].bindings.insert("ctrl+key:F1".into(), key("C"));
        cfg.profiles[1]
            .bindings
            .insert("mouse:right+mouse:left".into(), key("D"));
        let c = compile(&cfg);
        let combos = &c.profiles[1].layers.combos[0];
        assert_eq!(combos.len(), 3);
        assert_eq!(combos[0].mods, 1 | 2);
        let f1 = Input::Key(0x70);
        let ctrl = Modifier::Ctrl.bit();
        let t = |mods| {
            c.profiles[1]
                .layers
                .lookup(&f1, None, 0, mods, NONE_DOWN)
                .map(|(t, combo)| (t.clone(), combo))
        };
        assert!(matches!(t(ctrl), Some((Target::Key { vk: 0x43, .. }, true))));
        assert!(matches!(t(ctrl | 2), Some((Target::Key { vk: 0x42, .. }, true))));
        assert!(t(0).is_none());
        // Held inputs.
        let left = Input::Mouse(MouseButton::Left);
        let right = Input::Mouse(MouseButton::Right);
        let l = &c.profiles[1].layers;
        assert!(l.lookup(&left, None, 0, 0, NONE_DOWN).is_none());
        assert!(l.lookup(&left, None, 0, 0, |i| *i == right).is_some());
        assert!(l.uses_mouse());
        assert!(!c.profiles[0].layers.uses_mouse());
    }

    #[test]
    fn tap_hold_compiles_parts_and_drops_nesting() {
        let mut cfg = Config::default();
        let nested = Action::TapHold {
            tap: Some(Box::new(key("A"))),
            hold: None,
            double: None,
            hold_ms: 200,
            double_ms: 250,
        };
        cfg.profiles[0].bindings.insert(
            "key:CapsLock".into(),
            Action::TapHold {
                tap: Some(Box::new(key("Esc"))),
                hold: Some(Box::new(Action::Shift)),
                double: Some(Box::new(nested)),
                hold_ms: 1,
                double_ms: 300,
            },
        );
        let c = compile(&cfg);
        let Some(Target::TapHold(t)) = c.profiles[0].layers.maps[0].get(&Input::Key(0x14)) else {
            panic!("expected tap / hold");
        };
        assert!(matches!(t.tap, Some(Target::Key { vk: 0x1B, .. })));
        assert!(matches!(
            t.hold,
            Some(Target::Layer {
                layer: 1,
                toggle: false
            })
        ));
        assert!(t.double.is_none());
        assert_eq!(t.hold_ms, 10);
        assert_eq!(t.double_ms, 300);
    }

    #[test]
    fn profiles_are_picked_by_the_most_specific_rule() {
        let mut cfg = Config::default();
        let mut browser = Profile::new("Browser");
        browser.apps.push("chrome.exe".into());
        let mut video = Profile::new("Video");
        video.apps.push("title:YouTube".into());
        let mut game = Profile::new("Game");
        game.apps.push("game.exe".into());
        game.fullscreen_only = true;
        cfg.profiles.extend([browser, video, game]);
        let c = compile(&cfg);
        let w = |exe: &str, title: &str, fullscreen| WindowInfo {
            exe: exe.into(),
            path: format!(r"c:\x\{exe}"),
            title: title.into(),
            fullscreen,
        };
        assert_eq!(pick_profile(&c.profiles, &w("chrome.exe", "news", false)), 2);
        assert_eq!(pick_profile(&c.profiles, &w("chrome.exe", "cats - youtube", false)), 3);
        assert_eq!(pick_profile(&c.profiles, &w("game.exe", "", false)), 0);
        assert_eq!(pick_profile(&c.profiles, &w("game.exe", "", true)), 4);
        assert!(needs_window_poll(&c.profiles));
    }

    #[test]
    fn device_controller_and_gesture_bindings_compile() {
        let mut cfg = Config::default();
        cfg.devices.push(Device {
            name: "Pad".into(),
            id: "xinput:1".into(),
        });
        cfg.devices.push(Device {
            name: "Ball".into(),
            id: r"\\?\HID#VID_046D&PID_C539&MI_01&Col01#8&2d8e1a4&0&0000#{378de44c-56ef-11d1-bc8c-00a0c91405dd}".into(),
        });
        cfg.profiles[0].bindings.insert("pad:a@Pad".into(), key("A"));
        cfg.profiles[0].bindings.insert("pad:b".into(), key("B"));
        cfg.profiles[0].bindings.insert("mouse:back@Ball".into(), key("C"));
        cfg.profiles[0].bindings.insert("mouse:back@Gone".into(), key("D")); // no such device
        cfg.profiles[1].bindings.insert(
            "mouse:right".into(),
            Action::Gesture {
                tap: None,
                up: Some(Box::new(key("E"))),
                down: Some(Box::new(Action::Gesture {
                    tap: None,
                    up: None,
                    down: None,
                    left: None,
                    right: None,
                    distance: 1,
                })),
                left: None,
                right: None,
                distance: 1,
            },
        );
        cfg.profiles[1].stick_pointer = Some(StickPointer::Left);
        let c = compile(&cfg);
        let l = &c.profiles[0].layers;
        let pad_a = Input::Pad(PadButton::A);
        let back = Input::Mouse(MouseButton::Back);
        assert!(l.lookup(&pad_a, None, 0, 0, NONE_DOWN).is_none());
        assert!(l.lookup(&pad_a, Some(0), 0, 0, NONE_DOWN).is_some());
        assert!(l.lookup(&back, Some(1), 0, 0, NONE_DOWN).is_some());
        assert_eq!(l.device_maps[0].len(), 2);
        assert!(l.uses_pad() && l.uses_mouse() && l.mouse_device_bindings());
        assert!(l.device_specific(&back) && !l.device_specific(&Input::Pad(PadButton::B)));
        assert_eq!(l.stick_pointer, StickPointer::Off);
        let game = &c.profiles[1].layers;
        assert_eq!(game.stick_pointer, StickPointer::Left);
        let Some(Target::Gesture(g)) = game.maps[0].get(&Input::Mouse(MouseButton::Right)) else {
            panic!("expected a gesture");
        };
        assert_eq!(g.distance, 5);
        assert!(g.swipes[0].is_some());
        assert!(g.swipes[1].is_none(), "a gesture inside a gesture is dropped");
        assert!(matches!(g.pick(0, -9), Some(Target::Key { vk: 0x45, .. })));
        assert!(g.pick(2, 2).is_none());

        // Device ids: exact, or the same hardware in another port.
        assert_eq!(match_device(&cfg.devices, "XINPUT:1"), Some(0));
        assert_eq!(match_device(&cfg.devices, "xinput:2"), None);
        let moved = r"\\?\HID#VID_046D&PID_C539&MI_01&Col01#9&1111111&0&0000#{378de44c-56ef-11d1-bc8c-00a0c91405dd}";
        assert_eq!(match_device(&cfg.devices, moved), Some(1));
        let other = r"\\?\HID#VID_1532&PID_0084&MI_00#9&1111111&0&0000#{378de44c-56ef-11d1-bc8c-00a0c91405dd}";
        assert_eq!(match_device(&cfg.devices, other), None);
        // Another interface of the same mouse.
        let sibling = r"\\?\HID#VID_046D&PID_C539&MI_00#8&3333333&0&0000#{378de44c-56ef-11d1-bc8c-00a0c91405dd}";
        assert_eq!(match_device(&cfg.devices, sibling), Some(1));
        // No vendor / product in the path: exact matches only.
        assert_eq!(
            match_device(&cfg.devices, r"\\?\HID#hid&Col02#1&2d595ca7&0&0001#{x}"),
            None
        );
    }

    #[test]
    fn hotkeys_compile() {
        let mut cfg = Config::default();
        cfg.hotkeys.next_profile = Some(Hotkey {
            key: "F12".into(),
            mods: vec![Modifier::Ctrl, Modifier::Alt],
        });
        cfg.hotkeys.pause = Some(Hotkey {
            key: "Nope".into(),
            mods: vec![],
        });
        assert_eq!(
            compile(&cfg).hotkeys,
            vec![HotkeyBinding {
                vk: 0x7B,
                mods: 1 | 4,
                command: AppCommand::NextProfile
            }]
        );
    }

    #[test]
    fn turbo_plan_timing() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert(
            "mouse:back".into(),
            Action::Turbo {
                output: Output::Key {
                    key: "Q".into(),
                    mods: vec![Modifier::Shift],
                },
                interval_ms: 50,
                hold_ms: 80, // longer than the interval: clamped
                delay_ms: 400,
                toggle: false,
            },
        );
        let compiled = compile(&cfg);
        let Some(Target::Play(plan)) = compiled.profiles[0].layers.maps[0].get(&Input::Mouse(MouseButton::Back)) else {
            panic!("expected a plan");
        };
        assert_eq!(
            plan.steps,
            vec![
                Step::KeyDown(0xA0),
                Step::KeyDown(0x51),
                Step::Delay(49),
                Step::KeyUp(0x51),
                Step::KeyUp(0xA0),
                Step::Delay(1),
            ]
        );
        assert_eq!(plan.mode, player::Mode::WhileHeld);
        assert_eq!(plan.first_gap_ms, 350);
    }

    #[test]
    fn macro_steps_expand_taps_text_and_moves() {
        let steps = macro_steps(&[
            MacroStep::KeyTap {
                key: "A".into(),
                hold_ms: 10,
            },
            MacroStep::MouseClick {
                button: MouseButton::WheelUp,
                hold_ms: 10,
            },
            MacroStep::Text { text: "hi".into() },
            MacroStep::MouseMove {
                x: -5,
                y: 7,
                absolute: false,
            },
        ]);
        assert_eq!(
            steps,
            vec![
                Step::KeyDown(0x41),
                Step::Delay(10),
                Step::KeyUp(0x41),
                Step::Wheel(MouseButton::WheelUp),
                Step::Char('h'),
                Step::Char('i'),
                Step::Move {
                    x: -5,
                    y: 7,
                    absolute: false
                },
            ]
        );
        let _ = Macro::new("", vec![]);
    }

    #[test]
    fn recording_becomes_steps_with_delays() {
        use std::time::Duration;
        let ms = Duration::from_millis;
        let steps = recording_to_steps(&[
            (ms(0), RecEvent::Key { vk: 0x41, down: true }),
            (ms(30), RecEvent::Key { vk: 0x41, down: true }), // auto-repeat
            (ms(80), RecEvent::Key { vk: 0x41, down: false }),
            (
                ms(80),
                RecEvent::Button {
                    button: MouseButton::Left,
                    down: true,
                },
            ),
        ]);
        assert_eq!(
            steps,
            vec![
                MacroStep::KeyDown { key: "A".into() },
                MacroStep::Delay { ms: 80 },
                MacroStep::KeyUp { key: "A".into() },
                MacroStep::MouseDown {
                    button: MouseButton::Left
                },
            ]
        );
    }

    #[test]
    fn controller_output_compiles_and_saves() {
        let mut cfg = Config::default();
        cfg.profiles[0]
            .bindings
            .insert("key:Space".into(), Action::Pad { button: PadButton::A });
        // Not something the virtual controller has: ignored.
        cfg.profiles[0].bindings.insert(
            "key:Q".into(),
            Action::Pad {
                button: PadButton::Touchpad,
            },
        );
        cfg.profiles[0].bindings.insert(
            "pad:lb".into(),
            Action::Turbo {
                output: Output::Pad { button: PadButton::Rt },
                interval_ms: 100,
                hold_ms: 30,
                delay_ms: 0,
                toggle: false,
            },
        );
        cfg.profiles[0].stick_pointer = Some(StickPointer::Right);
        let compiled = compile(&cfg);
        let layers = &compiled.profiles[0].layers;
        assert!(layers.uses_vpad);
        assert!(matches!(
            layers.maps[0].get(&Input::Key(0x20)),
            Some(Target::Pad(PadButton::A))
        ));
        assert!(!layers.maps[0].contains_key(&Input::Key(0x51)));
        let Some(Target::Play(plan)) = layers.maps[0].get(&Input::Pad(PadButton::Lb)) else {
            panic!("expected a turbo plan");
        };
        assert_eq!(
            plan.steps[..3],
            [
                Step::PadDown(PadButton::Rt),
                Step::Delay(30),
                Step::PadUp(PadButton::Rt)
            ]
        );
        // Copying a real controller leaves out remapped inputs, the pointer
        // stick and Guide.
        let excluded = layers.pad_excluded();
        for b in [PadButton::Lb, PadButton::Guide, PadButton::RsUp, PadButton::RsLeft] {
            assert!(excluded & pad::bit(b) != 0, "{b:?}");
        }
        for b in [PadButton::A, PadButton::Rb, PadButton::LsUp] {
            assert!(excluded & pad::bit(b) == 0, "{b:?}");
        }
        // Saved by id, and read back.
        let json = serde_json::to_string(&cfg.profiles[0].bindings).unwrap();
        assert!(json.contains(r#"{"type":"pad","button":"a"}"#), "{json}");
        let back: crate::config::BindingMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg.profiles[0].bindings);
        assert!(
            serde_json::from_str::<crate::config::BindingMap>(r#"{"key:A":{"type":"pad","button":"nope"}}"#).is_err()
        );

        let mut other = Config::default();
        other.profiles[0].bindings.insert("key:A".into(), key("B"));
        assert!(!compile(&other).profiles[0].layers.uses_vpad);
    }

    #[test]
    fn controller_macro_steps_and_mouse_stick_use_the_virtual_controller() {
        let steps = macro_steps(&[
            MacroStep::PadDown { button: PadButton::A },
            MacroStep::PadTap {
                button: PadButton::LsUp,
                hold_ms: 40,
            },
            MacroStep::PadUp { button: PadButton::A },
            // Not an output of the virtual controller: skipped.
            MacroStep::PadTap {
                button: PadButton::Touchpad,
                hold_ms: 40,
            },
        ]);
        assert_eq!(
            steps,
            [
                Step::PadDown(PadButton::A),
                Step::PadDown(PadButton::LsUp),
                Step::Delay(40),
                Step::PadUp(PadButton::LsUp),
                Step::PadUp(PadButton::A),
            ]
        );
        let mut cfg = Config::default();
        cfg.macros
            .push(Macro::new("Pad", vec![MacroStep::PadDown { button: PadButton::B }]));
        cfg.profiles[0].bindings.insert(
            "key:F1".into(),
            Action::Macro {
                name: "Pad".into(),
                mode: MacroMode::Once,
                count: 1,
                gap_ms: 0,
            },
        );
        assert!(compile(&cfg).profiles[0].layers.uses_vpad);
        // Mouse → stick alone needs it too, and other profiles follow Default.
        let mut cfg = Config::default();
        cfg.profiles[0].mouse_stick = Some(StickPointer::Right);
        let compiled = compile(&cfg);
        assert!(compiled.profiles.iter().all(|p| p.layers.uses_vpad));
        assert_eq!(compiled.profiles[1].layers.mouse_stick, StickPointer::Right);
        cfg.profiles[1].mouse_stick = Some(StickPointer::Off);
        assert!(!compile(&cfg).profiles[1].layers.uses_vpad);
    }
}
