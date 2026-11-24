//! Converting a WorldPainter world into a Massif world.
//!
//! A `.world` file is a gzip stream (whose header may carry a JPEG thumbnail
//! in a "WP" extra subfield) wrapping a Java serialization stream: a metadata
//! map, then an `org.pepsoft.worldpainter.World2`. Tiles are converted the
//! moment they are read, on the worker threads, and their arrays dropped, so
//! memory stays flat even for worlds of several gigabytes.
//!
//! WorldPainter keeps, per column, a height `h` (stored ×256, relative to
//! `minHeight`; the top block is at `round(h)`), a terrain ordinal, a water
//! level (water fills up to that level inclusive) and layer values. Massif
//! surfaces sit one block higher (solid below `round(top)`), hence the `+ 1`.

use crate::javaser::{Arena, Array, Content, Handle, JavaError, Parser, Value};
use crate::layers::{Conv, Curve, Painted};
use crate::terrains::{CustomMaterial, Mapper};
use flate2::read::GzDecoder;
use massif_core::codec::snap_height;
use massif_core::compose::block_top;
use massif_core::level::{Blob, LevelKind};
use massif_core::plane::Plane;
use massif_core::registry::Registry;
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};
use massif_core::{TILE_AREA, TilePos};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const TILE_CLASS: &str = "org.pepsoft.worldpainter.Tile";
const WORLD_CLASS: &str = "org.pepsoft.worldpainter.World2";
const LEGACY_WORLD_CLASS: &str = "org.pepsoft.worldpainter.World";
const BIOME_LAYER: &str = "org.pepsoft.worldpainter.layers.Biome";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("this is not a WorldPainter world")]
    NotWorldPainter,
    #[error("the world file is damaged or incomplete ({0})")]
    Damaged(JavaError),
    #[error("{0}")]
    Unsupported(String),
    #[error("the import was cancelled")]
    Cancelled,
}

/// Progress of an import, shared with the thread running it.
#[derive(Debug, Default)]
pub struct ImportProgress {
    /// Compressed bytes read so far.
    pub read: AtomicU64,
    /// Size of the file.
    pub total: AtomicU64,
    /// Tiles converted so far.
    pub tiles: AtomicUsize,
    pub cancel: AtomicBool,
}

impl ImportProgress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            (self.read.load(Ordering::Relaxed) as f64 / total as f64).min(1.0) as f32
        }
    }
}

/// What was imported, and what could not be.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub name: String,
    /// WorldPainter version that saved the file.
    pub saved_by: Option<String>,
    /// Target platform of the world ("Minecraft 1.21.11").
    pub platform: Option<String>,
    /// One line per imported dimension.
    pub dimensions: Vec<String>,
    /// Dimensions Massif cannot represent yet.
    pub skipped_dimensions: Vec<String>,
    pub tiles: usize,
    /// Layers not imported, with the number of tiles using each.
    pub layers: Vec<(String, usize)>,
    /// Layers imported as Massif's (forests and trees as object layers,
    /// plants as ground covers, Frost), with the number of tiles using each.
    pub converted: Vec<(String, usize)>,
    /// WorldPainter terrains in the world, with their column counts.
    pub terrains: Vec<(String, u64)>,
    pub custom_terrains: usize,
    /// The JPEG thumbnail stored in the file, if any.
    pub thumbnail: Option<Vec<u8>>,
    /// The surface's overlay images, now reference images.
    pub overlays: usize,
    /// The files of those not found (by their path, or by their name beside
    /// the world): kept with their place, to find again.
    pub missing_images: Vec<String>,
    /// The ground covers made of the world's custom ground covers, by
    /// number.
    pub ground_covers: Vec<u16>,
}

impl ImportReport {
    /// A short human summary.
    pub fn summary(&self) -> String {
        let mut s = format!("{} tiles", self.tiles);
        if !self.dimensions.is_empty() {
            s += &format!(" in {}", self.dimensions.join(", "));
        }
        if !self.converted.is_empty() {
            let names: Vec<&str> = self.converted.iter().map(|l| l.0.as_str()).collect();
            s += &format!("; layers: {}", names.join(", "));
        }
        if !self.layers.is_empty() {
            let names: Vec<&str> = self.layers.iter().map(|l| l.0.as_str()).collect();
            s += &format!("; layers not imported yet: {}", names.join(", "));
        }
        if self.overlays > 0 {
            s += &format!("; {} overlay images", self.overlays);
        }
        if !self.missing_images.is_empty() {
            s += &format!(" ({} not found)", self.missing_images.len());
        }
        if !self.skipped_dimensions.is_empty() {
            s += &format!("; skipped: {}", self.skipped_dimensions.join(", "));
        }
        s
    }
}

/// Counts compressed bytes and honours cancellation.
struct Counting<'a, R> {
    inner: R,
    progress: &'a ImportProgress,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.progress.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("cancelled"));
        }
        let n = self.inner.read(buf)?;
        self.progress.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// The JPEG in the "WP" subfield of the gzip header's extra field.
pub fn read_thumbnail(head: &[u8]) -> Option<Vec<u8>> {
    if head.len() < 12 || head[0] != 0x1f || head[1] != 0x8b || head[3] & 0x04 == 0 {
        return None;
    }
    let xlen = u16::from_le_bytes([head[10], head[11]]) as usize;
    let extra = head.get(12..12 + xlen)?;
    let mut i = 0;
    while i + 4 <= extra.len() {
        let len = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        if extra[i] == b'W' && extra[i + 1] == b'P' {
            return extra.get(i + 4..i + 4 + len).map(<[u8]>::to_vec);
        }
        i += 4 + len;
    }
    None
}

/// Import a `.world` file.
pub fn import_world(path: &Path, progress: Option<&ImportProgress>) -> Result<(World, ImportReport), ImportError> {
    let local = ImportProgress::default();
    let progress = progress.unwrap_or(&local);
    let mut file = File::open(path)?;
    progress.total.store(file.metadata()?.len(), Ordering::Relaxed);
    let mut head = vec![0u8; 12 + 65_536];
    let mut n = 0;
    while n < head.len() {
        match file.read(&mut head[n..])? {
            0 => break,
            k => n += k,
        }
    }
    head.truncate(n);
    if head.len() < 2 || head[0] != 0x1f || head[1] != 0x8b {
        return Err(ImportError::NotWorldPainter);
    }
    let thumbnail = read_thumbnail(&head);
    let file = File::open(path)?;
    let counting = Counting { inner: file, progress };
    let gz = GzDecoder::new(BufReader::with_capacity(1 << 20, counting));
    // Inflating takes about as long as parsing what comes out: one thread
    // inflates ahead while this one parses.
    let mut result = std::thread::scope(|s| {
        let (send, receive) = std::sync::mpsc::sync_channel(INFLATED_AHEAD);
        let inflater = std::thread::Builder::new()
            .name("massif-inflate".into())
            .spawn_scoped(s, move || inflate(gz, &send))?;
        let result = import_stream(Inflated { chunks: receive, chunk: Vec::new(), at: 0 }, progress);
        // The parser stops early on a damaged stream: the inflater then
        // finds the channel closed and stops too.
        inflater.join().expect("the inflater does not panic");
        result
    });
    if let Ok((world, report)) = &mut result {
        report.thumbnail = thumbnail;
        // The overlays' files, where they are or beside the world.
        let dir = path.parent();
        for r in &mut world.meta.references {
            let name = r.path.rsplit(['/', '\\']).next().unwrap_or(&r.path).to_owned();
            let found = [Some(std::path::PathBuf::from(&r.path)), dir.filter(|_| !name.is_empty()).map(|d| d.join(&name))].into_iter().flatten().find(|p| p.is_file());
            match found.as_deref().and_then(|f| massif_core::reference::image_size(f).map(|size| (f, size))) {
                Some((file, size)) => {
                    r.path = file.to_string_lossy().into_owned();
                    r.set_size(size);
                }
                None => report.missing_images.push(r.path.clone()),
            }
        }
        if world.meta.name.is_empty() {
            world.meta.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Imported World".into());
        }
    }
    result
}

/// Inflated chunks waiting for the parser, 1 MiB each.
const INFLATED_AHEAD: usize = 8;
const INFLATED_CHUNK: usize = 1 << 20;

