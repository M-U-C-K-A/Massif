//! Snowfall: snow laid over the land by a small simulation, as Edit ▸
//! Whole map ▸ Snowfall does it. Snow falls over a band of heights, fading
//! at its edges; steep slopes do not hold it and it slides off them; the
//! wind lifts it off the slopes that face it and off the open ground and
//! drops it where the land shelters it, in the lee of ridges and in
//! hollows, so that drifts gather there; what does not settle where it
//! falls drifts with the wind.
//!
//! The depth found is laid on the map in eighths of a block — Minecraft's
//! snow layers, one to eight, then whole snow blocks under them —: the
//! ground rises by it, its terrain becomes snow, and its columns are
//! smoothed (whatever the map's Smooth terrain switch says) so that the
//! export writes the layers.

use super::map::sample_map;
use super::whole::ground_on_top;
use super::{Extent, Field, NO_WATER};
use crate::auto::{Changes, Steps};
use crate::codec::snap_height;
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::history::Edit;
use crate::registry::{Registry, TerrainId};
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The most samples a side the simulation works on: past it, a sample
/// stands for several blocks.
pub const SNOW_MAX: usize = 2048;

/// How snow falls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snowfall {
    /// How much snow falls, in blocks, before the wind and the slopes
    /// move it.
    pub amount: f32,
    /// The band of heights it falls on, and how many blocks it fades over
    /// past each end.
    pub low: f32,
    pub high: f32,
    pub fade: f32,
    /// The most it lies anywhere, in snow layers (eighths of a block): 8
    /// is a block.
    pub most: u32,
    /// How much of the snow settles where it falls (0 to 1); the rest
    /// drifts with the wind.
    pub settle: f32,
    /// How far the wind carries snow, in blocks (0: no wind).
    pub wind: f32,
    /// Where the wind blows from, in degrees: 0 north, 90 east.
    pub direction: f32,
    /// The steepest slope that holds snow, in degrees.
    pub slope: f32,
    /// Passes of the simulation: more, smoother drifts.
    pub iterations: u32,
    /// Only on the columns painted with Frost or another cover of snow.
    pub only_covers: bool,
}

impl Default for Snowfall {
    fn default() -> Self {
        Self {
            amount: 1.0,
            low: 90.0,
            high: 320.0,
            fade: 24.0,
            most: 32,
            settle: 0.6,
            wind: 16.0,
            direction: 300.0,
            slope: 45.0,
            iterations: 12,
            only_covers: false,
        }
    }
}

/// The snow the simulation finds: its depth in blocks at each sample.
pub struct Snow {
    pub extent: Extent,
    pub heights: Field,
    pub depth: Field,
}

impl Snowfall {
    /// Its settings for a map whose land lies between `lo` and `hi`: snow
    /// on its upper half.
    pub fn for_land(lo: f32, hi: f32, max_y: f32) -> Self {
        let low = (lo + (hi - lo) * 0.45).round();
        Self { low, high: max_y, fade: ((hi - lo) * 0.08).clamp(4.0, 48.0).round(), ..Self::default() }
    }

