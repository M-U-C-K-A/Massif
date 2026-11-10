//! One 128×128 raster of a tile, stored adaptively.
//!
//! A plane is `Uniform` (one value, no allocation), `Rle` (split-stream runs:
//! `values[i]` covers cells `ends[i-1]..ends[i]`) or `Dense`. Reads work on
//! every representation — `get` is a binary search over the cumulative run
//! ends, O(log R), nothing is decompressed. The first write unpacks the plane
//! to `Dense`; [`Plane::compact`] packs it again once the edit is over.

use crate::coords::TILE_AREA;
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum Plane<T> {
    Uniform(T),
    Rle {
        /// Value of each run.
        values: Vec<T>,
        /// Exclusive end of each run, strictly increasing, last == `TILE_AREA`.
        ends: Vec<u16>,
    },
    Dense(Box<[T; TILE_AREA]>),
}

/// Keep a plane in RLE form only while it stays this small; beyond that a
/// dense array is both smaller-ish and much faster to edit.
const MAX_RLE_RUNS: usize = TILE_AREA / 8;

impl<T: Copy + PartialEq> Plane<T> {
    /// Build a dense plane from exactly `TILE_AREA` values.
    pub fn from_vec(v: Vec<T>) -> Self {
        let boxed: Box<[T; TILE_AREA]> = v
            .into_boxed_slice()
            .try_into()
            .unwrap_or_else(|_| panic!("a plane needs exactly {TILE_AREA} values"));
        Plane::Dense(boxed)
    }

    /// Build a plane from validated runs (see the `Rle` variant).
    pub fn from_runs(values: Vec<T>, ends: Vec<u16>) -> Self {
        debug_assert_eq!(values.len(), ends.len());
        debug_assert_eq!(ends.last().copied(), Some(TILE_AREA as u16));
        debug_assert!(ends.windows(2).all(|w| w[0] < w[1]));
        if values.len() == 1 {
            Plane::Uniform(values[0])
        } else {
            Plane::Rle { values, ends }
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> T {
        match self {
            Plane::Uniform(v) => *v,
            Plane::Rle { values, ends } => {
                let run = ends.partition_point(|&end| (end as usize) <= i);
                values[run]
            }
            Plane::Dense(d) => d[i],
        }
    }

    /// The single value of the plane, if it is stored as uniform.
    pub fn uniform_value(&self) -> Option<T> {
        match self {
            Plane::Uniform(v) => Some(*v),
            _ => None,
        }
    }

    /// Write one cell (unpack-on-write).
    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        match self {
            Plane::Dense(d) => d[i] = v,
            Plane::Uniform(u) if *u == v => {}
            _ => self.dense_mut()[i] = v,
        }
    }

    pub fn fill(&mut self, v: T) {
        *self = Plane::Uniform(v);
    }

    /// Unpack to a dense array and borrow it mutably.
    pub fn dense_mut(&mut self) -> &mut [T; TILE_AREA] {
        if !matches!(self, Plane::Dense(_)) {
            let mut v = Vec::with_capacity(TILE_AREA);
            self.for_each_run(|val, len| v.extend(std::iter::repeat_n(val, len)));
            *self = Plane::from_vec(v);
        }
        match self {
            Plane::Dense(d) => d,
            _ => unreachable!(),
        }
    }

    /// All cells as a slice: borrowed when dense, expanded otherwise.
    pub fn dense(&self) -> Cow<'_, [T]> {
        match self {
            Plane::Dense(d) => Cow::Borrowed(&d[..]),
            _ => {
                let mut v = Vec::with_capacity(TILE_AREA);
                self.for_each_run(|val, len| v.extend(std::iter::repeat_n(val, len)));
                Cow::Owned(v)
            }
        }
    }

    /// Visit the maximal runs of equal values in cell order.
    pub fn for_each_run(&self, mut f: impl FnMut(T, usize)) {
        match self {
            Plane::Uniform(v) => f(*v, TILE_AREA),
            Plane::Rle { values, ends } => {
                let mut start = 0usize;
                for (&v, &end) in values.iter().zip(ends) {
                    f(v, end as usize - start);
                    start = end as usize;
                }
            }
            Plane::Dense(d) => {
                let mut cur = d[0];
                let mut len = 0usize;
                for &v in d.iter() {
                    if v == cur {
                        len += 1;
                    } else {
                        f(cur, len);
                        cur = v;
                        len = 1;
                    }
                }
                f(cur, len);
            }
        }
    }

