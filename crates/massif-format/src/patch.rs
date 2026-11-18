//! The `.massifpatch` file: a contribution (see [`massif_core::patch`]) in
//! the container of the `.massif` file, with bit 0 of the header's flags
//! set.
//!
//! ```text
//! header        64 bytes, as a map's; flags = 1
//! tile frames   the changed tiles, each distinct one once
//! PTCH          zstd(JSON): the manifest, and per dimension its changed
//!               levels (each tile: position, base fingerprint, frame) and
//!               the tiles whose sculpted chunks changed
//! REGY          zstd(JSON): the registry the tiles' numbers refer to
//! VOXL          a dimension's sculpted chunks over those tiles, as a map's
//! directory     as a map's
//! ```
//!
//! Reading checks everything a hostile file could get wrong: the
//! checksums, where tiles and sections lie, how many there are, their
//! coordinates, that each tile decodes and that the sculpted chunks match
//! their fingerprints. Nothing in it is ever run.

use super::*;
use massif_core::fingerprint::{self, Print};
use massif_core::patch::{DimChange, LevelChange, Manifest, Patch, TileChange, VoxelChange};
use massif_core::voxel::VoxelLayer;

const KIND_PATCH: [u8; 4] = *b"PTCH";
const KIND_REGISTRY: [u8; 4] = *b"REGY";

/// The most tiles a contribution may change: a map of 16k × 16k blocks has
/// 16,384 tiles a level.
pub const MAX_TILES: usize = 1 << 20;
/// The farthest a tile may lie from the world's middle, in tiles: past
/// Minecraft's world border (30 million blocks).
pub const MAX_TILE_COORD: i32 = 1 << 18;

