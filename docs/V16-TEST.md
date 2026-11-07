# Testing FM-1 Mirror with M-VAVE's V16

One real FM-1, tested 2026-11-03 and 2026-11-04. Your results may differ.

**Video:** [Every M-VAVE FM-1 Firmware Explained, and What Happened When We Tested V16](https://youtu.be/WOjvFdw5KMA)

**Reported to M-VAVE:** [m-vave/fm1_open#18](https://github.com/m-vave/fm1_open/issues/18)

## Why

A viewer on V16 lost the FM-1 partway through a Big Screen install, though it stayed plugged in. M-Upgrade got it running again. FM-1 Mirror waited for the FM-1's updater by the exact name `ota-FM-1_015`, and a unit from V16 may call it `ota-FM-1_016`. FM-1 Mirror 1.3.0 and FM-1 Mirror + Spyglass 1.4.0 accept `ota-FM-1_` plus any version.

## Setup

- Mac on macOS 27.2 (26B123), FM-1 Mirror 1.3.0, M-Upgrade 3.11.0311
- FM-1 connected through a chain of USB hubs, not straight into the Mac
- V16 file: `FM-1.fwsc`, dated 2026-10-21, 718,196 bytes, SHA-256 `d017fb65e70aca97ee29d944470c3515a74581df5c0576bf70de917c7b2b61d5`

## Results

| Step | Result |
|---|---|
| Community firmware to V15 (Workbench, Force flash) | Worked. Identity `FM-1_015` |
| V15 to V16 (M-Upgrade) | Worked in about 1 min 23 s. Identity `FM-1_016` |
| FM-1 Mirror 1.3.0 on plain V16 | Shows "This FM-1 has M-VAVE's original firmware" and the two steps. No install offered, as designed |
| V16 back to V15 (Workbench) | Worked. Identity `FM-1_015` |
| V16 to Official Open (Workbench, Force flash) | Worked. Identity `FM-1_8000014` |
| Official Open to Big Screen (FM-1 Mirror 1.3.0) | Worked, no lost connection, live screen |

**What this doesn't show:** we didn't capture the updater's name during the last step, so this shows the install works from V16, not that the `ota-FM-1_016` change is what made the difference. During the V15 to V16 update, M-Upgrade's log showed `ota-FM-1_015`.

**Big Screen can't run on V16 itself.** V16 is closed source, so the screen code can't be added to it. Installing Big Screen replaces V16.

## M-Upgrade problems

- **"The communication component returned invalid device information. Use the complete Release folder."** With a 2-port MIDI keyboard connected (a Keystation 49 MK3), M-Upgrade read the FM-1 correctly, then showed this on every scan. Unplugging the keyboard and restarting M-Upgrade fixed it.
- **Select firmware keeps greying out.** With an audio interface connected (an AIR 192|6), each scan took about 13 seconds and the button could hardly be clicked. Unplugging the interface made it usable.

## SysEx banks on V16

- **First V16 install:** a 32-voice DX7 bank sent over USB imported, saved ("All data saved") and survived a restart.
- **After going back to V15, sending a bank, and reinstalling V16:** every import failed with "Bank save failed", retry offered. 4 tries, 2 valid files, banks A and B, before and after a restart. Cause unknown.

## The same sound on V15 and V16

BLK FOREST, same bank, same notes sent from the Mac, recorded over USB. Two V15 takes matched exactly. Compared with V15, V16 measured the same loudness, about 3 cents lower, and a little brighter, mostly on low notes. This isn't proof: the sound had been edited on V16 before that take.

## Still to find out

- What breaks bank import after reinstalling V16
- Whether the updater names itself `ota-FM-1_016` on this unit
