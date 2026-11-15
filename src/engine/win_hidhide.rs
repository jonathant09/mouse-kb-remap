//! Windows: hiding the real controllers from games with HidHide, while the
//! virtual controller copies them (so a game sees only the remapped one).
//!
//! HidHide is a filter driver (github.com/nefarius/HidHide) with a list of
//! devices to hide (the blacklist) from every program not on its list of
//! allowed programs (the whitelist). The app adds itself to the whitelist,
//! so it keeps reading the controllers, then the controllers to the
//! blacklist, and turns hiding on.
//!
//! What it hides: the controllers the app reads (XInput ones: their XUSB
//! device and its HID `IG_` part; others: their HID gamepad interface),
//! never a virtual one (ViGEmBus children, Steam's 28DE:11FF).
//!
//! Undoing it: newer HidHide drivers keep a per-process list that the
//! driver forgets when the process ends (`IOCTL_ADD_SESSION_BLACKLIST`).
//! Older ones (1.5.230, the current release) only have the saved list, so
//! what the app added there is written to `hidhide-restore.json` next to
//! the config first and taken out again when hiding stops, when the app
//! quits, or at the next start after a crash.
//!
//! Hiding only applies when a program opens a device: a game that already
//! has the controller open keeps it until it opens it again.

use super::HideState;
use crate::config::PadSettings;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows::core::{w, GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_Registry_PropertyW, CM_Get_Device_IDW, CM_Get_Parent, CM_Locate_DevNodeW,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    CM_DRP_SERVICE, CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, MAX_PATH};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, QueryDosDeviceW, FILE_ATTRIBUTE_NORMAL,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::IO::DeviceIoControl;

/// XInput's device interface (XUSB).
const XUSB_INTERFACE: GUID = GUID::from_u128(0xEC87F1E3_C13B_4100_B5F7_8B84D54260CB);

const fn ctl(function: u32) -> u32 {
    // CTL_CODE(32769, function, METHOD_BUFFERED, FILE_READ_DATA)
    (32769 << 16) | (1 << 14) | (function << 2)
}
const GET_WHITELIST: u32 = ctl(2048);
const SET_WHITELIST: u32 = ctl(2049);
const GET_BLACKLIST: u32 = ctl(2050);
const SET_BLACKLIST: u32 = ctl(2051);
const GET_ACTIVE: u32 = ctl(2052);
const SET_ACTIVE: u32 = ctl(2053);
const GET_INVERSE: u32 = ctl(2054);
const ADD_SESSION_BLACKLIST: u32 = ctl(2056);
const CLR_SESSION_BLACKLIST: u32 = ctl(2057);

/// Look for new or removed controllers this often while hiding.
const RESCAN: Duration = Duration::from_secs(2);

/// What the app changed in HidHide's saved settings, to undo it.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
struct Restore {
    #[serde(default)]
    blacklist_added: Vec<String>,
    #[serde(default)]
    whitelist_added: Vec<String>,
    /// Hiding was off and the app turned it on.
    #[serde(default)]
    activated: bool,
}

/// What the profile and settings ask for now.
#[derive(Clone, Default)]
struct Wanted {
    on: bool,
    settings: PadSettings,
}

static WANTED: Mutex<Option<Wanted>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static STATUS: Mutex<HideState> = Mutex::new(HideState::Off);
static EVENTS: std::sync::OnceLock<Arc<dyn Fn(super::EngineEvent) + Send + Sync>> = std::sync::OnceLock::new();
/// Look at the driver again now (just installed).
static RETRY: AtomicBool = AtomicBool::new(false);
/// The thread's applied state, so `stop()` at exit can undo it.
static APPLIED: Mutex<Option<Applied>> = Mutex::new(None);

/// Start the thread; first undo what a crashed run may have left.
pub fn start(on_event: Arc<dyn Fn(super::EngineEvent) + Send + Sync>) {
    let _ = EVENTS.set(on_event);
    undo_saved();
    std::thread::Builder::new()
        .name("remap-hidhide".into())
        .spawn(run)
        .expect("failed to spawn HidHide thread");
}

