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
//! * **Controller thread.** Polls XInput controllers while the active profile
//!   uses one, turns button, trigger and stick changes into input events for
//!   the engine thread, and moves the pointer with a stick.

use super::pad::{self, PadReading};
use super::{match_device, DeviceIndex, EngineEvent};
use crate::config::{Device, PadButton, StickPointer};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetProductString;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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
/// To the engine thread: a controller input changed. `wParam` = index in
/// `PadButton::ALL`, plus 0x100 when released; `lParam` as above.
pub const WM_PAD: u32 = WM_APP + 4;
/// To the raw input thread: register or unregister as needed.
const WM_REGISTER: u32 = WM_APP + 1;

/// Raw button flags that are presses or wheel steps (not releases).
const RAW_PRESSES: u16 = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100 | 0x0400 | 0x0800;

/// Device id used for synthetic input while testing (`ACCEPT_INJECTED`).
const INJECTED_ID: &str = "injected";

/// How often controllers are read: 250 times a second.
const PAD_POLL: Duration = Duration::from_millis(4);
/// Unplugged controller slots are slow to ask about; check them this often.
const PAD_RESCAN: Duration = Duration::from_secs(1);

static RAW_TID: AtomicU32 = AtomicU32::new(0);
static RAW_WANTED: AtomicBool = AtomicBool::new(false);
static RAW_ON: AtomicBool = AtomicBool::new(false);
static CAPTURE: AtomicBool = AtomicBool::new(false);
static PAD_WANTED: AtomicBool = AtomicBool::new(false);
static STICK: AtomicU8 = AtomicU8::new(0);
static STICK_SPEED: AtomicU32 = AtomicU32::new(10);
static GENERATION: AtomicU32 = AtomicU32::new(0);
static DEVICES: RwLock<Vec<Device>> = RwLock::new(Vec::new());

struct Ctx {
    on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>,
    accept_injected: bool,
    magic: u32,
}

static CTX: OnceLock<Ctx> = OnceLock::new();

/// Start both threads (they idle until needed).
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
        .name("remap-controller".into())
        .spawn(pad_thread)
        .expect("failed to spawn controller thread");
}

