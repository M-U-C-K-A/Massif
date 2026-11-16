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
//! │ SELN  a dimension's selection                                           │
//! │ VERS  zstd(JSON): the save points, oldest first (0.6.0 on)              │
//! │ VIDX · VVOX · VSEL · VTHM: a save point's tile index of one level,      │
//! │       sculpted chunks, selection and picture, as TIDX · VOXL · SELN     │
//! │ THMB  a JPEG picture of the map, for file browsers (Finder, home)       │
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
//!
//! Save points live in the file, not beside it, so that they travel with a
//! map copied, sent or backed up as one file. The map's own tiles and
//! sections are written exactly as before them; a point's tile index points
//! at the map's frame where the tile is the same (the same blob in memory,
//! or the same bytes: same CRC32C, length and content), and only its other
//! tiles add frames. Older versions read the map and skip the points'
//! sections; saving from them drops the points.

use massif_core::codec::{self, CodecError};
use massif_core::level::{Blob, Level, LevelId, LevelKind};
use massif_core::registry::Registry;
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};
use massif_core::versions::SavePoint;
use massif_core::{Tile, TilePos};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
/// A dimension's selection (0.6.0 on; older versions skip it).
const KIND_SELECTION: [u8; 4] = *b"SELN";
/// A picture of the map: the Finder's Quick Look extension shows it as the
/// file's icon (see `tools/macos/thumbnails`).
pub const KIND_THUMBNAIL: [u8; 4] = *b"THMB";
/// Save points (0.6.0 on; older versions skip them): their list, and for
/// each the tile index of a level, its sculpted chunks, its selection and
/// its picture. Their tiles lie with the map's, each distinct one once.
const KIND_POINTS: [u8; 4] = *b"VERS";
const KIND_POINT_INDEX: [u8; 4] = *b"VIDX";
const KIND_POINT_VOXELS: [u8; 4] = *b"VVOX";
const KIND_POINT_SELECTION: [u8; 4] = *b"VSEL";
const KIND_POINT_THUMBNAIL: [u8; 4] = *b"VTHM";
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
    /// Id of the `SELN` section holding the selection, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<u32>,
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

/// A save point (see [`massif_core::versions`]), in the `VERS` section.
#[derive(Debug, Serialize, Deserialize)]
struct PointDoc {
    name: String,
    created: u64,
    world: WorldMeta,
    /// `None`: the map's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registry: Option<Registry>,
    /// Its levels' tile indices are `VIDX` sections, its sculpted chunks
    /// `VVOX` and its selection `VSEL`.
    dimensions: Vec<DimDoc>,
    /// Id of the `VTHM` section holding its picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    thumbnail: Option<u32>,
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
    /// Of those, the frames only save points use.
    pub point_bytes: u64,
}

struct Section {
    kind: [u8; 4],
    id: u32,
    flags: u32,
    payload: Vec<u8>,
}

/// Lays out the tile frames and sections of a file.
struct Layout {
    /// Where the next frame goes.
    offset: u64,
    frames: Vec<Blob>,
    sections: Vec<Section>,
    next_section: u32,
    /// The frames written, by allocation and by content (CRC32C and
    /// length, the bytes compared on a match): a save point's tile already
    /// written is not written again.
    by_ptr: HashMap<usize, (u64, u32)>,
    by_content: HashMap<(u32, u32), Vec<(u64, usize)>>,
}

/// The address of a blob's bytes, to know a shared tile without reading it.
fn ptr(blob: &Blob) -> usize {
    blob.as_ptr() as usize
}

impl Layout {
    fn section(&mut self, kind: [u8; 4], flags: u32, payload: Vec<u8>) -> u32 {
        let id = self.next_section;
        self.next_section += 1;
        self.sections.push(Section { kind, id, flags, payload });
        id
    }

