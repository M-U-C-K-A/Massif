//! Gradients of terrains, for the Gradient brush: terrains laid side by
//! side in bands of so many blocks, A → B → C → D, and between each pair a
//! transition — a hard cut, a ragged noise or blobs of one going into the
//! other — so that two terrains painted next to each other blend instead of
//! meeting on a straight line.
//!
//! The brush lays a gradient out by distance: across the boundary between
//! two terrains (or from the edge of the selection), or from the brush's
//! centre to its rim. Everything is laid in world coordinates from the
//! map's seed and the gradient's own, so the same stroke paints the same
//! ground.

use crate::coords::{BlockRect, TILE_AREA, TilePos, local_index};
use crate::noise::Noise;
use crate::registry::{Registry, TerrainId};
use crate::world::Dimension;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The most bands a gradient has.
pub const MAX_BANDS: usize = 16;
/// The widest a band or a transition is, in blocks.
pub const MAX_WIDTH: f32 = 256.0;
/// The farthest the brush looks for a boundary, in blocks.
pub const MAX_REACH: i32 = 512;

/// How one terrain gives way to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TransitionKind {
    /// A clean line.
    #[default]
    Hard,
    /// A ragged edge: fingers of each terrain reach into the other.
    Noise,
    /// Blobs of the next terrain, of every size, growing denser until they
    /// join, with blobs of the first left in it.
    Blobs,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 3] = [TransitionKind::Hard, TransitionKind::Noise, TransitionKind::Blobs];

    pub const fn label(self) -> &'static str {
        match self {
            TransitionKind::Hard => "Hard cut",
            TransitionKind::Noise => "Noise",
            TransitionKind::Blobs => "Blobs",
        }
    }
}

/// A terrain and the blocks it spans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Band {
    /// The terrain (or terrain mix), by name.
    pub terrain: String,
    /// Blocks across.
    pub width: f32,
}

/// The way from one band to the next.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Blocks across (a hard cut falls in its middle).
    pub width: f32,
    /// Noise: how ragged, 0 to 1. Blobs: how many, 0 to 1 — few blobs keep
    /// close to the middle of the transition, many fill all of it.
    pub amount: f32,
    /// The size of the noise's shapes or of the largest blobs, in blocks.
    pub size: f32,
    /// Where across the transition the two terrains are half and half, as
    /// a picture editor's midpoint between two stops: below 0.5 the next
    /// terrain goes further into the first, above it the first goes
    /// further out into the next. 0.05 to 0.95.
    pub midpoint: f32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::Noise,
            width: 6.0,
            amount: 0.6,
            size: 6.0,
            midpoint: 0.5,
        }
    }
}

/// The closest a midpoint comes to either side of its transition.
pub const MIDPOINT_RANGE: (f32, f32) = (0.05, 0.95);

impl Transition {
    pub fn hard() -> Self {
        Self { kind: TransitionKind::Hard, width: 0.0, ..Self::default() }
    }

    pub fn noise(width: f32, amount: f32, size: f32) -> Self {
        Self { kind: TransitionKind::Noise, width, amount, size, ..Self::default() }
    }

    pub fn blobs(width: f32, amount: f32, size: f32) -> Self {
        Self { kind: TransitionKind::Blobs, width, amount, size, ..Self::default() }
    }

    /// `t` of the way across, bent so that the midpoint falls at a half.
    #[inline]
    fn bent(&self, t: f32) -> f32 {
        let m = self.midpoint.clamp(MIDPOINT_RANGE.0, MIDPOINT_RANGE.1);
        if (m - 0.5).abs() < 1e-3 { t } else { t.clamp(0.0, 1.0).powf(0.5f32.ln() / m.ln()) }
    }
}

/// A gradient of terrains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gradient {
    pub name: String,
    /// The bands, from the start of the gradient to its end.
    pub bands: Vec<Band>,
    /// One fewer than the bands: `transitions[k]` leads from band `k` to
    /// band `k + 1`.
    pub transitions: Vec<Transition>,
    /// Painted across a boundary, how many blocks from the start the
    /// boundary falls: the bands before it are laid on the side of the
    /// first terrains, those after on the side of the last.
    pub edge: f32,
    /// How much the bands widen and narrow along the edge, 0 to
    /// [`MAX_WANDER`]: at 0 every ring is as wide all round, as an outline;
    /// higher, a rim is wide here and thin there, as on real ground. The
    /// edge itself stays where it is.
    pub wander: f32,
    pub seed: u32,
}

/// The most the bands widen and narrow along the edge, as a share of their
/// width.
pub const MAX_WANDER: f32 = 0.9;

impl Default for Gradient {
    fn default() -> Self {
        built_in().swap_remove(0)
    }
}

/// Where a column lies along a gradient: in band `k`, or `t` of the way
/// through transition `k`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Band(usize),
    Transition(usize, f32),
}

impl Gradient {
    /// A gradient of these bands (terrain, width) and transitions, the
    /// boundary in the middle of its first transition: the first terrain
    /// keeps its ground, the others ring it.
    pub fn new(name: &str, bands: &[(&str, f32)], transitions: &[Transition]) -> Self {
        let mut g = Self {
            name: name.to_owned(),
            bands: bands.iter().map(|(t, w)| Band { terrain: (*t).to_owned(), width: *w }).collect(),
            transitions: transitions.to_vec(),
            edge: 0.0,
            wander: 0.4,
            seed: 1,
        };
        g.normalize();
        g.edge = g.bands[0].width + g.transitions.first().map_or(0.0, |t| t.width / 2.0);
        g.normalize();
        g
    }

