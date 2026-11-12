//! Relief from tectonic plates.
//!
//! The map is shared between plates, each with a centre and a motion;
//! continental plates stand above the sea, oceanic ones under it. Where
//! two meet, their motions decide the relief along their boundary: plates
//! running into each other raise a range of mountains (an arc of islands
//! between two oceans, a coastal range over a trench where an ocean dives
//! under a continent), plates pulling apart open a rift valley or a ridge
//! under the sea, plates sliding past each other leave a fault. Boundaries
//! are warped by noise so that coasts and ranges wind as real ones do.

use super::{Extent, Field};
use crate::compose::hash2;
use crate::noise::Noise;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// A plate: where its centre is (fractions of the terrain), how it moves,
/// and whether it carries a continent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Plate {
    pub x: f32,
    pub z: f32,
    /// Its motion, −1 to 1 on each axis (east, south).
    pub dx: f32,
    pub dz: f32,
    pub continental: bool,
}

/// `count` plates scattered over the terrain from `seed`, about four in
/// ten of them continents.
pub fn scatter(count: usize, seed: u32) -> Vec<Plate> {
    let r = |k: usize, salt: u64| hash2(k as i32, seed as i32, 0x0091_a7e5 ^ salt) as f32 / u32::MAX as f32;
    // Jittered over a grid, so that no two plates sit on each other.
    let side = (count as f32).sqrt().ceil() as usize;
    let mut cells: Vec<usize> = (0..side * side).collect();
    for k in (1..cells.len()).rev() {
        let j = (r(k, 7) * (k + 1) as f32) as usize % (k + 1);
        cells.swap(k, j);
    }
    cells
        .into_iter()
        .take(count)
        .enumerate()
        .map(|(k, cell)| {
            let (ci, cj) = ((cell % side) as f32, (cell / side) as f32);
            let angle = r(k, 3) * std::f32::consts::TAU;
            let speed = 0.4 + 0.6 * r(k, 4);
            Plate {
                x: (ci + 0.2 + 0.6 * r(k, 1)) / side as f32,
                z: (cj + 0.2 + 0.6 * r(k, 2)) / side as f32,
                dx: angle.cos() * speed,
                dz: angle.sin() * speed,
                continental: r(k, 5) < 0.42 || k == 0,
            }
        })
        .collect()
}

/// The most samples a side plates are worked out at: their relief is
/// large; detail comes from the nodes after.
const PLATES_MAX: usize = 2048;

/// What [`relief`] makes.
pub struct TectonicMaps {
    pub height: Field,
    /// Where plates running into each other raise mountains, 0 to 1.
    pub ranges: Field,
    /// How near a boundary between plates, 1 on it.
    pub faults: Field,
}

