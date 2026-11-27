//! The debugger's window (debug.rs does the stopping): controls, breakpoints, the stopped message decoded, the app's
//! state (what changed since the last stop in yellow), sending bytes of your own, and the log. Its own window, so the
//! FM-1's screen keeps all of its size. ⌘⇧D / Ctrl+Shift+D, or Debug in the bar.

use crate::debug::{self, Breakpoint, Dir, Event, Kind, Step, What};
use eframe::egui::{self, Color32, RichText};
use std::time::Instant;

const Y: Color32 = Color32::from_rgb(255, 214, 10);
const GREY: Color32 = Color32::from_rgb(140, 152, 170);

/// What the "Add a breakpoint" list offers: the firmware's hello first.
fn choices() -> Vec<(String, What)> {
    let mut v = vec![("The firmware says hello (any mirror)".to_string(), What::SaysHello), ("Any message".to_string(), What::Any)];
    v.extend(Kind::ALL.iter().map(|k| (k.name().to_string(), What::Kind(*k))));
    v.push(("Bytes that start with…".to_string(), What::Bytes(vec![])));
    v
}

pub struct DebugUi {
    pub open: bool,
    pick: usize,
    pattern: String,
    port: String,
    dir: usize,
    skip: String,
    log_only: bool,
    selected: Option<Event>,
    hide_pictures: bool,
    filter: String,
    follow: bool,
    /// the app's state at the last stop, and which stop that was (for "changed since")
    before: Vec<(String, String)>,
    at_stop: Vec<(String, String)>,
    shown_stop: Option<u64>,
    now_state: Vec<(String, String)>,
    send: String,
    send_to: String,
    note: String,
}

impl Default for DebugUi {
    fn default() -> Self {
        DebugUi {
            open: false, pick: 0, pattern: "F0 7D 46 53 ??".into(), port: String::new(), dir: 0, skip: String::new(), log_only: false,
            selected: None, hide_pictures: true, filter: String::new(), follow: true, before: vec![], at_stop: vec![], shown_stop: None, now_state: vec![],
            send: "F0 7D 46 4C 01 F7".into(), send_to: String::new(), note: String::new(),
        }
    }
}

fn age(t: Option<Instant>) -> String {
    t.map_or("never".into(), |t| format!("{:.1} s ago", t.elapsed().as_secs_f32()))
}

impl crate::App {
    /// Everything the app knows, one row each: the FM-1s and their links, then the app's own settings.
    fn debug_state(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        let mut unit = |n: usize, port: &str, picked: bool, shared: &std::sync::Mutex<crate::Shared>, link: Option<&crate::Link>| {
            let s = shared.lock().unwrap();
            let p = format!("FM-1 {n}{}", if picked { " (picked)" } else { "" });
            let mut row = |k: &str, val: String| v.push((format!("{p} · {k}"), val));
            row("port", port.to_string());
            row("linked", link.map_or("no".into(), |l| format!("yes, {:.0} s{}", l.opened.elapsed().as_secs_f32(), if l.lost.load(std::sync::atomic::Ordering::Relaxed) { ", LOST" } else { "" })));
            row("mirror", link.map_or("—".into(), |l| if l.fb.is_active() { "fm1-base (Charles Vestal's)".into() } else { "Big Screen 1 or 2".into() }));
            row("firmware says", s.firmware.clone().unwrap_or("—".into()));
            row("identity", s.identity.clone().unwrap_or("—".into()));
            row("build", s.build.clone().unwrap_or("—".into()));
            row("sound", s.sound.clone().unwrap_or("—".into()));
            row("key lights", s.lights.map_or("none yet".into(), |l| format!("{l:?}")));
            row("last screen", age(s.last));
            row("screen since", age(s.screen_since));
        };
        unit(1, &self.port, true, &self.shared, self.link.as_ref());
        for (i, u) in self.others.iter().enumerate() {
            unit(i + 2, &u.port, false, &u.shared, u.link.as_ref());
        }
        let mut app = |k: &str, val: String| v.push((format!("App · {k}"), val));
        app("side by side", self.side_by_side.to_string());
        app("locked", self.locked.to_string());
        app("look", self.look.name().to_string());
        app("smooth", self.smooth_on.to_string());
        app("mirror shown", self.mirror_on.to_string());
        app("Big Knobs", self.knobs_on.to_string());
        app("Key Lights strip", self.lights_on.to_string());
        app("spyglass", self.glass_on.to_string());
        app("keyboard", if self.keys_choice.is_empty() { "any".into() } else { self.keys_choice.clone() });
        app("Mellotron sound", format!("{:?}", self.mellotron_sound));
        app("Shuttle knob", format!("{}", self.sh_knob));
        v
    }

