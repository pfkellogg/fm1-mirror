//! The other FM-1 screen mirror: Charles Vestal's, in firmware built on fm1-base with `mirror = true` in its fm1.toml
//! (LOOM-8, Polly, Ocarina, and others by other makers). It speaks in the updater's framing (F0, [0, 89, command,
//! length (3 bytes), body…, checksum] packed 7 bits at a time, F7), as documented in device/mirror.c of
//! github.com/charlesvestal/fm1-base. This is our own decoder of what that file describes:
//!
//! - we send command 0x50 with one byte: 2 = on, and the whole screen again; 1 = on (and keep on: at least every
//!   6 seconds, or it stops); 0 = off;
//! - it answers 0x50 (hello: version, on, width - 1, height - 1), then sends 0x51 rectangles
//!   (x, y, w, h, seq, then the pixels in a small code) and 0x52 fills (x, y, w, h, seq, colour high, low).
//!
//! The pixel code, per rectangle (the colour now starts at 0, and a cache of 64 colours all 0):
//!   0nnnnnnn          n + 1 more pixels of the colour now
//!   10iiiiii          the colour now = cache[i]; one pixel of it
//!   11nnnnnn hi lo    the colour now = hi << 8 | lo, kept in cache[hash(colour)]; n + 1 pixels of it
//! with hash(c) = (c * 40503) >> 10 & 63.
//!
//! Big Knobs and Key Lights speak its panel messages, also from device/mirror.c:
//! - 0x53 input, from the FM-1 while it mirrors: buttons held u16, keys held u32, the encoders' detents (7 x int8),
//!   MASTER u16, the lit buttons u16 (bit 15: PLAY lit red), the lit keys u32, then (version 4) the dim buttons u16
//!   and the dim keys u32 (little-endian). Buttons by its B_* (FX SEL ENV LFO EDIT GLO HOME SAVE ARP SEQ PLAY REC
//!   OCT- OCT+), keys bit k = key k (F3 = 0);
//! - 0x54 remote, to it: seq, op, a, b, c (op 3 a turn: a = EN_* (SELECT ALGO PRESET K1..K4), b = detents, int8;
//!   op 4 MASTER: a | b << 8 = 0..4096). One at a time: it answers 0x54 (seq, op, 1 done / 0 refused), then the next.

use crate::screen::{Screen, SIZE};

pub const REQUEST: u8 = 0x50;
const RECT: u8 = 0x51;
const FILL: u8 = 0x52;
const INPUT: u8 = 0x53;
const REMOTE: u8 = 0x54;

/// One FM-1's fm1-base mirror (each FM-1 plugged in has its own: several can be linked at once).
pub struct State {
    /// Its mirror answered on this link: Big Knobs speak its remote.
    pub active: std::sync::atomic::AtomicBool,
    /// The last rect / fill's seq (256: none yet), and a frame found missing (a gap in seq): the screen is asked for
    /// again (as fm1-base's own viewer does), so nothing a lost frame carried stays wrong (h1's full-screen animation).
    pub last_seq: std::sync::atomic::AtomicU16,
    pub missed: std::sync::atomic::AtomicBool,
    /// Big Knobs' and Key Lights' remote commands for this FM-1.
    pub panel: std::sync::Mutex<Remote>,
}

impl Default for State {
    fn default() -> Self {
        State { active: Default::default(), last_seq: std::sync::atomic::AtomicU16::new(256), missed: Default::default(), panel: Default::default() }
    }
}

impl State {
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn seen(&self, seq: u8) {
        use std::sync::atomic::Ordering::Relaxed;
        let last = self.last_seq.swap(seq as u16, Relaxed);
        if last < 256 && seq != (last as u8).wrapping_add(1) {
            self.missed.store(true, Relaxed);
        }
    }
}

/// Asks for the screen: 2 on with the whole screen again, 1 keep on, 0 off.
pub fn request(what: u8) -> Vec<u8> {
    crate::install::frame(REQUEST, &[what])
}

