//! Windows: controllers read directly over HID (USB or Bluetooth):
//! PlayStation, Nintendo Switch and Steam controllers with their own report
//! formats (`pad_hid.rs`), and any other gamepad or joystick through the
//! report descriptor (numbered buttons, sticks, hat).
//!
//! A scanner thread looks for new controllers every two seconds while the
//! active profile uses one; each controller gets a thread that reads its
//! reports. Devices are opened shared, so games and Steam keep reading them
//! too. Xbox-style controllers are left to XInput (`win_devices.rs`).

use super::pad::{hat_bits, PadReading};
use super::pad_hid::{self, Kind, Parser};
use super::win_devices::{self, Source};

use std::collections::HashSet;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature, HidD_GetHidGuid, HidD_GetPreparsedData,
    HidD_GetProductString, HidD_SetFeature, HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps,
    HidP_Input, HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};

const RESCAN: Duration = Duration::from_secs(2);
/// How often a Steam device's own mouse mode is turned off again.
const LIZARD_EVERY: Duration = Duration::from_secs(2);

/// Device paths being read now.
static OPEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// What enumeration found about a HID interface.
struct Found {
    path: String,
    vendor: u16,
    product: u16,
    kind: Option<Kind>,
}

/// Start the scanner (it idles until controllers are wanted).
pub fn start() {
    std::thread::Builder::new()
        .name("remap-hid-scan".into())
        .spawn(|| loop {
            if win_devices::wants_direct() {
                for f in enumerate() {
                    let fresh = OPEN
                        .lock()
                        .map(|mut o| o.get_or_insert_with(HashSet::new).insert(f.path.clone()))
                        .unwrap_or(false);
                    if fresh {
                        let path = f.path.clone();
                        let _ = std::thread::Builder::new().name("remap-hid".into()).spawn(move || {
                            read_device(f);
                            if let Ok(mut o) = OPEN.lock() {
                                if let Some(o) = o.as_mut() {
                                    o.remove(&path);
                                }
                            }
                        });
                    }
                }
            }
            std::thread::sleep(RESCAN);
        })
        .expect("failed to spawn HID scanner");
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn open(path: &str, access: u32, overlapped: bool) -> Option<HANDLE> {
    let w = wide(path);
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            if overlapped {
                FILE_FLAG_OVERLAPPED
            } else {
                FILE_FLAGS_AND_ATTRIBUTES(0)
            },
            None,
        )
        .ok()
    }
}

/// Preparsed report descriptor of an open device, freed on drop.
struct Preparsed(PHIDP_PREPARSED_DATA);

impl Preparsed {
    fn of(h: HANDLE) -> Option<Self> {
        let mut p = PHIDP_PREPARSED_DATA::default();
        unsafe { HidD_GetPreparsedData(h, &mut p) }.then_some(Preparsed(p))
    }

    fn caps(&self) -> Option<HIDP_CAPS> {
        let mut caps = HIDP_CAPS::default();
        (unsafe { HidP_GetCaps(self.0, &mut caps) } == HIDP_STATUS_SUCCESS).then_some(caps)
    }
}

impl Drop for Preparsed {
    fn drop(&mut self) {
        unsafe {
            let _ = HidD_FreePreparsedData(self.0);
        }
    }
}

/// Device instance ids (upper case) of the controllers `enumerate` finds, for
/// hiding them (`win_hidhide.rs`).
pub fn controller_instances() -> Vec<String> {
    enumerate()
        .iter()
        .filter_map(|f| interface_to_instance(&f.path))
        .collect()
}

/// `\\?\hid#vid_054c&pid_0ce6#7&2b4b&0&0000#{4d1e…}` →
/// `HID\VID_054C&PID_0CE6\7&2B4B&0&0000`.
pub fn interface_to_instance(path: &str) -> Option<String> {
    let rest = path.strip_prefix("\\\\?\\").or_else(|| path.strip_prefix("\\\\.\\"))?;
    let end = rest.rfind("#{")?;
    Some(rest[..end].replace('#', "\\").to_ascii_uppercase())
}

