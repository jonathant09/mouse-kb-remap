//! Timed playback for macros, turbo, text and launch actions.
//!
//! Input hooks must return immediately, so anything that waits runs here, on
//! a worker thread. The `Scheduler` is a plain state machine driven by
//! `command()` and `poll()` with explicit timestamps, so it is unit-tested
//! without threads or an OS; `run_worker` is the thread loop around it.

use super::Input;
use crate::config::MouseButton;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    pub path: String,
    pub args: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    KeyDown(u16),
    KeyUp(u16),
    ButtonDown(MouseButton),
    ButtonUp(MouseButton),
    Wheel(MouseButton),
    /// Type one character (Unicode input, independent of keyboard layout).
    Char(char),
    Delay(u32),
    Launch(Arc<LaunchSpec>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Once,
    Repeat(u32),
    WhileHeld,
    Toggle,
}

#[derive(Debug, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub mode: Mode,
    /// Pause between repetitions.
    pub gap_ms: u32,
    /// Pause after the first repetition instead of `gap_ms` (turbo's
    /// "repeat after" delay).
    pub first_gap_ms: u32,
    /// Total delay of one pass; loops with none get a 1 ms floor so they
    /// can't spin.
    has_delay: bool,
}

impl Plan {
    pub fn new(steps: Vec<Step>, mode: Mode, gap_ms: u32, first_gap_ms: u32) -> Self {
        let has_delay = steps.iter().any(|s| matches!(s, Step::Delay(ms) if *ms > 0));
        Plan {
            steps,
            mode,
            gap_ms,
            first_gap_ms,
            has_delay,
        }
    }
}

/// Requests from the input hook to the worker.
#[derive(Clone, Debug)]
pub enum Cmd {
    /// An input bound to a plan or sensitivity was pressed. `tap` is true for
    /// inputs without a release (wheel steps): loops then play one pass.
    Press {
        trigger: Input,
        target: super::Target,
        tap: bool,
    },
    Release(Input),
    /// Press and release a key with a short hold, so games that poll key
    /// state notice it (used for wheel → key).
    Tap {
        vk: u16,
        mods: u8,
    },
    /// Stop everything (profile switch, config change, pause, quit).
    StopAll,
}

/// Output produced by the worker; the platform turns these into OS input.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Key {
        vk: u16,
        down: bool,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    Wheel(MouseButton),
    Char(char),
    Launch(Arc<LaunchSpec>),
    /// Set the pointer speed, or restore the user's own speed with `None`.
    PointerSpeed(Option<u32>),
}

/// How long a wheel → key tap is held.
pub const TAP_HOLD_MS: u32 = 15;

/// If playback falls further behind than this (system busy), restart timing
/// from now instead of firing a burst to catch up.
const MAX_LAG: Duration = Duration::from_millis(25);

struct Player {
    trigger: Option<Input>,
    plan: Arc<Plan>,
    pos: usize,
    passes: u32,
    next: Instant,
    /// Play a single pass regardless of mode (tap triggers).
    single: bool,
    keys_down: Vec<u16>,
    buttons_down: Vec<MouseButton>,
}

impl Player {
    fn new(trigger: Option<Input>, plan: Arc<Plan>, now: Instant, single: bool) -> Self {
        Player {
            trigger,
            plan,
            pos: 0,
            passes: 0,
            next: now,
            single,
            keys_down: Vec::new(),
            buttons_down: Vec::new(),
        }
    }

    fn finished_after_pass(&self) -> bool {
        if self.single {
            return true;
        }
        match self.plan.mode {
            Mode::Once => true,
            Mode::Repeat(n) => self.passes >= n,
            Mode::WhileHeld | Mode::Toggle => false,
        }
    }

    /// Release whatever this player still holds down.
    fn release_all(&mut self, out: &mut Vec<Out>) {
        for vk in self.keys_down.drain(..).rev() {
            out.push(Out::Key { vk, down: false });
        }
        for button in self.buttons_down.drain(..).rev() {
            out.push(Out::Button { button, down: false });
        }
    }

    fn wait(&mut self, ms: u32, now: Instant) {
        let target = self.next + Duration::from_millis(ms as u64);
        self.next = if target + MAX_LAG < now {
            now + Duration::from_millis(ms as u64)
        } else {
            target
        };
    }

