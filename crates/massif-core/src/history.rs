//! Undo / redo.
//!
//! A step remembers, for every tile an operation touched, the compressed blob
//! the tile had before. Blobs are shared `Arc`s and a few kilobytes each, so
//! a long history costs very little. Structural changes (adding, removing or
//! reordering levels) remember a cold snapshot of the whole level stack,
//! which again only clones `Arc`s.

use crate::coords::{BlockRect, TilePos};
use crate::level::{Blob, Level, LevelId, LevelKind, TileRef};
use crate::selection::Selection;
use crate::tile::Tile;
use crate::voxel::{ChunkPos, VoxelChunk};
use crate::world::{Dimension, World};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// By how many blocks the surface of each column of a tile went up or down
/// in an edit, by local index; `None` where it stayed (see
/// [`Edit::follow_ground`]).
type Moved = Vec<Option<i32>>;

#[derive(Debug)]
struct Step {
    /// Its number, for a stroke to know whether the step open is still its
    /// own (see [`History::open_step`]).
    id: u64,
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
    /// Pre-image of the selection, when the step changed it.
    selection: Option<Option<Arc<Selection>>>,
    /// Pre-image of the dimension's cave layers, when the step changed them.
    caves: Option<crate::caves::CaveLayers>,
    /// Pre-image of the structures placed, when the step changed them.
    placed: Option<Vec<Arc<crate::structures::Structure>>>,
    /// Pre-image of the map's settings alone (its spawn point moved).
    meta: Option<Box<crate::world::WorldMeta>>,
    /// Pre-image of the whole map, for a save point put back: its settings,
    /// blocks and dimensions (not its save points).
    whole: Option<Box<Whole>>,
    /// The bytes its pre-images hold, once measured (see [`Step::measure`]).
    size: usize,
}

/// The map but its save points, as a step to undo keeps it.
#[derive(Debug)]
struct Whole {
    meta: crate::world::WorldMeta,
    registry: crate::registry::Registry,
    dims: Vec<Dimension>,
}

impl Whole {
    /// Put `self` in place in `world` and return what was there.
    fn swap(self, world: &mut World) -> Whole {
        let mut dims = self.dims;
        for dim in &mut dims {
            for level in &mut dim.levels {
                level.mark_all_dirty();
            }
        }
        Whole {
            meta: std::mem::replace(&mut world.meta, self.meta),
            registry: std::mem::replace(&mut world.registry, self.registry),
            dims: std::mem::replace(&mut world.dims, dims),
        }
    }
}

impl Step {
    fn new(label: String, dim: usize) -> Self {
        Self {
            id: 0,
            label,
            dim,
            tiles: Vec::new(),
            seen: HashSet::new(),
            structure: None,
            voxels: Vec::new(),
            voxel_seen: HashSet::new(),
            selection: None,
            caves: None,
            placed: None,
            meta: None,
            whole: None,
            size: 0,
        }
    }

    /// Count the bytes its tiles' and sculpted chunks' pre-images hold.
    /// What is shared with the map still counts: a step's tiles are the
    /// map's no more once it is edited again.
    fn measure(mut self) -> Self {
        let tiles: usize = self.tiles.iter().map(|(_, _, b)| b.as_ref().map_or(0, |b| b.len())).sum();
        let voxels: usize = self.voxels.iter().map(|(_, c)| c.as_ref().map_or(0, |c| crate::voxel::VOLUME * if c.terrain.is_some() { 3 } else { 1 })).sum();
        self.size = tiles + voxels;
        self
    }

    fn is_empty(&self) -> bool {
        self.tiles.is_empty() && self.structure.is_none() && self.voxels.is_empty() && self.selection.is_none() && self.caves.is_none() && self.placed.is_none() && self.meta.is_none() && self.whole.is_none()
    }
}

/// The undo and redo stacks of one document.
#[derive(Debug)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    open: Option<Step>,
    limit: usize,
    /// The bytes the steps may hold together; past it the oldest go, the
    /// last one always kept.
    budget: usize,
    /// The layers brushes leave as they are (see [`crate::registry::Locks`]),
    /// set by whoever paints before they do.
    pub locks: crate::registry::Locks,
    /// The number the next step begun takes, and the label and dimension
    /// of the last one begun.
    next_id: u64,
    last: (String, usize),
}

