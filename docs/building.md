# Building & releasing

## Toolchain

- **Rust** (stable) from <https://rustup.rs>. On Windows the default `x86_64-pc-windows-msvc`
  toolchain needs the Visual Studio C++ Build Tools; rustup offers to install them.
- Nothing else. Slint compiles the `.slint` files at build time via `build.rs`.

## Commands

```powershell
cargo run                    # debug build, console window shows logs
cargo run --release          # optimized, no console window
cargo build --release        # exe at target\release\mouse-kb-remap.exe
.\start.bat build            # release build, copied into dist\, then launched
cargo clippy                 # lints
cargo fmt                    # formatting (max width 120, see rustfmt.toml)
```

Debug builds show a console window so `eprintln!` output (hook failures, config errors)
is visible. Release builds hide it (`windows_subsystem = "windows"` in `main.rs`).

## Renderer

The default build uses Slint's **software (CPU) renderer**. For a settings window that's
open a few minutes at a time, it avoids creating a GPU/OpenGL context and keeps memory low.
To try the OpenGL renderer:

```powershell
cargo run --release --features gpu
```

## Developing on Linux / macOS

The UI builds and runs everywhere; `src/engine/fallback.rs` stands in for the Windows
engine, and the status bar says *Engine unavailable on this OS (UI preview)*. Linux needs
the usual winit/fontconfig packages (`libfontconfig-dev`, `libxkbcommon-dev`).

## Continuous integration

`.github/workflows/build.yml` runs on every push to `main`, on pull requests and on tags:

1. Builds the release exe on `windows-latest` (`cargo build --release --locked`).
2. Turns compiler errors and warnings into annotations on the commit, so they show up in
   the GitHub UI without opening the log.
3. Uploads `dist/` (exe, `start.bat`, README, LICENSE) as a workflow artifact.
4. On `v*` tags, publishes a GitHub Release with `mouse-kb-remap.exe` and
   `mouse-kb-remap-windows-x64.zip`.

## Cutting a release

1. Bump `version` in `Cargo.toml` (and the `v0.x` label in `ui/app.slint`).
2. Commit, then tag and push:

   ```powershell
   git tag v0.2.0
   git push origin main --tags
   ```

3. The workflow attaches the exe and zip to the new release. Edit the generated release
   notes if needed.

## `dist/`

`start.bat build` (and the CI package step) puts the exe in `dist\`. The folder is in
`.gitignore`: binaries in git history bloat every clone. Distribute builds through GitHub
Releases instead.