    /// Run due steps. Returns false when the player is done.
    fn run(&mut self, now: Instant, out: &mut Vec<Out>) -> bool {
        // Safety valve: never execute an unbounded number of steps in one go.
        let mut budget = 10_000;
        while self.next <= now && budget > 0 {
            budget -= 1;
            if self.pos >= self.plan.steps.len() {
                self.passes += 1;
                if self.finished_after_pass() {
                    self.release_all(out);
                    return false;
                }
                self.pos = 0;
                let mut gap = if self.passes == 1 {
                    self.plan.first_gap_ms
                } else {
                    self.plan.gap_ms
                };
                if gap == 0 && !self.plan.has_delay {
                    gap = 1;
                }
                if gap > 0 {
                    self.wait(gap, now);
                }
                continue;
            }
            let step = self.plan.steps[self.pos].clone();
            self.pos += 1;
            match step {
                Step::Delay(ms) => {
                    if ms > 0 {
                        self.wait(ms, now);
                    }
                }
                Step::KeyDown(vk) => {
                    if !self.keys_down.contains(&vk) {
                        self.keys_down.push(vk);
                    }
                    out.push(Out::Key { vk, down: true });
                }
                Step::KeyUp(vk) => {
                    self.keys_down.retain(|v| *v != vk);
                    out.push(Out::Key { vk, down: false });
                }
                Step::ButtonDown(button) => {
                    if !self.buttons_down.contains(&button) {
                        self.buttons_down.push(button);
                    }
                    out.push(Out::Button { button, down: true });
                }
                Step::ButtonUp(button) => {
                    self.buttons_down.retain(|b| *b != button);
                    out.push(Out::Button { button, down: false });
                }
                Step::Wheel(b) => out.push(Out::Wheel(b)),
                Step::Char(c) => out.push(Out::Char(c)),
                Step::Launch(spec) => out.push(Out::Launch(spec)),
            }
        }
        true
    }
}

#[derive(Default)]
pub struct Scheduler {
    players: Vec<Player>,
    /// Held sensitivity buttons, most recent last.
    sensitivity: Vec<(Input, u32)>,
}

impl Scheduler {
    pub fn command(&mut self, cmd: Cmd, now: Instant, out: &mut Vec<Out>) {
        match cmd {
            Cmd::Press { trigger, target, tap } => match target {
                super::Target::Play(plan) => {
                    if let Some(i) = self.players.iter().position(|p| p.trigger == Some(trigger)) {
                        // Pressing a running toggle stops it; other modes ignore
                        // presses while playing.
                        if plan.mode == Mode::Toggle {
                            let mut p = self.players.remove(i);
                            p.release_all(out);
                        }
                        return;
                    }
                    let single = tap && plan.mode != Mode::Toggle;
                    self.players.push(Player::new(Some(trigger), plan, now, single));
                }
                super::Target::Sensitivity(speed) => {
                    if tap {
                        return;
                    }
                    self.sensitivity.retain(|(i, _)| *i != trigger);
                    self.sensitivity.push((trigger, speed));
                    out.push(Out::PointerSpeed(Some(speed)));
                }
                _ => {}
            },
            Cmd::Release(trigger) => {
                if let Some(i) = self.players.iter().position(|p| p.trigger == Some(trigger)) {
                    if self.players[i].plan.mode == Mode::WhileHeld && !self.players[i].single {
                        let mut p = self.players.remove(i);
                        p.release_all(out);
                    }
                }
                let before = self.sensitivity.len();
                self.sensitivity.retain(|(i, _)| *i != trigger);
                if self.sensitivity.len() != before {
                    out.push(Out::PointerSpeed(self.sensitivity.last().map(|(_, s)| *s)));
                }
            }
            Cmd::Tap { vk, mods } => {
                let mods: Vec<u16> = crate::config::Modifier::from_bits(mods)
                    .iter()
                    .map(|m| m.vk())
                    .collect();
                let mut steps: Vec<Step> = mods.iter().map(|&m| Step::KeyDown(m)).collect();
                steps.extend([Step::KeyDown(vk), Step::Delay(TAP_HOLD_MS), Step::KeyUp(vk)]);
                steps.extend(mods.iter().rev().map(|&m| Step::KeyUp(m)));
                let plan = Arc::new(Plan::new(steps, Mode::Once, 0, 0));
                self.players.push(Player::new(None, plan, now, true));
            }
            Cmd::StopAll => {
                for mut p in self.players.drain(..) {
                    p.release_all(out);
                }
                if !self.sensitivity.is_empty() {
                    self.sensitivity.clear();
                    out.push(Out::PointerSpeed(None));
                }
            }
        }
    }

