//! What Massif does as it starts, written down step by step in a log of its
//! own (`startup.log`, the one before kept as `startup.previous.log`) with
//! every panic and crash in it, and a watch on the first frame: a start
//! that hangs or fails says where, on the user's machine, without any
//! telemetry.
//!
//! The first frames are followed through every step, until one is drawn on
//! the window's image and presented (eframe shows the window after it):
//! the interface laid out (each part of it), its shapes and textures sent
//! to the graphics card, the image the window gives to draw on, the frame
//! presented. Each step is Massif's own work or the graphics card's
//! ([`Stage`]). A start that stays too long in a step of the graphics card
//! is stuck in its driver: the graphics API it draws with (Vulkan, DirectX
//! 12, OpenGL) is set aside on this computer (`graphics.txt` beside the
//! log), and the next starts let wgpu choose among the others. On Windows
//! the user is asked to start again at once. A start that stays long in its
//! own work is only written down: another graphics API would not help it
//! (0.5.1 set every API aside for a first frame held up in its own work,
//! and those lines are left aside). A start that died without a word in a
//! step of the graphics card (a crash in a driver) is found by the next
//! one, from the mark it left (`first-frame.pending`), and its API set
//! aside in the same way.
//!
//! The main thread never waits on the disk for any of this: the lines go
//! to a thread of their own that writes them (a panic or a crash writes
//! what is left at once). The watch goes on after the start, for as long
//! as Massif runs: a frame that keeps the main thread longer than a few
//! seconds is written down with where it is (on Windows, the main thread's
//! stack, by module and offset), so that a window Windows calls "not
//! responding" says why in the log.
//!
//! eframe keeps the window hidden until its first frame is drawn (so that
//! it does not flash white); on Windows it is shown before, as drawing in a
//! window never shown is what some drivers handle worst. A failure to start
//! or a panic is said in a message box, since a Windows build has no
//! console to print it to; a crash on Windows is written down with its
//! exception and the module it came from.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "startup_windows.rs"]
mod windows;

/// After how long without a frame shown the watch writes where the start is.
const NOTE_AFTER: Duration = Duration::from_secs(3);
/// After how long in one step, before a frame is shown, the start is stuck.
const STUCK_AFTER: Duration = Duration::from_secs(12);
/// The frames whose every step is written down.
const FRAMES_LOGGED: u32 = 3;
/// After how long the main thread, held in one step of the start or in one
/// frame, is written down again with where it is.
const HELD_NOTES: [Duration; 4] = [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(60), Duration::from_secs(180)];

struct Log {
    started: Instant,
    /// The last step of the start, when it was written, and whose work it is.
    last: (Instant, String, Stage),
}

/// Whose work a step of the start is: what it means when the start stays
/// in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Massif's own work, on the processor: reading its settings, making
    /// the app, laying out the interface. Another graphics API would not
    /// make it go faster.
    App,
    /// The graphics card's driver: opening the device, taking the frame's
    /// shapes and textures, giving the window's image, presenting it.
    Graphics,
}

impl Stage {
    fn word(self) -> &'static str {
        match self {
            Stage::App => "app",
            Stage::Graphics => "graphics",
        }
    }

    fn from_word(word: &str) -> Option<Stage> {
        [Stage::App, Stage::Graphics].into_iter().find(|s| s.word() == word.trim())
    }
}

/// What the watch does about a start that has not shown a frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verdict {
    /// Not long enough to say.
    Wait,
    /// Long in Massif's own work: written down, and watched on.
    SlowWork,
    /// Stuck in the graphics card's driver: its API set aside.
    StuckInDriver,
}

