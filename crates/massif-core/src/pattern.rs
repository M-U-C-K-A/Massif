//! Patterns to paint the ground with, Massif's noise painter: several
//! terrains laid out by a noise (patches, veins, blobs, cells or stripes of
//! simplex, Perlin or value noise, with as many octaves as wanted), each
//! over the share of the ground its weight says, in the order they are
//! listed — neighbours in the list meet on the ground —, with soft edges.
//!
//! Noise values are not spread evenly (most of a fractal noise lies near
//! its middle), so the pattern first learns its own spread from a few
//! thousand samples and paints by rank: a terrain weighted a quarter covers
//! a quarter of the ground, whatever the pattern.
//!
//! A pattern is laid in world coordinates: painting the same place twice
//! paints the same thing, and strokes side by side join without a seam.

use crate::auto::{Changes, Steps};
use crate::compose::hash2;
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::filter::{Filter, Mask};
use crate::history::Edit;
use crate::noise::{Basis, Noise};
use crate::registry::TerrainId;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The shape of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PatternKind {
    /// Rounded patches of every size: grass with dirt, moss in the stone.
    Patches,
    /// Thin branching lines: gravel paths, cracks, veins.
    Veins,
    /// Round blobs with creases between them.
    Blobs,
    /// Cells, each one terrain: a patchwork of fields, crazy paving.
    Cells,
    /// Bands across the ground, wavering: strata, ploughed fields, dunes.
    Stripes,
}

impl PatternKind {
    pub const ALL: [PatternKind; 5] = [PatternKind::Patches, PatternKind::Veins, PatternKind::Blobs, PatternKind::Cells, PatternKind::Stripes];

    pub const fn label(self) -> &'static str {
        match self {
            PatternKind::Patches => "Patches",
            PatternKind::Veins => "Veins",
            PatternKind::Blobs => "Blobs",
            PatternKind::Cells => "Cells",
            PatternKind::Stripes => "Stripes",
        }
    }

    /// Whether its shapes come from a fractal noise (all but cells).
    pub const fn is_fractal(self) -> bool {
        !matches!(self, PatternKind::Cells)
    }
}

/// The most layers of detail a pattern has.
pub const MAX_OCTAVES: u32 = 8;
/// The most a pattern's shapes are drawn out.
pub const MAX_STRETCH: f32 = 16.0;

/// A pattern of terrains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    pub kind: PatternKind,
    /// The noise its shapes are made of (cells have their own).
    pub noise: Basis,
    /// Size of its features, in blocks.
    pub scale: f32,
    /// Layers of ever finer detail over the largest shapes, 1 to
    /// [`MAX_OCTAVES`] (the "octaves" of noise tools).
    pub octaves: u32,
    /// How much finer each layer of detail is than the one before.
    pub lacunarity: f32,
    /// How much each layer of detail counts against the one before (the
    /// "gain", or persistence): low, smooth edges; high, ragged ones.
    pub gain: f32,
    /// How much longer its shapes are than wide, along `angle` (1: round).
    pub stretch: f32,
    /// The direction its shapes run along — drawn out shapes, stripes —
    /// in degrees: 0 east–west, 90 north–south.
    pub angle: f32,
    /// How soft the edges between terrains are (0: sharp, 1: very ragged).
    pub softness: f32,
    /// How far neighbours in the list blend into each other, dithered
    /// block by block: 0, sharp bands; 1, a gradient from one to the next
    /// across all of their shares (an ordered gradient: dirt, then coarse
    /// dirt, then grass).
    pub blend: f32,
    pub seed: u32,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            kind: PatternKind::Patches,
            noise: Basis::Simplex,
            scale: 24.0,
            octaves: 4,
            lacunarity: 2.0,
            gain: 0.5,
            stretch: 1.0,
            angle: 30.0,
            softness: 0.3,
            blend: 0.0,
            seed: 1,
        }
    }
}

/// A pattern's settings as its noise reads them, worked out once.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// From the world to the pattern's own frame, in shapes: `u` along
    /// its direction (drawn out by the stretch), `v` across it.
    along: (f64, f64),
    across: (f64, f64),
    octaves: u32,
    lacunarity: f64,
    gain: f64,
}

