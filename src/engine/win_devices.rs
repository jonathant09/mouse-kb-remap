//! Windows: which device input came from, and game controllers.
//!
//! * **Raw input thread.** Low-level hooks don't say which mouse an event
//!   came from; raw input does, and for mice it still arrives when the hook
//!   blocked the event. While the active profile has bindings for one device,
//!   the hook holds those buttons back until this thread has reported the
//!   device (well under a millisecond later), see `windows.rs`. Keyboards
//!   can't be told apart this way: a key blocked by the hook never reaches
//!   raw input. The thread only registers for raw input while it's needed
//!   (per-device bindings, or adding a device), because every mouse movement
//!   then wakes it.
//! * **Controllers.** An XInput thread polls Xbox-style controllers;
//!   `win_hid.rs` reads PlayStation, Switch, Steam and other controllers.
//!   Both hand every reading to `on_reading`, which turns changes into input
//!   events for the engine thread and moves the pointer (stick, gyro,
//!   touchpad). Nothing is read while the active profile doesn't use a
//!   controller.

use super::pad::{PadReading, PadState, PointerSettings};
use super::{match_device, DeviceIndex, EngineEvent};
use crate::config::{Device, PadButton, PadSettings};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetProductString;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE, RIDI_DEVICENAME, RID_INPUT, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostThreadMessageW, RegisterClassW, HWND_MESSAGE,
    MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT, WNDCLASSW,
};

/// To the engine thread: a mouse's raw report. `wParam` = raw button flags
/// (`RI_MOUSE_*`), `lParam` = device index + 1, or 0 when not a named device.
pub const WM_RAW_BUTTON: u32 = WM_APP + 3;
/// To the engine thread: a controller input changed. `wParam` = `pad_code`,
/// plus `PAD_RELEASED` when released; `lParam` as above.
pub const WM_PAD: u32 = WM_APP + 4;
pub const PAD_RELEASED: usize = 0x1_0000;
/// To the raw input thread: register or unregister as needed.
const WM_REGISTER: u32 = WM_APP + 1;

/// Raw button flags that are presses or wheel steps (not releases).
const RAW_PRESSES: u16 = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100 | 0x0400 | 0x0800;

/// Device id used for synthetic input while testing (`ACCEPT_INJECTED`).
const INJECTED_ID: &str = "injected";

/// How often XInput controllers are read: 250 times a second.
const PAD_POLL: Duration = Duration::from_millis(4);
/// Unplugged controller slots are slow to ask about; check them this often.
const PAD_RESCAN: Duration = Duration::from_secs(1);

static RAW_TID: AtomicU32 = AtomicU32::new(0);
static RAW_WANTED: AtomicBool = AtomicBool::new(false);
static RAW_ON: AtomicBool = AtomicBool::new(false);
static CAPTURE: AtomicBool = AtomicBool::new(false);
static PAD_WANTED: AtomicBool = AtomicBool::new(false);
/// Gyro or touchpad pointer in use (some controllers need switching to
/// their full reports for that).
static MOTION: AtomicBool = AtomicBool::new(false);
/// "Detect button": the next controller press selects that input.
static DETECT: AtomicBool = AtomicBool::new(false);
static GYRO_HELD: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU32 = AtomicU32::new(0);
static DEVICES: RwLock<Vec<Device>> = RwLock::new(Vec::new());

#[derive(Clone, Default)]
struct PadConfig {
    pointer: PointerSettings,
    settings: PadSettings,
}

static PAD_CONFIG: Mutex<Option<PadConfig>> = Mutex::new(None);

fn pad_config() -> PadConfig {
    PAD_CONFIG.lock().ok().and_then(|c| c.clone()).unwrap_or_default()
}

