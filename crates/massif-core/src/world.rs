//! Worlds and dimensions.

use crate::coords::{BlockRect, TilePos, TILE_SIZE};
use crate::tile::Tile;
use crate::voxel::ChunkPos;
use std::collections::HashSet;
use crate::level::{Level, LevelId, LevelKind};
use crate::registry::Registry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldMeta {
    pub name: String,
    pub seed: i64,
    /// Spawn point in block coordinates (x, y, z).
    pub spawn: [i32; 3],
    /// Default fluid surface elevation of new tiles (63 = vanilla sea level).
    pub sea_level: i32,
    /// The world's fluid is lava rather than water.
    #[serde(default)]
    pub lava: bool,
    /// The terrain graph of the world, if one was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<crate::graph::Graph>,
    /// The terrain graph of Massif 1.3 on (`massif-terrain`'s format, a
    /// `massif.terrain-graph` document), kept as written: the world never
    /// reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain_graph: Option<serde_json::Value>,
    /// The ores the export lays in the rock (as Minecraft's own, unless
    /// set otherwise).
    #[serde(default)]
    pub ores: crate::ores::OreSettings,
    /// The gradients of the Gradient brush kept with the map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gradients: Vec<crate::gradient::Gradient>,
    /// The terrain the map was filled with (a new map of one terrain), or
    /// the one chosen for it since: what Paint terrain's right button puts
    /// back. `None`: the height chose the terrains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<crate::registry::TerrainId>,
    /// The zones Paint terrain paints by height with (`None`: Classic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint_rules: Option<crate::paint_rules::ZoneRules>,
    /// What the built-in terrains are made of: the game's blocks or a
    /// mod's (see [`crate::materials`]).
    #[serde(default, skip_serializing_if = "crate::materials::Palette::is_vanilla")]
    pub palette: crate::materials::Palette,
    /// Smooth terrain: the top of the land built of partial blocks when it
    /// is exported (see [`crate::shapes`]).
    #[serde(default, skip_serializing_if = "crate::shapes::SmoothSettings::is_off")]
    pub smooth: crate::shapes::SmoothSettings,
    /// The camera path of the Render side's animation, when one was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<crate::animation::Animation>,
    /// Pictures laid over or under the map to trace over: shown only,
    /// never exported.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<crate::reference::ReferenceImage>,
    /// The map's identifier, the same in every copy of it: given at its
    /// first version (see [`World::map_id`]), so that versions and
    /// contributions know which map they belong to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_id: Option<crate::versions::Id>,
    /// The version the map stands on: the save point made or put back
    /// last, the parent of the next one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<crate::versions::Id>,
    /// The part of another map this map was cut from (File ▸ Export
    /// Region… as a Massif map): merged back, only these tiles are its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<crate::region_map::RegionOf>,
    /// The rules, game mode and difficulty last exported with: the export
    /// dialog offers them again. `None`: never chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_rules: Option<ExportRules>,
    /// What the import of the WorldPainter world it was made of brought in
    /// and left out (`massif_legacy::ImportReport`), to read again: File ▸
    /// Import Report…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_report: Option<serde_json::Value>,
}

/// What the world is played with, kept with the map from one export to the
/// next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportRules {
    pub rules: crate::rules::GameRules,
    /// 0 survival, 1 creative, 2 adventure, 3 spectator, as the game numbers
    /// them.
    pub game_mode: u8,
    /// 0 peaceful to 3 hard.
    pub difficulty: u8,
}

