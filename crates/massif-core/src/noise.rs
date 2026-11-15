//! Seeded 2D noises — gradient ("Perlin"), simplex and value — and the
//! usual fractal sums built on them.

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// The kind of lattice noise a fractal is summed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Basis {
    /// Gradients on a triangle grid: round shapes, no grid to be seen.
    #[default]
    Simplex,
    /// Gradients on a square grid ("Perlin"): a little squarer.
    Perlin,
    /// Values on a square grid, blended: soft, blocky blobs.
    Value,
}

impl Basis {
    pub const ALL: [Basis; 3] = [Basis::Simplex, Basis::Perlin, Basis::Value];

    pub const fn label(self) -> &'static str {
        match self {
            Basis::Simplex => "Simplex",
            Basis::Perlin => "Perlin",
            Basis::Value => "Value",
        }
    }
}

/// 2D simplex noise peaks at about 1/99.2 with unit gradients.
const SIMPLEX_SCALE: f64 = 99.2;

/// 2D gradient ("Perlin") noise with hashed lattice gradients. Unlike the
/// classic permutation table it does not repeat every 256 cells, which
/// matters on maps tens of thousands of blocks wide.
#[derive(Debug, Clone)]
pub struct Noise {
    seed: u64,
    gradients: [(f64, f64); 64],
}

