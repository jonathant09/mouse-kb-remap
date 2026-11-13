//! Linux input backend (evdev + uinput).
//!
//! * Every keyboard and mouse under `/dev/input` is grabbed (exclusive
//!   access), so the desktop only sees what we forward.
//! * One thread per device reads its events: remapped inputs go through the
//!   shared `Dispatcher`, everything else is passed through unchanged to a
//!   virtual uinput device. Since each device is read on its own, bindings
//!   for one device (keyboards too) need nothing special here.
//! * Game controllers are read but not grabbed (games keep seeing them),
//!   and a ticker moves the pointer with a stick when a profile asks.
//! * A hot-plug thread picks up devices connected later.
//! * The focused app is tracked through X11 (`_NET_ACTIVE_WINDOW` +
//!   `_NET_WM_PID`). Under Wayland only XWayland windows can be seen; when the
//!   focus is unknown the Default profile applies.
//!
//! Needs read access to `/dev/input/event*` and write access to `/dev/uinput`
//! (see docs/troubleshooting.md for the udev rule).

use super::dispatch::{Ctx, Dispatcher, Phase, Sink};
use super::pad::{self, PadReading};
use super::player::{self, Cmd, Out};
use super::{
    match_device, pick_profile, AppCommand, Compiled, CompiledProfile, DeviceIndex, EngineEvent, FocusInfo,
    HotkeyBinding, Input, Layers, RecEvent, WindowInfo,
};
use crate::config::{MouseButton, StickPointer, SystemCommand};

use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode, RelativeAxisCode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

pub const SUPPORTED: bool = true;

const NO_PROFILE: usize = usize::MAX;
const VIRTUAL_NAME: &str = "mouse-kb-remap virtual input";

/// Our key ids use Windows virtual-key codes; this maps them to Linux key codes.
const VK_TO_EVDEV: &[(u16, u16)] = &[
    (0x1B, 1),  // Esc
    (0x70, 59), // F1
    (0x71, 60),
    (0x72, 61),
    (0x73, 62),
    (0x74, 63),
    (0x75, 64),
    (0x76, 65),
    (0x77, 66),
    (0x78, 67),
    (0x79, 68),  // F10
    (0x7A, 87),  // F11
    (0x7B, 88),  // F12
    (0x2C, 99),  // PrintScreen (SysRq)
    (0x91, 70),  // ScrollLock
    (0x13, 119), // Pause
    (0xC0, 41),  // Grave
    (0x31, 2),
    (0x32, 3),
    (0x33, 4),
    (0x34, 5),
    (0x35, 6),
    (0x36, 7),
    (0x37, 8),
    (0x38, 9),
    (0x39, 10),
    (0x30, 11),
    (0xBD, 12),  // Minus
    (0xBB, 13),  // Equal
    (0x08, 14),  // Backspace
    (0x2D, 110), // Insert
    (0x24, 102), // Home
    (0x21, 104), // PageUp
    (0x09, 15),  // Tab
    (0x51, 16),  // Q
    (0x57, 17),
    (0x45, 18),
    (0x52, 19),
    (0x54, 20),
    (0x59, 21),
    (0x55, 22),
    (0x49, 23),
    (0x4F, 24),
    (0x50, 25),  // P
    (0xDB, 26),  // [
    (0xDD, 27),  // ]
    (0xDC, 43),  // Backslash
    (0x2E, 111), // Delete
    (0x23, 107), // End
    (0x22, 109), // PageDown
    (0x14, 58),  // CapsLock
    (0x41, 30),  // A
    (0x53, 31),
    (0x44, 32),
    (0x46, 33),
    (0x47, 34),
    (0x48, 35),
    (0x4A, 36),
    (0x4B, 37),
    (0x4C, 38), // L
    (0xBA, 39), // ;
    (0xDE, 40), // '
    (0x0D, 28), // Enter
    (0xA0, 42), // LShift
    (0x5A, 44), // Z
    (0x58, 45),
    (0x43, 46),
    (0x56, 47),
    (0x42, 48),
    (0x4E, 49),
    (0x4D, 50),  // M
    (0xBC, 51),  // ,
    (0xBE, 52),  // .
    (0xBF, 53),  // /
    (0xA1, 54),  // RShift
    (0x26, 103), // Up
    (0xA2, 29),  // LCtrl
    (0x5B, 125), // LWin (LeftMeta)
    (0xA4, 56),  // LAlt
    (0x20, 57),  // Space
    (0xA5, 100), // RAlt
    (0x5C, 126), // RWin
    (0x5D, 127), // Menu (Compose)
    (0xA3, 97),  // RCtrl
    (0x25, 105), // Left
    (0x28, 108), // Down
    (0x27, 106), // Right
    (0x90, 69),  // NumLock
    (0x6F, 98),  // KP /
    (0x6A, 55),  // KP *
    (0x6D, 74),  // KP -
    (0x67, 71),  // KP 7
    (0x68, 72),
    (0x69, 73),
    (0x6B, 78), // KP +
    (0x64, 75), // KP 4
    (0x65, 76),
    (0x66, 77),
    (0x61, 79), // KP 1
    (0x62, 80),
    (0x63, 81),
    (crate::keys::VK_NUMPAD_ENTER, 96),
    (0x60, 82),  // KP 0
    (0x6E, 83),  // KP .
    (0x7C, 183), // F13
    (0x7D, 184),
    (0x7E, 185),
    (0x7F, 186),
    (0x80, 187),
    (0x81, 188),
    (0x82, 189),
    (0x83, 190),
    (0x84, 191),
    (0x85, 192),
    (0x86, 193),
    (0x87, 194), // F24
    (0xB3, 164), // Play/Pause
    (0xB0, 163), // Next
    (0xB1, 165), // Previous
    (0xB2, 166), // Stop
    (0xAF, 115), // Volume up
    (0xAE, 114), // Volume down
    (0xAD, 113), // Mute
    (0xA6, 158), // Browser back
    (0xA7, 159), // Browser forward
    (0xA8, 173), // Refresh
    (0xA9, 128), // Stop
    (0xAA, 217), // Search
    (0xAB, 156), // Bookmarks
    (0xAC, 172), // Home page
    (0xB4, 155), // Mail
    (0xB5, 226), // Media
    (0xB6, 157), // Computer
    (0xB7, 140), // Calculator
];

