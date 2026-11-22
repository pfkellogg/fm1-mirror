//! A Mellotron built in (the "Mellotron" button), as in FM-1 Big Screen + Spyglass for Android 1.6.9: the FM-1's own
//! keys play tape strips out of this computer's own sound, never back over USB to the FM-1. Like the real one: one
//! recording per key over 35 keys, G2 (43) to F5 (77), each 8 seconds long, which simply runs out if held longer; no
//! velocity; a key let go stops its strip quickly. Notes outside the 35 keys are moved by octaves into them.
//! Flute, strings and choir are made from scratch by programs in the Android repo (tools/make_mellotron_*.py: no
//! recordings, so nothing anyone else owns), as FLAC in assets/mellotron, built into the app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: f64 = 44100.0;
const LOW: u8 = 43;
const HIGH: u8 = 77;
const VOICES: usize = 16;

/// The sounds, in the order the button steps through them (then off).
pub const SOUNDS: [&str; 3] = ["flute", "strings", "choir"];

/// Windows and Linux: the strips built into the program.
#[cfg(not(target_os = "macos"))]
static FILES: [(&str, [&[u8]; 35]); 3] = [
    ("flute", [
        include_bytes!("../assets/mellotron/flute/043.flac"),
        include_bytes!("../assets/mellotron/flute/044.flac"),
        include_bytes!("../assets/mellotron/flute/045.flac"),
        include_bytes!("../assets/mellotron/flute/046.flac"),
        include_bytes!("../assets/mellotron/flute/047.flac"),
        include_bytes!("../assets/mellotron/flute/048.flac"),
        include_bytes!("../assets/mellotron/flute/049.flac"),
        include_bytes!("../assets/mellotron/flute/050.flac"),
        include_bytes!("../assets/mellotron/flute/051.flac"),
        include_bytes!("../assets/mellotron/flute/052.flac"),
        include_bytes!("../assets/mellotron/flute/053.flac"),
        include_bytes!("../assets/mellotron/flute/054.flac"),
        include_bytes!("../assets/mellotron/flute/055.flac"),
        include_bytes!("../assets/mellotron/flute/056.flac"),
        include_bytes!("../assets/mellotron/flute/057.flac"),
        include_bytes!("../assets/mellotron/flute/058.flac"),
        include_bytes!("../assets/mellotron/flute/059.flac"),
        include_bytes!("../assets/mellotron/flute/060.flac"),
        include_bytes!("../assets/mellotron/flute/061.flac"),
        include_bytes!("../assets/mellotron/flute/062.flac"),
        include_bytes!("../assets/mellotron/flute/063.flac"),
        include_bytes!("../assets/mellotron/flute/064.flac"),
        include_bytes!("../assets/mellotron/flute/065.flac"),
        include_bytes!("../assets/mellotron/flute/066.flac"),
        include_bytes!("../assets/mellotron/flute/067.flac"),
        include_bytes!("../assets/mellotron/flute/068.flac"),
        include_bytes!("../assets/mellotron/flute/069.flac"),
        include_bytes!("../assets/mellotron/flute/070.flac"),
        include_bytes!("../assets/mellotron/flute/071.flac"),
        include_bytes!("../assets/mellotron/flute/072.flac"),
        include_bytes!("../assets/mellotron/flute/073.flac"),
        include_bytes!("../assets/mellotron/flute/074.flac"),
        include_bytes!("../assets/mellotron/flute/075.flac"),
        include_bytes!("../assets/mellotron/flute/076.flac"),
        include_bytes!("../assets/mellotron/flute/077.flac"),
    ]),
    ("strings", [
        include_bytes!("../assets/mellotron/strings/043.flac"),
        include_bytes!("../assets/mellotron/strings/044.flac"),
        include_bytes!("../assets/mellotron/strings/045.flac"),
        include_bytes!("../assets/mellotron/strings/046.flac"),
        include_bytes!("../assets/mellotron/strings/047.flac"),
        include_bytes!("../assets/mellotron/strings/048.flac"),
        include_bytes!("../assets/mellotron/strings/049.flac"),
        include_bytes!("../assets/mellotron/strings/050.flac"),
        include_bytes!("../assets/mellotron/strings/051.flac"),
        include_bytes!("../assets/mellotron/strings/052.flac"),
        include_bytes!("../assets/mellotron/strings/053.flac"),
        include_bytes!("../assets/mellotron/strings/054.flac"),
        include_bytes!("../assets/mellotron/strings/055.flac"),
        include_bytes!("../assets/mellotron/strings/056.flac"),
        include_bytes!("../assets/mellotron/strings/057.flac"),
        include_bytes!("../assets/mellotron/strings/058.flac"),
        include_bytes!("../assets/mellotron/strings/059.flac"),
        include_bytes!("../assets/mellotron/strings/060.flac"),
        include_bytes!("../assets/mellotron/strings/061.flac"),
        include_bytes!("../assets/mellotron/strings/062.flac"),
        include_bytes!("../assets/mellotron/strings/063.flac"),
        include_bytes!("../assets/mellotron/strings/064.flac"),
        include_bytes!("../assets/mellotron/strings/065.flac"),
        include_bytes!("../assets/mellotron/strings/066.flac"),
        include_bytes!("../assets/mellotron/strings/067.flac"),
        include_bytes!("../assets/mellotron/strings/068.flac"),
        include_bytes!("../assets/mellotron/strings/069.flac"),
        include_bytes!("../assets/mellotron/strings/070.flac"),
        include_bytes!("../assets/mellotron/strings/071.flac"),
        include_bytes!("../assets/mellotron/strings/072.flac"),
        include_bytes!("../assets/mellotron/strings/073.flac"),
        include_bytes!("../assets/mellotron/strings/074.flac"),
        include_bytes!("../assets/mellotron/strings/075.flac"),
        include_bytes!("../assets/mellotron/strings/076.flac"),
        include_bytes!("../assets/mellotron/strings/077.flac"),
    ]),
    ("choir", [
        include_bytes!("../assets/mellotron/choir/043.flac"),
        include_bytes!("../assets/mellotron/choir/044.flac"),
        include_bytes!("../assets/mellotron/choir/045.flac"),
        include_bytes!("../assets/mellotron/choir/046.flac"),
        include_bytes!("../assets/mellotron/choir/047.flac"),
        include_bytes!("../assets/mellotron/choir/048.flac"),
        include_bytes!("../assets/mellotron/choir/049.flac"),
        include_bytes!("../assets/mellotron/choir/050.flac"),
        include_bytes!("../assets/mellotron/choir/051.flac"),
        include_bytes!("../assets/mellotron/choir/052.flac"),
        include_bytes!("../assets/mellotron/choir/053.flac"),
        include_bytes!("../assets/mellotron/choir/054.flac"),
        include_bytes!("../assets/mellotron/choir/055.flac"),
        include_bytes!("../assets/mellotron/choir/056.flac"),
        include_bytes!("../assets/mellotron/choir/057.flac"),
        include_bytes!("../assets/mellotron/choir/058.flac"),
        include_bytes!("../assets/mellotron/choir/059.flac"),
        include_bytes!("../assets/mellotron/choir/060.flac"),
        include_bytes!("../assets/mellotron/choir/061.flac"),
        include_bytes!("../assets/mellotron/choir/062.flac"),
        include_bytes!("../assets/mellotron/choir/063.flac"),
        include_bytes!("../assets/mellotron/choir/064.flac"),
        include_bytes!("../assets/mellotron/choir/065.flac"),
        include_bytes!("../assets/mellotron/choir/066.flac"),
        include_bytes!("../assets/mellotron/choir/067.flac"),
        include_bytes!("../assets/mellotron/choir/068.flac"),
        include_bytes!("../assets/mellotron/choir/069.flac"),
        include_bytes!("../assets/mellotron/choir/070.flac"),
        include_bytes!("../assets/mellotron/choir/071.flac"),
        include_bytes!("../assets/mellotron/choir/072.flac"),
        include_bytes!("../assets/mellotron/choir/073.flac"),
        include_bytes!("../assets/mellotron/choir/074.flac"),
        include_bytes!("../assets/mellotron/choir/075.flac"),
        include_bytes!("../assets/mellotron/choir/076.flac"),
        include_bytes!("../assets/mellotron/choir/077.flac"),
    ]),
];

