//! The virtual Xbox 360 controller, the part every platform shares: what
//! it should report, and the state the input threads change.
//!
//! Inputs bound to the *Controller* action (and macro and turbo steps)
//! press its buttons, triggers and stick directions. With *Copy the real
//! controller* on, what a real controller does that the profile doesn't
//! remap goes to it as well, so a remapped controller works in games. Mouse
//! movement can lean one of its sticks.
//!
//! The platform part (`win_vpad.rs` with ViGEmBus, `linux_vpad.rs` with
//! uinput) owns the device on its own thread: everyone else changes the
//! shared `State` under a mutex and wakes it.

use super::pad::{bit, MouseStick, PadReading};
use super::{EngineEvent, VpadState};
use crate::config::{PadButton, StickPointer};

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// A real Xbox 360 controller's ids, which games know best.
pub const VENDOR: u16 = 0x045E;
pub const PRODUCT: u16 = 0x028E;

/// Unplug this long after the profile stops needing it (switching between
/// apps shouldn't plug it in and out every time).
pub const LINGER: Duration = Duration::from_secs(10);
/// Without the driver, look again this often while it's wanted.
pub const RETRY: Duration = Duration::from_secs(10);
/// How often the stick follows the mouse.
pub const MOUSE_TICK: Duration = Duration::from_millis(8);

/// What the controller reports: the layout of `XINPUT_GAMEPAD` (and of
/// ViGEmBus's `XUSB_REPORT`). Sticks: up and right are positive.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct XReport {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

/// XInput button flag of a controller output.
pub fn xbox_flag(b: PadButton) -> Option<u16> {
    Some(match b {
        PadButton::Up => 0x0001,
        PadButton::Down => 0x0002,
        PadButton::Left => 0x0004,
        PadButton::Right => 0x0008,
        PadButton::Start => 0x0010,
        PadButton::Back => 0x0020,
        PadButton::Ls => 0x0040,
        PadButton::Rs => 0x0080,
        PadButton::Lb => 0x0100,
        PadButton::Rb => 0x0200,
        PadButton::Guide => 0x0400,
        PadButton::A => 0x1000,
        PadButton::B => 0x2000,
        PadButton::X => 0x4000,
        PadButton::Y => 0x8000,
        _ => return None,
    })
}

/// What a real controller passes on, leaving out the inputs in `excluded`
/// (bits of `pad::bit`): those are remapped, so the game shouldn't see
/// them. A stick direction left out stops the stick going that way.
pub fn mirror_report(r: &PadReading, excluded: u64) -> XReport {
    let kept = |b: PadButton| excluded & bit(b) == 0;
    let mut out = XReport::default();
    for b in PadButton::OUTPUTS {
        if let Some(flag) = xbox_flag(b) {
            if r.buttons & bit(b) != 0 && kept(b) {
                out.buttons |= flag;
            }
        }
    }
    out.lt = if kept(PadButton::Lt) { r.lt } else { 0 };
    out.rt = if kept(PadButton::Rt) { r.rt } else { 0 };
    let axis = |v: i16, plus: PadButton, minus: PadButton| {
        let v = if kept(plus) { v } else { v.min(0) };
        if kept(minus) {
            v
        } else {
            v.max(0)
        }
    };
    out.lx = axis(r.lx, PadButton::LsRight, PadButton::LsLeft);
    out.ly = axis(r.ly, PadButton::LsUp, PadButton::LsDown);
    out.rx = axis(r.rx, PadButton::RsRight, PadButton::RsLeft);
    out.ry = axis(r.ry, PadButton::RsUp, PadButton::RsDown);
    out
}

/// The stick the mouse leans and where to, while it moves.
pub type MouseLean = Option<(StickPointer, (i16, i16))>;

