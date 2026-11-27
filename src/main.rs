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
mod debug;
#[cfg(feature = "debugger")]
mod debugui;
mod fm1base;
mod glass;
mod glyphs;
mod install;
mod keys;
mod knobs;
mod mellotron;
mod rich;
mod scope;
mod shuttle;
mod screen;
mod smooth;
mod sweeten;
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

#[cfg(not(feature = "debugger"))]
const APP_NAME: &str = "FM-1 Big Screen";
#[cfg(feature = "debugger")]
const APP_NAME: &str = "FM-1 Debugger";
/// Firmwares whose authors built Big Screen in themselves (no Key Lights to send, nothing newer of ours to get): never
/// "the older Big Screen". SHRUB (Steve Duda, Xfer Records).
const OWN_BIG_SCREEN: [&str; 1] = ["FM-1_901"];
/// Settings stay in the folder named for the app's old name, FM-1 Mirror, so nothing is lost.
pub(crate) const SETTINGS_DIR: &str = "FM-1 Mirror";
/// How long the buttons stay after the mouse last moved.
const BAR_FOR: Duration = Duration::from_secs(4);
/// How often to say hello (the FM-1 stops sending three seconds after the last one).
const HELLO_EVERY: Duration = Duration::from_millis(900);

/// Wanted: the FM-1's own sound turned down while the Mellotron plays (the hello thread sends it, and sends it back up
/// when this goes false or the link is dropped).
pub(crate) static FM1_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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
    /// Key Lights: the FM-1's lit keys and buttons, when its firmware sends them (knobs.rs).
    lights: Option<knobs::Lights>,
    /// When this link's screen first came: a screen with no Key Lights a few seconds later is the older, screen-only
    /// Big Screen (stale: its new version has Big Knobs and Key Lights).
    screen_since: Option<Instant>,
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
    /// Its fm1-base mirror, if it speaks that one (this FM-1's own: several FM-1s can be linked at once).
    fb: Arc<fm1base::State>,
}

