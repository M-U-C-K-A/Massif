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
use rayon::prelude::*;
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

/// The brush at one dab: its shape around `(cx, cz)`, times its filter and
/// the map's selection.
struct Dab<'a> {
    brush: &'a Brush,
    cx: f32,
    cz: f32,
    mask: Option<crate::filter::Mask>,
    /// Wholly outside the selection: the dab changes nothing.
    idle: bool,
}

impl<'a> Dab<'a> {
    /// The dab on the edited level. Its tiles are decoded first when the
    /// filter reads them: the filter then reads them as they are, and the
    /// edit after it finds them decoded.
    fn new(edit: &mut Edit, brush: &'a Brush, cx: f32, cz: f32) -> Self {
        let rect = BlockRect::around(cx, cz, brush.reach());
        let off = edit.dim().selection.as_ref().is_some_and(|s| !rect.tiles().any(|p| s.tile(p).is_some()));
        if !off && brush.filter.as_ref().is_some_and(|f| f.is_active()) {
            let tiles: Vec<TilePos> = rect.tiles().collect();
            edit.warm(&tiles);
        }
        Self::on(edit.dim(), brush, cx, cz)
    }

    fn on(dim: &Dimension, brush: &'a Brush, cx: f32, cz: f32) -> Self {
        let rect = BlockRect::around(cx, cz, brush.reach());
        // Outside the selection, nothing to weigh: every weight is 0.
        let selected = dim.selection.as_ref().map(|s| s.weights(rect).unwrap_or_else(|| vec![0; rect.width().max(0) as usize * rect.depth().max(0) as usize]));
        if selected.as_ref().is_some_and(|w| w.iter().all(|&v| v == 0)) {
            let mask = Some(crate::filter::Mask::from_weights(rect, selected.unwrap_or_default()));
            return Self { brush, cx, cz, mask, idle: true };
        }
        let mut mask = brush
            .filter
            .as_ref()
            .filter(|f| f.is_active())
            .map(|f| f.mask_rows(dim, rect, &|z, x0, x1| brush.row_span(cx, cz, z, x0, x1)));
        if let Some(selected) = selected {
            match &mut mask {
                Some(m) => m.times(&selected),
                None => mask = Some(crate::filter::Mask::from_weights(rect, selected)),
            }
        }
        Self { brush, cx, cz, mask, idle: false }
    }

