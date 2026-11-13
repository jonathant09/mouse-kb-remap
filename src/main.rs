// Hide the console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod ipc;
mod keys;
mod log;
mod osd;
mod system;
mod update;

slint::include_modules!();

fn print_help() {
    println!("mouse-kb-remap {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage: mouse-kb-remap [option]");
    println!();
    println!("  (none)              open the app (or show the copy that's running)");
    println!("  --tray              start hidden in the tray");
    println!("  --quit              ask the running copy to quit");
    for (flag, _, help) in ipc::FLAGS {
        let flag = if *flag == "--profile" { "--profile <name>" } else { flag };
        println!("  {flag:<19} {help}");
    }
    println!("  --version           print the version");
    println!();
    println!("Exit codes: 0 done, 1 refused (e.g. unknown profile) or bad arguments, 2 the app isn't running.");
}

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = |a: &str| args.iter().any(|x| x == a);
    if cli("--version") || cli("-V") {
        system::attach_console();
        println!("mouse-kb-remap {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if cli("--help") || cli("-h") || cli("/?") {
        system::attach_console();
        print_help();
        return Ok(());
    }
    if cli(system::QUIT_FLAG) {
        // Exit code for scripts: 0 quit (or wasn't running), 2 couldn't.
        let code = match system::request_quit() {
            system::QuitResult::NotRunning | system::QuitResult::Quit => 0,
            system::QuitResult::Failed => 2,
        };
        std::process::exit(code);
    }
    if let Some(command) = ipc::command_from_args(&args) {
        system::attach_console();
        let command = match command {
            Ok(c) => c,
            Err(usage) => {
                eprintln!("{usage}");
                std::process::exit(1);
            }
        };
        let code = match ipc::send(&command) {
            Ok(reply) => {
                println!("{}", reply.trim());
                if reply.starts_with("ok") {
                    0
                } else {
                    1
                }
            }
            Err(ipc::SendError::NotRunning) => {
                eprintln!("mouse-kb-remap isn't running.");
                2
            }
            Err(ipc::SendError::Failed(e)) => {
                eprintln!("{e}");
                2
            }
        };
        std::process::exit(code);
    }
    let start_hidden = cli(system::TRAY_FLAG);

    // Started by the updater: let the old version quit first.
    let after_update = args
        .iter()
        .position(|a| a == update::AFTER_UPDATE_FLAG)
        .and_then(|i| args.get(i + 1))
        .and_then(|pid| pid.parse::<u32>().ok());
    if let Some(pid) = after_update {
        update::wait_for_exit(pid);
    }

    // A second copy would install a second set of hooks: show the first one instead.
    if !system::single_instance(app::show_window_from_any_thread, app::quit_from_any_thread) {
        return Ok(());
    }
    if after_update.is_some() {
        // Keep the previous version around for a minute, so a broken update
        // can still be undone by renaming it back.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(60));
            system::remove_replaced_exe();
        });
    } else {
        system::remove_replaced_exe();
    }
    log::init();
    log!("mouse-kb-remap {} starting", env!("CARGO_PKG_VERSION"));

    app::run(app::Options {
        start_hidden,
        updated: after_update.is_some(),
    })
}
