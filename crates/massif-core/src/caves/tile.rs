//! The caves of one tile: runs of air, water and lava per column, worked
//! out from the cave layers painted there and the ground.
//!
//! The noise caves — chambers, tunnels, noodles — come from a density
//! worked out every 4 blocks and blended in between (as the game does), the
//! least of each layer's kinds: under 0, a cave. The mask's weight, the
//! share of the columns around painted with a layer, scales how much rock
//! each kind takes, so caves thin out over the edge of what is painted.
//! Ravines are carved exactly, block by block. A cave keeps under the
//! ground's surface by the layer's roof, save in the columns where it may
//! open (dry land, picked by a noise of the share the layer opens over),
//! and stays within its heights. Its blocks then take their zone's fluid
//! (see [`super::aquifer`]), or stay rock where zones meet.
//!
//! The result depends on the tile's own planes alone, so that it can be
//! kept in a cache by a hash of them: the tiles around never come into it.

use super::aquifer::Zones;
use super::noise::{self, cheese_quantiles, ridge_quantiles, signed_quantiles};
use super::ravines::{self, Ball};
use super::{CaveDef, CaveLayers, Feature};
use crate::compose::block_top;
use crate::coords::{TILE_AREA, TILE_SIZE, TilePos};
use crate::tile::{NO_FLUID, Tile};
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

/// What a run of a cave holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Fill {
    Air,
    Water,
    Lava,
}

/// Blocks `y0..y1` of a column, carved out, holding `fill`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub y0: i16,
    pub y1: i16,
    pub fill: Fill,
}

/// The caves of a tile.
#[derive(Debug)]
pub struct TileCaves {
    /// Where each column's runs start in `runs` (and the end).
    offsets: Vec<u32>,
    runs: Vec<Run>,
    /// Whether some run reaches the top of the ground: a cave open to the
    /// sky changes the surface.
    opens: bool,
    fluid: bool,
}

impl TileCaves {
    /// The runs of column `cell`, bottom up, apart.
    #[inline]
    pub fn column(&self, cell: usize) -> &[Run] {
        &self.runs[self.offsets[cell] as usize..self.offsets[cell + 1] as usize]
    }

    /// Whether a cave opens to the sky somewhere in the tile.
    pub fn opens(&self) -> bool {
        self.opens
    }

    /// Whether some cave holds water or lava.
    pub fn has_fluid(&self) -> bool {
        self.fluid
    }

    /// The memory it takes.
    pub fn bytes(&self) -> usize {
        self.offsets.len() * 4 + self.runs.len() * std::mem::size_of::<Run>() + 64
    }

    /// Blocks carved, and of those, blocks holding water or lava.
    pub fn count(&self) -> (usize, usize) {
        self.runs.iter().fold((0, 0), |(all, wet), r| {
            let n = (r.y1 - r.y0) as usize;
            (all + n, wet + if r.fill == Fill::Air { 0 } else { n })
        })
    }
}

/// The density is worked out every this many blocks.
const STEP: i32 = 4;
/// Points of the lattice across a tile.
const CORNERS: usize = (TILE_SIZE / STEP) as usize + 1;
/// The share of the rock each kind takes at its densest.
const CHEESE_MAX: f32 = 0.14;
const SPAGHETTI_MAX: f32 = 0.05;
const NOODLES_MAX: f32 = 0.03;
/// The share of a chamber's room its pillars take.
const PILLARS: f32 = 0.06;
/// Blocks over which a kind thins out toward the ends of its heights.
const FADE: f32 = 10.0;
/// Rock everywhere.
const ROCK: f32 = 1.0e3;
/// Blocks from water within which no cave opens to the sky.
const SHORE: usize = 3;

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of its share a kind takes at height `y`: none out of its
/// heights, all of it inside but for its ends.
#[inline]
fn thinning(y: f32, f: &Feature) -> f32 {
    let (lo, hi) = (f.low as f32, f.high as f32);
    if y < lo || y > hi {
        return 0.0;
    }
    smoothstep(0.0, 1.0, ((y - lo) / FADE).min((hi - y) / FADE))
}

