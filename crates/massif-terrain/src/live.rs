//! Live: the result of a graph while it is edited, the visible area first.
//!
//! [`Live`] evaluates a graph on a thread of its own and hands back tiles as
//! they are made, for a view to draw over the map without writing it. Each
//! version of the graph set ([`Live::set_graph`]) starts again, and stops
//! what the older one was making at its next node. For each version it
//! makes, in order:
//!
//! 1. the tiles in view, at the view's level of detail (a sample a pixel or
//!    finer), nearest the view's centre first;
//! 2. the whole terrain at the coarsest level with at most
//!    [`Settings::overview`] samples a side, for an overview and the minimap;
//! 3. once the graph has rested ([`Settings::rest`] without a new version),
//!    for a graph that simulates: 1 and 2 again with its simulations at
//!    [`Settings::sim_res`] — they first ran at [`Settings::quick_sim_res`],
//!    quick while a slider moves;
//! 4. in the [`Mode::Whole`] mode and once the graph has rested, the rest of
//!    the terrain at the view's level (or coarser, at most
//!    [`Settings::whole_res`] samples a side), by distance from the view.
//!
//! The view moving is planned for after the batch of tiles being made (a
//! batch of the rest of the terrain stops at once); nothing made is lost,
//! the engine keeping it in its cache, so going back to a version or a place
//! is instant.

use crate::compile::{Compiled, Diagnostic, compile_in};
use crate::engine::{Engine, EvalError, TileData};
use crate::kernel::Env;
use crate::model::Graph;
use crate::registry::Kernel;
use crate::space::{MAX_LOD, Rect, Space, TILE, TileKey};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What Live makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Nothing.
    Off,
    /// The view, and an overview of the terrain.
    #[default]
    Visible,
    /// Then the rest of the terrain, at the view's level of detail.
    Whole,
}

/// How Live works.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    /// Samples a side simulations run at once the graph rests.
    pub sim_res: u32,
    /// Samples a side simulations run at first, quick while settings change.
    pub quick_sim_res: u32,
    /// How long the graph stays unchanged before the finer runs.
    pub rest: Duration,
    /// Samples a side of the overview of the whole terrain.
    pub overview: u32,
    /// Samples a side the rest of the terrain is made at, at most.
    pub whole_res: u32,
    /// The Map Output's inputs to make (`height`, `albedo`, `water`…).
    pub outputs: Vec<String>,
    /// Tiles given on the graphics card ([`TileData::gpu_buffer`]) rather
    /// than on the CPU.
    pub on_gpu: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { mode: Mode::Visible, sim_res: 2048, quick_sim_res: 1024, rest: Duration::from_millis(300), overview: 512, whole_res: 8192, outputs: vec!["height".to_owned()], on_gpu: false }
    }
}

/// Which part of the terrain a tile was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The view, at its level of detail.
    Visible,
    /// The whole terrain, coarse.
    Overview,
    /// The rest of the terrain at the view's level.
    Rest,
}

/// A tile made.
#[derive(Debug, Clone)]
pub struct Ready {
    /// The revision of the graph it was made from, as given to
    /// [`Live::set_graph`].
    pub revision: u64,
    /// Which of [`Settings::outputs`].
    pub output: usize,
    pub key: TileKey,
    pub tile: Arc<TileData>,
    pub phase: Phase,
    /// Made with simulations at [`Settings::sim_res`]; `false` for the
    /// quick first version of a graph that simulates.
    pub exact: bool,
}

/// Where Live is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    /// The revision being made.
    pub revision: u64,
    /// What is being made (with simulations at their setting's resolution),
    /// `None` when nothing is.
    pub phase: Option<(Phase, bool)>,
    /// Tiles made and to make in that phase.
    pub done: usize,
    pub total: usize,
    /// Nothing is left to make until something changes.
    pub idle: bool,
    /// Why the graph cannot be evaluated.
    pub error: Option<String>,
    /// From the revision being set to its first tile in view, to the view
    /// complete, and to everything the mode makes.
    pub first_visible: Option<Duration>,
    pub visible: Option<Duration>,
    pub finished: Option<Duration>,
}

