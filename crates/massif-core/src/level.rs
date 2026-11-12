//! A level: one sheet of the multi-level surface, stored as a sparse tile map.
//!
//! Each tile is either *hot* (a decoded [`Tile`], editable) or *cold* (the
//! compressed blob). Reading never changes that state — a cold tile is decoded
//! into a temporary — so a `&Level` can be shared freely across threads.
//! Editing promotes a tile to hot; [`Level::trim`] cools the least recently
//! used tiles again, which bounds memory on very large worlds.

use crate::coords::TilePos;
use crate::tile::Tile;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::Arc;

/// A compressed tile. Shared between the level, undo steps and savers.
pub type Blob = Arc<[u8]>;

/// Stable identity of a level inside its dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LevelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelKind {
    /// The base heightmap: solid from the world floor up to `top`.
    Ground,
    /// Adds solid matter in `[bottom, top)`.
    Solid,
    /// Removes solid matter in `[bottom, top)`.
    Carve,
}

impl LevelKind {
    pub const fn has_bottom(self) -> bool {
        !matches!(self, LevelKind::Ground)
    }

    pub const fn label(self) -> &'static str {
        match self {
            LevelKind::Ground => "Ground",
            LevelKind::Solid => "Solid",
            LevelKind::Carve => "Carve",
        }
    }
}

#[derive(Debug, Clone)]
enum Slot {
    Hot {
        tile: Box<Tile>,
        /// Encoding of `tile`, valid until the next mutable access.
        blob: Option<Blob>,
        /// Last mutable access, for LRU eviction.
        stamp: u64,
    },
    Cold(Blob),
}

/// Read access to a tile: borrowed when hot, decoded on the fly when cold.
pub enum TileRef<'a> {
    Hot(&'a Tile),
    Decoded(Box<Tile>),
}

