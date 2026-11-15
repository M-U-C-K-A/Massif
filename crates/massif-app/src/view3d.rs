//! The 3D preview: the whole map, every block.
//!
//! Most columns of a map are one span of ground resting on the bottom of
//! the world, so the map is mostly a height field. Those columns are drawn
//! by a ray tracer on the GPU, from textures holding, for every column, its
//! height and its colours in 4 bytes: every block of the whole map is on
//! screen at once, at any distance. A max pyramid (the highest column of
//! every 2 × 2, 4 × 4… square) lets the rays skip the air above the ground;
//! only a square smaller than a pixel is drawn as one box.
//!
//! The other columns — overhangs, floating islands, caves open to the side —
//! and the water are meshed, one chunk per tile: every span gets its top,
//! its underside and the walls its neighbours do not cover, faces of the
//! same height and colour are merged, and each face is one 12-byte instance
//! the vertex shader turns into a quad. Tracer and meshes write depth, so
//! they meet exactly.
//!
//! Tiles are rebuilt only where the world changed, nearest the camera first,
//! a few at a time in parallel, within a time budget per frame.

use crate::doc::Document;
use crate::map_render::{AutoBiome, ColumnSampler, MapMode, Sample, height_color, slope_color};
use crate::theme::{self, Icon, Palette};
use crate::tools::{SculptStroke, Tool, ToolState};
use bytemuck::{Pod, Zeroable};
use egui::{Align2, Color32, FontId, PointerButton, Rect, Sense, Ui, pos2, vec2};
use glam::{Mat4, Vec3, Vec4};
use massif_core::compose::{Materializer, Span, TileStack};
use massif_core::coords::{TILE_SIZE, TilePos, local_index};
use massif_core::filter::{SLOPE_REACH, trend_slope};
use massif_core::registry::{BlockId, Registry, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::world::Dimension;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Time spent building tiles per frame, once something is on screen.
const BUILD_BUDGET: Duration = Duration::from_millis(14);
/// From how many tiles to build the work goes away from the frames (see
/// [`Builder`]).
const BUILD_AWAY: usize = 32;
/// The most time a frame spends putting built tiles on the graphics card;
/// while a view moves, less.
const UPLOAD_BUDGET: Duration = Duration::from_millis(6);
const UPLOAD_BUDGET_MOVING: Duration = Duration::from_millis(2);
/// Vertical field of view of the camera.
const FOV_Y: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Walking, Minecraft's (wider: the land is all around).
const FOV_WALK: f32 = 70.0 * std::f32::consts::PI / 180.0;
/// Size of one chunk's slot in the uniform buffer of chunk origins.
const SLOT: u64 = 256;
/// The height of the game's clouds (since 1.18).
const CLOUDS_Y: f32 = 192.0;

/// Columns per side of a page of the tracer's textures.
pub const PAGE: i32 = 1024;
const PAGE_SHIFT: u32 = 10;
/// Tiles per side of a page.
const PAGE_TILES: i32 = PAGE / TILE_SIZE;
/// Colour pairs (top, soil) the tracer tells apart.
pub const MAX_PAIRS: usize = 4096;
/// Pages the tracer may hold (about 1.9 GB). A larger map is drawn with a
/// column every 2, 4, 8… blocks: still all of it, coarser.
const PAGE_BUDGET: usize = 400;
/// Bits of a packed column holding its height above the bottom of the world
/// (0: nothing for the tracer); the colour pair takes the next 12.
const HEIGHT_BITS: u32 = 12;
const HEIGHT_MASK: u32 = (1 << HEIGHT_BITS) - 1;

/// Directions of faces.
pub const UP: u8 = 0;
pub const DOWN: u8 = 1;
pub const EAST: u8 = 2;
pub const WEST: u8 = 3;
pub const SOUTH: u8 = 4;
pub const NORTH: u8 = 5;

/// One face, drawn as an instanced quad. Positions are in blocks from the
/// north-west corner of the chunk.
///
/// * up / down: a `len` × `h` rectangle at height `y`, `len` along x;
/// * east / west: the plane `x`, `len` blocks along z from `z`, `h` high from `y`;
/// * south / north: the plane `z`, `len` blocks along x from `x`, `h` high.
///
/// `mat` says what it is made of, for the game's textures (see
/// [`object_bases`], [`ROCK`]); 0: nothing known, its colour.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct Face {
    pub x: u8,
    pub z: u8,
    pub dir: u8,
    pub len: u8,
    pub y: i16,
    pub h: i16,
    pub color: [u8; 4],
    pub mat: u16,
    pub pad: u16,
}

/// What the top of a column shows and the side of the soil under it: their
/// colours, the top's own colour before what grows on it (`bare`), and the
/// blocks they are, for the game's textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Skin {
    pub top: [u8; 3],
    pub soil: [u8; 3],
    pub bare: [u8; 3],
    pub top_mat: u16,
    pub soil_mat: u16,
}

impl Skin {
    /// Colours alone, of no block.
    pub const fn flat(top: [u8; 3], soil: [u8; 3]) -> Self {
        Self {
            top,
            soil,
            bare: top,
            top_mat: 0,
            soil_mat: 0,
        }
    }
}

/// The faces of one chunk.
#[derive(Debug, Default, Clone)]
pub struct ChunkFaces {
    pub solid: Vec<Face>,
    pub water: Vec<Face>,
}

#[derive(Clone, Copy)]
struct CellSpan {
    y0: i32,
    y1: i32,
    terrain: u16,
    /// The exact height of its top (y1 is the top of its last block).
    top: f32,
    /// Blocks of an object, all of this colour, and of this material (see
    /// [`object_bases`]).
    color: Option<[u8; 3]>,
    mat: u16,
}

/// The materials of the objects' blocks: palette entry `k` of object `o` is
/// material `bases[o] + k`, after the registry's blocks (whose material is
/// their index). The game's textures are looked up by material (see
/// `crate::textures`).
pub fn object_bases(reg: &Registry) -> Vec<u32> {
    let mut next = reg.blocks.len() as u32;
    reg.objects
        .iter()
        .map(|o| {
            let base = next;
            next += o.palette.len() as u32;
            base
        })
        .collect()
}

/// A material as faces carry it: none past the last that fits (it keeps its
/// colour).
fn material(n: u32) -> u16 {
    if n < ROCK as u32 { n as u16 } else { 0 }
}

/// The material of the rock under the soil: stone, or deepslate under y 0,
/// told apart where it is drawn.
pub const ROCK: u16 = u16::MAX;

/// What stands on the land: the objects of the world's layers.
#[derive(Clone, Copy)]
struct Standing<'a> {
    reg: &'a Registry,
    seed: i64,
    lava: bool,
}

/// A run of an object's blocks in a column: `[y0, y1)`, its colour and its
/// material.
type ObjectRun = (i32, i32, [u8; 3], u16);

/// The blocks of the objects standing over the `w` × `d` rectangle at
/// `(x0, z0)`, column by column: runs, low to high.
fn object_runs(dim: &Dimension, s: Standing, x0: i32, z0: i32, w: usize, d: usize) -> Vec<Vec<ObjectRun>> {
    use massif_core::objects::{max_reach, tile_placements};
    let mut runs: Vec<Vec<ObjectRun>> = vec![Vec::new(); w * d];
    let reg = s.reg;
    if !reg.object_layers.iter().any(|l| !l.removed && !l.objects.is_empty()) {
        return runs;
    }
    let bases = object_bases(reg);
    let area = massif_core::BlockRect::new(x0, z0, x0 + w as i32, z0 + d as i32);
    let mat = Materializer::new(reg, s.seed, dim.min_y);
    let mut placed = Vec::new();
    for t in area.expand(max_reach(reg)).tiles() {
        let Some(stack) = TileStack::new(dim, t, false) else { continue };
        for p in tile_placements(reg, &mat, s.seed as u64, s.lava, &stack, t) {
            if reg.objects.get(p.object as usize).is_some_and(|def| !p.bounds(def).intersect(&area).is_empty()) {
                placed.push(p);
            }
        }
    }
    placed.sort_unstable_by_key(|p| (p.layer, p.x, p.z));
    let mut blocks: Vec<Vec<(i32, [u8; 3], u16)>> = vec![Vec::new(); w * d];
    for p in &placed {
        let def = &reg.objects[p.object as usize];
        let base = bases.get(p.object as usize).copied().unwrap_or(0);
        let [bw, bh, bl] = def.size.map(|v| v as i32);
        for bz in 0..bl {
            for bx in 0..bw {
                let (x, y, z) = p.world(def, bx, 0, bz);
                let (i, j) = (x - x0, z - z0);
                if i < 0 || j < 0 || i >= w as i32 || j >= d as i32 {
                    continue;
                }
                let column = &mut blocks[j as usize * w + i as usize];
                for by in 0..bh {
                    let k = def.at(bx as usize, by as usize, bz as usize);
                    if k != 0 {
                        column.push((y + by, def.color(k), material(base + k as u32)));
                    }
                }
            }
        }
    }
    for (c, column) in blocks.iter_mut().enumerate() {
        // The last object standing wins where two meet, as in the export.
        column.reverse();
        column.sort_by_key(|b| b.0);
        column.dedup_by_key(|b| b.0);
        for &(y, color, mat) in column.iter() {
            match runs[c].last_mut() {
                Some(r) if r.1 == y && r.2 == color && r.3 == mat => r.1 = y + 1,
                _ => runs[c].push((y, y + 1, color, mat)),
            }
        }
    }
    runs
}

/// Merged solid spans, surface water and top sample of every column of a
/// rectangle.
struct Grid {
    x0: i32,
    z0: i32,
    w: usize,
    d: usize,
    offsets: Vec<u32>,
    spans: Vec<CellSpan>,
    water: Vec<i32>,
    top: Vec<Sample>,
    /// The cover painted on the top of each column; 0: none.
    cover: Vec<u16>,
}

impl Grid {
    /// Spans of the column at block `(x, z)`; none outside the rectangle.
    fn spans(&self, x: i32, z: i32) -> &[CellSpan] {
        let (i, j) = (x - self.x0, z - self.z0);
        if i < 0 || j < 0 || i >= self.w as i32 || j >= self.d as i32 {
            return &[];
        }
        let c = j as usize * self.w + i as usize;
        &self.spans[self.offsets[c] as usize..self.offsets[c + 1] as usize]
    }

    fn top(&self, x: i32, z: i32) -> Option<f32> {
        self.spans(x, z).last().map(|s| s.top)
    }

    fn cell(&self, x: i32, z: i32) -> usize {
        (z - self.z0) as usize * self.w + (x - self.x0) as usize
    }
}

/// Compose the columns of the `w` × `d` rectangle at `(x0, z0)`, with the
/// objects `standing` on them.
#[allow(clippy::too_many_arguments)]
fn compose_area(dim: &Dimension, x0: i32, z0: i32, w: usize, d: usize, mode: MapMode, water: bool, standing: Option<Standing>) -> Grid {
    let runs = standing.map(|s| object_runs(dim, s, x0, z0, w, d));
    // The tiles the area lies on, each composed once; then its rows side by
    // side (a tile rebuilt alone, as while sculpting, uses every core).
    let (t0, t1) = (TilePos::containing(x0, z0), TilePos::containing(x0 + w as i32 - 1, z0 + d as i32 - 1));
    let tiles: Vec<TilePos> = (t0.z..=t1.z).flat_map(|tz| (t0.x..=t1.x).map(move |tx| TilePos::new(tx, tz))).collect();
    let stacks: HashMap<TilePos, TileStack> = tiles.par_iter().filter_map(|&p| TileStack::new(dim, p, false).map(|s| (p, s))).collect();
    let rows: Vec<GridRow> = (0..d as i32)
        .into_par_iter()
        .map(|j| {
            let z = z0 + j;
            let mut row = GridRow {
                spans: Vec::with_capacity(w * 2),
                ends: Vec::with_capacity(w),
                water: Vec::with_capacity(w),
                top: Vec::with_capacity(w),
                cover: Vec::with_capacity(w),
            };
            let mut sp: Vec<Span> = Vec::new();
            let mut fl: Vec<(i32, i32)> = Vec::new();
            let mut sampler = ColumnSampler::default();
            for i in 0..w as i32 {
                let x = x0 + i;
                let start = row.spans.len();
                let mut wet = i32::MIN;
                let mut top = Sample::EMPTY;
                let mut cover = 0;
                if let Some(stack) = stacks.get(&TilePos::containing(x, z)) {
                    let cell = local_index(x, z);
                    stack.spans(cell, &mut sp);
                    stack.fluids(cell, &sp, &mut fl);
                    for s in &sp {
                        let contiguous = row.spans.len() > start && row.spans.last().is_some_and(|l: &CellSpan| l.y1 == s.y0);
                        if contiguous {
                            let last = row.spans.last_mut().expect("checked above");
                            last.y1 = s.y1;
                            last.terrain = s.terrain.0;
                            last.top = s.top;
                        } else {
                            row.spans.push(CellSpan {
                                y0: s.y0,
                                y1: s.y1,
                                terrain: s.terrain.0,
                                top: s.top,
                                color: None,
                                mat: 0,
                            });
                        }
                    }
                    if water
                        && let (Some(&(f0, f1)), Some(t)) = (fl.last(), sp.last())
                        && f0 == t.y1
                    {
                        wet = f1;
                    }
                    if mode == MapMode::Biome {
                        top = sampler.sample(stack, cell);
                    }
                    if let Some(t) = sp.last() {
                        cover = stack.surface_cover(cell, t);
                    }
                }
                // The objects over the column, where it is air.
                if let Some(runs) = &runs {
                    let here = &runs[(j as usize) * w + i as usize];
                    if !here.is_empty() {
                        let ground: Vec<(i32, i32)> = row.spans[start..].iter().map(|s| (s.y0, s.y1)).collect();
                        let mut free = Vec::new();
                        for &(y0, y1, color, mat) in here {
                            let solid: Vec<CellSpan> = ground.iter().map(|&(a, b)| CellSpan { y0: a, y1: b, terrain: 0, top: b as f32, color: None, mat: 0 }).collect();
                            uncovered(y0, y1, &solid, &mut free);
                            for &(a, b) in &free {
                                row.spans.push(CellSpan { y0: a, y1: b, terrain: 0, top: b as f32, color: Some(color), mat });
                            }
                        }
                        row.spans[start..].sort_by_key(|s| s.y0);
                    }
                }
                row.ends.push(row.spans.len() as u32);
                row.water.push(wet);
                row.top.push(top);
                row.cover.push(cover);
            }
            row
        })
        .collect();
    let mut grid = Grid {
        x0,
        z0,
        w,
        d,
        offsets: Vec::with_capacity(w * d + 1),
        spans: Vec::with_capacity(rows.iter().map(|r| r.spans.len()).sum()),
        water: Vec::with_capacity(w * d),
        top: Vec::with_capacity(w * d),
        cover: Vec::with_capacity(w * d),
    };
    grid.offsets.push(0);
    for row in rows {
        let base = grid.spans.len() as u32;
        grid.spans.extend(row.spans);
        grid.offsets.extend(row.ends.iter().map(|e| base + e));
        grid.water.extend(row.water);
        grid.top.extend(row.top);
        grid.cover.extend(row.cover);
    }
    grid
}

/// A row of a [`Grid`] composed on its own: its columns' spans, where each
/// column's end (within the row), water, top and cover.
struct GridRow {
    spans: Vec<CellSpan>,
    ends: Vec<u32>,
    water: Vec<i32>,
    top: Vec<Sample>,
    cover: Vec<u16>,
}

/// What chunks are drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub mode: MapMode,
    pub water: bool,
    /// The world's fluid is lava.
    pub lava: bool,
}

struct Colors<'a> {
    reg: &'a Registry,
    mode: MapMode,
    sea: f32,
    auto: AutoBiome,
    stone: [u8; 3],
    deepslate: [u8; 3],
    /// Picks the exported blocks, so mixed terrains look speckled as they will in the game.
    mat: Materializer<'a>,
    seed: u64,
    /// The world's fluid is lava: nothing grows in it.
    lava: bool,
}

impl Colors<'_> {
    fn block(&self, id: BlockId) -> [u8; 3] {
        self.reg.block(id).color
    }

    /// Colour of the top face of a span, and of the soil just under it, for
    /// the column `(x, z)` whose ground slopes `slope` degrees, with the
    /// blocks they are. On the column's top, what the cover numbered
    /// `cover` grows there shows over the ground (`wet`: the top is under
    /// water).
    #[allow(clippy::too_many_arguments)]
    fn top(&self, terrain: u16, y0: i32, y1: i32, x: i32, z: i32, slope: f32, sample: &Sample, cover: u16, wet: bool) -> Skin {
        let flat = |c: [u8; 3]| Skin::flat(c, c);
        match self.mode {
            MapMode::Terrain => {
                let span = Span {
                    y0,
                    y1,
                    terrain: TerrainId(terrain),
                    top: y1 as f32,
                };
                let block = self.mat.block(&span, x, y1 - 1, z, false);
                let bare = self.block(block);
                let mut top = bare;
                if let Some(def) = self.reg.cover(cover)
                    && def.underwater == wet
                    && !(wet && self.lava)
                    && (!def.soil_only || massif_core::registry::is_soil(&self.reg.block(block).state))
                    && let Some(g) = def.growth(x, z, self.seed)
                {
                    let plant = def.plants.get(g.plant).and_then(|p| massif_core::registry::plant_color(&p.0)).unwrap_or(def.color);
                    let k = 0.75;
                    top = std::array::from_fn(|c| (top[c] as f32 + (plant[c] as f32 - top[c] as f32) * k) as u8);
                }
                let under = if y1 - 2 >= y0 { self.mat.block(&span, x, y1 - 2, z, false) } else { block };
                Skin {
                    top,
                    soil: self.block(under),
                    bare,
                    top_mat: block.0,
                    soil_mat: under.0,
                }
            }
            MapMode::Height => flat(height_color(y1 as f32, self.sea)),
            // The walls of a column are as steep as its top: its colour.
            MapMode::Slope => flat(slope_color(slope)),
            MapMode::Biome => {
                let id = self.auto.resolve(self.reg, sample, self.sea);
                flat(self.reg.biome(massif_core::BiomeId(id)).color)
            }
            MapMode::Relief => Skin::flat([206, 206, 210], [190, 190, 194]),
        }
    }

    /// Whether a column's walls are its colour all the way down (what the
    /// view shows is the column's: its height, its slope, its biome), not
    /// soil over rock.
    fn whole_walls(&self) -> bool {
        matches!(self.mode, MapMode::Height | MapMode::Slope | MapMode::Biome)
    }

    fn substrate(&self, y: i32) -> [u8; 3] {
        match self.mode {
            MapMode::Relief => [176, 176, 180],
            _ if y < 0 => self.deepslate,
            _ => self.stone,
        }
    }
}

fn rgba(c: [u8; 3], a: u8) -> [u8; 4] {
    [c[0], c[1], c[2], a]
}

/// Parts of `[a, b)` not covered by `cover` (sorted, disjoint).
fn uncovered(a: i32, b: i32, cover: &[CellSpan], out: &mut Vec<(i32, i32)>) {
    out.clear();
    let mut y = a;
    for s in cover {
        if s.y1 <= y {
            continue;
        }
        if s.y0 >= b {
            break;
        }
        if s.y0 > y {
            out.push((y, s.y0));
        }
        y = y.max(s.y1);
        if y >= b {
            return;
        }
    }
    if y < b {
        out.push((y, b));
    }
}

/// Merge equal cells of an `n` × `n` grid into rectangles (greedy meshing).
/// `key(i, j)` is `None` where there is nothing to draw; `emit(i, j, w, h,
/// key)` receives each rectangle.
fn greedy<K: PartialEq + Copy>(n: usize, key: impl Fn(usize, usize) -> Option<K>, mut emit: impl FnMut(usize, usize, usize, usize, K)) {
    let keys: Vec<Option<K>> = (0..n * n).map(|c| key(c % n, c / n)).collect();
    let mut done = vec![false; n * n];
    for j in 0..n {
        let mut i = 0;
        while i < n {
            let c = j * n + i;
            let Some(k) = keys[c].filter(|_| !done[c]) else {
                i += 1;
                continue;
            };
            let mut w = 1;
            while i + w < n && !done[c + w] && keys[c + w] == Some(k) {
                w += 1;
            }
            let mut h = 1;
            'grow: while j + h < n {
                for di in 0..w {
                    let c2 = (j + h) * n + i + di;
                    if done[c2] || keys[c2] != Some(k) {
                        break 'grow;
                    }
                }
                h += 1;
            }
            for dj in 0..h {
                for di in 0..w {
                    done[(j + dj) * n + i + di] = true;
                }
            }
            emit(i, j, w, h, k);
            i += w;
        }
    }
}

/// A piece of wall: `[y0, y1)`, its colour and its material.
type Piece = (i32, i32, [u8; 4], u16);

/// Merges wall pieces of one line of columns: a piece continues the run of
/// the previous column when its band, colour and material are the same.
struct WallRuns {
    open: HashMap<Piece, (usize, usize)>,
}

impl WallRuns {
    fn new() -> Self {
        Self { open: HashMap::new() }
    }

    /// A piece at position `t` along the line.
    fn add(&mut self, t: usize, piece: Piece) {
        let run = self.open.entry(piece).or_insert((t, t));
        if run.1 + 1 == t {
            run.1 = t;
        } else if run.1 != t {
            *run = (t, t);
        }
    }

    /// Close the runs that did not continue to `t`, or all of them.
    fn flush(&mut self, t: Option<usize>, mut emit: impl FnMut(usize, usize, Piece)) {
        self.open.retain(|&piece, &mut (a, b)| {
            if t.is_some_and(|t| b + 1 >= t) {
                return true;
            }
            emit(a, b - a + 1, piece);
            false
        });
    }
}

/// What one tile gives the preview.
#[derive(Debug, Default, Clone)]
pub struct TileBuild {
    /// Per column (row-major), the height of its ground above the bottom of
    /// the world, for the tracer; 0 where the tracer has nothing to draw.
    pub heights: Vec<u16>,
    /// Per column, its colours: an index into `pairs`.
    pub colors: Vec<u16>,
    /// Per column, how deep under water its top is (empty: dry): the game
    /// darkens what lies deep.
    pub depths: Vec<u8>,
    /// What the tile's traced columns show: colours and blocks.
    pub pairs: Vec<Skin>,
    /// Faces of the columns the tracer leaves out, and the water.
    pub faces: ChunkFaces,
}

