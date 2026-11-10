//! What happens to one physical input event. Shared by every backend, so the
//! rules (held inputs, key repeat, shift layer, what goes to the worker) are
//! the same everywhere and covered by tests.
//!
//! This runs inside the OS input hook: no blocking, no waiting.

use super::player::{Cmd, Out};
use super::{Input, Layers, Target};
use crate::config::{Modifier, MouseButton};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Up,
    /// Wheel steps have no up/down; they are a single "tap".
    Tap,
}

/// Where the dispatcher sends its results.
pub trait Sink {
    /// Send input right away (batched until `flush`).
    fn out(&mut self, out: Out);
    fn flush(&mut self);
    /// Hand work to the playback thread.
    fn worker(&mut self, cmd: Cmd);
}

#[derive(Default)]
pub struct Dispatcher {
    /// Inputs that were remapped on press, so the release goes to the same
    /// target even if the profile changed while the button was held.
    held: HashMap<Input, Target>,
    /// Number of held inputs bound to the shift layer.
    shift: u32,
}

fn mod_vks(mods: u8) -> impl DoubleEndedIterator<Item = u16> {
    Modifier::ALL
        .into_iter()
        .filter(move |m| mods & m.bit() != 0)
        .map(|m| m.vk())
}

impl Dispatcher {
    /// Returns true if the original input should be swallowed.
    pub fn handle(
        &mut self,
        input: Input,
        phase: Phase,
        layers: Option<&Layers>,
        enabled: bool,
        sink: &mut impl Sink,
    ) -> bool {
        if phase == Phase::Up {
            // Always honour a remapped press, even if remapping was disabled
            // or the profile changed since.
            return match self.held.remove(&input) {
                Some(target) => {
                    self.release(input, &target, sink);
                    true
                }
                None => false,
            };
        }

        if phase == Phase::Down {
            if let Some(target) = self.held.get(&input) {
                // Keyboard auto-repeat of a remapped key: repeat the same
                // target (never a new lookup, so nothing gets stuck).
                match *target {
                    Target::Key { vk, .. } => {
                        sink.out(Out::Key { vk, down: true });
                        sink.flush();
                    }
                    Target::Mouse(b) if b.is_wheel() => {
                        sink.out(Out::Wheel(b));
                        sink.flush();
                    }
                    _ => {}
                }
                return true;
            }
        }

        if !enabled {
            return false;
        }
        let Some(target) = layers.and_then(|l| l.lookup(&input, self.shift > 0)).cloned() else {
            return false;
        };

        match phase {
            Phase::Down => {
                self.press(input, &target, sink);
                self.held.insert(input, target);
            }
            Phase::Tap => self.tap(input, &target, sink),
            Phase::Up => unreachable!(),
        }
        true
    }

    fn press(&mut self, input: Input, target: &Target, sink: &mut impl Sink) {
        match *target {
            Target::Key { vk, mods } => {
                for m in mod_vks(mods) {
                    sink.out(Out::Key { vk: m, down: true });
                }
                sink.out(Out::Key { vk, down: true });
                sink.flush();
            }
            Target::Mouse(b) if b.is_wheel() => {
                sink.out(Out::Wheel(b));
                sink.flush();
            }
            Target::Mouse(b) => {
                sink.out(Out::Button { button: b, down: true });
                sink.flush();
            }
            Target::Disabled => {}
            Target::Shift => self.shift += 1,
            Target::Play(_) | Target::Sensitivity(_) => sink.worker(Cmd::Press {
                trigger: input,
                target: target.clone(),
                tap: false,
            }),
        }
    }

    fn release(&mut self, input: Input, target: &Target, sink: &mut impl Sink) {
        match *target {
            Target::Key { vk, mods } => {
                sink.out(Out::Key { vk, down: false });
                for m in mod_vks(mods).rev() {
                    sink.out(Out::Key { vk: m, down: false });
                }
                sink.flush();
            }
            Target::Mouse(b) if b.is_wheel() => {}
            Target::Mouse(b) => {
                sink.out(Out::Button { button: b, down: false });
                sink.flush();
            }
            Target::Disabled => {}
            Target::Shift => self.shift = self.shift.saturating_sub(1),
            Target::Play(_) | Target::Sensitivity(_) => sink.worker(Cmd::Release(input)),
        }
    }

