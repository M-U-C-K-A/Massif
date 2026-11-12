//! Editing operations: what the tools do to the world.
//!
//! Every operation works on an [`Edit`], so it is recorded for undo, and takes
//! the brush centre in continuous block coordinates. Height tools act on the
//! top or the bottom surface of the edited level; on slab levels they only
//! touch columns that exist and never let the two surfaces cross.

use crate::brush::Brush;
use crate::codec::snap_height;
use crate::compose::{block_top, hash2};
use crate::coords::{BlockRect, TILE_AREA, TILE_SIZE, TilePos, cell_index, local_index};
use crate::history::Edit;
use crate::level::{Level, LevelKind};
use crate::plane::Plane;
use crate::registry::{BiomeId, TerrainId};
use crate::tile::{NO_FLUID, Tile};
use crate::world::Dimension;
use std::collections::HashMap;

/// Which surface of a level a height tool edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Top,
    Bottom,
}

/// Where a stamped slab is centred vertically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// At a fixed elevation.
    Absolute(f32),
    /// At an offset from the ground surface (negative = underground).
    Ground(f32),
}

/// The brush at one dab: its shape around `(cx, cz)`, times its filter.
struct Dab<'a> {
    brush: &'a Brush,
    cx: f32,
    cz: f32,
    mask: Option<crate::filter::Mask>,
}

impl<'a> Dab<'a> {
    fn new(edit: &Edit, brush: &'a Brush, cx: f32, cz: f32) -> Self {
        let mask = brush
            .filter
            .as_ref()
            .filter(|f| f.is_active())
            .map(|f| f.mask(edit.dim(), BlockRect::around(cx, cz, brush.radius)));
        Self { brush, cx, cz, mask }
    }

    /// Strength at column `(x, z)`, filter included, without the intensity.
    #[inline]
    fn weight(&self, x: i32, z: i32) -> f32 {
        let w = self.brush.weight(x as f32 + 0.5 - self.cx, z as f32 + 0.5 - self.cz);
        match &self.mask {
            Some(m) if w > 0.0 => w * m.get(x, z),
            _ => w,
        }
    }
}

/// Visit the cells of `rect` tile by tile. `f` receives the tile position and
/// the part of `rect` inside that tile.
fn clip_tiles(rect: BlockRect) -> impl Iterator<Item = (TilePos, BlockRect)> {
    rect.tiles().map(move |pos| (pos, rect.intersect(&pos.rect())))
}

/// Run `f(height, weight, x, z) -> new_height` over the brush footprint of one
/// surface of the edited level.
fn edit_heights(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    surface: Surface,
    f: impl Fn(f32, f32, i32, i32) -> f32 + Sync,
) {
    edit_heights_by_tile(edit, brush, cx, cz, surface, |_, _| &f);
}

/// [`edit_heights`] where the height function is made per tile:
/// `per_tile(tile, clip)` returns the function for that tile's part of the
/// footprint. Tiles are edited in parallel.
fn edit_heights_by_tile<G>(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    surface: Surface,
    per_tile: impl Fn(TilePos, BlockRect) -> G + Sync,
) where
    G: FnMut(f32, f32, i32, i32) -> f32,
{
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let surface = if edit.kind().has_bottom() {
        surface
    } else {
        Surface::Top
    };
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(footprint.tiles(), footprint, false, |pos, clip, tile| {
        let mut f = per_tile(pos, clip);
        let Tile { top, bottom, .. } = tile;
        let top = top.dense_mut();
        let mut bottom = bottom.as_mut().map(Plane::dense_mut);
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 {
                    continue;
                }
                let i = local_index(x, z);
                match (&mut bottom, surface) {
                    (None, _) => {
                        top[i] = snap_height(f(top[i], w, x, z).clamp(lo, hi));
                    }
                    (Some(bottom), Surface::Top) => {
                        if top[i] > bottom[i] {
                            let h = f(top[i], w, x, z).clamp(lo, hi);
                            top[i] = snap_height(h.max(bottom[i]));
                        }
                    }
                    (Some(bottom), Surface::Bottom) => {
                        if top[i] > bottom[i] {
                            let h = f(bottom[i], w, x, z).clamp(lo, hi);
                            bottom[i] = snap_height(h.min(top[i]));
                        }
                    }
                }
            }
        }
    });
}

