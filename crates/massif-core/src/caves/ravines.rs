//! Ravines: long, deep, narrow cuts, laid out as chains of tall ellipses
//! the way the game's canyon carver lays them — the heading drifts, the
//! width swells and narrows, the walls are steep and close in toward the
//! floor. Each tile seeds its own from the layer's seed alone, so that a
//! ravine is the same whatever part of the map is composed.

use super::Feature;
use crate::coords::{TILE_SIZE, TilePos};
use std::f32::consts::PI;

/// How far a ravine reaches from the tile that seeds it, in tiles.
const REACH: i32 = 3;

/// One piece of a ravine: an upright ellipse of radii `r` at `c`.
#[derive(Debug, Clone, Copy)]
pub struct Ball {
    pub c: [f32; 3],
    /// Half its width and half its height.
    pub r: [f32; 2],
    /// Its depth under the ground's surface, for one open to the sky: it
    /// follows the surface of each column rather than a height.
    pub open: Option<f32>,
}

impl Ball {
    pub fn x_range(&self) -> (i32, i32) {
        ((self.c[0] - self.r[0]).floor() as i32 - 1, (self.c[0] + self.r[0]).ceil() as i32 + 1)
    }
    pub fn z_range(&self) -> (i32, i32) {
        ((self.c[2] - self.r[0]).floor() as i32 - 1, (self.c[2] + self.r[0]).ceil() as i32 + 1)
    }

    /// Whether block `(x, y, z)` lies inside, the ground's surface of its
    /// column being at `top`: under 1, in.
    #[inline]
    pub fn reach(&self, x: i32, y: i32, z: i32, top: i32) -> f32 {
        let (cy, h) = match self.open {
            // Its floor `depth` under the surface, its top over it.
            Some(depth) => {
                let floor = top as f32 - depth;
                let h = (depth + 3.0) / 2.0;
                (floor + h, h)
            }
            None => (self.c[1], self.r[1]),
        };
        let (dx, dy, dz) = (x as f32 + 0.5 - self.c[0], y as f32 + 0.5 - cy, z as f32 + 0.5 - self.c[2]);
        // Steep walls, narrower toward the floor.
        let v = (dy / h).clamp(-1.5, 1.5);
        let w = self.r[0] * (0.55 + 0.225 * (v + 1.0)).max(0.3);
        (((dx * dx + dz * dz) / (w * w)) + v.abs().powi(6)).sqrt()
    }
}

/// A small, fast generator (SplitMix64): the same numbers on every machine.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// In `[0, 1)`.
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
    /// How many of something expected `mean` times.
    pub fn count(&mut self, mean: f32) -> usize {
        let whole = mean.floor();
        whole as usize + usize::from(self.f() < mean - whole)
    }
}

/// The ravines of a layer of seed `seed` that reach into `tile`.
pub fn reaching(seed: u32, f: &Feature, openings: f32, tile: TilePos) -> Vec<Ball> {
    let mut out = Vec::new();
    if !f.carves() {
        return out;
    }
    let (x0, z0) = tile.origin();
    let (x1, z1) = (x0 + TILE_SIZE, z0 + TILE_SIZE);
    for dz in -REACH..=REACH {
        for dx in -REACH..=REACH {
            let source = TilePos::new(tile.x + dx, tile.z + dz);
            for b in seeded(seed, f, openings, source) {
                let ((bx0, bx1), (bz0, bz1)) = (b.x_range(), b.z_range());
                if bx1 >= x0 && bx0 < x1 && bz1 >= z0 && bz0 < z1 {
                    out.push(b);
                }
            }
        }
    }
    out
}

/// The ravines tile `source` seeds.
fn seeded(seed: u32, f: &Feature, openings: f32, source: TilePos) -> Vec<Ball> {
    let mut rng = Rng::new(super::noise::hash3(seed as u64 ^ 0x0BA7_1AE5, source.x, 0, source.z));
    let mut out = Vec::new();
    let (x0, z0) = source.origin();
    for _ in 0..rng.count(f.density) {
        let (x, z) = (x0 as f32 + rng.range(0.0, TILE_SIZE as f32), z0 as f32 + rng.range(0.0, TILE_SIZE as f32));
        ravine(&mut rng, f, openings, x, z, &mut out);
    }
    out
}

fn ravine(rng: &mut Rng, f: &Feature, openings: f32, x: f32, z: f32, out: &mut Vec<Ball>) {
    let size = f.size.clamp(0.5, 2.0);
    let open = rng.f() < openings;
    let length = (rng.range(80.0, 200.0) * size.sqrt()) as usize;
    let width = rng.range(2.0, 3.5) * size;
    let (low, high) = (f.low as f32, f.high as f32);
    let span = (high - low).max(4.0);
    let depth = (rng.range(14.0, 28.0) * size).min(span * 0.5).max(3.0);
    let mut y = rng.range(low + depth, (high - depth).max(low + depth));
    let (mut yaw, mut dyaw) = (rng.range(0.0, 2.0 * PI), 0.0f32);
    let mut p = [x, z];
    for i in 0..length {
        let t = i as f32 / length as f32;
        let swell = (t * PI).sin();
        let w = (width * (0.25 + 0.75 * swell)).max(1.0);
        let h = depth * (0.4 + 0.6 * swell.sqrt());
        out.push(Ball {
            c: [p[0], y, p[1]],
            r: [w, h],
            open: open.then_some(2.0 * h),
        });
        p = [p[0] + yaw.cos(), p[1] + yaw.sin()];
        yaw += dyaw * 0.05;
        dyaw = dyaw * 0.8 + (rng.f() - rng.f()) * 2.0;
        y = (y + rng.range(-0.3, 0.3)).clamp(low + h, (high - h).max(low + h));
    }
}