#[derive(Debug, Serialize, Deserialize)]
struct PatchDoc {
    manifest: Manifest,
    dimensions: Vec<PatchDimDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PatchDimDoc {
    kind: DimensionKind,
    min_y: i32,
    max_y: i32,
    levels: Vec<PatchLevelDoc>,
    /// Id of the `VOXL` section holding the sculpted chunks, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voxels: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    voxel_tiles: Vec<VoxelDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PatchLevelDoc {
    id: u32,
    name: String,
    kind: LevelKind,
    tiles: Vec<TileDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TileDoc {
    x: i32,
    z: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<Print>,
    /// Where its frame lies; `None`: the tile is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<u64>,
    #[serde(default)]
    len: u32,
    #[serde(default)]
    crc: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct VoxelDoc {
    x: i32,
    z: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<Print>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    new: Option<Print>,
}

/// Serialize a contribution. Returns the file's length.
pub fn write_patch<W: Write>(patch: &Patch, mut out: W) -> Result<u64> {
    let mut offset = HEADER_LEN as u64;
    let mut frames: Vec<Blob> = Vec::new();
    // Each distinct tile once (a contribution of open sea holds many alike).
    let mut written: HashMap<(u32, u32), Vec<(u64, usize)>> = HashMap::new();
    let mut sections = Vec::new();
    let mut next_section = 2;
    let mut dimensions = Vec::with_capacity(patch.dims.len());
    for dim in &patch.dims {
        let mut levels = Vec::with_capacity(dim.levels.len());
        for level in &dim.levels {
            let mut tiles = Vec::with_capacity(level.tiles.len());
            for t in &level.tiles {
                let (at, len, crc) = match &t.new {
                    None => (None, 0, 0),
                    Some(blob) => {
                        let crc = crc32c::crc32c(blob);
                        let len = u32::try_from(blob.len()).map_err(|_| FormatError::Corrupt("tile frame over 4 GiB".into()))?;
                        let same = written.get(&(crc, len)).and_then(|found| found.iter().find(|&&(_, k)| frames[k][..] == blob[..])).map(|&(at, _)| at);
                        let at = match same {
                            Some(at) => at,
                            None => {
                                let at = offset;
                                offset += len as u64;
                                written.entry((crc, len)).or_default().push((at, frames.len()));
                                frames.push(blob.clone());
                                at
                            }
                        };
                        (Some(at), len, crc)
                    }
                };
                tiles.push(TileDoc {
                    x: t.pos.x,
                    z: t.pos.z,
                    base: t.base,
                    at,
                    len,
                    crc,
                });
            }
            levels.push(PatchLevelDoc {
                id: level.id.0,
                name: level.name.clone(),
                kind: level.kind,
                tiles,
            });
        }
        let voxels = (!dim.voxels.is_empty()).then(|| {
            let id = next_section;
            next_section += 1;
            sections.push(Section {
                kind: KIND_VOXELS,
                id,
                flags: 0,
                payload: dim.voxels.encode(),
            });
            id
        });
        dimensions.push(PatchDimDoc {
            kind: dim.kind,
            min_y: dim.min_y,
            max_y: dim.max_y,
            levels,
            voxels,
            voxel_tiles: dim
                .voxel_tiles
                .iter()
                .map(|v| VoxelDoc {
                    x: v.pos.x,
                    z: v.pos.z,
                    base: v.base,
                    new: v.new,
                })
                .collect(),
        });
    }
    let doc = PatchDoc {
        manifest: patch.manifest.clone(),
        dimensions,
    };
    sections.insert(
        0,
        Section {
            kind: KIND_PATCH,
            id: 0,
            flags: FLAG_ZSTD,
            payload: codec::zstd_compress(&serde_json::to_vec(&doc)?),
        },
    );
    sections.insert(
        1,
        Section {
            kind: KIND_REGISTRY,
            id: 1,
            flags: FLAG_ZSTD,
            payload: codec::zstd_compress(&serde_json::to_vec(&patch.registry)?),
        },
    );
    finish(&mut out, &frames, &sections, FLAG_PATCH)
}

/// Write a contribution to `path` atomically (see [`save`]). Returns the
/// file's length.
pub fn save_patch(patch: &Patch, path: &Path) -> Result<u64> {
    let mut len = None;
    let written = massif_core::atomic::write_with(path, true, |out| match write_patch(patch, out) {
        Ok(n) => {
            len = Some(n);
            Ok(())
        }
        Err(FormatError::Io(e)) => Err(e),
        Err(other) => Err(io::Error::other(other)),
    });
    match (written, len) {
        (Ok(()), Some(n)) => Ok(n),
        (Err(e), _) => Err(match e.get_ref().is_some_and(|inner| inner.is::<FormatError>()) {
            true => *e.into_inner().expect("checked").downcast::<FormatError>().expect("checked"),
            false => FormatError::Io(e),
        }),
        (Ok(()), None) => corrupt("nothing was written"),
    }
}

/// Read a contribution from the bytes of a `.massifpatch` file.
pub fn read_patch(bytes: &[u8]) -> Result<Patch> {
    read_patch_from(&bytes)
}

/// Read a contribution from a `.massifpatch` file.
pub fn load_patch(path: &Path) -> Result<Patch> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    read_patch_from(&FileSource { file, len })
}

/// The manifest of the `.massifpatch` file at `path`, read without its
/// tiles: what a list of contributions shows.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let src = FileSource { file, len };
    let Container { flags, directory, .. } = container(&src)?;
    if flags & FLAG_PATCH == 0 {
        return Err(FormatError::NotPatch);
    }
    for entry in directory.as_chunks::<SECTION_LEN>().0 {
        if entry[..4] == KIND_PATCH {
            let json = section_payload(&src, entry, "the contribution's contents")?;
            return Ok(serde_json::from_slice::<PatchDoc>(&json)?.manifest);
        }
    }
    corrupt("the contribution's contents are missing")
}

fn read_patch_from(src: &dyn Source) -> Result<Patch> {
    let Container { flags, directory, .. } = container(src)?;
    if flags & FLAG_PATCH == 0 {
        return Err(FormatError::NotPatch);
    }
    let mut doc = None;
    let mut registry = None;
    let mut voxels: Vec<(u32, Vec<u8>)> = Vec::new();
    for entry in directory.as_chunks::<SECTION_LEN>().0 {
        let kind: [u8; 4] = entry[..4].try_into().unwrap();
        match kind {
            KIND_PATCH => {
                let json = section_payload(src, entry, "the contribution's contents")?;
                doc = Some(serde_json::from_slice::<PatchDoc>(&json)?);
            }
            KIND_REGISTRY => {
                let json = section_payload(src, entry, "the contribution's registry")?;
                registry = Some(serde_json::from_slice::<Registry>(&json)?);
            }
            KIND_VOXELS => voxels.push((u32_at(entry, 4), section_payload(src, entry, "the sculpted chunks")?)),
            _ => {} // a section from a newer version: ignore it
        }
    }
    let (Some(doc), Some(registry)) = (doc, registry) else {
        return corrupt("the contribution's contents are missing");
    };
    let manifest = doc.manifest;
    match manifest.format.strip_prefix("massifpatch/").and_then(|v| v.parse::<u32>().ok()) {
        Some(1) => {}
        Some(_) => return corrupt("the contribution was made by a newer version of Massif"),
        None => return corrupt("the contribution's format is unknown"),
    }
    if doc.dimensions.len() > 3 {
        return corrupt("the contribution has too many dimensions");
    }
    let total: usize = doc.dimensions.iter().flat_map(|d| &d.levels).map(|l| l.tiles.len()).sum();
    let total_voxels: usize = doc.dimensions.iter().map(|d| d.voxel_tiles.len()).sum();
    if total > MAX_TILES || total_voxels > MAX_TILES {
        return corrupt("the contribution changes too many tiles");
    }
    let place = |x: i32, z: i32| -> Result<TilePos> {
        if x.unsigned_abs() > MAX_TILE_COORD as u32 || z.unsigned_abs() > MAX_TILE_COORD as u32 {
            return corrupt(format!("a tile of the contribution lies too far away ({x}, {z})"));
        }
        Ok(TilePos::new(x, z))
    };
    let mut kinds = Vec::new();
    let mut dims = Vec::with_capacity(doc.dimensions.len());
    for d in doc.dimensions {
        if kinds.contains(&d.kind) {
            return corrupt("the contribution has a dimension twice");
        }
        kinds.push(d.kind);
        if d.min_y >= d.max_y || d.min_y < -(1 << 16) || d.max_y > 1 << 16 {
            return corrupt("a dimension of the contribution has no sensible height");
        }
        let mut levels = Vec::with_capacity(d.levels.len());
        let mut ids = Vec::new();
        for l in d.levels {
            if ids.contains(&l.id) {
                return corrupt("the contribution has a level twice");
            }
            ids.push(l.id);
            if (l.id == 0) != (l.kind == LevelKind::Ground) {
                return corrupt("the contribution's ground level is not the ground");
            }
            let mut seen = std::collections::HashSet::with_capacity(l.tiles.len());
            let mut docs = Vec::with_capacity(l.tiles.len());
            for t in l.tiles {
                let pos = place(t.x, t.z)?;
                if !seen.insert(pos) {
                    return corrupt(format!("tile ({}, {}) of level \"{}\" is in the contribution twice", t.x, t.z, l.name));
                }
                docs.push((pos, t));
            }
            let tiles: Vec<Result<TileChange>> = docs
                .into_par_iter()
                .map(|(pos, t)| {
                    let new = match t.at {
                        None => None,
                        Some(at) => {
                            if at < HEADER_LEN as u64 {
                                return corrupt("a tile lies in the header");
                            }
                            let blob = src.blob(at, t.len as u64, "a tile")?;
                            if crc32c::crc32c(&blob) != t.crc {
                                return corrupt(format!("tile ({}, {}) of level \"{}\" fails its checksum", pos.x, pos.z, l.name));
                            }
                            // A tile the editor could not open is refused
                            // here, not when the owner looks at it.
                            if Tile::decode(&blob).is_err() {
                                return corrupt(format!("tile ({}, {}) of level \"{}\" cannot be read", pos.x, pos.z, l.name));
                            }
                            Some(blob)
                        }
                    };
                    Ok(TileChange { pos, base: t.base, new })
                })
                .collect();
            levels.push(LevelChange {
                id: LevelId(l.id),
                name: l.name,
                kind: l.kind,
                tiles: tiles.into_iter().collect::<Result<_>>()?,
            });
        }
        let layer = match d.voxels {
            None => VoxelLayer::default(),
            Some(id) => {
                let Some((_, raw)) = voxels.iter().find(|(i, _)| *i == id) else {
                    return corrupt(format!("the sculpted chunks {id} are missing"));
                };
                VoxelLayer::decode(raw).map_err(FormatError::Corrupt)?
            }
        };
        let mut voxel_tiles = Vec::with_capacity(d.voxel_tiles.len());
        let mut seen = std::collections::HashSet::new();
        for v in d.voxel_tiles {
            let pos = place(v.x, v.z)?;
            if !seen.insert(pos) {
                return corrupt("the sculpted chunks of a tile are in the contribution twice");
            }
            if fingerprint::of_voxels(&layer, pos) != v.new {
                return corrupt(format!("the sculpted chunks over tile ({}, {}) do not match their fingerprint", pos.x, pos.z));
            }
            voxel_tiles.push(VoxelChange { pos, base: v.base, new: v.new });
        }
        if layer.iter().any(|(p, _)| !seen.contains(&p.tile())) {
            return corrupt("the contribution has sculpted chunks over tiles it does not list");
        }
        dims.push(DimChange {
            kind: d.kind,
            min_y: d.min_y,
            max_y: d.max_y,
            levels,
            voxels: layer,
            voxel_tiles,
        });
    }
    Ok(Patch { manifest, registry, dims })
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, generate};
    use massif_core::patch::Kind;
    use massif_core::voxel::{ChunkPos, VoxelChunk};

