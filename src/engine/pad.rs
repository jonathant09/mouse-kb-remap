//! Game controller readings → input events, and stick / gyro / touchpad →
//! pointer movement. Platform-independent: Windows reads controllers through
//! XInput and HID (`pad_hid.rs`), Linux through evdev; all fill a
//! `PadReading`.

use crate::config::{PadButton, StickPointer};

/// One reading of a controller.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PadReading {
    /// Digital buttons: `bit(b)` for named buttons (not the triggers or
    /// stick directions, which come from the analog values).
    pub buttons: u64,
    /// Numbered buttons of an unknown controller: bit n-1 = `Button(n)`.
    pub numbered: u32,
    /// Triggers, 0-255.
    pub lt: u8,
    pub rt: u8,
    /// Sticks, -32767..32767, up and right positive.
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
    /// Rotation in degrees per second: pitch (front up +), yaw (turn left +),
    /// roll. `None` if the controller has no gyro or didn't send it.
    pub gyro: Option<[f32; 3]>,
    /// Main touchpad finger, in touchpad widths from the top left. `None`
    /// when not touched (or no touchpad).
    pub touch: Option<(f32, f32)>,
}

/// Bit of a named button in masks.
pub fn bit(b: PadButton) -> u64 {
    match PadButton::ALL.iter().position(|x| *x == b) {
        Some(i) => 1 << i,
        None => 0,
    }
}

/// Bits for several buttons.
#[cfg(test)]
pub fn bits(list: &[PadButton]) -> u64 {
    list.iter().fold(0, |m, b| m | bit(*b))
}

/// XInput `wButtons` → digital button bits.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn from_xinput(buttons: u16) -> u64 {
    const MAP: [(u16, PadButton); 15] = [
        (0x0001, PadButton::Up),
        (0x0002, PadButton::Down),
        (0x0004, PadButton::Left),
        (0x0008, PadButton::Right),
        (0x0010, PadButton::Start),
        (0x0020, PadButton::Back),
        (0x0040, PadButton::Ls),
        (0x0080, PadButton::Rs),
        (0x0100, PadButton::Lb),
        (0x0200, PadButton::Rb),
        (0x0400, PadButton::Guide),
        (0x1000, PadButton::A),
        (0x2000, PadButton::B),
        (0x4000, PadButton::X),
        (0x8000, PadButton::Y),
    ];
    MAP.iter()
        .filter(|(flag, _)| buttons & flag != 0)
        .fold(0, |m, (_, b)| m | bit(*b))
}

/// A d-pad given as a hat switch (0 up, clockwise to 7, anything else
/// centred) → d-pad bits.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn hat_bits(hat: u8) -> u64 {
    let (up, right, down, left) = (
        bit(PadButton::Up),
        bit(PadButton::Right),
        bit(PadButton::Down),
        bit(PadButton::Left),
    );
    match hat {
        0 => up,
        1 => up | right,
        2 => right,
        3 => right | down,
        4 => down,
        5 => down | left,
        6 => left,
        7 => left | up,
        _ => 0,
    }
}

/// Which inputs count as pressed: the digital buttons plus triggers and
/// stick directions. Triggers and stick directions have some hysteresis, so
/// a value hovering at the threshold doesn't chatter.
pub fn pad_mask(r: &PadReading, prev: u64) -> u64 {
    let mut mask = r.buttons;
    let mut analog = |b: PadButton, value: f32, on: f32, off: f32| {
        let was = prev & bit(b) != 0;
        if value >= on || (was && value > off) {
            mask |= bit(b);
        }
    };
    analog(PadButton::Lt, r.lt as f32 / 255.0, 0.33, 0.2);
    analog(PadButton::Rt, r.rt as f32 / 255.0, 0.33, 0.2);
    let axis = |v: i16| v as f32 / 32767.0;
    for (up, down, left, right, x, y) in [
        (
            PadButton::LsUp,
            PadButton::LsDown,
            PadButton::LsLeft,
            PadButton::LsRight,
            r.lx,
            r.ly,
        ),
        (
            PadButton::RsUp,
            PadButton::RsDown,
            PadButton::RsLeft,
            PadButton::RsRight,
            r.rx,
            r.ry,
        ),
    ] {
        analog(up, axis(y), 0.5, 0.35);
        analog(down, -axis(y), 0.5, 0.35);
        analog(left, -axis(x), 0.5, 0.35);
        analog(right, axis(x), 0.5, 0.35);
    }
    mask
}

