//! The 3D preview, at full detail.
//!
//! The world around the camera target is meshed block by block, one chunk
//! per tile (128 × 128 columns), from the composed level stack: every solid
//! span of a column gets its top, its underside and the walls its neighbours
//! do not cover, so overhangs, floating islands and caves open to the side
//! all show up. Faces of the same height and colour are merged, and each
//! face is one 12-byte instance the vertex shader turns into a quad.
//!
//! Chunks are cached on the GPU and rebuilt only where the world changed;
//! missing ones are built nearest first, a few at a time in parallel, within
//! a time budget per frame. The view distance only decides how far chunks
//! are kept: there is no level of detail.

use crate::doc::Document;
use crate::tools::{SculptStroke, ToolState};
use crate::map_render::{AutoBiome, ColumnSampler, MapMode, Sample, height_color, slope_color};
use crate::theme::{self, Icon, Palette};
use bytemuck::{Pod, Zeroable};
use egui::{Align2, Color32, FontId, PointerButton, Rect, Sense, Ui, pos2, vec2};
use glam::{Mat4, Vec3, Vec4};
use massif_core::compose::{Materializer, Span, TileStack};
use massif_core::coords::{TILE_SIZE, TilePos, local_index};
use massif_core::registry::{BlockId, Registry, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::world::Dimension;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

/// View distances offered, in blocks (the side of the shown square).
pub const DISTANCES: [i32; 7] = [256, 512, 768, 1024, 1536, 2048, 3072];
const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Time spent building chunks per frame, once something is on screen.
const BUILD_BUDGET: Duration = Duration::from_millis(14);
/// Vertical field of view of the camera.
const FOV_Y: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Size of one chunk's slot in the uniform buffer of chunk origins.
const SLOT: u64 = 256;

/// Directions of faces.
pub const UP: u8 = 0;
pub const DOWN: u8 = 1;
pub const EAST: u8 = 2;
pub const WEST: u8 = 3;
pub const SOUTH: u8 = 4;
pub const NORTH: u8 = 5;
/// A sloped cell of the far terrain: heights at its four corners (`y`, then
/// `h` packs the x and z deltas as two bytes, `len` the diagonal one + 128).
pub const SLOPE: u8 = 6;

/// One face, drawn as an instanced quad. Positions are in blocks from the
/// north-west corner of the chunk.
///
/// * up / down: a `len` × `h` rectangle at height `y`, `len` along x;
/// * east / west: the plane `x`, `len` blocks along z from `z`, `h` high from `y`;
/// * south / north: the plane `z`, `len` blocks along x from `x`, `h` high.
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
        self.spans(x, z).last().map(|s| s.y1 as f32)
    }

    fn cell(&self, x: i32, z: i32) -> usize {
        (z - self.z0) as usize * self.w + (x - self.x0) as usize
    }
}