impl Link {
    /// Sends a Big Knobs / Key Lights command: through fm1-base's remote when this FM-1 speaks it, as it is otherwise.
    fn send_remote(&self, plain: &[u8], remote: impl FnOnce(&mut fm1base::Remote)) {
        if self.fb.is_active() {
            let mut panel = self.fb.panel.lock().unwrap();
            remote(&mut panel);
            if let Some(f) = panel.next() {
                if debug::check(&self.name, debug::Dir::Out, &f, "app window", false) {
                    let _ = self.out.lock().unwrap().send(&f);
                }
            }
        } else if debug::check(&self.name, debug::Dir::Out, plain, "app window", false) {
            let _ = self.out.lock().unwrap().send(plain);
        }
    }
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
    port: String,
    output: Arc<Mutex<MidiOutputConnection>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    lost: Arc<std::sync::atomic::AtomicBool>,
    base: Arc<std::sync::atomic::AtomicBool>,
    v2_answers: Arc<std::sync::atomic::AtomicU32>,
    fb: Arc<fm1base::State>,
) {
    use std::sync::atomic::Ordering;
    // every message this thread sends goes past the debugger first (a breakpoint stops this thread right here)
    let send = |m: &[u8]| -> Result<(), midir::SendError> {
        if debug::check(&port, debug::Dir::Out, m, "hello thread", true) { output.lock().unwrap().send(m) } else { Ok(()) }
    };
    const BASE_ASK_AT: [u64; 3] = [1000, 5000, 6000]; // ms after opening: clear of the questions at 2, 3 and 4 s
    let opened = Instant::now();
    let (mut base_asked, mut base_at, mut missed_at) = (0usize, Instant::now(), Instant::now());
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
            if send_fm1_volume(&mut output.lock().unwrap(), !down) {
                down_sent = down;
            }
        }
        if hello_at.is_none_or(|t| t.elapsed() >= asker.every(HELLO_EVERY)) {
            hello_at = Some(Instant::now());
            let hello = asker.next(v2_answers.load(Ordering::Relaxed));
            if send(&hello).is_err() {
                lost.store(true, Ordering::Relaxed);
                return;
            }
        }
        if !asked_identity && opened.elapsed() > Duration::from_secs(2) {
            asked_identity = true;
            let _ = send(&install::frame(17, &[]));
        }
        if !asked_build && opened.elapsed() > Duration::from_secs(3) {
            asked_build = true;
            let _ = send(&install::frame(49, &[]));
        }
        if !asked_info && opened.elapsed() > Duration::from_secs(4) {
            asked_info = true;
            let _ = send(&ASK_INFO);
        }
        if base.load(Ordering::Relaxed) {
            // Big Knobs' turns still waiting for the last answer (fm1base.rs Remote)
            let next = fb.panel.lock().unwrap().next();
            if let Some(f) = next {
                let _ = send(&f);
            }
            if fb.missed.load(Ordering::Relaxed) && missed_at.elapsed() >= Duration::from_millis(700) {
                // a frame went missing on the way: the whole screen again (what the viewer has is skipped)
                fb.missed.store(false, Ordering::Relaxed);
                missed_at = Instant::now();
                base_at = Instant::now();
                let _ = send(&fm1base::request(2));
            } else if base_at.elapsed() >= Duration::from_secs(2) {
                base_at = Instant::now();
                let _ = send(&fm1base::request(1));
            }
        } else if base_asked < BASE_ASK_AT.len() && opened.elapsed() > Duration::from_millis(BASE_ASK_AT[base_asked]) {
            base_asked += 1;
            base_at = Instant::now();
            let _ = send(&fm1base::request(2)); // on, and the whole screen
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

/// Another FM-1 plugged in at the same time, linked and listening, while the bar works with the one selected (which
/// lives in App's own fields; a tab swaps them).
struct Unit {
    port: String,
    shared: Arc<Mutex<Shared>>,
    link: Option<Link>,
    texture: Option<TextureHandle>,
    firmware_checked: bool,
}

struct App {
    shared: Arc<Mutex<Shared>>,
    link: Option<Link>,
    /// The selected FM-1's port, and the others plugged in (each with its own link and screen).
    port: String,
    others: Vec<Unit>,
    /// All the FM-1s' screens at once, side by side (else the selected one, big).
    side_by_side: bool,
    /// Locked: Big Knobs and the Key Lights strip work every FM-1 at once (unlocked: the one picked).
    locked: bool,
    /// An install asked for while several FM-1s were plugged in: said why not, until then.
    multi_install_note: Option<Instant>,
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
    /// The FM-1 light while the Mellotron is on: lit = you hear the FM-1 too (both sounds), dark = only the Mellotron.
    /// Remembered between sessions, like the Mellotron's (its sound).
    fm1_with_mellotron: bool,
    /// The firmware the FM-1 ran last time ("FM-1_480103|ERIK37 ..."): a different one (a flash) turns the app's sound
    /// off, so you hear the new firmware itself first; remembered. Checked once per link, when it has said what it is.
    last_firmware: String,
    firmware_checked: bool,
    /// When a new firmware turned the sound off: the Sound menu says so for a while.
    new_firmware_at: Option<Instant>,
    /// Sweeten the Mellotron (warmth and air, saturation, stereo chorus, a small room): remembered.
    sweeten: bool,
    /// Big Knobs (the FM-1's eight knobs, big) and Key Lights (its lit keys and buttons, big): remembered.
    knobs_on: bool,
    lights_on: bool,
    knobs: knobs::Knobs,
    /// The MIDI debugger's window (debugui.rs).
    #[cfg(feature = "debugger")]
    dbg: debugui::DebugUi,
    /// A Contour ShuttleXpress/ShuttlePRO as a remote (shuttle.rs): the knob it turns (0..7, knobs::NAMES), the
    /// ring's position and when it turns next, its buttons' presses, what it shows big, and if it speaks.
    shuttle: shuttle::Shuttle,
    sh_knob: usize,
    sh_ring: i8,
    sh_next: Instant,
    sh_presses: shuttle::Presses,
    sh_toast: Option<(String, Instant)>,
    sh_turned: (usize, i32, Instant),
    sh_speak: bool,
    sh_was: bool,
    /// The buttons and keys held down on the Lights strip (bit id), pressing the real ones.
    panel_held: u64,
    /// The buttons' height when last shown: kept while they're hidden, so nothing moves.
    bar_h: f32,
    /// Where the main window is (its top left, in points), kept so it opens there next time (the same screen).
    main_at: Option<Pos2>,
    /// "Mellotron sounds": the list online (None while it's being fetched), a message, a sound just downloaded.
    sounds_open: bool,
    sounds_list: Arc<Mutex<Option<Result<Vec<mellotron::Pack>, String>>>>,
    sounds_note: Arc<Mutex<Option<String>>>,
    sounds_ready: Arc<Mutex<Option<String>>>,
    /// Sounds found in the Downloads folder and put in place by themselves (mellotron::import_downloads).
    sounds_found: Arc<Mutex<Option<Vec<String>>>>,
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

/// A screen without Key Lights this long after it came: the older, screen-only Big Screen.
const STALE_AFTER: Duration = Duration::from_secs(5);
/// aikellogg.com's Make a Big Screen: any FM-1 firmware from The Firmware Hub, with Big Screen, Big Knobs and Key Lights.
const MAKE_BIG_SCREEN: &str = "https://aikellogg.com/#/make-big-screen";
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
        // Rounded buttons, every one (the bar's, the panels', the dialogs')
        let round = egui::CornerRadius::same(12);
        for w in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active,
                  &mut style.visuals.widgets.open, &mut style.visuals.widgets.noninteractive] {
            w.corner_radius = round;
        }
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
            port: saved["fm1_port"].as_str().unwrap_or("").to_string(),   // the one picked last time, if it's there
            others: Vec::new(),
            side_by_side: saved["side_by_side"].as_bool().unwrap_or(false),
            locked: saved["locked"].as_bool().unwrap_or(false),
            multi_install_note: None,
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
            knobs_on: {
                *rich::PICK.lock().unwrap() = saved["rich_pick"].as_u64().filter(|&p| p < 128).map(|p| p as u8);
                saved["knobs"].as_bool().unwrap_or(false)
            },
            lights_on: saved["lights"].as_bool().unwrap_or(false),
            knobs: knobs::Knobs::default(),
            #[cfg(feature = "debugger")]
            dbg: debugui::DebugUi::default(),
            shuttle: { let c = ctx.clone(); shuttle::Shuttle::start(move || c.request_repaint()) },
            sh_knob: 2,
            sh_ring: 0,
            sh_next: Instant::now(),
            sh_presses: shuttle::Presses::default(),
            sh_toast: None,
            sh_turned: (2, 0, Instant::now()),
            sh_speak: saved["voice"].as_bool().unwrap_or(false),   // the Voice button: off until turned on
            sh_was: false,
            panel_held: 0,
            glass_at: saved["spyglass_x"].as_f64().zip(saved["spyglass_y"].as_f64()).map(|(x, y)| (x as f32, y as f32)),
            glass_size: saved["spyglass_size"].as_f64().map(|d| d as f32),
            mirror_on: saved["mirror"].as_bool().unwrap_or(true),
            smooth_on: true, // always on at start (S or the button turns it off for this session)
            mellotron: None,
            mellotron_sound: {
                let have = mellotron::installed();
                saved["mellotron"].as_str().unwrap_or("").split(',').filter(|s| have.iter().any(|i| i == s)).collect::<Vec<_>>().join(",")
            },
            fm1_with_mellotron: saved["fm1_with_mellotron"].as_bool().unwrap_or(true),
            last_firmware: saved["last_firmware"].as_str().unwrap_or("").to_string(),
            firmware_checked: false,
            new_firmware_at: None,
            sweeten: {
                let on = saved["sweeten"].as_bool().unwrap_or(false);
                sweeten::ON.store(on, std::sync::atomic::Ordering::Relaxed);
                on
            },
            main_at: None,
            bar_h: 0.0,
            sounds_open: false,
            sounds_list: Arc::default(),
            sounds_note: Arc::default(),
            sounds_ready: Arc::default(),
            sounds_found: {
                let found: Arc<Mutex<Option<Vec<String>>>> = Arc::default();
                let f = found.clone();
                std::thread::spawn(move || {
                    let added = mellotron::import_downloads();
                    *f.lock().unwrap() = Some(added);
                });
                found
            },
            glass_float: saved["spyglass_float"].as_bool().unwrap_or(false),
            float: Arc::new(Mutex::new(Float { glass: glass::Glass::default(), size: float_size, at: float_at, close: false, dirty: false })),
            float_open: (float_size, float_at),
        };
        app.start_mellotron();   // the sound it played last time
        app.apply_fm1_light();
        // FM1_DEBUG=1 opens the debugger at start; FM1_DEBUG=hello also stops when the firmware says hello
        #[cfg(feature = "debugger")]
        debugui::set_dark(saved["debugger_dark"].as_bool().unwrap_or(false));
        #[cfg(feature = "debugger")]
        if let Ok(v) = std::env::var("FM1_DEBUG") {
            app.dbg.open = true;
            debug::set_on(true, ctx);
            if v.contains("hello") {
                debug::with_breakpoints(|b| b.push(debug::Breakpoint::new(debug::What::SaysHello)));
            }
        }
        app
    }

    fn store(&self) {
        let Some(path) = settings_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let mut s = json!({ "look": self.look.key(), "keys": self.keys_choice, "keys_channel": self.keys_channel, "spyglass": self.glass_on, "mirror": self.mirror_on, "smooth": self.smooth_on, "spyglass_float": self.glass_float, "mellotron": self.mellotron_sound, "fm1_with_mellotron": self.fm1_with_mellotron, "sweeten": self.sweeten, "knobs": self.knobs_on, "lights": self.lights_on, "rich_pick": *rich::PICK.lock().unwrap(), "last_firmware": self.last_firmware, "side_by_side": self.side_by_side, "fm1_port": self.port, "locked": self.locked, "voice": self.sh_speak });
        #[cfg(feature = "debugger")]
        {
            s["debugger_dark"] = json!(debugui::is_dark());
        }
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
            let ports = fm1_ports(self.scanner.as_ref());
            let lost = |l: &Link| !ports.contains(&l.name) || l.lost.load(std::sync::atomic::Ordering::Relaxed);
            // (a Bluetooth link while the FM-1 is also on USB isn't in `ports`: it moves over to USB, much faster)
            if self.link.as_ref().is_some_and(lost) {
                self.link = None;
            }
            self.others.retain(|u| u.link.as_ref().is_some_and(|l| !lost(l)));
            if self.link.is_none() && !ports.contains(&self.port) && !self.others.is_empty() {
                self.select(0);                       // the selected FM-1 went: the next one takes its place
            }
            if self.link.is_none() {
                // macOS may still call it by an old firmware's name ("ChoralRoot FM-1" for LOOM-8): the FM-1's own
                // USB name again, for every app (usbname.rs)
                if self.others.is_empty() {
                    for (old, new) in usbname::fix_stale_names(is_fm1) {
                        eprintln!("renamed the USB MIDI device \"{old}\" to \"{new}\", the name the FM-1 reports");
                    }
                }
                let taken: Vec<String> = self.others.iter().map(|u| u.port.clone()).collect();
                let free = |p: &&String| !taken.contains(p);
                let want = ports.iter().find(|p| **p == self.port).or_else(|| ports.iter().find(free)).cloned();
                if let Some(port) = want {
                    {
                        let mut s = self.shared.lock().unwrap();
                        s.identity = None;
                        s.build = None;
                        s.firmware = None;
                        s.sound = None;
                    }
                    self.firmware_checked = false;
                    self.link = open(&port, self.shared.clone(), self.played.clone(), ctx.clone());
                    self.port = port;
                }
            }
            // every other FM-1 plugged in: its own link, listening
            for port in &ports {
                if *port == self.port || self.others.iter().any(|u| u.port == *port) {
                    continue;
                }
                let shared: Arc<Mutex<Shared>> = Arc::default();
                if let Some(link) = open(port, shared.clone(), self.played.clone(), ctx.clone()) {
                    eprintln!("another FM-1: {port}");
                    self.others.push(Unit { port: port.clone(), shared, link: Some(link), texture: None, firmware_checked: false });
                }
            }
            *self.keys_out.lock().unwrap() = self.link.as_ref().map(|l| l.out.clone());
            self.keep_keys(ctx);
            self.check_new_firmware();
            if self.glass_on && self.listener.as_ref().is_none_or(|l| l.dead()) {
                self.listener = scope::open(self.sound_in.clone());
            } else if !self.glass_on {
                self.listener = None;
            }
        }
        ctx.request_repaint_after(HELLO_EVERY);
    }

    /// Makes others[i] the selected FM-1 (the bar, Big Knobs, Key Lights and the keyboard work with it).
    fn select(&mut self, i: usize) {
        if i >= self.others.len() {
            return;
        }
        if let Some(link) = &self.link {
            keys::silence(&link.out);                 // no note left hanging on the one let go
        }
        let u = &mut self.others[i];
        std::mem::swap(&mut self.shared, &mut u.shared);
        std::mem::swap(&mut self.link, &mut u.link);
        std::mem::swap(&mut self.texture, &mut u.texture);
        std::mem::swap(&mut self.firmware_checked, &mut u.firmware_checked);
        std::mem::swap(&mut self.port, &mut u.port);
        if u.link.is_none() {
            self.others.remove(i);                    // (the one that was selected had gone)
        }
        *self.keys_out.lock().unwrap() = self.link.as_ref().map(|l| l.out.clone());
        self.panel_held = Default::default();
        self.shared.lock().unwrap().changed = true;
        self.store();
    }

    /// Where Big Knobs and the Key Lights strip send: the FM-1 picked, or every one while locked.
    fn targets(&self) -> Vec<&Link> {
        let mut v: Vec<&Link> = self.link.iter().collect();
        if self.locked {
            v.extend(self.others.iter().filter_map(|u| u.link.as_ref()));
        }
        v
    }

    /// A Big Knobs message to the FM-1 picked, or every one while locked (as the Big Knobs do).
    fn send_knob(&self, m: &[u8]) {
        for l in self.targets() {
            l.send_remote(m, |panel| match m.get(4) {
                Some(4) => panel.turn(m[5], if m[6] >= 64 { m[6] as i32 - 128 } else { m[6] as i32 }),
                Some(6) => panel.master(m[5] as u16 | (m[6] as u16) << 7),
                _ => {}
            });
        }
    }

    // ---- the Contour Shuttle (shuttle.rs): turn knobs with a dial; everything shown big and said out loud ----

    /// Shows `show` big for a moment, and says `speak` (if speech is on).
    /// The Voice button (and holding Shuttle button 2): the spoken words on or off. Kept as a setting.
    fn toggle_voice(&mut self) {
        self.sh_speak = !self.sh_speak;
        self.store();
        self.sh_toast = Some((if self.sh_speak { "Voice on" } else { "Voice off" }.into(), Instant::now()));
        if self.sh_speak {
            shuttle::say("Voice on");
        }
    }

    fn sh_say(&mut self, show: String, speak: &str) {
        self.sh_toast = Some((show, Instant::now()));
        if self.sh_speak {
            shuttle::say(speak);
        }
    }

    /// How a name sounds: "KNOB 1" -> "Knob 1", "FM-1" -> "F M 1".
    fn spoken(text: &str) -> String {
        let t = text.replace("FM-1", "F M 1").replace(" · ", ", ");
        let mut out = String::new();
        for w in t.split(' ') {
            let lower = w.chars().all(|c| !c.is_lowercase()) && w.chars().filter(|c| c.is_alphabetic()).count() > 2 && w != "SLOOP";
            if lower {
                let l = w.to_lowercase();
                let mut c = l.chars();
                out += &c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
            } else {
                out += w;
            }
            out.push(' ');
        }
        out.trim().to_string()
    }

    /// Turns the Shuttle's knob `clicks` (the dial or the ring), on the FM-1 picked or all while locked.
    fn sh_turn(&mut self, clicks: i32) {
        if clicks == 0 {
            return;
        }
        let i = self.sh_knob;
        let m = self.knobs.nudge(i, clicks);
        self.send_knob(&m);
        let (k, n, at) = self.sh_turned;
        let n = if k == i && at.elapsed() < Duration::from_millis(1500) { n + clicks } else { clicks };
        self.sh_turned = (i, n, Instant::now());
        let shown = if i == 7 { format!("MASTER {}%", self.knobs.master_percent()) } else { format!("{}  {n:+}", knobs::NAMES[i]) };
        let all = if self.locked && !self.others.is_empty() { "  ·  ALL" } else { "" };
        self.sh_toast = Some((format!("{shown}{all}"), Instant::now())); // (not said: it would chatter while turning)
    }

    /// Where I am: "FM-1 2 of 5, SLOOP 2.8.5. Knob 1. Locked."
    fn sh_status(&self) -> (String, String) {
        let units = self.units_in_order();
        let at = units.iter().position(|u| u.1).map(|p| p + 1).unwrap_or(1);
        let name = Self::unit_name(at, &self.shared, &self.port);
        let of = if units.len() > 1 { format!(" of {}", units.len()) } else { String::new() };
        let knob = knobs::NAMES[self.sh_knob];
        let lock = if units.len() > 1 { if self.locked { ". Locked" } else { ". Unlocked" } } else { "" };
        let shown = format!("{name}{of}  ·  {knob}");
        (shown, Self::spoken(&format!("{name}{of}. {knob}{lock}.")))
    }

    fn sh_button(&mut self, b: u8, down: bool) {
        if b == 3 {
            // SEL, pressed for as long as the button is held (as a finger on the FM-1's own)
            let m = knobs::press(3, down);
            for l in self.targets() {
                l.send_remote(&m, |p| p.press(3, down));
            }
            if down {
                self.sh_toast = Some(("SEL".into(), Instant::now()));
            }
            return;
        }
        if down {
            self.sh_presses.down(b);
        } else if self.sh_presses.up(b) {
            self.sh_tap(b);
        }
    }

    fn sh_tap(&mut self, b: u8) {
        match b {
            1 | 2 => {
                self.sh_knob = (self.sh_knob + if b == 1 { 7 } else { 1 }) % 8;
                self.knobs.shuttle = Some(self.sh_knob);
                let name = knobs::NAMES[self.sh_knob];
                let extra = if self.sh_knob == 7 { format!(" {}%", self.knobs.master_percent()) } else { String::new() };
                self.sh_say(format!("{name}{extra}"), &Self::spoken(&format!("{name}{extra}")));
            }
            4 => {
                let units = self.units_in_order();
                if units.len() < 2 {
                    self.sh_say("Only one FM-1".into(), "Only one F M 1");
                    return;
                }
                let at = units.iter().position(|u| u.1).unwrap_or(0);
                if let Some(i) = units[(at + 1) % units.len()].2 {
                    self.select(i);
                }
                let (shown, said) = self.sh_status();
                self.sh_say(shown, &said);
            }
            5 => {
                if self.others.is_empty() {
                    self.sh_say("Only one FM-1".into(), "Only one F M 1: the lock is for several");
                    return;
                }
                self.locked = !self.locked;
                self.store();
                let (shown, said) = if self.locked { ("LOCKED: knobs move every FM-1", "Locked. Knobs move every F M 1.") } else { ("Unlocked: only the picked FM-1", "Unlocked. Only this F M 1.") };
                self.sh_say(shown.into(), said);
            }
            _ => {}
        }
    }

    fn sh_hold(&mut self, b: u8) {
        match b {
            1 => {
                let (shown, said) = self.sh_status();
                self.sh_say(shown, &said);
            }
            2 => self.toggle_voice(),
            4 => {
                if self.others.is_empty() {
                    self.sh_say("Only one FM-1".into(), "Only one F M 1");
                    return;
                }
                self.side_by_side = !self.side_by_side;
                self.store();
                let t = if self.side_by_side { "Side by side" } else { "One at a time" };
                self.sh_say(t.into(), t);
            }
            _ => {}
        }
    }

    /// Each frame: the Shuttle's events, the ring's repeats, the holds, and what it shows.
    fn shuttle_frame(&mut self, ctx: &egui::Context) {
        let conn = self.shuttle.connected();
        if conn != self.sh_was {
            self.sh_was = conn;
            self.knobs.shuttle = conn.then_some(self.sh_knob);
            if conn {
                let name = knobs::NAMES[self.sh_knob];
                self.sh_say(format!("Shuttle: {name}"), &format!("Shuttle connected. {}.", Self::spoken(name)));
            }
        }
        for e in self.shuttle.events() {
            match e {
                shuttle::Ev::Dial(d) => self.sh_turn(d),
                shuttle::Ev::Ring(r) => {
                    self.sh_ring = r;
                    self.sh_next = Instant::now();
                }
                shuttle::Ev::Button(b, down) => self.sh_button(b, down),
            }
        }
        for b in self.sh_presses.holds() {
            self.sh_hold(b);
        }
        if self.sh_ring != 0 && Instant::now() >= self.sh_next {
            let (every, clicks) = shuttle::ring_speed(self.sh_ring);
            self.sh_turn(clicks);
            self.sh_next = Instant::now() + every;
        }
        if self.sh_ring != 0 || self.sh_presses.any_down() {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
        if let Some((text, at)) = self.sh_toast.clone() {
            if at.elapsed() < Duration::from_millis(1800) {
                egui::Area::new(egui::Id::new("shuttle_toast")).order(egui::Order::Foreground).interactable(false)
                    .anchor(Align2::CENTER_TOP, vec2(0.0, 40.0))
                    .show(ctx, |ui| {
                        egui::Frame::new().fill(Color32::from_black_alpha(225)).corner_radius(16.0).inner_margin(egui::Margin::symmetric(26, 14))
                            .stroke(egui::Stroke::new(3.0_f32, Color32::from_rgb(255, 214, 10)))
                            .show(ui, |ui| ui.add(egui::Label::new(egui::RichText::new(text).size(52.0).strong().color(Color32::from_rgb(255, 214, 10))).wrap_mode(egui::TextWrapMode::Extend)));
                    });
                ctx.request_repaint_after(Duration::from_millis(100));
            } else {
                self.sh_toast = None;
            }
        }
    }

    /// Every FM-1's screen painted again (a new look, Smooth on or off).
    fn repaint_all(&self) {
        self.shared.lock().unwrap().changed = true;
        for u in &self.others {
            u.shared.lock().unwrap().changed = true;
        }
    }

    /// All the FM-1s in port order (stable tabs): (port, selected, its other index).
    fn units_in_order(&self) -> Vec<(String, bool, Option<usize>)> {
        let mut v: Vec<(String, bool, Option<usize>)> = self.others.iter().enumerate().map(|(i, u)| (u.port.clone(), false, Some(i))).collect();
        if self.link.is_some() || !self.port.is_empty() {
            v.push((self.port.clone(), true, None));
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// A tab's name: "FM-1 2 · SHRUB" (its number, then what it runs when it said so).
    fn unit_name(n: usize, shared: &Mutex<Shared>, port: &str) -> String {
        let s = shared.lock().unwrap();
        let known = screen::community(s.firmware.as_deref(), s.identity.as_deref(), port).map(|c| c.name.to_string());
        let what = known.or_else(|| s.firmware.clone().map(|f| f.split_whitespace().take(2).collect::<Vec<_>>().join(" ")));
        match what {
            Some(w) if !w.is_empty() => format!("FM-1 {n} · {w}"),
            _ => format!("FM-1 {n}"),
        }
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
        let mut tex = self.texture.take();
        Self::render(ctx, &self.shared, &mut tex, self.look, self.smooth_on);
        self.texture = tex;
        // the other FM-1s' screens too, while they're shown (side by side)
        if self.side_by_side {
            for u in &mut self.others {
                Self::render(ctx, &u.shared, &mut u.texture, self.look, self.smooth_on);
            }
        }
    }

    /// One FM-1's screen into its texture, when it changed.
    fn render(ctx: &egui::Context, shared: &Mutex<Shared>, texture: &mut Option<TextureHandle>, look: Look, smooth_on: bool) {
        let pixels = {
            let mut s = shared.lock().unwrap();
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
        let painted = look.paint_all(&pixels, SIZE);
        // Nearest: the FM-1's 5x7 letters stay crisp blocks however big they get. Smooth: a bigger copy, blended.
        let (img, options) = if smooth_on {
            let side = SIZE * smooth::SCALE;
            (ColorImage { size: [side, side], pixels: smooth::smooth(&painted, SIZE) }, TextureOptions::LINEAR)
        } else {
            (ColorImage { size: [SIZE, SIZE], pixels: painted }, TextureOptions::NEAREST)
        };
        match texture {
            Some(t) => t.set(img, options),
            None => *texture = Some(ctx.load_texture("fm1", img, options)),
        }
    }

    /// The Mellotron button: steps through the sounds this computer has (built in or downloaded), then off
    /// (remembered). With none yet (the lite version) it opens "Mellotron sounds"; a right-click always does.
    #[allow(dead_code)] // (the old Mellotron button stepped through the sounds: the Sound menu picks one)
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

    /// The sounds picked to play together (mellotron_sound holds them joined with commas).
    fn picked(&self) -> Vec<String> {
        self.mellotron_sound.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
    }

    /// Adds a sound to what plays, or takes it away.
    fn toggle_sound(&mut self, id: &str, on: bool) {
        let mut p = self.picked();
        p.retain(|s| s != id);
        if on {
            p.push(id.to_string());
        }
        self.set_mellotron(p.join(","));
    }

    fn set_mellotron(&mut self, sound: String) {
        self.mellotron = None;                 // (dropping it stops the sound)
        self.mellotron_sound = sound;
        self.start_mellotron();
        self.store();
        self.apply_fm1_light();
    }

    /// The FM-1's own sound as its light says: down only while the Mellotron is on and the FM-1 light is dark.
    /// (Back up is sent even if another app turned it down.)
    fn apply_fm1_light(&self) {
        let down = !self.mellotron_sound.is_empty() && !self.fm1_with_mellotron;
        if !FM1_DOWN.swap(down, std::sync::atomic::Ordering::Relaxed) && !down {
            FM1_UP_ONCE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// A light's width (for fit()).
    fn led_w(ui: &egui::Ui, name: &str) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(name.to_string(), font, Color32::WHITE);
        let d = galley.size().y.max(14.0);
        d + 6.0 + galley.size().x + 8.0
    }

    /// A light with its name: green and bright when lit, dark when not. Clickable.
    fn led(ui: &mut egui::Ui, lit: bool, name: &str) -> egui::Response {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let text_color = if lit { Color32::WHITE } else { Color32::from_gray(120) };
        let galley = ui.painter().layout_no_wrap(name.to_string(), font, text_color);
        let d = galley.size().y.max(14.0);
        let size = vec2(d + 6.0 + galley.size().x + 8.0, d + 8.0);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, 4.0, Color32::from_gray(45));
        }
        let c = rect.left_center() + vec2(4.0 + d / 2.0, 0.0);
        if lit {
            p.circle_filled(c, d / 2.0 + 3.0, Color32::from_rgba_unmultiplied(40, 255, 90, 60)); // the glow
            p.circle_filled(c, d / 2.0, Color32::from_rgb(40, 230, 80));
        } else {
            p.circle_filled(c, d / 2.0, Color32::from_gray(55));
            p.circle_stroke(c, d / 2.0, egui::Stroke::new(1.0_f32, Color32::from_gray(90)));
        }
        p.galley(egui::pos2(c.x + d / 2.0 + 6.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
        resp
    }

    /// After a flash: a different firmware than last time turns the app's sound off (it would play over the new
    /// firmware's own sound, and may not suit it). Decided 5 s after its screen first came, once it has said what it is.
    fn check_new_firmware(&mut self) {
        if self.firmware_checked || self.link.is_none() {
            return;
        }
        let (since, identity, firmware) = {
            let s = self.shared.lock().unwrap();
            (s.screen_since, s.identity.clone(), s.firmware.clone())
        };
        if !since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) || (identity.is_none() && firmware.is_none()) {
            return;
        }
        self.firmware_checked = true;
        let now = format!("{}|{}", identity.unwrap_or_default(), firmware.unwrap_or_default());
        if now == self.last_firmware {
            return;
        }
        if !self.last_firmware.is_empty() && !self.mellotron_sound.is_empty() {
            eprintln!("new firmware ({} -> {now}): the app's sound off", self.last_firmware);
            self.set_mellotron(String::new());
            self.new_firmware_at = Some(Instant::now());
        }
        self.last_firmware = now;
        self.store();
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
        let (list, note, ctx) = (self.sounds_list.clone(), self.sounds_note.clone(), ctx.clone());
        std::thread::spawn(move || {
            let added = mellotron::import_downloads();
            if !added.is_empty() {
                *note.lock().unwrap() = Some(format!("Found in your Downloads and added: {}. They're in the Sound menu.", added.join(", ")));
            }
            *list.lock().unwrap() = Some(mellotron::list());
            ctx.request_repaint();
        });
    }

    fn sounds_window(&mut self, ctx: &egui::Context) {
        let found = self.sounds_found.lock().unwrap().take();
        if let Some(added) = found {
            if !added.is_empty() {
                self.open_sounds(ctx);
                *self.sounds_note.lock().unwrap() = Some(format!("Found in your Downloads and added: {}. They're in the Sound menu.", added.join(", ")));
            } else {
                *self.sounds_found.lock().unwrap() = None;
            }
        }
        let ready = self.sounds_ready.lock().unwrap().take();
        if let Some(id) = ready {
            self.toggle_sound(&id, true);      // a sound just downloaded: it plays straight away (with any others on)
        }
        if !self.sounds_open {
            return;
        }
        let mut open = true;
        let list = self.sounds_list.lock().unwrap().clone();
        let note = self.sounds_note.lock().unwrap().clone();
        let mut remove: Option<String> = None;
        // always inside the app's window, its ✕ in sight: a long list scrolls inside it
        let room = ctx.screen_rect().shrink(20.0);
        egui::Window::new("Get sounds").open(&mut open).collapsible(false).resizable(false).constrain(true)
            .max_width(room.width().min(860.0)).max_height(room.height() - 40.0).vscroll(true)
            .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 20.0)).show(ctx, |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Heading);
                ui.label("Download the sounds you want, then pick them in the Sound menu: click one or several to light them and play them together. The lights in the bar show what you hear.");
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
                        let missing: Vec<mellotron::Pack> = packs.iter().filter(|p| !mellotron::bundled(&p.id) && !mellotron::downloaded(&p.id)).cloned().collect();
                        let busy = note.as_deref().is_some_and(|n| n.starts_with("Downloading"));
                        if (!missing.is_empty() || !rich::downloaded()) && !busy {
                            let mb = missing.iter().map(|p| p.bytes).sum::<u64>() as f64 / 1048576.0 + if rich::downloaded() { 0.0 } else { rich::SIZE_MB as f64 };
                            if ui.button(format!("Get all ({mb:.0} MB)")).on_hover_text("Every sound you don't have yet, one after another").clicked() {
                                let (note, ready, ctx) = (self.sounds_note.clone(), self.sounds_ready.clone(), ctx.clone());
                                std::thread::spawn(move || {
                                    let mut got = Vec::new();
                                    for p in &missing {
                                        *note.lock().unwrap() = Some(format!("Downloading {}…", p.name));
                                        ctx.request_repaint();
                                        if mellotron::download(p).is_ok() {
                                            got.push(p.name.clone());
                                        }
                                    }
                                    if !rich::downloaded() {
                                        *note.lock().unwrap() = Some("Downloading Rich (GeneralUser GS)…".into());
                                        ctx.request_repaint();
                                        if rich::download().is_ok() {
                                            got.push("Rich".into());
                                        }
                                    }
                                    *note.lock().unwrap() = Some(if got.is_empty() { "Nothing could be downloaded now. Try again later.".into() } else { format!("Ready: {}. They're in the Sound menu.", got.join(", ")) });
                                    if let Some(p) = missing.first() {
                                        *ready.lock().unwrap() = Some(p.id.clone());
                                    }
                                    ctx.request_repaint();
                                });
                            }
                        }
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
                // Rich: GeneralUser GS, the closest instrument to the FM-1's sound (rich.rs)
                ui.separator();
                ui.label(egui::RichText::new("Rich").strong());
                ui.label("A rich version of the FM-1's sound, out of this computer: the closest of 128 instruments to the sound the FM-1 plays (its flute, organ, strings, bells…), from GeneralUser GS by S. Christian Collins (free).");
                ui.horizontal(|ui| {
                    if rich::downloaded() {
                        if ui.button("Remove Rich").clicked() {
                            remove = Some(rich::ID.to_string());
                        }
                    } else if note.as_deref().is_some_and(|n| n.starts_with("Downloading")) {
                        ui.label(format!("{} MB", rich::SIZE_MB));
                    } else if ui.button(format!("Download Rich ({} MB)", rich::SIZE_MB)).clicked() {
                        let (note, ready, ctx) = (self.sounds_note.clone(), self.sounds_ready.clone(), ctx.clone());
                        *note.lock().unwrap() = Some("Downloading Rich (GeneralUser GS)…".into());
                        std::thread::spawn(move || {
                            match rich::download() {
                                Ok(()) => {
                                    *note.lock().unwrap() = Some("Rich is ready: the Mellotron button steps through it.".into());
                                    *ready.lock().unwrap() = Some(rich::ID.to_string());
                                }
                                Err(e) => *note.lock().unwrap() = Some(format!("Rich couldn't be downloaded ({e}). Try again later.")),
                            }
                            ctx.request_repaint();
                        });
                    }
                });
                if rich::downloaded() {
                    let pick = *rich::PICK.lock().unwrap();
                    let auto = format!("Auto: {}", rich::GM[rich::program_for(&rich::FM1_SOUND.lock().unwrap()) as usize]);
                    let shown = pick.map_or(auto.clone(), |p| rich::GM[p as usize].to_string());
                    let mut new_pick = pick;
                    egui::ComboBox::from_label("Instrument").selected_text(shown).height(400.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut new_pick, None, format!("{auto} (follows the FM-1's sound)"));
                        for (i, n) in rich::GM.iter().enumerate() {
                            ui.selectable_value(&mut new_pick, Some(i as u8), *n);
                        }
                    });
                    if new_pick != pick {
                        *rich::PICK.lock().unwrap() = new_pick;
                        self.store();
                    }
                }
                if let Some(n) = &note {
                    ui.add_space(8.0);
                    ui.label(n);
                }
            });
        if let Some(id) = remove {
            if self.picked().contains(&id) {
                self.toggle_sound(&id, false);
            }
            if id == rich::ID { rich::remove() } else { mellotron::remove(&id) }
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
        self.repaint_all(); // repaint it the other way
        self.store();
    }

    /// Why there's no picture, if there isn't one.
    /// The FM-1's firmware has the older Big Screen: its screen came a while ago, but no Key Lights (Charles Vestal's
    /// mirror sends its lights its own way: never stale).
    fn stale(&self) -> bool {
        if self.link.as_ref().is_none_or(|l| l.fb.is_active()) {
            return false;
        }
        let s = self.shared.lock().unwrap();
        // only a firmware that said what it is: our older builds always do; one whose author added the screen himself
        // and that doesn't answer (SHRUB) isn't "the older Big Screen" (nothing newer of it to get)
        let known = s.identity.is_some() || s.firmware.is_some();
        let own = s.identity.as_deref().is_some_and(|id| OWN_BIG_SCREEN.iter().any(|o| id.starts_with(o)));
        known && !own && s.lights.is_none() && s.screen_since.is_some_and(|t| t.elapsed() > STALE_AFTER)
    }

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
        // Any firmware without Big Screen: the big way to one (aikellogg.com's Make a Big Screen: any Firmware Hub
        // firmware, made Big Screen with Big Knobs and Key Lights)
        let make_big = |ui: &mut egui::Ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("You need to install a Big Screen firmware of your choice.").size(30.0).strong().color(Color32::from_rgb(255, 210, 31)));
            let b = egui::Button::new(egui::RichText::new("Choose one at aikellogg.com  ›").size(30.0).strong().color(Color32::BLACK))
                .fill(Color32::from_rgb(255, 210, 31)).min_size(vec2(460.0, 64.0));
            if ui.add(b).on_hover_text("aikellogg.com › Make a Big Screen: pick any FM-1 firmware from The Firmware Hub and download it with Big Screen, Big Knobs and Key Lights").clicked() {
                open_url(MAKE_BIG_SCREEN);
            }
            ui.add_space(10.0);
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
                    ui.horizontal_wrapped(|ui| {
                        heading(ui, "No FM-1 yet");
                        ui.add_space(24.0);
                        let b = egui::Button::new(egui::RichText::new("Get a Big Screen firmware at aikellogg.com  ›").size(26.0).strong().color(Color32::BLACK))
                            .fill(Color32::from_rgb(255, 210, 31)).min_size(vec2(520.0, 60.0));
                        if ui.add(b).clicked() {
                            open_url(MAKE_BIG_SCREEN);
                        }
                    });
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
                    make_big(ui);
                    body(ui, "To show its screen here it needs the FM-1 Big Screen firmware: M-VAVE's free Official Open firmware plus one small addition that sends the screen to this app. Two steps, and you can go back to the original whenever you like.");
                    step1(ui);
                }
                Need::Open => {
                    heading(ui, "This FM-1 runs M-VAVE's Open firmware");
                    make_big(ui);
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
                    make_big(ui);
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
                    make_big(ui);
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
                    make_big(ui);
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
        if !self.others.is_empty() {
            // with several FM-1s plugged in the installer could reach the wrong one: one at a time
            self.multi_install_note = Some(Instant::now());
            return;
        }
        let progress: Arc<Mutex<install::Progress>> = Arc::default();
        self.install = Some(progress.clone());
        self.link = None; // the installer has the FM-1 to itself
        self.shared.lock().unwrap().last = None;
        std::thread::spawn(move || install::install(progress));
    }

    /// A button's width for the longest of its labels, so it doesn't change size when its label does.
    fn width_for(ui: &egui::Ui, labels: &[&str]) -> f32 {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let pad = ui.spacing().button_padding.x * 2.0 + 2.0;
        labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), Color32::WHITE).size().x).fold(0.0, f32::max) + pad
    }

    /// A new row when the next control (w wide) won't fit: horizontal_wrapped wraps text and plain buttons, but a menu
    /// or a fixed-width button runs off the window's edge instead.
    fn fit(ui: &mut egui::Ui, w: f32) {
        let left = ui.clip_rect().right() - 14.0 - ui.cursor().min.x;   /* (to the window's edge, less the bar's margin) */
        if left < w && ui.cursor().min.x > ui.clip_rect().left() + 15.0 {
            ui.end_row();
        }
    }

    /// A menu's width with its arrow, for fit().
    fn combo_w(ui: &egui::Ui, w: f32) -> f32 {
        w + ui.spacing().icon_width + ui.spacing().icon_spacing + ui.spacing().button_padding.x * 2.0 + 4.0
    }

    /// A click: the button pressed now and let go TAP later, on a timer of its own (not waiting for a frame).
    fn tap(&self, id: u8) {
        self.send_press(id, true);
        for l in self.targets() {
            let (out, fb) = (l.out.clone(), l.fb.clone());
            std::thread::spawn(move || {
                std::thread::sleep(knobs::TAP);
                if fb.is_active() {
                    let mut panel = fb.panel.lock().unwrap();
                    panel.press(id, false);
                    if let Some(f) = panel.next() {
                        let _ = out.lock().unwrap().send(&f);
                    }
                } else {
                    let _ = out.lock().unwrap().send(&knobs::press(id, false));
                }
            });
        }
    }

    /// Presses (or lets go of) an FM-1 button or key, the way the Lights strip does.
    fn send_press(&self, id: u8, down: bool) {
        for l in self.targets() {
            l.send_remote(&knobs::press(id, down), |p| p.press(id, down));
        }
    }

    fn buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // the bar's text smaller than the app's: fewer rows, more room for the big screen
        for (t, size) in [(egui::TextStyle::Button, 15.0), (egui::TextStyle::Body, 15.0)] {
            if let Some(f) = ui.style_mut().text_styles.get_mut(&t) {
                f.size = f.size.min(size);
            }
        }
        ui.spacing_mut().button_padding.x = ui.spacing().button_padding.x.min(8.0);
        let h = ui.spacing().interact_size.y;
        // Several FM-1s plugged in: a tab for each (the bar works with the one picked), and Side by side
        if !self.others.is_empty() {
            ui.horizontal_wrapped(|ui| {
            let units = self.units_in_order();
            let mut pick: Option<usize> = None;
            for (n, (port, selected, other)) in units.iter().enumerate() {
                let shared = match other { Some(i) => self.others[*i].shared.clone(), None => self.shared.clone() };
                let name = Self::unit_name(n + 1, &shared, port);
                let tab = egui::Button::new(egui::RichText::new(&name).strong().color(if *selected { Color32::BLACK } else { Color32::WHITE }))
                    .fill(if *selected { Color32::from_rgb(255, 210, 31) } else { Color32::from_gray(55) });
                Self::fit(ui, Self::width_for(ui, &[name.as_str()]) + 8.0);
                if ui.add_sized([Self::width_for(ui, &[name.as_str()]) + 8.0, h], tab)
                    .on_hover_text(if *selected { "This FM-1: the buttons, Big Knobs, Key Lights and the keyboard work with it" } else { "Click to work with this FM-1" })
                    .clicked() {
                    pick = *other;
                }
            }
            if let Some(i) = pick {
                self.select(i);
            }
            if ui.add_sized([Self::width_for(ui, &["Side by side"]), h], egui::SelectableLabel::new(self.side_by_side, "Side by side"))
                .on_hover_text("Every FM-1's screen at once (click one to work with it). Off: the one picked, big")
                .clicked() {
                self.side_by_side = !self.side_by_side;
                self.repaint_all();
                self.store();
            }
            let lock = if self.locked { "LOCKED: knobs move all" } else { "Unlocked: one FM-1" };
            let lw = Self::width_for(ui, &["LOCKED: knobs move all", "Unlocked: one FM-1"]);
            let b = egui::Button::new(egui::RichText::new(lock).strong().color(if self.locked { Color32::BLACK } else { Color32::WHITE }))
                .fill(if self.locked { Color32::from_rgb(255, 150, 40) } else { Color32::from_gray(55) });
            Self::fit(ui, lw);
            if ui.add_sized([lw, h], b)
                .on_hover_text(if self.locked { "Locked: Big Knobs and the Key Lights strip work every FM-1 at once. Click to unlock" } else { "Unlocked: Big Knobs and the Key Lights strip work only the FM-1 picked. Click to lock them all together" })
                .clicked() {
                self.locked = !self.locked;
                self.store();
            }
            });
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            let shown = match &self.keys {
                Some(k) => format!("Keys: {}", k.name),
                None if self.keys_choice == "-" => "Keys: none".into(),
                None => "Keys…".into(),
            };
            let (choice, channel) = (self.keys_choice.clone(), self.keys_channel);
            egui::ComboBox::from_id_salt("keys").selected_text(shown).width(200.0).show_ui(ui, |ui| {
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
            let w = Self::width_for(ui, &["Hide mirror", "Show mirror"]);
            Self::fit(ui, w);
            if ui.add_sized([w, h], egui::Button::new(if self.mirror_on { "Hide mirror" } else { "Show mirror" })).clicked() {
                self.mirror_on = !self.mirror_on;
                self.store();
            }
            let w = Self::width_for(ui, &["Smooth: on", "Smooth: off"]);
            Self::fit(ui, w);
            if ui.add_sized([w, h], egui::SelectableLabel::new(self.smooth_on, if self.smooth_on { "Smooth: on" } else { "Smooth: off" })).clicked() {
                self.toggle_smooth();
            }
            // only while a ShuttleXpress is plugged in: it's the Shuttle's voice
            if self.sh_was && ui.add_sized([Self::width_for(ui, &["Voice"]), h], egui::SelectableLabel::new(self.sh_speak, "Voice"))
                .on_hover_text("Says what the Shuttle does out loud (off at first)")
                .clicked()
            {
                self.toggle_voice();
            }
            #[cfg(feature = "debugger")]
            if ui.add_sized([Self::width_for(ui, &["Debug"]), h], egui::SelectableLabel::new(self.dbg.open, "Debug"))
                .on_hover_text("The MIDI debugger: every message to and from the FM-1, decoded, with breakpoints (⌘⇧D)")
                .clicked()
            {
                self.dbg.open = !self.dbg.open;
                debug::set_on(self.dbg.open, ctx);
            }
            // One Sound menu: what you hear. FM-1 only, or tick one or several sounds (grouped by family) to play together
            let installed = mellotron::installed();
            let picked = self.picked();
            let names: Vec<String> = picked.iter().map(|id| mellotron::pretty(id)).collect();
            let shown = if picked.is_empty() && self.new_firmware_at.is_some_and(|t| t.elapsed() < Duration::from_secs(15)) {
                "New firmware: Sound FM-1 only".to_string()   // (a flash turned the app's sound off: on again here)
            } else if picked.is_empty() {
                "Sound: FM-1 only".to_string()
            } else if picked.len() <= 2 {
                format!("Sound: {}", names.join(" + "))
            } else {
                format!("Sound: {} sounds", picked.len())
            };
            let mut toggles: Vec<(String, bool)> = Vec::new();
            let mut only_fm1 = false;
            let (mut hear_fm1, mut sweeten_now, mut more) = (self.fm1_with_mellotron, self.sweeten, false);
            let mut new_pick = *rich::PICK.lock().unwrap();
            let pick_before = new_pick;
            // the lights: one small green light for each sound you hear (the FM-1's own too)
            let mut lights: Vec<(String, String)> = Vec::new();
            if picked.is_empty() || self.fm1_with_mellotron {
                lights.push(("fm1".into(), "FM-1".into()));
            }
            for (id, n) in picked.iter().zip(&names) {
                lights.push((id.clone(), n.clone()));
            }
            let sw = Self::width_for(ui, &[shown.as_str()]);
            let lights_w: f32 = lights.iter().map(|(_, n)| Self::led_w(ui, n) + ui.spacing().item_spacing.x).sum();
            Self::fit(ui, Self::combo_w(ui, sw) + ui.spacing().item_spacing.x + lights_w);   // the menu and all its lights on one row
            egui::ComboBox::from_id_salt("sound").selected_text(shown).width(sw).height(620.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show_ui(ui, |ui| {
                ui.set_min_width(260.0);
                if ui.selectable_label(picked.is_empty(), "FM-1 only (no sounds from this computer)").clicked() {
                    only_fm1 = true;
                }
                // each sound with the same light as in the bar: green = on. Click to turn it on or off
                let mut check = |ui: &mut egui::Ui, id: &str| {
                    let on = picked.iter().any(|p| p == id);
                    if Self::led(ui, on, &mellotron::pretty(id)).clicked() {
                        toggles.push((id.to_string(), !on));
                    }
                };
                let mut shown_ids: Vec<&str> = Vec::new();
                for (group, ids) in mellotron::GROUPS {
                    let have: Vec<&str> = ids.iter().copied().filter(|id| installed.iter().any(|i| i == id)).collect();
                    if have.is_empty() {
                        continue;
                    }
                    ui.separator();
                    ui.label(egui::RichText::new(*group).strong().color(Color32::from_rgb(255, 210, 31)));
                    for id in have {
                        check(ui, id);
                        shown_ids.push(id);
                    }
                }
                let others: Vec<&String> = installed.iter().filter(|i| i.as_str() != rich::ID && !shown_ids.contains(&i.as_str())).collect();
                if !others.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("More").strong().color(Color32::from_rgb(255, 210, 31)));
                    for id in others {
                        check(ui, id);
                    }
                }
                if installed.iter().any(|i| i == rich::ID) {
                    ui.separator();
                    ui.label(egui::RichText::new("Rich (follows the FM-1's sound)").strong().color(Color32::from_rgb(255, 210, 31)));
                    check(ui, rich::ID);
                    if picked.iter().any(|p| p == rich::ID) {
                        let auto_p = rich::program_for(&rich::FM1_SOUND.lock().unwrap());
                        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                            ui.selectable_value(&mut new_pick, None, format!("Auto · {} (follows the FM-1)", rich::GM[auto_p as usize]));
                            for (i, n) in rich::GM.iter().enumerate() {
                                ui.selectable_value(&mut new_pick, Some(i as u8), *n);
                            }
                        });
                    }
                }
                if installed.is_empty() {
                    ui.label("No sounds yet: Get sounds… below");
                }
                ui.separator();
                ui.add_enabled(!picked.is_empty(), egui::Checkbox::new(&mut hear_fm1, "Hear the FM-1 too"))
                    .on_hover_text("On: the FM-1's own sound as well. Off: only the sounds ticked above");
                ui.add_enabled(!picked.is_empty(), egui::Checkbox::new(&mut sweeten_now, "Sweeten"))
                    .on_hover_text("As a good stereo makes it: a little warmth and air, soft saturation, stereo width and a small room");
                ui.separator();
                if ui.button("Get sounds…").clicked() {
                    more = true;
                    ui.close_menu();
                }
            }).response.on_hover_text("What you hear: the FM-1 alone, or one or several sounds from this computer as you play");
            if only_fm1 {
                self.set_mellotron(String::new());
            }
            for (id, on) in toggles {
                self.toggle_sound(&id, on);
            }
            // the lights, right after the menu (click one to turn that sound off)
            for (id, n) in lights {
                let r = Self::led(ui, true, &n);
                if id == "fm1" {
                    r.on_hover_text(if picked.is_empty() { "You hear the FM-1's own sound" } else { "You hear the FM-1's own sound too (click: only the sounds from this computer)" })
                        .clicked().then(|| if !picked.is_empty() { hear_fm1 = false; });
                } else if r.on_hover_text(format!("You hear {n} (click to turn it off)")).clicked() {
                    self.toggle_sound(&id, false);
                }
            }
            if hear_fm1 != self.fm1_with_mellotron {
                self.fm1_with_mellotron = hear_fm1;
                self.apply_fm1_light();
                self.store();
            }
            if sweeten_now != self.sweeten {
                self.sweeten = sweeten_now;
                sweeten::ON.store(self.sweeten, std::sync::atomic::Ordering::Relaxed);
                self.store();
            }
            if new_pick != pick_before {
                *rich::PICK.lock().unwrap() = new_pick;
                self.store();
            }
            if more {
                self.open_sounds(ctx);
            }
            if self.stale() && !self.knobs_on && !self.lights_on {
                let b = egui::Button::new(egui::RichText::new("Get Knobs and Lights ›").strong().color(Color32::BLACK)).fill(Color32::from_rgb(255, 210, 31));
                if ui.add(b).on_hover_text("This firmware has the older Big Screen (the screen only). Its new version, with Big Knobs and Key Lights, is at aikellogg.com").clicked() {
                    open_url(MAKE_BIG_SCREEN);
                }
            }
            if ui.selectable_label(self.knobs_on, "Knobs")
                .on_hover_text("Big Knobs: the FM-1's eight knobs, big. Drag or scroll one to turn the real one (Big Screen firmware from aikellogg.com)")
                .clicked() {
                self.knobs_on = !self.knobs_on;
                self.store();
            }
            if ui.selectable_label(self.lights_on, "Lights")
                .on_hover_text("Key Lights: the FM-1's keys and buttons, big, lit when they're lit on the FM-1 (where your fingers hide them)")
                .clicked() {
                self.lights_on = !self.lights_on;
                self.store();
            }
            if ui.selectable_label(self.glass_on, "Spyglass").clicked() {
                self.glass_on = !self.glass_on;
                self.store();
            }
            // (its room kept while the spyglass is off)
            if ui.add_visible(self.glass_on, egui::SelectableLabel::new(self.glass_float, "Float")).clicked() {
                self.glass_float = !self.glass_float;
                let f = self.float.lock().unwrap();
                self.float_open = (f.size, f.at);
                drop(f);
                self.store();
            }
            let before = self.look;
            Self::fit(ui, Self::combo_w(ui, 120.0));
            egui::ComboBox::from_id_salt("look").selected_text(self.look.name()).width(120.0).show_ui(ui, |ui| {
                for l in Look::ALL {
                    ui.selectable_value(&mut self.look, l, l.name());
                }
            });
            if self.look != before {
                self.repaint_all(); // repaint in the new look
                self.store();
            }
            let w = Self::width_for(ui, &["Leave full screen", "Full screen"]);
            Self::fit(ui, w);
            if ui.add_sized([w, h], egui::Button::new(if self.full_screen { "Leave full screen" } else { "Full screen" })).clicked() {
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
            // (a fixed width: "FM-1: not found" and a firmware's name take the same room)
            Self::fit(ui, 170.0);
            ui.add_sized([170.0, h], egui::Label::new(egui::RichText::new(status).color(Color32::GRAY)).truncate());
            // The sound playing, standing out: big, bold, in gold on a dark chip.
            // the sound playing, in a box of its own size (a long name is cut short, not the row reflowed); its room
            // kept while there's none
            let sound = self.shared.lock().unwrap().sound.clone();
            {
                let sound = sound.unwrap_or_default();
                let empty = sound.is_empty();
                Self::fit(ui, 284.0);
                egui::Frame::new()
                    .fill(if empty { Color32::TRANSPARENT } else { Color32::from_rgb(0x2a, 0x22, 0x0a) })
                    .stroke(if empty { egui::Stroke::NONE } else { egui::Stroke::new(1.5_f32, Color32::from_rgb(0xf2, 0xc0, 0x4c)) })
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.set_width(260.0);
                        ui.add_visible(!empty, egui::Label::new(egui::RichText::new(format!("\u{266a} {sound}")).size(17.0).strong().color(Color32::from_rgb(0xf2, 0xc0, 0x4c))).truncate());
                    });
            }
        });
    }
}