/// One strip's FLAC: on a Mac from the app's Resources (make_app.sh copies them there once, rather than into each of
/// the program's two halves, Apple silicon and Intel), else from the program itself.
fn flac(sound: &str, key: u8) -> Option<std::borrow::Cow<'static, [u8]>> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok()?;
        let name = format!("mellotron/{sound}/{key:03}.flac");
        let res = exe.parent()?.parent()?.join("Resources").join(&name);
        let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(&name);   // (cargo run, tests)
        std::fs::read(res).or_else(|_| std::fs::read(dev)).ok().map(std::borrow::Cow::Owned)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let files = FILES.iter().find(|(n, _)| *n == sound).map(|(_, f)| f)?;
        files.get((key - LOW) as usize).map(|b| std::borrow::Cow::Borrowed(*b))
    }
}

type Set = Arc<Vec<Option<Arc<Vec<i16>>>>>;

/// Sounds decoded before (a second or so each), kept for switching back instantly.
fn cache() -> &'static Mutex<HashMap<&'static str, Set>> {
    static C: OnceLock<Mutex<HashMap<&'static str, Set>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn decode(bytes: &[u8]) -> Option<Vec<i16>> {
    let mut r = claxon::FlacReader::new(std::io::Cursor::new(bytes)).ok()?;
    let ch = r.streaminfo().channels as usize;
    let bits = r.streaminfo().bits_per_sample;
    let shift = bits.saturating_sub(16);
    let samples: Vec<i32> = r.samples().filter_map(Result::ok).collect();
    Some(samples.chunks(ch.max(1)).map(|f| (f.iter().map(|&s| (s >> shift) as i64).sum::<i64>() / f.len() as i64) as i16).collect())
}