/// Compose the columns of the `w` × `d` rectangle at `(x0, z0)`.
fn compose_area(dim: &Dimension, x0: i32, z0: i32, w: usize, d: usize, mode: MapMode, water: bool) -> Grid {
    let mut grid = Grid {
        x0,
        z0,
        w,
        d,
        offsets: Vec::with_capacity(w * d + 1),
        spans: Vec::new(),
        water: Vec::with_capacity(w * d),
        top: Vec::with_capacity(w * d),
    };
    grid.offsets.push(0);
    let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
    let mut sp: Vec<Span> = Vec::new();
    let mut fl: Vec<(i32, i32)> = Vec::new();
    let mut sampler = ColumnSampler::default();
    for j in 0..d as i32 {
        let z = z0 + j;
        for i in 0..w as i32 {
            let x = x0 + i;
            let pos = TilePos::containing(x, z);
            let stack = stacks.entry(pos).or_insert_with(|| TileStack::new(dim, pos, false));
            let start = grid.spans.len();
            let mut wet = i32::MIN;
            let mut top = Sample::EMPTY;
            if let Some(stack) = stack {
                let cell = local_index(x, z);
                stack.spans(cell, &mut sp);
                stack.fluids(cell, &sp, &mut fl);
                for s in &sp {
                    let contiguous = grid.spans.len() > start && grid.spans.last().is_some_and(|l: &CellSpan| l.y1 == s.y0);
                    if contiguous {
                        let last = grid.spans.last_mut().expect("checked above");
                        last.y1 = s.y1;
                        last.terrain = s.terrain.0;
                    } else {
                        grid.spans.push(CellSpan {
                            y0: s.y0,
                            y1: s.y1,
                            terrain: s.terrain.0,
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
            }
            grid.offsets.push(grid.spans.len() as u32);
            grid.water.push(wet);
            grid.top.push(top);
        }
    }
    grid
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
}

impl Colors<'_> {
    fn block(&self, id: BlockId) -> [u8; 3] {
        self.reg.block(id).color
    }

    /// Colour of the top face of a span, and of the soil just under it, for
    /// the column `(x, z)`.
    #[allow(clippy::too_many_arguments)]
    fn top(&self, terrain: u16, y0: i32, y1: i32, x: i32, z: i32, slope: f32, sample: &Sample) -> ([u8; 3], [u8; 3]) {
        match self.mode {
            MapMode::Terrain => {
                let span = Span {
                    y0,
                    y1,
                    terrain: TerrainId(terrain),
                    top: y1 as f32,
                };
                let top = self.block(self.mat.block(&span, x, y1 - 1, z, false));
                let soil = if y1 - 2 >= y0 {
                    self.block(self.mat.block(&span, x, y1 - 2, z, false))
                } else {
                    top
                };
                (top, soil)
            }
            MapMode::Height => {
                let c = height_color(y1 as f32, self.sea);
                (c, c)
            }
            MapMode::Slope => {
                let c = slope_color(slope.atan().to_degrees());
                (c, [150, 150, 150])
            }
            MapMode::Biome => {
                let id = self.auto.resolve(self.reg, sample, self.sea);
                let c = self.reg.biome(massif_core::BiomeId(id)).color;
                (c, c)
            }
            MapMode::Relief => ([206, 206, 210], [190, 190, 194]),
        }
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

/// Merges wall pieces of one line of columns: a piece continues the run of
/// the previous column when its band and colour are the same.
struct WallRuns {
    open: HashMap<(i32, i32, [u8; 4]), (usize, usize)>,
}

impl WallRuns {
    fn new() -> Self {
        Self { open: HashMap::new() }
    }

    /// A piece `[y0, y1)` of colour `c` at position `t` along the line.
    fn add(&mut self, t: usize, y0: i32, y1: i32, c: [u8; 4]) {
        let run = self.open.entry((y0, y1, c)).or_insert((t, t));
        if run.1 + 1 == t {
            run.1 = t;
        } else if run.1 != t {
            *run = (t, t);
        }
    }

    /// Close the runs that did not continue to `t`, or all of them.
    fn flush(&mut self, t: Option<usize>, mut emit: impl FnMut(usize, usize, i32, i32, [u8; 4])) {
        self.open.retain(|&(y0, y1, c), &mut (a, b)| {
            if t.is_some_and(|t| b + 1 >= t) {
                return true;
            }
            emit(a, b - a + 1, y0, y1, c);
            false
        });
    }
}

/// Mesh the chunk of tile `tile`.
pub fn build_chunk(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, tile: TilePos, style: Style) -> ChunkFaces {
    let n = TILE_SIZE as usize;
    let (cx, cz) = (tile.x * TILE_SIZE, tile.z * TILE_SIZE);
    // The chunk and a ring of neighbour columns, for walls and slopes.
    let grid = compose_area(dim, cx - 1, cz - 1, n + 2, n + 2, style.mode, style.water);
    let wk = reg.well_known;
    let colors = Colors {
        reg,
        mode: style.mode,
        sea,
        auto: AutoBiome::new(reg),
        stone: reg.block(wk.stone).color,
        deepslate: reg.block(wk.deepslate).color,
        mat: Materializer::new(reg, seed, dim.min_y),
    };
    let (min_y, max_y) = (dim.min_y, dim.max_y);
    let clamp16 = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let mut out = ChunkFaces::default();

    // Colours of the tops of every span, and of the soil under them.
    let mut tops: Vec<Vec<([u8; 3], [u8; 3])>> = Vec::with_capacity(n * n);
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            let (x, z) = (cx + i, cz + j);
            let slope = match (grid.top(x, z), grid.top(x - 1, z), grid.top(x + 1, z), grid.top(x, z - 1), grid.top(x, z + 1)) {
                (Some(c), l, r, u, d) => {
                    let dx = (r.unwrap_or(c) - l.unwrap_or(c)) / 2.0;
                    let dz = (d.unwrap_or(c) - u.unwrap_or(c)) / 2.0;
                    (dx * dx + dz * dz).sqrt()
                }
                _ => 0.0,
            };
            let sample = grid.top[grid.cell(x, z)];
            tops.push(
                grid.spans(x, z)
                    .iter()
                    .map(|s| colors.top(s.terrain, s.y0, s.y1, x, z, slope, &sample))
                    .collect(),
            );
        }
    }

    // Tops of the highest spans, merged into rectangles; other tops and the
    // undersides one by one (overhangs, islands and caves are small).
    greedy(
        n,
        |i, j| {
            let s = grid.spans(cx + i as i32, cz + j as i32).last()?;
            (s.y1 < max_y).then(|| (s.y1, rgba(tops[j * n + i].last().expect("one colour per span").0, 255)))
        },
        |i, j, w, h, (y, c)| {
            out.solid.push(Face {
                x: i as u8,
                z: j as u8,
                dir: UP,
                len: w as u8,
                y: clamp16(y),
                h: h as i16,
                color: c,
            })
        },
    );
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (cx + i as i32, cz + j as i32);
            let spans = grid.spans(x, z);
            for (k, s) in spans.iter().enumerate() {
                if k + 1 < spans.len() && s.y1 < max_y {
                    out.solid.push(Face {
                        x: i as u8,
                        z: j as u8,
                        dir: UP,
                        len: 1,
                        y: clamp16(s.y1),
                        h: 1,
                        color: rgba(tops[j * n + i][k].0, 255),
                    });
                }
                if s.y0 > min_y {
                    out.solid.push(Face {
                        x: i as u8,
                        z: j as u8,
                        dir: DOWN,
                        len: 1,
                        y: clamp16(s.y0),
                        h: 1,
                        color: rgba(theme_dark(colors.substrate(s.y0)), 255),
                    });
                }
            }
        }
    }

    // Walls, split into a soil band and the rock below, merged along their
    // length.
    let mut gaps = Vec::new();
    let mut pieces: Vec<(i32, i32, [u8; 4])> = Vec::new();
    // The pieces of wall the column `(x, z)` shows toward `(nx, nz)`.
    let mut wall = |x: i32, z: i32, nx: i32, nz: i32, pieces: &mut Vec<(i32, i32, [u8; 4])>| {
        pieces.clear();
        let (i, j) = ((x - cx) as usize, (z - cz) as usize);
        for (k, s) in grid.spans(x, z).iter().enumerate() {
            uncovered(s.y0, s.y1, grid.spans(nx, nz), &mut gaps);
            let soil_floor = (s.y1 - 3).max(s.y0);
            for &(a, b) in &gaps {
                for (lo, hi, soil) in [(a, b.min(soil_floor), false), (a.max(soil_floor), b, true)] {
                    if hi > lo {
                        let c = if soil { tops[j * n + i][k].1 } else { colors.substrate(lo) };
                        pieces.push((lo, hi, rgba(c, 255)));
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
            let mut emit = |start: usize, len: usize, y0: i32, y1: i32, c: [u8; 4]| {
                let (x, z) = if along_z { (plane, start as u8) } else { (start as u8, plane) };
                solid.push(Face {
                    x,
                    z,
                    dir,
                    len: len as u8,
                    y: clamp16(y0),
                    h: clamp16(y1 - y0),
                    color: c,
                });
            };
            for t in 0..n {
                runs.flush(Some(t), &mut emit);
                let (i, j) = if along_z { (line, t) } else { (t, line) };
                let (x, z) = (cx + i as i32, cz + j as i32);
                wall(x, z, x + dx, z + dz, &mut pieces);
                for &(y0, y1, c) in &pieces {
                    runs.add(t, y0, y1, c);
                }
            }
            runs.flush(None, &mut emit);
        }
    }

    // Water: its surface merged into rectangles, and its sides where the
    // map ends.
    if style.water {
        let blue = if style.lava { [214, 96, 18, 230] } else { [52, 104, 210, 168] };
        greedy(
            n,
            |i, j| {
                let wet = grid.water[grid.cell(cx + i as i32, cz + j as i32)];
                (wet != i32::MIN).then_some(wet)
            },
            |i, j, w, h, wet| {
                out.water.push(Face {
                    x: i as u8,
                    z: j as u8,
                    dir: UP,
                    len: w as u8,
                    y: clamp16(wet),
                    h: h as i16,
                    color: blue,
                })
            },
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
                    if !inside || !grid.spans(nx, nz).is_empty() {
                        continue;
                    }
                    let (px, pz) = (i as i32 + dx.max(0), j as i32 + dz.max(0));
                    out.water.push(Face {
                        x: px as u8,
                        z: pz as u8,
                        dir,
                        len: 1,
                        y: clamp16(floor),
                        h: clamp16(wet - floor),
                        color: blue,
                    });
                }
            }
        }
    }
    out
}

fn theme_dark(c: [u8; 3]) -> [u8; 3] {
    [c[0] / 2, c[1] / 2, c[2] / 2]
}

// ------------------------------------------------------------ far terrain --

/// Blocks per cell of the far terrain.
pub const FAR_STEP: i32 = 16;
/// Tiles per side of a far chunk (512 blocks, 32 × 32 cells).
pub const FAR_TILES: i32 = 4;
const FAR_CELLS: usize = (FAR_TILES * TILE_SIZE / FAR_STEP) as usize;
const CELLS_PER_TILE: usize = (TILE_SIZE / FAR_STEP) as usize;

/// The far chunk holding a tile.
pub fn far_key(tile: TilePos) -> TilePos {
    TilePos::new(tile.x.div_euclid(FAR_TILES), tile.z.div_euclid(FAR_TILES))
}

/// The whole map beyond the detailed area: one column sampled every
/// `FAR_STEP` blocks, drawn as blocks of that size. Positions along x and z
/// are in cells; faces never span two tiles, so the shader can hide the
/// tiles drawn in full detail.
pub fn build_far(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, key: TilePos, style: Style) -> ChunkFaces {
    let n = FAR_CELLS;
    let m = n + 2;
    let (bx, bz) = (key.x * FAR_TILES * TILE_SIZE, key.z * FAR_TILES * TILE_SIZE);
    let wk = reg.well_known;
    let colors = Colors {
        reg,
        mode: style.mode,
        sea,
        auto: AutoBiome::new(reg),
        stone: reg.block(wk.stone).color,
        deepslate: reg.block(wk.deepslate).color,
        mat: Materializer::new(reg, seed, dim.min_y),
    };
    let clamp16 = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    // Top, colour and water of every cell, with a ring of neighbours.
    let mut tops: Vec<Option<(i32, [u8; 3])>> = vec![None; m * m];
    // A cell stands for 256 columns: in terrain mode it takes the mean
    // colours of its terrain's layers, not one block picked at random.
    let mut means: HashMap<u16, ([u8; 3], [u8; 3])> = HashMap::new();
    let mut mean_colors = |terrain: u16| {
        *means.entry(terrain).or_insert_with(|| {
            let def = reg.terrain(TerrainId(terrain));
            let mean = |i: usize| {
                let s = def.strata.get(i)?;
                let total: u32 = s.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
                let mut c = [0u32; 3];
                for &(b, w) in &s.blocks {
                    let bc = reg.block(b).color;
                    for k in 0..3 {
                        c[k] += bc[k] as u32 * w as u32;
                    }
                }
                Some(c.map(|v| (v / total) as u8))
            };
            let top = mean(0).unwrap_or(def.color);
            (top, mean(1).unwrap_or(top))
        })
    };
    let mut wet = vec![i32::MIN; m * m];
    let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
    let (mut sp, mut fl) = (Vec::new(), Vec::new());
    let mut sampler = ColumnSampler::default();
    for j in 0..m {
        for i in 0..m {
            let x = bx + (i as i32 - 1) * FAR_STEP + FAR_STEP / 2;
            let z = bz + (j as i32 - 1) * FAR_STEP + FAR_STEP / 2;
            let pos = TilePos::containing(x, z);
            let Some(stack) = stacks.entry(pos).or_insert_with(|| TileStack::new(dim, pos, false)) else {
                continue;
            };
            let cell = local_index(x, z);
            stack.spans(cell, &mut sp);
            let Some(top) = sp.last().copied() else { continue };
            let inner = (1..=n).contains(&i) && (1..=n).contains(&j);
            let (color, _) = if !inner {
                ([0; 3], [0; 3])
            } else if style.mode == MapMode::Terrain {
                mean_colors(top.terrain.0)
            } else {
                let sample = if style.mode == MapMode::Biome { sampler.sample(stack, cell) } else { Sample::EMPTY };
                colors.top(top.terrain.0, top.y0, top.y1, x, z, 0.0, &sample)
            };
            tops[j * m + i] = Some((top.y1, color));
            if style.water {
                stack.fluids(cell, &sp, &mut fl);
                if let Some(&(f0, f1)) = fl.last()
                    && f0 == top.y1
                {
                    wet[j * m + i] = f1;
                }
            }
        }
        // Rows of tiles behind are done with.
        if stacks.len() > 32 {
            let row = TilePos::containing(0, bz + (j as i32 - 1) * FAR_STEP).z;
            stacks.retain(|p, _| p.z >= row);
        }
    }
    let mut out = ChunkFaces::default();
    // A smooth surface through the corners of the cells: each corner is the
    // mean height of the cells around it, so neighbouring far chunks meet.
    let corner = |ci: usize, cj: usize| -> Option<i32> {
        let mut sum = 0i32;
        let mut count = 0;
        for (a, b) in [(ci, cj), (ci + 1, cj), (ci, cj + 1), (ci + 1, cj + 1)] {
            if let Some((y, _)) = tops[b * m + a] {
                sum += y;
                count += 1;
            }
        }
        (count > 0).then(|| (sum as f32 / count as f32).round() as i32)
    };
    let corners: Vec<Option<i32>> = (0..(n + 1) * (n + 1)).map(|c| corner(c % (n + 1), c / (n + 1))).collect();
    let at_corner = |i: usize, j: usize| corners[j * (n + 1) + i];
    for j in 0..n {
        for i in 0..n {
            let Some((_, color)) = tops[(j + 1) * m + i + 1] else { continue };
            let (Some(c00), Some(c10), Some(c01), Some(c11)) = (at_corner(i, j), at_corner(i + 1, j), at_corner(i, j + 1), at_corner(i + 1, j + 1))
            else {
                continue;
            };
            let d = |c: i32| (c - c00).clamp(-127, 127);
            let (d1, d2, d3) = (d(c10), d(c01), d(c11));
            out.solid.push(Face {
                x: i as u8,
                z: j as u8,
                dir: SLOPE,
                len: (d3 + 128) as u8,
                y: clamp16(c00),
                h: ((d1 as u8 as u16) | ((d2 as u8 as u16) << 8)) as i16,
                color: rgba(color, 255),
            });
        }
    }
    // Water, flat, merged within each tile.
    if style.water {
        for tj in 0..FAR_TILES as usize {
            for ti in 0..FAR_TILES as usize {
                let (oi, oj) = (ti * CELLS_PER_TILE, tj * CELLS_PER_TILE);
                greedy(
                    CELLS_PER_TILE,
                    |i, j| {
                        let w = wet[(oj + j + 1) * m + oi + i + 1];
                        (w != i32::MIN).then_some(w)
                    },
                    |i, j, w, h, y| {
                        out.water.push(Face {
                            x: (oi + i) as u8,
                            z: (oj + j) as u8,
                            dir: UP,
                            len: w as u8,
                            y: clamp16(y),
                            h: h as i16,
                            color: if style.lava { [214, 96, 18, 230] } else { [52, 104, 210, 168] },
                        })
                    },
                );
            }
        }
    }
    out
}

// ------------------------------------------------------------- rendering --

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub sun: [f32; 4],
    pub sky: [f32; 4],
    /// Fog start, fog end, ambient weight, sun weight.
    pub fog: [f32; 4],
    /// The rectangle drawn in full detail (x0, z0, x1, z1, from the origin
    /// of the frame): the far terrain stays out of it.
    pub detail: [f32; 4],
}

/// Where a chunk is, relative to the origin of the frame.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ChunkUniform {
    /// x, y, z offset; w = 1 for water.
    origin: [f32; 4],
    /// Blocks per unit along x and z (1, or `FAR_STEP` for the far terrain).
    params: [f32; 4],
}

