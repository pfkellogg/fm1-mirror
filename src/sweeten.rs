//! Sweeten: what a good stereo does to a sound, for the Mellotron: a little warmth (+2.5 dB under 200 Hz) and air
//! (+2 dB over 6 kHz), soft tape saturation, a slow stereo chorus for width, and a small room. The same recipe as the
//! Flute Lab's (aikellogg.com, sweeten.js) and the Android app's. Mono in, stereo out; on and off crossfade.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};

/// On or off (remembered by the app).
pub static ON: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    /// RBJ's shelf filters: `high` a high shelf, else a low one; `db` the gain, slope 1.
    fn shelf(rate: f32, hz: f32, db: f32, high: bool) -> Biquad {
        let a = 10f32.powf(db / 40.0);
        let w = 2.0 * PI * hz / rate;
        let (cw, sw) = (w.cos(), w.sin());
        let alpha = sw / 2.0 * (2f32).sqrt();
        let s = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = if high {
            (a * ((a + 1.0) + (a - 1.0) * cw + s), -2.0 * a * ((a - 1.0) + (a + 1.0) * cw), a * ((a + 1.0) + (a - 1.0) * cw - s),
             (a + 1.0) - (a - 1.0) * cw + s, 2.0 * ((a - 1.0) - (a + 1.0) * cw), (a + 1.0) - (a - 1.0) * cw - s)
        } else {
            (a * ((a + 1.0) - (a - 1.0) * cw + s), 2.0 * a * ((a - 1.0) - (a + 1.0) * cw), a * ((a + 1.0) - (a - 1.0) * cw - s),
             (a + 1.0) + (a - 1.0) * cw + s, -2.0 * ((a - 1.0) + (a + 1.0) * cw), (a + 1.0) + (a - 1.0) * cw - s)
        };
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [a1 / a0, a2 / a0], z: [0.0; 2] }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// A comb filter with damping (Freeverb's), and an allpass.
struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
}
struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

const FEEDBACK: f32 = 0.74; // a small room (about a second to fade)
const DAMP: f32 = 0.42;

impl Comb {
    fn run(&mut self, x: f32) -> f32 {
        let y = self.buf[self.i];
        self.store = y * (1.0 - DAMP) + self.store * DAMP;
        self.buf[self.i] = x + self.store * FEEDBACK;
        self.i = (self.i + 1) % self.buf.len();
        y
    }
}

impl Allpass {
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        b - x
    }
}

pub struct Sweeten {
    rate: f32,
    low: Biquad,
    air: Biquad,
    delay: Vec<f32>,
    w: usize,
    phase: [f32; 2],
    combs: [Vec<Comb>; 2],
    alls: [Vec<Allpass>; 2],
    /// 0 off .. 1 on, moving toward ON (no click).
    mix: f32,
}

const SAT: f32 = 1.4;
// chorus, each side: base delay and depth (seconds), rate (Hz), level
const CHORUS: [(f32, f32, f32); 2] = [(0.012, 0.0015, 0.33), (0.015, 0.0018, 0.27)];
const CHORUS_LEVEL: f32 = 0.38;
const BODY: f32 = 0.82;
const ROOM_LEVEL: f32 = 0.05; // (Freeverb's combs add up: this is about the Flute Lab's room)
const WET: f32 = 0.9;

impl Sweeten {
    pub fn new(rate: f32) -> Sweeten {
        let s = rate / 44100.0;
        let len = |n: usize| ((n as f32 * s) as usize).max(1);
        let combs = |spread: usize| [1116, 1188, 1277, 1356].iter().map(|&n| Comb { buf: vec![0.0; len(n + spread)], i: 0, store: 0.0 }).collect();
        let alls = |spread: usize| [556, 441].iter().map(|&n| Allpass { buf: vec![0.0; len(n + spread)], i: 0 }).collect();
        Sweeten {
            rate,
            low: Biquad::shelf(rate, 200.0, 2.5, false),
            air: Biquad::shelf(rate, 6000.0, 2.0, true),
            delay: vec![0.0; (rate * 0.05) as usize],
            w: 0,
            phase: [0.0, 0.5],
            combs: [combs(0), combs(23)],
            alls: [alls(0), alls(23)],
            mix: if ON.load(Ordering::Relaxed) { 1.0 } else { 0.0 },
        }
    }

    fn tap(&self, seconds: f32) -> f32 {
        let n = self.delay.len();
        let d = seconds * self.rate;
        let (i, f) = (d.floor() as usize, d.fract());
        let at = |k: usize| self.delay[(self.w + n - k) % n];
        at(i) * (1.0 - f) + at(i + 1) * f
    }

    /// One sample in, the left and right out.
    pub fn run(&mut self, x: f32) -> (f32, f32) {
        let target = if ON.load(Ordering::Relaxed) { 1.0 } else { 0.0 };
        self.mix += (target - self.mix) * (1.0 / (0.03 * self.rate)).min(1.0);
        if self.mix < 1e-4 && target == 0.0 {
            return (x, x); // off: straight through (and nothing to work out)
        }
        let t = self.air.run(self.low.run(x));
        let s = (SAT * t).tanh() / SAT.tanh();
        let n = self.delay.len();
        self.w = (self.w + 1) % n;
        self.delay[self.w] = s;
        let mut out = [0.0f32; 2];
        for side in 0..2 {
            let (base, depth, hz) = CHORUS[side];
            self.phase[side] = (self.phase[side] + hz / self.rate).fract();
            let chorus = self.tap(base + depth * (2.0 * PI * self.phase[side]).sin());
            let mut room = 0.0;
            for c in self.combs[side].iter_mut() {
                room += c.run(s * 0.5);
            }
            for a in self.alls[side].iter_mut() {
                room = a.run(room);
            }
            out[side] = (BODY * s + CHORUS_LEVEL * chorus + ROOM_LEVEL * room) * WET;
        }
        let m = self.mix;
        (x * (1.0 - m) + out[0] * m, x * (1.0 - m) + out[1] * m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_straight_through_and_on_is_stereo_and_tame() {
        ON.store(false, Ordering::Relaxed);
        let mut s = Sweeten::new(48000.0);
        assert_eq!(s.run(0.5), (0.5, 0.5));
        ON.store(true, Ordering::Relaxed);
        let mut s = Sweeten::new(48000.0);
        let (mut peak, mut differ) = (0.0f32, false);
        for i in 0..48000 {
            let x = 0.6 * (2.0 * PI * 440.0 * i as f32 / 48000.0).sin();
            let (l, r) = s.run(x);
            peak = peak.max(l.abs()).max(r.abs());
            differ |= (l - r).abs() > 1e-3;
        }
        assert!(differ, "stereo");
        assert!(peak < 1.0, "no clipping: {peak}");
        ON.store(false, Ordering::Relaxed);
    }
}

/// SWEETEN_IN names a raw mono f32 file (44.1 kHz); writes SWEETEN_IN.sweet (raw stereo f32): the app's Sweeten on it.
#[cfg(test)]
#[test]
#[ignore]
fn sweeten_a_file() {
    let p = std::env::var("SWEETEN_IN").unwrap();
    let b = std::fs::read(&p).unwrap();
    ON.store(true, Ordering::Relaxed);
    let mut s = Sweeten::new(44100.0);
    let mut out = Vec::with_capacity(b.len() * 2);
    for c in b.chunks_exact(4) {
        let (l, r) = s.run(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&r.to_le_bytes());
    }
    std::fs::write(format!("{p}.sweet"), out).unwrap();
    ON.store(false, Ordering::Relaxed);
}
