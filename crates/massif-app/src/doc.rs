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
pub const MAX_HOT_TILES: usize = 2048;

/// How much of the map lies at each height: a sample of its columns (32 ×
/// 32 of each tile, of at most 4096 tiles spread over the map), in whole
/// blocks, for the brush's height limits; and the map cut through its
/// highest column, to draw them over.
#[derive(Debug, Clone, Default)]
pub struct Heights {
    /// The revision and generation of the document it was read at.
    pub revision: u64,
    pub generation: u64,
    /// The lowest height sampled, and how many columns are at each height
    /// from there.
    pub lo: i32,
    pub counts: Vec<u32>,
    /// Of those, the columns under water.
    pub wet: Vec<u32>,
    pub total: u64,
    /// The map cut through its highest column, from west to east: the
    /// height of the land at even steps across the map (the mean of the
    /// columns sampled in each), NaN where there is none.
    pub section: Vec<f32>,
}

impl Heights {
    /// The highest height sampled.
    pub fn hi(&self) -> i32 {
        self.lo + self.counts.len() as i32 - 1
    }

    /// The share of the map (0 to 1) where `filter`'s height tests let the
    /// brush act.
    pub fn share(&self, filter: &massif_core::filter::Filter) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let sum: f64 = self.counts.iter().enumerate().map(|(k, &n)| n as f64 * filter.height_weight((self.lo + k as i32) as f32) as f64).sum();
        (sum / self.total as f64) as f32
    }
}

/// Reading the map's heights away from the frames (see
/// [`Document::heights`]).
#[derive(Default)]
struct HeightsJob {
    ready: Option<std::sync::Arc<Heights>>,
    running: Option<std::sync::mpsc::Receiver<Heights>>,
    started: Option<std::time::Instant>,
}

/// A tile's columns as sampled: their height in whole blocks and whether
/// they are under water; and the highest of them, its height and its z.
type TileSample = (Vec<(i32, bool)>, Option<(f32, i32)>);

