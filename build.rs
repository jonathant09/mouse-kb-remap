fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile Slint UI");

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
