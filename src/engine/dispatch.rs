//! What happens to one physical input event. Shared by every backend, so the
//! rules (held inputs, key repeat, layers, combos, tap / hold, app hotkeys,
//! what goes to the worker) are the same everywhere and covered by tests.
//!
//! This runs inside the OS input hook: no blocking, no waiting. Tap / hold
//! decisions that need time are finished by `tick`, which the backend calls
//! at `deadline`.

use super::player::{Cmd, Out};
use super::{AppCommand, DeviceIndex, HotkeyBinding, Input, Layers, TapHold, Target};
use crate::config::{layer_name, Modifier, MouseButton, LAYER_COUNT};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    /// An app hotkey was pressed.
    fn command(&mut self, cmd: AppCommand);
    /// Something worth an on-screen notification ("Layer 2 on").
    fn notice(&mut self, text: String);
    /// Where the pointer is now, in pixels (only differences matter), for
    /// gestures. `None` if unknown.
    fn pointer(&mut self) -> Option<(i32, i32)>;
    /// The Gyro action turned the controller gyro mouse on or off.
    fn gyro(&mut self, _on: bool) {}
}

/// What the dispatcher needs to know about the world for one event.
pub struct Ctx<'a> {
    pub layers: Option<&'a Layers>,
    pub enabled: bool,
    pub hotkeys: &'a [HotkeyBinding],
    pub now: Instant,
    /// The named device the event came from, if known.
    pub device: Option<DeviceIndex>,
}

/// Unassigned virtual key sent while Alt or Win is held on its own, so
/// releasing it doesn't open a menu (the trick AutoHotkey uses).
const MASK_KEY: u16 = 0xE8;

#[derive(Debug, PartialEq)]
enum Wait {
    /// Pressed: a release before `hold_ms` is a tap.
    Hold,
    /// Tapped once: a second press before `double_ms` is a double tap.
    Double,
}

struct Pending {
    input: Input,
    spec: Arc<TapHold>,
    since: Instant,
    wait: Wait,
}

#[derive(Default)]
pub struct Dispatcher {
    /// Inputs that were remapped on press, so the release goes to the same
    /// target even if the profile changed while the button was held.
    held: HashMap<Input, Target>,
    /// Held inputs per layer that hold that layer on.
    layer_hold: [u32; LAYER_COUNT],
    /// Layers toggled on (bit n = layer n).
    layer_lock: u8,
    /// Physical keys and buttons currently down.
    down: Vec<Input>,
    /// A tap / hold input still being decided.
    pending: Option<Pending>,
    /// Inputs whose repeats and release are swallowed (hotkeys, taps that
    /// were sent early).
    swallow: Vec<Input>,
    /// Modifiers released while a combo sends its key, to press again after.
    lifted: Vec<(Input, Vec<u16>)>,
    /// Gesture inputs held, with the pointer position at the press.
    gestures: Vec<(Input, Option<(i32, i32)>)>,
    /// Inputs holding the gyro mouse on, and whether it is toggled on.
    gyro_hold: u32,
    gyro_lock: bool,
}

fn mod_vks(mods: u8) -> impl DoubleEndedIterator<Item = u16> {
    Modifier::ALL
        .into_iter()
        .filter(move |m| mods & m.bit() != 0)
        .map(|m| m.vk())
}

impl Dispatcher {
    /// Returns true if the original input should be swallowed.
    pub fn handle(&mut self, input: Input, phase: Phase, ctx: &Ctx, sink: &mut impl Sink) -> bool {
        let repeat = phase == Phase::Down && self.down.contains(&input);
        match phase {
            Phase::Down if !repeat => self.down.push(input),
            Phase::Up => self.down.retain(|i| *i != input),
            _ => {}
        }

        if let Some(i) = self.swallow.iter().position(|i| *i == input) {
            if phase == Phase::Up {
                self.swallow.remove(i);
            }
            if phase != Phase::Tap {
                return true;
            }
        }

        // App hotkeys work even while remapping is paused.
        if phase == Phase::Down && !repeat {
            if let Input::Key(vk) = input {
                let mods = self.mods_held(Some(input));
                if let Some(h) = ctx.hotkeys.iter().find(|h| h.vk == vk && h.mods == mods) {
                    self.interrupt_pending(sink);
                    sink.command(h.command);
                    self.swallow.push(input);
                    return true;
                }
            }
        }

        // A tap / hold input waiting to be decided.
        let mut reinject = false;
        if let Some(p) = &self.pending {
            if p.input == input {
                return self.pending_event(input, phase, ctx.now, sink);
            }
            if phase == Phase::Tap || (phase == Phase::Down && !repeat) {
                // Another input while it's held: that makes it a hold.
                self.interrupt_pending(sink);
                reinject = true;
            }
        }

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

        if self.dispatch_new(input, phase, ctx, sink) {
            return true;
        }
        if reinject {
            // What the tap / hold input produced was sent from here, after
            // this event had already been seen; send this one again so the
            // app gets them in the right order.
            let again = match (input, phase) {
                (Input::Key(vk), _) => Out::Key { vk, down: true },
                (Input::Mouse(b), Phase::Tap) => Out::Wheel(b),
                (Input::Mouse(b), _) => Out::Button { button: b, down: true },
                // Controller input isn't held back, so there is nothing to resend.
                (Input::Pad(_), _) => return false,
            };
            sink.out(again);
            sink.flush();
            return true;
        }
        false
    }

