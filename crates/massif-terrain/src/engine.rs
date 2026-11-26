//! Evaluating a graph: by tiles, at any level of detail, with a cache.
//!
//! A request asks for tiles of one node's output at one level of detail.
//! The engine plans it first, walking back from that node: each tile it
//! lacks needs tiles of what feeds it — its inputs and the fields driving
//! its settings — the same tiles for a point node, the tiles around them
//! for a local one (at the level its footprint says), down to tiles in the
//! cache or nodes with no inputs. Then it computes node after node, all the
//! tiles a node needs at once (in parallel on the CPU, or in one submission
//! on the graphics card), so nothing ever waits on a lock while computing.
//!
//! Every tile is cached under the hash of its node — its type, settings,
//! inputs' hashes and what it reads outside the graph — with its output,
//! level and place: changing a setting changes the hash of that node and of
//! those after it only, whose tiles are then made again, and undoing it
//! finds the old ones. A simulation is made over the whole terrain at the
//! simulation's level, cached whole, and its tiles at finer levels are the
//! input's tiles with the change the simulation made, resampled (or its
//! outputs resampled).
//!
//! Tiles stay where they were made (the CPU or the graphics card) until
//! something needs them elsewhere: [`Engine::tiles`] gives them on the CPU,
//! [`Engine::gpu_tiles`] on the card, for views that draw from it.

use crate::compile::Compiled;
use crate::gpu::{Gpu, GpuGrid, GpuTile, Job};
use crate::kernel::{BRICK, Brick, Geo, Grid, Stats};
use crate::registry::Kernel;
use crate::space::{Rect, Space, TILE};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Says to stop: checked between tiles and between steps of simulations.
pub type Cancel<'a> = &'a (dyn Fn() -> bool + Sync);

/// Never stop.
pub fn never() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    #[error("the evaluation was cancelled")]
    Cancelled,
    #[error("“{node}” cannot be evaluated: {why}")]
    Broken { node: String, why: String },
}

/// Samples of one tile, on the CPU, on the graphics card, or both; each
/// value of a sample (a colour's channels) is a plane of TILE² samples.
#[derive(Debug, Default)]
pub struct TileData {
    planes: usize,
    cpu: OnceLock<Arc<[f32]>>,
    gpu: OnceLock<GpuTile>,
}

impl TileData {
    pub fn cpu(data: Vec<f32>) -> Self {
        let t = Self { planes: (data.len() / (TILE * TILE) as usize).max(1), ..Default::default() };
        let _ = t.cpu.set(data.into());
        t
    }

    fn gpu(tile: GpuTile, planes: usize) -> Self {
        let t = Self { planes, ..Default::default() };
        let _ = t.gpu.set(tile);
        t
    }

    pub fn planes(&self) -> usize {
        self.planes
    }

    /// The samples, planes one after the other, each row by row, when they
    /// are on the CPU (every tile [`Engine::tiles`] gives back is).
    pub fn values(&self) -> Option<&[f32]> {
        self.cpu.get().map(|v| &v[..])
    }

    /// The tile's buffer on the graphics card, when it is there (every tile
    /// [`Engine::gpu_tiles`] gives back is, when the engine has a card).
    pub fn gpu_buffer(&self) -> Option<&wgpu::Buffer> {
        self.gpu.get().map(GpuTile::buffer)
    }
}

/// How the engine works.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Memory the cache may hold, in megabytes.
    pub cache_mb: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { cache_mb: 2048 }
    }
}

/// What the engine did, for timings and tests.
#[derive(Debug, Default)]
pub struct Counters {
    /// Tiles computed, by any kernel.
    pub tiles: AtomicU64,
    /// Tiles computed on the graphics card.
    pub gpu_tiles: AtomicU64,
    /// Tiles found in the cache.
    pub hits: AtomicU64,
    /// Simulations run over the whole terrain (on the graphics card: `gpu_globals`).
    pub globals: AtomicU64,
    pub gpu_globals: AtomicU64,
    /// Time spent by each node, by node name: tiles made and milliseconds
    /// (a simulation's time counts the inputs it evaluated).
    pub by_node: Mutex<HashMap<String, (u64, f64)>>,
}

impl Counters {
    pub fn tiles(&self) -> u64 {
        self.tiles.load(Ordering::Relaxed)
    }

    /// Tiles a node made.
    pub fn of(&self, node: &str) -> u64 {
        self.by_node.lock().expect("lock").get(node).map_or(0, |v| v.0)
    }

    fn add(&self, node: &str, tiles: usize, ms: f64) {
        let mut m = self.by_node.lock().expect("lock");
        let e = m.entry(node.to_owned()).or_default();
        e.0 += tiles as u64;
        e.1 += ms;
    }
}

/// What the cache keeps, under the hash of the node that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    /// A tile of an output.
    Tile { hash: u64, port: u8, lod: u8, x: i32, z: i32 },
    /// A simulation's output over the whole terrain at its level (the
    /// change it made, for the outputs it carries).
    Whole { hash: u64, port: u8, lod: u8 },
    /// Figures over a whole output.
    Stats { hash: u64, port: u8, lod: u8 },
    /// Geometry a node made, whole.
    Geo { hash: u64, port: u8 },
    /// A brick of a volume, by its west, bottom, north block.
    Brick { hash: u64, port: u8, x: i32, y: i32, z: i32 },
}

#[derive(Clone)]
enum Entry {
    Tile(Arc<TileData>),
    Whole(Arc<Grid>),
    Stats(Stats),
    Geo(Arc<Geo>),
    Brick(Arc<Brick>),
}

