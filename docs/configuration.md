# Configuration

All settings live in one JSON file, written every time you change something in the UI:

```
%APPDATA%\mouse-kb-remap\config\config.json
```

(On Linux during development: `~/.config/mouse-kb-remap/config.json`.)

You can edit it by hand while the app is closed. If the file doesn't parse, the app starts
with defaults and first copies your file to `config.broken.json` in the same folder, so a
typo never loses your profiles.

## Example

```json
{
  "version": 1,
  "theme": "dark",
  "enabled": true,
  "profiles": [
    {
      "name": "Default",
      "apps": [],
      "bindings": {
        "mouse:forward": { "type": "key", "key": "LCtrl" }
      }
    },
    {
      "name": "Genshin Impact",
      "apps": ["GenshinImpact.exe"],
      "bindings": {
        "mouse:back":    { "type": "key", "key": "F" },
        "mouse:forward": { "type": "key", "key": "E" },
        "key:CapsLock":  { "type": "disabled" },
        "key:Semicolon": { "type": "mouse", "button": "middle" }
      }
    }
  ]
}
```

## Top-level fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `version` | number | `1` | Format version, for future migrations |
| `theme` | `"dark"` / `"light"` | `"dark"` | UI theme |
| `enabled` | bool | `true` | Master switch (the status-bar toggle) |
| `profiles` | array | | **The first profile is always Default** |

## Profile

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Shown in the sidebar |
| `apps` | string[] | Exe file names that activate this profile while focused. Case-insensitive; ignored on Default. |
| `bindings` | object | Map of **input id → action** |

## Input ids

Bindings are keyed by the physical input being remapped.

**Mouse:** `mouse:left`, `mouse:right`, `mouse:middle`, `mouse:back` (button 4 /
XButton1), `mouse:forward` (button 5 / XButton2), `mouse:wheel_up`, `mouse:wheel_down`

**Keyboard:** `key:<key id>`, using the key ids below, e.g. `key:A`, `key:F5`,
`key:LShift`.

## Actions

| JSON | Effect |
| --- | --- |
| `{ "type": "key", "key": "<key id>" }` | Send a key (held for as long as the input is held) |
| `{ "type": "mouse", "button": "<button>" }` | Send a mouse button or wheel step. `<button>` is one of `left`, `right`, `middle`, `back`, `forward`, `wheel_up`, `wheel_down`. |
| `{ "type": "disabled" }` | Swallow the input |

## Key ids

Key ids follow the generic ANSI layout shown in the app. Left and right modifiers are
separate keys.

| Row | Key ids |
| --- | --- |
| Function row | `Esc`, `F1`, `F2`, `F3`, `F4`, `F5`, `F6`, `F7`, `F8`, `F9`, `F10`, `F11`, `F12`, `PrintScreen`, `ScrollLock`, `Pause` |
| Number row | `Grave`, `1`, `2`, `3`, `4`, `5`, `6`, `7`, `8`, `9`, `0`, `Minus`, `Equal`, `Backspace`, `Insert`, `Home`, `PageUp` |
| Top row | `Tab`, `Q`, `W`, `E`, `R`, `T`, `Y`, `U`, `I`, `O`, `P`, `LBracket`, `RBracket`, `Backslash`, `Delete`, `End`, `PageDown` |
| Home row | `CapsLock`, `A`, `S`, `D`, `F`, `G`, `H`, `J`, `K`, `L`, `Semicolon`, `Quote`, `Enter` |
| Bottom row | `LShift`, `Z`, `X`, `C`, `V`, `B`, `N`, `M`, `Comma`, `Period`, `Slash`, `RShift`, `Up` |
| Space row | `LCtrl`, `LWin`, `LAlt`, `Space`, `RAlt`, `RWin`, `Menu`, `RCtrl`, `Left`, `Down`, `Right` |

Notes:

- `Grave` is the `` ` `` / `~` key, `Quote` is `'`, `Menu` is the context-menu key.
- Number-pad keys aren't in the layout yet.
- Unknown ids are ignored (the binding is skipped, not deleted).