/// Raise (or lower, with a negative `amount`) by up to `amount` blocks.
pub fn raise(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, amount: f32, surface: Surface) {
    edit_heights(edit, brush, cx, cz, surface, |h, w, _, _| h + amount * w);
}

/// Pull the surface toward `target`.
pub fn flatten(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, target: f32, surface: Surface) {
    let k = brush.strength.clamp(0.0, 1.0);
    edit_heights(edit, brush, cx, cz, surface, |h, w, _, _| {
        h + (target - h) * (w * k).min(1.0)
    });
}

/// Set the surface to exactly `target` inside the footprint.
pub fn set_height(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, target: f32, surface: Surface) {
    edit_heights(edit, brush, cx, cz, surface, |_, _, _, _| target);
}

/// Add positional noise of the given amplitude: a quick way to break up flat
/// surfaces.
pub fn roughen(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    amplitude: f32,
    seed: u64,
    surface: Surface,
) {
    let noise = crate::noise::Noise::new(seed);
    edit_heights(edit, brush, cx, cz, surface, |h, w, x, z| {
        let n = noise.fbm(x as f64 * 0.11, z as f64 * 0.11, 3, 2.0, 0.5) as f32;
        h + n * amplitude * w
    });
}

/// Read one surface of a level over `rect` (row-major). Missing columns
/// are NaN.
fn gather(level: &Level, rect: BlockRect, surface: Surface) -> Vec<f32> {
    let width = rect.width() as usize;
    let mut out = vec![f32::NAN; width * rect.depth() as usize];
    for (pos, clip) in clip_tiles(rect) {
        let Some(tile) = level.view(pos) else { continue };
        let top = tile.top.dense();
        let bottom = tile.bottom.as_ref().map(|b| b.dense());
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let i = local_index(x, z);
                let value = match (&bottom, surface) {
                    (None, _) => top[i],
                    (Some(b), _) if top[i] <= b[i] => f32::NAN,
                    (Some(_), Surface::Top) => top[i],
                    (Some(b), Surface::Bottom) => b[i],
                };
                out[(z - rect.z0) as usize * width + (x - rect.x0) as usize] = value;
            }
        }
    }
    out
}

/// One pass of a box filter of radius `k` over a row-major buffer of the
/// given `width`: along each of the `lines` rows, or — with `across` — along
/// each of the `lines` columns. `len` is the length of one line.
fn box_pass(src: &[f32], dst: &mut [f32], lines: usize, len: usize, k: usize, across: bool, width: usize) {
    for line in 0..lines {
        let at = |i: usize| if across { i * width + line } else { line * width + i };
        let mut sum = 0.0f32;
        for i in 0..len.min(k + 1) {
            sum += src[at(i)];
        }
        for i in 0..len {
            dst[at(i)] = sum;
            if i + k + 1 < len {
                sum += src[at(i + k + 1)];
            }
            if i >= k {
                sum -= src[at(i - k)];
            }
        }
    }
}

/// Blend the surface toward its local average.
///
/// The averages are computed tile by tile, in parallel, from the heights
/// before the dab, so memory stays small whatever the brush size.
pub fn smooth(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, surface: Surface) {
    use rayon::prelude::*;
    let surface = if edit.kind().has_bottom() {
        surface
    } else {
        Surface::Top
    };
    let k = ((brush.radius * 0.2).round() as i32).clamp(1, 12);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    let level = edit.level();
    let tiles: Vec<TilePos> = footprint.tiles().filter(|p| level.contains(*p)).collect();
    let averages: HashMap<TilePos, Vec<f32>> = tiles
        .par_iter()
        .map(|&pos| {
            let clip = footprint.intersect(&pos.rect());
            (pos, local_averages(level, clip, k, surface))
        })
        .collect();

    let strength = brush.strength.clamp(0.0, 1.0);
    edit_heights_by_tile(edit, brush, cx, cz, surface, |pos, clip| {
        let avg = averages.get(&pos);
        let cw = clip.width() as usize;
        move |h, wgt, x, z| {
            let Some(avg) = avg else { return h };
            let a = avg[(z - clip.z0) as usize * cw + (x - clip.x0) as usize];
            if a.is_nan() {
                return h;
            }
            h + (a - h) * (wgt * strength).min(1.0)
        }
    });
}