impl Entry {
    fn bytes(&self) -> usize {
        match self {
            Entry::Tile(t) => (TILE * TILE * 4) as usize * t.planes.max(1),
            Entry::Whole(g) => g.data.len() * 4,
            Entry::Stats(_) => 64,
            Entry::Geo(g) => geo_bytes(g),
            Entry::Brick(b) => b.data.len() * 4 + 32,
        }
    }
}

/// Least recently used goes first.
#[derive(Default)]
struct Cache {
    map: HashMap<Key, (Entry, u64)>,
    order: BTreeMap<u64, Key>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl Cache {
    fn get(&mut self, key: &Key) -> Option<Entry> {
        self.tick += 1;
        let tick = self.tick;
        let (entry, last) = self.map.get_mut(key)?;
        self.order.remove(last);
        *last = tick;
        self.order.insert(tick, *key);
        Some(entry.clone())
    }

    fn put(&mut self, key: Key, entry: Entry) {
        self.tick += 1;
        self.bytes += entry.bytes();
        if let Some((old, last)) = self.map.insert(key, (entry, self.tick)) {
            self.bytes -= old.bytes();
            self.order.remove(&last);
        }
        self.order.insert(self.tick, key);
        if self.bytes <= self.budget {
            return;
        }
        // Tiles go first, oldest first: a simulation's whole result or a
        // node's geometry costs far more to make again than a tile, and a
        // map written window by window needs them for every window.
        let tiles: Vec<(u64, Key)> = self.order.iter().filter(|(_, k)| matches!(k, Key::Tile { .. } | Key::Brick { .. })).map(|(t, k)| (*t, *k)).collect();
        for (tick, oldest) in tiles {
            if self.bytes <= self.budget {
                return;
            }
            self.order.remove(&tick);
            if let Some((e, _)) = self.map.remove(&oldest) {
                self.bytes -= e.bytes();
            }
        }
        while self.bytes > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else { break };
            if let Some((e, _)) = self.map.remove(&oldest) {
                self.bytes -= e.bytes();
            }
        }
    }
}

/// A tile of a node's output: node, output, level, place.
type At = (usize, u8, u8, i32, i32);

/// Where a local kernel read each input: level and samples.
type Reads = Vec<Option<(u8, Rect)>>;

/// What a tile of a node reads of something feeding it: whose output, at
/// what level, which samples.
#[derive(Debug, Clone, Copy)]
struct Feed {
    src: (usize, u8),
    lod: u8,
    rect: Rect,
}

pub struct Engine {
    cache: Mutex<Cache>,
    gpu: Option<Arc<Gpu>>,
    pub counters: Counters,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Engine({})", self.gpu_name().unwrap_or("CPU"))
    }
}

impl Engine {
    /// An engine on the CPU only.
    pub fn new(settings: Settings) -> Self {
        Self { cache: Mutex::new(Cache { budget: settings.cache_mb << 20, ..Default::default() }), gpu: None, counters: Counters::default() }
    }

    /// An engine running what it can on `gpu`.
    pub fn with_gpu(gpu: Arc<Gpu>, settings: Settings) -> Self {
        let mut e = Self::new(settings);
        e.gpu = Some(gpu);
        e
    }

    /// The graphics card it runs on, if any.
    pub fn gpu(&self) -> Option<&Arc<Gpu>> {
        self.gpu.as_ref()
    }

    pub fn gpu_name(&self) -> Option<&str> {
        self.gpu.as_deref().map(Gpu::name)
    }

    /// Forgets everything computed.
    pub fn clear(&self) {
        let mut c = self.cache.lock().expect("lock");
        let budget = c.budget;
        *c = Cache { budget, ..Default::default() };
    }

    /// Bytes the cache holds.
    pub fn cache_bytes(&self) -> usize {
        self.cache.lock().expect("lock").bytes
    }

    fn get(&self, key: &Key) -> Option<Entry> {
        self.cache.lock().expect("lock").get(key)
    }

    fn put(&self, key: Key, entry: Entry) {
        self.cache.lock().expect("lock").put(key, entry);
    }

    /// The samples of `rect` at `lod` of output `out` (node and output, as
    /// [`Compiled::source`] or [`Compiled::output`] give them).
    pub fn region(&self, c: &Compiled, out: (usize, u8), lod: u8, rect: Rect, cancel: Cancel) -> Result<Grid, EvalError> {
        let keys = rect.tiles();
        let tiles = self.tiles(c, out, lod, &keys, cancel)?;
        let mut grid = Grid::new(lod, rect, vec![0.0; rect.area() * c.kind(resolve(c, out)).planes()]);
        for ((tx, tz), t) in keys.iter().zip(&tiles) {
            copy_into(&mut grid, Rect::tile(*tx, *tz), t.values().expect("tiles given back are on the CPU"));
        }
        Ok(grid)
    }

    /// The whole terrain at `lod` of output `out`.
    pub fn whole(&self, c: &Compiled, out: (usize, u8), lod: u8, cancel: Cancel) -> Result<Grid, EvalError> {
        self.region(c, out, lod, c.space.whole(lod), cancel)
    }

    /// Tiles `keys` of output `out` at `lod`, their samples on the CPU.
    pub fn tiles(&self, c: &Compiled, out: (usize, u8), lod: u8, keys: &[(i32, i32)], cancel: Cancel) -> Result<Vec<Arc<TileData>>, EvalError> {
        let tiles = self.evaluate(c, out, lod, keys, cancel)?;
        self.to_cpu(&tiles.iter().map(|t| &**t).collect::<Vec<_>>());
        Ok(tiles)
    }