    /// The columns of row `z` of `clip` the brush may change: the others
    /// weigh nothing.
    #[inline]
    fn row(&self, z: i32, clip: &BlockRect) -> std::ops::Range<i32> {
        self.brush.row_span(self.cx, self.cz, z, clip.x0, clip.x1)
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
    // Sharp height limits are kept to: the land stops on them.
    let band = brush.filter.as_ref().filter(|f| f.is_active()).and_then(|f| f.band());
    let held = |old: f32, new: f32| band.map_or(new, |b| b.hold(old, new));
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |pos, clip, tile| {
        let mut f = per_tile(pos, clip);
        let Tile { top, bottom, .. } = tile;
        let top = top.dense_mut();
        let mut bottom = bottom.as_mut().map(Plane::dense_mut);
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
                let w = dab.weight(x, z);
                if w <= 0.0 {
                    continue;
                }
                let i = local_index(x, z);
                match (&mut bottom, surface) {
                    (None, _) => {
                        top[i] = snap_height(held(top[i], f(top[i], w, x, z)).clamp(lo, hi));
                    }
                    (Some(bottom), Surface::Top) => {
                        if top[i] > bottom[i] {
                            let h = held(top[i], f(top[i], w, x, z)).clamp(lo, hi);
                            top[i] = snap_height(h.max(bottom[i]));
                        }
                    }
                    (Some(bottom), Surface::Bottom) => {
                        if top[i] > bottom[i] {
                            let h = held(bottom[i], f(bottom[i], w, x, z)).clamp(lo, hi);
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
    flatten_with(edit, brush, cx, cz, target, surface, FlattenMode::Both, None);
}

/// Which way Flatten moves the land, as WorldPainter's flatten options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FlattenMode {
    /// Up to the height and down to it.
    #[default]
    Both,
    /// Only the land under the height, up to it.
    RaiseOnly,
    /// Only the land over the height, down to it.
    LowerOnly,
}

impl FlattenMode {
    /// Whether land at `h` is moved toward `target`, or (once moved) was.
    fn takes(self, h: f32, target: f32) -> bool {
        match self {
            FlattenMode::Both => true,
            FlattenMode::RaiseOnly => h <= target,
            FlattenMode::LowerOnly => h >= target,
        }
    }
}

/// [`flatten`] one way or both (`mode`), and with `theme` the terrain of
/// the columns it takes painted again with what their new height calls for
/// (the top of the land only).
#[allow(clippy::too_many_arguments)]
pub fn flatten_with(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, target: f32, surface: Surface, mode: FlattenMode, theme: Option<&(dyn Fn(f32) -> TerrainId + Sync)>) {
    let k = brush.strength.clamp(0.0, 1.0);
    edit_heights(edit, brush, cx, cz, surface, |h, w, _, _| {
        if mode.takes(h, target) { h + (target - h) * (w * k).min(1.0) } else { h }
    });
    let Some(choose) = theme else { return };
    if surface == Surface::Bottom && edit.kind().has_bottom() {
        return;
    }
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
                if dab.weight(x, z) <= 0.0 {
                    continue;
                }
                let i = local_index(x, z);
                if slab && !tile.present(i) {
                    continue;
                }
                let h = tile.top.get(i);
                if mode.takes(h, target) {
                    tile.terrain.set(i, choose(h).0);
                }
            }
        }
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
///
/// Across, the columns are run through all at once, row after row, which
/// reads the buffer in order; every column's running sum still takes the
/// same additions in the same order as one column at a time would.
pub(crate) fn box_pass(src: &[f32], dst: &mut [f32], lines: usize, len: usize, k: usize, across: bool, width: usize) {
    if across {
        let mut sum = vec![0.0f32; lines];
        let row = |i: usize| &src[i * width..][..lines];
        for i in 0..len.min(k + 1) {
            for (s, v) in sum.iter_mut().zip(row(i)) {
                *s += v;
            }
        }
        for i in 0..len {
            dst[i * width..][..lines].copy_from_slice(&sum);
            if i + k + 1 < len {
                for (s, v) in sum.iter_mut().zip(row(i + k + 1)) {
                    *s += v;
                }
            }
            if i >= k {
                for (s, v) in sum.iter_mut().zip(row(i - k)) {
                    *s -= v;
                }
            }
        }
        return;
    }
    for line in 0..lines {
        let (src, dst) = (&src[line * width..][..len], &mut dst[line * width..][..len]);
        let mut sum = 0.0f32;
        for v in &src[..len.min(k + 1)] {
            sum += v;
        }
        for i in 0..len {
            dst[i] = sum;
            if i + k + 1 < len {
                sum += src[i + k + 1];
            }
            if i >= k {
                sum -= src[i - k];
            }
        }
    }
}

/// Blend the surface toward its local average.
///
/// The averages are computed tile by tile, in parallel, from the heights
/// before the dab, so memory stays small whatever the brush size.
pub fn smooth(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, surface: Surface) {
    let surface = if edit.kind().has_bottom() {
        surface
    } else {
        Surface::Top
    };
    let k = ((brush.radius * 0.2).round() as i32).clamp(1, 12);
    let footprint = BlockRect::around(cx, cz, brush.reach());
    // Each tile's averages read the tiles around it: decoded once, here,
    // rather than by every tile next to them.
    edit.warm(&footprint.expand(k).tiles().collect::<Vec<_>>());
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
/// Whether the automatic tools change column `(x, z)`: always in the core of
/// the brush, which widens with the intensity (all of it at 100%), here and
/// there along its edge so that it blends.
fn auto_hit(w: f32, strength: f32, x: i32, z: i32, salt: u64) -> bool {
    if w <= 0.0 {
        return false;
    }
    let p = if strength >= 0.99 { 1.0 } else { (w / (1.0 - strength.clamp(0.0, 0.99))).min(1.0) };
    p >= 1.0 || (hash2(x, z, salt) as f32) < p * u32::MAX as f32
}

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
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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

/// Paint a ground cover by its number (0 removes it). `salt` should
/// change from dab to dab.
pub fn paint_cover(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, cover: u16, salt: u64) {
    paint_plane(edit, brush, cx, cz, salt, cover, |t| &mut t.cover);
}

/// Paint an annotation, one of [`crate::tile::ANNOTATION_COLORS`] by its
/// number (1 to 15; 0 removes it). `salt` should change from dab to dab.
pub fn paint_annotation(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, color: u16, salt: u64) {
    paint_plane(edit, brush, cx, cz, salt, color.min(15), |t| &mut t.annotations);
}

/// Paint an object layer by its number (0 removes it). `salt` should
/// change from dab to dab.
pub fn paint_objects(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, layer: u16, salt: u64) {
    paint_plane(edit, brush, cx, cz, salt, layer, |t| &mut t.objects);
}

/// Paint the terrains of a pattern under the brush (see
/// [`crate::pattern`]). `salt` should change from dab to dab.
pub fn paint_pattern(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, pattern: &crate::pattern::PatternSampler) {
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        let slab = tile.bottom.is_some();
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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

/// Paint a gradient of terrains under the brush (see [`crate::gradient`]):
/// each column takes the band its distance to the boundary (or to the
/// brush's centre) says. `sides` is the stroke's: what side of the
/// boundary each column was on when the stroke began. `salt` should change
/// from dab to dab.
#[allow(clippy::too_many_arguments)]
pub fn paint_gradient(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    salt: u64,
    gradient: &crate::gradient::GradientSampler,
    layout: crate::gradient::Layout,
    sides: &mut crate::gradient::Sides,
) {
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    let level = edit.level_index();
    // Square by square, each whole tiles: a giant brush never holds more
    // than one square's distances at once.
    const SQUARE: i32 = 1024;
    let squares = |lo: i32, hi: i32| lo.div_euclid(SQUARE)..=(hi - 1).div_euclid(SQUARE);
    for sz in squares(footprint.z0, footprint.z1) {
        for sx in squares(footprint.x0, footprint.x1) {
            let square = footprint.intersect(&BlockRect::new(sx * SQUARE, sz * SQUARE, (sx + 1) * SQUARE, (sz + 1) * SQUARE));
            if square.is_empty() {
                continue;
            }
            let at = crate::gradient::positions(edit.dim(), level, gradient, layout, sides, square, cx, cz, brush.reach());
            if at.iter().all(|p| p.is_nan()) {
                continue;
            }
            let sw = square.width() as usize;
            edit.par_tiles(square.tiles(), square, false, |_, clip, tile| {
                let slab = tile.bottom.is_some();
                for z in clip.z0..clip.z1 {
                    for x in dab.row(z, &clip) {
                        let p = at[(z - square.z0) as usize * sw + (x - square.x0) as usize];
                        if p.is_nan() {
                            continue;
                        }
                        let w = dab.weight(x, z);
                        if w <= 0.0 || !paint_hit(w, brush.strength, x, z, salt) {
                            continue;
                        }
                        let i = local_index(x, z);
                        if slab && !tile.present(i) {
                            continue;
                        }
                        tile.terrain.set(i, gradient.terrain(p, x, z).0);
                    }
                }
            });
        }
    }
}

/// What a pattern would paint under a brush, for a picture of it drawn
/// over the map (see [`pattern_preview`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PatternPreview {
    /// The brush's footprint.
    pub rect: BlockRect,
    /// Columns from one sample to the next, each way.
    pub step: i32,
    /// Samples across and down, from the footprint's corner.
    pub size: [usize; 2],
    /// The terrain of each sample, row by row; `None` off the brush.
    pub cells: Vec<Option<TerrainId>>,
}

/// What `pattern` would paint under the brush at `(cx, cz)`: every
/// `step`-th column of the footprint each way, the fewest that keeps the
/// picture within `max` samples a side. When every column is looked at,
/// the brush's filter counts too.
pub fn pattern_preview(dim: &Dimension, brush: &Brush, cx: f32, cz: f32, pattern: &crate::pattern::PatternSampler, max: usize) -> PatternPreview {
    use rayon::prelude::*;
    let rect = BlockRect::around(cx, cz, brush.reach());
    let side = rect.width().max(rect.depth()).max(1) as usize;
    let step = side.div_ceil(max.max(1)).max(1);
    let size = [(rect.width().max(0) as usize).div_ceil(step), (rect.depth().max(0) as usize).div_ceil(step)];
    let dab = (step == 1).then(|| Dab::on(dim, brush, cx, cz));
    let step = step as i32;
    let cells = (0..size[0] * size[1])
        .into_par_iter()
        .map(|k| {
            // The column in the middle of the sample's square.
            let x = rect.x0 + (k % size[0]) as i32 * step + step / 2;
            let z = rect.z0 + (k / size[0]) as i32 * step + step / 2;
            let w = match &dab {
                Some(dab) => dab.weight(x, z),
                None => brush.weight(x as f32 + 0.5 - cx, z as f32 + 0.5 - cz),
            };
            if w > 0.0 { pattern.terrain(x, z) } else { None }
        })
        .collect();
    PatternPreview { rect, step, size, cells }
}

/// Give the columns under the brush the biome their land calls for (see
/// [`crate::auto`]), as `classes` (for biomes) has it, worked out where it
/// is not yet. `salt` should change from dab to dab. Returns how many
/// columns changed.
pub fn auto_biome(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, reader: &crate::auto::Reader, classes: &mut crate::auto::Classes) -> usize {
    auto_apply(edit, brush, cx, cz, salt, reader, classes, |t| &mut t.biome)
}

/// Paint the columns under the brush with the terrain their biome calls
/// for (see [`crate::auto`]), as `classes` (for terrains) has it. `salt`
/// should change from dab to dab. Returns how many columns changed.
pub fn auto_paint(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, salt: u64, reader: &crate::auto::Reader, classes: &mut crate::auto::Classes) -> usize {
    auto_apply(edit, brush, cx, cz, salt, reader, classes, |t| &mut t.terrain)
}

#[allow(clippy::too_many_arguments)]
fn auto_apply(
    edit: &mut Edit,
    brush: &Brush,
    cx: f32,
    cz: f32,
    salt: u64,
    reader: &crate::auto::Reader,
    classes: &mut crate::auto::Classes,
    plane: impl Fn(&mut Tile) -> &mut crate::plane::Plane<u16> + Sync,
) -> usize {
    let footprint = BlockRect::around(cx, cz, brush.reach());
    classes.ensure(edit.dim(), reader, footprint);
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return 0;
    }
    let classes = &*classes;
    let changed = std::sync::atomic::AtomicUsize::new(0);
    edit.par_tiles(footprint.tiles(), footprint, false, |pos, clip, tile| {
        let Some(values) = classes.tile(pos) else { return };
        let slab = tile.bottom.is_some();
        let mut n = 0;
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
                let w = dab.weight(x, z);
                if !auto_hit(w, brush.strength, x, z, salt) {
                    continue;
                }
                let i = local_index(x, z);
                let v = values[i];
                if v == u16::MAX || slab && !tile.present(i) {
                    continue;
                }
                let plane = plane(tile);
                if plane.get(i) != v {
                    plane.set(i, v);
                    n += 1;
                }
            }
        }
        changed.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    });
    changed.into_inner()
}

/// What an automatic tool would do to a column under the brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// Off the brush.
    Outside,
    /// Under it, left as it is.
    Same,
    /// Given this terrain (or biome).
    Becomes(u16),
}