    /// The depth of snow over `heights` (blocks, `spacing` blocks apart),
    /// dry where `level` (the water's) is not over the ground. None when
    /// cancelled.
    pub fn simulate(&self, heights: &Field, level: &Field, spacing: f32, cancel: &AtomicBool) -> Option<Field> {
        let n = heights.n;
        let h = &heights.data;
        let idx = |i: usize, j: usize| j * n + i;
        let sp = spacing.max(1e-3);
        let most = self.most.max(1) as f32 / 8.0;
        let wet: Vec<bool> = (0..n * n).map(|c| level.data[c] != NO_WATER && level.data[c] > h[c] + 0.3).collect();
        // Over the band, fading past its ends.
        let fade = self.fade.max(0.5);
        let band: Vec<f32> = h
            .iter()
            .map(|&v| {
                let lo = ((v - (self.low - fade)) / fade).clamp(0.0, 1.0);
                let hi = (((self.high + fade) - v) / fade).clamp(0.0, 1.0);
                let s = lo.min(hi);
                s * s * (3.0 - 2.0 * s)
            })
            .collect();
        // The land's slope, and how much of the snow it holds.
        let at = |i: isize, j: isize| h[idx(i.clamp(0, n as isize - 1) as usize, j.clamp(0, n as isize - 1) as usize)];
        let grad = |i: usize, j: usize| {
            let (i, j) = (i as isize, j as isize);
            ((at(i + 1, j) - at(i - 1, j)) / (2.0 * sp), (at(i, j + 1) - at(i, j - 1)) / (2.0 * sp))
        };
        let tan_max = self.slope.clamp(5.0, 89.0).to_radians().tan();
        let hold: Vec<f32> = (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (gx, gz) = grad(c % n, c / n);
                let g = gx.hypot(gz);
                ((tan_max - g) / (tan_max * 0.35)).clamp(0.0, 1.0)
            })
            .collect();
        // The wind: where it goes (from `direction`), how strong.
        let from = self.direction.to_radians();
        let (wx, wz) = (-from.sin(), from.cos());
        let windy = self.wind > 0.01;
        let reach = (self.wind / sp).clamp(0.0, 96.0);
        // How sheltered each place is from the wind (lee: 1), how exposed
        // to it (the slopes facing it, open ground: 1).
        let (lee, exposed): (Vec<f32>, Vec<f32>) = if windy {
            let look = (reach.max(4.0) * 0.75).ceil() as usize;
            (0..n * n)
                .into_par_iter()
                .map(|c| {
                    let (i, j) = ((c % n) as f32, (c / n) as f32);
                    let here = h[c];
                    let mut shelter = f32::MIN;
                    for k in 1..=look {
                        let d = k as f32;
                        let up = sample(h, n, i - wx * d, j - wz * d);
                        shelter = shelter.max((up - here) / (d * sp));
                    }
                    let lee = (shelter / 0.35).clamp(0.0, 1.0);
                    let (gx, gz) = grad(c % n, c / n);
                    let facing = ((gx * wx + gz * wz) / 0.4).clamp(0.0, 1.0);
                    (lee, (0.35 + facing - lee).clamp(0.0, 1.0))
                })
                .unzip()
        } else {
            (vec![0.0; n * n], vec![0.0; n * n])
        };
        let iterations = self.iterations.clamp(1, 100);
        let fall = self.amount.max(0.0) / iterations as f32;
        let settle = self.settle.clamp(0.0, 1.0);
        let scour = if windy { (self.wind / 32.0).clamp(0.0, 1.0) * 0.5 / iterations as f32 * 4.0 } else { 0.0 };
        let hops = reach.round() as usize;
        let mut snow = vec![0f32; n * n];
        let mut air = vec![0f32; n * n];
        let talus = tan_max * sp;
        for _ in 0..iterations {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            // It falls: part settles, the slopes shed what they cannot
            // hold, the wind lifts some off the exposed ground.
            snow.par_iter_mut().zip(air.par_iter_mut()).enumerate().for_each(|(c, (s, a))| {
                if wet[c] {
                    *s = 0.0;
                    return;
                }
                let f = fall * band[c];
                let kept = f * (settle + (1.0 - settle) * lee[c]) * hold[c];
                *s += kept;
                *a += f - kept;
                let lifted = *s * scour * exposed[c];
                *s -= lifted;
                *a += lifted;
            });
            // It drifts downwind, settling where the land shelters it.
            if hops == 0 {
                for (c, a) in air.iter_mut().enumerate() {
                    if !wet[c] {
                        snow[c] += *a * hold[c];
                    }
                    *a = 0.0;
                }
            } else {
                for hop in 0..hops {
                    let moved: Vec<f32> = (0..n * n).into_par_iter().map(|c| sample_zero(&air, n, (c % n) as f32 - wx, (c / n) as f32 - wz)).collect();
                    air = moved;
                    let last = hop + 1 == hops;
                    snow.par_iter_mut().zip(air.par_iter_mut()).enumerate().for_each(|(c, (s, a))| {
                        if wet[c] {
                            // Into the water: it melts.
                            *a = 0.0;
                            return;
                        }
                        let p = if last { 1.0 } else { (0.04 + 0.9 * lee[c]) * (1.0 - 0.6 * exposed[c]) };
                        let d = *a * p;
                        *s += d * hold[c];
                        *a -= d;
                    });
                }
            }
            // Too steep a surface of snow slides down.
            for _ in 0..2 {
                slide(&mut snow, h, n, talus, &wet);
            }
            snow.par_iter_mut().for_each(|s| *s = s.clamp(0.0, most));
        }
        // A little smoothing of its surface, as the wind leaves it.
        let smooth: Vec<f32> = (0..n * n)
            .into_par_iter()
            .map(|c| {
                if snow[c] <= 0.0 {
                    return 0.0;
                }
                let (i, j) = ((c % n) as isize, (c / n) as isize);
                let mut sum = 0.0;
                let mut w = 0.0;
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (a, b) = (i + di, j + dj);
                        if a < 0 || b < 0 || a >= n as isize || b >= n as isize {
                            continue;
                        }
                        let k = b as usize * n + a as usize;
                        if wet[k] {
                            continue;
                        }
                        let weight = if di == 0 && dj == 0 { 4.0 } else { 1.0 };
                        sum += (snow[k] + h[k] - h[c]).max(0.0) * weight;
                        w += weight;
                    }
                }
                (sum / w).min(most)
            })
            .collect();
        Some(Field::new(n, smooth))
    }
}

