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

/// The full version's built-in sounds, in the order the button steps through them (the lite version has none built
/// in; any sound can be downloaded from the list online, INDEX).
pub const BUILT_IN: [&str; 3] = ["flute", "strings", "choir"];

/// The list of sounds online (in the Android app's repo, shared by every version): each a zip of 35 strips on a
/// GitHub release. New sounds are added there, so the apps get them without an update.
pub const INDEX: &str = "https://raw.githubusercontent.com/pfkellogg/fm1-mirror-spyglass-android/main/sounds/sounds.json";

/// Windows and Linux: the strips built into the program.
#[cfg(all(not(target_os = "macos"), feature = "bundled-sounds"))]
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

/// A built-in strip's FLAC (the full version): on a Mac from the app's Resources (make_app.sh copies them there once,
/// rather than into each of the program's two halves, Apple silicon and Intel), else from the program itself.
fn bundled_flac(sound: &str, key: u8) -> Option<std::borrow::Cow<'static, [u8]>> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok()?;
        let name = format!("mellotron/{sound}/{key:03}.flac");
        let res = exe.parent()?.parent()?.join("Resources").join(&name);
        if let Ok(b) = std::fs::read(res) {
            return Some(std::borrow::Cow::Owned(b));
        }
        #[cfg(feature = "bundled-sounds")]
        {
            let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(&name);   // (cargo run, tests)
            if let Ok(b) = std::fs::read(dev) {
                return Some(std::borrow::Cow::Owned(b));
            }
        }
        None
    }
    #[cfg(all(not(target_os = "macos"), feature = "bundled-sounds"))]
    {
        let files = FILES.iter().find(|(n, _)| *n == sound).map(|(_, f)| f)?;
        files.get((key - LOW) as usize).map(|b| std::borrow::Cow::Borrowed(*b))
    }
    #[cfg(all(not(target_os = "macos"), not(feature = "bundled-sounds")))]
    {
        let _ = (sound, key);
        None
    }
}

/// Where downloaded sounds are kept: next to the settings, one folder each.
pub(crate) fn sounds_dir() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join(crate::SETTINGS_DIR).join("sounds"))
}

fn flac(sound: &str, key: u8) -> Option<std::borrow::Cow<'static, [u8]>> {
    bundled_flac(sound, key).or_else(|| {
        std::fs::read(sounds_dir()?.join(sound).join(format!("{key:03}.flac"))).ok().map(std::borrow::Cow::Owned)
    })
}

/// Built into this version.
pub fn bundled(id: &str) -> bool {
    bundled_flac(id, LOW).is_some()
}

/// Downloaded to this computer.
pub fn downloaded(id: &str) -> bool {
    sounds_dir().is_some_and(|d| d.join(id).join("043.flac").is_file() && d.join(id).join("077.flac").is_file())
}

/// The sounds this computer has (built in or downloaded): the built-in order first, then the others by name.
pub fn installed() -> Vec<String> {
    let mut ids: Vec<String> = BUILT_IN.iter().filter(|s| bundled(s) || downloaded(s)).map(|s| s.to_string()).collect();
    if crate::rich::downloaded() {
        ids.push(crate::rich::ID.to_string());       // Rich: GeneralUser GS, the closest instrument to the FM-1's sound
    }
    let mut more: Vec<String> = sounds_dir()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|r| r.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| !n.ends_with(".part") && n != crate::rich::ID && downloaded(n) && !ids.contains(n)).collect())
        .unwrap_or_default();
    more.sort();
    ids.extend(more);
    ids
}

