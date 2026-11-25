//! Roads drawn by hand: a few points become a smooth way graded into the
//! land, its sides eased back into it, its surface painted, its crossings of
//! water bridged.
//!
//! The points are joined by the curve the River tool draws ([`smooth_course`]).
//! Along it the land is read across the road's width, and the height the road
//! would like is that land, smoothed. The road then keeps to a slope limit: of
//! the heights that climb no steeper than it, it takes the one halfway between
//! the highest that never rises over the land (it only cuts) and the lowest
//! that never sinks under it (it only fills), so that it cuts through humps
//! and fills hollows alike, and follows the land where the land is gentle
//! already. Its two ends meet the land as it is.
//!
//! Across its width the road is level; past it, banks `banks` blocks wide
//! ease back to the land. Over water it is built up a block over the water,
//! or, with bridges, crosses on a deck of one block, a block over the water's
//! surface, laid on a level of its own (`Bridges`), the water left under it.
//! The selection weighs every change; the brush filters say where its surface
//! is painted.

use crate::codec::snap_height;
use crate::coords::{BlockRect, TilePos, local_index};
use crate::filter::Filter;
use crate::history::History;
use crate::level::LevelKind;
use crate::registry::TerrainId;
use crate::river::{Ground, STEP, smooth_course};
use crate::selection::lets;
use crate::tile::NO_FLUID;
use crate::world::{Dimension, World};
use std::collections::HashMap;
use std::sync::Mutex;

/// The name of the level the decks of bridges are laid on.
pub const BRIDGES: &str = "Bridges";

/// How a road is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadSpec {
    /// Width of the level way, in blocks.
    pub width: f32,
    /// Width of each bank, from the road's edge to the untouched land.
    pub banks: f32,
    /// The steepest it climbs: blocks up per block along (0.15: 15 %).
    pub grade: f32,
    /// Cross water on a deck rather than filling it.
    pub bridges: bool,
    /// Terrain of the decks.
    pub deck: Option<TerrainId>,
}

impl Default for RoadSpec {
    fn default() -> Self {
        Self {
            width: 5.0,
            banks: 6.0,
            grade: 0.15,
            bridges: true,
            deck: None,
        }
    }
}

/// Where a road runs and at what height, worked out before it is built.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoadPlan {
    /// Its course, a point every [`STEP`] blocks.
    pub course: Vec<(f32, f32)>,
    /// The height of the road at each point: one stands at this height.
    pub profile: Vec<f32>,
    /// The level of the water each point crosses (`None`: dry land).
    pub water: Vec<Option<i16>>,
    /// Whether each point is on a bridge.
    pub bridge: Vec<bool>,
    /// The steepest it climbs, blocks up per block along.
    pub steepest: f32,
    /// Whether it keeps to its slope limit everywhere (two bridges at
    /// different heights too close together may not let it).
    pub within: bool,
}

impl RoadPlan {
    /// Its length in blocks.
    pub fn length(&self) -> f32 {
        self.course.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum()
    }

    /// The blocks of it on bridges.
    pub fn bridged(&self) -> f32 {
        self.bridge.iter().filter(|b| **b).count() as f32 * STEP
    }
}

/// What building a road changed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RoadReport {
    /// Columns whose height changed, and the blocks cut and filled.
    pub columns: usize,
    pub cut: f64,
    pub fill: f64,
    /// Columns painted with the road's surface.
    pub painted: usize,
    /// Columns of bridge deck.
    pub deck: usize,
}

