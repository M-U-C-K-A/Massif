//! Contributions: the tiles of a map changed since one of its versions,
//! each with the fingerprint of what it was, to send to the map's owner
//! (see `docs/versioning.md`). `massif_format` writes and reads one as a
//! `.massifpatch` file.
//!
//! A contribution carries only what changed: the tiles of every level that
//! differ from the version it starts from (or that it removed), the
//! sculpted chunks over the tiles where they differ, the map's settings
//! before and after, and the registry its tiles' numbers refer to. The
//! fingerprint of each tile's *base* lets the owner tell, without having
//! that version at hand, whether they changed the tile since: if their tile
//! still has the base's fingerprint, the contribution's can be taken as it
//! is; if not, both sides changed it, and it is a conflict.

use crate::coords::TilePos;
use crate::fingerprint::{self, Print};
use crate::level::{Blob, LevelId, LevelKind};
use crate::registry::Registry;
use crate::versions::{Id, SavePoint, same_blob};
use crate::voxel::VoxelLayer;
use crate::world::{DimensionKind, World, WorldMeta};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The name of the format, written in each contribution's manifest.
pub const FORMAT: &str = "massifpatch/1";
/// The file extension, without the dot.
pub const EXTENSION: &str = "massifpatch";

/// The file written beside the images of a region of the map (see
/// [`crate::mask::export_region`]) for it to come back as a contribution:
/// which map and version it was taken from.
pub const ZONE_FILE: &str = "zone.json";

/// What [`ZONE_FILE`] holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    /// `massif-zone/1`.
    pub format: String,
    pub map: Id,
    pub map_name: String,
    /// The version the region was exported from: its images, edited in
    /// another tool, are painted onto it again and compared with it.
    pub base: VersionRef,
    /// The dimension, and the blocks the region covers: west, north, east,
    /// south.
    pub dim: usize,
    pub rect: [i32; 4],
    /// When it was exported, in seconds since 1970 (UTC).
    pub created: u64,
}

impl Zone {
    pub const FORMAT: &'static str = "massif-zone/1";

    /// The zone written in folder `dir`, if there is one Massif can read.
    pub fn read(dir: &std::path::Path) -> Option<Zone> {
        let text = std::fs::read(dir.join(ZONE_FILE)).ok()?;
        if text.len() > 1 << 20 {
            return None;
        }
        serde_json::from_slice::<Zone>(&text).ok().filter(|z| z.format == Self::FORMAT)
    }

    /// Write it in folder `dir`.
    pub fn write(&self, dir: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::atomic::write_with(&dir.join(ZONE_FILE), false, |out| out.write_all(&json))
    }
}

/// What a contribution is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A contributor's changes, for the owner to review.
    Contribution,
    /// A zone edited in another tool (World Machine, Gaea…) and brought
    /// back to the map, reviewed as a contribution.
    External,
    /// The owner's newer version, for a contributor to catch up with.
    Update,
}

/// A version, by identifier and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRef {
    pub id: Id,
    pub name: String,
}

/// What a contribution says of itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`FORMAT`].
    pub format: String,
    /// The contribution's own identifier: a depot knows by it which ones
    /// were merged.
    #[serde(default = "Id::random")]
    pub id: Id,
    pub kind: Kind,
    /// The map it belongs to, and its name when it was made.
    pub map: Id,
    pub map_name: String,
    /// The version it starts from.
    pub base: VersionRef,
    /// The contributor's last version, if they made one since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<VersionRef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// When it was made, in seconds since 1970 (UTC).
    pub created: u64,
    /// "Massif 1.1.0".
    pub generator: String,
    /// The map's settings in the base version and in the contribution: the
    /// owner accepts or refuses each one that changed.
    pub settings_base: WorldMeta,
    pub settings: WorldMeta,
    /// For a zone brought back from another tool: the blocks it covers,
    /// west, north, east, south.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<[i32; 4]>,
}

/// A tile of a level, changed.
#[derive(Debug, Clone, PartialEq)]
pub struct TileChange {
    pub pos: TilePos,
    /// The fingerprint of the tile in the base version; `None`: it had none
    /// there.
    pub base: Option<Print>,
    /// The tile now, as a cold blob; `None`: removed.
    pub new: Option<Blob>,
}

