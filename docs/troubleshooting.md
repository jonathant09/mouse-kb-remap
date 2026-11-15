# Troubleshooting

## Remaps don't work in a game

Work through these in order:

1. **Is the right profile active?** Look at the status bar while the game is focused (or
   just after switching back). *Focused* should show the game's exe and *Active profile*
   your game profile. If it says *Default*, add the exact exe name shown under *Focused*
   to the profile.
2. **Does the game run as administrator?** If the status bar shows the yellow
   *"Focused app runs as admin"* warning, Windows' UIPI is blocking a non-admin app from
   seeing or sending input to it. Run mouse-kb-remap as administrator (right-click the exe →
   **Run as administrator**), or set
   *Settings → Start with Windows → On, as administrator*. Many games with kernel
   anti-cheat run elevated, Genshin Impact included.
3. **Is remapping paused?** The dot at the bottom left should be green (*Remapping on*).
4. **Is the button visible to Windows at all?** Buttons handled in mouse firmware (DPI
   buttons, profile switches, some "sniper" buttons) never reach Windows. Test the button
   in a browser: if it does nothing anywhere, software can't see it.
5. **Is another remapper running?** See below.

## Other remappers (G HUB, Synapse, AutoHotkey…)

Logitech G HUB, Razer Synapse, AutoHotkey, X-Mouse Button Control, some games and overlays
install their own input hooks. Windows calls the most recently started one first; if it
rewrites or swallows an input, or doesn't pass it on properly, mouse-kb-remap may never see
it. If remaps stop working while one of those is running:

- disable the conflicting feature in the other program (for example the button's
  assignment in G HUB / Synapse should be its default), or
- quit the other program, or restart mouse-kb-remap after it.

Vendor software is still useful for things only it can do (DPI, lighting, onboard memory);
just leave the buttons you remap here on their default function there.

## Turbo or macro presses don't register in a game

Games that check key state once per frame can miss very short presses. Raise **Hold each
press for** (turbo) or the hold time of key-press steps to 15–30 ms, and keep the turbo
interval at least twice the hold time. Text steps are typed as Unicode, which many games
ignore; use key-press steps for in-game typing.

## The sensitivity button does nothing in a game

On Windows it changes the system pointer speed, which games reading raw mouse input (most
shooters) ignore. It works on the desktop and in games that use the Windows cursor.

## A key stays stuck down

Shouldn't happen: releases always go to the key that was pressed, and macros release
everything they pressed when they stop. If it does (for example after a crash), press and
release the stuck key physically, and please open an issue describing what you were doing.

Bringing the mouse-kb-remap window to the front stops every running macro and turbo.

## Does it add input latency?

Barely. Inputs you haven't remapped pass through in microseconds; remapped ones add one
extra trip through Windows' input queue (well under a millisecond). Mouse movement is never
changed, and the app only watches the mouse at all while the active profile remaps a mouse
button or the wheel: a profile that only remaps keys leaves the mouse completely alone. The
input thread runs at high priority so a busy PC doesn't delay it.

