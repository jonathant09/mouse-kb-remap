//! OS integration outside the input engine: single instance, start with
//! Windows, opening files and URLs.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Autostart {
    Off,
    /// Start at sign-in as a normal app (registry Run key).
    On,
    /// Start at sign-in with admin rights (Task Scheduler, highest privileges).
    Elevated,
}

impl Autostart {
    pub fn index(self) -> i32 {
        match self {
            Autostart::Off => 0,
            Autostart::On => 1,
            Autostart::Elevated => 2,
        }
    }

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Autostart::On,
            2 => Autostart::Elevated,
            _ => Autostart::Off,
        }
    }
}

/// Command-line flag used by autostart: start hidden in the tray.
pub const TRAY_FLAG: &str = "--tray";

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::{Autostart, TRAY_FLAG};
    use std::os::windows::process::CommandExt;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, GetExitCodeProcess, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK, SW_HIDE, SW_SHOWNORMAL};

    const MUTEX_NAME: PCWSTR = w!("Local\\mouse-kb-remap.instance");
    const SHOW_EVENT_NAME: PCWSTR = w!("Local\\mouse-kb-remap.show");
    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const APP_NAME: PCWSTR = w!("mouse-kb-remap");
    const TASK_NAME: &str = "mouse-kb-remap";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Make sure only one copy runs (two would install two sets of hooks).
    /// Returns false if another copy is running; it is asked to show its
    /// window. `on_show` is called (on a background thread) whenever a later
    /// launch asks this copy to show itself.
    pub fn single_instance(on_show: impl Fn() + Send + 'static) -> bool {
        unsafe {
            let mutex = CreateMutexW(None, false, MUTEX_NAME);
            let already = match &mutex {
                Ok(_) => GetLastError() == ERROR_ALREADY_EXISTS,
                Err(_) => true, // access denied: created by an elevated copy
            };
            if already {
                let signalled = OpenEventW(EVENT_MODIFY_STATE, false, SHOW_EVENT_NAME)
                    .and_then(|e| {
                        let r = SetEvent(e);
                        let _ = CloseHandle(e);
                        r
                    })
                    .is_ok();
                if !signalled {
                    message_box(
                        "mouse-kb-remap is already running as administrator.\n\n\
                         Open it from its tray icon, or quit it there first.",
                    );
                }
                return false;
            }
            // Keep the mutex for the life of the process.
            std::mem::forget(mutex);
            if let Ok(event) = CreateEventW(None, false, false, SHOW_EVENT_NAME) {
                let event = event.0 as usize;
                std::thread::spawn(move || loop {
                    let h = HANDLE(event as *mut _);
                    if WaitForSingleObject(h, INFINITE).0 != 0 {
                        break;
                    }
                    on_show();
                });
            }
            true
        }
    }

    pub fn message_box(text: &str) {
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                w!("mouse-kb-remap"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }

    /// Open a folder, file or URL with its default program.
    pub fn open(target: &str) {
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(target),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    pub fn autostart_supported() -> bool {
        true
    }

    fn exe_path() -> Result<String, String> {
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .map_err(|e| format!("can't find the app's own path: {e}"))
    }

    fn run_value_exists() -> bool {
        let mut size = 0u32;
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                APP_NAME,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            ) == ERROR_SUCCESS
        }
    }

    fn set_run_value(on: bool) -> Result<(), String> {
        unsafe {
            if on {
                let value = format!("\"{}\" {TRAY_FLAG}", exe_path()?);
                let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
                let err = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN_KEY,
                    APP_NAME,
                    REG_SZ.0,
                    Some(wide.as_ptr() as *const _),
                    (wide.len() * 2) as u32,
                );
                if err != ERROR_SUCCESS {
                    return Err(format!("couldn't write the startup entry (error {})", err.0));
                }
            } else if run_value_exists() {
                let err = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, APP_NAME);
                if err != ERROR_SUCCESS {
                    return Err(format!("couldn't remove the startup entry (error {})", err.0));
                }
            }
        }
        Ok(())
    }

    fn task_exists() -> bool {
        std::process::Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Run `schtasks` with admin rights: directly if we have them, otherwise
    /// through one UAC prompt. Returns the exit code.
    fn schtasks_elevated(args: &str) -> Result<u32, String> {
        if crate::system::is_elevated() {
            let status = std::process::Command::new("schtasks")
                .raw_arg(args)
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            return Ok(status.code().unwrap_or(1) as u32);
        }
        unsafe {
            let params = HSTRING::from(args);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
                lpVerb: w!("runas"),
                lpFile: w!("schtasks.exe"),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_HIDE.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut info).map_err(|_| "the admin prompt was cancelled".to_string())?;
            if info.hProcess.is_invalid() {
                return Err("couldn't start schtasks".into());
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            Ok(code)
        }
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn task_xml(exe: &str, user: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts mouse-kb-remap with admin rights at sign-in, so it can remap games that run as administrator.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{TRAY_FLAG}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
            exe = xml_escape(exe),
            user = xml_escape(user),
        )
    }

    #[cfg(test)]
    pub fn task_xml_for_test(exe: &str, user: &str) -> String {
        task_xml(exe, user)
    }

    fn create_task() -> Result<(), String> {
        let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
            (Ok(d), Ok(u)) => format!("{d}\\{u}"),
            (_, Ok(u)) => u,
            _ => return Err("can't tell which user is signed in".into()),
        };
        let xml = task_xml(&exe_path()?, &user);
        // schtasks wants UTF-16 with a byte-order mark.
        let mut bytes = vec![0xFF, 0xFE];
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let file = std::env::temp_dir().join("mouse-kb-remap-task.xml");
        std::fs::write(&file, bytes).map_err(|e| format!("couldn't write the task file: {e}"))?;
        let code = schtasks_elevated(&format!("/Create /TN \"{TASK_NAME}\" /XML \"{}\" /F", file.display()));
        let _ = std::fs::remove_file(&file);
        match code? {
            0 => Ok(()),
            c => Err(format!("schtasks couldn't create the task (exit code {c})")),
        }
    }

    fn delete_task() -> Result<(), String> {
        if !task_exists() {
            return Ok(());
        }
        match schtasks_elevated(&format!("/Delete /TN \"{TASK_NAME}\" /F"))? {
            0 => Ok(()),
            c => Err(format!("schtasks couldn't remove the task (exit code {c})")),
        }
    }

    pub fn autostart_status() -> Autostart {
        if task_exists() {
            Autostart::Elevated
        } else if run_value_exists() {
            Autostart::On
        } else {
            Autostart::Off
        }
    }

    pub fn set_autostart(mode: Autostart) -> Result<(), String> {
        match mode {
            Autostart::Off => {
                set_run_value(false)?;
                delete_task()
            }
            Autostart::On => {
                delete_task()?;
                set_run_value(true)
            }
            Autostart::Elevated => {
                create_task()?;
                set_run_value(false)
            }
        }
    }

    /// The primary monitor's work area (screen minus taskbar), physical pixels:
    /// (x, y, width, height).
    pub fn work_area() -> Option<(i32, i32, i32, i32)> {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        };
        let mut r = RECT::default();
        unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut r as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .ok()?;
        }
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }

    pub fn is_elevated() -> bool {
        use std::ffi::c_void;
        use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn task_xml_escapes_and_mentions_everything() {
        let xml = super::win::task_xml_for_test(r"C:\Games & Tools\mkr.exe", r"PC\me");
        assert!(xml.contains(r"<Command>C:\Games &amp; Tools\mkr.exe</Command>"));
        assert!(xml.contains(r"<UserId>PC\me</UserId>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<Arguments>--tray</Arguments>"));
        assert!(xml.contains("<Priority>4</Priority>"));
        // Write it out for a manual schtasks check when asked.
        if let Ok(path) = std::env::var("MKR_TASK_XML_OUT") {
            let mut bytes = vec![0xFF, 0xFE];
            for u in xml.replace("HighestAvailable", "LeastPrivilege").encode_utf16() {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            std::fs::write(path, bytes).unwrap();
        }
    }
}

#[cfg(not(windows))]
pub use other::*;

#[cfg(not(windows))]
mod other {
    use super::Autostart;

    pub fn single_instance(_on_show: impl Fn() + Send + 'static) -> bool {
        true
    }

    pub fn open(target: &str) {
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(cmd).arg(target).spawn();
    }

    /// XDG autostart entry (Linux desktops).
    fn desktop_file() -> Option<std::path::PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().join("autostart").join("mouse-kb-remap.desktop"))
    }

    pub fn autostart_supported() -> bool {
        cfg!(target_os = "linux")
    }

    pub fn autostart_status() -> Autostart {
        if cfg!(target_os = "linux") && desktop_file().is_some_and(|f| f.exists()) {
            Autostart::On
        } else {
            Autostart::Off
        }
    }

    pub fn set_autostart(mode: Autostart) -> Result<(), String> {
        if !cfg!(target_os = "linux") {
            return Err("starting with the system isn't supported on this OS yet".into());
        }
        let file = desktop_file().ok_or("can't find the config folder")?;
        match mode {
            Autostart::Off => {
                if file.exists() {
                    std::fs::remove_file(&file).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Autostart::On => {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let entry = format!(
                    "[Desktop Entry]
Type=Application
Name=mouse-kb-remap
Exec=\"{}\" {}
X-GNOME-Autostart-enabled=true
",
                    exe.display(),
                    super::TRAY_FLAG
                );
                if let Some(dir) = file.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                std::fs::write(&file, entry).map_err(|e| e.to_string())
            }
            Autostart::Elevated => Err("not needed on Linux: use On (access comes from the input group)".into()),
        }
    }

    pub fn is_elevated() -> bool {
        false
    }

    pub fn work_area() -> Option<(i32, i32, i32, i32)> {
        None
    }
}
