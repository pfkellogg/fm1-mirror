//! Big Screen version 2: David Konsumer's protocol (github.com/konsumer/foma/tree/main/bigscreen, MIT), the successor
//! of our version 1 (screen.rs). The device sends only the 16 x 16 tiles that changed, compressed, and says when a
//! frame is whole, so we show whole frames. This is our own decoder of what its README describes:
//!
//! - we say hello with `F0 7D 'B' 'S' 00 <version 2> <flags> F7` (flags bit 0: send all of the screen again);
//! - a device that speaks it answers each hello with INFO `F0 7D 'B' 'S' 01 <version> <w (14)> <h (14)> <format> F7`;
//!   firmware that only knows version 1 doesn't answer, and after 3 hellos we use version 1;
//! - RECT `F0 7D 'B' 'S' 02 <x (14)> <y (14)> <w (14)> <h (14)> <pixels…> F7` fills a rectangle row by row;
//! - SYNC `F0 7D 'B' 'S' 03 F7`: everything that changed has been sent (show it now). A screen that changes faster
//!   than it can be sent may not get to one for a while: show what came after about 60 ms anyway.
//!
//! The pixel code, per RECT (the cache starts as 64 zeros, the last colour as 0):
//!   00iiiiii           the colour in cache slot i (one pixel)
//!   010nnnnn           the last colour again, n + 1 times
//!   0110nnnn b         the last colour again, (n << 7 | b) + 33 times
//!   011100hh b1 b2     a new colour c = hh << 14 | b1 << 7 | b2 (RGB565), kept in cache[hash(c)] (one pixel)
//!   anything else      not version 2: stop reading
//! with hash(c) = (r * 3 + g * 5 + b * 7) & 63 (r = c >> 11, g = c >> 5 & 63, b = c & 31).
//! (Numbers marked (14) are two 7-bit bytes, low first.)

use crate::screen::SIZE;
use std::time::{Duration, Instant};

pub const VERSION: u8 = 2;
const HELLO: u8 = 0;
const INFO: u8 = 1;
const RECT: u8 = 2;
const SYNC: u8 = 3;
/// Show what came this long after the last SYNC-less rectangle, even with no SYNC.
pub const SHOW_AFTER: Duration = Duration::from_millis(60);
/// Version 2 hellos with no INFO back: then it's version 1 firmware.
pub const PROBE_TRIES: u32 = 3;
/// How often to say hello while asking which version the FM-1 speaks.
pub const PROBE_EVERY: Duration = Duration::from_millis(500);
/// Speaking version 1: every this many hellos, one asks for version 2 instead (a firmware update brings us up).
pub const REPROBE: u32 = 10;

/// Our hello: version 2, and maybe "send all of the screen again".
pub fn hello(all: bool) -> [u8; 8] {
    [0xf0, 0x7d, b'B', b'S', HELLO, VERSION, all as u8, 0xf7]
}

fn hash(c: u16) -> usize {
    let (r, g, b) = ((c >> 11) as usize, ((c >> 5) & 63) as usize, (c & 31) as usize);
    (r * 3 + g * 5 + b * 7) & 63
}

/// What a message was.
#[derive(Debug, PartialEq)]
pub enum Got {
    /// Not a version 2 message from a device (or one we can't read).
    Nothing,
    /// Its answer to a hello: it speaks version 2.
    Info,
    /// Part of the next frame (not shown yet).
    Painted,
    /// A frame is whole: show it.
    Frame,
}

/// The frame being put together: rectangles go here, and are shown (copied to the screen) at a SYNC.
pub struct Viewer {
    pub back: Vec<u16>,
    /// The device's screen size (from its INFO).
    pub w: usize,
    pub h: usize,
    /// When the first rectangle since the frame last shown came (None: nothing waiting).
    pub waiting_since: Option<Instant>,
}

impl Default for Viewer {
    fn default() -> Self {
        Viewer { back: vec![0; SIZE * SIZE], w: SIZE, h: SIZE, waiting_since: None }
    }
}

impl Viewer {
    /// One message from the FM-1.
    pub fn apply(&mut self, m: &[u8]) -> Got {
        if m.len() < 6 || m[0] != 0xf0 || m[1] != 0x7d || m[2] != b'B' || m[3] != b'S' || m[m.len() - 1] != 0xf7 {
            return Got::Nothing;
        }
        let body = &m[4..m.len() - 1];
        if body.iter().any(|&b| b > 127) {
            return Got::Nothing;
        }
        let n14 = |i: usize| body[i] as usize | (body[i + 1] as usize) << 7;
        match body[0] {
            INFO if body.len() >= 7 => {
                self.w = n14(2);
                self.h = n14(4);
                Got::Info
            }
            SYNC => Got::Frame,
            RECT if body.len() >= 9 => {
                self.rect(n14(1), n14(3), n14(5), n14(7), &body[9..]);
                self.waiting_since.get_or_insert_with(Instant::now);
                Got::Painted
            }
            _ => Got::Nothing, // (another viewer's hello, a short message, or something newer)
        }
    }

    fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, code: &[u8]) {
        let total = w * h;
        if w == 0 || total == 0 {
            return;
        }
        let (mut cache, mut c, mut p, mut i) = ([0u16; 64], 0u16, 0usize, 0usize);
        let at = |i: usize| code.get(i).copied().unwrap_or(0) as u16;
        while i < code.len() && p < total {
            let b = code[i];
            i += 1;
            let mut run = 1usize;
            match b {
                0x00..=0x3f => c = cache[b as usize],
                0x40..=0x5f => run = (b & 31) as usize + 1,
                0x60..=0x6f => {
                    run = (((b & 15) as usize) << 7 | at(i) as usize) + 33;
                    i += 1;
                }
                0x70..=0x73 => {
                    c = ((b & 3) as u16) << 14 | at(i) << 7 | at(i + 1);
                    i += 2;
                    cache[hash(c)] = c;
                }
                _ => break, // (not version 2: something newer)
            }
            for _ in 0..run.min(total - p) {
                let (x, y) = (x0 + p % w, y0 + p / w);
                if x < SIZE && y < SIZE {
                    self.back[y * SIZE + x] = c;
                }
                p += 1;
            }
        }
    }

    /// Time to show what came, though no SYNC has (the screen is changing faster than it can be sent)?
    pub fn overdue(&self) -> bool {
        self.waiting_since.is_some_and(|t| t.elapsed() >= SHOW_AFTER)
    }

    /// Copies the frame to the screen.
    pub fn show(&mut self, screen: &mut [u16]) {
        screen.copy_from_slice(&self.back);
        self.waiting_since = None;
    }
}

/// Which version we're speaking: the hello thread's side of it. `answered` counts INFOs (set by the input).
pub struct Asker {
    pub ver: u8,
    tries: u32,
    count: u32,
    seen: u32,
    all: bool,
}

impl Default for Asker {
    fn default() -> Self {
        Asker { ver: 0, tries: 0, count: 0, seen: 0, all: true }
    }
}

impl Asker {
    /// How long to wait between hellos now.
    pub fn every(&self, usual: Duration) -> Duration {
        if self.ver == 0 { PROBE_EVERY } else { usual }
    }

