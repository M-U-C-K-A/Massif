//! Aquifers: the rock cut into zones some tens of blocks across, the
//! centres of a jittered grid, each dry or holding water — deep down, lava —
//! up to a level of its own. A cave's block takes the fluid of the zone it
//! lies in, under that zone's level. Where a zone meets another whose fluid
//! differs at that height, the cave is not carved: a wall of rock stays, so
//! that no lake runs into a dry cave or onto lava, as the game's barriers.
//! Distances are measured in cells of the grid, the centres jittered
//! within the middle third of their cell: the nearest zone to a block is
//! then always one of the 27 around its cell, and so is every zone closer
//! than [`WALL`] past it. Two blocks side by side, a cell's 1/24 apart at
//! most, are in zones whose distances differ by under 2/24 at both, so a
//! wall that thick seals every pair.

use super::Aquifers;
use super::noise::hash3;
use super::tile::Fill;

/// A zone: its centre (in cells), and what it holds under its level.
#[derive(Debug, Clone, Copy)]
struct Zone {
    c: [f32; 3],
    /// Its fluid fills the caves under this height; `i32::MIN`: dry.
    level: i32,
    fluid: Fill,
}

/// The zones around a box of the world.
pub struct Zones {
    floor: Option<i32>,
    cell: [i32; 3],
    /// The grid cell of `zones[0]`, and the cells across.
    g0: [i32; 3],
    n: [i32; 3],
    zones: Vec<Zone>,
}

/// How much farther than its own zone, in cells, another zone may be for
/// a block to be a wall when their fluids differ: over 2/24 (two blocks
/// side by side), under 1.32 − √3 × 0.68 (the nearest zone two cells off).
const WALL: f32 = 0.1;
/// The centres lie in the middle of their cell, this far from its sides.
const JITTER: f32 = 0.32;

impl Zones {
    /// The zones of `aq` (seeded by `seed`) around blocks `x0..x1`,
    /// `y0..y1`, `z0..z1`.
    pub fn new(seed: u32, aq: &Aquifers, (x0, x1): (i32, i32), (y0, y1): (i32, i32), (z0, z1): (i32, i32)) -> Self {
        let size = aq.size.clamp(0.5, 2.0);
        // No side under 24 blocks, so that two blocks side by side are
        // 1/24 of a cell apart at most (see [`WALL`]).
        let cell = [((48.0 * size) as i32).max(24), ((24.0 * size) as i32).max(24), ((48.0 * size) as i32).max(24)];
        let g0 = [x0.div_euclid(cell[0]) - 1, y0.div_euclid(cell[1]) - 1, z0.div_euclid(cell[2]) - 1];
        let g1 = [(x1 - 1).div_euclid(cell[0]) + 1, (y1 - 1).div_euclid(cell[1]) + 1, (z1 - 1).div_euclid(cell[2]) + 1];
        let n = [g1[0] - g0[0] + 1, g1[1] - g0[1] + 1, g1[2] - g0[2] + 1];
        let mut zones = Vec::with_capacity((n[0] * n[1] * n[2]) as usize);
        for gz in g0[2]..=g1[2] {
            for gy in g0[1]..=g1[1] {
                for gx in g0[0]..=g1[0] {
                    zones.push(zone(seed, aq, cell, gx, gy, gz));
                }
            }
        }
        Self { floor: aq.on.then_some(aq.lava_floor).flatten(), cell, g0, n, zones }
    }

    #[inline]
    fn at(&self, gx: i32, gy: i32, gz: i32) -> &Zone {
        let (i, j, k) = (gx - self.g0[0], gy - self.g0[1], gz - self.g0[2]);
        &self.zones[((k * self.n[1] + j) * self.n[0] + i) as usize]
    }

    #[inline]
    fn fill_of(&self, zone: &Zone, y: i32) -> Fill {
        match self.floor {
            Some(f) if y < f => Fill::Lava,
            _ if y < zone.level => zone.fluid,
            _ => Fill::Air,
        }
    }

