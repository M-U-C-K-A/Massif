//! The performance recorder (Help ▸ Record Performance…, Settings ▸
//! Developer, and the problem report's Start Recording): every frame
//! between Start and Stop, where its time went, part by part and thread by
//! thread, and what the user did meanwhile — one `.zip` to send, and to
//! replay against the same map (`replay`, in the tests).
//!
//! The parts are the scopes of the `profiling` crate: egui's, eframe's and
//! wgpu's own (laying out, tessellating, waiting for the screen, sending to
//! the graphics card) and Massif's (`profiling::scope!`). They reach a
//! small `tracing` subscriber ([`install`]) that answers "not now" to each
//! of them until a recording is on: a load of an atomic a scope, nothing
//! kept. While on, a scope costs two readings of the clock and a push.
//!
//! What goes in the file: the frames (a table), a timeline of every scope
//! on every thread (Trace Event format: chrome://tracing, ui.perfetto.dev),
//! the computer, the settings that change how Massif draws, the map's size,
//! the workspaces and the areas shown, and the input (pointer, keys, the
//! tool in hand) frame by frame. Never the map's content, and of the files
//! only their names: typed text goes in as its length.

use profiling::tracing;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
#[cfg(test)]
use std::time::Duration;

/// The longest a recording goes on: then it stops by itself.
pub const MAX_SECONDS: f64 = 600.0;
/// The most scopes kept: past them, the frames go on being recorded, their
/// parts no longer (about 40 bytes each).
const MAX_SPANS: usize = 4_000_000;

// ---------------------------------------------------------------------------
// The scopes

/// How many recordings are on (one in the app; tests may run several).
static ON: AtomicUsize = AtomicUsize::new(0);
/// The clock's zero: every time kept is nanoseconds since.
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// The scopes that ended, every thread's.
static SPANS: Mutex<Vec<Span>> = Mutex::new(Vec::new());
/// The names of scopes named as they run (`function_scope!`): their ids
/// have [`DYNAMIC`] set, and index this.
static NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NAME_IDS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
/// The threads seen: number, name.
static THREADS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static NEXT_THREAD: AtomicU32 = AtomicU32::new(1);
const DYNAMIC: u64 = 1 << 63;

/// A scope that ended.
#[derive(Debug, Clone, Copy)]
struct Span {
    /// Its callsite's metadata's address, or an index into [`NAMES`].
    id: u64,
    thread: u32,
    depth: u16,
    start: u64,
    dur: u64,
}

thread_local! {
    static THREAD: Cell<u32> = const { Cell::new(0) };
    /// The scopes entered and not left yet: id, start.
    static STACK: RefCell<Vec<(u64, u64)>> = const { RefCell::new(Vec::new()) };
    /// The part of the frame `mark` began last: its name and start.
    static MARK: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

fn now_ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn at_ns(t: Instant) -> u64 {
    t.saturating_duration_since(*EPOCH.get_or_init(Instant::now)).as_nanos() as u64
}

fn thread() -> u32 {
    THREAD.with(|t| {
        if t.get() == 0 {
            let n = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
            t.set(n);
            let name = std::thread::current().name().map_or_else(|| format!("thread {n}"), str::to_owned);
            if let Ok(mut threads) = THREADS.lock() {
                threads.push((n, name));
            }
        }
        t.get()
    })
}

/// A recording is on.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed) > 0
}

fn push(span: Span) {
    if let Ok(mut spans) = SPANS.lock()
        && spans.len() < MAX_SPANS
    {
        spans.push(span);
    }
}

/// The name of the scope `id`.
fn name_of(id: u64) -> String {
    if id & DYNAMIC != 0 {
        NAMES.lock().ok().and_then(|n| n.get((id & !DYNAMIC) as usize).cloned()).unwrap_or_default()
    } else {
        // SAFETY: ids without DYNAMIC are the addresses of callsites'
        // `'static` metadata, given by `new_span`.
        let meta = unsafe { &*(id as usize as *const tracing::Metadata<'static>) };
        meta.name().to_owned()
    }
}

fn intern(name: String) -> u64 {
    let ids = NAME_IDS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut ids) = ids.lock() else { return DYNAMIC };
    if let Some(id) = ids.get(&name) {
        return *id;
    }
    let Ok(mut names) = NAMES.lock() else { return DYNAMIC };
    let id = DYNAMIC | names.len() as u64;
    names.push(name.clone());
    ids.insert(name, id);
    id
}

/// The `tracing` subscriber the `profiling` scopes reach.
struct Recorder;

/// A function scope's name: its `message` field, with the module path cut
/// to its last two parts.
struct Message(Option<String>);

impl tracing::field::Visit for Message {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let full = format!("{value:?}");
            let parts: Vec<&str> = full.rsplitn(3, "::").collect();
            self.0 = Some(if parts.len() >= 2 { format!("{}::{}", parts[1], parts[0]) } else { full });
        }
    }
}

