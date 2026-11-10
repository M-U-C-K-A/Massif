//! The `.massif` world container.
//!
//! One file, little-endian, built for random access: a fixed header points to
//! a directory of sections at the end of the file; every tile is its own
//! zstd frame with its own checksum, so tiles can be read, verified and
//! decoded independently and in parallel.
//!
//! ```text
//! ┌──────────────────────────── header, 64 bytes ───────────────────────────┐
//! │ 0x00 [u8; 8] magic  "MASSIF\x1a\n"                                      │
//! │ 0x08 u16     format version                                             │
//! │ 0x0A u16     oldest reader version able to read this file               │
//! │ 0x0C u32     flags (0)                                                  │
//! │ 0x10 u64     directory offset                                           │
//! │ 0x18 u32     directory length in bytes                                  │
//! │ 0x1C u32     directory CRC32C                                           │
//! │ 0x20 u64     file length (detects truncation)                           │
//! │ 0x28 [u8;20] reserved, zero                                             │
//! │ 0x3C u32     CRC32C of bytes 0x00..0x3C                                 │
//! ├──────────────────────────── tile frames ────────────────────────────────┤
//! │ one zstd frame per tile, back to back (see `massif_core::tile`)         │
//! ├──────────────────────────── sections ───────────────────────────────────┤
//! │ META  zstd(JSON): world settings, registry, dimensions, levels          │
//! │ TIDX  zstd(tile index of one level), 24 bytes per tile:                 │
//! │         i32 tile_x · i32 tile_z · u64 offset · u32 length · u32 CRC32C  │
//! │ VOXL  zstd(sculpted 3D chunks of one dimension), per chunk:             │
//! │         i32 x · i32 y · i32 z (in chunks) · 4096 densities (y, z, x)    │
//! ├──────────────────────────── directory ──────────────────────────────────┤
//! │ 32 bytes per section:                                                   │
//! │   [u8; 4] kind · u32 id · u64 offset · u64 length · u32 CRC32C · u32    │
//! │   flags (bit 0: zstd)                                                   │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Readers skip section kinds they do not know, and tiles skip planes they do
//! not know, so the format can grow without breaking old files or old
//! readers. The metadata is JSON on purpose: it is a few kilobytes, and being
//! able to read it with any tool is worth more than saving them.
//!
//! A tile frame is exactly the "cold" blob the editor keeps in memory, so
//! saving copies bytes and opening a file decodes nothing.

use massif_core::codec::{self, CodecError};
use massif_core::level::{Blob, Level, LevelId, LevelKind};
use massif_core::registry::Registry;
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};
use massif_core::{Tile, TilePos};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::Path;

/// File extension, without the dot.
pub const EXTENSION: &str = "massif";
pub const MAGIC: [u8; 8] = *b"MASSIF\x1a\n";
/// Version written by this crate.
pub const VERSION: u16 = 1;

