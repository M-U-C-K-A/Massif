//! Making the map larger or smaller by whole tiles, and filling the tiles
//! added: sea, flat land, land generated anew, or the land of the map
//! continued past its edges. Whatever the new land is, it starts as the
//! map's edge and eases into it over a band of blocks, so the map ends in
//! no cliff (see `docs/map-size.md`).
//!
//! The land past the edges is worked out on a grid of cells of
//! [`CELL`] blocks, kept only over the tiles that matter: the tiles added,
//! and the map's tiles along its edges. Each cell added knows the nearest
//! cell of the edge (a search out from the edge, nearest first), and the
//! continued height is the edge's heights spread outward as a stretched
//! membrane would (the mean of its neighbours, the edge held), which keeps
//! the edge where it is near it and smooths it farther out.

use crate::coords::{BlockRect, TILE_AREA, TILE_SIZE, TilePos};
use crate::generate::Theme;
use crate::noise::Noise;
use crate::registry::Registry;
use crate::tile::{NO_FLUID, Tile};
use crate::world::Dimension;
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

/// The side of a cell of the grid the land past the edges is worked out
/// on, in blocks.
pub const CELL: i32 = 8;
const CELLS: i32 = TILE_SIZE / CELL;
const CELLS_PER_TILE: usize = (CELLS * CELLS) as usize;
/// The rounds of averaging that spread the edge's heights outward.
const MEMBRANE_ROUNDS: usize = 160;
/// Within this many blocks of the map, the land added starts from the very
/// column of the edge nearest, not from the cells.
const SEAM: f32 = 24.0;

/// Tiles added (positive) or taken away (negative) on each side of the map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sides {
    pub west: i32,
    pub east: i32,
    pub north: i32,
    pub south: i32,
}

impl Sides {
    pub fn is_zero(&self) -> bool {
        *self == Sides::default()
    }
}

/// The tiles of a map of `tiles` grown or cropped by `sides`: its tiles
/// within the new bounds, and every tile of the bounds outside the old
/// ones. A map without tiles stays without.
pub fn sided(tiles: &HashSet<TilePos>, sides: Sides) -> HashSet<TilePos> {
    let Some((lo, hi)) = bounds(tiles) else { return HashSet::new() };
    let (x0, x1) = (lo.x - sides.west, hi.x + sides.east);
    let (z0, z1) = (lo.z - sides.north, hi.z + sides.south);
    let mut out = HashSet::new();
    for z in z0..=z1 {
        for x in x0..=x1 {
            let p = TilePos::new(x, z);
            let outside = x < lo.x || x > hi.x || z < lo.z || z > hi.z;
            if outside || tiles.contains(&p) {
                out.insert(p);
            }
        }
    }
    out
}

/// Inclusive bounds of a set of tiles.
pub fn bounds(tiles: &HashSet<TilePos>) -> Option<(TilePos, TilePos)> {
    let mut it = tiles.iter();
    let first = *it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (TilePos::new(lo.x.min(p.x), lo.z.min(p.z)), TilePos::new(hi.x.max(p.x), hi.z.max(p.z)))))
}

/// What the tiles added are made of, past the band where they ease out of
/// the map's edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    /// Sea, `depth` blocks deep under the water level.
    Sea { depth: f32 },
    /// Flat land at `height`.
    Flat { height: f32 },
    /// The land of the map continued past its edges: their heights,
    /// water, terrains and biomes, smoothed farther out, with some relief
    /// of their own.
    Continue,
    /// Land from the generator of new maps (Hills), seeded by the map.
    Generated,
    /// The land around the map given to [`apply`] (the world Minecraft
    /// makes for a seed): its heights, water and biomes.
    World,
}

