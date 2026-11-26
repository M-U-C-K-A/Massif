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

const SAMPLES: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Time spent building tiles per frame, once something is on screen.
const BUILD_BUDGET: Duration = Duration::from_millis(14);
/// From how many tiles to build the work goes away from the frames (see
/// [`Builder`]).
const BUILD_AWAY: usize = 32;
/// Cells to build from which a first look at all of them comes first (see
/// [`build_rough`]).
const ROUGH_FROM: usize = 256;
/// The most time a frame spends putting built tiles on the graphics card;
/// while a view moves, less.
const UPLOAD_BUDGET: Duration = Duration::from_millis(6);
const UPLOAD_BUDGET_MOVING: Duration = Duration::from_millis(2);
/// Pages across and down (of [`PAGE`] blocks) drawn with every block
/// around the eyes on a map drawn with a column every few blocks: the eyes'
/// page and those around it, a page of land at least on every side.
const NEAR_PAGES: i32 = 3;
/// How high over the land the eyes may be for every block around them to
/// be drawn: higher, a block is smaller than a pixel anyway.
const NEAR_HEIGHT: f64 = 1500.0;
/// Time a still frame spends building again the tiles around the eyes an
/// edit changed (the others are built away from the frames).
const NEAR_BUDGET: Duration = Duration::from_millis(8);

/// The closest the orbiting camera comes to the point it turns about, in
/// blocks: wheeled toward the land, a few blocks over it, whatever the
/// size of the map.
const MIN_DISTANCE: f32 = 3.0;
/// Vertical field of view of the camera.
const FOV_Y: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// Walking, Minecraft's (wider: the land is all around).
const FOV_WALK: f32 = 70.0 * std::f32::consts::PI / 180.0;
/// Faces of the first page of a face arena (see [`Arena`]); each new page
/// holds twice as many, up to [`ARENA_FACES`].
const FIRST_ARENA_FACES: u32 = 1 << 16;
const ARENA_FACES: u32 = 1 << 21;
/// Columns of the texture of the chunks' tiles (see [`Renderer3d::upload`]).
const SLOT_ROW: u32 = 256;
/// The height of the game's clouds (since 1.18).
const CLOUDS_Y: f32 = 192.0;

/// Columns per side of a page of the tracer's textures.
pub const PAGE: i32 = 1024;
const PAGE_SHIFT: u32 = 10;
/// Tiles per side of a page.
const PAGE_TILES: i32 = PAGE / TILE_SIZE;
/// Colour pairs (top, soil) the tracer tells apart.
pub const MAX_PAIRS: usize = 4096;
/// Pages the tracer may hold at most (about 1.9 GB; Settings ▸ Graphics
/// sets fewer). A larger map is drawn with a column every 2, 4, 8… blocks:
/// still all of it, coarser.
#[cfg(test)]
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
/// A face's direction with a piece's height, in eighths of a block (1 to
/// 7): its top (up) lies that much over `y`, its sides are that part of
/// `h` high (see [`Kind::eighths`](massif_core::shapes::Kind::eighths)).
pub const fn part(dir: u8, eighths: u8) -> u8 {
    dir | (eighths & 7) << 3
}

/// The settings of Smooth terrain the tiles are built with (the map's,
/// set by the view before it builds them).
static SMOOTH: std::sync::RwLock<Option<std::sync::Arc<massif_core::shapes::SmoothSettings>>> = std::sync::RwLock::new(None);

/// Build the tiles with these settings of Smooth terrain; whether they
/// changed.
pub fn set_smooth(settings: &massif_core::shapes::SmoothSettings) -> bool {
    let Ok(mut s) = SMOOTH.write() else { return false };
    if s.as_deref() == Some(settings) {
        return false;
    }
    *s = Some(std::sync::Arc::new(settings.clone()));
    true
}

fn smooth_settings() -> std::sync::Arc<massif_core::shapes::SmoothSettings> {
    SMOOTH.read().ok().and_then(|s| s.clone()).unwrap_or_default()
}

