# Getting started

mouse-kb-remap runs on **Windows 10 and 11**. A Linux engine is included but experimental
(see [Troubleshooting](troubleshooting.md#linux-experimental)); on other systems the UI
opens in preview mode with remapping disabled.

## Install

1. Download the latest release from the repository's
   [Releases page](https://github.com/jonathant09/mouse-kb-remap/releases/latest):
   `mouse-kb-remap-<version>-windows-x64.zip` (a folder with the app), or just
   `mouse-kb-remap.exe`.
2. Unzip it anywhere you keep apps, for example `Documents\mouse-kb-remap\`.
3. Run `mouse-kb-remap.exe`. There's nothing else to install.

The exe isn't code-signed yet, so Windows SmartScreen may show "Windows protected your
PC". Click **More info → Run anyway**. See [Troubleshooting](troubleshooting.md#smartscreen-warning).

**Updating:** quit the app from its tray icon, replace `mouse-kb-remap.exe` with the new one
and start it again. Your settings live in `%APPDATA%\mouse-kb-remap\config\`, not next to
the exe, so they carry over. If you move the exe somewhere else, open
**Settings → Start with Windows**: the app offers to point startup at the copy you're using.

## Build from source

Only needed if you want to change the app.

1. Install Rust from <https://rustup.rs> and accept the defaults. If the installer asks for
   the **Visual Studio C++ Build Tools**, let it install them (Rust uses them to link on
   Windows).
2. Clone the repository and run the launcher:

   ```powershell
   git clone https://github.com/jonathant09/mouse-kb-remap
   cd mouse-kb-remap
   .\start.bat
   ```

   The first build downloads dependencies and takes a few minutes. Later starts are instant.

| Command | What it does |
| --- | --- |
| `start.bat` | Runs the app from this folder (`target\release\`). Builds it first if needed. |
| `start.bat build` | Rebuilds, then runs it. Use this after changing or pulling code. |
| `start.bat admin` | Runs it as administrator (one UAC prompt). |
| `start.bat install` | Builds and installs the app to `Documents\mouse-kb-remap\`, or updates it there in place: the running copy is replaced and restarted the way it was running (as administrator through the startup task, with no prompt). |
## First run

When the app opens you'll see two profiles:

- **Default** applies to every app that doesn't have its own profile.
- **Genshin Impact** is an example that activates while `GenshinImpact.exe` is focused. It
  maps the side buttons to **F** (Back) and **E** (Forward).

Try it:

1. Select **Default**, click the **Back** side button on the mouse drawing, choose
   **Key**, click the box and press a key (or a combo like Ctrl + C).
2. Switch to another app, for example Notepad, and press your mouse's back button. It now
   sends that key.
3. Remaps never apply while mouse-kb-remap itself is focused, so you can't lock yourself out.

Then try a **Turbo** on a button, or record a **macro** from the sidebar: see
[Macros & turbo](macros.md).

## Keep it running

Closing the window keeps mouse-kb-remap running in the tray (notification area). To have it
start when you sign in, open **Settings → Start with Windows** and pick **On**, or
**On, as administrator** if you play games that run as admin.

> If a game runs as administrator, Windows won't let a normal app send it input. Run
> it as administrator (right-click `mouse-kb-remap.exe` → **Run as administrator**) or use the
> administrator autostart. See
> [Troubleshooting](troubleshooting.md#remaps-dont-work-in-a-game).

Next: the [user guide](user-guide.md).