const BUTTONS: &[(MouseButton, u16)] = &[
    (MouseButton::Left, 0x110),
    (MouseButton::Right, 0x111),
    (MouseButton::Middle, 0x112),
    (MouseButton::Back, 0x113),    // BTN_SIDE
    (MouseButton::Forward, 0x114), // BTN_EXTRA
];

fn vk_to_code(vk: u16) -> Option<u16> {
    VK_TO_EVDEV.iter().find(|(v, _)| *v == vk).map(|(_, c)| *c)
}

fn code_to_vk(code: u16) -> Option<u16> {
    VK_TO_EVDEV.iter().find(|(_, c)| *c == code).map(|(v, _)| *v)
}

fn button_code(b: MouseButton) -> Option<u16> {
    BUTTONS.iter().find(|(x, _)| *x == b).map(|(_, c)| *c)
}

fn code_to_button(code: u16) -> Option<MouseButton> {
    BUTTONS.iter().find(|(_, c)| *c == code).map(|(b, _)| *b)
}

/// Controller buttons (evdev code → `PadReading` bit). Codes as the xpad
/// driver uses them; some drivers swap X and Y.
const PAD_KEYS: &[(u16, u16)] = &[
    (0x130, pad::PAD_A),     // BTN_SOUTH
    (0x131, pad::PAD_B),     // BTN_EAST
    (0x133, pad::PAD_X),     // BTN_X / BTN_NORTH
    (0x134, pad::PAD_Y),     // BTN_Y / BTN_WEST
    (0x136, pad::PAD_LB),    // BTN_TL
    (0x137, pad::PAD_RB),    // BTN_TR
    (0x13a, pad::PAD_BACK),  // BTN_SELECT
    (0x13b, pad::PAD_START), // BTN_START
    (0x13d, pad::PAD_LS),    // BTN_THUMBL
    (0x13e, pad::PAD_RS),    // BTN_THUMBR
    (0x220, pad::PAD_UP),    // BTN_DPAD_UP
    (0x221, pad::PAD_DOWN),
    (0x222, pad::PAD_LEFT),
    (0x223, pad::PAD_RIGHT),
];
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const REL_WHEEL_HI_RES: u16 = 0x0b;
const REL_HWHEEL_HI_RES: u16 = 0x0c;

struct Recording {
    start: Instant,
    events: Vec<(Duration, RecEvent)>,
}

#[derive(Default)]
struct CaptureState {
    mods: u8,
    lone_mod: Option<u16>,
    swallow_up: Vec<u16>,
}

struct Shared {
    profiles: RwLock<Vec<CompiledProfile>>,
    hotkeys: RwLock<Arc<Vec<HotkeyBinding>>>,
    active: RwLock<Option<Arc<Layers>>>,
    active_index: AtomicUsize,
    forced: Mutex<Option<usize>>,
    enabled: AtomicBool,
    self_pid: u32,
    /// (focused window, exe file name, is_self)
    last_focus: Mutex<(WindowInfo, String, bool)>,
    focus_self: AtomicBool,
    on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>,
    worker: Sender<Cmd>,
    out: Mutex<Option<evdev::uinput::VirtualDevice>>,
    dispatch: Mutex<Dispatcher>,
    capture: AtomicBool,
    capture_state: Mutex<CaptureState>,
    recording: AtomicBool,
    record: Mutex<Option<Recording>>,
    rec_down: Mutex<Vec<u16>>,
    /// Pointer speed while a sensitivity button is held (0 = normal).
    /// Applied by scaling relative motion we forward: a true DPI shift.
    sensitivity: AtomicU32,
    /// Named devices, and a counter bumped when they change.
    devices: RwLock<Vec<crate::config::Device>>,
    devices_gen: AtomicU32,
    device_capture: AtomicBool,
    /// Sum of all relative mouse motion: where the pointer is, as far as
    /// gestures need to know.
    pointer: (AtomicI32, AtomicI32),
    /// The last controller reading, for moving the pointer with a stick.
    stick: Mutex<PadReading>,
    stick_speed: AtomicU32,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get().expect("engine not started")
}

fn emit_raw(events: &[InputEvent]) {
    if events.is_empty() {
        return;
    }
    if let Ok(mut out) = shared().out.lock() {
        if let Some(dev) = out.as_mut() {
            if let Err(e) = dev.emit(events) {
                crate::log!("engine: uinput write failed: {e}");
            }
        }
    }
}

pub struct Engine;

