//! Caves, ravines and caverns, carved into the sculpted matter.
//!
//! The generator lays out its features first, as chains of balls: winding
//! tunnels that branch (the "spaghetti" of Minecraft), clusters of wide
//! chambers with pillars left standing (its "cheese"), ravines — long,
//! deep, narrow cuts, some open to the sky — and large caverns, which can
//! hold a lake of water or lava up to a level. Each tile of the map seeds
//! its own features, so that the same seed carves the same caves wherever
//! it runs, over the whole map or over a part of it.
//!
//! Then it carves them into the 3D matter (see [`crate::voxel`]), a tile at
//! a time: only the 16³ chunks a feature reaches into are made, and only
//! those whose matter changed are kept. Closed caves keep to the depth
//! range and leave a roof under the ground's surface; openings reach up to
//! the sky.

use crate::auto::Steps;
use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::voxel::{self, ChunkPos, Fluid, SIZE, SOLID, VoxelChunk};
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// What to carve, and where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaveSettings {
    pub seed: u32,
    /// Winding tunnels, with their chambers.
    pub tunnels: bool,
    /// Long, deep, narrow cuts.
    pub ravines: bool,
    /// Large open caverns.
    pub caverns: bool,
    /// How many, 0 to 1.
    pub density: f32,
    /// How wide and long, 0.5 to 2 (1: as Minecraft carves them).
    pub size: f32,
    /// The heights the caves keep to, `low..=high`.
    pub low: i32,
    pub high: i32,
    /// The share of tunnels and ravines that open to the surface, 0 to 1.
    pub openings: f32,
    /// What fills the caverns up to `lake_level`; `None`: nothing.
    pub lake: Option<Fluid>,
    pub lake_level: i32,
}

impl CaveSettings {
    /// Settings to start from for a dimension whose sea is at `sea`.
    pub fn for_dimension(dim: &Dimension, sea: i32) -> Self {
        let low = dim.min_y + 8;
        let high = (sea - 6).clamp(low + 16, dim.max_y - 1);
        Self {
            seed: 1,
            tunnels: true,
            ravines: true,
            caverns: true,
            density: 0.5,
            size: 1.0,
            low,
            high,
            openings: 0.2,
            lake: None,
            lake_level: low + 16,
        }
    }

    /// Whether it carves anything at all.
    pub fn carves(&self) -> bool {
        (self.tunnels || self.ravines || self.caverns) && self.density > 0.0 && self.high > self.low
    }
}

/// What carving gave: the chunks it changed, to put in place.
#[derive(Debug, Default)]
pub struct Carved {
    pub chunks: Vec<(ChunkPos, Arc<VoxelChunk>)>,
    /// Blocks turned from matter to air.
    pub blocks: usize,
    /// Of those, blocks filled with water or lava.
    pub flooded: usize,
    pub cancelled: bool,
}

/// Blocks of matter kept over a closed cave, under the ground's surface.
const ROOF: f32 = 5.0;
/// How far a feature reaches from the tile that seeds it, in tiles.
const REACH: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Tunnel,
    /// A chamber with pillars.
    Cheese,
    Ravine,
    Cavern,
}

/// One piece of a feature: an ellipsoid of radii `r` at `c`.
#[derive(Debug, Clone, Copy)]
struct Ball {
    c: [f32; 3],
    r: [f32; 3],
    kind: Kind,
    /// Allowed up to the sky (an opening), not only inside the range.
    open: bool,
}

impl Ball {
    fn x_range(&self) -> (i32, i32) {
        ((self.c[0] - self.r[0]).floor() as i32 - 1, (self.c[0] + self.r[0]).ceil() as i32 + 1)
    }
    fn z_range(&self) -> (i32, i32) {
        ((self.c[2] - self.r[2]).floor() as i32 - 1, (self.c[2] + self.r[2]).ceil() as i32 + 1)
    }
    fn y_range(&self) -> (i32, i32) {
        ((self.c[1] - self.r[1]).floor() as i32 - 1, (self.c[1] + self.r[1]).ceil() as i32 + 1)
    }
}

