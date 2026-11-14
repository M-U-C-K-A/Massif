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

/// On Windows: make `.massif` files Massif's for the current user — a
/// double-click opens them, and Explorer shows their map as their icon
/// through the thumbnail library next to `Massif.exe` (see
/// `massif-thumbnails`). Done each time Massif starts, so that moving its
/// folder moves the registration too; nothing is written when it is
/// already right.
#[cfg(windows)]
pub fn register() {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW};
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(folder) = exe.parent() else { return };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let get = |key: &str, name: Option<&str>| -> Option<String> {
        let (k, n) = (wide(key), name.map(wide));
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: the buffers outlive the call; `len` is their size in bytes.
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        (err == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
    };
    let mut changed = false;
    let mut set = |key: &str, name: Option<&str>, value: &str| {
        if get(key, name).as_deref() == Some(value) {
            return;
        }
        let (k, n, v) = (wide(key), name.map(wide), wide(value));
        // SAFETY: the strings are NUL-terminated and outlive the call.
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) };
        changed = true;
    };
    let exe = exe.display().to_string();
    set(r"Software\Classes\.massif", None, "Massif.Map");
    set(r"Software\Classes\.massif", Some("Content Type"), "application/x-massif");
    set(r"Software\Classes\Massif.Map", None, "Massif map");
    set(r"Software\Classes\Massif.Map\DefaultIcon", None, &format!("\"{exe}\",0"));
    set(r"Software\Classes\Massif.Map\shell\open\command", None, &format!("\"{exe}\" \"%1\""));
    if let Some(library) = [massif_thumbnails::FILE_NAME, "massif_thumbnails.dll"].iter().map(|n| folder.join(n)).find(|p| p.is_file()) {
        let class = massif_thumbnails::CLSID;
        set(&format!(r"Software\Classes\CLSID\{class}"), None, "Massif map thumbnails");
        set(&format!(r"Software\Classes\CLSID\{class}\InprocServer32"), None, &library.display().to_string());
        set(&format!(r"Software\Classes\CLSID\{class}\InprocServer32"), Some("ThreadingModel"), "Apartment");
        // The shell's own key for thumbnail handlers.
        set(r"Software\Classes\.massif\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}", None, class);
    }
    if changed {
        // SAFETY: no items are passed.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    }
}

#[cfg(not(windows))]
pub fn register() {}

/// The app's context, to wake it when a file comes.
pub fn wake(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// The files asked for since the last call.
pub fn take() -> Vec<PathBuf> {
    OPENED.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
}