/// The land around the map, wherever it comes from (generated, or the
/// world Minecraft makes past the map): what [`fill`] eases the map into.
pub trait Surroundings: Sync {
    /// The surface elevation at column `(x, z)`.
    fn height(&self, x: i32, z: i32) -> f32;
    /// The fluid's surface over the column, if it is under water.
    fn fluid(&self, _x: i32, _z: i32) -> Option<f32> {
        None
    }
    /// The biome there (0: the export's choice).
    fn biome(&self, _x: i32, _z: i32) -> u16 {
        0
    }
}

/// How the tiles added are filled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FillSettings {
    pub fill: Fill,
    /// Blocks over which the land added eases from the map's edge into
    /// what [`FillSettings::fill`] makes.
    pub ease: f32,
    /// The water level of the map.
    pub sea_level: i32,
    /// The map's fluid is lava.
    pub lava: bool,
    pub seed: i64,
    /// The build limits of the dimension.
    pub min_y: i32,
    pub max_y: i32,
}

/// What [`apply`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Applied {
    pub added: usize,
    pub removed: usize,
    pub cancelled: bool,
}

/// Make the map of `dim` the tiles of `target`: its tiles outside
/// dropped (every level's, and the sculpted chunks there), the tiles of
/// `target` it has not filled by `settings`. The selection is dropped if
/// tiles are.
pub fn apply(dim: &mut Dimension, target: &HashSet<TilePos>, settings: &FillSettings, reg: &Registry, around: Option<&dyn Surroundings>, cancel: &AtomicBool) -> Applied {
    let added: Vec<TilePos> = target.iter().copied().filter(|p| !dim.ground().contains(*p)).collect();
    let removed = dim.ground().positions().filter(|p| !target.contains(p)).count();
    let kept: HashSet<TilePos> = dim.ground().positions().filter(|p| target.contains(p)).collect();
    let tiles = fill(dim, &kept, &added, settings, reg, around, cancel);
    if cancel.load(Ordering::Relaxed) {
        return Applied { cancelled: true, ..Applied::default() };
    }
    dim.keep_tiles(target);
    let ground = dim.ground_mut();
    for (pos, tile) in tiles {
        ground.insert(pos, tile);
    }
    Applied { added: added.len(), removed, cancelled: false }
}

/// The columns of a tile of the map along its edge: its first and last
/// rows and columns.
struct Rim {
    /// North (z = 0), south, west (x = 0) and east edges, 128 columns each.
    sides: [[Column; TILE_SIZE as usize]; 4],
}

#[derive(Debug, Clone, Copy, Default)]
struct Column {
    top: f32,
    fluid: i16,
    terrain: u16,
    biome: u16,
}

impl Column {
    fn wet(&self) -> Option<f32> {
        (self.fluid != NO_FLUID && self.fluid as f32 > self.top).then_some(self.fluid as f32)
    }
}

/// A cell of the map's edge: what the land added starts from.
#[derive(Debug, Clone, Copy, Default)]
struct Source {
    height: f32,
    fluid: Option<f32>,
    terrain: u16,
    biome: u16,
    /// How much the land rises and falls about there, in blocks: the
    /// relief the continued land takes on.
    relief: f32,
}

/// The cells kept: the tiles added and the map's tiles along its edges.
struct Grid {
    tiles: HashMap<TilePos, usize>,
    order: Vec<TilePos>,
}

impl Grid {
    fn new(tiles: impl IntoIterator<Item = TilePos>) -> Self {
        let order: Vec<TilePos> = tiles.into_iter().collect();
        Self { tiles: order.iter().enumerate().map(|(i, p)| (*p, i)).collect(), order }
    }

    fn len(&self) -> usize {
        self.order.len() * CELLS_PER_TILE
    }

    /// The cell at cell coordinates `(cx, cz)`, if kept.
    #[inline]
    fn at(&self, cx: i32, cz: i32) -> Option<usize> {
        let t = self.tiles.get(&TilePos::new(cx.div_euclid(CELLS), cz.div_euclid(CELLS)))?;
        Some(t * CELLS_PER_TILE + (cz.rem_euclid(CELLS) * CELLS + cx.rem_euclid(CELLS)) as usize)
    }

