//! Linux: which window is in front, on Wayland compositors that say so.
//!
//! Wayland has no common way for a program to see other programs' windows,
//! so this asks the compositor over its own IPC:
//!
//! - **sway** (`$SWAYSOCK`, i3's IPC): window events, and the tree once at
//!   start.
//! - **Hyprland** (`$HYPRLAND_INSTANCE_SIGNATURE`): the event socket says
//!   when the active window, its title or fullscreen changes; the request
//!   socket's `j/activewindow` says which process it is.
//! - **niri** (`$NIRI_SOCKET`): its JSON event stream.
//!
//! Elsewhere (GNOME, KDE Plasma, X11 sessions) `linux.rs` falls back to X11,
//! which on a Wayland desktop only sees programs running through XWayland.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// The window in front.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Focused {
    pub pid: u32,
    pub title: String,
    pub fullscreen: bool,
}

/// Which compositor IPC this session offers, if any.
pub fn detect() -> Option<&'static str> {
    if std::env::var_os("SWAYSOCK").is_some() {
        Some("sway")
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Some("Hyprland")
    } else if std::env::var_os("NIRI_SOCKET").is_some() {
        Some("niri")
    } else {
        None
    }
}

/// Follow the focused window, calling `on` whenever it (or its title or
/// fullscreen state) changes. Returns only on error.
pub fn watch(on: &mut dyn FnMut(Focused)) -> Result<(), String> {
    match detect() {
        Some("sway") => sway(on),
        Some("Hyprland") => hyprland(on),
        Some("niri") => niri(on),
        _ => Err("no supported Wayland compositor".into()),
    }
}

// ---------------------------------------------------------------------------
// sway (i3 IPC)
// ---------------------------------------------------------------------------

const I3_MAGIC: &[u8] = b"i3-ipc";
const I3_SUBSCRIBE: u32 = 2;
const I3_GET_TREE: u32 = 4;
const I3_EVENT_WINDOW: u32 = 0x8000_0003;

pub fn i3_frame(kind: u32, payload: &str) -> Vec<u8> {
    let mut f = I3_MAGIC.to_vec();
    f.extend((payload.len() as u32).to_le_bytes());
    f.extend(kind.to_le_bytes());
    f.extend(payload.as_bytes());
    f
}

fn i3_read(s: &mut impl Read) -> Result<(u32, Value), String> {
    let mut head = [0u8; 14];
    s.read_exact(&mut head).map_err(|e| format!("sway IPC: {e}"))?;
    if &head[..6] != I3_MAGIC {
        return Err("sway IPC: bad reply".into());
    }
    let len = u32::from_le_bytes(head[6..10].try_into().unwrap()) as usize;
    let kind = u32::from_le_bytes(head[10..14].try_into().unwrap());
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).map_err(|e| format!("sway IPC: {e}"))?;
    let v = serde_json::from_slice(&body).map_err(|e| format!("sway IPC: {e}"))?;
    Ok((kind, v))
}

/// A sway container as a `Focused`.
fn sway_node(c: &Value) -> Option<Focused> {
    Some(Focused {
        pid: c.get("pid")?.as_u64()? as u32,
        title: c.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        fullscreen: c.get("fullscreen_mode").and_then(Value::as_u64).unwrap_or(0) != 0,
    })
}

/// The focused window in sway's tree.
pub fn sway_focused(tree: &Value) -> Option<Focused> {
    if tree.get("focused").and_then(Value::as_bool) == Some(true) && tree.get("pid").is_some() {
        return sway_node(tree);
    }
    ["nodes", "floating_nodes"]
        .iter()
        .filter_map(|k| tree.get(*k).and_then(Value::as_array))
        .flatten()
        .find_map(sway_focused)
}

/// What a sway window event means for the focused window.
pub fn sway_event(ev: &Value) -> Option<Focused> {
    let change = ev.get("change")?.as_str()?;
    let c = ev.get("container")?;
    let focused = c.get("focused").and_then(Value::as_bool) == Some(true);
    match change {
        "focus" => sway_node(c),
        "title" | "fullscreen_mode" if focused => sway_node(c),
        _ => None,
    }
}