/// What a message was.
#[derive(Debug, PartialEq)]
pub enum Got {
    /// Not this mirror's.
    Nothing,
    /// Its hello: this firmware has the mirror.
    Hello,
    /// Part of the screen, painted.
    Painted,
    /// Its panel's lights (Key Lights).
    Lights(crate::knobs::Lights),
    /// A remote command was answered: the next may go.
    Answered,
}

/// Its B_* order, as our button ids (knobs.rs BUTTONS: OCT- OCT+ FX SEL ENV LFO EDIT GLO HOME SAVE ARP SEQ PLAY REC).
fn our_buttons(b: u16) -> u16 {
    let mut m = 0;
    for i in 0..14 {
        if b >> i & 1 == 1 {
            m |= 1 << if i < 12 { i + 2 } else { i - 12 };
        }
    }
    if b >> 15 & 1 == 1 {
        m |= 1 << 12; // PLAY lit red: lit
    }
    m
}

fn lights(body: &[u8]) -> Option<crate::knobs::Lights> {
    if body.len() < 21 {
        return None;
    }
    let u16_at = |i: usize| body[i] as u16 | (body[i + 1] as u16) << 8;
    let u32_at = |i: usize| u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]) & ((1 << 27) - 1);
    let mut l = crate::knobs::Lights { btns: our_buttons(u16_at(15)), keys: u32_at(17), ..Default::default() };
    if body.len() >= 27 {
        l.glow_btns = our_buttons(u16_at(21)) & !l.btns;
        l.glow_keys = u32_at(23) & !l.keys;
    }
    Some(l)
}

/// Big Knobs through its remote: turns gathered per knob while one command is on its way, MASTER the latest.
#[derive(Default)]
pub struct Remote {
    seq: u8,
    waiting: Option<std::time::Instant>,
    turns: [i32; 7],
    master: Option<u16>,
    /// Presses and lets-go, in order (op 1 a button, op 2 a key; a, down).
    presses: std::collections::VecDeque<(u8, u8, u8)>,
}

impl Remote {
    /// One of our knobs (knobs.rs: 0 SELECT, 1 ALGORITHM, 2..5 KNOB 1..4, 6 PRESETS) turned `n` clicks.
    pub fn turn(&mut self, ours: u8, n: i32) {
        let en = [0usize, 1, 3, 4, 5, 6, 2][(ours as usize).min(6)];
        self.turns[en] = (self.turns[en] + n).clamp(-127, 127);
    }
    /// MASTER to `v` of 1023.
    pub fn master(&mut self, v: u16) {
        self.master = Some((v.min(1023) as u32 * 4096 / 1023) as u16);
    }
    /// Our button (0..13: knobs.rs BUTTONS) or key (14 + k) pressed or let go.
    pub fn press(&mut self, id: u8, down: bool) {
        let ev = if id < 14 {
            (1, if id >= 2 { id - 2 } else { 12 + id }, down as u8) // its B_*: FX.. REC, then OCT- OCT+
        } else {
            (2, (id - 14).min(26), down as u8)
        };
        self.presses.push_back(ev);
    }
    pub fn answered(&mut self) {
        self.waiting = None;
    }
    /// The next command to send, if there is one and none is on its way (or its answer was lost: 300 ms).
    pub fn next(&mut self) -> Option<Vec<u8>> {
        if self.waiting.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(300)) {
            return None;
        }
        let body = if let Some((op, a, b)) = self.presses.pop_front() {
            [op, a, b, 0]
        } else if let Some(e) = self.turns.iter().position(|&t| t != 0) {
            let n = std::mem::take(&mut self.turns[e]);
            [3, e as u8, n as i8 as u8, 0]
        } else if let Some(v) = self.master.take() {
            [4, (v & 255) as u8, (v >> 8) as u8, 0]
        } else {
            return None;
        };
        self.seq = self.seq.wrapping_add(1);
        self.waiting = Some(std::time::Instant::now());
        Some(crate::install::frame(REMOTE, &[self.seq, body[0], body[1], body[2], body[3]]))
    }
}

