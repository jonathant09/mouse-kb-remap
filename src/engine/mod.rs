//! The remapping engine: watches the focused window, picks the matching
//! profile and rewrites input.
//!
//! * `mod.rs` (this file): platform-independent types, `compile()` and the
//!   `Engine` facade the UI talks to.
//! * `dispatch.rs`: what happens to one input event (shared by all backends).
//! * `player.rs`: timed playback of macros, turbo and text on a worker thread.
//! * `windows.rs` / `linux.rs`: OS hooks and input injection.
//! * `fallback.rs`: no-op engine so the UI runs elsewhere.

use crate::config::{Action, Config, MacroMode, MacroStep, Modifier, MouseButton, Output};
use std::collections::HashMap;
use std::sync::Arc;

pub mod dispatch;
pub mod player;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(windows, target_os = "linux")))]
mod fallback;
#[cfg(not(any(windows, target_os = "linux")))]
use fallback as imp;

pub use player::{LaunchSpec, Plan, Step};

/// Sent to the UI whenever the foreground window changes.
#[derive(Clone, Debug, Default)]
pub struct FocusInfo {
    /// Executable name of the focused window, e.g. `GenshinImpact.exe`.
    pub exe: String,
    /// True when the focused window belongs to this app (remaps are paused).
    pub is_self: bool,
    /// True when the focused process runs as admin but we don't, so Windows
    /// will block our hooks from seeing/sending input to it.
    pub elevation_blocked: bool,
}

/// Events from the engine thread to the UI.
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Focus(FocusInfo),
    /// A key pressed while capture mode was on: virtual key + modifier bits
    /// (see `Modifier::bit`) that were held with it.
    Captured {
        vk: u16,
        mods: u8,
    },
}

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
    /// Hold to use the shift layer.
    Shift,
    /// Timed playback on the worker thread (macro, turbo, text, launch).
    Play(Arc<Plan>),
    /// Hold to set the pointer speed (1-20).
    Sensitivity(u32),
}

pub type Bindings = HashMap<Input, Target>;

/// The two binding layers of a profile, Default already merged underneath.
#[derive(Default, Debug)]
pub struct Layers {
    pub base: Bindings,
    pub shift: Bindings,
}

impl Layers {
    /// Whether any mouse button or wheel input is remapped.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn uses_mouse(&self) -> bool {
        self.base
            .keys()
            .chain(self.shift.keys())
            .any(|i| matches!(i, Input::Mouse(_)))
    }

    pub fn lookup(&self, input: &Input, shifted: bool) -> Option<&Target> {
        if shifted {
            if let Some(t) = self.shift.get(input) {
                return Some(t);
            }
        }
        self.base.get(input)
    }
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct CompiledProfile {
    /// Lower-cased executable names.
    pub apps: Vec<String>,
    pub layers: Arc<Layers>,
}

pub fn parse_input(id: &str) -> Option<Input> {
    if let Some(k) = id.strip_prefix("key:") {
        crate::keys::find(k).map(|k| Input::Key(k.vk))
    } else if let Some(m) = id.strip_prefix("mouse:") {
        MouseButton::from_id(m).map(Input::Mouse)
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
        Action::Shift => Target::Shift,
        Action::Sensitivity { speed } => Target::Sensitivity((*speed).clamp(1, 20)),
        Action::Macro {
            name,
            mode,
            count,
            gap_ms,
        } => {
            let m = cfg.find_macro(name)?;
            Target::Play(Arc::new(Plan::new(
                macro_steps(&m.steps),
                match mode {
                    MacroMode::Once => player::Mode::Once,
                    MacroMode::Repeat => player::Mode::Repeat((*count).max(1)),
                    MacroMode::WhileHeld => player::Mode::WhileHeld,
                    MacroMode::Toggle => player::Mode::Toggle,
                },
                *gap_ms,
                *gap_ms,
            )))
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
            Target::Play(Arc::new(Plan::new(
                steps,
                if *toggle {
                    player::Mode::Toggle
                } else {
                    player::Mode::WhileHeld
                },
                0,
                delay_ms.saturating_sub(interval),
            )))
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
    })
}