/// Build tile `tile`: heights and colours of its columns of plain ground,
/// faces of the others and of its water.
pub fn build_tile(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, tile: TilePos, style: Style) -> TileBuild {
    let n = TILE_SIZE as usize;
    let (cx, cz) = (tile.x * TILE_SIZE, tile.z * TILE_SIZE);
    // The tile and a ring of neighbour columns, for walls, and for slopes,
    // measured as the trend of the ground around each column.
    let ring = if style.mode == MapMode::Slope { SLOPE_REACH } else { 1 };
    let standing = Standing { reg, seed, lava: style.lava };
    let grid = compose_area(dim, cx - ring, cz - ring, n + 2 * ring as usize, n + 2 * ring as usize, style.mode, style.water, Some(standing));
    let wk = reg.well_known;
    let colors = Colors {
        reg,
        mode: style.mode,
        sea,
        auto: AutoBiome::new(reg),
        stone: reg.block(wk.stone).color,
        deepslate: reg.block(wk.deepslate).color,
        mat: Materializer::new(reg, seed, dim.min_y),
        seed: seed as u64,
        lava: style.lava,
    };
    let (min_y, max_y) = (dim.min_y, dim.max_y);
    let clamp16 = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    // A column the tracer draws: one span from the bottom of the world.
    let traced = |x: i32, z: i32| {
        let s = grid.spans(x, z);
        s.len() == 1 && s[0].y0 <= min_y
    };
    // The others, when they hold anything, are meshed.
    let meshed = |x: i32, z: i32| !grid.spans(x, z).is_empty() && !traced(x, z);

    // Colours of the tops of every span, and of the soil under them.
    let mut tops: Vec<Vec<Skin>> = Vec::with_capacity(n * n);
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            let (x, z) = (cx + i, cz + j);
            let slope = if style.mode == MapMode::Slope {
                trend_slope(|dx, dz| grid.top(x + dx, z + dz), SLOPE_REACH, 1.0)
            } else {
                0.0
            };
            let c = grid.cell(x, z);
            let (sample, cover, wet) = (grid.top[c], grid.cover[c], grid.water[c] != i32::MIN);
            let spans = grid.spans(x, z);
            // The cover grows on the top of the land, under any object.
            let land = spans.iter().rposition(|s| s.color.is_none());
            tops.push(
                spans
                    .iter()
                    .enumerate()
                    .map(|(k, s)| {
                        if let Some(c) = s.color {
                            return Skin {
                                top_mat: s.mat,
                                soil_mat: s.mat,
                                ..Skin::flat(c, c)
                            };
                        }
                        let on_top = Some(k) == land;
                        colors.top(s.terrain, s.y0, s.y1, x, z, slope, &sample, if on_top { cover } else { 0 }, on_top && wet)
                    })
                    .collect(),
            );
        }
    }

    // The traced columns: height, colours and the water over them.
    let mut heights = vec![0u16; n * n];
    let mut column_colors = vec![0u16; n * n];
    let mut depths = vec![0u8; n * n];
    let mut pairs: Vec<Skin> = Vec::new();
    let mut known: HashMap<Skin, u16> = HashMap::new();
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (cx + i as i32, cz + j as i32);
            if !traced(x, z) {
                continue;
            }
            let c = j * n + i;
            let top = grid.spans(x, z)[0].y1;
            heights[c] = (top - min_y).clamp(1, HEIGHT_MASK as i32) as u16;
            let wet = grid.water[grid.cell(x, z)];
            if wet != i32::MIN {
                depths[c] = (wet - top).clamp(0, 255) as u8;
            }
            let pair = tops[c][0];
            column_colors[c] = *known.entry(pair).or_insert_with(|| {
                pairs.push(pair);
                (pairs.len() - 1) as u16
            });
        }
    }

    let face = |x: u8, z: u8, dir: u8, len: u8, y: i16, h: i16, color: [u8; 4], mat: u16| Face {
        x,
        z,
        dir,
        len,
        y,
        h,
        color,
        mat,
        pad: 0,
    };
    let mut out = ChunkFaces::default();
    // Tops of the highest spans of meshed columns, merged into rectangles;
    // other tops and the undersides one by one (overhangs, islands and caves
    // are small).
    greedy(
        n,
        |i, j| {
            let (x, z) = (cx + i as i32, cz + j as i32);
            if !meshed(x, z) {
                return None;
            }
            let s = grid.spans(x, z).last()?;
            let skin = tops[j * n + i].last().expect("one colour per span");
            (s.y1 < max_y).then(|| (s.y1, rgba(skin.top, 255), skin.top_mat))
        },
        |i, j, w, h, (y, c, mat)| out.solid.push(face(i as u8, j as u8, UP, w as u8, clamp16(y), h as i16, c, mat)),
    );
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (cx + i as i32, cz + j as i32);
            if !meshed(x, z) {
                continue;
            }
            let spans = grid.spans(x, z);
            for (k, s) in spans.iter().enumerate() {
                let skin = tops[j * n + i][k];
                if k + 1 < spans.len() && s.y1 < max_y {
                    out.solid.push(face(i as u8, j as u8, UP, 1, clamp16(s.y1), 1, rgba(skin.top, 255), skin.top_mat));
                }
                if s.y0 > min_y {
                    let (under, mat) = match s.color {
                        Some(c) => (c.map(|v| (v as f32 * 0.6) as u8), s.mat),
                        None if colors.whole_walls() => (theme_dark(skin.soil), 0),
                        None => (theme_dark(colors.substrate(s.y0)), ROCK),
                    };
                    out.solid.push(face(i as u8, j as u8, DOWN, 1, clamp16(s.y0), 1, rgba(under, 255), mat));
                }
            }
        }
    }

    // Walls of the meshed columns, split into the top block, the soil under
    // it and the rock below, merged along their length. (The tracer draws
    // the walls of the other columns, toward everything it does not draw
    // itself.)
    let mut gaps = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    // The pieces of wall the column `(x, z)` shows toward `(nx, nz)`.
    let mut wall = |x: i32, z: i32, nx: i32, nz: i32, pieces: &mut Vec<Piece>| {
        pieces.clear();
        let (i, j) = ((x - cx) as usize, (z - cz) as usize);
        for (k, s) in grid.spans(x, z).iter().enumerate() {
            uncovered(s.y0, s.y1, grid.spans(nx, nz), &mut gaps);
            if let Some(c) = s.color {
                pieces.extend(gaps.iter().map(|&(a, b)| (a, b, rgba(c, 255), s.mat)));
                continue;
            }
            let skin = tops[j * n + i][k];
            let soil_floor = if colors.whole_walls() { s.y0 } else { (s.y1 - 3).max(s.y0) };
            let top_floor = (s.y1 - 1).max(soil_floor);
            for &(a, b) in &gaps {
                let soil = rgba(skin.soil, 255);
                for (lo, hi, c, mat) in [
                    (a, b.min(soil_floor), rgba(colors.substrate(a), 255), ROCK),
                    (a.max(soil_floor), b.min(top_floor), soil, skin.soil_mat),
                    (a.max(top_floor), b, soil, skin.top_mat),
                ] {
                    if hi <= lo {
                        continue;
                    }
                    // Pieces alike one on the other make one.
                    match pieces.last_mut() {
                        Some(last) if last.1 == lo && last.2 == c && last.3 == mat => last.1 = hi,
                        _ => pieces.push((lo, hi, c, mat)),
                    }
                }
            }
        }
    };
    // East and west walls run along z, south and north walls along x.
    for (dir, (dx, dz)) in [(EAST, (1, 0)), (WEST, (-1, 0)), (SOUTH, (0, 1)), (NORTH, (0, -1))] {
        let along_z = dx != 0;
        for line in 0..n {
            let plane = (line as i32 + (dx + dz).max(0)) as u8;
            let mut runs = WallRuns::new();
            let solid = &mut out.solid;
            let mut emit = |start: usize, len: usize, (y0, y1, c, mat): Piece| {
                let (x, z) = if along_z { (plane, start as u8) } else { (start as u8, plane) };
                solid.push(face(x, z, dir, len as u8, clamp16(y0), clamp16(y1 - y0), c, mat));
            };
            for t in 0..n {
                runs.flush(Some(t), &mut emit);
                let (i, j) = if along_z { (line, t) } else { (t, line) };
                let (x, z) = (cx + i as i32, cz + j as i32);
                if !meshed(x, z) {
                    continue;
                }
                wall(x, z, x + dx, z + dz, &mut pieces);
                for &piece in &pieces {
                    runs.add(t, piece);
                }
            }
            runs.flush(None, &mut emit);
        }
    }

    // Water: its surface merged into rectangles, and its sides wherever
    // it stands over what is beside it — a lower water (the steps of a
    // river coming down: falls, as in the game), lower ground, the end of
    // the map.
    if style.water {
        let blue = if style.lava { [238, 112, 22, 255] } else { [26, 92, 150, 205] };
        greedy(
            n,
            |i, j| {
                let wet = grid.water[grid.cell(cx + i as i32, cz + j as i32)];
                (wet != i32::MIN).then_some(wet)
            },
            |i, j, w, h, wet| out.water.push(face(i as u8, j as u8, UP, w as u8, clamp16(wet), h as i16, blue, 0)),
        );
        for j in 0..n {
            for i in 0..n {
                let (x, z) = (cx + i as i32, cz + j as i32);
                let wet = grid.water[grid.cell(x, z)];
                if wet == i32::MIN {
                    continue;
                }
                let floor = grid.spans(x, z).last().map_or(min_y, |s| s.y1);
                for (dir, (dx, dz)) in [(EAST, (1, 0)), (WEST, (-1, 0)), (SOUTH, (0, 1)), (NORTH, (0, -1))] {
                    let (nx, nz) = (x + dx, z + dz);
                    let inside = nx >= cx - 1 && nz >= cz - 1 && nx <= cx + n as i32 && nz <= cz + n as i32;
                    if !inside {
                        continue;
                    }
                    // Up from what is beside: its water, its ground.
                    let bottom = match grid.spans(nx, nz).last() {
                        None => floor,
                        Some(s) => {
                            let beside = grid.water[grid.cell(nx, nz)];
                            s.y1.max(beside).max(floor)
                        }
                    };
                    if bottom >= wet {
                        continue;
                    }
                    let (px, pz) = (i as i32 + dx.max(0), j as i32 + dz.max(0));
                    out.water.push(face(px as u8, pz as u8, dir, 1, clamp16(bottom), clamp16(wet - bottom), blue, 0));
                }
            }
        }
    }
    TileBuild {
        heights,
        colors: column_colors,
        depths,
        pairs,
        faces: out,
    }
}

fn theme_dark(c: [u8; 3]) -> [u8; 3] {
    [c[0] / 2, c[1] / 2, c[2] / 2]
}

/// The colour pairs of the whole map: what the 12 bits of a packed column
/// point at.
#[derive(Debug, Default)]
pub struct Pairs {
    index: HashMap<Skin, u16>,
    list: Vec<Skin>,
    /// Pairs were added since the GPU last had them.
    dirty: bool,
}

impl Pairs {
    /// The index of a pair, added if new; once all are taken, the closest.
    fn get(&mut self, pair: Skin) -> u16 {
        if let Some(&k) = self.index.get(&pair) {
            return k;
        }
        if self.list.is_empty() {
            // Index 0: what shows where a far pixel lands between columns.
            self.list.push(Skin::flat([128, 128, 128], [112, 112, 112]));
        }
        let k = if self.list.len() < MAX_PAIRS {
            self.list.push(pair);
            self.dirty = true;
            (self.list.len() - 1) as u16
        } else {
            let d = |a: [u8; 3], b: [u8; 3]| (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2)).sum::<i32>();
            // The same blocks first, then the closest colours.
            (1..self.list.len())
                .min_by_key(|&k| {
                    let o = &self.list[k];
                    let blocks = (o.top_mat != pair.top_mat) as i32 + (o.soil_mat != pair.soil_mat) as i32;
                    (blocks, d(o.top, pair.top) + d(o.soil, pair.soil))
                })
                .unwrap_or(0) as u16
        };
        self.index.insert(pair, k);
        k
    }

    /// The columns of a tile, packed for the tracer: height, then colours,
    /// then the depth of water over it.
    pub fn pack(&mut self, b: &TileBuild) -> Vec<u32> {
        let map: Vec<u32> = b.pairs.iter().map(|&p| self.get(p) as u32).collect();
        b.heights
            .iter()
            .zip(&b.colors)
            .enumerate()
            .map(|(k, (&h, &c))| if h == 0 { 0 } else { h as u32 | map[c as usize] << HEIGHT_BITS | (b.depths.get(k).copied().unwrap_or(0) as u32) << 24 })
            .collect()
    }

    pub fn list(&self) -> &[Skin] {
        &self.list
    }
}

// ------------------------------------------------------------- rendering --

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    /// The eye, from the frame's origin; w: the origin's x.
    pub eye: [f32; 4],
    pub sun: [f32; 4],
    /// The sky overhead, and at the horizon.
    pub sky: [f32; 4],
    pub horizon: [f32; 4],
    /// Fog start, fog end, ambient weight, sun weight.
    pub fog: [f32; 4],
    /// Colour of the haze far away, and how much of it at most.
    pub mist: [f32; 4],
    /// The camera's forward, right and up directions, for the tracer's rays.
    pub cam_f: [f32; 4],
    pub cam_s: [f32; 4],
    pub cam_u: [f32; 4],
    /// tan(fov / 2) × aspect, tan(fov / 2), radians per pixel, the
    /// frame's origin's z.
    pub cam_k: [f32; 4],
    /// The game's look (the player's view): x, its textures; y, its light,
    /// sky and fog; z, the height of its clouds (0: none); w, the number of
    /// materials the atlas knows.
    pub look: [f32; 4],
    /// The blocks the sculpting brush takes, shown in the player's view:
    /// its centre (from the frame's origin) and radius (0: none); the
    /// surface's normal and the brush's shape (1: round, 2: square); the
    /// direction of a square's side.
    pub brush: [f32; 4],
    pub brush_n: [f32; 4],
    pub brush_t: [f32; 4],
    /// The block looked at (its corner, from the frame's origin); w: 1 when
    /// its outline shows.
    pub pick: [f32; 4],
    /// The materials of stone and deepslate, the layers of water and lava.
    pub rock: [f32; 4],
}

/// Floats in [`Globals`]: what tells a frame from the last.
const GLOBAL_FLOATS: usize = size_of::<Globals>() / 4;

/// Where a chunk is, relative to the origin of the frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ChunkUniform {
    /// x, y, z offset; w = 1 for water.
    origin: [f32; 4],
}

/// What the tracer needs besides its textures.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct TerrainUniform {
    /// x and z of the grid's corner from the origin of the frame, bottom of
    /// the world, top of the highest column.
    origin: [f32; 4],
    /// Columns across, columns down, top level of the max pyramid, blocks
    /// per column.
    size: [i32; 4],
    stone: [f32; 4],
    deepslate: [f32; 4],
}

/// Colours of the rock under the soil, shown on cliffs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Substrate {
    pub stone: [u8; 3],
    pub deepslate: [u8; 3],
    /// Walls are the colour of their column all the way down (Height,
    /// Slope and Biome), not soil over rock.
    pub whole: bool,
}

const GLOBALS_WGSL: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    sky: vec4<f32>,
    horizon: vec4<f32>,
    fog: vec4<f32>,
    mist: vec4<f32>,
    cam_f: vec4<f32>,
    cam_s: vec4<f32>,
    cam_u: vec4<f32>,
    cam_k: vec4<f32>,
    look: vec4<f32>,
    brush: vec4<f32>,
    brush_n: vec4<f32>,
    brush_t: vec4<f32>,
    pick: vec4<f32>,
    rock: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

/// Light and haze of a surface of colour `base` facing `n`, `d` blocks
/// away: the light of the whole sky (strongest from above) and of the sun,
/// so that tops keep their colour and walls stay readable.
fn shade(base: vec3<f32>, n: vec3<f32>, d: f32) -> vec3<f32> {
    let sun = max(dot(n, g.sun.xyz), 0.0);
    let sky = 0.72 + 0.28 * n.y;
    let c = to_linear(base) * (g.fog.z * sky + g.fog.w * sun);
    let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
    return mix(c, to_linear(g.mist.rgb), f * f * g.mist.w);
}

/// How light the block at `world` (on a face turned `n`, `d` blocks away)
/// is: a little lighter or darker than its neighbours, and darker along
/// its edges up close, so that a wall reads as blocks, as in the game.
fn block_tint(world: vec3<f32>, n: vec3<f32>, d: f32) -> f32 {
    let bi = vec3<i32>(floor(world - n * 0.01)) + vec3<i32>(i32(g.eye.w), 0, i32(g.cam_k.w));
    var h = u32(bi.x) * 73856093u ^ u32(bi.y) * 19349663u ^ u32(bi.z) * 83492791u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    let jitter = f32((h >> 8u) & 255u) / 255.0 - 0.5;
    let near = 1.0 - smoothstep(10.0, 48.0, d);
    // Where on the face: the two coordinates along it.
    var q = fract(world.xz);
    if (abs(n.x) > 0.5) {
        q = fract(world.zy);
    } else if (abs(n.z) > 0.5) {
        q = fract(world.xy);
    }
    let e = min(min(q.x, 1.0 - q.x), min(q.y, 1.0 - q.y));
    let line = max(1.3 * d * g.cam_k.z, 0.012);
    let edge = 1.0 - smoothstep(line * 0.5, line * 1.5, e);
    return (1.0 + jitter * 0.09 * (0.4 + 0.6 * near)) * (1.0 - 0.16 * edge * near);
}

/// The sky in direction `d`: the horizon colour low, the zenith's high.
fn sky_color(d: vec3<f32>) -> vec3<f32> {
    let t = clamp(d.y * 1.8 + 0.06, 0.0, 1.0);
    return mix(to_linear(g.horizon.rgb), to_linear(g.sky.rgb), sqrt(t));
}
"#;

const MESH_WGSL: &str = r#"
struct Chunk {
    origin: vec4<f32>,
};
@group(1) @binding(0) var<uniform> chunk: Chunk;

struct FaceIn {
    @location(0) xz_dir_len: vec4<u32>,
    @location(1) y_h: vec2<i32>,
    @location(2) color: vec4<f32>,
    @location(3) mat: vec2<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) water: f32,
    @location(4) @interpolate(flat) mat: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, f: FaceIn) -> VertexOut {
    // Corners (0,0) (1,0) (1,1) (0,0) (1,1) (0,1): two triangles.
    let u = f32((0x16u >> vi) & 1u);
    let v = f32((0x34u >> vi) & 1u);
    let dir = f.xz_dir_len.z;
    let x = f32(f.xz_dir_len.x);
    let z = f32(f.xz_dir_len.y);
    let len = f32(f.xz_dir_len.w);
    let y = f32(f.y_h.x);
    let h = f32(f.y_h.y);
    var p: vec3<f32>;
    var n: vec3<f32>;
    var top = y + h;
    switch dir {
        case 0u: {
            p = vec3<f32>(x + v * len, y, z + u * h);
            n = vec3<f32>(0.0, 1.0, 0.0);
            top = y;
        }
        case 1u: {
            p = vec3<f32>(x + u * len, y, z + v * h);
            n = vec3<f32>(0.0, -1.0, 0.0);
            top = y + 1.0e9;
        }
        case 2u: {
            p = vec3<f32>(x, y + u * h, z + v * len);
            n = vec3<f32>(1.0, 0.0, 0.0);
        }
        case 3u: {
            p = vec3<f32>(x, y + v * h, z + u * len);
            n = vec3<f32>(-1.0, 0.0, 0.0);
        }
        case 4u: {
            p = vec3<f32>(x + u * len, y + v * h, z);
            n = vec3<f32>(0.0, 0.0, 1.0);
        }
        default: {
            p = vec3<f32>(x + v * len, y + u * h, z);
            n = vec3<f32>(0.0, 0.0, -1.0);
        }
    }
    // Water sits a little below the top of its block; its sides go down
    // as much, so that a fall meets the water under it.
    if (chunk.origin.w > 0.5 && (dir >= 2u || p.y >= top - 0.01)) {
        p.y = p.y - 0.12;
    }
    let world = p + chunk.origin.xyz;
    var out: VertexOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.color = f.color;
    out.normal = n;
    out.world = world;
    out.water = chunk.origin.w;
    out.mat = f.mat.x;
    return out;
}

/// A face in the game's look: its block's texture, lit and fogged as the
/// game does, the brush and the block looked at marked, under the clouds.
fn game_fragment(in: VertexOut, n: vec3<f32>, d: f32) -> vec4<f32> {
    let view = normalize(in.world - g.eye.xyz);
    let lod = tex_lod(d, abs(dot(n, view)));
    let uv = face_uv(in.world, n);
    if (in.water > 0.5) {
        let lava = in.color.r > in.color.b;
        let layer = u32(select(g.rock.z, g.rock.w, lava));
        var tex = texel(layer, uv, lod);
        if (layer == 0u) {
            tex = vec4<f32>(to_linear(in.color.rgb), in.color.a);
        }
        if (lava) {
            // Lava glows: its own light.
            return vec4<f32>(game_fog(tex.rgb, d), 1.0);
        }
        var c = game_fog(tex.rgb * pow(game_face(n), 2.2), d);
        let cl = game_clouds(g.eye.xyz, view, d);
        c = mix(c, cl.rgb, cl.a);
        return vec4<f32>(c, max(tex.a, 0.55));
    }
    let layer = face_layer(in.mat, n, in.world.y);
    // Without the game's textures, its colour, block by block.
    var base = to_linear(in.color.rgb) * block_tint(in.world, n, d);
    if (layer != 0u) {
        base = texel(layer, uv, lod).rgb;
    }
    var c = game_light(base, n, 1.0, d);
    c = game_marks(c, in.world, n, d);
    let cl = game_clouds(g.eye.xyz, view, d);
    return vec4<f32>(mix(c, cl.rgb, cl.a), in.color.a);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let d = distance(in.world, g.eye.xyz);
    if (g.look.y > 0.5) {
        return game_fragment(in, n, d);
    }
    if (in.water > 0.5 && in.color.r > in.color.b) {
        // Lava glows: its own light, not the sun's.
        var c = to_linear(in.color.rgb) * 1.1;
        let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
        c = mix(c, to_linear(g.mist.rgb), f * f * g.mist.w * 0.6);
        return vec4<f32>(c, 1.0);
    }
    if (in.water > 0.5 && in.color.b > in.color.r) {
        // Water: its own colour seen from above, the sky mirrored where it
        // is seen at a grazing angle, a glint of the sun.
        let v = normalize(g.eye.xyz - in.world);
        // Small waves on its surface: the sky and the sun break up in them.
        var nw = n;
        if (n.y > 0.5) {
            let p = in.world.xz;
            let rx = sin(p.x * 0.73 + p.y * 0.31) + 0.6 * sin(p.x * 1.91 - p.y * 1.27) + 0.35 * sin(p.x * 3.7 + p.y * 2.9);
            let rz = cos(p.y * 0.67 - p.x * 0.29) + 0.6 * cos(p.y * 2.03 + p.x * 1.11) + 0.35 * cos(p.y * 3.3 - p.x * 3.1);
            nw = normalize(vec3<f32>(rx * 0.045, 1.0, rz * 0.045));
        }
        let fres = pow(1.0 - max(dot(nw, v), 0.0), 3.0);
        let sky = sky_color(reflect(-v, nw));
        let h = normalize(v + g.sun.xyz);
        let glint = pow(max(dot(nw, h), 0.0), 900.0) * 1.1 * max(n.y, 0.0);
        var c = to_linear(in.color.rgb) * (g.fog.z + 0.3 * g.fog.w);
        c = mix(c, sky, 0.1 + 0.7 * fres) + vec3<f32>(glint);
        let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
        c = mix(c, to_linear(g.mist.rgb), f * f * g.mist.w);
        return vec4<f32>(c, mix(in.color.a, 1.0, 0.55 * fres));
    }
    let c = shade(in.color.rgb, n, d) * block_tint(in.world, n, d);
    return vec4<f32>(c, in.color.a);
}
"#;

