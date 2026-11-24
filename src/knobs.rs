//! Big Knobs and Key Lights, for players who can't see the FM-1's knobs and lights well (fingers cover the lit keys).
//!
//! Big Screen firmware made at aikellogg.com/#/make-big-screen understands, on top of the screen (screen.rs):
//! - Big Knobs, to the FM-1: `F0 7D 'F' 'S' 04 e n F7` turns encoder e (0 SELECT, 1 ALGORITHM, 2..5 KNOB 1..4,
//!   6 PRESETS) n clicks (7-bit signed: 1..63 clockwise, 127..65 = -1..-63), as if turned by hand;
//!   `F0 7D 'F' 'S' 06 lo hi F7` sets MASTER (0..1023) until its real knob moves;
//! - Remote panel, to the FM-1: `F0 7D 'F' 'S' 07 id down F7` presses (1) or lets go (0) button id (0..13) or key
//!   14 + k (F3 = 14), as a finger would (holds and combinations work); all let go when we stop asking;
//! - Key Lights, from the FM-1 while we ask for the screen: `F0 7D 'F' 'S' 05 k0 k1 k2 k3 b0 b1 [g0..g3 h0 h1] F7`, the
//!   lit keys (bit k = key k, F3 .. G5, 7 bits a byte) and buttons (bit i = button i, 0..13), then (from firmware that
//!   has them) the ones only glowing (dim, breathing, mid-level) the same way.
use eframe::egui::{self, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use std::time::{Duration, Instant};

pub const NAMES: [&str; 8] = ["SELECT", "ALGORITHM", "KNOB 1", "KNOB 2", "KNOB 3", "KNOB 4", "PRESETS", "MASTER"];
/// The buttons by id (0..13), as the FM-1's panel prints them.
pub const BUTTONS: [&str; 14] = ["OCT-", "OCT+", "FX", "SEL", "ENV", "LFO", "EDIT", "GLO", "HOME", "SAVE", "ARP", "SEQ", "PLAY", "REC"];
/// The order the buttons are drawn in (left to right).
const BUTTON_ROW: [usize; 14] = [0, 1, 8, 6, 3, 4, 5, 2, 7, 10, 11, 9, 13, 12];
const MASTER: usize = 7;
/// Points of drag for one click of an encoder.
const DRAG_PER_CLICK: f32 = 12.0;

/// Turn encoder `e` by `clicks` (clamped to ±63).
pub fn turn(e: u8, clicks: i32) -> [u8; 8] {
    let n = clicks.clamp(-63, 63);
    [0xF0, 0x7D, b'F', b'S', 4, e & 7, (n & 127) as u8, 0xF7]
}

/// Press (`down`) or let go of button or key `id` (0..13 buttons, 14..40 keys).
pub fn press(id: u8, down: bool) -> [u8; 8] {
    [0xF0, 0x7D, b'F', b'S', 7, id.min(40), down as u8, 0xF7]
}

/// MASTER to `v` (0..1023).
pub fn master(v: u16) -> [u8; 8] {
    let v = v.min(1023);
    [0xF0, 0x7D, b'F', b'S', 6, (v & 127) as u8, (v >> 7) as u8, 0xF7]
}

/// The FM-1's lights: lit and glowing, keys (27 bits) and buttons (14 bits).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Lights {
    pub keys: u32,
    pub btns: u16,
    pub glow_keys: u32,
    pub glow_btns: u16,
}

/// The lights, if this is Key Lights (with or without the glows).
pub fn lights(m: &[u8]) -> Option<Lights> {
    if !(m.len() == 12 || m.len() == 18) || m[..5] != [0xF0, 0x7D, b'F', b'S', 5] || m[m.len() - 1] != 0xF7 {
        return None;
    }
    let k = |at: usize| ((m[at] as u32 & 127) | (m[at + 1] as u32 & 127) << 7 | (m[at + 2] as u32 & 127) << 14
        | (m[at + 3] as u32 & 127) << 21) & ((1 << 27) - 1);
    let b = |at: usize| ((m[at] as u16 & 127) | (m[at + 1] as u16 & 127) << 7) & 0x3FFF;
    let mut l = Lights { keys: k(5), btns: b(9), ..Default::default() };
    if m.len() == 18 {
        l.glow_keys = k(11);
        l.glow_btns = b(15);
    }
    Some(l)
}

/// What the knobs show, and drags not yet a whole click.
pub struct Knobs {
    left: [f32; 8],
    angle: [f32; 8],
    pub master: f32,
    master_sent: Option<(u16, Instant)>,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs { left: [0.0; 8], angle: [0.0; 8], master: 512.0, master_sent: None }
    }
}

