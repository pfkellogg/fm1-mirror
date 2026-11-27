#![cfg_attr(not(feature = "debugger"), allow(dead_code))]
//! The MIDI debugger (⌘⇧D / Ctrl+Shift+D, in the "FM-1 Big Screen Debugger" build only: `--features debugger`): every message between the app and each FM-1, decoded, with breakpoints
//! that really stop the app.
//!
//! - The input callback (open()) and each link's hello thread (hellos()) call `check` with every message before the
//!   app acts on it / before it is sent. A breakpoint that matches parks that thread right there: the app has not
//!   handled the message yet, and nothing after it is handled either, until Continue, a Step or Skip.
//! - The UI thread never stops (the debugger's own window runs on it): messages the UI sends (Big Knobs, presses)
//!   are logged, and a breakpoint on them is reported as a hit without stopping.
//! - Breakpoints: a kind of message, or bytes that start with a pattern (`F0 7D 46 53 05 ??`), optionally only one
//!   FM-1's port, only from or only to the FM-1, after the first N hits; "log only" ones just mark the log.

use eframe::egui;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(false);
static DBG: LazyLock<(Mutex<Inner>, Condvar)> = LazyLock::new(|| (Mutex::new(Inner::default()), Condvar::new()));
static T0: LazyLock<Instant> = LazyLock::new(Instant::now);
const LOG_MAX: usize = 4000;
const KEEP_BYTES: usize = 160;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Dir {
    In,
    Out,
}

/// What a message is (`classify`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Kind {
    HelloV1,
    HelloV2,
    BaseRequest,
    AskInfo,
    AskIdentity,
    AskBuild,
    FirmwareInfo,
    Identity,
    Build,
    V2Info,
    BaseHello,
    ScreenRow,
    V2Rect,
    V2Sync,
    BaseRect,
    BaseFill,
    SoundName,
    Bpm,
    KeyLights,
    BaseInput,
    KnobTurn,
    Master,
    Press,
    BaseRemote,
    Updater,
    NoteOn,
    NoteOff,
    Control,
    PitchBend,
    Clock,
    Transport,
    OtherSysex,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 33] = [
        Kind::HelloV1, Kind::HelloV2, Kind::BaseRequest, Kind::AskInfo, Kind::AskIdentity, Kind::AskBuild,
        Kind::FirmwareInfo, Kind::Identity, Kind::Build, Kind::V2Info, Kind::BaseHello, Kind::ScreenRow, Kind::V2Rect,
        Kind::V2Sync, Kind::BaseRect, Kind::BaseFill, Kind::SoundName, Kind::Bpm, Kind::KeyLights, Kind::BaseInput,
        Kind::KnobTurn, Kind::Master, Kind::Press, Kind::BaseRemote, Kind::Updater, Kind::NoteOn, Kind::NoteOff,
        Kind::Control, Kind::PitchBend, Kind::Clock, Kind::Transport, Kind::OtherSysex, Kind::Other,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Kind::HelloV1 => "Hello (Big Screen 1)",
            Kind::HelloV2 => "Hello (Big Screen 2)",
            Kind::BaseRequest => "Hello (fm1-base mirror)",
            Kind::AskInfo => "Which firmware? (ask)",
            Kind::AskIdentity => "Which FM-1? (ask)",
            Kind::AskBuild => "Which build? (ask)",
            Kind::FirmwareInfo => "Firmware says hello: its name",
            Kind::Identity => "FM-1 identity",
            Kind::Build => "Firmware build",
            Kind::V2Info => "Firmware says hello (Big Screen 2)",
            Kind::BaseHello => "Firmware says hello (fm1-base mirror)",
            Kind::ScreenRow => "Screen row",
            Kind::V2Rect => "Screen rectangle (BS 2)",
            Kind::V2Sync => "Frame done (BS 2)",
            Kind::BaseRect => "Screen rectangle (fm1-base)",
            Kind::BaseFill => "Screen fill (fm1-base)",
            Kind::SoundName => "Sound name",
            Kind::Bpm => "BPM",
            Kind::KeyLights => "Key Lights",
            Kind::BaseInput => "Panel and lights (fm1-base)",
            Kind::KnobTurn => "Big Knobs: turn",
            Kind::Master => "Big Knobs: MASTER",
            Kind::Press => "Remote press",
            Kind::BaseRemote => "Remote (fm1-base)",
            Kind::Updater => "Updater message",
            Kind::NoteOn => "Note on",
            Kind::NoteOff => "Note off",
            Kind::Control => "Control change",
            Kind::PitchBend => "Pitch bend",
            Kind::Clock => "Clock",
            Kind::Transport => "Start / stop",
            Kind::OtherSysex => "Other SysEx",
            Kind::Other => "Other",
        }
    }
    /// The screen's pictures: many a second, hidden from the log unless asked for.
    pub fn picture(self) -> bool {
        matches!(self, Kind::ScreenRow | Kind::V2Rect | Kind::V2Sync | Kind::BaseRect | Kind::BaseFill | Kind::Clock)
    }
    /// The firmware answering a hello, whichever mirror it speaks.
    pub fn says_hello(self) -> bool {
        matches!(self, Kind::FirmwareInfo | Kind::V2Info | Kind::BaseHello)
    }
}

