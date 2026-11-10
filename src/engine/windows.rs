//! Windows input backend.
//!
//! One dedicated thread owns:
//! * a low-level mouse hook and keyboard hook (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`)
//! * a foreground-window event hook (`EVENT_SYSTEM_FOREGROUND`)
//! * the message loop those hooks need.
//!
//! The hook callbacks must return quickly: Windows silently removes
//! low-level hooks that time out. So they only do a non-blocking lookup in a
//! pre-built table, a `SendInput` and maybe a channel send; anything timed
//! (macros, turbo, text) runs on a separate worker thread (`player.rs`).

use super::dispatch::{wheel_for, Dispatcher, Phase, Sink};
use super::player::{self, Cmd, Out};
use super::{CompiledProfile, EngineEvent, FocusInfo, Input, Layers, RecEvent};
use crate::config::MouseButton;

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC_EX,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, SetWindowsHookExW,
    SystemParametersInfoW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
    LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SPI_GETMOUSESPEED, SPI_SETMOUSESPEED, SW_SHOWNORMAL,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WHEEL_DELTA, WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

pub const SUPPORTED: bool = true;

/// Tag on every event we inject (`dwExtraInfo`), so we can tell our own
/// output apart from other programs' synthetic input.
const MAGIC: usize = 0x4D4B_5252; // "MKRR"

/// Set to 1 to remap synthetic input from other programs too (used by the
/// end-to-end tests, which drive the app with `SendInput`). Our own output is
/// always ignored, so there are no feedback loops.
const ACCEPT_INJECTED_ENV: &str = "MOUSE_KB_REMAP_ACCEPT_INJECTED";

const NO_PROFILE: usize = usize::MAX;

struct Recording {
    start: Instant,
    events: Vec<(Duration, RecEvent)>,
}

struct Shared {
    profiles: RwLock<Vec<CompiledProfile>>,
    /// Bindings for the currently focused window (`None` while our own
    /// window is focused). Swapped on focus change.
    active: RwLock<Option<Arc<Layers>>>,
    active_index: AtomicUsize,
    enabled: AtomicBool,
    self_pid: u32,
    self_elevated: bool,
    accept_injected: bool,
    last_focus: Mutex<(String, bool)>, // (lower-cased exe, is_self)
    focus_self: AtomicBool,
    on_event: Box<dyn Fn(EngineEvent) + Send + Sync>,
    worker: Sender<Cmd>,
    capture: AtomicBool,
    recording: AtomicBool,
    record: Mutex<Option<Recording>>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get().expect("engine not started")
}

#[derive(Default)]
struct CaptureState {
    /// Modifiers pressed during capture.
    mods: u8,
    /// A modifier pressed with nothing after it yet (captured alone on release).
    lone_mod: Option<u16>,
    /// Keys whose press we swallowed, so their release is swallowed too.
    swallow_up: Vec<u16>,
}

thread_local! {
    // Hooks run on the engine thread only.
    static DISPATCH: RefCell<Dispatcher> = RefCell::new(Dispatcher::default());
    static CAPTURE: RefCell<CaptureState> = RefCell::new(CaptureState::default());
    static REC_DOWN: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
}

pub struct Engine;

impl Engine {
    pub fn start(
        profiles: Vec<CompiledProfile>,
        enabled: bool,
        on_event: Box<dyn Fn(EngineEvent) + Send + Sync>,
    ) -> Self {
        let self_pid = unsafe { GetCurrentProcessId() };
        let self_elevated = process_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = SHARED.set(Shared {
            profiles: RwLock::new(profiles),
            active: RwLock::new(None),
            active_index: AtomicUsize::new(NO_PROFILE),
            enabled: AtomicBool::new(enabled),
            self_pid,
            self_elevated,
            accept_injected: std::env::var(ACCEPT_INJECTED_ENV).is_ok_and(|v| v == "1"),
            last_focus: Mutex::new((String::new(), true)),
            focus_self: AtomicBool::new(true),
            on_event,
            worker: tx,
            capture: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            record: Mutex::new(None),
        });

        std::thread::Builder::new()
            .name("remap-player".into())
            .spawn(move || player::run_worker(rx, emit_outs))
            .expect("failed to spawn player thread");
        std::thread::Builder::new()
            .name("remap-engine".into())
            .spawn(engine_thread)
            .expect("failed to spawn engine thread");
        Engine
    }

