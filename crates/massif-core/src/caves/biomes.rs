//! Cave biomes: biomes painted in volume, under the ground.
//!
//! The game keeps a biome for every cell of 4 × 4 × 4 blocks; a column's
//! painted biome (see [`crate::tile::Tile::biome`]) holds over its whole
//! height. A ground tile keeps, besides, the cave biomes painted in it:
//! layers of 32 × 32 cells, one every 4 blocks of height where something
//! is painted, each cell a biome ([`crate::registry::BiomeId`]) or 0 for
//! the column's own. The export writes them over the column's biome in
//! the cells they fill (see [`crate::compose::TileStack::biome_column`]),
//! and the decoration of the caves follows them (lush caves, dripstone
//! caves, the deep dark).

use crate::codec::{self, CodecError, Reader};

/// Cells across a tile.
pub const CELLS: usize = 32;
/// Cells in a layer.
const LAYER: usize = CELLS * CELLS;

/// The cave biomes of a tile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaveBiomes {
    /// `(cell y, biomes of its 32 × 32 cells)`, by height; no layer holds
    /// only 0.
    layers: Vec<(i16, Box<[u16; LAYER]>)>,
}

impl CaveBiomes {
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// The biome painted in the cell of block `(lx, y, lz)` of the tile (`lx`,
    /// `lz` in 0..128); 0: none.
    #[inline]
    pub fn get(&self, lx: i32, y: i32, lz: i32) -> u16 {
        let cy = y.div_euclid(4) as i16;
        match self.layers.binary_search_by_key(&cy, |l| l.0) {
            Ok(k) => self.layers[k].1[(lz as usize / 4) * CELLS + lx as usize / 4],
            Err(_) => 0,
        }
    }

    /// Paint `biome` (0: none) in the cells `cx0..cx1` × `cy0..cy1` ×
    /// `cz0..cz1` (cells of the tile across, of the world up) where `keep`
    /// lets it. Returns the cells changed.
    pub fn paint(&mut self, (cx0, cx1): (usize, usize), (cy0, cy1): (i32, i32), (cz0, cz1): (usize, usize), biome: u16, mut keep: impl FnMut(usize, i32, usize) -> bool) -> usize {
        let mut changed = 0;
        for cy in cy0..cy1 {
            let cy16 = cy as i16;
            let k = match self.layers.binary_search_by_key(&cy16, |l| l.0) {
                Ok(k) => k,
                Err(_) if biome == 0 => continue,
                Err(at) => {
                    self.layers.insert(at, (cy16, Box::new([0; LAYER])));
                    at
                }
            };
            let layer = &mut self.layers[k].1;
            for cz in cz0..cz1.min(CELLS) {
                for cx in cx0..cx1.min(CELLS) {
                    let i = cz * CELLS + cx;
                    if layer[i] != biome && keep(cx, cy, cz) {
                        layer[i] = biome;
                        changed += 1;
                    }
                }
            }
        }
        self.layers.retain(|l| l.1.iter().any(|&b| b != 0));
        changed
    }

    /// The runs of cave biome over the 4 × 4 column of cells `(cx, cz)`, as
    /// `(y0, y1, biome)` in blocks, bottom up.
    pub fn column(&self, cx: usize, cz: usize, out: &mut Vec<(i32, i32, u16)>) {
        out.clear();
        let i = cz * CELLS + cx;
        for (cy, layer) in &self.layers {
            let b = layer[i];
            if b == 0 {
                continue;
            }
            let (y0, y1) = (*cy as i32 * 4, *cy as i32 * 4 + 4);
            match out.last_mut() {
                Some(last) if last.1 == y0 && last.2 == b => last.1 = y1,
                _ => out.push((y0, y1, b)),
            }
        }
    }

    /// The biomes painted, each once.
    pub fn used(&self) -> Vec<u16> {
        let mut out: Vec<u16> = Vec::new();
        for (_, layer) in &self.layers {
            for &b in layer.iter() {
                if b != 0 && !out.contains(&b) {
                    out.push(b);
                }
            }
        }
        out
    }

    /// Map every biome painted through `f` (biomes renumbered).
    pub fn remap(&mut self, f: impl Fn(u16) -> u16) {
        for (_, layer) in &mut self.layers {
            for b in layer.iter_mut() {
                if *b != 0 {
                    *b = f(*b);
                }
            }
        }
        self.layers.retain(|l| l.1.iter().any(|&b| b != 0));
    }

    pub fn heap_bytes(&self) -> usize {
        self.layers.len() * (LAYER * 2 + 16)
    }

    /// The layers as the tile's plane of tag 15: their count, then for each
    /// its cell height and its cells in runs of one biome.
    pub fn encode(&self, out: &mut Vec<u8>) {
        codec::put_varu(out, self.layers.len() as u64);
        for (cy, layer) in &self.layers {
            out.extend_from_slice(&cy.to_le_bytes());
            let mut i = 0;
            while i < LAYER {
                let b = layer[i];
                let mut n = 1;
                while i + n < LAYER && layer[i + n] == b {
                    n += 1;
                }
                codec::put_varu(out, b as u64);
                codec::put_varu(out, n as u64);
                i += n;
            }
        }
    }

    pub fn decode(r: &mut Reader) -> Result<Self, CodecError> {
        let count = r.varu()? as usize;
        let mut layers = Vec::with_capacity(count.min(128));
        for _ in 0..count {
            let raw = r.bytes(2)?;
            let cy = i16::from_le_bytes([raw[0], raw[1]]);
            let mut layer = Box::new([0u16; LAYER]);
            let mut i = 0;
            while i < LAYER {
                let b = r.varu()? as u16;
                let n = r.varu()? as usize;
                if n == 0 || i + n > LAYER {
                    return Err(CodecError::Corrupt("cave biomes"));
                }
                layer[i..i + n].fill(b);
                i += n;
            }
            layers.push((cy, layer));
        }
        layers.sort_by_key(|l| l.0);
        layers.dedup_by_key(|l| l.0);
        Ok(Self { layers })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn painted_cells_read_back_and_round_trip() {
        let mut c = CaveBiomes::default();
        assert_eq!(c.paint((2, 6), (-10, -4), (0, 3), 7, |_, _, _| true), 4 * 6 * 3);
        assert_eq!(c.get(8, -40, 0), 7);
        assert_eq!(c.get(8, -12, 0), 0, "above");
        assert_eq!(c.get(30, -40, 0), 0, "beside");
        let mut runs = Vec::new();
        c.column(3, 1, &mut runs);
        assert_eq!(runs, [(-40, -16, 7)]);
        let mut bytes = Vec::new();
        c.encode(&mut bytes);
        let back = CaveBiomes::decode(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(back, c);
        // Rubbed out, nothing is left.
        c.paint((0, 32), (-20, 0), (0, 32), 0, |_, _, _| true);
        assert!(c.is_empty());
    }
}