struct Ctx {
    on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>,
    accept_injected: bool,
    magic: u32,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

/// Start the threads (they idle until needed).
pub fn start(on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>, accept_injected: bool, magic: usize) {
    let _ = CTX.set(Ctx {
        on_event,
        accept_injected,
        magic: magic as u32,
    });
    std::thread::Builder::new()
        .name("remap-rawinput".into())
        .spawn(raw_thread)
        .expect("failed to spawn raw input thread");
    std::thread::Builder::new()
        .name("remap-xinput".into())
        .spawn(xinput_thread)
        .expect("failed to spawn controller thread");
    super::win_hid::start();
}

/// The named devices changed.
pub fn set_devices(devices: Vec<Device>) {
    if let Ok(mut d) = DEVICES.write() {
        *d = devices;
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// What the active profile needs: device reports for mouse buttons,
/// controllers (and how they move the pointer).
pub fn update(raw: bool, pad: bool, motion: bool, pointer: PointerSettings, settings: PadSettings) {
    RAW_WANTED.store(raw, Ordering::SeqCst);
    PAD_WANTED.store(pad, Ordering::SeqCst);
    MOTION.store(motion, Ordering::SeqCst);
    if let Ok(mut c) = PAD_CONFIG.lock() {
        *c = Some(PadConfig { pointer, settings });
    }
    post_raw(WM_REGISTER);
}

/// Report the next press on a mouse or controller.
pub fn set_capture(on: bool) {
    CAPTURE.store(on, Ordering::SeqCst);
    post_raw(WM_REGISTER);
}

/// Report the next controller input pressed (to select it in the UI).
pub fn set_detect(on: bool) {
    DETECT.store(on, Ordering::SeqCst);
}

/// The Gyro action is holding the gyro mouse on (or toggled it).
pub fn set_gyro_held(on: bool) {
    GYRO_HELD.store(on, Ordering::SeqCst);
}

/// Whether device reports are coming (so holding back mouse buttons for
/// them is worth it).
pub fn reporting() -> bool {
    RAW_ON.load(Ordering::SeqCst) && RAW_WANTED.load(Ordering::SeqCst)
}

/// Whether controllers should be read at all right now.
fn pads_needed() -> bool {
    PAD_WANTED.load(Ordering::SeqCst) || CAPTURE.load(Ordering::SeqCst) || DETECT.load(Ordering::SeqCst)
}

/// Read controllers directly over HID (`win_hid.rs`).
pub fn wants_direct() -> bool {
    pads_needed() && pad_config().settings.direct
}

fn wants_xinput() -> bool {
    pads_needed() && pad_config().settings.xinput
}

/// Gyro or touchpad pointer in use.
pub fn wants_motion() -> bool {
    MOTION.load(Ordering::SeqCst)
}

/// Turn off Steam devices' own mouse mode while reading them.
pub fn steam_lizard_off() -> bool {
    pad_config().settings.steam_lizard_off
}

/// `WM_PAD` code of a controller input.
pub fn pad_code(b: PadButton) -> usize {
    match b {
        PadButton::Button(n) => 0x100 + n as usize - 1,
        b => PadButton::ALL.iter().position(|x| *x == b).unwrap_or(0),
    }
}

/// The controller input of a `WM_PAD` code.
pub fn pad_from_code(code: usize) -> Option<PadButton> {
    let code = code & !PAD_RELEASED;
    if code >= 0x100 {
        let n = code - 0x100 + 1;
        (n <= PadButton::MAX_NUMBERED as usize).then_some(PadButton::Button(n as u8))
    } else {
        PadButton::ALL.get(code).copied()
    }
}

fn post_raw(message: u32) {
    let tid = RAW_TID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, message, WPARAM(0), LPARAM(0));
        }
    }
}

fn captured(id: String, name: String) {
    if CAPTURE.swap(false, Ordering::SeqCst) {
        post_raw(WM_REGISTER);
        if let Some(c) = CTX.get() {
            (c.on_event)(EngineEvent::DeviceCaptured { id, name });
        }
    }
}

// ---------------------------------------------------------------------------
// Raw input
// ---------------------------------------------------------------------------

