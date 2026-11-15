//! Installing the drivers the virtual controller uses (Windows): ViGEmBus
//! (the virtual controller itself) and HidHide (hiding the real controller
//! from games).
//!
//! Only when the user asks (*Install controller driver*, *Install HidHide*),
//! never silently:
//! 1. download the official installer from the project's GitHub release;
//! 2. check its size and SHA-256 against the values built into the app, so
//!    only that exact, signed file is ever run;
//! 3. run it through Windows' admin prompt; the installer shows its own
//!    window, where the user clicks through it;
//! 4. check the driver is there, and tell the engine to look for it.
//!
//! ViGEmBus is no longer developed (its last release, 1.22.0, is from
//! 2023), but it is still what other controller tools use and works on
//! Windows 10 and 11. Both are by Nefarius Software Solutions.

/// A driver the app knows how to install.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Driver {
    /// What gets installed, as shown in the app.
    pub name: &'static str,
    /// The driver's project page.
    pub project_url: &'static str,
    url: &'static str,
    file_name: &'static str,
    /// The installer's size and SHA-256 (signed by Nefarius Software
    /// Solutions e.U.), checked when this version was added to the app.
    size: usize,
    sha256: &'static str,
    /// Whether the driver answers now.
    present: fn() -> bool,
    /// Tell the engine it's there.
    found: fn(),
}

pub const VIGEMBUS: Driver = Driver {
    name: "ViGEmBus 1.22.0",
    project_url: "https://github.com/nefarius/ViGEmBus",
    url: "https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/ViGEmBus_1.22.0_x64_x86_arm64.exe",
    file_name: "ViGEmBus_1.22.0_x64_x86_arm64.exe",
    size: 6_278_576,
    sha256: "89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a",
    present: crate::engine::vpad_driver_present,
    found: crate::engine::vpad_retry,
};

pub const HIDHIDE: Driver = Driver {
    name: "HidHide 1.5.230",
    project_url: "https://github.com/nefarius/HidHide",
    url: "https://github.com/nefarius/HidHide/releases/download/v1.5.230.0/HidHide_1.5.230_x64.exe",
    file_name: "HidHide_1.5.230_x64.exe",
    size: 8_078_016,
    sha256: "f4bbbcb82e6258641b887c74bc81c4c5f66e4aa811808dfc304347687b7605f6",
    present: crate::engine::hidhide_present,
    found: crate::engine::hidhide_retry,
};

impl Driver {
    /// Whether a download is exactly the installer the app knows.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn verify(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != self.size {
            return Err(format!(
                "the download is {} bytes instead of {}; nothing was installed",
                bytes.len(),
                self.size
            ));
        }
        if crate::update::sha256_hex(bytes) != self.sha256 {
            return Err("the download isn't the expected installer (checksum differs); nothing was installed".into());
        }
        Ok(())
    }

    /// Download, check and run the installer, then wait for the driver.
    /// Blocks: call from a background thread. `progress(done, total)` is
    /// called while downloading. `Ok(true)`: Windows has to restart first.
    #[cfg(windows)]
    pub fn install(&self, progress: &dyn Fn(u64, u64)) -> Result<bool, String> {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
        use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let bytes = crate::update::http::get(self.url, 16 << 20, progress)?;
        self.verify(&bytes)?;
        let dir = std::env::temp_dir().join("mouse-kb-remap-driver");
        let file = dir.join(self.file_name);
        std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&file, &bytes))
            .map_err(|e| format!("can't save the installer to {}: {e}", dir.display()))?;
        crate::log!("driver: downloaded and checked {}, starting its installer", self.name);

        let code = unsafe {
            let path = HSTRING::from(file.as_os_str());
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                lpVerb: w!("runas"),
                lpFile: PCWSTR(path.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            let started = ShellExecuteExW(&mut info);
            if started.is_err() || info.hProcess.is_invalid() {
                let _ = std::fs::remove_file(&file);
                return Err("the admin prompt was declined; nothing was installed".into());
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            code
        };
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
        crate::log!("driver: {} installer finished with code {code}", self.name);
        let restart = match code {
            0 => false,
            // 3010: done, a restart is needed to finish.
            3010 => true,
            1602 => return Err("the installer was cancelled; nothing was installed".into()),
            c => return Err(format!("the installer stopped with code {c}")),
        };
        // The driver starts a moment after the installer ends.
        for _ in 0..20 {
            if (self.present)() {
                (self.found)();
                return Ok(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if restart {
            Ok(true)
        } else {
            Err("the installer finished, but the driver isn't running yet: restart Windows".into())
        }
    }

    #[cfg(not(windows))]
    pub fn install(&self, _progress: &dyn Fn(u64, u64)) -> Result<bool, String> {
        Err(format!("{} is only for Windows", self.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_known_installer_passes() {
        for d in [&VIGEMBUS, &HIDHIDE] {
            assert!(d.verify(b"MZ").is_err());
            let mut fake = vec![0u8; d.size];
            fake[..2].copy_from_slice(b"MZ");
            assert!(d.verify(&fake).unwrap_err().contains("checksum"));
        }
    }

    /// Downloads the installers the way the app does and checks them
    /// (doesn't run them): `cargo test download_matches -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn download_matches() {
        for d in [&VIGEMBUS, &HIDHIDE] {
            let bytes = crate::update::http::get(d.url, 16 << 20, &|_, _| {}).unwrap();
            d.verify(&bytes).unwrap();
        }
    }

    /// Installs HidHide the way the *Install HidHide* button does (admin
    /// prompt, then the installer's window):
    /// `cargo test install_hidhide_for_real -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn install_hidhide_for_real() {
        let restart = HIDHIDE.install(&|_, _| {}).unwrap();
        println!(
            "installed; restart needed: {restart}; driver answers: {}",
            (HIDHIDE.present)()
        );
    }

    /// Checks a downloaded installer: set `DRIVER_INSTALLER` to its path and
    /// run `cargo test known_installer_matches -- --ignored`.
    #[test]
    #[ignore]
    fn known_installer_matches() {
        let path = std::env::var("DRIVER_INSTALLER").expect("set DRIVER_INSTALLER");
        let bytes = std::fs::read(path).unwrap();
        assert!(VIGEMBUS.verify(&bytes).is_ok() || HIDHIDE.verify(&bytes).is_ok());
    }
}