/// Inputs that changed: (button, pressed). Masks are (named, numbered).
pub fn changes(before: (u64, u32), after: (u64, u32)) -> Vec<(PadButton, bool)> {
    let mut out = Vec::new();
    let named = before.0 ^ after.0;
    for (i, b) in PadButton::ALL.into_iter().enumerate() {
        if named & (1 << i) != 0 {
            out.push((b, after.0 & (1 << i) != 0));
        }
    }
    let numbered = before.1 ^ after.1;
    for n in 0..PadButton::MAX_NUMBERED as u32 {
        if numbered & (1 << n) != 0 {
            out.push((PadButton::Button(n as u8 + 1), after.1 & (1 << n) != 0));
        }
    }
    out
}

/// Pointer movement for one poll from a stick: a dead zone, then speed
/// growing with the square of the tilt (fine control near the centre).
/// `speed` is 1-20; `rest` keeps the fractions of a pixel between polls.
pub fn stick_move(x: i16, y: i16, speed: u32, dt: f32, rest: &mut (f32, f32)) -> (i32, i32) {
    const DEAD: f32 = 0.24;
    let (fx, fy) = (x as f32 / 32767.0, -(y as f32) / 32767.0);
    let mag = (fx * fx + fy * fy).sqrt();
    if mag < DEAD {
        *rest = (0.0, 0.0);
        return (0, 0);
    }
    let tilt = ((mag - DEAD) / (1.0 - DEAD)).min(1.0);
    // Speed 10 → 1500 px/s at full tilt.
    let v = tilt * tilt * speed as f32 * 150.0 * dt;
    take_whole(fx / mag * v, fy / mag * v, rest)
}

/// Add the fractions left from last time, keep the new fractions.
fn take_whole(dx: f32, dy: f32, rest: &mut (f32, f32)) -> (i32, i32) {
    let (x, y) = (dx + rest.0, dy + rest.1);
    let (ix, iy) = (x.trunc(), y.trunc());
    *rest = (x - ix, y - iy);
    (ix as i32, iy as i32)
}

/// Gyro → pointer: turning the controller left / right moves the pointer
/// sideways, tilting it up / down moves it up and down. Learns the gyro's
/// resting drift while the controller lies still.
#[derive(Default, Debug)]
pub struct GyroMouse {
    bias: [f32; 3],
    /// Seconds the controller has been still (for drift learning).
    still: f32,
    learned: bool,
    rest: (f32, f32),
}

/// Pointer settings for one controller update.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerSettings {
    pub stick: StickPointer,
    pub stick_speed: u32,
    pub gyro: bool,
    pub gyro_speed: u32,
    pub gyro_invert_x: bool,
    pub gyro_invert_y: bool,
    pub touch: bool,
    pub touch_speed: u32,
}

impl GyroMouse {
    /// Pixels per degree at speed 10.
    const PX_PER_DEGREE: f32 = 15.0;

    /// One gyro sample (degrees per second) over `dt` seconds.
    pub fn update(&mut self, gyro: [f32; 3], dt: f32, s: &PointerSettings, on: bool) -> (i32, i32) {
        // Drift: a still controller still reports a little rotation.
        let near = |g: [f32; 3], b: [f32; 3], lim: f32| g.iter().zip(b).all(|(g, b)| (g - b).abs() < lim);
        if near(gyro, self.bias, if self.learned { 1.5 } else { 6.0 }) {
            self.still += dt;
            if self.still > 0.5 {
                let k = if self.learned { 0.02 } else { 0.2 };
                for (b, g) in self.bias.iter_mut().zip(gyro) {
                    *b += (g - *b) * k;
                }
                if self.still > 1.0 {
                    self.learned = true;
                }
            }
        } else {
            self.still = 0.0;
        }
        if !on {
            self.rest = (0.0, 0.0);
            return (0, 0);
        }
        let g: Vec<f32> = gyro.iter().zip(self.bias).map(|(g, b)| g - b).collect();
        // Soft dead band against what's left of the drift and hand tremor.
        let soft = |v: f32| {
            const BAND: f32 = 0.6;
            if v.abs() < BAND {
                v * v.abs() / BAND
            } else {
                v
            }
        };
        let scale = Self::PX_PER_DEGREE * s.gyro_speed.clamp(1, 20) as f32 / 10.0 * dt;
        let mut dx = -soft(g[1]) * scale;
        let mut dy = -soft(g[0]) * scale;
        if s.gyro_invert_x {
            dx = -dx;
        }
        if s.gyro_invert_y {
            dy = -dy;
        }
        take_whole(dx, dy, &mut self.rest)
    }
}

