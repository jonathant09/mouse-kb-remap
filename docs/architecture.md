# Architecture

A single Rust binary with two halves: a **Slint UI** on the main thread and an **input
engine** on its own thread. They share no mutable state directly: the UI sends the engine
compiled lookup tables, and the engine sends the UI focus events.

```
┌────────────── main thread ──────────────┐        ┌──────────── engine thread ────────────┐
│ Slint event loop (ui/*.slint)           │        │ Win32 message loop                    │
│   ▲ properties / models   │ callbacks   │        │  • WH_MOUSE_LL hook                   │
│   │                       ▼             │ apply  │  • WH_KEYBOARD_LL hook                │
│ main.rs: State { Config, selection… }   │───────▶│  • EVENT_SYSTEM_FOREGROUND hook       │
│   refresh() pushes state into the UI    │        │  active: RwLock<Arc<HashMap<…>>>      │
│                                          │◀───────│ on_focus(FocusInfo)                   │
│ config.rs: load/save config.json        │ invoke_from_event_loop                         │
└─────────────────────────────────────────┘        └───────────────────────────────────────┘
```

## Source map

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Creates the window, owns `State`, wires every Slint callback, `refresh()` |
| `src/config.rs` | `Config` / `Profile` / `Action` types, JSON load + atomic save, profile matching |
| `src/keys.rs` | Keyboard layout (positions + Windows virtual-key codes) and key-id parsing |
| `src/engine/mod.rs` | Platform-independent API (`Engine`), `Input` / `Target` enums, `compile()` |
| `src/engine/windows.rs` | Hooks, focus tracking, elevation checks, `SendInput` |
| `src/engine/fallback.rs` | No-op engine for non-Windows builds (UI preview) |
| `ui/app.slint` | Main window layout and all callbacks/properties exposed to Rust |
| `ui/mouse.slint`, `ui/keyboard.slint` | Device illustrations |
| `ui/widgets.slint`, `ui/theme.slint` | Shared controls and design tokens |

## Data flow

### UI → engine

1. A Slint callback (for example `key-captured`) runs a closure in `main.rs`.
2. The closure mutates `State.cfg`, calls `Config::save()` and `Engine::apply(&cfg)`.
3. `engine::compile()` turns the config into `Vec<CompiledProfile>`:
   - input ids (`"key:A"`, `"mouse:back"`) become `Input::Key(vk)` / `Input::Mouse(..)`
   - actions become `Target::Key(vk)` / `Target::Mouse(..)` / `Target::Disabled`
   - Default's bindings are merged underneath every other profile, so inheritance costs
     nothing at input time
4. The engine stores the profiles and immediately recomputes the active table for the
   currently focused window.
5. `refresh()` rebuilds the Slint models. It's a few hundred small structs, cheap enough
   to rebuild wholesale instead of diffing.

### Engine → UI

The foreground-window hook calls `on_foreground(hwnd)` on the engine thread. It:

1. resolves the window's process id and exe name (`QueryFullProcessImageNameW`),
2. checks whether that process is elevated while we are not (UIPI would block us),
3. swaps the `active` bindings table,
4. calls the `on_focus` callback, which uses `slint::invoke_from_event_loop` to invoke
   the Slint callback `engine-focus` on the UI thread. `main.rs` handles that like any
   other callback (updates "Recent" apps, the LIVE badge and the status bar).

## The hot path

`mouse_proc` / `keyboard_proc` run for **every** mouse move, click and key press on the
system. Windows removes a low-level hook without telling anyone if it takes too long
(`LowLevelHooksTimeout`), so the hot path is deliberately tiny:

1. Ignore injected events (`LLMHF_INJECTED` / `LLKHF_INJECTED`). That covers our own
   `SendInput` output and other tools' synthetic input, which also prevents feedback loops.
2. Decode the event into `(Input, Phase)`. Mouse moves return early.
3. `active.try_read()`: never block. If a writer holds the lock we just pass the event
   through.
4. On press, remember `input → target` in a thread-local `HELD` map. On release, use that
   remembered target, so a profile switch mid-press can't leave a key stuck down.
5. `SendInput` the target and return `1` to swallow the original.

Keys are sent as **scan codes** (`KEYEVENTF_SCANCODE`, with `KEYEVENTF_EXTENDEDKEY` when
`MapVirtualKeyW(.., MAPVK_VK_TO_VSC_EX)` reports an `E0`/`E1` prefix). Many games read
raw input or DirectInput and ignore virtual-key-only events.

## Safety rules

- **Never remap our own window.** When the foreground process is us, the active table is
  empty. A user who disables left-click in Default can still click the UI to undo it.
- **Config writes are atomic**: write `config.json.tmp`, then rename.
- **Broken configs are preserved** as `config.broken.json` before defaults load.

## UI conventions

- All colors, radii and font sizes come from the `Theme` global in `ui/theme.slint`. Don't
  hard-code colors in components.
- Dark theme surfaces follow a Discord-like charcoal ramp (`#1e1f22 → #2b2d31 → #313338
  → #383a40`); light mirrors it with neutral greys.
- Rust owns all state. Slint components only expose `in` properties and callbacks, except
  purely visual state like the Mouse / Keyboard tab.
- Mouse buttons are passed as a fixed-order array (see `MOUSE_ORDER` in `main.rs` and the
  comment in `mouse.slint`). Keep them in sync if you add a button.

## Adding a new action type

1. Add a variant to `config::Action` (serde tag `type`) and a `label()`.
2. Add a matching `engine::Target` variant and map it in `engine::parse_action`.
3. Emit it in `engine/windows.rs::emit()`.
4. Expose it in the inspector (`ui/app.slint`, the *ACTION* segmented control) and handle
   the new kind index in `main.rs` (`kind_of`, `on_set_kind`).
5. Document the JSON shape in [configuration.md](configuration.md).
