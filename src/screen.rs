//! The FM-1's screen as the Big Screen firmware sends it over USB MIDI.
//!
//! The computer says hello (F0 7D 'F' 'S' 00 F7, or 01 to have the whole screen sent again)
//! about once a second; while it does, the FM-1 sends every row it paints as F0 7D 'F' 'S' 01,
//! the row and first column (two 7-bit bytes each), then runs of four bytes: a count, and the
//! RGB565 colour in 7, 7 and 2 bits. (Rows are at least ten bytes, hellos six.)
//! Firmware that knows it also sends the sound playing, F0 7D 'F' 'S' 02 and its name in ASCII
//! ("T1 FM6 ICE BELL"), after each hello and whenever it changes.

pub const SIZE: usize = 240;
pub const HELLO: [u8; 6] = [0xf0, 0x7d, b'F', b'S', 0, 0xf7];
/// Hello, and send the whole screen again.
pub const HELLO_ALL: [u8; 6] = [0xf0, 0x7d, b'F', b'S', 1, 0xf7];
const ROW: [u8; 5] = [0xf0, 0x7d, b'F', b'S', 1];
const SOUND: [u8; 5] = [0xf0, 0x7d, b'F', b'S', 2];
/// Asks a firmware of the Felucca family what it is (its editor's INFO); others don't answer.
pub const ASK_INFO: [u8; 6] = [0xf0, 0x7d, b'F', b'L', 1, 0xf7];
const INFO: [u8; 5] = [0xf0, 0x7d, b'F', b'L', 1];

/// The firmware's own name, if this is its answer to `ASK_INFO`: "FELUCCA SLOOP 2.8.4", then other things.
pub fn firmware_info(m: &[u8]) -> Option<String> {
    if m.len() < 7 || m[..5] != INFO || m[m.len() - 1] != 0xf7 {
        return None;
    }
    let name: String = m[5..m.len() - 1].iter().take_while(|&&b| b != 0).filter(|b| (0x20..0x7f).contains(*b)).map(|&b| b as char).collect();
    (!name.trim().is_empty()).then(|| name.trim().to_string())
}

/// The name to show. SLOOP and Skiff keep Felucca's USB name, and their answers start with "FELUCCA " for the web
/// tools: "FELUCCA SLOOP 2.8.4" is SLOOP, and "FELUCCA v2.1.1" is Skiff when its identity is FM-1_7xx, else Felucca.
/// Skiff Atlatl (LIkst4's fork of Skiff, FM-1_742xx) answers the same way: "FELUCCA v1.2" is Skiff Atlatl 1.2.
/// Skiff Atlatl's identities (FM-1_742xx): it sends its screen itself (LIkst4 built the screen part in, V1.2).
const ATLATL: &str = "FM-1_742";

pub fn firmware_name(info: &str, identity: Option<&str>) -> String {
    let Some(rest) = info.strip_prefix("FELUCCA ") else { return info.to_string() };
    let bare_version = rest.strip_prefix('v').is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()));
    if bare_version && identity.is_some_and(|id| id.starts_with(ATLATL)) {
        return format!("Skiff Atlatl {rest}");
    }
    match (bare_version, identity.is_some_and(|id| id.starts_with("FM-1_7"))) {
        (true, true) => format!("Skiff {rest}"),
        (true, false) => format!("Felucca {rest}"),
        _ => rest.to_string(),
    }
}

/// A community firmware that has a Big Screen version (or one on the way), and where to get it.
#[derive(Clone, Debug, PartialEq)]
pub struct Community {
    pub name: &'static str,
    /// The Big Screen build: which version of the firmware it is.
    pub build: &'static str,
    pub url: &'static str,
    /// A ready-made download (else the page explains: offered to its author, or its own mirror).
    pub released: bool,
}

