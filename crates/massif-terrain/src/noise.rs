//! Gradient noise in 32-bit arithmetic, the same on the CPU as in the WGSL
//! kernels (`nodes/common.wgsl` holds its twin, line for line).
//!
//! Large worlds and fine noise do not fit a float's precision: at 30,000
//! blocks a `f32` keeps a few thousandths of a block, too little for a
//! lattice a few blocks wide. So each octave's lattice position is split
//! into a whole cell and a fraction where a tile starts, in double
//! precision on the CPU, and samples only add their small offset within the
//! tile to the fraction. Every sample of a level of detail belongs to one
//! tile, so its value never depends on how a region was asked for.

use crate::util::mix32;

/// Unit gradients, half a step off the axes.
pub const GRADS: [[f32; 2]; 16] = [
    [0.980_785_3, 0.195_090_32],
    [0.831_469_6, 0.555_570_24],
    [0.555_570_24, 0.831_469_6],
    [0.195_090_32, 0.980_785_3],
    [-0.195_090_32, 0.980_785_3],
    [-0.555_570_24, 0.831_469_6],
    [-0.831_469_6, 0.555_570_24],
    [-0.980_785_3, 0.195_090_32],
    [-0.980_785_3, -0.195_090_32],
    [-0.831_469_6, -0.555_570_24],
    [-0.555_570_24, -0.831_469_6],
    [-0.195_090_32, -0.980_785_3],
    [0.195_090_32, -0.980_785_3],
    [0.555_570_24, -0.831_469_6],
    [0.831_469_6, -0.555_570_24],
    [0.980_785_3, -0.195_090_32],
];

/// Brings gradient noise (within ±√½) to about ±1.
pub const NORM: f32 = std::f32::consts::SQRT_2;

/// Most octaves a fractal sums.
pub const MAX_OCTAVES: usize = 16;

#[inline]
pub fn hash2(x: u32, z: u32, seed: u32) -> u32 {
    mix32(x.wrapping_mul(0x8da6_b343) ^ z.wrapping_mul(0xd816_3841) ^ seed)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn corner(cx: u32, cz: u32, seed: u32, ox: f32, oz: f32) -> f32 {
    let g = GRADS[(hash2(cx, cz, seed) & 15) as usize];
    g[0] * ox + g[1] * oz
}

/// Gradient noise in cell `(cx, cz)` at `(fx, fz)` within it (each in
/// `[0, 1)`), about ±1.
#[inline]
pub fn gradient(cx: u32, cz: u32, fx: f32, fz: f32, seed: u32) -> f32 {
    let n00 = corner(cx, cz, seed, fx, fz);
    let n10 = corner(cx.wrapping_add(1), cz, seed, fx - 1.0, fz);
    let n01 = corner(cx, cz.wrapping_add(1), seed, fx, fz - 1.0);
    let n11 = corner(cx.wrapping_add(1), cz.wrapping_add(1), seed, fx - 1.0, fz - 1.0);
    let (ux, uz) = (fade(fx), fade(fz));
    let a = n00 + (n10 - n00) * ux;
    let b = n01 + (n11 - n01) * ux;
    (a + (b - a) * uz) * NORM
}

/// One octave of a fractal over one tile: where the tile's first sample
/// lies on the octave's lattice, and how far each sample step moves on it.
/// Laid out as the WGSL `Octave` struct (48 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Octave {
    pub cell: [i32; 2],
    pub frac: [f32; 2],
    /// `q = frac + (step[0]·di + step[1]·dj, step[2]·di + step[3]·dj)`.
    pub step: [f32; 4],
    pub amp: f32,
    pub seed: u32,
}

impl Octave {
    /// The octave's noise at sample offset `(di, dj)` from the tile's first.
    #[inline]
    pub fn at(&self, di: f32, dj: f32) -> f32 {
        let qx = self.frac[0] + self.step[0] * di + self.step[1] * dj;
        let qz = self.frac[1] + self.step[2] * di + self.step[3] * dj;
        let (fx, fz) = (qx.floor(), qz.floor());
        let cx = self.cell[0].wrapping_add(fx as i32) as u32;
        let cz = self.cell[1].wrapping_add(fz as i32) as u32;
        gradient(cx, cz, qx - fx, qz - fz, self.seed)
    }

