//! Where samples lie: the terrain, its levels of detail and its tiles.
//!
//! A terrain is a square of the world `size` blocks a side. At level of
//! detail `lod` there is a sample every `2^lod` blocks, sample `(i, j)` lying
//! at `(i·2^lod, j·2^lod)` blocks from the terrain's north-west corner: the
//! samples of a coarser level are samples of the finer ones, so a preview
//! that sharpens keeps the heights it showed. Samples are grouped in tiles
//! of [`TILE`] × [`TILE`]. Tiles exist past the terrain's edges too (the
//! neighbourhood of an edge sample reaches out of it), which procedural
//! nodes simply compute and nodes over the whole terrain clamp.

/// Samples a side of a tile.
pub const TILE: u32 = 256;

/// The finest level of detail is one sample per block; the coarsest kept
/// is one per 2^15 blocks.
pub const MAX_LOD: u8 = 15;

/// The square of the world a graph makes, and what all its nodes share.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Space {
    /// North-west corner in world blocks (where map inputs read).
    pub x0: i32,
    pub z0: i32,
    /// Side, in blocks.
    pub size: u32,
    /// The seed all randomness starts from.
    pub seed: u64,
    /// The world's sea level, in blocks.
    pub sea_level: f32,
}

impl Space {
    pub fn new(size: u32) -> Self {
        Self { x0: 0, z0: 0, size: size.max(1), seed: 0, sea_level: 62.0 }
    }

    /// Samples a side at `lod`: enough to reach the far edge.
    pub fn samples(&self, lod: u8) -> u32 {
        let step = 1u64 << lod;
        (self.size as u64).div_ceil(step).max(1) as u32
    }

    /// Blocks between samples at `lod`.
    pub fn spacing(lod: u8) -> f64 {
        (1u64 << lod) as f64
    }

    /// The finest level with at most `max` samples a side.
    pub fn lod_for(&self, max: u32) -> u8 {
        (0..=MAX_LOD).find(|&l| self.samples(l) <= max.max(1)).unwrap_or(MAX_LOD)
    }

    /// Tiles a side at `lod`.
    pub fn tiles(&self, lod: u8) -> u32 {
        self.samples(lod).div_ceil(TILE)
    }

    /// The whole terrain at `lod`, as a rectangle of samples.
    pub fn whole(&self, lod: u8) -> Rect {
        let n = self.samples(lod);
        Rect::new(0, 0, n, n)
    }
}

/// A rectangle of samples at some level of detail: `[x, x + w) × [z, z + h)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub z: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, z: i32, w: u32, h: u32) -> Self {
        Self { x, z, w, h }
    }

    /// The samples of tile `(tx, tz)`.
    pub const fn tile(tx: i32, tz: i32) -> Self {
        Self::new(tx * TILE as i32, tz * TILE as i32, TILE, TILE)
    }

    pub fn area(&self) -> usize {
        self.w as usize * self.h as usize
    }

    pub fn x1(&self) -> i32 {
        self.x + self.w as i32
    }

    pub fn z1(&self) -> i32 {
        self.z + self.h as i32
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Grown by `m` samples on every side.
    pub fn expand(&self, m: u32) -> Self {
        Self::new(self.x - m as i32, self.z - m as i32, self.w + 2 * m, self.h + 2 * m)
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        x >= self.x && x < self.x1() && z >= self.z && z < self.z1()
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x, z) = (self.x.max(o.x), self.z.max(o.z));
        let (x1, z1) = (self.x1().min(o.x1()), self.z1().min(o.z1()));
        Rect::new(x, z, (x1 - x).max(0) as u32, (z1 - z).max(0) as u32)
    }

    /// The tiles holding these samples, row by row.
    pub fn tiles(&self) -> Vec<(i32, i32)> {
        if self.is_empty() {
            return Vec::new();
        }
        let t = TILE as i32;
        let (tx0, tz0) = (self.x.div_euclid(t), self.z.div_euclid(t));
        let (tx1, tz1) = ((self.x1() - 1).div_euclid(t), (self.z1() - 1).div_euclid(t));
        let mut out = Vec::with_capacity(((tx1 - tx0 + 1) * (tz1 - tz0 + 1)) as usize);
        for tz in tz0..=tz1 {
            for tx in tx0..=tx1 {
                out.push((tx, tz));
            }
        }
        out
    }

    /// The samples at level `to` that cover these samples at level `from`,
    /// with what interpolating between them needs: one more sample past the
    /// last when `to` is coarser, so that every position lies between two
    /// samples.
    pub fn at_lod(&self, from: u8, to: u8) -> Rect {
        if from == to {
            return *self;
        }
        if to < from {
            // Finer: each sample becomes 2^(from - to) of them.
            let k = from - to;
            return Rect::new(self.x << k, self.z << k, self.w << k, self.h << k);
        }
        let k = to - from;
        let d = 1i64 << k;
        let lo = |v: i32| (v as i64).div_euclid(d) as i32;
        let hi = |v: i32| (v as i64 + d - 1).div_euclid(d) as i32;
        let (x0, z0) = (lo(self.x), lo(self.z));
        let (x1, z1) = (hi(self.x1() - 1) + 1, hi(self.z1() - 1) + 1);
        Rect::new(x0, z0, (x1 - x0) as u32, (z1 - z0) as u32)
    }
}

/// A tile of samples of one level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub lod: u8,
    pub x: i32,
    pub z: i32,
}

impl TileKey {
    pub const fn new(lod: u8, x: i32, z: i32) -> Self {
        Self { lod, x, z }
    }

    pub const fn rect(&self) -> Rect {
        Rect::tile(self.x, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_and_levels() {
        let s = Space::new(16384);
        assert_eq!(s.samples(0), 16384);
        assert_eq!(s.samples(6), 256);
        assert_eq!(s.tiles(0), 64);
        assert_eq!(s.lod_for(4096), 2);
        let odd = Space::new(1000);
        assert_eq!(odd.samples(3), 125);
        assert_eq!(odd.tiles(0), 4);
    }

    #[test]
    fn rects_across_levels() {
        let r = Rect::new(10, -3, 7, 5);
        assert_eq!(r.at_lod(2, 0), Rect::new(40, -12, 28, 20));
        // Samples 10..17 at level 0 lie between samples 2 and 4 at level 2.
        assert_eq!(Rect::new(10, 0, 7, 1).at_lod(0, 2), Rect::new(2, 0, 3, 1));
        assert_eq!(Rect::new(-5, 0, 2, 1).at_lod(0, 1), Rect::new(-3, 0, 2, 1));
        assert_eq!(Rect::new(-1, -1, 258, 2).tiles(), vec![(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0)]);
    }
}