/// The graph's result while it is edited (see the module).
pub struct Live {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Live({:?})", self.progress())
    }
}

type Notify = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    engine: Arc<Engine>,
    state: Mutex<State>,
    wake: Condvar,
    /// The current generation and view: older batches stop.
    generation: AtomicU64,
    view: AtomicU64,
    paused: AtomicBool,
    stop: AtomicBool,
    ready: Mutex<Vec<Ready>>,
    progress: Mutex<Progress>,
    idle: Condvar,
    notify: Mutex<Option<Notify>>,
}

/// The graph as set, to compile it again when the settings change.
struct Source {
    graph: Graph,
    space: Space,
    env: Arc<dyn Env>,
    revision: u64,
}

/// The graph compiled with its simulations at the setting's resolution, and
/// quick (the same when it does not simulate).
struct Graphs {
    exact: Arc<Compiled>,
    quick: Arc<Compiled>,
}

impl Graphs {
    fn simulates(&self) -> bool {
        !Arc::ptr_eq(&self.exact, &self.quick)
    }
}

struct State {
    settings: Settings,
    source: Option<Source>,
    graphs: Option<Graphs>,
    /// When the graph was last set.
    changed: Instant,
    /// The view, in blocks from the terrain's corner, and its blocks a pixel.
    view: Option<(Rect, f64)>,
    /// Each version of the graph and of the settings is a generation.
    generation: u64,
    started: Instant,
    /// Tiles made in this generation, with exact simulations or quick.
    done: HashSet<(bool, TileKey)>,
    error: Option<String>,
}

/// A batch of tiles to make.
struct Job {
    generation: u64,
    view: u64,
    revision: u64,
    graph: Arc<Compiled>,
    exact: bool,
    phase: Phase,
    lod: u8,
    keys: Vec<(i32, i32)>,
    /// Each output made: its index in the settings and what feeds it.
    outputs: Vec<(usize, (usize, u8))>,
    on_gpu: bool,
}

enum Next {
    Job(Job),
    /// Nothing to do until something changes, or until then.
    Wait(Option<Instant>),
}

impl Live {
    /// Live over `engine` (shared with what else evaluates graphs, so that
    /// they find each other's tiles).
    pub fn new(engine: Arc<Engine>, settings: Settings) -> Self {
        let now = Instant::now();
        let shared = Arc::new(Shared {
            engine,
            state: Mutex::new(State { settings, source: None, graphs: None, changed: now, view: None, generation: 0, started: now, done: HashSet::new(), error: None }),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            view: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            ready: Mutex::new(Vec::new()),
            progress: Mutex::new(Progress { idle: true, ..Default::default() }),
            idle: Condvar::new(),
            notify: Mutex::new(None),
        });
        let worker = {
            let shared = shared.clone();
            std::thread::Builder::new().name("massif-live".to_owned()).spawn(move || work(&shared)).expect("a thread for Live")
        };
        Self { shared, worker: Some(worker) }
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.shared.engine
    }

    /// Shows `graph` over `space` (its own terrain settings win), reading
    /// the map and files through `env`; `revision` tags the tiles made from
    /// it. Gives what compiling it found wrong.
    pub fn set_graph(&self, graph: &Graph, space: Space, env: Arc<dyn Env>, revision: u64) -> Vec<Diagnostic> {
        let source = Source { graph: graph.clone(), space, env, revision };
        let mut st = self.shared.state.lock().expect("lock");
        let (graphs, diags) = compile(&source, &st.settings);
        st.source = Some(source);
        st.graphs = Some(graphs);
        st.changed = Instant::now();
        self.restart(&mut st);
        diags
    }

    /// Shows nothing.
    pub fn clear(&self) {
        let mut st = self.shared.state.lock().expect("lock");
        st.source = None;
        st.graphs = None;
        self.restart(&mut st);
    }

