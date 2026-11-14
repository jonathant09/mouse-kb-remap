//! Windows: the virtual Xbox 360 controller, through the ViGEmBus driver.
//!
//! Inputs bound to the *Controller* action press its buttons, triggers and
//! stick directions. With *Copy the real controller* on, what a real
//! controller does that the profile doesn't remap goes to it as well, so a
//! remapped controller works in games (which should then be kept from seeing
//! the real one, e.g. with HidHide).
//!
//! It's plugged in only while the active profile sends controller output,
//! and unplugged a few seconds after that stops, so games only see it while
//! it's wanted. One thread owns the driver connection; the hooks and the
//! controller threads change the shared state and wake it.
//!
//! The driver protocol (device interface, control codes and structures) is
//! the one ViGEmClient uses, see github.com/nefarius/ViGEmBus. The driver
//! unplugs our controller by itself when the app exits.

use super::pad::{bit, PadReading};
use super::{EngineEvent, VpadState};
use crate::config::PadButton;

use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED, FILE_FLAG_WRITE_THROUGH,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};

/// ViGEmBus's device interface.
const BUS_INTERFACE: GUID = GUID::from_u128(0x96E42B22_F5E9_42F8_B043_ED0F932F014F);

const IOCTL_PLUGIN_TARGET: u32 = 0x2A_A004;
const IOCTL_UNPLUG_TARGET: u32 = 0x2A_A008;
const IOCTL_CHECK_VERSION: u32 = 0x2A_A00C;
const IOCTL_WAIT_DEVICE_READY: u32 = 0x2A_A010;
const IOCTL_XUSB_SUBMIT_REPORT: u32 = 0x2A_A808;

/// Protocol version the driver must speak.
const BUS_VERSION: u32 = 1;
/// Target type: wired Xbox 360 controller.
const XBOX360_WIRED: i32 = 0;
/// A real Xbox 360 controller's ids, which games know best.
pub const VENDOR: u16 = 0x045E;
pub const PRODUCT: u16 = 0x028E;

/// Unplug this long after the profile stops needing it (switching between
/// apps shouldn't plug it in and out every time).
const LINGER: Duration = Duration::from_secs(10);
/// Without the driver, look again this often while it's wanted.
const RETRY: Duration = Duration::from_secs(10);

#[repr(C)]
struct CheckVersion {
    size: u32,
    version: u32,
}

#[repr(C)]
struct PluginTarget {
    size: u32,
    serial: u32,
    kind: i32,
    vendor: u16,
    product: u16,
}

/// `VIGEM_UNPLUG_TARGET` and `VIGEM_WAIT_DEVICE_READY`.
#[repr(C)]
struct Serial {
    size: u32,
    serial: u32,
}

/// `XUSB_REPORT`: the same layout as `XINPUT_GAMEPAD`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct XReport {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

#[repr(C)]
struct SubmitReport {
    size: u32,
    serial: u32,
    report: XReport,
}

/// XInput button flag of a controller output.
pub fn xbox_flag(b: PadButton) -> Option<u16> {
    Some(match b {
        PadButton::Up => 0x0001,
        PadButton::Down => 0x0002,
        PadButton::Left => 0x0004,
        PadButton::Right => 0x0008,
        PadButton::Start => 0x0010,
        PadButton::Back => 0x0020,
        PadButton::Ls => 0x0040,
        PadButton::Rs => 0x0080,
        PadButton::Lb => 0x0100,
        PadButton::Rb => 0x0200,
        PadButton::Guide => 0x0400,
        PadButton::A => 0x1000,
        PadButton::B => 0x2000,
        PadButton::X => 0x4000,
        PadButton::Y => 0x8000,
        _ => return None,
    })
}

/// What a real controller passes on, leaving out the inputs in `excluded`
/// (bits of `pad::bit`): those are remapped, so the game shouldn't see
/// them. A stick direction left out stops the stick going that way.
pub fn mirror_report(r: &PadReading, excluded: u64) -> XReport {
    let kept = |b: PadButton| excluded & bit(b) == 0;
    let mut out = XReport::default();
    for b in PadButton::OUTPUTS {
        if let Some(flag) = xbox_flag(b) {
            if r.buttons & bit(b) != 0 && kept(b) {
                out.buttons |= flag;
            }
        }
    }
    out.lt = if kept(PadButton::Lt) { r.lt } else { 0 };
    out.rt = if kept(PadButton::Rt) { r.rt } else { 0 };
    let axis = |v: i16, plus: PadButton, minus: PadButton| {
        let v = if kept(plus) { v } else { v.min(0) };
        if kept(minus) {
            v
        } else {
            v.max(0)
        }
    };
    out.lx = axis(r.lx, PadButton::LsRight, PadButton::LsLeft);
    out.ly = axis(r.ly, PadButton::LsUp, PadButton::LsDown);
    out.rx = axis(r.rx, PadButton::RsRight, PadButton::RsLeft);
    out.ry = axis(r.ry, PadButton::RsUp, PadButton::RsDown);
    out
}

