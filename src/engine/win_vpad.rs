//! Windows: the virtual Xbox 360 controller, through the ViGEmBus driver.
//!
//! What it reports comes from `vpad.rs`; this is the driver connection and
//! the thread that owns it. It's plugged in only while the active profile
//! sends controller output, and unplugged a few seconds after that stops,
//! so games only see it while it's wanted.
//!
//! The driver protocol (device interface, control codes and structures) is
//! the one ViGEmClient uses, see github.com/nefarius/ViGEmBus. The driver
//! unplugs our controller by itself when the app exits.

use super::vpad::{self, MouseFeed, XReport, LINGER, MOUSE_TICK, PRODUCT, RETRY, VENDOR};
use super::{EngineEvent, VpadState};

use std::sync::Arc;
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

#[repr(C)]
struct SubmitReport {
    size: u32,
    serial: u32,
    report: XReport,
}

/// Start the thread (it idles until a profile needs the controller).
pub fn start(on_event: Arc<dyn Fn(EngineEvent) + Send + Sync>) {
    vpad::set_events(on_event);
    std::thread::Builder::new()
        .name("remap-vpad".into())
        .spawn(run)
        .expect("failed to spawn virtual controller thread");
}

/// ViGEmBus serial number of our controller while plugged in (0: none).
static SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Our controller's ViGEmBus serial number (its device is
/// `USBVID_045E&PID_028E<serial>`), while plugged in.
pub fn serial() -> Option<u32> {
    match SERIAL.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

/// The XInput slot (0-3) of our controller, once known.
pub fn slot() -> Option<u32> {
    vpad::peek(|s| s.slot)
}

/// Whether a new Xbox 360 controller may be ours: it's plugged in and its
/// slot isn't known yet.
pub fn finding_slot() -> bool {
    vpad::peek(|s| s.plugged && s.slot.is_none())
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
    let mut feed = MouseFeed::default();
    loop {
        let (wanted, retry, settings, slot_known) = vpad::peek(|s| (s.wanted, s.retry, s.mouse, s.slot.is_some()));
        if retry {
            vpad::with(|s| s.retry = false);
            next_try = Instant::now();
        }
        let lean = feed.next(if wanted { settings } else { Default::default() });
        let report = vpad::peek(|s| s.report(lean));

        if wanted && plugged.is_none() && Instant::now() >= next_try {
            match Bus::open() {
                Ok(bus) => {
                    vpad::with(|s| s.plugged = true);
                    let before = xinput_slots();
                    match bus.plug_in() {
                        Ok(serial) => {
                            SERIAL.store(serial, std::sync::atomic::Ordering::Relaxed);
                            vpad::set_status(VpadState::Plugged { slot: None });
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
                            vpad::with(|s| s.plugged = false);
                            vpad::set_status(VpadState::Failed(e));
                            next_try = Instant::now() + RETRY;
                        }
                    }
                }
                Err(BusError::Missing) => {
                    vpad::set_status(VpadState::NoDriver);
                    next_try = Instant::now() + RETRY;
                }
                Err(BusError::Other(e)) => {
                    vpad::set_status(VpadState::Failed(e));
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
                    vpad::with(|s| s.slot = Some(slot as u32));
                    vpad::set_status(VpadState::Plugged {
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
                SERIAL.store(0, std::sync::atomic::Ordering::Relaxed);
                // Keep its slot skipped until XInput has seen it go.
                std::thread::sleep(Duration::from_millis(500));
            }
            vpad::with(|s| {
                s.slot = None;
                s.plugged = false;
            });
            vpad::set_status(VpadState::Unplugged);
        }

        // Sleep until something changes, or a timer is due. While the mouse
        // leans a stick, tick to follow it.
        let timeout = match &plugged {
            Some(_) if lean.is_some() => MOUSE_TICK,
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
        vpad::wait(timeout, |s| {
            s.wanted != wanted || s.retry || s.mouse != settings || s.report(lean) != report
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_layout_matches_the_driver() {
        assert_eq!(std::mem::size_of::<XReport>(), 12);
        assert_eq!(std::mem::size_of::<SubmitReport>(), 20);
        assert_eq!(std::mem::size_of::<PluginTarget>(), 16);
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

    /// Records a macro from a stand-in controller while no profile uses
    /// controllers (so they're read only because of the recording). Needs
    /// ViGEmBus; run with
    /// `cargo test record_stand_in_controller -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn record_stand_in_controller() {
        use crate::config::{MacroStep, PadButton};
        let engine = crate::engine::Engine::start(&crate::config::Config::default(), |_| {});
        std::thread::sleep(Duration::from_millis(300));
        assert!(engine.start_recording());
        let bus = Bus::open().expect("ViGEmBus not installed");
        let serial = bus.plug_in().expect("plug in");
        let hold = |buttons: u16, rt: u8, ms: u64| {
            let report = XReport {
                buttons,
                rt,
                ..Default::default()
            };
            let until = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < until {
                let _ = bus.submit(serial, report);
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        // Windows sets the controller up, and new ones are looked for once
        // a second: wait with nothing pressed first.
        hold(0, 0, 2500);
        hold(0x1000, 0, 400); // A
        hold(0, 0, 300);
        hold(0, 255, 400); // right trigger
        hold(0, 0, 300);
        bus.unplug(serial);
        let events = engine.stop_recording();
        println!("{events:?}");
        let steps: Vec<_> = crate::engine::recording_to_steps(&events)
            .into_iter()
            .filter(|s| !matches!(s, MacroStep::Delay { .. }))
            .collect();
        assert_eq!(
            steps,
            vec![
                MacroStep::PadDown { button: PadButton::A },
                MacroStep::PadUp { button: PadButton::A },
                MacroStep::PadDown { button: PadButton::Rt },
                MacroStep::PadUp { button: PadButton::Rt },
            ]
        );
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