impl Default for WorldMeta {
    fn default() -> Self {
        Self {
            name: "Untitled World".into(),
            seed: 0,
            spawn: [0, 80, 0],
            sea_level: 63,
            lava: false,
            graph: None,
            terrain_graph: None,
            ores: crate::ores::OreSettings::default(),
            gradients: Vec::new(),
            fill: None,
            paint_rules: None,
            palette: Default::default(),
            smooth: Default::default(),
            animation: None,
            references: Vec::new(),
            map_id: None,
            head: None,
            region: None,
            export_rules: None,
            import_report: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionKind {
    Overworld,
    Nether,
    End,
}

impl DimensionKind {
    pub const fn label(self) -> &'static str {
        match self {
            DimensionKind::Overworld => "Surface",
            DimensionKind::Nether => "Nether",
            DimensionKind::End => "End",
        }
    }
}

/// A dimension: build limits plus the ordered stack of levels.
///
/// `levels[0]` is always the ground. The remaining levels are applied in
/// order when a column is composed, so a later level wins where two overlap.
#[derive(Debug, Clone)]
pub struct Dimension {
    pub kind: DimensionKind,
    /// Lowest block Y (inclusive). Vanilla overworld: -64.
    pub min_y: i32,
    /// Highest block Y (exclusive). Vanilla overworld: 320.
    pub max_y: i32,
    pub levels: Vec<Level>,
    /// Sculpted 3D chunks, over the levels.
    pub voxels: crate::voxel::VoxelLayer,
    /// The part of the map the tools may change; `None`: all of it.
    pub selection: Option<std::sync::Arc<crate::selection::Selection>>,
    /// Its cave layers, which the ground's tiles paint by number (see
    /// [`crate::caves`]).
    pub caves: crate::caves::CaveLayers,
    /// The structures placed on it, in the order they were placed: where
    /// two meet, the later wins.
    pub structures: Vec<std::sync::Arc<crate::structures::Structure>>,
    next_level_id: u32,
}

impl Dimension {
    pub fn new(kind: DimensionKind, min_y: i32, max_y: i32) -> Self {
        assert!(min_y < max_y, "a dimension needs a positive height");
        Self {
            kind,
            min_y,
            max_y,
            levels: vec![Level::new(LevelId(0), "Ground", LevelKind::Ground)],
            voxels: Default::default(),
            selection: None,
            caves: Default::default(),
            structures: Vec::new(),
            next_level_id: 1,
        }
    }

    /// Rebuild a dimension from loaded levels (the first must be the ground).
    pub fn from_levels(kind: DimensionKind, min_y: i32, max_y: i32, levels: Vec<Level>) -> Self {
        assert!(
            levels.first().is_some_and(|l| l.kind == LevelKind::Ground),
            "the first level of a dimension must be the ground"
        );
        let next_level_id = levels.iter().map(|l| l.id.0).max().unwrap_or(0) + 1;
        Self {
            kind,
            min_y,
            max_y,
            levels,
            voxels: Default::default(),
            selection: None,
            caves: Default::default(),
            structures: Vec::new(),
            next_level_id,
        }
    }

    pub fn ground(&self) -> &Level {
        &self.levels[0]
    }

    pub fn ground_mut(&mut self) -> &mut Level {
        &mut self.levels[0]
    }

    /// Append a slab level on top of the stack and return its index.
    pub fn add_level(&mut self, name: impl Into<String>, kind: LevelKind) -> usize {
        assert!(kind.has_bottom(), "a dimension has exactly one ground level");
        let id = LevelId(self.next_level_id);
        self.next_level_id += 1;
        self.levels.push(Level::new(id, name, kind));
        self.levels.len() - 1
    }

    /// Remove a slab level. The ground cannot be removed.
    pub fn remove_level(&mut self, index: usize) -> Option<Level> {
        if index == 0 || index >= self.levels.len() {
            return None;
        }
        Some(self.levels.remove(index))
    }

    pub fn level_index(&self, id: LevelId) -> Option<usize> {
        self.levels.iter().position(|l| l.id == id)
    }

    /// Replace the whole level stack (used by structural undo).
    pub fn replace_levels(&mut self, mut levels: Vec<Level>) -> Vec<Level> {
        assert!(levels.first().is_some_and(|l| l.kind == LevelKind::Ground));
        self.next_level_id = self
            .next_level_id
            .max(levels.iter().map(|l| l.id.0).max().unwrap_or(0) + 1);
        for l in &mut levels {
            l.mark_all_dirty();
        }
        std::mem::replace(&mut self.levels, levels)
    }

    /// Cold copies of every level (see [`Level::snapshot`]).
    pub fn snapshot_levels(&mut self) -> Vec<Level> {
        self.levels.iter_mut().map(Level::snapshot).collect()
    }

    /// A fully cold copy of the dimension.
    pub fn snapshot(&mut self) -> Dimension {
        Dimension {
            kind: self.kind,
            min_y: self.min_y,
            max_y: self.max_y,
            levels: self.snapshot_levels(),
            voxels: self.voxels.clone(),
            selection: self.selection.clone(),
            caves: self.caves.clone(),
            structures: self.structures.clone(),
            next_level_id: self.next_level_id,
        }
    }

    /// A copy that encodes nothing (see [`Level::copy_unfrozen`]).
    pub fn copy_unfrozen(&self) -> Dimension {
        Dimension {
            kind: self.kind,
            min_y: self.min_y,
            max_y: self.max_y,
            levels: self.levels.iter().map(Level::copy_unfrozen).collect(),
            voxels: self.voxels.clone(),
            selection: self.selection.clone(),
            caves: self.caves.clone(),
            structures: self.structures.clone(),
            next_level_id: self.next_level_id,
        }
    }

    /// The boxes of the structures placed that reach into the columns of
    /// tile `tile`: each piece's box (the whole structure's, for one built
    /// without pieces), as `(min, max)` in blocks, `max` exclusive. The
    /// caves keep their water and lava away from them, so that none runs
    /// into an Ancient City's halls.
    pub fn structure_boxes(&self, tile: TilePos) -> Vec<([i32; 3], [i32; 3])> {
        let rect = tile.rect();
        let mut out = Vec::new();
        for s in &self.structures {
            if s.columns().intersect(&rect).is_empty() {
                continue;
            }
            let mut add = |min: [i32; 3], max: [i32; 3]| {
                if !BlockRect::new(min[0], min[2], max[0] + 1, max[2] + 1).intersect(&rect).is_empty() {
                    out.push((min, [max[0] + 1, max[1] + 1, max[2] + 1]));
                }
            };
            if s.pieces.is_empty() {
                add(s.min, s.max);
            } else {
                for p in &s.pieces {
                    add(p.min, p.max);
                }
            }
        }
        out
    }

    /// Inclusive tile bounds of the ground level: the extent of the map.
    pub fn bounds(&self) -> Option<(TilePos, TilePos)> {
        self.ground().bounds()
    }

    /// Keep at most `max_hot` decoded tiles per level.
    pub fn trim(&mut self, max_hot: usize) {
        for level in &mut self.levels {
            level.trim(max_hot);
        }
    }

    /// Make the map the whole tiles of `rect`: the tiles of every level and
    /// the sculpted chunks outside it dropped, the ground filled with
    /// `blank` tiles where it has none inside it, the selection dropped.
    /// Returns the ground tiles added.
    pub fn resize(&mut self, rect: BlockRect, blank: impl Fn() -> Tile) -> Vec<TilePos> {
        let inside: HashSet<TilePos> = rect.tiles().collect();
        for level in &mut self.levels {
            let out: Vec<TilePos> = level.positions().filter(|p| !inside.contains(p)).collect();
            for pos in out {
                level.remove(pos);
            }
        }
        let out: Vec<ChunkPos> = self.voxels.iter().map(|(p, _)| p).filter(|p| !inside.contains(&p.tile())).collect();
        for pos in out {
            self.voxels.remove(pos);
        }
        // A structure stays while some of it is on the map still.
        self.structures.retain(|s| !s.columns().intersect(&rect).is_empty());
        self.selection = None;
        let ground = self.ground_mut();
        let added: Vec<TilePos> = rect.tiles().filter(|p| !ground.contains(*p)).collect();
        for &pos in &added {
            ground.insert(pos, blank());
        }
        added
    }
}

/// `width × depth` blocks, made whole tiles, around the middle of `map`
/// (itself on the bounds of tiles): what [`Dimension::resize`] takes to give
/// a map that size while keeping its middle in place.
pub fn resized_rect(map: BlockRect, width: i32, depth: i32) -> BlockRect {
    let tiles = |blocks: i32| (blocks as f64 / TILE_SIZE as f64).round().max(1.0) as i32;
    let (tx, tz) = (tiles(width), tiles(depth));
    let (ox, oz) = (map.x0.div_euclid(TILE_SIZE), map.z0.div_euclid(TILE_SIZE));
    let (ow, od) = (tiles(map.width()), tiles(map.depth()));
    let x0 = ox + (ow - tx).div_euclid(2);
    let z0 = oz + (od - tz).div_euclid(2);
    BlockRect::new(x0 * TILE_SIZE, z0 * TILE_SIZE, (x0 + tx) * TILE_SIZE, (z0 + tz) * TILE_SIZE)
}

#[derive(Debug, Clone)]
pub struct World {
    pub meta: WorldMeta,
    pub registry: Registry,
    pub dims: Vec<Dimension>,
    /// Named copies of the map to go back to (see [`crate::versions`]),
    /// oldest first.
    pub points: Vec<crate::versions::SavePoint>,
}

impl World {
    /// An empty overworld with the vanilla registry and build limits.
    pub fn new(meta: WorldMeta) -> Self {
        Self {
            meta,
            registry: Registry::vanilla(),
            dims: vec![Dimension::new(DimensionKind::Overworld, -64, 320)],
            points: Vec::new(),
        }
    }

    /// The map's identifier, given now if it had none.
    pub fn map_id(&mut self) -> crate::versions::Id {
        *self.meta.map_id.get_or_insert_with(crate::versions::Id::random)
    }

    /// A fully cold copy, cheap to send to a background thread.
    pub fn snapshot(&mut self) -> World {
        World {
            meta: self.meta.clone(),
            registry: self.registry.clone(),
            dims: self.dims.iter_mut().map(Dimension::snapshot).collect(),
            points: self.points.clone(),
        }
    }

    /// A copy taken without encoding anything, so without holding up the
    /// editor (see [`Level::copy_unfrozen`]): [`World::snapshot`] of it,
    /// on another thread, gives what [`World::snapshot`] of this world
    /// would.
    pub fn copy_unfrozen(&self) -> World {
        World {
            meta: self.meta.clone(),
            registry: self.registry.clone(),
            dims: self.dims.iter().map(Dimension::copy_unfrozen).collect(),
            points: self.points.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ids_stay_unique() {
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320);
        let a = d.add_level("Island", LevelKind::Solid);
        let b = d.add_level("Cave", LevelKind::Carve);
        assert_eq!((a, b), (1, 2));
        let removed = d.remove_level(1).unwrap();
        assert_eq!(removed.name, "Island");
        assert!(d.remove_level(0).is_none());
        let c = d.add_level("Arch", LevelKind::Solid);
        let ids: Vec<_> = d.levels.iter().map(|l| l.id.0).collect();
        assert_eq!(ids, vec![0, 2, 3]);
        assert_eq!(d.level_index(d.levels[c].id), Some(c));
    }

    /// A map of 4 × 2 tiles made 2 × 4 keeps its middle: the tiles out of
    /// the new size go from every level, with the sculpture over them, and
    /// the ground is filled where it grows.
    #[test]
    fn a_map_resized_keeps_its_middle() {
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320);
        let slab = d.add_level("Island", LevelKind::Solid);
        for x in 0..4 {
            for z in 0..2 {
                d.ground_mut().insert(TilePos::new(x, z), Tile::ground(70.0, 1, -1));
            }
        }
        d.levels[slab].insert(TilePos::new(0, 0), Tile::ground(90.0, 1, -1));
        d.levels[slab].insert(TilePos::new(1, 1), Tile::ground(90.0, 1, -1));
        let chunk = |x, z| ChunkPos::new(x * 8, 4, z * 8);
        d.voxels.insert(chunk(0, 0), std::sync::Arc::new(crate::voxel::VoxelChunk::air()));
        d.voxels.insert(chunk(2, 1), std::sync::Arc::new(crate::voxel::VoxelChunk::air()));
        let map = BlockRect::new(0, 0, 4 * TILE_SIZE, 2 * TILE_SIZE);
        let rect = resized_rect(map, 2 * TILE_SIZE, 4 * TILE_SIZE);
        assert_eq!(rect, BlockRect::new(TILE_SIZE, -TILE_SIZE, 3 * TILE_SIZE, 3 * TILE_SIZE));
        let added = d.resize(rect, || Tile::ground(10.0, 0, -1));
        assert_eq!(added.len(), 4);
        assert_eq!(d.bounds(), Some((TilePos::new(1, -1), TilePos::new(2, 2))));
        assert_eq!(d.ground().len(), 8);
        let slab_tiles: Vec<TilePos> = d.levels[slab].positions().collect();
        assert_eq!(slab_tiles, vec![TilePos::new(1, 1)]);
        let chunks: Vec<ChunkPos> = d.voxels.iter().map(|(p, _)| p).collect();
        assert_eq!(chunks, vec![chunk(2, 1)]);
    }

    /// Sizes are made whole tiles, at least one; the same size keeps the
    /// map where it is, an odd number of tiles more takes it to the west
    /// and the north.
    #[test]
    fn resized_rects_are_whole_tiles() {
        let map = BlockRect::new(-512, -256, 512, 256);
        assert_eq!(resized_rect(map, 1024, 512), map);
        assert_eq!(resized_rect(map, 1000, 530), map);
        assert_eq!(resized_rect(map, 10, 10), BlockRect::new(-128, -128, 0, 0));
        assert_eq!(resized_rect(map, 1152, 640), BlockRect::new(-640, -384, 512, 256));
        assert_eq!(resized_rect(map, 32_768, 32_768).width(), 32_768);
    }
}