    pub fn apply(&self, profiles: Vec<CompiledProfile>, enabled: bool) {
        let sh = shared();
        sh.enabled.store(enabled, Ordering::Relaxed);
        if let Ok(mut p) = sh.profiles.write() {
            *p = profiles;
        }
        let (exe, is_self) = sh.last_focus.lock().map(|f| f.clone()).unwrap_or_default();
        recompute_active(&exe, is_self);
        // Bindings may have changed under a running macro or turbo.
        let _ = sh.worker.send(Cmd::StopAll);
    }

    pub fn elevated(&self) -> bool {
        shared().self_elevated
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
        // Give the worker a moment to release held keys, then make sure the
        // pointer speed is back to the user's setting.
        std::thread::sleep(Duration::from_millis(40));
        set_pointer_speed(None);
    }
}

fn engine_thread() {
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null()).ok();
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod.map(Into::into), 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod.map(Into::into), 0);
        if let Err(e) = &mouse {
            crate::log!("engine: mouse hook failed: {e}");
        }
        if let Err(e) = &keyboard {
            crate::log!("engine: keyboard hook failed: {e}");
        }
        let _focus_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        on_foreground(GetForegroundWindow());

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---------------------------------------------------------------------------
// Focus tracking
// ---------------------------------------------------------------------------

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    on_foreground(hwnd);
}

fn on_foreground(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return;
    }
    let sh = shared();
    let is_self = pid == sh.self_pid;
    let (exe, elevated) = process_info(pid);
    let exe_lower = exe.to_ascii_lowercase();

    if let Ok(mut f) = sh.last_focus.lock() {
        *f = (exe_lower.clone(), is_self);
    }
    sh.focus_self.store(is_self, Ordering::Relaxed);
    if !is_self {
        // Never keep swallowing keys for a capture the user walked away from.
        sh.capture.store(false, Ordering::SeqCst);
    }
    let before = sh.active_index.load(Ordering::Relaxed);
    let after = recompute_active(&exe_lower, is_self);
    if before != after {
        // A toggled turbo or looping macro stops when you switch profile, and
        // focusing this window stops everything (a handy panic button).
        let _ = sh.worker.send(Cmd::StopAll);
    }

    (sh.on_event)(EngineEvent::Focus(FocusInfo {
        exe,
        is_self,
        elevation_blocked: elevated && !sh.self_elevated,
    }));
}

/// Pick the active profile; returns its index (`NO_PROFILE` for our window).
fn recompute_active(exe_lower: &str, is_self: bool) -> usize {
    let sh = shared();
    let (index, layers) = if is_self {
        // Never remap our own window, so a bad binding can't lock you out of the UI.
        (NO_PROFILE, None)
    } else {
        let profiles = match sh.profiles.read() {
            Ok(p) => p,
            Err(_) => return NO_PROFILE,
        };
        let index = profiles
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, p)| p.apps.iter().any(|a| a == exe_lower))
            .map(|(i, _)| i)
            .unwrap_or(0);
        (index, profiles.get(index).map(|p| p.layers.clone()))
    };
    if let Ok(mut a) = sh.active.write() {
        *a = layers;
    }
    sh.active_index.store(index, Ordering::Relaxed);
    index
}

/// Returns (exe file name, is elevated). If we can't inspect the process at
/// all we assume it is elevated, since that is the common reason for failure.
fn process_info(pid: u32) -> (String, bool) {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), true);
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let exe = if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
        } else {
            String::new()
        };
        let elevated = process_elevated(handle).unwrap_or(true);
        let _ = CloseHandle(handle);
        (exe, elevated)
    }
}

fn process_elevated(process: HANDLE) -> Option<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        let _ = CloseHandle(token);
        result.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

// ---------------------------------------------------------------------------
// Input hooks
// ---------------------------------------------------------------------------

/// Should the hook look at this event at all?
fn is_physical(injected: bool, extra: usize) -> bool {
    if extra == MAGIC {
        return false;
    }
    !injected || shared().accept_injected
}

