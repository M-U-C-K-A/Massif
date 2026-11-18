//! Brush filters, as in WorldPainter: where on the map a brush acts.
//!
//! A filter turns the world under a brush into a weight per column, from 0
//! (the brush does nothing here) to 1. It looks at what the map shows: the
//! height of the topmost surface, its terrain and biome, whether it is under
//! water, its slope and its curvature (hollows and bumps). Every test that is
//! switched on must pass; the height band can fade out over a few blocks.

use crate::compose::{Span, TileStack, block_top};
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::tile::NO_FLUID;
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Blocks over which "fade at the limits" softens the height band.
pub const FADE: f32 = 4.0;

/// Something a column can be "on".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Match {
    /// The terrain of the surface.
    Terrain(u16),
    /// The biome painted on the column (0 = automatic).
    Biome(u16),
    /// Under water (or another fluid).
    Water,
    /// Dry land.
    Land,
}

/// The tests of a filter. The default filter lets everything through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    /// Act only at or above this height.
    pub above: Option<i32>,
    /// Act only at or below this height.
    pub below: Option<i32>,
    /// With both limits: act outside the band instead of inside it.
    pub beyond: bool,
    /// Soften the height limits over [`FADE`] blocks.
    pub fade: bool,
    /// Act only on columns matching one of these.
    pub only_on: Option<Vec<Match>>,
    /// Never act on columns matching one of these.
    pub except_on: Option<Vec<Match>>,
    /// Act only where the slope is at least / at most `slope` degrees.
    pub slope_above: bool,
    pub slope_below: bool,
    pub slope: f32,
    /// Act only in hollows / on bumps: columns at least `depth` blocks
    /// below / above the mean of the `radius` blocks around them.
    pub hollows: bool,
    pub bumps: bool,
    pub depth: f32,
    pub radius: i32,
    /// Act only on slopes that face `facing` (degrees: 0 north, 90 east),
    /// give or take `facing_spread` degrees: the side the sun lights, the
    /// side in the shade. Level ground faces nowhere.
    pub facing_on: bool,
    pub facing: f32,
    pub facing_spread: f32,
}

/// Sharp height limits, as a height tool keeps to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    above: Option<i32>,
    below: Option<i32>,
    /// Act outside the band (both limits set).
    beyond: bool,
}

impl Band {
    /// The height `new` that a tool gives a column at `old`, kept on the
    /// side of the limits `old` is on: land at or below 63 raised or
    /// flattened toward 63.8 stops at 63.0 (rather than at 63.1, where the
    /// test lets go of it); land at or above 70 lowered stops at 70.0.
    /// Outside a band it acts beyond, the land stays outside it. A column
    /// whose top is not where the test acts (the brush reached it through
    /// another level) is left as the tool has it.
    pub fn hold(&self, old: f32, new: f32) -> f32 {
        let y = block_top(old);
        if self.beyond {
            let (a, b) = (self.above.unwrap_or(i32::MIN), self.below.unwrap_or(i32::MAX));
            return if y < a {
                new.min((a - 1) as f32)
            } else if y > b {
                new.max((b + 1) as f32)
            } else {
                new
            };
        }
        if self.above.is_some_and(|a| y < a) || self.below.is_some_and(|b| y > b) {
            return new;
        }
        let mut h = new;
        if let Some(b) = self.below {
            h = h.min(b as f32);
        }
        if let Some(a) = self.above {
            h = h.max(a as f32);
        }
        h
    }
}

/// Degrees over which the facing test softens at its limits.
pub const FACING_FADE: f32 = 12.0;
/// Slopes gentler than this (degrees) face nowhere.
pub const FACING_MIN_SLOPE: f32 = 4.0;

impl Default for Filter {
    fn default() -> Self {
        Self {
            above: None,
            below: None,
            beyond: false,
            fade: false,
            only_on: None,
            except_on: None,
            slope_above: false,
            slope_below: false,
            slope: 45.0,
            hollows: false,
            bumps: false,
            depth: 1.0,
            radius: 4,
            facing_on: false,
            facing: 180.0,
            facing_spread: 60.0,
        }
    }
}

/// What a filter knows of one column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub height: f32,
    pub terrain: u16,
    pub biome: u16,
    pub wet: bool,
    /// Degrees.
    pub slope: f32,
    /// Height minus the mean height around.
    pub relief: f32,
    /// The way the slope faces, in degrees (0 north, 90 east); `None`
    /// when not worked out.
    pub aspect: Option<f32>,
}

impl Filter {
    /// Whether any test is on.
    pub fn is_active(&self) -> bool {
        self.above.is_some()
            || self.below.is_some()
            || self.only_on.is_some()
            || self.except_on.is_some()
            || self.slope_above
            || self.slope_below
            || self.hollows
            || self.bumps
            || self.facing_on
    }