impl Pattern {
    fn frame(&self) -> Frame {
        let s = self.scale.max(1.0) as f64;
        let (sa, ca) = (self.angle as f64).to_radians().sin_cos();
        let stretch = self.stretch.clamp(1.0, MAX_STRETCH) as f64;
        Frame {
            along: (ca / (s * stretch), sa / (s * stretch)),
            across: (-sa / s, ca / s),
            octaves: self.octaves.clamp(1, MAX_OCTAVES),
            lacunarity: self.lacunarity.clamp(1.0, 4.0) as f64,
            gain: self.gain.clamp(0.0, 1.0) as f64,
        }
    }

    /// The pattern's raw value at a point of the world.
    fn raw(&self, frame: &Frame, noise: &Noise, x: f64, z: f64) -> f64 {
        let u = x * frame.along.0 + z * frame.along.1;
        let v = x * frame.across.0 + z * frame.across.1;
        let (basis, octaves, lacunarity, gain) = (self.noise, frame.octaves, frame.lacunarity, frame.gain);
        match self.kind {
            PatternKind::Patches => noise.fbm_of(basis, u, v, octaves, lacunarity, gain),
            PatternKind::Veins => -noise.ridged_of(basis, u, v, octaves, lacunarity, gain),
            PatternKind::Blobs => noise.fbm_of(basis, u, v, octaves, lacunarity, gain).abs(),
            PatternKind::Cells => {
                let (cx, cz) = (u.floor() as i32, v.floor() as i32);
                let mut best = (f64::MAX, 0u32);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let (gx, gz) = (cx + dx, cz + dz);
                        let h = hash2(gx, gz, self.seed as u64 ^ 0xce11_5eed);
                        let ox = (h & 0xffff) as f64 / 65535.0;
                        let oz = (h >> 16) as f64 / 65535.0;
                        let d = (gx as f64 + ox - u).hypot(gz as f64 + oz - v);
                        if d < best.0 {
                            best = (d, hash2(gx, gz, self.seed as u64 ^ 0x0c0f_fee5));
                        }
                    }
                }
                best.1 as f64
            }
            PatternKind::Stripes => {
                // Bands along the direction, wavering: the value changes
                // across them.
                let waver = noise.fbm_of(basis, u / 4.0, v / 4.0, octaves, lacunarity, gain) * 1.2;
                let t = (v + waver) / 2.0;
                // A triangle wave: every value as often as any other.
                (t - t.floor() - 0.5).abs()
            }
        }
    }

    /// Ready to paint `terrains` (each with its weight), in that order.
    pub fn sampler(&self, terrains: &[(TerrainId, f32)]) -> PatternSampler {
        let noise = Noise::new(self.seed as u64 ^ 0x9a77_e2a5);
        let frame = self.frame();
        // The pattern's own spread, from samples far apart: forty shapes
        // and more each way.
        let span = self.scale.max(1.0) as f64 * 40.0 * self.stretch.clamp(1.0, MAX_STRETCH) as f64;
        let mut quantiles: Vec<f64> = (0..4096)
            .map(|k| {
                let (u, v) = (hash2(k, 1, 0x5a3e) as f64 / u32::MAX as f64, hash2(k, 2, 0x5a3e) as f64 / u32::MAX as f64);
                self.raw(&frame, &noise, u * span, v * span)
            })
            .collect();
        quantiles.sort_by(f64::total_cmp);
        let total: f32 = terrains.iter().map(|t| t.1.max(0.0)).sum::<f32>().max(1e-6);
        let mut acc = 0.0;
        let cumulative = terrains
            .iter()
            .map(|t| {
                acc += t.1.max(0.0) / total;
                (t.0, acc)
            })
            .collect();
        PatternSampler {
            pattern: self.clone(),
            frame,
            noise,
            quantiles,
            cumulative,
        }
    }
}

/// A pattern with its spread learnt and its terrains placed.
pub struct PatternSampler {
    pattern: Pattern,
    frame: Frame,
    noise: Noise,
    quantiles: Vec<f64>,
    /// Each terrain, with the share of the ground up to and including it.
    cumulative: Vec<(TerrainId, f32)>,
}

/// How the values of a pattern spread, for a picture of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spread {
    /// How often each value comes, in steps from the lowest value to the
    /// highest; the most frequent is 1.
    pub density: Vec<f32>,
    /// Where the values of each terrain end, as a fraction of the way from
    /// the lowest value to the highest, in the order of the terrains.
    pub limits: Vec<f32>,
}

impl PatternSampler {
    /// Where column `(x, z)` stands in the pattern's spread, 0 to 1.
    pub fn rank(&self, x: i32, z: i32) -> f32 {
        let v = self.pattern.raw(&self.frame, &self.noise, x as f64 + 0.5, z as f64 + 0.5);
        let k = self.quantiles.partition_point(|q| *q < v);
        k as f32 / self.quantiles.len() as f32
    }