impl Engine {
    pub fn start(compiled: Compiled, enabled: bool, on_event: Box<dyn Fn(EngineEvent) + Send + Sync>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let out = match build_virtual_device() {
            Ok(d) => Some(d),
            Err(e) => {
                crate::log!("engine: can't create the virtual device (/dev/uinput): {e}");
                None
            }
        };
        let have_output = out.is_some();
        let _ = SHARED.set(Shared {
            profiles: RwLock::new(compiled.profiles),
            hotkeys: RwLock::new(Arc::new(compiled.hotkeys)),
            active: RwLock::new(None),
            active_index: AtomicUsize::new(NO_PROFILE),
            forced: Mutex::new(None),
            enabled: AtomicBool::new(enabled),
            self_pid: std::process::id(),
            last_focus: Mutex::new((WindowInfo::default(), String::new(), false)),
            focus_self: AtomicBool::new(false),
            on_event: Arc::from(on_event),
            worker: tx,
            out: Mutex::new(out),
            dispatch: Mutex::new(Dispatcher::default()),
            capture: AtomicBool::new(false),
            capture_state: Mutex::new(CaptureState::default()),
            recording: AtomicBool::new(false),
            record: Mutex::new(None),
            rec_down: Mutex::new(Vec::new()),
            sensitivity: AtomicU32::new(0),
            devices: RwLock::new(compiled.devices),
            devices_gen: AtomicU32::new(0),
            device_capture: AtomicBool::new(false),
            pointer: (AtomicI32::new(0), AtomicI32::new(0)),
            stick: Mutex::new(PadReading::default()),
            stick_speed: AtomicU32::new(compiled.stick_speed),
        });
        recompute_active(&WindowInfo::default(), false);

        std::thread::Builder::new()
            .name("remap-player".into())
            .spawn(move || player::run_worker(rx, emit_outs))
            .expect("failed to spawn player thread");
        std::thread::Builder::new()
            .name("remap-ticker".into())
            .spawn(ticker)
            .expect("failed to spawn ticker thread");
        if have_output {
            std::thread::Builder::new()
                .name("remap-stick".into())
                .spawn(stick_thread)
                .expect("failed to spawn stick thread");
        }
        // Without an output device, grabbing would swallow all input.
        if have_output {
            std::thread::Builder::new()
                .name("remap-devices".into())
                .spawn(device_scanner)
                .expect("failed to spawn device thread");
        }
        std::thread::Builder::new()
            .name("remap-focus".into())
            .spawn(focus_thread)
            .expect("failed to spawn focus thread");
        Engine
    }

    pub fn apply(&self, compiled: Compiled, enabled: bool) {
        let sh = shared();
        sh.enabled.store(enabled, Ordering::Relaxed);
        if let Ok(mut p) = sh.profiles.write() {
            *p = compiled.profiles;
            if let Ok(mut f) = sh.forced.lock() {
                if f.is_some_and(|i| i >= p.len()) {
                    *f = None;
                }
            }
        }
        if let Ok(mut h) = sh.hotkeys.write() {
            *h = Arc::new(compiled.hotkeys);
        }
        if let Ok(mut d) = sh.devices.write() {
            *d = compiled.devices;
        }
        sh.devices_gen.fetch_add(1, Ordering::SeqCst);
        sh.stick_speed.store(compiled.stick_speed, Ordering::SeqCst);
        profile_changed_elsewhere();
    }

    pub fn capture_device(&self, on: bool) {
        shared().device_capture.store(on, Ordering::SeqCst);
    }

    pub fn force_profile(&self, index: Option<usize>) {
        if let Ok(mut f) = shared().forced.lock() {
            *f = index;
        }
        profile_changed_elsewhere();
        emit_focus();
    }

    pub fn elevated(&self) -> bool {
        false
    }

    pub fn start_capture(&self) -> bool {
        shared().capture.store(true, Ordering::SeqCst);
        true
    }

    pub fn cancel_capture(&self) {
        shared().capture.store(false, Ordering::SeqCst);
    }

    pub fn start_recording(&self) -> bool {
        let sh = shared();
        if let Ok(mut r) = sh.record.lock() {
            *r = Some(Recording {
                start: Instant::now(),
                events: Vec::new(),
            });
        }
        sh.recording.store(true, Ordering::SeqCst);
        true
    }

    pub fn stop_recording(&self) -> Vec<(Duration, RecEvent)> {
        let sh = shared();
        sh.recording.store(false, Ordering::SeqCst);
        sh.record
            .lock()
            .ok()
            .and_then(|mut r| r.take())
            .map(|r| r.events)
            .unwrap_or_default()
    }

