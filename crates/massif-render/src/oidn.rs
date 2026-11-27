//! Intel Open Image Denoise, the denoiser Blender's Cycles uses, loaded
//! when the app runs: the library comes with Massif's builds, beside the
//! executable (in the app's Frameworks on macOS); `MASSIF_OIDN_DIR` names
//! another folder, and on a Mac Blender's own copy serves when it is
//! installed. Without it, the renderer keeps its own filter on the graphics
//! card (see `denoise.wgsl`).
//!
//! Only the few calls of OIDN's public C API (Apache-2.0) the denoising of
//! an image takes are bound here: a device, the "RT" filter with the
//! colour, albedo and normal images, run once.

use libloading::{Library, Symbol};
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type Handle = *mut c_void;

/// OIDN_FORMAT_FLOAT3.
const FLOAT3: i32 = 3;
/// OIDN_DEVICE_TYPE_DEFAULT: the best the computer has (Metal on a Mac,
/// a GPU elsewhere when it has one, else the processor).
const DEVICE_DEFAULT: i32 = 0;
/// OIDN_QUALITY_HIGH, as Blender's final renders.
const QUALITY_HIGH: i32 = 6;

/// The library's file, by platform.
#[cfg(target_os = "macos")]
const FILE: &str = "libOpenImageDenoise.dylib";
#[cfg(target_os = "windows")]
const FILE: &str = "OpenImageDenoise.dll";
#[cfg(all(unix, not(target_os = "macos")))]
const FILE: &str = "libOpenImageDenoise.so.2";

struct Api {
    _lib: Library,
    device: Handle,
    new_filter: unsafe extern "C" fn(Handle, *const c_char) -> Handle,
    new_buffer: unsafe extern "C" fn(Handle, usize) -> Handle,
    write_buffer: unsafe extern "C" fn(Handle, usize, usize, *const c_void),
    read_buffer: unsafe extern "C" fn(Handle, usize, usize, *mut c_void),
    release_buffer: unsafe extern "C" fn(Handle),
    set_image: unsafe extern "C" fn(Handle, *const c_char, Handle, i32, usize, usize, usize, usize, usize),
    set_bool: unsafe extern "C" fn(Handle, *const c_char, bool),
    set_int: unsafe extern "C" fn(Handle, *const c_char, i32),
    commit_filter: unsafe extern "C" fn(Handle),
    execute: unsafe extern "C" fn(Handle),
    release_filter: unsafe extern "C" fn(Handle),
    error: unsafe extern "C" fn(Handle, *mut *const c_char) -> i32,
}

// SAFETY: OIDN's devices may be used from any thread; the calls here are
// made one at a time (behind the mutex of `api`).
unsafe impl Send for Api {}

/// Where the library may be, in order.
fn places() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("MASSIF_OIDN_DIR") {
        out.push(PathBuf::from(d));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.to_owned());
        out.push(dir.join("../Frameworks"));
        out.push(dir.join("lib"));
        out.push(dir.join("../lib"));
    }
    if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/Applications/Blender.app/Contents/Resources/lib"));
    }
    out
}