    /// Writes the octave as WGSL lays it out.
    pub fn write(&self, u: &mut crate::kernel::Uniforms) {
        u.align(16).i32(self.cell[0]).i32(self.cell[1]).f32(self.frac[0]).f32(self.frac[1]);
        for s in self.step {
            u.f32(s);
        }
        u.f32(self.amp).u32(self.seed).u32(0).u32(0);
    }
}

/// How a fractal's octaves are laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fractal {
    /// Blocks across a cell of the first octave.
    pub scale: f64,
    pub octaves: u32,
    /// How much finer each octave is.
    pub lacunarity: f64,
    /// How much weaker each octave is.
    pub gain: f64,
    /// Where the noise is taken, in blocks.
    pub offset: (f64, f64),
    pub seed: u32,
}

impl Fractal {
    /// The octaves for a tile whose first sample lies at `origin` (blocks
    /// from the terrain's corner), samples `spacing` blocks apart.
    pub fn octaves(&self, origin: (f64, f64), spacing: f64) -> Vec<Octave> {
        let n = (self.octaves as usize).clamp(1, MAX_OCTAVES);
        let (ox, oz) = (origin.0 + self.offset.0, origin.1 + self.offset.1);
        (0..n)
            .map(|k| {
                let freq = self.lacunarity.powi(k as i32) / self.scale.max(1e-3);
                // Each octave turned and shifted, so that their lattices
                // never line up.
                let angle = 0.6 + k as f64 * 1.1;
                let (s, c) = angle.sin_cos();
                let seed = mix32(self.seed ^ (k as u32).wrapping_mul(0x9e37_79b9));
                let shift = ((mix32(seed ^ 0x51ed) as f64 / u32::MAX as f64) * 4096.0, (mix32(seed ^ 0xa3c1) as f64 / u32::MAX as f64) * 4096.0);
                let (px, pz) = (ox * freq, oz * freq);
                let qx = c * px - s * pz + shift.0;
                let qz = s * px + c * pz + shift.1;
                let (fx, fz) = (qx.floor(), qz.floor());
                let d = freq * spacing;
                Octave {
                    cell: [(fx as i64) as i32, (fz as i64) as i32],
                    frac: [(qx - fx) as f32, (qz - fz) as f32],
                    step: [(c * d) as f32, (-s * d) as f32, (s * d) as f32, (c * d) as f32],
                    amp: self.gain.powi(k as i32) as f32,
                    seed,
                }
            })
            .collect()
    }
}

/// How octaves add up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Smooth hills: the octaves summed.
    Fbm = 0,
    /// Sharp crests: each octave folded into a ridge, finer ridges where
    /// the coarser ones are high.
    Ridged = 1,
    /// Puffy mounds: each octave folded the other way.
    Billow = 2,
}

/// The fractal at sample offset `(di, dj)`, from 0 to 1.
#[inline]
pub fn fractal(octaves: &[Octave], style: Style, di: f32, dj: f32) -> f32 {
    let (mut sum, mut norm, mut weight) = (0.0f32, 0.0f32, 1.0f32);
    for o in octaves {
        let n = o.at(di, dj);
        let v = match style {
            Style::Fbm => n,
            Style::Billow => 2.0 * n.abs() - 1.0,
            Style::Ridged => {
                let mut r = 1.0 - n.abs();
                r *= r;
                r *= weight;
                weight = (r * 2.0).clamp(0.0, 1.0);
                r
            }
        };
        sum += v * o.amp;
        norm += o.amp;
    }
    let v = sum / norm.max(1e-6);
    match style {
        Style::Ridged => v,
        _ => 0.5 + 0.5 * v,
    }
}

#[cfg(test)]
mod tests {
    /// A fractal stays within ±1, as the archipelago's skipping of far
    /// islands takes it (with room to spare: it allows ±2).
    #[test]
    fn fractals_stay_within_one() {
        let mut most = 0.0f32;
        for seed in [1u32, 7, 0x9e37_79b9] {
            let f = super::Fbm::down_to(900.0, 6.0, 0.55, seed);
            for i in 0..400_000u32 {
                let (x, z) = ((i % 631) as f64 * 3.71 + seed as f64, (i / 631) as f64 * 2.93);
                most = most.max(f.at(x, z).abs()).max(super::noise2(x * 0.37, z * 0.41, seed).abs());
            }
        }
        assert!(most <= 1.0 + 1e-5, "{most}");
        assert!(most > 0.5, "{most}");
    }

    use super::*;

