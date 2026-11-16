//! The graph's filters over the whole map: what Edit ▸ Whole map does.
//!
//! The surface of the map is read into a field, as a Map node reads it; a
//! filter of the graph — erosion, thermal erosion, a blur, terraces, lakes,
//! rivers — changes it; and the change goes back into the ground, column by
//! column. Between the samples of a map read coarser than its blocks the
//! change is interpolated, so that the ground keeps its own detail. What
//! lies over the ground — the rock of other levels, what was sculpted in
//! 3D — is left as it is, and so is the ground's paint, but for the beds
//! the rivers dig.

use crate::selection::IN;
use super::map::{reads_level, sample_map};
use super::{Extent, Field, NO_WATER, NodeKind, Value, ops};
use crate::auto::{Changes, Steps};
use crate::codec::snap_height;
use crate::compose::hash2;
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::history::{Edit, Others};
use crate::level::LevelKind;
use crate::registry::TerrainId;
use crate::tile::Tile;
use crate::world::Dimension;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Samples a side the map is read at, at most: one a block up to this,
/// coarser beyond (the simulations work at this size anyway).
pub const WHOLE_MAX: usize = 4096;

/// Tiles written at a time, and kept decoded after each batch.
const BATCH: usize = 256;
const KEEP_HOT: usize = 2048;

/// What a pass over the whole map does to its relief.
#[derive(Debug, Clone, PartialEq)]
pub enum Reshape {
    /// A filter of the graph, with its settings: a node that takes heights
    /// and gives heights ([`Reshape::runs`]).
    Filter(NodeKind),
    /// The relief scaled around the sea, `scale` times as high, then moved
    /// up (or down) by `shift` blocks.
    Relief { scale: f32, shift: f32 },
}

impl Reshape {
    /// Whether a whole-map pass can run `kind`.
    pub fn runs(kind: &NodeKind) -> bool {
        matches!(
            kind,
            NodeKind::Erosion { .. }
                | NodeKind::Thermal { .. }
                | NodeKind::Blur { .. }
                | NodeKind::Sharpen { .. }
                | NodeKind::Terrace { .. }
                | NodeKind::Stratify { .. }
                | NodeKind::Lakes { .. }
                | NodeKind::Rivers { .. }
        )
    }

    /// Its name, for the history and what is said when it is done.
    pub fn label(&self) -> &'static str {
        match self {
            Reshape::Filter(kind) => kind.label(),
            Reshape::Relief { .. } => "Relief",
        }
    }
}

/// How the heights change: by what a filter changed at each sample, or
/// column by column.
enum Change {
    Samples {
        extent: Extent,
        before: Arc<Field>,
        after: Arc<Field>,
        /// The level of the water the filter lays ([`NO_WATER`]: none).
        water: Option<Arc<Field>>,
        /// Where rivers run (0..1), to paint their beds.
        beds: Option<Arc<Field>>,
    },
    Columns(Box<dyn Fn(f32) -> f32 + Sync + Send>),
}

/// The map's tiles, as a rectangle of blocks.
fn map_rect(edit: &Edit) -> Option<BlockRect> {
    let (lo, hi) = edit.level().bounds()?;
    Some(BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE))
}

/// The square the map is read over, `n` samples a side at most.
fn extent_of(rect: BlockRect, n: usize) -> Extent {
    let side = rect.width().max(rect.depth()).max(1) as usize;
    Extent::covering(rect, side.clamp(2, n))
}

