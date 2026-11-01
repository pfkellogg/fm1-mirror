// Runs the app's installer from a terminal and prints how it goes (for testing on a real FM-1).
//   cargo run --example install_cli [--stop-at-updater]
// --stop-at-updater quits once the FM-1 is waiting in its updater, as if the install had been
// cut off there; running it again (or FM-1 Mirror) must then finish the update.
#[path = "../src/screen.rs"]
#[allow(dead_code)]
mod screen;
#[path = "../src/install.rs"]
#[allow(dead_code)]
mod install;

use std::sync::{Arc, Mutex};

fn main() {
    let stop = std::env::args().any(|a| a == "--stop-at-updater");
    let progress: Arc<Mutex<install::Progress>> = Arc::default();
    let p = progress.clone();
    std::thread::spawn(move || install::install(p));
    let mut said = String::new();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let p = progress.lock().unwrap().clone();
        let now = format!("{} {}%", p.step, p.percent / 10 * 10);
        if now != said {
            println!("{now}");
            said = now;
        }
        if stop && p.step.starts_with("Waiting for the FM-1's updater") {
            println!("stopped here on purpose");
            return;
        }
        if let Some(e) = p.error {
            println!("ERROR: {e}");
            std::process::exit(1);
        }
        if p.done {
            println!("done");
            return;
        }
    }
}
