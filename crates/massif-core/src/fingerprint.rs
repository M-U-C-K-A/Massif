//! Fingerprints of tiles: 128 bits of the BLAKE3 hash of a tile's cold
//! blob, by which two copies of a map, on two computers, know the tiles
//! they share without sending them (see `docs/versioning.md`).
//!
//! They are computed when needed (writing or reading a contribution),
//! never stored in the `.massif` file: the file's CRC32C detects damage,
//! a fingerprint recognises a tile. The sculpted chunks over a tile have
//! a fingerprint of their own, of the chunks of that tile in order.

use crate::coords::TilePos;
use crate::level::{Level, LevelId};
use crate::voxel::VoxelLayer;
use crate::world::Dimension;
use rayon::prelude::*;
use std::collections::HashMap;

/// The fingerprint of a tile, or of the sculpted chunks over one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Print(pub [u8; 16]);

impl Print {
    /// The fingerprint of a tile's cold blob.
    pub fn of(bytes: &[u8]) -> Print {
        let mut h = blake3::Hasher::new();
        h.update(b"massif tile");
        h.update(bytes);
        Print::from_hash(h.finalize())
    }

    fn from_hash(hash: blake3::Hash) -> Print {
        let mut out = [0u8; 16];
        out.copy_from_slice(&hash.as_bytes()[..16]);
        Print(out)
    }

    /// The 32 hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The fingerprint written by [`Print::hex`].
    pub fn parse(s: &str) -> Option<Print> {
        crate::versions::Id::parse(s).map(|id| Print(id.0))
    }
}

impl std::fmt::Debug for Print {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Print({})", &self.hex()[..8])
    }
}

impl serde::Serialize for Print {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> serde::Deserialize<'de> for Print {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Print, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Print::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a fingerprint: {s:?}")))
    }
}

/// The fingerprints of every tile of a level, the level frozen (its tiles
/// cold; a hot tile is left out), in parallel.
pub fn of_level(level: &Level) -> HashMap<TilePos, Print> {
    level.blobs().unwrap_or_default().par_iter().map(|(pos, blob)| (*pos, Print::of(blob))).collect()
}

/// The fingerprint of the sculpted chunks over `tile`, if it has any: the
/// positions and contents of its chunks, in order.
pub fn of_voxels(voxels: &VoxelLayer, tile: TilePos) -> Option<Print> {
    let mut chunks: Vec<_> = voxels.in_tile(tile).collect();
    if chunks.is_empty() {
        return None;
    }
    chunks.sort_by_key(|(p, _)| *p);
    let mut h = blake3::Hasher::new();
    h.update(b"massif voxels");
    for (pos, chunk) in chunks {
        h.update(&pos.x.to_le_bytes());
        h.update(&pos.y.to_le_bytes());
        h.update(&pos.z.to_le_bytes());
        h.update(&chunk.densities()[..]);
        match chunk.terrains() {
            Some(t) => {
                h.update(&[1]);
                for v in t.iter() {
                    h.update(&v.to_le_bytes());
                }
            }
            None => {
                h.update(&[0]);
            }
        }
    }
    Some(Print::from_hash(h.finalize()))
}

/// The fingerprints of a dimension: of each level's tiles, and of the
/// sculpted chunks over each tile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prints {
    pub levels: HashMap<LevelId, HashMap<TilePos, Print>>,
    pub voxels: HashMap<TilePos, Print>,
}

impl Prints {
    /// Every fingerprint of `dim`, frozen.
    pub fn of(dim: &Dimension) -> Prints {
        let levels = dim.levels.iter().map(|l| (l.id, of_level(l))).collect();
        let mut tiles: Vec<TilePos> = dim.voxels.iter().map(|(p, _)| p.tile()).collect();
        tiles.sort_unstable_by_key(|p| (p.z, p.x));
        tiles.dedup();
        let voxels = tiles.par_iter().filter_map(|&t| of_voxels(&dim.voxels, t).map(|p| (t, p))).collect();
        Prints { levels, voxels }
    }

    /// The fingerprint of tile `pos` of level `level`.
    pub fn tile(&self, level: LevelId, pos: TilePos) -> Option<Print> {
        self.levels.get(&level).and_then(|l| l.get(&pos)).copied()
    }

    /// How many tiles, of every level.
    pub fn len(&self) -> usize {
        self.levels.values().map(HashMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile::Tile;
    use crate::world::{World, WorldMeta};

    #[test]
    fn the_same_tile_has_the_same_print() {
        let mut world = World::new(WorldMeta::default());
        let dim = &mut world.dims[0];
        dim.ground_mut().insert(TilePos::new(0, 0), Tile::ground(64.0, 1, -1));
        dim.ground_mut().insert(TilePos::new(1, 0), Tile::ground(64.0, 1, -1));
        dim.ground_mut().insert(TilePos::new(0, 1), Tile::ground(70.0, 1, -1));
        let mut snapshot = world.snapshot();
        let prints = Prints::of(&snapshot.dims[0]);
        let ground = prints.levels[&LevelId(0)].clone();
        assert_eq!(ground.len(), 3);
        assert_eq!(ground[&TilePos::new(0, 0)], ground[&TilePos::new(1, 0)]);
        assert_ne!(ground[&TilePos::new(0, 0)], ground[&TilePos::new(0, 1)]);
        assert_eq!(Print::parse(&ground[&TilePos::new(0, 0)].hex()), Some(ground[&TilePos::new(0, 0)]));
        assert!(prints.voxels.is_empty());
        // Recomputed, the same.
        assert_eq!(Prints::of(&snapshot.snapshot().dims[0]), prints);
    }
}
