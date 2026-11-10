//! Seeded 2D gradient noise and the usual fractal sums built on it.

use std::f64::consts::TAU;

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

    /// Fractional Brownian motion, normalised to roughly `-1..1`.
    pub fn fbm(&self, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            // Offset every octave so that their lattices do not line up.
            let shift = o as f64 * 37.31;
            sum += amp * self.get(x * freq + shift, y * freq - shift);
            norm += amp;
            amp *= gain;
            freq *= lacunarity;
        }
        if norm > 0.0 { sum / norm } else { 0.0 }
    }

    /// Ridged multifractal in `0..1`: sharp crests, rounded valleys.
    pub fn ridged(&self, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
        let (mut sum, mut amp, mut freq, mut norm, mut weight) = (0.0, 1.0, 1.0, 0.0, 1.0);
        for o in 0..octaves {
            let shift = o as f64 * 19.17;
            let n = 1.0 - self.get(x * freq + shift, y * freq + shift).abs();
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
        }
        assert_ne!(n.derive(1).get(0.5, 0.5), n.derive(2).get(0.5, 0.5));
    }
}