    /// Tiles `keys` of output `out` at `lod`, their samples on the graphics
    /// card ([`TileData::gpu_buffer`]); on the CPU when the engine has no
    /// card.
    pub fn gpu_tiles(&self, c: &Compiled, out: (usize, u8), lod: u8, keys: &[(i32, i32)], cancel: Cancel) -> Result<Vec<Arc<TileData>>, EvalError> {
        let tiles = self.evaluate(c, out, lod, keys, cancel)?;
        let refs: Vec<&TileData> = tiles.iter().map(|t| &**t).collect();
        match &self.gpu {
            Some(_) => self.to_gpu(&refs),
            None => self.to_cpu(&refs),
        }
        Ok(tiles)
    }

    /// Plans and makes tiles `keys` of `out` at `lod`, wherever they are made.
    fn evaluate(&self, c: &Compiled, out: (usize, u8), lod: u8, keys: &[(i32, i32)], cancel: Cancel) -> Result<Vec<Arc<TileData>>, EvalError> {
        Ok(self.evaluate_many(c, &[out], lod, keys, cancel)?.pop().expect("one output asked"))
    }

    /// Tiles `keys` at `lod` of several outputs, planned together: what
    /// they share upstream is made once, whatever the cache keeps.
    pub fn regions(&self, c: &Compiled, outs: &[(usize, u8)], lod: u8, rect: Rect, cancel: Cancel) -> Result<Vec<Grid>, EvalError> {
        let keys = rect.tiles();
        let all = self.evaluate_many(c, outs, lod, &keys, cancel)?;
        let mut grids = Vec::with_capacity(outs.len());
        for (&out, tiles) in outs.iter().zip(all) {
            self.to_cpu(&tiles.iter().map(|t| &**t).collect::<Vec<_>>());
            let mut grid = Grid::new(lod, rect, vec![0.0; rect.area() * c.kind(resolve(c, out)).planes()]);
            for ((tx, tz), t) in keys.iter().zip(&tiles) {
                copy_into(&mut grid, Rect::tile(*tx, *tz), t.values().expect("tiles given back are on the CPU"));
            }
            grids.push(grid);
        }
        Ok(grids)
    }

    /// Plans and makes tiles `keys` at `lod` of each of `outs`, wherever
    /// they are made.
    fn evaluate_many(&self, c: &Compiled, outs: &[(usize, u8)], lod: u8, keys: &[(i32, i32)], cancel: Cancel) -> Result<Vec<Vec<Arc<TileData>>>, EvalError> {
        // What each output really is: an output node gives what feeds it.
        let mut roots = Vec::with_capacity(outs.len());
        for &out in outs {
            let node = &c.nodes[out.0];
            if node.spec.is_output() && node.inputs.first().copied().flatten().is_none() {
                return Err(EvalError::Broken { node: node.id.clone(), why: node.broken.clone().unwrap_or_else(|| "nothing feeds it".to_owned()) });
            }
            let out = resolve(c, out);
            let kind = c.kind(out);
            if !kind.is_raster() {
                let why = if kind.is_geometry() { "its geometry is made whole (Engine::geometry), not by tiles" } else { "its volume is made by bricks (Engine::bricks), not by tiles" };
                return Err(EvalError::Broken { node: c.nodes[out.0].id.clone(), why: why.to_owned() });
            }
            roots.push(out);
        }
        let Some(root) = roots.iter().map(|r| r.0).max() else { return Ok(Vec::new()) };

        // Plan: what each node must make, walking back from the root.
        let mut need: Vec<BTreeSet<(u8, i32, i32)>> = vec![BTreeSet::new(); root + 1];
        let mut todo: Vec<Vec<(u8, i32, i32)>> = vec![Vec::new(); root + 1];
        let mut have: HashMap<At, Arc<TileData>> = HashMap::new();
        for &(r, _) in &roots {
            need[r].extend(keys.iter().map(|&(x, z)| (lod, x, z)));
        }
        for n in (0..=root).rev() {
            if need[n].is_empty() {
                continue;
            }
            let node = &c.nodes[n];
            if let Some(why) = &node.broken {
                return Err(EvalError::Broken { node: node.id.clone(), why: why.clone() });
            }
            for (l, x, z) in std::mem::take(&mut need[n]) {
                if let Some(found) = self.cached(c, n, l, x, z) {
                    self.counters.hits.fetch_add(1, Ordering::Relaxed);
                    for (p, t) in found.into_iter().enumerate() {
                        have.insert((n, p as u8, l, x, z), t);
                    }
                    continue;
                }
                todo[n].push((l, x, z));
                for f in feeds(c, n, l, x, z).into_iter().flatten() {
                    for (tx, tz) in f.rect.tiles() {
                        need[f.src.0].insert((f.lod, tx, tz));
                    }
                }
            }
        }

        // Make: node after node.
        for (n, todo) in todo.iter().enumerate() {
            if todo.is_empty() {
                continue;
            }
            if cancel() {
                return Err(EvalError::Cancelled);
            }
            let started = std::time::Instant::now();
            let made = self.make(c, n, todo, &have, cancel)?;
            self.counters.add(&c.nodes[n].id, todo.len(), started.elapsed().as_secs_f64() * 1000.0);
            self.counters.tiles.fetch_add(todo.len() as u64, Ordering::Relaxed);
            let hash = c.nodes[n].hash;
            for (&(l, x, z), outs) in todo.iter().zip(made) {
                for (p, t) in outs.into_iter().enumerate() {
                    self.put(Key::Tile { hash, port: p as u8, lod: l, x, z }, Entry::Tile(t.clone()));
                    have.insert((n, p as u8, l, x, z), t);
                }
            }
        }
        Ok(roots.iter().map(|&(r, port)| keys.iter().map(|&(x, z)| have[&(r, port, lod, x, z)].clone()).collect()).collect())
    }

