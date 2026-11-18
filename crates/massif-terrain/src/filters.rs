//! Filters over grids that several nodes share: a Gaussian blur, minimum
//! and maximum filters, an exact distance transform, the resampling of a
//! coarser level's samples at a finer level's places, and a level to work
//! at for filters wider than a few samples.

use crate::kernel::{Ctx, Grid};
use crate::space::{Rect, Space};

/// The finest level from `lod` up where `reach` blocks span at most
/// `samples` samples: a wide filter works there and is resampled, its
/// result being smooth at that scale anyway.
pub fn work_lod(lod: u8, reach: f64, samples: f64) -> u8 {
    let mut l = lod;
    while l < crate::space::MAX_LOD && reach / Space::spacing(l) > samples {
        l += 1;
    }
    l
}

/// The weights of a Gaussian of deviation `s` samples, out to 3 deviations.
pub fn gaussian(s: f64) -> Vec<f32> {
    let r = (3.0 * s).ceil() as i32;
    let w: Vec<f64> = (-r..=r).map(|k| (-(k * k) as f64 / (2.0 * s * s)).exp()).collect();
    let sum: f64 = w.iter().sum();
    w.iter().map(|v| (v / sum) as f32).collect()
}

/// `src` blurred with a Gaussian of deviation `s` samples, over what the
/// kernel fully covers (the grid shrinks by its reach on every side).
pub fn blur(src: &Grid, s: f64) -> Grid {
    if s < 0.3 {
        return src.clone();
    }
    let k = gaussian(s);
    let r = (k.len() / 2) as i32;
    let (w, h) = (src.rect.w as i32, src.rect.h as i32);
    // Rows, then columns.
    let (iw, ih) = ((w - 2 * r).max(1), (h - 2 * r).max(1));
    let mut rows = vec![0.0f32; (iw * h) as usize];
    for z in 0..h {
        let line = &src.data[(z * w) as usize..((z + 1) * w) as usize];
        for x in 0..iw {
            let mut acc = 0.0;
            for (t, kt) in k.iter().enumerate() {
                acc += kt * line[(x + t as i32).min(w - 1) as usize];
            }
            rows[(z * iw + x) as usize] = acc;
        }
    }
    let mut cols = vec![0.0f32; (iw * ih) as usize];
    for z in 0..ih {
        for x in 0..iw {
            let mut acc = 0.0;
            for (t, kt) in k.iter().enumerate() {
                acc += kt * rows[((z + t as i32).min(h - 1) * iw + x) as usize];
            }
            cols[(z * iw + x) as usize] = acc;
        }
    }
    Grid::new(src.lod, Rect::new(src.rect.x + r, src.rect.z + r, iw as u32, ih as u32), cols)
}

/// The largest (`max`) or smallest value within `r` samples (a disc),
/// over what the disc fully covers (the grid shrinks by `r` on every side).
pub fn extreme(src: &Grid, r: i32, max: bool) -> Grid {
    if r <= 0 {
        return src.clone();
    }
    let (w, h) = (src.rect.w as i32, src.rect.h as i32);
    let (iw, ih) = ((w - 2 * r).max(1), (h - 2 * r).max(1));
    // Each row of the disc, as its half-width.
    let rows: Vec<(i32, i32)> = (-r..=r).map(|dz| (dz, (((r * r - dz * dz) as f64).sqrt()).floor() as i32)).collect();
    let mut out = vec![0.0f32; (iw * ih) as usize];
    for z in 0..ih {
        for x in 0..iw {
            let (cx, cz) = (x + r, z + r);
            let mut v = if max { f32::MIN } else { f32::MAX };
            for &(dz, half) in &rows {
                let line = &src.data[((cz + dz).clamp(0, h - 1) * w) as usize..][..w as usize];
                for dx in -half..=half {
                    let s = line[(cx + dx).clamp(0, w - 1) as usize];
                    v = if max { v.max(s) } else { v.min(s) };
                }
            }
            out[(z * iw + x) as usize] = v;
        }
    }
    Grid::new(src.lod, Rect::new(src.rect.x + r, src.rect.z + r, iw as u32, ih as u32), out)
}