A binding [for one device](user-guide.md#devices) costs a little more: that button is held
back until Windows says which mouse sent it, about 0.3 ms in testing (0.2 ms more than a
plain remap). Buttons without per-device bindings are not affected.

## Controller buttons do nothing

- It only reads controllers while the active profile remaps a controller input or moves the
  pointer with one. `mouse-kb-remap.log` then says *controller: … connected (XInput)* or
  *controller: reading <name>*.
- Check *Settings → Controllers*: *Xbox-style controllers* covers Xbox and XInput pads,
  *PlayStation, Switch, Steam and other controllers* everything else.
- An unknown controller's buttons are numbered: use **Detect button** on the Controller tab
  and press the button to find which one it is.
- If a **Switch** controller does nothing over Bluetooth, unpair and pair it again: it has to
  accept the app's request for full reports.
- **The original Steam Controller (2015)** isn't read directly; with Steam running, its
  desktop layout can send keys the app remaps. **Switch 2** controllers aren't supported yet.
- A game still sees the original buttons as well. To change what a game sees, remap the
  controller onto the [virtual controller](user-guide.md#virtual-controller) and turn on
  *Hide the real controllers from games meanwhile*.

## Controller buttons count twice

Something else presents the same controller as an Xbox controller too: DS4Windows,
BetterJoy, DSX, or Steam Input. Steam's copies are skipped by default; for the others, turn
off one of the two sources in *Settings → Controllers* (or hide the real controller in that
tool's settings, e.g. HidHide for DS4Windows, and keep *Xbox-style controllers* on).

## The virtual controller does nothing in a game

- **Is it plugged in?** Select an input with the Controller action: the box under the list
  says *plugged in, as controller N*. It's plugged in only while the active profile has a
  Controller binding (or a turbo of a controller button), so check the status bar shows that
  profile while the game is focused.
- **Driver missing:** use *Install controller driver*, or install ViGEmBus yourself. If the
  installer finished but the app still says it's missing, restart Windows.
- **The game reads only controller 1** and your real controller is controller 1: turn on
  *Hide the real controllers from games meanwhile* (see
  [Hiding the real controller](user-guide.md#hiding-the-real-controller-hidhide)), or
  connect it after the virtual one is plugged in.
- **The game ignores it:** some games, and some anti-cheats, refuse virtual controllers.
  Nothing on the app's side can change that.
- **Steam** may take over the virtual controller like any Xbox controller (Steam Input):
  set the game's *Steam Input* to off if the controller acts strangely in a Steam game.

## "Get an app to open this 'ms-gamebar' link" when the virtual controller appears

Windows tries to open the Xbox Game Bar a few seconds after any Xbox controller connects,
the virtual one included. If the Game Bar was removed from Windows, it asks for an app to
open the link instead. It isn't the app's doing and happens with a real Xbox controller
too. Fixes: install *Xbox Game Bar* again from the Microsoft Store (it then stays out of
the way unless you open it), or keep the virtual controller plugged in longer by using it
from one profile that stays active (it's only plugged in when a profile uses it).

## Remapped controller buttons still do their old thing in a game

The game is also reading your real controller. Turn on *Settings → Virtual controller →
Hide the real controllers from games meanwhile* (Windows installs HidHide for it), so only
the virtual controller (your controller with its remaps) reaches games. See
[Hiding the real controller](user-guide.md#hiding-the-real-controller-hidhide).

- **Still there?** Hiding applies when a program opens the controller: restart the game,
  or unplug the controller and plug it in again, after the virtual one is plugged in.
- The box under the switch says how many devices are hidden, or why it couldn't hide them
  (HidHide's *inverse* mode on, for instance).
- **The controller stays hidden after the app was closed by force** (Task Manager, a
  crash): start the app again and it gives the controller back. Or open the *HidHide
  Configuration Client* and untick the controller.

## Mouse → stick moves too little, too much, or drifts

- Change *Speed* in *Settings → Virtual controller → Mouse → stick*: higher leans the stick
  further for the same mouse movement.
- **Small movements do nothing in the game:** raise *Stick starts at* until they do (the
  game's own dead zone is bigger). **The view creeps on its own:** lower it.
- **The pointer still moves:** *Keep the pointer still meanwhile* only works while the
  virtual controller is plugged in and the profile's app is in front.

## Installing the controller driver fails

This is about *Install controller driver* (ViGEmBus) and *Install HidHide* alike. The
message says why. *The admin prompt was declined* or *the installer was cancelled*: nothing
was installed, try again (Windows closes an admin prompt nobody answers after a while).
*The download isn't the expected installer*: the file from GitHub didn't match the
checksum built into the app (a broken download or a changed file), so it wasn't run. You
can always install them by hand from their release pages:
[ViGEmBus](https://github.com/nefarius/ViGEmBus/releases),
[HidHide](https://github.com/nefarius/HidHide/releases).

## Gyro or touchpad does nothing, or the gyro drifts

- **Gyro** needs a PlayStation, Switch or Steam controller (Xbox controllers have none) and
  either *Gyro: On* for the profile or a button bound to the **Gyro** action held down.
- Let the controller lie still for a second: the app learns the gyro's resting drift then.
  If it moves the wrong way, use the invert switches in *Settings → Controllers*.
- On Bluetooth, a PlayStation controller sends motion and touchpad only in its full report
  mode, which the app requests when a profile uses them; reconnect it if it was connected
  before the app started.
- A Steam Deck or Steam Controller moves the mouse with its own trackpad too unless Steam is
  running or *Turn off … own mouse mode* is on.

## A per-device binding doesn't apply

- **Keys:** on Windows a key can't have a binding for one keyboard (Windows doesn't say
  which keyboard a key came from). Bind it under *All devices*.
- **The mouse was re-added, or plugged into another port:** the app matches it by vendor and
  product too, so this normally keeps working. If you have two identical mice, add each one
  again after moving it.
- **Some buttons work for the device, others don't:** a few mice report some buttons through
  a second interface. The app treats interfaces with the same vendor and product as one
  device; if a button still isn't recognised, *Add a device* again with that button and
  give its bindings to the new entry.
- Buttons from other devices are sent on as simulated input; a few games ignore simulated
  clicks. Bind the button per device only where you need to.

## A gesture does nothing in a game

Gestures follow the pointer on screen. Games that keep the pointer in the middle of the
screen (most first-person games) never let it move, so every gesture counts as *No move*.

## Remapping stops working after a while

Windows silently removes low-level hooks that respond too slowly, for example when the
system is under extreme load. Restart the app. If it happens repeatedly, open an issue with
your Windows version, what was running, and `mouse-kb-remap.log` from the config folder.

## A tap / hold key types its tap late, or holds when I meant to tap

A tap is sent when you let go, so a tap / hold key sends its tap a moment later than a plain
key, and with a double-tap part, after the double-tap time too. If you roll onto the next
key before letting go, that counts as holding (that's what makes *Caps Lock: hold for Ctrl*
work when typed quickly). Tap / hold suits keys you don't type fast through: Caps Lock,
side buttons, F-keys. Lower *Held this long counts as a hold* if holds feel slow to start.

## Notifications don't show in a game

Games in exclusive fullscreen draw over every other window, notifications included.
Switch the game to borderless or windowed fullscreen to see them. They're off unless turned
on in *Settings → Notifications*.

## A window rule (title or fullscreen) doesn't switch the profile

Titles and fullscreen are checked twice a second, so a profile can take up to half a second
to follow. The status bar shows which profile is in use; if it says *chosen by hand*, a
hotkey or the command line picked it, and clicking that goes back to automatic. A window
counts as fullscreen when it covers its whole monitor, including the taskbar.

## "Next audio output" or window actions do nothing

*Next audio output* needs at least two enabled playback devices (Sound settings). Window
actions can't touch windows of programs running as administrator unless mouse-kb-remap runs
as administrator too, and leave the desktop and taskbar alone.

## Updating from the app fails

The message says why. *Can't write to …*: the app's folder isn't writable (e.g. under
Program Files); download the new version from the release page. *GitHub is limiting
requests*: try again later. A download that doesn't match the release (size, checksum, or
the version it reports) is thrown away and nothing changes. If a new version won't start
after updating, `mouse-kb-remap.old.exe` next to it is the previous version for the first
minute: rename it back to `mouse-kb-remap.exe`.

## The command line does nothing in PowerShell

The app is a windowed program, so PowerShell and cmd don't wait for it and its answer can
arrive after the prompt. Pipe it (`mouse-kb-remap.exe --status | Write-Output`) or use
`start /wait` in cmd to wait for the answer and the exit code. Exit code 2 means no copy is
running; copies started with a different `MOUSE_KB_REMAP_CONFIG` are separate and need the
same variable set.

## "mouse-kb-remap is already running as administrator"

Only one copy runs at a time; starting it again opens the running one. Versions before 0.2.0
couldn't be reached that way when they ran as administrator and showed this message instead:
open the old copy from its tray icon and quit it there, then start the new one.

## Anti-cheat and game rules

mouse-kb-remap uses standard, documented Windows APIs (low-level hooks and `SendInput`),
the same mechanism as AutoHotkey and X-Mouse Button Control. It touches no game memory and
installs no driver of its own; the virtual controller uses the ViGEmBus driver, installed
only when you ask, which some anti-cheats distrust.

Still, many games forbid macros, turbo or remapping software in their terms of service,
and some anti-cheats flag or ignore synthetic input. One-to-one remaps (side button → key)
are rarely an issue; turbo and macros are much more likely to be against the rules. It's
your responsibility to check the rules of the games you play, especially competitive ones.

## SmartScreen warning

Release builds aren't code-signed yet, so the first launch may show *"Windows protected
your PC"*. Click **More info → Run anyway**. You can also build from source
([Getting started](getting-started.md#option-b-build-from-source)) and run your own exe.

## Parts of the window show what's behind it

Fixed in 0.4.0: the window is repainted completely when it's activated, deactivated,
uncovered or opened from the tray. If it still happens, resizing the window repaints it;
please report what you did just before (and whether the app runs as administrator).

## My settings disappeared

If `config.json` can't be parsed (usually a hand-edit typo), the app starts with defaults
and keeps your file as `config.broken.json` next to it, in
`%APPDATA%\mouse-kb-remap\config\`. Fix the JSON, rename it back to `config.json` and
restart. *Settings → Configuration file → Open folder* takes you there.

## Linux (experimental)

The Linux engine reads keyboards and mice from `/dev/input` and sends output through a
virtual device, so it needs access to both:

```bash
sudo usermod -aG input "$USER"
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/99-mouse-kb-remap.rules
sudo modprobe uinput
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then log out and back in. `mouse-kb-remap.log` (in `~/.config/mouse-kb-remap/`) lists the
devices it grabbed, or why it couldn't.

- **Per-app profiles** follow the focused window on **sway**, **Hyprland** and **niri**
  (through their own IPC, with window titles and, on sway and Hyprland, fullscreen) and on
  X11. GNOME and KDE Plasma on Wayland don't tell other programs which window is in front,
  so there only programs running through XWayland are seen; elsewhere the Default profile
  applies. The log says which one the app uses ("following the focused window through
  sway").
- **Virtual controller**: needs the same `/dev/uinput` access. Games and Steam see it as an
  Xbox 360 controller. Hiding the real controller takes it with an evdev grab, which hidraw
  readers (Steam, some SDL games) don't respect.
- **Text** actions type printable ASCII only (US layout).
- Touchpads and tablets are left alone. Game controllers are read (not grabbed) for
  controller bindings; some drivers swap the X and Y buttons.

## `git` says "detected dubious ownership"

This can happen when the repository folder was created by a different user account or tool.
Run the command git suggests:

```powershell
git config --global --add safe.directory C:/path/to/mouse-kb-remap
```

## Build errors

- *"linker `link.exe` not found"*: install the Visual Studio C++ Build Tools (re-run
  `rustup-init.exe` and accept the prompt, or install "Desktop development with C++").
- *"could not compile slint"* after updating Rust: run `cargo clean`, then build again.
- *"could not embed the exe icon"* (warning): the Windows SDK's resource compiler wasn't
  found. The app still builds, just without an icon in Explorer. Installing "Desktop
  development with C++" includes it.
