//! Patterns to paint the ground with: several terrains laid out by a noise
//! (patches, veins, blobs, cells or stripes), each over the share of the
//! ground its weight says, with soft edges.
//!
//! Noise values are not spread evenly (most of a fractal noise lies near
//! its middle), so the pattern first learns its own spread from a few
//! thousand samples and paints by rank: a terrain weighted a quarter covers
//! a quarter of the ground, whatever the pattern.

use crate::compose::hash2;
use crate::noise::Noise;
use crate::registry::TerrainId;
use serde::{Deserialize, Serialize};

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
}

/// A pattern of terrains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    pub kind: PatternKind,
    /// Size of its features, in blocks.
    pub scale: f32,
    /// Direction of the stripes, in degrees.
    pub angle: f32,
    /// How soft the edges between terrains are (0: sharp, 1: very ragged).
    pub softness: f32,
    pub seed: u32,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            kind: PatternKind::Patches,
            scale: 24.0,
            angle: 30.0,
            softness: 0.3,
            seed: 1,
        }
    }
}

impl Pattern {
    /// The pattern's raw value at a point of the world.
    fn raw(&self, noise: &Noise, x: f64, z: f64) -> f64 {
        let s = self.scale.max(1.0) as f64;
        match self.kind {
            PatternKind::Patches => noise.fbm(x / s, z / s, 5, 2.0, 0.5),
            PatternKind::Veins => -noise.ridged(x / s, z / s, 4, 2.0, 0.55),
            PatternKind::Blobs => noise.fbm(x / s, z / s, 4, 2.0, 0.45).abs(),
            PatternKind::Cells => {
                let (fx, fz) = (x / s, z / s);
                let (cx, cz) = (fx.floor() as i32, fz.floor() as i32);
                let mut best = (f64::MAX, 0u32);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let (gx, gz) = (cx + dx, cz + dz);
                        let h = hash2(gx, gz, self.seed as u64 ^ 0xce11_5eed);
                        let ox = (h & 0xffff) as f64 / 65535.0;
                        let oz = (h >> 16) as f64 / 65535.0;
                        let d = (gx as f64 + ox - fx).hypot(gz as f64 + oz - fz);
                        if d < best.0 {
                            best = (d, hash2(gx, gz, self.seed as u64 ^ 0x0c0f_fee5));
                        }
                    }
                }
                best.1 as f64
            }
            PatternKind::Stripes => {
                let (sa, ca) = (self.angle as f64).to_radians().sin_cos();
                let across = x * ca + z * sa;
                let waver = noise.fbm(x / (s * 4.0), z / (s * 4.0), 3, 2.0, 0.5) * s * 1.2;
                let t = (across + waver) / (s * 2.0);
                // A triangle wave: every value as often as any other.
                (t - t.floor() - 0.5).abs()
            }
        }
    }

    /// Ready to paint `terrains` (each with its weight).
    pub fn sampler(&self, terrains: &[(TerrainId, f32)]) -> PatternSampler {
        let noise = Noise::new(self.seed as u64 ^ 0x9a77_e2a5);
        // The pattern's own spread, from samples far apart.
        let span = self.scale.max(1.0) as f64 * 40.0;
        let mut quantiles: Vec<f64> = (0..4096)
            .map(|k| {
                let (u, v) = (hash2(k, 1, 0x5a3e) as f64 / u32::MAX as f64, hash2(k, 2, 0x5a3e) as f64 / u32::MAX as f64);
                self.raw(&noise, u * span, v * span)
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
            noise,
            quantiles,
            cumulative,
        }
    }
}

/// A pattern with its spread learnt and its terrains placed.
pub struct PatternSampler {
    pattern: Pattern,
    noise: Noise,
    quantiles: Vec<f64>,
    /// Each terrain, with the share of the ground up to and including it.
    cumulative: Vec<(TerrainId, f32)>,
}

impl PatternSampler {
    /// Where column `(x, z)` stands in the pattern's spread, 0 to 1.
    pub fn rank(&self, x: i32, z: i32) -> f32 {
        let v = self.pattern.raw(&self.noise, x as f64 + 0.5, z as f64 + 0.5);
        let k = self.quantiles.partition_point(|q| *q < v);
        k as f32 / self.quantiles.len() as f32
    }

    /// The terrain of column `(x, z)`.
    pub fn terrain(&self, x: i32, z: i32) -> Option<TerrainId> {
        let soft = self.pattern.softness.clamp(0.0, 1.0) * 0.12;
        let jitter = (hash2(x, z, self.pattern.seed as u64 ^ 0x5017) as f32 / u32::MAX as f32 - 0.5) * soft;
        let r = (self.rank(x, z) + jitter).clamp(0.0, 0.9999);
        self.cumulative.iter().find(|(_, c)| r < *c).or(self.cumulative.last()).map(|t| t.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_gives_each_terrain_its_share() {
        let (a, b, c) = (TerrainId(1), TerrainId(2), TerrainId(3));
        for kind in PatternKind::ALL {
            let p = Pattern {
                kind,
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
            assert!((share(1) - 0.5).abs() < 0.12 && (share(2) - 0.25).abs() < 0.1 && (share(3) - 0.25).abs() < 0.1, "{kind:?}: {counts:?}");
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
}
