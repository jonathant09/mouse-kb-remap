//! The remapping engine: watches the focused window, picks the matching
//! profile and rewrites input. The platform layer lives in `windows.rs`;
//! other platforms get a no-op fallback so the UI can still be developed.

use crate::config::{Action, Config, MouseButton};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(not(windows))]
mod fallback;
#[cfg(not(windows))]
use fallback as imp;

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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Input {
    /// Windows virtual-key code.
    Key(u16),
    Mouse(MouseButton),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Key(u16),
    Mouse(MouseButton),
    Disabled,
}

pub type Bindings = HashMap<Input, Target>;

#[cfg_attr(not(windows), allow(dead_code))]
pub struct CompiledProfile {
    /// Lower-cased executable names.
    pub apps: Vec<String>,
    /// Bindings with the Default profile already merged underneath.
    pub bindings: Arc<Bindings>,
}

fn parse_input(id: &str) -> Option<Input> {
    if let Some(k) = id.strip_prefix("key:") {
        crate::keys::find(k).map(|k| Input::Key(k.vk))
    } else if let Some(m) = id.strip_prefix("mouse:") {
        MouseButton::from_id(m).map(Input::Mouse)
    } else {
        None
    }
}

fn parse_action(action: &Action) -> Option<Target> {
    match action {
        Action::Key { key } => crate::keys::find(key).map(|k| Target::Key(k.vk)),
        Action::Mouse { button } => Some(Target::Mouse(*button)),
        Action::Disabled => Some(Target::Disabled),
    }
}

/// Turn the user-facing config into lookup tables the input hook can use
/// without allocating.
pub fn compile(cfg: &Config) -> Vec<CompiledProfile> {
    let to_map = |p: &crate::config::Profile| -> Bindings {
        p.bindings
            .iter()
            .filter_map(|(input, action)| Some((parse_input(input)?, parse_action(action)?)))
            .collect()
    };
    let default = cfg.profiles.first().map(to_map).unwrap_or_default();
    let mut out = vec![CompiledProfile {
        apps: vec![],
        bindings: Arc::new(default.clone()),
    }];
    for p in cfg.profiles.iter().skip(1) {
        let mut merged = default.clone();
        merged.extend(to_map(p));
        out.push(CompiledProfile {
            apps: p.apps.iter().map(|a| a.to_ascii_lowercase()).collect(),
            bindings: Arc::new(merged),
        });
    }
    out
}

pub struct Engine {
    inner: imp::Engine,
}

impl Engine {
    /// Start the engine. `on_focus` is called from the engine thread.
    pub fn start(cfg: &Config, on_focus: impl Fn(FocusInfo) + Send + Sync + 'static) -> Self {
        Engine {
            inner: imp::Engine::start(compile(cfg), cfg.enabled, Box::new(on_focus)),
        }
    }

    /// Push a new configuration to the running engine.
    pub fn apply(&self, cfg: &Config) {
        self.inner.apply(compile(cfg), cfg.enabled);
    }

    /// Whether this platform has a real input backend yet.
    pub fn supported(&self) -> bool {
        imp::SUPPORTED
    }

    /// Whether this process runs with admin rights.
    pub fn elevated(&self) -> bool {
        self.inner.elevated()
    }
}