/// What the watch makes of a start in a step of `stage` for `waited`.
fn verdict(stage: Stage, waited: Duration) -> Verdict {
    match stage {
        _ if waited < STUCK_AFTER => Verdict::Wait,
        Stage::App => Verdict::SlowWork,
        Stage::Graphics => Verdict::StuckInDriver,
    }
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// What goes to the disk: a line of the log, or the mark of the start
/// (`None`: taken away).
enum Out {
    Line(String),
    Mark(Option<String>),
}

/// The lines on their way to the disk, and the log they go to: written by
/// a thread of their own, so that a slow disk (or a file being scanned)
/// never holds up the thread that writes a line down.
struct Disk {
    /// The log, and the mark of the start.
    log: Option<PathBuf>,
    pending: Option<PathBuf>,
    queue: Mutex<Vec<Out>>,
    ready: Condvar,
    /// The log, once opened (the one before kept aside first): `None`
    /// until then, `Some(None)` when it could not be.
    file: Mutex<Option<Option<std::fs::File>>>,
}

static DISK: OnceLock<Disk> = OnceLock::new();

impl Disk {
    fn at(log: Option<PathBuf>, pending: Option<PathBuf>) -> Disk {
        Disk { log, pending, queue: Mutex::default(), ready: Condvar::new(), file: Mutex::default() }
    }

    fn send(&self, out: Out) {
        lock(&self.queue).push(out);
        self.ready.notify_one();
    }

    /// Write what waits, on this thread (`file`: the log, held).
    fn write_out(&self, file: &mut Option<Option<std::fs::File>>, queue: Vec<Out>) {
        let file = file.get_or_insert_with(|| self.log.as_deref().and_then(open_log));
        for out in queue {
            match out {
                Out::Line(line) => {
                    if let Some(file) = file {
                        let _ = file.write_all(line.as_bytes());
                    }
                }
                Out::Mark(Some(text)) => {
                    if let Some(path) = &self.pending {
                        let _ = std::fs::write(path, text);
                    }
                }
                Out::Mark(None) => {
                    if let Some(path) = &self.pending {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
    }

    /// The thread that writes: what is sent, as it comes.
    fn run(&self) {
        loop {
            let queue = {
                let mut queue = lock(&self.queue);
                while queue.is_empty() {
                    queue = self.ready.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
                std::mem::take(&mut *queue)
            };
            self.write_out(&mut lock(&self.file), queue);
        }
    }

    /// Write what waits now, on this thread: before a message box, the end
    /// of the process, a crash.
    fn flush(&self) {
        let mut file = lock(&self.file);
        let queue = std::mem::take(&mut *lock(&self.queue));
        self.write_out(&mut file, queue);
    }
}

/// Open the log, the one before kept as `startup.previous.log`.
fn open_log(path: &std::path::Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(path.parent()?).ok()?;
    let _ = std::fs::rename(path, path.with_file_name("startup.previous.log"));
    std::fs::File::create(path).ok()
}

/// Write down at once what waits to be written: before a message box, or
/// the end of the process.
pub fn flush() {
    if let Some(disk) = DISK.get() {
        disk.flush();
    }
}
/// A frame was drawn on the window's image and presented: the start is over.
static SHOWN: AtomicBool = AtomicBool::new(false);
/// The last frame drawn on an image the window gave.
static DRAWN: AtomicU32 = AtomicU32::new(0);
/// The graphics API and card Massif draws with, once wgpu chose them.
static GRAPHICS: OnceLock<Graphics> = OnceLock::new();
/// The watch set this start's graphics API aside, the start being stuck.
static SET_ASIDE: AtomicBool = AtomicBool::new(false);

struct Graphics {
    api: wgpu::Backend,
    /// The card and its driver.
    card: String,
}

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

/// The graphics APIs set aside on this computer, a line each: its name as
/// `WGPU_BACKEND` has it (`vulkan`, `dx12`, `gl`, `metal`), then why.
fn set_aside_path() -> Option<PathBuf> {
    log_dir().map(|d| d.join("graphics.txt"))
}

/// The mark of a start that has not shown a frame yet: its graphics API,
/// its process and its card.
fn pending_path() -> Option<PathBuf> {
    log_dir().map(|d| d.join("first-frame.pending"))
}

/// Begin: the log (the last one kept aside), the panic hook, the crash
/// filter, the last start looked at, the watch on the first frame.
pub fn begin() {
    let now = Instant::now();
    let _ = LOG.set(Mutex::new(Log { started: now, last: (now, String::new(), Stage::App) }));
    let disk = DISK.get_or_init(|| Disk::at(log_path(), pending_path()));
    if std::thread::Builder::new().name("massif startup log".into()).spawn(|| disk.run()).is_err() {
        // No thread: written on the way.
        disk.flush();
    }
    #[cfg(windows)]
    windows::remember_main_thread();
    step(&format!("Massif {} on {} {}", crate::update::RUNNING, std::env::consts::OS, std::env::consts::ARCH));
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
        write(&format!("panic in thread {name}: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        flush();
        printed(info);
        LAST_PANIC.set(Some(message(info)));
        // A panic `catch` stops is told by the app, the map still open.
        if name == "main" && !CATCHING.get() {
            tell(&tr!("Massif stopped on an error:\n\n{message}{where_logged}", message = message(info), where_logged = where_logged()));
        }
    }));
    // A crash (in a driver, say) ends the process with no panic.
    #[cfg(windows)]
    windows::catch_crashes();
    last_start();
    let _ = std::thread::Builder::new().name("massif startup watch".into()).spawn(watch);
}

thread_local! {
    /// Whether this thread runs inside [`catch`].
    static CATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// What the last panic on this thread said, and where.
    static LAST_PANIC: std::cell::Cell<Option<String>> = const { std::cell::Cell::new(None) };
}

/// Run `f`, stopping a panic in it: `Err` holds what the panic said and
/// where. The panic is written to the log as any other, but not told in a
/// message box: the caller tells it, with the map still open.
pub fn catch<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    let was = CATCHING.replace(true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CATCHING.set(was);
    result.map_err(|payload| {
        let said = LAST_PANIC.take();
        said.or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "an error with no message".to_owned())
    })
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
    log_path().map(|p| tr!("\n\nWhat Massif did as it started is written in:\n{p}", p = p.display())).unwrap_or_default()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Write a line, with the time since the start; a step of the start
/// (`progress`, with whose work it is) is what the watch measures the time
/// spent in.
fn put(log: &mut Log, what: &str, progress: Option<Stage>) {
    let ms = log.started.elapsed().as_millis();
    if let Some(disk) = DISK.get() {
        disk.send(Out::Line(format!("[{ms:>7} ms] {what}\n")));
    }
    if let Some(stage) = progress {
        log.last = (Instant::now(), what.to_owned(), stage);
    }
}

/// The stage the mark of the start (`first-frame.pending`) says it is in.
static MARKED: AtomicU8 = AtomicU8::new(u8::MAX);

/// Write down a step of the start that is Massif's own work, with the time
/// since it began.
pub fn step(what: &str) {
    step_in(what, Stage::App);
}

/// Write down a step of the start, and whose work it is.
pub fn step_in(what: &str, stage: Stage) {
    if let Some(log) = LOG.get() {
        put(&mut lock(log), what, Some(stage));
    }
    // The mark of the start says where it is, for the next start to know
    // whether the graphics card is to blame if this one dies.
    if GRAPHICS.get().is_some() && !SHOWN.load(Ordering::SeqCst) && MARKED.swap(stage as u8, Ordering::SeqCst) != stage as u8 {
        mark(stage);
    }
}

/// A step of a frame of the start that is Massif's own work: a part of the
/// interface laid out. Written down only for the first frames, until one
/// is shown, so that a start held up names the part.
pub fn frame_step(ctx: &egui::Context, what: &str) {
    // A performance recording times each part (see `report`).
    crate::perf::mark(what);
    if SHOWN.load(Ordering::Relaxed) || LOG.get().is_none() {
        return;
    }
    let n = frame_nr(ctx);
    if n <= FRAMES_LOGGED && ctx.current_pass_index() == 0 {
        step(&format!("frame {n}: {what}"));
    }
}

/// Write down what a thread of the start's did, and how long it took: not
/// a step the main thread is in.
pub fn note(what: &str) {
    write(what);
}

/// The warnings and errors of Massif's crates (a tile that does not read,
/// an edit with no step to undo it) in the log, with the start's steps.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        meta.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            write(&format!("{}: {}", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

/// Send the crates' warnings and errors to the log (see [`Logger`]).
pub fn log_warnings() {
    static LOGGER: Logger = Logger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// Run `f`, a step of the start that is Massif's own work, and write down
/// how long it took.
pub fn timed<R>(what: &str, f: impl FnOnce() -> R) -> R {
    step(what);
    let started = Instant::now();
    let r = f();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    if LOG.get().is_some() {
        write(&format!("  {what}: done in {ms:.1} ms"));
    }
    r
}

/// Write down what is not a step of the start: what the watch sees, an
/// error, the graphics card's news.
fn write(what: &str) {
    if let Some(log) = LOG.get() {
        put(&mut lock(log), what, None);
    }
}

/// Write down from a crash, never waiting long for the log: the thread that
/// crashed may hold it.
#[cfg(windows)]
fn write_now(what: &str) {
    fn try_lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
        for _ in 0..50 {
            match mutex.try_lock() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        None
    }
    let (Some(log), Some(disk)) = (LOG.get(), DISK.get()) else { return };
    let Some(mut log) = try_lock(log) else { return };
    put(&mut log, what, None);
    drop(log);
    let Some(mut file) = try_lock(&disk.file) else { return };
    let Some(mut queue) = try_lock(&disk.queue) else { return };
    let queue = std::mem::take(&mut *queue);
    disk.write_out(&mut file, queue);
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
    tell(&tr!("Massif could not start:\n\n{error}\n\nIt needs a graphics card that {graphics} can use: updating its driver often helps.{where_logged}", where_logged = where_logged(), error, graphics));
}

/// A message box of the system, waited on.
pub fn tell(text: &str) {
    flush();
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Massif")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

// ---------------------------------------------------------------------------
// The graphics APIs

/// The graphics APIs wgpu may draw with here, as it prefers them.
fn apis() -> &'static [wgpu::Backend] {
    use wgpu::Backend::{Dx12, Gl, Metal, Vulkan};
    if cfg!(windows) {
        &[Vulkan, Dx12, Gl]
    } else if cfg!(target_os = "macos") {
        &[Metal]
    } else {
        &[Vulkan, Gl]
    }
}

fn api_name(api: wgpu::Backend) -> &'static str {
    match api {
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::Metal => "Metal",
        other => other.to_str(),
    }
}

/// What Massif 0.5.1 wrote in `graphics.txt` when its first frame did not
/// come in time, wherever the start was held up: its interface (Massif's
/// own work) as much as the driver. Those lines are left aside.
const OLD_WATCH: &str = ": the first frame did not come";

/// Take out of `graphics.txt` the APIs Massif 0.5.1 set aside for a first
/// frame that did not come in time (see [`OLD_WATCH`]): they are tried
/// again. Returns those taken out.
fn forget_old_watch(path: &std::path::Path) -> Vec<wgpu::Backend> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let (old, kept): (Vec<&str>, Vec<&str>) = text.lines().partition(|line| line.trim_end().ends_with(OLD_WATCH) && !parse_set_aside(line).is_empty());
    if old.is_empty() {
        return Vec::new();
    }
    if parse_set_aside(&kept.join("\n")).is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        let _ = std::fs::write(path, kept.iter().map(|line| format!("{line}\n")).collect::<String>());
    }
    parse_set_aside(&old.join("\n"))
}

/// The APIs named in `graphics.txt`.
fn set_aside_apis() -> Vec<wgpu::Backend> {
    let text = set_aside_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    parse_set_aside(&text)
}

fn parse_set_aside(text: &str) -> Vec<wgpu::Backend> {
    text.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|word| apis().iter().copied().find(|api| api.to_str() == word))
        .collect()
}

/// Set `api` aside on this computer, saying why.
fn set_aside(api: wgpu::Backend, why: &str) {
    if let Some(path) = set_aside_path() {
        set_aside_in(&path, api, why);
    }
}

fn set_aside_in(path: &std::path::Path, api: wgpu::Backend, why: &str) {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if parse_set_aside(&text).contains(&api) {
        return;
    }
    if text.trim().is_empty() {
        text = "# The graphics APIs Massif does not draw with on this computer: the driver of each got stuck, or crashed, before the first frame was shown.\n# Delete a line, or this file, to let Massif try that API again.\n".to_owned();
    }
    text.push_str(&format!("{} {why}\n", api.to_str()));
    let _ = std::fs::write(path, text);
}

/// Take `api` back among those wgpu may choose.
fn put_back(api: wgpu::Backend) {
    if let Some(path) = set_aside_path() {
        put_back_in(&path, api);
    }
}

fn put_back_in(path: &std::path::Path, api: wgpu::Backend) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let kept: String = text.lines().filter(|line| line.split_whitespace().next() != Some(api.to_str())).map(|line| format!("{line}\n")).collect();
    if parse_set_aside(&kept).is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        let _ = std::fs::write(path, kept);
    }
}