    /// The cell coordinates of cell `i`.
    #[inline]
    fn coords(&self, i: usize) -> (i32, i32) {
        let p = self.order[i / CELLS_PER_TILE];
        let k = (i % CELLS_PER_TILE) as i32;
        (p.x * CELLS + k % CELLS, p.z * CELLS + k / CELLS)
    }
}

/// The ground tiles that fill `added`, the map being the tiles `kept` of
/// `dim`.
pub fn fill(dim: &Dimension, kept: &HashSet<TilePos>, added: &[TilePos], s: &FillSettings, reg: &Registry, around: Option<&dyn Surroundings>, cancel: &AtomicBool) -> Vec<(TilePos, Tile)> {
    if added.is_empty() {
        return Vec::new();
    }
    let added_set: HashSet<TilePos> = added.iter().copied().collect();
    let neighbours = |p: TilePos| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| p.offset(dx, dz))).filter(move |q| *q != p);
    // The map's tiles along an edge.
    let edge: Vec<TilePos> = kept.iter().copied().filter(|p| neighbours(*p).any(|q| !kept.contains(&q))).collect();
    let rims: HashMap<TilePos, Rim> = edge.par_iter().filter_map(|&p| Some((p, rim(&*dim.ground().view(p)?)))).collect();
    let grid = Grid::new(added.iter().copied().chain(edge.iter().copied()));
    let n = grid.len();

    // The edge cells, and how much the land about them rises and falls.
    let mut sources: Vec<Option<Source>> = vec![None; n];
    let edge_cells: Vec<(usize, Source)> = edge
        .par_iter()
        .filter_map(|&p| Some((p, dim.ground().view(p)?)))
        .flat_map_iter(|(p, tile)| {
            let base = grid.tiles[&p] * CELLS_PER_TILE;
            (0..CELLS_PER_TILE).map(move |k| (base + k, cell_source(&tile, k)))
        })
        .collect();
    for (i, src) in edge_cells {
        sources[i] = Some(src);
    }
    let relief: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| {
            let Some(src) = sources[i] else { return 0.0 };
            let (cx, cz) = grid.coords(i);
            let (mut lo, mut hi) = (src.height, src.height);
            for dz in -2..=2 {
                for dx in -2..=2 {
                    if let Some(h) = grid.at(cx + dx, cz + dz).and_then(|j| sources[j]).map(|s| s.height) {
                        lo = lo.min(h);
                        hi = hi.max(h);
                    }
                }
            }
            hi - lo
        })
        .collect();
    for (src, r) in sources.iter_mut().zip(relief) {
        if let Some(src) = src {
            src.relief = r;
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    // The nearest edge cell to every cell, nearest first.
    let mut nearest: Vec<u32> = vec![u32::MAX; n];
    let mut dist: Vec<f32> = vec![f32::INFINITY; n];
    let mut heap = BinaryHeap::new();
    for (i, src) in sources.iter().enumerate() {
        if src.is_some() {
            nearest[i] = i as u32;
            dist[i] = 0.0;
            heap.push(Reverse((0u32, i as u32)));
        }
    }
    while let Some(Reverse((d, i))) = heap.pop() {
        let i = i as usize;
        if (d as f32) > dist[i] * 1024.0 + 0.5 {
            continue;
        }
        let (cx, cz) = grid.coords(i);
        let (sx, sz) = grid.coords(nearest[i] as usize);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let Some(j) = grid.at(cx + dx, cz + dz) else { continue };
            let (ex, ez) = ((cx + dx - sx) as f32, (cz + dz - sz) as f32);
            let dj = (ex * ex + ez * ez).sqrt();
            if dj + 1e-4 < dist[j] {
                dist[j] = dj;
                nearest[j] = nearest[i];
                heap.push(Reverse(((dj * 1024.0) as u32, j as u32)));
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    // The edge's heights spread outward: each cell added the mean of its
    // neighbours, the edge cells held, the outer border free.
    let src_of = |i: usize| (nearest[i] != u32::MAX).then(|| sources[nearest[i] as usize].unwrap_or_default());
    let mut field: Vec<f32> = (0..n).map(|i| src_of(i).map_or(s.sea_level as f32, |s| s.height)).collect();
    let links: Vec<[u32; 4]> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (cx, cz) = grid.coords(i);
            let mut l = [u32::MAX; 4];
            for (k, (dx, dz)) in [(-1, 0), (1, 0), (0, -1), (0, 1)].into_iter().enumerate() {
                if let Some(j) = grid.at(cx + dx, cz + dz) {
                    l[k] = j as u32;
                }
            }
            l
        })
        .collect();
    for _ in 0..MEMBRANE_ROUNDS {
        let next: Vec<f32> = (0..n)
            .into_par_iter()
            .map(|i| {
                if sources[i].is_some() {
                    return field[i];
                }
                let (sum, count) = links[i].iter().filter(|&&j| j != u32::MAX).fold((0.0, 0), |(s, c), &j| (s + field[j as usize], c + 1));
                if count == 0 { field[i] } else { sum / count as f32 }
            })
            .collect();
        field = next;
    }
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }

    let ctx = Context {
        grid: &grid,
        field: &field,
        dist: &dist,
        nearest: &nearest,
        sources: &sources,
        rims: &rims,
        kept,
        settings: s,
        theme: Theme::new(reg),
        detail: Noise::new(s.seed as u64 ^ 0x6578_7061_6e64),
        generated: matches!(s.fill, Fill::Generated).then(|| {
            crate::generate::HeightField::new(&crate::generate::GenParams { seed: s.seed, sea_level: s.sea_level, ..Default::default() })
        }),
        around,
    };
    let _ = added_set;
    added.par_iter().map(|&pos| (pos, ctx.tile(pos))).collect()
}

