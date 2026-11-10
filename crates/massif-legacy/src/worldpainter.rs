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
    /// WorldPainter terrains in the world, with their column counts.
    pub terrains: Vec<(String, u64)>,
    pub custom_terrains: usize,
    /// The JPEG thumbnail stored in the file, if any.
    pub thumbnail: Option<Vec<u8>>,
}

impl ImportReport {
    /// A short human summary.
    pub fn summary(&self) -> String {
        let mut s = format!("{} tiles", self.tiles);
        if !self.dimensions.is_empty() {
            s += &format!(" in {}", self.dimensions.join(", "));
        }
        if !self.layers.is_empty() {
            let names: Vec<&str> = self.layers.iter().map(|l| l.0.as_str()).collect();
            s += &format!("; layers not imported yet: {}", names.join(", "));
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
    let mut result = import_stream(BufReader::with_capacity(1 << 20, gz), progress);
    if let Ok((world, report)) = &mut result {
        report.thumbnail = thumbnail;
        if world.meta.name.is_empty() {
            world.meta.name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Imported World".into());
        }
    }
    result
}

/// A tile's data, taken out of the arena, waiting to be encoded.
struct RawTile {
    handle: Handle,
    pos: TilePos,
    top: Vec<f32>,
    terrain: Vec<u16>,
    fluid: Vec<i16>,
    biome: Option<Vec<u16>>,
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
                *self.layers.entry(name).or_default() += 1;
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

        let top: Vec<f32> = match heights {
            Some(Array::Int(raw)) if raw.len() == TILE_AREA => {
                raw.iter().map(|&r| snap_height(r as f32 / 256.0 + min + 1.0)).collect()
            }
            Some(Array::Short(raw)) if raw.len() == TILE_AREA => {
                raw.iter().map(|&r| snap_height((r as u16) as f32 / 256.0 + min + 1.0)).collect()
            }
            _ => {
                self.damaged += 1;
                return None;
            }
        };
        let terrain: Vec<u16> = match terrain {
            Some(Array::Byte(raw)) if raw.len() == TILE_AREA => {
                let mut counts = [0u64; 256];
                for &o in &raw {
                    counts[o as usize] += 1;
                }
                for (o, &c) in counts.iter().enumerate() {
                    self.histogram[o] += c;
                }
                raw.iter().map(|&o| self.mapper.terrain(o)).collect()
            }
            _ => vec![self.mapper.terrain(0); TILE_AREA],
        };
        let fluid: Vec<i16> = match water {
            Some(Array::Short(raw)) if raw.len() == TILE_AREA => raw
                .iter()
                .map(|&w| ((w as u16) as i32 + min as i32 + 1).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16)
                .collect(),
            Some(Array::Byte(raw)) if raw.len() == TILE_AREA => raw
                .iter()
                .map(|&w| (w as i32 + min as i32 + 1).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16)
                .collect(),
            _ => vec![NO_FLUID; TILE_AREA],
        };
        let biome = biome_bytes
            .filter(|b| b.len() == TILE_AREA)
            .map(|b| b.iter().map(|&wp| self.mapper.biome(wp)).collect());
        Some(RawTile {
            handle: h,
            pos: TilePos::new(x, z),
            top,
            terrain,
            fluid,
            biome,
        })
    }
}

fn encode(raw: RawTile) -> (Handle, TilePos, Blob) {
    let mut tile = Tile::ground(0.0, 0, NO_FLUID);
    tile.top = Plane::from_vec(raw.top);
    tile.terrain = Plane::from_vec(raw.terrain);
    tile.fluid = Plane::from_vec(raw.fluid);
    if let Some(b) = raw.biome {
        tile.biome = Plane::from_vec(b);
    }
    tile.compact();
    (raw.handle, raw.pos, Blob::from(tile.encode()))
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

fn read_mixed(a: &Arena, v: Value) -> Option<CustomMaterial> {
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
fn material_state(a: &Arena, material: Value) -> String {
    let identity = a.field(material, "identity").filter(|i| *i != Value::Null);
    let Some(identity) = identity else {
        // A pre-1.13 material known only by its numeric id.
        return "minecraft:stone".into();
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
    let converted: Mutex<HashMap<Handle, (TilePos, Blob)>> = Mutex::new(HashMap::new());
    let mut histogram = [0u64; 256];
    let mut damaged_tiles = 0;

    // In place: the parse runs on this thread (whose stack the caller
    // chose) while tiles are encoded on the pool.
    let (tops, arena) = rayon::in_place_scope(|s| -> Result<(Vec<Value>, Arena), ImportError> {
        let mut parser = Parser::new(reader).map_err(|e| damaged(e, progress))?;
        let mut hook = TileHook {
            mapper: &mut mapper,
            layers: &mut layers,
            histogram: [0; 256],
            damaged: 0,
        };
        let converted = &converted;
        let result = {
            let hook_ref = &mut hook;
            parser.set_hook(Box::new(move |arena: &mut Arena, h: Handle| {
                if let Some(raw) = hook_ref.extract(arena, h) {
                    s.spawn(move |_| {
                        let (handle, pos, blob) = encode(raw);
                        converted.lock().expect("no panics while holding the lock").insert(handle, (pos, blob));
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
        for (_, tile) in tiles {
            let Value::Ref(h) = tile else { continue };
            if let Some((pos, blob)) = converted.get(&h) {
                ground.insert_blob(*pos, blob.clone());
                count += 1;
            }
        }
        ground.take_dirty();
        report.tiles += count;
        report
            .dimensions
            .push(format!("{} ({count} tiles, Y {min_y} to {})", kind.label(), max_y - 1));
        if kind == DimensionKind::Overworld {
            overworld = Some(f.dimension);
        }
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

    report.layers = layers.into_iter().collect();
    report.layers.sort_by_key(|l| std::cmp::Reverse(l.1));
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
        },
        registry: mapper.registry,
        dims,
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
        w.i32(320).i32(-64);

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
        w.i32(62).i32(320).i32(-64);
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
        w.i32(320).i32(-64).u8(1).i32(0).i32(0);
        w.u8(TC_NULL); // bitLayerData
        w.u8(TC_NULL); // heightMap
        // layerData: HashMap { Biome -> byte[16384] of 4 (jungle), Trees -> byte[8192] }
        w.u8(TC_OBJECT).reference(hashmap_desc);
        w.handle();
        w.i32(0x3F40_0000).i32(12).u8(TC_BLOCKDATA).u8(8).i32(16).i32(2);
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
        w.out.extend(std::iter::repeat_n(0u8, 8192));
        w.u8(TC_ENDBLOCKDATA);
        // tallHeightMap: 70.0 everywhere except column 5 at 99.5 (raw = (h + 64) * 256)
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(TILE_AREA as i32);
        for i in 0..TILE_AREA {
            let h = if i == 5 { 99.5f32 } else { 70.0 };
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
        assert_eq!(report.layers, vec![("Tall spruces".to_owned(), 1)]);
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

        // The spawn point stands on the surface.
        assert_eq!(world.meta.spawn, [3, 71, 2]);
        assert!(report.summary().contains("2 tiles"));
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
            assert!(report.tiles > 0);
            let tiles: usize = world.dims.iter().map(|d| d.ground().len()).sum();
            assert_eq!(tiles, report.tiles);
        }
    }
}
