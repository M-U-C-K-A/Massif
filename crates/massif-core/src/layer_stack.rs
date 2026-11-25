//! Several layers of one slot on one column, each with its own intensity,
//! as WorldPainter keeps them: a custom ground cover of Conquest grass
//! layers, plants growing on it, frost over everything; a forest at half
//! strength and two kinds of bushes at 15 % and 8 % over the same block.
//!
//! A column holds one layer of each slot in its tile's planes
//! ([`crate::Tile::cover`], [`crate::Tile::objects`],
//! [`crate::Tile::understory`]): the one every brush paints, the map shows
//! and the slot's tools pick. A map brought from WorldPainter may hold more
//! in [`crate::Tile::stacked`], the column's list of entries:
//!
//! - **covers** lying under the column's cover, bottom to top: the export
//!   grows each over the one under it, then the cover on top; rubbing out
//!   the cover on top uncovers the one under it;
//! - **tree layers** (base or sub-canopy) of the column, with their
//!   strength: all of the column's layers of that slot, the one of the
//!   plane among them (the strongest; one without an entry is at full
//!   strength). Each stands its objects as its own spacing places them,
//!   thinned to its strength there.
//!
//! Painting a layer of a slot over the column takes the place of all of
//! that slot's (as a slot holds one layer), none of them locked; rubbing
//! one out takes it alone out.
//!
//! The lists live in the tile: a table of those in use, and a plane giving
//! each column's (0: none, k: the table's list k − 1), so that brushes
//! change them tile by tile, in parallel, without the map's registry. A
//! map without stacked layers has no such plane, and is saved as before.

use crate::codec::{self, Reader, Result};
use crate::plane::Plane;

/// The slot of a stacked layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Cover = 0,
    Objects = 1,
    Understory = 2,
    /// An annotation layer of the map's own (a named colour of
    /// [`crate::annotations::AnnotationPalette::custom`]): unlike the other
    /// slots, several cover the same column, each painted and rubbed out
    /// on its own, as WorldPainter's custom annotation layers.
    Note = 3,
}

impl Kind {
    fn from_bits(b: u32) -> Option<Kind> {
        match b {
            0 => Some(Kind::Cover),
            1 => Some(Kind::Objects),
            2 => Some(Kind::Understory),
            3 => Some(Kind::Note),
            _ => None,
        }
    }
}

/// One stacked layer: its slot, its number, its strength (255: full).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub layer: u16,
    pub strength: u8,
}

impl Entry {
    pub fn new(kind: Kind, layer: u16, strength: u8) -> Self {
        Self { kind, layer, strength }
    }

    fn pack(self) -> u32 {
        ((self.kind as u32) << 24) | ((self.strength as u32) << 16) | self.layer as u32
    }

    pub fn unpack(v: u32) -> Option<Entry> {
        Some(Entry { kind: Kind::from_bits(v >> 24)?, strength: (v >> 16) as u8, layer: v as u16 })
    }
}

/// The stacked layers of each column of a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerStacks {
    /// Each column's list: 0 for none, else its place in `lists` plus one.
    pub plane: Plane<u16>,
    /// The lists of packed entries, never empty: covers bottom to top.
    pub lists: Vec<Vec<u32>>,
}

impl Default for LayerStacks {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerStacks {
    pub fn new() -> Self {
        Self { plane: Plane::Uniform(0), lists: Vec::new() }
    }

    /// Whether no column holds stacked layers.
    pub fn is_empty(&self) -> bool {
        self.plane.uniform_value() == Some(0)
    }

    fn raw(&self, i: usize) -> &[u32] {
        match self.plane.get(i) {
            0 => &[],
            k => self.lists.get(k as usize - 1).map_or(&[], Vec::as_slice),
        }
    }

    /// The stacked layers of column `i`.
    pub fn entries(&self, i: usize) -> impl Iterator<Item = Entry> + '_ {
        self.raw(i).iter().filter_map(|&v| Entry::unpack(v))
    }

    /// Whether column `i` holds stacked layers.
    pub fn has(&self, i: usize) -> bool {
        !self.is_empty() && self.plane.get(i) != 0
    }

    /// The covers lying under column `i`'s cover, bottom to top.
    pub fn covers_under(&self, i: usize) -> Vec<u16> {
        self.entries(i).filter(|e| e.kind == Kind::Cover).map(|e| e.layer).collect()
    }

