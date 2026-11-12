//! Rivers drawn by hand: a course of a few points becomes a smooth bed dug
//! into the ground, with banks that ease into the land around and water
//! that only ever runs down.
//!
//! The points are joined by a curve through all of them. Along it the
//! water's level is the lowest the land allows so far, a little under the
//! ground: it never rises downstream, whatever the bumps. The bed lies
//! `depth` under the water in the middle and rises to the water's edge;
//! past it, banks `banks` blocks wide climb back to the ground. A course
//! that starts in water (a river already there) leaves it at that water's
//! level: a branch; one that ends in water flows in at that level: a
//! tributary. Which end is the source is told by the land: water runs from
//! the higher end to the lower one.
//!
//! The level of each column is a whole block, so the water stands in steps
//! along the river: still water, for the game; a current for mods that let
//! water run over water.

use crate::codec::snap_height;
use crate::coords::{BlockRect, TilePos, local_index};
use crate::history::Edit;
use crate::level::TileRef;
use crate::registry::{BiomeId, TerrainId};
use crate::tile::NO_FLUID;
use std::collections::HashMap;

/// How a river is dug.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiverSpec {
    /// Width of the water, in blocks, at the source and at the mouth.
    pub width: f32,
    pub width_end: f32,
    /// Depth of the water in the middle of the bed.
    pub depth: f32,
    /// Width of each bank, from the water's edge to the untouched ground.
    pub banks: f32,
    /// Fill the bed with water.
    pub water: bool,
    /// Terrain of the bed under the water (`None`: left as it is).
    pub bed: Option<TerrainId>,
    /// Biome of the water (`None`: left as it is).
    pub biome: Option<BiomeId>,
}

impl Default for RiverSpec {
    fn default() -> Self {
        Self {
            width: 12.0,
            width_end: 12.0,
            depth: 4.0,
            banks: 10.0,
            water: true,
            bed: None,
            biome: None,
        }
    }
}

/// What a river dug.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RiverReport {
    /// Length of its course, in blocks.
    pub length: f32,
    /// Columns changed, and of them, under its water.
    pub columns: usize,
    pub wet: usize,
    /// The level of its water at the source and at the mouth.
    pub source: f32,
    pub mouth: f32,
}

/// Blocks between the points of the smoothed course.
const STEP: f32 = 0.5;

/// The curve through `points` (Catmull-Rom), a point every [`STEP`] blocks.
pub fn smooth_course(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for &p in points {
        // Points on top of each other add nothing.
        if pts.last().is_none_or(|q: &(f32, f32)| (q.0 - p.0).hypot(q.1 - p.1) > 0.5) {
            pts.push(p);
        }
    }
    match pts.len() {
        0 => return Vec::new(),
        1 => return pts,
        _ => {}
    }
    let at = |k: isize| pts[k.clamp(0, pts.len() as isize - 1) as usize];
    let mut out = Vec::new();
    for i in 0..pts.len() - 1 {
        let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
        let len = (p2.0 - p1.0).hypot(p2.1 - p1.1);
        let steps = (len / STEP).ceil().max(1.0) as usize;
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
            out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out.push(*pts.last().expect("two points or more"));
    out
}

/// Reads the ground of a level at single columns, decoding each tile once.
struct Ground<'a> {
    level: &'a crate::level::Level,
    tiles: HashMap<TilePos, Option<TileRef<'a>>>,
}

impl<'a> Ground<'a> {
    /// The height of the ground at `(x, z)`, and the level of its water if
    /// it is under water.
    fn at(&mut self, x: i32, z: i32) -> Option<(f32, Option<f32>)> {
        let pos = TilePos::containing(x, z);
        let level = self.level;
        let tile = self.tiles.entry(pos).or_insert_with(|| level.view(pos)).as_ref()?;
        let i = local_index(x, z);
        let top = tile.top.get(i);
        let fluid = tile.fluid.get(i);
        Some((top, (fluid != NO_FLUID && fluid as f32 > top + 0.3).then_some(fluid as f32)))
    }
}

