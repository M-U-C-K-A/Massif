//! The map as an input of a graph: its ground, read over an extent.
//!
//! A graph usually makes its relief from nothing; with a Map node it starts
//! from the map as it is — one that was painted by hand, or imported from
//! WorldPainter — and works on that: rivers dug where its water gathers,
//! erosion over its slopes. Only the ground level is read, never the levels
//! a graph made itself, so that converting twice does not dig twice.

use super::{ColorField, Extent, Field};
use crate::coords::{TILE_SIZE, TilePos, cell_index};
use crate::tile::NO_FLUID;
use crate::world::Dimension;
use rayon::prelude::*;
use std::sync::Arc;

/// Where a graph's Map node reads the map.
pub trait MapSource: Sync {
    /// A number that changes whenever the map does: what was read before
    /// it changed is out of date.
    fn revision(&self) -> u64;

    /// The ground over `extent`.
    fn sample(&self, extent: &Extent) -> MapSample;
}

/// The ground of a map over an extent.
pub struct MapSample {
    /// Its heights, in blocks.
    pub heights: Field,
    /// Where water lies on it: 1 under water, 0 dry.
    pub water: Field,
    /// The colours of its terrains.
    pub colors: ColorField,
}

/// Read the ground of `dim` at every sample of `extent`: its height,
/// whether it is under water, and the colour of its terrain (`colors`, by
/// terrain). Samples off the map get `fallback`, no water and grey. Each
/// tile is decoded once, bands of tiles in parallel.
pub fn sample_ground(dim: &Dimension, e: &Extent, fallback: f32, colors: &[[u8; 3]]) -> MapSample {
    let n = e.n;
    let spacing = e.size / n as f64;
    let ground = dim.ground();
    let mut heights = vec![fallback; n * n];
    let mut water = vec![0f32; n * n];
    let mut paint = vec![[128u8; 3]; n * n];
    // Rows of samples, cut where they pass from one row of tiles to the
    // next: each band is filled by itself.
    let tile_row = |j: usize| ((e.z0 + (j as f64 + 0.5) * spacing).floor() as i32).div_euclid(TILE_SIZE);
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for j in 1..=n {
        if j == n || tile_row(j) != tile_row(start) {
            bands.push((start, j));
            start = j;
        }
    }
    type Band<'a> = (usize, &'a mut [f32], &'a mut [f32], &'a mut [[u8; 3]]);
    let mut parts: Vec<Band> = Vec::with_capacity(bands.len());
    let (mut rest_h, mut rest_w, mut rest_c) = (&mut heights[..], &mut water[..], &mut paint[..]);
    for &(j0, j1) in &bands {
        let (h, tail_h) = rest_h.split_at_mut((j1 - j0) * n);
        let (w, tail_w) = rest_w.split_at_mut((j1 - j0) * n);
        let (c, tail_c) = rest_c.split_at_mut((j1 - j0) * n);
        parts.push((j0, h, w, c));
        (rest_h, rest_w, rest_c) = (tail_h, tail_w, tail_c);
    }
    parts.into_par_iter().for_each(|(j0, h, w, c)| {
        let tz = tile_row(j0);
        let rows = h.len() / n;
        let mut i = 0;
        while i < n {
            // The samples of this band over one tile.
            let x = (e.x0 + (i as f64 + 0.5) * spacing).floor() as i32;
            let tx = x.div_euclid(TILE_SIZE);
            let mut end = i + 1;
            while end < n && ((e.x0 + (end as f64 + 0.5) * spacing).floor() as i32).div_euclid(TILE_SIZE) == tx {
                end += 1;
            }
            if let Some(tile) = ground.view(TilePos::new(tx, tz)) {
                for r in 0..rows {
                    let z = (e.z0 + ((j0 + r) as f64 + 0.5) * spacing).floor() as i32;
                    let lz = z.rem_euclid(TILE_SIZE) as usize;
                    for k in i..end {
                        let x = (e.x0 + (k as f64 + 0.5) * spacing).floor() as i32;
                        let cell = cell_index(x.rem_euclid(TILE_SIZE) as usize, lz);
                        let height = tile.top.get(cell);
                        let fluid = tile.fluid.get(cell);
                        h[r * n + k] = height;
                        w[r * n + k] = (fluid != NO_FLUID && fluid as f32 > height) as u8 as f32;
                        c[r * n + k] = colors.get(tile.terrain.get(cell) as usize).copied().unwrap_or([128; 3]);
                    }
                }
            }
            i = end;
        }
    });
    MapSample {
        heights: Field::new(n, heights),
        water: Field::new(n, water),
        colors: ColorField { n, data: paint },
    }
}

/// A copy of a dimension to read the map from on another thread.
pub struct GroundMap {
    dim: Dimension,
    revision: u64,
    fallback: f32,
    colors: Vec<[u8; 3]>,
}

impl GroundMap {
    /// `dim` is usually a snapshot ([`Dimension::snapshot`]); `fallback` is
    /// the height of what lies off the map, `colors` the colour of every
    /// terrain of the world's registry.
    pub fn new(dim: Dimension, revision: u64, fallback: f32, colors: Vec<[u8; 3]>) -> Self {
        Self {
            dim,
            revision,
            fallback,
            colors,
        }
    }
}

impl MapSource for GroundMap {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn sample(&self, extent: &Extent) -> MapSample {
        sample_ground(&self.dim, extent, self.fallback, &self.colors)
    }
}

/// The map already read over one extent (for previews): other extents are
/// resampled from it.
pub struct SampledMap {
    pub extent: Extent,
    pub heights: Arc<Field>,
    pub water: Arc<Field>,
    pub colors: Arc<ColorField>,
    pub revision: u64,
}

impl MapSource for SampledMap {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn sample(&self, extent: &Extent) -> MapSample {
        if extent.key() == self.extent.key() {
            return MapSample {
                heights: (*self.heights).clone(),
                water: (*self.water).clone(),
                colors: (*self.colors).clone(),
            };
        }
        let n = extent.n;
        let read = |f: &Field| {
            Field::new(
                n,
                (0..n * n)
                    .into_par_iter()
                    .map(|c| {
                        let (x, z) = extent.world(c % n, c / n);
                        let (u, v) = self.extent.grid(x, z);
                        f.sample(u, v)
                    })
                    .collect(),
            )
        };
        let m = self.colors.n;
        let colors = (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (x, z) = extent.world(c % n, c / n);
                let (u, v) = self.extent.grid(x, z);
                let at = |t: f32| (t.round().max(0.0) as usize).min(m - 1);
                self.colors.data[at(v) * m + at(u)]
            })
            .collect();
        MapSample {
            heights: read(&self.heights),
            water: read(&self.water),
            colors: ColorField { n, data: colors },
        }
    }
}
