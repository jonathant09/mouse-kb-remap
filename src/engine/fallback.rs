//! No-op engine for platforms without an input backend (macOS for now).
//! Lets the UI run everywhere during development.

use super::{CompiledProfile, EngineEvent, RecEvent};

pub const SUPPORTED: bool = false;

pub struct Engine;

impl Engine {
    pub fn start(
        _profiles: Vec<CompiledProfile>,
        _enabled: bool,
        _on_event: Box<dyn Fn(EngineEvent) + Send + Sync>,
    ) -> Self {
        Engine
    }

    pub fn apply(&self, _profiles: Vec<CompiledProfile>, _enabled: bool) {}

    pub fn elevated(&self) -> bool {
        false
    }

    pub fn start_capture(&self) -> bool {
        false
    }

    pub fn cancel_capture(&self) {}

    pub fn start_recording(&self) -> bool {
        false
    }

    pub fn stop_recording(&self) -> Vec<(std::time::Duration, RecEvent)> {
        Vec::new()
    }

    pub fn shutdown(&self) {}
}
