//! What Massif does as it starts, written down step by step in a log of its
//! own (`startup.log`, the one before kept as `startup.previous.log`) with
//! every panic in it, and a watch on the first frame: a start that hangs or
//! fails says where, on the user's machine, without any telemetry.
//!
//! eframe keeps the window hidden until its first frame is drawn (so that
//! it does not flash white). When that frame never comes — the graphics
//! card or its driver stuck, the app stuck before it —, the process runs
//! with no window to see. On Windows the watch shows the window after a few
//! seconds anyway, and after longer tells the user where the log is; a
//! failure to start or a panic is said in a message box, since a Windows
//! build has no console to print it to.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "startup_windows.rs"]
mod windows;

/// After how long without a first frame the window is shown anyway, and
/// after how long the user is told.
const SHOW_AFTER: Duration = Duration::from_secs(3);
const TELL_AFTER: Duration = Duration::from_secs(20);

struct Log {
    file: Option<std::fs::File>,
    started: Instant,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// The folder of the log: local app data on Windows (`%LOCALAPPDATA%\Massif`),
/// the logs on macOS (`~/Library/Logs/Massif`), the state folder elsewhere
/// (`~/.local/state/massif`).
pub fn log_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
    if cfg!(windows) {
        var("LOCALAPPDATA").map(|p| p.join("Massif"))
    } else if cfg!(target_os = "macos") {
        std::env::home_dir().map(|h| h.join("Library").join("Logs").join("Massif"))
    } else {
        var("XDG_STATE_HOME").or_else(|| std::env::home_dir().map(|h| h.join(".local").join("state"))).map(|p| p.join("massif"))
    }
}

pub fn log_path() -> Option<PathBuf> {
    log_dir().map(|d| d.join("startup.log"))
}

/// Begin: the log (the last one kept aside), the panic hook, the watch on
/// the first frame.
pub fn begin() {
    let file = log_path().and_then(|path| {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let _ = std::fs::rename(&path, path.with_file_name("startup.previous.log"));
        std::fs::File::create(&path).ok()
    });
    let _ = LOG.set(Mutex::new(Log { file, started: Instant::now() }));
    step(&format!("Massif {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if !args.is_empty() {
        step(&format!("asked to open {}", args.join(" ")));
    }
    // A panic: written down, printed as before, and on the main thread said
    // to the user.
    let printed = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_owned();
        step(&format!("panic in thread {name}: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        printed(info);
        if name == "main" {
            tell(&format!("Massif stopped on an error:\n\n{}{}", message(info), where_logged()));
        }
    }));
    let _ = std::thread::Builder::new().name("massif startup watch".into()).spawn(watch);
}

/// What a panic said, and where.
fn message(info: &std::panic::PanicHookInfo) -> String {
    let payload = info.payload();
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "an error with no message".to_owned());
    match info.location() {
        Some(at) => format!("{text}\n({}:{})", at.file(), at.line()),
        None => text,
    }
}

/// The end of a message: where the log is.
fn where_logged() -> String {
    log_path().map(|p| format!("\n\nWhat Massif did as it started is written in:\n{}", p.display())).unwrap_or_default()
}

/// Write down a step, with the time since the start.
pub fn step(what: &str) {
    let Some(log) = LOG.get() else { return };
    let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
    let ms = log.started.elapsed().as_millis();
    if let Some(file) = &mut log.file {
        let _ = writeln!(file, "[{ms:>7} ms] {what}");
        let _ = file.flush();
    }
}

/// The first frame is being drawn: the start is over.
pub fn first_frame() {
    if !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        step("first frame: started");
    }
}

/// The start failed: written down, and said in a message box.
pub fn failed(error: &dyn std::fmt::Display) {
    step(&format!("could not start: {error}"));
    let graphics = if cfg!(windows) {
        "DirectX 12, Vulkan or OpenGL"
    } else if cfg!(target_os = "macos") {
        "Metal"
    } else {
        "Vulkan or OpenGL"
    };
    tell(&format!("Massif could not start:\n\n{error}\n\nIt needs a graphics card that {graphics} can use: updating its driver often helps.{}", where_logged()));
}

/// A message box of the system, waited on.
pub fn tell(text: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Massif")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Show Massif's window if it is still hidden; how many were.
#[cfg(windows)]
fn show_window() -> usize {
    windows::show_hidden_windows("Massif")
}

#[cfg(not(windows))]
fn show_window() -> usize {
    0
}

/// Watch for the first frame, writing down while it does not come. On
/// Windows, the window is shown after a few seconds anyway, and after
/// longer the user is told where the log is (elsewhere a message box needs
/// the main thread, which is the one stuck).
fn watch() {
    std::thread::sleep(SHOW_AFTER);
    if FIRST_FRAME.load(Ordering::Relaxed) {
        return;
    }
    step(&format!("no first frame after {} s", SHOW_AFTER.as_secs()));
    let shown = show_window();
    if shown > 0 {
        step("showed the window, still hidden");
    }
    std::thread::sleep(TELL_AFTER - SHOW_AFTER);
    if FIRST_FRAME.load(Ordering::Relaxed) {
        return;
    }
    step(&format!("no first frame after {} s", TELL_AFTER.as_secs()));
    if cfg!(windows) {
        tell(&format!(
            "Massif has not drawn its window after {} seconds: it may be stuck on the graphics card or its driver.{}",
            TELL_AFTER.as_secs(),
            where_logged()
        ));
    }
}
