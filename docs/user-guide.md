# User guide

![Main window](screenshot-dark.png)

The window has four areas:

| Area | Purpose |
| --- | --- |
| **Sidebar** (left) | Profiles, macros, Settings and the theme switch |
| **Header** (top) | Profile name, which apps activate it, Mouse / Keyboard, Base / Shift layer |
| **Canvas** (center) | A generic mouse or keyboard. Click any button or key to select it. |
| **Inspector** (right) | What the selected input does, or a list of all bindings in the profile |

A **status bar** along the bottom shows whether remapping is on, which app is focused and
which profile is active.

## Profiles

A profile is a set of bindings. Exactly one profile is active at a time, picked from the
window you're using right now:

1. If the focused window's program (for example `GenshinImpact.exe`) is listed in a
   profile's **Activates for** list, that profile is active.
2. Otherwise **Default** is active.

The active profile is marked **LIVE** in the sidebar and shown in the status bar.

### Inheritance

Bindings in **Default** also apply inside every other profile, unless that profile binds
the same input itself:

| Input | Default | Genshin Impact | Result in Genshin |
| --- | --- | --- | --- |
| Back | `Page Down` | `F` | `F` (the profile's own binding wins) |
| Forward | `Left Ctrl` | *(not bound)* | `Left Ctrl` (inherited) |

Unbound inputs show the inherited value, e.g. "Left Ctrl (Default)".

### Creating, copying and deleting

- **+** next to *PROFILES* adds one. Click the name in the header to rename it.
- Add the programs it's for: type the exe name (`eldenring.exe`) and press **Enter**
  (`.exe` is added if you leave it out), or switch to the game once, come back and click it
  under **Recent**. Matching uses the exe file name only and ignores capitalization.
- **Duplicate** copies the profile with all its bindings (the copy starts with no apps, so
  the two don't fight over the same game).
- **Delete** removes it (Default can't be deleted). Remove an app with the **×** on its chip.

## Remapping an input

1. Choose **Mouse** or **Keyboard** in the header.
2. Click a button or key on the drawing (or its callout box).
3. Pick an **Action** in the inspector. Changes save and apply immediately.

| Action | Effect |
| --- | --- |
| **Default** | No binding in this profile/layer: passes through, or inherits |
| **Key** | Sends a key or combo. Click the box and press it (Ctrl + C, Alt + Tab, Win…), or use the Ctrl/Shift/Alt/Win toggles and the *Common shortcuts* presets. Held as long as the input is held. |
| **Mouse** | Sends a mouse button, a scroll step or a horizontal scroll step |
| **Macro** | Plays a macro: once, repeat, while held or toggle. See [Macros & turbo](macros.md). |
| **Turbo** | Repeats a key or button with your timing, while held or toggled. See [Macros & turbo](macros.md#turbo). |
| **Text** | Types a piece of text |
| **Launch** | Starts a program, or opens a file, folder or `https://` address |
| **Media** | Play/pause, next/previous track, volume, mute, browser and launcher keys |
| **Shift layer** | Hold to switch every other input to the shift layer (below) |
| **Sensitivity** | Hold to change the pointer speed (a "sniper" / DPI-shift button) |
| **Off** | The input is swallowed and does nothing |

Remapped inputs are outlined in the accent color. On the keyboard, the new key is printed in
the bottom-right corner of the keycap.

### Keys that aren't on the drawing

Switch on **Number pad** for a full-size layout. For anything else (F13–F24, media keys,
the extra keys some mice send), click **Detect key** and press it: the key gets selected
and you can remap it like any other. It shows up in *All bindings*.

### Inputs that can't be remapped

The **DPI button** is drawn for reference only. On almost every mouse it's handled by the
mouse's firmware and never reaches the PC, so no software (other than the vendor's) can see
it. The same is true of some extra buttons on certain mice.

### Holding, repeating and scrolling

A remapped key is held for as long as you hold the input, so Back mapped to `W` walks
forward. Holding a remapped keyboard key auto-repeats the new key, like a real one. If you
switch windows while holding, releasing still releases the right key.

Scroll steps and wheel tilt are single steps. Mapping them to a key presses that key
briefly (15 ms) per notch, so games that check key state per frame notice it.

## Shift layer

Every profile has a second set of bindings, the **shift layer**, like G-Shift on Logitech
or Hypershift on Razer:

1. Bind an input (e.g. *Back*) to **Shift layer** on the **Base** layer.
2. Switch the header to **Shift layer** and bind other inputs there.
3. While you hold *Back*, the shift-layer bindings apply. Inputs with nothing on the shift
   layer keep their normal binding.

The Default profile's shift layer is inherited like its base layer.

## Sensitivity

Bind an input to **Sensitivity** and pick a pointer speed (1–20; Windows' default is 10).
While held, the pointer moves at that speed; on release your own speed comes back. The
change is never saved to Windows' settings, so a crash or reboot also restores it.

Games that read raw mouse input (most shooters) ignore the Windows pointer speed; use the
game's own sensitivity or your mouse's DPI button there. On Linux the clutch scales the
mouse movement itself, so it works everywhere.

## Status bar

| Item | Meaning |
| --- | --- |
| ● **Remapping on / paused** | Click to pause or resume all remapping |
| **Focused** | The last program you used other than this one |
| **Active profile** | The profile applied to that program |
| Yellow warning | The focused program runs as admin and this app doesn't, so Windows is blocking remaps. Run this app as administrator (right-click the exe → **Run as administrator**). |
| **Admin / Standard user** | Whether this app itself is elevated |

## Tray

Closing the window keeps mouse-kb-remap running in the notification area (tray), so
remapping keeps working. Click the tray icon to open the window; right-click it to pause or
resume remapping, or **Quit**. Turn this off in *Settings → Window* if you'd rather have the
close button quit.

Starting the app a second time opens the running copy instead.

## Settings

| Setting | |
| --- | --- |
| **Start with Windows** | *On* starts hidden in the tray at sign-in. *On, as administrator* does the same with admin rights (Task Scheduler, one admin prompt to set up), for games that run as admin. |
| **Window** | Keep running in the tray when the window is closed |
| **Profiles** | Import, export the current profile, or export all. Exports include the macros the profiles use. Imported profiles are added next to yours (names made unique). |
| **Configuration file** | Where settings live, with a button to open the folder |

## Safety

- Remaps never apply while this window is focused, so you can always click your way out of
  a bad binding.
- Bringing this window to the front also stops any running macro or turbo.
- Settings are saved as you change them; a config file with a typo is kept as
  `config.broken.json` instead of being lost.

## Themes

Switch **Dark / Light** at the bottom of the sidebar. The choice is saved.