/// The average of one surface over a `(2k+1)²` window around every cell of
/// `clip` (row-major, NaN where no column is near). Missing columns (outside
/// the map, absent from a slab) are left out rather than counted as zero.
fn local_averages(level: &Level, clip: BlockRect, k: i32, surface: Surface) -> Vec<f32> {
    let rect = clip.expand(k);
    let (w, d) = (rect.width() as usize, rect.depth() as usize);
    let samples = gather(level, rect, surface);
    let mut val: Vec<f32> = samples.iter().map(|s| if s.is_nan() { 0.0 } else { *s }).collect();
    let mut cov: Vec<f32> = samples.iter().map(|s| (!s.is_nan()) as u8 as f32).collect();
    drop(samples);
    let mut tmp = vec![0f32; w * d];
    for buf in [&mut val, &mut cov] {
        box_pass(buf, &mut tmp, d, w, k as usize, false, w);
        box_pass(&tmp, buf, w, d, k as usize, true, w);
    }
    let cw = clip.width() as usize;
    let mut out = vec![f32::NAN; cw * clip.depth() as usize];
    for z in clip.z0..clip.z1 {
        for x in clip.x0..clip.x1 {
            let i = (z - rect.z0) as usize * w + (x - rect.x0) as usize;
            if cov[i] > 0.0 {
                out[(z - clip.z0) as usize * cw + (x - clip.x0) as usize] = val[i] / cov[i];
            }
        }
    }
    out
}

/// Whether a paint dab hits a cell of weight `w`: the core is solid, the
/// falloff is dithered so that repeated dabs build up like a spray.
#[inline]
fn paint_hit(w: f32, strength: f32, x: i32, z: i32, salt: u64) -> bool {
    let p = (w * strength * 2.0).min(1.0);
    p >= 1.0 || (hash2(x, z, salt) as f32) < p * p * u32::MAX as f32
}

fn paint_plane(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    salt: u64,
    value: u16,
    plane: impl Fn(&mut Tile) -> &mut Plane<u16> + Sync,
) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                plane(tile).set(i, value);
            }
        }
    });
}

/// Paint, per column, the terrain `choose` picks from the surface height
/// (used to put back the terrain the height calls for).
pub fn paint_terrain_with(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    salt: u64,
    choose: impl Fn(f32) -> TerrainId + Sync,
) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                let t = choose(tile.top.get(i));
                tile.terrain.set(i, t.0);
            }
        }
    });
}

/// Paint a terrain. `salt` should change from dab to dab.
pub fn paint_terrain(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, terrain: TerrainId, salt: u64) {
    paint_plane(edit, brush, cx, cz, salt, terrain.0, |t| &mut t.terrain);
}

/// Paint a biome. `salt` should change from dab to dab.
pub fn paint_biome(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, biome: BiomeId, salt: u64) {
    paint_plane(edit, brush, cx, cz, salt, biome.0, |t| &mut t.biome);
}

/// Paint the terrains of a pattern under the brush (see
/// [`crate::pattern`]). `salt` should change from dab to dab.
pub fn paint_pattern(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, pattern: &crate::pattern::PatternSampler) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                if let Some(t) = pattern.terrain(x, z) {
                    tile.terrain.set(i, t.0);
                }
            }
        }
    });
}

/// Give the columns under the brush the biome their land calls for (see
/// [`crate::auto`]). `salt` should change from dab to dab.
pub fn auto_biome(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, reader: &crate::auto::Reader) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    let survey = reader.survey(edit.dim(), footprint);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                if let Some(biome) = reader.biome(&survey, x, z) {
                    tile.biome.set(i, biome.0);
                }
            }
        }
    });
}

/// Paint the columns under the brush with the terrain their biome calls
/// for (see [`crate::auto`]). `salt` should change from dab to dab.
pub fn auto_paint(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, reader: &crate::auto::Reader) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    let survey = reader.survey(edit.dim(), footprint);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                if let Some(terrain) = reader.terrain(&survey, x, z) {
                    tile.terrain.set(i, terrain.0);
                }
            }
        }
    });
}