/// The source a cell `k` of an edge tile gives: its mean height, its
/// water, its middle column's terrain and biome.
fn cell_source(tile: &Tile, k: usize) -> Source {
    let (cx, cz) = ((k as i32 % CELLS) * CELL, (k as i32 / CELLS) * CELL);
    let (mut sum, mut fluid, mut wet) = (0.0, f32::MIN, 0);
    for z in cz..cz + CELL {
        for x in cx..cx + CELL {
            let i = (z * TILE_SIZE + x) as usize;
            let top = tile.top.get(i);
            sum += top;
            let f = tile.fluid.get(i);
            if f != NO_FLUID && f as f32 > top {
                fluid = fluid.max(f as f32);
                wet += 1;
            }
        }
    }
    let mid = ((cz + CELL / 2) * TILE_SIZE + cx + CELL / 2) as usize;
    let count = (CELL * CELL) as f32;
    Source { height: sum / count, fluid: (wet * 4 >= CELL * CELL).then_some(fluid), terrain: tile.terrain.get(mid), biome: tile.biome.get(mid), relief: 0.0 }
}

fn rim(tile: &Tile) -> Rim {
    let at = |x: i32, z: i32| {
        let i = (z * TILE_SIZE + x) as usize;
        Column { top: tile.top.get(i), fluid: tile.fluid.get(i), terrain: tile.terrain.get(i), biome: tile.biome.get(i) }
    };
    let last = TILE_SIZE - 1;
    Rim { sides: [std::array::from_fn(|x| at(x as i32, 0)), std::array::from_fn(|x| at(x as i32, last)), std::array::from_fn(|z| at(0, z as i32)), std::array::from_fn(|z| at(last, z as i32))] }
}

struct Context<'a> {
    grid: &'a Grid,
    field: &'a [f32],
    dist: &'a [f32],
    nearest: &'a [u32],
    sources: &'a [Option<Source>],
    rims: &'a HashMap<TilePos, Rim>,
    kept: &'a HashSet<TilePos>,
    settings: &'a FillSettings,
    theme: Theme,
    detail: Noise,
    generated: Option<crate::generate::HeightField>,
    around: Option<&'a dyn Surroundings>,
}