/// The changed tiles of a level.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelChange {
    pub id: LevelId,
    pub name: String,
    pub kind: LevelKind,
    /// Sorted by row, then column.
    pub tiles: Vec<TileChange>,
}

/// The sculpted chunks over a tile, changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelChange {
    pub pos: TilePos,
    /// The fingerprint of the chunks over the tile in the base version, if
    /// it had any.
    pub base: Option<Print>,
    /// Their fingerprint now; `None`: none are left. The chunks themselves
    /// are in [`DimChange::voxels`].
    pub new: Option<Print>,
}

/// What changed in a dimension.
#[derive(Debug, Clone)]
pub struct DimChange {
    pub kind: DimensionKind,
    pub min_y: i32,
    pub max_y: i32,
    pub levels: Vec<LevelChange>,
    /// The sculpted chunks over the tiles of [`DimChange::voxel_tiles`],
    /// as they are now.
    pub voxels: VoxelLayer,
    pub voxel_tiles: Vec<VoxelChange>,
}

impl DimChange {
    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(|l| l.tiles.is_empty()) && self.voxel_tiles.is_empty()
    }
}

/// A contribution.
#[derive(Debug, Clone)]
pub struct Patch {
    pub manifest: Manifest,
    /// The registry the numbers in its tiles refer to (terrains, biomes,
    /// covers, object layers, pockets): the contributor's.
    pub registry: Registry,
    pub dims: Vec<DimChange>,
}

/// Why a contribution could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    #[error("the map has no identifier yet: make a version of it first")]
    NoMap,
    #[error("the version “{0}” belongs to another map")]
    OtherMap(String),
    #[error("the world has unsaved hot tiles; snapshot it first")]
    NotFrozen,
}

