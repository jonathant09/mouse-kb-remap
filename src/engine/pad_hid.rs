//! Report formats of controllers the app reads directly (over USB or
//! Bluetooth HID): PlayStation, Nintendo Switch and Steam. Pure functions
//! from report bytes to a `PadReading`, so they're tested without hardware.
//!
//! Layouts follow SDL's HIDAPI drivers (`src/joystick/hidapi/` in
//! libsdl-org/SDL), the best-tested public description of these protocols.
//! `data[0]` is the report id, as Windows and hidapi deliver it.

use super::pad::{bit, hat_bits, PadReading};
use crate::config::PadButton;

pub const VID_SONY: u16 = 0x054C;
pub const VID_NINTENDO: u16 = 0x057E;
pub const VID_VALVE: u16 = 0x28DE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// DualShock 4 (and the PS4 wireless adapter).
    Ds4,
    /// DualSense and DualSense Edge.
    DualSense,
    SwitchPro,
    JoyConL,
    JoyConR,
    SteamDeck,
    /// The Steam Controller (2025), wired, Bluetooth or through its dongle.
    SteamController,
}

impl Kind {
    pub fn of(vendor: u16, product: u16) -> Option<Kind> {
        Some(match (vendor, product) {
            (VID_SONY, 0x05C4 | 0x09CC | 0x0BA0) => Kind::Ds4,
            (VID_SONY, 0x0CE6 | 0x0DF2) => Kind::DualSense,
            (VID_NINTENDO, 0x2009) => Kind::SwitchPro,
            (VID_NINTENDO, 0x2006) => Kind::JoyConL,
            (VID_NINTENDO, 0x2007) => Kind::JoyConR,
            (VID_VALVE, 0x1205) => Kind::SteamDeck,
            (VID_VALVE, 0x1302..=0x1305) => Kind::SteamController,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Ds4 => "DualShock 4",
            Kind::DualSense => "DualSense",
            Kind::SwitchPro => "Switch Pro Controller",
            Kind::JoyConL => "Joy-Con (L)",
            Kind::JoyConR => "Joy-Con (R)",
            Kind::SteamDeck => "Steam Deck",
            Kind::SteamController => "Steam Controller",
        }
    }

    pub fn is_switch(self) -> bool {
        matches!(self, Kind::SwitchPro | Kind::JoyConL | Kind::JoyConR)
    }

    pub fn is_steam(self) -> bool {
        matches!(self, Kind::SteamDeck | Kind::SteamController)
    }
}

fn le16(d: &[u8], i: usize) -> i16 {
    i16::from_le_bytes([d[i], d[i + 1]])
}

fn ule16(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}

fn ule32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

/// 0-255 with 128 at rest → -32767..32767 (`invert` for axes where 0 is up).
fn axis8(v: u8, invert: bool) -> i16 {
    let a = ((v as i32 - 128) * 32767 / 127).clamp(-32767, 32767);
    (if invert { -a } else { a }) as i16
}

/// Reads one controller's reports; keeps what has to be learned over time
/// (Switch stick ranges).
pub struct Parser {
    pub kind: Kind,
    /// Switch stick extents: per stick and axis, (below centre, above).
    extents: [[(i32, i32); 2]; 2],
}

impl Parser {
    pub fn new(kind: Kind) -> Self {
        Parser {
            kind,
            extents: [[(1433, 1433); 2]; 2],
        }
    }

    pub fn parse(&mut self, data: &[u8]) -> Option<PadReading> {
        if data.is_empty() {
            return None;
        }
        match self.kind {
            Kind::Ds4 => parse_ds4(data),
            Kind::DualSense => parse_dualsense(data),
            Kind::SwitchPro | Kind::JoyConL | Kind::JoyConR => self.parse_switch(data),
            Kind::SteamDeck => parse_deck(data),
            Kind::SteamController => parse_steam_controller(data),
        }
    }

    /// 12-bit Switch stick value → -32767..32767, learning the range.
    fn switch_axis(&mut self, stick: usize, axis: usize, raw: i32) -> i16 {
        let v = raw - 2048;
        let ext = &mut self.extents[stick][axis];
        let out = if v >= 0 {
            ext.1 = ext.1.max(v);
            v * 32767 / ext.1.max(1)
        } else {
            ext.0 = ext.0.max(-v);
            v * 32767 / ext.0.max(1)
        };
        out.clamp(-32767, 32767) as i16
    }

