#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows
//! FM-1 Mirror: the M-VAVE FM-1's screen, big, on the Mac.
//!
//! With the FM-1 Big Screen firmware, the FM-1 sends its screen over its USB MIDI connection
//! while this app keeps asking for it (see screen.rs). It's drawn as large as the window allows,
//! pixel-sharp, in colour or in high-contrast looks for reading. The buttons hide after a few
//! seconds so the screen is as big as it can be: move the mouse to bring them back.
//! Keys plays the FM-1 from a keyboard through the app, and the spyglass beside the screen shows
//! what's played (keys.rs, glass.rs), as in FM-1 Mirror + Spyglass on Android.

mod chord;
mod glass;
mod glyphs;
mod install;
mod keys;
mod scope;
mod screen;
mod smooth;
mod staff;
mod usbname;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align2, Color32, ColorImage, FontId, Pos2, Rect, TextureHandle, TextureOptions};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde_json::{json, Value};

use screen::{Screen, ASK_INFO, HELLO, HELLO_ALL, SIZE};

const APP_NAME: &str = "FM-1 Mirror";
/// How long the buttons stay after the mouse last moved.
const BAR_FOR: Duration = Duration::from_secs(4);
/// How often to say hello (the FM-1 stops sending three seconds after the last one).
const HELLO_EVERY: Duration = Duration::from_millis(900);
/// Nothing from the FM-1 this long after connecting: it isn't sending its screen.
const QUIET: Duration = Duration::from_secs(4);
/// The floating spyglass: room around the eyepiece in its window, and its smallest and largest size.
const FLOAT_PAD: f32 = 14.0;
const FLOAT_SIZES: (f32, f32) = (140.0, 900.0);

/// The spyglass in a window of its own. It keeps its own state and repaints on its own: macOS stops
/// redrawing FM-1 Mirror's window behind others, and the spyglass has to keep going then.
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
fn hellos(output: Arc<Mutex<MidiOutputConnection>>, stop: Arc<std::sync::atomic::AtomicBool>, lost: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    let opened = Instant::now();
    let (mut asked_identity, mut asked_build, mut asked_info, mut first) = (false, false, false, true);
    let mut hello_at: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        if hello_at.is_none_or(|t| t.elapsed() >= HELLO_EVERY) {
            hello_at = Some(Instant::now());
            if output.lock().unwrap().send(if first { &HELLO_ALL } else { &HELLO }).is_err() {
                lost.store(true, Ordering::Relaxed);
                return;
            }
            first = false;
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
        std::thread::sleep(Duration::from_millis(100));
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
    Some(dirs::config_dir()?.join(APP_NAME).join("settings.json"))
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
        App {
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
            smooth_on: saved["smooth"].as_bool().unwrap_or(true),
            glass_float: saved["spyglass_float"].as_bool().unwrap_or(false),
            float: Arc::new(Mutex::new(Float { glass: glass::Glass::default(), size: float_size, at: float_at, close: false, dirty: false })),
            float_open: (float_size, float_at),
        }
    }

    fn store(&self) {
        let Some(path) = settings_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let mut s = json!({ "look": self.look.key(), "keys": self.keys_choice, "keys_channel": self.keys_channel, "spyglass": self.glass_on, "mirror": self.mirror_on, "smooth": self.smooth_on, "spyglass_float": self.glass_float });
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
            if !s.changed {
                return;
            }
            s.changed = false;
            s.screen.pixels.clone()
        };
        let painted: Vec<Color32> = pixels.into_iter().map(|c| self.look.paint(c)).collect();
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

    fn toggle_smooth(&mut self) {
        self.smooth_on = !self.smooth_on;
        self.mirror_on = true; // it's the mirror that's smooth: show it
        self.shared.lock().unwrap().changed = true; // repaint it the other way
        self.store();
    }

    /// Why there's no picture, if there isn't one.
    fn need(&self) -> Option<Need> {
        let (last, identity) = {
            let s = self.shared.lock().unwrap();
            (s.last, s.identity.clone())
        };
        let Some(link) = &self.link else { return Some(Need::Plug) };
        // The FM-1 only sends what changes, so a still screen is quiet: once it has come, keep it.
        if last.is_some_and(|t| t >= link.opened) {
            return None;
        }
        if link.opened.elapsed() < QUIET {
            return Some(Need::Connecting);
        }
        Some(match identity {
            Some(id) if is_bluetooth(&link.name) && !install::is_updater(&id) => Need::Bluetooth, // installs need USB
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
    let output = Arc::new(Mutex::new(output.connect(&out_port, "FM-1 Mirror out").ok()?));
    let mut parser = keys::Parser::default();
    let input = input
        .connect(
            in_port,
            "FM-1 Mirror in",
            move |_, buffer, _| {
                // Notes from the FM-1's own keys (KEY OUT on), for the spyglass.
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
                if s.screen.apply(m) {
                    s.last = Some(Instant::now());
                    s.changed = true;
                    ctx.request_repaint();
                }
                }
            },
            (),
        )
        .ok()?;
    let stop: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let lost: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let (st, lo, out) = (stop.clone(), lost.clone(), output.clone());
    std::thread::spawn(move || hellos(out, st, lo));
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
        let sound = self.shared.lock().unwrap().sound.clone();
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
    let sound = shared.lock().unwrap().sound.clone();
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
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([900.0, 960.0])
            .with_min_inner_size([300.0, 340.0])
            .with_transparent(true), // (lets the floating spyglass's window be see-through)
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