/// What a breakpoint looks for.
#[derive(Clone, PartialEq, Debug)]
pub enum What {
    Kind(Kind),
    /// The firmware answering a hello, any mirror (FirmwareInfo, V2Info, BaseHello).
    SaysHello,
    /// Bytes that start with these (None: any byte).
    Bytes(Vec<Option<u8>>),
    Any,
}

#[derive(Clone, Debug)]
pub struct Breakpoint {
    pub what: What,
    pub dir: Option<Dir>,
    pub port: String,
    pub skip_first: u64,
    pub log_only: bool,
    pub enabled: bool,
    pub hits: u64,
}

impl Breakpoint {
    pub fn new(what: What) -> Self {
        Breakpoint { what, dir: None, port: String::new(), skip_first: 0, log_only: false, enabled: true, hits: 0 }
    }
    pub fn label(&self) -> String {
        let w = match &self.what {
            What::Kind(k) => k.name().to_string(),
            What::SaysHello => "The firmware says hello (any mirror)".into(),
            What::Bytes(p) => format!("Bytes {}", p.iter().map(|b| b.map_or("??".into(), |b| format!("{b:02X}"))).collect::<Vec<_>>().join(" ")),
            What::Any => "Any message".into(),
        };
        let d = match self.dir {
            Some(Dir::In) => ", from the FM-1",
            Some(Dir::Out) => ", to the FM-1",
            None => "",
        };
        let p = if self.port.is_empty() { String::new() } else { format!(", port {}", self.port) };
        let s = if self.skip_first > 0 { format!(", after {} hits", self.skip_first) } else { String::new() };
        format!("{w}{d}{p}{s}")
    }
    fn matches(&self, ev: &Event, bytes: &[u8]) -> bool {
        if !self.enabled || self.dir.is_some_and(|d| d != ev.dir) || (!self.port.is_empty() && !ev.port.to_lowercase().contains(&self.port.to_lowercase())) {
            return false;
        }
        match &self.what {
            What::Kind(k) => ev.kind == *k,
            What::SaysHello => ev.kind.says_hello() || ev.answers_hello,
            What::Bytes(p) => bytes.len() >= p.len() && p.iter().zip(bytes).all(|(want, b)| want.is_none_or(|w| w == *b)),
            What::Any => true,
        }
    }
}

/// "F0 7D 46 53 ?? 05": hex bytes, ?? for any.
pub fn parse_pattern(s: &str) -> Option<Vec<Option<u8>>> {
    let v: Option<Vec<_>> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| if t == "??" || t == "?" { Some(None) } else { u8::from_str_radix(t.trim_start_matches("0x"), 16).ok().map(Some) })
        .collect();
    v.filter(|v| !v.is_empty())
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u64,
    pub t: f64,
    pub port: String,
    pub dir: Dir,
    pub kind: Kind,
    pub len: usize,
    pub bytes: Vec<u8>,
    /// a log-only breakpoint marked it, or a breakpoint on a message the UI sent (which never stops)
    pub mark: Option<String>,
    /// the first message from this FM-1 after a hello: its answer (a Big Screen 1 firmware answers with its screen)
    pub answers_hello: bool,
}

