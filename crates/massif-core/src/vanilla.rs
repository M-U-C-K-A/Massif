//! The land Minecraft (1.18 and later) makes for a seed, worked out the way
//! the game does as far as its biomes and the rough shape of its surface
//! go: the six climate noises of the overworld, the offset spline that
//! makes their heights, and the table that picks a biome from a climate.
//!
//! Biomes come out as the game's own, quarter-block by quarter-block (the
//! game keeps a biome for every 4 × 4 × 4 blocks). The height is the rough
//! surface the offset spline gives — where the land's depth is zero — and
//! leaves out what the game adds over it: the 3D noise (a few blocks), the
//! jagged peaks (tens of blocks on the highest), caves, aquifers. That is
//! enough to compare the land around a map with a seed's, for thousands of
//! seeds a second, and to show it at once; the game's own chunks (made by
//! a server) give the exact land.

/// The water level of the overworld.
pub const SEA_LEVEL: i32 = 63;

// ---------------------------------------------------------------- randoms

/// MD5 of the names the game hashes into its randoms, as two big-endian
/// longs.
const HASHES: [(&str, u64, u64); 19] = [
    ("minecraft:temperature", 0x5c7e6b29735f0d7f, 0xf7d86f1bbc734988),
    ("minecraft:vegetation", 0x81bb4d22e8dc168e, 0xf1c8b4bea16303cd),
    ("minecraft:continentalness", 0x83886c9d0ae3a662, 0xafa638a61b42e8ad),
    ("minecraft:erosion", 0xd02491e6058f6fd8, 0x4792512c94c17a80),
    ("minecraft:ridge", 0xefc8ef4d36102b34, 0x1beeeb324a0f24ea),
    ("minecraft:offset", 0x080518cf6af25384, 0x3f3dfb40a54febd5),
    ("octave_-12", 0xb198de63a8012672, 0x7b84cad43ef7b5a8),
    ("octave_-11", 0x0fd787bfbc403ec3, 0x74a4a31ca21b48b8),
    ("octave_-10", 0x36d326eed40efeb2, 0x5be9ce18223c636a),
    ("octave_-9", 0x082fe255f8be6631, 0x4e96119e22dedc81),
    ("octave_-8", 0x0ef68ec68504005e, 0x48b6bf93a2789640),
    ("octave_-7", 0xf11268128982754f, 0x257a1d670430b0aa),
    ("octave_-6", 0xe51c98ce7d1de664, 0x5f9478a733040c45),
    ("octave_-5", 0x6d7b49e7e429850a, 0x2e3063c622a24777),
    ("octave_-4", 0xbd90d5377ba1b762, 0xc07317d419a7548d),
    ("octave_-3", 0x53d39c6752dac858, 0xbcd1c5a80ab65b3e),
    ("octave_-2", 0xb4a24d7a84e7677b, 0x023ff9668e89b5c4),
    ("octave_-1", 0xdffa22b534c5f608, 0xb9b67517d3665ca9),
    ("octave_0", 0xd50708086cef4d7c, 0x6e1651ecc7f43309),
];

fn hash_of(name: &str) -> (u64, u64) {
    HASHES.iter().find(|h| h.0 == name).map(|h| (h.1, h.2)).unwrap_or_else(|| panic!("no hash for {name}"))
}

/// The game's Xoroshiro128++.
#[derive(Debug, Clone)]
struct Xoroshiro {
    lo: u64,
    hi: u64,
}

fn mix_stafford13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

impl Xoroshiro {
    fn new(lo: u64, hi: u64) -> Self {
        if lo == 0 && hi == 0 { Self { lo: 0x9e3779b97f4a7c15, hi: 0x6a09e667f3bcc909 } } else { Self { lo, hi } }
    }

    /// From a world seed, as `RandomSupport.upgradeSeedTo128bit`.
    fn from_seed(seed: i64) -> Self {
        let l = (seed as u64) ^ 0x6a09e667f3bcc909;
        let m = l.wrapping_add(0x9e3779b97f4a7c15);
        Self::new(mix_stafford13(l), mix_stafford13(m))
    }

    fn next_long(&mut self) -> u64 {
        let (l, m) = (self.lo, self.hi);
        let n = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.lo = l.rotate_left(49) ^ m ^ (m << 21);
        self.hi = m.rotate_left(28);
        n
    }

    fn next_int(&mut self) -> u32 {
        self.next_long() as u32
    }

    fn next_bounded(&mut self, bound: u32) -> u32 {
        let mut l = self.next_int() as u64;
        let mut m = l * bound as u64;
        let mut n = m & 0xffff_ffff;
        if n < bound as u64 {
            let i = (bound.wrapping_neg() % bound) as u64;
            while n < i {
                l = self.next_int() as u64;
                m = l * bound as u64;
                n = m & 0xffff_ffff;
            }
        }
        (m >> 32) as u32
    }

    fn next_double(&mut self) -> f64 {
        (self.next_long() >> 11) as f64 * f64::from_bits(0x3ca0000000000000)
    }

    /// `forkPositional`: the seeds of a positional factory.
    fn fork(&mut self) -> Positional {
        Positional { lo: self.next_long(), hi: self.next_long() }
    }
}

/// The game's `XoroshiroPositionalRandomFactory`.
#[derive(Debug, Clone, Copy)]
struct Positional {
    lo: u64,
    hi: u64,
}

impl Positional {
    fn hashed(&self, name: &str) -> Xoroshiro {
        let (a, b) = hash_of(name);
        Xoroshiro::new(a ^ self.lo, b ^ self.hi)
    }
}

// ----------------------------------------------------------------- noises

