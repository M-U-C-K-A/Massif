//! Small helpers: a stable hasher, and finding the name meant.

use std::hash::Hasher;

/// FNV-1a, 64 bits: the same hashes on every machine and every build, so
/// that a cache key or a seed never depends on where it was computed.
#[derive(Debug, Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        // A last mix: FNV's low bits are weak.
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// The stable hash of a string.
pub fn hash_str(s: &str) -> u64 {
    let mut h = Fnv::default();
    h.write(s.as_bytes());
    h.finish()
}

/// Mixes 32 bits well (Chris Wellons' "lowbias32").
#[inline]
pub fn mix32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// How many single-letter edits turn `a` into `b`.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = if ca == *cb { prev } else { 1 + prev.min(row[j]).min(row[j + 1]) };
            prev = cur;
        }
    }
    row[b.len()]
}

/// The name among `names` closest to `name`, when one is close enough to
/// be what was meant.
pub fn closest<'a>(name: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    names
        .into_iter()
        .map(|n| {
            let d = edit_distance(&lower, &n.to_ascii_lowercase());
            // The part after the family counts as a match too: "fbm" for "noise.fbm".
            let tail = n.rsplit('.').next().map_or(usize::MAX, |t| edit_distance(&lower, &t.to_ascii_lowercase()));
            (d.min(tail), n)
        })
        .filter(|(d, n)| *d <= (n.len() / 3).max(2))
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(closest("hieght", ["height", "base", "scale"]), Some("height"));
        assert_eq!(closest("fbm", ["noise.fbm", "adjust.levels"]), Some("noise.fbm"));
        assert_eq!(closest("zzz", ["height", "base"]), None);
        assert_eq!(hash_str("alps"), hash_str("alps"));
        assert_ne!(hash_str("alps"), hash_str("alpz"));
    }
}
