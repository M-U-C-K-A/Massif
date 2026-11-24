//! The Diorama: the open map seen from straight above, block by block, as
//! the game would show it: each column's top block with the game's
//! texture, the trees, plants and structures the export would place, the
//! water over the shallows, and relief shading that makes slopes, cliffs
//! and valleys read.
//!
//! The tiles are built by the exporter itself (as for the Render side), in
//! the background, nearest the view first. Of each column only its
//! *floors* are kept: every place where something solid lies under air or
//! water, from the top down, with what stands on it (a plant, a torch) and
//! how deep the water over it is. The surface is the first floor; the
//! caves at a height are the highest floor under it. Both are drawn by a
//! compute shader from the floors put on the graphics card for the tiles
//! in view, and from a small image of every tile for a view from afar.

use crate::materials::{Lookup, Materials, TopKind, TopView};
use massif_anvil::Target;
use massif_anvil::chunk::{ExportContext, Scratch, chunk_blocks, tile_placements};
use massif_core::compose::TileStack;
use massif_core::coords::TilePos;
use massif_core::objects::Placement;
use massif_core::world::World;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// Blocks on a side of a tile.
pub const TILE: usize = 128;
/// Columns of a tile.
const AREA: usize = TILE * TILE;
/// Chunks on a side of a tile.
const TILE_CHUNKS: usize = 8;
/// The height of a column with no floor where it is looked at.
pub const NO_FLOOR: u16 = u16::MAX;

/// A place in a column where something solid lies under air or water.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Floor {
    /// The height of its block (the top is one higher).
    pub y: i16,
    /// The material of its block.
    pub block: u16,
    /// What stands on it, seen from above (a plant, a torch): 0 for
    /// nothing.
    pub over: u16,
    /// How many blocks of water or lava lie on it.
    pub depth: u8,
    /// That fluid is lava.
    pub lava: bool,
}

/// The floors of every column of a tile, from the top of each down.
#[derive(Debug, Clone, Default)]
pub struct TileFloors {
    starts: Vec<u32>,
    floors: Vec<Floor>,
    /// The surface of each column as a small image (see [`colours`]),
    /// `size × size`, sRGB.
    pub surface: Vec<u8>,
}

