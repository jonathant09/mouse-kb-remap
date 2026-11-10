# Architecture

A single Rust binary: a **Slint UI** on the main thread, and an **input engine** on its own
threads. They share no mutable state directly: the UI sends the engine compiled lookup
tables, and the engine sends the UI events.

```
┌──────────── main thread ────────────┐       ┌────────── engine thread (Windows) ──────────┐
│ Slint event loop (ui/*.slint)       │       │ Win32 message loop                          │
│   Store global ◀── refresh()        │ apply │  • WH_MOUSE_LL / WH_KEYBOARD_LL hooks       │
│   callbacks ──▶ app.rs (State)      │──────▶│  • EVENT_SYSTEM_FOREGROUND hook             │
│ config.rs: load/save config.json    │       │  active: RwLock<Option<Arc<Layers>>>        │
│ AppTray (SystemTrayIcon)            │◀──────│  Dispatcher (dispatch.rs): held keys, shift │
│                                     │ events└──────────────┬──────────────────────────────┘
└─────────────────────────────────────┘ (invoke_from_        │ Cmd (mpsc)
                                          event_loop)        ▼
                                        ┌──────────── player thread ──────────────────────────┐
                                        │ Scheduler (player.rs): macros, turbo, text, launch, │
                                        │ sensitivity; precise sleeps → SendInput             │
                                        └─────────────────────────────────────────────────────┘
```

## Source map

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Single-instance check, `--tray` flag, starts the app |
| `src/app.rs` | Owns `State`, wires every Slint callback, `refresh()` pushes state into `Store`; tray and close-to-tray |
| `src/config.rs` | `Config` / `Profile` / `Action` / `Macro` types, JSON load + atomic save, import/export |
| `src/keys.rs` | Key table (positions, Windows virtual-key codes), media keys, shortcut presets |
| `src/system.rs` | Single instance (named mutex + event), start with Windows (Run key / Task Scheduler), open files and URLs |
| `src/log.rs` | `log!` macro: stderr and `mouse-kb-remap.log` |
| `src/engine/mod.rs` | Platform-independent API (`Engine`), `Input` / `Target` / `Layers`, `compile()`, recording → macro steps |
| `src/engine/dispatch.rs` | `Dispatcher`: what one input event does. Shared by all backends, unit-tested. |
| `src/engine/player.rs` | `Scheduler` (timed playback state machine, unit-tested) and the worker loop |
| `src/engine/windows.rs` | Hooks, focus tracking, elevation checks, key capture, recording, `SendInput` |
| `src/engine/linux.rs` | evdev grab per device, uinput output, X11 focus tracking |
| `src/engine/fallback.rs` | No-op engine for other OSes (UI preview) |
| `ui/store.slint` | The `Store` global: every property and callback shared with Rust |
| `ui/app.slint` | Main window and `AppTray` |
| `ui/*-view.slint`, `ui/inspector.slint`, `ui/sidebar.slint` | Views |
| `ui/mouse.slint`, `ui/keyboard.slint` | Device illustrations |
| `ui/controls.slint`, `ui/widgets.slint`, `ui/theme.slint` | Shared controls and design tokens |

## Data flow

### UI → engine

1. A Slint callback on `Store` (for example `set-turbo-number`) runs a closure in `app.rs`.
2. `App::edit` mutates `State.cfg`, calls `Config::save()` and `Engine::apply(&cfg)`.
   (`App::nav` is for UI-only changes: it just redraws.)
3. `engine::compile()` turns the config into `Vec<CompiledProfile>`:
   - input ids (`"key:A"`, `"mouse:back"`) become `Input::Key(vk)` / `Input::Mouse(..)`
   - actions become `Target`s: `Key { vk, mods }`, `Mouse`, `Disabled`, `Shift`,
     `Sensitivity`, or `Play(Arc<Plan>)` for anything timed (macro, turbo, text, launch)
   - Default's base and shift layers are merged underneath every other profile, so
     inheritance costs nothing at input time
4. The engine stores the profiles, recomputes the active `Layers` for the focused window
   and tells the player to stop everything (bindings may have changed under it).
5. `refresh()` rebuilds the Slint models. It's a few hundred small structs, cheap enough
   to rebuild wholesale instead of diffing.

### Engine → UI

The engine calls `on_event(EngineEvent)` from its thread; `app.rs` forwards it with
`slint::invoke_from_event_loop` to the UI thread, where a thread-local handle to the `App`
handles it:

- `Focus(FocusInfo)` on every foreground change: updates *Recent* apps, the LIVE badge and
  the status bar.
- `Captured { vk, mods }` when a key was pressed while a capture box was waiting.

## The hot path

`mouse_proc` / `keyboard_proc` run for **every** mouse click and key press on the system
(mouse moves return immediately). Windows removes a low-level hook without telling anyone if
it takes too long (`LowLevelHooksTimeout`), so the hook does very little.

Every hooked event makes a round trip to the engine thread, so:

- **The mouse hook is only installed while it's needed**: when the active profile remaps a
  mouse button or the wheel, or while a macro is recording (`update_mouse_hook`, re-checked
  on focus changes, config changes and recording start/stop). Otherwise mouse movement and
  clicks never pass through the app. If a remapped button is held when the hook should go,
  removal waits for its release so the key it sends can't get stuck. The keyboard hook stays
  installed (key capture, recording).
