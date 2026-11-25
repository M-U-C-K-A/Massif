//! Stage 1, the site: where the squares of the districts are, and how far
//! the village reaches round them.

use super::{Area, Params, dist2, height};
use crate::ground::Ground;
use serde::{Deserialize, Serialize};

/// The squares of a village's districts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Site {
    /// The middle of each district's square, the first the village's.
    pub centres: Vec<[i32; 2]>,
    /// How far from its square a district reaches, in blocks: the houses
    /// asked for, at the density asked for, or the area's.
    pub reach: i32,
    /// The middle of the area.
    pub middle: [i32; 2],
    /// The area's columns every 8 blocks (for the walls' hull and the
    /// roads' ends).
    pub samples: Vec<[i32; 2]>,
}

/// How steep the land is round `(x, z)`, in blocks over 8, and whether it
/// is wet.
fn roughness(ground: &dyn Ground, x: i32, z: i32) -> (i32, bool) {
    let (h, wet) = height(ground, x, z);
    let mut worst = 0;
    for [dx, dz] in [[8, 0], [-8, 0], [0, 8], [0, -8]] {
        worst = worst.max((height(ground, x + dx, z + dz).0 - h).abs());
    }
    (worst, wet)
}

/// Stage 1: the squares, each in the middle of its district, on flat dry
/// land as far as `params.flat` asks.
pub fn site(ground: &dyn Ground, area: &Area, params: &Params, _seed: u64) -> Result<Site, String> {
    let samples = area.samples(8);
    if samples.len() < 9 {
        return Err("the selection is too small for a village: 24 × 24 blocks at least".into());
    }
    let area_blocks = samples.len() as i64 * 64;
    // How far a district reaches: its houses, each a plot of about 14 + its
    // margins, on both sides of roads — or the area.
    let density = params.density.clamp(0.2, 1.0);
    let reach = match params.houses {
        Some(n) => {
            let per = (16.0 / density.sqrt()).powi(2) * (1.0 + params.spread.clamp(0.0, 1.0));
            ((n.clamp(3, 120) as f32 * per / std::f32::consts::PI).sqrt() as i32).clamp(24, 400)
        }
        None => 120,
    };
    // Districts: one for about 200 × 200 blocks of the area, or as many as
    // the houses asked for need (40 to a district).
    let by_area = (area_blocks / (200 * 200)).clamp(1, 48) as usize;
    let districts = match params.houses {
        Some(n) => (n as usize).div_ceil(40).clamp(1, by_area.max(1)),
        None => by_area,
    };
    let mean = samples.iter().fold((0i64, 0i64), |a, p| (a.0 + p[0] as i64, a.1 + p[1] as i64));
    let middle = [(mean.0 / samples.len() as i64) as i32, (mean.1 / samples.len() as i64) as i32];
    // Each sample's fitness for a square: near the middle, flat, dry.
    let flat = params.flat.clamp(0.0, 1.0);
    let span = ((area.rect[2] - area.rect[0]).max(area.rect[3] - area.rect[1]) as f32).max(1.0);
    let rough: Vec<(i32, bool)> = samples.iter().map(|&[x, z]| roughness(ground, x, z)).collect();
    let cost = |k: usize, near: [i32; 2]| -> f32 {
        let (r, wet) = rough[k];
        let d = (dist2(samples[k], near) as f32).sqrt() / span;
        d + flat * (r as f32 / 6.0).min(2.0) + if wet { 4.0 } else { 0.0 }
    };
    let best_near = |near: [i32; 2], within: f32| -> [i32; 2] {
        samples
            .iter()
            .enumerate()
            .filter(|(_, p)| (dist2(**p, near) as f32) <= within * within)
            .min_by(|a, b| cost(a.0, near).total_cmp(&cost(b.0, near)))
            .map_or(near, |(_, p)| *p)
    };
    let first = best_near(middle, span * 0.25);
    let mut centres = vec![first];
    while centres.len() < districts {
        let far = *samples.iter().max_by_key(|p| centres.iter().map(|c| dist2(**p, *c)).min().unwrap_or(0)).expect("samples");
        centres.push(far);
    }
    // Each in the middle of its district (a few rounds of k-means), on the
    // flattest dry column near there.
    if centres.len() > 1 {
        for _ in 0..6 {
            let mut sums = vec![(0i64, 0i64, 0i64); centres.len()];
            for p in &samples {
                let k = centres.iter().enumerate().min_by_key(|(_, c)| dist2(*p, **c)).map_or(0, |(k, _)| k);
                sums[k] = (sums[k].0 + p[0] as i64, sums[k].1 + p[1] as i64, sums[k].2 + 1);
            }
            for (c, s) in centres.iter_mut().zip(&sums) {
                if s.2 > 0 {
                    let m = [(s.0 / s.2) as i32, (s.1 / s.2) as i32];
                    *c = *samples.iter().min_by_key(|p| dist2(**p, m)).expect("samples");
                }
            }
        }
        for c in centres.iter_mut() {
            *c = best_near(*c, 40.0);
        }
    }
    Ok(Site { centres, reach, middle, samples })
}