    pub fn poll(&mut self, now: Instant, out: &mut Vec<Out>) {
        self.players.retain_mut(|p| p.run(now, out));
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.players.iter().map(|p| p.next).min()
    }

    #[cfg(test)]
    pub fn is_idle(&self) -> bool {
        self.players.is_empty()
    }
}

/// Waits longer than this use an interruptible (but coarse, ~16 ms on
/// Windows) channel wait for the bulk, then a precise sleep for the rest.
const PRECISE_WINDOW: Duration = Duration::from_millis(20);

/// The worker thread loop: wait for commands or the next due step, emit.
pub fn run_worker(rx: Receiver<Cmd>, mut emit: impl FnMut(&[Out])) {
    let mut sched = Scheduler::default();
    let mut out = Vec::new();
    loop {
        let now = Instant::now();
        let first = match sched.next_deadline() {
            None => match rx.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            },
            Some(due) if due <= now => None,
            Some(due) => {
                let wait = due - now;
                if wait > PRECISE_WINDOW {
                    match rx.recv_timeout(wait - PRECISE_WINDOW) {
                        Ok(c) => Some(c),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                } else {
                    // std's sleep uses a high-resolution timer on Windows 10+.
                    std::thread::sleep(wait);
                    None
                }
            }
        };
        let now = Instant::now();
        if let Some(c) = first {
            sched.command(c, now, &mut out);
        }
        while let Ok(c) = rx.try_recv() {
            sched.command(c, now, &mut out);
        }
        sched.poll(now, &mut out);
        if !out.is_empty() {
            emit(&out);
            out.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Target;

    const T: Input = Input::Key(0x7C);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn turbo(mode: Mode, first_gap: u32) -> Target {
        Target::Play(Arc::new(Plan::new(
            vec![Step::KeyDown(0x41), Step::Delay(10), Step::KeyUp(0x41), Step::Delay(40)],
            mode,
            0,
            first_gap,
        )))
    }

    /// Drive the scheduler like the worker would, in 1 ms ticks, and return
    /// (time ms, output) pairs.
    fn simulate(s: &mut Scheduler, start: Instant, until_ms: u64) -> Vec<(u64, Out)> {
        let mut log = Vec::new();
        for t in 0..=until_ms {
            let mut out = Vec::new();
            s.poll(start + ms(t), &mut out);
            log.extend(out.into_iter().map(|o| (t, o)));
        }
        log
    }

    fn down_times(log: &[(u64, Out)]) -> Vec<u64> {
        log.iter()
            .filter(|(_, o)| matches!(o, Out::Key { down: true, .. }))
            .map(|(t, _)| *t)
            .collect()
    }

    #[test]
    fn turbo_while_held_repeats_on_schedule_and_releases() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: turbo(Mode::WhileHeld, 0),
                tap: false,
            },
            start,
            &mut out,
        );
        let log = simulate(&mut s, start, 152);
        assert_eq!(down_times(&log), vec![0, 50, 100, 150]);
        // Released mid-press: the held key is let go.
        let mut out = Vec::new();
        s.command(Cmd::Release(T), start + ms(155), &mut out);
        assert_eq!(out, vec![Out::Key { vk: 0x41, down: false }]);
        assert!(s.is_idle());
    }

    #[test]
    fn repeat_after_delay_works_like_auto_repeat() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: turbo(Mode::WhileHeld, 250),
                tap: false,
            },
            start,
            &mut out,
        );
        let log = simulate(&mut s, start, 420);
        // First press at 0, pass ends at 50, then 250 ms extra wait.
        assert_eq!(down_times(&log), vec![0, 300, 350, 400]);
    }

    #[test]
    fn toggle_starts_and_stops_on_presses() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        let press = || Cmd::Press {
            trigger: T,
            target: turbo(Mode::Toggle, 0),
            tap: false,
        };
        s.command(press(), start, &mut out);
        s.command(Cmd::Release(T), start, &mut out); // release doesn't stop a toggle
        let log = simulate(&mut s, start, 120);
        assert_eq!(down_times(&log), vec![0, 50, 100]);
        let mut out = Vec::new();
        s.command(press(), start + ms(120), &mut out);
        assert!(s.is_idle());
        assert!(out.is_empty()); // key was already up at 120 ms
    }

    #[test]
    fn repeat_mode_plays_n_times_with_gap() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let plan = Plan::new(vec![Step::KeyDown(1), Step::KeyUp(1)], Mode::Repeat(3), 20, 20);
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: Target::Play(Arc::new(plan)),
                tap: false,
            },
            start,
            &mut out,
        );
        let log = simulate(&mut s, start, 200);
        assert_eq!(down_times(&log), vec![0, 20, 40]);
        assert!(s.is_idle());
    }

    #[test]
    fn loops_without_delays_do_not_spin() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let plan = Plan::new(vec![Step::Char('x')], Mode::WhileHeld, 0, 0);
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: Target::Play(Arc::new(plan)),
                tap: false,
            },
            start,
            &mut out,
        );
        let mut out = Vec::new();
        s.poll(start + ms(10), &mut out);
        // At most one character per millisecond (plus the first one).
        assert!(out.len() <= 11, "{}", out.len());
    }

    #[test]
    fn tap_plays_one_pass_of_a_hold_loop() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: turbo(Mode::WhileHeld, 0),
                tap: true,
            },
            start,
            &mut out,
        );
        let log = simulate(&mut s, start, 200);
        assert_eq!(down_times(&log), vec![0]);
        assert!(s.is_idle());
    }

    #[test]
    fn wheel_tap_holds_key_briefly() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        s.command(Cmd::Tap { vk: 0x20, mods: 1 }, start, &mut out);
        let log = simulate(&mut s, start, 30);
        assert_eq!(
            log,
            vec![
                (0, Out::Key { vk: 0xA2, down: true }),
                (0, Out::Key { vk: 0x20, down: true }),
                (15, Out::Key { vk: 0x20, down: false }),
                (15, Out::Key { vk: 0xA2, down: false }),
            ]
        );
    }

    #[test]
    fn sensitivity_stacks_and_restores() {
        let now = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        let a = Input::Key(1);
        let b = Input::Key(2);
        let press = |i, v| Cmd::Press {
            trigger: i,
            target: Target::Sensitivity(v),
            tap: false,
        };
        s.command(press(a, 3), now, &mut out);
        s.command(press(b, 15), now, &mut out);
        s.command(Cmd::Release(b), now, &mut out);
        s.command(Cmd::Release(a), now, &mut out);
        assert_eq!(
            out,
            vec![
                Out::PointerSpeed(Some(3)),
                Out::PointerSpeed(Some(15)),
                Out::PointerSpeed(Some(3)),
                Out::PointerSpeed(None),
            ]
        );
    }

    #[test]
    fn stop_all_releases_held_keys_and_buttons() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let plan = Plan::new(
            vec![Step::KeyDown(5), Step::ButtonDown(MouseButton::Left), Step::Delay(1000)],
            Mode::Once,
            0,
            0,
        );
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: Target::Play(Arc::new(plan)),
                tap: false,
            },
            start,
            &mut out,
        );
        s.poll(start, &mut out);
        out.clear();
        s.command(Cmd::StopAll, start, &mut out);
        assert_eq!(
            out,
            vec![
                Out::Key { vk: 5, down: false },
                Out::Button {
                    button: MouseButton::Left,
                    down: false
                },
            ]
        );
    }

    #[test]
    fn falls_back_to_now_when_far_behind() {
        let start = Instant::now();
        let mut s = Scheduler::default();
        let mut out = Vec::new();
        s.command(
            Cmd::Press {
                trigger: T,
                target: turbo(Mode::WhileHeld, 0),
                tap: false,
            },
            start,
            &mut out,
        );
        s.poll(start, &mut out);
        // The worker was stalled for a second: no burst of 20 presses.
        let mut out = Vec::new();
        s.poll(start + ms(1000), &mut out);
        assert!(out.len() <= 2, "{out:?}");
    }
}