const SHADER: &str = r#"
struct Globals {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    sky: vec4<f32>,
    fog: vec4<f32>,
    detail: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

struct Chunk {
    origin: vec4<f32>,
    params: vec4<f32>,
};
@group(1) @binding(0) var<uniform> chunk: Chunk;

struct FaceIn {
    @location(0) xz_dir_len: vec4<u32>,
    @location(1) y_h: vec2<i32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, f: FaceIn) -> VertexOut {
    // Corners (0,0) (1,0) (1,1) (0,0) (1,1) (0,1): two triangles.
    let u = f32((0x16u >> vi) & 1u);
    let v = f32((0x34u >> vi) & 1u);
    let k = chunk.params.x;
    let dir = f.xz_dir_len.z;
    let x = f32(f.xz_dir_len.x) * k;
    let z = f32(f.xz_dir_len.y) * k;
    var len = f32(f.xz_dir_len.w) * k;
    if (dir == 6u) {
        len = k;
    }
    let y = f32(f.y_h.x);
    var h = f32(f.y_h.y);
    if (dir < 2u) {
        h = h * k;
    }
    if (dir == 6u) {
        h = k;
    }
    // The far terrain gives way to the area drawn in full detail: find the
    // cell the face belongs to and drop the face if it lies there.
    if (k > 1.5) {
        var c = vec2<f32>(x + len * 0.5, z + h * 0.5);
        if (dir == 2u) { c = vec2<f32>(x - k * 0.5, z + len * 0.5); }
        if (dir == 3u) { c = vec2<f32>(x + k * 0.5, z + len * 0.5); }
        if (dir == 4u) { c = vec2<f32>(x + len * 0.5, z - k * 0.5); }
        if (dir == 5u) { c = vec2<f32>(x + len * 0.5, z + k * 0.5); }
        let w = c + chunk.origin.xz;
        if (w.x > g.detail.x && w.x < g.detail.z && w.y > g.detail.y && w.y < g.detail.w) {
            var gone: VertexOut;
            gone.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
            gone.color = vec4<f32>(0.0);
            gone.normal = vec3<f32>(0.0, 1.0, 0.0);
            gone.world = vec3<f32>(0.0);
            return gone;
        }
    }
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
        case 5u: {
            p = vec3<f32>(x + v * len, y + u * h, z);
            n = vec3<f32>(0.0, 0.0, -1.0);
        }
        default: {
            // A sloped cell: corner heights from the packed deltas.
            let packed = f.y_h.y;
            let d1 = f32((packed << 24u) >> 24u);
            let d2 = f32(packed >> 8u);
            let d3 = f32(i32(f.xz_dir_len.w) - 128);
            // v runs along x, u along z (counter-clockwise from above).
            let hx0 = mix(y, y + d1, v);
            let hx1 = mix(y + d2, y + d3, v);
            p = vec3<f32>(x + v * k, mix(hx0, hx1, u), z + u * k);
            let dx = ((d1 + d3) - d2) * 0.5;
            let dz = ((d2 + d3) - d1) * 0.5;
            n = normalize(vec3<f32>(-dx / k, 1.0, -dz / k));
            top = 1.0e9;
        }
    }
    // Water sits a little below the top of its block.
    if (chunk.origin.w > 0.5 && p.y >= top - 0.01) {
        p.y = p.y - 0.12;
    }
    let world = p + chunk.origin.xyz;
    var out: VertexOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.color = f.color;
    out.normal = n;
    out.world = world;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let base = to_linear(in.color.rgb);
    let sun = max(dot(n, g.sun.xyz), 0.0);
    let sky = 0.55 + 0.45 * n.y;
    var c = base * (g.fog.z * sky + g.fog.w * sun);
    let d = distance(in.world, g.eye.xyz);
    let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
    c = mix(c, to_linear(g.sky.rgb), f * f);
    return vec4<f32>(c, in.color.a);
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

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    #[cfg_attr(not(test), allow(dead_code))]
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// GPU resources of the 3D preview. Independent of egui, so it can render
/// headlessly in tests.
pub struct Renderer3d {
    device: wgpu::Device,
    queue: wgpu::Queue,
    opaque_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    chunk_layout: wgpu::BindGroupLayout,
    /// One slot per drawn chunk (twice: solid and water).
    origins: wgpu::Buffer,
    origins_group: wgpu::BindGroup,
    slots: u64,
    target: Option<Target>,
    chunks: HashMap<TilePos, GpuChunk>,
    /// Far terrain, per far chunk.
    far: HashMap<TilePos, GpuChunk>,
}

impl Renderer3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif 3d"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let uniform = |dynamic: bool, size: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: NonZeroU64::new(size),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d globals"),
            entries: &[uniform(false, size_of::<Globals>() as u64)],
        });
        let chunk_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif 3d chunk"),
            entries: &[uniform(true, size_of::<ChunkUniform>() as u64)],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif 3d"),
            bind_group_layouts: &[Some(&layout), Some(&chunk_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Uint8x4, 1 => Sint16x2, 2 => Unorm8x4];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<Face>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        })];
        let pipeline = |label: &str, blend: wgpu::BlendState, depth_write: bool, cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
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
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque_pipeline = pipeline("massif 3d opaque", wgpu::BlendState::REPLACE, true, Some(wgpu::Face::Back));
        let water_pipeline = pipeline("massif 3d water", wgpu::BlendState::ALPHA_BLENDING, false, None);
        let (origins, origins_group) = Self::make_origins(device, &chunk_layout, 64);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            opaque_pipeline,
            water_pipeline,
            globals,
            bind_group,
            chunk_layout,
            origins,
            origins_group,
            slots: 64,
            target: None,
            chunks: HashMap::new(),
            far: HashMap::new(),
        }
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

    /// Put (or replace) the faces of a chunk on the GPU.
    pub fn upload(&mut self, tile: TilePos, faces: &ChunkFaces) {
        let chunk = GpuChunk {
            solid: self.upload_faces(&faces.solid, "massif 3d chunk"),
            water: self.upload_faces(&faces.water, "massif 3d water"),
        };
        self.chunks.insert(tile, chunk);
    }

    /// Put (or replace) a far chunk on the GPU.
    pub fn upload_far(&mut self, key: TilePos, faces: &ChunkFaces) {
        let chunk = GpuChunk {
            solid: self.upload_faces(&faces.solid, "massif 3d far"),
            water: self.upload_faces(&faces.water, "massif 3d far water"),
        };
        self.far.insert(key, chunk);
    }

    /// Forget the chunks `keep` says no to.
    pub fn retain(&mut self, mut keep: impl FnMut(TilePos) -> bool) {
        self.chunks.retain(|t, _| keep(*t));
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.far.clear();
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
        let texture = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
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
            resolve,
            depth: view(&depth),
        });
        true
    }

    pub fn image_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.resolve_view)
    }

    /// Draw the detailed chunks `tiles` and the far chunks `far` into the
    /// render target. Positions are made relative to `origin` (block
    /// coordinates), the origin of `globals`.
    pub fn render(&mut self, globals: &Globals, origin: (i32, i32), tiles: &[TilePos], far: &[TilePos]) {
        if self.target.is_none() {
            return;
        }
        // (key, far?) of everything drawn, with its offset and scale.
        let mut drawn: Vec<(TilePos, bool, f32, f32, f32)> = Vec::new();
        for t in tiles.iter().filter(|t| self.chunks.contains_key(t)) {
            drawn.push((*t, false, (t.x * TILE_SIZE - origin.0) as f32, (t.z * TILE_SIZE - origin.1) as f32, 1.0));
        }
        let span = FAR_TILES * TILE_SIZE;
        for k in far.iter().filter(|k| self.far.contains_key(k)) {
            drawn.push((*k, true, (k.x * span - origin.0) as f32, (k.z * span - origin.1) as f32, FAR_STEP as f32));
        }
        let needed = (drawn.len() as u64 * 2).max(1);
        if needed > self.slots {
            self.slots = needed.next_power_of_two();
            let (buffer, group) = Self::make_origins(&self.device, &self.chunk_layout, self.slots);
            self.origins = buffer;
            self.origins_group = group;
        }
        // Slot 2k is chunk k's solid faces, 2k + 1 its water.
        let mut data = vec![0u8; (drawn.len() * 2).max(1) * SLOT as usize];
        for (k, &(_, _, ox, oz, scale)) in drawn.iter().enumerate() {
            for (slot, water) in [(2 * k, 0.0), (2 * k + 1, 1.0)] {
                let u = ChunkUniform {
                    origin: [ox, 0.0, oz, water],
                    params: [scale, 0.0, 0.0, 0.0],
                };
                data[slot * SLOT as usize..][..size_of::<ChunkUniform>()].copy_from_slice(bytemuck::bytes_of(&u));
            }
        }
        self.queue.write_buffer(&self.origins, 0, &data);
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
        let target = self.target.as_ref().expect("checked above");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif 3d") });
        {
            let sky = globals.sky;
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
            pass.set_bind_group(0, &self.bind_group, &[]);
            // Solid chunks first, then the water over them.
            for water in [false, true] {
                pass.set_pipeline(if water { &self.water_pipeline } else { &self.opaque_pipeline });
                for (k, (t, is_far, ..)) in drawn.iter().enumerate() {
                    let chunk = if *is_far { &self.far[t] } else { &self.chunks[t] };
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

    /// Uniforms for a mesh whose positions are relative to `origin`.
    /// `view_size` is the side of the detailed area, `extent` that of
    /// everything drawn (the whole map).
    pub fn globals(&self, origin: (i32, i32), aspect: f32, view_size: f32, extent: f32, sky: [u8; 3]) -> Globals {
        let local = |p: [f64; 3]| Vec3::new((p[0] - origin.0 as f64) as f32, p[1] as f32, (p[2] - origin.1 as f64) as f32);
        let eye = local(self.eye());
        let target = local(self.target);
        let extent = extent.max(view_size);
        let far = self.distance + extent * 1.6 + 600.0;
        let near = (self.distance * 0.01).clamp(0.2, 8.0);
        let proj = perspective(FOV_Y, aspect.max(0.01), near, far);
        let view = look_at(eye, target, Vec3::Y);
        let sun = Vec3::new(-0.42, 0.82, -0.38).normalize();
        Globals {
            view_proj: (proj * view).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            sun: [sun.x, sun.y, sun.z, 0.0],
            sky: [sky[0] as f32 / 255.0, sky[1] as f32 / 255.0, sky[2] as f32 / 255.0, 1.0],
            fog: [self.distance + view_size * 0.6, self.distance + extent * 1.5, 0.44, 0.66],
            detail: [0.0; 4],
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

// ----------------------------------------------------------------- widget --

/// A rectangle of blocks: x0, z0, x1, z1.
type BlockBox = (i32, i32, i32, i32);

/// Height of the ground (or the water on it) at a point, as composed.
fn ground_at(doc: &Document, x: f64, z: f64) -> Option<f64> {
    let p = crate::tools::probe(doc, x.floor() as i32, z.floor() as i32)?;
    Some(p.height.max(if p.fluid != NO_FLUID { p.fluid as f32 } else { f32::MIN }) as f64)
}

/// What the view knows of a chunk it built.
struct ChunkInfo {
    faces: usize,
    stale: bool,
}

/// The 3D preview editor (shared by every 3D area).
pub struct View3d {
    pub mode: MapMode,
    pub distance: usize,
    pub water: bool,
    pub follow_map: bool,
    pub camera: Camera,
    renderer: Option<Renderer3d>,
    texture: Option<egui::TextureId>,
    chunks: HashMap<TilePos, ChunkInfo>,
    /// Far terrain chunks built, for the whole map.
    far: HashMap<TilePos, ChunkInfo>,
    style: Option<Style>,
    generation: Option<u64>,
    needs_render: bool,
    last_globals: Option<[f32; 36]>,
    /// Chunks still to build in view, and the time the last batch took.
    pending: usize,
    build_ms: f32,
    error: Option<String>,
    /// A sculpting stroke in progress, and when its last dab was.
    sculpt: Option<(SculptStroke, Instant)>,
    /// Where the cursor points in the world.
    hover: Option<massif_core::voxel::Hit>,
    /// The target is still easing toward the ground under the map's centre.
    settling: bool,
}

impl Default for View3d {
    fn default() -> Self {
        Self {
            mode: MapMode::Terrain,
            distance: 2,
            water: true,
            follow_map: true,
            camera: Camera::default(),
            renderer: None,
            texture: None,
            chunks: HashMap::new(),
            far: HashMap::new(),
            style: None,
            generation: None,
            needs_render: true,
            last_globals: None,
            pending: 0,
            build_ms: 0.0,
            error: None,
            sculpt: None,
            hover: None,
            settling: false,
        }
    }
}

impl View3d {
    pub fn header(&mut self, ui: &mut Ui) {
        let narrow = ui.available_width() < 560.0;
        crate::map_view::mode_selector(ui, &mut self.mode, narrow, "3d mode");
        ui.separator();
        ui.toggle_value(&mut self.water, theme::glyph(Icon::Waves).to_string())
            .on_hover_text("Show water");
        ui.toggle_value(&mut self.follow_map, theme::glyph(Icon::Crosshair).to_string())
            .on_hover_text("Follow the centre of the map");
        let reset = |camera: &mut Camera, distance: usize| {
            *camera = Camera {
                target: camera.target,
                distance: DISTANCES[distance] as f32 * 0.62,
                ..Camera::default()
            };
        };
        if narrow {
            ui.menu_button(theme::glyph(Icon::SlidersHorizontal).to_string(), |ui| {
                for (i, d) in DISTANCES.iter().enumerate() {
                    ui.radio_value(&mut self.distance, i, format!("{d} blocks"));
                }
                ui.separator();
                if ui.button(theme::with_icon(Icon::Rotate3d, "Reset the view")).clicked() {
                    reset(&mut self.camera, self.distance);
                }
            })
            .response
            .on_hover_text(format!("View distance: {} blocks", DISTANCES[self.distance]));
        } else {
            egui::ComboBox::from_id_salt("view distance")
                .width(96.0)
                .selected_text(format!("{} blocks", DISTANCES[self.distance]))
                .show_ui(ui, |ui| {
                    for (i, d) in DISTANCES.iter().enumerate() {
                        ui.selectable_value(&mut self.distance, i, format!("{d} blocks"));
                    }
                });
            if ui.small_button(theme::glyph(Icon::Rotate3d).to_string()).on_hover_text("Reset the view").clicked() {
                reset(&mut self.camera, self.distance);
            }
        }
    }

    /// Terrain changed: rebuild the chunks showing it, and their neighbours
    /// (whose walls face it).
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        for t in tiles {
            if let Some(c) = self.far.get_mut(&far_key(*t)) {
                c.stale = true;
            }
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(c) = self.chunks.get_mut(&t.offset(dx, dz)) {
                    c.stale = true;
                }
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        for c in self.chunks.values_mut().chain(self.far.values_mut()) {
            c.stale = true;
        }
    }

    /// Build missing and stale far chunks of the map, nearest first, until
    /// `deadline`. Returns whether some remain.
    fn build_far(&mut self, doc: &Document, deadline: Instant, renderer: &mut Renderer3d) -> bool {
        let dim = doc.dimension();
        let Some((lo, hi)) = dim.bounds() else { return false };
        let (a, b) = (far_key(lo), far_key(hi));
        let (tx, tz) = (self.camera.target[0], self.camera.target[2]);
        let span = (FAR_TILES * TILE_SIZE) as f64;
        let mut todo: Vec<(f64, TilePos)> = Vec::new();
        for z in a.z..=b.z {
            for x in a.x..=b.x {
                let k = TilePos::new(x, z);
                if self.far.get(&k).is_none_or(|c| c.stale) {
                    let d = ((x as f64 + 0.5) * span - tx).hypot((z as f64 + 0.5) * span - tz);
                    todo.push((d, k));
                }
            }
        }
        if todo.is_empty() {
            return false;
        }
        todo.sort_by(|p, q| p.0.total_cmp(&q.0));
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
        };
        let reg = &doc.world.registry;
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let batch = rayon::current_num_threads().clamp(2, 16);
        let mut done = 0;
        for part in todo.chunks(batch) {
            if Instant::now() > deadline {
                break;
            }
            let built: Vec<(TilePos, ChunkFaces)> = part
                .par_iter()
                .map(|&(_, k)| (k, build_far(dim, reg, sea, seed, k, style)))
                .collect();
            for (k, faces) in built {
                renderer.upload_far(k, &faces);
                self.far.insert(
                    k,
                    ChunkInfo {
                        faces: faces.solid.len() + faces.water.len(),
                        stale: false,
                    },
                );
                done += 1;
            }
            self.needs_render = true;
        }
        done < todo.len()
    }

    /// The detailed tiles to draw: the rings of `view` around the camera
    /// target that are entirely built, so the far terrain never shows
    /// through holes. Also returns their rectangle in blocks.
    fn detailed(&self, view: &[TilePos]) -> (Vec<TilePos>, Option<BlockBox>) {
        let centre = TilePos::containing(self.camera.target[0].floor() as i32, self.camera.target[2].floor() as i32);
        let ring = |t: &TilePos| (t.x - centre.x).abs().max((t.z - centre.z).abs());
        let mut complete = i32::MAX;
        for t in view {
            if !self.chunks.contains_key(t) {
                complete = complete.min(ring(t) - 1);
            }
        }
        let tiles: Vec<TilePos> = view.iter().copied().filter(|t| ring(t) <= complete).collect();
        let rect = tiles.iter().fold(None, |r: Option<BlockBox>, t| {
            let (x0, z0) = (t.x * TILE_SIZE, t.z * TILE_SIZE);
            let (x1, z1) = (x0 + TILE_SIZE, z0 + TILE_SIZE);
            Some(match r {
                None => (x0, z0, x1, z1),
                Some((a, b, c, d)) => (a.min(x0), b.min(z0), c.max(x1), d.max(z1)),
            })
        });
        (tiles, rect)
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

    /// Tiles of the square of side `size` around the camera target, nearest
    /// first.
    fn tiles_in_view(&self, size: i32) -> Vec<TilePos> {
        let (tx, tz) = (self.camera.target[0], self.camera.target[2]);
        let half = size as f64 / 2.0;
        let lo = TilePos::containing((tx - half).floor() as i32, (tz - half).floor() as i32);
        let hi = TilePos::containing((tx + half).floor() as i32, (tz + half).floor() as i32);
        let mut tiles: Vec<(f64, TilePos)> = Vec::new();
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                let cx = (x * TILE_SIZE + TILE_SIZE / 2) as f64;
                let cz = (z * TILE_SIZE + TILE_SIZE / 2) as f64;
                tiles.push(((cx - tx).hypot(cz - tz), TilePos::new(x, z)));
            }
        }
        tiles.sort_by(|a, b| a.0.total_cmp(&b.0));
        tiles.into_iter().map(|(_, t)| t).collect()
    }

    /// Build missing and stale chunks of `view`, nearest first, for about
    /// `budget`. Returns whether some remain.
    fn build_chunks(&mut self, doc: &Document, view: &[TilePos], budget: Duration, renderer: &mut Renderer3d) -> bool {
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
        };
        let dim = doc.dimension();
        let ground = dim.ground();
        let todo: Vec<TilePos> = view
            .iter()
            .copied()
            .filter(|t| self.chunks.get(t).is_none_or(|c| c.stale))
            .collect();
        // Tiles outside the map have nothing to show: no need to mesh them,
        // unless a level other than the ground reaches there.
        let todo: Vec<TilePos> = todo
            .into_iter()
            .filter(|t| {
                let has = (-1..=1).any(|dz| (-1..=1).any(|dx| ground.contains(t.offset(dx, dz))))
                    || dim.levels.iter().any(|l| l.contains(*t));
                if !has {
                    self.chunks.insert(*t, ChunkInfo { faces: 0, stale: false });
                    renderer.retain(|p| p != *t);
                }
                has
            })
            .collect();
        let started = Instant::now();
        let batch = rayon::current_num_threads().clamp(2, 16);
        let reg = &doc.world.registry;
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let mut done = 0;
        for part in todo.chunks(batch) {
            if done > 0 && started.elapsed() > budget {
                break;
            }
            let built: Vec<(TilePos, ChunkFaces)> = part
                .par_iter()
                .map(|&t| (t, build_chunk(dim, reg, sea, seed, t, style)))
                .collect();
            for (t, faces) in built {
                renderer.upload(t, &faces);
                self.chunks.insert(
                    t,
                    ChunkInfo {
                        faces: faces.solid.len() + faces.water.len(),
                        stale: false,
                    },
                );
                done += 1;
            }
            self.needs_render = true;
        }
        if done > 0 {
            self.build_ms = started.elapsed().as_secs_f32() * 1000.0;
        }
        self.pending = todo.len() - done;
        self.pending > 0
    }

    /// The ray through screen point `p` of the view `rect`: origin and
    /// direction in block coordinates.
    fn ray(&self, rect: Rect, p: egui::Pos2) -> ([f64; 3], [f64; 3]) {
        let eye = self.camera.eye();
        let t = self.camera.target;
        let f = Vec3::new((t[0] - eye[0]) as f32, (t[1] - eye[1]) as f32, (t[2] - eye[2]) as f32).normalize();
        let s = f.cross(Vec3::Y).normalize();
        let u = s.cross(f);
        let half = (FOV_Y * 0.5).tan();
        let aspect = rect.width() / rect.height().max(1.0);
        let nx = (p.x - rect.center().x) / (rect.width() * 0.5);
        let ny = -(p.y - rect.center().y) / (rect.height() * 0.5);
        let d = f + s * (nx * half * aspect) + u * (ny * half);
        (eye, [d.x as f64, d.y as f64, d.z as f64])
    }

    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, tools: &ToolState, map_center: Option<[f64; 2]>) {
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

        // A new world: look at its centre, forget the old chunks.
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            let center = doc.dimension().bounds().map_or([0.0, 0.0], |(lo, hi)| {
                [((lo.x + hi.x + 1) * 64) as f64, ((lo.z + hi.z + 1) * 64) as f64]
            });
            let ground = ground_at(doc, center[0], center[1]).unwrap_or(64.0);
            self.camera = Camera {
                target: [center[0], ground, center[1]],
                distance: DISTANCES[self.distance.min(DISTANCES.len() - 1)] as f32 * 0.62,
                ..Camera::default()
            };
            self.chunks.clear();
            self.far.clear();
            renderer.clear();
            self.needs_render = true;
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

        // ---- camera control ------------------------------------------------
        // With a sculpting tool the buttons sculpt (the right one inverts);
        // the middle button turns the view (Alt + drag too), Shift + middle
        // pans. Otherwise the left button turns, the right one pans.
        let sculpting = tools.tool.is_sculpt();
        let (shift, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let (orbit, pan) = if sculpting {
            (
                (response.dragged_by(PointerButton::Middle) && !shift) || (response.dragged_by(PointerButton::Primary) && alt),
                response.dragged_by(PointerButton::Middle) && shift,
            )
        } else {
            (
                response.dragged_by(PointerButton::Primary),
                response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle),
            )
        };
        let mut moved = false;
        if orbit {
            let d = response.drag_delta();
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.006).clamp(-1.2, 1.55);
            moved = true;
        }
        if pan {
            let d = response.drag_delta();
            let k = self.camera.distance as f64 * 0.0018;
            let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
            self.camera.target[0] -= (d.x as f64 * c + d.y as f64 * s) * k;
            self.camera.target[2] -= (-d.x as f64 * s + d.y as f64 * c) * k;
            self.follow_map = false;
            moved = true;
        }
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y));
            let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
            if factor != 1.0 {
                self.camera.distance = (self.camera.distance * factor).clamp(2.0, 30000.0);
                moved = true;
            }
        }

        // ---- sculpting -------------------------------------------------------
        let size = DISTANCES[self.distance.min(DISTANCES.len() - 1)];
        self.hover = None;
        if sculpting && let Some(p) = response.hover_pos() {
            let (o, d) = self.ray(rect, p);
            self.hover = massif_core::voxel::raycast(doc.dimension(), o, d, (self.camera.distance as f64 + size as f64) * 1.5);
        }
        let (primary, secondary) = ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down()));
        let pressed = sculpting && response.hovered() && !alt && (primary || secondary);
        match (&mut self.sculpt, pressed) {
            (None, true) if response.is_pointer_button_down_on() => {
                self.sculpt = crate::tools::begin_sculpt(doc, tools, secondary).map(|s| (s, Instant::now()));
            }
            (Some(_), false) => {
                if let Some((stroke, _)) = self.sculpt.take() {
                    crate::tools::end_sculpt(doc, stroke);
                }
            }
            _ => {}
        }
        if let (Some((stroke, last)), Some(hit)) = (&mut self.sculpt, self.hover) {
            let dt = last.elapsed().as_secs_f32().max(1.0 / 60.0);
            *last = Instant::now();
            let center = [hit.point[0] as f32, hit.point[1] as f32, hit.point[2] as f32];
            crate::tools::sculpt_dab(doc, tools, stroke, center, dt);
        }
        if self.sculpt.is_some() {
            ui.ctx().request_repaint();
        }

        // Fly with the keyboard while the pointer is over the view: W A S D
        // (or the arrows) move, Q and E go down and up, Shift goes faster.
        if response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
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
            if keys != [0.0; 3] {
                let speed = (self.camera.distance as f64).max(40.0) * if shift { 3.0 } else { 1.0 } * dt;
                let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
                self.camera.target[0] += (-s * keys[0] + c * keys[1]) * speed;
                self.camera.target[2] += (-c * keys[0] - s * keys[1]) * speed;
                self.camera.target[1] += keys[2] * speed * 0.6;
                self.follow_map = false;
                moved = true;
                ui.ctx().request_repaint();
            }
        }
        // Double-click: look at the point under the cursor.
        if response.double_clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            let (o, d) = self.ray(rect, p);
            if let Some(hit) = massif_core::voxel::raycast(doc.dimension(), o, d, 100_000.0) {
                self.camera.target = hit.point;
                self.follow_map = false;
                moved = true;
            }
        }
        // Following the map: its centre, with the height easing toward the
        // ground there (no jumps, and the camera never sticks to the ground).
        if self.follow_map
            && let Some(c) = map_center
            && (c[0] != self.camera.target[0] || c[1] != self.camera.target[2] || self.settling)
        {
            self.camera.target[0] = c[0];
            self.camera.target[2] = c[1];
            if let Some(y) = ground_at(doc, c[0], c[1]) {
                let dy = y - self.camera.target[1];
                self.camera.target[1] += dy * 0.25;
                self.settling = dy.abs() > 0.5;
                if self.settling {
                    ui.ctx().request_repaint();
                }
            }
            moved = true;
        }
        let _ = moved;

        // ---- chunks ----------------------------------------------------------
        let view = self.tiles_in_view(size);
        // Forget chunks well out of view.
        let keep: HashSet<TilePos> = self.tiles_in_view(size + 2 * TILE_SIZE).into_iter().collect();
        let before = self.chunks.len();
        self.chunks.retain(|t, _| keep.contains(t));
        renderer.retain(|t| keep.contains(&t));
        if self.chunks.len() != before {
            self.needs_render = true;
        }
        // Until something is on screen, build more at once. The detail
        // nearest the camera comes first, then the far terrain of the map.
        let first = !view.iter().any(|t| self.chunks.contains_key(t));
        let budget = if first { BUILD_BUDGET * 4 } else { BUILD_BUDGET };
        let started = Instant::now();
        let mut more = self.build_chunks(doc, &view, budget, &mut renderer);
        if !more || self.far.is_empty() {
            more |= self.build_far(doc, started + budget, &mut renderer);
        } else {
            more = true;
        }
        if more {
            ui.ctx().request_repaint();
        }
        let (detail, detail_rect) = self.detailed(&view);
        let far_keys: Vec<TilePos> = self.far.keys().copied().collect();
        let extent = doc
            .dimension()
            .bounds()
            .map_or(size as f32, |(lo, hi)| (((hi.x - lo.x + 1).max(hi.z - lo.z + 1)) * TILE_SIZE) as f32);

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
        let origin = (self.camera.target[0].floor() as i32, self.camera.target[2].floor() as i32);
        let sky: [u8; 3] = [pal.sky.r(), pal.sky.g(), pal.sky.b()];
        let mut globals = self.camera.globals(origin, rect.width() / rect.height().max(1.0), size as f32, extent, sky);
        if let Some((x0, z0, x1, z1)) = detail_rect {
            globals.detail = [(x0 - origin.0) as f32, (z0 - origin.1) as f32, (x1 - origin.0) as f32, (z1 - origin.1) as f32];
        }
        let key: [f32; 36] = bytemuck::cast(globals);
        if self.needs_render || self.last_globals != Some(key) {
            renderer.render(&globals, origin, &detail, &far_keys);
            self.last_globals = Some(key);
            self.needs_render = false;
        }
        self.renderer = Some(renderer);
        if let Some(id) = self.texture {
            painter.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if let Some(err) = &self.error {
            painter.text(rect.center(), Align2::CENTER_CENTER, err, FontId::proportional(13.0), pal.danger);
        }
        // The sculpting brush: a ring where the ball meets the surface.
        if let Some(hit) = self.hover {
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let to_screen = |p: [f64; 3]| -> Option<egui::Pos2> {
                let v = m * Vec4::new((p[0] - origin.0 as f64) as f32, p[1] as f32, (p[2] - origin.1 as f64) as f32, 1.0);
                (v.w > 0.0).then(|| pos2(rect.center().x + v.x / v.w * rect.width() * 0.5, rect.center().y - v.y / v.w * rect.height() * 0.5))
            };
            let n = [hit.normal.0 as f64, hit.normal.1 as f64, hit.normal.2 as f64];
            // Two axes across the face that was hit.
            let a = if n[1] != 0.0 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
            let b = [n[1] * a[2] - n[2] * a[1], n[2] * a[0] - n[0] * a[2], n[0] * a[1] - n[1] * a[0]];
            let r = tools.brush.radius as f64;
            let ring: Vec<egui::Pos2> = (0..48)
                .filter_map(|i| {
                    let t = i as f64 / 48.0 * std::f64::consts::TAU;
                    let (c, s) = (t.cos() * r, t.sin() * r);
                    to_screen([
                        hit.point[0] + a[0] * c + b[0] * s,
                        hit.point[1] + a[1] * c + b[1] * s,
                        hit.point[2] + a[2] * c + b[2] * s,
                    ])
                })
                .collect();
            if ring.len() > 2 {
                painter.add(egui::Shape::closed_line(ring.clone(), egui::Stroke::new(3.0, Color32::from_black_alpha(90))));
                painter.add(egui::Shape::closed_line(ring, egui::Stroke::new(1.4, Color32::WHITE)));
            }
            if let Some(c) = to_screen(hit.point) {
                painter.circle_filled(c, 2.0, Color32::WHITE);
            }
        }

        // ---- overlay -------------------------------------------------------
        let faces: usize = view.iter().filter_map(|t| self.chunks.get(t)).map(|c| c.faces).sum::<usize>()
            + self.far.values().map(|c| c.faces).sum::<usize>();
        let mut text = format!("Full detail over {size} blocks, the whole map beyond · {} k faces", faces / 1000);
        if self.pending > 0 {
            text.push_str(&format!(" · building {} chunks…", self.pending));
        } else if self.build_ms > 0.0 {
            text.push_str(&format!(" · {:.0} ms", self.build_ms));
        }
        let at = rect.left_bottom() + vec2(10.0, -10.0);
        let galley = painter.layout_no_wrap(text, FontId::proportional(10.5), Color32::WHITE);
        let bg = Rect::from_min_size(at - vec2(4.0, galley.size().y + 4.0), galley.size() + vec2(8.0, 6.0));
        painter.rect_filled(bg, 4, Color32::from_black_alpha(110));
        painter.galley(at - vec2(0.0, galley.size().y + 1.0), galley, Color32::WHITE);
        if response.hovered() {
            ui.ctx().set_cursor_icon(if sculpting {
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
    use massif_core::ops::{self, Anchor};
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
        let cover = [CellSpan { y0: -64, y1: 10, terrain: 0 }, CellSpan { y0: 20, y1: 30, terrain: 0 }];
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

    /// Every column of a chunk is drawn at its own height: no cells of
    /// several blocks, whatever the view distance.
    #[test]
    fn chunks_are_meshed_block_by_block() {
        let w = world();
        let dim = &w.dims[0];
        let tile = TilePos::new(0, -1);
        let faces = build_chunk(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE);
        assert!(!faces.solid.is_empty() && !faces.water.is_empty());
        // The tops of the ground cover every column of the tile exactly once.
        let mut covered = vec![0u8; 128 * 128];
        let stack = TileStack::new(dim, tile, false).unwrap();
        let mut sp = Vec::new();
        for f in faces.solid.iter().filter(|f| f.dir == UP) {
            for j in f.z as usize..f.z as usize + f.h as usize {
                for i in f.x as usize..f.x as usize + f.len as usize {
                    let cell = j * 128 + i;
                    stack.spans(cell, &mut sp);
                    if sp.last().unwrap().y1 == f.y as i32 {
                        covered[cell] += 1;
                    }
                }
            }
        }
        assert!(covered.iter().all(|&c| c == 1), "every column has one top face");
        // The island's top at 165 and its underside at 135 are there.
        assert!(faces.solid.iter().any(|f| f.dir == UP && f.y == 165));
        assert!(faces.solid.iter().any(|f| f.dir == DOWN && f.y == 135));
        // Merging pays: far fewer faces than columns × sides.
        assert!(faces.solid.len() < 128 * 128 * 3, "{} faces", faces.solid.len());
        // Walls never extend past the chunk.
        for f in &faces.solid {
            match f.dir {
                EAST | WEST => assert!(f.x <= 128 && f.z as usize + f.len as usize <= 128),
                SOUTH | NORTH => assert!(f.z <= 128 && f.x as usize + f.len as usize <= 128),
                _ => assert!(f.x as usize + f.len as usize <= 128 && f.z as usize + f.h as usize <= 128),
            }
        }
        // The cave opens a pit: undersides (its roof) in the cave's tile.
        let cave = build_chunk(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(-1, 0), STYLE);
        assert!(cave.solid.iter().any(|f| f.dir == DOWN));
        // Outside the map there is nothing.
        let empty = build_chunk(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(20, 20), STYLE);
        assert!(empty.solid.is_empty() && empty.water.is_empty());
    }

    /// Walls must close every step between neighbouring columns: summing
    /// the wall area of a chunk equals the height differences.
    #[test]
    fn walls_close_every_step() {
        let w = world();
        let dim = &w.dims[0];
        let tile = TilePos::new(-1, -1);
        let faces = build_chunk(dim, &w.registry, 63.0, w.meta.seed, tile, Style { mode: MapMode::Height, water: false, lava: false });
        let grid = compose_area(dim, -129, -129, 130, 130, MapMode::Height, false);
        let (cx, cz) = (-128, -128);
        let mut expected = 0i64;
        let mut gaps = Vec::new();
        for j in 0..128 {
            for i in 0..128 {
                let (x, z) = (cx + i, cz + j);
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    for s in grid.spans(x, z) {
                        uncovered(s.y0, s.y1, grid.spans(x + dx, z + dz), &mut gaps);
                        expected += gaps.iter().map(|(a, b)| (b - a) as i64).sum::<i64>();
                    }
                }
            }
        }
        let walls: i64 = faces
            .solid
            .iter()
            .filter(|f| f.dir >= EAST)
            .map(|f| f.len as i64 * f.h as i64)
            .sum();
        assert_eq!(walls, expected);
    }

    /// How long a full-detail view of a real world takes to mesh. Runs with
    /// MASSIF_WORLD_SAMPLES (a .world file); prints the timings.
    #[test]
    fn full_detail_of_a_real_world() {
        let Some(samples) = std::env::var_os("MASSIF_WORLD_SAMPLES") else { return };
        let Some(path) = std::env::split_paths(&samples).next() else { return };
        let started = Instant::now();
        let (world, _) = massif_legacy::import_world(&path, None).expect("import");
        eprintln!("import: {:.1} s", started.elapsed().as_secs_f32());
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        let centre = TilePos::new((lo.x + hi.x) / 2, (lo.z + hi.z) / 2);
        for size in [768, 2048] {
            let r = size / 2 / TILE_SIZE;
            let tiles: Vec<TilePos> = (-r..r).flat_map(|z| (-r..r).map(move |x| centre.offset(x, z))).collect();
            let started = Instant::now();
            let chunks: Vec<ChunkFaces> = tiles
                .par_iter()
                .map(|&t| build_chunk(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, t, STYLE))
                .collect();
            let faces: usize = chunks.iter().map(|c| c.solid.len() + c.water.len()).sum();
            eprintln!(
                "{size} blocks: {} chunks, {} k faces ({:.1} MB), {:.0} ms",
                tiles.len(),
                faces / 1000,
                (faces * size_of::<Face>()) as f64 / 1e6,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// The far terrain covers the map with blocks of `FAR_STEP`, never
    /// merging across tiles, at the height of the sampled columns.
    #[test]
    fn far_terrain_covers_the_map_coarsely() {
        let w = world();
        let dim = &w.dims[0];
        let far = build_far(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(-1, -1), STYLE);
        let cells: Vec<&Face> = far.solid.iter().filter(|f| f.dir == SLOPE).collect();
        // The map's 2 × 2 tiles in this far chunk: one sloped cell each.
        assert_eq!(cells.len(), 4 * CELLS_PER_TILE * CELLS_PER_TILE);
        // A cell's corner sits near the height of the columns around it.
        let x = -512 + 19 * FAR_STEP;
        let z = -512 + 21 * FAR_STEP;
        let p = TileStack::new(dim, TilePos::containing(x, z), false).unwrap();
        let mut sp = Vec::new();
        p.spans(local_index(x, z), &mut sp);
        let cell = cells.iter().find(|f| (f.x, f.z) == (19, 21)).unwrap();
        assert!((cell.y as i32 - sp.last().unwrap().y1).abs() < 24, "{} vs {}", cell.y, sp.last().unwrap().y1);
        // Neighbouring cells share their corners: the surface is closed.
        let right = cells.iter().find(|f| (f.x, f.z) == (20, 21)).unwrap();
        let d1 = (cell.h as u16 & 0xff) as u8 as i8 as i32;
        assert_eq!(cell.y as i32 + d1, right.y as i32);
        assert!(!far.water.is_empty());
        assert_eq!(far_key(TilePos::new(-1, 3)), TilePos::new(-1, 0));
    }

    /// Render the preview offscreen. Needs a GPU; skipped without one.
    #[test]
    fn renders_headlessly() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let w = world();
        let mut renderer = Renderer3d::new(&device, &queue);
        let mut tiles = Vec::new();
        for z in -3..3 {
            for x in -3..3 {
                let t = TilePos::new(x, z);
                renderer.upload(t, &build_chunk(&w.dims[0], &w.registry, 63.0, w.meta.seed, t, STYLE));
                tiles.push(t);
            }
        }
        renderer.ensure_target([640, 400]);
        let camera = Camera {
            target: [20.0, 80.0, 0.0],
            distance: 420.0,
            ..Camera::default()
        };
        let origin = (20, 0);
        // The far terrain of the whole map around, hidden where the detail is.
        let far = TilePos::new(-1, -1);
        renderer.upload_far(far, &build_far(&w.dims[0], &w.registry, 63.0, w.meta.seed, far, STYLE));
        let mut globals = camera.globals(origin, 1.6, 768.0, 768.0, [178, 210, 240]);
        globals.detail = [-128.0 - 20.0, -128.0, 128.0 - 20.0, 128.0];
        renderer.render(&globals, origin, &tiles, &[far]);
        let (width, height, rgba) = renderer.read_image().unwrap();
        assert_eq!((width, height), (640, 400));
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") {
            image::save_buffer(std::path::Path::new(&dir).join("view3d.png"), &rgba, width, height, image::ColorType::Rgba8).unwrap();
        }
        // The terrain covers a good part of the frame, and it is shaded.
        let sky = [178u8, 210, 240];
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
        assert!(colours.len() > 300, "{} colours", colours.len());
    }
}
