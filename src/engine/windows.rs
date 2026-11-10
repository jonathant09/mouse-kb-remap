//! Windows input backend.
//!
//! One dedicated thread owns:
//! * a low-level mouse hook and keyboard hook (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`)
//! * a foreground-window event hook (`EVENT_SYSTEM_FOREGROUND`)
//! * the message loop those hooks need.
//!
//! The hook callbacks must return quickly: Windows silently removes
//! low-level hooks that time out. So they only do a non-blocking lookup in a
//! pre-built `HashMap` and call `SendInput`; everything else (process
//! lookups, profile matching) happens on focus changes.

use super::{Bindings, CompiledProfile, FocusInfo, Input, Target};
use crate::config::MouseButton;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, SetWindowsHookExW,
    TranslateMessage, EVENT_SYSTEM_FOREGROUND, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG,
    MSLLHOOKSTRUCT, WHEEL_DELTA, WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

pub const SUPPORTED: bool = true;

struct Shared {
    profiles: RwLock<Vec<CompiledProfile>>,
    /// Bindings for the currently focused window. Swapped on focus change.
    active: RwLock<Arc<Bindings>>,
    enabled: AtomicBool,
    self_pid: u32,
    self_elevated: bool,
    last_focus: Mutex<(String, bool)>, // (lower-cased exe, is_self)
    on_focus: Box<dyn Fn(FocusInfo) + Send + Sync>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
    SHARED.get().expect("engine not started")
}

thread_local! {
    /// Inputs that were remapped on press, so the release goes to the same
    /// target even if the profile changed while the button was held.
    static HELD: RefCell<HashMap<Input, Target>> = RefCell::new(HashMap::new());
}

pub struct Engine;

impl Engine {
    pub fn start(
        profiles: Vec<CompiledProfile>,
        enabled: bool,
        on_focus: Box<dyn Fn(FocusInfo) + Send + Sync>,
    ) -> Self {
        let self_pid = unsafe { GetCurrentProcessId() };
        let self_elevated = process_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        let initial = profiles.first().map(|p| p.bindings.clone()).unwrap_or_default();
        let _ = SHARED.set(Shared {
            profiles: RwLock::new(profiles),
            active: RwLock::new(initial),
            enabled: AtomicBool::new(enabled),
            self_pid,
            self_elevated,
            last_focus: Mutex::new((String::new(), true)),
            on_focus,
        });

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
    }

    pub fn elevated(&self) -> bool {
        shared().self_elevated
    }
}

fn engine_thread() {
    unsafe {
        let hmod = GetModuleHandleW(PCWSTR::null()).ok();
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod.map(Into::into), 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod.map(Into::into), 0);
        if let Err(e) = &mouse {
            eprintln!("engine: mouse hook failed: {e}");
        }
        if let Err(e) = &keyboard {
            eprintln!("engine: keyboard hook failed: {e}");
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
    recompute_active(&exe_lower, is_self);

    (sh.on_focus)(FocusInfo {
        exe,
        is_self,
        elevation_blocked: elevated && !sh.self_elevated,
    });
}

fn recompute_active(exe_lower: &str, is_self: bool) {
    let sh = shared();
    let bindings = if is_self {
        // Never remap our own window, so a bad binding can't lock you out of the UI.
        Arc::new(Bindings::new())
    } else {
        let profiles = match sh.profiles.read() {
            Ok(p) => p,
            Err(_) => return,
        };
        profiles
            .iter()
            .skip(1)
            .find(|p| p.apps.iter().any(|a| a == exe_lower))
            .or_else(|| profiles.first())
            .map(|p| p.bindings.clone())
            .unwrap_or_default()
    };
    if let Ok(mut a) = sh.active.write() {
        *a = bindings;
    }
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

#[derive(Clone, Copy)]
enum Phase {
    Down,
    Up,
    /// Wheel steps have no up/down; they are a single "tap".
    Tap,
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.flags & LLMHF_INJECTED == 0 {
            if let Some((input, phase)) = decode_mouse(wparam.0 as u32, info.mouseData) {
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
        WM_MOUSEWHEEL => {
            let delta = hi as i16;
            (
                if delta > 0 {
                    MouseButton::WheelUp
                } else {
                    MouseButton::WheelDown
                },
                Phase::Tap,
            )
        }
        _ => return None,
    };
    Some((Input::Mouse(button), phase))
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if !info.flags.contains(LLKHF_INJECTED) {
            let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            let phase = if down { Phase::Down } else { Phase::Up };
            if handle(Input::Key(info.vkCode as u16), phase) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns true if the original input should be swallowed.
fn handle(input: Input, phase: Phase) -> bool {
    let sh = shared();
    if let Phase::Up = phase {
        // Always honour a remapped press, even if remapping was just disabled.
        if let Some(target) = HELD.with(|h| h.borrow_mut().remove(&input)) {
            emit(target, Phase::Up);
            return true;
        }
        return false;
    }
    if !sh.enabled.load(Ordering::Relaxed) {
        return false;
    }
    // try_read: never block inside a low-level hook.
    let target = match sh.active.try_read() {
        Ok(active) => active.get(&input).copied(),
        Err(_) => None,
    };
    let Some(target) = target else { return false };
    if let Phase::Down = phase {
        HELD.with(|h| h.borrow_mut().insert(input, target));
    }
    emit(target, phase);
    true
}

fn emit(target: Target, phase: Phase) {
    match target {
        Target::Disabled => {}
        Target::Key(vk) => match phase {
            Phase::Down => send(&[key_input(vk, false)]),
            Phase::Up => send(&[key_input(vk, true)]),
            Phase::Tap => send(&[key_input(vk, false), key_input(vk, true)]),
        },
        Target::Mouse(button) => {
            let (down, up, data) = match button {
                MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
                MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
                MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
                MouseButton::Back => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1 as u32),
                MouseButton::Forward => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2 as u32),
                MouseButton::WheelUp | MouseButton::WheelDown => {
                    // A wheel step fires once, on press.
                    if let Phase::Down | Phase::Tap = phase {
                        let delta = if button == MouseButton::WheelUp {
                            WHEEL_DELTA as i32
                        } else {
                            -(WHEEL_DELTA as i32)
                        };
                        send(&[mouse_input(MOUSEEVENTF_WHEEL, delta as u32)]);
                    }
                    return;
                }
            };
            match phase {
                Phase::Down => send(&[mouse_input(down, data)]),
                Phase::Up => send(&[mouse_input(up, data)]),
                Phase::Tap => send(&[mouse_input(down, data), mouse_input(up, data)]),
            }
        }
    }
}

/// Keys are sent as scan codes: many games read scan codes (raw input /
/// DirectInput) and ignore virtual-key-only events.
fn key_input(vk: u16, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    let mut flags = KEYEVENTF_SCANCODE;
    if scan & 0xFF00 == 0xE000 || scan & 0xFF00 == 0xE100 {
        flags = flags | KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags = flags | KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: (scan & 0xFF) as u16,
                dwFlags: KEYBD_EVENT_FLAGS(flags.0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
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
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}