#[inline]
fn hash(ix: i64, iy: i64, seed: u64) -> u64 {
    let mut h = (ix as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (iy as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ seed;
    h ^= h >> 32;
    h = h.wrapping_mul(0xd6e8_feb8_6659_fd93);
    h ^= h >> 32;
    h = h.wrapping_mul(0xd6e8_feb8_6659_fd93);
    h ^ (h >> 32)
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

impl Noise {
    pub fn new(seed: u64) -> Self {
        let mut gradients = [(0.0, 0.0); 64];
        for (i, g) in gradients.iter_mut().enumerate() {
            let a = (i as f64 + 0.5) / 64.0 * TAU;
            *g = (a.cos(), a.sin());
        }
        Self { seed, gradients }
    }

    /// A differently seeded copy, for decorrelated layers.
    pub fn derive(&self, salt: u64) -> Self {
        Self {
            seed: hash(salt as i64, 0x5eed, self.seed),
            gradients: self.gradients,
        }
    }

    #[inline]
    fn corner(&self, ix: i64, iy: i64, dx: f64, dy: f64) -> f64 {
        let (gx, gy) = self.gradients[(hash(ix, iy, self.seed) >> 58) as usize];
        gx * dx + gy * dy
    }

    /// Gradient noise in roughly `-1..1`, zero at lattice points.
    pub fn get(&self, x: f64, y: f64) -> f64 {
        let (fx, fy) = (x.floor(), y.floor());
        let (ix, iy) = (fx as i64, fy as i64);
        let (dx, dy) = (x - fx, y - fy);
        let (u, v) = (fade(dx), fade(dy));
        let a = self.corner(ix, iy, dx, dy);
        let b = self.corner(ix + 1, iy, dx - 1.0, dy);
        let c = self.corner(ix, iy + 1, dx, dy - 1.0);
        let d = self.corner(ix + 1, iy + 1, dx - 1.0, dy - 1.0);
        let top = a + (b - a) * u;
        let bottom = c + (d - c) * u;
        // 2D gradient noise peaks at sqrt(1/2); rescale to about -1..1.
        (top + (bottom - top) * v) * std::f64::consts::SQRT_2
    }

    /// Simplex noise in roughly `-1..1`: gradients on a grid of triangles,
    /// so that no direction stands out.
    pub fn simplex(&self, x: f64, y: f64) -> f64 {
        // Skew to the grid of triangles, and back.
        const F2: f64 = 0.366_025_403_784_438_6; // (√3 − 1) / 2
        const G2: f64 = 0.211_324_865_405_187_1; // (3 − √3) / 6
        let s = (x + y) * F2;
        let (i, j) = ((x + s).floor(), (y + s).floor());
        let t = (i + j) * G2;
        let (x0, y0) = (x - (i - t), y - (j - t));
        // The triangle the point is in: its middle corner.
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let (x1, y1) = (x0 - i1 as f64 + G2, y0 - j1 as f64 + G2);
        let (x2, y2) = (x0 - 1.0 + 2.0 * G2, y0 - 1.0 + 2.0 * G2);
        let (ii, jj) = (i as i64, j as i64);
        let corner = |ix: i64, iy: i64, dx: f64, dy: f64| {
            let t = 0.5 - dx * dx - dy * dy;
            if t <= 0.0 {
                0.0
            } else {
                let t2 = t * t;
                t2 * t2 * self.corner(ix, iy, dx, dy)
            }
        };
        (corner(ii, jj, x0, y0) + corner(ii + i1, jj + j1, x1, y1) + corner(ii + 1, jj + 1, x2, y2)) * SIMPLEX_SCALE
    }

    /// Value noise in `-1..1`: a random value at every point of a square
    /// grid, blended smoothly in between.
    pub fn value(&self, x: f64, y: f64) -> f64 {
        let (fx, fy) = (x.floor(), y.floor());
        let (ix, iy) = (fx as i64, fy as i64);
        let (u, v) = (fade(x - fx), fade(y - fy));
        let at = |ix: i64, iy: i64| (hash(ix, iy, self.seed ^ 0x7a1e_5eed) >> 11) as f64 / (1u64 << 52) as f64 - 1.0;
        let (a, b) = (at(ix, iy), at(ix + 1, iy));
        let (c, d) = (at(ix, iy + 1), at(ix + 1, iy + 1));
        let top = a + (b - a) * u;
        let bottom = c + (d - c) * u;
        top + (bottom - top) * v
    }

    /// Noise of the kind `basis`, in roughly `-1..1`.
    #[inline]
    pub fn sample(&self, basis: Basis, x: f64, y: f64) -> f64 {
        match basis {
            Basis::Perlin => self.get(x, y),
            Basis::Simplex => self.simplex(x, y),
            Basis::Value => self.value(x, y),
        }
    }

    /// Fractional Brownian motion, normalised to roughly `-1..1`.
    pub fn fbm(&self, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        self.fbm_of(Basis::Perlin, x, y, octaves, lacunarity, gain)
    }

    /// [`Noise::fbm`] summed from noise of the kind `basis`: `octaves`
    /// layers, each `lacunarity` times finer than the one before and
    /// counting `gain` times as much.
    pub fn fbm_of(&self, basis: Basis, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            // Offset every octave so that their lattices do not line up.
            let shift = o as f64 * 37.31;
            sum += amp * self.sample(basis, x * freq + shift, y * freq - shift);
            norm += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        if norm > 0.0 { sum / norm } else { 0.0 }
    }

    /// Ridged multifractal in `0..1`: sharp crests, rounded valleys.
    pub fn ridged(&self, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        self.ridged_of(Basis::Perlin, x, y, octaves, lacunarity, gain)
    }

    /// [`Noise::ridged`] summed from noise of the kind `basis`.
    pub fn ridged_of(&self, basis: Basis, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm, mut weight) = (0.0, 1.0, 1.0, 0.0, 1.0);
        for o in 0..octaves {
            let shift = o as f64 * 19.17;
            let n = 1.0 - self.sample(basis, x * freq + shift, y * freq + shift).abs();
            let n = n * n * weight;
            weight = (n * 2.0).clamp(0.0, 1.0);
            sum += n * amp;
            norm += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        if norm > 0.0 { sum / norm } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_bounded_and_continuous() {
        let n = Noise::new(42);
        let m = Noise::new(42);
        let other = Noise::new(43);
        let mut differs = false;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..4000 {
            let (x, y) = (i as f64 * 0.137 - 300.0, i as f64 * 0.071 + 50.0);
            let v = n.get(x, y);
            assert_eq!(v, m.get(x, y));
            differs |= v != other.get(x, y);
            assert!((-1.05..=1.05).contains(&v), "{v}");
            lo = lo.min(v);
            hi = hi.max(v);
            // Lipschitz: a tiny step moves the value by a tiny amount.
            assert!((n.get(x + 1e-3, y) - v).abs() < 0.01);
        }
        assert!(differs);
        assert!(lo < -0.5 && hi > 0.5, "range {lo}..{hi}");
        assert_eq!(n.get(3.0, -7.0), 0.0);
    }

    #[test]
    fn fractals_stay_in_range() {
        let n = Noise::new(7);
        for i in 0..2000 {
            let (x, y) = (i as f64 * 0.31, -(i as f64) * 0.17);
            let f = n.fbm(x, y, 5, 2.0, 0.5);
            let r = n.ridged(x, y, 5, 2.0, 0.5);
            assert!((-1.05..=1.05).contains(&f), "{f}");
            assert!((0.0..=1.0).contains(&r), "{r}");
            for basis in Basis::ALL {
                let f = n.fbm_of(basis, x, y, 6, 2.5, 0.6);
                let r = n.ridged_of(basis, x, y, 6, 2.5, 0.6);
                assert!((-1.05..=1.05).contains(&f), "{basis:?} {f}");
                assert!((0.0..=1.0).contains(&r), "{basis:?} {r}");
            }
        }
        assert_ne!(n.derive(1).get(0.5, 0.5), n.derive(2).get(0.5, 0.5));
        // The Perlin sums are the ones the rest of the code has always used.
        assert_eq!(n.fbm(3.7, -1.2, 4, 2.0, 0.5), n.fbm_of(Basis::Perlin, 3.7, -1.2, 4, 2.0, 0.5));
    }

    #[test]
    fn simplex_and_value_noises_are_bounded_continuous_and_spread() {
        let n = Noise::new(11);
        for basis in [Basis::Simplex, Basis::Value] {
            let (mut lo, mut hi, mut sum) = (f64::MAX, f64::MIN, 0.0);
            for i in 0..20_000 {
                let (x, y) = (i as f64 * 0.0731 - 500.0, (i % 137) as f64 * 0.391 + i as f64 * 0.013);
                let v = n.sample(basis, x, y);
                assert!((-1.01..=1.01).contains(&v), "{basis:?} {v}");
                assert!((n.sample(basis, x + 1e-3, y) - v).abs() < 0.02, "{basis:?} jumps at {x} {y}");
                assert_eq!(v, Noise::new(11).sample(basis, x, y));
                lo = lo.min(v);
                hi = hi.max(v);
                sum += v;
            }
            assert!(lo < -0.5 && hi > 0.5, "{basis:?} range {lo}..{hi}");
            assert!((sum / 20_000.0).abs() < 0.1, "{basis:?} mean {}", sum / 20_000.0);
            assert_ne!(n.sample(basis, 0.3, 0.7), Noise::new(12).sample(basis, 0.3, 0.7));
        }
    }
}
