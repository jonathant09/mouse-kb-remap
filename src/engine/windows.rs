//! Windows input backend.
//!
//! One dedicated thread owns:
//! * a low-level mouse hook and keyboard hook (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`)
//! * a foreground-window event hook (`EVENT_SYSTEM_FOREGROUND`)
//! * two thread timers: one finishes tap / hold decisions, the other
//!   re-checks the focused window's title and fullscreen state (only while a
//!   profile has rules that need it)
//! * the message loop those need.
//!
//! The hook callbacks must return quickly: Windows silently removes
//! low-level hooks that time out. So they only do a non-blocking lookup in a
//! pre-built table, a `SendInput` and maybe a channel send; anything timed
//! (macros, turbo, text) runs on a separate worker thread (`player.rs`).
//!
//! Per-device bindings: the hook can't tell which mouse a button came from.
//! When the active profile has a binding limited to a device, presses of
//! that button are held back (swallowed) and queued until the raw input
//! thread (`win_devices.rs`) reports the device, normally well under a
//! millisecond later; then they are dispatched, or sent on unchanged.
//! Controller events arrive from the controller thread as thread messages.

use super::dispatch::{wheel_for, Ctx, Dispatcher, Phase, Sink};
use super::pad::PointerSettings;
use super::player::{self, Cmd, Out};
use super::win_devices::{self, WM_PAD, WM_RAW_BUTTON};
use super::{
    pick_profile, AppCommand, Compiled, CompiledProfile, DeviceIndex, EngineEvent, FocusInfo, HotkeyBinding, Input,
    Layers, RecEvent, WindowInfo,
};
use crate::config::{MouseButton, PadSettings};

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, SetThreadPriority, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE,
    MAPVK_VK_TO_VSC_EX, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, KillTimer, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, SystemParametersInfoW, TranslateMessage, UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND,
    HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SPI_GETMOUSESPEED, SPI_SETMOUSESPEED,
    SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WHEEL_DELTA, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WINEVENT_OUTOFCONTEXT, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN,
    WM_XBUTTONUP, XBUTTON1, XBUTTON2,
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

/// Thread message asking the engine thread to re-check whether the mouse
/// hook and the window poll are needed.
const WM_UPDATE_HOOKS: u32 = WM_APP + 1;
/// Thread message: the profile changed from another thread (config change,
/// forced profile); toggled layers go off.
const WM_RESET_LAYERS: u32 = WM_APP + 2;

/// How often the focused window's title / fullscreen state is re-checked
/// while a profile has a title or fullscreen rule.
const WINDOW_POLL_MS: u32 = 500;

/// A held-back mouse button waits at most this long for its device report
/// (only reached if raw input fails); then it counts as "no named device".
const DEVICE_WAIT: Duration = Duration::from_millis(30);
/// A device report that came before its hook event is kept this long.
const EARLY_REPORT: Duration = Duration::from_millis(20);

/// A mouse event held back until its device is known.
struct Deferred {
    input: Input,
    phase: Phase,
    /// The event as the hook saw it, to send on unchanged.
    msg: u32,
    mouse_data: u32,
    /// `RI_MOUSE_*` flag of the raw report that belongs to it.
    flag: u16,
    /// Whether the binding depends on the device. Others only queue to
    /// keep their order.
    needs_device: bool,
    /// Filled in when the report arrives: Some(None) = not a named device.
    device: Option<Option<DeviceIndex>>,
    at: Instant,
}

/// Engine thread id, for posting it messages.
static ENGINE_TID: AtomicU32 = AtomicU32::new(0);

struct Recording {
    start: Instant,
    events: Vec<(Duration, RecEvent)>,
}

/// The focused window as last seen.
#[derive(Default, Clone)]
struct Focus {
    hwnd: usize,
    /// Executable file name as Windows reports it (for the UI).
    exe: String,
    window: WindowInfo,
    is_self: bool,
    elevated: bool,
}

