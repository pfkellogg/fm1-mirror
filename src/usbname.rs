//! The FM-1's name as the FM-1 says it, on a Mac. macOS remembers a USB MIDI device by its USB IDs and the port it is
//! plugged into, and keeps the name it had the first time: after a new firmware (they share the same IDs) every app
//! still sees the old one ("ChoralRoot FM-1" for LOOM-8). Here a USB MIDI device whose name differs from the USB
//! product name the FM-1 reports now (IORegistry, matched by its USB location) gets that name again, as renaming it
//! in Audio MIDI Setup would. Only FM-1s (by their new name); elsewhere it does nothing.

/// Renames stale FM-1 USB MIDI devices; returns (old, new) for each one renamed.
#[cfg(target_os = "macos")]
pub fn fix_stale_names(is_fm1: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let usb = usb_products();
    let mut done = Vec::new();
    if usb.is_empty() {
        return done;
    }
    unsafe {
        use coremidi_sys::*;
        for i in 0..MIDIGetNumberOfDevices() {
            let d = MIDIGetDevice(i);
            if int(d, "offline") != Some(0) {
                continue;
            }
            let (Some(loc), Some(old)) = (int(d, "USBLocationID"), string(d, "name")) else { continue };
            let Some((_, product)) = usb.iter().find(|(l, _)| *l == loc as u32) else { continue };
            if old == *product || !is_fm1(product) {
                continue;
            }
            set(d, "name", product);
            set(d, "model", product);                // (Audio MIDI Setup shows it too)
            for e in 0..MIDIDeviceGetNumberOfEntities(d) {
                let en = MIDIDeviceGetEntity(d, e);
                if string(en, "name").as_deref() == Some(old.as_str()) {
                    set(en, "name", product);
                }
                let ends = (0..MIDIEntityGetNumberOfSources(en)).map(|s| MIDIEntityGetSource(en, s))
                    .chain((0..MIDIEntityGetNumberOfDestinations(en)).map(|s| MIDIEntityGetDestination(en, s)));
                for p in ends.collect::<Vec<_>>() {
                    if string(p, "name").as_deref() == Some(old.as_str()) {
                        set(p, "name", product);
                    }
                }
            }
            done.push((old, product.clone()));
        }
    }
    done
}

#[cfg(not(target_os = "macos"))]
pub fn fix_stale_names(_is_fm1: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    Vec::new()
}

/// The USB devices plugged in now: (location, product name), from `ioreg` ("+-o LOOM-8 FM-1@02122000 <class ...").
#[cfg(target_os = "macos")]
fn usb_products() -> Vec<(u32, String)> {
    let Ok(out) = std::process::Command::new("/usr/sbin/ioreg").args(["-p", "IOUSB", "-w0"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_ioreg_line).collect()
}

fn parse_ioreg_line(line: &str) -> Option<(u32, String)> {
    let rest = &line[line.find("+-o ")? + 4..];
    let (head, _) = rest.split_once("  <class IOUSB")?;
    let (name, loc) = head.rsplit_once('@')?;
    Some((u32::from_str_radix(loc.trim(), 16).ok()?, name.to_string()))
}

#[cfg(target_os = "macos")]
fn cf(s: &str) -> core_foundation_sys::string::CFStringRef {
    let c = std::ffi::CString::new(s).unwrap();
    unsafe {
        core_foundation_sys::string::CFStringCreateWithCString(std::ptr::null(), c.as_ptr(),
            core_foundation_sys::string::kCFStringEncodingUTF8)
    }
}

#[cfg(target_os = "macos")]
unsafe fn int(o: coremidi_sys::MIDIObjectRef, key: &str) -> Option<i32> {
    let k = cf(key);
    let mut v: i32 = 0;
    let r = coremidi_sys::MIDIObjectGetIntegerProperty(o, k, &mut v);
    core_foundation_sys::base::CFRelease(k as _);
    (r == 0).then_some(v)
}

#[cfg(target_os = "macos")]
unsafe fn string(o: coremidi_sys::MIDIObjectRef, key: &str) -> Option<String> {
    use core_foundation_sys::string::*;
    let k = cf(key);
    let mut s: CFStringRef = std::ptr::null();
    let r = coremidi_sys::MIDIObjectGetStringProperty(o, k, &mut s);
    core_foundation_sys::base::CFRelease(k as _);
    if r != 0 || s.is_null() {
        return None;
    }
    let mut buf = [0i8; 256];
    let ok = CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as _, kCFStringEncodingUTF8);
    core_foundation_sys::base::CFRelease(s as _);
    (ok != 0).then(|| std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
unsafe fn set(o: coremidi_sys::MIDIObjectRef, key: &str, value: &str) {
    let (k, v) = (cf(key), cf(value));
    coremidi_sys::MIDIObjectSetStringProperty(o, k, v);
    core_foundation_sys::base::CFRelease(k as _);
    core_foundation_sys::base::CFRelease(v as _);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_ioreg_line() {
        let l = "  | +-o LOOM-8 FM-1@02122000  <class IOUSBHostDevice, id 0x10009cce9, registered, matched, active>";
        assert_eq!(parse_ioreg_line(l), Some((0x02122000, "LOOM-8 FM-1".to_string())));
        assert_eq!(parse_ioreg_line("+-o Root  <class IORegistryEntry, id 0x100000100>"), None);
    }
}