    /// Set right what may be wrong (from a file): at least one band, one
    /// transition between each two, widths and settings in their ranges.
    pub fn normalize(&mut self) {
        if self.bands.is_empty() {
            self.bands.push(Band { terrain: "Grass".into(), width: 4.0 });
        }
        self.bands.truncate(MAX_BANDS);
        let clean = |v: f32, lo: f32, hi: f32, or: f32| if v.is_finite() { v.clamp(lo, hi) } else { or };
        for b in &mut self.bands {
            b.width = clean(b.width, 0.0, MAX_WIDTH, 4.0);
        }
        self.transitions.resize(self.bands.len() - 1, Transition::default());
        for t in &mut self.transitions {
            t.width = clean(t.width, 0.0, MAX_WIDTH, 0.0);
            t.amount = clean(t.amount, 0.0, 1.0, 0.5);
            t.size = clean(t.size, 1.0, MAX_WIDTH, 4.0);
            t.midpoint = clean(t.midpoint, MIDPOINT_RANGE.0, MIDPOINT_RANGE.1, 0.5);
        }
        self.edge = clean(self.edge, 0.0, self.total(), self.total() / 2.0);
        self.wander = clean(self.wander, 0.0, MAX_WANDER, 0.0);
    }

    /// Blocks from the start of the gradient to its end.
    pub fn total(&self) -> f32 {
        self.bands.iter().map(|b| b.width).sum::<f32>() + self.transitions.iter().map(|t| t.width).sum::<f32>()
    }

    /// The segments along the gradient in order — band 0, transition 0,
    /// band 1, … — each with where it starts and its width.
    pub fn segments(&self) -> impl Iterator<Item = (Place, f32, f32)> + '_ {
        let mut at = 0.0;
        (0..self.bands.len() * 2 - 1).map(move |s| {
            let (place, width) = if s % 2 == 0 { (Place::Band(s / 2), self.bands[s / 2].width) } else { (Place::Transition(s / 2, 0.0), self.transitions[s / 2].width) };
            let start = at;
            at += width;
            (place, start, width)
        })
    }

    /// Where position `p` (blocks from the start) lies; before the start is
    /// the first band, past the end the last.
    pub fn place(&self, p: f32) -> Place {
        let last = Place::Band(self.bands.len() - 1);
        if p.is_nan() || p <= 0.0 {
            return Place::Band(0);
        }
        for (place, start, width) in self.segments() {
            if p < start + width {
                return match place {
                    Place::Transition(k, _) => Place::Transition(k, ((p - start) / width.max(1e-6)).clamp(0.0, 1.0)),
                    band => band,
                };
            }
        }
        last
    }

    /// Where the middle of band `k` lies, blocks from the start.
    pub fn band_middle(&self, k: usize) -> f32 {
        self.segments().find(|s| s.0 == Place::Band(k)).map_or(0.0, |(_, start, w)| start + w / 2.0)
    }

    /// Split band `k` in two halves, a hard cut between them: the gradient
    /// keeps its length.
    pub fn split(&mut self, k: usize) {
        if let Some(b) = self.bands.get(k) {
            let half = (b.width / 2.0).round();
            self.split_at(k, half);
        }
    }

    /// Split band `k` `left` blocks into it (whole blocks, at least one
    /// each side), as a stop added under a picture editor's gradient: the
    /// new band of the same terrain after a hard cut, the gradient keeping
    /// its length. Returns whether it was split.
    pub fn split_at(&mut self, k: usize, left: f32) -> bool {
        if self.bands.len() >= MAX_BANDS || k >= self.bands.len() || self.bands[k].width < 2.0 {
            return false;
        }
        let band = self.bands[k].clone();
        let left = left.round().clamp(1.0, band.width - 1.0);
        self.bands[k].width = left;
        self.bands.insert(k + 1, Band { terrain: band.terrain, width: band.width - left });
        self.transitions.insert(k, Transition::hard());
        // The edge stays where it was along the gradient.
        self.normalize();
        true
    }

    /// Take band `k` out with the transition after it (before it, for the
    /// last); a gradient keeps one band.
    pub fn remove(&mut self, k: usize) {
        if self.bands.len() <= 1 || k >= self.bands.len() {
            return;
        }
        self.bands.remove(k);
        self.transitions.remove(k.min(self.transitions.len() - 1));
        self.normalize();
    }

    /// The same gradient the other way round: its end where its start was,
    /// the boundary as far from the end as it was from the start.
    pub fn reversed(&self) -> Gradient {
        let mut g = self.clone();
        g.bands.reverse();
        g.transitions.reverse();
        for t in &mut g.transitions {
            t.midpoint = 1.0 - t.midpoint;
        }
        g.edge = self.total() - self.edge;
        g
    }

    /// The terrain names it paints, each once, in order.
    pub fn terrains(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for b in &self.bands {
            if !out.contains(&b.terrain.as_str()) {
                out.push(&b.terrain);
            }
        }
        out
    }

    /// The gradient with its terrains, those of `reg`, seeded with the
    /// map's seed. `None` when none of its terrains is in the map.
    pub fn sampler(&self, reg: &Registry, world_seed: i64) -> Option<GradientSampler> {
        let mut g = self.clone();
        g.normalize();
        let ids: Vec<Option<TerrainId>> = g.bands.iter().map(|b| reg.terrain_by_name(&b.terrain)).collect();
        if ids.iter().all(Option::is_none) {
            return None;
        }
        // A terrain missing from the map gives its band to its neighbour.
        let mut terrains = Vec::with_capacity(ids.len());
        for k in 0..ids.len() {
            let near = (0..ids.len()).filter_map(|j| ids[j].map(|id| (j.abs_diff(k), id))).min_by_key(|c| c.0).map(|c| c.1);
            terrains.push(near.expect("a terrain of the map"));
        }
        let seed = (world_seed as u64) ^ ((g.seed as u64) << 32 | 0x6a2d) ^ 0x9e37_79b9_7f4a_7c15;
        let noise = Noise::new(seed);
        let noises = (0..g.transitions.len()).map(|k| noise.derive(k as u64 + 1)).collect();
        let salts = (0..g.transitions.len() as u64).map(|k| seed.rotate_left(17) ^ (k + 1).wrapping_mul(0xa076_1d64_78bd_642f)).collect();
        let wander = noise.derive(0x77a4);
        Some(GradientSampler { gradient: g, terrains, noises, salts, wander })
    }
}

