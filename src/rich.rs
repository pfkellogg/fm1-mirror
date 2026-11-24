//! Rich: a rich version of the FM-1's sound, out of this computer, like the Mellotron (the FM-1's 1 MB of flash has no
//! room for sounds like these). The FM-1 says which sound it plays (its name, screen.rs SOUND); the closest of the 128
//! General MIDI instruments in GeneralUser GS (S. Christian Collins, github.com/mrbumpy409/GeneralUser-GS: a free
//! SoundFont, its license allows apps to use it) plays what the FM-1's keys and a keyboard play, through rustysynth
//! (MIT). "Auto" follows the FM-1's sound; any instrument can be picked instead. The SoundFont (32 MB) is downloaded
//! once, from its author's repository, into the Mellotron sounds' folder.
use std::sync::{Arc, Mutex, OnceLock};

pub const ID: &str = "rich";
pub const URL: &str = "https://github.com/mrbumpy409/GeneralUser-GS/raw/main/GeneralUser-GS.sf2";
pub const SIZE_MB: u32 = 32;
const FILE: &str = "GeneralUser-GS.sf2";

/// The 128 General MIDI instruments, by program number.
pub const GM: [&str; 128] = [
    "Grand Piano", "Bright Piano", "Electric Grand", "Honky-tonk", "Electric Piano", "Electric Piano 2", "Harpsichord", "Clavinet",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica", "Bandoneon",
    "Nylon Guitar", "Steel Guitar", "Jazz Guitar", "Clean Guitar", "Muted Guitar", "Overdrive Guitar", "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Finger Bass", "Pick Bass", "Fretless Bass", "Slap Bass", "Slap Bass 2", "Synth Bass", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Harp", "Timpani",
    "Strings", "Slow Strings", "Synth Strings", "Synth Strings 2", "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet", "French Horn", "Brass", "Synth Brass", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Square Lead", "Saw Lead", "Calliope", "Chiff Lead", "Charang", "Voice Lead", "Fifths Lead", "Bass + Lead",
    "New Age Pad", "Warm Pad", "Polysynth", "Choir Pad", "Bowed Pad", "Metal Pad", "Halo Pad", "Sweep Pad",
    "Rain", "Soundtrack", "Crystal", "Atmosphere", "Brightness", "Goblins", "Echoes", "Sci-fi",
    "Sitar", "Banjo", "Shamisen", "Koto", "Kalimba", "Bagpipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock", "Taiko", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Fret Noise", "Breath", "Seashore", "Bird", "Telephone", "Helicopter", "Applause", "Gunshot",
];

/// Words in an FM-1 sound's name (or a firmware's) and the instrument they mean: the first that matches, longest words
/// first so "E.PIANO" is an electric piano, not a piano.
const WORDS: &[(&str, u8)] = &[
    ("electric piano", 4), ("e.piano", 4), ("epiano", 4), ("e piano", 4), ("rhodes", 4), ("wurli", 5), ("dx piano", 5), ("tine", 4),
    ("grand", 0), ("piano", 0), ("upright", 0), ("honky", 3), ("harpsi", 6), ("clav", 7), ("celest", 8), ("glock", 9),
    ("music box", 10), ("musicbox", 10), ("vibra", 11), ("marimba", 12), ("xylo", 13), ("tubular", 14), ("bell", 14), ("chime", 14),
    ("dulcimer", 15), ("drawbar", 16), ("b3", 16), ("hammond", 16), ("perc org", 17), ("rock org", 18), ("church", 19), ("pipe org", 19),
    ("organ", 16), ("accordion", 21), ("harmonica", 22), ("bandoneon", 23), ("nylon", 24), ("acoustic gt", 25), ("guitar", 25), ("jazz gt", 26),
    ("fretless", 35), ("slap", 36), ("acid", 38), ("303", 38), ("sub bass", 38), ("synth bass", 38), ("bass", 33),
    ("violin", 40), ("viola", 41), ("cello", 42), ("contrabass", 43), ("trem str", 44), ("pizz", 45), ("harp", 46), ("timpani", 47),
    ("slow str", 49), ("syn str", 50), ("synth str", 50), ("ensemble", 48), ("string", 48), ("choir", 52), ("aah", 52), ("ooh", 53), ("vox", 54), ("voice", 53),
    ("orch hit", 55), ("trumpet", 56), ("trombone", 57), ("tuba", 58), ("mute tp", 59), ("horn", 60), ("syn brass", 62), ("brass", 61),
    ("soprano", 64), ("alto", 65), ("tenor", 66), ("sax", 66), ("bari", 67), ("oboe", 68), ("english h", 69), ("bassoon", 70), ("clarinet", 71),
    ("piccolo", 72), ("flute", 73), ("recorder", 74), ("pan", 75), ("bottle", 76), ("shakuhachi", 77), ("whistle", 78), ("ocarina", 79),
    ("square", 80), ("chip", 80), ("sid", 80), ("8-bit", 80), ("saw", 81), ("lead", 81), ("calliope", 82), ("chiff", 83), ("charang", 84),
    ("fifth", 86), ("new age", 88), ("warm", 89), ("polysynth", 90), ("poly", 90), ("pad", 89), ("halo", 94), ("sweep", 95), ("drone", 89), ("ambient", 88),
    ("rain", 96), ("soundtrack", 97), ("crystal", 98), ("atmos", 99), ("bright", 100), ("goblin", 101), ("echo", 102), ("sci", 103),
    ("sitar", 104), ("banjo", 105), ("shamisen", 106), ("koto", 107), ("kalimba", 108), ("mbira", 108), ("bagpipe", 109), ("fiddle", 110), ("shanai", 111),
    ("steel drum", 114), ("taiko", 116), ("tom", 117), ("synth drum", 118), ("mellotron", 49),
    // fm1-dock's synths (its list's names) and other firmwares' own names (last: an instrument's word wins)
    ("b3 organ", 16), ("cs-1", 50), ("scintilla", 90), ("synthex", 90), ("obervave", 90), ("prophet", 90), ("s1vave", 90),
    ("monk", 52), ("fomni", 46), ("omnichord", 46), ("fimba", 108), ("gator", 81), ("flat-20", 81), ("ms-20", 81),
    ("niagara", 89), ("eminent", 50), ("artihive", 90), ("fmini", 5), ("fm6", 5), ("fm-106", 4), ("atek303", 38), ("dub-1", 38),
    ("amb-1", 89), ("keys", 0), ("minipops", 118), ("sloop", 0), ("melodee", 89), ("erik37", 0), ("loom", 48), ("polly", 0),
    ("synth", 90),
];

