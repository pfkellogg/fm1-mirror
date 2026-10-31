//! FM-1 Mirror: the M-VAVE FM-1's screen, big, on the Mac.
//!
//! With the FM-1 Big Screen firmware, the FM-1 sends its screen over its USB MIDI connection
//! while this app keeps asking for it (see screen.rs). It's drawn as large as the window allows,
//! pixel-sharp, in colour or in high-contrast looks for reading. The buttons hide after a few
//! seconds so the screen is as big as it can be: move the mouse to bring them back.

mod screen;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align2, Color32, ColorImage, FontId, Pos2, Rect, TextureHandle, TextureOptions};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde_json::{json, Value};

use screen::{Screen, HELLO, SIZE};

const APP_NAME: &str = "FM-1 Mirror";
/// How long the buttons stay after the mouse last moved.
const BAR_FOR: Duration = Duration::from_secs(4);
/// How often to say hello (the FM-1 stops sending three seconds after the last one).
const HELLO_EVERY: Duration = Duration::from_millis(900);
/// Nothing from the FM-1 for this long while connected: it isn't sending its screen.
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
}

/// The FM-1's USB MIDI ports, open both ways.
struct Link {
    name: String,
    _input: MidiInputConnection<()>,
    output: MidiOutputConnection,
    opened: Instant,
}

/// The FM-1's MIDI port: "FM1 Community" with Open firmware, "USB Composite Device" with the
/// original (which has no mirror, but is worth recognising to say so).
fn is_fm1(name: &str) -> bool {
    let n = name.to_lowercase();
    (n.contains("fm1") || n.contains("fm-1") || n.contains("usb composite")) && !n.contains("bridge")
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(APP_NAME).join("settings.json"))
}

struct App {
    shared: Arc<Mutex<Shared>>,
    link: Option<Link>,
    looked: Option<Instant>,
    hello_at: Option<Instant>,
    look: Look,
    full_screen: bool,
    texture: Option<TextureHandle>,
    last_move: Instant,
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
            looked: None,
            hello_at: None,
            look: Look::ALL.into_iter().find(|l| saved["look"] == l.key()).unwrap_or(Look::Color),
            full_screen: false,
            texture: None,
            last_move: Instant::now(),
        }
    }

    fn store(&self) {
        let Some(path) = settings_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(path, json!({ "look": self.look.key() }).to_string());
    }

    /// Finds and opens the FM-1 (now and then while it's not there), and keeps saying hello.
    fn keep_linked(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(link) = &self.link {
            let still_there = MidiOutput::new(APP_NAME)
                .map(|o| o.ports().iter().any(|p| o.port_name(p).is_ok_and(|n| n == link.name)))
                .unwrap_or(false);
            if !still_there && self.looked.is_none_or(|t| now - t > Duration::from_secs(1)) {
                self.link = None;
            }
        }
        if self.link.is_none() && self.looked.is_none_or(|t| now - t > Duration::from_secs(2)) {
            self.looked = Some(now);
            self.link = open(self.shared.clone(), ctx.clone());
            self.hello_at = None;
        }
        if let Some(link) = &mut self.link {
            if self.hello_at.is_none_or(|t| now - t >= HELLO_EVERY) {
                self.hello_at = Some(now);
                if link.output.send(&HELLO).is_err() {
                    self.link = None;
                }
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

    /// What's wrong, in words, if there's no picture.
    fn waiting(&self) -> Option<String> {
        let last = self.shared.lock().unwrap().last;
        let Some(link) = &self.link else {
            return Some("Waiting for the FM-1…\n\nPlug it into this Mac with its USB cable and turn it on.".into());
        };
        if last.is_some_and(|t| t.elapsed() < QUIET) {
            return None;
        }
        if link.opened.elapsed() < QUIET {
            return Some("Connecting to the FM-1…".into());
        }
        Some(if link.name.to_lowercase().contains("usb composite") {
            "This FM-1 has its original firmware, which can't send its screen.\n\nIt needs the FM-1 Big Screen firmware.".into()
        } else {
            format!("Connected to \"{}\", but it isn't sending its screen.\n\nIt needs the FM-1 Big Screen firmware.", link.name)
        })
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
            move |_, m, _| {
                let mut s = shared.lock().unwrap();
                if s.screen.apply(m) {
                    s.last = Some(Instant::now());
                    s.changed = true;
                    ctx.request_repaint();
                }
            },
            (),
        )
        .ok()?;
    Some(Link { name, _input: input, output, opened: Instant::now() })
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
        let waiting = self.waiting();
        if waiting.is_some() || self.last_move.elapsed() < BAR_FOR {
            egui::TopBottomPanel::bottom("buttons")
                .frame(egui::Frame::new().fill(Color32::from_gray(28)).inner_margin(egui::Margin::same(14)))
                .show(ctx, |ui| self.buttons(ui, ctx));
            ctx.request_repaint_after(BAR_FOR); // to hide them again
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
            let area = ui.max_rect();
            let p = ui.painter();
            if let Some(text) = waiting {
                let g = p.layout(text, FontId::proportional(32.0), Color32::WHITE, area.width() - 80.0);
                p.galley(area.center() - g.size() / 2.0, g, Color32::WHITE);
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