const TRACE_WGSL: &str = r#"
struct Terrain {
    origin: vec4<f32>,
    size: vec4<i32>,
    stone: vec4<f32>,
    deepslate: vec4<f32>,
};
@group(1) @binding(0) var<uniform> terrain: Terrain;
// Packed columns, one page of 1024 × 1024 per layer: height above the
// bottom of the world (12 bits, 0: nothing), then the colour pair (12 bits).
@group(1) @binding(1) var cols: texture_2d_array<u32>;
// Highest column of every 2^k square of a page, mip k - 1.
@group(1) @binding(2) var maxes: texture_2d_array<u32>;
// Layer + 1 of every page of the grid (0: no page).
@group(1) @binding(3) var pages: texture_2d<u32>;
// Highest column of every page, and of every 2^k × 2^k pages at mip k.
@group(1) @binding(4) var pyramid: texture_2d<u32>;
// Row 0: top colours, row 1: soil colours, row 2: the tops' own colours
// (without what grows on them), of every pair.
@group(1) @binding(5) var palette: texture_2d<f32>;
// The blocks of every pair: its top's material, its soil's.
@group(1) @binding(6) var pair_mats: texture_2d<u32>;

const PAGE_SHIFT: i32 = 10;
const STEPS: i32 = 768;

struct TraceOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_trace(@builtin(vertex_index) vi: u32) -> TraceOut {
    // One triangle over the whole screen.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: TraceOut;
    out.ndc = uv * 2.0 - 1.0;
    out.clip = vec4<f32>(out.ndc, 0.0, 1.0);
    return out;
}

/// Cells of the grid along x and z at `level` (cells of 2^level columns).
fn cells_at(level: i32) -> vec2<i32> {
    let size = 1 << u32(level);
    return (terrain.size.xy + vec2<i32>(size - 1)) >> vec2<u32>(u32(level));
}

/// The highest column (above the bottom of the world) of cell `c` of `level`.
fn cell_max(level: i32, c: vec2<i32>) -> u32 {
    if (level >= PAGE_SHIFT) {
        return textureLoad(pyramid, c, level - PAGE_SHIFT).r;
    }
    let shift = vec2<u32>(u32(PAGE_SHIFT - level));
    let page = c >> shift;
    let layer = i32(textureLoad(pages, page, 0).r) - 1;
    if (layer < 0) {
        return 0u;
    }
    let local = c - (page << shift);
    if (level == 0) {
        return textureLoad(cols, local, layer, 0).r & 0xfffu;
    }
    return textureLoad(maxes, local, layer, level - 1).r;
}

/// The packed column `c` (0 outside the map).
fn column(c: vec2<i32>) -> u32 {
    if (any(c < vec2<i32>(0)) || any(c >= terrain.size.xy)) {
        return 0u;
    }
    let page = c >> vec2<u32>(10u);
    let layer = i32(textureLoad(pages, page, 0).r) - 1;
    if (layer < 0) {
        return 0u;
    }
    return textureLoad(cols, c & vec2<i32>(1023), layer, 0).r;
}

struct Hit {
    t: f32,
    normal: vec3<f32>,
    cell: vec2<i32>,
    level: i32,
    found: bool,
};

/// Walk the ray `o + t d` (grid coordinates) from `t_in` to `t_out` down
/// the max pyramid: skip every square whose highest column stays under the
/// ray, go down into the others, stop on a column, or on a square smaller
/// than a pixel (`pix` radians). `axis` is the side the ray came in by.
fn trace(o: vec3<f32>, d: vec3<f32>, t_in: f32, t_out: f32, axis_in: i32, pix: f32) -> Hit {
    var hit: Hit;
    hit.found = false;
    let bottom = terrain.origin.z;
    let top = terrain.size.z;
    let step = vec2<i32>(select(-1, 1, d.x > 0.0), select(-1, 1, d.z > 0.0));
    let inv = 1.0 / d.xz;
    var level = top;
    var t = t_in;
    var axis = axis_in;
    var cell = vec2<i32>(floor((o.xz + d.xz * t) / f32(1 << u32(level))));
    cell = clamp(cell, vec2<i32>(0), cells_at(level) - vec2<i32>(1));
    for (var i = 0; i < STEPS; i++) {
        if (any(cell < vec2<i32>(0)) || any(cell >= cells_at(level))) {
            break;
        }
        let size = f32(1 << u32(level));
        let corner = vec2<f32>(cell) * size;
        let bound = corner + select(vec2<f32>(0.0), vec2<f32>(size), step > vec2<i32>(0));
        let tb = (bound - o.xz) * inv;
        let t_exit = min(tb.x, tb.y);
        let m = cell_max(level, cell);
        let surface = bottom + f32(m);
        let y_in = o.y + d.y * t;
        let y_exit = o.y + d.y * t_exit;
        if (m == 0u || min(y_in, y_exit) >= surface) {
            // Over everything in this square: on to the next one.
            if (t_exit >= t_out) {
                break;
            }
            let old = cell;
            if (tb.x < tb.y) {
                cell.x += step.x;
                axis = 0;
            } else {
                cell.y += step.y;
                axis = 1;
            }
            t = max(t, t_exit);
            if (level < top && any((cell >> vec2<u32>(1u)) != (old >> vec2<u32>(1u)))) {
                level += 1;
                cell = cell >> vec2<u32>(1u);
            }
            continue;
        }
        if (level == 0 || size <= t * pix) {
            hit.found = true;
            hit.level = level;
            hit.cell = cell;
            if (y_in < surface) {
                // Into the side of the column.
                hit.t = t;
                if (axis == 0) {
                    hit.normal = vec3<f32>(-f32(step.x), 0.0, 0.0);
                } else if (axis == 1) {
                    hit.normal = vec3<f32>(0.0, 0.0, -f32(step.y));
                } else {
                    hit.normal = vec3<f32>(0.0, 1.0, 0.0);
                }
            } else {
                // Down onto its top.
                hit.t = (surface - o.y) / d.y;
                hit.normal = vec3<f32>(0.0, 1.0, 0.0);
            }
            return hit;
        }
        // Down into the quarter of the square the ray is in.
        level -= 1;
        let mid = corner + vec2<f32>(size * 0.5);
        let p = o.xz + d.xz * t;
        var child = cell * 2;
        if (p.x > mid.x || (p.x == mid.x && step.x > 0)) {
            child.x += 1;
        }
        if (p.y > mid.y || (p.y == mid.y && step.y > 0)) {
            child.y += 1;
        }
        cell = child;
    }
    return hit;
}

/// Height of the ground at a point of the grid (the bottom of the world
/// where there is none).
fn ground_height(q: vec2<f32>) -> f32 {
    return terrain.origin.z + f32(column(vec2<i32>(floor(q))) & 0xfffu);
}

/// The colour of a packed column at height `y`: its top, the soil of its
/// first blocks down, the rock under them.
fn surface_color(col: u32, y: f32, up: bool) -> vec3<f32> {
    let height = terrain.origin.z + f32(col & 0xfffu);
    let pair = i32((col >> 12u) & 0xfffu);
    if (up) {
        return textureLoad(palette, vec2<i32>(pair, 0), 0).rgb;
    }
    if (height - y <= 3.0 || terrain.stone.w > 1.5) {
        return textureLoad(palette, vec2<i32>(pair, 1), 0).rgb;
    }
    if (y < 0.0) {
        return terrain.deepslate.rgb;
    }
    return terrain.stone.rgb;
}

