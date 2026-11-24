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
pub(super) fn ground_on_top(pos: TilePos, tile: &Tile, others: &Others<'_>) -> Option<Vec<bool>> {
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
pub fn reshape_ground(edit: &mut Edit, op: &Reshape, sea: i32, bed: Option<TerrainId>, shore: Option<TerrainId>, steps: &Steps) -> Changes {
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
    let beds_painted = bed.is_some() && matches!(&change, Change::Samples { beds: Some(_), .. });
    steps.done.store(3 * count, Ordering::Relaxed);
    let (min_y, max_y) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    // The water this pass poured, and the ground it dug as it was: the
    // banks rise back to hold the water, or the water goes down.
    let poured = Mutex::new(std::collections::HashSet::new());
    let dug = Mutex::new(std::collections::HashMap::new());
    // The beds painted under the water, with the terrain they had: where
    // the water does not stay over them, they get it back.
    let painted = Mutex::new(std::collections::HashMap::new());
    // The water poured into lakes.
    let lakes = Mutex::new(std::collections::HashSet::new());
    // The ground dug in the rivers' beds (not their banks).
    let in_bed = Mutex::new(std::collections::HashSet::new());
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
                                // A block of water at least over the ground.
                                if level != NO_WATER && level.round() as i32 > crate::compose::block_top(new) && lets(x, z) {
                                    let level = level.round().clamp(min_y, max_y) as i16;
                                    let was = tile.fluid.get(i);
                                    if was != level {
                                        tile.fluid.set(i, level);
                                        moved = true;
                                        // Water where there was none: the
                                        // pass's own, which it may lower.
                                        if was == crate::tile::NO_FLUID || was as f32 <= old + 0.3 {
                                            poured.lock().expect("no panic while held").insert((x, z));
                                            // A lake's: all the water lakes
                                            // pour, what rivers pour out of
                                            // their beds.
                                            if beds.as_ref().is_none_or(|b| b.sample(u, v) < 0.5) {
                                                lakes.lock().expect("no panic while held").insert((x, z));
                                            }
                                        }
                                    }
                                }
                            }
                            if let (Some(beds), Some(bed)) = (beds, bed) {
                                let dither = hash2(x, z, 0xbed5) as f32 / u32::MAX as f32;
                                // Only under the water: gravel in its
                                // depths, sand in its shallows and in its
                                // channel under the sea; the banks keep
                                // their ground.
                                // Its depth in blocks: water blocks over the ground's top.
                                let fluid = tile.fluid.get(i);
                                let depth = if fluid != crate::tile::NO_FLUID { fluid as i32 - crate::compose::block_top(new) } else { 0 };
                                if beds.sample(u, v) > dither.max(0.05) && depth >= 1 && lets(x, z) {
                                    let river = water.as_ref().is_some_and(|w| w.at(u.round().max(0.0) as usize, v.round().max(0.0) as usize) != NO_WATER);
                                    let terrain = if !river || depth < 2 { shore.unwrap_or(bed) } else { bed };
                                    let was = tile.terrain.get(i);
                                    if was != terrain.0 {
                                        tile.terrain.set(i, terrain.0);
                                        moved = true;
                                        painted.lock().expect("no panic while held").insert((x, z), was);
                                    }
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
                        if new < old {
                            dug.lock().expect("no panic while held").insert((x, z), old);
                            // In a river's bed itself, not on its banks.
                            if let Change::Samples { extent, beds: Some(beds), .. } = &change {
                                let (u, v) = extent.grid(x as f64 + 0.5, z as f64 + 0.5);
                                if beds.sample(u, v) >= 0.99 {
                                    in_bed.lock().expect("no panic while held").insert((x, z));
                                }
                            }
                        }
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
    let (mut poured, dug) = (poured.into_inner().expect("no panic while held"), dug.into_inner().expect("no panic while held"));
    let mut painted = painted.into_inner().expect("no panic while held");
    if !cancelled && !poured.is_empty() && !dug.is_empty() {
        // The water fills the bed it runs in: a column it dug that stays
        // dry beside the water, under its surface (at a step, where the
        // level below comes too low to cover it), is under the water above.
        let in_bed = in_bed.into_inner().expect("no panic while held");
        let filled = flood_dug(edit, &poured, &in_bed, beds_painted.then_some(shore.or(bed)).flatten(), &mut painted);
        poured.extend(filled);
    }
    if !cancelled && !poured.is_empty() {
        // A body of the pass's water that runs over open land its ground
        // does not hold (a lake worked out on samples coarser than the
        // blocks, over a notch in its rim) goes down as a whole to where
        // it spills, filling all of its hollow below that.
        settle_bodies(edit, &mut poured, &dug, &lakes.lock().expect("no panic while held"));
    }
    if !cancelled && !poured.is_empty() {
        // Tile by tile, and again across their edges.
        for shift in [0, TILE_SIZE / 2] {
            for &pos in &touched {
                let r = pos.rect();
                let r = BlockRect::new(r.x0 + shift, r.z0 + shift, r.x1 + shift, r.z1 + shift);
                crate::water::seal(edit, r, &|x, z| dug.get(&(x, z)).copied().unwrap_or(f32::MIN), &|x, z| poured.contains(&(x, z)), sea as i16);
            }
        }
    }
    // A block of land left alone in the water, flush with it, is under it.
    if !cancelled && !poured.is_empty() {
        fill_islets(edit, &mut poured, &dug, beds_painted.then_some(shore.or(bed)).flatten(), &mut painted);
    }
    // The lakes' floors: a bowl, sand in the shallows, gravel deeper.
    let lakes = lakes.into_inner().expect("no panic while held");
    let mut beaches = Vec::new();
    if !cancelled && !lakes.is_empty() && let Some(floor) = bed.or(shore) {
        let lakes: std::collections::HashSet<(i32, i32)> = lakes.into_iter().filter(|p| poured.contains(p)).collect();
        beaches = lake_floors(edit, &lakes, shore.unwrap_or(floor), floor, &mut painted);
    }
    // A bed the water left (its bank raised back to hold the water, the
    // water lowered) is not a bed: it gets its ground's terrain back.
    if !cancelled && !painted.is_empty() {
        let mut by_tile: std::collections::HashMap<TilePos, Vec<(i32, i32, u16)>> = std::collections::HashMap::new();
        for (&(x, z), &was) in &painted {
            by_tile.entry(TilePos::containing(x, z)).or_default().push((x, z, was));
        }
        for (pos, cols) in by_tile {
            let Some(tile) = edit.tile_mut(pos) else { continue };
            let mut n = 0;
            for (x, z, was) in cols {
                let i = local_index(x, z);
                let fluid = tile.fluid.get(i);
                if fluid == crate::tile::NO_FLUID || fluid as i32 <= crate::compose::block_top(tile.top.get(i)) {
                    tile.terrain.set(i, was);
                    n += 1;
                }
            }
            if n > 0 {
                tile.compact();
            }
        }
    }
    // Their beaches, after: dry, they stay.
    if let Some(sand) = shore.filter(|_| !beaches.is_empty()) {
        let mut by_tile: std::collections::HashMap<TilePos, Vec<(i32, i32)>> = std::collections::HashMap::new();
        for &(x, z) in &beaches {
            by_tile.entry(TilePos::containing(x, z)).or_default().push((x, z));
        }
        for (pos, cols) in by_tile {
            let Some(tile) = edit.tile_mut(pos) else { continue };
            for (x, z) in cols {
                tile.terrain.set(local_index(x, z), sand.0);
            }
            tile.compact();
        }
    }
    Changes {
        columns: changed.into_inner(),
        tiles: touched,
        cancelled,
    }
}

/// A column, by its block coordinates.
type Spot = (i32, i32);
/// What goes to each tile.
type PerTile<T> = std::collections::HashMap<TilePos, Vec<T>>;
/// The tops and water levels of a tile's columns.
type TileTops = (Vec<f32>, Vec<i16>);

/// The columns of the beds a pass dug (`dug`) that stay dry beside the
/// water it `poured` though their ground lies under its surface, filled
/// with that water, and so on from them: the highest water around first.
/// Painted `bed` (their terrain as it was in `painted`). Returns the
/// columns filled.
fn flood_dug(edit: &mut Edit, poured: &std::collections::HashSet<(i32, i32)>, dug: &std::collections::HashSet<(i32, i32)>, bed: Option<TerrainId>, painted: &mut std::collections::HashMap<(i32, i32), u16>) -> Vec<(i32, i32)> {
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    use std::collections::{BinaryHeap, HashMap};
    // The first air over each column read, and its water's level.
    let mut by_tile: HashMap<TilePos, Vec<(i32, i32)>> = HashMap::new();
    for &(x, z) in poured.iter().chain(dug.iter()) {
        by_tile.entry(TilePos::containing(x, z)).or_default().push((x, z));
    }
    let mut cols: HashMap<(i32, i32), (i32, i16)> = HashMap::with_capacity(poured.len() + dug.len());
    for (pos, list) in &by_tile {
        let Some(t) = edit.level().view(*pos) else { continue };
        for &(x, z) in list {
            let i = local_index(x, z);
            cols.insert((x, z), (block_top(t.top.get(i)), t.fluid.get(i)));
        }
    }
    let wet = |(air, fluid): (i32, i16)| fluid != NO_FLUID && fluid as i32 > air;
    let mut heap: BinaryHeap<(i16, i32, i32)> = poured.iter().filter_map(|&p| cols.get(&p).filter(|&&c| wet(c)).map(|&(_, f)| (f, p.0, p.1))).collect();
    let mut filled: HashMap<(i32, i32), i16> = HashMap::new();
    while let Some((f, x, z)) = heap.pop() {
        for n in [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)] {
            if !dug.contains(&n) || filled.contains_key(&n) {
                continue;
            }
            let Some(&c) = cols.get(&n) else { continue };
            if wet(c) || c.0 >= f as i32 {
                continue;
            }
            filled.insert(n, f);
            heap.push((f, n.0, n.1));
        }
    }
    let mut by_tile: PerTile<(Spot, i16)> = HashMap::new();
    for (&p, &f) in &filled {
        by_tile.entry(TilePos::containing(p.0, p.1)).or_default().push((p, f));
    }
    for (pos, list) in by_tile {
        let Some(tile) = edit.tile_mut(pos) else { continue };
        for (p, f) in list {
            let i = local_index(p.0, p.1);
            tile.fluid.set(i, f);
            if let Some(bed) = bed {
                let was = tile.terrain.get(i);
                if was != bed.0 {
                    tile.terrain.set(i, bed.0);
                    painted.entry(p).or_insert(was);
                }
            }
        }
        tile.compact();
    }
    filled.into_keys().collect()
}

/// The dry columns with water on every side, flush with it, beside the
/// water a pass `poured` or the ground it `dug` (a block the water was dug
/// around and left, at a mouth, in a bed): a block lower, under that
/// water, painted `bed` (their terrain as it was in `painted`). Returns
/// how many.
fn fill_islets(edit: &mut Edit, poured: &mut std::collections::HashSet<(i32, i32)>, dug: &std::collections::HashMap<(i32, i32), f32>, bed: Option<TerrainId>, painted: &mut std::collections::HashMap<(i32, i32), u16>) -> usize {
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    use std::collections::{HashMap, HashSet};
    const N4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut cols: HashMap<(i32, i32), (i32, i16)> = HashMap::new();
    {
        let mut by_tile: HashMap<TilePos, HashSet<(i32, i32)>> = HashMap::new();
        for &(x, z) in poured.iter().chain(dug.keys()) {
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1), (2, 0), (-2, 0), (0, 2), (0, -2), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                by_tile.entry(TilePos::containing(x + dx, z + dz)).or_default().insert((x + dx, z + dz));
            }
        }
        for (pos, list) in &by_tile {
            let Some(t) = edit.level().view(*pos) else { continue };
            for &(x, z) in list {
                let i = local_index(x, z);
                cols.insert((x, z), (block_top(t.top.get(i)), t.fluid.get(i)));
            }
        }
    }
    let wet = |c: &(i32, i16)| c.1 != NO_FLUID && c.1 as i32 > c.0;
    let mut islets: Vec<((i32, i32), i32)> = Vec::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    for &(x, z) in poured.iter().chain(dug.keys()) {
        for (dx, dz) in N4 {
            let p = (x + dx, z + dz);
            let Some(c) = cols.get(&p).copied() else { continue };
            if wet(&c) || !seen.insert(p) {
                continue;
            }
            let around: Vec<(i32, i16)> = N4.iter().filter_map(|(ex, ez)| cols.get(&(p.0 + ex, p.1 + ez)).copied()).collect();
            if around.len() == 4 && around.iter().all(|n| wet(n) && n.1 as i32 == c.0) {
                islets.push((p, c.0));
            }
        }
    }
    let mut by_tile: PerTile<(Spot, i32)> = HashMap::new();
    for &(p, air) in &islets {
        by_tile.entry(TilePos::containing(p.0, p.1)).or_default().push((p, air));
    }
    for (pos, list) in by_tile {
        let Some(tile) = edit.tile_mut(pos) else { continue };
        for (p, air) in list {
            let i = local_index(p.0, p.1);
            tile.top.set(i, snap_height((air - 1) as f32));
            tile.fluid.set(i, air as i16);
            if let Some(bed) = bed {
                let was = tile.terrain.get(i);
                if was != bed.0 {
                    tile.terrain.set(i, bed.0);
                    painted.entry(p).or_insert(was);
                }
            }
            poured.insert(p);
        }
        tile.compact();
    }
    islets.len()
}

