//! A part of a map as a map of its own, and a map merged into another.
//!
//! The owner of a large map sends a collaborator the part they work on as a
//! `.massif` of its own (File ▸ Export Region… as a Massif map): the whole
//! tiles around the blocks chosen, and a border of tiles around them, at
//! their coordinates, with the map's identifier, settings, terrains and
//! layers, and its versions cut to the same tiles. The collaborator edits
//! it as any map; the owner brings it back with File ▸ Merge Massif Map…
//! ([`patch_from_map`]): its changes since the newest version both maps
//! have are a contribution like a `.massifpatch` (see [`crate::patch`]),
//! classified tile by tile and reviewed before anything changes.
//!
//! A map with no version in common with the owner's (another map, or one
//! made apart) merges too: its tiles that differ from the map as it is are
//! the changes, none of them in conflict, and none taken away.

use crate::coords::TilePos;
use crate::level::Level;
use crate::patch::{Kind, Patch, PatchError};
use crate::coords::TILE_SIZE;
use crate::versions::{Id, SavePoint};
use crate::world::{Dimension, World};
use crate::BlockRect;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Where a map cut from another lies in it: the dimension, and the tiles
/// (west, north, east, south, inclusive). Only these tiles are its own:
/// merged back, the tiles outside it are left as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionOf {
    pub dim: usize,
    pub tiles: [i32; 4],
    /// The blocks asked for, within the tiles (west, north, east, south;
    /// east and south exclusive).
    #[serde(default)]
    pub blocks: [i32; 4],
}

impl RegionOf {
    /// The tiles covering `rect` and `border` more on every side.
    pub fn around(dim: usize, rect: BlockRect, border: i32) -> RegionOf {
        let b = border.max(0);
        RegionOf {
            dim,
            tiles: [
                rect.x0.div_euclid(TILE_SIZE) - b,
                rect.z0.div_euclid(TILE_SIZE) - b,
                (rect.x1 - 1).div_euclid(TILE_SIZE) + b,
                (rect.z1 - 1).div_euclid(TILE_SIZE) + b,
            ],
            blocks: [rect.x0, rect.z0, rect.x1, rect.z1],
        }
    }

    /// Whether tile `pos` of dimension `dim` is the region's.
    pub fn contains(&self, dim: usize, pos: TilePos) -> bool {
        dim == self.dim && pos.x >= self.tiles[0] && pos.x <= self.tiles[2] && pos.z >= self.tiles[1] && pos.z <= self.tiles[3]
    }

    /// How many tiles wide and deep.
    pub fn size(&self) -> (i32, i32) {
        (self.tiles[2] - self.tiles[0] + 1, self.tiles[3] - self.tiles[1] + 1)
    }
}

/// `world` (cold) cut to `region`: the region's tiles of every level and
/// the sculpted chunks over them; the other dimensions keep their levels,
/// empty. Its versions are cut the same way.
pub fn crop(world: &World, region: RegionOf) -> World {
    let dims = world
        .dims
        .iter()
        .enumerate()
        .map(|(d, dim)| {
            let levels = dim
                .levels
                .iter()
                .map(|level| {
                    let mut l = Level::new(level.id, level.name.clone(), level.kind);
                    l.visible = level.visible;
                    for (pos, blob) in level.blobs().unwrap_or_default() {
                        if region.contains(d, pos) {
                            l.insert_blob(pos, blob);
                        }
                    }
                    l
                })
                .collect();
            let mut out = Dimension::from_levels(dim.kind, dim.min_y, dim.max_y, levels);
            for (pos, chunk) in dim.voxels.iter() {
                if region.contains(d, pos.tile()) {
                    out.voxels.insert(pos, chunk.clone());
                }
            }
            out.voxels.take_dirty();
            out
        })
        .collect();
    let mut meta = world.meta.clone();
    meta.region = Some(region);
    let points = world
        .points
        .iter()
        .map(|p| SavePoint {
            world: std::sync::Arc::new(crop(&p.world, region)),
            ..p.clone()
        })
        .collect();
    World { meta, registry: world.registry.clone(), dims, points }
}

