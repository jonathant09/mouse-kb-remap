<p align="center"><img src="assets/icon-256.png" width="96" alt="mouse-kb-remap icon"></p>

# mouse-kb-remap

A lightweight, open-source, brand-agnostic remapper for mice and keyboards. Think the
per-game profiles, macros and turbo of Logitech G HUB or Razer Synapse, but in one small
app that works with **any** mouse or keyboard.

![Dark theme](docs/screenshot-dark.png)

## What it does

- **Per-app profiles.** A profile activates automatically while its app's window is focused
  (e.g. `GenshinImpact.exe`) and switches back when you alt-tab away. The **Default**
  profile applies everywhere else, and other profiles inherit its bindings.
- **Remap any button or key** to:
  - a **key or combo** (Ctrl + C, Alt + Tab, Win + Shift + S…), with one-click presets
  - a **mouse button** or scroll step, including horizontal scroll (wheel tilt)
  - a **macro**: recorded or hand-built keys, clicks, delays and text, played once, a set
    number of times, while held, or toggled on/off
  - **turbo** / **repeat on hold** with your own timing: press interval, hold time and an
    optional "start repeating after" delay, while held or toggled
  - **text** to type (any language), **launch** a program, file or URL, **media keys**
  - a **shift layer** button (like G-Shift / Hypershift): hold it for a second set of
    bindings on every other button and key
  - a **sensitivity clutch** ("sniper" button): lower pointer speed while held
  - **off**
- **Games-friendly output.** Keys are sent as scan codes, which most games read, and timed
  output (turbo, macros) runs on a precise timer thread (typically within 1 ms).
- **Safe by design.** Remaps never apply while this app's window is focused, and focusing it
  stops any running macro or turbo, so a bad binding can't lock you out.
- **Tray icon, start with Windows** (optionally as administrator, for games that run
  elevated), **import / export** of profiles with their macros.
- **Light and dark themes.** The dark theme is Discord-style charcoal, not pitch black.

![Macro editor](docs/screenshot-light.png)

## Installation