/// The relief of `plates` over the terrain: continents `relief` × 0.15
/// over `sea`, oceans `relief` × 0.3 under it, mountains up to `relief`
/// high where plates collide, along belts `width` blocks wide; boundaries
/// warped by `roughness` (0..1).
#[allow(clippy::too_many_arguments)]
pub fn relief(plates: &[Plate], e: &Extent, relief: f32, width: f32, roughness: f32, sea: f32, seed: u32) -> TectonicMaps {
    let n = e.n;
    let m = n.min(PLATES_MAX);
    let flat = |v: f32| Field::new(n, vec![v; n * n]);
    if plates.is_empty() {
        return TectonicMaps {
            height: flat(sea + 0.15 * relief),
            ranges: flat(0.0),
            faults: flat(0.0),
        };
    }
    let warp = Noise::new(seed as u64 ^ 0x7ec7);
    let warp_z = warp.derive(1);
    let detail = Noise::new(seed as u64 ^ 0x51de);
    let coast = Noise::new(seed as u64 ^ 0xc0a5);
    let size = e.size as f32;
    let width = width.max(8.0);
    let wobble = roughness.clamp(0.0, 1.0) * 0.18 * size;
    let centres: Vec<(f32, f32)> = plates.iter().map(|p| (p.x * size, p.z * size)).collect();
    let land = sea + 0.15 * relief;
    let ocean = sea - 0.3 * relief;
    let cell = (e.size / m as f64) as f32;
    let samples: Vec<(f32, f32, f32)> = (0..m * m)
        .into_par_iter()
        .map(|k| {
            let (i, j) = (k % m, k / m);
            let (u, v) = ((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
            // Boundaries that wind.
            let (wx, wz) = ((e.x0 as f32 + u) as f64 / (size as f64 * 0.22), (e.z0 as f32 + v) as f64 / (size as f64 * 0.22));
            let (px, pz) = (u + wobble * warp.fbm(wx, wz, 4, 2.0, 0.5) as f32, v + wobble * warp_z.fbm(wx, wz, 4, 2.0, 0.5) as f32);
            // The nearest plate and the next.
            let (mut a, mut b) = (0usize, usize::MAX);
            let (mut da, mut db) = (f32::MAX, f32::MAX);
            for (k, c) in centres.iter().enumerate() {
                let d = (px - c.0).powi(2) + (pz - c.1).powi(2);
                if d < da {
                    (b, db) = (a, da);
                    (a, da) = (k, d);
                } else if d < db {
                    (b, db) = (k, d);
                }
            }
            let pa = &plates[a];
            if b == usize::MAX {
                return (if pa.continental { land } else { ocean }, 0.0, 0.0);
            }
            let pb = &plates[b];
            let (ca, cb) = (centres[a], centres[b]);
            let (nx, nz) = (cb.0 - ca.0, cb.1 - ca.1);
            let len = (nx * nx + nz * nz).sqrt().max(1e-3);
            // Distance to the line between the two plates.
            let d = ((db - da) / (2.0 * len)).max(0.0);
            let (nx, nz) = (nx / len, nz / len);
            let (rvx, rvz) = (pa.dx - pb.dx, pa.dz - pb.dz);
            let closing = rvx * nx + rvz * nz;
            let sliding = (rvx * nz - rvz * nx).abs();
            let near = (-(d / width).powi(2)).exp();
            let wide = (-(d / (width * 2.2)).powi(2)).exp();
            // How much of a continent stands here: all of it inland and
            // where continents meet; toward an ocean, a coast that winds,
            // capes and bays, islands off it. Measured from the nearest
            // ocean's boundary, so that it never jumps.
            let ragged = coast.fbm(wx * 2.4, wz * 2.4, 5, 2.0, 0.55) as f32;
            let continent = |k: usize| {
                if !plates[k].continental {
                    return 0.0;
                }
                let dk = (px - centres[k].0).powi(2) + (pz - centres[k].1).powi(2);
                let mut to_ocean = f32::MAX;
                for (o, c) in centres.iter().enumerate() {
                    if plates[o].continental {
                        continue;
                    }
                    let gap = ((c.0 - centres[k].0).powi(2) + (c.1 - centres[k].1).powi(2)).sqrt().max(1e-3);
                    let d = (((px - c.0).powi(2) + (pz - c.1).powi(2) - dk) / (2.0 * gap)).max(0.0);
                    to_ocean = to_ocean.min(d);
                }
                if to_ocean == f32::MAX {
                    1.0
                } else {
                    super::ops::smoothstep(-0.18, 0.18, (to_ocean / (2.4 * width)).min(2.0) - 0.5 + 0.6 * ragged)
                }
            };
            let base_a = ocean + (land - ocean) * continent(a);
            // The two plates' grounds meet half way across the boundary.
            let base_b = ocean + (land - ocean) * continent(b);
            let shore = 0.5 * (1.0 - (d / (width * 0.9)).min(1.0)).powi(2);
            let mut h = base_a + (base_b - base_a) * shore;
            let mut range = 0.0;
            // Ranges of ridged peaks, not walls.
            let s = width * 1.4;
            let craggy = 0.45 + 0.55 * detail.ridged(((e.x0 as f32 + px) / s) as f64, ((e.z0 as f32 + pz) / s) as f64, 5, 2.0, 0.5) as f32;
            if closing > 0.0 {
                let push = closing.min(1.6);
                match (pa.continental, pb.continental) {
                    // Two continents: the highest ranges, on both sides.
                    (true, true) => range = push * near * craggy,
                    // An ocean diving under a continent: a range on the
                    // land, set back from the coast, a trench off it.
                    (true, false) => range = push * (-(((d - width * 0.8) / width).powi(2))).exp() * craggy,
                    (false, true) => h -= 0.3 * relief * push.min(1.0) * near,
                    // Two oceans: an arc of volcanic islands.
                    (false, false) => range = 0.62 * push * near * craggy,
                }
                h += relief * range;
            } else {
                let pull = (-closing).min(1.6);
                if pa.continental && pb.continental {
                    // A rift valley, its shoulders raised.
                    h += relief * pull * (0.12 * wide - 0.32 * near);
                } else {
                    // A ridge along the sea floor.
                    h += relief * pull * 0.2 * near;
                }
            }
            // A fault where the plates slide past each other.
            h -= relief * 0.06 * sliding.min(1.5) * near * (d / width * 3.0).cos().max(0.0);
            (h, range.clamp(0.0, 1.0), near)
        })
        .collect();
    let take = |k: usize| Field::new(m, samples.iter().map(|s| [s.0, s.1, s.2][k]).collect());
    TectonicMaps {
        height: super::ops::upsample(&take(0), n),
        ranges: super::ops::upsample(&take(1), n),
        faults: super::ops::upsample(&take(2), n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(n: usize) -> Extent {
        Extent {
            x0: 0.0,
            z0: 0.0,
            size: 4096.0,
            n,
        }
    }

    #[test]
    fn colliding_continents_raise_mountains() {
        let plates = [
            Plate {
                x: 0.3,
                z: 0.5,
                dx: 1.0,
                dz: 0.0,
                continental: true,
            },
            Plate {
                x: 0.7,
                z: 0.5,
                dx: -1.0,
                dz: 0.0,
                continental: true,
            },
        ];
        let r = relief(&plates, &ext(128), 160.0, 200.0, 0.0, 63.0, 1);
        let middle = r.height.at(64, 64);
        let inland = r.height.at(12, 64);
        assert!(middle > inland + 80.0, "a range where they meet: {middle} against {inland}");
        assert!(inland > 63.0, "continents stand over the sea: {inland}");
        assert!(r.ranges.at(64, 64) > 0.5 && r.faults.at(64, 64) > 0.9);
        // Pulled apart, a rift instead.
        let apart = [Plate { dx: -1.0, ..plates[0] }, Plate { dx: 1.0, ..plates[1] }];
        let r = relief(&apart, &ext(128), 160.0, 200.0, 0.0, 63.0, 1);
        assert!(r.height.at(64, 64) < r.height.at(12, 64) - 20.0, "a rift: {}", r.height.at(64, 64));
    }

    #[test]
    fn oceans_lie_under_the_sea() {
        let plates = scatter(9, 4);
        assert_eq!(plates.len(), 9);
        assert!(plates.iter().any(|p| p.continental) && plates.iter().any(|p| !p.continental));
        let r = relief(&plates, &ext(96), 160.0, 220.0, 0.6, 63.0, 2);
        let under = r.height.data.iter().filter(|&&h| h < 63.0).count() as f32 / r.height.data.len() as f32;
        assert!(under > 0.15 && under < 0.85, "sea and land both: {under}");
    }
}