    /// The graph shown, compiled with its simulations at the setting's
    /// resolution: what previews of the same graph should evaluate, to find
    /// Live's tiles in the cache.
    pub fn compiled(&self) -> Option<Arc<Compiled>> {
        self.shared.state.lock().expect("lock").graphs.as_ref().map(|g| g.exact.clone())
    }

    /// The view: `rect` in blocks from the terrain's corner, `blocks_per_pixel`
    /// how many blocks a pixel of the screen spans.
    pub fn set_view(&self, rect: Rect, blocks_per_pixel: f64) {
        let mut st = self.shared.state.lock().expect("lock");
        if st.view == Some((rect, blocks_per_pixel)) {
            return;
        }
        st.view = Some((rect, blocks_per_pixel));
        self.shared.view.fetch_add(1, Ordering::Relaxed);
        self.busy();
        self.shared.wake.notify_all();
    }

    pub fn settings(&self) -> Settings {
        self.shared.state.lock().expect("lock").settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        let mut guard = self.shared.state.lock().expect("lock");
        let st = &mut *guard;
        if st.settings == settings {
            return;
        }
        let old = std::mem::replace(&mut st.settings, settings);
        let s = &st.settings;
        if (old.sim_res, old.quick_sim_res) != (s.sim_res, s.quick_sim_res)
            && let Some(source) = &st.source
        {
            st.graphs = Some(compile(source, s).0);
        }
        let s = &st.settings;
        if (old.sim_res, old.quick_sim_res, old.outputs.as_slice(), old.on_gpu) != (s.sim_res, s.quick_sim_res, s.outputs.as_slice(), s.on_gpu) {
            self.restart(st);
        } else {
            self.busy();
            self.shared.wake.notify_all();
        }
    }

    /// Stops making tiles until resumed: while the window is hidden, or the
    /// map being written.
    pub fn set_paused(&self, paused: bool) {
        let _st = self.shared.state.lock().expect("lock");
        self.shared.paused.store(paused, Ordering::Relaxed);
        if !paused {
            self.busy();
        }
        self.shared.wake.notify_all();
    }

    /// Calls `f` (on Live's thread) whenever tiles are ready: for a view to
    /// ask for a repaint.
    pub fn on_ready(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.shared.notify.lock().expect("lock") = Some(Arc::new(f));
    }

    /// The tiles made since last asked, in the order they were made.
    pub fn tiles_ready(&self) -> Vec<Ready> {
        std::mem::take(&mut *self.shared.ready.lock().expect("lock"))
    }

    pub fn progress(&self) -> Progress {
        self.shared.progress.lock().expect("lock").clone()
    }

