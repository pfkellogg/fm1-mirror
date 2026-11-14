//! macOS: clicking FM-1 Mirror's Dock icon brings back its minimized window.
//!
//! With the floating spyglass showing, the app still has a visible window, so macOS doesn't
//! bring back the minimized main window on a Dock click by itself: only the spyglass showed.
//! winit's application delegate doesn't answer `applicationShouldHandleReopen:hasVisibleWindows:`,
//! so we add that method to it: it un-minimizes every minimized window of the app.

use std::ffi::{c_char, c_void};

type Id = *mut c_void;
type Sel = *const c_void;
#[cfg(target_arch = "aarch64")]
type Bool = bool;
#[cfg(target_arch = "aarch64")]
const YES: Bool = true;
#[cfg(not(target_arch = "aarch64"))]
type Bool = i8;
#[cfg(not(target_arch = "aarch64"))]
const YES: Bool = 1;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_addMethod(cls: Id, name: Sel, imp: *const c_void, types: *const c_char) -> Bool;
    fn objc_msgSend();
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

unsafe fn sel(name: &[u8]) -> Sel {
    sel_registerName(name.as_ptr() as *const c_char)
}

extern "C" fn should_handle_reopen(_this: Id, _cmd: Sel, app: Id, _has_visible: Bool) -> Bool {
    unsafe {
        let send_id: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        let send_usize: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const c_void);
        let send_at: extern "C" fn(Id, Sel, usize) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        let send_bool: extern "C" fn(Id, Sel) -> Bool = std::mem::transmute(objc_msgSend as *const c_void);
        let send_with: extern "C" fn(Id, Sel, Id) = std::mem::transmute(objc_msgSend as *const c_void);

        let windows = send_id(app, sel(b"windows\0"));
        let count = send_usize(windows, sel(b"count\0"));
        for i in 0..count {
            let w = send_at(windows, sel(b"objectAtIndex:\0"), i);
            if send_bool(w, sel(b"isMiniaturized\0")) == YES {
                send_with(w, sel(b"deminiaturize:\0"), std::ptr::null_mut());
            }
        }
    }
    YES
}

/// Call once the event loop is running (winit's delegate class exists by then).
pub fn install() {
    unsafe {
        let cls = objc_getClass(b"WinitApplicationDelegate\0".as_ptr() as *const c_char);
        if cls.is_null() {
            return;
        }
        // c = BOOL (x86_64), B = bool (arm64); @ = id, : = SEL
        #[cfg(target_arch = "aarch64")]
        let types = b"B@:@B\0";
        #[cfg(not(target_arch = "aarch64"))]
        let types = b"c@:@c\0";
        class_addMethod(
            cls,
            sel(b"applicationShouldHandleReopen:hasVisibleWindows:\0"),
            should_handle_reopen as *const c_void,
            types.as_ptr() as *const c_char,
        );
    }
}