/// The report: real controllers copied in, then what bindings hold down
/// on top. A stick a binding pushes ignores the real stick meanwhile.
pub fn compose(mirrors: &[(u32, XReport)], held: &[u16; PadButton::OUTPUTS.len()]) -> XReport {
    let mut r = XReport::default();
    let bigger = |a: i16, b: i16| if b.unsigned_abs() > a.unsigned_abs() { b } else { a };
    for (_, m) in mirrors {
        r.buttons |= m.buttons;
        r.lt = r.lt.max(m.lt);
        r.rt = r.rt.max(m.rt);
        r.lx = bigger(r.lx, m.lx);
        r.ly = bigger(r.ly, m.ly);
        r.rx = bigger(r.rx, m.rx);
        r.ry = bigger(r.ry, m.ry);
    }
    let on = |b: PadButton| {
        PadButton::OUTPUTS
            .iter()
            .position(|o| *o == b)
            .is_some_and(|i| held[i] > 0)
    };
    for b in PadButton::OUTPUTS {
        if let Some(flag) = xbox_flag(b) {
            if on(b) {
                r.buttons |= flag;
            }
        }
    }
    if on(PadButton::Lt) {
        r.lt = 255;
    }
    if on(PadButton::Rt) {
        r.rt = 255;
    }
    let stick = |up, down, left, right| -> Option<(i16, i16)> {
        let x = on(right) as i32 - on(left) as i32;
        let y = on(up) as i32 - on(down) as i32;
        if !(on(up) || on(down) || on(left) || on(right)) {
            return None;
        }
        // Diagonals stay on the stick's circle.
        let full = if x != 0 && y != 0 { 23170 } else { 32767 };
        Some(((x * full) as i16, (y * full) as i16))
    };
    if let Some((x, y)) = stick(
        PadButton::LsUp,
        PadButton::LsDown,
        PadButton::LsLeft,
        PadButton::LsRight,
    ) {
        (r.lx, r.ly) = (x, y);
    }
    if let Some((x, y)) = stick(
        PadButton::RsUp,
        PadButton::RsDown,
        PadButton::RsLeft,
        PadButton::RsRight,
    ) {
        (r.rx, r.ry) = (x, y);
    }
    r
}

#[derive(Default)]
struct State {
    /// The active profile sends controller output.
    wanted: bool,
    /// Presses per output (two bindings can hold the same button).
    held: [u16; PadButton::OUTPUTS.len()],
    /// Real controllers being copied: (source key, what they pass on).
    mirrors: Vec<(u32, XReport)>,
    /// Look for the driver again now (it was just installed).
    retry: bool,
    status: VpadState,
    /// XInput slot (0-3) of our controller, so it isn't read as a real one.
    slot: Option<u32>,
    /// Ours is plugged in (or being plugged in): until its slot is known,
    /// any new Xbox 360 controller may be it.
    plugged: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
static EVENTS: OnceLock<Arc<dyn Fn(EngineEvent) + Send + Sync>> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let r = f(guard.get_or_insert_with(State::default));
    WAKE.notify_all();
    r
}

