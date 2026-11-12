//! Undo / redo.
//!
//! A step remembers, for every tile an operation touched, the compressed blob
//! the tile had before. Blobs are shared `Arc`s and a few kilobytes each, so
//! a long history costs very little. Structural changes (adding, removing or
//! reordering levels) remember a cold snapshot of the whole level stack,
//! which again only clones `Arc`s.

use crate::coords::{BlockRect, TilePos};
use crate::level::{Blob, Level, LevelId, LevelKind, TileRef};
use crate::tile::Tile;
use crate::voxel::{ChunkPos, VoxelChunk};
use crate::world::{Dimension, World};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug)]
struct Step {
    label: String,
    dim: usize,
    /// Pre-images of the touched tiles, in the order they were first touched.
    tiles: Vec<(LevelId, TilePos, Option<Blob>)>,
    seen: HashSet<(LevelId, TilePos)>,
    /// Pre-image of the whole level stack, for structural changes.
    structure: Option<Vec<Level>>,
    /// Pre-images of the sculpted chunks touched (`None`: there was none).
    voxels: Vec<(ChunkPos, Option<Arc<VoxelChunk>>)>,
    voxel_seen: HashSet<ChunkPos>,
}

impl Step {
    fn new(label: String, dim: usize) -> Self {
        Self {
            label,
            dim,
            tiles: Vec::new(),
            seen: HashSet::new(),
            structure: None,
            voxels: Vec::new(),
            voxel_seen: HashSet::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.tiles.is_empty() && self.structure.is_none() && self.voxels.is_empty()
    }
}

/// The undo and redo stacks of one document.
#[derive(Debug)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    open: Option<Step>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
            limit: limit.max(1),
        }
    }

    /// Start recording a step. Any step still open is committed first.
    pub fn begin(&mut self, label: impl Into<String>, dim: usize) {
        self.commit();
        self.open = Some(Step::new(label.into(), dim));
    }

    pub fn is_recording(&self) -> bool {
        self.open.is_some()
    }

    /// Edit `level` of the dimension the open step was begun on.
    ///
    /// Panics if no step is open: every edit must be undoable.
    pub fn edit<'a>(&'a mut self, world: &'a mut World, level: usize) -> Edit<'a> {
        let step = self.open.as_mut().expect("History::begin was not called");
        let dim = &mut world.dims[step.dim];
        assert!(level < dim.levels.len(), "no such level");
        Edit { dim, step, level }
    }

    /// Close the open step. Returns whether it changed anything.
    pub fn commit(&mut self) -> bool {
        let Some(step) = self.open.take() else {
            return false;
        };
        if step.is_empty() {
            return false;
        }
        self.redo.clear();
        self.undo.push(step);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Undo the last step. Returns its label.
    pub fn undo(&mut self, world: &mut World) -> Option<String> {
        self.commit();
        let step = self.undo.pop()?;
        let label = step.label.clone();
        self.redo.push(swap(step, world));
        Some(label)
    }

    /// Redo the last undone step. Returns its label.
    pub fn redo(&mut self, world: &mut World) -> Option<String> {
        self.commit();
        let step = self.redo.pop()?;
        let label = step.label.clone();
        self.undo.push(swap(step, world));
        Some(label)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
    }
}

/// Apply a step's pre-images and return the step that reverses it.
fn swap(step: Step, world: &mut World) -> Step {
    let mut inverse = Step::new(step.label, step.dim);
    let Some(dim) = world.dims.get_mut(step.dim) else {
        return inverse;
    };
    for (pos, state) in step.voxels.into_iter().rev() {
        inverse.voxels.push((pos, dim.voxels.get_shared(pos)));
        dim.voxels.restore(pos, state);
    }
    if let Some(levels) = step.structure {
        inverse.structure = Some(dim.snapshot_levels());
        dim.replace_levels(levels);
        return inverse;
    }
    for (id, pos, state) in step.tiles.into_iter().rev() {
        if let Some(index) = dim.level_index(id) {
            let level = &mut dim.levels[index];
            inverse.tiles.push((id, pos, level.blob(pos)));
            level.restore(pos, state);
        }
    }
    inverse
}

/// Read access to the levels of a dimension other than the one being edited.
pub struct Others<'a> {
    before: &'a [Level],
    after: &'a [Level],
    voxels: &'a crate::voxel::VoxelLayer,
    min_y: i32,
    max_y: i32,
}