/// `f` at fractional sample `(x, z)`, bilinear, held at the edges.
fn sample(f: &[f32], n: usize, x: f32, z: f32) -> f32 {
    let m = (n - 1) as f32;
    let (x, z) = (x.clamp(0.0, m), z.clamp(0.0, m));
    let (i, j) = (x.floor() as usize, z.floor() as usize);
    let (i1, j1) = ((i + 1).min(n - 1), (j + 1).min(n - 1));
    let (fx, fz) = (x - i as f32, z - j as f32);
    let a = f[j * n + i] + (f[j * n + i1] - f[j * n + i]) * fx;
    let b = f[j1 * n + i] + (f[j1 * n + i1] - f[j1 * n + i]) * fx;
    a + (b - a) * fz
}

/// `f` at fractional sample `(x, z)`, bilinear, nothing past the edges.
fn sample_zero(f: &[f32], n: usize, x: f32, z: f32) -> f32 {
    let (i0, j0) = (x.floor(), z.floor());
    let (fx, fz) = (x - i0, z - j0);
    let get = |i: f32, j: f32| if i < 0.0 || j < 0.0 || i >= n as f32 || j >= n as f32 { 0.0 } else { f[j as usize * n + i as usize] };
    let a = get(i0, j0) * (1.0 - fx) + get(i0 + 1.0, j0) * fx;
    let b = get(i0, j0 + 1.0) * (1.0 - fx) + get(i0 + 1.0, j0 + 1.0) * fx;
    a * (1.0 - fz) + b * fz
}

/// One pass of snow sliding off a surface (ground and snow) steeper than
/// `talus` a sample, toward its four neighbours, as much as it lies there.
fn slide(snow: &mut [f32], h: &[f32], n: usize, talus: f32, wet: &[bool]) {
    const SIDES: [(isize, isize); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let top = |c: usize| h[c] + snow[c];
    let near = |c: usize, k: usize| {
        let (i, j) = ((c % n) as isize + SIDES[k].0, (c / n) as isize + SIDES[k].1);
        (i >= 0 && j >= 0 && i < n as isize && j < n as isize).then(|| j as usize * n + i as usize)
    };
    // What each sample sends to each side.
    let out: Vec<[f32; 4]> = (0..n * n)
        .into_par_iter()
        .map(|c| {
            let mut send = [0f32; 4];
            if snow[c] <= 0.0 {
                return send;
            }
            let mut total = 0.0;
            for (k, s) in send.iter_mut().enumerate() {
                if let Some(o) = near(c, k) {
                    let excess = top(c) - top(o) - talus;
                    if excess > 0.0 {
                        *s = excess;
                        total += excess;
                    }
                }
            }
            if total <= 0.0 {
                return [0.0; 4];
            }
            let moving = (total * 0.25).min(snow[c]);
            send.map(|s| s / total * moving)
        })
        .collect();
    let next: Vec<f32> = (0..n * n)
        .into_par_iter()
        .map(|c| {
            let mut s = snow[c] - out[c].iter().sum::<f32>();
            for k in 0..4 {
                if let Some(o) = near(c, k) {
                    // The side of `o` facing `c` is the opposite one.
                    s += out[o][k ^ 1];
                }
            }
            if wet[c] { 0.0 } else { s.max(0.0) }
        })
        .collect();
    snow.copy_from_slice(&next);
}

/// The rectangle of the map's ground, in blocks.
fn ground_rect(dim: &Dimension) -> Option<BlockRect> {
    let (lo, hi) = dim.ground().bounds()?;
    Some(BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE))
}

/// The snow `fall` lays over the map (its selection's tiles, when it has
/// one), worked out on at most `n` samples a side. None when cancelled or
/// for a map without ground.
pub fn snow_over(dim: &Dimension, fall: &Snowfall, n: usize, cancel: &AtomicBool) -> Option<Snow> {
    let mut rect = ground_rect(dim)?;
    if let Some(sel) = &dim.selection
        && let Some(r) = sel.bounds()
    {
        // A margin around it: the wind brings snow from outside.
        let m = (fall.wind.max(16.0) as i32 + 15) / 16 * 16;
        rect = BlockRect::new((r.x0 - m).max(rect.x0), (r.z0 - m).max(rect.z0), (r.x1 + m).min(rect.x1), (r.z1 + m).min(rect.z1));
    }
    let side = rect.width().max(rect.depth()).max(1) as usize;
    let extent = Extent::covering(rect, side.clamp(2, n));
    let read = sample_map(dim, &extent, dim.min_y as f32, &[], &[]);
    let depth = fall.simulate(&read.heights, &read.level, extent.spacing(), cancel)?;
    Some(Snow { extent, heights: read.heights, depth })
}