/// The report: real controllers copied in, the mouse's stick over the
/// copied one while it moves, then what bindings hold down on top. A stick
/// a binding pushes ignores the others meanwhile.
pub fn compose(mirrors: &[(u32, XReport)], held: &[u16; PadButton::OUTPUTS.len()], mouse: MouseLean) -> XReport {
    let mut r = XReport::default();
    let bigger = |a: i16, b: i16| if b.unsigned_abs() > a.unsigned_abs() { b } else { a };
    for (_, m) in mirrors {
        r.buttons |= m.buttons;
        r.lt = r.lt.max(m.lt);
        r.rt = r.rt.max(m.rt);
        r.lx = bigger(r.lx, m.lx);
        r.ly = bigger(r.ly, m.ly);
        r.rx = bigger(r.rx, m.rx);
        r.ry = bigger(r.ry, m.ry);
    }
    match mouse {
        Some((StickPointer::Left, xy)) if xy != (0, 0) => (r.lx, r.ly) = xy,
        Some((StickPointer::Right, xy)) if xy != (0, 0) => (r.rx, r.ry) = xy,
        _ => {}
    }
    let on = |b: PadButton| {
        PadButton::OUTPUTS
            .iter()
            .position(|o| *o == b)
            .is_some_and(|i| held[i] > 0)
    };
    for b in PadButton::OUTPUTS {
        if let Some(flag) = xbox_flag(b) {
            if on(b) {
                r.buttons |= flag;
            }
        }
    }
    if on(PadButton::Lt) {
        r.lt = 255;
    }
    if on(PadButton::Rt) {
        r.rt = 255;
    }
    let stick = |up, down, left, right| -> Option<(i16, i16)> {
        let x = on(right) as i32 - on(left) as i32;
        let y = on(up) as i32 - on(down) as i32;
        if !(on(up) || on(down) || on(left) || on(right)) {
            return None;
        }
        // Diagonals stay on the stick's circle.
        let full = if x != 0 && y != 0 { 23170 } else { 32767 };
        Some(((x * full) as i16, (y * full) as i16))
    };
    if let Some((x, y)) = stick(
        PadButton::LsUp,
        PadButton::LsDown,
        PadButton::LsLeft,
        PadButton::LsRight,
    ) {
        (r.lx, r.ly) = (x, y);
    }
    if let Some((x, y)) = stick(
        PadButton::RsUp,
        PadButton::RsDown,
        PadButton::RsLeft,
        PadButton::RsRight,
    ) {
        (r.rx, r.ry) = (x, y);
    }
    r
}

/// How mouse movement leans a stick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseStickSettings {
    pub stick: StickPointer,
    /// 1-20.
    pub speed: u32,
    /// Percent.
    pub deadzone: u32,
}

#[derive(Default)]
pub struct State {
    /// The active profile sends controller output.
    pub wanted: bool,
    /// Presses per output (two bindings can hold the same button).
    pub held: [u16; PadButton::OUTPUTS.len()],
    /// Real controllers being copied: (source key, what they pass on).
    pub mirrors: Vec<(u32, XReport)>,
    /// Look for the driver again now (it was just installed).
    pub retry: bool,
    pub status: VpadState,
    /// Windows: XInput slot (0-3) of our controller, so it isn't read as a
    /// real one.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub slot: Option<u32>,
    /// Ours is plugged in (or being plugged in).
    pub plugged: bool,
    pub mouse: MouseStickSettings,
}

