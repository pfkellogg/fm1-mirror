// Sends one hello to the FM-1 with midir and counts the screen messages that come back.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
fn main() {
    let mut input = midir::MidiInput::new("hello-test").unwrap();
    input.ignore(midir::Ignore::None);
    let output = midir::MidiOutput::new("hello-test").unwrap();
    let ip = input.ports().into_iter().find(|p| input.port_name(p).unwrap().contains("FM1")).unwrap();
    let op = output.ports().into_iter().find(|p| output.port_name(p).unwrap().contains("FM1")).unwrap();
    let n = Arc::new(AtomicUsize::new(0));
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (n2, s2) = (n.clone(), sizes.clone());
    let _c = input.connect(&ip, "in", move |_, m, _| { n2.fetch_add(1, Ordering::SeqCst); s2.lock().unwrap().push(m.len()); }, ()).unwrap();
    let mut out = output.connect(&op, "out").unwrap();
    let every: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(0);
    for k in 0..5 {
        if k == 0 || every > 0 { out.send(&[0xf0, 0x7d, b'F', b'S', 0, 0xf7]).unwrap(); }
        std::thread::sleep(std::time::Duration::from_millis(if every > 0 { every } else { 900 }));
        println!("after {} ms: {}", (k + 1) * 900, n.load(Ordering::SeqCst));
    }
    let s = sizes.lock().unwrap();
    println!("received {} messages; first sizes {:?}; max {:?}", n.load(Ordering::SeqCst), &s[..s.len().min(8)], s.iter().max());
}