impl tracing::Subscriber for Recorder {
    fn register_callsite(&self, meta: &'static tracing::Metadata<'static>) -> tracing::subscriber::Interest {
        // Asked each time: on only while a recording is.
        if meta.is_span() { tracing::subscriber::Interest::sometimes() } else { tracing::subscriber::Interest::never() }
    }

    fn enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
        meta.is_span() && on()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let meta = attrs.metadata();
        if meta.name() == "function_scope" {
            let mut m = Message(None);
            attrs.record(&mut m);
            if let Some(name) = m.0 {
                return tracing::span::Id::from_u64(intern(name));
            }
        }
        tracing::span::Id::from_u64(meta as *const tracing::Metadata<'static> as usize as u64)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, _: &tracing::Event<'_>) {}

    fn enter(&self, id: &tracing::span::Id) {
        let start = now_ns();
        STACK.with_borrow_mut(|s| s.push((id.into_u64(), start)));
    }

    fn exit(&self, id: &tracing::span::Id) {
        let end = now_ns();
        let id = id.into_u64();
        let thread = thread();
        let popped = STACK.with_borrow_mut(|s| {
            let k = s.iter().rposition(|e| e.0 == id)?;
            let (_, start) = s.remove(k);
            Some((start, k))
        });
        if let Some((start, depth)) = popped
            && on()
        {
            push(Span { id, thread, depth: depth as u16, start, dur: end - start });
        }
    }

    fn clone_span(&self, id: &tracing::span::Id) -> tracing::span::Id {
        id.clone()
    }

    fn try_close(&self, _: tracing::span::Id) -> bool {
        false
    }
}

/// Make the scopes reach the recorder: once, before the first frame.
pub fn install() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        EPOCH.get_or_init(Instant::now);
        let _ = tracing::subscriber::set_global_default(Recorder);
    });
}

/// A part of the frame begins (`startup::frame_step`), lasting until the
/// next or the frame's end: noted while a recording is on.
pub fn mark(what: &str) {
    if !on() {
        return;
    }
    let now = now_ns();
    let thread = thread();
    MARK.with_borrow_mut(|m| {
        if let Some((name, start)) = m.take() {
            push(Span { id: intern(name), thread, depth: 1, start, dur: now - start });
        }
        *m = Some((what.to_owned(), now));
    });
}

/// The frame's last part ends.
fn end_marks() {
    let now = now_ns();
    let thread = thread();
    MARK.with_borrow_mut(|m| {
        if let Some((name, start)) = m.take()
            && on()
        {
            push(Span { id: intern(name), thread, depth: 1, start, dur: now - start });
        }
    });
}

/// A part of the frame that is not a scope (a frame's whole work, timed
/// by the app): kept as one.
fn push_named(name: &str, start: Instant, end: Instant, depth: u16) {
    if on() {
        let start = at_ns(start);
        push(Span { id: intern(name.to_owned()), thread: thread(), depth, start, dur: at_ns(end).saturating_sub(start) });
    }
}

// ---------------------------------------------------------------------------
// The recording

/// What the app says of a frame, once its work is done.
#[derive(Debug, Clone, Default)]
pub struct FrameInfo {
    /// When the frame's work began, and ended.
    pub begun: Option<Instant>,
    pub ended: Option<Instant>,
    pub memory: Option<f32>,
    pub cpu: Option<f32>,
    pub jobs: Vec<String>,
    /// Why it was drawn: egui's repaint causes.
    pub causes: Vec<String>,
    /// The frame before asked for this one at once (not after a while).
    pub again: bool,
    /// The input of the frame, as JSON (`input_json`).
    pub events: Vec<serde_json::Value>,
    /// The tool in hand.
    pub tool: String,
    /// The areas shown, in a few words (`Map 1200×800, 3D View 600×800`).
    pub areas: String,
    /// The window's size in points, and its pixels a point.
    pub window: [f32; 2],
    pub ppp: f32,
    /// Map tiles kept decoded, and the graphics card's memory Massif knows of (MB).
    pub tiles: usize,
    pub gpu_mb: f32,
    /// Windows of floating panels open.
    pub windows: usize,
}

