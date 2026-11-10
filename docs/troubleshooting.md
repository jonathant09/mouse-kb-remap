# Troubleshooting

## Remaps don't work in a game

Work through these in order:

1. **Is the right profile active?** Look at the status bar while the game is focused (or
   just after switching back). *Focused* should show the game's exe and *Active profile*
   your game profile. If it says *Default*, add the exact exe name shown under *Focused*
   to the profile.
2. **Does the game run as administrator?** If the status bar shows the yellow
   *"Focused app runs as admin"* warning, Windows' UIPI is blocking a non-admin app from
   seeing or sending input to it. Start mouse-kb-remap with `start.bat admin`, or set
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

## Remapping stops working after a while

Windows silently removes low-level hooks that respond too slowly, for example when the
system is under extreme load. Restart the app. If it happens repeatedly, open an issue with
your Windows version, what was running, and `mouse-kb-remap.log` from the config folder.

## "mouse-kb-remap is already running as administrator"

Only one copy runs at a time. Starting it again opens the running copy, except when that
copy runs as administrator and the new one doesn't (Windows doesn't let it). Open it from its
tray icon, or quit it there first.

## Anti-cheat and game rules

mouse-kb-remap uses standard, documented Windows APIs (low-level hooks and `SendInput`),
the same mechanism as AutoHotkey and X-Mouse Button Control. It installs no driver and
touches no game memory.

Still, many games forbid macros, turbo or remapping software in their terms of service,
and some anti-cheats flag or ignore synthetic input. One-to-one remaps (side button → key)
are rarely an issue; turbo and macros are much more likely to be against the rules. It's
your responsibility to check the rules of the games you play, especially competitive ones.

## SmartScreen warning

Release builds aren't code-signed yet, so the first launch may show *"Windows protected
your PC"*. Click **More info → Run anyway**. You can also build from source
([Getting started](getting-started.md#option-b-build-from-source)) and run your own exe.

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

- **Per-app profiles** need X11 or XWayland: the focused window is read from
  `_NET_ACTIVE_WINDOW`. On a pure Wayland session the Default profile applies everywhere.
- **Text** actions type printable ASCII only (US layout).
- Touchpads, tablets and game controllers are left alone.

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
