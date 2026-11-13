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
use crate::tools::{SculptStroke, ToolState};
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
/// Vertical field of view of the camera.
const FOV_Y: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Size of one chunk's slot in the uniform buffer of chunk origins.
const SLOT: u64 = 256;

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
    /// The exact height of its top (y1 is the top of its last block).
    top: f32,
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
        self.spans(x, z).last().map(|s| s.top)
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
                        last.top = s.top;
                    } else {
                        grid.spans.push(CellSpan {
                            y0: s.y0,
                            y1: s.y1,
                            terrain: s.terrain.0,
                            top: s.top,
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
    /// the column `(x, z)` whose ground slopes `slope` degrees.
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
                let c = slope_color(slope);
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

/// What one tile gives the preview.
#[derive(Debug, Default, Clone)]
pub struct TileBuild {
    /// Per column (row-major), the height of its ground above the bottom of
    /// the world, for the tracer; 0 where the tracer has nothing to draw.
    pub heights: Vec<u16>,
    /// Per column, its colours: an index into `pairs`.
    pub colors: Vec<u16>,
    /// The (top, soil) colours of the tile's traced columns.
    pub pairs: Vec<([u8; 3], [u8; 3])>,
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
    let grid = compose_area(dim, cx - ring, cz - ring, n + 2 * ring as usize, n + 2 * ring as usize, style.mode, style.water);
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
    // A column the tracer draws: one span from the bottom of the world.
    let traced = |x: i32, z: i32| {
        let s = grid.spans(x, z);
        s.len() == 1 && s[0].y0 <= min_y
    };
    // The others, when they hold anything, are meshed.
    let meshed = |x: i32, z: i32| !grid.spans(x, z).is_empty() && !traced(x, z);

    // Colours of the tops of every span, and of the soil under them.
    let mut tops: Vec<Vec<([u8; 3], [u8; 3])>> = Vec::with_capacity(n * n);
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            let (x, z) = (cx + i, cz + j);
            let slope = if style.mode == MapMode::Slope {
                trend_slope(|dx, dz| grid.top(x + dx, z + dz), SLOPE_REACH, 1.0)
            } else {
                0.0
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

    // The traced columns: height and colours.
    let mut heights = vec![0u16; n * n];
    let mut column_colors = vec![0u16; n * n];
    let mut pairs: Vec<([u8; 3], [u8; 3])> = Vec::new();
    let mut known: HashMap<([u8; 3], [u8; 3]), u16> = HashMap::new();
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (cx + i as i32, cz + j as i32);
            if !traced(x, z) {
                continue;
            }
            let c = j * n + i;
            heights[c] = (grid.spans(x, z)[0].y1 - min_y).clamp(1, HEIGHT_MASK as i32) as u16;
            let pair = tops[c][0];
            column_colors[c] = *known.entry(pair).or_insert_with(|| {
                pairs.push(pair);
                (pairs.len() - 1) as u16
            });
        }
    }

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
            if !meshed(x, z) {
                continue;
            }
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

    // Walls of the meshed columns, split into a soil band and the rock
    // below, merged along their length. (The tracer draws the walls of the
    // other columns, toward everything it does not draw itself.)
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
                if !meshed(x, z) {
                    continue;
                }
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
        let blue = if style.lava { [214, 96, 18, 230] } else { [26, 92, 150, 205] };
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
    TileBuild {
        heights,
        colors: column_colors,
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
    index: HashMap<([u8; 3], [u8; 3]), u16>,
    list: Vec<([u8; 3], [u8; 3])>,
    /// Pairs were added since the GPU last had them.
    dirty: bool,
}

impl Pairs {
    /// The index of a pair, added if new; once all are taken, the closest.
    fn get(&mut self, pair: ([u8; 3], [u8; 3])) -> u16 {
        if let Some(&k) = self.index.get(&pair) {
            return k;
        }
        if self.list.is_empty() {
            // Index 0: what shows where a far pixel lands between columns.
            self.list.push(([128, 128, 128], [112, 112, 112]));
        }
        let k = if self.list.len() < MAX_PAIRS {
            self.list.push(pair);
            self.dirty = true;
            (self.list.len() - 1) as u16
        } else {
            let d = |a: [u8; 3], b: [u8; 3]| (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2)).sum::<i32>();
            (1..self.list.len())
                .min_by_key(|&k| d(self.list[k].0, pair.0) + d(self.list[k].1, pair.1))
                .unwrap_or(0) as u16
        };
        self.index.insert(pair, k);
        k
    }

    /// The columns of a tile, packed for the tracer: height, then colours.
    pub fn pack(&mut self, b: &TileBuild) -> Vec<u32> {
        let map: Vec<u32> = b.pairs.iter().map(|&p| self.get(p) as u32).collect();
        b.heights
            .iter()
            .zip(&b.colors)
            .map(|(&h, &c)| if h == 0 { 0 } else { h as u32 | map[c as usize] << HEIGHT_BITS })
            .collect()
    }

    pub fn list(&self) -> &[([u8; 3], [u8; 3])] {
        &self.list
    }
}

// ------------------------------------------------------------- rendering --

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
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
    /// tan(fov / 2) × aspect, tan(fov / 2), radians per pixel, unused.
    pub cam_k: [f32; 4],
}

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
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) water: f32,
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
    out.water = chunk.origin.w;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let d = distance(in.world, g.eye.xyz);
    if (in.water > 0.5 && in.color.b > in.color.r) {
        // Water: its own colour seen from above, the sky mirrored where it
        // is seen at a grazing angle, a glint of the sun.
        let v = normalize(g.eye.xyz - in.world);
        let fres = pow(1.0 - max(dot(n, v), 0.0), 3.0);
        let sky = sky_color(reflect(-v, n));
        let h = normalize(v + g.sun.xyz);
        let glint = pow(max(dot(n, h), 0.0), 2400.0) * 1.4 * max(n.y, 0.0);
        var c = to_linear(in.color.rgb) * (g.fog.z + 0.3 * g.fog.w);
        c = mix(c, sky, 0.1 + 0.7 * fres) + vec3<f32>(glint);
        let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
        c = mix(c, to_linear(g.mist.rgb), f * f * g.mist.w);
        return vec4<f32>(c, mix(in.color.a, 1.0, 0.55 * fres));
    }
    let c = shade(in.color.rgb, n, d);
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
// Row 0: top colours, row 1: soil colours, of every pair.
@group(1) @binding(5) var palette: texture_2d<f32>;

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
    if (height - y <= 3.0) {
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
        out.color = vec4<f32>(sky_color(view_d), 1.0);
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
        out.color = vec4<f32>(sky_color(view_d), 1.0);
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
    out.color = vec4<f32>(shade(base, normal, hit.t), 1.0);
    let world = vec3<f32>(p.x * stride + terrain.origin.x, p.y, p.z * stride + terrain.origin.y);
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
    tracer: Option<Tracer>,
    target: Option<Target>,
    chunks: HashMap<TilePos, GpuChunk>,
}

