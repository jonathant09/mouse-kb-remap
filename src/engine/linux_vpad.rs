//! Linux: the virtual Xbox 360 controller, a uinput device laid out like the
//! kernel's xpad driver reports a real one (same ids, buttons and axes), so
//! SDL, Steam and games map it like an Xbox 360 controller.
//!
//! What it reports comes from `vpad.rs`; this thread owns the device. As on
//! Windows it exists only while the active profile sends controller output,
//! and goes away a few seconds after that stops.

use super::vpad::{self, MouseFeed, XReport, LINGER, MOUSE_TICK, PRODUCT, RETRY, VENDOR};
use super::{EngineEvent, VpadState};

use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputEvent, InputId, KeyCode, UinputAbsSetup};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The device's name; the input threads skip it.
pub const NAME: &str = "mouse-kb-remap virtual controller";
/// xpad's version number for wired Xbox 360 controllers.
const VERSION: u16 = 0x0114;

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

/// XInput button flag → evdev key, as xpad reports them.
const BUTTONS: [(u16, u16); 11] = [
    (0x1000, 0x130), // A: BTN_SOUTH
    (0x2000, 0x131), // B: BTN_EAST
    (0x4000, 0x133), // X: BTN_NORTH (xpad's naming)
    (0x8000, 0x134), // Y: BTN_WEST
    (0x0100, 0x136), // LB: BTN_TL
    (0x0200, 0x137), // RB: BTN_TR
    (0x0020, 0x13a), // Back: BTN_SELECT
    (0x0010, 0x13b), // Start: BTN_START
    (0x0400, 0x13c), // Guide: BTN_MODE
    (0x0040, 0x13d), // LS: BTN_THUMBL
    (0x0080, 0x13e), // RS: BTN_THUMBR
];

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

pub fn start(on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>) {
    vpad::set_events(on_event);
    std::thread::Builder::new()
        .name("remap-vpad".into())
        .spawn(run)
        .expect("failed to spawn virtual controller thread");
}

/// Whether the app may create input devices (`/dev/uinput` writable).
pub fn driver_present() -> bool {
    std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_ok()
}

fn build() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for (_, code) in BUTTONS {
        keys.insert(KeyCode(code));
    }
    let stick = AbsInfo::new(0, -32768, 32767, 16, 128, 0);
    let trigger = AbsInfo::new(0, 0, 255, 0, 0, 0);
    let hat = AbsInfo::new(0, -1, 1, 0, 0, 0);
    let mut b = VirtualDevice::builder()?
        .name(NAME)
        .input_id(InputId::new(BusType::BUS_USB, VENDOR, PRODUCT, VERSION))
        .with_keys(&keys)?;
    for (code, info) in [
        (ABS_X, stick),
        (ABS_Y, stick),
        (ABS_Z, trigger),
        (ABS_RX, stick),
        (ABS_RY, stick),
        (ABS_RZ, trigger),
        (ABS_HAT0X, hat),
        (ABS_HAT0Y, hat),
    ] {
        b = b.with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode(code), info))?;
    }
    b.build()
}

/// The events that take the device from `old` to `new` (all of `new` when
/// there's no `old`).
pub fn events(old: Option<XReport>, new: XReport) -> Vec<InputEvent> {
    let mut out = Vec::new();
    for (flag, code) in BUTTONS {
        let on = new.buttons & flag != 0;
        if old.is_none_or(|o| (o.buttons & flag != 0) != on) {
            out.push(InputEvent::new(EV_KEY, code, on as i32));
        }
    }
    // evdev's y grows downwards (xpad flips it the same way).
    let flip = |v: i16| -(v as i32) - 1;
    let hat = |r: XReport, minus: u16, plus: u16| (r.buttons & plus != 0) as i32 - (r.buttons & minus != 0) as i32;
    let axes = |r: XReport| {
        [
            (ABS_X, r.lx as i32),
            (ABS_Y, flip(r.ly)),
            (ABS_Z, r.lt as i32),
            (ABS_RX, r.rx as i32),
            (ABS_RY, flip(r.ry)),
            (ABS_RZ, r.rt as i32),
            (ABS_HAT0X, hat(r, 0x0004, 0x0008)),
            (ABS_HAT0Y, hat(r, 0x0001, 0x0002)),
        ]
    };
    let before = old.map(axes);
    for (i, (code, v)) in axes(new).into_iter().enumerate() {
        if before.is_none_or(|b| b[i].1 != v) {
            out.push(InputEvent::new(EV_ABS, code, v));
        }
    }
    out
}

