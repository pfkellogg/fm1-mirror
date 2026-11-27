//! The Contour ShuttleXpress (and ShuttlePRO) as a remote for FM-1 Big Screen: turn the FM-1's knobs with a dial
//! instead of a mouse, pick the FM-1, the lock, Side by side. Everything it does is said out loud and shown big.
//!
//! Its HID report (5 bytes, read with hidapi, no driver needed): [ring, dial, 0, buttons 1-4, button 5]
//! - ring: the spring-loaded outer ring, -7..7 (two's complement), 0 at rest;
//! - dial: the inner jog dial, a counter that goes up turning right and down turning left (wraps at 256);
//! - buttons: 0x10, 0x20, 0x40, 0x80 in byte 3 = buttons 1..4, 0x01 in byte 4 = button 5 (left to right).
//!   (The ShuttlePRO's extra buttons are in the same two bytes; only the first five are used here.)
//!
//! The layout (user's choice, 2026-11-27): dial = 1 notch, 1 click of the chosen knob; ring = the same knob, fast
//! (further = faster); 1 previous knob (hold: say where I am), 2 next knob (hold: speech on/off), 3 SEL (pressed as
//! long as it's held), 4 next FM-1 (hold: Side by side / Tabs), 5 the lock.
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ev {
    /// The inner dial turned: + right (clockwise), - left, in notches.
    Dial(i32),
    /// The outer ring's position, -7..7 (0 = let go).
    Ring(i8),
    /// Button 1..5 pressed (true) or let go (false).
    Button(u8, bool),
}

/// Contour Design's USB vendor id; the ShuttleXpress is 0x0020, the ShuttlePRO 0x0010, the ShuttlePRO v2 0x0030.
const CONTOUR: u16 = 0x0B33;

pub struct Shuttle {
    rx: Receiver<Ev>,
    connected: Arc<AtomicBool>,
}

impl Shuttle {
    /// Looks for a Shuttle now and every 2 s (plug it in any time); reads it in its own thread.
    pub fn start(wake: impl Fn() + Send + 'static) -> Shuttle {
        let (tx, rx) = channel();
        let connected = Arc::new(AtomicBool::new(false));
        let c = connected.clone();
        std::thread::Builder::new().name("shuttle".into()).spawn(move || run(tx, c, wake)).ok();
        Shuttle { rx, connected }
    }
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
    /// The events since last time.
    pub fn events(&self) -> Vec<Ev> {
        self.rx.try_iter().collect()
    }
}