    /// A generated map, a version of it, then changes: a hill raised,
    /// tiles painted, one removed, a level added and a chunk sculpted.
    fn contribution() -> (World, Patch) {
        let mut world = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            ..GenParams::default()
        });
        let base = SavePoint::new(&mut world, "Before the rivers", 1_700_000_000);
        world.points.push(base.clone());
        let mut tiles: Vec<TilePos> = world.dims[0].ground().positions().collect();
        tiles.sort_unstable_by_key(|p| (p.z, p.x));
        let (a, b, far) = (tiles[0], tiles[5], tiles[tiles.len() - 1]);
        let mut tile = world.dims[0].ground_mut().tile_mut(a).unwrap().clone();
        tile.top = massif_core::plane::Plane::Uniform(99.0);
        world.dims[0].ground_mut().insert(a, tile.clone());
        world.dims[0].ground_mut().insert(b, tile);
        world.dims[0].ground_mut().remove(far);
        let island = world.dims[0].add_level("Island", LevelKind::Solid);
        world.dims[0].levels[island].insert(TilePos::new(0, 0), Tile::ground(90.0, 1, -1));
        let mut chunk = VoxelChunk::air();
        chunk.set(3, 4, 5, 255);
        world.dims[0].voxels.insert(ChunkPos::new(2, 4, 3), std::sync::Arc::new(chunk));
        world.meta.sea_level = 60;
        let now = world.snapshot();
        let patch = Patch::since(&base, &now, Kind::Contribution, "Jack (Fornad)", "Rivers of the northern valley", 1_700_000_100).unwrap();
        (world, patch)
    }

    fn same(a: &Patch, b: &Patch) {
        assert_eq!(a.manifest, b.manifest);
        assert_eq!(a.registry, b.registry);
        assert_eq!(a.dims.len(), b.dims.len());
        for (x, y) in a.dims.iter().zip(&b.dims) {
            assert_eq!((x.kind, x.min_y, x.max_y), (y.kind, y.min_y, y.max_y));
            assert_eq!(x.levels, y.levels);
            assert_eq!(x.voxel_tiles, y.voxel_tiles);
            assert_eq!(x.voxels.encode(), y.voxels.encode());
        }
    }

    #[test]
    fn a_contribution_round_trips() {
        let (_, patch) = contribution();
        assert_eq!(patch.dims[0].levels.len(), 2, "the ground and the new level");
        assert_eq!(patch.dims[0].voxel_tiles.len(), 1);
        let mut bytes = Vec::new();
        let len = write_patch(&patch, &mut bytes).unwrap();
        assert_eq!(len, bytes.len() as u64);
        let back = read_patch(&bytes).unwrap();
        same(&patch, &back);
        // The same twice: written alike.
        let mut again = Vec::new();
        write_patch(&back, &mut again).unwrap();
        assert_eq!(again, bytes);
        // Each distinct tile once: the two raised tiles are one frame.
        let raised: Vec<_> = patch.dims[0].levels[0].tiles.iter().filter(|t| t.new.as_ref().is_some_and(|b| Tile::decode(b).unwrap().top.uniform_value() == Some(99.0))).collect();
        assert_eq!(raised.len(), 2);
        assert_eq!(raised[0].new, raised[1].new);
        let frame = raised[0].new.as_ref().unwrap().len();
        assert!(bytes.len() + frame < patch.tile_bytes() as usize + 20_000, "{} bytes for {} of tiles", bytes.len(), patch.tile_bytes());
        // Saved and loaded from a file.
        let dir = std::env::temp_dir().join(format!("massif-patch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("riverlands.massifpatch");
        save_patch(&patch, &path).unwrap();
        same(&patch, &load_patch(&path).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A map and a contribution are told apart, both ways.
    #[test]
    fn a_map_is_not_a_contribution() {
        let (mut world, patch) = contribution();
        let mut bytes = Vec::new();
        write_patch(&patch, &mut bytes).unwrap();
        assert!(matches!(read(&bytes), Err(FormatError::IsPatch)));
        assert!(matches!(verify(&bytes), Err(FormatError::IsPatch)));
        let map = crate::tests::to_bytes(&mut world);
        assert!(matches!(read_patch(&map), Err(FormatError::NotPatch)));
        assert!(matches!(read_patch(b"hello"), Err(FormatError::NotMassif)));
    }

    #[test]
    fn damage_to_a_contribution_is_detected() {
        let (_, patch) = contribution();
        let mut bytes = Vec::new();
        write_patch(&patch, &mut bytes).unwrap();
        assert!(read_patch(&bytes[..bytes.len() - 1]).is_err());
        for at in (0..bytes.len()).step_by(97).chain([9, 0x30, HEADER_LEN + 5, bytes.len() - 3]) {
            let mut bad = bytes.clone();
            bad[at] ^= 0x10;
            assert!(read_patch(&bad).is_err(), "flip at {at} went unnoticed");
        }
    }

    /// The contribution's contents, rewritten by `edit` (checksums made
    /// right again), as a hostile file would hold them.
    fn forged(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let src = bytes;
        let Container { directory, .. } = container(&src).unwrap();
        let entry = directory.as_chunks::<SECTION_LEN>().0.iter().find(|e| e[..4] == KIND_PATCH).unwrap();
        let json = section_payload(&src, entry, "contents").unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        edit(&mut value);
        crate::tests::with_section(bytes, KIND_PATCH, FLAG_ZSTD, &codec::zstd_compress(&serde_json::to_vec(&value).unwrap()))
    }

    #[test]
    fn a_hostile_contribution_is_refused() {
        let (_, patch) = contribution();
        let mut bytes = Vec::new();
        write_patch(&patch, &mut bytes).unwrap();
        // Untouched, the forger's copy reads.
        assert!(read_patch(&forged(&bytes, |_| {})).is_ok());
        let tile = |v: &mut serde_json::Value| -> serde_json::Value { v["dimensions"][0]["levels"][0]["tiles"][0].clone() };
        type Edit = Box<dyn Fn(&mut serde_json::Value)>;
        let cases: Vec<(&str, Edit)> = vec![
            ("far away", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["x"] = (1i64 << 30).into())),
            ("twice", Box::new(move |v| {
                let t = tile(v);
                v["dimensions"][0]["levels"][0]["tiles"].as_array_mut().unwrap().push(t);
            })),
            ("outside the file", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["at"] = (1u64 << 40).into())),
            ("in the header", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["at"] = 8.into())),
            ("too long", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["len"] = u32::MAX.into())),
            ("a wrong checksum", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["crc"] = 7.into())),
            ("a bad fingerprint", Box::new(|v| v["dimensions"][0]["levels"][0]["tiles"][0]["base"] = "zz".into())),
            ("a ground that is not", Box::new(|v| v["dimensions"][0]["levels"][0]["id"] = 5.into())),
            ("a level twice", Box::new(|v| {
                let l = v["dimensions"][0]["levels"][0].clone();
                v["dimensions"][0]["levels"].as_array_mut().unwrap().push(l);
            })),
            ("a dimension twice", Box::new(|v| {
                let d = v["dimensions"][0].clone();
                v["dimensions"].as_array_mut().unwrap().push(d);
            })),
            ("no height", Box::new(|v| v["dimensions"][0]["max_y"] = v["dimensions"][0]["min_y"].clone())),
            ("chunks that do not match", Box::new(|v| v["dimensions"][0]["voxel_tiles"][0]["new"] = "00000000000000000000000000000000".into())),
            ("chunks not listed", Box::new(|v| v["dimensions"][0]["voxel_tiles"] = serde_json::json!([]))),
            ("a newer format", Box::new(|v| v["manifest"]["format"] = "massifpatch/2".into())),
            ("another format", Box::new(|v| v["manifest"]["format"] = "zip".into())),
        ];
        for (what, edit) in cases {
            assert!(read_patch(&forged(&bytes, edit)).is_err(), "a contribution with a tile {what} was read");
        }
        // A tile that is not a tile, with a right checksum: its bytes in a
        // section of their own, the first tile pointed at them.
        let junk = codec::zstd_compress(b"not a tile at all");
        let at = u64_at(&bytes, 0x10);
        let with_junk = crate::tests::with_section(&bytes, *b"JUNK", 0, &junk);
        let crc = crc32c::crc32c(&junk);
        let bad = forged(&with_junk, |v| {
            v["dimensions"][0]["levels"][0]["tiles"][0]["at"] = at.into();
            v["dimensions"][0]["levels"][0]["tiles"][0]["len"] = (junk.len() as u64).into();
            v["dimensions"][0]["levels"][0]["tiles"][0]["crc"] = crc.into();
        });
        assert!(matches!(read_patch(&bad), Err(FormatError::Corrupt(e)) if e.contains("cannot be read")));
    }
}