/// Every FM-1 plugged in, by port name: over USB (one that names itself, Open firmware, before the original's "USB
/// Composite"); over Bluetooth only an FM-1 that isn't also on USB (it would be the same one, slower).
fn fm1_ports(scanner: Option<&MidiOutput>) -> Vec<String> {
    let Some(o) = scanner else { return Vec::new() };
    let names: Vec<String> = o.ports().iter().filter_map(|p| o.port_name(p).ok()).filter(|n| is_fm1(n)).collect();
    let mut usb: Vec<String> = names.iter().filter(|n| !is_bluetooth(n)).cloned().collect();
    usb.sort_by_key(|n| (n.to_lowercase().contains("usb composite"), n.clone()));
    if usb.is_empty() {
        let mut bt: Vec<String> = names.into_iter().filter(|n| is_bluetooth(n)).collect();
        bt.sort();
        return bt;
    }
    usb
}

/// Opens the FM-1 on port `name`, both ways; its screen messages go into `shared`.
fn open(name: &str, shared: Arc<Mutex<Shared>>, played: Arc<Mutex<keys::Played>>, ctx: egui::Context) -> Option<Link> {
    let mut input = MidiInput::new(APP_NAME).ok()?;
    input.ignore(Ignore::None); // SysEx is the whole point
    let output = MidiOutput::new(APP_NAME).ok()?;
    let ports = input.ports();
    let in_port = ports.iter().find(|p| input.port_name(p).is_ok_and(|n| n == name))?;
    let name = name.to_string();
    let out_port = output.ports().into_iter().find(|p| output.port_name(p).is_ok_and(|n| n == name))?;
    let output = Arc::new(Mutex::new(output.connect(&out_port, "FM-1 Big Screen out").ok()?));
    let fb: Arc<fm1base::State> = Arc::default();       // a new link: which mirror it speaks, found again
    let fb_in = fb.clone();
    let mut parser = keys::Parser::default();
    {
        let mut s = shared.lock().unwrap();
        s.lights = None; // and its lights: the last firmware's are not this one's
        s.screen_since = None;
    }
    let base: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let base_in = base.clone();
    let v2_answers: Arc<std::sync::atomic::AtomicU32> = Arc::default();
    let v2_in = v2_answers.clone();
    let port_in = name.clone();
    let input = input
        .connect(
            in_port,
            "FM-1 Big Screen in",
            move |_, raw, _| {
                // The debugger first: a breakpoint stops this thread here, before the app has acted on the message.
                let kept: Vec<u8>;
                let buffer: &[u8] = if debug::is_on() {
                    let mut keep = Vec::with_capacity(raw.len());
                    let (mut i, mut skipped) = (0, false);
                    while i < raw.len() {
                        // one message: a SysEx to its F7, else a status byte and its data
                        let end = if raw[i] == 0xF0 {
                            raw[i..].iter().position(|&b| b == 0xF7).map_or(raw.len(), |p| i + p + 1)
                        } else {
                            let mut e = i + 1;
                            while e < raw.len() && raw[e] < 0x80 { e += 1; }
                            e
                        };
                        let m = &raw[i..end];
                        if debug::check(&port_in, debug::Dir::In, m, "MIDI in", true) { keep.extend_from_slice(m) } else { skipped = true }
                        i = end;
                    }
                    if skipped { kept = keep; &kept } else { raw }
                } else {
                    raw
                };
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
                if let Some(l) = knobs::lights(m) {
                    if s.lights != Some(l) {
                        s.lights = Some(l);
                        ctx.request_repaint();
                    }
                }
                if let Some(name) = screen::sound(m) {
                    if s.sound.as_deref() != Some(name.as_str()) {
                        *rich::FM1_SOUND.lock().unwrap() = name.clone();   // Rich "Auto" follows it
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
                        s.screen_since.get_or_insert_with(Instant::now);
                        s.last = Some(Instant::now());
                        s.changed = true;
                        ctx.request_repaint();
                    }
                    bigscreen2::Got::Nothing => {}
                }
                if s.screen.apply(m) {
                    s.screen_since.get_or_insert_with(Instant::now);
                    s.last = Some(Instant::now());
                    s.changed = true;
                    ctx.request_repaint();
                }
                // Charles Vestal's mirror (fm1-base firmware): its answer, and its rectangles and fills.
                match fm1base::apply(&mut s.screen, m, &fb_in) {
                    fm1base::Got::Hello => {
                        base_in.store(true, std::sync::atomic::Ordering::Relaxed);
                        fb_in.active.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    fm1base::Got::Lights(l) => {
                        if s.lights != Some(l) {
                            s.lights = Some(l);
                            ctx.request_repaint();
                        }
                    }
                    fm1base::Got::Answered => fb_in.panel.lock().unwrap().answered(),
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
    let fb_h = fb.clone();
    let port_h = name.clone();
    std::thread::spawn(move || hellos(port_h, out, st, lo, base, v2_answers, fb_h));
    Some(Link { name, _input: input, out: output, opened: Instant::now(), stop, lost, fb })
}

impl App {
    /// Every FM-1's screen at once, in a row (or a grid when there are many), the one picked framed in yellow. Click one
    /// to work with it. The spyglass floats over them as usual.
    fn side_by_side_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, area: Rect) {
        let units = self.units_in_order();
        let n = units.len().max(1);
        // as many columns as make each screen biggest
        let (mut cols, mut best) = (1, 0.0f32);
        for c in 1..=n {
            let rows = n.div_ceil(c);
            let side = (area.width() / c as f32).min((area.height() - 34.0 * rows as f32) / rows as f32);
            if side > best {
                (cols, best) = (c, side);
            }
        }
        let rows = n.div_ceil(cols);
        let cell = vec2(area.width() / cols as f32, area.height() / rows as f32);
        let mut pick: Option<usize> = None;
        for (k, (port, selected, other)) in units.iter().enumerate() {
            let at = area.min + vec2((k % cols) as f32 * cell.x, (k / cols) as f32 * cell.y);
            let r = Rect::from_min_size(at, cell).shrink(6.0);
            let (shared, tex) = match other {
                Some(i) => (self.others[*i].shared.clone(), self.others[*i].texture.clone()),
                None => (self.shared.clone(), self.texture.clone()),
            };
            let name = Self::unit_name(k + 1, &shared, port);
            let screen = fit(Rect::from_min_max(r.min + vec2(0.0, 34.0), r.max));
            let label = Rect::from_min_size(Pos2::new(r.left(), screen.top() - 34.0), vec2(r.width(), 30.0));   // right above it
            let p = ui.painter();
            p.text(label.center(), Align2::CENTER_CENTER, &name, FontId::proportional(22.0),
                if *selected { Color32::from_rgb(255, 210, 31) } else { Color32::from_gray(200) });
            match &tex {
                Some(t) if self.mirror_on => {
                    p.image(t.id(), screen, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                _ => {
                    p.text(screen.center(), Align2::CENTER_CENTER, "…", FontId::proportional(40.0), Color32::WHITE);
                }
            }
            if *selected {
                p.rect_stroke(screen.expand(4.0), 6.0, egui::Stroke::new(4.0_f32, Color32::from_rgb(255, 210, 31)), egui::StrokeKind::Outside);
            }
            let resp = ui.interact(screen.union(label), ui.id().with(("unit", k)), egui::Sense::click());
            if !*selected {
                if resp.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.on_hover_text("Click to work with this FM-1").clicked() {
                    pick = *other;
                }
            }
        }
        if let Some(i) = pick {
            self.select(i);
        }
        if self.glass_on && !self.glass_float {
            self.spyglass(ui, ctx, area);
        }
    }

    /// The spyglass over the screen area: the first time in the room beside the FM-1's (square,
    /// centred) screen if there's enough, else top right over it. Drag to move; drag the grip or scroll to resize.
    fn spyglass(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, area: Rect) {
        let (hearing, wave, wave2) = hear(&self.sound_in, &self.glass);
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
        self.glass.draw(ui.painter(), c, d, sound.as_deref(), hearing, wave.as_deref(), wave2.as_deref());
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
/// What the spyglass shows on the right: whether a sound is coming in, its waveform, and a second one at once. The
/// Mellotron's own sound while it plays; with the FM-1 light lit too (both sounds), the FM-1's (from its USB sound)
/// with the Mellotron's over it.
fn hear(sound_in: &Mutex<scope::Ring>, glass: &glass::Glass) -> (bool, Option<Vec<f32>>, Option<Vec<f32>>) {
    let mel = mellotron::wave(glass.hz());
    let ring = sound_in.lock().unwrap();
    let fm1_heard = !FM1_DOWN.load(std::sync::atomic::Ordering::Relaxed) && ring.listening();
    let fm1 = if fm1_heard { glass.hz().and_then(|hz| ring.wave(hz)) } else { None };
    match (mel, fm1_heard) {
        (Some(m), true) => (true, fm1, Some(m)),   // both: the FM-1's, and the Mellotron's over it
        (Some(m), false) => (true, Some(m), None), // the Mellotron's own sound
        (None, _) => (ring.listening(), glass.hz().and_then(|hz| ring.wave(hz)), None),
    }
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
        let (hearing, wave, wave2) = hear(sound_in, &f.glass);
        f.glass.draw(ui.painter(), c, d, sound.as_deref(), hearing, wave.as_deref(), wave2.as_deref());
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
        #[cfg(feature = "debugger")]
        self.debug_window(ctx);
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
        self.shuttle_frame(ctx);
        let need = self.need();
        if self.install.as_ref().is_some_and(|p| p.lock().unwrap().done) && need.is_none() {
            self.install = None; // the screen is back
        }
        // No FM-1 with the spyglass in this window: no panel over it, the spyglass goes on with the keyboard.
        let no_fm1 = self.link.is_none() && self.install.is_none();
        let spyglass_alone = no_fm1 && self.glass_on && !self.glass_float;
        let showing_help = (need.is_some() || self.install.is_some()) && !spyglass_alone;
        if showing_help || self.last_move.elapsed() < BAR_FOR || self.bar_h == 0.0 {
            let shown = egui::TopBottomPanel::bottom("buttons")
                .frame(egui::Frame::new().fill(Color32::from_gray(28)).inner_margin(egui::Margin::same(14)))
                .show(ctx, |ui| self.buttons(ui, ctx));
            self.bar_h = shown.response.rect.height();
            ctx.request_repaint_after(BAR_FOR); // to hide them again
        } else {
            // hidden, but its room kept (dark): the FM-1's screen never changes size when the buttons come and go
            egui::TopBottomPanel::bottom("buttons").exact_height(self.bar_h).show_separator_line(false)
                .frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |_| {});
        }
        let stale = self.stale();
        if !stale && self.shared.lock().unwrap().lights.is_none() && self.link.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1)); // (to notice a stale Big Screen in time)
        }
        if stale && (self.knobs_on || self.lights_on) && !showing_help {
            // the older Big Screen: no knobs or lights to show, the way to them instead (the screen keeps its size)
            egui::TopBottomPanel::bottom("stale").show_separator_line(false)
                .frame(egui::Frame::new().fill(Color32::from_gray(18)).inner_margin(egui::Margin::same(12)))
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("This firmware has the older Big Screen: the screen only. For Big Knobs and Key Lights, install its new version.")
                            .size(22.0).strong().color(Color32::from_rgb(255, 210, 31)));
                        let b = egui::Button::new(egui::RichText::new("Get it at aikellogg.com  ›").size(24.0).strong().color(Color32::BLACK))
                            .fill(Color32::from_rgb(255, 210, 31)).min_size(vec2(320.0, 48.0));
                        if ui.add(b).clicked() {
                            open_url(MAKE_BIG_SCREEN);
                        }
                    });
                });
        }
        if self.knobs_on && !showing_help && !stale {
            let links: Vec<&Link> = { let mut v: Vec<&Link> = self.link.iter().collect(); if self.locked { v.extend(self.others.iter().filter_map(|u| u.link.as_ref())); } v };
            let knobs = &mut self.knobs;
            egui::TopBottomPanel::bottom("knobs").exact_height(130.0).show_separator_line(false)
                .frame(egui::Frame::new().fill(Color32::from_gray(18)).inner_margin(egui::Margin::same(6)))
                .show(ctx, |ui| knobs::knobs_ui(ui, knobs, &mut |m| {
                    // to the FM-1 picked, or every one while locked
                    for l in &links {
                        // Charles Vestal's fm1-base mirror: its own remote, one command at a time (fm1base.rs)
                        l.send_remote(m, |panel| match m.get(4) {
                            Some(4) => panel.turn(m[5], if m[6] >= 64 { m[6] as i32 - 128 } else { m[6] as i32 }),
                            Some(6) => panel.master(m[5] as u16 | (m[6] as u16) << 7),
                            _ => {}
                        });
                    }
                }));
        }
        if self.lights_on && !showing_help && !stale {
            let played = self.played.lock().unwrap().keys_down();
            let links: Vec<&Link> = { let mut v: Vec<&Link> = self.link.iter().collect(); if self.locked { v.extend(self.others.iter().filter_map(|u| u.link.as_ref())); } v };
            let held = &mut self.panel_held;
            let got = self.shared.lock().unwrap().lights;
            let (l, seen) = (got.unwrap_or_default(), got.is_some());
            egui::TopBottomPanel::bottom("lights").exact_height(150.0).show_separator_line(false)
                .frame(egui::Frame::new().fill(Color32::from_gray(12)).inner_margin(egui::Margin::same(6)))
                .show(ctx, |ui| knobs::lights_ui(ui, l, knobs::played_keys(&played), seen, held, &mut |id, down| {
                    for lk in &links {
                        lk.send_remote(&knobs::press(id, down), |p| p.press(id, down));
                    }
                }));
        }
        if self.multi_install_note.is_some_and(|t| t.elapsed() < Duration::from_secs(12)) {
            egui::TopBottomPanel::top("multi_install").show_separator_line(false)
                .frame(egui::Frame::new().fill(Color32::from_gray(18)).inner_margin(egui::Margin::same(12)))
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new("To install firmware, leave just that one FM-1 plugged in: with several, it could go to the wrong one.")
                        .size(22.0).strong().color(Color32::from_rgb(255, 210, 31)));
                });
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        let side_by_side = self.side_by_side && !self.others.is_empty() && !showing_help;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
            let area = ui.max_rect();
            let p = ui.painter();
            if side_by_side {
                self.side_by_side_ui(ui, ctx, area);
            } else if showing_help {
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
                    // B3 ORGAN's switches, clickable on the big screen (PERC, 3RD, FAST, SOFT, LES, OD, KEYS, vibrato)
                    let b3 = self.shared.lock().unwrap().sound.as_deref().is_some_and(|n| n.starts_with("B3 ORGAN"));
                    if b3 {
                        let r = fit(area);
                        let resp = ui.interact(r, ui.id().with("screen_switches"), egui::Sense::click());
                        let at = |pos: Pos2| knobs::b3_hotspot((pos.x - r.left()) / r.width() * 240.0, (pos.y - r.top()) / r.height() * 240.0);
                        if resp.hover_pos().is_some_and(|q| at(q).is_some()) {
                            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            if let Some(id) = resp.interact_pointer_pos().and_then(at) {
                                self.tap(id);
                            }
                        }
                    }
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