/// Inflate `gz` into chunks until it ends, fails or nobody listens.
fn inflate(mut gz: impl Read, send: &std::sync::mpsc::SyncSender<io::Result<Vec<u8>>>) {
    loop {
        let mut chunk = vec![0u8; INFLATED_CHUNK];
        let mut n = 0;
        let outcome = loop {
            match gz.read(&mut chunk[n..]) {
                Ok(0) => break Ok(true),
                Ok(k) => {
                    n += k;
                    if n == chunk.len() {
                        break Ok(false);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => break Err(e),
            }
        };
        chunk.truncate(n);
        let done = match outcome {
            Ok(end) => {
                let sent = n == 0 || send.send(Ok(chunk)).is_ok();
                end || !sent
            }
            Err(e) => {
                let _ = send.send(Err(e));
                true
            }
        };
        if done {
            return;
        }
    }
}

/// The inflated stream, read from the chunks the inflater sends.
struct Inflated {
    chunks: std::sync::mpsc::Receiver<io::Result<Vec<u8>>>,
    chunk: Vec<u8>,
    at: usize,
}

impl Read for Inflated {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at == self.chunk.len() {
            match self.chunks.recv() {
                Ok(chunk) => {
                    self.chunk = chunk?;
                    self.at = 0;
                }
                // The inflater is done: the end of the stream.
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.chunk.len() - self.at);
        buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// A tile's data, taken out of the arena, waiting to be converted and
/// encoded on a worker thread.
struct RawTile {
    handle: Handle,
    pos: TilePos,
    /// `minHeight` of the tile, as an offset.
    min: f32,
    heights: Heights,
    /// Terrain ordinals, and the terrain each ordinal maps to.
    terrain: Option<(Vec<u8>, [u16; 256])>,
    /// The terrain of a tile without terrain data.
    default_terrain: u16,
    water: Water,
    /// WorldPainter biome numbers, and the biome each maps to.
    biome: Option<(Vec<u8>, [u16; 256])>,
    /// Its layers that Massif has, Frost and Lava first.
    painted: Vec<Painted>,
}

enum Heights {
    Int(Vec<i32>),
    Short(Vec<i16>),
}

enum Water {
    Short(Vec<i16>),
    Byte(Vec<u8>),
    None,
}

impl RawTile {
    fn top(&self) -> Vec<f32> {
        let (min, mut top) = (self.min, Vec::with_capacity(TILE_AREA));
        match &self.heights {
            Heights::Int(raw) => top.extend(raw.iter().map(|&r| snap_height(r as f32 / 256.0 + min + 1.0))),
            Heights::Short(raw) => top.extend(raw.iter().map(|&r| snap_height((r as u16) as f32 / 256.0 + min + 1.0))),
        }
        top
    }

    fn fluid(&self) -> Vec<i16> {
        let min = self.min as i32;
        let level = |w: i32| (w + min + 1).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16;
        match &self.water {
            Water::Short(raw) => raw.iter().map(|&w| level((w as u16) as i32)).collect(),
            Water::Byte(raw) => raw.iter().map(|&w| level(w as i32)).collect(),
            Water::None => vec![NO_FLUID; TILE_AREA],
        }
    }
}

/// How many times each value appears in `raw`.
fn counts(raw: &[u8]) -> [u64; 256] {
    let mut counts = [0u64; 256];
    for &v in raw {
        counts[v as usize] += 1;
    }
    counts
}

fn take_array(arena: &mut Arena, v: Option<Value>) -> Option<Array> {
    match v {
        Some(Value::Ref(h)) => match arena.take(h) {
            Content::Array { data, .. } => Some(data),
            other => {
                // Not an array: put it back untouched.
                arena.items[h] = other;
                None
            }
        },
        _ => None,
    }
}

fn short_class(class: &str) -> String {
    class.rsplit(['.', '$']).next().unwrap_or(class).to_owned()
}

/// Called for every completed object: converts tiles and frees their data.
struct TileHook<'m> {
    mapper: &'m mut Mapper,
    layers: &'m mut BTreeMap<String, usize>,
    /// Layers converted, with the tiles using each.
    converted: &'m mut BTreeMap<String, usize>,
    /// What each layer (by its handle) becomes, once asked.
    converter: &'m mut crate::layers::Converter,
    histogram: [u64; 256],
    damaged: usize,
}

impl TileHook<'_> {
    fn extract(&mut self, arena: &mut Arena, h: Handle) -> Option<RawTile> {
        let v = Value::Ref(h);
        if arena.class_name(v) != Some(TILE_CLASS) {
            return None;
        }
        let int = |arena: &Arena, name: &str| arena.field(v, name).and_then(|f| arena.int(f));
        let x = int(arena, "x")? as i32;
        let z = int(arena, "y")? as i32;
        let min = int(arena, "minHeight").unwrap_or(0) as f32;
        let tall = arena.field(v, "tall").and_then(|f| arena.bool(f)).unwrap_or(false);

        let mut biome_bytes = None;
        let mut painted = Vec::new();
        for map_field in ["layerData", "bitLayerData"] {
            let Some(entries) = arena.field(v, map_field).and_then(|m| arena.map_entries(m)) else {
                continue;
            };
            for (key, value) in entries {
                let class = arena.class_name(key).unwrap_or("?").to_owned();
                if class == BIOME_LAYER && map_field == "layerData" {
                    if let Some(Array::Byte(b)) = take_array(arena, Some(value)) {
                        biome_bytes = Some(b);
                    }
                    continue;
                }
                let name = arena
                    .field(key, "name")
                    .and_then(|n| arena.string(n))
                    .map(str::to_owned)
                    .unwrap_or_else(|| short_class(&class));
                let parts = match key {
                    Value::Ref(hk) => self.converter.parts(arena, key, hk, self.mapper),
                    _ => Vec::new(),
                };
                let strength = if parts.is_empty() { None } else { crate::layers::strengths(arena, value) };
                match strength {
                    Some(strength) => {
                        // A combined layer's parts, each as strong as its
                        // factor says.
                        for (conv, factor) in parts {
                            let strength = if factor == 1.0 { strength.clone() } else { strength.iter().map(|&s| (s as f32 * factor).round().clamp(0.0, 255.0) as u8).collect() };
                            let priority = match conv {
                                Conv::Cover(n) if self.mapper.registry.cover(n).is_some_and(|c| c.changes_water()) => 0,
                                // Custom ground covers before plants: their
                                // blocks are where plants would grow.
                                Conv::Laid { .. } => 1,
                                _ => 2,
                            };
                            painted.push(Painted { conv, strength, priority });
                        }
                        *self.converted.entry(name).or_default() += 1;
                    }
                    None => *self.layers.entry(name).or_default() += 1,
                }
                // Free the layer's data (a byte array, or a BitSet and its words).
                let bits = arena.field(value, "bits");
                take_array(arena, bits);
                if let Value::Ref(hv) = value {
                    arena.take(hv);
                }
            }
        }

        let field = arena.field(v, if tall { "tallHeightMap" } else { "heightMap" });
        let heights = take_array(arena, field);
        let field = arena.field(v, "terrain");
        let terrain = take_array(arena, field);
        let field = arena.field(v, if tall { "tallWaterLevel" } else { "waterLevel" });
        let water = take_array(arena, field);

        let heights = match heights {
            Some(Array::Int(raw)) if raw.len() == TILE_AREA => Heights::Int(raw),
            Some(Array::Short(raw)) if raw.len() == TILE_AREA => Heights::Short(raw),
            _ => {
                self.damaged += 1;
                return None;
            }
        };
        let terrain = match terrain {
            Some(Array::Byte(raw)) if raw.len() == TILE_AREA => {
                let counts = counts(&raw);
                for (total, c) in self.histogram.iter_mut().zip(counts) {
                    *total += c;
                }
                let table = self.mapper.terrain_table(&raw, &counts);
                Some((raw, table))
            }
            _ => None,
        };
        let default_terrain = if terrain.is_none() { self.mapper.terrain(0) } else { 0 };
        let water = match water {
            Some(Array::Short(raw)) if raw.len() == TILE_AREA => Water::Short(raw),
            Some(Array::Byte(raw)) if raw.len() == TILE_AREA => Water::Byte(raw),
            _ => Water::None,
        };
        let biome = biome_bytes
            .filter(|b| b.len() == TILE_AREA)
            .map(|b| {
                let table = self.mapper.biome_table(&b, &counts(&b));
                (b, table)
            });
        Some(RawTile {
            handle: h,
            pos: TilePos::new(x, z),
            min,
            heights,
            terrain,
            default_terrain,
            water,
            biome,
            painted: {
                painted.sort_by_key(|p| p.priority);
                painted
            },
        })
    }
}

use crate::layers::After;

fn encode(raw: RawTile, beaches: &crate::terrains::Beaches) -> (Handle, TilePos, Blob, After) {
    let mut tile = Tile::ground(0.0, 0, NO_FLUID);
    let mut top = raw.top();
    let mut fluid = raw.fluid();
    // Void: nothing at all, down to the floor of the world.
    for p in raw.painted.iter().filter(|p| p.conv == Conv::Void) {
        for (i, _) in p.strength.iter().enumerate().filter(|s| *s.1 > 0) {
            top[i] = raw.min;
            fluid[i] = NO_FLUID;
        }
    }
    let mut terrain: Vec<u16> = match &raw.terrain {
        Some((ordinals, table)) => ordinals.iter().map(|&o| table[o as usize]).collect(),
        None => vec![raw.default_terrain; TILE_AREA],
    };
    if terrain.contains(&crate::terrains::BEACH_MARK) {
        let (ox, oz) = raw.pos.origin();
        for (i, t) in terrain.iter_mut().enumerate() {
            if *t == crate::terrains::BEACH_MARK {
                *t = beaches.at(ox + (i & 127) as i32, oz + (i >> 7) as i32, top[i]);
            }
        }
    }
    tile.fluid = Plane::from_vec(fluid);
    tile.top = Plane::from_vec(top);
    tile.terrain = Plane::from_vec(terrain);
    if let Some((numbers, table)) = &raw.biome {
        tile.biome = Plane::from_vec(numbers.iter().map(|&b| table[b as usize]).collect());
    }
    if !raw.painted.is_empty() {
        let (ox, oz) = raw.pos.origin();
        let (mut covers, mut objects, mut under) = (vec![0u16; TILE_AREA], vec![0u16; TILE_AREA], vec![0u16; TILE_AREA]);
        let (mut best, mut best_under) = (vec![0u32; TILE_AREA], vec![0u32; TILE_AREA]);
        for p in &raw.painted {
            for (i, &strength) in p.strength.iter().enumerate() {
                if strength == 0 {
                    continue;
                }
                let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
                match p.conv {
                    Conv::Cover(n) if covers[i] == 0 && crate::layers::kept(strength, Curve::Linear, x, z, n as u64) => covers[i] = n,
                    // A custom ground cover that takes the place of the top
                    // of the land: the ground goes down under its blocks.
                    Conv::Laid { cover, sink } if covers[i] == 0 => {
                        covers[i] = cover;
                        if sink > 0 {
                            tile.top.set(i, tile.top.get(i) - sink as f32);
                        }
                    }
                    // One base object layer a column, and one sub-canopy
                    // one: where several overlap, each of them in a share
                    // of the columns as large as its strength, so that all
                    // of them stand mixed (WorldPainter stands them all).
                    Conv::Objects(n, curve, under_canopy) if crate::layers::kept(strength, curve, x, z, 0x10000 | n as u64) => {
                        let (layer, total) = if under_canopy { (&mut under[i], &mut best_under[i]) } else { (&mut objects[i], &mut best[i]) };
                        *total += strength as u32;
                        if massif_core::compose::hash2(x, z, 0x6d69_7800 ^ n as u64) % *total < strength as u32 {
                            *layer = n;
                        }
                    }
                    _ => {}
                }
            }
        }
        tile.cover = Plane::from_vec(covers);
        tile.objects = Plane::from_vec(objects);
        tile.understory = Plane::from_vec(under);
        // Underground pockets, where painted at all.
        for p in &raw.painted {
            if let Conv::Pockets(n) = p.conv {
                let mut pockets = tile.pockets.dense().to_vec();
                for (i, &s) in p.strength.iter().enumerate() {
                    if s > 0 {
                        pockets[i] = n;
                    }
                }
                tile.pockets = Plane::from_vec(pockets);
            }
        }
        if let Some(p) = raw.painted.iter().find(|p| p.conv == Conv::Annotations) {
            // Strengths are nibbles × 17: back to the colour.
            tile.annotations = Plane::from_vec(p.strength.iter().map(|&s| (s / 17) as u16).collect());
        }
        // A combined layer's terrain and biome, where it is painted at half
        // strength or more, and at random a share of the rest.
        for p in &raw.painted {
            let (Conv::Terrain(_) | Conv::Biome(_)) = p.conv else { continue };
            for (i, &strength) in p.strength.iter().enumerate() {
                let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
                if strength == 0 || !crate::layers::kept(strength.saturating_mul(2), Curve::Linear, x, z, 0x7465_7272) {
                    continue;
                }
                match p.conv {
                    Conv::Terrain(t) => tile.terrain.set(i, t),
                    Conv::Biome(b) => tile.biome.set(i, b),
                    _ => {}
                }
            }
        }
    }
    tile.compact();
    let after = raw.painted.into_iter().filter(|p| p.conv.after()).map(|p| (p.conv, p.strength)).collect();
    (raw.handle, raw.pos, Blob::from(tile.encode()), after)
}

/// A dimension's overlay images (WorldPainter's View ▸ Overlays) as
/// reference images: at the same place (their top left corner, in blocks),
/// scale and opacity, shown if they were; placed by their corner until
/// their size is read. Before WorldPainter 2.10 a dimension held one, in
/// fields of its own.
fn overlays(arena: &Arena, dimension: Value) -> Vec<massif_core::reference::ReferenceImage> {
    use massif_core::reference::ReferenceImage;
    let all_on = arena.field(dimension, "overlayEnabled").and_then(|v| arena.bool(v)).unwrap_or(true);
    let path = |file: Option<Value>| file.filter(|f| *f != Value::Null).and_then(|f| arena.field(f, "path")).and_then(|p| arena.string(p)).map(str::to_owned);
    let image = |path: String, x: i64, z: i64, scale: f64, transparency: f64, shown: bool| {
        let mut r = ReferenceImage::at_corner(&path, x as f64, z as f64, if scale > 0.0 { scale as f32 } else { 1.0 }, None);
        r.opacity = (1.0 - if transparency > 0.0 { transparency } else { 0.5 } as f32).clamp(0.05, 1.0);
        r.shown = shown && all_on;
        r
    };
    let mut out = Vec::new();
    if let Some(items) = arena.field(dimension, "overlays").filter(|l| *l != Value::Null).and_then(|l| arena.list_items(l)) {
        for o in items {
            let Some(file) = path(arena.field(o, "file")) else { continue };
            let int = |n: &str| arena.field(o, n).and_then(|v| arena.int(v)).unwrap_or(0);
            let float = |n: &str, default: f64| arena.field(o, n).and_then(|v| arena.float(v)).unwrap_or(default);
            let enabled = arena.field(o, "enabled").and_then(|v| arena.bool(v)).unwrap_or(true);
            out.push(image(file, int("offsetX"), int("offsetY"), float("scale", 1.0), float("transparency", 0.5), enabled));
        }
    } else if let Some(file) = path(arena.field(dimension, "overlay")) {
        let int = |n: &str| arena.field(dimension, n).and_then(|v| arena.int(v)).unwrap_or(0);
        let float = |n: &str| arena.field(dimension, n).and_then(|v| arena.float(v)).unwrap_or(0.0);
        out.push(image(file, int("overlayOffsetX"), int("overlayOffsetY"), float("overlayScale"), float("overlayTransparency"), true));
    }
    out
}

/// The palettes WorldPainter hid (`Dimension.hiddenPalettes`) and the one it
/// showed alone (`soloedPalette`): their layers are hidden on the map as
/// they were, every other layer too while a palette is soloed.
fn palette_visibility(arena: &Arena, dimension: Value, reg: &mut Registry) {
    let hidden: Vec<String> = arena
        .field(dimension, "hiddenPalettes")
        .filter(|v| *v != Value::Null)
        .and_then(|v| arena.list_items(v))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| arena.string(n).map(|n| n.trim().to_owned()))
        .collect();
    let solo = arena.field(dimension, "soloedPalette").and_then(|v| arena.string(v)).map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned);
    let hide = |palette: &str| solo.as_ref().is_some_and(|s| s != palette) || (!palette.is_empty() && hidden.iter().any(|h| h == palette));
    for c in &mut reg.covers {
        c.hidden |= hide(&c.palette);
    }
    for l in &mut reg.object_layers {
        l.hidden |= hide(&l.palette);
    }
}