/// Run the filter `kind` over heights read over `extent` (and the map's
/// water, for rivers); `progress` hears how far it is. `None`: cancelled.
fn filter(kind: &NodeKind, extent: &Extent, heights: Arc<Field>, level: Arc<Field>, cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Option<Vec<Arc<Field>>> {
    let mut inputs = vec![Some(Value::Field(heights))];
    if matches!(kind, NodeKind::Rivers { .. }) {
        inputs.push(Some(Value::Field(level)));
    }
    // The heights, and for lakes and rivers their mask and water.
    let wanted = match kind {
        NodeKind::Lakes { .. } => vec![true, false, true],
        NodeKind::Rivers { .. } => vec![true, true, true],
        _ => vec![true, false, false, false],
    };
    let out = ops::run(kind, extent, &inputs, &wanted, cancel, None, progress).ok()?;
    Some(out.iter().map(Value::as_field).collect())
}

/// The heights of the map's surface over `extent`, and the level of its
/// water; samples off the map lie under the sea.
fn read(dim: &Dimension, extent: &Extent, sea: i32) -> (Field, Field) {
    let read = sample_map(dim, extent, sea as f32 - 8.0, &[], &[]);
    (read.heights, read.level)
}

/// The lowest and highest heights of the map's own samples (those of its
/// tiles), for the filters that work over its range.
fn range_on_map(f: &Field, extent: &Extent, dim: &Dimension) -> (f32, f32) {
    let ground = dim.ground();
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for j in 0..f.n {
        for i in 0..f.n {
            let (x, z) = extent.world(i, j);
            if ground.contains(TilePos::containing(x.floor() as i32, z.floor() as i32)) {
                let h = f.at(i, j);
                lo = lo.min(h);
                hi = hi.max(h);
            }
        }
    }
    if lo > hi { (0.0, 1.0) } else { (lo, hi) }
}

/// The share of the work of [`change_of`] reading the map takes.
const READING: f32 = 0.15;

/// What `op` changes, worked out over the map read at `n` samples a side;
/// `progress` hears how far it is (0..1). `None`: cancelled.
fn change_of(dim: &Dimension, rect: BlockRect, op: &Reshape, sea: i32, n: usize, cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Option<Change> {
    let sea_f = sea as f32;
    match op {
        &Reshape::Relief { scale, shift } => Some(Change::Columns(Box::new(move |h| sea_f + (h - sea_f) * scale + shift))),
        Reshape::Filter(NodeKind::Terrace { steps, strength }) => {
            // Exact at every column: a function of its height alone.
            let extent = extent_of(rect, n.min(1024));
            let (heights, _) = read(dim, &extent, sea);
            let (lo, hi) = range_on_map(&heights, &extent, dim);
            let span = (hi - lo).max(1e-6);
            let k = (*steps).max(1) as f32;
            let s = strength.clamp(0.0, 1.0);
            Some(Change::Columns(Box::new(move |v| {
                let t = (v - lo) / span * k;
                let floor = t.floor();
                let frac = t - floor;
                let riser = (frac - 0.85).max(0.0) / 0.15;
                let stepped = (floor + riser * riser * (3.0 - 2.0 * riser)) / k;
                let t = t / k;
                lo + (t + (stepped - t) * s) * span
            })))
        }
        Reshape::Filter(kind) => {
            let extent = extent_of(rect, n);
            let (heights, level) = read(dim, &extent, sea);
            progress(READING);
            let before = Arc::new(heights);
            let out = filter(kind, &extent, before.clone(), Arc::new(level), cancel, &|f| progress(READING + (1.0 - READING) * f))?;
            let (water, beds) = match kind {
                NodeKind::Lakes { .. } => (out.get(2).cloned(), None),
                NodeKind::Rivers { .. } => (out.get(2).cloned(), out.get(1).cloned()),
                _ => (None, None),
            };
            Some(Change::Samples {
                extent,
                before,
                after: out[0].clone(),
                water,
                beds,
            })
        }
    }
}

/// The columns of a tile whose surface is the ground's — not covered by
/// the rock of another level or by what was sculpted in 3D; `None`: all.
fn ground_on_top(pos: TilePos, tile: &Tile, others: &Others<'_>) -> Option<Vec<bool>> {
    let stack = others.stack(pos, Some((LevelKind::Ground, tile)), |_, l| reads_level(l, &[]))?;
    if stack.is_ground_only() {
        return None;
    }
    let mut spans = Vec::new();
    Some(
        (0..TILE_SIZE as usize * TILE_SIZE as usize)
            .map(|i| {
                stack.spans(i, &mut spans);
                spans.last().is_some_and(|top| stack.surface_level(i, top) == Some(0))
            })
            .collect(),
    )
}

/// Run `op` over the ground of the whole map (the level `edit` works on
/// must be the ground). `sea` is the world's; `bed`, the terrain of the
/// beds rivers dig. Writes the heights, water and terrains of the columns
/// it changes; stops between tiles if `steps` is cancelled.
pub fn reshape_ground(edit: &mut Edit, op: &Reshape, sea: i32, bed: Option<TerrainId>, steps: &Steps) -> Changes {
    // Only the tiles of the selection, when there is one.
    let selection = edit.dim().selection.clone();
    let positions: Vec<TilePos> = edit.level().positions().filter(|p| selection.as_ref().is_none_or(|s| s.tile(*p).is_some())).collect();
    // Steps: three a tile for working out the change, one a tile for
    // writing it.
    let count = positions.len().max(1);
    steps.total.store(4 * count, Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let Some(rect) = map_rect(edit) else { return Changes::default() };
    let worked = |f: f32| steps.done.store((f.clamp(0.0, 1.0) * 3.0 * count as f32) as usize, Ordering::Relaxed);
    let Some(change) = change_of(edit.dim(), rect, op, sea, WHOLE_MAX, &steps.cancel, &worked) else {
        return Changes {
            cancelled: true,
            ..Changes::default()
        };
    };
    steps.done.store(3 * count, Ordering::Relaxed);
    let (min_y, max_y) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    let mut cancelled = false;
    for batch in positions.chunks(BATCH) {
        if steps.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        edit.warm(batch);
        let edited = Mutex::new(Vec::new());
        edit.par_tiles_with(batch.iter().copied(), everywhere, false, |pos, _, tile, others| {
            let on_top = ground_on_top(pos, tile, others);
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            let (ox, oz) = pos.origin();
            let mut n = 0;
            for lz in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (x, z) = (ox + lx, oz + lz);
                    let i = local_index(x, z);
                    if on_top.as_ref().is_some_and(|t| !t[i]) {
                        continue;
                    }
                    // Weighed by the selection: the change fades with it.
                    let weight = cells.map_or(IN, |c| c.get(i));
                    if weight == 0 {
                        continue;
                    }
                    let lets = |x, z| crate::selection::lets(weight, x, z);
                    let old = tile.top.get(i);
                    let mut moved = false;
                    let new = match &change {
                        Change::Columns(f) => f(old),
                        Change::Samples {
                            extent,
                            before,
                            after,
                            water,
                            beds,
                        } => {
                            let (u, v) = extent.grid(x as f64 + 0.5, z as f64 + 0.5);
                            let new = snap_height((old + after.sample(u, v) - before.sample(u, v)).clamp(min_y, max_y));
                            if let Some(w) = water {
                                let level = w.at(u.round().max(0.0) as usize, v.round().max(0.0) as usize);
                                if level != NO_WATER && level > new + 0.3 && lets(x, z) {
                                    let level = level.round().clamp(min_y, max_y) as i16;
                                    if tile.fluid.get(i) != level {
                                        tile.fluid.set(i, level);
                                        moved = true;
                                    }
                                }
                            }
                            if let (Some(beds), Some(bed)) = (beds, bed) {
                                let dither = hash2(x, z, 0xbed5) as f32 / u32::MAX as f32;
                                if beds.sample(u, v) > dither.max(0.05) && tile.terrain.get(i) != bed.0 && lets(x, z) {
                                    tile.terrain.set(i, bed.0);
                                    moved = true;
                                }
                            }
                            new
                        }
                    };
                    let new = if weight == IN { new } else { old + (new - old) * weight as f32 / IN as f32 };
                    let new = snap_height(new.clamp(min_y, max_y));
                    if (new - old).abs() > 1e-3 {
                        tile.top.set(i, new);
                        moved = true;
                    }
                    n += moved as usize;
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, Ordering::Relaxed);
                edited.lock().expect("no panic while held").push(pos);
            }
            steps.done.fetch_add(1, Ordering::Relaxed);
        });
        touched.extend(edited.into_inner().expect("no panic while held"));
        edit.cool(KEEP_HOT);
    }
    Changes {
        columns: changed.into_inner(),
        tiles: touched,
        cancelled,
    }
}

/// `op` over the map at `n` samples a side, for a picture of it: the square
/// read, and its surface before and after (what the filters change; the
/// relief's function applied to each sample). `None`: cancelled, or no map.
pub fn reshape_preview(dim: &Dimension, op: &Reshape, sea: i32, n: usize, cancel: &AtomicBool) -> Option<(Extent, Field, Field)> {
    let (lo, hi) = dim.ground().bounds()?;
    let rect = BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
    let extent = extent_of(rect, n);
    match change_of(dim, rect, op, sea, n, cancel, &|_| {})? {
        Change::Samples { before, after, .. } => Some((extent, Arc::unwrap_or_clone(before), Arc::unwrap_or_clone(after))),
        Change::Columns(f) => {
            let (before, _) = read(dim, &extent, sea);
            let after = Field::new(before.n, before.data.iter().map(|&h| f(h)).collect());
            Some((extent, before, after))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::tile::{NO_FLUID, Tile};
    use crate::world::{DimensionKind, World, WorldMeta};

    const SEA: i32 = 62;

    /// 2 × 2 tiles of grass: a plain at 80, a round hill of 60 blocks over
    /// it, a round hollow 12 deep beside it; the sea's water level (62) in
    /// every column, all of them above it.
    fn world() -> World {
        let mut world = World::new(WorldMeta::default());
        let grass = world.registry.terrain_by_name("Grass").unwrap();
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let cone = |x: i32, z: i32, cx: i32, cz: i32, r: f32| (1.0 - (((x - cx).pow(2) + (z - cz).pow(2)) as f32).sqrt() / r).max(0.0);
        for tz in 0..2 {
            for tx in 0..2 {
                let mut tile = Tile::ground(80.0, grass.0, SEA as i16);
                for lz in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let (x, z) = (tx * TILE_SIZE + lx, tz * TILE_SIZE + lz);
                        let h = 80.0 + 60.0 * cone(x, z, 100, 100, 70.0) - 12.0 * cone(x, z, 200, 200, 30.0);
                        tile.top.set(local_index(x, z), snap_height(h));
                    }
                }
                dim.ground_mut().insert(TilePos::new(tx, tz), tile);
            }
        }
        world.dims = vec![dim];
        world
    }

    fn run(world: &mut World, op: &Reshape, steps: &Steps) -> Changes {
        let bed = world.registry.terrain_by_name("Gravel");
        let mut history = History::new(2);
        history.begin(op.label(), 0);
        reshape_ground(&mut history.edit(world, 0), op, SEA, bed, steps)
    }

    fn column(world: &World, x: i32, z: i32) -> (f32, i16, u16) {
        let tile = world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
        let i = local_index(x, z);
        (tile.top.get(i), tile.fluid.get(i), tile.terrain.get(i))
    }

    #[test]
    fn a_whole_map_filter_keeps_to_the_selection() {
        use crate::selection::{Selection, Shape};
        let mut w = world();
        let mut h = History::default();
        let sel = Selection::shape(&Shape::Rect(BlockRect::new(0, 0, 60, 256))).feather(10);
        assert!(h.select(&mut w, 0, "Select", Some(sel.clone())));
        let changes = run(&mut w, &Reshape::Relief { scale: 1.0, shift: 10.0 }, &Steps::default());
        // Only the two tiles the selection holds were read and written.
        assert_eq!(changes.tiles.len(), 2);
        for x in 0..128 {
            let want = 80.0 + 10.0 * sel.weight(x, 240);
            assert!((column(&w, x, 240).0 - want).abs() <= 0.06, "{x}");
        }
        assert_eq!(column(&w, 20, 240).0, 90.0);
        assert_eq!(column(&w, 100, 240).0, 80.0);
        assert_eq!(column(&w, 200, 10).0, 80.0);
    }

    #[test]
    fn the_relief_moves_every_column() {
        let mut w = world();
        let steps = Steps::default();
        let changes = run(&mut w, &Reshape::Relief { scale: 2.0, shift: 3.0 }, &steps);
        assert!(!changes.cancelled);
        assert_eq!(steps.fraction(), 1.0);
        assert_eq!((changes.columns, changes.tiles.len()), (4 * 128 * 128, 4));
        // Twice as high over the sea, three blocks up.
        assert_eq!(column(&w, 10, 240).0, SEA as f32 + 2.0 * 18.0 + 3.0);
        assert_eq!(column(&w, 100, 100).0, SEA as f32 + 2.0 * 78.0 + 3.0);
        // Cancelled: nothing written.
        let stop = Steps::default();
        stop.cancel.store(true, Ordering::Relaxed);
        let before = column(&w, 10, 240);
        assert!(run(&mut w, &Reshape::Relief { scale: 1.0, shift: 9.0 }, &stop).cancelled);
        assert_eq!(column(&w, 10, 240), before);
    }

    #[test]
    fn terraces_are_flat_steps() {
        let mut w = world();
        let op = Reshape::Filter(NodeKind::Terrace { steps: 4, strength: 1.0 });
        assert!(run(&mut w, &op, &Steps::default()).columns > 1000);
        // On the hill, the columns stand on a few levels.
        let mut levels = std::collections::HashMap::new();
        for z in 40..160 {
            for x in 40..160 {
                *levels.entry(column(&w, x, z).0.to_bits()).or_insert(0usize) += 1;
            }
        }
        let mut counts: Vec<usize> = levels.into_values().collect();
        counts.sort_unstable_by(|a, b| b.cmp(a));
        let flat: usize = counts.iter().take(5).sum();
        assert!(flat as f32 > 0.6 * 120.0 * 120.0, "{flat} of the hill's columns on its five main levels");
    }

    #[test]
    fn erosion_wears_the_hill_and_keeps_the_paint() {
        let mut w = world();
        let grass = w.registry.terrain_by_name("Grass").unwrap().0;
        let before: Vec<f32> = (40..160).map(|x| column(&w, x, 100).0).collect();
        let op = Reshape::Filter(NodeKind::Erosion {
            rain: 1.0,
            erosion: 0.5,
            deposition: 0.3,
            capacity: 4.0,
            evaporation: 0.02,
            radius: 3.0,
            seed: 5,
        });
        let changes = run(&mut w, &op, &Steps::default());
        assert!(changes.columns > 1000, "{} columns", changes.columns);
        let after: Vec<f32> = (40..160).map(|x| column(&w, x, 100).0).collect();
        assert!(before.iter().zip(&after).any(|(a, b)| (a - b).abs() > 0.2), "the hill's flanks change");
        for z in (0..256).step_by(7) {
            for x in (0..256).step_by(7) {
                let (_, fluid, terrain) = column(&w, x, z);
                assert_eq!((fluid, terrain), (SEA as i16, grass), "water and paint stay, at {x} {z}");
            }
        }
    }

    #[test]
    fn lakes_fill_the_hollows() {
        let mut w = world();
        let op = Reshape::Filter(NodeKind::Lakes { amount: 0.0, sea: SEA as f32 });
        let changes = run(&mut w, &op, &Steps::default());
        assert!(changes.columns > 500, "{} columns", changes.columns);
        // Water up to the plain, over the bottom of the hollow, which stays.
        let (top, fluid, _) = column(&w, 200, 200);
        assert!((top - 68.0).abs() < 0.5 && (fluid - 80).abs() <= 1, "top {top}, water {fluid}");
        // The plain stays dry, at the sea's level as before.
        assert_eq!(column(&w, 10, 240).1, SEA as i16);
        assert!(NO_FLUID != SEA as i16);
    }

    #[test]
    fn every_filter_runs_over_the_map() {
        for kind in NodeKind::catalogue().into_iter().filter(Reshape::runs) {
            let mut w = world();
            let changes = run(&mut w, &Reshape::Filter(kind.clone()), &Steps::default());
            assert!(!changes.cancelled && changes.columns > 0, "{}: {} columns", kind.label(), changes.columns);
        }
    }

    #[test]
    fn a_preview_reads_the_map_small() {
        let w = world();
        let op = Reshape::Relief { scale: 1.0, shift: 4.0 };
        let (extent, before, after) = reshape_preview(&w.dims[0], &op, SEA, 64, &AtomicBool::new(false)).unwrap();
        assert_eq!((extent.n, before.n, after.n), (64, 64, 64));
        assert!(before.data.iter().zip(&after.data).all(|(a, b)| (b - a - 4.0).abs() < 1e-4));
    }
}