/// A frame of a recording.
#[derive(Debug, Clone)]
struct Frame {
    /// Milliseconds since the recording started, when the frame's work began.
    at: f64,
    /// Milliseconds until the next frame began (or the recording stopped).
    gap: f32,
    /// Milliseconds of the app's own work (the interface laid out, built, drawn).
    work: f32,
    /// Milliseconds the frame kept the thread busy, its drawing included.
    busy: f32,
    info: FrameInfo,
    /// The main thread's parts: name, total milliseconds, the outermost of
    /// each (nested ones of the same name not counted twice).
    parts: Vec<(String, f32)>,
}

/// A performance recording: Start, the slow thing, Stop.
pub struct Recording {
    started: Instant,
    start_ns: u64,
    /// The thread that draws the interface.
    thread: u32,
    frames: Vec<Frame>,
    /// The frame whose drawing goes on after its work: completed when the
    /// next begins.
    pending: Option<FrameInfo>,
    /// Every scope recorded, drained frame by frame.
    spans: Vec<Span>,
    pub stopped: Option<f64>,
    /// Started from the problem report: Stop goes back to it.
    pub for_report: bool,
    /// What does not change frame by frame: name, value (the computer,
    /// the settings, the map, the workspaces).
    pub context: Vec<(String, Vec<(String, String)>)>,
    /// The workspaces, as saved (for the replay).
    pub layout: Option<String>,
}