    pub fn debug_window(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::D)) {
            self.dbg.open = !self.dbg.open;
            debug::set_on(self.dbg.open, ctx);
        }
        if !self.dbg.open {
            return;
        }
        let builder = egui::ViewportBuilder::default().with_title("FM-1 Big Screen · Debugger").with_inner_size([1180.0, 820.0]).with_icon(crate::app_icon());
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("debugger"), builder, |ctx, _| {
            if ctx.input(|i| i.viewport().close_requested()) {
                self.dbg.open = false;
                debug::set_on(false, ctx);
                return;
            }
            self.debug_ui(ctx);
        });
    }

    fn debug_ui(&mut self, ctx: &egui::Context) {
        let stop = debug::stopped();

        // a new stop: keep what the state was at the last one, to show what changed
        if let Some(s) = &stop {
            if self.dbg.shown_stop != Some(s.event.id) {
                self.dbg.shown_stop = Some(s.event.id);
                // what it was at the last stop (empty at the first)
                self.dbg.before = std::mem::take(&mut self.dbg.at_stop);
                self.dbg.selected = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
        self.dbg.now_state = self.debug_state();
        if stop.is_some() && stop.as_ref().map(|s| s.event.id) == self.dbg.shown_stop && self.dbg.at_stop.is_empty() {
            self.dbg.at_stop = self.dbg.now_state.clone();
        }
        // keys: F5 continue, F10 next message, F11 next from the FM-1, Shift+F11 next to it, F6 pause
        let (f5, f10, f11, sf11, f6) = ctx.input(|i| {
            (i.key_pressed(egui::Key::F5), i.key_pressed(egui::Key::F10), i.key_pressed(egui::Key::F11) && !i.modifiers.shift,
             i.key_pressed(egui::Key::F11) && i.modifiers.shift, i.key_pressed(egui::Key::F6))
        });
        if stop.is_some() {
            if f5 { debug::release(Step::None, false) }
            if f10 { debug::release(Step::Any, false) }
            if f11 { debug::release(Step::In, false) }
            if sf11 { debug::release(Step::Out, false) }
        } else if f6 {
            debug::pause();
        }

        egui::TopBottomPanel::top("dbg_top").show(ctx, |ui| {
            compact(ui);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                match &stop {
                    Some(s) => {
                        ui.label(RichText::new(format!("STOPPED  #{}  {}", s.event.id, s.why)).color(Y).strong().size(17.0));
                        ui.label(RichText::new(format!("{} thread · {} {} · stopped {:.1} s", s.thread, if s.event.dir == Dir::In { "from" } else { "to" }, s.event.port, s.since.elapsed().as_secs_f32())).color(GREY));
                    }
                    None => {
                        ui.label(RichText::new("RUNNING").color(Color32::from_rgb(70, 200, 120)).strong().size(17.0));
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                let on = stop.is_some();
                if ui.add_enabled(on, egui::Button::new("Continue  F5")).clicked() { debug::release(Step::None, false) }
                if ui.add_enabled(on, egui::Button::new("Next message  F10")).clicked() { debug::release(Step::Any, false) }
                if ui.add_enabled(on, egui::Button::new("Next from the FM-1  F11")).clicked() { debug::release(Step::In, false) }
                if ui.add_enabled(on, egui::Button::new("Next to the FM-1  Shift+F11")).clicked() { debug::release(Step::Out, false) }
                if let Some(s) = &stop {
                    if ui.button(format!("Next {}", s.event.kind.name())).clicked() { debug::release(Step::Kind(s.event.kind), false) }
                    if ui.button("Skip this message").on_hover_text("The app never sees it (from the FM-1) or never sends it (to the FM-1)").clicked() {
                        debug::release(Step::Any, true)
                    }
                }
                if ui.add_enabled(!on, egui::Button::new("Pause  F6")).on_hover_text("Stop at the next message, whatever it is").clicked() { debug::pause() }
            });
            if let Some(s) = &stop {
                if s.since.elapsed().as_secs_f32() > 2.5 {
                    ui.label(RichText::new("While the app is stopped it says no hello: the FM-1 stops sending its screen about 3 seconds after the last one. Continue and it starts again.").color(GREY).size(12.0));
                }
            }
            ui.add_space(4.0);
        });

        egui::SidePanel::left("dbg_bps").resizable(true).default_width(340.0).show(ctx, |ui| {
            compact(ui);
            ui.heading("Breakpoints");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Stop when the firmware says hello").clicked() { debug::with_breakpoints(|b| b.push(Breakpoint::new(What::SaysHello))) }
                if ui.button("Stop at the sound name").clicked() { debug::with_breakpoints(|b| b.push(Breakpoint::new(What::Kind(Kind::SoundName)))) }
                if ui.button("Stop at Key Lights").clicked() { debug::with_breakpoints(|b| b.push(Breakpoint::new(What::Kind(Kind::KeyLights)))) }
            });
            ui.separator();
            let mut remove = None;
            debug::with_breakpoints(|bps| {
                if bps.is_empty() {
                    ui.label(RichText::new("None yet: add one below, or click a message in the log.").color(GREY));
                }
                for (i, bp) in bps.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut bp.enabled, "");
                        dot(ui, if bp.log_only { Color32::LIGHT_BLUE } else { Color32::from_rgb(230, 70, 70) });
                        ui.label(bp.label()).on_hover_text(if bp.log_only { "Log only: marks the log, never stops" } else { "Stops the app here" });
                        ui.label(RichText::new(format!("{} hits", bp.hits)).color(GREY));
                        if ui.small_button("Remove").clicked() { remove = Some(i) }
                    });
                }
                if let Some(i) = remove { bps.remove(i); }
            });
            ui.separator();
            ui.label(RichText::new("Add a breakpoint").strong());
            let ch = choices();
            egui::ComboBox::from_id_salt("dbg_pick").width(300.0).selected_text(ch[self.dbg.pick.min(ch.len() - 1)].0.clone()).show_ui(ui, |ui| {
                for (i, (n, _)) in ch.iter().enumerate() {
                    ui.selectable_value(&mut self.dbg.pick, i, n);
                }
            });
            let bytes = matches!(ch[self.dbg.pick.min(ch.len() - 1)].1, What::Bytes(_));
            if bytes {
                ui.horizontal(|ui| { ui.label("Bytes"); ui.text_edit_singleline(&mut self.dbg.pattern).on_hover_text("Hex, ?? for any byte"); });
            }
            ui.horizontal(|ui| {
                ui.label("Way");
                ui.selectable_value(&mut self.dbg.dir, 0, "both");
                ui.selectable_value(&mut self.dbg.dir, 1, "from FM-1");
                ui.selectable_value(&mut self.dbg.dir, 2, "to FM-1");
            });
            ui.horizontal(|ui| { ui.label("Only the port with"); ui.add(egui::TextEdit::singleline(&mut self.dbg.port).desired_width(120.0)); });
            ui.horizontal(|ui| {
                ui.label("Stop after the first");
                ui.add(egui::TextEdit::singleline(&mut self.dbg.skip).desired_width(40.0));
                ui.label("hits");
            });
            ui.checkbox(&mut self.dbg.log_only, "Log only (mark the log, don't stop)");
            if ui.button("Add").clicked() {
                let what = if bytes {
                    debug::parse_pattern(&self.dbg.pattern).map(What::Bytes)
                } else {
                    Some(ch[self.dbg.pick.min(ch.len() - 1)].1.clone())
                };
                match what {
                    Some(w) => {
                        let mut bp = Breakpoint::new(w);
                        bp.dir = [None, Some(Dir::In), Some(Dir::Out)][self.dbg.dir];
                        bp.port = self.dbg.port.trim().to_string();
                        bp.skip_first = self.dbg.skip.trim().parse().unwrap_or(0);
                        bp.log_only = self.dbg.log_only;
                        debug::with_breakpoints(|b| b.push(bp));
                        self.dbg.note.clear();
                    }
                    None => self.dbg.note = "Those bytes aren't hex (e.g. F0 7D 46 53 ??)".into(),
                }
            }
            if !self.dbg.note.is_empty() {
                ui.label(RichText::new(&self.dbg.note).color(Y));
            }
            ui.separator();
            ui.collapsing("How many of each", |ui| {
                egui::Grid::new("dbg_counts").striped(true).show(ui, |ui| {
                    for (k, n) in debug::counts() {
                        ui.label(k.name());
                        ui.label(n.to_string());
                        ui.end_row();
                    }
                });
            });
        });

        // the log, at the bottom
        egui::TopBottomPanel::bottom("dbg_log").resizable(true).default_height(270.0).show(ctx, |ui| {
            compact(ui);
            ui.horizontal(|ui| {
                ui.heading("Messages");
                ui.checkbox(&mut self.dbg.hide_pictures, "Hide the screen's pictures");
                ui.checkbox(&mut self.dbg.follow, "Follow the newest");
                ui.add(egui::TextEdit::singleline(&mut self.dbg.filter).hint_text("filter: kind, port, bytes…").desired_width(200.0));
                if ui.button("Clear").clicked() { debug::clear_log() }
                if ui.button("Save the log…").clicked() {
                    let dir = dirs_home().join("Documents");
                    let f = dir.join(format!("FM-1 Big Screen debug log {}.txt", chrono_like()));
                    self.dbg.note = match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&f, debug::log_text())) {
                        Ok(()) => format!("Saved: {}", f.display()),
                        Err(e) => format!("Couldn't save: {e}"),
                    };
                }
            });
            let (hide, filt) = (self.dbg.hide_pictures, self.dbg.filter.to_lowercase());
            let rows = debug::log_snapshot(
                |e| {
                    (!hide || !e.kind.picture() || e.mark.is_some())
                        && (filt.is_empty() || e.kind.name().to_lowercase().contains(&filt) || e.port.to_lowercase().contains(&filt) || debug::hex(&e.bytes).to_lowercase().contains(&filt))
                },
                600,
            );
            let stopped_id = stop.as_ref().map(|s| s.event.id);
            let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if self.dbg.follow { scroll = scroll.stick_to_bottom(true) }
            scroll.show(ui, |ui| {
                egui::Grid::new("dbg_rows").striped(true).num_columns(6).show(ui, |ui| {
                    let mut prev_t = None;
                    for e in &rows {
                        let mono = |s: String| RichText::new(s).monospace().size(12.0);
                        let here = Some(e.id) == stopped_id;
                        let sel = self.dbg.selected.as_ref().map(|s| s.id) == Some(e.id);
                        let txt = |s: String| if here { mono(s).color(Y).strong() } else { mono(s) };
                        if ui.selectable_label(sel || here, txt(format!("#{}", e.id))).clicked() { self.dbg.selected = Some(e.clone()) }
                        ui.label(txt(format!("{:9.3}{}", e.t, prev_t.map_or(String::new(), |p: f64| format!(" +{:.0}ms", (e.t - p) * 1000.0)))));
                        ui.label(txt(format!("{} {}", if e.dir == Dir::In { "from" } else { "to  " }, short(&e.port))));
                        ui.label(txt(if e.answers_hello && !e.kind.says_hello() { format!("{} (hello answer)", e.kind.name()) } else { e.kind.name().to_string() }));
                        ui.label(txt(debug::summary(e.dir, &e.bytes)));
                        ui.label(match &e.mark { Some(m) => mono(m.clone()).color(Color32::LIGHT_BLUE), None => mono(debug::hex(&e.bytes[..e.bytes.len().min(12)])).color(GREY) });
                        ui.end_row();
                        prev_t = Some(e.t);
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            compact(ui);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // the message: the stopped one, or one picked in the log
                let (ev, full) = match (&self.dbg.selected, &stop) {
                    (Some(e), _) => (Some(e.clone()), e.bytes.clone()),
                    (None, Some(s)) => (Some(s.event.clone()), s.full.clone()),
                    _ => (None, vec![]),
                };
                match ev {
                    Some(e) => {
                        let (kind, fields) = debug::decode(e.dir, &full);
                        ui.horizontal(|ui| {
                            ui.heading(RichText::new(kind.name()).color(Y));
                            ui.label(RichText::new(format!("#{} · {} {} · {} bytes", e.id, if e.dir == Dir::In { "from" } else { "to" }, e.port, e.len)).color(GREY));
                            if ui.small_button("Stop on this kind").clicked() {
                                debug::with_breakpoints(|b| b.push(Breakpoint::new(What::Kind(kind))));
                            }
                            if ui.small_button("Stop on these bytes").clicked() {
                                let p = full.iter().take(6).map(|b| Some(*b)).collect();
                                debug::with_breakpoints(|b| b.push(Breakpoint::new(What::Bytes(p))));
                            }
                            if ui.small_button("Copy as hex").clicked() { ui.ctx().copy_text(debug::hex(&full)) }
                            if ui.small_button("Send it again…").clicked() { self.dbg.send = debug::hex(&full); self.dbg.send_to = e.port.clone(); }
                        });
                        egui::Grid::new("dbg_fields").num_columns(2).striped(true).show(ui, |ui| {
                            for (k, v) in &fields {
                                ui.label(RichText::new(k).color(GREY));
                                ui.label(RichText::new(v).monospace());
                                ui.end_row();
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(debug::dump(&full[..full.len().min(512)])).monospace().size(12.0));
                        if e.len > full.len() || full.len() > 512 {
                            ui.label(RichText::new(format!("(the first {} of {} bytes)", full.len().min(512), e.len)).color(GREY));
                        }
                    }
                    None => {
                        ui.heading("Nothing stopped");
                        ui.label("Add a breakpoint on the left (\"When the firmware says hello\" is a good first one), or press Pause (F6) to stop at the next message. Click any message in the log to see it decoded.");
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("The app's state");
                    if stop.is_some() && !self.dbg.before.is_empty() {
                        ui.label(RichText::new("yellow: changed since the last stop").color(Y).size(12.0));
                    }
                });
                egui::Grid::new("dbg_state").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in &self.dbg.now_state {
                        let ticking = k.ends_with("linked") || k.ends_with("last screen") || k.ends_with("screen since");
                        let changed = stop.is_some() && !ticking && self.dbg.before.iter().find(|(bk, _)| bk == k).is_some_and(|(_, bv)| bv != v);
                        ui.label(RichText::new(k).color(GREY));
                        let t = RichText::new(v).monospace();
                        ui.label(if changed { t.color(Y).strong() } else { t });
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.heading("Send bytes to an FM-1");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.dbg.send).desired_width(380.0).font(egui::TextStyle::Monospace));
                    let ports: Vec<String> = std::iter::once(self.port.clone()).chain(self.others.iter().map(|u| u.port.clone())).collect();
                    if self.dbg.send_to.is_empty() || !ports.contains(&self.dbg.send_to) { self.dbg.send_to = self.port.clone() }
                    egui::ComboBox::from_id_salt("dbg_to").selected_text(short(&self.dbg.send_to)).show_ui(ui, |ui| {
                        for p in &ports { ui.selectable_value(&mut self.dbg.send_to, p.clone(), p); }
                    });
                    if ui.button("Send").clicked() {
                        let bytes: Option<Vec<u8>> = debug::parse_pattern(&self.dbg.send).and_then(|p| p.into_iter().collect());
                        let link = if self.dbg.send_to == self.port { self.link.as_ref() } else { self.others.iter().find(|u| u.port == self.dbg.send_to).and_then(|u| u.link.as_ref()) };
                        self.dbg.note = match (bytes, link) {
                            (Some(b), Some(l)) => {
                                debug::check(&l.name, Dir::Out, &b, "debugger", false);
                                match l.out.lock().unwrap().send(&b) { Ok(()) => format!("Sent {} bytes", b.len()), Err(e) => format!("Not sent: {e}") }
                            }
                            (None, _) => "Those aren't hex bytes".into(),
                            (_, None) => "That FM-1 isn't linked".into(),
                        };
                    }
                });
                ui.label(RichText::new("e.g. F0 7D 46 4C 01 F7 asks which firmware · F0 7D 46 53 07 0A 01 F7 presses PLAY").color(GREY).size(12.0));
            });
        });
    }
}

/// This window's own size of type (the app's is big, for reading the FM-1; this window is for detail). Only these
/// panels: the FM-1's window keeps its big type.
fn compact(ui: &mut egui::Ui) {
    use egui::{FontFamily, FontId, TextStyle};
    ui.style_mut().text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();
    ui.style_mut().spacing.button_padding = egui::vec2(6.0, 3.0);
    ui.style_mut().spacing.interact_size.y = 20.0;
}

fn dot(ui: &mut egui::Ui, c: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, c);
}

fn short(port: &str) -> String {
    if port.chars().count() > 22 { port.chars().take(21).collect::<String>() + "…" } else { port.to_string() }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(Into::into).unwrap_or_else(|| ".".into())
}

/// A file-name-safe time: seconds since 1970 (no extra crate).
fn chrono_like() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or("now".into(), |d| d.as_secs().to_string())
}
