//! The draws that assemble a structure: seeded, so that the same seed at
//! the same place gives the same structure again.

/// A small, fast generator (SplitMix64), good enough for laying out
/// pieces and rotting stone bricks.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x6d61_7373_6966_2121)
    }

    /// A generator of its own for the block at `pos`, as the game's
    /// processors draw: the same at the same place for the same seed.
    pub fn at(seed: u64, pos: [i32; 3]) -> Self {
        let mut h = seed;
        for v in pos {
            h = mix(h ^ (v as u32 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        }
        Self(h)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }

    /// A whole number in `0..bound` (0 for a bound of 0).
    pub fn below(&mut self, bound: u32) -> u32 {
        if bound <= 1 {
            return 0;
        }
        // Lemire's method: no bias worth speaking of at these sizes.
        ((self.next_u64() >> 32) * bound as u64 >> 32) as u32
    }

    /// A whole number in `lo..=hi`.
    pub fn between(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    /// A number in `[0, 1)`.
    pub fn float(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.float() < p
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// One of `items`, if any.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { items.get(self.below(items.len() as u32) as usize) }
    }

    /// A generator drawn from this one, for a part of the work that should
    /// not change the draws of the rest.
    pub fn fork(&mut self) -> Rng {
        Rng(self.next_u64())
    }
}

#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_seeded_and_spread() {
        let (mut a, mut b) = (Rng::new(5), Rng::new(5));
        let xs: Vec<u32> = (0..50).map(|_| a.below(10)).collect();
        let ys: Vec<u32> = (0..50).map(|_| b.below(10)).collect();
        assert_eq!(xs, ys);
        assert!(xs.iter().all(|&x| x < 10));
        let seen: std::collections::HashSet<u32> = xs.iter().copied().collect();
        assert_eq!(seen.len(), 10);
        let mut r = Rng::new(1);
        let f: Vec<f32> = (0..1000).map(|_| r.float()).collect();
        assert!(f.iter().all(|&v| (0.0..1.0).contains(&v)));
        let mean = f.iter().sum::<f32>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert_eq!(Rng::at(3, [1, 2, 3]).next_u64(), Rng::at(3, [1, 2, 3]).next_u64());
        assert_ne!(Rng::at(3, [1, 2, 3]).next_u64(), Rng::at(3, [1, 2, 4]).next_u64());
        assert!((0..100).all(|_| (-3..=4).contains(&r.between(-3, 4))));
    }
}