    /// All of a node's outputs at a tile, when the cache has them.
    fn cached(&self, c: &Compiled, n: usize, lod: u8, x: i32, z: i32) -> Option<Vec<Arc<TileData>>> {
        let node = &c.nodes[n];
        let mut cache = self.cache.lock().expect("lock");
        (0..node.spec.outputs.len())
            .map(|p| match cache.get(&Key::Tile { hash: node.hash, port: p as u8, lod, x, z }) {
                Some(Entry::Tile(t)) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Makes `tiles` of node `n`, what feeds them in `have`.
    fn make(&self, c: &Compiled, n: usize, tiles: &[(u8, i32, i32)], have: &HashMap<At, Arc<TileData>>, cancel: Cancel) -> Result<Vec<Vec<Arc<TileData>>>, EvalError> {
        let node = &c.nodes[n];
        let geos = self.geos(c, n, cancel)?;
        let area = (TILE * TILE) as usize;
        let planes: Vec<usize> = node.spec.outputs.iter().map(|o| o.kind.planes()).collect();
        let fresh = || planes.iter().map(|p| vec![0.0f32; area * p]).collect::<Vec<_>>();
        let ninputs = node.inputs.len();
        // A tile fed at its own place: point kernels' inputs and driving fields.
        // (Geometry is no tile: kernels read it through `Ctx::geo`.)
        let same = |src: Option<(usize, u8)>, l: u8, x: i32, z: i32| src.filter(|&s| c.kind(s).is_raster()).map(|(s, p)| &have[&(s, p, l, x, z)]);
        let driving = |i: usize| node.driven.get(i).copied().flatten();
        let done = |outs: Vec<Vec<f32>>| outs.into_iter().map(|o| Arc::new(TileData::cpu(o))).collect::<Vec<_>>();
        match node.spec.kernel {
            Kernel::Point { cpu, gpu } => {
                if let (Some(g), Some(kernel)) = (&self.gpu, gpu)
                    && g.takes(kernel.name)
                {
                    // Bound after the uniforms: the inputs, then each drivable setting.
                    let drivable: Vec<usize> = node.spec.params.iter().enumerate().filter(|(_, p)| p.drivable).map(|(i, _)| i).collect();
                    let sources: Vec<Option<(usize, u8)>> = node.inputs.iter().copied().chain(drivable.iter().map(|&i| driving(i))).collect();
                    let connected: Vec<bool> = sources.iter().map(Option::is_some).collect();
                    let fed: Vec<Vec<Option<&TileData>>> = tiles.iter().map(|&(l, x, z)| sources.iter().map(|s| same(*s, l, x, z).map(|t| &**t)).collect()).collect();
                    self.to_gpu(&fed.iter().flatten().flatten().copied().collect::<Vec<_>>());
                    let jobs: Vec<Job> = tiles
                        .iter()
                        .zip(&fed)
                        .map(|(&(l, x, z), row)| ((kernel.uniforms)(&ctx_of(c, &geos, n, l, Rect::tile(x, z), &[], &[]), &connected), row.iter().map(|t| t.and_then(TileData::gpu_buffer)).collect()))
                        .collect();
                    if let Some(made) = g.run_tiles(kernel.name, kernel.source, &jobs, &planes) {
                        self.counters.gpu_tiles.fetch_add(tiles.len() as u64, Ordering::Relaxed);
                        return Ok(made.into_iter().map(|outs| outs.into_iter().zip(&planes).map(|(t, p)| Arc::new(TileData::gpu(t, *p))).collect()).collect());
                    }
                }
                self.feeds_to_cpu(c, n, tiles, have);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let ins: Vec<Option<&[f32]>> = (0..ninputs).map(|k| same(node.inputs[k], l, x, z).map(|t| t.values().expect("fed on the CPU"))).collect();
                        let driven: Vec<Option<&[f32]>> = (0..node.spec.params.len()).map(|i| same(driving(i), l, x, z).map(|t| t.values().expect("fed on the CPU"))).collect();
                        let mut outs = fresh();
                        cpu(&ctx_of(c, &geos, n, l, Rect::tile(x, z), &driven, &[]), &ins, &mut outs);
                        Ok(done(outs))
                    })
                    .collect()
            }
            Kernel::Stats { cpu } => {
                let stats: Vec<Option<Stats>> = node.inputs.iter().map(|i| i.map(|src| self.stats(c, src, cancel)).transpose()).collect::<Result<_, _>>()?;
                self.feeds_to_cpu(c, n, tiles, have);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let ins: Vec<Option<&[f32]>> = (0..ninputs).map(|k| same(node.inputs[k], l, x, z).map(|t| t.values().expect("fed on the CPU"))).collect();
                        let mut outs = fresh();
                        cpu(&ctx_of(c, &geos, n, l, Rect::tile(x, z), &[], &[]), &stats, &ins, &mut outs);
                        Ok(done(outs))
                    })
                    .collect()
            }
            Kernel::Local { cpu, gpu, .. } => {
                self.feeds_to_cpu(c, n, tiles, have);
                let regions = |l: u8, x: i32, z: i32| -> (Reads, Vec<Option<Grid>>) {
                    let fs = feeds(c, n, l, x, z);
                    let reads = fs[..ninputs].iter().map(|f| f.map(|f| (f.lod, f.rect))).collect();
                    let grids = fs[..ninputs].iter().map(|f| f.map(|f| assemble(have, f.src, f.lod, f.rect))).collect();
                    (reads, grids)
                };
                if let (Some(g), Some(kernel)) = (&self.gpu, gpu)
                    && g.takes(kernel.name)
                {
                    // Bound after the uniforms: the inputs' regions, then each
                    // drivable setting's field at the tile.
                    let drivable: Vec<usize> = node.spec.params.iter().enumerate().filter(|(_, p)| p.drivable).map(|(i, _)| i).collect();
                    let connected: Vec<bool> = node.inputs.iter().map(Option::is_some).chain(drivable.iter().map(|&i| driving(i).is_some())).collect();
                    let fields: Vec<&TileData> = tiles.iter().flat_map(|&(l, x, z)| drivable.iter().filter_map(move |&i| same(driving(i), l, x, z).map(|t| &**t))).collect();
                    self.to_gpu(&fields);
                    let prepared: Vec<(Vec<u8>, Vec<Option<wgpu::Buffer>>)> = tiles
                        .iter()
                        .map(|&(l, x, z)| {
                            let (reads, grids) = regions(l, x, z);
                            let uniforms = (kernel.uniforms)(&ctx_of(c, &geos, n, l, Rect::tile(x, z), &[], &reads), &connected);
                            let mut bound: Vec<Option<wgpu::Buffer>> = grids.iter().map(|gr| gr.as_ref().map(|gr| g.upload_grid(gr).buffer)).collect();
                            bound.extend(drivable.iter().map(|&i| same(driving(i), l, x, z).and_then(|t| t.gpu_buffer().cloned())));
                            (uniforms, bound)
                        })
                        .collect();
                    let jobs: Vec<Job> = prepared.iter().map(|(u, b)| (u.clone(), b.iter().map(Option::as_ref).collect())).collect();
                    if let Some(made) = g.run_tiles(kernel.name, kernel.source, &jobs, &planes) {
                        self.counters.gpu_tiles.fetch_add(tiles.len() as u64, Ordering::Relaxed);
                        return Ok(made.into_iter().map(|outs| outs.into_iter().zip(&planes).map(|(t, p)| Arc::new(TileData::gpu(t, *p))).collect()).collect());
                    }
                }
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let (reads, grids) = regions(l, x, z);
                        let ins: Vec<Option<&Grid>> = grids.iter().map(Option::as_ref).collect();
                        let driven: Vec<Option<&[f32]>> = (0..node.spec.params.len()).map(|i| same(driving(i), l, x, z).map(|t| t.values().expect("fed on the CPU"))).collect();
                        let mut outs = fresh();
                        cpu(&ctx_of(c, &geos, n, l, Rect::tile(x, z), &driven, &reads), &ins, &mut outs);
                        Ok(done(outs))
                    })
                    .collect()
            }
            Kernel::Global { carry, .. } => {
                let lods: BTreeSet<u8> = tiles.iter().map(|t| t.0).collect();
                let mut wholes: HashMap<u8, Vec<Arc<Grid>>> = HashMap::new();
                for l in lods {
                    wholes.insert(l, self.global(c, n, l.max(c.sim_lod()), cancel)?);
                }
                self.feeds_to_cpu(c, n, tiles, have);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let run = l.max(c.sim_lod());
                        let f = Space::spacing(l) / Space::spacing(run);
                        let rect = Rect::tile(x, z);
                        let outs = (0..planes.len())
                            .map(|p| {
                                let g = &wholes[&l][p];
                                let kind = node.spec.outputs[p].kind;
                                let base = carry[p].and_then(|k| same(node.inputs[k as usize], l, x, z)).map(|t| t.values().expect("fed on the CPU"));
                                let mut o = vec![0.0f32; area * planes[p]];
                                for q in 0..planes[p] {
                                    for (j, row) in o[q * area..(q + 1) * area].chunks_mut(TILE as usize).enumerate() {
                                        let v = (rect.z + j as i32) as f64 * f;
                                        for (i, s) in row.iter_mut().enumerate() {
                                            let u = (rect.x + i as i32) as f64 * f;
                                            let d = if run == l {
                                                g.at_plane(q, rect.x + i as i32, rect.z + j as i32)
                                            } else if kind.categorical() {
                                                g.nearest(q, u, v)
                                            } else {
                                                g.bicubic_plane(q, u, v)
                                            };
                                            *s = match base {
                                                Some(b) => b[q * area + j * TILE as usize + i] + d,
                                                None => d,
                                            };
                                        }
                                    }
                                }
                                Arc::new(TileData::cpu(o))
                            })
                            .collect();
                        Ok(outs)
                    })
                    .collect()
            }
            Kernel::Output => unreachable!("outputs are not made"),
            Kernel::Geometry { .. } | Kernel::Volume { .. } => unreachable!("not made by tiles"),
        }
    }

    /// For each input of node `n`, the geometry feeding it, made whole;
    /// empty when none of its inputs is geometry.
    fn geos(&self, c: &Compiled, n: usize, cancel: Cancel) -> Result<Vec<Option<Arc<Geo>>>, EvalError> {
        let node = &c.nodes[n];
        if !node.inputs.iter().flatten().any(|&src| c.kind(src).is_geometry()) {
            return Ok(Vec::new());
        }
        node.inputs.iter().map(|i| i.filter(|&src| c.kind(src).is_geometry()).map(|src| self.geometry(c, src, cancel)).transpose()).collect()
    }

    /// Bricks of volume output `out`, each at its west, bottom, north
    /// block (a multiple of 16 from the world's origin), on every core;
    /// each brick from the same bricks of its volume inputs and, for its
    /// raster inputs, the columns under it at one sample a block with the
    /// node's halo around (cut to the terrain).
    pub fn bricks(&self, c: &Compiled, out: (usize, u8), origins: &[[i32; 3]], cancel: Cancel) -> Result<Vec<Arc<Brick>>, EvalError> {
        let out = resolve(c, out);
        if let Some(o) = origins.iter().find(|o| o.iter().any(|v| v.rem_euclid(BRICK) != 0)) {
            return Err(EvalError::Broken { node: c.nodes[out.0].id.clone(), why: format!("a brick starts on a multiple of {BRICK}, not at {o:?}") });
        }
        origins.par_iter().map(|&o| self.brick(c, out, o, cancel)).collect()
    }

    /// One brick of volume output `out`, from the cache or made.
    fn brick(&self, c: &Compiled, out: (usize, u8), origin: [i32; 3], cancel: Cancel) -> Result<Arc<Brick>, EvalError> {
        let (n, port) = out;
        let node = &c.nodes[n];
        if let Some(why) = &node.broken {
            return Err(EvalError::Broken { node: node.id.clone(), why: why.clone() });
        }
        let Kernel::Volume { cpu, halo } = node.spec.kernel else {
            return Err(EvalError::Broken { node: node.id.clone(), why: "it makes no volume".to_owned() });
        };
        let [x, y, z] = origin;
        if let Some(Entry::Brick(b)) = self.get(&Key::Brick { hash: node.hash, port, x, y, z }) {
            self.counters.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(b);
        }
        if cancel() {
            return Err(EvalError::Cancelled);
        }
        let geos = self.geos(c, n, cancel)?;
        // The columns under the brick, in samples of level 0 from the
        // terrain's corner.
        let under = Rect::new(x - c.space.x0 - halo as i32, z - c.space.z0 - halo as i32, BRICK as u32 + 2 * halo, BRICK as u32 + 2 * halo).intersect(&c.space.whole(0));
        let mut rasters: Vec<Option<Grid>> = Vec::with_capacity(node.inputs.len());
        let mut volumes: Vec<Option<Arc<Brick>>> = Vec::with_capacity(node.inputs.len());
        for i in &node.inputs {
            let kind = i.map(|src| c.kind(src));
            rasters.push(match (i, kind) {
                (Some(src), Some(k)) if k.is_raster() && !under.is_empty() => Some(self.region(c, *src, 0, under, cancel)?),
                _ => None,
            });
            volumes.push(match (i, kind) {
                (Some(src), Some(k)) if k.is_volume() => Some(self.brick(c, resolve(c, *src), origin, cancel)?),
                _ => None,
            });
        }
        let rs: Vec<Option<&Grid>> = rasters.iter().map(Option::as_ref).collect();
        let vs: Vec<Option<&Brick>> = volumes.iter().map(|v| v.as_deref()).collect();
        let mut outs: Vec<Brick> = node.spec.outputs.iter().map(|_| Brick::filled(origin, 1.0)).collect();
        cpu(&ctx_of(c, &geos, n, 0, under, &[], &[]), origin, &rs, &vs, &mut outs);
        self.counters.tiles.fetch_add(1, Ordering::Relaxed);
        let made: Vec<Arc<Brick>> = outs.into_iter().map(Arc::new).collect();
        for (p, b) in made.iter().enumerate() {
            self.put(Key::Brick { hash: node.hash, port: p as u8, x, y, z }, Entry::Brick(b.clone()));
        }
        Ok(made[port as usize].clone())
    }

    /// Geometry output `out` (points, splines, footprints, instances),
    /// made whole: its raster inputs read over the whole terrain at the
    /// simulations' level, its geometry inputs whole.
    pub fn geometry(&self, c: &Compiled, out: (usize, u8), cancel: Cancel) -> Result<Arc<Geo>, EvalError> {
        let out = resolve(c, out);
        let (n, port) = out;
        let node = &c.nodes[n];
        if let Some(why) = &node.broken {
            return Err(EvalError::Broken { node: node.id.clone(), why: why.clone() });
        }
        let Kernel::Geometry { cpu } = node.spec.kernel else {
            return Err(EvalError::Broken { node: node.id.clone(), why: "it makes no geometry".to_owned() });
        };
        if let Some(Entry::Geo(g)) = self.get(&Key::Geo { hash: node.hash, port }) {
            self.counters.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(g);
        }
        let run = c.sim_lod();
        let whole = c.space.whole(run);
        let geos = self.geos(c, n, cancel)?;
        let rasters: Vec<Option<Grid>> = node.inputs.iter().map(|i| i.filter(|&src| c.kind(src).is_raster()).map(|src| self.region(c, src, run, whole, cancel)).transpose()).collect::<Result<_, _>>()?;
        let refs: Vec<Option<&Grid>> = rasters.iter().map(Option::as_ref).collect();
        let started = std::time::Instant::now();
        let made = cpu(&ctx_of(c, &geos, n, run, whole, &[], &[]), &refs, cancel).map_err(|why| if cancel() { EvalError::Cancelled } else { EvalError::Broken { node: node.id.clone(), why } })?;
        self.counters.add(&node.id, 1, started.elapsed().as_secs_f64() * 1000.0);
        if made.len() != node.spec.outputs.len() || made.iter().zip(node.spec.outputs).any(|(g, o)| g.kind() != o.kind) {
            return Err(EvalError::Broken { node: node.id.clone(), why: "its kernel gave other types than its outputs".to_owned() });
        }
        let made: Vec<Arc<Geo>> = made.into_iter().map(Arc::new).collect();
        for (p, g) in made.iter().enumerate() {
            self.put(Key::Geo { hash: node.hash, port: p as u8 }, Entry::Geo(g.clone()));
        }
        Ok(made[port as usize].clone())
    }

    /// A simulation over the whole terrain at level `run`: for each output,
    /// the change it made to the input it carries (or its value). On the
    /// graphics card when the node can and the card takes it.
    fn global(&self, c: &Compiled, n: usize, run: u8, cancel: Cancel) -> Result<Vec<Arc<Grid>>, EvalError> {
        let node = &c.nodes[n];
        let Kernel::Global { cpu, gpu, carry } = node.spec.kernel else { unreachable!("only simulations run whole") };
        let keys: Vec<Key> = (0..node.spec.outputs.len()).map(|p| Key::Whole { hash: node.hash, port: p as u8, lod: run }).collect();
        let found: Option<Vec<Arc<Grid>>> = keys
            .iter()
            .map(|k| match self.get(k) {
                Some(Entry::Whole(g)) => Some(g),
                _ => None,
            })
            .collect();
        if let Some(found) = found {
            return Ok(found);
        }
        let whole = c.space.whole(run);
        let geos = self.geos(c, n, cancel)?;
        let inputs: Vec<Option<Grid>> = node.inputs.iter().map(|i| i.filter(|&src| c.kind(src).is_raster()).map(|src| self.region(c, src, run, whole, cancel)).transpose()).collect::<Result<_, _>>()?;
        let driven_grids: Vec<Option<Grid>> = (0..node.spec.params.len()).map(|i| node.driven.get(i).copied().flatten().map(|src| self.region(c, src, run, whole, cancel)).transpose()).collect::<Result<_, _>>()?;
        let driven: Vec<Option<&[f32]>> = driven_grids.iter().map(|g| g.as_ref().map(|g| g.plane(0))).collect();
        let ctx = ctx_of(c, &geos, n, run, whole, &driven, &[]);
        let mut outs: Option<Vec<Vec<f32>>> = None;
        if let (Some(g), Some(kernel)) = (&self.gpu, gpu)
            && g.takes(node.spec.id)
        {
            let up: Vec<Option<GpuGrid>> = inputs.iter().map(|i| i.as_ref().map(|i| g.upload_grid(i))).collect();
            let refs: Vec<Option<&GpuGrid>> = up.iter().map(Option::as_ref).collect();
            match kernel(g, &ctx, &refs, cancel) {
                Some(made) => {
                    self.counters.gpu_globals.fetch_add(1, Ordering::Relaxed);
                    outs = Some(made.iter().map(|m| g.download_grid(m).data).collect());
                }
                None if cancel() => return Err(EvalError::Cancelled),
                None => g.refuse(node.spec.id, "its kernel gave nothing"),
            }
        }
        let outs = match outs {
            Some(o) => o,
            None => {
                let ins: Vec<Option<&Grid>> = inputs.iter().map(Option::as_ref).collect();
                cpu(&ctx, &ins, cancel).ok_or(EvalError::Cancelled)?
            }
        };
        self.counters.globals.fetch_add(1, Ordering::Relaxed);
        let grids: Vec<Arc<Grid>> = outs
            .into_iter()
            .enumerate()
            .map(|(p, mut o)| {
                if let Some(k) = carry[p]
                    && let Some(src) = &inputs[k as usize]
                {
                    for (v, b) in o.iter_mut().zip(&src.data) {
                        *v -= b;
                    }
                }
                Arc::new(Grid::new(run, whole, o))
            })
            .collect();
        for (k, g) in keys.into_iter().zip(&grids) {
            self.put(k, Entry::Whole(g.clone()));
        }
        Ok(grids)
    }

    /// Figures over the whole of an output, measured at the simulations'
    /// level (or 1024 samples a side, if finer).
    fn stats(&self, c: &Compiled, src: (usize, u8), cancel: Cancel) -> Result<Stats, EvalError> {
        let lod = c.space.lod_for(c.sim_res.min(1024));
        let key = Key::Stats { hash: c.nodes[src.0].hash, port: src.1, lod };
        if let Some(Entry::Stats(s)) = self.get(&key) {
            return Ok(s);
        }
        let g = self.whole(c, src, lod, cancel)?;
        let s = Stats::of(g.plane(0));
        self.put(key, Entry::Stats(s));
        Ok(s)
    }

    /// Brings to the CPU every tile feeding `tiles` of node `n`.
    fn feeds_to_cpu(&self, c: &Compiled, n: usize, tiles: &[(u8, i32, i32)], have: &HashMap<At, Arc<TileData>>) {
        let mut wanted: Vec<&TileData> = Vec::new();
        for &(l, x, z) in tiles {
            for f in feeds(c, n, l, x, z).into_iter().flatten() {
                for (tx, tz) in f.rect.tiles() {
                    if let Some(t) = have.get(&(f.src.0, f.src.1, f.lod, tx, tz)) {
                        wanted.push(t);
                    }
                }
            }
        }
        self.to_cpu(&wanted);
    }

    /// Reads back the tiles that are only on the graphics card.
    fn to_cpu(&self, tiles: &[&TileData]) {
        let Some(g) = &self.gpu else { return };
        let mut seen = std::collections::HashSet::new();
        let missing: Vec<&TileData> = tiles.iter().copied().filter(|t| t.cpu.get().is_none() && seen.insert(*t as *const TileData)).collect();
        if missing.is_empty() {
            return;
        }
        let gpu: Vec<&GpuTile> = missing.iter().map(|t| t.gpu.get().expect("a tile is somewhere")).collect();
        for (t, data) in missing.iter().zip(g.download(&gpu)) {
            let _ = t.cpu.set(data.into());
        }
    }

    /// Sends to the graphics card the tiles that are only on the CPU.
    fn to_gpu(&self, tiles: &[&TileData]) {
        let Some(g) = &self.gpu else { return };
        for t in tiles {
            t.gpu.get_or_init(|| g.upload(t.values().expect("a tile is somewhere")));
        }
    }
}

