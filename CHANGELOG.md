# Changelog

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