    pub fn shutdown(&self) {
        let _ = shared().worker.send(Cmd::StopAll);
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn build_virtual_device() -> std::io::Result<evdev::uinput::VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for (_, code) in VK_TO_EVDEV {
        keys.insert(KeyCode(*code));
    }
    // Everything else a real keyboard might send, so pass-through works.
    for code in 1..0x2ff {
        keys.insert(KeyCode(code));
    }
    for (_, code) in BUTTONS {
        keys.insert(KeyCode(*code));
    }
    let mut rel = AttributeSet::<RelativeAxisCode>::new();
    for code in [REL_X, REL_Y, REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES] {
        rel.insert(RelativeAxisCode(code));
    }
    evdev::uinput::VirtualDevice::builder()?
        .name(VIRTUAL_NAME)
        .with_keys(&keys)?
        .with_relative_axes(&rel)?
        .build()
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Keyboard or mouse: grabbed, events go through the dispatcher.
    Grab,
    /// Game controller: read only, games keep seeing it.
    Pad,
}

/// What to do with a device. Keyboards and mice are grabbed; game
/// controllers are read; touchpads, tablets and joysticks are left alone.
fn wanted(dev: &Device) -> Option<Kind> {
    if dev.name() == Some(VIRTUAL_NAME) {
        return None;
    }
    let keys = dev.supported_keys();
    if keys.is_some_and(|k| k.contains(KeyCode(0x130)) && k.contains(KeyCode(0x131))) {
        return Some(Kind::Pad);
    }
    if dev.supported_absolute_axes().is_some_and(|a| a.iter().next().is_some()) {
        return None;
    }
    let keyboard = keys.is_some_and(|k| k.contains(KeyCode::KEY_A) && k.contains(KeyCode::KEY_SPACE));
    let mouse = keys.is_some_and(|k| k.contains(KeyCode::BTN_LEFT))
        && dev
            .supported_relative_axes()
            .is_some_and(|r| r.contains(RelativeAxisCode::REL_X));
    (keyboard || mouse).then_some(Kind::Grab)
}

/// `Device::id` for an evdev device: vendor, product and name.
fn device_id(dev: &Device) -> String {
    let id = dev.input_id();
    format!(
        "evdev:{:04x}:{:04x}:{}",
        id.vendor(),
        id.product(),
        dev.name().unwrap_or("")
    )
}

fn device_scanner() {
    let grabbed: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    loop {
        for (path, mut dev) in evdev::enumerate() {
            if grabbed.lock().map(|g| g.contains(&path)).unwrap_or(true) {
                continue;
            }
            let Some(kind) = wanted(&dev) else { continue };
            if kind == Kind::Grab {
                if let Err(e) = dev.grab() {
                    crate::log!("engine: can't grab {}: {e}", path.display());
                    continue;
                }
            }
            crate::log!("engine: using {} ({})", path.display(), dev.name().unwrap_or("?"));
            if let Ok(mut g) = grabbed.lock() {
                g.insert(path.clone());
            }
            let grabbed = grabbed.clone();
            std::thread::spawn(move || {
                match kind {
                    Kind::Grab => device_loop(dev),
                    Kind::Pad => pad_loop(dev),
                }
                // Unplugged (or failed): allow a re-grab when it comes back.
                if let Ok(mut g) = grabbed.lock() {
                    g.remove(&path);
                }
            });
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Which named device a device thread reads, kept up to date with the
/// device list.
struct DeviceMatch {
    id: String,
    name: String,
    generation: u32,
    index: Option<DeviceIndex>,
}

impl DeviceMatch {
    fn new(dev: &Device) -> Self {
        DeviceMatch {
            id: device_id(dev),
            name: dev.name().unwrap_or("Input device").to_string(),
            generation: u32::MAX,
            index: None,
        }
    }

    fn index(&mut self) -> Option<DeviceIndex> {
        let sh = shared();
        let generation = sh.devices_gen.load(Ordering::SeqCst);
        if generation != self.generation {
            self.generation = generation;
            self.index = sh.devices.read().ok().and_then(|d| match_device(&d, &self.id));
        }
        self.index
    }

    /// A press while a device is being added: report this one.
    fn pressed(&self) {
        let sh = shared();
        if sh.device_capture.swap(false, Ordering::SeqCst) {
            (sh.on_event)(EngineEvent::DeviceCaptured {
                id: self.id.clone(),
                name: self.name.clone(),
            });
        }
    }
}

/// Fractional remainder of scaled pointer motion, per axis.
#[derive(Default)]
struct MotionScale {
    rest_x: f32,
    rest_y: f32,
}

fn device_loop(mut dev: Device) {
    let mut batch: Vec<InputEvent> = Vec::new();
    let mut scale = MotionScale::default();
    let mut device = DeviceMatch::new(&dev);
    let sh = shared();
    loop {
        let events: Vec<InputEvent> = match dev.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) => {
                crate::log!("engine: device read failed: {e}");
                return;
            }
        };
        for ev in events {
            let (ty, code, value) = (ev.event_type().0, ev.code(), ev.value());
            if ty == EV_SYN && code == SYN_REPORT {
                emit_raw(&batch);
                batch.clear();
                continue;
            }
            if ty == EV_SYN {
                continue;
            }
            if ty == EV_REL && code == REL_X {
                sh.pointer.0.fetch_add(value, Ordering::Relaxed);
            } else if ty == EV_REL && code == REL_Y {
                sh.pointer.1.fetch_add(value, Ordering::Relaxed);
            } else if ty == EV_KEY && value == 1 {
                device.pressed();
            }
            if !process_event(ty, code, value, &mut device) {
                batch.push(scale_motion(ev, &mut scale));
            }
        }
    }
}

/// Map an absolute axis value to -32767..32767 (or 0..255 for triggers).
fn scale_axis(value: i32, min: i32, max: i32, trigger: bool) -> i32 {
    let span = (max - min).max(1) as f32;
    let t = (value - min) as f32 / span;
    if trigger {
        (t * 255.0).round().clamp(0.0, 255.0) as i32
    } else {
        ((t * 2.0 - 1.0) * 32767.0).round().clamp(-32767.0, 32767.0) as i32
    }
}

/// A game controller: build up a reading from its events, and at each
/// report turn the changes into inputs. Not grabbed, so nothing is
/// swallowed: games keep seeing the controller.
fn pad_loop(mut dev: Device) {
    let mut ranges: Vec<(u16, i32, i32)> = Vec::new();
    if let Ok(infos) = dev.get_absinfo() {
        for (axis, info) in infos {
            ranges.push((axis.0, info.minimum(), info.maximum()));
        }
    }
    let range = |code: u16| {
        ranges
            .iter()
            .find(|(c, _, _)| *c == code)
            .map(|(_, a, b)| (*a, *b))
            .unwrap_or((-32768, 32767))
    };
    let mut device = DeviceMatch::new(&dev);
    let mut reading = PadReading::default();
    let mut mask = 0u32;
    let sh = shared();
    loop {
        let events: Vec<InputEvent> = match dev.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) => {
                crate::log!("engine: controller read failed: {e}");
                // Unplugged: release whatever was held.
                for (b, _) in pad::changes(mask, 0) {
                    dispatch(Input::Pad(b), Phase::Up, device.index());
                }
                return;
            }
        };
        for ev in events {
            let (ty, code, value) = (ev.event_type().0, ev.code(), ev.value());
            match ty {
                EV_KEY => {
                    if value == 1 {
                        device.pressed();
                    }
                    if let Some((_, bit)) = PAD_KEYS.iter().find(|(c, _)| *c == code) {
                        if value != 0 {
                            reading.buttons |= bit;
                        } else {
                            reading.buttons &= !bit;
                        }
                    } else if code == BTN_TL2 {
                        reading.lt = if value != 0 { 255 } else { 0 };
                    } else if code == BTN_TR2 {
                        reading.rt = if value != 0 { 255 } else { 0 };
                    }
                }
                EV_ABS => {
                    let (min, max) = range(code);
                    match code {
                        ABS_X => reading.lx = scale_axis(value, min, max, false) as i16,
                        // evdev's y grows downwards; a reading's grows up.
                        ABS_Y => reading.ly = -scale_axis(value, min, max, false) as i16,
                        ABS_RX => reading.rx = scale_axis(value, min, max, false) as i16,
                        ABS_RY => reading.ry = -scale_axis(value, min, max, false) as i16,
                        ABS_Z => reading.lt = scale_axis(value, min, max, true) as u8,
                        ABS_RZ => reading.rt = scale_axis(value, min, max, true) as u8,
                        ABS_HAT0X | ABS_HAT0Y => {
                            let (neg, pos) = if code == ABS_HAT0X {
                                (pad::PAD_LEFT, pad::PAD_RIGHT)
                            } else {
                                (pad::PAD_UP, pad::PAD_DOWN)
                            };
                            reading.buttons &= !(neg | pos);
                            if value < 0 {
                                reading.buttons |= neg;
                            } else if value > 0 {
                                reading.buttons |= pos;
                            }
                        }
                        _ => {}
                    }
                }
                EV_SYN if code == SYN_REPORT => {
                    if let Ok(mut s) = sh.stick.lock() {
                        *s = reading;
                    }
                    let next = pad::pad_mask(&reading, mask);
                    for (b, down) in pad::changes(mask, next) {
                        dispatch(
                            Input::Pad(b),
                            if down { Phase::Down } else { Phase::Up },
                            device.index(),
                        );
                    }
                    mask = next;
                }
                _ => {}
            }
        }
    }
}