    /// The dimensions of `world`, their frames and sections. With `share`,
    /// a tile already written is pointed at instead of written again, and
    /// the sections are of the kinds older versions skip (a save point's).
    fn dims(&mut self, world: &World, share: bool) -> Result<Vec<DimDoc>> {
        let (index_kind, voxel_kind, selection_kind) = if share {
            (KIND_POINT_INDEX, KIND_POINT_VOXELS, KIND_POINT_SELECTION)
        } else {
            (KIND_TILE_INDEX, KIND_VOXELS, KIND_SELECTION)
        };
        let mut dims = Vec::new();
        for dim in &world.dims {
            let mut levels = Vec::new();
            for level in &dim.levels {
                let blobs = level.blobs().ok_or(FormatError::NotFrozen)?;
                // A save point's tiles are mostly the map's own: only those
                // not met yet need their checksum.
                let crcs: Vec<Option<(u64, u32)>> = if share {
                    blobs.par_iter().map(|(_, b)| self.by_ptr.get(&ptr(b)).copied()).collect()
                } else {
                    vec![None; blobs.len()]
                };
                let fresh: Vec<u32> = blobs
                    .par_iter()
                    .zip(&crcs)
                    .map(|((_, b), known)| if known.is_some() { 0 } else { crc32c::crc32c(b) })
                    .collect();
                let mut index = Vec::with_capacity(blobs.len() * TILE_ENTRY_LEN);
                for (((pos, blob), known), crc) in blobs.into_iter().zip(crcs).zip(fresh) {
                    let len = u32::try_from(blob.len()).map_err(|_| FormatError::Corrupt("tile frame over 4 GiB".into()))?;
                    let (at, crc) = match known {
                        Some(found) => found,
                        None => {
                            let same = if share {
                                self.by_content
                                    .get(&(crc, len))
                                    .and_then(|found| found.iter().find(|&&(_, k)| self.frames[k][..] == blob[..]))
                                    .map(|&(at, _)| at)
                            } else {
                                None
                            };
                            match same {
                                Some(at) => (at, crc),
                                None => {
                                    let at = self.offset;
                                    self.offset += len as u64;
                                    self.by_ptr.insert(ptr(&blob), (at, crc));
                                    self.by_content.entry((crc, len)).or_default().push((at, self.frames.len()));
                                    self.frames.push(blob);
                                    (at, crc)
                                }
                            }
                        }
                    };
                    index.extend_from_slice(&pos.x.to_le_bytes());
                    index.extend_from_slice(&pos.z.to_le_bytes());
                    index.extend_from_slice(&at.to_le_bytes());
                    index.extend_from_slice(&len.to_le_bytes());
                    index.extend_from_slice(&crc.to_le_bytes());
                }
                let id = self.section(index_kind, FLAG_ZSTD, codec::zstd_compress(&index));
                levels.push(LevelDoc {
                    id: level.id.0,
                    name: level.name.clone(),
                    kind: level.kind,
                    visible: level.visible,
                    index: id,
                });
            }
            // The layer is zstd'd already: stored as is.
            let voxels = (!dim.voxels.is_empty()).then(|| self.section(voxel_kind, 0, dim.voxels.encode()));
            let selection = dim
                .selection
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| self.section(selection_kind, 0, s.encode()));
            dims.push(DimDoc {
                kind: dim.kind,
                min_y: dim.min_y,
                max_y: dim.max_y,
                levels,
                voxels,
                selection,
            });
        }
        Ok(dims)
    }
}