/// One column added, before its terrain is chosen.
#[derive(Clone, Copy)]
struct Made {
    height: f32,
    fluid: Option<f32>,
    /// The edge column or cell it starts from.
    edge: Column,
    edge_wet: bool,
    /// Blocks from the map's edge.
    distance: f32,
    biome: u16,
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Context<'_> {
    /// The cell value at block column `(x, z)`, bilinear between cell
    /// centres, from cells kept only.
    fn sample(&self, values: &[f32], x: f32, z: f32) -> Option<f32> {
        let (gx, gz) = (x / CELL as f32 - 0.5, z / CELL as f32 - 0.5);
        let (x0, z0) = (gx.floor() as i32, gz.floor() as i32);
        let (fx, fz) = (gx - x0 as f32, gz - z0 as f32);
        let (mut sum, mut weight) = (0.0, 0.0);
        for (dx, dz, w) in [(0, 0, (1.0 - fx) * (1.0 - fz)), (1, 0, fx * (1.0 - fz)), (0, 1, (1.0 - fx) * fz), (1, 1, fx * fz)] {
            if let Some(i) = self.grid.at(x0 + dx, z0 + dz)
                && values[i].is_finite()
            {
                sum += values[i] * w;
                weight += w;
            }
        }
        (weight > 1e-6).then(|| sum / weight)
    }

