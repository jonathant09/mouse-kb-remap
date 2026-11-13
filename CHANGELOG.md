# Changelog

## 0.3.0

- **Tap / hold.** One button or key can do one thing on a quick press, another when held, and
  a third on a double tap: Caps Lock as Esc on a tap and Ctrl when held, Back as Back on a
  tap and a layer when held. Pressing another key while it's held counts as holding it, so
  it works as a modifier however fast you type.
- **More layers.** Besides the shift layer, every profile has layers 2 and 3. A *Layer*
  button can hold a layer or toggle it on and off.
- **Combos as triggers.** Bind *Ctrl + Back*, *Right click + Left click* or *Shift + Wheel up*
  to something other than the button alone. When a combo sends a key, the held modifiers are
  let go while it's sent.
- **Hotkeys.** Keys to pause / resume remapping, switch to the next profile and go back to
  the automatic profile. They work in any app, even while paused.
- **Command line.** `--pause`, `--resume`, `--toggle-pause`, `--profile "<name>"`,
  `--next-profile`, `--auto-profile` and `--status` control the running app from scripts,
  Stream Deck or AutoHotkey, also when it runs as administrator.
- **System actions.** Lock the PC, mute the microphone, switch to the next audio output,
  minimize / maximize / close the focused window, keep it on top, move it to the next monitor,
  or switch profile, from any button or key.
- **Better profile matching.** Besides exe names: wildcards (`*launcher*.exe`), full paths and
  window titles (`title:YouTube`), the most specific match winning, and *Only when
  fullscreen* for a profile that shouldn't apply to a game's launcher.
- **Mouse moves in macros.** *Move by* moves the pointer like the mouse would; *Move to* goes to
  a screen position (and can read it from the pointer). *Randomize ±* varies every delay a
  little each time a macro plays.
- **Undo and redo** for bindings, profiles and macros: buttons in the status bar, Ctrl + Z and
  Ctrl + Y.
- **On-screen notifications** (off unless turned on in Settings): a short note when the profile
  changes, remapping pauses, or a toggle (turbo, macro, layer, microphone) goes on or off.
- **Updates from the app.** *Settings → Updates* checks for a new version (automatically once a
  day if you like), then downloads it, checks its size, checksum and that it runs, swaps it in
  and restarts, keeping admin rights and settings.
- `--help` lists the command-line options; `--version` now prints in a console too.

## 0.2.0

- **Download and run.** Releases are a single `mouse-kb-remap.exe` (or a zip of a folder
  holding it) that runs on Windows 10 and 11 without installing anything else.
- **Macros.** Record keys, mouse buttons and the pauses between them, or build a macro step by
  step (key press, key down/up, mouse click, delay, text). Bind it to any button or key to play
  once, a set number of times, while held, or toggled on and off.
- **Turbo and repeat on hold.** Rapid-fire any key or mouse button with your own press interval
  and hold time, while held or toggled, with an optional "start repeating after" delay that works
  like keyboard auto-repeat.
- **More things a button can do:** key combos (Ctrl + C, Alt + Tab…) with common-shortcut presets,
  media and volume keys, typing text, launching a program, file or web address, a shift layer
  (hold one button for a second set of bindings, like G-Shift or Hypershift), and a sensitivity
  "sniper" button.
- **Wheel tilt, the number pad, F13–F24 and media keys** can be remapped. *Detect key* selects a
  key that isn't on the drawing by pressing it.
- **Tray icon.** Closing the window keeps remapping running in the tray; right-click it to pause
  or quit.
- **Start with Windows**, normally or as administrator (for games that run as admin, set up with
  one admin prompt).
- **Import and export profiles**, together with the macros they use.
- **New app icon.**
- **Lower latency.** The app only watches the mouse while a profile remaps a mouse button, and its
  input thread runs at high priority. Measured: about 0.025 ms added to a typical input.
- Fixed: holding a remapped key while switching apps could leave a key stuck down.
- Fixed: key capture missed Win, PrintScreen, Alt + Tab, the number pad and F13–F24.
- Fixed: starting the app twice ran two copies.
- Fixed: the window could open partly off-screen at high display scaling.

## 0.1.0

- First prototype: per-app profiles, remapping mouse buttons and keys to keys or mouse buttons,
  light and dark themes.