/// Read the state without waking the thread.
fn peek<R>(f: impl FnOnce(&State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Start the thread (it idles until a profile needs the controller).
pub fn start(on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>) {
    let _ = EVENTS.set(on_event);
    std::thread::Builder::new()
        .name("remap-vpad".into())
        .spawn(run)
        .expect("failed to spawn virtual controller thread");
}

/// Whether the active profile sends controller output.
pub fn set_wanted(on: bool) {
    with(|s| s.wanted = on);
}

/// A binding pressed or released an output.
pub fn press(b: PadButton, down: bool) {
    let Some(i) = PadButton::OUTPUTS.iter().position(|o| *o == b) else {
        return;
    };
    with(|s| {
        s.held[i] = if down {
            s.held[i].saturating_add(1)
        } else {
            s.held[i].saturating_sub(1)
        }
    });
}

/// What a real controller passes on now, or `None` when it's gone.
pub fn mirror(source: u32, report: Option<XReport>) {
    with(|s| {
        let i = s.mirrors.iter().position(|(k, _)| *k == source);
        match (i, report) {
            (Some(i), Some(r)) => s.mirrors[i].1 = r,
            (None, Some(r)) => s.mirrors.push((source, r)),
            (Some(i), None) => {
                s.mirrors.remove(i);
            }
            (None, None) => {}
        }
    });
}

/// Look for the driver again (after installing it).
pub fn retry() {
    with(|s| s.retry = true);
}

pub fn status() -> VpadState {
    peek(|s| s.status.clone())
}

/// The XInput slot (0-3) of our controller, once known.
pub fn slot() -> Option<u32> {
    peek(|s| s.slot)
}

/// Whether a new Xbox 360 controller may be ours: it's plugged in and its
/// slot isn't known yet.
pub fn finding_slot() -> bool {
    peek(|s| s.plugged && s.slot.is_none())
}

/// Which XInput slots have a controller. (The driver can't say which slot
/// it got, so ours is the one that wasn't there before plugging it in.)
fn xinput_slots() -> [bool; 4] {
    let mut out = [false; 4];
    for (i, o) in out.iter_mut().enumerate() {
        let mut st = XINPUT_STATE::default();
        *o = unsafe { XInputGetState(i as u32, &mut st) } == 0;
    }
    out
}

/// Whether the ViGEmBus driver is installed and running.
pub fn driver_present() -> bool {
    match Bus::open() {
        Ok(_) => true,
        Err(e) => e != BusError::Missing,
    }
}

fn set_status(status: VpadState) {
    let changed = with(|s| {
        let changed = s.status != status;
        s.status = status.clone();
        changed
    });
    if changed {
        match &status {
            VpadState::Plugged { slot } => crate::log!(
                "virtual controller: plugged in{}",
                slot.map(|n| format!(" as controller {n}")).unwrap_or_default()
            ),
            VpadState::Unplugged => crate::log!("virtual controller: unplugged"),
            VpadState::NoDriver => crate::log!("virtual controller: the ViGEmBus driver isn't installed"),
            VpadState::Failed(e) => crate::log!("virtual controller: {e}"),
        }
        if let Some(f) = EVENTS.get() {
            f(EngineEvent::Vpad(status));
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BusError {
    Missing,
    Other(String),
}

/// An open connection to the driver, closed (and our controller unplugged
/// by the driver) on drop.
struct Bus {
    handle: HANDLE,
    event: HANDLE,
}

impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
            let _ = CloseHandle(self.event);
        }
    }
}

impl Bus {
    /// Find the driver's device and check it speaks our protocol.
    fn open() -> Result<Bus, BusError> {
        let mut found = false;
        unsafe {
            let set = SetupDiGetClassDevsW(
                Some(&BUS_INTERFACE),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .map_err(|e| BusError::Other(format!("can't list devices: {}", e.message())))?;
            let mut result = Err(BusError::Missing);
            let mut i = 0;
            loop {
                let mut data = SP_DEVICE_INTERFACE_DATA {
                    cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                    ..Default::default()
                };
                if SetupDiEnumDeviceInterfaces(set, None, &BUS_INTERFACE, i, &mut data).is_err() {
                    break;
                }
                i += 1;
                found = true;
                let mut need = 0u32;
                let _ = SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut need), None);
                if !(8..=4096).contains(&need) {
                    continue;
                }
                let mut buf = vec![0u64; need as usize / 8 + 1];
                let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), need, None, None).is_err() {
                    continue;
                }
                let path = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
                let handle = match CreateFileW(
                    PCWSTR(path),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH | FILE_FLAG_OVERLAPPED,
                    None,
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        result = Err(BusError::Other(format!("can't open the driver: {}", e.message())));
                        continue;
                    }
                };
                let event = match CreateEventW(None, false, false, PCWSTR::null()) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = CloseHandle(handle);
                        result = Err(BusError::Other(e.message()));
                        continue;
                    }
                };
                let bus = Bus { handle, event };
                let mut check = CheckVersion {
                    size: std::mem::size_of::<CheckVersion>() as u32,
                    version: BUS_VERSION,
                };
                if bus.ioctl(IOCTL_CHECK_VERSION, &mut check, false).is_ok() {
                    result = Ok(bus);
                    break;
                }
                result = Err(BusError::Other(
                    "the installed ViGEmBus driver is a version the app can't use".into(),
                ));
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            if !found {
                return Err(BusError::Missing);
            }
            result
        }
    }

    /// Send a control request with `data` as input (and output, if
    /// `reply`), and wait for it. Returns the Windows error code on failure.
    fn ioctl<T>(&self, code: u32, data: &mut T, reply: bool) -> Result<(), u32> {
        let size = std::mem::size_of::<T>() as u32;
        let ptr = data as *mut T as *mut core::ffi::c_void;
        let mut overlapped = OVERLAPPED {
            hEvent: self.event,
            ..Default::default()
        };
        let mut done = 0u32;
        let code_of = |e: windows::core::Error| (e.code().0 as u32) & 0xFFFF;
        unsafe {
            match DeviceIoControl(
                self.handle,
                code,
                Some(ptr as *const _),
                size,
                if reply { Some(ptr) } else { None },
                if reply { size } else { 0 },
                None,
                Some(&mut overlapped),
            ) {
                Ok(()) => Ok(()),
                // ERROR_IO_PENDING: the driver answers later.
                Err(e) if code_of(e.clone()) == 997 => {
                    GetOverlappedResult(self.handle, &overlapped, &mut done, true).map_err(code_of)
                }
                Err(e) => Err(code_of(e)),
            }
        }
    }

    /// Plug in an Xbox 360 controller. Returns its serial number.
    fn plug_in(&self) -> Result<u32, String> {
        // The driver takes the first free serial number: try them in turn.
        for serial in 1..=16u32 {
            let mut plug = PluginTarget {
                size: std::mem::size_of::<PluginTarget>() as u32,
                serial,
                kind: XBOX360_WIRED,
                vendor: VENDOR,
                product: PRODUCT,
            };
            if self.ioctl(IOCTL_PLUGIN_TARGET, &mut plug, false).is_ok() {
                let mut wait = Serial {
                    size: std::mem::size_of::<Serial>() as u32,
                    serial,
                };
                // Older drivers don't know this request: fine.
                let _ = self.ioctl(IOCTL_WAIT_DEVICE_READY, &mut wait, false);
                return Ok(serial);
            }
        }
        Err("couldn't plug in a virtual controller (all slots in use?)".into())
    }

    fn unplug(&self, serial: u32) {
        let mut un = Serial {
            size: std::mem::size_of::<Serial>() as u32,
            serial,
        };
        let _ = self.ioctl(IOCTL_UNPLUG_TARGET, &mut un, false);
    }

    fn submit(&self, serial: u32, report: XReport) -> Result<(), u32> {
        let mut r = SubmitReport {
            size: std::mem::size_of::<SubmitReport>() as u32,
            serial,
            report,
        };
        self.ioctl(IOCTL_XUSB_SUBMIT_REPORT, &mut r, false)
    }
}