/// A layer ready to be worked out over a tile.
struct Prepared<'a> {
    def: &'a CaveDef,
    /// Its weight at the points of the lattice.
    weights: Vec<f32>,
    seed: u64,
    pillar: f32,
    opening: f32,
    zones: Option<Zones>,
    balls: Vec<Ball>,
}

impl Prepared<'_> {
    /// The density at `(x, y, z)` where the mask weighs `w`: under 0, a cave.
    fn density(&self, x: f32, y: f32, z: f32, w: f32) -> f32 {
        let mask = smoothstep(0.15, 0.6, w);
        if mask <= 0.0 {
            return ROCK;
        }
        let mut d = ROCK;
        let def = self.def;
        let c = &def.cheese;
        if c.carves() {
            let share = CHEESE_MAX * c.density * mask * thinning(y, c);
            if share > 0.0 {
                let s = c.size.clamp(0.5, 2.0);
                let n = noise::fbm(self.seed ^ 0xC4EE, x / (56.0 * s), y / (34.0 * s), z / (56.0 * s), 3);
                let mut cheese = n - cheese_quantiles().quantile(share);
                // Pillars: thin columns of rock left standing.
                let p = noise::gradient(self.seed ^ 0x9111, x / (6.0 * s), y / (36.0 * s), z / (6.0 * s));
                cheese = cheese.max((p - self.pillar) * 0.5);
                d = d.min(cheese);
            }
        }
        let t = &def.spaghetti;
        if t.carves() {
            let share = SPAGHETTI_MAX * t.density * mask * thinning(y, t);
            if share > 0.0 {
                let s = t.size.clamp(0.5, 2.0);
                // Thicker and thinner along their length.
                let swell = 0.7 + 0.3 * (noise::gradient(self.seed ^ 0x5E11, x / 150.0, y / 100.0, z / 150.0) + 1.0);
                let w = ridge_quantiles().quantile(share.sqrt()) * swell;
                let a = noise::gradient(self.seed ^ 0x5A, x / (80.0 * s), y / (52.0 * s), z / (80.0 * s));
                let b = noise::gradient(self.seed ^ 0x5B, x / (80.0 * s), y / (52.0 * s), z / (80.0 * s));
                d = d.min(a.abs().max(b.abs()) - w);
            }
        }
        let o = &def.noodles;
        if o.carves() {
            // In half the rock.
            let toggle = smoothstep(-0.04, 0.04, noise::gradient(self.seed ^ 0x7066, x / 110.0, y / 80.0, z / 110.0));
            let share = (2.0 * NOODLES_MAX * o.density * mask * thinning(y, o) * toggle).min(0.5);
            if share > 0.0 {
                let s = o.size.clamp(0.5, 2.0);
                let w = ridge_quantiles().quantile(share.sqrt());
                let a = noise::gradient(self.seed ^ 0x40A, x / (30.0 * s), y / (26.0 * s), z / (30.0 * s));
                let b = noise::gradient(self.seed ^ 0x40B, x / (30.0 * s), y / (26.0 * s), z / (30.0 * s));
                d = d.min(a.abs().max(b.abs()) - w);
            }
        }
        d
    }
}

/// The caves of the tile at `pos`, whose ground is `ground`, in a
/// dimension from `min_y` to `max_y` with cave layers `layers`; `None`
/// where none is painted.
pub fn tile_caves(layers: &CaveLayers, ground: &Tile, pos: TilePos, min_y: i32, max_y: i32, dry: &[super::CaveBox]) -> Option<Arc<TileCaves>> {
    let plane = &ground.caves;
    if plane.uniform_value() == Some(0) || layers.is_empty() {
        return None;
    }
    let mut present: Vec<u16> = Vec::new();
    plane.for_each_run(|v, _| {
        if v != 0 && !present.contains(&v) && layers.get(v).is_some_and(CaveDef::carves) {
            present.push(v);
        }
    });
    if present.is_empty() {
        return None;
    }
    present.sort_unstable();
    let key = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (pos.x, pos.z, min_y, max_y).hash(&mut h);
        for &n in &present {
            n.hash(&mut h);
            hash_def(layers.get(n).expect("present"), &mut h);
        }
        plane.for_each_run(|v, len| (v, len).hash(&mut h));
        ground.top.for_each_run(|v, len| (v.to_bits(), len).hash(&mut h));
        ground.fluid.for_each_run(|v, len| (v, len).hash(&mut h));
        dry.hash(&mut h);
        h.finish()
    };
    if let Some(found) = cache().lock().ok().and_then(|mut c| c.get(key)) {
        return Some(found);
    }
    let made = Arc::new(compute(layers, &present, ground, pos, min_y, max_y, dry));
    if let Ok(mut c) = cache().lock() {
        c.put(key, made.clone());
    }
    Some(made)
}