/// Turn the user-facing config into lookup tables the input hook can use
/// without allocating.
pub fn compile(cfg: &Config) -> Vec<CompiledProfile> {
    let to_map = |bindings: &std::collections::BTreeMap<String, Action>| -> Bindings {
        bindings
            .iter()
            .filter_map(|(input, action)| Some((parse_input(input)?, parse_action(action, cfg)?)))
            .collect()
    };
    let (def_base, def_shift) = cfg
        .profiles
        .first()
        .map(|p| (to_map(&p.bindings), to_map(&p.shift_bindings)))
        .unwrap_or_default();
    let mut out = vec![CompiledProfile {
        apps: vec![],
        layers: Arc::new(Layers {
            base: def_base.clone(),
            shift: def_shift.clone(),
        }),
    }];
    for p in cfg.profiles.iter().skip(1) {
        let mut base = def_base.clone();
        base.extend(to_map(&p.bindings));
        let mut shift = def_shift.clone();
        shift.extend(to_map(&p.shift_bindings));
        out.push(CompiledProfile {
            apps: p.apps.iter().map(|a| a.to_ascii_lowercase()).collect(),
            layers: Arc::new(Layers { base, shift }),
        });
    }
    out
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

pub struct Engine {
    inner: imp::Engine,
}

impl Engine {
    /// Start the engine. `on_event` is called from the engine thread.
    pub fn start(cfg: &Config, on_event: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        Engine {
            inner: imp::Engine::start(compile(cfg), cfg.enabled, Box::new(on_event)),
        }
    }

    /// Push a new configuration to the running engine. Stops running macros.
    pub fn apply(&self, cfg: &Config) {
        self.inner.apply(compile(cfg), cfg.enabled);
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
    use crate::config::{Macro, Profile};

    fn key(k: &str) -> Action {
        Action::Key {
            key: k.into(),
            mods: vec![],
        }
    }

    #[test]
    fn profiles_inherit_default_per_layer() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert("key:F1".into(), key("A"));
        cfg.profiles[0].shift_bindings.insert("key:F2".into(), key("B"));
        let mut game = Profile::new("Game");
        game.apps.push("Game.EXE".into());
        game.bindings.insert("key:F1".into(), key("C"));
        cfg.profiles.push(game);

        let compiled = compile(&cfg);
        let game = compiled.last().unwrap();
        assert_eq!(game.apps, vec!["game.exe"]);
        let f1 = Input::Key(0x70);
        let f2 = Input::Key(0x71);
        assert!(matches!(
            game.layers.lookup(&f1, false),
            Some(Target::Key { vk: 0x43, .. })
        ));
        // Shifted lookups fall back to the base layer...
        assert!(matches!(
            game.layers.lookup(&f1, true),
            Some(Target::Key { vk: 0x43, .. })
        ));
        // ...and see Default's shift layer.
        assert!(matches!(
            game.layers.lookup(&f2, true),
            Some(Target::Key { vk: 0x42, .. })
        ));
        assert!(game.layers.lookup(&f2, false).is_none());
    }

    #[test]
    fn unknown_keys_and_missing_macros_are_skipped() {
        let mut cfg = Config::default();
        cfg.profiles[0].bindings.insert("key:Nope".into(), key("A"));
        cfg.profiles[0].bindings.insert("key:F1".into(), key("Nope"));
        cfg.profiles[0].bindings.insert(
            "key:F2".into(),
            Action::Macro {
                name: "missing".into(),
                mode: MacroMode::Once,
                count: 1,
                gap_ms: 0,
            },
        );
        assert!(compile(&cfg)[0].layers.base.is_empty());
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
        let Some(Target::Play(plan)) = compiled[0].layers.base.get(&Input::Mouse(MouseButton::Back)) else {
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
    fn macro_steps_expand_taps_and_text() {
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
            ]
        );
        let _ = Macro {
            name: String::new(),
            steps: vec![],
        };
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
}