/// Set the fluid surface (or [`NO_FLUID`]) under the whole footprint.
pub fn set_fluid(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, level: i16) {
    let dab = Dab::new(edit, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                if dab.weight(x, z) > 0.0 {
                    tile.fluid.set(local_index(x, z), level);
                }
            }
        }
    });
}

/// Add a lens-shaped slab to a solid or carve level: full `thickness` at the
/// brush core, thinning out along the falloff. Existing columns only grow.
/// Slabs are only created above ground tiles, i.e. inside the map.
pub fn stamp_slab(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, anchor: Anchor, thickness: f32) {
    if edit.kind() == LevelKind::Ground {
        return;
    }
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let rect = BlockRect::around(cx, cz, brush.radius);
    let dab = Dab::new(edit, brush, cx, cz);
    let inside: Vec<TilePos> = rect.tiles().filter(|p| edit.dim().ground().contains(*p)).collect();
    edit.par_tiles_with(inside, rect, true, |pos, clip, tile, others| {
        let Tile { top, bottom, .. } = tile;
        let Some(bottom) = bottom else { return };
        let (top, bottom) = (top.dense_mut(), bottom.dense_mut());
        let ground = others.get(0).and_then(|g| g.view(pos));
        let ground = ground.as_ref().map(|g| g.top.dense());
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let w = dab.weight(x, z);
                let half = thickness * 0.5 * w;
                if half < 0.5 {
                    continue; // thinner than a block: nothing to show
                }
                let i = local_index(x, z);
                let centre = match anchor {
                    Anchor::Absolute(y) => y,
                    Anchor::Ground(offset) => match &ground {
                        Some(g) => g[i] + offset,
                        None => continue,
                    },
                };
                let new_top = snap_height((centre + half).clamp(lo, hi));
                let new_bottom = snap_height((centre - half).clamp(lo, hi));
                if top[i] > bottom[i] {
                    top[i] = top[i].max(new_top);
                    bottom[i] = bottom[i].min(new_bottom);
                } else {
                    top[i] = new_top;
                    bottom[i] = new_bottom;
                }
            }
        }
    });
    // Tiles the footprint's bounding box touched without adding a column.
    edit.prune_empty(rect.tiles());
}

/// Remove the columns of a slab level under the footprint.
pub fn erase_slab(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32) {
    if edit.kind() == LevelKind::Ground {
        return;
    }
    let dab = Dab::new(edit, brush, cx, cz);
    let rect = BlockRect::around(cx, cz, brush.radius);
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let Tile { top, bottom, .. } = tile;
        let Some(bottom) = bottom else { return };
        let (top, bottom) = (top.dense_mut(), bottom.dense_mut());
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                if dab.weight(x, z) > 0.0 {
                    let i = local_index(x, z);
                    top[i] = 0.0;
                    bottom[i] = 0.0;
                }
            }
        }
    });
    edit.prune_empty(rect.tiles());
}

/// Elevation of one surface of a level at a block column.
pub fn height_at(dim: &Dimension, level: usize, x: i32, z: i32, surface: Surface) -> Option<f32> {
    let tile = dim.levels.get(level)?.view(TilePos::containing(x, z))?;
    let i = local_index(x, z);
    match (&tile.bottom, surface) {
        (None, _) => Some(tile.top.get(i)),
        (Some(_), _) if !tile.present(i) => None,
        (Some(_), Surface::Top) => Some(tile.top.get(i)),
        (Some(b), Surface::Bottom) => Some(b.get(i)),
    }
}

// ------------------------------------------------------------ flood fill --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flood {
    /// Fill the connected basin up to this fluid surface elevation.
    Fill(i16),
    /// Remove the connected body of fluid.
    Drain,
}

struct FloodTile {
    pos: TilePos,
    top: Vec<i32>,
    fluid: Vec<i16>,
    changed: bool,
}

struct FloodGrid<'a> {
    dim: &'a Dimension,
    tiles: Vec<FloodTile>,
    lookup: HashMap<TilePos, Option<usize>>,
    last: Option<(TilePos, usize)>,
}