/// One message from the FM-1, into the screen.
pub fn apply(screen: &mut Screen, m: &[u8], st: &State) -> Got {
    let Some((command, body)) = parse(m) else { return Got::Nothing };
    match command {
        REQUEST => Got::Hello,
        INPUT => lights(&body).map_or(Got::Nothing, Got::Lights),
        REMOTE => Got::Answered,
        RECT | FILL if body.len() >= 5 => {
            st.seen(body[4]);
            if (command == RECT && rect(screen, &body)) || (command == FILL && fill(screen, &body)) { Got::Painted } else { Got::Nothing }
        }
        _ => Got::Nothing,
    }
}

/// An updater-framed message (any length, unlike the updater's own replies): its command and body.
fn parse(m: &[u8]) -> Option<(u8, Vec<u8>)> {
    if m.len() < 10 || m[0] != 0xf0 || m[m.len() - 1] != 0xf7 {
        return None;
    }
    let (mut raw, mut bits, mut value) = (Vec::with_capacity(m.len()), 0u32, 0u32);
    for &b in &m[1..m.len() - 1] {
        if b > 127 {
            return None;
        }
        value |= (b as u32) << bits;
        bits += 7;
        while bits >= 8 {
            raw.push((value & 255) as u8);
            value >>= 8;
            bits -= 8;
        }
    }
    if raw.len() < 7 || raw[0] != 0 || raw[1] != 89 {
        return None;
    }
    let n = raw[3] as usize | (raw[4] as usize) << 8 | (raw[5] as usize) << 16;
    if n + 7 > raw.len() || raw[6..7 + n].iter().fold(0u8, |s, &b| s.wrapping_add(b)) != 255 {
        return None;
    }
    Some((raw[2], raw[6..6 + n].to_vec()))
}

/// A rectangle's size: 0 means 256 (never sent).
fn side(b: u8) -> usize {
    if b == 0 { 256 } else { b as usize }
}

fn rect(screen: &mut Screen, b: &[u8]) -> bool {
    if b.len() < 5 {
        return false;
    }
    let (x, y, w, h) = (b[0] as usize, b[1] as usize, side(b[2]), side(b[3]));
    if x + w > SIZE || y + h > SIZE {
        return false;
    }
    let total = w * h;
    let (mut p, mut i, mut now, mut cache) = (0usize, 5usize, 0u16, [0u16; 64]);
    let mut put = |colour: u16, n: usize, p: &mut usize| -> bool {
        if *p + n > total {
            return false;
        }
        for _ in 0..n {
            screen.pixels[(y + *p / w) * SIZE + x + *p % w] = colour;
            *p += 1;
        }
        true
    };
    while i < b.len() {
        let op = b[i];
        i += 1;
        let ok = if op < 0x80 {
            put(now, op as usize + 1, &mut p)
        } else if op < 0xc0 {
            now = cache[(op & 63) as usize];
            put(now, 1, &mut p)
        } else {
            if i + 2 > b.len() {
                return false;
            }
            now = (b[i] as u16) << 8 | b[i + 1] as u16;
            i += 2;
            cache[hash(now)] = now;
            put(now, (op & 63) as usize + 1, &mut p)
        };
        if !ok {
            return false;
        }
    }
    p == total
}

fn hash(c: u16) -> usize {
    ((c as u32).wrapping_mul(40503) >> 10 & 63) as usize
}

