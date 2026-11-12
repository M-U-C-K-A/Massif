//! The open document: a world, its undo history and where it is saved.

use massif_core::history::History;
use massif_core::level::LevelKind;
use massif_core::world::Dimension;
use massif_core::{TilePos, World};
use std::collections::HashSet;
use std::path::PathBuf;

/// Decoded tiles kept per level before the least recently edited ones are
/// compressed again. About 100 KiB each, so this bounds memory near 200 MiB
/// per level however large the map is.
const MAX_HOT_TILES: usize = 2048;

pub struct Document {
    pub world: World,
    pub history: History,
    pub path: Option<PathBuf>,
    /// Index of the dimension being edited.
    pub dim: usize,
    /// Index of the level tools act on.
    pub active_level: usize,
    /// Incremented by every change.
    pub revision: u64,
    saved_revision: u64,
    /// Changes whenever a different world is loaded: caches keyed on it must
    /// be dropped.
    pub generation: u64,
}

impl Document {
    pub fn new(world: World, path: Option<PathBuf>, generation: u64) -> Self {
        Self {
            world,
            history: History::default(),
            path,
            dim: 0,
            active_level: 0,
            revision: 0,
            saved_revision: 0,
            generation,
        }
    }

    pub fn dimension(&self) -> &Dimension {
        &self.world.dims[self.dim]
    }

    pub fn dimension_mut(&mut self) -> &mut Dimension {
        &mut self.world.dims[self.dim]
    }

    pub fn is_modified(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_changed(&mut self) {
        self.revision += 1;
    }

    /// Record that revision `revision` is what the file on disk holds.
    pub fn mark_saved(&mut self, revision: u64, path: PathBuf) {
        self.saved_revision = revision;
        self.path = Some(path);
    }

    /// "World name · Surface", as shown in the title bar.
    pub fn title(&self) -> String {
        format!("{} · {}", self.world.meta.name, self.dimension().kind.label())
    }

    #[cfg(test)]
    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unsaved".into())
    }

    pub fn active_kind(&self) -> LevelKind {
        self.dimension().levels[self.active_level].kind
    }

    /// Keep `active_level` valid after levels were added, removed or undone.
    pub fn clamp_active_level(&mut self) {
        let n = self.dimension().levels.len();
        if self.active_level >= n {
            self.active_level = n - 1;
        }
    }

    /// Tiles changed since the last call, across all levels of the active
    /// dimension.
    pub fn take_dirty(&mut self) -> Vec<TilePos> {
        let mut set = HashSet::new();
        let dim = &mut self.world.dims[self.dim];
        for level in &mut dim.levels {
            if level.has_dirty() {
                set.extend(level.take_dirty());
            }
        }
        if dim.voxels.has_dirty() {
            set.extend(dim.voxels.take_dirty());
        }
        set.into_iter().collect()
    }

    pub fn undo(&mut self) -> Option<String> {
        let label = self.history.undo(&mut self.world)?;
        self.after_history();
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let label = self.history.redo(&mut self.world)?;
        self.after_history();
        Some(label)
    }

    fn after_history(&mut self) {
        self.clamp_active_level();
        self.mark_changed();
    }

    /// Close the current undo step and bound memory use.
    pub fn commit(&mut self) {
        if self.history.commit() {
            self.mark_changed();
        }
        let dim = self.dim;
        self.world.dims[dim].trim(MAX_HOT_TILES);
    }

    /// Add a slab level above the stack and make it active.
    pub fn add_level(&mut self, kind: LevelKind) {
        let existing = self.dimension().levels.iter().filter(|l| l.kind == kind).count();
        let base = match kind {
            LevelKind::Solid => "Overhang",
            LevelKind::Carve => "Cave",
            LevelKind::Ground => return,
        };
        let name = format!("{base} {}", existing + 1);
        self.history.begin("Add level", self.dim);
        let index = self.history.edit(&mut self.world, 0).structure().add_level(name, kind);
        self.commit();
        self.active_level = index;
    }

    /// Give the whole map the biomes its land calls for (one undoable
    /// step). Returns how many columns changed.
    pub fn auto_biome(&mut self, rules: &massif_core::auto::AutoRules) -> usize {
        let reader = massif_core::auto::Reader::new(rules, &self.world.registry, self.world.meta.sea_level);
        self.history.begin("Auto biome", self.dim);
        let changed = massif_core::auto::biomes_over_the_map(&mut self.history.edit(&mut self.world, 0), &reader);
        self.commit();
        changed
    }