impl TileFloors {
    /// The floors of column `cell` (`z * 128 + x`), highest first.
    pub fn column(&self, cell: usize) -> &[Floor] {
        if self.starts.len() <= cell + 1 {
            return &[];
        }
        &self.floors[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }

    /// The floor seen of column `cell`: its highest, or with `slice` (the
    /// caves at a height), the highest whose block is under it.
    pub fn seen(&self, cell: usize, slice: Option<i32>) -> Option<Floor> {
        let column = self.column(cell);
        match slice {
            None => column.first().copied(),
            Some(s) => column.iter().find(|f| (f.y as i32) < s).copied(),
        }
    }

    /// Bytes it holds.
    pub fn bytes(&self) -> usize {
        self.starts.len() * 4 + self.floors.len() * std::mem::size_of::<Floor>() + self.surface.len() + 64
    }

    /// Floors in all.
    pub fn len(&self) -> usize {
        self.floors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.floors.is_empty()
    }

    /// The floors seen, as the shader reads them: four words a column
    /// (block, height over `min_y` or [`NO_FLOOR`], what stands on it, the
    /// depth of its fluid | lava << 8).
    pub fn pack(&self, slice: Option<i32>, min_y: i32, out: &mut Vec<u16>) {
        out.clear();
        out.reserve(AREA * 4);
        for cell in 0..AREA {
            match self.seen(cell, slice) {
                Some(f) => out.extend_from_slice(&[f.block, (f.y as i32 - min_y).clamp(0, NO_FLOOR as i32 - 1) as u16, f.over, f.depth as u16 | (f.lava as u16) << 8]),
                None => out.extend_from_slice(&[0, NO_FLOOR, 0, 0]),
            }
        }
    }
}

/// The light of the relief, from the north-west and above.
const LIGHT: [f32; 3] = [-0.55, 0.75, -0.55];

/// How bright the relief makes a column of height `h`, its neighbours to
/// the west, east, north and south at `w`, `e`, `n` and `s`: the same in
/// the shader.
pub fn relief(h: f32, w: f32, e: f32, n: f32, s: f32) -> f32 {
    let (dx, dz) = (((e - w) * 0.5).clamp(-6.0, 6.0), ((s - n) * 0.5).clamp(-6.0, 6.0));
    let len = (dx * dx + 1.0 + dz * dz).sqrt();
    let normal = [-dx / len, 1.0 / len, -dz / len];
    let l = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
    let lit = (normal[0] * LIGHT[0] + normal[1] * LIGHT[1] + normal[2] * LIGHT[2]) / l;
    let flat = LIGHT[1] / l;
    // A step up from the north-west catches the light; a block lower than
    // its neighbours there lies in their shade.
    let step = ((h - n) + (h - w)).clamp(-4.0, 4.0) * 0.025;
    (0.3 + 0.7 * lit.max(0.0) / flat + step).clamp(0.25, 1.35)
}

/// The colour seen of a floor, from the materials' mean colours.
fn floor_colour(f: &Floor, views: &[TopView], water: [u8; 4], lava: [u8; 4]) -> [f32; 3] {
    let view = |m: u16| views.get(m as usize).copied().unwrap_or(TopView::AIR);
    let lin = |c: u8| c as f32 / 255.0;
    let g = view(f.block).mean;
    let mut c = [lin(g[0]), lin(g[1]), lin(g[2])];
    if f.depth > 0 {
        let w = if f.lava { lava } else { water };
        let a = if f.lava { 1.0 } else { (0.55 + f.depth as f32 * 0.08).min(0.95) };
        for k in 0..3 {
            let floor = c[k] * (lin(w[k]) * 2.5).min(1.0);
            c[k] = floor + (lin(w[k]) - floor) * a;
        }
    }
    if f.over != 0 {
        let o = view(f.over).mean;
        let a = lin(o[3]) * 0.8;
        for k in 0..3 {
            c[k] += (lin(o[k]) - c[k]) * a;
        }
    }
    c
}

/// What the colours are lifted by: the game's daylight (the shader's).
const EXPOSURE: f32 = 1.25;

/// The colour the caves view gives a column with no floor under the slice:
/// rock.
pub const ROCK: [u8; 3] = [24, 22, 26];

/// How dark the caves view makes a floor `depth` blocks under the slice.
pub fn cave_light(depth: f32) -> f32 {
    (1.0 - depth / 120.0).clamp(0.45, 1.0)
}

/// A tile's floors seen (the surface, or with `slice` the caves under it)
/// as a small image, `size × size` (a power of two up to 128), sRGB
/// colours with the relief's light: what the view shows from afar.
pub fn colours(t: &TileFloors, slice: Option<i32>, size: usize, views: &[TopView], water: [u8; 4], lava: [u8; 4]) -> Vec<u8> {
    let mut height = vec![f32::NAN; AREA];
    let mut colour = vec![[0f32; 3]; AREA];
    for cell in 0..AREA {
        if let Some(f) = t.seen(cell, slice) {
            height[cell] = f.y as f32 + if f.depth > 0 { f.depth as f32 * 0.5 } else { 0.0 };
            // In linear light, as the shader draws.
            let mut c = floor_colour(&f, views, water, lava).map(|v| v.powf(2.2));
            if let Some(s) = slice {
                let k = cave_light((s - 1 - f.y as i32) as f32);
                c = c.map(|v| v * k);
            }
            colour[cell] = c;
        } else if slice.is_some() {
            colour[cell] = ROCK.map(|v| (v as f32 / 255.0).powf(2.2));
        }
    }
    let at = |x: i64, z: i64, own: f32| {
        if !(0..TILE as i64).contains(&x) || !(0..TILE as i64).contains(&z) {
            return own;
        }
        let h = height[z as usize * TILE + x as usize];
        if h.is_nan() { own } else { h }
    };
    let step = TILE / size.clamp(1, TILE);
    let mut out = vec![0u8; size * size * 4];
    for oz in 0..size {
        for ox in 0..size {
            let (mut sum, mut n, mut any) = ([0f32; 3], 0f32, false);
            for z in oz * step..(oz + 1) * step {
                for x in ox * step..(ox + 1) * step {
                    let cell = z * TILE + x;
                    let h = height[cell];
                    let mut c = colour[cell];
                    if !h.is_nan() {
                        let (xi, zi) = (x as i64, z as i64);
                        let k = relief(h, at(xi - 1, zi, h), at(xi + 1, zi, h), at(xi, zi - 1, h), at(xi, zi + 1, h)) * EXPOSURE;
                        c = c.map(|v| v * k);
                        any = true;
                    } else if slice.is_some() {
                        any = true;
                    }
                    for k in 0..3 {
                        sum[k] += c[k];
                    }
                    n += 1.0;
                }
            }
            let o = &mut out[(oz * size + ox) * 4..][..4];
            for k in 0..3 {
                o[k] = ((sum[k] / n).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
            }
            o[3] = if any { 255 } else { 0 };
        }
    }
    out
}

#[doc(hidden)]
#[path = "diorama_sample.rs"]
pub mod sample;

// ------------------------------------------------------------- building --

/// What a thread knows of the states it has met: the material of each and
/// how it looks from above.
struct Seen {
    lookup: Lookup,
    states: Vec<Option<(u16, TopKind)>>,
    kinds: HashMap<u16, TopKind>,
}

impl Seen {
    fn new(states: usize) -> Self {
        Self { lookup: Lookup::default(), states: vec![None; states], kinds: HashMap::new() }
    }

    fn of(&mut self, ctx: &ExportContext, materials: &Materials, state: u16) -> (u16, TopKind) {
        if let Some(Some(k)) = self.states.get(state as usize) {
            return *k;
        }
        let Some((name, props)) = ctx.state(state) else { return (0, TopKind::Air) };
        let props: Vec<(&str, &str)> = props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let material = self.lookup.id(materials, name, &props);
        let kind = *self.kinds.entry(material).or_insert_with(|| if material == 0 { TopKind::Air } else { materials.top_view(material).kind });
        if let Some(slot) = self.states.get_mut(state as usize) {
            *slot = Some((material, kind));
        }
        (material, kind)
    }
}

/// The floors of the columns of a chunk's blocks (as [`chunk_blocks`] lays
/// them), into `floors` for each column `(lz << 4) | lx`.
fn chunk_floors(ctx: &ExportContext, materials: &Materials, seen: &mut Seen, blocks: &[u16], floors: &mut [Vec<Floor>]) {
    let rows = blocks.len() / 256;
    for (column, out) in floors.iter_mut().enumerate().take(256) {
        out.clear();
        let (mut gap, mut over, mut depth, mut lava) = (true, 0u16, 0u8, false);
        for y in (0..rows).rev() {
            let state = blocks[(y << 8) | column];
            let (material, kind) = if state == 0 { (0, TopKind::Air) } else { seen.of(ctx, materials, state) };
            match kind {
                TopKind::Air => {
                    if !gap {
                        (gap, over, depth, lava) = (true, 0, 0, false);
                    }
                }
                TopKind::Overlay => {
                    if gap && over == 0 && depth == 0 {
                        over = material;
                    }
                }
                TopKind::Water | TopKind::Lava => {
                    if !gap {
                        (gap, over, depth) = (true, 0, 0);
                    }
                    depth = depth.saturating_add(1);
                    lava = kind == TopKind::Lava;
                }
                TopKind::Block => {
                    if gap {
                        out.push(Floor { y: (ctx.min_y + y as i32) as i16, block: material, over, depth, lava });
                        gap = false;
                    }
                }
            }
        }
    }
}

/// A tile's floors, built.
pub struct BuiltTile {
    pub tile: TilePos,
    pub floors: Arc<TileFloors>,
    /// How long it took, seconds of a thread.
    pub took: f32,
}

/// The tiles still to build, kept in the order they are wanted in: the
/// next is the last.
#[derive(Default)]
struct Queue {
    tiles: Vec<TilePos>,
    stop: bool,
}

/// Builds the floors of tiles of a map in the background, the nearest the
/// view first.
pub struct Builder {
    rx: Receiver<BuiltTile>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicUsize>,
    thread: Option<JoinHandle<Result<(), String>>>,
    error: Option<String>,
    /// Materials of the map: the floors' numbers are theirs.
    pub materials: Arc<Materials>,
    pub min_y: i32,
    pub max_y: i32,
}

/// Tiles built side by side.
const BATCH: usize = 16;

impl Builder {
    /// Build the floors of dimension `dim` of `world` (a snapshot the
    /// builder keeps) for `target`, their small images `size` across, as
    /// tiles are asked for (see [`Builder::want`]).
    pub fn start(world: World, dim: usize, target: Target, materials: Arc<Materials>, size: usize) -> Result<Self, String> {
        let (min_y, max_y) = {
            let d = world.dims.get(dim).ok_or("no such dimension")?;
            let ctx = ExportContext::new(&world, d, &target)?;
            (ctx.min_y, ctx.max_y)
        };
        let (tx, rx) = channel();
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicUsize::new(0));
        let (q, stop, working, mats) = (queue.clone(), cancel.clone(), busy.clone(), materials.clone());
        let thread = std::thread::Builder::new()
            .name("massif diorama".into())
            .spawn(move || -> Result<(), String> {
                // In the builders' own threads: never in the pool the
                // interface's brushes and map work in, which would wait for them.
                crate::builder_pool().install(move || -> Result<(), String> {
                    let d = world.dims.get(dim).ok_or("no such dimension")?;
                    let ctx = ExportContext::new(&world, d, &target)?;
                    let planner = (!world.meta.lava).then(|| massif_core::water::Planner::new(d));
                    let placed: Mutex<HashMap<TilePos, Arc<Vec<Placement>>>> = Mutex::new(HashMap::new());
                    let ring = (ctx.reach + 127) / 128;
                    let water = mats.top_view(mats.id("minecraft:water", &[("level", "0")])).mean;
                    let lava = mats.top_view(mats.id("minecraft:lava", &[("level", "0")])).mean;
                    loop {
                        let batch: Vec<TilePos> = {
                            let (lock, wake) = &*q;
                            let mut g = lock.lock().unwrap();
                            while g.tiles.is_empty() && !g.stop && !stop.load(Ordering::Relaxed) {
                                g = wake.wait(g).unwrap();
                            }
                            if g.stop || stop.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            let n = g.tiles.len().saturating_sub(BATCH);
                            working.fetch_add(g.tiles.len() - n, Ordering::Relaxed);
                            g.tiles.split_off(n)
                        };
                        batch
                            .par_iter()
                            .rev()
                            .map_init(
                                || (Scratch::new(&ctx), Seen::new(ctx.state_count()), vec![Vec::new(); 256]),
                                |(scratch, seen, columns), &pos| {
                                    if stop.load(Ordering::Relaxed) {
                                        return None;
                                    }
                                    let started = Instant::now();
                                    let Some(stack) = TileStack::new(d, pos, true) else {
                                        return Some(BuiltTile { tile: pos, floors: Arc::new(TileFloors::default()), took: 0.0 });
                                    };
                                    let mut objects = Vec::new();
                                    if ctx.reach > 0 {
                                        let area = pos.rect().expand(ctx.reach);
                                        for tz in pos.z - ring..=pos.z + ring {
                                            for tx in pos.x - ring..=pos.x + ring {
                                                let here = TilePos::new(tx, tz);
                                                let cached = placed.lock().unwrap().get(&here).cloned();
                                                let list = cached.unwrap_or_else(|| {
                                                    let list = Arc::new(tile_placements(&ctx, &world, d, here));
                                                    let mut cache = placed.lock().unwrap();
                                                    if cache.len() > 4096 {
                                                        cache.clear();
                                                    }
                                                    cache.insert(here, list.clone());
                                                    list
                                                });
                                                objects.extend(list.iter().filter(|p| area.x0 <= p.x && p.x < area.x1 && area.z0 <= p.z && p.z < area.z1).copied());
                                            }
                                        }
                                        objects.sort_unstable_by_key(|p| (p.layer, p.x, p.z));
                                    }
                                    scratch.lay_ores(&ctx, &stack, pos);
                                    let steady = if planner.as_ref().is_some_and(|p| p.needs(pos)) { massif_core::water::steady_tile(d, pos, planner.as_ref()) } else { Vec::new() };
                                    let slabs = massif_core::shapes::plan_tile(d, &world.meta.smooth, pos);
                                    let mut per_cell: Vec<Vec<Floor>> = vec![Vec::new(); AREA];
                                    for cz in 0..TILE_CHUNKS {
                                        for cx in 0..TILE_CHUNKS {
                                            let blocks = chunk_blocks(&ctx, &stack, pos, cx, cz, &objects, &steady, &slabs, scratch);
                                            chunk_floors(&ctx, &mats, seen, blocks, columns);
                                            for (column, list) in columns.iter_mut().enumerate() {
                                                let (lx, lz) = (column & 15, column >> 4);
                                                per_cell[(cz * 16 + lz) * TILE + cx * 16 + lx] = std::mem::take(list);
                                            }
                                        }
                                    }
                                    let mut floors = TileFloors { starts: Vec::with_capacity(AREA + 1), floors: Vec::new(), surface: Vec::new() };
                                    for list in &per_cell {
                                        floors.starts.push(floors.floors.len() as u32);
                                        floors.floors.extend_from_slice(list);
                                    }
                                    floors.starts.push(floors.floors.len() as u32);
                                    floors.floors.shrink_to_fit();
                                    let views = mats.top_views();
                                    floors.surface = colours(&floors, None, size, &views, water, lava);
                                    Some(BuiltTile { tile: pos, floors: Arc::new(floors), took: started.elapsed().as_secs_f32() })
                                },
                            )
                            .for_each_with(tx.clone(), |tx, built| {
                                if let Some(built) = built {
                                    let _ = tx.send(built);
                                }
                            });
                        working.store(0, Ordering::Relaxed);
                    }
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { rx, queue, cancel, busy, thread: Some(thread), error: None, materials, min_y, max_y })
    }

    /// Build these tiles next, the first first, before those asked for
    /// earlier and not begun (which stay after them unless `only`).
    pub fn want(&self, tiles: &[TilePos], only: bool) {
        let (lock, wake) = &*self.queue;
        let mut g = lock.lock().unwrap();
        let fresh: HashSet<TilePos> = tiles.iter().copied().collect();
        if only {
            g.tiles.clear();
        } else {
            g.tiles.retain(|t| !fresh.contains(t));
        }
        g.tiles.extend(tiles.iter().rev().copied());
        wake.notify_all();
    }

    /// Tiles waiting or being built.
    pub fn pending(&self) -> usize {
        self.queue.0.lock().unwrap().tiles.len() + self.busy.load(Ordering::Relaxed)
    }

    /// A tile built, if one is ready.
    pub fn take(&mut self) -> Option<BuiltTile> {
        match self.rx.try_recv() {
            Ok(b) => Some(b),
            Err(_) => {
                if self.thread.as_ref().is_some_and(JoinHandle::is_finished)
                    && let Some(t) = self.thread.take()
                    && let Ok(Err(e)) = t.join()
                {
                    self.error = Some(e);
                }
                None
            }
        }
    }

    /// Wait for a tile (tests and stills).
    pub fn wait(&mut self) -> Option<BuiltTile> {
        if self.pending() == 0 {
            return self.rx.try_recv().ok();
        }
        self.rx.recv().ok()
    }

    /// Why the builder stopped, if it failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Drop for Builder {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap().stop = true;
        wake.notify_all();
    }
}

// ------------------------------------------------------------------ gpu --

/// Slots on a side of the atlas of floors on the graphics card: tiles in
/// view drawn block by block.
const NEAR_SIDE: u32 = 24;
/// Most tiles put on the card a frame.
const UPLOADS: usize = 24;

/// What is drawn, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The block at the middle of the image, x and z.
    pub center: [f64; 2],
    /// Blocks a pixel of the image covers.
    pub blocks_per_px: f32,
    /// Quarter turns clockwise: 0 north up, 1 east up, 2 south, 3 west.
    pub turn: u8,
    /// The caves under this height, else the surface.
    pub slice: Option<i32>,
    /// Behind the map, sRGB.
    pub background: [u8; 3],
    /// How strong the relief's light and shadows are, 0 to 1.
    pub relief: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// Tile of the first entry of the page table, and the table's size.
    table: [i32; 4],
    background: [f32; 4],
    /// The block at the image's middle, relative to `origin`.
    center: [f32; 2],
    size: [u32; 2],
    /// The whole tile `origin` is relative to (blocks: × 128).
    origin: [i32; 2],
    blocks_per_px: f32,
    turn: u32,
    slice: i32,
    caves: u32,
    min_y: i32,
    near_side: u32,
    far_size: u32,
    far_side: u32,
    water: u32,
    lava: u32,
    relief: f32,
    use_near: u32,
    pad: [u32; 2],
}

struct Image {
    size: [u32; 2],
    texture: wgpu::Texture,
    storage: wgpu::TextureView,
    view: wgpu::TextureView,
}

/// The Diorama on the graphics card: the floors of the tiles in view, a
/// small image of every tile, the textures of the materials, and the
/// shader drawing them into an image.
pub struct DioramaGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    /// The tiles of the map: which slot each has in either atlas.
    table: wgpu::Texture,
    table_size: [u32; 2],
    table_origin: [i32; 2],
    table_words: Vec<u32>,
    table_dirty: bool,
    near: wgpu::Texture,
    near_slots: HashMap<TilePos, u32>,
    /// Last frame each slot was used, to give the oldest away.
    near_used: Vec<u64>,
    near_tile: Vec<Option<TilePos>>,
    far: wgpu::Texture,
    far_size: u32,
    far_side: u32,
    far_slot: HashMap<TilePos, u32>,
    materials_buf: wgpu::Buffer,
    materials_revision: u64,
    water: u32,
    lava: u32,
    atlas: (wgpu::Texture, wgpu::TextureView),
    atlas_layers: u32,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    image: Option<Image>,
    bind_group: Option<wgpu::BindGroup>,
    frame: u64,
    min_y: i32,
    pack: Vec<u16>,
    /// What the floors in the near slots were packed for.
    packed_slice: Option<Option<i32>>,
}

impl DioramaGpu {
    /// For a map whose tiles lie in `tiles` (min and max, inclusive),
    /// `count` of them, worlds from height `min_y`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, lo: TilePos, hi: TilePos, count: usize, min_y: i32) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("massif diorama"), source: wgpu::ShaderSource::Wgsl(include_str!("diorama.wgsl").into()) });
        let tex = |binding: u32, sample_type: wgpu::TextureSampleType, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: dim, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif diorama"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(size_of::<Params>() as u64) },
                    count: None,
                },
                tex(1, wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2),
                tex(2, wgpu::TextureSampleType::Uint, wgpu::TextureViewDimension::D2),
                tex(3, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex(5, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: crate::gpu::IMAGE_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("massif diorama"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("massif diorama"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif diorama params"), size: size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let table_size = [(hi.x - lo.x + 1).max(1) as u32, (hi.z - lo.z + 1).max(1) as u32];
        let plain = |label: &str, size: [u32; 2], format: wgpu::TextureFormat, mips: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let table = plain("massif diorama tiles", table_size, wgpu::TextureFormat::R32Uint, 1);
        let near = plain("massif diorama floors", [NEAR_SIDE * TILE as u32; 2], wgpu::TextureFormat::Rgba16Uint, 1);
        let (far_size, far_side) = far_layout(count);
        let far_mips = far_size.trailing_zeros() + 1;
        let far = plain("massif diorama far", [far_side * far_size; 2], wgpu::TextureFormat::Rgba8UnormSrgb, far_mips);
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif diorama blocks"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif diorama far"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let materials_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif diorama materials"), size: 4096, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let atlas = crate::gpu::Renderer::make_array(device, 16, wgpu::TextureFormat::Rgba8UnormSrgb);
        let slots = (NEAR_SIDE * NEAR_SIDE) as usize;
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            layout,
            params,
            table,
            table_size,
            table_origin: [lo.x, lo.z],
            table_words: vec![0; (table_size[0] * table_size[1]) as usize],
            table_dirty: true,
            near,
            near_slots: HashMap::new(),
            near_used: vec![0; slots],
            near_tile: vec![None; slots],
            far,
            far_size,
            far_side,
            far_slot: HashMap::new(),
            materials_buf,
            materials_revision: u64::MAX,
            water: 0,
            lava: 0,
            atlas,
            atlas_layers: 0,
            nearest,
            linear,
            image: None,
            bind_group: None,
            frame: 0,
            min_y,
            pack: Vec::new(),
            packed_slice: None,
        }
    }

    /// The size of a tile's small image on this map.
    pub fn far_size(&self) -> usize {
        self.far_size as usize
    }

    /// Whether `tile` lies in the map this was made for.
    pub fn holds(&self, tile: TilePos) -> bool {
        let (x, z) = (tile.x - self.table_origin[0], tile.z - self.table_origin[1]);
        x >= 0 && z >= 0 && (x as u32) < self.table_size[0] && (z as u32) < self.table_size[1]
    }

    /// The materials and their textures, as new ones come.
    pub fn sync_materials(&mut self, materials: &Materials) {
        if materials.revision() == self.materials_revision {
            return;
        }
        let (revision, _, _, layers) = materials.snapshot();
        let views = materials.top_views();
        let pack_px = |a: [f32; 4]| a.iter().enumerate().fold(0u32, |w, (k, v)| w | (((v * 16.0).round().clamp(0.0, 16.0) as u32) << (k * 8)));
        let mut words: Vec<u32> = Vec::with_capacity(views.len() * 4);
        for v in &views {
            let kind = match v.kind {
                TopKind::Air => 0,
                TopKind::Block => 1,
                TopKind::Overlay => 2,
                TopKind::Water => 3,
                TopKind::Lava => 4,
            };
            let rgb = |c: [u8; 3]| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16;
            words.extend_from_slice(&[v.layer as u32 | kind << 16 | (v.light as u32) << 24, rgb(v.tint) | (v.mean[3] as u32) << 24, pack_px(v.uv), pack_px(v.area)]);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&words);
        if bytes.len() as u64 > self.materials_buf.size() {
            self.materials_buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif diorama materials"), size: (bytes.len() as u64 * 2).next_power_of_two(), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.bind_group = None;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.materials_buf, 0, bytes);
        }
        let have = self.atlas.0.size().depth_or_array_layers;
        if layers.len() as u32 > have {
            let most = self.device.limits().max_texture_array_layers;
            let room = (layers.len() as u32).next_power_of_two().clamp(16, most.max(1));
            self.atlas = crate::gpu::Renderer::make_array(&self.device, room, wgpu::TextureFormat::Rgba8UnormSrgb);
            self.atlas_layers = 0;
            self.bind_group = None;
        }
        let room = self.atlas.0.size().depth_or_array_layers as usize;
        for (k, layer) in layers.iter().enumerate().take(room).skip(self.atlas_layers as usize) {
            crate::gpu::Renderer::write_into(&self.queue, &self.atlas.0, k as u32, layer, true);
        }
        self.atlas_layers = layers.len().min(room) as u32;
        self.water = materials.id("minecraft:water", &[("level", "0")]) as u32;
        self.lava = materials.id("minecraft:lava", &[("level", "0")]) as u32;
        self.materials_revision = revision;
    }

    /// Put a tile's small image on the card.
    pub fn put_far(&mut self, tile: TilePos, rgba: &[u8]) {
        if !self.holds(tile) || rgba.len() != (self.far_size * self.far_size * 4) as usize {
            return;
        }
        let next = self.far_slot.len() as u32;
        let slot = *self.far_slot.entry(tile).or_insert(next);
        if slot >= self.far_side * self.far_side {
            return;
        }
        let (sx, sz) = (slot % self.far_side * self.far_size, slot / self.far_side * self.far_size);
        let mut level = rgba.to_vec();
        let mut side = self.far_size as usize;
        let mut mip = 0;
        loop {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.far, mip_level: mip, origin: wgpu::Origin3d { x: sx >> mip, y: sz >> mip, z: 0 }, aspect: wgpu::TextureAspect::All },
                &level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side as u32 * 4), rows_per_image: Some(side as u32) },
                wgpu::Extent3d { width: side as u32, height: side as u32, depth_or_array_layers: 1 },
            );
            if side == 1 {
                break;
            }
            level = half(&level, side);
            side /= 2;
            mip += 1;
        }
        self.set_table(tile, None, Some(slot));
    }

    fn set_table(&mut self, tile: TilePos, near: Option<Option<u32>>, far: Option<u32>) {
        if !self.holds(tile) {
            return;
        }
        let i = ((tile.z - self.table_origin[1]) as u32 * self.table_size[0] + (tile.x - self.table_origin[0]) as u32) as usize;
        let w = &mut self.table_words[i];
        if let Some(n) = near {
            *w = (*w & 0xffff_0000) | n.map_or(0, |s| s + 1);
        }
        if let Some(f) = far {
            *w = (*w & 0xffff) | (f + 1) << 16;
        }
        self.table_dirty = true;
    }

    /// Forget the floors on the card of these tiles (built again since).
    pub fn forget(&mut self, tiles: &[TilePos]) {
        for t in tiles {
            if let Some(slot) = self.near_slots.remove(t) {
                self.near_tile[slot as usize] = None;
                self.set_table(*t, Some(None), None);
            }
        }
    }

    /// Put the floors of the tiles in view on the card, a few a frame, for
    /// `slice`; the tiles without floors at hand are returned, to build.
    pub fn put_near(&mut self, wanted: &[TilePos], floors: &dyn Fn(TilePos) -> Option<Arc<TileFloors>>, slice: Option<i32>) -> Vec<TilePos> {
        self.frame += 1;
        if self.packed_slice != Some(slice) {
            // Another height: every slot is packed again.
            for t in std::mem::take(&mut self.near_slots).into_keys() {
                self.set_table(t, Some(None), None);
            }
            self.near_tile.iter_mut().for_each(|t| *t = None);
            self.packed_slice = Some(slice);
        }
        let mut missing = Vec::new();
        let mut uploads = 0;
        for &t in wanted.iter().take((NEAR_SIDE * NEAR_SIDE) as usize) {
            if let Some(&slot) = self.near_slots.get(&t) {
                self.near_used[slot as usize] = self.frame;
                continue;
            }
            let Some(f) = floors(t) else {
                missing.push(t);
                continue;
            };
            if uploads >= UPLOADS {
                continue;
            }
            // A free slot, or the one used the longest ago (not this frame).
            let Some(slot) = (0..self.near_used.len()).filter(|&s| self.near_used[s] != self.frame).min_by_key(|&s| (self.near_tile[s].is_some(), self.near_used[s])) else { break };
            if let Some(old) = self.near_tile[slot].take() {
                self.near_slots.remove(&old);
                self.set_table(old, Some(None), None);
            }
            f.pack(slice, self.min_y, &mut self.pack);
            let (sx, sz) = (slot as u32 % NEAR_SIDE * TILE as u32, slot as u32 / NEAR_SIDE * TILE as u32);
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.near, mip_level: 0, origin: wgpu::Origin3d { x: sx, y: sz, z: 0 }, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&self.pack),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(TILE as u32 * 8), rows_per_image: Some(TILE as u32) },
                wgpu::Extent3d { width: TILE as u32, height: TILE as u32, depth_or_array_layers: 1 },
            );
            self.near_tile[slot] = Some(t);
            self.near_used[slot] = self.frame;
            self.near_slots.insert(t, slot as u32);
            self.set_table(t, Some(Some(slot as u32)), None);
            uploads += 1;
        }
        missing
    }

    /// Tiles whose floors are on the card.
    pub fn near_count(&self) -> usize {
        self.near_slots.len()
    }

    /// How many tiles the card can draw block by block at once.
    pub fn near_capacity() -> usize {
        (NEAR_SIDE * NEAR_SIDE) as usize
    }

    /// Make the image `width × height` pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let size = [width.max(1), height.max(1)];
        if self.image.as_ref().is_some_and(|i| i.size == size) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif diorama image"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::gpu::IMAGE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let storage = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Its bytes as they are (sRGB already), which is how egui shows an
        // image: an sRGB view would decode them once more, darker.
        let view = texture.create_view(&wgpu::TextureViewDescriptor { usage: Some(wgpu::TextureUsages::TEXTURE_BINDING), ..Default::default() });
        self.image = Some(Image { size, texture, storage, view });
        self.bind_group = None;
    }

    pub fn size(&self) -> Option<[u32; 2]> {
        self.image.as_ref().map(|i| i.size)
    }

    /// The image to show, its bytes sRGB colours.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.image.as_ref().map(|i| &i.view)
    }

    /// The image's texture (to read back in tests).
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.image.as_ref().map(|i| &i.texture)
    }

    /// Draw `view` into the image; `near`: block by block where the floors
    /// are on the card.
    pub fn render(&mut self, view: &View, near: bool) -> Option<wgpu::CommandBuffer> {
        let image = self.image.as_ref()?;
        if self.table_dirty {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.table, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&self.table_words),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.table_size[0] * 4), rows_per_image: Some(self.table_size[1]) },
                wgpu::Extent3d { width: self.table_size[0], height: self.table_size[1], depth_or_array_layers: 1 },
            );
            self.table_dirty = false;
        }
        let origin = [(view.center[0] / TILE as f64).floor() as i32, (view.center[1] / TILE as f64).floor() as i32];
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let params = Params {
            center: [(view.center[0] - origin[0] as f64 * TILE as f64) as f32, (view.center[1] - origin[1] as f64 * TILE as f64) as f32],
            blocks_per_px: view.blocks_per_px,
            turn: view.turn as u32 & 3,
            size: image.size,
            table: [self.table_origin[0], self.table_origin[1], self.table_size[0] as i32, self.table_size[1] as i32],
            origin,
            slice: view.slice.unwrap_or(0),
            caves: view.slice.is_some() as u32,
            min_y: self.min_y,
            near_side: NEAR_SIDE,
            far_size: self.far_size,
            far_side: self.far_side,
            background: [lin(view.background[0]), lin(view.background[1]), lin(view.background[2]), 1.0],
            water: self.water,
            lava: self.lava,
            relief: view.relief,
            use_near: near as u32,
            pad: [0; 2],
        };
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        if self.bind_group.is_none() {
            let table = self.table.create_view(&Default::default());
            let near = self.near.create_view(&Default::default());
            let far = self.far.create_view(&Default::default());
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif diorama"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&table) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&near) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&far) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.materials_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.atlas.1) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.nearest) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.linear) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&image.storage) },
                ],
            }));
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif diorama") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("massif diorama"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, self.bind_group.as_ref(), &[]);
            pass.dispatch_workgroups(image.size[0].div_ceil(8), image.size[1].div_ceil(8), 1);
        }
        Some(encoder.finish())
    }

    /// Read the image back, RGBA (tests and stills).
    pub fn read_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.image.as_ref()?;
        let [w, h] = image.size;
        let row = (w * 4).next_multiple_of(256);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif diorama read"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &image.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..][..w as usize * 4]);
        }
        drop(data);
        buffer.unmap();
        Some((w, h, out))
    }
}