/// Dig a river along `points` (block coordinates, in any order of flow)
/// into the level `edit` works on, recorded for undo.
pub fn carve_river(edit: &mut Edit, points: &[(f32, f32)], spec: &RiverSpec) -> RiverReport {
    let mut course = smooth_course(points);
    if course.len() < 2 {
        return RiverReport::default();
    }
    let (lo, hi) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
    let half = |w: f32| w.max(1.0) / 2.0;
    let reach = half(spec.width.max(spec.width_end)) + spec.banks.max(0.0) + 1.0;
    let depth = spec.depth.max(0.5);
    let freeboard = (depth * 0.3).max(1.0);

    // The land along the course: its ground (or the surface of the water
    // it crosses), smoothed over the river's width.
    let read = |course: &[(f32, f32)], ground: &mut Ground| -> Vec<(f32, Option<f32>)> {
        course
            .iter()
            .map(|&(x, z)| ground.at(x.floor() as i32, z.floor() as i32).unwrap_or((lo, None)))
            .collect()
    };
    let mut ground = Ground {
        level: edit.level(),
        tiles: HashMap::new(),
    };
    let mut land = read(&course, &mut ground);
    let surface = |l: &(f32, Option<f32>)| l.1.unwrap_or(l.0);
    // Water runs from the higher end to the lower one.
    let end_mean = |l: &[(f32, Option<f32>)]| {
        let k = l.len().min(16);
        l[..k].iter().map(surface).sum::<f32>() / k as f32
    };
    let mut reversed = land.clone();
    reversed.reverse();
    if end_mean(&land) < end_mean(&reversed) {
        course.reverse();
        land = reversed;
    }
    let window = ((half(spec.width) / STEP) as usize).max(2);
    let smooth: Vec<f32> = (0..land.len())
        .map(|k| {
            let (a, b) = (k.saturating_sub(window), (k + window + 1).min(land.len()));
            // Where the course crosses water, the water's surface counts,
            // raised so that the river may meet it at its level.
            let v: Vec<f32> = land[a..b].iter().map(|l| l.1.map_or(l.0, |w| w + freeboard)).collect();
            v.iter().sum::<f32>() / v.len() as f32
        })
        .collect();
    // The level of the water: never rising downstream; from the water a
    // branch leaves, into the water a tributary meets.
    let mut level = Vec::with_capacity(course.len());
    let mut current = land[0].1.unwrap_or(smooth[0] - freeboard);
    for g in &smooth {
        current = current.min(g - freeboard);
        level.push(current);
    }
    if let Some(mouth) = land.last().and_then(|l| l.1) {
        for l in &mut level {
            *l = l.max(mouth);
        }
    }
    let length = course.len() as f32 * STEP;
    let widths: Vec<f32> = (0..course.len()).map(|k| half(spec.width + (spec.width_end - spec.width) * k as f32 / (course.len() - 1) as f32)).collect();

    // The course in buckets, to find the nearest point of it fast.
    let cell = reach.max(4.0);
    let key = |x: f32, z: f32| ((x / cell).floor() as i32, (z / cell).floor() as i32);
    let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, &(x, z)) in course.iter().enumerate() {
        buckets.entry(key(x, z)).or_default().push(k);
    }
    let nearest = |x: f32, z: f32| -> Option<(usize, f32)> {
        let (cx, cz) = key(x, z);
        let mut best: Option<(usize, f32)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &k in buckets.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    let d = (course[k].0 - x).hypot(course[k].1 - z);
                    if best.is_none_or(|b| d < b.1) {
                        best = Some((k, d));
                    }
                }
            }
        }
        best
    };

    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, z) in &course {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
    }
    let rect = BlockRect::new(
        (x0 - reach).floor() as i32,
        (z0 - reach).floor() as i32,
        (x1 + reach).ceil() as i32 + 1,
        (z1 + reach).ceil() as i32 + 1,
    );
    let counts = std::sync::Mutex::new((0usize, 0usize));
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let (mut changed, mut wet) = (0, 0);
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
                let Some((k, d)) = nearest(cx, cz) else { continue };
                let hw = widths[k];
                if d > hw + spec.banks.max(0.0) {
                    continue;
                }
                let i = local_index(x, z);
                let ground = tile.top.get(i);
                let water = level[k];
                let target = if d <= hw {
                    // The bed: deepest in the middle, up to the water's
                    // edge at the sides.
                    let t = d / hw;
                    water - depth * (1.0 - t * t)
                } else {
                    // The bank: from just over the water up to the ground.
                    let u = ((d - hw) / spec.banks.max(0.01)).clamp(0.0, 1.0);
                    let u = u * u * (3.0 - 2.0 * u);
                    let edge = water + 0.6;
                    edge + (ground - edge).max(0.0) * u
                };
                let new = snap_height(target.min(ground).clamp(lo, hi));
                if new < ground {
                    tile.top.set(i, new);
                    changed += 1;
                }
                if d <= hw && spec.water && water > new + 0.3 {
                    let level = water.round().clamp(lo, hi) as i16;
                    let before = tile.fluid.get(i);
                    // Where two rivers meet, the higher water stays.
                    let keep = if before != NO_FLUID && before as f32 > ground { before.max(level) } else { level };
                    tile.fluid.set(i, keep);
                    wet += 1;
                    if let Some(bed) = spec.bed {
                        tile.terrain.set(i, bed.0);
                    }
                    if let Some(biome) = spec.biome {
                        tile.biome.set(i, biome.0);
                    }
                }
            }
        }
        if changed + wet > 0 {
            tile.compact();
            let mut c = counts.lock().expect("not poisoned");
            c.0 += changed;
            c.1 += wet;
        }
    });
    let (columns, wet) = counts.into_inner().expect("not poisoned");
    RiverReport {
        length,
        columns,
        wet,
        source: level[0],
        mouth: *level.last().expect("a course"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::tile::Tile;
    use crate::world::{Dimension, DimensionKind, World, WorldMeta};

    /// Land falling from 120 in the west to 70 in the east, over 2 × 1
    /// tiles, with a lake in the east (water at 75 over ground at 68).
    fn slope() -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..2 {
            let mut tile = Tile::ground(0.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let bx = tx * 128 + x;
                    let i = local_index(bx, z);
                    tile.top.set(i, 120.0 - bx as f32 * 50.0 / 256.0);
                    if bx >= 230 {
                        tile.top.set(i, 68.0);
                        tile.fluid.set(i, 75);
                    }
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        world
    }

    fn column(world: &World, x: i32, z: i32) -> (f32, i16) {
        let tile = world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
        (tile.top.get(local_index(x, z)), tile.fluid.get(local_index(x, z)))
    }

    #[test]
    fn the_course_is_smooth_and_goes_through_its_points() {
        let pts = [(0.0, 0.0), (20.0, 10.0), (40.0, 0.0)];
        let c = smooth_course(&pts);
        assert!(c.iter().any(|p| (p.0 - 20.0).abs() < 0.01 && (p.1 - 10.0).abs() < 0.01));
        assert_eq!(*c.last().unwrap(), (40.0, 0.0));
        // No step longer than a block.
        assert!(c.windows(2).all(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) < 1.0));
        assert!(smooth_course(&[(3.0, 3.0), (3.2, 3.1)]).len() == 1);
    }

    #[test]
    fn rivers_run_down_into_the_lake() {
        let mut world = slope();
        let mut h = History::default();
        h.begin("river", 0);
        // Drawn from the lake up to the hills: it flows the other way.
        let spec = RiverSpec {
            width: 10.0,
            width_end: 30.0,
            depth: 4.0,
            banks: 8.0,
            ..RiverSpec::default()
        };
        let report = carve_river(&mut h.edit(&mut world, 0), &[(240.0, 64.0), (150.0, 40.0), (10.0, 64.0)], &spec);
        h.commit();
        assert!(report.length > 230.0, "{report:?}");
        assert!(report.source > report.mouth, "it runs down: {report:?}");
        assert_eq!(report.mouth, 75.0, "into the lake at its level");
        assert!(report.wet > 2000, "{report:?}");
        // Upstream, near the source: a bed under water, under the ground.
        let (top, fluid) = column(&world, 20, 64);
        let ground = 120.0 - 20.0 * 50.0 / 256.0;
        assert!(top < ground - 3.0, "dug: {top} under {ground}");
        assert!(fluid as f32 > top + 2.0 && (fluid as f32) < ground, "water in the bed, under the banks: {fluid}");
        // Water never rises downstream, along the middle of the course.
        let course = smooth_course(&[(240.0, 64.0), (150.0, 40.0), (10.0, 64.0)]);
        let mut last = i16::MAX;
        for &(x, z) in course.iter().rev() {
            let (_, f) = column(&world, x.floor() as i32, z.floor() as i32);
            if f == NO_FLUID {
                continue;
            }
            assert!(f <= last, "the water rises at ({x}, {z}): {f} after {last}");
            last = f;
        }
        // Wider downstream: the water reaches farther from the middle.
        let wet_across = |x: i32, z0: i32| (z0 - 20..z0 + 20).filter(|&z| column(&world, x, z).1 != NO_FLUID && column(&world, x, z).1 as f32 > column(&world, x, z).0).count();
        assert!(wet_across(200, 47) > wet_across(30, 62), "{} vs {}", wet_across(200, 47), wet_across(30, 62));
        // Far from it, nothing changed.
        assert_eq!(column(&world, 100, 120), (120.0 - 100.0 * 50.0 / 256.0, NO_FLUID));
        // Undone in one step.
        h.undo(&mut world);
        assert_eq!(column(&world, 20, 64).1, NO_FLUID);
    }

    #[test]
    fn a_branch_leaves_a_river_at_its_level() {
        let mut world = slope();
        let mut h = History::default();
        h.begin("river", 0);
        let spec = RiverSpec::default();
        carve_river(&mut h.edit(&mut world, 0), &[(10.0, 64.0), (240.0, 64.0)], &spec);
        let (_, main) = column(&world, 60, 64);
        // A branch from the river at x = 60, going south-east.
        let report = carve_river(&mut h.edit(&mut world, 0), &[(60.0, 64.0), (120.0, 110.0)], &spec);
        h.commit();
        assert_eq!(report.source.round() as i16, main, "it leaves the river at the river's level");
        let (top, fluid) = column(&world, 110, 103);
        assert!(fluid != NO_FLUID && fluid as f32 > top, "the branch holds water");
    }
}
