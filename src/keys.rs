//! Keys: a keyboard (or any MIDI device) playing the FM-1 through this app, and what's been
//! played, for the spyglass. Like FM-1 Mirror + Spyglass on Android (Keys.java, Played.java).
//!
//! Everything from the keyboard goes on to the FM-1 as it came, clock included, except SysEx
//! (that's how the FM-1 is updated). By default every channel message moves to channel 1, where
//! the FM-1 listens: a keyboard knocked onto another channel would be silent otherwise. The FM-1's
//! own keys reach the spyglass too, when KEY OUT is on (its SEQ page 6/8 MIDI TRACK).

use std::sync::{Arc, Mutex};

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutputConnection};

use crate::{is_fm1, APP_NAME};

/// What's been played: the last note and how hard, and the keys held now (for chord names).
pub struct Played {
    down: [bool; 128],
    pub last_note: Option<u8>,
    last_velocity: u8,
    first_velocity: Option<u8>,
    velocity_varies: bool,
    pub note_ons: u64,
}

impl Default for Played {
    fn default() -> Played {
        Played { down: [false; 128], last_note: None, last_velocity: 0, first_velocity: None, velocity_varies: false, note_ons: 0 }
    }
}

impl Played {
    fn event(&mut self, kind: u8, note: u8, value: u8) {
        let note = note & 127;
        match kind {
            0x90 if value > 0 => {
                self.down[note as usize] = true;
                self.last_note = Some(note);
                self.last_velocity = value;
                match self.first_velocity {
                    None => self.first_velocity = Some(value),
                    Some(v) if v != value => self.velocity_varies = true,
                    _ => {}
                }
                self.note_ons += 1;
            }
            0x80 | 0x90 => self.down[note as usize] = false,
            0xB0 if note == 123 || note == 120 => self.down = [false; 128],
            _ => {}
        }
    }

    /// How hard the last key was hit; none until two different ones came (some keyboards always send the same).
    pub fn velocity(&self) -> Option<u8> {
        self.velocity_varies.then_some(self.last_velocity)
    }

    pub fn keys_down(&self) -> Vec<u8> {
        (0..128u8).filter(|&n| self.down[n as usize]).collect()
    }
}

/// One stream's running status; SysEx (the FM-1's screen) passes through untouched.
#[derive(Default)]
pub struct Parser {
    status: u8,
    d1: u8,
    have: bool,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8], played: &mut Played) {
        for &b in bytes {
            if b >= 0xF8 {
                continue; // clock and the like
            }
            if b >= 0x80 {
                self.status = if b < 0xF0 { b } else { 0 };
                self.have = false;
                continue;
            }
            let kind = self.status & 0xF0;
            if self.status == 0 || kind == 0xC0 || kind == 0xD0 {
                continue; // one data byte, nothing to track
            }
            if !self.have {
                self.d1 = b;
                self.have = true;
            } else {
                self.have = false; // running status: the next pair reuses it
                played.event(kind, self.d1, b);
            }
        }
    }
}

/// Keeps what can be passed on: SysEx dropped; with `channel` 1 to 16, channel messages moved to that channel
/// (1 by default, where the FM-1 listens; another for a firmware that listens there, e.g. ChoralRoot's MIDI Keys).
#[derive(Default)]
struct Filter {
    sysex: bool,
}

impl Filter {
    fn pass(&mut self, bytes: &[u8], channel: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        for &b in bytes {
            if b == 0xF0 {
                self.sysex = true;
            } else if b == 0xF7 {
                self.sysex = false;
            } else if b >= 0xF8 {
                out.push(b); // clock and the like, even inside SysEx
            } else if !self.sysex || b >= 0x80 {
                self.sysex = false; // a status byte ends a SysEx that never closed
                out.push(if channel > 0 && (0x80..0xF0).contains(&b) { (b & 0xF0) | (channel - 1) } else { b });
            }
        }
        out
    }
}

/// Ports that can play the FM-1: everything but the FM-1 itself.
pub fn sources() -> Vec<String> {
    let Ok(input) = MidiInput::new(APP_NAME) else { return vec![] };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).filter(|n| !is_fm1(n)).collect()
}

/// Picked by itself when nothing was chosen: a keyboard, not a virtual port or another app's, nor a
/// keyboard's "Transport" / "DAW" port.
pub fn plugged(name: &str) -> bool {
    let n = name.to_lowercase();
    !["iac", "bridge", "spyglass", "network", "session", "virtual", "wi-fi", "loopmidi", "through", "transport", "daw", "ctrl"]
        .iter()
        .any(|h| n.contains(h))
}