/// How the stopped thread goes on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    /// Run until a breakpoint.
    None,
    /// Stop at the very next message, either way.
    Any,
    /// Stop at the next message from an FM-1.
    In,
    /// Stop at the next message to an FM-1.
    Out,
    /// Stop at the next message of this kind.
    Kind(Kind),
}

#[derive(Clone, Debug)]
pub struct Stop {
    pub event: Event,
    /// all of it (the log keeps the start)
    pub full: Vec<u8>,
    pub why: String,
    pub thread: &'static str,
    pub since: Instant,
}

#[derive(Default)]
struct Inner {
    log: VecDeque<Event>,
    next: u64,
    bps: Vec<Breakpoint>,
    step: Option<Step>,
    stop: Option<Stop>,
    /// The stopped message's id, and what to do with it once released (true: skip it).
    released: Option<(u64, bool)>,
    counts: HashMap<Kind, u64>,
    ctx: Option<egui::Context>,
    last_by_port: HashMap<(String, Kind), f64>,
    /// ports the app has said hello to, not yet answered
    awaiting: std::collections::HashSet<String>,
}

/// Turn the debugger on (its window is open) or off (every stopped thread goes on at once).
pub fn set_on(on: bool, ctx: &egui::Context) {
    let (m, cv) = &*DBG;
    let mut g = m.lock().unwrap();
    g.ctx = Some(ctx.clone());
    ON.store(on, Ordering::Relaxed);
    if !on {
        if let Some(s) = g.stop.take() {
            g.released = Some((s.event.id, false));
        }
        g.step = None;
        cv.notify_all();
    }
}

pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Every message, before the app acts on it (In) or sends it (Out). `may_stop`: false on the UI thread.
/// Returns false when the message is to be skipped (the debugger's Skip).
pub fn check(port: &str, dir: Dir, bytes: &[u8], thread: &'static str, may_stop: bool) -> bool {
    if !ON.load(Ordering::Relaxed) {
        return true;
    }
    let (m, cv) = &*DBG;
    let mut g = m.lock().unwrap();
    let kind = classify(dir, bytes);
    let id = g.next;
    g.next += 1;
    let t = T0.elapsed().as_secs_f64();
    let mut ev = Event { id, t, port: port.to_string(), dir, kind, len: bytes.len(), bytes: bytes[..bytes.len().min(KEEP_BYTES)].to_vec(), mark: None, answers_hello: false };
    if dir == Dir::Out && matches!(kind, Kind::HelloV1 | Kind::HelloV2 | Kind::BaseRequest) {
        g.awaiting.insert(port.to_string());
    } else if dir == Dir::In && g.awaiting.remove(port) {
        ev.answers_hello = true;
    }
    *g.counts.entry(kind).or_default() += 1;
    // the breakpoints and the step
    let mut why = None;
    for bp in g.bps.iter_mut() {
        if bp.matches(&ev, bytes) {
            bp.hits += 1;
            if bp.hits > bp.skip_first {
                if bp.log_only {
                    ev.mark = Some(format!("log: {}", bp.label()));
                } else if why.is_none() {
                    why = Some(format!("Breakpoint: {}", bp.label()));
                }
            }
        }
    }
    if why.is_none() {
        why = match g.step {
            Some(Step::Any) => Some("Step: the next message".into()),
            Some(Step::In) if dir == Dir::In => Some("Step: the next message from the FM-1".into()),
            Some(Step::Out) if dir == Dir::Out => Some("Step: the next message to the FM-1".into()),
            Some(Step::Kind(k)) if k == kind => Some(format!("Step: the next {}", k.name())),
            _ => None,
        };
    }
    if why.is_some() && !may_stop {
        ev.mark = Some(format!("would stop here ({}), but the app's own window sent it", why.take().unwrap_or_default()));
    }
    g.last_by_port.insert((port.to_string(), kind), t);
    g.log.push_back(ev.clone());
    while g.log.len() > LOG_MAX {
        g.log.pop_front();
    }
    let Some(why) = why else { return true };
    // stop: wait our turn (another thread may be stopped), then until released
    while g.stop.is_some() && ON.load(Ordering::Relaxed) {
        g = cv.wait_timeout(g, Duration::from_millis(200)).unwrap().0;
    }
    if !ON.load(Ordering::Relaxed) {
        return true;
    }
    g.step = None;
    g.stop = Some(Stop { event: ev, full: bytes.to_vec(), why, thread, since: Instant::now() });
    if let Some(c) = &g.ctx {
        c.request_repaint();
    }
    loop {
        if let Some((rid, skip)) = g.released {
            if rid == id {
                g.released = None;
                cv.notify_all();
                return !skip;
            }
        }
        if !ON.load(Ordering::Relaxed) {
            return true;
        }
        g = cv.wait_timeout(g, Duration::from_millis(200)).unwrap().0;
    }
}

