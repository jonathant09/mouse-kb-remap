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
│ AppTray (SystemTrayIcon)            │◀──────│  Dispatcher (dispatch.rs): held keys, layers│
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
| `src/main.rs` | Command-line flags, single-instance check, hand-over after an update, starts the app |
| `src/app.rs` | Owns `State`, wires every Slint callback, `refresh()` pushes state into `Store`; tray and close-to-tray, undo, updates, command-line answers |
| `src/config.rs` | `Config` / `Profile` / `Action` / `Macro` types, combo ids, app rules and wildcards, hotkeys, JSON load + atomic save, import/export |
| `src/keys.rs` | Key table (positions, Windows virtual-key codes), media keys, shortcut presets |
| `src/system.rs` | Single instance (named mutex + event), start with Windows (Run key / Task Scheduler), open files and URLs, console for CLI output |
| `src/log.rs` | `log!` macro: stderr and `mouse-kb-remap.log` |
| `src/osd.rs` | On-screen notifications (Windows) |
| `src/update.rs` | Release check and in-place update |
| `src/ipc.rs` | Command-line control over a named pipe |
| `src/engine/mod.rs` | Platform-independent API (`Engine`), `Input` / `Target` / `Layers` / `ComboBinding`, `compile()`, profile picking, recording → macro steps |
| `src/engine/dispatch.rs` | `Dispatcher`: what one input event does (layers, combos, tap / hold, hotkeys). Shared by all backends, unit-tested. |
| `src/engine/player.rs` | `Scheduler` (timed playback state machine, unit-tested) and the worker loop |
| `src/engine/windows.rs` | Hooks, timers, focus tracking (exe, path, title, fullscreen), elevation checks, key capture, recording, `SendInput` |
| `src/engine/win_actions.rs` | System actions: lock, microphone, audio output, window actions |
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
3. `engine::compile()` turns the config into `Compiled { profiles, hotkeys }`:
   - input ids (`"key:A"`, `"mouse:back"`) become `Input::Key(vk)` / `Input::Mouse(..)`;
     combo ids (`"ctrl+mouse:back"`) become `ComboBinding`s
   - actions become `Target`s: `Key { vk, mods }`, `Mouse`, `Disabled`, `Layer`,
     `Sensitivity`, `TapHold(Arc<TapHold>)`, or `Play(Arc<Plan>)` for anything timed
     (macro, turbo, text, launch, system actions)
   - Default's layers (base, shift, 2, 3, plain and combos) are merged underneath every
     other profile, so inheritance costs nothing at input time
   - `apps` entries become `AppRule`s (exe name, path or title, with wildcards)
4. The engine stores the profiles, picks the active one for the focused window
   (`pick_profile`: the most specific matching rule, or a profile forced by hand) and tells
   the player to stop everything (bindings may have changed under it).
5. `refresh()` rebuilds the Slint models. It's a few hundred small structs, cheap enough
   to rebuild wholesale instead of diffing.

### Engine → UI

The engine calls `on_event(EngineEvent)` from its thread; `app.rs` forwards it with
`slint::invoke_from_event_loop` to the UI thread, where a thread-local handle to the `App`
handles it:

- `Focus(FocusInfo)` on every foreground change and whenever the active profile changes:
  updates *Recent* apps, the LIVE badge (from the engine's profile index) and the status
  bar, and announces profile switches when notifications are on.
- `Captured { vk, mods }` when a key was pressed while a capture box was waiting.
- `Command(AppCommand)` for app hotkeys and *Next / Automatic profile* actions: pause,
  or `Engine::force_profile`.
- `Notice(String)` for things worth an on-screen notification (layer and playback toggles,
  microphone, audio output).

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
   - Layer targets adjust a per-layer hold counter or toggle a lock bit; lookups try the
     highest active layer first and fall back down to base.
   - Anything timed is handed to the player thread as a `Cmd` over a channel.
5. Return `1` to swallow the original.

The dispatcher also keeps the set of physical keys and buttons that are down, which drives:

- **App hotkeys**, checked first on every new key press (even while paused) against the
  exact modifier bits held. A matching press and its release are swallowed and reported to
  the UI as `EngineEvent::Command`.
- **Combos** (`ComboBinding`: modifier bits + held inputs + trigger), stored per layer, most
  specific first, and checked before the layer's plain bindings. Modifier bits come from
  physical modifier keys plus modifiers that remapped held keys are sending. When a combo
  sends a key, the held modifiers are released around it (`lift_mods`), with an unassigned
  "mask" key (`0xE8`) first when Alt or Win is involved so lifting them doesn't open a menu.
- **Tap / hold** (`Target::TapHold`): a press stores a `Pending` decision instead of acting.
  Its release before `hold_ms` is a tap (or starts the double-tap wait); `tick()` at
  `deadline()` turns it into a hold or sends the delayed tap; any other press first resolves
  it as a hold. That other input was already seen by Windows before our output, so when it
  would pass through it's swallowed and re-sent after, keeping the order.
- **`prune`** on focus changes forgets pass-through keys whose release went to a window the
  hooks couldn't see (checked with `GetAsyncKeyState`), so they don't stay "held" for combos.

The engine thread runs two thread timers (`SetTimer` with no window): one at the
dispatcher's tap / hold deadline, one every 500 ms that re-reads the focused window's title
and fullscreen state, only while some profile has a title or fullscreen rule.

Keys are sent as **scan codes** (`KEYEVENTF_SCANCODE`, with `KEYEVENTF_EXTENDEDKEY` when
`MapVirtualKeyW(.., MAPVK_VK_TO_VSC_EX)` reports an `E0`/`E1` prefix). Many games read
raw input or DirectInput and ignore virtual-key-only events. Media/browser keys, Pause and
NumLock are sent by virtual key; Number-pad Enter uses the unassigned code `0x0E`
internally.

## Timed playback

`player.rs` keeps a list of running players (one per trigger input). Each runs a `Plan`:
a list of `Step`s (key down/up, button down/up, wheel, char, delay, launch, pointer move,
system action) plus a mode (once, repeat N, while held, toggle), gaps, optional delay jitter
(a small xorshift generator) and a label used for toggle notices. The worker loop:

1. waits for the next due step, using an interruptible channel wait for the bulk and a
   precise `std::thread::sleep` (high-resolution timer on Windows 10+) for the last 20 ms;
2. drains new commands (press, release, tap, stop all);
3. runs due steps and sends the output in one batch.

Timing is drift-free (each wait is added to the previous deadline, not to "now") but never
bursts to catch up after a stall. Players track what they hold down and release it when
stopped. Loops without any delay get a 1 ms floor, so they can't spin.

The `Scheduler` takes explicit timestamps, so its tests simulate time instead of sleeping.

System actions (`win_actions.rs`) run from the playback thread: window actions on the
foreground window directly, audio (Core Audio `IAudioEndpointVolume`, and the undocumented
`IPolicyConfig` that Windows' own Sound settings use to change the default device) on a
short-lived COM thread. *Next profile* / *Automatic profile* go to the UI as commands.

## Around the engine

| Module | |
| --- | --- |
| `osd.rs` | On-screen notifications: a layered, click-through, never-activated Win32 popup on its own thread, painted with GDI. Using a Slint window would activate it, which counts as "our window is focused" and pauses remapping. |
| `update.rs` | GitHub release check and in-place update (WinHTTP, SHA-256 via `sha2`). Download next to the exe, verify size / checksum / `MZ` / `--version`, rename the running exe to `.old.exe`, move the new one in, start it with `--after-update <pid>`; it waits for the old process to exit before taking the single-instance slot and deletes `.old.exe` a minute later. `MOUSE_KB_REMAP_UPDATE_URL` points the check elsewhere (tests). |
| `ipc.rs` | Command-line control: a named pipe (`\\.\pipe\mouse-kb-remap[.<config hash>]`) whose security allows only the current user, with a medium integrity label so a non-admin caller reaches an admin copy. One line in, one `ok …` / `error …` line out; answered on the UI thread. |

Undo lives in `app.rs`: every `edit` snapshots profiles and macros; changes within 700 ms
merge into one step (typing). Settings (theme, startup, hotkeys…) aren't part of undo.

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
- **One instance**: a second launch signals the first to show its window and exits. `--quit` signals it to quit instead (the install script uses it). The named mutex and events get a medium integrity label, so a normal launch can reach a copy running as administrator. Copies started with a different `MOUSE_KB_REMAP_CONFIG` count as separate instances.
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
4. Add a kind constant and name in `app.rs` (`KIND_NAMES`, and `SLOT_NAMES` / `SLOT_KINDS`
   if it can be a tap / hold part), handle it in `kind_of`, `set_kind` and `refresh`, and add
   an editor in `ui/inspector.slint` keyed on `Store.selected-kind`.
5. Document the JSON shape in [configuration.md](configuration.md) and add a test.
