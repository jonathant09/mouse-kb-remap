# Changelog

## 0.5.0

- **Virtual controller** (Windows). The new *Controller* action presses a button, trigger or
  stick direction of a virtual Xbox controller that games see as a real one: play games
  that want a controller with the keyboard and mouse (WASD on the left stick, clicks on the
  triggers), add buttons from the keyboard or mouse, or remap a controller for games. It
  works in tap / hold, gestures, combos, layers and per device, and *Turbo* can rapid-fire a
  controller button. Taps are held for 50 ms so games don't miss them.
- **Remap a controller for games.** *Copy the real controller to the virtual one* (on by
  default) passes on what your controller does that the profile doesn't remap, so the
  virtual controller is your controller with its remaps. Games still see the real one too:
  hide it with HidHide (the user guide explains how).
- **Off unless used.** The virtual controller is plugged in only while the active profile
  uses it, and unplugged ten seconds after it stops (or when the app quits).
- **One-click driver install.** Virtual controllers need the free ViGEmBus driver. When it's
  missing, *Install controller driver* downloads the official installer (ViGEmBus 1.22.0),
  checks it against a checksum built into the app and runs it after Windows' admin prompt;
  never silently. ViGEmBus is no longer developed but works on Windows 10 and 11; some
  anti-cheats refuse virtual controllers (the app says so).
- New *Settings → Virtual controller* card: driver state, the install button and copying.
- Config format version 5 (older files load unchanged; 0.4 can't read a file that uses the
  Controller action).

## 0.4.0

- **Game controllers.** A new *Controller* tab maps every button, trigger, d-pad direction and
  stick direction to keys, mouse buttons, macros and every other action, with layers, combos
  and tap / hold like any other input. Supported: Xbox and XInput controllers (now with the
  Guide button), DualShock 4, DualSense and DualSense Edge (PS button, touchpad click, mic
  button, Edge paddles and Fn buttons), Switch Pro Controller and Joy-Cons (Home, Capture,
  SL / SR; two Joy-Cons act as one controller), the Steam Deck and the Steam Controller
  (2025) with their back buttons, and any other gamepad or joystick (Bluetooth phone
  controllers, 8BitDo in DirectInput mode…) as numbered buttons, which *Detect button*
  picks for you. Button names can follow Xbox, PlayStation or Nintendo.
- **Pointer from a controller**, per profile: a stick, the gyro (tilt to aim, always or
  while a *Gyro* button is held or toggled) or the touchpad (PlayStation touchpad, Steam
  trackpad). Speeds and inversion in *Settings → Controllers*.
- **Works alongside Steam and other tools**: controllers are read shared, Steam Input's
  virtual Xbox copies are skipped, and the Steam Deck's / Steam Controller's own mouse mode
  can be turned off while the app reads them. Games still see a controller's own buttons, so
  remap ones the game doesn't use or use the controller in other apps.
- **Mouse gestures.** The *Gesture* action: hold a button, move the mouse up, down, left or
  right and let go to do something different for each direction; letting go without moving
  does what the button did (a normal click by default). The distance to move is adjustable.
- **Bindings for one device.** Name a mouse or controller in *Settings → Devices* (press a
  button on it to add it), then pick it under *Bindings for* at the top of a profile: its
  bindings there win over the ones for all devices, so two mice can do different things.
  Windows can't tell keyboards apart, so keys apply to every keyboard (on Linux they can be
  per keyboard too). Buttons with a binding for one device are held back for a fraction of a
  millisecond until Windows says which device sent them (measured 0.3 ms median), and sent
  on unchanged when they came from another one.
- **Choose a key from a list.** Every "press a key" box (key bindings, turbo, combos, macro
  steps) has *Choose from a list*: pick the key from groups (letters, numbers, F1–F24,
  modifiers, number pad, media…) with the mouse, then add Ctrl, Shift, Alt or Win with the
  buttons. No working keyboard needed.
- **Fixed parts of the window showing what's behind it** after alt-tab or opening it from the
  tray (until it was resized): the whole window is now repainted when it's activated,
  deactivated, uncovered or shown.
- The window opens at its smallest size (1080 × 660); maximize it for more room. The mouse
  and controller views fit at that size: the controller lists move under the drawing and
  scroll.
- Crashes now leave a line in `mouse-kb-remap.log`.
- Config format version 4 (older files load unchanged; 0.3 can't read a file that uses the new
  actions).

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
