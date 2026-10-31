#![cfg_attr(windows, windows_subsystem = "windows")] // no console window on Windows
//! FM-1 Mirror: the M-VAVE FM-1's screen, big, on the Mac.
//!
//! With the FM-1 Big Screen firmware, the FM-1 sends its screen over its USB MIDI connection
//! while this app keeps asking for it (see screen.rs). It's drawn as large as the window allows,
//! pixel-sharp, in colour or in high-contrast looks for reading. The buttons hide after a few
//! seconds so the screen is as big as it can be: move the mouse to bring them back.

mod install;
mod screen;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align2, Color32, ColorImage, FontId, Pos2, Rect, TextureHandle, TextureOptions};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde_json::{json, Value};

use screen::{Screen, HELLO, HELLO_ALL, SIZE};

const APP_NAME: &str = "FM-1 Mirror";
/// How long the buttons stay after the mouse last moved.
const BAR_FOR: Duration = Duration::from_secs(4);
/// How often to say hello (the FM-1 stops sending three seconds after the last one).
const HELLO_EVERY: Duration = Duration::from_millis(900);
/// Nothing from the FM-1 this long after connecting: it isn't sending its screen.
const QUIET: Duration = Duration::from_secs(4);

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
                let k = |v: u8| (v as f32 * 1.6).min(255.0) as u8;
                Color32::from_rgb(k(r), k(g), k(b))
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
}

/// The FM-1's USB MIDI ports, open both ways.
struct Link {
    name: String,
    _input: MidiInputConnection<()>,
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
fn hellos(mut output: MidiOutputConnection, stop: Arc<std::sync::atomic::AtomicBool>, lost: Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    let opened = Instant::now();
    let (mut asked_identity, mut asked_build, mut first) = (false, false, true);
    let mut hello_at: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        if hello_at.is_none_or(|t| t.elapsed() >= HELLO_EVERY) {
            hello_at = Some(Instant::now());
            if output.send(if first { &HELLO_ALL } else { &HELLO }).is_err() {
                lost.store(true, Ordering::Relaxed);
                return;
            }
            first = false;
        }
        if !asked_identity && opened.elapsed() > Duration::from_secs(2) {
            asked_identity = true;
            let _ = output.send(&install::frame(17, &[]));
        }
        if !asked_build && opened.elapsed() > Duration::from_secs(3) {
            asked_build = true;
            let _ = output.send(&install::frame(49, &[]));
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
    (n.contains("fm1") || n.contains("fm-1") || n.contains("usb composite") || n.starts_with("usb-midi") || n.starts_with("usb midi"))
        && !n.contains("bridge")
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
}

/// Why there's no picture yet.
enum Need {
    Plug,
    Connecting,
    /// M-VAVE's original firmware: first M-VAVE's Official Open (on M-VAVE's page), then this app.
    Original,
    /// M-VAVE's Open firmware without the mirror: this app can install Big Screen.
    Open,
    Other(String),
    Unknown(String),
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
        }
    }

