#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows
//! FM-1 Big Screen: the M-VAVE FM-1's screen, big, on the Mac.
//!
//! With the FM-1 Big Screen firmware, the FM-1 sends its screen over its USB MIDI connection
//! while this app keeps asking for it (see screen.rs). It's drawn as large as the window allows,
//! pixel-sharp, in colour or in high-contrast looks for reading. The buttons hide after a few
//! seconds so the screen is as big as it can be: move the mouse to bring them back.
//! Keys plays the FM-1 from a keyboard through the app, and the spyglass beside the screen shows
//! what's played (keys.rs, glass.rs), as in FM-1 Big Screen + Spyglass on Android.

mod bigscreen2;
mod chord;
mod fm1base;
mod glass;
mod glyphs;
mod install;
mod keys;
mod mellotron;
mod scope;
mod screen;
mod smooth;
mod staff;
mod usbname;
#[cfg(target_os = "macos")]
mod reopen;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align2, Color32, ColorImage, FontId, Pos2, Rect, TextureHandle, TextureOptions};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde_json::{json, Value};

use screen::{Screen, ASK_INFO, SIZE};

const APP_NAME: &str = "FM-1 Big Screen";
/// Settings stay in the folder named for the app's old name, FM-1 Mirror, so nothing is lost.
pub(crate) const SETTINGS_DIR: &str = "FM-1 Mirror";
/// How long the buttons stay after the mouse last moved.
const BAR_FOR: Duration = Duration::from_secs(4);
/// How often to say hello (the FM-1 stops sending three seconds after the last one).
const HELLO_EVERY: Duration = Duration::from_millis(900);

/// Wanted: the FM-1's own sound turned down while the Mellotron plays (the hello thread sends it, and sends it back up
/// when this goes false or the link is dropped).
static FM1_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Send "back up" once even if this app didn't turn it down (another app, the tablet's, may have): "No", or the
/// Mellotron off, means the FM-1's own sound is wanted.
static FM1_UP_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The FM-1's own sound down (0) or back up (127): CC 11, EXPRESSION, on all 16 channels. SLOOP (2.7.1+) takes it as a
/// gain over every track's LEVEL that isn't saved with the song, so nothing in a project changes and a restart is full
/// again; firmware that doesn't know it ignores it.
/// (One message at a time: Windows' MIDI takes a short message per send.)
fn send_fm1_volume(out: &mut MidiOutputConnection, up: bool) -> bool {
    (0..16u8).all(|ch| out.send(&[0xb0 | ch, 11, if up { 127 } else { 0 }]).is_ok())
}
/// Nothing from the FM-1 this long after connecting: it isn't sending its screen.
const QUIET: Duration = Duration::from_secs(4);
/// The floating spyglass: room around the eyepiece in its window, and its smallest and largest size.
const FLOAT_PAD: f32 = 14.0;
const FLOAT_SIZES: (f32, f32) = (140.0, 900.0);

/// The spyglass in a window of its own. It keeps its own state and repaints on its own: macOS stops
/// redrawing FM-1 Big Screen's window behind others, and the spyglass has to keep going then.
struct Float {
    glass: glass::Glass,
    size: f32,
    /// Where its window is on the desktop.
    at: Option<Pos2>,
    /// Its window was closed.
    close: bool,
    /// Moved or resized since the settings were saved.
    dirty: bool,
}

/// Which pixels belong to a filled area: within 4 pixels of a spot whose 9x9 neighbourhood is more than 60% drawn
/// (anything brighter than the black background, dark dotted shading included). Text strokes on black never get that
/// full; a picture, a panel or a dotted background does, and so do its edges.
fn picture_mask(px: &[u16], side: usize) -> Vec<bool> {
    const R: usize = 4;
    let ink: Vec<u32> = px
        .iter()
        .map(|&c| {
            let [r, g, b] = screen::rgb(c);
            u32::from(0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 0.03 * 255.0)
        })
        .collect();
    // Counts in each pixel's (2R+1)^2 neighbourhood (clipped at the edges), by a summed-area table.
    let window = |v: &[u32]| -> Vec<(u32, u32)> {
        let w = side + 1;
        let mut sat = vec![0u32; w * w];
        for y in 0..side {
            for x in 0..side {
                sat[(y + 1) * w + x + 1] = v[y * side + x] + sat[y * w + x + 1] + sat[(y + 1) * w + x] - sat[y * w + x];
            }
        }
        let mut out = Vec::with_capacity(side * side);
        for y in 0..side {
            let (y0, y1) = (y.saturating_sub(R), (y + R + 1).min(side));
            for x in 0..side {
                let (x0, x1) = (x.saturating_sub(R), (x + R + 1).min(side));
                let n = sat[y1 * w + x1] + sat[y0 * w + x0] - sat[y0 * w + x1] - sat[y1 * w + x0];
                out.push((n, ((y1 - y0) * (x1 - x0)) as u32));
            }
        }
        out
    };
    let core: Vec<u32> = window(&ink).into_iter().map(|(n, area)| u32::from(n * 10 > area * 6)).collect();
    window(&core).into_iter().map(|(n, _)| n > 0).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Color,
    Bright,
    Yellow,
    White,
    Inverted,
}

impl Look {
    const ALL: [Look; 5] = [Look::Color, Look::Bright, Look::Yellow, Look::White, Look::Inverted];