    /// The terrain of column `(x, z)`.
    pub fn terrain(&self, x: i32, z: i32) -> Option<TerrainId> {
        self.terrain_at(x, z, self.rank(x, z))
    }

    /// The terrain of column `(x, z)` standing at `rank` in the spread.
    fn terrain_at(&self, x: i32, z: i32, rank: f32) -> Option<TerrainId> {
        let soft = self.pattern.softness.clamp(0.0, 1.0) * 0.12;
        let jitter = (hash2(x, z, self.pattern.seed as u64 ^ 0x5017) as f32 / u32::MAX as f32 - 0.5) * soft;
        let r = (rank + jitter).clamp(0.0, 0.9999);
        let c = &self.cumulative;
        if c.is_empty() {
            return None;
        }
        let mut k = c.iter().position(|(_, e)| r < *e).unwrap_or(c.len() - 1);
        let blend = self.pattern.blend.clamp(0.0, 1.0);
        if blend > 0.0 && c.len() > 1 {
            // Across each limit between neighbours, a band as wide as the
            // blend says (at most half the smaller share each side), where
            // the chance of the next one rises evenly from 0 to 1.
            let share = |k: usize| c[k].1 - if k == 0 { 0.0 } else { c[k - 1].1 };
            let dice = hash2(x, z, self.pattern.seed as u64 ^ 0x00b1_e4d0) as f32 / u32::MAX as f32;
            let band = |k: usize| blend * 0.5 * share(k).min(share(k + 1));
            if k + 1 < c.len() && band(k) > 0.0 && r > c[k].1 - band(k) {
                let h = band(k);
                if dice < (r - (c[k].1 - h)) / (2.0 * h) {
                    k += 1;
                }
            } else if k > 0 && band(k - 1) > 0.0 && r < c[k - 1].1 + band(k - 1) {
                let h = band(k - 1);
                if dice >= (r - (c[k - 1].1 - h)) / (2.0 * h) {
                    k -= 1;
                }
            }
        }
        Some(c[k].0)
    }

    /// The terrains in their order, each with the share of the ground up to
    /// and including it.
    pub fn cumulative(&self) -> &[(TerrainId, f32)] {
        &self.cumulative
    }

    /// The spread of the pattern's values in `bins` steps, and where each
    /// terrain's values end.
    pub fn spread(&self, bins: usize) -> Spread {
        let bins = bins.max(2);
        let q = &self.quantiles;
        let (lo, hi) = (q[0], q[q.len() - 1]);
        let width = (hi - lo).max(1e-12);
        let at = |v: f64| ((v - lo) / width).clamp(0.0, 1.0);
        let mut counts = vec![0.0f32; bins];
        for &v in q {
            counts[((at(v) * bins as f64) as usize).min(bins - 1)] += 1.0;
        }
        // A little smoothing: a few thousand samples are grainy.
        let density: Vec<f32> = (0..bins)
            .map(|k| {
                let near = |d: isize| counts[(k as isize + d).clamp(0, bins as isize - 1) as usize];
                (near(-2) + 2.0 * near(-1) + 3.0 * near(0) + 2.0 * near(1) + near(2)) / 9.0
            })
            .collect();
        let peak = density.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        let limits = self
            .cumulative
            .iter()
            .map(|(_, c)| {
                let k = ((*c as f64 * q.len() as f64) as usize).min(q.len() - 1);
                if *c >= 0.9999 { 1.0 } else { at(q[k]) as f32 }
            })
            .collect();
        Spread {
            density: density.into_iter().map(|d| d / peak).collect(),
            limits,
        }
    }
}

/// Tiles read together by a filter, as squares this many tiles on a side.
const SQUARE: i32 = 4;
/// Squares painted together; between two batches the map is cooled down
/// again, so that a map of any size is never decoded whole.
const BATCH: usize = 64;
/// Tiles of the edited level left decoded after each batch.
const KEEP_HOT: usize = 2048;

/// Whether a column a filter lets through at weight `w` is painted: always
/// at full weight, never at none, and in between as often as the weight
/// says, so that soft limits fade out.
fn lets(w: f32, x: i32, z: i32) -> bool {
    w >= 1.0 || (w > 0.0 && (hash2(x, z, 0xd1_7e43) as f32) < w * u32::MAX as f32)
}