/// Changes when a sound does (downloaded again): part of its cache key.
fn stamp(id: &str) -> String {
    if bundled(id) {
        return "app".into();
    }
    sounds_dir()
        .and_then(|d| std::fs::metadata(d.join(id).join("077.flac")).ok()?.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or("dl".into(), |t| format!("dl{}", t.as_secs()))
}

type Set = Arc<Vec<Option<Arc<Vec<i16>>>>>;

/// Sounds decoded before (a second or so each), kept for switching back instantly.
fn cache() -> &'static Mutex<HashMap<String, Set>> {
    static C: OnceLock<Mutex<HashMap<String, Set>>> = OnceLock::new();
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

fn strips(sound: &str) -> Set {
    let key = format!("{sound}/{}", stamp(sound));
    if let Some(s) = cache().lock().unwrap().get(&key) {
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
    cache().lock().unwrap().insert(key, set.clone());
    set
}

// ---- the list of sounds online, and downloads

/// A sound on the list online.
#[derive(Clone)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The list of sounds online (network: call it off the window's thread).
pub fn list() -> Result<Vec<Pack>, String> {
    let body = ureq::get(INDEX).set("User-Agent", "FM-1-Big-Screen").call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|_| "the list of sounds can't be read".to_string())?;
    let ok = |id: &str| !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    Ok(v["sounds"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let id = o["id"].as_str()?.to_string();
                    ok(&id).then(|| Pack {
                        name: o["name"].as_str().unwrap_or(&id).to_string(),
                        url: o["url"].as_str().unwrap_or_default().to_string(),
                        sha256: o["sha256"].as_str().unwrap_or_default().to_string(),
                        bytes: o["bytes"].as_u64().unwrap_or(0),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Downloads a sound, checks it, and puts it in place (network and disk: off the window's thread).
pub fn download(p: &Pack) -> Result<(), String> {
    use sha2::Digest;
    use std::io::Read;
    let mut zip = Vec::new();
    ureq::get(&p.url).set("User-Agent", "FM-1-Big-Screen").call().map_err(|e| e.to_string())?
        .into_reader().take(200 << 20).read_to_end(&mut zip).map_err(|e| e.to_string())?;
    if !p.sha256.is_empty() && !p.sha256.eq_ignore_ascii_case(&format!("{:x}", sha2::Sha256::digest(&zip))) {
        return Err("the download was damaged".into());
    }
    let dir = sounds_dir().ok_or("no folder for sounds")?;
    let part = dir.join(format!("{}.part", p.id));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    let mut n = 0;
    for i in 0..a.len() {
        let mut f = a.by_index(i).map_err(|e| e.to_string())?;
        // just the strips: no folders, no paths
        let name = std::path::Path::new(f.name()).file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let strip = name.len() == 8 && name.ends_with(".flac") && name[..3].chars().all(|c| c.is_ascii_digit());
        if f.is_dir() || !strip {
            continue;
        }
        let mut out = std::fs::File::create(part.join(&name)).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        n += 1;
    }
    if n < 35 {
        let _ = std::fs::remove_dir_all(&part);
        return Err(format!("the sound was incomplete ({n} of 35 strips)"));
    }
    let dest = dir.join(&p.id);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove(id: &str) {
    if let Some(d) = sounds_dir() {
        let _ = std::fs::remove_dir_all(d.join(id));
    }
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
    sound: String,
    strips: Option<Set>,
    voices: Vec<Voice>,
    fm1: Parser,
    keys: Parser,
    /// Who played the last note ("FM-1", or the keyboard's name), for the spyglass.
    player: String,
    /// The last moment of what it played, for the spyglass's waveform.
    ring: crate::scope::Ring,
    /// Rich instead of strips: GeneralUser GS (rich.rs)
    rich: Option<crate::rich::Rich>,
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
                0x90 if b > 0 => self.note_on(d1, b),
                0x80 | 0x90 => self.note_off(d1),
                0xb0 if d1 == 120 || d1 == 123 => {
                    self.voices.iter_mut().for_each(|v| v.released = true);
                    if let Some(r) = &mut self.rich {
                        r.all_off();
                    }
                }
                _ => {}
            }
        }
        if keyboard { self.keys = p } else { self.fm1 = p }
    }

    fn parse(&mut self, m: &[u8]) {
        self.parse_from(false, m);
    }

    fn note_on(&mut self, note: u8, vel: u8) {
        if let Some(r) = &mut self.rich {
            r.note_on(note, vel);                    // Rich: every key as played, with its velocity
            return;
        }
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
        if let Some(r) = &mut self.rich {
            r.note_off(note);
            return;
        }
        let key = fold(note);
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.released = true;
        }
    }

    /// `out` mono at `rate`: the strips (44.1 kHz) stepped to it, a 4 ms start, 0.12 s fade when let go, headroom for
    /// chords and only squeezed near the top (Android 1.6.9's fixes).
    fn mix(&mut self, out: &mut [f32], rate: f32) {
        if let Some(r) = &mut self.rich {
            r.render(out);                           // Rich: the SoundFont's own level, squeezed only near the top
            let mut loud = 0.0f32;
            for s in out.iter_mut() {
                let a = s.abs();
                *s = if a > 0.8 { s.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh()) } else { *s };
                loud = loud.max(s.abs());
            }
            self.ring.take(out, rate, loud > 0.002);
            return;
        }
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
    pub fn start(sound: &str) -> Option<Mellotron> {
        let device = output()?;
        let config = device.default_output_config().ok()?;
        let (rate, channels) = (config.sample_rate().0 as f32, config.channels() as usize);
        let engine = Arc::new(Mutex::new(Engine { sound: sound.to_string(), strips: None, voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default(), rich: None }));
        let e = engine.clone();
        let name = sound.to_string();
        std::thread::spawn(move || {
            if name == crate::rich::ID {
                let r = crate::rich::Rich::new(rate as u32);       // (the SoundFont read once: ~1 s the first time)
                e.lock().unwrap().rich = r;
                return;
            }
            let s = strips(&name);
            e.lock().unwrap().strips = Some(s);
        });
        let e = engine.clone();
        let mut mono = Vec::new();
        let mut sweet = crate::sweeten::Sweeten::new(rate);
        // the mix, then Sweeten (stereo; straight through while it's off): left and right per frame
        let mut render = move |frames: usize| -> Vec<(f32, f32)> {
            mono.resize(frames, 0.0);
            if let Ok(mut en) = e.lock() {
                en.mix(&mut mono, rate);
            }
            mono.iter().map(|&x| sweet.run(x)).collect()
        };
        // a frame's channels: left, right, and any more the average (a mono output: the average)
        let put = move |f: &mut [f32], (l, r): (f32, f32)| match f.len() {
            1 => f[0] = (l + r) * 0.5,
            _ => f.iter_mut().enumerate().for_each(|(i, x)| *x = match i { 0 => l, 1 => r, _ => (l + r) * 0.5 }),
        };
        let err = |e| eprintln!("mellotron: {e}");
        let stream_config: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(&stream_config, move |d: &mut [f32], _: &_| {
                let m = render(d.len() / channels);
                for (f, s) in d.chunks_mut(channels).zip(m) { put(f, s); }
            }, err, None),
            cpal::SampleFormat::I16 => device.build_output_stream(&stream_config, move |d: &mut [i16], _: &_| {
                let m = render(d.len() / channels);
                let mut tmp = vec![0.0f32; channels];
                for (f, s) in d.chunks_mut(channels).zip(m) { put(&mut tmp, s); f.iter_mut().zip(&tmp).for_each(|(x, v)| *x = (v * 32767.0) as i16); }
            }, err, None),
            cpal::SampleFormat::I32 => device.build_output_stream(&stream_config, move |d: &mut [i32], _: &_| {
                let m = render(d.len() / channels);
                let mut tmp = vec![0.0f32; channels];
                for (f, s) in d.chunks_mut(channels).zip(m) { put(&mut tmp, s); f.iter_mut().zip(&tmp).for_each(|(x, v)| *x = (*v as f64 * 2147483647.0) as i32); }
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
    if en.sound == crate::rich::ID {
        return Some(format!("{} \u{203A} Rich {}", en.player, crate::rich::GM[crate::rich::program() as usize]));
    }
    Some(format!("{} \u{203A} Mellotron {}", en.player, en.sound))
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

#[cfg(all(test, feature = "bundled-sounds"))]
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
        for s in BUILT_IN {
            let set = strips(s);
            for k in LOW..=HIGH {
                let n = set[k as usize].as_ref().map_or(0, |v| v.len());
                assert!(n > 44100 * 7, "{s} {k}: {n} samples");
            }
        }
    }

    #[test]
    fn screen_sysex_between_notes_makes_no_notes() {
        let mut e = Engine { sound: "flute".into(), strips: Some(strips("flute")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default(), rich: None };
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
        let mut e = Engine { sound: "flute".into(), strips: Some(strips("flute")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default(), rich: None };
        e.parse_from(false, &[0xf0, 0x7d, 0x46]);     // the FM-1 is in the middle of a screen message
        e.parse_from(true, &[0x90, 64, 90]);         // a keyboard key
        e.parse_from(false, &[61, 100, 62, 0xf7]);   // the rest of the screen message: no notes
        assert_eq!(e.voices.len(), 1);
        assert_eq!(e.voices[0].key, 64);
    }

    #[test]
    fn a_chord_stays_clean() {
        let mut e = Engine { sound: "strings".into(), strips: Some(strips("strings")), voices: Vec::new(), fm1: Parser::default(), keys: Parser::default(), player: "FM-1".to_string(), ring: Default::default(), rich: None };
        e.parse(&[0x90, 48, 100, 0x90, 52, 100, 0x90, 55, 100, 0x90, 60, 100]);
        let mut out = vec![0.0f32; 48000];
        e.mix(&mut out, 48000.0);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05 && peak < 0.95, "peak {peak}");
    }
}