/// What tile `(x, z)` at `l` of node `n` reads: one entry per input, then
/// one per setting (the field driving it), `None` where nothing is read
/// tile by tile (what is unconnected, and what a simulation reads whole).
fn feeds(c: &Compiled, n: usize, l: u8, x: i32, z: i32) -> Vec<Option<Feed>> {
    let node = &c.nodes[n];
    let tile = Rect::tile(x, z);
    // Geometry is read whole (see `Engine::geometry`), never by tiles.
    let at_tile = |src: Option<(usize, u8)>| src.filter(|&s| c.kind(s).is_raster()).map(|src| Feed { src, lod: l, rect: tile });
    let driven = (0..node.spec.params.len()).map(|i| node.driven.get(i).copied().flatten());
    match node.spec.kernel {
        Kernel::Point { .. } | Kernel::Stats { .. } => node.inputs.iter().map(|i| at_tile(*i)).chain(driven.map(at_tile)).collect(),
        Kernel::Local { footprint, .. } => node
            .inputs
            .iter()
            .enumerate()
            .map(|(k, i)| {
                i.filter(|&s| c.kind(s).is_raster()).map(|src| {
                    let f = footprint(&node.params, l, k);
                    let rect = match f.map {
                        Some(m) => mapped(tile, l, f.lod, &m, c.space.size as f64),
                        None => tile.at_lod(l, f.lod),
                    };
                    Feed { src, lod: f.lod, rect: rect.expand(f.halo) }
                })
            })
            .chain(driven.map(at_tile))
            .collect(),
        Kernel::Global { carry, .. } => node
            .inputs
            .iter()
            .enumerate()
            .map(|(k, i)| if carry.contains(&Some(k as u8)) { at_tile(*i) } else { None })
            .chain(driven.map(|_| None))
            .collect(),
        Kernel::Output | Kernel::Geometry { .. } | Kernel::Volume { .. } => Vec::new(),
    }
}

