# User guide

![Main window](screenshot-dark.png)

The window has four areas:

| Area | Purpose |
| --- | --- |
| **Sidebar** (left) | Profiles, macros, Settings and the theme switch |
| **Header** (top) | Profile name, which apps activate it, Mouse / Keyboard / Controller, the binding layer (Base / Shift / Layer 2 / Layer 3) |
| **Canvas** (center) | A generic mouse, keyboard or game controller. Click any button or key to select it. Once you've named devices, *Bindings for* at its top picks whose bindings you see. |
| **Inspector** (right) | What the selected input does, or a list of all bindings in the profile |

A **status bar** along the bottom shows whether remapping is on, which app is focused and
which profile is active.

## Profiles

A profile is a set of bindings. Exactly one profile is active at a time, picked from the
window you're using right now:

1. If the focused window matches an entry in a profile's **Activates for** list, that
   profile is active.
2. Otherwise **Default** is active.

The active profile is marked **LIVE** in the sidebar and shown in the status bar. You can
also pick one by hand with a [hotkey](#hotkeys), a [System action](#system-actions) or the
[command line](#command-line); the status bar then says *chosen by hand · back to
automatic* (click it to go back).

### What activates a profile

| Entry | Matches |
| --- | --- |
| `GenshinImpact.exe` | The program's file name |
| `*launcher*.exe` | File names with wildcards (`*` any text, `?` one character) |
| `C:\Games\*\game.exe` | The program's full path (anything with `\` or `/`), wildcards allowed |
| `title:YouTube` | Windows whose title contains the text (or matches it, with wildcards) |

Capitalization is ignored. Titles are re-checked twice a second, so a browser profile can
follow the tab you're on. When several profiles match, the most specific entry wins (a title
over a path over a file name), then the one higher in the list.

**Only when fullscreen** (switch in the header) makes the profile apply only while the
matching window covers its whole screen, e.g. a game but not its launcher or windowed menus.

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
- Add the programs it's for: type the exe name (`eldenring.exe`), a path or `title:…` and
  press **Enter** (`.exe` is added to a plain name), or switch to the game once, come back
  and click it under **Recent**.
- **Duplicate** copies the profile with all its bindings (the copy starts with no apps, so
  the two don't fight over the same game).
- **Delete** removes it (Default can't be deleted). Remove an app with the **×** on its chip.

## Remapping an input

1. Choose **Mouse**, **Keyboard** or **Controller** in the header.
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
| **Layer** | Hold (or toggle) to use another set of bindings: the shift layer, layer 2 or 3 ([Layers](#layers)) |
| **Sensitivity** | Hold to change the pointer speed (a "sniper" / DPI-shift button) |
| **Tap / hold** | One action on a quick press, another when held, optionally a third on a double tap ([Tap / hold](#tap--hold)) |
| **System** | Lock the PC, mute the microphone, switch audio output, act on the focused window, switch profile ([System actions](#system-actions)) |
| **Gesture** | Hold and move the mouse up, down, left or right, then let go: a different action per direction ([Gestures](#gestures)) |
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

## Layers

Every profile has three more sets of bindings on top of **Base**: **Shift** (like G-Shift on
Logitech or Hypershift on Razer), **Layer 2** and **Layer 3**.

1. Bind an input (e.g. *Back*) to **Layer** on the **Base** layer and pick which layer.
2. Switch the header to that layer and bind other inputs there.
3. While you hold *Back*, that layer's bindings apply. Inputs with nothing on the layer keep
   their normal binding.

Choose **Toggle on / off** instead of *While held* to lock a layer on with one press and off
with the next (handy for a "game mode" or a numpad layer). Toggled layers go off when the
profile changes. When several layers are on, the highest one wins for each input. The
banner on the canvas says which inputs switch to the layer you're looking at.

The Default profile's layers are inherited like its base layer.

## Tap / hold

**Tap / hold** gives one input up to three jobs:

| Part | When |
| --- | --- |
| **Tap** | Pressed and released quickly |
| **Hold** | Held for longer than *Held this long counts as a hold* (200 ms by default) |
| **Double tap** | Tapped twice within *Second tap within* (250 ms) |

Click **Edit** next to a part to choose what it does (any action except another tap /
hold); **← Tap / hold** goes back. Choosing Tap / hold for an input that already does
something keeps that as its tap.

Pressing another key or button while a tap / hold input is down counts as holding it right
away, so it works as a modifier: *Caps Lock: tap Esc, hold Ctrl* gives Ctrl + C when you
press C with Caps Lock held, however quickly. With a double-tap part, a single tap is sent
once the double-tap time has passed (that small wait is the price of telling them apart).
Holding a layer is a good hold part: tap for a key, hold for a layer.

## Combos

A combo binds what an input does while other keys or buttons are held: *Ctrl + Back*,
*Right click + Left click*, *Shift + Wheel up*.

1. Select the input you'll press last (e.g. *Back*).
2. Under **Combos**, click **+ Add combo**. It starts as *Ctrl + Back*.
3. Under **Hold**, pick the modifiers (Ctrl, Shift, Alt, Win: either side counts), and
   optionally a mouse button or a key to hold as well.
4. Choose the action as usual.

The combo list under each input shows its combos; *All bindings* lists them too. A combo
wins over the input's plain binding on the same layer, and the combo with the most held
keys wins over a shorter one.

The keys and buttons you hold still reach the app, so a combo whose action is a macro runs
with them held. When the action is a **Key**, the held modifiers are let go while it's sent,
so *Ctrl + Back → F* arrives as F, not Ctrl + F.

## Gestures

**Gesture** turns a held button into five: hold it, move the mouse at least *Move at least*
pixels (40 by default) in a direction, and let go.

| Part | When |
| --- | --- |
| **↑ Up**, **↓ Down**, **← Left**, **→ Right** | Let go after moving mostly in that direction |
| **No move** | Let go without moving that far |

Click **Edit** next to a part to choose what it does (any action except a tap / hold or
another gesture); **← Gesture** goes back. Choosing Gesture for a button keeps what it did
as its *No move* part, so a gesture on the right button still right-clicks when you don't
move. The button itself never reaches the app while it's a gesture; other buttons work as
usual while you hold it.

Gestures follow the pointer on screen, so they don't work in games that hold the pointer in
the middle of the screen.

## Controllers

The **Controller** tab shows an Xbox-style controller. Every button, both triggers, the d-pad
and each stick pushed up, down, left or right (past half way) can be remapped like a mouse
button: to keys, mouse buttons, macros, layers, tap / hold and so on. Combos work too, e.g.
*Ctrl + A*.

**Pointer moves with** (top right) lets the left or right stick move the mouse pointer while
this profile is active, faster the further you push it; **Speed** is 1–20. Other profiles
use Default's choice unless they pick their own (*Same as Default*), so a game profile can
turn it off.

On Windows this works with controllers that use XInput: Xbox controllers and most PC
gamepads (PlayStation controllers through DS4Windows or a similar tool). The app reads the
controller only while the active profile uses it.

Remapping can't hide a controller's buttons from games: a game that reads the controller
still sees them. Remap buttons the game doesn't use, or use the controller to drive other
apps (a media PC, a browser, a second screen while gaming).

## Devices

By default a binding applies to every mouse, keyboard and controller. To give one device
its own bindings (a second mouse as a macro pad, a trackball that scrolls instead):

1. In **Settings → Devices**, click **Add a device** and press a button or turn the wheel on
   it. It's added with the name it reports; click the name to rename it.
2. In a profile, pick it under **Bindings for** at the top of the canvas and bind its
   buttons as usual. Labels then read *Back (Trackball)*.

A device's own binding wins over the one for all devices (also over one inherited from
Default); its unbound buttons do what they do on every device. Layers, combos (the device is
the one the last button is pressed on), tap / hold and gestures all work per device.
Removing a device removes its bindings too.

On **Windows**, mice and controllers can have their own bindings, keyboards can't: Windows
doesn't tell programs like this one which keyboard a key came from. (On Linux every device
is read separately, so keyboards can.) A button that has a binding for one device is held
back for a fraction of a millisecond (0.3 ms median in testing) until Windows says which
mouse it came from; if it came from another one it is sent on unchanged, as a simulated
click, which a few games ignore. Buttons without per-device bindings are never held back.

## System actions

| Action | |
| --- | --- |
| **Lock PC** | Locks Windows (like Win + L) |
| **Mute microphone** | Mutes or unmutes the default microphone (and the default communications microphone) |
| **Next audio output** | Makes the next playback device the default (speakers → headset → …) |
| **Minimize window**, **Maximize / restore**, **Close window** | The focused window |
| **Always on top** | Keeps the focused window above others, or stops |
| **Move to next monitor** | Moves the focused window to the next screen, left to right |
| **Next profile** | Uses the next profile in the list whatever app is focused |
| **Automatic profile** | Goes back to picking the profile by the focused app |

Windows of programs running as administrator can only be moved or closed when this app
runs as administrator too. With notifications on, mute, audio output and always-on-top say
what they did.

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
| **chosen by hand · back to automatic** | The profile was picked by a hotkey, binding or the command line; click to pick by app again |
| **Undo / Redo** | Undo or redo changes to bindings, profiles and macros (also **Ctrl + Z** / **Ctrl + Y**) |
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
| **Hotkeys** | Keys that control the app itself ([below](#hotkeys)) |
| **Devices** | Mice and controllers with names, for bindings of their own ([Devices](#devices)) |
| **Notifications** | On-screen notifications, off unless you turn them on ([below](#notifications)) |
| **Updates** | Check for a new version, install it, check automatically ([below](#updates)) |
| **Command line** | The flags for scripts ([below](#command-line)) |
| **Profiles** | Import, export the current profile, or export all. Exports include the macros the profiles use. Imported profiles are added next to yours (names made unique). |
| **Configuration file** | Where settings live, with a button to open the folder |

### Hotkeys

Three keys or combos that control the app: **Pause / resume remapping**, **Next profile**
and **Back to automatic profile**. Click the box and press the combination (e.g.
Ctrl + Alt + F12). They work in any app, even while remapping is paused (so you can always
resume), and aren't passed on. **Clear** removes one.

### Notifications

When on, a small box near the bottom of the screen says when the profile changes, remapping
is paused or resumed, or a toggle goes on or off (toggled turbo and macros, toggled layers,
microphone mute, always on top), and shows the new audio output. It never takes focus and
lets clicks through. Games in exclusive fullscreen draw over it; borderless and windowed
games show it. **Show an example** shows one.

### Updates

**Check for updates** asks GitHub for the latest release. When there's a newer one,
**Update to … and restart** downloads it, checks it (size, checksum, and that it runs and
reports the right version), puts it in place of the running exe and restarts the app, with
the same admin rights it had. Settings are kept. The previous exe stays next to it as
`mouse-kb-remap.old.exe` for a minute; if the new version won't start, rename that back.

The check reads the releases of the GitHub repository without signing in, so it only finds
releases of a public repository.

**Check automatically** (off unless you turn it on) looks once when the app starts and once
a day, and shows *Update available* in the sidebar. Nothing is installed without your
click. If the app's folder isn't writable (e.g. under Program Files), download the new
version from the release page instead.

### Command line

The running app can be controlled from scripts, Stream Deck, AutoHotkey or a shortcut:

| Flag | |
| --- | --- |
| `--pause`, `--resume`, `--toggle-pause` | Pause or resume remapping |
| `--profile "<name>"` | Use that profile whatever app is focused |
| `--next-profile` | Use the next profile |
| `--auto-profile` | Pick the profile by the focused app again |
| `--status` | Print whether remapping is on, the profile in use and the focused app |
| `--quit` | Quit the running copy |
| `--help`, `--version` | |

The answer is printed (`ok: …` or `error: …`), and the exit code is 0 when it worked, 1 when
the app said no (unknown profile, missing name) and 2 when the app isn't running. These
work when the app runs as administrator too. Windows doesn't wait for this (windowed) program
in a plain PowerShell prompt; use `mouse-kb-remap.exe --status | Write-Output`, or
`start /wait` in cmd, when a script needs the answer or the exit code.

## Safety

- Remaps never apply while this window is focused, so you can always click your way out of
  a bad binding.
- Bringing this window to the front also stops any running macro or turbo.
- Settings are saved as you change them; a config file with a typo is kept as
  `config.broken.json` instead of being lost.

## Themes

Switch **Dark / Light** at the bottom of the sidebar. The choice is saved.