/// The floors of the lakes a pass poured (`lakes`, their columns): each
/// lake dug into a bowl, a block deeper every three blocks from its shore,
/// down to a depth that grows with its size (2 to 10 blocks), never
/// raised; `shallow` (sand) under one or two blocks of water, `deep`
/// (gravel) under more (their terrain as it was in `painted`). Returns
/// its beaches: the dry columns beside it at most a block over its water.
fn lake_floors(edit: &mut Edit, lakes: &std::collections::HashSet<(i32, i32)>, shallow: TerrainId, deep: TerrainId, painted: &mut std::collections::HashMap<(i32, i32), u16>) -> Vec<(i32, i32)> {
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    use std::collections::{HashMap, HashSet, VecDeque};
    const N4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    // The first air and water of the lakes' columns and those beside.
    let mut cols: HashMap<(i32, i32), (i32, i16)> = HashMap::new();
    {
        let mut by_tile: HashMap<TilePos, Vec<(i32, i32)>> = HashMap::new();
        for &(x, z) in lakes {
            for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                by_tile.entry(TilePos::containing(x + dx, z + dz)).or_default().push((x + dx, z + dz));
            }
        }
        for (pos, list) in &by_tile {
            let Some(t) = edit.level().view(*pos) else { continue };
            for &(x, z) in list {
                let i = local_index(x, z);
                cols.insert((x, z), (block_top(t.top.get(i)), t.fluid.get(i)));
            }
        }
    }
    let wet = |c: &(i32, i16)| c.1 != NO_FLUID && c.1 as i32 > c.0;
    let mut floors: HashMap<(i32, i32), (i32, TerrainId)> = HashMap::new();
    let mut beaches: HashSet<(i32, i32)> = HashSet::new();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut list: Vec<(i32, i32)> = lakes.iter().copied().collect();
    list.sort_unstable();
    for start in list {
        if seen.contains(&start) {
            continue;
        }
        let Some(&(_, f)) = cols.get(&start).filter(|c| wet(c)) else { continue };
        // The lake: its columns side by side at its level.
        let mut body = vec![start];
        seen.insert(start);
        let mut k = 0;
        while k < body.len() {
            let (x, z) = body[k];
            k += 1;
            for (dx, dz) in N4 {
                let p = (x + dx, z + dz);
                if lakes.contains(&p) && !seen.contains(&p) && cols.get(&p).is_some_and(|c| wet(c) && c.1 == f) {
                    seen.insert(p);
                    body.push(p);
                }
            }
        }
        let mine: HashSet<(i32, i32)> = body.iter().copied().collect();
        // How far each column lies from its shore, in blocks (1 on it).
        let mut far: HashMap<(i32, i32), i32> = HashMap::with_capacity(body.len());
        let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
        for &p in &body {
            if N4.iter().any(|(dx, dz)| !mine.contains(&(p.0 + dx, p.1 + dz))) {
                far.insert(p, 1);
                queue.push_back(p);
            }
        }
        while let Some(p) = queue.pop_front() {
            let d = far[&p];
            for (dx, dz) in N4 {
                let q = (p.0 + dx, p.1 + dz);
                if mine.contains(&q) && !far.contains_key(&q) {
                    far.insert(q, d + 1);
                    queue.push_back(q);
                }
            }
        }
        let most = ((body.len() as f32).sqrt() / 8.0).round().clamp(2.0, 10.0) as i32;
        for &p in &body {
            let air = cols[&p].0;
            let want = 1 + (far.get(&p).copied().unwrap_or(1) - 1) / 3;
            let air = air.min(f as i32 - want.min(most));
            let depth = f as i32 - air;
            floors.insert(p, (air, if depth <= 2 { shallow } else { deep }));
            for (dx, dz) in N4 {
                let q = (p.0 + dx, p.1 + dz);
                if let Some(c) = cols.get(&q)
                    && !mine.contains(&q)
                    && !wet(c)
                    && (c.0 == f as i32 || c.0 == f as i32 + 1)
                {
                    beaches.insert(q);
                }
            }
        }
    }
    let mut by_tile: PerTile<(Spot, (i32, TerrainId))> = HashMap::new();
    for (&p, &v) in &floors {
        by_tile.entry(TilePos::containing(p.0, p.1)).or_default().push((p, v));
    }
    for (pos, list) in by_tile {
        let Some(tile) = edit.tile_mut(pos) else { continue };
        for (p, (air, terrain)) in list {
            let i = local_index(p.0, p.1);
            if block_top(tile.top.get(i)) > air {
                tile.top.set(i, snap_height(air as f32));
            }
            let was = tile.terrain.get(i);
            if was != terrain.0 {
                tile.terrain.set(i, terrain.0);
                painted.entry(p).or_insert(was);
            }
        }
        tile.compact();
    }
    beaches.into_iter().collect()
}

