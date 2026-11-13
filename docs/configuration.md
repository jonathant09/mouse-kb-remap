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
  "version": 3,
  "theme": "dark",
  "enabled": true,
  "close_to_tray": true,
  "full_keyboard": false,
  "notifications": false,
  "check_updates": false,
  "hotkeys": {
    "pause": { "key": "F12", "mods": ["ctrl", "alt"] }
  },
  "profiles": [
    {
      "name": "Default",
      "apps": [],
      "bindings": {
        "mouse:forward": { "type": "key", "key": "C", "mods": ["ctrl"] },
        "mouse:back": { "type": "layer", "layer": 1 },
        "key:CapsLock": {
          "type": "tap_hold",
          "tap": { "type": "key", "key": "Esc" },
          "hold": { "type": "key", "key": "LCtrl" }
        },
        "ctrl+mouse:forward": { "type": "system", "command": "next_monitor" }
      },
      "shift_bindings": {
        "mouse:wheel_up": { "type": "key", "key": "VolumeUp" },
        "mouse:wheel_down": { "type": "key", "key": "VolumeDown" }
      }
    },
    {
      "name": "Genshin Impact",
      "apps": ["GenshinImpact.exe"],
      "fullscreen_only": true,
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
| `version` | number | `4` | Format version. Version 1–3 files load unchanged. |
| `theme` | `"dark"` / `"light"` | `"dark"` | UI theme |
| `enabled` | bool | `true` | Master switch (the status-bar toggle) |
| `close_to_tray` | bool | `true` | Closing the window keeps the app in the tray |
| `full_keyboard` | bool | `false` | Show the number pad on the keyboard drawing |
| `notifications` | bool | `false` | On-screen notifications |
| `check_updates` | bool | `false` | Look for a new version at start and once a day |
| `hotkeys` | object | `{}` | App hotkeys: `pause`, `next_profile`, `auto_profile`, each `{ "key": …, "mods": [ … ] }` (optional) |
| `profiles` | array | | **The first profile is always Default** |
| `macros` | array | `[]` | Macro library, referenced by name |
| `devices` | array | `[]` | Named mice and controllers: `{ "name": "Trackball", "id": … }` (below) |
| `stick_speed` | number | `10` | Pointer speed (1–20) when a controller stick moves the pointer |
| `pad` | object | | Controller settings: `gyro_speed`, `touch_speed` (1–20, default 10), `gyro_invert_x`, `gyro_invert_y`, `labels` (`"xbox"`, `"play_station"`, `"nintendo"`), `xinput`, `direct`, `skip_steam_virtual` (all three on by default), `steam_lizard_off` (off) |

## Profile

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Shown in the sidebar |
| `apps` | string[] | What activates this profile (below). Ignored on Default. |
| `fullscreen_only` | bool | Only while the matching window covers its whole screen (optional) |
| `bindings` | object | Map of **input id → action** (the base layer) |
| `shift_bindings` | object | Layer 1 (the shift layer), used while an input bound to it is held or toggled (optional) |
| `layers` | object[] | Layers 2 and 3, the same kind of map (optional) |
| `stick_pointer` | `"off"` / `"left"` / `"right"` | Controller stick that moves the pointer. Left out: same as Default (off for Default itself). |
| `gyro_pointer`, `touch_pointer` | bool | The controller gyro / touchpad moves the pointer. Left out: same as Default (off for Default itself). |

Entries in `apps`, all case-insensitive, `*` and `?` as wildcards:

| Entry | Matches |
| --- | --- |
| `game.exe`, `*launcher*.exe` | The program's file name |
| `C:\\Games\\*\\game.exe` | The program's full path (anything containing `\` or `/`) |
| `title:YouTube` | The window title: contains the text, or matches it when it has wildcards |

The most specific match wins (title, then path, then file name), then the profile higher
in the list.

## Input ids

Bindings are keyed by the physical input being remapped.

**Mouse:** `mouse:left`, `mouse:right`, `mouse:middle`, `mouse:back` (button 4 /
XButton1), `mouse:forward` (button 5 / XButton2), `mouse:wheel_up`, `mouse:wheel_down`,
`mouse:wheel_left`, `mouse:wheel_right` (wheel tilt / horizontal scroll)

**Keyboard:** `key:<key id>`, using the key ids below, e.g. `key:A`, `key:F5`,
`key:LShift`, `key:Numpad7`.

**Controller:** `pad:a`, `pad:b`, `pad:x`, `pad:y`, `pad:lb`, `pad:rb`, `pad:lt`, `pad:rt`
(triggers, pressed past a third), `pad:back` (View), `pad:start` (Menu), `pad:ls`, `pad:rs`
(stick clicks), `pad:up`, `pad:down`, `pad:left`, `pad:right` (d-pad), and the sticks pushed
past half way: `pad:ls_up`, `pad:ls_down`, `pad:ls_left`, `pad:ls_right`, `pad:rs_up`,
`pad:rs_down`, `pad:rs_left`, `pad:rs_right`; `pad:guide` (Xbox, PS, Home, Steam), `pad:share`
(Share, Capture, Steam quick access), `pad:touchpad` (touchpad or Steam right trackpad click),
`pad:mic`, `pad:p1` … `pad:p4` (back paddles, Joy-Con SL / SR, Steam L4 / R4 / L5 / R5, see
the user guide), `pad:left_pad` (Steam left trackpad click), and `pad:button1` … `pad:button32`
for numbered buttons of controllers without a known layout. Buttons are named by position, Xbox
style: `pad:a` is the bottom face button (Cross, or B on Nintendo).

**Combos:** parts joined by `+`; the last is the input pressed, the others must be held:
modifiers (`ctrl`, `shift`, `alt`, `win`, either side) and other input ids. For example
`ctrl+mouse:back`, `ctrl+shift+key:F1`, `mouse:right+mouse:left`, `key:Q+key:E`.

**One device:** `@` and a device's name after an id limits the binding to that device:
`mouse:back@Trackball`, `ctrl+mouse:back@Trackball` (the device is the one the last input
comes from). Bindings for a device that isn't in `devices` are ignored.

## Devices

```json
"devices": [
  { "name": "Trackball", "id": "\\\\?\\HID#VID_046D&PID_C539&MI_01&Col01#8&2d8e1a4&0&0000#{378de44c-56ef-11d1-bc8c-00a0c91405dd}" },
  { "name": "Pad", "id": "xinput:1" }
]
```

`name` is what binding ids use (no `@` or `+`). `id` says how the device is recognised:

| `id` | Device |
| --- | --- |
| `\\?\HID#VID_…&PID_…#…` | A mouse on Windows (its raw input device path). If the exact path isn't connected (another USB port), a device with the same vendor, product and interface matches, then one with the same vendor and product. |
| `xinput:1` … `xinput:4` | A controller in that XInput slot (Windows) |
| `\\?\HID#VID_054C&PID_0CE6…` | A PlayStation, Switch, Steam or other controller read directly (Windows; its HID device path, matched like a mouse's) |
| `evdev:vendor:product:name` | Any input device on Linux, e.g. `evdev:046d:c539:Logitech G502` |

Use *Settings → Devices → Add a device* rather than writing ids by hand.

## Actions

| JSON | Effect |
| --- | --- |
| `{ "type": "key", "key": "C", "mods": ["ctrl", "shift"] }` | Send a key, with modifiers held around it (`mods` optional: `ctrl`, `shift`, `alt`, `win`). Held for as long as the input is held. |
| `{ "type": "mouse", "button": "left" }` | Send a mouse button or scroll step. Buttons: `left`, `right`, `middle`, `back`, `forward`, `wheel_up`, `wheel_down`, `wheel_left`, `wheel_right`. |
| `{ "type": "macro", "macro": "Name", "mode": "once", "count": 1, "gap_ms": 0 }` | Play a macro. `mode`: `once`, `repeat` (plays `count` times), `while_held`, `toggle`. `gap_ms`: pause between repeats. |
| `{ "type": "turbo", "output": {…}, "interval_ms": 50, "hold_ms": 20, "delay_ms": 0, "toggle": false }` | Repeat `output` (`{ "type": "key", "key": …, "mods": […] }` or `{ "type": "mouse", "button": … }`). `interval_ms` ≥ 5: press to press. `hold_ms`: press length. `delay_ms`: wait after the first press before repeating. `toggle`: press to start / stop instead of while held. |
| `{ "type": "text", "text": "gg wp", "delay_ms": 5 }` | Type text, `delay_ms` between characters |
| `{ "type": "launch", "path": "C:\\Tools\\app.exe", "args": "" }` | Start a program, or open a file, folder or URL |
| `{ "type": "layer", "layer": 2, "toggle": false }` | Use layer 1 (shift), 2 or 3 while held, or (`toggle`) switch it on and off |
| `{ "type": "shift" }` | Older files: the same as `{ "type": "layer", "layer": 1 }` (converted when loaded) |
| `{ "type": "sensitivity", "speed": 4 }` | Hold to set the pointer speed (1–20, Windows default 10) |
| `{ "type": "tap_hold", "tap": {…}, "hold": {…}, "double": {…}, "hold_ms": 200, "double_ms": 250 }` | An action on a tap, one when held for `hold_ms`, one on a double tap within `double_ms`. Each part is optional and is any action except another `tap_hold`. |
| `{ "type": "system", "command": "mute_mic" }` | `lock_pc`, `mute_mic`, `next_audio_output`, `minimize_window`, `maximize_window` (toggles), `always_on_top` (toggles), `next_monitor`, `close_window`, `next_profile`, `auto_profile` |
| `{ "type": "gesture", "tap": {…}, "up": {…}, "down": {…}, "left": {…}, "right": {…}, "distance": 40 }` | Hold, move the pointer at least `distance` pixels and let go: the part for the direction moved most. `tap`: let go without moving that far. Each part is optional and is any action except a `tap_hold` or `gesture`. |
| `{ "type": "gyro", "toggle": false }` | Controller gyro moves the pointer while held, or (`toggle`) switch it on and off |
| `{ "type": "disabled" }` | Swallow the input |

## Macros

```json
{ "name": "Burst", "steps": [ … ], "jitter_ms": 0 }
```

`jitter_ms` (optional) makes every delay up to that much shorter or longer, at random.

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
| `{ "type": "mouse_move", "x": 40, "y": -10 }` | Move the pointer by (x, y), like the mouse would |
| `{ "type": "mouse_move", "x": 960, "y": 540, "absolute": true }` | Move the pointer to a screen position in pixels (Windows only) |

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
