//! Rock above the ground: floating islands and the rims of overhanging
//! cliffs, each given as a top and a bottom field. Where there is no rock,
//! the bottom equals the top, so that both stay continuous at the edges.

use super::ops::smoothstep;
use super::{Extent, Field};
use crate::compose::hash2;
use crate::noise::Noise;
use rayon::prelude::*;

/// A random number in 0..1 for cell `(a, b)`, `k`-th of its kind.
#[inline]
fn rand(seed: u32, a: i32, b: i32, k: u64) -> f64 {
    hash2(a, b, (seed as u64 ^ 0xf10a_7000) + k * 0x9e37_79b9) as f64 / u32::MAX as f64
}

/// One floating island: where it is, how big, how high, how tall a tower
/// of rock it is, and where its underside hangs lowest.
#[derive(Clone, Copy)]
struct Island {
    cell: (i32, i32),
    centre: (f64, f64),
    radius: f64,
    altitude: f64,
    tower: f64,
    lowest: (f64, f64),
}

/// Floating islands over the map: their top, their bottom and their mask.
///
/// Islands sit on a jittered grid of cells about two and a half radii
/// wide, more of them where a slow noise says (they come in flocks), many
/// small and a few large. Each has a rolling top on sheer sides, and an
/// underside that hangs down to a ribbed point, deepest under its middle.
pub(super) fn floating_islands(e: &Extent, altitude: f32, spread: f32, size: f32, coverage: f32, depth: f32, seed: u32) -> (Field, Field, Field) {
    let n = e.n;
    let noise = Noise::new(seed as u64 ^ 0xf10a);
    let flock = noise.derive(1);
    let relief = noise.derive(2);
    let ribs = noise.derive(3);
    let r_max = size.max(4.0) as f64;
    let cell = r_max * 2.5;
    let cover = coverage.clamp(0.0, 1.0) as f64;
    let (alt0, alt_spread) = (altitude as f64, spread.max(0.0) as f64);
    let hang = depth.clamp(0.05, 5.0) as f64;
    let island = |cx: i32, cz: i32| -> Option<Island> {
        let (mx, mz) = ((cx as f64 + 0.5) * cell, (cz as f64 + 0.5) * cell);
        // Flocks: more islands where the slow noise is high.
        let here = (flock.fbm(mx / (cell * 5.0), mz / (cell * 5.0), 3, 2.0, 0.5) * 0.5 + 0.5).clamp(0.0, 1.0);
        let chance = (cover * (0.3 + 1.4 * here)).min(1.0);
        if rand(seed, cx, cz, 0) >= chance {
            return None;
        }
        let radius = r_max * (0.35 + 0.65 * rand(seed, cx, cz, 1).powf(1.4));
        let centre = (
            (cx as f64 + 0.25 + 0.5 * rand(seed, cx, cz, 2)) * cell,
            (cz as f64 + 0.25 + 0.5 * rand(seed, cx, cz, 3)) * cell,
        );
        let altitude = alt0 + alt_spread * (2.0 * rand(seed, cx, cz, 4) - 1.0);
        let lowest = (
            centre.0 + radius * 0.3 * (rand(seed, cx, cz, 6) - 0.5),
            centre.1 + radius * 0.3 * (rand(seed, cx, cz, 7) - 0.5),
        );
        Some(Island {
            cell: (cx, cz),
            centre,
            radius,
            altitude,
            tower: rand(seed, cx, cz, 5),
            lowest,
        })
    };
    // The islands of every cell the map touches, and of those around.
    let cell_of = |v: f64| (v / cell).floor() as i32;
    let ((x_lo, z_lo), (x_hi, z_hi)) = (e.world(0, 0), e.world(n - 1, n - 1));
    let (cx0, cz0) = (cell_of(x_lo.min(x_hi)) - 1, cell_of(z_lo.min(z_hi)) - 1);
    let (cx1, cz1) = (cell_of(x_lo.max(x_hi)) + 1, cell_of(z_lo.max(z_hi)) + 1);
    let across = (cx1 - cx0 + 1) as usize;
    let down = (cz1 - cz0 + 1) as usize;
    let islands: Vec<Option<Island>> = (0..across * down)
        .into_par_iter()
        .map(|k| island(cx0 + (k % across) as i32, cz0 + (k / across) as i32))
        .collect();
    let island_at = |cx: i32, cz: i32| islands[(cz - cz0) as usize * across + (cx - cx0) as usize];
    let values: Vec<[f32; 3]> = (0..n * n)
        .into_par_iter()
        .map(|c| {
            let (x, z) = e.world(c % n, c / n);
            let (gx, gz) = (cell_of(x), cell_of(z));
            // The island whose edge is farthest out here.
            let mut best: Option<(f64, Island)> = None;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let Some(isl) = island_at(gx + dx, gz + dz) else { continue };
                    let d = (x - isl.centre.0).hypot(z - isl.centre.1);
                    // Well outside it, its ragged edge does not matter.
                    let q = if d > isl.radius * 1.4 {
                        d / isl.radius
                    } else {
                        let (ox, oz) = (isl.cell.0 as f64 * 7.31, isl.cell.1 as f64 * 3.17);
                        let k = isl.radius * 0.42;
                        d / (isl.radius * (1.0 + 0.32 * noise.fbm(x / k + ox, z / k + oz, 5, 2.0, 0.5)))
                    };
                    if best.is_none_or(|b| q < b.0) {
                        best = Some((q, isl));
                    }
                }
            }
            let Some((q, isl)) = best else {
                return [altitude, altitude, 0.0];
            };
            if q >= 1.0 {
                let a = isl.altitude as f32;
                return [a, a, 0.0];
            }
            let r = isl.radius;
            // Sheer sides: the top stays up almost to the edge. Some islands
            // are towers of rock, with a crag rising from their middle.
            let rim = smoothstep(1.0, 0.88, q as f32) as f64;
            let wall = r * (0.22 + 0.42 * isl.tower) * rim;
            let k = r * 0.5;
            let hills = r * 0.25 * (1.0 - q * q).max(0.0).powf(1.5) * (0.45 + 0.55 * relief.ridged(x / k, z / k, 4, 2.0, 0.5));
            let crag = r * 0.55 * isl.tower * isl.tower * (1.0 - q / 0.55).max(0.0).powf(1.6) * (0.6 + 0.4 * relief.ridged(x / (r * 0.25), z / (r * 0.25), 3, 2.0, 0.5));
            let top = isl.altitude + wall + hills + crag + 1.2 * relief.fbm(x / 9.0, z / 9.0, 2, 2.0, 0.5);
            // The underside: ribbed and jagged, hanging lowest somewhere off
            // the middle.
            let angle = (z - isl.centre.1).atan2(x - isl.centre.0);
            let rib = ribs.ridged(angle * 4.0 + isl.cell.0 as f64, q * 2.5 + isl.cell.1 as f64, 4, 2.0, 0.5);
            let off = ((x - isl.lowest.0).hypot(z - isl.lowest.1) / r).min(1.0);
            let jag = ribs.fbm(x / (r * 0.18), z / (r * 0.18), 4, 2.0, 0.5);
            let hanging = r * hang * (1.0 - q).powf(1.1) * (0.55 + 0.45 * rib) * (1.3 - 0.6 * off) * (1.0 + 0.25 * jag);
            let bottom = isl.altitude - 1.5 * rim - hanging.max(0.0);
            [top as f32, bottom.min(top - 0.01) as f32, smoothstep(1.0, 0.9, q as f32)]
        })
        .collect();
    let take = |k: usize| Field::new(n, values.iter().map(|v| v[k]).collect());
    (take(0), take(1), take(2))
}