/// The first API wgpu would try here among those not set aside.
fn next_api() -> &'static str {
    let aside = set_aside_apis();
    apis().iter().find(|api| !aside.contains(api)).map(|api| api_name(*api)).unwrap_or("another graphics API")
}

/// The graphics APIs wgpu may choose from: those `WGPU_BACKEND` names (in
/// quotes or not), or else all but those set aside on this computer.
pub fn backends() -> wgpu::Backends {
    let all = wgpu::Backends::PRIMARY | wgpu::Backends::GL;
    if let Some(asked) = std::env::var_os("WGPU_BACKEND") {
        let asked = asked.to_string_lossy();
        let named = named_backends(&asked);
        if !named.is_empty() {
            step(&format!("graphics API asked for: WGPU_BACKEND={asked}"));
            return named;
        }
        step(&format!("WGPU_BACKEND={asked} names no graphics API (vulkan, dx12, gl or metal): left aside"));
    }
    if let Some(path) = set_aside_path() {
        let back = forget_old_watch(&path);
        if !back.is_empty() {
            let names = back.iter().map(|api| api_name(*api)).collect::<Vec<_>>().join(", ");
            step(&format!("{names} tried again: Massif 0.5.1 set them aside for a first frame that was slow in its own work, not in the graphics driver"));
        }
    }
    let aside = set_aside_apis();
    if aside.is_empty() {
        return all;
    }
    let names = aside.iter().map(|api| api_name(*api)).collect::<Vec<_>>().join(", ");
    let left: Vec<&str> = apis().iter().filter(|api| !aside.contains(api)).map(|api| api_name(*api)).collect();
    if left.is_empty() {
        // Nothing else to try: all of them again.
        step(&format!("{names} set aside on this computer, and no other graphics API to try: trying them again"));
        if let Some(path) = set_aside_path() {
            let _ = std::fs::remove_file(path);
        }
        return all;
    }
    step(&format!("{names} set aside on this computer (in graphics.txt): drawing with {}", left.join(" or ")));
    let mut allowed = all;
    for api in aside {
        allowed.remove(api.into());
    }
    allowed
}

