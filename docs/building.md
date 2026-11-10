# Building & releasing

## Toolchain

- **Rust** (stable) from <https://rustup.rs>. On Windows the default `x86_64-pc-windows-msvc`
  toolchain needs the Visual Studio C++ Build Tools; rustup offers to install them. Their
  Windows SDK also provides the resource compiler that embeds the exe icon.
- Nothing else. Slint compiles the `.slint` files at build time via `build.rs`, and the
  images they use are compiled into the exe.

## Commands

```powershell
cargo run                    # debug build, console window shows logs
cargo run --release          # optimized, no console window
cargo run -- --tray          # start hidden in the tray (what autostart does)
cargo build --release        # exe at target\release\mouse-kb-remap.exe
.\start.bat build            # release build, copied into dist\, then launched
cargo test                   # unit tests (config, keys, dispatcher, scheduler, …)
cargo clippy --all-targets   # lints (CI fails on warnings)
cargo fmt                    # formatting (max width 120, see rustfmt.toml)
```

Debug builds show a console window so log output is visible. Release builds hide it
(`windows_subsystem = "windows"` in `main.rs`); everything logged also goes to
`mouse-kb-remap.log` next to `config.json`.

### Environment variables

| Variable | Effect |
| --- | --- |
| `MOUSE_KB_REMAP_CONFIG` | Use this config file instead of the default (portable installs, testing) |
| `MOUSE_KB_REMAP_ACCEPT_INJECTED=1` | Also remap synthetic input from other programs. For end-to-end tests that drive the app with `SendInput`; leave it off otherwise. |

## Renderer

The default build uses Slint's **software (CPU) renderer**. For a settings window that's
open a few minutes at a time, it avoids creating a GPU/OpenGL context and keeps memory low.
To try the OpenGL renderer:

```powershell
cargo run --release --features gpu
```

## Icon

`assets/icon.svg` (and `icon-small.svg` for 16–24 px) are the sources. The PNGs and the
multi-size `icon.ico` are generated and committed; after editing the SVGs:

```powershell
npm install --no-save @resvg/resvg-js@2.6.2
node tools/render-icon.mjs
```

`build.rs` embeds `assets/icon.ico` into the exe (via `winresource`); the window and the
tray use the PNGs.

## Linux

```bash
sudo apt install libfontconfig1-dev libxkbcommon-dev   # Debian/Ubuntu
cargo build --release
```

The engine is experimental; see [Troubleshooting](troubleshooting.md#linux-experimental) for
the `/dev/input` and `/dev/uinput` permissions it needs. On other systems (macOS) the UI
runs with a no-op engine and the status bar says *Engine unavailable on this OS*.

## Continuous integration

`.github/workflows/build.yml` runs on every push to `main`, on pull requests and on tags:

1. **windows**: tests, clippy (warnings are errors), release build, compiler output as
   annotations on the commit, and uploads `dist/` + the zip as a workflow artifact.
2. **linux**: tests, clippy and a release build; uploads the binary as an experimental
   artifact.
3. **release** (only on `v*` tags): publishes a GitHub Release with `mouse-kb-remap.exe`
   and `mouse-kb-remap-windows-x64.zip`. It's the only job with write permission.

## Cutting a release

1. Bump `version` in `Cargo.toml` (the UI and `--version` read it from there).
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
