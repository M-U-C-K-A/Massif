//! Structures placed on the map: an Ancient City, a village, a Trial
//! Chambers, a builder's house.
//!
//! A structure is kept with the map as what it sets in the world, resolved
//! once when it is placed (by the `massif-structures` crate, from the
//! player's own game): its blocks — air too, which carves its halls out of
//! the rock —, its block entities (chests with their loot tables, spawners,
//! beds, signs) and its entities (villagers, animals). The export writes
//! them over the land, the map and the 3D view draw them, and the pieces it
//! was assembled from are kept to tell the game where it stands.

use crate::coords::BlockRect;
use serde::{Deserialize, Serialize};

/// The blocks of a structure in one section of 16³ blocks of the world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// The section's coordinates: those of its blocks divided by 16.
    pub pos: [i32; 3],
    /// Each block's place in the section (`y << 8 | z << 4 | x`) and its
    /// state, an index into [`Structure::palette`]; sorted by place.
    #[serde(with = "packed")]
    pub blocks: Vec<(u16, u16)>,
}

impl Section {
    /// The place of block `(x, y, z)` (world coordinates) in its section.
    #[inline]
    pub fn index(x: i32, y: i32, z: i32) -> u16 {
        ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as u16
    }

    /// The world coordinates of the block at place `i`.
    #[inline]
    pub fn block(&self, i: u16) -> [i32; 3] {
        let i = i as i32;
        [self.pos[0] * 16 + (i & 15), self.pos[1] * 16 + (i >> 8), self.pos[2] * 16 + (i >> 4 & 15)]
    }
}

/// A chest, a spawner, a bed: a block's data, as the game keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEntity {
    pub pos: [i32; 3],
    /// Its NBT, a compound as a file of NBT holds it (an unnamed root):
    /// its `id` and its data, without its coordinates, which the export
    /// writes.
    #[serde(with = "bytes")]
    pub nbt: Vec<u8>,
}

/// A villager, a horse, an armor stand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub pos: [f64; 3],
    /// Its NBT as a file of NBT holds it: its `id`, `Pos`, `Rotation`,
    /// `UUID` and the rest.
    #[serde(with = "bytes")]
    pub nbt: Vec<u8>,
}

/// One piece a structure was assembled from: a template placed in a box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    /// Its box, both corners inclusive.
    pub min: [i32; 3],
    pub max: [i32; 3],
    /// The template (`minecraft:ancient_city/structures/barracks`); empty
    /// for what has none (a tree, a road Massif laid).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
    /// Where the template's corner went, and its quarter turns clockwise
    /// seen from above.
    pub origin: [i32; 3],
    pub turn: u8,
    /// How many jigsaw connections away from the first piece it is.
    #[serde(default)]
    pub depth: u16,
    /// Blocks from the bottom of its box to the ground it stands on.
    #[serde(default)]
    pub ground_delta: i32,
    /// Placed as it is, rather than following the land block by block.
    #[serde(default)]
    pub rigid: bool,
    /// The processors it was placed with, as the game names their list
    /// (empty: none, or some of its own).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub processors: String,
    /// For a building of a village, the column before its door the draw
    /// gave it: what changes made to it by hand name it by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<[i32; 2]>,
}

/// A structure placed on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    /// Its number in the map, never given twice.
    pub id: u32,
    /// What the player sees: "Ancient City", "Plains village".
    pub name: String,
    /// The game's own name of the structure (`minecraft:ancient_city`), to
    /// tell the game where it stands; empty for one of Massif's.
    #[serde(default)]
    pub kind: String,
    /// The draw it was assembled from: the same draw, at the same place,
    /// assembles it the same.
    pub seed: u64,
    /// Block states, `minecraft:chest[facing=north,type=single,waterlogged=false]`.
    pub palette: Vec<String>,
    /// Sorted by the column of sections (x, then z), then by height.
    pub sections: Vec<Section>,
    #[serde(default)]
    pub block_entities: Vec<BlockEntity>,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub pieces: Vec<Piece>,
    /// The box of its blocks, both corners inclusive.
    pub min: [i32; 3],
    pub max: [i32; 3],
    /// Its blocks that can hold water do where they are set in still water,
    /// as the game's structures do (Trial Chambers' do not).
    #[serde(default)]
    pub waterlog: bool,
    /// The biome it brings with it, between the bottom and the top of its
    /// box (`minecraft:deep_dark` for an Ancient City); none: the land's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub biome: Option<String>,
    /// The data version of the game it was read from.
    #[serde(default)]
    pub data_version: i32,
    /// The colour of each entry of its palette, the game's own when it was
    /// known (see [`Structure::color`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<[u8; 3]>,
    /// Who placed it: a tool by hand, or the terrain graph (which replaces
    /// its own as it applies again, never those placed by hand).
    #[serde(default, skip_serializing_if = "StructureSource::is_user")]
    pub source: StructureSource,
    /// How to make it again (its kind, place, settings and draw), as the
    /// tool that placed it writes it (JSON): what lets it be moved,
    /// resized or drawn again. None for those placed before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// The land it levelled, as it was: each column's height before, so
    /// that taking it away or making it again leaves the land as it found
    /// it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ground: Vec<(i32, i32, f32)>,
}