    fn needs_slope(&self) -> bool {
        self.slope_above || self.slope_below || self.facing_on
    }

    /// Weight of the facing test for a slope of `slope` degrees that faces
    /// `aspect`: 1 within the spread, fading to 0 just outside it.
    pub fn facing_weight(&self, slope: f32, aspect: Option<f32>) -> f32 {
        if !self.facing_on {
            return 1.0;
        }
        let Some(aspect) = aspect.filter(|_| slope >= FACING_MIN_SLOPE) else { return 0.0 };
        let away = ((aspect - self.facing + 540.0).rem_euclid(360.0) - 180.0).abs();
        let spread = self.facing_spread.clamp(1.0, 180.0);
        ((spread - away) / FACING_FADE + 0.5).clamp(0.0, 1.0)
    }

    fn needs_relief(&self) -> bool {
        self.hollows || self.bumps
    }

    /// Weight of the height tests at height `h`.
    ///
    /// The tests are in whole blocks, as the game has the land: a column is
    /// at the height of its top ([`block_top`]), whatever fraction of a
    /// block it carries (a map from WorldPainter keeps them) — at 63.2 it is
    /// at 63, at or below 63 like one at 63.0.
    pub fn height_weight(&self, h: f32) -> f32 {
        let y = block_top(h);
        let fade = if self.fade { FADE } else { 0.0 };
        // How far inside a limit the column is, in blocks: 1 well inside, 0
        // outside.
        let inside = |d: i32| {
            if fade > 0.0 {
                (d as f32 / fade + 0.5).clamp(0.0, 1.0)
            } else if d >= 0 {
                1.0
            } else {
                0.0
            }
        };
        let lo = self.above.map(|a| inside(y - a));
        let hi = self.below.map(|b| inside(b - y));
        match (lo, hi) {
            (None, None) => 1.0,
            (Some(w), None) | (None, Some(w)) => w,
            (Some(a), Some(b)) if self.beyond => (1.0 - a).max(1.0 - b),
            (Some(a), Some(b)) => a.min(b),
        }
    }

    /// The limits a height tool keeps to, when they are sharp (no fade):
    /// see [`Band::hold`].
    pub fn band(&self) -> Option<Band> {
        (!self.fade && (self.above.is_some() || self.below.is_some())).then_some(Band {
            above: self.above,
            below: self.below,
            beyond: self.beyond && self.above.is_some() && self.below.is_some(),
        })
    }

    fn matches(c: &Column, m: &Match) -> bool {
        match *m {
            Match::Terrain(t) => c.terrain == t,
            Match::Biome(b) => c.biome == b,
            Match::Water => c.wet,
            Match::Land => !c.wet,
        }
    }

    /// The weight of one column, from 0 to 1.
    pub fn weight(&self, c: &Column) -> f32 {
        if let Some(only) = &self.only_on
            && !only.iter().any(|m| Self::matches(c, m))
        {
            return 0.0;
        }
        if let Some(except) = &self.except_on
            && except.iter().any(|m| Self::matches(c, m))
        {
            return 0.0;
        }
        if self.slope_above || self.slope_below {
            let ok = (self.slope_above && c.slope >= self.slope) || (self.slope_below && c.slope <= self.slope);
            if !ok {
                return 0.0;
            }
        }
        if self.needs_relief() {
            let ok = (self.hollows && -c.relief >= self.depth) || (self.bumps && c.relief >= self.depth);
            if !ok {
                return 0.0;
            }
        }
        self.height_weight(c.height) * self.facing_weight(c.slope, c.aspect)
    }

    /// The weights of the columns of `rect` (row-major), worked out by
    /// squares of tiles in parallel, each read with the ring around it, so
    /// that a brush of any size takes little memory at a time.
    pub fn mask(&self, dim: &Dimension, rect: BlockRect) -> Mask {
        self.mask_rows(dim, rect, &|_, x0, x1| x0..x1)
    }

