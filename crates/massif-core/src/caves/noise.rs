//! The noises caves are shaped from: 3D gradient noise ("Perlin") on a
//! hashed lattice, so that it never repeats across a map tens of thousands
//! of blocks wide, its fractal sums, and the share of space under a value,
//! measured once, so that a cave's density is the share of the rock it
//! takes whatever noise shapes it.

use std::sync::OnceLock;

/// The twelve edges of a cube, and four of them again: a gradient picked by
/// four bits of a hash.
const GRADIENTS: [[f32; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [0.0, -1.0, -1.0],
];

/// A hash of a lattice point, spread over all 64 bits.
#[inline]
pub fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let mut h = seed ^ (x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (z as u32 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^ (h >> 29)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 3D gradient noise, about −1 to 1, 0 at every lattice point.
#[inline]
pub fn gradient(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (dx, dy, dz) = (x - fx, y - fy, z - fz);
    let corner = |i: i32, j: i32, k: i32| {
        let g = GRADIENTS[(hash3(seed, ix + i, iy + j, iz + k) >> 60) as usize];
        g[0] * (dx - i as f32) + g[1] * (dy - j as f32) + g[2] * (dz - k as f32)
    };
    let (u, v, w) = (fade(dx), fade(dy), fade(dz));
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// 2D gradient noise, about −1 to 1 (a slice of [`gradient`]).
#[inline]
pub fn gradient2(seed: u64, x: f32, z: f32) -> f32 {
    gradient(seed, x, 0.5, z)
}

/// A sum of `octaves` of [`gradient`] noise, each twice as fine and half as
/// strong as the last, scaled back to about −1 to 1.
#[inline]
pub fn fbm(seed: u64, x: f32, y: f32, z: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * gradient(seed.wrapping_add(o as u64 * 0x632B_E59B_D9B4_E019), x * f, y * f, z * f);
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

fn value_hash(seed: u64, a: i64, b: i64, salt: u64) -> u64 {
    let mut r = super::ravines::Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ (a as u64).wrapping_mul(0x9E37_79B1) ^ (b as u64).wrapping_mul(0x85EB_CA77_C2B2_AE63) ^ salt);
    r.next()
}

/// Value noise in 3D, 0 to 1, smooth: the underground pockets' (see
/// [`crate::pockets`]), which must stay as they are block for block.
pub fn noise3(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (xf as i64, yf as i64, zf as i64);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v, w) = (s(x - xf), s(y - yf), s(z - zf));
    let at = |i: i64, j: i64, k: i64| (value_hash(seed, (i * 73_856_093) ^ (k * 19_349_663), j, 7) >> 40) as f32 / (1u64 << 24) as f32;
    let x00 = lerp(at(xi, yi, zi), at(xi + 1, yi, zi), u);
    let x10 = lerp(at(xi, yi + 1, zi), at(xi + 1, yi + 1, zi), u);
    let x01 = lerp(at(xi, yi, zi + 1), at(xi + 1, yi, zi + 1), u);
    let x11 = lerp(at(xi, yi + 1, zi + 1), at(xi + 1, yi + 1, zi + 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// The share of space where a noise lies under a value, measured once over
/// many points: `quantile(p)` is the value under which a share `p` of space
/// lies.
pub struct Quantiles {
    /// The values at shares 0, 1/N, … 1.
    values: Vec<f32>,
}

const STEPS: usize = 1024;

impl Quantiles {
    fn measure(sample: impl Fn(f32, f32, f32) -> f32) -> Self {
        // A fixed scatter of points, far apart in noise cells, so that the
        // same table comes out on every machine.
        let n = 1 << 17;
        let mut all: Vec<f32> = (0..n)
            .map(|i: u32| {
                let h = hash3(0x0A5E_ED, i as i32, 7, 11);
                let at = |s: u32| ((h >> s) & 0xFFFF) as f32 / 65536.0 * 4096.0;
                sample(at(0), at(16), at(32))
            })
            .collect();
        all.sort_by(f32::total_cmp);
        let values = (0..=STEPS).map(|k| all[((k * (n as usize - 1)) / STEPS).min(n as usize - 1)]).collect();
        Self { values }
    }

    /// The value under which share `p` (0 to 1) of space lies.
    pub fn quantile(&self, p: f32) -> f32 {
        let t = p.clamp(0.0, 1.0) * STEPS as f32;
        let i = (t.floor() as usize).min(STEPS - 1);
        lerp(self.values[i], self.values[i + 1], t - i as f32)
    }
}

/// The spread of the cheese noise ([`fbm`] of three octaves).
pub fn cheese_quantiles() -> &'static Quantiles {
    static Q: OnceLock<Quantiles> = OnceLock::new();
    Q.get_or_init(|| Quantiles::measure(|x, y, z| fbm(0x00C4_EE5E, x, y, z, 3)))
}

/// The spread of one [`gradient`] noise.
pub fn signed_quantiles() -> &'static Quantiles {
    static Q: OnceLock<Quantiles> = OnceLock::new();
    Q.get_or_init(|| Quantiles::measure(|x, y, z| gradient(0x0051_6E3D, x, y, z)))
}

/// The spread of the size of one [`gradient`] noise, `|n|`.
pub fn ridge_quantiles() -> &'static Quantiles {
    static Q: OnceLock<Quantiles> = OnceLock::new();
    Q.get_or_init(|| Quantiles::measure(|x, y, z| gradient(0x0051_DA6E, x, y, z).abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_smooth_and_bounded() {
        let mut max = 0.0f32;
        for i in 0..20_000 {
            let (x, y, z) = (i as f32 * 0.137, i as f32 * 0.071, i as f32 * 0.093);
            let v = gradient(3, x, y, z);
            max = max.max(v.abs());
            // Close points, close values.
            assert!((gradient(3, x + 0.01, y, z) - v).abs() < 0.05);
        }
        assert!(max <= 1.2 && max > 0.5, "{max}");
        assert_eq!(gradient(3, 5.0, 6.0, 7.0), 0.0, "0 at lattice points");
    }

    /// The pockets' noise, as it was: the same blocks in maps made before.
    #[test]
    fn the_pockets_noise_stays_the_same() {
        // As the carver of 1.0 worked them out.
        let v = [noise3(7, 1.5, 2.25, -3.75), noise3(0x706f_636b_6574, 10.1, -4.2, 7.7)];
        assert_eq!(v.map(f32::to_bits), [0x3f22_8923, 0x3f06_4ef4]);
    }

    #[test]
    fn quantiles_match_the_share_of_space() {
        let q = ridge_quantiles();
        let w = q.quantile(0.1);
        let (mut under, n) = (0, 50_000);
        for i in 0..n {
            let h = hash3(99, i, 1, 2);
            let at = |s: u32| ((h >> s) & 0xFFFF) as f32 / 65536.0 * 2000.0;
            if gradient(0x0051_DA6E ^ 5, at(0), at(16), at(32)).abs() < w {
                under += 1;
            }
        }
        let share = under as f32 / n as f32;
        assert!((share - 0.1).abs() < 0.01, "{share}");
        assert!(cheese_quantiles().quantile(0.0) < cheese_quantiles().quantile(0.5));
    }
}
