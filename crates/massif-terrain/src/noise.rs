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