    pub fn run_count(&self) -> usize {
        match self {
            Plane::Uniform(_) => 1,
            Plane::Rle { values, .. } => values.len(),
            Plane::Dense(_) => {
                let mut n = 0;
                self.for_each_run(|_, _| n += 1);
                n
            }
        }
    }

    /// Re-pack a dense plane as `Uniform` or `Rle` when that is worthwhile.
    pub fn compact(&mut self) {
        let Plane::Dense(d) = self else { return };
        let first = d[0];
        let mut runs = 1usize;
        let mut prev = first;
        for &v in d.iter() {
            if v != prev {
                runs += 1;
                prev = v;
                if runs > MAX_RLE_RUNS {
                    return;
                }
            }
        }
        if runs == 1 {
            *self = Plane::Uniform(first);
            return;
        }
        let mut values = Vec::with_capacity(runs);
        let mut ends = Vec::with_capacity(runs);
        let mut end = 0usize;
        self.for_each_run(|v, len| {
            end += len;
            values.push(v);
            ends.push(end as u16);
        });
        *self = Plane::Rle { values, ends };
    }

    /// Heap bytes held by this plane.
    pub fn heap_bytes(&self) -> usize {
        match self {
            Plane::Uniform(_) => 0,
            Plane::Rle { values, ends } => {
                values.capacity() * size_of::<T>() + ends.capacity() * size_of::<u16>()
            }
            Plane::Dense(_) => TILE_AREA * size_of::<T>(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_reads_and_noop_writes_do_not_allocate() {
        let mut p = Plane::Uniform(4u16);
        assert_eq!(p.get(0), 4);
        assert_eq!(p.get(TILE_AREA - 1), 4);
        p.set(77, 4);
        assert_eq!(p.heap_bytes(), 0);
        assert_eq!(p.run_count(), 1);
    }

    #[test]
    fn write_unpacks_and_compact_repacks() {
        let mut p = Plane::Uniform(0u16);
        for i in 1000..3000 {
            p.set(i, 5);
        }
        assert!(matches!(p, Plane::Dense(_)));
        p.compact();
        match &p {
            Plane::Rle { values, ends } => {
                assert_eq!(values, &[0, 5, 0]);
                assert_eq!(ends, &[1000, 3000, TILE_AREA as u16]);
            }
            other => panic!("expected RLE, got {other:?}"),
        }
        // O(log R) reads on the packed form.
        assert_eq!(p.get(999), 0);
        assert_eq!(p.get(1000), 5);
        assert_eq!(p.get(2999), 5);
        assert_eq!(p.get(3000), 0);
        assert_eq!(p.get(TILE_AREA - 1), 0);

        // Writing the plane back to one value collapses it to uniform.
        for i in 1000..3000 {
            p.set(i, 0);
        }
        p.compact();
        assert_eq!(p.uniform_value(), Some(0));
    }

    #[test]
    fn rle_matches_dense_everywhere() {
        let mut dense = vec![0u16; TILE_AREA];
        let mut s = 99u64;
        let mut i = 0;
        while i < TILE_AREA {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = 1 + (s >> 40) as usize % 400;
            let val = (s >> 20) as u16 % 6;
            let end = (i + len).min(TILE_AREA);
            dense[i..end].fill(val);
            i = end;
        }
        let mut p = Plane::from_vec(dense.clone());
        p.compact();
        assert!(matches!(p, Plane::Rle { .. }));
        for (i, &v) in dense.iter().enumerate() {
            assert_eq!(p.get(i), v, "cell {i}");
        }
        assert_eq!(&p.dense()[..], &dense[..]);
        let mut total = 0;
        p.for_each_run(|_, len| total += len);
        assert_eq!(total, TILE_AREA);
    }

    #[test]
    fn chaotic_planes_stay_dense() {
        let v: Vec<u16> = (0..TILE_AREA).map(|i| (i % 2) as u16).collect();
        let mut p = Plane::from_vec(v);
        p.compact();
        assert!(matches!(p, Plane::Dense(_)));
    }
}
