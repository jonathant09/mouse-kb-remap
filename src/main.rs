// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod keys;
mod log;
mod system;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("mouse-kb-remap {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let start_hidden = args.iter().any(|a| a == system::TRAY_FLAG);

    // A second copy would install a second set of hooks: show the first one instead.
    if !system::single_instance(app::show_window_from_any_thread) {
        return Ok(());
    }
    log::init();
    log!("mouse-kb-remap {} starting", env!("CARGO_PKG_VERSION"));

    app::run(start_hidden)
}
