//! System actions on Windows (`Action::System`): lock, microphone, audio
//! output, and the focused window. Run on the playback thread; anything that
//! talks to COM gets its own short-lived thread.

use super::{AppCommand, EngineEvent};
use crate::config::SystemCommand;
use std::sync::Arc;

use windows::core::{GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToString};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsZoomed,
    PostMessageW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE, WS_EX_TOPMOST,
};

type Notify = Arc<dyn Fn(EngineEvent) + Send + Sync>;

pub fn run(command: SystemCommand, notify: Notify) {
    let notice = |text: String| notify(EngineEvent::Notice(text));
    match command {
        SystemCommand::NextProfile => notify(EngineEvent::Command(AppCommand::NextProfile)),
        SystemCommand::AutoProfile => notify(EngineEvent::Command(AppCommand::AutoProfile)),
        SystemCommand::LockPc => unsafe {
            if let Err(e) = LockWorkStation() {
                crate::log!("system: couldn't lock: {e}");
            }
        },
        SystemCommand::MuteMic => {
            std::thread::spawn(move || match with_com(toggle_mic) {
                Ok(muted) => notify(EngineEvent::Notice(
                    if muted { "Microphone muted" } else { "Microphone on" }.into(),
                )),
                Err(e) => crate::log!("system: couldn't mute the microphone: {e}"),
            });
        }
        SystemCommand::NextAudioOutput => {
            std::thread::spawn(move || match with_com(next_output) {
                Ok(name) => notify(EngineEvent::Notice(format!("Audio output: {name}"))),
                Err(e) => crate::log!("system: couldn't switch audio output: {e}"),
            });
        }
        SystemCommand::MinimizeWindow => {
            if let Some(w) = target_window() {
                unsafe {
                    let _ = ShowWindow(w, SW_MINIMIZE);
                }
            }
        }
        SystemCommand::MaximizeWindow => {
            if let Some(w) = target_window() {
                unsafe {
                    let _ = ShowWindow(w, if IsZoomed(w).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                }
            }
        }
        SystemCommand::AlwaysOnTop => {
            if let Some(w) = target_window() {
                unsafe {
                    let on = GetWindowLongPtrW(w, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 == 0;
                    let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
                    if SetWindowPos(w, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE).is_ok() {
                        notice(format!("Always on top {}", if on { "on" } else { "off" }));
                    }
                }
            }
        }
        SystemCommand::NextMonitor => {
            if let Some(w) = target_window() {
                move_to_next_monitor(w);
            }
        }
        SystemCommand::CloseWindow => {
            if let Some(w) = target_window() {
                unsafe {
                    let _ = PostMessageW(Some(w), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
}

/// The focused window, unless it's the desktop or the taskbar.
fn target_window() -> Option<HWND> {
    unsafe {
        let w = GetForegroundWindow();
        if w.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        if pid == std::process::id() {
            return None;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(w, &mut buf) as usize;
        let class = String::from_utf16_lossy(&buf[..n]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return None;
        }
        Some(w)
    }
}

fn monitor_work_area(m: HMONITOR) -> Option<RECT> {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(m, &mut mi).as_bool().then_some(mi.rcWork) }
}

fn move_to_next_monitor(w: HWND) {
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows::core::BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(m);
        true.into()
    }
    unsafe {
        let mut monitors: Vec<HMONITOR> = Vec::new();
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut monitors as *mut _ as isize));
        if monitors.len() < 2 {
            return;
        }
        // Left to right, so "next" is predictable.
        monitors.sort_by_key(|m| monitor_work_area(*m).map(|r| (r.left, r.top)).unwrap_or_default());
        let current = MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST);
        let i = monitors.iter().position(|m| *m == current).unwrap_or(0);
        let next = monitors[(i + 1) % monitors.len()];
        let (Some(from), Some(to)) = (monitor_work_area(current), monitor_work_area(next)) else {
            return;
        };
        let maximized = IsZoomed(w).as_bool();
        if maximized {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        let mut r = RECT::default();
        if GetWindowRect(w, &mut r).is_err() {
            return;
        }
        let (tw, th) = (to.right - to.left, to.bottom - to.top);
        let width = (r.right - r.left).min(tw);
        let height = (r.bottom - r.top).min(th);
        // Same offset from the top-left of the work area, kept on screen.
        let x = (to.left + (r.left - from.left)).clamp(to.left, to.right - width);
        let y = (to.top + (r.top - from.top)).clamp(to.top, to.bottom - height);
        let _ = SetWindowPos(w, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
        if maximized {
            let _ = ShowWindow(w, SW_MAXIMIZE);
        }
    }
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

fn with_com<T>(f: impl FnOnce(&IMMDeviceEnumerator) -> windows::core::Result<T>) -> Result<T, String> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .and_then(|e| f(&e))
            .map_err(|e| e.message());
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

/// Toggle mute on the default microphone (and the default communications
/// microphone, if that's a different one). Returns whether it's now muted.
fn toggle_mic(e: &IMMDeviceEnumerator) -> windows::core::Result<bool> {
    unsafe {
        let console = e.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        let volume: IAudioEndpointVolume = console.Activate(CLSCTX_ALL, None)?;
        let mute = !volume.GetMute()?.as_bool();
        volume.SetMute(mute, std::ptr::null())?;
        if let Ok(comms) = e.GetDefaultAudioEndpoint(eCapture, eCommunications) {
            if device_id(&comms).ok() != device_id(&console).ok() {
                if let Ok(v) = comms.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let _ = v.SetMute(mute, std::ptr::null());
                }
            }
        }
        Ok(mute)
    }
}

fn device_id(d: &IMMDevice) -> windows::core::Result<String> {
    unsafe {
        let id: PWSTR = d.GetId()?;
        let s = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.0 as *const _));
        Ok(s)
    }
}

fn device_name(d: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = d.OpenPropertyStore(STGM_READ) else {
            return String::new();
        };
        let Ok(mut value) = store.GetValue(&PKEY_Device_FriendlyName) else {
            return String::new();
        };
        let mut buf = [0u16; 256];
        let name = if PropVariantToString(&value, &mut buf).is_ok() {
            let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..n])
        } else {
            String::new()
        };
        let _ = PropVariantClear(&mut value);
        name
    }
}