/// The graphics APIs a `WGPU_BACKEND` value names: `dx12`, `"dx12"`,
/// `vulkan, gl`…
fn named_backends(value: &str) -> wgpu::Backends {
    let bare: String = value.chars().filter(|c| !matches!(c, '"' | '\'')).collect();
    wgpu::Backends::from_comma_list(bare.trim())
}

/// A start that left its mark without showing a frame (it died, or was
/// ended while stuck) had its graphics API fail: set it aside.
fn last_start() {
    let Some(path) = pending_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let mut lines = text.lines();
    let api = lines.next().and_then(|word| apis().iter().copied().find(|api| api.to_str() == word.trim()));
    let process = lines.next().and_then(|p| p.trim().parse::<u32>().ok());
    let card = lines.next().unwrap_or_default().to_owned();
    // Where it was: a mark without (0.5.1's) cannot tell.
    let stage = lines.next().and_then(Stage::from_word);
    if let Some(process) = process.filter(|&p| p != std::process::id() && running(p)) {
        step(&format!("another Massif is starting (process {process}): its start is left to it"));
        return;
    }
    let _ = std::fs::remove_file(&path);
    let Some(api) = api else { return };
    if stage == Some(Stage::Graphics) {
        set_aside(api, &format!("{card}: a start ended in the graphics driver before its first frame"));
        step(&format!("the last start ended in the graphics driver before its first frame, drawing with {} on {card}: {} set aside", api_name(api), api_name(api)));
    } else {
        step(&format!("the last start ended before its first frame, drawing with {} on {card}, but not in the graphics driver: no graphics API set aside", api_name(api)));
    }
}

/// Leave the mark of a start without a frame: its API, process, card and
/// stage.
fn mark(stage: Stage) {
    let (Some(disk), Some(graphics)) = (DISK.get(), GRAPHICS.get()) else { return };
    disk.send(Out::Mark(Some(format!("{}\n{}\n{}\n{}\n", graphics.api.to_str(), std::process::id(), graphics.card, stage.word()))));
}

/// Whether Massif runs as process `process`.
#[cfg(windows)]
fn running(process: u32) -> bool {
    windows::running(process)
}

#[cfg(unix)]
fn running(process: u32) -> bool {
    // Safety: signal 0 only asks whether the process is there.
    unsafe { libc::kill(process as libc::pid_t, 0) == 0 }
}

#[cfg(not(any(windows, unix)))]
fn running(_process: u32) -> bool {
    false
}