/// The plugged-in controller.
struct Plugged {
    bus: Bus,
    serial: u32,
    last: Option<XReport>,
    /// When the profile stopped wanting it.
    unwanted_since: Option<Instant>,
    /// XInput slots taken before it was plugged in, and how long to look
    /// for its own.
    before: [bool; 4],
    find_until: Instant,
}

fn run() {
    let mut plugged: Option<Plugged> = None;
    let mut next_try = Instant::now();
    loop {
        let (wanted, retry, report, slot_known) = {
            let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let s = guard.get_or_insert_with(State::default);
            let retry = std::mem::take(&mut s.retry);
            (s.wanted, retry, compose(&s.mirrors, &s.held), s.slot.is_some())
        };
        if retry {
            next_try = Instant::now();
        }

        if wanted && plugged.is_none() && Instant::now() >= next_try {
            match Bus::open() {
                Ok(bus) => {
                    with(|s| s.plugged = true);
                    let before = xinput_slots();
                    match bus.plug_in() {
                        Ok(serial) => {
                            set_status(VpadState::Plugged { slot: None });
                            plugged = Some(Plugged {
                                bus,
                                serial,
                                last: None,
                                unwanted_since: None,
                                before,
                                find_until: Instant::now() + Duration::from_secs(5),
                            });
                        }
                        Err(e) => {
                            with(|s| s.plugged = false);
                            set_status(VpadState::Failed(e));
                            next_try = Instant::now() + RETRY;
                        }
                    }
                }
                Err(BusError::Missing) => {
                    set_status(VpadState::NoDriver);
                    next_try = Instant::now() + RETRY;
                }
                Err(BusError::Other(e)) => {
                    set_status(VpadState::Failed(e));
                    next_try = Instant::now() + RETRY;
                }
            }
        }
        if !wanted && plugged.is_none() {
            set_status(VpadState::Unplugged);
        }

        let mut unplug = false;
        if let Some(p) = plugged.as_mut() {
            if wanted {
                p.unwanted_since = None;
            } else {
                let since = *p.unwanted_since.get_or_insert_with(Instant::now);
                unplug = since.elapsed() >= LINGER;
            }
            if !slot_known && Instant::now() < p.find_until {
                // Windows takes a moment to set the controller up.
                let now = xinput_slots();
                if let Some(slot) = (0..4).find(|&i| now[i] && !p.before[i]) {
                    // Now it's really there: send the state again. The driver
                    // skips a report equal to the one it has, and a new
                    // controller shows its left stick slightly off centre in
                    // XInput until a report changes something, so nudge first.
                    let nudge = XReport {
                        lx: report.lx.wrapping_add(1),
                        ..report
                    };
                    let _ = p.bus.submit(p.serial, nudge);
                    p.last = None;
                    with(|s| s.slot = Some(slot as u32));
                    set_status(VpadState::Plugged {
                        slot: Some(slot as u8 + 1),
                    });
                } else if Instant::now() + Duration::from_millis(100) >= p.find_until {
                    crate::log!("virtual controller: couldn't tell which XInput slot it got");
                }
            }
            if p.last != Some(report) && !unplug {
                match p.bus.submit(p.serial, report) {
                    Ok(()) => p.last = Some(report),
                    // 0x37 ERROR_DEV_NOT_EXIST: not ready yet, try again.
                    Err(0x37) => {}
                    Err(e) => {
                        crate::log!("virtual controller: sending failed (error {e}), plugging in again");
                        unplug = true;
                        next_try = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
        }
        if unplug {
            if let Some(p) = plugged.take() {
                let _ = p.bus.submit(p.serial, XReport::default());
                p.bus.unplug(p.serial);
                // Keep its slot skipped until XInput has seen it go.
                std::thread::sleep(Duration::from_millis(500));
            }
            with(|s| {
                s.slot = None;
                s.plugged = false;
            });
            set_status(VpadState::Unplugged);
        }

        // Sleep until something changes, or a timer is due.
        let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let timeout = match &plugged {
            Some(p)
                if p.unwanted_since.is_some()
                    || (!slot_known && Instant::now() < p.find_until)
                    || p.last != Some(report) =>
            {
                Duration::from_millis(100)
            }
            Some(_) => Duration::from_secs(60),
            None if wanted => next_try
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(50)),
            None => Duration::from_secs(60),
        };
        let unchanged = |s: &mut Option<State>| {
            let s = s.get_or_insert_with(State::default);
            s.wanted == wanted && !s.retry && compose(&s.mirrors, &s.held) == report
        };
        let _ = WAKE.wait_timeout_while(guard, timeout, unchanged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(list: &[PadButton]) -> [u16; PadButton::OUTPUTS.len()] {
        let mut h = [0; PadButton::OUTPUTS.len()];
        for b in list {
            h[PadButton::OUTPUTS.iter().position(|o| o == b).unwrap()] += 1;
        }
        h
    }

    #[test]
    fn report_layout_matches_the_driver() {
        assert_eq!(std::mem::size_of::<XReport>(), 12);
        assert_eq!(std::mem::size_of::<SubmitReport>(), 20);
        assert_eq!(std::mem::size_of::<PluginTarget>(), 16);
    }

    #[test]
    fn bindings_press_buttons_triggers_and_sticks() {
        let r = compose(&[], &held(&[PadButton::A, PadButton::Rt, PadButton::LsUp]));
        assert_eq!(r.buttons, 0x1000);
        assert_eq!((r.lt, r.rt), (0, 255));
        assert_eq!((r.lx, r.ly), (0, 32767));
        let r = compose(&[], &held(&[PadButton::RsLeft, PadButton::RsDown]));
        assert_eq!((r.rx, r.ry), (-23170, -23170));
        // Opposite directions cancel out.
        let r = compose(&[], &held(&[PadButton::LsLeft, PadButton::LsRight]));
        assert_eq!((r.lx, r.ly), (0, 0));
    }

    #[test]
    fn mirror_leaves_out_remapped_inputs() {
        let real = PadReading {
            buttons: bit(PadButton::A) | bit(PadButton::Lb) | bit(PadButton::Guide),
            lt: 200,
            lx: 20000,
            ly: -15000,
            ..Default::default()
        };
        let excluded = bit(PadButton::Lb) | bit(PadButton::Lt) | bit(PadButton::LsRight) | bit(PadButton::Guide);
        let m = mirror_report(&real, excluded);
        assert_eq!(m.buttons, 0x1000);
        assert_eq!(m.lt, 0);
        assert_eq!((m.lx, m.ly), (0, -15000));
        // The binding's output goes on top; its stick overrides the real one.
        let r = compose(&[(1, m)], &held(&[PadButton::B, PadButton::LsLeft]));
        assert_eq!(r.buttons, 0x1000 | 0x2000);
        assert_eq!((r.lx, r.ly), (-32767, 0));
    }

    #[test]
    fn two_controllers_merge() {
        let a = XReport {
            buttons: 0x1000,
            lx: 100,
            ..Default::default()
        };
        let b = XReport {
            buttons: 0x2000,
            lx: -9000,
            rt: 50,
            ..Default::default()
        };
        let r = compose(&[(1, a), (2, b)], &held(&[]));
        assert_eq!(r.buttons, 0x3000);
        assert_eq!((r.lx, r.rt), (-9000, 50));
    }

    /// Stands in for a real controller in end-to-end tests: plugs in an
    /// Xbox 360 controller holding `VPAD_TEST_BUTTONS` (XInput flags, hex)
    /// with the left stick at `VPAD_TEST_LX`, for `VPAD_TEST_SECS` seconds.
    #[test]
    #[ignore]
    fn vpad_stand_in_controller() {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
        let buttons = u16::from_str_radix(&env("VPAD_TEST_BUTTONS", "1100"), 16).unwrap();
        let lx: i16 = env("VPAD_TEST_LX", "0").parse().unwrap();
        let secs: u64 = env("VPAD_TEST_SECS", "10").parse().unwrap();
        let bus = Bus::open().expect("ViGEmBus not installed");
        let serial = bus.plug_in().expect("plug in");
        let report = XReport {
            buttons,
            lx,
            ..Default::default()
        };
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            let _ = bus.submit(serial, report);
            std::thread::sleep(Duration::from_millis(200));
        }
        bus.unplug(serial);
    }

    /// Plugs in a virtual controller, presses A and reads it back through
    /// XInput. Needs ViGEmBus installed; run with
    /// `cargo test vpad_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn vpad_round_trip() {
        let bus = Bus::open().expect("ViGEmBus not installed");
        let before = xinput_slots();
        let serial = bus.plug_in().expect("plug in");
        // The first time, Windows takes a moment to set the device up.
        let started = Instant::now();
        let slot = loop {
            let now = xinput_slots();
            if let Some(s) = (0..4u32).find(|&i| now[i as usize] && !before[i as usize]) {
                break s;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "no XInput slot");
            std::thread::sleep(Duration::from_millis(100));
        };
        println!("plugged in as serial {serial}, XInput slot {slot}");
        let report = XReport {
            buttons: 0x1000,
            rt: 255,
            ly: 32767,
            ..Default::default()
        };
        let mut st = XINPUT_STATE::default();
        loop {
            let sent = bus.submit(serial, report);
            std::thread::sleep(Duration::from_millis(100));
            if sent.is_ok() && unsafe { XInputGetState(slot, &mut st) } == 0 && st.Gamepad.wButtons.0 != 0 {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "XInput doesn't see it ({sent:?})"
            );
        }
        println!("ready after {} ms", started.elapsed().as_millis());
        println!(
            "XInput sees buttons {:#06x}, rt {}, ly {}",
            st.Gamepad.wButtons.0, st.Gamepad.bRightTrigger, st.Gamepad.sThumbLY
        );
        assert_eq!(st.Gamepad.wButtons.0, 0x1000);
        assert_eq!(st.Gamepad.bRightTrigger, 255);
        assert_eq!(st.Gamepad.sThumbLY, 32767);
        bus.unplug(serial);
    }
}