fn hash_def(d: &CaveDef, h: &mut impl Hasher) {
    d.seed.hash(h);
    for f in [&d.cheese, &d.spaghetti, &d.noodles, &d.ravines] {
        (f.on, f.density.to_bits(), f.size.to_bits(), f.low, f.high).hash(h);
    }
    (d.roof, d.openings.to_bits()).hash(h);
    let a = &d.aquifers;
    (a.on, a.density.to_bits(), a.size.to_bits(), a.low, a.high, a.lava.to_bits(), a.lava_under, a.lava_floor).hash(h);
}

fn compute(layers: &CaveLayers, present: &[u16], ground: &Tile, pos: TilePos, min_y: i32, max_y: i32, dry: &[super::CaveBox]) -> TileCaves {
    // The structures over the tile: no fluid within a block of them.
    let (x0t, z0t) = pos.origin();
    let reach = super::STRUCTURE_MARGIN + 1;
    let structures: Vec<super::CaveBox> = dry.iter().copied().filter(|(a, b)| b[0] + reach > x0t && a[0] - reach < x0t + TILE_SIZE && b[2] + reach > z0t && a[2] - reach < z0t + TILE_SIZE).collect();
    let n = TILE_SIZE as usize;
    let (x0, z0) = pos.origin();
    let mask = ground.caves.dense();
    let top_plane = ground.top.dense();
    let fluid_plane = ground.fluid.dense();
    let tops: Vec<i32> = top_plane.iter().map(|&t| block_top(t).clamp(min_y, max_y)).collect();
    // Columns under water, and those near them.
    let wet: Vec<bool> = (0..TILE_AREA).map(|i| fluid_plane[i] != NO_FLUID && fluid_plane[i] as i32 > tops[i]).collect();
    let near_water = dilate(&wet, SHORE);
    // Per column, the lowest ground of the columns around (fluid stays a
    // block under it, never beside the open air), and the lowest floor of
    // the water within two blocks (caves stay a block under it, never
    // beside the water).
    let low_ground = lowest(&tops, |_| true, 1);
    let wet_floor = lowest(&tops, |i| wet[i], 2);
    let top_max = tops.iter().copied().max().unwrap_or(min_y);

    // Each layer: its weight over the lattice (the share of the columns
    // within 4 blocks of a point painted with it), its zones and ravines.
    let (ylo, yhi) = present.iter().filter_map(|&k| layers.get(k)).fold((i32::MAX, i32::MIN), |(lo, hi), d| {
        let (a, b) = d.span();
        (lo.min(a), hi.max(b))
    });
    let ylo = ylo.max(min_y + 1);
    let yhi = yhi.min(max_y - 1).min(top_max);
    let prepared: Vec<Prepared> = present
        .iter()
        .filter_map(|&k| {
            let def = layers.get(k)?;
            let sums = summed(&mask, k);
            let weights = (0..CORNERS * CORNERS)
                .map(|c| {
                    let (cx, cz) = ((c % CORNERS) as i32 * STEP, (c / CORNERS) as i32 * STEP);
                    let (a0, a1) = ((cx - STEP).max(0), (cx + STEP).min(TILE_SIZE));
                    let (b0, b1) = ((cz - STEP).max(0), (cz + STEP).min(TILE_SIZE));
                    let inside = window(&sums, a0, a1, b0, b1);
                    inside as f32 / ((a1 - a0) * (b1 - b0)) as f32
                })
                .collect();
            let seed = noise::hash3(def.seed as u64, 0x0CA7, 0xE5, k as i32);
            let zones = (def.aquifers.on && def.aquifers.density > 0.0).then(|| Zones::new(def.seed, &def.aquifers, (x0, x0 + TILE_SIZE), (ylo.min(yhi), yhi.max(ylo) + 1), (z0, z0 + TILE_SIZE)));
            let balls = ravines::reaching(def.seed, &def.ravines, def.openings, pos);
            Some(Prepared {
                def,
                weights,
                seed,
                pillar: signed_quantiles().quantile(1.0 - PILLARS),
                opening: signed_quantiles().quantile(1.0 - def.openings.clamp(0.0, 1.0)),
                zones,
                balls,
            })
        })
        .collect();
    if prepared.is_empty() || yhi < ylo {
        return TileCaves { offsets: vec![0; TILE_AREA + 1], runs: Vec::new(), opens: false, fluid: false };
    }

    // The density at the points of the lattice.
    let (ly0, ly1) = (ylo.div_euclid(STEP), yhi.div_euclid(STEP) + 1);
    let ny = (ly1 - ly0 + 1) as usize;
    let lattice: Vec<f32> = (0..CORNERS * CORNERS)
        .into_par_iter()
        .flat_map_iter(|c| {
            let (cx, cz) = ((c % CORNERS) as i32 * STEP, (c / CORNERS) as i32 * STEP);
            let (x, z) = ((x0 + cx) as f32, (z0 + cz) as f32);
            let prepared = &prepared;
            (0..ny).map(move |j| {
                let y = ((ly0 + j as i32) * STEP) as f32;
                prepared.iter().fold(ROCK, |d, p| {
                    let w = p.weights[c];
                    if w <= 0.0 { d } else { d.min(p.density(x, y, z, w)) }
                })
            })
        })
        .collect();
    let at = |cx: usize, cz: usize| &lattice[(cz * CORNERS + cx) * ny..(cz * CORNERS + cx + 1) * ny];

    // Block by block, a row of the tile at a time.
    let rows: Vec<(Vec<u32>, Vec<Run>, bool)> = (0..n)
        .into_par_iter()
        .map(|lz| {
            let z = z0 + lz as i32;
            let mut counts = Vec::with_capacity(n);
            let mut runs: Vec<Run> = Vec::new();
            let mut opens = false;
            let mut column = vec![0f32; ny];
            let mut weights = vec![0f32; prepared.len()];
            // The ravines whose reach crosses this row, by layer.
            let crossing: Vec<Vec<&Ball>> = prepared.iter().map(|p| p.balls.iter().filter(|b| (b.z_range().0..=b.z_range().1).contains(&z)).collect()).collect();
            let (cz, fz) = (lz / STEP as usize, (lz % STEP as usize) as f32 / STEP as f32);
            for lx in 0..n {
                let start = runs.len();
                let i = lz * n + lx;
                let x = x0 + lx as i32;
                let (cx, fx) = (lx / STEP as usize, (lx % STEP as usize) as f32 / STEP as f32);
                let bilerp = |v: &[f32]| {
                    let (a, b) = (v[cz * CORNERS + cx], v[cz * CORNERS + cx + 1]);
                    let (c, d) = (v[(cz + 1) * CORNERS + cx], v[(cz + 1) * CORNERS + cx + 1]);
                    let near = a + (b - a) * fx;
                    near + (c + (d - c) * fx - near) * fz
                };
                // The layer that weighs most here rules the roof, the
                // openings and the fluids.
                for (w, p) in weights.iter_mut().zip(&prepared) {
                    *w = bilerp(&p.weights);
                }
                let (best, &weight) = weights.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).expect("one layer at least");
                if weight < 0.05 {
                    counts.push(0);
                    continue;
                }
                let lead = &prepared[best];
                let top = tops[i];
                let dry = !near_water[i];
                let open = dry && lead.def.openings > 0.0 && noise::gradient2(lead.seed ^ 0x0E17, x as f32 / 44.0, z as f32 / 44.0) > lead.opening;
                let ceiling = if open { top } else { top - lead.def.roof as i32 }.min(wet_floor[i] - 1);
                let (lo, hi) = lead.def.span();
                // The noise caves within `from..to`.
                let (from, to) = (lo.max(ylo), (hi + 1).min(ceiling).min(yhi + 1));
                let (c00, c10, c01, c11) = (at(cx, cz), at(cx + 1, cz), at(cx, cz + 1), at(cx + 1, cz + 1));
                for j in 0..ny {
                    let near = c00[j] + (c10[j] - c00[j]) * fx;
                    column[j] = near + (c01[j] + (c11[j] - c01[j]) * fx - near) * fz;
                }
                // The ravines of the layers painted here (at half weight or
                // more), within their heights and under their roof — up to
                // the surface for those open to the sky, on dry land.
                let mut first = from;
                let mut last = to;
                let shore = wet_floor[i] - 1;
                for (k, list) in crossing.iter().enumerate() {
                    if weights[k] < 0.5 || list.is_empty() {
                        continue;
                    }
                    let f = &prepared[k].def.ravines;
                    let roof = if dry { top } else { top - prepared[k].def.roof as i32 }.min(shore);
                    first = first.min(f.low.max(min_y + 1));
                    last = last.max((f.high + 1).min(roof));
                }
                let mut current: Option<(i32, Fill)> = None;
                for y in first..last {
                    let mut carved = false;
                    if (from..to).contains(&y) {
                        let t = (y as f32 / STEP as f32) - ly0 as f32;
                        let j = (t.floor().max(0.0) as usize).min(ny - 2);
                        carved = column[j] + (column[j + 1] - column[j]) * (t - j as f32) < 0.0;
                    }
                    if !carved {
                        'ravines: for (k, list) in crossing.iter().enumerate() {
                            let f = &prepared[k].def.ravines;
                            if weights[k] < 0.5 || y < f.low || y > f.high {
                                continue;
                            }
                            for b in list {
                                let (bx0, bx1) = b.x_range();
                                if x < bx0 || x > bx1 {
                                    continue;
                                }
                                let roof = if b.open.is_some() && dry { top } else { top - prepared[k].def.roof as i32 }.min(shore);
                                if y < roof && b.reach(x, y, z, top) < 1.0 {
                                    carved = true;
                                    break 'ravines;
                                }
                            }
                        }
                    }
                    let fill = if carved {
                        match &lead.zones {
                            // Fluid a block under the ground around at
                            // least, or rock where it would be higher.
                            Some(zones) => zones.fill(x, y, z).filter(|&f| f == Fill::Air || (y < low_ground[i] - 1 && !super::in_boxes(&structures, x, y, z, super::STRUCTURE_MARGIN + 1))),
                            None => Some(Fill::Air),
                        }
                    } else {
                        None
                    };
                    match (current, fill) {
                        (Some((_, a)), Some(b)) if a == b => {}
                        (Some((y0, a)), _) => {
                            runs.push(Run { y0: y0 as i16, y1: y as i16, fill: a });
                            current = fill.map(|f| (y, f));
                        }
                        (None, Some(f)) => current = Some((y, f)),
                        (None, None) => {}
                    }
                }
                if let Some((y0, a)) = current {
                    runs.push(Run { y0: y0 as i16, y1: last.max(y0) as i16, fill: a });
                }
                tidy(&mut runs, start);
                if runs[start..].last().is_some_and(|r| r.y1 as i32 >= top) {
                    opens = true;
                }
                counts.push((runs.len() - start) as u32);
            }
            (counts, runs, opens)
        })
        .collect();
    let mut offsets = Vec::with_capacity(TILE_AREA + 1);
    let mut runs = Vec::with_capacity(rows.iter().map(|r| r.1.len()).sum());
    let mut opens = false;
    offsets.push(0u32);
    for (counts, row, o) in rows {
        opens |= o;
        let mut at = runs.len() as u32;
        for c in counts {
            at += c;
            offsets.push(at);
        }
        runs.extend(row);
    }
    let fluid = runs.iter().any(|r| r.fill != Fill::Air);
    TileCaves { offsets, runs, opens, fluid }
}