/// wgpu chose its card and API: written down with the others it found,
/// its loss and errors watched, the mark of a start without a frame left
/// until one is shown.
pub fn graphics_ready(render: &egui_wgpu::RenderState, window: &dyn wgpu::rwh::HasWindowHandle) {
    let info = render.adapter.get_info();
    let card = format!("{} (driver {} {})", info.name, info.driver, info.driver_info);
    step_in(&format!("graphics card: {} ({:?}, driver {} {})", info.name, info.backend, info.driver, info.driver_info), Stage::Graphics);
    let others: Vec<String> = render
        .available_adapters
        .iter()
        .map(|a| a.get_info())
        .filter(|i| !(i.name == info.name && i.backend == info.backend))
        .map(|i| format!("{} ({})", i.name, api_name(i.backend)))
        .collect();
    if !others.is_empty() {
        write(&format!("also found: {}", others.join(", ")));
    }
    render.device.set_device_lost_callback(|reason, message| write(&format!("the graphics card was lost ({reason:?}): {message}")));
    // A card busy with another program does not give the window its image
    // in time: that frame is skipped and the window's surface set up again
    // on the next one. Any other error stays fatal, as wgpu makes it.
    render.device.on_uncaptured_error(Arc::new(|error| {
        if !of_the_surface(&error) {
            panic!("wgpu error: {error}\n");
        }
        let n = SURFACE_ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= 5 || n.is_multiple_of(100) {
            write(&format!("the window's surface failed, the frame skipped and the surface set up again ({n} times): {}", error.to_string().replace('\n', " ")));
        }
        RECONFIGURE.store(true, Ordering::SeqCst);
    }));
    let _ = GRAPHICS.set(Graphics { api: info.backend, card });
    MARKED.store(Stage::Graphics as u8, Ordering::SeqCst);
    mark(Stage::Graphics);
    // eframe draws the first frame in a window it keeps hidden until then,
    // a case some Windows drivers handle badly: there it is shown now.
    #[cfg(windows)]
    if let Ok(handle) = window.window_handle()
        && let wgpu::rwh::RawWindowHandle::Win32(handle) = handle.as_raw()
        && windows::show_now(handle.hwnd.get())
    {
        step_in("the window shown before its first frame", Stage::Graphics);
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// Set when setting up the window's surface or taking its image failed
/// (the card busy): the next frame sets the surface up again.
static RECONFIGURE: AtomicBool = AtomicBool::new(false);
/// How many times the window's surface failed so, for the log.
static SURFACE_ERRORS: AtomicU32 = AtomicU32::new(0);

/// Whether a wgpu error is one of the window's surface (`Surface::configure`,
/// `Surface::get_current_texture`), which a busy card gives and the next
/// frame recovers from.
fn of_the_surface(error: &wgpu::Error) -> bool {
    let of = |text: &str| ["In Surface::configure", "In Surface::get_current_texture"].iter().any(|f| text.contains(f));
    std::error::Error::source(error).is_some_and(|source| of(&source.to_string())) || of(&error.to_string())
}

/// egui's answer when the window gives no image to draw on, written down
/// while the start lasts; after the surface failed, set up again.
pub fn surface_status(answer: Arc<dyn Fn(&wgpu::CurrentSurfaceTexture) -> egui_wgpu::SurfaceErrorAction + Send + Sync>) -> Arc<dyn Fn(&wgpu::CurrentSurfaceTexture) -> egui_wgpu::SurfaceErrorAction + Send + Sync> {
    let times = AtomicU32::new(0);
    Arc::new(move |status| {
        let n = times.fetch_add(1, Ordering::Relaxed) + 1;
        // No image, no wait for it: the frame is over.
        set_busy(0, Busy::Idle);
        if !SHOWN.load(Ordering::Relaxed) && (n <= 5 || n.is_multiple_of(50)) {
            write(&format!("the window gave no image to draw on: {status:?} ({n} times)"));
        }
        // The surface failed to be set up or to give its image: set up
        // again, the frame skipped (egui asks for the next one).
        if RECONFIGURE.swap(false, Ordering::SeqCst) || matches!(status, wgpu::CurrentSurfaceTexture::Validation) {
            return egui_wgpu::SurfaceErrorAction::Reconfigure;
        }
        answer(status)
    })
}

// ---------------------------------------------------------------------------
// The first frames

/// What the main thread is in, for the watch, once the start is over: a
/// frame's layout or the window's image it waits for, or neither (waiting
/// for the system, as between frames).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Busy {
    Idle,
    LayingOut,
    WaitingForImage,
}

/// Since when the main thread is busy (milliseconds since the start, plus
/// one; 0: not busy), and with what (frame << 2 | phase).
static BUSY_SINCE: AtomicU64 = AtomicU64::new(0);
static BUSY_WHAT: AtomicU64 = AtomicU64::new(0);

fn set_busy(frame: u32, busy: Busy) {
    let since = match (busy, LOG.get()) {
        (Busy::Idle, _) | (_, None) => 0,
        (_, Some(log)) => lock(log).started.elapsed().as_millis() as u64 + 1,
    };
    BUSY_WHAT.store((frame as u64) << 2 | busy as u64, Ordering::SeqCst);
    BUSY_SINCE.store(since, Ordering::SeqCst);
}

/// What the main thread is busy with, the frame, and since when (ms).
fn busy() -> Option<(Busy, u32, u64)> {
    let since = BUSY_SINCE.load(Ordering::SeqCst);
    let what = BUSY_WHAT.load(Ordering::SeqCst);
    let busy = match what & 3 {
        1 => Busy::LayingOut,
        2 => Busy::WaitingForImage,
        _ => Busy::Idle,
    };
    (since != 0 && busy != Busy::Idle).then(|| (busy, (what >> 2) as u32, since - 1))
}

/// The number of the frame being laid out, from 1.
fn frame_nr(ctx: &egui::Context) -> u32 {
    (ctx.cumulative_frame_nr() + 1).min(u32::MAX as u64) as u32
}

/// A frame begins. Until one is shown its steps are written down, and the
/// frame before told apart: presented, or given no image by the window.
pub fn frame_begins(ctx: &egui::Context, frame: &eframe::Frame) {
    if LOG.get().is_none() {
        return;
    }
    let n = frame_nr(ctx);
    set_busy(n, Busy::LayingOut);
    if SHOWN.load(Ordering::SeqCst) {
        return;
    }
    if n > 1 && DRAWN.load(Ordering::Relaxed) == n - 1 {
        SHOWN.store(true, Ordering::SeqCst);
        step(&format!("frame {}: presented, the window shown: the start is over", n - 1));
        first_frame_shown(frame);
        return;
    }
    // One frame after the other until one is shown.
    ctx.request_repaint();
    if n <= FRAMES_LOGGED && ctx.current_pass_index() == 0 {
        step(&format!("frame {n}: laying out the interface"));
    }
}

/// The frame is laid out: a probe follows it to the graphics card.
pub fn frame_laid_out(ui: &egui::Ui) {
    if LOG.get().is_none() {
        return;
    }
    let n = frame_nr(ui.ctx());
    set_busy(n, Busy::Idle);
    if !SHOWN.load(Ordering::SeqCst) && n <= FRAMES_LOGGED {
        step_in(&format!("frame {n}: laid out; its shapes and textures go to the graphics card"), Stage::Graphics);
    }
    let corner = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(corner, Probe(n)));
}

