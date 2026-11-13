//! Updates from GitHub releases: check for a newer version, and install it
//! in place (download, verify, swap the exe, restart).
//!
//! Installing:
//! 1. download `mouse-kb-remap.exe` from the release next to the running exe;
//! 2. check its size and SHA-256 against what GitHub reports, that it's a
//!    Windows program, and that it runs and reports the expected version
//!    (`--version`);
//! 3. move the running exe aside to `mouse-kb-remap.old.exe` (a running exe
//!    can be renamed, not overwritten) and put the new one in its place,
//!    moving the old one back if that fails;
//! 4. start the new exe with `--after-update <pid>`: it waits for this copy
//!    to quit, then takes over (with the same admin rights, since a program
//!    started by an elevated one is elevated too) and deletes the `.old.exe`
//!    a minute later. Until then, renaming it back undoes the update.
//!
//! Nothing personal is sent: the requests carry a generic user agent.

use serde::Deserialize;

pub const REPO: &str = "jonathant09/mouse-kb-remap";

/// Override the release API address (tests).
pub const URL_ENV: &str = "MOUSE_KB_REMAP_UPDATE_URL";

/// Flag the updater starts the new version with: `--after-update <pid>`.
pub const AFTER_UPDATE_FLAG: &str = "--after-update";

const ASSET: &str = "mouse-kb-remap.exe";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    /// Release page (notes).
    pub page: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex SHA-256 when GitHub reports it.
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// Parse GitHub's "latest release" answer.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let r: ApiRelease = serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    let asset = r
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET))
        .ok_or("the latest release has no mouse-kb-remap.exe")?;
    Ok(Release {
        version: r.tag_name.trim_start_matches('v').to_string(),
        page: r.html_url,
        url: asset.browser_download_url.clone(),
        size: asset.size,
        sha256: asset
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(|h| h.to_ascii_lowercase()),
    })
}

fn version_parts(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Whether version `a` is newer than `b` ("0.10.0" > "0.9.3").
pub fn is_newer(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (version_parts(a), version_parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn api_url() -> String {
    std::env::var(URL_ENV)
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

/// Whether this build can install updates itself.
pub fn supported() -> bool {
    cfg!(windows)
}

/// The latest release, if it's newer than this version.
pub fn check() -> Result<Option<Release>, String> {
    let body = http::get(&api_url(), 1 << 20, &|_, _| {})?;
    let text = String::from_utf8(body).map_err(|_| "unexpected answer from GitHub".to_string())?;
    let release = parse_release(&text)?;
    Ok(is_newer(&release.version, current_version()).then_some(release))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Check a download against the release. Returns why it's unusable.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn verify(bytes: &[u8], release: &Release) -> Result<(), String> {
    if bytes.len() as u64 != release.size {
        return Err(format!(
            "the download is {} bytes, the release says {}",
            bytes.len(),
            release.size
        ));
    }
    if !bytes.starts_with(b"MZ") {
        return Err("the download isn't a Windows program".into());
    }
    if let Some(expected) = &release.sha256 {
        let got = sha256_hex(bytes);
        if &got != expected {
            return Err("the download's checksum doesn't match the release".into());
        }
    }
    Ok(())
}

/// Download, verify and put the new version in place, then start it.
/// The caller must quit right after `Ok`. `progress(done, total)` is called
/// from this thread while downloading.
#[cfg(windows)]
pub fn install(release: &Release, hidden: bool, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| format!("can't find the app's own path: {e}"))?;
    let dir = exe.parent().ok_or("can't find the app's folder")?.to_path_buf();
    let download = dir.join("mouse-kb-remap.download");
    let new = dir.join("mouse-kb-remap.new.exe");
    let old = exe.with_extension("old.exe");
    let cannot_write = |e: std::io::Error| {
        format!(
            "can't write to {} ({e}). Download the new version from the release page instead.",
            dir.display()
        )
    };

    // Fail early, before downloading, if the folder isn't writable.
    std::fs::write(&download, b"").map_err(cannot_write)?;
    let bytes = match http::get(&release.url, 256 << 20, progress) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&download);
            return Err(e);
        }
    };
    if let Err(e) = verify(&bytes, release) {
        let _ = std::fs::remove_file(&download);
        return Err(e);
    }
    std::fs::write(&download, &bytes).map_err(cannot_write)?;
    let _ = std::fs::remove_file(&new);
    std::fs::rename(&download, &new).map_err(cannot_write)?;

    // It must run and be the version we asked for.
    let reported = std::process::Command::new(&new)
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if reported != format!("mouse-kb-remap {}", release.version) {
        let _ = std::fs::remove_file(&new);
        return Err(format!(
            "the downloaded program didn't start properly (it said {reported:?}); nothing was changed"
        ));
    }

    // Swap: the running exe can be renamed, not overwritten.
    if old.exists() && std::fs::remove_file(&old).is_err() {
        return Err(format!(
            "{} is in use; quit any other copy and try again",
            old.display()
        ));
    }
    std::fs::rename(&exe, &old).map_err(cannot_write)?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(cannot_write(e));
    }

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(AFTER_UPDATE_FLAG).arg(std::process::id().to_string());
    if hidden {
        cmd.arg(crate::system::TRAY_FLAG);
    }
    if let Err(e) = cmd.spawn() {
        // Put the old version back.
        let _ = std::fs::rename(&exe, &new);
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("couldn't start the new version: {e}"));
    }
    crate::log!("update: installed {}, restarting", release.version);
    Ok(())
}

#[cfg(not(windows))]
pub fn install(_release: &Release, _hidden: bool, _progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    Err("updating from the app isn't available on this system; download the new version instead".into())
}