struct TraceFrag {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

/// Behind everything: the sky, the game's in the player's view, with its
/// clouds.
fn background(d: vec3<f32>) -> vec3<f32> {
    if (g.look.y > 0.5) {
        let cl = game_clouds(g.eye.xyz, d, 1.0e9);
        return mix(game_sky(d), cl.rgb, cl.a);
    }
    return sky_color(d);
}

/// Whether the block at row `y` of column `c` is there: the tracer's
/// columns fill up from the bottom of the world.
fn filled(c: vec2<i32>, y: f32) -> f32 {
    return select(0.0, 1.0, terrain.origin.z + f32(column(c) & 0xfffu) > y + 0.5);
}

/// The light at a corner of a face, as the game's smooth lighting has it:
/// 3 in the open, less for each block beside it, none between two.
fn corner_light(s1: f32, s2: f32, c: f32) -> f32 {
    if (s1 + s2 > 1.5) {
        return 0.0;
    }
    return 3.0 - (s1 + s2 + c);
}

/// How much light reaches point `p` of a face of column `cell` turned `n`
/// (`height`: the column's top): darker in the corners that blocks close.
fn column_light(cell: vec2<i32>, p: vec3<f32>, n: vec3<f32>, height: f32) -> f32 {
    var c00: f32;
    var c10: f32;
    var c01: f32;
    var c11: f32;
    var uv: vec2<f32>;
    if (n.y > 0.5) {
        // The blocks of the layer over the top, around.
        let y = height;
        let xm = filled(cell + vec2<i32>(-1, 0), y);
        let xp = filled(cell + vec2<i32>(1, 0), y);
        let zm = filled(cell + vec2<i32>(0, -1), y);
        let zp = filled(cell + vec2<i32>(0, 1), y);
        c00 = corner_light(xm, zm, filled(cell + vec2<i32>(-1, -1), y));
        c10 = corner_light(xp, zm, filled(cell + vec2<i32>(1, -1), y));
        c01 = corner_light(xm, zp, filled(cell + vec2<i32>(-1, 1), y));
        c11 = corner_light(xp, zp, filled(cell + vec2<i32>(1, 1), y));
        uv = fract(p.xz);
    } else {
        // Around the block of air the wall faces.
        let f = cell + vec2<i32>(i32(n.x), i32(n.z));
        let a = vec2<i32>(i32(abs(n.z)), i32(abs(n.x)));
        let y = floor(p.y);
        let am = filled(f - a, y);
        let ap = filled(f + a, y);
        let below = filled(f, y - 1.0);
        let above = filled(f, y + 1.0);
        c00 = corner_light(am, below, filled(f - a, y - 1.0));
        c10 = corner_light(ap, below, filled(f + a, y - 1.0));
        c01 = corner_light(am, above, filled(f - a, y + 1.0));
        c11 = corner_light(ap, above, filled(f + a, y + 1.0));
        uv = vec2<f32>(fract(dot(p.xz, vec2<f32>(a))), fract(p.y));
    }
    let light = mix(mix(c00, c10, uv.x), mix(c01, c11, uv.x), uv.y);
    return 0.55 + 0.15 * light;
}

/// A traced column's face in the game's look: its block's texture (the top
/// block's at the top, the soil's under it, then the rock), tinted by what
/// grows on its top, in the game's light, marked.
fn game_column(cell: vec2<i32>, p: vec3<f32>, world: vec3<f32>, n: vec3<f32>, t: f32, view_d: vec3<f32>) -> vec3<f32> {
    let col = column(cell);
    let pair = i32((col >> 12u) & 0xfffu);
    let height = terrain.origin.z + f32(col & 0xfffu);
    let up = n.y > 0.5;
    let m = textureLoad(pair_mats, vec2<i32>(pair, 0), 0);
    var mat = ROCK;
    if (up || height - p.y <= 1.0) {
        mat = m.x;
    } else if (height - p.y <= 3.0) {
        mat = m.y;
    }
    let layer = face_layer(mat, n, p.y);
    // Without the game's textures, its colour, block by block.
    var base = to_linear(surface_color(col, p.y, up)) * block_tint(world, n, t);
    if (layer != 0u) {
        base = texel(layer, face_uv(world, n), tex_lod(t, abs(dot(n, view_d)))).rgb;
        if (up) {
            let top = to_linear(textureLoad(palette, vec2<i32>(pair, 0), 0).rgb);
            let bare = to_linear(textureLoad(palette, vec2<i32>(pair, 2), 0).rgb);
            base *= clamp(top / max(bare, vec3<f32>(0.004)), vec3<f32>(0.25), vec3<f32>(4.0));
        }
    }
    // Under water the light fades a level a block, as in the game.
    var light = column_light(cell, p, n, height);
    let depth = f32(col >> 24u);
    if (depth > 0.0) {
        light *= water_light(depth + height - p.y);
    }
    let c = game_light(base, n, light, t);
    return game_marks(c, world, n, t);
}

/// The game's light `deep` blocks under water (in its gamma, at its default
/// brightness): the sky's 15 levels, one fewer a block.
fn water_light(deep: f32) -> f32 {
    let f = clamp(15.0 - deep, 0.0, 15.0) / 15.0;
    let g = f / (4.0 - 3.0 * f);
    let lifted = 1.0 - pow(1.0 - g, 4.0);
    return max(mix(g, lifted, 0.5), 0.05);
}

@fragment
fn fs_trace(in: TraceOut) -> TraceFrag {
    let view_d = normalize(g.cam_f.xyz + g.cam_s.xyz * (in.ndc.x * g.cam_k.x) + g.cam_u.xyz * (in.ndc.y * g.cam_k.y));
    // On a very large map a column of the grid is `stride` blocks wide: the
    // ray goes into the grid's own coordinates (t stays the distance).
    let stride = f32(max(terrain.size.w, 1));
    var d = vec3<f32>(view_d.x / stride, view_d.y, view_d.z / stride);
    // Never exactly along a plane of the grid: the walk divides by these.
    d = select(d, vec3<f32>(1.0e-7), abs(d) < vec3<f32>(1.0e-7)) ;
    let o = vec3<f32>((g.eye.x - terrain.origin.x) / stride, g.eye.y, (g.eye.z - terrain.origin.y) / stride);
    // The ray inside the box of the map.
    let lo = vec3<f32>(0.0, terrain.origin.z, 0.0);
    let hi = vec3<f32>(f32(terrain.size.x), terrain.origin.w, f32(terrain.size.y));
    let ta = (lo - o) / d;
    let tb = (hi - o) / d;
    let near = min(ta, tb);
    let far = max(ta, tb);
    let t0 = max(max(near.x, near.z), max(near.y, 0.0));
    let t1 = min(min(far.x, far.z), far.y);
    var out: TraceFrag;
    if (t1 <= t0 || terrain.origin.w <= terrain.origin.z) {
        out.color = vec4<f32>(background(view_d), 1.0);
        out.depth = 1.0;
        return out;
    }
    var axis = 2;
    if (t0 > 0.0) {
        if (near.x >= near.z && near.x >= near.y) {
            axis = 0;
        } else if (near.z >= near.y) {
            axis = 1;
        }
    }
    let hit = trace(o, d, t0, t1, axis, g.cam_k.z / stride);
    if (!hit.found) {
        out.color = vec4<f32>(background(view_d), 1.0);
        out.depth = 1.0;
        return out;
    }
    let p = o + d * hit.t;
    var base: vec3<f32>;
    var normal = hit.normal;
    if (hit.level == 0) {
        base = surface_color(column(hit.cell), p.y, hit.normal.y > 0.5);
    } else {
        // A square smaller than a pixel: the ground seen from afar, smooth.
        // Its colour is the mean of the tops of four of its columns (mixed
        // ground does not flicker), its light that of the hillside.
        let size = f32(1 << u32(hit.level));
        let corner = vec2<f32>(hit.cell) * size;
        var sum = vec3<f32>(0.0);
        var count = 0.0;
        for (var k = 0; k < 4; k++) {
            let q = corner + size * vec2<f32>(0.25 + 0.5 * f32(k & 1), 0.25 + 0.5 * f32(k >> 1u));
            let col = column(vec2<i32>(floor(q)));
            if (col != 0u) {
                sum += surface_color(col, p.y, true);
                count += 1.0;
            }
        }
        base = select(textureLoad(palette, vec2<i32>(0, 0), 0).rgb, sum / max(count, 1.0), count > 0.0);
        let mid = corner + vec2<f32>(size * 0.5);
        let hl = ground_height(mid - vec2<f32>(size, 0.0));
        let hr = ground_height(mid + vec2<f32>(size, 0.0));
        let hu = ground_height(mid - vec2<f32>(0.0, size));
        let hd = ground_height(mid + vec2<f32>(0.0, size));
        normal = normalize(vec3<f32>(-(hr - hl) / (2.0 * size * stride), 1.0, -(hd - hu) / (2.0 * size * stride)));
    }
    let world = vec3<f32>(p.x * stride + terrain.origin.x, p.y, p.z * stride + terrain.origin.y);
    if (g.look.y > 0.5) {
        var c: vec3<f32>;
        if (hit.level == 0 && stride == 1.0) {
            c = game_column(hit.cell, p, world, normal, hit.t, view_d);
        } else {
            c = game_light(to_linear(base), normal, 1.0, hit.t);
        }
        let cl = game_clouds(g.eye.xyz, view_d, hit.t);
        out.color = vec4<f32>(mix(c, cl.rgb, cl.a), 1.0);
    } else {
        var tint = 1.0;
        if (hit.level == 0 && stride == 1.0) {
            tint = block_tint(world, normal, hit.t);
        }
        out.color = vec4<f32>(shade(base, normal, hit.t) * tint, 1.0);
    }
    let clip = g.view_proj * vec4<f32>(world, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
"#;

struct GpuFaces {
    buffer: wgpu::Buffer,
    count: u32,
}

/// The GPU side of one chunk.
struct GpuChunk {
    solid: Option<GpuFaces>,
    water: Option<GpuFaces>,
}

impl GpuChunk {
    fn bytes(&self) -> u64 {
        [&self.solid, &self.water]
            .iter()
            .filter_map(|f| f.as_ref())
            .map(|f| f.count as u64 * size_of::<Face>() as u64)
            .sum()
    }
}

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    #[cfg_attr(not(test), allow(dead_code))]
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same image as plain bytes, for egui.
    display_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// The tracer's textures for a grid of pages, and what the CPU keeps to
/// update their max pyramids.
struct Tracer {
    /// Blocks per column of the grid (1, or more on very large maps).
    stride: i32,
    /// The page at the grid's north-west corner, and pages across and down.
    page0: (i32, i32),
    pages: (u32, u32),
    /// Layer + 1 of every page of the grid, row-major (0: no page).
    table: Vec<u32>,
    /// Side of the pyramid over pages (a power of two), and its levels.
    side: u32,
    /// Top level of the whole max pyramid (squares of 2^top columns).
    top: i32,
    /// Highest column of every tile, and of every page.
    tile_tops: HashMap<TilePos, u16>,
    page_tops: Vec<u16>,
    dirty_pages: HashSet<usize>,
    cols: wgpu::Texture,
    maxes: wgpu::Texture,
    pyramid: wgpu::Texture,
    group: wgpu::BindGroup,
    bytes: u64,
}

impl Tracer {
    /// The page holding a tile: its index in the grid, and its layer.
    fn page_of(&self, tile: TilePos) -> Option<(usize, u32)> {
        let px = tile.x.div_euclid(PAGE_TILES) - self.page0.0;
        let pz = tile.z.div_euclid(PAGE_TILES) - self.page0.1;
        if px < 0 || pz < 0 || px as u32 >= self.pages.0 || pz as u32 >= self.pages.1 {
            return None;
        }
        let k = pz as usize * self.pages.0 as usize + px as usize;
        let layer = self.table[k].checked_sub(1)?;
        Some((k, layer))
    }
}

/// Halve a square grid of `side` × `side` maxima.
fn halve(level: &[u16], side: usize) -> Vec<u16> {
    let half = (side / 2).max(1);
    (0..half * half)
        .map(|k| {
            let (i, j) = (2 * (k % half), 2 * (k / half));
            let at = |i: usize, j: usize| if i < side && j < side { level[j * side + i] } else { 0 };
            at(i, j).max(at(i + 1, j)).max(at(i, j + 1)).max(at(i + 1, j + 1))
        })
        .collect()
}

/// GPU resources of the 3D preview. Independent of egui, so it can render
/// headlessly in tests.
pub struct Renderer3d {
    device: wgpu::Device,
    queue: wgpu::Queue,
    opaque_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    trace_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    chunk_layout: wgpu::BindGroupLayout,
    trace_layout: wgpu::BindGroupLayout,
    /// One slot per drawn chunk (twice: solid and water).
    origins: wgpu::Buffer,
    origins_group: wgpu::BindGroup,
    slots: u64,
    terrain: wgpu::Buffer,
    palette: wgpu::Texture,
    /// The blocks of every colour pair (see [`Skin`]).
    pair_mats: wgpu::Texture,
    /// The game's look: its textures, the materials' layers, its clouds.
    look_layout: wgpu::BindGroupLayout,
    look_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    atlas: wgpu::TextureView,
    materials: wgpu::TextureView,
    clouds: wgpu::TextureView,
    /// Materials the atlas knows, and the layers of water and lava.
    material_count: u32,
    fluid_layers: [u16; 2],
    tracer: Option<Tracer>,
    target: Option<Target>,
    chunks: HashMap<TilePos, GpuChunk>,
}

impl Renderer3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif 3d meshes"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{GLOBALS_WGSL}{}{MESH_WGSL}", crate::textures::LOOK_WGSL))),
        });
        let trace_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif 3d tracer"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{GLOBALS_WGSL}{}{TRACE_WGSL}", crate::textures::LOOK_WGSL))),
        });
        let uniform = |binding: u32, dynamic: bool, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: NonZeroU64::new(size),
            },
            count: None,
        };
        let texture = |binding: u32, sample_type: wgpu::TextureSampleType, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d globals"),
            entries: &[uniform(0, false, size_of::<Globals>() as u64)],
        });
        let chunk_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d chunk"),
            entries: &[uniform(0, true, size_of::<ChunkUniform>() as u64)],
        });
        let uint = wgpu::TextureSampleType::Uint;
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d terrain"),
            entries: &[
                uniform(0, false, size_of::<TerrainUniform>() as u64),
                texture(1, uint, wgpu::TextureViewDimension::D2Array),
                texture(2, uint, wgpu::TextureViewDimension::D2Array),
                texture(3, uint, wgpu::TextureViewDimension::D2),
                texture(4, uint, wgpu::TextureViewDimension::D2),
                texture(5, wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2),
                texture(6, uint, wgpu::TextureViewDimension::D2),
            ],
        });
        let look_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d look"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::D2Array),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(2, uint, wgpu::TextureViewDimension::D2),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2),
            ],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let terrain = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d terrain"),
            size: size_of::<TerrainUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d palette"),
            size: wgpu::Extent3d {
                width: MAX_PAIRS as u32,
                height: 3,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pair_mats = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d pair blocks"),
            size: wgpu::Extent3d {
                width: MAX_PAIRS as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Pixels sharp up close, as in the game; their levels blended far off.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif 3d textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let (atlas, materials, count) = Self::make_atlas(device, queue, &crate::textures::Atlas::plain());
        let clouds = Self::make_clouds(device, queue, &crate::textures::clouds(None));
        let look_group = Self::make_look(device, &look_layout, &atlas, &sampler, &materials, &clouds);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif 3d meshes"),
            bind_group_layouts: &[Some(&layout), Some(&chunk_layout), Some(&look_layout)],
            immediate_size: 0,
        });
        let trace_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif 3d tracer"),
            bind_group_layouts: &[Some(&layout), Some(&trace_layout), Some(&look_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Uint8x4, 1 => Sint16x2, 2 => Unorm8x4, 3 => Uint16x2];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<Face>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        })];
        let depth = |write: bool| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let target = |blend: wgpu::BlendState| {
            [Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };
        let multisample = wgpu::MultisampleState {
            count: SAMPLES,
            ..Default::default()
        };
        let pipeline = |label: &str, blend: wgpu::BlendState, depth_write: bool, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: depth(depth_write),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &target(blend),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque_pipeline = pipeline("massif 3d opaque", wgpu::BlendState::REPLACE, true, Some(wgpu::Face::Back));
        let water_pipeline = pipeline("massif 3d water", wgpu::BlendState::ALPHA_BLENDING, false, None);
        let trace_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("massif 3d tracer"),
            layout: Some(&trace_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &trace_shader,
                entry_point: Some("vs_trace"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // First in the frame, it writes every pixel, the sky included.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &trace_shader,
                entry_point: Some("fs_trace"),
                compilation_options: Default::default(),
                targets: &target(wgpu::BlendState::REPLACE),
            }),
            multiview_mask: None,
            cache: None,
        });
        let (origins, origins_group) = Self::make_origins(device, &chunk_layout, 64);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            opaque_pipeline,
            water_pipeline,
            trace_pipeline,
            globals,
            bind_group,
            chunk_layout,
            trace_layout,
            origins,
            origins_group,
            slots: 64,
            terrain,
            palette,
            pair_mats,
            look_layout,
            look_group,
            sampler,
            atlas,
            materials,
            clouds,
            material_count: count,
            fluid_layers: [0, 0],
            tracer: None,
            target: None,
            chunks: HashMap::new(),
        }
    }

    /// The game's textures on the graphics card: the layers with their
    /// levels of detail, and the materials' layers (1,024 a row). Returns
    /// the views and the number of materials.
    fn make_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &crate::textures::Atlas) -> (wgpu::TextureView, wgpu::TextureView, u32) {
        use crate::textures::{MIPS, SIDE, mip_chain};
        let layers = atlas.layers.len().max(1) as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d block textures"),
            size: wgpu::Extent3d {
                width: SIDE as u32,
                height: SIDE as u32,
                depth_or_array_layers: layers,
            },
            mip_level_count: MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let chains: Vec<Vec<Vec<u8>>> = atlas.layers.iter().map(mip_chain).collect();
        for mip in 0..MIPS {
            let side = (SIDE >> mip) as u32;
            let bytes: Vec<u8> = chains.iter().flat_map(|c| c[mip as usize].iter().copied()).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: layers,
                },
            );
        }
        const ROW: usize = 1024;
        let count = atlas.materials.len();
        let (w, h) = (count.clamp(1, ROW), count.div_ceil(ROW).max(1));
        let mut table = vec![0u16; w * h * 4];
        for (k, m) in atlas.materials.iter().enumerate() {
            table[k * 4..k * 4 + 4].copy_from_slice(m);
        }
        let materials = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d materials"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            materials.as_image_copy(),
            bytemuck::cast_slice(&table),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w as u32 * 8),
                rows_per_image: Some(h as u32),
            },
            wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
        );
        let array = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        (array, materials.create_view(&wgpu::TextureViewDescriptor::default()), count as u32)
    }

    fn make_clouds(device: &wgpu::Device, queue: &wgpu::Queue, map: &[u8]) -> wgpu::TextureView {
        let side = crate::textures::CLOUDS as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d clouds"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            map,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn make_look(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, atlas: &wgpu::TextureView, sampler: &wgpu::Sampler, materials: &wgpu::TextureView, clouds: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d look"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(materials),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(clouds),
                },
            ],
        })
    }

    /// The game's textures for the player's view, and its clouds.
    pub fn set_look(&mut self, atlas: &crate::textures::Atlas, clouds: &[u8]) {
        let most = self.device.limits().max_texture_array_layers as usize;
        let (view, materials, count) = if atlas.layers.len() <= most {
            Self::make_atlas(&self.device, &self.queue, atlas)
        } else {
            Self::make_atlas(&self.device, &self.queue, &crate::textures::Atlas::plain())
        };
        self.atlas = view;
        self.materials = materials;
        self.material_count = count;
        self.fluid_layers = [atlas.water, atlas.lava];
        self.clouds = Self::make_clouds(&self.device, &self.queue, clouds);
        self.look_group = Self::make_look(&self.device, &self.look_layout, &self.atlas, &self.sampler, &self.materials, &self.clouds);
    }

    /// The materials the atlas knows, and the layers of water and lava.
    pub fn look_info(&self) -> (u32, [u16; 2]) {
        (self.material_count, self.fluid_layers)
    }

    fn make_origins(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, slots: u64) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d chunk origins"),
            size: slots * SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d chunk origins"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: NonZeroU64::new(size_of::<ChunkUniform>() as u64),
                }),
            }],
        });
        (buffer, group)
    }

    /// Make the tracer's textures for a grid of `pages` pages from `page0`,
    /// with a layer for each page `present` says holds tiles (row-major).
    /// Everything uploaded before is dropped.
    pub fn set_grid(&mut self, stride: i32, page0: (i32, i32), pages: (u32, u32), present: &[bool]) -> Result<(), String> {
        let count = present.iter().filter(|p| **p).count().max(1) as u32;
        let limit = self.device.limits().max_texture_array_layers;
        if count > limit {
            self.tracer = None;
            return Err(format!(
                "This map needs {count} pages of {PAGE} × {PAGE} blocks; this GPU holds {limit}"
            ));
        }
        let mut table = vec![0u32; present.len()];
        let mut next = 1;
        for (k, p) in present.iter().enumerate() {
            if *p {
                table[k] = next;
                next += 1;
            }
        }
        let side = pages.0.max(pages.1).max(1).next_power_of_two();
        let pyramid_levels = side.trailing_zeros() + 1;
        let make = |label: &str, w: u32, h: u32, layers: u32, mips: u32, format: wgpu::TextureFormat| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: layers,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let page = PAGE as u32;
        let cols = make("massif 3d columns", page, page, count, 1, wgpu::TextureFormat::R32Uint);
        let maxes = make("massif 3d maxes", page / 2, page / 2, count, PAGE_SHIFT, wgpu::TextureFormat::R16Uint);
        let table_tex = make("massif 3d pages", pages.0, pages.1, 1, 1, wgpu::TextureFormat::R32Uint);
        let pyramid = make("massif 3d pyramid", side, side, 1, pyramid_levels, wgpu::TextureFormat::R16Uint);
        self.queue.write_texture(
            table_tex.as_image_copy(),
            bytemuck::cast_slice(&table),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pages.0 * 4),
                rows_per_image: Some(pages.1),
            },
            wgpu::Extent3d {
                width: pages.0,
                height: pages.1,
                depth_or_array_layers: 1,
            },
        );
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let flat = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let views = [array(&cols), array(&maxes), flat(&table_tex), flat(&pyramid), flat(&self.palette), flat(&self.pair_mats)];
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.terrain.as_entire_binding(),
        }];
        for (k, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: k as u32 + 1,
                resource: wgpu::BindingResource::TextureView(v),
            });
        }
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d terrain"),
            layout: &self.trace_layout,
            entries: &entries,
        });
        let per_layer = (page * page * 4) as u64 + (page * page / 4 * 2) as u64 * 4 / 3;
        self.tracer = Some(Tracer {
            stride: stride.max(1),
            page0,
            pages,
            table,
            side,
            top: (PAGE_SHIFT + side.trailing_zeros()) as i32,
            tile_tops: HashMap::new(),
            page_tops: vec![0; present.len()],
            dirty_pages: HashSet::new(),
            cols,
            maxes,
            pyramid,
            group,
            bytes: per_layer * count as u64,
        });
        Ok(())
    }

    /// Put the packed columns of a tile (row-major, `TILE_SIZE`²) in the
    /// tracer's textures, with the max pyramid over them.
    pub fn upload_columns(&mut self, tile: TilePos, packed: &[u32]) {
        let Some(tr) = &mut self.tracer else { return };
        let Some((page, layer)) = tr.page_of(tile) else { return };
        let n = TILE_SIZE as u32;
        let (lx, lz) = (tile.x.rem_euclid(PAGE_TILES) as u32 * n, tile.z.rem_euclid(PAGE_TILES) as u32 * n);
        let write = |queue: &wgpu::Queue, texture: &wgpu::Texture, mip: u32, x: u32, z: u32, side: u32, bytes: &[u8], texel: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d { x, y: z, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * texel),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        };
        write(&self.queue, &tr.cols, 0, lx, lz, n, bytemuck::cast_slice(packed), 4);
        // Mips 0 to 6 of the maxes: squares of 2 to 128 columns.
        let mut level: Vec<u16> = packed.iter().map(|&c| (c & HEIGHT_MASK) as u16).collect();
        let mut side = n as usize;
        for mip in 0..7u32 {
            level = halve(&level, side);
            side /= 2;
            write(&self.queue, &tr.maxes, mip, lx >> (mip + 1), lz >> (mip + 1), side as u32, bytemuck::cast_slice(&level), 2);
        }
        tr.tile_tops.insert(tile, level[0]);
        tr.dirty_pages.insert(page);
    }

    /// After tiles were uploaded: the maxes of their pages and the pyramid
    /// over pages.
    pub fn flush(&mut self) {
        let Some(tr) = &mut self.tracer else { return };
        if tr.dirty_pages.is_empty() {
            return;
        }
        let mut changed = false;
        let dirty: Vec<usize> = tr.dirty_pages.drain().collect();
        for page in dirty {
            let Some(layer) = tr.table[page].checked_sub(1) else { continue };
            let (px, pz) = ((page % tr.pages.0 as usize) as i32, (page / tr.pages.0 as usize) as i32);
            let k = PAGE_TILES as usize;
            let mut level: Vec<u16> = (0..k * k)
                .map(|c| {
                    let t = TilePos::new((tr.page0.0 + px) * PAGE_TILES + (c % k) as i32, (tr.page0.1 + pz) * PAGE_TILES + (c / k) as i32);
                    tr.tile_tops.get(&t).copied().unwrap_or(0)
                })
                .collect();
            let mut side = k;
            // Mips 7 to 9: squares of 2, 4 and 8 tiles.
            for mip in 7..PAGE_SHIFT {
                level = halve(&level, side);
                side /= 2;
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tr.maxes,
                        mip_level: mip,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::cast_slice(&level),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side as u32 * 2),
                        rows_per_image: Some(side as u32),
                    },
                    wgpu::Extent3d {
                        width: side as u32,
                        height: side as u32,
                        depth_or_array_layers: 1,
                    },
                );
            }
            if tr.page_tops[page] != level[0] {
                tr.page_tops[page] = level[0];
                changed = true;
            }
        }
        if !changed {
            return;
        }
        // The pyramid over pages, every level.
        let side = tr.side as usize;
        let mut level = vec![0u16; side * side];
        for (k, &top) in tr.page_tops.iter().enumerate() {
            let (px, pz) = (k % tr.pages.0 as usize, k / tr.pages.0 as usize);
            level[pz * side + px] = top;
        }
        let mut s = side;
        let mut mip = 0;
        loop {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tr.pyramid,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&level),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(s as u32 * 2),
                    rows_per_image: Some(s as u32),
                },
                wgpu::Extent3d {
                    width: s as u32,
                    height: s as u32,
                    depth_or_array_layers: 1,
                },
            );
            if s == 1 {
                break;
            }
            level = halve(&level, s);
            s /= 2;
            mip += 1;
        }
    }

    /// The colours and blocks the packed columns point at.
    pub fn set_palette(&mut self, pairs: &[Skin]) {
        let n = pairs.len().min(MAX_PAIRS);
        if n == 0 {
            return;
        }
        let mut rows = vec![0u8; n * 4 * 3];
        let mut mats = vec![0u16; n * 4];
        for (k, skin) in pairs.iter().take(n).enumerate() {
            for (row, c) in [skin.top, skin.soil, skin.bare].iter().enumerate() {
                let i = (row * n + k) * 4;
                rows[i..i + 3].copy_from_slice(c);
                rows[i + 3] = 255;
            }
            mats[k * 4] = skin.top_mat;
            mats[k * 4 + 1] = skin.soil_mat;
        }
        let extent = |h: u32| wgpu::Extent3d {
            width: n as u32,
            height: h,
            depth_or_array_layers: 1,
        };
        self.queue.write_texture(
            self.palette.as_image_copy(),
            &rows,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n as u32 * 4),
                rows_per_image: Some(3),
            },
            extent(3),
        );
        self.queue.write_texture(
            self.pair_mats.as_image_copy(),
            bytemuck::cast_slice(&mats),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n as u32 * 8),
                rows_per_image: Some(1),
            },
            extent(1),
        );
    }

    fn upload_faces(&self, faces: &[Face], label: &str) -> Option<GpuFaces> {
        if faces.is_empty() {
            return None;
        }
        Some(GpuFaces {
            buffer: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(faces),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            count: faces.len() as u32,
        })
    }

    /// Put (or replace) the meshed faces of a tile on the GPU.
    pub fn upload(&mut self, tile: TilePos, faces: &ChunkFaces) {
        if faces.solid.is_empty() && faces.water.is_empty() {
            self.chunks.remove(&tile);
            return;
        }
        let chunk = GpuChunk {
            solid: self.upload_faces(&faces.solid, "massif 3d chunk"),
            water: self.upload_faces(&faces.water, "massif 3d water"),
        };
        self.chunks.insert(tile, chunk);
    }

    /// Forget a tile that left the map.
    pub fn remove(&mut self, tile: TilePos) {
        self.chunks.remove(&tile);
        self.upload_columns(tile, &[0; (TILE_SIZE * TILE_SIZE) as usize]);
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.tracer = None;
    }

    /// Bytes of GPU memory the terrain takes.
    pub fn bytes(&self) -> u64 {
        self.tracer.as_ref().map_or(0, |t| t.bytes) + self.chunks.values().map(GpuChunk::bytes).sum::<u64>()
    }

    /// Make sure the render target has `size` pixels. Returns whether it was
    /// (re)created.
    pub fn ensure_target(&mut self, size: [u32; 2]) -> bool {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        // The image egui shows is read as plain bytes: egui works with
        // gamma-encoded colours, and an sRGB view would decode them once
        // more, darkening everything.
        let texture = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: if format == COLOR_FORMAT { &[wgpu::TextureFormat::Rgba8Unorm] } else { &[] },
            })
        };
        let msaa = texture("massif 3d msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolve = texture(
            "massif 3d image",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture("massif 3d depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(Target {
            size,
            msaa: view(&msaa),
            resolve_view: view(&resolve),
            display_view: resolve.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..Default::default()
            }),
            resolve,
            depth: view(&depth),
        });
        true
    }

    pub fn image_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.display_view)
    }

    /// Draw the traced columns and the meshed chunks of `tiles` into the
    /// render target. Positions are made relative to `origin` (block
    /// coordinates), the origin of `globals`; `bottom` is the bottom of the
    /// world.
    /// Draw the frame into the top left `region` of the target (all of it,
    /// or less while the camera moves).
    pub fn render(&mut self, globals: &Globals, origin: (i32, i32), tiles: &[TilePos], bottom: i32, substrate: Substrate, region: [u32; 2]) {
        if self.target.is_none() {
            return;
        }
        let drawn: Vec<(TilePos, f32, f32)> = tiles
            .iter()
            .filter(|t| self.chunks.contains_key(t))
            .map(|t| (*t, (t.x * TILE_SIZE - origin.0) as f32, (t.z * TILE_SIZE - origin.1) as f32))
            .collect();
        let needed = (drawn.len() as u64 * 2).max(1);
        if needed > self.slots {
            self.slots = needed.next_power_of_two();
            let (buffer, group) = Self::make_origins(&self.device, &self.chunk_layout, self.slots);
            self.origins = buffer;
            self.origins_group = group;
        }
        // Slot 2k is chunk k's solid faces, 2k + 1 its water.
        let mut data = vec![0u8; (drawn.len() * 2).max(1) * SLOT as usize];
        for (k, &(_, ox, oz)) in drawn.iter().enumerate() {
            for (slot, water) in [(2 * k, 0.0), (2 * k + 1, 1.0)] {
                let u = ChunkUniform {
                    origin: [ox, 0.0, oz, water],
                };
                data[slot * SLOT as usize..][..size_of::<ChunkUniform>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
        }
        self.queue.write_buffer(&self.origins, 0, &data);
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
        if let Some(tr) = &self.tracer {
            let top = tr.page_tops.iter().copied().max().unwrap_or(0);
            let unit = |c: [u8; 3]| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0];
            let u = TerrainUniform {
                origin: [
                    (tr.page0.0 * PAGE * tr.stride - origin.0) as f32,
                    (tr.page0.1 * PAGE * tr.stride - origin.1) as f32,
                    bottom as f32,
                    (bottom + top as i32) as f32,
                ],
                size: [tr.pages.0 as i32 * PAGE, tr.pages.1 as i32 * PAGE, tr.top, tr.stride],
                // w: 2 when walls are their column's colour all the way down.
                stone: {
                    let mut c = unit(substrate.stone);
                    c[3] = if substrate.whole { 2.0 } else { 1.0 };
                    c
                },
                deepslate: unit(substrate.deepslate),
            };
            self.queue.write_buffer(&self.terrain, 0, bytemuck::bytes_of(&u));
        }
        let target = self.target.as_ref().expect("checked above");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif 3d") });
        {
            let sky = globals.horizon;
            let lin = |c: f32| (c as f64).powf(2.2);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("massif 3d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: lin(sky[0]),
                            g: lin(sky[1]),
                            b: lin(sky[2]),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let [w, h] = target.size;
            pass.set_viewport(0.0, 0.0, region[0].clamp(1, w) as f32, region[1].clamp(1, h) as f32, 0.0, 1.0);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(2, &self.look_group, &[]);
            // The traced ground first, then the meshed chunks, then the
            // water over everything.
            if let Some(tr) = &self.tracer {
                pass.set_pipeline(&self.trace_pipeline);
                pass.set_bind_group(1, &tr.group, &[]);
                pass.draw(0..3, 0..1);
            }
            for water in [false, true] {
                pass.set_pipeline(if water { &self.water_pipeline } else { &self.opaque_pipeline });
                for (k, (t, ..)) in drawn.iter().enumerate() {
                    let chunk = &self.chunks[t];
                    let Some(faces) = (if water { &chunk.water } else { &chunk.solid }) else {
                        continue;
                    };
                    let slot = (2 * k + water as usize) as u64 * SLOT;
                    pass.set_bind_group(1, &self.origins_group, &[slot as u32]);
                    pass.set_vertex_buffer(0, faces.buffer.slice(..));
                    pass.draw(0..6, 0..faces.count);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Wait until the graphics card has drawn what it was given.
    #[cfg(test)]
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Copy the rendered image back to the CPU (RGBA, row-major). Blocks.
    #[cfg(test)]
    pub fn read_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.target.as_ref()?;
        let [w, h] = target.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d readback"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.resolve,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = buffer.get_mapped_range(..).ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]);
        }
        Some((w, h, out))
    }
}

/// Colours of the sky behind the terrain and of the haze over it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub zenith: [u8; 3],
    pub horizon: [u8; 3],
    /// The haze far away, and how much of it at most (0..1).
    pub mist: [u8; 3],
    pub haze: f32,
    /// Under water: everything fades into its colour within so many blocks.
    pub murk: Option<f32>,
}

impl Sky {
    /// The sky over the terrain, and the haze, light in both themes so
    /// that the far ground never sinks into the dark.
    pub fn of(pal: &Palette) -> Sky {
        // Daylight in both themes, as a terrain is looked at outdoors; a
        // deeper blue in the dark one, so that it does not glare.
        if pal.dark {
            Sky {
                zenith: [52, 98, 170],
                horizon: [160, 186, 216],
                mist: [176, 198, 224],
                haze: 0.4,
                murk: None,
            }
        } else {
            Sky {
                zenith: [86, 140, 214],
                horizon: [200, 218, 238],
                mist: [206, 222, 240],
                haze: 0.46,
                murk: None,
            }
        }
    }

    /// The game's sky at noon: its blue overhead, its fog's paler blue at the
    /// horizon, into which far land fades.
    pub const GAME: Sky = Sky {
        zenith: [120, 167, 255],
        horizon: [192, 216, 255],
        mist: [192, 216, 255],
        haze: 1.0,
        murk: None,
    };

    /// One flat colour: tests tell the sky from the ground by it.
    #[cfg(test)]
    pub fn flat(c: [u8; 3]) -> Sky {
        Sky {
            zenith: c,
            horizon: c,
            mist: c,
            haze: 0.0,
            murk: None,
        }
    }

    /// Seen from under water (`c`, its colour), as in the game: the light
    /// from above, everything blue and gone within a few blocks.
    pub fn under_water(c: [u8; 3]) -> Sky {
        let light = |k: f32| [(c[0] as f32 * k).min(255.0) as u8, (c[1] as f32 * k).min(255.0) as u8, (c[2] as f32 * k).min(255.0) as u8];
        Sky {
            zenith: light(1.6),
            horizon: c,
            mist: c,
            haze: 1.0,
            murk: Some(20.0),
        }
    }
}

/// An orbit camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Point looked at, block coordinates.
    pub target: [f64; 3],
    /// Rotation around the vertical axis; 0 looks north.
    pub yaw: f32,
    /// Elevation angle above the horizon.
    pub pitch: f32,
    pub distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: [0.0, 64.0, 0.0],
            yaw: 0.65,
            pitch: 0.62,
            distance: 520.0,
        }
    }
}

impl Camera {
    pub fn eye(&self) -> [f64; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let d = self.distance as f64;
        [
            self.target[0] + d * (cp * sy) as f64,
            self.target[1] + d * sp as f64,
            self.target[2] + d * (cp * cy) as f64,
        ]
    }

    /// The vertical field of view: wider at a player's eyes (the orbit
    /// camera never comes as close to what it looks at).
    pub fn fov(&self) -> f32 {
        if self.distance < 2.0 { FOV_WALK } else { FOV_Y }
    }