    fn parse_switch(&mut self, data: &[u8]) -> Option<PadReading> {
        match data[0] {
            // Full report: buttons, 12-bit sticks, motion.
            0x30 | 0x31 if data.len() >= 49 => {
                let p = &data[1..];
                let (right, shared, left) = (p[2], p[3], p[4]);
                let mut r = PadReading::default();
                let set = |r: &mut PadReading, on: bool, b: PadButton| {
                    if on {
                        r.buttons |= bit(b);
                    }
                };
                // Positions: Nintendo's B is at the bottom (our A), A on the right (our B).
                set(&mut r, right & 0x01 != 0, PadButton::X); // Y
                set(&mut r, right & 0x02 != 0, PadButton::Y); // X
                set(&mut r, right & 0x04 != 0, PadButton::A); // B
                set(&mut r, right & 0x08 != 0, PadButton::B); // A
                set(&mut r, right & 0x40 != 0, PadButton::Rb); // R
                set(&mut r, shared & 0x01 != 0, PadButton::Back); // -
                set(&mut r, shared & 0x02 != 0, PadButton::Start); // +
                set(&mut r, shared & 0x04 != 0, PadButton::Rs);
                set(&mut r, shared & 0x08 != 0, PadButton::Ls);
                set(&mut r, shared & 0x10 != 0, PadButton::Guide); // Home
                set(&mut r, shared & 0x20 != 0, PadButton::Share); // Capture
                set(&mut r, left & 0x01 != 0, PadButton::Down);
                set(&mut r, left & 0x02 != 0, PadButton::Up);
                set(&mut r, left & 0x04 != 0, PadButton::Right);
                set(&mut r, left & 0x08 != 0, PadButton::Left);
                set(&mut r, left & 0x40 != 0, PadButton::Lb); // L
                if self.kind == Kind::JoyConL {
                    set(&mut r, left & 0x20 != 0, PadButton::P1); // SL
                    set(&mut r, left & 0x10 != 0, PadButton::P3); // SR
                }
                if self.kind == Kind::JoyConR {
                    set(&mut r, right & 0x10 != 0, PadButton::P2); // SR
                    set(&mut r, right & 0x20 != 0, PadButton::P4); // SL
                }
                r.lt = if left & 0x80 != 0 { 255 } else { 0 }; // ZL
                r.rt = if right & 0x80 != 0 { 255 } else { 0 }; // ZR
                                                                // Each Joy-Con has one stick; the other one's bytes are meaningless.
                if self.kind != Kind::JoyConR {
                    let x = p[5] as i32 | ((p[6] as i32 & 0x0F) << 8);
                    let y = (p[6] as i32 >> 4) | ((p[7] as i32) << 4);
                    r.lx = self.switch_axis(0, 0, x);
                    r.ly = self.switch_axis(0, 1, y);
                }
                if self.kind != Kind::JoyConL {
                    let x = p[8] as i32 | ((p[9] as i32 & 0x0F) << 8);
                    let y = (p[9] as i32 >> 4) | ((p[10] as i32) << 4);
                    r.rx = self.switch_axis(1, 0, x);
                    r.ry = self.switch_axis(1, 1, y);
                }
                // First of three motion samples: accel x y z, gyro x y z.
                let (gx, gy, gz) = (le16(p, 18) as f32, le16(p, 20) as f32, le16(p, 22) as f32);
                if (gx, gy, gz) != (0.0, 0.0, 0.0) {
                    const PER_DEGREE: f32 = 14.2842;
                    let mut g = [-gy / PER_DEGREE, gz / PER_DEGREE, -gx / PER_DEGREE];
                    if self.kind == Kind::JoyConR {
                        g[0] = -g[0];
                        g[1] = -g[1];
                    }
                    r.gyro = Some(g);
                }
                Some(r)
            }
            // Simple report (before the full mode is set up): Pro Controller only.
            0x3F if self.kind == Kind::SwitchPro && data.len() >= 12 => {
                let p = &data[1..];
                let (b0, b1) = (p[0], p[1]);
                let mut r = PadReading::default();
                let map0 = [
                    (0x01, PadButton::A), // B
                    (0x02, PadButton::B), // A
                    (0x04, PadButton::X), // Y
                    (0x08, PadButton::Y), // X
                    (0x10, PadButton::Lb),
                    (0x20, PadButton::Rb),
                ];
                let map1 = [
                    (0x01, PadButton::Back),
                    (0x02, PadButton::Start),
                    (0x04, PadButton::Ls),
                    (0x08, PadButton::Rs),
                    (0x10, PadButton::Guide),
                    (0x20, PadButton::Share),
                ];
                for (f, b) in map0 {
                    if b0 & f != 0 {
                        r.buttons |= bit(b);
                    }
                }
                for (f, b) in map1 {
                    if b1 & f != 0 {
                        r.buttons |= bit(b);
                    }
                }
                r.buttons |= hat_bits(p[2]);
                r.lt = if b0 & 0x40 != 0 { 255 } else { 0 };
                r.rt = if b0 & 0x80 != 0 { 255 } else { 0 };
                // 16-bit, 0x8000 at rest, y grows downwards.
                let a = |i: usize| (ule16(p, i) as i32 - 0x8000).clamp(-32767, 32767) as i16;
                r.lx = a(3);
                r.ly = -a(5);
                r.rx = a(7);
                r.ry = -a(9);
                Some(r)
            }
            _ => None,
        }
    }
}

/// Face buttons, hat, shoulders, sticks and triggers in the layout both
/// PlayStation controllers share. `b` are the three button bytes.
fn ps_common(r: &mut PadReading, b: [u8; 3], sticks: [u8; 4], lt: u8, rt: u8) {
    let face = b[0] >> 4;
    let mut m = hat_bits(b[0] & 0x0F);
    for (f, btn) in [
        (0x01, PadButton::X), // square
        (0x02, PadButton::A), // cross
        (0x04, PadButton::B), // circle
        (0x08, PadButton::Y), // triangle
    ] {
        if face & f != 0 {
            m |= bit(btn);
        }
    }
    for (f, btn) in [
        (0x01, PadButton::Lb),
        (0x02, PadButton::Rb),
        (0x10, PadButton::Back),  // Share / Create
        (0x20, PadButton::Start), // Options
        (0x40, PadButton::Ls),
        (0x80, PadButton::Rs),
    ] {
        if b[1] & f != 0 {
            m |= bit(btn);
        }
    }
    if b[2] & 0x01 != 0 {
        m |= bit(PadButton::Guide);
    }
    if b[2] & 0x02 != 0 {
        m |= bit(PadButton::Touchpad);
    }
    r.buttons |= m;
    r.lx = axis8(sticks[0], false);
    r.ly = axis8(sticks[1], true);
    r.rx = axis8(sticks[2], false);
    r.ry = axis8(sticks[3], true);
    // Some pads only set the digital trigger bits.
    r.lt = if lt == 0 && b[1] & 0x04 != 0 { 255 } else { lt };
    r.rt = if rt == 0 && b[1] & 0x08 != 0 { 255 } else { rt };
}

/// A PlayStation touchpad finger: 4 bytes, top bit of the first set when
/// not touching, then 12-bit x and y. Width is 1920.
fn ps_touch(d: &[u8]) -> Option<(f32, f32)> {
    if d[0] & 0x80 != 0 {
        return None;
    }
    let x = d[1] as u32 | ((d[2] as u32 & 0x0F) << 8);
    let y = (d[2] as u32 >> 4) | ((d[3] as u32) << 4);
    Some((x as f32 / 1920.0, y as f32 / 1920.0))
}

/// PlayStation gyro: 1/16 degree per second, already pitch / yaw / roll.
fn ps_gyro(d: &[u8], i: usize) -> [f32; 3] {
    [
        le16(d, i) as f32 / 16.0,
        le16(d, i + 2) as f32 / 16.0,
        le16(d, i + 4) as f32 / 16.0,
    ]
}

fn parse_ds4(data: &[u8]) -> Option<PadReading> {
    // USB (and Bluetooth before enhanced mode): report 1; Bluetooth: 0x11-0x19
    // with two more bytes in front.
    let p = match data[0] {
        0x01 => &data[1..],
        0x11..=0x19 if data.len() >= 3 + 42 => &data[3..],
        _ => return None,
    };
    if p.len() < 9 {
        return None;
    }
    let mut r = PadReading::default();
    ps_common(&mut r, [p[4], p[5], p[6]], [p[0], p[1], p[2], p[3]], p[7], p[8]);
    if p.len() >= 42 {
        r.gyro = Some(ps_gyro(p, 12));
        r.touch = ps_touch(&p[34..38]);
    }
    Some(r)
}

fn parse_dualsense(data: &[u8]) -> Option<PadReading> {
    let p = match data[0] {
        // Bluetooth simple report (DirectInput mode): DualShock 4 layout.
        0x01 if data.len() == 10 || data.len() == 78 => {
            let p = &data[1..];
            let mut r = PadReading::default();
            ps_common(&mut r, [p[4], p[5], p[6]], [p[0], p[1], p[2], p[3]], p[7], p[8]);
            return Some(r);
        }
        0x01 => &data[1..],
        0x31 if data.len() >= 2 + 40 => &data[2..],
        _ => return None,
    };
    if p.len() < 40 {
        return None;
    }
    let mut r = PadReading::default();
    ps_common(&mut r, [p[7], p[8], p[9]], [p[0], p[1], p[2], p[3]], p[4], p[5]);
    for (f, btn) in [
        (0x04, PadButton::Mic),
        (0x40, PadButton::P1), // Edge left paddle
        (0x80, PadButton::P2), // Edge right paddle
        (0x10, PadButton::P3), // Edge left Fn
        (0x20, PadButton::P4), // Edge right Fn
    ] {
        if p[9] & f != 0 {
            r.buttons |= bit(btn);
        }
    }
    r.gyro = Some(ps_gyro(p, 15));
    r.touch = ps_touch(&p[32..36]);
    Some(r)
}

/// Steam trackpad: -32768..32767 each way, y up → (0..1, 0..1) from the top left.
fn steam_touch(x: i16, y: i16) -> (f32, f32) {
    (x as f32 / 65536.0 + 0.5, -(y as f32) / 65536.0 + 0.5)
}

/// Steam gyro: 2000 degrees per second full scale; x pitch, z yaw, y roll.
fn steam_gyro(d: &[u8], i: usize) -> [f32; 3] {
    const PER_DEGREE: f32 = 32768.0 / 2000.0;
    let (x, y, z) = (le16(d, i) as f32, le16(d, i + 2) as f32, le16(d, i + 4) as f32);
    [x / PER_DEGREE, z / PER_DEGREE, -y / PER_DEGREE]
}

fn set_bits(r: &mut PadReading, value: u32, map: &[(u32, PadButton)]) {
    for (f, b) in map {
        if value & f != 0 {
            r.buttons |= bit(*b);
        }
    }
}

/// Steam Deck: 64-byte reports with a 4-byte header (version 1, type 9).
fn parse_deck(data: &[u8]) -> Option<PadReading> {
    if data.len() < 64 || ule16(data, 0) != 1 || data[2] != 9 {
        return None;
    }
    let p = &data[4..];
    let (low, high) = (ule32(p, 4), ule32(p, 8));
    let mut r = PadReading::default();
    set_bits(
        &mut r,
        low,
        &[
            (0x0000_0004, PadButton::Rb),
            (0x0000_0008, PadButton::Lb),
            (0x0000_0010, PadButton::Y),
            (0x0000_0020, PadButton::B),
            (0x0000_0040, PadButton::X),
            (0x0000_0080, PadButton::A),
            (0x0000_0100, PadButton::Up),
            (0x0000_0200, PadButton::Right),
            (0x0000_0400, PadButton::Left),
            (0x0000_0800, PadButton::Down),
            (0x0000_1000, PadButton::Back),  // View
            (0x0000_2000, PadButton::Guide), // Steam
            (0x0000_4000, PadButton::Start), // Menu
            (0x0000_8000, PadButton::P3),    // L5
            (0x0001_0000, PadButton::P4),    // R5
            (0x0002_0000, PadButton::LeftPad),
            (0x0004_0000, PadButton::Touchpad),
            (0x0040_0000, PadButton::Ls),
            (0x0400_0000, PadButton::Rs),
        ],
    );
    set_bits(
        &mut r,
        high,
        &[
            (0x0000_0200, PadButton::P1),    // L4
            (0x0000_0400, PadButton::P2),    // R4
            (0x0004_0000, PadButton::Share), // quick access (…)
        ],
    );
    r.lt = (ule16(p, 40) / 128).min(255) as u8;
    r.rt = (ule16(p, 42) / 128).min(255) as u8;
    r.lx = le16(p, 44).max(-32767);
    r.ly = le16(p, 46).max(-32767);
    r.rx = le16(p, 48).max(-32767);
    r.ry = le16(p, 50).max(-32767);
    r.gyro = Some(steam_gyro(p, 26));
    if low & 0x0010_0000 != 0 {
        r.touch = Some(steam_touch(le16(p, 16), le16(p, 18)));
    }
    Some(r)
}

/// Steam Controller (2025): reports 0x42 / 0x45, or 0x47 with a trackpad
/// timestamp that moves the trackpad fields by two bytes.
fn parse_steam_controller(data: &[u8]) -> Option<PadReading> {
    let pad_at = match data[0] {
        0x42 | 0x45 => 17,
        0x47 => 19,
        _ => return None,
    };
    if data.len() < 1 + 45 {
        return None;
    }
    let p = &data[1..];
    let buttons = ule32(p, 1);
    let mut r = PadReading::default();
    set_bits(
        &mut r,
        buttons,
        &[
            (0x0000_0001, PadButton::A),
            (0x0000_0002, PadButton::B),
            (0x0000_0004, PadButton::X),
            (0x0000_0008, PadButton::Y),
            (0x0000_0010, PadButton::Share), // quick access (…)
            (0x0000_0020, PadButton::Rs),
            (0x0000_0040, PadButton::Back), // View
            (0x0000_0080, PadButton::P2),   // R4
            (0x0000_0100, PadButton::P4),   // R5
            (0x0000_0200, PadButton::Rb),
            (0x0000_0400, PadButton::Down),
            (0x0000_0800, PadButton::Right),
            (0x0000_1000, PadButton::Left),
            (0x0000_2000, PadButton::Up),
            (0x0000_4000, PadButton::Start), // Menu
            (0x0000_8000, PadButton::Ls),
            (0x0001_0000, PadButton::Guide), // Steam
            (0x0002_0000, PadButton::P1),    // L4
            (0x0004_0000, PadButton::P3),    // L5
            (0x0008_0000, PadButton::Lb),
            (0x0040_0000, PadButton::Touchpad), // right trackpad click
            (0x0400_0000, PadButton::LeftPad),
        ],
    );
    r.lt = (le16(p, 5).max(0) / 128) as u8;
    r.rt = (le16(p, 7).max(0) / 128) as u8;
    r.lx = le16(p, 9).max(-32767);
    r.ly = le16(p, 11).max(-32767);
    r.rx = le16(p, 13).max(-32767);
    r.ry = le16(p, 15).max(-32767);
    // Both layouts have the gyro at the same place.
    r.gyro = Some(steam_gyro(p, 39));
    if buttons & 0x0020_0000 != 0 {
        r.touch = Some(steam_touch(le16(p, pad_at + 6), le16(p, pad_at + 8)));
    }
    Some(r)
}

/// Output reports that put a Switch controller into full reports with
/// motion: on USB the Pro Controller first needs a handshake. Each is padded
/// to the length the connection expects (49 bytes Bluetooth, 64 USB).
pub fn switch_setup(usb_pro: bool, counter: &mut u8) -> Vec<Vec<u8>> {
    let len = if usb_pro { 64 } else { 49 };
    let pad = |mut v: Vec<u8>| {
        v.resize(len, 0);
        v
    };
    let mut out = Vec::new();
    if usb_pro {
        // Handshake, high speed, handshake, then HID only (no timeout).
        for cmd in [0x02, 0x03, 0x02, 0x04] {
            out.push(pad(vec![0x80, cmd]));
        }
    }
    // Subcommands: rumble data at rest, then the command and its argument.
    const NEUTRAL: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    for (cmd, arg) in [(0x03u8, 0x30u8), (0x40, 0x01)] {
        let mut v = vec![0x01, *counter & 0x0F];
        v.extend_from_slice(&NEUTRAL);
        v.extend_from_slice(&[cmd, arg]);
        *counter = counter.wrapping_add(1);
        out.push(pad(v));
    }
    out
}

/// Feature reports that turn off a Steam Deck's / Steam Controller's own
/// mouse and keyboard emulation ("lizard mode"). It comes back by itself a
/// few seconds after the last one, so they're resent regularly.
pub fn steam_lizard_off(kind: Kind) -> Vec<Vec<u8>> {
    const CLEAR_DIGITAL_MAPPINGS: u8 = 0x81;
    const SET_SETTINGS: u8 = 0x87;
    const LEFT_TRACKPAD_MODE: u8 = 7;
    const RIGHT_TRACKPAD_MODE: u8 = 8;
    const LIZARD_MODE: u8 = 9;
    const SMOOTH_ABSOLUTE_MOUSE: u8 = 24;
    const LEFT_CLICK_PRESSURE: u8 = 52;
    const RIGHT_CLICK_PRESSURE: u8 = 53;
    const TRACKPAD_NONE: u16 = 7;
    let settings = |report_id: u8, list: &[(u8, u16)]| {
        let mut v = vec![report_id, SET_SETTINGS, (list.len() * 3) as u8];
        for (n, value) in list {
            v.push(*n);
            v.extend_from_slice(&value.to_le_bytes());
        }
        v
    };
    match kind {
        Kind::SteamDeck => {
            let mut clear = vec![0x00, CLEAR_DIGITAL_MAPPINGS, 0];
            clear.resize(65, 0);
            let mut set = settings(
                0x00,
                &[
                    (SMOOTH_ABSOLUTE_MOUSE, 0),
                    (LEFT_TRACKPAD_MODE, TRACKPAD_NONE),
                    (RIGHT_TRACKPAD_MODE, TRACKPAD_NONE),
                    (LEFT_CLICK_PRESSURE, 0xFFFF),
                    (RIGHT_CLICK_PRESSURE, 0xFFFF),
                ],
            );
            set.resize(65, 0);
            vec![clear, set]
        }
        Kind::SteamController => {
            let mut set = settings(0x01, &[(LIZARD_MODE, 0)]);
            set.resize(64, 0);
            vec![set]
        }
        _ => vec![],
    }
}

/// Feature report whose reading puts a PlayStation controller on Bluetooth
/// into its full reports (with motion and touchpad). Games reading it as a
/// plain DirectInput controller may stop seeing it until it reconnects, so
/// it's only asked for when gyro or touchpad is used.
pub fn ps_enhanced_feature(kind: Kind) -> Option<u8> {
    match kind {
        Kind::Ds4 => Some(0x05),
        Kind::DualSense => Some(0x09),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pad::bits;

    fn has(r: &PadReading, list: &[PadButton]) -> bool {
        r.buttons == bits(list)
    }

    #[test]
    fn kinds_by_vendor_and_product() {
        assert_eq!(Kind::of(0x054C, 0x09CC), Some(Kind::Ds4));
        assert_eq!(Kind::of(0x054C, 0x0DF2), Some(Kind::DualSense));
        assert_eq!(Kind::of(0x057E, 0x2007), Some(Kind::JoyConR));
        assert_eq!(Kind::of(0x28DE, 0x1205), Some(Kind::SteamDeck));
        assert_eq!(Kind::of(0x28DE, 0x1304), Some(Kind::SteamController));
        assert_eq!(Kind::of(0x28DE, 0x11FF), None); // Steam's virtual gamepad
        assert_eq!(Kind::of(0x045E, 0x02FD), None);
    }

    #[test]
    fn dualshock4_usb_and_bluetooth() {
        // USB report 1: sticks centred-ish, cross + d-pad right, L1, Options, PS.
        let mut d = vec![0u8; 64];
        d[0] = 0x01;
        d[1..5].copy_from_slice(&[128, 0, 255, 128]); // LX, LY (up), RX (right), RY
        d[5] = 0x20 | 2; // cross, hat right
        d[6] = 0x01 | 0x20; // L1, Options
        d[7] = 0x01; // PS
        d[8] = 200; // L2
        d[13..15].copy_from_slice(&(-160i16).to_le_bytes()); // gyro pitch: -10 deg/s
        d[35] = 0x05; // finger down (top bit clear)
        d[36] = 0x80; // x = 0x780 = 1920 → 1.0
        d[37] = 0x07;
        d[38] = 0x00;
        let r = parse_ds4(&d).unwrap();
        assert!(has(
            &r,
            &[
                PadButton::A,
                PadButton::Right,
                PadButton::Lb,
                PadButton::Start,
                PadButton::Guide
            ]
        ));
        assert_eq!((r.lx, r.rx), (0, 32767));
        assert_eq!(r.ly, 32767, "0 is up");
        assert_eq!(r.lt, 200);
        assert_eq!(r.gyro.unwrap()[0], -10.0);
        assert_eq!(r.touch, Some((1.0, 0.0)));
        // The same state over Bluetooth (report 0x11, two bytes more in front).
        let mut bt = vec![0x11, 0xC0, 0x00];
        bt.extend_from_slice(&d[1..]);
        bt.resize(78, 0);
        assert_eq!(parse_ds4(&bt), Some(r));
        // Bluetooth before enhanced mode: 10 bytes, no motion.
        let simple = parse_ds4(&d[..10]).unwrap();
        assert!(simple.gyro.is_none() && simple.buttons == r.buttons);
    }

    #[test]
    fn dualsense_full_simple_and_edge() {
        let mut d = vec![0u8; 64];
        d[0] = 0x01;
        let p = &mut d[1..];
        p[..4].copy_from_slice(&[128, 128, 128, 128]);
        p[4] = 0; // L2
        p[5] = 255; // R2
        p[7] = 0x80 | 8; // triangle, hat centred
        p[8] = 0x08 | 0x10; // R2 digital, Create
        p[9] = 0x02 | 0x04 | 0x40 | 0x20; // touchpad click, mic, left paddle, right Fn
        p[17..19].copy_from_slice(&(320i16).to_le_bytes()); // yaw +20 deg/s
        p[32] = 0x80; // no finger
        let r = parse_dualsense(&d).unwrap();
        assert!(has(
            &r,
            &[
                PadButton::Y,
                PadButton::Back,
                PadButton::Touchpad,
                PadButton::Mic,
                PadButton::P1,
                PadButton::P4
            ]
        ));
        assert_eq!((r.lt, r.rt), (0, 255));
        assert_eq!(r.gyro.unwrap()[1], 20.0);
        assert_eq!(r.touch, None);
        // Bluetooth 0x31: one byte more in front.
        let mut bt = vec![0x31, 0x00];
        bt.extend_from_slice(&d[1..]);
        bt.resize(78, 0);
        assert_eq!(parse_dualsense(&bt), Some(r));
        // Bluetooth simple report.
        let s = parse_dualsense(&[0x01, 128, 128, 128, 128, 0x20 | 8, 0x02, 0x01, 0, 0]).unwrap();
        assert!(has(&s, &[PadButton::A, PadButton::Rb, PadButton::Guide]));
    }

    #[test]
    fn switch_full_reports_and_joycons() {
        let mut d = vec![0u8; 49];
        d[0] = 0x30;
        let p = &mut d[1..];
        p[2] = 0x04 | 0x80; // B (bottom), ZR
        p[3] = 0x10 | 0x20; // Home, Capture
        p[4] = 0x02 | 0x20; // d-pad up, SL (left)
                            // Left stick: x = 2048 + 1433 (full right), y = 2048 (centre).
        let (x, y) = (2048 + 1433u32, 2048u32);
        p[5] = (x & 0xFF) as u8;
        p[6] = ((x >> 8) & 0x0F) as u8 | ((y & 0x0F) << 4) as u8;
        p[7] = (y >> 4) as u8;
        p[18..20].copy_from_slice(&(143i16).to_le_bytes()); // gyro x → roll
        let mut pro = Parser::new(Kind::SwitchPro);
        let r = pro.parse(&d).unwrap();
        assert!(has(
            &r,
            &[PadButton::A, PadButton::Guide, PadButton::Share, PadButton::Up]
        ));
        assert_eq!(r.rt, 255);
        assert_eq!((r.lx, r.ly), (32767, 0));
        let g = r.gyro.unwrap();
        assert!((g[2] + 10.01).abs() < 0.05, "{g:?}");
        // The left Joy-Con reports SL as an extra button and has no right stick.
        let mut l = Parser::new(Kind::JoyConL);
        let r = l.parse(&d).unwrap();
        assert!(r.buttons & bit(PadButton::P1) != 0);
        assert_eq!((r.rx, r.ry), (0, 0));
        // Pro Controller simple report: Y button, hat left, sticks centred.
        let mut s = vec![0x3Fu8, 0x04, 0x10, 6];
        for _ in 0..4 {
            s.extend_from_slice(&0x8000u16.to_le_bytes());
        }
        let r = pro.parse(&s).unwrap();
        assert!(has(&r, &[PadButton::X, PadButton::Guide, PadButton::Left]));
        assert_eq!((r.lx, r.ly), (0, 0));
    }

    #[test]
    fn steam_deck_and_controller() {
        let mut d = vec![0u8; 64];
        d[0] = 1; // version
        d[2] = 9; // deck state
        d[3] = 64;
        let p = &mut d[4..];
        let low: u32 = 0x80 | 0x2000 | 0x8000 | 0x0010_0000; // A, Steam, L5, right pad touch
        p[4..8].copy_from_slice(&low.to_le_bytes());
        p[8..12].copy_from_slice(&0x0004_0200u32.to_le_bytes()); // L4, quick access
        p[16..18].copy_from_slice(&16384i16.to_le_bytes()); // right pad x
        p[18..20].copy_from_slice(&16384i16.to_le_bytes()); // right pad y (up)
        p[30..32].copy_from_slice(&(328i16).to_le_bytes()); // gyro z → yaw 20 deg/s
        p[42..44].copy_from_slice(&32767u16.to_le_bytes()); // right trigger
        p[46..48].copy_from_slice(&(-32768i16).to_le_bytes()); // left stick y down
        let r = parse_deck(&d).unwrap();
        assert!(has(
            &r,
            &[
                PadButton::A,
                PadButton::Guide,
                PadButton::P3,
                PadButton::P1,
                PadButton::Share
            ]
        ));
        assert_eq!((r.rt, r.ly), (255, -32767));
        assert!((r.gyro.unwrap()[1] - 20.02).abs() < 0.01);
        assert_eq!(r.touch, Some((0.75, 0.25)));
        assert!(parse_deck(&d[..40]).is_none());

        let mut s = vec![0u8; 64];
        s[0] = 0x47;
        let p = &mut s[1..];
        p[1..5].copy_from_slice(&(0x10000u32 | 0x0020_0000).to_le_bytes()); // Steam, right touch
        p[19 + 6..19 + 8].copy_from_slice(&(-32768i16).to_le_bytes()); // right pad x (timestamped layout)
        let r = parse_steam_controller(&s).unwrap();
        assert!(has(&r, &[PadButton::Guide]));
        assert_eq!(r.touch, Some((0.0, 0.5)));
    }

    #[test]
    fn setup_reports() {
        let mut n = 0;
        let usb = switch_setup(true, &mut n);
        assert_eq!(usb.len(), 6);
        assert_eq!(&usb[0][..2], &[0x80, 0x02]);
        assert!(usb.iter().all(|r| r.len() == 64));
        assert_eq!(
            &usb[4][..12],
            &[0x01, 0, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40, 0x03, 0x30]
        );
        assert_eq!(usb[5][1], 1, "packet counter counts");
        let bt = switch_setup(false, &mut n);
        assert!(bt.len() == 2 && bt.iter().all(|r| r.len() == 49));
        let deck = steam_lizard_off(Kind::SteamDeck);
        assert_eq!(&deck[1][..6], &[0x00, 0x87, 15, 24, 0, 0]);
        assert_eq!(deck[1].len(), 65);
        let sc = steam_lizard_off(Kind::SteamController);
        assert_eq!(&sc[0][..6], &[0x01, 0x87, 3, 9, 0, 0]);
    }
}
