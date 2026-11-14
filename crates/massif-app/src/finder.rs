//! Files the Finder asks Massif to open: a double-click on a `.massif` map,
//! Open With on a WorldPainter `.world`, a file dropped on the Dock icon.
//!
//! AppKit hands them to the application's delegate, as
//! `application:openURLs:`, which winit's delegate does not answer. So every
//! object is taught to answer it — only the delegate is ever asked — and the
//! paths wait here until the app takes them, which it does each frame. On
//! other systems files come on the command line only, and nothing waits.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static OPENED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Start listening, before the application runs.
#[cfg(target_os = "macos")]
pub fn listen() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2_foundation::{NSArray, NSURL};

    unsafe extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *const NSArray<NSURL>) {
        // SAFETY: AppKit passes the array of the URLs to open.
        let Some(urls) = (unsafe { urls.as_ref() }) else { return };
        if let Ok(mut opened) = OPENED.lock() {
            opened.extend(urls.iter().filter_map(|url| url.path()).map(|p| PathBuf::from(p.to_string())));
        }
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }

    let Some(object) = AnyClass::get(c"NSObject") else { return };
    // SAFETY: the method has the signature of `application:openURLs:`
    // (void, self, _cmd, two objects), as its type string says.
    unsafe {
        let imp: Imp = std::mem::transmute(open_urls as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *const NSArray<NSURL>));
        objc2::ffi::class_addMethod(object as *const AnyClass as *mut AnyClass, objc2::sel!(application:openURLs:), imp, c"v@:@@".as_ptr());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn listen() {}

/// The app's context, to wake it when a file comes.
pub fn wake(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// The files asked for since the last call.
pub fn take() -> Vec<PathBuf> {
    OPENED.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
}