/// The samples at level `to` around where affine map `m` (in fractions of
/// a terrain `size` blocks wide, from its middle) sends the samples `rect`
/// of level `from`: the box of its corners, with one more sample on each
/// side for interpolating.
fn mapped(rect: Rect, from: u8, to: u8, m: &[f64; 6], size: f64) -> Rect {
    let (s, t) = (Space::spacing(from), Space::spacing(to));
    let (x0, z0, x1, z1) = (rect.x as f64 * s, rect.z as f64 * s, rect.x1() as f64 * s, rect.z1() as f64 * s);
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (x, z) in [(x0, z0), (x1, z0), (x0, z1), (x1, z1)] {
        let (x, z) = (x / size - 0.5, z / size - 0.5);
        let (u, v) = ((m[0] * x + m[1] * z + m[2] + 0.5) * size, (m[3] * x + m[4] * z + m[5] + 0.5) * size);
        lo = (lo.0.min(u), lo.1.min(v));
        hi = (hi.0.max(u), hi.1.max(v));
    }
    // Kept to a sane size whatever the map: a few terrains across.
    let clamp = |v: f64| (v / t).clamp(-(1 << 26) as f64, (1 << 26) as f64);
    let (ax, az) = (clamp(lo.0).floor() as i32 - 1, clamp(lo.1).floor() as i32 - 1);
    let (bx, bz) = (clamp(hi.0).ceil() as i32 + 2, clamp(hi.1).ceil() as i32 + 2);
    Rect::new(ax, az, (bx - ax).max(1) as u32, (bz - az).max(1) as u32)
}