impl Deref for TileRef<'_> {
    type Target = Tile;

    #[inline]
    fn deref(&self) -> &Tile {
        match self {
            TileRef::Hot(t) => t,
            TileRef::Decoded(t) => t,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LevelStats {
    pub tiles: usize,
    pub hot_tiles: usize,
    pub hot_bytes: usize,
    pub cold_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub id: LevelId,
    pub name: String,
    pub kind: LevelKind,
    pub visible: bool,
    tiles: HashMap<TilePos, Slot>,
    hot: usize,
    clock: u64,
    dirty: HashSet<TilePos>,
}

fn decode_or_blank(blob: &[u8], kind: LevelKind, pos: TilePos) -> Tile {
    match Tile::decode(blob) {
        Ok(tile) => tile,
        Err(err) => {
            // Blobs are checksummed when a file is opened, so this means a
            // bug or memory corruption. Keep the editor alive with a blank.
            log::error!("tile {pos:?} cannot be decoded ({err}); replaced by a blank tile");
            if kind.has_bottom() {
                Tile::empty_slab()
            } else {
                Tile::ground(0.0, 0, crate::tile::NO_FLUID)
            }
        }
    }
}

enum Taken {
    Hot(Box<Tile>),
    Cold(Blob),
    Blank,
}

/// A tile taken out of a level by [`Level::check_out`].
pub struct CheckedOut {
    taken: Taken,
    kind: LevelKind,
    pos: TilePos,
}

impl CheckedOut {
    /// The tile, decoded if it was stored compressed, or a blank one.
    pub fn into_tile(self) -> Box<Tile> {
        match self.taken {
            Taken::Hot(tile) => tile,
            Taken::Cold(blob) => Box::new(decode_or_blank(&blob, self.kind, self.pos)),
            Taken::Blank => Box::new(if self.kind.has_bottom() {
                Tile::empty_slab()
            } else {
                Tile::ground(0.0, 0, crate::tile::NO_FLUID)
            }),
        }
    }
}

impl Level {
    pub fn new(id: LevelId, name: impl Into<String>, kind: LevelKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            visible: true,
            tiles: HashMap::new(),
            hot: 0,
            clock: 0,
            dirty: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn contains(&self, pos: TilePos) -> bool {
        self.tiles.contains_key(&pos)
    }

    pub fn positions(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.tiles.keys().copied()
    }

    /// Inclusive tile bounds `(min, max)` of the level, if it has any tile.
    pub fn bounds(&self) -> Option<(TilePos, TilePos)> {
        let mut it = self.tiles.keys();
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo.x = lo.x.min(p.x);
            lo.z = lo.z.min(p.z);
            hi.x = hi.x.max(p.x);
            hi.z = hi.z.max(p.z);
        }
        Some((lo, hi))
    }

    /// Read a tile without changing its hot/cold state.
    pub fn view(&self, pos: TilePos) -> Option<TileRef<'_>> {
        match self.tiles.get(&pos)? {
            Slot::Hot { tile, .. } => Some(TileRef::Hot(tile)),
            Slot::Cold(blob) => Some(TileRef::Decoded(Box::new(decode_or_blank(
                blob, self.kind, pos,
            )))),
        }
    }

    /// Edit an existing tile. Promotes it to hot and marks it dirty.
    pub fn tile_mut(&mut self, pos: TilePos) -> Option<&mut Tile> {
        let kind = self.kind;
        self.clock += 1;
        let slot = self.tiles.get_mut(&pos)?;
        if let Slot::Cold(blob) = slot {
            let tile = Box::new(decode_or_blank(blob, kind, pos));
            *slot = Slot::Hot {
                tile,
                blob: None,
                stamp: 0,
            };
            self.hot += 1;
        }
        self.dirty.insert(pos);
        match slot {
            Slot::Hot { tile, blob, stamp } => {
                *blob = None;
                *stamp = self.clock;
                Some(tile)
            }
            Slot::Cold(_) => unreachable!(),
        }
    }

    /// Edit a tile, creating a blank one if the level has none at `pos`.
    pub fn tile_mut_or_insert(&mut self, pos: TilePos, blank: impl FnOnce() -> Tile) -> &mut Tile {
        if !self.tiles.contains_key(&pos) {
            self.insert(pos, blank());
        }
        self.tile_mut(pos).expect("tile was just inserted")
    }

    /// Take a tile out of the level to edit it elsewhere (on another
    /// thread); give it back with [`Level::check_in`]. With `create`, a
    /// blank tile is made where the level has none. Decoding a cold tile is
    /// left to [`CheckedOut::into_tile`], so that it can run in parallel.
    pub fn check_out(&mut self, pos: TilePos, create: bool) -> Option<CheckedOut> {
        let taken = match self.tiles.remove(&pos) {
            Some(Slot::Hot { tile, .. }) => {
                self.hot -= 1;
                Taken::Hot(tile)
            }
            Some(Slot::Cold(blob)) => Taken::Cold(blob),
            None if create => Taken::Blank,
            None => return None,
        };
        Some(CheckedOut {
            taken,
            kind: self.kind,
            pos,
        })
    }

    /// Make sure the tiles at `positions` have an up-to-date blob, encoding
    /// the edited ones in parallel.
    pub fn encode_blobs(&mut self, positions: &[TilePos]) {
        if positions.is_empty() {
            return;
        }
        let wanted: HashSet<TilePos> = positions.iter().copied().collect();
        let mut pending: Vec<(&mut Box<Tile>, &mut Option<Blob>)> = self
            .tiles
            .iter_mut()
            .filter(|(pos, _)| wanted.contains(pos))
            .filter_map(|(_, slot)| match slot {
                Slot::Hot { tile, blob: blob @ None, .. } => Some((tile, blob)),
                _ => None,
            })
            .collect();
        pending.par_iter_mut().for_each(|(tile, blob)| {
            tile.compact();
            **blob = Some(Blob::from(tile.encode()));
        });
    }

    /// Put back a tile taken by [`Level::check_out`], as edited.
    pub fn check_in(&mut self, pos: TilePos, tile: Box<Tile>) {
        self.clock += 1;
        let slot = Slot::Hot {
            tile,
            blob: None,
            stamp: self.clock,
        };
        if let Some(Slot::Hot { .. }) = self.tiles.insert(pos, slot) {
            self.hot -= 1;
        }
        self.hot += 1;
        self.dirty.insert(pos);
    }

    /// Insert or replace a tile (hot).
    pub fn insert(&mut self, pos: TilePos, tile: Tile) {
        self.clock += 1;
        let slot = Slot::Hot {
            tile: Box::new(tile),
            blob: None,
            stamp: self.clock,
        };
        match self.tiles.insert(pos, slot) {
            Some(Slot::Hot { .. }) => {}
            _ => self.hot += 1,
        }
        self.dirty.insert(pos);
    }

    /// Insert or replace a tile from its compressed form (cold).
    pub fn insert_blob(&mut self, pos: TilePos, blob: Blob) {
        if let Some(Slot::Hot { .. }) = self.tiles.insert(pos, Slot::Cold(blob)) {
            self.hot -= 1;
        }
        self.dirty.insert(pos);
    }

    pub fn remove(&mut self, pos: TilePos) -> bool {
        match self.tiles.remove(&pos) {
            Some(Slot::Hot { .. }) => {
                self.hot -= 1;
                self.dirty.insert(pos);
                true
            }
            Some(Slot::Cold(_)) => {
                self.dirty.insert(pos);
                true
            }
            None => false,
        }
    }

    /// The compressed form of a tile, encoding (and caching) it if needed.
    /// Bytes the tile at `pos` takes, compressed (0 when it is hot and has
    /// not been encoded yet).
    pub fn blob_len(&self, pos: TilePos) -> usize {
        match self.tiles.get(&pos) {
            Some(Slot::Cold(blob)) => blob.len(),
            Some(Slot::Hot { blob: Some(blob), .. }) => blob.len(),
            _ => 0,
        }
    }

    pub fn blob(&mut self, pos: TilePos) -> Option<Blob> {
        match self.tiles.get_mut(&pos)? {
            Slot::Cold(blob) => Some(blob.clone()),
            Slot::Hot { tile, blob, .. } => {
                if blob.is_none() {
                    tile.compact();
                    *blob = Some(Blob::from(tile.encode()));
                }
                blob.clone()
            }
        }
    }

    /// Put a tile back to a previously captured state (`None` = no tile).
    pub fn restore(&mut self, pos: TilePos, state: Option<Blob>) {
        match state {
            Some(blob) => self.insert_blob(pos, blob),
            None => {
                self.remove(pos);
            }
        }
    }

    /// Every tile in compressed form, sorted by position, without encoding
    /// anything: `None` if some hot tile has been edited since it was last
    /// encoded. Always succeeds on a [`Level::snapshot`] or after
    /// [`Level::freeze`].
    pub fn blobs(&self) -> Option<Vec<(TilePos, Blob)>> {
        let mut out = Vec::with_capacity(self.tiles.len());
        for (pos, slot) in &self.tiles {
            let blob = match slot {
                Slot::Cold(b) => b.clone(),
                Slot::Hot { blob, .. } => blob.clone()?,
            };
            out.push((*pos, blob));
        }
        out.sort_unstable_by_key(|(pos, _)| (pos.z, pos.x));
        Some(out)
    }

    /// Encode every modified hot tile, in parallel. Afterwards [`Level::blob`]
    /// is free for every tile.
    pub fn freeze(&mut self) {
        self.tiles.par_iter_mut().for_each(|(_, slot)| {
            if let Slot::Hot { tile, blob, .. } = slot
                && blob.is_none()
            {
                tile.compact();
                *blob = Some(Blob::from(tile.encode()));
            }
        });
    }

    /// A cheap, fully cold copy of the level: only `Arc`s are cloned. Use it
    /// to hand a consistent snapshot to a background save or export.
    pub fn snapshot(&mut self) -> Level {
        self.freeze();
        let tiles = self
            .tiles
            .iter()
            .map(|(pos, slot)| {
                let blob = match slot {
                    Slot::Cold(b) => b.clone(),
                    Slot::Hot { blob, .. } => blob.clone().expect("frozen above"),
                };
                (*pos, Slot::Cold(blob))
            })
            .collect();
        Level {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind,
            visible: self.visible,
            tiles,
            hot: 0,
            clock: 0,
            dirty: HashSet::new(),
        }
    }

    /// Cool the least recently edited tiles until at most `max_hot` stay hot.
    pub fn trim(&mut self, max_hot: usize) {
        if self.hot <= max_hot {
            return;
        }
        let mut stamps: Vec<(u64, TilePos)> = self
            .tiles
            .iter()
            .filter_map(|(pos, slot)| match slot {
                Slot::Hot { stamp, .. } => Some((*stamp, *pos)),
                Slot::Cold(_) => None,
            })
            .collect();
        stamps.sort_unstable();
        let evict = &stamps[..self.hot - max_hot];
        let blobs: Vec<(TilePos, Blob)> = evict
            .par_iter()
            .map(|&(_, pos)| {
                let blob = match &self.tiles[&pos] {
                    Slot::Hot {
                        blob: Some(b), ..
                    } => b.clone(),
                    Slot::Hot { tile, .. } => {
                        let mut tile = tile.clone();
                        tile.compact();
                        Blob::from(tile.encode())
                    }
                    Slot::Cold(b) => b.clone(),
                };
                (pos, blob)
            })
            .collect();
        for (pos, blob) in blobs {
            self.tiles.insert(pos, Slot::Cold(blob));
        }
        self.hot = max_hot;
    }

    /// Drop slab tiles in which no column exists any more.
    pub fn prune_empty(&mut self, candidates: impl IntoIterator<Item = TilePos>) {
        if !self.kind.has_bottom() {
            return;
        }
        for pos in candidates {
            let empty = self.view(pos).is_some_and(|t| t.is_empty_slab());
            if empty {
                self.remove(pos);
            }
        }
    }

    /// Tiles changed since the last call; used to invalidate render caches.
    pub fn take_dirty(&mut self) -> Vec<TilePos> {
        self.dirty.drain().collect()
    }

    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Mark every tile dirty (after a wholesale replacement such as undo of a
    /// structural change).
    pub fn mark_all_dirty(&mut self) {
        self.dirty.extend(self.tiles.keys().copied());
    }

    pub fn stats(&self) -> LevelStats {
        let mut s = LevelStats {
            tiles: self.tiles.len(),
            ..Default::default()
        };
        for slot in self.tiles.values() {
            match slot {
                Slot::Hot { tile, blob, .. } => {
                    s.hot_tiles += 1;
                    s.hot_bytes += size_of::<Tile>() + tile.heap_bytes();
                    s.cold_bytes += blob.as_ref().map_or(0, |b| b.len());
                }
                Slot::Cold(blob) => s.cold_bytes += blob.len(),
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::NO_FLUID;

    fn level_with(n: i32) -> Level {
        let mut l = Level::new(LevelId(0), "Ground", LevelKind::Ground);
        for x in 0..n {
            l.insert(TilePos::new(x, -x), Tile::ground(60.0 + x as f32, 1, 63));
        }
        l
    }

    #[test]
    fn hot_and_cold_tiles_read_the_same() {
        let mut l = level_with(4);
        let pos = TilePos::new(2, -2);
        let before = l.view(pos).unwrap().top.get(0);
        l.trim(0);
        assert_eq!(l.stats().hot_tiles, 0);
        assert_eq!(l.view(pos).unwrap().top.get(0), before);
        assert!(matches!(l.view(pos).unwrap(), TileRef::Decoded(_)));
        // Editing promotes exactly that tile.
        l.tile_mut(pos).unwrap().top.set(5, 99.0);
        assert_eq!(l.stats().hot_tiles, 1);
        assert_eq!(l.view(pos).unwrap().top.get(5), 99.0);
        assert!(l.view(TilePos::new(9, 9)).is_none());
    }

    #[test]
    fn trim_evicts_least_recently_edited_first() {
        let mut l = level_with(5);
        for x in [3, 1] {
            l.tile_mut(TilePos::new(x, -x)).unwrap().top.set(0, 1.0);
        }
        l.trim(2);
        let s = l.stats();
        assert_eq!((s.tiles, s.hot_tiles), (5, 2));
        // The two tiles touched last are the ones still borrowed, not decoded.
        for x in [3, 1] {
            assert!(matches!(l.view(TilePos::new(x, -x)).unwrap(), TileRef::Hot(_)));
        }
    }

    #[test]
    fn blob_restore_is_exact() {
        let mut l = level_with(2);
        let pos = TilePos::new(1, -1);
        let saved = l.blob(pos);
        l.tile_mut(pos).unwrap().terrain.set(3, 7);
        assert_eq!(l.view(pos).unwrap().terrain.get(3), 7);
        l.restore(pos, saved);
        assert_eq!(l.view(pos).unwrap().terrain.get(3), 1);
        l.restore(pos, None);
        assert!(!l.contains(pos));
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn snapshot_is_independent_and_cold() {
        let mut l = level_with(3);
        let snap = l.snapshot();
        assert_eq!(snap.stats().hot_tiles, 0);
        l.tile_mut(TilePos::new(0, 0)).unwrap().top.fill(-10.0);
        assert_eq!(snap.view(TilePos::new(0, 0)).unwrap().top.get(0), 60.0);
        assert_eq!(l.view(TilePos::new(0, 0)).unwrap().top.get(0), -10.0);
    }

    #[test]
    fn dirty_tracking_and_bounds() {
        let mut l = level_with(3);
        assert_eq!(l.take_dirty().len(), 3);
        assert!(!l.has_dirty());
        l.tile_mut(TilePos::new(1, -1));
        assert_eq!(l.take_dirty(), vec![TilePos::new(1, -1)]);
        assert_eq!(l.bounds(), Some((TilePos::new(0, -2), TilePos::new(2, 0))));
    }

    #[test]
    fn empty_slab_tiles_are_pruned() {
        let mut l = Level::new(LevelId(1), "Island", LevelKind::Solid);
        let pos = TilePos::new(0, 0);
        l.insert(pos, Tile::empty_slab());
        l.insert(TilePos::new(1, 0), Tile::empty_slab());
        {
            let t = l.tile_mut(pos).unwrap();
            t.top.set(0, 100.0);
            t.bottom.as_mut().unwrap().set(0, 90.0);
            t.fluid.fill(NO_FLUID);
        }
        l.prune_empty([pos, TilePos::new(1, 0)]);
        assert!(l.contains(pos));
        assert!(!l.contains(TilePos::new(1, 0)));
    }
}