/// The size of a tile's small image, and the slots on a side of their
/// atlas, for a map of `count` tiles: as large as an atlas of 4096 pixels
/// allows, at most 64 (two blocks a pixel).
pub fn far_layout(count: usize) -> (u32, u32) {
    let side = (count.max(1) as f64).sqrt().ceil() as u32;
    let mut size = 64u32;
    while size > 4 && side * size > 4096 {
        size /= 2;
    }
    (size, side.max(1))
}

/// An sRGB image half as large on a side (plain averages).
fn half(level: &[u8], side: usize) -> Vec<u8> {
    let h = side / 2;
    let mut out = vec![0u8; h * h * 4];
    for y in 0..h {
        for x in 0..h {
            for k in 0..4 {
                let s: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| level[((y * 2 + dy) * side + x * 2 + dx) * 4 + k] as u32).sum();
                out[(y * h + x) * 4 + k] = (s / 4) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn far_images_fit_the_atlas() {
        assert_eq!(far_layout(1024), (64, 32));
        assert_eq!(far_layout(16384), (32, 128));
        assert_eq!(far_layout(1), (64, 1));
    }

    #[test]
    fn floors_seen_from_above_and_at_a_height() {
        let f = |y: i16, block: u16| Floor { y, block, over: 0, depth: 0, lava: false };
        let t = TileFloors { starts: std::iter::once(0).chain(std::iter::repeat_n(3, AREA)).collect(), floors: vec![f(80, 1), f(40, 2), f(-20, 3)], surface: Vec::new() };
        assert_eq!(t.seen(0, None).map(|f| f.block), Some(1));
        assert_eq!(t.seen(0, Some(60)).map(|f| f.block), Some(2));
        assert_eq!(t.seen(0, Some(41)).map(|f| f.block), Some(2));
        assert_eq!(t.seen(0, Some(40)).map(|f| f.block), Some(3));
        assert_eq!(t.seen(0, Some(-30)), None);
        assert_eq!(t.seen(1, None), None);
    }

    #[test]
    fn flat_ground_keeps_its_colour() {
        let k = relief(64.0, 64.0, 64.0, 64.0, 64.0);
        assert!((k - 1.0).abs() < 0.01, "{k}");
        // A slope facing the light (north-west) is lit, one facing away
        // in shade.
        assert!(relief(70.0, 64.0, 76.0, 64.0, 76.0) > k);
        assert!(relief(70.0, 76.0, 64.0, 76.0, 64.0) < k);
    }
}