    /// The strength of layer `n` of slot `kind` stacked on column `i`.
    pub fn strength(&self, i: usize, kind: Kind, n: u16) -> Option<u8> {
        self.entries(i).find(|e| e.kind == kind && e.layer == n).map(|e| e.strength)
    }

    /// The layers of slot `kind` (trees, under the trees) of column `i`,
    /// with their strength, `plane` being the layer the tile's plane holds
    /// there: that one at full strength without stacked ones.
    pub fn layers_of(&self, i: usize, kind: Kind, plane: u16) -> Vec<(u16, u8)> {
        let mut out: Vec<(u16, u8)> = self.entries(i).filter(|e| e.kind == kind).map(|e| (e.layer, e.strength)).collect();
        if plane != 0 && !out.iter().any(|e| e.0 == plane) {
            out.insert(0, (plane, 255));
        }
        out
    }

    /// Set column `i`'s whole list.
    pub fn set(&mut self, i: usize, entries: &[Entry]) {
        if entries.is_empty() {
            if self.plane.get(i) != 0 {
                self.plane.set(i, 0);
            }
            return;
        }
        let packed: Vec<u32> = entries.iter().map(|e| e.pack()).collect();
        let k = match self.lists.iter().position(|l| *l == packed) {
            Some(k) => k,
            None => {
                self.lists.push(packed);
                self.lists.len() - 1
            }
        };
        self.plane.set(i, k as u16 + 1);
    }

    fn edit(&mut self, i: usize, f: impl FnOnce(&mut Vec<Entry>)) {
        if !self.has(i) {
            return;
        }
        let mut list: Vec<Entry> = self.entries(i).collect();
        f(&mut list);
        self.set(i, &list);
    }