const HEADER_LEN: usize = 64;
const SECTION_LEN: usize = 32;
const TILE_ENTRY_LEN: usize = 24;
const KIND_META: [u8; 4] = *b"META";
const KIND_TILE_INDEX: [u8; 4] = *b"TIDX";
const KIND_VOXELS: [u8; 4] = *b"VOXL";
const FLAG_ZSTD: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not a Massif world file")]
    NotMassif,
    #[error("the file needs a newer version of Massif (format {found}, this build reads up to {supported})")]
    TooNew { found: u16, supported: u16 },
    #[error("the file is damaged: {0}")]
    Corrupt(String),
    #[error("the file's metadata cannot be read: {0}")]
    Meta(#[from] serde_json::Error),
    #[error("the world has unsaved hot tiles; snapshot or freeze it before writing")]
    NotFrozen,
    #[error("{0}")]
    Codec(#[from] CodecError),
}

pub type Result<T> = std::result::Result<T, FormatError>;

fn corrupt<T>(what: impl Into<String>) -> Result<T> {
    Err(FormatError::Corrupt(what.into()))
}

// ------------------------------------------------------------- metadata --

#[derive(Debug, Serialize, Deserialize)]
struct MetaDoc {
    format: String,
    generator: String,
    world: WorldMeta,
    registry: Registry,
    dimensions: Vec<DimDoc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DimDoc {
    kind: DimensionKind,
    min_y: i32,
    max_y: i32,
    levels: Vec<LevelDoc>,
    /// Id of the `VOXL` section holding the sculpted chunks, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voxels: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LevelDoc {
    id: u32,
    name: String,
    kind: LevelKind,
    #[serde(default = "yes")]
    visible: bool,
    /// Id of the `TIDX` section holding this level's tile index.
    index: u32,
}

fn yes() -> bool {
    true
}

// --------------------------------------------------------------- writing --

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub file_len: u64,
    pub tiles: usize,
    /// Bytes taken by the tile frames.
    pub tile_bytes: u64,
}

struct Section {
    kind: [u8; 4],
    id: u32,
    flags: u32,
    payload: Vec<u8>,
}

/// Serialize a world. Every level must be frozen (see
/// [`massif_core::World::snapshot`]); nothing is encoded here.
pub fn write<W: Write>(world: &World, mut out: W) -> Result<Stats> {
    // Lay the tile frames out right after the header.
    let mut offset = HEADER_LEN as u64;
    let mut frames: Vec<Blob> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut dims = Vec::new();
    let mut next_section = 1u32;

    for dim in &world.dims {
        let mut levels = Vec::new();
        for level in &dim.levels {
            let blobs = level.blobs().ok_or(FormatError::NotFrozen)?;
            let crcs: Vec<u32> = blobs.par_iter().map(|(_, b)| crc32c::crc32c(b)).collect();
            let mut index = Vec::with_capacity(blobs.len() * TILE_ENTRY_LEN);
            for ((pos, blob), crc) in blobs.into_iter().zip(crcs) {
                let len = u32::try_from(blob.len())
                    .map_err(|_| FormatError::Corrupt("tile frame over 4 GiB".into()))?;
                index.extend_from_slice(&pos.x.to_le_bytes());
                index.extend_from_slice(&pos.z.to_le_bytes());
                index.extend_from_slice(&offset.to_le_bytes());
                index.extend_from_slice(&len.to_le_bytes());
                index.extend_from_slice(&crc.to_le_bytes());
                offset += len as u64;
                frames.push(blob);
            }
            let id = next_section;
            next_section += 1;
            sections.push(Section {
                kind: KIND_TILE_INDEX,
                id,
                flags: FLAG_ZSTD,
                payload: codec::zstd_compress(&index),
            });
            levels.push(LevelDoc {
                id: level.id.0,
                name: level.name.clone(),
                kind: level.kind,
                visible: level.visible,
                index: id,
            });
        }
        let voxels = (!dim.voxels.is_empty()).then(|| {
            let id = next_section;
            next_section += 1;
            // The layer is zstd'd already: stored as is.
            sections.push(Section {
                kind: KIND_VOXELS,
                id,
                flags: 0,
                payload: dim.voxels.encode(),
            });
            id
        });
        dims.push(DimDoc {
            kind: dim.kind,
            min_y: dim.min_y,
            max_y: dim.max_y,
            levels,
            voxels,
        });
    }
    let tile_bytes = offset - HEADER_LEN as u64;

    let meta = MetaDoc {
        format: "massif".into(),
        generator: format!("Massif {}", env!("CARGO_PKG_VERSION")),
        world: world.meta.clone(),
        registry: world.registry.clone(),
        dimensions: dims,
    };
    sections.insert(
        0,
        Section {
            kind: KIND_META,
            id: 0,
            flags: FLAG_ZSTD,
            payload: codec::zstd_compress(&serde_json::to_vec(&meta)?),
        },
    );

    // Sections follow the frames, the directory follows the sections.
    let mut directory = Vec::with_capacity(sections.len() * SECTION_LEN);
    for s in &sections {
        directory.extend_from_slice(&s.kind);
        directory.extend_from_slice(&s.id.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(&(s.payload.len() as u64).to_le_bytes());
        directory.extend_from_slice(&crc32c::crc32c(&s.payload).to_le_bytes());
        directory.extend_from_slice(&s.flags.to_le_bytes());
        offset += s.payload.len() as u64;
    }
    let directory_offset = offset;
    let file_len = directory_offset + directory.len() as u64;

    let mut header = [0u8; HEADER_LEN];
    header[0x00..0x08].copy_from_slice(&MAGIC);
    header[0x08..0x0A].copy_from_slice(&VERSION.to_le_bytes());
    header[0x0A..0x0C].copy_from_slice(&VERSION.to_le_bytes());
    header[0x10..0x18].copy_from_slice(&directory_offset.to_le_bytes());
    header[0x18..0x1C].copy_from_slice(&(directory.len() as u32).to_le_bytes());
    header[0x1C..0x20].copy_from_slice(&crc32c::crc32c(&directory).to_le_bytes());
    header[0x20..0x28].copy_from_slice(&file_len.to_le_bytes());
    let crc = crc32c::crc32c(&header[..0x3C]);
    header[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());

    out.write_all(&header)?;
    for frame in &frames {
        out.write_all(frame)?;
    }
    for s in &sections {
        out.write_all(&s.payload)?;
    }
    out.write_all(&directory)?;
    out.flush()?;
    Ok(Stats {
        file_len,
        tiles: frames.len(),
        tile_bytes,
    })
}

/// Write a frozen world to `path` atomically: the data goes to a temporary
/// file next to it, is synced, then renamed over the target.
pub fn save(world: &World, path: &Path) -> Result<Stats> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        let mut out = io::BufWriter::with_capacity(1 << 20, file);
        let stats = write(world, &mut out)?;
        let file = out.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(stats)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// --------------------------------------------------------------- reading --

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `bytes[offset .. offset + len]`, or an error naming `what`.
fn slice<'a>(bytes: &'a [u8], offset: u64, len: u64, what: &str) -> Result<&'a [u8]> {
    let end = offset.checked_add(len);
    match end {
        Some(end) if end <= bytes.len() as u64 => Ok(&bytes[offset as usize..end as usize]),
        _ => corrupt(format!("{what} lies outside the file")),
    }
}