fn damaged(e: JavaError, progress: &ImportProgress) -> ImportError {
    if progress.cancel.load(Ordering::Relaxed) {
        return ImportError::Cancelled;
    }
    match e {
        JavaError::NotJava => ImportError::NotWorldPainter,
        e => ImportError::Damaged(e),
    }
}

/// A WorldPainter `.terrain` file (a custom terrain, a `MixedMaterial`
/// Java-serialised and gzipped, as its Export writes it): its name, its
/// blocks with their counts and its colour.
pub fn read_terrain_file(bytes: &[u8]) -> Result<CustomMaterial, String> {
    let mut raw = Vec::new();
    let data: &[u8] = if bytes.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(bytes).take(16 << 20).read_to_end(&mut raw).map_err(|_| "not a WorldPainter custom terrain file".to_owned())?;
        &raw
    } else {
        bytes
    };
    let mut parser = Parser::new(data).map_err(|_| "not a WorldPainter custom terrain file".to_owned())?;
    let value = match parser.next_object() {
        Ok(Some(v @ Value::Ref(_))) => v,
        Ok(_) => return Err("not a WorldPainter custom terrain file".into()),
        Err(e) => return Err(format!("a WorldPainter custom terrain file that could not be read: {e:?}")),
    };
    read_mixed(&parser.arena, value).filter(|m| !m.rows.is_empty()).ok_or_else(|| "not a WorldPainter custom terrain file".to_owned())
}

pub(crate) fn read_mixed(a: &Arena, v: Value) -> Option<CustomMaterial> {
    if !a.class_name(v)?.ends_with("MixedMaterial") {
        return None;
    }
    let name = a.field(v, "name").and_then(|n| a.string(n)).unwrap_or("").to_owned();
    let rows = a
        .list_items(a.field(v, "rows")?)?
        .into_iter()
        .filter_map(|row| {
            let material = a.field(row, "material")?;
            let count = a.int(a.field(row, "occurrence")?)?;
            Some((material_state(a, material), count.max(1) as u32))
        })
        .collect();
    let colour = a
        .field(v, "colour")
        .and_then(|c| a.int(c))
        .map(|argb| [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]);
    Some(CustomMaterial { name, rows, colour })
}

/// `namespace:name[key=value,...]` of a serialized `org.pepsoft.minecraft.Material`.
pub(crate) fn material_state(a: &Arena, material: Value) -> String {
    let identity = a.field(material, "identity").filter(|i| *i != Value::Null);
    let Some(identity) = identity else {
        // A pre-1.13 material known only by its numeric id and data.
        let int = |n: &str| a.field(material, n).and_then(|v| a.int(v));
        return int("blockType").and_then(|id| massif_objects::legacy::state(u16::try_from(id).ok()?, int("data").unwrap_or(0).clamp(0, 15) as u8)).unwrap_or_else(|| "minecraft:stone".into());
    };
    let name = a
        .field(identity, "name")
        .and_then(|n| a.string(n))
        .unwrap_or("minecraft:stone")
        .to_owned();
    let mut props: Vec<String> = a
        .field(identity, "properties")
        .and_then(|p| a.map_entries(p))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some(format!("{}={}", a.string(k)?, a.string(v)?)))
        .filter(|kv| !kv.starts_with("data_value="))
        .collect();
    props.sort();
    if props.is_empty() {
        name
    } else {
        format!("{name}[{}]", props.join(","))
    }
}

struct Found {
    dim: i32,
    role: String,
    invert: bool,
    dimension: Value,
}