    /// [`Filter::mask`], weighed only where it is wanted: on row `z`, at
    /// the columns `rows(z, x0, x1)` of `x0..x1` (0 elsewhere).
    pub fn mask_rows(&self, dim: &Dimension, rect: BlockRect, rows: &(dyn Fn(i32, i32, i32) -> std::ops::Range<i32> + Sync)) -> Mask {
        let mut ring = 0;
        if self.needs_relief() {
            ring = self.radius.clamp(1, 64);
        }
        if self.needs_slope() {
            ring = ring.max(SLOPE_REACH);
        }
        let (w, d) = (rect.width() as usize, rect.depth() as usize);
        let side = 4 * TILE_SIZE;
        let mut parts = Vec::new();
        let mut z = rect.z0;
        while z < rect.z1 {
            let z1 = ((z.div_euclid(side) + 1) * side).min(rect.z1);
            let mut x = rect.x0;
            while x < rect.x1 {
                let x1 = ((x.div_euclid(side) + 1) * side).min(rect.x1);
                parts.push(BlockRect::new(x, z, x1, z1));
                x = x1;
            }
            z = z1;
        }
        let done: Vec<(BlockRect, Vec<u8>)> = parts
            .into_par_iter()
            .map(|part| {
                let surface = Surface::gather(dim, part.expand(ring));
                let relief = self.needs_relief().then(|| surface.local_means(ring));
                // The slope of every column, from the heights alone.
                let (sw, sd) = (surface.rect.width() as usize, surface.rect.depth() as usize);
                let gradients = self.needs_slope().then(|| {
                    let heights: Vec<f32> = surface.columns.iter().map(|c| c.map_or(f32::NAN, |c| c.height)).collect();
                    trend_gradients(&heights, sw, sd, SLOPE_REACH)
                });
                let pw = part.width() as usize;
                let mut out = vec![0u8; pw * part.depth() as usize];
                out.par_chunks_mut(pw).enumerate().for_each(|(j, row)| {
                    let z = part.z0 + j as i32;
                    for x in rows(z, part.x0, part.x1) {
                        let v = &mut row[(x - part.x0) as usize];
                        let Some(mut c) = surface.column(x, z) else { continue };
                        if let Some(gradients) = &gradients {
                            let (gx, gz) = gradients[surface.index(x, z)];
                            c.slope = gx.hypot(gz).atan().to_degrees();
                            if self.facing_on {
                                c.aspect = aspect_of(gx, gz);
                            }
                        }
                        if let Some(means) = &relief {
                            let m = means[surface.index(x, z)];
                            c.relief = if m.is_nan() { 0.0 } else { c.height - m };
                        }
                        *v = (self.weight(&c) * 255.0).round() as u8;
                    }
                });
                (part, out)
            })
            .collect();
        let mut weights = vec![0u8; w * d];
        for (part, out) in done {
            let pw = part.width() as usize;
            for (j, row) in out.chunks(pw).enumerate() {
                let at = ((part.z0 - rect.z0) as usize + j) * w + (part.x0 - rect.x0) as usize;
                weights[at..at + pw].copy_from_slice(row);
            }
        }
        Mask { rect, weights }
    }
}

/// Blocks around a column the slope is measured over, each way.
pub const SLOPE_REACH: i32 = 4;

/// The slope, in degrees, of the plane that best fits the heights around a
/// point (least squares over a square of `2 r + 1` samples a side): the
/// trend of the hillside there, not the steps between single blocks.
/// `h(dx, dz)` is the height `dx`, `dz` samples away (`None` off the map),
/// `spacing` the blocks between samples.
pub fn trend_slope(h: impl Fn(i32, i32) -> Option<f32>, r: i32, spacing: f32) -> f32 {
    let (gx, gz) = trend_gradient(h, r, spacing);
    gx.hypot(gz).atan().to_degrees()
}

/// The way a hillside faces, in degrees (0 north, 90 east), from the rise
/// of its heights toward the east (`gx`) and the south (`gz`): the way
/// water would run down it. `None` on level ground.
pub fn aspect_of(gx: f32, gz: f32) -> Option<f32> {
    if gx == 0.0 && gz == 0.0 {
        return None;
    }
    // Downhill is (−gx, −gz); north is −z.
    Some((-gx).atan2(gz).to_degrees().rem_euclid(360.0))
}

/// The rise of the best-fitting plane (see [`trend_slope`]) toward the east
/// and toward the south, in blocks per block.
pub fn trend_gradient(h: impl Fn(i32, i32) -> Option<f32>, r: i32, spacing: f32) -> (f32, f32) {
    let Some(centre) = h(0, 0) else { return (0.0, 0.0) };
    let (mut sx, mut sz, mut w) = (0.0f32, 0.0f32, 0.0f32);
    for dz in -r..=r {
        for dx in -r..=r {
            // Missing samples count as level with the centre.
            let v = h(dx, dz).unwrap_or(centre) - centre;
            sx += dx as f32 * v;
            sz += dz as f32 * v;
            w += (dx * dx) as f32;
        }
    }
    if w == 0.0 {
        return (0.0, 0.0);
    }
    (sx / w / spacing, sz / w / spacing)
}