impl FloodGrid<'_> {
    fn tile(&mut self, pos: TilePos) -> Option<usize> {
        if let Some((p, i)) = self.last
            && p == pos
        {
            return Some(i);
        }
        let dim = self.dim;
        let tiles = &mut self.tiles;
        let slot = *self.lookup.entry(pos).or_insert_with(|| {
            let tile = dim.ground().view(pos)?;
            tiles.push(FloodTile {
                pos,
                top: tile.top.dense().iter().map(|&h| block_top(h)).collect(),
                fluid: tile.fluid.dense().into_owned(),
                changed: false,
            });
            Some(tiles.len() - 1)
        });
        if let Some(i) = slot {
            self.last = Some((pos, i));
        }
        slot
    }

    /// Flood the cell if `mode` applies to it. Returns whether it did.
    #[inline]
    fn visit(&mut self, x: i32, z: i32, mode: Flood) -> bool {
        let Some(t) = self.tile(TilePos::containing(x, z)) else {
            return false;
        };
        let tile = &mut self.tiles[t];
        let i = local_index(x, z);
        let (top, fluid) = (tile.top[i], tile.fluid[i]);
        let new = match mode {
            Flood::Fill(level) => {
                // Needs room for water, and must not already be that deep.
                if top >= level as i32 || (fluid != NO_FLUID && fluid >= level) {
                    return false;
                }
                level
            }
            Flood::Drain => {
                if fluid == NO_FLUID || (fluid as i32) <= top {
                    return false;
                }
                NO_FLUID
            }
        };
        tile.fluid[i] = new;
        tile.changed = true;
        true
    }
}

/// Flood-fill fluid on the ground level starting from column `(x, z)`.
///
/// Returns the number of columns changed. The fill stops at the edge of the
/// map and after `max_cells` columns.
pub fn flood_fill(edit: &mut Edit, x: i32, z: i32, mode: Flood, max_cells: usize) -> usize {
    if edit.level_index() != 0 {
        return 0;
    }
    let mut filled = 0usize;
    let changed: Vec<(TilePos, Vec<i16>)> = {
        let mut grid = FloodGrid {
            dim: edit.dim(),
            tiles: Vec::new(),
            lookup: HashMap::new(),
            last: None,
        };
        // Span-based scanline fill: the stack holds runs `(x0, x1, z)` that are
        // already flooded, so it grows with the number of runs, not of cells.
        let run = |grid: &mut FloodGrid, x: i32, z: i32, filled: &mut usize| {
            if !grid.visit(x, z, mode) {
                return None;
            }
            *filled += 1;
            let (mut x0, mut x1) = (x, x);
            while grid.visit(x0 - 1, z, mode) {
                x0 -= 1;
                *filled += 1;
            }
            while grid.visit(x1 + 1, z, mode) {
                x1 += 1;
                *filled += 1;
            }
            Some((x0, x1, z))
        };
        let mut stack = Vec::new();
        stack.extend(run(&mut grid, x, z, &mut filled));
        while let Some((x0, x1, sz)) = stack.pop() {
            if filled >= max_cells {
                break;
            }
            for nz in [sz - 1, sz + 1] {
                let mut nx = x0;
                while nx <= x1 {
                    match run(&mut grid, nx, nz, &mut filled) {
                        Some(span) => {
                            nx = span.1 + 1;
                            stack.push(span);
                        }
                        None => nx += 1,
                    }
                }
            }
        }
        grid.tiles
            .into_iter()
            .filter(|t| t.changed)
            .map(|t| (t.pos, t.fluid))
            .collect()
    };
    for (pos, fluid) in changed {
        if let Some(tile) = edit.tile_mut(pos) {
            debug_assert_eq!(fluid.len(), TILE_AREA);
            tile.fluid = Plane::from_vec(fluid);
            tile.fluid.compact();
        }
    }
    filled
}

// --------------------------------------------------------- whole-map ops --

/// Create blank ground tiles over a rectangle of tiles (inclusive bounds).
/// Existing tiles are left alone. Returns the number of tiles added.
pub fn add_ground_tiles(
    edit: &mut Edit,
    min: TilePos,
    max: TilePos,
    height: f32,
    terrain: TerrainId,
    fluid: i16,
) -> usize {
    if edit.level_index() != 0 {
        return 0;
    }
    let mut added = 0;
    for z in min.z..=max.z {
        for x in min.x..=max.x {
            let pos = TilePos::new(x, z);
            if !edit.level().contains(pos) {
                let tile = edit.tile_mut_or_insert(pos);
                *tile = Tile::ground(snap_height(height), terrain.0, fluid);
                added += 1;
            }
        }
    }
    added
}