fn raw_thread() {
    unsafe {
        RAW_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
        let hinstance = GetModuleHandleW(PCWSTR::null()).ok().map(Into::into);
        let class = WNDCLASSW {
            lpfnWndProc: Some(raw_wnd_proc),
            hInstance: hinstance.unwrap_or_default(),
            lpszClassName: w!("mouse-kb-remap-rawinput"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("mouse-kb-remap-rawinput"),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            hinstance,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                crate::log!("engine: raw input window failed: {e}; per-device bindings won't work");
                return;
            }
        };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_REGISTER {
                register(hwnd);
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}

/// Register for mouse raw input, or stop, as needed.
fn register(hwnd: HWND) {
    let want = RAW_WANTED.load(Ordering::SeqCst) || CAPTURE.load(Ordering::SeqCst);
    if want == RAW_ON.load(Ordering::SeqCst) {
        return;
    }
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop
        usUsage: 0x02,     // mouse
        dwFlags: if want { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if want { hwnd } else { HWND::default() },
    };
    match unsafe { RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) } {
        Ok(()) => {
            RAW_ON.store(want, Ordering::SeqCst);
            crate::log!("engine: mouse device reports {}", if want { "on" } else { "off" });
        }
        Err(e) => crate::log!("engine: raw input registration failed: {e}"),
    }
}

thread_local! {
    /// Raw device handle → named device, for the current device list.
    static CACHE: std::cell::RefCell<(u32, HashMap<isize, Option<DeviceIndex>>)> =
        std::cell::RefCell::new((u32::MAX, HashMap::new()));
}

unsafe extern "system" fn raw_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_INPUT {
        on_raw_input(HRAWINPUT(lparam.0 as *mut _));
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn on_raw_input(handle: HRAWINPUT) {
    let mut raw = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    let got = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some(&mut raw as *mut RAWINPUT as *mut _),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if got == u32::MAX || got == 0 || raw.header.dwType != RIM_TYPEMOUSE.0 {
        return;
    }
    let Some(ctx) = CTX.get() else { return };
    let mouse = unsafe { raw.data.mouse };
    let flags = unsafe { mouse.Anonymous.Anonymous.usButtonFlags } & 0x0FFF;
    if flags == 0 || mouse.ulExtraInformation == ctx.magic {
        // Movement only, or our own output.
        return;
    }
    let device = raw.header.hDevice;
    if CAPTURE.load(Ordering::SeqCst) && flags & RAW_PRESSES != 0 {
        if device.is_invalid() {
            if ctx.accept_injected {
                captured(INJECTED_ID.into(), "Synthetic input".into());
            }
        } else if let Some(path) = device_path(device) {
            let name = product_name(&path).unwrap_or_else(|| "Mouse".into());
            captured(path, name);
        }
    }
    if RAW_WANTED.load(Ordering::SeqCst) {
        let index = resolve(device, ctx.accept_injected);
        super::windows::post_engine_with(
            WM_RAW_BUTTON,
            WPARAM(flags as usize),
            LPARAM(index.map_or(0, |i| i as isize + 1)),
        );
    }
}

/// The named device a raw device handle belongs to.
fn resolve(device: HANDLE, accept_injected: bool) -> Option<DeviceIndex> {
    let generation = GENERATION.load(Ordering::SeqCst);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.0 != generation {
            *c = (generation, HashMap::new());
        }
        *c.1.entry(device.0 as isize).or_insert_with(|| {
            let id = if device.is_invalid() {
                accept_injected.then(|| INJECTED_ID.to_string())?
            } else {
                device_path(device)?
            };
            let devices = DEVICES.read().ok()?;
            match_device(&devices, &id)
        })
    })
}