/// Who placed a structure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureSource {
    /// A tool, by hand: the Structure tool, the village brush.
    #[default]
    User,
    /// The terrain graph, when it applied revision `revision` of itself.
    Graph { revision: u64 },
}

impl StructureSource {
    pub fn is_user(&self) -> bool {
        *self == StructureSource::User
    }
}

impl Structure {
    /// The colour of palette entry `k`.
    pub fn color(&self, k: u16) -> [u8; 3] {
        match self.colors.get(k as usize) {
            Some(&c) => c,
            None => self.palette.get(k as usize).map_or([125, 125, 125], |s| crate::objects::guess_color(s)),
        }
    }

    /// Whether palette entry `k` is air, which shows nothing.
    pub fn is_air(&self, k: u16) -> bool {
        self.palette.get(k as usize).is_none_or(|s| crate::objects::is_nothing(s))
    }

    /// Its topmost block in each of its columns, by column of sections:
    /// `(x, z) → (height, state)`, air left out.
    pub fn tops(&self) -> std::collections::HashMap<(i32, i32), (i32, u16)> {
        let mut out: std::collections::HashMap<(i32, i32), (i32, u16)> = std::collections::HashMap::new();
        for sec in &self.sections {
            for &(i, k) in &sec.blocks {
                if self.is_air(k) {
                    continue;
                }
                let [x, y, z] = sec.block(i);
                let e = out.entry((x, z)).or_insert((y, k));
                if y > e.0 {
                    *e = (y, k);
                }
            }
        }
        out
    }

    /// The columns its blocks stand in.
    pub fn columns(&self) -> BlockRect {
        BlockRect::new(self.min[0], self.min[2], self.max[0] + 1, self.max[2] + 1)
    }

    /// How many blocks it sets.
    pub fn block_count(&self) -> usize {
        self.sections.iter().map(|s| s.blocks.len()).sum()
    }

    /// Its sections in the column of sections `(sx, sz)`, low to high.
    pub fn sections_at(&self, sx: i32, sz: i32) -> &[Section] {
        let lo = self.sections.partition_point(|s| (s.pos[0], s.pos[2]) < (sx, sz));
        let hi = lo + self.sections[lo..].partition_point(|s| (s.pos[0], s.pos[2]) == (sx, sz));
        &self.sections[lo..hi]
    }

    /// Its blocks in column `(x, z)`, low to high: their height and state.
    pub fn column(&self, x: i32, z: i32, out: &mut Vec<(i32, u16)>) {
        out.clear();
        let (lx, lz) = (x & 15, z & 15);
        for s in self.sections_at(x.div_euclid(16), z.div_euclid(16)) {
            for ly in 0..16 {
                let i = (ly << 8 | lz << 4 | lx) as u16;
                if let Ok(k) = s.blocks.binary_search_by_key(&i, |b| b.0) {
                    out.push((s.pos[1] * 16 + ly, s.blocks[k].1));
                }
            }
        }
    }