/// The named devices changed.
pub fn set_devices(devices: Vec<Device>) {
    if let Ok(mut d) = DEVICES.write() {
        *d = devices;
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// What the active profile needs: device reports for mouse buttons, and
/// controller polling (with the stick that moves the pointer).
pub fn update(raw: bool, pad: bool, stick: StickPointer, speed: u32) {
    RAW_WANTED.store(raw, Ordering::SeqCst);
    PAD_WANTED.store(pad, Ordering::SeqCst);
    STICK.store(
        match stick {
            StickPointer::Off => 0,
            StickPointer::Left => 1,
            StickPointer::Right => 2,
        },
        Ordering::SeqCst,
    );
    STICK_SPEED.store(speed.clamp(1, 20), Ordering::SeqCst);
    post_raw(WM_REGISTER);
}

/// Report the next press on a mouse or controller.
pub fn set_capture(on: bool) {
    CAPTURE.store(on, Ordering::SeqCst);
    post_raw(WM_REGISTER);
}

/// Whether device reports are coming (so holding back mouse buttons for
/// them is worth it).
pub fn reporting() -> bool {
    RAW_ON.load(Ordering::SeqCst) && RAW_WANTED.load(Ordering::SeqCst)
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
// Controllers (XInput)
// ---------------------------------------------------------------------------

fn pad_thread() {
    let mut masks = [0u32; 4];
    let mut connected = [false; 4];
    let mut rest = [(0.0f32, 0.0f32); 4];
    let mut last_scan = Instant::now() - PAD_RESCAN;
    let mut last = Instant::now();
    loop {
        let capture = CAPTURE.load(Ordering::SeqCst);
        if !PAD_WANTED.load(Ordering::SeqCst) && !capture {
            // Idle: forget state, so nothing counts as held when it resumes.
            masks = [0; 4];
            connected = [false; 4];
            last_scan = Instant::now() - PAD_RESCAN;
            std::thread::sleep(Duration::from_millis(200));
            last = Instant::now();
            continue;
        }
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().min(0.05);
        last = now;
        let rescan = now.duration_since(last_scan) >= PAD_RESCAN;
        if rescan {
            last_scan = now;
        }
        for slot in 0..4 {
            if !connected[slot] && !rescan {
                continue;
            }
            let mut state = XINPUT_STATE::default();
            if unsafe { XInputGetState(slot as u32, &mut state) } != 0 {
                if connected[slot] {
                    // Unplugged: release whatever was held.
                    send_changes(slot, masks[slot], 0);
                    masks[slot] = 0;
                    crate::log!("engine: controller {} disconnected", slot + 1);
                }
                connected[slot] = false;
                continue;
            }
            if !connected[slot] {
                connected[slot] = true;
                crate::log!("engine: controller {} connected", slot + 1);
            }
            let g = state.Gamepad;
            let reading = PadReading {
                buttons: g.wButtons.0,
                lt: g.bLeftTrigger,
                rt: g.bRightTrigger,
                lx: g.sThumbLX,
                ly: g.sThumbLY,
                rx: g.sThumbRX,
                ry: g.sThumbRY,
            };
            let mask = pad::pad_mask(&reading, masks[slot]);
            if capture && mask & !masks[slot] != 0 {
                captured(format!("xinput:{}", slot + 1), format!("Controller {}", slot + 1));
            }
            if PAD_WANTED.load(Ordering::SeqCst) {
                send_changes(slot, masks[slot], mask);
                let (sx, sy) = match STICK.load(Ordering::SeqCst) {
                    1 => (reading.lx, reading.ly),
                    2 => (reading.rx, reading.ry),
                    _ => (0, 0),
                };
                let (dx, dy) = pad::stick_move(sx, sy, STICK_SPEED.load(Ordering::SeqCst), dt, &mut rest[slot]);
                if dx != 0 || dy != 0 {
                    super::windows::send_move(dx, dy);
                }
            }
            masks[slot] = mask;
        }
        std::thread::sleep(PAD_POLL);
    }
}

/// Tell the engine thread about inputs that went down or up.
fn send_changes(slot: usize, before: u32, after: u32) {
    if before == after {
        return;
    }
    let device = DEVICES
        .read()
        .ok()
        .and_then(|d| match_device(&d, &format!("xinput:{}", slot + 1)));
    for (b, down) in pad::changes(before, after) {
        let i = PadButton::ALL.iter().position(|x| *x == b).unwrap_or(0);
        super::windows::post_engine_with(
            WM_PAD,
            WPARAM(i | if down { 0 } else { 0x100 }),
            LPARAM(device.map_or(0, |d| d as isize + 1)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::{GetRawInputDeviceList, RAWINPUTDEVICELIST, RIM_TYPEMOUSE};

    /// Lists the mice and controllers on this PC as the app would name them.
    /// `cargo test list_devices -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn list_devices() {
        let mut n = 0u32;
        let size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
        unsafe { GetRawInputDeviceList(None, &mut n, size) };
        let mut list = vec![RAWINPUTDEVICELIST::default(); n as usize];
        let got = unsafe { GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut n, size) };
        assert_ne!(got, u32::MAX);
        let mut mice = 0;
        for d in &list[..got as usize] {
            if d.dwType != RIM_TYPEMOUSE {
                continue;
            }
            let path = device_path(d.hDevice).expect("device path");
            println!("mouse: {:?} → {}", product_name(&path), path);
            mice += 1;
        }
        assert!(mice > 0, "no mouse found");
        for slot in 0..4 {
            let mut state = XINPUT_STATE::default();
            let connected = unsafe { XInputGetState(slot, &mut state) } == 0;
            println!(
                "controller slot {}: {}",
                slot + 1,
                if connected { "connected" } else { "empty" }
            );
        }
    }
}