fn sway(on: &mut dyn FnMut(Focused)) -> Result<(), String> {
    let path = std::env::var_os("SWAYSOCK").ok_or("no SWAYSOCK")?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("sway IPC: {e}"))?;
    s.write_all(&i3_frame(I3_GET_TREE, "")).map_err(|e| e.to_string())?;
    let (_, tree) = i3_read(&mut s)?;
    if let Some(f) = sway_focused(&tree) {
        on(f);
    }
    s.write_all(&i3_frame(I3_SUBSCRIBE, "[\"window\"]"))
        .map_err(|e| e.to_string())?;
    loop {
        let (kind, v) = i3_read(&mut s)?;
        if kind == I3_EVENT_WINDOW {
            if let Some(f) = sway_event(&v) {
                on(f);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hyprland
// ---------------------------------------------------------------------------

fn hypr_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(|r| PathBuf::from(r).join("hypr").join(&sig));
    // Older versions kept the sockets in /tmp.
    runtime
        .filter(|d| d.join(".socket2.sock").exists())
        .or_else(|| Some(PathBuf::from("/tmp/hypr").join(&sig)))
}

/// `j/activewindow` → the focused window.
pub fn hypr_active(v: &Value) -> Option<Focused> {
    let pid = v.get("pid")?.as_i64().filter(|p| *p > 0)? as u32;
    let fullscreen = match v.get("fullscreen") {
        Some(Value::Bool(b)) => *b,
        // Newer versions: 0 none, 1 maximized, 2 fullscreen.
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) >= 2,
        _ => false,
    };
    Some(Focused {
        pid,
        title: v.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        fullscreen,
    })
}

/// Whether an event line can change the focused window, its title or
/// fullscreen state.
pub fn hypr_relevant(line: &str) -> bool {
    let name = line.split(">>").next().unwrap_or("");
    matches!(
        name,
        "activewindow" | "activewindowv2" | "windowtitle" | "windowtitlev2" | "fullscreen" | "closewindow"
    )
}

fn hypr_query(dir: &std::path::Path) -> Option<Focused> {
    let mut s = UnixStream::connect(dir.join(".socket.sock")).ok()?;
    s.write_all(b"j/activewindow").ok()?;
    let mut text = String::new();
    s.read_to_string(&mut text).ok()?;
    hypr_active(&serde_json::from_str(&text).ok()?)
}

fn hyprland(on: &mut dyn FnMut(Focused)) -> Result<(), String> {
    let dir = hypr_dir().ok_or("no Hyprland instance")?;
    let events = UnixStream::connect(dir.join(".socket2.sock")).map_err(|e| format!("Hyprland IPC: {e}"))?;
    if let Some(f) = hypr_query(&dir) {
        on(f);
    }
    for line in BufReader::new(events).lines() {
        let line = line.map_err(|e| format!("Hyprland IPC: {e}"))?;
        if hypr_relevant(&line) {
            if let Some(f) = hypr_query(&dir) {
                on(f);
            }
        }
    }
    Err("Hyprland closed its event socket".into())
}

// ---------------------------------------------------------------------------
// niri
// ---------------------------------------------------------------------------

/// Windows niri has told us about, and which is focused.
#[derive(Default)]
pub struct NiriState {
    windows: std::collections::HashMap<u64, Value>,
    focused: Option<u64>,
}

impl NiriState {
    /// One event-stream line; returns the focused window if it may have
    /// changed.
    pub fn event(&mut self, ev: &Value) -> Option<Focused> {
        if let Some(w) = ev.pointer("/WindowsChanged/windows").and_then(Value::as_array) {
            self.windows.clear();
            self.focused = None;
            for w in w {
                if let Some(id) = w.get("id").and_then(Value::as_u64) {
                    if w.get("is_focused").and_then(Value::as_bool) == Some(true) {
                        self.focused = Some(id);
                    }
                    self.windows.insert(id, w.clone());
                }
            }
        } else if let Some(w) = ev.pointer("/WindowOpenedOrChanged/window") {
            let id = w.get("id").and_then(Value::as_u64)?;
            if w.get("is_focused").and_then(Value::as_bool) == Some(true) {
                self.focused = Some(id);
            }
            self.windows.insert(id, w.clone());
        } else if let Some(f) = ev.get("WindowFocusChanged") {
            self.focused = f.get("id").and_then(Value::as_u64);
        } else {
            let id = ev.pointer("/WindowClosed/id").and_then(Value::as_u64)?;
            self.windows.remove(&id);
        }
        let w = self.windows.get(&self.focused?)?;
        Some(Focused {
            pid: w.get("pid").and_then(Value::as_u64)? as u32,
            title: w.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
            fullscreen: false,
        })
    }
}

fn niri(on: &mut dyn FnMut(Focused)) -> Result<(), String> {
    let path = std::env::var_os("NIRI_SOCKET").ok_or("no NIRI_SOCKET")?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("niri IPC: {e}"))?;
    s.write_all(b"\"EventStream\"\n").map_err(|e| e.to_string())?;
    let mut state = NiriState::default();
    for line in BufReader::new(s).lines() {
        let line = line.map_err(|e| format!("niri IPC: {e}"))?;
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(f) = state.event(&v) {
            on(f);
        }
    }
    Err("niri closed its event stream".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn i3_frames() {
        let f = i3_frame(I3_SUBSCRIBE, "[\"window\"]");
        assert_eq!(&f[..6], b"i3-ipc");
        assert_eq!(u32::from_le_bytes(f[6..10].try_into().unwrap()), 10);
        assert_eq!(u32::from_le_bytes(f[10..14].try_into().unwrap()), 2);
        let mut reply = i3_frame(I3_EVENT_WINDOW, "{\"change\":\"focus\"}");
        let (kind, v) = i3_read(&mut reply.as_slice()).unwrap();
        assert_eq!(kind, I3_EVENT_WINDOW);
        assert_eq!(v["change"], "focus");
        reply.truncate(10);
        assert!(i3_read(&mut reply.as_slice()).is_err());
    }

    #[test]
    fn sway_tree_and_events() {
        let tree = json!({"nodes": [{"nodes": [
            {"pid": 10, "name": "a", "focused": false},
            {"nodes": [], "floating_nodes": [{"pid": 42, "name": "Game", "focused": true, "fullscreen_mode": 1}]}
        ]}]});
        assert_eq!(
            sway_focused(&tree),
            Some(Focused {
                pid: 42,
                title: "Game".into(),
                fullscreen: true
            })
        );
        let ev = json!({"change": "focus", "container": {"pid": 7, "name": "foot", "focused": true}});
        assert_eq!(sway_event(&ev).map(|f| f.pid), Some(7));
        // A title change of a window in the background doesn't count.
        let ev = json!({"change": "title", "container": {"pid": 8, "name": "x", "focused": false}});
        assert_eq!(sway_event(&ev), None);
        let ev = json!({"change": "new", "container": {"pid": 9, "focused": true}});
        assert_eq!(sway_event(&ev), None);
    }

    #[test]
    fn hyprland_replies() {
        let v = json!({"pid": 1234, "title": "Editor", "fullscreen": 2});
        assert_eq!(
            hypr_active(&v),
            Some(Focused {
                pid: 1234,
                title: "Editor".into(),
                fullscreen: true
            })
        );
        assert_eq!(
            hypr_active(&json!({"pid": 5, "fullscreen": false})).map(|f| f.fullscreen),
            Some(false)
        );
        // No window focused: Hyprland answers with an empty object.
        assert_eq!(hypr_active(&json!({})), None);
        assert!(hypr_relevant("activewindowv2>>55d0a1"));
        assert!(hypr_relevant("windowtitlev2>>55d0a1,New title"));
        assert!(!hypr_relevant("workspace>>2"));
    }

    #[test]
    fn niri_stream() {
        let mut s = NiriState::default();
        let first = s.event(&json!({"WindowsChanged": {"windows": [
            {"id": 1, "pid": 100, "title": "one", "is_focused": false},
            {"id": 2, "pid": 200, "title": "two", "is_focused": true}
        ]}}));
        assert_eq!(first.map(|f| f.pid), Some(200));
        assert_eq!(
            s.event(&json!({"WindowFocusChanged": {"id": 1}})).map(|f| f.pid),
            Some(100)
        );
        let renamed = s.event(
            &json!({"WindowOpenedOrChanged": {"window": {"id": 1, "pid": 100, "title": "uno", "is_focused": true}}}),
        );
        assert_eq!(renamed.map(|f| f.title), Some("uno".into()));
        assert_eq!(s.event(&json!({"WindowFocusChanged": {"id": null}})), None);
        assert_eq!(s.event(&json!({"WorkspacesChanged": {}})), None);
    }
}