    /// The annotation layers on column `i`, as they were painted.
    pub fn notes(&self, i: usize) -> impl Iterator<Item = u16> + '_ {
        self.entries(i).filter(|e| e.kind == Kind::Note).map(|e| e.layer)
    }

    /// Put `entry` on column `i`, on top of those there; nothing when that
    /// layer of its slot is there already.
    pub fn add(&mut self, i: usize, entry: Entry) {
        if self.has(i) && self.entries(i).any(|e| e.kind == entry.kind && e.layer == entry.layer) {
            return;
        }
        let mut list: Vec<Entry> = if self.has(i) { self.entries(i).collect() } else { Vec::new() };
        list.push(entry);
        self.set(i, &list);
    }

    /// Take the layers of slot `kind` out of column `i`.
    pub fn clear_kind(&mut self, i: usize, kind: Kind) {
        self.edit(i, |l| l.retain(|e| e.kind != kind));
    }

    /// Take layer `n` of slot `kind` out of column `i`; whether it was there.
    pub fn remove(&mut self, i: usize, kind: Kind, n: u16) -> bool {
        let mut found = false;
        self.edit(i, |l| {
            let before = l.len();
            l.retain(|e| !(e.kind == kind && e.layer == n));
            found = l.len() != before;
        });
        found
    }

    /// Take the topmost cover under column `i`'s out of the stack.
    pub fn pop_cover(&mut self, i: usize) -> Option<u16> {
        let mut top = None;
        self.edit(i, |l| {
            if let Some(k) = l.iter().rposition(|e| e.kind == Kind::Cover) {
                top = Some(l.remove(k).layer);
            }
        });
        top
    }

    /// Whether any column may hold layer `n` of slot `kind` stacked (since
    /// the last [`LayerStacks::compact`], exactly).
    pub fn holds(&self, kind: Kind, n: u16) -> bool {
        !self.is_empty() && self.lists.iter().flatten().filter_map(|&v| Entry::unpack(v)).any(|e| e.kind == kind && e.layer == n)
    }

    /// Change every layer's number by `f` (for its slot): a map whose layers
    /// were numbered again.
    pub fn renumber(&mut self, f: impl Fn(Kind, u16) -> u16) {
        for list in &mut self.lists {
            for v in list.iter_mut() {
                if let Some(e) = Entry::unpack(*v) {
                    *v = Entry { layer: f(e.kind, e.layer), ..e }.pack();
                }
            }
        }
    }

    /// Re-pack: the plane, and the lists no column uses any more dropped.
    pub fn compact(&mut self) {
        self.plane.compact();
        if self.is_empty() {
            self.lists.clear();
            return;
        }
        let mut used = vec![false; self.lists.len()];
        let mut mark = |v: u16| {
            if v > 0 && (v as usize) <= used.len() {
                used[v as usize - 1] = true;
            }
        };
        match &self.plane {
            Plane::Uniform(v) => mark(*v),
            p => p.dense().iter().for_each(|&v| mark(v)),
        }
        if used.iter().all(|&u| u) {
            return;
        }
        let mut new_index = vec![0u16; self.lists.len()];
        let mut lists = Vec::new();
        for (k, list) in std::mem::take(&mut self.lists).into_iter().enumerate() {
            if used[k] {
                lists.push(list);
                new_index[k] = lists.len() as u16;
            }
        }
        self.lists = lists;
        self.plane = crate::tile::map_plane(&self.plane, |v| if v == 0 { 0 } else { new_index[v as usize - 1] });
        self.plane.compact();
    }

    pub fn heap_bytes(&self) -> usize {
        self.plane.heap_bytes() + self.lists.iter().map(|l| l.len() * 4 + 24).sum::<usize>()
    }

    /// The tile plane's payload: the lists, then the plane.
    pub fn encode(&self, out: &mut Vec<u8>) {
        codec::put_varu(out, self.lists.len() as u64);
        for list in &self.lists {
            codec::put_varu(out, list.len() as u64);
            for &v in list {
                codec::put_varu(out, v as u64);
            }
        }
        codec::encode_categorical(&self.plane, out);
    }

    pub fn decode(r: &mut Reader) -> Result<Self> {
        let n = r.varu_max(u16::MAX as u64)? as usize;
        let mut lists = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let len = r.varu_max(u16::MAX as u64)? as usize;
            let mut list = Vec::with_capacity(len.min(256));
            for _ in 0..len {
                list.push(r.varu_max(u32::MAX as u64)? as u32);
            }
            lists.push(list);
        }
        let plane = codec::decode_categorical(r)?;
        Ok(Self { plane, lists })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_hold_covers_and_trees_and_come_back_encoded() {
        let mut s = LayerStacks::new();
        assert!(s.is_empty() && s.covers_under(5).is_empty());
        let two = [Entry::new(Kind::Cover, 3, 255), Entry::new(Kind::Cover, 7, 255), Entry::new(Kind::Objects, 4, 128)];
        s.set(5, &two);
        s.set(6, &two);
        s.set(9, &[Entry::new(Kind::Objects, 2, 40), Entry::new(Kind::Understory, 8, 20)]);
        assert_eq!((s.covers_under(5), s.lists.len()), (vec![3, 7], 2), "one list for both columns");
        assert_eq!(s.layers_of(5, Kind::Objects, 1), vec![(1, 255), (4, 128)], "the plane's layer at full strength");
        assert_eq!(s.layers_of(9, Kind::Objects, 2), vec![(2, 40)]);
        assert_eq!(s.strength(9, Kind::Understory, 8), Some(20));
        assert_eq!(s.pop_cover(5), Some(7));
        assert_eq!(s.covers_under(5), vec![3]);
        assert!(s.remove(6, Kind::Cover, 3) && !s.remove(6, Kind::Cover, 3));
        assert_eq!(s.covers_under(6), vec![7]);
        s.clear_kind(9, Kind::Understory);
        assert_eq!(s.strength(9, Kind::Understory, 8), None);
        // Lists no column uses any more go at the next compaction.
        s.compact();
        assert!(s.holds(Kind::Cover, 3) && s.holds(Kind::Objects, 4) && !s.holds(Kind::Understory, 8));
        s.renumber(|k, n| if k == Kind::Cover { n + 10 } else { n });
        s.compact();
        assert_eq!((s.covers_under(5), s.covers_under(6), s.strength(9, Kind::Objects, 2)), (vec![13], vec![17], Some(40)));
        let mut out = Vec::new();
        s.encode(&mut out);
        let back = LayerStacks::decode(&mut Reader::new(&out)).unwrap();
        assert_eq!(back, s);
        for i in [5, 6, 9] {
            s.set(i, &[]);
        }
        s.compact();
        assert!(s.is_empty() && s.lists.is_empty());
    }
}