/// The bodies of the water a pass `poured` (columns side by side at one
/// level) that stand over open land beside them (not ground the pass dug:
/// that rises back to hold them), each brought down to the level its
/// ground holds, block by block: the hollow around it filled from its
/// rim inward (the water already on the map a wall), the body's columns
/// to the lower of their level and that, and the dry columns of its
/// hollow under that level joined to it. Returns the columns lowered (or
/// dried) and added; those added are added to `poured`.
fn settle_bodies(edit: &mut Edit, poured: &mut std::collections::HashSet<(i32, i32)>, dug: &std::collections::HashMap<(i32, i32), f32>, lakes: &std::collections::HashSet<(i32, i32)>) -> (usize, usize) {
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
    // The first air over a column and its water's level (NO_FLUID: none),
    // read a tile at a time; `None` past the map.
    struct Reader<'a> {
        level: &'a crate::level::Level,
        tiles: HashMap<TilePos, Option<TileTops>>,
    }
    impl Reader<'_> {
        fn at(&mut self, x: i32, z: i32) -> Option<(i32, i16)> {
            let pos = TilePos::containing(x, z);
            let level = self.level;
            let t = self.tiles.entry(pos).or_insert_with(|| {
                level.view(pos).map(|t| {
                    let n = (TILE_SIZE * TILE_SIZE) as usize;
                    ((0..n).map(|i| t.top.get(i)).collect(), (0..n).map(|i| t.fluid.get(i)).collect())
                })
            });
            let (top, fluid) = t.as_ref()?;
            let i = local_index(x, z);
            Some((block_top(top[i]), fluid[i]))
        }
    }
    let wet = |(air, fluid): (i32, i16)| fluid != NO_FLUID && fluid as i32 > air;
    const N4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut changes: HashMap<(i32, i32), i16> = HashMap::new();
    {
        let mut read = Reader { level: edit.level(), tiles: HashMap::new() };
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        let mut list: Vec<(i32, i32)> = poured.iter().copied().collect();
        list.sort_unstable();
        for start in list {
            if seen.contains(&start) {
                continue;
            }
            let Some(c0) = read.at(start.0, start.1).filter(|&c| wet(c)) else { continue };
            let f = c0.1;
            // The body: the pass's water at this level, side by side.
            let mut body = vec![start];
            seen.insert(start);
            let mut k = 0;
            while k < body.len() {
                let (x, z) = body[k];
                k += 1;
                for (dx, dz) in N4 {
                    let p = (x + dx, z + dz);
                    if poured.contains(&p) && !seen.contains(&p) && read.at(p.0, p.1).is_some_and(|c| wet(c) && c.1 == f) {
                        seen.insert(p);
                        body.push(p);
                    }
                }
            }
            // A lake (most of it poured as one) stands a block over the
            // water beside it at most: a river that cuts into its rim lower
            // down drains it to there.
            let lake = 2 * body.iter().filter(|p| lakes.contains(p)).count() > body.len();
            let mine: HashSet<(i32, i32)> = body.iter().copied().collect();
            let mut leaks = false;
            'look: for &(x, z) in &body {
                for (dx, dz) in N4 {
                    let p = (x + dx, z + dz);
                    if mine.contains(&p) {
                        continue;
                    }
                    let Some(c) = read.at(p.0, p.1) else { continue };
                    if (!wet(c) && c.0 < f as i32 && !dug.contains_key(&p)) || (lake && wet(c) && (c.1 as i32) < f as i32 - 1) {
                        leaks = true;
                        break 'look;
                    }
                }
            }
            if !leaks {
                continue;
            }
            // Its hollow, in a window around it: filled from the window's
            // edge inward, over the first air of each column; the water of
            // the map and of the pass, but for this body's, as walls.
            const MARGIN: i32 = 12;
            let (x0, z0) = (body.iter().map(|p| p.0).min().unwrap_or(0) - MARGIN, body.iter().map(|p| p.1).min().unwrap_or(0) - MARGIN);
            let (x1, z1) = (body.iter().map(|p| p.0).max().unwrap_or(0) + MARGIN, body.iter().map(|p| p.1).max().unwrap_or(0) + MARGIN);
            let (w, d) = ((x1 - x0 + 1) as usize, (z1 - z0 + 1) as usize);
            // The ground the pass dug counts as it was: it rises back to
            // hold the water, rather than let it out.
            // Other water is a wall to a river; to a lake, a sill a block
            // over its surface.
            const WALL: i32 = i32::MAX;
            let mut ground = vec![WALL; w * d];
            let mut rim = vec![WALL; w * d];
            for j in 0..d {
                for i in 0..w {
                    let p = (x0 + i as i32, z0 + j as i32);
                    let c = j * w + i;
                    (ground[c], rim[c]) = match read.at(p.0, p.1) {
                        None => (WALL, WALL),
                        Some(c) if wet(c) && !mine.contains(&p) => (WALL, if lake { c.1 as i32 + 1 } else { WALL }),
                        Some(c) => (c.0, dug.get(&p).map_or(c.0, |&was| c.0.max(block_top(was)))),
                    };
                }
            }
            let mut filled = vec![WALL; w * d];
            let mut heap: BinaryHeap<Reverse<(i32, usize)>> = BinaryHeap::new();
            for j in 0..d {
                for i in 0..w {
                    let c = j * w + i;
                    if (i == 0 || j == 0 || i == w - 1 || j == d - 1) && rim[c] != WALL {
                        filled[c] = rim[c];
                        heap.push(Reverse((rim[c], c)));
                    }
                }
            }
            while let Some(Reverse((level, c))) = heap.pop() {
                if level > filled[c] {
                    continue;
                }
                let (i, j) = ((c % w) as i32, (c / w) as i32);
                for (di, dj) in N4 {
                    let (a, b) = (i + di, j + dj);
                    if a < 0 || b < 0 || a >= w as i32 || b >= d as i32 {
                        continue;
                    }
                    let nb = b as usize * w + a as usize;
                    if rim[nb] == WALL {
                        continue;
                    }
                    let l = level.max(rim[nb]);
                    if l < filled[nb] {
                        filled[nb] = l;
                        heap.push(Reverse((l, nb)));
                    }
                }
            }
            let at = |p: (i32, i32)| (p.1 - z0) as usize * w + (p.0 - x0) as usize;
            let new_level = |c: usize| (f as i32).min(filled[c]);
            // The body's columns down to what holds them; the dry columns
            // beside them under that, joined to them.
            let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
            let mut joined: HashSet<(i32, i32)> = HashSet::new();
            for &p in &body {
                let c = at(p);
                let l = new_level(c);
                if l > ground[c] {
                    if l != f as i32 {
                        changes.insert(p, l as i16);
                    }
                    queue.push_back(p);
                } else {
                    changes.insert(p, NO_FLUID);
                }
            }
            while let Some(p) = queue.pop_front() {
                for (dx, dz) in N4 {
                    let q = (p.0 + dx, p.1 + dz);
                    if q.0 < x0 || q.1 < z0 || q.0 > x1 || q.1 > z1 || mine.contains(&q) || joined.contains(&q) {
                        continue;
                    }
                    let c = at(q);
                    if ground[c] == WALL {
                        continue;
                    }
                    let l = new_level(c);
                    if l > ground[c] {
                        joined.insert(q);
                        changes.insert(q, l as i16);
                        queue.push_back(q);
                    }
                }
            }
        }
    }
    let lowered = changes.values().filter(|&&l| l == NO_FLUID).count() + changes.keys().filter(|p| poured.contains(p)).filter(|p| changes[p] != NO_FLUID).count();
    let added = changes.keys().filter(|p| !poured.contains(p)).count();
    let mut by_tile: PerTile<(Spot, i16)> = HashMap::new();
    for (&p, &l) in &changes {
        by_tile.entry(TilePos::containing(p.0, p.1)).or_default().push((p, l));
        if l != NO_FLUID {
            poured.insert(p);
        }
    }
    for (pos, list) in by_tile {
        let Some(tile) = edit.tile_mut(pos) else { continue };
        for (p, l) in list {
            tile.fluid.set(local_index(p.0, p.1), l);
        }
        tile.compact();
    }
    (lowered, added)
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
        let shore = world.registry.terrain_by_name("Sand");
        let mut history = History::new(2);
        history.begin(op.label(), 0);
        reshape_ground(&mut history.edit(world, 0), op, SEA, bed, shore, steps)
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

    /// A generated map of hills, 4 × 4 tiles: valleys, shallow hollows,
    /// the sea at 63.
    fn hills() -> World {
        use crate::generate::{GenParams, Landform, generate};
        generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: Landform::Hills, seed: 3, ..GenParams::default() })
    }

    /// `op` over a map at its own sea level, as Edit ▸ Whole map runs it:
    /// gravel beds, sand shores.
    fn run_at_sea(world: &mut World, op: &Reshape) -> Changes {
        let sea = world.meta.sea_level;
        let (bed, shore) = (world.registry.terrain_by_name("Gravel"), world.registry.terrain_by_name("Sand"));
        let mut history = History::new(2);
        history.begin(op.label(), 0);
        reshape_ground(&mut history.edit(world, 0), op, sea, bed, shore, &Steps::default())
    }

    /// Every column of a map: its first air, its water's level, its
    /// terrain.
    fn columns(world: &World) -> std::collections::HashMap<(i32, i32), (i32, i16, u16)> {
        let mut out = std::collections::HashMap::new();
        for p in world.dims[0].ground().positions() {
            let t = world.dims[0].ground().view(p).unwrap();
            let (ox, oz) = p.origin();
            for z in oz..oz + TILE_SIZE {
                for x in ox..ox + TILE_SIZE {
                    let i = local_index(x, z);
                    out.insert((x, z), (crate::compose::block_top(t.top.get(i)), t.fluid.get(i), t.terrain.get(i)));
                }
            }
        }
        out
    }

    const N4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    /// Water over a column's ground.
    fn wet(c: &(i32, i16, u16)) -> bool {
        c.1 != NO_FLUID && c.1 as i32 > c.0
    }

    /// Holes in the water's surface: dry columns among water on every side
    /// that stands as high as their ground or higher.
    fn holes(cols: &std::collections::HashMap<(i32, i32), (i32, i16, u16)>) -> Vec<(i32, i32)> {
        cols.iter()
            .filter(|(p, c)| !wet(c) && N4.iter().all(|(dx, dz)| cols.get(&(p.0 + dx, p.1 + dz)).is_some_and(|n| wet(n) && n.1 as i32 >= c.0)))
            .map(|(p, _)| *p)
            .collect()
    }

    /// The columns a pass painted sand that lie neither under the water
    /// nor beside it: sand left on the land.
    fn sand_on_land(before: &std::collections::HashMap<(i32, i32), (i32, i16, u16)>, after: &std::collections::HashMap<(i32, i32), (i32, i16, u16)>, sand: u16) -> Vec<(i32, i32)> {
        after
            .iter()
            .filter(|(p, c)| c.2 == sand && before[p].2 != sand && !wet(c) && !N4.iter().any(|(dx, dz)| after.get(&(p.0 + dx, p.1 + dz)).is_some_and(wet)))
            .map(|(p, _)| *p)
            .collect()
    }

    /// On hills, Lakes fills their hollows with lakes that stand in them:
    /// one level each, held by their rim on every side (no water beside
    /// lower land, none over a step down to other water, none with air
    /// under it), no hole in their surface; in a bowl, deeper than their
    /// shallow hollow where they are wide; sand under their shallows and
    /// on the flat of their shores, nowhere else.
    #[test]
    fn lakes_on_hills_stand_in_their_hollows() {
        let mut w = hills();
        let (before, check) = (columns(&w), crate::water::check(&w.dims[0]));
        let changes = run_at_sea(&mut w, &Reshape::Filter(NodeKind::Lakes { amount: 0.0, sea: 63.0 }));
        let (after, held) = (columns(&w), crate::water::check(&w.dims[0]));
        assert!(held.columns > check.columns + 2000, "{} columns of water, {} before ({} changed)", held.columns, check.columns, changes.columns);
        assert_eq!((held.spills, held.floating, held.steps, held.falls), (check.spills, check.floating, check.steps, check.falls), "{held:?}");
        assert_eq!(holes(&after), holes(&before));
        // Deeper than the hollow was: its floor dug into a bowl.
        let deepest = |cols: &std::collections::HashMap<(i32, i32), (i32, i16, u16)>| after.iter().filter(|(_, c)| wet(c) && c.1 > 63).map(|(p, c)| c.1 as i32 - cols[p].0).max().unwrap_or(0);
        assert!(deepest(&after) >= deepest(&before) + 2, "{} blocks deep, {} over the ground as it was", deepest(&after), deepest(&before));
        let sand = w.registry.terrain_by_name("Sand").unwrap().0;
        assert!(after.iter().any(|(p, c)| c.2 == sand && before[p].2 != sand && !wet(c)), "beaches");
        assert_eq!(sand_on_land(&before, &after, sand), Vec::<(i32, i32)>::new());
    }

    /// On hills, Rivers' water holds as it is dug: none beside lower land,
    /// none with air under it, no hole or dam of land across its surface
    /// (a bed dug under the water above a step is under it), falls rare;
    /// its beds' sand and gravel only under it.
    #[test]
    fn rivers_on_hills_hold_their_water() {
        let mut w = hills();
        let (before, check) = (columns(&w), crate::water::check(&w.dims[0]));
        let op = Reshape::Filter(NodeKind::Rivers { threshold: 0.5, depth: 4.0, width: 10.0, sea: 63.0, banks: 1.0, lakes: 0.5 });
        run_at_sea(&mut w, &op);
        let (after, held) = (columns(&w), crate::water::check(&w.dims[0]));
        assert!(held.columns > check.columns + 1000, "{} columns of water, {} before", held.columns, check.columns);
        assert_eq!((held.spills, held.floating), (check.spills, check.floating), "{held:?}");
        assert!(held.steps > check.steps && held.falls * 20 <= held.steps, "{held:?}");
        assert_eq!(holes(&after), holes(&before));
        let sand = w.registry.terrain_by_name("Sand").unwrap().0;
        let gravel = w.registry.terrain_by_name("Gravel").unwrap().0;
        assert_eq!(sand_on_land(&before, &after, sand), Vec::<(i32, i32)>::new());
        let gravel_on_land: Vec<(i32, i32)> = after.iter().filter(|(p, c)| c.2 == gravel && before[p].2 != gravel && !wet(c)).map(|(p, _)| *p).collect();
        assert_eq!(gravel_on_land, Vec::<(i32, i32)>::new());
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
