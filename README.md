# mouse-kb-remap

A lightweight, open-source, brand-agnostic remapper for mice and keyboards. Think the
per-game profiles of Logitech G HUB or Razer Synapse, but in one small app that works with
**any** mouse or keyboard.

![Dark theme](docs/screenshot-dark.png)

## What it does

- **Per-app profiles.** A profile activates automatically while its app's window is focused
  (e.g. `GenshinImpact.exe`) and switches back when you alt-tab away.
- **Default profile.** Applies everywhere else. Other profiles inherit its bindings, so
  global remaps keep working inside games unless you override them.
- **Remap mouse buttons and keys.** Any of them can send a key, a mouse button or scroll
  step, or be disabled.
- **Games-friendly key output.** Keys are sent as scan codes, which most games read
  (unlike virtual-key-only tools).
- **Safe by design.** Remaps never apply while this app's own window is focused, so a bad
  binding can't lock you out.
- **Admin awareness.** Warns you when the focused game runs as admin (Windows blocks
  non-admin apps from sending it input).
- **Light and dark themes.** The dark theme is Discord-style charcoal, not pitch black.

![Light theme](docs/screenshot-light.png)

## Status

Early prototype (v0.1).

| Area | State |
| --- | --- |
| UI (profiles, mouse and keyboard diagrams, bindings) | Working |
| Windows engine (hooks, focus tracking, `SendInput`) | Written, needs real-world testing |
| Linux / macOS engine | Not yet (the UI runs in preview mode) |

## Building on Windows

1. Install Rust: download `rustup-init.exe` from <https://rustup.rs> and accept the defaults.
   If it asks for the **Visual Studio C++ Build Tools**, install them (it's the linker Rust
   uses on Windows).
2. Build and run:

   ```powershell
   cd mouse-kb-remap
   cargo run            # debug build with a console window for logs
   cargo run --release  # optimized build, no console window
   ```

The release binary lands in `target\release\mouse-kb-remap.exe`.

Settings are saved to `%APPDATA%\mouse-kb-remap\config\config.json`.

> **Games that run as admin** (Genshin Impact does, for its anti-cheat) only accept input
> from programs that are also elevated. Right-click the exe and choose **Run as
> administrator** for now. A "start elevated at login" option via Task Scheduler is on
> the roadmap.

## Project layout

```
src/
  main.rs            UI wiring: pushes state into Slint, handles callbacks
  config.rs          Profiles, bindings, JSON load/save
  keys.rs            Keyboard layout (generic 87-key ANSI) + key ids / VK codes
  engine/
    mod.rs           Platform-independent engine API + profile compilation
    windows.rs       Low-level hooks, focus tracking, SendInput (Windows)
    fallback.rs      No-op engine so the UI runs on other OSes
ui/
  app.slint          Main window: sidebar, header, canvas, inspector, status bar
  mouse.slint        Generic mouse illustration with callouts
  keyboard.slint     Generic TKL keyboard
  widgets.slint      Buttons, chips, segmented control, text field
  theme.slint        Colors, radii, type scale (light + dark)
```

### How the engine works

A dedicated thread installs `WH_MOUSE_LL` / `WH_KEYBOARD_LL` hooks and an
`EVENT_SYSTEM_FOREGROUND` hook. When the foreground window changes, it looks up the exe
name, picks the matching profile and swaps in a pre-built `HashMap` of bindings. The input
hooks only do a non-blocking lookup and a `SendInput`, because Windows silently removes
low-level hooks that respond too slowly.

## Roadmap

- [ ] Tray icon + close-to-tray (Slint's `system-tray` feature)
- [ ] Start with Windows, optionally elevated (Task Scheduler, one UAC prompt)
- [ ] Macros: sequences, hold-to-repeat / turbo (opt-in, with a game ToS warning)
- [ ] Modifier combos (Ctrl+C, Shift+F) as targets
- [ ] Import / export profiles
- [ ] Per-device bindings (tell two mice apart)
- [ ] Linux backend (evdev + uinput)

## License

MIT
