//! What each tile of a map holds: the terrains, biomes and layers painted
//! on it, each with a column where it is. Kept from one look to the next,
//! a tile read again only once it changed, so that finding where a layer
//! is painted on a map of thousands of tiles, or listing what a map uses,
//! costs little after the first time.

use crate::coords::{TILE_SIZE, TilePos};
use crate::filter::{Match, Painted};
use crate::level::Blob;
use crate::plane::Plane;
use crate::tile::Tile;
use crate::world::Dimension;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What one tile holds: each terrain, biome and layer on it, with the
/// first column (its index in the tile) it is on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TileSummary {
    pub found: Vec<(Match, u16)>,
}

impl TileSummary {
    /// Read a tile's planes.
    pub fn of(tile: &Tile) -> Self {
        let mut found = Vec::new();
        let mut plane = |plane: &Plane<u16>, make: &dyn Fn(u16) -> Option<Match>| {
            if let Some(v) = plane.uniform_value() {
                if let Some(m) = make(v) {
                    found.push((m, 0));
                }
                return;
            }
            let dense = plane.dense();
            let mut seen: Vec<u16> = Vec::new();
            let mut last = None;
            for (i, &v) in dense.iter().enumerate() {
                if last == Some(v) {
                    continue;
                }
                last = Some(v);
                if !tile.present(i) || seen.contains(&v) {
                    continue;
                }
                seen.push(v);
                if let Some(m) = make(v) {
                    found.push((m, i as u16));
                }
            }
        };
        plane(&tile.terrain, &|v| Some(Match::Terrain(v)));
        plane(&tile.biome, &|v| (v != 0).then_some(Match::Biome(v)));
        for p in Painted::ALL {
            plane(p.plane(tile), &|v| (v != 0).then_some(Match::Layer(p, v)));
        }
        // The layers stacked on columns: each at its first column.
        if !tile.stacked.is_empty() {
            use crate::layer_stack::Kind;
            let dense = tile.stacked.plane.dense();
            for (k, _) in tile.stacked.lists.iter().enumerate() {
                let Some(i) = dense.iter().position(|&v| v as usize == k + 1) else { continue };
                for e in tile.stacked.entries(i) {
                    let p = match e.kind {
                        Kind::Cover => Painted::Cover,
                        Kind::Objects => Painted::Objects,
                        Kind::Understory => Painted::Understory,
                    };
                    let m = Match::Layer(p, e.layer);
                    if !found.iter().any(|f| f.0 == m) {
                        found.push((m, i as u16));
                    }
                }
            }
        }
        Self { found }
    }
}

/// A tile of a level: the level's index and the tile.
type TileKey = (usize, TilePos);
/// What a tile was read from, and what it holds.
type Read = (Option<Blob>, Arc<TileSummary>);

/// What the tiles of a dimension hold, kept up to date by
/// [`Presence::update`].
#[derive(Debug, Default)]
pub struct Presence {
    /// By level and tile: the tile's encoding it was read from (`None`: a
    /// tile changed since it was encoded, read again at every update), and
    /// what it holds.
    tiles: HashMap<TileKey, Read>,
}

impl Presence {
    /// Read again the tiles of `dim` that changed since the last update,
    /// in parallel; forget those no longer there. Returns how many were
    /// read.
    pub fn update(&mut self, dim: &Dimension) -> usize {
        let mut wanted: Vec<(usize, TilePos, Option<Blob>)> = Vec::new();
        let mut keep: HashSet<(usize, TilePos)> = HashSet::new();
        for (li, level) in dim.levels.iter().enumerate() {
            for pos in level.positions() {
                keep.insert((li, pos));
                let blob = level.cached_blob(pos);
                let same = match (self.tiles.get(&(li, pos)), &blob) {
                    (Some((Some(old), _)), Some(new)) => Arc::ptr_eq(old, new),
                    _ => false,
                };
                if !same {
                    wanted.push((li, pos, blob));
                }
            }
        }
        self.tiles.retain(|k, _| keep.contains(k));
        let read: Vec<(TileKey, Read)> = wanted
            .into_par_iter()
            .filter_map(|(li, pos, blob)| {
                let tile = dim.levels[li].view(pos)?;
                Some(((li, pos), (blob, Arc::new(TileSummary::of(&tile)))))
            })
            .collect();
        let n = read.len();
        self.tiles.extend(read);
        n
    }

    /// Everything painted somewhere on the dimension.
    pub fn used(&self) -> HashSet<Match> {
        self.tiles.values().flat_map(|(_, s)| s.found.iter().map(|(m, _)| *m)).collect()
    }