    fn name(self) -> &'static str {
        match self {
            Look::Color => "Color",
            Look::Bright => "Bright color",
            Look::Yellow => "Yellow on black",
            Look::White => "White on black",
            Look::Inverted => "Black on white",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Look::Color => "color",
            Look::Bright => "bright",
            Look::Yellow => "yellow",
            Look::White => "white",
            Look::Inverted => "inverted",
        }
    }

    /// The whole screen in this look. Bright color lifts only thin things on a dark background (text, lines): what
    /// lies in or next to a filled area (a picture, a panel, a dotted background) keeps its own colours, so a
    /// firmware's pictures (Erik37's piano) aren't washed out.
    fn paint_all(self, px: &[u16], side: usize) -> Vec<Color32> {
        if self != Look::Bright {
            return px.iter().map(|&c| self.paint(c)).collect();
        }
        let near = picture_mask(px, side);
        px.iter()
            .zip(near)
            .map(|(&c, pic)| if pic { Look::Color.paint(c) } else { self.paint(c) })
            .collect()
    }

    fn paint(self, c: u16) -> Color32 {
        let [r, g, b] = screen::rgb(c);
        let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;
        // The FM-1 draws light text on black: lift anything that isn't background to full strength.
        let lit = |v: f32| ((v * 1.6).min(1.0) * 255.0) as u8;
        match self {
            Look::Color => Color32::from_rgb(r, g, b),
            Look::Bright => {
                // Brighter, and any colour that isn't the black background up to at least 80%
                // brightness, keeping its hue: a dark purple on black becomes a light lilac.
                let k = |v: u8| (v as f32 * 1.6).min(255.0);
                let (r, g, b) = (k(r), k(g), k(b));
                let l = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
                let t = if l > 0.03 && l < 0.8 { (0.8 - l) / (1.0 - l) } else { 0.0 };
                let up = |v: f32| (v + (255.0 - v) * t) as u8;
                Color32::from_rgb(up(r), up(g), up(b))
            }
            Look::Yellow => {
                let l = lit(luma);
                Color32::from_rgb(l, l, 0)
            }
            Look::White => {
                let l = lit(luma);
                Color32::from_rgb(l, l, l)
            }
            Look::Inverted => {
                let l = 255 - lit(luma);
                Color32::from_rgb(l, l, l)
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    screen: Screen,
    /// When the last screen message came, and whether there's a new one to show.
    last: Option<Instant>,
    changed: bool,
    /// What the FM-1 said it is when asked ("FM-1_015", "FM-1_8000014"…).
    identity: Option<String>,
    /// The firmware build it reports (updater capability), to offer a newer Big Screen.
    build: Option<String>,
    /// What a Felucca-family firmware says it is ("FELUCCA SLOOP 2.8.4"): its USB name may only say "Felucca".
    firmware: Option<String>,
    /// The sound playing, when the firmware says it ("T1 FM6 ICE BELL"): shown after "FM-1" on top.
    sound: Option<String>,
    /// Big Screen version 2 (konsumer's): the frame being put together, shown whole.
    v2: bigscreen2::Viewer,
}

/// The FM-1's MIDI ports (USB or Bluetooth), open both ways.
struct Link {
    name: String,
    _input: MidiInputConnection<()>,
    /// Shared by the hellos and Keys.
    out: Arc<Mutex<MidiOutputConnection>>,
    opened: Instant,
    /// Stops the hello thread when the link is dropped.
    stop: Arc<std::sync::atomic::AtomicBool>,
    /// Set by the hello thread when the FM-1 has gone away.
    lost: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Keeps asking the FM-1 for its screen about once a second (the whole screen the first time:
/// someone else may be keeping the mirror on), and after a couple of seconds which firmware
/// it runs and which build, one question at a time (its updater drops a second one sent
/// straight after). On a thread of its own: macOS stops redrawing a window that's hidden behind
/// others, and the FM-1 stops sending three seconds after the last hello.
/// Also asks the other way, Charles Vestal's mirror in fm1-base firmware (fm1base.rs): three times, between the other
/// questions; once it answers (`base`, set by the input), it's kept on every two seconds (it stops after six).
fn hellos(
    output: Arc<Mutex<MidiOutputConnection>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    lost: Arc<std::sync::atomic::AtomicBool>,
    base: Arc<std::sync::atomic::AtomicBool>,
    v2_answers: Arc<std::sync::atomic::AtomicU32>,
) {
    use std::sync::atomic::Ordering;
    const BASE_ASK_AT: [u64; 3] = [1000, 5000, 6000]; // ms after opening: clear of the questions at 2, 3 and 4 s
    let opened = Instant::now();
    let (mut base_asked, mut base_at) = (0usize, Instant::now());
    let (mut asked_identity, mut asked_build, mut asked_info) = (false, false, false);
    let mut hello_at: Option<Instant> = None;
    // Big Screen version 2 first (konsumer's: whole frames, less to send); version 1 if the firmware doesn't answer.
    let mut asker = bigscreen2::Asker::default();
    let mut down_sent = false; // the FM-1 turned down on this link
    while !stop.load(Ordering::Relaxed) {
        let down = FM1_DOWN.load(Ordering::Relaxed);
        if !down && FM1_UP_ONCE.swap(false, Ordering::Relaxed) {
            down_sent = true; // (so it's sent below)
        }
        if down != down_sent {
            let ok = send_fm1_volume(&mut output.lock().unwrap(), !down);
            eprintln!("[down] sent {} ok={ok}", if down { "DOWN" } else { "UP" });
            if ok {
                down_sent = down;
            }
        }
        if hello_at.is_none_or(|t| t.elapsed() >= asker.every(HELLO_EVERY)) {
            hello_at = Some(Instant::now());
            let hello = asker.next(v2_answers.load(Ordering::Relaxed));
            if output.lock().unwrap().send(&hello).is_err() {
                lost.store(true, Ordering::Relaxed);
                return;
            }
        }
        if !asked_identity && opened.elapsed() > Duration::from_secs(2) {
            asked_identity = true;
            let _ = output.lock().unwrap().send(&install::frame(17, &[]));
        }
        if !asked_build && opened.elapsed() > Duration::from_secs(3) {
            asked_build = true;
            let _ = output.lock().unwrap().send(&install::frame(49, &[]));
        }
        if !asked_info && opened.elapsed() > Duration::from_secs(4) {
            asked_info = true;
            let _ = output.lock().unwrap().send(&ASK_INFO);
        }
        if base.load(Ordering::Relaxed) {
            if base_at.elapsed() >= Duration::from_secs(2) {
                base_at = Instant::now();
                let _ = output.lock().unwrap().send(&fm1base::request(1));
            }
        } else if base_asked < BASE_ASK_AT.len() && opened.elapsed() > Duration::from_millis(BASE_ASK_AT[base_asked]) {
            base_asked += 1;
            base_at = Instant::now();
            let _ = output.lock().unwrap().send(&fm1base::request(2)); // on, and the whole screen
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if down_sent {
        send_fm1_volume(&mut output.lock().unwrap(), true); // let go: the FM-1's own sound back up
    }
}

/// The FM-1's MIDI port: "FM1 Community" with Open firmware, "USB Composite Device" with the
/// original on a Mac (which has no mirror, but is worth recognising to say so). On Windows, close
/// other apps using the FM-1 first: there a MIDI port can only be open in one app at a time.
fn is_fm1(name: &str) -> bool {
    let n = name.to_lowercase();
    // Windows may list it as just "USB-Midi" (a keyboard's "… (USB MIDI)" doesn't count).
    // Its updater, waiting mid-update, may have a name of its own.
    let updater = ["felucca", "sloop", "clipper", "ota", "sinco"].iter().any(|h| n.contains(h));
    // The Felucca family names itself: JIANT FM "JIANT" (its updater "JIANT Update" is not one to mirror), ChoralRoot.
    let felucca = (n.contains("jiant") && !n.contains("update")) || n.contains("choralroot");
    (n.contains("fm1") || n.contains("fm-1") || felucca || n.contains("usb composite") || n.starts_with("usb-midi") || n.starts_with("usb midi") || updater)
        && !n.contains("bridge")
}

/// The FM-1 over Bluetooth MIDI ("FM1 Community_BLE"): connected once in Audio MIDI Setup on a Mac,
/// then it's a MIDI port like USB. The Big Screen firmware answers on the port that asked, so the
/// screen comes over it too (slower than USB). Firmware is only ever installed over USB.
fn is_bluetooth(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("_ble") || n.contains(" ble") || n.contains("bluetooth")
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(SETTINGS_DIR).join("settings.json"))
}

struct App {
    shared: Arc<Mutex<Shared>>,
    link: Option<Link>,
    /// For seeing whether the FM-1 is still plugged in.
    scanner: Option<MidiOutput>,
    looked: Option<Instant>,
    look: Look,
    full_screen: bool,
    texture: Option<TextureHandle>,
    last_move: Instant,
    /// Installing the Big Screen firmware: how it's going.
    install: Option<Arc<Mutex<install::Progress>>>,
    /// "Install Big Screen…" was clicked: the last check before it starts.
    confirming: bool,
    /// What's been played (the keyboard, and the FM-1's own keys), for the spyglass.
    played: Arc<Mutex<keys::Played>>,
    sound_in: Arc<Mutex<scope::Ring>>,
    listener: Option<scope::Listener>,
    keys: Option<keys::Keys>,
    /// "" any keyboard plugged in, "-" none, or a port's name; and everything to channel 1.
    keys_choice: String,
    /// Where the keyboard's notes go: the FM-1's link while there is one. The keyboard stays open without it, for the
    /// spyglass: the show goes on.
    keys_out: Arc<Mutex<Option<Arc<Mutex<MidiOutputConnection>>>>>,
    /// The channel Keys sends on: 1 to 16, or 0 for as they come.
    keys_channel: u8,
    keys_changed: bool,
    glass: glass::Glass,
    glass_on: bool,
    /// Where the spyglass is: its centre as a fraction of the screen area, and its diameter in points.
    glass_at: Option<(f32, f32)>,
    glass_size: Option<f32>,
    /// The FM-1's screen shows (off: only the spyglass, or nothing).
    mirror_on: bool,
    /// The FM-1's screen with the jaggies taken off (off: exactly as it sends it).
    smooth_on: bool,
    /// The built-in Mellotron while it's on (flute, strings or choir): the FM-1's keys play it, out of this computer.
    mellotron: Option<mellotron::Mellotron>,
    mellotron_sound: String,
    /// "always": the FM-1's own sound is turned down whenever the Mellotron plays; "ask": a question each time it comes on.
    fm1_down_pref: String,
    /// The question is showing ("Two sounds?").
    ask_down: bool,
    /// Where the main window is (its top left, in points), kept so it opens there next time (the same screen).
    main_at: Option<Pos2>,
    /// "Mellotron sounds": the list online (None while it's being fetched), a message, a sound just downloaded.
    sounds_open: bool,
    sounds_list: Arc<Mutex<Option<Result<Vec<mellotron::Pack>, String>>>>,
    sounds_note: Arc<Mutex<Option<String>>>,
    sounds_ready: Arc<Mutex<Option<String>>>,
    /// The spyglass floats in a window of its own, outside this one.
    glass_float: bool,
    float: Arc<Mutex<Float>>,
    /// The size and place its window opens with.
    float_open: (f32, Option<Pos2>),
}

/// Why there's no picture yet.
enum Need {
    Plug,
    Connecting,
    /// M-VAVE's original firmware: first M-VAVE's Official Open (on M-VAVE's page), then this app.
    Original,
    /// M-VAVE's Open firmware without the mirror: this app can install Big Screen.
    Open,
    /// In its updater, waiting: an update stopped partway (this app can finish it).
    Updater,
    /// A community firmware whose regular release doesn't send its screen: its Big Screen version does.
    Community(screen::Community),
    Other(String),
    Unknown(String),
    /// Over Bluetooth with a Big Screen build that only sends its screen over USB.
    Bluetooth,
}

const WORKBENCH: &str = "https://open.m-vave.com/fm1-workbench";

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

impl App {
    fn new(cc: &eframe::CreationContext) -> App {
        let ctx = &cc.egui_ctx;
        ctx.set_visuals(egui::Visuals::dark());
        // Big type everywhere: this is for reading.
        let mut style = (*ctx.style()).clone();
        for (ts, f) in style.text_styles.iter_mut() {
            f.size = match ts {
                egui::TextStyle::Heading => 34.0,
                egui::TextStyle::Small => 18.0,
                _ => 24.0,
            };
        }
        style.spacing.button_padding = vec2(18.0, 10.0);
        style.spacing.item_spacing = vec2(12.0, 12.0);
        style.spacing.interact_size.y = 44.0;
        ctx.set_style(style);
        let saved: Value = settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let float_size = saved["spyglass_float_size"].as_f64().map_or(320.0, |d| d as f32).clamp(FLOAT_SIZES.0, FLOAT_SIZES.1);
        let float_at = saved["spyglass_float_x"].as_f64().zip(saved["spyglass_float_y"].as_f64()).map(|(x, y)| Pos2::new(x as f32, y as f32));
        let mut app = App {
            shared: Arc::default(),
            link: None,
            scanner: MidiOutput::new(APP_NAME).ok(),
            looked: None,
            look: Look::ALL.into_iter().find(|l| saved["look"] == l.key()).unwrap_or(Look::Color),
            full_screen: false,
            texture: None,
            last_move: Instant::now(),
            install: None,
            confirming: false,
            played: Arc::default(),
            sound_in: Arc::default(),
            listener: None,
            keys: None,
            keys_choice: saved["keys"].as_str().unwrap_or("").to_string(),
            keys_out: Arc::new(Mutex::new(None)),
            // (an older "all to channel 1" switch carries over)
            keys_channel: saved["keys_channel"].as_u64().map(|c| c.min(16) as u8).unwrap_or(if saved["keys_to_one"].as_bool().unwrap_or(true) { 1 } else { 0 }),
            keys_changed: false,
            glass: glass::Glass::default(),
            glass_on: saved["spyglass"].as_bool().unwrap_or(true),
            glass_at: saved["spyglass_x"].as_f64().zip(saved["spyglass_y"].as_f64()).map(|(x, y)| (x as f32, y as f32)),
            glass_size: saved["spyglass_size"].as_f64().map(|d| d as f32),
            mirror_on: saved["mirror"].as_bool().unwrap_or(true),
            smooth_on: true, // always on at start (S or the button turns it off for this session)
            mellotron: None,
            mellotron_sound: saved["mellotron"].as_str().filter(|s| mellotron::installed().iter().any(|i| i == s)).unwrap_or("").to_string(),
            fm1_down_pref: saved["fm1_down"].as_str().unwrap_or("ask").to_string(),
            ask_down: false,
            main_at: None,
            sounds_open: false,
            sounds_list: Arc::default(),
            sounds_note: Arc::default(),
            sounds_ready: Arc::default(),
            glass_float: saved["spyglass_float"].as_bool().unwrap_or(false),
            float: Arc::new(Mutex::new(Float { glass: glass::Glass::default(), size: float_size, at: float_at, close: false, dirty: false })),
            float_open: (float_size, float_at),
        };
        app.start_mellotron();   // the sound it played last time
        if !app.mellotron_sound.is_empty() {
            app.mellotron_came_on();
        }
        app
    }

    fn store(&self) {
        let Some(path) = settings_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let mut s = json!({ "look": self.look.key(), "keys": self.keys_choice, "keys_channel": self.keys_channel, "spyglass": self.glass_on, "mirror": self.mirror_on, "smooth": self.smooth_on, "spyglass_float": self.glass_float, "mellotron": self.mellotron_sound, "fm1_down": self.fm1_down_pref });
        if let Some(at) = self.main_at {
            s["window_x"] = json!(at.x);
            s["window_y"] = json!(at.y);
        }
        {
            let f = self.float.lock().unwrap();
            s["spyglass_float_size"] = json!(f.size);
            if let Some(at) = f.at {
                s["spyglass_float_x"] = json!(at.x);
                s["spyglass_float_y"] = json!(at.y);
            }
        }
        if let Some((x, y)) = self.glass_at {
            s["spyglass_x"] = json!(x);
            s["spyglass_y"] = json!(y);
        }
        if let Some(d) = self.glass_size {
            s["spyglass_size"] = json!(d);
        }
        let _ = std::fs::write(path, s.to_string());
    }

    /// Finds and opens the FM-1 (now and then while it's not there), and keeps saying hello.
    fn keep_linked(&mut self, ctx: &egui::Context) {
        // Look every two seconds, with the one client: making CoreMIDI clients on every frame
        // swamped the MIDI server so that no hello got through to the FM-1.
        let now = Instant::now();
        // While installing, the installer has the FM-1 to itself; afterwards, start afresh.
        if let Some(p) = &self.install {
            let p = p.lock().unwrap();
            if !(p.done || p.error.is_some()) {
                self.link = None;
                ctx.request_repaint_after(Duration::from_millis(200));
                return;
            }
        }
        if self.looked.is_none_or(|t| now - t >= Duration::from_secs(1)) {
            self.looked = Some(now);
            if let Some(link) = &self.link {
                let still_there = self.scanner.as_ref().is_some_and(|o| o.ports().iter().any(|p| o.port_name(p).is_ok_and(|n| n == link.name)));
                // Plugged in while on Bluetooth: USB is much faster, so move over.
                let usb_now = is_bluetooth(&link.name)
                    && self.scanner.as_ref().is_some_and(|o| o.ports().iter().any(|p| o.port_name(p).is_ok_and(|n| is_fm1(&n) && !is_bluetooth(&n))));
                if !still_there || usb_now || link.lost.load(std::sync::atomic::Ordering::Relaxed) {
                    self.link = None;
                }
            }
            if self.link.is_none() {
                // macOS may still call it by an old firmware's name ("ChoralRoot FM-1" for LOOM-8): the FM-1's own
                // USB name again, for every app (usbname.rs)
                for (old, new) in usbname::fix_stale_names(is_fm1) {
                    eprintln!("renamed the USB MIDI device \"{old}\" to \"{new}\", the name the FM-1 reports");
                }
                {
                    let mut s = self.shared.lock().unwrap();
                    s.identity = None;
                    s.build = None;
                    s.firmware = None;
                    s.sound = None;
                }
                self.link = open(self.shared.clone(), self.played.clone(), ctx.clone());
            }
            *self.keys_out.lock().unwrap() = self.link.as_ref().map(|l| l.out.clone());
            self.keep_keys(ctx);
            if self.glass_on && self.listener.as_ref().is_none_or(|l| l.dead()) {
                self.listener = scope::open(self.sound_in.clone());
            } else if !self.glass_on {
                self.listener = None;
            }
        }
        ctx.request_repaint_after(HELLO_EVERY);
    }

    /// The keyboard, FM-1 or not (the spyglass shows it either way; it plays the FM-1 while there is one); let go if
    /// it's unplugged or another is chosen.
    fn keep_keys(&mut self, ctx: &egui::Context) {
        let sources = keys::sources();
        if let Some(k) = &self.keys {
            if self.keys_changed || !sources.contains(&k.name) {
                self.keys = None;
                if let Some(link) = &self.link {
                    keys::silence(&link.out);
                }
            }
        }
        self.keys_changed = false;
        if self.keys.is_none() {
            let want = match self.keys_choice.as_str() {
                "-" => None,
                "" => keys::keyboard_first(&sources),
                name => sources.iter().find(|n| *n == name).cloned(),
            };
            if let Some(name) = want {
                self.keys = keys::Keys::open(&name, self.keys_out.clone(), self.played.clone(), self.keys_channel, ctx.clone());
            }
        }
    }

    fn take_screen(&mut self, ctx: &egui::Context) {
        let pixels = {
            let mut s = self.shared.lock().unwrap();
            if s.v2.overdue() {
                // version 2 with no SYNC for a while (the screen changes faster than it can be sent): show what came
                let s = &mut *s;
                s.v2.show(&mut s.screen.pixels);
                s.last = Some(Instant::now());
                s.changed = true;
            }
            if !s.changed {
                return;
            }
            s.changed = false;
            s.screen.pixels.clone()
        };
        let painted = self.look.paint_all(&pixels, SIZE);
        // Nearest: the FM-1's 5x7 letters stay crisp blocks however big they get. Smooth: a bigger copy, blended.
        let (img, options) = if self.smooth_on {
            let side = SIZE * smooth::SCALE;
            (ColorImage { size: [side, side], pixels: smooth::smooth(&painted, SIZE) }, TextureOptions::LINEAR)
        } else {
            (ColorImage { size: [SIZE, SIZE], pixels: painted }, TextureOptions::NEAREST)
        };
        match &mut self.texture {
            Some(t) => t.set(img, options),
            None => self.texture = Some(ctx.load_texture("fm1", img, options)),
        }
    }

    /// The Mellotron button: steps through the sounds this computer has (built in or downloaded), then off
    /// (remembered). With none yet (the lite version) it opens "Mellotron sounds"; a right-click always does.
    fn next_mellotron(&mut self, ctx: &egui::Context) {
        let have = mellotron::installed();
        if have.is_empty() {
            self.open_sounds(ctx);
            return;
        }
        let i = have.iter().position(|s| *s == self.mellotron_sound);
        let next = match i {
            None => have[0].clone(),
            Some(i) if i + 1 < have.len() => have[i + 1].clone(),
            _ => String::new(),
        };
        self.set_mellotron(next);
    }

    fn set_mellotron(&mut self, sound: String) {
        self.mellotron = None;                 // (dropping it stops the sound)
        let was_on = !self.mellotron_sound.is_empty();
        self.mellotron_sound = sound;
        self.start_mellotron();
        self.store();
        if self.mellotron_sound.is_empty() {
            FM1_DOWN.store(false, std::sync::atomic::Ordering::Relaxed);
            FM1_UP_ONCE.store(true, std::sync::atomic::Ordering::Relaxed);
            self.ask_down = false;
        } else if !was_on {
            self.mellotron_came_on();
        }
    }

    /// The Mellotron came on: turn the FM-1's own sound down if "always", else ask.
    fn mellotron_came_on(&mut self) {
        if self.fm1_down_pref == "always" {
            FM1_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.ask_down = true;
        }
    }

    /// A big, easy-to-read button: black bold text on yellow, like the Android app's.
    fn big_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add(egui::Button::new(egui::RichText::new(text).size(20.0).strong().color(Color32::BLACK))
            .fill(Color32::from_rgb(0xff, 0xd6, 0x0a)).min_size(vec2(0.0, 40.0)))
    }

    /// "Two sounds?": turn the FM-1's own sound down while the Mellotron plays: always, just this time, or no.
    fn ask_down_window(&mut self, ctx: &egui::Context) {
        if !self.ask_down {
            return;
        }
        eprintln!("[down] asking (pref {})", self.fm1_down_pref);
        let mut choice = None;
        egui::Window::new("Two sounds?").collapsible(false).resizable(false).title_bar(true)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.label(egui::RichText::new("While the Mellotron plays, the FM-1 plays its own sound too. Turn the FM-1's own sound down while the Mellotron is on? It comes back up when you turn the Mellotron off.").size(20.0));
                ui.label(egui::RichText::new("(Works with SLOOP; other firmware may ignore it.)").size(16.0));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if Self::big_button(ui, "Always").clicked() {
                        choice = Some(2);
                    }
                    if Self::big_button(ui, "Just this time").clicked() {
                        choice = Some(1);
                    }
                    if Self::big_button(ui, "No").clicked() {
                        choice = Some(0);
                    }
                });
            });
        if let Some(c) = choice {
            eprintln!("[down] chose {c}");
            self.ask_down = false;
            if c == 2 {
                self.fm1_down_pref = "always".into();
                self.store();
            }
            FM1_DOWN.store(c > 0 && !self.mellotron_sound.is_empty(), std::sync::atomic::Ordering::Relaxed);
            if c == 0 {
                FM1_UP_ONCE.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    fn start_mellotron(&mut self) {
        if !self.mellotron_sound.is_empty() && self.mellotron.is_none() {
            self.mellotron = mellotron::Mellotron::start(&self.mellotron_sound);
        }
    }

    /// Opens "Mellotron sounds" and fetches the list online (the only time the app goes online, with a download).
    fn open_sounds(&mut self, ctx: &egui::Context) {
        self.sounds_open = true;
        *self.sounds_list.lock().unwrap() = None;
        let (list, ctx) = (self.sounds_list.clone(), ctx.clone());
        std::thread::spawn(move || {
            *list.lock().unwrap() = Some(mellotron::list());
            ctx.request_repaint();
        });
    }

    fn sounds_window(&mut self, ctx: &egui::Context) {
        let ready = self.sounds_ready.lock().unwrap().take();
        if let Some(id) = ready {
            self.set_mellotron(id);            // a sound just downloaded: play it straight away
        }
        if !self.sounds_open {
            return;
        }
        let mut open = true;
        let list = self.sounds_list.lock().unwrap().clone();
        let note = self.sounds_note.lock().unwrap().clone();
        let mut remove: Option<String> = None;
        egui::Window::new("Mellotron sounds").open(&mut open).collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                // the FM-1's own sound: what it is right now, and what happens when the Mellotron comes on
                let down = FM1_DOWN.load(std::sync::atomic::Ordering::Relaxed);
                let on = !self.mellotron_sound.is_empty();
                ui.label(egui::RichText::new("The FM-1's own sound").strong());
                ui.horizontal(|ui| {
                    ui.label(if down { "Right now: turned down (you hear only the Mellotron)." }
                        else if on { "Right now: playing (you hear the FM-1 and the Mellotron)." }
                        else { "Right now: playing (the Mellotron is off)." });
                    if down && Self::big_button(ui, "Turn the FM-1 back up").clicked() {
                        FM1_DOWN.store(false, std::sync::atomic::Ordering::Relaxed);
                        FM1_UP_ONCE.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    if !down && on && Self::big_button(ui, "Turn the FM-1 down").clicked() {
                        FM1_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
                        self.ask_down = false;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("When the Mellotron comes on:");
                    let always = self.fm1_down_pref == "always";
                    // the choice in force is yellow; the other is plain
                    let choice = |ui: &mut egui::Ui, text: &str, chosen: bool| {
                        if chosen { Self::big_button(ui, text) } else { ui.add(egui::Button::new(egui::RichText::new(text).size(20.0)).min_size(vec2(0.0, 40.0))) }
                    };
                    if choice(ui, "Ask me", !always).clicked() && always {
                        self.fm1_down_pref = "ask".into();
                        self.store();
                    }
                    if choice(ui, "Always turn the FM-1 down", always).clicked() && !always {
                        self.fm1_down_pref = "always".into();
                        if on {
                            FM1_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
                            self.ask_down = false;
                        }
                        self.store();
                    }
                });
                ui.separator();
                match &list {
                    None => {
                        ui.label("Getting the list of sounds…");
                    }
                    Some(Err(e)) => {
                        ui.label(format!("The list of sounds can't be reached now ({e}). Check the internet connection and try again."));
                        let have = mellotron::installed();
                        if !have.is_empty() {
                            ui.label(format!("On this computer: {}.", have.join(", ")));
                        }
                    }
                    Some(Ok(packs)) => {
                        egui::Grid::new("sounds").spacing(vec2(24.0, 12.0)).show(ui, |ui| {
                            for p in packs {
                                ui.label(&p.name);
                                if mellotron::bundled(&p.id) {
                                    ui.label("built in");
                                } else if mellotron::downloaded(&p.id) {
                                    if ui.button("Remove").clicked() {
                                        remove = Some(p.id.clone());
                                    }
                                } else if note.as_deref().is_some_and(|n| n.starts_with("Downloading")) {
                                    ui.label(format!("{:.1} MB", p.bytes as f64 / 1048576.0));
                                } else if ui.button(format!("Download ({:.1} MB)", p.bytes as f64 / 1048576.0)).clicked() {
                                    let (p, note, ready, ctx) = (p.clone(), self.sounds_note.clone(), self.sounds_ready.clone(), ctx.clone());
                                    *note.lock().unwrap() = Some(format!("Downloading {}…", p.name));
                                    std::thread::spawn(move || {
                                        match mellotron::download(&p) {
                                            Ok(()) => {
                                                *note.lock().unwrap() = Some(format!("{} is ready: the Mellotron button steps through it.", p.name));
                                                *ready.lock().unwrap() = Some(p.id.clone());
                                            }
                                            Err(e) => *note.lock().unwrap() = Some(format!("{} couldn't be downloaded ({e}). Try again later.", p.name)),
                                        }
                                        ctx.request_repaint();
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    }
                }
                if let Some(n) = &note {
                    ui.add_space(8.0);
                    ui.label(n);
                }
            });
        if let Some(id) = remove {
            if self.mellotron_sound == id {
                self.set_mellotron(String::new());
            }
            mellotron::remove(&id);
        }
        if !open {
            self.sounds_open = false;
            *self.sounds_note.lock().unwrap() = None;
        }
        if self.sounds_note.lock().unwrap().as_deref().is_some_and(|n| n.starts_with("Downloading")) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn toggle_smooth(&mut self) {
        self.smooth_on = !self.smooth_on;
        self.mirror_on = true; // it's the mirror that's smooth: show it
        self.shared.lock().unwrap().changed = true; // repaint it the other way
        self.store();
    }

    /// Why there's no picture, if there isn't one.
    fn need(&self) -> Option<Need> {
        let (last, identity, firmware) = {
            let s = self.shared.lock().unwrap();
            (s.last, s.identity.clone(), s.firmware.clone())
        };
        let Some(link) = &self.link else { return Some(Need::Plug) };
        // The FM-1 only sends what changes, so a still screen is quiet: once it has come, keep it.
        if last.is_some_and(|t| t >= link.opened) {
            return None;
        }
        if link.opened.elapsed() < QUIET {
            return Some(Need::Connecting);
        }
        let known = screen::community(firmware.as_deref(), identity.as_deref(), &link.name);
        Some(match identity {
            Some(id) if is_bluetooth(&link.name) && !install::is_updater(&id) => Need::Bluetooth, // installs need USB
            _ if known.is_some() => Need::Community(known.unwrap()),
            Some(id) if id.starts_with("FM-1_0") => Need::Original,
            Some(id) if install::is_open_firmware(&id) => Need::Open,
            Some(id) if install::is_updater(&id) => Need::Updater,
            Some(id) => Need::Other(id),
            None if link.name.to_lowercase().contains("usb composite") => Need::Original,
            None => Need::Unknown(link.name.clone()),
        })
    }

    /// Instead of the screen: what to do, step by step, in big type.
    fn need_panel(&mut self, ui: &mut egui::Ui, need: Need) {
        let body = |ui: &mut egui::Ui, t: &str| {
            ui.label(egui::RichText::new(t).size(24.0));
        };
        let heading = |ui: &mut egui::Ui, t: &str| {
            ui.label(egui::RichText::new(t).size(34.0).strong().color(Color32::WHITE));
        };
        let step1 = |ui: &mut egui::Ui| {
            body(ui, "Step 1 · On M-VAVE's FM1 Workbench page (in Chrome or Edge), put M-VAVE's free Official Open firmware on it: Device update › Recovery & force update › tick \"Force flash\" › Connect device › tick \"I have backed up…\" › Force flash. It replaces the FM-1's sounds with M-VAVE's: if you made your own, save them first on the page's Projects & sounds tab.");
            if ui.button("Open M-VAVE's page").clicked() {
                open_url(WORKBENCH);
            }
            body(ui, "Step 2 · Come back to this window: it will offer to install Big Screen.");
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(820.0);
            ui.add_space(30.0);
            if let Some(progress) = self.install.clone() {
                let p = progress.lock().unwrap().clone();
                if let Some(e) = p.error {
                    heading(ui, "The install stopped");
                    body(ui, &format!("{}{}.", e[..1].to_uppercase(), &e[1..]));
                    body(ui, "Leave the FM-1 plugged in and on, close other apps that use it, and click Try again: it carries on where it stopped. If the FM-1 is plugged in through a hub, plug it straight into the computer. M-VAVE's page (or M-Upgrade) can also always install firmware on it.");
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            self.start_install();
                        }
                        if ui.button("Close").clicked() {
                            self.install = None;
                        }
                    });
                    return;
                }
                if !p.done {
                    heading(ui, "Installing Big Screen…");
                    body(ui, &p.step);
                    ui.add(egui::ProgressBar::new(p.percent as f32 / 100.0).show_percentage().desired_height(36.0));
                    body(ui, "Keep the FM-1 plugged in and on. It restarts by itself at the end (about 2 minutes in all).");
                    return;
                }
                heading(ui, "Big Screen is installed");
                body(ui, "The FM-1 is coming back; its screen will appear here.");
                return;
            }
            match need {
                Need::Plug => {
                    heading(ui, "No FM-1 yet");
                    body(ui, "Plug it into this computer with its USB cable and turn it on.");
                    #[cfg(target_os = "macos")]
                    {
                        body(ui, "Or over Bluetooth: turn on the FM-1's Bluetooth (hold HOME and press GLO: K1 BLUETOOTH ON, K3 BLE ROLE DEVICE). Then in Audio MIDI Setup, Window › Show MIDI Studio, click the Bluetooth button and Connect next to FM1 Community_BLE. It appears here by itself.");
                        if ui.button("Open Audio MIDI Setup").clicked() {
                            let _ = std::process::Command::new("open").args(["-a", "Audio MIDI Setup"]).spawn();
                        }
                    }
                }
                Need::Connecting => heading(ui, "Connecting to the FM-1…"),
                Need::Original => {
                    heading(ui, "This FM-1 has M-VAVE's original firmware");
                    body(ui, "To show its screen here it needs the FM-1 Big Screen firmware: M-VAVE's free Official Open firmware plus one small addition that sends the screen to this app. Two steps, and you can go back to the original whenever you like.");
                    step1(ui);
                }
                Need::Open => {
                    heading(ui, "This FM-1 runs M-VAVE's Open firmware");
                    body(ui, "Install the Big Screen version of it: the same firmware, plus sending its screen to this app. It takes about 2 minutes.");
                    if self.confirming {
                        body(ui, "Ready? Keep the FM-1 plugged in and on until it's done, and close other apps that use it (on Windows only one app can use it at a time). If you made your own sounds or projects, saving them first on M-VAVE's page is always a good idea.");
                        ui.horizontal(|ui| {
                            if ui.button("Install now").clicked() {
                                self.confirming = false;
                                self.start_install();
                            }
                            if ui.button("Cancel").clicked() {
                                self.confirming = false;
                            }
                        });
                    } else if ui.button("Install Big Screen…").clicked() {
                        self.confirming = true;
                    }
                }
                Need::Updater => {
                    heading(ui, "The FM-1 is waiting in its updater");
                    body(ui, "An update stopped partway, so the FM-1 is waiting for the rest of its firmware. Nothing is broken. Keep it plugged in and on, close other apps that use it, and finish installing Big Screen here (about 2 minutes). M-VAVE's page can also finish it.");
                    if ui.button("Finish installing Big Screen").clicked() {
                        self.start_install();
                    }
                }
                Need::Community(c) => {
                    heading(ui, &format!("This FM-1 runs {}", c.name));
                    body(ui, &format!("It's fine to keep using it. But the regular {} doesn't send its screen, so it can't show here: FM-1 Big Screen needs the Big Screen version of the firmware on the FM-1.", c.name));
                    if c.released {
                        body(ui, &format!("Install {}: the same {}, plus sending its screen to this app. Its page has the download and how to install it. Then plug the FM-1 in again and open this app.", c.build, c.name));
                        if ui.button(format!("Open the {} + Big Screen page", c.name)).clicked() {
                            open_url(c.url);
                        }
                    } else if c.name == "LOOM-8" {
                        body(ui, "LOOM-8 shows its own screen big on its Mirror page (Chrome or Edge, over USB).");
                        if ui.button("Open LOOM-8's Mirror page").clicked() {
                            open_url(c.url);
                        }
                    } else {
                        body(ui, &format!("A Big Screen version of {} is made and offered to its author, but there's no download yet. Its page says where it stands.", c.name));
                        if ui.button(format!("About {} + Big Screen", c.name)).clicked() {
                            open_url(c.url);
                        }
                    }
                    body(ui, "The spyglass (notes, chords, the staff) works with any firmware, Big Screen or not: turn it on with Spyglass, and play through Keys.");
                }
                Need::Other(id) => {
                    heading(ui, "This FM-1 runs another firmware");
                    body(ui, &format!("It says it is \"{id}\". It's fine to keep using it, and nothing needs changing. This firmware doesn't send its screen, so it can't show here."));
                    body(ui, "Only if you want to see the screen here: Big Screen replaces this firmware, and it installs over M-VAVE's Official Open firmware, so that comes first.");
                    step1(ui);
                }
                Need::Bluetooth => {
                    heading(ui, "No screen over Bluetooth yet");
                    body(ui, "This FM-1 sends its screen over USB only. The newer Big Screen firmware in this app sends it over Bluetooth too: plug the FM-1 in with its USB cable once, and this app offers to update it (about 2 minutes). Installing over Bluetooth isn't possible.");
                }
                Need::Unknown(name) => {
                    heading(ui, "No screen from the FM-1 yet");
                    body(ui, &format!("Connected to \"{name}\", but it isn't sending its screen and didn't say which firmware it runs. If you're using a community firmware, it's fine to keep it, and nothing needs changing: it just can't show here."));
                    body(ui, "Only if you want to see the screen here, or if this FM-1 has M-VAVE's original firmware:");
                    step1(ui);
                }
            }
            ui.add_space(24.0);
            ui.label(
                egui::RichText::new(
                    "Going back is easy: on M-VAVE's page, Device update › Start update puts back M-VAVE's own Official Open firmware, and Recovery & force update › Restore original V15 the firmware it came with.",
                )
                .size(20.0)
                .color(Color32::GRAY),
            );
        });
    }

    fn start_install(&mut self) {
        let progress: Arc<Mutex<install::Progress>> = Arc::default();
        self.install = Some(progress.clone());
        self.link = None; // the installer has the FM-1 to itself
        self.shared.lock().unwrap().last = None;
        std::thread::spawn(move || install::install(progress));
    }

    fn buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            let shown = match &self.keys {
                Some(k) => format!("Keys: {}", k.name),
                None if self.keys_choice == "-" => "Keys: none".into(),
                None => "Keys…".into(),
            };
            let (choice, channel) = (self.keys_choice.clone(), self.keys_channel);
            egui::ComboBox::from_id_salt("keys").selected_text(shown).width(300.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.keys_choice, String::new(), "Any keyboard plugged in");
                for name in keys::sources() {
                    ui.selectable_value(&mut self.keys_choice, name.clone(), name);
                }
                ui.selectable_value(&mut self.keys_choice, "-".to_string(), "None");
                ui.separator();
                ui.label("Send on");
                ui.selectable_value(&mut self.keys_channel, 1, "Channel 1 (the FM-1's own: the usual one)");
                for c in 2..=16u8 {
                    ui.selectable_value(&mut self.keys_channel, c, format!("Channel {c}"));
                }
                ui.selectable_value(&mut self.keys_channel, 0, "As they come (each note on its own channel)");
            });
            if self.keys_choice != choice || self.keys_channel != channel {
                self.keys_changed = true;
                self.looked = None; // connect it now
                self.store();
            }
            if ui.button(if self.mirror_on { "Hide mirror" } else { "Show mirror" }).clicked() {
                self.mirror_on = !self.mirror_on;
                self.store();
            }
            if ui.selectable_label(self.smooth_on, if self.smooth_on { "Smooth: on" } else { "Smooth: off" }).clicked() {
                self.toggle_smooth();
            }
            let mel = if !self.mellotron_sound.is_empty() {
                format!("Mellotron: {}", self.mellotron_sound)
            } else if mellotron::installed().is_empty() {
                "Mellotron: get sounds".to_string()
            } else {
                "Mellotron: off".to_string()
            };
            let r = ui.selectable_label(!self.mellotron_sound.is_empty(), mel)
                .on_hover_text("A Mellotron the FM-1's keys play, out of this computer: click for the next sound (or off), right-click for Mellotron sounds to download");
            if r.clicked() {
                self.next_mellotron(ctx);
            }
            if r.secondary_clicked() {
                self.open_sounds(ctx);
            }
            if ui.selectable_label(self.glass_on, "Spyglass").clicked() {
                self.glass_on = !self.glass_on;
                self.store();
            }
            if self.glass_on && ui.selectable_label(self.glass_float, "Float").clicked() {
                self.glass_float = !self.glass_float;
                let f = self.float.lock().unwrap();
                self.float_open = (f.size, f.at);
                drop(f);
                self.store();
            }
            let before = self.look;
            egui::ComboBox::from_id_salt("look").selected_text(self.look.name()).width(260.0).show_ui(ui, |ui| {
                for l in Look::ALL {
                    ui.selectable_value(&mut self.look, l, l.name());
                }
            });
            if self.look != before {
                self.shared.lock().unwrap().changed = true; // repaint in the new look
                self.store();
            }
            if ui.button(if self.full_screen { "Leave full screen" } else { "Full screen" }).clicked() {
                self.full_screen = !self.full_screen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full_screen));
            }
            // A Big Screen FM-1 on another build than the one in this app: offer it.
            let other_build = self.shared.lock().unwrap().build.clone().filter(|b| *b != install::build_id());
            let showing = self.shared.lock().unwrap().last.is_some();
            let wired = self.link.as_ref().is_some_and(|l| !is_bluetooth(&l.name));
            // Only over M-VAVE's Open firmware: another firmware with the mirror (ChoralRoot + Big Screen) stays.
            let open = self.shared.lock().unwrap().identity.as_deref().is_some_and(install::is_open_firmware);
            if showing && wired && open && other_build.is_some() && self.install.is_none() {
                if self.confirming {
                    if ui.button("Install now (keep the FM-1 plugged in)").clicked() {
                        self.confirming = false;
                        self.start_install();
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirming = false;
                    }
                } else if ui.button("Update the FM-1's Big Screen firmware…").clicked() {
                    self.confirming = true;
                }
            }
            let (firmware, identity) = {
                let s = self.shared.lock().unwrap();
                (s.firmware.clone(), s.identity.clone())
            };
            let status = match &self.link {
                Some(_) if firmware.is_some() => format!("FM-1: {}", screen::firmware_name(firmware.as_deref().unwrap_or(""), identity.as_deref())),
                Some(l) => format!("FM-1: {}", l.name),
                None => "FM-1: not found".into(),
            };
            ui.label(egui::RichText::new(status).color(Color32::GRAY));
            // The sound playing, standing out: big, bold, in gold on a dark chip.
            if let Some(sound) = self.shared.lock().unwrap().sound.clone() {
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x2a, 0x22, 0x0a))
                    .stroke(egui::Stroke::new(1.5_f32, Color32::from_rgb(0xf2, 0xc0, 0x4c)))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("\u{266a} {sound}")).size(20.0).strong().color(Color32::from_rgb(0xf2, 0xc0, 0x4c)));
                    });
            }
        });
    }
}