/// Windows has no public API to change the default audio device; this
/// undocumented interface is what the Sound settings use (and what tools like
/// EarTrumpet and SoundSwitch rely on). Only `SetDefaultEndpoint` is called;
/// the others keep the method slots in order.
#[allow(non_snake_case, dead_code)]
mod policy {
    use windows::core::{interface, HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::ERole;

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows::core::IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, default: i32, format: *mut *mut core::ffi::c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, a: *mut core::ffi::c_void, b: *mut core::ffi::c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, id: PCWSTR, default: i32, a: *mut i64, b: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, a: *mut i64) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *mut core::ffi::c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, key: *const core::ffi::c_void, value: *mut core::ffi::c_void)
            -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, key: *const core::ffi::c_void, value: *mut core::ffi::c_void)
            -> HRESULT;
        fn SetDefaultEndpoint(&self, id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }

    pub fn set_default_endpoint(p: &IPolicyConfig, id: PCWSTR, role: ERole) -> HRESULT {
        unsafe { p.SetDefaultEndpoint(id, role) }
    }
}
use policy::{set_default_endpoint, IPolicyConfig};

const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// Make the next active playback device the default. Returns its name.
fn next_output(e: &IMMDeviceEnumerator) -> windows::core::Result<String> {
    unsafe {
        let list = e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let count = list.GetCount()?;
        let devices: Vec<IMMDevice> = (0..count).filter_map(|i| list.Item(i).ok()).collect();
        if devices.len() < 2 {
            return Err(windows::core::Error::new(HRESULT(-1), "only one audio output"));
        }
        let current = device_id(&e.GetDefaultAudioEndpoint(eRender, eConsole)?)?;
        let ids: Vec<String> = devices.iter().map(|d| device_id(d).unwrap_or_default()).collect();
        let i = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = (i + 1) % devices.len();
        let policy: IPolicyConfig = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL)?;
        let wide: Vec<u16> = ids[next].encode_utf16().chain(std::iter::once(0)).collect();
        for role in [eConsole, eMultimedia, eCommunications] {
            set_default_endpoint(&policy, PCWSTR(wide.as_ptr()), role).ok()?;
        }
        Ok(device_name(&devices[next]))
    }
}