    /// The places `what` is on: one column for each patch of tiles that
    /// touch (by a side or a corner) and hold it, the one of its tile
    /// nearest the middle of the patch.
    pub fn places(&self, what: Match) -> Vec<(i32, i32)> {
        let mut at: HashMap<TilePos, u16> = HashMap::new();
        for ((_, pos), (_, s)) in &self.tiles {
            if let Some((_, cell)) = s.found.iter().find(|(m, _)| *m == what) {
                at.entry(*pos).or_insert(*cell);
            }
        }
        let mut out = Vec::new();
        let mut left: HashSet<TilePos> = at.keys().copied().collect();
        let mut order: Vec<TilePos> = at.keys().copied().collect();
        order.sort_by_key(|p| (p.z, p.x));
        for start in order {
            if !left.remove(&start) {
                continue;
            }
            let mut patch = vec![start];
            let mut k = 0;
            while k < patch.len() {
                let p = patch[k];
                k += 1;
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let q = p.offset(dx, dz);
                        if left.remove(&q) {
                            patch.push(q);
                        }
                    }
                }
            }
            let n = patch.len() as f64;
            let (mx, mz) = (patch.iter().map(|p| p.x as f64).sum::<f64>() / n, patch.iter().map(|p| p.z as f64).sum::<f64>() / n);
            let mid = *patch.iter().min_by(|a, b| {
                let d = |p: &&TilePos| (p.x as f64 - mx).powi(2) + (p.z as f64 - mz).powi(2);
                d(a).total_cmp(&d(b)).then((a.z, a.x).cmp(&(b.z, b.x)))
            }).expect("a patch has a tile");
            let cell = at[&mid] as i32;
            let (x0, z0) = mid.origin();
            out.push((x0 + cell % TILE_SIZE, z0 + cell / TILE_SIZE));
        }
        out
    }
}

/// `places` from the nearest to `from` to the farthest.
pub fn by_distance(mut places: Vec<(i32, i32)>, from: (f64, f64)) -> Vec<(i32, i32)> {
    let d = |p: &(i32, i32)| (p.0 as f64 + 0.5 - from.0).powi(2) + (p.1 as f64 + 0.5 - from.1).powi(2);
    places.sort_by(|a, b| d(a).total_cmp(&d(b)).then(a.cmp(b)));
    places
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::local_index;
    use crate::world::{World, WorldMeta};

    fn map() -> World {
        let mut w = World::new(WorldMeta::default());
        let mut dim = Dimension::new(crate::world::DimensionKind::Overworld, -64, 320);
        for tz in 0..6 {
            for tx in 0..6 {
                dim.ground_mut().insert(TilePos::new(tx, tz), Tile::ground(64.0, 1, crate::tile::NO_FLUID));
            }
        }
        w.dims[0] = dim;
        w
    }

    fn paint(w: &mut World, x: i32, z: i32, n: u16) {
        let tile = w.dims[0].ground_mut().tile_mut(TilePos::containing(x, z)).unwrap();
        tile.cover.set(local_index(x, z), n);
    }

    /// A cover painted in two patches is found at two places, the nearest
    /// first; a tile changed is read again, one unchanged is not.
    #[test]
    fn finds_each_patch_and_reads_only_what_changed() {
        let mut w = map();
        paint(&mut w, 10, 10, 3);
        paint(&mut w, 140, 20, 3);
        paint(&mut w, 650, 700, 3);
        w.dims[0].ground_mut().trim(0);
        let mut p = Presence::default();
        assert_eq!(p.update(&w.dims[0]), 36);
        assert!(p.used().contains(&Match::Layer(Painted::Cover, 3)));
        assert!(!p.used().contains(&Match::Layer(Painted::Cover, 4)));
        let places = p.places(Match::Layer(Painted::Cover, 3));
        assert_eq!(places.len(), 2, "two tiles side by side are one patch: {places:?}");
        let near = by_distance(places, (600.0, 600.0));
        assert_eq!(near[0], (650, 700));
        assert_eq!(p.update(&w.dims[0]), 0, "nothing changed");
        paint(&mut w, 300, 300, 5);
        assert_eq!(p.update(&w.dims[0]), 1, "only the tile painted");
        assert_eq!(p.places(Match::Layer(Painted::Cover, 5)), vec![(300, 300)]);
    }

    /// The planes of a tile, uniform or not, each value once with its first
    /// column; nothing for 0 where 0 is "none".
    #[test]
    fn a_tile_summary_lists_each_value_once() {
        let mut t = Tile::ground(64.0, 2, crate::tile::NO_FLUID);
        t.objects.set(local_index(5, 7), 9);
        t.objects.set(local_index(6, 7), 9);
        t.annotations.set(local_index(1, 1), 4);
        let s = TileSummary::of(&t);
        assert!(s.found.contains(&(Match::Terrain(2), 0)));
        assert!(s.found.contains(&(Match::Layer(Painted::Objects, 9), local_index(5, 7) as u16)));
        assert!(s.found.contains(&(Match::Layer(Painted::Annotation, 4), local_index(1, 1) as u16)));
        assert_eq!(s.found.len(), 3);
    }
}