impl Preview {
    pub fn becomes(self) -> Option<u16> {
        match self {
            Preview::Becomes(v) => Some(v),
            _ => None,
        }
    }
}

/// What the automatic tools would change under the brush at `(cx, cz)`:
/// the footprint, and for each of its columns (row by row) the terrain
/// (`biomes` false) or biome it would get.
pub fn auto_preview(dim: &Dimension, brush: &Brush, cx: f32, cz: f32, reader: &crate::auto::Reader, biomes: bool) -> (BlockRect, Vec<Preview>) {
    let mut classes = crate::auto::Classes::new(biomes);
    classes.ensure(dim, reader, BlockRect::around(cx, cz, brush.reach()));
    auto_preview_with(dim, brush, cx, cz, &classes, 1)
}

/// [`auto_preview`] from what `classes` worked out (see
/// [`crate::auto::Classes::ensure`]), a column every `step` blocks each way
/// (a preview need not be finer than the screen: the first column of each
/// `step` × `step` square): the footprint, and the samples row by row,
/// ⌈`footprint.width()` / `step`⌉ a row.
pub fn auto_preview_with(dim: &Dimension, brush: &Brush, cx: f32, cz: f32, classes: &crate::auto::Classes, step: i32) -> (BlockRect, Vec<Preview>) {
    let step = step.max(1);
    let dab = Dab::on(dim, brush, cx, cz);
    let footprint = BlockRect::around(cx, cz, brush.reach());
    let (w, d) = ((footprint.width() + step - 1) / step, (footprint.depth() + step - 1) / step);
    // The samples whose coordinate, from `lo`, is within a..b.
    let samples = |lo: i32, n: i32, a: i32, b: i32| ((a - lo + step - 1) / step).min(n)..((b - lo + step - 1) / step).min(n);
    let biomes = classes.biomes();
    // Tile by tile, each read once.
    let parts: Vec<Vec<(usize, Preview)>> = footprint
        .tiles()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|pos| {
            let clip = footprint.intersect(&pos.rect());
            let ground = dim.ground().view(pos);
            let mut out = Vec::new();
            for j in samples(footprint.z0, d, clip.z0, clip.z1) {
                let z = footprint.z0 + j * step;
                for i in samples(footprint.x0, w, clip.x0, clip.x1) {
                    let x = footprint.x0 + i * step;
                    if dab.weight(x, z) <= 0.0 {
                        continue;
                    }
                    let k = (j * w + i) as usize;
                    let (Some(new), Some(ground)) = (classes.get(x, z), &ground) else {
                        out.push((k, Preview::Same));
                        continue;
                    };
                    let cell = local_index(x, z);
                    let old = if biomes { ground.biome.get(cell) } else { ground.terrain.get(cell) };
                    out.push((k, if old != new { Preview::Becomes(new) } else { Preview::Same }));
                }
            }
            out
        })
        .collect();
    let mut out = vec![Preview::Outside; (w * d) as usize];
    for (k, p) in parts.into_iter().flatten() {
        out[k] = p;
    }
    (footprint, out)
}