- **The engine thread runs at `THREAD_PRIORITY_HIGHEST`** so hooked input never queues
  behind ordinary work on a busy PC; the playback thread runs above normal.

Per event:

1. Ignore our own injected events (tagged with `dwExtraInfo = "MKRR"`) and, by default, all
   other synthetic input too. That prevents feedback loops.
2. Decode the event into `(Input, Phase)` (Down, Up, or Tap for wheel steps).
3. `active.try_read()`: never block. If a writer holds the lock the event passes through.
4. `Dispatcher::handle` decides:
   - On press, remember `input → target` in `held`. On release, use that remembered target,
     so a profile switch mid-press can't leave a key stuck down. Keyboard auto-repeat
     re-sends the *held* target instead of looking it up again.
   - Keys, combos and mouse buttons are sent immediately with one `SendInput`.
   - `Shift` adjusts a counter; while it's above zero, lookups try the shift layer first.
   - Anything timed is handed to the player thread as a `Cmd` over a channel.
5. Return `1` to swallow the original.

Keys are sent as **scan codes** (`KEYEVENTF_SCANCODE`, with `KEYEVENTF_EXTENDEDKEY` when
`MapVirtualKeyW(.., MAPVK_VK_TO_VSC_EX)` reports an `E0`/`E1` prefix). Many games read
raw input or DirectInput and ignore virtual-key-only events. Media/browser keys, Pause and
NumLock are sent by virtual key; Number-pad Enter uses the unassigned code `0x0E`
internally.

## Timed playback

`player.rs` keeps a list of running players (one per trigger input). Each runs a `Plan`:
a list of `Step`s (key down/up, button down/up, wheel, char, delay, launch) plus a mode
(once, repeat N, while held, toggle) and gaps. The worker loop:

1. waits for the next due step, using an interruptible channel wait for the bulk and a
   precise `std::thread::sleep` (high-resolution timer on Windows 10+) for the last 20 ms;
2. drains new commands (press, release, tap, stop all);
3. runs due steps and sends the output in one batch.

Timing is drift-free (each wait is added to the previous deadline, not to "now") but never
bursts to catch up after a stall. Players track what they hold down and release it when
stopped. Loops without any delay get a 1 ms floor, so they can't spin.

The `Scheduler` takes explicit timestamps, so its tests simulate time instead of sleeping.

## Linux backend

Every keyboard and relative mouse is grabbed with `EVIOCGRAB`; one thread per device reads
events, feeds remappable ones through the same `Dispatcher`, and forwards everything else to
a uinput virtual device at each `SYN_REPORT`. A scanner thread grabs devices plugged in
later. The focused window comes from X11 (`_NET_ACTIVE_WINDOW` → `_NET_WM_PID` →
`/proc/<pid>/exe`). Key identity uses the same virtual-key codes as Windows, mapped to
evdev codes by a table. The sensitivity clutch scales forwarded `REL_X` / `REL_Y` motion.

## Safety rules

- **Never remap our own window.** When the foreground process is us, there is no active
  table. A user who disables left-click in Default can still click the UI to undo it.
- **Focusing our window stops all playback** (and so does any profile switch or config
  change), so a toggled turbo can always be stopped.
- **Key capture is cancelled** when focus leaves our window.
- **Pointer speed** is changed for the session only (`SPI_SETMOUSESPEED` without
  `SPIF_UPDATEINIFILE`) and restored on release and on quit.
- **Config writes are atomic**: write `config.json.tmp`, then rename.
- **Broken configs are preserved** as `config.broken.json` before defaults load.
- **One instance**: a second launch signals the first to show its window and exits. Copies started with a different `MOUSE_KB_REMAP_CONFIG` count as separate instances.
- **No raw keyboard input in the UI.** winit registers the window for raw keyboard input, and while such a window is focused Windows skips low-level keyboard hooks owned by the same process: key capture and macro recording would go deaf whenever the app is in front. `system::stop_raw_keyboard_input()` removes that registration right after the window is created.

## UI conventions

- All colors, radii and font sizes come from the `Theme` global in `ui/theme.slint`. Don't
  hard-code colors in components.
- Dark theme surfaces follow a Discord-like charcoal ramp (`#1e1f22 → #2b2d31 → #313338
  → #383a40`); light mirrors it with neutral greys.
- Rust owns all state, exposed through the `Store` global. Components keep only purely
  visual state (the Mouse / Keyboard tab, the inspector tab).
- Text fields bound to Rust values use `ValueField`, which reports edits on Enter / focus
  loss and then shows whatever Rust stored.
- Mouse buttons are passed as a fixed-order array (see `MOUSE_ORDER` in `app.rs` and the
  comment in `mouse.slint`). Keep them in sync if you add a button.
- Views get an explicit `max-width` in `app.slint`: without it, a view whose content has a
  bounded width would shrink the window.

## Adding a new action type

1. Add a variant to `config::Action` (serde tag `type`) with `label()` and `short_label()`.
2. Map it to a `Target` in `engine::parse_action` (often a `Plan` of existing `Step`s).
3. If it needs new output, add an `Out` variant and handle it in `windows.rs::emit_outs`
   and `linux.rs::emit_outs`.
4. Add it to `kind-names` in `ui/store.slint`, an editor in `ui/inspector.slint`, and the
   kind index in `app.rs` (`kind_of`, `set_kind`, `refresh`).
5. Document the JSON shape in [configuration.md](configuration.md) and add a test.
