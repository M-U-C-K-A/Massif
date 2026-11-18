//! Where a map and its contributions are put for others: a *depot* (see
//! `docs/versioning.md`). Massif speaks to one through [`Depot`], whatever
//! it is: a shared folder for now (Google Drive, Dropbox, iCloud or a Git
//! working copy), a server later, without the files or the steps changing.
//!
//! A folder depot:
//!
//! ```text
//! depot.json                    the map it holds, who keeps it, the version
//!                               published, the contributions merged
//! <map>.massif                  the map as published: only its owner writes it
//! contributions/*.massifpatch   a file per contribution, never rewritten
//! ```
//!
//! Each file has one writer: the owner writes `depot.json` and the map, a
//! contributor only adds files to `contributions/`. A sync service never
//! has two versions of one file to choose from.

use crate::{FormatError, Result};
use massif_core::World;
use massif_core::patch::{Manifest, Patch, VersionRef};
use massif_core::versions::{Id, SavePoint};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file that makes a folder a depot.
pub const DEPOT_FILE: &str = "depot.json";
/// The folder of the contributions.
pub const CONTRIBUTIONS: &str = "contributions";
const FORMAT: &str = "massif-depot/1";

/// What a depot says of itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepotInfo {
    pub format: String,
    pub map: Id,
    /// The map's name when the depot was made: its file's name.
    pub name: String,
    /// Who keeps the map, as they named themselves.
    #[serde(default)]
    pub owner: String,
    /// The version published: what contributors start from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<VersionRef>,
    /// The contributions merged into the map (or set aside), by identifier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<Id>,
}

/// A contribution in a depot.
#[derive(Debug, Clone)]
pub struct Waiting {
    pub path: PathBuf,
    pub manifest: Manifest,
    pub bytes: u64,
}

/// What Massif asks of a depot.
pub trait Depot {
    /// What it is, in a few words: "Shared folder Alpes".
    fn describe(&self) -> String;
    fn info(&self) -> &DepotInfo;
    /// The map as published, as a file to open, if it was published.
    fn published_map(&self) -> Option<PathBuf>;
    /// Publish `world` (cold) as `version` of the map, the one contributors
    /// start from.
    fn publish(&mut self, world: &World, version: &SavePoint) -> Result<()>;
    /// Add a contribution. Returns where it went.
    fn send(&self, patch: &Patch) -> Result<PathBuf>;
    /// The contributions not merged yet, oldest first.
    fn waiting(&self) -> Result<Vec<Waiting>>;
    /// Remember contribution `id` as merged (or set aside).
    fn mark_merged(&mut self, id: Id) -> Result<()>;
}

/// A depot in a folder shared by other means.
#[derive(Debug, Clone)]
pub struct FolderDepot {
    root: PathBuf,
    info: DepotInfo,
}

/// A name a file system takes.
fn file_safe(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || " -_'(),.".contains(c) { c } else { '_' }).collect();
    let safe = safe.trim().trim_matches('.').to_owned();
    if safe.is_empty() { "map".into() } else { safe }
}

impl FolderDepot {
    /// The depot in folder `root`.
    pub fn open(root: &Path) -> Result<FolderDepot> {
        let text = std::fs::read(root.join(DEPOT_FILE))?;
        if text.len() > 16 << 20 {
            return Err(FormatError::Corrupt("depot.json is too large".into()));
        }
        let info: DepotInfo = serde_json::from_slice(&text)?;
        if info.format != FORMAT {
            return Err(FormatError::Corrupt("depot.json was written by a newer version of Massif".into()));
        }
        Ok(FolderDepot { root: root.to_path_buf(), info })
    }