    /// Uniforms for a frame whose positions are relative to `origin`.
    /// `extent` is the side of the map, `height` that of the image in
    /// pixels.
    pub fn globals(&self, origin: (i32, i32), aspect: f32, extent: f32, sky: Sky, height: u32) -> Globals {
        let local = |p: [f64; 3]| Vec3::new((p[0] - origin.0 as f64) as f32, p[1] as f32, (p[2] - origin.1 as f64) as f32);
        let eye = local(self.eye());
        let target = local(self.target);
        let extent = extent.max(256.0);
        let far = self.distance + extent * 1.6 + 600.0;
        // At a player's eyes, near: their body keeps 0.3 block from walls.
        let near = if self.distance < 2.0 { 0.1 } else { (self.distance * 0.01).clamp(0.2, 8.0) };
        let aspect = aspect.max(0.01);
        let proj = perspective(self.fov(), aspect, near, far);
        let view = look_at(eye, target, Vec3::Y);
        let sun = Vec3::new(-0.42, 0.82, -0.38).normalize();
        let f = (target - eye).normalize();
        let s = f.cross(Vec3::Y).normalize();
        let u = s.cross(f);
        let tan = (self.fov() * 0.5).tan();
        let unit = |c: [u8; 3], w: f32| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, w];
        Globals {
            view_proj: (proj * view).to_cols_array_2d(),
            // The frame's origin rides along, for what is drawn block by
            // block to stay put as the origin moves.
            eye: [eye.x, eye.y, eye.z, origin.0 as f32],
            sun: [sun.x, sun.y, sun.z, 0.0],
            sky: unit(sky.zenith, 1.0),
            horizon: unit(sky.horizon, 1.0),
            // The haze only veils what is far beyond the map's middle;
            // under water, everything near.
            fog: match sky.murk {
                Some(reach) => [0.0, reach, 0.62, 0.5],
                None => [self.distance + extent * 0.4, self.distance + extent * 1.6, 0.62, 0.5],
            },
            mist: unit(sky.mist, sky.haze),
            cam_f: [f.x, f.y, f.z, 0.0],
            cam_s: [s.x, s.y, s.z, 0.0],
            cam_u: [u.x, u.y, u.z, 0.0],
            cam_k: [tan * aspect, tan, 2.0 * tan / height.max(1) as f32, origin.1 as f32],
            // The game's look, the brush and the block looked at: set for
            // the player's view.
            look: [0.0; 4],
            brush: [0.0; 4],
            brush_n: [0.0; 4],
            brush_t: [0.0; 4],
            pick: [0.0; 4],
            rock: [0.0; 4],
        }
    }
}

/// Right-handed perspective projection with a 0..1 depth range (wgpu).
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    let r = far / (near - far);
    Mat4::from_cols(
        Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, f, 0.0, 0.0),
        Vec4::new(0.0, 0.0, r, -1.0),
        Vec4::new(0.0, 0.0, r * near, 0.0),
    )
}

/// Right-handed view matrix looking from `eye` at `center`.
fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Mat4 {
    let f = (center - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols(
        Vec4::new(s.x, u.x, -f.x, 0.0),
        Vec4::new(s.y, u.y, -f.y, 0.0),
        Vec4::new(s.z, u.z, -f.z, 0.0),
        Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Whether the box `lo`..`hi` may be on screen through `m`.
fn in_view(m: &Mat4, lo: Vec3, hi: Vec3) -> bool {
    let corners: Vec<Vec4> = (0..8)
        .map(|k| {
            let p = Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            );
            *m * p.extend(1.0)
        })
        .collect();
    let all = |f: &dyn Fn(&Vec4) -> bool| corners.iter().all(f);
    !(all(&|c| c.x < -c.w) || all(&|c| c.x > c.w) || all(&|c| c.y < -c.w) || all(&|c| c.y > c.w) || all(&|c| c.z < 0.0) || all(&|c| c.z > c.w))
}

// ----------------------------------------------------------------- widget --

/// Height of the ground (or the water on it) at a point, as composed.
fn ground_at(doc: &Document, x: f64, z: f64) -> Option<f64> {
    let p = crate::tools::probe(doc, x.floor() as i32, z.floor() as i32)?;
    Some(p.height.max(if p.fluid != NO_FLUID { p.fluid as f32 } else { f32::MIN }) as f64)
}

/// The map's rectangle in blocks: x0, z0, x1, z1.
fn map_rect(doc: &Document) -> Option<(f64, f64, f64, f64)> {
    let (lo, hi) = doc.dimension().bounds()?;
    let t = TILE_SIZE as f64;
    Some((lo.x as f64 * t, lo.z as f64 * t, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t))
}

/// A camera showing the whole map.
fn framing(doc: &Document) -> Camera {
    let Some((x0, z0, x1, z1)) = map_rect(doc) else { return Camera::default() };
    let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
    let extent = (x1 - x0).max(z1 - z0);
    // The ground over the map, coarsely: the camera looks at its middle
    // height, from far enough to stand over its highest peaks.
    let mut heights = Vec::new();
    for j in 0..9 {
        for i in 0..9 {
            let (x, z) = (x0 + (x1 - x0) * (i as f64 + 0.5) / 9.0, z0 + (z1 - z0) * (j as f64 + 0.5) / 9.0);
            heights.extend(ground_at(doc, x, z));
        }
    }
    let middle = ground_at(doc, cx, cz).unwrap_or(64.0);
    let highest = heights.iter().copied().fold(middle, f64::max);
    let mean = if heights.is_empty() { middle } else { heights.iter().sum::<f64>() / heights.len() as f64 };
    let target_y = (mean + highest) / 2.0;
    Camera {
        target: [cx, target_y, cz],
        distance: (extent * 0.85).max((highest - target_y) * 3.2).max(160.0) as f32,
        ..Camera::default()
    }
}

/// The pages of the tracer's textures a map needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GridShape {
    /// Blocks per column of the grid: 1, or 2, 4, 8, 16 on maps too large
    /// for the GPU at every block. With a stride, the grid's tiles are
    /// virtual: each holds `stride` × `stride` tiles of the map.
    stride: i32,
    page0: (i32, i32),
    pages: (u32, u32),
    /// Pages holding tiles, row-major.
    present: Vec<bool>,
}

/// The tile of the grid a tile of the map is drawn in, at `stride`.
fn grid_tile(t: TilePos, stride: i32) -> TilePos {
    TilePos::new(t.x.div_euclid(stride), t.z.div_euclid(stride))
}

impl GridShape {
    /// The pages a map needs, at the finest stride that fits the budget.
    fn of(dim: &Dimension) -> Option<Self> {
        let mut shape = Self::at(dim, 1)?;
        while shape.present.iter().filter(|p| **p).count() > PAGE_BUDGET && shape.stride < 16 {
            shape = Self::at(dim, shape.stride * 2)?;
        }
        Some(shape)
    }

    fn at(dim: &Dimension, stride: i32) -> Option<Self> {
        let (lo, hi) = dim.bounds()?;
        let (lo, hi) = (grid_tile(lo, stride), grid_tile(hi, stride));
        let page0 = (lo.x.div_euclid(PAGE_TILES), lo.z.div_euclid(PAGE_TILES));
        let page1 = (hi.x.div_euclid(PAGE_TILES), hi.z.div_euclid(PAGE_TILES));
        let pages = ((page1.0 - page0.0 + 1) as u32, (page1.1 - page0.1 + 1) as u32);
        let mut present = vec![false; (pages.0 * pages.1) as usize];
        for t in dim.ground().positions() {
            let t = grid_tile(t, stride);
            let (px, pz) = (t.x.div_euclid(PAGE_TILES) - page0.0, t.z.div_euclid(PAGE_TILES) - page0.1);
            present[pz as usize * pages.0 as usize + px as usize] = true;
        }
        Some(Self { stride, page0, pages, present })
    }
}

/// A tile of the grid on a very large map: `stride` × `stride` tiles of the
/// map, a column every `stride` blocks, their water drawn as a surface (the
/// meshes are left out at this scale).
fn build_coarse(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, style: Style) -> TileBuild {
    let n = TILE_SIZE as usize;
    let per = n / stride as usize;
    let ground = dim.ground();
    let mut heights = vec![0u16; n * n];
    let mut colors = vec![0u16; n * n];
    let mut pairs: Vec<Skin> = Vec::new();
    let water_pair = if style.lava { Skin::flat([207, 92, 20], [160, 60, 10]) } else { Skin::flat([52, 96, 200], [40, 70, 160]) };
    let mut water_at: Option<u16> = None;
    for rz in 0..stride {
        for rx in 0..stride {
            let real = TilePos::new(cell.x * stride + rx, cell.z * stride + rz);
            let Some(view) = ground.view(real) else { continue };
            let b = build_tile(dim, reg, sea, seed, real, style);
            let base = pairs.len() as u16;
            pairs.extend(&b.pairs);
            for j in 0..per {
                for i in 0..per {
                    let (lx, lz) = (i * stride as usize + stride as usize / 2, j * stride as usize + stride as usize / 2);
                    let c = lz * n + lx;
                    let k = (rz as usize * per + j) * n + rx as usize * per + i;
                    let top = view.top.get(c);
                    let fluid = view.fluid.get(c);
                    let (mut h, mut col) = (b.heights[c], base + b.colors[c]);
                    if h == 0 {
                        // Overhangs and caves: their ground, at this scale.
                        h = (massif_core::compose::block_top(top) - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16;
                    }
                    if style.water && fluid != NO_FLUID && fluid as f32 > top + 0.3 {
                        h = (fluid as i32 - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16;
                        col = *water_at.get_or_insert_with(|| {
                            pairs.push(water_pair);
                            (pairs.len() - 1) as u16
                        });
                    }
                    heights[k] = h;
                    colors[k] = col;
                }
            }
        }
    }
    TileBuild {
        heights,
        colors,
        depths: Vec::new(),
        pairs,
        faces: ChunkFaces::default(),
    }
}

/// What the view knows of a tile it built.
struct TileInfo {
    faces: usize,
    stale: bool,
}

/// Tiles built away from the frames, on threads of their own, from a cold
/// copy of the map: what a whole map takes to build (a first look, another
/// colouring) no longer holds the frames up. What it brings comes back
/// through `results`, and the frames put it on the graphics card a little
/// at a time.
struct Builder {
    /// What it builds for: the document's generation, the style and the
    /// grid's stride.
    key: (u64, Style, i32),
    results: std::sync::mpsc::Receiver<(TilePos, TileBuild)>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The tiles it builds, and how many it brought.
    given: HashSet<TilePos>,
    delivered: usize,
    /// Tiles changed since it copied the map: what it brings of them is out
    /// of date, and they are built again here.
    edited: HashSet<TilePos>,
}

impl Drop for Builder {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The threads that build tiles away from the frames: all the cores but
/// two, which stay for the interface's own work.
fn builders() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("massif 3d tiles {i}"))
            .build()
            .expect("threads for the 3D tiles")
    })
}

/// A tile of the view's grid: one of the map's (`stride` 1), or the
/// columns of `stride` × `stride` of them, a column every `stride` blocks.
fn build_cell(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, style: Style) -> TileBuild {
    if stride > 1 {
        build_coarse(dim, reg, sea, seed, cell, stride, style)
    } else {
        build_tile(dim, reg, sea, seed, cell, style)
    }
}

/// The tiles of the view's grid that the map covers, with what they were
/// listed for: the generation, the number of the map's tiles, the stride.
type Cells = ((u64, usize, i32), std::sync::Arc<[TilePos]>);

/// The 3D preview editor (shared by every 3D area).
pub struct View3d {
    pub mode: MapMode,
    pub water: bool,
    pub camera: Camera,
    renderer: Option<Renderer3d>,
    texture: Option<egui::TextureId>,
    tiles: HashMap<TilePos, TileInfo>,
    /// Tiles being built away from the frames.
    builder: Option<Builder>,
    /// The tiles of the grid the map covers, and what they were listed for.
    cells: Option<Cells>,
    /// The pages the GPU holds.
    grid: Option<GridShape>,
    pairs: Pairs,
    style: Option<Style>,
    generation: Option<u64>,
    needs_render: bool,
    last_globals: Option<[f32; GLOBAL_FLOATS]>,
    /// The part of the target the last frame was drawn in.
    last_region: [u32; 2],
    /// Tiles still to build, and the time the last batch took.
    pending: usize,
    build_ms: f32,
    error: Option<String>,
    /// A sculpting stroke in progress, and when its last dab was.
    sculpt: Option<(SculptStroke, Instant)>,
    /// Where the cursor points in the world.
    hover: Option<massif_core::voxel::Hit>,
    /// The surface there, smoothed: where the brush lies, which way.
    frame: Option<massif_core::voxel::Frame>,
    /// Asked from the header: look where the map view is, frame the map.
    goto_map: bool,
    frame_map: bool,
    /// Walking in the view (3D Sculpt's player mode).
    walk: Option<Walk>,
    /// The turn (yaw, pitch, radians a second) and the move (blocks a
    /// second) a drag let go while moving leaves the view, dying away; and
    /// when the drag last moved.
    spin: [f32; 2],
    drift: [f64; 2],
    dragged_at: f64,
    /// The speed of flying with the keys, eased toward what they ask.
    fly: [f64; 3],
    /// Where the view glides to look (a double-click, the map view's place).
    look_goal: Option<[f64; 3]>,
    /// Until when the camera counts as moving, and the camera last drawn.
    moving_until: f64,
    last_camera: Option<[f64; 6]>,
    /// While the camera moves, the image is drawn smaller and stretched:
    /// sharp again the moment it stops.
    pub reduce_while_moving: bool,
    /// The game's blocks, for its textures in the player's view (set by the
    /// app as they are read).
    pub catalogue: Option<std::sync::Arc<massif_assets::Catalogue>>,
    /// What the atlas on the graphics card was made from (the catalogue,
    /// the registry's materials), and the document and catalogue last
    /// checked against it.
    look_key: Option<(usize, u64)>,
    look_seen: Option<(u64, u64, usize)>,
}

/// What the drawn image's side is, while the camera moves, of the view's
/// pixels: on a high-density screen, and on another.
const MOVING_SCALE_DENSE: f32 = 0.6;
const MOVING_SCALE: f32 = 0.8;
/// How long the camera counts as moving after it last moved, in seconds.
const STILL_AFTER: f64 = 0.15;
/// How quickly a drag's leftover turn and move die away (seconds for e).
const COAST: f32 = 0.09;

/// Walking in the view, as in Minecraft.
struct Walk {
    player: crate::player::Player,
    /// The view holds the mouse: moving it looks around, its buttons
    /// sculpt at the crosshair.
    captured: bool,
    /// Buttons still down from the click that took the mouse: they do not
    /// sculpt.
    swallow: bool,
    /// Where egui last saw the pointer while the view held it (where the
    /// system cannot lock it, it is brought back to the middle).
    anchor: Option<egui::Pos2>,
    /// The point a drag with Ctrl turns the view about (or that moving it
    /// keeps pace with).
    pivot: Option<[f64; 3]>,
    /// The pass the view was last drawn in, and the frame the body last
    /// moved in (a frame drawn twice moves it once).
    pass: u64,
    frame: u64,
}

/// The ray through screen point `p` of a view `rect` seen by `camera`:
/// origin and direction in block coordinates.
fn camera_ray(camera: &Camera, rect: Rect, p: egui::Pos2) -> ([f64; 3], [f64; 3]) {
    let eye = camera.eye();
    let t = camera.target;
    let f = Vec3::new((t[0] - eye[0]) as f32, (t[1] - eye[1]) as f32, (t[2] - eye[2]) as f32).normalize();
    let s = f.cross(Vec3::Y).normalize();
    let u = s.cross(f);
    let half = (camera.fov() * 0.5).tan();
    let aspect = rect.width() / rect.height().max(1.0);
    let nx = (p.x - rect.center().x) / (rect.width() * 0.5);
    let ny = -(p.y - rect.center().y) / (rect.height() * 0.5);
    let d = f + s * (nx * half * aspect) + u * (ny * half);
    (eye, [d.x as f64, d.y as f64, d.z as f64])
}

/// How far ahead of the eyes the camera looks while walking (the orbit
/// camera never comes this close).
const LOOK: f32 = 1.0;
/// Radians a point of mouse motion turns the eyes (Minecraft's default).
const MOUSE_LOOK: f32 = 0.0026;
/// Radians a point of dragging turns them.
const DRAG_LOOK: f32 = 0.005;

/// Hold the mouse (locked where the system can, else kept in the window),
/// or let it go.
fn hold_mouse(ctx: &egui::Context, hold: bool) {
    use egui::viewport::CursorGrab;
    let grab = |g| ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(g));
    if !hold {
        grab(CursorGrab::None);
    } else if cfg!(target_os = "macos") {
        grab(CursorGrab::Locked);
    } else if cfg!(target_os = "windows") {
        grab(CursorGrab::Confined);
    } else {
        // X11 only confines, Wayland locks too.
        grab(CursorGrab::Confined);
        grab(CursorGrab::Locked);
    }
}

impl Default for View3d {
    fn default() -> Self {
        Self {
            mode: MapMode::Terrain,
            water: true,
            camera: Camera::default(),
            renderer: None,
            texture: None,
            tiles: HashMap::new(),
            builder: None,
            cells: None,
            grid: None,
            pairs: Pairs::default(),
            style: None,
            generation: None,
            needs_render: true,
            last_globals: None,
            last_region: [0, 0],
            pending: 0,
            build_ms: 0.0,
            error: None,
            sculpt: None,
            hover: None,
            frame: None,
            goto_map: false,
            frame_map: false,
            walk: None,
            spin: [0.0; 2],
            drift: [0.0; 2],
            dragged_at: 0.0,
            fly: [0.0; 3],
            look_goal: None,
            moving_until: 0.0,
            last_camera: None,
            reduce_while_moving: !cfg!(test),
            catalogue: None,
            look_key: None,
            look_seen: None,
        }
    }
}

impl View3d {
    pub fn header(&mut self, ui: &mut Ui) {
        let narrow = ui.available_width() < 480.0;
        crate::map_view::mode_selector(ui, &mut self.mode, narrow, "3d mode");
        ui.separator();
        ui.toggle_value(&mut self.water, theme::glyph(Icon::Waves).to_string())
            .on_hover_text("Show water");
        if ui
            .small_button(theme::glyph(Icon::Crosshair).to_string())
            .on_hover_text("Look at the middle of the map view")
            .clicked()
        {
            self.goto_map = true;
        }
        if ui
            .small_button(theme::glyph(Icon::Rotate3d).to_string())
            .on_hover_text("Show the whole map")
            .clicked()
        {
            self.frame_map = true;
        }
    }

    /// Terrain changed: rebuild the tiles showing it, and their neighbours
    /// (whose walls and slopes depend on it).
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        for t in tiles {
            // Walls look at the neighbours; objects reach over the corners.
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let cell = grid_tile(t.offset(dx, dz), stride);
                if let Some(c) = self.tiles.get_mut(&cell) {
                    c.stale = true;
                }
                // What the builder brings of it is out of date.
                if let Some(b) = &mut self.builder
                    && b.given.contains(&cell)
                {
                    b.edited.insert(cell);
                }
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        for c in self.tiles.values_mut() {
            c.stale = true;
        }
        self.builder = None;
    }

    /// Where the cursor points, with a sculpting tool.
    #[cfg(test)]
    pub fn hover(&self) -> Option<massif_core::voxel::Hit> {
        self.hover
    }

    /// The last rendered image, read back from the GPU.
    #[cfg(test)]
    pub fn read_back(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.renderer.as_ref()?.read_image()
    }

    /// Tiles still to build.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Tiles built, and what the graphics card holds of the view.
    pub fn tiles_built(&self) -> usize {
        self.tiles.len()
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.renderer.as_ref().map_or(0, |r| r.bytes())
    }

    /// The player walking in the view, if any.
    #[cfg(test)]
    pub fn player(&self) -> Option<crate::player::Player> {
        self.walk.as_ref().map(|w| w.player)
    }

    /// Put the player at `feet`, flying or not.
    #[cfg(test)]
    pub fn place_player(&mut self, feet: [f64; 3], flying: bool) {
        if let Some(walk) = &mut self.walk {
            walk.player = crate::player::Player::new(feet);
            walk.player.flying = flying;
        }
    }

    /// The view held the mouse and was not drawn in the last frame (another
    /// workspace, its area closed): let the mouse go.
    pub fn release_if_hidden(&mut self, ctx: &egui::Context) {
        if let Some(walk) = &mut self.walk
            && walk.captured
            && walk.pass + 1 < ctx.cumulative_pass_nr()
        {
            walk.captured = false;
            hold_mouse(ctx, false);
        }
    }

    /// Come down into the map where the middle of the view looks, as a
    /// player: on the ground there, before a wall, under a roof.
    fn start_walking(&mut self, doc: &Document, rect: Rect) {
        let (o, d) = self.ray(rect, rect.center());
        let dim = doc.dimension();
        let mut blocks = crate::player::MapBlocks::new(dim);
        let feet = match massif_core::voxel::raycast(dim, o, d, 100_000.0) {
            Some(hit) => {
                let (n, p) = (hit.normal, hit.point);
                let y = if n.1 < 0 { p[1] - crate::player::HEIGHT - 0.05 } else { p[1] + 0.02 };
                Some([p[0] + n.0 as f64 * 0.35, y, p[2] + n.2 as f64 * 0.35])
            }
            None => {
                let (x, z) = (self.camera.target[0], self.camera.target[2]);
                blocks.top(x.floor() as i32, z.floor() as i32).map(|y| [x, y as f64, z])
            }
        };
        let mut player = crate::player::Player::new(feet.unwrap_or(self.camera.target));
        // Over nothing, flying.
        player.flying = feet.is_none();
        // Looking ahead, a little down.
        self.camera.pitch = 0.15;
        self.walk = Some(Walk {
            player,
            captured: false,
            swallow: false,
            anchor: None,
            pivot: None,
            pass: 0,
            frame: u64::MAX,
        });
    }

    /// Back to the orbit camera: from the player's eyes, around what they
    /// looked at.
    fn stop_walking(&mut self, ctx: &egui::Context, doc: &Document) {
        let Some(walk) = self.walk.take() else { return };
        if walk.captured {
            hold_mouse(ctx, false);
        }
        let eye = walk.player.eye();
        let (sy, cy) = self.camera.yaw.sin_cos();
        let (sp, cp) = self.camera.pitch.sin_cos();
        let ahead = [-(cp * sy) as f64, -sp as f64, -(cp * cy) as f64];
        let to = massif_core::voxel::raycast(doc.dimension(), eye, ahead, 256.0).map_or(32.0, |h| h.distance).clamp(8.0, 256.0);
        self.camera.pitch = self.camera.pitch.clamp(-1.2, 1.55);
        self.camera.distance = to as f32;
        self.camera.target = [eye[0] + ahead[0] * to, eye[1] + ahead[1] * to, eye[2] + ahead[2] * to];
    }