/// Touchpad → pointer, like a laptop touchpad (relative).
#[derive(Default, Debug)]
pub struct TouchMouse {
    last: Option<(f32, f32)>,
    rest: (f32, f32),
}

impl TouchMouse {
    /// Pixels for a swipe across the whole touchpad at speed 10.
    const PX_PER_WIDTH: f32 = 1200.0;

    pub fn update(&mut self, touch: Option<(f32, f32)>, speed: u32) -> (i32, i32) {
        let Some(now) = touch else {
            self.last = None;
            self.rest = (0.0, 0.0);
            return (0, 0);
        };
        let Some(last) = self.last.replace(now) else {
            return (0, 0);
        };
        let (dx, dy) = (now.0 - last.0, now.1 - last.1);
        // A jump means another finger: start over.
        if dx.abs() > 0.25 || dy.abs() > 0.25 {
            return (0, 0);
        }
        let k = Self::PX_PER_WIDTH * speed.clamp(1, 20) as f32 / 10.0;
        take_whole(dx * k, dy * k, &mut self.rest)
    }
}

/// Mouse movement → a controller stick (for games that only take a
/// controller). The stick leans as far as the mouse moves fast: its
/// velocity, smoothed over a few milliseconds, scaled by the speed setting
/// and lifted past the game's own dead zone so slow movements still turn.
#[derive(Default, Debug, Clone, Copy)]
pub struct MouseStick {
    /// Smoothed velocity, counts per second (y down, like the mouse).
    vel: (f32, f32),
}

impl MouseStick {
    /// Mouse counts per second for a full deflection at speed 10.
    const FULL_AT: f32 = 4000.0;
    /// How quickly the velocity follows the mouse (seconds).
    const SMOOTH: f32 = 0.025;
    /// Below this deflection the stick is centred.
    const REST: f32 = 0.01;

    /// The mouse moved `dx`, `dy` counts in the last `dt` seconds. Returns
    /// the stick position (XInput convention: up is positive). `speed` is
    /// 1-20, `deadzone` the percentage the stick starts at once it moves.
    pub fn update(&mut self, dx: i32, dy: i32, dt: f32, speed: u32, deadzone: u32) -> (i16, i16) {
        if dt <= 0.0 {
            return self.stick(speed, deadzone);
        }
        let k = 1.0 - (-dt / Self::SMOOTH).exp();
        let (vx, vy) = (dx as f32 / dt, dy as f32 / dt);
        self.vel.0 += (vx - self.vel.0) * k;
        self.vel.1 += (vy - self.vel.1) * k;
        self.stick(speed, deadzone)
    }

    fn stick(&mut self, speed: u32, deadzone: u32) -> (i16, i16) {
        let full = Self::FULL_AT * 10.0 / speed.clamp(1, 20) as f32;
        let (x, y) = (self.vel.0 / full, self.vel.1 / full);
        let mag = (x * x + y * y).sqrt();
        if mag < Self::REST {
            // Centred, but keep a slow movement building up.
            if mag < Self::REST / 10.0 {
                self.vel = (0.0, 0.0);
            }
            return (0, 0);
        }
        if mag > 1.0 {
            // Past full: a fast flick shouldn't keep the stick out longer.
            self.vel = (self.vel.0 / mag, self.vel.1 / mag);
        }
        let floor = deadzone.min(90) as f32 / 100.0;
        let out = floor + (1.0 - floor) * mag.min(1.0);
        let scale = out / mag * 32767.0;
        ((x * scale).round() as i16, (-y * scale).round() as i16)
    }

    pub fn reset(&mut self) {
        self.vel = (0.0, 0.0);
    }
}

/// Everything one controller remembers between readings.
#[derive(Default, Debug)]
pub struct PadState {
    pub named: u64,
    pub numbered: u32,
    stick_rest: (f32, f32),
    gyro: GyroMouse,
    touch: TouchMouse,
}