/// Whether the active profile copies a real controller to the virtual one
/// with hiding turned on.
pub fn set_wanted(on: bool, settings: &PadSettings) {
    let mut w = WANTED.lock().unwrap_or_else(|e| e.into_inner());
    *w = Some(Wanted {
        on,
        settings: settings.clone(),
    });
    WAKE.notify_all();
}

pub fn retry() {
    RETRY.store(true, Ordering::SeqCst);
    WAKE.notify_all();
}

pub fn status() -> HideState {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Whether the HidHide driver is installed and answers.
pub fn driver_present() -> bool {
    Control::open().is_ok()
}

/// Undo the hiding now (the app is quitting).
pub fn stop() {
    let mut applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(a) = applied.take() {
        a.undo();
    }
}

fn set_status(s: HideState) {
    let changed = {
        let mut cur = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        let changed = *cur != s;
        *cur = s.clone();
        changed
    };
    if changed {
        match &s {
            HideState::Hiding(n) => crate::log!("hidhide: hiding {n} controller device(s) from games"),
            HideState::Off => crate::log!("hidhide: not hiding"),
            HideState::NotInstalled => crate::log!("hidhide: not installed"),
            HideState::Failed(e) => crate::log!("hidhide: {e}"),
        }
        if let Some(f) = EVENTS.get() {
            f(super::EngineEvent::Hide(s));
        }
    }
}

fn run() {
    loop {
        let wanted = WANTED.lock().ok().and_then(|w| w.clone()).unwrap_or_default();
        RETRY.store(false, Ordering::SeqCst);
        let on = wanted.on && super::vpad::is_plugged();
        let mut applied = APPLIED.lock().unwrap_or_else(|e| e.into_inner());
        if on {
            let devices = controllers(&wanted.settings);
            match applied.as_mut() {
                // No real controller (any more): nothing to change in HidHide.
                _ if devices.is_empty() => {
                    if let Some(a) = applied.take() {
                        a.undo();
                    }
                    set_status(if driver_present() {
                        HideState::Off
                    } else {
                        HideState::NotInstalled
                    });
                }
                Some(a) if a.devices == devices => {}
                Some(a) => {
                    if let Err(e) = a.change(devices) {
                        set_status(HideState::Failed(e));
                    }
                }
                None => match Applied::hide(devices) {
                    Ok(a) => *applied = Some(a),
                    Err(Error::Missing) => set_status(HideState::NotInstalled),
                    Err(Error::Other(e)) => set_status(HideState::Failed(e)),
                },
            }
            if let Some(a) = applied.as_ref() {
                set_status(HideState::Hiding(a.devices.len()));
            }
        } else {
            if let Some(a) = applied.take() {
                a.undo();
            }
            set_status(if wanted.on && !driver_present() {
                HideState::NotInstalled
            } else {
                HideState::Off
            });
        }
        drop(applied);
        let guard = WANTED.lock().unwrap_or_else(|e| e.into_inner());
        let plugged = super::vpad::is_plugged();
        let timeout = if wanted.on { RESCAN } else { Duration::from_secs(60) };
        let _ = WAKE.wait_timeout_while(guard, timeout, |w| {
            let w = w.clone().unwrap_or_default();
            w.on == wanted.on
                && w.settings == wanted.settings
                && !RETRY.load(Ordering::SeqCst)
                && super::vpad::is_plugged() == plugged
        });
    }
}

/// Wake the thread: the virtual controller was plugged in or out.
pub fn vpad_changed() {
    WAKE.notify_all();
}

// ---------------------------------------------------------------------------
// Applying and undoing
// ---------------------------------------------------------------------------

struct Applied {
    devices: BTreeSet<String>,
    /// The driver keeps the list for us and forgets it when we exit.
    session: bool,
    restore: Restore,
}

impl Applied {
    fn hide(devices: BTreeSet<String>) -> Result<Applied, Error> {
        let c = Control::open()?;
        if c.get_bool(GET_INVERSE).map_err(Error::Other)? {
            return Err(Error::Other(
                "HidHide's \"inverse\" mode is on (its list of programs hides devices from them instead): turn it off in the HidHide Configuration Client".into(),
            ));
        }
        let mut restore = Restore::default();
        // Allowed first, so the app never loses its own controllers.
        let me = own_image_name().ok_or_else(|| Error::Other("can't tell the app's own path for HidHide".into()))?;
        let mut whitelist = c.get_list(GET_WHITELIST).map_err(Error::Other)?;
        if !whitelist.iter().any(|w| w.eq_ignore_ascii_case(&me)) {
            whitelist.push(me.clone());
            restore.whitelist_added.push(me);
            save(&restore);
            c.set_list(SET_WHITELIST, &whitelist).map_err(Error::Other)?;
        }
        let list: Vec<String> = devices.iter().cloned().collect();
        let session = match c.set_list(ADD_SESSION_BLACKLIST, &list) {
            Ok(()) => true,
            Err(_) => {
                // An older driver: the saved list, noting what was added.
                let mut blacklist = c.get_list(GET_BLACKLIST).map_err(Error::Other)?;
                for d in &list {
                    if !blacklist.iter().any(|b| b.eq_ignore_ascii_case(d)) {
                        blacklist.push(d.clone());
                        restore.blacklist_added.push(d.clone());
                    }
                }
                save(&restore);
                c.set_list(SET_BLACKLIST, &blacklist).map_err(Error::Other)?;
                false
            }
        };
        if !c.get_bool(GET_ACTIVE).map_err(Error::Other)? {
            restore.activated = true;
            save(&restore);
            c.set_bool(SET_ACTIVE, true).map_err(Error::Other)?;
        }
        crate::log!(
            "hidhide: hiding {} ({}): {}",
            list.len(),
            if session { "until the app exits" } else { "saved list" },
            list.join(", ")
        );
        Ok(Applied {
            devices,
            session,
            restore,
        })
    }

    /// Controllers came or went.
    fn change(&mut self, devices: BTreeSet<String>) -> Result<(), String> {
        let c = Control::open().map_err(|e| e.text())?;
        let list: Vec<String> = devices.iter().cloned().collect();
        if self.session {
            c.call(CLR_SESSION_BLACKLIST, &[], false)?;
            c.set_list(ADD_SESSION_BLACKLIST, &list)?;
        } else {
            let mut blacklist = c.get_list(GET_BLACKLIST)?;
            // Take out what we added that's gone, add what's new.
            let gone: Vec<String> = self
                .restore
                .blacklist_added
                .iter()
                .filter(|d| !devices.contains(*d))
                .cloned()
                .collect();
            blacklist.retain(|b| !gone.iter().any(|g| g.eq_ignore_ascii_case(b)));
            self.restore.blacklist_added.retain(|d| devices.contains(d));
            for d in &list {
                if !blacklist.iter().any(|b| b.eq_ignore_ascii_case(d)) {
                    blacklist.push(d.clone());
                    self.restore.blacklist_added.push(d.clone());
                }
            }
            save(&self.restore);
            c.set_list(SET_BLACKLIST, &blacklist)?;
        }
        crate::log!("hidhide: now hiding {}: {}", list.len(), list.join(", "));
        self.devices = devices;
        Ok(())
    }

    fn undo(self) {
        if let Ok(c) = Control::open() {
            if self.session {
                let _ = c.call(CLR_SESSION_BLACKLIST, &[], false);
            }
        }
        undo(&self.restore);
    }
}

/// Take out what `r` says the app added.
fn undo(r: &Restore) {
    let Ok(c) = Control::open() else {
        crate::log!("hidhide: can't reach the driver to undo the hiding");
        return;
    };
    let mut ok = true;
    if !r.blacklist_added.is_empty() {
        match c.get_list(GET_BLACKLIST) {
            Ok(mut list) => {
                list.retain(|b| !r.blacklist_added.iter().any(|a| a.eq_ignore_ascii_case(b)));
                ok &= c.set_list(SET_BLACKLIST, &list).is_ok();
            }
            Err(_) => ok = false,
        }
    }
    if r.activated {
        ok &= c.set_bool(SET_ACTIVE, false).is_ok();
    }
    if !r.whitelist_added.is_empty() {
        match c.get_list(GET_WHITELIST) {
            Ok(mut list) => {
                list.retain(|w| !r.whitelist_added.iter().any(|a| a.eq_ignore_ascii_case(w)));
                ok &= c.set_list(SET_WHITELIST, &list).is_ok();
            }
            Err(_) => ok = false,
        }
    }
    if ok {
        if let Some(p) = restore_path() {
            let _ = std::fs::remove_file(p);
        }
        crate::log!("hidhide: hiding undone");
    } else {
        crate::log!("hidhide: couldn't undo all of the hiding; will try again at the next start");
    }
}

/// At start: undo what a run that didn't quit cleanly left.
fn undo_saved() {
    let Some(p) = restore_path() else { return };
    let Ok(text) = std::fs::read_to_string(&p) else { return };
    match serde_json::from_str::<Restore>(&text) {
        Ok(r) => {
            crate::log!("hidhide: undoing what the last run left hidden");
            undo(&r);
        }
        Err(_) => {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn restore_path() -> Option<PathBuf> {
    crate::config::Config::path().map(|p| p.with_file_name("hidhide-restore.json"))
}

fn save(r: &Restore) {
    let Some(p) = restore_path() else { return };
    if r == &Restore::default() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(r) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &p))
            .is_err()
        {
            crate::log!("hidhide: can't save {}", p.display());
        }
    }
}

// ---------------------------------------------------------------------------
// The driver's control device
// ---------------------------------------------------------------------------

enum Error {
    Missing,
    Other(String),
}

impl Error {
    fn text(self) -> String {
        match self {
            Error::Missing => "HidHide isn't installed".into(),
            Error::Other(e) => e,
        }
    }
}

struct Control(HANDLE);

impl Drop for Control {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Control {
    fn open() -> Result<Control, Error> {
        unsafe {
            CreateFileW(
                w!("\\\\.\\HidHide"),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map(Control)
        .map_err(|e| match e.code().0 as u32 & 0xFFFF {
            2 | 3 => Error::Missing,
            _ => Error::Other(format!("can't open HidHide: {}", e.message())),
        })
    }

    /// One request with `input`; returns the reply when `reply` is set.
    fn call(&self, code: u32, input: &[u8], reply: bool) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut done = 0u32;
        let err = |e: windows::core::Error| format!("HidHide refused a request: {}", e.message());
        unsafe {
            let inp = (!input.is_empty()).then_some(input.as_ptr() as *const core::ffi::c_void);
            if reply {
                // Ask for the size first.
                DeviceIoControl(self.0, code, inp, input.len() as u32, None, 0, Some(&mut done), None).map_err(err)?;
                out = vec![0u8; (done as usize).max(2)];
                DeviceIoControl(
                    self.0,
                    code,
                    inp,
                    input.len() as u32,
                    Some(out.as_mut_ptr() as *mut _),
                    out.len() as u32,
                    Some(&mut done),
                    None,
                )
                .map_err(err)?;
                out.truncate(done as usize);
            } else {
                DeviceIoControl(self.0, code, inp, input.len() as u32, None, 0, Some(&mut done), None).map_err(err)?;
            }
        }
        Ok(out)
    }

    fn get_bool(&self, code: u32) -> Result<bool, String> {
        let mut b = [0u8; 1];
        let mut done = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                None,
                0,
                Some(b.as_mut_ptr() as *mut _),
                1,
                Some(&mut done),
                None,
            )
            .map_err(|e| format!("HidHide refused a request: {}", e.message()))?;
        }
        Ok(b[0] != 0)
    }

    fn set_bool(&self, code: u32, v: bool) -> Result<(), String> {
        self.call(code, &[v as u8], false).map(|_| ())
    }

    fn get_list(&self, code: u32) -> Result<Vec<String>, String> {
        Ok(from_multi_sz(&self.call(code, &[], true)?))
    }

    fn set_list(&self, code: u32, list: &[String]) -> Result<(), String> {
        self.call(code, &to_multi_sz(list), false).map(|_| ())
    }
}

/// Strings as a REG_MULTI_SZ: each ends in a NUL, then one more NUL.
fn to_multi_sz(list: &[String]) -> Vec<u8> {
    let mut w: Vec<u16> = Vec::new();
    for s in list {
        w.extend(s.encode_utf16());
        w.push(0);
    }
    w.push(0);
    w.iter().flat_map(|c| c.to_le_bytes()).collect()
}

fn from_multi_sz(bytes: &[u8]) -> Vec<String> {
    let w: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    w.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

// ---------------------------------------------------------------------------
// Which devices
// ---------------------------------------------------------------------------

/// Device instance ids of the real controllers the app reads.
fn controllers(settings: &PadSettings) -> BTreeSet<String> {
    let include_virtual = std::env::var_os("MOUSE_KB_REMAP_HIDE_VIRTUAL").is_some();
    let mut out = BTreeSet::new();
    let skip = |id: &str| {
        let up = id.to_ascii_uppercase();
        up.contains("VID_28DE&PID_11FF") || (!include_virtual && under_vigem(id)) || is_own_vpad(id)
    };
    if settings.xinput {
        // XInput opens the XUSB device; DirectInput and HID readers its
        // `IG_` HID part.
        for id in instances(&XUSB_INTERFACE) {
            if !skip(&id) {
                out.insert(id);
            }
        }
    }
    let hid = unsafe { HidD_GetHidGuid() };
    let readable: BTreeSet<String> = if settings.direct {
        super::win_hid::controller_instances().into_iter().collect()
    } else {
        BTreeSet::new()
    };
    for id in instances(&hid) {
        let up = id.to_ascii_uppercase();
        let xinput_part = up.contains("IG_") && settings.xinput;
        if (xinput_part || readable.contains(&up)) && !skip(&id) {
            out.insert(id);
        }
    }
    out
}

/// Instance ids of the present devices with a device interface.
fn instances(interface: &GUID) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(
            Some(interface),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        ) else {
            return out;
        };
        let mut i = 0;
        loop {
            let mut data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, i, &mut data).is_err() {
                break;
            }
            i += 1;
            let mut buf = [0u16; 512];
            let mut need = 0u32;
            if SetupDiGetDeviceInstanceIdW(set, &data, Some(&mut buf), Some(&mut need)).is_ok() {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                out.push(String::from_utf16_lossy(&buf[..len]));
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    out
}

/// Whether a device hangs under the ViGEmBus driver (a virtual controller:
/// ours, or one of another tool such as DS4Windows).
fn under_vigem(id: &str) -> bool {
    ancestors(id)
        .iter()
        .any(|(_, service)| service.eq_ignore_ascii_case("ViGEmBus"))
}

/// Our own virtual controller, even when other virtual ones are hidden
/// (`MOUSE_KB_REMAP_HIDE_VIRTUAL`, for tests): its XUSB device under
/// ViGEmBus is named after its serial number.
fn is_own_vpad(id: &str) -> bool {
    let Some(serial) = super::win_vpad::serial() else {
        return false;
    };
    let own = format!("USB\\VID_045E&PID_028E\\{serial:02}");
    let chain = ancestors(id);
    id.eq_ignore_ascii_case(&own) || chain.iter().any(|(p, _)| p.eq_ignore_ascii_case(&own))
}

/// (instance id, service) of a device's parents, nearest first.
fn ancestors(id: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let wid: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    let mut node = 0u32;
    unsafe {
        if CM_Locate_DevNodeW(&mut node, PCWSTR(wid.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
            return out;
        }
        for _ in 0..16 {
            let mut parent = 0u32;
            if CM_Get_Parent(&mut parent, node, 0) != CR_SUCCESS {
                break;
            }
            node = parent;
            let mut buf = [0u16; 512];
            let name = if CM_Get_Device_IDW(node, &mut buf, 0) == CR_SUCCESS {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf16_lossy(&buf[..len])
            } else {
                String::new()
            };
            let mut svc = [0u16; 128];
            let mut size = std::mem::size_of_val(&svc) as u32;
            let service = if CM_Get_DevNode_Registry_PropertyW(
                node,
                CM_DRP_SERVICE,
                None,
                Some(svc.as_mut_ptr() as *mut _),
                &mut size,
                0,
            ) == CR_SUCCESS
            {
                let len = svc.iter().position(|&c| c == 0).unwrap_or(svc.len());
                String::from_utf16_lossy(&svc[..len])
            } else {
                String::new()
            };
            out.push((name, service));
        }
    }
    out
}

/// The app's exe as HidHide names programs: `\Device\HarddiskVolume3\…`.
fn own_image_name() -> Option<String> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..n]);
    let wpath: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut mount = [0u16; MAX_PATH as usize];
    unsafe { GetVolumePathNameW(PCWSTR(wpath.as_ptr()), &mut mount) }.ok()?;
    let mlen = mount.iter().position(|&c| c == 0)?;
    let mount_str = String::from_utf16_lossy(&mount[..mlen]);
    let mut volume = [0u16; 64];
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut volume) }.ok()?;
    let vlen = volume.iter().position(|&c| c == 0)?;
    // `\\?\Volume{…}\` → `Volume{…}`
    let vol = String::from_utf16_lossy(&volume[..vlen]);
    let inner = vol.strip_prefix("\\\\?\\")?.trim_end_matches('\\');
    let winner: Vec<u16> = inner.encode_utf16().chain(Some(0)).collect();
    let mut dos = [0u16; 512];
    let dlen = unsafe { QueryDosDeviceW(PCWSTR(winner.as_ptr()), Some(&mut dos)) } as usize;
    if dlen == 0 {
        return None;
    }
    let first = dos.iter().position(|&c| c == 0).unwrap_or(dlen);
    let device = String::from_utf16_lossy(&dos[..first]);
    let rest = path.get(mount_str.len()..)?;
    Some(format!(
        "{}\\{}",
        device.trim_end_matches('\\'),
        rest.trim_start_matches('\\')
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_codes_match_the_driver() {
        // From HidHide's ioctl_contract_tests.cpp: (32769 << 16) |
        // (FILE_READ_DATA << 14) | (function << 2) | METHOD_BUFFERED.
        assert_eq!(GET_WHITELIST, 0x8001_6000);
        assert_eq!(SET_BLACKLIST, 0x8001_600C);
        assert_eq!(CLR_SESSION_BLACKLIST, 0x8001_6024);
    }

    #[test]
    fn multi_strings_round_trip() {
        let list = vec!["HID\\VID_054C&PID_0CE6\\7&1".to_string(), "USB\\X".to_string()];
        let bytes = to_multi_sz(&list);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(from_multi_sz(&bytes), list);
        assert_eq!(to_multi_sz(&[]), vec![0, 0]);
        assert!(from_multi_sz(&[0, 0]).is_empty());
    }

    #[test]
    fn own_path_is_a_device_path() {
        let me = own_image_name().expect("image name");
        assert!(me.starts_with("\\Device\\"), "{me}");
        assert!(me.to_ascii_lowercase().ends_with(".exe"), "{me}");
    }
}