fn strips(sound: &'static str) -> Set {
    if let Some(s) = cache().lock().unwrap().get(sound) {
        return s.clone();
    }
    let keys: Vec<u8> = (LOW..=HIGH).collect();
    let decoded: Vec<Option<Arc<Vec<i16>>>> = std::thread::scope(|sc| {
        let hs: Vec<_> = keys.chunks(9).map(|part| sc.spawn(move || part.iter().map(|&k| flac(sound, k).and_then(|b| decode(&b)).map(Arc::new)).collect::<Vec<_>>())).collect();
        hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut all = vec![None; HIGH as usize + 1];
    for (i, s) in decoded.into_iter().enumerate() {
        all[LOW as usize + i] = s;
    }
    let set: Set = Arc::new(all);
    cache().lock().unwrap().insert(sound, set.clone());
    set
}

struct Voice {
    key: u8,
    data: Arc<Vec<i16>>,
    pos: f64,
    env: f32,
    released: bool,
}

/// One source's running status (each source has its own: bytes of two sources mixed would make up notes).
#[derive(Default)]
struct Parser {
    status: u8,
    d1: Option<u8>,
    sysex: bool,
}

struct Engine {
    sound: &'static str,
    strips: Option<Set>,
    voices: Vec<Voice>,
    fm1: Parser,
    keys: Parser,
    /// Who played the last note ("FM-1", or the keyboard's name), for the spyglass.
    player: String,
    /// The last moment of what it played, for the spyglass's waveform.
    ring: crate::scope::Ring,
}

impl Engine {
    /// MIDI from the FM-1 (`keyboard` false) or from a keyboard plugged into the computer (true): each source keeps
    /// its own running status.
    fn parse_from(&mut self, keyboard: bool, m: &[u8]) {
        let mut p = std::mem::take(if keyboard { &mut self.keys } else { &mut self.fm1 });
        for &b in m {
            if b >= 0xf8 {
                continue;
            }
            if b == 0xf0 {
                p.sysex = true;
                p.status = 0;
                continue;
            }
            if b >= 0x80 {
                p.sysex = false;
                p.status = if b < 0xf0 { b } else { 0 };
                p.d1 = None;
                continue;
            }
            if p.sysex || p.status == 0 {
                continue;
            }
            let kind = p.status & 0xf0;
            if kind == 0xc0 || kind == 0xd0 {
                continue;
            }
            let Some(d1) = p.d1 else {
                p.d1 = Some(b);
                continue;
            };
            p.d1 = None;
            match kind {
                0x90 if b > 0 => self.note_on(d1),
                0x80 | 0x90 => self.note_off(d1),
                0xb0 if d1 == 120 || d1 == 123 => self.voices.iter_mut().for_each(|v| v.released = true),
                _ => {}
            }
        }
        if keyboard { self.keys = p } else { self.fm1 = p }
    }

    fn parse(&mut self, m: &[u8]) {
        self.parse_from(false, m);
    }

    fn note_on(&mut self, note: u8) {
        let key = fold(note);
        let Some(data) = self.strips.as_ref().and_then(|s| s[key as usize].clone()) else { return };
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.released = true;                       // the same key again: its strip starts over
        }
        if self.voices.len() >= VOICES {
            self.voices.remove(0);
        }
        self.voices.push(Voice { key, data, pos: 0.0, env: 0.0, released: false });
    }

    fn note_off(&mut self, note: u8) {
        let key = fold(note);
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.released = true;
        }
    }

    /// `out` mono at `rate`: the strips (44.1 kHz) stepped to it, a 4 ms start, 0.12 s fade when let go, headroom for
    /// chords and only squeezed near the top (Android 1.6.9's fixes).
    fn mix(&mut self, out: &mut [f32], rate: f32) {
        out.iter_mut().for_each(|s| *s = 0.0);
        let step = RATE / rate as f64;
        let attack = 1.0 / (0.004 * rate);
        let fade = 0.001f32.powf(1.0 / (0.12 * rate));
        self.voices.retain_mut(|v| {
            let d = &v.data[..];
            let last = d.len().saturating_sub(1);
            let (mut pos, mut env) = (v.pos, v.env);
            for o in out.iter_mut() {
                let p = pos as usize;
                if p >= last {
                    return false;
                }
                if v.released {
                    env *= fade;
                    if env < 0.0005 {
                        return false;
                    }
                } else if env < 1.0 {
                    env = (env + attack).min(1.0);
                }
                let f = (pos - p as f64) as f32;
                let a = d[p] as f32;
                *o += (a + (d[p + 1] as f32 - a) * f) * env;
                pos += step;
            }
            v.pos = pos;
            v.env = env;
            true
        });
        let mut loud = 0.0f32;
        for s in out.iter_mut() {
            let x = *s * 0.3 / 32768.0;
            let a = x.abs();
            *s = if a > 0.6 { x.signum() * (0.6 + 0.4 * ((a - 0.6) / 0.4).tanh()) } else { x };
            loud = loud.max(s.abs());
        }
        self.ring.take(out, rate, loud > 0.002);
    }
}