/// Paint the pattern over every column of the edited level — where
/// `filter` lets it, when there is one —, telling how far it is in
/// `steps` and stopping when asked. Columns that already have the terrain
/// the pattern gives them are not counted as changed.
pub fn paint_over_the_map(edit: &mut Edit, sampler: &PatternSampler, filter: Option<&Filter>, steps: &Steps) -> Changes {
    let filter = filter.filter(|f| f.is_active());
    let selection = edit.dim().selection.clone();
    let mut squares: HashMap<(i32, i32), Vec<TilePos>> = HashMap::new();
    for pos in edit.level().positions().filter(|p| selection.as_ref().is_none_or(|s| s.tile(*p).is_some())) {
        squares.entry((pos.x.div_euclid(SQUARE), pos.z.div_euclid(SQUARE))).or_default().push(pos);
    }
    let mut squares: Vec<((i32, i32), Vec<TilePos>)> = squares.into_iter().collect();
    squares.sort_unstable_by_key(|((x, z), _)| (*z, *x));
    steps.total.store(squares.iter().map(|s| s.1.len()).sum(), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    let mut cancelled = false;
    for batch in squares.chunks(BATCH) {
        if steps.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let positions: Vec<TilePos> = batch.iter().flat_map(|s| s.1.iter().copied()).collect();
        let count = positions.len();
        edit.warm(&positions);
        // Where the filter lets the paint go, a square at a time.
        let masks: HashMap<(i32, i32), Mask> = match filter {
            Some(f) => {
                let dim = edit.dim();
                batch
                    .par_iter()
                    .map(|((sx, sz), _)| {
                        let side = SQUARE * TILE_SIZE;
                        let rect = BlockRect::new(sx * side, sz * side, (sx + 1) * side, (sz + 1) * side);
                        ((*sx, *sz), f.mask(dim, rect))
                    })
                    .collect()
            }
            None => HashMap::new(),
        };
        let edited = Mutex::new(Vec::new());
        edit.par_tiles(positions, everywhere, false, |pos, _, tile| {
            let mask = masks.get(&(pos.x.div_euclid(SQUARE), pos.z.div_euclid(SQUARE)));
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            let slab = tile.bottom.is_some();
            let rect = pos.rect();
            let mut n = 0;
            for z in rect.z0..rect.z1 {
                for x in rect.x0..rect.x1 {
                    let i = local_index(x, z);
                    if (slab && !tile.present(i)) || mask.is_some_and(|m| !lets(m.get(x, z), x, z)) || cells.is_some_and(|c| !crate::selection::lets(c.get(i), x, z)) {
                        continue;
                    }
                    if let Some(t) = sampler.terrain(x, z)
                        && tile.terrain.get(i) != t.0
                    {
                        tile.terrain.set(i, t.0);
                        n += 1;
                    }
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, Ordering::Relaxed);
                edited.lock().expect("no panic while held").push(pos);
            }
        });
        touched.extend(edited.into_inner().expect("no panic while held"));
        steps.done.fetch_add(count, Ordering::Relaxed);
        edit.cool(KEEP_HOT);
    }
    Changes {
        columns: changed.into_inner(),
        tiles: touched,
        cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_dithers_neighbours_into_each_other_in_order() {
        // Dirt 30 %, coarse dirt 30 %, grass 40 %: an ordered gradient.
        let (dirt, coarse, grass) = (TerrainId(1), TerrainId(2), TerrainId(3));
        let p = Pattern { softness: 0.0, blend: 1.0, ..Pattern::default() };
        let s = p.sampler(&[(dirt, 0.3), (coarse, 0.3), (grass, 0.4)]);
        let picks = |rank: f32| {
            let mut n = [0usize; 4];
            for k in 0..4000 {
                n[s.terrain_at(k * 7, k * 13, rank).unwrap().0 as usize] += 1;
            }
            n
        };
        // On a limit, half and half of the two neighbours, never the third.
        let at = picks(0.3);
        assert!(at[3] == 0 && (at[1] as f32 / 4000.0 - 0.5).abs() < 0.05, "{at:?}");
        let at = picks(0.6);
        assert!(at[1] == 0 && (at[2] as f32 / 4000.0 - 0.5).abs() < 0.05, "{at:?}");
        // In the middle of a share, that one alone; part way, mostly it.
        assert_eq!(picks(0.45), [0, 0, 4000, 0]);
        let near = picks(0.25);
        assert!(near[1] > near[2] && near[2] > 0 && near[3] == 0, "{near:?}");
        // Sharp without it.
        let sharp = Pattern { softness: 0.0, ..Pattern::default() }.sampler(&[(dirt, 0.3), (coarse, 0.3), (grass, 0.4)]);
        assert!((0..4000).all(|k| sharp.terrain_at(k, k, 0.29) == Some(dirt)));
        // Over the ground, each keeps its share, and strokes join: the same
        // column gives the same block from any sampler.
        let mut counts = [0usize; 4];
        for z in 0..256 {
            for x in 0..256 {
                counts[s.terrain(x, z).unwrap().0 as usize] += 1;
            }
        }
        let share = |k: usize| counts[k] as f32 / 65536.0;
        assert!((share(1) - 0.3).abs() < 0.08 && (share(2) - 0.3).abs() < 0.08 && (share(3) - 0.4).abs() < 0.08, "{counts:?}");
        let again = p.sampler(&[(dirt, 0.3), (coarse, 0.3), (grass, 0.4)]);
        assert!((0..500).all(|k| s.terrain(k * 31 - 4000, k * 17 + 900) == again.terrain(k * 31 - 4000, k * 17 + 900)));
    }

    #[test]
    fn every_pattern_gives_each_terrain_its_share() {
        let (a, b, c) = (TerrainId(1), TerrainId(2), TerrainId(3));
        for kind in PatternKind::ALL {
            for noise in Basis::ALL {
                let p = Pattern {
                    kind,
                    noise,
                    softness: 0.0,
                    ..Pattern::default()
                };
                let s = p.sampler(&[(a, 2.0), (b, 1.0), (c, 1.0)]);
                let mut counts = [0usize; 4];
                for z in 0..256 {
                    for x in 0..256 {
                        counts[s.terrain(x, z).unwrap().0 as usize] += 1;
                    }
                }
                let share = |k: usize| counts[k] as f32 / 65536.0;
                assert!((share(1) - 0.5).abs() < 0.12 && (share(2) - 0.25).abs() < 0.1 && (share(3) - 0.25).abs() < 0.1, "{kind:?} {noise:?}: {counts:?}");
            }
        }
    }

    #[test]
    fn patterns_have_features_of_their_size() {
        // Patches of 24 blocks: neighbours mostly alike, far columns not.
        let s = Pattern::default().sampler(&[(TerrainId(1), 1.0), (TerrainId(2), 1.0)]);
        let same = |d: i32| (0..2000).filter(|&k| s.terrain(k * 7, k * 3) == s.terrain(k * 7 + d, k * 3)).count() as f32 / 2000.0;
        assert!(same(1) > 0.85, "{}", same(1));
        assert!(same(200) < 0.65, "{}", same(200));
        // Cells: one terrain across a cell.
        let cells = Pattern {
            kind: PatternKind::Cells,
            scale: 40.0,
            softness: 0.0,
            ..Pattern::default()
        }
        .sampler(&[(TerrainId(1), 1.0), (TerrainId(2), 1.0), (TerrainId(3), 1.0)]);
        let alike = (0..2000).filter(|&k| cells.terrain(k * 13, k * 5) == cells.terrain(k * 13 + 1, k * 5)).count();
        assert!(alike > 1850, "{alike}");
    }

    /// How often the terrain changes from a column to the next one `(dx,
    /// dz)` away.
    fn edges(s: &PatternSampler, dx: i32, dz: i32) -> f32 {
        let mut n = 0;
        for z in 0..200 {
            for x in 0..200 {
                n += (s.terrain(x * 3, z * 3) != s.terrain(x * 3 + dx, z * 3 + dz)) as usize;
            }
        }
        n as f32 / 40_000.0
    }

    #[test]
    fn stretched_shapes_run_along_their_direction() {
        let two = [(TerrainId(1), 1.0), (TerrainId(2), 1.0)];
        for angle in [0.0f32, 90.0] {
            let s = Pattern {
                stretch: 6.0,
                angle,
                softness: 0.0,
                ..Pattern::default()
            }
            .sampler(&two);
            // Along the shapes the terrain changes far less often than across.
            let (east, south) = (edges(&s, 1, 0), edges(&s, 0, 1));
            let (along, across) = if angle == 0.0 { (east, south) } else { (south, east) };
            assert!(along * 3.0 < across, "{angle}°: along {along}, across {across}");
        }
        // Round shapes change as often both ways.
        let round = Pattern { softness: 0.0, ..Pattern::default() }.sampler(&two);
        let (east, south) = (edges(&round, 1, 0), edges(&round, 0, 1));
        assert!((east / south - 1.0).abs() < 0.3, "{east} {south}");
    }

    #[test]
    fn octaves_and_gain_make_the_edges_ragged() {
        let two = [(TerrainId(1), 1.0), (TerrainId(2), 1.0)];
        let with = |octaves: u32, gain: f32| {
            let s = Pattern {
                octaves,
                gain,
                softness: 0.0,
                ..Pattern::default()
            }
            .sampler(&two);
            edges(&s, 1, 0)
        };
        let (smooth, ragged) = (with(1, 0.5), with(6, 0.75));
        assert!(ragged > smooth * 1.5, "one octave {smooth}, six {ragged}");
    }

    #[test]
    fn the_spread_shows_where_each_terrain_lies() {
        let s = Pattern::default().sampler(&[(TerrainId(1), 1.0), (TerrainId(2), 2.0), (TerrainId(3), 1.0)]);
        let spread = s.spread(48);
        assert_eq!(spread.density.len(), 48);
        assert!(spread.density.iter().all(|d| (0.0..=1.0).contains(d)));
        assert_eq!(spread.density.iter().copied().fold(0.0, f32::max), 1.0);
        // A fractal noise lies mostly near its middle.
        assert!(spread.density[24] > spread.density[2] * 2.0, "{:?}", spread.density);
        assert_eq!(spread.limits.len(), 3);
        assert!(spread.limits.windows(2).all(|w| w[0] < w[1]), "{:?}", spread.limits);
        assert_eq!(spread.limits[2], 1.0);
        // The middle terrain, half of the ground, takes the middle values:
        // a narrower band than half the range.
        assert!(spread.limits[1] - spread.limits[0] < 0.5, "{:?}", spread.limits);
    }

    #[test]
    fn a_pattern_paints_the_whole_map_where_the_filter_lets_it() {
        use crate::history::History;
        use crate::tile::{NO_FLUID, Tile};
        use crate::world::{Dimension, DimensionKind, World, WorldMeta};
        let mut world = World::new(WorldMeta::default());
        let reg = world.registry.clone();
        let t = |name: &str| reg.terrain_by_name(name).unwrap();
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        // Two tiles of grass, the eastern half of each high ground.
        for tx in 0..2 {
            let mut tile = Tile::ground(64.0, t("Grass").0, NO_FLUID);
            for z in 0..128 {
                for x in 64..128 {
                    tile.top.set(local_index(x, z), 90.0);
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        let sampler = Pattern::default().sampler(&[(t("Podzol"), 1.0), (t("Gravel"), 1.0)]);
        let filter = Filter {
            above: Some(80),
            ..Filter::default()
        };
        let mut history = History::new(4);
        history.begin("pattern", 0);
        let steps = Steps::default();
        let changes = paint_over_the_map(&mut history.edit(&mut world, 0), &sampler, Some(&filter), &steps);
        assert!(!changes.cancelled);
        assert_eq!(steps.fraction(), 1.0);
        assert_eq!(changes.tiles.len(), 2);
        assert_eq!(changes.columns, 2 * 64 * 128, "only the high ground, and all of it");
        let ground = world.dims[0].ground();
        let mut seen = HashMap::new();
        for tx in 0..2 {
            let tile = ground.view(TilePos::new(tx, 0)).unwrap();
            for z in 0..128 {
                for x in 0..128 {
                    let terrain = tile.terrain.get(local_index(x, z));
                    if x < 64 {
                        assert_eq!(terrain, t("Grass").0, "the low ground is left alone");
                    } else {
                        *seen.entry(terrain).or_insert(0usize) += 1;
                    }
                }
            }
        }
        assert!(seen.len() == 2 && seen.values().all(|&n| n > 3000), "{seen:?}");
        // As a brush would: the same terrain where the pattern says.
        let tile = ground.view(TilePos::new(1, 0)).unwrap();
        assert_eq!(tile.terrain.get(local_index(200 - 128, 30)), sampler.terrain(200, 30).unwrap().0);
        // Again: nothing more to change. Cancelled: nothing done.
        history.begin("again", 0);
        let again = paint_over_the_map(&mut history.edit(&mut world, 0), &sampler, Some(&filter), &Steps::default());
        assert_eq!(again.columns, 0);
        let stop = Steps::default();
        stop.cancel.store(true, Ordering::Relaxed);
        history.begin("stopped", 0);
        let other = Pattern::default().sampler(&[(t("Sand"), 1.0)]);
        assert!(paint_over_the_map(&mut history.edit(&mut world, 0), &other, None, &stop).cancelled);
    }
}
