//! Installing the Big Screen firmware from the app, over the FM-1's USB MIDI connection.
//!
//! A port of the "tagged" update in M-VAVE's own updater (fm1_open sdk/WebUpdate.js, and
//! tools/flash_fm1.py here), so the FM-1 sees exactly what M-VAVE's workbench page sends. It
//! needs an FM-1 that already runs M-VAVE's Open firmware: the FM-1's own updater asks for
//! each block by address and length and gets it back, then checks and restarts by itself.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The firmware this app installs, and the build it reports afterwards.
pub const PACKAGE: &[u8] = include_bytes!("../firmware/FM1_BIG_SCREEN_V14.fwsc");
const BUILD_ID: &str = include_str!("../firmware/FM1_BIG_SCREEN_V14.build-identity.json");
/// The package uses M-VAVE's own updater program, which names itself this while it works.
/// The FM-1's updater names itself "ota-FM-1_" and the version of the start-up part it came with: "ota-FM-1_015" on a
/// unit from V15, "ota-FM-1_016" from V16 (M-VAVE's FM-1.fwsc since 2026-10-21). Any of them is the FM-1's updater.
const LOADER_IDENTITY: &str = "ota-FM-1_";
const ENTER_LOADER: [u8; 6] = [0xf0, 0x22, 0x24, 0x35, 0x7f, 0xf7];

pub fn build_id() -> String {
    let v: serde_json::Value = serde_json::from_str(BUILD_ID).unwrap_or_default();
    v["build_id"].as_str().unwrap_or_default().to_lowercase()
}

// ---- the updater's messages: F0, [0, 89, command, length (3 bytes), payload…, checksum] packed 7 bits at a time, F7

fn pack7(bytes: &[u8]) -> Vec<u8> {
    let (mut out, mut bits, mut value) = (Vec::new(), 0u32, 0u32);
    for &b in bytes {
        value |= (b as u32) << bits;
        bits += 8;
        while bits >= 7 {
            out.push((value & 127) as u8);
            value >>= 7;
            bits -= 7;
        }
    }
    if bits > 0 {
        out.push((value & 127) as u8);
    }
    out
}

pub fn frame(command: u8, payload: &[u8]) -> Vec<u8> {
    let sum = payload.iter().fold(0u8, |s, &b| s.wrapping_add(b));
    let n = payload.len();
    let mut raw = vec![0, 89, command, (n & 255) as u8, (n >> 8) as u8, 0];
    raw.extend_from_slice(payload);
    raw.push(!sum);
    let mut m = vec![0xf0];
    m.extend(pack7(&raw));
    m.push(0xf7);
    m
}

/// An updater reply: its command and payload.
pub fn parse(m: &[u8]) -> Option<(u8, Vec<u8>)> {
    if m.len() < 10 || m.len() > 608 || m[0] != 0xf0 || m[m.len() - 1] != 0xf7 {
        return None;
    }
    let (mut raw, mut bits, mut value) = (Vec::new(), 0u32, 0u32);
    for &b in &m[1..m.len() - 1] {
        if b > 127 {
            return None;
        }
        value |= (b as u32) << bits;
        bits += 7;
        if bits >= 8 {
            raw.push((value & 255) as u8);
            value >>= 8;
            bits -= 8;
        }
    }
    if bits >= 7 || value != 0 || raw.len() < 7 || raw[0] != 0 || raw[1] != 89 {
        return None;
    }
    if le(&raw, 3, 3) as usize != raw.len() - 7 || raw[6..].iter().fold(0u8, |s, &b| s.wrapping_add(b)) != 255 {
        return None;
    }
    Some((raw[2], raw[6..raw.len() - 1].to_vec()))
}

fn le(p: &[u8], at: usize, n: usize) -> u32 {
    (0..n).fold(0, |v, i| v | (p[at + i] as u32) << (8 * i))
}

// ---- the package: an image with the product name woven into its first 960 bytes