struct TileEntry {
    pos: TilePos,
    offset: u64,
    len: u32,
    crc: u32,
}

struct Parsed<'a> {
    bytes: &'a [u8],
    version: u16,
    meta: MetaDoc,
    /// Tile index per `TIDX` section id.
    indices: Vec<(u32, Vec<TileEntry>)>,
    /// Sculpted chunks per `VOXL` section id, still encoded.
    voxels: Vec<(u32, Vec<u8>)>,
}

impl Parsed<'_> {
    fn index(&self, id: u32) -> Result<&[TileEntry]> {
        self.indices
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, e)| e.as_slice())
            .ok_or_else(|| FormatError::Corrupt(format!("tile index {id} is missing")))
    }
}

fn section_payload(bytes: &[u8], entry: &[u8], what: &str) -> Result<Vec<u8>> {
    let payload = slice(bytes, u64_at(entry, 8), u64_at(entry, 16), what)?;
    if crc32c::crc32c(payload) != u32_at(entry, 24) {
        return corrupt(format!("{what} fails its checksum"));
    }
    if u32_at(entry, 28) & FLAG_ZSTD != 0 {
        Ok(codec::zstd_decompress(payload)?)
    } else {
        Ok(payload.to_vec())
    }
}

fn parse(bytes: &[u8]) -> Result<Parsed<'_>> {
    if bytes.len() < HEADER_LEN || bytes[..8] != MAGIC {
        return Err(FormatError::NotMassif);
    }
    let header = &bytes[..HEADER_LEN];
    if crc32c::crc32c(&header[..0x3C]) != u32_at(header, 0x3C) {
        return corrupt("the header fails its checksum");
    }
    let version = u16_at(header, 0x08);
    let needs = u16_at(header, 0x0A);
    if needs > VERSION {
        return Err(FormatError::TooNew {
            found: needs,
            supported: VERSION,
        });
    }
    if u64_at(header, 0x20) != bytes.len() as u64 {
        return corrupt(format!(
            "it should be {} bytes long but is {}",
            u64_at(header, 0x20),
            bytes.len()
        ));
    }
    let directory = slice(
        bytes,
        u64_at(header, 0x10),
        u32_at(header, 0x18) as u64,
        "the section directory",
    )?;
    if crc32c::crc32c(directory) != u32_at(header, 0x1C) {
        return corrupt("the section directory fails its checksum");
    }
    if directory.len() % SECTION_LEN != 0 {
        return corrupt("the section directory has a ragged length");
    }

    let mut meta = None;
    let mut indices = Vec::new();
    let mut voxels = Vec::new();
    for entry in directory.as_chunks::<SECTION_LEN>().0 {
        let kind: [u8; 4] = entry[..4].try_into().unwrap();
        let id = u32_at(entry, 4);
        match kind {
            KIND_META => {
                let json = section_payload(bytes, entry, "the metadata")?;
                meta = Some(serde_json::from_slice::<MetaDoc>(&json)?);
            }
            KIND_TILE_INDEX => {
                let raw = section_payload(bytes, entry, "a tile index")?;
                if raw.len() % TILE_ENTRY_LEN != 0 {
                    return corrupt("a tile index has a ragged length");
                }
                let mut entries = Vec::with_capacity(raw.len() / TILE_ENTRY_LEN);
                for e in raw.as_chunks::<TILE_ENTRY_LEN>().0 {
                    let entry = TileEntry {
                        pos: TilePos::new(i32_at(e, 0), i32_at(e, 4)),
                        offset: u64_at(e, 8),
                        len: u32_at(e, 16),
                        crc: u32_at(e, 20),
                    };
                    slice(bytes, entry.offset, entry.len as u64, "a tile")?;
                    entries.push(entry);
                }
                indices.push((id, entries));
            }
            KIND_VOXELS => voxels.push((id, section_payload(bytes, entry, "the sculpted chunks")?)),
            _ => {} // a section from a newer version: ignore it
        }
    }
    let meta = match meta {
        Some(meta) => meta,
        None => return corrupt("the metadata section is missing"),
    };
    Ok(Parsed {
        bytes,
        version,
        meta,
        indices,
        voxels,
    })
}