/// The instrument for an FM-1 sound's name: a word in it, else strings (a firmware that says only its own name).
pub fn program_for(name: &str) -> u8 {
    let n = format!(" {} ", name.to_lowercase());
    // an organ's switches change its kind: overdrive (B3's OD) a rock organ, percussion (PERC) a percussive one
    if n.contains("organ") || n.contains(" b3") {
        if n.contains(" od ") || n.contains("overdrive") {
            return 18;
        }
        if n.contains(" perc") {
            return 17;
        }
    }
    WORDS.iter().find(|(w, _)| n.contains(*w)).map_or(48, |&(_, p)| p)
}

/// "Auto" (None: follow the FM-1's sound) or a picked instrument; and the FM-1's sound now. Remembered by main.rs.
pub static PICK: Mutex<Option<u8>> = Mutex::new(None);
pub static FM1_SOUND: Mutex<String> = Mutex::new(String::new());

/// The instrument playing now.
pub fn program() -> u8 {
    PICK.lock().unwrap().unwrap_or_else(|| program_for(&FM1_SOUND.lock().unwrap()))
}

fn path() -> Option<std::path::PathBuf> {
    Some(crate::mellotron::sounds_dir()?.join(ID).join(FILE))
}

/// GeneralUser GS is on this computer.
pub fn downloaded() -> bool {
    path().is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 1_000_000))
}

/// Downloads GeneralUser GS from its author's repository (32 MB), into a .part file first.
pub fn download() -> Result<(), String> {
    let p = path().ok_or("no folder for sounds")?;
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = p.with_extension("part");
    let r = ureq::get(URL).call().map_err(|e| format!("couldn't download GeneralUser GS: {e}"))?;
    let mut f = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    std::io::copy(&mut r.into_reader(), &mut f).map_err(|e| e.to_string())?;
    drop(f);
    let ok = std::fs::metadata(&part).is_ok_and(|m| m.len() > 1_000_000)
        && rustysynth::SoundFont::new(&mut std::fs::File::open(&part).map_err(|e| e.to_string())?).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&part);
        return Err("the download wasn't a SoundFont".into());
    }
    std::fs::rename(&part, &p).map_err(|e| e.to_string())
}