/// Shift a whole slab level (or the ground) vertically by `delta` blocks.
pub fn shift_level(edit: &mut Edit, delta: f32) {
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let positions: Vec<TilePos> = edit.level().positions().collect();
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    edit.par_tiles(positions, everywhere, false, |_, _, tile| {
        let Tile { top, bottom, .. } = tile;
        match bottom {
            None => {
                for h in top.dense_mut().iter_mut() {
                    *h = snap_height((*h + delta).clamp(lo, hi));
                }
            }
            Some(bottom) => {
                let (top, bottom) = (top.dense_mut(), bottom.dense_mut());
                for i in 0..TILE_AREA {
                    if top[i] > bottom[i] {
                        top[i] = snap_height((top[i] + delta).clamp(lo, hi));
                        bottom[i] = snap_height((bottom[i] + delta).clamp(lo, hi));
                    }
                }
            }
        }
        tile.compact();
    });
}

/// Number of existing columns of a slab tile (all of them for ground tiles).
pub fn present_columns(tile: &Tile) -> usize {
    match &tile.bottom {
        None => TILE_AREA,
        Some(bottom) => {
            let (top, bottom) = (tile.top.dense(), bottom.dense());
            let n = TILE_SIZE as usize;
            (0..n)
                .flat_map(|z| (0..n).map(move |x| cell_index(x, z)))
                .filter(|&i| top[i] > bottom[i])
                .count()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{BrushShape, Profile};
    use crate::history::History;
    use crate::world::{World, WorldMeta};

    fn world(tiles: i32) -> World {
        let mut w = World::new(WorldMeta::default());
        for z in -tiles..tiles {
            for x in -tiles..tiles {
                w.dims[0]
                    .ground_mut()
                    .insert(TilePos::new(x, z), Tile::ground(64.0, 1, 63));
            }
        }
        w
    }

    fn hard_brush(radius: f32) -> Brush {
        Brush {
            shape: BrushShape::Circle,
            radius,
            profile: Profile::Constant,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        }
    }

    fn h(w: &World, x: i32, z: i32) -> f32 {
        height_at(&w.dims[0], 0, x, z, Surface::Top).unwrap()
    }

    #[test]
    fn raise_spans_tile_borders_and_negative_coordinates() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Raise", 0);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(6.0), 0.0, 0.0, 10.0, Surface::Top);
        hist.commit();
        for (x, z) in [(0, 0), (-1, -1), (-1, 0), (0, -1), (4, 3), (-5, -3)] {
            assert_eq!(h(&w, x, z), 74.0, "({x}, {z})");
        }
        assert_eq!(h(&w, 6, 0), 64.0);
        assert_eq!(h(&w, -7, 0), 64.0);
        // Four tiles were touched; undo restores all of them.
        hist.undo(&mut w);
        assert_eq!(h(&w, -1, -1), 64.0);
    }

    #[test]
    fn soft_brush_raises_less_at_the_rim_and_stays_on_the_grid() {
        let mut w = world(1);
        let mut hist = History::default();
        let brush = Brush {
            radius: 20.0,
            ..Brush::default()
        };
        hist.begin("Raise", 0);
        raise(&mut hist.edit(&mut w, 0), &brush, 0.5, 0.5, 8.0, Surface::Top);
        hist.commit();
        let centre = h(&w, 0, 0);
        let mid = h(&w, 10, 0);
        let rim = h(&w, 19, 0);
        assert!(centre > mid && mid > rim && rim >= 64.0, "{centre} {mid} {rim}");
        assert_eq!(mid, snap_height(mid));
    }

    #[test]
    fn heights_are_clamped_to_the_build_limits() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Raise", 0);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 0.0, 0.0, 1e6, Surface::Top);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 50.0, 50.0, -1e6, Surface::Top);
        hist.commit();
        assert_eq!(h(&w, 0, 0), 320.0);
        assert_eq!(h(&w, 50, 50), -64.0);
    }

    #[test]
    fn flatten_and_smooth_converge() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Shape", 0);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(4.0), 10.0, 10.0, 20.0, Surface::Top);
        // (13, 10) sits on the rim of the plateau, (15, 10) just outside.
        let before = h(&w, 13, 10);
        assert_eq!((before, h(&w, 15, 10)), (84.0, 64.0));
        smooth(&mut hist.edit(&mut w, 0), &hard_brush(12.0), 10.0, 10.0, Surface::Top);
        let after = h(&w, 13, 10);
        assert!(after < before && after > 64.0, "{before} -> {after}");
        // The plateau's surroundings went up a little.
        assert!(h(&w, 15, 10) > 64.0);
        for _ in 0..20 {
            flatten(&mut hist.edit(&mut w, 0), &hard_brush(12.0), 10.0, 10.0, 70.0, Surface::Top);
        }
        assert_eq!(h(&w, 10, 10), 70.0);
        hist.commit();
    }

    #[test]
    fn smooth_ignores_columns_outside_the_map() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Smooth", 0);
        // At the map corner: a naive blur would pull the edge toward zero.
        smooth(&mut hist.edit(&mut w, 0), &hard_brush(10.0), 127.0, 127.0, Surface::Top);
        hist.commit();
        assert_eq!(h(&w, 127, 127), 64.0);
        assert_eq!(h(&w, 120, 125), 64.0);
    }

    #[test]
    fn painting_sets_terrain_and_biome() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Paint", 0);
        paint_terrain(&mut hist.edit(&mut w, 0), &hard_brush(5.0), 0.0, 0.0, TerrainId(7), 1);
        paint_biome(&mut hist.edit(&mut w, 0), &hard_brush(5.0), 0.0, 0.0, BiomeId(3), 1);
        hist.commit();
        let t = w.dims[0].ground().view(TilePos::new(-1, -1)).unwrap();
        assert_eq!(t.terrain.get(local_index(-1, -1)), 7);
        assert_eq!(t.biome.get(local_index(-2, -2)), 3);
        assert_eq!(t.terrain.get(local_index(-20, -20)), 1);
    }

    #[test]
    fn slabs_are_stamped_sculpted_and_erased() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Add level", 0);
        let lvl = hist
            .edit(&mut w, 0)
            .structure()
            .add_level("Island", LevelKind::Solid);
        hist.commit();

        hist.begin("Stamp", 0);
        stamp_slab(
            &mut hist.edit(&mut w, lvl),
            &hard_brush(8.0),
            0.0,
            0.0,
            Anchor::Ground(40.0),
            10.0,
        );
        hist.commit();
        let d = &w.dims[0];
        assert_eq!(height_at(d, lvl, 0, 0, Surface::Top), Some(109.0));
        assert_eq!(height_at(d, lvl, -1, -1, Surface::Bottom), Some(99.0));
        assert_eq!(height_at(d, lvl, 20, 0, Surface::Top), None);
        // The stamp crossed the origin, so four slab tiles exist.
        assert_eq!(d.levels[lvl].len(), 4);

        // Height tools never let the two surfaces cross.
        hist.begin("Lower", 0);
        raise(&mut hist.edit(&mut w, lvl), &hard_brush(3.0), 0.0, 0.0, -500.0, Surface::Top);
        hist.commit();
        let d = &w.dims[0];
        assert_eq!(height_at(d, lvl, 0, 0, Surface::Top), None); // collapsed
        assert_eq!(height_at(d, lvl, 5, 0, Surface::Top), Some(109.0));

        hist.begin("Erase", 0);
        erase_slab(&mut hist.edit(&mut w, lvl), &hard_brush(30.0), 0.0, 0.0);
        hist.commit();
        assert_eq!(w.dims[0].levels[lvl].len(), 0);
        hist.undo(&mut w);
        assert_eq!(w.dims[0].levels[lvl].len(), 4);
    }

    #[test]
    fn flood_fill_stays_inside_the_basin() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Dig", 0);
        // No water anywhere, then a pit of radius 10 that is 14 blocks deep.
        for pos in w.dims[0].ground().positions().collect::<Vec<_>>() {
            hist.edit(&mut w, 0).tile_mut(pos).unwrap().fluid.fill(NO_FLUID);
        }
        raise(&mut hist.edit(&mut w, 0), &hard_brush(10.0), 0.0, 0.0, -14.0, Surface::Top);
        hist.commit();

        hist.begin("Flood", 0);
        let n = flood_fill(&mut hist.edit(&mut w, 0), 0, 0, Flood::Fill(60), usize::MAX);
        hist.commit();
        let fluid = |w: &World, x: i32, z: i32| {
            let t = w.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
            t.fluid.get(local_index(x, z))
        };
        assert!(n > 250 && n < 400, "{n} columns");
        assert_eq!(fluid(&w, 0, 0), 60);
        assert_eq!(fluid(&w, -9, 0), 60);
        assert_eq!(fluid(&w, 12, 0), NO_FLUID);
        // Filling again changes nothing; draining removes the whole lake.
        hist.begin("Flood", 0);
        assert_eq!(flood_fill(&mut hist.edit(&mut w, 0), 0, 0, Flood::Fill(60), usize::MAX), 0);
        assert_eq!(flood_fill(&mut hist.edit(&mut w, 0), 3, 3, Flood::Drain, usize::MAX), n);
        hist.commit();
        assert_eq!(fluid(&w, 0, 0), NO_FLUID);
    }

    #[test]
    fn flood_fill_honours_the_cell_budget() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Flood", 0);
        // Flat land at 64 with the default sea at 63: flooding to 80 would
        // cover the whole map.
        let n = flood_fill(&mut hist.edit(&mut w, 0), 0, 0, Flood::Fill(80), 1000);
        hist.commit();
        assert!((1000..3000).contains(&n), "{n}");
    }

    #[test]
    fn whole_map_operations() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Grow", 0);
        let added = add_ground_tiles(
            &mut hist.edit(&mut w, 0),
            TilePos::new(-2, -2),
            TilePos::new(1, 1),
            70.0,
            TerrainId(2),
            63,
        );
        hist.commit();
        assert_eq!(added, 16 - 4);
        assert_eq!(h(&w, -200, -200), 70.0);
        assert_eq!(h(&w, 0, 0), 64.0);

        hist.begin("Shift", 0);
        shift_level(&mut hist.edit(&mut w, 0), 5.0);
        hist.commit();
        assert_eq!(h(&w, 0, 0), 69.0);
        hist.undo(&mut w);
        hist.undo(&mut w);
        assert_eq!(w.dims[0].ground().len(), 4);
    }
    #[test]
    fn filters_limit_where_brushes_act() {
        use crate::filter::{Filter, Match};
        use std::sync::Arc;
        let mut w = world(2);
        let mut hist = History::default();
        // Only above 70: the flat ground at 64 is left alone.
        let high = Brush {
            filter: Some(Arc::new(Filter {
                above: Some(70),
                ..Filter::default()
            })),
            ..hard_brush(10.0)
        };
        hist.begin("raise", 0);
        raise(&mut hist.edit(&mut w, 0), &high, 0.0, 0.0, 5.0, Surface::Top);
        hist.commit();
        assert_eq!(h(&w, 0, 0), 64.0);
        // Except on grass (terrain 1, everywhere here): nothing either.
        let except = Brush {
            filter: Some(Arc::new(Filter {
                except_on: Some(vec![Match::Terrain(1)]),
                ..Filter::default()
            })),
            ..hard_brush(10.0)
        };
        hist.begin("raise", 0);
        raise(&mut hist.edit(&mut w, 0), &except, 0.0, 0.0, 5.0, Surface::Top);
        hist.commit();
        assert_eq!(h(&w, 0, 0), 64.0);
        // Only below 70: it raises.
        let low = Brush {
            filter: Some(Arc::new(Filter {
                below: Some(70),
                ..Filter::default()
            })),
            ..hard_brush(10.0)
        };
        hist.begin("raise", 0);
        raise(&mut hist.edit(&mut w, 0), &low, 0.0, 0.0, 5.0, Surface::Top);
        hist.commit();
        assert_eq!(h(&w, 0, 0), 69.0);
        // Painting obeys filters too.
        let sand = TerrainId(7);
        hist.begin("paint", 0);
        paint_terrain(&mut hist.edit(&mut w, 0), &high, 0.0, 0.0, sand, 1);
        hist.commit();
        let tile = w.dims[0].ground().view(TilePos::containing(0, 0)).unwrap();
        assert_eq!(tile.terrain.get(local_index(0, 0)), 1);
    }

}