/// The covers of `reg` that are snow: Frost, and those that grow snow
/// layers alone. Snowfall covers them (a cover's snow over its layers
/// would hide them), and may keep to them.
pub fn snow_covers(reg: &Registry) -> Vec<bool> {
    let mut out = vec![false];
    out.extend(reg.covers.iter().map(|c| c.frost || (!c.plants.is_empty() && c.plants.iter().all(|p| p.0.starts_with("minecraft:snow[") || p.0 == "minecraft:snow"))));
    out
}

/// Lay `snow` on the ground level of `edit` (its selection, when there is
/// one): each column rises by the snow's depth in eighths of a block, takes
/// the terrain `terrain`, is smoothed so that the export writes the layers,
/// and loses a cover of snow (`covers`, by number) it had. One step to undo.
pub fn lay_snow(edit: &mut Edit, snow: &Snow, fall: &Snowfall, terrain: TerrainId, covers: &[bool], steps: &Steps) -> Changes {
    let selection = edit.dim().selection.clone();
    let positions: Vec<TilePos> = edit.level().positions().filter(|p| selection.as_ref().is_none_or(|s| s.tile(*p).is_some())).collect();
    steps.total.store(positions.len().max(1), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let max_y = (edit.dim().max_y - 1) as f32;
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    let most = fall.most.max(1);
    let smoothed = crate::shapes::value(None, false);
    let mut cancelled = false;
    for batch in positions.chunks(256) {
        if steps.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        edit.warm(batch);
        let edited = Mutex::new(Vec::new());
        edit.par_tiles_with(batch.iter().copied(), everywhere, false, |pos, _, tile, others| {
            let on_top = ground_on_top(pos, tile, others);
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            let (ox, oz) = pos.origin();
            let mut n = 0;
            for lz in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (x, z) = (ox + lx, oz + lz);
                    let i = local_index(x, z);
                    if on_top.as_ref().is_some_and(|t| !t[i]) {
                        continue;
                    }
                    let weight = cells.map_or(crate::selection::IN, |c| c.get(i));
                    if weight == 0 || !crate::selection::lets(weight, x, z) {
                        continue;
                    }
                    let cover = tile.cover.get(i) as usize;
                    if fall.only_covers && !covers.get(cover).copied().unwrap_or(false) {
                        continue;
                    }
                    let top = tile.top.get(i);
                    let fluid = tile.fluid.get(i);
                    if fluid != crate::tile::NO_FLUID && fluid as f32 > top + 0.3 {
                        continue;
                    }
                    let (u, v) = snow.extent.grid(x as f64 + 0.5, z as f64 + 0.5);
                    let eighths = ((snow.depth.sample(u, v) * 8.0).round() as u32).min(most);
                    if eighths == 0 {
                        continue;
                    }
                    let base = crate::compose::block_top(top) as f32;
                    let new = snap_height((base + eighths as f32 / 8.0).min(max_y));
                    if new <= top + 1e-3 {
                        continue;
                    }
                    tile.top.set(i, new);
                    tile.terrain.set(i, terrain.0);
                    tile.slabs.set(i, smoothed);
                    if covers.get(cover).copied().unwrap_or(false) {
                        tile.cover.set(i, 0);
                        tile.stacked.clear_kind(i, crate::layer_stack::Kind::Cover);
                    }
                    n += 1;
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, Ordering::Relaxed);
                edited.lock().expect("no panic while held").push(pos);
            }
            steps.done.fetch_add(1, Ordering::Relaxed);
        });
        touched.extend(edited.into_inner().expect("no panic while held"));
        edit.cool(2048);
    }
    Changes { columns: changed.into_inner(), tiles: touched, cancelled }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ridge running north–south across a plain at 100, its crest at
    /// 130 in the middle, and a lake in the south-west corner.
    fn ridge(n: usize) -> (Field, Field) {
        let mut h = vec![100.0f32; n * n];
        let mut level = vec![NO_WATER; n * n];
        for j in 0..n {
            for i in 0..n {
                let d = (i as f32 - n as f32 / 2.0).abs();
                h[j * n + i] = 100.0 + (30.0 - d * 1.2).max(0.0);
                if i < 8 && j > n - 8 {
                    h[j * n + i] = 95.0;
                    level[j * n + i] = 99.0;
                }
            }
        }
        (Field::new(n, h), Field::new(n, level))
    }

    #[test]
    fn snow_drifts_into_the_lee_and_stays_off_water_and_out_of_its_band() {
        let n = 96;
        let (h, level) = ridge(n);
        // Wind from the west: it blows east, over the ridge.
        let fall = Snowfall { amount: 1.0, low: 90.0, high: 400.0, fade: 4.0, wind: 24.0, direction: 270.0, slope: 60.0, most: 64, ..Snowfall::default() };
        let depth = fall.simulate(&h, &level, 1.0, &AtomicBool::new(false)).unwrap();
        let mid = n / 2;
        let row = n / 3;
        let avg = |i0: usize, i1: usize| (i0..i1).map(|i| depth.at(i, row)).sum::<f32>() / (i1 - i0) as f32;
        let windward = avg(mid - 20, mid - 4);
        let lee = avg(mid + 4, mid + 20);
        assert!(lee > windward * 1.5, "the lee {lee} gets more than the windward slope {windward}");
        assert_eq!(depth.at(3, n - 3), 0.0, "no snow on the lake");
        assert!(depth.data.iter().all(|&d| d <= 8.0 + 1e-4), "never past the most");
        // A band above the land: none at all.
        let high = Snowfall { low: 200.0, high: 300.0, ..fall.clone() };
        let none = high.simulate(&h, &level, 1.0, &AtomicBool::new(false)).unwrap();
        assert!(none.data.iter().all(|&d| d == 0.0));
        // Without wind, the plain is covered evenly.
        let calm = Snowfall { wind: 0.0, settle: 1.0, ..fall.clone() };
        let even = calm.simulate(&h, &level, 1.0, &AtomicBool::new(false)).unwrap();
        let (a, b) = (even.at(5, row), even.at(n - 5, row));
        assert!((a - 1.0).abs() < 0.15 && (b - 1.0).abs() < 0.15, "{a} {b}");
    }

    #[test]
    fn snow_is_laid_in_eighths_on_the_ground_and_undone_in_one_step() {
        use crate::generate::{GenParams, Landform, generate};
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, ..GenParams::default() });
        let snow_terrain = world.registry.terrain_by_name("Snow").unwrap();
        let ground = world.dims[0].ground();
        let tops: Vec<f32> = ground.positions().flat_map(|p| { let t = ground.view(p).unwrap(); (0..(TILE_SIZE * TILE_SIZE) as usize).map(move |i| t.top.get(i)).collect::<Vec<_>>() }).collect();
        let (lo, hi) = (tops.iter().copied().fold(f32::MAX, f32::min), tops.iter().copied().fold(f32::MIN, f32::max));
        let fall = Snowfall { amount: 1.5, ..Snowfall::for_land(lo, hi, 319.0) };
        let snow = snow_over(&world.dims[0], &fall, 512, &AtomicBool::new(false)).unwrap();
        let before = world.dims[0].snapshot();
        let mut history = crate::history::History::new(4);
        history.begin("Snowfall", 0);
        let covers = snow_covers(&world.registry);
        let changes = lay_snow(&mut history.edit(&mut world, 0), &snow, &fall, snow_terrain, &covers, &Steps::default());
        history.commit();
        assert!(changes.columns > 1000, "{} columns", changes.columns);
        let ground = world.dims[0].ground();
        let mut seen = 0;
        for pos in ground.positions() {
            let tile = ground.view(pos).unwrap();
            let was = before.ground().view(pos).unwrap();
            for i in 0..(TILE_SIZE * TILE_SIZE) as usize {
                if tile.terrain.get(i) == snow_terrain.0 && tile.top.get(i) > was.top.get(i) + 1e-3 {
                    let frac = tile.top.get(i) - crate::compose::block_top(was.top.get(i)) as f32;
                    assert!((frac * 8.0 - (frac * 8.0).round()).abs() < 1e-3 && frac > 0.0, "a depth of eighths: {frac}");
                    assert_eq!(tile.slabs.get(i), crate::shapes::value(None, false), "smoothed, for its layers");
                    seen += 1;
                }
            }
        }
        assert!(seen > 1000);
        history.undo(&mut world);
        assert!(world.dims[0].ground().positions().all(|p| world.dims[0].ground().view(p).unwrap().top == before.ground().view(p).unwrap().top));
    }
}