fn fold(mut n: u8) -> u8 {
    while n < LOW {
        n += 12;
    }
    while n > HIGH {
        n -= 12;
    }
    n
}

/// The Mellotron playing now (none: off).
pub struct Mellotron {
    engine: Arc<Mutex<Engine>>,
    _stream: cpal::Stream,
}

fn current() -> &'static Mutex<Option<Arc<Mutex<Engine>>>> {
    static C: OnceLock<Mutex<Option<Arc<Mutex<Engine>>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// This computer's own sound: the default output, unless that's the FM-1 itself (then the first other one).
/// FM1_MELLOTRON_OUTPUT=name picks another output by (part of) its name, e.g. a loopback device for recording a
/// demo, without changing the computer's sound settings.
fn output() -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Ok(want) = std::env::var("FM1_MELLOTRON_OUTPUT") {
        let want = want.to_lowercase();
        if let Some(d) = host.output_devices().ok()?.find(|d| d.name().is_ok_and(|n| n.to_lowercase().contains(&want))) {
            return Some(d);
        }
    }
    let fm1 = |d: &cpal::Device| d.name().is_ok_and(|n| crate::scope::is_fm1(&n));
    match host.default_output_device() {
        Some(d) if !fm1(&d) => Some(d),
        _ => host.output_devices().ok()?.find(|d| !fm1(d)),
    }
}

impl Mellotron {
    /// Starts `sound` (one of SOUNDS): its strips load on a thread of their own (from the cache when used before).
    pub fn start(sound: &'static str) -> Option<Mellotron> {
        let device = output()?;
        let config = device.default_output_config().ok()?;
        let (rate, channels) = (config.sample_rate().0 as f32, config.channels() as usize);
        let engine = Arc::new(Mutex::new(Engine { sound, strips: None, voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default() }));
        let e = engine.clone();
        std::thread::spawn(move || {
            let s = strips(sound);
            e.lock().unwrap().strips = Some(s);
        });
        let e = engine.clone();
        let mut mono = Vec::new();
        let mut render = move |frames: usize| -> Vec<f32> {
            mono.resize(frames, 0.0);
            if let Ok(mut en) = e.lock() {
                en.mix(&mut mono, rate);
            }
            mono.clone()
        };
        let err = |e| eprintln!("mellotron: {e}");
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(&stream_config, move |d: &mut [f32], _: &_| {
                let m = render(d.len() / channels);
                for (f, s) in d.chunks_mut(channels).zip(m) { f.iter_mut().for_each(|x| *x = s); }
            }, err, None),
            cpal::SampleFormat::I16 => device.build_output_stream(&stream_config, move |d: &mut [i16], _: &_| {
                let m = render(d.len() / channels);
                for (f, s) in d.chunks_mut(channels).zip(m) { f.iter_mut().for_each(|x| *x = (s * 32767.0) as i16); }
            }, err, None),
            cpal::SampleFormat::I32 => device.build_output_stream(&stream_config, move |d: &mut [i32], _: &_| {
                let m = render(d.len() / channels);
                for (f, s) in d.chunks_mut(channels).zip(m) { f.iter_mut().for_each(|x| *x = (s as f64 * 2147483647.0) as i32); }
            }, err, None),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        *current().lock().unwrap() = Some(engine.clone());
        Some(Mellotron { engine, _stream: stream })
    }
}

