# Troubleshooting

## Remaps don't work in a game

Work through these in order:

1. **Is the right profile active?** Look at the status bar while the game is focused (or
   just after switching back). *Focused* should show the game's exe and *Active profile*
   your game profile. If it says *Default*, add the exact exe name shown under *Focused*
   to the profile.
2. **Does the game run as administrator?** If the status bar shows the yellow
   *"Focused app runs as admin"* warning, Windows' UIPI is blocking a non-admin app from
   seeing or sending input to it. Start mouse-kb-remap with `start.bat admin`. Many games
   with kernel anti-cheat run elevated, Genshin Impact included.
3. **Is remapping paused?** The dot at the bottom left should be green (*Remapping on*).
4. **Is the button visible to Windows at all?** Buttons handled in mouse firmware (DPI
   buttons, profile switches, some "sniper" buttons) never reach Windows. Test the button
   in a browser: if it does nothing anywhere, software can't see it.
5. **Is another remapper running?** G HUB, Synapse, AutoHotkey, X-Mouse Button Control and
   similar tools install their own hooks and can rewrite or swallow input before or after
   this app. Close them while testing.

## Anti-cheat and game rules

mouse-kb-remap uses standard, documented Windows APIs (low-level hooks and `SendInput`),
the same mechanism as AutoHotkey and X-Mouse Button Control. It installs no driver and
touches no game memory.

Still, some games forbid remapping or macro software in their terms of service, and some
anti-cheats flag or ignore synthetic input. One-to-one remaps (side button → key) are rarely
an issue, but it's your responsibility to check the rules of the games you play, especially
competitive ones.

## SmartScreen warning

Release builds aren't code-signed yet, so the first launch may show *"Windows protected
your PC"*. Click **More info → Run anyway**. You can also build from source
([Getting started](getting-started.md#option-b-build-from-source)) and run your own exe.

## A key stays stuck down

Shouldn't happen: releases always go to the key that was pressed. If it does (for
example after a crash), press and release the stuck key physically. Please open an issue
describing what you were doing.

## Remapping stops working after a while

Windows silently removes low-level hooks that respond too slowly, for example when the
system is under extreme load. Restart the app. If it happens repeatedly, open an issue with
your Windows version and what was running.

## My settings disappeared

If `config.json` can't be parsed (usually a hand-edit typo), the app starts with defaults
and keeps your file as `config.broken.json` next to it, in
`%APPDATA%\mouse-kb-remap\config\`. Fix the JSON, rename it back to `config.json` and
restart.

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
