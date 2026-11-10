//! Tiny log: release builds have no console, so messages also go to
//! `mouse-kb-remap.log` next to `config.json`. The file is started fresh on
//! each launch.

use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init() {
    let Some(dir) = crate::config::Config::path().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(f) = std::fs::File::create(dir.join("mouse-kb-remap.log")) {
        if let Ok(mut slot) = FILE.lock() {
            *slot = Some(f);
        }
    }
}

pub fn write(msg: std::fmt::Arguments) {
    eprintln!("{msg}");
    if let Ok(mut slot) = FILE.lock() {
        if let Some(f) = slot.as_mut() {
            let _ = writeln!(f, "{msg}");
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(format_args!($($t)*)) };
}