/// A column's runs from `start` without the crumbs the noise leaves: a
/// sheet of rock a block thick between two caves of air goes (a flake
/// floating in a chamber, a floor thinner than a block), and so does a
/// slit of air a block high with rock over and under it. Rock beside water
/// or lava stays: it may be what seals a lake.
fn tidy(runs: &mut Vec<Run>, start: usize) {
    let mut k = start + 1;
    while k < runs.len() {
        let (a, b) = (runs[k - 1], runs[k]);
        if a.fill == Fill::Air && b.fill == Fill::Air && b.y0 - a.y1 <= 1 {
            runs[k - 1].y1 = b.y1;
            runs.remove(k);
        } else {
            k += 1;
        }
    }
    let mut k = start;
    while k < runs.len() {
        let r = runs[k];
        let touches = (k > start && runs[k - 1].y1 == r.y0) || (k + 1 < runs.len() && runs[k + 1].y0 == r.y1);
        if r.fill == Fill::Air && r.y1 - r.y0 <= 1 && !touches {
            runs.remove(k);
        } else {
            k += 1;
        }
    }
}

/// The columns within `r` of one where `on` holds, in the tile.
fn dilate(on: &[bool], r: usize) -> Vec<bool> {
    let n = TILE_SIZE as usize;
    let mut rows = vec![false; TILE_AREA];
    for z in 0..n {
        for x in 0..n {
            if on[z * n + x] {
                for xx in x.saturating_sub(r)..(x + r + 1).min(n) {
                    rows[z * n + xx] = true;
                }
            }
        }
    }
    let mut out = vec![false; TILE_AREA];
    for z in 0..n {
        for x in 0..n {
            if rows[z * n + x] {
                for zz in z.saturating_sub(r)..(z + r + 1).min(n) {
                    out[zz * n + x] = true;
                }
            }
        }
    }
    out
}