impl Api {
    fn load(dir: &Path) -> Result<Api, String> {
        let path = dir.join(FILE);
        if !path.exists() {
            return Err(format!("no {}", path.display()));
        }
        // SAFETY: loading OIDN runs its initialisers, which do nothing
        // but register its devices; the symbols are those of its C API,
        // with the signatures of OpenImageDenoise/oidn.h (2.x).
        unsafe {
            let lib = Library::new(&path).map_err(|e| e.to_string())?;
            macro_rules! sym {
                ($name:literal) => {{
                    let s: Symbol<_> = lib.get(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $name))?;
                    *s
                }};
            }
            let new_device: unsafe extern "C" fn(i32) -> Handle = sym!("oidnNewDevice");
            let commit_device: unsafe extern "C" fn(Handle) = sym!("oidnCommitDevice");
            let error: unsafe extern "C" fn(Handle, *mut *const c_char) -> i32 = sym!("oidnGetDeviceError");
            let api = Api {
                device: std::ptr::null_mut(),
                new_filter: sym!("oidnNewFilter"),
                new_buffer: sym!("oidnNewBuffer"),
                write_buffer: sym!("oidnWriteBuffer"),
                read_buffer: sym!("oidnReadBuffer"),
                release_buffer: sym!("oidnReleaseBuffer"),
                set_image: sym!("oidnSetFilterImage"),
                set_bool: sym!("oidnSetFilterBool"),
                set_int: sym!("oidnSetFilterInt"),
                commit_filter: sym!("oidnCommitFilter"),
                execute: sym!("oidnExecuteFilter"),
                release_filter: sym!("oidnReleaseFilter"),
                error,
                _lib: lib,
            };
            let device = new_device(DEVICE_DEFAULT);
            if device.is_null() {
                return Err("no device".into());
            }
            commit_device(device);
            let api = Api { device, ..api };
            api.check()?;
            Ok(api)
        }
    }

    /// The device's last error, if any.
    fn check(&self) -> Result<(), String> {
        let mut message: *const c_char = std::ptr::null();
        // SAFETY: the device is alive; the message, when set, is a C string
        // OIDN keeps until the next call.
        let code = unsafe { (self.error)(self.device, &mut message) };
        if code == 0 {
            return Ok(());
        }
        let text = if message.is_null() { String::new() } else { unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned() };
        Err(format!("OIDN error {code}: {text}"))
    }
}

fn api() -> &'static Option<Mutex<Api>> {
    static API: OnceLock<Option<Mutex<Api>>> = OnceLock::new();
    API.get_or_init(|| {
        let mut why = Vec::new();
        for dir in places() {
            match Api::load(&dir) {
                Ok(api) => {
                    log::info!("Open Image Denoise from {}", dir.display());
                    return Some(Mutex::new(api));
                }
                Err(e) => why.push(e),
            }
        }
        log::info!("no Open Image Denoise: {}", why.join("; "));
        None
    })
}

/// Whether OIDN can be used here.
pub fn available() -> bool {
    api().is_some()
}

/// Denoise `color` (linear light, three floats a pixel, `width` ×
/// `height`), helped by the albedo and the normals the rays first met (the
/// same layout, or none), in place. Err: OIDN is missing or failed.
pub fn denoise(color: &mut [f32], albedo: Option<&[f32]>, normal: Option<&[f32]>, width: usize, height: usize) -> Result<(), String> {
    let api = api().as_ref().ok_or("Open Image Denoise is not installed")?;
    let api = api.lock().map_err(|_| "OIDN poisoned")?;
    let bytes = width * height * 12;
    if color.len() * 4 != bytes || albedo.is_some_and(|a| a.len() * 4 != bytes) || normal.is_some_and(|n| n.len() * 4 != bytes) {
        return Err("images of the wrong size".into());
    }
    let name = |s: &str| CString::new(s).expect("a name");
    // SAFETY: the buffers are as large as the images set on the filter;
    // everything made here is released before returning.
    unsafe {
        let mut buffers = Vec::new();
        let mut image = |filter: Handle, key: &str, data: &[f32]| {
            let b = (api.new_buffer)(api.device, bytes);
            (api.write_buffer)(b, 0, bytes, data.as_ptr().cast());
            (api.set_image)(filter, name(key).as_ptr(), b, FLOAT3, width, height, 0, 0, 0);
            buffers.push(b);
            b
        };
        let filter = (api.new_filter)(api.device, name("RT").as_ptr());
        let input = image(filter, "color", color);
        if let Some(a) = albedo {
            image(filter, "albedo", a);
            if let Some(n) = normal {
                image(filter, "normal", n);
            }
        }
        let output = (api.new_buffer)(api.device, bytes);
        buffers.push(output);
        (api.set_image)(filter, name("output").as_ptr(), output, FLOAT3, width, height, 0, 0, 0);
        let _ = input;
        (api.set_bool)(filter, name("hdr").as_ptr(), true);
        (api.set_int)(filter, name("quality").as_ptr(), QUALITY_HIGH);
        (api.commit_filter)(filter);
        (api.execute)(filter);
        let result = api.check();
        if result.is_ok() {
            (api.read_buffer)(output, 0, bytes, color.as_mut_ptr().cast());
        }
        (api.release_filter)(filter);
        for b in buffers {
            (api.release_buffer)(b);
        }
        result.and_then(|()| api.check())
    }
}