// -------------------------------------------------------------- random --

/// A small, fast generator (SplitMix64): the same numbers on every
/// machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// In `[0, 1)`.
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
    /// How many of something expected `mean` times.
    fn count(&mut self, mean: f32) -> usize {
        let whole = mean.floor();
        whole as usize + usize::from(self.f() < mean - whole)
    }
}

fn hash(seed: u64, a: i64, b: i64, salt: u64) -> u64 {
    let mut r = Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (a as u64).wrapping_mul(0x9E37_79B1) ^ (b as u64).wrapping_mul(0x85EB_CA77_C2B2_AE63) ^ salt);
    r.next()
}

/// Value noise in 3D, 0 to 1, smooth.
fn noise3(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (xf as i64, yf as i64, zf as i64);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v, w) = (s(x - xf), s(y - yf), s(z - zf));
    let at = |i: i64, j: i64, k: i64| (hash(seed, (i * 73_856_093) ^ (k * 19_349_663), j, 7) >> 40) as f32 / (1u64 << 24) as f32;
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(at(xi, yi, zi), at(xi + 1, yi, zi), u);
    let x10 = lerp(at(xi, yi + 1, zi), at(xi + 1, yi + 1, zi), u);
    let x01 = lerp(at(xi, yi, zi + 1), at(xi + 1, yi, zi + 1), u);
    let x11 = lerp(at(xi, yi + 1, zi + 1), at(xi + 1, yi + 1, zi + 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

// ------------------------------------------------------------- surface --

/// The height of the ground, a sample every 8 blocks, for laying features
/// out (openings start on it, open ravines follow it).
struct Heights {
    tiles: HashMap<TilePos, Box<[f32; 256]>>,
}

impl Heights {
    fn read(dim: &Dimension, tiles: &[TilePos]) -> Self {
        let ground = dim.ground();
        let tiles = tiles
            .par_iter()
            .filter_map(|&pos| {
                let tile = ground.view(pos)?;
                let mut h = Box::new([0f32; 256]);
                for j in 0..16 {
                    for i in 0..16 {
                        h[j * 16 + i] = tile.top.get(local_index(i as i32 * 8 + 4, j as i32 * 8 + 4));
                    }
                }
                Some((pos, h))
            })
            .collect();
        Self { tiles }
    }

    fn at(&self, x: f32, z: f32) -> Option<f32> {
        let (x, z) = (x.floor() as i32, z.floor() as i32);
        let h = self.tiles.get(&TilePos::containing(x, z))?;
        let (i, j) = (x.rem_euclid(TILE_SIZE) / 8, z.rem_euclid(TILE_SIZE) / 8);
        Some(h[(j * 16 + i) as usize])
    }
}

// -------------------------------------------------------------- layout --

struct Layout<'a> {
    s: &'a CaveSettings,
    heights: &'a Heights,
    /// The range closed caves keep to, a little inside so that their
    /// balls stay in it.
    low: f32,
    high: f32,
}

impl Layout<'_> {
    /// The features tile `tile` seeds.
    fn tile(&self, tile: TilePos) -> Vec<Ball> {
        let s = self.s;
        let mut rng = Rng::new(hash(s.seed as u64, tile.x as i64, tile.z as i64, 0xCA7E));
        let mut out = Vec::new();
        let (x0, z0) = tile.origin();
        let spot = |rng: &mut Rng| [x0 as f32 + rng.range(0.0, TILE_SIZE as f32), z0 as f32 + rng.range(0.0, TILE_SIZE as f32)];
        // Each kind draws from its own numbers, so that turning one off
        // leaves the others as they were.
        if s.tunnels {
            let mut rng = Rng::new(rng.next() ^ 1);
            for _ in 0..rng.count(s.density * 1.6) {
                let [x, z] = spot(&mut rng);
                self.tunnel_system(&mut rng, x, z, &mut out);
            }
        } else {
            rng.next();
        }
        if s.ravines {
            let mut rng = Rng::new(rng.next() ^ 2);
            for _ in 0..rng.count(s.density * 0.14) {
                let [x, z] = spot(&mut rng);
                self.ravine(&mut rng, x, z, &mut out);
            }
        } else {
            rng.next();
        }
        if s.caverns {
            let mut rng = Rng::new(rng.next() ^ 3);
            for _ in 0..rng.count(s.density * 0.07) {
                let [x, z] = spot(&mut rng);
                self.cavern(&mut rng, x, z, &mut out);
            }
        }
        out
    }

    fn tunnel_system(&self, rng: &mut Rng, x: f32, z: f32, out: &mut Vec<Ball>) {
        let size = self.s.size;
        let open = rng.f() < self.s.openings;
        let (y, pitch) = match (open, self.heights.at(x, z)) {
            (true, Some(top)) if top > self.low + 4.0 => (top + 1.0, -0.8),
            _ => (rng.range(self.low, self.high), rng.range(-0.2, 0.2)),
        };
        let open = open && pitch < -0.5;
        let length = (rng.range(70.0, 190.0) * size.sqrt()) as usize;
        let radius = rng.range(1.5, 3.0) * size;
        let yaw = rng.range(0.0, 2.0 * PI);
        self.worm(rng, [x, y, z], yaw, pitch, length, radius, open, true, out);
        // A chamber now and then, with pillars standing in it.
        if rng.f() < 0.3 {
            let c = [x + rng.range(-24.0, 24.0), rng.range(self.low, self.high), z + rng.range(-24.0, 24.0)];
            for _ in 0..rng.range(3.0, 7.0) as usize {
                let r = rng.range(5.0, 10.0) * size;
                out.push(Ball {
                    c: [c[0] + rng.range(-14.0, 14.0) * size, (c[1] + rng.range(-4.0, 4.0)).clamp(self.low, self.high), c[2] + rng.range(-14.0, 14.0) * size],
                    r: [r, r * 0.55, r],
                    kind: Kind::Cheese,
                    open: false,
                });
            }
        }
    }

    /// A winding tunnel, from `p`, as Minecraft's carver lays them: its
    /// heading and slope drift, its width swells and narrows.
    #[allow(clippy::too_many_arguments)]
    fn worm(&self, rng: &mut Rng, mut p: [f32; 3], mut yaw: f32, mut pitch: f32, length: usize, radius: f32, open: bool, branches: bool, out: &mut Vec<Ball>) {
        let (mut dyaw, mut dpitch) = (0.0f32, 0.0f32);
        let fork = if branches && length > 40 { Some(rng.range(length as f32 * 0.25, length as f32 * 0.75) as usize) } else { None };
        // An opening heads down until it is in the range.
        let mut descending = open;
        for i in 0..length {
            let t = i as f32 / length as f32;
            let r = (radius * (0.35 + 0.65 * (t * PI).sin()) * (1.0 + 0.2 * (i as f32 * 0.17).sin())).max(1.0);
            let step = (r * 0.5).max(1.0) as usize;
            if i % step == 0 {
                out.push(Ball {
                    c: p,
                    r: [r, r * 0.8, r],
                    kind: Kind::Tunnel,
                    open: descending,
                });
            }
            if Some(i) == fork {
                for side in [-1.0, 1.0] {
                    let length = ((length - i) as f32 * rng.range(0.5, 0.9)) as usize;
                    let (turn, narrow) = (rng.range(-0.4, 0.4), rng.range(0.6, 0.9));
                    self.worm(rng, p, yaw + side * PI / 2.0 + turn, pitch * 0.5, length, radius * narrow, false, false, out);
                }
            }
            let (cy, sy) = (pitch.cos(), pitch.sin());
            p = [p[0] + yaw.cos() * cy, p[1] + sy, p[2] + yaw.sin() * cy];
            if descending {
                if p[1] < self.high - r {
                    descending = false;
                }
                pitch = pitch * 0.9 + -0.8 * 0.1;
            } else {
                pitch *= 0.7;
                pitch += dpitch * 0.1;
                // Back into the range.
                if p[1] < self.low + r {
                    pitch = pitch.abs().max(0.2);
                } else if p[1] > self.high - r {
                    pitch = -pitch.abs().max(0.2);
                }
            }
            yaw += dyaw * 0.1;
            dpitch = dpitch * 0.9 + (rng.f() - rng.f()) * rng.f() * 2.0;
            dyaw = dyaw * 0.75 + (rng.f() - rng.f()) * rng.f() * 4.0;
        }
    }

    fn ravine(&self, rng: &mut Rng, x: f32, z: f32, out: &mut Vec<Ball>) {
        let size = self.s.size;
        let open = rng.f() < self.s.openings;
        let length = (rng.range(90.0, 220.0) * size.sqrt()) as usize;
        let width = rng.range(2.0, 3.5) * size;
        let span = self.high - self.low;
        let depth = (rng.range(14.0, 28.0) * size).min(span * 0.5).max(3.0);
        let mut y = rng.range(self.low + depth, (self.high - depth).max(self.low + depth));
        let (mut yaw, mut dyaw) = (rng.range(0.0, 2.0 * PI), 0.0f32);
        let mut p = [x, z];
        for i in 0..length {
            let t = i as f32 / length as f32;
            let swell = (t * PI).sin();
            let w = (width * (0.25 + 0.75 * swell)).max(1.0);
            let mut h = depth * (0.4 + 0.6 * swell.sqrt());
            let c = if open && let Some(top) = self.heights.at(p[0], p[1]) {
                // Its top above the ground, its floor down in the range.
                let floor = (top - 2.0 * h).max(self.low);
                h = ((top + 3.0 - floor) / 2.0).max(1.0);
                floor + h
            } else {
                y
            };
            out.push(Ball { c: [p[0], c, p[1]], r: [w, h, w], kind: Kind::Ravine, open });
            p = [p[0] + yaw.cos(), p[1] + yaw.sin()];
            yaw += dyaw * 0.05;
            dyaw = dyaw * 0.8 + (rng.f() - rng.f()) * 2.0;
            y = (y + rng.range(-0.3, 0.3)).clamp(self.low + h, (self.high - h).max(self.low + h));
        }
    }

    fn cavern(&self, rng: &mut Rng, x: f32, z: f32, out: &mut Vec<Ball>) {
        let size = self.s.size;
        let span = self.high - self.low;
        let ry = (rng.range(9.0, 16.0) * size).min(span * 0.45).max(3.0);
        let y = rng.range(self.low + ry * 0.6, (self.high - ry).max(self.low + ry * 0.6));
        let rx = rng.range(18.0, 34.0) * size;
        let rz = rng.range(18.0, 34.0) * size;
        out.push(Ball { c: [x, y, z], r: [rx, ry, rz], kind: Kind::Cavern, open: false });
        // Lobes around it, so that it is not an egg.
        for _ in 0..rng.range(2.0, 5.0) as usize {
            let a = rng.range(0.0, 2.0 * PI);
            let k = rng.range(0.5, 0.9);
            let r = rng.range(0.35, 0.6);
            out.push(Ball {
                c: [x + a.cos() * rx * k, y + rng.range(-0.3, 0.2) * ry, z + a.sin() * rz * k],
                r: [rx * r, ry * rng.range(0.5, 0.8), rz * r],
                kind: Kind::Cavern,
                open: false,
            });
        }
        // Tunnels lead out of it.
        if self.s.tunnels {
            for _ in 0..rng.range(1.0, 3.0) as usize {
                let a = rng.range(0.0, 2.0 * PI);
                let length = (rng.range(60.0, 140.0) * size.sqrt()) as usize;
                let (pitch, radius) = (rng.range(-0.1, 0.1), rng.range(1.5, 2.5) * size);
                self.worm(rng, [x, y - ry * 0.3, z], a, pitch, length, radius, false, false, out);
            }
        }
    }
}

