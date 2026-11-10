//! No-op engine for platforms without an input backend yet (Linux/macOS).
//! Lets the UI run everywhere during development.

use super::{CompiledProfile, FocusInfo};

pub const SUPPORTED: bool = false;

pub struct Engine;

impl Engine {
    pub fn start(
        _profiles: Vec<CompiledProfile>,
        _enabled: bool,
        _on_focus: Box<dyn Fn(FocusInfo) + Send + Sync>,
    ) -> Self {
        Engine
    }

    pub fn apply(&self, _profiles: Vec<CompiledProfile>, _enabled: bool) {}

    pub fn elevated(&self) -> bool {
        false
    }
}