    /// The next hello to send, given how many INFOs have come so far. Asks version 2 first; after 3 with no answer,
    /// speaks version 1 (asking for version 2 again every 10th hello); back to asking if a version 2 device goes quiet.
    pub fn next(&mut self, answered: u32) -> Vec<u8> {
        if answered != self.seen {
            self.seen = answered;
            self.tries = 0;
            if self.ver != 2 {
                self.ver = 2;
                self.all = true; // (a frame from the start)
            }
        }
        if self.tries >= PROBE_TRIES {
            match self.ver {
                0 => {
                    self.ver = 1;
                    self.all = true;
                    self.count = 0;
                }
                2 => {
                    self.ver = 0; // it went quiet: ask again
                    self.tries = 0;
                    self.all = true;
                }
                _ => {}
            }
        }
        if self.ver == 1 {
            self.count += 1;
            if self.count % REPROBE != 0 {
                let all = std::mem::take(&mut self.all);
                return (if all { crate::screen::HELLO_ALL } else { crate::screen::HELLO }).to_vec();
            }
            return hello(false).to_vec(); // (a version 1 firmware ignores it; one that updated answers)
        }
        self.tries += 1;
        let all = std::mem::take(&mut self.all) || self.ver == 0;
        hello(all).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n14(v: usize) -> [u8; 2] {
        [(v & 127) as u8, (v >> 7) as u8]
    }

    fn rect_msg(x: usize, y: usize, w: usize, h: usize, code: &[u8]) -> Vec<u8> {
        let mut m = vec![0xf0, 0x7d, b'B', b'S', RECT];
        for v in [x, y, w, h] {
            m.extend(n14(v));
        }
        m.extend_from_slice(code);
        m.push(0xf7);
        m
    }

    #[test]
    fn info_and_sync() {
        let mut v = Viewer::default();
        let mut m = vec![0xf0, 0x7d, b'B', b'S', INFO, 2];
        m.extend(n14(240));
        m.extend(n14(240));
        m.extend([0, 0xf7]);
        assert_eq!(v.apply(&m), Got::Info);
        assert_eq!((v.w, v.h), (240, 240));
        assert_eq!(v.apply(&[0xf0, 0x7d, b'B', b'S', SYNC, 0xf7]), Got::Frame);
        assert_eq!(v.apply(&hello(true)), Got::Nothing); // (another viewer's hello)
        assert_eq!(v.apply(&crate::screen::HELLO), Got::Nothing);
    }

    #[test]
    fn colours_runs_and_the_cache() {
        let mut v = Viewer::default();
        let red: u16 = 0xf800;
        let blue: u16 = 0x001f;
        let new = |c: u16| [0x70 | (c >> 14) as u8, (c >> 7 & 127) as u8, (c & 127) as u8];
        // 4 x 3: red, red x3 more (run of 3), blue, a long run of blue to the end, then red from the cache
        let mut code = vec![];
        code.extend(new(red));
        code.push(0x40 | 2); // 3 more red
        code.extend(new(blue));
        code.push(0x40 | 4); // 5 more blue (to pixel 9)
        code.push(hash(red) as u8); // red from the cache
        code.push(0x40); // one more
        assert_eq!(v.apply(&rect_msg(10, 20, 4, 3, &code)), Got::Painted);
        let px = |x: usize, y: usize| v.back[y * SIZE + x];
        assert_eq!([px(10, 20), px(13, 20)], [red, red]);
        assert_eq!([px(10, 21), px(13, 21), px(11, 22)], [blue, blue, blue]);
        assert_eq!([px(12, 22), px(13, 22)], [red, red]);
        assert_eq!(px(14, 20), 0); // (nothing outside)
        assert!(v.waiting_since.is_some());
        let mut screen = vec![0; SIZE * SIZE];
        v.show(&mut screen);
        assert_eq!(screen[20 * SIZE + 10], red);
        assert!(v.waiting_since.is_none());
    }

    #[test]
    fn a_long_run_and_clipping() {
        let mut v = Viewer::default();
        // a whole 240 x 240 screen of one colour: a new colour, then runs of (n << 7 | b) + 33
        let c: u16 = 0x07e0;
        let mut code = vec![0x70 | (c >> 14) as u8, (c >> 7 & 127) as u8, (c & 127) as u8];
        let mut left = SIZE * SIZE - 1;
        while left > 0 {
            let r = left.clamp(33, 2080);
            let k = r - 33;
            code.extend([0x60 | (k >> 7) as u8, (k & 127) as u8]);
            left -= r.min(left);
        }
        v.apply(&rect_msg(0, 0, SIZE, SIZE, &code));
        assert!(v.back.iter().all(|&p| p == c));
        // a rectangle past the edge doesn't panic
        v.apply(&rect_msg(235, 235, 16, 16, &[0x70, 0, 1, 0x6f, 127]));
        assert_eq!(v.back[SIZE * SIZE - 1], 1);
    }

    #[test]
    fn asks_version_2_then_falls_back_and_reprobes() {
        let mut a = Asker::default();
        for _ in 0..3 {
            assert_eq!(a.next(0), hello(true).to_vec());
        }
        assert_eq!(a.next(0), crate::screen::HELLO_ALL.to_vec()); // no answer: version 1, the whole screen
        assert_eq!(a.ver, 1);
        let later: Vec<Vec<u8>> = (0..9).map(|_| a.next(0)).collect();
        assert_eq!(later[0], crate::screen::HELLO.to_vec());
        assert_eq!(later[8], hello(false).to_vec()); // the 10th asks for version 2
        assert_eq!(a.next(1), hello(true).to_vec()); // it answered: version 2, from a whole frame
        assert_eq!(a.ver, 2);
        assert_eq!(a.next(2), hello(false).to_vec());
    }

    #[test]
    fn a_version_2_device_answers_at_once() {
        let mut a = Asker::default();
        a.next(0);
        assert_eq!(a.next(1), hello(true).to_vec());
        assert_eq!(a.ver, 2);
        // it goes quiet (another firmware): after 3 unanswered, ask again from the start
        for _ in 0..3 {
            a.next(1);
        }
        a.next(1);
        assert_eq!(a.ver, 0);
    }
}

/// Against konsumer's own device code (bigscreen.h): BS2_MESSAGES names a file of what it sent, one message per hex
/// line, and "S <the screen>" at each SYNC. Every frame we show must be its screen, pixel for pixel.
#[cfg(test)]
#[test]
#[ignore]
fn matches_bigscreen_h() {
    let text = std::fs::read_to_string(std::env::var("BS2_MESSAGES").expect("BS2_MESSAGES")).unwrap();
    let hex = |s: &str| (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
    let (mut v, mut screen, mut frames, mut info) = (Viewer::default(), vec![0u16; SIZE * SIZE], 0, 0);
    for line in text.lines() {
        if let Some(want) = line.strip_prefix("S ") {
            let want: Vec<u16> = (0..SIZE * SIZE).map(|i| u16::from_str_radix(&want[i * 4..i * 4 + 4], 16).unwrap()).collect();
            let bad = screen.iter().zip(&want).filter(|(a, b)| a != b).count();
            assert_eq!(bad, 0, "frame {frames}: {bad} pixels differ");
            frames += 1;
            continue;
        }
        match v.apply(&hex(line)) {
            Got::Frame => v.show(&mut screen),
            Got::Info => info += 1,
            Got::Nothing => panic!("not read: {line}"),
            Got::Painted => {}
        }
    }
    assert!(frames > 0 && info > 0);
    eprintln!("{frames} frames, all pixel for pixel; {info} INFOs");
}