impl Patch {
    /// The contribution of `now` since version `base`: every tile that
    /// differs, in every level and dimension, and the sculpted chunks over
    /// the tiles where they differ. `now` must be cold (see
    /// [`World::snapshot`]).
    pub fn since(base: &SavePoint, now: &World, kind: Kind, author: &str, message: &str, created: u64) -> Result<Patch, PatchError> {
        let map = now.meta.map_id.ok_or(PatchError::NoMap)?;
        if base.world.meta.map_id.is_some_and(|m| m != map) {
            return Err(PatchError::OtherMap(base.name.clone()));
        }
        let mut dims = Vec::with_capacity(now.dims.len());
        for (d, dim) in now.dims.iter().enumerate() {
            let then = base.world.dims.get(d).filter(|t| t.kind == dim.kind);
            let mut levels = Vec::new();
            for level in &dim.levels {
                let now_blobs = level.blobs().ok_or(PatchError::NotFrozen)?;
                let then_level = then.and_then(|t| t.levels.iter().find(|l| l.id == level.id));
                let then_blobs: HashMap<TilePos, Blob> = then_level.and_then(|l| l.blobs()).unwrap_or_default().into_iter().collect();
                let mut tiles = Vec::new();
                for (pos, blob) in &now_blobs {
                    match then_blobs.get(pos) {
                        Some(old) if same_blob(old, blob) => {}
                        old => tiles.push(TileChange {
                            pos: *pos,
                            base: old.map(|b| Print::of(b)),
                            new: Some(blob.clone()),
                        }),
                    }
                }
                let here: HashSet<TilePos> = now_blobs.iter().map(|(p, _)| *p).collect();
                for (pos, old) in &then_blobs {
                    if !here.contains(pos) {
                        tiles.push(TileChange {
                            pos: *pos,
                            base: Some(Print::of(old)),
                            new: None,
                        });
                    }
                }
                if !tiles.is_empty() {
                    tiles.sort_unstable_by_key(|t| (t.pos.z, t.pos.x));
                    levels.push(LevelChange {
                        id: level.id,
                        name: level.name.clone(),
                        kind: level.kind,
                        tiles,
                    });
                }
            }
            // Levels the contribution removed: every tile of theirs, gone.
            if let Some(then) = then {
                for old in then.levels.iter().filter(|l| !dim.levels.iter().any(|n| n.id == l.id)) {
                    let mut tiles: Vec<TileChange> = old
                        .blobs()
                        .ok_or(PatchError::NotFrozen)?
                        .iter()
                        .map(|(pos, b)| TileChange {
                            pos: *pos,
                            base: Some(Print::of(b)),
                            new: None,
                        })
                        .collect();
                    tiles.sort_unstable_by_key(|t| (t.pos.z, t.pos.x));
                    levels.push(LevelChange {
                        id: old.id,
                        name: old.name.clone(),
                        kind: old.kind,
                        tiles,
                    });
                }
            }
            // The sculpted chunks, tile by tile.
            let mut over: Vec<TilePos> = dim.voxels.iter().map(|(p, _)| p.tile()).collect();
            if let Some(then) = then {
                over.extend(then.voxels.iter().map(|(p, _)| p.tile()));
            }
            over.sort_unstable_by_key(|p| (p.z, p.x));
            over.dedup();
            let mut voxels = VoxelLayer::default();
            let mut voxel_tiles = Vec::new();
            for pos in over {
                let new = fingerprint::of_voxels(&dim.voxels, pos);
                let old = then.and_then(|t| fingerprint::of_voxels(&t.voxels, pos));
                if new != old {
                    for (chunk, _) in dim.voxels.in_tile(pos) {
                        if let Some(shared) = dim.voxels.get_shared(chunk) {
                            voxels.insert(chunk, shared);
                        }
                    }
                    voxel_tiles.push(VoxelChange { pos, base: old, new });
                }
            }
            voxels.take_dirty();
            dims.push(DimChange {
                kind: dim.kind,
                min_y: dim.min_y,
                max_y: dim.max_y,
                levels,
                voxels,
                voxel_tiles,
            });
        }
        let head = now.meta.head.filter(|h| *h != base.id).map(|id| VersionRef {
            id,
            name: now.points.iter().find(|p| p.id == id).map(|p| p.name.clone()).unwrap_or_default(),
        });
        Ok(Patch {
            manifest: Manifest {
                format: FORMAT.into(),
                id: Id::random(),
                kind,
                map,
                map_name: now.meta.name.clone(),
                base: VersionRef {
                    id: base.id,
                    name: base.name.clone(),
                },
                head,
                author: author.into(),
                message: message.into(),
                created,
                generator: format!("Massif {}", env!("CARGO_PKG_VERSION")),
                settings_base: base.world.meta.clone(),
                settings: now.meta.clone(),
                zone: None,
            },
            registry: now.registry.clone(),
            dims,
        })
    }

    /// How many tiles it changes, of every level and dimension.
    pub fn tiles(&self) -> usize {
        self.dims.iter().flat_map(|d| &d.levels).map(|l| l.tiles.len()).sum()
    }

    /// The bytes of its tiles, as stored.
    pub fn tile_bytes(&self) -> u64 {
        self.dims.iter().flat_map(|d| &d.levels).flat_map(|l| &l.tiles).filter_map(|t| t.new.as_ref()).map(|b| b.len() as u64).sum()
    }

    /// Whether it changes nothing (its settings aside).
    pub fn is_empty(&self) -> bool {
        self.dims.iter().all(DimChange::is_empty)
    }

    /// The tiles it touches in dimension `dim`, of any level or sculpted,
    /// sorted.
    pub fn touched(&self, dim: usize) -> Vec<TilePos> {
        let Some(d) = self.dims.get(dim) else { return Vec::new() };
        let mut out: Vec<TilePos> = d.levels.iter().flat_map(|l| l.tiles.iter().map(|t| t.pos)).chain(d.voxel_tiles.iter().map(|v| v.pos)).collect();
        out.sort_unstable_by_key(|p| (p.z, p.x));
        out.dedup();
        out
    }
}