/// Moves the pointer with a controller stick while the profile asks for it.
fn stick_thread() {
    let sh = shared();
    let mut rest = (0.0f32, 0.0f32);
    let mut last = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(4));
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().min(0.05);
        last = now;
        let stick = sh
            .active
            .read()
            .ok()
            .and_then(|a| a.as_ref().map(|l| l.stick_pointer))
            .unwrap_or_default();
        if stick == StickPointer::Off || !sh.enabled.load(Ordering::Relaxed) {
            continue;
        }
        let r = sh.stick.lock().map(|s| *s).unwrap_or_default();
        let (x, y) = if stick == StickPointer::Left {
            (r.lx, r.ly)
        } else {
            (r.rx, r.ry)
        };
        let (dx, dy) = pad::stick_move(x, y, sh.stick_speed.load(Ordering::Relaxed), dt, &mut rest);
        if dx != 0 || dy != 0 {
            let mut events = Vec::new();
            out_to_events(
                &Out::Move {
                    x: dx,
                    y: dy,
                    absolute: false,
                },
                &mut events,
            );
            emit_raw(&events);
        }
    }
}

fn scale_motion(ev: InputEvent, scale: &mut MotionScale) -> InputEvent {
    let speed = shared().sensitivity.load(Ordering::Relaxed);
    if speed == 0 || ev.event_type() != EventType::RELATIVE || !(ev.code() == REL_X || ev.code() == REL_Y) {
        return ev;
    }
    let factor = speed as f32 / 10.0;
    let rest = if ev.code() == REL_X {
        &mut scale.rest_x
    } else {
        &mut scale.rest_y
    };
    let exact = ev.value() as f32 * factor + *rest;
    let whole = exact.trunc();
    *rest = exact - whole;
    InputEvent::new(EV_REL, ev.code(), whole as i32)
}

