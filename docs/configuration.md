# Configuration

All settings live in one JSON file, written every time you change something in the UI:

```
%APPDATA%\mouse-kb-remap\config\config.json
```

(On Linux: `~/.config/mouse-kb-remap/config.json`.) Set the environment variable
`MOUSE_KB_REMAP_CONFIG` to a file path to use a different file (portable installs,
testing). The same folder holds `mouse-kb-remap.log`.

You can edit the file by hand while the app is closed. If it doesn't parse, the app starts
with defaults and first copies your file to `config.broken.json` in the same folder, so a
typo never loses your profiles.

## Example

```json
{
  "version": 2,
  "theme": "dark",
  "enabled": true,
  "close_to_tray": true,
  "full_keyboard": false,
  "profiles": [
    {
      "name": "Default",
      "apps": [],
      "bindings": {
        "mouse:forward": { "type": "key", "key": "C", "mods": ["ctrl"] },
        "mouse:back": { "type": "shift" }
      },
      "shift_bindings": {
        "mouse:wheel_up": { "type": "key", "key": "VolumeUp" },
        "mouse:wheel_down": { "type": "key", "key": "VolumeDown" }
      }
    },
    {
      "name": "Genshin Impact",
      "apps": ["GenshinImpact.exe"],
      "bindings": {
        "mouse:back": { "type": "key", "key": "F" },
        "mouse:middle": {
          "type": "turbo",
          "output": { "type": "mouse", "button": "left" },
          "interval_ms": 40, "hold_ms": 15
        },
        "key:F1": { "type": "macro", "macro": "Burst", "mode": "while_held" },
        "key:CapsLock": { "type": "disabled" }
      }
    }
  ],
  "macros": [
    {
      "name": "Burst",
      "steps": [
        { "type": "key_tap", "key": "E", "hold_ms": 20 },
        { "type": "delay", "ms": 120 },
        { "type": "mouse_click", "button": "left", "hold_ms": 20 }
      ]
    }
  ]
}
```

## Top-level fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `version` | number | `2` | Format version. Version 1 files load unchanged. |
| `theme` | `"dark"` / `"light"` | `"dark"` | UI theme |
| `enabled` | bool | `true` | Master switch (the status-bar toggle) |
| `close_to_tray` | bool | `true` | Closing the window keeps the app in the tray |
| `full_keyboard` | bool | `false` | Show the number pad on the keyboard drawing |
| `profiles` | array | | **The first profile is always Default** |
| `macros` | array | `[]` | Macro library, referenced by name |

## Profile

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Shown in the sidebar |
| `apps` | string[] | Exe file names that activate this profile while focused. Case-insensitive; ignored on Default. |
| `bindings` | object | Map of **input id → action** |
| `shift_bindings` | object | Same, used while an input bound to `shift` is held (optional) |

## Input ids

Bindings are keyed by the physical input being remapped.

**Mouse:** `mouse:left`, `mouse:right`, `mouse:middle`, `mouse:back` (button 4 /
XButton1), `mouse:forward` (button 5 / XButton2), `mouse:wheel_up`, `mouse:wheel_down`,
`mouse:wheel_left`, `mouse:wheel_right` (wheel tilt / horizontal scroll)

**Keyboard:** `key:<key id>`, using the key ids below, e.g. `key:A`, `key:F5`,
`key:LShift`, `key:Numpad7`.

## Actions