    /// The column of the map's edge nearest to `(x, z)` (a column off the
    /// map), and how far it is, looking at the tiles about it.
    fn edge_column(&self, x: i32, z: i32) -> Option<(Column, f32)> {
        let here = TilePos::containing(x, z);
        let mut best: Option<(Column, f32)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let p = here.offset(dx, dz);
                if !self.kept.contains(&p) {
                    continue;
                }
                let Some(rim) = self.rims.get(&p) else { continue };
                let r = p.rect();
                let (px, pz) = (x.clamp(r.x0, r.x1 - 1), z.clamp(r.z0, r.z1 - 1));
                let d = (((x - px) * (x - px) + (z - pz) * (z - pz)) as f32).sqrt();
                if best.is_some_and(|b| b.1 <= d) {
                    continue;
                }
                let (lx, lz) = ((px - r.x0) as usize, (pz - r.z0) as usize);
                let last = TILE_SIZE as usize - 1;
                let col = if lz == 0 {
                    rim.sides[0][lx]
                } else if lz == last {
                    rim.sides[1][lx]
                } else if lx == 0 {
                    rim.sides[2][lz]
                } else {
                    rim.sides[3][lz]
                };
                best = Some((col, d));
            }
        }
        best
    }

    fn column(&self, x: i32, z: i32) -> Made {
        let s = self.settings;
        let (fx, fz) = (x as f32 + 0.5, z as f32 + 0.5);
        let cell = self.grid.at(x.div_euclid(CELL), z.div_euclid(CELL));
        let src = cell.and_then(|i| (self.nearest[i] != u32::MAX).then(|| self.sources[self.nearest[i] as usize]).flatten()).unwrap_or(Source {
            height: s.sea_level as f32,
            fluid: None,
            terrain: 0,
            biome: 0,
            relief: 0.0,
        });
        let far = self.sample(self.dist, fx, fz).unwrap_or(f32::INFINITY) * CELL as f32;
        let exact = self.edge_column(x, z);
        let d = match exact {
            Some((_, d)) if d < 2.0 * TILE_SIZE as f32 => d,
            _ => far,
        };
        // The edge continued: the membrane, some relief growing with the
        // distance, and near the edge the edge's own column.
        let membrane = self.sample(self.field, fx, fz).unwrap_or(src.height);
        let relief = (src.relief * 0.35).clamp(1.5, 18.0);
        let noise = self.detail.fbm(fx as f64 / 96.0, fz as f64 / 96.0, 4, 2.0, 0.5) as f32;
        let mut continued = membrane + noise * relief * smoothstep(d / 64.0);
        let mut edge = Column { top: src.height, fluid: src.fluid.map_or(NO_FLUID, |f| f as i16), terrain: src.terrain, biome: src.biome };
        if let Some((col, de)) = exact
            && de < SEAM
        {
            continued = col.top + (continued - col.top) * smoothstep(de / SEAM);
            edge = col;
        }
        let edge_wet = edge.wet().is_some() || src.fluid.is_some();
        let edge_fluid = edge.wet().or(src.fluid);
        let eased = if matches!(s.fill, Fill::Continue) { 0.0 } else { smoothstep(d / s.ease.max(1.0)) };
        let sea = s.sea_level as f32;
        let (target, target_fluid, target_biome) = match s.fill {
            Fill::Continue => (continued, None, 0),
            Fill::Sea { depth } => (sea - depth.max(1.0), Some(sea), 0),
            Fill::Flat { height } => (height, None, 0),
            Fill::Generated => match &self.generated {
                Some(g) => (g.height(fx as f64, fz as f64), None, 0),
                None => (continued, None, 0),
            },
            Fill::World => match self.around {
                Some(a) => (a.height(x, z), a.fluid(x, z), a.biome(x, z)),
                None => (continued, None, 0),
            },
        };
        let height = continued + (target - continued) * eased;
        // Water: the edge's own near it, then the land's past it; the sea
        // level wherever the ground dips under it, but along an edge of
        // lakes alone.
        let sea_fluid = (!matches!(s.fill, Fill::Continue) || edge_fluid.is_some_and(|f| (f - sea).abs() < 0.5)).then_some(sea);
        let level = if eased < 0.5 { edge_fluid.or(target_fluid).or(sea_fluid) } else { target_fluid.or(sea_fluid).or(if matches!(s.fill, Fill::Continue) { edge_fluid } else { None }) };
        let fluid = level.filter(|l| height < *l);
        // The biomes hand over along a wavering line halfway through the
        // band.
        let waver = 0.18 * self.detail.get(fx as f64 / 40.0 + 91.7, fz as f64 / 40.0 - 13.3) as f32;
        let biome = if eased + waver < 0.5 { edge.biome } else { target_biome };
        Made { height, fluid, edge, edge_wet, distance: d, biome }
    }

    fn tile(&self, pos: TilePos) -> Tile {
        let n = TILE_SIZE as usize;
        let m = n + 2;
        let (ox, oz) = pos.origin();
        let lo = (self.settings.min_y + 1) as f32;
        let hi = (self.settings.max_y - 1) as f32;
        let made: Vec<Made> = (0..m * m)
            .map(|k| {
                let (x, z) = (ox + (k % m) as i32 - 1, oz + (k / m) as i32 - 1);
                let mut c = self.column(x, z);
                c.height = crate::codec::snap_height(c.height.clamp(lo, hi));
                c
            })
            .collect();
        let sea = self.settings.sea_level as f32;
        let mut top = vec![0f32; TILE_AREA];
        let mut terrain = vec![0u16; TILE_AREA];
        let mut fluid = vec![NO_FLUID; TILE_AREA];
        let mut biome = vec![0u16; TILE_AREA];
        for z in 0..n {
            for x in 0..n {
                let c = (z + 1) * m + x + 1;
                let here = made[c];
                let slope = ((made[c + 1].height - made[c - 1].height) * 0.5).abs().max(((made[c + m].height - made[c - m].height) * 0.5).abs());
                let i = z * n + x;
                top[i] = here.height;
                if let Some(f) = here.fluid {
                    fluid[i] = f.round() as i16;
                }
                let jitter = self.detail.get((ox + x as i32) as f64 / 9.0, (oz + z as i32) as f64 / 9.0) as f32;
                // The edge's own terrain near it only: farther, carried straight out,
                // it would draw streaks.
                let keep_edge = here.edge.terrain != 0 && here.distance < SEAM && here.edge_wet == here.fluid.is_some() && slope < 1.35;
                terrain[i] = if keep_edge { here.edge.terrain } else { self.theme.terrain_with(here.height, slope, sea, jitter, true, self.settings.lava).0 };
                biome[i] = here.biome;
            }
        }
        let mut tile = Tile::ground(sea, 0, NO_FLUID);
        tile.top = crate::plane::Plane::from_vec(top);
        tile.terrain = crate::plane::Plane::from_vec(terrain);
        tile.fluid = crate::plane::Plane::from_vec(fluid);
        tile.biome = crate::plane::Plane::from_vec(biome);
        tile
    }
}