/// Serialize a world, with its save points. Every level must be frozen
/// (see [`massif_core::World::snapshot`]); nothing is encoded here.
///
/// The map is written as before save points were: older versions read it
/// and skip them. Each save point's tiles follow the map's, but only those
/// not written already (by allocation or by content), so a point costs the
/// tiles changed since it was made.
pub fn write<W: Write>(world: &World, mut out: W) -> Result<Stats> {
    // Lay the tile frames out right after the header.
    let mut layout = Layout {
        offset: HEADER_LEN as u64,
        frames: Vec::new(),
        sections: Vec::new(),
        next_section: 1,
        by_ptr: HashMap::new(),
        by_content: HashMap::new(),
    };
    let dims = layout.dims(world, false)?;
    let map_bytes = layout.offset - HEADER_LEN as u64;
    let mut points = Vec::with_capacity(world.points.len());
    for point in &world.points {
        let dimensions = layout.dims(&point.world, true)?;
        let thumbnail = point.thumbnail.as_ref().map(|jpeg| layout.section(KIND_POINT_THUMBNAIL, 0, jpeg.to_vec()));
        points.push(PointDoc {
            name: point.name.clone(),
            created: point.created,
            world: point.world.meta.clone(),
            registry: (point.world.registry != world.registry).then(|| point.world.registry.clone()),
            dimensions,
            thumbnail,
        });
    }
    let tile_bytes = layout.offset - HEADER_LEN as u64;
    let Layout {
        mut offset,
        frames,
        mut sections,
        next_section,
        ..
    } = layout;
    let mut next_section = next_section;
    if !points.is_empty() {
        sections.push(Section {
            kind: KIND_POINTS,
            id: next_section,
            flags: FLAG_ZSTD,
            payload: codec::zstd_compress(&serde_json::to_vec(&points)?),
        });
        next_section += 1;
    }

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
    // A picture of the map, last, for file browsers.
    if let Some(jpeg) = massif_core::thumbnail::jpeg(world) {
        sections.push(Section {
            kind: KIND_THUMBNAIL,
            id: next_section,
            flags: 0,
            payload: jpeg,
        });
    }

    // A section this version could not read back is refused before anything
    // is written: the file saved before stays as it was.
    for s in &sections {
        if (s.flags & FLAG_ZSTD != 0 || [KIND_VOXELS, KIND_POINT_VOXELS, KIND_SELECTION, KIND_POINT_SELECTION].contains(&s.kind))
            && codec::zstd_content_size(&s.payload).is_none_or(|n| n > codec::MAX_RAW_SECTION)
        {
            return Err(FormatError::Corrupt(format!("the {} section is too large to be read again", String::from_utf8_lossy(&s.kind))));
        }
    }

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
        point_bytes: tile_bytes - map_bytes,
    })
}

/// Write a frozen world to `path` atomically: the data goes to a temporary
/// file next to it, is synced, then renamed over the target. On any
/// failure the file already at `path` is left as it was (see
/// [`massif_core::atomic`]; [`describe`] words the error).
pub fn save(world: &World, path: &Path) -> Result<Stats> {
    let mut stats = None;
    let written = massif_core::atomic::write_with(path, true, |out| match write(world, out) {
        Ok(s) => {
            stats = Some(s);
            Ok(())
        }
        Err(FormatError::Io(e)) => Err(e),
        Err(other) => Err(io::Error::other(other)),
    });
    match (written, stats) {
        (Ok(()), Some(stats)) => Ok(stats),
        (Err(e), _) => Err(match e.get_ref().is_some_and(|inner| inner.is::<FormatError>()) {
            true => *e.into_inner().expect("checked").downcast::<FormatError>().expect("checked"),
            false => FormatError::Io(e),
        }),
        (Ok(()), None) => corrupt("nothing was written"),
    }
}

/// Why saving to `path` failed, in words a map maker can act on.
pub fn describe(e: &FormatError, path: &Path) -> String {
    match e {
        FormatError::Io(io) => massif_core::atomic::describe(io, path),
        other => format!("{other}. The file already there was not changed."),
    }
}

// --------------------------------------------------------------- reading --