    /// Waits until nothing is left to make, at most `timeout`; whether it is.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let p = self.shared.progress.lock().expect("lock");
        let (p, _) = self.shared.idle.wait_timeout_while(p, timeout, |p| !p.idle).expect("lock");
        p.idle
    }

    /// A new generation: everything is made again (found in the cache when
    /// it was made before).
    fn restart(&self, st: &mut State) {
        st.generation += 1;
        st.started = Instant::now();
        st.done.clear();
        st.error = None;
        self.shared.generation.store(st.generation, Ordering::Relaxed);
        *self.shared.progress.lock().expect("lock") = Progress { revision: st.source.as_ref().map_or(0, |s| s.revision), ..Default::default() };
        self.shared.wake.notify_all();
    }

    fn busy(&self) {
        self.shared.progress.lock().expect("lock").idle = false;
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        {
            let _st = self.shared.state.lock().expect("lock");
            self.shared.stop.store(true, Ordering::Relaxed);
            self.shared.wake.notify_all();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn compile(source: &Source, s: &Settings) -> (Graphs, Vec<Diagnostic>) {
    let (exact, diags) = compile_in(&source.graph, source.space, s.sim_res, source.env.clone());
    let simulates = exact.nodes.iter().any(|n| matches!(n.spec.kernel, Kernel::Global { .. }));
    let quick_res = s.quick_sim_res.min(s.sim_res);
    let exact = Arc::new(exact);
    let quick = if simulates && exact.space.lod_for(quick_res) != exact.sim_lod() { Arc::new(compile_in(&source.graph, source.space, quick_res, source.env.clone()).0) } else { exact.clone() };
    (Graphs { exact, quick }, diags)
}

/// The level of detail of a view: a sample a pixel or finer.
pub fn view_lod(blocks_per_pixel: f64) -> u8 {
    if blocks_per_pixel <= 1.0 || !blocks_per_pixel.is_finite() { 0 } else { (blocks_per_pixel.log2().floor() as u8).min(MAX_LOD) }
}

/// Tiles at `lod` covering `rect` (blocks), on the terrain, nearest its
/// centre first; and that centre in samples.
fn tiles_in(space: &Space, rect: Rect, lod: u8) -> (Vec<(i32, i32)>, (f64, f64)) {
    let r = rect.at_lod(0, lod).intersect(&space.whole(lod));
    let centre = (r.x as f64 + r.w as f64 / 2.0, r.z as f64 + r.h as f64 / 2.0);
    let mut tiles = r.tiles();
    by_distance(&mut tiles, centre);
    (tiles, centre)
}

fn by_distance(tiles: &mut [(i32, i32)], (cx, cz): (f64, f64)) {
    let t = TILE as f64;
    let d = |&(x, z): &(i32, i32)| {
        let (dx, dz) = ((x as f64 + 0.5) * t - cx, (z as f64 + 0.5) * t - cz);
        dx * dx + dz * dz
    };
    tiles.sort_by(|a, b| d(a).total_cmp(&d(b)).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
}

fn work(sh: &Shared) {
    let mut st = sh.state.lock().expect("lock");
    loop {
        if sh.stop.load(Ordering::Relaxed) {
            return;
        }
        match plan(&st, sh) {
            Next::Job(job) => {
                drop(st);
                let made = run(sh, &job);
                st = sh.state.lock().expect("lock");
                if finish(&mut st, sh, &job, made) {
                    drop(st);
                    let notify = sh.notify.lock().expect("lock").clone();
                    if let Some(f) = notify {
                        f();
                    }
                    st = sh.state.lock().expect("lock");
                }
            }
            Next::Wait(until) => {
                st = match until {
                    Some(t) => sh.wake.wait_timeout(st, t.saturating_duration_since(Instant::now())).expect("lock").0,
                    None => sh.wake.wait(st).expect("lock"),
                };
            }
        }
    }
}

/// The next batch to make, nearest first in the order of the phases.
fn plan(st: &State, sh: &Shared) -> Next {
    let idle = |finished: bool| {
        let mut p = sh.progress.lock().expect("lock");
        p.phase = None;
        p.idle = true;
        if finished && p.finished.is_none() {
            p.finished = Some(st.started.elapsed());
        }
        sh.idle.notify_all();
        Next::Wait(None)
    };
    let s = &st.settings;
    let Some(g) = &st.graphs else { return idle(false) };
    if s.mode == Mode::Off || sh.paused.load(Ordering::Relaxed) || st.error.is_some() {
        return idle(false);
    }
    let outputs: Vec<(usize, (usize, u8))> = s.outputs.iter().enumerate().filter_map(|(i, name)| g.exact.output(name).map(|src| (i, src))).collect();
    if outputs.is_empty() {
        return idle(false);
    }
    let space = g.exact.space;
    let rested = st.changed.elapsed() >= s.rest;
    let simulates = g.simulates();
    let view = st.view.map(|(rect, bpp)| {
        let lod = view_lod(bpp);
        let (tiles, centre) = tiles_in(&space, rect, lod);
        (lod, tiles, centre)
    });
    let overview = space.lod_for(s.overview);
    let mut stages = vec![(Phase::Visible, !simulates), (Phase::Overview, !simulates)];
    if rested {
        if simulates {
            stages.extend([(Phase::Visible, true), (Phase::Overview, true)]);
        }
        if s.mode == Mode::Whole {
            stages.push((Phase::Rest, true));
        }
    }
    for (phase, exact) in stages {
        let (lod, tiles) = match phase {
            Phase::Visible => match &view {
                Some((lod, tiles, _)) => (*lod, tiles.clone()),
                None => continue,
            },
            Phase::Overview => (overview, space.whole(overview).tiles()),
            Phase::Rest => {
                let Some((at, _, centre)) = &view else { continue };
                let lod = (*at).max(space.lod_for(s.whole_res));
                let k = Space::spacing(lod) / Space::spacing(*at);
                let mut tiles = space.whole(lod).tiles();
                by_distance(&mut tiles, (centre.0 / k, centre.1 / k));
                (lod, tiles)
            }
        };
        let total = tiles.len();
        let left: Vec<(i32, i32)> = tiles.into_iter().filter(|&(x, z)| !st.done.contains(&(exact, TileKey::new(lod, x, z)))).collect();
        let mut p = sh.progress.lock().expect("lock");
        if left.is_empty() {
            if phase == Phase::Visible && p.visible.is_none() {
                p.visible = Some(st.started.elapsed());
            }
            continue;
        }
        p.phase = Some((phase, exact));
        p.done = total - left.len();
        p.total = total;
        p.idle = false;
        drop(p);
        let threads = rayon::current_num_threads();
        let batch = match phase {
            Phase::Visible if st.done.is_empty() => threads.clamp(4, 16),
            Phase::Rest => 32,
            _ => 16,
        };
        let revision = st.source.as_ref().map_or(0, |s| s.revision);
        let graph = if exact { g.exact.clone() } else { g.quick.clone() };
        return Next::Job(Job { generation: st.generation, view: sh.view.load(Ordering::Relaxed), revision, graph, exact, phase, lod, keys: left[..left.len().min(batch)].to_vec(), outputs, on_gpu: s.on_gpu });
    }
    if !rested {
        return Next::Wait(Some(st.changed + s.rest));
    }
    idle(true)
}

/// Makes a batch: each output's tiles.
fn run(sh: &Shared, job: &Job) -> Result<Vec<Ready>, EvalError> {
    let cancel = || sh.stop.load(Ordering::Relaxed) || sh.paused.load(Ordering::Relaxed) || sh.generation.load(Ordering::Relaxed) != job.generation || (job.phase == Phase::Rest && sh.view.load(Ordering::Relaxed) != job.view);
    let mut made = Vec::with_capacity(job.keys.len() * job.outputs.len());
    for &(i, src) in &job.outputs {
        let tiles = if job.on_gpu { sh.engine.gpu_tiles(&job.graph, src, job.lod, &job.keys, &cancel)? } else { sh.engine.tiles(&job.graph, src, job.lod, &job.keys, &cancel)? };
        made.extend(job.keys.iter().zip(tiles).map(|(&(x, z), tile)| Ready { revision: job.revision, output: i, key: TileKey::new(job.lod, x, z), tile, phase: job.phase, exact: job.exact }));
    }
    Ok(made)
}

/// Keeps what a batch made, if it is still wanted; whether tiles are ready.
fn finish(st: &mut State, sh: &Shared, job: &Job, made: Result<Vec<Ready>, EvalError>) -> bool {
    if job.generation != st.generation {
        return false;
    }
    match made {
        Ok(made) => {
            for &(x, z) in &job.keys {
                st.done.insert((job.exact, TileKey::new(job.lod, x, z)));
            }
            if job.phase == Phase::Visible {
                let mut p = sh.progress.lock().expect("lock");
                if p.first_visible.is_none() {
                    p.first_visible = Some(st.started.elapsed());
                }
            }
            sh.ready.lock().expect("lock").extend(made);
            true
        }
        Err(EvalError::Cancelled) => false,
        Err(e) => {
            let why = e.to_string();
            st.error = Some(why.clone());
            sh.progress.lock().expect("lock").error = Some(why);
            false
        }
    }
}