impl<'a> Others<'a> {
    /// Level `index` of the dimension, unless it is the edited one.
    pub fn get(&self, index: usize) -> Option<&'a Level> {
        if index < self.before.len() {
            return self.before.get(index);
        }
        index
            .checked_sub(self.before.len() + 1)
            .and_then(|i| self.after.get(i))
    }

    /// The columns of the dimension at `pos`, composed of the levels `keep`
    /// accepts and of the sculpted chunks. `edited` is the edited level's
    /// tile there, as it is now, with its kind (`None`: leave it out).
    pub fn stack<'t>(&'t self, pos: TilePos, edited: Option<(LevelKind, &'t Tile)>, keep: impl Fn(usize, &Level) -> bool) -> Option<crate::compose::TileStack<'t>> {
        let at = self.before.len();
        let mut layers = Vec::new();
        for index in 0..at + 1 + self.after.len() {
            if index == at {
                if let Some((kind, tile)) = edited {
                    layers.push((index, kind, TileRef::Hot(tile)));
                }
                continue;
            }
            let Some(level) = self.get(index) else { continue };
            if (index == 0 || keep(index, level))
                && let Some(tile) = level.view(pos)
            {
                layers.push((index, level.kind, tile));
            }
        }
        if layers.first().is_none_or(|l| l.1 != LevelKind::Ground) {
            return None;
        }
        Some(crate::compose::TileStack::gather(self.min_y, self.max_y, self.voxels, pos, layers))
    }
}

/// Recorded, mutable access to one level of a dimension.
pub struct Edit<'a> {
    dim: &'a mut Dimension,
    step: &'a mut Step,
    level: usize,
}