/// The squared distance transform of one line (Felzenszwalb and
/// Huttenlocher): for each place, the least `(q − p)² + f[p]`.
fn transform_line(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let parabola = |p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
        let mut s = parabola(v[k]);
        while s <= z[k] {
            k -= 1;
            s = parabola(v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        *o = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
    }
}

/// The exact Euclidean distance, in samples, from each sample of a grid
/// `n_x` × `n_z` to the nearest one where `inside` holds, rows then
/// columns; `f32::MAX` where there is none.
pub fn distance(n_x: usize, n_z: usize, inside: impl Fn(usize) -> bool) -> Vec<f32> {
    // Farther than any distance (2^17 squared is under 2e10), and exact
    // when small squares are added to it.
    const FAR: f64 = 1e12;
    let mut d: Vec<f64> = (0..n_x * n_z).map(|k| if inside(k) { 0.0 } else { FAR }).collect();
    let mut f = vec![0.0f64; n_z.max(n_x)];
    let mut out = vec![0.0f64; n_z.max(n_x)];
    for x in 0..n_x {
        for z in 0..n_z {
            f[z] = d[z * n_x + x];
        }
        transform_line(&f[..n_z], &mut out[..n_z]);
        for z in 0..n_z {
            d[z * n_x + x] = out[z];
        }
    }
    for z in 0..n_z {
        f[..n_x].copy_from_slice(&d[z * n_x..(z + 1) * n_x]);
        transform_line(&f[..n_x], &mut out[..n_x]);
        d[z * n_x..(z + 1) * n_x].copy_from_slice(&out[..n_x]);
    }
    d.iter().map(|v| if *v >= FAR * 0.5 { f32::MAX } else { v.sqrt() as f32 }).collect()
}

/// Writes into `out` the values of `g` (at its own, coarser or equal,
/// level) at the samples of the kernel's tile: as they are at the same
/// level, by clamped Catmull-Rom from a coarser one.
pub fn resample(ctx: &Ctx, g: &Grid, out: &mut [f32]) {
    let rect = ctx.rect;
    if g.lod == ctx.lod {
        out.copy_from_slice(&g.crop(rect).data);
        return;
    }
    let f = Space::spacing(ctx.lod) / Space::spacing(g.lod);
    let w = rect.w as usize;
    for (j, row) in out.chunks_mut(w).enumerate() {
        let v = (rect.z + j as i32) as f64 * f;
        for (i, o) in row.iter_mut().enumerate() {
            *o = g.bicubic((rect.x + i as i32) as f64 * f, v);
        }
    }
}

/// The gradient of `g` at `(x, z)` (absolute samples), in blocks per block,
/// by central differences.
#[inline]
pub fn gradient(g: &Grid, x: i32, z: i32) -> (f32, f32) {
    let inv = 1.0 / (2.0 * Space::spacing(g.lod) as f32);
    ((g.at(x + 1, z) - g.at(x - 1, z)) * inv, (g.at(x, z + 1) - g.at(x, z - 1)) * inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_exact() {
        // One point in the middle of 9 × 7.
        let d = distance(9, 7, |k| k == 3 * 9 + 4);
        assert_eq!(d[3 * 9 + 4], 0.0);
        assert!((d[0] - (16.0f32 + 9.0).sqrt()).abs() < 1e-5);
        assert!((d[3 * 9 + 8] - 4.0).abs() < 1e-5);
        // Two points: each sample takes the nearer.
        let d = distance(10, 1, |k| k == 0 || k == 9);
        assert_eq!(d, vec![0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
        assert!(distance(3, 3, |_| false).iter().all(|v| *v == f32::MAX));
    }

    #[test]
    fn extremes_and_levels() {
        let g = Grid::new(0, Rect::new(0, 0, 5, 1), vec![0.0, 0.0, 5.0, 0.0, 0.0]);
        let m = extreme(&g, 1, true);
        assert_eq!(m.data, vec![5.0, 5.0, 5.0]);
        assert_eq!(work_lod(0, 64.0, 32.0), 1);
        assert_eq!(work_lod(2, 64.0, 32.0), 2);
    }
}