/// The picture of the map a `.massif` file keeps (a JPEG image), read
/// without loading the map; `None` for a file without one.
pub fn read_thumbnail(path: &Path) -> Result<Option<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    if header[..8] != MAGIC {
        return Err(FormatError::NotMassif);
    }
    let (offset, len) = (u64_at(&header, 0x10), u32_at(&header, 0x18) as usize);
    if len % SECTION_LEN != 0 || len > 1 << 24 {
        return corrupt("the section directory has a ragged length");
    }
    let mut directory = vec![0u8; len];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut directory)?;
    for entry in directory.as_chunks::<SECTION_LEN>().0 {
        if entry[..4] == KIND_THUMBNAIL {
            let (at, len) = (u64_at(entry, 8), u64_at(entry, 16));
            if len > 1 << 24 {
                return corrupt("the picture is too large");
            }
            let mut jpeg = vec![0u8; len as usize];
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut jpeg)?;
            if crc32c::crc32c(&jpeg) != u32_at(entry, 24) {
                return corrupt("the picture fails its checksum");
            }
            return Ok(Some(jpeg));
        }
    }
    Ok(None)
}

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
    /// Selections per `SELN` section id, still encoded.
    selections: Vec<(u32, Vec<u8>)>,
    /// The save points, and their pictures per `VTHM` section id.
    points: Vec<PointDoc>,
    point_thumbnails: Vec<(u32, Vec<u8>)>,
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
        Ok(codec::zstd_decompress_section(payload)?)
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
    let mut selections = Vec::new();
    let mut points = Vec::new();
    let mut point_thumbnails = Vec::new();
    for entry in directory.as_chunks::<SECTION_LEN>().0 {
        let kind: [u8; 4] = entry[..4].try_into().unwrap();
        let id = u32_at(entry, 4);
        match kind {
            KIND_META => {
                let json = section_payload(bytes, entry, "the metadata")?;
                meta = Some(serde_json::from_slice::<MetaDoc>(&json)?);
            }
            KIND_TILE_INDEX | KIND_POINT_INDEX => {
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
            KIND_VOXELS | KIND_POINT_VOXELS => voxels.push((id, section_payload(bytes, entry, "the sculpted chunks")?)),
            KIND_SELECTION | KIND_POINT_SELECTION => selections.push((id, section_payload(bytes, entry, "the selection")?)),
            KIND_POINTS => {
                let json = section_payload(bytes, entry, "the save points")?;
                points = serde_json::from_slice::<Vec<PointDoc>>(&json)?;
            }
            KIND_POINT_THUMBNAIL => point_thumbnails.push((id, section_payload(bytes, entry, "a save point's picture")?)),
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
        selections,
        points,
        point_thumbnails,
    })
}

/// The tiles read so far, by where they lie in the file: a tile a save
/// point shares with the map, or with another point, is one blob in memory.
type Frames = HashMap<u64, Blob>;