/// The part of `world` around `rect` in dimension `dim`, with `border`
/// tiles more on each side, as a map of its own (see [`crop`]). The map is
/// given its identifier if it had none, so that the part knows it.
pub fn region_copy(world: &mut World, dim: usize, rect: BlockRect, border: i32) -> World {
    world.map_id();
    let snapshot = world.snapshot();
    crop(&snapshot, RegionOf::around(dim, rect, border))
}

/// The newest version of `mine` that `theirs` was made from: the first of
/// its versions, walking back from the one it stands on, that `mine` has.
pub fn common_version<'a>(mine: &'a World, theirs: &World) -> Option<&'a SavePoint> {
    if mine.meta.map_id.is_none() || mine.meta.map_id != theirs.meta.map_id {
        return None;
    }
    let find = |id: Id| mine.points.iter().find(|p| p.id == id);
    let mut seen = HashSet::new();
    let mut queue: std::collections::VecDeque<Id> = theirs.meta.head.into_iter().collect();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(p) = find(id) {
            return Some(p);
        }
        if let Some(p) = theirs.points.iter().find(|p| p.id == id) {
            queue.extend(p.parents.iter().copied());
        }
    }
    // No line back from where it stands: the newest version both have.
    theirs.points.iter().filter_map(|p| find(p.id)).max_by_key(|p| p.created)
}

/// What [`patch_from_map`] found.
pub struct FromMap {
    pub patch: Patch,
    /// Whether the two maps have a version in common, the one the changes
    /// are counted from; if not, they are counted from the map as it is.
    pub common: bool,
}