/// A gradient ready to paint: its terrains in the map, its noises.
#[derive(Debug, Clone)]
pub struct GradientSampler {
    gradient: Gradient,
    terrains: Vec<TerrainId>,
    noises: Vec<Noise>,
    /// Where each transition's blobs lie.
    salts: Vec<u64>,
    /// What widens and narrows the bands along the edge.
    wander: Noise,
}

/// How deep column `(x, z)` (in blobs: `x / size`) lies in the blobs of a
/// transition: round shapes of every size, one around a point jittered in
/// each square of the grid, their edges roughened by a noise. Under 1,
/// inside a blob as large as it gets; the lower, the deeper.
fn blob_depth(noise: &Noise, salt: u64, x: f64, z: f64) -> f32 {
    let unit = |h: u32| h as f64 / u32::MAX as f64;
    let (cx, cz) = (x.floor() as i32, z.floor() as i32);
    let mut best = f64::INFINITY;
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (ix, iz) = (cx + dx, cz + dz);
            let jx = ix as f64 + unit(crate::compose::hash2(ix, iz, salt));
            let jz = iz as f64 + unit(crate::compose::hash2(ix, iz, salt ^ 0x5bd1_e995));
            // Many small blobs, a few large ones.
            let r = 0.1 + 0.9 * unit(crate::compose::hash2(ix, iz, salt ^ 0x2545_f491)).powf(2.2);
            best = best.min(((x - jx).powi(2) + (z - jz).powi(2)).sqrt() / r);
        }
    }
    (best * (1.0 + 0.25 * noise.simplex(x * 1.7, z * 1.7))) as f32
}

/// The blob depth under which a share `s` of the ground lies: blobs cover
/// that share of a transition where it calls for it. Learnt once from the
/// blobs themselves (their spread is the same at any size and seed).
fn blob_level(s: f32) -> f32 {
    static LEVELS: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let levels = LEVELS.get_or_init(|| {
        let noise = Noise::new(0xb10b);
        let n = 160;
        let mut v: Vec<f32> = (0..n * n).map(|i| blob_depth(&noise, 0x5eed, (i % n) as f64 * 0.31, (i / n) as f64 * 0.31)).collect();
        v.sort_by(f32::total_cmp);
        (0..=100).map(|k| v[(k * (v.len() - 1)) / 100]).collect()
    });
    let at = s.clamp(0.0, 1.0) * 100.0;
    let (k, f) = (at.floor() as usize, at.fract());
    let (a, b) = (levels[k.min(100)], levels[(k + 1).min(100)]);
    a + (b - a) * f
}

/// Fractal noise turned to about `0..1`, spread evenly: a share `s` of the
/// ground lies under `s`.
#[inline]
fn even(n: f64) -> f32 {
    // Fractal noise is near-normal around 0 with a spread of about 0.22;
    // its normal distribution, roughly.
    (0.5 + 0.5 * (n * 3.4).tanh()) as f32
}

impl GradientSampler {
    pub fn gradient(&self) -> &Gradient {
        &self.gradient
    }

    /// The terrain each band paints.
    pub fn terrains(&self) -> &[TerrainId] {
        &self.terrains
    }

    /// The band painted at position `p` of the gradient on column `(x, z)`.
    pub fn band(&self, p: f32, x: i32, z: i32) -> usize {
        match self.gradient.place(p) {
            Place::Band(k) => k,
            Place::Transition(k, t) => {
                let tr = &self.gradient.transitions[k];
                let t = tr.bent(t);
                let noise = &self.noises[k];
                let s = tr.size.max(1.0) as f64;
                let (fx, fz) = (x as f64 + 0.5, z as f64 + 0.5);
                let next = match tr.kind {
                    TransitionKind::Hard => t >= 0.5,
                    TransitionKind::Noise => {
                        let n = noise.fbm_of(crate::noise::Basis::Simplex, fx / s, fz / s, 4, 2.0, 0.55);
                        // The edge moves by up to the whole transition each way.
                        (t - 0.5) + tr.amount * (even(n) - 0.5) * 1.2 >= 0.0
                    }
                    TransitionKind::Blobs => {
                        // Blobs of the next terrain, of every size, that grow
                        // until they join, leaving blobs of the first.
                        let share = (0.5 + (t - 0.5) / tr.amount.max(0.05)).clamp(0.0, 1.0);
                        share >= 1.0 || (share > 0.0 && blob_depth(noise, self.salts[k], fx / s, fz / s) < blob_level(share))
                    }
                };
                k + next as usize
            }
        }
    }