impl Edit<'_> {
    pub fn dim(&self) -> &Dimension {
        self.dim
    }

    pub fn level_index(&self) -> usize {
        self.level
    }

    pub fn level(&self) -> &Level {
        &self.dim.levels[self.level]
    }

    pub fn kind(&self) -> LevelKind {
        self.level().kind
    }

    /// Read a tile of the edited level.
    pub fn view(&self, pos: TilePos) -> Option<TileRef<'_>> {
        self.level().view(pos)
    }

    fn touch(&mut self, pos: TilePos) {
        if self.step.structure.is_some() {
            return; // the snapshot already covers every tile
        }
        let level = &mut self.dim.levels[self.level];
        if self.step.seen.insert((level.id, pos)) {
            let before = level.blob(pos);
            self.step.tiles.push((level.id, pos, before));
        }
    }

    /// Record the pre-images of every tile in `positions` not seen yet by
    /// this step, encoding them in parallel.
    fn touch_all(&mut self, positions: &[TilePos]) {
        if self.step.structure.is_some() {
            return;
        }
        let level = &mut self.dim.levels[self.level];
        let id = level.id;
        let fresh: Vec<TilePos> = positions
            .iter()
            .copied()
            .filter(|p| !self.step.seen.contains(&(id, *p)))
            .collect();
        level.encode_blobs(&fresh);
        for pos in fresh {
            if self.step.seen.insert((id, pos)) {
                let before = level.blob(pos);
                self.step.tiles.push((id, pos, before));
            }
        }
    }

    /// Edit the tiles at `positions` in parallel, recorded for undo. `f`
    /// gets each tile and the part of `rect` inside it. With `create`, blank
    /// tiles are made where the level has none; otherwise those positions
    /// are skipped.
    pub fn par_tiles(
        &mut self,
        positions: impl IntoIterator<Item = TilePos>,
        rect: BlockRect,
        create: bool,
        f: impl Fn(TilePos, BlockRect, &mut Tile) + Sync,
    ) {
        self.par_tiles_with(positions, rect, create, |pos, clip, tile, _| f(pos, clip, tile));
    }

    /// [`Edit::par_tiles`], with read access to the other levels of the
    /// dimension.
    pub fn par_tiles_with(
        &mut self,
        positions: impl IntoIterator<Item = TilePos>,
        rect: BlockRect,
        create: bool,
        f: impl Fn(TilePos, BlockRect, &mut Tile, &Others<'_>) + Sync,
    ) {
        use rayon::prelude::*;
        let positions: Vec<TilePos> = positions
            .into_iter()
            .filter(|p| create || self.level().contains(*p))
            .collect();
        if positions.is_empty() {
            return;
        }
        self.touch_all(&positions);
        let (min_y, max_y) = (self.dim.min_y, self.dim.max_y);
        let voxels = &self.dim.voxels;
        let (before, rest) = self.dim.levels.split_at_mut(self.level);
        let (level, after) = rest.split_first_mut().expect("the edited level exists");
        let others = Others {
            before: &*before,
            after: &*after,
            voxels,
            min_y,
            max_y,
        };
        let taken: Vec<_> = positions
            .into_iter()
            .filter_map(|pos| level.check_out(pos, create).map(|t| (pos, t)))
            .collect();
        let edited: Vec<(TilePos, Box<Tile>)> = taken
            .into_par_iter()
            .map(|(pos, taken)| {
                let mut tile = taken.into_tile();
                f(pos, rect.intersect(&pos.rect()), &mut tile, &others);
                (pos, tile)
            })
            .collect();
        for (pos, tile) in edited {
            level.check_in(pos, tile);
        }
    }

    /// Edit an existing tile of the level.
    pub fn tile_mut(&mut self, pos: TilePos) -> Option<&mut Tile> {
        if !self.level().contains(pos) {
            return None;
        }
        self.touch(pos);
        self.dim.levels[self.level].tile_mut(pos)
    }

    /// Edit a tile of the level, creating a blank one if needed.
    pub fn tile_mut_or_insert(&mut self, pos: TilePos) -> &mut Tile {
        self.touch(pos);
        let level = &mut self.dim.levels[self.level];
        let slab = level.kind.has_bottom();
        level.tile_mut_or_insert(pos, || {
            if slab {
                Tile::empty_slab()
            } else {
                Tile::ground(0.0, 0, crate::tile::NO_FLUID)
            }
        })
    }

    /// Remove a tile of the level.
    pub fn remove_tile(&mut self, pos: TilePos) {
        if self.level().contains(pos) {
            self.touch(pos);
            self.dim.levels[self.level].remove(pos);
        }
    }

    /// Record the pre-image of a sculpted chunk the first time this step
    /// touches it.
    fn touch_voxel(&mut self, pos: ChunkPos) {
        if self.step.voxel_seen.insert(pos) {
            let before = self.dim.voxels.get_shared(pos);
            self.step.voxels.push((pos, before));
        }
    }

    /// Edit a sculpted chunk, recorded for undo. Where there is none yet, it
    /// starts as what the levels compose there.
    pub fn voxel_chunk_mut(&mut self, pos: ChunkPos) -> &mut VoxelChunk {
        self.touch_voxel(pos);
        if self.dim.voxels.get(pos).is_none() {
            let base = crate::voxel::densities_at(self.dim, pos);
            self.dim.voxels.insert(pos, Arc::new(base));
        }
        self.dim.voxels.get_mut(pos).expect("the chunk was just made")
    }

    /// Edit sculpted chunks in parallel, recorded for undo; missing ones
    /// start as what the levels compose there.
    pub fn par_voxels(&mut self, positions: &[ChunkPos], f: impl Fn(ChunkPos, &mut VoxelChunk) + Sync) {
        use rayon::prelude::*;
        for &pos in positions {
            self.touch_voxel(pos);
        }
        let missing: Vec<ChunkPos> = positions.iter().copied().filter(|p| self.dim.voxels.get(*p).is_none()).collect();
        let dim = &*self.dim;
        let made: Vec<(ChunkPos, VoxelChunk)> = missing
            .par_iter()
            .map(|&p| (p, crate::voxel::densities_at(dim, p)))
            .collect();
        for (pos, chunk) in made {
            self.dim.voxels.insert(pos, Arc::new(chunk));
        }
        let mut taken = self.dim.voxels.take_out(positions);
        taken.par_iter_mut().for_each(|(pos, chunk)| f(*pos, Arc::make_mut(chunk)));
        self.dim.voxels.put_back(taken);
    }

    /// Drop slab tiles of the level that no longer hold any column.
    pub fn prune_empty(&mut self, candidates: impl IntoIterator<Item = TilePos>) {
        self.dim.levels[self.level].prune_empty(candidates);
    }

    /// Record the whole level stack, then hand out the dimension for a
    /// structural change (add / remove / reorder / rename levels).
    pub fn structure(&mut self) -> &mut Dimension {
        if self.step.structure.is_none() {
            self.step.structure = Some(self.dim.snapshot_levels());
            self.step.tiles.clear();
            self.step.seen.clear();
        }
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    const POS: TilePos = TilePos::new(0, 0);

    fn world() -> World {
        let mut w = World::new(WorldMeta::default());
        w.dims[0].ground_mut().insert(POS, Tile::ground(64.0, 1, 63));
        w
    }

    fn top(w: &World, cell: usize) -> f32 {
        w.dims[0].ground().view(POS).unwrap().top.get(cell)
    }

    #[test]
    fn undo_and_redo_restore_tiles_exactly() {
        let mut w = world();
        let mut h = History::default();

        h.begin("Raise", 0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(7, 80.0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(8, 81.0);
        assert!(h.commit());
        h.begin("Paint", 0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().terrain.set(7, 5);
        assert!(h.commit());

        assert_eq!(h.undo(&mut w).as_deref(), Some("Paint"));
        assert_eq!(w.dims[0].ground().view(POS).unwrap().terrain.get(7), 1);
        assert_eq!(top(&w, 7), 80.0);
        assert_eq!(h.undo(&mut w).as_deref(), Some("Raise"));
        assert_eq!((top(&w, 7), top(&w, 8)), (64.0, 64.0));
        assert!(!h.can_undo());

        assert_eq!(h.redo(&mut w).as_deref(), Some("Raise"));
        assert_eq!((top(&w, 7), top(&w, 8)), (80.0, 81.0));
        assert_eq!(h.redo(&mut w).as_deref(), Some("Paint"));
        assert_eq!(w.dims[0].ground().view(POS).unwrap().terrain.get(7), 5);
        assert!(!h.can_redo());
    }

    #[test]
    fn empty_steps_are_dropped_and_new_edits_clear_redo() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Nothing", 0);
        assert!(!h.commit());
        assert!(!h.can_undo());

        h.begin("A", 0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, 1.0);
        h.commit();
        h.undo(&mut w);
        assert!(h.can_redo());
        h.begin("B", 0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, 2.0);
        h.commit();
        assert!(!h.can_redo());
        assert_eq!(h.undo_label(), Some("B"));
    }

    #[test]
    fn created_and_removed_tiles_are_undone() {
        let mut w = world();
        let mut h = History::default();
        let far = TilePos::new(3, -2);

        h.begin("Create", 0);
        h.edit(&mut w, 0).tile_mut_or_insert(far).top.fill(70.0);
        h.commit();
        assert!(w.dims[0].ground().contains(far));
        h.undo(&mut w);
        assert!(!w.dims[0].ground().contains(far));
        h.redo(&mut w);
        assert_eq!(w.dims[0].ground().view(far).unwrap().top.get(0), 70.0);

        h.begin("Remove", 0);
        h.edit(&mut w, 0).remove_tile(far);
        h.commit();
        assert!(!w.dims[0].ground().contains(far));
        h.undo(&mut w);
        assert_eq!(w.dims[0].ground().view(far).unwrap().top.get(0), 70.0);
    }

    #[test]
    fn structural_changes_are_undone() {
        let mut w = world();
        let mut h = History::default();

        h.begin("Add level", 0);
        let idx = h
            .edit(&mut w, 0)
            .structure()
            .add_level("Island", LevelKind::Solid);
        h.commit();
        h.begin("Stamp", 0);
        {
            let mut e = h.edit(&mut w, idx);
            let t = e.tile_mut_or_insert(POS);
            t.top.set(0, 100.0);
            t.bottom.as_mut().unwrap().set(0, 90.0);
        }
        h.commit();
        h.begin("Delete level", 0);
        h.edit(&mut w, 0).structure().remove_level(idx);
        h.commit();
        assert_eq!(w.dims[0].levels.len(), 1);

        h.undo(&mut w); // the level and its tile are back
        assert_eq!(w.dims[0].levels.len(), 2);
        assert!(w.dims[0].levels[1].view(POS).unwrap().present(0));
        h.undo(&mut w); // the stamp is gone
        assert!(!w.dims[0].levels[1].contains(POS));
        h.undo(&mut w); // the level is gone
        assert_eq!(w.dims[0].levels.len(), 1);
        h.redo(&mut w);
        h.redo(&mut w);
        assert!(w.dims[0].levels[1].view(POS).unwrap().present(0));
    }

    #[test]
    fn history_is_bounded() {
        let mut w = world();
        let mut h = History::new(3);
        for i in 0..10 {
            h.begin(format!("step {i}"), 0);
            h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, i as f32);
            h.commit();
        }
        let mut n = 0;
        while h.undo(&mut w).is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert_eq!(top(&w, 0), 6.0);
    }
}