/// [`trend_gradient`] at every point of a `w` × `d` grid of heights (row
/// by row, NaN where there is none), at once: the sums over each square
/// split into a pass along the rows and one along the columns, each a few
/// additions a point, rather than the whole square summed point by point.
/// Each pass runs over whole rows at a time, in parallel, the sums at
/// every point made in the same order as one point at a time would.
pub fn trend_gradients(heights: &[f32], w: usize, d: usize, r: i32) -> Vec<(f32, f32)> {
    let r = r as isize;
    // Along the rows: Σ dx·f and Σ f over the row's part of the square, for
    // the heights where there are some (0 elsewhere) and for where there
    // are (1 or 0).
    let (mut hm, mut hs, mut pm, mut ps) = (vec![0f32; w * d], vec![0f32; w * d], vec![0f32; w * d], vec![0f32; w * d]);
    hm.par_chunks_mut(w.max(1))
        .zip(hs.par_chunks_mut(w.max(1)))
        .zip(pm.par_chunks_mut(w.max(1)).zip(ps.par_chunks_mut(w.max(1))))
        .enumerate()
        .for_each(|(z, ((hm, hs), (pm, ps)))| {
            let row = &heights[z * w..][..w];
            let h: Vec<f32> = row.iter().map(|v| if v.is_nan() { 0.0 } else { *v }).collect();
            let p: Vec<f32> = row.iter().map(|v| if v.is_nan() { 0.0 } else { 1.0 }).collect();
            for dx in -r..=r {
                // The points x whose x + dx is on the row.
                let (x0, x1) = ((-dx).max(0) as usize, (w as isize - dx.max(0)).max(0) as usize);
                if x0 >= x1 {
                    continue;
                }
                let k = dx as f32;
                let at = (x0 as isize + dx) as usize;
                let (h, p) = (&h[at..at + (x1 - x0)], &p[at..at + (x1 - x0)]);
                for (((((hm, hs), pm), ps), &h), &p) in hm[x0..x1].iter_mut().zip(&mut hs[x0..x1]).zip(&mut pm[x0..x1]).zip(&mut ps[x0..x1]).zip(h).zip(p) {
                    *hm += k * h;
                    *hs += h;
                    *pm += k * p;
                    *ps += p;
                }
            }
        });
    // Then down the columns: Σ over dz of the first, Σ dz·(…) of the second.
    let weight = (2 * r + 1) as f32 * (-r..=r).map(|k| (k * k) as f32).sum::<f32>();
    let mut out = vec![(0f32, 0f32); w * d];
    out.par_chunks_mut(w.max(1)).enumerate().for_each(|(z, out)| {
        let (mut hx, mut hz, mut px, mut pz) = (vec![0f32; w], vec![0f32; w], vec![0f32; w], vec![0f32; w]);
        for dz in -r..=r {
            let j = z as isize + dz;
            if !(0..d as isize).contains(&j) {
                continue;
            }
            let k = dz as f32;
            let at = j as usize * w;
            let (hm, hs, pm, ps) = (&hm[at..at + w], &hs[at..at + w], &pm[at..at + w], &ps[at..at + w]);
            for x in 0..w {
                hx[x] += hm[x];
                px[x] += pm[x];
                hz[x] += k * hs[x];
                pz[x] += k * ps[x];
            }
        }
        // Σ dx (h − centre) over the heights there are: missing ones count
        // as level with the centre.
        for (x, o) in out.iter_mut().enumerate() {
            let c = heights[z * w + x];
            if !c.is_nan() {
                *o = ((hx[x] - c * px[x]) / weight, (hz[x] - c * pz[x]) / weight);
            }
        }
    });
    out
}

/// The weights of a filter over a rectangle.
#[derive(Debug, Clone)]
pub struct Mask {
    pub rect: BlockRect,
    weights: Vec<u8>,
}

impl Mask {
    /// A mask of the given weights (row-major over `rect`).
    pub(crate) fn from_weights(rect: BlockRect, weights: Vec<u8>) -> Self {
        debug_assert_eq!(weights.len(), rect.width() as usize * rect.depth() as usize);
        Self { rect, weights }
    }

    /// Multiply every weight by the matching one of `other` (the same rect).
    pub(crate) fn times(&mut self, other: &[u8]) {
        for (w, &o) in self.weights.iter_mut().zip(other) {
            *w = ((*w as u32 * o as u32 + 127) / 255) as u8;
        }
    }

    /// Weight of column `(x, z)`, 0 outside the rectangle.
    #[inline]
    pub fn get(&self, x: i32, z: i32) -> f32 {
        let r = &self.rect;
        if x < r.x0 || z < r.z0 || x >= r.x1 || z >= r.z1 {
            return 0.0;
        }
        self.weights[(z - r.z0) as usize * r.width() as usize + (x - r.x0) as usize] as f32 / 255.0
    }
}