/// Which community firmware this is, from its own name (the answer to `ASK_INFO`), its USB name, or its identity, when
/// it isn't sending its screen: its regular release has no Big Screen part. None: not one we know.
pub fn community(info: Option<&str>, identity: Option<&str>, port: &str) -> Option<Community> {
    let t = format!("{} {}", info.unwrap_or(""), port).to_lowercase();
    let id = identity.unwrap_or("");
    let c = |name, build, url, released| Some(Community { name, build, url, released });
    if t.contains("sloop") {
        return c("SLOOP", "SLOOP 2.8.5 + Big Screen", "https://github.com/pfkellogg/sloop-fm1/releases", true);
    }
    if t.contains("jiant") {
        return c("JIANT FM", "JIANT FM V10.49 + Big Screen", "https://github.com/pfkellogg/JIANT/releases", true);
    }
    if t.contains("melodee") || id.starts_with("FM-1_910") {
        return c("Melodee", "Melodee 1.6.1 + Big Screen", "https://github.com/pfkellogg/melodee/releases", true);
    }
    if t.contains("choralroot") {
        return c("ChoralRoot", "ChoralRoot 0.29 + Big Screen", "https://github.com/pfkellogg/ChoralRootFM1/releases", true);
    }
    if id.starts_with(ATLATL) {
        return None; // (Skiff Atlatl sends its screen itself: nothing to offer)
    }
    if t.contains("skiff") || (t.contains("felucca v") && id.starts_with("FM-1_7")) {
        return c("Skiff", "Skiff 2.1.1 + Big Screen", "https://github.com/pfkellogg/skiff/releases", true);
    }
    if t.contains("erik37") || id.starts_with("FM-1_480") {
        return c("Erik37", "Erik37 1.3 + Big Screen", "https://github.com/pfkellogg/erik37/blob/big-screen/BIG_SCREEN.md", false);
    }
    if t.contains("nachtfahrt") || id.starts_with("FM-1_197") {
        return c("NACHTFAHRT", "NACHTFAHRT 1.0 + Big Screen", "https://github.com/pfkellogg/fm1-nachtfahrt/blob/big-screen/BIG_SCREEN.md", false);
    }
    if t.contains("loom") {
        return c("LOOM-8", "LOOM-8's own Mirror page", "https://charlesvestal.github.io/fm1-loom/mirror/", false);
    }
    None
}