/// Opens the first FM-1 found, both ways; its screen messages go into `shared`.
fn open(shared: Arc<Mutex<Shared>>, played: Arc<Mutex<keys::Played>>, ctx: egui::Context) -> Option<Link> {
    let mut input = MidiInput::new(APP_NAME).ok()?;
    input.ignore(Ignore::None); // SysEx is the whole point
    let output = MidiOutput::new(APP_NAME).ok()?;
    // An FM-1 that names itself first (Open firmware), then one that doesn't (the original).
    let ports = input.ports();
    // USB before Bluetooth; one that names itself (Open firmware) before the original's "USB Composite".
    let usb = |p: &&midir::MidiInputPort| input.port_name(p).is_ok_and(|n| is_fm1(&n) && !is_bluetooth(&n));
    let named = |p: &&midir::MidiInputPort| usb(p) && input.port_name(p).is_ok_and(|n| !n.to_lowercase().contains("usb composite"));
    let in_port = ports
        .iter()
        .find(named)
        .or_else(|| ports.iter().find(usb))
        .or_else(|| ports.iter().find(|p| input.port_name(p).is_ok_and(|n| is_fm1(&n))))?;
    let name = input.port_name(in_port).ok()?;
    let out_port = output.ports().into_iter().find(|p| output.port_name(p).is_ok_and(|n| n == name))?;
    let output = Arc::new(Mutex::new(output.connect(&out_port, "FM-1 Big Screen out").ok()?));
    let mut parser = keys::Parser::default();
    let base: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let base_in = base.clone();
    let v2_answers: Arc<std::sync::atomic::AtomicU32> = Arc::default();
    let v2_in = v2_answers.clone();
    let input = input
        .connect(
            in_port,
            "FM-1 Big Screen in",
            move |_, buffer, _| {
                // Notes from the FM-1's own keys (KEY OUT on), for the spyglass, and for the Mellotron when it's on.
                mellotron::feed(buffer);
                let before = played.lock().unwrap().note_ons;
                parser.feed(buffer, &mut played.lock().unwrap());
                if played.lock().unwrap().note_ons != before {
                    ctx.request_repaint();
                }
                let mut s = shared.lock().unwrap();
                // macOS can deliver several SysEx messages in one buffer: take them one by one.
                for m in screen::messages(buffer) {
                match install::parse(m) {
                    Some((17, id)) if id.len() == 27 => {
                        s.identity = Some(String::from_utf8_lossy(&id).trim_end_matches('\0').to_string());
                        ctx.request_repaint();
                    }
                    Some((49, cap)) if cap.len() == 47 && &cap[..4] == b"FMC1" => {
                        s.build = Some(cap[14..46].iter().map(|b| format!("{b:02x}")).collect());
                        ctx.request_repaint();
                    }
                    _ => {}
                }
                if let Some(info) = screen::firmware_info(m) {
                    s.firmware = Some(info);
                    ctx.request_repaint();
                }
                if let Some(name) = screen::sound(m) {
                    if s.sound.as_deref() != Some(name.as_str()) {
                        s.sound = (!name.is_empty()).then_some(name);
                        ctx.request_repaint();
                    }
                }
                // Big Screen version 2: its answer, rectangles into the next frame, and the frame shown whole.
                match s.v2.apply(m) {
                    bigscreen2::Got::Info => {
                        v2_in.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    bigscreen2::Got::Painted => ctx.request_repaint_after(bigscreen2::SHOW_AFTER),
                    bigscreen2::Got::Frame => {
                        let s = &mut *s;
                        s.v2.show(&mut s.screen.pixels);
                        s.last = Some(Instant::now());
                        s.changed = true;
                        ctx.request_repaint();
                    }
                    bigscreen2::Got::Nothing => {}
                }
                if s.screen.apply(m) {
                    s.last = Some(Instant::now());
                    s.changed = true;
                    ctx.request_repaint();
                }
                // Charles Vestal's mirror (fm1-base firmware): its answer, and its rectangles and fills.
                match fm1base::apply(&mut s.screen, m) {
                    fm1base::Got::Hello => base_in.store(true, std::sync::atomic::Ordering::Relaxed),
                    fm1base::Got::Painted => {
                        s.last = Some(Instant::now());
                        s.changed = true;
                        ctx.request_repaint();
                    }
                    fm1base::Got::Nothing => {}
                }
                }
            },
            (),
        )
        .ok()?;
    let stop: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let lost: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let (st, lo, out) = (stop.clone(), lost.clone(), output.clone());
    std::thread::spawn(move || hellos(out, st, lo, base, v2_answers));
    Some(Link { name, _input: input, out: output, opened: Instant::now(), stop, lost })
}

impl App {
    /// The spyglass over the screen area: the first time in the room beside the FM-1's (square,
    /// centred) screen if there's enough, else top right over it. Drag to move; drag the grip or scroll to resize.
    fn spyglass(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, area: Rect) {
        let (hearing, wave) = hear(&self.sound_in, &self.glass);
        if self.glass.tick(&self.played.lock().unwrap()) || self.glass.busy() || wave.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let side = (area.width() - area.height()).max(0.0) / 2.0;
        let room = side >= 200.0;
        let d = self.glass_size.unwrap_or(if room { (side - 24.0).min(380.0) } else { area.width().min(area.height()) * 0.38 });
        let d = d.clamp(140.0, area.width().min(area.height()));
        let (fx, fy) = self.glass_at.unwrap_or(if room { ((area.width() - side / 2.0) / area.width(), 0.5) } else { (1.0 - (d / 2.0 + 16.0) / area.width(), (d / 2.0 + 16.0) / area.height()) });
        // Kept whole inside the area, also after a resize (of it or the window) that would push it past an edge.
        let inside = |c: Pos2| c.clamp(area.min + vec2(d, d) / 2.0, area.max - vec2(d, d) / 2.0);
        let c = inside(Pos2::new(area.left() + fx * area.width(), area.top() + fy * area.height()));
        let r = ui.interact(Rect::from_center_size(c, vec2(d, d)), ui.id().with("spyglass"), egui::Sense::drag());
        let mut moved = false;
        if r.dragged() {
            let c = inside(c + r.drag_delta());
            self.glass_at = Some(((c.x - area.left()) / area.width(), (c.y - area.top()) / area.height()));
            moved = true;
        }
        if r.hovered() {
            let grow = ctx.input(|i| i.smooth_scroll_delta.y);
            if grow != 0.0 {
                self.glass_size = Some((d + grow * 0.6).clamp(140.0, area.width().min(area.height())));
                moved = true;
            }
        }
        // The grip on the rim: the centre stays, so the rim follows the pointer at twice the size.
        let grip_at = glass::grip_at(c, d);
        let grip = ui.interact(Rect::from_center_size(grip_at, vec2(30.0, 30.0)), ui.id().with("spyglass grip"), egui::Sense::drag());
        if grip.dragged() {
            let m = grip.drag_delta();
            self.glass_size = Some((d + (m.x + m.y) * std::f32::consts::SQRT_2).clamp(140.0, area.width().min(area.height())));
        }
        if grip.hovered() || grip.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        if r.drag_stopped() || grip.drag_stopped() || (moved && !r.dragged()) {
            self.store();
        }
        let sound = mellotron::label().or_else(|| self.shared.lock().unwrap().sound.clone());
        self.glass.draw(ui.painter(), c, d, sound.as_deref(), hearing, wave.as_deref());
        if ui.rect_contains_pointer(Rect::from_center_size(c, vec2(d, d))) || grip.dragged() {
            glass::draw_grip(ui.painter(), grip_at, grip.hovered() || grip.dragged());
        }
    }

    /// The spyglass's own window, outside this one: no frame, see-through around the eyepiece, on top of other windows.
    /// Deferred: it runs by itself, also while this window is hidden behind others.
    fn float_window(&mut self, ctx: &egui::Context) {
        let (size, at) = self.float_open;
        let side = size + 2.0 * FLOAT_PAD;
        // Fixed while it's open: moving and resizing go straight to the window.
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Spyglass")
            .with_icon(app_icon())
            .with_inner_size([side, side])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top();
        if let Some(at) = at {
            builder = builder.with_position(at);
        }
        let (float, played, shared, sound_in) = (self.float.clone(), self.played.clone(), self.shared.clone(), self.sound_in.clone());
        ctx.show_viewport_deferred(egui::ViewportId::from_hash_of("spyglass"), builder, move |ctx, _| float_ui(ctx, &float, &played, &shared, &sound_in));
    }
}

/// Whether the FM-1's sound is coming in, and its wave for the note on `glass`.
fn hear(sound_in: &Mutex<scope::Ring>, glass: &glass::Glass) -> (bool, Option<Vec<f32>>) {
    if let Some(w) = mellotron::wave(glass.hz()) {
        return (true, Some(w));              // the Mellotron's own sound, while it plays
    }
    let ring = sound_in.lock().unwrap();
    (ring.listening(), glass.hz().and_then(|hz| ring.wave(hz)))
}

/// One frame of the floating spyglass: drag it to move the window, the grip or a scroll to resize it.
fn float_ui(ctx: &egui::Context, float: &Mutex<Float>, played: &Mutex<keys::Played>, shared: &Mutex<Shared>, sound_in: &Mutex<scope::Ring>) {
    ctx.request_repaint_after(Duration::from_millis(50));
    let mut guard = float.lock().unwrap();
    let f = &mut *guard;
    f.glass.tick(&played.lock().unwrap());
    let (close, at, scroll) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().outer_rect.map(|r| r.min), i.smooth_scroll_delta.y));
    if close {
        f.close = true;
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    if at.is_some() && at != f.at {
        f.at = at;
        f.dirty = true;
    }
    let sound = mellotron::label().or_else(|| shared.lock().unwrap().sound.clone());
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let area = ui.max_rect();
        let (c, d) = (area.center(), f.size);
        let body = ui.interact(area, ui.id().with("float"), egui::Sense::click_and_drag());
        if body.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let grip_at = glass::grip_at(c, d);
        let grip = ui.interact(Rect::from_center_size(grip_at, vec2(30.0, 30.0)), ui.id().with("float grip"), egui::Sense::drag());
        let mut size = d;
        if grip.dragged() {
            let m = grip.drag_delta(); // the window's top left stays, so the grip moves about 0.85 of the size's change
            size += (m.x + m.y) * 0.59;
        }
        if body.hovered() {
            size += scroll * 0.6;
        }
        let size = size.clamp(FLOAT_SIZES.0, FLOAT_SIZES.1);
        if size != d {
            f.size = size;
            f.dirty = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::splat(size + 2.0 * FLOAT_PAD)));
        }
        if grip.hovered() || grip.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        let (hearing, wave) = hear(sound_in, &f.glass);
        f.glass.draw(ui.painter(), c, d, sound.as_deref(), hearing, wave.as_deref());
        if ui.rect_contains_pointer(area) || grip.dragged() {
            glass::draw_grip(ui.painter(), grip_at, grip.hovered() || grip.dragged());
        }
    });
}

