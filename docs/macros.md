# Macros & turbo

Two ways to make one press do more:

| | Use it for |
| --- | --- |
| **Turbo** | Pressing the same key or button over and over (rapid fire, auto-repeat) |
| **Macro** | A sequence: several keys, clicks, pauses and text |

Both run on a separate timer thread, so their timing doesn't depend on how busy the input
hook is. Measured on Windows 11, presses land within about 1 ms of the set time.

> Turbo and macros are against the rules in some online games, especially competitive ones.
> Check before using them there. See [Troubleshooting](troubleshooting.md#anti-cheat-and-game-rules).

## Turbo

Select an input, choose **Turbo**:

| Setting | Meaning |
| --- | --- |
| **Presses** | The key (or combo) or mouse button to repeat. Defaults to the input itself, so turbo on Left click rapid-fires left click. |
| **Press every** | Time from one press to the next. 50 ms = 20 presses per second (shown next to the label). Minimum 5 ms. |
| **Hold each press for** | How long each press stays down. Games that check key state once per frame can miss very short presses; 15–20 ms is safe at 60 fps. |
| **Start repeating after** | `0`: repeat right away. Above 0: one press, then this pause, then repeats, like keyboard auto-repeat ("repeat on hold"). |
| **While held / Toggle on / off** | Repeat while you hold the input, or press once to start and again to stop. |

Examples:

- *Rapid fire*: Left click → Turbo, Left click, every 40 ms, hold 15 ms, while held.
- *Auto-repeat for a key*: E → Turbo, E, every 100 ms, hold 20 ms, start after 400 ms.
- *Auto-clicker*: Forward → Turbo, Left click, every 100 ms, toggle.

A toggled turbo stops when you switch to an app with a different profile, change any
setting, or bring this app's window to the front.

## Macros

Macros live in the **Macros** section of the sidebar and can be used by any profile.

### Recording

1. Click **+** next to *MACROS* (or *Action → Macro → New macro…* from a binding).
2. Click **● Record**, then press keys and mouse buttons. The pauses between them are
   recorded too. Keys pressed while this window is focused are recorded without doing
   anything here; you can also switch to another app and record there.
3. Click **■ Stop recording**. The click on the Stop button itself is left out.

Recording again adds to the end.

### Editing

Click a step to edit it on the right: change its kind, its key (click the box and press a
key), mouse button, hold time or delay, or move, duplicate and delete it. **Add step**
inserts after the selected step.

| Step | Does |
| --- | --- |
| Key press | Press and release a key, holding it for the given time |
| Key down / Key up | Press or release a key (hold a key across other steps) |
| Mouse click | Click a button (or one scroll step), holding it for the given time |
| Mouse down / Mouse up | Press or release a mouse button |
| Delay | Wait |
| Text | Type text (Unicode on Windows; ASCII on Linux) |

**Delays:** *Remove all* deletes every delay step. *Set all to* changes every delay at once;
on a macro without delays it adds one between every two steps.

Any key or button still held when a macro finishes or is stopped is released, so nothing
gets stuck down.

### Binding a macro

Select an input in a profile, choose **Macro**, pick the macro and how it plays:

| Playback | Behavior |
| --- | --- |
| **Once** | Plays once per press. Pressing again while it plays does nothing. |
| **Repeat** | Plays *Times* times per press, with *Pause between repeats*. |
| **While held** | Loops while the input is held; stops (and releases its keys) the moment you let go. |
| **Toggle** | First press starts looping, next press stops. |

Playback is set per binding, so the same macro can be a one-shot on one button and a loop
on another. Renaming a macro updates its bindings; deleting it removes them.

Scroll steps have no hold, so a macro or turbo bound to *Scroll up/down* plays one pass per
wheel notch (toggles still toggle).