    fn store(&self) {
        let Some(path) = settings_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(path, json!({ "look": self.look.key() }).to_string());
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
        if self.looked.is_none_or(|t| now - t >= Duration::from_secs(2)) {
            self.looked = Some(now);
            if let Some(link) = &self.link {
                let still_there = self.scanner.as_ref().is_some_and(|o| o.ports().iter().any(|p| o.port_name(p).is_ok_and(|n| n == link.name)));
                if !still_there || link.lost.load(std::sync::atomic::Ordering::Relaxed) {
                    self.link = None;
                }
            }
            if self.link.is_none() {
                {
                    let mut s = self.shared.lock().unwrap();
                    s.identity = None;
                    s.build = None;
                }
                self.link = open(self.shared.clone(), ctx.clone());
            }
        }
        ctx.request_repaint_after(HELLO_EVERY);
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
        let img = ColorImage { size: [SIZE, SIZE], pixels: pixels.into_iter().map(|c| self.look.paint(c)).collect() };
        // Nearest: the FM-1's 5x7 letters stay crisp blocks however big they get.
        match &mut self.texture {
            Some(t) => t.set(img, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture("fm1", img, TextureOptions::NEAREST)),
        }
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
            Some(id) if id.starts_with("FM-1_0") => Need::Original,
            Some(id) if install::is_open_firmware(&id) => Need::Open,
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
            body(ui, "Step 1 · On M-VAVE's FM1 Workbench page (in Chrome or Edge), put M-VAVE's free Official Open firmware on it: Device update → Recovery & force update → tick \"Force flash\" → Connect device → tick \"I have backed up…\" → Force flash. It replaces the FM-1's sounds with M-VAVE's: if you made your own, save them first on the page's Projects & sounds tab.");
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
                    body(ui, "Leave the FM-1 plugged in and on, and click Try again. M-VAVE's page can also always install firmware on it (Device update).");
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
                    heading(ui, "Waiting for the FM-1…");
                    body(ui, "Plug it into this computer with its USB cable and turn it on.");
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
                Need::Other(id) => {
                    heading(ui, "This FM-1 runs another firmware");
                    body(ui, &format!("It says it is \"{id}\". Big Screen installs over M-VAVE's Official Open firmware, so that comes first."));
                    step1(ui);
                }
                Need::Unknown(name) => {
                    heading(ui, "No screen from the FM-1 yet");
                    body(ui, &format!("Connected to \"{name}\", but it isn't sending its screen and didn't say which firmware it runs. With M-VAVE's original firmware:"));
                    step1(ui);
                }
            }
            ui.add_space(24.0);
            ui.label(
                egui::RichText::new(
                    "Going back is easy: on M-VAVE's page, Device update → Start update puts back M-VAVE's own Official Open firmware, and Recovery & force update → Restore original V15 the firmware it came with.",
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
            if showing && other_build.is_some() && self.install.is_none() {
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
            let status = match &self.link {
                Some(l) => format!("FM-1: {}", l.name),
                None => "FM-1: not found".into(),
            };
            ui.label(egui::RichText::new(status).color(Color32::GRAY));
        });
    }
}

/// Opens the first FM-1 found, both ways; its screen messages go into `shared`.
fn open(shared: Arc<Mutex<Shared>>, ctx: egui::Context) -> Option<Link> {
    let mut input = MidiInput::new(APP_NAME).ok()?;
    input.ignore(Ignore::None); // SysEx is the whole point
    let output = MidiOutput::new(APP_NAME).ok()?;
    // An FM-1 that names itself first (Open firmware), then one that doesn't (the original).
    let ports = input.ports();
    let named = |p: &&midir::MidiInputPort| input.port_name(p).is_ok_and(|n| is_fm1(&n) && !n.to_lowercase().contains("usb composite"));
    let in_port = ports.iter().find(named).or_else(|| ports.iter().find(|p| input.port_name(p).is_ok_and(|n| is_fm1(&n))))?;
    let name = input.port_name(in_port).ok()?;
    let out_port = output.ports().into_iter().find(|p| output.port_name(p).is_ok_and(|n| n == name))?;
    let output = output.connect(&out_port, "FM-1 Mirror out").ok()?;
    let input = input
        .connect(
            in_port,
            "FM-1 Mirror in",
            move |_, buffer, _| {
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
    let (st, lo) = (stop.clone(), lost.clone());
    std::thread::spawn(move || hellos(output, st, lo));
    Some(Link { name, _input: input, opened: Instant::now(), stop, lost })
}

/// The biggest square that fits in `area`, in its middle.
fn fit(area: Rect) -> Rect {
    let side = area.width().min(area.height());
    Rect::from_center_size(area.center(), vec2(side, side))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.keep_linked(ctx);
        self.take_screen(ctx);
        let (moved, full_key, escape) = ctx.input(|i| {
            (
                i.pointer.delta().length() > 0.0 || i.pointer.any_pressed(),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if moved {
            self.last_move = Instant::now();
        }
        if full_key || (escape && self.full_screen) {
            self.full_screen = !self.full_screen && !escape;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full_screen));
        }
        let need = self.need();
        if self.install.as_ref().is_some_and(|p| p.lock().unwrap().done) && need.is_none() {
            self.install = None; // the screen is back
        }
        let showing_help = need.is_some() || self.install.is_some();
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
            } else if let Some(tex) = &self.texture {
                p.image(tex.id(), fit(area), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
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
            .with_min_inner_size([300.0, 340.0]),
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