/// Deserialize a world from the bytes of a `.massif` file.
///
/// Every tile's checksum is verified; tiles stay compressed until used.
pub fn read(bytes: &[u8]) -> Result<World> {
    let parsed = parse(bytes)?;
    let mut dims = Vec::with_capacity(parsed.meta.dimensions.len());
    for dim in &parsed.meta.dimensions {
        if dim.min_y >= dim.max_y {
            return corrupt("a dimension has no height");
        }
        if dim.levels.first().map(|l| l.kind) != Some(LevelKind::Ground)
            || dim.levels.iter().skip(1).any(|l| l.kind == LevelKind::Ground)
        {
            return corrupt("a dimension must start with its one ground level");
        }
        let mut levels = Vec::with_capacity(dim.levels.len());
        for doc in &dim.levels {
            let entries = parsed.index(doc.index)?;
            let blobs: Vec<Result<(TilePos, Blob)>> = entries
                .par_iter()
                .map(|e| {
                    let frame = &bytes[e.offset as usize..e.offset as usize + e.len as usize];
                    if crc32c::crc32c(frame) != e.crc {
                        return corrupt(format!(
                            "tile ({}, {}) of level \"{}\" fails its checksum",
                            e.pos.x, e.pos.z, doc.name
                        ));
                    }
                    Ok((e.pos, Blob::from(frame)))
                })
                .collect();
            let mut level = Level::new(LevelId(doc.id), doc.name.clone(), doc.kind);
            level.visible = doc.visible;
            for blob in blobs {
                let (pos, blob) = blob?;
                level.insert_blob(pos, blob);
            }
            level.take_dirty();
            levels.push(level);
        }
        let mut dimension = Dimension::from_levels(dim.kind, dim.min_y, dim.max_y, levels);
        if let Some(id) = dim.voxels {
            let Some((_, raw)) = parsed.voxels.iter().find(|(i, _)| *i == id) else {
                return corrupt(format!("the sculpted chunks {id} are missing"));
            };
            dimension.voxels = massif_core::voxel::VoxelLayer::decode(raw).map_err(FormatError::Corrupt)?;
        }
        dims.push(dimension);
    }
    if dims.is_empty() {
        return corrupt("the world has no dimension");
    }
    Ok(World {
        meta: parsed.meta.world,
        registry: parsed.meta.registry,
        dims,
    })
}