/// Import from a decompressed Java serialization stream.
pub fn import_stream<R: Read + Send>(reader: R, progress: &ImportProgress) -> Result<(World, ImportReport), ImportError> {
    let mut mapper = Mapper::new(Registry::vanilla());
    let mut layers = BTreeMap::new();
    let mut converted_layers = BTreeMap::new();
    let mut converter = crate::layers::Converter::default();
    converter.lay_covers = true;
    let converted: Mutex<HashMap<Handle, (TilePos, Blob, After)>> = Mutex::new(HashMap::new());
    let mut histogram = [0u64; 256];
    let mut damaged_tiles = 0;
    let beaches = crate::terrains::Beaches::new(&mapper.registry);

    // In place: the parse runs on this thread (whose stack the caller
    // chose) while tiles are encoded on the pool.
    let (tops, arena) = rayon::in_place_scope(|s| -> Result<(Vec<Value>, Arena), ImportError> {
        let mut parser = Parser::new(reader).map_err(|e| damaged(e, progress))?;
        let mut hook = TileHook {
            mapper: &mut mapper,
            layers: &mut layers,
            converted: &mut converted_layers,
            converter: &mut converter,
            histogram: [0; 256],
            damaged: 0,
        };
        let converted = &converted;
        let beaches = &beaches;
        let result = {
            let hook_ref = &mut hook;
            parser.set_hook(Box::new(move |arena: &mut Arena, h: Handle| {
                if let Some(raw) = hook_ref.extract(arena, h) {
                    s.spawn(move |_| {
                        let (handle, pos, blob, after) = encode(raw, beaches);
                        converted.lock().expect("no panics while holding the lock").insert(handle, (pos, blob, after));
                        progress.tiles.fetch_add(1, Ordering::Relaxed);
                    });
                }
            }));
            let mut tops = Vec::new();
            let outcome = loop {
                match parser.next_object() {
                    Ok(Some(v)) => tops.push(v),
                    Ok(None) => break Ok(()),
                    Err(e) => break Err(damaged(e, progress)),
                }
            };
            let arena = std::mem::take(&mut parser.arena);
            drop(parser);
            outcome.map(|()| (tops, arena))
        };
        histogram = hook.histogram;
        damaged_tiles = hook.damaged;
        result
    })?;
    let converted = converted.into_inner().expect("workers finished");

    // ---- the world --------------------------------------------------------
    let world2 = tops.iter().copied().find(|v| arena.class_name(*v) == Some(WORLD_CLASS));
    let Some(world2) = world2 else {
        if tops.iter().any(|v| arena.class_name(*v) == Some(LEGACY_WORLD_CLASS)) {
            return Err(ImportError::Unsupported(
                "this world was saved by WorldPainter 0.1; open it in a recent WorldPainter and save it again first".into(),
            ));
        }
        return Err(ImportError::NotWorldPainter);
    };
    let mut report = ImportReport::default();
    if let Some(entries) = tops.first().and_then(|m| arena.map_entries(*m)) {
        for (k, v) in entries {
            if arena.string(k) == Some("org.pepsoft.worldpainter.wp.version") {
                report.saved_by = arena.string(v).map(|s| format!("WorldPainter {s}"));
            }
        }
    }
    report.platform = arena
        .field(world2, "platform")
        .and_then(|p| arena.field(p, "displayName"))
        .and_then(|n| arena.string(n))
        .map(str::to_owned);
    let name = arena.field(world2, "name").and_then(|n| arena.string(n)).unwrap_or("").to_owned();
    report.name = name.clone();

    // Custom terrains, now that their definitions are known.
    let materials: Vec<Option<CustomMaterial>> = arena
        .field(world2, "mixedMaterials")
        .and_then(|m| arena.list_items(m))
        .unwrap_or_default()
        .into_iter()
        .map(|m| read_mixed(&arena, m))
        .collect();
    report.custom_terrains = mapper.finish_custom(&materials);

    // ---- dimensions ---------------------------------------------------------
    let mut found = Vec::new();
    if let Some(entries) = arena.field(world2, "dimensionsByAnchor").and_then(|m| arena.map_entries(m)) {
        for (anchor, dimension) in entries {
            found.push(Found {
                dim: arena.field(anchor, "dim").and_then(|d| arena.int(d)).unwrap_or(0) as i32,
                role: arena.field(anchor, "role").and_then(|r| arena.enum_name(r)).unwrap_or("DETAIL").to_owned(),
                invert: arena.field(anchor, "invert").and_then(|i| arena.bool(i)).unwrap_or(false),
                dimension,
            });
        }
    } else if let Some(entries) = arena.field(world2, "dimensions").and_then(|m| arena.map_entries(m)) {
        for (key, dimension) in entries {
            let dim = arena.int(key).unwrap_or(0) as i32;
            found.push(Found {
                dim: dim.max(0),
                role: "DETAIL".into(),
                invert: dim < 0,
                dimension,
            });
        }
    }
    found.sort_by_key(|f| (f.dim, f.invert, f.role.clone()));

    let world_min = arena.field(world2, "minHeight").and_then(|v| arena.int(v)).unwrap_or(0) as i32;
    let world_max = arena.field(world2, "maxheight").and_then(|v| arena.int(v)).unwrap_or(256) as i32;
    let mut dims = Vec::new();
    let mut overworld: Option<Value> = None;
    let mut references = Vec::new();
    for f in &found {
        let label = match f.dim {
            0 => "Surface",
            1 => "Nether",
            2 => "End",
            _ => "Dimension",
        };
        let kind = match (f.dim, f.role.as_str(), f.invert) {
            (0, "DETAIL", false) => Some(DimensionKind::Overworld),
            (1, "DETAIL", false) => Some(DimensionKind::Nether),
            (2, "DETAIL", false) => Some(DimensionKind::End),
            _ => None,
        };
        let tiles = arena
            .field(f.dimension, "tiles")
            .and_then(|t| arena.map_entries(t))
            .unwrap_or_default();
        let Some(kind) = kind else {
            let what = match (f.role.as_str(), f.invert) {
                (_, true) => format!("{label} ceiling"),
                ("CAVE_FLOOR", _) => format!("{label} cave floor"),
                ("FLOATING_FLOOR", _) => format!("{label} floating floor"),
                ("MASTER", _) => format!("{label} master (low resolution)"),
                (role, _) => format!("{label} {}", role.to_lowercase()),
            };
            report.skipped_dimensions.push(format!("{what} ({} tiles)", tiles.len()));
            continue;
        };
        let int = |name: &str| arena.field(f.dimension, name).and_then(|v| arena.int(v));
        let mut min_y = int("minHeight").unwrap_or(world_min as i64) as i32;
        let mut max_y = int("maxHeight").unwrap_or(world_max as i64) as i32;
        if max_y <= min_y {
            (min_y, max_y) = (world_min, world_max.max(world_min + 16));
        }
        let mut dim = Dimension::new(kind, min_y, max_y);
        let ground = dim.ground_mut();
        let mut count = 0;
        let mut after = Vec::new();
        for (_, tile) in tiles {
            let Value::Ref(h) = tile else { continue };
            if let Some((pos, blob, later)) = converted.get(&h) {
                ground.insert_blob(*pos, blob.clone());
                count += 1;
                if !later.is_empty() {
                    after.push((*pos, later.clone()));
                }
            }
        }
        ground.take_dirty();
        if !after.is_empty() {
            // Ground covers, tunnels and caves, now that the dimension is
            // whole.
            let sea = int("borderLevel").map_or(63, |b| b as i32 + 1);
            let seed = arena
                .field(f.dimension, "generator")
                .and_then(|g| arena.field(g, "seed"))
                .and_then(|s| arena.int(s))
                .or_else(|| int("minecraftSeed"))
                .unwrap_or(0);
            crate::layers::apply_after(&mut dim, &after, &converter, &mut mapper.registry, seed, sea);
        }
        report.tiles += count;
        report
            .dimensions
            .push(format!("{} ({count} tiles, Y {min_y} to {})", kind.label(), max_y - 1));
        if kind == DimensionKind::Overworld {
            overworld = Some(f.dimension);
            references = overlays(&arena, f.dimension);
        }
        // Every tile stored as a file holds it, none decoded: the map as
        // light and quick as when it is opened again (a map with layers
        // applied after its tiles kept those tiles decoded, hundreds of
        // kilobytes each, gigabytes on a large map).
        for level in &mut dim.levels {
            level.freeze();
        }
        dim.trim(0);
        dims.push(dim);
    }
    if dims.is_empty() {
        return Err(ImportError::Unsupported("the world has no surface, Nether or End to import".into()));
    }
    // Massif edits the first dimension: put the surface first.
    dims.sort_by_key(|d| match d.kind {
        DimensionKind::Overworld => 0,
        DimensionKind::Nether => 1,
        DimensionKind::End => 2,
    });

    // ---- settings -----------------------------------------------------------
    let main = overworld.unwrap_or(found[0].dimension);
    let seed = arena
        .field(main, "generator")
        .and_then(|g| arena.field(g, "seed"))
        .and_then(|s| arena.int(s))
        .or_else(|| arena.field(main, "minecraftSeed").and_then(|s| arena.int(s)))
        .unwrap_or(0);
    let sea_level = arena
        .field(main, "borderLevel")
        .and_then(|b| arena.int(b))
        .map_or(63, |b| b as i32 + 1);
    let (sx, sz) = arena
        .field(world2, "spawnPoint")
        .map(|p| {
            let c = |n| arena.field(p, n).and_then(|v| arena.int(v)).unwrap_or(0) as i32;
            (c("x"), c("y"))
        })
        .unwrap_or((0, 0));
    let spawn_y = massif_core::ops::height_at(&dims[0], 0, sx, sz, massif_core::ops::Surface::Top)
        .map_or(sea_level + 1, block_top);

    palette_visibility(&arena, main, &mut mapper.registry);
    report.overlays = references.len();
    report.ground_covers = converter.laid.clone();
    report.layers = layers.into_iter().collect();
    report.layers.sort_by_key(|l| std::cmp::Reverse(l.1));
    report.converted = converted_layers.into_iter().collect();
    report.converted.sort_by_key(|l| std::cmp::Reverse(l.1));
    report.terrains = histogram
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(o, c)| {
            let name = crate::tables::TERRAINS.get(o).copied().unwrap_or("UNKNOWN");
            let pretty = match name.strip_prefix("CUSTOM_").and_then(|n| n.parse::<usize>().ok()) {
                Some(n) => materials
                    .get(n - 1)
                    .and_then(|m| m.as_ref())
                    .map_or(format!("Custom {n}"), |m| m.name.clone()),
                None => name.to_owned(),
            };
            (pretty, *c)
        })
        .collect();
    report.terrains.sort_by_key(|t| std::cmp::Reverse(t.1));
    if damaged_tiles > 0 {
        report.skipped_dimensions.push(format!("{damaged_tiles} damaged tiles"));
    }

    let world = World {
        meta: WorldMeta {
            name,
            seed,
            spawn: [sx, spawn_y, sz],
            sea_level,
            lava: false,
            graph: None,
            ores: Default::default(),
            gradients: Vec::new(),
            fill: None,
            paint_rules: None,
            palette: Default::default(),
            smooth: Default::default(),
            references,
            ..Default::default()
        },
        registry: mapper.registry,
        dims,
        points: Vec::new(),
    };
    debug_assert!(world.dims[0].levels[0].kind == LevelKind::Ground);
    Ok((world, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::javaser::tests::StreamWriter;
    use crate::javaser::*;
    use massif_core::TerrainId;
    use massif_core::coords::local_index;

    /// A tiny but complete World2 stream: one surface dimension with two
    /// tiles (one "tall", one legacy short), a biome layer, a tree layer and
    /// a custom terrain.
    fn synthetic_world() -> Vec<u8> {
        synthetic_world_with(320, 99.5)
    }

    /// [`synthetic_world`] with the world `max` high, and the tall tile's
    /// column 5 at `column`.
    fn synthetic_world_with(max: i32, column: f32) -> Vec<u8> {
        let mut w = StreamWriter::new();
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        // World2 {I minHeight, I maxheight, L dimensionsByAnchor, L mixedMaterials, L name, L spawnPoint}
        w.u8(TC_OBJECT).class(
            WORLD_CLASS,
            SC_SERIALIZABLE,
            &[
                (b'I', "maxheight", None),
                (b'I', "minHeight", None),
                (b'L', "dimensionsByAnchor", Some("Ljava/util/Map;")),
                (b'[', "mixedMaterials", Some("[Lorg/pepsoft/worldpainter/MixedMaterial;")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'L', "spawnPoint", Some("Ljava/awt/Point;")),
            ],
            null,
        );
        w.handle();
        w.i32(max).i32(-64);

        // dimensionsByAnchor: HashMap with one entry.
        let hashmap_desc = w.next;
        w.u8(TC_OBJECT).class(
            "java.util.HashMap",
            SC_SERIALIZABLE | SC_WRITE_METHOD,
            &[(b'F', "loadFactor", None), (b'I', "threshold", None)],
            null,
        );
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(1);
        // Key: Anchor {I dim, I id, Z invert, L role}
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.Dimension$Anchor",
            SC_SERIALIZABLE,
            &[
                (b'I', "dim", None),
                (b'I', "id", None),
                (b'Z', "invert", None),
                (b'L', "role", Some("Lorg/pepsoft/worldpainter/Dimension$Role;")),
            ],
            null,
        );
        w.handle();
        w.i32(0).i32(0).u8(0);
        w.u8(TC_ENUM).class("org.pepsoft.worldpainter.Dimension$Role", SC_SERIALIZABLE | 0x10, &[], |w| {
            w.class("java.lang.Enum", SC_SERIALIZABLE | 0x10, &[], |w| {
                w.u8(TC_NULL);
            });
        });
        w.handle();
        w.string("DETAIL");
        // Value: Dimension {I borderLevel, I maxHeight, I minHeight, L generator, L tiles}
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.Dimension",
            SC_SERIALIZABLE,
            &[
                (b'I', "borderLevel", None),
                (b'I', "maxHeight", None),
                (b'I', "minHeight", None),
                (b'L', "generator", Some("Lorg/pepsoft/worldpainter/MapGenerator;")),
                (b'L', "tiles", Some("Ljava/util/Map;")),
            ],
            null,
        );
        w.handle();
        w.i32(62).i32(max).i32(-64);
        // generator: SeededGenerator {J seed}
        w.u8(TC_OBJECT).class("org.pepsoft.minecraft.SeededGenerator", SC_SERIALIZABLE, &[(b'J', "seed", None)], null);
        w.handle();
        w.i64(27594263);
        // tiles: HashMap { Point(0,0) -> tall tile, Point(-1,0) -> short tile }
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(2);

        let point_desc = w.next;
        w.u8(TC_OBJECT).class("java.awt.Point", SC_SERIALIZABLE, &[(b'I', "x", None), (b'I', "y", None)], null);
        w.handle();
        w.i32(0).i32(0);

        let tile_desc = w.next;
        let tile_fields: [(u8, &str, Option<&str>); 11] = [
            (b'I', "maxHeight", None),
            (b'I', "minHeight", None),
            (b'Z', "tall", None),
            (b'I', "x", None),
            (b'I', "y", None),
            (b'L', "bitLayerData", Some("Ljava/util/Map;")),
            (b'[', "heightMap", Some("[S")),
            (b'L', "layerData", Some("Ljava/util/Map;")),
            (b'[', "tallHeightMap", Some("[I")),
            (b'[', "tallWaterLevel", Some("[S")),
            (b'[', "terrain", Some("[B")),
        ];
        // The legacy tile also has "waterLevel" ([B): use a second class
        // layout for it, as an older WorldPainter would have written.
        w.u8(TC_OBJECT).class(TILE_CLASS, SC_SERIALIZABLE, &tile_fields, null);
        w.handle();
        w.i32(max).i32(-64).u8(1).i32(0).i32(0);
        // bitLayerData: HashMap { Frost -> row 1, PlantLayer "Flowers" -> row
        // 2, FloodWithLava -> row 5, Void -> row 6 }
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(4);
        w.u8(TC_OBJECT).class("org.pepsoft.worldpainter.layers.Frost", SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
        w.handle();
        w.string("Frost");
        let bitset = |w: &mut StreamWriter, row: usize, desc: &mut Option<(i32, i32)>| {
            match *desc {
                None => {
                    let set = w.next;
                    w.u8(TC_OBJECT).class("java.util.BitSet", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'[', "bits", Some("[J"))], null);
                    w.handle();
                    let longs = w.next;
                    w.u8(TC_ARRAY).class("[J", SC_SERIALIZABLE, &[], null);
                    *desc = Some((set, longs));
                }
                Some((set, longs)) => {
                    w.u8(TC_OBJECT).reference(set);
                    w.handle();
                    w.u8(TC_ARRAY).reference(longs);
                }
            }
            w.handle();
            w.i32(256);
            for k in 0..256 {
                w.i64(if k / 2 == row { -1 } else { 0 });
            }
            w.u8(TC_ENDBLOCKDATA);
        };
        let mut bitset_desc = None;
        bitset(&mut w, 1, &mut bitset_desc);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.plants.PlantLayer",
            SC_SERIALIZABLE,
            &[
                (b'I', "colour", None),
                (b'Z', "onlyOnValidBlocks", None),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'[', "settings", Some("[Lorg/pepsoft/worldpainter/layers/plants/PlantLayer$PlantSettings;")),
            ],
            null,
        );
        w.handle();
        w.i32(0xd6a8ba).u8(1);
        w.string("Flowers");
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.worldpainter.layers.plants.PlantLayer$PlantSettings;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(95);
        let mut settings_desc = None;
        for k in 0..95 {
            // Short grass ×30 and dandelions ×10; kelp, a water plant, ×5.
            let occurrence = match k {
                0 => 30,
                5 => 10,
                48 => 5,
                _ => {
                    w.u8(TC_NULL);
                    continue;
                }
            };
            match settings_desc {
                None => {
                    settings_desc = Some(w.next);
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.worldpainter.layers.plants.PlantLayer$PlantSettings",
                        SC_SERIALIZABLE,
                        &[(b'I', "growthFrom", None), (b'I', "growthTo", None), (b'S', "occurrence", None)],
                        null,
                    );
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.i32(1).i32(1).u16(occurrence);
        }
        bitset(&mut w, 2, &mut bitset_desc);
        for (class, name, row) in [("FloodWithLava", "Lava", 5), ("Void", "Void", 6)] {
            w.u8(TC_OBJECT).class(&format!("org.pepsoft.worldpainter.layers.{class}"), SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
            w.handle();
            w.string(name);
            bitset(&mut w, row, &mut bitset_desc);
        }
        w.u8(TC_ENDBLOCKDATA);
        w.u8(TC_NULL); // heightMap
        // layerData: HashMap { Biome -> byte[16384] of 4 (jungle), Tall
        // spruces -> nibbles, 15 on row 0, Deciduous -> 8 from row 3,
        // Annotations -> 14 (red) on row 4 }
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(4);
        w.u8(TC_OBJECT).class(BIOME_LAYER, SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
        w.handle();
        w.string("Biome");
        let byte_array_desc = w.next;
        w.u8(TC_ARRAY).class("[B", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for i in 0..TILE_AREA {
            w.u8(if i < 128 { 4 } else { 255 });
        }
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.Bo2Layer",
            SC_SERIALIZABLE,
            &[(b'L', "name", Some("Ljava/lang/String;"))],
            null,
        );
        w.handle();
        w.string("Tall spruces");
        w.u8(TC_ARRAY).reference(byte_array_desc);
        w.handle();
        w.i32(8192);
        w.out.extend((0..8192).map(|i| if i < 64 { 0xff } else { 0 }));
        w.u8(TC_OBJECT).class("org.pepsoft.worldpainter.layers.DeciduousForest", SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
        w.handle();
        w.string("Deciduous");
        w.u8(TC_ARRAY).reference(byte_array_desc);
        w.handle();
        w.i32(8192);
        w.out.extend((0..8192).map(|i| if i >= 192 { 0x88 } else { 0 }));
        w.u8(TC_OBJECT).class("org.pepsoft.worldpainter.layers.Annotations", SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
        w.handle();
        w.string("Annotations");
        w.u8(TC_ARRAY).reference(byte_array_desc);
        w.handle();
        w.i32(8192);
        w.out.extend((0..8192).map(|i| if (256..320).contains(&i) { 0xee } else { 0 }));
        w.u8(TC_ENDBLOCKDATA);
        // tallHeightMap: 70.0 everywhere except column 5 at 99.5 (raw = (h + 64) * 256)
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for i in 0..TILE_AREA {
            let h = if i == 5 { column } else { 70.0 };
            w.i32(((h + 64.0) * 256.0) as i32);
        }
        // tallWaterLevel: 62 everywhere (raw = 62 + 64)
        let short_array_desc = w.next;
        w.u8(TC_ARRAY).class("[S", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for _ in 0..TILE_AREA {
            w.u16(126);
        }
        // terrain: SAND (2) on the first row, CUSTOM_1 (22) elsewhere.
        w.u8(TC_ARRAY).reference(byte_array_desc);
        w.handle();
        w.i32(TILE_AREA as i32);
        for i in 0..TILE_AREA {
            w.u8(if i < 128 { 2 } else { 22 });
        }

        // Second entry: Point(-1, 0) -> non-tall tile with heightMap.
        w.u8(TC_OBJECT).reference(point_desc);
        w.handle();
        w.i32(-1).i32(0);
        w.u8(TC_OBJECT).reference(tile_desc);
        w.handle();
        w.i32(320).i32(-64).u8(0).i32(-1).i32(0);
        w.u8(TC_NULL); // bitLayerData
        // heightMap: 40.0 everywhere → raw (40 + 64) * 256
        w.u8(TC_ARRAY).reference(short_array_desc);
        w.handle();
        w.i32(TILE_AREA as i32);
        for _ in 0..TILE_AREA {
            w.u16(((40.0f32 + 64.0) * 256.0) as u16);
        }
        w.u8(TC_NULL); // layerData
        w.u8(TC_NULL); // tallHeightMap
        w.u8(TC_NULL); // tallWaterLevel
        w.u8(TC_ARRAY).reference(byte_array_desc);
        w.handle();
        w.i32(TILE_AREA as i32);
        w.out.extend(std::iter::repeat_n(0u8, TILE_AREA)); // GRASS
        w.u8(TC_ENDBLOCKDATA); // end of tiles HashMap
        w.u8(TC_ENDBLOCKDATA); // end of dimensionsByAnchor HashMap

        // mixedMaterials: [MixedMaterial("Volcanic", rows: [Row(material: Material(identity: Identity("minecraft:basalt", {axis=y})), occurrence 1000)])]
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.worldpainter.MixedMaterial;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.MixedMaterial",
            SC_SERIALIZABLE,
            &[
                (b'L', "colour", Some("Ljava/lang/Integer;")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'[', "rows", Some("[Lorg/pepsoft/worldpainter/MixedMaterial$Row;")),
            ],
            null,
        );
        w.handle();
        w.u8(TC_NULL); // colour
        w.string("Volcanic");
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.worldpainter.MixedMaterial$Row;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.MixedMaterial$Row",
            SC_SERIALIZABLE,
            &[(b'I', "occurrence", None), (b'L', "material", Some("Lorg/pepsoft/minecraft/Material;"))],
            null,
        );
        w.handle();
        w.i32(1000);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material",
            SC_SERIALIZABLE,
            &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
            null,
        );
        w.handle();
        w.i32(-1).i32(-1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material$Identity",
            SC_SERIALIZABLE,
            &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
            null,
        );
        w.handle();
        w.string("minecraft:basalt");
        w.u8(TC_OBJECT).class(
            "java.util.Collections$SingletonMap",
            SC_SERIALIZABLE,
            &[(b'L', "k", Some("Ljava/lang/Object;")), (b'L', "v", Some("Ljava/lang/Object;"))],
            null,
        );
        w.handle();
        w.string("axis").string("y");

        w.string("Synthetic");
        w.u8(TC_OBJECT).reference(point_desc);
        w.handle();
        w.i32(3).i32(2);
        w.out
    }

    /// A World2 stream of one tall tile at (0, 0), 70 high, its rows of
    /// three terrains (grass, sand, stone, a third each), painted with
    /// custom ground covers, each `(name, thickness, block, rows)` set on
    /// the rows `rows` of the tile.
    fn layered_world(covers: &[(&str, i32, &str, std::ops::Range<usize>)]) -> Vec<u8> {
        layered_world_with(covers, &[])
    }

    /// [`layered_world`] with overlay images on the surface, each `(path,
    /// offset x, offset z, scale, transparency, enabled)`.
    fn layered_world_with(covers: &[(&str, i32, &str, std::ops::Range<usize>)], overlays: &[(&str, i32, i32, f32, f32, bool)]) -> Vec<u8> {
        let mut w = StreamWriter::new();
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        w.u8(TC_OBJECT).class(
            WORLD_CLASS,
            SC_SERIALIZABLE,
            &[(b'I', "maxheight", None), (b'I', "minHeight", None), (b'L', "dimensionsByAnchor", Some("Ljava/util/Map;")), (b'L', "name", Some("Ljava/lang/String;"))],
            null,
        );
        w.handle();
        w.i32(320).i32(-64);
        let hashmap_desc = w.next;
        w.u8(TC_OBJECT).class("java.util.HashMap", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'F', "loadFactor", None), (b'I', "threshold", None)], null);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.Dimension$Anchor",
            SC_SERIALIZABLE,
            &[(b'I', "dim", None), (b'I', "id", None), (b'Z', "invert", None), (b'L', "role", Some("Lorg/pepsoft/worldpainter/Dimension$Role;"))],
            null,
        );
        w.handle();
        w.i32(0).i32(0).u8(0);
        w.u8(TC_ENUM).class("org.pepsoft.worldpainter.Dimension$Role", SC_SERIALIZABLE | 0x10, &[], |w| {
            w.class("java.lang.Enum", SC_SERIALIZABLE | 0x10, &[], |w| {
                w.u8(TC_NULL);
            });
        });
        w.handle();
        w.string("DETAIL");
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.Dimension",
            SC_SERIALIZABLE,
            &[(b'I', "borderLevel", None), (b'I', "maxHeight", None), (b'I', "minHeight", None), (b'Z', "overlayEnabled", None), (b'L', "overlays", Some("Ljava/util/List;")), (b'L', "tiles", Some("Ljava/util/Map;"))],
            null,
        );
        w.handle();
        w.i32(62).i32(320).i32(-64).u8(1);
        // overlays: ArrayList of Overlay { Z enabled, I offsetX, I offsetY,
        // F scale, F transparency, L file (java.io.File { L path }) }
        w.u8(TC_OBJECT).class("java.util.ArrayList", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'I', "size", None)], null);
        w.handle();
        w.i32(overlays.len() as i32).u8(TC_BLOCKDATA).u8(4).i32(overlays.len() as i32);
        let (mut overlay_desc, mut file_desc) = (None, None);
        for &(path, x, z, scale, transparency, enabled) in overlays {
            match overlay_desc {
                None => {
                    overlay_desc = Some(w.next);
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.worldpainter.Overlay",
                        SC_SERIALIZABLE,
                        &[(b'Z', "enabled", None), (b'I', "offsetX", None), (b'I', "offsetY", None), (b'F', "scale", None), (b'F', "transparency", None), (b'L', "file", Some("Ljava/io/File;"))],
                        null,
                    );
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.u8(enabled as u8).i32(x).i32(z).i32(scale.to_bits() as i32).i32(transparency.to_bits() as i32);
            match file_desc {
                None => {
                    file_desc = Some(w.next);
                    w.u8(TC_OBJECT).class("java.io.File", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'L', "path", Some("Ljava/lang/String;"))], null);
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.string(path);
            w.u8(TC_BLOCKDATA).u8(2).u16(b'\\' as u16).u8(TC_ENDBLOCKDATA);
        }
        w.u8(TC_ENDBLOCKDATA);
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(1);
        w.u8(TC_OBJECT).class("java.awt.Point", SC_SERIALIZABLE, &[(b'I', "x", None), (b'I', "y", None)], null);
        w.handle();
        w.i32(0).i32(0);
        let tile_fields: [(u8, &str, Option<&str>); 9] = [
            (b'I', "maxHeight", None),
            (b'I', "minHeight", None),
            (b'Z', "tall", None),
            (b'I', "x", None),
            (b'I', "y", None),
            (b'L', "bitLayerData", Some("Ljava/util/Map;")),
            (b'[', "tallHeightMap", Some("[I")),
            (b'[', "tallWaterLevel", Some("[S")),
            (b'[', "terrain", Some("[B")),
        ];
        w.u8(TC_OBJECT).class(TILE_CLASS, SC_SERIALIZABLE, &tile_fields, null);
        w.handle();
        w.i32(320).i32(-64).u8(1).i32(0).i32(0);
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(covers.len() as i32);
        let (mut layer_desc, mut material_desc, mut identity_desc, mut set_desc) = (None, None, None, None);
        for (name, thickness, block, rows) in covers {
            match layer_desc {
                None => {
                    layer_desc = Some(w.next);
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer",
                        SC_SERIALIZABLE,
                        &[(b'I', "thickness", None), (b'L', "material", Some("Lorg/pepsoft/minecraft/Material;")), (b'L', "name", Some("Ljava/lang/String;"))],
                        null,
                    );
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.i32(*thickness);
            match material_desc {
                None => {
                    material_desc = Some(w.next);
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material",
                        SC_SERIALIZABLE,
                        &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
                        null,
                    );
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.i32(-1).i32(-1);
            match identity_desc {
                None => {
                    identity_desc = Some(w.next);
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material$Identity",
                        SC_SERIALIZABLE,
                        &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
                        null,
                    );
                }
                Some(d) => {
                    w.u8(TC_OBJECT).reference(d);
                }
            }
            w.handle();
            w.string(block);
            w.u8(TC_NULL);
            w.string(name);
            match set_desc {
                None => {
                    let set = w.next;
                    w.u8(TC_OBJECT).class("java.util.BitSet", SC_SERIALIZABLE | SC_WRITE_METHOD, &[(b'[', "bits", Some("[J"))], null);
                    w.handle();
                    let longs = w.next;
                    w.u8(TC_ARRAY).class("[J", SC_SERIALIZABLE, &[], null);
                    set_desc = Some((set, longs));
                }
                Some((set, longs)) => {
                    w.u8(TC_OBJECT).reference(set);
                    w.handle();
                    w.u8(TC_ARRAY).reference(longs);
                }
            }
            w.handle();
            w.i32(256);
            for k in 0..256 {
                w.i64(if rows.contains(&(k / 2)) { -1 } else { 0 });
            }
            w.u8(TC_ENDBLOCKDATA);
        }
        w.u8(TC_ENDBLOCKDATA);
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for _ in 0..TILE_AREA {
            w.i32(((70.0f32 + 64.0) * 256.0) as i32);
        }
        w.u8(TC_ARRAY).class("[S", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for _ in 0..TILE_AREA {
            w.u16(0);
        }
        w.u8(TC_ARRAY).class("[B", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for i in 0..TILE_AREA {
            w.u8([0, 2, 4][(i / 128) * 3 / 128]);
        }
        w.u8(TC_ENDBLOCKDATA);
        w.u8(TC_ENDBLOCKDATA);
        w.string("Layered");
        w.out
    }

    /// A Dimension holding only `hiddenPalettes` (a HashSet of names) and
    /// `soloedPalette`.
    fn palette_dimension(hidden: &[&str], solo: Option<&str>) -> Vec<u8> {
        let mut w = StreamWriter::new();
        w.u8(TC_OBJECT).class("org.pepsoft.worldpainter.Dimension", SC_SERIALIZABLE, &[(b'L', "hiddenPalettes", Some("Ljava/util/Set;")), (b'L', "soloedPalette", Some("Ljava/lang/String;"))], |w| {
            w.u8(TC_NULL);
        });
        w.handle();
        w.u8(TC_OBJECT).class("java.util.HashSet", SC_SERIALIZABLE | SC_WRITE_METHOD, &[], |w| {
            w.u8(TC_NULL);
        });
        w.handle();
        w.u8(TC_BLOCKDATA).u8(12).i32(16).i32(0x3F40_0000).i32(hidden.len() as i32);
        for name in hidden {
            w.string(name);
        }
        w.u8(TC_ENDBLOCKDATA);
        match solo {
            Some(s) => {
                w.string(s);
            }
            None => {
                w.u8(TC_NULL);
            }
        }
        w.out
    }

    /// The palettes WorldPainter hid stay hidden: their covers and object
    /// layers are hidden on the map; a soloed palette hides every layer
    /// but its own.
    #[test]
    fn hidden_palettes_stay_hidden() {
        let visible = |hidden: &[&str], solo: Option<&str>| -> Vec<(String, bool)> {
            let mut reg = Registry::vanilla();
            let fern = reg.covers.iter().position(|c| c.name == "Flowers").unwrap();
            reg.covers[fern].palette = "Forest".into();
            let mut oaks = massif_core::objects::ObjectLayerDef::new("Oaks", &[], &reg);
            oaks.palette = "Forest".into();
            reg.add_object_layer(oaks);
            let mut rocks = massif_core::objects::ObjectLayerDef::new("Rocks", &[], &reg);
            rocks.palette = "Stones".into();
            reg.add_object_layer(rocks);
            let bytes = palette_dimension(hidden, solo);
            let mut parser = Parser::new(&bytes[..]).unwrap();
            let dim = parser.next_object().unwrap().unwrap();
            palette_visibility(&parser.arena, dim, &mut reg);
            let grass = reg.covers.iter().find(|c| c.name == "Grass").unwrap();
            [(grass.name.clone(), !grass.hidden), ("Flowers".into(), !reg.covers[fern].hidden)].into_iter().chain(reg.object_layers.iter().map(|l| (l.name.clone(), !l.hidden))).collect()
        };
        let shown = |v: Vec<(String, bool)>| v.into_iter().filter(|s| s.1).map(|s| s.0).collect::<Vec<_>>();
        assert_eq!(shown(visible(&[], None)), ["Grass", "Flowers", "Oaks", "Rocks"]);
        assert_eq!(shown(visible(&["Forest"], None)), ["Grass", "Rocks"]);
        assert_eq!(shown(visible(&["Forest", "Stones"], None)), ["Grass"]);
        assert_eq!(shown(visible(&[], Some("Stones"))), ["Rocks"]);
    }

    /// WorldPainter's custom ground covers are ground covers of Massif,
    /// painted where they were: three terrains under four covers that
    /// overlap import as the three terrains and the four covers, with no
    /// terrain made of a cover over another terrain. A cover that takes the
    /// place of the top of the land lies on ground lowered under it.
    #[test]
    fn ground_covers_stay_layers_over_the_terrains() {
        let covers = [
            ("Water Block", -1, "minecraft:water", 0..40),
            ("Small stones", 1, "minecraft:stone_button", 20..80),
            ("Gravel", 2, "minecraft:gravel", 60..110),
            ("Moss", 1, "minecraft:moss_carpet", 90..128),
        ];
        let bytes = layered_world(&covers);
        let progress = ImportProgress::default();
        let (world, report) = import_stream(&bytes[..], &progress).unwrap();
        assert!(report.layers.is_empty(), "nothing left out: {:?}", report.layers);
        let reg = &world.registry;
        assert_eq!(report.ground_covers.iter().map(|&n| reg.cover(n).unwrap().name.as_str()).collect::<Vec<_>>(), covers.iter().map(|c| c.0).collect::<Vec<_>>());
        let vanilla = Registry::vanilla();
        assert_eq!(reg.terrains.len(), vanilla.terrains.len(), "no terrain made: {:?}", reg.terrains.iter().skip(vanilla.terrains.len()).map(|t| &t.name).collect::<Vec<_>>());
        assert!(reg.terrains.iter().all(|t| !t.name.contains(" on ")));
        let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        let terrains: std::collections::BTreeSet<u16> = tile.terrain.dense().iter().copied().collect();
        assert_eq!(terrains.len(), 3, "the three terrains, as painted");
        for (name, thickness, block, rows) in &covers {
            let n = reg.covers.iter().position(|c| c.name == *name).unwrap_or_else(|| panic!("{name} is a cover")) as u16 + 1;
            let c = reg.cover(n).unwrap();
            assert_eq!((c.plants[0].0.as_str(), c.height, c.density, c.soil_only), (*block, [thickness.unsigned_abs() as u8; 2], 1.0, false));
            // Its own rows (past the cover painted before it) hold it.
            let own = rows.start + 20.min(rows.len() - 1);
            assert_eq!(tile.cover.get(own * 128 + 7), n, "{name} on row {own}");
            assert!(tile.cover.dense().iter().filter(|&&v| v == n).count() >= 128, "{name}");
        }
        assert_eq!(tile.top.get(5 * 128), 70.0, "the ground goes down under water laid into it");
        assert_eq!(tile.top.get(100 * 128), 71.0, "and stays under other covers");
        assert_eq!(world.dims[0].ground().stats().hot_tiles, 0, "every tile stored, none decoded");
    }

    /// The memory a large imported map holds: as stored now, and with every
    /// ground tile decoded as layers applied after the tiles used to leave
    /// them. `MASSIF_IMPORT_MEMORY=<a .world> cargo test --release -p
    /// massif-legacy import_memory -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn import_memory() {
        let Some(path) = std::env::var_os("MASSIF_IMPORT_MEMORY") else { return };
        let started = std::time::Instant::now();
        let (mut world, _) = import_world(Path::new(&path), None).unwrap();
        let took = started.elapsed();
        let stats = |w: &World| w.dims[0].levels.iter().map(|l| l.stats()).fold((0, 0, 0), |(t, h, b), s| (t + s.tiles, h + s.hot_tiles, b + s.hot_bytes + s.cold_bytes));
        let (tiles, hot, bytes) = stats(&world);
        eprintln!("imported in {:.2} s: {tiles} tiles, {hot} decoded, {:.1} MB", took.as_secs_f32(), bytes as f64 / 1e6);
        let positions: Vec<TilePos> = world.dims[0].ground().positions().collect();
        for pos in positions {
            world.dims[0].ground_mut().tile_mut(pos);
        }
        world.dims[0].ground_mut().take_dirty();
        world.dims[0].ground_mut().freeze();
        let (tiles, hot, bytes) = stats(&world);
        eprintln!("every tile decoded, as before: {tiles} tiles, {hot} decoded, {:.1} MB", bytes as f64 / 1e6);
    }

    /// WorldPainter's overlay images come in as reference images at the
    /// same place, scale and opacity: one written with a Windows path found
    /// by its name beside the world, its middle set from its corner and its
    /// size; one whose file is nowhere kept by its corner, hidden as it was,
    /// and named in the report.
    #[test]
    fn overlays_become_reference_images() {
        let dir = std::env::temp_dir().join(format!("massif-overlays-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A PNG of 40 × 20 transparent pixels.
        let chunk = |png: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            png.extend((data.len() as u32).to_be_bytes());
            png.extend(kind);
            png.extend(data);
            png.extend(crc.sum().to_be_bytes());
        };
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(40u32.to_be_bytes());
        ihdr.extend(20u32.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(&mut png, b"IHDR", &ihdr);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut z, &vec![0u8; 20 * (1 + 40 * 4)]).unwrap();
        chunk(&mut png, b"IDAT", &z.finish().unwrap());
        chunk(&mut png, b"IEND", &[]);
        std::fs::write(dir.join("plan.png"), &png).unwrap();
        let bytes = layered_world_with(&[], &[("C:\\Users\\Jack\\maps\\plan.png", 100, -50, 2.0, 0.25, true), ("/nowhere/gone.png", -10, 5, 1.5, 0.0, false)]);
        let world_file = dir.join("Layered.world");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &bytes).unwrap();
        std::fs::write(&world_file, gz.finish().unwrap()).unwrap();
        let (world, report) = import_world(&world_file, None).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let refs = &world.meta.references;
        assert_eq!((report.overlays, refs.len()), (2, 2));
        let plan = &refs[0];
        assert_eq!(Path::new(&plan.path), dir.join("plan.png"));
        assert_eq!((plan.size, plan.x, plan.z, plan.scale, plan.corner), ([40, 20], 140.0, -30.0, 2.0, None));
        assert!((plan.opacity - 0.75).abs() < 1e-6 && plan.shown);
        let gone = &refs[1];
        assert_eq!((gone.path.as_str(), gone.size, gone.corner, gone.scale, gone.shown), ("/nowhere/gone.png", [0, 0], Some([-10.0, 5.0]), 1.5, false));
        assert!((gone.opacity - 0.5).abs() < 1e-6, "WorldPainter's default transparency");
        assert_eq!(report.missing_images, ["/nowhere/gone.png"]);
        // Found again, it takes its place from its corner.
        let mut found = gone.clone();
        assert!(found.set_size([10, 10]));
        assert_eq!((found.x, found.z, found.corner), (-2.5, 12.5, None));
    }

    #[test]
    fn a_synthetic_world_imports_faithfully() {
        let bytes = synthetic_world();
        let progress = ImportProgress::default();
        let (world, report) = import_stream(&bytes[..], &progress).unwrap();
        assert_eq!(world.meta.name, "Synthetic");
        assert_eq!(world.meta.seed, 27594263);
        assert_eq!(world.meta.sea_level, 63);
        assert_eq!(report.tiles, 2);
        assert_eq!(progress.tiles.load(Ordering::Relaxed), 2);
        assert_eq!(report.custom_terrains, 1);
        assert!(report.layers.is_empty(), "{:?}", report.layers);
        let mut converted: Vec<&str> = report.converted.iter().map(|l| l.0.as_str()).collect();
        converted.sort_unstable();
        assert_eq!(converted, ["Annotations", "Deciduous", "Flowers", "Frost", "Lava", "Tall spruces", "Void"]);
        assert!(report.skipped_dimensions.is_empty());

        let dim = &world.dims[0];
        assert_eq!((dim.kind, dim.min_y, dim.max_y), (DimensionKind::Overworld, -64, 320));
        let ground = dim.ground();
        assert_eq!(ground.len(), 2);
        let tall = ground.view(TilePos::new(0, 0)).unwrap();
        // WorldPainter height 70 → solid up to y = 70 → Massif surface 71.
        assert_eq!(tall.top.get(0), 71.0);
        assert_eq!(tall.top.get(5), 100.5);
        assert_eq!(block_top(tall.top.get(5)), 101); // WP: top block at round(99.5) = 100
        assert_eq!(tall.fluid.get(0), 63);
        let reg = &world.registry;
        assert_eq!(reg.terrain(TerrainId(tall.terrain.get(0))).name, "Sand");
        let custom = reg.terrain(TerrainId(tall.terrain.get(500)));
        assert_eq!(custom.name, "Volcanic");
        assert_eq!(reg.block(custom.strata[0].blocks[0].0).state, "minecraft:basalt[axis=y]");
        assert_eq!(reg.biome(massif_core::BiomeId(tall.biome.get(0))).id, "minecraft:forest");
        assert_eq!(tall.biome.get(200), 0);

        let short = ground.view(TilePos::new(-1, 0)).unwrap();
        assert_eq!(short.top.get(local_index(-1, 0)), 41.0);
        assert_eq!(reg.terrain(TerrainId(short.terrain.get(0))).name, "Grass");
        // Legacy tiles without water data are dry.
        assert_eq!(short.fluid.get(0), NO_FLUID);

        // The layers: the tree pack's spruces on row 0 at full strength, Frost
        // on row 1, the plant layer on row 2, the deciduous forest at half
        // strength from row 3 (a quarter of its columns, as WorldPainter's
        // forests thin out by the square of their strength).
        let frost = reg.cover(tall.cover.get(128)).unwrap();
        assert!(frost.frost && frost.name == "Frost");
        assert!((128..256).all(|i| tall.cover.get(i) == tall.cover.get(128)));
        let flowers = reg.cover(tall.cover.get(256)).unwrap();
        assert_eq!(flowers.name, "Flowers");
        assert_eq!(flowers.plants, [("minecraft:short_grass".to_owned(), 30), ("minecraft:dandelion".to_owned(), 10)]);
        assert_eq!((flowers.color, flowers.density, flowers.soil_only, flowers.underwater), ([0xd6, 0xa8, 0xba], 1.0, true, false));
        assert_eq!(tall.cover.get(0), 0);
        assert_eq!(tall.cover.get(400), 0);
        let spruces = reg.object_layer(tall.objects.get(0)).unwrap();
        assert_eq!(spruces.name, "Tall spruces");
        assert!((0..128).all(|i| tall.objects.get(i) == tall.objects.get(0)));
        assert!((128..384).all(|i| tall.objects.get(i) == 0));
        let forest: Vec<u16> = (384..TILE_AREA).map(|i| tall.objects.get(i)).filter(|&n| n != 0).collect();
        let deciduous = reg.object_layer(forest[0]).unwrap();
        assert_eq!(deciduous.name, "Deciduous");
        assert!(forest.iter().all(|&n| n == forest[0]));
        let share = forest.len() as f32 / (TILE_AREA - 384) as f32;
        // Painted at about half strength: WorldPainter's forest is still
        // half as dense as at full, its trees soon meeting.
        assert!((0.4..0.65).contains(&share), "{share}");
        assert_eq!(deciduous.objects.iter().filter(|o| o.1 > 0).count(), 16, "big oaks and birches, eight of each");
        assert_eq!(deciduous.objects.len(), 32, "and their bushes and small ones, for Tree sizes");
        assert_eq!(short.cover.get(0), 0);
        assert_eq!(short.objects.get(0), 0);
        // Lava on row 5, as a cover; Void on row 6, nothing down to the
        // floor and no water; annotations in red on row 4.
        let lava = reg.cover(tall.cover.get(5 * 128)).unwrap();
        assert!(lava.lava && lava.name == "Lava");
        assert_eq!((tall.top.get(6 * 128 + 9), tall.fluid.get(6 * 128 + 9)), (-64.0, NO_FLUID));
        assert_eq!(tall.top.get(7 * 128 + 9), 71.0);
        assert!((4 * 128..5 * 128).all(|i| tall.annotations.get(i) == 14));
        assert_eq!(tall.annotations.get(3 * 128), 0);
        assert_eq!(short.annotations.get(4 * 128), 0);

        // The spawn point stands on the surface.
        assert_eq!(world.meta.spawn, [3, 71, 2]);
        assert!(report.summary().contains("2 tiles"));
    }

    /// A world WorldPainter made taller than the game (2032 high, as its
    /// 1.18+ platforms allow): its build limits and heights are kept.
    #[test]
    fn a_tall_world_keeps_its_height() {
        let bytes = synthetic_world_with(2032, 1500.0);
        let (world, _) = import_stream(&bytes[..], &ImportProgress::default()).unwrap();
        let dim = &world.dims[0];
        assert_eq!((dim.min_y, dim.max_y), (-64, 2032));
        let tall = dim.ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!(tall.top.get(5), 1501.0);
        assert_eq!(tall.top.get(0), 71.0);
    }

    #[test]
    fn thumbnails_are_found_in_the_gzip_header() {
        let mut head = vec![0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 0xff];
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let extra = [b'X', b'Y', 1, 0, 9, b'W', b'P', 4, 0, jpeg[0], jpeg[1], jpeg[2], jpeg[3]];
        head.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        head.extend_from_slice(&extra);
        assert_eq!(read_thumbnail(&head), Some(jpeg.to_vec()));
        head[3] = 0;
        assert_eq!(read_thumbnail(&head), None);
    }

    #[test]
    fn junk_is_not_a_world() {
        let progress = ImportProgress::default();
        assert!(matches!(import_stream(&b"hello"[..], &progress), Err(ImportError::NotWorldPainter)));
        let mut w = StreamWriter::new();
        w.string("just a string");
        assert!(matches!(import_stream(&w.out[..], &progress), Err(ImportError::NotWorldPainter)));
        let bytes = synthetic_world();
        let truncated = &bytes[..bytes.len() / 2];
        assert!(matches!(import_stream(truncated, &progress), Err(ImportError::Damaged(_))));
    }

    /// The user's own worlds, when present: `MASSIF_WORLD_SAMPLES` lists
    /// `.world` files separated by `:`.
    #[test]
    fn real_worlds_import() {
        let Some(list) = std::env::var_os("MASSIF_WORLD_SAMPLES") else {
            return;
        };
        for path in std::env::split_paths(&list) {
            let started = std::time::Instant::now();
            let (world, report) = import_world(&path, None).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            eprintln!("{}: {:.2?}\n  {}\n  {report:?}", path.display(), started.elapsed(), report.summary());
            if std::env::var_os("MASSIF_PRINT_BLOCKS").is_some() {
                for t in world.registry.terrains.iter().filter(|t| t.group == "Imported") {
                    let blocks: Vec<String> = t.strata.iter().flat_map(|s| s.blocks.iter().map(|(b, w)| format!("{}×{w}", world.registry.block(*b).state))).collect();
                    eprintln!("  {:<24} {:?} {}", t.name, t.color, blocks.join(" "));
                }
            }
            if std::env::var_os("MASSIF_PRINT_LAYERS").is_some() {
                let reg = &world.registry;
                for l in reg.object_layers.iter() {
                    let objects: Vec<String> = l.objects.iter().take(4).map(|&(k, w)| {
                        let o = &reg.objects[k as usize];
                        format!("{} {:?} anchor {:?} ×{w}", o.name, o.size, o.anchor)
                    }).collect();
                    eprintln!("  objects {:<22} spacing {} density {:.2} rotate {} under water {} soil {}: {}", l.name, l.spacing, l.density, l.rotate, l.underwater, l.soil_only, objects.join("; "));
                }
                for c in reg.covers.iter() {
                    eprintln!("  cover {:<24} {:?} density {} soil {} under water {} frost {} lava {}", c.name, c.plants, c.density, c.soil_only, c.underwater, c.frost, c.lava);
                }
                for d in &world.dims {
                    for l in &d.levels {
                        eprintln!("  level {:?} {} {:?}: {} tiles", d.kind, l.name, l.kind, l.len());
                    }
                    eprintln!("  voxel chunks {:?}: {}", d.kind, d.voxels.len());
                }
                for t in reg.terrains.iter().filter(|t| t.name.contains(" on ")) {
                    eprintln!("  terrain {} {:?}", t.name, t.strata.iter().map(|s| (s.blocks.len(), s.depth)).collect::<Vec<_>>());
                }
            }
            assert!(report.tiles > 0);
            let tiles: usize = world.dims.iter().map(|d| d.ground().len()).sum();
            assert_eq!(tiles, report.tiles);
        }
    }
}