/// Rims of rock jutting out from the top of cliffs over the void below.
///
/// From any point, the highest ground within `reach` tells whether a cliff
/// rises nearby (a drop of at least `drop`) and how high its top is: the
/// rim's top. How far a rim reaches changes along the cliff; it is thickest
/// where it leaves the cliff and droops a little toward its tip, and never
/// reaches down to the ground under it.
pub(super) fn overhang(f: &Field, e: &Extent, reach: f32, thickness: f32, drop: f32, variation: f32, seed: u32) -> (Field, Field) {
    let n = f.n;
    let spacing = e.spacing();
    let r = (reach.max(0.0) / spacing).round() as usize;
    if r == 0 || n < 3 {
        return (f.clone(), f.clone());
    }
    // The highest ground at a third, two thirds and all of the reach.
    let radii = [r.div_ceil(3), (2 * r).div_ceil(3), r];
    let highs: Vec<Field> = radii.iter().map(|&k| max_filter(f, k)).collect();
    let noise = Noise::new(seed as u64 ^ 0x0e7a);
    let lumps = noise.derive(1);
    let vary = variation.clamp(0.0, 1.0) as f64;
    let (drop, thick) = (drop.max(1.0), thickness.max(1.0));
    let values: Vec<[f32; 2]> = (0..n * n)
        .into_par_iter()
        .map(|c| {
            let g = f.data[c];
            // No cliff within reach: nothing, and nothing to work out.
            let far = highs[2].data[c];
            if far - g <= drop {
                return [far, far];
            }
            let (x, z) = e.world(c % n, c / n);
            let s = (reach as f64 * 5.0).max(8.0);
            let along = noise.fbm(x / s, z / s, 3, 2.0, 0.5) * 0.5 + 0.5;
            // How many of the three reaches this stretch of cliff uses.
            let level = 1.0 - vary + vary * along.clamp(0.0, 1.0);
            let used = ((level * 3.0).ceil() as usize).clamp(1, 3);
            let mut seen = 0;
            let mut top = None;
            for h in highs.iter().take(used) {
                if h.data[c] - g > drop {
                    seen += 1;
                    top = Some(h.data[c]);
                }
            }
            let w = seen as f32 / used as f32;
            let outer = highs[used - 1].data[c] - thick * 0.4 * (1.0 - 1.0 / used as f32);
            let Some(cliff) = top else {
                return [outer, outer];
            };
            let lump = (lumps.fbm(x / 7.0, z / 7.0, 3, 2.0, 0.5) * 0.5 + 0.5) as f32;
            let top = cliff - thick * 0.4 * (1.0 - w);
            let bottom = (top - thick * (0.45 + 0.55 * w) * (0.7 + 0.6 * lump)).max(g + 3.0);
            if top - bottom < 1.0 { [top, top] } else { [top, bottom] }
        })
        .collect();
    let take = |k: usize| Field::new(n, values.iter().map(|v| v[k]).collect());
    (take(0), take(1))
}