/// The eight knobs in a row; `send` gets each message for the FM-1.
pub fn knobs_ui(ui: &mut egui::Ui, k: &mut Knobs, send: &mut dyn FnMut(&[u8])) {
    let area = ui.available_rect_before_wrap();
    let n = NAMES.len() as f32;
    let cell = (area.width() / n).min(area.height());
    let r = (cell * 0.36).max(14.0);
    let x0 = area.center().x - cell * n / 2.0;
    for i in 0..NAMES.len() {
        let c = Pos2::new(x0 + cell * (i as f32 + 0.5), area.top() + r + 6.0);
        let hit = Rect::from_center_size(c, vec2(cell, cell));
        let resp = ui.interact(hit, ui.id().with(("knob", i)), Sense::drag()).on_hover_text(if i == MASTER {
            "Drag up or right (or scroll) to turn MASTER up. The FM-1's own MASTER knob takes over when you move it."
        } else {
            "Drag up or right (or scroll) to turn it clockwise, down or left the other way: one click every few points, as the real knob."
        });
        let mut d = resp.drag_delta().x - resp.drag_delta().y;
        if resp.hovered() {
            d += ui.input(|inp| inp.smooth_scroll_delta.y) * 0.5;
        }
        if i == MASTER {
            if d != 0.0 {
                k.master = (k.master + d * 2.5).clamp(0.0, 1023.0);
            }
            let v = k.master.round() as u16;
            let due = k.master_sent.is_none_or(|(sent, at)| sent != v && at.elapsed() >= Duration::from_millis(30));
            if due && (d != 0.0 || k.master_sent.is_some_and(|(s, _)| s != v)) {
                send(&master(v));
                k.master_sent = Some((v, Instant::now()));
            }
            k.angle[i] = (k.master / 1023.0 - 0.5) * 270f32.to_radians();
        } else if d != 0.0 {
            k.left[i] += d;
            let clicks = (k.left[i] / DRAG_PER_CLICK).trunc() as i32;
            if clicks != 0 {
                k.left[i] -= clicks as f32 * DRAG_PER_CLICK;
                send(&turn(i as u8, clicks));
                k.angle[i] += clicks as f32 * 18f32.to_radians();
            }
        }
        let p = ui.painter();
        let lit = resp.hovered() || resp.dragged();
        p.circle_filled(c, r, Color32::from_gray(if lit { 70 } else { 52 }));
        p.circle_stroke(c, r, Stroke::new(2.0_f32, Color32::from_gray(110)));
        let a = k.angle[i] - std::f32::consts::FRAC_PI_2;
        p.line_segment([c + vec2(a.cos(), a.sin()) * r * 0.25, c + vec2(a.cos(), a.sin()) * r * 0.9],
            Stroke::new((r * 0.12).max(3.0), Color32::from_rgb(255, 200, 90)));
        p.text(Pos2::new(c.x, c.y + r + 6.0), Align2::CENTER_TOP, NAMES[i], FontId::proportional((r * 0.32).clamp(11.0, 18.0)),
            Color32::from_gray(200));
    }
    ui.advance_cursor_after_rect(Rect::from_min_size(area.min, vec2(area.width(), (r * 2.0 + 34.0).min(area.height()))));
}

/// The 27 keys (bit k = key k, F3 .. G5) the notes being played fall on: notes outside the FM-1's keys folded in by
/// octaves, so every note shows where its name is.
pub fn played_keys(notes: &[u8]) -> u32 {
    let mut m = 0;
    for &n in notes {
        let mut n = n as i32;
        while n < 53 {
            n += 12;
        }
        while n > 79 {
            n -= 12;
        }
        m |= 1 << (n - 53);
    }
    m
}