impl Drop for Mellotron {
    fn drop(&mut self) {
        let mut c = current().lock().unwrap();
        if c.as_ref().is_some_and(|e| Arc::ptr_eq(e, &self.engine)) {
            *c = None;
        }
    }
}

/// The FM-1's MIDI (whole or partial messages; its screen's SysEx is skipped): played while the Mellotron is on.
pub fn feed(m: &[u8]) {
    let e = current().lock().unwrap().clone();
    if let Some(e) = e {
        let mut en = e.lock().unwrap();
        let before = en.voices.len();
        en.parse(m);
        if en.voices.len() > before && en.player != "FM-1" {
            en.player = "FM-1".to_string();
        }
    }
}

/// The Mellotron is on.
pub fn is_on() -> bool {
    current().lock().unwrap().is_some()
}

/// A keyboard's MIDI (the one FM-1 Big Screen passes on to the FM-1): it plays the Mellotron too.
pub fn feed_keys(name: &str, m: &[u8]) {
    let e = current().lock().unwrap().clone();
    if let Some(e) = e {
        let mut en = e.lock().unwrap();
        if m.iter().any(|&b| b & 0xf0 == 0x90) && en.player != name {
            en.player = name.to_string();
        }
        en.parse_from(true, m);
    }
}

/// For the spyglass: who plays what ("FM-1 → Mellotron flute", "Keystation 49 MK3 → Mellotron strings").
pub fn label() -> Option<String> {
    let e = current().lock().unwrap().clone()?;
    let en = e.lock().unwrap();
    Some(format!("{} \u{2192} Mellotron {}", en.player, en.sound))
}

/// For the spyglass: the Mellotron's real sound while it plays.
pub fn wave(hz: Option<f64>) -> Option<Vec<f32>> {
    let e = current().lock().unwrap().clone()?;
    let en = e.lock().unwrap();
    if !en.ring.listening() {
        return None;
    }
    en.ring.wave(hz?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_fold_into_the_35_keys() {
        assert_eq!(fold(31), 43);
        assert_eq!(fold(60), 60);
        assert_eq!(fold(90), 78 - 12);
    }

    #[test]
    fn every_strip_decodes_to_8_seconds() {
        for s in SOUNDS {
            let set = strips(s);
            for k in LOW..=HIGH {
                let n = set[k as usize].as_ref().map_or(0, |v| v.len());
                assert!(n > 44100 * 7, "{s} {k}: {n} samples");
            }
        }
    }

    #[test]
    fn screen_sysex_between_notes_makes_no_notes() {
        let mut e = Engine { sound: "flute", strips: Some(strips("flute")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default() };
        // a note, then a screen message whose data looks like notes, then the note let go
        e.parse(&[0x90, 60, 100]);
        e.parse(&[0xf0, 0x7d, 0x46, 0x53, 0x01, 60, 0, 61, 100, 62]);
        e.parse(&[0x40, 0xf7]);
        assert_eq!(e.voices.len(), 1);
        assert!(!e.voices[0].released);
        e.parse(&[0x80, 60, 0]);
        assert!(e.voices[0].released);
    }

    #[test]
    fn a_keyboard_and_the_fm1_dont_mix_their_bytes() {
        let mut e = Engine { sound: "flute", strips: Some(strips("flute")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default() };
        e.parse_from(false, &[0xf0, 0x7d, 0x46]);     // the FM-1 is in the middle of a screen message
        e.parse_from(true, &[0x90, 64, 90]);         // a keyboard key
        e.parse_from(false, &[61, 100, 62, 0xf7]);   // the rest of the screen message: no notes
        assert_eq!(e.voices.len(), 1);
        assert_eq!(e.voices[0].key, 64);
    }

    #[test]
    fn a_chord_stays_clean() {
        let mut e = Engine { sound: "strings", strips: Some(strips("strings")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default() };
        e.parse(&[0x90, 48, 100, 0x90, 52, 100, 0x90, 55, 100, 0x90, 60, 100]);
        let mut out = vec![0.0f32; 48000];
        e.mix(&mut out, 48000.0);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05 && peak < 0.95, "peak {peak}");
    }
}