    #[test]
    fn noise_is_bounded_and_varied() {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..4000u32 {
            let v = gradient(i * 7919 % 101, i / 37, (i % 13) as f32 / 13.0, (i % 7) as f32 / 7.0, 42);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo > -1.01 && hi < 1.01 && hi - lo > 1.0, "{lo} {hi}");
    }

    #[test]
    fn noise_is_continuous_across_cells() {
        let f = Fractal { scale: 37.0, octaves: 1, lacunarity: 2.0, gain: 0.5, offset: (0.0, 0.0), seed: 9 };
        let o = f.octaves((0.0, 0.0), 0.25)[0];
        let mut prev = o.at(0.0, 0.0);
        for k in 1..2000 {
            let v = o.at(k as f32, 3.0);
            assert!((v - prev).abs() < 0.1, "a jump at {k}");
            prev = v;
        }
    }

    #[test]
    fn tiles_agree_where_they_meet() {
        // The same world position from two tiles' origins.
        let f = Fractal { scale: 300.0, octaves: 8, lacunarity: 2.0, gain: 0.5, offset: (0.0, 0.0), seed: 3 };
        let a = f.octaves((30_000.0, 12_000.0), 1.0);
        let b = f.octaves((30_256.0, 12_000.0), 1.0);
        for j in 0..8 {
            let va = fractal(&a, Style::Fbm, 256.0, j as f32 * 31.0);
            let vb = fractal(&b, Style::Fbm, 0.0, j as f32 * 31.0);
            assert!((va - vb).abs() < 1e-4, "{va} {vb}");
        }
    }
}

// ------------------------------------------------- noise for shapes (CPU) --

/// fade'(t), for derivatives.
#[inline]
fn fade_d(t: f32) -> f32 {
    30.0 * t * t * (t * (t - 2.0) + 1.0)
}

/// Gradient noise at any world position (lattice units), in double
/// precision before the cell is found: exact however far from the origin.
/// About ±1.
#[inline]
pub fn noise2(x: f64, z: f64, seed: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    gradient((fx as i64) as u32, (fz as i64) as u32, (x - fx) as f32, (z - fz) as f32, seed)
}

/// [`noise2`] with its derivatives along x and z (lattice units).
#[inline]
pub fn noise2_d(x: f64, z: f64, seed: u32) -> (f32, f32, f32) {
    let (fx, fz) = (x.floor(), z.floor());
    let (cx, cz) = ((fx as i64) as u32, (fz as i64) as u32);
    let (tx, tz) = ((x - fx) as f32, (z - fz) as f32);
    let g = |dx: u32, dz: u32| GRADS[(hash2(cx.wrapping_add(dx), cz.wrapping_add(dz), seed) & 15) as usize];
    let (g00, g10, g01, g11) = (g(0, 0), g(1, 0), g(0, 1), g(1, 1));
    let a = g00[0] * tx + g00[1] * tz;
    let b = g10[0] * (tx - 1.0) + g10[1] * tz;
    let c = g01[0] * tx + g01[1] * (tz - 1.0);
    let d = g11[0] * (tx - 1.0) + g11[1] * (tz - 1.0);
    let (u, v) = (fade(tx), fade(tz));
    let (du, dv) = (fade_d(tx), fade_d(tz));
    let k = a - b - c + d;
    let n = a + (b - a) * u + (c - a) * v + k * u * v;
    let nx = g00[0] + (g10[0] - g00[0]) * u + (b - a) * du + (g01[0] - g00[0]) * v + (g00[0] - g10[0] - g01[0] + g11[0]) * u * v + k * du * v;
    let nz = g00[1] + (g10[1] - g00[1]) * u + (g01[1] - g00[1]) * v + (c - a) * dv + (g00[1] - g10[1] - g01[1] + g11[1]) * u * v + k * u * dv;
    (n * NORM, nx * NORM, nz * NORM)
}

/// Noise along a line (a curve's length, an angle), about ±1.
#[inline]
pub fn noise1(t: f64, seed: u32) -> f32 {
    noise2(t, 0.37, seed)
}

/// A fractal at a world position (blocks), for shapes: `octaves` of
/// [`noise2`] from cells `scale` blocks wide, each `lacunarity` times finer
/// and `gain` times weaker, turned so that their lattices never line up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fbm {
    pub scale: f64,
    pub octaves: u32,
    pub lacunarity: f64,
    pub gain: f64,
    pub seed: u32,
    /// Each octave's frequency, turn (sine, cosine), seed and shift.
    oct: [(f64, f64, f64, u32, f64, f64); MAX_OCTAVES],
}