// ------------------------------------------------------------- carving --

/// Carve caves into the matter of `dim`: over the whole map, or over its
/// selection when it has one, by the selection's weight. `steps` counts the
/// tiles done and can cancel. Nothing in `dim` changes: the chunks to put
/// in place come back.
pub fn carve(dim: &Dimension, s: &CaveSettings, steps: &Steps) -> Carved {
    let ground = dim.ground();
    let selection = dim.selection.clone();
    let mut area: Vec<TilePos> = match &selection {
        Some(sel) => sel.positions().filter(|p| ground.contains(*p)).collect(),
        None => ground.positions().collect(),
    };
    area.sort();
    steps.total.store(area.len().max(1), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let low = s.low.max(dim.min_y + 1);
    let high = s.high.min(dim.max_y - 1);
    if area.is_empty() || !s.carves() || high <= low {
        return Carved::default();
    }

    // The tiles whose features may reach into the area.
    let mut sources: Vec<TilePos> = {
        let mut set = std::collections::HashSet::new();
        for p in &area {
            for dz in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    let q = TilePos::new(p.x + dx, p.z + dz);
                    if ground.contains(q) {
                        set.insert(q);
                    }
                }
            }
        }
        set.into_iter().collect()
    };
    sources.sort();
    let heights = Heights::read(dim, &sources);
    let layout = Layout { s, heights: &heights, low: low as f32, high: high as f32 };
    let balls: Vec<Ball> = sources.par_iter().flat_map_iter(|&t| layout.tile(t)).collect();
    drop(heights);

    // Which balls reach each tile of the area.
    let in_area: std::collections::HashSet<TilePos> = area.iter().copied().collect();
    let mut by_tile: HashMap<TilePos, Vec<u32>> = HashMap::new();
    for (i, b) in balls.iter().enumerate() {
        let (x0, x1) = b.x_range();
        let (z0, z1) = b.z_range();
        let (t0, t1) = (TilePos::containing(x0, z0), TilePos::containing(x1, z1));
        for tz in t0.z..=t1.z {
            for tx in t0.x..=t1.x {
                let t = TilePos::new(tx, tz);
                if in_area.contains(&t) {
                    by_tile.entry(t).or_default().push(i as u32);
                }
            }
        }
    }

    let fluid_seed = s.seed as u64 ^ 0x5EED;
    let results: Vec<TileCarved> = area
        .par_iter()
        .map(|&tile| {
            let out = if steps.cancel.load(Ordering::Relaxed) {
                (Vec::new(), 0, 0)
            } else {
                match by_tile.get(&tile) {
                    Some(list) => carve_tile(dim, tile, list.iter().map(|&i| &balls[i as usize]).collect(), s, low, high, fluid_seed),
                    None => (Vec::new(), 0, 0),
                }
            };
            steps.done.fetch_add(1, Ordering::Relaxed);
            out
        })
        .collect();
    if steps.cancel.load(Ordering::Relaxed) {
        return Carved { cancelled: true, ..Carved::default() };
    }
    let mut carved = Carved::default();
    for (chunks, blocks, flooded) in results {
        carved.chunks.extend(chunks);
        carved.blocks += blocks;
        carved.flooded += flooded;
    }
    carved
}