/// Let the stopped thread go on: `step` where to stop next (None: the next breakpoint), `skip` the stopped message.
pub fn release(step: Step, skip: bool) {
    let (m, cv) = &*DBG;
    let mut g = m.lock().unwrap();
    if let Some(s) = g.stop.take() {
        g.released = Some((s.event.id, skip));
    }
    g.step = (step != Step::None).then_some(step);
    cv.notify_all();
}

/// Stop at the next message, whatever it is.
pub fn pause() {
    DBG.0.lock().unwrap().step = Some(Step::Any);
}

pub fn stopped() -> Option<Stop> {
    DBG.0.lock().unwrap().stop.clone()
}

pub fn with_breakpoints<R>(f: impl FnOnce(&mut Vec<Breakpoint>) -> R) -> R {
    f(&mut DBG.0.lock().unwrap().bps)
}

pub fn log_snapshot(filter: impl Fn(&Event) -> bool, max: usize) -> Vec<Event> {
    let g = DBG.0.lock().unwrap();
    let mut v: Vec<Event> = g.log.iter().rev().filter(|e| filter(e)).take(max).cloned().collect();
    v.reverse();
    v
}

pub fn counts() -> Vec<(Kind, u64)> {
    let g = DBG.0.lock().unwrap();
    let mut v: Vec<_> = g.counts.iter().map(|(k, n)| (*k, *n)).collect();
    v.sort();
    v
}

pub fn clear_log() {
    let mut g = DBG.0.lock().unwrap();
    g.log.clear();
    g.counts.clear();
}

