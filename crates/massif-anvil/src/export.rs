//! Exporting a dimension as a Minecraft Java world folder.

use crate::chunk::{ExportContext, Scratch, build_chunk, tile_placements};
use crate::level_dat::{Difficulty, GameMode, LevelSettings, Surroundings, WorldBorder, level_dat};
use crate::region::RegionBuilder;
use crate::version::{Era, Target};
use massif_core::TilePos;
use massif_core::compose::TileStack;
use massif_core::world::World;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A region file covers 4×4 tiles (512×512 blocks).
const TILES_PER_REGION: i32 = 4;
/// A tile covers 8×8 chunks.
const CHUNKS_PER_TILE: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("{} already exists and is not empty", .0.display())]
    Exists(PathBuf),
    #[error("the export was cancelled")]
    Cancelled,
    #[error("this world cannot be exported: {0}")]
    Unsupported(String),
    #[error("the map is empty: there is nothing to export")]
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    /// Index of the dimension to export.
    pub dimension: usize,
    /// World name shown in Minecraft; defaults to the world's own name.
    pub level_name: Option<String>,
    pub game_mode: GameMode,
    pub allow_commands: bool,
    pub surroundings: Surroundings,
    pub generate_structures: bool,
    /// Replace an existing export in the target folder. Region files already
    /// there are deleted first; everything else is left alone.
    pub overwrite: bool,
    /// zlib level for chunk data (1 = fastest, 9 = smallest).
    pub compression: u32,
    pub difficulty: Difficulty,
    pub hardcore: bool,
    pub border: WorldBorder,
    /// Data packs (folders or zip files) copied into the world and enabled.
    pub datapacks: Vec<PathBuf>,
    /// Where players appear; `None`: the world's own spawn point.
    pub spawn: Option<[i32; 3]>,
    /// Only export the tiles in this rectangle (inclusive), for a quick test.
    pub area: Option<(TilePos, TilePos)>,
    /// The version of Minecraft the world is written for.
    pub target: Target,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            dimension: 0,
            level_name: None,
            game_mode: GameMode::Creative,
            allow_commands: true,
            surroundings: Surroundings::Vanilla,
            generate_structures: true,
            overwrite: false,
            compression: 3,
            difficulty: Difficulty::Normal,
            hardcore: false,
            border: WorldBorder::default(),
            datapacks: Vec::new(),
            spawn: None,
            area: None,
            target: Target::default(),
        }
    }
}