/// The sound playing, if this is the FM-1 saying it (an empty name: none).
pub fn sound(m: &[u8]) -> Option<String> {
    if m.len() < 6 || m[..5] != SOUND || m[m.len() - 1] != 0xf7 {
        return None;
    }
    let name: String = m[5..m.len() - 1].iter().filter(|b| (0x20..0x7f).contains(*b)).map(|&b| b as char).collect();
    Some(name.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub struct Screen {
    /// RGB565, row by row.
    pub pixels: Vec<u16>,
}

impl Default for Screen {
    fn default() -> Self {
        Screen { pixels: vec![0; SIZE * SIZE] }
    }
}

impl Screen {
    /// Paints one row message; false if it isn't one.
    pub fn apply(&mut self, m: &[u8]) -> bool {
        if m.len() < 10 || m[..5] != ROW || m[m.len() - 1] != 0xf7 {
            return false;
        }
        let y = m[5] as usize | (m[6] as usize) << 7;
        let mut x = m[7] as usize | (m[8] as usize) << 7;
        if y >= SIZE {
            return false;
        }
        for run in m[9..m.len() - 1].chunks_exact(4) {
            let c = run[1] as u16 | (run[2] as u16) << 7 | (run[3] as u16) << 14;
            for _ in 0..run[0] {
                if x < SIZE {
                    self.pixels[y * SIZE + x] = c;
                }
                x += 1;
            }
        }
        true
    }
}

/// The SysEx messages in a buffer, one by one (F0 … F7 each), whatever else is around them.
pub fn messages(buffer: &[u8]) -> impl Iterator<Item = &[u8]> {
    buffer.split_inclusive(|&b| b == 0xf7).filter_map(|part| {
        let start = part.iter().position(|&b| b == 0xf0)?;
        (part.last() == Some(&0xf7)).then(|| &part[start..])
    })
}

/// RGB565 to 8-bit red, green, blue.
pub fn rgb(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r << 3 | r >> 2) as u8, (g << 2 | g >> 4) as u8, (b << 3 | b >> 2) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_firmware_says_its_name() {
        let mut m = INFO.to_vec();
        m.extend_from_slice(b"FELUCCA SLOOP 2.8.4\0");
        m.extend_from_slice(&[6, 40, 12, 0xf7]);
        assert_eq!(firmware_info(&m).as_deref(), Some("FELUCCA SLOOP 2.8.4"));
        assert_eq!(firmware_info(&HELLO), None);
    }

    #[test]
    fn firmware_names_past_felucca_usb_name() {
        assert_eq!(firmware_name("FELUCCA SLOOP 2.8.4", Some("FM-1_928")), "SLOOP 2.8.4");
        assert_eq!(firmware_name("FELUCCA ChoralRoot 0.29", Some("FM-1_920")), "ChoralRoot 0.29");
        assert_eq!(firmware_name("JIANT v0.9.18", Some("FM-1_900")), "JIANT v0.9.18");
        assert_eq!(firmware_name("FELUCCA v2.1.1", Some("FM-1_700")), "Skiff v2.1.1");
        assert_eq!(firmware_name("FELUCCA v1.5", Some("FM-1_915")), "Felucca v1.5");
        assert_eq!(firmware_name("FELUCCA v1.2", Some("FM-1_74212")), "Skiff Atlatl v1.2");
        let n = |info: Option<&str>, id: Option<&str>, port: &str| community(info, id, port).map(|c| c.name);
        assert_eq!(n(Some("FELUCCA SLOOP 2.8.6"), Some("FM-1_928"), "Felucca"), Some("SLOOP"));
        assert_eq!(n(None, None, "JIANT"), Some("JIANT FM"));
        assert_eq!(n(Some("JIANT v0.9.18"), Some("FM-1_900"), "FM-1"), Some("JIANT FM"));
        assert_eq!(n(None, Some("FM-1_91061"), "Melodee FM-1"), Some("Melodee"));
        assert_eq!(n(Some("FELUCCA v2.1.1"), Some("FM-1_700"), "Felucca"), Some("Skiff"));
        assert_eq!(n(None, Some("FM-1_480103"), "ERIK37"), Some("Erik37"));
        assert_eq!(n(None, Some("FM-1_197000000"), "DUB-1 FM-1"), Some("NACHTFAHRT"));
        assert_eq!(n(Some("FELUCCA v1.5"), Some("FM-1_915"), "Felucca"), None);
        assert_eq!(n(Some("FELUCCA v1.2"), Some("FM-1_74212"), "Felucca"), None);
        assert_eq!(n(None, Some("FM-1_8000014"), "FM1 Community"), None);
        assert_eq!(firmware_name("FELUCCA v1.5", None), "Felucca v1.5");
    }

    /// The firmware's encoding, for a row segment.
    fn encode(y: usize, x: usize, px: &[u16]) -> Vec<u8> {
        let mut m = vec![0xf0, 0x7d, b'F', b'S', 1, (y & 127) as u8, (y >> 7) as u8, (x & 127) as u8, (x >> 7) as u8];
        let mut i = 0;
        while i < px.len() {
            let mut run = 1;
            while i + run < px.len() && run < 127 && px[i + run] == px[i] {
                run += 1;
            }
            let c = px[i];
            m.extend([run as u8, (c & 127) as u8, ((c >> 7) & 127) as u8, (c >> 14) as u8]);
            i += run;
        }
        m.push(0xf7);
        m
    }

    #[test]
    fn rows_round_trip() {
        let mut s = Screen::default();
        let px: Vec<u16> = (0..200).map(|i| if i % 7 < 3 { 0xef5d } else { (i * 977) as u16 }).collect();
        assert!(s.apply(&encode(200, 30, &px)));
        assert_eq!(&s.pixels[200 * SIZE + 30..200 * SIZE + 230], &px[..]);
        assert_eq!(s.pixels[200 * SIZE + 29], 0);
    }

    #[test]
    fn ignores_other_messages() {
        let mut s = Screen::default();
        assert!(!s.apply(&HELLO));
        assert!(!s.apply(&[0xf0, 0x7d, b'F', b'M', 2, 0, 0, 0, 0, 0xf7]));
        assert!(!s.apply(&encode(240, 0, &[1, 2, 3])));
    }

    #[test]
    fn glued_messages_come_apart() {
        let glued = [0xf0, 1, 2, 0xf7, 0xf0, 3, 0xf7];
        let all: Vec<&[u8]> = messages(&glued).collect();
        assert_eq!(all, vec![&[0xf0, 1, 2, 0xf7][..], &[0xf0, 3, 0xf7][..]]);
        assert_eq!(messages(&HELLO).count(), 1);
    }

    #[test]
    fn colours() {
        assert_eq!(rgb(0xffff), [255, 255, 255]);
        assert_eq!(rgb(0x07ff), [0, 255, 255]);
        assert_eq!(rgb(0), [0, 0, 0]);
    }
}