    /// How much the bands are drawn out (over 1) or squeezed (under 1) at
    /// column `(x, z)`: the gradient's wander, by a noise some one and a
    /// half gradients across.
    #[inline]
    pub fn stretch(&self, x: i32, z: i32) -> f32 {
        let w = self.gradient.wander;
        if w <= 0.0 {
            return 1.0;
        }
        let s = (self.gradient.total() as f64 * 1.5).max(12.0);
        let n = self.wander.fbm_of(crate::noise::Basis::Simplex, x as f64 / s, z as f64 / s, 2, 2.0, 0.5);
        1.0 + w * (n as f32 * 1.8).clamp(-1.0, 1.0)
    }

    /// The terrain painted at position `p` on column `(x, z)`.
    #[inline]
    pub fn terrain(&self, p: f32, x: i32, z: i32) -> TerrainId {
        self.terrains[self.band(p, x, z)]
    }

    /// Which side of the boundary terrain `t` lies on: `Some(true)` for
    /// the bands before the edge, `Some(false)` after, `None` when the
    /// gradient does not paint it. The first band is always before, the
    /// last after, wherever the edge falls; a terrain on both sides counts
    /// where it is first.
    pub fn side(&self, t: TerrainId) -> Option<bool> {
        let n = self.terrains.len();
        let k = self.terrains.iter().position(|&id| id == t)?;
        Some(n == 1 || k == 0 || (k + 1 < n && self.gradient.band_middle(k) < self.gradient.edge))
    }

    /// What terrain `t` is to the Across layout: the gradient's first
    /// terrain ([`INSIDE`]) or its last ([`OUTSIDE`]), between which the
    /// boundary is found and which alone are painted; one of its middle
    /// terrains ([`MIDDLE`]), the transition a stroke laid; or a terrain it
    /// does not paint ([`OTHER`]). A gradient that ends with the terrain it
    /// starts with sides its terrains as [`GradientSampler::side`] says.
    fn role(&self, t: TerrainId) -> u8 {
        let (first, last) = (self.terrains[0], self.terrains[self.terrains.len() - 1]);
        if first == last {
            return match self.side(t) {
                Some(true) => INSIDE,
                Some(false) => OUTSIDE,
                None => OTHER,
            };
        }
        if t == first {
            INSIDE
        } else if t == last {
            OUTSIDE
        } else if self.terrains.contains(&t) {
            MIDDLE
        } else {
            OTHER
        }
    }
}

/// How the Gradient brush measures where a column lies along the gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Layout {
    /// By distance to the boundary between two terrains (or to the
    /// selection's edge).
    #[default]
    Across,
    /// From the brush's centre (the start) to its rim (the end).
    Radial,
}

/// What each column was to the gradient before it painted there (see
/// [`GradientSampler::role`]): kept for the stroke, so that what it paints
/// does not move the boundary under it, and from stroke to stroke of the
/// same gradient ([`Sides::begin`], [`Sides::end`]), so that painting
/// again where it painted measures from the boundary as it first was and
/// paints the same: the blocks of its transitions (its first terrain
/// scattered past the edge, say) never stand for a new boundary. A tile
/// changed since by anything else is read afresh.
#[derive(Debug, Default, Clone)]
pub struct Sides {
    tiles: HashMap<TilePos, Box<[u8; TILE_AREA]>>,
    /// The terrains of each tile as the last stroke left them (or as they
    /// were read): while they are still so, what was learnt holds.
    seen: HashMap<TilePos, u64>,
    /// The tiles looked at by the stroke under way.
    checked: HashSet<TilePos>,
    /// What the memory is for (the gradient, its way, the level…).
    key: u64,
}

/// Tiles of sides kept from stroke to stroke, at most (16 kB each).
const SIDES_KEPT: usize = 4096;

/// What the terrains of a tile are, as a number: equal while they are
/// unchanged.
fn terrain_print(lvl: &crate::level::Level, pos: TilePos) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(tile) = lvl.view(pos) {
        tile.terrain.dense().hash(&mut h);
        tile.fluid.dense().hash(&mut h);
        tile.bottom.is_some().hash(&mut h);
    }
    h.finish()
}

/// No column there.
const NONE: u8 = 0;
/// The gradient's first terrain.
const INSIDE: u8 = 1;
/// Its last terrain.
const OUTSIDE: u8 = 2;
/// A terrain the gradient does not paint: never painted, no side.
const OTHER: u8 = 3;
/// One of its middle terrains: the transition a stroke laid, never painted
/// again, no side.
const MIDDLE: u8 = 4;

impl Sides {
    pub fn new() -> Self {
        Self::default()
    }

    /// A stroke begins, of the gradient (and way, level…) `key` stands for:
    /// what was learnt for another is forgotten.
    pub fn begin(&mut self, key: u64) {
        if key != self.key || self.tiles.len() > SIDES_KEPT {
            *self = Sides { key, ..Sides::default() };
        }
        self.checked.clear();
    }

    /// The stroke ended: the terrains it left are those its sides hold for.
    pub fn end(&mut self, dim: &Dimension, level: usize) {
        let Some(lvl) = dim.levels.get(level) else { return };
        for &pos in &self.checked {
            self.seen.insert(pos, terrain_print(lvl, pos));
        }
        self.checked.clear();
    }