impl Renderer3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif 3d meshes"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{GLOBALS_WGSL}{MESH_WGSL}"))),
        });
        let trace_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif 3d tracer"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("{GLOBALS_WGSL}{TRACE_WGSL}"))),
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
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
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
            bind_group_layouts: &[Some(&layout), Some(&chunk_layout)],
            immediate_size: 0,
        });
        let trace_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif 3d tracer"),
            bind_group_layouts: &[Some(&layout), Some(&trace_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Uint8x4, 1 => Sint16x2, 2 => Unorm8x4];
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
            tracer: None,
            target: None,
            chunks: HashMap::new(),
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
        let views = [array(&cols), array(&maxes), flat(&table_tex), flat(&pyramid), flat(&self.palette)];
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

    /// The colours the packed columns point at.
    pub fn set_palette(&mut self, pairs: &[([u8; 3], [u8; 3])]) {
        let n = pairs.len().min(MAX_PAIRS);
        if n == 0 {
            return;
        }
        let mut rows = vec![0u8; n * 4 * 2];
        for (k, (top, soil)) in pairs.iter().take(n).enumerate() {
            rows[k * 4..k * 4 + 3].copy_from_slice(top);
            rows[k * 4 + 3] = 255;
            rows[(n + k) * 4..(n + k) * 4 + 3].copy_from_slice(soil);
            rows[(n + k) * 4 + 3] = 255;
        }
        self.queue.write_texture(
            self.palette.as_image_copy(),
            &rows,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n as u32 * 4),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: n as u32,
                height: 2,
                depth_or_array_layers: 1,
            },
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
    pub fn render(&mut self, globals: &Globals, origin: (i32, i32), tiles: &[TilePos], bottom: i32, substrate: Substrate) {
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
                stone: unit(substrate.stone),
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
            pass.set_bind_group(0, &self.bind_group, &[]);
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
            }
        } else {
            Sky {
                zenith: [86, 140, 214],
                horizon: [200, 218, 238],
                mist: [206, 222, 240],
                haze: 0.46,
            }
        }
    }

    /// One flat colour: tests tell the sky from the ground by it.
    #[cfg(test)]
    pub fn flat(c: [u8; 3]) -> Sky {
        Sky {
            zenith: c,
            horizon: c,
            mist: c,
            haze: 0.0,
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

    /// Uniforms for a frame whose positions are relative to `origin`.
    /// `extent` is the side of the map, `height` that of the image in
    /// pixels.
    pub fn globals(&self, origin: (i32, i32), aspect: f32, extent: f32, sky: Sky, height: u32) -> Globals {
        let local = |p: [f64; 3]| Vec3::new((p[0] - origin.0 as f64) as f32, p[1] as f32, (p[2] - origin.1 as f64) as f32);
        let eye = local(self.eye());
        let target = local(self.target);
        let extent = extent.max(256.0);
        let far = self.distance + extent * 1.6 + 600.0;
        let near = (self.distance * 0.01).clamp(0.2, 8.0);
        let aspect = aspect.max(0.01);
        let proj = perspective(FOV_Y, aspect, near, far);
        let view = look_at(eye, target, Vec3::Y);
        let sun = Vec3::new(-0.42, 0.82, -0.38).normalize();
        let f = (target - eye).normalize();
        let s = f.cross(Vec3::Y).normalize();
        let u = s.cross(f);
        let tan = (FOV_Y * 0.5).tan();
        let unit = |c: [u8; 3], w: f32| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, w];
        Globals {
            view_proj: (proj * view).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            sun: [sun.x, sun.y, sun.z, 0.0],
            sky: unit(sky.zenith, 1.0),
            horizon: unit(sky.horizon, 1.0),
            // The haze only veils what is far beyond the map's middle.
            fog: [self.distance + extent * 0.4, self.distance + extent * 1.6, 0.62, 0.5],
            mist: unit(sky.mist, sky.haze),
            cam_f: [f.x, f.y, f.z, 0.0],
            cam_s: [s.x, s.y, s.z, 0.0],
            cam_u: [u.x, u.y, u.z, 0.0],
            cam_k: [tan * aspect, tan, 2.0 * tan / height.max(1) as f32, 0.0],
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
    Camera {
        target: [cx, ground_at(doc, cx, cz).unwrap_or(64.0), cz],
        distance: (extent * 0.85).max(160.0) as f32,
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
    let mut pairs: Vec<([u8; 3], [u8; 3])> = Vec::new();
    let water_pair = if style.lava { ([207, 92, 20], [160, 60, 10]) } else { ([52, 96, 200], [40, 70, 160]) };
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
        pairs,
        faces: ChunkFaces::default(),
    }
}

/// What the view knows of a tile it built.
struct TileInfo {
    faces: usize,
    stale: bool,
}

/// The 3D preview editor (shared by every 3D area).
pub struct View3d {
    pub mode: MapMode,
    pub water: bool,
    pub camera: Camera,
    renderer: Option<Renderer3d>,
    texture: Option<egui::TextureId>,
    tiles: HashMap<TilePos, TileInfo>,
    /// The pages the GPU holds.
    grid: Option<GridShape>,
    pairs: Pairs,
    style: Option<Style>,
    generation: Option<u64>,
    needs_render: bool,
    last_globals: Option<[f32; 56]>,
    /// Tiles still to build, and the time the last batch took.
    pending: usize,
    build_ms: f32,
    error: Option<String>,
    /// A sculpting stroke in progress, and when its last dab was.
    sculpt: Option<(SculptStroke, Instant)>,
    /// Where the cursor points in the world.
    hover: Option<massif_core::voxel::Hit>,
    /// Asked from the header: look where the map view is, frame the map.
    goto_map: bool,
    frame_map: bool,
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
            grid: None,
            pairs: Pairs::default(),
            style: None,
            generation: None,
            needs_render: true,
            last_globals: None,
            pending: 0,
            build_ms: 0.0,
            error: None,
            sculpt: None,
            hover: None,
            goto_map: false,
            frame_map: false,
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
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                if let Some(c) = self.tiles.get_mut(&grid_tile(t.offset(dx, dz), stride)) {
                    c.stale = true;
                }
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        for c in self.tiles.values_mut() {
            c.stale = true;
        }
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

    /// Build the missing and stale tiles of the map, nearest the camera
    /// first, for about `budget`. Returns whether some remain.
    fn build_tiles(&mut self, doc: &Document, budget: Duration, renderer: &mut Renderer3d) -> bool {
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
        };
        let dim = doc.dimension();
        let ground = dim.ground();
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        if stride > 1 {
            return self.build_coarse_tiles(doc, budget, renderer, style, stride);
        }
        // Tiles gone from the map.
        let gone: Vec<TilePos> = self.tiles.keys().copied().filter(|t| !ground.contains(*t)).collect();
        for t in gone {
            renderer.remove(t);
            self.tiles.remove(&t);
            self.needs_render = true;
        }
        let (tx, tz) = (self.camera.target[0], self.camera.target[2]);
        let mut todo: Vec<(f64, TilePos)> = ground
            .positions()
            .filter(|t| self.tiles.get(t).is_none_or(|c| c.stale))
            .map(|t| {
                let c = |v: i32| (v * TILE_SIZE + TILE_SIZE / 2) as f64;
                ((c(t.x) - tx).hypot(c(t.z) - tz), t)
            })
            .collect();
        todo.sort_by(|a, b| a.0.total_cmp(&b.0));
        let started = Instant::now();
        let batch = rayon::current_num_threads().clamp(2, 16) * 2;
        let reg = &doc.world.registry;
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let mut done = 0;
        for part in todo.chunks(batch) {
            if done > 0 && started.elapsed() > budget {
                break;
            }
            let built: Vec<(TilePos, TileBuild)> = part
                .par_iter()
                .map(|&(_, t)| (t, build_tile(dim, reg, sea, seed, t, style)))
                .collect();
            for (t, b) in built {
                let packed = self.pairs.pack(&b);
                renderer.upload_columns(t, &packed);
                renderer.upload(t, &b.faces);
                self.tiles.insert(
                    t,
                    TileInfo {
                        faces: b.faces.solid.len() + b.faces.water.len(),
                        stale: false,
                    },
                );
                done += 1;
            }
            self.needs_render = true;
        }
        if done > 0 {
            renderer.flush();
            self.build_ms = started.elapsed().as_secs_f32() * 1000.0;
        }
        if self.pairs.dirty {
            renderer.set_palette(self.pairs.list());
            self.pairs.dirty = false;
        }
        self.pending = todo.len() - done;
        self.pending > 0
    }

    /// [`View3d::build_tiles`] on a map drawn with a column every `stride`
    /// blocks: the grid's tiles, each from `stride` × `stride` of the map's.
    fn build_coarse_tiles(&mut self, doc: &Document, budget: Duration, renderer: &mut Renderer3d, style: Style, stride: i32) -> bool {
        let dim = doc.dimension();
        let (tx, tz) = (self.camera.target[0], self.camera.target[2]);
        let cells: HashSet<TilePos> = dim.ground().positions().map(|t| grid_tile(t, stride)).collect();
        let span = (TILE_SIZE * stride) as f64;
        let mut todo: Vec<(f64, TilePos)> = cells
            .into_iter()
            .filter(|t| self.tiles.get(t).is_none_or(|c| c.stale))
            .map(|t| (((t.x as f64 + 0.5) * span - tx).hypot((t.z as f64 + 0.5) * span - tz), t))
            .collect();
        todo.sort_by(|a, b| a.0.total_cmp(&b.0));
        let started = Instant::now();
        let batch = rayon::current_num_threads().clamp(2, 16);
        let reg = &doc.world.registry;
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let mut done = 0;
        for part in todo.chunks(batch) {
            if done > 0 && started.elapsed() > budget {
                break;
            }
            let built: Vec<(TilePos, TileBuild)> = part.par_iter().map(|&(_, t)| (t, build_coarse(dim, reg, sea, seed, t, stride, style))).collect();
            for (t, b) in built {
                let packed = self.pairs.pack(&b);
                renderer.upload_columns(t, &packed);
                self.tiles.insert(t, TileInfo { faces: 0, stale: false });
                done += 1;
            }
            self.needs_render = true;
        }
        if done > 0 {
            renderer.flush();
            self.build_ms = started.elapsed().as_secs_f32() * 1000.0;
        }
        if self.pairs.dirty {
            renderer.set_palette(self.pairs.list());
            self.pairs.dirty = false;
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

    /// Show the view; `sculpt`: the 3D Sculpt editor's, where the buttons
    /// sculpt with its tool.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, tools: &ToolState, map_center: Option<[f64; 2]>, sculpt: bool) {
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

        // A new world: show all of it, forget the old tiles.
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            self.camera = framing(doc);
            self.tiles.clear();
            self.grid = None;
            self.pairs = Pairs::default();
            renderer.clear();
            self.needs_render = true;
        }
        if std::mem::take(&mut self.frame_map) {
            self.camera = Camera {
                yaw: self.camera.yaw,
                ..framing(doc)
            };
        }
        if std::mem::take(&mut self.goto_map)
            && let Some(c) = map_center
        {
            self.camera.target = [c[0], ground_at(doc, c[0], c[1]).unwrap_or(self.camera.target[1]), c[1]];
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

        // ---- camera control ------------------------------------------------
        // With a sculpting tool the buttons sculpt (the right one inverts);
        // the middle button turns the view (Alt + drag too), Shift + middle
        // pans. Otherwise the left button turns, the right one pans.
        let sculpting = sculpt && tools.sculpt.tool.is_sculpt();
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
        if orbit {
            let d = response.drag_delta();
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.006).clamp(-1.2, 1.55);
        }
        if pan {
            let d = response.drag_delta();
            let k = self.camera.distance as f64 * 0.0018;
            let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
            self.camera.target[0] -= (d.x as f64 * c + d.y as f64 * s) * k;
            self.camera.target[2] -= (-d.x as f64 * s + d.y as f64 * c) * k;
        }
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y));
            let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
            if factor != 1.0 {
                self.camera.distance = (self.camera.distance * factor).clamp(2.0, (extent * 3.0).max(30000.0));
            }
        }

        // ---- sculpting -------------------------------------------------------
        self.hover = None;
        if sculpting && let Some(p) = response.hover_pos() {
            let (o, d) = self.ray(rect, p);
            self.hover = massif_core::voxel::raycast(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5);
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
        // Until the map is on screen, build more at once.
        let first = self.tiles.is_empty();
        let budget = if first { BUILD_BUDGET * 4 } else { BUILD_BUDGET };
        if self.build_tiles(doc, budget, &mut renderer) {
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
        let origin = (self.camera.target[0].floor() as i32, self.camera.target[2].floor() as i32);
        let globals = self.camera.globals(origin, rect.width() / rect.height().max(1.0), extent, Sky::of(&pal), px[1].max(1));
        let key: [f32; 56] = bytemuck::cast(globals);
        if self.needs_render || self.last_globals != Some(key) {
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
            let substrate = if self.mode == MapMode::Relief {
                Substrate {
                    stone: [176, 176, 180],
                    deepslate: [176, 176, 180],
                }
            } else {
                Substrate {
                    stone: doc.world.registry.block(wk.stone).color,
                    deepslate: doc.world.registry.block(wk.deepslate).color,
                }
            };
            renderer.render(&globals, origin, &meshed, dim.min_y, substrate);
            self.last_globals = Some(key);
            self.needs_render = false;
        }
        let bytes = renderer.bytes();
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
            let r = tools.sculpt.brush.radius as f64;
            let ring: Vec<egui::Pos2> = (0..64)
                .filter_map(|i| {
                    let t = i as f64 / 64.0 * std::f64::consts::TAU;
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
            CellSpan { y0: -64, y1: 10, terrain: 0, top: 10.0 },
            CellSpan { y0: 20, y1: 30, terrain: 0, top: 30.0 },
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
        let grid = compose_area(dim, 0, -128, 128, 128, MapMode::Terrain, false);
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
        let grid = compose_area(dim, -1, -129, 130, 130, MapMode::Height, false);
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
        let b = TileBuild {
            heights: vec![10, 0, 20],
            colors: vec![0, 0, 1],
            pairs: vec![([1, 2, 3], [4, 5, 6]), ([7, 8, 9], [1, 1, 1])],
            faces: ChunkFaces::default(),
        };
        let packed = pairs.pack(&b);
        assert_eq!(packed[1], 0, "no column, nothing to trace");
        assert_eq!(packed[0] & HEIGHT_MASK, 10);
        let (k0, k2) = (packed[0] >> HEIGHT_BITS, packed[2] >> HEIGHT_BITS);
        assert!(k0 >= 1 && k0 != k2, "pair 0 is kept for gaps");
        assert_eq!(pairs.list()[k0 as usize], ([1, 2, 3], [4, 5, 6]));
        // Packing again finds the same pairs.
        assert_eq!(pairs.pack(&b), packed);
        for i in 0..5000u32 {
            pairs.get(([i as u8, (i >> 8) as u8, 7], [0, 0, 0]));
        }
        assert_eq!(pairs.list().len(), MAX_PAIRS);
        // Once full, a new pair takes the closest.
        let k = pairs.get(([1, 2, 3], [4, 5, 7]));
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
        renderer.render(&globals, (0, 0), &tiles, w.dims[0].min_y, substrate(&w));
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
        renderer.render(&globals, (0, 0), &tiles, w.dims[0].min_y, substrate(&w));
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
        renderer.render(&globals, origin, &tiles, w.dims[0].min_y, substrate(&w));
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