impl Fbm {
    pub fn new(scale: f64, octaves: u32, gain: f64, seed: u32) -> Self {
        let scale = scale.max(1e-3);
        let octaves = octaves.clamp(1, MAX_OCTAVES as u32);
        let mut oct = [(0.0, 0.0, 1.0, 0, 0.0, 0.0); MAX_OCTAVES];
        for (k, o) in oct.iter_mut().enumerate().take(octaves as usize) {
            let f = 2.0f64.powi(k as i32) / scale;
            let (s, c) = (0.6 + k as f64 * 1.1).sin_cos();
            let sd = mix32(seed ^ (k as u32).wrapping_mul(0x9e37_79b9));
            *o = (f, s, c, sd, (sd & 0xfff) as f64, (sd >> 20) as f64);
        }
        Self { scale, octaves, lacunarity: 2.0, gain, seed, oct }
    }

    /// Octaves fine enough to reach `finest` blocks.
    pub fn down_to(scale: f64, finest: f64, gain: f64, seed: u32) -> Self {
        let octaves = ((scale / finest.max(0.5)).log2().ceil() as u32).clamp(1, MAX_OCTAVES as u32);
        Self::new(scale, octaves, gain, seed)
    }

    #[inline]
    fn octave(&self, k: u32, x: f64, z: f64) -> (f64, f64, u32) {
        let (f, s, c, seed, ox, oz) = self.oct[k as usize];
        let (px, pz) = (x * f, z * f);
        (c * px - s * pz + ox, s * px + c * pz + oz, seed)
    }

    /// Summed octaves, about ±1.
    pub fn at(&self, x: f64, z: f64) -> f32 {
        let (mut sum, mut norm, mut amp) = (0.0f32, 0.0f32, 1.0f32);
        for k in 0..self.octaves {
            let (qx, qz, seed) = self.octave(k, x, z);
            sum += amp * noise2(qx, qz, seed);
            norm += amp;
            amp *= self.gain as f32;
        }
        sum / norm
    }