/// The log as text: every line, oldest first.
pub fn log_text() -> String {
    let g = DBG.0.lock().unwrap();
    let mut s = String::from("FM-1 Big Screen debugger log\n#\ttime s\tport\tdir\tkind\tlength\tsummary\tbytes\n");
    for e in &g.log {
        s += &format!(
            "{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}{}\n",
            e.id, e.t, e.port, if e.dir == Dir::In { "from FM-1" } else { "to FM-1" }, e.kind.name(), e.len,
            summary(e.dir, &e.bytes), hex(&e.bytes), if e.len > e.bytes.len() { " …" } else { "" }
        );
    }
    s
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn ascii(b: &[u8]) -> String {
    b.iter().map(|&c| if (32..127).contains(&c) { c as char } else { '·' }).collect()
}

const NOTE: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
fn note(n: u8) -> String {
    format!("{}{} ({n})", NOTE[n as usize % 12], n as i32 / 12 - 1)
}
const ENC: [&str; 7] = ["SELECT", "ALGORITHM", "KNOB 1", "KNOB 2", "KNOB 3", "KNOB 4", "PRESETS"];
const BTN: [&str; 14] = ["FX", "SEL", "ENV", "LFO", "EDIT", "GLOBAL", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC", "OCT-", "OCT+"];
fn s7(b: u8) -> i32 {
    if b & 64 != 0 { b as i32 - 128 } else { b as i32 }
}
fn u14(lo: u8, hi: u8) -> u32 {
    lo as u32 | (hi as u32) << 7
}

pub fn classify(dir: Dir, m: &[u8]) -> Kind {
    let n = m.len();
    if n == 0 {
        return Kind::Other;
    }
    match m[0] {
        0xF0 if n >= 6 && m[1] == 0x7D && m[2] == b'F' && m[3] == b'S' => match (m[4], dir) {
            (0 | 1, Dir::Out) => Kind::HelloV1,
            (1, Dir::In) => Kind::ScreenRow,
            (2, _) => Kind::SoundName,
            (3, _) => Kind::Bpm,
            (4, _) => Kind::KnobTurn,
            (5, _) => Kind::KeyLights,
            (6, _) => Kind::Master,
            (7, _) => Kind::Press,
            _ => Kind::OtherSysex,
        },
        0xF0 if n >= 6 && m[1] == 0x7D && m[2] == b'F' && m[3] == b'L' => if dir == Dir::Out { Kind::AskInfo } else { Kind::FirmwareInfo },
        0xF0 if n >= 6 && m[1] == 0x7D && m[2] == b'B' && m[3] == b'S' => match m[4] {
            0 => Kind::HelloV2,
            1 => Kind::V2Info,
            2 => Kind::V2Rect,
            3 => Kind::V2Sync,
            _ => Kind::OtherSysex,
        },
        0xF0 => match crate::install::parse(m) {
            Some((17, _)) => if dir == Dir::Out { Kind::AskIdentity } else { Kind::Identity },
            Some((49, _)) => if dir == Dir::Out { Kind::AskBuild } else { Kind::Build },
            Some((0x50, _)) => if dir == Dir::Out { Kind::BaseRequest } else { Kind::BaseHello },
            Some((0x51, _)) => Kind::BaseRect,
            Some((0x52, _)) => Kind::BaseFill,
            Some((0x53, _)) => Kind::BaseInput,
            Some((0x54, _)) => Kind::BaseRemote,
            Some(_) => Kind::Updater,
            None => Kind::OtherSysex,
        },
        0xF8 => Kind::Clock,
        0xFA | 0xFB | 0xFC => Kind::Transport,
        s if s & 0xF0 == 0x90 && n >= 3 => if m[2] > 0 { Kind::NoteOn } else { Kind::NoteOff },
        s if s & 0xF0 == 0x80 => Kind::NoteOff,
        s if s & 0xF0 == 0xB0 => Kind::Control,
        s if s & 0xF0 == 0xE0 => Kind::PitchBend,
        _ => Kind::Other,
    }
}

/// One line about it, for the log.
pub fn summary(dir: Dir, m: &[u8]) -> String {
    decode(dir, m).1.iter().take(4).map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(" · ")
}

/// Its kind and its fields, as the app reads them.
pub fn decode(dir: Dir, m: &[u8]) -> (Kind, Vec<(String, String)>) {
    let kind = classify(dir, m);
    let n = m.len();
    let mut f: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str, v: String| f.push((k.to_string(), v));
    match kind {
        Kind::HelloV1 => add("asks for", if m[4] == 1 { "the screen, all of it again".into() } else { "the screen (what changes)".into() }),
        Kind::HelloV2 => {
            add("version", m.get(5).map_or("?".into(), |v| v.to_string()));
            add("flags", m.get(6).map_or("?".into(), |v| if v & 1 != 0 { "send the whole screen again".into() } else { "what changes".into() }));
        }
        Kind::ScreenRow if n > 9 => {
            add("row y", u14(m[5], m[6]).to_string());
            add("from x", u14(m[7], m[8]).to_string());
            add("data", format!("{} bytes", n - 10));
        }
        Kind::SoundName | Kind::FirmwareInfo => add("text", format!("\"{}\"", String::from_utf8_lossy(&m[5..n.saturating_sub(1).max(5)]))),
        Kind::Bpm if n >= 8 => add("BPM", u14(m[5], m[6]).to_string()),
        Kind::KnobTurn if n >= 8 => {
            add("encoder", ENC.get(m[5] as usize).map_or(m[5].to_string(), |s| s.to_string()));
            add("clicks", format!("{:+}", s7(m[6])));
        }
        Kind::Master if n >= 8 => add("MASTER", format!("{} of 1023", u14(m[5], m[6]))),
        Kind::Press if n >= 8 => {
            let id = m[5] as usize;
            add("button", if id < BTN.len() { BTN[id].into() } else { format!("key {}", id - BTN.len()) });
            add("", if m[6] != 0 { "pressed".into() } else { "let go".into() });
        }
        Kind::KeyLights if crate::knobs::lights(m).is_some() => {
            let l = crate::knobs::lights(m).unwrap();
            add("lights", format!("{l:?}"));
        }
        Kind::V2Info if n >= 12 => {
            add("version", m[5].to_string());
            add("size", format!("{} × {}", u14(m[6], m[7]), u14(m[8], m[9])));
            add("format", m[10].to_string());
        }
        Kind::V2Rect if n >= 14 => {
            add("x, y", format!("{}, {}", u14(m[5], m[6]), u14(m[7], m[8])));
            add("w × h", format!("{} × {}", u14(m[9], m[10]), u14(m[11], m[12])));
        }
        Kind::AskInfo => add("asks", "which firmware is running".into()),
        Kind::AskIdentity => add("asks", "which FM-1 this is (updater 17)".into()),
        Kind::AskBuild => add("asks", "which build (updater 49)".into()),
        _ => {}
    }
    if let Some((cmd, body)) = (m.first() == Some(&0xF0)).then(|| crate::install::parse(m)).flatten() {
        match (kind, cmd) {
            (Kind::Identity, _) => add("identity", format!("\"{}\"", String::from_utf8_lossy(&body).trim_end_matches('\0'))),
            (Kind::Build, _) if body.len() >= 46 => add("build", hex(&body[14..46]).replace(' ', "")),
            (Kind::BaseRequest, _) => add("asks for", match body.first() { Some(2) => "on, and the whole screen".into(), Some(1) => "keep on".into(), Some(0) => "off".into(), _ => "?".into() }),
            (Kind::BaseHello, _) if body.len() >= 4 => {
                add("version", body[0].to_string());
                add("mirror", if body[1] != 0 { "on".into() } else { "off".into() });
                add("size", format!("{} × {}", body[2] as u32 + 1, body[3] as u32 + 1));
            }
            (Kind::BaseRect | Kind::BaseFill, _) if body.len() >= 9 => {
                let w = |i: usize| body[i] as u32 | (body[i + 1] as u32) << 8;
                add("x, y", format!("{}, {}", w(0), w(2)));
                add("w × h", format!("{} × {}", w(4), w(6)));
                add("seq", body[8].to_string());
            }
            (Kind::BaseRemote, _) if body.len() >= 3 => {
                add("seq", body[0].to_string());
                add("op", body[1].to_string());
                add(if dir == Dir::In { "answer" } else { "a b c" }, hex(&body[2..]));
            }
            _ => {}
        }
        add("updater command", format!("{cmd} (0x{cmd:02X}), {} bytes of body", body.len()));
    }
    match kind {
        Kind::NoteOn | Kind::NoteOff if n >= 3 => {
            add("channel", ((m[0] & 15) + 1).to_string());
            add("note", note(m[1]));
            add("velocity", m[2].to_string());
        }
        Kind::Control if n >= 3 => {
            add("channel", ((m[0] & 15) + 1).to_string());
            add("controller", m[1].to_string());
            add("value", m[2].to_string());
        }
        Kind::PitchBend if n >= 3 => add("bend", (u14(m[1], m[2]) as i32 - 8192).to_string()),
        _ => {}
    }
    if m.first() == Some(&0xF0) && n > 5 && f.is_empty() {
        f.push(("text".into(), ascii(&m[1..n - 1])));
    }
    (kind, f)
}

/// A hex dump, 16 bytes a line: offset, bytes, the printable ones.
pub fn dump(b: &[u8]) -> String {
    b.chunks(16).enumerate().map(|(i, c)| format!("{:04X}  {:<48} {}\n", i * 16, hex(c), ascii(c))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn kinds_and_patterns() {
        assert_eq!(classify(Dir::Out, &[0xF0, 0x7D, b'F', b'S', 0, 0xF7]), Kind::HelloV1);
        assert_eq!(classify(Dir::In, &[0xF0, 0x7D, b'F', b'S', 5, 0, 0, 0, 0, 0, 0, 0xF7]), Kind::KeyLights);
        assert_eq!(classify(Dir::In, &[0x90, 60, 100]), Kind::NoteOn);
        assert_eq!(classify(Dir::In, &[0x90, 60, 0]), Kind::NoteOff);
        assert_eq!(classify(Dir::Out, &crate::fm1base::request(2)), Kind::BaseRequest);
        assert_eq!(classify(Dir::Out, &crate::install::frame(17, &[])), Kind::AskIdentity);
        let p = parse_pattern("F0 7D 46 53 ??").unwrap();
        let bp = Breakpoint::new(What::Bytes(p));
        let ev = |m: &[u8]| Event { id: 0, t: 0.0, port: "FM1".into(), dir: Dir::In, kind: classify(Dir::In, m), len: m.len(), bytes: m.to_vec(), mark: None, answers_hello: false };
        let m = [0xF0, 0x7D, b'F', b'S', 2, b'A', 0xF7];
        assert!(bp.matches(&ev(&m), &m));
        let m2 = [0xF0, 0x7D, b'B', b'S', 1, 0xF7];
        assert!(!bp.matches(&ev(&m2), &m2));
        assert_eq!(decode(Dir::Out, &crate::knobs::turn(2, -3)).1[1].1, "-3");
    }

    #[test]
    fn a_breakpoint_really_stops_the_thread_until_released() {
        let _g = SERIAL.lock().unwrap();
        let ctx = egui::Context::default();
        set_on(true, &ctx);
        with_breakpoints(|b| b.push(Breakpoint::new(What::Kind(Kind::SoundName))));
        let h = std::thread::spawn(|| check("FM-1", Dir::In, &[0xF0, 0x7D, b'F', b'S', 2, b'X', 0xF7], "MIDI in", true));
        let t = Instant::now();
        while stopped().is_none() && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stopped().is_some_and(|s| s.event.kind == Kind::SoundName));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!h.is_finished()); // really stopped
        release(Step::None, true); // skip it
        assert!(!h.join().unwrap());
        with_breakpoints(|b| b.clear());
        set_on(false, &ctx);
    }

    fn wait_stop(id_kind: Kind) -> Stop {
        let t = Instant::now();
        loop {
            if let Some(s) = stopped() {
                if s.event.kind == id_kind {
                    return s;
                }
            }
            assert!(t.elapsed() < Duration::from_secs(2), "no stop at {id_kind:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn stepping_to_the_next_message_from_the_fm1_passes_the_ones_to_it() {
        let _g = SERIAL.lock().unwrap();
        let ctx = egui::Context::default();
        set_on(true, &ctx);
        pause(); // stop at the next message, whatever it is
        let sound = [0xF0, 0x7D, b'F', b'S', 2, b'A', 0xF7];
        let a = std::thread::spawn(move || check("FM-1", Dir::In, &sound, "MIDI in", true));
        wait_stop(Kind::SoundName);
        release(Step::In, false); // the next message from the FM-1
        assert!(a.join().unwrap());
        // a hello to the FM-1 goes by without stopping
        let b = std::thread::spawn(|| check("FM-1", Dir::Out, &crate::screen::HELLO, "hello thread", true));
        assert!(b.join().unwrap());
        assert!(stopped().is_none());
        // the next one from the FM-1 stops, and says it was the step
        let c = std::thread::spawn(|| check("FM-1", Dir::In, &[0x90, 60, 100], "MIDI in", true));
        let s = wait_stop(Kind::NoteOn);
        assert!(s.why.contains("from the FM-1"));
        release(Step::None, false); // continue
        assert!(c.join().unwrap());
        // and with no breakpoint, nothing stops
        assert!(check("FM-1", Dir::In, &[0x90, 61, 100], "MIDI in", true));
        set_on(false, &ctx);
    }

    #[test]
    fn hello_answer_and_hits_after() {
        let _g = SERIAL.lock().unwrap();
        let ctx = egui::Context::default();
        set_on(true, &ctx);
        let mut bp = Breakpoint::new(What::SaysHello);
        bp.skip_first = 1; // the second answer
        with_breakpoints(|b| b.push(bp));
        let row = [0xF0, 0x7D, b'F', b'S', 1, 0, 0, 0, 0, 0xF7];
        assert!(check("FM-1", Dir::Out, &crate::screen::HELLO, "hello thread", true));
        assert!(check("FM-1", Dir::In, &row, "MIDI in", true)); // first answer: skipped by "after 1 hit"
        assert!(check("FM-1", Dir::In, &row, "MIDI in", true)); // not an answer (no hello before it)
        assert!(check("FM-1", Dir::Out, &crate::screen::HELLO, "hello thread", true));
        let t = std::thread::spawn(move || check("FM-1", Dir::In, &row, "MIDI in", true));
        let s = wait_stop(Kind::ScreenRow);
        assert!(s.event.answers_hello);
        release(Step::None, false);
        assert!(t.join().unwrap());
        with_breakpoints(|b| b.clear());
        set_on(false, &ctx);
    }
}
