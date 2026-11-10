# Getting started

mouse-kb-remap runs on **Windows 10 and 11**. A Linux engine is included but experimental
(see [Troubleshooting](troubleshooting.md#linux-experimental)); on other systems the UI
opens in preview mode with remapping disabled.

## Option A: use a prebuilt exe

1. Download `mouse-kb-remap-windows-x64.zip` from the repository's **Releases** page, or
   use the copy in the `dist\` folder if you have one.
2. Unzip it anywhere, for example `C:\Tools\mouse-kb-remap\`.
3. Double-click `start.bat`, or `mouse-kb-remap.exe` directly.

The exe isn't code-signed yet, so Windows SmartScreen may show "Windows protected your
PC". Click **More info → Run anyway**. See [Troubleshooting](troubleshooting.md#smartscreen-warning).

## Option B: build from source

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

## `start.bat`

| Command | What it does |
| --- | --- |
| `start.bat` | Runs the app. Builds it first if there's no exe yet. |
| `start.bat build` | Rebuilds from source into `dist\`, then runs it. Use this after pulling changes. |
| `start.bat admin` | Runs the app as administrator (one UAC prompt). |
| `start.bat build admin` | Both. |

It looks for the exe next to itself first (the release zip layout), then in `dist\`.

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

> If a game runs as administrator, Windows won't let a normal app send it input. Launch
> with `start.bat admin` or use the administrator autostart. See
> [Troubleshooting](troubleshooting.md#remaps-dont-work-in-a-game).

Next: the [user guide](user-guide.md).