    fn tap(&mut self, input: Input, target: &Target, sink: &mut impl Sink) {
        match *target {
            // Through the worker so the key is held briefly: games that poll
            // key state miss an instant down+up.
            Target::Key { vk, mods } => sink.worker(Cmd::Tap { vk, mods }),
            Target::Mouse(b) if b.is_wheel() => {
                sink.out(Out::Wheel(b));
                sink.flush();
            }
            Target::Mouse(b) => {
                sink.out(Out::Button { button: b, down: true });
                sink.out(Out::Button { button: b, down: false });
                sink.flush();
            }
            Target::Play(_) => sink.worker(Cmd::Press {
                trigger: input,
                target: target.clone(),
                tap: true,
            }),
            Target::Disabled | Target::Shift | Target::Sensitivity(_) => {}
        }
    }

    /// Whether a remapped mouse button is being held (its release must still
    /// be seen).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn holds_mouse(&self) -> bool {
        self.held.keys().any(|i| matches!(i, Input::Mouse(_)))
    }

    #[cfg(test)]
    pub fn shifted(&self) -> bool {
        self.shift > 0
    }
}

/// Map a mouse button id back for callers that decode raw events.
pub fn wheel_for(delta: i32, horizontal: bool) -> MouseButton {
    match (horizontal, delta > 0) {
        (false, true) => MouseButton::WheelUp,
        (false, false) => MouseButton::WheelDown,
        (true, true) => MouseButton::WheelRight,
        (true, false) => MouseButton::WheelLeft,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::player::{Mode, Plan};
    use std::sync::Arc;

    #[derive(Default)]
    struct Rec {
        out: Vec<Out>,
        cmds: Vec<String>,
    }
    impl Sink for Rec {
        fn out(&mut self, out: Out) {
            self.out.push(out);
        }
        fn flush(&mut self) {}
        fn worker(&mut self, cmd: Cmd) {
            self.cmds.push(match cmd {
                Cmd::Press { tap, .. } => format!("press tap={tap}"),
                Cmd::Release(_) => "release".into(),
                Cmd::Tap { vk, .. } => format!("tap {vk}"),
                Cmd::StopAll => "stop".into(),
            });
        }
    }

    const IN: Input = Input::Key(0x41);

    fn layers(base: Vec<(Input, Target)>, shift: Vec<(Input, Target)>) -> Layers {
        Layers {
            base: base.into_iter().collect(),
            shift: shift.into_iter().collect(),
        }
    }

    #[test]
    fn combo_presses_mods_first_and_releases_them_last() {
        let l = layers(vec![(IN, Target::Key { vk: 0x43, mods: 1 | 2 })], vec![]);
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        assert!(d.handle(IN, Phase::Down, Some(&l), true, &mut s));
        assert!(d.handle(IN, Phase::Up, Some(&l), true, &mut s));
        let k = |vk, down| Out::Key { vk, down };
        assert_eq!(
            s.out,
            vec![
                k(0xA2, true),
                k(0xA0, true),
                k(0x43, true),
                k(0x43, false),
                k(0xA0, false),
                k(0xA2, false)
            ]
        );
    }

    #[test]
    fn auto_repeat_keeps_the_original_target_after_a_profile_switch() {
        let a = layers(vec![(IN, Target::Key { vk: 0x42, mods: 0 })], vec![]);
        let b = layers(vec![(IN, Target::Key { vk: 0x43, mods: 0 })], vec![]);
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        d.handle(IN, Phase::Down, Some(&a), true, &mut s);
        d.handle(IN, Phase::Down, Some(&b), true, &mut s); // repeat after switch
        d.handle(IN, Phase::Up, Some(&b), true, &mut s);
        assert_eq!(
            s.out,
            vec![
                Out::Key { vk: 0x42, down: true },
                Out::Key { vk: 0x42, down: true },
                Out::Key { vk: 0x42, down: false },
            ]
        );
    }

    #[test]
    fn release_is_honoured_after_disabling() {
        let l = layers(vec![(IN, Target::Key { vk: 0x42, mods: 0 })], vec![]);
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        d.handle(IN, Phase::Down, Some(&l), true, &mut s);
        assert!(d.handle(IN, Phase::Up, None, false, &mut s));
        assert_eq!(s.out.last(), Some(&Out::Key { vk: 0x42, down: false }));
        // Unmapped inputs pass through.
        assert!(!d.handle(Input::Key(1), Phase::Down, Some(&l), true, &mut s));
        assert!(!d.handle(IN, Phase::Down, Some(&l), false, &mut s));
    }

    #[test]
    fn shift_layer_applies_while_held() {
        let shift_key = Input::Mouse(MouseButton::Back);
        let l = layers(
            vec![(shift_key, Target::Shift), (IN, Target::Key { vk: 0x42, mods: 0 })],
            vec![(IN, Target::Key { vk: 0x43, mods: 0 })],
        );
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        assert!(d.handle(shift_key, Phase::Down, Some(&l), true, &mut s));
        assert!(d.shifted());
        d.handle(IN, Phase::Down, Some(&l), true, &mut s);
        d.handle(shift_key, Phase::Up, Some(&l), true, &mut s);
        assert!(!d.shifted());
        // Released after the shift key: still releases the shifted target.
        d.handle(IN, Phase::Up, Some(&l), true, &mut s);
        d.handle(IN, Phase::Down, Some(&l), true, &mut s);
        assert_eq!(
            s.out,
            vec![
                Out::Key { vk: 0x43, down: true },
                Out::Key { vk: 0x43, down: false },
                Out::Key { vk: 0x42, down: true },
            ]
        );
    }

    #[test]
    fn plans_go_to_the_worker_and_repeats_are_swallowed() {
        let plan = Target::Play(Arc::new(Plan::new(vec![], Mode::WhileHeld, 0, 0)));
        let wheel = Input::Mouse(MouseButton::WheelUp);
        let l = layers(
            vec![
                (IN, plan.clone()),
                (wheel, Target::Key { vk: 0x20, mods: 0 }),
                (Input::Key(2), plan),
            ],
            vec![],
        );
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        d.handle(IN, Phase::Down, Some(&l), true, &mut s);
        assert!(d.handle(IN, Phase::Down, Some(&l), true, &mut s)); // auto-repeat
        d.handle(IN, Phase::Up, Some(&l), true, &mut s);
        d.handle(wheel, Phase::Tap, Some(&l), true, &mut s);
        d.handle(Input::Key(2), Phase::Tap, Some(&l), true, &mut s);
        assert_eq!(s.cmds, vec!["press tap=false", "release", "tap 32", "press tap=true"]);
        assert!(s.out.is_empty());
    }

    #[test]
    fn knows_when_a_remapped_mouse_button_is_held() {
        let back = Input::Mouse(MouseButton::Back);
        let l = layers(
            vec![(back, Target::Key { vk: 0x42, mods: 0 }), (IN, Target::Disabled)],
            vec![],
        );
        assert!(l.uses_mouse());
        assert!(!layers(vec![(IN, Target::Disabled)], vec![]).uses_mouse());
        let mut d = Dispatcher::default();
        let mut s = Rec::default();
        d.handle(IN, Phase::Down, Some(&l), true, &mut s);
        assert!(!d.holds_mouse());
        d.handle(back, Phase::Down, Some(&l), true, &mut s);
        assert!(d.holds_mouse());
        d.handle(back, Phase::Up, None, true, &mut s);
        assert!(!d.holds_mouse());
    }

    #[test]
    fn wheel_directions() {
        assert_eq!(wheel_for(120, false), MouseButton::WheelUp);
        assert_eq!(wheel_for(-120, false), MouseButton::WheelDown);
        assert_eq!(wheel_for(120, true), MouseButton::WheelRight);
        assert_eq!(wheel_for(-120, true), MouseButton::WheelLeft);
    }
}