    /// A press (or wheel step) that isn't held yet: look it up and act.
    fn dispatch_new(&mut self, input: Input, phase: Phase, ctx: &Ctx, sink: &mut impl Sink) -> bool {
        if !ctx.enabled {
            return false;
        }
        let Some(layers) = ctx.layers else { return false };
        let mods = self.mods_held(Some(input));
        let down = &self.down;
        let Some((target, combo)) = layers.lookup(&input, ctx.device, self.active_layers(), mods, |i| down.contains(i))
        else {
            return false;
        };
        let target = target.clone();
        match phase {
            Phase::Down => {
                if let Target::TapHold(spec) = target {
                    self.pending = Some(Pending {
                        input,
                        spec,
                        since: ctx.now,
                        wait: Wait::Hold,
                    });
                    return true;
                }
                if combo {
                    self.lift_mods(input, &target, sink);
                }
                self.press(input, &target, sink);
                self.held.insert(input, target);
            }
            Phase::Tap => {
                let target = match &target {
                    Target::TapHold(spec) => spec.tap.clone(),
                    Target::Gesture(spec) => spec.tap.clone(),
                    t => Some(t.clone()),
                };
                if let Some(t) = target {
                    let lifted = combo && self.lift_mods(input, &t, sink);
                    self.tap(input, &t, sink, lifted);
                    if lifted {
                        self.restore_mods(input, sink);
                    }
                }
            }
            Phase::Up => unreachable!(),
        }
        true
    }

    /// The next event of the pending tap / hold input itself.
    fn pending_event(&mut self, input: Input, phase: Phase, now: Instant, sink: &mut impl Sink) -> bool {
        let Some(p) = self.pending.take() else { return false };
        match (phase, &p.wait) {
            (Phase::Up, Wait::Hold) => {
                if p.spec.double.is_some() {
                    self.pending = Some(Pending {
                        since: now,
                        wait: Wait::Double,
                        ..p
                    });
                } else if let Some(t) = &p.spec.tap {
                    self.tap(input, t, sink, false);
                }
            }
            (Phase::Down, Wait::Double) => {
                if let Some(t) = p.spec.double.clone() {
                    self.press(input, &t, sink);
                    self.held.insert(input, t);
                }
            }
            // Auto-repeat while deciding, or a stray event.
            _ => self.pending = Some(p),
        }
        true
    }

    /// Decide a pending tap / hold now, because something else happened.
    fn interrupt_pending(&mut self, sink: &mut impl Sink) {
        let Some(p) = self.pending.take() else { return };
        match p.wait {
            Wait::Hold => match &p.spec.hold {
                Some(h) => {
                    self.press(p.input, h, sink);
                    self.held.insert(p.input, h.clone());
                }
                None => {
                    // Nothing to hold: it's a tap, and the release that is
                    // still to come does nothing.
                    if let Some(t) = &p.spec.tap {
                        self.tap(p.input, t, sink, true);
                    }
                    self.swallow.push(p.input);
                }
            },
            Wait::Double => {
                if let Some(t) = &p.spec.tap {
                    self.tap(p.input, t, sink, true);
                }
            }
        }
    }

    /// When the pending tap / hold must be decided if nothing happens.
    pub fn deadline(&self) -> Option<Instant> {
        let p = self.pending.as_ref()?;
        match p.wait {
            Wait::Hold => p.spec.hold.as_ref()?,
            Wait::Double => p.spec.double.as_ref()?,
        };
        let ms = match p.wait {
            Wait::Hold => p.spec.hold_ms,
            Wait::Double => p.spec.double_ms,
        };
        Some(p.since + Duration::from_millis(ms as u64))
    }

    /// Finish a tap / hold whose time is up (call at `deadline`).
    pub fn tick(&mut self, now: Instant, sink: &mut impl Sink) {
        if self.deadline().is_none_or(|d| now < d) {
            return;
        }
        let Some(p) = self.pending.take() else { return };
        match p.wait {
            Wait::Hold => {
                if let Some(h) = &p.spec.hold {
                    self.press(p.input, h, sink);
                    self.held.insert(p.input, h.clone());
                }
            }
            Wait::Double => {
                if let Some(t) = &p.spec.tap {
                    self.tap(p.input, t, sink, false);
                }
            }
        }
    }

    /// Bit n set for each layer n in use.
    fn active_layers(&self) -> u8 {
        (1..LAYER_COUNT).fold(0, |bits, l| {
            if self.layer_hold[l] > 0 || self.layer_lock & (1 << l) != 0 {
                bits | (1 << l)
            } else {
                bits
            }
        })
    }