/// Returns true if the event was consumed.
fn process_event(ty: u16, code: u16, value: i32, device: &mut DeviceMatch) -> bool {
    let sh = shared();
    let (input, phase) = match ty {
        EV_KEY => {
            let input = if let Some(b) = code_to_button(code) {
                Input::Mouse(b)
            } else if let Some(vk) = code_to_vk(code) {
                Input::Key(vk)
            } else {
                return false;
            };
            (input, if value == 0 { Phase::Up } else { Phase::Down })
        }
        EV_REL => {
            let (horizontal, hi_res) = match code {
                REL_WHEEL => (false, false),
                REL_HWHEEL => (true, false),
                REL_WHEEL_HI_RES => (false, true),
                REL_HWHEEL_HI_RES => (true, true),
                _ => return false,
            };
            if value == 0 {
                return false;
            }
            let wheel = super::dispatch::wheel_for(value, horizontal);
            let bound = sh
                .active
                .read()
                .ok()
                .and_then(|a| a.clone())
                .is_some_and(|l| l.binds(&Input::Mouse(wheel)));
            if hi_res {
                // The matching low-res event carries the step; drop the
                // hi-res one only if the wheel is remapped.
                return bound && sh.enabled.load(Ordering::Relaxed);
            }
            (Input::Mouse(wheel), Phase::Tap)
        }
        _ => return false,
    };

    if let Input::Key(vk) = input {
        if capture_key(vk, phase != Phase::Up, value == 2) {
            return true;
        }
        if sh.recording.load(Ordering::Relaxed) {
            let mut down = sh.rec_down.lock().unwrap_or_else(|e| e.into_inner());
            let was_down = down.contains(&vk);
            let is_down = phase != Phase::Up;
            if is_down && !was_down {
                down.push(vk);
            } else if !is_down {
                down.retain(|v| *v != vk);
            }
            drop(down);
            if !(is_down && was_down) {
                record(RecEvent::Key { vk, down: is_down });
            }
            if sh.focus_self.load(Ordering::Relaxed) {
                return true;
            }
        }
    } else if sh.recording.load(Ordering::Relaxed) {
        if let Input::Mouse(button) = input {
            record(match phase {
                Phase::Tap => RecEvent::Wheel(button),
                p => RecEvent::Button {
                    button,
                    down: p == Phase::Down,
                },
            });
        }
    }
    dispatch(input, phase, device.index())
}

/// Run one input through the dispatcher. Returns true if it was consumed.
fn dispatch(input: Input, phase: Phase, device: Option<DeviceIndex>) -> bool {
    let sh = shared();
    let active = sh.active.read().ok().and_then(|a| a.clone());
    let hotkeys = sh.hotkeys.read().map(|h| h.clone()).unwrap_or_default();
    let ctx = Ctx {
        layers: active.as_deref(),
        enabled: sh.enabled.load(Ordering::Relaxed),
        hotkeys: &hotkeys,
        now: Instant::now(),
        device,
    };
    let mut sink = UinputSink { events: Vec::new(), sh };
    let mut d = sh.dispatch.lock().unwrap_or_else(|e| e.into_inner());
    let swallow = d.handle(input, phase, &ctx, &mut sink);
    sink.flush();
    swallow
}

/// Finishes tap / hold decisions when their time is up.
fn ticker() {
    let sh = shared();
    loop {
        let deadline = sh.dispatch.lock().ok().and_then(|d| d.deadline());
        std::thread::sleep(match deadline {
            Some(at) => at
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5)),
            None => Duration::from_millis(20),
        });
        if deadline.is_some_and(|at| Instant::now() >= at) {
            let mut sink = UinputSink { events: Vec::new(), sh };
            let mut d = sh.dispatch.lock().unwrap_or_else(|e| e.into_inner());
            d.tick(Instant::now(), &mut sink);
            sink.flush();
        }
    }
}

fn record(ev: RecEvent) {
    if let Ok(mut r) = shared().record.lock() {
        if let Some(r) = r.as_mut() {
            if r.events.len() < 5000 {
                r.events.push((r.start.elapsed(), ev));
            }
        }
    }
}

/// Same rules as the Windows capture: modifiers wait for a key, a lone
/// modifier is captured on release.
fn capture_key(vk: u16, down: bool, repeat: bool) -> bool {
    let sh = shared();
    let capturing = sh.capture.load(Ordering::SeqCst);
    let mut c = sh.capture_state.lock().unwrap_or_else(|e| e.into_inner());
    if !down {
        if capturing {
            if c.lone_mod == Some(vk) {
                sh.capture.store(false, Ordering::SeqCst);
                (sh.on_event)(EngineEvent::Captured { vk, mods: 0 });
                c.mods = 0;
                c.lone_mod = None;
            } else if let Some(m) = crate::keys::modifier_of(vk) {
                c.mods &= !m.bit();
            }
        }
        if let Some(i) = c.swallow_up.iter().position(|v| *v == vk) {
            c.swallow_up.remove(i);
            return true;
        }
        return false;
    }
    if repeat && c.swallow_up.contains(&vk) {
        return true;
    }
    if !capturing {
        c.mods = 0;
        c.lone_mod = None;
        return false;
    }
    if let Some(m) = crate::keys::modifier_of(vk) {
        c.mods |= m.bit();
        c.lone_mod = Some(vk);
    } else {
        sh.capture.store(false, Ordering::SeqCst);
        let mods = c.mods;
        c.mods = 0;
        c.lone_mod = None;
        (sh.on_event)(EngineEvent::Captured { vk, mods });
    }
    if !c.swallow_up.contains(&vk) {
        c.swallow_up.push(vk);
    }
    true
}