/// The FM-1's buttons and its 27 keys (F3 .. G5), big: lit (amber) as they are on the FM-1, and blue while a note on
/// them is being played (the FM-1's own keys, or a keyboard through Keys). `seen`: Key Lights have come.
/// `held`: the buttons and keys the pointer holds down now (bit id); `press` gets each change (id, down), so a click
/// or a hold on the strip presses the real one.
pub fn lights_ui(ui: &mut egui::Ui, l: Lights, played: u32, seen: bool, held: &mut u64, press: &mut dyn FnMut(u8, bool)) {
    let mut now_held = 0u64;
    let (keys, btns) = (l.keys, l.btns);
    let area = ui.available_rect_before_wrap();
    let p = ui.painter();
    let glow = Color32::from_rgb(255, 214, 120);
    let soft_white = Color32::from_rgb(245, 222, 170); // a white key glowing (dim)
    let soft_black = Color32::from_rgb(120, 85, 30); // a black key glowing
    let soft_btn = Color32::from_rgb(105, 85, 45);
    // the buttons in one row
    let bh = (area.height() * 0.22).clamp(18.0, 40.0);
    let bw = area.width() / BUTTON_ROW.len() as f32;
    for (j, &id) in BUTTON_ROW.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(area.left() + bw * j as f32, area.top()), vec2(bw, bh)).shrink(3.0);
        if ui.interact(r, ui.id().with(("btn", id)), Sense::click_and_drag()).is_pointer_button_down_on() {
            now_held |= 1 << id;
        }
        let on = btns >> id & 1 == 1 || now_held >> id & 1 == 1;
        let dim = l.glow_btns >> id & 1 == 1;
        p.rect_filled(r, 6.0, if on { glow } else if dim { soft_btn } else { Color32::from_gray(45) });
        p.text(r.center(), Align2::CENTER_CENTER, BUTTONS[id], FontId::proportional((bh * 0.42).clamp(10.0, 17.0)),
            if on { Color32::BLACK } else { Color32::from_gray(170) });
    }
    // the keys: 16 white (F G A B C D E F G A B C D E F G), 11 black
    let top = area.top() + bh + 6.0;
    let kh = (area.bottom() - top).max(30.0);
    let ww = area.width() / 16.0;
    let mut white = Vec::new();
    let mut black = Vec::new();
    let (mut w, note) = (0usize, 5u32); // F3: 5 semitones above C
    for k in 0..27u32 {
        let pc = (note + k) % 12;
        if matches!(pc, 1 | 3 | 6 | 8 | 10) {
            black.push((k, w));
        } else {
            white.push((k, w));
            w += 1;
        }
    }
    // black keys first, so a press on one isn't also the white key under it
    let black_rect = |i: usize| Rect::from_min_size(Pos2::new(area.left() + ww * i as f32 - ww * 0.32, top), vec2(ww * 0.64, kh * 0.6));
    let pointer = ui.input(|inp| inp.pointer.interact_pos());
    let on_black = pointer.is_some_and(|pt| black.iter().any(|&(_, i)| black_rect(i).contains(pt)));
    for &(k, i) in &black {
        if ui.interact(black_rect(i), ui.id().with(("key", k)), Sense::click_and_drag()).is_pointer_button_down_on() {
            now_held |= 1 << (14 + k);
        }
    }
    for &(k, i) in &white {
        let r = Rect::from_min_size(Pos2::new(area.left() + ww * i as f32, top), vec2(ww, kh)).shrink2(vec2(1.5, 0.0));
        if !on_black && ui.interact(r, ui.id().with(("key", k)), Sense::click_and_drag()).is_pointer_button_down_on() {
            now_held |= 1 << (14 + k);
        }
        let p = ui.painter();
        let on = keys >> k & 1 == 1 || now_held >> (14 + k) & 1 == 1;
        let play = played >> k & 1 == 1;
        let dim = l.glow_keys >> k & 1 == 1;
        p.rect_filled(r, 4.0, if on { glow } else if play { Color32::from_rgb(110, 175, 255) } else if dim { soft_white } else { Color32::from_gray(200) });
        if on && play {
            p.rect_stroke(r.shrink(2.0), 4.0, Stroke::new(4.0_f32, Color32::from_rgb(40, 110, 230)), egui::StrokeKind::Inside);
        }
    }
    let p = ui.painter();
    for &(k, i) in &black {
        let r = black_rect(i);
        let on = keys >> k & 1 == 1 || now_held >> (14 + k) & 1 == 1;
        let play = played >> k & 1 == 1;
        let dim = l.glow_keys >> k & 1 == 1;
        p.rect_filled(r, 3.0, if on { Color32::from_rgb(255, 170, 40) } else if play { Color32::from_rgb(60, 130, 240) } else if dim { soft_black } else { Color32::from_gray(25) });
        if on && play {
            p.rect_stroke(r.shrink(1.5), 3.0, Stroke::new(3.0_f32, Color32::from_rgb(40, 110, 230)), egui::StrokeKind::Inside);
        }
    }
    let _ = seen; // (an FM-1 without lights: the app says so in the strip's place, main.rs stale)
    for id in 0..41u8 {
        let (was, is) = (*held >> id & 1 == 1, now_held >> id & 1 == 1);
        if was != is {
            press(id, is);
        }
    }
    *held = now_held;
    ui.advance_cursor_after_rect(Rect::from_min_max(area.min, Pos2::new(area.right(), top + kh)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns() {
        assert_eq!(turn(2, 1), [0xF0, 0x7D, b'F', b'S', 4, 2, 1, 0xF7]);
        assert_eq!(turn(6, -1), [0xF0, 0x7D, b'F', b'S', 4, 6, 127, 0xF7]);
        assert_eq!(turn(0, -200)[6], 65);
        assert_eq!(master(1023), [0xF0, 0x7D, b'F', b'S', 6, 127, 7, 0xF7]);
    }

    #[test]
    fn reads_lights() {
        let m = [0xF0, 0x7D, b'F', b'S', 5, 1, 0, 0, 0x20, 0x01, 0x40, 0xF7];
        assert_eq!(lights(&m), Some(Lights { keys: 1 | 1 << 26, btns: 1 | 1 << 13, ..Default::default() }));
        assert_eq!(lights(&m[..11]), None);
        let g = [0xF0, 0x7D, b'F', b'S', 5, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1, 0xF7];
        assert_eq!(lights(&g), Some(Lights { glow_keys: 2, glow_btns: 1 << 7, ..Default::default() }));
    }

    #[test]
    fn folds_played_notes() {
        assert_eq!(played_keys(&[53, 79]), 1 | 1 << 26);
        assert_eq!(played_keys(&[41, 91, 36]), 1 | 1 << 26 | 1 << 7); // F2 -> F3, G6 -> G5, C2 -> C4
    }
}