#[cfg(windows)]
mod http {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Networking::WinHttp::{
        WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
        WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE,
        WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };

    /// Closes a WinHTTP handle when dropped.
    struct Handle(*mut core::ffi::c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn open(h: *mut core::ffi::c_void, what: &str) -> Result<Handle, String> {
        if h.is_null() {
            Err(format!(
                "{what} failed: {}",
                windows::core::Error::from_thread().message()
            ))
        } else {
            Ok(Handle(h))
        }
    }

    fn query_number(req: &Handle, info: u32) -> Option<u32> {
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        unsafe {
            WinHttpQueryHeaders(
                req.0,
                info | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut value as *mut u32 as *mut _),
                &mut size,
                std::ptr::null_mut(),
            )
            .ok()?;
        }
        Some(value)
    }

    /// GET a URL (http or https), following redirects. At most `limit` bytes.
    pub fn get(url: &str, limit: usize, progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
        let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(format!("not a web address: {url}"));
        };
        let (host_port, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let path = if path.is_empty() { "/" } else { path };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().map_err(|_| format!("bad port in {url}"))?),
            None => (host_port, if secure { 443 } else { 80 }),
        };
        unsafe {
            let agent = HSTRING::from(format!("mouse-kb-remap/{}", super::current_version()));
            let session = open(
                WinHttpOpen(
                    &agent,
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                ),
                "starting the download",
            )?;
            let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
            let connect = open(WinHttpConnect(session.0, &HSTRING::from(host), port, 0), "connecting")?;
            let flags = if secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            };
            let req = open(
                WinHttpOpenRequest(
                    connect.0,
                    &HSTRING::from("GET"),
                    &HSTRING::from(path),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    flags,
                ),
                "the request",
            )?;
            if host.eq_ignore_ascii_case("api.github.com") {
                let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n".encode_utf16().collect();
                let _ = WinHttpAddRequestHeaders(req.0, &headers, WINHTTP_ADDREQ_FLAG_ADD);
            }
            WinHttpSendRequest(req.0, None, None, 0, 0, 0)
                .map_err(|e| format!("couldn't reach {host}: {}", e.message()))?;
            WinHttpReceiveResponse(req.0, std::ptr::null_mut())
                .map_err(|e| format!("no answer from {host}: {}", e.message()))?;
            let status = query_number(&req, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
            if status != 200 {
                return Err(match status {
                    404 => "no published release found (GitHub only shows releases of public repositories)".to_string(),
                    403 | 429 => "GitHub is limiting requests; try again later".to_string(),
                    s => format!("{host} answered with status {s}"),
                });
            }
            let total = query_number(&req, WINHTTP_QUERY_CONTENT_LENGTH).unwrap_or(0) as u64;
            let mut body = Vec::new();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let mut available = 0u32;
                WinHttpQueryDataAvailable(req.0, &mut available)
                    .map_err(|e| format!("download failed: {}", e.message()))?;
                if available == 0 {
                    break;
                }
                let want = (available as usize).min(buf.len());
                let mut read = 0u32;
                WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, want as u32, &mut read)
                    .map_err(|e| format!("download failed: {}", e.message()))?;
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..read as usize]);
                if body.len() > limit {
                    return Err("the download is larger than expected".into());
                }
                progress(body.len() as u64, total);
            }
            Ok(body)
        }
    }
}

#[cfg(not(windows))]
mod http {
    pub fn get(_url: &str, _limit: usize, _progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
        Err("checking for updates isn't available on this system yet".into())
    }
}

/// `--after-update <pid>`: wait (up to 20 s) for the copy that installed us
/// to quit, so we can take over its single-instance slot.
#[cfg(windows)]
pub fn wait_for_exit(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, 20_000);
            let _ = CloseHandle(h);
        }
    }
}

#[cfg(not(windows))]
pub fn wait_for_exit(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asks GitHub for real (network): `cargo test -- --ignored live_check`.
    #[test]
    #[ignore]
    fn live_check() {
        let result = check();
        println!("{result:?}");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(is_newer("0.3.1", "0.3"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(!is_newer("0.3.0-beta", "0.3.0"));
    }

    #[test]
    fn release_json_is_read() {
        let json = r#"{
            "tag_name": "v0.4.0",
            "html_url": "https://github.com/jonathant09/mouse-kb-remap/releases/tag/v0.4.0",
            "assets": [
                { "name": "mouse-kb-remap-0.4.0-windows-x64.zip", "browser_download_url": "https://x/zip", "size": 10 },
                { "name": "mouse-kb-remap.exe", "browser_download_url": "https://x/exe", "size": 4,
                  "digest": "sha256:ABCD" }
            ]
        }"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.4.0");
        assert_eq!(r.url, "https://x/exe");
        assert_eq!(r.size, 4);
        assert_eq!(r.sha256.as_deref(), Some("abcd"));
        assert!(parse_release(r#"{"tag_name":"v1","html_url":"","assets":[]}"#).is_err());
    }

    #[test]
    fn downloads_are_verified() {
        let bytes = b"MZ-program";
        let mut r = Release {
            version: "1.0.0".into(),
            page: String::new(),
            url: String::new(),
            size: bytes.len() as u64,
            sha256: Some(sha256_hex(bytes)),
        };
        assert!(verify(bytes, &r).is_ok());
        assert!(verify(b"MZ-progra", &r).is_err());
        assert!(verify(b"XX-program", &r).is_err());
        r.sha256 = Some("00".repeat(32));
        assert!(verify(bytes, &r).is_err());
        r.sha256 = None;
        assert!(verify(bytes, &r).is_ok());
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
