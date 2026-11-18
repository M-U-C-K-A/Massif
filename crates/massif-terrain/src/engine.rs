//! Evaluating a graph: by tiles, at any level of detail, with a cache.
//!
//! A request asks for tiles of one node's output at one level of detail.
//! The engine plans it first, walking back from that node: each tile it
//! lacks needs tiles of the nodes feeding it — the same tiles for a point
//! node, the tiles around them for a local one (at the level its footprint
//! says) — down to tiles in the cache or nodes with no inputs. Then it
//! computes node after node, all the tiles a node needs at once (in
//! parallel on the CPU, or in one submission on the graphics card), so
//! nothing ever waits on a lock while computing.
//!
//! Every tile is cached under the hash of its node — its type, settings and
//! inputs' hashes — with its output, level and place: changing a setting
//! changes the hash of that node and of those after it only, whose tiles
//! are then made again, and undoing it finds the old ones. A simulation is
//! made over the whole terrain at the simulation's level, cached whole,
//! and its tiles at finer levels are the input's tiles with the change the
//! simulation made, resampled (or its outputs resampled).

use crate::compile::{CNode, Compiled};
use crate::kernel::{Grid, Stats};
use crate::registry::Kernel;
use crate::space::{Rect, TILE};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "gpu")]
use crate::gpu::{Gpu, GpuTile};

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
    #[cfg(feature = "gpu")]
    gpu: OnceLock<GpuTile>,
}

impl TileData {
    pub fn cpu(data: Vec<f32>) -> Self {
        let t = Self { planes: data.len() / (TILE * TILE) as usize, ..Default::default() };
        let _ = t.cpu.set(data.into());
        t
    }

    #[cfg(feature = "gpu")]
    fn gpu(tile: GpuTile, planes: usize) -> Self {
        let t = Self { planes, ..Default::default() };
        let _ = t.gpu.set(tile);
        t
    }

    pub fn planes(&self) -> usize {
        self.planes
    }