/// A frame of the start on its way: eframe calls it once the frame's
/// shapes and textures are sent, before asking the window for an image,
/// and again as it draws on the image the window gave. It draws nothing.
struct Probe(u32);

impl egui_wgpu::CallbackTrait for Probe {
    fn prepare(&self, _: &wgpu::Device, _: &wgpu::Queue, _: &egui_wgpu::ScreenDescriptor, _: &mut wgpu::CommandEncoder, _: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        set_busy(self.0, Busy::WaitingForImage);
        if self.0 <= FRAMES_LOGGED && !SHOWN.load(Ordering::SeqCst) {
            step_in(&format!("frame {}: sent; asking the window for an image to draw on (get_current_texture)", self.0), Stage::Graphics);
        }
        Vec::new()
    }

    fn paint(&self, _: egui::PaintCallbackInfo, _: &mut wgpu::RenderPass<'static>, _: &egui_wgpu::CallbackResources) {
        DRAWN.store(self.0, Ordering::Relaxed);
        set_busy(self.0, Busy::Idle);
        if self.0 <= FRAMES_LOGGED && !SHOWN.load(Ordering::SeqCst) {
            step_in(&format!("frame {}: the window gave its image; drawing on it, then submitting and presenting it (submit, present)", self.0), Stage::Graphics);
        }
    }
}

/// The first frame is shown: the start's mark goes, its API comes back if
/// the watch had set it aside, and the graphics card says when it is done.
fn first_frame_shown(frame: &eframe::Frame) {
    if let Some(disk) = DISK.get() {
        disk.send(Out::Mark(None));
    }
    if SET_ASIDE.swap(false, Ordering::SeqCst)
        && let Some(graphics) = GRAPHICS.get()
    {
        put_back(graphics.api);
        step(&format!("{} drew after all: not set aside", api_name(graphics.api)));
    }
    if let Some(render) = frame.wgpu_render_state() {
        render.queue.on_submitted_work_done(|| write("the graphics card is done with the first frame"));
    }
}

/// Watch the start until a frame is shown: write down where it stands
/// after a few seconds; when it stays too long in one step, it is stuck.
fn watch() {
    let mut noted = false;
    // The step of Massif's own work found slow, and how many times the
    // main thread was written down held in it.
    let mut slow: Option<(Instant, usize)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if SHOWN.load(Ordering::SeqCst) {
            return watch_frames();
        }
        let Some(log) = LOG.get() else { return };
        let (since_start, (at, last, stage)) = {
            let log = lock(log);
            (log.started.elapsed(), log.last.clone())
        };
        if !noted && since_start >= NOTE_AFTER {
            noted = true;
            write(&format!("no frame shown after {} s; the start is at: {last}", NOTE_AFTER.as_secs()));
        }
        match verdict(stage, at.elapsed()) {
            Verdict::Wait => {}
            Verdict::SlowWork => {
                let notes = match slow {
                    Some((when, notes)) if when == at => notes,
                    _ => 0,
                };
                if notes == 0 {
                    write(&format!("slow for {} s at: {last} (Massif's own work, not the graphics driver: no graphics API set aside; still waiting){}", STUCK_AFTER.as_secs(), main_stack()));
                    slow = Some((at, 1));
                } else if let Some(&after) = HELD_NOTES.iter().filter(|d| **d > STUCK_AFTER).nth(notes - 1)
                    && at.elapsed() >= after
                {
                    write(&format!("still at it after {} s: {last}{}", after.as_secs(), main_stack()));
                    slow = Some((at, notes + 1));
                }
            }
            Verdict::StuckInDriver => {
                stuck(&last);
                return;
            }
        }
    }
}