/// The chunks carved over a tile, the blocks carved and flooded there.
type TileCarved = (Vec<(ChunkPos, Arc<VoxelChunk>)>, usize, usize);

/// Carve `balls` into the chunks over `tile`.
fn carve_tile(dim: &Dimension, tile: TilePos, balls: Vec<&Ball>, s: &CaveSettings, low: i32, high: i32, seed: u64) -> TileCarved {
    let Some(ground) = dim.ground().view(tile) else { return (Vec::new(), 0, 0) };
    let (tx0, tz0) = tile.origin();
    let tops: Vec<f32> = (0..(TILE_SIZE * TILE_SIZE) as usize).map(|i| ground.top.get(i)).collect();
    drop(ground);
    let weights = dim.selection.as_ref().map(|sel| crate::selection::tile_weights(sel.tile(tile)));
    let top_max = tops.iter().copied().fold(f32::MIN, f32::max);

    // The chunks the balls reach, inside the tile and the heights they may
    // carve.
    let per = TILE_SIZE / SIZE;
    let mut wanted: Vec<ChunkPos> = Vec::new();
    for b in &balls {
        let (x0, x1) = b.x_range();
        let (z0, z1) = b.z_range();
        let (y0, y1) = b.y_range();
        let ceiling = if b.open { top_max as i32 + 4 } else { high.min(top_max as i32) };
        let (y0, y1) = (y0.max(low), y1.min(ceiling));
        if y1 < y0 {
            continue;
        }
        let cx = (x0.max(tx0).div_euclid(SIZE), x1.min(tx0 + TILE_SIZE - 1).div_euclid(SIZE));
        let cz = (z0.max(tz0).div_euclid(SIZE), z1.min(tz0 + TILE_SIZE - 1).div_euclid(SIZE));
        for cy in y0.div_euclid(SIZE)..=y1.div_euclid(SIZE) {
            for z in cz.0..=cz.1 {
                for x in cx.0..=cx.1 {
                    wanted.push(ChunkPos::new(x, cy, z));
                }
            }
        }
    }
    wanted.sort();
    wanted.dedup();
    debug_assert!(wanted.iter().all(|p| p.tile() == tile), "{per} chunks a tile side");
    if wanted.is_empty() {
        return (Vec::new(), 0, 0);
    }

    let made = voxel::densities_of(dim, &wanted);
    let mut kept = Vec::new();
    let (mut blocks, mut flooded) = (0, 0);
    let mut target = vec![255u8; voxel::VOLUME];
    let mut lake = vec![false; voxel::VOLUME];
    for (pos, mut chunk) in made {
        let (ox, oy, oz) = pos.origin();
        target.fill(255);
        lake.fill(false);
        let mut any = false;
        for b in &balls {
            let (x0, x1) = b.x_range();
            let (z0, z1) = b.z_range();
            let (y0, y1) = b.y_range();
            let (ax, bx) = ((x0 - ox).max(0), (x1 - ox).min(SIZE - 1));
            let (ay, by) = ((y0 - oy).max(0), (y1 - oy).min(SIZE - 1));
            let (az, bz) = ((z0 - oz).max(0), (z1 - oz).min(SIZE - 1));
            if ax > bx || ay > by || az > bz {
                continue;
            }
            let rmin = b.r[0].min(b.r[1]).min(b.r[2]);
            for lz in az..=bz {
                for lx in ax..=bx {
                    let (x, z) = (ox + lx, oz + lz);
                    let top = tops[local_index(x, z)];
                    let ceiling = if b.open { top + 4.0 } else { (high as f32 + 1.0).min(top - ROOF) };
                    for ly in ay..=by {
                        let y = oy + ly;
                        if y < low || y as f32 >= ceiling {
                            continue;
                        }
                        let p = [x as f32 + 0.5 - b.c[0], y as f32 + 0.5 - b.c[1], z as f32 + 0.5 - b.c[2]];
                        let d = match b.kind {
                            Kind::Tunnel => ((p[0] / b.r[0]).powi(2) + (p[1] / b.r[1]).powi(2) + (p[2] / b.r[2]).powi(2)).sqrt(),
                            Kind::Ravine => {
                                // Steep walls, narrower toward the floor.
                                let v = (p[1] / b.r[1]).clamp(-1.5, 1.5);
                                let w = b.r[0] * (0.55 + 0.45 * (v + 1.0) * 0.5).max(0.3);
                                (((p[0] * p[0] + p[2] * p[2]) / (w * w)) + v.abs().powi(6)).sqrt()
                            }
                            Kind::Cheese => {
                                // Pillars: tall, thin columns of matter left.
                                if noise3(seed, x as f32 * 0.11, y as f32 * 0.03, z as f32 * 0.11) > 0.68 {
                                    continue;
                                }
                                ((p[0] / b.r[0]).powi(2) + (p[1] / b.r[1]).powi(2) + (p[2] / b.r[2]).powi(2)).sqrt() + (noise3(seed ^ 1, x as f32 * 0.15, y as f32 * 0.15, z as f32 * 0.15) - 0.5) * 0.4
                            }
                            Kind::Cavern => {
                                // A flatter floor, a rough outline.
                                let ry = if p[1] < 0.0 { b.r[1] * 0.6 } else { b.r[1] };
                                ((p[0] / b.r[0]).powi(2) + (p[1] / ry).powi(2) + (p[2] / b.r[2]).powi(2)).sqrt() + (noise3(seed ^ 2, x as f32 * 0.08, y as f32 * 0.1, z as f32 * 0.08) - 0.5) * 0.35
                            }
                        };
                        // The wall over a block of smooth density.
                        let outside = (d - 1.0) * rmin;
                        if outside >= 0.5 {
                            continue;
                        }
                        let t = ((outside + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
                        let i = voxel::voxel_index(lx, ly, lz);
                        if t < target[i] {
                            target[i] = t;
                            any = true;
                        }
                        if b.kind == Kind::Cavern && s.lake.is_some() && y < s.lake_level && d < 1.2 {
                            lake[i] = true;
                        }
                    }
                }
            }
        }
        if !any {
            continue;
        }
        let mut changed = false;
        for ly in 0..SIZE {
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let i = voxel::voxel_index(lx, ly, lz);
                    let t = target[i];
                    if t == 255 && !lake[i] {
                        continue;
                    }
                    let w = weights.as_ref().map_or(255, |w| w[local_index(ox + lx, oz + lz)]);
                    if w == 0 {
                        continue;
                    }
                    let old = chunk.get(lx, ly, lz);
                    let aim = old.min(t);
                    let new = (old as i32 - ((old as i32 - aim as i32) * w as i32 + 127) / 255) as u8;
                    if new != old {
                        chunk.set(lx, ly, lz, new);
                        changed = true;
                        if old >= SOLID && new < SOLID {
                            blocks += 1;
                        }
                    }
                    if lake[i] && new < SOLID && w >= crate::selection::HALF && chunk.fluid(lx, ly, lz).is_none() {
                        chunk.set_fluid(lx, ly, lz, s.lake);
                        changed = true;
                        flooded += 1;
                    }
                }
            }
        }
        if changed {
            kept.push((pos, Arc::new(chunk)));
        }
    }
    (kept, blocks, flooded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::Tile;
    use crate::world::{World, WorldMeta};

    /// A flat map of `n` × `n` tiles, its ground at 64.
    fn world(n: i32) -> World {
        let mut w = World::new(WorldMeta::default());
        for z in 0..n {
            for x in 0..n {
                w.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(64.0, 1, 63));
            }
        }
        w
    }

    fn settings() -> CaveSettings {
        CaveSettings { seed: 7, density: 1.0, openings: 0.0, low: -40, high: 40, ..CaveSettings::for_dimension(&world(1).dims[0], 62) }
    }

    fn air_blocks(chunks: &[(ChunkPos, Arc<VoxelChunk>)]) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        for (pos, c) in chunks {
            let (ox, oy, oz) = pos.origin();
            for ly in 0..SIZE {
                for lz in 0..SIZE {
                    for lx in 0..SIZE {
                        if !c.solid(lx, ly, lz) {
                            out.push((ox + lx, oy + ly, oz + lz));
                        }
                    }
                }
            }
        }
        out
    }

    /// The same seed carves the same caves; another carves others.
    #[test]
    fn carving_is_deterministic() {
        let w = world(3);
        let a = carve(&w.dims[0], &settings(), &Steps::default());
        let b = carve(&w.dims[0], &settings(), &Steps::default());
        assert!(a.blocks > 1000, "caves were carved: {}", a.blocks);
        assert_eq!(a.chunks.len(), b.chunks.len());
        for ((pa, ca), (pb, cb)) in a.chunks.iter().zip(&b.chunks) {
            assert_eq!(pa, pb);
            assert!(ca == cb);
        }
        let other = carve(&w.dims[0], &CaveSettings { seed: 8, ..settings() }, &Steps::default());
        assert_ne!(air_blocks(&a.chunks), air_blocks(&other.chunks));
    }

    /// Closed caves keep to the depth range and under a roof; openings
    /// reach the surface.
    #[test]
    fn caves_keep_to_the_depth_range() {
        let w = world(3);
        let s = CaveSettings { low: -20, high: 20, ..settings() };
        let c = carve(&w.dims[0], &s, &Steps::default());
        let air = air_blocks(&c.chunks);
        assert!(!air.is_empty());
        assert!(air.iter().all(|&(_, y, _)| (-20..=20).contains(&y)), "{:?}", air.iter().map(|a| a.1).min());
        // Under ground at 64 closed caves never reach 64 − 5.
        let s = CaveSettings { high: 100, ..settings() };
        let c = carve(&w.dims[0], &s, &Steps::default());
        assert!(air_blocks(&c.chunks).iter().all(|&(_, y, _)| (y as f32) < 64.0 - ROOF));
        // With openings, some reach the surface.
        let s = CaveSettings { openings: 1.0, ..settings() };
        let c = carve(&w.dims[0], &s, &Steps::default());
        let air = air_blocks(&c.chunks);
        assert!(air.iter().any(|&(_, y, _)| y == 63), "an opening reaches the surface");
        assert!(air.iter().all(|&(_, y, _)| y >= -40));
    }

    /// Inside a selection, the caves are those of the whole map; outside,
    /// nothing changes.
    #[test]
    fn caves_keep_to_the_selection() {
        let mut w = world(3);
        let whole = carve(&w.dims[0], &settings(), &Steps::default());
        let rect = crate::coords::BlockRect::new(100, 100, 300, 260);
        let sel = crate::selection::Selection::shape(&crate::selection::Shape::Rect(rect));
        w.dims[0].selection = Some(Arc::new(sel));
        let part = carve(&w.dims[0], &settings(), &Steps::default());
        let inside = |&(x, _, z): &(i32, i32, i32)| (100..300).contains(&x) && (100..260).contains(&z);
        let a = air_blocks(&part.chunks);
        assert!(!a.is_empty());
        assert!(a.iter().all(inside), "nothing carved outside the selection");
        let mut b: Vec<_> = air_blocks(&whole.chunks).into_iter().filter(inside).collect();
        let mut a = a;
        a.sort();
        b.sort();
        assert_eq!(a, b, "the same caves as over the whole map");
    }

    /// A cavern's lake fills it up to its level and no higher.
    #[test]
    fn lakes_fill_caverns_to_their_level() {
        let w = world(4);
        let s = CaveSettings { tunnels: false, ravines: false, lake: Some(Fluid::Lava), lake_level: -10, ..settings() };
        let c = carve(&w.dims[0], &s, &Steps::default());
        assert!(c.flooded > 0, "a lake");
        let mut lava = 0;
        for (pos, chunk) in &c.chunks {
            let (_, oy, _) = pos.origin();
            for ly in 0..SIZE {
                for lz in 0..SIZE {
                    for lx in 0..SIZE {
                        if let Some(f) = chunk.fluid(lx, ly, lz) {
                            assert_eq!(f, Fluid::Lava);
                            assert!(oy + ly < -10);
                            lava += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(lava, c.flooded);
    }

    /// A cancelled run gives nothing.
    #[test]
    fn cancelling_gives_nothing() {
        let w = world(2);
        let steps = Steps::default();
        steps.cancel.store(true, Ordering::Relaxed);
        let c = carve(&w.dims[0], &settings(), &steps);
        assert!(c.cancelled && c.chunks.is_empty());
    }
}