    /// Put its sections in the order [`Structure::sections_at`] reads them,
    /// and its box around its blocks.
    pub fn finish(&mut self) {
        self.sections.retain(|s| !s.blocks.is_empty());
        self.sections.sort_by_key(|s| (s.pos[0], s.pos[2], s.pos[1]));
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for s in &self.sections {
            for &(i, _) in &s.blocks {
                let p = s.block(i);
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
        }
        for e in &self.block_entities {
            for a in 0..3 {
                lo[a] = lo[a].min(e.pos[a]);
                hi[a] = hi[a].max(e.pos[a]);
            }
        }
        if lo[0] > hi[0] {
            (lo, hi) = ([0; 3], [0; 3]);
        }
        (self.min, self.max) = (lo, hi);
    }

    /// Whether it is air or the rock it was carved from where nothing of it
    /// stands: whether its top is under the land around (an Ancient City)
    /// rather than standing on it (a village). Told from its pieces.
    pub fn is_underground(&self, surface_at: impl Fn(i32, i32) -> i32) -> bool {
        let r = self.columns();
        let mid = ((r.x0 + r.x1) / 2, (r.z0 + r.z1) / 2);
        self.max[1] < surface_at(mid.0, mid.1) - 4
    }
}

/// Give the columns of `heights` the height of their top, the first free
/// block above it (the land flattened under a structure's buildings), on
/// the level edited.
pub fn set_heights(edit: &mut crate::history::Edit, heights: &std::collections::HashMap<(i32, i32), i32>) {
    if heights.is_empty() {
        return;
    }
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, z) in heights.keys() {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x + 1), z1.max(z + 1));
    }
    let rect = BlockRect::new(x0, z0, x1, z1);
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let top = tile.top.dense_mut();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                if let Some(&h) = heights.get(&(x, z)) {
                    // Elevation `h` puts the first free block at `h`.
                    top[crate::coords::local_index(x, z)] = crate::codec::snap_height((h as f32).clamp(lo, hi));
                }
            }
        }
    });
}

/// The heights of the columns of `heights`, as they are before a
/// structure levels them (for [`Structure::ground`]).
pub fn heights_before(dim: &crate::world::Dimension, heights: &std::collections::HashMap<(i32, i32), i32>) -> Vec<(i32, i32, f32)> {
    let mut out: Vec<(i32, i32, f32)> = heights.keys().filter_map(|&(x, z)| crate::ops::height_at(dim, 0, x, z, crate::ops::Surface::Top).map(|h| (x, z, h))).collect();
    out.sort_by_key(|e| (e.0, e.1));
    out
}

/// Give the columns of `ground` back the heights they had.
pub fn restore_ground(edit: &mut crate::history::Edit, ground: &[(i32, i32, f32)]) {
    if ground.is_empty() {
        return;
    }
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let map: std::collections::HashMap<(i32, i32), f32> = ground.iter().map(|&(x, z, h)| ((x, z), h)).collect();
    let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(x, z) in map.keys() {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x + 1), z1.max(z + 1));
    }
    let rect = BlockRect::new(x0, z0, x1, z1);
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let top = tile.top.dense_mut();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                if let Some(&h) = map.get(&(x, z)) {
                    top[crate::coords::local_index(x, z)] = crate::codec::snap_height(h.clamp(lo, hi));
                }
            }
        }
    });
}

/// The number the next structure placed in `structures` takes.
pub fn next_id(structures: &[std::sync::Arc<Structure>]) -> u32 {
    structures.iter().map(|s| s.id).max().map_or(1, |m| m + 1)
}

/// Blocks of sections in files: zstd-compressed little-endian `u16` pairs,
/// in base 64.
mod packed {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(blocks: &[(u16, u16)], s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = blocks.iter().flat_map(|&(i, k)| [i.to_le_bytes(), k.to_le_bytes()]).flatten().collect();
        let packed = zstd::bulk::compress(&bytes, 9).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&crate::objects::base64::encode(&packed))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(u16, u16)>, D::Error> {
        let text = String::deserialize(d)?;
        let packed = crate::objects::base64::decode(&text).ok_or_else(|| serde::de::Error::custom("the blocks of a structure are not in base 64"))?;
        let bytes = crate::codec::zstd_decompress_section(&packed).map_err(serde::de::Error::custom)?;
        let mut out: Vec<(u16, u16)> = bytes.as_chunks::<4>().0.iter().map(|c| (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]))).collect();
        // Sorted and once each, whatever the file says: lookups search them.
        out.sort_by_key(|b| b.0);
        out.dedup_by_key(|b| b.0);
        out.retain(|b| b.0 < 4096);
        Ok(out)
    }
}