/// The highest value within `r` samples (a square), in O(1) per sample
/// whatever `r`: rows, then columns as the rows of a transposed copy.
pub(super) fn max_filter(f: &Field, r: usize) -> Field {
    let n = f.n;
    let mut a = f.data.clone();
    let mut t = vec![0f32; n * n];
    max_rows(&mut a, n, r);
    super::ops::transpose(&a, &mut t, n);
    max_rows(&mut t, n, r);
    super::ops::transpose(&t, &mut a, n);
    Field::new(n, a)
}

/// The highest value within `r` along every row (van Herk / Gil-Werman:
/// running maxima forward and backward over blocks of the window's width).
fn max_rows(data: &mut [f32], n: usize, r: usize) {
    let w = 2 * r + 1;
    let m = n + 2 * r;
    data.par_chunks_mut(n).for_each_init(
        || (vec![0f32; m], vec![0f32; m], vec![0f32; m]),
        |(src, fwd, back), row| {
            src[..r].fill(f32::NEG_INFINITY);
            src[r + n..].fill(f32::NEG_INFINITY);
            src[r..r + n].copy_from_slice(row);
            for k in 0..m {
                fwd[k] = if k % w == 0 { src[k] } else { fwd[k - 1].max(src[k]) };
            }
            for k in (0..m).rev() {
                back[k] = if k % w == w - 1 || k == m - 1 { src[k] } else { back[k + 1].max(src[k]) };
            }
            for (t, o) in row.iter_mut().enumerate() {
                *o = back[t].max(fwd[t + w - 1]);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_max_filter_matches_a_plain_one() {
        let n = 37;
        let data: Vec<f32> = (0..n * n).map(|c| ((c * 7919) % 101) as f32).collect();
        let f = Field::new(n, data);
        for r in [1, 2, 5, 11] {
            let fast = max_filter(&f, r);
            for j in 0..n {
                for i in 0..n {
                    let mut m = f32::NEG_INFINITY;
                    for b in j.saturating_sub(r)..(j + r + 1).min(n) {
                        for a in i.saturating_sub(r)..(i + r + 1).min(n) {
                            m = m.max(f.at(a, b));
                        }
                    }
                    assert_eq!(fast.at(i, j), m, "r {r} at {i}, {j}");
                }
            }
        }
    }
}
