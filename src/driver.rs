//! Installing the ViGEmBus driver the virtual controller needs (Windows).
//!
//! Only when the user asks (*Install controller driver*), never silently:
//! 1. download the official installer from the ViGEmBus GitHub release;
//! 2. check its size and SHA-256 against the values built into the app, so
//!    only that exact, signed file is ever run;
//! 3. run it through Windows' admin prompt; the installer shows its own
//!    window, where the user clicks through it;
//! 4. check the driver is there, and tell the engine to look for it.
//!
//! ViGEmBus is no longer developed (its last release, 1.22.0, is from
//! 2023), but it is still what other controller tools use and works on
//! Windows 10 and 11.

/// What gets installed, as shown in the app.
pub const NAME: &str = "ViGEmBus 1.22.0";
/// The driver's project page.
pub const PROJECT_URL: &str = "https://github.com/nefarius/ViGEmBus";

#[cfg_attr(not(windows), allow(dead_code))]
const URL: &str = "https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/ViGEmBus_1.22.0_x64_x86_arm64.exe";
#[cfg_attr(not(windows), allow(dead_code))]
const FILE_NAME: &str = "ViGEmBus_1.22.0_x64_x86_arm64.exe";
/// The installer's size and SHA-256 (signed by Nefarius Software Solutions
/// e.U.), checked when this version was added to the app.
#[cfg_attr(not(windows), allow(dead_code))]
const SIZE: usize = 6_278_576;
#[cfg_attr(not(windows), allow(dead_code))]
const SHA256: &str = "89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a";

/// Whether a download is exactly the installer the app knows.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn verify(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != SIZE {
        return Err(format!(
            "the download is {} bytes instead of {SIZE}; nothing was installed",
            bytes.len()
        ));
    }
    if crate::update::sha256_hex(bytes) != SHA256 {
        return Err("the download isn't the expected installer (checksum differs); nothing was installed".into());
    }
    Ok(())
}

/// Download, check and run the installer, then wait for the driver.
/// Blocks: call from a background thread. `progress(done, total)` is called
/// while downloading.
#[cfg(windows)]
pub fn install(progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let bytes = crate::update::http::get(URL, 16 << 20, progress)?;
    verify(&bytes)?;
    let dir = std::env::temp_dir().join("mouse-kb-remap-driver");
    let file = dir.join(FILE_NAME);
    std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::write(&file, &bytes))
        .map_err(|e| format!("can't save the installer to {}: {e}", dir.display()))?;
    crate::log!("driver: downloaded and checked {NAME}, starting its installer");

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
    crate::log!("driver: installer finished with code {code}");
    match code {
        // 3010: done, a restart is recommended.
        0 | 3010 => {}
        1602 => return Err("the installer was cancelled; nothing was installed".into()),
        c => return Err(format!("the installer stopped with code {c}")),
    }
    // The driver starts a moment after the installer ends.
    for _ in 0..20 {
        if crate::engine::vpad_driver_present() {
            crate::engine::vpad_retry();
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err("the installer finished, but the driver isn't running yet: restart Windows".into())
}

#[cfg(not(windows))]
pub fn install(_progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    Err("the virtual controller is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_known_installer_passes() {
        assert!(verify(b"MZ").is_err());
        let mut fake = vec![0u8; SIZE];
        fake[..2].copy_from_slice(b"MZ");
        assert!(verify(&fake).unwrap_err().contains("checksum"));
    }

    /// Downloads the installer the way the app does and checks it (doesn't
    /// run it): `cargo test download_matches -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn download_matches() {
        let bytes = crate::update::http::get(URL, 16 << 20, &|_, _| {}).unwrap();
        verify(&bytes).unwrap();
    }

    /// Checks a downloaded installer: set `VIGEMBUS_INSTALLER` to its path
    /// and run `cargo test known_installer_matches -- --ignored`.
    #[test]
    #[ignore]
    fn known_installer_matches() {
        let path = std::env::var("VIGEMBUS_INSTALLER").expect("set VIGEMBUS_INSTALLER");
        verify(&std::fs::read(path).unwrap()).unwrap();
    }
}