/// The changes `theirs` (a map, cold) brings to `mine` (cold, with its
/// identifier): since the newest version both have, or, with none, the
/// tiles where `theirs` differs from `mine` as it is. A part of a map
/// (see [`crop`]) changes only its own tiles; a map with no version in
/// common takes no tile away.
pub fn patch_from_map(mine: &World, theirs: &World, created: u64) -> Result<FromMap, PatchError> {
    let map = mine.meta.map_id.ok_or(PatchError::NoMap)?;
    let common = common_version(mine, theirs).cloned();
    let is_common = common.is_some();
    let base = match common {
        Some(p) => p,
        None => {
            let mut copy = World { meta: mine.meta.clone(), registry: mine.registry.clone(), dims: mine.dims.clone(), points: Vec::new() };
            copy.meta.head = None;
            SavePoint::of(copy, mine.meta.name.clone(), created)
        }
    };
    let mut now = World { meta: theirs.meta.clone(), registry: theirs.registry.clone(), dims: theirs.dims.clone(), points: theirs.points.clone() };
    now.meta.map_id = Some(map);
    now.meta.region = mine.meta.region;
    let head = theirs.meta.head.and_then(|h| theirs.points.iter().find(|p| p.id == h));
    let author = head.map(|p| p.author.clone()).unwrap_or_default();
    let message = match head {
        Some(p) if is_common && p.id == base.id => String::new(),
        Some(p) => p.name.clone(),
        None => String::new(),
    };
    let mut patch = Patch::since(&base, &now, Kind::Contribution, &author, &message, created)?;
    patch.manifest.map_name = theirs.meta.name.clone();
    patch.manifest.settings.region = mine.meta.region;
    patch.manifest.settings_base.region = mine.meta.region;
    if !is_common {
        // Not a version the map has: the merge's second parent is unknown.
        patch.manifest.head = None;
    }
    let region = theirs.meta.region;
    for (d, dim) in patch.dims.iter_mut().enumerate() {
        let keep = |pos: TilePos| region.is_none_or(|r| r.contains(d, pos));
        for level in &mut dim.levels {
            level.tiles.retain(|t| keep(t.pos) && (is_common || t.new.is_some()));
        }
        dim.levels.retain(|l| !l.tiles.is_empty());
        dim.voxel_tiles.retain(|v| keep(v.pos) && (is_common || v.new.is_some()));
        if region.is_some() || !is_common {
            let kept: HashSet<TilePos> = dim.voxel_tiles.iter().map(|v| v.pos).collect();
            let mut voxels = crate::voxel::VoxelLayer::default();
            for (pos, chunk) in dim.voxels.iter() {
                if kept.contains(&pos.tile()) {
                    voxels.insert(pos, chunk.clone());
                }
            }
            voxels.take_dirty();
            dim.voxels = voxels;
        }
    }
    Ok(FromMap { patch, common: is_common })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::{Decision, Status, apply, classify};
    use crate::tile::Tile;
    use crate::world::WorldMeta;

    fn flat(n: i32) -> World {
        let mut world = World::new(WorldMeta::default());
        for x in 0..n {
            for z in 0..n {
                world.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(64.0, 1, -1));
            }
        }
        world
    }

    fn set(world: &mut World, x: i32, z: i32, h: f32) {
        world.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(h, 1, -1));
    }

    fn height(world: &mut World, x: i32, z: i32) -> f32 {
        world.dims[0].ground_mut().blob(TilePos::new(x, z)).and_then(|b| Tile::decode(&b).ok()).map_or(f32::NAN, |t| t.top.get(0))
    }

    fn take_all(review: &crate::merge::Review) -> Decision {
        Decision { take: review.areas.iter().flat_map(|a| a.tiles.iter().map(move |t| (a.dim, t.0))).collect(), settings: Default::default() }
    }

    /// The owner's map of 8 × 8 tiles with a version; the region of tiles
    /// (2..=4, 2..=4) sent to a collaborator.
    fn sent() -> (World, World) {
        let mut owner = flat(8);
        let v = SavePoint::new(&mut owner, "Sent to Jack", 1);
        owner.points.push(v);
        let rect = BlockRect::new(3 * TILE_SIZE, 3 * TILE_SIZE, 4 * TILE_SIZE, 4 * TILE_SIZE);
        let region = region_copy(&mut owner, 0, rect, 1);
        (owner, region)
    }

    #[test]
    fn a_region_keeps_its_tiles_coordinates_and_versions() {
        let (owner, region) = sent();
        let r = region.meta.region.unwrap();
        assert_eq!(r.tiles, [2, 2, 4, 4]);
        assert_eq!(region.meta.map_id, owner.meta.map_id);
        assert_eq!(region.dims[0].levels[0].blobs().unwrap().len(), 9);
        assert!(region.dims[0].levels[0].blobs().unwrap().iter().all(|(p, _)| r.contains(0, *p)));
        assert_eq!(region.points.len(), 1);
        assert_eq!(region.points[0].world.dims[0].levels[0].blobs().unwrap().len(), 9);
        assert_eq!(common_version(&owner, &region).map(|p| p.name.as_str()), Some("Sent to Jack"));
    }

    /// The collaborator reshapes their region; the owner changed nothing:
    /// every change merges without conflict, and the tiles outside the
    /// region are not taken away.
    #[test]
    fn a_region_merges_without_conflict() {
        let (mut owner, mut region) = sent();
        set(&mut region, 3, 3, 90.0);
        set(&mut region, 4, 3, 85.0);
        let mine = owner.snapshot();
        let from = patch_from_map(&mine, &region.snapshot(), 2).unwrap();
        assert!(from.common);
        assert_eq!(from.patch.tiles(), 2, "only the tiles changed, none taken away outside the region");
        let review = classify(&mine, &from.patch);
        assert_eq!(review.counts(), (0, 2, 0));
        assert_eq!(review.areas.len(), 1);
        let (meta, registry, dims) = apply(&mine, &from.patch, &take_all(&review));
        let mut merged = World { meta, registry, dims, points: Vec::new() };
        assert_eq!(height(&mut merged, 3, 3), 90.0);
        assert_eq!(height(&mut merged, 4, 3), 85.0);
        assert_eq!(height(&mut merged, 0, 0), 64.0);
        assert_eq!(merged.dims[0].levels[0].blobs().unwrap().len(), 64);
        assert_eq!(merged.meta.region, None, "the map stays a whole map");
    }

    /// Both changed tile (3, 3): a conflict; the tile only the collaborator
    /// changed is not.
    #[test]
    fn tiles_both_changed_are_in_conflict() {
        let (mut owner, mut region) = sent();
        set(&mut region, 3, 3, 90.0);
        set(&mut region, 2, 2, 70.0);
        set(&mut owner, 3, 3, 40.0);
        let mine = owner.snapshot();
        let from = patch_from_map(&mine, &region.snapshot(), 2).unwrap();
        let review = classify(&mine, &from.patch);
        let of = |x, z| review.items.iter().find(|i| i.pos == TilePos::new(x, z)).unwrap().status;
        assert_eq!(of(3, 3), Status::Conflict);
        assert_eq!(of(2, 2), Status::Changed);
        // Keeping the owner's tile in conflict: only (2, 2) changes.
        let take = [(0, TilePos::new(2, 2))].into_iter().collect();
        let (meta, registry, dims) = apply(&mine, &from.patch, &Decision { take, settings: Default::default() });
        let mut merged = World { meta, registry, dims, points: Vec::new() };
        assert_eq!(height(&mut merged, 3, 3), 40.0);
        assert_eq!(height(&mut merged, 2, 2), 70.0);
    }

    /// A terrain, a ground cover and a layer level the collaborator made
    /// come into the map with the tiles that use them.
    #[test]
    fn new_terrains_and_layers_come_in() {
        use crate::registry::{CoverDef, TerrainDef};
        let (mut owner, mut region) = sent();
        let n = region.registry.terrains.len() as u16;
        region.registry.terrains.push(TerrainDef { name: "Jack's moss".into(), ..region.registry.terrains[1].clone() });
        let cover = region.registry.add_cover(CoverDef::new("Moss", [60, 120, 60], &[("minecraft:moss_carpet", 1)], 0.5));
        let mut tile = Tile::ground(70.0, n, -1);
        tile.cover = crate::plane::Plane::Uniform(cover);
        region.dims[0].ground_mut().insert(TilePos::new(3, 3), tile);
        let level = region.dims[0].add_level("Jack's roads", crate::level::LevelKind::Solid);
        region.dims[0].levels[level].insert(TilePos::new(3, 3), Tile::ground(71.0, n, -1));
        let mine = owner.snapshot();
        let from = patch_from_map(&mine, &region.snapshot(), 2).unwrap();
        let mut registry = mine.registry.clone();
        let (patch, remap) = from.patch.remapped(&mut registry);
        assert_eq!(remap.added, ["Jack's moss", "Moss"]);
        let review = classify(&mine, &patch);
        let (meta, registry, dims) = apply(&mine, &patch, &take_all(&review));
        let merged = World { meta, registry, dims, points: Vec::new() };
        let moss = merged.registry.terrain_by_name("Jack's moss").unwrap();
        assert!(merged.registry.cover_by_name("Moss").is_some());
        let roads = merged.dims[0].levels.iter().find(|l| l.name == "Jack's roads").expect("the layer level came in");
        assert!(roads.blobs().unwrap().iter().any(|(p, _)| *p == TilePos::new(3, 3)));
        let ground = merged.dims[0].levels[0].blobs().unwrap();
        let t = Tile::decode(&ground.iter().find(|(p, _)| *p == TilePos::new(3, 3)).unwrap().1).unwrap();
        assert_eq!(t.terrain.uniform_value(), Some(moss.0));
    }

    /// A map with no version in common merges its differing tiles, none in
    /// conflict, and takes none away.
    #[test]
    fn a_map_made_apart_merges_its_differences() {
        let mut owner = flat(4);
        owner.map_id();
        let mut other = flat(2);
        set(&mut other, 1, 1, 100.0);
        set(&mut other, 9, 9, 64.0);
        let mine = owner.snapshot();
        let from = patch_from_map(&mine, &other.snapshot(), 2).unwrap();
        assert!(!from.common);
        let review = classify(&mine, &from.patch);
        assert_eq!(review.counts(), (1, 1, 0), "a new tile and a changed one");
        assert_eq!(from.patch.tiles(), 2);
    }
}