fn crc16(bytes: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &b in bytes {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

fn decode_header(bytes: &[u8]) -> Vec<u8> {
    let mut key: u16 = 0xffff;
    bytes
        .iter()
        .map(|&b| {
            let out = b ^ (key & 255) as u8;
            key = (key << 1) ^ if key & 0x8000 != 0 { 0x1021 } else { 0 };
            out
        })
        .collect()
}

/// The product it installs as ("FM-1_8000014") and the image the FM-1 asks for, checked the
/// way M-VAVE's updater checks them.
pub fn inspect(data: &[u8]) -> Result<(String, Vec<u8>), String> {
    if data.len() < 1024 || data.len() > 2_097_152 {
        return Err("package length".into());
    }
    let mut image = vec![0u8; data.len() - 20];
    let (mut product, mut ended) = (String::new(), false);
    for i in 0..20 {
        image[i * 47..i * 47 + 47].copy_from_slice(&data[i * 48..i * 48 + 47]);
        let marker = data[i * 48 + 47];
        if marker == 125 {
            ended = true;
        } else {
            let c = marker.wrapping_sub(i as u8 + 1);
            if ended || !(32..=126).contains(&c) {
                return Err("package identity".into());
            }
            product.push(c as char);
        }
    }
    image[940..].copy_from_slice(&data[960..]);
    let head = decode_header(&image[..64]);
    let count = le(&head, 8, 2) as usize;
    if le(&head, 0, 2) as u16 != crc16(&head[2..]) || le(&head, 4, 4) as usize != image.len() {
        return Err("package header".into());
    }
    if &head[16..22] != b"AC791N" || count == 0 || count > 12 || crc16(&image[64..64 + count * 80]) != le(&head, 2, 2) as u16 {
        return Err("package directory".into());
    }
    Ok((product, image))
}

// ---- talking to the FM-1

struct Link {
    _input: midir::MidiInputConnection<()>,
    output: midir::MidiOutputConnection,
    replies: Arc<Mutex<VecDeque<(u8, Vec<u8>)>>>,
}

impl Link {
    fn open(name: &str) -> Option<Link> {
        let mut input = midir::MidiInput::new("FM-1 Mirror installer").ok()?;
        input.ignore(midir::Ignore::None);
        let output = midir::MidiOutput::new("FM-1 Mirror installer").ok()?;
        let ip = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == name))?;
        let op = output.ports().into_iter().find(|p| output.port_name(p).is_ok_and(|n| n == name))?;
        let replies: Arc<Mutex<VecDeque<(u8, Vec<u8>)>>> = Arc::default();
        let r = replies.clone();
        let input = input
            .connect(&ip, "in", move |_, buffer, _| {
                // macOS can deliver several SysEx messages in one buffer: take them one by one.
                for m in crate::screen::messages(buffer) {
                    if let Some(p) = parse(m) {
                        r.lock().unwrap().push_back(p);
                    }
                }
            }, ())
            .ok()?;
        let output = output.connect(&op, "out").ok()?;
        Some(Link { _input: input, output, replies })
    }

    fn send(&mut self, m: &[u8]) -> Result<(), String> {
        self.output.send(m).map_err(|_| "the USB connection went away".to_string())
    }

    fn read(&self, wait: Duration) -> Option<(u8, Vec<u8>)> {
        let until = Instant::now() + wait;
        loop {
            if let Some(p) = self.replies.lock().unwrap().pop_front() {
                return Some(p);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn query(&mut self, command: u8, payload: &[u8], wait: Duration) -> Option<Vec<u8>> {
        self.replies.lock().unwrap().clear();
        self.send(&frame(command, payload)).ok()?;
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if let Some((c, data)) = self.read(Duration::from_millis(100)) {
                if c == command {
                    return Some(data);
                }
            }
        }
        None
    }

    fn identity(&mut self) -> Option<String> {
        (0..2).find_map(|_| self.query(17, &[], Duration::from_millis(750)))
            .filter(|r| r.len() == 27)
            .map(|r| String::from_utf8_lossy(&r).trim_end_matches('\0').to_string())
    }
}

/// The updater's status: family, state, current build.
fn capability(data: Option<Vec<u8>>) -> Result<(u8, u8, String), String> {
    let d = data.ok_or("the FM-1 didn't answer")?;
    if d.len() != 47 || &d[..4] != b"FMC1" || d[4] != 1 || d[5] > 7 || le(&d, 8, 2) != 512 || le(&d, 10, 4) != 1_048_576 || d[7] != 1 {
        return Err("the FM-1's updater isn't one this app knows".into());
    }
    Ok((d[5], d[6], d[14..46].iter().map(|b| format!("{b:02x}")).collect()))
}

/// Ports that may be the FM-1 (or its updater while it's working).
fn candidates(updater: bool) -> Vec<String> {
    let Ok(out) = midir::MidiOutput::new("FM-1 Mirror installer") else { return vec![] };
    out.ports()
        .iter()
        .filter_map(|p| out.port_name(p).ok())
        .filter(|n| {
            let n = n.to_lowercase();
            let fm1 = n.contains("fm1") || n.contains("fm-1") || n.starts_with("usb-midi") || n.starts_with("usb midi") || n.contains("composite");
            let bluetooth = n.contains("_ble") || n.contains(" ble") || n.contains("bluetooth"); // firmware goes over USB only
            !n.contains("bridge") && !bluetooth && (fm1 || (updater && ["felucca", "sloop", "clipper", "ota", "sinco"].iter().any(|h| n.contains(h))))
        })
        .collect()
}

/// Opens the one port that answers as `want` (or as anything), trying again for `wait`.
fn find(want: Option<&str>, updater: bool, wait: Duration) -> Result<Option<(Link, String)>, String> {
    let until = Instant::now() + wait;
    loop {
        let mut found = Vec::new();
        for name in candidates(updater) {
            if let Some(mut link) = Link::open(&name) {
                if let Some(id) = link.identity() {
                    if want.is_none_or(|w| same_identity(w, &id)) {
                        found.push((link, id));
                    }
                }
            }
        }
        if found.len() > 1 {
            return Err("more than one FM-1 answered: unplug the others".into());
        }
        if !found.is_empty() || Instant::now() >= until {
            return Ok(found.pop());
        }
        std::thread::sleep(Duration::from_millis(600));
    }
}

/// The FM-1's updater, waiting for firmware (an update stopped partway, or was started elsewhere).
/// The identity looked for: exact, or (ending in "_") that model with any version, "ota-FM-1_015" or "ota-FM-1_016".
fn same_identity(want: &str, id: &str) -> bool {
    if !want.ends_with('_') {
        return want == id;
    }
    let version = id.strip_prefix(want).unwrap_or("");
    !version.is_empty() && version.chars().all(|c| c.is_ascii_digit())
}

pub fn is_updater(id: &str) -> bool {
    id.to_lowercase().starts_with("ota-")
}

pub fn is_open_firmware(id: &str) -> bool {
    id.starts_with("FM-1_8") || id == "FM1_OFFICIAL_OPEN" || id.starts_with("FM1_COMMUNITY_")
}

#[derive(Clone, Default)]
pub struct Progress {
    /// What's happening, in words.
    pub step: String,
    /// 0-100 while writing.
    pub percent: u32,
    pub done: bool,
    pub error: Option<String>,
}

/// Answers the FM-1's requests for blocks of `image` until it says it has finished `finish`.
fn serve(link: &mut Link, image: &[u8], finish: u32, idle: Duration, progress: &Mutex<Progress>, step: &str) -> Result<(), String> {
    let mut covered = vec![false; image.len()];
    let (mut total, started) = (0usize, Instant::now());
    let mut last = Instant::now();
    loop {
        if started.elapsed() > (idle * 3).max(Duration::from_secs(120)) {
            return Err("the update took too long".into());
        }
        if last.elapsed() > idle {
            return Err("the FM-1 stopped asking for the firmware. Nothing is broken: the FM-1 keeps waiting for the rest of the update".into());
        }
        let Some((48, d)) = link.read(Duration::from_millis(250)) else { continue };
        if d.len() != 8 {
            continue;
        }
        let (address, n) = (le(&d, 1, 4), le(&d, 5, 3) as usize);
        if n == 0 || n > 512 {
            return Err("the FM-1 asked for a bad length".into());
        }
        if address == 0xe000_0000 || address == 0xf000_0000 {
            if address != finish || n != 8 {
                return Err("the FM-1 is at a different step of the update".into());
            }
            let done = |d: &[u8]| {
                let mut p = d[..8].to_vec();
                p.extend_from_slice(b"success\0");
                frame(48, &p)
            };
            link.send(&done(&d))?;
            // Answer repeats of the finish request for a moment, as the workbench page does.
            let until = Instant::now() + Duration::from_millis(600);
            while Instant::now() < until {
                if let Some((48, again)) = link.read(Duration::from_millis(100)) {
                    if again.len() == 8 && le(&again, 1, 4) == finish {
                        link.send(&done(&again))?;
                    }
                }
            }
            return Ok(());
        }
        let a = address as usize;
        if a > image.len() || n > image.len() - a {
            return Err("the FM-1 asked for data outside the firmware".into());
        }
        last = Instant::now();
        let mut p = d[..8].to_vec();
        p.extend_from_slice(&image[a..a + n]);
        link.send(&frame(48, &p))?;
        for c in &mut covered[a..a + n] {
            if !*c {
                *c = true;
                total += 1;
            }
        }
        let mut pr = progress.lock().unwrap();
        pr.step = step.into();
        pr.percent = (total * 100 / image.len()).min(99) as u32;
    }
}

/// When the FM-1 doesn't come back on USB in time: an update can always be finished.
const LOST: &str = "the computer lost the FM-1 partway through. Nothing is broken: the FM-1 keeps waiting for the rest of the update";

/// The whole install, on its own thread; `progress` says how it's going.
pub fn install(progress: Arc<Mutex<Progress>>) {
    let set = |step: &str| {
        let mut p = progress.lock().unwrap();
        p.step = step.into();
        p.percent = 0;
    };
    let result = (|| -> Result<(), String> {
        let (product, image) = inspect(PACKAGE)?;
        let build = build_id();
        set("Finding the FM-1…");
        // The window's own link to the FM-1 lets go first (on Windows a port is one app's at a time).
        std::thread::sleep(Duration::from_millis(400));
        let (mut link, who) = find(None, true, Duration::from_secs(5))?.ok_or("no FM-1 answered. Is it plugged in and on?")?;
        if is_updater(&who) {
            // Left in its updater by an update that stopped partway: carry on from there,
            // as M-VAVE's own page does.
            link.send(&ENTER_LOADER)?;
            serve(&mut link, &image, 0xf000_0000, Duration::from_secs(180), &progress, "Finishing the update: writing the firmware…")?;
        } else {
            if !is_open_firmware(&who) {
                return Err(format!("this FM-1 runs {who}, not M-VAVE's Open firmware. Install that first (see Step 1)."));
            }
            let (family, state, current) = capability(link.query(49, &[], Duration::from_millis(1500)))?;
            if state != 0 {
                return Err("the FM-1 is busy with another update".into());
            }
            if current == build {
                return Ok(()); // already there
            }
            let n = image.len() as u32;
            let mut tag = vec![1, family, n as u8, (n >> 8) as u8, (n >> 16) as u8, (n >> 24) as u8];
            tag.extend((0..32).map(|i| u8::from_str_radix(&build[i * 2..i * 2 + 2], 16).unwrap_or(0)));
            if link.query(50, &tag, Duration::from_millis(1500)) != Some(vec![0]) {
                return Err("the FM-1 didn't accept the firmware".into());
            }
            link.send(&ENTER_LOADER)?;
            serve(&mut link, &image, 0xe000_0000, Duration::from_secs(10), &progress, "Preparing the FM-1's updater…")?;
            drop(link);
            set("Waiting for the FM-1's updater…");
            let (mut link, _) = find(Some(LOADER_IDENTITY), true, Duration::from_secs(60))?.ok_or(LOST)?;
            link.send(&ENTER_LOADER)?;
            serve(&mut link, &image, 0xf000_0000, Duration::from_secs(180), &progress, "Writing the firmware…")?;
        }
        set("Waiting for the FM-1 to restart…");
        let (mut link, _) = find(Some(&product), false, Duration::from_secs(40))?
            .ok_or("the FM-1 didn't come back. Turn it off and on; if it shows M-VAVE's updater, try again.")?;
        let (_, state, now) = capability(link.query(49, &[], Duration::from_millis(1500)))?;
        if now != build || state != 0 {
            return Err(format!("the FM-1 restarted, but on another build ({}) than expected", &now[..16.min(now.len())]));
        }
        Ok(())
    })();
    let mut p = progress.lock().unwrap();
    match result {
        Ok(()) => {
            p.step = "Installed. The FM-1's screen will appear here.".into();
            p.percent = 100;
            p.done = true;
        }
        Err(e) => p.error = Some(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_is_sound() {
        let (product, image) = inspect(PACKAGE).unwrap();
        assert_eq!(product, "FM-1_8000014");
        assert_eq!(image.len(), PACKAGE.len() - 20);
        assert_eq!(build_id().len(), 64);
    }

    #[test]
    fn frames_round_trip() {
        let payload: Vec<u8> = (0..200).map(|i| i as u8).collect();
        assert_eq!(parse(&frame(48, &payload)), Some((48, payload)));
        assert_eq!(parse(&frame(17, &[])), Some((17, vec![])));
    }
}

#[cfg(test)]
mod real_replies {
    /// A capability reply captured from an FM-1 running Big Screen.
    #[test]
    fn capability_reply_parses() {
        let m = [240, 0, 50, 69, 121, 2, 0, 0, 35, 77, 6, 69, 9, 112, 0, 64, 0, 0, 4, 0, 0, 0, 2, 64, 122, 50, 80, 14, 106, 24, 88, 26, 113, 24, 66, 14, 41, 104, 93, 43, 87, 116, 37, 63, 65, 16, 118, 46, 117, 39, 55, 90, 79, 65, 123, 13, 8, 22, 124, 53, 7, 96, 7, 247];
        let (c, d) = super::parse(&m).expect("parses");
        assert_eq!((c, d.len()), (49, 47));
        assert_eq!(&d[..4], b"FMC1");
    }
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    #[test]
    fn the_updater_from_v15_or_v16() {
        assert!(same_identity(LOADER_IDENTITY, "ota-FM-1_015"));
        assert!(same_identity(LOADER_IDENTITY, "ota-FM-1_016")); // a unit that came from V16 (a viewer's report, 2026-11-03)
        assert!(!same_identity(LOADER_IDENTITY, "ota-FM-1_"));
        assert!(!same_identity(LOADER_IDENTITY, "ota-FM-1_900x"));
        assert!(!same_identity(LOADER_IDENTITY, "ota-SMK-37 Pro_016"));
        assert!(same_identity("FM1_COMMUNITY_V14", "FM1_COMMUNITY_V14"));
        assert!(!same_identity("FM1_COMMUNITY_V14", "FM1_COMMUNITY_V15"));
    }
}
