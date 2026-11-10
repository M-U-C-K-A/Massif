//! Exporting a dimension as a Minecraft Java world folder.

use crate::chunk::{ExportContext, Scratch, build_chunk};
use crate::level_dat::{GameMode, LevelSettings, Surroundings, level_dat};
use crate::region::RegionBuilder;
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
    let ctx = ExportContext::new(world, dim).map_err(ExportError::Unsupported)?;

    // Group the map's tiles by region file.
    let mut regions: HashMap<(i32, i32), Vec<TilePos>> = HashMap::new();
    for pos in dim.ground().positions() {
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
            for &pos in tiles {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(ExportError::Cancelled);
                }
                let Some(stack) = TileStack::new(dim, pos, true) else {
                    continue;
                };
                for cz in 0..CHUNKS_PER_TILE {
                    for cx in 0..CHUNKS_PER_TILE {
                        let nbt = build_chunk(&ctx, &stack, pos, cx, cz, &mut scratch);
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

    let settings = LevelSettings {
        name: options
            .level_name
            .clone()
            .unwrap_or_else(|| world.meta.name.clone()),
        seed: world.meta.seed,
        spawn: world.meta.spawn,
        game_mode: options.game_mode,
        allow_commands: options.allow_commands,
        surroundings: options.surroundings,
        generate_structures: options.generate_structures,
        last_played: now.as_millis() as i64,
    };
    std::fs::write(dir.join("level.dat"), level_dat(&settings)?)?;

    Ok(ExportStats {
        regions: regions.len(),
        chunks: chunks.load(Ordering::Relaxed),
        bytes: bytes.load(Ordering::Relaxed),
        elapsed: started.elapsed(),
    })
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