/// NBT in files: in base 64.
mod bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&crate::objects::base64::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        crate::objects::base64::decode(&text).ok_or_else(|| serde::de::Error::custom("the data of a structure's block is not in base 64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn hut() -> Structure {
        let mut s = Structure {
            id: 1,
            name: "Hut".into(),
            kind: String::new(),
            seed: 7,
            palette: vec!["minecraft:air".into(), "minecraft:oak_planks".into(), "minecraft:chest[facing=north,type=single,waterlogged=false]".into()],
            sections: Vec::new(),
            block_entities: vec![BlockEntity { pos: [1, 65, 1], nbt: vec![10, 0, 0, 0] }],
            entities: Vec::new(),
            pieces: Vec::new(),
            min: [0; 3],
            max: [0; 3],
            waterlog: true,
            biome: None,
            data_version: 4671,
            colors: Vec::new(),
            source: StructureSource::User,
            recipe: None,
            ground: Vec::new(),
        };
        // A floor of planks 3 × 3 at y 64 across a section's edge, air
        // above it, a chest in the middle.
        let mut sections: std::collections::BTreeMap<[i32; 3], Vec<(u16, u16)>> = Default::default();
        for x in -1i32..2 {
            for z in 0i32..3 {
                for y in 64i32..67 {
                    let k = if y == 64 { 1 } else if (x, y, z) == (0, 65, 1) { 2 } else { 0 };
                    sections.entry([x.div_euclid(16), y.div_euclid(16), z.div_euclid(16)]).or_default().push((Section::index(x, y, z), k));
                }
            }
        }
        s.sections = sections
            .into_iter()
            .map(|(pos, mut blocks)| {
                blocks.sort_by_key(|b| b.0);
                Section { pos, blocks }
            })
            .collect();
        s.finish();
        s
    }

    #[test]
    fn structures_read_by_column_and_round_trip() {
        let s = hut();
        assert_eq!((s.min, s.max), ([-1, 64, 0], [1, 66, 2]));
        assert_eq!(s.block_count(), 27);
        assert_eq!(s.columns(), BlockRect::new(-1, 0, 2, 3));
        let mut column = Vec::new();
        s.column(0, 1, &mut column);
        assert_eq!(column, vec![(64, 1), (65, 2), (66, 0)]);
        s.column(-1, 2, &mut column);
        assert_eq!(column, vec![(64, 1), (65, 0), (66, 0)]);
        s.column(5, 5, &mut column);
        assert!(column.is_empty());
        let json = serde_json::to_string(&s).unwrap();
        let back: Structure = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert!(!json.contains("source"), "placed by hand: as before");
        let graph = Structure { source: StructureSource::Graph { revision: 9 }, ..s.clone() };
        let back: Structure = serde_json::from_str(&serde_json::to_string(&graph).unwrap()).unwrap();
        assert_eq!(back.source, StructureSource::Graph { revision: 9 });
        assert_eq!(next_id(&[std::sync::Arc::new(s.clone())]), 2);
        assert_eq!(next_id(&[]), 1);
        // The boxes the caves keep their fluids from: its own (it has no
        // pieces), max exclusive, in the tiles it reaches only.
        let mut dim = crate::world::Dimension::new(crate::world::DimensionKind::Overworld, -64, 320);
        dim.structures.push(std::sync::Arc::new(s.clone()));
        assert_eq!(dim.structure_boxes(crate::coords::TilePos::new(0, 0)), vec![([-1, 64, 0], [2, 67, 3])]);
        assert_eq!(dim.structure_boxes(crate::coords::TilePos::new(-1, 0)), vec![([-1, 64, 0], [2, 67, 3])]);
        assert!(dim.structure_boxes(crate::coords::TilePos::new(3, 3)).is_empty());
        // With pieces: theirs.
        let mut t = s;
        t.pieces = vec![Piece { min: [10, 60, 10], max: [19, 70, 19], template: String::new(), origin: [10, 60, 10], turn: 0, depth: 0, ground_delta: 1, rigid: true, processors: String::new(), key: None }];
        t.min = [-1, 60, 0];
        t.max = [19, 70, 19];
        dim.structures[0] = std::sync::Arc::new(t);
        assert_eq!(dim.structure_boxes(crate::coords::TilePos::new(0, 0)), vec![([10, 60, 10], [20, 71, 20])]);
    }
}