impl Patch {
    /// Leave out the tiles that differ from what they were in `base` by at
    /// most `tolerance` blocks of height and in nothing else: what a 16-bit
    /// heightmap brings back of a tile another tool did not touch.
    pub fn drop_noise(&mut self, base: &World, tolerance: f32) {
        for (d, dim) in self.dims.iter_mut().enumerate() {
            let Some(then) = base.dims.get(d).filter(|b| b.kind == dim.kind) else { continue };
            for level in &mut dim.levels {
                let Some(old) = then.levels.iter().find(|l| l.id == level.id) else { continue };
                let blobs: HashMap<TilePos, Blob> = old.blobs().unwrap_or_default().into_iter().collect();
                level.tiles.retain(|t| match (&t.new, blobs.get(&t.pos)) {
                    (Some(new), Some(old)) => !nearly_same(old, new, tolerance),
                    _ => true,
                });
            }
            dim.levels.retain(|l| !l.tiles.is_empty());
        }
    }
}

/// Whether two tiles hold the same but for heights at most `tolerance`
/// blocks apart.
fn nearly_same(a: &Blob, b: &Blob, tolerance: f32) -> bool {
    use crate::coords::TILE_AREA;
    use crate::plane::Plane;
    use crate::tile::Tile;
    let (Ok(x), Ok(y)) = (Tile::decode(a), Tile::decode(b)) else { return false };
    let close = |p: &Plane<f32>, q: &Plane<f32>| (0..TILE_AREA).all(|i| (p.get(i) - q.get(i)).abs() <= tolerance);
    let same = |p: &Plane<u16>, q: &Plane<u16>| p == q || (0..TILE_AREA).all(|i| p.get(i) == q.get(i));
    same(&x.terrain, &y.terrain)
        && (x.fluid == y.fluid || (0..TILE_AREA).all(|i| x.fluid.get(i) == y.fluid.get(i)))
        && same(&x.biome, &y.biome)
        && same(&x.cover, &y.cover)
        && same(&x.objects, &y.objects)
        && same(&x.annotations, &y.annotations)
        && same(&x.understory, &y.understory)
        && same(&x.sizes, &y.sizes)
        && same(&x.pockets, &y.pockets)
        && x.unknown == y.unknown
        && close(&x.top, &y.top)
        && match (&x.bottom, &y.bottom) {
            (None, None) => true,
            (Some(p), Some(q)) => close(p, q),
            _ => false,
        }
}

/// How the numbers in a contribution's tiles (terrains, biomes, covers,
/// object layers, pockets) read in the map's registry, which gains what it
/// lacks: an entry of the same number and name keeps its number, one of
/// another number but the same name takes the map's, and one the map does
/// not have is added to it (with the blocks, biomes and objects it uses).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remap {
    terrains: Vec<u16>,
    biomes: Vec<u16>,
    /// By number, 0 (none) staying 0.
    covers: Vec<u16>,
    layers: Vec<u16>,
    pockets: Vec<u16>,
    /// Whether every number reads the same in both.
    pub identity: bool,
    /// The names of the entries the map gained.
    pub added: Vec<String>,
}

fn same_block(theirs: &Registry, mine: &mut Registry, seen: &mut HashMap<u16, crate::registry::BlockId>, id: crate::registry::BlockId) -> crate::registry::BlockId {
    *seen.entry(id.0).or_insert_with(|| match theirs.blocks.get(id.index()) {
        Some(b) => mine.intern_block(&b.state, b.color),
        None => crate::registry::BlockId::AIR,
    })
}