impl Recording {
    pub fn start() -> Recording {
        install();
        ON.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut s) = SPANS.lock() {
            s.clear();
        }
        MARK.with_borrow_mut(|m| *m = None);
        Recording {
            started: Instant::now(),
            start_ns: now_ns(),
            thread: thread(),
            frames: Vec::new(),
            pending: None,
            spans: Vec::new(),
            stopped: None,
            for_report: false,
            context: Vec::new(),
            layout: None,
        }
    }

    pub fn is_on(&self) -> bool {
        self.stopped.is_none()
    }

    /// Seconds recorded.
    pub fn seconds(&self) -> f64 {
        self.stopped.unwrap_or_else(|| self.started.elapsed().as_secs_f64())
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// The median and the worst time a frame kept the interface busy (ms).
    pub fn busy_median_worst(&self) -> (f32, f32) {
        let busy: Vec<f32> = self.frames.iter().map(|f| f.busy).collect();
        (percentile(&busy, 0.5), percentile(&busy, 1.0))
    }

    pub fn stop(&mut self) {
        if self.stopped.is_none() {
            end_marks();
            self.drain();
            if let Some(info) = self.pending.take() {
                let next = info.ended.unwrap_or_else(Instant::now).max(Instant::now());
                self.complete(info, next);
            }
            self.stopped = Some(self.started.elapsed().as_secs_f64());
            ON.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn drain(&mut self) {
        if let Ok(mut s) = SPANS.lock() {
            self.spans.append(&mut s);
        }
    }

    /// A frame's work is done (`info`): the one before it is complete, its
    /// drawing included. Stops by itself after [`MAX_SECONDS`].
    pub fn frame(&mut self, info: FrameInfo) {
        if !self.is_on() {
            return;
        }
        end_marks();
        if let (Some(b), Some(e)) = (info.begun, info.ended) {
            push_named("frame work", b, e, 0);
        }
        self.drain();
        if let Some(prev) = self.pending.take() {
            let next = info.begun.unwrap_or_else(Instant::now);
            self.complete(prev, next);
        }
        self.pending = Some(info);
        if self.started.elapsed().as_secs_f64() >= MAX_SECONDS {
            self.stop();
        }
    }

    /// Frame `info`, whose successor began at `next`.
    fn complete(&mut self, info: FrameInfo, next: Instant) {
        let (Some(begun), Some(ended)) = (info.begun, info.ended) else { return };
        let (from, to) = (at_ns(begun), at_ns(next));
        // The main thread's scopes that began in the frame, outermost of
        // each name only.
        let mut parts: Vec<(String, f32)> = Vec::new();
        let mut busy_until = at_ns(ended);
        let mut open: Vec<(u64, u64)> = Vec::new(); // id, end: of the counted ones enclosing
        let mut mine: Vec<&Span> = self.spans.iter().rev().take_while(|s| s.start >= from.saturating_sub(1_000_000_000)).filter(|s| s.thread == self.thread && s.start >= from && s.start < to).collect();
        mine.sort_by_key(|s| (s.start, std::cmp::Reverse(s.dur)));
        for s in mine {
            busy_until = busy_until.max(s.start + s.dur);
            open.retain(|o| o.1 > s.start);
            if open.iter().any(|o| o.0 == s.id) {
                continue;
            }
            open.push((s.id, s.start + s.dur));
            let name = name_of(s.id);
            let ms = s.dur as f32 / 1e6;
            match parts.iter_mut().find(|p| p.0 == name) {
                Some(p) => p.1 += ms,
                None => parts.push((name, ms)),
            }
        }
        let at = begun.saturating_duration_since(self.started).as_secs_f64() * 1000.0;
        let gap = next.saturating_duration_since(begun).as_secs_f32() * 1000.0;
        let work = ended.saturating_duration_since(begun).as_secs_f32() * 1000.0;
        let busy = (busy_until.saturating_sub(from)) as f32 / 1e6;
        self.frames.push(Frame { at, gap, work, busy: busy.min(gap.max(work)), info, parts });
    }

    /// The parts that matter, a column each in `frames.csv`: a name, and
    /// the scopes it adds up (by the end of their names).
    const COLUMNS: [(&'static str, &'static [&'static str]); 14] = [
        ("ui_ms", &["App::ui"]),
        ("map_ms", &["map view"]),
        ("map_tiles_ms", &["map tiles"]),
        ("view3d_ms", &["3D view"]),
        ("view3d_gpu_ms", &["3D view: draw"]),
        ("diorama_ms", &["Diorama"]),
        ("render_ms", &["Render"]),
        ("panels_ms", &["panels"]),
        ("floating_ms", &["floating window"]),
        ("end_pass_ms", &["Context::end_pass"]),
        ("tessellate_ms", &["tessellate", "tessellator::tessellate_shapes"]),
        ("paint_ms", &["paint_and_update_textures"]),
        ("wait_screen_ms", &["get_current_texture"]),
        ("present_ms", &["present"]),
    ];

    fn column(f: &Frame, names: &[&str]) -> f32 {
        f.parts.iter().filter(|(n, _)| names.iter().any(|w| n == w || n.ends_with(&format!("::{w}")))).map(|p| p.1).sum()
    }

    /// A line a frame.
    pub fn frames_csv(&self) -> String {
        let mut out = String::from("frame,start_ms,frame_ms,busy_ms,work_ms");
        for (c, _) in Self::COLUMNS {
            out.push(',');
            out.push_str(c);
        }
        out.push_str(",slowest_part,slowest_part_ms,memory_mb,cpu_percent,tiles,gpu_mb,again,events,tool,windows,why,tasks\n");
        for (n, f) in self.frames.iter().enumerate() {
            out.push_str(&format!("{n},{:.2},{:.2},{:.2},{:.2}", f.at, f.gap, f.busy, f.work));
            for (_, names) in Self::COLUMNS {
                out.push_str(&format!(",{:.2}", Self::column(f, names)));
            }
            let slow = slowest_part(f);
            out.push_str(&format!(
                ",{},{},{},{},{},{:.0},{},{},{},{},{},{}\n",
                csv_field(slow.map_or("", |s| s.0.as_str())),
                slow.map_or(String::new(), |s| format!("{:.2}", s.1)),
                f.info.memory.map_or(String::new(), |m| format!("{m:.0}")),
                f.info.cpu.map_or(String::new(), |c| format!("{c:.0}")),
                f.info.tiles,
                f.info.gpu_mb,
                u8::from(f.info.again),
                f.info.events.len(),
                csv_field(&f.info.tool),
                f.info.windows,
                csv_field(&f.info.causes.join("; ")),
                csv_field(&f.info.jobs.join("; ")),
            ));
        }
        out
    }

    /// A timeline in the Trace Event format (chrome://tracing, Perfetto):
    /// every scope on its thread's track, the frames, the input, the
    /// memory and the processor as counters.
    pub fn trace_json(&self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(self.spans.len() * 90 + 4096);
        out.push_str("{\"displayTimeUnit\":\"ms\",\"traceEvents\":[\n");
        let mut first = true;
        let mut ev = |out: &mut String, s: &str| {
            if !first {
                out.push_str(",\n");
            }
            first = false;
            out.push_str(s);
        };
        let us = |ns: u64| ns.saturating_sub(self.start_ns) as f64 / 1000.0;
        let threads = THREADS.lock().map(|t| t.clone()).unwrap_or_default();
        for (n, name) in &threads {
            if self.spans.iter().any(|s| s.thread == *n) || *n == self.thread {
                let name = if *n == self.thread { "Massif (interface)".to_owned() } else { name.clone() };
                ev(&mut out, &format!("{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":{n},\"args\":{{\"name\":{}}}}}", json_str(&name)));
                ev(&mut out, &format!("{{\"name\":\"thread_sort_index\",\"ph\":\"M\",\"pid\":1,\"tid\":{n},\"args\":{{\"sort_index\":{}}}}}", if *n == self.thread { 0 } else { *n }));
            }
        }
        ev(&mut out, "{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":0,\"args\":{\"name\":\"Frames\"}}");
        let mut names: HashMap<u64, String> = HashMap::new();
        for s in &self.spans {
            let name = names.entry(s.id).or_insert_with(|| json_str(&name_of(s.id)));
            let mut line = String::new();
            let _ = write!(line, "{{\"name\":{name},\"ph\":\"X\",\"pid\":1,\"tid\":{},\"ts\":{:.1},\"dur\":{:.1}}}", s.thread, us(s.start), (s.dur as f64 / 1000.0).max(0.1));
            ev(&mut out, &line);
        }
        for (n, f) in self.frames.iter().enumerate() {
            let ts = f.at * 1000.0;
            let slow = slowest_part(f).map_or(String::new(), |s| s.0.clone());
            ev(
                &mut out,
                &format!(
                    "{{\"name\":{},\"ph\":\"X\",\"pid\":1,\"tid\":0,\"ts\":{ts:.1},\"dur\":{:.1},\"args\":{{\"frame_ms\":{:.2},\"busy_ms\":{:.2},\"slowest\":{},\"why\":{}}}}}",
                    json_str(&format!("frame {n} · {:.1} ms", f.busy)),
                    (f.busy as f64 * 1000.0).max(1.0),
                    f.gap,
                    f.busy,
                    json_str(&slow),
                    json_str(&f.info.causes.join("; "))
                ),
            );
            for e in &f.info.events {
                ev(&mut out, &format!("{{\"name\":{},\"ph\":\"i\",\"s\":\"g\",\"pid\":1,\"tid\":0,\"ts\":{ts:.1},\"args\":{{\"event\":{e}}}}}", json_str(event_name(e))));
            }
            if let Some(m) = f.info.memory {
                ev(&mut out, &format!("{{\"name\":\"memory (MB)\",\"ph\":\"C\",\"pid\":1,\"ts\":{ts:.1},\"args\":{{\"MB\":{m:.0}}}}}"));
            }
            if let Some(c) = f.info.cpu {
                ev(&mut out, &format!("{{\"name\":\"processor (% of a core)\",\"ph\":\"C\",\"pid\":1,\"ts\":{ts:.1},\"args\":{{\"percent\":{c:.0}}}}}"));
            }
            ev(&mut out, &format!("{{\"name\":\"map tiles decoded\",\"ph\":\"C\",\"pid\":1,\"ts\":{ts:.1},\"args\":{{\"tiles\":{}}}}}", f.info.tiles));
        }
        out.push_str("\n]}\n");
        out
    }

    /// The input, a frame a line (JSON): what the replay plays back.
    pub fn inputs_jsonl(&self) -> String {
        let mut out = String::new();
        let mut tool = String::new();
        let mut areas = String::new();
        for (n, f) in self.frames.iter().enumerate() {
            let mut line = serde_json::json!({"frame": n, "ms": (f.at * 10.0).round() / 10.0, "window": f.info.window, "ppp": f.info.ppp});
            if f.info.tool != tool {
                tool.clone_from(&f.info.tool);
                line["tool"] = serde_json::json!(tool);
            }
            if f.info.areas != areas {
                areas.clone_from(&f.info.areas);
                line["areas"] = serde_json::json!(areas);
            }
            if !f.info.events.is_empty() {
                line["events"] = serde_json::Value::Array(f.info.events.clone());
            }
            out.push_str(&line.to_string());
            out.push('\n');
        }
        out
    }

    /// What the recording found, in words: its frames, the slowest of them,
    /// and the parts that took the most time.
    pub fn summary(&self) -> String {
        let mut s = format!("Performance recording: {:.1} s, {} frames\n", self.seconds(), self.frames.len());
        if self.frames.is_empty() {
            return s;
        }
        let busy: Vec<f32> = self.frames.iter().map(|f| f.busy).collect();
        let work: Vec<f32> = self.frames.iter().map(|f| f.work).collect();
        // Frames that followed one asking to be drawn again at once: the
        // time between them is what the eye sees.
        let steady: Vec<f32> = self.frames.windows(2).filter(|w| w[1].info.again).map(|w| w[0].gap).collect();
        s.push_str(&format!(
            "Busy a frame (its work and drawing): median {:.1} ms, 95th percentile {:.1} ms, 99th {:.1} ms, worst {:.1} ms\nWork a frame (the app's own): median {:.1} ms, worst {:.1} ms\nFrames busy over 16.7 ms: {}, over 33 ms: {}, over 100 ms: {}\n",
            percentile(&busy, 0.5),
            percentile(&busy, 0.95),
            percentile(&busy, 0.99),
            percentile(&busy, 1.0),
            percentile(&work, 0.5),
            percentile(&work, 1.0),
            busy.iter().filter(|g| **g > 16.7).count(),
            busy.iter().filter(|g| **g > 33.3).count(),
            busy.iter().filter(|g| **g > 100.0).count(),
        ));
        if !steady.is_empty() {
            s.push_str(&format!(
                "While drawing continuously ({} frames): {:.0} frames a second, frame time median {:.1} ms, 95th percentile {:.1} ms, worst {:.1} ms\n",
                steady.len(),
                steady.len() as f32 * 1000.0 / steady.iter().sum::<f32>().max(1e-3),
                percentile(&steady, 0.5),
                percentile(&steady, 0.95),
                percentile(&steady, 1.0)
            ));
        }
        // The parts, by the time spent in them all together.
        let mut parts: Vec<(String, f64, f32, usize)> = Vec::new();
        for f in &self.frames {
            for (name, dur) in &f.parts {
                match parts.iter_mut().find(|p| &p.0 == name) {
                    Some(p) => {
                        p.1 += *dur as f64;
                        p.2 = p.2.max(*dur);
                        p.3 += 1;
                    }
                    None => parts.push((name.clone(), *dur as f64, *dur, 1)),
                }
            }
        }
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        if !parts.is_empty() {
            s.push_str("\nWhere the interface thread's time went (all frames, worst frame, frames):\n");
            for (name, total, worst, n) in parts.iter().take(30) {
                s.push_str(&format!("  {total:>9.1} ms  {worst:>7.1} ms  {n:>6}  {name}\n"));
            }
        }
        // The other threads.
        let mut others: Vec<(u32, u64, usize)> = Vec::new();
        for sp in self.spans.iter().filter(|sp| sp.thread != self.thread && sp.depth == 0) {
            match others.iter_mut().find(|o| o.0 == sp.thread) {
                Some(o) => {
                    o.1 += sp.dur;
                    o.2 += 1;
                }
                None => others.push((sp.thread, sp.dur, 1)),
            }
        }
        if !others.is_empty() {
            let threads = THREADS.lock().map(|t| t.clone()).unwrap_or_default();
            others.sort_by_key(|o| std::cmp::Reverse(o.1));
            s.push_str("\nOther threads (time in their outermost scopes):\n");
            for (t, ns, n) in others.iter().take(12) {
                let name = threads.iter().find(|x| x.0 == *t).map_or("?", |x| x.1.as_str());
                s.push_str(&format!("  {:>9.1} ms  {n:>6}  {name}\n", *ns as f64 / 1e6));
            }
        }
        let mut slow: Vec<(usize, &Frame)> = self.frames.iter().enumerate().collect();
        slow.sort_by(|a, b| b.1.busy.total_cmp(&a.1.busy));
        s.push_str("\nSlowest frames:\n");
        for (n, f) in slow.iter().take(15) {
            let mut top: Vec<&(String, f32)> = f.parts.iter().filter(|p| !is_whole(&p.0)).collect();
            top.sort_by(|a, b| b.1.total_cmp(&a.1));
            let top: Vec<String> = top.iter().take(4).map(|p| format!("{} {:.1}", p.0, p.1)).collect();
            let jobs = if f.info.jobs.is_empty() { String::new() } else { format!(", tasks: {}", f.info.jobs.join("; ")) };
            let why = if f.info.causes.is_empty() { String::new() } else { format!(", why: {}", f.info.causes.join("; ")) };
            s.push_str(&format!("  frame {n} at {:.2} s: busy {:.1} ms, work {:.1} ms; {}{jobs}{why}\n", f.at / 1000.0, f.busy, f.work, top.join(", ")));
        }
        // Drawing when nothing happens: frames that asked for the next at
        // once without input.
        let idle = self.frames.windows(2).filter(|w| w[1].info.again && w[0].info.events.is_empty() && w[0].info.jobs.is_empty()).count();
        let mut causes: Vec<(String, usize)> = Vec::new();
        for f in self.frames.iter().filter(|f| f.info.events.is_empty()) {
            for c in &f.info.causes {
                match causes.iter_mut().find(|x| &x.0 == c) {
                    Some(x) => x.1 += 1,
                    None => causes.push((c.clone(), 1)),
                }
            }
        }
        causes.sort_by_key(|c| std::cmp::Reverse(c.1));
        s.push_str(&format!("\nFrames without input or task that asked for the next at once: {idle}\n"));
        if !causes.is_empty() {
            s.push_str("Why frames without input were drawn:\n");
            for (c, n) in causes.iter().take(10) {
                s.push_str(&format!("  {n:>6}  {c}\n"));
            }
        }
        s
    }

    /// The files of the recording, for its own zip or a problem report.
    pub fn parts(&self, prefix: &str) -> Vec<crate::report::Part> {
        let text = |name: &str, about: &'static str, s: String| crate::report::Part { name: format!("{prefix}{name}"), about, bytes: s.into_bytes(), text: true };
        let mut out = vec![
            text("summary.txt", tn!("The recording: its slowest frames, where the work went"), self.summary()),
            text("frames.csv", tn!("Each frame recorded"), self.frames_csv()),
            text("trace.json", tn!("A timeline for chrome://tracing or ui.perfetto.dev"), self.trace_json()),
            text("inputs.jsonl", tn!("The pointer, the keys and the tool in hand, frame by frame"), self.inputs_jsonl()),
        ];
        if !self.context.is_empty() {
            let mut s = String::from("Massif performance recording\n");
            for (title, lines) in &self.context {
                s.push_str(&format!("\n## {title}\n"));
                for (k, v) in lines {
                    s.push_str(&format!("{k}: {v}\n"));
                }
            }
            out.push(text("system.txt", tn!("Massif, the computer, the settings, the map's size, the workspaces"), s));
        }
        if let Some(layout) = &self.layout {
            out.push(text("layout.json", tn!("The workspaces, for the replay"), layout.clone()));
        }
        out
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if self.stopped.is_none() {
            ON.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Scopes that hold the whole frame: not a part of it.
fn is_whole(name: &str) -> bool {
    name == "frame work" || name.ends_with("App::ui") || name.ends_with("run_ui_and_paint") || name.ends_with("::run") || name.ends_with("viewport_callback") || name.contains("window_event") || name.ends_with("::paint") || name == "Massif frame"
}

/// The frame's longest part that is not all of it.
fn slowest_part(f: &Frame) -> Option<&(String, f32)> {
    f.parts.iter().filter(|p| !is_whole(&p.0)).max_by(|a, b| a.1.total_cmp(&b.1))
}

/// An input event, as the trace names it.
fn event_name(e: &serde_json::Value) -> &str {
    match e {
        serde_json::Value::Object(m) => m.keys().next().map_or("input", String::as_str),
        serde_json::Value::String(s) => s,
        _ => "input",
    }
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
}

/// The value under which `q` of `v` are (0.5 the median, 1 the largest).
pub fn percentile(v: &[f32], q: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[((s.len() - 1) as f32 * q).round() as usize]
}

/// The input of a frame, as the recording keeps it: typed and pasted text
/// as its length only, nothing copied.
pub fn input_json(events: &[egui::Event]) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|e| {
            let e = match e {
                egui::Event::Text(t) => egui::Event::Text("x".repeat(t.chars().count().min(64))),
                egui::Event::Paste(t) => egui::Event::Paste("x".repeat(t.chars().count().min(64))),
                egui::Event::Copy | egui::Event::Cut | egui::Event::AccessKitActionRequest(_) => return None,
                egui::Event::Ime(_) => return None,
                other => other.clone(),
            };
            serde_json::to_value(&e).ok()
        })
        .collect()
}

/// The tests that record take turns: the profiler is one for the whole
/// process, and a recording stopped by one test would cut another's short.
#[cfg(test)]
pub static RECORDING: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    /// The tests' own folder for recordings, never the desktop.
    pub static TEST_DIR: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
}

/// Where a recording is saved: the desktop, or the home folder.
pub fn file_path() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with_borrow(Clone::clone) {
        return dir.join(crate::report::file_name().replacen("Massif report", "Massif performance", 1));
    }
    let home = std::env::home_dir().unwrap_or_default();
    let desktop = home.join("Desktop");
    let dir = if desktop.is_dir() { desktop } else { home };
    dir.join(crate::report::file_name().replacen("Massif report", "Massif performance", 1))
}

/// A recording read back from its files (for the replay).
#[cfg(test)]
#[derive(Debug, Default)]
pub struct Recorded {
    /// The frames: window size, pixels a point, tool, its events.
    pub frames: Vec<RecordedFrame>,
    pub layout: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct RecordedFrame {
    pub ms: f64,
    pub window: [f32; 2],
    pub ppp: f32,
    pub tool: Option<String>,
    pub events: Vec<egui::Event>,
}

/// Read a recording: its zip, or a folder it was unzipped into.
#[cfg(test)]
pub fn read_recording(path: &std::path::Path) -> std::io::Result<Recorded> {
    let files: Vec<(String, Vec<u8>)> = if path.is_dir() {
        ["inputs.jsonl", "layout.json"].iter().filter_map(|n| std::fs::read(path.join(n)).ok().map(|b| ((*n).to_owned(), b))).collect()
    } else {
        crate::report::unzip(&std::fs::read(path)?)
    };
    let find = |n: &str| files.iter().find(|f| f.0 == n || f.0.ends_with(&format!("/{n}"))).map(|f| String::from_utf8_lossy(&f.1).into_owned());
    let inputs = find("inputs.jsonl").ok_or_else(|| std::io::Error::other("no inputs.jsonl in the recording"))?;
    let mut out = Recorded { layout: find("layout.json"), ..Recorded::default() };
    for line in inputs.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).map_err(std::io::Error::other)?;
        let window = v["window"].as_array().map_or([1600.0, 1000.0], |a| [a.first().and_then(|x| x.as_f64()).unwrap_or(1600.0) as f32, a.get(1).and_then(|x| x.as_f64()).unwrap_or(1000.0) as f32]);
        let events = v["events"].as_array().map(|a| a.iter().filter_map(|e| serde_json::from_value(e.clone()).ok()).collect()).unwrap_or_default();
        out.frames.push(RecordedFrame { ms: v["ms"].as_f64().unwrap_or(0.0), window, ppp: v["ppp"].as_f64().unwrap_or(1.0) as f32, tool: v["tool"].as_str().map(str::to_owned), events });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scopes reach the recording only while it is on; marks last until
    /// the next; the frames, their parts, the trace and the input come out.
    #[test]
    fn a_recording_has_its_frames_parts_and_input() {
        let _turn = RECORDING.lock().unwrap_or_else(|e| e.into_inner());
        install();
        // Another test may be recording at the same time: its scopes then
        // reach this recording too.
        let alone = !on();
        {
            profiling::scope!("before the start");
        }
        let mut r = Recording::start();
        for n in 0..3 {
            let begun = Instant::now();
            mark("the map");
            {
                profiling::scope!("map view");
                std::thread::sleep(Duration::from_millis(2));
            }
            mark("the panels");
            let events = input_json(&[egui::Event::PointerMoved(egui::pos2(10.0, 20.0 + n as f32)), egui::Event::Text("secret name".into())]);
            r.frame(FrameInfo {
                begun: Some(begun),
                ended: Some(Instant::now()),
                memory: Some(500.0),
                cpu: Some(30.0),
                jobs: if n == 1 { vec!["Exporting · 40 %".into()] } else { vec![] },
                causes: vec!["map_view.rs:12".into()],
                again: true,
                events,
                tool: "Raise".into(),
                window: [800.0, 600.0],
                ppp: 2.0,
                ..FrameInfo::default()
            });
        }
        r.stop();
        {
            profiling::scope!("after the stop");
        }
        mark("after the stop");
        assert_eq!(r.frames(), 3);
        let csv = r.frames_csv();
        assert_eq!(csv.lines().count(), 4);
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        let map_col = header.iter().position(|h| *h == "map_ms").unwrap();
        let row: Vec<&str> = csv.lines().nth(1).unwrap().split(',').collect();
        assert!(row[map_col].parse::<f32>().unwrap() >= 1.5, "{csv}");
        let trace: serde_json::Value = serde_json::from_str(&r.trace_json()).expect("the trace is JSON");
        let names: Vec<&str> = trace["traceEvents"].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect();
        assert!(names.contains(&"the panels") && names.contains(&"map view") && names.iter().any(|n| n.starts_with("frame 2")), "{names:?}");
        assert!(!names.contains(&"after the stop"), "{names:?}");
        assert!(!alone || !names.contains(&"before the start"), "{names:?}");
        let summary = r.summary();
        assert!(summary.contains("3 frames") && summary.contains("map view"), "{summary}");
        let inputs = r.inputs_jsonl();
        assert!(!inputs.contains("secret") && inputs.contains("xxxxxxxxxxx") && inputs.contains("Raise"), "{inputs}");
        // Read back as the replay does.
        let dir = std::env::temp_dir().join(format!("massif-perf-{}", std::process::id()));
        let path = dir.join("r.zip");
        std::fs::create_dir_all(&dir).unwrap();
        crate::report::save(&path, &r.parts("")).unwrap();
        let back = read_recording(&path).unwrap();
        assert_eq!(back.frames.len(), 3);
        assert!(matches!(back.frames[1].events[0], egui::Event::PointerMoved(p) if p.y == 21.0));
        assert_eq!(back.frames[0].tool.as_deref(), Some("Raise"));
        assert_eq!(back.frames[1].tool, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Off, a scope costs next to nothing: a few nanoseconds.
    #[test]
    fn scopes_cost_nothing_while_off() {
        install();
        if on() {
            return; // another test records now
        }
        let started = Instant::now();
        for _ in 0..200_000 {
            profiling::scope!("an idle scope");
        }
        let each = started.elapsed().as_nanos() as f64 / 200_000.0;
        assert!(each < 200.0, "{each:.1} ns a scope");
    }
}