pub fn remove() {
    if let Some(p) = path() {
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}

/// The SoundFont, read once.
fn font() -> Option<Arc<rustysynth::SoundFont>> {
    static F: OnceLock<Mutex<Option<Arc<rustysynth::SoundFont>>>> = OnceLock::new();
    let m = F.get_or_init(Default::default);
    let mut g = m.lock().unwrap();
    if g.is_none() {
        let mut f = std::fs::File::open(path()?).ok()?;
        *g = Some(Arc::new(rustysynth::SoundFont::new(&mut f).ok()?));
    }
    g.clone()
}

/// The synthesizer the Mellotron engine plays instead of its strips while Rich is the sound.
pub struct Rich {
    synth: rustysynth::Synthesizer,
    program: u8,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Rich {
    pub fn new(rate: u32) -> Option<Rich> {
        let mut settings = rustysynth::SynthesizerSettings::new(rate as i32);
        settings.maximum_polyphony = 48;
        let synth = rustysynth::Synthesizer::new(&font()?, &settings).ok()?;
        let mut r = Rich { synth, program: 255, left: Vec::new(), right: Vec::new() };
        r.follow();
        Some(r)
    }

    /// The instrument "Auto" or the picked one says, when it changed.
    fn follow(&mut self) {
        let p = program();
        if p != self.program {
            self.program = p;
            self.synth.process_midi_message(0, 0xC0, p as i32, 0);
        }
    }

    pub fn note_on(&mut self, key: u8, vel: u8) {
        self.follow();
        self.synth.note_on(0, key as i32, vel.max(1) as i32);
    }

    pub fn note_off(&mut self, key: u8) {
        self.synth.note_off(0, key as i32);
    }

    pub fn all_off(&mut self) {
        self.synth.note_off_all(false);
    }

    /// `out` mono (the Mellotron's mix is mono; Sweeten makes it stereo).
    pub fn render(&mut self, out: &mut [f32]) {
        self.left.resize(out.len(), 0.0);
        self.right.resize(out.len(), 0.0);
        self.synth.render(&mut self.left, &mut self.right);
        for (o, (l, r)) in out.iter_mut().zip(self.left.iter().zip(&self.right)) {
            *o = (l + r) * 0.5 * 6.0;                // GeneralUser GS leaves ~20 dB of headroom: up to the Mellotron's level
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pick_instruments() {
        assert_eq!(program_for("T1 FM6 ICE BELL"), 14);
        assert_eq!(program_for("I FLUTE"), 73);
        assert_eq!(program_for("II ORGAN 2"), 16);
        assert_eq!(program_for("TINE EP"), 4);
        assert_eq!(program_for("E.PIANO 1"), 4);
        assert_eq!(program_for("PIANO 1"), 0);
        assert_eq!(program_for("ACID BASS"), 38);
        assert_eq!(program_for("B3 ORGAN"), 16);
        assert_eq!(program_for("B3 ORGAN  Gospel PERC"), 17);
        assert_eq!(program_for("B3 ORGAN  Gospel PERC OD"), 18);
        assert_eq!(program_for("CS-1"), 50);
        assert_eq!(program_for("OBERVAVE"), 90);
        assert_eq!(program_for("FM1-DOCK 0.9.0"), 48);
        assert_eq!(program_for("FELUCCA 1.8.3"), 48);
    }
}

#[cfg(test)]
mod sound_tests {
    use super::*;

    /// With GeneralUser GS downloaded: a flute C5 sounds (and goes quiet after its release).
    #[test]
    fn plays_when_downloaded() {
        if !downloaded() {
            return;
        }
        *PICK.lock().unwrap() = Some(73);
        let mut r = Rich::new(44100).expect("the SoundFont loads");
        r.note_on(72, 100);
        let mut b = vec![0f32; 22050];
        r.render(&mut b);
        let rms = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
        println!("flute C5 rms {rms:.4}");
        assert!(rms > 0.01, "silent: {rms}");
        r.note_off(72);
        for _ in 0..4 {
            r.render(&mut b);
        }
        let tail = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
        assert!(tail < rms / 4.0, "doesn't stop: {tail}");
        *PICK.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod level_tests {
    use super::*;

    #[test]
    #[ignore]
    fn levels() {
        if !downloaded() {
            return;
        }
        for (p, name) in [(89u8, "warm pad"), (73, "flute"), (0, "piano"), (16, "organ"), (48, "strings"), (4, "e.piano")] {
            *PICK.lock().unwrap() = Some(p);
            let mut r = Rich::new(44100).unwrap();
            for k in [60u8, 64, 67] {
                r.note_on(k, 100);
            }
            let mut b = vec![0f32; 44100];
            r.render(&mut b);
            let peak = b.iter().fold(0f32, |m, x| m.max(x.abs()));
            let rms = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
            println!("{name:10} chord: peak {peak:.3} rms {rms:.3} ({:.1} dBFS rms)", 20.0 * rms.log10());
        }
        *PICK.lock().unwrap() = None;
    }
}