/// What the steps to undo and redo may hold by default: on a map of 44,544
/// × 33,536 blocks, one step over the whole map holds about 1 GB.
pub const HISTORY_BYTES: usize = 1 << 30;

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
            budget: HISTORY_BYTES,
            locks: Default::default(),
            next_id: 1,
            last: (String::new(), 0),
        }
    }

    /// Hold at most `bytes` in the steps to undo and redo.
    pub fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.trim();
    }

    /// The bytes the steps hold.
    pub fn bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|s| s.size).sum()
    }

    /// Let the oldest steps go past the limit of steps or of bytes, keeping
    /// the last one; then steps to redo, the farthest first.
    fn trim(&mut self) {
        while self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        let mut held = self.bytes();
        while held > self.budget && self.undo.len() + self.redo.len() > 1 {
            let gone = if self.undo.len() > 1 || self.redo.is_empty() { self.undo.remove(0) } else { self.redo.remove(0) };
            held -= gone.size;
        }
    }

    /// Start recording a step. Any step still open is committed first.
    /// Returns its number (see [`History::open_step`]).
    pub fn begin(&mut self, label: impl Into<String>, dim: usize) -> u64 {
        self.commit();
        let label = label.into();
        let id = self.next_id;
        self.next_id += 1;
        self.last = (label.clone(), dim);
        self.open = Some(Step { id, ..Step::new(label, dim) });
        id
    }

    pub fn is_recording(&self) -> bool {
        self.open.is_some()
    }

    /// The number of the step open, if any. A stroke keeps the one it
    /// began: anything else may close it under the stroke's feet (an undo,
    /// a selection, a task's result put in place), and the stroke then
    /// begins another rather than edit with no step open.
    pub fn open_step(&self) -> Option<u64> {
        self.open.as_ref().map(|s| s.id)
    }

    /// Edit `level` of the dimension the open step was begun on.
    ///
    /// Every edit must be undoable: with no step open, a bug, debug builds
    /// (and the tests) stop here; a build for use begins a step, as the
    /// last one begun was labelled, and says so in the log, rather than
    /// lose the map.
    pub fn edit<'a>(&'a mut self, world: &'a mut World, level: usize) -> Edit<'a> {
        if self.open.is_none() {
            debug_assert!(false, "History::begin was not called");
            log::warn!("an edit with no step open: a step “{}” is begun for it", self.last.0);
            let (label, dim) = self.last.clone();
            self.begin(label, dim.min(world.dims.len().saturating_sub(1)));
        }
        let step = self.open.as_mut().expect("begun above");
        let sea = world.meta.sea_level.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16;
        let dim = &mut world.dims[step.dim];
        assert!(level < dim.levels.len(), "no such level");
        Edit { dim, step, level, locks: &self.locks, sea }
    }

    /// Undo the open step, as far as it went, and drop it: after an error
    /// in the middle of a stroke, the map as it was before it, whatever the
    /// stroke left half done (tiles taken out to edit included). Returns
    /// whether a step was open.
    pub fn abandon(&mut self, world: &mut World) -> bool {
        let Some(step) = self.open.take() else {
            return false;
        };
        swap(step, world);
        true
    }

    /// A copy of `world` as the last step committed left it: the open
    /// step's edits, if any, taken back on the copy (see [`World::copy_unfrozen`]);
    /// `world` stays as it is.
    pub fn copy_committed(&self, world: &World) -> World {
        let mut copy = world.copy_unfrozen();
        let Some(step) = &self.open else {
            return copy;
        };
        if let Some(whole) = &step.whole {
            copy.meta = whole.meta.clone();
            copy.registry = whole.registry.clone();
            copy.dims = whole.dims.clone();
            return copy;
        }
        if let Some(meta) = &step.meta {
            copy.meta = (**meta).clone();
        }
        let Some(dim) = copy.dims.get_mut(step.dim) else {
            return copy;
        };
        if let Some(selection) = &step.selection {
            dim.selection = selection.clone();
        }
        if let Some(caves) = &step.caves {
            dim.caves = caves.clone();
        }
        if let Some(placed) = &step.placed {
            dim.structures = placed.clone();
        }
        for (pos, state) in step.voxels.iter().rev() {
            dim.voxels.restore(*pos, state.clone());
        }
        if let Some(levels) = &step.structure {
            dim.replace_levels(levels.clone());
            return copy;
        }
        for (id, pos, state) in step.tiles.iter().rev() {
            if let Some(index) = dim.level_index(*id) {
                dim.levels[index].restore(*pos, state.clone());
            }
        }
        copy
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
        self.undo.push(step.measure());
        self.trim();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// How many steps can be undone.
    pub fn undo_count(&self) -> usize {
        self.undo.len()
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
        self.redo.push(swap(step, world).measure());
        Some(label)
    }

    /// Redo the last undone step. Returns its label.
    pub fn redo(&mut self, world: &mut World) -> Option<String> {
        self.commit();
        let step = self.redo.pop()?;
        let label = step.label.clone();
        self.undo.push(swap(step, world).measure());
        Some(label)
    }

    /// Make `selection` the selection of dimension `dim`, as one step to
    /// undo labelled `label`. An empty selection is none. Returns whether
    /// it changed.
    pub fn select(&mut self, world: &mut World, dim: usize, label: impl Into<String>, selection: Option<Selection>) -> bool {
        let selection = selection.filter(|s| !s.is_empty()).map(Arc::new);
        let Some(d) = world.dims.get_mut(dim) else { return false };
        if d.selection.as_deref() == selection.as_deref() {
            return false;
        }
        self.begin(label, dim);
        let step = self.open.as_mut().expect("just begun");
        step.selection = Some(std::mem::replace(&mut d.selection, selection));
        self.commit()
    }

    /// Give dimension `dim` the cave layers `caves` (a layer added, its
    /// settings changed), as one step to undo labelled `label`. Returns
    /// whether they changed.
    pub fn set_caves(&mut self, world: &mut World, dim: usize, label: impl Into<String>, caves: crate::caves::CaveLayers) -> bool {
        let Some(d) = world.dims.get_mut(dim) else { return false };
        if d.caves == caves {
            return false;
        }
        self.begin(label, dim);
        let step = self.open.as_mut().expect("just begun");
        step.caves = Some(std::mem::replace(&mut d.caves, caves));
        self.commit()
    }

    /// Give the map the settings `meta` (its spawn point moved), as one
    /// step to undo labelled `label`, its tiles untouched. Returns whether
    /// they changed.
    pub fn set_meta(&mut self, world: &mut World, label: impl Into<String>, meta: crate::world::WorldMeta) -> bool {
        if world.meta == meta {
            return false;
        }
        self.begin(label, 0);
        let step = self.open.as_mut().expect("just begun");
        step.meta = Some(Box::new(std::mem::replace(&mut world.meta, meta)));
        self.commit()
    }

    /// Put the map back as `point` holds it (its settings, blocks and
    /// dimensions; the map keeps its name and its save points), as one step
    /// to undo labelled `label`.
    pub fn restore_point(&mut self, world: &mut World, point: &World, label: impl Into<String>) {
        self.begin(label, 0);
        let mut meta = point.meta.clone();
        meta.name = world.meta.name.clone();
        // A point made before the map had its identifier has none.
        meta.map_id = world.meta.map_id.or(meta.map_id);
        let whole = Whole {
            meta,
            registry: point.registry.clone(),
            dims: point.dims.clone(),
        };
        // The map as it is, cold, to undo it.
        let before = world.snapshot();
        world.dims = before.dims;
        let step = self.open.as_mut().expect("just begun");
        step.whole = Some(Box::new(whole.swap(world)));
        self.commit();
    }

    /// Put the map's settings, registry and dimensions in place of its own
    /// (a contribution merged into it, say), as one step to undo labelled
    /// `label`. The map keeps its save points.
    pub fn replace_world(&mut self, world: &mut World, meta: crate::world::WorldMeta, registry: crate::registry::Registry, dims: Vec<Dimension>, label: impl Into<String>) {
        self.begin(label, 0);
        let whole = Whole { meta, registry, dims };
        // The map as it is, cold, to undo it.
        let before = world.snapshot();
        world.dims = before.dims;
        let step = self.open.as_mut().expect("just begun");
        step.whole = Some(Box::new(whole.swap(world)));
        self.commit();
    }

    /// Put `dimension` in place of the map's dimension `dim` (the map made
    /// another size, say), as one step to undo labelled `label`: undone, the
    /// whole map is back as it was.
    pub fn replace_dimension(&mut self, world: &mut World, dim: usize, dimension: Dimension, label: impl Into<String>) {
        self.replace_dimension_and_settings(world, dim, dimension, None, label);
    }

    /// The same, the map given the settings and blocks `settings` too (its
    /// terrains made of another palette's blocks, say), in the same step.
    pub fn replace_dimension_and_settings(&mut self, world: &mut World, dim: usize, dimension: Dimension, settings: Option<(crate::world::WorldMeta, crate::registry::Registry)>, label: impl Into<String>) {
        self.begin(label, dim);
        // The map as it is, cold, to undo it.
        let before = world.snapshot();
        world.dims = before.dims;
        let mut dims = world.dims.clone();
        dims[dim] = dimension;
        let (meta, registry) = settings.unwrap_or_else(|| (world.meta.clone(), world.registry.clone()));
        let whole = Whole { meta, registry, dims };
        let step = self.open.as_mut().expect("just begun");
        step.whole = Some(Box::new(whole.swap(world)));
        self.commit();
    }

    /// Give the map the settings `meta` and the blocks and terrains
    /// `registry` (its terrains made of other blocks, say), as one step to
    /// undo labelled `label`; its dimensions stay as they are.
    pub fn replace_settings(&mut self, world: &mut World, meta: crate::world::WorldMeta, registry: crate::registry::Registry, label: impl Into<String>) {
        self.begin(label, 0);
        // The map as it is, cold, to undo it: its tiles shared, not copied.
        let before = world.snapshot();
        world.dims = before.dims;
        let whole = Whole { meta, registry, dims: world.dims.clone() };
        let step = self.open.as_mut().expect("just begun");
        step.whole = Some(Box::new(whole.swap(world)));
        self.commit();
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
    if let Some(whole) = step.whole {
        inverse.whole = Some(Box::new(whole.swap(world)));
        return inverse;
    }
    if let Some(meta) = step.meta {
        inverse.meta = Some(Box::new(std::mem::replace(&mut world.meta, *meta)));
    }
    let Some(dim) = world.dims.get_mut(step.dim) else {
        return inverse;
    };
    if let Some(selection) = step.selection {
        inverse.selection = Some(std::mem::replace(&mut dim.selection, selection));
    }
    if let Some(caves) = step.caves {
        inverse.caves = Some(std::mem::replace(&mut dim.caves, caves));
    }
    if let Some(placed) = step.placed {
        inverse.placed = Some(std::mem::replace(&mut dim.structures, placed));
    }
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
    caves: &'a crate::caves::CaveLayers,
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
        let mut stack = crate::compose::TileStack::gather(self.min_y, self.max_y, self.voxels, pos, layers);
        stack.with_cave_layers(self.caves, pos, crate::compose::Caves::All, &[]);
        Some(stack)
    }
}