/// The context of node `n`'s kernel, with the geometry feeding it.
fn ctx_of<'a>(c: &'a Compiled, geos: &'a [Option<Arc<Geo>>], n: usize, lod: u8, rect: Rect, driven: &'a [Option<&'a [f32]>], reads: &'a [Option<(u8, Rect)>]) -> crate::kernel::Ctx<'a> {
    let mut ctx = c.ctx(n, lod, rect, driven, reads);
    ctx.geos = geos;
    ctx
}

/// About how many bytes geometry takes in the cache.
fn geo_bytes(g: &Geo) -> usize {
    match g {
        Geo::Points(p) => 64 + p.items.len() * 96,
        Geo::Splines(s) => 64 + s.lines.iter().map(|l| 96 + l.points.len() * 24).sum::<usize>(),
        Geo::Footprints(f) => 64 + f.items.len() * 96,
        Geo::Instances(i) => 64 + i.items.iter().map(|x| 128 + x.asset.len()).sum::<usize>(),
    }
}

/// Copies the samples of a tile at `from` (all its planes) into `grid`
/// where they overlap.
fn copy_into(grid: &mut Grid, from: Rect, data: &[f32]) {
    let r = grid.rect.intersect(&from);
    if r.is_empty() {
        return;
    }
    let (n_from, n_grid) = (from.area(), grid.rect.area());
    for p in 0..grid.planes.min(data.len() / n_from) {
        for z in r.z..r.z1() {
            let src = p * n_from + ((z - from.z) as usize) * from.w as usize + (r.x - from.x) as usize;
            let dst = p * n_grid + ((z - grid.rect.z) as usize) * grid.rect.w as usize + (r.x - grid.rect.x) as usize;
            grid.data[dst..dst + r.w as usize].copy_from_slice(&data[src..src + r.w as usize]);
        }
    }
}

/// The samples of `rect` at `lod` of an output, from its tiles in `have`.
fn assemble(have: &HashMap<At, Arc<TileData>>, (s, p): (usize, u8), lod: u8, rect: Rect) -> Grid {
    let tiles = rect.tiles();
    let planes = tiles.first().map_or(1, |&(tx, tz)| have[&(s, p, lod, tx, tz)].planes.max(1));
    let mut grid = Grid::new(lod, rect, vec![0.0; rect.area() * planes]);
    for (tx, tz) in tiles {
        let t = &have[&(s, p, lod, tx, tz)];
        copy_into(&mut grid, Rect::tile(tx, tz), t.values().expect("fed on the CPU"));
    }
    grid
}

/// An output node's place in a request: what feeds it.
fn resolve(c: &Compiled, out: (usize, u8)) -> (usize, u8) {
    let node = &c.nodes[out.0];
    if node.spec.is_output()
        && let Some(src) = node.inputs.first().copied().flatten()
    {
        return resolve(c, src);
    }
    out
}