fn run(tx: Sender<Ev>, connected: Arc<AtomicBool>, wake: impl Fn()) {
    let Ok(mut api) = hidapi::HidApi::new() else { return };
    loop {
        let _ = api.refresh_devices();
        let found = api.device_list().find(|d| d.vendor_id() == CONTOUR).map(|d| d.open_device(&api));
        if let Some(Ok(dev)) = found {
            connected.store(true, Ordering::Relaxed);
            wake();
            let mut dec = Decoder::default();
            let mut buf = [0u8; 16];
            loop {
                match dev.read_timeout(&mut buf, 250) {
                    Ok(0) => continue,
                    Ok(n) => {
                        let evs = dec.feed(&buf[..n]);
                        if !evs.is_empty() {
                            for e in evs {
                                if tx.send(e).is_err() {
                                    return; // the app is gone
                                }
                            }
                            wake();
                        }
                    }
                    Err(_) => break, // unplugged
                }
            }
            connected.store(false, Ordering::Relaxed);
            wake();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Turns reports into events: the dial's steps, the ring's moves, the buttons' presses and lets go.
#[derive(Default)]
pub struct Decoder {
    dial: Option<u8>,
    ring: i8,
    buttons: u8,
}

impl Decoder {
    pub fn feed(&mut self, r: &[u8]) -> Vec<Ev> {
        let mut out = vec![];
        if r.len() < 5 {
            return out;
        }
        let ring = (r[0] as i8).clamp(-7, 7);
        if ring != self.ring {
            self.ring = ring;
            out.push(Ev::Ring(ring));
        }
        let dial = r[1];
        if let Some(prev) = self.dial {
            let d = dial.wrapping_sub(prev) as i8 as i32;
            // A real turn moves a few notches per report; a big jump is the counter resetting (it can, as the ring
            // springs back): no turn, just a new starting point.
            if d != 0 && d.abs() <= 6 {
                out.push(Ev::Dial(d));
            }
        }
        self.dial = Some(dial);
        let b = (r[3] >> 4) | ((r[4] & 1) << 4);
        for i in 0..5 {
            let (was, is) = (self.buttons >> i & 1, b >> i & 1);
            if was != is {
                out.push(Ev::Button(i + 1, is == 1));
            }
        }
        self.buttons = b;
        out
    }
}

/// Says `text` out loud (the system's voice), cutting off whatever it was saying.
pub fn say(text: &str) {
    static NOW: Mutex<Option<Child>> = Mutex::new(None);
    let mut now = NOW.lock().unwrap();
    if let Some(mut c) = now.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    let text = text.replace(['"', '\''], "");
    #[cfg(target_os = "macos")]
    let child = Command::new("say").args(["-r", "215", &text]).spawn();
    #[cfg(target_os = "windows")]
    let child = Command::new("powershell").args(["-NoProfile", "-Command",
        &format!("Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('{text}')")]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let child = Command::new("spd-say").args(["-r", "20", &text]).spawn();
    *now = child.ok();
}

/// How often the ring turns the knob, and by how much, at each of its 7 positions.
pub fn ring_speed(pos: i8) -> (Duration, i32) {
    let a = pos.unsigned_abs().min(7) as usize;
    const EVERY: [u64; 8] = [0, 320, 220, 150, 110, 80, 60, 45];
    const CLICKS: [i32; 8] = [0, 1, 1, 1, 1, 2, 3, 4];
    (Duration::from_millis(EVERY[a]), CLICKS[a] * pos.signum() as i32)
}

/// A press that lasts this long is a hold (its second job).
pub const HOLD: Duration = Duration::from_millis(600);

/// Holds the five buttons' press times, to tell a tap from a hold.
#[derive(Default)]
pub struct Presses {
    since: [Option<Instant>; 5],
    pub held_done: [bool; 5],
}

impl Presses {
    pub fn down(&mut self, b: u8) {
        if let Some(s) = self.since.get_mut(b as usize - 1) {
            *s = Some(Instant::now());
        }
        self.held_done[b as usize - 1] = false;
    }
    /// On letting go: was it a tap (and not already handled as a hold)?
    pub fn up(&mut self, b: u8) -> bool {
        let i = b as usize - 1;
        let tap = self.since[i].take().is_some_and(|s| s.elapsed() < HOLD) && !self.held_done[i];
        tap
    }
    /// Buttons held long enough, once each (to act while still held).
    pub fn holds(&mut self) -> Vec<u8> {
        let mut v = vec![];
        for i in 0..5 {
            if let Some(s) = self.since[i] {
                if s.elapsed() >= HOLD && !self.held_done[i] {
                    self.held_done[i] = true;
                    v.push(i as u8 + 1);
                }
            }
        }
        v
    }
    pub fn any_down(&self) -> bool {
        self.since.iter().any(|s| s.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_the_shuttlexpress() {
        let mut d = Decoder::default();
        assert_eq!(d.feed(&[0, 0x0a, 0, 0, 0]), vec![]);                       // the dial's first position
        assert_eq!(d.feed(&[0, 0x0b, 0, 0, 0]), vec![Ev::Dial(1)]);             // right
        assert_eq!(d.feed(&[0, 0x09, 0, 0, 0]), vec![Ev::Dial(-2)]);            // left
        assert_eq!(d.feed(&[0, 0xff, 0, 0, 0]), vec![]);                       // a jump: no turn
        assert_eq!(d.feed(&[0, 0x01, 0, 0, 0]), vec![Ev::Dial(2)]);             // across the wrap
        assert_eq!(d.feed(&[0x05, 0x01, 0, 0, 0]), vec![Ev::Ring(5)]);
        assert_eq!(d.feed(&[0xf9, 0x01, 0, 0, 0]), vec![Ev::Ring(-7)]);
        assert_eq!(d.feed(&[0, 0x01, 0, 0x10, 0]), vec![Ev::Ring(0), Ev::Button(1, true)]);
        assert_eq!(d.feed(&[0, 0x01, 0, 0, 0x01]), vec![Ev::Button(1, false), Ev::Button(5, true)]);
        assert_eq!(d.feed(&[0, 0x01, 0, 0x80, 0]), vec![Ev::Button(4, true), Ev::Button(5, false)]);
    }
    #[test]
    fn the_ring_goes_faster_further() {
        assert_eq!(ring_speed(0).1, 0);
        assert_eq!(ring_speed(1), (Duration::from_millis(320), 1));
        assert_eq!(ring_speed(-7), (Duration::from_millis(45), -4));
    }
}