struct UinputSink<'a> {
    events: Vec<InputEvent>,
    sh: &'a Shared,
}

impl Sink for UinputSink<'_> {
    fn out(&mut self, out: Out) {
        out_to_events(&out, &mut self.events);
    }

    fn flush(&mut self) {
        // Each key change is its own report, like a real keyboard.
        for ev in self.events.drain(..) {
            emit_raw(&[ev]);
        }
    }

    fn worker(&mut self, cmd: Cmd) {
        let _ = self.sh.worker.send(cmd);
    }

    fn command(&mut self, cmd: AppCommand) {
        (self.sh.on_event)(EngineEvent::Command(cmd));
    }

    fn notice(&mut self, text: String) {
        (self.sh.on_event)(EngineEvent::Notice(text));
    }

    fn pointer(&mut self) -> Option<(i32, i32)> {
        Some((
            self.sh.pointer.0.load(Ordering::Relaxed),
            self.sh.pointer.1.load(Ordering::Relaxed),
        ))
    }
}

fn out_to_events(out: &Out, events: &mut Vec<InputEvent>) {
    match *out {
        Out::Key { vk, down } => {
            if let Some(code) = vk_to_code(vk) {
                events.push(InputEvent::new(EV_KEY, code, down as i32));
            }
        }
        Out::Button { button, down } => {
            if let Some(code) = button_code(button) {
                events.push(InputEvent::new(EV_KEY, code, down as i32));
            } else if down {
                push_wheel(button, events);
            }
        }
        Out::Wheel(b) => push_wheel(b, events),
        Out::Char(c) => {
            if let Some((code, shift)) = ascii_key(c) {
                if shift {
                    events.push(InputEvent::new(EV_KEY, 42, 1));
                }
                events.push(InputEvent::new(EV_KEY, code, 1));
                events.push(InputEvent::new(EV_KEY, code, 0));
                if shift {
                    events.push(InputEvent::new(EV_KEY, 42, 0));
                }
            }
        }
        Out::Move { x, y, absolute: false } => {
            if x != 0 {
                events.push(InputEvent::new(EV_REL, REL_X, x));
            }
            if y != 0 {
                events.push(InputEvent::new(EV_REL, REL_Y, y));
            }
        }
        Out::Move { absolute: true, .. } => crate::log!("macro: \"move to\" isn't supported on Linux yet"),
        Out::Launch(_) | Out::PointerSpeed(_) | Out::System(_) | Out::Notice(_) => {}
    }
}

fn push_wheel(b: MouseButton, events: &mut Vec<InputEvent>) {
    let (code, hi, v) = match b {
        MouseButton::WheelUp => (REL_WHEEL, REL_WHEEL_HI_RES, 1),
        MouseButton::WheelDown => (REL_WHEEL, REL_WHEEL_HI_RES, -1),
        MouseButton::WheelRight => (REL_HWHEEL, REL_HWHEEL_HI_RES, 1),
        _ => (REL_HWHEEL, REL_HWHEEL_HI_RES, -1),
    };
    events.push(InputEvent::new(EV_REL, code, v));
    events.push(InputEvent::new(EV_REL, hi, v * 120));
}

/// Text output on Linux: printable ASCII on a US layout (uinput has no
/// Unicode input).
fn ascii_key(c: char) -> Option<(u16, bool)> {
    const ROW: &[(char, char, u16)] = &[
        ('1', '!', 2),
        ('2', '@', 3),
        ('3', '#', 4),
        ('4', '$', 5),
        ('5', '%', 6),
        ('6', '^', 7),
        ('7', '&', 8),
        ('8', '*', 9),
        ('9', '(', 10),
        ('0', ')', 11),
        ('-', '_', 12),
        ('=', '+', 13),
        ('[', '{', 26),
        (']', '}', 27),
        ('\\', '|', 43),
        (';', ':', 39),
        ('\'', '"', 40),
        ('`', '~', 41),
        (',', '<', 51),
        ('.', '>', 52),
        ('/', '?', 53),
        (' ', ' ', 57),
        ('\n', '\n', 28),
        ('\t', '\t', 15),
    ];
    if c.is_ascii_alphabetic() {
        let vk = c.to_ascii_uppercase() as u16;
        return vk_to_code(vk).map(|code| (code, c.is_ascii_uppercase()));
    }
    ROW.iter().find_map(|(plain, shifted, code)| {
        if c == *plain {
            Some((*code, false))
        } else if c == *shifted {
            Some((*code, true))
        } else {
            None
        }
    })
}

/// Called on the player thread with a batch of outputs.
fn emit_outs(outs: &[Out]) {
    let mut batch = Vec::new();
    for out in outs {
        match out {
            Out::Launch(spec) => launch(spec),
            Out::PointerSpeed(speed) => shared().sensitivity.store(speed.unwrap_or(0), Ordering::Relaxed),
            Out::Notice(text) => (shared().on_event)(EngineEvent::Notice(text.clone())),
            Out::System(SystemCommand::NextProfile) => {
                (shared().on_event)(EngineEvent::Command(AppCommand::NextProfile))
            }
            Out::System(SystemCommand::AutoProfile) => {
                (shared().on_event)(EngineEvent::Command(AppCommand::AutoProfile))
            }
            Out::System(c) => crate::log!("system: \"{}\" isn't supported on Linux yet", c.name()),
            Out::Move { .. } => {
                // A move is one report.
                out_to_events(out, &mut batch);
                emit_raw(&batch);
                batch.clear();
            }
            other => {
                out_to_events(other, &mut batch);
                for ev in batch.drain(..) {
                    emit_raw(&[ev]);
                }
            }
        }
    }
}