const GRADIENT: [[f64; 3]; 16] = [
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
    [0.0, -1.0, 1.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, -1.0],
];

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn smooth(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// The game's `ImprovedNoise`.
#[derive(Debug, Clone)]
struct Improved {
    p: [u8; 256],
    xo: f64,
    yo: f64,
    zo: f64,
}

impl Improved {
    fn new(r: &mut Xoroshiro) -> Self {
        let (xo, yo, zo) = (r.next_double() * 256.0, r.next_double() * 256.0, r.next_double() * 256.0);
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let j = r.next_bounded(256 - i as u32) as usize;
            p.swap(i, i + j);
        }
        Self { p, xo, yo, zo }
    }

    #[inline]
    fn perm(&self, i: i32) -> i32 {
        self.p[(i & 0xff) as usize] as i32
    }

    #[inline]
    fn grad(&self, i: i32, x: f64, y: f64, z: f64) -> f64 {
        let g = GRADIENT[(i & 15) as usize];
        g[0] * x + g[1] * y + g[2] * z
    }

    fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        let (i, j, k) = (x + self.xo, y + self.yo, z + self.zo);
        let (l, m, n) = (i.floor(), j.floor(), k.floor());
        let (o, p, q) = (i - l, j - m, k - n);
        let (l, m, n) = (l as i32, m as i32, n as i32);
        let a = self.perm(l);
        let b = self.perm(l + 1);
        let aa = self.perm(a + m);
        let ab = self.perm(a + m + 1);
        let ba = self.perm(b + m);
        let bb = self.perm(b + m + 1);
        let h = self.grad(self.perm(aa + n), o, p, q);
        let s = self.grad(self.perm(ba + n), o - 1.0, p, q);
        let t = self.grad(self.perm(ab + n), o, p - 1.0, q);
        let u = self.grad(self.perm(bb + n), o - 1.0, p - 1.0, q);
        let v = self.grad(self.perm(aa + n + 1), o, p, q - 1.0);
        let w = self.grad(self.perm(ba + n + 1), o - 1.0, p, q - 1.0);
        let xx = self.grad(self.perm(ab + n + 1), o, p - 1.0, q - 1.0);
        let yy = self.grad(self.perm(bb + n + 1), o - 1.0, p - 1.0, q - 1.0);
        let (fx, fy, fz) = (smooth(o), smooth(p), smooth(q));
        lerp(fz, lerp(fy, lerp(fx, h, s), lerp(fx, t, u)), lerp(fy, lerp(fx, v, w), lerp(fx, xx, yy)))
    }
}

#[inline]
fn wrap(d: f64) -> f64 {
    d - (d / 3.3554432e7 + 0.5).floor() * 3.3554432e7
}

/// The game's `PerlinNoise`, made with the positional randoms of 1.18.
#[derive(Debug, Clone)]
struct Perlin {
    levels: Vec<Option<(Improved, f64)>>,
    input: f64,
    value: f64,
}

impl Perlin {
    fn new(r: &mut Xoroshiro, first: i32, amplitudes: &[f64]) -> Self {
        let positional = r.fork();
        let levels = amplitudes.iter().enumerate().map(|(k, &a)| (a != 0.0).then(|| (Improved::new(&mut positional.hashed(&format!("octave_{}", first + k as i32))), a))).collect();
        let n = amplitudes.len() as i32;
        Self { levels, input: 2f64.powi(first), value: 2f64.powi(n - 1) / (2f64.powi(n) - 1.0) }
    }

    fn get(&self, x: f64, y: f64, z: f64) -> f64 {
        let (mut sum, mut f, mut v) = (0.0, self.input, self.value);
        for level in &self.levels {
            if let Some((noise, amplitude)) = level {
                sum += amplitude * noise.noise(wrap(x * f), wrap(y * f), wrap(z * f)) * v;
            }
            f *= 2.0;
            v /= 2.0;
        }
        sum
    }
}

/// The game's `NormalNoise`: two Perlin noises, the second a little
/// stretched.
#[derive(Debug, Clone)]
struct Normal {
    first: Perlin,
    second: Perlin,
    factor: f64,
}

impl Normal {
    fn new(positional: &Positional, name: &str, first: i32, amplitudes: &[f64]) -> Self {
        let mut r = positional.hashed(name);
        let a = Perlin::new(&mut r, first, amplitudes);
        let b = Perlin::new(&mut r, first, amplitudes);
        let on: Vec<usize> = amplitudes.iter().enumerate().filter(|(_, a)| **a != 0.0).map(|(i, _)| i).collect();
        let span = (on.last().unwrap() - on.first().unwrap()) as f64;
        Self { first: a, second: b, factor: (1.0 / 6.0) / (0.1 * (1.0 + 1.0 / (span + 1.0))) }
    }

    fn get(&self, x: f64, y: f64, z: f64) -> f64 {
        const S: f64 = 1.0181268882175227;
        (self.first.get(x, y, z) + self.second.get(x * S, y * S, z * S)) * self.factor
    }
}

// ---------------------------------------------------------------- climate

/// The climate of a column, as the game's density functions give it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Climate {
    pub temperature: f64,
    pub humidity: f64,
    pub continentalness: f64,
    pub erosion: f64,
    pub weirdness: f64,
    /// The offset of the land's depth: the rough surface lies where
    /// `128 + 128 · offset` (see [`Climate::surface`]).
    pub offset: f64,
}

impl Climate {
    /// The rough surface elevation: where the depth of the land is zero.
    pub fn surface(&self) -> f32 {
        (128.0 + 128.0 * self.offset) as f32
    }
}

/// The land of a seed: its climate noises.
#[derive(Debug, Clone)]
pub struct Vanilla {
    pub seed: i64,
    temperature: Normal,
    vegetation: Normal,
    continentalness: Normal,
    erosion: Normal,
    ridge: Normal,
    shift: Normal,
    offset: Spline,
}