    /// Modifier bits held: physical modifier keys, plus modifiers that
    /// remapped held inputs are sending.
    fn mods_held(&self, except: Option<Input>) -> u8 {
        let mut bits = 0;
        for i in self.down.iter().filter(|i| Some(**i) != except) {
            match self.held.get(i) {
                Some(Target::Key { vk, mods }) => {
                    bits |= mods | crate::keys::modifier_of(*vk).map_or(0, |m| m.bit());
                }
                Some(_) => {}
                None => {
                    if let Input::Key(vk) = i {
                        bits |= crate::keys::modifier_of(*vk).map_or(0, |m| m.bit());
                    }
                }
            }
        }
        bits
    }

    /// A combo that sends a key: let go of the modifiers held for it, so
    /// `Ctrl + Back → F` arrives as F, not Ctrl + F. Returns whether any were.
    fn lift_mods(&mut self, input: Input, target: &Target, sink: &mut impl Sink) -> bool {
        let Target::Key { mods: keep, .. } = *target else {
            return false;
        };
        let vks: Vec<u16> = self
            .down
            .iter()
            .filter(|i| **i != input && !self.held.contains_key(i))
            .filter_map(|i| match i {
                Input::Key(vk) => crate::keys::modifier_of(*vk)
                    .filter(|m| keep & m.bit() == 0)
                    .map(|_| *vk),
                Input::Mouse(_) | Input::Pad(_) => None,
            })
            .collect();
        if vks.is_empty() {
            return false;
        }
        if vks.iter().any(|vk| matches!(vk, 0xA4 | 0xA5 | 0x12 | 0x5B | 0x5C)) {
            sink.out(Out::Key {
                vk: MASK_KEY,
                down: true,
            });
            sink.out(Out::Key {
                vk: MASK_KEY,
                down: false,
            });
        }
        for &vk in &vks {
            sink.out(Out::Key { vk, down: false });
        }
        self.lifted.push((input, vks));
        true
    }

    /// Press again the modifiers `lift_mods` let go of, if still held.
    fn restore_mods(&mut self, input: Input, sink: &mut impl Sink) {
        if let Some(i) = self.lifted.iter().position(|(t, _)| *t == input) {
            let (_, vks) = self.lifted.remove(i);
            for vk in vks {
                if self.down.contains(&Input::Key(vk)) {
                    sink.out(Out::Key { vk, down: true });
                }
            }
            sink.flush();
        }
    }