struct Shared {
    profiles: RwLock<Vec<CompiledProfile>>,
    hotkeys: RwLock<Arc<Vec<HotkeyBinding>>>,
    /// Bindings for the currently focused window (`None` while our own
    /// window is focused). Swapped on focus change.
    active: RwLock<Option<Arc<Layers>>>,
    active_index: AtomicUsize,
    /// Profile chosen by hand, whatever app is focused.
    forced: Mutex<Option<usize>>,
    enabled: AtomicBool,
    self_pid: u32,
    self_elevated: bool,
    accept_injected: bool,
    focus: Mutex<Focus>,
    focus_self: AtomicBool,
    on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>,
    worker: Sender<Cmd>,
    capture: AtomicBool,
    recording: AtomicBool,
    record: Mutex<Option<Recording>>,
    stick_speed: AtomicU32,
    pad: Mutex<PadSettings>,
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
    /// The mouse hook, when installed (only while it is needed).
    static MOUSE_HOOK: Cell<Option<HHOOK>> = const { Cell::new(None) };
    /// Remove the mouse hook once no remapped button is held any more.
    static PENDING_UNHOOK: Cell<bool> = const { Cell::new(false) };
    /// Thread timer ids (0 = not running).
    static TAP_TIMER: Cell<usize> = const { Cell::new(0) };
    static POLL_TIMER: Cell<usize> = const { Cell::new(0) };
    static DEVICE_TIMER: Cell<usize> = const { Cell::new(0) };
    /// Mouse events waiting for their device, oldest first.
    static DEFERRED: RefCell<VecDeque<Deferred>> = const { RefCell::new(VecDeque::new()) };
    /// Device reports no held-back event claimed yet: (flags, device, when).
    static EARLY: RefCell<VecDeque<(u16, Option<DeviceIndex>, Instant)>> = const { RefCell::new(VecDeque::new()) };
}

pub struct Engine;