/// Shared progress and cancellation state of a running export.
#[derive(Debug, Default)]
pub struct Progress {
    /// Tiles exported so far.
    pub done: AtomicUsize,
    /// Tiles to export.
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            self.done.load(Ordering::Relaxed) as f32 / total as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportStats {
    pub regions: usize,
    pub chunks: usize,
    pub bytes: u64,
    pub elapsed: Duration,
}

fn dir_is_empty(dir: &Path) -> io::Result<bool> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

/// Export one dimension of `world` to the world folder `dir`.
///
/// Reads tiles through [`massif_core::Level::view`], so it works on a live
/// world as well as on a snapshot; pass a snapshot to export in the
/// background while editing continues.
pub fn export(
    world: &World,
    dir: &Path,
    options: &ExportOptions,
    progress: &Progress,
) -> Result<ExportStats, ExportError> {
    let started = Instant::now();
    let dim = world
        .dims
        .get(options.dimension)
        .ok_or_else(|| ExportError::Unsupported("no such dimension".into()))?;
    let ctx = ExportContext::new(world, dim, &options.target).map_err(ExportError::Unsupported)?;

    // Group the map's tiles by region file.
    let mut regions: HashMap<(i32, i32), Vec<TilePos>> = HashMap::new();
    let in_area = |p: TilePos| options.area.is_none_or(|(lo, hi)| p.x >= lo.x && p.x <= hi.x && p.z >= lo.z && p.z <= hi.z);
    for pos in dim.ground().positions().filter(|p| in_area(*p)) {
        let key = (
            pos.x.div_euclid(TILES_PER_REGION),
            pos.z.div_euclid(TILES_PER_REGION),
        );
        regions.entry(key).or_default().push(pos);
    }
    if regions.is_empty() {
        return Err(ExportError::Empty);
    }
    let total: usize = regions.values().map(Vec::len).sum();
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(total, Ordering::Relaxed);

    if !dir_is_empty(dir)? && !options.overwrite {
        return Err(ExportError::Exists(dir.to_owned()));
    }
    let region_dir = dir.join("region");
    std::fs::create_dir_all(&region_dir)?;
    if options.overwrite {
        for entry in std::fs::read_dir(&region_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "mca") {
                std::fs::remove_file(path)?;
            }
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let timestamp = now.as_secs() as u32;
    let chunks = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let level = options.compression.clamp(1, 9);

    let regions: Vec<((i32, i32), Vec<TilePos>)> = regions.into_iter().collect();
    regions
        .par_iter()
        .try_for_each(|((rx, rz), tiles)| -> Result<(), ExportError> {
            let mut scratch = Scratch::new(&ctx);
            let mut region = RegionBuilder::new();
            // The objects of the tiles within their reach of each tile:
            // those of the region's edges stand on its neighbours too.
            let ring = (ctx.reach + 127) / 128;
            let mut placed: HashMap<TilePos, Vec<massif_core::objects::Placement>> = HashMap::new();
            for &pos in tiles {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(ExportError::Cancelled);
                }
                let Some(stack) = TileStack::new(dim, pos, true) else {
                    continue;
                };
                let mut objects = Vec::new();
                if ctx.reach > 0 {
                    let area = pos.rect().expand(ctx.reach);
                    for tz in pos.z - ring..=pos.z + ring {
                        for tx in pos.x - ring..=pos.x + ring {
                            let t = TilePos::new(tx, tz);
                            let here = placed.entry(t).or_insert_with(|| tile_placements(&ctx, world, dim, t));
                            objects.extend(here.iter().filter(|p| area.x0 <= p.x && p.x < area.x1 && area.z0 <= p.z && p.z < area.z1).copied());
                        }
                    }
                    objects.sort_unstable_by_key(|p| (p.layer, p.x, p.z));
                }
                for cz in 0..CHUNKS_PER_TILE {
                    for cx in 0..CHUNKS_PER_TILE {
                        let nbt = build_chunk(&ctx, &stack, pos, cx, cz, &objects, &mut scratch);
                        let chunk_x = pos.x * CHUNKS_PER_TILE as i32 + cx as i32;
                        let chunk_z = pos.z * CHUNKS_PER_TILE as i32 + cz as i32;
                        region.put(
                            chunk_x.rem_euclid(32) as usize,
                            chunk_z.rem_euclid(32) as usize,
                            &nbt,
                            level,
                        )?;
                    }
                }
                progress.done.fetch_add(1, Ordering::Relaxed);
            }
            let path = region_dir.join(format!("r.{rx}.{rz}.mca"));
            let file = io::BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?);
            bytes.fetch_add(region.write(file, timestamp)?, Ordering::Relaxed);
            chunks.fetch_add(region.chunk_count(), Ordering::Relaxed);
            Ok(())
        })?;

    // Data packs: copied into the world's datapacks folder and enabled.
    let mut packs = Vec::new();
    if !options.datapacks.is_empty() {
        let target = dir.join("datapacks");
        std::fs::create_dir_all(&target)?;
        for pack in &options.datapacks {
            let Some(name) = pack.file_name() else { continue };
            copy_recursively(pack, &target.join(name))?;
            packs.push(format!("file/{}", name.to_string_lossy()));
        }
    }
    // Build limits past the game's own: a data pack gives the world that
    // height.
    if let Some(name) = height_pack(dir, &options.target, ctx.min_y, ctx.max_y)? {
        packs.push(name);
    }
    let settings = LevelSettings {
        name: options
            .level_name
            .clone()
            .unwrap_or_else(|| world.meta.name.clone()),
        seed: world.meta.seed,
        spawn: options.spawn.unwrap_or(world.meta.spawn),
        game_mode: options.game_mode,
        allow_commands: options.allow_commands,
        surroundings: options.surroundings,
        generate_structures: options.generate_structures,
        last_played: now.as_millis() as i64,
        difficulty: options.difficulty,
        hardcore: options.hardcore,
        border: options.border,
        datapacks: packs,
    };
    std::fs::write(dir.join("level.dat"), level_dat(&settings, &options.target)?)?;