    /// Learn the columns of `rect` not learnt in this stroke, unless what
    /// an earlier one learnt still holds (the tile as it left it).
    fn learn(&mut self, dim: &Dimension, level: usize, rect: BlockRect, sampler: &GradientSampler) {
        let Some(lvl) = dim.levels.get(level) else { return };
        let mut sides: HashMap<u16, u8> = HashMap::new();
        for pos in rect.tiles() {
            if !self.checked.insert(pos) {
                continue;
            }
            let print = terrain_print(lvl, pos);
            if self.tiles.contains_key(&pos) && self.seen.get(&pos) == Some(&print) {
                continue;
            }
            self.seen.insert(pos, print);
            let mut cells = Box::new([NONE; TILE_AREA]);
            if let Some(tile) = lvl.view(pos) {
                let slab = tile.bottom.is_some();
                for (i, c) in cells.iter_mut().enumerate() {
                    if slab && !tile.present(i) {
                        continue;
                    }
                    // The gradient stops at the water's edge: the ground
                    // under water (a sea floor) is left as it is.
                    let fluid = tile.fluid.get(i);
                    if fluid != crate::tile::NO_FLUID && fluid as f32 > tile.top.get(i) {
                        *c = OTHER;
                        continue;
                    }
                    let t = tile.terrain.get(i);
                    *c = *sides.entry(t).or_insert_with(|| sampler.role(TerrainId(t)));
                }
            }
            self.tiles.insert(pos, cells);
        }
    }

    /// What every column of `rect` is to the gradient, row-major.
    fn read(&mut self, dim: &Dimension, level: usize, rect: BlockRect, sampler: &GradientSampler) -> Vec<u8> {
        use rayon::prelude::*;
        self.learn(dim, level, rect, sampler);
        let w = rect.width() as usize;
        let mut out = vec![NONE; w * rect.depth().max(0) as usize];
        let tiles = &self.tiles;
        out.par_chunks_mut(w.max(1)).enumerate().for_each(|(row, line)| {
            let z = rect.z0 + row as i32;
            let mut x = rect.x0;
            while x < rect.x1 {
                let pos = TilePos::containing(x, z);
                let end = (pos.rect().x1).min(rect.x1);
                let cells = &tiles[&pos];
                let start = local_index(x, z);
                let n = (end - x) as usize;
                line[(x - rect.x0) as usize..(end - rect.x0) as usize].copy_from_slice(&cells[start..start + n]);
                x = end;
            }
        });
        out
    }
}

/// Squared Euclidean distance transform of one line (Felzenszwalb and
/// Huttenlocher): `f` holds 0 at the sources and a large value elsewhere;
/// `d` receives the squared distance to the nearest source.
fn edt_line(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        if f[q] >= BIG {
            continue;
        }
        if f[v[0]] >= BIG {
            v[0] = q;
            continue;
        }
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    if f[v[0]] >= BIG {
        d.fill(BIG);
        return;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *out = dq * dq + f[p];
    }
}

const BIG: f32 = 1e20;

/// The Euclidean distance from every cell of a `w × h` grid to the nearest
/// cell where `source` is true (`BIG` where there is none).
pub fn distance_field(w: usize, h: usize, source: impl Fn(usize) -> bool + Sync) -> Vec<f32> {
    use rayon::prelude::*;
    let mut grid: Vec<f32> = (0..w * h).into_par_iter().map(|i| if source(i) { 0.0 } else { BIG }).collect();
    if w == 0 || h == 0 {
        return grid;
    }
    // Columns first (into a transposed copy), then rows.
    let mut cols = vec![BIG; w * h];
    cols.par_chunks_mut(h).enumerate().for_each(|(x, out)| {
        let f: Vec<f32> = (0..h).map(|y| grid[y * w + x]).collect();
        let (mut v, mut z) = (vec![0usize; h], vec![0f32; h + 1]);
        edt_line(&f, out, &mut v, &mut z);
    });
    grid.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let f: Vec<f32> = (0..w).map(|x| cols[x * h + y]).collect();
        let (mut v, mut z) = (vec![0usize; w], vec![0f32; w + 1]);
        edt_line(&f, out, &mut v, &mut z);
        for d in out.iter_mut() {
            *d = if *d >= BIG * 0.5 { BIG } else { d.sqrt() };
        }
    });
    grid
}

