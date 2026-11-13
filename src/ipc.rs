//! Command-line control of the running copy (`mouse-kb-remap --pause`,
//! `--profile <name>`, …), for scripts, Stream Deck, AutoHotkey and the
//! like.
//!
//! The running copy serves a named pipe that only this user can open (also
//! when the app runs as administrator and the caller doesn't). A command is
//! one line of text; the answer starts with `ok` or `error`.

/// Command-line flags and the pipe command each one sends.
pub const FLAGS: &[(&str, &str, &str)] = &[
    ("--pause", "pause", "pause remapping"),
    ("--resume", "resume", "resume remapping"),
    ("--toggle-pause", "toggle-pause", "pause or resume remapping"),
    ("--profile", "profile", "use the named profile whatever app is focused"),
    (
        "--auto-profile",
        "auto-profile",
        "go back to choosing the profile by the focused app",
    ),
    ("--next-profile", "next-profile", "use the next profile in the list"),
    (
        "--status",
        "status",
        "print whether remapping is on and the profile in use",
    ),
];

/// The pipe command for the given arguments, if they ask for one.
/// `Err` for a flag missing its value.
pub fn command_from_args(args: &[String]) -> Option<Result<String, String>> {
    for (i, a) in args.iter().enumerate() {
        let Some((flag, cmd, _)) = FLAGS.iter().find(|(f, _, _)| f == a) else {
            continue;
        };
        if *flag == "--profile" {
            return Some(match args.get(i + 1) {
                Some(name) if !name.starts_with("--") => Ok(format!("profile {name}")),
                _ => Err("--profile needs a profile name, e.g. --profile \"Genshin Impact\"".into()),
            });
        }
        return Some(Ok(cmd.to_string()));
    }
    None
}

#[cfg(windows)]
pub use win::{send, serve};

#[cfg(not(windows))]
pub fn serve(_handler: impl Fn(String) -> String + Send + 'static) {}

#[cfg(not(windows))]
pub fn send(_command: &str) -> Result<String, SendError> {
    Err(SendError::Failed(
        "command-line control isn't available on this system yet".into(),
    ))
}

#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum SendError {
    NotRunning,
    Failed(String),
}

#[cfg(windows)]
mod win {
    use super::SendError;
    use std::io::{Read, Write};
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, INVALID_HANDLE_VALUE};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_MESSAGE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn pipe_name() -> String {
        format!(r"\\.\pipe\mouse-kb-remap{}", crate::system::instance_suffix())
    }

    /// "S-1-5-21-…" of the user running this process.
    fn user_sid() -> Option<String> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
            let mut buf = vec![0u8; 256];
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as *mut _),
                buf.len() as u32,
                &mut len,
            );
            let _ = CloseHandle(token);
            ok.ok()?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
            let sid = s.to_string().ok();
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
            sid
        }
    }

    /// Serve commands on a background thread; `handler` answers each one.
    pub fn serve(handler: impl Fn(String) -> String + Send + 'static) {
        let Some(sid) = user_sid() else {
            crate::log!("ipc: can't tell which user this is; command-line control is off");
            return;
        };
        std::thread::Builder::new()
            .name("ipc".into())
            .spawn(move || unsafe {
                // Only this user, from a normal or an admin process (the
                // medium integrity label lets a non-admin caller reach an
                // app running as administrator).
                let sddl = HSTRING::from(format!("D:(A;;GRGW;;;{sid})(A;;GA;;;SY)S:(ML;;NW;;;ME)"));
                let mut sd = PSECURITY_DESCRIPTOR::default();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None).is_err()
                {
                    crate::log!("ipc: couldn't build the pipe's security; command-line control is off");
                    return;
                }
                let attrs = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: sd.0,
                    bInheritHandle: false.into(),
                };
                let name = HSTRING::from(pipe_name());
                loop {
                    let pipe = CreateNamedPipeW(
                        &name,
                        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                        PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                        1,
                        4096,
                        4096,
                        0,
                        Some(&attrs),
                    );
                    if pipe == INVALID_HANDLE_VALUE {
                        crate::log!(
                            "ipc: couldn't create the pipe: {}",
                            windows::core::Error::from_thread().message()
                        );
                        break;
                    }
                    // Returns an error when a client connected in between, which is fine.
                    let _ = ConnectNamedPipe(pipe, None);
                    let mut buf = [0u8; 1024];
                    let mut read = 0u32;
                    if ReadFile(pipe, Some(&mut buf), Some(&mut read), None).is_ok() {
                        let command = String::from_utf8_lossy(&buf[..read as usize]).trim().to_string();
                        let reply = handler(command);
                        let _ = WriteFile(pipe, Some(reply.as_bytes()), None, None);
                        let _ = FlushFileBuffers(pipe);
                    }
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = CloseHandle(pipe);
                }
                let _ = LocalFree(Some(HLOCAL(sd.0)));
            })
            .ok();
    }

    /// Send a command to the running copy and return its answer.
    pub fn send(command: &str) -> Result<String, SendError> {
        let path = pipe_name();
        let mut tries = 0;
        let mut pipe = loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(f) => break f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && tries > 0 => return Err(SendError::NotRunning),
                // Busy (another command being answered) or between pipe
                // instances: try again shortly.
                Err(e) if tries < 40 => {
                    let _ = e;
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(SendError::Failed(e.to_string())),
            }
        };
        pipe.write_all(command.as_bytes())
            .map_err(|e| SendError::Failed(e.to_string()))?;
        let mut reply = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => reply.extend_from_slice(&buf[..n]),
                // The server disconnects after answering.
                Err(_) => break,
            }
        }
        if reply.is_empty() {
            return Err(SendError::Failed("no answer from the running copy".into()));
        }
        Ok(String::from_utf8_lossy(&reply).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_become_commands() {
        assert_eq!(command_from_args(&args(&["--pause"])), Some(Ok("pause".into())));
        assert_eq!(
            command_from_args(&args(&["--profile", "Genshin Impact"])),
            Some(Ok("profile Genshin Impact".into()))
        );
        assert!(matches!(command_from_args(&args(&["--profile"])), Some(Err(_))));
        assert!(matches!(
            command_from_args(&args(&["--profile", "--tray"])),
            Some(Err(_))
        ));
        assert_eq!(command_from_args(&args(&["--tray"])), None);
    }
}