    /// Make folder `root` a depot of `world` (which must have its
    /// identifier), kept by `owner`. A folder that is a depot already is
    /// opened instead, if it holds this map.
    pub fn create(root: &Path, world: &World, owner: &str) -> Result<FolderDepot> {
        let map = world.meta.map_id.ok_or_else(|| FormatError::Corrupt("the map has no identifier yet: make a version of it first".into()))?;
        if root.join(DEPOT_FILE).exists() {
            let depot = FolderDepot::open(root)?;
            if depot.info.map != map {
                return Err(FormatError::Corrupt(format!("that folder holds another map, “{}”", depot.info.name)));
            }
            return Ok(depot);
        }
        std::fs::create_dir_all(root.join(CONTRIBUTIONS))?;
        let depot = FolderDepot {
            root: root.to_path_buf(),
            info: DepotInfo {
                format: FORMAT.into(),
                map,
                name: world.meta.name.clone(),
                owner: owner.into(),
                published: None,
                merged: Vec::new(),
            },
        };
        depot.write_info()?;
        Ok(depot)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn write_info(&self) -> Result<()> {
        let json = serde_json::to_vec_pretty(&self.info)?;
        massif_core::atomic::write_with(&self.root.join(DEPOT_FILE), true, |out| out.write_all(&json))?;
        Ok(())
    }

    fn map_path(&self) -> PathBuf {
        self.root.join(format!("{}.{}", file_safe(&self.info.name), crate::EXTENSION))
    }
}

impl Depot for FolderDepot {
    fn describe(&self) -> String {
        self.root.file_name().map_or_else(|| self.root.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    fn info(&self) -> &DepotInfo {
        &self.info
    }

    fn published_map(&self) -> Option<PathBuf> {
        let path = self.map_path();
        (self.info.published.is_some() && path.exists()).then_some(path)
    }

    fn publish(&mut self, world: &World, version: &SavePoint) -> Result<()> {
        if world.meta.map_id != Some(self.info.map) {
            return Err(FormatError::Corrupt(format!("the depot holds another map, “{}”", self.info.name)));
        }
        crate::save(world, &self.map_path())?;
        self.info.published = Some(VersionRef { id: version.id, name: version.name.clone() });
        self.write_info()
    }

    fn send(&self, patch: &Patch) -> Result<PathBuf> {
        if patch.manifest.map != self.info.map {
            return Err(FormatError::Corrupt(format!("the depot holds another map, “{}”", self.info.name)));
        }
        let dir = self.root.join(CONTRIBUTIONS);
        std::fs::create_dir_all(&dir)?;
        let who = if patch.manifest.author.is_empty() { "contribution".to_owned() } else { file_safe(&patch.manifest.author) };
        // Never over another file: a contribution is never rewritten.
        let mut path = dir.join(format!("{who} {}.{}", patch.manifest.id.short(), massif_core::patch::EXTENSION));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{who} {} ({n}).{}", patch.manifest.id.short(), massif_core::patch::EXTENSION));
            n += 1;
        }
        crate::save_patch(patch, &path)?;
        Ok(path)
    }

    fn waiting(&self) -> Result<Vec<Waiting>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.root.join(CONTRIBUTIONS)) else { return Ok(out) };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case(massif_core::patch::EXTENSION)) {
                continue;
            }
            // A file a sync service is still bringing, or one that is not a
            // contribution, is left for later.
            let Ok(manifest) = crate::patch::read_manifest(&path) else { continue };
            if manifest.map != self.info.map || self.info.merged.contains(&manifest.id) {
                continue;
            }
            let bytes = entry.metadata().map_or(0, |m| m.len());
            out.push(Waiting { path, manifest, bytes });
        }
        out.sort_by_key(|w| (w.manifest.created, w.path.clone()));
        Ok(out)
    }

    fn mark_merged(&mut self, id: Id) -> Result<()> {
        if !self.info.merged.contains(&id) {
            self.info.merged.push(id);
            self.write_info()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, generate};
    use massif_core::patch::Kind;

    #[test]
    fn a_folder_holds_the_map_and_its_contributions() {
        let root = std::env::temp_dir().join(format!("massif-depot-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        // A map without a version has no identifier yet.
        assert!(FolderDepot::create(&root, &world, "Hugo").is_err());
        let base = SavePoint::new(&mut world, "Published", 1);
        world.points.push(base.clone());
        let mut depot = FolderDepot::create(&root, &world, "Hugo").unwrap();
        assert!(depot.published_map().is_none());
        depot.publish(&world.snapshot(), &base).unwrap();
        let published = depot.published_map().expect("the map is published");
        let copy = crate::load(&published).unwrap();
        assert_eq!(copy.meta.map_id, world.meta.map_id);
        assert_eq!(copy.points.len(), 1, "with its versions, for contributors to start from one");
        // Opened again, the same.
        let again = FolderDepot::open(&root).unwrap();
        assert_eq!(again.info(), depot.info());
        assert_eq!(again.info().published.as_ref().unwrap().id, base.id);
        // A contributor sends two; the owner sees them waiting, merges one.
        let mut theirs = copy.clone();
        theirs.dims[0].ground_mut().insert(massif_core::TilePos::new(5, 5), massif_core::Tile::ground(70.0, 1, -1));
        let a = Patch::since(&base, &theirs.snapshot(), Kind::Contribution, "Jack", "One", 2).unwrap();
        let b = Patch::since(&base, &theirs.snapshot(), Kind::Contribution, "Jack", "Two", 3).unwrap();
        let pa = depot.send(&a).unwrap();
        depot.send(&b).unwrap();
        assert!(pa.starts_with(root.join(CONTRIBUTIONS)));
        let waiting = depot.waiting().unwrap();
        assert_eq!(waiting.iter().map(|w| w.manifest.message.as_str()).collect::<Vec<_>>(), ["One", "Two"]);
        depot.mark_merged(a.manifest.id).unwrap();
        let waiting = FolderDepot::open(&root).unwrap().waiting().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].manifest.id, b.manifest.id);
        // Another map's contribution is refused; a folder of another map too.
        let mut other = generate(&GenParams { tiles_x: 1, tiles_z: 1, ..GenParams::default() });
        let other_base = SavePoint::new(&mut other, "base", 1);
        let foreign = Patch::since(&other_base, &other.snapshot(), Kind::Contribution, "", "", 2).unwrap();
        assert!(depot.send(&foreign).is_err());
        assert!(FolderDepot::create(&root, &other, "Hugo").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