    fn toggle_layer(&mut self, layer: u8, sink: &mut impl Sink) {
        self.layer_lock ^= 1 << layer;
        let on = self.layer_lock & (1 << layer) != 0;
        let name = match layer {
            1 => "Shift layer".to_string(),
            l => layer_name(l as usize).to_string(),
        };
        sink.notice(format!("{name} {}", if on { "on" } else { "off" }));
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
            Target::Disabled | Target::TapHold(_) => {}
            Target::Gesture(_) => {
                let at = sink.pointer();
                self.gestures.retain(|(i, _)| *i != input);
                self.gestures.push((input, at));
            }
            Target::Layer { layer, toggle: false } => self.layer_hold[layer as usize] += 1,
            Target::Layer { layer, toggle: true } => self.toggle_layer(layer, sink),
            Target::Play(_) | Target::Sensitivity(_) => sink.worker(Cmd::Press {
                trigger: input,
                target: target.clone(),
                tap: false,
            }),
            Target::Gyro { toggle: false } => {
                self.gyro_hold += 1;
                sink.gyro(self.gyro_on());
            }
            Target::Gyro { toggle: true } => self.toggle_gyro(sink),
            Target::Pad(button) => {
                sink.out(Out::Pad { button, down: true });
                sink.flush();
            }
        }
    }

    fn gyro_on(&self) -> bool {
        self.gyro_hold > 0 || self.gyro_lock
    }

    fn toggle_gyro(&mut self, sink: &mut impl Sink) {
        self.gyro_lock = !self.gyro_lock;
        sink.gyro(self.gyro_on());
        sink.notice(format!("Gyro mouse {}", if self.gyro_lock { "on" } else { "off" }));
    }

    fn release(&mut self, input: Input, target: &Target, sink: &mut impl Sink) {
        match *target {
            Target::Key { vk, mods } => {
                sink.out(Out::Key { vk, down: false });
                for m in mod_vks(mods).rev() {
                    sink.out(Out::Key { vk: m, down: false });
                }
                sink.flush();
                self.restore_mods(input, sink);
            }
            Target::Mouse(b) if b.is_wheel() => {}
            Target::Mouse(b) => {
                sink.out(Out::Button { button: b, down: false });
                sink.flush();
            }
            Target::Disabled
            | Target::TapHold(_)
            | Target::Layer { toggle: true, .. }
            | Target::Gyro { toggle: true } => {}
            Target::Layer { layer, toggle: false } => {
                let n = &mut self.layer_hold[layer as usize];
                *n = n.saturating_sub(1);
            }
            Target::Gyro { toggle: false } => {
                self.gyro_hold = self.gyro_hold.saturating_sub(1);
                sink.gyro(self.gyro_on());
            }
            Target::Pad(button) => {
                sink.out(Out::Pad { button, down: false });
                sink.flush();
            }
            Target::Play(_) | Target::Sensitivity(_) => sink.worker(Cmd::Release(input)),
            Target::Gesture(ref spec) => {
                let start = self
                    .gestures
                    .iter()
                    .position(|(i, _)| *i == input)
                    .and_then(|i| self.gestures.remove(i).1);
                let (dx, dy) = match (start, sink.pointer()) {
                    (Some((x0, y0)), Some((x1, y1))) => (x1 - x0, y1 - y0),
                    _ => (0, 0),
                };
                if let Some(t) = spec.pick(dx, dy).cloned() {
                    self.tap(input, &t, sink, false);
                }
            }
        }
    }

    /// A press with no release to come: wheel steps, tap / hold taps.
    /// `immediate` sends a key down and up right away instead of holding it
    /// briefly on the worker, to keep the order with what follows.
    fn tap(&mut self, input: Input, target: &Target, sink: &mut impl Sink, immediate: bool) {
        match *target {
            Target::Key { vk, mods } if immediate => {
                for m in mod_vks(mods) {
                    sink.out(Out::Key { vk: m, down: true });
                }
                sink.out(Out::Key { vk, down: true });
                sink.out(Out::Key { vk, down: false });
                for m in mod_vks(mods).rev() {
                    sink.out(Out::Key { vk: m, down: false });
                }
                sink.flush();
            }
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
            Target::Layer { layer, toggle: true } => self.toggle_layer(layer, sink),
            Target::Gesture(ref spec) => {
                if let Some(t) = spec.tap.clone() {
                    self.tap(input, &t, sink, immediate);
                }
            }
            Target::Pad(b) => sink.worker(Cmd::PadTap(b)),
            Target::Gyro { toggle: true } => self.toggle_gyro(sink),
            Target::Disabled
            | Target::Layer { .. }
            | Target::Sensitivity(_)
            | Target::TapHold(_)
            | Target::Gyro { toggle: false } => {}
        }
    }

    /// Turn toggled layers off (the profile changed).
    pub fn reset_layers(&mut self) {
        self.layer_lock = 0;
    }

    /// Forget pass-through keys that are no longer down although their
    /// release was never seen (it went to a window our hooks can't see), so
    /// they don't count as held for combos and hotkeys. Remapped keys are
    /// kept: the OS never saw them go down, so it can't say.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn prune(&mut self, still_down: impl Fn(&Input) -> bool) {
        let ours = |i: &Input| {
            self.held.contains_key(i)
                || self.swallow.contains(i)
                || self.pending.as_ref().is_some_and(|p| p.input == *i)
        };
        let keep: Vec<Input> = self
            .down
            .iter()
            .filter(|i| matches!(i, Input::Mouse(_) | Input::Pad(_)) || ours(i) || still_down(i))
            .copied()
            .collect();
        self.down = keep;
    }

    /// The mouse hook was removed: its releases won't be seen.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn forget_mouse(&mut self) {
        self.down
            .retain(|i| !matches!(i, Input::Mouse(_)) || self.held.contains_key(i));
    }

    /// Whether a remapped mouse button is being held (its release must still
    /// be seen).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn holds_mouse(&self) -> bool {
        self.held.keys().any(|i| matches!(i, Input::Mouse(_)))
            || self
                .pending
                .as_ref()
                .is_some_and(|p| matches!(p.input, Input::Mouse(_)))
    }

    #[cfg(test)]
    pub fn layers_on(&self) -> u8 {
        self.active_layers()
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
    use crate::engine::ComboBinding;

    #[derive(Default)]
    struct Rec {
        out: Vec<Out>,
        cmds: Vec<String>,
        events: Vec<String>,
        pointer: (i32, i32),
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
                Cmd::PadTap(b) => format!("pad tap {}", b.id()),
            });
        }
        fn command(&mut self, cmd: AppCommand) {
            self.events.push(format!("{cmd:?}"));
        }
        fn notice(&mut self, text: String) {
            self.events.push(text);
        }
        fn pointer(&mut self) -> Option<(i32, i32)> {
            Some(self.pointer)
        }
        fn gyro(&mut self, on: bool) {
            self.events.push(format!("gyro {on}"));
        }
    }

    const IN: Input = Input::Key(0x41);
    const CTRL: Input = Input::Key(0xA2);

    fn k(vk: u16, down: bool) -> Out {
        Out::Key { vk, down }
    }

    fn key(vk: u16) -> Target {
        Target::Key { vk, mods: 0 }
    }

    fn layers(base: Vec<(Input, Target)>, shift: Vec<(Input, Target)>) -> Layers {
        let mut l = Layers::default();
        l.maps[0] = base.into_iter().collect();
        l.maps[1] = shift.into_iter().collect();
        l
    }

    /// Drives a dispatcher with one fixed layer set and a clock.
    struct T {
        d: Dispatcher,
        s: Rec,
        l: Layers,
        hotkeys: Vec<HotkeyBinding>,
        start: Instant,
        ms: u64,
        enabled: bool,
        device: Option<DeviceIndex>,
    }

    impl T {
        fn new(l: Layers) -> Self {
            T {
                d: Dispatcher::default(),
                s: Rec::default(),
                l,
                hotkeys: vec![],
                start: Instant::now(),
                ms: 0,
                enabled: true,
                device: None,
            }
        }
        fn now(&self) -> Instant {
            self.start + Duration::from_millis(self.ms)
        }
        fn ev(&mut self, input: Input, phase: Phase) -> bool {
            let ctx = Ctx {
                layers: Some(&self.l),
                enabled: self.enabled,
                hotkeys: &self.hotkeys,
                now: self.now(),
                device: self.device,
            };
            self.d.handle(input, phase, &ctx, &mut self.s)
        }
        fn down(&mut self, input: Input) -> bool {
            self.ev(input, Phase::Down)
        }
        fn up(&mut self, input: Input) -> bool {
            self.ev(input, Phase::Up)
        }
        /// Let time pass, running the tick like a backend would.
        fn wait(&mut self, ms: u64) {
            self.ms += ms;
            let now = self.now();
            self.d.tick(now, &mut self.s);
        }
    }

    fn gesture(distance: u32) -> Target {
        Target::Gesture(Arc::new(crate::engine::Gesture {
            tap: Some(Target::Mouse(MouseButton::Right)),
            swipes: [Some(key(0x70)), Some(key(0x71)), Some(key(0x72)), None],
            distance,
        }))
    }

    #[test]
    fn gestures_pick_by_direction_and_click_without_moving() {
        let right = Input::Mouse(MouseButton::Right);
        let mut t = T::new(layers(vec![(right, gesture(40))], vec![]));
        let swipe = |t: &mut T, dx: i32, dy: i32| {
            t.s.cmds.clear();
            t.s.out.clear();
            t.s.pointer = (500, 500);
            assert!(t.down(right));
            t.s.pointer = (500 + dx, 500 + dy);
            assert!(t.up(right));
        };
        swipe(&mut t, 3, -80);
        assert_eq!(t.s.cmds, vec!["tap 112"]); // up: F1, held briefly on the worker
        swipe(&mut t, -10, 60);
        assert_eq!(t.s.cmds, vec!["tap 113"]); // down
        swipe(&mut t, -90, 20);
        assert_eq!(t.s.cmds, vec!["tap 114"]); // left
        swipe(&mut t, 90, 0);
        assert!(t.s.cmds.is_empty() && t.s.out.is_empty()); // right: nothing set
        swipe(&mut t, 20, 20); // under the distance: a click
        assert_eq!(
            t.s.out,
            vec![
                Out::Button {
                    button: MouseButton::Right,
                    down: true
                },
                Out::Button {
                    button: MouseButton::Right,
                    down: false
                }
            ]
        );
        // Other input while the gesture button is held passes through.
        t.s.out.clear();
        t.down(right);
        assert!(!t.down(Input::Mouse(MouseButton::Left)));
        assert!(!t.up(Input::Mouse(MouseButton::Left)));
    }

    #[test]
    fn device_bindings_win_for_their_device_only() {
        let back = Input::Mouse(MouseButton::Back);
        let mut l = layers(vec![(back, key(0x41))], vec![]);
        l.device_maps[0].insert((1, back), key(0x42));
        assert!(l.device_specific(&back) && l.mouse_device_bindings());
        let mut t = T::new(l);
        let press = |t: &mut T, device| {
            t.device = device;
            t.s.out.clear();
            t.down(back);
            t.up(back);
            t.s.out.first().cloned()
        };
        assert_eq!(press(&mut t, None), Some(k(0x41, true)));
        assert_eq!(press(&mut t, Some(0)), Some(k(0x41, true)));
        assert_eq!(press(&mut t, Some(1)), Some(k(0x42, true)));
        // A device combo beats the device's plain binding.
        t.l.combos[0].push(ComboBinding {
            mods: Modifier::Ctrl.bit(),
            held: vec![],
            trigger: back,
            device: Some(1),
            target: key(0x43),
        });
        t.down(CTRL);
        assert_eq!(press(&mut t, Some(0)), Some(k(0x41, true)));
        assert_eq!(press(&mut t, Some(1)), Some(k(0xA2, false)));
        assert!(t.s.out.contains(&k(0x43, true)));
    }

    #[test]
    fn gyro_action_holds_or_toggles() {
        let rb = Input::Pad(crate::config::PadButton::Rb);
        let lb = Input::Pad(crate::config::PadButton::Lb);
        let mut t = T::new(layers(
            vec![
                (rb, Target::Gyro { toggle: false }),
                (lb, Target::Gyro { toggle: true }),
            ],
            vec![],
        ));
        t.down(rb);
        t.up(rb);
        assert_eq!(t.s.events, vec!["gyro true", "gyro false"]);
        t.s.events.clear();
        t.down(lb);
        t.up(lb);
        t.down(rb); // held while toggled on: stays on after release
        t.up(rb);
        t.down(lb);
        assert_eq!(
            t.s.events,
            vec![
                "gyro true",
                "Gyro mouse on",
                "gyro true",
                "gyro true",
                "gyro false",
                "Gyro mouse off"
            ]
        );
    }

    #[test]
    fn controller_buttons_work_like_keys() {
        let a = Input::Pad(crate::config::PadButton::A);
        let mut t = T::new(layers(vec![(a, key(0x20))], vec![]));
        t.down(a);
        t.up(a);
        assert_eq!(t.s.out, vec![k(0x20, true), k(0x20, false)]);
        // An unbound controller button never resends anything.
        let b = Input::Pad(crate::config::PadButton::B);
        assert!(!t.down(b));
        assert!(t.s.out.len() == 2);
    }

    #[test]
    fn combo_presses_mods_first_and_releases_them_last() {
        let mut t = T::new(layers(vec![(IN, Target::Key { vk: 0x43, mods: 1 | 2 })], vec![]));
        assert!(t.down(IN));
        assert!(t.up(IN));
        assert_eq!(
            t.s.out,
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
        let mut t = T::new(layers(vec![(IN, key(0x42))], vec![]));
        t.down(IN);
        t.l = layers(vec![(IN, key(0x43))], vec![]);
        t.down(IN); // repeat after switch
        t.up(IN);
        assert_eq!(t.s.out, vec![k(0x42, true), k(0x42, true), k(0x42, false)]);
    }

    #[test]
    fn release_is_honoured_after_disabling() {
        let mut t = T::new(layers(vec![(IN, key(0x42))], vec![]));
        t.down(IN);
        t.enabled = false;
        assert!(t.up(IN));
        assert_eq!(t.s.out.last(), Some(&k(0x42, false)));
        // Unmapped inputs pass through, and so does everything while paused.
        t.enabled = true;
        assert!(!t.down(Input::Key(1)));
        t.enabled = false;
        assert!(!t.down(IN));
    }

    #[test]
    fn shift_layer_applies_while_held() {
        let shift_key = Input::Mouse(MouseButton::Back);
        let mut t = T::new(layers(
            vec![
                (
                    shift_key,
                    Target::Layer {
                        layer: 1,
                        toggle: false,
                    },
                ),
                (IN, key(0x42)),
            ],
            vec![(IN, key(0x43))],
        ));
        assert!(t.down(shift_key));
        assert_eq!(t.d.layers_on(), 0b10);
        t.down(IN);
        t.up(shift_key);
        assert_eq!(t.d.layers_on(), 0);
        // Released after the shift key: still releases the shifted target.
        t.up(IN);
        t.down(IN);
        assert_eq!(t.s.out, vec![k(0x43, true), k(0x43, false), k(0x42, true)]);
    }

    #[test]
    fn toggled_layers_stay_on_and_higher_layers_win() {
        let l2 = Input::Key(0x71);
        let l3 = Input::Key(0x72);
        let mut l = layers(
            vec![
                (l2, Target::Layer { layer: 2, toggle: true }),
                (
                    l3,
                    Target::Layer {
                        layer: 3,
                        toggle: false,
                    },
                ),
                (IN, key(0x42)),
            ],
            vec![],
        );
        l.maps[2].insert(IN, key(0x43));
        l.maps[3].insert(IN, key(0x44));
        let mut t = T::new(l);
        t.down(l2);
        t.up(l2);
        assert_eq!(t.d.layers_on(), 0b100);
        t.down(IN);
        t.up(IN);
        t.down(l3);
        t.down(IN);
        t.up(IN);
        t.up(l3);
        t.down(l2); // toggle off
        t.up(l2);
        t.down(IN);
        assert_eq!(
            t.s.out,
            vec![
                k(0x43, true),
                k(0x43, false),
                k(0x44, true),
                k(0x44, false),
                k(0x42, true)
            ]
        );
        assert_eq!(t.s.events, vec!["Layer 2 on", "Layer 2 off"]);
        t.d.layer_lock = 0b100;
        t.d.reset_layers();
        assert_eq!(t.d.layers_on(), 0);
    }

    #[test]
    fn plans_go_to_the_worker_and_repeats_are_swallowed() {
        let plan = Target::Play(Arc::new(Plan::new(vec![], Mode::WhileHeld, 0, 0)));
        let wheel = Input::Mouse(MouseButton::WheelUp);
        let mut t = T::new(layers(
            vec![(IN, plan.clone()), (wheel, key(0x20)), (Input::Key(2), plan)],
            vec![],
        ));
        t.down(IN);
        assert!(t.down(IN)); // auto-repeat
        t.up(IN);
        t.ev(wheel, Phase::Tap);
        t.ev(Input::Key(2), Phase::Tap);
        assert_eq!(t.s.cmds, vec!["press tap=false", "release", "tap 32", "press tap=true"]);
        assert!(t.s.out.is_empty());
    }

    #[test]
    fn knows_when_a_remapped_mouse_button_is_held() {
        let back = Input::Mouse(MouseButton::Back);
        let l = layers(vec![(back, key(0x42)), (IN, Target::Disabled)], vec![]);
        assert!(l.uses_mouse());
        assert!(!layers(vec![(IN, Target::Disabled)], vec![]).uses_mouse());
        let mut t = T::new(l);
        t.down(IN);
        assert!(!t.d.holds_mouse());
        t.down(back);
        assert!(t.d.holds_mouse());
        t.l = Layers::default();
        t.up(back);
        assert!(!t.d.holds_mouse());
    }

    fn tap_hold(tap: Option<Target>, hold: Option<Target>, double: Option<Target>) -> Target {
        Target::TapHold(Arc::new(TapHold {
            tap,
            hold,
            double,
            hold_ms: 200,
            double_ms: 250,
        }))
    }

    const CAPS: Input = Input::Key(0x14);

    #[test]
    fn tap_hold_taps_on_a_quick_release() {
        let mut t = T::new(layers(
            vec![(CAPS, tap_hold(Some(key(0x1B)), Some(key(0xA2)), None))],
            vec![],
        ));
        assert!(t.down(CAPS));
        assert_eq!(t.d.deadline(), Some(t.now() + Duration::from_millis(200)));
        t.wait(50);
        assert!(t.down(CAPS)); // auto-repeat while deciding
        assert!(t.up(CAPS));
        assert_eq!(t.s.cmds, vec!["tap 27"]);
        assert!(t.s.out.is_empty());
        assert!(t.d.deadline().is_none());
    }

    #[test]
    fn tap_hold_holds_after_the_threshold() {
        let mut t = T::new(layers(
            vec![(CAPS, tap_hold(Some(key(0x1B)), Some(key(0xA2)), None))],
            vec![],
        ));
        t.down(CAPS);
        t.wait(199);
        assert!(t.s.out.is_empty());
        t.wait(1);
        assert_eq!(t.s.out, vec![k(0xA2, true)]);
        t.down(CAPS); // auto-repeat repeats the hold key
        t.up(CAPS);
        assert_eq!(t.s.out, vec![k(0xA2, true), k(0xA2, true), k(0xA2, false)]);
        assert!(t.s.cmds.is_empty());
    }

    #[test]
    fn another_key_while_deciding_makes_it_a_hold_in_order() {
        let c = Input::Key(0x43);
        let mut t = T::new(layers(
            vec![(CAPS, tap_hold(Some(key(0x1B)), Some(key(0xA2)), None))],
            vec![],
        ));
        t.down(CAPS);
        t.wait(30);
        // C isn't remapped, but is sent again after Ctrl so the order holds.
        assert!(t.down(c));
        assert!(!t.up(c));
        t.up(CAPS);
        assert_eq!(t.s.out, vec![k(0xA2, true), k(0x43, true), k(0xA2, false)]);
    }

    #[test]
    fn tap_hold_with_a_layer_hold_switches_layers_for_the_next_key() {
        let mut t = T::new(layers(
            vec![
                (
                    CAPS,
                    tap_hold(
                        Some(key(0x1B)),
                        Some(Target::Layer {
                            layer: 1,
                            toggle: false,
                        }),
                        None,
                    ),
                ),
                (IN, key(0x42)),
            ],
            vec![(IN, key(0x43))],
        ));
        t.down(CAPS);
        t.down(IN);
        t.up(IN);
        t.up(CAPS);
        t.down(IN);
        assert_eq!(t.s.out, vec![k(0x43, true), k(0x43, false), k(0x42, true)]);
    }

    #[test]
    fn double_tap_and_single_tap_after_the_window() {
        let mut t = T::new(layers(
            vec![(CAPS, tap_hold(Some(key(0x1B)), None, Some(key(0x70))))],
            vec![],
        ));
        // Double tap.
        t.down(CAPS);
        t.wait(40);
        t.up(CAPS);
        t.wait(100);
        t.down(CAPS);
        t.up(CAPS);
        assert_eq!(t.s.out, vec![k(0x70, true), k(0x70, false)]);
        assert!(t.s.cmds.is_empty());
        // Single tap: sent once the double-tap window has passed.
        t.down(CAPS);
        t.up(CAPS);
        t.wait(249);
        assert!(t.s.cmds.is_empty());
        t.wait(1);
        assert_eq!(t.s.cmds, vec!["tap 27"]);
        // No hold action: holding long is still a tap.
        t.down(CAPS);
        t.wait(1000);
        t.up(CAPS);
        t.wait(300);
        assert_eq!(t.s.cmds, vec!["tap 27", "tap 27"]);
    }

    #[test]
    fn tap_without_hold_is_sent_early_when_interrupted() {
        let mut t = T::new(layers(vec![(CAPS, tap_hold(Some(key(0x1B)), None, None))], vec![]));
        t.down(CAPS);
        t.down(IN);
        assert!(t.up(CAPS)); // its release does nothing more
        assert_eq!(t.s.out, vec![k(0x1B, true), k(0x1B, false), k(0x41, true)]);
    }

    #[test]
    fn hotkeys_fire_even_when_paused_and_are_swallowed() {
        let f12 = Input::Key(0x7B);
        let mut t = T::new(layers(vec![(f12, key(0x42))], vec![]));
        t.hotkeys = vec![HotkeyBinding {
            vk: 0x7B,
            mods: Modifier::Ctrl.bit(),
            command: AppCommand::TogglePause,
        }];
        t.enabled = false;
        assert!(!t.down(CTRL));
        assert!(t.down(f12));
        assert!(t.down(f12)); // repeat swallowed
        assert!(t.up(f12));
        assert!(!t.up(CTRL));
        assert_eq!(t.s.events, vec!["TogglePause"]);
        // Without Ctrl it's the normal binding.
        t.enabled = true;
        t.down(f12);
        assert_eq!(t.s.out, vec![k(0x42, true)]);
    }

    fn combo(mods: u8, held: Vec<Input>, trigger: Input, target: Target) -> ComboBinding {
        ComboBinding {
            mods,
            held,
            trigger,
            device: None,
            target,
        }
    }

    #[test]
    fn combos_need_their_keys_held_and_lift_modifiers_for_keys() {
        let back = Input::Mouse(MouseButton::Back);
        let mut l = layers(vec![(back, key(0x42))], vec![]);
        l.combos[0].push(combo(Modifier::Ctrl.bit(), vec![], back, key(0x46)));
        let mut t = T::new(l);
        t.down(back);
        t.up(back);
        assert!(!t.down(CTRL));
        t.down(back);
        t.up(back);
        assert_eq!(
            t.s.out,
            vec![
                k(0x42, true),
                k(0x42, false),
                // Ctrl let go, F, Ctrl pressed again.
                k(0xA2, false),
                k(0x46, true),
                k(0x46, false),
                k(0xA2, true),
            ]
        );
    }

    #[test]
    fn combos_with_held_buttons_and_alt_use_the_mask_key() {
        let left = Input::Mouse(MouseButton::Left);
        let right = Input::Mouse(MouseButton::Right);
        let alt = Input::Key(0xA4);
        let mut l = Layers::default();
        l.combos[0].push(combo(0, vec![right], left, key(0x47)));
        l.combos[0].push(combo(Modifier::Alt.bit(), vec![], IN, key(0x48)));
        let mut t = T::new(l);
        assert!(!t.down(left));
        t.up(left);
        t.down(right);
        assert!(t.down(left));
        t.up(left);
        t.up(right);
        t.down(alt);
        t.down(IN);
        t.up(IN);
        assert_eq!(
            t.s.out,
            vec![
                k(0x47, true),
                k(0x47, false),
                k(MASK_KEY, true),
                k(MASK_KEY, false),
                k(0xA4, false),
                k(0x48, true),
                k(0x48, false),
                k(0xA4, true),
            ]
        );
    }

    #[test]
    fn prune_forgets_pass_through_keys_whose_release_was_missed() {
        let mut t = T::new(layers(vec![(IN, key(0x42))], vec![]));
        t.down(IN);
        t.down(CTRL);
        t.d.prune(|_| false);
        // Ctrl no longer counts as held; the remapped key stays.
        assert_eq!(t.d.down, vec![IN]);
        assert_eq!(t.d.mods_held(None), 0);
    }

    #[test]
    fn controller_output_is_held_with_the_input() {
        use crate::config::PadButton;
        let pad = |b: PadButton, down: bool| Out::Pad { button: b, down };
        let lb = Input::Pad(PadButton::Lb);
        let mut t = T::new(layers(
            vec![
                (IN, Target::Pad(PadButton::A)),
                (lb, Target::Pad(PadButton::RsUp)),
                (
                    CAPS,
                    tap_hold(Some(Target::Pad(PadButton::B)), Some(Target::Pad(PadButton::Lt)), None),
                ),
            ],
            vec![],
        ));
        assert!(t.down(IN));
        assert!(t.down(lb));
        assert!(t.up(IN));
        assert!(t.up(lb));
        assert_eq!(
            t.s.out,
            vec![
                pad(PadButton::A, true),
                pad(PadButton::RsUp, true),
                pad(PadButton::A, false),
                pad(PadButton::RsUp, false)
            ]
        );
        t.s.out.clear();
        // A tap goes through the worker, which holds it for a few frames.
        t.down(CAPS);
        t.wait(50);
        t.up(CAPS);
        assert_eq!(t.s.cmds, vec!["pad tap b"]);
        // A hold presses until released.
        t.down(CAPS);
        t.wait(300);
        assert_eq!(t.s.out, vec![pad(PadButton::Lt, true)]);
        t.up(CAPS);
        assert_eq!(t.s.out.last(), Some(&pad(PadButton::Lt, false)));
    }

    #[test]
    fn wheel_directions() {
        assert_eq!(wheel_for(120, false), MouseButton::WheelUp);
        assert_eq!(wheel_for(-120, false), MouseButton::WheelDown);
        assert_eq!(wheel_for(120, true), MouseButton::WheelRight);
        assert_eq!(wheel_for(-120, true), MouseButton::WheelLeft);
    }
}