/// Per column, the lowest of `tops` over the columns within `r` of it
/// (in the tile) where `on` holds; `i32::MAX` where none does.
fn lowest(tops: &[i32], on: impl Fn(usize) -> bool, r: usize) -> Vec<i32> {
    let n = TILE_SIZE as usize;
    let mut rows = vec![i32::MAX; TILE_AREA];
    for z in 0..n {
        for x in 0..n {
            let i = z * n + x;
            if on(i) {
                for xx in x.saturating_sub(r)..(x + r + 1).min(n) {
                    rows[z * n + xx] = rows[z * n + xx].min(tops[i]);
                }
            }
        }
    }
    let mut out = vec![i32::MAX; TILE_AREA];
    for z in 0..n {
        for x in 0..n {
            let v = rows[z * n + x];
            if v != i32::MAX {
                for zz in z.saturating_sub(r)..(z + r + 1).min(n) {
                    out[zz * n + x] = out[zz * n + x].min(v);
                }
            }
        }
    }
    out
}

/// Summed counts of the columns painted with layer `k`: `(n + 1)²`.
fn summed(mask: &[u16], k: u16) -> Vec<u32> {
    let n = TILE_SIZE as usize;
    let mut s = vec![0u32; (n + 1) * (n + 1)];
    for z in 0..n {
        let mut row = 0;
        for x in 0..n {
            row += u32::from(mask[z * n + x] == k);
            s[(z + 1) * (n + 1) + x + 1] = s[z * (n + 1) + x + 1] + row;
        }
    }
    s
}

