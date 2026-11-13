//! Game controller readings → input events, and stick → pointer movement.
//! Platform-independent: Windows reads controllers through XInput, Linux
//! through evdev; both fill a `PadReading` in XInput's terms.

use crate::config::PadButton;

/// Button bits of `PadReading::buttons` (XInput's).
pub const PAD_UP: u16 = 0x0001;
pub const PAD_DOWN: u16 = 0x0002;
pub const PAD_LEFT: u16 = 0x0004;
pub const PAD_RIGHT: u16 = 0x0008;
pub const PAD_START: u16 = 0x0010;
pub const PAD_BACK: u16 = 0x0020;
pub const PAD_LS: u16 = 0x0040;
pub const PAD_RS: u16 = 0x0080;
pub const PAD_LB: u16 = 0x0100;
pub const PAD_RB: u16 = 0x0200;
pub const PAD_A: u16 = 0x1000;
pub const PAD_B: u16 = 0x2000;
pub const PAD_X: u16 = 0x4000;
pub const PAD_Y: u16 = 0x8000;

/// One reading of a controller: buttons, triggers 0-255, sticks with up
/// and right positive.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct PadReading {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

const BUTTONS: [(PadButton, u16); 14] = [
    (PadButton::Up, PAD_UP),
    (PadButton::Down, PAD_DOWN),
    (PadButton::Left, PAD_LEFT),
    (PadButton::Right, PAD_RIGHT),
    (PadButton::Start, PAD_START),
    (PadButton::Back, PAD_BACK),
    (PadButton::Ls, PAD_LS),
    (PadButton::Rs, PAD_RS),
    (PadButton::Lb, PAD_LB),
    (PadButton::Rb, PAD_RB),
    (PadButton::A, PAD_A),
    (PadButton::B, PAD_B),
    (PadButton::X, PAD_X),
    (PadButton::Y, PAD_Y),
];

/// Bit of `b` in a `pad_mask`.
pub fn bit(b: PadButton) -> u32 {
    1 << PadButton::ALL.iter().position(|x| *x == b).unwrap_or(0)
}

/// Which controller inputs count as pressed (bit n = `PadButton::ALL[n]`).
/// Triggers and stick directions have some hysteresis, so a value hovering
/// at the threshold doesn't chatter.
pub fn pad_mask(r: &PadReading, prev: u32) -> u32 {
    let mut mask = 0;
    for (b, flag) in BUTTONS {
        if r.buttons & flag != 0 {
            mask |= bit(b);
        }
    }
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

/// Inputs that changed between two masks: (button, pressed).
pub fn changes(before: u32, after: u32) -> impl Iterator<Item = (PadButton, bool)> {
    let changed = before ^ after;
    PadButton::ALL
        .into_iter()
        .enumerate()
        .filter(move |(i, _)| changed & (1 << i) != 0)
        .map(move |(i, b)| (b, after & (1 << i) != 0))
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
    let dx = fx / mag * v + rest.0;
    let dy = fy / mag * v + rest.1;
    let (ix, iy) = (dx.trunc(), dy.trunc());
    *rest = (dx - ix, dy - iy);
    (ix as i32, iy as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_mask_has_hysteresis() {
        let mut r = PadReading {
            buttons: PAD_A | PAD_UP,
            ..Default::default()
        };
        let m = pad_mask(&r, 0);
        assert_eq!(m, bit(PadButton::A) | bit(PadButton::Up));
        assert_eq!(
            changes(0, m).collect::<Vec<_>>(),
            vec![(PadButton::A, true), (PadButton::Up, true)]
        );
        r.buttons = 0;
        r.rt = 100; // past a third
        r.ly = 20000; // past half way up
        let m = pad_mask(&r, 0);
        assert_eq!(m, bit(PadButton::Rt) | bit(PadButton::LsUp));
        // Eased off a little: still held. Further: released.
        r.rt = 60;
        r.ly = 13000;
        let m = pad_mask(&r, m);
        assert_eq!(m, bit(PadButton::Rt) | bit(PadButton::LsUp));
        r.rt = 40;
        r.ly = 10000;
        assert_eq!(pad_mask(&r, m), 0);
        assert_eq!(
            changes(m, 0).collect::<Vec<_>>(),
            vec![(PadButton::Rt, false), (PadButton::LsUp, false)]
        );
        // Not pushed far enough from rest.
        r.rt = 60;
        r.rx = -13000;
        assert_eq!(pad_mask(&r, 0), 0);
        r.rx = -20000;
        assert_eq!(pad_mask(&r, 0), bit(PadButton::RsLeft));
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
}
