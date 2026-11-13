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
.\start.bat build            # release build, then launched
.\start.bat install          # build + install/update Documents\mouse-kb-remap (see below)
cargo run -- --quit          # ask the running copy to quit
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
| `MOUSE_KB_REMAP_CONFIG` | Use this config file instead of the default (portable installs, testing). A copy with its own config file runs as a separate instance. |
| `MOUSE_KB_REMAP_ACCEPT_INJECTED=1` | Also remap synthetic input from other programs. For end-to-end tests that drive the app with `SendInput`; leave it off otherwise. |
| `MOUSE_KB_REMAP_UPDATE_URL` | Where the update check reads the "latest release" JSON (GitHub's format) instead of the GitHub API, e.g. `http://127.0.0.1:8765/latest.json` from a local server, to test updating without publishing a release. |

### Checking the Linux build from Windows

CI builds Linux, but you can type-check and lint it locally after
`rustup target add x86_64-unknown-linux-gnu`. Nothing is linked, so only `pkg-config` (asked
about fontconfig by a build script) has to be satisfied; a stub that succeeds is enough:

```powershell
"@echo off`r`necho %* | findstr /C:`"--modversion`" >nul && echo 2.15.0`r`nexit /b 0" | Set-Content $env:TEMP\pkg-config.bat
$env:PKG_CONFIG = "$env:TEMP\pkg-config.bat"; $env:PKG_CONFIG_ALLOW_CROSS = "1"
cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
```

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
   annotations on the commit; packages the exe and the versioned zip and uploads them as a
   workflow artifact (so any build can be downloaded from its run page).
2. **linux**: tests, clippy and a release build; uploads the binary as an experimental
   artifact (not released).
3. **release** (only on `v*` tags): publishes the GitHub Release, see below. It's the only
   job with write permission.

## Cutting a release

1. Bump `version` in `Cargo.toml` (the UI and `--version` read it from there) and run
   `cargo build` so `Cargo.lock` follows.
2. Add a `## <version>` section at the top of `CHANGELOG.md`: it becomes the release notes.
3. Commit, then tag and push:

   ```powershell
   git tag v0.3.0
   git push origin main v0.3.0
   ```

4. The workflow checks the tag matches `Cargo.toml`, builds and tests on Windows and Linux,
   and publishes the release **mouse-kb-remap 0.3.0** with two downloads:
   `mouse-kb-remap-0.3.0-windows-x64.zip` (a `mouse-kb-remap` folder with the exe and the
   license) and the bare `mouse-kb-remap.exe`. If you wrote the release by hand first, its
   notes are kept and the files are attached to it.

The exe runs on its own: `.cargo/config.toml` links the C runtime in, so users don't need the
Visual C++ Redistributable, and the UI's images are compiled in.

## Installing your own build

`start.bat install` (or `scripts\install.ps1`) builds the release exe and installs it as
`Documents\mouse-kb-remap\mouse-kb-remap.exe`, or updates it in place:

1. a running exe can't be overwritten but can be renamed, so the old one is moved aside to
   `mouse-kb-remap.old.exe` and the new one copied in;
2. the running copy is asked to quit with `mouse-kb-remap.exe --quit` (which also reaches a
   copy running as administrator);
3. it's started again the way it starts at sign-in: through the startup task when that
   points at this exe (as administrator, no prompt), otherwise normally, in the tray;
4. the `.old.exe` is deleted (the app also deletes it when it next starts).

`-NoBuild` skips the build; `-Dir <folder>` installs somewhere else.