/// Read a world from a `.massif` file.
pub fn load(path: &Path) -> Result<World> {
    read(&std::fs::read(path)?)
}

/// Decode every tile of a file, to prove it is fully readable. Returns the
/// number of tiles checked.
pub fn verify(bytes: &[u8]) -> Result<usize> {
    let parsed = parse(bytes)?;
    let mut total = 0;
    for (_, entries) in &parsed.indices {
        entries.par_iter().try_for_each(|e| -> Result<()> {
            let frame = &parsed.bytes[e.offset as usize..e.offset as usize + e.len as usize];
            if crc32c::crc32c(frame) != e.crc {
                return corrupt(format!("tile ({}, {}) fails its checksum", e.pos.x, e.pos.z));
            }
            Tile::decode(frame)?;
            Ok(())
        })?;
        total += entries.len();
    }
    Ok(total)
}

// ------------------------------------------------------------ inspection --

#[derive(Debug, Clone, PartialEq)]
pub struct LevelInfo {
    pub id: u32,
    pub name: String,
    pub kind: LevelKind,
    pub tiles: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DimensionInfo {
    pub kind: DimensionKind,
    pub min_y: i32,
    pub max_y: i32,
    pub levels: Vec<LevelInfo>,
    /// Chunks sculpted in 3D (16³ blocks each), and the bytes they take.
    pub sculpted_chunks: usize,
    pub sculpted_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    pub version: u16,
    pub generator: String,
    pub file_len: u64,
    pub world: WorldMeta,
    pub blocks: usize,
    pub terrains: usize,
    pub biomes: usize,
    pub dimensions: Vec<DimensionInfo>,
}

/// Describe a file without decoding any tile.
pub fn inspect(bytes: &[u8]) -> Result<FileInfo> {
    let parsed = parse(bytes)?;
    let mut dimensions = Vec::new();
    for dim in &parsed.meta.dimensions {
        let mut levels = Vec::new();
        for doc in &dim.levels {
            let entries = parsed.index(doc.index)?;
            levels.push(LevelInfo {
                id: doc.id,
                name: doc.name.clone(),
                kind: doc.kind,
                tiles: entries.len(),
                bytes: entries.iter().map(|e| e.len as u64).sum(),
            });
        }
        let (sculpted_chunks, sculpted_bytes) = match dim.voxels.and_then(|id| parsed.voxels.iter().find(|(i, _)| *i == id)) {
            Some((_, raw)) => {
                let chunks = codec::zstd_decompress(raw)?.len() / (12 + massif_core::voxel::VOLUME);
                (chunks, raw.len() as u64)
            }
            None => (0, 0),
        };
        dimensions.push(DimensionInfo {
            kind: dim.kind,
            min_y: dim.min_y,
            max_y: dim.max_y,
            levels,
            sculpted_chunks,
            sculpted_bytes,
        });
    }
    Ok(FileInfo {
        version: parsed.version,
        generator: parsed.meta.generator.clone(),
        file_len: bytes.len() as u64,
        world: parsed.meta.world.clone(),
        blocks: parsed.meta.registry.blocks.len(),
        terrains: parsed.meta.registry.terrains.len(),
        biomes: parsed.meta.registry.biomes.len(),
        dimensions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, Landform, generate};
    use massif_core::history::History;
    use massif_core::ops::{self, Anchor};
    use massif_core::{Brush, BrushShape, Profile};

    fn sample_world() -> World {
        let mut world = generate(&GenParams {
            tiles_x: 3,
            tiles_z: 2,
            landform: Landform::Hills,
            ..GenParams::default()
        });
        let mut hist = History::default();
        hist.begin("levels", 0);
        let island = hist
            .edit(&mut world, 0)
            .structure()
            .add_level("Île flottante", LevelKind::Solid);
        let cave = hist
            .edit(&mut world, 0)
            .structure()
            .add_level("Cave", LevelKind::Carve);
        hist.commit();
        let brush = Brush {
            shape: BrushShape::Circle,
            radius: 20.0,
            profile: Profile::Dome,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
        };
        hist.begin("stamp", 0);
        ops::stamp_slab(&mut hist.edit(&mut world, island), &brush, -3.0, 5.0, Anchor::Absolute(150.0), 24.0);
        ops::stamp_slab(&mut hist.edit(&mut world, cave), &brush, 40.0, -10.0, Anchor::Ground(-20.0), 12.0);
        ops::raise(&mut hist.edit(&mut world, 0), &brush, 0.0, 0.0, 9.0, ops::Surface::Top);
        hist.commit();
        world.dims[0].levels[cave].visible = false;
        // A tunnel sculpted in 3D.
        hist.begin("dig", 0);
        massif_core::voxel::sculpt(&mut hist.edit(&mut world, 0), &brush, [10.5, 40.5, 10.5], 1.0, massif_core::voxel::SculptMode::Remove);
        hist.commit();
        world
    }

    fn to_bytes(world: &mut World) -> Vec<u8> {
        let mut bytes = Vec::new();
        let stats = write(&world.snapshot(), &mut bytes).unwrap();
        assert_eq!(stats.file_len, bytes.len() as u64);
        bytes
    }

    fn assert_same(a: &mut World, b: &mut World) {
        assert_eq!(a.meta, b.meta);
        assert_eq!(a.registry, b.registry);
        assert_eq!(a.dims.len(), b.dims.len());
        for (da, db) in a.dims.iter_mut().zip(b.dims.iter_mut()) {
            assert_eq!((da.kind, da.min_y, da.max_y), (db.kind, db.min_y, db.max_y));
            assert_eq!(da.levels.len(), db.levels.len());
            for (la, lb) in da.levels.iter_mut().zip(db.levels.iter_mut()) {
                assert_eq!((la.id, &la.name, la.kind, la.visible), (lb.id, &lb.name, lb.kind, lb.visible));
                la.freeze();
                lb.freeze();
                assert_eq!(la.blobs().unwrap(), lb.blobs().unwrap(), "level {}", la.name);
            }
            assert_eq!(da.voxels.len(), db.voxels.len());
            for (pos, chunk) in da.voxels.iter() {
                assert_eq!(db.voxels.get(pos), Some(&**chunk), "sculpted chunk {pos:?}");
            }
        }
    }

    #[test]
    fn worlds_round_trip() {
        let mut world = sample_world();
        let bytes = to_bytes(&mut world);
        let mut back = read(&bytes).unwrap();
        assert_same(&mut world, &mut back);
        assert_eq!(back.dims[0].levels[1].name, "Île flottante");
        assert!(back.dims[0].voxels.len() > 8, "the sculpted tunnel is saved");
        let info = inspect(&bytes).unwrap();
        assert_eq!(info.dimensions[0].sculpted_chunks, back.dims[0].voxels.len());
        // Nothing is decoded on load, and nothing is flagged for redraw.
        for level in &back.dims[0].levels {
            assert_eq!(level.stats().hot_tiles, 0);
            assert!(!level.has_dirty());
        }
        assert_eq!(verify(&bytes).unwrap(), 6 + 4 + 2);
        // Saving what was loaded reproduces the file byte for byte.
        assert_eq!(to_bytes(&mut back), bytes);
    }

    #[test]
    fn writing_requires_a_frozen_world() {
        let mut world = sample_world();
        // `sample_world` leaves edited tiles hot.
        assert!(matches!(write(&world, Vec::new()), Err(FormatError::NotFrozen)));
        world.dims[0].levels.iter_mut().for_each(Level::freeze);
        assert!(write(&world, Vec::new()).is_ok());
    }

    #[test]
    fn inspect_reports_levels_without_decoding() {
        let mut world = sample_world();
        let bytes = to_bytes(&mut world);
        let info = inspect(&bytes).unwrap();
        assert_eq!(info.version, VERSION);
        assert_eq!(info.world.name, "Generated World");
        let levels = &info.dimensions[0].levels;
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].kind, levels[0].tiles), (LevelKind::Ground, 6));
        assert_eq!((levels[1].kind, levels[1].tiles), (LevelKind::Solid, 4));
        assert_eq!((levels[2].kind, levels[2].tiles), (LevelKind::Carve, 2));
        assert!(levels[0].bytes > levels[1].bytes);
    }

    #[test]
    fn damage_is_detected_everywhere() {
        let mut world = sample_world();
        let bytes = to_bytes(&mut world);

        assert!(matches!(read(b"hello"), Err(FormatError::NotMassif)));
        assert!(matches!(read(&[0u8; 200]), Err(FormatError::NotMassif)));

        // Truncation.
        assert!(matches!(read(&bytes[..bytes.len() - 1]), Err(FormatError::Corrupt(_))));
        assert!(matches!(read(&bytes[..HEADER_LEN]), Err(FormatError::Corrupt(_))));

        // A flipped bit anywhere must be caught by one of the checksums.
        for at in [9usize, 0x30, HEADER_LEN + 5, bytes.len() / 2, bytes.len() - 40, bytes.len() - 3] {
            let mut bad = bytes.clone();
            bad[at] ^= 0x10;
            assert!(read(&bad).is_err(), "flip at {at} went unnoticed");
        }
    }

    #[test]
    fn newer_files_are_refused_politely() {
        let mut world = sample_world();
        let mut bytes = to_bytes(&mut world);
        bytes[0x0A..0x0C].copy_from_slice(&(VERSION + 1).to_le_bytes());
        let crc = crc32c::crc32c(&bytes[..0x3C]);
        bytes[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());
        match read(&bytes) {
            Err(FormatError::TooNew { found, supported }) => {
                assert_eq!((found, supported), (VERSION + 1, VERSION));
            }
            other => panic!("expected TooNew, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn unknown_sections_are_ignored() {
        let mut world = sample_world();
        let bytes = to_bytes(&mut world);
        // Append a section of an unknown kind by rebuilding the directory.
        let dir_off = u64_at(&bytes, 0x10) as usize;
        let dir_len = u32_at(&bytes, 0x18) as usize;
        let mut out = bytes[..dir_off].to_vec();
        let extra = b"from the future";
        out.extend_from_slice(extra);
        let mut dir = bytes[dir_off..dir_off + dir_len].to_vec();
        dir.extend_from_slice(b"XTRA");
        dir.extend_from_slice(&7u32.to_le_bytes());
        dir.extend_from_slice(&(dir_off as u64).to_le_bytes());
        dir.extend_from_slice(&(extra.len() as u64).to_le_bytes());
        dir.extend_from_slice(&crc32c::crc32c(extra).to_le_bytes());
        dir.extend_from_slice(&0u32.to_le_bytes());
        let new_dir_off = out.len() as u64;
        out.extend_from_slice(&dir);
        let file_len = out.len() as u64;
        out[0x10..0x18].copy_from_slice(&new_dir_off.to_le_bytes());
        out[0x18..0x1C].copy_from_slice(&(dir.len() as u32).to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&crc32c::crc32c(&dir).to_le_bytes());
        out[0x20..0x28].copy_from_slice(&file_len.to_le_bytes());
        let crc = crc32c::crc32c(&out[..0x3C]);
        out[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());

        let mut back = read(&out).unwrap();
        assert_same(&mut world, &mut back);
    }

    #[test]
    fn save_is_atomic_and_loadable() {
        let dir = std::env::temp_dir().join(format!("massif-format-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.massif");
        let mut world = sample_world();
        let stats = save(&world.snapshot(), &path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), stats.file_len);
        assert!(!dir.join("world.massif.tmp").exists());
        let mut back = load(&path).unwrap();
        assert_same(&mut world, &mut back);
        // Saving over an existing file replaces it.
        save(&back.snapshot(), &path).unwrap();
        assert!(load(&path).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_large_flat_world_is_tiny_on_disk() {
        let mut world = generate(&GenParams {
            tiles_x: 32,
            tiles_z: 32,
            landform: Landform::Flat,
            ..GenParams::default()
        });
        let bytes = to_bytes(&mut world);
        // 4096×4096 blocks, 16.7 million columns.
        assert!(bytes.len() < 80_000, "{} bytes", bytes.len());
        let back = read(&bytes).unwrap();
        assert_eq!(back.dims[0].ground().len(), 1024);
    }
}
