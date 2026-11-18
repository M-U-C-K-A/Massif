//! Joining a contribution to the map (see `docs/versioning.md`): each tile
//! it changes classified against the map as it is now, the tiles grouped
//! into areas to accept or reject, the map's settings it changes listed
//! apart, and the merge applied.
//!
//! A tile is classified by three fingerprints: of the tile in the version
//! the contribution starts from (its *base*, which the contribution
//! records), of the owner's tile now (*mine*) and of the contribution's
//! (*theirs*). If mine is still the base, only the contributor changed the
//! tile: theirs is taken as it is (*new* if there was none, *changed*
//! otherwise). If mine is theirs, there is nothing to do. Otherwise both
//! changed it: a *conflict*, settled by the owner, tile by tile.

use crate::coords::TilePos;
use crate::fingerprint::{self, Print};
use crate::level::{Blob, Level, LevelId};
use crate::patch::Patch;
use crate::tile::Tile;
use crate::world::{Dimension, World, WorldMeta};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What a contribution does to a tile, against the map as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    /// The map has it as the contribution has it already.
    Same,
    /// The map had no tile there, nor the base: the contribution adds it.
    New,
    /// Only the contribution changed it since the base.
    Changed,
    /// Both the map and the contribution changed it since the base.
    Conflict,
}

/// A tile the contribution changes: in one level, or its sculpted chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub dim: usize,
    /// The level; `None`: the sculpted chunks over the tile.
    pub level: Option<LevelId>,
    pub pos: TilePos,
    pub status: Status,
}

/// Tiles of one dimension the contribution changes, touching each other
/// (by a side or a corner): what the owner accepts or rejects at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    /// Numbered from 1, from the north-west.
    pub number: usize,
    pub dim: usize,
    /// Its tiles, sorted by row then column, with the status of each (the
    /// worst of its levels' and sculpted chunks').
    pub tiles: Vec<(TilePos, Status)>,
}

impl Area {
    /// How many of its tiles are new, changed and in conflict.
    pub fn counts(&self) -> (usize, usize, usize) {
        let n = |s| self.tiles.iter().filter(|t| t.1 == s).count();
        (n(Status::New), n(Status::Changed), n(Status::Conflict))
    }

    /// The worst status of its tiles.
    pub fn status(&self) -> Status {
        self.tiles.iter().map(|t| t.1).max().unwrap_or(Status::Same)
    }

    /// The blocks it covers: west, north, east, south.
    pub fn rect(&self) -> [i32; 4] {
        use crate::coords::TILE_SIZE;
        let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (p, _) in &self.tiles {
            x0 = x0.min(p.x);
            z0 = z0.min(p.z);
            x1 = x1.max(p.x);
            z1 = z1.max(p.z);
        }
        [x0 * TILE_SIZE, z0 * TILE_SIZE, (x1 + 1) * TILE_SIZE, (z1 + 1) * TILE_SIZE]
    }
}

/// A setting of the map the contribution changes (sea level, seed…): the
/// owner accepts or refuses each.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    /// Its name in the map's settings (`sea_level`).
    pub key: String,
    pub base: serde_json::Value,
    pub theirs: serde_json::Value,
    pub mine: serde_json::Value,
}

impl SettingChange {
    /// Whether the owner changed it too, otherwise.
    pub fn conflict(&self) -> bool {
        self.mine != self.base
    }
}

/// The settings a contribution may change; the map's name, identifier and
/// version are the owner's.
const SETTINGS: [&str; 10] = ["seed", "spawn", "sea_level", "lava", "graph", "ores", "gradients", "fill", "palette", "smooth"];

/// A contribution classified against the map.
#[derive(Debug, Clone, Default)]
pub struct Review {
    pub items: Vec<Item>,
    pub areas: Vec<Area>,
    pub settings: Vec<SettingChange>,
}

impl Review {
    /// How many tiles (by position) are new, changed and in conflict.
    pub fn counts(&self) -> (usize, usize, usize) {
        self.areas.iter().map(Area::counts).fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
    }

    /// The area holding tile `pos` of dimension `dim`.
    pub fn area_at(&self, dim: usize, pos: TilePos) -> Option<&Area> {
        self.areas.iter().find(|a| a.dim == dim && a.tiles.iter().any(|t| t.0 == pos))
    }
}