1. [Download the latest release](https://github.com/jonathant09/mouse-kb-remap/releases/latest):
   `mouse-kb-remap-<version>-windows-x64.zip`, or just `mouse-kb-remap.exe`.
2. Unzip it anywhere you keep apps, for example your Documents folder.
3. Run `mouse-kb-remap.exe`. Nothing else to install (Windows 10 and 11).

The exe isn't code-signed yet, so Windows SmartScreen may warn the first time: click
**More info → Run anyway**. Settings are saved to `%APPDATA%\mouse-kb-remap\config\`, so
replacing the exe with a newer version keeps them. To have it start when you sign in, use
*Settings → Start with Windows*.

> **Games that run as admin** (Genshin Impact does, for its anti-cheat) only accept input
> from programs that are also elevated. Use *Settings → Start with Windows → On, as
> administrator*, or right-click the exe → **Run as administrator**.

## Status

| Area | State |
| --- | --- |
| Windows engine (hooks, focus tracking, `SendInput`, macros, turbo) | Working; timing, combos, macros, shift layer and sensitivity tested end-to-end on Windows 11 |
| UI (profiles, mouse and keyboard diagrams, macro editor, settings) | Working |
| Linux engine (evdev + uinput, X11 focus tracking) | Experimental: builds and passes unit tests, not yet tried on real hardware |
| macOS engine | Not yet (the UI runs in preview mode) |

## Input latency

Measured on Windows 11 (release build, 1,000 samples per row): time from an input being
sent until a hook further down the chain receives it, without and with mouse-kb-remap.

| | Median | p95 | p99 | Max |
| --- | --- | --- | --- | --- |
| Key, without the app | 0.042 ms | 0.12 ms | 0.19 ms | 0.67 ms |
| Key, with the app (not remapped) | 0.067 ms | 0.22 ms | 0.44 ms | 1.4 ms |
| Mouse movement, without the app | 0.040 ms | 0.17 ms | 0.29 ms | 0.81 ms |
| Mouse movement, with the app | 0.066 ms | 0.25 ms | 0.38 ms | 1.2 ms |
| Remapped key, until the new key arrives | 0.075 ms | 0.26 ms | 0.36 ms | 1.7 ms |

So the app adds about 0.025 ms to a typical input, around 2.5% of one report from a
1,000 Hz mouse. When the active profile doesn't remap any mouse button or the wheel, the app
doesn't watch the mouse at all and mouse movement isn't affected. The tests used
`SendInput`, which enters Windows slightly earlier than real hardware but goes through the
same hooks.

## Building from source

Only needed to change the app. Install Rust from <https://rustup.rs>, then:

```powershell
git clone https://github.com/jonathant09/mouse-kb-remap
cd mouse-kb-remap
.\start.bat           # build and run from this folder
.\start.bat install   # build and install, or update in place, Documents\mouse-kb-remap
cargo test            # tests
```

See [Building & releasing](docs/building.md).

## Documentation

- [Getting started](docs/getting-started.md)
- [User guide](docs/user-guide.md): profiles, actions, shift layer, tray, settings
- [Macros & turbo](docs/macros.md): recording, editing, playback modes, timing tips
- [Configuration](docs/configuration.md): `config.json` format and all key ids
- [Troubleshooting](docs/troubleshooting.md): games, admin rights, anti-cheat, other remappers, Linux permissions
- [Architecture](docs/architecture.md): how the engine and UI fit together
- [Building & releasing](docs/building.md): toolchain, tests, CI, icon, cutting a release

## Project layout

```
start.bat            From source: run / build / run as admin / install to Documents
CHANGELOG.md         Release notes (each release's notes come from here)
.cargo/config.toml   Links the C runtime into the exe (no installs needed)
scripts/             install.ps1 (local install / update), release-notes.sh
assets/              App icon (SVG source, PNGs, .ico)
tools/               render-icon.mjs: regenerates the icon files
docs/                Documentation
.github/workflows/   Windows + Linux build, test and release CI
src/
  main.rs            Entry point: single instance, --tray flag
  app.rs             UI wiring: state, Slint callbacks, refresh()
  config.rs          Profiles, bindings, macros, JSON load/save, import/export
  keys.rs            Key table (TKL + number pad + extra keys), presets
  system.rs          Single instance, start with Windows, open files/URLs
  log.rs             Log file next to config.json
  engine/
    mod.rs           Engine API, profile compilation, recording → macro steps
    dispatch.rs      What one input event does (shared by all backends)
    player.rs        Timed playback: macros, turbo, text, launch
    windows.rs       Low-level hooks, focus tracking, SendInput (Windows)
    linux.rs         evdev grab + uinput output, X11 focus (Linux, experimental)
    fallback.rs      No-op engine so the UI runs on other OSes
ui/
  app.slint          Main window, status bar, tray icon
  store.slint        Global state shared with Rust
  sidebar.slint      Profiles, macros, settings, theme
  profile-view.slint Header + device canvas
  inspector.slint    Action editors (key, macro, turbo, …)
  macro-view.slint   Macro editor
  settings-view.slint Settings page
  mouse.slint        Generic mouse illustration with callouts
  keyboard.slint     Generic TKL / full-size keyboard
  controls.slint     Key capture box, modifier toggles
  widgets.slint      Buttons, chips, segmented control, fields, switch
  theme.slint        Colors, radii, type scale (light + dark)
```

## Roadmap

- [x] Tray icon + close-to-tray
- [x] Start with Windows, optionally elevated (Task Scheduler, one UAC prompt)
- [x] Macros: recording, editing, once / repeat / while held / toggle
- [x] Turbo and repeat-on-hold with custom timing
- [x] Modifier combos (Ctrl+C, Shift+F) as targets
- [x] Import / export profiles
- [x] Shift layer, sensitivity clutch, text, launch, media keys, wheel tilt
- [x] Linux backend (evdev + uinput), experimental
- [ ] Per-device bindings (tell two mice apart). Windows doesn't tell low-level hooks which
      device an event came from, so this needs a filter driver (e.g. Interception), which
      anti-cheats often block. Linux could do it with evdev; planned there first.
- [ ] Linux: try on real hardware, Wayland focus tracking (per compositor)
- [ ] macOS backend

## License

MIT