fn fill(screen: &mut Screen, b: &[u8]) -> bool {
    if b.len() < 7 {
        return false;
    }
    let (x, y, w, h) = (b[0] as usize, b[1] as usize, side(b[2]), side(b[3]));
    let colour = (b[5] as u16) << 8 | b[6] as u16;
    for row in y..(y + h).min(SIZE) {
        for col in x..(x + w).min(SIZE) {
            screen.pixels[row * SIZE + col] = colour;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_messages() {
        // B_HOME (6) lit, B_OCTUP (13) dim, PLAY red; key 0 lit, key 26 dim
        let mut b = vec![0u8; 27];
        b[15] = 1 << 6;
        b[16] = 0x80;
        b[17] = 1;
        b[22] = 1 << 5;
        b[26] = 1 << 2;
        let l = lights(&b).unwrap();
        assert_eq!(l.btns, 1 << 8 | 1 << 12);
        assert_eq!(l.keys, 1);
        assert_eq!(l.glow_btns, 1 << 1);
        assert_eq!(l.glow_keys, 1 << 26);
        let mut r = Remote::default();
        r.turn(6, -2); // PRESETS: its EN_PRESET (2)
        r.turn(2, 1); // KNOB 1: EN_K1 (3)
        assert!(r.next().is_some());
        assert!(r.next().is_none(), "one at a time");
        r.answered();
        assert!(r.next().is_some());
        r.answered();
        assert!(r.next().is_none());
        assert_eq!((r.turns, r.seq), ([0; 7], 2));
        r.press(0, true); // OCT- down: its B_OCTDN (12)
        r.press(8, true); // HOME: its B_HOME (6)
        let f = r.next().unwrap();
        assert_eq!(parse(&f).unwrap().1, vec![3, 1, 12, 1, 0]);
        r.answered();
        assert_eq!(parse(&r.next().unwrap()).unwrap().1, vec![4, 1, 6, 1, 0]);
    }

    /// A device frame, as the firmware builds one.
    fn device(command: u8, body: &[u8]) -> Vec<u8> {
        crate::install::frame(command, body)
    }

    #[test]
    fn request_is_framed_like_the_updater() {
        assert_eq!(parse(&request(2)), Some((REQUEST, vec![2])));
    }

    #[test]
    fn hello_rect_and_fill() {
        let mut s = Screen::default();
        assert_eq!(apply(&mut s, &device(REQUEST, &[5, 1, 239, 239]), &State::default()), Got::Hello);
        // A 4x2 rectangle at (10, 20): 3 pixels of 0xF800 (new colour), 1 more of it (repeat), then 4 of 0x001F
        // (new), and nothing left over.
        let body = [10, 20, 4, 2, 0, 0xc2, 0xf8, 0x00, 0x00, 0xc3, 0x00, 0x1f];
        assert_eq!(apply(&mut s, &device(RECT, &body), &State::default()), Got::Painted);
        assert_eq!(s.pixels[20 * SIZE + 10], 0xf800);
        assert_eq!(s.pixels[20 * SIZE + 13], 0xf800);
        assert_eq!(s.pixels[21 * SIZE + 10], 0x001f);
        assert_eq!(s.pixels[21 * SIZE + 13], 0x001f);
        // The cache, within one rectangle (it starts empty for each): 0xF800, 0x07E0, then 0xF800 again by its index.
        let i = hash(0xf800) as u8;
        let body = [0, 0, 3, 1, 1, 0xc0, 0xf8, 0x00, 0xc0, 0x07, 0xe0, 0x80 | i];
        assert_eq!(apply(&mut s, &device(RECT, &body), &State::default()), Got::Painted);
        assert_eq!(s.pixels[..3], [0xf800, 0x07e0, 0xf800]);
        // A fill.
        assert_eq!(apply(&mut s, &device(FILL, &[100, 100, 3, 3, 2, 0x12, 0x34]), &State::default()), Got::Painted);
        assert_eq!(s.pixels[102 * SIZE + 102], 0x1234);
    }

    #[test]
    fn damaged_or_foreign_messages_are_ignored() {
        let mut s = Screen::default();
        let mut m = device(RECT, &[0, 0, 2, 1, 0, 0xc1, 0x12, 0x34]);
        let at = m.len() - 2;
        m[at] ^= 1; // the checksum no longer adds up
        assert_eq!(apply(&mut s, &m, &State::default()), Got::Nothing);
        assert_eq!(apply(&mut s, &crate::screen::HELLO, &State::default()), Got::Nothing);
        // Too many pixels for the rectangle.
        assert_eq!(apply(&mut s, &device(RECT, &[0, 0, 1, 1, 0, 0xc5, 0x12, 0x34]), &State::default()), Got::Nothing);
        // Off the screen.
        assert_eq!(apply(&mut s, &device(RECT, &[239, 0, 2, 1, 0, 0xc1, 0x12, 0x34]), &State::default()), Got::Nothing);
    }
}