/// Whether two tiles hold the same, though their bytes may differ (encoded
/// by another version of Massif, say).
fn same_tile(a: &Blob, b: &Blob) -> bool {
    crate::versions::same_blob(a, b) || matches!((Tile::decode(a), Tile::decode(b)), (Ok(x), Ok(y)) if x == y)
}

/// The status of a tile from its three fingerprints, and the tiles
/// themselves to tell a mere re-encoding from a change.
fn status(base: Option<Print>, mine: Option<&Blob>, theirs: Option<&Blob>) -> Status {
    let mine_print = mine.map(|b| Print::of(b));
    let theirs_print = theirs.map(|b| Print::of(b));
    if mine_print == theirs_print {
        return Status::Same;
    }
    if mine_print == base {
        return if base.is_none() { Status::New } else { Status::Changed };
    }
    match (mine, theirs) {
        (Some(m), Some(t)) if same_tile(m, t) => Status::Same,
        _ => Status::Conflict,
    }
}

/// Classify `patch` against `map` (cold, see [`World::snapshot`]).
pub fn classify(map: &World, patch: &Patch) -> Review {
    let mut items = Vec::new();
    for (d, change) in patch.dims.iter().enumerate() {
        let dim = map.dims.get(d).filter(|m| m.kind == change.kind);
        for level in &change.levels {
            let mine_level = dim.and_then(|m| m.levels.iter().find(|l| l.id == level.id && l.kind == level.kind));
            let mine: HashMap<TilePos, Blob> = mine_level.and_then(Level::blobs).unwrap_or_default().into_iter().collect();
            for t in &level.tiles {
                items.push(Item {
                    dim: d,
                    level: Some(level.id),
                    pos: t.pos,
                    status: status(t.base, mine.get(&t.pos), t.new.as_ref()),
                });
            }
        }
        for v in &change.voxel_tiles {
            let mine = dim.and_then(|m| fingerprint::of_voxels(&m.voxels, v.pos));
            let status = if mine == v.new {
                Status::Same
            } else if mine == v.base {
                if v.base.is_none() { Status::New } else { Status::Changed }
            } else {
                Status::Conflict
            };
            items.push(Item {
                dim: d,
                level: None,
                pos: v.pos,
                status,
            });
        }
    }
    let areas = areas(&items);
    Review {
        items,
        areas,
        settings: settings(&patch.manifest.settings_base, &patch.manifest.settings, &map.meta),
    }
}

/// The changed tiles (all but [`Status::Same`]) grouped into areas.
fn areas(items: &[Item]) -> Vec<Area> {
    // Each position's worst status, by dimension.
    let mut by_dim: BTreeMap<usize, HashMap<TilePos, Status>> = BTreeMap::new();
    for item in items.iter().filter(|i| i.status != Status::Same) {
        let s = by_dim.entry(item.dim).or_default().entry(item.pos).or_insert(item.status);
        *s = (*s).max(item.status);
    }
    let mut out = Vec::new();
    for (dim, tiles) in by_dim {
        let mut left: HashSet<TilePos> = tiles.keys().copied().collect();
        let mut order: Vec<TilePos> = tiles.keys().copied().collect();
        order.sort_unstable_by_key(|p| (p.z, p.x));
        for start in order {
            if !left.remove(&start) {
                continue;
            }
            let mut area = vec![start];
            let mut next = vec![start];
            while let Some(p) = next.pop() {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let q = TilePos::new(p.x + dx, p.z + dz);
                        if left.remove(&q) {
                            area.push(q);
                            next.push(q);
                        }
                    }
                }
            }
            area.sort_unstable_by_key(|p| (p.z, p.x));
            out.push(Area {
                number: 0,
                dim,
                tiles: area.into_iter().map(|p| (p, tiles[&p])).collect(),
            });
        }
    }
    for (k, a) in out.iter_mut().enumerate() {
        a.number = k + 1;
    }
    out
}

/// The settings `theirs` changed since `base`, with the owner's.
fn settings(base: &WorldMeta, theirs: &WorldMeta, mine: &WorldMeta) -> Vec<SettingChange> {
    let as_map = |m: &WorldMeta| serde_json::to_value(m).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
    let (b, t, m) = (as_map(base), as_map(theirs), as_map(mine));
    let null = serde_json::Value::Null;
    SETTINGS
        .iter()
        .filter_map(|&key| {
            let (b, t, m) = (b.get(key).unwrap_or(&null), t.get(key).unwrap_or(&null), m.get(key).unwrap_or(&null));
            (t != b && t != m).then(|| SettingChange {
                key: key.into(),
                base: b.clone(),
                theirs: t.clone(),
                mine: m.clone(),
            })
        })
        .collect()
}