    /// [`Fbm::at`] with its gradient (per block).
    pub fn at_d(&self, x: f64, z: f64) -> (f32, f32, f32) {
        let (mut sum, mut gx, mut gz, mut norm, mut amp) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32);
        for k in 0..self.octaves {
            let (f, s, c, _, _, _) = self.oct[k as usize];
            let (qx, qz, seed) = self.octave(k, x, z);
            let (n, dx, dz) = noise2_d(qx, qz, seed);
            // Back from the octave's turned lattice to the world.
            let (wx, wz) = ((c * dx as f64 + s * dz as f64) * f, (-s * dx as f64 + c * dz as f64) * f);
            sum += amp * n;
            gx += amp * wx as f32;
            gz += amp * wz as f32;
            norm += amp;
            amp *= self.gain as f32;
        }
        (sum / norm, gx / norm, gz / norm)
    }

    /// How far, in blocks, the nearest place where the fractal crosses zero
    /// lies: its value over its slope, the first-order estimate.
    pub fn zero_distance(&self, x: f64, z: f64) -> f32 {
        let (v, gx, gz) = self.at_d(x, z);
        v.abs() / gx.hypot(gz).max(1e-6)
    }

    /// Ridged: crests where the noise crosses zero, from 0 to 1, finer
    /// crests where the coarser ones are high. `sharpness` (0–1) narrows
    /// the crests.
    pub fn ridged(&self, x: f64, z: f64, sharpness: f32) -> f32 {
        let power = 1.0 + 3.0 * sharpness;
        let (mut sum, mut norm, mut amp, mut weight) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
        for k in 0..self.octaves {
            let (qx, qz, seed) = self.octave(k, x, z);
            let r = (1.0 - noise2(qx, qz, seed).abs()).max(0.0).powf(power) * weight;
            weight = (r * 2.0).clamp(0.0, 1.0);
            sum += amp * r;
            norm += amp;
            amp *= self.gain as f32;
        }
        sum / norm
    }

    /// Mountains: ridged octaves whose crests sharpen with each octave —
    /// the first broad, its flanks straight from crest to valley, the last
    /// as sharp as `sharpness` says — finer crests where the coarser ones
    /// are high. From 0 to 1.
    pub fn mountains(&self, x: f64, z: f64, sharpness: f32) -> f32 {
        let (mut sum, mut norm, mut amp, mut weight) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
        let last = (self.octaves.max(2) - 1) as f32;
        for k in 0..self.octaves {
            let (qx, qz, seed) = self.octave(k, x, z);
            let power = 1.0 + 2.5 * sharpness * (k as f32 / last);
            let r = (1.0 - noise2(qx, qz, seed).abs()).max(0.0).powf(power) * weight;
            weight = (r * 1.6).clamp(0.0, 1.0);
            sum += amp * r;
            norm += amp;
            amp *= self.gain as f32;
        }
        sum / norm
    }

    /// Eroded: octaves damped where the coarser ones slope, so that slopes
    /// stay smooth, valleys gather and detail stays on the heights (after
    /// Iñigo Quilez). From 0 to 1, about.
    pub fn eroded(&self, x: f64, z: f64, strength: f32) -> f32 {
        let (mut sum, mut norm, mut amp) = (0.0f32, 0.0f32, 1.0f32);
        let (mut dx, mut dz) = (0.0f32, 0.0f32);
        for k in 0..self.octaves {
            let (qx, qz, seed) = self.octave(k, x, z);
            let (n, nx, nz) = noise2_d(qx, qz, seed);
            dx += nx;
            dz += nz;
            sum += amp * n / (1.0 + strength * (dx * dx + dz * dz));
            norm += amp;
            amp *= self.gain as f32;
        }
        (0.5 + 0.5 * sum / norm).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod shape_noise_tests {
    use super::*;

    #[test]
    fn derivatives_match_differences() {
        let h = 1e-3;
        for (x, z) in [(3.3, 7.9), (-12.6, 0.2), (100.05, -40.7)] {
            let (n, dx, dz) = noise2_d(x, z, 77);
            assert!((n - noise2(x, z, 77)).abs() < 1e-6);
            let ex = (noise2(x + h, z, 77) - noise2(x - h, z, 77)) / (2.0 * h as f32);
            let ez = (noise2(x, z + h, 77) - noise2(x, z - h, 77)) / (2.0 * h as f32);
            assert!((dx - ex).abs() < 2e-2 && (dz - ez).abs() < 2e-2, "{dx} {ex} {dz} {ez}");
        }
    }

    #[test]
    fn fractal_gradients_match_differences() {
        let f = Fbm::new(300.0, 5, 0.5, 9);
        for (x, z) in [(10.0, 20.0), (-3000.5, 777.25)] {
            let (v, gx, gz) = f.at_d(x, z);
            let h = 0.01;
            assert!((v - f.at(x, z)).abs() < 1e-6);
            let ex = (f.at(x + h, z) - f.at(x - h, z)) / (2.0 * h as f32);
            let ez = (f.at(x, z + h) - f.at(x, z - h)) / (2.0 * h as f32);
            assert!((gx - ex).abs() < 2e-4 && (gz - ez).abs() < 2e-4, "{gx} {ex} {gz} {ez}");
        }
    }

    #[test]
    fn shape_fractals_are_bounded() {
        let f = Fbm::down_to(500.0, 4.0, 0.5, 3);
        for k in 0..2000 {
            let (x, z) = (k as f64 * 37.1, k as f64 * 11.3 - 5000.0);
            let r = f.ridged(x, z, 0.7);
            let e = f.eroded(x, z, 1.0);
            assert!((0.0..=1.0).contains(&r) && (0.0..=1.0).contains(&e) && f.at(x, z).abs() <= 1.01);
        }
    }
}

/// Cellular (Worley) noise at lattice position `(x, z)`: the distances to
/// the nearest and the second nearest of the points scattered one per cell
/// (`jitter` from 0, a grid, to 1, at random), in cells, and a hash of the
/// nearest point's cell.
pub fn voronoi(x: f64, z: f64, jitter: f64, seed: u32) -> (f32, f32, u32) {
    let (cx, cz) = (x.floor() as i64, z.floor() as i64);
    let (mut f1, mut f2, mut id) = (f64::MAX, f64::MAX, 0u32);
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (gx, gz) = (cx + dx, cz + dz);
            let h = hash2(gx as u32, gz as u32, seed);
            let jx = (h & 0xffff) as f64 / 65535.0 - 0.5;
            let jz = (h >> 16) as f64 / 65535.0 - 0.5;
            let (px, pz) = (gx as f64 + 0.5 + jitter * jx, gz as f64 + 0.5 + jitter * jz);
            let d = (x - px).hypot(z - pz);
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = h;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1 as f32, f2 as f32, id)
}