/// The biggest square that fits in `area`, in its middle.
fn fit(area: Rect) -> Rect {
    let side = area.width().min(area.height());
    Rect::from_center_size(area.center(), vec2(side, side))
}

impl eframe::App for App {
    /// See-through, for the floating spyglass: this window's panels cover all of it.
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.store();
        if FM1_DOWN.swap(false, std::sync::atomic::Ordering::Relaxed) {
            if let Some(link) = &self.link {
                send_fm1_volume(&mut link.out.lock().unwrap(), true); // closing: the FM-1's own sound back up
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
            self.main_at = Some(r.min);
        }
        self.sounds_window(ctx);
        self.ask_down_window(ctx);
        self.keep_linked(ctx);
        self.take_screen(ctx);
        let (closed, dirty) = {
            let mut f = self.float.lock().unwrap();
            let got = (std::mem::take(&mut f.close), std::mem::take(&mut f.dirty));
            if got.0 || got.1 {
                self.float_open = (f.size, f.at);
            }
            got
        };
        if closed {
            self.glass_float = false;
        }
        if closed || dirty {
            self.store();
        }
        if self.glass_on && self.glass_float && self.install.is_none() {
            self.float_window(ctx);
        }
        let (moved, full_key, escape, mirror_key, smooth_key) = ctx.input(|i| {
            (
                i.pointer.delta().length() > 0.0 || i.pointer.any_pressed(),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::M),
                i.key_pressed(egui::Key::S),
            )
        });
        if moved {
            self.last_move = Instant::now();
        }
        if full_key || (escape && self.full_screen) {
            self.full_screen = !self.full_screen && !escape;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full_screen));
        }
        if mirror_key {
            self.mirror_on = !self.mirror_on;
            self.store();
        }
        if smooth_key {
            self.toggle_smooth();
        }
        let need = self.need();
        if self.install.as_ref().is_some_and(|p| p.lock().unwrap().done) && need.is_none() {
            self.install = None; // the screen is back
        }
        // No FM-1 with the spyglass in this window: no panel over it, the spyglass goes on with the keyboard.
        let no_fm1 = self.link.is_none() && self.install.is_none();
        let spyglass_alone = no_fm1 && self.glass_on && !self.glass_float;
        let showing_help = (need.is_some() || self.install.is_some()) && !spyglass_alone;
        if showing_help || self.last_move.elapsed() < BAR_FOR {
            egui::TopBottomPanel::bottom("buttons")
                .frame(egui::Frame::new().fill(Color32::from_gray(28)).inner_margin(egui::Margin::same(14)))
                .show(ctx, |ui| self.buttons(ui, ctx));
            ctx.request_repaint_after(BAR_FOR); // to hide them again
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
            let area = ui.max_rect();
            let p = ui.painter();
            if showing_help {
                let need = need.unwrap_or(Need::Connecting);
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(area.shrink2(vec2(40.0, 10.0))), |ui| self.need_panel(ui, need));
            } else if spyglass_alone {
                p.text(area.center_bottom() - vec2(0.0, 24.0), Align2::CENTER_CENTER, "No FM-1 yet: its screen shows here when it's plugged in",
                    FontId::proportional(18.0), Color32::GRAY);
                self.spyglass(ui, ctx, area);
            } else if let Some(tex) = &self.texture {
                let in_here = self.glass_on && !self.glass_float;
                if self.mirror_on {
                    p.image(tex.id(), fit(area), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                } else if !in_here {
                    p.text(area.center() - vec2(0.0, 50.0), Align2::CENTER_CENTER, "The mirror is hidden", FontId::proportional(24.0), Color32::GRAY);
                }
                if in_here {
                    self.spyglass(ui, ctx, area);
                }
                // Always a way back that doesn't need the bar, which hides itself.
                if !self.mirror_on {
                    let at = if in_here { area.center_bottom() - vec2(0.0, 44.0) } else { area.center() + vec2(0.0, 10.0) };
                    if ui.put(Rect::from_center_size(at, vec2(220.0, 56.0)), egui::Button::new("Show mirror")).clicked() {
                        self.mirror_on = true;
                        self.store();
                    }
                }
            } else {
                p.text(area.center(), Align2::CENTER_CENTER, "…", FontId::proportional(40.0), Color32::WHITE);
            }
        });
    }
}