/// Where every column of `clip` lies along the gradient (blocks from its
/// start, row-major; NaN where nothing is to be painted: past its ends, or
/// with no boundary in reach), for a dab at
/// `(cx, cz)` of a brush reaching `reach` blocks.
#[allow(clippy::too_many_arguments)]
pub fn positions(dim: &Dimension, level: usize, sampler: &GradientSampler, layout: Layout, sides: &mut Sides, clip: BlockRect, cx: f32, cz: f32, reach: f32) -> Vec<f32> {
    use rayon::prelude::*;
    let g = &sampler.gradient;
    let (w, h) = (clip.width().max(0) as usize, clip.depth().max(0) as usize);
    if layout == Layout::Radial {
        let total = g.total();
        let r = reach.max(0.5);
        let mut out = Vec::with_capacity(w * h);
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (z as f32 + 0.5 - cz).powi(2)).sqrt();
                out.push(d / r * total * sampler.stretch(x, z));
            }
        }
        return out;
    }
    // Columns farther from the edge than this are past the gradient's
    // ends however much it wanders.
    let far = g.total() / (1.0 - g.wander).max(0.1) + 1.0;
    // Far enough to find the boundary of every column the gradient spans.
    let reach = (far.ceil() as i32 + 2).min(MAX_REACH);
    let rect = clip.expand(reach);
    let (rw, rh) = (rect.width() as usize, rect.depth() as usize);
    // Inside the selection when there is one: the gradient starts at its
    // edge and runs in.
    if let Some(sel) = dim.selection.as_ref() {
        let weights = sel.weights(rect).unwrap_or_else(|| vec![0; rw * rh]);
        let to_out = distance_field(rw, rh, |i| weights[i] < 128);
        let mut out = vec![f32::NAN; w * h];
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let i = (z - rect.z0) as usize * rw + (x - rect.x0) as usize;
                let d = to_out[i] - 0.5;
                if weights[i] >= 128 && d < far {
                    let p = d * sampler.stretch(x, z);
                    if p < g.total() {
                        out[(z - clip.z0) as usize * w + (x - clip.x0) as usize] = p;
                    }
                }
            }
        }
        return out;
    }
    let side = sides.read(dim, level, rect, sampler);
    let has = |s: u8| side.contains(&s);
    let mut out = vec![f32::NAN; w * h];
    if !has(INSIDE) || !has(OUTSIDE) {
        // No boundary in reach: nothing to blend.
        return out;
    }
    let (to_in, to_out) = rayon::join(|| distance_field(rw, rh, |i| side[i] == INSIDE), || distance_field(rw, rh, |i| side[i] == OUTSIDE));
    // The middle terrains: a transition laid already, unless none is near.
    let to_mid = has(MIDDLE).then(|| distance_field(rw, rh, |i| side[i] == MIDDLE));
    let total = g.total();
    // Where the middle terrains can lie along the gradient: from the start
    // of its first transition to the end of its last.
    let (mid_from, mid_to) = (g.bands[0].width, total - g.bands[g.bands.len() - 1].width);
    out.par_chunks_mut(w.max(1)).enumerate().for_each(|(row, line)| {
        let z = clip.z0 + row as i32;
        for (x, o) in (clip.x0..clip.x1).zip(line.iter_mut()) {
            let i = (z - rect.z0) as usize * rw + (x - rect.x0) as usize;
            // Only the gradient's first and last terrains are painted, each
            // by its distance to the other across the boundary (which lies
            // between the cells, half a block from each). Its middle ones
            // are a transition laid already: painting again where one was
            // laid keeps what is there. A column of the first or last
            // terrain nearer a middle one than the other end is measured
            // from as far along as a middle terrain can lie, so that it
            // lies at least as far toward its own end as it did, where its
            // band is still its own (the band a column takes only grows
            // along the gradient); and so does one whose other end lies
            // farther off now. Terrains it does not paint (a sand shore,
            // the sea floor) stay as they are.
            let mid = to_mid.as_ref().map_or(f32::INFINITY, |m| m[i] - 0.5);
            let s = sampler.stretch(x, z);
            let p = match side[i] {
                INSIDE if mid < to_out[i] - 0.5 => mid_from - mid * s,
                INSIDE => g.edge - (to_out[i] - 0.5) * s,
                OUTSIDE if mid < to_in[i] - 0.5 => mid_to + mid * s,
                OUTSIDE => g.edge + (to_in[i] - 0.5) * s,
                _ => f32::NAN,
            };
            let near = match side[i] {
                INSIDE => mid.min(to_out[i] - 0.5),
                OUTSIDE => mid.min(to_in[i] - 0.5),
                _ => f32::INFINITY,
            };
            // Past the gradient's ends the ground is left as it is.
            *o = if near < far && (0.0..total).contains(&p) { p } else { f32::NAN };
        }
    });
    out
}