    /// Paint the whole map as its biomes call for (one undoable step).
    /// Returns how many columns changed.
    pub fn auto_paint(&mut self, rules: &massif_core::auto::AutoRules) -> usize {
        let reader = massif_core::auto::Reader::new(rules, &self.world.registry, self.world.meta.sea_level);
        self.history.begin("Auto paint", self.dim);
        let changed = massif_core::auto::paint_over_the_map(&mut self.history.edit(&mut self.world, 0), &reader);
        self.commit();
        changed
    }

    pub fn remove_level(&mut self, index: usize) {
        if index == 0 || index >= self.dimension().levels.len() {
            return;
        }
        self.history.begin("Delete level", self.dim);
        self.history.edit(&mut self.world, 0).structure().remove_level(index);
        self.commit();
        if self.active_level >= index {
            self.active_level -= 1;
        }
        self.clamp_active_level();
    }

    /// Move a slab level one step up (`+1`) or down (`-1`) the stack. The
    /// ground always stays at the bottom.
    pub fn move_level(&mut self, index: usize, delta: i32) {
        let n = self.dimension().levels.len();
        let target = index as i32 + delta;
        if index == 0 || target < 1 || target >= n as i32 {
            return;
        }
        let target = target as usize;
        self.history.begin("Reorder levels", self.dim);
        self.history.edit(&mut self.world, 0).structure().levels.swap(index, target);
        self.commit();
        if self.active_level == index {
            self.active_level = target;
        } else if self.active_level == target {
            self.active_level = index;
        }
    }

    pub fn rename_level(&mut self, index: usize, name: String) {
        if self.dimension().levels[index].name == name {
            return;
        }
        self.history.begin("Rename level", self.dim);
        self.history.edit(&mut self.world, 0).structure().levels[index].name = name;
        self.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, generate};

    fn doc() -> Document {
        let world = generate(&GenParams {
            tiles_x: 2,
            tiles_z: 2,
            ..GenParams::default()
        });
        Document::new(world, None, 1)
    }

    #[test]
    fn level_management_is_undoable_and_tracks_the_active_level() {
        let mut d = doc();
        assert!(!d.is_modified());
        d.add_level(LevelKind::Solid);
        d.add_level(LevelKind::Carve);
        d.add_level(LevelKind::Solid);
        let names: Vec<_> = d.dimension().levels.iter().map(|l| l.name.clone()).collect();
        assert_eq!(names, ["Ground", "Overhang 1", "Cave 1", "Overhang 2"]);
        assert_eq!(d.active_level, 3);
        assert!(d.is_modified());

        d.move_level(3, -1);
        assert_eq!((d.active_level, d.dimension().levels[2].name.as_str()), (2, "Overhang 2"));
        d.move_level(1, -1); // cannot go below the ground
        assert_eq!(d.dimension().levels[1].name, "Overhang 1");

        d.remove_level(1);
        assert_eq!((d.dimension().levels.len(), d.active_level), (3, 1));
        d.remove_level(0); // the ground stays
        assert_eq!(d.dimension().levels.len(), 3);

        d.rename_level(1, "Arche".into());
        assert_eq!(d.undo().as_deref(), Some("Rename level"));
        assert_eq!(d.undo().as_deref(), Some("Delete level"));
        assert_eq!(d.dimension().levels.len(), 4);
        while d.undo().is_some() {}
        assert_eq!((d.dimension().levels.len(), d.active_level), (1, 0));
    }

    #[test]
    fn saving_clears_the_modified_flag_for_that_revision_only() {
        let mut d = doc();
        d.add_level(LevelKind::Carve);
        let at_save = d.revision;
        d.add_level(LevelKind::Carve); // edited while the save was running
        d.mark_saved(at_save, PathBuf::from("/tmp/x.massif"));
        assert!(d.is_modified());
        d.mark_saved(d.revision, PathBuf::from("/tmp/x.massif"));
        assert!(!d.is_modified());
        assert_eq!(d.file_name(), "x.massif");
    }
}