/// Watch the frames, once the start is over, for as long as Massif runs: a
/// frame that keeps the main thread more than a few seconds (Windows calls
/// the window "not responding" after five) is written down, with where the
/// main thread is, and again while it lasts; then how long it took.
fn watch_frames() {
    // The frame written down as held: since when, and how many notes.
    let mut held: Option<(u64, usize)> = None;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let Some(log) = LOG.get() else { return };
        let now = lock(log).started.elapsed().as_millis() as u64;
        let current = busy();
        if let Some((since, _)) = held
            && current.is_none_or(|(_, _, s)| s != since)
        {
            write(&format!("the main thread went on, after about {:.1} s", now.saturating_sub(since) as f64 / 1000.0));
            held = None;
        }
        let Some((what, frame, since)) = current else { continue };
        let notes = held.map_or(0, |(_, n)| n);
        if let Some(after) = HELD_NOTES.get(notes)
            && now.saturating_sub(since) >= after.as_millis() as u64
        {
            let doing = match what {
                Busy::LayingOut => "laying out",
                _ => "waiting for the window's image (get_current_texture) for",
            };
            write(&format!("the main thread has been {doing} frame {frame} for {} s{}", after.as_secs(), main_stack()));
            held = Some((since, notes + 1));
        }
    }
}

/// Run the calling thread below the interface's priority (on Windows; as
/// it is elsewhere).
pub fn below_normal_priority() {
    #[cfg(windows)]
    windows::below_normal_priority();
}

/// Where the main thread is, to end a line of the log: on Windows its
/// stack, by module and offset (see `windows::main_stack`).
fn main_stack() -> String {
    #[cfg(windows)]
    if let Some(stack) = windows::main_stack() {
        return format!("\n    the main thread is in: {stack}");
    }
    String::new()
}

