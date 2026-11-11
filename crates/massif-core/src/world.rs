//! Worlds and dimensions.

use crate::coords::TilePos;
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
            next_level_id: self.next_level_id,
        }
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
}

#[derive(Debug, Clone)]
pub struct World {
    pub meta: WorldMeta,
    pub registry: Registry,
    pub dims: Vec<Dimension>,
}

impl World {
    /// An empty overworld with the vanilla registry and build limits.
    pub fn new(meta: WorldMeta) -> Self {
        Self {
            meta,
            registry: Registry::vanilla(),
            dims: vec![Dimension::new(DimensionKind::Overworld, -64, 320)],
        }
    }

    /// A fully cold copy, cheap to send to a background thread.
    pub fn snapshot(&mut self) -> World {
        World {
            meta: self.meta.clone(),
            registry: self.registry.clone(),
            dims: self.dims.iter_mut().map(Dimension::snapshot).collect(),
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
}