fn record(ev: RecEvent) {
    let sh = shared();
    if let Ok(mut r) = sh.record.try_lock() {
        if let Some(r) = r.as_mut() {
            if r.events.len() < 5000 {
                r.events.push((r.start.elapsed(), ev));
            }
        }
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let msg = wparam.0 as u32;
    if code == HC_ACTION as i32 && msg != WM_MOUSEMOVE {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if is_physical(info.flags & LLMHF_INJECTED != 0, info.dwExtraInfo) {
            if let Some((input, phase)) = decode_mouse(msg, info.mouseData) {
                let sh = shared();
                if sh.recording.load(Ordering::Relaxed) {
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
                if handle(input, phase) {
                    return LRESULT(1);
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn decode_mouse(msg: u32, mouse_data: u32) -> Option<(Input, Phase)> {
    let hi = (mouse_data >> 16) as u16;
    let xbutton = || match hi {
        XBUTTON1 => Some(MouseButton::Back),
        XBUTTON2 => Some(MouseButton::Forward),
        _ => None,
    };
    let (button, phase) = match msg {
        WM_LBUTTONDOWN => (MouseButton::Left, Phase::Down),
        WM_LBUTTONUP => (MouseButton::Left, Phase::Up),
        WM_RBUTTONDOWN => (MouseButton::Right, Phase::Down),
        WM_RBUTTONUP => (MouseButton::Right, Phase::Up),
        WM_MBUTTONDOWN => (MouseButton::Middle, Phase::Down),
        WM_MBUTTONUP => (MouseButton::Middle, Phase::Up),
        WM_XBUTTONDOWN => (xbutton()?, Phase::Down),
        WM_XBUTTONUP => (xbutton()?, Phase::Up),
        WM_MOUSEWHEEL => (wheel_for(hi as i16 as i32, false), Phase::Tap),
        WM_MOUSEHWHEEL => (wheel_for(hi as i16 as i32, true), Phase::Tap),
        _ => return None,
    };
    Some((Input::Mouse(button), phase))
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if is_physical(info.flags.contains(LLKHF_INJECTED), info.dwExtraInfo) {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let mut vk = info.vkCode as u16;
            if vk == 0x0D && info.flags.contains(LLKHF_EXTENDED) {
                vk = crate::keys::VK_NUMPAD_ENTER;
            }
            if capture_key(vk, down) {
                return LRESULT(1);
            }
            let sh = shared();
            if sh.recording.load(Ordering::Relaxed) {
                let repeat = REC_DOWN.with(|d| {
                    let mut d = d.borrow_mut();
                    let was_down = d.contains(&vk);
                    if down && !was_down {
                        d.push(vk);
                    } else if !down {
                        d.retain(|v| *v != vk);
                    }
                    down && was_down
                });
                if !repeat {
                    record(RecEvent::Key { vk, down });
                }
                // Keys typed while recording in our own window shouldn't
                // also act on the UI.
                if sh.focus_self.load(Ordering::Relaxed) {
                    return LRESULT(1);
                }
            }
            let phase = if down { Phase::Down } else { Phase::Up };
            if handle(Input::Key(vk), phase) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Key capture for the UI ("press the key to send"). Returns true to swallow.
fn capture_key(vk: u16, down: bool) -> bool {
    let sh = shared();
    let capturing = sh.capture.load(Ordering::SeqCst);
    CAPTURE.with(|c| {
        let mut c = c.borrow_mut();
        if !down {
            if capturing {
                if c.lone_mod == Some(vk) {
                    // A modifier pressed and released on its own.
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
            (sh.on_event)(EngineEvent::Captured { vk, mods: c.mods });
            c.mods = 0;
            c.lone_mod = None;
        }
        if !c.swallow_up.contains(&vk) {
            c.swallow_up.push(vk);
        }
        true
    })
}

/// Returns true if the original input should be swallowed.
fn handle(input: Input, phase: Phase) -> bool {
    let sh = shared();
    let enabled = sh.enabled.load(Ordering::Relaxed);
    // try_read: never block inside a low-level hook. If a writer holds the
    // lock (a profile switch this instant) the event passes through.
    let active = sh.active.try_read().ok().and_then(|a| a.clone());
    let mut sink = HookSink {
        buf: [INPUT::default(); 16],
        len: 0,
        worker: &sh.worker,
    };
    DISPATCH.with(|d| {
        d.borrow_mut()
            .handle(input, phase, active.as_deref(), enabled, &mut sink)
    })
}

struct HookSink<'a> {
    buf: [INPUT; 16],
    len: usize,
    worker: &'a Sender<Cmd>,
}

impl Sink for HookSink<'_> {
    fn out(&mut self, out: Out) {
        let mut push = |i: INPUT| {
            if self.len < self.buf.len() {
                self.buf[self.len] = i;
                self.len += 1;
            }
        };
        out_to_inputs(&out, &mut push);
    }

    fn flush(&mut self) {
        if self.len > 0 {
            send(&self.buf[..self.len]);
            self.len = 0;
        }
    }

    fn worker(&mut self, cmd: Cmd) {
        let _ = self.worker.send(cmd);
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Called on the player thread with a batch of outputs.
fn emit_outs(outs: &[Out]) {
    let mut batch: Vec<INPUT> = Vec::with_capacity(outs.len() * 2);
    for out in outs {
        match out {
            Out::Launch(spec) => {
                send(&batch);
                batch.clear();
                launch(spec.clone());
            }
            Out::PointerSpeed(speed) => {
                send(&batch);
                batch.clear();
                set_pointer_speed(*speed);
            }
            other => out_to_inputs(other, &mut |i| batch.push(i)),
        }
    }
    send(&batch);
}

fn out_to_inputs(out: &Out, push: &mut impl FnMut(INPUT)) {
    match *out {
        Out::Key { vk, down } => push(key_input(vk, !down)),
        Out::Button { button, down } => {
            let (d, u, data) = match button {
                MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
                MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
                MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
                MouseButton::Back => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1 as u32),
                MouseButton::Forward => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2 as u32),
                wheel => {
                    if down {
                        push(wheel_input(wheel));
                    }
                    return;
                }
            };
            push(mouse_input(if down { d } else { u }, data));
        }
        Out::Wheel(b) => push(wheel_input(b)),
        Out::Char(c) => {
            if c == '\n' {
                push(key_input(0x0D, false));
                push(key_input(0x0D, true));
                return;
            }
            if c == '\t' {
                push(key_input(0x09, false));
                push(key_input(0x09, true));
                return;
            }
            let mut units = [0u16; 2];
            for &unit in c.encode_utf16(&mut units).iter() {
                push(unicode_input(unit, false));
                push(unicode_input(unit, true));
            }
        }
        Out::Launch(_) | Out::PointerSpeed(_) => {}
    }
}

/// Keys that must be sent by virtual-key code: media/browser keys and a few
/// whose scan codes don't round-trip through `SendInput`.
fn send_as_vk(vk: u16) -> bool {
    matches!(vk, 0x13 | 0x90 | 0xA6..=0xB7)
}

/// Keys are sent as scan codes where possible: many games read scan codes
/// (raw input / DirectInput) and ignore virtual-key-only events.
fn key_input(vk: u16, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    let mut w_vk = 0u16;
    let mut w_scan = 0u16;
    if vk == crate::keys::VK_NUMPAD_ENTER {
        flags = KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY;
        w_scan = 0x1C;
    } else {
        let scan = if send_as_vk(vk) {
            0
        } else {
            unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) }
        };
        if scan == 0 {
            w_vk = vk;
        } else {
            flags = KEYEVENTF_SCANCODE;
            if scan & 0xFF00 == 0xE000 || scan & 0xFF00 == 0xE100 {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            w_scan = (scan & 0xFF) as u16;
        }
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(w_vk),
                wScan: w_scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

fn wheel_input(button: MouseButton) -> INPUT {
    let delta = WHEEL_DELTA as i32;
    let (flags, amount) = match button {
        MouseButton::WheelUp => (MOUSEEVENTF_WHEEL, delta),
        MouseButton::WheelDown => (MOUSEEVENTF_WHEEL, -delta),
        MouseButton::WheelRight => (MOUSEEVENTF_HWHEEL, delta),
        _ => (MOUSEEVENTF_HWHEEL, -delta),
    };
    mouse_input(flags, amount as u32)
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Start a program / open a file or URL, off the input threads.
fn launch(spec: Arc<player::LaunchSpec>) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let path = HSTRING::from(spec.path.as_str());
        let args = HSTRING::from(spec.args.as_str());
        let result = ShellExecuteW(
            None,
            w!("open"),
            &path,
            if spec.args.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(args.as_ptr())
            },
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if result.0 as usize <= 32 {
            crate::log!("launch: could not start {:?} (error {})", spec.path, result.0 as usize);
        }
    });
}

/// The user's pointer speed before a sensitivity button changed it.
static SAVED_SPEED: Mutex<Option<u32>> = Mutex::new(None);

/// Set the pointer speed (1-20) for this session only, or restore the saved
/// one with `None`. Not written to the registry, so a crash or reboot always
/// gets the user's own setting back.
fn set_pointer_speed(speed: Option<u32>) {
    let Ok(mut saved) = SAVED_SPEED.lock() else { return };
    unsafe {
        match speed {
            Some(speed) => {
                if saved.is_none() {
                    let mut current: u32 = 10;
                    if SystemParametersInfoW(
                        SPI_GETMOUSESPEED,
                        0,
                        Some(&mut current as *mut u32 as *mut c_void),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    )
                    .is_ok()
                    {
                        *saved = Some(current);
                    }
                }
                let _ = SystemParametersInfoW(
                    SPI_SETMOUSESPEED,
                    0,
                    Some(speed.clamp(1, 20) as usize as *mut c_void),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
            }
            None => {
                if let Some(original) = saved.take() {
                    let _ = SystemParametersInfoW(
                        SPI_SETMOUSESPEED,
                        0,
                        Some(original as usize as *mut c_void),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    );
                }
            }
        }
    }
}