/// The start stays in a step of the graphics card: its graphics API is set
/// aside for the next starts, and on Windows the user is asked to start
/// again at once (a message box needs the main thread elsewhere, and it is
/// the one stuck).
fn stuck(last: &str) {
    write(&format!("stuck for {} s in the graphics driver at: {last}{}", STUCK_AFTER.as_secs(), main_stack()));
    let Some(graphics) = GRAPHICS.get() else {
        if cfg!(windows) {
            tell(&tr!("Massif has not opened its window after {as_secs} seconds: it may be stuck on the graphics card or its driver.{where_logged}", as_secs = STUCK_AFTER.as_secs(), where_logged = where_logged()));
        }
        return;
    };
    let api = graphics.api;
    SET_ASIDE.store(true, Ordering::SeqCst);
    set_aside(api, &format!("{}: stuck in the graphics driver at: {last}", graphics.card));
    if SHOWN.load(Ordering::SeqCst) {
        // It came meanwhile.
        if SET_ASIDE.swap(false, Ordering::SeqCst) {
            put_back(api);
        }
        return;
    }
    let next = next_api();
    write(&format!("{} set aside on this computer: the next start draws with {next}", api_name(api)));
    #[cfg(windows)]
    {
        let again = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Massif")
            .set_description(tr!("Massif has not drawn its window: the graphics card's driver has not answered for {as_secs} seconds with {api_name}.\n\nClick OK to start Massif again with {next}, which it will keep to on this computer, or Cancel to wait.{where_logged}", as_secs = STUCK_AFTER.as_secs(), api_name = api_name(api), where_logged = where_logged(), next))
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show()
            == rfd::MessageDialogResult::Ok;
        if SHOWN.load(Ordering::SeqCst) {
            return;
        }
        if again {
            write(&format!("starting again with {next}"));
            flush();
            if let Some(path) = pending_path() {
                let _ = std::fs::remove_file(path);
            }
            if let Ok(exe) = std::env::current_exe() {
                // Not with the API WGPU_BACKEND asked for, which is the one stuck.
                let _ = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env_remove("WGPU_BACKEND").spawn();
            }
            windows::end_now();
        }
        write("waiting");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed set-up of the window's surface (the card busy) is told
    /// from the errors that stay fatal.
    #[test]
    fn surface_errors_are_told_apart() {
        #[derive(Debug)]
        struct In(&'static str);
        impl std::fmt::Display for In {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for In {}
        let error = |what: &'static str| wgpu::Error::Validation { source: Box::new(In(what)), description: format!("Validation Error\n\nCaused by:\n  {what}\n") };
        assert!(of_the_surface(&error("In Surface::configure")));
        assert!(of_the_surface(&error("In Surface::get_current_texture_view")));
        assert!(!of_the_surface(&error("In Device::create_buffer, label = 'tiles'")));
        assert!(!of_the_surface(&wgpu::Error::OutOfMemory { source: Box::new(In("In Queue::submit")) }));
    }

    /// A disk that takes its time (here the log held by a write that does
    /// not end) never holds up the thread that writes lines down: they wait
    /// for it, in order, and the mark of the start with them.
    #[test]
    fn a_slow_disk_never_holds_up_the_log() {
        let dir = std::env::temp_dir().join(format!("massif-startup-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("startup.log"), "the last start\n").unwrap();
        let disk: &'static Disk = Box::leak(Box::new(Disk::at(Some(dir.join("startup.log")), Some(dir.join("first-frame.pending")))));
        // A write stuck on the disk for two seconds.
        let (held, is_held) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let file = lock(&disk.file);
            held.send(()).unwrap();
            std::thread::sleep(Duration::from_secs(2));
            drop(file);
        });
        is_held.recv().unwrap();
        std::thread::spawn(|| disk.run());
        let started = Instant::now();
        for k in 0..1000 {
            disk.send(Out::Line(format!("line {k}\n")));
        }
        disk.send(Out::Mark(Some("vulkan\n1\ncard\ngraphics\n".to_owned())));
        let sent = started.elapsed();
        assert!(sent < Duration::from_millis(500), "the lines waited on the disk: {sent:?}");
        // Once the disk answers, every line is there, in order.
        let log = dir.join("startup.log");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !std::fs::read_to_string(&log).is_ok_and(|t| t.ends_with("line 999\n")) || !dir.join("first-frame.pending").exists() {
            assert!(Instant::now() < deadline, "the lines were never written");
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("line 0\nline 1\n") && text.lines().count() == 1000, "{}", &text[..text.len().min(200)]);
        assert_eq!(std::fs::read_to_string(dir.join("startup.previous.log")).unwrap(), "the last start\n");
        disk.send(Out::Mark(None));
        disk.flush();
        assert!(!dir.join("first-frame.pending").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A frame that keeps the main thread is written down after 5 s, then
    /// after 15 s, a minute and three; not before.
    #[test]
    fn a_held_frame_is_written_down_as_it_lasts() {
        assert_eq!(HELD_NOTES[0], Duration::from_secs(5));
        assert!(HELD_NOTES.windows(2).all(|w| w[0] < w[1]));
        BUSY_WHAT.store(7 << 2 | Busy::WaitingForImage as u64, Ordering::SeqCst);
        BUSY_SINCE.store(1235, Ordering::SeqCst);
        assert_eq!(busy(), Some((Busy::WaitingForImage, 7, 1234)));
        BUSY_SINCE.store(0, Ordering::SeqCst);
        assert_eq!(busy(), None, "between frames the main thread waits for the system");
    }

    #[test]
    fn set_aside_apis_are_read_from_their_lines() {
        let api = apis()[0];
        let text = format!("# a comment\n{} AMD Radeon RX 6400 (driver 26.10.1): the first frame did not come\n\nnonsense\n", api.to_str());
        assert_eq!(parse_set_aside(&text), [api]);
        assert!(parse_set_aside(&format!("# {}\n", api.to_str())).is_empty());
    }

    /// An API is set aside once, with a word on the file, and put back.
    #[test]
    fn an_api_is_set_aside_and_put_back() {
        let dir = std::env::temp_dir().join(format!("massif-graphics-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("graphics.txt");
        let _ = std::fs::remove_file(&path);
        let api = apis()[0];
        set_aside_in(&path, api, "AMD Radeon RX 6400: the first frame did not come");
        set_aside_in(&path, api, "a second time");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse_set_aside(&text), [api]);
        assert!(text.starts_with('#') && !text.contains("a second time"), "{text}");
        put_back_in(&path, api);
        assert!(!path.exists(), "nothing set aside: no file");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only a step of the graphics card's sets its API aside: a start long
    /// in its own work (laying out the interface) is only written down.
    #[test]
    fn only_a_stuck_driver_sets_an_api_aside() {
        let long = STUCK_AFTER + Duration::from_secs(1);
        assert_eq!(verdict(Stage::App, Duration::from_secs(1)), Verdict::Wait);
        assert_eq!(verdict(Stage::Graphics, Duration::from_secs(1)), Verdict::Wait);
        assert_eq!(verdict(Stage::App, long), Verdict::SlowWork);
        assert_eq!(verdict(Stage::Graphics, long), Verdict::StuckInDriver);
        for stage in [Stage::App, Stage::Graphics] {
            assert_eq!(Stage::from_word(&format!("{}\r", stage.word())), Some(stage));
        }
        assert_eq!(Stage::from_word(""), None);
    }

    /// The lines 0.5.1 wrote for a first frame that did not come in time
    /// (Zumonym's three, on an RX 6400) are taken out, and the APIs tried
    /// again; a line of a stuck driver stays.
    #[test]
    fn old_watch_lines_are_left_aside() {
        let dir = std::env::temp_dir().join(format!("massif-graphics-old-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("graphics.txt");
        let card = "AMD Radeon RX 6400 (driver 4.6.0 Compatibility Profile Context 26.10.1.261008)";
        let old: String = apis().iter().map(|api| format!("{} {card}: the first frame did not come\r\n", api.to_str())).collect();
        std::fs::write(&path, format!("# The graphics APIs Massif does not draw with on this computer: each failed to draw its first frame.\n# Delete a line, or this file, to let Massif try that API again.\n{old}")).unwrap();
        assert_eq!(forget_old_watch(&path), apis());
        assert!(!path.exists(), "nothing left set aside: no file");
        set_aside_in(&path, apis()[0], &format!("{card}: stuck in the graphics driver at: frame 1: sent; asking the window for an image to draw on (get_current_texture)"));
        std::fs::write(&path, format!("{}{}", std::fs::read_to_string(&path).unwrap(), old)).unwrap();
        assert_eq!(forget_old_watch(&path), apis());
        assert_eq!(parse_set_aside(&std::fs::read_to_string(&path).unwrap()), [apis()[0]]);
        assert!(forget_old_watch(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wgpu_backend_is_read_in_quotes_or_not() {
        assert_eq!(named_backends("dx12"), wgpu::Backends::DX12);
        assert_eq!(named_backends("\"dx12\""), wgpu::Backends::DX12);
        assert_eq!(named_backends(" 'DX12' "), wgpu::Backends::DX12);
        assert_eq!(named_backends("vulkan,gl"), wgpu::Backends::VULKAN | wgpu::Backends::GL);
        assert!(named_backends("directx").is_empty());
    }

    /// The probe of a frame is drawn: egui_wgpu calls it once the window
    /// (here an offscreen image) gave an image to draw on.
    #[test]
    fn the_probe_follows_a_frame_to_its_image() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(64.0, 64.0)).wgpu().build_ui(|ui| {
            let corner = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0));
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(corner, Probe(1000)));
        });
        h.run();
        h.render().expect("rendered");
        assert_eq!(DRAWN.load(Ordering::Relaxed), 1000);
    }
}