impl Remap {
    /// The numbers of `theirs` in `mine`, `mine` gaining what it lacks.
    pub fn into(theirs: &Registry, mine: &mut Registry) -> Remap {
        use crate::registry::BiomeId;
        let mut added = Vec::new();
        let mut blocks = HashMap::new();
        let biomes: Vec<u16> = theirs
            .biomes
            .iter()
            .enumerate()
            .map(|(i, def)| match mine.biomes.get(i) {
                Some(m) if m.id == def.id => i as u16,
                _ => match mine.biome_by_id(&def.id) {
                    Some(b) => b.0,
                    None => {
                        added.push(def.name.clone());
                        mine.intern_biome(def.clone()).0
                    }
                },
            })
            .collect();
        let mut terrains = Vec::with_capacity(theirs.terrains.len());
        for (i, def) in theirs.terrains.iter().enumerate() {
            let n = match mine.terrains.get(i) {
                Some(m) if m.name.eq_ignore_ascii_case(&def.name) => i as u16,
                _ => match mine.terrain_by_name(&def.name) {
                    Some(t) => t.0,
                    None => {
                        let mut def = def.clone();
                        for stratum in &mut def.strata {
                            for (b, _) in &mut stratum.blocks {
                                *b = same_block(theirs, mine, &mut blocks, *b);
                            }
                        }
                        def.submerged_top = def.submerged_top.map(|b| same_block(theirs, mine, &mut blocks, b));
                        def.biome_hint = def.biome_hint.map(|b| BiomeId(biomes.get(b.index()).copied().unwrap_or(0)));
                        added.push(def.name.clone());
                        mine.terrains.push(def);
                        (mine.terrains.len() - 1) as u16
                    }
                },
            };
            terrains.push(n);
        }
        let mut covers = vec![0u16];
        for (k, def) in theirs.covers.iter().enumerate() {
            let n = match mine.covers.get(k) {
                Some(m) if m.name.eq_ignore_ascii_case(&def.name) => k as u16 + 1,
                _ => match mine.cover_by_name(&def.name) {
                    Some(n) => n,
                    None => {
                        added.push(def.name.clone());
                        mine.add_cover(def.clone())
                    }
                },
            };
            covers.push(n);
        }
        let mut objects: HashMap<u32, u32> = HashMap::new();
        let mut layers = vec![0u16];
        for (k, def) in theirs.object_layers.iter().enumerate() {
            let n = match mine.object_layers.get(k) {
                Some(m) if m.name.eq_ignore_ascii_case(&def.name) => k as u16 + 1,
                _ => match mine.object_layers.iter().position(|m| !m.removed && m.name.eq_ignore_ascii_case(&def.name)) {
                    Some(i) => i as u16 + 1,
                    None => {
                        let mut def = def.clone();
                        for (o, _) in &mut def.objects {
                            *o = *objects.entry(*o).or_insert_with(|| match theirs.objects.get(*o as usize) {
                                Some(obj) => match mine.objects.iter().position(|m| m == obj) {
                                    Some(i) => i as u32,
                                    None => {
                                        mine.objects.push(obj.clone());
                                        (mine.objects.len() - 1) as u32
                                    }
                                },
                                None => *o,
                            });
                        }
                        added.push(def.name.clone());
                        mine.add_object_layer(def)
                    }
                },
            };
            layers.push(n);
        }
        let mut pockets = vec![0u16];
        for (k, def) in theirs.pockets.iter().enumerate() {
            let n = match mine.pockets.get(k) {
                Some(m) if m.name.eq_ignore_ascii_case(&def.name) => k as u16 + 1,
                _ => match mine.pockets.iter().position(|m| !m.removed && m.name.eq_ignore_ascii_case(&def.name)) {
                    Some(i) => i as u16 + 1,
                    None => {
                        let mut def = def.clone();
                        for (b, _) in &mut def.blocks {
                            *b = same_block(theirs, mine, &mut blocks, *b);
                        }
                        added.push(def.name.clone());
                        mine.add_pocket(def)
                    }
                },
            };
            pockets.push(n);
        }
        let same = |t: &[u16]| t.iter().enumerate().all(|(i, &n)| n as usize == i);
        let identity = added.is_empty() && same(&terrains) && same(&biomes) && same(&covers) && same(&layers) && same(&pockets);
        Remap {
            terrains,
            biomes,
            covers,
            layers,
            pockets,
            identity,
            added,
        }
    }

    fn read(table: &[u16], v: u16) -> u16 {
        table.get(v as usize).copied().unwrap_or(v)
    }

