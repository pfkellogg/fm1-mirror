//! The FM-1's real sound, for the spyglass's waveform (Synth Spyglass's Scope). The FM-1 sends its audio back over
//! its USB cable, and the computer lists it as an input. This is what it actually plays, whatever the voice, so the
//! wave is always right. On a Mac the first listen asks for the microphone: any audio input counts as one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Its USB name has one of these in it.
const HINTS: [&str; 13] = ["fm-1", "fm1", "jieli", "usb-audio", "usb composite", "felucca", "melodee", "choralroot", "jiant",
    "erik37", "sloop", "skiff", "loom"];   // (community firmwares name their USB audio after themselves: Erik37 "ERIK37")
/// Samples kept: plenty for the window plus a period to find the trigger in.
const KEEP: usize = 8192;
/// How much of the sound is shown, in seconds: the same for every note.
const WINDOW: f32 = 0.02;
/// No sound for this long: it has gone.
const STALE: Duration = Duration::from_secs(1);

pub struct Ring {
    buf: Vec<f32>,
    head: usize,
    filled: u64,
    last: Option<Instant>,
    rate: f32,
}

impl Default for Ring {
    fn default() -> Ring {
        Ring { buf: vec![0.0; KEEP], head: 0, filled: 0, last: None, rate: 48000.0 }
    }
}

impl Ring {
    fn push(&mut self, sample: f32) {
        self.buf[self.head] = sample;
        self.head = (self.head + 1) % KEEP;
        self.filled += 1;
    }

    fn at(&self, i: usize) -> f32 {
        self.buf[(self.head + i) % KEEP]
    }

    /// Hearing it now.
    pub fn listening(&self) -> bool {
        self.last.is_some_and(|t| t.elapsed() < STALE)
    }

    /// The last WINDOW of the sound, starting where the wave crosses zero going up (searched within one period at
    /// `hz`, the note played), so it stands still like an oscilloscope's. Scaled so the loudest point is ±1; none
    /// while it's silent.
    pub fn wave(&self, hz: f64) -> Option<Vec<f32>> {
        if !self.listening() {
            return None;
        }
        let span = ((WINDOW * self.rate).round() as usize).clamp(32, KEEP / 2);
        if self.filled < span as u64 * 2 {
            return None;
        }
        let period = (self.rate as f64 / hz.max(30.0)) as usize;
        let last_start = KEEP - span;
        let first = last_start.saturating_sub(period + 1).max(1);
        let start = (first..=last_start).rev().find(|&i| self.at(i - 1) < 0.0 && self.at(i) >= 0.0).unwrap_or(last_start);
        let mut w: Vec<f32> = (0..span).map(|i| self.at(start + i)).collect();
        let peak = w.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak < 0.002 {
            return None;
        }
        w.iter_mut().for_each(|s| *s /= peak);
        Some(w)
    }
}

/// The input stream; dropping it stops listening.
pub struct Listener {
    _stream: cpal::Stream,
    dead: Arc<AtomicBool>,
}

impl Listener {
    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }
}

fn is_fm1(name: &str) -> bool {
    let n = name.to_lowercase();
    HINTS.iter().any(|h| n.contains(h))
}

fn feed<T: Copy>(ring: &Mutex<Ring>, data: &[T], channels: usize, sample: impl Fn(T) -> f32) {
    if let Ok(mut r) = ring.lock() {
        for frame in data.chunks(channels.max(1)) {
            r.push(sample(frame[0]));
        }
        if !data.is_empty() {
            r.last = Some(Instant::now());
        }
    }
}

/// Starts listening to the FM-1's USB input; none if it isn't there or isn't allowed.
pub fn open(ring: Arc<Mutex<Ring>>) -> Option<Listener> {
    let device = cpal::default_host().input_devices().ok()?.find(|d| d.name().is_ok_and(|n| is_fm1(&n)))?;
    let config = device.default_input_config().ok()?;
    let channels = config.channels() as usize;
    ring.lock().ok()?.rate = config.sample_rate().0 as f32;
    let dead = Arc::new(AtomicBool::new(false));
    let gone = dead.clone();
    let on_error = move |_| gone.store(true, Ordering::Relaxed);
    let stream_config = config.config();
    let r = ring.clone();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(&stream_config, move |d: &[f32], _: &_| feed(&r, d, channels, |s| s), on_error, None),
        cpal::SampleFormat::I16 => device.build_input_stream(&stream_config, move |d: &[i16], _: &_| feed(&r, d, channels, |s| s as f32 / 32768.0), on_error, None),
        cpal::SampleFormat::I32 => device.build_input_stream(&stream_config, move |d: &[i32], _: &_| feed(&r, d, channels, |s| s as f32 / 2147483648.0), on_error, None),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(Listener { _stream: stream, dead })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wave_starts_rising_through_zero_and_fills_the_height() {
        let mut r = Ring::default();
        for i in 0..KEEP {
            r.push(0.25 * (i as f32 * 440.0 * std::f32::consts::TAU / 48000.0).sin());
        }
        r.last = Some(Instant::now());
        let w = r.wave(440.0).unwrap();
        assert_eq!(w.len(), 960);
        assert!(w[0].abs() < 0.1 && w[1] > w[0]);
        assert!((w.iter().fold(0.0f32, |m, s| m.max(s.abs())) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn silence_or_nothing_heard_is_no_wave() {
        let mut r = Ring::default();
        assert!(r.wave(440.0).is_none());
        r.filled = KEEP as u64;
        r.last = Some(Instant::now());
        assert!(r.wave(440.0).is_none());
    }

    #[test]
    fn finds_the_fm1_by_its_name() {
        assert!(is_fm1("Melodee FM-1") && is_fm1("USB-Audio") && !is_fm1("AIR 192 6") && !is_fm1("HD Webcam eMeet C960"));
        assert!(is_fm1("ERIK37") && is_fm1("LOOM-8 FM-1") && !is_fm1("Mac mini Microphone"));
    }
}
