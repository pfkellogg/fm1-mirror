# FM-1 Mirror: the M-VAVE FM-1's screen, big

The M-VAVE FM-1 is a wonderful little FM synth, but its screen is only 1.54 inches across. If your
eyes aren't what they were, it's hard to read. **FM-1 Mirror shows the FM-1's screen on your
computer, tablet or phone, as big as you like**: the whole window or the whole monitor, sharp, in colour, or in
high-contrast looks such as yellow on black.

It's live. Turn a knob, press a button, play a note, and the big screen changes with the little one.

![The FM-1's screen in FM-1 Mirror](docs/big-screen.png)

Free, for **Mac, Windows and Linux**, and on **Android** as
[FM-1 Mirror + Spyglass](https://github.com/pfkellogg/fm1-mirror-spyglass-android) (the same big screen, plus playing the FM-1 from a USB keyboard and
seeing your notes). Questions, or something not working? Write to me:
**pfkellogg@gmail.com**

## Download

Click the file for your computer. These links always give the newest version:

| Computer | File | First time you open it |
|---|---|---|
| Mac (Apple silicon and Intel) | [**FM-1-Mirror-mac.zip**](https://github.com/pfkellogg/fm1-mirror/releases/latest/download/FM-1-Mirror-mac.zip) | Double-click the zip to unzip it, drag **FM-1 Mirror** (the app, with the FM-1 Mirror name) to Applications and open it. It isn't from the App Store, so the first time macOS may refuse: open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to FM-1 Mirror. |
| Windows 10 and 11 | [**FM-1-Mirror-windows.exe**](https://github.com/pfkellogg/fm1-mirror/releases/latest/download/FM-1-Mirror-windows.exe) | Double-click. If Windows says "protected your PC", click **More info → Run anyway**. |
| Linux | [**FM-1-Mirror-linux**](https://github.com/pfkellogg/fm1-mirror/releases/latest/download/FM-1-Mirror-linux) | `chmod +x FM-1-Mirror-linux` and run it. |
| Android 11 or newer (tablet or phone) | [**FM-1 Mirror + Spyglass**](https://github.com/pfkellogg/fm1-mirror-spyglass-android) | Its own page, with the download and how to plug it in. |

> **Not the green "Code" button.** GitHub's green **Code → Download ZIP** button at the top of this
> page gives the app's source code (a folder called `fm1-mirror-master`), which doesn't run. Use the
> links above, or **Releases** on the right of the page.

Plug the FM-1 into the computer with a USB **data** cable (some cables only charge), turn it on, and
open FM-1 Mirror.

**On Android:** see [FM-1 Mirror + Spyglass](https://github.com/pfkellogg/fm1-mirror-spyglass-android).

## How it works, in plain words

The FM-1 doesn't normally tell a computer what's on its screen. So the FM-1 needs slightly different
firmware (the software inside it) that does.

- **M-VAVE's Open firmware.** In October 2026 M-VAVE made the FM-1's firmware *open*: they published
  its source code so the community can improve it, plus a free web page, the
  [FM1 Workbench](https://open.m-vave.com/fm1-workbench), that installs it. That's called "Official
  Open firmware", and it's M-VAVE's own.
- **FM-1 Big Screen firmware.** This is M-VAVE's Open firmware with one small addition: while FM-1
  Mirror is running, the FM-1 also sends its screen to the computer over the same USB cable it already
  uses for MIDI. When nothing is asking, it sends nothing, and the FM-1 plays exactly as before.
  ([The source code and what changed.](https://github.com/pfkellogg/fm1_open/blob/screen-out/BIG_SCREEN.md))
- **FM-1 Mirror** asks for the screen about once a second and draws it big.

Everything else about the FM-1 is M-VAVE's Open firmware: its sounds, sequencer and menus.

## Setting it up

FM-1 Mirror walks you through this itself. Open it with the FM-1 plugged in and it tells you which
firmware your FM-1 has and what to do next.

**If your FM-1 still has the firmware it came with (most FM-1s), it's two steps:**

1. **Put M-VAVE's Official Open firmware on it, on M-VAVE's own page.** In Chrome or Edge, open the
   [FM1 Workbench](https://open.m-vave.com/fm1-workbench) and go to **Device update**.
   Open **Recovery & force update**, tick **Force flash (migrate from other firmware)**, click
   **Connect device**, tick **I have backed up…** and click **Force flash**. Allow MIDI access if
   Chrome asks. It takes about two minutes and the FM-1 restarts by itself.
   - This replaces the FM-1's sounds with M-VAVE's factory sounds. If you made your own, save them
     first: the page's **Projects & sounds** tab can read and export each group of 32 voices as a
     `.syx` file, and load them back afterwards with **Replace this group…**.
2. **Install Big Screen from FM-1 Mirror.** Back in FM-1 Mirror, click **Install Big Screen…**, then
   **Install now**. A progress bar shows how it goes; it takes a minute or two and the FM-1 restarts
   by itself. Your sound library stays (save any unsaved work on the FM-1 first, as with any update).

![Installing from FM-1 Mirror](docs/installing.png)

**If your FM-1 already runs M-VAVE's Open firmware**, only step 2.

**On Android**, step 2 works the same in FM-1 Mirror + Spyglass. Step 1 is easiest on a computer: the
workbench page needs Chrome or Edge, and then the FM-1 goes back on the tablet for step 2.

Keep the FM-1 plugged in and turned on while either step runs. On Windows, close other music apps
that use the FM-1 first (Windows lets only one app use a MIDI device at a time).

## Going back is easy

Nothing here is permanent, and you don't need FM-1 Mirror to undo it. M-VAVE's own
[FM1 Workbench](https://open.m-vave.com/fm1-workbench) can put either of their firmwares back,
whenever you like, the same way as step 1:

- **Back to M-VAVE's Official Open firmware:** Device update → tick **I have backed up…** →
  Connect device → **Start update**.
- **Back to the original firmware the FM-1 came with:** Device update → **Recovery & force update**
  → **Restore original V15** → Connect device → tick **I have backed up…** → **Install original V15**.
  That also puts back the 128 factory sounds.

If an update is ever interrupted (a cable pulled out, the computer asleep, the computer losing the
FM-1 while it restarts into its updater), nothing is broken: the FM-1 waits in its updater for the
rest of the firmware. Plug it back in if it came out and open FM-1 Mirror: it says **The FM-1 is
waiting in its updater** and **Finish installing Big Screen** carries on from there. M-VAVE's page,
or M-Upgrade, can finish it too.

If the computer keeps losing the FM-1 during Step 2:

- Close every other app that uses the FM-1 first (a DAW, Synth Spyglass, M-VAVE's page in a browser
  tab). On Windows only one app can use it at a time, and the FM-1 reappears under a new name partway
  through the update, so another app can grab it.
- Plug the FM-1 straight into the computer, not through a hub, with a USB data cable.

Coming from M-VAVE's V16 firmware is fine: Step 1 replaces it completely, so there's no need to go
back to V15 first.

## Using FM-1 Mirror

- **F**: full screen (Esc to leave). Drag the window to another monitor or a TV first if you like.
- **Looks**: Color, Bright color, Yellow on black, White on black, Black on white.
- The buttons hide after a few seconds so the screen is as big as it can be; move the mouse to bring
  them back.
- When a newer Big Screen firmware comes with a new FM-1 Mirror, a button offers to update the FM-1.

**On Android**, [FM-1 Mirror + Spyglass](https://github.com/pfkellogg/fm1-mirror-spyglass-android) also plays the FM-1 from a USB keyboard and shows
the notes you play in a spyglass beside the screen.

Also playing it from a USB keyboard? [Synth Spyglass](https://youtu.be/KoUeE4bOoy0) (a separate
app) recognises the Big Screen FM-1 too, shows the name of the sound it's playing, and has a **See
big screen** button of its own.

## For the curious

- `src/screen.rs`: the screen messages. The computer sends `F0 7D 'F' 'S' 00 F7` about once a second
  (`01` to have the whole screen sent again); the FM-1 sends each row it paints as
  `F0 7D 'F' 'S' 01 row col <runs> F7`, each run a count and an RGB565 colour.
- `src/install.rs`: the installer, a port of M-VAVE's own web updater (`sdk/WebUpdate.js` in their
  source): the FM-1's updater asks for each block of the firmware and checks it itself.
- `firmware/`: the Big Screen firmware package this app installs, and its build identity. Built from
  [pfkellogg/fm1_open, branch `screen-out`](https://github.com/pfkellogg/fm1_open/tree/screen-out).
- `tools/simulate_fm1.py`: a pretend FM-1 for trying the app without one;
  `tools/flash_fm1.py`: the installer as a script.
- The Android app, FM-1 Mirror + Spyglass, has its own repository:
  [pfkellogg/fm1-mirror-spyglass-android](https://github.com/pfkellogg/fm1-mirror-spyglass-android).

Build it yourself with Rust: `cargo build --release` (on Linux, install `libasound2-dev` first);
on a Mac, `./make_app.sh` makes the app.

## Thanks

To M-VAVE for opening the FM-1's firmware ([FM1 Open](https://github.com/m-vave/fm1_open)). The Big
Screen firmware is their work with a small addition; it's not an official M-VAVE product, and
M-VAVE doesn't support it.

## License

GPL-3.0 (see LICENSE), like M-VAVE's FM1 Open application code it builds on.

Contact: **pfkellogg@gmail.com**
