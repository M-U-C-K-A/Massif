//! Files brought into a graph by its File input node: a height map image,
//! or a whole world — WorldPainter's `.world`, Massif's `.massif` — read for
//! its surface, its water and the colours of its terrains.
//!
//! Worlds are read by a reader the application gives
//! ([`set_world_reader`]): the readers of those formats live in crates that
//! build on this one. What was read is kept while the file stays as it is.

use super::map::{MapSample, sample_map};
use super::{ColorField, Extent, Field};
use crate::world::World;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Reads a world from a file.
pub type WorldReader = fn(&Path) -> Result<World, String>;

static READER: OnceLock<WorldReader> = OnceLock::new();

/// Give the reader of world files (once; later calls change nothing).
pub fn set_world_reader(reader: WorldReader) {
    let _ = READER.set(reader);
}

/// Whether a file is a world (else, a height map).
pub fn is_world(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("world" | "massif")
    )
}

/// The world in `path`, read once while the file stays as it is.
pub fn world(path: &Path) -> Result<Arc<World>, String> {
    type Cache = Mutex<HashMap<PathBuf, (Option<SystemTime>, Arc<World>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| e.to_string())?;
    let cache = CACHE.get_or_init(Default::default);
    if let Some((when, w)) = cache.lock().expect("not poisoned").get(path)
        && *when == Some(modified)
    {
        return Ok(w.clone());
    }
    let reader = READER.get().ok_or("worlds cannot be read here")?;
    let w = Arc::new(reader(path)?);
    cache
        .lock()
        .expect("not poisoned")
        .insert(path.to_path_buf(), (Some(modified), w.clone()));
    Ok(w)
}

/// A file over the extent `e`: a height map stretched over it (black at
/// `low`, white at `high`), or a world's surface — fitted to the extent
/// (`fit`), or at its own blocks.
pub fn sample_file(
    path: &Path,
    e: &Extent,
    low: f32,
    high: f32,
    fit: bool,
) -> Result<MapSample, String> {
    let n = e.n;
    if is_world(path) {
        let w = world(path)?;
        let dim = w.dims.first().ok_or("the world has no dimension")?;
        let extent = match (fit, dim.bounds()) {
            (true, Some((lo, hi))) => {
                let t = crate::coords::TILE_SIZE;
                Extent::covering(
                    crate::coords::BlockRect::new(
                        lo.x * t,
                        lo.z * t,
                        (hi.x + 1) * t,
                        (hi.z + 1) * t,
                    ),
                    n,
                )
            }
            _ => *e,
        };
        let colors: Vec<[u8; 3]> = w.registry.terrains.iter().map(|t| t.color).collect();
        return Ok(sample_map(
            dim,
            &extent,
            w.meta.sea_level as f32 - 8.0,
            &colors,
            &[],
        ));
    }
    let map = crate::heightmap::cached(path)?;
    let (w, h) = (map.width as f32, map.height as f32);
    let values: Vec<f32> = (0..n * n)
        .map(|c| {
            map.sample(
                ((c % n) as f32 + 0.5) / n as f32 * w - 0.5,
                ((c / n) as f32 + 0.5) / n as f32 * h - 0.5,
            )
        })
        .collect();
    Ok(MapSample {
        heights: Field::new(n, values.iter().map(|v| low + (high - low) * v).collect()),
        water: Field::new(n, vec![0.0; n * n]),
        // Grey, lighter with height, to see the relief by.
        colors: ColorField {
            n,
            data: values
                .iter()
                .map(|v| [(70.0 + 170.0 * v) as u8; 3])
                .collect(),
        },
        level: Field::new(n, vec![super::NO_WATER; n * n]),
    })
}