impl PadState {
    /// A new reading `dt` seconds after the last: which inputs changed, and
    /// how far to move the pointer. `gyro_held` is the Gyro action being on.
    pub fn update(
        &mut self,
        r: &PadReading,
        dt: f32,
        s: &PointerSettings,
        gyro_held: bool,
    ) -> (Vec<(PadButton, bool)>, (i32, i32)) {
        let named = pad_mask(r, self.named);
        let changed = changes((self.named, self.numbered), (named, r.numbered));
        self.named = named;
        self.numbered = r.numbered;

        let (sx, sy) = match s.stick {
            StickPointer::Off => (0, 0),
            StickPointer::Left => (r.lx, r.ly),
            StickPointer::Right => (r.rx, r.ry),
        };
        let mut moved = stick_move(sx, sy, s.stick_speed, dt, &mut self.stick_rest);
        if let Some(g) = r.gyro {
            let (x, y) = self.gyro.update(g, dt, s, s.gyro || gyro_held);
            moved = (moved.0 + x, moved.1 + y);
        }
        if s.touch {
            let (x, y) = self.touch.update(r.touch, s.touch_speed);
            moved = (moved.0 + x, moved.1 + y);
        }
        (changed, moved)
    }

    /// Release everything (controller gone).
    pub fn release_all(&mut self) -> Vec<(PadButton, bool)> {
        let out = changes((self.named, self.numbered), (0, 0));
        self.named = 0;
        self.numbered = 0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_moves_the_stick_and_lets_go() {
        let mut m = MouseStick::default();
        let dt = 0.008;
        // A steady 2000 counts/s to the right at speed 10: half of full.
        let mut last = (0, 0);
        for _ in 0..40 {
            last = m.update(16, 0, dt, 10, 20);
        }
        let expect = ((0.2 + 0.8 * 0.5) * 32767.0) as i16;
        assert!((last.0 - expect).abs() < 300, "{last:?} vs {expect}");
        assert_eq!(last.1, 0);
        // Moving down pulls the stick down (negative), and fast is capped.
        for _ in 0..40 {
            last = m.update(0, 400, dt, 10, 20);
        }
        assert!(last.0.abs() < 50 && last.1 <= -32700, "{last:?}");
        // A tiny movement still clears the dead zone.
        let mut slow = MouseStick::default();
        let mut s = (0, 0);
        for _ in 0..40 {
            s = slow.update(1, 0, dt, 10, 25);
        }
        assert!(s.0 >= (0.25 * 32767.0) as i16, "{s:?}");
        // Stopping centres it within a fraction of a second.
        for _ in 0..20 {
            last = m.update(0, 0, dt, 10, 20);
        }
        assert_eq!(last, (0, 0));
        // Diagonals stay inside the circle.
        let mut d = MouseStick::default();
        for _ in 0..40 {
            last = d.update(400, -400, dt, 10, 0);
        }
        let r = ((last.0 as f32).powi(2) + (last.1 as f32).powi(2)).sqrt();
        assert!(r <= 32768.0 && last.0 > 0 && last.1 > 0, "{last:?}");
    }

    #[test]
    fn controller_mask_has_hysteresis() {
        let mut r = PadReading {
            buttons: from_xinput(0x1000 | 0x0001 | 0x0400),
            ..Default::default()
        };
        let m = pad_mask(&r, 0);
        assert_eq!(m, bits(&[PadButton::A, PadButton::Up, PadButton::Guide]));
        assert_eq!(
            changes((0, 0), (m, 0)),
            vec![(PadButton::A, true), (PadButton::Up, true), (PadButton::Guide, true)]
        );
        r.buttons = 0;
        r.rt = 100; // past a third
        r.ly = 20000; // past half way up
        let m = pad_mask(&r, 0);
        assert_eq!(m, bits(&[PadButton::Rt, PadButton::LsUp]));
        // Eased off a little: still held. Further: released.
        r.rt = 60;
        r.ly = 13000;
        let m = pad_mask(&r, m);
        assert_eq!(m, bits(&[PadButton::Rt, PadButton::LsUp]));
        r.rt = 40;
        r.ly = 10000;
        assert_eq!(pad_mask(&r, m), 0);
        assert_eq!(
            changes((m, 0), (0, 0)),
            vec![(PadButton::Rt, false), (PadButton::LsUp, false)]
        );
        // Not pushed far enough from rest.
        r.rt = 60;
        r.rx = -13000;
        assert_eq!(pad_mask(&r, 0), 0);
        r.rx = -20000;
        assert_eq!(pad_mask(&r, 0), bit(PadButton::RsLeft));
        // Numbered buttons.
        assert_eq!(
            changes((0, 0b1), (0, 0b100)),
            vec![(PadButton::Button(1), false), (PadButton::Button(3), true)]
        );
        assert_eq!(hat_bits(3), bits(&[PadButton::Right, PadButton::Down]));
        assert_eq!(hat_bits(8), 0);
    }

    #[test]
    fn stick_moves_the_pointer_outside_the_dead_zone() {
        let mut rest = (0.0, 0.0);
        assert_eq!(stick_move(5000, -5000, 10, 0.004, &mut rest), (0, 0));
        // Full right at speed 10: 1500 px/s, 6 px per 4 ms poll.
        assert_eq!(stick_move(32767, 0, 10, 0.004, &mut rest), (6, 0));
        // Up on the stick is up on screen (negative y).
        let (_, dy) = stick_move(0, 32767, 10, 0.004, &mut rest);
        assert!(dy < 0);
        // Fractions add up over polls. Tilt 0.49 → (0.49 - 0.24) / 0.76 =
        // 0.33 → 0.107 of full speed: 64 px over 100 polls.
        let mut rest = (0.0, 0.0);
        let total: i32 = (0..100).map(|_| stick_move(16000, 0, 10, 0.004, &mut rest).0).sum();
        assert!((62..=65).contains(&total), "{total}");
    }

    #[test]
    fn gyro_learns_drift_then_follows_turns() {
        let s = PointerSettings {
            gyro_speed: 10,
            ..Default::default()
        };
        let mut g = GyroMouse::default();
        let dt = 0.004;
        // Lying still with a little drift: learned, and no movement after.
        for _ in 0..1000 {
            g.update([0.8, -0.5, 0.3], dt, &s, false);
        }
        let mut total = (0, 0);
        for _ in 0..250 {
            let (x, y) = g.update([0.8, -0.5, 0.3], dt, &s, true);
            total = (total.0 + x, total.1 + y);
        }
        assert_eq!(total, (0, 0), "drift is removed");
        // Turning right at 30°/s for half a second (yaw negative): 15° → 225 px right.
        let mut total = (0, 0);
        for _ in 0..125 {
            let (x, y) = g.update([0.8, -30.5, 0.3], dt, &s, true);
            total = (total.0 + x, total.1 + y);
        }
        assert!((215..=230).contains(&total.0) && total.1 == 0, "{total:?}");
        // Tilting the front up moves the pointer up; inverted, down.
        let (_, up) = g.update([40.8, -0.5, 0.3], 0.1, &s, true);
        assert!(up < 0);
        let inv = PointerSettings {
            gyro_invert_y: true,
            ..s
        };
        let (_, down) = g.update([40.8, -0.5, 0.3], 0.1, &inv, true);
        assert!(down > 0);
    }

    #[test]
    fn touchpad_moves_relative_to_the_finger() {
        let mut t = TouchMouse::default();
        assert_eq!(t.update(Some((0.5, 0.25)), 10), (0, 0)); // finger down
        assert_eq!(t.update(Some((0.625, 0.125)), 10), (150, -150));
        assert_eq!(t.update(None, 10), (0, 0)); // lifted
        assert_eq!(t.update(Some((0.25, 0.25)), 10), (0, 0)); // down elsewhere: no jump
        assert_eq!(t.update(Some((0.25, 0.375)), 20), (0, 300));
    }

    #[test]
    fn pad_state_reports_changes_and_motion() {
        let mut p = PadState::default();
        let s = PointerSettings {
            stick: StickPointer::Right,
            stick_speed: 10,
            ..Default::default()
        };
        let r = PadReading {
            buttons: bit(PadButton::P1),
            numbered: 0b10,
            rx: 32767,
            ..Default::default()
        };
        let (changed, moved) = p.update(&r, 0.004, &s, false);
        assert_eq!(
            changed,
            vec![
                (PadButton::RsRight, true),
                (PadButton::P1, true),
                (PadButton::Button(2), true)
            ]
        );
        assert_eq!(moved, (6, 0));
        assert_eq!(p.release_all().len(), 3);
    }
}