    /// A frame of walking: the mouse taken and given back, looking around,
    /// the keys, the body's move, the camera at its eyes. The cursor free,
    /// the view works as Axiom's editor: the left button turns it, with
    /// Ctrl (⌘) about the point under the cursor, the right button with
    /// Ctrl moves it, the middle one with Ctrl sizes the brush, the wheel
    /// sets the speed of flight; a click lends the view the mouse, as in
    /// the game.
    fn walk_ui(&mut self, ui: &Ui, response: &egui::Response, doc: &Document, tools: &mut ToolState, rect: Rect) {
        let Some(walk) = &mut self.walk else { return };
        let ctx = ui.ctx().clone();
        walk.pass = ctx.cumulative_pass_nr();
        let (escape, focused, motion, moved, pointer, dt, turn, mods, scroll) = ui.input(|i| {
            let key = |k: egui::Key| i.key_down(k) as i32 as f32;
            (
                i.key_pressed(egui::Key::Escape),
                i.viewport().focused.unwrap_or(true),
                i.pointer.motion(),
                i.pointer.delta(),
                i.pointer.latest_pos(),
                i.stable_dt.min(0.1) as f64,
                // The arrows turn the eyes: left, right, up, down.
                [key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft), key(egui::Key::ArrowDown) - key(egui::Key::ArrowUp)],
                i.modifiers,
                i.smooth_scroll_delta.y + i.smooth_scroll_delta.x,
            )
        });
        let ctrl = mods.ctrl || mods.command;
        let free = !walk.captured;
        let camera = self.camera;
        // Where a drag with Ctrl began: the point it turns about, and how
        // far it is (moving the view goes as fast as the land there).
        let pressed_at = |button| response.drag_started_by(button).then(|| response.interact_pointer_pos()).flatten();
        if free
            && ctrl
            && let Some(p) = pressed_at(PointerButton::Primary).or_else(|| pressed_at(PointerButton::Secondary))
        {
            let (o, d) = camera_ray(&camera, rect, p);
            walk.pivot = Some(match massif_core::voxel::raycast(doc.dimension(), o, d, 4096.0) {
                Some(hit) => hit.point,
                None => [o[0] + d[0] * 32.0, o[1] + d[1] * 32.0, o[2] + d[2] * 32.0],
            });
        }
        let basis = |yaw: f32, pitch: f32| {
            let (sy, cy) = (yaw as f64).sin_cos();
            let (sp, cp) = (pitch as f64).sin_cos();
            let f = [-cp * sy, -sp, -cp * cy];
            let s = [cy, 0.0, -sy];
            let u = [s[1] * f[2] - s[2] * f[1], s[2] * f[0] - s[0] * f[2], s[0] * f[1] - s[1] * f[0]];
            [f, s, u]
        };
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        // The eyes moved by hand: flying there.
        let put_eyes = |walk: &mut Walk, eye: [f64; 3]| {
            let height = walk.player.eye()[1] - walk.player.feet[1];
            walk.player.feet = [eye[0], eye[1] - height, eye[2]];
            walk.player.velocity = [0.0; 3];
            walk.player.flying = true;
        };
        if free && response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            let (yaw, pitch) = (self.camera.yaw - d.x * DRAG_LOOK, (self.camera.pitch + d.y * DRAG_LOOK).clamp(-1.55, 1.55));
            if ctrl && let Some(pivot) = walk.pivot {
                // Axiom's arcball: the eyes go round the point, still
                // looking at it.
                let eye = walk.player.eye();
                let v = [eye[0] - pivot[0], eye[1] - pivot[1], eye[2] - pivot[2]];
                let (old, new) = (basis(self.camera.yaw, self.camera.pitch), basis(yaw, pitch));
                let k = [dot(v, old[0]), dot(v, old[1]), dot(v, old[2])];
                let to = std::array::from_fn(|i| pivot[i] + k[0] * new[0][i] + k[1] * new[1][i] + k[2] * new[2][i]);
                put_eyes(walk, to);
            }
            self.camera.yaw = yaw;
            self.camera.pitch = pitch;
        }
        if free && ctrl && response.dragged_by(PointerButton::Secondary) {
            // Moved across the view, as fast as the land under the cursor.
            let d = response.drag_delta();
            let eye = walk.player.eye();
            let far = walk.pivot.map_or(32.0, |p| ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt()).max(4.0);
            let k = far * 2.0 * (camera.fov() as f64 * 0.5).tan() / rect.height().max(1.0) as f64;
            let [_, s, u] = basis(self.camera.yaw, self.camera.pitch);
            let to = std::array::from_fn(|i| eye[i] - s[i] * d.x as f64 * k + u[i] * d.y as f64 * k);
            put_eyes(walk, to);
        }
        if free && ctrl && response.dragged_by(PointerButton::Middle) {
            // The brush's size, sideways.
            let most = massif_core::voxel::MAX_SCULPT_RADIUS;
            let r = &mut tools.brush.radius;
            *r = (r.min(most) * (response.drag_delta().x * 0.01).exp()).clamp(1.0, most);
        }
        if free && !ctrl && response.clicked() {
            // The click that takes the mouse sculpts nothing.
            walk.captured = true;
            walk.swallow = true;
            walk.anchor = pointer;
            hold_mouse(&ctx, true);
        } else if walk.captured && (escape || !focused) {
            walk.captured = false;
            hold_mouse(&ctx, false);
        }
        if free && !ctrl && response.clicked_by(PointerButton::Middle) {
            // Pick block: the terrain under the cursor becomes the one
            // painted.
            if let Some(p) = response.interact_pointer_pos() {
                let (o, d) = camera_ray(&camera, rect, p);
                if let Some(hit) = massif_core::voxel::raycast(doc.dimension(), o, d, 4096.0)
                    && let Some(probe) = crate::tools::probe(doc, hit.block.0, hit.block.2)
                {
                    tools.terrain = probe.terrain;
                    tools.pattern.on = false;
                }
            }
        }
        // The wheel: the speed of flight, as Axiom's slider.
        if response.hovered() && !ctrl && scroll != 0.0 {
            let (lo, hi) = crate::tools::SculptSettings::FLY_SPEEDS;
            let s = &mut tools.sculpt.fly_speed;
            *s = (*s * (scroll * 0.003).exp()).clamp(lo, hi);
        }
        let typing = ctx.egui_wants_keyboard_input();
        if (walk.captured || response.hovered()) && !typing && turn != [0.0, 0.0] {
            self.camera.yaw -= turn[0] * 2.4 * dt as f32;
            self.camera.pitch = (self.camera.pitch + turn[1] * 1.6 * dt as f32).clamp(-1.55, 1.55);
        }
        // Held, the mouse turns the eyes as it moves: its own motion where
        // the system gives it, else where the pointer goes.
        let motion = motion.unwrap_or(if walk.captured { moved } else { egui::Vec2::ZERO });
        if walk.captured {
            self.camera.yaw -= motion.x * MOUSE_LOOK;
            self.camera.pitch = (self.camera.pitch + motion.y * MOUSE_LOOK).clamp(-1.55, 1.55);
            // No widget keeps the keys (a focused button takes Space).
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            // Where the system cannot lock the pointer, it goes back to the
            // middle of the view.
            if let (Some(p), Some(a)) = (pointer, walk.anchor)
                && p.distance(a) > 2.0
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(rect.center()));
                walk.anchor = Some(rect.center());
            }
        }
        use crate::player::Keys;
        let keys = if walk.captured || (response.hovered() && !typing) {
            ui.input(|i| {
                let held = |k: egui::Key| i.key_down(k) as i32 as f64;
                // Pressed now, not repeated by the system.
                let first = |key: egui::Key| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key));
                // ⌘ and Alt keep their shortcuts.
                if i.modifiers.mac_cmd || i.modifiers.alt {
                    Keys::default()
                } else {
                    // The game's keys: Shift sneaks (flying: goes down), Ctrl
                    // sprints; Q goes down too. (The arrows turn the eyes.)
                    Keys {
                        forward: held(egui::Key::W) - held(egui::Key::S),
                        right: held(egui::Key::D) - held(egui::Key::A),
                        forward_pressed: first(egui::Key::W),
                        jump: i.key_down(egui::Key::Space),
                        jump_pressed: first(egui::Key::Space),
                        sneak: i.modifiers.shift || i.key_down(egui::Key::Q),
                        run: i.modifiers.ctrl,
                    }
                }
            })
        } else {
            Keys::default()
        };
        let frame = ctx.cumulative_frame_nr();
        if walk.frame != frame {
            walk.frame = frame;
            walk.player.fly_speed = tools.sculpt.fly_speed as f64;
            let mut blocks = crate::player::MapBlocks::new(doc.dimension());
            walk.player.step(&mut blocks, keys, self.camera.yaw, dt);
        }
        // The camera at the eyes, looking just ahead.
        let eye = walk.player.eye();
        let (sy, cy) = self.camera.yaw.sin_cos();
        let (sp, cp) = self.camera.pitch.sin_cos();
        self.camera.distance = LOOK;
        self.camera.target = [eye[0] - (LOOK * cp * sy) as f64, eye[1] - (LOOK * sp) as f64, eye[2] - (LOOK * cp * cy) as f64];
        if walk.player.moving() || keys != Keys::default() || turn != [0.0, 0.0] || (walk.captured && motion != egui::Vec2::ZERO) {
            ctx.request_repaint();
        }
    }

    /// Over the player's view: the game's crosshair while the view holds
    /// the mouse, and along the bottom, as in Axiom's editor, what the
    /// buttons and keys do, how the player moves and the speed of flight.
    fn player_bar(&self, ui: &mut Ui, rect: Rect, tools: &mut ToolState) {
        let Some(walk) = &self.walk else { return };
        let pal = Palette::of(ui);
        let painter = ui.painter_at(rect);
        let c = rect.center();
        if walk.captured {
            for (w, color) in [(3.5, Color32::from_black_alpha(120)), (1.5, Color32::WHITE)] {
                painter.line_segment([c - vec2(8.0, 0.0), c + vec2(8.0, 0.0)], egui::Stroke::new(w, color));
                painter.line_segment([c - vec2(0.0, 8.0), c + vec2(0.0, 8.0)], egui::Stroke::new(w, color));
            }
        }
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 26.0), rect.right_bottom());
        painter.rect_filled(bar, 0, pal.panel.gamma_multiply(0.94));
        painter.hline(bar.x_range(), bar.top(), egui::Stroke::new(1.0, pal.border));
        // What the buttons do with the tool chosen.
        let (tool, inverse) = match tools.sculpt.tool {
            Tool::Smooth3d => ("Smooth", None),
            Tool::PaintTerrain => ("Paint", Some("Paint by height")),
            _ => ("Add", Some("Dig")),
        };
        let mut hints: Vec<(&str, &str)> = Vec::new();
        if walk.captured {
            hints.push(("RMB", tool));
            if let Some(inv) = inverse {
                hints.push(("LMB", inv));
            }
            hints.extend([("W A S D", "Move"), ("Space", "Jump · twice: fly"), ("Shift", "Sneak · down"), ("Ctrl", "Sprint"), ("Esc", "Free the mouse")]);
        } else {
            hints.push(("LMB", "Look around"));
            hints.push(("RMB", tool));
            if let Some(inv) = inverse {
                hints.push(("Alt+RMB", inv));
            }
            hints.extend([
                ("MMB", "Pick block"),
                ("Ctrl+LMB", "Orbit"),
                ("Ctrl+RMB", "Pan"),
                ("Ctrl+MMB", "Size"),
                ("Click", "Take the mouse"),
                ("W A S D", "Move"),
            ]);
        }
        // How the player goes and the speed of flight, on the right.
        let p = &walk.player;
        let state = if p.flying {
            "Flying"
        } else if p.swimming {
            "Swimming"
        } else if p.sneaking {
            "Sneaking"
        } else {
            "Walking"
        };
        let right_w = 260.0;
        let font = FontId::proportional(11.0);
        let mut x = bar.left() + 10.0;
        let limit = bar.right() - right_w - 8.0;
        for (key, what) in hints {
            let k = painter.layout_no_wrap(key.to_owned(), FontId::proportional(10.0), pal.text);
            let t = painter.layout_no_wrap(what.to_owned(), font.clone(), pal.weak);
            let w = k.size().x + 10.0 + 5.0 + t.size().x + 14.0;
            if x + w > limit {
                break;
            }
            let chip = Rect::from_min_size(pos2(x, bar.center().y - 8.5), vec2(k.size().x + 10.0, 17.0));
            painter.rect(chip, theme::TILE_RADIUS, pal.header, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            painter.galley(chip.center() - k.size() / 2.0, k, pal.text);
            painter.galley(pos2(chip.right() + 5.0, bar.center().y - t.size().y / 2.0), t, pal.weak);
            x += w;
        }
        let right = Rect::from_min_max(pos2(bar.right() - right_w, bar.top() + 1.0), bar.right_bottom());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(right.shrink2(vec2(8.0, 3.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let mut percent = (tools.sculpt.fly_speed * 100.0).round();
        let (lo, hi) = crate::tools::SculptSettings::FLY_SPEEDS;
        let field = child.add(crate::widgets::Field::new(&mut percent, (lo * 100.0).round()..=(hi * 100.0).round()).text("Flight speed").suffix("%").width(150.0));
        if field.changed() {
            tools.sculpt.fly_speed = percent / 100.0;
        }
        child.add_space(8.0);
        child.label(egui::RichText::new(state).size(11.0).color(pal.text));
    }

    /// Build the missing and stale tiles of the map, nearest the camera
    /// first, for about `budget`. Returns whether some remain.
    /// The tiles of the view's grid that the map covers: listed again only
    /// when the map gains or loses tiles.
    fn cells(&mut self, dim: &Dimension, generation: u64, stride: i32) -> std::sync::Arc<[TilePos]> {
        let ground = dim.ground();
        let key = (generation, ground.len(), stride);
        if let Some((k, cells)) = &self.cells
            && *k == key
        {
            return cells.clone();
        }
        let cells: std::sync::Arc<[TilePos]> = if stride > 1 {
            let set: HashSet<TilePos> = ground.positions().map(|t| grid_tile(t, stride)).collect();
            set.into_iter().collect()
        } else {
            ground.positions().collect()
        };
        self.cells = Some((key, cells.clone()));
        cells
    }

    /// Build the tiles missing or out of date, the nearest first: many at
    /// once away from the frames (see [`Builder`]), a few here within
    /// `budget`. Whether to draw again at once.
    fn build_tiles(&mut self, doc: &mut Document, budget: Duration, renderer: &mut Renderer3d, ctx: &egui::Context) -> bool {
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
        };
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        // A builder for another map, colouring or grid stops.
        if self.builder.as_ref().is_some_and(|b| b.key != (doc.generation, style, stride)) {
            self.builder = None;
        }
        if stride == 1 {
            // Tiles gone from the map.
            let ground = doc.dimension().ground();
            let gone: Vec<TilePos> = self.tiles.keys().copied().filter(|t| !ground.contains(*t)).collect();
            for t in gone {
                renderer.remove(t);
                self.tiles.remove(&t);
                self.needs_render = true;
            }
        }

        // What the builder brought, a frame's share of it.
        let (mut more, mut finished, mut uploaded) = (false, false, 0);
        if let Some(b) = &mut self.builder {
            let share = if budget.is_zero() { UPLOAD_BUDGET_MOVING } else { UPLOAD_BUDGET };
            let started = Instant::now();
            loop {
                if uploaded > 0 && started.elapsed() > share {
                    more = true;
                    break;
                }
                match b.results.try_recv() {
                    Ok((t, built)) => {
                        b.delivered += 1;
                        let edited = b.edited.contains(&t);
                        // Built again here since it changed: newer than this.
                        if edited && self.tiles.get(&t).is_some_and(|c| !c.stale) {
                            continue;
                        }
                        let packed = self.pairs.pack(&built);
                        renderer.upload_columns(t, &packed);
                        if stride == 1 {
                            renderer.upload(t, &built.faces);
                        }
                        self.tiles.insert(
                            t,
                            TileInfo {
                                faces: built.faces.solid.len() + built.faces.water.len(),
                                stale: edited,
                            },
                        );
                        uploaded += 1;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
            if uploaded > 0 {
                self.build_ms = started.elapsed().as_secs_f32() * 1000.0;
            }
        }
        if finished {
            self.builder = None;
        }

        // What is left to build: missing or out of date, but for what the
        // builder brings.
        let (tx, tz) = (self.camera.target[0], self.camera.target[2]);
        let span = (TILE_SIZE * stride) as f64;
        let cells = self.cells(doc.dimension(), doc.generation, stride);
        let mut todo: Vec<(f64, TilePos)> = cells
            .iter()
            .filter(|t| self.tiles.get(t).is_none_or(|c| c.stale))
            .filter(|t| self.builder.as_ref().is_none_or(|b| !b.given.contains(t) || b.edited.contains(t)))
            .map(|&t| (((t.x as f64 + 0.5) * span - tx).hypot((t.z as f64 + 0.5) * span - tz), t))
            .collect();
        todo.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Many: away from the frames.
        if todo.len() > BUILD_AWAY && self.builder.is_none() {
            let order: Vec<TilePos> = todo.drain(..).map(|(_, t)| t).collect();
            self.start_builder(doc, order, style, stride, ctx);
        }
        // A few: here, now (while a view moves, nothing: a tile takes
        // longer than a frame should).
        let mut done = 0;
        if !todo.is_empty() && !budget.is_zero() {
            let started = Instant::now();
            // One tile a thread at a time: a frame goes past its budget by
            // one tile's time at most.
            let batch = rayon::current_num_threads().clamp(2, 16);
            let dim = doc.dimension();
            let reg = &doc.world.registry;
            let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
            for part in todo.chunks(batch) {
                if done > 0 && started.elapsed() > budget {
                    break;
                }
                let built: Vec<(TilePos, TileBuild)> = part.par_iter().map(|&(_, t)| (t, build_cell(dim, reg, sea, seed, t, stride, style))).collect();
                for (t, b) in built {
                    let packed = self.pairs.pack(&b);
                    renderer.upload_columns(t, &packed);
                    if stride == 1 {
                        renderer.upload(t, &b.faces);
                    }
                    self.tiles.insert(
                        t,
                        TileInfo {
                            faces: b.faces.solid.len() + b.faces.water.len(),
                            stale: false,
                        },
                    );
                    done += 1;
                }
            }
            if done > 0 {
                self.build_ms = started.elapsed().as_secs_f32() * 1000.0;
            }
        }
        if uploaded + done > 0 {
            renderer.flush();
            self.needs_render = true;
        }
        if self.pairs.dirty {
            renderer.set_palette(self.pairs.list());
            self.pairs.dirty = false;
        }
        let away = self.builder.as_ref().map_or(0, |b| b.given.len().saturating_sub(b.delivered));
        self.pending = todo.len() - done + away;
        // At once for the work left here, or what the builder brought that
        // did not fit in this frame; the builder wakes the frames itself as
        // it brings more.
        more || todo.len() > done
    }

    /// Build `order`'s tiles away from the frames, from a cold copy of the
    /// map (see [`Builder`]).
    fn start_builder(&mut self, doc: &mut Document, order: Vec<TilePos>, style: Style, stride: i32, ctx: &egui::Context) {
        let dim = doc.world.dims[doc.dim].snapshot();
        let reg = doc.world.registry.clone();
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancel.clone();
        let ctx = ctx.clone();
        let given: HashSet<TilePos> = order.iter().copied().collect();
        let spawned = std::thread::Builder::new().name("massif 3d builder".into()).spawn(move || {
            let pool = builders();
            pool.install(|| {
                // The nearest first, as many at a time as there are threads
                // (twice: they do not all take as long).
                for part in order.chunks(2 * pool.current_num_threads()) {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let built: Vec<(TilePos, TileBuild)> = part.par_iter().map(|&t| (t, build_cell(&dim, &reg, sea, seed, t, stride, style))).collect();
                    for b in built {
                        if tx.send(b).is_err() {
                            return;
                        }
                    }
                    ctx.request_repaint();
                }
            });
        });
        if spawned.is_ok() {
            self.builder = Some(Builder {
                key: (doc.generation, style, stride),
                results: rx,
                cancel,
                given,
                delivered: 0,
                edited: HashSet::new(),
            });
        }
    }

    /// The game's textures for the blocks of the map, made again when the
    /// game's blocks arrive or the registry gains blocks or objects.
    fn update_look(&mut self, doc: &Document, renderer: &mut Renderer3d) {
        let cat = self.catalogue.as_ref().map_or(0, |c| std::sync::Arc::as_ptr(c) as usize);
        let seen = (doc.generation, doc.revision, cat);
        if self.look_seen == Some(seen) {
            return;
        }
        self.look_seen = Some(seen);
        let key = (cat, crate::textures::materials_key(&doc.world.registry));
        if self.look_key == Some(key) {
            return;
        }
        let most = renderer.device.limits().max_texture_array_layers as usize;
        let atlas = match &self.catalogue {
            Some(c) => crate::textures::atlas(&doc.world.registry, c, most),
            None => crate::textures::Atlas::plain(),
        };
        renderer.set_look(&atlas, &crate::textures::clouds(self.catalogue.as_deref()));
        // Objects added or gone: their materials are numbered anew.
        if self.look_key.is_some_and(|k| k.1 != key.1) {
            self.invalidate_all();
        }
        self.look_key = Some(key);
        self.needs_render = true;
    }

    /// The ray through screen point `p` of the view `rect`: origin and
    /// direction in block coordinates.
    fn ray(&self, rect: Rect, p: egui::Pos2) -> ([f64; 3], [f64; 3]) {
        camera_ray(&self.camera, rect, p)
    }

    /// Show the view; `sculpt`: the 3D Sculpt editor's, where the buttons
    /// sculpt with its tool.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, tools: &mut ToolState, map_center: Option<[f64; 2]>, sculpt: bool) {
        let pal = Palette::of(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let Some(render_state) = frame.wgpu_render_state() else {
            painter.rect_filled(rect, 0, pal.canvas);
            painter.text(rect.center(), Align2::CENTER_CENTER, "The 3D view needs the wgpu renderer", FontId::proportional(13.0), pal.weak);
            return;
        };
        let mut renderer = self
            .renderer
            .take()
            .unwrap_or_else(|| Renderer3d::new(&render_state.device, &render_state.queue));
        self.update_look(doc, &mut renderer);

        // A new world: show all of it, forget the old tiles.
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            if let Some(walk) = self.walk.take() {
                if walk.captured {
                    hold_mouse(ui.ctx(), false);
                }
                tools.sculpt.player = false;
            }
            self.camera = framing(doc);
            self.tiles.clear();
            self.grid = None;
            self.pairs = Pairs::default();
            renderer.clear();
            self.needs_render = true;
        }
        if std::mem::take(&mut self.frame_map) {
            // The whole map is seen from above: no more walking.
            if let Some(walk) = self.walk.take() {
                if walk.captured {
                    hold_mouse(ui.ctx(), false);
                }
                tools.sculpt.player = false;
            }
            self.camera = Camera {
                yaw: self.camera.yaw,
                ..framing(doc)
            };
        }
        if std::mem::take(&mut self.goto_map)
            && let Some(c) = map_center
        {
            let y = ground_at(doc, c[0], c[1]).unwrap_or(self.camera.target[1]);
            match &mut self.walk {
                // The player goes there.
                Some(walk) => walk.player = crate::player::Player::new([c[0], y, c[1]]),
                None => self.look_goal = Some([c[0], y, c[1]]),
            }
        }
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
        };
        if self.style != Some(style) {
            self.style = Some(style);
            self.invalidate_all();
        }
        let extent = map_rect(doc).map_or(512.0, |(x0, z0, x1, z1)| (x1 - x0).max(z1 - z0) as f32);

        // ---- walking ---------------------------------------------------------
        // 3D Sculpt's player mode: a click lends the view the mouse (Esc
        // takes it back), which looks around; the keys walk, jump, fly; the
        // buttons sculpt at the crosshair.
        let walking = sculpt && tools.sculpt.player;
        if walking != self.walk.is_some() {
            if walking {
                self.start_walking(doc, rect);
            } else {
                self.stop_walking(ui.ctx(), doc);
            }
        }
        self.walk_ui(ui, &response, doc, tools, rect);
        let captured = self.walk.as_ref().is_some_and(|w| w.captured);

        // ---- camera control ------------------------------------------------
        // With a tool of the Sculpt editor the buttons sculpt or paint (the
        // right one inverts); Shift + drag moves the view, Alt + drag (or
        // the middle button) turns it. Otherwise the left button turns, the
        // right one moves.
        let sculpting = sculpt && crate::tools::SculptSettings::TOOLS.contains(&tools.sculpt.tool);
        let (shift, alt, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command || i.modifiers.ctrl));
        let (orbit, pan) = if sculpting {
            (
                (response.dragged_by(PointerButton::Middle) && !shift) || (response.dragged_by(PointerButton::Primary) && alt && !shift),
                (response.dragged_by(PointerButton::Middle) && shift) || (response.dragged_by(PointerButton::Primary) && shift),
            )
        } else {
            (
                response.dragged_by(PointerButton::Primary),
                response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle),
            )
        };
        let (now, dt) = ui.input(|i| (i.time, i.stable_dt.clamp(1.0 / 240.0, 0.1)));
        // Reduce motion (Settings ▸ Accessibility): no coasting, no easing,
        // looking somewhere goes straight there.
        let still = crate::theme::reduce_motion(ui.ctx());
        if orbit && !walking {
            let d = response.drag_delta();
            let turn = [-d.x * 0.008, d.y * 0.006];
            self.camera.yaw += turn[0];
            self.camera.pitch = (self.camera.pitch + turn[1]).clamp(-1.2, 1.55);
            if d != egui::Vec2::ZERO {
                self.spin = [self.spin[0] * 0.5 + turn[0] / dt * 0.5, self.spin[1] * 0.5 + turn[1] / dt * 0.5];
                self.dragged_at = now;
            }
            self.drift = [0.0; 2];
            self.look_goal = None;
        }
        if pan && !walking {
            let d = response.drag_delta();
            let k = self.camera.distance as f64 * 0.0018;
            let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
            let step = [-(d.x as f64 * c + d.y as f64 * s) * k, -(-d.x as f64 * s + d.y as f64 * c) * k];
            self.camera.target[0] += step[0];
            self.camera.target[2] += step[1];
            if d != egui::Vec2::ZERO {
                let dt = dt as f64;
                self.drift = [self.drift[0] * 0.5 + step[0] / dt * 0.5, self.drift[1] * 0.5 + step[1] / dt * 0.5];
                self.dragged_at = now;
            }
            self.spin = [0.0; 2];
            self.look_goal = None;
        }
        // Let go while moving, the view turns or moves on a little, dying
        // away; a press stops it.
        if !(orbit || pan) {
            let pressed = ui.input(|i| i.pointer.any_pressed());
            if walking || pressed || still || (response.drag_stopped() && now - self.dragged_at > 0.06) {
                self.spin = [0.0; 2];
                self.drift = [0.0; 2];
            }
            if self.spin != [0.0; 2] || self.drift != [0.0; 2] {
                self.camera.yaw += self.spin[0] * dt;
                self.camera.pitch = (self.camera.pitch + self.spin[1] * dt).clamp(-1.2, 1.55);
                self.camera.target[0] += self.drift[0] * dt as f64;
                self.camera.target[2] += self.drift[1] * dt as f64;
                let k = (-dt / COAST).exp();
                self.spin = self.spin.map(|v| v * k);
                self.drift = self.drift.map(|v| v * k as f64);
                let slow = self.spin[0].abs() + self.spin[1].abs() < 0.02 && self.drift[0].abs() + self.drift[1].abs() < 0.02 * self.camera.distance as f64;
                if slow {
                    self.spin = [0.0; 2];
                    self.drift = [0.0; 2];
                }
                ui.ctx().request_repaint();
            }
        }
        // Gliding to look somewhere.
        if let Some(goal) = self.look_goal {
            let t = if still { 1.0 } else { 1.0 - (-dt as f64 * 12.0).exp() };
            for (v, g) in self.camera.target.iter_mut().zip(goal) {
                *v += (g - *v) * t;
            }
            let left = (0..3).map(|k| (goal[k] - self.camera.target[k]).abs()).fold(0.0, f64::max);
            if left < 0.05 + self.camera.distance as f64 * 0.0005 {
                self.camera.target = goal;
                self.look_goal = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        // The wheel: closer or farther around what the view turns about;
        // with Shift, forward or back over the land; with Alt, toward the
        // point under the cursor or away from it; with ⌘ (Ctrl), the size
        // of the sculpting brush.
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y + i.smooth_scroll_delta.x));
            let far = (extent * 3.0).max(30000.0);
            if sculpting && command && zoom != 1.0 {
                // The brush of every tool, as in Brush Settings, as wide as
                // it sculpts in 3D.
                let most = massif_core::voxel::MAX_SCULPT_RADIUS;
                let r = &mut tools.brush.radius;
                *r = (r.min(most) * zoom).clamp(1.0, most);
            } else if walking {
                // Walking, the wheel only sizes the brush.
            } else if alt && scroll != 0.0 {
                let factor = (-scroll * 0.003).exp();
                if let Some(p) = response.hover_pos() {
                    let (o, d) = self.ray(rect, p);
                    let to = massif_core::voxel::raycast(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5)
                        .map(|hit| hit.point)
                        .unwrap_or([o[0] + d[0] * self.camera.distance as f64, o[1] + d[1] * self.camera.distance as f64, o[2] + d[2] * self.camera.distance as f64]);
                    // The point stays under the cursor as the view nears it.
                    let k = 1.0 - factor as f64;
                    for (t, p) in self.camera.target.iter_mut().zip(to) {
                        *t += (p - *t) * k;
                    }
                }
                self.camera.distance = (self.camera.distance * factor).clamp(2.0, far);
            } else if shift && scroll != 0.0 {
                let step = self.camera.distance as f64 * 0.002 * scroll as f64;
                let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
                self.camera.target[0] -= s * step;
                self.camera.target[2] -= c * step;
            } else {
                let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
                if factor != 1.0 {
                    self.camera.distance = (self.camera.distance * factor).clamp(2.0, far);
                }
            }
        }

        // ---- sculpting -------------------------------------------------------
        self.hover = None;
        // Holding the mouse, the brush is where the crosshair is; else under
        // the cursor.
        let aim = if captured { Some(rect.center()) } else { response.hover_pos() };
        if sculpting && let Some(p) = aim {
            let (o, d) = self.ray(rect, p);
            self.hover = massif_core::voxel::raycast(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5);
            // The slope under the cursor, smoothed over the blocks around;
            // a little of the last one kept, so that it does not shake.
            let found = self.hover.map(|hit| massif_core::voxel::surface_frame(doc.dimension(), &hit, o, d));
            self.frame = match (found, self.frame) {
                (Some(new), Some(old)) => {
                    let n = [0.65 * new.normal[0] + 0.35 * old.normal[0], 0.65 * new.normal[1] + 0.35 * old.normal[1], 0.65 * new.normal[2] + 0.35 * old.normal[2]];
                    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    Some(if l > 0.3 {
                        massif_core::voxel::Frame {
                            center: new.center,
                            normal: [n[0] / l, n[1] / l, n[2] / l],
                        }
                    } else {
                        new
                    })
                }
                (new, _) => new,
            };
        } else {
            self.frame = None;
        }
        let (primary, secondary) = ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
        let (pressed, inverse) = match &mut self.walk {
            Some(walk) => {
                if !(primary || secondary) {
                    walk.swallow = false;
                }
                if walk.captured {
                    // As in the game: the right button builds (the tool),
                    // the left one breaks (its inverse).
                    (sculpting && !walk.swallow && (primary || secondary), primary && !secondary)
                } else {
                    // As in Axiom's editor: the right button uses the tool
                    // under the cursor (Alt: its inverse), the left one
                    // turns the view.
                    (sculpting && secondary && !command && !primary, alt)
                }
            }
            None => (sculpting && response.hovered() && !alt && !shift && (primary || secondary), secondary),
        };
        match (&mut self.sculpt, pressed) {
            (None, true) if captured || response.is_pointer_button_down_on() => {
                self.sculpt = crate::tools::begin_sculpt(doc, tools, inverse).map(|s| (s, Instant::now()));
            }
            (Some(_), false) => {
                if let Some((stroke, _)) = self.sculpt.take() {
                    crate::tools::end_sculpt(doc, stroke);
                }
            }
            _ => {}
        }
        if let (Some((stroke, last)), Some(frame)) = (&mut self.sculpt, self.frame) {
            let dt = last.elapsed().as_secs_f32().max(1.0 / 60.0);
            *last = Instant::now();
            crate::tools::sculpt_dab(doc, tools, stroke, &frame, dt);
        }
        if self.sculpt.is_some() {
            ui.ctx().request_repaint();
        }

        // Fly with the keyboard while the pointer is over the view: W A S D
        // (or the arrows) move, Q and E go down and up, Shift goes faster.
        if !walking && response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
            let (dt, keys) = ui.input(|i| {
                let k = |a: egui::Key, b: egui::Key| (i.key_down(a) || i.key_down(b)) as i32 as f64;
                let mods = i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt;
                let keys = if mods {
                    [0.0; 3]
                } else {
                    [
                        k(egui::Key::W, egui::Key::ArrowUp) - k(egui::Key::S, egui::Key::ArrowDown),
                        k(egui::Key::D, egui::Key::ArrowRight) - k(egui::Key::A, egui::Key::ArrowLeft),
                        k(egui::Key::E, egui::Key::PageUp) - k(egui::Key::Q, egui::Key::PageDown),
                    ]
                };
                (i.stable_dt.min(0.1) as f64, keys)
            });
            // The speed eases toward what the keys ask: starting and
            // stopping take a tenth of a second.
            let speed = (self.camera.distance as f64).max(40.0) * if shift { 3.0 } else { 1.0 };
            let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
            let want = [(-s * keys[0] + c * keys[1]) * speed, keys[2] * speed * 0.6, (-c * keys[0] - s * keys[1]) * speed];
            let t = if still { 1.0 } else { 1.0 - (-dt * 14.0).exp() };
            for (v, w) in self.fly.iter_mut().zip(want) {
                *v += (w - *v) * t;
            }
            if keys == [0.0; 3] && self.fly.iter().all(|v| v.abs() < 0.02 * speed) {
                self.fly = [0.0; 3];
            }
            if self.fly != [0.0; 3] {
                for (v, f) in self.camera.target.iter_mut().zip(self.fly) {
                    *v += f * dt;
                }
                self.look_goal = None;
                ui.ctx().request_repaint();
            }
        } else {
            self.fly = [0.0; 3];
        }
        // Double-click: look at the point under the cursor.
        if !walking
            && response.double_clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let (o, d) = self.ray(rect, p);
            if let Some(hit) = massif_core::voxel::raycast(doc.dimension(), o, d, 100_000.0) {
                self.look_goal = Some(hit.point);
            }
        }

        // ---- tiles -----------------------------------------------------------
        let dim = doc.dimension();
        let shape = GridShape::of(dim);
        if self.grid != shape {
            // Another stride: the tiles of the grid are not the same ones.
            if self.grid.as_ref().map(|g| g.stride) != shape.as_ref().map(|g| g.stride) {
                for t in self.tiles.keys() {
                    renderer.remove(*t);
                }
                self.tiles.clear();
            }
            match &shape {
                Some(s) => match renderer.set_grid(s.stride, s.page0, s.pages, &s.present) {
                    Ok(()) => self.error = None,
                    Err(e) => self.error = Some(e),
                },
                None => renderer.clear(),
            }
            // New textures: everything goes up again.
            self.grid = shape;
            self.invalidate_all();
            self.needs_render = true;
        }
        // Until the map is on screen, build more at once; while a view
        // moves, nothing (a tile takes longer than a frame should), so
        // that it keeps up with the hand — the building goes on as soon
        // as it stops.
        let first = self.tiles.is_empty();
        let budget = if first {
            BUILD_BUDGET * 4
        } else if crate::map_render::moved_lately(ui.ctx()) {
            Duration::ZERO
        } else {
            BUILD_BUDGET
        };
        if self.build_tiles(doc, budget, &mut renderer, ui.ctx()) {
            ui.ctx().request_repaint();
        }

        // ---- render ----------------------------------------------------------
        let ppp = ui.ctx().pixels_per_point();
        let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        if renderer.ensure_target(px) {
            let view = renderer.image_view().expect("target exists");
            let mut egui_renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(&render_state.device, view, wgpu::FilterMode::Linear, id),
                None => self.texture = Some(egui_renderer.register_native_texture(&render_state.device, view, wgpu::FilterMode::Linear)),
            }
            self.needs_render = true;
        }
        // While the camera moves, a smaller image, stretched; sharp again a
        // moment after it stops.
        let cam = &self.camera;
        let shown = [cam.target[0], cam.target[1], cam.target[2], cam.yaw as f64, cam.pitch as f64, cam.distance as f64];
        if self.last_camera.is_some_and(|c| c != shown) {
            self.moving_until = now + STILL_AFTER;
            crate::map_render::mark_moving(ui.ctx());
        }
        self.last_camera = Some(shown);
        let moving = self.reduce_while_moving && now < self.moving_until;
        let scale = if !moving { 1.0 } else if ppp >= 1.5 { MOVING_SCALE_DENSE } else { MOVING_SCALE };
        let region = [((px[0] as f32 * scale).round() as u32).clamp(1, px[0].max(1)), ((px[1] as f32 * scale).round() as u32).clamp(1, px[1].max(1))];
        if moving {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((self.moving_until - now).max(0.0) + 0.01));
        }
        let origin = (self.camera.target[0].floor() as i32, self.camera.target[2].floor() as i32);
        // The eyes under water (walking, or the camera brought down there):
        // the water's own light.
        let eye = self.camera.eye();
        let under = {
            use crate::player::Blocks;
            crate::player::MapBlocks::new(doc.dimension()).fluid(eye[0].floor() as i32, (eye[1] + 0.12).floor() as i32, eye[2].floor() as i32)
        };
        // The player's view looks like the game: its blocks, light, sky and
        // clouds, the brush marking the blocks it takes as Axiom does.
        let game = walking && self.mode == MapMode::Terrain;
        let under_water = under && self.water;
        let sky = if under_water {
            Sky::under_water(if doc.world.meta.lava { [150, 52, 12] } else { [22, 64, 112] })
        } else if game {
            Sky::GAME
        } else {
            Sky::of(&pal)
        };
        let mut globals = self.camera.globals(origin, rect.width() / rect.height().max(1.0), extent, sky, region[1].max(1));
        if game {
            let (materials, fluids) = renderer.look_info();
            let textured = self.catalogue.is_some() && materials > 0;
            globals.look = [textured as u8 as f32, 1.0, if under_water { 0.0 } else { CLOUDS_Y }, materials as f32];
            if !under_water {
                // The game's fog: the land fades into the horizon far off.
                let reach = (extent * 0.45).max(1536.0);
                globals.fog = [192.0, reach, globals.fog[2], globals.fog[3]];
            }
            let wk = doc.world.registry.well_known;
            globals.rock = [wk.stone.0 as f32, wk.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
            let local = |p: [f64; 3]| [(p[0] - origin.0 as f64) as f32, p[1] as f32, (p[2] - origin.1 as f64) as f32];
            if let Some(hit) = self.hover {
                let b = [hit.block.0 as f64, hit.block.1 as f64, hit.block.2 as f64];
                let l = local(b);
                globals.pick = [l[0], l[1], l[2], 1.0];
            }
            if let (Some(f), true) = (self.frame, sculpting) {
                let brush = tools.sculpt_brush();
                let c = local([f.center[0] as f64, f.center[1] as f64, f.center[2] as f64]);
                let square = brush.is_square() && brush.outline().is_none();
                let (t1, t2) = massif_core::voxel::tangents(f.normal);
                // A square's side, turned with the brush.
                let side = if square {
                    let k = brush.square_corners(0.0, 0.0);
                    let (du, dv) = (k[1].0 - k[0].0, k[1].1 - k[0].1);
                    let l = du.hypot(dv).max(1e-6);
                    let (du, dv) = (du / l, dv / l);
                    [t1[0] * du + t2[0] * dv, t1[1] * du + t2[1] * dv, t1[2] * du + t2[2] * dv]
                } else {
                    t1
                };
                // (A square's half side is its radius.)
                globals.brush = [c[0], c[1], c[2], brush.radius.max(0.5)];
                globals.brush_n = [f.normal[0], f.normal[1], f.normal[2], if square { 2.0 } else { 1.0 }];
                globals.brush_t = [side[0], side[1], side[2], 0.0];
            }
        }
        let key: [f32; GLOBAL_FLOATS] = bytemuck::cast(globals);
        if self.needs_render || self.last_globals != Some(key) || self.last_region != region {
            self.last_region = region;
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let dim = doc.dimension();
            let (lo_y, hi_y) = (dim.min_y as f32, dim.max_y as f32);
            let meshed: Vec<TilePos> = self
                .tiles
                .iter()
                .filter(|(_, info)| info.faces > 0)
                .map(|(t, _)| *t)
                .filter(|t| {
                    let (x, z) = ((t.x * TILE_SIZE - origin.0) as f32, (t.z * TILE_SIZE - origin.1) as f32);
                    let n = TILE_SIZE as f32;
                    in_view(&m, Vec3::new(x, lo_y, z), Vec3::new(x + n, hi_y, z + n))
                })
                .collect();
            let wk = doc.world.registry.well_known;
            let whole = matches!(self.mode, MapMode::Height | MapMode::Slope | MapMode::Biome);
            let substrate = if self.mode == MapMode::Relief {
                Substrate {
                    stone: [176, 176, 180],
                    deepslate: [176, 176, 180],
                    whole,
                }
            } else {
                Substrate {
                    stone: doc.world.registry.block(wk.stone).color,
                    deepslate: doc.world.registry.block(wk.deepslate).color,
                    whole,
                }
            };
            renderer.render(&globals, origin, &meshed, dim.min_y, substrate, region);
            self.last_globals = Some(key);
            self.needs_render = false;
        }
        let bytes = renderer.bytes();
        self.renderer = Some(renderer);
        if let Some(id) = self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(region[0] as f32 / px[0].max(1) as f32, region[1] as f32 / px[1].max(1) as f32));
            painter.image(id, rect, uv, Color32::WHITE);
        }
        if let Some(err) = &self.error {
            painter.text(rect.center(), Align2::CENTER_CENTER, err, FontId::proportional(13.0), pal.danger);
        }
        // The sculpting brush, laid on the slope under the cursor: its
        // outline (a custom brush's too), and the way the surface faces. In
        // the game's look the blocks it takes are marked instead (but for a
        // custom brush's outline).
        let marked = game && tools.sculpt_brush().outline().is_none();
        if let Some(f) = self.frame.filter(|_| !marked) {
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let to_screen = |p: [f32; 3]| -> Option<egui::Pos2> {
                let v = m * Vec4::new(p[0] - origin.0 as f32, p[1], p[2] - origin.1 as f32, 1.0);
                // Before the eyes (past the near plane), or not drawn.
                (v.w > 0.1).then(|| pos2(rect.center().x + v.x / v.w * rect.width() * 0.5, rect.center().y - v.y / v.w * rect.height() * 0.5))
            };
            let (t1, t2) = massif_core::voxel::tangents(f.normal);
            let on = |u: f32, v: f32| to_screen([f.center[0] + t1[0] * u + t2[0] * v, f.center[1] + t1[1] * u + t2[1] * v, f.center[2] + t1[2] * u + t2[2] * v]);
            let brush = &tools.sculpt_brush();
            let r = brush.radius;
            // Points behind the eyes (walking, the brush is around the
            // feet) are None: the rest is drawn in runs, without a chord
            // across the view.
            let rings: Vec<Vec<Option<egui::Pos2>>> = match brush.outline() {
                Some(outline) => outline.iter().map(|ring| ring.iter().map(|&(u, v)| on(u, v)).collect()).collect(),
                None if brush.is_square() => vec![brush.square_corners(0.0, 0.0).iter().map(|&(u, v)| on(u, v)).collect()],
                None => vec![(0..64).map(|i| {
                    let t = i as f32 / 64.0 * std::f32::consts::TAU;
                    on(t.cos() * r, t.sin() * r)
                }).collect()],
            };
            let draw = |points: Vec<egui::Pos2>, closed: bool| {
                for (w, color) in [(3.0, Color32::from_black_alpha(90)), (1.4, Color32::WHITE)] {
                    let stroke = egui::Stroke::new(w, color);
                    painter.add(if closed { egui::Shape::closed_line(points.clone(), stroke) } else { egui::Shape::line(points.clone(), stroke) });
                }
            };
            for ring in rings.into_iter().filter(|r| r.len() > 2) {
                let Some(hidden) = ring.iter().position(Option::is_none) else {
                    draw(ring.into_iter().flatten().collect(), true);
                    continue;
                };
                let mut run = Vec::new();
                for k in 1..=ring.len() {
                    match ring[(hidden + k) % ring.len()] {
                        Some(p) => run.push(p),
                        None => {
                            if run.len() > 1 {
                                draw(std::mem::take(&mut run), false);
                            }
                            run.clear();
                        }
                    }
                }
            }
            // Which way the surface faces (walking, the crosshair says
            // where the brush is, and the line would rise to the eyes).
            let walking = self.walk.is_some();
            if let (Some(c), Some(tip)) = (to_screen(f.center), to_screen([f.center[0] + f.normal[0] * r * 0.5, f.center[1] + f.normal[1] * r * 0.5, f.center[2] + f.normal[2] * r * 0.5]))
                && !walking
            {
                painter.line_segment([c, tip], egui::Stroke::new(1.4, Color32::from_white_alpha(200)));
                painter.circle_filled(c, 2.0, Color32::WHITE);
            }
        }

        // ---- overlay -------------------------------------------------------
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        let mut text = if stride > 1 {
            format!(
                "The whole map, a column every {stride} blocks (too large for every block) · {} tiles · {} MB",
                self.tiles.len() * (stride * stride) as usize,
                bytes.div_ceil(1 << 20)
            )
        } else {
            format!("The whole map, every block · {} tiles · {} MB", self.tiles.len(), bytes.div_ceil(1 << 20))
        };
        if self.pending > 0 {
            text.push_str(&format!(" · building {} tiles…", self.pending));
        } else if self.build_ms > 0.0 {
            text.push_str(&format!(" · {:.0} ms", self.build_ms));
        }
        if self.walk.is_none() {
            let at = rect.left_bottom() + vec2(10.0, -10.0);
            let galley = painter.layout_no_wrap(text, FontId::proportional(10.5), Color32::WHITE);
            let bg = Rect::from_min_size(at - vec2(4.0, galley.size().y + 4.0), galley.size() + vec2(8.0, 6.0));
            painter.rect_filled(bg, 4, Color32::from_black_alpha(110));
            painter.galley(at - vec2(0.0, galley.size().y + 1.0), galley, Color32::WHITE);
        }
        self.player_bar(ui, rect, tools);
        if captured {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(if walking {
                egui::CursorIcon::Default
            } else if sculpting {
                egui::CursorIcon::Crosshair
            } else if response.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, Landform, generate};
    use massif_core::history::History;
    use massif_core::level::LevelKind;
    use massif_core::ops::{self, Anchor, Surface};
    use massif_core::{Brush, BrushShape, Profile, World};

    fn world() -> World {
        let mut w = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Islands,
            seed: 11,
            ..GenParams::default()
        });
        let mut h = History::default();
        h.begin("levels", 0);
        let island = h.edit(&mut w, 0).structure().add_level("Island", LevelKind::Solid);
        let cave = h.edit(&mut w, 0).structure().add_level("Cave", LevelKind::Carve);
        h.commit();
        let brush = Brush {
            shape: BrushShape::Circle,
            radius: 40.0,
            profile: Profile::Dome,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        };
        h.begin("stamp", 0);
        ops::stamp_slab(&mut h.edit(&mut w, island), &brush, 60.0, -60.0, Anchor::Absolute(150.0), 30.0);
        ops::stamp_slab(&mut h.edit(&mut w, cave), &Brush { radius: 30.0, ..brush }, -80.0, 60.0, Anchor::Ground(-6.0), 30.0);
        h.commit();
        w
    }

    const STYLE: Style = Style {
        mode: MapMode::Terrain,
        water: true,
        lava: false,
    };

    #[test]
    fn walls_only_where_neighbours_leave_air() {
        let mut out = Vec::new();
        let cover = [
            CellSpan { y0: -64, y1: 10, terrain: 0, top: 10.0, color: None, mat: 0 },
            CellSpan { y0: 20, y1: 30, terrain: 0, top: 30.0, color: None, mat: 0 },
        ];
        uncovered(-64, 64, &cover, &mut out);
        assert_eq!(out, vec![(10, 20), (30, 64)]);
        uncovered(0, 5, &cover, &mut out);
        assert!(out.is_empty());
        uncovered(0, 40, &[], &mut out);
        assert_eq!(out, vec![(0, 40)]);
    }

    #[test]
    fn greedy_meshing_merges_equal_cells() {
        // A 4×4 grid: a 3×2 block of 'a', the rest 'b' except one hole.
        let key = |i: usize, j: usize| match (i, j) {
            (3, 3) => None,
            (0..=2, 0..=1) => Some('a'),
            _ => Some('b'),
        };
        let mut rects = Vec::new();
        greedy(4, key, |i, j, w, h, k| rects.push((i, j, w, h, k)));
        assert!(rects.contains(&(0, 0, 3, 2, 'a')));
        let area: usize = rects.iter().map(|r| r.2 * r.3).sum();
        assert_eq!(area, 15);
        assert!(rects.len() <= 4, "{rects:?}");
    }

    /// Columns of plain ground go to the tracer at their exact height; the
    /// others (the island, the cave) are meshed, block by block.
    #[test]
    fn tiles_trace_the_ground_and_mesh_the_rest() {
        let w = world();
        let dim = &w.dims[0];
        let tile = TilePos::new(0, -1);
        let b = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE);
        assert!(!b.faces.solid.is_empty() && !b.faces.water.is_empty());
        let grid = compose_area(dim, 0, -128, 128, 128, MapMode::Terrain, false, None);
        // Top faces of the meshed columns, per column.
        let mut tops = vec![0u8; 128 * 128];
        for f in b.faces.solid.iter().filter(|f| f.dir == UP) {
            for j in f.z as usize..f.z as usize + f.h as usize {
                for i in f.x as usize..f.x as usize + f.len as usize {
                    if grid.spans(i as i32, j as i32 - 128).last().is_some_and(|s| s.y1 == f.y as i32) {
                        tops[j * 128 + i] += 1;
                    }
                }
            }
        }
        let (mut traced, mut meshed) = (0, 0);
        for j in 0..128 {
            for i in 0..128 {
                let c = j * 128 + i;
                let spans = grid.spans(i as i32, j as i32 - 128);
                if spans.len() == 1 && spans[0].y0 <= dim.min_y {
                    assert_eq!(b.heights[c] as i32, spans[0].y1 - dim.min_y);
                    assert!((b.colors[c] as usize) < b.pairs.len());
                    assert_eq!(tops[c], 0, "a traced column is not meshed");
                    traced += 1;
                } else if !spans.is_empty() {
                    assert_eq!(b.heights[c], 0);
                    assert_eq!(tops[c], 1, "a meshed column has one top face");
                    meshed += 1;
                }
            }
        }
        assert!(traced > 10_000 && meshed > 1_000, "{traced} traced, {meshed} meshed");
        // The island's top at 165 and its underside at 135 are there.
        assert!(b.faces.solid.iter().any(|f| f.dir == UP && f.y == 165));
        assert!(b.faces.solid.iter().any(|f| f.dir == DOWN && f.y == 135));
        // Walls never extend past the chunk.
        for f in &b.faces.solid {
            match f.dir {
                EAST | WEST => assert!(f.x <= 128 && f.z as usize + f.len as usize <= 128),
                SOUTH | NORTH => assert!(f.z <= 128 && f.x as usize + f.len as usize <= 128),
                _ => assert!(f.x as usize + f.len as usize <= 128 && f.z as usize + f.h as usize <= 128),
            }
        }
        // The cave opens a pit: undersides (its roof) in the cave's tile.
        let cave = build_tile(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(-1, 0), STYLE);
        assert!(cave.faces.solid.iter().any(|f| f.dir == DOWN));
        // Plain ground has nothing to mesh but its water.
        let plain = build_tile(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(1, 1), STYLE);
        assert!(plain.faces.solid.is_empty());
        assert!(plain.heights.iter().all(|&h| h > 0));
        // Outside the map there is nothing.
        let empty = build_tile(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(20, 20), STYLE);
        assert!(empty.faces.solid.is_empty() && empty.faces.water.is_empty());
        assert!(empty.heights.iter().all(|&h| h == 0));
    }

    /// The walls of the meshed columns close every step toward their
    /// neighbours: their area equals the height differences.
    #[test]
    fn walls_close_every_step() {
        let w = world();
        let dim = &w.dims[0];
        let tile = TilePos::new(0, -1);
        let b = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, Style { mode: MapMode::Height, water: false, lava: false });
        let grid = compose_area(dim, -1, -129, 130, 130, MapMode::Height, false, None);
        let mut expected = 0i64;
        let mut gaps = Vec::new();
        for z in -128..0 {
            for x in 0..128 {
                let spans = grid.spans(x, z);
                if spans.is_empty() || (spans.len() == 1 && spans[0].y0 <= dim.min_y) {
                    continue;
                }
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    for s in spans {
                        uncovered(s.y0, s.y1, grid.spans(x + dx, z + dz), &mut gaps);
                        expected += gaps.iter().map(|(a, b)| (b - a) as i64).sum::<i64>();
                    }
                }
            }
        }
        let walls: i64 = b.faces.solid.iter().filter(|f| f.dir >= EAST).map(|f| f.len as i64 * f.h as i64).sum();
        assert!(expected > 0);
        assert_eq!(walls, expected);
    }

    #[test]
    fn colour_pairs_are_shared_and_bounded() {
        let mut pairs = Pairs::default();
        let grass = Skin {
            top_mat: 9,
            soil_mat: 10,
            ..Skin::flat([1, 2, 3], [4, 5, 6])
        };
        let b = TileBuild {
            heights: vec![10, 0, 20],
            colors: vec![0, 0, 1],
            depths: Vec::new(),
            pairs: vec![grass, Skin::flat([7, 8, 9], [1, 1, 1])],
            faces: ChunkFaces::default(),
        };
        let packed = pairs.pack(&b);
        assert_eq!(packed[1], 0, "no column, nothing to trace");
        assert_eq!(packed[0] & HEIGHT_MASK, 10);
        let (k0, k2) = (packed[0] >> HEIGHT_BITS, packed[2] >> HEIGHT_BITS);
        assert!(k0 >= 1 && k0 != k2, "pair 0 is kept for gaps");
        assert_eq!(pairs.list()[k0 as usize], grass);
        // Packing again finds the same pairs.
        assert_eq!(pairs.pack(&b), packed);
        for i in 0..5000u32 {
            pairs.get(Skin::flat([i as u8, (i >> 8) as u8, 7], [0, 0, 0]));
        }
        assert_eq!(pairs.list().len(), MAX_PAIRS);
        // Once full, a new pair takes the closest of the same blocks.
        let k = pairs.get(Skin {
            top_mat: 9,
            soil_mat: 10,
            ..Skin::flat([1, 2, 30], [4, 5, 7])
        });
        assert_eq!(k as u32, k0);
    }

    #[test]
    fn max_pyramids_keep_the_highest_column() {
        let level: Vec<u16> = (0..16).collect();
        assert_eq!(halve(&level, 4), vec![5, 7, 13, 15]);
        assert_eq!(halve(&halve(&level, 4), 2), vec![15]);
        assert_eq!(halve(&[3], 1), vec![3]);
    }

    /// How long the whole of a real world takes to build, and what it costs
    /// in GPU memory. Runs with MASSIF_WORLD_SAMPLES (a .world file).
    #[test]
    fn whole_real_world() {
        let Some(samples) = std::env::var_os("MASSIF_WORLD_SAMPLES") else { return };
        let Some(path) = std::env::split_paths(&samples).next() else { return };
        let started = Instant::now();
        let (world, _) = massif_legacy::import_world(&path, None).expect("import");
        eprintln!("import: {:.1} s", started.elapsed().as_secs_f32());
        let dim = &world.dims[0];
        let tiles: Vec<TilePos> = dim.ground().positions().collect();
        let started = Instant::now();
        let built: Vec<TileBuild> = tiles
            .par_iter()
            .map(|&t| build_tile(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, t, STYLE))
            .collect();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let faces: usize = built.iter().map(|b| b.faces.solid.len() + b.faces.water.len()).sum();
        let shape = GridShape::of(dim).unwrap();
        let pages = shape.present.iter().filter(|p| **p).count();
        let per_page = (PAGE * PAGE) as f64 * (4.0 + 0.5 * 4.0 / 3.0);
        let mut pairs = Pairs::default();
        for b in &built {
            pairs.pack(b);
        }
        eprintln!(
            "{} tiles in {ms:.0} ms: {pages} pages ({:.0} MB) for the tracer, {} k meshed faces ({:.1} MB), {} colour pairs",
            tiles.len(),
            pages as f64 * per_page / 1e6,
            faces / 1000,
            (faces * size_of::<Face>()) as f64 / 1e6,
            pairs.list().len()
        );
    }

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    /// Put every tile of a world on the GPU.
    fn load(renderer: &mut Renderer3d, w: &World, style: Style) -> Vec<TilePos> {
        let dim = &w.dims[0];
        let shape = GridShape::of(dim).unwrap();
        renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).unwrap();
        let mut pairs = Pairs::default();
        let tiles: Vec<TilePos> = dim.ground().positions().collect();
        for &t in &tiles {
            let b = build_tile(dim, &w.registry, w.meta.sea_level as f32, w.meta.seed, t, style);
            renderer.upload_columns(t, &pairs.pack(&b));
            renderer.upload(t, &b.faces);
        }
        renderer.flush();
        renderer.set_palette(pairs.list());
        tiles
    }

    fn substrate(w: &World) -> Substrate {
        Substrate {
            stone: w.registry.block(w.registry.well_known.stone).color,
            deepslate: w.registry.block(w.registry.well_known.deepslate).color,
            whole: false,
        }
    }

    fn mean(rgba: &[u8], width: u32, x0: u32, x1: u32, y0: u32, y1: u32) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        let mut n = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = ((y * width + x) * 4) as usize;
                for k in 0..3 {
                    sum[k] += rgba[p + k] as f32;
                }
                n += 1.0;
            }
        }
        sum.map(|s| s / n)
    }

    /// The tracer on a plain height field: grass at 64 in the west, a sand
    /// plateau at 90 in the east. From above, both halves fill the frame
    /// with their colours; from the side, the plateau's wall shows its sand
    /// over the rock. Needs a GPU; skipped without one.
    #[test]
    fn the_tracer_draws_the_height_field() {
        let Some((device, queue)) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let mut w = generate(&GenParams {
            tiles_x: 2,
            tiles_z: 2,
            landform: Landform::Flat,
            flat_height: 64.0,
            sea_level: 20,
            beaches: false,
            ..GenParams::default()
        });
        let grass = w.registry.terrain_by_name("Grass").unwrap();
        let sand = w.registry.terrain_by_name("Sand").unwrap();
        let half = Brush::symmetric(BrushShape::Square, Profile::Constant, 128.0);
        let mut h = History::default();
        h.begin("plateau", 0);
        ops::paint_terrain(&mut h.edit(&mut w, 0), &Brush::symmetric(BrushShape::Square, Profile::Constant, 300.0), 0.0, 0.0, grass, 0);
        ops::set_height(&mut h.edit(&mut w, 0), &half, 128.0, 0.0, 90.0, Surface::Top);
        ops::paint_terrain(&mut h.edit(&mut w, 0), &half, 128.0, 0.0, sand, 0);
        h.commit();
        let mut renderer = Renderer3d::new(&device, &queue);
        let tiles = load(&mut renderer, &w, STYLE);
        // A flat magenta sky: whatever has that colour is sky.
        let sky = [255u8, 0, 255];
        renderer.ensure_target([400, 400]);
        // Straight down on the middle of the map.
        let above = Camera {
            target: [0.0, 64.0, 0.0],
            yaw: 0.0,
            pitch: 1.55,
            distance: 160.0,
        };
        let globals = above.globals((0, 0), 1.0, 256.0, Sky::flat(sky), 400);
        renderer.render(&globals, (0, 0), &tiles, w.dims[0].min_y, substrate(&w), [8192, 8192]);
        let (width, height, rgba) = renderer.read_image().unwrap();
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") {
            image::save_buffer(std::path::Path::new(&dir).join("trace-above.png"), &rgba, width, height, image::ColorType::Rgba8).unwrap();
        }
        let west = mean(&rgba, width, 20, 180, 20, 380);
        let east = mean(&rgba, width, 220, 380, 20, 380);
        assert!(west[1] > west[0] && west[1] > west[2], "the west is grass: {west:?}");
        assert!(east[0] > east[2] + 20.0 && east[0] > west[0] + 20.0, "the east is sand: {east:?}");
        let sky_pixels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| (0..3).map(|k| (p[k] as i32 - sky[k] as i32).abs()).sum::<i32>() < 12)
            .count();
        assert_eq!(sky_pixels, 0, "the map fills the frame");

        // From the west, low: the plateau's wall, sand over stone.
        let side = Camera {
            target: [0.0, 77.0, 0.0],
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.12,
            distance: 90.0,
        };
        let globals = side.globals((0, 0), 1.0, 256.0, Sky::flat(sky), 400);
        renderer.render(&globals, (0, 0), &tiles, w.dims[0].min_y, substrate(&w), [8192, 8192]);
        let (width, height, rgba) = renderer.read_image().unwrap();
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") {
            image::save_buffer(std::path::Path::new(&dir).join("trace-side.png"), &rgba, width, height, image::ColorType::Rgba8).unwrap();
        }
        let grey = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| {
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                (r - g).abs() < 14 && (g - b).abs() < 14 && (60..200).contains(&r)
            })
            .count();
        assert!(grey > 400 * 400 / 10, "the rock of the wall shows: {grey} grey pixels");
    }

    /// Render the island world offscreen: the tracer and the meshed island
    /// and cave together. Needs a GPU; skipped without one.
    #[test]
    fn renders_headlessly() {
        let Some((device, queue)) = gpu() else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let w = world();
        let mut renderer = Renderer3d::new(&device, &queue);
        let tiles = load(&mut renderer, &w, STYLE);
        renderer.ensure_target([640, 400]);
        let camera = Camera {
            target: [20.0, 80.0, 0.0],
            distance: 420.0,
            ..Camera::default()
        };
        let origin = (20, 0);
        let globals = camera.globals(origin, 1.6, 512.0, Sky::flat([255, 0, 255]), 400);
        renderer.render(&globals, origin, &tiles, w.dims[0].min_y, substrate(&w), [8192, 8192]);
        let (width, height, rgba) = renderer.read_image().unwrap();
        assert_eq!((width, height), (640, 400));
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") {
            image::save_buffer(std::path::Path::new(&dir).join("view3d.png"), &rgba, width, height, image::ColorType::Rgba8).unwrap();
        }
        // The terrain covers a good part of the frame, and it is shaded.
        let sky = [255u8, 0, 255];
        let mut terrain = 0;
        let mut colours = std::collections::HashSet::new();
        for px in rgba.as_chunks::<4>().0 {
            let d: i32 = (0..3).map(|i| (px[i] as i32 - sky[i] as i32).abs()).sum();
            if d > 30 {
                terrain += 1;
            }
            colours.insert([px[0], px[1], px[2]]);
        }
        let share = terrain as f32 / (width * height) as f32;
        assert!(share > 0.3, "terrain covers only {:.0}% of the frame", share * 100.0);
        assert!(colours.len() > 200, "{} colours", colours.len());
        assert!(renderer.bytes() > 0);
    }
}