/// Built-in gradients, of terrains every map has: the two examples of the
/// terraformers who asked for the brush (patches of rock ringed by a
/// gold-green rim fading into grass, a dark pit ringed by two greys), the
/// gradient of their sketch (30 blocks, a noise, blobs and a hard cut), and
/// a shore.
pub fn built_in() -> Vec<Gradient> {
    vec![
        Gradient::new(
            "Rock to grass",
            &[("Stone", 4.0), ("Andesite", 3.0), ("Moss", 4.0), ("Grass", 4.0)],
            &[Transition::noise(3.0, 0.7, 3.0), Transition::noise(3.0, 0.6, 3.0), Transition::blobs(6.0, 0.65, 5.0)],
        ),
        Gradient::new(
            "Pit to grass",
            &[("Deepslate", 4.0), ("Andesite", 3.0), ("Gravel", 3.0), ("Grass", 4.0)],
            &[Transition::hard(), Transition::noise(3.0, 0.6, 3.0), Transition::blobs(5.0, 0.7, 4.0)],
        ),
        Gradient::new(
            "Deciduous forest to grass",
            &[("Podzol", 4.0), ("Coarse Dirt", 5.0), ("Moss", 6.0), ("Grass", 4.0)],
            &[Transition::noise(6.0, 0.7, 5.0), Transition::blobs(5.0, 0.6, 5.0), Transition::hard()],
        ),
        Gradient::new(
            "Shore",
            &[("Sand", 6.0), ("Gravel", 3.0), ("Coarse Dirt", 3.0), ("Grass", 4.0)],
            &[Transition::noise(4.0, 0.6, 5.0), Transition::blobs(4.0, 0.5, 3.0), Transition::noise(5.0, 0.8, 4.0)],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        Registry::vanilla()
    }

    #[test]
    fn places_follow_the_widths() {
        let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition::noise(6.0, 0.5, 4.0)]);
        assert_eq!(g.total(), 14.0);
        assert_eq!(g.place(-3.0), Place::Band(0));
        assert_eq!(g.place(2.0), Place::Band(0));
        assert_eq!(g.place(7.0), Place::Transition(0, 0.5));
        assert_eq!(g.place(12.0), Place::Band(1));
        assert_eq!(g.place(99.0), Place::Band(1));
        assert_eq!(g.edge, 7.0);
    }

    #[test]
    fn normalize_mends_a_broken_gradient() {
        let mut g = Gradient { bands: vec![], transitions: vec![Transition::default(); 4], edge: f32::NAN, ..Gradient::default() };
        g.normalize();
        assert_eq!(g.bands.len(), 1);
        assert!(g.transitions.is_empty());
        assert!(g.edge.is_finite());
        let mut g = Gradient::new("t", &[("Stone", -3.0), ("Grass", f32::INFINITY)], &[]);
        g.normalize();
        assert_eq!(g.transitions.len(), 1);
        assert!(g.bands.iter().all(|b| (0.0..=MAX_WIDTH).contains(&b.width)));
    }

    #[test]
    fn split_and_remove_keep_one_transition_between_bands() {
        let mut g = Gradient::new("t", &[("Stone", 4.0), ("Andesite", 4.0), ("Moss", 3.0), ("Grass", 4.0)], &[Transition::hard(); 3]);
        let total = g.total();
        assert!(g.split_at(0, 3.0));
        assert_eq!(g.bands.iter().map(|b| b.width).collect::<Vec<_>>(), [3.0, 1.0, 4.0, 3.0, 4.0]);
        assert_eq!(g.total(), total, "a new stop keeps the length");
        assert!(!g.split_at(1, 0.5), "a band of one block is not split");
        g.remove(1);
        g.split(1);
        assert_eq!(g.bands.len(), 5);
        assert_eq!(g.transitions.len(), 4);
        g.remove(4);
        g.remove(0);
        assert_eq!((g.bands.len(), g.transitions.len()), (3, 2));
        for _ in 0..5 {
            g.remove(0);
        }
        assert_eq!((g.bands.len(), g.transitions.len()), (1, 0));
    }

    /// The share of columns of a strip `t` of the way through a transition
    /// that take the next band.
    fn share(s: &GradientSampler, p: f32) -> f32 {
        let n = 4000;
        (0..n).filter(|i| s.band(p, i % 200 * 3, i / 200 * 7) == 1).count() as f32 / n as f32
    }

    #[test]
    fn a_hard_cut_falls_in_the_middle() {
        let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition { width: 2.0, ..Transition::hard() }]);
        let s = g.sampler(&reg(), 0).unwrap();
        assert_eq!(share(&s, 4.9), 0.0);
        assert_eq!(share(&s, 5.1), 1.0);
    }

    #[test]
    fn noise_and_blobs_go_in_and_out_of_each_other() {
        for tr in [Transition::noise(10.0, 0.8, 4.0), Transition::blobs(10.0, 0.8, 4.0)] {
            let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[tr]);
            let s = g.sampler(&reg(), 3).unwrap();
            let (early, middle, late) = (share(&s, 5.5), share(&s, 9.0), share(&s, 12.5));
            // Some of the next terrain early, some of the first late, and
            // more of the next the farther along.
            assert!(early > 0.0 && early < 0.3, "{:?}: {early}", tr.kind);
            assert!((0.3..0.7).contains(&middle), "{:?}: {middle}", tr.kind);
            assert!(late > 0.7 && late < 1.0, "{:?}: {late}", tr.kind);
            assert_eq!(share(&s, 3.0), 0.0);
            assert_eq!(share(&s, 14.5), 1.0);
        }
    }

    #[test]
    fn blobs_come_in_several_sizes() {
        let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition::blobs(10.0, 0.7, 8.0)]);
        let s = g.sampler(&reg(), 5).unwrap();
        // Blobs of the next terrain early in the transition: count them
        // and their sizes over a field.
        let n = 160usize;
        let grid: Vec<bool> = (0..n * n).map(|i| s.band(6.0, (i % n) as i32, (i / n) as i32) == 1).collect();
        let mut seen = vec![false; n * n];
        let mut sizes = Vec::new();
        for start in 0..n * n {
            if !grid[start] || seen[start] {
                continue;
            }
            let mut stack = vec![start];
            seen[start] = true;
            let mut size = 0;
            while let Some(c) = stack.pop() {
                size += 1;
                let (x, z) = (c % n, c / n);
                for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                    if nx < 0 || nz < 0 || nx >= n as i32 || nz >= n as i32 {
                        continue;
                    }
                    let j = nz as usize * n + nx as usize;
                    if grid[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            sizes.push(size);
        }
        sizes.sort();
        assert!(sizes.len() >= 8, "blobs: {sizes:?}");
        assert!(sizes[sizes.len() - 1] >= 4 * sizes[sizes.len() / 2], "blobs of one size: {sizes:?}");
    }

    #[test]
    fn blobs_cover_the_share_called_for_and_stand_apart() {
        let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition::blobs(10.0, 1.0, 6.0)]);
        let s = g.sampler(&reg(), 9).unwrap();
        for t in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let got = share(&s, 4.0 + 10.0 * t);
            assert!((got - t).abs() < 0.08, "{t}: {got}");
        }
        // Early on, separate blobs: most of the next terrain's columns have
        // a neighbour of it, few stand alone.
        let n = 120usize;
        let grid: Vec<bool> = (0..n * n).map(|i| s.band(6.0, (i % n) as i32, (i / n) as i32) == 1).collect();
        let alone = (1..n - 1).flat_map(|z| (1..n - 1).map(move |x| (x, z))).filter(|&(x, z)| grid[z * n + x] && !grid[z * n + x - 1] && !grid[z * n + x + 1] && !grid[(z - 1) * n + x] && !grid[(z + 1) * n + x]).count();
        let all = grid.iter().filter(|&&b| b).count();
        assert!(all > 100 && alone * 10 < all, "{alone} alone of {all}");
    }

    #[test]
    fn the_same_seed_paints_the_same_and_another_seed_not() {
        let g = Gradient::default();
        let (a, b, c) = (g.sampler(&reg(), 42).unwrap(), g.sampler(&reg(), 42).unwrap(), g.sampler(&reg(), 43).unwrap());
        let at = |s: &GradientSampler| (0..5000).map(|i| s.band(6.5, i % 71, i / 71)).collect::<Vec<_>>();
        assert_eq!(at(&a), at(&b));
        assert_ne!(at(&a), at(&c));
        let mut reseeded = g.clone();
        reseeded.seed = 2;
        assert_ne!(at(&a), at(&reseeded.sampler(&reg(), 42).unwrap()));
    }

    #[test]
    fn a_terrain_missing_from_the_map_gives_way_to_its_neighbour() {
        let g = Gradient::new("t", &[("Stone", 4.0), ("Nowhere", 4.0), ("Grass", 4.0)], &[Transition::hard(), Transition::hard()]);
        let r = reg();
        let s = g.sampler(&r, 0).unwrap();
        assert_eq!(s.terrains()[1], r.terrain_by_name("Stone").unwrap());
        assert!(Gradient::new("t", &[("Nowhere", 4.0)], &[]).sampler(&r, 0).is_none());
    }

    #[test]
    fn the_distance_field_is_euclidean() {
        let (w, h) = (40, 30);
        let d = distance_field(w, h, |i| i == 10 * w + 5 || i == 25 * w + 33);
        for y in 0..h {
            for x in 0..w {
                let a = ((x as f32 - 5.0).powi(2) + (y as f32 - 10.0).powi(2)).sqrt();
                let b = ((x as f32 - 33.0).powi(2) + (y as f32 - 25.0).powi(2)).sqrt();
                assert!((d[y * w + x] - a.min(b)).abs() < 1e-3, "({x}, {y})");
            }
        }
        assert!(distance_field(5, 5, |_| false).iter().all(|&v| v >= BIG));
    }

    #[test]
    fn the_first_terrain_lies_before_the_edge_and_the_last_after_it() {
        let r = reg();
        let id = |n: &str| r.terrain_by_name(n).unwrap();
        let four = Gradient::new("t", &[("Stone", 4.0), ("Andesite", 4.0), ("Moss", 3.0), ("Grass", 4.0)], &[Transition::noise(4.0, 0.7, 4.0), Transition::blobs(5.0, 0.6, 5.0), Transition::noise(4.0, 0.5, 3.0)]);
        for edge in [0.0, 2.0, 14.0, 28.0] {
            let g = Gradient { edge, ..four.clone() };
            let s = g.sampler(&r, 0).unwrap();
            assert_eq!(s.side(id("Stone")), Some(true), "edge {edge}");
            assert_eq!(s.side(id("Grass")), Some(false), "edge {edge}");
            assert_eq!(s.side(id("Sand")), None);
        }
        let g = Gradient { edge: 15.0, ..four };
        let s = g.sampler(&r, 0).unwrap();
        assert_eq!((s.side(id("Andesite")), s.side(id("Moss"))), (Some(true), Some(false)));
    }

    #[test]
    fn the_midpoint_sets_how_far_each_terrain_goes_into_the_other() {
        for tr in [Transition::hard(), Transition::noise(10.0, 0.6, 4.0), Transition::blobs(10.0, 0.6, 4.0)] {
            let tr = Transition { width: 10.0, ..tr };
            let at = |m: f32| {
                let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition { midpoint: m, ..tr }]);
                share(&g.sampler(&reg(), 2).unwrap(), 8.0)
            };
            // Four blocks into the transition (0.4 of it): the next terrain
            // takes more of it as the midpoint moves towards the first.
            let (early, even, late) = (at(0.25), at(0.5), at(0.75));
            assert!(early >= even && even >= late && early > late, "{:?}: {early} {even} {late}", tr.kind);
        }
        // The other way round, the midpoint mirrors.
        let g = Gradient::new("t", &[("Stone", 4.0), ("Grass", 4.0)], &[Transition { midpoint: 0.3, ..Transition::noise(10.0, 0.6, 4.0) }]);
        assert!((g.reversed().transitions[0].midpoint - 0.7).abs() < 1e-6);
        // Old files without one are halfway.
        let old: Transition = serde_json::from_str(r#"{"kind":"Noise","width":4,"amount":0.5,"size":3}"#).unwrap();
        assert_eq!(old.midpoint, 0.5);
    }

    #[test]
    fn the_sketch_of_the_terraformers_is_built_in() {
        let g = built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
        assert_eq!(g.total(), 30.0);
        assert_eq!(g.transitions.iter().map(|t| t.kind).collect::<Vec<_>>(), [TransitionKind::Noise, TransitionKind::Blobs, TransitionKind::Hard]);
        // The first terrain keeps its ground: the edge in its transition.
        assert_eq!(g.edge, 7.0);
    }

    #[test]
    fn built_in_gradients_paint_terrains_every_map_has() {
        let r = reg();
        for g in built_in() {
            for t in g.terrains() {
                assert!(r.terrain_by_name(t).is_some(), "{}: {t}", g.name);
            }
        }
    }

    #[test]
    fn gradients_read_back_from_json() {
        for g in built_in() {
            let json = serde_json::to_string(&g).unwrap();
            assert_eq!(serde_json::from_str::<Gradient>(&json).unwrap(), g);
        }
        let old: Gradient = serde_json::from_str(r#"{"name":"x","bands":[{"terrain":"Sand","width":3}]}"#).unwrap();
        assert_eq!(old.bands.len(), 1);
    }
}