/// The dimensions described by `docs`, their tiles read from `parsed`;
/// with `keep`, the tiles are kept in `frames` for the save points to share.
fn read_dims(parsed: &Parsed, docs: &[DimDoc], frames: &mut Frames, keep: bool) -> Result<Vec<Dimension>> {
    let bytes = parsed.bytes;
    let mut dims = Vec::with_capacity(docs.len());
    for dim in docs {
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
            let known: &Frames = frames;
            let blobs: Vec<Result<(TilePos, u64, Blob, bool)>> = entries
                .par_iter()
                .map(|e| {
                    if let Some(blob) = known.get(&e.offset)
                        && blob.len() == e.len as usize
                    {
                        return Ok((e.pos, e.offset, blob.clone(), false));
                    }
                    let frame = &bytes[e.offset as usize..e.offset as usize + e.len as usize];
                    if crc32c::crc32c(frame) != e.crc {
                        return corrupt(format!(
                            "tile ({}, {}) of level \"{}\" fails its checksum",
                            e.pos.x, e.pos.z, doc.name
                        ));
                    }
                    Ok((e.pos, e.offset, Blob::from(frame), true))
                })
                .collect();
            let mut level = Level::new(LevelId(doc.id), doc.name.clone(), doc.kind);
            level.visible = doc.visible;
            for blob in blobs {
                let (pos, at, blob, fresh) = blob?;
                if fresh && keep {
                    frames.entry(at).or_insert_with(|| blob.clone());
                }
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
        if let Some(id) = dim.selection {
            let Some((_, raw)) = parsed.selections.iter().find(|(i, _)| *i == id) else {
                return corrupt(format!("the selection {id} is missing"));
            };
            let selection = massif_core::selection::Selection::decode(raw).map_err(FormatError::Corrupt)?;
            dimension.selection = (!selection.is_empty()).then(|| std::sync::Arc::new(selection));
        }
        dims.push(dimension);
    }
    if dims.is_empty() {
        return corrupt("the world has no dimension");
    }
    Ok(dims)
}

/// Deserialize a world from the bytes of a `.massif` file, with its save
/// points.
///
/// Every tile's checksum is verified; tiles stay compressed until used.
pub fn read(bytes: &[u8]) -> Result<World> {
    let mut parsed = parse(bytes)?;
    let mut frames = Frames::new();
    let dims = read_dims(&parsed, &parsed.meta.dimensions, &mut frames, !parsed.points.is_empty())?;
    let mut points = Vec::with_capacity(parsed.points.len());
    for doc in std::mem::take(&mut parsed.points) {
        let world = World {
            meta: doc.world,
            registry: doc.registry.unwrap_or_else(|| parsed.meta.registry.clone()),
            dims: read_dims(&parsed, &doc.dimensions, &mut frames, true)?,
            points: Vec::new(),
        };
        let thumbnail = doc
            .thumbnail
            .and_then(|id| parsed.point_thumbnails.iter().find(|(i, _)| *i == id))
            .map(|(_, jpeg)| std::sync::Arc::<[u8]>::from(jpeg.as_slice()));
        points.push(SavePoint {
            name: doc.name,
            created: doc.created,
            world: std::sync::Arc::new(world),
            thumbnail,
        });
    }
    Ok(World {
        meta: parsed.meta.world,
        registry: parsed.meta.registry,
        dims,
        points,
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
    /// The save points' names, oldest first, and the bytes of the tiles
    /// only they use.
    pub points: Vec<String>,
    pub point_bytes: u64,
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
                let chunks = codec::zstd_decompress_section(raw)?.len() / (12 + massif_core::voxel::VOLUME);
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
    let map: std::collections::HashSet<u64> = parsed
        .meta
        .dimensions
        .iter()
        .flat_map(|d| &d.levels)
        .filter_map(|l| parsed.index(l.index).ok())
        .flat_map(|entries| entries.iter().map(|e| e.offset))
        .collect();
    let mut only_points = HashMap::new();
    for entries in parsed.points.iter().flat_map(|p| &p.dimensions).flat_map(|d| &d.levels).filter_map(|l| parsed.index(l.index).ok()) {
        for e in entries.iter().filter(|e| !map.contains(&e.offset)) {
            only_points.insert(e.offset, e.len as u64);
        }
    }
    Ok(FileInfo {
        version: parsed.version,
        points: parsed.points.iter().map(|p| p.name.clone()).collect(),
        point_bytes: only_points.values().sum(),
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
            filter: None,
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
            assert_eq!(da.selection, db.selection);
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
    fn a_selection_is_saved_with_the_map() {
        use massif_core::selection::{Combine, Selection, Shape};
        let mut world = sample_world();
        let without = to_bytes(&mut world);
        assert!(read(&without).unwrap().dims[0].selection.is_none());
        let shape = Selection::shape(&Shape::Rect(massif_core::BlockRect::new(-20, -20, 140, 60))).feather(10);
        let lasso = Selection::shape(&Shape::Polygon(vec![(0.0, 0.0), (300.0, 10.0), (50.0, 200.0)]));
        let mut hist = massif_core::History::default();
        assert!(hist.select(&mut world, 0, "Select", Some(shape.combine(&lasso, Combine::Add))));
        let bytes = to_bytes(&mut world);
        let mut back = read(&bytes).unwrap();
        assert!(back.dims[0].selection.is_some());
        assert_same(&mut world, &mut back);
        // Undone, the map has none again, and saves as it did.
        hist.undo(&mut world);
        assert!(world.dims[0].selection.is_none());
        assert!(to_bytes(&mut world) == without);
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
        // The file keeps a picture of the map, read without the rest.
        let jpeg = read_thumbnail(&path).unwrap().expect("a picture of the map");
        assert!(jpeg.starts_with(&[0xff, 0xd8]));
        // Saving over an existing file replaces it.
        save(&back.snapshot(), &path).unwrap();
        assert!(load(&path).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The file without the sections of the given kinds, as a reader that
    /// does not know them sees it.
    fn without_sections(bytes: &[u8], drop: &[[u8; 4]]) -> Vec<u8> {
        let dir_off = u64_at(bytes, 0x10) as usize;
        let dir_len = u32_at(bytes, 0x18) as usize;
        let mut out = bytes[..dir_off].to_vec();
        let mut dir = Vec::new();
        for entry in bytes[dir_off..dir_off + dir_len].as_chunks::<SECTION_LEN>().0 {
            if !drop.iter().any(|k| entry[..4] == k[..]) {
                dir.extend_from_slice(entry);
            }
        }
        let new_dir_off = out.len() as u64;
        out.extend_from_slice(&dir);
        let file_len = out.len() as u64;
        out[0x10..0x18].copy_from_slice(&new_dir_off.to_le_bytes());
        out[0x18..0x1C].copy_from_slice(&(dir.len() as u32).to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&crc32c::crc32c(&dir).to_le_bytes());
        out[0x20..0x28].copy_from_slice(&file_len.to_le_bytes());
        let crc = crc32c::crc32c(&out[..0x3C]);
        out[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());
        out
    }

    /// Raise the land in a circle at (`x`, `z`), as one step.
    fn raise_at(world: &mut World, hist: &mut History, x: f32, z: f32) {
        let brush = Brush {
            shape: BrushShape::Circle,
            radius: 12.0,
            profile: Profile::Dome,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        };
        hist.begin("raise", 0);
        ops::raise(&mut hist.edit(world, 0), &brush, x, z, 6.0, ops::Surface::Top);
        hist.commit();
    }

    #[test]
    fn save_points_round_trip_and_share_their_tiles() {
        let mut world = sample_world();
        let mut hist = History::default();
        let plain = to_bytes(&mut world);

        // A point of the map as it is costs no tile.
        let first = SavePoint::new(&mut world, "before the rivers", 1_700_000_000);
        assert!(first.thumbnail.as_ref().is_some_and(|j| j.starts_with(&[0xff, 0xd8])));
        world.points.push(first);
        let mut one = Vec::new();
        let stats = write(&world.snapshot(), &mut one).unwrap();
        assert_eq!(stats.point_bytes, 0);
        assert_eq!(stats.tile_bytes, write(&{ let mut w = world.snapshot(); w.points.clear(); w }, Vec::new()).unwrap().tile_bytes);
        assert!(one.len() - plain.len() < 20_000, "a point of an unchanged map takes {} bytes", one.len() - plain.len());

        // The map changes in one corner: the point keeps the tiles it had
        // there, and only those are added.
        let (lo, hi) = world.dims[0].bounds().unwrap();
        raise_at(&mut world, &mut hist, (hi.x * 128 + 64) as f32, (hi.z * 128 + 64) as f32);
        let changed = SavePoint::changed_tiles(&world.snapshot(), &world.points[0].world, 0);
        assert!(!changed.is_empty() && changed.len() < 6, "{changed:?}");
        let old: u64 = changed.iter().map(|&p| world.points[0].world.dims[0].levels[0].blob_len(p) as u64).sum();
        let point = SavePoint::new(&mut world, "version 2", 1_700_000_100);
        world.points.push(point);
        let mut two = Vec::new();
        let stats = write(&world.snapshot(), &mut two).unwrap();
        assert_eq!(stats.point_bytes, old, "only the tiles changed since the first point are added");
        let info = inspect(&two).unwrap();
        assert_eq!(info.points, ["before the rivers", "version 2"]);
        assert_eq!(info.point_bytes, old);

        // Read back: the map and both points, the shared tiles one blob.
        let mut back = read(&two).unwrap();
        assert_same(&mut world, &mut back);
        assert_eq!(back.points.len(), 2);
        for (a, b) in world.points.iter().zip(&back.points) {
            assert_eq!((&a.name, a.created, &a.thumbnail), (&b.name, b.created, &b.thumbnail));
            let (mut wa, mut wb) = ((*a.world).clone(), (*b.world).clone());
            assert_same(&mut wa, &mut wb);
        }
        let far = lo;
        assert!(!changed.contains(&far));
        let map_tile = back.dims[0].levels[0].blob(far).unwrap();
        for p in &back.points {
            assert!(std::sync::Arc::ptr_eq(&p.world.dims[0].levels[0].blobs().unwrap().into_iter().find(|t| t.0 == far).unwrap().1, &map_tile));
        }
        assert_eq!(verify(&two).unwrap(), 2 * (6 + 4 + 2) + 6 + 4 + 2);
        // Saving what was loaded gives the same file.
        assert_eq!(to_bytes(&mut back), two);
    }

    #[test]
    fn older_versions_read_the_map_and_skip_its_save_points() {
        let mut world = sample_world();
        let mut hist = History::default();
        let plain = to_bytes(&mut world);
        let point = SavePoint::new(&mut world, "first", 1);
        world.points.push(point);
        raise_at(&mut world, &mut hist, 40.0, 40.0);
        let mut map_alone = world.snapshot();
        map_alone.points.clear();
        let alone = to_bytes(&mut map_alone);
        let with_points = to_bytes(&mut world);
        // Every section older versions know is the same, at the same place
        // but for the frames the points add before them.
        let old = [KIND_META, KIND_TILE_INDEX, KIND_VOXELS, KIND_SELECTION, KIND_THUMBNAIL];
        let kinds = |bytes: &[u8]| -> Vec<[u8; 4]> {
            let (at, len) = (u64_at(bytes, 0x10) as usize, u32_at(bytes, 0x18) as usize);
            bytes[at..at + len].as_chunks::<SECTION_LEN>().0.iter().map(|e| e[..4].try_into().unwrap()).collect()
        };
        assert!(kinds(&alone).iter().all(|k| old.contains(k)));
        assert!(kinds(&with_points).iter().any(|k| !old.contains(k)));
        // A reader that knows none of the new sections reads the map as it is.
        let seen_by_old = without_sections(&with_points, &[KIND_POINTS, KIND_POINT_INDEX, KIND_POINT_VOXELS, KIND_POINT_SELECTION, KIND_POINT_THUMBNAIL]);
        let mut back = read(&seen_by_old).unwrap();
        assert!(back.points.is_empty());
        assert_same(&mut map_alone, &mut back);
        // ... and this one too, the points' kinds unknown to it.
        let mut back = read(&with_points).unwrap();
        back.points.clear();
        assert_same(&mut map_alone, &mut back);
        // A file without points is written as before them.
        assert!(kinds(&plain).iter().all(|k| old.contains(k)));
    }

    #[test]
    fn a_save_point_put_back_is_one_step_to_undo() {
        let mut world = sample_world();
        let mut hist = History::default();
        world.dims[0].levels.iter_mut().for_each(Level::freeze);
        let mut before = world.snapshot();
        let point = SavePoint::new(&mut world, "first", 1);
        world.points.push(point);
        raise_at(&mut world, &mut hist, 40.0, 40.0);
        hist.begin("levels", 0);
        hist.edit(&mut world, 0).structure().add_level("Late", LevelKind::Solid);
        hist.commit();
        world.meta.name = "Renamed".into();
        let mut after = world.snapshot();
        let point = world.points[0].world.clone();
        hist.restore_point(&mut world, &point, "Restore “first”");
        // The map is the point's, but keeps its name and its points.
        before.meta.name = "Renamed".into();
        before.points = world.points.clone();
        assert_same(&mut before, &mut world);
        assert_eq!(world.points.len(), 1);
        assert_eq!(hist.undo_label(), Some("Restore “first”"));
        assert_eq!(hist.undo(&mut world).as_deref(), Some("Restore “first”"));
        assert_same(&mut after, &mut world);
        hist.redo(&mut world);
        assert_same(&mut before, &mut world);
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