/// The columns counted in `x0..x1`, `z0..z1`.
fn window(s: &[u32], x0: i32, x1: i32, z0: i32, z1: i32) -> u32 {
    let w = TILE_SIZE as usize + 1;
    let (x0, x1, z0, z1) = (x0 as usize, x1 as usize, z0 as usize, z1 as usize);
    s[z1 * w + x1] + s[z0 * w + x0] - s[z0 * w + x1] - s[z1 * w + x0]
}

// ---------------------------------------------------------------- cache --

/// The caves worked out lately, by the hash of what they came from, within
/// a budget of memory: the least lately asked for go first.
struct Cache {
    map: HashMap<u64, (Arc<TileCaves>, u64)>,
    bytes: usize,
    clock: u64,
}

const BUDGET: usize = 384 << 20;

impl Cache {
    fn get(&mut self, key: u64) -> Option<Arc<TileCaves>> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(&key).map(|(c, stamp)| {
            *stamp = clock;
            c.clone()
        })
    }

    fn put(&mut self, key: u64, caves: Arc<TileCaves>) {
        self.clock += 1;
        self.bytes += caves.bytes();
        if let Some((old, _)) = self.map.insert(key, (caves, self.clock)) {
            self.bytes -= old.bytes();
        }
        while self.bytes > BUDGET && self.map.len() > 1 {
            let Some((&oldest, _)) = self.map.iter().min_by_key(|(_, (_, stamp))| *stamp) else { break };
            if let Some((gone, _)) = self.map.remove(&oldest) {
                self.bytes -= gone.bytes();
            }
        }
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache { map: HashMap::new(), bytes: 0, clock: 0 }))
}