/// Set the fluid surface (or [`NO_FLUID`]) under the whole footprint.
pub fn set_fluid(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, level: i16) {
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let footprint = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(footprint.tiles(), footprint, false, |_, clip, tile| {
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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
    let rect = BlockRect::around(cx, cz, brush.reach());
    let dab = Dab::new(edit, brush, cx, cz);
    if dab.idle {
        return;
    }
    let inside: Vec<TilePos> = rect.tiles().filter(|p| edit.dim().ground().contains(*p)).collect();
    edit.par_tiles_with(inside, rect, true, |pos, clip, tile, others| {
        let Tile { top, bottom, .. } = tile;
        let Some(bottom) = bottom else { return };
        let (top, bottom) = (top.dense_mut(), bottom.dense_mut());
        let ground = others.get(0).and_then(|g| g.view(pos));
        let ground = ground.as_ref().map(|g| g.top.dense());
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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
    if dab.idle {
        return;
    }
    let rect = BlockRect::around(cx, cz, brush.reach());
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let Tile { top, bottom, .. } = tile;
        let Some(bottom) = bottom else { return };
        let (top, bottom) = (top.dense_mut(), bottom.dense_mut());
        for z in clip.z0..clip.z1 {
            for x in dab.row(z, &clip) {
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
    /// The selection the fill keeps to: columns under half its weight are
    /// walls.
    selection: Option<&'a crate::selection::Selection>,
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
        if self.selection.is_some_and(|s| s.get(x, z) < crate::selection::HALF) {
            return false;
        }
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

/// The level water poured at `(x, z)` rises to before it spills: the
/// hollow around it filled lowest first, up to the lowest point of its rim
/// (the water of a lake in it counts as ground at its surface). A fluid
/// level, as [`Flood::Fill`] takes it; `None` where the column is not in a
/// hollow and water poured there runs away. A hollow of more than `cap`
/// columns is filled to the level reached by then.
pub fn spill_level(dim: &Dimension, x: i32, z: i32, cap: usize) -> Option<i16> {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashSet};
    let mut ground = crate::river::Ground::new(dim.ground());
    let mut height = |x: i32, z: i32| ground.at(x, z).map(|(h, w)| w.map_or(block_top(h), |w| (w as i32).max(block_top(h))));
    let start = height(x, z)?;
    let mut level = start;
    let mut seen: HashSet<(i32, i32)> = HashSet::from([(x, z)]);
    let mut heap = BinaryHeap::from([Reverse((start, x, z))]);
    let mut inside = 0usize;
    while let Some(Reverse((h, cx, cz))) = heap.pop() {
        if h < level {
            // Lower than the water: it runs out here.
            break;
        }
        level = level.max(h);
        inside += 1;
        if inside > cap {
            break;
        }
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let n = (cx + dx, cz + dz);
            if seen.insert(n) {
                match height(n.0, n.1) {
                    Some(nh) => heap.push(Reverse((nh, n.0, n.1))),
                    // Off the map: the water runs off at this level.
                    None => heap.push(Reverse((i32::MIN, n.0, n.1))),
                }
            }
        }
    }
    (level > start).then(|| level.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
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
            selection: edit.dim().selection.as_deref(),
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

    /// A pit in a plateau at 80, a notch at 75 out to the edge: water poured in it
    /// rises to the notch, where it would spill; poured on the plateau, it
    /// runs away.
    #[test]
    fn spill_level_fills_the_hollow_clicked() {
        let mut w = world(1);
        let positions: Vec<TilePos> = w.dims[0].ground().positions().collect();
        for pos in positions {
            let tile = w.dims[0].ground_mut().tile_mut(pos).unwrap();
            let (ox, oz) = pos.origin();
            for c in 0..TILE_AREA {
                let (x, z) = (ox + (c % 128) as i32, oz + (c / 128) as i32);
                let d = (x as f32).hypot(z as f32);
                let mut h = if d < 20.0 { 70.0 } else { 80.0 };
                if x >= 19 && z.abs() <= 1 {
                    h = 75.0;
                }
                tile.top.set(c, h);
            }
        }
        assert_eq!(spill_level(&w.dims[0], 0, 0, 1 << 20), Some(75));
        assert_eq!(spill_level(&w.dims[0], 60, 60, 1 << 20), None);
        let mut hist = History::default();
        hist.begin("flood", 0);
        let n = flood_fill(&mut hist.edit(&mut w, 0), 0, 0, Flood::Fill(75), usize::MAX);
        hist.commit();
        // The pit fills; the plateau and the notch stay dry.
        assert!(n > 1000 && n < 1400, "{n}");
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

    /// Annotations are painted in their colour, rubbed out with 0, and
    /// undone as any paint.
    #[test]
    fn annotations_are_painted_rubbed_out_and_undone() {
        let mut w = world(1);
        let mut hist = History::default();
        let at = |w: &World, x: i32, z: i32| w.dims[0].ground().view(TilePos::containing(x, z)).unwrap().annotations.get(local_index(x, z));
        hist.begin("mark", 0);
        paint_annotation(&mut hist.edit(&mut w, 0), &hard_brush(6.0), 10.0, 10.0, 14, 1);
        hist.commit();
        assert_eq!((at(&w, 10, 10), at(&w, 15, 10), at(&w, 30, 10)), (14, 14, 0));
        // A colour past the fifteen is the last.
        hist.begin("mark", 0);
        paint_annotation(&mut hist.edit(&mut w, 0), &hard_brush(1.0), 40.0, 40.0, 99, 2);
        hist.commit();
        assert_eq!(at(&w, 40, 40), 15);
        hist.begin("rub out", 0);
        paint_annotation(&mut hist.edit(&mut w, 0), &hard_brush(2.0), 10.0, 10.0, 0, 3);
        hist.commit();
        assert_eq!((at(&w, 10, 10), at(&w, 15, 10)), (0, 14));
        hist.undo(&mut w);
        assert_eq!(at(&w, 10, 10), 14);
    }

    fn select(w: &mut World, hist: &mut History, s: crate::selection::Selection) {
        assert!(hist.select(w, 0, "Select", Some(s)));
    }

    fn heights(w: &mut World) -> Vec<(TilePos, Vec<f32>)> {
        let ground = w.dims[0].ground_mut();
        // In the order of the tiles, not of the map's table, which an undo
        // may change.
        let mut positions: Vec<TilePos> = ground.positions().collect();
        positions.sort_by_key(|p| (p.z, p.x));
        positions.into_iter().map(|p| (p, ground.tile_mut(p).unwrap().top.dense().to_vec())).collect()
    }

    #[test]
    fn a_brush_changes_nothing_outside_the_selection() {
        use crate::selection::{Selection, Shape};
        let mut w = world(2);
        let mut hist = History::default();
        select(&mut w, &mut hist, Selection::shape(&Shape::Rect(BlockRect::new(0, 0, 40, 40))));
        let before = heights(&mut w);
        let terrain = w.registry.terrain_by_name("Gravel").unwrap();
        // Strokes wholly outside: nothing at all, not even a step to undo.
        hist.begin("outside", 0);
        let mut e = hist.edit(&mut w, 0);
        raise(&mut e, &hard_brush(10.0), -60.0, -60.0, 5.0, Surface::Top);
        smooth(&mut e, &hard_brush(10.0), 100.0, 100.0, Surface::Top);
        paint_terrain(&mut e, &hard_brush(10.0), -60.0, 100.0, terrain, 1);
        assert!(!hist.commit());
        assert_eq!(heights(&mut w), before);
        // Across its edge: raised inside, as it was outside.
        hist.begin("across", 0);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(30.0), 0.0, 20.0, 5.0, Surface::Top);
        paint_terrain(&mut hist.edit(&mut w, 0), &hard_brush(30.0), 0.0, 20.0, terrain, 1);
        hist.commit();
        let h = |w: &World, x, z| height_at(&w.dims[0], 0, x, z, Surface::Top).unwrap();
        let t = |w: &World, x, z| w.dims[0].ground().view(TilePos::containing(x, z)).unwrap().terrain.get(local_index(x, z));
        assert_eq!(h(&w, 5, 20), 69.0);
        assert_eq!(h(&w, -5, 20), 64.0);
        assert_eq!(t(&w, 5, 20), terrain.0);
        assert_eq!(t(&w, -5, 20), 1);
        // Undone, the map is as it was; and the selection goes too.
        hist.undo(&mut w);
        assert_eq!(heights(&mut w), before);
        hist.undo(&mut w);
        assert!(w.dims[0].selection.is_none());
        hist.redo(&mut w);
        assert!(w.dims[0].selection.is_some());
    }

    #[test]
    fn a_feathered_selection_weighs_the_brush() {
        use crate::selection::{Selection, Shape};
        let mut w = world(2);
        let mut hist = History::default();
        select(&mut w, &mut hist, Selection::shape(&Shape::Rect(BlockRect::new(-100, -100, 0, 100))).feather(20));
        hist.begin("raise", 0);
        raise(&mut hist.edit(&mut w, 0), &hard_brush(60.0), 0.0, 0.0, 20.0, Surface::Top);
        hist.commit();
        let sel = w.dims[0].selection.clone().unwrap();
        let mut last = f32::MAX;
        for x in -30..30 {
            let got = height_at(&w.dims[0], 0, x, 0, Surface::Top).unwrap() - 64.0;
            let want = 20.0 * sel.weight(x, 0);
            assert!((got - want).abs() <= 0.06, "{x}: {got} for {want}");
            assert!(got <= last);
            last = got;
        }
        assert_eq!(height_at(&w.dims[0], 0, -30, 0, Surface::Top), Some(84.0));
        assert_eq!(height_at(&w.dims[0], 0, 30, 0, Surface::Top), Some(64.0));
        let mid = height_at(&w.dims[0], 0, 0, 0, Surface::Top).unwrap();
        assert!(mid > 66.0 && mid < 82.0, "{mid}");
    }

    #[test]
    fn a_fill_keeps_to_the_selection() {
        use crate::selection::{Selection, Shape};
        let mut w = world(1);
        let mut hist = History::default();
        select(&mut w, &mut hist, Selection::shape(&Shape::Ellipse(BlockRect::new(-20, -20, 20, 20))));
        hist.begin("fill", 0);
        let n = flood_fill(&mut hist.edit(&mut w, 0), 0, 0, Flood::Fill(70), 1_000_000);
        hist.commit();
        let area = w.dims[0].selection.as_ref().unwrap().area() as usize;
        assert_eq!(n, area);
    }

    #[test]
    fn a_pattern_preview_shows_what_the_brush_would_paint() {
        use crate::pattern::Pattern;
        let w = world(4);
        let reg = &w.registry;
        let sampler = Pattern::default().sampler(&[(reg.terrain_by_name("Grass").unwrap(), 1.0), (reg.terrain_by_name("Gravel").unwrap(), 1.0)]);
        // A small brush: every column, as painting would give it.
        let small = pattern_preview(&w.dims[0], &hard_brush(20.0), 10.0, -5.0, &sampler, 256);
        assert_eq!(small.step, 1);
        assert_eq!(small.size, [small.rect.width() as usize, small.rect.depth() as usize]);
        let at = |p: &PatternPreview, i: usize, j: usize| p.cells[j * p.size[0] + i];
        assert_eq!(at(&small, 0, 0), None, "a corner is off a round brush");
        let (i, j) = (small.size[0] / 2, small.size[1] / 2);
        assert_eq!(at(&small, i, j), sampler.terrain(small.rect.x0 + i as i32, small.rect.z0 + j as i32));
        let mut edit_world = world(4);
        let mut history = History::new(2);
        history.begin("paint", 0);
        paint_pattern(&mut history.edit(&mut edit_world, 0), &hard_brush(20.0), 10.0, -5.0, 1, &sampler);
        let (x, z) = (small.rect.x0 + i as i32, small.rect.z0 + j as i32);
        let tile = edit_world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
        assert_eq!(Some(TerrainId(tile.terrain.get(local_index(x, z)))), at(&small, i, j));
        // A big one: sampled, never more than asked.
        let big = pattern_preview(&w.dims[0], &hard_brush(1000.0), 0.0, 0.0, &sampler, 128);
        assert!(big.step > 1 && big.size[0] <= 128 && big.size[1] <= 128, "{} {:?}", big.step, big.size);
        assert!(big.cells.iter().filter(|c| c.is_some()).count() > big.cells.len() / 2);
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

    /// Flatten one way leaves the land on the other side of the height as
    /// it was; both ways, it levels both.
    #[test]
    fn flatten_modes_move_the_land_one_way_or_both() {
        let shaped = || {
            let mut w = world(1);
            let mut hist = History::default();
            hist.begin("Shape", 0);
            // A pit at 50 west of x 10, a hill at 80 east of it, on land at 64.
            set_height(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 4.0, 10.0, 50.0, Surface::Top);
            set_height(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 16.0, 10.0, 80.0, Surface::Top);
            hist.commit();
            (w, hist)
        };
        for (mode, pit, hill) in [(FlattenMode::Both, 64.0, 64.0), (FlattenMode::RaiseOnly, 64.0, 80.0), (FlattenMode::LowerOnly, 50.0, 64.0)] {
            let (mut w, mut hist) = shaped();
            hist.begin("Flatten", 0);
            for _ in 0..20 {
                flatten_with(&mut hist.edit(&mut w, 0), &hard_brush(12.0), 10.0, 10.0, 64.0, Surface::Top, mode, None);
            }
            hist.commit();
            assert_eq!((h(&w, 4, 10), h(&w, 16, 10)), (pit, hill), "{mode:?}");
            assert_eq!(h(&w, 10, 10), 64.0, "{mode:?}: the land at the height stays");
        }
    }

    /// With the theme, the columns flattened get the terrain their new
    /// height calls for, and those the mode leaves keep theirs.
    #[test]
    fn flatten_applies_the_theme_to_what_it_flattens() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Shape", 0);
        set_height(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 4.0, 10.0, 50.0, Surface::Top);
        set_height(&mut hist.edit(&mut w, 0), &hard_brush(3.0), 16.0, 10.0, 80.0, Surface::Top);
        hist.commit();
        let terrain = |w: &World, x: i32, z: i32| w.dims[0].ground().view(TilePos::containing(x, z)).unwrap().terrain.get(local_index(x, z));
        let before = terrain(&w, 16, 10);
        // Each height its own terrain, to see which columns were painted.
        let theme = |h: f32| TerrainId(100 + h as u16);
        hist.begin("Flatten", 0);
        for _ in 0..20 {
            flatten_with(&mut hist.edit(&mut w, 0), &hard_brush(12.0), 10.0, 10.0, 64.0, Surface::Top, FlattenMode::RaiseOnly, Some(&theme));
        }
        hist.commit();
        assert_eq!(terrain(&w, 4, 10), 164, "the pit, raised to 64");
        assert_eq!(terrain(&w, 10, 10), 164, "the land at the height");
        assert_eq!(terrain(&w, 16, 10), before, "the hill, left as it was");
        assert_eq!(terrain(&w, 40, 10), before, "outside the brush");
    }

    /// Flattening toward 63.8 the land at or below 63 brings it to 63
    /// exactly, from below and from 63.3 (at 63 in blocks), rather than
    /// leaving it at 63.1, where the test let go of it.
    #[test]
    fn flatten_stops_on_a_sharp_limit() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Flatten", 0);
        set_height(&mut hist.edit(&mut w, 0), &hard_brush(12.0), 10.0, 10.0, 60.4, Surface::Top);
        set_height(&mut hist.edit(&mut w, 0), &hard_brush(0.5), 10.5, 10.5, 63.3, Surface::Top);
        let mut brush = hard_brush(12.0);
        brush.strength = 0.3;
        brush.filter = Some(std::sync::Arc::new(crate::filter::Filter {
            below: Some(63),
            ..crate::filter::Filter::default()
        }));
        for _ in 0..30 {
            flatten(&mut hist.edit(&mut w, 0), &brush, 10.0, 10.0, 63.8, Surface::Top);
        }
        hist.commit();
        assert_eq!(h(&w, 5, 10), 63.0);
        assert_eq!(h(&w, 10, 10), 63.0);
        // Outside the brush, the land is as it was.
        assert_eq!(h(&w, 40, 10), 64.0);
    }

    /// A square turned 45° acts on all of its footprint, its corners too,
    /// not on the square of its radius alone.
    #[test]
    fn a_turned_square_reaches_its_corners() {
        let mut w = world(1);
        let mut hist = History::default();
        hist.begin("Raise", 0);
        let mut brush = hard_brush(10.0);
        brush.shape = BrushShape::Square;
        brush.rotation = 45.0;
        raise(&mut hist.edit(&mut w, 0), &brush, 0.0, 0.0, 5.0, Surface::Top);
        hist.commit();
        // 13 blocks east: in the turned square (its corner is 14.1 out), past
        // its radius.
        assert_eq!(h(&w, 13, 0), 69.0);
        assert_eq!(h(&w, 9, 9), 64.0);
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
