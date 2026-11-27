//! cargo run --example shuttle_probe: prints the Contour Shuttle's raw HID reports (to learn its layout).
fn main() {
    let api = hidapi::HidApi::new().expect("hidapi");
    for d in api.device_list() {
        if d.vendor_id() == 0x0B33 {
            println!("found {:04x}:{:04x} {:?} usage_page={:#x} usage={:#x}", d.vendor_id(), d.product_id(), d.product_string(), d.usage_page(), d.usage());
        }
    }
    let d = api.device_list().find(|d| d.vendor_id() == 0x0B33).expect("no Shuttle");
    let dev = d.open_device(&api).expect("open");
    let mut buf = [0u8; 16];
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 90 {
        if let Ok(n) = dev.read_timeout(&mut buf, 200) {
            if n > 0 { println!("{:6.2}s {:02x?}", start.elapsed().as_secs_f32(), &buf[..n]); }
        }
    }
}