fn launch(spec: &player::LaunchSpec) {
    let path = spec.path.clone();
    let args: Vec<String> = spec.args.split_whitespace().map(String::from).collect();
    let is_program = std::fs::metadata(&path).is_ok_and(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    });
    let result = if is_program || (!path.contains('/') && !path.contains(':')) {
        std::process::Command::new(&path).args(&args).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&path).spawn()
    };
    if let Err(e) = result {
        crate::log!("launch: could not start {path:?}: {e}");
    }
}

// ---------------------------------------------------------------------------
// Focus tracking (X11)
// ---------------------------------------------------------------------------

fn recompute_active(window: &WindowInfo, is_self: bool) -> usize {
    let sh = shared();
    let (index, layers) = if is_self {
        (NO_PROFILE, None)
    } else {
        let profiles = match sh.profiles.read() {
            Ok(p) => p,
            Err(_) => return NO_PROFILE,
        };
        let forced = sh.forced.lock().ok().and_then(|f| *f).filter(|i| *i < profiles.len());
        let index = forced.unwrap_or_else(|| pick_profile(&profiles, window));
        (index, profiles.get(index).map(|p| p.layers.clone()))
    };
    if let Ok(mut a) = sh.active.write() {
        *a = layers;
    }
    sh.active_index.store(index, Ordering::Relaxed);
    index
}

/// Profiles or the forced profile changed: pick again, stop playback, and
/// turn toggled layers off.
fn profile_changed_elsewhere() {
    let sh = shared();
    let (window, _, is_self) = sh.last_focus.lock().map(|f| f.clone()).unwrap_or_default();
    recompute_active(&window, is_self);
    sh.dispatch.lock().unwrap_or_else(|e| e.into_inner()).reset_layers();
    let _ = sh.worker.send(Cmd::StopAll);
}

fn emit_focus() {
    let sh = shared();
    let (_, exe, is_self) = sh.last_focus.lock().map(|f| f.clone()).unwrap_or_default();
    let index = sh.active_index.load(Ordering::Relaxed);
    let forced = sh.forced.lock().map(|f| f.is_some()).unwrap_or(false);
    (sh.on_event)(EngineEvent::Focus(FocusInfo {
        exe,
        is_self,
        elevation_blocked: false,
        profile: (index != NO_PROFILE).then_some(index),
        forced: forced && !is_self,
    }));
}

fn on_focus(pid: u32) {
    let sh = shared();
    let is_self = pid == sh.self_pid;
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    let exe = path
        .as_ref()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
        .unwrap_or_default();
    // Window titles and fullscreen aren't tracked on Linux yet.
    let window = WindowInfo {
        exe: exe.to_lowercase(),
        path: path.map(|p| p.display().to_string().to_lowercase()).unwrap_or_default(),
        title: String::new(),
        fullscreen: false,
    };
    if let Ok(mut f) = sh.last_focus.lock() {
        *f = (window.clone(), exe, is_self);
    }
    sh.focus_self.store(is_self, Ordering::Relaxed);
    if !is_self {
        sh.capture.store(false, Ordering::SeqCst);
    }
    let before = sh.active_index.load(Ordering::Relaxed);
    if recompute_active(&window, is_self) != before {
        let _ = sh.worker.send(Cmd::StopAll);
        sh.dispatch.lock().unwrap_or_else(|e| e.into_inner()).reset_layers();
    }
    emit_focus();
}

fn focus_thread() {
    if let Err(e) = watch_x11_focus() {
        crate::log!("engine: can't track the focused window ({e}); the Default profile applies everywhere");
    }
}

fn watch_x11_focus() -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt, EventMask};
    use x11rb::protocol::Event;

    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let active_atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
    let pid_atom = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    conn.flush()?;

    let mut last_pid = 0u32;
    loop {
        let window = conn
            .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)?
            .reply()?
            .value32()
            .and_then(|mut v| v.next())
            .unwrap_or(0);
        if window != 0 {
            let pid = conn
                .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().and_then(|mut v| v.next()))
                .unwrap_or(0);
            if pid != 0 && pid != last_pid {
                last_pid = pid;
                on_focus(pid);
            }
        }
        // Wait for the next change of the active window.
        loop {
            if let Event::PropertyNotify(e) = conn.wait_for_event()? {
                if e.atom == active_atom {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_a_linux_code() {
        for k in crate::keys::KEYS {
            assert!(vk_to_code(k.vk).is_some(), "{} has no evdev code", k.id);
        }
        for b in MouseButton::ALL.iter().filter(|b| !b.is_wheel()) {
            assert!(button_code(*b).is_some());
        }
    }

    #[test]
    fn ascii_text_maps_to_keys() {
        assert_eq!(ascii_key('a'), Some((30, false)));
        assert_eq!(ascii_key('A'), Some((30, true)));
        assert_eq!(ascii_key('?'), Some((53, true)));
        assert_eq!(ascii_key('é'), None);
    }
}