/// The app's own icon (FM-1 screen + spyglass). Without it eframe swaps the Dock icon for egui's "e" at launch.
fn app_icon() -> std::sync::Arc<egui::IconData> {
    std::sync::Arc::new(
        eframe::icon_data::from_png_bytes(include_bytes!("../icon/icon-1024.png")).unwrap_or_default(),
    )
}

fn main() -> eframe::Result {
    // where the main window was last time (so it opens on the same screen)
    let saved: Value = settings_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let at = saved["window_x"].as_f64().zip(saved["window_y"].as_f64()).map(|(x, y)| [x as f32, y as f32]);
    let mut viewport = egui::ViewportBuilder::default();
    if let Some(at) = at {
        viewport = viewport.with_position(at);
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title(APP_NAME)
            .with_icon(app_icon())
            .with_inner_size([900.0, 960.0])
            .with_min_inner_size([300.0, 340.0])
            .with_transparent(true), // (lets the floating spyglass's window be see-through)
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| {
        #[cfg(target_os = "macos")]
        {
            reopen::install(); // a Dock click brings back the minimized window
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = cc.window_handle() {
                if let RawWindowHandle::AppKit(a) = h.as_raw() {
                    reopen::solid_main_window(a.ns_view.as_ptr()); // a clear, solid title bar
                }
            }
        }
        Ok(Box::new(App::new(cc)))
    }))
}

#[cfg(test)]
mod bright_tests {
    use super::*;

    #[test]
    fn bright_lifts_thin_text_and_leaves_filled_areas() {
        let side = 40;
        let dim_purple: u16 = (6 << 11) | (2 << 5) | 10; // a dark purple
        let mut px = vec![0u16; side * side];
        for y in 5..15 {
            px[y * side + 5] = dim_purple; // a one-pixel stroke on black
        }
        for y in 20..38 {
            for x in 20..38 {
                px[y * side + x] = dim_purple; // a filled block
            }
        }
        let out = Look::Bright.paint_all(&px, side);
        let colour = Look::Color.paint(dim_purple);
        assert_ne!(out[10 * side + 5], colour, "the stroke is lifted");
        assert_eq!(out[29 * side + 29], colour, "the block's inside keeps its colour");
        assert_eq!(out[20 * side + 20], colour, "and so does its corner");
        assert_eq!(out[0], Look::Color.paint(0), "black stays black");
    }
}