/// What the owner decided: the tiles to take from the contribution (by
/// dimension and position: every level and the sculpted chunks there),
/// and the settings to take.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    pub take: HashSet<(usize, TilePos)>,
    pub settings: HashSet<String>,
}

/// The map with what `decision` takes from `patch`: its settings, registry
/// and dimensions. `map` must be cold, and the numbers in the contribution's
/// tiles must be the map's (terrains, biomes, layers).
pub fn apply(map: &World, patch: &Patch, decision: &Decision) -> (WorldMeta, crate::registry::Registry, Vec<Dimension>) {
    let mut dims = map.dims.clone();
    for (d, change) in patch.dims.iter().enumerate() {
        let wanted = |pos: TilePos| decision.take.contains(&(d, pos));
        let touches = change.levels.iter().flat_map(|l| &l.tiles).any(|t| wanted(t.pos)) || change.voxel_tiles.iter().any(|v| wanted(v.pos));
        if !touches {
            continue;
        }
        // A dimension the map lacks is made for it.
        if dims.get(d).is_none_or(|m| m.kind != change.kind) {
            if dims.iter().any(|m| m.kind == change.kind) || d != dims.len() {
                continue;
            }
            dims.push(Dimension::new(change.kind, change.min_y, change.max_y));
        }
        let dim = &mut dims[d];
        for level in &change.levels {
            if !level.tiles.iter().any(|t| wanted(t.pos)) {
                continue;
            }
            let index = match dim.levels.iter().position(|l| l.id == level.id) {
                Some(i) if dim.levels[i].kind == level.kind => i,
                Some(_) => continue,
                None => {
                    let mut levels = std::mem::take(&mut dim.levels);
                    levels.push(Level::new(level.id, level.name.clone(), level.kind));
                    let (voxels, selection) = (std::mem::take(&mut dim.voxels), dim.selection.take());
                    *dim = Dimension::from_levels(dim.kind, dim.min_y, dim.max_y, levels);
                    dim.voxels = voxels;
                    dim.selection = selection;
                    dim.levels.len() - 1
                }
            };
            let target = &mut dim.levels[index];
            for t in level.tiles.iter().filter(|t| wanted(t.pos)) {
                match &t.new {
                    Some(blob) => target.insert_blob(t.pos, blob.clone()),
                    None => {
                        target.remove(t.pos);
                    }
                }
            }
            target.take_dirty();
        }
        for v in change.voxel_tiles.iter().filter(|v| wanted(v.pos)) {
            let gone: Vec<_> = dim.voxels.in_tile(v.pos).map(|(p, _)| p).collect();
            for p in gone {
                dim.voxels.remove(p);
            }
            let theirs: Vec<_> = change.voxels.in_tile(v.pos).map(|(p, _)| p).collect();
            for p in theirs {
                if let Some(chunk) = change.voxels.get_shared(p) {
                    dim.voxels.insert(p, chunk);
                }
            }
        }
        let _ = dim.voxels.take_dirty();
    }
    // The settings taken.
    let mut meta = map.meta.clone();
    if !decision.settings.is_empty()
        && let (Ok(serde_json::Value::Object(mut mine)), Ok(serde_json::Value::Object(theirs))) = (serde_json::to_value(&meta), serde_json::to_value(&patch.manifest.settings))
    {
        for key in &decision.settings {
            if !SETTINGS.contains(&key.as_str()) {
                continue;
            }
            match theirs.get(key) {
                Some(v) => {
                    mine.insert(key.clone(), v.clone());
                }
                None => {
                    mine.remove(key);
                }
            }
        }
        if let Ok(taken) = serde_json::from_value::<WorldMeta>(serde_json::Value::Object(mine)) {
            meta = taken;
        }
    }
    (meta, map.registry.clone(), dims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::Kind;
    use crate::versions::SavePoint;
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

    /// The owner and a contributor start from the same version, each change
    /// tiles; the contribution is classified against the owner's map.
    fn scene() -> (World, Patch) {
        let mut owner = flat(8);
        let base = SavePoint::new(&mut owner, "base", 1);
        owner.points.push(base.clone());
        let mut theirs = owner.snapshot();
        // The contributor: a hill over (1..3, 1..2), a far tile, (6, 6) as the
        // owner will make it, a new tile at (12, 0), and the sea level.
        for (x, z) in [(1, 1), (2, 1), (1, 2)] {
            set(&mut theirs, x, z, 90.0);
        }
        set(&mut theirs, 6, 6, 70.0);
        set(&mut theirs, 5, 6, 75.0);
        set(&mut theirs, 12, 0, 64.0);
        theirs.meta.sea_level = 60;
        theirs.meta.seed = 99;
        // The owner: (5, 6) otherwise, (6, 6) the same, and the seed.
        set(&mut owner, 5, 6, 80.0);
        set(&mut owner, 6, 6, 70.0);
        owner.meta.seed = 7;
        let patch = Patch::since(&base, &theirs.snapshot(), Kind::Contribution, "Jack", "", 2).unwrap();
        (owner.snapshot(), patch)
    }

    #[test]
    fn tiles_are_classified_by_three_fingerprints() {
        let (map, patch) = scene();
        let review = classify(&map, &patch);
        let of = |x, z| review.items.iter().find(|i| i.pos == TilePos::new(x, z)).unwrap().status;
        assert_eq!(of(1, 1), Status::Changed);
        assert_eq!(of(12, 0), Status::New);
        assert_eq!(of(6, 6), Status::Same);
        assert_eq!(of(5, 6), Status::Conflict);
        // Areas: the hill, the conflict (with its twin beside it, the same,
        // left out), and the far tile.
        assert_eq!(review.areas.len(), 3, "{:?}", review.areas);
        let hill = review.area_at(0, TilePos::new(2, 1)).unwrap();
        assert_eq!(hill.tiles.len(), 3);
        assert_eq!(hill.status(), Status::Changed);
        assert_eq!(hill.rect(), [128, 128, 384, 384]);
        assert_eq!(review.area_at(0, TilePos::new(5, 6)).unwrap().status(), Status::Conflict);
        // Numbered from the north: the far tile, in the first row, first.
        let far = review.area_at(0, TilePos::new(12, 0)).unwrap();
        assert_eq!((far.status(), far.number), (Status::New, 1));
        assert_eq!(review.counts(), (1, 3, 1));
        // Settings: the sea level (only theirs changed it) and the seed (both).
        let keys: Vec<_> = review.settings.iter().map(|s| (s.key.as_str(), s.conflict())).collect();
        assert_eq!(keys, [("seed", true), ("sea_level", false)]);
    }

    /// A tile encoded otherwise but holding the same is not a conflict.
    #[test]
    fn a_tile_encoded_otherwise_is_the_same() {
        let tile = Tile::ground(70.0, 1, -1);
        let a: Blob = tile.encode().into();
        let b: Blob = crate::codec::zstd_compress(&crate::codec::zstd_decompress(&a).unwrap()).into();
        let other: Blob = Tile::ground(71.0, 1, -1).encode().into();
        let base = Some(Print::of(&Tile::ground(64.0, 1, -1).encode()));
        assert_eq!(status(base, Some(&a), Some(&b)), Status::Same);
        assert_eq!(status(base, Some(&a), Some(&other)), Status::Conflict);
        assert_eq!(status(None, None, Some(&a)), Status::New);
        assert_eq!(status(base, None, Some(&a)), Status::Conflict, "removed by the owner, changed by the contributor");
    }

    #[test]
    fn the_merge_takes_what_was_decided() {
        let (map, patch) = scene();
        let review = classify(&map, &patch);
        let mut decision = Decision::default();
        // The hill taken, the conflict settled for theirs, the far tile left.
        for a in review.areas.iter().filter(|a| a.status() != Status::New) {
            decision.take.extend(a.tiles.iter().map(|t| (a.dim, t.0)));
        }
        decision.settings.insert("sea_level".into());
        let (meta, _, mut dims) = apply(&map, &patch, &decision);
        assert_eq!(meta.sea_level, 60);
        assert_eq!(meta.seed, 7, "the owner's seed kept");
        let ground = dims.remove(0).levels.remove(0);
        let top = |l: &Level, x, z| l.view(TilePos::new(x, z)).map(|t| t.top.uniform_value());
        assert_eq!(top(&ground, 1, 1), Some(Some(90.0)));
        assert_eq!(top(&ground, 5, 6), Some(Some(75.0)));
        assert_eq!(top(&ground, 12, 0), None);
        assert_eq!(top(&ground, 7, 7), Some(Some(64.0)));
    }
}