/// Controllers worth reading: known ones by vendor / product (Steam ones on
/// their vendor-defined interface), anything else that calls itself a
/// joystick or gamepad. Xbox-style controllers (`IG_` in the path) are
/// XInput's.
fn enumerate() -> Vec<Found> {
    let mut out = Vec::new();
    unsafe {
        let guid = HidD_GetHidGuid();
        let Ok(set) = SetupDiGetClassDevsW(Some(&guid), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE)
        else {
            return out;
        };
        let mut i = 0;
        loop {
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(set, None, &guid, i, &mut data).is_err() {
                break;
            }
            i += 1;
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
            let p = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
            let len = (0..).take_while(|&k| *p.add(k) != 0).count();
            let path = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            if path.to_ascii_uppercase().contains("IG_") {
                continue;
            }
            // Querying needs no access rights, so mice and keyboards (which
            // Windows keeps to itself) can be looked at and skipped.
            let Some(h) = open(&path, 0, false) else { continue };
            let mut attr = HIDD_ATTRIBUTES {
                Size: std::mem::size_of::<HIDD_ATTRIBUTES>() as u32,
                ..Default::default()
            };
            let ok = HidD_GetAttributes(h, &mut attr);
            let caps = Preparsed::of(h).and_then(|p| p.caps());
            let _ = CloseHandle(h);
            let (true, Some(caps)) = (ok, caps) else { continue };
            let kind = Kind::of(attr.VendorID, attr.ProductID);
            let usable = match kind {
                Some(k) if k.is_steam() => caps.UsagePage >= 0xFF00,
                Some(_) => caps.UsagePage == 0x01 && matches!(caps.Usage, 0x04 | 0x05),
                None => {
                    caps.UsagePage == 0x01
                        && matches!(caps.Usage, 0x04 | 0x05 | 0x08)
                        && !(attr.VendorID == pad_hid::VID_VALVE && attr.ProductID == 0x11FF)
                }
            };
            if usable {
                out.push(Found {
                    path,
                    vendor: attr.VendorID,
                    product: attr.ProductID,
                    kind,
                });
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    out
}

fn product_name(h: HANDLE) -> Option<String> {
    let mut buf = [0u16; 128];
    unsafe { HidD_GetProductString(h, buf.as_mut_ptr() as *mut _, std::mem::size_of_val(&buf) as u32) }
        .then(|| {
            let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..end]).trim().to_string()
        })
        .filter(|s| !s.is_empty())
}

/// An overlapped read or write, waited for with a timeout.
struct Io {
    h: HANDLE,
    ov: Box<OVERLAPPED>,
    pending: bool,
}

impl Io {
    fn new(h: HANDLE) -> Option<Self> {
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?;
        let ov = Box::new(OVERLAPPED {
            hEvent: event,
            ..Default::default()
        });
        Some(Io { h, ov, pending: false })
    }

    /// Read one report into `buf`. `Ok(None)`: nothing within `wait`.
    fn read(&mut self, buf: &mut [u8], wait: u32) -> Result<Option<usize>, ()> {
        unsafe {
            if !self.pending {
                match ReadFile(self.h, Some(buf), None, Some(&mut *self.ov)) {
                    Ok(()) => {}
                    Err(e) if e.code() == windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() => {}
                    Err(_) => return Err(()),
                }
                self.pending = true;
            }
            let mut n = 0u32;
            match GetOverlappedResultEx(self.h, &*self.ov, &mut n, wait, false) {
                Ok(()) => {
                    self.pending = false;
                    Ok(Some(n as usize))
                }
                Err(e)
                    if e.code() == windows::core::HRESULT::from_win32(WAIT_TIMEOUT.0)
                        || e.code() == windows::Win32::Foundation::ERROR_IO_INCOMPLETE.to_hresult() =>
                {
                    Ok(None)
                }
                Err(_) => {
                    self.pending = false;
                    Err(())
                }
            }
        }
    }

    fn cancel(&mut self) {
        if self.pending {
            unsafe {
                let _ = CancelIoEx(self.h, Some(&*self.ov));
                let mut n = 0u32;
                let _ = GetOverlappedResultEx(self.h, &*self.ov, &mut n, 1000, false);
            }
            self.pending = false;
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        self.cancel();
        unsafe {
            let _ = CloseHandle(self.ov.hEvent);
        }
    }
}

/// Write an output report (overlapped handle, so wait for it here).
fn write(h: HANDLE, report: &[u8]) -> bool {
    let Some(mut io) = Io::new(h) else { return false };
    unsafe {
        let started = WriteFile(h, Some(report), None, Some(&mut *io.ov));
        if let Err(e) = started {
            if e.code() != windows::Win32::Foundation::ERROR_IO_PENDING.to_hresult() {
                return false;
            }
        }
        let mut n = 0u32;
        let done = GetOverlappedResultEx(h, &*io.ov, &mut n, 200, false).is_ok();
        if !done {
            io.pending = true; // cancelled on drop
        }
        done
    }
}

fn read_device(f: Found) {
    // Read + write lets us set Switch controllers up; some devices only allow reading.
    let (h, writable) = match open(&f.path, GENERIC_READ.0 | GENERIC_WRITE.0, true) {
        Some(h) => (h, true),
        None => match open(&f.path, GENERIC_READ.0, true) {
            Some(h) => (h, false),
            None => {
                crate::log!("controller: can't open {}", f.path);
                return;
            }
        },
    };
    let name = product_name(h)
        .or_else(|| f.kind.map(|k| k.name().to_string()))
        .unwrap_or_else(|| "Controller".into());
    let preparsed = Preparsed::of(h);
    let caps = preparsed.as_ref().and_then(|p| p.caps());
    let report_len = caps.map_or(64, |c| c.InputReportByteLength as usize).max(16);
    crate::log!(
        "controller: reading {name} ({:04x}:{:04x}{})",
        f.vendor,
        f.product,
        if writable { "" } else { ", read-only" }
    );

    let mut parser = f.kind.map(Parser::new);
    let mut generic = match (&parser, &preparsed) {
        (None, Some(p)) => Generic::new(p),
        _ => None,
    };
    let mut source = Source::new(f.path.clone(), name.clone());
    let Some(mut io) = Io::new(h) else {
        unsafe {
            let _ = CloseHandle(h);
        }
        return;
    };
    let mut buf = vec![0u8; report_len];
    let mut counter = 0u8;
    let mut set_up = false;
    let mut enhanced = false;
    let mut lizard_at: Option<Instant> = None;

    loop {
        if !win_devices::wants_direct() {
            break;
        }
        if let (Some(kind), true) = (f.kind, writable) {
            // Switch: full reports with motion. Writing needs a moment after
            // connecting, so this goes on once the first report has come.
            if kind.is_switch() && !set_up && source.has_reported() {
                let usb_pro = kind == Kind::SwitchPro && report_len == 64;
                for r in pad_hid::switch_setup(usb_pro, &mut counter) {
                    write(h, &r);
                    std::thread::sleep(Duration::from_millis(30));
                }
                set_up = true;
            }
            // PlayStation on Bluetooth: full reports only when motion or the
            // touchpad is wanted (it changes what other programs see).
            if !enhanced && win_devices::wants_motion() {
                if let Some(id) = pad_hid::ps_enhanced_feature(kind) {
                    let mut feat = [0u8; 64];
                    feat[0] = id;
                    unsafe {
                        let _ = HidD_GetFeature(h, feat.as_mut_ptr() as *mut _, feat.len() as u32);
                    }
                }
                enhanced = true;
            }
            if kind.is_steam()
                && win_devices::steam_lizard_off()
                && lizard_at.is_none_or(|t| t.elapsed() >= LIZARD_EVERY)
            {
                for r in pad_hid::steam_lizard_off(kind) {
                    unsafe {
                        let _ = HidD_SetFeature(h, r.as_ptr() as *const _, r.len() as u32);
                    }
                }
                lizard_at = Some(Instant::now());
            }
        }
        match io.read(&mut buf, 250) {
            Ok(None) => continue,
            Err(()) => break,
            Ok(Some(n)) => {
                let data = &buf[..n.min(buf.len())];
                let reading = match (&mut parser, &mut generic, &preparsed) {
                    (Some(p), _, _) => {
                        // Steam Deck reports have no report id: Windows puts a 0 in front.
                        let d = if p.kind == Kind::SteamDeck && data.len() == 65 && data[0] == 0 {
                            &data[1..]
                        } else {
                            data
                        };
                        p.parse(d)
                    }
                    (None, Some(g), Some(pre)) => g.parse(pre, data),
                    _ => None,
                };
                if let Some(r) = reading {
                    win_devices::on_reading(&mut source, &r);
                }
            }
        }
    }
    io.cancel();
    win_devices::on_gone(&mut source);
    crate::log!("controller: stopped reading {name}");
    drop(io);
    drop(preparsed);
    unsafe {
        let _ = CloseHandle(h);
    }
}

/// An unknown controller read through its report descriptor.
struct Generic {
    /// (usage, logical min, logical max) of the axes present.
    axes: Vec<(u16, i32, i32)>,
}

const X: u16 = 0x30;
const Y: u16 = 0x31;
const Z: u16 = 0x32;
const RX: u16 = 0x33;
const RY: u16 = 0x34;
const RZ: u16 = 0x35;
const HAT: u16 = 0x39;

impl Generic {
    fn new(p: &Preparsed) -> Option<Self> {
        let caps = p.caps()?;
        let mut n = caps.NumberInputValueCaps;
        let mut vc = vec![HIDP_VALUE_CAPS::default(); n as usize];
        if n > 0 && unsafe { HidP_GetValueCaps(HidP_Input, vc.as_mut_ptr(), &mut n, p.0) } != HIDP_STATUS_SUCCESS {
            return None;
        }
        let mut axes = Vec::new();
        for c in vc.iter().take(n as usize).filter(|c| c.UsagePage == 0x01) {
            let usage = if c.IsRange {
                unsafe { c.Anonymous.Range.UsageMin }
            } else {
                unsafe { c.Anonymous.NotRange.Usage }
            };
            // Logical ranges are signed in the descriptor; small unsigned
            // ranges (0..255) can come back as -1 for the maximum.
            let (min, mut max) = (c.LogicalMin, c.LogicalMax);
            if max < min && c.BitSize < 32 {
                max = ((1i64 << c.BitSize) - 1) as i32;
            }
            axes.push((usage, min, max));
        }
        Some(Generic { axes })
    }

    fn has(&self, usage: u16) -> bool {
        self.axes.iter().any(|a| a.0 == usage)
    }

    fn parse(&mut self, p: &Preparsed, data: &[u8]) -> Option<PadReading> {
        let mut report = data.to_vec();
        let mut r = PadReading::default();
        // Buttons (usage page 9): numbered 1..32.
        let mut usages = [0u16; 64];
        let mut len = usages.len() as u32;
        let st = unsafe { HidP_GetUsages(HidP_Input, 0x09, None, usages.as_mut_ptr(), &mut len, p.0, &mut report) };
        if st == HIDP_STATUS_SUCCESS {
            for &u in &usages[..len as usize] {
                if (1..=32).contains(&u) {
                    r.numbered |= 1 << (u - 1);
                }
            }
        }
        let value = |usage: u16| -> Option<f32> {
            let &(_, min, max) = self.axes.iter().find(|a| a.0 == usage)?;
            let mut v = 0u32;
            let st = unsafe { HidP_GetUsageValue(HidP_Input, 0x01, None, usage, &mut v, p.0, data) };
            if st != HIDP_STATUS_SUCCESS || max <= min {
                return None;
            }
            // 0..1 across the logical range.
            Some(((v as i64 - min as i64) as f32 / (max - min) as f32).clamp(0.0, 1.0))
        };
        let stick = |v: Option<f32>, invert: bool| -> i16 {
            let a = v.map_or(0.0, |v| v * 2.0 - 1.0);
            ((if invert { -a } else { a }) * 32767.0) as i16
        };
        r.lx = stick(value(X), false);
        r.ly = stick(value(Y), true);
        // Right stick: Z / Rz on most DirectInput pads (Rx / Ry then the
        // triggers), else Rx / Ry.
        if self.has(Z) && self.has(RZ) {
            r.rx = stick(value(Z), false);
            r.ry = stick(value(RZ), true);
            r.lt = value(RX).map_or(0, |v| (v * 255.0) as u8);
            r.rt = value(RY).map_or(0, |v| (v * 255.0) as u8);
        } else {
            r.rx = stick(value(RX), false);
            r.ry = stick(value(RY), true);
            r.lt = value(Z).map_or(0, |v| (v * 255.0) as u8);
        }
        if let Some(&(_, min, _)) = self.axes.iter().find(|a| a.0 == HAT) {
            let mut v = 0u32;
            let st = unsafe { HidP_GetUsageValue(HidP_Input, 0x01, None, HAT, &mut v, p.0, data) };
            if st == HIDP_STATUS_SUCCESS {
                let h = v as i64 - min as i64;
                r.buttons |= hat_bits(if (0..8).contains(&h) { h as u8 } else { 8 });
            }
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists the controllers this PC has as the app would read them.
    /// `cargo test list_controllers -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn list_controllers() {
        let found = enumerate();
        for f in &found {
            let name = open(&f.path, 0, false).and_then(|h| {
                let n = product_name(h);
                unsafe {
                    let _ = CloseHandle(h);
                }
                n
            });
            println!(
                "{:04x}:{:04x} {:?} {:?} → {}",
                f.vendor, f.product, f.kind, name, f.path
            );
        }
        println!("{} controller interface(s) found", found.len());
    }
}
