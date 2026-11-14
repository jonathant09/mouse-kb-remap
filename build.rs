fn main() {
    // Debug builds (and so `cargo test`) keep element names in the generated
    // code: the layout test in src/ui_fit_tests.rs finds elements by them.
    // Release builds leave them out.
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile Slint UI");

    // Icon and version info for the Windows exe (Explorer, taskbar, Task Manager).
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "mouse-kb-remap")
            .set("FileDescription", "mouse-kb-remap: mouse & keyboard remapper")
            .set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            // A missing resource compiler shouldn't block development builds.
            println!("cargo:warning=could not embed the exe icon: {e}");
        }
    }
}