/// The memory the caves kept in the cache take.
pub fn cache_bytes() -> usize {
    cache().lock().map_or(0, |c| c.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{Span, TileStack};
    use crate::world::{World, WorldMeta};

    /// A map of `n` × `n` tiles, its ground at `top`, painted with one
    /// layer of caves as the game's where `painted` says.
    fn world(n: i32, top: f32, painted: impl Fn(i32, i32) -> bool) -> World {
        let mut w = World::new(WorldMeta::default());
        let dim = &mut w.dims[0];
        dim.caves = CaveLayers::new(vec![CaveDef::vanilla("Caves", 7, dim.min_y, dim.max_y, 63)]);
        for z in 0..n {
            for x in 0..n {
                let mut tile = Tile::ground(top, 1, NO_FLUID);
                for i in 0..TILE_AREA {
                    let (bx, bz) = (x * TILE_SIZE + (i % 128) as i32, z * TILE_SIZE + (i / 128) as i32);
                    if painted(bx, bz) {
                        tile.caves.set(i, 1);
                    }
                }
                tile.compact();
                dim.ground_mut().insert(TilePos::new(x, z), tile);
            }
        }
        w
    }

    /// What the stack composes at `(x, z)`: solid spans and fluids.
    fn column(w: &World, x: i32, z: i32) -> (Vec<Span>, Vec<(i32, i32, crate::voxel::Fluid)>) {
        let dim = &w.dims[0];
        let stack = TileStack::new(dim, TilePos::containing(x, z), false).unwrap();
        let cell = crate::coords::local_index(x, z);
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        stack.spans(cell, &mut spans);
        stack.cave_fluids(cell, &mut fluids);
        (spans, fluids)
    }

    fn air(spans: &[Span], top: i32) -> usize {
        let solid: i32 = spans.iter().map(|s| s.y1.min(top) - s.y0.min(top)).sum();
        (top - spans[0].y0 - solid) as usize
    }

    /// Caves come where a layer is painted, the same every time, take a
    /// share of the rock near the layer's, and keep a roof under the
    /// ground but where they open; the result of a tile is kept.
    #[test]
    fn caves_are_carved_where_painted() {
        let w = world(2, 120.0, |x, _| x < 192);
        let dim = &w.dims[0];
        let ground = dim.ground().view(TilePos::new(0, 0)).unwrap();
        let a = tile_caves(&dim.caves, &ground, TilePos::new(0, 0), dim.min_y, dim.max_y, &[]).unwrap();
        let b = tile_caves(&dim.caves, &ground, TilePos::new(0, 0), dim.min_y, dim.max_y, &[]).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "kept in the cache");
        let (carved, wet) = a.count();
        // Of the rock from −56 to 120.
        let share = carved as f32 / (TILE_AREA as f32 * 176.0);
        assert!((0.04..0.2).contains(&share), "{share} of the rock carved");
        assert!(wet > 1000, "aquifers hold water and lava: {wet}");
        let opened = |w: &World| {
            let mut opened = 0;
            for z in (0..256).step_by(3) {
                for x in (0..256).step_by(3) {
                    let (spans, _) = column(w, x, z);
                    if x >= 196 {
                        assert_eq!(spans.len(), 1, "nothing carved at {x} {z}: {spans:?}");
                        continue;
                    }
                    assert!(spans[0].y0 == dim.min_y && spans[0].y1 > dim.min_y + 1, "the floor stays: {spans:?}");
                    let last = spans.last().unwrap();
                    if last.y1 < 120 {
                        opened += 1;
                    } else if spans.len() > 1 {
                        assert!(last.y1 - last.y0 >= 6, "a roof of 6 blocks at least: {spans:?}");
                    }
                }
            }
            opened
        };
        // With openings, some reach the sky (and a cave may come within a
        // block of the surface where they may); without, a roof everywhere.
        let mut defs = dim.caves.to_vec();
        defs[0].openings = 1.0;
        let mut open = w.clone();
        open.dims[0].caves = CaveLayers::new(defs.clone());
        let mut closed = w.clone();
        defs[0].openings = 0.0;
        closed.dims[0].caves = CaveLayers::new(defs);
        assert_eq!(opened(&closed), 0);
        let some = { let w = &open; let mut n = 0; for z in (0..192).step_by(2) { for x in (0..192).step_by(2) { n += usize::from(column(w, x, z).0.last().unwrap().y1 < 120); } } n };
        assert!(some > 20, "{some} columns open to the sky");
        // Another seed, other caves.
        let mut other = w.clone();
        let mut defs = other.dims[0].caves.to_vec();
        defs[0].seed = 8;
        other.dims[0].caves = CaveLayers::new(defs);
        let differ = (0..64).filter(|i| column(&w, i * 3, 50).0 != column(&other, i * 3, 50).0).count();
        assert!(differ > 10, "{differ}");
    }

    /// Tiles meet: the caves of two tiles side by side go on across their
    /// border as if they were one.
    #[test]
    fn caves_go_on_across_tiles() {
        let w = world(2, 100.0, |_, _| true);
        let mut same = 0;
        let mut both = 0;
        for z in 0..128 {
            let (a, _) = column(&w, 127, z);
            let (b, _) = column(&w, 128, z);
            let (a, b) = (air(&a, 90), air(&b, 90));
            both += usize::from(a > 0 && b > 0);
            same += usize::from((a as i32 - b as i32).abs() <= 6);
        }
        assert!(same > 115 && both > 20, "{same} alike, {both} both carved");
    }

    /// No water or lava runs: beside and under each block of it, rock or
    /// the same fluid, save the air over a lake.
    #[test]
    fn lakes_do_not_spill() {
        let w = world(1, 110.0, |_, _| true);
        let dim = &w.dims[0];
        let stack = TileStack::new(dim, TilePos::new(0, 0), false).unwrap();
        let h = (dim.max_y - dim.min_y) as usize;
        // 0 rock, 1 air, 2 water, 3 lava.
        let mut grid = vec![0u8; TILE_AREA * h];
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        for cell in 0..TILE_AREA {
            stack.spans(cell, &mut spans);
            stack.cave_fluids(cell, &mut fluids);
            for y in dim.min_y..dim.max_y {
                let solid = spans.iter().any(|s| s.y0 <= y && y < s.y1);
                grid[cell * h + (y - dim.min_y) as usize] = if solid { 0 } else { 1 };
            }
            for &(y0, y1, f) in &fluids {
                for y in y0..y1 {
                    grid[cell * h + (y - dim.min_y) as usize] = 2 + f as u8;
                }
            }
        }
        let at = |x: i32, y: i32, z: i32| grid[crate::coords::local_index(x, z) * h + (y - dim.min_y) as usize];
        let mut wet = 0;
        for z in 1..127 {
            for x in 1..127 {
                for y in dim.min_y + 1..dim.max_y - 1 {
                    let f = at(x, y, z);
                    if f < 2 {
                        continue;
                    }
                    wet += 1;
                    for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
                        let n = at(x + dx, y + dy, z + dz);
                        assert!(n == 0 || n == f, "fluid {f} at {x} {y} {z} beside {n} at {dx} {dy} {dz}");
                    }
                }
            }
        }
        assert!(wet > 1000, "{wet}");
    }
}