impl State {
    pub fn report(&self, mouse: MouseLean) -> XReport {
        compose(&self.mirrors, &self.held, mouse)
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static EVENTS: OnceLock<Arc<dyn Fn(EngineEvent) + Send + Sync>> = OnceLock::new();
/// Mouse movement not yet turned into a stick position. Atomics: it comes
/// a thousand times a second and needn't wake anyone (the thread ticks).
static MOUSE_DX: AtomicI32 = AtomicI32::new(0);
static MOUSE_DY: AtomicI32 = AtomicI32::new(0);
/// Plugged in now (read by the mouse hook, which can't wait for a lock).
static PLUGGED: AtomicBool = AtomicBool::new(false);

fn lock() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Change the state and wake the controller thread.
pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = lock();
    let r = f(guard.get_or_insert_with(State::default));
    WAKE.notify_all();
    r
}

/// Read the state without waking the thread.
pub fn peek<R>(f: impl FnOnce(&State) -> R) -> R {
    let mut guard = lock();
    f(guard.get_or_insert_with(State::default))
}

/// The controller thread: sleep up to `timeout`, or until `changed` says
/// the state is different from what it last acted on.
pub fn wait(timeout: Duration, changed: impl Fn(&State) -> bool) {
    let guard = lock();
    let _ = WAKE.wait_timeout_while(guard, timeout, |s| !changed(s.get_or_insert_with(State::default)));
}

pub fn set_events(on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>) {
    let _ = EVENTS.set(on_event);
}

/// Whether the active profile sends controller output.
pub fn set_wanted(on: bool) {
    with(|s| s.wanted = on);
}

/// A binding pressed or released an output.
pub fn press(b: PadButton, down: bool) {
    let Some(i) = PadButton::OUTPUTS.iter().position(|o| *o == b) else {
        return;
    };
    with(|s| {
        s.held[i] = if down {
            s.held[i].saturating_add(1)
        } else {
            s.held[i].saturating_sub(1)
        }
    });
}

/// What a real controller passes on now, or `None` when it's gone.
pub fn mirror(source: u32, report: Option<XReport>) {
    with(|s| {
        let i = s.mirrors.iter().position(|(k, _)| *k == source);
        match (i, report) {
            (Some(i), Some(r)) => s.mirrors[i].1 = r,
            (None, Some(r)) => s.mirrors.push((source, r)),
            (Some(i), None) => {
                s.mirrors.remove(i);
            }
            (None, None) => {}
        }
    });
}

/// Look for the driver again (after installing it).
pub fn retry() {
    with(|s| s.retry = true);
}

pub fn status() -> VpadState {
    peek(|s| s.status.clone())
}

/// Which stick mouse movement leans in the active profile, and how.
pub fn set_mouse_stick(settings: MouseStickSettings) {
    let changed = with(|s| std::mem::replace(&mut s.mouse, settings) != settings);
    if changed {
        take_motion();
    }
}

/// The mouse moved (counts, y down).
pub fn mouse_motion(dx: i32, dy: i32) {
    MOUSE_DX.fetch_add(dx, Ordering::Relaxed);
    MOUSE_DY.fetch_add(dy, Ordering::Relaxed);
}

fn take_motion() -> (i32, i32) {
    (MOUSE_DX.swap(0, Ordering::Relaxed), MOUSE_DY.swap(0, Ordering::Relaxed))
}

/// The controller thread's view of the mouse: where the stick leans now.
#[derive(Default)]
pub struct MouseFeed {
    stick: MouseStick,
    last: Option<Instant>,
}

impl MouseFeed {
    pub fn next(&mut self, settings: MouseStickSettings) -> MouseLean {
        let (dx, dy) = take_motion();
        if settings.stick == StickPointer::Off {
            self.stick.reset();
            self.last = None;
            return None;
        }
        let now = Instant::now();
        let dt = self.last.replace(now).map_or(0.0, |t| (now - t).as_secs_f32());
        let xy = self
            .stick
            .update(dx, dy, dt.min(0.1), settings.speed, settings.deadzone);
        Some((settings.stick, xy))
    }
}

/// Whether the controller is plugged in.
pub fn is_plugged() -> bool {
    PLUGGED.load(Ordering::Relaxed)
}

pub fn set_status(status: VpadState) {
    PLUGGED.store(matches!(status, VpadState::Plugged { .. }), Ordering::Relaxed);
    #[cfg(windows)]
    super::win_hidhide::vpad_changed();
    let changed = with(|s| {
        let changed = s.status != status;
        s.status = status.clone();
        changed
    });
    if changed {
        match &status {
            VpadState::Plugged { slot } => crate::log!(
                "virtual controller: plugged in{}",
                slot.map(|n| format!(" as controller {n}")).unwrap_or_default()
            ),
            VpadState::Unplugged => crate::log!("virtual controller: unplugged"),
            VpadState::NoDriver => crate::log!(
                "virtual controller: {}",
                if cfg!(windows) {
                    "the ViGEmBus driver isn't installed"
                } else {
                    "can't open /dev/uinput"
                }
            ),
            VpadState::Failed(e) => crate::log!("virtual controller: {e}"),
        }
        if let Some(f) = EVENTS.get() {
            f(EngineEvent::Vpad(status));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(list: &[PadButton]) -> [u16; PadButton::OUTPUTS.len()] {
        let mut h = [0; PadButton::OUTPUTS.len()];
        for b in list {
            h[PadButton::OUTPUTS.iter().position(|o| o == b).unwrap()] += 1;
        }
        h
    }

    #[test]
    fn report_has_the_xinput_layout() {
        assert_eq!(std::mem::size_of::<XReport>(), 12);
    }

    #[test]
    fn bindings_press_buttons_triggers_and_sticks() {
        let r = compose(&[], &held(&[PadButton::A, PadButton::Rt, PadButton::LsUp]), None);
        assert_eq!(r.buttons, 0x1000);
        assert_eq!((r.lt, r.rt), (0, 255));
        assert_eq!((r.lx, r.ly), (0, 32767));
        let r = compose(&[], &held(&[PadButton::RsLeft, PadButton::RsDown]), None);
        assert_eq!((r.rx, r.ry), (-23170, -23170));
        // Opposite directions cancel out.
        let r = compose(&[], &held(&[PadButton::LsLeft, PadButton::LsRight]), None);
        assert_eq!((r.lx, r.ly), (0, 0));
    }

    #[test]
    fn mirror_leaves_out_remapped_inputs() {
        let real = PadReading {
            buttons: bit(PadButton::A) | bit(PadButton::Lb) | bit(PadButton::Guide),
            lt: 200,
            lx: 20000,
            ly: -15000,
            ..Default::default()
        };
        let excluded = bit(PadButton::Lb) | bit(PadButton::Lt) | bit(PadButton::LsRight) | bit(PadButton::Guide);
        let m = mirror_report(&real, excluded);
        assert_eq!(m.buttons, 0x1000);
        assert_eq!(m.lt, 0);
        assert_eq!((m.lx, m.ly), (0, -15000));
        // The binding's output goes on top; its stick overrides the real one.
        let r = compose(&[(1, m)], &held(&[PadButton::B, PadButton::LsLeft]), None);
        assert_eq!(r.buttons, 0x1000 | 0x2000);
        assert_eq!((r.lx, r.ly), (-32767, 0));
    }

    #[test]
    fn two_controllers_merge() {
        let a = XReport {
            buttons: 0x1000,
            lx: 100,
            ..Default::default()
        };
        let b = XReport {
            buttons: 0x2000,
            lx: -9000,
            rt: 50,
            ..Default::default()
        };
        let r = compose(&[(1, a), (2, b)], &held(&[]), None);
        assert_eq!(r.buttons, 0x3000);
        assert_eq!((r.lx, r.rt), (-9000, 50));
    }

    #[test]
    fn mouse_leans_its_stick_over_the_real_one() {
        let real = XReport {
            rx: 5000,
            ry: 6000,
            lx: 700,
            ..Default::default()
        };
        // Moving: the mouse's stick wins over the copied one.
        let r = compose(&[(1, real)], &held(&[]), Some((StickPointer::Right, (-20000, 100))));
        assert_eq!((r.rx, r.ry, r.lx), (-20000, 100, 700));
        // Still: the real stick shows through.
        let r = compose(&[(1, real)], &held(&[]), Some((StickPointer::Right, (0, 0))));
        assert_eq!((r.rx, r.ry), (5000, 6000));
        // A binding holding a direction still wins.
        let r = compose(&[], &held(&[PadButton::LsUp]), Some((StickPointer::Left, (9000, 0))));
        assert_eq!((r.lx, r.ly), (0, 32767));
    }
}