/// The composed surface of a rectangle of columns.
struct Surface {
    rect: BlockRect,
    columns: Vec<Option<Column>>,
}

impl Surface {
    fn gather(dim: &Dimension, rect: BlockRect) -> Self {
        let width = rect.width() as usize;
        let mut columns: Vec<Option<Column>> = vec![None; width * rect.depth() as usize];
        // In bands, one per row of tiles, filled in parallel; the ground
        // alone (most tiles) read from its planes as they are.
        let mut bands: Vec<(i32, &mut [Option<Column>])> = Vec::new();
        let mut rest: &mut [Option<Column>] = &mut columns;
        let mut z = rect.z0;
        while z < rect.z1 {
            let end = ((z.div_euclid(TILE_SIZE) + 1) * TILE_SIZE).min(rect.z1);
            let (band, tail) = std::mem::take(&mut rest).split_at_mut((end - z) as usize * width);
            bands.push((z, band));
            rest = tail;
            z = end;
        }
        bands.into_par_iter().for_each(|(z0, band)| {
            let tz = z0.div_euclid(TILE_SIZE);
            let mut spans: Vec<Span> = Vec::new();
            for tx in rect.x0.div_euclid(TILE_SIZE)..=(rect.x1 - 1).div_euclid(TILE_SIZE) {
                let pos = TilePos::new(tx, tz);
                let Some(stack) = TileStack::from_above(dim, pos, false) else { continue };
                let clip = rect.intersect(&pos.rect());
                let ground = stack.ground();
                let flat = stack.is_ground_only().then(|| (ground.top.dense(), ground.terrain.dense()));
                let (fluids, biomes) = (ground.fluid.dense(), ground.biome.dense());
                for z in clip.z0..clip.z1 {
                    let row = &mut band[(z - z0) as usize * width..][..width];
                    for x in clip.x0..clip.x1 {
                        let cell = local_index(x, z);
                        let (height, terrain) = match &flat {
                            Some((tops, terrains)) => (tops[cell], terrains[cell]),
                            None => match stack.surface(cell, &mut spans) {
                                Some((h, t)) => (h, t.0),
                                None => continue,
                            },
                        };
                        let fluid = fluids[cell];
                        row[(x - rect.x0) as usize] = Some(Column {
                            height,
                            terrain,
                            biome: biomes[cell],
                            wet: fluid != NO_FLUID && fluid as f32 > height,
                            slope: 0.0,
                            relief: 0.0,
                            aspect: None,
                        });
                    }
                }
            }
        });
        Self { rect, columns }
    }