/// Recorded, mutable access to one level of a dimension.
pub struct Edit<'a> {
    dim: &'a mut Dimension,
    step: &'a mut Step,
    level: usize,
    locks: &'a crate::registry::Locks,
    /// The world's sea level (its water's default level).
    sea: i16,
}

impl Edit<'_> {
    /// The world's sea level: the level of the water a new tile has.
    pub fn sea_level(&self) -> i16 {
        self.sea
    }

    pub fn dim(&self) -> &Dimension {
        self.dim
    }

    pub fn level_index(&self) -> usize {
        self.level
    }

    /// The layers brushes leave as they are.
    pub fn locks(&self) -> &crate::registry::Locks {
        self.locks
    }

    /// Give the dimension the cave layers `caves`, in the step (its first
    /// layers kept to undo).
    pub fn set_caves(&mut self, caves: crate::caves::CaveLayers) {
        if self.dim.caves == caves {
            return;
        }
        let before = std::mem::replace(&mut self.dim.caves, caves);
        self.step.caves.get_or_insert(before);
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

    /// Decode tiles of the edited level about to be read many times (see
    /// [`Level::warm`]).
    pub fn warm(&mut self, positions: &[TilePos]) {
        self.dim.levels[self.level].warm(positions);
    }

    /// Keep at most `max_hot` tiles of the edited level decoded: long passes
    /// over a big map cool it down as they go.
    pub fn cool(&mut self, max_hot: usize) {
        self.dim.levels[self.level].trim(max_hot);
    }

    /// Drop what was sculpted in 3D over `tiles`, recorded for undo: the
    /// ground under it was written anew, and the sculpture belonged to the
    /// old one. Returns how many chunks went.
    pub fn clear_voxels(&mut self, tiles: &[TilePos]) -> usize {
        let chunks: Vec<ChunkPos> = tiles
            .iter()
            .flat_map(|t| self.dim.voxels.in_tile(*t).map(|(p, _)| p).collect::<Vec<_>>())
            .collect();
        for &pos in &chunks {
            self.touch_voxel(pos);
            self.dim.voxels.remove(pos);
        }
        chunks.len()
    }

    /// Put whole tiles in place, in their compressed form (worked out on a
    /// copy of the map), recorded for undo.
    pub fn replace_tiles(&mut self, tiles: Vec<(TilePos, crate::level::Blob)>) {
        let positions: Vec<TilePos> = tiles.iter().map(|t| t.0).collect();
        self.touch_all(&positions);
        let level = &mut self.dim.levels[self.level];
        for (pos, blob) in tiles {
            level.insert_blob(pos, blob);
        }
    }

    /// The tile at `pos` of the edited level as it was when this step
    /// began, if this step has changed it (`None` too where the level had
    /// no tile there).
    pub fn original(&self, pos: TilePos) -> Option<Tile> {
        let id = self.dim.levels[self.level].id;
        let blob = self.step.tiles.iter().find(|t| t.0 == id && t.1 == pos)?.2.as_ref()?;
        Tile::decode(blob).ok()
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
        {
            profiling::scope!("edit: undo copies");
            level.encode_blobs(&fresh);
        }
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
        profiling::scope!("edit: tiles");
        let positions: Vec<TilePos> = positions
            .into_iter()
            .filter(|p| create || self.level().contains(*p))
            .collect();
        if positions.is_empty() {
            return;
        }
        self.touch_all(&positions);
        // Tiles with sculpted chunks over them: where their columns move,
        // the chunks follow (see `follow_ground`), from what the levels
        // composed under those the edit reaches before it.
        let sculpted: HashSet<TilePos> = positions.iter().copied().filter(|p| self.dim.voxels.has_tile(*p)).collect();
        let under: HashMap<ChunkPos, VoxelChunk> = if sculpted.is_empty() {
            HashMap::new()
        } else {
            let size = crate::voxel::SIZE;
            let reached: Vec<ChunkPos> = sculpted
                .iter()
                .flat_map(|t| self.dim.voxels.in_tile(*t).map(|(p, _)| p).collect::<Vec<_>>())
                .filter(|p| {
                    let (x0, _, z0) = p.origin();
                    x0 < rect.x1 && x0 + size > rect.x0 && z0 < rect.z1 && z0 + size > rect.z0
                })
                .collect();
            crate::voxel::bare_densities(self.dim, &reached).into_iter().collect()
        };
        // Only the ground's surface carries what was sculpted over it up or
        // down; a level's own surfaces leave it where it is.
        let ground = self.level == 0;
        let moved: Vec<(TilePos, Moved)> = {
            let (min_y, max_y) = (self.dim.min_y, self.dim.max_y);
            let (voxels, caves) = (&self.dim.voxels, &self.dim.caves);
            let (before, rest) = self.dim.levels.split_at_mut(self.level);
            let (level, after) = rest.split_first_mut().expect("the edited level exists");
            let others = Others {
                before: &*before,
                after: &*after,
                voxels,
                caves,
                min_y,
                max_y,
            };
            let taken: Vec<_> = positions
                .into_iter()
                .filter_map(|pos| level.check_out(pos, create).map(|t| (pos, t)))
                .collect();
            let edited: Vec<(TilePos, Box<Tile>, Option<Moved>)> = taken
                .into_par_iter()
                .map(|(pos, taken)| {
                    let mut tile = taken.into_tile();
                    let old = sculpted.contains(&pos).then(|| (tile.top.clone(), tile.bottom.clone()));
                    f(pos, rect.intersect(&pos.rect()), &mut tile, &others);
                    // Each column that moved, and by how many blocks its
                    // surface went up or down.
                    let moved = old.map(|(top, bottom)| {
                        (0..crate::coords::TILE_AREA)
                            .map(|k| {
                                let (was, is) = (top.get(k), tile.top.get(k));
                                let changed = was != is || bottom.as_ref().map(|b| b.get(k)) != tile.bottom.as_ref().map(|b| b.get(k));
                                changed.then(|| if ground { crate::compose::block_top(is) - crate::compose::block_top(was) } else { 0 })
                            })
                            .collect::<Moved>()
                    });
                    (pos, tile, moved)
                })
                .collect();
            let mut moved = Vec::new();
            for (pos, tile, m) in edited {
                level.check_in(pos, tile);
                if let Some(m) = m.filter(|m| m.iter().any(Option::is_some)) {
                    moved.push((pos, m));
                }
            }
            moved
        };
        for (pos, m) in moved {
            self.follow_ground(pos, &m, &under);
        }
    }

    /// The sculpted chunks over `tile` follow its ground where it moved
    /// (`moved`, by local index: by how many blocks each column's surface
    /// went up or down, `None` where it stayed). What was sculpted in such a
    /// column, what a chunk holds beyond what the levels composed there
    /// before (`under`), goes up or down with the surface, onto what they
    /// compose now: an overhang, a cave or a smoothed slope keeps its shape
    /// as the brushes of the map move the land under it, and the land they
    /// moved shows around it (a chunk stands for its whole 16³ cube: the
    /// ground changed under it did not show there, and the brush seemed to
    /// do nothing). Recorded for undo; a chunk left holding nothing of its
    /// own goes.
    fn follow_ground(&mut self, tile: TilePos, moved: &[Option<i32>], under: &HashMap<ChunkPos, VoxelChunk>) {
        use crate::voxel::SIZE;
        let shift = |x: i32, z: i32| moved[crate::coords::local_index(x, z)];
        // The stacks of chunks over moved columns: the heights of their
        // chunks, and the most their columns moved.
        let mut stacks: std::collections::BTreeMap<(i32, i32), (Vec<i32>, i32)> = std::collections::BTreeMap::new();
        for (p, _) in self.dim.voxels.in_tile(tile) {
            let (x0, _, z0) = p.origin();
            let reach = (0..SIZE * SIZE).filter_map(|k| shift(x0 + k % SIZE, z0 + k / SIZE)).map(i32::abs).max();
            if let Some(reach) = reach {
                let stack = stacks.entry((p.x, p.z)).or_default();
                stack.0.push(p.y);
                stack.1 = stack.1.max(reach);
            }
        }
        if stacks.is_empty() {
            return;
        }
        // The chunks the sculpture lands in: its own, and those its column's
        // surface takes it into, within the height of the world.
        let (bottom, top) = (self.dim.min_y.div_euclid(SIZE), (self.dim.max_y - 1).div_euclid(SIZE));
        let span = |(ys, reach): &(Vec<i32>, i32)| {
            let k = (reach + SIZE - 1) / SIZE;
            let (a, b) = (ys.iter().min().copied().unwrap_or(0), ys.iter().max().copied().unwrap_or(0));
            ((a - k).max(bottom), (b + k).min(top))
        };
        let reached: Vec<ChunkPos> = stacks.iter().flat_map(|(&(cx, cz), s)| {
            let (a, b) = span(s);
            (a..=b).map(move |cy| ChunkPos::new(cx, cy, cz))
        }).collect();
        let now: HashMap<ChunkPos, VoxelChunk> = crate::voxel::bare_densities(self.dim, &reached).into_iter().collect();
        for (&(cx, cz), stack) in &stacks {
            let (a, b) = span(stack);
            let base = a * SIZE;
            let mut chunks: Vec<(ChunkPos, Option<VoxelChunk>)> = (a..=b).map(|cy| {
                let pos = ChunkPos::new(cx, cy, cz);
                (pos, self.dim.voxels.get(pos).cloned())
            }).collect();
            let mut changed = vec![false; chunks.len()];
            let (x0, z0) = (cx * SIZE, cz * SIZE);
            // A column's sculpture, bottom up: its density over what the
            // levels composed, and the mark of its matter (see
            // `VoxelChunk::mark`).
            let mut was: Vec<(i16, u16)> = Vec::new();
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let Some(s) = shift(x0 + lx, z0 + lz) else { continue };
                    was.clear();
                    for (pos, chunk) in &chunks {
                        for ly in 0..SIZE {
                            was.push(match chunk {
                                Some(c) => {
                                    // Under a chunk the edit did not reach,
                                    // what is composed now.
                                    let bare = under.get(pos).or_else(|| now.get(pos)).map_or(0, |u| u.get(lx, ly, lz));
                                    (c.get(lx, ly, lz) as i16 - bare as i16, c.mark(lx, ly, lz))
                                }
                                None => (0, crate::voxel::FOLLOW),
                            });
                        }
                    }
                    for (k, (pos, chunk)) in chunks.iter_mut().enumerate() {
                        let Some(bare) = now.get(pos) else { continue };
                        for ly in 0..SIZE {
                            let y = pos.y * SIZE + ly;
                            let (d, mark) = usize::try_from(y - s - base).ok().and_then(|i| was.get(i).copied()).unwrap_or((0, crate::voxel::FOLLOW));
                            let density = (bare.get(lx, ly, lz) as i16 + d).clamp(0, 255) as u8;
                            let c = match chunk {
                                Some(c) => c,
                                None if d == 0 && mark == crate::voxel::FOLLOW => continue,
                                None => chunk.insert(bare.clone()),
                            };
                            if c.get(lx, ly, lz) != density || c.mark(lx, ly, lz) != mark {
                                c.set(lx, ly, lz, density);
                                c.set_mark(lx, ly, lz, mark);
                                changed[k] = true;
                            }
                        }
                    }
                }
            }
            for ((pos, chunk), changed) in chunks.into_iter().zip(changed) {
                if !changed {
                    continue;
                }
                self.touch_voxel(pos);
                match chunk {
                    Some(c) if now.get(&pos).is_some_and(|bare| c.is_bare(bare)) => {
                        self.dim.voxels.remove(pos);
                    }
                    Some(c) => self.dim.voxels.insert(pos, Arc::new(c)),
                    None => {}
                }
            }
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
        let made: Vec<(ChunkPos, VoxelChunk)> = crate::voxel::densities_of(self.dim, &missing);
        let bases: std::collections::HashMap<ChunkPos, VoxelChunk> = made.iter().cloned().collect();
        for (pos, chunk) in made {
            self.dim.voxels.insert(pos, Arc::new(chunk));
        }
        let mut taken = self.dim.voxels.take_out(positions);
        taken.par_iter_mut().for_each(|(pos, chunk)| f(*pos, Arc::make_mut(chunk)));
        self.dim.voxels.put_back(taken);
        // A chunk made here that `f` left as the levels compose it holds
        // nothing of its own: it goes (it would only stand in the way of
        // the ground changed under it later).
        for (pos, base) in &bases {
            if self.dim.voxels.get(*pos).is_some_and(|c| c.is_bare(base)) {
                self.dim.voxels.remove(*pos);
            }
        }
    }

    /// Take a sculpted chunk away (the levels compose its blocks again),
    /// recorded for undo.
    pub fn remove_voxels(&mut self, pos: ChunkPos) {
        if self.dim.voxels.get(pos).is_some() {
            self.touch_voxel(pos);
            self.dim.voxels.remove(pos);
        }
    }

    /// Put sculpted chunks in place, made elsewhere (from what
    /// [`crate::voxel::densities_of`] read here).
    pub fn set_voxels(&mut self, chunks: Vec<(ChunkPos, Arc<VoxelChunk>)>) {
        for (pos, chunk) in chunks {
            self.touch_voxel(pos);
            self.dim.voxels.insert(pos, chunk);
        }
    }

    /// The structures placed on the dimension, to add to or take from: what
    /// they were is kept to undo.
    pub fn structures_mut(&mut self) -> &mut Vec<Arc<crate::structures::Structure>> {
        if self.step.placed.is_none() {
            self.step.placed = Some(self.dim.structures.clone());
        }
        &mut self.dim.structures
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

    /// A dimension replaced by one of another size is one step: undone, the
    /// map has its tiles and size back; redone, the new size again.
    #[test]
    fn a_dimension_replaced_comes_back_on_undo() {
        let mut w = world();
        let mut h = History::default();
        let mut bigger = w.dims[0].snapshot();
        bigger.resize(BlockRect::new(-128, -128, 256, 256), || Tile::ground(10.0, 0, -1));
        h.replace_dimension(&mut w, 0, bigger, "resize");
        assert_eq!(w.dims[0].ground().len(), 9);
        assert_eq!(h.undo(&mut w).as_deref(), Some("resize"));
        assert_eq!(w.dims[0].ground().len(), 1);
        assert_eq!(w.dims[0].ground().view(POS).unwrap().top.get(0), 64.0);
        h.redo(&mut w);
        assert_eq!(w.dims[0].bounds(), Some((TilePos::new(-1, -1), TilePos::new(1, 1))));
    }

    /// Sculpture dropped with the ground it stood on comes back on undo.
    #[test]
    fn cleared_sculpture_comes_back_on_undo() {
        let mut w = world();
        let mut h = History::default();
        h.begin("sculpt", 0);
        let pos = crate::voxel::ChunkPos::new(0, 4, 0);
        h.edit(&mut w, 0).voxel_chunk_mut(pos).set(1, 2, 3, 0);
        h.commit();
        assert!(w.dims[0].voxels.has_tile(POS));
        h.begin("graph", 0);
        assert_eq!(h.edit(&mut w, 0).clear_voxels(&[POS]), 1);
        h.commit();
        assert!(!w.dims[0].voxels.has_tile(POS));
        h.undo(&mut w);
        assert!(w.dims[0].voxels.get(pos).is_some(), "the sculpture is back");
    }

    fn top(w: &World, cell: usize) -> f32 {
        w.dims[0].ground().view(POS).unwrap().top.get(cell)
    }

    #[test]
    fn the_history_keeps_within_its_bytes() {
        let mut w = crate::generate::generate(&crate::generate::GenParams { tiles_x: 4, tiles_z: 4, ..Default::default() });
        let mut h = History::default();
        let tiles: Vec<TilePos> = w.dims[0].levels[0].positions().collect();
        let step = |h: &mut History, w: &mut World, k: f32| {
            h.begin("raise", 0);
            for &p in &tiles {
                h.edit(w, 0).tile_mut(p).unwrap().top.set(0, 70.0 + k);
            }
            h.commit();
            w.dims[0].levels[0].encode_blobs(&tiles);
        };
        for k in 0..6 {
            step(&mut h, &mut w, k as f32);
        }
        assert_eq!(h.undo_count(), 6);
        let one = h.bytes() / 6;
        assert!(one > 0);
        // Room for two and a half steps: the oldest three go.
        h.set_budget(one * 5 / 2);
        assert_eq!(h.undo_count(), 2);
        assert!(h.bytes() <= one * 5 / 2);
        // Room for none: the last is kept, and undoes.
        h.set_budget(0);
        assert_eq!(h.undo_count(), 1);
        assert!(h.undo(&mut w).is_some());
        assert_eq!(w.dims[0].levels[0].view(tiles[0]).unwrap().top.get(0), 74.0);
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

    /// A map of four tiles at 64, each tile's encoded form.
    fn four_tiles() -> World {
        let mut w = World::new(WorldMeta::default());
        for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            w.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(64.0, 1, 63));
        }
        w
    }

    fn blobs(w: &mut World) -> Vec<(TilePos, crate::level::Blob)> {
        let ground = &mut w.dims[0].levels[0];
        ground.freeze();
        ground.blobs().expect("frozen")
    }

    /// A step's number tells a stroke whether the step open is still its
    /// own: another one begun, or an undo, closes it.
    #[test]
    fn steps_are_numbered() {
        let mut w = world();
        let mut h = History::default();
        let a = h.begin("Stroke", 0);
        assert_eq!(h.open_step(), Some(a));
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, 70.0);
        // Something else begins (a selection, a task's result): the
        // stroke's step is committed and another is open.
        let b = h.begin("Select", 0);
        assert_ne!(a, b);
        assert_eq!(h.open_step(), Some(b));
        h.commit();
        assert_eq!(h.open_step(), None);
        assert_eq!(h.undo_label(), Some("Stroke"));
    }

    /// An error in the middle of a stroke, while tiles are taken out to be
    /// edited on other threads: the map is short of them. A copy of the
    /// map as committed has them all, as before the stroke; abandoning
    /// the step puts the map itself back so.
    #[test]
    fn a_stroke_cut_short_by_an_error_is_taken_back() {
        let mut w = four_tiles();
        let before = blobs(&mut w);
        let mut h = History::default();
        h.begin("Raise", 0);
        h.edit(&mut w, 0).tile_mut(TilePos::new(0, 0)).unwrap().top.set(0, 80.0);
        let rect = BlockRect::new(0, 0, 256, 256);
        let cut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            h.edit(&mut w, 0).par_tiles([TilePos::new(1, 0), TilePos::new(0, 1), TilePos::new(1, 1)], rect, false, |pos, _, tile| {
                if pos == TilePos::new(1, 1) {
                    panic!("an error in the middle of the stroke");
                }
                tile.top.set(0, 90.0);
            });
        }));
        assert!(cut.is_err());
        assert!(w.dims[0].ground().positions().count() < 4, "tiles taken out and lost");
        let mut copy = h.copy_committed(&w);
        assert_eq!(blobs(&mut copy), before, "the copy is the map before the stroke");
        assert!(h.abandon(&mut w));
        assert_eq!(h.open_step(), None);
        assert_eq!(blobs(&mut w), before, "the map itself is back as before");
        assert!(!h.can_undo(), "nothing of it to undo");
        // With no step open, the copy is the map as it is.
        let mut copy = h.copy_committed(&w);
        assert_eq!(blobs(&mut copy), before);
    }

    /// An edit with no step open is a bug: the tests stop on it.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "History::begin was not called")]
    fn an_edit_with_no_step_stops_the_tests() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Raise", 0);
        h.commit();
        let _ = h.edit(&mut w, 0);
    }

    /// A build for use records such an edit in a step of its own, as the
    /// last one begun was labelled, rather than stop.
    #[test]
    #[cfg(not(debug_assertions))]
    fn an_edit_with_no_step_begins_one() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Smooth", 0);
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, 70.0);
        h.commit();
        h.edit(&mut w, 0).tile_mut(POS).unwrap().top.set(0, 75.0);
        assert!(h.open_step().is_some());
        h.commit();
        assert_eq!(h.undo_count(), 2);
        assert_eq!(h.undo_label(), Some("Smooth"));
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
