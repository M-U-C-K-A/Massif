//! Save points: the map as it was at a moment the user named ("before the
//! rivers"), kept with the map to go back to, open or compare.
//!
//! A point holds a cold copy of the world. Its tiles are the same shared
//! blobs as the map's until the map changes them, so a point costs only the
//! tiles changed since it was made, in memory as in the file (see
//! `massif_format`, which writes each distinct tile once).
//!
//! Save points are the map's *versions*: each has an [`Id`] and the ids of
//! the versions it was made from (its parents: one, or two for a merge),
//! so that maps copied to other people can be told apart and joined again
//! (see `docs/versioning.md`). The map itself knows its own [`Id`]
//! ([`crate::WorldMeta::map_id`]) and the version it stands on
//! ([`crate::WorldMeta::head`]).

use crate::coords::TilePos;
use crate::level::Blob;
use crate::world::World;
use std::collections::HashMap;
use std::sync::Arc;

/// A 128-bit identifier, of a map or of a version, written as 32
/// hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub [u8; 16]);

impl Id {
    /// A new identifier, from the system's random source (the clock, the
    /// process and a counter, hashed, should that fail).
    pub fn random() -> Id {
        let mut bytes = [0u8; 16];
        if getrandom::fill(&mut bytes).is_err() {
            static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Id::derived(&[&nanos.to_le_bytes(), &std::process::id().to_le_bytes(), &n.to_le_bytes()]);
        }
        Id(bytes)
    }

    /// The identifier `parts` always give: the first 16 bytes of their
    /// BLAKE3 hash (each part prefixed by its length).
    pub fn derived(parts: &[&[u8]]) -> Id {
        let mut h = blake3::Hasher::new();
        h.update(b"massif id");
        for part in parts {
            h.update(&(part.len() as u64).to_le_bytes());
            h.update(part);
        }
        let mut out = [0u8; 16];
        out.copy_from_slice(&h.finalize().as_bytes()[..16]);
        Id(out)
    }

    /// The 32 hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The identifier written by [`Id::hex`].
    pub fn parse(s: &str) -> Option<Id> {
        if s.len() != 32 || !s.is_ascii() {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Id(out))
    }

    /// The first 8 digits, enough to tell versions apart in a list.
    pub fn short(&self) -> String {
        self.hex()[..8].to_string()
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({})", self.hex())
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

impl serde::Serialize for Id {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> serde::Deserialize<'de> for Id {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Id, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Id::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not an identifier: {s:?}")))
    }
}

/// A named copy of the map: a version of it.
#[derive(Debug, Clone)]
pub struct SavePoint {
    /// Its identifier, the same in every copy of the map.
    pub id: Id,
    /// The versions it was made from: none for the first, one, or two for
    /// a merge (the map's first, then the one merged into it). A parent
    /// may be missing from the list (deleted, or another person's).
    pub parents: Vec<Id>,
    pub name: String,
    /// Who made it, as they named themselves; empty if unknown.
    pub author: String,
    /// The branch it was made on; empty for the map's own line.
    pub branch: String,
    /// When it was made, in seconds since 1970 (UTC).
    pub created: u64,
    /// The map as it was: cold, and without save points of its own. Its
    /// [`crate::WorldMeta::head`] is this point's id, so that putting the
    /// map back as it was puts it back on this version.
    pub world: Arc<World>,
    /// A small picture of it from above (a JPEG image).
    pub thumbnail: Option<Arc<[u8]>>,
}

/// The longest side of a save point's picture, in pixels.
pub const THUMBNAIL_SIZE: u32 = 160;

impl SavePoint {
    /// A point of `world` as it is now, named `name`: a cold copy that shares
    /// its tiles, and its picture.
    /// The map then stands on that version.
    pub fn new(world: &mut World, name: impl Into<String>, created: u64) -> SavePoint {
        let snapshot = Self::copy_of(world);
        let point = SavePoint::of(snapshot, name, created);
        world.meta.head = Some(point.id);
        point
    }

    /// A cold copy of `world` without its points, sharing its tiles. The
    /// map is given its identifier if it had none.
    pub fn copy_of(world: &mut World) -> World {
        world.map_id();
        let mut snapshot = world.snapshot();
        snapshot.points.clear();
        snapshot
    }

    /// A point of a cold copy of the map (see [`SavePoint::copy_of`]), with
    /// its picture drawn: what a worker thread does. Its parent is the
    /// version the copy stood on.
    pub fn of(mut snapshot: World, name: impl Into<String>, created: u64) -> SavePoint {
        let id = Id::random();
        let parents = snapshot.meta.head.into_iter().collect();
        snapshot.meta.head = Some(id);
        let thumbnail = crate::thumbnail::render(&snapshot, THUMBNAIL_SIZE).and_then(|img| {
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).ok()?;
            Some(Arc::<[u8]>::from(out))
        });
        SavePoint {
            id,
            parents,
            name: name.into(),
            author: String::new(),
            branch: String::new(),
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

/// Whether `world` (cold) is as version `point` holds it: the same
/// registry, and every dimension the same tiles and sculpted chunks.
pub fn unchanged_since(world: &World, point: &SavePoint) -> bool {
    let then = &point.world;
    world.registry == then.registry
        && world.dims.len() == then.dims.len()
        && world.dims.iter().zip(&then.dims).all(|(a, b)| a.kind == b.kind && a.levels.len() == b.levels.len())
        && (0..world.dims.len()).all(|d| SavePoint::changed_tiles(world, then, d).is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    #[test]
    fn ids_read_back_and_differ() {
        let a = Id::random();
        assert_eq!(Id::parse(&a.hex()), Some(a));
        assert_ne!(a, Id::random());
        assert_eq!(serde_json::from_str::<Id>(&serde_json::to_string(&a).unwrap()).unwrap(), a);
        assert_eq!(Id::parse("xyz"), None);
        assert_eq!(Id::parse(&"g".repeat(32)), None);
        assert_eq!(Id::derived(&[b"a", b"bc"]), Id::derived(&[b"a", b"bc"]));
        assert_ne!(Id::derived(&[b"a", b"bc"]), Id::derived(&[b"ab", b"c"]));
    }

    /// Each point stands on the one before; the map on the last; putting
    /// the map back as a point puts it on that point, and keeps the map's
    /// identifier.
    #[test]
    fn points_chain_into_versions() {
        let mut world = World::new(WorldMeta::default());
        assert!(world.meta.map_id.is_none());
        let first = SavePoint::new(&mut world, "first", 1);
        let map = world.meta.map_id.expect("given at the first point");
        assert!(first.parents.is_empty());
        assert_eq!(first.world.meta.head, Some(first.id));
        assert_eq!(first.world.meta.map_id, Some(map));
        world.points.push(first.clone());
        let second = SavePoint::new(&mut world, "second", 2);
        assert_eq!(second.parents, vec![first.id]);
        assert_eq!(world.meta.head, Some(second.id));
        world.points.push(second);
        let mut history = crate::history::History::default();
        history.restore_point(&mut world, &first.world, "Restore");
        assert_eq!(world.meta.head, Some(first.id));
        assert_eq!(world.meta.map_id, Some(map));
        let third = SavePoint::new(&mut world, "third", 3);
        assert_eq!(third.parents, vec![first.id]);
    }
}