    /// A tile of theirs with the map's numbers.
    pub fn tile(&self, blob: &Blob) -> Blob {
        use crate::tile::{Tile, map_plane};
        if self.identity {
            return blob.clone();
        }
        let Ok(mut t) = Tile::decode(blob) else { return blob.clone() };
        t.terrain = map_plane(&t.terrain, |v| Self::read(&self.terrains, v));
        t.biome = map_plane(&t.biome, |v| Self::read(&self.biomes, v));
        t.cover = map_plane(&t.cover, |v| Self::read(&self.covers, v));
        t.stacked.renumber(|kind, n| match kind {
            crate::layer_stack::Kind::Cover => Self::read(&self.covers, n),
            crate::layer_stack::Kind::Note => n,
            _ => Self::read(&self.layers, n),
        });
        t.objects = map_plane(&t.objects, |v| Self::read(&self.layers, v));
        t.understory = map_plane(&t.understory, |v| Self::read(&self.layers, v));
        t.pockets = map_plane(&t.pockets, |v| Self::read(&self.pockets, v));
        t.encode().into()
    }

    /// Sculpted chunks of theirs with the map's terrain numbers.
    pub fn voxels(&self, layer: &VoxelLayer) -> VoxelLayer {
        if self.identity {
            return layer.clone();
        }
        let mut out = VoxelLayer::default();
        for (pos, chunk) in layer.iter() {
            let mut chunk = (**chunk).clone();
            chunk.remap_terrains(|t| Self::read(&self.terrains, t));
            out.insert(pos, std::sync::Arc::new(chunk));
        }
        let _ = out.take_dirty();
        out
    }
}