/// The heights over `desired` that climb at most `rise` a step, halfway
/// between the highest under it and the lowest over it, kept to the heights
/// `pins` hold where they hold one (as near as `rise` lets).
fn grade(desired: &[f32], pins: &[Option<f32>], rise: f32) -> Vec<f32> {
    let n = desired.len();
    // The largest function under `v` (min) or the smallest over it (max)
    // that climbs at most `rise` a step.
    let envelope = |v: &[f32], under: bool| -> Vec<f32> {
        let mut e = v.to_vec();
        let step = if under { rise } else { -rise };
        let pick = |a: f32, b: f32| if under { a.min(b) } else { a.max(b) };
        for k in 1..n {
            e[k] = pick(e[k], e[k - 1] + step);
        }
        for k in (0..n.saturating_sub(1)).rev() {
            e[k] = pick(e[k], e[k + 1] + step);
        }
        e
    };
    let cut = envelope(desired, true);
    let fill = envelope(desired, false);
    let up: Vec<f32> = pins.iter().map(|p| p.unwrap_or(f32::INFINITY)).collect();
    let down: Vec<f32> = pins.iter().map(|p| p.unwrap_or(f32::NEG_INFINITY)).collect();
    let (ceiling, floor) = (envelope(&up, true), envelope(&down, false));
    (0..n).map(|k| (0.5 * (cut[k] + fill[k])).max(floor[k]).min(ceiling[k])).collect()
}

/// `line` again with a point every [`STEP`] blocks along it, its ends kept.
fn even(line: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let Some(&first) = line.first() else { return Vec::new() };
    let mut out = vec![first];
    let mut want = STEP;
    let mut walked = 0.0f32;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        while len > 0.0 && walked + len >= want {
            let t = (want - walked) / len;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            want += STEP;
        }
        walked += len;
    }
    let last = *line.last().expect("a point");
    match out.last() {
        Some(p) if (p.0 - last.0).hypot(p.1 - last.1) < STEP * 0.5 => *out.last_mut().expect("a point") = last,
        _ => out.push(last),
    }
    out
}

/// Work out the road through `points` (block coordinates) over the ground
/// of `dim`.
pub fn plan_road(dim: &Dimension, points: &[(f32, f32)], spec: &RoadSpec) -> RoadPlan {
    let course = even(&smooth_course(points));
    let n = course.len();
    if n < 2 {
        return RoadPlan {
            course,
            within: true,
            ..RoadPlan::default()
        };
    }
    let lo = dim.min_y as f32;
    let hw = spec.width.max(1.0) / 2.0;
    let mut ground = Ground::new(dim.ground());
    // Across the road at each point: the mean of the dry land under it (the
    // water's surface where it is wet), and the water under its middle.
    let mut across = Vec::with_capacity(n);
    let mut water = Vec::with_capacity(n);
    for k in 0..n {
        let (a, b) = (course[k.saturating_sub(1)], course[(k + 1).min(n - 1)]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dz).max(1e-6);
        let side = (-dz / len, dx / len);
        let (x, z) = course[k];
        let (mut sum, mut count) = (0.0, 0);
        for t in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let (sx, sz) = (x + side.0 * hw * t, z + side.1 * hw * t);
            if let Some((h, w)) = ground.at(sx.floor() as i32, sz.floor() as i32) {
                sum += w.map_or(h, |w| w + 1.0);
                count += 1;
            }
        }
        across.push(if count > 0 { Some(sum / count as f32) } else { None });
        water.push(ground.at(x.floor() as i32, z.floor() as i32).and_then(|g| g.1).map(|w| w.round() as i16));
    }
    // Off the map: as the nearest point on it.
    let first = across.iter().flatten().next().copied().unwrap_or(lo);
    let mut last = first;
    let across: Vec<f32> = across
        .into_iter()
        .map(|a| {
            last = a.unwrap_or(last);
            last
        })
        .collect();
    // Smoothed along, over about its width.
    let window = ((spec.width.max(4.0) / STEP) as usize).max(2);
    let mut sums = vec![0.0f64; n + 1];
    for k in 0..n {
        sums[k + 1] = sums[k] + across[k] as f64;
    }
    let desired: Vec<f32> = (0..n)
        .map(|k| {
            let (a, b) = (k.saturating_sub(window), (k + window + 1).min(n));
            ((sums[b] - sums[a]) / (b - a) as f64) as f32
        })
        .collect();
    // Held: its ends, on the land; the decks of its bridges, a block over
    // the water.
    let bridge: Vec<bool> = water.iter().map(|w| spec.bridges && w.is_some()).collect();
    let mut pins: Vec<Option<f32>> = (0..n).map(|k| bridge[k].then(|| water[k].expect("wet") as f32 + 2.0)).collect();
    for k in [0, n - 1] {
        if pins[k].is_none() {
            pins[k] = Some(across[k]);
        }
    }
    let rise = spec.grade.max(0.005) * STEP;
    let profile = grade(&desired, &pins, rise);
    let steepest = profile.windows(2).map(|w| (w[1] - w[0]).abs() / STEP).fold(0.0f32, f32::max);
    RoadPlan {
        course,
        profile,
        water,
        bridge,
        steepest,
        within: steepest <= spec.grade.max(0.005) + 1e-3,
    }
}