    #[inline]
    fn index(&self, x: i32, z: i32) -> usize {
        (z - self.rect.z0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize
    }

    fn column(&self, x: i32, z: i32) -> Option<Column> {
        let r = &self.rect;
        if x < r.x0 || z < r.z0 || x >= r.x1 || z >= r.z1 {
            return None;
        }
        self.columns[self.index(x, z)]
    }

    /// Mean height of the `(2k + 1)²` columns around each one (NaN where
    /// none is on the map).
    fn local_means(&self, k: i32) -> Vec<f32> {
        let (w, d) = (self.rect.width() as usize, self.rect.depth() as usize);
        let k = k as usize;
        let mut sum: Vec<f32> = self.columns.iter().map(|c| c.map_or(0.0, |c| c.height)).collect();
        let mut count: Vec<f32> = self.columns.iter().map(|c| c.is_some() as u8 as f32).collect();
        for buf in [&mut sum, &mut count] {
            let mut tmp = vec![0f32; w * d];
            // Rows, then columns: a box filter in two passes.
            crate::ops::box_pass(buf, &mut tmp, d, w, k, false, w);
            crate::ops::box_pass(&tmp, buf, w, d, k, true, w);
        }
        sum.iter().zip(&count).map(|(s, c)| if *c > 0.0 { s / c } else { f32::NAN }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gradients as they were first worked out, point by point down the
    /// columns: what [`trend_gradients`] must still give, bit for bit.
    fn trend_gradients_one_by_one(heights: &[f32], w: usize, d: usize, r: i32) -> Vec<(f32, f32)> {
        let r = r as isize;
        // Heights where there are some (0 elsewhere), and where there are.
        let h: Vec<f32> = heights.iter().map(|v| if v.is_nan() { 0.0 } else { *v }).collect();
        let p: Vec<f32> = heights.iter().map(|v| if v.is_nan() { 0.0 } else { 1.0 }).collect();
        // Along the rows: Σ dx·f and Σ f over the row's part of the square.
        let along = |f: &[f32]| -> (Vec<f32>, Vec<f32>) {
            let (mut moment, mut sum) = (vec![0f32; w * d], vec![0f32; w * d]);
            for z in 0..d {
                let row = &f[z * w..][..w];
                for x in 0..w as isize {
                    let (mut m, mut s) = (0.0, 0.0);
                    for dx in -r..=r {
                        let i = x + dx;
                        if (0..w as isize).contains(&i) {
                            let v = row[i as usize];
                            m += dx as f32 * v;
                            s += v;
                        }
                    }
                    moment[z * w + x as usize] = m;
                    sum[z * w + x as usize] = s;
                }
            }
            (moment, sum)
        };
        let (hm, hs) = along(&h);
        let (pm, ps) = along(&p);
        // Then down the columns: Σ over dz of the first, Σ dz·(…) of the second.
        let weight = (2 * r + 1) as f32 * (-r..=r).map(|k| (k * k) as f32).sum::<f32>();
        let mut out = vec![(0f32, 0f32); w * d];
        for z in 0..d as isize {
            for x in 0..w {
                let k = z as usize * w + x;
                if heights[k].is_nan() {
                    continue;
                }
                let (mut hx, mut hz, mut px, mut pz) = (0f32, 0f32, 0f32, 0f32);
                for dz in -r..=r {
                    let j = z + dz;
                    if (0..d as isize).contains(&j) {
                        let i = j as usize * w + x;
                        hx += hm[i];
                        px += pm[i];
                        hz += dz as f32 * hs[i];
                        pz += dz as f32 * ps[i];
                    }
                }
                // Σ dx (h − centre) over the heights there are: missing ones
                // count as level with the centre.
                let c = heights[k];
                out[k] = ((hx - c * px) / weight, (hz - c * pz) / weight);
            }
        }
        out
    }

    #[test]
    fn gradients_by_rows_are_bit_for_bit_those_point_by_point() {
        for (w, d, r) in [(1usize, 1usize, 4), (3, 40, 4), (61, 47, 4), (130, 9, 2), (9, 9, 6)] {
            let mut heights: Vec<f32> = (0..w * d).map(|i| (i as f32 * 0.37).sin() * 40.0 + 64.0 + (i % 7) as f32 * 0.125).collect();
            for k in (0..w * d).filter(|k| (k * 7919) % 11 == 3) {
                heights[k] = f32::NAN;
            }
            let (new, old) = (trend_gradients(&heights, w, d, r), trend_gradients_one_by_one(&heights, w, d, r));
            let bits = |g: &[(f32, f32)]| g.iter().map(|(a, b)| (a.to_bits(), b.to_bits())).collect::<Vec<_>>();
            assert_eq!(bits(&new), bits(&old), "{w} × {d}, r {r}");
        }
    }

    #[test]
    fn gradients_at_once_are_those_one_by_one() {
        let (w, d) = (23usize, 17usize);
        let mut heights: Vec<f32> = (0..w * d).map(|i| ((i % w) as f32 * 0.7 + (i / w) as f32 * 1.3 + ((i * 7919) % 13) as f32 * 0.4).floor()).collect();
        // Some columns missing.
        for k in [0, 5, 40, 41, 200, 390] {
            heights[k] = f32::NAN;
        }
        let all = trend_gradients(&heights, w, d, SLOPE_REACH);
        for z in 0..d as i32 {
            for x in 0..w as i32 {
                let k = z as usize * w + x as usize;
                if heights[k].is_nan() {
                    continue;
                }
                let one = trend_gradient(
                    |dx, dz| {
                        let (sx, sz) = (x + dx, z + dz);
                        if sx < 0 || sz < 0 || sx >= w as i32 || sz >= d as i32 {
                            return None;
                        }
                        let h = heights[sz as usize * w + sx as usize];
                        (!h.is_nan()).then_some(h)
                    },
                    SLOPE_REACH,
                    1.0,
                );
                assert!((all[k].0 - one.0).abs() < 1e-3 && (all[k].1 - one.1).abs() < 1e-3, "at {x}, {z}: {:?} vs {one:?}", all[k]);
            }
        }
    }

    #[test]
    fn slopes_follow_the_hillside_not_the_steps() {
        // A staircase of one block every two: 26.6° on the whole, though
        // every step is either flat or a vertical block.
        let stairs = |dx: i32, _dz: i32| Some((dx.div_euclid(2)) as f32);
        let s = trend_slope(stairs, SLOPE_REACH, 1.0);
        assert!((s - 26.57).abs() < 3.0, "{s}");
        // Flat ground, and a 45° plane, whatever the spacing.
        assert_eq!(trend_slope(|_, _| Some(64.0), 4, 1.0), 0.0);
        let plane = |dx: i32, dz: i32| Some((dx + dz) as f32 * 4.0);
        let s = trend_slope(plane, 2, 4.0);
        assert!((s - 54.74).abs() < 0.1, "{s}");
        // Off the map counts as level.
        assert_eq!(trend_slope(|dx, dz| (dx == 0 && dz == 0).then_some(10.0), 3, 1.0), 0.0);
    }
    use crate::tile::Tile;
    use crate::world::DimensionKind;

    fn column(height: f32) -> Column {
        Column {
            height,
            terrain: 1,
            biome: 0,
            wet: false,
            slope: 10.0,
            relief: 0.0,
            aspect: None,
        }
    }

    #[test]
    fn slopes_face_the_way_water_runs() {
        // Rising toward the south: the hillside faces north.
        assert_eq!(aspect_of(0.0, 1.0), Some(0.0));
        // Rising toward the east: it faces west.
        assert_eq!(aspect_of(1.0, 0.0), Some(270.0));
        assert_eq!(aspect_of(-1.0, 0.0), Some(90.0));
        assert_eq!(aspect_of(0.0, -1.0), Some(180.0));
        assert_eq!(aspect_of(0.0, 0.0), None);
        let south = Filter {
            facing_on: true,
            facing: 180.0,
            facing_spread: 45.0,
            ..Filter::default()
        };
        assert!(south.is_active());
        assert_eq!(south.facing_weight(30.0, Some(180.0)), 1.0);
        assert_eq!(south.facing_weight(30.0, Some(200.0)), 1.0);
        assert_eq!(south.facing_weight(30.0, Some(0.0)), 0.0, "the north side");
        assert_eq!(south.facing_weight(30.0, Some(270.0)), 0.0);
        assert_eq!(south.facing_weight(1.0, Some(180.0)), 0.0, "level ground faces nowhere");
        // The limit is soft.
        let edge = south.facing_weight(30.0, Some(225.0));
        assert!((edge - 0.5).abs() < 1e-4, "{edge}");
        // Across north: 350° is 20° from 10°.
        let north = Filter {
            facing_on: true,
            facing: 10.0,
            facing_spread: 30.0,
            ..Filter::default()
        };
        assert_eq!(north.facing_weight(20.0, Some(350.0)), 1.0);
    }

    #[test]
    fn the_facing_test_reads_the_map() {
        // A pyramid: each of its four sides faces its own way.
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(60.0, 1, NO_FLUID);
        for z in 0..128i32 {
            for x in 0..128i32 {
                let d = (x - 64).abs().max((z - 64).abs());
                tile.top.set(local_index(x, z), 60.0 + (50 - d).max(0) as f32);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        let rect = BlockRect::new(0, 0, 128, 128);
        let facing = |degrees: f32| {
            Filter {
                facing_on: true,
                facing: degrees,
                facing_spread: 40.0,
                ..Filter::default()
            }
            .mask(&dim, rect)
        };
        // North is −z: the north side is above the middle on the map.
        let north = facing(0.0);
        assert_eq!(north.get(64, 34), 1.0);
        assert_eq!(north.get(64, 94), 0.0);
        assert_eq!(north.get(34, 64), 0.0);
        let east = facing(90.0);
        assert_eq!(east.get(94, 64), 1.0);
        assert_eq!(east.get(34, 64), 0.0);
        let south = facing(180.0);
        assert_eq!(south.get(64, 94), 1.0);
        // The plain around faces nowhere.
        assert_eq!(south.get(5, 120), 0.0);
    }

    #[test]
    fn the_default_filter_lets_everything_through() {
        let f = Filter::default();
        assert!(!f.is_active());
        assert_eq!(f.weight(&column(-30.0)), 1.0);
    }

    #[test]
    fn height_bands_with_and_without_fade() {
        let mut f = Filter {
            above: Some(60),
            below: Some(100),
            ..Filter::default()
        };
        assert_eq!(f.height_weight(59.0), 0.0);
        assert_eq!(f.height_weight(60.0), 1.0);
        assert_eq!(f.height_weight(100.0), 1.0);
        assert_eq!(f.height_weight(101.0), 0.0);
        f.beyond = true;
        assert_eq!(f.height_weight(80.0), 0.0);
        assert_eq!(f.height_weight(30.0), 1.0);
        assert_eq!(f.height_weight(130.0), 1.0);
        f.beyond = false;
        f.fade = true;
        assert_eq!(f.height_weight(60.0), 0.5);
        assert!(f.height_weight(57.0) < 0.01);
        assert!(f.height_weight(63.0) > 0.99);
        let only_above = Filter {
            above: Some(70),
            ..Filter::default()
        };
        assert_eq!(only_above.height_weight(500.0), 1.0);
        assert_eq!(only_above.height_weight(69.4), 0.0);
        // In whole blocks: 69.9 and 63.2 are at 70 and 63.
        assert_eq!(only_above.height_weight(69.9), 1.0);
        let at_or_below = Filter {
            below: Some(63),
            ..Filter::default()
        };
        assert_eq!(at_or_below.height_weight(63.2), 1.0);
        assert_eq!(at_or_below.height_weight(63.0), 1.0);
        assert_eq!(at_or_below.height_weight(63.5), 0.0);
        assert_eq!(at_or_below.height_weight(63.8), 0.0);
    }

    /// Height tools stop at sharp limits, on whole blocks.
    #[test]
    fn height_tools_stop_at_the_limits() {
        let at_or_below = Filter {
            below: Some(63),
            ..Filter::default()
        }
        .band()
        .unwrap();
        // Flattened toward 63.8: stops at 63, from below or from 63.3.
        assert_eq!(at_or_below.hold(60.0, 63.1), 63.0);
        assert_eq!(at_or_below.hold(63.3, 63.5), 63.0);
        assert_eq!(at_or_below.hold(60.0, 62.4), 62.4);
        // Above the limit, the test does not act: left alone.
        assert_eq!(at_or_below.hold(70.0, 75.0), 75.0);
        let above = Filter {
            above: Some(70),
            ..Filter::default()
        }
        .band()
        .unwrap();
        assert_eq!(above.hold(74.0, 66.0), 70.0);
        let beyond = Filter {
            above: Some(60),
            below: Some(80),
            beyond: true,
            ..Filter::default()
        }
        .band()
        .unwrap();
        assert_eq!(beyond.hold(50.0, 65.0), 59.0);
        assert_eq!(beyond.hold(90.0, 70.0), 81.0);
        // Faded limits soften the tests: no sharp limit to keep to.
        let faded = Filter {
            below: Some(63),
            fade: true,
            ..Filter::default()
        };
        assert_eq!(faded.band(), None);
    }

    #[test]
    fn only_on_except_on_slope_and_relief() {
        let mut c = column(64.0);
        let only_grass = Filter {
            only_on: Some(vec![Match::Terrain(1), Match::Water]),
            ..Filter::default()
        };
        assert_eq!(only_grass.weight(&c), 1.0);
        c.terrain = 5;
        assert_eq!(only_grass.weight(&c), 0.0);
        c.wet = true;
        assert_eq!(only_grass.weight(&c), 1.0);
        let not_wet = Filter {
            except_on: Some(vec![Match::Water]),
            ..Filter::default()
        };
        assert_eq!(not_wet.weight(&c), 0.0);
        c.wet = false;
        let steep = Filter {
            slope_above: true,
            slope: 30.0,
            ..Filter::default()
        };
        assert_eq!(steep.weight(&c), 0.0);
        c.slope = 40.0;
        assert_eq!(steep.weight(&c), 1.0);
        let hollows = Filter {
            hollows: true,
            depth: 2.0,
            ..Filter::default()
        };
        c.relief = -3.0;
        assert_eq!(hollows.weight(&c), 1.0);
        c.relief = 3.0;
        assert_eq!(hollows.weight(&c), 0.0);
    }

    #[test]
    fn masks_read_the_map() {
        // A tile at 60 with a 10-block-high square plateau in it, half of it
        // under water.
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(60.0, 1, NO_FLUID);
        for z in 40..60 {
            for x in 40..60 {
                tile.top.set(local_index(x, z), 70.0);
            }
        }
        for z in 0..128 {
            for x in 0..20 {
                tile.fluid.set(local_index(x, z), 63);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        let rect = BlockRect::new(0, 0, 128, 128);

        let high = Filter {
            above: Some(65),
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(high.get(50, 50), 1.0);
        assert_eq!(high.get(10, 10), 0.0);
        assert_eq!(high.get(500, 10), 0.0, "outside the mask");

        let dry = Filter {
            except_on: Some(vec![Match::Water]),
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(dry.get(5, 5), 0.0);
        assert_eq!(dry.get(30, 5), 1.0);

        // The plateau's edge is steep, its middle and the plain are flat.
        let steep = Filter {
            slope_above: true,
            slope: 30.0,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(steep.get(40, 50), 1.0);
        assert_eq!(steep.get(50, 50), 0.0);
        assert_eq!(steep.get(90, 90), 0.0);

        // Next to the plateau, the plain is a hollow; on its rim, a bump.
        let hollows = Filter {
            hollows: true,
            depth: 1.0,
            radius: 4,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(hollows.get(38, 50), 1.0);
        assert_eq!(hollows.get(90, 90), 0.0);
        let bumps = Filter {
            bumps: true,
            depth: 1.0,
            radius: 4,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(bumps.get(40, 50), 1.0);
        assert_eq!(bumps.get(50, 50), 0.0);
    }
}