#[cfg(test)]
mod perf {
    use super::*;
    use massif_core::brush::{Brush, BrushShape, Profile};
    use massif_core::generate::{GenParams, Landform, generate};
    use massif_core::history::History;

    /// What the game's look costs the graphics card (MASSIF_PERF): frames
    /// of a 2,048-block map seen at a player's eyes, 1,600 × 1,000 pixels,
    /// in the views' own look and in the game's, with the game's textures
    /// where Minecraft is installed. Prints the best and the middle time.
    #[test]
    fn the_players_view_on_the_gpu() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut world = generate(&GenParams { tiles_x: 16, tiles_z: 16, landform: Landform::Hills, seed: 5, ..GenParams::default() });
        let cat = massif_assets::load_newest().and_then(Result::ok);
        if let Some(cat) = &cat {
            massif_assets::apply_colors(&mut world.registry, cat);
        }
        let mut renderer = Renderer3d::new(&device, &queue);
        let style = Style { mode: MapMode::Terrain, water: true, lava: false };
        let dim = &world.dims[0];
        let shape = GridShape::of(dim).unwrap();
        renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).unwrap();
        let mut pairs = Pairs::default();
        let tiles: Vec<TilePos> = dim.ground().positions().collect();
        for &t in &tiles {
            let b = build_tile(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, t, style);
            renderer.upload_columns(t, &pairs.pack(&b));
            renderer.upload(t, &b.faces);
        }
        renderer.flush();
        renderer.set_palette(pairs.list());
        let most = device.limits().max_texture_array_layers as usize;
        let atlas = cat.as_ref().map_or_else(crate::textures::Atlas::plain, |c| crate::textures::atlas(&world.registry, c, most));
        renderer.set_look(&atlas, &crate::textures::clouds(cat.as_ref()));
        let (materials, fluids) = renderer.look_info();
        let size = [1600u32, 1000];
        renderer.ensure_target(size);
        let ground = dim.ground().view(TilePos::new(0, 0)).map_or(70.0, |v| v.top.get(0));
        let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
        for game in [false, true] {
            let mut times = Vec::new();
            for k in 0..24 {
                let camera = Camera { target: [10.0, ground as f64 + 1.62, 10.0], yaw: 0.6 + k as f32 * 0.01, pitch: 0.12, distance: 1.0 };
                let origin = (10, 10);
                let mut globals = camera.globals(origin, 1.6, 2048.0, if game { Sky::GAME } else { Sky::of(&crate::theme::Palette::DARK) }, size[1]);
                if game {
                    globals.look = [cat.is_some() as u8 as f32, 1.0, CLOUDS_Y, materials as f32];
                    globals.fog = [192.0, 1536.0, globals.fog[2], globals.fog[3]];
                    globals.rock = [world.registry.well_known.stone.0 as f32, world.registry.well_known.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
                }
                let t = Instant::now();
                renderer.render(&globals, origin, &tiles, dim.min_y, substrate, size);
                renderer.wait();
                if k >= 4 {
                    times.push(t.elapsed().as_secs_f32() * 1000.0);
                }
            }
            times.sort_by(f32::total_cmp);
            eprintln!("{} look: best {:.1} ms, middle {:.1} ms a frame", if game { "the game's" } else { "the views' own" }, times[0], times[times.len() / 2]);
        }
    }

    /// A tile sculpted all over, rebuilt (MASSIF_PERF): its columns
    /// composed, then all of it.
    #[test]
    fn rebuilding_a_sculpted_tile() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let mut world = generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: Landform::Hills, seed: 3, ..GenParams::default() });
        let mut history = History::default();
        let brush = Brush::symmetric(BrushShape::Circle, Profile::Sine, 48.0);
        history.begin("perf", 0);
        {
            let mut edit = history.edit(&mut world, 0);
            for k in 0..8 {
                let (x, z) = (10.0 + k as f32 * 14.0, 60.0);
                let y = edit.dim().ground().view(TilePos::containing(x as i32, z as i32)).map_or(64.0, |t| t.top.get(massif_core::coords::local_index(x as i32, z as i32)));
                massif_core::voxel::displace_at(&mut edit, &brush, [x, y, z], [0.0, 1.0, 0.0], 4.0, massif_core::voxel::SculptMode::Add);
            }
        }
        history.commit();
        let dim = &world.dims[0];
        eprintln!("chunks sculpted: {}", dim.voxels.len());
        let tile = TilePos::new(0, 0);
        let style = Style { mode: MapMode::Terrain, water: true, lava: false };
        let reg = &world.registry;
        let n = TILE_SIZE as usize;
        let t = Instant::now();
        for _ in 0..10 {
            let standing = Standing { reg, seed: world.meta.seed, lava: false };
            std::hint::black_box(compose_area(dim, -1, -1, n + 2, n + 2, style.mode, style.water, Some(standing)));
        }
        eprintln!("compose a tile: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
        let t = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(build_tile(dim, reg, world.meta.sea_level as f32, world.meta.seed, tile, style));
        }
        eprintln!("build a tile: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
    }
}