/// One face, drawn as an instanced quad. Positions are in blocks from the
/// north-west corner of the chunk.
///
/// * up / down: a `len` × `h` rectangle at height `y`, `len` along x;
/// * east / west: the plane `x`, `len` blocks along z from `z`, `h` high from `y`;
/// * south / north: the plane `z`, `len` blocks along x from `x`, `h` high.
///
/// `mat` says what it is made of, for the game's textures (see
/// [`object_bases`], [`ROCK`]); 0: nothing known, its colour. `pad` is set
/// on the graphics card to its chunk's slot, which says where the chunk is.
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
    /// The floors and ceilings of the caves under the ground, drawn through
    /// it while the X-ray is on.
    pub xray: Vec<Face>,
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
    /// The objects of the layers drawn (the structures always are).
    objects: bool,
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
    let objects = s.objects && reg.object_layers.iter().any(|l| !l.removed && !l.hidden && !l.objects.is_empty());
    if !objects && dim.structures.is_empty() {
        return runs;
    }
    let bases = object_bases(reg);
    let area = massif_core::BlockRect::new(x0, z0, x0 + w as i32, z0 + d as i32);
    let mat = Materializer::new(reg, s.seed, dim.min_y);
    let mut placed = Vec::new();
    for t in area.expand(max_reach(reg)).tiles().filter(|_| objects) {
        let Some(stack) = TileStack::from_above(dim, t, false) else { continue };
        for p in tile_placements(reg, &mat, s.seed as u64, s.lava, &stack, t) {
            if reg.object_layer(p.layer).is_some_and(|l| !l.hidden) && reg.objects.get(p.object as usize).is_some_and(|def| !p.bounds(def).intersect(&area).is_empty()) {
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
    // The structures placed, over the objects as in the export: their
    // blocks (their air, which carves their halls, is not drawn).
    for st in &dim.structures {
        if st.columns().intersect(&area).is_empty() {
            continue;
        }
        for sec in &st.sections {
            let (sx, sz) = (sec.pos[0] * 16, sec.pos[2] * 16);
            if sx + 16 <= x0 || sx >= x0 + w as i32 || sz + 16 <= z0 || sz >= z0 + d as i32 {
                continue;
            }
            for &(i, k) in &sec.blocks {
                if st.is_air(k) {
                    continue;
                }
                let [x, y, z] = sec.block(i);
                let (i, j) = (x - x0, z - z0);
                if i < 0 || j < 0 || i >= w as i32 || j >= d as i32 {
                    continue;
                }
                blocks[j as usize * w + i as usize].push((y, st.color(k), 0));
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

/// The pieces of Smooth terrain of tile `tile`, as the export lays them:
/// where (`i`, `j` in the tile), at what height and how many eighths high,
/// and their colour and material. A piece in place of its column's top
/// block lowers the column in `grid` a block. Pieces of a terrain's mix
/// take its first block's.
fn smooth_pieces(dim: &Dimension, reg: &Registry, tile: TilePos, grid: &mut Grid) -> Vec<(usize, usize, i32, u8, [u8; 3], u16)> {
    use massif_core::shapes::{Shape, ShapeSet, choose, family, plan_tile, top_block};
    let settings = smooth_settings();
    let plan = plan_tile(dim, &settings, tile);
    if plan.is_empty() {
        return Vec::new();
    }
    let mods = crate::blocks::mod_blocks();
    let families = crate::blocks::families();
        let known = massif_core::shapes::Known { vanilla: &crate::blocks::vanilla_exists, mods: &mods, families: &families };
    let mut families: HashMap<u16, massif_core::shapes::Family> = HashMap::new();
    let (cx, cz) = tile.origin();
    let n = TILE_SIZE as usize;
    let mut out = Vec::new();
    for c in plan {
        let (i, j) = (c.cell as usize % n, c.cell as usize / n);
        let (x, z) = (cx + i as i32, cz + j as i32);
        let Some(block) = c.block.or_else(|| top_block(reg, c.terrain)) else { continue };
        let f = families.entry(block.0).or_insert_with(|| family(reg.block(block).name(), &known));
        let mut may = massif_core::shapes::usable(
            match c.block {
                Some(_) => ShapeSet::of(&[Shape::Slab, Shape::Stairs]),
                None => settings.shapes_of(reg, c.terrain),
            },
            f,
        );
        // Only on the ground's top, in the air (as the export: never under
        // an object or a level over it); under a cover's plants, its height
        // only, in the cover's colour.
        let cell = grid.cell(x, z);
        let cover = reg.cover(grid.cover[cell]);
        if let Some(d) = cover {
            // The game's plants cannot sit on layers: the top stays whole
            // under them, as the export leaves it.
            if !d.is_laid() && !d.changes_water() && !d.plants.iter().all(|(s, _)| s.contains("layers=")) {
                continue;
            }
            may = may.and(ShapeSet::of(&[Shape::Layers, Shape::Slab]));
        }
        let Some(piece) = choose(&c, may, f.slab_layers) else { continue };
        let (a, b) = (grid.offsets[cell] as usize, grid.offsets[cell + 1] as usize);
        let Some(last) = grid.spans[a..b].last().copied() else { continue };
        if last.y1 != c.y1 || last.color.is_some() {
            continue;
        }
        if piece.replaces {
            grid.spans[b - 1].y1 -= 1;
            grid.spans[b - 1].top -= 1.0;
            if grid.spans[b - 1].y1 <= grid.spans[b - 1].y0 {
                continue;
            }
        }
        let (color, mat) = (cover.map_or(reg.block(block).color, |d| d.color), material(block.0 as u32));
        out.push((i, j, piece.y, piece.kind.eighths(), color, mat));
    }
    out
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
fn compose_area(dim: &Dimension, x0: i32, z0: i32, w: usize, d: usize, mode: MapMode, water: bool, caves: massif_core::compose::Caves, standing: Option<Standing>) -> Grid {
    let runs = standing.map(|s| object_runs(dim, s, x0, z0, w, d));
    // The tiles the area lies on, each composed once; then its rows side by
    // side (a tile rebuilt alone, as while sculpting, uses every core).
    let (t0, t1) = (TilePos::containing(x0, z0), TilePos::containing(x0 + w as i32 - 1, z0 + d as i32 - 1));
    let tiles: Vec<TilePos> = (t0.z..=t1.z).flat_map(|tz| (t0.x..=t1.x).map(move |tx| TilePos::new(tx, tz))).collect();
    let stacks: HashMap<TilePos, TileStack> = tiles.par_iter().filter_map(|&p| TileStack::new_seen(dim, p, caves, |_, level| level.visible).map(|s| (p, s))).collect();
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
                    // The water over the land: on its highest span, or on
                    // the bottom of the world over ground that deep.
                    if water
                        && let Some(&(f0, f1)) = fl.last()
                        && f0 == sp.last().map_or(dim.min_y, |t| t.y1)
                    {
                        wet = f1;
                    }
                    // Ground at the bottom of the world leaves its column no
                    // block; a block of its terrain stands for that bottom
                    // here. With nothing, the column was taken for the end
                    // of the map: no water over it, and the water around it
                    // walled down to its floor, a pit in the sea.
                    if sp.is_empty() {
                        let ground = stack.ground();
                        sp.push(Span { y0: dim.min_y, y1: dim.min_y + 1, terrain: TerrainId(ground.terrain.get(cell)), top: ground.top.get(cell) });
                    }
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
                    if matches!(mode, MapMode::Biome | MapMode::TrueColour) {
                        top = sampler.sample(stack, cell);
                    } else {
                        // The annotation, which tints the top in any mode.
                        top.annotation = stack.ground().annotations.get(cell) as u8;
                        top.notes = crate::map_render::note_layers(stack.ground(), cell);
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
    /// The annotations shown on the land: the fingerprint of the map's
    /// colours of annotations, and whether all are shown (else only the
    /// layers in a palette, by their own eyes); `None`: none.
    pub notes: Option<(u64, bool)>,
    /// The caves of the cave layers drawn: those open to the sky seen from
    /// above, all of them from inside (walking, as a ghost).
    pub caves: massif_core::compose::Caves,
    /// The caves under the ground drawn through it (the X-ray).
    pub xray: bool,
    /// The objects of the world's layers stand on the land.
    pub objects: bool,
    /// The land tinted by how many objects its layers place (see
    /// [`object_density`]).
    pub density: bool,
}

/// The colours of the density overlay, from few objects to many (objects
/// per 100 blocks²): the scale its legend shows.
pub const DENSITY_STOPS: [(f32, [u8; 3]); 5] = [(0.0, [52, 84, 170]), (2.0, [40, 170, 190]), (6.0, [120, 200, 80]), (14.0, [240, 196, 60]), (30.0, [232, 72, 56])];

/// The colour of a density on the overlay's scale.
pub fn density_color(v: f32) -> [u8; 3] {
    let stops = DENSITY_STOPS;
    if v <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if v <= b {
            let t = (v - a) / (b - a);
            return [0, 1, 2].map(|k| (ca[k] as f32 + (cb[k] as f32 - ca[k] as f32) * t).round() as u8);
        }
    }
    stops[stops.len() - 1].1
}

/// Cells the density is counted in, blocks a side.
const DENSITY_CELL: i32 = 8;

/// How many objects the world's shown layers place around each column of
/// the `n` × `n` blocks at `(x0, z0)`, as objects per 100 blocks²: counted
/// by cells of [`DENSITY_CELL`] blocks, between cells smoothly.
pub fn object_density(dim: &Dimension, reg: &Registry, seed: i64, lava: bool, x0: i32, z0: i32, n: usize) -> Vec<f32> {
    use massif_core::objects::tile_placements;
    let mut out = vec![0.0; n * n];
    if !reg.object_layers.iter().any(|l| !l.removed && !l.hidden && !l.objects.is_empty()) {
        return out;
    }
    let c = DENSITY_CELL;
    // Cells over the area and one around, for the smoothing.
    let cx0 = x0.div_euclid(c) - 1;
    let cz0 = z0.div_euclid(c) - 1;
    let cells = (n as i32).div_euclid(c) + 3;
    let mut count = vec![0u32; (cells * cells) as usize];
    let area = massif_core::BlockRect::new(cx0 * c, cz0 * c, (cx0 + cells) * c, (cz0 + cells) * c);
    let mat = Materializer::new(reg, seed, dim.min_y);
    for t in area.tiles() {
        let Some(stack) = TileStack::from_above(dim, t, false) else { continue };
        for p in tile_placements(reg, &mat, seed as u64, lava, &stack, t) {
            if !reg.object_layer(p.layer).is_some_and(|l| !l.hidden) {
                continue;
            }
            let (i, k) = (p.x.div_euclid(c) - cx0, p.z.div_euclid(c) - cz0);
            if i >= 0 && k >= 0 && i < cells && k < cells {
                count[(k * cells + i) as usize] += 1;
            }
        }
    }
    let per = 100.0 / (c * c) as f32;
    let at = |i: i32, k: i32| count[(k.clamp(0, cells - 1) * cells + i.clamp(0, cells - 1)) as usize] as f32 * per;
    for j in 0..n {
        for i in 0..n {
            // Between the centres of the four cells around.
            let fx = (x0 + i as i32) as f32 / c as f32 - 0.5 - cx0 as f32;
            let fz = (z0 + j as i32) as f32 / c as f32 - 0.5 - cz0 as f32;
            let (ix, iz) = (fx.floor() as i32, fz.floor() as i32);
            let (tx, tz) = (fx - ix as f32, fz - iz as f32);
            let a = at(ix, iz) * (1.0 - tx) + at(ix + 1, iz) * tx;
            let b = at(ix, iz + 1) * (1.0 - tx) + at(ix + 1, iz + 1) * tx;
            out[j * n + i] = a * (1.0 - tz) + b * tz;
        }
    }
    out
}

/// The colour of each annotation by number and how opaque it is drawn (0
/// to 1), for `notes` (see [`Style::notes`]); empty when hidden.
fn note_colors(reg: &Registry, notes: Option<(u64, bool)>) -> Vec<Option<([u8; 3], f32)>> {
    let Some((_, all)) = notes else { return Vec::new() };
    let p = &reg.annotations;
    (0..=massif_core::annotations::MAX)
        .map(|n| {
            let alpha = p.shown_opacity(n, all) as f32 / 100.0;
            p.color(n).filter(|_| alpha > 0.0).map(|c| (c, alpha))
        })
        .collect()
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
    /// The annotations' colours (see [`note_colors`]).
    notes: Vec<Option<([u8; 3], f32)>>,
    /// The game's colours, in True Colour (see `true_color`).
    truth: Option<std::sync::Arc<crate::true_color::TrueColors>>,
}

impl Colors<'_> {
    fn block(&self, id: BlockId) -> [u8; 3] {
        self.reg.block(id).color
    }

    /// How the column's biome tints the top of `block` against the plains
    /// the blocks' colours are tinted for, channel by channel: grass by the
    /// biome's grass colour, leaves by its leaves', once read from the game
    /// or its mod; `None` for a block the biome does not tint.
    fn biome_tint(&self, block: BlockId, sample: &Sample) -> Option<[f32; 3]> {
        let state = &self.reg.block(block).state;
        let name = state.split('[').next().unwrap_or(state);
        let short = name.rsplit(':').next().unwrap_or(name);
        let biome = self.reg.biome(massif_core::BiomeId(self.auto.resolve(self.reg, sample, self.sea)));
        let (c, plains) = if short.ends_with("grass_block") && !state.contains("snowy=true") {
            (biome.grass?, massif_assets::Tint::Grass.color())
        } else if short.ends_with("leaves") && !["spruce_leaves", "birch_leaves", "cherry_leaves", "pale_oak_leaves"].contains(&short) {
            (biome.foliage?, massif_assets::Tint::Foliage.color())
        } else {
            return None;
        };
        let t: [f32; 3] = std::array::from_fn(|k| (c[k] as f32 / plains[k].max(1) as f32).clamp(0.25, 4.0));
        (t != [1.0; 3]).then_some(t)
    }

    /// Colour of the top face of a span, and of the soil just under it, for
    /// the column `(x, z)` whose ground slopes `slope` degrees, with the
    /// blocks they are. On the column's top, what the cover numbered
    /// `cover` grows there shows over the ground (`wet`: the top is under
    /// water).
    ///
    /// An annotation shows over the top, in its colour, without the
    /// texture of its block.
    #[allow(clippy::too_many_arguments)]
    fn top(&self, terrain: u16, y0: i32, y1: i32, x: i32, z: i32, slope: f32, sample: &Sample, cover: u16, wet: bool) -> Skin {
        let mut skin = self.bare_top(terrain, y0, y1, x, z, slope, sample, cover, wet);
        for n in std::iter::once(sample.annotation).chain(sample.notes) {
            if let Some(Some((c, alpha))) = self.notes.get(n as usize) {
                skin.top = std::array::from_fn(|k| (skin.top[k] as f32 + (c[k] as f32 - skin.top[k] as f32) * alpha).round() as u8);
                skin.top_mat = 0;
            }
        }
        skin
    }

    #[allow(clippy::too_many_arguments)]
    fn bare_top(&self, terrain: u16, y0: i32, y1: i32, x: i32, z: i32, slope: f32, sample: &Sample, cover: u16, wet: bool) -> Skin {
        let flat = |c: [u8; 3]| Skin::flat(c, c);
        match self.mode {
            MapMode::Terrain | MapMode::TrueColour => {
                let span = Span {
                    y0,
                    y1,
                    terrain: TerrainId(terrain),
                    top: y1 as f32,
                };
                let block = self.mat.block(&span, x, y1 - 1, z, false);
                let bare = self.block(block);
                let mut top = bare;
                // Grass and leaves in the colours of the column's biome, as
                // the game tints them (the blocks' colours are the plains');
                // in True Colour, every block as the game and the packs
                // colour it.
                let biome = self.truth.as_ref().map(|_| self.auto.resolve(self.reg, sample, self.sea));
                if let (Some(t), Some(b)) = (&self.truth, biome) {
                    top = t.block_u8(block, b, y1 - 1);
                } else if let Some(t) = self.biome_tint(block, sample) {
                    top = std::array::from_fn(|c| (top[c] as f32 * t[c]).round().clamp(0.0, 255.0) as u8);
                }
                if let Some(def) = self.reg.cover(cover)
                    && !def.hidden
                    && def.underwater == wet
                    && !(wet && self.lava)
                    && (!def.soil_only || massif_core::registry::is_soil(&self.reg.block(block).state))
                    && let Some(g) = def.growth(x, z, self.seed)
                {
                    let truth = |p: &str| self.truth.as_ref().zip(biome).and_then(|(t, b)| t.state(p, b, y1)).map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8));
                    let plant = def.plants.get(g.plant).and_then(|p| truth(&p.0).or_else(|| massif_core::registry::plant_color(&p.0))).unwrap_or(def.color);
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
    /// Per column the meshes draw, where its air lies, packed for the
    /// tracer (see [`air_bits`]); empty: none.
    pub airs: Vec<u32>,
    /// Faces of the columns the tracer leaves out, and the water.
    pub faces: ChunkFaces,
}

/// Where the air of a column the meshes draw lies, for a ghost's view from
/// inside the ground (the tracer comes out of the ground there): from the
/// top of its lowest span (the bottom of the world when it does not stand
/// on it) to the bottom of its highest, in the bits over the height's, which
/// stay 0 for the tracer to go by (10 bits each, from the bottom of the
/// world). A cave, an arch, the room under an overhang; between several
/// caves of a column, its rock counts as air.
fn air_bits(spans: &[CellSpan], min_y: i32) -> u32 {
    let (Some(first), Some(last)) = (spans.first(), spans.last()) else { return 0 };
    let lo = if first.y0 <= min_y { first.y1 } else { min_y };
    let hi = last.y0.max(lo);
    ((lo - min_y).clamp(0, 1023) as u32) << HEIGHT_BITS | ((hi - min_y).clamp(1, 1023) as u32) << 22
}

/// The colour of a cave's floor or ceiling at `y` seen through the ground,
/// from the bottom of the world `min_y` to the land at `top`: warm near the
/// surface, cold deep down, see-through.
fn xray_color(y: i32, min_y: i32, top: i32) -> [u8; 4] {
    let t = ((y - min_y) as f32 / (top - min_y).max(1) as f32).clamp(0.0, 1.0);
    let (deep, high) = ([90.0, 120.0, 255.0], [255.0, 196.0, 90.0]);
    let c = |k: usize| (deep[k] + (high[k] - deep[k]) * t) as u8;
    [c(0), c(1), c(2), 70]
}

/// Build tile `tile`: heights and colours of its columns of plain ground,
/// faces of the others and of its water.
pub fn build_tile(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, tile: TilePos, style: Style) -> TileBuild {
    profiling::scope!("3D build tile");
    build_tile_part(dim, reg, sea, seed, tile, style, None)
}

/// [`build_tile`], or with `every` only the traced columns at the middle of
/// each `every` × `every` square of the tile, without faces: what a coarse
/// cell of the grid keeps of it (see [`build_coarse`]).
fn build_tile_part(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, tile: TilePos, style: Style, every: Option<usize>) -> TileBuild {
    let n = TILE_SIZE as usize;
    let wanted = |i: usize, j: usize| every.is_none_or(|e| i % e == e / 2 && j % e == e / 2);
    let (cx, cz) = (tile.x * TILE_SIZE, tile.z * TILE_SIZE);
    // The tile and a ring of neighbour columns, for walls, and for slopes,
    // measured as the trend of the ground around each column.
    // The water's slopes need two columns more around (what flows over a
    // neighbour, and the heights around that).
    let ring = (if style.mode == MapMode::Slope { SLOPE_REACH } else { 1 }).max(if style.water { 2 } else { 1 });
    // With the density shown, the objects step aside for the land's tint.
    let standing = Standing { reg, seed, lava: style.lava, objects: style.objects && !style.density };
    let density = if style.density { object_density(dim, reg, seed, style.lava, cx, cz, n) } else { Vec::new() };
    let mut grid = compose_area(dim, cx - ring, cz - ring, n + 2 * ring as usize, n + 2 * ring as usize, style.mode, style.water, style.caves, Some(standing));
    // Smooth terrain: the piece of each column, as the export lays it (of
    // its terrain's top block); a piece in place of its top block lowers
    // the column a block first.
    let smoothed = if every.is_none() { smooth_pieces(dim, reg, tile, &mut grid) } else { Vec::new() };
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
        notes: note_colors(reg, style.notes),
        truth: (style.mode == MapMode::TrueColour).then(|| crate::true_color::current(reg)),
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
            if !wanted(i as usize, j as usize) {
                tops.push(Vec::new());
                continue;
            }
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
                        let skin = colors.top(s.terrain, s.y0, s.y1, x, z, slope, &sample, if on_top { cover } else { 0 }, on_top && wet);
                        match density.get(j as usize * n + i as usize) {
                            // The land tinted by its density, its blocks'
                            // pictures left out for the tint to show.
                            // None: the land as it is; a little: a light
                            // tint, then the colour of the scale.
                            Some(&d) if on_top && d > 0.05 => {
                                let heat = density_color(d);
                                let a = (0.35 + d / 4.0).min(0.8);
                                let top = [0, 1, 2].map(|k| (skin.top[k] as f32 * (1.0 - a) + heat[k] as f32 * a).round() as u8);
                                Skin { top, top_mat: 0, ..skin }
                            }
                            _ => skin,
                        }
                    })
                    .collect(),
            );
        }
    }

    // The traced columns: height, colours and the water over them; the
    // air of the others.
    let mut heights = vec![0u16; n * n];
    let mut airs = vec![0u32; if every.is_some() { 0 } else { n * n }];
    let mut column_colors = vec![0u16; n * n];
    let mut depths = vec![0u8; n * n];
    let mut pairs: Vec<Skin> = Vec::new();
    let mut known: HashMap<Skin, u16> = HashMap::new();
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (cx + i as i32, cz + j as i32);
            if every.is_none() && meshed(x, z) {
                airs[j * n + i] = air_bits(grid.spans(x, z), min_y);
            }
            if !wanted(i, j) || !traced(x, z) {
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

    if every.is_some() {
        return TileBuild {
            heights,
            colors: column_colors,
            depths,
            pairs,
            airs,
            faces: ChunkFaces::default(),
        };
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
            // Through the ground: the floors and ceilings of the air under
            // the top run, coloured by depth, see-through.
            if style.xray && spans.len() > 1 {
                let top = spans.last().map_or(max_y, |s| s.y0);
                for w in spans.windows(2) {
                    let (floor, ceiling) = (w[0].y1, w[1].y0);
                    if ceiling > top || ceiling <= floor {
                        continue;
                    }
                    let c = xray_color(floor, min_y, top);
                    out.xray.push(face(i as u8, j as u8, UP, 1, clamp16(floor), 1, c, 0));
                    out.xray.push(face(i as u8, j as u8, DOWN, 1, clamp16(ceiling), 1, [c[0] / 2, c[1] / 2, c[2] / 2, c[3]], 0));
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

    // The pieces of Smooth terrain, as boxes of their height.
    let solid_at = |x: i32, z: i32, y: i32| grid.spans(x, z).iter().any(|s| s.y0 <= y && y < s.y1);
    for &(i, j, y, eighths, color, mat) in &smoothed {
        let (x, z) = (cx + i as i32, cz + j as i32);
        let c = rgba(color, 255);
        out.solid.push(face(i as u8, j as u8, part(UP, eighths), 1, clamp16(y), 1, c, mat));
        for (dir, (dx, dz)) in [(EAST, (1, 0)), (WEST, (-1, 0)), (SOUTH, (0, 1)), (NORTH, (0, -1))] {
            if solid_at(x + dx, z + dz, y) {
                continue;
            }
            let (fx, fz) = (i as i32 + dx.max(0), j as i32 + dz.max(0));
            out.solid.push(face(fx as u8, fz as u8, part(dir, eighths), 1, clamp16(y), 1, c, mat));
        }
    }

    // Water, as the game draws it. Each column's water is a run of blocks
    // from its ground up. Where it stands over a lower neighbour (a step
    // of a river, a fall, open land) the game pours flowing water over that
    // neighbour: a block over lower water (flowing water over water spreads
    // no further), down to the water or the ground from higher still. The
    // top of each run slopes as the game's does: each corner at the mean
    // of the heights of the four blocks around it (a source 8/9 of a block,
    // flowing water less, a block with water over it a whole block), the
    // water's own blocks weighing ten times more; its sides go up to it.
    if style.water {
        let blue = if style.lava { [238, 112, 22, 255] } else { [26, 92, 150, 205] };
        // Under a frost cover, the top of the water is ice; under a lava
        // cover, the water is lava.
        let (ice, lava) = ([168, 200, 240, 255], [238, 112, 22, 255]);
        let frozen = |c: usize| match reg.cover(grid.cover[c]) {
            Some(d) if !style.lava && !d.hidden && d.frost => 1u8,
            Some(d) if !style.lava && !d.hidden && d.lava => 2,
            _ => 0,
        };
        let (gw, gd) = (grid.w, grid.d);
        let cell = |x: i32, z: i32| -> Option<usize> {
            let (i, j) = (x - grid.x0, z - grid.z0);
            (i >= 0 && j >= 0 && (i as usize) < gw && (j as usize) < gd).then(|| j as usize * gw + i as usize)
        };
        // The first air over each column's ground (MIN: no map there), and
        // the top block of its water (MIN: dry).
        let ground: Vec<i32> = (0..gw * gd).map(|c| grid.spans(grid.x0 + (c % gw) as i32, grid.z0 + (c / gw) as i32).last().map_or(i32::MIN, |s| s.y1)).collect();
        let top: Vec<i32> = grid.water.iter().map(|&w| if w == i32::MIN { i32::MIN } else { w - 1 }).collect();
        let kind: Vec<u8> = (0..gw * gd).map(|c| if top[c] == i32::MIN { 0 } else { frozen(c) }).collect();
        // Each column's fluid: from its ground up to `hi` (MIN: none),
        // flowing at the top or not.
        let mut fluid = vec![(i32::MIN, false); gw * gd];
        for c in 0..gw * gd {
            if ground[c] == i32::MIN {
                continue;
            }
            let (x, z) = (grid.x0 + (c % gw) as i32, grid.z0 + (c / gw) as i32);
            // The first block free over it, and the highest water beside it
            // (not ice: it does not flow).
            let free = if top[c] == i32::MIN { ground[c] } else { top[c] + 1 };
            let beside = [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().filter_map(|(dx, dz)| cell(x + dx, z + dz)).filter(|&m| kind[m] != 1).map(|m| top[m]).max().unwrap_or(i32::MIN);
            fluid[c] = if beside != i32::MIN && beside >= free && kind[c] == 0 { (beside, true) } else { (top[c], false) };
        }
        // The heights the game gives the blocks, as fractions of a block:
        // a source's, flowing water's (lava loses two levels a block).
        let (source, flowing) = (8.0 / 9.0, if style.lava { 6.0 / 9.0 } else { 7.0 / 9.0 });
        // The height of the block at `y` of column `m`, as the game reads
        // it: of its fluid (a whole block under more), -1 for solid (or
        // past the map), 0 for air.
        let height = |m: Option<usize>, y: i32| -> f32 {
            let Some(m) = m else { return -1.0 };
            let (hi, flows) = fluid[m];
            if hi != i32::MIN && y >= ground[m] && y <= hi {
                if kind[m] == 1 && y == hi {
                    -1.0
                } else if y < hi {
                    1.0
                } else if flows {
                    flowing
                } else {
                    source
                }
            } else if ground[m] == i32::MIN || y < ground[m] {
                -1.0
            } else {
                0.0
            }
        };
        // How far under the top of its block each corner of the top of a
        // column's water lies, in 16ths: north-west, north-east, south-west,
        // south-east, four bits each.
        let corners = |x: i32, z: i32, c: usize| -> u16 {
            let (hi, flows) = fluid[c];
            if kind[c] != 0 {
                return 0x2222;
            }
            let own = if flows { flowing } else { source };
            let mut packed = 0u16;
            for (k, (dx, dz)) in [(-1, -1), (1, -1), (-1, 1), (1, 1)].into_iter().enumerate() {
                let (a, b) = (height(cell(x + dx, z), hi), height(cell(x, z + dz), hi));
                let h = if a >= 1.0 || b >= 1.0 {
                    1.0
                } else {
                    let (mut sum, mut weight) = (0.0f32, 0.0f32);
                    let mut add = |h: f32| {
                        if h >= 0.8 {
                            sum += 10.0 * h;
                            weight += 10.0;
                        } else if h >= 0.0 {
                            sum += h;
                            weight += 1.0;
                        }
                    };
                    let mut whole = false;
                    if a > 0.0 || b > 0.0 {
                        let d = height(cell(x + dx, z + dz), hi);
                        whole = d >= 1.0;
                        add(d);
                    }
                    add(own);
                    add(a);
                    add(b);
                    if whole { 1.0 } else { sum / weight.max(1.0) }
                };
                packed |= (((1.0 - h) * 16.0).round().clamp(0.0, 15.0) as u16) << (4 * k);
            }
            packed
        };
        let water_face = |x: u8, z: u8, dir: u8, len: u8, y: i32, h: i32, color: [u8; 4], drops: u16| Face {
            x,
            z,
            dir,
            len,
            y: clamp16(y),
            h: clamp16(h),
            color,
            mat: drops,
            pad: 0,
        };
        let colour = |k: u8| [blue, ice, lava][k as usize];
        // The tops: merged into rectangles where flat (the same height at
        // every corner), one by one where they slope.
        let tops: Vec<Option<(i32, u8, u16)>> = (0..n * n)
            .map(|c| {
                let (x, z) = (cx + (c % n) as i32, cz + (c / n) as i32);
                let g = cell(x, z)?;
                let (hi, _) = fluid[g];
                (hi != i32::MIN && hi >= ground[g]).then(|| (hi, kind[g], corners(x, z, g)))
            })
            .collect();
        let flat = |d: u16| d == (d & 15) * 0x1111;
        greedy(n, |i, j| tops[j * n + i].filter(|t| flat(t.2)), |i, j, w, h, (hi, k, d)| out.water.push(water_face(i as u8, j as u8, UP, w as u8, hi + 1, h as i32, colour(k), d)));
        for (c, t) in tops.iter().enumerate() {
            if let Some((hi, k, d)) = t.filter(|t| !flat(t.2)) {
                out.water.push(water_face((c % n) as u8, (c / n) as u8, UP, 1, hi + 1, 1, colour(k), d));
            }
        }
        // The sides, wherever the water stands over what is beside it: air
        // over lower water or ground, or the end of the map.
        for j in 0..n {
            for i in 0..n {
                let Some((hi, k, d)) = tops[j * n + i] else { continue };
                let (x, z) = (cx + i as i32, cz + j as i32);
                let Some(g) = cell(x, z) else { continue };
                let lo = ground[g];
                for (dir, (dx, dz)) in [(EAST, (1, 0)), (WEST, (-1, 0)), (SOUTH, (0, 1)), (NORTH, (0, -1))] {
                    let Some(m) = cell(x + dx, z + dz) else { continue };
                    let free = match fluid[m] {
                        _ if ground[m] == i32::MIN => lo,
                        (h, _) if h != i32::MIN => h + 1,
                        _ => ground[m],
                    };
                    let bottom = free.max(lo);
                    if bottom > hi {
                        continue;
                    }
                    let (px, pz) = (i as i32 + dx.max(0), j as i32 + dz.max(0));
                    out.water.push(water_face(px as u8, pz as u8, dir, 1, bottom, hi + 1 - bottom, colour(k), d));
                }
            }
        }
    }
    TileBuild {
        heights,
        colors: column_colors,
        depths,
        pairs,
        airs,
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
    /// A colour a build does not have (it never should) is the list's
    /// first, rather than a panic in the middle of a frame.
    pub fn pack(&mut self, b: &TileBuild) -> Vec<u32> {
        let map: Vec<u32> = b.pairs.iter().map(|&p| self.get(p) as u32).collect();
        b.heights
            .iter()
            .zip(&b.colors)
            .enumerate()
            .map(|(k, (&h, &c))| if h == 0 { b.airs.get(k).copied().unwrap_or(0) } else { h as u32 | map.get(c as usize).copied().unwrap_or(0) << HEIGHT_BITS | (b.depths.get(k).copied().unwrap_or(0) as u32) << 24 })
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
    /// Toward the sun; w: 1 when water reflects (Settings ▸ Graphics).
    pub sun: [f32; 4],
    /// The sky overhead, and at the horizon.
    pub sky: [f32; 4],
    pub horizon: [f32; 4],
    /// Fog start, fog end, ambient weight, sun weight.
    pub fog: [f32; 4],
    /// Colour of the haze far away, and how much of it at most.
    pub mist: [f32; 4],
    /// The camera's forward, right and up directions, for the tracer's rays;
    /// `cam_f`'s w: 1 when the eyes are in the ground in 3D Sculpt (a
    /// ghost's), which is then seen through.
    pub cam_f: [f32; 4],
    pub cam_s: [f32; 4],
    pub cam_u: [f32; 4],
    /// tan(fov / 2) × aspect, tan(fov / 2), radians per pixel, the
    /// frame's origin's z.
    pub cam_k: [f32; 4],
    /// x, the game's textures (in the player's view, and in the views' own
    /// look with Settings ▸ Graphics); y, the game's light, sky and fog (the
    /// player's view); z, the height of its clouds (0: none); w, the number
    /// of materials the atlas knows.
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

/// What the shaders draw: the views' own look in flat colours or with the
/// game's textures (Settings ▸ Graphics), or the game's (the player's
/// view). Each is a shader of its own (see [`Renderer3d::with_samples`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Flat,
    Textured,
    Game,
}

impl Look {
    const ALL: [Look; 3] = [Look::Flat, Look::Textured, Look::Game];

    /// The look `globals` asks for.
    fn of(globals: &Globals) -> Look {
        if globals.look[1] > 0.5 {
            Look::Game
        } else if globals.look[0] > 0.5 {
            Look::Textured
        } else {
            Look::Flat
        }
    }
}

/// Floats in [`Globals`]: what tells a frame from the last.
const GLOBAL_FLOATS: usize = size_of::<Globals>() / 4;

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
    /// Columns x0, z0, x1, z1 of the grid where the near grid draws every
    /// block (see [`Renderer3d::begin_near`]): empty for this one. None
    /// when x1 ≤ x0.
    hole: [i32; 4],
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
    return shade_lit(to_linear(base), n, d, 1.0);
}

/// The same for a colour in linear light (a texture's), `ao` the light
/// its corners leave it (1: all).
fn shade_lit(c: vec3<f32>, n: vec3<f32>, d: f32, ao: f32) -> vec3<f32> {
    let sun = max(dot(n, g.sun.xyz), 0.0);
    let sky = 0.72 + 0.28 * n.y;
    let lit = c * (g.fog.z * sky + g.fog.w * sun) * ao;
    let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
    return mix(lit, to_linear(g.mist.rgb), f * f * g.mist.w);
}

/// How light the block at `world` (on a face turned `n`, `d` blocks away)
/// is: a little lighter or darker than its neighbours, and darker along
/// its edges up close, so that a wall reads as blocks, as in the game.
fn block_tint(world: vec3<f32>, n: vec3<f32>, d: f32) -> f32 {
    let bi = vec3<i32>(floor(world - n * 0.01)) + vec3<i32>(i32(g.eye.w), 0, i32(g.cam_k.w));
    var h = u32(bi.x) * 73856093u ^ u32(bi.y) * 19349663u ^ u32(bi.z) * 83492791u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    // Blocks under a pixel: no block-to-block jitter, it would sparkle.
    let fine = 1.0 - smoothstep(0.3, 1.0, d * g.cam_k.z);
    let jitter = (f32((h >> 8u) & 255u) / 255.0 - 0.5) * fine;
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
// The tile of every chunk (x, z), by its slot: each face carries its own.
@group(1) @binding(0) var chunk_tiles: texture_2d<i32>;

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
    // A slab's faces: half a block high.
    // A piece of Smooth terrain: its height in eighths of a block.
    let eighths = (f.xz_dir_len.z >> 3u) & 7u;
    let part = f32(eighths) / 8.0;
    let half = eighths != 0u;
    let dir = f.xz_dir_len.z & 7u;
    let x = f32(f.xz_dir_len.x);
    let z = f32(f.xz_dir_len.y);
    let len = f32(f.xz_dir_len.w);
    let y = f32(f.y_h.x) + select(0.0, part, half && dir == 0u);
    let h = f32(f.y_h.y) * select(1.0, part, half && dir >= 2u);
    let slot = i32(f.mat.y);
    let tile = textureLoad(chunk_tiles, vec2<i32>(slot % 256, slot / 256), 0).xy;
    // Where the chunk is from the frame's origin, in whole blocks.
    let corner = vec3<f32>(f32(tile.x * 128 - i32(g.eye.w)), 0.0, f32(tile.y * 128 - i32(g.cam_k.w)));
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
    // Water: the top of its blocks where the game puts it, each corner so
    // far under the top of its block (`mat`, in 16ths: north-west,
    // north-east, south-west, south-east), and its sides up to it.
    if (WATER) {
        var k = 0u;
        var lift = false;
        switch dir {
            case 0u: {
                k = 2u * u32(u) + u32(v);
                lift = true;
            }
            case 2u: {
                k = 1u + 2u * u32(v);
                lift = u > 0.5;
            }
            case 3u: {
                k = 2u * u32(u);
                lift = v > 0.5;
            }
            case 4u: {
                k = 2u + u32(u);
                lift = v > 0.5;
            }
            default: {
                k = u32(v);
                lift = u > 0.5;
            }
        }
        if (lift) {
            p.y = p.y - f32((f.mat.x >> (4u * k)) & 15u) / 16.0;
        }
    }
    let world = p + corner;
    var out: VertexOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.color = f.color;
    out.normal = n;
    out.world = world;
    out.water = select(0.0, 1.0, WATER);
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
        if (n.y > 0.5) {
            base *= top_tint(in.mat, in.color.rgb);
        }
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
    // Blocks across a pixel, here (before any branch: derivatives need
    // every pixel of the quad).
    let fw = max(length(dpdx(in.world.xz)), length(dpdy(in.world.xz)));
    if (GAME_LOOK) {
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
        // Each wave fades out where a pixel spans more than half of it:
        // finer than the screen, waves only make moiré (rings far off).
        var nw = n;
        var calm = 1.0;
        let p = in.world.xz;
        if (n.y > 0.5) {
            let w0 = 1.0 - smoothstep(0.6, 1.6, 0.8 * fw);
            let w1 = 0.6 * (1.0 - smoothstep(0.6, 1.6, 2.3 * fw));
            let w2 = 0.35 * (1.0 - smoothstep(0.6, 1.6, 4.6 * fw));
            let rx = w0 * sin(p.x * 0.73 + p.y * 0.31) + w1 * sin(p.x * 1.91 - p.y * 1.27) + w2 * sin(p.x * 3.7 + p.y * 2.9);
            let rz = w0 * cos(p.y * 0.67 - p.x * 0.29) + w1 * cos(p.y * 2.03 + p.x * 1.11) + w2 * cos(p.y * 3.3 - p.x * 3.1);
            nw = normalize(vec3<f32>(rx * 0.045, 1.0, rz * 0.045));
            calm = w0;
        }
        var fres = pow(1.0 - max(dot(nw, v), 0.0), 3.0);
        var c = to_linear(in.color.rgb) * (g.fog.z + 0.3 * g.fog.w);
        // sun.w: reflections (Settings ▸ Graphics).
        if (g.sun.w > 0.5) {
            let sky = sky_color(reflect(-v, nw));
            let h = normalize(v + g.sun.xyz);
            // The sun's glint, as sharp as a pixel allows: far off, a
            // broad sheen rather than sparks that come and go.
            let sharp = mix(60.0, 900.0, calm);
            let glint = pow(max(dot(nw, h), 0.0), sharp) * mix(0.25, 1.1, calm) * max(n.y, 0.0);
            c = mix(c, sky, 0.1 + 0.7 * fres) + vec3<f32>(glint);
        } else {
            fres = 0.0;
        }
        let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
        c = mix(c, to_linear(g.mist.rgb), f * f * g.mist.w);
        return vec4<f32>(c, mix(in.color.a, 1.0, 0.55 * fres));
    }
    if (TEXTURED) {
        // The views' own light on the game's textures (Settings ▸
        // Graphics): the block's texture, tinted on a top by what grows
        // there.
        let layer = face_layer(in.mat, n, in.world.y);
        if (layer != 0u) {
            let view = normalize(in.world - g.eye.xyz);
            var base = texel(layer, face_uv(in.world, n), tex_lod(d, abs(dot(n, view)))).rgb;
            if (n.y > 0.5) {
                base *= top_tint(in.mat, in.color.rgb);
            }
            return vec4<f32>(shade_lit(base, n, d, 1.0), in.color.a);
        }
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
    // Columns [x0, x1) × [z0, z1) the near grid draws: empty here.
    hole: vec4<i32>,
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
// The columns a ray from eyes in the ground crosses at most before air:
// past them, the ground is too deep to see through.
const OUT_STEPS: i32 = 512;

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
    // A square wholly in the near grid's room is empty for this grid: the
    // ray goes over it at once, the near grid drawing what lies there.
    if (terrain.hole.z > terrain.hole.x) {
        let a = c << vec2<u32>(u32(level));
        let b = a + vec2<i32>(1 << u32(level));
        if (all(a >= terrain.hole.xy) && all(b <= terrain.hole.zw)) {
            return 0u;
        }
    }
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

/// Whether one of the four columns a square far off is coloured by (see
/// `shade`) is built.
fn square_seen(level: i32, cell: vec2<i32>) -> bool {
    let size = f32(1 << u32(level));
    let corner = vec2<f32>(cell) * size;
    for (var k = 0; k < 4; k++) {
        let q = corner + size * vec2<f32>(0.25 + 0.5 * f32(k & 1), 0.25 + 0.5 * f32(k >> 1u));
        if ((column(vec2<i32>(floor(q))) & 0xfffu) != 0u) {
            return true;
        }
    }
    return false;
}

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
        // A square smaller than a pixel is coloured by four of its columns
        // (see `shade`): while none of them is built yet, a finer square
        // that is, rather than a square of no colour (it was black).
        if (level == 0 || (size <= t * pix && square_seen(level, cell))) {
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

/// The heights between which column `c` is air: over a traced column's
/// top, between the lowest and the highest span of one the meshes draw
/// (packed with it), everywhere where there is nothing.
fn air_of(c: vec2<i32>) -> vec2<f32> {
    let col = column(c);
    let bottom = terrain.origin.z;
    if (col == 0u) {
        return vec2<f32>(-1.0e9, 1.0e9);
    }
    let h = col & 0xfffu;
    if (h != 0u) {
        return vec2<f32>(bottom + f32(h), 1.0e9);
    }
    return vec2<f32>(bottom + f32((col >> 12u) & 0x3ffu), bottom + f32(col >> 22u));
}

/// Where the ray `o + t d` (grid coordinates), from `t_in` in the ground,
/// first comes out into air (see `air_of`), and the side it came in by;
/// x: -1 when it is still in the ground at `t_out`, or far on.
fn come_out(o: vec3<f32>, d: vec3<f32>, t_in: f32, t_out: f32) -> vec2<f32> {
    let step = vec2<i32>(select(-1, 1, d.x > 0.0), select(-1, 1, d.z > 0.0));
    let inv = 1.0 / d.xz;
    var cell = vec2<i32>(floor(o.xz + d.xz * t_in));
    var t = t_in;
    var axis = 2.0;
    for (var i = 0; i < OUT_STEPS; i++) {
        let bound = vec2<f32>(cell) + select(vec2<f32>(0.0), vec2<f32>(1.0), step > vec2<i32>(0));
        let tb = (bound - o.xz) * inv;
        let t_exit = min(min(tb.x, tb.y), t_out);
        // The part of the way through the cell where the ray is in air.
        let air = air_of(cell);
        let ta = (air.x - o.y) / d.y;
        let tz = (air.y - o.y) / d.y;
        let a = max(t, min(ta, tz));
        let b = min(t_exit, max(ta, tz));
        if (a < b) {
            return vec2<f32>(a, axis);
        }
        if (t_exit >= t_out) {
            break;
        }
        if (tb.x < tb.y) {
            cell.x += step.x;
            axis = 0.0;
        } else {
            cell.y += step.y;
            axis = 1.0;
        }
        t = t_exit;
    }
    return vec2<f32>(-1.0, 0.0);
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
    if (GAME_LOOK) {
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
fn column_light(cell: vec2<i32>, p: vec3<f32>, n: vec3<f32>, height: f32, t: f32) -> f32 {
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
    // A block about a pixel wide: the mean of its corners, rather than
    // shadows finer than a pixel that ripple into moiré.
    let q = mix(uv, vec2<f32>(0.5), smoothstep(0.3, 1.0, t * g.cam_k.z));
    let light = mix(mix(c00, c10, q.x), mix(c01, c11, q.x), q.y);
    return 0.55 + 0.15 * light;
}

/// How what grows on the top of pair `pair` tints its block's texture: the
/// top's colour over the block's own, in linear light.
fn cover_tint(pair: i32) -> vec3<f32> {
    let top = to_linear(textureLoad(palette, vec2<i32>(pair, 0), 0).rgb);
    let bare = to_linear(textureLoad(palette, vec2<i32>(pair, 2), 0).rgb);
    return clamp(top / max(bare, vec3<f32>(0.004)), vec3<f32>(0.25), vec3<f32>(4.0));
}

/// The colour (linear) of point `p` (`world`, from the frame's origin) of a
/// face of the packed column `col` turned `n`, `t` blocks away: its block's
/// texture (the top block's at the top, the soil's under it, then the
/// rock), tinted on the top by what grows there; without the game's
/// textures, its colour, block by block.
fn column_base(col: u32, p: vec3<f32>, world: vec3<f32>, n: vec3<f32>, t: f32, view_d: vec3<f32>) -> vec3<f32> {
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
    if (layer == 0u) {
        return to_linear(surface_color(col, p.y, up)) * block_tint(world, n, t);
    }
    var base = texel(layer, face_uv(world, n), tex_lod(t, abs(dot(n, view_d)))).rgb;
    if (up) {
        base *= cover_tint(pair);
    }
    return base;
}

/// The top of the packed column `col` seen from afar (sRGB): with the
/// game's textures, the mean of its block's (the texture's last level of
/// detail, what it fades to up close), tinted; else its colour.
fn far_top(col: u32) -> vec3<f32> {
    let pair = i32((col >> 12u) & 0xfffu);
    let top = textureLoad(palette, vec2<i32>(pair, 0), 0).rgb;
    if (!TEXTURED) {
        return top;
    }
    let layer = face_layer(textureLoad(pair_mats, vec2<i32>(pair, 0), 0).x, vec3<f32>(0.0, 1.0, 0.0), 0.0);
    if (layer == 0u) {
        return top;
    }
    return pow(texel(layer, vec2<f32>(0.5), 4.0).rgb * cover_tint(pair), vec3<f32>(1.0 / 2.2));
}

/// The side of the packed column `col` at height `y` seen from afar (sRGB):
/// its top block, its soil, then the rock down to the bottom (as the side
/// up close), with the game's textures the mean of theirs.
fn far_side(col: u32, y: f32, n: vec3<f32>) -> vec3<f32> {
    let flat = surface_color(col, y, false);
    if (!TEXTURED) {
        return flat;
    }
    let pair = i32((col >> 12u) & 0xfffu);
    let height = terrain.origin.z + f32(col & 0xfffu);
    let m = textureLoad(pair_mats, vec2<i32>(pair, 0), 0);
    var mat = ROCK;
    if (height - y <= 1.0) {
        mat = m.x;
    } else if (height - y <= 3.0) {
        mat = m.y;
    }
    let layer = face_layer(mat, n, y);
    if (layer == 0u) {
        return flat;
    }
    return pow(texel(layer, vec2<f32>(0.5), 4.0).rgb, vec3<f32>(1.0 / 2.2));
}

/// A traced column's face in the views' own light with the game's
/// textures (Settings ▸ Graphics): the texture of its block, darker in the
/// corners that blocks close, as the game's smooth light has it; `lit`,
/// the normal it is lit by.
fn view_column(cell: vec2<i32>, col: u32, p: vec3<f32>, world: vec3<f32>, n: vec3<f32>, lit: vec3<f32>, t: f32, view_d: vec3<f32>) -> vec3<f32> {
    let height = terrain.origin.z + f32(col & 0xfffu);
    return shade_lit(column_base(col, p, world, n, t, view_d), lit, t, column_light(cell, p, n, height, t));
}

/// A traced column's face in the game's look: its block's texture (the top
/// block's at the top, the soil's under it, then the rock), tinted by what
/// grows on its top, in the game's light, marked.
fn game_column(cell: vec2<i32>, p: vec3<f32>, world: vec3<f32>, n: vec3<f32>, t: f32, view_d: vec3<f32>) -> vec3<f32> {
    let col = column(cell);
    let height = terrain.origin.z + f32(col & 0xfffu);
    let base = column_base(col, p, world, n, t, view_d);
    // Under water the light fades a level a block, as in the game.
    var light = column_light(cell, p, n, height, t);
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
    var start = t0;
    if (g.cam_f.w > 0.5) {
        // The eyes in the ground (in 3D Sculpt, a ghost's): it is seen
        // through, up to where the ray comes out into air; deeper, the dark
        // of the ground.
        let out_at = come_out(o, d, t0, t1);
        if (out_at.x < 0.0) {
            out.color = vec4<f32>(background(view_d) * 0.25, 1.0);
            out.depth = 1.0;
            return out;
        }
        start = out_at.x;
        axis = i32(out_at.y);
    }
    let hit = trace(o, d, start, t1, axis, g.cam_k.z / stride);
    if (!hit.found) {
        out.color = vec4<f32>(background(view_d), 1.0);
        out.depth = 1.0;
        return out;
    }
    let p = o + d * hit.t;
    var base: vec3<f32>;
    var normal = hit.normal;
    // A block about a pixel wide: its side is a sliver of the pixel, the
    // rest the tops around it. Faded into the top and the hillside as it
    // narrows, the steps of a slope do not break into stair-stepped
    // stripes, and meet the smooth ground seen from further away without a
    // seam.
    var sliver = 0.0;
    var hill = normal;
    if (hit.level == 0) {
        base = surface_color(column(hit.cell), p.y, hit.normal.y > 0.5);
        if (hit.normal.y < 0.5) {
            // How many pixels high the side is: its drop to the column the
            // ray came from (the bottom of the world off the map).
            let height = terrain.origin.z + f32(column(hit.cell) & 0xfffu);
            let beside = terrain.origin.z + f32(column(hit.cell + vec2<i32>(i32(hit.normal.x), i32(hit.normal.z))) & 0xfffu);
            let tall = smoothstep(1.0, 2.5, (height - beside) / max(hit.t * g.cam_k.z, 1.0e-4));
            sliver = smoothstep(0.35, 1.0, hit.t * g.cam_k.z / stride) * (1.0 - tall);
        }
        if (sliver > 0.0) {
            let mid = vec2<f32>(hit.cell) + vec2<f32>(0.5);
            let hl = ground_height(mid - vec2<f32>(1.0, 0.0));
            let hr = ground_height(mid + vec2<f32>(1.0, 0.0));
            let hu = ground_height(mid - vec2<f32>(0.0, 1.0));
            let hd = ground_height(mid + vec2<f32>(0.0, 1.0));
            hill = normalize(vec3<f32>(-(hr - hl) / (2.0 * stride), 1.0, -(hd - hu) / (2.0 * stride)));
        }
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
            if ((col & 0xfffu) != 0u) {
                sum += far_top(col);
                count += 1.0;
            }
        }
        base = select(textureLoad(palette, vec2<i32>(0, 0), 0).rgb, sum / max(count, 1.0), count > 0.0);
        if (hit.normal.y < 0.5) {
            // Into its side (the map's sides, a cliff seen from afar): the
            // section through the column there, not the colour of tops.
            let inside = p.xz - vec2<f32>(hit.normal.x, hit.normal.z) * 0.5;
            let col = column(vec2<i32>(floor(inside)));
            if ((col & 0xfffu) != 0u) {
                base = far_side(col, p.y, hit.normal);
            }
        }
        let mid = corner + vec2<f32>(size * 0.5);
        let hl = ground_height(mid - vec2<f32>(size, 0.0));
        let hr = ground_height(mid + vec2<f32>(size, 0.0));
        let hu = ground_height(mid - vec2<f32>(0.0, size));
        let hd = ground_height(mid + vec2<f32>(0.0, size));
        if (hit.normal.y > 0.5) {
            normal = normalize(vec3<f32>(-(hr - hl) / (2.0 * size * stride), 1.0, -(hd - hu) / (2.0 * size * stride)));
        }
    }
    let world = vec3<f32>(p.x * stride + terrain.origin.x, p.y, p.z * stride + terrain.origin.y);
    if (GAME_LOOK) {
        var c: vec3<f32>;
        if (hit.level == 0 && stride == 1.0) {
            c = game_column(hit.cell, p, world, normal, hit.t, view_d);
            if (sliver > 0.0) {
                let top = terrain.origin.z + f32(column(hit.cell) & 0xfffu);
                let up = vec3<f32>(0.0, 1.0, 0.0);
                c = mix(c, game_column(hit.cell, vec3<f32>(p.x, top, p.z), vec3<f32>(world.x, top, world.z), up, hit.t, view_d), sliver);
            }
        } else if (hit.level == 0) {
            c = game_light(to_linear(mix(base, surface_color(column(hit.cell), p.y, true), sliver)), normalize(mix(normal, hill, sliver)), 1.0, hit.t);
        } else {
            c = game_light(to_linear(base), normal, 1.0, hit.t);
        }
        let cl = game_clouds(g.eye.xyz, view_d, hit.t);
        out.color = vec4<f32>(mix(c, cl.rgb, cl.a), 1.0);
    } else if (TEXTURED && hit.level == 0) {
        let col = column(hit.cell);
        var c = view_column(hit.cell, col, p, world, normal, normal, hit.t, view_d);
        if (sliver > 0.0) {
            // A side narrower than a pixel: into the top, lit as the hillside.
            let top = terrain.origin.z + f32(col & 0xfffu);
            let up = vec3<f32>(0.0, 1.0, 0.0);
            c = mix(c, view_column(hit.cell, col, vec3<f32>(p.x, top, p.z), vec3<f32>(world.x, top, world.z), up, hill, hit.t, view_d), sliver);
        }
        out.color = vec4<f32>(c, 1.0);
    } else {
        var tint = 1.0;
        if (hit.level == 0 && stride == 1.0) {
            tint = block_tint(world, normal, hit.t);
        }
        if (sliver > 0.0) {
            base = mix(base, surface_color(column(hit.cell), p.y, true), sliver);
            normal = normalize(mix(normal, hill, sliver));
        }
        out.color = vec4<f32>(shade(base, normal, hit.t) * tint, 1.0);
    }
    let clip = g.view_proj * vec4<f32>(world, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
"#;

/// Where a chunk's faces of one kind lie: a page of an arena, the first
/// face and how many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    page: u32,
    start: u32,
    count: u32,
}

/// The GPU side of one chunk: its slot (see [`Renderer3d::upload`]) and its
/// solid faces and water.
struct GpuChunk {
    slot: u16,
    solid: Option<Range>,
    water: Option<Range>,
    xray: Option<Range>,
}

/// One vertex buffer holding the faces of many chunks, each a range of it:
/// chunks side by side in it are drawn in one call, where a buffer a chunk
/// cost a call each (thousands on a large map with much water or many
/// trees). Ranges freed are taken again.
struct Arena {
    buffer: wgpu::Buffer,
    /// Faces it holds at most.
    capacity: u32,
    free: FreeRanges,
}

/// The free ranges of an arena: (first face, count), in order, never
/// touching.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FreeRanges(Vec<(u32, u32)>);

impl FreeRanges {
    /// The first face of a free range of `count`, taken (first fit).
    fn take(&mut self, count: u32) -> Option<u32> {
        let k = self.0.iter().position(|&(_, n)| n >= count)?;
        let (start, n) = self.0[k];
        if n == count {
            self.0.remove(k);
        } else {
            self.0[k] = (start + count, n - count);
        }
        Some(start)
    }

    /// Give `count` faces from `start` back, joined to the free ranges
    /// beside them.
    fn give(&mut self, start: u32, count: u32) {
        let k = self.0.partition_point(|&(s, _)| s < start);
        self.0.insert(k, (start, count));
        if k + 1 < self.0.len() && self.0[k].0 + self.0[k].1 == self.0[k + 1].0 {
            self.0[k].1 += self.0[k + 1].1;
            self.0.remove(k + 1);
        }
        if k > 0 && self.0[k - 1].0 + self.0[k - 1].1 == self.0[k].0 {
            self.0[k - 1].1 += self.0[k].1;
            self.0.remove(k);
        }
    }
}

struct Target {
    size: [u32; 2],
    /// Where the samples are drawn before they are resolved (none at one
    /// sample a pixel: drawn straight into the image).
    msaa: Option<wgpu::TextureView>,
    #[cfg_attr(not(test), allow(dead_code))]
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same image as plain bytes, for egui.
    shown: crate::view_image::Shown,
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

/// The game's textures as made on the graphics card (see
/// [`Renderer3d::make_atlas`]).
struct MadeAtlas {
    atlas: wgpu::TextureView,
    materials: wgpu::TextureView,
    colors: wgpu::TextureView,
    /// Materials, and bytes of memory taken.
    count: u32,
    bytes: u64,
}

/// GPU resources of the 3D preview. Independent of egui, so it can render
/// headlessly in tests.
pub struct Renderer3d {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Samples a pixel: 4 with edges smoothed, else 1.
    samples: u32,
    /// Where the device cannot view the image as plain bytes (OpenGL):
    /// the pass that copies it so.
    copier: Option<crate::view_image::Copier>,
    /// In each [`Look`].
    opaque_pipeline: [wgpu::RenderPipeline; 3],
    water_pipeline: [wgpu::RenderPipeline; 3],
    /// The caves through the ground: see-through, over everything.
    xray_pipeline: [wgpu::RenderPipeline; 3],
    trace_pipeline: [wgpu::RenderPipeline; 3],
    /// The tracer once a sample: a ray through each of a pixel's samples
    /// rather than one through its middle, which smooths the edges of the
    /// traced ground as the samples do the meshes'. Built with 4 samples.
    trace_fine_pipeline: Option<[wgpu::RenderPipeline; 3]>,
    /// The same over what is drawn already, nearer only: the near grid.
    trace_near_pipeline: [wgpu::RenderPipeline; 3],
    trace_fine_near_pipeline: Option<[wgpu::RenderPipeline; 3]>,
    /// The next frames trace a ray a sample (when it was built): a still
    /// view. Moving, one a pixel is enough, and four times cheaper.
    pub fine: bool,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    chunk_layout: wgpu::BindGroupLayout,
    trace_layout: wgpu::BindGroupLayout,
    /// The tile of every chunk's slot (two integers a slot, `SLOT_ROW` a
    /// row), and its copy here; the slots free.
    chunk_tiles: wgpu::Texture,
    chunk_group: wgpu::BindGroup,
    slot_tiles: Vec<[i32; 2]>,
    free_slots: Vec<u16>,
    /// The faces of every chunk: solid, then water.
    arenas: [Vec<Arena>; 3],
    terrain: wgpu::Buffer,
    palette: wgpu::Texture,
    /// The blocks of every colour pair (see [`Skin`]).
    pair_mats: wgpu::Texture,
    /// The game's look: its textures, the materials' layers, its clouds.
    look_layout: wgpu::BindGroupLayout,
    look_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    clouds: wgpu::TextureView,
    /// Materials the atlas knows, and the layers of water and lava.
    material_count: u32,
    /// What the graphics card said when the shaders or pipelines were
    /// made, if anything went wrong: every frame is then drawn in the flat
    /// look, which has the least in it, rather than not at all.
    pub looks_failed: Option<String>,
    /// Bytes of the graphics card's memory the atlas takes.
    look_bytes: u64,
    fluid_layers: [u16; 2],
    tracer: Option<Tracer>,
    /// On a map drawn with a column every few blocks, every block around
    /// the eyes: the near grid shown, and the one being made as the eyes
    /// move on (see [`Self::begin_near`]), and their uniforms.
    near: Option<Tracer>,
    near_next: Option<Tracer>,
    near_terrain: wgpu::Buffer,
    target: Option<Target>,
    chunks: HashMap<TilePos, GpuChunk>,
}

impl Renderer3d {
    #[cfg(test)]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::with_samples(device, queue, SAMPLES)
    }

    /// The same, `samples` a pixel (1 or 4).
    pub fn with_samples(device: &wgpu::Device, queue: &wgpu::Queue, samples: u32) -> Self {
        // What goes wrong making the shaders and pipelines (a shader the
        // driver's compiler refuses, a format the card lacks) is caught
        // here rather than lost or fatal: written in startup.log, and the
        // views drawn in flat colours (see `looks_failed`).
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        // Each shader once a look (see [`Look`]): in the views' own in flat
        // colours, with the game's textures, and in the game's. One shader
        // for all, choosing at each pixel, made the views' own look pay for
        // the others (registers held for textures and light whether used or
        // not, which leaves fewer pixels in flight).
        let shader = |label: &str, body: &str, look: Look| {
            let (game, textured) = (look == Look::Game, look != Look::Flat);
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!("const GAME_LOOK: bool = {game};\nconst TEXTURED: bool = {textured};\n{GLOBALS_WGSL}{}{body}", crate::textures::LOOK_WGSL))),
            })
        };
        // The meshes twice more: solid, and water (its surface a little
        // below its blocks' tops, see-through).
        let (solid_wgsl, water_wgsl) = (format!("const WATER: bool = false;\n{MESH_WGSL}"), format!("const WATER: bool = true;\n{MESH_WGSL}"));
        let mesh_shaders = Look::ALL.map(|look| shader("massif 3d meshes", &solid_wgsl, look));
        let water_shaders = Look::ALL.map(|look| shader("massif 3d water", &water_wgsl, look));
        let trace_shaders = Look::ALL.map(|look| shader("massif 3d tracer", TRACE_WGSL, look));
        // The same, its direction read at each sample: the pixel shaded
        // once a sample.
        let fine_wgsl = TRACE_WGSL.replace("@location(0) ndc: vec2<f32>,", "@location(0) @interpolate(perspective, sample) ndc: vec2<f32>,");
        let fine_shaders = (samples > 1).then(|| Look::ALL.map(|look| shader("massif 3d tracer, a ray a sample", &fine_wgsl, look)));
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
            label: Some("massif 3d chunks"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Sint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
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
                texture(4, wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2),
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
        let near_terrain = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif 3d near terrain"),
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
        let made = Self::make_atlas(device, queue, &crate::textures::Atlas::plain());
        let clouds = Self::make_clouds(device, queue, &crate::textures::clouds(None));
        let look_group = Self::make_look(device, &look_layout, &made, &sampler, &clouds);
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
        let depth = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
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
            count: samples,
            ..Default::default()
        };
        let pipeline = |label: &str, blend: wgpu::BlendState, depth_write: bool, compare: wgpu::CompareFunction, cull: Option<wgpu::Face>, mesh_shader: &wgpu::ShaderModule| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: mesh_shader,
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
                depth_stencil: depth(depth_write, compare),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: mesh_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &target(blend),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque_pipeline = mesh_shaders.each_ref().map(|m| pipeline("massif 3d opaque", wgpu::BlendState::REPLACE, true, wgpu::CompareFunction::Less, Some(wgpu::Face::Back), m));
        let water_pipeline = water_shaders.each_ref().map(|m| pipeline("massif 3d water", wgpu::BlendState::ALPHA_BLENDING, false, wgpu::CompareFunction::Less, None, m));
        let xray_pipeline = mesh_shaders.each_ref().map(|m| pipeline("massif 3d x-ray", wgpu::BlendState::ALPHA_BLENDING, false, wgpu::CompareFunction::Always, None, m));
        let trace = |trace_shader: &wgpu::ShaderModule, compare: wgpu::CompareFunction| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("massif 3d tracer"),
            layout: Some(&trace_pipeline_layout),
            vertex: wgpu::VertexState {
                module: trace_shader,
                entry_point: Some("vs_trace"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            // First in the frame, it writes every pixel, the sky included
            // (Always); the near grid's, over it, where it is nearer (Less).
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: trace_shader,
                entry_point: Some("fs_trace"),
                compilation_options: Default::default(),
                targets: &target(wgpu::BlendState::REPLACE),
            }),
            multiview_mask: None,
            cache: None,
        });
        let trace_pipeline = trace_shaders.each_ref().map(|m| trace(m, wgpu::CompareFunction::Always));
        let trace_fine_pipeline = fine_shaders.as_ref().map(|f| f.each_ref().map(|m| trace(m, wgpu::CompareFunction::Always)));
        let trace_near_pipeline = trace_shaders.each_ref().map(|m| trace(m, wgpu::CompareFunction::Less));
        let trace_fine_near_pipeline = fine_shaders.as_ref().map(|f| f.each_ref().map(|m| trace(m, wgpu::CompareFunction::Less)));
        let (chunk_tiles, chunk_group) = Self::make_chunk_tiles(device, &chunk_layout, &[]);
        let errors: Vec<String> = [pollster::block_on(validation.pop()), pollster::block_on(internal.pop())].into_iter().flatten().map(|e| e.to_string()).collect();
        let looks_failed = (!errors.is_empty()).then(|| errors.join("; "));
        if let Some(e) = &looks_failed {
            log::error!("the 3D views' shaders failed: {e}");
            crate::startup::note(&format!("the 3D views' shaders or pipelines failed on this graphics card, the views are drawn in flat colours: {e}"));
        }
        Self {
            device: device.clone(),
            queue: queue.clone(),
            samples,
            opaque_pipeline,
            water_pipeline,
            xray_pipeline,
            trace_pipeline,
            trace_fine_pipeline,
            trace_near_pipeline,
            trace_fine_near_pipeline,
            fine: false,
            globals,
            bind_group,
            chunk_layout,
            trace_layout,
            chunk_tiles,
            chunk_group,
            slot_tiles: Vec::new(),
            free_slots: Vec::new(),
            arenas: [Vec::new(), Vec::new(), Vec::new()],
            terrain,
            palette,
            pair_mats,
            look_layout,
            look_group,
            sampler,
            clouds,
            material_count: made.count,
            looks_failed,
            look_bytes: made.bytes,
            fluid_layers: [0, 0],
            tracer: None,
            near: None,
            near_next: None,
            near_terrain,
            target: None,
            chunks: HashMap::new(),
            copier: crate::view_image::Copier::new(device),
        }
    }

    /// The game's textures on the graphics card: the layers with their
    /// levels of detail, and the materials' layers and colours (1,024 a
    /// row).
    fn make_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &crate::textures::Atlas) -> MadeAtlas {
        use crate::textures::{MIPS, SIDE, mip_chain};
        // Two layers at least, for OpenGL (see `set_grid`).
        let layers = atlas.layers.len().max(2) as u32;
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
        for mip in (0..MIPS).filter(|_| !chains.is_empty()) {
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
                    depth_or_array_layers: chains.len() as u32,
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
        let mut rgba = vec![0u8; w * h * 4];
        for (k, c) in atlas.colors.iter().take(count).enumerate() {
            rgba[k * 4..k * 4 + 3].copy_from_slice(c);
            rgba[k * 4 + 3] = 255;
        }
        let colors = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d material colours"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            colors.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w as u32 * 4),
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
        // Each layer and its levels of detail (a third more), the tables.
        let bytes = layers as u64 * (SIDE * SIDE * 4) as u64 * 4 / 3 + (w * h * 12) as u64;
        MadeAtlas {
            atlas: array,
            materials: materials.create_view(&wgpu::TextureViewDescriptor::default()),
            colors: colors.create_view(&wgpu::TextureViewDescriptor::default()),
            count: count as u32,
            bytes,
        }
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

    fn make_look(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, made: &MadeAtlas, sampler: &wgpu::Sampler, clouds: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d look"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&made.atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&made.materials),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(clouds),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&made.colors),
                },
            ],
        })
    }

    /// The game's textures, for the player's view and the views' own look
    /// (Settings ▸ Graphics), and its clouds.
    pub fn set_look(&mut self, atlas: &crate::textures::Atlas, clouds: &[u8]) {
        let most = self.device.limits().max_texture_array_layers as usize;
        let made = if atlas.layers.len() <= most {
            Self::make_atlas(&self.device, &self.queue, atlas)
        } else {
            Self::make_atlas(&self.device, &self.queue, &crate::textures::Atlas::plain())
        };
        self.clouds = Self::make_clouds(&self.device, &self.queue, clouds);
        self.look_group = Self::make_look(&self.device, &self.look_layout, &made, &self.sampler, &self.clouds);
        self.material_count = made.count;
        self.look_bytes = made.bytes;
        self.fluid_layers = [atlas.water, atlas.lava];
    }

    /// Bytes of the graphics card's memory the game's textures take.
    pub fn look_bytes(&self) -> u64 {
        self.look_bytes
    }

    /// The materials the atlas knows, and the layers of water and lava.
    pub fn look_info(&self) -> (u32, [u16; 2]) {
        (self.material_count, self.fluid_layers)
    }

    /// The texture of the chunks' tiles, room for `tiles` and more (rows of
    /// `SLOT_ROW` slots, as many rows as a power of two), filled with them.
    fn make_chunk_tiles(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, tiles: &[[i32; 2]]) -> (wgpu::Texture, wgpu::BindGroup) {
        let rows = (tiles.len() as u32).div_ceil(SLOT_ROW).max(1).next_power_of_two();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif 3d chunk tiles"),
            size: wgpu::Extent3d {
                width: SLOT_ROW,
                height: rows,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Sint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif 3d chunk tiles"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture.create_view(&wgpu::TextureViewDescriptor::default())),
            }],
        });
        (texture, group)
    }

    /// Write the tile of slots `from..` to the texture of the chunks' tiles
    /// (whole rows).
    fn write_chunk_tiles(&self, from: usize) {
        let row = SLOT_ROW as usize;
        let (first, last) = (from / row, self.slot_tiles.len().div_ceil(row));
        if last <= first {
            return;
        }
        let mut data = vec![[0i32; 2]; (last - first) * row];
        let end = self.slot_tiles.len().min(last * row);
        data[..end - first * row].copy_from_slice(&self.slot_tiles[first * row..end]);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.chunk_tiles,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: first as u32, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SLOT_ROW * 8),
                rows_per_image: Some((last - first) as u32),
            },
            wgpu::Extent3d {
                width: SLOT_ROW,
                height: (last - first) as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    /// A slot for a chunk of `tile`, its tile written for the shaders.
    fn take_slot(&mut self, tile: TilePos) -> u16 {
        if let Some(slot) = self.free_slots.pop() {
            self.slot_tiles[slot as usize] = [tile.x, tile.z];
            self.write_chunk_tiles(slot as usize);
            return slot;
        }
        let slot = self.slot_tiles.len();
        self.slot_tiles.push([tile.x, tile.z]);
        if self.slot_tiles.len() as u32 > SLOT_ROW * self.chunk_tiles.height() {
            // Room for twice as many: everything written again.
            let (texture, group) = Self::make_chunk_tiles(&self.device, &self.chunk_layout, &self.slot_tiles);
            self.chunk_tiles = texture;
            self.chunk_group = group;
            self.write_chunk_tiles(0);
        } else {
            self.write_chunk_tiles(slot);
        }
        slot as u16
    }

    /// Put `faces` in an arena of `kind` (0: solid, 1: water), each carrying
    /// `slot`.
    fn place(&mut self, kind: usize, faces: &[Face], slot: u16) -> Option<Range> {
        if faces.is_empty() {
            return None;
        }
        let count = faces.len() as u32;
        let arenas = &mut self.arenas[kind];
        let (page, start) = match arenas.iter_mut().enumerate().find_map(|(k, a)| a.free.take(count).map(|s| (k, s))) {
            Some(found) => found,
            None => {
                // A new page, twice the last (or as large as these faces).
                let capacity = arenas.last().map_or(FIRST_ARENA_FACES, |a| (a.capacity * 2).min(ARENA_FACES)).max(count);
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(["massif 3d faces", "massif 3d water faces", "massif 3d x-ray faces"][kind]),
                    size: capacity as u64 * size_of::<Face>() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut arena = Arena { buffer, capacity, free: FreeRanges(vec![(0, capacity)]) };
                let start = arena.free.take(count).expect("a new page holds them");
                arenas.push(arena);
                (arenas.len() - 1, start)
            }
        };
        let carried: Vec<Face> = faces.iter().map(|f| Face { pad: slot, ..*f }).collect();
        self.queue.write_buffer(&self.arenas[kind][page].buffer, start as u64 * size_of::<Face>() as u64, bytemuck::cast_slice(&carried));
        Some(Range { page: page as u32, start, count })
    }

    /// Give a chunk's faces and slot back.
    fn free_chunk(&mut self, chunk: GpuChunk) {
        for (kind, range) in [(0, chunk.solid), (1, chunk.water), (2, chunk.xray)] {
            if let Some(r) = range {
                self.arenas[kind][r.page as usize].free.give(r.start, r.count);
            }
        }
        self.free_slots.push(chunk.slot);
    }

    /// Make the tracer's textures for a grid of `pages` pages from `page0`,
    /// with a layer for each page `present` says holds tiles (row-major).
    /// Everything uploaded before is dropped.
    pub fn set_grid(&mut self, stride: i32, page0: (i32, i32), pages: (u32, u32), present: &[bool]) -> Result<(), String> {
        self.tracer = None;
        self.end_near();
        self.tracer = Some(self.make_tracer(stride, page0, pages, present, &self.terrain)?);
        Ok(())
    }

    /// The textures and bind group of a tracer over `pages` pages from
    /// `page0`, its uniforms in `uniform`.
    fn make_tracer(&self, stride: i32, page0: (i32, i32), pages: (u32, u32), present: &[bool], uniform: &wgpu::Buffer) -> Result<Tracer, String> {
        let count = present.iter().filter(|p| **p).count().max(1) as u32;
        let limit = self.device.limits().max_texture_array_layers;
        if count > limit {
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
        // Two layers at least: OpenGL takes a texture of one layer for a
        // plain 2D one, which a view of an array cannot read.
        let layers = count.max(2);
        let cols = make("massif 3d columns", page, page, layers, 1, wgpu::TextureFormat::R32Uint);
        let maxes = make("massif 3d maxes", page / 2, page / 2, layers, PAGE_SHIFT, wgpu::TextureFormat::R16Uint);
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
            resource: uniform.as_entire_binding(),
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
        Ok(Tracer {
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
            bytes: per_layer * layers as u64,
        })
    }

    /// Put the packed columns of a tile (row-major, `TILE_SIZE`²) in the
    /// tracer's textures, with the max pyramid over them.
    pub fn upload_columns(&mut self, tile: TilePos, packed: &[u32]) {
        profiling::scope!("3D upload");
        if let Some(tr) = &mut self.tracer {
            write_columns(&self.queue, tr, tile, packed);
        }
    }

    /// Put a tile's packed columns in the near grid being made (`next`) or
    /// in the one shown (see [`Self::begin_near`]).
    pub fn upload_near_columns(&mut self, tile: TilePos, packed: &[u32], next: bool) {
        if let Some(tr) = if next { &mut self.near_next } else { &mut self.near } {
            write_columns(&self.queue, tr, tile, packed);
        }
    }

    /// Make the near grid of `pages` pages from `page0`, every block, to
    /// fill with [`Self::upload_near_columns`] while the one shown stays,
    /// and show with [`Self::show_near`] once whole: the grid's own tracer
    /// leaves that room empty (its `hole`) and the near one draws it, over
    /// it in a pass of its own.
    pub fn begin_near(&mut self, page0: (i32, i32), pages: (u32, u32)) -> Result<(), String> {
        let present = vec![true; (pages.0 * pages.1) as usize];
        self.near_next = Some(self.make_tracer(1, page0, pages, &present, &self.near_terrain)?);
        Ok(())
    }

    /// Show the near grid made since [`Self::begin_near`], in the place of
    /// the one shown.
    pub fn show_near(&mut self) {
        if self.near_next.is_some() {
            self.near = self.near_next.take();
        }
    }

    /// No near grid: the grid's own tracer draws everything again.
    pub fn end_near(&mut self) {
        self.near = None;
        self.near_next = None;
    }

    /// The meshed faces of a tile gone, its columns left.
    pub fn forget_faces(&mut self, tile: TilePos) {
        if let Some(old) = self.chunks.remove(&tile) {
            self.free_chunk(old);
        }
    }
}

/// Write a tile's packed columns in `tr`, and the maxes of its squares.
fn write_columns(queue: &wgpu::Queue, tr: &mut Tracer, tile: TilePos, packed: &[u32]) {
    {
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
        write(queue, &tr.cols, 0, lx, lz, n, bytemuck::cast_slice(packed), 4);
        // Mips 0 to 6 of the maxes: squares of 2 to 128 columns.
        let mut level: Vec<u16> = packed.iter().map(|&c| (c & HEIGHT_MASK) as u16).collect();
        let mut side = n as usize;
        for mip in 0..7u32 {
            level = halve(&level, side);
            side /= 2;
            write(queue, &tr.maxes, mip, lx >> (mip + 1), lz >> (mip + 1), side as u32, bytemuck::cast_slice(&level), 2);
        }
        tr.tile_tops.insert(tile, level[0]);
        tr.dirty_pages.insert(page);
    }
}

impl Renderer3d {
    /// After tiles were uploaded: the maxes of their pages and the pyramid
    /// over pages, of every tracer.
    pub fn flush(&mut self) {
        for tr in [&mut self.tracer, &mut self.near, &mut self.near_next].into_iter().flatten() {
            flush_tracer(&self.queue, tr);
        }
    }
}

/// The maxes of the pages tiles were written in, and the pyramid over pages.
fn flush_tracer(queue: &wgpu::Queue, tr: &mut Tracer) {
    {
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
                queue.write_texture(
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
            queue.write_texture(
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
}

impl Renderer3d {
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

    /// Put (or replace) the meshed faces of a tile on the GPU: in the
    /// arenas, each face carrying the chunk's slot, which holds its tile.
    pub fn upload(&mut self, tile: TilePos, faces: &ChunkFaces) {
        profiling::scope!("3D upload");
        if let Some(old) = self.chunks.remove(&tile) {
            self.free_chunk(old);
        }
        if faces.solid.is_empty() && faces.water.is_empty() && faces.xray.is_empty() {
            return;
        }
        let slot = self.take_slot(tile);
        let solid = self.place(0, &faces.solid, slot);
        let water = self.place(1, &faces.water, slot);
        let xray = self.place(2, &faces.xray, slot);
        self.chunks.insert(tile, GpuChunk { slot, solid, water, xray });
    }

    /// Forget a tile that left the map.
    pub fn remove(&mut self, tile: TilePos) {
        if let Some(old) = self.chunks.remove(&tile) {
            self.free_chunk(old);
        }
        self.upload_columns(tile, &[0; (TILE_SIZE * TILE_SIZE) as usize]);
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.arenas = [Vec::new(), Vec::new(), Vec::new()];
        self.slot_tiles.clear();
        self.free_slots.clear();
        self.tracer = None;
        self.end_near();
    }

    /// Bytes of GPU memory the terrain takes.
    pub fn bytes(&self) -> u64 {
        let faces: u64 = self.arenas.iter().flatten().map(|a| a.capacity as u64 * size_of::<Face>() as u64).sum();
        [&self.tracer, &self.near, &self.near_next].into_iter().flatten().map(|t| t.bytes).sum::<u64>() + faces + self.chunk_tiles.height() as u64 * SLOT_ROW as u64 * 8
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
        // The image egui shows is read as plain bytes (see `view_image`).
        let texture = |label: &str, format, samples, usage, view_formats| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = (self.samples > 1).then(|| texture("massif 3d msaa", COLOR_FORMAT, self.samples, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]));
        let resolve = texture(
            "massif 3d image",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            crate::view_image::view_formats(&self.device),
        );
        let depth = texture("massif 3d depth", DEPTH_FORMAT, self.samples, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(Target {
            size,
            msaa: msaa.as_ref().map(view),
            resolve_view: view(&resolve),
            shown: crate::view_image::Shown::of(&self.device, self.copier.as_ref(), &resolve),
            resolve,
            depth: view(&depth),
        });
        true
    }

    pub fn image_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.shown.view)
    }

    /// Draw the traced columns and the meshed chunks of `tiles` into the
    /// render target. Positions are made relative to `origin` (block
    /// coordinates), the origin of `globals`; `bottom` is the bottom of the
    /// world.
    /// Draw the frame into the top left `region` of the target (all of it,
    /// or less while the camera moves).
    pub fn render(&mut self, globals: &Globals, origin: (i32, i32), tiles: &[TilePos], bottom: i32, substrate: Substrate, region: [u32; 2]) {
        profiling::scope!("3D view: draw");
        if self.target.is_none() {
            return;
        }
        // The faces to draw, solid and water, in the order they lie in the
        // arenas: ranges side by side are drawn in one call.
        let runs = |kind: usize| -> Vec<Range> {
            let mut ranges: Vec<Range> = tiles.iter().filter_map(|t| self.chunks.get(t)).filter_map(|c| [c.solid, c.water, c.xray][kind]).collect();
            ranges.sort_unstable_by_key(|r| (r.page, r.start));
            let mut runs: Vec<Range> = Vec::with_capacity(ranges.len());
            for r in ranges {
                match runs.last_mut() {
                    Some(last) if last.page == r.page && last.start + last.count == r.start => last.count += r.count,
                    _ => runs.push(r),
                }
            }
            runs
        };
        let draws = [runs(0), runs(1), runs(2)];
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(globals));
        let unit = |c: [u8; 3]| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0];
        let uniform = |tr: &Tracer, hole: [i32; 4]| {
            let top = tr.page_tops.iter().copied().max().unwrap_or(0);
            TerrainUniform {
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
                hole,
            }
        };
        if let Some(tr) = &self.tracer {
            // The near grid's room, in this grid's columns: pages of every
            // block are whole columns of any stride.
            let hole = match &self.near {
                Some(n) => {
                    let x0 = (n.page0.0 * PAGE - tr.page0.0 * PAGE * tr.stride) / tr.stride;
                    let z0 = (n.page0.1 * PAGE - tr.page0.1 * PAGE * tr.stride) / tr.stride;
                    [x0, z0, x0 + n.pages.0 as i32 * PAGE / tr.stride, z0 + n.pages.1 as i32 * PAGE / tr.stride]
                }
                None => [0; 4],
            };
            self.queue.write_buffer(&self.terrain, 0, bytemuck::bytes_of(&uniform(tr, hole)));
        }
        if let Some(n) = &self.near {
            self.queue.write_buffer(&self.near_terrain, 0, bytemuck::bytes_of(&uniform(n, [0; 4])));
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
                    view: target.msaa.as_ref().unwrap_or(&target.resolve_view),
                    depth_slice: None,
                    resolve_target: target.msaa.as_ref().map(|_| &target.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: lin(sky[0]),
                            g: lin(sky[1]),
                            b: lin(sky[2]),
                            a: 1.0,
                        }),
                        store: if target.msaa.is_some() { crate::view_image::msaa_store(&self.device) } else { wgpu::StoreOp::Store },
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
            let look = if self.looks_failed.is_some() { Look::Flat as usize } else { Look::of(globals) as usize };
            // The traced ground first, then the meshed chunks, then the
            // water over everything.
            if let Some(tr) = &self.tracer {
                let pipelines = self.trace_fine_pipeline.as_ref().filter(|_| self.fine).unwrap_or(&self.trace_pipeline);
                pass.set_pipeline(&pipelines[look]);
                pass.set_bind_group(1, &tr.group, &[]);
                pass.draw(0..3, 0..1);
                // Every block near the eyes, where the grid left room.
                if let Some(near) = &self.near {
                    let pipelines = self.trace_fine_near_pipeline.as_ref().filter(|_| self.fine).unwrap_or(&self.trace_near_pipeline);
                    pass.set_pipeline(&pipelines[look]);
                    pass.set_bind_group(1, &near.group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
            for (kind, runs) in draws.iter().enumerate() {
                if runs.is_empty() {
                    continue;
                }
                pass.set_pipeline([&self.opaque_pipeline[look], &self.water_pipeline[look], &self.xray_pipeline[look]][kind]);
                pass.set_bind_group(1, &self.chunk_group, &[]);
                let mut page = u32::MAX;
                for r in runs {
                    if r.page != page {
                        page = r.page;
                        pass.set_vertex_buffer(0, self.arenas[kind][page as usize].buffer.slice(..));
                    }
                    pass.draw(0..6, r.start..r.start + r.count);
                }
            }
        }
        target.shown.update(self.copier.as_ref(), &mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// Wait until the graphics card has drawn what it was given.
    pub fn wait(&self) {
        crate::view_image::wait(&self.device);
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
        if !crate::view_image::wait(&self.device) {
            return None;
        }
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

#[cfg(test)]
thread_local! {
    /// The brush faces the block face under the cursor, as it did before.
    pub static FACE_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
            sun: [sun.x, sun.y, sun.z, 1.0],
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
    let corners: [Vec4; 8] = std::array::from_fn(|k| {
        let p = Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        *m * p.extend(1.0)
    });
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

/// Where the wheel takes the view: the land the ray `o + t d` meets, or the
/// surface of the water over it (the rays go through water to its bed, and
/// the view would have gone under the sea).
fn zoom_point(dim: &Dimension, o: [f64; 3], d: [f64; 3], max: f64) -> Option<[f64; 3]> {
    let hit = massif_core::voxel::raycast(dim, o, d, max)?;
    let (x, _, z) = hit.block;
    if let Some(level) = massif_core::ops::water_at(dim, x, z)
        && d[1] < 0.0
        && (level as f64) > hit.point[1]
    {
        let t = (level as f64 - o[1]) / d[1];
        if t > 0.0 && t < hit.distance {
            return Some([o[0] + d[0] * t, level as f64, o[2] + d[2] * t]);
        }
    }
    Some(hit.point)
}

/// The pages around the eyes drawn with every block on a map drawn with a
/// column every few blocks (see [`Renderer3d::begin_near`]).
struct Near {
    /// The window's north-west page (pages of [`PAGE`] blocks), of
    /// [`NEAR_PAGES`] × [`NEAR_PAGES`].
    page0: (i32, i32),
    /// The map's tiles in it, and of those built, the faces they mesh.
    tiles: Vec<TilePos>,
    built: HashMap<TilePos, usize>,
    /// Built tiles changed since: built again.
    stale: HashSet<TilePos>,
    /// What they were built for.
    generation: u64,
    style: Style,
    /// The tiles built away from the frames, from a cold copy of the map
    /// (as [`Builder`]'s), and what stops that.
    results: Option<std::sync::mpsc::Receiver<(TilePos, TileBuild)>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Near {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Near {
    fn new(dim: &Dimension, page0: (i32, i32), generation: u64, style: Style) -> Self {
        let ground = dim.ground();
        let side = NEAR_PAGES * PAGE_TILES;
        let tiles = (0..side * side).map(|k| TilePos::new(page0.0 * PAGE_TILES + k % side, page0.1 * PAGE_TILES + k / side)).filter(|t| ground.contains(*t)).collect();
        Self {
            page0,
            tiles,
            built: HashMap::new(),
            stale: HashSet::new(),
            generation,
            style,
            results: None,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Build the window's tiles away from the frames, the nearest `eye`
    /// first, from a cold copy of the map.
    fn start(&mut self, doc: &mut Document, eye: [f64; 3], ctx: &egui::Context) {
        let mut order = self.tiles.clone();
        let n = TILE_SIZE as f64;
        order.sort_by(|a, b| {
            let d = |t: &TilePos| ((t.x as f64 + 0.5) * n - eye[0]).hypot((t.z as f64 + 0.5) * n - eye[2]);
            d(a).total_cmp(&d(b))
        });
        // A copy that encodes nothing here: the tiles changed since they
        // were last saved are encoded by the builder's thread (a map
        // edited at length took seconds of the frame to freeze).
        let copy = doc.world.dims[doc.dim].copy_unfrozen();
        let reg = doc.world.registry.clone();
        let style = self.style;
        let ring = if style.mode == MapMode::Slope { SLOPE_REACH } else { 1 };
        let around = ((massif_core::objects::max_reach(&reg) + ring).max(1) as u32).div_ceil(TILE_SIZE as u32) as i32;
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let (tx, rx) = std::sync::mpsc::channel();
        let stop = self.cancel.clone();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("massif 3d near builder".into()).spawn(move || {
            let mut copy = copy;
            let mut dim = copy.snapshot();
            let pool = builders();
            pool.install(|| {
                for part in order.chunks(pool.current_num_threads().max(1)) {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    // Each tile the part reads decoded once (see `Builder`).
                    let mut read: Vec<TilePos> = part.iter().flat_map(|&c| cell_reads(c, 1, around)).collect();
                    read.sort_unstable_by_key(|t| (t.z, t.x));
                    read.dedup();
                    for level in &mut dim.levels {
                        level.warm(&read);
                        level.trim(WARM_TILES.max(read.len()));
                    }
                    let built: Vec<(TilePos, TileBuild)> = part.par_iter().map(|&t| (t, build_cell(&dim, &reg, sea, seed, t, 1, style))).collect();
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
            self.results = Some(rx);
        }
    }

    /// Every tile built, none changed since.
    fn whole(&self) -> bool {
        self.stale.is_empty() && self.tiles.iter().all(|t| self.built.contains_key(t))
    }

    /// Whether `tile` is in the window.
    fn holds(&self, tile: TilePos) -> bool {
        let (x0, z0) = (self.page0.0 * PAGE_TILES, self.page0.1 * PAGE_TILES);
        let side = NEAR_PAGES * PAGE_TILES;
        (x0..x0 + side).contains(&tile.x) && (z0..z0 + side).contains(&tile.z)
    }
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
    /// The pages a map needs, at the finest stride that fits `budget`
    /// pages.
    fn of(dim: &Dimension, budget: usize) -> Option<Self> {
        let mut shape = Self::at(dim, 1)?;
        while shape.present.iter().filter(|p| **p).count() > budget && shape.stride < 16 {
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
/// The summaries of the 2D map's tiles (see `MapCache::summaries_of`).
pub type Glimpse = HashMap<TilePos, std::sync::Arc<crate::map_render::Summary>>;

/// How far, in tiles, a tile the 2D map did not read takes the summary of
/// one it did: seen whole, the map reads a tile of every 2 × 2 or 4 × 4.
const GLIMPSE_REACH: i32 = 4;

/// The summary of `tile`, or of the nearest tile read within
/// [`GLIMPSE_REACH`].
fn nearest_summary(glimpse: &Glimpse, tile: TilePos) -> Option<&std::sync::Arc<crate::map_render::Summary>> {
    (0..=GLIMPSE_REACH).find_map(|r| {
        (-r..=r).flat_map(move |dz| (-r..=r).map(move |dx| (dx, dz))).filter(|(dx, dz)| dx.abs().max(dz.abs()) == r).find_map(|(dx, dz)| glimpse.get(&TilePos::new(tile.x + dx, tile.z + dz)))
    })
}

/// Whether `glimpse` gives every tile of grid cell `cell` the map has a
/// summary (its own or a near one).
fn glimpsed(dim: &Dimension, glimpse: &Glimpse, cell: TilePos, stride: i32) -> bool {
    let ground = dim.ground();
    (0..stride * stride).all(|k| {
        let t = TilePos::new(cell.x * stride + k % stride, cell.z * stride + k / stride);
        !ground.contains(t) || nearest_summary(glimpse, t).is_some()
    })
}

/// A first look at grid cell `cell` from what the 2D map already read of
/// its tiles (a few columns a tile, `glimpse`): no tile of the map is
/// decoded, so the whole of a large map stands within a second. Each place
/// takes the summary's column nearest it: its height, its top as the view
/// colours it, its water.
#[allow(clippy::too_many_arguments)]
fn build_glimpse(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, style: Style, glimpse: &Glimpse) -> TileBuild {
    let n = TILE_SIZE as usize;
    let places = n / stride as usize;
    let mut heights = vec![0u16; n * n];
    let mut colors = vec![0u16; n * n];
    let mut pairs: Vec<Skin> = Vec::new();
    let mut index: HashMap<Skin, u16> = HashMap::new();
    let water_pair = if style.lava { Skin::flat([207, 92, 20], [160, 60, 10]) } else { Skin::flat([52, 96, 200], [40, 70, 160]) };
    let colours = Colors {
        reg,
        mode: style.mode,
        sea,
        auto: AutoBiome::new(reg),
        stone: reg.block(reg.well_known.stone).color,
        deepslate: reg.block(reg.well_known.deepslate).color,
        mat: Materializer::new(reg, seed, dim.min_y),
        seed: seed as u64,
        lava: style.lava,
        notes: note_colors(reg, style.notes),
        truth: (style.mode == MapMode::TrueColour).then(|| crate::true_color::current(reg)),
    };
    let mut pair = |skin: Skin| *index.entry(skin).or_insert_with(|| {
        pairs.push(skin);
        (pairs.len() - 1) as u16
    });
    for rz in 0..stride {
        for rx in 0..stride {
            let tile = TilePos::new(cell.x * stride + rx, cell.z * stride + rz);
            if !dim.ground().contains(tile) {
                continue;
            }
            let Some(summary) = nearest_summary(glimpse, tile) else { continue };
            // Each of the summary's columns coloured once (a few a tile),
            // for every place it stands for.
            let mut known: Vec<Option<(u16, u16)>> = vec![None; summary.len()];
            for j in 0..places {
                for i in 0..places {
                    let (lx, lz) = ((i * stride as usize + stride as usize / 2) as i64, (j * stride as usize + stride as usize / 2) as i64);
                    let slot = summary.slot(lx, lz);
                    let (h, col) = match known[slot] {
                        Some(v) => v,
                        None => {
                            let sample = summary.sample(slot);
                            let v = if sample.height.is_nan() {
                                (0, 0)
                            } else {
                                let (x, z) = (tile.x * TILE_SIZE + lx as i32, tile.z * TILE_SIZE + lz as i32);
                                let top = massif_core::compose::block_top(sample.height);
                                let wet = sample.fluid != NO_FLUID && sample.fluid as f32 > sample.height + 0.3;
                                if style.water && wet {
                                    ((sample.fluid as i32 - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16, pair(water_pair))
                                } else {
                                    let skin = colours.top(sample.terrain, dim.min_y, top, x, z, 0.0, &sample, sample.cover, wet);
                                    ((top - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16, pair(skin))
                                }
                            };
                            known[slot] = Some(v);
                            v
                        }
                    };
                    if h == 0 {
                        continue;
                    }
                    let k = (rz as usize * places + j) * n + rx as usize * places + i;
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
        airs: Vec::new(),
        faces: ChunkFaces::default(),
    }
}

fn build_coarse(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, style: Style) -> TileBuild {
    build_sampled(dim, reg, sea, seed, cell, stride, 1, style)
}

/// A first look at grid cell `cell` (`stride` × `stride` tiles), a column
/// every `stride × rough` blocks spread over its `rough` × `rough` places:
/// the whole map is up at once, rightly shaped and coloured, before every
/// column is read (see [`ROUGH`]).
fn build_rough(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, style: Style) -> TileBuild {
    build_sampled(dim, reg, sea, seed, cell, stride, ROUGH, style)
}

/// The passes of a large build (see [`View3d::start_builder`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// From the 2D map's summaries of the tiles.
    Glimpse,
    /// A column every [`ROUGH`] places, decoded.
    Rough,
    /// Every column.
    Fine,
}

/// How much coarser the first look at a large map is than its grid: a
/// column every 8 of its places, a 64th of the reading.
const ROUGH: usize = 8;

#[allow(clippy::too_many_arguments)]
fn build_sampled(dim: &Dimension, reg: &Registry, sea: f32, seed: i64, cell: TilePos, stride: i32, rough: usize, style: Style) -> TileBuild {
    let n = TILE_SIZE as usize;
    // Columns read along a tile, and every how many blocks.
    let step = stride as usize * rough;
    let per = n / step;
    let ground = dim.ground();
    let mut heights = vec![0u16; n * n];
    let mut colors = vec![0u16; n * n];
    let mut pairs: Vec<Skin> = Vec::new();
    let water_pair = if style.lava { Skin::flat([207, 92, 20], [160, 60, 10]) } else { Skin::flat([52, 96, 200], [40, 70, 160]) };
    let mut water_at: Option<u16> = None;
    // The colour of the columns the meshes draw (overhangs, caves, levels)
    // in a tile the tracer draws none of: rock.
    let rock = reg.block(reg.well_known.stone).color;
    let rock_pair = Skin::flat(rock, rock);
    let mut rock_at: Option<u16> = None;
    for rz in 0..stride {
        for rx in 0..stride {
            let real = TilePos::new(cell.x * stride + rx, cell.z * stride + rz);
            let Some(view) = ground.view(real) else { continue };
            let b = build_tile_part(dim, reg, sea, seed, real, style, Some(step));
            let base = pairs.len() as u16;
            pairs.extend(&b.pairs);
            // Places of the cell a tile covers, along each side.
            let places = n / stride as usize;
            for j in 0..per {
                for i in 0..per {
                    let (lx, lz) = (i * step + step / 2, j * step + step / 2);
                    let c = lz * n + lx;
                    let top = view.top.get(c);
                    let fluid = view.fluid.get(c);
                    let (mut h, mut col) = (b.heights[c], base + b.colors[c]);
                    if h == 0 {
                        // Overhangs and caves: their ground, at this scale,
                        // in a colour of the tile (rock where the tracer
                        // draws none of its columns: it has no colours).
                        h = (massif_core::compose::block_top(top) - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16;
                        if b.pairs.is_empty() {
                            col = *rock_at.get_or_insert_with(|| {
                                pairs.push(rock_pair);
                                (pairs.len() - 1) as u16
                            });
                        }
                    }
                    if style.water && fluid != NO_FLUID && fluid as f32 > top + 0.3 {
                        h = (fluid as i32 - dim.min_y).clamp(1, HEIGHT_MASK as i32) as u16;
                        col = *water_at.get_or_insert_with(|| {
                            pairs.push(water_pair);
                            (pairs.len() - 1) as u16
                        });
                    }
                    for dj in 0..rough {
                        for di in 0..rough {
                            let k = (rz as usize * places + j * rough + dj) * n + rx as usize * places + i * rough + di;
                            heights[k] = h;
                            colors[k] = col;
                        }
                    }
                }
            }
        }
    }
    TileBuild {
        heights,
        colors,
        depths: Vec::new(),
        pairs,
        airs: Vec::new(),
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
    /// What it built: the cell, its build, and whether it is a first look
    /// (see [`build_rough`]) the cell's own build comes after.
    results: std::sync::mpsc::Receiver<(TilePos, TileBuild, bool)>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The tiles it builds, and how many it brought.
    given: HashSet<TilePos>,
    delivered: usize,
    /// Tiles changed since it copied the map: what it brings of them is out
    /// of date, and they are built again here.
    edited: HashSet<TilePos>,
    /// Where the camera looked when it started, loading around the view:
    /// gone far from there, it stops for one around the new place.
    around: Option<(f64, f64)>,
}

impl Drop for Builder {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Tiles the builder keeps decoded at most (a few hundred kB each): the
/// nearest first is a ring growing around the camera, whose neighbours are
/// read again soon.
const WARM_TILES: usize = 512;

/// The tiles of the map the build of grid cell `cell` reads: its own
/// (`stride` × `stride` of them on a coarse grid) and `around` tiles
/// around.
fn cell_reads(cell: TilePos, stride: i32, around: i32) -> impl Iterator<Item = TilePos> {
    let (x0, z0) = (cell.x * stride - around, cell.z * stride - around);
    let side = stride + 2 * around;
    (0..side * side).map(move |k| TilePos::new(x0 + k % side, z0 + k / side))
}

/// Whether the builder decodes the tiles it reads once a part (see
/// [`View3d::start_builder`]); tests time it without.
/// Whether a large build begins with a first look at every cell (see
/// [`build_rough`]); tests time it against no first look.
fn rough_first() -> bool {
    #[cfg(test)]
    return !NO_ROUGH.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    true
}

#[cfg(test)]
pub static NO_ROUGH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn warm_builds() -> bool {
    #[cfg(test)]
    return !COLD_BUILDS.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    true
}

#[cfg(test)]
pub static COLD_BUILDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The least time between two glimpses of the 2D map's summaries for a
/// 3D view that has built nothing yet (see [`View3d::wants_glimpse`]).
const GLIMPSE_EVERY: Duration = Duration::from_millis(500);

/// The threads that build tiles away from the frames: all the cores but
/// two, which stay for the interface's own work. (At a lower priority, on
/// macOS's efficiency cores, a whole map took two to six times as long to
/// build again, for frames that were not smoother.)
fn builders() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        // At most 16: past that, on a processor of 24 cores and 32 threads,
        // more threads only crowd the memory and the interface's own.
        let n = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).clamp(1, 16);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("massif 3d tiles {i}"))
            // Below the interface on Windows, where a thread of normal
            // priority takes a core from the window's.
            .start_handler(|_| crate::startup::below_normal_priority())
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
    /// The annotations show (the tools', set every frame).
    pub annotations: bool,
    pub camera: Camera,
    renderer: Option<Renderer3d>,
    texture: Option<egui::TextureId>,
    tiles: HashMap<TilePos, TileInfo>,
    /// On a map drawn with a column every few blocks, the pages around the
    /// eyes drawn with every block: shown, and being made as the eyes move.
    near: Option<Near>,
    near_next: Option<Near>,
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
    /// When the frame last changed (the camera, the brush, the map), and
    /// whether the last one drawn traced a ray a sample: once nothing has
    /// changed for a moment, the frame is drawn again so.
    changed_at: f64,
    last_fine: bool,
    last_globals: Option<[f32; GLOBAL_FLOATS]>,
    /// The part of the target the last frame was drawn in.
    last_region: [u32; 2],
    /// Tiles still to build, and the time the last batch took.
    pending: usize,
    build_ms: f32,
    error: Option<String>,
    /// A sculpting stroke in progress, and when its last dab was.
    sculpt: Option<(SculptStroke, Instant)>,
    /// The buttons went down with a click tool, or one that refused to
    /// act there: nothing more until they are let go.
    pressed: bool,
    /// What a tool said it did (or why it did not), for the status bar.
    pub feedback: Option<String>,
    /// Where the cursor points in the world.
    hover: Option<massif_core::voxel::Hit>,
    /// The column under the pointer while it is over the view (the Info
    /// panel's, the filters' eyedropper's).
    pub pointed: Option<(i32, i32)>,
    /// The surface there, smoothed: where the brush lies, which way.
    frame: Option<massif_core::voxel::Frame>,
    /// Asked from the header: look where the map view is, frame the map.
    goto_map: bool,
    frame_map: bool,
    /// Walking in the view (3D Sculpt's player mode).
    walk: Option<Walk>,
    /// The caves under the ground drawn through it.
    pub xray: bool,
    /// The objects of the world's layers drawn.
    pub objects: bool,
    /// The land tinted by how many objects its layers place.
    pub density: bool,
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
    /// the registry's materials and their colours), and the document and
    /// catalogue last checked against it.
    look_key: Option<(usize, u64, u64)>,
    look_seen: Option<(u64, u64, usize)>,
    /// The pass in which the view was last shown, and when: a view no
    /// longer shown builds nothing, and gives its memory back after a
    /// while (see [`View3d::rest_if_hidden`]).
    shown_pass: u64,
    shown_at: f64,
    /// What the 2D map read of the tiles, given when the view is about to
    /// build all of a map (see [`View3d::wants_glimpse`]).
    /// It is for the map's generation and revision then.
    glimpse: Option<((u64, u64), std::sync::Arc<Glimpse>)>,
    /// When the glimpse was last given: a map edited meanwhile (a reference
    /// image dragged changes it every frame) has it again at most every
    /// [`GLIMPSE_EVERY`], not each frame.
    glimpse_at: Option<Instant>,
    /// The grid the map takes, and what it was worked out for: its
    /// generation, dimension, ground tiles, bounds and the pages allowed.
    #[allow(clippy::type_complexity)]
    shape_for: Option<((u64, usize, usize, Option<(TilePos, TilePos)>, u32), Option<GridShape>)>,
}

/// How long a 3D view stays out of sight before its tiles and the
/// graphics card's memory it holds are given back (seconds): switching
/// workspaces back and forth finds it as it was.
const HIDDEN_RELEASE: f64 = 20.0;

/// The height of the bar along the bottom of the player's view.
const BAR_HEIGHT: f32 = 36.0;

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
    /// While the view holds the mouse, the pointer's events are moved to
    /// its middle before egui sees them (`pin_pointer`): the buttons never
    /// reach another panel, nor the pointer leave the view. Here: that
    /// middle, where the system put the pointer last, and how far it moved
    /// in the frame.
    pinned: Option<egui::Pos2>,
    raw: Option<egui::Pos2>,
    raw_delta: egui::Vec2,
    /// The point a drag with Ctrl turns the view about (or that moving it
    /// keeps pace with).
    pivot: Option<[f64; 3]>,
    /// The pass the view was last drawn in, and the frame the body last
    /// moved in (a frame drawn twice moves it once).
    pass: u64,
    frame: u64,
}

/// The points of the closed ring `ring`, with more between them so that
/// none is farther than `step` from the next (`most` points at most).
fn along(ring: &[(f32, f32)], step: f32, most: usize) -> Vec<(f32, f32)> {
    let length: f32 = (0..ring.len()).map(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        (b.0 - a.0).hypot(b.1 - a.1)
    }).sum();
    let step = step.max(length / most as f32);
    let mut out = Vec::new();
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let n = (((b.0 - a.0).hypot(b.1 - a.1) / step).ceil() as usize).max(1);
        out.extend((0..n).map(|k| {
            let t = k as f32 / n as f32;
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        }));
    }
    out
}

/// The edges of the view darkened, the middle clear: the eyes are in the
/// ground, seen through.
fn vignette(painter: &egui::Painter, rect: Rect) {
    let inner = rect.shrink2(rect.size() * 0.22);
    let (dark, clear) = (Color32::from_black_alpha(110), Color32::TRANSPARENT);
    let mut mesh = egui::Mesh::default();
    for (p, c) in [(rect.left_top(), dark), (rect.right_top(), dark), (rect.right_bottom(), dark), (rect.left_bottom(), dark)] {
        mesh.colored_vertex(p, c);
    }
    for (p, c) in [(inner.left_top(), clear), (inner.right_top(), clear), (inner.right_bottom(), clear), (inner.left_bottom(), clear)] {
        mesh.colored_vertex(p, c);
    }
    // A band of two triangles along each side, outer corners to inner.
    for k in 0..4u32 {
        let (a, b) = (k, (k + 1) % 4);
        mesh.add_triangle(a, b, b + 4);
        mesh.add_triangle(a, b + 4, a + 4);
    }
    painter.add(mesh);
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

/// The system locks the pointer where the view holds the mouse (else it is
/// kept in the window and brought back to the middle of the view).
const LOCKS_POINTER: bool = cfg!(target_os = "macos");

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
            annotations: true,
            camera: Camera::default(),
            renderer: None,
            texture: None,
            tiles: HashMap::new(),
            near: None,
            near_next: None,
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
            pressed: false,
            feedback: None,
            hover: None,
            pointed: None,
            frame: None,
            goto_map: false,
            frame_map: false,
            walk: None,
            xray: false,
            objects: true,
            density: false,
            spin: [0.0; 2],
            drift: [0.0; 2],
            dragged_at: 0.0,
            fly: [0.0; 3],
            look_goal: None,
            moving_until: 0.0,
            changed_at: 0.0,
            last_fine: false,
            last_camera: None,
            reduce_while_moving: !cfg!(test),
            catalogue: None,
            look_key: None,
            look_seen: None,
            shown_pass: 0,
            shown_at: 0.0,
            glimpse: None,
            glimpse_at: None,
            shape_for: None,
        }
    }
}

impl View3d {
    /// The annotations as the style shows them (see [`Style::notes`]).
    fn notes(&self, doc: &Document) -> Option<(u64, bool)> {
        Some((doc.world.registry.annotations.fingerprint(), self.annotations))
    }

    /// A view of its own beside this one: looking where it looks, with
    /// the game's blocks it has, nothing built yet.
    pub fn beside(&self) -> Self {
        Self { camera: self.camera, reduce_while_moving: self.reduce_while_moving, catalogue: self.catalogue.clone(), ..Self::default() }
    }

    /// The caves the chunks show: all of them while walking or flying in
    /// the land (a ghost goes into the rock), those open to the sky seen
    /// from above.
    fn caves_drawn(&self) -> massif_core::compose::Caves {
        if self.walk.is_some() || self.xray { massif_core::compose::Caves::All } else { massif_core::compose::Caves::Openings }
    }

    pub fn header(&mut self, ui: &mut Ui) {
        let narrow = ui.available_width() < 480.0;
        crate::map_view::mode_selector(ui, &mut self.mode, narrow, "3d mode");
        ui.separator();
        ui.toggle_value(&mut self.water, theme::glyph(Icon::Waves).to_string())
            .on_hover_text(tr!("Show water"));
        ui.toggle_value(&mut self.xray, theme::glyph(Icon::ScanEye).to_string())
            .on_hover_text(tr!("X-ray: the caves under the ground drawn through it, warm near the surface, cold deep down"));
        ui.toggle_value(&mut self.objects, theme::glyph(Icon::TreePine).to_string())
            .on_hover_text(tr!("Objects: the trees, bushes and rocks of the layers, shown or hidden"));
        ui.toggle_value(&mut self.density, theme::glyph(Icon::Grid3x3).to_string())
            .on_hover_text(tr!("Density: the land coloured by how many objects its layers will place, from few (blue) to many (red); the objects step aside meanwhile"));
        if ui
            .small_button(theme::glyph(Icon::Crosshair).to_string())
            .on_hover_text(tr!("Look at the middle of the map view"))
            .clicked()
        {
            self.goto_map = true;
        }
        if ui
            .small_button(theme::glyph(Icon::Rotate3d).to_string())
            .on_hover_text(tr!("Show the whole map"))
            .clicked()
        {
            self.frame_map = true;
        }
    }

    /// No pages of every block around the eyes: their faces gone, the grid
    /// drawing everything again.
    fn drop_near(&mut self, renderer: &mut Renderer3d) {
        for n in [self.near.take(), self.near_next.take()].into_iter().flatten() {
            for t in n.built.keys() {
                renderer.forget_faces(*t);
            }
        }
        renderer.end_near();
        self.needs_render = true;
    }

    /// On a map drawn with a column every few blocks, the pages around the
    /// eyes with every block, while they are low enough over the land for
    /// a block to show: made as the eyes move into another page (the pages
    /// shown staying until the new ones are whole), the nearest tiles
    /// first, for [`NEAR_BUDGET`] a frame. Whether some remain to build.
    fn update_near(&mut self, doc: &mut Document, renderer: &mut Renderer3d, moving: bool, ctx: &egui::Context) -> bool {
        profiling::scope!("3D view: near");
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
            notes: self.notes(doc),
            caves: self.caves_drawn(),
            xray: self.xray,
            objects: self.objects,
            density: self.density,
        };
        let eye = self.camera.eye();
        let low = stride > 1 && ground_at(doc, eye[0], eye[2]).is_some_and(|g| eye[1] - g < NEAR_HEIGHT);
        let fits = |n: &Near| n.generation == doc.generation && n.style == style;
        if !low || self.near.as_ref().is_some_and(|n| !fits(n)) || self.near_next.as_ref().is_some_and(|n| !fits(n)) {
            if self.near.is_some() || self.near_next.is_some() {
                self.drop_near(renderer);
            }
            if !low {
                return false;
            }
        }
        // The eyes' page in the middle.
        let page = ((eye[0] / PAGE as f64).floor() as i32, (eye[2] / PAGE as f64).floor() as i32);
        let page0 = (page.0 - NEAR_PAGES / 2, page.1 - NEAR_PAGES / 2);
        let made = self.near_next.as_ref().or(self.near.as_ref()).map(|n| n.page0);
        if made != Some(page0) {
            if let Err(e) = renderer.begin_near(page0, (NEAR_PAGES as u32, NEAR_PAGES as u32)) {
                log::warn!("the 3D view's near pages: {e}");
                return false;
            }
            if let Some(old) = self.near_next.take() {
                for t in old.built.keys().filter(|t| !self.near.as_ref().is_some_and(|n| n.built.contains_key(t))) {
                    renderer.forget_faces(*t);
                }
            }
            let mut next = Near::new(doc.dimension(), page0, doc.generation, style);
            next.start(doc, eye, ctx);
            self.near_next = Some(next);
        }
        // What the builder brought for the pages being made, a frame's
        // share of it; then, still, what changed in those shown, built
        // here (an edit's few tiles).
        let started = Instant::now();
        let share = if moving { UPLOAD_BUDGET_MOVING } else { UPLOAD_BUDGET };
        let mut built_any = false;
        let mut more = false;
        let mut done = Vec::new();
        if let Some(rx) = self.near_next.as_ref().and_then(|n| n.results.as_ref()) {
            while started.elapsed() <= share || done.is_empty() {
                match rx.try_recv() {
                    Ok(b) => done.push(b),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
                // Each one's upload is counted in the share.
                let (t, b) = done.last().expect("just pushed");
                let packed = self.pairs.pack(b);
                renderer.upload_near_columns(*t, &packed, true);
                renderer.upload(*t, &b.faces);
            }
            more = !done.is_empty();
        }
        if let Some(n) = self.near_next.as_mut() {
            for (t, b) in &done {
                // Changed since the copy: built again.
                if n.stale.contains(t) {
                    continue;
                }
                n.built.insert(*t, b.faces.solid.len() + b.faces.water.len());
                built_any = true;
            }
        }
        if !moving {
            let dim = doc.dimension();
            let reg = &doc.world.registry;
            let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
            let batch = rayon::current_num_threads().clamp(2, 16);
            for next in [true, false] {
                let Some(n) = (if next { &self.near_next } else { &self.near }) else { continue };
                let todo: Vec<TilePos> = n.stale.iter().copied().collect();
                for part in todo.chunks(batch) {
                    if started.elapsed() > NEAR_BUDGET {
                        more = true;
                        break;
                    }
                    let built: Vec<(TilePos, TileBuild)> = part.par_iter().map(|&t| (t, build_cell(dim, reg, sea, seed, t, 1, style))).collect();
                    for (t, b) in built {
                        let packed = self.pairs.pack(&b);
                        renderer.upload_near_columns(t, &packed, next);
                        renderer.upload(t, &b.faces);
                        let n = if next { self.near_next.as_mut() } else { self.near.as_mut() }.expect("listed above");
                        n.built.insert(t, b.faces.solid.len() + b.faces.water.len());
                        n.stale.remove(&t);
                        built_any = true;
                    }
                }
            }
        } else if self.near.as_ref().is_some_and(|n| !n.stale.is_empty()) {
            more = true;
        }
        if built_any {
            renderer.flush();
            if self.pairs.dirty {
                renderer.set_palette(self.pairs.list());
                self.pairs.dirty = false;
            }
            self.needs_render = true;
        }
        // The pages made whole: shown in the place of the others, whose
        // tiles out of the new ones lose their faces.
        if self.near_next.as_ref().is_some_and(Near::whole) {
            let next = self.near_next.take().expect("checked");
            if let Some(old) = self.near.take() {
                for t in old.built.keys().filter(|t| !next.holds(**t)) {
                    renderer.forget_faces(*t);
                }
            }
            renderer.show_near();
            self.near = Some(next);
            self.needs_render = true;
        }
        more || self.near_next.is_some()
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
                // Its every block near the eyes: built again, and what the
                // near builder brings of it, from a copy made before, left.
                let near = t.offset(dx, dz);
                for n in [&mut self.near, &mut self.near_next].into_iter().flatten() {
                    if n.built.contains_key(&near) || (n.results.is_some() && n.tiles.contains(&near)) {
                        n.stale.insert(near);
                    }
                }
            }
        }
    }

    /// Called every frame, before the views are shown: a view the
    /// workspace no longer shows stops building its tiles at once (its
    /// threads go back to the pool, what they brought and nobody took is
    /// dropped), and once out of sight for [`HIDDEN_RELEASE`] gives back
    /// its tiles and what it holds on the graphics card, made again when
    /// it is shown.
    pub fn rest_if_hidden(&mut self, ctx: &egui::Context) {
        if self.shown_pass + 1 >= ctx.cumulative_pass_nr() {
            return;
        }
        if self.builder.take().is_some() {
            log::info!("3D view out of sight: its tile building stopped");
        }
        let now = ctx.input(|i| i.time);
        if self.renderer.is_some() && now - self.shown_at > HIDDEN_RELEASE {
            log::info!("3D view out of sight for {HIDDEN_RELEASE} s: {} tiles and {} MB of the graphics card given back", self.tiles.len(), self.gpu_bytes() / 1_000_000);
            self.renderer = None;
            self.tiles.clear();
            self.near = None;
            self.near_next = None;
            self.grid = None;
            self.look_key = None;
            self.look_seen = None;
            self.pairs.dirty = true;
            self.needs_render = true;
        }
    }

    pub fn invalidate_all(&mut self) {
        for c in self.tiles.values_mut() {
            c.stale = true;
        }
        self.builder = None;
        for n in [&mut self.near, &mut self.near_next].into_iter().flatten() {
            n.stale.extend(n.built.keys().copied());
        }
    }

    /// Where the cursor points, with a sculpting tool.
    #[cfg(test)]
    pub fn hover(&self) -> Option<massif_core::voxel::Hit> {
        self.hover
    }

    /// Where the brush lies and which way it faces.
    #[cfg(test)]
    pub fn brush_frame(&self) -> Option<massif_core::voxel::Frame> {
        self.frame
    }

    /// The last rendered image, read back from the GPU.
    #[cfg(test)]
    pub fn read_back(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.renderer.as_ref()?.read_image()
    }

    /// What the views' own look has: the game's blocks read, and the
    /// materials the atlas on the graphics card knows.
    #[cfg(test)]
    pub fn look_state(&self) -> (bool, u32) {
        (self.catalogue.is_some(), self.renderer.as_ref().map_or(0, |r| r.look_info().0))
    }

    /// Samples a pixel of the renderer, once there is one.
    #[cfg(test)]
    pub fn samples(&self) -> Option<u32> {
        self.renderer.as_ref().map(|r| r.samples)
    }

    /// The pixels of the image last drawn, and those of the view.
    #[cfg(test)]
    pub fn last_region(&self) -> [u32; 2] {
        self.last_region
    }

    /// Blocks per column of the view's grid: 1, or more on a map too large
    /// for every block.
    #[cfg(test)]
    pub fn stride(&self) -> i32 {
        self.grid.as_ref().map_or(1, |g| g.stride)
    }

    /// Whether the pages around the eyes are drawn with every block.
    #[cfg(test)]
    pub fn near_shown(&self) -> bool {
        self.near.is_some()
    }

    /// The pages around the eyes shown, whole, and none being made.
    #[cfg(test)]
    pub fn near_settled(&self) -> bool {
        self.near_next.is_none() && self.near.as_ref().is_some_and(Near::whole)
    }

    /// Whether the pages around the eyes shown hold the column `(x, z)`.
    #[cfg(test)]
    pub fn near_holds(&self, x: i32, z: i32) -> bool {
        self.near.as_ref().is_some_and(|n| n.holds(TilePos::containing(x, z)))
    }

    /// Tiles still to build.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Tiles built, and what the graphics card holds of the view.
    /// Colour pairs the tracer's palette holds.
    #[cfg(test)]
    pub fn pairs(&self) -> usize {
        self.pairs.list().len()
    }

    /// The cells of the grid that show something (built, or a first look),
    /// and all of them.
    #[cfg(test)]
    pub fn covered(&self) -> (usize, usize) {
        let all = self.cells.as_ref().map_or(0, |c| c.1.len());
        (self.tiles.len(), all)
    }

    /// Whether tiles are being built away from the frames.
    #[cfg(test)]
    pub fn building(&self) -> bool {
        self.builder.is_some()
    }

    /// Whether the view has nothing built and is about to build the whole
    /// map: the 2D map's summaries of the tiles then give it a first look
    /// at once ([`View3d::give_glimpse`]).
    pub fn wants_glimpse(&self, doc: &Document) -> bool {
        let fresh = self.tiles.is_empty() || self.generation != Some(doc.generation);
        let due = self.glimpse_at.is_none_or(|t| t.elapsed() >= GLIMPSE_EVERY);
        fresh && self.glimpse.as_ref().is_none_or(|(k, _)| k.0 != doc.generation || (k.1 != doc.revision && due))
    }

    pub fn give_glimpse(&mut self, doc: &Document, glimpse: Glimpse) {
        self.glimpse = (!glimpse.is_empty()).then(|| ((doc.generation, doc.revision), std::sync::Arc::new(glimpse)));
        self.glimpse_at = Some(Instant::now());
    }

    /// Whether tiles are still to build, here or away from the frames.
    pub fn is_building(&self) -> bool {
        self.builder.is_some() || self.pending > 0
    }

    pub fn tiles_built(&self) -> usize {
        self.tiles.len()
    }

    /// Cells built for good, not only a first look; and whether the cell
    /// holding block `(x, z)` is one.
    #[cfg(test)]
    pub fn cells_built(&self) -> usize {
        self.tiles.values().filter(|c| !c.stale).count()
    }

    #[cfg(test)]
    pub fn built_at(&self, x: i32, z: i32) -> bool {
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        self.tiles.get(&grid_tile(TilePos::containing(x, z), stride)).is_some_and(|c| !c.stale)
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.renderer.as_ref().map_or(0, |r| r.bytes() + r.look_bytes())
    }

    /// The player walking in the view, if any.
    #[cfg(test)]
    pub fn player(&self) -> Option<crate::player::Player> {
        self.walk.as_ref().map(|w| w.player)
    }

    /// The player walking in the view, to move or change.
    #[cfg(test)]
    pub fn player_mut(&mut self) -> Option<&mut crate::player::Player> {
        self.walk.as_mut().map(|w| &mut w.player)
    }

    /// Whether the view holds the mouse.
    #[cfg(test)]
    pub fn captured(&self) -> bool {
        self.walk.as_ref().is_some_and(|w| w.captured)
    }

    /// Put the player at `feet`, flying or not.
    #[cfg(test)]
    pub fn place_player(&mut self, feet: [f64; 3], flying: bool) {
        if let Some(walk) = &mut self.walk {
            walk.player = crate::player::Player::new(feet);
            walk.player.flying = flying;
        }
    }

    /// Before egui reads the frame's input (`App::raw_input_hook`): while
    /// the view holds the mouse, the pointer stays at its middle, where the
    /// crosshair is, whatever point the system gives (on macOS the button
    /// events of a locked pointer come with points of their own): a press
    /// is the view's, never a panel's, a menu's or another window's.
    pub fn pin_pointer(&mut self, input: &mut egui::RawInput) {
        let Some(walk) = &mut self.walk else { return };
        let Some(c) = walk.pinned.filter(|_| walk.captured) else { return };
        walk.raw_delta = egui::Vec2::ZERO;
        input.events.retain_mut(|e| match e {
            egui::Event::PointerMoved(p) => {
                if let Some(r) = walk.raw {
                    walk.raw_delta += *p - r;
                }
                walk.raw = Some(*p);
                *p = c;
                true
            }
            egui::Event::PointerButton { pos, .. } => {
                *pos = c;
                true
            }
            egui::Event::PointerGone => false,
            _ => true,
        });
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
    /// Walking as a ghost, through the ground, for tests.
    #[cfg(test)]
    pub fn ghost_for_tests(&mut self) {
        if let Some(walk) = &mut self.walk {
            walk.player.set_motion(crate::player::Motion::Ghost);
        }
    }

    fn start_walking(&mut self, doc: &Document, rect: Rect) {
        let (o, d) = self.ray(rect, rect.center());
        let dim = doc.dimension();
        let mut blocks = crate::player::MapBlocks::new(dim);
        let feet = match massif_core::voxel::raycast_out(dim, o, d, 100_000.0) {
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
            pinned: None,
            raw: None,
            raw_delta: egui::Vec2::ZERO,
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
        let to = massif_core::voxel::raycast_out(doc.dimension(), eye, ahead, 256.0).map_or(32.0, |h| h.distance).clamp(8.0, 256.0);
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
        profiling::scope!("3D view: walking");
        let Some(walk) = &mut self.walk else { return };
        let ctx = ui.ctx().clone();
        walk.pass = ctx.cumulative_pass_nr();
        // Read before the input is locked: `peek` locks the context itself,
        // and a lock taken inside another freezes the app as soon as a
        // thread asks for a repaint in between.
        let escape = crate::keymap::peek(&ctx, crate::keymap::Action::ReleaseMouse);
        // Ghost on and off (back to flying), over the view or holding it.
        if (walk.captured || response.hovered()) && !ctx.egui_wants_keyboard_input() && crate::keymap::take(&ctx, crate::keymap::Action::Ghost).is_some() {
            use crate::player::Motion;
            walk.player.set_motion(if walk.player.ghost { Motion::Flying } else { Motion::Ghost });
            ctx.request_repaint();
        }
        // Rays see through the ground the eyes are in (a ghost's).
        let cast = massif_core::voxel::raycast_out;
        let (escape, focused, motion, pointer, dt, turn, mods, scroll, pressed_now) = ui.input(|i| {
            use crate::keymap::{Action, held};
            let key = |a: Action| held(i, a) as i32 as f32;
            (
                escape,
                i.viewport().focused.unwrap_or(true),
                i.pointer.motion(),
                i.pointer.latest_pos(),
                i.stable_dt.min(0.1) as f64,
                // The arrows turn the eyes: left, right, up, down.
                [key(Action::LookRight) - key(Action::LookLeft), key(Action::LookDown) - key(Action::LookUp)],
                i.modifiers,
                i.smooth_scroll_delta.y + i.smooth_scroll_delta.x,
                i.pointer.any_pressed(),
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
            walk.pivot = Some(match cast(doc.dimension(), o, d, 4096.0) {
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
            // The brush's size, sideways (as wide as the tool goes in 3D).
            use crate::tools::Use3d;
            let in_3d = matches!(tools.tool.in_3d(), Use3d::Sculpt | Use3d::Smooth | Use3d::Paint);
            let most = if in_3d { massif_core::voxel::MAX_SCULPT_RADIUS } else { massif_core::MAX_RADIUS };
            let r = &mut tools.brush.radius;
            *r = (r.min(most) * (response.drag_delta().x * 0.01).exp()).clamp(1.0, most);
        }
        if free && !ctrl && response.clicked() {
            // The click that takes the mouse sculpts nothing.
            walk.captured = true;
            walk.swallow = true;
            walk.anchor = pointer;
            walk.raw = pointer;
            walk.raw_delta = egui::Vec2::ZERO;
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
                if let Some(hit) = cast(doc.dimension(), o, d, 4096.0)
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
        let motion = motion.unwrap_or(if walk.captured { std::mem::take(&mut walk.raw_delta) } else { egui::Vec2::ZERO });
        walk.pinned = walk.captured.then(|| rect.center());
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
            // middle of the view. Never on macOS, where it is locked: moving
            // it there (winit) ties the pointer to the mouse again, which
            // undid the lock at the first press of the right button, whose
            // event comes with a point other than the click's; the hidden
            // pointer then wandered off the view, out of the window.
            if !LOCKS_POINTER
                && let (Some(p), Some(a)) = (walk.raw, walk.anchor)
                && p.distance(a) > 2.0
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(rect.center()));
                walk.anchor = Some(rect.center());
                walk.raw = Some(rect.center());
            }
            // A press holds the mouse again, should anything have let it go
            // (a gesture of the system, another app's dialog).
            if pressed_now && !walk.swallow {
                hold_mouse(&ctx, true);
            }
        }
        use crate::player::Keys;
        let keys = if walk.captured || (response.hovered() && !typing) {
            ui.input(|i| {
                use crate::keymap::{Action, first_press};
                let down = |a: Action| crate::keymap::held(i, a);
                let held = |a: Action| down(a) as i32 as f64;
                // ⌘ and Alt keep their shortcuts.
                if i.modifiers.mac_cmd || i.modifiers.alt {
                    Keys::default()
                } else {
                    // The game's keys, as the keymap has them: Shift sneaks
                    // (flying: goes down), Ctrl sprints; Q goes down too.
                    // (The arrows turn the eyes.)
                    Keys {
                        forward: held(Action::WalkForward) - held(Action::WalkBack),
                        right: held(Action::WalkRight) - held(Action::WalkLeft),
                        forward_pressed: first_press(i, Action::WalkForward),
                        jump: down(Action::Jump),
                        jump_pressed: first_press(i, Action::Jump),
                        sneak: down(Action::Sneak),
                        run: down(Action::Sprint),
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
    fn player_bar(&mut self, ui: &mut Ui, rect: Rect, tools: &mut ToolState, back: Option<&str>, inside: bool) {
        use crate::player::Motion;
        use crate::tools::Use3d;
        let Some(walk) = &mut self.walk else { return };
        let pal = Palette::of(ui);
        let painter = ui.painter_at(rect);
        let c = rect.center();
        if walk.captured {
            crate::reticle::paint(&painter, c, &crate::reticle::get(ui.ctx()));
        }
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - BAR_HEIGHT), rect.right_bottom());
        // The bar takes the pointer, under its controls too: a click or a
        // drag begun on it never reaches the view.
        ui.interact(bar, ui.scope_id().with("player bar"), Sense::click_and_drag());
        painter.rect_filled(bar, 0, pal.panel.gamma_multiply(0.94));
        painter.hline(bar.x_range(), bar.top(), egui::Stroke::new(1.0, pal.border));
        // What the buttons do with the tool chosen.
        // (Painting, the inverse puts back the map's own terrain, or the one
        // the height calls for.)
        // (A tool of the map only: the buttons look and move.)
        let back = back.map(|name| tr!("Back to {name}", name));
        let name = crate::i18n::tr_name(tools.tool.label());
        let (tool, inverse) = match tools.tool.in_3d() {
            Use3d::MapOnly => (None, None),
            Use3d::Smooth => (Some(tr!("Smooth")), None),
            Use3d::Brush if tools.tool == Tool::Smooth => (Some(tr!("Smooth")), None),
            Use3d::Paint => (Some(tr!("Paint")), Some(back.as_deref().unwrap_or(tr!("Paint by height")))),
            Use3d::Sculpt if tools.tool == Tool::Dig => (Some(tr!("Dig")), Some(tr!("Add"))),
            Use3d::Sculpt => (Some(tr!("Add")), Some(tr!("Dig"))),
            Use3d::Click if tools.tool == Tool::Flood => (Some(name), Some(tr!("Lower the water"))),
            Use3d::Click => (Some(name), None),
            Use3d::Brush => (Some(name), Some(tr!("Inverse"))),
        };
        let ghost_key = crate::keymap::get().label(crate::keymap::Action::Ghost);
        let ghost_hint = (walk.player.ghost, ghost_key.as_deref());
        let map_only = tool.is_none().then(|| tr!("{name}: on the map only", name));
        let mut hints: Vec<(&str, &str)> = Vec::new();
        if walk.captured {
            if let Some(tool) = tool {
                hints.push(("RMB", tool));
            }
            if let Some(inv) = inverse {
                hints.push(("LMB", inv));
            }
            hints.extend([("W A S D", tr!("Move")), ("Space", if walk.player.ghost { tr!("Up") } else { tr!("Jump · twice: fly") }), ("Shift", tr!("Sneak · down")), ("Ctrl", tr!("Sprint"))]);
            if let (on, Some(key)) = ghost_hint {
                hints.push((key, if on { tr!("Leave Ghost") } else { tr!("Ghost: through the ground") }));
            }
            hints.push(("Esc", tr!("Free the mouse")));
        } else {
            hints.push(("LMB", tr!("Look around")));
            match (tool, &map_only) {
                (Some(tool), _) => hints.push(("RMB", tool)),
                (None, Some(text)) => hints.push(("RMB", text)),
                (None, None) => {}
            }
            if let Some(inv) = inverse {
                hints.push(("Alt+RMB", inv));
            }
            hints.extend([
                ("MMB", tr!("Pick block")),
                ("Ctrl+LMB", tr!("Orbit")),
                ("Ctrl+RMB", tr!("Pan")),
                ("Ctrl+MMB", tr!("Size")),
                (tr!("Click"), tr!("Take the mouse")),
                ("W A S D", tr!("Move")),
            ]);
        }
        // On the right, how the player goes (Walking, Flying or Ghost, to
        // choose; swimming, sneaking or in the ground, said) and the speed
        // of flight.
        let p = &walk.player;
        let state = if inside {
            Some(tr!("In the ground"))
        } else if p.swimming {
            Some(tr!("Swimming"))
        } else if p.sneaking {
            Some(tr!("Sneaking"))
        } else {
            None
        };
        let motions = [Motion::Walking, Motion::Flying, Motion::Ghost];
        let labels = [tr!("Walking"), tr!("Flying"), tr!("Ghost")];
        let now = p.motion();
        let font = FontId::proportional(11.0);
        let width = |text: &str, size: f32| painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), pal.text).size().x;
        let group_w: f32 = labels.iter().map(|l| width(l, 12.5) + 26.0).sum::<f32>() + 6.0;
        let right_w = 16.0 + 150.0 + 8.0 + group_w + state.map_or(0.0, |s| 8.0 + width(s, 11.0));
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
        let field = child.add(crate::widgets::Field::new(&mut percent, (lo * 100.0).round()..=(hi * 100.0).round()).text(tr!("Flight speed")).suffix("%").width(150.0));
        if field.changed() {
            tools.sculpt.fly_speed = percent / 100.0;
        }
        child.add_space(8.0);
        if let Some(k) = crate::widgets::button_group(&mut child, "player motion", &labels, &motions.map(|m| m == now)) {
            walk.player.set_motion(motions[k]);
        }
        if let Some(state) = state {
            child.add_space(8.0);
            child.label(egui::RichText::new(state).size(11.0).color(pal.text));
        }
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
        profiling::scope!("3D view: tiles");
        let style = Style {
            mode: self.mode,
            water: self.water,
            lava: doc.world.meta.lava,
            notes: self.notes(doc),
            caves: self.caves_drawn(),
            xray: self.xray,
            objects: self.objects,
            density: self.density,
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
                    Ok((t, built, rough)) => {
                        if !rough {
                            b.delivered += 1;
                        }
                        let edited = b.edited.contains(&t);
                        // Built again here since it changed: newer than this.
                        // A first look never covers a cell's own build.
                        if (edited || rough) && self.tiles.get(&t).is_some_and(|c| !c.stale) {
                            continue;
                        }
                        let packed = self.pairs.pack(&built);
                        renderer.upload_columns(t, &packed);
                        if stride == 1 && !rough {
                            renderer.upload(t, &built.faces);
                        }
                        self.tiles.insert(
                            t,
                            TileInfo {
                                faces: built.faces.solid.len() + built.faces.water.len(),
                                // A first look stays to build.
                                stale: edited || rough,
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
        // Loading around the view: the cells within the view distance of
        // the camera (a cell's middle within its half diagonal more), the
        // rest at most a first look from the 2D map's summaries. A builder
        // for a place left far behind stops.
        let load = crate::graphics::loading();
        let reach = (!load.whole).then_some(load.distance as f64 + span * 0.71);
        // So does one for the whole map once the setting says around it.
        if let (Some(reach), Some(b)) = (reach, &self.builder)
            && b.around.is_none_or(|(x, z)| (x - tx).hypot(z - tz) > reach / 2.0)
        {
            self.builder = None;
        }
        let cells = self.cells(doc.dimension(), doc.generation, stride);
        let mut todo: Vec<(f64, TilePos)> = cells
            .iter()
            .filter(|t| self.tiles.get(t).is_none_or(|c| c.stale))
            .filter(|t| self.builder.as_ref().is_none_or(|b| !b.given.contains(t) || b.edited.contains(t)))
            .map(|&t| (((t.x as f64 + 0.5) * span - tx).hypot((t.z as f64 + 0.5) * span - tz), t))
            .collect();
        todo.sort_by(|a, b| a.0.total_cmp(&b.0));
        let far: Vec<TilePos> = match reach {
            Some(reach) => {
                let k = todo.partition_point(|(d, _)| *d <= reach);
                todo.split_off(k).into_iter().filter(|(_, t)| !self.tiles.contains_key(t)).map(|(_, t)| t).collect()
            }
            None => Vec::new(),
        };
        // Many: away from the frames, once a stroke is over. A brush held
        // over the map changes the same tiles every frame: a builder for
        // them would copy the map and take every core but two again and
        // again, under the brush's feet, for tiles changed again before it
        // brings them. Until it lets go, they are shown as they were.
        if todo.len() > BUILD_AWAY && self.builder.is_none() && !doc.history.is_recording() {
            let order: Vec<TilePos> = todo.drain(..).map(|(_, t)| t).collect();
            self.start_builder(doc, order, far, style, stride, (tx, tz), ctx);
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
    /// `far`, beyond the view distance, has only the first look the 2D
    /// map's summaries give, if any (nothing decoded).
    #[allow(clippy::too_many_arguments)]
    fn start_builder(&mut self, doc: &mut Document, order: Vec<TilePos>, far: Vec<TilePos>, style: Style, stride: i32, at: (f64, f64), ctx: &egui::Context) {
        let around_at = (!far.is_empty() || !crate::graphics::loading().whole).then_some(at);
        // A copy that encodes nothing here: the tiles changed since they
        // were last saved are encoded by the builder's thread (a map
        // edited at length took seconds of the frame to freeze).
        let copy = doc.world.dims[doc.dim].copy_unfrozen();
        let reg = doc.world.registry.clone();
        // The tiles around a cell its build reads: its neighbours' columns
        // (walls, slopes), the objects that reach over it.
        let ring = if style.mode == MapMode::Slope { SLOPE_REACH } else { 1 };
        let around = ((massif_core::objects::max_reach(&reg) + ring).max(1) as u32).div_ceil(TILE_SIZE as u32) as i32;
        let warm = warm_builds();
        let rough_first = rough_first();
        // What the 2D map read, if of the map as it is.
        let glimpse = self.glimpse.take().filter(|(k, _)| *k == (doc.generation, doc.revision)).map(|(_, g)| g);
        let (sea, seed) = (doc.world.meta.sea_level as f32, doc.world.meta.seed);
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancel.clone();
        let ctx = ctx.clone();
        let given: HashSet<TilePos> = order.iter().copied().collect();
        let spawned = std::thread::Builder::new().name("massif 3d builder".into()).spawn(move || {
            let mut copy = copy;
            let mut dim = copy.snapshot();
            let pool = builders();
            pool.install(|| {
                // Many cells: a first look at all of them first, the nearest
                // first, then every column. The first look is the 2D map's
                // summaries of the tiles where it read them all (nothing
                // decoded: the whole map within a second), else a column
                // every `ROUGH` places (see `build_rough`): the whole map
                // stands at once rather than black or missing.
                let first = rough_first && order.len() >= ROUGH_FROM;
                let (mut seen, unseen): (Vec<TilePos>, Vec<TilePos>) = match glimpse.as_deref().filter(|_| first) {
                    Some(g) => order.iter().partition(|&&c| glimpsed(&dim, g, c, stride)),
                    None => (Vec::new(), order.clone()),
                };
                // Beyond the view distance: a first look where the 2D map
                // read the tiles, after those within.
                if let Some(g) = glimpse.as_deref() {
                    seen.extend(far.iter().copied().filter(|&c| glimpsed(&dim, g, c, stride)));
                }
                let first = first || !seen.is_empty();
                let rough_too = rough_first && order.len() >= ROUGH_FROM;
                let passes: Vec<(Pass, &[TilePos])> = if first {
                    vec![(Pass::Glimpse, &seen[..]), (Pass::Rough, if rough_too { &unseen[..] } else { &[] }), (Pass::Fine, &order[..])]
                } else {
                    vec![(Pass::Fine, &order[..])]
                };
                for (pass, cells) in passes {
                    // As many at a time as there are threads (twice: they do
                    // not all take as long; more for a first look, quicker).
                    let batch = match pass {
                        Pass::Glimpse => 32,
                        Pass::Rough => 8,
                        Pass::Fine => 2,
                    } * pool.current_num_threads();
                    for part in cells.chunks(batch) {
                        if stop.load(std::sync::atomic::Ordering::Relaxed) {
                            return;
                        }
                        // The copy is cold: each read of a tile decoded it
                        // again, and a tile is read by its own build and by
                        // its neighbours' (a dozen times a rebuild of the
                        // whole map). The tiles this part reads are decoded
                        // once, and those read longest ago put away again.
                        if warm && pass != Pass::Glimpse {
                            let mut read: Vec<TilePos> = part.iter().flat_map(|&c| cell_reads(c, stride, around)).collect();
                            read.sort_unstable_by_key(|t| (t.z, t.x));
                            read.dedup();
                            for level in &mut dim.levels {
                                level.warm(&read);
                                level.trim(WARM_TILES.max(read.len()));
                            }
                        }
                        let built: Vec<(TilePos, TileBuild, bool)> = part
                            .par_iter()
                            .map(|&t| {
                                let b = match (pass, glimpse.as_deref()) {
                                    (Pass::Glimpse, Some(g)) => build_glimpse(&dim, &reg, sea, seed, t, stride, style, g),
                                    (Pass::Fine, _) => build_cell(&dim, &reg, sea, seed, t, stride, style),
                                    _ => build_rough(&dim, &reg, sea, seed, t, stride, style),
                                };
                                (t, b, pass != Pass::Fine)
                            })
                            .collect();
                        for b in built {
                            if tx.send(b).is_err() {
                                return;
                            }
                        }
                        ctx.request_repaint();
                    }
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
                around: around_at,
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
        profiling::scope!("3D view: look keys");
        let reg = &doc.world.registry;
        let key = (cat, crate::textures::materials_key(reg), crate::textures::colors_key(reg));
        if self.look_key == Some(key) {
            return;
        }
        let most = renderer.device.limits().max_texture_array_layers as usize;
        let atlas = match &self.catalogue {
            Some(c) => crate::textures::atlas(&doc.world.registry, c, most),
            None => crate::textures::Atlas::plain(),
        };
        renderer.set_look(&atlas, &crate::textures::clouds(self.catalogue.as_deref()));
        // Said once a change, in startup.log: what the textured look has.
        crate::startup::note(&match &self.catalogue {
            None => "the 3D views' game textures: the game's blocks are not read (no Minecraft jar found or chosen), the views draw each block in its colour".to_owned(),
            Some(_) => format!("the 3D views' game textures: {} layers for {} of the map's {} materials", atlas.layers.len(), atlas.materials.iter().filter(|m| m[0] != 0).count(), atlas.materials.len()),
        });
        // Objects added or gone: their materials are numbered anew.
        if self.look_key.is_some_and(|k| k.1 != key.1) {
            self.invalidate_all();
        }
        self.look_key = Some(key);
        self.needs_render = true;
    }

    /// Where the brush lies on the surface `f` for the tool in hand: along
    /// its slope for the tools that shape matter (`along`), flat for Paint
    /// and the map's brushes, which act on the columns around.
    fn laid(f: massif_core::voxel::Frame, along: bool) -> massif_core::voxel::Frame {
        if along {
            f
        } else {
            massif_core::voxel::Frame {
                center: f.center,
                normal: [0.0, 1.0, 0.0],
            }
        }
    }

    /// The ray through screen point `p` of the view `rect`: origin and
    /// direction in block coordinates.
    fn ray(&self, rect: Rect, p: egui::Pos2) -> ([f64; 3], [f64; 3]) {
        camera_ray(&self.camera, rect, p)
    }

    /// Why the game's textures, on in Settings ▸ Graphics, are not drawn
    /// in the 3D views, if they are not: said under the setting.
    pub fn look_problem(&self) -> Option<String> {
        if let Some(e) = self.renderer.as_ref().and_then(|r| r.looks_failed.as_ref()) {
            return Some(tr!("This graphics card could not draw the textures: the 3D views show each block in its colour. What it said is in startup.log ({e}).", e).to_owned());
        }
        if self.renderer.is_some() && self.catalogue.is_none() {
            return Some(tr!("No Minecraft found: the 3D views show each block in its colour. Choose the game's jar in the Blocks panel to see its textures.").to_owned());
        }
        None
    }

    /// Where the view stands, for the 2D map to show: the player's eyes
    /// while walking, else the point it turns about, in block coordinates
    /// `(x, z)`; the way it looks over the land, a unit vector `(x, z)`; and
    /// whether it is the player.
    pub fn map_marker(&self) -> ([f64; 2], [f32; 2], bool) {
        let (eye, target) = (self.camera.eye(), self.camera.target);
        let (dx, dz) = ((target[0] - eye[0]) as f32, (target[2] - eye[2]) as f32);
        let len = (dx * dx + dz * dz).sqrt();
        // Looking straight down: the way the top of the view faces.
        let dir = if len > 1e-4 { [dx / len, dz / len] } else { [-self.camera.yaw.sin(), -self.camera.yaw.cos()] };
        match &self.walk {
            Some(walk) => ([walk.player.feet[0], walk.player.feet[2]], dir, true),
            None => ([target[0], target[2]], dir, false),
        }
    }

    /// Sent from the 2D map: look at block `(x, z)` (on the ground there),
    /// or take the player there; `glide`, over a few frames.
    pub fn go_to(&mut self, doc: &Document, x: f64, z: f64, glide: bool) {
        let y = ground_at(doc, x, z).unwrap_or(self.camera.target[1]);
        match &mut self.walk {
            Some(walk) => walk.player = crate::player::Player::new([x, y, z]),
            None if glide => self.look_goal = Some([x, y, z]),
            None => {
                self.look_goal = None;
                self.camera.target = [x, y, z];
            }
        }
    }

    /// Show the view; `sculpt`: the 3D Sculpt editor's, where the buttons
    /// sculpt with its tool.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, tools: &mut ToolState, map_center: Option<[f64; 2]>, sculpt: bool) {
        self.shown_pass = ui.ctx().cumulative_pass_nr();
        self.shown_at = ui.input(|i| i.time);
        let pal = Palette::of(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let Some(render_state) = frame.wgpu_render_state() else {
            painter.rect_filled(rect, 0, pal.canvas);
            painter.text(rect.center(), Align2::CENTER_CENTER, tr!("The 3D view needs the wgpu renderer"), FontId::proportional(13.0), pal.weak);
            return;
        };
        let section = profiling::tracing::span!(profiling::tracing::Level::INFO, "3D view: setup").entered();
        let graphics = crate::graphics::get();
        // Edges smoothed or not (Settings ▸ Graphics): another renderer,
        // everything sent to the graphics card again.
        if self.renderer.as_ref().is_some_and(|r| r.samples != graphics.samples()) {
            self.renderer = None;
            self.tiles.clear();
            self.grid = None;
            self.look_key = None;
            self.needs_render = true;
        }
        let mut renderer = self
            .renderer
            .take()
            .unwrap_or_else(|| Renderer3d::with_samples(&render_state.device, &render_state.queue, graphics.samples()));
        self.update_look(doc, &mut renderer);

        // A new world: show all of it, forget the old tiles, and a stroke
        // that was the old map's.
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            self.sculpt = None;
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
            notes: self.notes(doc),
            caves: self.caves_drawn(),
            xray: self.xray,
            objects: self.objects,
            density: self.density,
        };
        let smooth_changed = set_smooth(&doc.world.meta.smooth);
        if self.style != Some(style) || smooth_changed {
            self.style = Some(style);
            self.invalidate_all();
        }
        let extent = map_rect(doc).map_or(512.0, |(x0, z0, x1, z1)| (x1 - x0).max(z1 - z0) as f32);

        // ---- walking ---------------------------------------------------------
        drop(section);
        let section = profiling::tracing::span!(profiling::tracing::Level::INFO, "3D view: walking and camera").entered();
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
        // In the Sculpt editor the buttons use the tool of the Tools panel
        // (the right one inverts); Shift + drag moves the view, Alt + drag
        // (or the middle button) turns it. Otherwise, and with a tool of
        // the map only, the left button turns, the right one moves.
        use crate::tools::Use3d;
        let use3d = tools.tool.in_3d();
        // The filters' eyedropper takes the click rather than the tool.
        let sculpting = sculpt && use3d != Use3d::MapOnly && tools.filter_pick.is_none();
        // The tools that shape or paint in 3D sculpt this far at most; the
        // map's brushes go as wide as on the map.
        let in_3d = matches!(use3d, Use3d::Sculpt | Use3d::Smooth | Use3d::Paint);
        // Laid along the slope: the tools that shape matter. Paint, as the
        // map's brushes, takes whole columns, its footprint straight down.
        let along_slope = matches!(use3d, Use3d::Sculpt | Use3d::Smooth);
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
        // The wheel: toward the land under the cursor or away from it;
        // with Alt, closer or farther around what the view turns about;
        // with Shift, forward or back over the land; with ⌘ (Ctrl), the size
        // of the sculpting brush.
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y + i.smooth_scroll_delta.x));
            let far = (extent * 3.0).max(30000.0);
            if sculpting && command && zoom != 1.0 {
                // The brush of every tool, as in Brush Settings, as wide as
                // it sculpts in 3D.
                let most = if in_3d { massif_core::voxel::MAX_SCULPT_RADIUS } else { massif_core::MAX_RADIUS };
                let r = &mut tools.brush.radius;
                *r = (r.min(most) * zoom).clamp(1.0, most);
            } else if walking {
                // Walking, the wheel only sizes the brush.
            } else if shift && scroll != 0.0 {
                let step = self.camera.distance as f64 * 0.002 * scroll as f64;
                let (s, c) = (self.camera.yaw.sin() as f64, self.camera.yaw.cos() as f64);
                self.camera.target[0] -= s * step;
                self.camera.target[2] -= c * step;
            } else {
                let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
                // Toward the land under the cursor, which stays under it,
                // the view turning about it from then on: on a large map the
                // point the view first turns about is high over the land,
                // and closing in on it alone stopped the eyes far over the
                // ground. With Alt, around that point as it is. Each turn of
                // the wheel goes a share of the way: slower near the land.
                if factor != 1.0
                    && !alt
                    && let Some(p) = response.hover_pos()
                {
                    let (o, d) = self.ray(rect, p);
                    if let Some(point) = zoom_point(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5) {
                        let k = 1.0 - factor as f64;
                        for (t, p) in self.camera.target.iter_mut().zip(point) {
                            *t += (p - *t) * k;
                        }
                    }
                }
                if factor != 1.0 {
                    self.camera.distance = (self.camera.distance * factor).clamp(MIN_DISTANCE, far);
                }
            }
        }

        // ---- sculpting -------------------------------------------------------
        drop(section);
        let section = profiling::tracing::span!(profiling::tracing::Level::INFO, "3D view: sculpting").entered();
        self.hover = None;
        // The column under the pointer, for the Info panel and the
        // eyedropper: only when the pointer moved or the view did.
        self.pointed = match response.hover_pos() {
            Some(p) if !captured => {
                let (o, d) = self.ray(rect, p);
                massif_core::voxel::raycast_out(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5).map(|h| (h.block.0, h.block.2))
            }
            _ => None,
        };
        if tools.filter_pick.is_some()
            && response.clicked()
            && let (Some(at), Some(p)) = (self.pointed, response.interact_pointer_pos())
        {
            tools.filter_pick_at = Some((at, p));
        }
        if tools.filter_pick.is_some() && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        // Holding the mouse, the brush is where the crosshair is; else under
        // the cursor.
        let aim = if captured { Some(rect.center()) } else { response.hover_pos() };
        if sculpting && let Some(p) = aim {
            let (o, d) = self.ray(rect, p);
            // Through the ground the eyes are in (a ghost's): on the
            // surface beyond, a cave's wall.
            self.hover = massif_core::voxel::raycast_out(doc.dimension(), o, d, (self.camera.distance as f64 + extent as f64) * 1.5);
            // The way the land faces over the brush's whole area, not the
            // face of the block under the cursor (a step's riser would turn
            // it sideways); Raise / Lower straight up on gentle slopes, as
            // on the map. Eased toward it over a few hundredths of a second,
            // so that it glides rather than shakes as the cursor moves.
            let radius = tools.brush_3d().radius;
            let found = self.hover.map(|hit| {
                // (As before, for the tests' pictures: the face under the
                // cursor, smoothed over 5 blocks.)
                #[cfg(test)]
                if FACE_FRAME.get() {
                    return massif_core::voxel::surface_frame(doc.dimension(), &hit, o, d);
                }
                let f = massif_core::voxel::brush_frame(doc.dimension(), &hit, o, d, radius);
                match use3d {
                    Use3d::Sculpt => massif_core::voxel::Frame { normal: massif_core::voxel::upright(f.normal), ..f },
                    _ => f,
                }
            });
            let keep = (-ui.input(|i| i.stable_dt).min(0.1) / 0.05).exp();
            self.frame = match (found, self.frame) {
                (Some(new), Some(old)) => {
                    let n = [(1.0 - keep) * new.normal[0] + keep * old.normal[0], (1.0 - keep) * new.normal[1] + keep * old.normal[1], (1.0 - keep) * new.normal[2] + keep * old.normal[2]];
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
        // Alt + click (a drag turns the view): what is under the cursor, as
        // WorldPainter's eyedropper.
        if sculpting && alt && self.walk.is_none() && (response.clicked() || response.secondary_clicked())
            && let Some(frame) = self.frame
        {
            let (x, z) = (frame.center[0].floor() as i32, frame.center[2].floor() as i32);
            self.feedback = crate::tools::pick(doc, tools, x, z).or(self.feedback.take());
        }
        // The Eyedropper tool with Shift: everything there at once.
        if sculpting && shift && tools.tool == crate::tools::Tool::Eyedropper && self.walk.is_none() && response.clicked()
            && let Some(frame) = self.frame
        {
            let (x, z) = (frame.center[0].floor() as i32, frame.center[2].floor() as i32);
            self.feedback = crate::tools::pick_all(doc, tools, x, z).or(self.feedback.take());
        }
        match (&mut self.sculpt, pressed, self.frame) {
            // Once over the land: a stroke of the map's brushes begins where
            // it is.
            (None, true, Some(frame)) if (captured || response.is_pointer_button_down_on()) && !self.pressed => {
                let (stroke, note) = crate::tools::begin_sculpt(doc, tools, inverse, &frame);
                self.feedback = note.or(self.feedback.take());
                // A click tool, or one that refused, waits for the button to
                // be let go.
                self.pressed = stroke.is_none();
                self.sculpt = stroke.map(|s| (s, Instant::now()));
            }
            (Some(_), false, _) => {
                if let Some((stroke, _)) = self.sculpt.take()
                    && let Some(note) = crate::tools::end_sculpt(doc, stroke)
                {
                    self.feedback = Some(note);
                }
            }
            _ => {}
        }
        if !pressed {
            self.pressed = false;
        }
        if let (Some((stroke, last)), Some(frame)) = (&mut self.sculpt, self.frame) {
            let dt = last.elapsed().as_secs_f32().max(1.0 / 60.0);
            *last = Instant::now();
            crate::tools::sculpt_dab(doc, tools, stroke, &frame, dt);
            if let Some(note) = stroke.take_note() {
                self.feedback = Some(note);
            }
        }
        if self.sculpt.is_some() {
            ui.ctx().request_repaint();
        }

        // Fly with the keyboard while the pointer is over the view: W A S D
        // (or the arrows) move, Q and E go down and up, Shift goes faster.
        if !walking && response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
            let (dt, keys, faster) = ui.input(|i| {
                use crate::keymap::{Action, held};
                let k = |a: Action| held(i, a) as i32 as f64;
                let mods = i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt;
                let keys = if mods {
                    [0.0; 3]
                } else {
                    [k(Action::FlyForward) - k(Action::FlyBack), k(Action::FlyRight) - k(Action::FlyLeft), k(Action::FlyUp) - k(Action::FlyDown)]
                };
                (i.stable_dt.min(0.1) as f64, keys, held(i, Action::FlyFaster))
            });
            // The speed eases toward what the keys ask: starting and
            // stopping take a tenth of a second.
            let speed = (self.camera.distance as f64).max(40.0) * if faster { 3.0 } else { 1.0 };
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
        drop(section);
        let section = profiling::tracing::span!(profiling::tracing::Level::INFO, "3D view: tiles and near").entered();
        let dim = doc.dimension();
        // Worked out again only when the map's tiles or the pages allowed
        // change: it reads every tile of the map, up to five times.
        let shape_key = (doc.generation, doc.dim, dim.ground().len(), dim.bounds(), graphics.gpu_pages);
        if self.shape_for.as_ref().is_none_or(|(k, _)| *k != shape_key) {
            self.shape_for = Some((shape_key, GridShape::of(dim, graphics.gpu_pages as usize)));
        }
        let shape = self.shape_for.as_ref().and_then(|(_, s)| s.clone());
        if self.grid != shape {
            // Another stride: the tiles of the grid are not the same ones.
            if self.grid.as_ref().map(|g| g.stride) != shape.as_ref().map(|g| g.stride) {
                for t in self.tiles.keys() {
                    renderer.remove(*t);
                }
                self.tiles.clear();
            }
            // The pages of every block near the eyes go with the grid.
            for n in [self.near.take(), self.near_next.take()].into_iter().flatten() {
                for t in n.built.keys() {
                    renderer.forget_faces(*t);
                }
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
        if self.update_near(doc, &mut renderer, budget.is_zero(), ui.ctx()) {
            ui.ctx().request_repaint();
        }

        // ---- render ----------------------------------------------------------
        drop(section);
        let section = profiling::tracing::span!(profiling::tracing::Level::INFO, "3D view: render").entered();
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
        let scale = graphics.render_scale * if !moving { 1.0 } else if ppp >= 1.5 { MOVING_SCALE_DENSE } else { MOVING_SCALE };
        let region = [((px[0] as f32 * scale).round() as u32).clamp(1, px[0].max(1)), ((px[1] as f32 * scale).round() as u32).clamp(1, px[1].max(1))];
        if moving {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((self.moving_until - now).max(0.0) + 0.01));
        }
        let origin = (self.camera.target[0].floor() as i32, self.camera.target[2].floor() as i32);
        // The eyes under water (walking, or the camera brought down there):
        // the water's own light.
        let eye = self.camera.eye();
        let (under, inside) = {
            use crate::player::Blocks;
            let mut blocks = crate::player::MapBlocks::new(doc.dimension());
            let (x, z) = (eye[0].floor() as i32, eye[2].floor() as i32);
            // In 3D Sculpt, the eyes in the ground (a ghost's): it is seen
            // through, as in the game's spectator mode, up to the air
            // beyond (the tracer starts there; the meshes show only the
            // faces turned to the eyes, those of the caves beyond).
            (blocks.fluid(x, (eye[1] + 0.12).floor() as i32, z), sculpt && blocks.solid(x, eye[1].floor() as i32, z))
        };
        // The player's view looks like the game: its blocks, light, sky and
        // clouds, the brush marking the blocks it takes as Axiom does.
        let game = walking && matches!(self.mode, MapMode::Terrain | MapMode::TrueColour);
        let under_water = under && self.water;
        let sky = if under_water {
            Sky::under_water(if doc.world.meta.lava { [150, 52, 12] } else { [22, 64, 112] })
        } else if game {
            Sky::GAME
        } else {
            Sky::of(&pal)
        };
        let mut globals = self.camera.globals(origin, rect.width() / rect.height().max(1.0), extent, sky, region[1].max(1));
        globals.cam_f[3] = inside as u8 as f32;
        // No fog (Settings ▸ Graphics): nothing veiled far off. Under water
        // the water's own murk stays.
        globals.sun[3] = graphics.reflections as u8 as f32;
        if !graphics.fog && !under_water {
            globals.mist[3] = 0.0;
        }
        // The game's textures (Settings ▸ Graphics), in the player's view and
        // in the views' own look of the map's blocks (Terrain).
        let (materials, fluids) = renderer.look_info();
        let textured = graphics.textures && self.catalogue.is_some() && materials > 0;
        if textured && !game && matches!(self.mode, MapMode::Terrain | MapMode::TrueColour) {
            let wk = doc.world.registry.well_known;
            globals.look = [1.0, 0.0, 0.0, materials as f32];
            globals.rock = [wk.stone.0 as f32, wk.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
        }
        if game {
            globals.look = [textured as u8 as f32, 1.0, if under_water || !graphics.clouds { 0.0 } else { CLOUDS_Y }, materials as f32];
            if !under_water && graphics.fog {
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
                let f = Self::laid(f, along_slope);
                let brush = tools.brush_3d();
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
                // (A square's half side is its radius.) The map's brushes
                // take whole columns: their footprint is marked at any
                // height (3, 4), the others' blocks in a ball or a box.
                globals.brush = [c[0], c[1], c[2], brush.radius.max(0.5)];
                let shape = match (along_slope, square) {
                    (true, false) => 1.0,
                    (true, true) => 2.0,
                    (false, false) => 3.0,
                    (false, true) => 4.0,
                };
                globals.brush_n = [f.normal[0], f.normal[1], f.normal[2], shape];
                globals.brush_t = [side[0], side[1], side[2], 0.0];
            }
        }
        let key: [f32; GLOBAL_FLOATS] = bytemuck::cast(globals);
        let changed = self.needs_render || self.last_globals != Some(key) || self.last_region != region;
        if changed {
            self.changed_at = now;
        }
        // Still for a moment, the frame is drawn once more, a ray through
        // each sample of a pixel: the traced ground's edges smoothed (Smooth
        // edges, in Settings ▸ Graphics). Moving, or the brush moving over
        // it, a ray a pixel: four times cheaper.
        let fine = now >= self.changed_at + STILL_AFTER && now >= self.moving_until;
        let multi = renderer.samples > 1;
        if changed || (fine && !self.last_fine && multi) {
            self.last_region = region;
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let dim = doc.dimension();
            let (lo_y, hi_y) = (dim.min_y as f32, dim.max_y as f32);
            // The faces of every block near the eyes, when shown.
            let near = self.near.iter().flat_map(|n| n.built.iter().filter(|(_, f)| **f > 0).map(|(t, _)| *t));
            let meshed: Vec<TilePos> = self
                .tiles
                .iter()
                .filter(|(_, info)| info.faces > 0)
                .map(|(t, _)| *t)
                .chain(near)
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
            renderer.fine = fine;
            self.last_fine = fine;
            renderer.render(&globals, origin, &meshed, dim.min_y, substrate, region);
            crate::graphics::drew_3d();
            self.last_globals = Some(key);
            self.needs_render = false;
        }
        if multi && !self.last_fine {
            let still_at = (self.changed_at + STILL_AFTER).max(self.moving_until);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((still_at - now).max(0.0) + 0.01));
        }
        let bytes = renderer.bytes() + renderer.look_bytes();
        self.renderer = Some(renderer);
        if let Some(id) = self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(region[0] as f32 / px[0].max(1) as f32, region[1] as f32 / px[1].max(1) as f32));
            painter.image(id, rect, uv, Color32::WHITE);
        }
        if inside {
            vignette(&painter, rect);
        }
        if let Some(err) = &self.error {
            painter.text(rect.center(), Align2::CENTER_CENTER, err, FontId::proportional(13.0), pal.danger);
        }
        // The sculpting brush, laid on the slope under the cursor: its
        // outline (a custom brush's too), and the way the surface faces. In
        // the game's look the blocks it takes are marked instead (but for a
        // custom brush's outline).
        let marked = game && tools.brush_3d().outline().is_none();
        if let Some(f) = self.frame.filter(|_| !marked).map(|f| Self::laid(f, along_slope)) {
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let to_screen = |p: [f32; 3]| -> Option<egui::Pos2> {
                let v = m * Vec4::new(p[0] - origin.0 as f32, p[1], p[2] - origin.1 as f32, 1.0);
                // Before the eyes (past the near plane), or not drawn.
                (v.w > 0.1).then(|| pos2(rect.center().x + v.x / v.w * rect.width() * 0.5, rect.center().y - v.y / v.w * rect.height() * 0.5))
            };
            let (t1, t2) = massif_core::voxel::tangents(f.normal);
            let on = |u: f32, v: f32| to_screen([f.center[0] + t1[0] * u + t2[0] * v, f.center[1] + t1[1] * u + t2[1] * v, f.center[2] + t1[2] * u + t2[2] * v]);
            let brush = &tools.brush_3d();
            let r = brush.radius;
            // Points behind the eyes (walking, the brush is around the
            // feet) are None: the rest is drawn in runs, without a chord
            // across the view.
            // The outline in the brush's plane, (u, v): along the slope for
            // the tools that shape in 3D, along x and z for the map's.
            let flat: Vec<Vec<(f32, f32)>> = match &brush.mask {
                Some(mask) => {
                    // As wide as it shows across the surface (by squares of
                    // the image's pixels where they are smaller than the
                    // screen's).
                    let side = match (on(-r, 0.0), on(r, 0.0), on(0.0, -r), on(0.0, r)) {
                        (Some(a), Some(b), Some(c), Some(d)) => a.distance(b).max(c.distance(d)),
                        _ => 4096.0,
                    };
                    let traced = crate::brushes::outline_for(mask, side);
                    brush.place_outline(&traced)
                }
                None if brush.is_square() => vec![brush.square_corners(0.0, 0.0).to_vec()],
                None => {
                    let n = if along_slope { 64 } else { ((r * std::f32::consts::TAU) as usize).clamp(64, 2048) };
                    vec![(0..n).map(|i| {
                        let t = i as f32 / n as f32 * std::f32::consts::TAU;
                        (t.cos() * r, t.sin() * r)
                    }).collect()]
                }
            };
            let rings: Vec<Vec<Option<egui::Pos2>>> = if along_slope {
                flat.iter().map(|ring| ring.iter().map(|&(u, v)| on(u, v)).collect()).collect()
            } else {
                // The map's brushes take whole columns: their footprint, as
                // the map draws it, laid on the land column by column, a
                // block at a time, on the top of each.
                let mut blocks = crate::player::MapBlocks::new(doc.dimension());
                let mut drape = |u: f32, v: f32| {
                    let (x, z) = (f.center[0] + u, f.center[2] + v);
                    let y = blocks.top(x.floor() as i32, z.floor() as i32).map_or(f.center[1], |t| t as f32);
                    to_screen([x, y + 0.05, z])
                };
                flat.iter().map(|ring| along(ring, 1.0, 8192).into_iter().map(|(u, v)| drape(u, v)).collect()).collect()
            };
            let draw = |points: Vec<egui::Pos2>, closed: bool| {
                let Some(points) = crate::brushes::drawable(points, 0.75, closed) else { return };
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

        drop(section);
        // ---- overlay -------------------------------------------------------
        let stride = self.grid.as_ref().map_or(1, |g| g.stride);
        let mut text = if stride > 1 {
            let mut text = tr!("The whole map, a column every {stride} blocks (too large for every block) · {usize} tiles · {div_ceil} MB", usize = self.tiles.len() * (stride * stride) as usize, div_ceil = bytes.div_ceil(1 << 20), stride);
            if self.near.is_some() {
                text = format!("{text} · {}", tr!("every block near the eyes"));
            }
            text
        } else {
            tr!("The whole map, every block · {n} tiles · {div_ceil} MB", n = self.tiles.len(), div_ceil = bytes.div_ceil(1 << 20))
        };
        if self.pending > 0 {
            text.push_str(&tr!(" · building {pending} tiles…", pending = self.pending));
        } else if self.build_ms > 0.0 {
            text.push_str(&tr!(" · {build_ms:.0} ms", build_ms = self.build_ms));
        }
        if self.walk.is_none() {
            let at = rect.left_bottom() + vec2(10.0, -10.0);
            let galley = painter.layout_no_wrap(text, FontId::proportional(10.5), Color32::WHITE);
            let bg = Rect::from_min_size(at - vec2(4.0, galley.size().y + 4.0), galley.size() + vec2(8.0, 6.0));
            painter.rect_filled(bg, 4, Color32::from_black_alpha(110));
            painter.galley(at - vec2(0.0, galley.size().y + 1.0), galley, Color32::WHITE);
            if self.density {
                density_legend(&painter, bg.left_top() - vec2(0.0, 8.0));
            }
        }
        let back = crate::tools::unpaint_to(doc, tools).map(|t| crate::i18n::tr_name(&doc.world.registry.terrain(t).name).to_owned());
        self.player_bar(ui, rect, tools, back.as_deref(), inside);
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

/// The density overlay's scale, its bottom left at `at`: the colours from
/// few objects to many, and what they count.
fn density_legend(painter: &egui::Painter, at: egui::Pos2) {
    let (w, h) = (180.0, 8.0);
    let title = painter.layout_no_wrap(tr!("Objects per 100 blocks²").to_owned(), FontId::proportional(10.5), Color32::WHITE);
    let height = title.size().y + h + 22.0;
    let bg = Rect::from_min_size(at - vec2(0.0, height), vec2(w + 16.0, height));
    painter.rect_filled(bg, 4, Color32::from_black_alpha(130));
    painter.galley(bg.min + vec2(8.0, 4.0), title.clone(), Color32::WHITE);
    let bar = Rect::from_min_size(bg.min + vec2(8.0, title.size().y + 8.0), vec2(w, h));
    let top = DENSITY_STOPS[DENSITY_STOPS.len() - 1].0;
    let steps = 60;
    for k in 0..steps {
        let v = top * (k as f32 + 0.5) / steps as f32;
        let c = density_color(v);
        let r = Rect::from_min_max(egui::pos2(bar.left() + bar.width() * k as f32 / steps as f32, bar.top()), egui::pos2(bar.left() + bar.width() * (k + 1) as f32 / steps as f32 + 0.5, bar.bottom()));
        painter.rect_filled(r, 0, Color32::from_rgb(c[0], c[1], c[2]));
    }
    for (v, label) in [(0.0, "0".to_owned()), (top / 2.0, format!("{}", (top / 2.0) as u32)), (top, format!("{}+", top as u32))] {
        let x = bar.left() + bar.width() * v / top;
        let align = if v == 0.0 { egui::Align2::LEFT_TOP } else if v == top { egui::Align2::RIGHT_TOP } else { egui::Align2::CENTER_TOP };
        painter.text(egui::pos2(x, bar.bottom() + 2.0), align, label, FontId::proportional(9.5), Color32::from_white_alpha(220));
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

    /// The 3D view's Objects button hides the objects of the layers (the
    /// structures stay), and its Density overlay counts what the layers
    /// place: many where a forest is painted, none elsewhere.
    #[test]
    fn objects_hide_and_their_density_shows() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Flat, seed: 3, ..GenParams::default() });
        // A little tree: a log under a block of leaves.
        let palette = vec!["minecraft:air".to_owned(), "minecraft:oak_log[axis=y]".to_owned(), "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]".to_owned()];
        let mut blocks = vec![0u16; 3 * 4 * 3];
        for y in 0..4 {
            for z in 0..3 {
                for x in 0..3 {
                    blocks[x + z * 3 + y * 9] = if x == 1 && z == 1 && y < 2 { 1 } else if y >= 2 { 2 } else { 0 };
                }
            }
        }
        let k = w.registry.objects.len() as u32;
        w.registry.objects.push(massif_core::objects::ObjectDef::new("Little tree", [3, 4, 3], palette, blocks));
        let mut def = massif_core::objects::ObjectLayerDef::new("Trees", &[k], &w.registry);
        def.spacing = 6;
        def.density = 1.0;
        def.soil_only = false;
        let layer = w.registry.add_object_layer(def);
        let corner = w.dims[0].bounds().expect("a tile").0;
        let (x0, z0) = (corner.x * TILE_SIZE, corner.z * TILE_SIZE);
        let brush = Brush { shape: BrushShape::Circle, radius: 40.0, profile: Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let mut h = History::default();
        h.begin("forest", 0);
        ops::paint_objects(&mut h.edit(&mut w, 0), &brush, (x0 + 64) as f32, (z0 + 64) as f32, layer, 1);
        h.commit();
        let (dim, reg) = (&w.dims[0], &w.registry);
        let n = TILE_SIZE as usize;
        let density = object_density(dim, reg, w.meta.seed, false, x0, z0, n);
        let at = |i: usize, j: usize| density[j * n + i];
        assert!(at(64, 64) > 1.0, "{} objects per 100 blocks² in the forest", at(64, 64));
        assert!(at(n - 4, n - 4) < 0.01, "none far from it");
        let shown = object_runs(dim, Standing { reg, seed: w.meta.seed, lava: false, objects: true }, x0 + 24, z0 + 24, 80, 80);
        let hidden = object_runs(dim, Standing { reg, seed: w.meta.seed, lava: false, objects: false }, x0 + 24, z0 + 24, 80, 80);
        assert!(shown.iter().any(|c| !c.is_empty()), "the forest stands");
        assert!(hidden.iter().all(|c| c.is_empty()), "hidden with the button");
        assert_eq!(density_color(0.0), DENSITY_STOPS[0].1);
        assert_eq!(density_color(1000.0), DENSITY_STOPS[DENSITY_STOPS.len() - 1].1);
    }

    /// Where the water of MASSIF_WATER_WORLD (a .massif) stands over what
    /// is beside it with nothing to hold it: a column of water whose level
    /// is more than a block over a neighbour's water or ground, in clusters,
    /// with the levels and the levels of the map there.
    #[test]
    fn water_standing_over_its_neighbours() {
        let Some(path) = std::env::var_os("MASSIF_WATER_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let dim = &world.dims[0];
        eprintln!("levels: {:?}", dim.levels.iter().map(|l| (l.name.clone(), l.kind, l.len())).collect::<Vec<_>>());
        eprintln!("sea {}, bounds {:?}, min_y {}, max_y {}", world.meta.sea_level, dim.bounds(), dim.min_y, dim.max_y);
        let ground = dim.ground();
        let (lo, hi) = dim.bounds().unwrap();
        let at = |x: i32, z: i32| -> Option<(f32, i16)> {
            let t = ground.view(TilePos::containing(x, z))?;
            let i = massif_core::coords::local_index(x, z);
            Some((t.top.get(i), t.fluid.get(i)))
        };
        let mut found: Vec<(i32, i32, i16, i32)> = Vec::new();
        for tz in lo.z..=hi.z {
            for tx in lo.x..=hi.x {
                let pos = TilePos::new(tx, tz);
                let Some(t) = ground.view(pos) else { continue };
                if t.fluid.get(0) == NO_FLUID && matches!(t.fluid, massif_core::Plane::Uniform(_)) {
                    continue;
                }
                for k in 0..(TILE_SIZE * TILE_SIZE) as usize {
                    let (x, z) = (tx * TILE_SIZE + (k as i32 % TILE_SIZE), tz * TILE_SIZE + (k as i32 / TILE_SIZE));
                    let (top, f) = (t.top.get(k), t.fluid.get(k));
                    if f == NO_FLUID || f as f32 <= top {
                        continue;
                    }
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let Some((nt, nf)) = at(x + dx, z + dz) else { continue };
                        let held = (massif_core::compose::block_top(nt) as i32) >= f as i32 - 1;
                        let level = if nf != NO_FLUID && nf as f32 > nt { nf as i32 } else { massif_core::compose::block_top(nt) + 1 };
                        if !held && level < f as i32 - 1 {
                            found.push((x, z, f, level));
                            break;
                        }
                    }
                }
            }
        }
        eprintln!("{} columns of water standing over a neighbour", found.len());
        // In clusters of 256 × 256: how many columns, and their extremes.
        type Cluster = (usize, i16, i16, i32, i32);
        let mut groups: std::collections::BTreeMap<(i32, i32), Cluster> = std::collections::BTreeMap::new();
        for &(x, z, f, l) in &found {
            let e = groups.entry((x.div_euclid(256), z.div_euclid(256))).or_insert((0, i16::MAX, i16::MIN, i32::MAX, i32::MIN));
            e.0 += 1;
            e.1 = e.1.min(f);
            e.2 = e.2.max(f);
            e.3 = e.3.min(l);
            e.4 = e.4.max(l);
        }
        let mut top: Vec<_> = groups.into_iter().collect();
        top.sort_by_key(|g| std::cmp::Reverse(g.1.0));
        for ((gx, gz), (n, fmin, fmax, lmin, lmax)) in top.iter().take(20) {
            eprintln!("  around x {}, z {}: {n} columns, water {fmin}…{fmax}, beside {lmin}…{lmax}", gx * 256 + 128, gz * 256 + 128);
        }
    }

    /// A tile none of whose columns the tracer draws (a slab floating over
    /// the whole of it: two spans a column) has no colours of its own; on a
    /// grid of a column every few blocks its columns took the colour after
    /// the tile's last, past the end of the list, and packing them panicked
    /// in the middle of a frame (a user's crash: "the len is 0 but the index
    /// is 0"). They are rock, and the packing never panics.
    #[test]
    fn a_tile_the_tracer_draws_none_of_packs_on_a_coarse_grid() {
        let mut w = generate(&GenParams { tiles_x: 1, tiles_z: 1, landform: Landform::Flat, ..GenParams::default() });
        let tile = w.dims[0].ground().positions().next().unwrap();
        let slab = w.dims[0].add_level("Floating", LevelKind::Solid);
        let mut t = massif_core::tile::Tile::empty_slab();
        t.top = massif_core::Plane::Uniform(160.0);
        t.bottom = Some(massif_core::Plane::Uniform(150.0));
        t.terrain = massif_core::Plane::Uniform(1);
        w.dims[0].levels[slab].insert(tile, t);
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let dim = &w.dims[0];
        for stride in [2, 4] {
            let cell = grid_tile(tile, stride);
            for rough in [1, ROUGH] {
                let b = build_sampled(dim, &w.registry, 63.0, 1, cell, stride, rough, style);
                assert!(b.heights.iter().any(|&h| h > 0), "drawn");
                for (k, &h) in b.heights.iter().enumerate() {
                    if h > 0 {
                        assert!((b.colors[k] as usize) < b.pairs.len(), "stride {stride}, rough {rough}: colour {} of {}", b.colors[k], b.pairs.len());
                    }
                }
                let mut pairs = Pairs::default();
                pairs.pack(&b);
            }
        }
        // A build whose colours point past its list packs all the same.
        let mut b = build_tile(dim, &w.registry, 63.0, 1, tile, style);
        b.pairs.clear();
        b.heights[0] = 10;
        b.colors[0] = 3;
        let packed = Pairs::default().pack(&b);
        assert_eq!(packed[0] & HEIGHT_MASK, 10);
    }

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
        notes: None,
        caves: massif_core::compose::Caves::Openings,
        xray: false,
        objects: true,
        density: false,
    };

    /// A coarse cell of the grid keeps, of each tile, the columns at the
    /// middle of its squares: built alone, without the faces, they are what
    /// the whole tile has there, height, water and colours.
    #[test]
    fn coarse_columns_are_those_of_the_whole_tile() {
        let w = world();
        let dim = &w.dims[0];
        for stride in [2usize, 4] {
            for tile in [TilePos::new(0, -1), TilePos::new(-1, 0), TilePos::new(1, 1)] {
                let whole = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE);
                let part = build_tile_part(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE, Some(stride));
                assert!(part.faces.solid.is_empty() && part.faces.water.is_empty());
                let mut kept = 0;
                for j in (stride / 2..128).step_by(stride) {
                    for i in (stride / 2..128).step_by(stride) {
                        let c = j * 128 + i;
                        assert_eq!(part.heights[c], whole.heights[c], "{tile:?} ({i}, {j})");
                        assert_eq!(part.depths[c], whole.depths[c]);
                        if whole.heights[c] != 0 {
                            assert_eq!(part.pairs[part.colors[c] as usize], whole.pairs[whole.colors[c] as usize]);
                            kept += 1;
                        }
                    }
                }
                assert!(kept > 0);
                assert!(part.pairs.len() <= whole.pairs.len());
            }
        }
    }

    /// Water down a step is drawn as the game draws it: flowing water over
    /// the lower water, its top sloping from the upper water's down toward
    /// the far side, no wall of water at the step; the upper water flat.
    #[test]
    fn water_flows_down_a_step() {
        use massif_core::coords::local_index;
        use massif_core::world::{Dimension, DimensionKind, WorldMeta};
        let mut w = World::new(WorldMeta::default());
        let grass = w.registry.terrain_by_name("Grass").unwrap();
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        // Ground at 60, its first air; water to 66 west of x 64, to 65 east.
        let mut tile = massif_core::tile::Tile::ground(60.0, grass.0, massif_core::tile::NO_FLUID);
        for z in 0..128 {
            for x in 0..128 {
                tile.fluid.set(local_index(x, z), if x < 64 { 66 } else { 65 });
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        w.dims = vec![dim];
        let b = build_tile(&w.dims[0], &w.registry, 63.0, w.meta.seed, TilePos::new(0, 0), STYLE);
        let water = &b.faces.water;
        let top_at = |i: u8, j: u8| water.iter().find(|f| f.dir == UP && (f.x..f.x + f.len).contains(&i) && (f.z as i16..f.z as i16 + f.h).contains(&(j as i16))).copied();
        // The upper water: flat, a ninth under the top of its block.
        let upper = top_at(30, 64).expect("the upper water's top");
        assert_eq!((upper.y, upper.mat), (66, 0x2222));
        // Over the first column of the lower water, flowing water at 65: its
        // top nearly as high as the upper water's on the west, low on the
        // east (corners north-west, north-east, south-west, south-east).
        let flow = top_at(64, 64).expect("the flowing water's top");
        assert_eq!((flow.y, flow.len, flow.h, flow.mat), (66, 1, 1, 0xa2a2), "{flow:?}");
        // The lower water beyond: flat.
        assert_eq!(top_at(100, 64).map(|f| (f.y, f.mat)), Some((65, 0x2222)));
        // No wall at the step (the plane x = 64); the flowing water's lip,
        // from 65 to its corners, on its east side.
        assert!(!water.iter().any(|f| (f.dir == EAST || f.dir == WEST) && f.x == 64 && f.z == 64), "a wall at the step");
        assert!(water.iter().any(|f| f.dir == EAST && f.x == 65 && f.z == 64 && f.y == 65 && f.h == 1 && f.mat == 0xa2a2));
    }

    /// Ground at the bottom of the world (the deep sea of a map whose ocean
    /// floor is down there) leaves its columns no block. They were taken for
    /// the end of the map: no water over them, and the water around them
    /// walled down to its floor, a pit in the sea wherever every block was
    /// drawn. They hold the sea, over a block of their terrain.
    #[test]
    fn the_sea_over_ground_at_the_bottom_of_the_world() {
        use massif_core::coords::local_index;
        use massif_core::world::{Dimension, DimensionKind, WorldMeta};
        let mut w = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        // The sea at 63 over ground at 20, at the bottom of the world in the
        // middle of the tile, and under it beside.
        let mut tile = massif_core::tile::Tile::ground(20.0, TerrainId::ROCK.0, 63);
        for z in 32..96 {
            for x in 32..96 {
                tile.top.set(local_index(x, z), if x < 64 { -64.0 } else { -80.0 });
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        w.dims = vec![dim];
        let b = build_tile(&w.dims[0], &w.registry, 63.0, w.meta.seed, TilePos::new(0, 0), STYLE);
        let water = &b.faces.water;
        // The sea is level: no wall of water but at the end of the map.
        let at_the_end = |f: &Face| (f.dir == EAST && f.x == 128) || (f.dir == WEST && f.x == 0) || (f.dir == SOUTH && f.z == 128) || (f.dir == NORTH && f.z == 0);
        let wall = water.iter().find(|f| f.dir != UP && !at_the_end(f));
        assert!(wall.is_none(), "a wall of water in the sea: {wall:?}");
        // Its top over every column, the deep ones too.
        let top_at = |i: u8, j: u8| water.iter().find(|f| f.dir == UP && (f.x..f.x + f.len).contains(&i) && (f.z as i16..f.z as i16 + f.h).contains(&(j as i16))).map(|f| f.y);
        for (i, j) in [(10, 10), (40, 40), (63, 63), (64, 64), (95, 95), (120, 120)] {
            assert_eq!(top_at(i, j), Some(63), "the sea's top over {i} {j}");
        }
        // Under it, a block of floor at the bottom of the world, 126 blocks
        // deep, for the tracer.
        for c in [40 * 128 + 40, 80 * 128 + 80] {
            assert_eq!((b.heights[c], b.depths[c]), (1, 126));
        }
        assert_eq!((b.heights[10 * 128 + 10], b.depths[10 * 128 + 10]), (20 + 64, 43));
    }

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

    /// An arena's free ranges: taken first fit, given back joined to those
    /// beside them, so that it fills again without holes.
    #[test]
    fn arena_ranges_are_taken_and_given_back() {
        let mut free = FreeRanges(vec![(0, 100)]);
        let (a, b, c) = (free.take(10).unwrap(), free.take(20).unwrap(), free.take(30).unwrap());
        assert_eq!((a, b, c), (0, 10, 30));
        assert_eq!(free, FreeRanges(vec![(60, 40)]));
        free.give(b, 20);
        assert_eq!(free, FreeRanges(vec![(10, 20), (60, 40)]));
        assert_eq!(free.take(25), Some(60), "too large for the hole: after");
        free.give(a, 10);
        assert_eq!(free, FreeRanges(vec![(0, 30), (85, 15)]), "joined to the range after it");
        free.give(c, 30);
        assert_eq!(free, FreeRanges(vec![(0, 60), (85, 15)]), "and to the one before");
        free.give(60, 25);
        assert_eq!(free, FreeRanges(vec![(0, 100)]), "all of it again");
        assert_eq!(free.take(101), None);
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

    /// A map whose land steps up a block east of x = 0, stone south of
    /// z = 0 and grass north of it, its steps smoothed with slabs.
    pub(crate) fn stepped_world() -> World {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 3, ..GenParams::default() });
        let (stone, grass) = (w.registry.terrain_by_name("Stone").unwrap(), w.registry.terrain_by_name("Grass").unwrap());
        let positions: Vec<TilePos> = w.dims[0].ground().positions().collect();
        for pos in positions {
            let (ox, oz) = pos.origin();
            let tile = w.dims[0].ground_mut().tile_mut(pos).unwrap();
            for i in 0..massif_core::TILE_AREA {
                let (x, z) = (ox + (i % 128) as i32, oz + (i / 128) as i32);
                // Steps up east every 4 blocks from x = -24, and a notch.
                let step = ((x + 24).max(0) / 4).min(12) as f32;
                tile.top.set(i, 64.0 + step + if (x, z) == (-1, -6) { 1.0 } else { 0.0 });
                tile.fluid.set(i, massif_core::NO_FLUID);
                tile.terrain.set(i, if z < 0 { stone.0 } else { grass.0 });
                tile.cover.set(i, 0);
                tile.stacked.clear_kind(i, massif_core::layer_stack::Kind::Cover);
                tile.objects.set(i, 0);
            }
        }
        let mut h = massif_core::History::new(4);
        h.begin("slabs", 0);
        massif_core::shapes::fill_slabs(&mut h.edit(&mut w, 0), massif_core::shapes::value(None, true));
        h.commit();
        w
    }

    /// Smooth terrain shows in 3D: a stair (drawn three quarters high) at
    /// the foot of each step of stone, none on grass, which has no pieces
    /// in the game.
    #[test]
    fn slabs_show_half_a_block_high() {
        let w = stepped_world();
        let dim = &w.dims[0];
        let tile = TilePos::new(-1, -1);
        let b = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE);
        let tops: Vec<&Face> = b.faces.solid.iter().filter(|f| f.dir == part(UP, 6)).collect();
        // The foot of the step at x = -1 (its top at 69, x = 0.s at 70)…
        let (i, j) = ((-1 - tile.x * 128) as u8, (-10 - tile.z * 128) as u8);
        let foot = tops.iter().find(|f| f.x == i && f.z == j).expect("a slab at the foot of the step");
        assert_eq!(foot.y, 69);
        // …and a side half a block high toward the lower land west of it.
        assert!(b.faces.solid.iter().any(|f| f.dir == part(WEST, 6) && f.x == i && f.z == j && f.h == 1));
        // None on the flat land west of the steps.
        assert!(!tops.iter().any(|f| (f.x as i32 + tile.x * 128) < -25));
        // None on grass, north of z = 0.
        let b = build_tile(dim, &w.registry, 63.0, w.meta.seed, TilePos::new(-1, 0), STYLE);
        assert!(!b.faces.solid.iter().any(|f| f.dir >> 3 != 0), "grass has no pieces of its own");
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
        let grid = compose_area(dim, 0, -128, 128, 128, MapMode::Terrain, false, massif_core::compose::Caves::Openings, None);
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

    /// Annotations tint the tops of the columns they are painted on, in
    /// their colour, while shown; hidden, the tops are the ground's.
    #[test]
    fn annotations_tint_the_tops_in_3d() {
        use massif_core::generate::{GenParams, Landform, generate};
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Plains, seed: 5, ..GenParams::default() });
        let brush = massif_core::Brush { shape: massif_core::BrushShape::Circle, radius: 30.0, profile: massif_core::Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let mut h = massif_core::history::History::default();
        h.begin("mark", 0);
        massif_core::ops::paint_annotation(&mut h.edit(&mut w, 0), &brush, 64.0, 64.0, 11, 1);
        h.commit();
        let dim = &w.dims[0];
        let tile = TilePos::new(0, 0);
        let shown = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, Style { notes: Some((w.registry.annotations.fingerprint(), true)), ..STYLE });
        let hidden = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, STYLE);
        let at = |b: &TileBuild| b.pairs[b.colors[64 * 128 + 64] as usize].top;
        let (blue, ground) = (at(&shown), at(&hidden));
        assert!(blue[2] as i32 > ground[2] as i32 + 40, "blue over the land: {blue:?} against {ground:?}");
        assert_eq!(shown.pairs[shown.colors[5] as usize].top, hidden.pairs[hidden.colors[5] as usize].top, "only where painted");
    }

    /// The walls of the meshed columns close every step toward their
    /// neighbours: their area equals the height differences.
    #[test]
    fn walls_close_every_step() {
        let w = world();
        let dim = &w.dims[0];
        let tile = TilePos::new(0, -1);
        let b = build_tile(dim, &w.registry, 63.0, w.meta.seed, tile, Style { mode: MapMode::Height, water: false, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false });
        let grid = compose_area(dim, -1, -129, 130, 130, MapMode::Height, false, massif_core::compose::Caves::Openings, None);
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
            airs: Vec::new(),
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
        // A column the meshes draw keeps where its air is, for a ghost's
        // view from the ground, its height 0 for the tracer to go by.
        let span = |y0, y1| CellSpan { y0, y1, terrain: 0, top: y1 as f32, color: None, mat: 0 };
        let cave = air_bits(&[span(-64, 40), span(50, 70)], -64);
        assert_eq!((cave & HEIGHT_MASK, (cave >> HEIGHT_BITS) & 0x3ff, cave >> 22), (0, 104, 114));
        let overhang = air_bits(&[span(50, 70)], -64);
        assert_eq!(((overhang >> HEIGHT_BITS) & 0x3ff, overhang >> 22), (0, 114), "air from the bottom of the world");
        let b = TileBuild { airs: vec![0, cave, 0], ..b };
        assert_eq!(pairs.pack(&b)[1], cave);
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
        let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
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

    /// A device as the app asks for one, on the graphics API WGPU_BACKEND
    /// names (`gl` tries the views under OpenGL's limits).
    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).ok()
    }

    /// Put every tile of a world on the GPU.
    fn load(renderer: &mut Renderer3d, w: &World, style: Style) -> Vec<TilePos> {
        let dim = &w.dims[0];
        let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
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

    /// The tracer and the meshes on a huge map (MASSIF_HUGE_WORLD: a
    /// .massif), as the 3D view shows it: its grid built, then frames at
    /// 2,560 × 1,440 from above the whole map, from halfway and near the
    /// ground, the graphics card waited for. Best and middle of 16.
    #[test]
    fn a_huge_map_on_the_gpu() {
        use rayon::prelude::*;
        let Some(path) = std::env::var_os("MASSIF_HUGE_WORLD") else { return };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).unwrap();
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let dim = &world.dims[0];
        let mut renderer = Renderer3d::new(&device, &queue);
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
        renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).unwrap();
        let stride = shape.stride;
        let mut cells: Vec<TilePos> = dim.ground().positions().map(|t| grid_tile(t, stride)).collect();
        cells.sort_by_key(|t| (t.z, t.x));
        cells.dedup();
        if let Some(n) = std::env::var("MASSIF_HUGE_CELLS").ok().and_then(|n| n.parse::<usize>().ok()) {
            // Only building: n cells spread over the map, one after another.
            let step = (cells.len() / n).max(1);
            let some: Vec<TilePos> = cells.iter().copied().step_by(step).take(n).collect();
            let mut best = f32::MAX;
            for _ in 0..3 {
                let t = Instant::now();
                let faces: usize = some.iter().map(|&c| build_cell(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, c, stride, style).pairs.len()).sum();
                best = best.min(t.elapsed().as_secs_f32());
                eprintln!("{} cells: {:.2} ms a cell ({faces} pairs)", some.len(), t.elapsed().as_secs_f32() * 1000.0 / some.len() as f32);
            }
            eprintln!("best {:.2} ms a cell", best * 1000.0 / some.len() as f32);
            return;
        }
        let t = Instant::now();
        let built: Vec<(TilePos, TileBuild)> = cells.par_iter().map(|&c| (c, build_cell(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, c, stride, style))).collect();
        eprintln!("{} cells at stride {stride} built in {:.1} s", cells.len(), t.elapsed().as_secs_f32());
        let mut pairs = Pairs::default();
        for (c, b) in &built {
            renderer.upload_columns(*c, &pairs.pack(b));
            if stride == 1 {
                renderer.upload(*c, &b.faces);
            }
        }
        drop(built);
        renderer.flush();
        renderer.set_palette(pairs.list());
        let size = [2560u32, 1440];
        renderer.ensure_target(size);
        let (lo, hi) = dim.bounds().unwrap();
        let (x0, z0, x1, z1) = ((lo.x * 128) as f64, (lo.z * 128) as f64, ((hi.x + 1) * 128) as f64, ((hi.z + 1) * 128) as f64);
        let extent = (x1 - x0).max(z1 - z0);
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
        let tiles: Vec<TilePos> = if stride == 1 { cells.clone() } else { Vec::new() };
        for (name, distance, pitch) in [("whole map", extent * 0.85, None), ("halfway", extent * 0.2, Some(0.35f32)), ("near the ground", 400.0, Some(0.12f32))] {
            let mut camera = Camera { target: [cx, 300.0, cz], distance: distance as f32, ..Camera::default() };
            if let Some(p) = pitch {
                camera.pitch = p;
            }
            let mut times = Vec::new();
            for k in 0..20 {
                camera.yaw += 0.01;
                let origin = (camera.target[0].floor() as i32, camera.target[2].floor() as i32);
                let globals = camera.globals(origin, size[0] as f32 / size[1] as f32, extent as f32, Sky::flat([120, 160, 220]), size[1]);
                let started = Instant::now();
                renderer.render(&globals, origin, &tiles, dim.min_y, substrate, size);
                let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
                if k >= 4 {
                    times.push(started.elapsed().as_secs_f32() * 1000.0);
                }
            }
            times.sort_by(f32::total_cmp);
            eprintln!("{name}: best {:.1} ms, middle {:.1} ms a frame", times[0], times[times.len() / 2]);
        }
    }

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
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let dim = &world.dims[0];
        let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
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
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let reg = &world.registry;
        let n = TILE_SIZE as usize;
        let t = Instant::now();
        for _ in 0..10 {
            let standing = Standing { reg, seed: world.meta.seed, lava: false, objects: true };
            std::hint::black_box(compose_area(dim, -1, -1, n + 2, n + 2, style.mode, style.water, style.caves, Some(standing)));
        }
        eprintln!("compose a tile: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
        let t = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(build_tile(dim, reg, world.meta.sea_level as f32, world.meta.seed, tile, style));
        }
        eprintln!("build a tile: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
    }
}

/// How long the graphics card takes to draw the 3D view, in milliseconds:
/// a small map of hills (1,024 blocks a side) seen from above at 1,280 × 720
/// pixels, four samples a pixel, the best of six frames after two. Run once,
/// off the main thread, on the very first start to pick the graphics preset
/// (see `graphics::suggest`).
pub fn probe_ms(device: &wgpu::Device, queue: &wgpu::Queue) -> f32 {
    use massif_core::generate::{GenParams, Landform, generate};
    let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Hills, seed: 3, ..GenParams::default() });
    let dim = &world.dims[0];
    let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
    let mut renderer = Renderer3d::with_samples(device, queue, SAMPLES);
    let Some(shape) = GridShape::of(dim, 64) else { return 0.0 };
    if renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).is_err() {
        return 0.0;
    }
    let tiles: Vec<TilePos> = dim.ground().positions().collect();
    let built: Vec<(TilePos, TileBuild)> = tiles.par_iter().map(|&t| (t, build_tile(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, t, style))).collect();
    let mut pairs = Pairs::default();
    for (t, b) in &built {
        renderer.upload_columns(*t, &pairs.pack(b));
        renderer.upload(*t, &b.faces);
    }
    renderer.flush();
    renderer.set_palette(pairs.list());
    let size = [1280u32, 720];
    renderer.ensure_target(size);
    let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
    let mut camera = Camera { target: [512.0, 80.0, 512.0], distance: 700.0, pitch: 0.5, ..Camera::default() };
    let mut best = f32::MAX;
    for k in 0..8 {
        camera.yaw += 0.05;
        let origin = (512, 512);
        let globals = camera.globals(origin, 1280.0 / 720.0, 1024.0, Sky::flat([120, 160, 220]), size[1]);
        let started = Instant::now();
        renderer.render(&globals, origin, &tiles, dim.min_y, substrate, size);
        renderer.wait();
        if k >= 2 {
            best = best.min(started.elapsed().as_secs_f32() * 1000.0);
        }
    }
    best
}

/// What each setting of Settings ▸ Graphics costs the 3D view
/// (MASSIF_GRAPHICS_WORLD, a large .massif): the map built at each preset's
/// budget of pages, then frames at 2,560 × 1,440 over it, each setting
/// changed alone from High, and the player's view with and without the
/// game's textures and clouds. Prints the best and middle times and the
/// graphics card's memory.
#[cfg(test)]
mod graphics_perf {
    use super::*;
    use crate::graphics::{Graphics, Preset};

    fn frames(renderer: &mut Renderer3d, camera: &Camera, extent: f32, tiles: &[TilePos], bottom: i32, region: [u32; 2], tweak: &dyn Fn(&mut Globals)) -> (f32, f32) {
        let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
        let mut times = Vec::new();
        let mut camera = *camera;
        for k in 0..16 {
            camera.yaw += 0.01;
            let origin = (camera.target[0].floor() as i32, camera.target[2].floor() as i32);
            let mut globals = camera.globals(origin, 2560.0 / 1440.0, extent, Sky::flat([120, 160, 220]), region[1]);
            tweak(&mut globals);
            let started = Instant::now();
            // As the app does: only the chunks with faces, in view.
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let shown: Vec<TilePos> = tiles
                .iter()
                .copied()
                .filter(|t| renderer.chunks.contains_key(t))
                .filter(|t| {
                    let (x, z) = ((t.x * TILE_SIZE - origin.0) as f32, (t.z * TILE_SIZE - origin.1) as f32);
                    in_view(&m, Vec3::new(x, bottom as f32, z), Vec3::new(x + TILE_SIZE as f32, 320.0, z + TILE_SIZE as f32))
                })
                .collect();
            if k == 0 {
                eprint!("[{} chunks] ", shown.len());
            }
            renderer.render(&globals, origin, &shown, bottom, substrate, region);
            renderer.wait();
            if k >= 4 {
                times.push(started.elapsed().as_secs_f32() * 1000.0);
            }
        }
        times.sort_by(f32::total_cmp);
        (times[0], times[times.len() / 2])
    }

    #[test]
    fn graphics_settings_on_the_gpu() {
        use rayon::prelude::*;
        let Some(path) = std::env::var_os("MASSIF_GRAPHICS_WORLD") else { return };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        eprintln!("{:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).unwrap();
        let mut world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let cat = massif_assets::load_newest().and_then(Result::ok);
        if let Some(cat) = &cat {
            massif_assets::apply_colors(&mut world.registry, cat);
        }
        let dim = &world.dims[0];
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let (lo, hi) = dim.bounds().unwrap();
        let (x0, z0, x1, z1) = ((lo.x * 128) as f64, (lo.z * 128) as f64, ((hi.x + 1) * 128) as f64, ((hi.z + 1) * 128) as f64);
        let extent = (x1 - x0).max(z1 - z0);
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        let full = [2560u32, 1440];
        let only = std::env::var("MASSIF_GRAPHICS_PRESETS").unwrap_or_else(|_| "low,medium,high".into());
        for preset in Preset::NAMED {
            if !only.contains(&format!("{preset:?}").to_lowercase()) {
                continue;
            }
            let g = Graphics::of(preset);
            let shape = GridShape::of(dim, g.gpu_pages as usize).unwrap();
            let stride = shape.stride;
            let mut cells: Vec<TilePos> = dim.ground().positions().map(|t| grid_tile(t, stride)).collect();
            cells.sort_by_key(|t| (t.z, t.x));
            cells.dedup();
            let t = Instant::now();
            let built: Vec<(TilePos, TileBuild)> = cells.par_iter().map(|&c| (c, build_cell(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, c, stride, style))).collect();
            eprintln!("\n== {preset:?}: {} pages budget, stride {stride}, {} cells built in {:.1} s", g.gpu_pages, cells.len(), t.elapsed().as_secs_f32());
            let tiles: Vec<TilePos> = if stride == 1 { cells.clone() } else { Vec::new() };
            for samples in [4, 1] {
                let mut renderer = Renderer3d::with_samples(&device, &queue, samples);
                renderer.set_grid(stride, shape.page0, shape.pages, &shape.present).unwrap();
                let mut pairs = Pairs::default();
                for (c, b) in &built {
                    renderer.upload_columns(*c, &pairs.pack(b));
                    if stride == 1 {
                        renderer.upload(*c, &b.faces);
                    }
                }
                renderer.flush();
                renderer.set_palette(pairs.list());
                if let Some(cat) = &cat {
                    let atlas = crate::textures::atlas(&world.registry, cat, device.limits().max_texture_array_layers as usize);
                    renderer.set_look(&atlas, &crate::textures::clouds(Some(cat)));
                }
                renderer.ensure_target(full);
                eprintln!("-- {samples} samples a pixel: terrain {:.0} MB on the card, textures {:.2} MB, target {:.0} MB", renderer.bytes() as f64 / 1048576.0, renderer.look_bytes() as f64 / 1048576.0, (2560.0 * 1440.0 * (4.0 * samples as f64 + 4.0 * samples as f64 + 8.0)) / 1048576.0);
                let scales: &[f32] = if samples == 4 { &[1.0, 0.67, 0.5] } else { &[1.0, 0.67] };
                for (name, distance, pitch) in [("whole map", extent * 0.85, None), ("halfway", extent * 0.2, Some(0.35f32)), ("near the ground", 400.0, Some(0.12f32))] {
                    let mut camera = Camera { target: [cx, 300.0, cz], distance: distance as f32, ..Camera::default() };
                    if let Some(p) = pitch {
                        camera.pitch = p;
                    }
                    for &scale in scales {
                        let region = [(2560.0 * scale) as u32, (1440.0 * scale) as u32];
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, region, &|_| {});
                        eprintln!("{name}, scale {scale}: best {b:.1} ms, middle {m:.1} ms");
                    }
                    // The views' own look with the game's textures, a ray a
                    // pixel (moving) and, with four samples, a ray a sample
                    // (still).
                    let (materials, fluids) = renderer.look_info();
                    let wk = world.registry.well_known;
                    let textured = move |g: &mut Globals| {
                        g.look = [(materials > 0) as u8 as f32, 0.0, 0.0, materials as f32];
                        g.rock = [wk.stone.0 as f32, wk.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
                    };
                    for fine in if samples == 4 { &[false, true][..] } else { &[false][..] } {
                        set_fine(&mut renderer, *fine);
                        let still = if *fine { ", a ray a sample" } else { "" };
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, full, &|_| {});
                        eprintln!("{name}, flat{still}: best {b:.1} ms, middle {m:.1} ms");
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, full, &textured);
                        eprintln!("{name}, textures{still}: best {b:.1} ms, middle {m:.1} ms");
                    }
                    set_fine(&mut renderer, false);
                    if samples == 4 && name != "whole map" {
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, full, &|g| g.sun[3] = 0.0);
                        eprintln!("{name}, no reflections: best {b:.1} ms, middle {m:.1} ms");
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, full, &|g| g.mist[3] = 0.0);
                        eprintln!("{name}, no fog: best {b:.1} ms, middle {m:.1} ms");
                    }
                }
                // The player's view at a player's eyes in the middle.
                if stride == 1 {
                    let ground = dim.ground().view(TilePos::new((cx as i32).div_euclid(128), (cz as i32).div_euclid(128))).map_or(70.0, |v| v.top.get(local_index(cx as i32, cz as i32)));
                    let camera = Camera { target: [cx, ground as f64 + 1.62, cz], yaw: 0.6, pitch: 0.12, distance: 1.0 };
                    let (materials, fluids) = renderer.look_info();
                    let wk = world.registry.well_known;
                    for (label, textured, clouds, fog) in [("game, textures, clouds, fog", true, true, true), ("game, no textures", false, true, true), ("game, no clouds", true, false, true), ("game, no fog", true, true, false)] {
                        let look = move |g: &mut Globals| {
                            g.look = [(textured && materials > 0) as u8 as f32, 1.0, if clouds { CLOUDS_Y } else { 0.0 }, materials as f32];
                            if fog {
                                g.fog = [192.0, (extent as f32 * 0.45).max(1536.0), g.fog[2], g.fog[3]];
                            } else {
                                g.mist[3] = 0.0;
                            }
                            g.rock = [wk.stone.0 as f32, wk.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
                        };
                        let (b, m) = frames(&mut renderer, &camera, extent as f32, &tiles, dim.min_y, full, &look);
                        eprintln!("{label}: best {b:.1} ms, middle {m:.1} ms");
                    }
                }
            }
        }
    }

}

/// The Gradient brush seen in the 3D view, with the game's look and
/// textures: Conquest Reforged's blocks on hills, before and after. Runs
/// with MASSIF_SNAPSHOT_DIR and MASSIF_CONQUEST_JAR (the mod's jar), on a
/// graphics card, where Minecraft is installed.
#[cfg(test)]
mod gradient_shots {
    use super::*;
    use crate::gradient_ui::shots;
    use massif_core::generate::{GenParams, Landform, generate};
    use massif_core::history::History;
    use massif_core::ops::{self, Surface};
    use massif_core::{Brush, BrushShape, Profile, World};

    /// The world drawn as the game's look draws it, from `camera`.
    fn game_look(device: &wgpu::Device, queue: &wgpu::Queue, world: &World, cat: &massif_assets::Catalogue, camera: Camera, size: [u32; 2]) -> image::RgbImage {
        let mut renderer = Renderer3d::new(device, queue);
        let style = Style { mode: MapMode::Terrain, water: true, lava: false, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
        let dim = &world.dims[0];
        let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
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
        let atlas = crate::textures::atlas(&world.registry, cat, most);
        renderer.set_look(&atlas, &crate::textures::clouds(Some(cat)));
        let (materials, fluids) = renderer.look_info();
        renderer.ensure_target(size);
        let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
        let origin = (camera.target[0].floor() as i32, camera.target[2].floor() as i32);
        let mut globals = camera.globals(origin, size[0] as f32 / size[1] as f32, 512.0, Sky::GAME, size[1]);
        globals.look = [1.0, 1.0, CLOUDS_Y, materials as f32];
        globals.fog = [192.0, 1536.0, globals.fog[2], globals.fog[3]];
        globals.rock = [world.registry.well_known.stone.0 as f32, world.registry.well_known.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
        renderer.render(&globals, origin, &tiles, dim.min_y, substrate, size);
        renderer.wait();
        let (w, h, rgba) = renderer.read_image().expect("the image");
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, rgba).unwrap()).to_rgb8()
    }

    fn edit(world: &mut World, f: impl FnOnce(&mut massif_core::Edit)) {
        let mut h = History::default();
        h.begin("shot", 0);
        f(&mut h.edit(world, 0));
        h.commit();
    }

    /// Smooth terrain on hills, off and on, in the game's look, at 1× and
    /// 2× (MASSIF_SNAPSHOT_DIR; needs Minecraft, and with MASSIF_CONQUEST_JAR
    /// the grass takes Conquest Reforged's layers): grass west, stone east.
    #[test]
    fn smooth_terrain_in_3d_renders() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let Some((vanilla, modded)) = shots::catalogues() else {
            eprintln!("needs Minecraft: skipping");
            return;
        };
        let cat = modded.unwrap_or(vanilla);
        let shared = std::sync::Arc::new(cat.clone());
        crate::blocks::note_catalogue(Some(&shared));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 21, ..GenParams::default() });
        massif_assets::apply_colors(&mut world.registry, &cat);
        let (grass, stone) = (world.registry.terrain_by_name("Grass").unwrap(), world.registry.terrain_by_name("Stone").unwrap());
        let half = Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Square, Profile::Constant, 64.0) };
        edit(&mut world, |e| {
            ops::paint_terrain(e, &half, -64.0, 0.0, grass, 1);
            ops::paint_terrain(e, &half, 64.0, 0.0, stone, 2);
        });
        let h = ops::height_at(&world.dims[0], 0, 0, 0, Surface::Top).unwrap_or(70.0) as f64;
        let camera = Camera { target: [0.0, h, 0.0], yaw: 0.75, pitch: 0.45, distance: 46.0 };
        let tag = if cat.mods.is_empty() { "vanilla" } else { "conquest" };
        for (scale, ppp) in [(1, "1x"), (2, "2x")] {
            let size = [1200 * scale, 760 * scale];
            set_smooth(&massif_core::shapes::SmoothSettings::default());
            game_look(&device, &queue, &world, &cat, camera, size).save(dir.join(format!("3d-smooth-{tag}-off-{ppp}.png"))).unwrap();
            set_smooth(&massif_core::shapes::SmoothSettings { on: true, ..Default::default() });
            game_look(&device, &queue, &world, &cat, camera, size).save(dir.join(format!("3d-smooth-{tag}-on-{ppp}.png"))).unwrap();
        }
        set_smooth(&massif_core::shapes::SmoothSettings::default());
    }

    /// Smooth terrain over Conquest Reforged's ground, on a slope
    /// (MASSIF_SNAPSHOT_DIR, Minecraft and MASSIF_CONQUEST_JAR): the game's
    /// grass with meadow grass growing on half of it, the game's stone
    /// (Conquest's limestone), its dirt (Loamy Dirt) and Conquest's brown
    /// moss, which has a slab by eighths and no layers. Off; on, the shapes
    /// found by name only (before); on, with the mod's recipes (after).
    #[test]
    fn smooth_conquest_families_in_3d_renders() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let Some((_, Some(cat))) = shots::catalogues() else {
            eprintln!("needs Minecraft and MASSIF_CONQUEST_JAR: skipping");
            return;
        };
        let shared = std::sync::Arc::new(cat.clone());
        crate::blocks::note_catalogue(Some(&shared));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 21, ..GenParams::default() });
        let mut terrain = |id: &str| {
            let reg = &mut world.registry;
            let b = reg.intern_block(id, [128, 128, 128]);
            reg.terrains.push(massif_core::registry::TerrainDef { name: id.into(), group: "Test".into(), color: [128, 128, 128], strata: vec![massif_core::registry::Stratum { blocks: vec![(b, 1)], depth: (3, 5), mix: massif_core::registry::Mix::Grain }], submerged_top: None, biome_hint: None });
            (reg.terrains.len() - 1) as u16
        };
        let (dirt, moss) = (terrain("minecraft:dirt"), terrain("conquest:brown_moss"));
        let (grass, stone) = (world.registry.terrain_by_name("Grass").unwrap().0, world.registry.terrain_by_name("Stone").unwrap().0);
        let meadow = world.registry.add_cover(massif_core::registry::CoverDef::new("Meadow grass", [70, 130, 50], &[("conquest:common_meadow_grass[layers=8]", 1)], 0.6));
        massif_assets::apply_colors(&mut world.registry, &cat);
        let quarter = Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Square, Profile::Constant, 32.0) };
        edit(&mut world, |e| {
            ops::paint_terrain(e, &quarter, -32.0, -32.0, TerrainId(grass), 1);
            ops::paint_terrain(e, &quarter, 32.0, -32.0, TerrainId(stone), 2);
            ops::paint_terrain(e, &quarter, -32.0, 32.0, TerrainId(dirt), 3);
            ops::paint_terrain(e, &quarter, 32.0, 32.0, TerrainId(moss), 4);
        });
        edit(&mut world, |e| ops::paint_cover(e, &Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Square, Profile::Constant, 16.0) }, -48.0, -32.0, meadow, 5));
        let h = ops::height_at(&world.dims[0], 0, 0, 0, Surface::Top).unwrap_or(70.0) as f64;
        let camera = Camera { target: [0.0, h, 0.0], yaw: 0.75, pitch: 0.5, distance: 70.0 };
        let families = crate::blocks::families();
        for (scale, ppp) in [(1, "1x"), (2, "2x")] {
            let size = [1200 * scale, 760 * scale];
            set_smooth(&massif_core::shapes::SmoothSettings::default());
            game_look(&device, &queue, &world, &cat, camera, size).save(dir.join(format!("3d-conquest-families-off-{ppp}.png"))).unwrap();
            set_smooth(&massif_core::shapes::SmoothSettings { on: true, ..Default::default() });
            crate::blocks::set_families(Default::default());
            game_look(&device, &queue, &world, &cat, camera, size).save(dir.join(format!("3d-conquest-families-by-name-{ppp}.png"))).unwrap();
            crate::blocks::set_families(families.clone());
            set_smooth(&massif_core::shapes::SmoothSettings { on: false, ..Default::default() });
            set_smooth(&massif_core::shapes::SmoothSettings { on: true, ..Default::default() });
            game_look(&device, &queue, &world, &cat, camera, size).save(dir.join(format!("3d-conquest-families-recipes-{ppp}.png"))).unwrap();
        }
        set_smooth(&massif_core::shapes::SmoothSettings::default());
    }

    #[test]
    fn the_gradient_brush_on_hills_in_3d() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let Some((_, Some(cat))) = shots::catalogues() else {
            eprintln!("needs Minecraft and MASSIF_CONQUEST_JAR: skipping");
            return;
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let size = [1600, 1000];
        for (file, g, around, patch) in shots::conquest() {
            let pit = file == "conquest-dark-pit-two-greys";
            // Rock on the hills: the rim's example and each transition alone.
            let rocks = matches!(file, "conquest-rock-gold-green-rim" | "conquest-hard-cut" | "conquest-noise" | "conquest-blobs");
            let mut world = generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: Landform::Hills, seed: 21, ..GenParams::default() });
            massif_assets::apply_colors(&mut world.registry, &cat);
            for name in g.terrains() {
                shots::terrain(&mut world.registry, &cat, name);
            }
            let (around, patch) = (shots::terrain(&mut world.registry, &cat, around), shots::terrain(&mut world.registry, &cat, patch));
            let ground = Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Square, Profile::Constant, 300.0) };
            edit(&mut world, |e| ops::paint_terrain(e, &ground, 0.0, 0.0, around, 1));
            if pit {
                // A pit dug in the hills, its floor dark.
                edit(&mut world, |e| {
                    ops::raise(e, &Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, 34.0) }, 0.0, 0.0, -12.0, Surface::Top);
                    ops::paint_terrain(e, &shots::patch_brush(22.0, 5), 0.0, 0.0, patch, 1);
                });
            } else if rocks {
                // Rock where the hills are steep, and a few patches by hand.
                let steep = massif_core::filter::Filter { slope_above: true, slope: 24.0, ..Default::default() };
                edit(&mut world, |e| {
                    ops::paint_terrain(e, &Brush { filter: Some(std::sync::Arc::new(steep)), strength: 1.0, ..Brush::symmetric(BrushShape::Circle, Profile::Constant, 110.0) }, 0.0, 0.0, patch, 1);
                    for (k, (x, z)) in [(-30.0, 20.0), (35.0, -25.0), (10.0, 40.0)].into_iter().enumerate() {
                        ops::paint_terrain(e, &shots::patch_brush(16.0, 30 + k as u64), x, z, patch, 1);
                    }
                });
            } else {
                edit(&mut world, |e| ops::paint_terrain(e, &shots::patch_brush(36.0, 9), 0.0, 0.0, patch, 1));
            }
            // The whole scene, or close on one patch for the transitions.
            let close = matches!(file, "conquest-hard-cut" | "conquest-noise" | "conquest-blobs");
            let at = if close { (35.0, -25.0) } else { (0.0, 0.0) };
            let h = ops::height_at(&world.dims[0], 0, at.0 as i32, at.1 as i32, Surface::Top).unwrap_or(70.0) as f64;
            let camera = match (pit, close) {
                (true, _) => Camera { target: [0.0, h, 0.0], yaw: 0.75, pitch: 0.72, distance: 95.0 },
                (_, true) => Camera { target: [at.0, h, at.1], yaw: 0.75, pitch: 0.62, distance: 62.0 },
                _ => Camera { target: [0.0, h, 0.0], yaw: 0.75, pitch: 0.6, distance: 150.0 },
            };
            let before = game_look(&device, &queue, &world, &cat, camera, size);
            shots::paint_over(&mut world, &g, (0.0, 0.0), 130.0);
            let after = game_look(&device, &queue, &world, &cat, camera, size);
            before.save(dir.join(format!("3d-{file}-before.png"))).unwrap();
            after.save(dir.join(format!("3d-{file}-after.png"))).unwrap();
            // Before over after, to compare at a glance.
            let mut both = image::RgbImage::from_pixel(size[0], size[1] * 2 + 16, image::Rgb([250, 250, 250]));
            image::imageops::replace(&mut both, &before, 0, 0);
            image::imageops::replace(&mut both, &after, 0, (size[1] + 16) as i64);
            both.save(dir.join(format!("3d-{file}.png"))).unwrap();
        }
    }
}

/// Trace a ray through each sample of a pixel, or one through its middle.
#[cfg(test)]
fn set_fine(renderer: &mut Renderer3d, on: bool) {
    renderer.fine = on;
}

#[cfg(test)]
#[path = "view3d_aa.rs"]
mod aa;

#[cfg(test)]
#[path = "view3d_textures.rs"]
mod textured;