/// [`Heights`] of `dim`, from a sample of its columns.
fn sample_heights(dim: &Dimension, revision: u64, generation: u64) -> Heights {
    use massif_core::coords::{TILE_SIZE, local_index};
    use rayon::prelude::*;
    let positions: Vec<TilePos> = dim.ground().positions().collect();
    let step = positions.len().div_ceil(4096).max(1);
    let picked: Vec<TilePos> = positions.iter().copied().step_by(step).collect();
    let samples: Vec<TileSample> = picked
        .par_iter()
        .map(|&t| {
            let mut out = Vec::with_capacity(1024);
            let mut highest: Option<(f32, i32)> = None;
            if let Some(stack) = massif_core::compose::TileStack::new(dim, t, false) {
                let mut sampler = crate::map_render::ColumnSampler::default();
                for j in (2..TILE_SIZE).step_by(4) {
                    for i in (2..TILE_SIZE).step_by(4) {
                        let s = sampler.sample(&stack, local_index(t.x * TILE_SIZE + i, t.z * TILE_SIZE + j));
                        if !s.is_empty() {
                            out.push((massif_core::compose::block_top(s.height), s.fluid != massif_core::NO_FLUID));
                            if highest.is_none_or(|(h, _)| s.height > h) {
                                highest = Some((s.height, t.z * TILE_SIZE + j));
                            }
                        }
                    }
                }
            }
            (out, highest)
        })
        .collect();
    let mut heights = Heights {
        revision,
        generation,
        ..Heights::default()
    };
    let (lo, hi) = samples.iter().flat_map(|s| &s.0).fold((i32::MAX, i32::MIN), |(lo, hi), &(y, _)| (lo.min(y), hi.max(y)));
    if lo > hi {
        return heights;
    }
    heights.lo = lo;
    heights.counts = vec![0; (hi - lo + 1) as usize];
    heights.wet = vec![0; (hi - lo + 1) as usize];
    for &(y, wet) in samples.iter().flat_map(|s| &s.0) {
        let k = (y - lo) as usize;
        heights.counts[k] += 1;
        heights.wet[k] += wet as u32;
        heights.total += 1;
    }
    // The map cut from west to east through its highest column: at most
    // 1024 steps across it, each the mean of the columns every few blocks
    // in it.
    if let Some(z) = samples.iter().filter_map(|s| s.1).max_by(|a, b| a.0.total_cmp(&b.0)).map(|h| h.1) {
        let (west, east) = positions.iter().fold((i32::MAX, i32::MIN), |(w, e), p| (w.min(p.x), e.max(p.x)));
        let (x0, width) = (west * TILE_SIZE, (east - west + 1) * TILE_SIZE);
        let n = (width / 2).clamp(1, 1024) as usize;
        let stretch = width as f32 / n as f32;
        let every = ((stretch / 4.0) as usize).max(1);
        let row: Vec<TilePos> = positions.iter().copied().filter(|p| p.z == z.div_euclid(TILE_SIZE)).collect();
        let parts: Vec<Vec<(usize, f32)>> = row
            .par_iter()
            .map(|&t| {
                let mut out = Vec::new();
                if let Some(stack) = massif_core::compose::TileStack::new(dim, t, false) {
                    let mut sampler = crate::map_render::ColumnSampler::default();
                    for i in (0..TILE_SIZE).step_by(every) {
                        let x = t.x * TILE_SIZE + i;
                        let s = sampler.sample(&stack, local_index(x, z));
                        if !s.is_empty() {
                            let k = (((x - x0) as f32 / stretch) as usize).min(n - 1);
                            out.push((k, s.height));
                        }
                    }
                }
                out
            })
            .collect();
        let mut sum = vec![(0.0f64, 0u32); n];
        for &(k, h) in parts.iter().flatten() {
            sum[k].0 += h as f64;
            sum[k].1 += 1;
        }
        heights.section = sum.into_iter().map(|(h, n)| if n > 0 { (h / n as f64) as f32 } else { f32::NAN }).collect();
    }
    heights
}

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
    /// The terrain the map was filled with, as of a revision (see
    /// [`massif_core::auto::base_terrain`]).
    auto_base: std::sync::Mutex<Option<(u64, Option<massif_core::TerrainId>)>>,
    /// Tiles each level keeps decoded (the memory limit of the settings).
    pub max_hot_tiles: usize,
    /// The heights of the map, read away from the frames.
    heights: HeightsJob,
    /// The name of the save point this map is, opened read-only: changes
    /// are taken back, and saving asks for a new file.
    pub read_only: Option<String>,
    /// What the Gradient brush learnt of the ground before it painted,
    /// kept from stroke to stroke: painting it again where it painted
    /// measures from the boundary as it first was (see
    /// [`massif_core::gradient::Sides`]).
    pub gradient_sides: Option<massif_core::gradient::Sides>,
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
            auto_base: std::sync::Mutex::new(None),
            gradient_sides: None,
            max_hot_tiles: MAX_HOT_TILES,
            heights: HeightsJob::default(),
            read_only: None,
        }
    }

    /// How much of the map lies at each height, from a sample of it read
    /// away from the frames: read again a moment after the map changes
    /// (not in the middle of a stroke). `None` until a first reading.
    pub fn heights(&mut self, ctx: &egui::Context) -> Option<std::sync::Arc<Heights>> {
        let job = &mut self.heights;
        if let Some(results) = &job.running {
            match results.try_recv() {
                Ok(h) => {
                    job.ready = Some(std::sync::Arc::new(h));
                    job.running = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => job.running = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let current = job.ready.as_ref().is_some_and(|h| h.revision == self.revision && h.generation == self.generation);
        let due = job.ready.is_none() || job.started.is_none_or(|t| t.elapsed().as_secs_f32() > 1.5);
        if !current && due && job.running.is_none() && !self.history.is_recording() {
            let dim = self.world.dims[self.dim].snapshot();
            let (revision, generation) = (self.revision, self.generation);
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("massif heights".into()).spawn(move || {
                let _ = tx.send(sample_heights(&dim, revision, generation));
                ctx.request_repaint();
            });
            if spawned.is_ok() {
                job.running = Some(rx);
                job.started = Some(std::time::Instant::now());
            }
        }
        job.ready.clone()
    }

    /// The terrain the map was filled with, if one covers nearly all of it:
    /// the automatic tools read the land through it. Worked out again once
    /// the map has changed.
    pub fn auto_base(&self) -> Option<massif_core::TerrainId> {
        let mut cache = self.auto_base.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((revision, base)) = *cache
            && revision == self.revision
        {
            return base;
        }
        let base = massif_core::auto::base_terrain(self.dimension(), &self.world.registry);
        *cache = Some((self.revision, base));
        base
    }

    /// The map's own terrain: the one it was filled with (or chosen for it
    /// since), else one other than grass that covers nearly all of it. What
    /// Paint terrain's right button puts back; `None`: the height chose the
    /// terrains.
    pub fn map_terrain(&self) -> Option<massif_core::TerrainId> {
        self.world.meta.fill.or_else(|| self.auto_base())
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
    /// Take the map as it is now for one with nothing to save (a sample
    /// just made): closing it asks nothing until it is changed.
    pub fn forget_changes(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn mark_saved(&mut self, revision: u64, path: PathBuf) {
        self.saved_revision = revision;
        self.path = Some(path);
    }

    /// "World name · Surface", as shown in the title bar.
    pub fn title(&self) -> String {
        match &self.read_only {
            Some(point) => tr!("{name} · {point} (read-only)", name = self.world.meta.name, point),
            None => format!("{} · {}", self.world.meta.name, crate::i18n::tr_name(self.dimension().kind.label())),
        }
    }

    #[cfg(test)]
    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("unsaved").into())
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
        self.dim = self.dim.min(self.world.dims.len() - 1);
        self.clamp_active_level();
        self.mark_changed();
    }

    /// Put the map back as save point `index` holds it, as one step to undo.
    /// Returns its name.
    pub fn restore_point(&mut self, index: usize) -> Option<String> {
        let point = self.world.points.get(index)?;
        let (name, then) = (point.name.clone(), point.world.clone());
        self.history.restore_point(&mut self.world, &then, tr!("Restore “{name}”", name));
        self.after_history();
        Some(name)
    }

    /// Put `dimension` in place of dimension `dim` (the map made another
    /// size), as one step to undo named `label`.
    pub fn replace_dimension(&mut self, dim: usize, dimension: massif_core::world::Dimension, label: &str) {
        self.history.replace_dimension(&mut self.world, dim, dimension, label);
        self.after_history();
    }

    /// Take back every change made to a map open read-only. Returns whether
    /// there was one.
    pub fn take_back(&mut self) -> bool {
        if self.read_only.is_none() || self.history.is_recording() || !self.history.can_undo() {
            return false;
        }
        while self.history.undo(&mut self.world).is_some() {}
        self.history.clear();
        self.after_history();
        self.saved_revision = self.revision;
        true
    }

    /// The selection of the dimension shown: the part of it the tools
    /// change (`None`: all of it).
    pub fn selection(&self) -> Option<&std::sync::Arc<massif_core::selection::Selection>> {
        self.dimension().selection.as_ref()
    }

    /// Make `selection` the selection, as one step to undo named `label`.
    /// Returns whether it changed.
    pub fn select(&mut self, label: &str, selection: Option<massif_core::selection::Selection>) -> bool {
        let changed = self.history.select(&mut self.world, self.dim, label, selection);
        if changed {
            self.mark_changed();
        }
        changed
    }

    /// After an error in a frame: the step open, if any, taken back as far
    /// as it went (a stroke cut short, tiles it had taken out to edit
    /// included), so that the map is as the last step left it. Returns
    /// whether one was open.
    pub fn abandon_step(&mut self) -> bool {
        self.history.abandon(&mut self.world)
    }

    /// Close the current undo step and bound memory use.
    pub fn commit(&mut self) {
        if self.history.commit() {
            self.mark_changed();
        }
        let dim = self.dim;
        self.world.dims[dim].trim(self.max_hot_tiles);
    }

    /// Add a slab level above the stack and make it active.
    pub fn add_level(&mut self, kind: LevelKind) {
        let existing = self.dimension().levels.iter().filter(|l| l.kind == kind).count();
        let base = match kind {
            LevelKind::Solid => tr!("Overhang"),
            LevelKind::Carve => tr!("Cave"),
            LevelKind::Ground => return,
        };
        let name = format!("{base} {}", existing + 1);
        self.history.begin(tr!("Add level"), self.dim);
        let index = self.history.edit(&mut self.world, 0).structure().add_level(name, kind);
        self.commit();
        self.active_level = index;
    }

    /// Give the whole map the biomes its land calls for (one undoable
    /// step), here and now; the app runs it in the background
    /// ([`crate::jobs::Job::auto`]). Returns how many columns changed.
    #[cfg(test)]
    pub fn auto_biome(&mut self, rules: &massif_core::auto::AutoRules) -> usize {
        let base = self.auto_base();
        let reader = massif_core::auto::Reader::new(rules, &self.world.registry, self.world.meta.sea_level).with_base(base);
        self.history.begin(tr!("Auto biome"), self.dim);
        let changed = massif_core::auto::biomes_over_the_map(&mut self.history.edit(&mut self.world, 0), &reader);
        self.commit();
        changed
    }

    /// Paint the whole map as its biomes call for (one undoable step), here
    /// and now. Returns how many columns changed.
    #[cfg(test)]
    pub fn auto_paint(&mut self, rules: &massif_core::auto::AutoRules) -> usize {
        let base = self.auto_base();
        let reader = massif_core::auto::Reader::new(rules, &self.world.registry, self.world.meta.sea_level).with_base(base);
        self.history.begin(tr!("Auto paint"), self.dim);
        let changed = massif_core::auto::paint_over_the_map(&mut self.history.edit(&mut self.world, 0), &reader);
        self.commit();
        changed
    }

    pub fn remove_level(&mut self, index: usize) {
        if index == 0 || index >= self.dimension().levels.len() {
            return;
        }
        self.history.begin(tr!("Delete level"), self.dim);
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
        self.history.begin(tr!("Reorder levels"), self.dim);
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
        self.history.begin(tr!("Rename level"), self.dim);
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