/// Build the road of `plan` into dimension `dim` of `world`, as part of the
/// step `history` is recording (begun on `dim`). `paint` gives the terrain
/// of each column of the road (`None`: left as it is); `filter`, where it is
/// painted.
#[allow(clippy::too_many_arguments)]
pub fn build_road(
    history: &mut History,
    world: &mut World,
    dim: usize,
    plan: &RoadPlan,
    spec: &RoadSpec,
    paint: &(dyn Fn(i32, i32) -> Option<TerrainId> + Sync),
    filter: Option<&Filter>,
) -> RoadReport {
    let course = &plan.course;
    if course.len() < 2 || plan.profile.len() != course.len() {
        return RoadReport::default();
    }
    let hw = spec.width.max(1.0) / 2.0;
    let banks = spec.banks.max(0.0);
    let reach = hw + banks + 1.0;

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
    for &(x, z) in course {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
    }
    let rect = BlockRect::new(
        (x0 - reach).floor() as i32,
        (z0 - reach).floor() as i32,
        (x1 + reach).ceil() as i32 + 1,
        (z1 + reach).ceil() as i32 + 1,
    );

    let d = &world.dims[dim];
    let selection = d.selection.clone();
    let mask = filter.filter(|f| f.is_active()).map(|f| f.mask(d, rect));
    let (lo, hi) = (d.min_y as f32, (d.max_y - 1) as f32);
    let existing = d.levels.iter().position(|l| l.name == BRIDGES && l.kind == LevelKind::Solid);
    // The level of the decks: found, or made first, so that the step's
    // record of the levels holds the ground as it was.
    let bridges = if plan.bridge.iter().any(|b| *b) {
        Some(match existing {
            Some(k) => k,
            None => history.edit(world, 0).structure().add_level(BRIDGES, LevelKind::Solid),
        })
    } else {
        None
    };

    let weight = |x: i32, z: i32| selection.as_ref().map_or(255u8, |s| s.get(x, z));
    let decks: Mutex<Vec<(i32, i32, i16)>> = Mutex::new(Vec::new());
    let totals = Mutex::new(RoadReport::default());
    history.edit(world, 0).par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let mut r = RoadReport::default();
        let mut deck = Vec::new();
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let Some((k, d)) = nearest(x as f32 + 0.5, z as f32 + 0.5) else { continue };
                if d > hw + banks {
                    continue;
                }
                let w = weight(x, z);
                if w == 0 {
                    continue;
                }
                let i = local_index(x, z);
                let ground = tile.top.get(i);
                let fluid = tile.fluid.get(i);
                let wet = fluid != NO_FLUID && fluid as f32 > ground + 0.3;
                let on_road = d <= hw;
                if wet && plan.bridge[k] {
                    // Under a bridge: the water stays; the deck goes over it.
                    if on_road && lets(w, x, z) {
                        deck.push((x, z, plan.water[k].expect("a bridge crosses water")));
                    }
                    continue;
                }
                let road = plan.profile[k];
                let target = if on_road {
                    road
                } else {
                    let u = ((d - hw) / banks.max(0.01)).clamp(0.0, 1.0);
                    road + (ground - road) * u * u * (3.0 - 2.0 * u)
                };
                let new = snap_height((ground + (target - ground) * w as f32 / 255.0).clamp(lo, hi));
                if (new - ground).abs() >= 0.01 {
                    tile.top.set(i, new);
                    r.columns += 1;
                    if new > ground {
                        r.fill += (new - ground) as f64;
                    } else {
                        r.cut += (ground - new) as f64;
                    }
                    // Built up out of the water: dry.
                    if wet && new + 0.3 >= fluid as f32 {
                        tile.fluid.set(i, NO_FLUID);
                    }
                }
                if on_road && lets(w, x, z) {
                    let allowed = mask.as_ref().is_none_or(|m| lets((m.get(x, z) * 255.0).round() as u8, x, z));
                    if allowed && let Some(t) = paint(x, z) {
                        tile.terrain.set(i, t.0);
                        r.painted += 1;
                    }
                    // Nothing grows on a road.
                    tile.cover.set(i, 0);
                    tile.stacked.clear_kind(i, crate::layer_stack::Kind::Cover);
                }
                if d <= hw + 1.0 && lets(w, x, z) {
                    tile.objects.set(i, 0);
                }
            }
        }
        tile.compact();
        let mut t = totals.lock().expect("not poisoned");
        t.columns += r.columns;
        t.cut += r.cut;
        t.fill += r.fill;
        t.painted += r.painted;
        if !deck.is_empty() {
            decks.lock().expect("not poisoned").extend(deck);
        }
    });
    let mut report = totals.into_inner().expect("not poisoned");

    let decks = decks.into_inner().expect("not poisoned");
    if let (Some(level), false) = (bridges, decks.is_empty()) {
        let mut by_tile: HashMap<TilePos, Vec<(usize, i16)>> = HashMap::new();
        for &(x, z, water) in &decks {
            by_tile.entry(TilePos::containing(x, z)).or_default().push((local_index(x, z), water));
        }
        let deck = spec.deck.map_or(0, |t| t.0);
        let positions: Vec<TilePos> = by_tile.keys().copied().collect();
        history.edit(world, level).par_tiles(positions, rect, true, |pos, _, tile| {
            let Some(bottom) = tile.bottom.as_mut() else { return };
            for &(i, water) in &by_tile[&pos] {
                let under = (water as f32 + 1.0).clamp(lo, hi);
                bottom.set(i, under);
                tile.top.set(i, (under + 1.0).min(hi + 1.0));
                tile.terrain.set(i, deck);
            }
            tile.compact();
        });
        report.deck = decks.len();
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Selection, Shape};
    use crate::tile::Tile;
    use crate::world::{DimensionKind, WorldMeta};

    /// 3 × 1 tiles of land: flat at 70 in the west, a hill rising to 110 in
    /// the middle, a lake (water at 64 over a bed at 56) in the east from
    /// x = 300 to 340, flat at 70 again past it.
    fn land() -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..3 {
            let mut tile = Tile::ground(70.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let bx = tx * 128 + x;
                    let i = local_index(bx, z);
                    let hill = (40.0 - (bx - 170).abs() as f32 * 0.6).max(0.0);
                    tile.top.set(i, 70.0 + hill);
                    if (300..340).contains(&bx) {
                        tile.top.set(i, 56.0);
                        tile.fluid.set(i, 64);
                    }
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        world
    }

    fn column(world: &World, level: usize, x: i32, z: i32) -> (f32, i16, u16) {
        let tile = world.dims[0].levels[level].view(TilePos::containing(x, z)).unwrap();
        let i = local_index(x, z);
        (tile.top.get(i), tile.fluid.get(i), tile.terrain.get(i))
    }

    fn build(world: &mut World, history: &mut History, points: &[(f32, f32)], spec: &RoadSpec, paint: Option<TerrainId>) -> (RoadPlan, RoadReport) {
        let plan = plan_road(&world.dims[0], points, spec);
        history.begin("Road", 0);
        let report = build_road(history, world, 0, &plan, spec, &|_, _| paint, None);
        history.commit();
        (plan, report)
    }

    #[test]
    fn the_grade_climbs_no_steeper_than_the_limit() {
        let desired = [0.0, 0.0, 10.0, 10.0, 0.0, 0.0, 0.0, 30.0];
        let pins = [Some(0.0), None, None, None, None, None, None, Some(30.0)];
        let p = grade(&desired, &pins, 2.0);
        assert!(p.windows(2).all(|w| (w[1] - w[0]).abs() <= 2.0 + 1e-4), "{p:?}");
        assert_eq!(p[0], 0.0);
        // As near the far end as the limit lets.
        assert_eq!(p[7], 14.0);
        // Gentle land is followed as it is.
        let flat = grade(&[5.0; 6], &[Some(5.0), None, None, None, None, Some(5.0)], 1.0);
        assert_eq!(flat, vec![5.0; 6]);
    }

    /// Over the hill: the road climbs no steeper than its limit, cutting the
    /// top and filling the foot; its ends meet the land; its surface is
    /// painted; it is undone in one step.
    #[test]
    fn a_road_is_graded_over_a_hill() {
        let mut world = land();
        let mut h = History::default();
        let spec = RoadSpec {
            width: 6.0,
            banks: 6.0,
            grade: 0.2,
            bridges: false,
            deck: None,
        };
        let (plan, report) = build(&mut world, &mut h, &[(20.0, 64.0), (170.0, 64.0), (280.0, 64.0)], &spec, Some(TerrainId(7)));
        assert!(plan.within && plan.steepest <= 0.2 + 1e-3, "{}", plan.steepest);
        assert!(report.cut > 1000.0 && report.fill > 100.0, "{report:?}");
        // Along the middle, the ground climbs no steeper than the limit
        // (give or take the block the columns are rounded to).
        let mut last: Option<(f32, f32, f32)> = None;
        for &(x, z) in plan.course.iter().step_by(8) {
            let (top, _, terrain) = column(&world, 0, x.floor() as i32, z.floor() as i32);
            assert_eq!(terrain, 7, "painted at ({x}, {z})");
            if let Some((px, pz, ph)) = last {
                let run = (x - px).hypot(z - pz);
                assert!((top - ph).abs() <= 0.2 * (run + 1.5) + 0.01, "too steep at ({x}, {z}): {ph} → {top} over {run}");
            }
            last = Some((x, z, top));
        }
        // The top of the hill is cut.
        let (top, _, _) = column(&world, 0, 170, 64);
        assert!(top < 105.0, "cut: {top}");
        // The ends meet the land.
        assert!((column(&world, 0, 20, 64).0 - 70.0).abs() < 0.6, "{:?}", column(&world, 0, 20, 64));
        assert!((column(&world, 0, 280, 64).0 - 70.0).abs() < 0.6);
        // Beside it, the banks ease back to the land; far from it, nothing
        // changed.
        assert_eq!(column(&world, 0, 170, 64 + 3 + 7), (110.0, NO_FLUID, 1));
        assert_eq!(column(&world, 0, 160, 120).0, 104.0);
        // Undone in one step.
        h.undo(&mut world);
        assert_eq!(column(&world, 0, 170, 64), (110.0, NO_FLUID, 1));
        assert_eq!(column(&world, 0, 60, 64).2, 1);
    }

    /// Over the lake with bridges: a deck a block over the water, on a level
    /// of its own, the water left under it; the road climbs down to it no
    /// steeper than its limit. Without bridges, the road is built up out of
    /// the water.
    #[test]
    fn a_road_crosses_water_on_a_bridge() {
        let mut world = land();
        let mut h = History::default();
        let spec = RoadSpec {
            width: 5.0,
            banks: 4.0,
            grade: 0.25,
            bridges: true,
            deck: Some(TerrainId(9)),
        };
        let (plan, report) = build(&mut world, &mut h, &[(260.0, 40.0), (320.0, 40.0), (380.0, 40.0)], &spec, Some(TerrainId(7)));
        assert!(plan.bridged() > 35.0, "{}", plan.bridged());
        assert!(report.deck > 150, "{report:?}");
        let levels = &world.dims[0].levels;
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[1].name.as_str(), levels[1].kind), (BRIDGES, LevelKind::Solid));
        // The deck: one block, at Y 65 (the water's surface is 64).
        let tile = levels[1].view(TilePos::containing(320, 40)).unwrap();
        let i = local_index(320, 40);
        assert_eq!((tile.bottom.as_ref().unwrap().get(i), tile.top.get(i), tile.terrain.get(i)), (65.0, 66.0, 9));
        // The water stays under it; the lake bed is as it was.
        assert_eq!(column(&world, 0, 320, 40), (56.0, 64, 1));
        // The shore climbs to the deck: at the water's edge, the road is at
        // the deck's height, give or take the grade over the last blocks.
        assert!((column(&world, 0, 298, 40).0 - 66.0).abs() <= 0.25 * 2.5, "{:?}", column(&world, 0, 298, 40));
        // One step: undone, no deck and no level.
        h.undo(&mut world);
        assert_eq!(world.dims[0].levels.len(), 1);
        assert_eq!(column(&world, 0, 297, 40).0, 70.0);

        // Without bridges: the road is built up a block over the water.
        let spec = RoadSpec { bridges: false, ..spec };
        let (_, report) = build(&mut world, &mut h, &[(260.0, 40.0), (320.0, 40.0), (380.0, 40.0)], &spec, Some(TerrainId(7)));
        assert_eq!(report.deck, 0);
        assert_eq!(world.dims[0].levels.len(), 1);
        let (top, fluid, terrain) = column(&world, 0, 320, 40);
        assert!(top >= 64.9 && fluid == NO_FLUID && terrain == 7, "{top} {fluid} {terrain}");
        // Beside it, the lake is still there.
        assert_eq!(column(&world, 0, 320, 60), (56.0, 64, 1));
    }

    /// A selection holds the road: outside it nothing changes.
    #[test]
    fn a_road_keeps_to_the_selection() {
        let mut world = land();
        world.dims[0].selection = Some(std::sync::Arc::new(Selection::shape(&Shape::Rect(BlockRect::new(0, 0, 150, 128)))));
        let mut h = History::default();
        let spec = RoadSpec { bridges: false, ..RoadSpec::default() };
        build(&mut world, &mut h, &[(20.0, 64.0), (170.0, 64.0), (280.0, 64.0)], &spec, Some(TerrainId(7)));
        assert_eq!(column(&world, 0, 60, 64).2, 7, "painted inside");
        assert_eq!(column(&world, 0, 170, 64), (110.0, NO_FLUID, 1), "untouched outside");
        assert_eq!(column(&world, 0, 200, 64).2, 1);
    }

    /// The brush filters say where the surface is painted.
    #[test]
    fn the_filters_choose_where_it_is_painted() {
        let mut world = land();
        let mut h = History::default();
        let spec = RoadSpec { bridges: false, ..RoadSpec::default() };
        let plan = plan_road(&world.dims[0], &[(20.0, 64.0), (170.0, 64.0)], &spec);
        let filter = Filter { below: Some(80), ..Filter::default() };
        h.begin("Road", 0);
        build_road(&mut h, &mut world, 0, &plan, &spec, &|_, _| Some(TerrainId(7)), Some(&filter));
        h.commit();
        assert_eq!(column(&world, 0, 40, 64).2, 7, "low: painted");
        assert_eq!(column(&world, 0, 165, 64).2, 1, "high: not painted");
    }
}