/// The rectangle of whole tiles `tiles` cover, in blocks.
pub fn block_bounds(tiles: &HashSet<TilePos>) -> Option<BlockRect> {
    let (lo, hi) = bounds(tiles)?;
    Some(BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{DimensionKind, World, WorldMeta};

    fn square(n: i32) -> HashSet<TilePos> {
        (0..n).flat_map(|z| (0..n).map(move |x| TilePos::new(x, z))).collect()
    }

    fn settings(fill: Fill) -> FillSettings {
        FillSettings { fill, ease: 64.0, sea_level: 63, lava: false, seed: 7, min_y: -64, max_y: 320 }
    }

    /// A map of 2 × 2 tiles: land rising from west (70) to east (100),
    /// a lake in its south-west corner.
    fn map() -> World {
        let mut world = World::new(WorldMeta::default());
        let dim = &mut world.dims[0];
        assert_eq!(dim.kind, DimensionKind::Overworld);
        for p in square(2) {
            let mut tile = Tile::ground(70.0, 0, NO_FLUID);
            let (ox, oz) = p.origin();
            let mut top = vec![0f32; TILE_AREA];
            let mut fluid = vec![NO_FLUID; TILE_AREA];
            for z in 0..TILE_SIZE {
                for x in 0..TILE_SIZE {
                    let (bx, bz) = (ox + x, oz + z);
                    let i = (z * TILE_SIZE + x) as usize;
                    top[i] = 70.0 + 30.0 * bx as f32 / 255.0;
                    if bx < 40 && bz > 200 {
                        top[i] = 60.0;
                        fluid[i] = 66;
                    }
                }
            }
            tile.top = crate::plane::Plane::from_vec(top);
            tile.fluid = crate::plane::Plane::from_vec(fluid);
            tile.terrain = crate::plane::Plane::Uniform(world.registry.terrain_by_name("Grass").unwrap().0);
            dim.ground_mut().insert(p, tile);
        }
        world
    }

    #[test]
    fn sides_grow_and_crop() {
        let tiles = square(4);
        let grown = sided(&tiles, Sides { west: 1, east: 2, north: 0, south: -1 });
        assert_eq!(bounds(&grown), Some((TilePos::new(-1, 0), TilePos::new(5, 2))));
        assert_eq!(grown.len(), 7 * 3);
        // A hole in the map stays a hole; the band outside is whole.
        let mut holed = tiles.clone();
        holed.remove(&TilePos::new(1, 1));
        let grown = sided(&holed, Sides { west: 1, ..Sides::default() });
        assert!(!grown.contains(&TilePos::new(1, 1)));
        assert!((0..4).all(|z| grown.contains(&TilePos::new(-1, z))));
        assert!(sided(&tiles, Sides { west: -4, ..Sides::default() }).is_empty());
    }

    #[test]
    fn a_map_continued_starts_from_its_edge() {
        let mut world = map();
        let reg = world.registry.clone();
        let target = sided(&square(2), Sides { west: 1, east: 1, north: 1, south: 1 });
        let applied = apply(&mut world.dims[0], &target, &settings(Fill::Continue), &reg, None, &AtomicBool::new(false));
        assert_eq!((applied.added, applied.removed), (12, 0));
        let ground = world.dims[0].ground();
        let top = |x: i32, z: i32| {
            let t = ground.view(TilePos::containing(x, z)).unwrap();
            let (ox, oz) = TilePos::containing(x, z).origin();
            t.top.get(((z - oz) * TILE_SIZE + x - ox) as usize)
        };
        // Next to the east edge (100) and the west edge (70).
        assert!((top(256, 100) - top(255, 100)).abs() < 1.5, "east seam {} → {}", top(255, 100), top(256, 100));
        assert!((top(-1, 100) - top(0, 100)).abs() < 1.5, "west seam {} → {}", top(0, 100), top(-1, 100));
        // The lake goes on west of the map.
        let t = ground.view(TilePos::containing(-3, 230)).unwrap();
        let i = ((230 - 128) * TILE_SIZE + (-3 + 128)) as usize;
        assert_eq!(t.fluid.get(i), 66, "the lake continues");
        // Farther out the land keeps the side's height, within its relief.
        assert!((top(380, 100) - 100.0).abs() < 20.0, "far east {}", top(380, 100));
    }

    #[test]
    fn the_sea_eases_out_of_the_edge() {
        let mut world = map();
        let reg = world.registry.clone();
        let target = sided(&square(2), Sides { east: 2, ..Sides::default() });
        apply(&mut world.dims[0], &target, &settings(Fill::Sea { depth: 12.0 }), &reg, None, &AtomicBool::new(false));
        let ground = world.dims[0].ground();
        let col = |x: i32, z: i32| {
            let p = TilePos::containing(x, z);
            let t = ground.view(p).unwrap();
            let (ox, oz) = p.origin();
            let i = ((z - oz) * TILE_SIZE + x - ox) as usize;
            (t.top.get(i), t.fluid.get(i))
        };
        let (h, _) = col(256, 50);
        assert!((h - 100.0).abs() < 1.5, "the seam {h}");
        let (h, f) = col(256 + 200, 50);
        assert_eq!((h, f), (51.0, 63), "past the band: the sea floor under the sea");
        // Heights only go down from the edge to the sea floor.
        let mut last = f32::MAX;
        for x in 256..256 + 100 {
            let (h, _) = col(x, 50);
            assert!(h <= last + 0.5, "{x}: {h} after {last}");
            last = h;
        }
    }

    #[test]
    fn cropping_drops_tiles_of_every_level() {
        let mut world = map();
        let reg = world.registry.clone();
        let slab = world.dims[0].add_level("Slab", crate::level::LevelKind::Solid);
        world.dims[0].levels[slab].insert(TilePos::new(1, 1), Tile::empty_slab());
        let target = sided(&square(2), Sides { east: -1, ..Sides::default() });
        let applied = apply(&mut world.dims[0], &target, &settings(Fill::Continue), &reg, None, &AtomicBool::new(false));
        assert_eq!((applied.added, applied.removed), (0, 2));
        assert_eq!(world.dims[0].ground().len(), 2);
        assert!(world.dims[0].levels[slab].is_empty());
    }

    #[test]
    fn a_large_ring_is_quick() {
        // 16 × 16 tiles grown by two on every side: 144 tiles to fill.
        let mut world = World::new(WorldMeta::default());
        for p in square(16) {
            world.dims[0].ground_mut().insert(p, Tile::ground(80.0, 0, NO_FLUID));
        }
        let reg = world.registry.clone();
        let target = sided(&square(16), Sides { west: 2, east: 2, north: 2, south: 2 });
        let t = std::time::Instant::now();
        let applied = apply(&mut world.dims[0], &target, &settings(Fill::Generated), &reg, None, &AtomicBool::new(false));
        assert_eq!(applied.added, 20 * 20 - 16 * 16);
        eprintln!("{} tiles filled in {:.2} s", applied.added, t.elapsed().as_secs_f32());
    }
}