/// Of those, which to pick first: one that looks like a keyboard before, say, an audio interface's MIDI port.
pub fn keyboard_first(names: &[String]) -> Option<String> {
    let looks = |n: &String| {
        let n = n.to_lowercase();
        ["key", "piano", "minilab", "launch", "oxygen", "keystep", "microkorg", "akai", "arturia", "novation", "m-audio", "roland", "yamaha", "casio", "korg"]
            .iter()
            .any(|h| n.contains(h))
    };
    let usable: Vec<&String> = names.iter().filter(|n| plugged(n)).collect();
    usable.iter().find(|n| looks(n)).or(usable.first()).map(|n| n.to_string())
}

/// A keyboard's input, passed on to the FM-1 (`out`) and into `played`.
pub struct Keys {
    pub name: String,
    _input: MidiInputConnection<()>,
}

impl Keys {
    /// `out`: where the notes go, the FM-1's link while there is one (None: the spyglass still shows them).
    pub fn open(name: &str, out: Arc<Mutex<Option<Arc<Mutex<MidiOutputConnection>>>>>, played: Arc<Mutex<Played>>, channel: u8, ctx: eframe::egui::Context) -> Option<Keys> {
        let mut input = MidiInput::new(APP_NAME).ok()?;
        input.ignore(Ignore::None); // clock goes through too
        let port = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == name))?;
        let (mut filter, mut parser) = (Filter::default(), Parser::default());
        let connection = input
            .connect(
                &port,
                "FM-1 Mirror keys",
                move |_, bytes, _| {
                    let pass = filter.pass(bytes, channel);
                    if pass.is_empty() {
                        return;
                    }
                    if let Some(o) = out.lock().unwrap().clone() {
                        let _ = o.lock().unwrap().send(&pass);
                    }
                    parser.feed(&pass, &mut played.lock().unwrap());
                    ctx.request_repaint(); // the spyglass
                },
                (),
            )
            .ok()?;
        Some(Keys { name: name.to_string(), _input: connection })
    }
}

/// All notes off on every channel, so nothing hangs when the keyboard goes mid-note.
pub fn silence(out: &Mutex<MidiOutputConnection>) {
    let mut out = out.lock().unwrap();
    for ch in 0..16u8 {
        let _ = out.send(&[0xB0 | ch, 123, 0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_notes_on_channel_one_and_drops_sysex() {
        let mut f = Filter::default();
        // A Keystation on channel 13; a SysEx in the middle; clock inside it still goes.
        assert_eq!(f.pass(&[0x9C, 60, 100], 1), vec![0x90, 60, 100]);
        assert_eq!(f.pass(&[0x9C, 60, 100], 3), vec![0x92, 60, 100]); // ChoralRoot's MIDI Keys on 3
        assert_eq!(f.pass(&[0x9C, 60, 100], 0), vec![0x9C, 60, 100]);
        assert_eq!(f.pass(&[0xF0, 0x7D, 0xF8, 1, 2, 0xF7, 0x8C, 60, 0], 1), vec![0xF8, 0x80, 60, 0]);
    }

    #[test]
    fn picks_the_keyboard_first() {
        let ports = ["IAC Driver Bus 1", "AIR 192 6", "Keystation 49 MK3 (USB MIDI)", "Keystation 49 MK3 (Transport)"].map(String::from);
        assert_eq!(keyboard_first(&ports).as_deref(), Some("Keystation 49 MK3 (USB MIDI)"));
        assert_eq!(keyboard_first(&["AIR 192 6".to_string()]).as_deref(), Some("AIR 192 6"));
    }

    #[test]
    fn tracks_notes_and_chords() {
        let (mut p, mut played) = (Parser::default(), Played::default());
        p.feed(&[0x90, 60, 90, 64, 90, 67, 90], &mut played); // running status
        assert_eq!(played.keys_down(), vec![60, 64, 67]);
        assert_eq!(played.last_note, Some(67));
        assert_eq!(played.velocity(), None); // all the same so far
        p.feed(&[0x80, 64, 0, 0x90, 72, 40], &mut played);
        assert_eq!(played.keys_down(), vec![60, 67, 72]);
        assert_eq!(played.velocity(), Some(40));
        // The FM-1's screen (SysEx) in between changes nothing.
        p.feed(&[0xF0, 0x7D, b'F', b'S', 1, 0, 0, 0xF7], &mut played);
        assert_eq!(played.note_ons, 4);
    }
}