    Ok(ExportStats {
        regions: regions.len(),
        chunks: chunks.load(Ordering::Relaxed),
        bytes: bytes.load(Ordering::Relaxed),
        elapsed: started.elapsed(),
    })
}

/// The data pack that gives the overworld the build limits of a map that
/// goes past the game's own (below −64 or above 320): the folder it is
/// written to and enabled as. `None` when the limits are the game's.
fn height_pack(dir: &Path, target: &Target, min_y: i32, max_y: i32) -> io::Result<Option<String>> {
    if target.era() != Era::Modern || (min_y >= -64 && max_y <= 320) {
        return Ok(None);
    }
    // The land around the map is generated between −64 and 320: the world
    // holds that too.
    let (bottom, top) = (min_y.min(-64), max_y.max(320));
    let height = top - bottom;
    let name = "massif_build_limits";
    let pack = dir.join("datapacks").join(name);
    let types = pack.join("data").join("minecraft").join("dimension_type");
    std::fs::create_dir_all(&types)?;
    let format = target.pack_format();
    let mcmeta = format!(
        r#"{{
  "pack": {{
    "description": "The build limits of this map: Y {bottom} to {top}. Made by Massif.",
    "pack_format": {format},
    "supported_formats": {{ "min_inclusive": 8, "max_inclusive": 1000 }},
    "min_format": 8,
    "max_format": 1000
  }}
}}
"#
    );
    std::fs::write(pack.join("pack.mcmeta"), mcmeta)?;
    // What 1.19 added; the way 1.20.5 writes a range.
    let monsters = match target.data_version {
        ..3105 => String::new(),
        3105..3837 => r#",
  "monster_spawn_light_level": { "type": "minecraft:uniform", "value": { "min_inclusive": 0, "max_inclusive": 7 } },
  "monster_spawn_block_light_limit": 0"#
            .to_owned(),
        _ => r#",
  "monster_spawn_light_level": { "type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 7 },
  "monster_spawn_block_light_limit": 0"#
            .to_owned(),
    };
    let overworld = format!(
        r##"{{
  "ultrawarm": false,
  "natural": true,
  "coordinate_scale": 1.0,
  "has_skylight": true,
  "has_ceiling": false,
  "ambient_light": 0.0,
  "piglin_safe": false,
  "bed_works": true,
  "respawn_anchor_works": false,
  "has_raids": true,
  "min_y": {bottom},
  "height": {height},
  "logical_height": {height},
  "infiniburn": "#minecraft:infiniburn_overworld",
  "effects": "minecraft:overworld"{monsters}
}}
"##
    );
    std::fs::write(types.join("overworld.json"), overworld)?;
    Ok(Some(format!("file/{name}")))
}

/// Copy a file, or a folder and everything in it.
fn copy_recursively(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// A folder name inside `parent` that does not exist yet: `name`, or
/// `name (2)`, `name (3)`, ...
pub fn unique_dir(parent: &Path, name: &str) -> PathBuf {
    let clean: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c })
        .collect();
    let clean = clean.trim().trim_matches('.');
    let base = if clean.is_empty() { "World" } else { clean };
    let mut candidate = parent.join(base);
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{base} ({n})"));
        n += 1;
    }
    candidate
}

/// Minecraft's `saves` folder for the current user, if it exists.
pub fn default_saves_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let home = PathBuf::from(home);
    let candidates = [
        home.join("Library/Application Support/minecraft/saves"),
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join(".minecraft/saves"),
        home.join(".minecraft/saves"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}