struct Plugged {
    dev: VirtualDevice,
    last: Option<XReport>,
    unwanted_since: Option<Instant>,
}

fn run() {
    let mut plugged: Option<Plugged> = None;
    let mut next_try = Instant::now();
    let mut feed = MouseFeed::default();
    loop {
        let (wanted, retry, settings) = vpad::peek(|s| (s.wanted, s.retry, s.mouse));
        if retry {
            vpad::with(|s| s.retry = false);
            next_try = Instant::now();
        }
        let lean = feed.next(if wanted { settings } else { Default::default() });
        let report = vpad::peek(|s| s.report(lean));

        if wanted && plugged.is_none() && Instant::now() >= next_try {
            match build() {
                Ok(dev) => {
                    vpad::with(|s| s.plugged = true);
                    vpad::set_status(VpadState::Plugged { slot: None });
                    plugged = Some(Plugged {
                        dev,
                        last: None,
                        unwanted_since: None,
                    });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::PermissionDenied || e.kind() == std::io::ErrorKind::NotFound =>
                {
                    vpad::set_status(VpadState::NoDriver);
                    next_try = Instant::now() + RETRY;
                }
                Err(e) => {
                    vpad::set_status(VpadState::Failed(format!("can't create it: {e}")));
                    next_try = Instant::now() + RETRY;
                }
            }
        }
        if !wanted && plugged.is_none() {
            vpad::set_status(VpadState::Unplugged);
        }

        let mut unplug = false;
        if let Some(p) = plugged.as_mut() {
            if wanted {
                p.unwanted_since = None;
            } else {
                let since = *p.unwanted_since.get_or_insert_with(Instant::now);
                unplug = since.elapsed() >= LINGER;
            }
            if p.last != Some(report) && !unplug {
                let ev = events(p.last, report);
                match p.dev.emit(&ev) {
                    Ok(()) => p.last = Some(report),
                    Err(e) => {
                        crate::log!("virtual controller: sending failed ({e}), creating it again");
                        unplug = true;
                        next_try = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
        }
        if unplug {
            if let Some(mut p) = plugged.take() {
                let _ = p.dev.emit(&events(p.last, XReport::default()));
            }
            vpad::with(|s| s.plugged = false);
            vpad::set_status(VpadState::Unplugged);
        }

        let timeout = match &plugged {
            Some(_) if lean.is_some() => MOUSE_TICK,
            Some(p) if p.unwanted_since.is_some() || p.last != Some(report) => Duration::from_millis(100),
            Some(_) => Duration::from_secs(60),
            None if wanted => next_try
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(50)),
            None => Duration::from_secs(60),
        };
        vpad::wait(timeout, |s| {
            s.wanted != wanted || s.retry || s.mouse != settings || s.report(lean) != report
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_become_xpad_events() {
        let r = XReport {
            buttons: 0x1000 | 0x0001 | 0x0008,
            lt: 200,
            ly: 32767,
            rx: -5,
            ..Default::default()
        };
        let all = events(None, r);
        let has = |ty: u16, code: u16, v: i32| {
            all.iter()
                .any(|e| e.event_type().0 == ty && e.code() == code && e.value() == v)
        };
        assert!(has(EV_KEY, 0x130, 1)); // A down
        assert!(has(EV_KEY, 0x131, 0)); // B up
        assert!(has(EV_ABS, ABS_Y, -32768)); // stick up is evdev's minimum
        assert!(has(EV_ABS, ABS_Z, 200));
        assert!(has(EV_ABS, ABS_RX, -5));
        assert!(has(EV_ABS, ABS_HAT0X, 1)); // d-pad right
        assert!(has(EV_ABS, ABS_HAT0Y, -1)); // d-pad up
                                             // Only what changed after that.
        let next = XReport {
            buttons: 0x0001 | 0x0008,
            ..r
        };
        let ev = events(Some(r), next);
        assert_eq!(ev.len(), 1);
        assert_eq!((ev[0].code(), ev[0].value()), (0x130, 0));
        // Centred stick: evdev -1 (xpad's flip of 0).
        assert!(events(None, XReport::default())
            .iter()
            .any(|e| e.code() == ABS_Y && e.value() == -1));
    }
}