/// The device interface path, e.g. `\\?\HID#VID_046D&PID_C539&MI_01&Col01#…`.
fn device_path(device: HANDLE) -> Option<String> {
    unsafe {
        let mut len = 0u32;
        GetRawInputDeviceInfoW(Some(device), RIDI_DEVICENAME, None, &mut len);
        if len == 0 || len > 4096 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        let n = GetRawInputDeviceInfoW(
            Some(device),
            RIDI_DEVICENAME,
            Some(buf.as_mut_ptr() as *mut _),
            &mut len,
        );
        if n == u32::MAX || n == 0 {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

/// What the device calls itself ("G502 HERO Gaming Mouse").
fn product_name(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let file = CreateFileW(
            PCWSTR(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()?;
        let mut buf = [0u16; 128];
        let ok = HidD_GetProductString(file, buf.as_mut_ptr() as *mut _, std::mem::size_of_val(&buf) as u32);
        let _ = CloseHandle(file);
        if !ok {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let name = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

// ---------------------------------------------------------------------------
// Controllers
// ---------------------------------------------------------------------------

/// One controller being read (an XInput slot or a HID device).
pub struct Source {
    /// `Device::id`: `xinput:1`, or the HID device path.
    pub id: String,
    pub name: String,
    state: PadState,
    last: Option<Instant>,
    generation: u32,
    device: Option<DeviceIndex>,
}

impl Source {
    pub fn new(id: String, name: String) -> Self {
        Source {
            id,
            name,
            state: PadState::default(),
            last: None,
            generation: u32::MAX,
            device: None,
        }
    }

    pub fn has_reported(&self) -> bool {
        self.last.is_some()
    }

    /// The named device this controller is, kept up to date with the list.
    fn device(&mut self) -> Option<DeviceIndex> {
        let generation = GENERATION.load(Ordering::SeqCst);
        if generation != self.generation {
            self.generation = generation;
            self.device = DEVICES.read().ok().and_then(|d| match_device(&d, &self.id));
        }
        self.device
    }
}

/// A controller sent a reading: report what changed to the engine thread
/// and move the pointer.
pub fn on_reading(src: &mut Source, r: &PadReading) {
    let now = Instant::now();
    let dt = src
        .last
        .map_or(0.004, |t| now.duration_since(t).as_secs_f32())
        .min(0.05);
    src.last = Some(now);
    let config = pad_config();
    let wanted = PAD_WANTED.load(Ordering::SeqCst) && super::windows::remapping_on();
    let pointer = if wanted {
        config.pointer
    } else {
        PointerSettings::default()
    };
    let (changes, moved) = src
        .state
        .update(r, dt, &pointer, wanted && GYRO_HELD.load(Ordering::SeqCst));
    if let Some(&(b, _)) = changes.iter().find(|(_, down)| *down) {
        if CAPTURE.load(Ordering::SeqCst) {
            captured(src.id.clone(), src.name.clone());
        }
        if DETECT.swap(false, Ordering::SeqCst) {
            if let Some(c) = CTX.get() {
                (c.on_event)(EngineEvent::PadDetected {
                    input: format!("pad:{}", b.id()),
                });
            }
            return;
        }
    }
    if PAD_WANTED.load(Ordering::SeqCst) {
        send_changes(src.device(), &changes);
    }
    if moved != (0, 0) {
        super::windows::send_move(moved.0, moved.1);
    }
}

/// A controller went away: release whatever it held.
pub fn on_gone(src: &mut Source) {
    let changes = src.state.release_all();
    send_changes(src.device(), &changes);
}

fn send_changes(device: Option<DeviceIndex>, changes: &[(PadButton, bool)]) {
    for &(b, down) in changes {
        super::windows::post_engine_with(
            WM_PAD,
            WPARAM(pad_code(b) | if down { 0 } else { PAD_RELEASED }),
            LPARAM(device.map_or(0, |d| d as isize + 1)),
        );
    }
}

type GetStateFn = unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32;
type GetCapsExFn = unsafe extern "system" fn(u32, u32, u32, *mut CapsEx) -> u32;

/// `XINPUT_CAPABILITIES_EX`: the documented capabilities, then vendor and
/// product ids.
#[repr(C)]
#[derive(Default)]
struct CapsEx {
    caps: [u8; 20],
    vendor: u16,
    product: u16,
    version: u16,
    unknown1: u16,
    unknown2: u32,
}

/// XInput's unnamed exports: `XInputGetStateEx` (ordinal 100) also reports
/// the Guide button, `XInputGetCapabilitiesEx` (108) the vendor / product.
struct XInputEx {
    get_state: Option<GetStateFn>,
    get_caps: Option<GetCapsExFn>,
}

impl XInputEx {
    fn load() -> Self {
        unsafe {
            let Ok(lib) = LoadLibraryW(w!("xinput1_4.dll")) else {
                return XInputEx {
                    get_state: None,
                    get_caps: None,
                };
            };
            let ordinal = |n: usize| GetProcAddress(lib, windows::core::PCSTR(n as *const u8));
            XInputEx {
                get_state: ordinal(100).map(|f| std::mem::transmute::<_, GetStateFn>(f)),
                get_caps: ordinal(108).map(|f| std::mem::transmute::<_, GetCapsExFn>(f)),
            }
        }
    }

    fn state(&self, slot: u32, state: &mut XINPUT_STATE) -> u32 {
        unsafe {
            match self.get_state {
                Some(f) => f(slot, state),
                None => XInputGetState(slot, state),
            }
        }
    }

    /// Vendor and product of the controller in a slot, if XInput says.
    fn ids(&self, slot: u32) -> Option<(u16, u16)> {
        let f = self.get_caps?;
        let mut caps = CapsEx::default();
        (unsafe { f(1, slot, 0, &mut caps) } == 0).then_some((caps.vendor, caps.product))
    }
}

/// Steam Input's virtual Xbox controller.
const STEAM_VIRTUAL: (u16, u16) = (0x28DE, 0x11FF);

fn xinput_thread() {
    let ex = XInputEx::load();
    let mut sources: [Option<Source>; 4] = Default::default();
    let mut skipped = [false; 4];
    let mut last_scan = Instant::now() - PAD_RESCAN;
    loop {
        if !wants_xinput() {
            for s in sources.iter_mut() {
                if let Some(mut src) = s.take() {
                    on_gone(&mut src);
                }
            }
            skipped = [false; 4];
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let rescan = last_scan.elapsed() >= PAD_RESCAN;
        if rescan {
            last_scan = Instant::now();
        }
        let skip_virtual = pad_config().settings.skip_steam_virtual;
        for slot in 0..4u32 {
            let i = slot as usize;
            if sources[i].is_none() && !skipped[i] && !rescan {
                continue;
            }
            let mut state = XINPUT_STATE::default();
            if ex.state(slot, &mut state) != 0 {
                if let Some(mut src) = sources[i].take() {
                    on_gone(&mut src);
                    crate::log!("controller: {} disconnected", src.name);
                }
                skipped[i] = false;
                continue;
            }
            if sources[i].is_none() {
                if skipped[i] {
                    continue;
                }
                if skip_virtual && ex.ids(slot) == Some(STEAM_VIRTUAL) {
                    crate::log!("controller: skipping Steam's virtual controller {}", slot + 1);
                    skipped[i] = true;
                    continue;
                }
                let name = format!("Controller {}", slot + 1);
                crate::log!("controller: {name} connected (XInput)");
                sources[i] = Some(Source::new(format!("xinput:{}", slot + 1), name));
            }
            let g = state.Gamepad;
            let reading = PadReading {
                buttons: super::pad::from_xinput(g.wButtons.0),
                lt: g.bLeftTrigger,
                rt: g.bRightTrigger,
                lx: g.sThumbLX.max(-32767),
                ly: g.sThumbLY.max(-32767),
                rx: g.sThumbRX.max(-32767),
                ry: g.sThumbRY.max(-32767),
                ..Default::default()
            };
            if let Some(src) = sources[i].as_mut() {
                on_reading(src, &reading);
            }
        }
        std::thread::sleep(PAD_POLL);
    }
}