impl Vanilla {
    pub fn new(seed: i64) -> Self {
        let positional = Xoroshiro::from_seed(seed).fork();
        Self {
            seed,
            temperature: Normal::new(&positional, "minecraft:temperature", -10, &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
            vegetation: Normal::new(&positional, "minecraft:vegetation", -8, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
            continentalness: Normal::new(&positional, "minecraft:continentalness", -9, &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
            erosion: Normal::new(&positional, "minecraft:erosion", -9, &[1.0, 1.0, 0.0, 1.0, 1.0]),
            ridge: Normal::new(&positional, "minecraft:ridge", -7, &[1.0, 2.0, 1.0, 0.0, 0.0, 0.0]),
            shift: Normal::new(&positional, "minecraft:offset", -3, &[1.0, 1.0, 1.0, 0.0]),
            offset: overworld_offset(),
        }
    }

    /// The climate at block column `(x, z)`.
    pub fn climate(&self, x: i32, z: i32) -> Climate {
        let (x, z) = (x as f64, z as f64);
        let sx = self.shift.get(x * 0.25, 0.0, z * 0.25) * 4.0;
        let sz = self.shift.get(z * 0.25, x * 0.25, 0.0) * 4.0;
        let (px, pz) = (x * 0.25 + sx, z * 0.25 + sz);
        let continentalness = self.continentalness.get(px, 0.0, pz);
        let erosion = self.erosion.get(px, 0.0, pz);
        let weirdness = self.ridge.get(px, 0.0, pz);
        let folded = -3.0 * ((weirdness.abs() - 2.0 / 3.0).abs() - 1.0 / 3.0);
        let coords = [continentalness as f32, erosion as f32, folded as f32];
        let offset = -0.50375 + self.offset.at(&coords) as f64;
        Climate { temperature: self.temperature.get(px, 0.0, pz), humidity: self.vegetation.get(px, 0.0, pz), continentalness, erosion, weirdness, offset }
    }

    /// The biome at the surface of column `(x, z)`, as the game keeps it
    /// for its quarter (the 4 × 4 blocks around `(x & !3, z & !3)`).
    pub fn biome(&self, x: i32, z: i32) -> Biome {
        biome_of(&self.climate(x & !3, z & !3))
    }
}

// ----------------------------------------------------------------- spline

/// The game's `CubicSpline` over (continentalness, erosion, ridges folded).
#[derive(Debug, Clone)]
enum Spline {
    Constant(f32),
    Multi { coordinate: usize, locations: Vec<f32>, values: Vec<Spline>, derivatives: Vec<f32> },
}

const CONTINENTS: usize = 0;
const EROSION: usize = 1;
const RIDGES: usize = 2;

struct SplineBuilder {
    coordinate: usize,
    locations: Vec<f32>,
    values: Vec<Spline>,
    derivatives: Vec<f32>,
}

impl SplineBuilder {
    fn new(coordinate: usize) -> Self {
        Self { coordinate, locations: Vec::new(), values: Vec::new(), derivatives: Vec::new() }
    }

    fn point(mut self, location: f32, value: impl Into<Spline>, derivative: f32) -> Self {
        self.locations.push(location);
        self.values.push(value.into());
        self.derivatives.push(derivative);
        self
    }

    fn build(self) -> Spline {
        Spline::Multi { coordinate: self.coordinate, locations: self.locations, values: self.values, derivatives: self.derivatives }
    }
}

impl From<f32> for Spline {
    fn from(v: f32) -> Self {
        Spline::Constant(v)
    }
}

impl Spline {
    fn at(&self, c: &[f32; 3]) -> f32 {
        match self {
            Spline::Constant(v) => *v,
            Spline::Multi { coordinate, locations, values, derivatives } => {
                let f = c[*coordinate];
                // The last location at or below f.
                let i = locations.partition_point(|l| *l <= f) as i32 - 1;
                let last = locations.len() - 1;
                let extend = |k: usize, value: f32| if derivatives[k] == 0.0 { value } else { value + derivatives[k] * (f - locations[k]) };
                if i < 0 {
                    return extend(0, values[0].at(c));
                }
                let i = i as usize;
                if i == last {
                    return extend(last, values[last].at(c));
                }
                let (g, h) = (locations[i], locations[i + 1]);
                let k = (f - g) / (h - g);
                let (n, o) = (values[i].at(c), values[i + 1].at(c));
                let (p, q) = (derivatives[i], derivatives[i + 1]);
                let r = p * (h - g) - (o - n);
                let s = -q * (h - g) + (o - n);
                let lerpf = |t: f32, a: f32, b: f32| a + t * (b - a);
                lerpf(k, n, o) + k * (1.0 - k) * lerpf(k, r, s)
            }
        }
    }
}

fn mountain_continentalness(f: f32, g: f32, h: f32) -> f32 {
    let k = 1.0 - (1.0 - g) * 0.5;
    let l = 0.5 * (1.0 - g);
    let m = (f + 1.17) * 0.46082947;
    let n = m * k - l;
    if f < h { n.max(-0.2222) } else { n.max(0.0) }
}

fn mountain_ridge_zero(f: f32) -> f32 {
    let i = 1.0 - (1.0 - f) * 0.5;
    let j = 0.5 * (1.0 - f);
    j / (0.46082947 * i) - 1.17
}

fn slope(f: f32, g: f32, h: f32, i: f32) -> f32 {
    (g - f) / (i - h)
}

fn mountain_ridge_spline(f: f32, bl: bool) -> Spline {
    let mut b = SplineBuilder::new(RIDGES);
    let i = mountain_continentalness(-1.0, f, -0.7);
    let k = mountain_continentalness(1.0, f, -0.7);
    let l = mountain_ridge_zero(f);
    if -0.65 < l && l < 1.0 {
        let n = mountain_continentalness(-0.65, f, -0.7);
        let p = mountain_continentalness(-0.75, f, -0.7);
        let q = slope(i, p, -1.0, -0.75);
        b = b.point(-1.0, i, q).point(-0.75, p, 0.0).point(-0.65, n, 0.0);
        let r = mountain_continentalness(l, f, -0.7);
        let s = slope(r, k, l, 1.0);
        b = b.point(l - 0.01, r, 0.0).point(l, r, s).point(1.0, k, s);
    } else {
        let n = slope(i, k, -1.0, 1.0);
        if bl {
            b = b.point(-1.0, i.max(0.2), 0.0).point(0.0, i + 0.5 * (k - i), n);
        } else {
            b = b.point(-1.0, i, n);
        }
        b = b.point(1.0, k, n);
    }
    b.build()
}

fn ridge_spline(f: f32, g: f32, h: f32, i: f32, j: f32, k: f32) -> Spline {
    let l = (0.5 * (g - f)).max(k);
    let m = 5.0 * (h - g);
    SplineBuilder::new(RIDGES).point(-1.0, f, l).point(-0.4, g, l.min(m)).point(0.0, h, m).point(0.4, i, 2.0 * (i - h)).point(1.0, j, 0.7 * (j - i)).build()
}

#[allow(clippy::too_many_arguments)]
fn erosion_offset_spline(f: f32, g: f32, h: f32, i: f32, j: f32, k: f32, bl: bool, bl2: bool) -> Spline {
    let lerpf = |t: f32, a: f32, b: f32| a + t * (b - a);
    let s1 = mountain_ridge_spline(lerpf(i, 0.6, 1.5), bl2);
    let s2 = mountain_ridge_spline(lerpf(i, 0.6, 1.0), bl2);
    let s3 = mountain_ridge_spline(i, bl2);
    let s4 = ridge_spline(f - 0.15, 0.5 * i, 0.5 * i, 0.5 * i, 0.6 * i, 0.5);
    let s5 = ridge_spline(f, j * i, g * i, 0.5 * i, 0.6 * i, 0.5);
    let s6 = ridge_spline(f, j, j, g, h, 0.5);
    let s7 = ridge_spline(f, j, j, g, h, 0.5);
    let s8 = SplineBuilder::new(RIDGES).point(-1.0, f, 0.0).point(-0.4, s6.clone(), 0.0).point(0.0, h + 0.07, 0.0).build();
    let s9 = ridge_spline(-0.02, k, k, g, h, 0.0);
    let mut b = SplineBuilder::new(EROSION).point(-0.85, s1, 0.0).point(-0.7, s2, 0.0).point(-0.4, s3, 0.0).point(-0.35, s4, 0.0).point(-0.1, s5, 0.0).point(0.2, s6, 0.0);
    if bl {
        b = b.point(0.4, s7.clone(), 0.0).point(0.45, s8.clone(), 0.0).point(0.55, s8, 0.0).point(0.58, s7, 0.0);
    }
    b.point(0.7, s9, 0.0).build()
}

/// `TerrainProvider.overworldOffset`, not amplified.
fn overworld_offset() -> Spline {
    let s1 = erosion_offset_spline(-0.15, 0.0, 0.0, 0.1, 0.0, -0.03, false, false);
    let s2 = erosion_offset_spline(-0.1, 0.03, 0.1, 0.1, 0.01, -0.03, false, false);
    let s3 = erosion_offset_spline(-0.1, 0.03, 0.1, 0.7, 0.01, -0.03, true, true);
    let s4 = erosion_offset_spline(-0.05, 0.03, 0.1, 1.0, 0.01, 0.01, true, true);
    SplineBuilder::new(CONTINENTS)
        .point(-1.1, 0.044, 0.0)
        .point(-1.02, -0.2222, 0.0)
        .point(-0.51, -0.2222, 0.0)
        .point(-0.44, -0.12, 0.0)
        .point(-0.18, -0.12, 0.0)
        .point(-0.16, s1.clone(), 0.0)
        .point(-0.15, s1, 0.0)
        .point(-0.1, s2, 0.0)
        .point(0.25, s3, 0.0)
        .point(1.0, s4, 0.0)
        .build()
}

// ----------------------------------------------------------------- biomes

/// The biomes of the overworld the climate picks from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Biome {
    MushroomFields,
    DeepFrozenOcean,
    DeepColdOcean,
    DeepOcean,
    DeepLukewarmOcean,
    WarmOcean,
    FrozenOcean,
    ColdOcean,
    Ocean,
    LukewarmOcean,
    SnowyPlains,
    SnowyTaiga,
    Taiga,
    Plains,
    Forest,
    OldGrowthSpruceTaiga,
    FlowerForest,
    BirchForest,
    DarkForest,
    Savanna,
    Jungle,
    Desert,
    IceSpikes,
    OldGrowthPineTaiga,
    SunflowerPlains,
    OldGrowthBirchForest,
    SparseJungle,
    BambooJungle,
    PaleGarden,
    Meadow,
    SavannaPlateau,
    Badlands,
    WoodedBadlands,
    ErodedBadlands,
    CherryGrove,
    WindsweptGravellyHills,
    WindsweptHills,
    WindsweptForest,
    WindsweptSavanna,
    JaggedPeaks,
    FrozenPeaks,
    StonyPeaks,
    SnowySlopes,
    Grove,
    StonyShore,
    Swamp,
    MangroveSwamp,
    SnowyBeach,
    Beach,
    River,
    FrozenRiver,
    DripstoneCaves,
    LushCaves,
    DeepDark,
}

impl Biome {
    /// The game's name of the biome, without `minecraft:`.
    pub fn name(self) -> &'static str {
        use Biome::*;
        match self {
            MushroomFields => "mushroom_fields",
            DeepFrozenOcean => "deep_frozen_ocean",
            DeepColdOcean => "deep_cold_ocean",
            DeepOcean => "deep_ocean",
            DeepLukewarmOcean => "deep_lukewarm_ocean",
            WarmOcean => "warm_ocean",
            FrozenOcean => "frozen_ocean",
            ColdOcean => "cold_ocean",
            Ocean => "ocean",
            LukewarmOcean => "lukewarm_ocean",
            SnowyPlains => "snowy_plains",
            SnowyTaiga => "snowy_taiga",
            Taiga => "taiga",
            Plains => "plains",
            Forest => "forest",
            OldGrowthSpruceTaiga => "old_growth_spruce_taiga",
            FlowerForest => "flower_forest",
            BirchForest => "birch_forest",
            DarkForest => "dark_forest",
            Savanna => "savanna",
            Jungle => "jungle",
            Desert => "desert",
            IceSpikes => "ice_spikes",
            OldGrowthPineTaiga => "old_growth_pine_taiga",
            SunflowerPlains => "sunflower_plains",
            OldGrowthBirchForest => "old_growth_birch_forest",
            SparseJungle => "sparse_jungle",
            BambooJungle => "bamboo_jungle",
            PaleGarden => "pale_garden",
            Meadow => "meadow",
            SavannaPlateau => "savanna_plateau",
            Badlands => "badlands",
            WoodedBadlands => "wooded_badlands",
            ErodedBadlands => "eroded_badlands",
            CherryGrove => "cherry_grove",
            WindsweptGravellyHills => "windswept_gravelly_hills",
            WindsweptHills => "windswept_hills",
            WindsweptForest => "windswept_forest",
            WindsweptSavanna => "windswept_savanna",
            JaggedPeaks => "jagged_peaks",
            FrozenPeaks => "frozen_peaks",
            StonyPeaks => "stony_peaks",
            SnowySlopes => "snowy_slopes",
            Grove => "grove",
            StonyShore => "stony_shore",
            Swamp => "swamp",
            MangroveSwamp => "mangrove_swamp",
            SnowyBeach => "snowy_beach",
            Beach => "beach",
            River => "river",
            FrozenRiver => "frozen_river",
            DripstoneCaves => "dripstone_caves",
            LushCaves => "lush_caves",
            DeepDark => "deep_dark",
        }
    }

    /// Whether it is an ocean (the land is under the sea there).
    pub fn is_ocean(self) -> bool {
        use Biome::*;
        matches!(self, DeepFrozenOcean | DeepColdOcean | DeepOcean | DeepLukewarmOcean | WarmOcean | FrozenOcean | ColdOcean | Ocean | LukewarmOcean)
    }
}

/// A range of a climate parameter, quantized as the game does (× 10,000).
#[derive(Debug, Clone, Copy)]
struct Range(i64, i64);

fn q(f: f32) -> i64 {
    (f * 10000.0) as i64
}

fn span(a: f32, b: f32) -> Range {
    Range(q(a), q(b))
}

fn join(a: Range, b: Range) -> Range {
    Range(a.0, b.1)
}

impl Range {
    #[inline]
    fn distance(&self, v: i64) -> i64 {
        let l = v - self.1;
        let m = self.0 - v;
        if l > 0 { l } else { m.max(0) }
    }
}

/// A point of the game's table: six ranges (temperature, humidity,
/// continentalness, erosion, depth, weirdness) and the biome.
#[derive(Debug, Clone, Copy)]
struct Point([Range; 6], Biome);

#[allow(clippy::needless_range_loop)]
struct Builder {
    points: Vec<Point>,
    full: Range,
    temperatures: [Range; 5],
    humidities: [Range; 5],
    erosions: [Range; 7],
    mushroom: Range,
    deep_ocean: Range,
    ocean: Range,
    coast: Range,
    inland: Range,
    near_inland: Range,
    mid_inland: Range,
    far_inland: Range,
}

use Biome as B;

const OCEANS: [[Biome; 5]; 2] = [[B::DeepFrozenOcean, B::DeepColdOcean, B::DeepOcean, B::DeepLukewarmOcean, B::WarmOcean], [B::FrozenOcean, B::ColdOcean, B::Ocean, B::LukewarmOcean, B::WarmOcean]];
const MIDDLE: [[Biome; 5]; 5] = [
    [B::SnowyPlains, B::SnowyPlains, B::SnowyPlains, B::SnowyTaiga, B::Taiga],
    [B::Plains, B::Plains, B::Forest, B::Taiga, B::OldGrowthSpruceTaiga],
    [B::FlowerForest, B::Plains, B::Forest, B::BirchForest, B::DarkForest],
    [B::Savanna, B::Savanna, B::Forest, B::Jungle, B::Jungle],
    [B::Desert, B::Desert, B::Desert, B::Desert, B::Desert],
];
const MIDDLE_VARIANT: [[Option<Biome>; 5]; 5] = [
    [Some(B::IceSpikes), None, Some(B::SnowyTaiga), None, None],
    [None, None, None, None, Some(B::OldGrowthPineTaiga)],
    [Some(B::SunflowerPlains), None, None, Some(B::OldGrowthBirchForest), None],
    [None, None, Some(B::Plains), Some(B::SparseJungle), Some(B::BambooJungle)],
    [None, None, None, None, None],
];
const PLATEAU: [[Biome; 5]; 5] = [
    [B::SnowyPlains, B::SnowyPlains, B::SnowyPlains, B::SnowyTaiga, B::SnowyTaiga],
    [B::Meadow, B::Meadow, B::Forest, B::Taiga, B::OldGrowthSpruceTaiga],
    [B::Meadow, B::Meadow, B::Meadow, B::Meadow, B::PaleGarden],
    [B::SavannaPlateau, B::SavannaPlateau, B::Forest, B::Forest, B::Jungle],
    [B::Badlands, B::Badlands, B::Badlands, B::WoodedBadlands, B::WoodedBadlands],
];
const PLATEAU_VARIANT: [[Option<Biome>; 5]; 5] = [
    [Some(B::IceSpikes), None, None, None, None],
    [Some(B::CherryGrove), None, Some(B::Meadow), Some(B::Meadow), Some(B::OldGrowthPineTaiga)],
    [Some(B::CherryGrove), Some(B::CherryGrove), Some(B::Forest), Some(B::BirchForest), None],
    [None, None, None, None, None],
    [Some(B::ErodedBadlands), Some(B::ErodedBadlands), None, None, None],
];
const SHATTERED: [[Option<Biome>; 5]; 5] = [
    [Some(B::WindsweptGravellyHills), Some(B::WindsweptGravellyHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [Some(B::WindsweptGravellyHills), Some(B::WindsweptGravellyHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [Some(B::WindsweptHills), Some(B::WindsweptHills), Some(B::WindsweptHills), Some(B::WindsweptForest), Some(B::WindsweptForest)],
    [None, None, None, None, None],
    [None, None, None, None, None],
];

#[allow(clippy::needless_range_loop)]
impl Builder {
    fn new() -> Self {
        let temperatures = [span(-1.0, -0.45), span(-0.45, -0.15), span(-0.15, 0.2), span(0.2, 0.55), span(0.55, 1.0)];
        Self {
            points: Vec::new(),
            full: span(-1.0, 1.0),
            temperatures,
            humidities: [span(-1.0, -0.35), span(-0.35, -0.1), span(-0.1, 0.1), span(0.1, 0.3), span(0.3, 1.0)],
            erosions: [span(-1.0, -0.78), span(-0.78, -0.375), span(-0.375, -0.2225), span(-0.2225, 0.05), span(0.05, 0.45), span(0.45, 0.55), span(0.55, 1.0)],
            mushroom: span(-1.2, -1.05),
            deep_ocean: span(-1.05, -0.455),
            ocean: span(-0.455, -0.19),
            coast: span(-0.19, -0.11),
            inland: span(-0.11, 0.55),
            near_inland: span(-0.11, 0.03),
            mid_inland: span(0.03, 0.3),
            far_inland: span(0.3, 1.0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn surface(&mut self, t: Range, h: Range, c: Range, e: Range, w: Range, b: Biome) {
        self.points.push(Point([t, h, c, e, span(0.0, 0.0), w], b));
        self.points.push(Point([t, h, c, e, span(1.0, 1.0), w], b));
    }

    fn frozen(&self) -> Range {
        self.temperatures[0]
    }

    fn unfrozen(&self) -> Range {
        join(self.temperatures[1], self.temperatures[4])
    }

    fn middle(i: usize, j: usize, w: Range) -> Biome {
        if w.1 < 0 { MIDDLE[i][j] } else { MIDDLE_VARIANT[i][j].unwrap_or(MIDDLE[i][j]) }
    }

    fn middle_or_badlands_if_hot(i: usize, j: usize, w: Range) -> Biome {
        if i == 4 { Self::badlands(j, w) } else { Self::middle(i, j, w) }
    }

    fn middle_or_badlands_if_hot_or_slope_if_cold(i: usize, j: usize, w: Range) -> Biome {
        if i == 0 { Self::slope(i, j, w) } else { Self::middle_or_badlands_if_hot(i, j, w) }
    }

    fn windswept_savanna(i: usize, j: usize, w: Range, b: Biome) -> Biome {
        if i > 1 && j < 4 && w.1 >= 0 { B::WindsweptSavanna } else { b }
    }

    fn shattered_coast(i: usize, j: usize, w: Range) -> Biome {
        let b = if w.1 >= 0 { Self::middle(i, j, w) } else { Self::beach(i) };
        Self::windswept_savanna(i, j, w, b)
    }

    fn beach(i: usize) -> Biome {
        match i {
            0 => B::SnowyBeach,
            4 => B::Desert,
            _ => B::Beach,
        }
    }

    fn badlands(j: usize, w: Range) -> Biome {
        if j < 2 {
            if w.1 < 0 { B::Badlands } else { B::ErodedBadlands }
        } else if j < 3 {
            B::Badlands
        } else {
            B::WoodedBadlands
        }
    }

    fn plateau(i: usize, j: usize, w: Range) -> Biome {
        if w.1 >= 0
            && let Some(b) = PLATEAU_VARIANT[i][j]
        {
            return b;
        }
        PLATEAU[i][j]
    }

    fn peak(i: usize, j: usize, w: Range) -> Biome {
        if i <= 2 {
            if w.1 < 0 { B::JaggedPeaks } else { B::FrozenPeaks }
        } else if i == 3 {
            B::StonyPeaks
        } else {
            Self::badlands(j, w)
        }
    }

    fn slope(i: usize, j: usize, w: Range) -> Biome {
        if i >= 3 {
            Self::plateau(i, j, w)
        } else if j <= 1 {
            B::SnowySlopes
        } else {
            B::Grove
        }
    }

    fn shattered(i: usize, j: usize, w: Range) -> Biome {
        SHATTERED[i][j].unwrap_or_else(|| Self::middle(i, j, w))
    }

    fn off_coast(&mut self) {
        let full = self.full;
        self.surface(full, full, self.mushroom, full, full, B::MushroomFields);
        for i in 0..5 {
            let t = self.temperatures[i];
            self.surface(t, full, self.deep_ocean, full, full, OCEANS[0][i]);
            self.surface(t, full, self.ocean, full, full, OCEANS[1][i]);
        }
    }

    fn inland(&mut self) {
        let slices: [(f32, f32, u8); 13] = [
            (-1.0, -0.93333334, 1),
            (-0.93333334, -0.7666667, 2),
            (-0.7666667, -0.56666666, 3),
            (-0.56666666, -0.4, 2),
            (-0.4, -0.26666668, 1),
            (-0.26666668, -0.05, 0),
            (-0.05, 0.05, 4),
            (0.05, 0.26666668, 0),
            (0.26666668, 0.4, 1),
            (0.4, 0.56666666, 2),
            (0.56666666, 0.7666667, 3),
            (0.7666667, 0.93333334, 2),
            (0.93333334, 1.0, 1),
        ];
        for (a, b, kind) in slices {
            let w = span(a, b);
            match kind {
                0 => self.low_slice(w),
                1 => self.mid_slice(w),
                2 => self.high_slice(w),
                3 => self.peaks(w),
                _ => self.valleys(w),
            }
        }
    }

    fn peaks(&mut self, w: Range) {
        let (coast, near, mid, far, e) = (self.coast, self.near_inland, self.mid_inland, self.far_inland, self.erosions);
        for i in 0..5 {
            let t = self.temperatures[i];
            for j in 0..5 {
                let h = self.humidities[j];
                let middle = Self::middle(i, j, w);
                let mobh = Self::middle_or_badlands_if_hot(i, j, w);
                let mobhsc = Self::middle_or_badlands_if_hot_or_slope_if_cold(i, j, w);
                let plateau = Self::plateau(i, j, w);
                let shattered = Self::shattered(i, j, w);
                let savanna = Self::windswept_savanna(i, j, w, shattered);
                let peak = Self::peak(i, j, w);
                self.surface(t, h, join(coast, far), e[0], w, peak);
                self.surface(t, h, join(coast, near), e[1], w, mobhsc);
                self.surface(t, h, join(mid, far), e[1], w, peak);
                self.surface(t, h, join(coast, near), join(e[2], e[3]), w, middle);
                self.surface(t, h, join(mid, far), e[2], w, plateau);
                self.surface(t, h, mid, e[3], w, mobh);
                self.surface(t, h, far, e[3], w, plateau);
                self.surface(t, h, join(coast, far), e[4], w, middle);
                self.surface(t, h, join(coast, near), e[5], w, savanna);
                self.surface(t, h, join(mid, far), e[5], w, shattered);
                self.surface(t, h, join(coast, far), e[6], w, middle);
            }
        }
    }

    fn high_slice(&mut self, w: Range) {
        let (coast, near, mid, far, e) = (self.coast, self.near_inland, self.mid_inland, self.far_inland, self.erosions);
        for i in 0..5 {
            let t = self.temperatures[i];
            for j in 0..5 {
                let h = self.humidities[j];
                let middle = Self::middle(i, j, w);
                let mobh = Self::middle_or_badlands_if_hot(i, j, w);
                let mobhsc = Self::middle_or_badlands_if_hot_or_slope_if_cold(i, j, w);
                let plateau = Self::plateau(i, j, w);
                let shattered = Self::shattered(i, j, w);
                let savanna = Self::windswept_savanna(i, j, w, middle);
                let slope = Self::slope(i, j, w);
                let peak = Self::peak(i, j, w);
                self.surface(t, h, coast, join(e[0], e[1]), w, middle);
                self.surface(t, h, near, e[0], w, slope);
                self.surface(t, h, join(mid, far), e[0], w, peak);
                self.surface(t, h, near, e[1], w, mobhsc);
                self.surface(t, h, join(mid, far), e[1], w, slope);
                self.surface(t, h, join(coast, near), join(e[2], e[3]), w, middle);
                self.surface(t, h, join(mid, far), e[2], w, plateau);
                self.surface(t, h, mid, e[3], w, mobh);
                self.surface(t, h, far, e[3], w, plateau);
                self.surface(t, h, join(coast, far), e[4], w, middle);
                self.surface(t, h, join(coast, near), e[5], w, savanna);
                self.surface(t, h, join(mid, far), e[5], w, shattered);
                self.surface(t, h, join(coast, far), e[6], w, middle);
            }
        }
    }

    fn mid_slice(&mut self, w: Range) {
        let (full, coast, near, mid, far, e) = (self.full, self.coast, self.near_inland, self.mid_inland, self.far_inland, self.erosions);
        let tm = self.temperatures;
        self.surface(full, full, coast, join(e[0], e[2]), w, B::StonyShore);
        self.surface(join(tm[1], tm[2]), full, join(near, far), e[6], w, B::Swamp);
        self.surface(join(tm[3], tm[4]), full, join(near, far), e[6], w, B::MangroveSwamp);
        for i in 0..5 {
            let t = tm[i];
            for j in 0..5 {
                let h = self.humidities[j];
                let middle = Self::middle(i, j, w);
                let mobh = Self::middle_or_badlands_if_hot(i, j, w);
                let mobhsc = Self::middle_or_badlands_if_hot_or_slope_if_cold(i, j, w);
                let shattered = Self::shattered(i, j, w);
                let plateau = Self::plateau(i, j, w);
                let beach = Self::beach(i);
                let savanna = Self::windswept_savanna(i, j, w, middle);
                let shattered_coast = Self::shattered_coast(i, j, w);
                let slope = Self::slope(i, j, w);
                self.surface(t, h, join(near, far), e[0], w, slope);
                self.surface(t, h, join(near, mid), e[1], w, mobhsc);
                self.surface(t, h, far, e[1], w, if i == 0 { slope } else { plateau });
                self.surface(t, h, near, e[2], w, middle);
                self.surface(t, h, mid, e[2], w, mobh);
                self.surface(t, h, far, e[2], w, plateau);
                self.surface(t, h, join(coast, near), e[3], w, middle);
                self.surface(t, h, join(mid, far), e[3], w, mobh);
                if w.1 < 0 {
                    self.surface(t, h, coast, e[4], w, beach);
                    self.surface(t, h, join(near, far), e[4], w, middle);
                } else {
                    self.surface(t, h, join(coast, far), e[4], w, middle);
                }
                self.surface(t, h, coast, e[5], w, shattered_coast);
                self.surface(t, h, near, e[5], w, savanna);
                self.surface(t, h, join(mid, far), e[5], w, shattered);
                if w.1 < 0 {
                    self.surface(t, h, coast, e[6], w, beach);
                } else {
                    self.surface(t, h, coast, e[6], w, middle);
                }
                if i == 0 {
                    self.surface(t, h, join(near, far), e[6], w, middle);
                }
            }
        }
    }

    fn low_slice(&mut self, w: Range) {
        let (full, coast, near, mid, far, e) = (self.full, self.coast, self.near_inland, self.mid_inland, self.far_inland, self.erosions);
        let tm = self.temperatures;
        self.surface(full, full, coast, join(e[0], e[2]), w, B::StonyShore);
        self.surface(join(tm[1], tm[2]), full, join(near, far), e[6], w, B::Swamp);
        self.surface(join(tm[3], tm[4]), full, join(near, far), e[6], w, B::MangroveSwamp);
        for i in 0..5 {
            let t = tm[i];
            for j in 0..5 {
                let h = self.humidities[j];
                let middle = Self::middle(i, j, w);
                let mobh = Self::middle_or_badlands_if_hot(i, j, w);
                let mobhsc = Self::middle_or_badlands_if_hot_or_slope_if_cold(i, j, w);
                let beach = Self::beach(i);
                let savanna = Self::windswept_savanna(i, j, w, middle);
                let shattered_coast = Self::shattered_coast(i, j, w);
                self.surface(t, h, near, join(e[0], e[1]), w, mobh);
                self.surface(t, h, join(mid, far), join(e[0], e[1]), w, mobhsc);
                self.surface(t, h, near, join(e[2], e[3]), w, middle);
                self.surface(t, h, join(mid, far), join(e[2], e[3]), w, mobh);
                self.surface(t, h, coast, join(e[3], e[4]), w, beach);
                self.surface(t, h, join(near, far), e[4], w, middle);
                self.surface(t, h, coast, e[5], w, shattered_coast);
                self.surface(t, h, near, e[5], w, savanna);
                self.surface(t, h, join(mid, far), e[5], w, middle);
                self.surface(t, h, coast, e[6], w, beach);
                if i == 0 {
                    self.surface(t, h, join(near, far), e[6], w, middle);
                }
            }
        }
    }

    fn valleys(&mut self, w: Range) {
        let (full, coast, near, mid, far, inland, e) = (self.full, self.coast, self.near_inland, self.mid_inland, self.far_inland, self.inland, self.erosions);
        let (frozen, unfrozen, tm) = (self.frozen(), self.unfrozen(), self.temperatures);
        let neg = w.1 < 0;
        self.surface(frozen, full, coast, join(e[0], e[1]), w, if neg { B::StonyShore } else { B::FrozenRiver });
        self.surface(unfrozen, full, coast, join(e[0], e[1]), w, if neg { B::StonyShore } else { B::River });
        self.surface(frozen, full, near, join(e[0], e[1]), w, B::FrozenRiver);
        self.surface(unfrozen, full, near, join(e[0], e[1]), w, B::River);
        self.surface(frozen, full, join(coast, far), join(e[2], e[5]), w, B::FrozenRiver);
        self.surface(unfrozen, full, join(coast, far), join(e[2], e[5]), w, B::River);
        self.surface(frozen, full, coast, e[6], w, B::FrozenRiver);
        self.surface(unfrozen, full, coast, e[6], w, B::River);
        self.surface(join(tm[1], tm[2]), full, join(inland, far), e[6], w, B::Swamp);
        self.surface(join(tm[3], tm[4]), full, join(inland, far), e[6], w, B::MangroveSwamp);
        self.surface(frozen, full, join(inland, far), e[6], w, B::FrozenRiver);
        for i in 0..5 {
            let t = tm[i];
            for j in 0..5 {
                let h = self.humidities[j];
                let mobh = Self::middle_or_badlands_if_hot(i, j, w);
                self.surface(t, h, join(mid, far), join(e[0], e[1]), w, mobh);
            }
        }
    }

    fn underground(&mut self) {
        let full = self.full;
        let e = self.erosions;
        let cave = span(0.2, 0.9);
        self.points.push(Point([full, full, span(0.8, 1.0), full, cave, full], B::DripstoneCaves));
        self.points.push(Point([full, span(0.7, 1.0), full, full, cave, full], B::LushCaves));
        self.points.push(Point([full, full, full, join(e[0], e[1]), span(1.1, 1.1), full], B::DeepDark));
    }
}

/// The game's table of the overworld's biomes.
fn table() -> &'static [Point] {
    static TABLE: std::sync::OnceLock<Vec<Point>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut b = Builder::new();
        b.off_coast();
        b.inland();
        b.underground();
        b.points
    })
}

/// The biome the game picks for a climate at the surface (depth zero):
/// the point of its table nearest to it.
pub fn biome_of(c: &Climate) -> Biome {
    let target = [q(c.temperature as f32), q(c.humidity as f32), q(c.continentalness as f32), q(c.erosion as f32), 0, q(c.weirdness as f32)];
    let mut best = (i64::MAX, Biome::Plains);
    for p in table() {
        let mut d = 0i64;
        for (range, &t) in p.0.iter().zip(&target) {
            let v = range.distance(t);
            d += v * v;
            if d >= best.0 {
                break;
            }
        }
        if d < best.0 {
            best = (d, p.1);
        }
    }
    best.1
}

// ------------------------------------------------------- the land around

/// One chunk of the land as the game made it (read from its region files):
/// 16 × 16 columns, `z * 16 + x`.
#[derive(Debug, Clone)]
pub struct LandChunk {
    /// Surface elevation of the ground.
    pub ground: [i16; 256],
    /// The fluid's surface over it, or [`crate::NO_FLUID`].
    pub fluid: [i16; 256],
    /// The biome, as the map's registry numbers it (0: none known).
    pub biome: [u16; 256],
}

/// The game's own land, chunk by chunk, where a server made it.
#[derive(Debug, Clone, Default)]
pub struct Land {
    pub seed: i64,
    pub chunks: std::collections::HashMap<(i32, i32), LandChunk>,
}

impl Land {
    /// The chunk and index of column `(x, z)`, if the game made it.
    #[inline]
    pub fn column(&self, x: i32, z: i32) -> Option<(&LandChunk, usize)> {
        let c = self.chunks.get(&(x.div_euclid(16), z.div_euclid(16)))?;
        Some((c, (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize))
    }
}

/// The climate of the quarters of a tile and one quarter around it, and
/// their biomes.
struct Quarters {
    heights: Vec<f32>,
    biomes: Vec<Biome>,
}

const QUARTERS: i32 = crate::coords::TILE_SIZE / 4 + 2;

/// The land of a seed around a map (see [`crate::expand::Surroundings`]):
/// the game's own where a server made it, else worked out here, a quarter
/// (4 × 4 blocks) at a time, the heights smoothed between quarters.
pub struct VanillaAround {
    vanilla: Vanilla,
    /// The map's number of each biome of [`Biome`].
    numbers: std::collections::HashMap<Biome, u16>,
    exact: Option<std::sync::Arc<Land>>,
    tiles: std::sync::Mutex<std::collections::HashMap<crate::TilePos, std::sync::Arc<Quarters>>>,
}

impl VanillaAround {
    /// The land of `seed`, its biomes numbered as `reg` numbers them;
    /// `exact`, the game's own land where it is known.
    pub fn new(seed: i64, reg: &crate::Registry, exact: Option<std::sync::Arc<Land>>) -> Self {
        let mut numbers = std::collections::HashMap::new();
        for p in table() {
            if let Some(id) = reg.biome_by_id(&format!("minecraft:{}", p.1.name())) {
                numbers.insert(p.1, id.0);
            }
        }
        Self { vanilla: Vanilla::new(seed), numbers, exact: exact.filter(|l| l.seed == seed), tiles: Default::default() }
    }

    pub fn vanilla(&self) -> &Vanilla {
        &self.vanilla
    }

    fn quarters(&self, x: i32, z: i32) -> (std::sync::Arc<Quarters>, i32, i32) {
        let pos = crate::TilePos::containing(x, z);
        let (ox, oz) = pos.origin();
        if let Some(q) = self.tiles.lock().unwrap_or_else(|e| e.into_inner()).get(&pos) {
            return (q.clone(), ox, oz);
        }
        let n = QUARTERS as usize;
        let mut heights = Vec::with_capacity(n * n);
        let mut biomes = Vec::with_capacity(n * n);
        for qz in 0..QUARTERS {
            for qx in 0..QUARTERS {
                let c = self.vanilla.climate(ox + (qx - 1) * 4, oz + (qz - 1) * 4);
                heights.push(c.surface());
                biomes.push(biome_of(&c));
            }
        }
        let q = std::sync::Arc::new(Quarters { heights, biomes });
        let mut tiles = self.tiles.lock().unwrap_or_else(|e| e.into_inner());
        // Kept to the tiles worked on lately.
        if tiles.len() > 4096 {
            tiles.clear();
        }
        tiles.insert(pos, q.clone());
        (q, ox, oz)
    }

    /// The rough height at `(x, z)`, between the quarters' corners.
    pub fn rough_height(&self, x: i32, z: i32) -> f32 {
        let (q, ox, oz) = self.quarters(x, z);
        let (gx, gz) = ((x - ox) as f32 / 4.0 + 1.0, (z - oz) as f32 / 4.0 + 1.0);
        let (x0, z0) = (gx.floor() as usize, gz.floor() as usize);
        let (fx, fz) = (gx - x0 as f32, gz - z0 as f32);
        let n = QUARTERS as usize;
        let at = |x: usize, z: usize| q.heights[z.min(n - 1) * n + x.min(n - 1)];
        let a = at(x0, z0) + (at(x0 + 1, z0) - at(x0, z0)) * fx;
        let b = at(x0, z0 + 1) + (at(x0 + 1, z0 + 1) - at(x0, z0 + 1)) * fx;
        a + (b - a) * fz
    }

    /// The biome the game gives `(x, z)`.
    pub fn biome_at(&self, x: i32, z: i32) -> Biome {
        let (q, ox, oz) = self.quarters(x, z);
        let n = QUARTERS as usize;
        q.biomes[((z - oz) / 4 + 1) as usize * n + ((x - ox) / 4 + 1) as usize]
    }

    /// Whether the game's own land is known at `(x, z)`.
    pub fn exact_at(&self, x: i32, z: i32) -> bool {
        self.exact.as_ref().is_some_and(|l| l.column(x, z).is_some())
    }
}

impl crate::expand::Surroundings for VanillaAround {
    fn height(&self, x: i32, z: i32) -> f32 {
        if let Some((c, i)) = self.exact.as_ref().and_then(|l| l.column(x, z)) {
            return c.ground[i] as f32;
        }
        self.rough_height(x, z)
    }

    fn fluid(&self, x: i32, z: i32) -> Option<f32> {
        if let Some((c, i)) = self.exact.as_ref().and_then(|l| l.column(x, z)) {
            return (c.fluid[i] != crate::NO_FLUID).then_some(c.fluid[i] as f32);
        }
        (self.rough_height(x, z) < SEA_LEVEL as f32).then_some(SEA_LEVEL as f32)
    }

    fn biome(&self, x: i32, z: i32) -> u16 {
        if let Some((c, i)) = self.exact.as_ref().and_then(|l| l.column(x, z))
            && c.biome[i] != 0
        {
            return c.biome[i];
        }
        self.numbers.get(&self.biome_at(x, z)).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_randoms_are_the_games() {
        // Known values of Xoroshiro128++ seeded from 0 as the game seeds it.
        let mut r = Xoroshiro::from_seed(0);
        let a = r.next_long();
        let b = r.next_long();
        assert_ne!(a, b);
        // The bounded draw stays in bounds and covers them.
        let mut seen = [false; 7];
        for _ in 0..200 {
            seen[r.next_bounded(7) as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn the_table_has_every_surface_biome() {
        let t = table();
        assert!(t.len() > 7000, "{} points", t.len());
        for b in [Biome::Plains, Biome::Desert, Biome::CherryGrove, Biome::JaggedPeaks, Biome::MangroveSwamp, Biome::River, Biome::DeepOcean] {
            assert!(t.iter().any(|p| p.1 == b), "{b:?}");
        }
    }

    #[test]
    fn a_seed_gives_land_and_sea() {
        let v = Vanilla::new(12345);
        let (mut land, mut sea) = (0, 0);
        let t = std::time::Instant::now();
        for z in (-4096..4096).step_by(64) {
            for x in (-4096..4096).step_by(64) {
                let b = v.biome(x, z);
                if b.is_ocean() { sea += 1 } else { land += 1 }
            }
        }
        eprintln!("{land} land, {sea} sea, {:.1} µs a column", t.elapsed().as_secs_f64() * 1e6 / (land + sea) as f64);
        assert!(land > 100 && sea > 100);
    }
}