impl Engine {
    pub fn start(compiled: Compiled, enabled: bool, on_event: Box<dyn Fn(EngineEvent) + Send + Sync>) -> Self {
        let self_pid = unsafe { GetCurrentProcessId() };
        let self_elevated = process_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = SHARED.set(Shared {
            profiles: RwLock::new(compiled.profiles),
            hotkeys: RwLock::new(Arc::new(compiled.hotkeys)),
            active: RwLock::new(None),
            active_index: AtomicUsize::new(NO_PROFILE),
            forced: Mutex::new(None),
            enabled: AtomicBool::new(enabled),
            self_pid,
            self_elevated,
            accept_injected: std::env::var(ACCEPT_INJECTED_ENV).is_ok_and(|v| v == "1"),
            focus: Mutex::new(Focus {
                is_self: true,
                ..Default::default()
            }),
            focus_self: AtomicBool::new(true),
            on_event: Arc::from(on_event),
            worker: tx,
            capture: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            record: Mutex::new(None),
            stick_speed: AtomicU32::new(compiled.stick_speed),
            pad: Mutex::new(compiled.pad.clone()),
        });
        let sh = shared();
        win_devices::set_devices(compiled.devices);
        win_devices::start(sh.on_event.clone(), sh.accept_injected, MAGIC);

        std::thread::Builder::new()
            .name("remap-player".into())
            .spawn(move || {
                // Timed output (turbo, macros) shouldn't wait behind normal work.
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
                }
                player::run_worker(rx, emit_outs)
            })
            .expect("failed to spawn player thread");
        std::thread::Builder::new()
            .name("remap-engine".into())
            .spawn(engine_thread)
            .expect("failed to spawn engine thread");
        Engine
    }

    pub fn apply(&self, compiled: Compiled, enabled: bool) {
        let sh = shared();
        sh.enabled.store(enabled, Ordering::Relaxed);
        if let Ok(mut p) = sh.profiles.write() {
            *p = compiled.profiles;
            // A deleted profile can't stay forced.
            if let Ok(mut f) = sh.forced.lock() {
                if f.is_some_and(|i| i >= p.len()) {
                    *f = None;
                }
            }
        }
        if let Ok(mut h) = sh.hotkeys.write() {
            *h = Arc::new(compiled.hotkeys);
        }
        sh.stick_speed.store(compiled.stick_speed, Ordering::SeqCst);
        if let Ok(mut p) = sh.pad.lock() {
            *p = compiled.pad.clone();
        }
        win_devices::set_devices(compiled.devices);
        profile_changed_elsewhere();
    }

    pub fn capture_device(&self, on: bool) {
        win_devices::set_capture(on);
        post_engine(WM_UPDATE_HOOKS);
    }

    pub fn detect_pad(&self, on: bool) {
        win_devices::set_detect(on);
    }

    pub fn force_profile(&self, index: Option<usize>) {
        let sh = shared();
        if let Ok(mut f) = sh.forced.lock() {
            *f = index;
        }
        profile_changed_elsewhere();
        emit_focus();
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
        post_engine(WM_UPDATE_HOOKS);
        true
    }

    pub fn stop_recording(&self) -> Vec<(Duration, RecEvent)> {
        let sh = shared();
        sh.recording.store(false, Ordering::SeqCst);
        post_engine(WM_UPDATE_HOOKS);
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

/// Profiles or the forced profile changed on another thread: pick the
/// active profile again and tell the engine thread.
fn profile_changed_elsewhere() {
    let sh = shared();
    let focus = sh.focus.lock().map(|f| f.clone()).unwrap_or_default();
    recompute_active(&focus);
    post_engine(WM_UPDATE_HOOKS);
    post_engine(WM_RESET_LAYERS);
    // Bindings may have changed under a running macro or turbo.
    let _ = sh.worker.send(Cmd::StopAll);
}

fn engine_thread() {
    unsafe {
        // Every hooked event waits for this thread: never let it queue
        // behind ordinary work when the PC is busy. It does very little.
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        ENGINE_TID.store(GetCurrentThreadId(), Ordering::SeqCst);

        let hmod = GetModuleHandleW(PCWSTR::null()).ok();
        // The mouse hook comes and goes with the active profile (see
        // update_mouse_hook); the keyboard hook is always needed for key
        // capture, recording and app hotkeys.
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod.map(Into::into), 0);
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

        on_foreground(GetForegroundWindow(), false);
        update_window_poll();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() {
                match msg.message {
                    WM_UPDATE_HOOKS => {
                        update_mouse_hook();
                        update_window_poll();
                        continue;
                    }
                    WM_RESET_LAYERS => {
                        DISPATCH.with(|d| d.borrow_mut().reset_layers());
                        continue;
                    }
                    WM_TIMER if msg.wParam.0 == TAP_TIMER.get() && msg.wParam.0 != 0 => {
                        on_tap_timer();
                        continue;
                    }
                    WM_TIMER if msg.wParam.0 == POLL_TIMER.get() && msg.wParam.0 != 0 => {
                        on_foreground(GetForegroundWindow(), true);
                        continue;
                    }
                    WM_TIMER if msg.wParam.0 == DEVICE_TIMER.get() && msg.wParam.0 != 0 => {
                        run_deferred(Instant::now());
                        continue;
                    }
                    WM_RAW_BUTTON => {
                        on_device_report(msg.wParam.0 as u16, device_of(msg.lParam));
                        continue;
                    }
                    WM_PAD => {
                        if let Some(b) = win_devices::pad_from_code(msg.wParam.0) {
                            let phase = if msg.wParam.0 & win_devices::PAD_RELEASED != 0 {
                                Phase::Up
                            } else {
                                Phase::Down
                            };
                            handle(Input::Pad(b), phase, device_of(msg.lParam));
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Post a message to the engine thread (from any thread).
fn post_engine(message: u32) {
    post_engine_with(message, WPARAM(0), LPARAM(0));
}

pub(super) fn post_engine_with(message: u32, wparam: WPARAM, lparam: LPARAM) {
    let tid = ENGINE_TID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, message, wparam, lparam);
        }
    }
}

/// Device index from a `WM_RAW_BUTTON` / `WM_PAD` lParam (index + 1, 0 none).
fn device_of(lparam: LPARAM) -> Option<DeviceIndex> {
    (lparam.0 > 0).then(|| (lparam.0 - 1) as DeviceIndex)
}

/// Install the mouse hook only while something needs it: the active profile
/// remaps a mouse button or the wheel, or a macro is being recorded.
/// Otherwise mouse movement and clicks never pass through this app at all.
/// Also starts or stops device reports and controller polling to match.
/// Engine thread only.
fn update_mouse_hook() {
    let sh = shared();
    let active = sh.active.read().ok().and_then(|a| a.clone());
    let wanted = sh.recording.load(Ordering::SeqCst) || active.as_ref().is_some_and(|l| l.uses_mouse());
    let settings = sh.pad.lock().map(|p| p.clone()).unwrap_or_default();
    let pointer = PointerSettings {
        stick: active.as_ref().map(|l| l.stick_pointer).unwrap_or_default(),
        stick_speed: sh.stick_speed.load(Ordering::SeqCst),
        gyro: active.as_ref().is_some_and(|l| l.gyro_pointer),
        gyro_speed: settings.gyro_speed,
        gyro_invert_x: settings.gyro_invert_x,
        gyro_invert_y: settings.gyro_invert_y,
        touch: active.as_ref().is_some_and(|l| l.touch_pointer),
        touch_speed: settings.touch_speed,
    };
    win_devices::update(
        active.as_ref().is_some_and(|l| l.mouse_device_bindings()),
        active.as_ref().is_some_and(|l| l.uses_pad()),
        active
            .as_ref()
            .is_some_and(|l| l.gyro_pointer || l.touch_pointer || l.gyro_bound),
        pointer,
        settings,
    );
    let busy = DISPATCH.with(|d| d.borrow().holds_mouse()) || DEFERRED.with(|q| !q.borrow().is_empty());
    MOUSE_HOOK.with(|hook| match (wanted, hook.get()) {
        (true, None) => {
            PENDING_UNHOOK.with(|p| p.set(false));
            let hmod = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok();
            match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod.map(Into::into), 0) } {
                Ok(h) => {
                    hook.set(Some(h));
                    crate::log!("engine: mouse hook on");
                }
                Err(e) => crate::log!("engine: mouse hook failed: {e}"),
            }
        }
        (true, Some(_)) => PENDING_UNHOOK.with(|p| p.set(false)),
        (false, Some(h)) => {
            // A remapped button still held needs its release seen, or the
            // key it sends would stay down: wait for it.
            if busy {
                PENDING_UNHOOK.with(|p| p.set(true));
            } else {
                PENDING_UNHOOK.with(|p| p.set(false));
                unsafe {
                    let _ = UnhookWindowsHookEx(h);
                }
                hook.set(None);
                DISPATCH.with(|d| d.borrow_mut().forget_mouse());
                crate::log!("engine: mouse hook off");
            }
        }
        (false, None) => {}
    });
}

/// Run the window poll timer only while a profile has a title or
/// fullscreen rule. Engine thread only.
fn update_window_poll() {
    let wanted = shared()
        .profiles
        .read()
        .map(|p| super::needs_window_poll(&p))
        .unwrap_or(false);
    let id = POLL_TIMER.get();
    unsafe {
        if wanted && id == 0 {
            POLL_TIMER.set(SetTimer(None, 0, WINDOW_POLL_MS, None));
        } else if !wanted && id != 0 {
            let _ = KillTimer(None, id);
            POLL_TIMER.set(0);
        }
    }
}

/// Keep the tap / hold timer in step with the dispatcher. Engine thread only.
fn arm_tap_timer() {
    let deadline = DISPATCH.with(|d| d.borrow().deadline());
    let id = TAP_TIMER.get();
    unsafe {
        match deadline {
            Some(at) => {
                let ms = at
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .clamp(1, 60_000) as u32;
                TAP_TIMER.set(SetTimer(None, id, ms, None));
            }
            None if id != 0 => {
                let _ = KillTimer(None, id);
                TAP_TIMER.set(0);
            }
            None => {}
        }
    }
}

fn on_tap_timer() {
    let sh = shared();
    let mut sink = HookSink::new(sh);
    DISPATCH.with(|d| {
        if let Ok(mut d) = d.try_borrow_mut() {
            d.tick(Instant::now(), &mut sink);
        }
    });
    sink.send_all();
    arm_tap_timer();
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
    on_foreground(hwnd, false);
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// Whether the window covers its whole monitor (fullscreen or borderless).
fn is_fullscreen(hwnd: HWND) -> bool {
    unsafe {
        let mut class = [0u16; 32];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        if matches!(String::from_utf16_lossy(&class[..n]).as_str(), "Progman" | "WorkerW") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// The foreground window changed, or (`poll`) its title / fullscreen state
/// may have.
fn on_foreground(hwnd: HWND, poll: bool) {
    if hwnd.is_invalid() {
        return;
    }
    let sh = shared();
    let same_window = sh.focus.lock().is_ok_and(|f| f.hwnd == hwnd.0 as usize);
    if poll && !same_window {
        // A focus change the event hook hasn't reported yet; it will.
        return;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return;
    }
    let is_self = pid == sh.self_pid;
    let focus = if poll {
        let Ok(mut f) = sh.focus.lock().map(|f| f.clone()) else {
            return;
        };
        let title = window_title(hwnd).to_lowercase();
        let fullscreen = is_fullscreen(hwnd);
        if f.window.title == title && f.window.fullscreen == fullscreen {
            return;
        }
        f.window.title = title;
        f.window.fullscreen = fullscreen;
        f
    } else {
        let (path, elevated) = process_info(pid);
        let exe = path.rsplit(['\\', '/']).next().unwrap_or_default().to_string();
        Focus {
            hwnd: hwnd.0 as usize,
            window: WindowInfo {
                exe: exe.to_lowercase(),
                path: path.to_lowercase(),
                title: window_title(hwnd).to_lowercase(),
                fullscreen: is_fullscreen(hwnd),
            },
            exe,
            is_self,
            elevated,
        }
    };
    if let Ok(mut f) = sh.focus.lock() {
        *f = focus.clone();
    }

    if !poll {
        sh.focus_self.store(is_self, Ordering::Relaxed);
        if !is_self {
            // Never keep swallowing keys for a capture the user walked away from.
            sh.capture.store(false, Ordering::SeqCst);
        }
        // Keys released while a window our hooks can't see was in front.
        DISPATCH.with(|d| {
            d.borrow_mut().prune(|i| match *i {
                Input::Key(vk) => {
                    let vk = if vk == crate::keys::VK_NUMPAD_ENTER { 0x0D } else { vk };
                    (unsafe { GetAsyncKeyState(vk as i32) }) < 0
                }
                Input::Mouse(_) | Input::Pad(_) => true,
            })
        });
    }
    let before = sh.active_index.load(Ordering::Relaxed);
    let after = recompute_active(&focus);
    update_mouse_hook();
    if before != after {
        // A toggled turbo or looping macro stops when you switch profile, and
        // focusing this window stops everything (a handy panic button).
        let _ = sh.worker.send(Cmd::StopAll);
        DISPATCH.with(|d| d.borrow_mut().reset_layers());
    }
    if !poll || before != after {
        emit_focus();
    }
}

/// Tell the UI what's focused and which profile is in use.
fn emit_focus() {
    let sh = shared();
    let f = sh.focus.lock().map(|f| f.clone()).unwrap_or_default();
    let index = sh.active_index.load(Ordering::Relaxed);
    let forced = sh.forced.lock().map(|f| f.is_some()).unwrap_or(false);
    (sh.on_event)(EngineEvent::Focus(FocusInfo {
        exe: f.exe,
        is_self: f.is_self,
        elevation_blocked: f.elevated && !sh.self_elevated,
        profile: (index != NO_PROFILE).then_some(index),
        forced: forced && !f.is_self,
    }));
}

/// Pick the active profile; returns its index (`NO_PROFILE` for our window).
fn recompute_active(focus: &Focus) -> usize {
    let sh = shared();
    let (index, layers) = if focus.is_self {
        // Never remap our own window, so a bad binding can't lock you out of the UI.
        (NO_PROFILE, None)
    } else {
        let profiles = match sh.profiles.read() {
            Ok(p) => p,
            Err(_) => return NO_PROFILE,
        };
        let forced = sh.forced.lock().ok().and_then(|f| *f).filter(|i| *i < profiles.len());
        let index = forced.unwrap_or_else(|| pick_profile(&profiles, &focus.window));
        (index, profiles.get(index).map(|p| p.layers.clone()))
    };
    if let Ok(mut a) = sh.active.write() {
        *a = layers;
    }
    sh.active_index.store(index, Ordering::Relaxed);
    index
}

/// Returns (full exe path, is elevated). If we can't inspect the process at
/// all we assume it is elevated, since that is the common reason for failure.
fn process_info(pid: u32) -> (String, bool) {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (String::new(), true);
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path = if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok()
        {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        };
        let elevated = process_elevated(handle).unwrap_or(true);
        let _ = CloseHandle(handle);
        (path, elevated)
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
                let swallow = handle_mouse(input, phase, msg, info.mouseData);
                if phase == Phase::Up
                    && PENDING_UNHOOK.with(|p| p.get())
                    && !DISPATCH.with(|d| d.borrow().holds_mouse())
                {
                    // Finish a deferred unhook (not from inside the callback).
                    post_engine(WM_UPDATE_HOOKS);
                }
                if swallow {
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
            if handle(Input::Key(vk), phase, None) {
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
fn handle(input: Input, phase: Phase, device: Option<DeviceIndex>) -> bool {
    let sh = shared();
    // try_read: never block inside a low-level hook. If a writer holds the
    // lock (a profile switch this instant) the event passes through.
    let active = sh.active.try_read().ok().and_then(|a| a.clone());
    let hotkeys = sh.hotkeys.try_read().ok().map(|h| h.clone()).unwrap_or_default();
    let ctx = Ctx {
        layers: active.as_deref(),
        enabled: sh.enabled.load(Ordering::Relaxed),
        hotkeys: &hotkeys,
        now: Instant::now(),
        device,
    };
    let mut sink = HookSink::new(sh);
    // The dispatcher is never borrowed while output is sent (see HookSink),
    // so this always succeeds; if it ever didn't, pass the event on rather
    // than crash inside a hook.
    let swallow = DISPATCH.with(|d| match d.try_borrow_mut() {
        Ok(mut d) => d.handle(input, phase, &ctx, &mut sink),
        Err(_) => {
            crate::log!("engine: re-entrant event passed on");
            false
        }
    });
    sink.send_all();
    arm_tap_timer();
    swallow
}

/// `RI_MOUSE_*` raw report flag for a hook event.
fn raw_flag(input: Input, phase: Phase) -> u16 {
    let Input::Mouse(b) = input else { return 0 };
    let up = phase == Phase::Up;
    match b {
        MouseButton::Left => {
            if up {
                0x0002
            } else {
                0x0001
            }
        }
        MouseButton::Right => {
            if up {
                0x0008
            } else {
                0x0004
            }
        }
        MouseButton::Middle => {
            if up {
                0x0020
            } else {
                0x0010
            }
        }
        MouseButton::Back => {
            if up {
                0x0080
            } else {
                0x0040
            }
        }
        MouseButton::Forward => {
            if up {
                0x0200
            } else {
                0x0100
            }
        }
        MouseButton::WheelUp | MouseButton::WheelDown => 0x0400,
        MouseButton::WheelLeft | MouseButton::WheelRight => 0x0800,
    }
}

/// A mouse event from the hook. Buttons whose binding depends on the device
/// are held back until the device is known; while any are waiting, other
/// buttons queue behind them so the order is kept.
fn handle_mouse(input: Input, phase: Phase, msg: u32, mouse_data: u32) -> bool {
    let sh = shared();
    let needs_device = win_devices::reporting()
        && sh.enabled.load(Ordering::Relaxed)
        && sh
            .active
            .try_read()
            .ok()
            .and_then(|a| a.clone())
            .is_some_and(|l| l.device_specific(&input));
    let queued = DEFERRED.with(|q| !q.borrow().is_empty());
    if !needs_device && !queued {
        return handle(input, phase, None);
    }
    let flag = raw_flag(input, phase);
    let now = Instant::now();
    let mut device = None;
    if needs_device {
        // The report may have come first.
        EARLY.with(|e| {
            let mut e = e.borrow_mut();
            e.retain(|(_, _, at)| now.duration_since(*at) < EARLY_REPORT);
            if let Some(r) = e.iter_mut().find(|(flags, _, _)| flags & flag != 0) {
                r.0 &= !flag;
                device = Some(r.1);
            }
            e.retain(|(flags, _, _)| *flags != 0);
        });
        if !queued {
            if let Some(d) = device {
                return handle(input, phase, d);
            }
        }
    }
    DEFERRED.with(|q| {
        q.borrow_mut().push_back(Deferred {
            input,
            phase,
            msg,
            mouse_data,
            flag,
            needs_device,
            device,
            at: now,
        })
    });
    arm_device_timer();
    true
}

/// The raw input thread reports a mouse's button flags and device: give
/// them to the held-back events waiting for them, oldest first.
fn on_device_report(mut flags: u16, device: Option<DeviceIndex>) {
    DEFERRED.with(|q| {
        for d in q.borrow_mut().iter_mut() {
            if d.needs_device && d.device.is_none() && flags & d.flag != 0 {
                d.device = Some(device);
                flags &= !d.flag;
            }
        }
    });
    if flags != 0 {
        EARLY.with(|e| {
            let mut e = e.borrow_mut();
            if e.len() >= 32 {
                e.pop_front();
            }
            e.push_back((flags, device, Instant::now()));
        });
    }
    run_deferred(Instant::now());
}

/// Dispatch held-back events from the front of the queue as far as their
/// device is known (or they waited too long), sending on the ones nothing
/// is bound to.
fn run_deferred(now: Instant) {
    loop {
        let next = DEFERRED.with(|q| {
            let mut q = q.borrow_mut();
            let ready = q
                .front()
                .is_some_and(|d| !d.needs_device || d.device.is_some() || now.duration_since(d.at) >= DEVICE_WAIT);
            if ready {
                q.pop_front()
            } else {
                None
            }
        });
        let Some(d) = next else { break };
        if !handle(d.input, d.phase, d.device.flatten()) {
            send(&[original_mouse_input(d.msg, d.mouse_data)]);
        }
    }
    arm_device_timer();
    if PENDING_UNHOOK.with(|p| p.get()) && DEFERRED.with(|q| q.borrow().is_empty()) {
        post_engine(WM_UPDATE_HOOKS);
    }
}

/// Keep the device wait timer running while events are held back.
fn arm_device_timer() {
    let waiting = DEFERRED.with(|q| !q.borrow().is_empty());
    let id = DEVICE_TIMER.get();
    unsafe {
        if waiting && id == 0 {
            DEVICE_TIMER.set(SetTimer(None, 0, DEVICE_WAIT.as_millis() as u32, None));
        } else if !waiting && id != 0 {
            let _ = KillTimer(None, id);
            DEVICE_TIMER.set(0);
        }
    }
}

/// A mouse event as the hook saw it, to send on after holding it back.
fn original_mouse_input(msg: u32, mouse_data: u32) -> INPUT {
    let hi = mouse_data >> 16;
    let (flags, data) = match msg {
        WM_LBUTTONDOWN => (MOUSEEVENTF_LEFTDOWN, 0),
        WM_LBUTTONUP => (MOUSEEVENTF_LEFTUP, 0),
        WM_RBUTTONDOWN => (MOUSEEVENTF_RIGHTDOWN, 0),
        WM_RBUTTONUP => (MOUSEEVENTF_RIGHTUP, 0),
        WM_MBUTTONDOWN => (MOUSEEVENTF_MIDDLEDOWN, 0),
        WM_MBUTTONUP => (MOUSEEVENTF_MIDDLEUP, 0),
        WM_XBUTTONDOWN => (MOUSEEVENTF_XDOWN, hi),
        WM_XBUTTONUP => (MOUSEEVENTF_XUP, hi),
        WM_MOUSEHWHEEL => (MOUSEEVENTF_HWHEEL, hi as u16 as i16 as i32 as u32),
        _ => (MOUSEEVENTF_WHEEL, hi as u16 as i16 as i32 as u32),
    };
    mouse_input(flags, data)
}

/// Whether remapping is on (controller pointer movement pauses with it).
pub(super) fn remapping_on() -> bool {
    shared().enabled.load(Ordering::Relaxed)
}

/// Move the pointer by (dx, dy) like a mouse would (controller stick, gyro, touchpad).
pub(super) fn send_move(dx: i32, dy: i32) {
    send(&[move_input(dx, dy, false)]);
}

/// Collects the dispatcher's output and sends it in one go with `send_all`,
/// once the dispatcher is no longer borrowed: `SendInput` from this thread
/// can run this thread's own hooks re-entrantly (with other input that was
/// waiting for them), and those need the dispatcher.
struct HookSink<'a> {
    buf: [INPUT; 32],
    len: usize,
    /// Only used if an event ever produces more than `buf` holds.
    overflow: Vec<INPUT>,
    sh: &'a Shared,
}

impl<'a> HookSink<'a> {
    fn new(sh: &'a Shared) -> Self {
        HookSink {
            buf: [INPUT::default(); 32],
            len: 0,
            overflow: Vec::new(),
            sh,
        }
    }

    fn send_all(&mut self) {
        if self.overflow.is_empty() {
            send(&self.buf[..self.len]);
        } else {
            let mut all = self.buf[..self.len].to_vec();
            all.append(&mut self.overflow);
            send(&all);
        }
        self.len = 0;
    }
}

impl Sink for HookSink<'_> {
    fn out(&mut self, out: Out) {
        let mut push = |i: INPUT| {
            if self.len < self.buf.len() && self.overflow.is_empty() {
                self.buf[self.len] = i;
                self.len += 1;
            } else {
                self.overflow.push(i);
            }
        };
        out_to_inputs(&out, &mut push);
    }

    /// Output is kept in order and sent by `send_all`.
    fn flush(&mut self) {}

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
        let mut p = POINT::default();
        unsafe { GetCursorPos(&mut p) }.ok()?;
        Some((p.x, p.y))
    }

    fn gyro(&mut self, on: bool) {
        win_devices::set_gyro_held(on);
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
            Out::System(command) => {
                send(&batch);
                batch.clear();
                super::win_actions::run(*command, shared().on_event.clone());
            }
            Out::Notice(text) => (shared().on_event)(EngineEvent::Notice(text.clone())),
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
        Out::Move { x, y, absolute } => push(move_input(x, y, absolute)),
        Out::Launch(_) | Out::PointerSpeed(_) | Out::System(_) | Out::Notice(_) => {}
    }
}

/// Keys that must be sent by virtual-key code: media/browser keys and a few
/// whose scan codes don't round-trip through `SendInput`.
fn send_as_vk(vk: u16) -> bool {
    matches!(vk, 0x13 | 0x90 | 0xA6..=0xB7 | 0xE8)
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
    raw_mouse_input(flags, data, 0, 0)
}

/// Relative moves go through like a mouse's (pointer speed applies);
/// absolute ones are pixels on the virtual desktop (all monitors).
fn move_input(x: i32, y: i32, absolute: bool) -> INPUT {
    if !absolute {
        return raw_mouse_input(MOUSEEVENTF_MOVE, 0, x, y);
    }
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2),
        )
    };
    let nx = ((x - vx) as i64 * 65535 / (vw - 1) as i64).clamp(0, 65535) as i32;
    let ny = ((y - vy) as i64 * 65535 / (vh - 1) as i64).clamp(0, 65535) as i32;
    raw_mouse_input(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        0,
        nx,
        ny,
    )
}

fn raw_mouse_input(flags: MOUSE_EVENT_FLAGS, data: u32, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
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
