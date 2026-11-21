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

use crate::screen::{Screen, SIZE};

pub const REQUEST: u8 = 0x50;
const RECT: u8 = 0x51;
const FILL: u8 = 0x52;

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
}

/// One message from the FM-1, into the screen.
pub fn apply(screen: &mut Screen, m: &[u8]) -> Got {
    let Some((command, body)) = parse(m) else { return Got::Nothing };
    match command {
        REQUEST => Got::Hello,
        RECT if rect(screen, &body) => Got::Painted,
        FILL if fill(screen, &body) => Got::Painted,
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
        assert_eq!(apply(&mut s, &device(REQUEST, &[5, 1, 239, 239])), Got::Hello);
        // A 4x2 rectangle at (10, 20): 3 pixels of 0xF800 (new colour), 1 more of it (repeat), then 4 of 0x001F
        // (new), and nothing left over.
        let body = [10, 20, 4, 2, 0, 0xc2, 0xf8, 0x00, 0x00, 0xc3, 0x00, 0x1f];
        assert_eq!(apply(&mut s, &device(RECT, &body)), Got::Painted);
        assert_eq!(s.pixels[20 * SIZE + 10], 0xf800);
        assert_eq!(s.pixels[20 * SIZE + 13], 0xf800);
        assert_eq!(s.pixels[21 * SIZE + 10], 0x001f);
        assert_eq!(s.pixels[21 * SIZE + 13], 0x001f);
        // The cache, within one rectangle (it starts empty for each): 0xF800, 0x07E0, then 0xF800 again by its index.
        let i = hash(0xf800) as u8;
        let body = [0, 0, 3, 1, 1, 0xc0, 0xf8, 0x00, 0xc0, 0x07, 0xe0, 0x80 | i];
        assert_eq!(apply(&mut s, &device(RECT, &body)), Got::Painted);
        assert_eq!(s.pixels[..3], [0xf800, 0x07e0, 0xf800]);
        // A fill.
        assert_eq!(apply(&mut s, &device(FILL, &[100, 100, 3, 3, 2, 0x12, 0x34])), Got::Painted);
        assert_eq!(s.pixels[102 * SIZE + 102], 0x1234);
    }

    #[test]
    fn damaged_or_foreign_messages_are_ignored() {
        let mut s = Screen::default();
        let mut m = device(RECT, &[0, 0, 2, 1, 0, 0xc1, 0x12, 0x34]);
        let at = m.len() - 2;
        m[at] ^= 1; // the checksum no longer adds up
        assert_eq!(apply(&mut s, &m), Got::Nothing);
        assert_eq!(apply(&mut s, &crate::screen::HELLO), Got::Nothing);
        // Too many pixels for the rectangle.
        assert_eq!(apply(&mut s, &device(RECT, &[0, 0, 1, 1, 0, 0xc5, 0x12, 0x34])), Got::Nothing);
        // Off the screen.
        assert_eq!(apply(&mut s, &device(RECT, &[239, 0, 2, 1, 0, 0xc1, 0x12, 0x34])), Got::Nothing);
    }
}