| JSON | Effect |
| --- | --- |
| `{ "type": "key", "key": "C", "mods": ["ctrl", "shift"] }` | Send a key, with modifiers held around it (`mods` optional: `ctrl`, `shift`, `alt`, `win`). Held for as long as the input is held. |
| `{ "type": "mouse", "button": "left" }` | Send a mouse button or scroll step. Buttons: `left`, `right`, `middle`, `back`, `forward`, `wheel_up`, `wheel_down`, `wheel_left`, `wheel_right`. |
| `{ "type": "macro", "macro": "Name", "mode": "once", "count": 1, "gap_ms": 0 }` | Play a macro. `mode`: `once`, `repeat` (plays `count` times), `while_held`, `toggle`. `gap_ms`: pause between repeats. |
| `{ "type": "turbo", "output": {…}, "interval_ms": 50, "hold_ms": 20, "delay_ms": 0, "toggle": false }` | Repeat `output` (`{ "type": "key", "key": …, "mods": […] }` or `{ "type": "mouse", "button": … }`). `interval_ms` ≥ 5: press to press. `hold_ms`: press length. `delay_ms`: wait after the first press before repeating. `toggle`: press to start / stop instead of while held. |
| `{ "type": "text", "text": "gg wp", "delay_ms": 5 }` | Type text, `delay_ms` between characters |
| `{ "type": "launch", "path": "C:\\Tools\\app.exe", "args": "" }` | Start a program, or open a file, folder or URL |
| `{ "type": "shift" }` | Hold to use the profile's `shift_bindings` |
| `{ "type": "sensitivity", "speed": 4 }` | Hold to set the pointer speed (1–20, Windows default 10) |
| `{ "type": "disabled" }` | Swallow the input |

## Macros

```json
{ "name": "Burst", "steps": [ … ] }
```

| Step | Fields |
| --- | --- |
| `{ "type": "key_tap", "key": "E", "hold_ms": 20 }` | Press and release |
| `{ "type": "key_down", "key": "LShift" }` | Press |
| `{ "type": "key_up", "key": "LShift" }` | Release |
| `{ "type": "mouse_click", "button": "left", "hold_ms": 20 }` | Click, or one scroll step for wheel buttons |
| `{ "type": "mouse_down", "button": "right" }` | Press |
| `{ "type": "mouse_up", "button": "right" }` | Release |
| `{ "type": "delay", "ms": 100 }` | Wait |
| `{ "type": "text", "text": "hello" }` | Type text |

## Key ids

Key ids follow the generic ANSI layout shown in the app. Left and right modifiers are
separate keys.

| Group | Key ids |
| --- | --- |
| Function row | `Esc`, `F1` … `F12`, `PrintScreen`, `ScrollLock`, `Pause` |
| Number row | `Grave`, `1` … `9`, `0`, `Minus`, `Equal`, `Backspace`, `Insert`, `Home`, `PageUp` |
| Top row | `Tab`, `Q` … `P`, `LBracket`, `RBracket`, `Backslash`, `Delete`, `End`, `PageDown` |
| Home row | `CapsLock`, `A` … `L`, `Semicolon`, `Quote`, `Enter` |
| Bottom row | `LShift`, `Z` … `M`, `Comma`, `Period`, `Slash`, `RShift`, `Up` |
| Space row | `LCtrl`, `LWin`, `LAlt`, `Space`, `RAlt`, `RWin`, `Menu`, `RCtrl`, `Left`, `Down`, `Right` |
| Number pad | `NumLock`, `NumpadDivide`, `NumpadMultiply`, `NumpadSubtract`, `NumpadAdd`, `NumpadEnter`, `NumpadDecimal`, `Numpad0` … `Numpad9` |
| Extra function keys | `F13` … `F24` |
| Media | `MediaPlayPause`, `MediaNext`, `MediaPrev`, `MediaStop`, `VolumeUp`, `VolumeDown`, `VolumeMute` |
| Browser & launch | `BrowserBack`, `BrowserForward`, `BrowserRefresh`, `BrowserStop`, `BrowserSearch`, `BrowserFavorites`, `BrowserHome`, `LaunchMail`, `LaunchMedia`, `LaunchApp1` (This PC), `LaunchApp2` (Calculator) |

Notes:

- `Grave` is the `` ` `` / `~` key, `Quote` is `'`, `Menu` is the context-menu key.
- Unknown ids are ignored (the binding is skipped, not deleted). A binding to a macro that
  doesn't exist is skipped too.
