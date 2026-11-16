//! Save points: the map as it was at a moment the user named ("before the
//! rivers"), kept with the map to go back to, open or compare.
//!
//! A point holds a cold copy of the world. Its tiles are the same shared
//! blobs as the map's until the map changes them, so a point costs only the
//! tiles changed since it was made, in memory as in the file (see
//! `massif_format`, which writes each distinct tile once).

use crate::coords::TilePos;
use crate::level::Blob;
use crate::world::World;
use std::collections::HashMap;
use std::sync::Arc;

/// A named copy of the map.
#[derive(Debug, Clone)]
pub struct SavePoint {
    pub name: String,
    /// When it was made, in seconds since 1970 (UTC).
    pub created: u64,
    /// The map as it was: cold, and without save points of its own.
    pub world: Arc<World>,
    /// A small picture of it from above (a JPEG image).
    pub thumbnail: Option<Arc<[u8]>>,
}

/// The longest side of a save point's picture, in pixels.
pub const THUMBNAIL_SIZE: u32 = 160;

impl SavePoint {
    /// A point of `world` as it is now, named `name`: a cold copy that shares
    /// its tiles, and its picture.
    pub fn new(world: &mut World, name: impl Into<String>, created: u64) -> SavePoint {
        let snapshot = Self::copy_of(world);
        SavePoint::of(snapshot, name, created)
    }

    /// A cold copy of `world` without its points, sharing its tiles.
    pub fn copy_of(world: &mut World) -> World {
        let mut snapshot = world.snapshot();
        snapshot.points.clear();
        snapshot
    }

    /// A point of a cold copy of the map (see [`SavePoint::copy_of`]), with
    /// its picture drawn: what a worker thread does.
    pub fn of(snapshot: World, name: impl Into<String>, created: u64) -> SavePoint {
        let thumbnail = crate::thumbnail::render(&snapshot, THUMBNAIL_SIZE).and_then(|img| {
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).ok()?;
            Some(Arc::<[u8]>::from(out))
        });
        SavePoint {
            name: name.into(),
            created,
            world: Arc::new(snapshot),
            thumbnail,
        }
    }

    /// The tiles of `dim`'s ground and other levels whose blob differs from
    /// those of `other` (by content), with the tiles only one of them has.
    pub fn changed_tiles(a: &World, b: &World, dim: usize) -> Vec<TilePos> {
        let (Some(da), Some(db)) = (a.dims.get(dim), b.dims.get(dim)) else {
            return Vec::new();
        };
        let mut out = std::collections::HashSet::new();
        let blobs = |level: &crate::level::Level| -> HashMap<TilePos, Blob> { level.blobs().unwrap_or_default().into_iter().collect() };
        for la in &da.levels {
            let ta = blobs(la);
            match db.levels.iter().find(|l| l.id == la.id) {
                Some(lb) => {
                    let tb = blobs(lb);
                    for (pos, blob) in &ta {
                        if tb.get(pos).is_none_or(|other| !same_blob(blob, other)) {
                            out.insert(*pos);
                        }
                    }
                    out.extend(tb.keys().filter(|p| !ta.contains_key(p)));
                }
                None => out.extend(ta.keys()),
            }
        }
        for lb in db.levels.iter().filter(|l| !da.levels.iter().any(|la| la.id == l.id)) {
            out.extend(lb.positions());
        }
        for (pos, chunk) in da.voxels.iter() {
            if db.voxels.get(pos) != Some(&**chunk) {
                out.insert(TilePos::containing(pos.x * 16, pos.z * 16));
            }
        }
        for (pos, _) in db.voxels.iter() {
            if da.voxels.get(pos).is_none() {
                out.insert(TilePos::containing(pos.x * 16, pos.z * 16));
            }
        }
        let mut out: Vec<TilePos> = out.into_iter().collect();
        out.sort_unstable_by_key(|p| (p.z, p.x));
        out
    }
}

/// Whether two blobs hold the same tile: the same allocation, or the same
/// bytes.
pub fn same_blob(a: &Blob, b: &Blob) -> bool {
    Arc::ptr_eq(a, b) || a[..] == b[..]
}

/// The current time, in seconds since 1970 (UTC).
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}