impl Patch {
    /// The contribution with the numbers of `mine` in its tiles and sculpted
    /// chunks (see [`Remap`]), `mine` gaining what it lacks; and how they
    /// were read.
    pub fn remapped(&self, mine: &mut Registry) -> (Patch, Remap) {
        let remap = Remap::into(&self.registry, mine);
        if remap.identity {
            let mut patch = self.clone();
            patch.registry = mine.clone();
            return (patch, remap);
        }
        let dims = self
            .dims
            .iter()
            .map(|d| DimChange {
                kind: d.kind,
                min_y: d.min_y,
                max_y: d.max_y,
                levels: d
                    .levels
                    .iter()
                    .map(|l| LevelChange {
                        id: l.id,
                        name: l.name.clone(),
                        kind: l.kind,
                        tiles: l.tiles.iter().map(|t| TileChange { pos: t.pos, base: t.base, new: t.new.as_ref().map(|b| remap.tile(b)) }).collect(),
                    })
                    .collect(),
                voxels: remap.voxels(&d.voxels),
                voxel_tiles: d.voxel_tiles.clone(),
            })
            .collect();
        (
            Patch {
                manifest: self.manifest.clone(),
                registry: mine.clone(),
                dims,
            },
            remap,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_contribution_holds_what_changed_since_its_version() {
        let mut world = flat(4);
        let base = SavePoint::new(&mut world, "base", 1);
        world.points.push(base.clone());
        // Two tiles changed, one added, one removed.
        world.dims[0].ground_mut().insert(TilePos::new(1, 1), Tile::ground(80.0, 1, -1));
        world.dims[0].ground_mut().insert(TilePos::new(2, 1), Tile::ground(70.0, 2, -1));
        world.dims[0].ground_mut().insert(TilePos::new(9, 9), Tile::ground(64.0, 1, -1));
        assert!(world.dims[0].ground_mut().remove(TilePos::new(3, 3)));
        world.meta.sea_level = 70;
        let now = world.snapshot();
        let patch = Patch::since(&base, &now, Kind::Contribution, "Jack", "Rivers", 2).unwrap();
        assert_eq!(patch.manifest.map, world.meta.map_id.unwrap());
        assert_eq!(patch.manifest.base, VersionRef { id: base.id, name: "base".into() });
        assert_eq!(patch.manifest.settings_base.sea_level, 63);
        assert_eq!(patch.manifest.settings.sea_level, 70);
        let tiles = &patch.dims[0].levels[0].tiles;
        let at = |x, z| tiles.iter().find(|t| t.pos == TilePos::new(x, z)).unwrap();
        assert_eq!(tiles.len(), 4);
        let flat = Print::of(&Tile::ground(64.0, 1, -1).encode());
        assert_eq!(at(1, 1).base, Some(flat));
        assert!(at(1, 1).new.is_some());
        assert_eq!(at(9, 9).base, None);
        assert_eq!((at(3, 3).base, at(3, 3).new.is_none()), (Some(flat), true));
        assert_eq!(patch.tiles(), 4);
        assert_eq!(patch.touched(0).len(), 4);
        assert!(!patch.is_empty());
        // Nothing changed since: an empty contribution.
        let point = SavePoint::new(&mut world, "now", 3);
        let empty = Patch::since(&point, &world.snapshot(), Kind::Contribution, "", "", 4).unwrap();
        assert!(empty.is_empty());
    }

    /// A contribution whose registry gained a terrain and a cover the map
    /// does not have, the map one of its own at the same numbers: theirs
    /// are added to the map and the tiles read with the map's numbers.
    #[test]
    fn a_contribution_reads_with_the_maps_numbers() {
        use crate::registry::TerrainDef;
        let mut world = flat(2);
        let base = SavePoint::new(&mut world, "base", 1);
        let mut theirs = world.snapshot();
        let mut mine = world.registry.clone();
        let n = theirs.registry.terrains.len() as u16;
        let def = |name: &str| TerrainDef { name: name.into(), ..theirs.registry.terrains[1].clone() };
        mine.terrains.push(def("Owner's sand"));
        theirs.registry.terrains.push(def("Jack's moss"));
        let cover = theirs.registry.add_cover(crate::registry::CoverDef::new("Moss", [60, 120, 60], &[("minecraft:moss_carpet", 1)], 0.5));
        let mut tile = Tile::ground(70.0, n, -1);
        tile.cover = crate::plane::Plane::Uniform(cover);
        theirs.dims[0].ground_mut().insert(TilePos::new(0, 0), tile);
        let patch = Patch::since(&base, &theirs.snapshot(), Kind::Contribution, "", "", 2).unwrap();
        let (read, remap) = patch.remapped(&mut mine);
        assert!(!remap.identity);
        assert_eq!(remap.added, ["Jack's moss", "Moss"]);
        assert_eq!(mine.terrains[n as usize].name, "Owner's sand");
        let moss = mine.terrain_by_name("Jack's moss").unwrap();
        assert_eq!(moss.0, n + 1);
        let t = Tile::decode(read.dims[0].levels[0].tiles[0].new.as_ref().unwrap()).unwrap();
        assert_eq!(t.terrain.uniform_value(), Some(moss.0));
        assert_eq!(t.cover.uniform_value(), mine.cover_by_name("Moss"));
        assert_eq!(read.registry, mine);
        // Read again into the map that has them now: the same numbers.
        let (_, again) = patch.remapped(&mut mine.clone());
        assert!(again.added.is_empty());
    }

    /// Tiles a heightmap brought back a hair off are not changes.
    #[test]
    fn heights_a_hair_off_are_not_changes() {
        let mut world = flat(3);
        let base = SavePoint::new(&mut world, "base", 1);
        let mut theirs = world.snapshot();
        theirs.dims[0].ground_mut().insert(TilePos::new(0, 0), Tile::ground(64.004, 1, -1));
        theirs.dims[0].ground_mut().insert(TilePos::new(1, 0), Tile::ground(66.0, 1, -1));
        let mut patch = Patch::since(&base, &theirs.snapshot(), Kind::External, "", "", 2).unwrap();
        assert_eq!(patch.tiles(), 2);
        patch.drop_noise(&base.world, 0.05);
        assert_eq!(patch.tiles(), 1);
        assert_eq!(patch.dims[0].levels[0].tiles[0].pos, TilePos::new(1, 0));
    }

    #[test]
    fn a_version_of_another_map_is_refused() {
        let mut a = flat(1);
        let mut b = flat(1);
        let base = SavePoint::new(&mut a, "a", 1);
        SavePoint::new(&mut b, "b", 1);
        assert_eq!(Patch::since(&base, &b.snapshot(), Kind::Contribution, "", "", 2).unwrap_err(), PatchError::OtherMap("a".into()));
        let mut c = flat(1);
        assert_eq!(Patch::since(&base, &c.snapshot(), Kind::Contribution, "", "", 2).unwrap_err(), PatchError::NoMap);
    }
}