    /// The samples, row by row, when they are on the CPU (every tile an
    /// evaluation gives back is).
    pub fn values(&self) -> Option<&[f32]> {
        self.cpu.get().map(|v| &v[..])
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
    /// Simulations run over the whole terrain.
    pub globals: AtomicU64,
    /// Time spent by each node, by node name: tiles made and milliseconds.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Tile { hash: u64, port: u8, lod: u8, x: i32, z: i32 },
    Whole { hash: u64, port: u8, lod: u8 },
    Stats { hash: u64, port: u8, lod: u8 },
}

#[derive(Clone)]
enum Entry {
    Tile(Arc<TileData>),
    Whole(Arc<Grid>),
    Stats(Stats),
}

impl Entry {
    fn bytes(&self) -> usize {
        match self {
            Entry::Tile(t) => (TILE * TILE * 4) as usize * t.planes.max(1),
            Entry::Whole(g) => g.data.len() * 4,
            Entry::Stats(_) => 64,
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

pub struct Engine {
    cache: Mutex<Cache>,
    #[cfg(feature = "gpu")]
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
        Self {
            cache: Mutex::new(Cache { budget: settings.cache_mb << 20, ..Default::default() }),
            #[cfg(feature = "gpu")]
            gpu: None,
            counters: Counters::default(),
        }
    }

    /// An engine running what it can on `gpu`.
    #[cfg(feature = "gpu")]
    pub fn with_gpu(gpu: Arc<Gpu>, settings: Settings) -> Self {
        let mut e = Self::new(settings);
        e.gpu = Some(gpu);
        e
    }

    /// The graphics card it runs on, if any.
    pub fn gpu_name(&self) -> Option<&str> {
        #[cfg(feature = "gpu")]
        {
            self.gpu.as_deref().map(Gpu::name)
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
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

    /// Tiles `keys` of output `out` at `lod`, their samples on the CPU.
    pub fn tiles(&self, c: &Compiled, out: (usize, u8), lod: u8, keys: &[(i32, i32)], cancel: Cancel) -> Result<Vec<Arc<TileData>>, EvalError> {
        let (root, port) = out;
        let node = &c.nodes[root];
        if node.spec.is_output() {
            let Some(src) = node.inputs[0] else {
                return Err(EvalError::Broken { node: node.id.clone(), why: node.broken.clone().unwrap_or_else(|| "it has no input".to_owned()) });
            };
            return self.tiles(c, src, lod, keys, cancel);
        }

        // Plan: what each node must make, walking back from the root.
        let mut need: Vec<BTreeSet<(u8, i32, i32)>> = vec![BTreeSet::new(); root + 1];
        let mut todo: Vec<Vec<(u8, i32, i32)>> = vec![Vec::new(); root + 1];
        let mut have: HashMap<At, Arc<TileData>> = HashMap::new();
        need[root].extend(keys.iter().map(|&(x, z)| (lod, x, z)));
        for n in (0..=root).rev() {
            if need[n].is_empty() {
                continue;
            }
            let node = &c.nodes[n];
            if let Some(why) = &node.broken {
                return Err(EvalError::Broken { node: node.id.clone(), why: why.clone() });
            }
            let wanted = std::mem::take(&mut need[n]);
            for (l, x, z) in wanted {
                if let Some(found) = self.cached(node, l, x, z) {
                    self.counters.hits.fetch_add(1, Ordering::Relaxed);
                    for (p, t) in found.into_iter().enumerate() {
                        have.insert((n, p as u8, l, x, z), t);
                    }
                    continue;
                }
                todo[n].push((l, x, z));
                match node.spec.kernel {
                    Kernel::Point { .. } | Kernel::Stats { .. } => {
                        for (s, _) in node.inputs.iter().flatten() {
                            need[*s].insert((l, x, z));
                        }
                    }
                    Kernel::Local { footprint, .. } => {
                        for (k, input) in node.inputs.iter().enumerate() {
                            if let Some((s, _)) = input {
                                let f = footprint(&node.params, l, k);
                                for (tx, tz) in Rect::tile(x, z).at_lod(l, f.lod).expand(f.halo).tiles() {
                                    need[*s].insert((f.lod, tx, tz));
                                }
                            }
                        }
                    }
                    Kernel::Global { carry, .. } => {
                        for k in carry.iter().flatten() {
                            if let Some((s, _)) = node.inputs[*k as usize] {
                                need[s].insert((l, x, z));
                            }
                        }
                    }
                    Kernel::Output => unreachable!("outputs feed nothing"),
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
            let node = &c.nodes[n];
            for (&(l, x, z), outs) in todo.iter().zip(made) {
                for (p, t) in outs.into_iter().enumerate() {
                    self.put(Key::Tile { hash: node.hash, port: p as u8, lod: l, x, z }, Entry::Tile(t.clone()));
                    have.insert((n, p as u8, l, x, z), t);
                }
            }
        }

        let out: Vec<Arc<TileData>> = keys.iter().map(|&(x, z)| have[&(root, port, lod, x, z)].clone()).collect();
        self.to_cpu(&out.iter().map(|t| &**t).collect::<Vec<_>>());
        Ok(out)
    }

    /// All of a node's outputs at a tile, when the cache has them.
    fn cached(&self, node: &CNode, lod: u8, x: i32, z: i32) -> Option<Vec<Arc<TileData>>> {
        let mut cache = self.cache.lock().expect("lock");
        (0..node.spec.outputs.len())
            .map(|p| match cache.get(&Key::Tile { hash: node.hash, port: p as u8, lod, x, z }) {
                Some(Entry::Tile(t)) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Makes `tiles` of node `n`, its inputs' tiles in `have`.
    fn make(&self, c: &Compiled, n: usize, tiles: &[(u8, i32, i32)], have: &HashMap<At, Arc<TileData>>, cancel: Cancel) -> Result<Vec<Vec<Arc<TileData>>>, EvalError> {
        let node = &c.nodes[n];
        let nout = node.spec.outputs.len();
        let area = (TILE * TILE) as usize;
        let planes: Vec<usize> = node.spec.outputs.iter().map(|o| o.kind.planes()).collect();
        let fresh = || planes.iter().map(|p| vec![0.0f32; area * p]).collect::<Vec<_>>();
        let input = |k: usize, l: u8, x: i32, z: i32| node.inputs[k].map(|(s, p)| &have[&(s, p, l, x, z)]);
        match node.spec.kernel {
            Kernel::Point { cpu, gpu } => {
                #[cfg(feature = "gpu")]
                if let (Some(g), Some(kernel)) = (&self.gpu, gpu)
                    && g.takes(kernel)
                {
                    let connected: Vec<bool> = node.inputs.iter().map(Option::is_some).collect();
                    let ins: Vec<Vec<Option<&TileData>>> = tiles.iter().map(|&(l, x, z)| (0..node.inputs.len()).map(|k| input(k, l, x, z).map(|t| &**t)).collect()).collect();
                    for row in &ins {
                        for t in row.iter().flatten() {
                            t.gpu.get_or_init(|| g.upload(t.values().expect("a tile is on the CPU when not on the GPU")));
                        }
                    }
                    let jobs: Vec<(Vec<u8>, Vec<Option<&GpuTile>>)> = tiles
                        .iter()
                        .zip(&ins)
                        .map(|(&(l, x, z), row)| ((kernel.uniforms)(&node.ctx(&c.space, l, Rect::tile(x, z)), &connected), row.iter().map(|t| t.and_then(|t| t.gpu.get())).collect()))
                        .collect();
                    if let Some(made) = g.run_point(kernel, &jobs, &planes) {
                        self.counters.gpu_tiles.fetch_add(tiles.len() as u64, Ordering::Relaxed);
                        return Ok(made.into_iter().map(|outs| outs.into_iter().zip(&planes).map(|(t, p)| Arc::new(TileData::gpu(t, *p))).collect()).collect());
                    }
                }
                #[cfg(not(feature = "gpu"))]
                let _ = gpu;
                self.inputs_to_cpu(node, tiles, have, |l, x, z| vec![(l, Rect::tile(x, z)); node.inputs.len()]);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let ins: Vec<Option<&[f32]>> = (0..node.inputs.len()).map(|k| input(k, l, x, z).map(|t| t.values().expect("inputs are on the CPU"))).collect();
                        let mut outs = fresh();
                        cpu(&node.ctx(&c.space, l, Rect::tile(x, z)), &ins, &mut outs);
                        Ok(outs.into_iter().map(|o| Arc::new(TileData::cpu(o))).collect())
                    })
                    .collect()
            }
            Kernel::Stats { cpu } => {
                let stats: Vec<Option<Stats>> = node.inputs.iter().map(|i| i.map(|src| self.stats(c, src, cancel)).transpose()).collect::<Result<_, _>>()?;
                self.inputs_to_cpu(node, tiles, have, |l, x, z| vec![(l, Rect::tile(x, z)); node.inputs.len()]);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let ins: Vec<Option<&[f32]>> = (0..node.inputs.len()).map(|k| input(k, l, x, z).map(|t| t.values().expect("inputs are on the CPU"))).collect();
                        let mut outs = fresh();
                        cpu(&node.ctx(&c.space, l, Rect::tile(x, z)), &stats, &ins, &mut outs);
                        Ok(outs.into_iter().map(|o| Arc::new(TileData::cpu(o))).collect())
                    })
                    .collect()
            }
            Kernel::Local { cpu, footprint } => {
                let read = |k: usize, l: u8, x: i32, z: i32| {
                    let f = footprint(&node.params, l, k);
                    (f.lod, Rect::tile(x, z).at_lod(l, f.lod).expand(f.halo))
                };
                self.inputs_to_cpu(node, tiles, have, |l, x, z| (0..node.inputs.len()).map(|k| read(k, l, x, z)).collect());
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let grids: Vec<Option<Grid>> = node
                            .inputs
                            .iter()
                            .enumerate()
                            .map(|(k, i)| {
                                i.map(|(s, p)| {
                                    let (fl, r) = read(k, l, x, z);
                                    assemble(have, (s, p), fl, r)
                                })
                            })
                            .collect();
                        let ins: Vec<Option<&Grid>> = grids.iter().map(Option::as_ref).collect();
                        let mut outs = fresh();
                        cpu(&node.ctx(&c.space, l, Rect::tile(x, z)), &ins, &mut outs);
                        Ok(outs.into_iter().map(|o| Arc::new(TileData::cpu(o))).collect())
                    })
                    .collect()
            }
            Kernel::Global { carry, .. } => {
                let lods: BTreeSet<u8> = tiles.iter().map(|t| t.0).collect();
                let mut wholes: HashMap<u8, Vec<Arc<Grid>>> = HashMap::new();
                for l in lods {
                    let run = l.max(c.sim_lod());
                    wholes.insert(l, self.global(c, n, run, cancel)?);
                }
                self.inputs_to_cpu(node, tiles, have, |l, x, z| vec![(l, Rect::tile(x, z)); node.inputs.len()]);
                tiles
                    .par_iter()
                    .map(|&(l, x, z)| {
                        if cancel() {
                            return Err(EvalError::Cancelled);
                        }
                        let run = l.max(c.sim_lod());
                        let f = crate::space::Space::spacing(l) / crate::space::Space::spacing(run);
                        let rect = Rect::tile(x, z);
                        let outs = (0..nout)
                            .map(|p| {
                                let g = &wholes[&l][p];
                                let kind = node.spec.outputs[p].kind;
                                let base = carry[p].and_then(|k| input(k as usize, l, x, z)).map(|t| t.values().expect("inputs are on the CPU"));
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
        }
    }

    /// A simulation over the whole terrain at level `run`: for each output,
    /// the change it made to the input it carries (or its value).
    fn global(&self, c: &Compiled, n: usize, run: u8, cancel: Cancel) -> Result<Vec<Arc<Grid>>, EvalError> {
        let node = &c.nodes[n];
        let Kernel::Global { cpu, carry } = node.spec.kernel else { unreachable!("only simulations run whole") };
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
        let inputs: Vec<Option<Grid>> = node.inputs.iter().map(|i| i.map(|src| self.region(c, src, run, whole, cancel)).transpose()).collect::<Result<_, _>>()?;
        let ins: Vec<Option<&Grid>> = inputs.iter().map(Option::as_ref).collect();
        let outs = cpu(&node.ctx(&c.space, run, whole), &ins, cancel).ok_or(EvalError::Cancelled)?;
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
        let g = self.region(c, src, lod, c.space.whole(lod), cancel)?;
        let s = Stats::of(g.plane(0));
        self.put(key, Entry::Stats(s));
        Ok(s)
    }

    /// Brings to the CPU the input tiles a CPU kernel will read: for each
    /// tile made, `reads` gives the level and samples it reads of each input.
    fn inputs_to_cpu(&self, node: &CNode, tiles: &[(u8, i32, i32)], have: &HashMap<At, Arc<TileData>>, reads: impl Fn(u8, i32, i32) -> Vec<(u8, Rect)>) {
        let mut wanted: Vec<&TileData> = Vec::new();
        for &(l, x, z) in tiles {
            for (input, (rl, r)) in node.inputs.iter().zip(reads(l, x, z)) {
                if let Some((s, p)) = input {
                    for (tx, tz) in r.tiles() {
                        if let Some(t) = have.get(&(*s, *p, rl, tx, tz)) {
                            wanted.push(t);
                        }
                    }
                }
            }
        }
        self.to_cpu(&wanted);
    }

    /// Reads back the tiles that are only on the graphics card.
    fn to_cpu(&self, tiles: &[&TileData]) {
        #[cfg(feature = "gpu")]
        if let Some(g) = &self.gpu {
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
        #[cfg(not(feature = "gpu"))]
        let _ = tiles;
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
        copy_into(&mut grid, Rect::tile(tx, tz), t.values().expect("inputs are on the CPU"));
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