    /// What a cave holds at block `(x, y, z)`: air, water or lava, or
    /// `None` where it must stay rock, a wall between zones.
    pub fn fill(&self, x: i32, y: i32, z: i32) -> Option<Fill> {
        let (gx, gy, gz) = (x.div_euclid(self.cell[0]), y.div_euclid(self.cell[1]), z.div_euclid(self.cell[2]));
        let c = [self.cell[0] as f32, self.cell[1] as f32, self.cell[2] as f32];
        let p = [(x as f32 + 0.5) / c[0], (y as f32 + 0.5) / c[1], (z as f32 + 0.5) / c[2]];
        let mut near: [(f32, &Zone); 27] = [(f32::MAX, &self.zones[0]); 27];
        let mut k = 0;
        let mut best = (f32::MAX, 0usize);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let zone = self.at(gx + dx, gy + dy, gz + dz);
                    let d = (zone.c[0] - p[0]).powi(2) + (zone.c[1] - p[1]).powi(2) + (zone.c[2] - p[2]).powi(2);
                    near[k] = (d, zone);
                    if d < best.0 {
                        best = (d, k);
                    }
                    k += 1;
                }
            }
        }
        let own = near[best.1].1;
        let fill = self.fill_of(own, y);
        // Water right over the lava floor would meet it.
        if fill == Fill::Water && self.floor == Some(y) {
            return None;
        }
        let reach = (best.0.sqrt() + WALL).powi(2);
        for (i, &(d, zone)) in near.iter().enumerate() {
            if i != best.1 && d < reach && self.fill_of(zone, y) != fill {
                return None;
            }
        }
        Some(fill)
    }
}

fn zone(seed: u32, aq: &Aquifers, cell: [i32; 3], gx: i32, gy: i32, gz: i32) -> Zone {
    let h = hash3(seed as u64 ^ 0xA9F1_FE45, gx, gy, gz);
    let unit = |s: u32| ((h >> s) & 0xFF) as f32 / 255.0;
    let spread = 1.0 - 2.0 * JITTER;
    let c = [gx as f32 + JITTER + spread * unit(0), gy as f32 + JITTER + spread * unit(8), gz as f32 + JITTER + spread * unit(16)];
    let dry = Zone { c, level: i32::MIN, fluid: Fill::Air };
    // The height of its centre, in blocks.
    let y = c[1] * cell[1] as f32;
    if !aq.on || unit(24) >= aq.density || y < aq.low as f32 - cell[1] as f32 * 0.5 || y > aq.high as f32 {
        return dry;
    }
    let level = ((y + (unit(32) - 0.5) * cell[1] as f32 * 0.8) as i32).min(aq.high + 1);
    let fluid = if y < aq.lava_under as f32 && unit(40) < aq.lava { Fill::Lava } else { Fill::Water };
    Zone { c, level, fluid }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aq() -> Aquifers {
        Aquifers { on: true, density: 0.6, size: 1.0, low: -64, high: 60, lava: 0.4, lava_under: 0, lava_floor: Some(-54) }
    }

    /// No two blocks side by side hold different fluids, nor water over
    /// air or lava: every lake is sealed by rock where it is not open to
    /// the air above it.
    #[test]
    fn zones_are_sealed() {
        let zones = Zones::new(5, &aq(), (0, 160), (-64, 64), (0, 160));
        let at = |x: i32, y: i32, z: i32| zones.fill(x, y, z);
        let (mut wet, mut walls) = (0, 0);
        for z in 1..159 {
            for y in -63..63 {
                for x in 1..159 {
                    let Some(f) = at(x, y, z) else {
                        walls += 1;
                        continue;
                    };
                    if f == Fill::Air {
                        continue;
                    }
                    wet += 1;
                    // Beside it and under it, rock or the same fluid.
                    for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
                        if let Some(n) = at(x + dx, y + dy, z + dz) {
                            assert_eq!(n, f, "{f:?} at {x} {y} {z} beside {n:?}");
                        }
                    }
                }
            }
        }
        assert!(wet > 10_000 && walls > 100, "{wet} wet, {walls} walls");
        // Under the floor, lava.
        assert_eq!(at(10, -60, 10), Some(Fill::Lava));
    }
}
