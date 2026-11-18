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
    /// Depth of the water in the middle of the bed, at the source and at
    /// the mouth.
    pub depth: f32,
    pub depth_end: f32,
    /// Width of each bank, from the water's edge to the untouched ground.
    pub banks: f32,
    /// Fill the bed with water.
    pub water: bool,
    /// Terrain of the bed under the water (`None`: left as it is).
    pub bed: Option<TerrainId>,
    /// Terrain of the shallows: the edges of the bed, the inside of its
    /// bends and the foot of the bank there, and its channel under the sea
    /// (`None`: the bed's). Only with a `bed`.
    pub shore: Option<TerrainId>,
    /// Terrain of the patches of the bed where the water runs slowly, on
    /// gentle ground (`None`: the bed's). Only with a `bed`.
    pub clay: Option<TerrainId>,
    /// The level of the sea it runs into, if it does: its water never goes
    /// under it, even where it reaches the sea over dry land lower than the
    /// sea's surface (a beach at the water's level); and its channel goes on
    /// under the sea, fading into the sea floor.
    pub sea: Option<f32>,
    /// Biome of the water (`None`: left as it is).
    pub biome: Option<BiomeId>,
}

impl Default for RiverSpec {
    fn default() -> Self {
        Self {
            width: 12.0,
            width_end: 12.0,
            depth: 4.0,
            depth_end: 4.0,
            banks: 10.0,
            water: true,
            bed: None,
            shore: None,
            clay: None,
            sea: None,
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
pub const STEP: f32 = 0.5;

/// The curve through `points` (centripetal Catmull-Rom: no loop nor cusp
/// where it turns sharply), a point every [`STEP`] blocks.
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
    // Past its ends, points as far on the way out: no bunching there.
    let n = pts.len() as isize;
    let at = |k: isize| match k {
        -1 => (2.0 * pts[0].0 - pts[1].0, 2.0 * pts[0].1 - pts[1].1),
        k if k == n => (2.0 * pts[(n - 1) as usize].0 - pts[(n - 2) as usize].0, 2.0 * pts[(n - 1) as usize].1 - pts[(n - 2) as usize].1),
        k => pts[k.clamp(0, n - 1) as usize],
    };
    let mut out = Vec::new();
    for i in 0..pts.len() - 1 {
        let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
        let len = (p2.0 - p1.0).hypot(p2.1 - p1.1);
        let steps = (len / STEP).ceil().max(1.0) as usize;
        // Knots spaced as the square root of the distances.
        let gap = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1).sqrt().max(1e-3);
        let (t0, t1) = (0.0, gap(p0, p1));
        let t2 = t1 + gap(p1, p2);
        let t3 = t2 + gap(p2, p3);
        let lerp = |a: (f32, f32), b: (f32, f32), ta: f32, tb: f32, t: f32| {
            let (u, v) = ((tb - t) / (tb - ta), (t - ta) / (tb - ta));
            (a.0 * u + b.0 * v, a.1 * u + b.1 * v)
        };
        let curve = |s: f32| {
            let t = t1 + (t2 - t1) * s;
            let (a1, a2, a3) = (lerp(p0, p1, t0, t1, t), lerp(p1, p2, t1, t2, t), lerp(p2, p3, t2, t3, t));
            let (b1, b2) = (lerp(a1, a2, t0, t2, t), lerp(a2, a3, t1, t3, t));
            lerp(b1, b2, t1, t2, t)
        };
        // Its pace varies: points spaced evenly along it, as the levels
        // along the course count them, read off a finer sampling.
        let fine: Vec<(f32, f32)> = (0..=steps * 4).map(|s| curve(s as f32 / (steps * 4) as f32)).collect();
        let mut run = vec![0.0f32];
        for w in fine.windows(2) {
            run.push(run.last().expect("a start") + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let total = *run.last().expect("a start");
        let mut f = 0;
        for s in 0..steps {
            let want = total * s as f32 / steps as f32;
            while f + 1 < run.len() - 1 && run[f + 1] < want {
                f += 1;
            }
            let span = (run[f + 1] - run[f]).max(1e-6);
            let u = ((want - run[f]) / span).clamp(0.0, 1.0);
            out.push((fine[f].0 + (fine[f + 1].0 - fine[f].0) * u, fine[f].1 + (fine[f + 1].1 - fine[f].1) * u));
        }
    }
    out.push(*pts.last().expect("two points or more"));
    out
}

/// Reads the ground of a level at single columns, decoding each tile once.
pub(crate) struct Ground<'a> {
    level: &'a crate::level::Level,
    tiles: HashMap<TilePos, Option<TileRef<'a>>>,
}

impl<'a> Ground<'a> {
    pub(crate) fn new(level: &'a crate::level::Level) -> Self {
        Self { level, tiles: HashMap::new() }
    }

    /// The height of the ground at `(x, z)`, and the level of its water if
    /// it is under water.
    pub(crate) fn at(&mut self, x: i32, z: i32) -> Option<(f32, Option<f32>)> {
        let pos = TilePos::containing(x, z);
        let level = self.level;
        let tile = self.tiles.entry(pos).or_insert_with(|| level.view(pos)).as_ref()?;
        let i = local_index(x, z);
        let top = tile.top.get(i);
        let fluid = tile.fluid.get(i);
        Some((top, (fluid != NO_FLUID && fluid as f32 > top + 0.3).then_some(fluid as f32)))
    }
}

/// Blocks ahead a river looks to tell a steep stretch from a gentle one.
const AHEAD: f32 = 24.0;
/// Blocks it drops over [`AHEAD`] from which the stretch is steep (a grade
/// of some 30 %): there it falls from pool to pool rather than stepping
/// down a block at a time.
const STEEP: i32 = 7;
/// The highest fall.
const FALL_MAX: i32 = 5;

/// The level of the water along a river (`level`, a point every `step`
/// blocks from the source, never rising), made into pools and falls where
/// the land is steep: rather than a step of a block every few blocks, the
/// water stays level in a pool dug into the slope, then falls two to five
/// blocks into the next one — the steeper, the higher the falls. Gentle
/// stretches keep their steps of a block, rapids. Points `keep` names are
/// left as they are (water already on the map). Returns, for each point,
/// how much deeper its bed is dug: the plunge pool at the foot of each
/// fall, as deep as most of the fall and as long as the river is wide
/// (`half`: half its width at each point, in blocks).
pub(crate) fn pools_and_falls(level: &mut [f32], step: f32, half: &dyn Fn(usize) -> f32, keep: &dyn Fn(usize) -> bool) -> Vec<f32> {
    let n = level.len();
    let mut plunge = vec![0.0f32; n];
    if n < 2 {
        return plunge;
    }
    let block: Vec<i32> = level.iter().map(|l| l.round() as i32).collect();
    let ahead = ((AHEAD / step).round() as usize).max(1);
    // Never under the water it ends in, nor under that of the next lake or
    // river it crosses.
    let mut floor = vec![block[n - 1]; n];
    for k in (0..n - 1).rev() {
        floor[k] = if keep(k + 1) { block[k + 1] } else { floor[k + 1] };
    }
    // The pool the river is in on a steep stretch, and the level of the
    // water at the last point.
    let mut pool: Option<i32> = None;
    let mut last = block[0];
    for k in 0..n {
        if keep(k) {
            pool = None;
            last = block[k];
            continue;
        }
        let allowed = block[k];
        if let Some(p) = pool
            && allowed >= p
        {
            level[k] = p as f32;
            last = p;
            continue;
        }
        let later = block[(k + ahead).min(n - 1)];
        let drop = allowed - later;
        if drop < STEEP {
            pool = None;
            last = allowed;
            continue;
        }
        let height = (drop / 2).clamp(2, FALL_MAX);
        let p = (allowed - (height - 1)).max(later).max(floor[k]);
        let fall = last - p;
        pool = Some(p);
        level[k] = p as f32;
        last = p;
        if fall >= 2 {
            let deeper = (fall as f32 * 0.6).min(3.0);
            let reach = ((2.0 * half(k)).max(4.0) / step).ceil() as usize;
            for (j, d) in plunge.iter_mut().enumerate().skip(k).take(reach) {
                let t = (j - k) as f32 / reach as f32;
                *d = d.max(deeper * (1.0 - t * t));
            }
        }
    }
    plunge
}

/// The way a river's channel goes on under the sea from the end of
/// `course` (a point every [`STEP`] blocks): down the sea floor, keeping
/// more or less to its heading, until the floor lies as deep as its bed
/// (`depth` under the `sea`: there the channel has faded into it), it
/// comes to land again (an island), the map ends, or it has run six times
/// its `width` (24 to 128 blocks).
fn under_sea(ground: &mut Ground, course: &[(f32, f32)], sea: f32, depth: f32, width: f32) -> Vec<(f32, f32)> {
    let n = course.len();
    if n < 2 {
        return Vec::new();
    }
    let back = course[n.saturating_sub(1 + (8.0 / STEP) as usize)];
    let last = course[n - 1];
    let (mut hx, mut hz) = (last.0 - back.0, last.1 - back.1);
    let l = hx.hypot(hz);
    if l < 1e-3 {
        return Vec::new();
    }
    (hx, hz) = (hx / l, hz / l);
    let floor = |g: &mut Ground, x: f32, z: f32| g.at(x.floor() as i32, z.floor() as i32).map(|(h, _)| h);
    let bed = sea - depth;
    let longest = (6.0 * width).clamp(24.0, 128.0);
    let (mut x, mut z) = last;
    let mut out = Vec::new();
    let mut run = 0.0;
    while run < longest {
        // Of a few ways ahead, the one down the floor the most; straight
        // on, at the same depth.
        let mut best: Option<(f32, (f32, f32))> = None;
        for a in [-0.5f32, -0.25, 0.0, 0.25, 0.5] {
            let (s, c) = a.sin_cos();
            let (dx, dz) = (hx * c - hz * s, hx * s + hz * c);
            let Some(h) = floor(ground, x + 4.0 * dx, z + 4.0 * dz) else { continue };
            let score = h + 0.8 * a.abs();
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, (dx, dz)));
            }
        }
        let Some((_, (dx, dz))) = best else { break };
        // Keeping most of its heading: no sharp turn under the water.
        let (nx, nz) = (0.8 * hx + 0.2 * dx, 0.8 * hz + 0.2 * dz);
        let nl = nx.hypot(nz).max(1e-6);
        (hx, hz) = (nx / nl, nz / nl);
        x += hx * STEP;
        z += hz * STEP;
        run += STEP;
        match floor(ground, x, z) {
            // Over the shallows of a beach, out to the deep.
            Some(h) if h < sea + 0.5 => {
                out.push((x, z));
                if h < bed - 1.0 {
                    break;
                }
            }
            // Land again, or the edge of the map.
            _ => break,
        }
    }
    out
}

/// How much land drains through each of the first `n` points of `course`,
/// in square blocks, as water finds its way over the ground around it: a
/// grid of cells over the course and 192 blocks around (4 blocks a side,
/// coarser to keep it under 250,000 cells) is flooded from its edges and
/// from what lies under the `sea`, lowest first, every cell draining to the
/// one the flood reached it from; the cells upstream of each are summed.
/// Each point takes the most of the cells around it (it lies on the flow,
/// not beside it), and never less than the point before: downstream, a
/// river only gathers water.
fn catchment(ground: &mut Ground, course: &[(f32, f32)], n: usize, sea: f32) -> Vec<f32> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    const MARGIN: f32 = 192.0;
    let pts = &course[..n.min(course.len())];
    if pts.is_empty() {
        return Vec::new();
    }
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, z) in pts {
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
    }
    let (x0, z0) = (x0 - MARGIN, z0 - MARGIN);
    let (w, d) = (x1 + MARGIN - x0, z1 + MARGIN - z0);
    let cell = ((w * d / 250_000.0).sqrt().ceil()).max(4.0);
    let (nx, nz) = ((w / cell).ceil() as usize + 1, (d / cell).ceil() as usize + 1);
    // The surface of each cell: the water's where there is water; none off
    // the map.
    let h: Vec<f32> = (0..nx * nz)
        .map(|c| {
            let (x, z) = (x0 + ((c % nx) as f32 + 0.5) * cell, z0 + ((c / nx) as f32 + 0.5) * cell);
            ground.at(x.floor() as i32, z.floor() as i32).map_or(f32::NAN, |(g, w)| w.unwrap_or(g))
        })
        .collect();
    #[derive(PartialEq)]
    struct Key(f32);
    impl Eq for Key {}
    impl Ord for Key {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    impl PartialOrd for Key {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    const NONE: u32 = u32::MAX;
    let mut receiver = vec![NONE; nx * nz];
    let mut done = vec![false; nx * nz];
    let mut order: Vec<u32> = Vec::with_capacity(nx * nz);
    let mut heap: BinaryHeap<Reverse<(Key, u32)>> = BinaryHeap::new();
    // The edges of the grid, the edges of the map and the sea drain away;
    // past the map, nothing.
    let off = |i: i32, j: i32| i < 0 || j < 0 || i >= nx as i32 || j >= nz as i32 || h[j as usize * nx + i as usize].is_nan();
    for c in 0..nx * nz {
        let (i, j) = ((c % nx) as i32, (c / nx) as i32);
        if h[c].is_nan() {
            done[c] = true;
        } else if i == 0 || j == 0 || i == nx as i32 - 1 || j == nz as i32 - 1 || h[c] < sea || AROUND.iter().any(|&(di, dj)| off(i + di, j + dj)) {
            done[c] = true;
            heap.push(Reverse((Key(h[c]), c as u32)));
        }
    }
    let mut filled = vec![f32::NAN; nx * nz];
    while let Some(Reverse((Key(level), c))) = heap.pop() {
        let c = c as usize;
        order.push(c as u32);
        filled[c] = level;
        let (i, j) = ((c % nx) as i32, (c / nx) as i32);
        for (di, dj) in AROUND {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= nx as i32 || b >= nz as i32 {
                continue;
            }
            let nb = b as usize * nx + a as usize;
            if done[nb] {
                continue;
            }
            done[nb] = true;
            receiver[nb] = c as u32;
            heap.push(Reverse((Key(h[nb].max(level)), nb as u32)));
        }
    }
    // Down the slope, each cell drains to the steepest of the cells around
    // it (the flood alone would send the water of an even slope along its
    // ties); where it lies flat or in a hollow, the way the flood found.
    for &c in &order {
        let c = c as usize;
        if receiver[c] == NONE {
            continue;
        }
        let (i, j) = ((c % nx) as i32, (c / nx) as i32);
        let mut best: Option<(f32, usize)> = None;
        for (di, dj) in AROUND {
            let (a, b) = (i + di, j + dj);
            if a < 0 || b < 0 || a >= nx as i32 || b >= nz as i32 {
                continue;
            }
            let nb = b as usize * nx + a as usize;
            let drop = (filled[c] - filled[nb]) / if di != 0 && dj != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            if drop > 0.0 && best.is_none_or(|(d, _)| drop > d) {
                best = Some((drop, nb));
            }
        }
        if let Some((_, nb)) = best {
            receiver[c] = nb as u32;
        }
    }
    let mut area = vec![cell * cell; nx * nz];
    for &c in order.iter().rev() {
        let r = receiver[c as usize];
        if r != NONE {
            area[r as usize] += area[c as usize];
        }
    }
    let mut most = 0.0f32;
    pts.iter()
        .map(|&(x, z)| {
            let (i, j) = (((x - x0) / cell) as i32, ((z - z0) / cell) as i32);
            let mut a = 0.0f32;
            for dj in -1..=1 {
                for di in -1..=1 {
                    let (p, q) = (i + di, j + dj);
                    if p >= 0 && q >= 0 && (p as usize) < nx && (q as usize) < nz {
                        a = a.max(area[q as usize * nx + p as usize]);
                    }
                }
            }
            most = most.max(a);
            most
        })
        .collect()
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
    let depth = spec.depth.max(spec.depth_end).max(0.5);
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
    // Into the sea, its channel goes on under the water, down the sea
    // floor: the points from `coast` on are under the sea.
    let coast = course.len();
    if let Some(sea) = spec.sea
        && land.last().is_some_and(|l| l.0 < sea + 0.5)
    {
        for p in under_sea(&mut ground, &course, sea, spec.depth.max(spec.depth_end), spec.width.max(spec.width_end)) {
            land.push(ground.at(p.0.floor() as i32, p.1.floor() as i32).unwrap_or((lo, Some(sea))));
            course.push(p);
        }
    }
    let n = course.len();
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
    // Never under the water it runs into: that at its end, or the sea's.
    let mouth = land.last().and_then(|l| l.1).or(spec.sea);
    if let Some(mouth) = mouth {
        for l in &mut level {
            *l = l.max(mouth);
        }
    }
    // Nor under the water it crosses further down: a lake on its way
    // (filled before the river is dug) stands at its own level, and the
    // river above it may not stand lower, or the lake would stand over the
    // river that feeds it.
    let mut below = f32::MIN;
    for k in (0..level.len()).rev() {
        if let Some(w) = land[k].1 {
            below = below.max(w);
        }
        level[k] = level[k].max(below);
    }
    let length = coast as f32 * STEP;
    // From the source to the mouth: under the sea, as at the mouth.
    // From the source to the mouth, wider and deeper as the land draining
    // through it grows (as the square root of its catchment's share of the
    // mouth's): where a tributary joins, it widens over some thirty blocks
    // downstream. Under the sea, as at the mouth.
    let gathered = catchment(&mut ground, &course, coast, spec.sea.unwrap_or(f32::MIN));
    let at_mouth = gathered.last().copied().unwrap_or(1.0).max(1.0);
    let raw: Vec<f32> = (0..coast).map(|k| (gathered.get(k).copied().unwrap_or(at_mouth) / at_mouth).sqrt().clamp(0.0, 1.0)).collect();
    // The mean over the points behind each: a ramp, still never narrower
    // downstream.
    let behind = (30.0 / STEP) as usize;
    let mut sums = vec![0.0f32; coast + 1];
    for k in 0..coast {
        sums[k + 1] = sums[k] + raw[k];
    }
    let ramp: Vec<f32> = (0..coast).map(|k| (sums[k + 1] - sums[k.saturating_sub(behind)]) / (k + 1 - k.saturating_sub(behind)) as f32).collect();
    // As wide at its mouth as set.
    let last = ramp.last().copied().unwrap_or(1.0).max(1e-6);
    let share: Vec<f32> = ramp.iter().map(|r| (r / last).min(1.0)).collect();
    let along = |k: usize, a: f32, b: f32| a + (b - a) * share[k.min(coast - 1)];
    // How far under the sea a point is, 0 at the mouth to 1 where the
    // channel has faded into the floor.
    let out = |k: usize| if k < coast { 0.0 } else { (k + 1 - coast) as f32 / (n - coast) as f32 };
    // How steep the land is along the course, over 16 blocks either way:
    // 0 on the flat, 1 from a grade of 12 % up. A river is narrow and cut
    // in a V where the land is steep, wide over a flat bed in the plains.
    let ahead = (16.0 / STEP) as usize;
    let steep: Vec<f32> = (0..n)
        .map(|k| {
            let (i, j) = (k.saturating_sub(ahead), (k + ahead).min(n - 1));
            let run = ((j - i) as f32 * STEP).max(1.0);
            ((smooth[i] - smooth[j]) / run / 0.12).clamp(0.0, 1.0)
        })
        .collect();
    // Its width wanders a little, over some fifty blocks, as a real
    // river's does.
    let wobble = crate::noise::Noise::new(0x71de_5a17);
    // Under the sea the channel opens out a little, a third wider at most.
    let widths: Vec<f32> = (0..n)
        .map(|k| {
            let natural = (1.1 - 0.35 * steep[k]) * (1.0 + 0.12 * wobble.fbm(k as f64 * STEP as f64 / 48.0, 0.5, 2, 2.0, 0.5) as f32);
            half(along(k, spec.width, spec.width_end) * if k < coast { natural } else { 1.0 }) * (1.0 + 0.3 * out(k))
        })
        .collect();
    // How the course bends at each point: the sine of its turn over a few
    // blocks, positive to the left. On the inside of a bend the river
    // leaves sand.
    let span = 8usize;
    let bend: Vec<f32> = (0..n)
        .map(|k| {
            let (a, b, c) = (course[k.saturating_sub(span)], course[k], course[(k + span).min(n - 1)]);
            let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
            let norm = (u.0.hypot(u.1) * v.0.hypot(v.1)).max(1e-6);
            (u.0 * v.1 - u.1 * v.0) / norm
        })
        .collect();
    // Pools and falls down the steep stretches, a plunge pool under each
    // fall; through the water it crosses, it keeps that water's level.
    let crossed: Vec<bool> = land.iter().map(|l| l.1.is_some()).collect();
    let plunge = pools_and_falls(&mut level, STEP, &|k| widths[k], &|k| crossed[k]);
    // Under a fall, its pool rounds out, a quarter wider.
    let widths: Vec<f32> = widths.iter().zip(&plunge).map(|(w, p)| w * (1.0 + 0.25 * (p / 3.0).min(1.0))).collect();
    // The valley's floor along the river, for its banks: the water's level
    // smoothed over some twenty blocks either way (the banks of each pool
    // would make a terrace of it), never under the water.
    let floor_window = ((20.0 / STEP) as usize).max(1);
    let mut level_sums = vec![0.0f32; n + 1];
    for k in 0..n {
        level_sums[k + 1] = level_sums[k] + level[k];
    }
    let valley: Vec<f32> = (0..n)
        .map(|k| {
            let (a, b) = (k.saturating_sub(floor_window), (k + floor_window + 1).min(n));
            ((level_sums[b] - level_sums[a]) / (b - a) as f32).max(level[k])
        })
        .collect();
    let depths: Vec<f32> = (0..n).map(|k| (along(k, spec.depth, spec.depth_end) * (1.0 - 0.25 * steep[k])).max(0.5) + plunge[k]).collect();
    // The shape of its bed across: flat with rounded sides on the flat,
    // a V where the land is steep.
    let shape: Vec<f32> = steep.iter().map(|s| 3.0 - 1.8 * s).collect();
    // Its banks: as wide as set at least, and as wide as two and a half
    // times as deep as the river lies under the land around (48 blocks at
    // most): a valley sloping down to the water, not the walls of a trench.
    let banks: Vec<f32> = (0..n).map(|k| if k < coast { spec.banks.max(0.0).max((2.5 * (smooth[k] - valley[k])).min(48.0)) } else { 0.0 }).collect();
    // Their width smoothed along the river too: one valley, not a bank
    // for each pool.
    let banks: Vec<f32> = {
        let mut sums = vec![0.0f32; n + 1];
        for k in 0..n {
            sums[k + 1] = sums[k] + banks[k];
        }
        (0..n)
            .map(|k| {
                if k >= coast {
                    return 0.0;
                }
                let (a, b) = (k.saturating_sub(floor_window), (k + floor_window + 1).min(coast));
                (sums[b] - sums[a]) / (b - a) as f32
            })
            .collect()
    };
    let reach = reach.max((0..n).map(|k| widths[k] * if k < coast { 1.0 } else { 1.6 } + banks[k]).fold(0.0f32, f32::max) + 1.0);
    // Under the sea: how deep the channel is cut into the floor, from its
    // bed at the mouth to nothing where it fades out.
    let incision: Vec<f32> = (0..n)
        .map(|k| match spec.sea {
            Some(sea) if k >= coast => (land[k].0 - (sea - depths[coast - 1])).max(0.0) * (1.0 - out(k)),
            _ => 0.0,
        })
        .collect();

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
    let patches = crate::noise::Noise::new(0xc1a7_0b5e);
    let counts = std::sync::Mutex::new((0usize, 0usize));
    // The ground of the columns dug, as it was: the banks may rise back
    // to it to hold the water.
    let dug = std::sync::Mutex::new(HashMap::new());
    // The columns whose water is the river's.
    let poured = std::sync::Mutex::new(std::collections::HashSet::new());
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let (mut changed, mut wet) = (0, 0);
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
                let Some((k, d)) = nearest(cx, cz) else { continue };
                let hw = widths[k];
                if d > hw + banks[k] {
                    continue;
                }
                let i = local_index(x, z);
                let ground = tile.top.get(i);
                if k >= coast {
                    // Under the sea: a channel cut into the floor, fading
                    // out; sand on its floor and in a fan where it ends,
                    // the river's biome thinning out into the sea's.
                    let t = d / hw;
                    let f = out(k);
                    let mut any = false;
                    if t < 1.0 {
                        let new = snap_height((ground - incision[k] * (1.0 - t * t)).clamp(lo, hi));
                        if new < ground {
                            tile.top.set(i, new);
                            dug.lock().expect("not poisoned").insert((x, z), ground);
                            any = true;
                        }
                        if let Some(biome) = spec.biome
                            && (crate::compose::hash2(x, z, 0x51ea) % 1000) as f32 > 1000.0 * f
                        {
                            tile.biome.set(i, biome.0);
                            any = true;
                        }
                    }
                    if let (Some(_), Some(shore)) = (spec.bed, spec.shore)
                        && (t < 1.0 || (f > 0.6 && t < 1.6))
                        && tile.terrain.get(i) != shore.0
                    {
                        tile.terrain.set(i, shore.0);
                        any = true;
                    }
                    changed += any as usize;
                    continue;
                }
                // The sea, a lake or a river already there: the river runs
                // into it and leaves its floor, its terrain and its water as
                // they are.
                let before = tile.fluid.get(i);
                if before != NO_FLUID && before as f32 > ground + 0.3 {
                    continue;
                }
                let water = level[k];
                let target = if d <= hw {
                    // The bed: deepest in the middle, up to the water's
                    // edge at the sides.
                    let t = d / hw;
                    water - depths[k] * (1.0 - t.powf(shape[k]))
                } else {
                    // The bank: from just over the water up to the ground.
                    let u = ((d - hw) / banks[k].max(0.01)).clamp(0.0, 1.0);
                    let u = u * u * (3.0 - 2.0 * u);
                    // As high as the highest water beside it (upstream
                    // of its nearest point, the water may stand a step
                    // higher), and the valley's floor there: the banks
                    // ramp down past a fall rather than step.
                    let edge = level[k.saturating_sub(((hw + 2.0) / STEP) as usize)].max(valley[k]) + 0.6;
                    edge + (ground - edge).max(0.0) * u
                };
                let new = snap_height(target.min(ground).clamp(lo, hi));
                if new < ground {
                    tile.top.set(i, new);
                    changed += 1;
                    dug.lock().expect("not poisoned").insert((x, z), ground);
                }
                // The shallows: the edges of the bed, and the inside of a
                // bend (the side the course turns towards).
                let inner = {
                    let (a, b) = (course[k.saturating_sub(1)], course[(k + 1).min(n - 1)]);
                    let side = (b.0 - a.0) * (cz - course[k].1) - (b.1 - a.1) * (cx - course[k].0);
                    bend[k].abs() > 0.12 && side * bend[k] > 0.0
                };
                let t = d / hw;
                let shallow = t > 0.7 || (inner && t > 0.3);
                if d <= hw && spec.water && water > new + 0.3 {
                    let level = water.round().clamp(lo, hi) as i16;
                    tile.fluid.set(i, level);
                    // Where there was no water: the river's own, which it
                    // may lower to hold it in its bed.
                    poured.lock().expect("not poisoned").insert((x, z));
                    wet += 1;
                    if let Some(bed) = spec.bed {
                        // Patches of clay where the water runs slowly.
                        let slow = steep[k] < 0.3 && spec.clay.is_some() && patches.fbm(x as f64 / 9.0, z as f64 / 9.0, 2, 2.0, 0.5) > 0.3;
                        let terrain = if shallow {
                            spec.shore.unwrap_or(bed)
                        } else if slow {
                            spec.clay.unwrap_or(bed)
                        } else {
                            bed
                        };
                        tile.terrain.set(i, terrain.0);
                    }
                    if let Some(biome) = spec.biome {
                        tile.biome.set(i, biome.0);
                    }
                } else if d > hw && let (Some(_), Some(shore)) = (spec.bed, spec.shore) {
                    // A beach at the foot of the bank inside a bend, just
                    // over the water.
                    let beach = if inner { 1.5 } else { 0.0 };
                    if d <= hw + beach && new <= water + 1.5 && tile.terrain.get(i) != shore.0 {
                        tile.terrain.set(i, shore.0);
                        changed += 1;
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
    let dug = dug.into_inner().expect("not poisoned");
    let poured = poured.into_inner().expect("not poisoned");
    // Never under the sea it runs into. Water a river runs into on land
    // (a lake, a river) may stand higher than the land around it: there the
    // river's water goes down rather than run over the land.
    let floor = spec.sea.filter(|_| mouth.is_some()).map_or(i16::MIN, |w| w.round() as i16);
    crate::water::seal(edit, rect, &|x, z| dug.get(&(x, z)).copied().unwrap_or(f32::MIN), &|x, z| poured.contains(&(x, z)), floor);
    RiverReport {
        length,
        columns,
        wet,
        source: level[0],
        mouth: *level.last().expect("a course"),
    }
}

// ------------------------------------------------------------ springs --

/// Why a river let go at a spring ended where it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouth {
    /// In the sea.
    Sea,
    /// In water already there: a river, a lake.
    Water,
    /// Off the edge of the map.
    Edge,
    /// In a hollow too wide to fill: a lake it ends in.
    Lake,
    /// It ran as far as it was let.
    Far,
    /// Where it was asked to end.
    Chosen,
}

impl Mouth {
    pub const fn label(self) -> &'static str {
        match self {
            Mouth::Sea => "the sea",
            Mouth::Water => "the water it meets",
            Mouth::Edge => "the edge of the map",
            Mouth::Lake => "a lake",
            Mouth::Far => "as far as it may run",
            Mouth::Chosen => "the end you chose",
        }
    }
}

/// Where water let go at a spring goes.
#[derive(Debug, Clone, Default)]
pub struct Descent {
    /// Its course from the spring down, a point every block or so.
    pub course: Vec<(f32, f32)>,
    /// The hollows it fills on its way: the columns under their water, and
    /// the level they overflow at.
    pub lakes: Vec<(Vec<(i32, i32)>, f32)>,
    pub mouth: Option<Mouth>,
    /// The level of the sea, when it ends in the sea.
    pub sea: Option<f32>,
}

/// The ground as water let go at a spring sees it: the hollows it filled
/// are flat at the level of their water.
struct Surface<'a> {
    ground: Ground<'a>,
    raised: HashMap<(i32, i32), f32>,
}

impl Surface<'_> {
    fn at(&mut self, x: i32, z: i32) -> Option<(f32, Option<f32>)> {
        let (h, w) = self.ground.at(x, z)?;
        Some((self.raised.get(&(x, z)).map_or(h, |r| r.max(h)), w))
    }
}

/// What a hollow filled up to its brim gives.
enum Spill {
    /// The water overflows at the end of `path`: on from there (or into
    /// the water or off the map: `end`).
    Out { path: Vec<(i32, i32)>, lake: Vec<(i32, i32)>, level: f32, end: Option<Mouth>, inside: Vec<(i32, i32)> },
    /// Too wide to fill: the water stays in it.
    Full { lake: Vec<(i32, i32)>, level: f32 },
}

const AROUND: [(i32, i32); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];

/// Fill the hollow at `from` up to where it overflows, lowest columns
/// first, as water would: the way out, the columns under water, its level.
fn fill_hollow(ground: &mut Surface, jitter: &dyn Fn(i32, i32) -> f32, from: (i32, i32), upstream: &std::collections::HashSet<(i32, i32)>, sea: f32, cap: usize) -> Spill {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashSet};
    #[derive(PartialEq)]
    struct Key(f32);
    impl Eq for Key {}
    impl Ord for Key {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    impl PartialOrd for Key {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    /// A column waiting: its height, its noise, when it came, where it is.
    type Waiting = Reverse<(Key, Key, u32, (i32, i32))>;
    // Lowest first, by the true height; the noise only settles ties.
    let mut heap: BinaryHeap<Waiting> = BinaryHeap::new();
    let mut parent: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut inside: HashSet<(i32, i32)> = HashSet::from([from]);
    let mut level = ground.at(from.0, from.1).map_or(0.0, |g| g.0);
    let mut seq = 0u32;
    let mut push = |heap: &mut BinaryHeap<_>, parent: &mut HashMap<_, _>, inside: &HashSet<(i32, i32)>, ground: &mut Surface, c: (i32, i32)| {
        for (dx, dz) in AROUND {
            let nb = (c.0 + dx, c.1 + dz);
            if inside.contains(&nb) || parent.contains_key(&nb) {
                continue;
            }
            parent.insert(nb, c);
            let h = ground.at(nb.0, nb.1).map_or(f32::MIN, |g| g.0);
            heap.push(Reverse((Key(h), Key(jitter(nb.0, nb.1)), seq, nb)));
            seq += 1;
        }
    };
    push(&mut heap, &mut parent, &inside, ground, from);
    let walk = |parent: &HashMap<(i32, i32), (i32, i32)>, mut c: (i32, i32)| {
        let mut path = vec![c];
        while let Some(&p) = parent.get(&c) {
            if p == from {
                break;
            }
            path.push(p);
            c = p;
        }
        path.reverse();
        path
    };
    // Under its water: the columns whose ground (not the water of a hollow
    // filled before) lies well under the level.
    let lake_of = |ground: &mut Surface, inside: &HashSet<(i32, i32)>, level: f32| -> Vec<(i32, i32)> {
        inside.iter().copied().filter(|c| ground.ground.at(c.0, c.1).is_some_and(|g| g.0 < level - 0.3)).collect()
    };
    while let Some(Reverse((_, _, _, c))) = heap.pop() {
        if inside.contains(&c) {
            continue;
        }
        let end = match ground.at(c.0, c.1) {
            None => Some(Some(Mouth::Edge)),
            Some((h, _)) if h < sea => Some(Some(Mouth::Sea)),
            Some((_, Some(_))) => Some(Some(Mouth::Water)),
            // Lower than the water: it runs out here.
            Some((h, None)) if h < level - 0.05 && !upstream.contains(&c) => Some(None),
            _ => None,
        };
        if let Some(end) = end {
            return Spill::Out {
                path: walk(&parent, c),
                lake: lake_of(ground, &inside, level),
                level,
                end,
                inside: inside.into_iter().collect(),
            };
        }
        inside.insert(c);
        level = level.max(ground.at(c.0, c.1).map_or(level, |g| g.0));
        if inside.len() > cap {
            return Spill::Full {
                lake: lake_of(ground, &inside, level),
                level,
            };
        }
        push(&mut heap, &mut parent, &inside, ground, c);
    }
    Spill::Full {
        lake: lake_of(ground, &inside, level),
        level,
    }
}

/// Follow water let go at `(x, z)` down the ground of `level`: down the
/// slope, a step of a block at a time; a hollow fills up to where it
/// overflows — a lake — and the water goes on from there. It ends in the
/// sea (under `sea`), in water already there, off the map, in a hollow of
/// more than `lake_cap` columns, or after `max_len` blocks. Only hollows
/// deeper than `lake_depth` blocks hold a lake; through the others the
/// river passes, and digging it cuts through their rim.
pub fn descend(level: &crate::level::Level, start: (i32, i32), sea: f32, max_len: usize, lake_cap: usize, lake_depth: f32) -> Descent {
    let noise = crate::noise::Noise::new(0x5f21_9a3b);
    // A breath of noise, so that water wanders across flats as it would.
    let jitter = |x: i32, z: i32| 0.25 * noise.fbm(x as f64 / 28.0, z as f64 / 28.0, 3, 2.0, 0.5) as f32;
    let mut ground = Surface {
        ground: Ground { level, tiles: HashMap::new() },
        raised: HashMap::new(),
    };
    let mut d = Descent::default();
    let Some((h0, w0)) = ground.at(start.0, start.1) else {
        d.mouth = Some(Mouth::Edge);
        return d;
    };
    if w0.is_some() || h0 < sea {
        d.mouth = Some(if h0 < sea { Mouth::Sea } else { Mouth::Water });
        d.sea = (h0 < sea).then_some(sea);
        return d;
    }
    // The height between columns, noise and all.
    let height = |ground: &mut Surface, x: f32, z: f32| -> Option<f32> {
        let (fx, fz) = (x - 0.5, z - 0.5);
        let (i, j) = (fx.floor() as i32, fz.floor() as i32);
        let (u, v) = (fx - i as f32, fz - j as f32);
        let mut at = |a: i32, b: i32| ground.at(a, b).map(|g| g.0 + jitter(a, b));
        let (h00, h10, h01, h11) = (at(i, j)?, at(i + 1, j)?, at(i, j + 1)?, at(i + 1, j + 1)?);
        Some((h00 + (h10 - h00) * u) + ((h01 + (h11 - h01) * u) - (h00 + (h10 - h00) * u)) * v)
    };
    let mut upstream = std::collections::HashSet::new();
    let (mut x, mut z) = (start.0 as f32 + 0.5, start.1 as f32 + 0.5);
    let mut heading = (0.0f32, 0.0f32);
    d.course.push((x, z));
    upstream.insert(start);
    let mut length = 0.0f32;
    loop {
        if length >= max_len as f32 {
            d.mouth = Some(Mouth::Far);
            break;
        }
        // Down the slope, keeping a little of the way it was going.
        let next = height(&mut ground, x, z).and_then(|here| {
            let gx = (height(&mut ground, x + 1.0, z)? - height(&mut ground, x - 1.0, z)?) / 2.0;
            let gz = (height(&mut ground, x, z + 1.0)? - height(&mut ground, x, z - 1.0)?) / 2.0;
            let g = (gx * gx + gz * gz).sqrt();
            if g < 1e-4 {
                return None;
            }
            let down = (-gx / g, -gz / g);
            for weight in [0.6f32, 0.0] {
                let (dx, dz) = (down.0 + heading.0 * weight, down.1 + heading.1 * weight);
                let len = (dx * dx + dz * dz).sqrt().max(1e-6);
                let (nx, nz) = (x + dx / len, z + dz / len);
                if height(&mut ground, nx, nz).is_some_and(|h| h < here - 1e-3) {
                    return Some((nx, nz, (dx / len, dz / len)));
                }
            }
            None
        });
        // Back on its own way: going round, as on a flat; fill it.
        let next = next.filter(|&(nx, nz, _)| {
            let c = (nx.floor() as i32, nz.floor() as i32);
            c == (x.floor() as i32, z.floor() as i32) || !upstream.contains(&c)
        });
        match next {
            Some((nx, nz, h)) => {
                (x, z, heading) = (nx, nz, h);
                length += 1.0;
                let cell = (x.floor() as i32, z.floor() as i32);
                d.course.push((x, z));
                upstream.insert(cell);
                match ground.at(cell.0, cell.1) {
                    None => {
                        d.mouth = Some(Mouth::Edge);
                        break;
                    }
                    Some((h, _)) if h < sea => {
                        d.mouth = Some(Mouth::Sea);
                        break;
                    }
                    Some((_, Some(_))) => {
                        d.mouth = Some(Mouth::Water);
                        break;
                    }
                    _ => {}
                }
            }
            None => {
                // A hollow, or a flat: fill it until it overflows.
                let cell = (x.floor() as i32, z.floor() as i32);
                match fill_hollow(&mut ground, &jitter, cell, &upstream, sea, lake_cap) {
                    Spill::Out { path, lake, level, end, inside } => {
                        let deepest = lake.iter().filter_map(|c| ground.ground.at(c.0, c.1)).map(|g| level - g.0).fold(0.0f32, f32::max);
                        if !lake.is_empty() && deepest >= lake_depth {
                            d.lakes.push((lake, level));
                        }
                        // Filled: from now on, water at its level.
                        for c in inside {
                            ground.raised.insert(c, level);
                        }
                        for c in &path {
                            d.course.push((c.0 as f32 + 0.5, c.1 as f32 + 0.5));
                            upstream.insert(*c);
                        }
                        length += path.len() as f32;
                        if let Some(&last) = path.last() {
                            (x, z) = (last.0 as f32 + 0.5, last.1 as f32 + 0.5);
                        }
                        heading = (0.0, 0.0);
                        if end.is_some() {
                            d.mouth = end;
                            break;
                        }
                    }
                    Spill::Full { lake, level } => {
                        if !lake.is_empty() {
                            d.lakes.push((lake, level));
                        }
                        d.mouth = Some(Mouth::Lake);
                        break;
                    }
                }
            }
        }
    }
    // A hollow that overflowed into another one higher than it became one
    // lake with it, filled again: only the last, whole, lake counts. The
    // course goes straight across a lake, from where it enters to where it
    // leaves.
    type Kept = (std::collections::HashSet<(i32, i32)>, Vec<(i32, i32)>, f32);
    let mut kept: Vec<Kept> = Vec::new();
    for (cells, level) in std::mem::take(&mut d.lakes).into_iter().rev() {
        if cells.iter().step_by((cells.len() / 8).max(1)).any(|c| kept.iter().any(|k| k.0.contains(c))) {
            continue;
        }
        kept.push((cells.iter().copied().collect(), cells, level));
    }
    kept.reverse();
    if !kept.is_empty() {
        let last = d.course.len().saturating_sub(1);
        let under = |p: &(f32, f32)| {
            let c = (p.0.floor() as i32, p.1.floor() as i32);
            kept.iter().any(|k| k.0.contains(&c))
        };
        d.course = d.course.iter().enumerate().filter(|(k, p)| *k == 0 || *k == last || !under(p)).map(|(_, p)| *p).collect();
    }
    d.lakes = kept.into_iter().map(|(_, cells, level)| (cells, level)).collect();
    d.sea = (d.mouth == Some(Mouth::Sea)).then_some(sea);
    d
}

/// Dig the river of a [`Descent`] into the level `edit` works on: its
/// lakes filled to their level, then its bed along its course
/// ([`carve_river`]). Recorded for undo.
pub fn dig_descent(edit: &mut Edit, descent: &Descent, spec: &RiverSpec) -> RiverReport {
    if spec.water {
        let (lo, hi) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
        for (cells, level) in &descent.lakes {
            let set: std::collections::HashSet<(i32, i32)> = cells.iter().copied().collect();
            let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for &(x, z) in cells {
                (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
            }
            let rect = BlockRect::new(x0, z0, x1 + 1, z1 + 1);
            // Rounded down: rounded up, the water would stand a block over
            // the lowest of its rim, and run out over it.
            let water = level.floor().clamp(lo, hi) as i16;
            edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
                let mut any = false;
                for z in clip.z0..clip.z1 {
                    for x in clip.x0..clip.x1 {
                        let i = local_index(x, z);
                        if set.contains(&(x, z)) && (water as f32) > tile.top.get(i) + 0.3 {
                            tile.fluid.set(i, water);
                            any = true;
                        }
                    }
                }
                if any {
                    tile.compact();
                }
            });
            // A lake held by its rim stays as it is; one that would run out
            // over lower ground beside it (a hollow not closed all round)
            // goes down to what holds it.
            crate::water::seal(edit, rect, &|_, _| f32::MIN, &|x, z| set.contains(&(x, z)), i16::MIN);
        }
    }
    // Every point of the course: the curve passes through those it was
    // dragged through, and runs where it was shown.
    let points = descent.course.clone();
    // Into the sea: its channel goes on under it.
    let spec = RiverSpec {
        sea: spec.sea.or(descent.sea),
        ..*spec
    };
    carve_river(edit, &points, &spec)
}

// ------------------------------------------------------- natural course --

/// A river's course made natural before it is dug (`course`: a point every
/// block or so, from the source; `width`: the river's width there, in
/// blocks; `anchors`: the indices of the points it must pass, kept where
/// they are). Returns the new course and the new indices of its anchors.
///
/// - Its loops are cut: where it comes back within a width of where it
///   was before (water filling a shallow hollow wanders round it), it goes
///   straight across instead, unless that would climb.
/// - Its runs along the grid (the ways found across hollows and over the
///   land go from cell to cell) are smoothed over about its width.
/// - On gentle ground it meanders: a wave across its way some ten widths
///   long, half a width to either side, smaller where it would climb out
///   of its valley, none near its ends and its anchors.
pub fn natural_course(level: &crate::level::Level, course: &[(f32, f32)], width: f32, anchors: &[usize]) -> (Vec<(f32, f32)>, Vec<usize>) {
    let (course, anchors) = erase_loops(course, anchors);
    let (course, anchors) = trim_spurs(&course, &anchors, (0.5 * width).max(8.0));
    natural(level, &course, width, &anchors)
}

/// A course without its small loops: where water filling a hollow, or a
/// way over a grid, comes back within a block and a half of where it was
/// before (wandering round a cell, stepping back onto it), what lies
/// between is cut out. Never a loop through an anchor. Returns the course
/// and its anchors' new places.
fn erase_loops(course: &[(f32, f32)], anchors: &[usize]) -> (Vec<(f32, f32)>, Vec<usize>) {
    const NEAR: f32 = 1.5;
    let key = |p: (f32, f32)| ((p.0 / NEAR).floor() as i32, (p.1 / NEAR).floor() as i32);
    // The indices into `course` of the points kept, and by cell their
    // places among those (stale once that place holds another).
    let mut kept: Vec<usize> = Vec::with_capacity(course.len());
    let mut seen: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    // The place of the last anchor kept: no loop is cut back past it.
    let mut floor = 0;
    for (k, &p) in course.iter().enumerate() {
        let (cx, cz) = key(p);
        let mut back: Option<usize> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &(j, orig) in seen.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if kept.get(j) != Some(&orig) || j < floor || j + 2 > kept.len() {
                        continue;
                    }
                    let q = course[orig];
                    if (q.0 - p.0).hypot(q.1 - p.1) <= NEAR && back.is_none_or(|b| j < b) {
                        back = Some(j);
                    }
                }
            }
        }
        let anchor = anchors.contains(&k);
        if let Some(j) = back {
            kept.truncate(j + 1);
            // Back on a point kept: it adds nothing, unless it is an anchor
            // (then it takes that point's place).
            let q = course[kept[j]];
            if (q.0 - p.0).hypot(q.1 - p.1) < 0.5 && !anchor && k + 1 < course.len() {
                continue;
            }
            if anchor && (q.0 - p.0).hypot(q.1 - p.1) < 0.5 && j > floor {
                kept.pop();
            }
        }
        seen.entry((cx, cz)).or_default().push((kept.len(), k));
        kept.push(k);
        if anchor {
            floor = kept.len() - 1;
        }
    }
    let out: Vec<(f32, f32)> = kept.iter().map(|&k| course[k]).collect();
    let at = anchors.iter().map(|a| kept.iter().position(|k| k == a).unwrap_or(kept.len() - 1)).collect();
    (out, at)
}

/// A course without the spurs at its anchors: the ways found to a point
/// and on from it end and begin on cells of a grid, which can lie past
/// it, so that the course overshoots the point and comes back to it, or
/// leaves it back the way it came. Points within `reach` of an anchor
/// that come at it from the side it leaves by (or leave it towards the
/// side it came from) are dropped. Returns the course and its anchors'
/// new places.
fn trim_spurs(course: &[(f32, f32)], anchors: &[usize], reach: f32) -> (Vec<(f32, f32)>, Vec<usize>) {
    let n = course.len();
    let mut keep = vec![true; n];
    let is_anchor = |k: usize| anchors.contains(&k);
    let dist = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
    for &a in anchors {
        if a == 0 || a + 1 >= n {
            continue;
        }
        let p = course[a];
        // Its way on and its way in, from beyond `reach`.
        let ahead = (a + 1..n).find(|&k| dist(course[k], p) >= reach || is_anchor(k)).unwrap_or(n - 1);
        let out = (course[ahead].0 - p.0, course[ahead].1 - p.1);
        let mut j = a;
        while j > 0 {
            j -= 1;
            let q = course[j];
            if is_anchor(j) || dist(q, p) > reach || (p.0 - q.0) * out.0 + (p.1 - q.1) * out.1 >= 0.0 {
                break;
            }
            keep[j] = false;
        }
        let behind = (0..a).rev().find(|&k| keep[k] && (dist(course[k], p) >= reach || is_anchor(k))).unwrap_or(0);
        let inn = (p.0 - course[behind].0, p.1 - course[behind].1);
        for k in a + 1..ahead {
            let q = course[k];
            if (q.0 - p.0) * inn.0 + (q.1 - p.1) * inn.1 >= 0.0 {
                break;
            }
            keep[k] = false;
        }
    }
    let mut out = Vec::with_capacity(n);
    let mut place = vec![0; n];
    for k in 0..n {
        if keep[k] {
            out.push(course[k]);
        }
        place[k] = out.len().saturating_sub(1);
    }
    (out, anchors.iter().map(|&a| place[a]).collect())
}

/// [`natural_course`], its spurs trimmed.
fn natural(level: &crate::level::Level, course: &[(f32, f32)], width: f32, anchors: &[usize]) -> (Vec<(f32, f32)>, Vec<usize>) {
    let n = course.len();
    if n < 8 {
        return (course.to_vec(), anchors.to_vec());
    }
    let mut ground = Ground::new(level);
    let mut height = |p: (f32, f32)| ground.at(p.0.floor() as i32, p.1.floor() as i32).map_or(f32::MAX, |(h, w)| w.unwrap_or(h));
    let width = width.max(2.0);

    // ---- loops: where it comes back near where it was, straight across.
    let near = (0.9 * width).max(4.0);
    let cell = near.max(4.0);
    let key = |p: (f32, f32)| ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32);
    let mut kept: Vec<usize> = Vec::with_capacity(n);
    // The points kept, by cell: their place in `kept` and their index in
    // the course (stale once that place holds another point).
    let mut seen: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    let is_anchor = |k: usize| anchors.contains(&k);
    // A loop is worth cutting when the way round is much longer than
    // across: at least four widths.
    let shortest = (4.0 * width).max(16.0);
    for (k, &p) in course.iter().enumerate() {
        // The earliest point kept so far that it comes back near.
        let (cx, cz) = key(p);
        let mut back: Option<usize> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                for &(j, orig) in seen.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if kept.get(j) != Some(&orig) {
                        continue;
                    }
                    let q = course[orig];
                    if (q.0 - p.0).hypot(q.1 - p.1) <= near && back.is_none_or(|b| j < b) {
                        back = Some(j);
                    }
                }
            }
        }
        if let Some(j) = back {
            let from = kept[j];
            let round: f32 = course[from..=k].windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
            let anchored = kept[j + 1..].iter().any(|&a| is_anchor(a));
            // Across must not climb over what lies between.
            let ends = height(course[from]).max(height(p));
            let climbs = (1..8).any(|s| {
                let t = s as f32 / 8.0;
                height((course[from].0 + (p.0 - course[from].0) * t, course[from].1 + (p.1 - course[from].1) * t)) > ends + 1.5
            });
            if round >= shortest && !anchored && !climbs {
                kept.truncate(j + 1);
            }
        }
        seen.entry(key(p)).or_default().push((kept.len(), k));
        kept.push(k);
    }
    let mut pts: Vec<(f32, f32)> = kept.iter().map(|&k| course[k]).collect();
    let mut anchor_at: Vec<usize> = anchors.iter().filter_map(|a| kept.iter().position(|k| k == a)).collect();

    // ---- runs along the grid: a moving average over about its width,
    // its ends and anchors held; no point moved up a slope.
    let fixed = |k: usize, anchor_at: &[usize], len: usize| k == 0 || k + 1 == len || anchor_at.contains(&k);
    let radius = ((0.6 * width).clamp(3.0, 12.0)) as usize;
    for _ in 0..2 {
        let len = pts.len();
        let before = pts.clone();
        for k in 0..len {
            if fixed(k, &anchor_at, len) {
                continue;
            }
            let (a, b) = (k.saturating_sub(radius), (k + radius).min(len - 1));
            // Not across an anchor.
            let a = anchor_at.iter().filter(|&&s| s < k).map(|&s| s.max(a)).max().unwrap_or(a);
            let b = anchor_at.iter().filter(|&&s| s > k).map(|&s| s.min(b)).min().unwrap_or(b);
            let m = (b - a + 1) as f32;
            let avg = before[a..=b].iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
            let q = (avg.0 / m, avg.1 / m);
            if height(q) <= height(before[k]) + 1.0 {
                pts[k] = q;
            }
        }
    }

    // ---- meanders on gentle ground.
    let len = pts.len();
    let along: Vec<f32> = std::iter::once(0.0)
        .chain(pts.windows(2).scan(0.0f32, |s, w| {
            *s += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            Some(*s)
        }))
        .collect();
    let wave = (10.0 * width).clamp(40.0, 400.0);
    let amplitude = 0.5 * width;
    let noise = crate::noise::Noise::new(0x3ea7_d3e2);
    let total = *along.last().unwrap_or(&0.0);
    let mut out = pts.clone();
    for k in 1..len - 1 {
        // Gentle: under one block of drop in twenty, over a wave either way.
        let span = (wave / 2.0) as usize;
        let (a, b) = (k.saturating_sub(span), (k + span).min(len - 1));
        let run = (along[b] - along[a]).max(1.0);
        let grade = ((height(pts[a]) - height(pts[b])) / run).max(0.0);
        let gentle = (1.0 - grade / 0.05).clamp(0.0, 1.0);
        if gentle <= 0.0 {
            continue;
        }
        // None near its ends and its anchors: half a wave to grow.
        let from_ends = along[k].min(total - along[k]);
        let from_anchor = anchor_at.iter().map(|&s| (along[s] - along[k]).abs()).fold(f32::MAX, f32::min);
        let room = (from_ends.min(from_anchor) / (0.5 * wave)).clamp(0.0, 1.0);
        let room = room * room * (3.0 - 2.0 * room);
        // A wave whose length wanders a little, as meanders' do.
        let phase = std::f32::consts::TAU * along[k] / wave + 1.5 * noise.fbm(along[k] as f64 / wave as f64, 0.37, 2, 2.0, 0.5) as f32;
        let mut offset = amplitude * gentle * room * phase.sin();
        if offset.abs() < 0.25 {
            continue;
        }
        let (d0, d1) = (pts[k + 1].0 - pts[k - 1].0, pts[k + 1].1 - pts[k - 1].1);
        let dl = d0.hypot(d1).max(1e-6);
        let normal = (-d1 / dl, d0 / dl);
        // Smaller where it would climb out of its valley.
        let here = height(pts[k]);
        for _ in 0..4 {
            let q = (pts[k].0 + normal.0 * offset, pts[k].1 + normal.1 * offset);
            if height(q) <= here + 1.0 {
                out[k] = q;
                break;
            }
            offset *= 0.5;
        }
    }
    // The reductions leave kinks: smoothed once more over a few blocks.
    let mut done = out.clone();
    for (k, d) in done.iter_mut().enumerate().take(len - 1).skip(1) {
        if fixed(k, &anchor_at, len) {
            continue;
        }
        let (a, b) = (k.saturating_sub(3), (k + 3).min(len - 1));
        let m = (b - a + 1) as f32;
        let avg = out[a..=b].iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
        *d = (avg.0 / m, avg.1 / m);
    }
    anchor_at.sort_unstable();
    keep_to(course, anchors, done, anchor_at, 0.5 * width)
}

/// A course made natural (`done`, its anchors at `done_at`) held to the
/// way it was made from (`course`, its anchors at `anchors`): no point
/// farther than `reach` from it, pulled back where it strays; and each
/// stretch between two anchors that crosses itself or another, or turns
/// back on itself, the way it was made from instead.
fn keep_to(course: &[(f32, f32)], anchors: &[usize], mut done: Vec<(f32, f32)>, done_at: Vec<usize>, reach: f32) -> (Vec<(f32, f32)>, Vec<usize>) {
    // The nearest point of the course, by cells of `reach`.
    let cell = reach.max(2.0);
    let key = |p: (f32, f32)| ((p.0 / cell).floor() as i32, (p.1 / cell).floor() as i32);
    let mut segs: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, w) in course.windows(2).enumerate() {
        let (a, b) = (key(w[0]), key(w[1]));
        for j in a.1.min(b.1) - 1..=a.1.max(b.1) + 1 {
            for i in a.0.min(b.0) - 1..=a.0.max(b.0) + 1 {
                segs.entry((i, j)).or_default().push(k);
            }
        }
    }
    let nearest = |p: (f32, f32)| -> Option<(f32, f32)> {
        let mut best: Option<((f32, f32), f32)> = None;
        for &k in segs.get(&key(p)).into_iter().flatten() {
            let (a, b) = (course[k], course[k + 1]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let q = (a.0 + t * dx, a.1 + t * dz);
            let d = (p.0 - q.0).hypot(p.1 - q.1);
            if best.is_none_or(|b| d < b.1) {
                best = Some((q, d));
            }
        }
        best.map(|b| b.0)
    };
    for p in done.iter_mut() {
        match nearest(*p) {
            Some(q) => {
                let d = (p.0 - q.0).hypot(p.1 - q.1);
                if d > reach {
                    *p = (q.0 + (p.0 - q.0) * reach / d, q.1 + (p.1 - q.1) * reach / d);
                }
            }
            // Farther than a cell from any of it: back onto the course.
            None => {
                let q = course.iter().copied().min_by(|a, b| (a.0 - p.0).hypot(a.1 - p.1).total_cmp(&(b.0 - p.0).hypot(b.1 - p.1))).unwrap_or(*p);
                *p = q;
            }
        }
    }
    // Stretch by stretch between anchors: the natural one where it is sound,
    // else the way it was made from.
    let mut cuts_d = vec![0];
    cuts_d.extend(done_at.iter().copied().filter(|&k| k > 0 && k + 1 < done.len()));
    cuts_d.push(done.len() - 1);
    let mut cuts_c = vec![0];
    cuts_c.extend(anchors.iter().copied().filter(|&k| k > 0 && k + 1 < course.len()));
    cuts_c.push(course.len() - 1);
    if cuts_d.len() != cuts_c.len() {
        return (done, done_at);
    }
    let mut out: Vec<(f32, f32)> = vec![done[0]];
    let mut out_at = Vec::with_capacity(done_at.len());
    if done_at.contains(&0) {
        out_at.push(0);
    }
    let mut pieces: Vec<Vec<(f32, f32)>> = Vec::new();
    for s in 0..cuts_d.len() - 1 {
        let natural = &done[cuts_d[s]..=cuts_d[s + 1]];
        let sound = !turns_back(natural) && crossings(natural, &pieces) == 0;
        let piece = if sound { natural.to_vec() } else { course[cuts_c[s]..=cuts_c[s + 1]].to_vec() };
        out.extend(piece.iter().skip(1));
        if s + 2 < cuts_d.len() || done_at.contains(&(done.len() - 1)) {
            out_at.push(out.len() - 1);
        }
        pieces.push(piece);
    }
    (out, out_at)
}

/// Whether a course turns back on itself: two blocks on, its way reversed
/// by more than 120°.
fn turns_back(line: &[(f32, f32)]) -> bool {
    // Points a block and a half apart at least.
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last().is_none_or(|q: &(f32, f32)| (q.0 - p.0).hypot(q.1 - p.1) >= 1.5) {
            pts.push(p);
        }
    }
    pts.windows(3).any(|w| {
        let (a, b) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), (w[2].0 - w[1].0, w[2].1 - w[1].1));
        let cos = (a.0 * b.0 + a.1 * b.1) / (a.0.hypot(a.1) * b.0.hypot(b.1)).max(1e-6);
        cos < -0.5
    })
}

/// How many times `line` crosses itself or any of `others`.
fn crossings(line: &[(f32, f32)], others: &[Vec<(f32, f32)>]) -> usize {
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        let o = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
        let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    };
    // By cells of 8 blocks: only segments that share one are compared.
    let key = |p: (f32, f32)| ((p.0 / 8.0).floor() as i32, (p.1 / 8.0).floor() as i32);
    let mut cells: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
    let all: Vec<&[(f32, f32)]> = std::iter::once(line).chain(others.iter().map(|v| v.as_slice())).collect();
    for (l, pts) in all.iter().enumerate() {
        for k in 0..pts.len().saturating_sub(1) {
            let (a, b) = (key(pts[k]), key(pts[k + 1]));
            for j in a.1.min(b.1)..=a.1.max(b.1) {
                for i in a.0.min(b.0)..=a.0.max(b.0) {
                    cells.entry((i, j)).or_default().push((l, k));
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for list in cells.values() {
        for &(_, k1) in list.iter().filter(|s| s.0 == 0) {
            for &(l2, k2) in list {
                // Neighbours share an end; so do the ends of joined stretches.
                if (l2 == 0 && k2 <= k1 + 1) || (l2 > 0 && l2 == all.len() - 1 && k1 == 0 && k2 + 2 == all[l2].len()) {
                    continue;
                }
                let (a, b) = (all[0][k1], all[0][k1 + 1]);
                let (c, d) = (all[l2][k2], all[l2][k2 + 1]);
                if cross(a, b, c, d) {
                    seen.insert((k1, l2, k2));
                }
            }
        }
    }
    seen.len()
}

// ------------------------------------------------------------- routes --

/// What climbing a block costs a river laid out by hand, in blocks of
/// level ground it would rather run.
const CLIMB: f32 = 14.0;

/// The way water takes from `from` to `to` over the ground of `level`:
/// along the valleys, climbing as little as it can, into the water it
/// meets — the cheapest path over a grid of the ground of `cells` samples
/// at most (coarser for longer ways). Points in blocks, `from` first and
/// `to` last; a straight line where no way is found.
///
/// It keeps near the line from `from` to `to`: wandering off it by more
/// than a sixth of its length (32 blocks at least) costs more the farther,
/// so that it takes the valleys along the way it was given rather than one
/// far off.
pub fn route(level: &crate::level::Level, from: (f32, f32), to: (f32, f32), cells: usize) -> Vec<(f32, f32)> {
    route_near(level, from, to, cells, &[])
}

/// Blocks either side of a way already shown that [`route_near`] keeps to.
pub const GUIDE_REACH: f32 = 12.0;

/// [`route`], within [`GUIDE_REACH`] of `guide` (a way found over a coarser
/// grid, shown while its ends were dragged) when it is not empty: the way
/// worked out finely once they are let go is the one that was shown, not
/// another down the next valley. Where no way is found within it, the way
/// [`route`] finds.
pub fn route_near(level: &crate::level::Level, from: (f32, f32), to: (f32, f32), cells: usize, guide: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let dist = (to.0 - from.0).hypot(to.1 - from.1);
    if dist < 3.0 {
        return vec![from, to];
    }
    let margin = (dist * 0.35).max(48.0);
    let (x0, z0) = (from.0.min(to.0) - margin, from.1.min(to.1) - margin);
    let (x1, z1) = (from.0.max(to.0) + margin, from.1.max(to.1) + margin);
    let step = (((x1 - x0) * (z1 - z0)) / cells.max(1_000) as f32).sqrt().ceil().max(1.0);
    let nx = ((x1 - x0) / step).ceil() as usize + 1;
    let nz = ((z1 - z0) / step).ceil() as usize + 1;
    let at = |k: usize| (x0 + (k % nx) as f32 * step, z0 + (k / nx) as f32 * step);
    // The height of each sample — the water's surface where there is
    // water — NaN off the map.
    let mut ground = Ground { level, tiles: HashMap::new() };
    let mut h = vec![f32::NAN; nx * nz];
    let mut wet = vec![false; nx * nz];
    for (k, (hk, wk)) in h.iter_mut().zip(wet.iter_mut()).enumerate() {
        let (x, z) = at(k);
        if let Some((g, w)) = ground.at(x.floor() as i32, z.floor() as i32) {
            *hk = w.unwrap_or(g);
            *wk = w.is_some();
        }
    }
    let sample = |p: (f32, f32)| {
        let i = (((p.0 - x0) / step).round().max(0.0) as usize).min(nx - 1);
        let j = (((p.1 - z0) / step).round().max(0.0) as usize).min(nz - 1);
        j * nx + i
    };
    let (start, goal) = (sample(from), sample(to));
    if !h[start].is_finite() || !h[goal].is_finite() {
        return vec![from, to];
    }
    let (lo, hi) = h.iter().filter(|v| v.is_finite()).fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1.0);
    let noise = crate::noise::Noise::new(0x7a11_0e5d);
    // What crossing a sample costs, a block at a time: more the higher it
    // lies, with a little noise so that the way wanders as rivers do; a
    // little on water.
    let cost: Vec<f32> = (0..nx * nz)
        .map(|k| {
            if !h[k].is_finite() {
                return f32::INFINITY;
            }
            if wet[k] {
                return 0.35;
            }
            let (x, z) = at(k);
            1.0 + 1.6 * (h[k] - lo) / span + 0.3 * noise.fbm(x as f64 / 40.0, z as f64 / 40.0, 3, 2.0, 0.5) as f32
        })
        .collect();
    // Off the line between its ends: a cost growing as the square of how
    // far past a sixth of its length.
    let free = (dist / 6.0).max(32.0);
    let (lx, lz) = ((to.0 - from.0) / dist, (to.1 - from.1) / dist);
    let mut cost = cost;
    for (k, c) in cost.iter_mut().enumerate() {
        if c.is_finite() {
            let (x, z) = at(k);
            let t = ((x - from.0) * lx + (z - from.1) * lz).clamp(0.0, dist);
            let off = (x - from.0 - t * lx).hypot(z - from.1 - t * lz);
            if off > free {
                let e = (off - free) / free;
                *c += 1.5 * e * e;
            }
        }
    }
    // Along the way shown: nowhere else.
    if guide.len() >= 2 {
        let mut inside = vec![false; nx * nz];
        let r = (GUIDE_REACH / step).ceil() as i32;
        let mut stamp = |p: (f32, f32)| {
            let (ci, cj) = (((p.0 - x0) / step).round() as i32, ((p.1 - z0) / step).round() as i32);
            for dj in -r..=r {
                for di in -r..=r {
                    let (i, j) = (ci + di, cj + dj);
                    if i >= 0 && j >= 0 && i < nx as i32 && j < nz as i32 && ((di * di + dj * dj) as f32) * step * step <= GUIDE_REACH * GUIDE_REACH {
                        inside[j as usize * nx + i as usize] = true;
                    }
                }
            }
        };
        for w in guide.windows(2) {
            let n = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / (0.5 * step).max(1.0)).ceil().max(1.0) as usize;
            for s in 0..=n {
                let t = s as f32 / n as f32;
                stamp((w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t));
            }
        }
        stamp(from);
        stamp(to);
        if inside[start] && inside[goal] {
            let mut kept = cost.clone();
            for (c, &i) in kept.iter_mut().zip(&inside) {
                if !i {
                    *c = f32::INFINITY;
                }
            }
            if let Some(way) = cheapest(&kept, &h, (nx, nz), step, (start, goal), at) {
                return finish(way, from, to, at);
            }
        }
    }
    match cheapest(&cost, &h, (nx, nz), step, (start, goal), at) {
        Some(way) => finish(way, from, to, at),
        None => vec![from, to],
    }
}

/// The cheapest way over a grid of `nx` × `nz` samples `step` blocks
/// apart (`cost` of crossing each, `h` its height) from sample `start` to
/// `goal`, as their indices; `None` where there is none.
fn cheapest(cost: &[f32], h: &[f32], (nx, nz): (usize, usize), step: f32, (start, goal): (usize, usize), at: impl Fn(usize) -> (f32, f32)) -> Option<Vec<usize>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    #[derive(PartialEq)]
    struct Key(f32);
    impl Eq for Key {}
    impl Ord for Key {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&o.0)
        }
    }
    impl PartialOrd for Key {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    let (gx, gz) = at(goal);
    let guess = |k: usize| {
        let (x, z) = at(k);
        0.6 * (x - gx).hypot(z - gz)
    };
    let mut best = vec![f32::INFINITY; nx * nz];
    let mut came = vec![u32::MAX; nx * nz];
    let mut heap = BinaryHeap::new();
    best[start] = 0.0;
    heap.push(Reverse((Key(guess(start)), start as u32)));
    while let Some(Reverse((Key(f), c))) = heap.pop() {
        let c = c as usize;
        if c == goal {
            break;
        }
        let g = best[c];
        if f > g + guess(c) + 1e-3 {
            continue;
        }
        let (ci, cj) = ((c % nx) as i32, (c / nx) as i32);
        for (di, dj) in AROUND {
            let (ni, nj) = (ci + di, cj + dj);
            if ni < 0 || nj < 0 || ni >= nx as i32 || nj >= nz as i32 {
                continue;
            }
            let n = nj as usize * nx + ni as usize;
            if !cost[n].is_finite() {
                continue;
            }
            let d = if di != 0 && dj != 0 { step * std::f32::consts::SQRT_2 } else { step };
            let next = g + d * 0.5 * (cost[c] + cost[n]) + CLIMB * (h[n] - h[c]).max(0.0);
            if next < best[n] {
                best[n] = next;
                came[n] = c as u32;
                heap.push(Reverse((Key(next + guess(n)), n as u32)));
            }
        }
    }
    if goal != start && came[goal] == u32::MAX {
        return None;
    }
    let mut way = vec![goal];
    while *way.last().expect("the goal") != start {
        way.push(came[*way.last().expect("the goal")] as usize);
    }
    way.reverse();
    Some(way)
}

/// The way over the grid (`way`, samples' indices) in blocks, from `from`
/// to `to`, its corners cut.
fn finish(way: Vec<usize>, from: (f32, f32), to: (f32, f32), at: impl Fn(usize) -> (f32, f32)) -> Vec<(f32, f32)> {
    let mut points: Vec<(f32, f32)> = Vec::with_capacity(way.len() + 2);
    points.push(from);
    points.extend(way[1..way.len().saturating_sub(1)].iter().map(|&k| at(k)).map(|(x, z)| (x + 0.5, z + 0.5)));
    points.push(to);
    // The corners of the grid cut, twice.
    for _ in 0..2 {
        let mut cut = Vec::with_capacity(points.len() * 2);
        cut.push(points[0]);
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            cut.push((a.0 + (b.0 - a.0) * 0.25, a.1 + (b.1 - a.1) * 0.25));
            cut.push((a.0 + (b.0 - a.0) * 0.75, a.1 + (b.1 - a.1) * 0.75));
        }
        cut.push(*points.last().expect("two points"));
        points = cut;
    }
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for (k, &p) in points.iter().enumerate() {
        if k + 1 == points.len() || out.last().is_none_or(|q: &(f32, f32)| (q.0 - p.0).hypot(q.1 - p.1) >= 1.5) {
            out.push(p);
        }
    }
    out
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
        slope_of(50.0 / 256.0)
    }

    /// Two tiles down a slope of `grade` from 120, a lake (its water at 75)
    /// from x = 230 on.
    fn slope_of(grade: f32) -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..2 {
            let mut tile = Tile::ground(0.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let bx = tx * 128 + x;
                    let i = local_index(bx, z);
                    tile.top.set(i, 120.0 - bx as f32 * grade);
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

    /// A way from one side of a hill to the other goes round it, through
    /// the low ground, without a break.
    #[test]
    fn a_route_goes_round_a_hill() {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tz in 0..2 {
            for tx in 0..2 {
                let mut tile = Tile::ground(70.0, 1, NO_FLUID);
                for z in 0..128 {
                    for x in 0..128 {
                        let (bx, bz) = (tx * 128 + x, tz * 128 + z);
                        let d = ((bx - 128) as f32).hypot((bz - 128) as f32);
                        tile.top.set(local_index(bx, bz), 70.0 + (90.0 - d).max(0.0) * 1.2);
                    }
                }
                dim.ground_mut().insert(TilePos::new(tx, tz), tile);
            }
        }
        world.dims = vec![dim];
        let level = world.dims[0].ground();
        for cells in [400_000, 20_000] {
            let way = route(level, (10.0, 128.0), (246.0, 128.0), cells);
            assert_eq!(way.first(), Some(&(10.0, 128.0)));
            assert_eq!(way.last(), Some(&(246.0, 128.0)));
            let mut ground = Ground { level, tiles: HashMap::new() };
            let top = way.iter().map(|p| ground.at(p.0 as i32, p.1 as i32).unwrap().0).fold(0.0f32, f32::max);
            assert!(top < 95.0, "round the hill, not over it: up to {top}");
            let gap = way.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).fold(0.0f32, f32::max);
            assert!(gap < 12.0, "no break: {gap}");
        }
    }

    /// Water let go on a slope runs down it, into the lake at its foot.
    #[test]
    fn a_spring_runs_down_into_the_lake() {
        let world = slope();
        let d = descend(world.dims[0].ground(), (20, 64), 0.0, 10_000, 100_000, 1.0);
        assert_eq!(d.mouth, Some(Mouth::Water), "{:?}", d.mouth);
        let last = *d.course.last().unwrap();
        assert!(last.0 >= 229.0, "it reaches the lake: {last:?}");
        // Straight down the slope: it hardly strays from its row.
        assert!(d.course.iter().all(|p| (p.1 - 64.5).abs() < 12.0), "down the slope");
        assert!(d.lakes.is_empty());
        // Dug: a bed with water all the way.
        let mut world = world;
        let mut h = History::default();
        h.begin("spring", 0);
        let report = dig_descent(&mut h.edit(&mut world, 0), &d, &RiverSpec::default());
        h.commit();
        assert!(report.wet > 1000 && report.length > 200.0, "{report:?}");
        assert!(report.source > report.mouth && report.mouth >= 75.0 - 0.01, "{report:?}");
    }

    /// The water of a river down a slope stays in its bed: no water beside
    /// open land lower than it, and once the export settles its steps, the
    /// game's rules leave all of it as it is.
    #[test]
    fn a_river_down_a_slope_holds_its_water() {
        let mut world = slope();
        let d = descend(world.dims[0].ground(), (20, 64), 0.0, 10_000, 100_000, 1.0);
        let mut h = History::default();
        h.begin("spring", 0);
        dig_descent(&mut h.edit(&mut world, 0), &d, &RiverSpec::default());
        h.commit();
        let dim = &world.dims[0];
        let check = crate::water::check(dim);
        assert_eq!(check.spills, 0, "{check:?}");
        assert!(check.steps > 50, "it steps down the slope: {check:?}");
        // As exported, the game changes none of it.
        for pos in [TilePos::new(0, 0), TilePos::new(1, 0)] {
            let s = crate::water::settle_tile(dim, pos, true);
            assert_eq!((s.unsettled, s.over_land, s.sources), (0, 0, 0), "{pos:?}: {s:?}");
        }
        // As still water only, the steps would run once a block changes.
        let s = crate::water::settle_tile(dim, TilePos::new(0, 0), false);
        assert!(s.added > 0, "{s:?}");
    }

    /// Down a slope of one block in three, the river falls from pool to
    /// pool: its water stays level for a while, then drops two blocks or
    /// more at once, a plunge pool dug deeper under the fall; on gentler
    /// ground (one in five, one in twelve) it steps down a block at a time.
    #[test]
    fn a_steep_river_falls_from_pool_to_pool() {
        let mut level = vec![0.0f32; 400];
        // 60 blocks flat at 100, then down one block in three, then flat.
        for (k, l) in level.iter_mut().enumerate() {
            let x = k as f32 * STEP;
            *l = 100.0 - ((x - 60.0).clamp(0.0, 100.0) / 3.0).floor();
        }
        let before = level.clone();
        let plunge = pools_and_falls(&mut level, STEP, &|_| 6.0, &|_| false);
        let drops = |l: &[f32]| l.windows(2).map(|w| (w[0].round() - w[1].round()) as i32).filter(|d| *d > 0).collect::<Vec<_>>();
        assert!(drops(&before).iter().all(|d| *d == 1));
        // Falls of two blocks or more down the slope; steps of a block
        // where it starts and where it eases off at its foot.
        let falls = drops(&level);
        assert!(falls.iter().filter(|d| **d >= 2).count() >= 6, "{falls:?}");
        assert!(falls.iter().filter(|d| **d == 1).count() <= 8, "{falls:?}");
        assert!(falls.len() * 10 < drops(&before).len() * 7, "{falls:?}");
        // Never over what the land allows, nor under where it ends.
        assert!(level.iter().zip(&before).all(|(a, b)| a <= b && *a >= 67.0));
        // A plunge pool under each fall, none on the flat.
        assert!(plunge[..100].iter().all(|d| *d == 0.0));
        assert!(plunge.iter().any(|d| *d >= 1.0));
        // On gentler ground, nothing changes: rapids, a block at a time.
        for run in [5.0, 12.0] {
            let mut gentle: Vec<f32> = (0..400).map(|k| 100.0 - (k as f32 * STEP / run).floor()).collect();
            let was = gentle.clone();
            pools_and_falls(&mut gentle, STEP, &|_| 6.0, &|_| false);
            assert_eq!(gentle, was, "one in {run}");
        }
    }

    /// Down a slope of 18 %, a river steps down a block at a time, with no
    /// pool or fall, and its banks slope down to it as one valley: along
    /// each bank the ground drops as evenly as the slope, without a ledge.
    #[test]
    fn a_river_down_a_moderate_slope_is_a_valley() {
        let mut world = slope_of(0.18);
        let spec = RiverSpec { width: 8.0, width_end: 12.0, depth: 3.0, depth_end: 4.0, banks: 10.0, ..RiverSpec::default() };
        let mut h = History::default();
        h.begin("river", 0);
        carve_river(&mut h.edit(&mut world, 0), &[(20.0, 64.0), (225.0, 64.0)], &spec);
        h.commit();
        let first_air = |x: i32, z: i32| crate::compose::block_top(column(&world, x, z).0);
        // The water down its middle: a block at a time.
        let mut levels: Vec<i16> = (30..220).map(|x| column(&world, x, 64).1).filter(|&f| f != NO_FLUID).collect();
        levels.dedup();
        let drops: Vec<i16> = levels.windows(2).map(|w| w[0] - w[1]).collect();
        assert!(levels.len() > 15 && drops.iter().all(|&d| d == 1), "{levels:?}");
        // Along each bank, from its foot to its top: no ledge, the ground
        // dropping a block or two at a time down the slope, never rising.
        let mut ledges = Vec::new();
        for dz in [-10, -8, -6, 6, 8, 10] {
            for x in 30..215 {
                let (a, b) = (first_air(x, 64 + dz), first_air(x + 1, 64 + dz));
                if a - b >= 2 || b > a {
                    ledges.push((x, dz, a, b));
                }
            }
        }
        assert!(ledges.len() <= 3, "{} ledges: {:?}", ledges.len(), &ledges[..ledges.len().min(12)]);
    }

    /// The bed is gravel in its middle and sand in its shallows and on the
    /// inside of its bends.
    #[test]
    fn a_river_leaves_sand_in_its_shallows() {
        let mut world = slope();
        let gravel = world.registry.terrain_by_name("Gravel").expect("gravel");
        let sand = world.registry.terrain_by_name("Sand").expect("sand");
        let spec = RiverSpec {
            width: 10.0,
            width_end: 14.0,
            bed: Some(gravel),
            shore: Some(sand),
            ..RiverSpec::default()
        };
        let mut h = History::default();
        h.begin("river", 0);
        // Down the slope with a bend, into the lake.
        carve_river(&mut h.edit(&mut world, 0), &[(20.0, 64.0), (120.0, 64.0), (170.0, 30.0), (240.0, 40.0)], &spec);
        h.commit();
        let (mut gravel_n, mut sand_n) = (0, 0);
        for z in 0..128 {
            for x in 0..230 {
                let t = world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
                let i = local_index(x, z);
                if t.fluid.get(i) != NO_FLUID && t.fluid.get(i) as f32 > t.top.get(i) + 0.3 {
                    gravel_n += (t.terrain.get(i) == gravel.0) as usize;
                    sand_n += (t.terrain.get(i) == sand.0) as usize;
                }
            }
        }
        assert!(gravel_n > sand_n && sand_n * 6 > gravel_n, "gravel {gravel_n}, sand {sand_n}");
    }

    /// Land falling from 100 to the sea (63) at x = 180, the sea floor on
    /// down to 40 at x = 383.
    fn coast() -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..3 {
            let mut tile = Tile::ground(0.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let bx = tx * 128 + x;
                    let i = local_index(bx, z);
                    let h = if bx < 180 { 100.0 - bx as f32 * 37.0 / 180.0 } else { 63.0 - (bx - 180) as f32 * 23.0 / 203.0 };
                    tile.top.set(i, h);
                    if h < 62.5 {
                        tile.fluid.set(i, 63);
                    }
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        world
    }

    /// Into the sea, the river does not widen: its channel goes on under the
    /// water, cut into the sea floor, shallower and shallower until it fades
    /// into it, with sand on its floor; the sea's water stays as it was.
    #[test]
    fn a_river_goes_on_under_the_sea_and_fades_into_its_floor() {
        let mut world = coast();
        let before = world.clone();
        let d = descend(world.dims[0].ground(), (20, 64), 63.0, 10_000, 100_000, 1.0);
        assert_eq!(d.mouth, Some(Mouth::Sea));
        assert_eq!(d.sea, Some(63.0));
        let sand = world.registry.terrain_by_name("Sand").expect("sand");
        let spec = RiverSpec {
            width: 8.0,
            width_end: 12.0,
            depth: 3.0,
            depth_end: 5.0,
            bed: world.registry.terrain_by_name("Gravel"),
            shore: Some(sand),
            ..RiverSpec::default()
        };
        let mut h = History::default();
        h.begin("river", 0);
        dig_descent(&mut h.edit(&mut world, 0), &d, &spec);
        h.commit();
        let column = |w: &World, x: i32, z: i32| {
            let t = w.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
            let i = local_index(x, z);
            (t.top.get(i), t.fluid.get(i), t.terrain.get(i))
        };
        // How much deeper than the floor the middle of the channel is, past
        // the coast.
        let cut = |x: i32| (60..69).map(|z| column(&before, x, z).0 - column(&world, x, z).0).fold(0.0f32, f32::max);
        let near = cut(195);
        let farther = cut(215);
        assert!(near >= 2.0, "a channel under the sea by the coast: {near}");
        assert!(farther < near, "fading out: {near} by the coast, {farther} farther");
        assert_eq!(cut(300), 0.0, "and gone far out");
        // The sea's water and the floor away from the channel untouched
        // (from x = 185, where the fixture's sea has water).
        for x in 185..383 {
            assert_eq!(column(&world, x, 64).1, 63);
            assert_eq!(column(&world, x, 20).0, column(&before, x, 20).0);
        }
        // Sand on its floor.
        assert_eq!(column(&world, 195, 64).2, sand.0);
        // Not wider at the coast than upstream: no funnel.
        let wet = |x: i32| (0..128).filter(|&z| column(&world, x, z).0 < column(&before, x, z).0 - 1.0).count();
        assert!(wet(176) <= wet(150) + 6, "{} dug at the coast, {} upstream", wet(176), wet(150));
    }

    /// Flat ground at 70 over 3 × 3 tiles.
    fn flat() -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tz in 0..3 {
            for tx in 0..3 {
                dim.ground_mut().insert(TilePos::new(tx, tz), Tile::ground(70.0, 1, NO_FLUID));
            }
        }
        world.dims = vec![dim];
        world
    }

    /// The course made natural: a loop round a hollow is cut, a staircase
    /// along the grid smoothed, meanders on the flat and none down a steep
    /// slope; the points it must pass stay where they are.
    #[test]
    fn a_course_is_made_natural() {
        let world = flat();
        let level = world.dims[0].ground();
        // East, a loop round a circle 30 blocks wide, then on east.
        let mut looped: Vec<(f32, f32)> = (0..100).map(|x| (20.0 + x as f32, 190.0)).collect();
        for a in 0..94 {
            let t = a as f32 / 94.0 * std::f32::consts::TAU;
            looped.push((120.0 + 15.0 * t.sin(), 175.0 + 15.0 * t.cos()));
        }
        looped.extend((0..100).map(|x| (121.0 + x as f32, 190.0)));
        let (out, _) = natural_course(level, &looped, 8.0, &[]);
        let length = |c: &[(f32, f32)]| c.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum::<f32>();
        assert!(length(&out) < length(&looped) - 60.0, "the loop is cut: {} → {}", length(&looped), length(&out));
        assert!(out.iter().all(|p| p.1 > 165.0), "nothing left of the loop's far side");
        // A staircase along the grid, a block east then a block south.
        let stairs: Vec<(f32, f32)> = (0..300).map(|k| (20.0 + ((k + 1) / 2) as f32, 20.0 + (k / 2) as f32)).collect();
        let (out, _) = natural_course(level, &stairs, 8.0, &[]);
        let turn = |c: &[(f32, f32)]| {
            c.windows(3).map(|w| {
                let (a, b) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), (w[2].0 - w[1].0, w[2].1 - w[1].1));
                (a.0 * b.1 - a.1 * b.0).abs() / (a.0.hypot(a.1) * b.0.hypot(b.1)).max(1e-6)
            })
            .fold(0.0f32, f32::max)
        };
        assert!(turn(&stairs) > 0.9 && turn(&out[20..out.len() - 20]) < 0.5, "smoothed: {} → {}", turn(&stairs), turn(&out[20..out.len() - 20]));
        // Straight east over the flat: it meanders, half its width to
        // either side at most, through its anchor.
        let straight: Vec<(f32, f32)> = (0..340).map(|x| (10.0 + x as f32, 190.0)).collect();
        let (out, anchors) = natural_course(level, &straight, 8.0, &[170]);
        let off = out.iter().map(|p| (p.1 - 190.0).abs()).fold(0.0f32, f32::max);
        assert!(off > 2.0 && off <= 4.5, "meanders on the flat: {off}");
        assert_eq!(out[anchors[0]], straight[170], "through its anchor");
        assert_eq!((out[0], *out.last().unwrap()), (straight[0], *straight.last().unwrap()));
        // Down a steep slope, straight on.
        let world = slope();
        let down: Vec<(f32, f32)> = (0..200).map(|x| (10.0 + x as f32, 64.0)).collect();
        let (out, _) = natural_course(world.dims[0].ground(), &down, 8.0, &[]);
        let off = out.iter().map(|p| (p.1 - 64.0).abs()).fold(0.0f32, f32::max);
        assert!(off < 0.5, "no meanders down a slope: {off}");
    }

    /// Down a slope, the land draining through a river grows from its
    /// source to its mouth, and never shrinks; down a valley that gathers
    /// the water of its sides, it grows faster.
    #[test]
    fn a_river_gathers_more_land_downstream() {
        let world = slope();
        let mut ground = Ground::new(world.dims[0].ground());
        let course: Vec<(f32, f32)> = (0..400).map(|k| (20.0 + k as f32 * 0.5, 64.5)).collect();
        let gathered = catchment(&mut ground, &course, course.len(), 0.0);
        assert!(gathered.windows(2).all(|w| w[1] >= w[0]));
        assert!(gathered[399] > 4.0 * gathered[20], "{} → {}", gathered[20], gathered[399]);
    }

    /// A hollow on the way fills into a lake up to where it overflows, and
    /// the water runs on from there to the sea.
    #[test]
    fn a_spring_fills_the_hollows_on_its_way() {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..3 {
            let mut tile = Tile::ground(0.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let bx = tx * 128 + x;
                    let i = local_index(bx, z);
                    // Falling east to the sea, a bowl 12 deep at x = 180.
                    let r = (((bx - 180) * (bx - 180) + (z - 64) * (z - 64)) as f32).sqrt();
                    let h = 130.0 - bx as f32 * 0.25 - (12.0 - r * 0.4).max(0.0);
                    tile.top.set(i, h);
                    if h < 62.0 {
                        tile.fluid.set(i, 62);
                    }
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        let d = descend(world.dims[0].ground(), (40, 64), 62.0, 10_000, 100_000, 1.0);
        assert_eq!(d.lakes.len(), 1, "one lake: {:?}", d.lakes.iter().map(|l| (l.0.len(), l.1)).collect::<Vec<_>>());
        let (cells, level) = &d.lakes[0];
        assert!(cells.len() > 100, "a lake of {} columns", cells.len());
        assert!(cells.contains(&(180, 64)), "where the bowl is");
        assert!(*level < 130.0 - 180.0 * 0.25 + 1.0, "up to its rim: {level}");
        assert_eq!(d.mouth, Some(Mouth::Sea), "and on to the sea");
        assert!(d.course.last().unwrap().0 > 270.0);
        // Asked for lakes deeper than the hollow: none; the river goes
        // through it, and digging it cuts through the rim.
        let through = descend(world.dims[0].ground(), (40, 64), 62.0, 10_000, 100_000, 20.0);
        assert!(through.lakes.is_empty() && through.mouth == Some(Mouth::Sea));
        let mut h = History::default();
        h.begin("spring", 0);
        let report = dig_descent(&mut h.edit(&mut world, 0), &through, &RiverSpec::default());
        h.commit();
        let (rim, _) = column(&world, 192, 64);
        assert!(rim < 130.0 - 192.0 * 0.25 - 2.0, "the rim is cut: {rim}, {report:?}");
    }

    /// A deep river (its water well under the land) that fills lakes on
    /// its way stands at a lake's level at least above it: a lake never
    /// stands over the river that feeds it, and the water never rises
    /// downstream. Here, in the hills, water let go at a spring fills two
    /// hollows; dug wide and deep, the river stood at 63 where it ran into
    /// a lake at 68.
    #[test]
    fn a_deep_river_feeds_its_lakes_from_above() {
        let mut world = crate::generate::generate(&crate::generate::GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: crate::generate::Landform::Hills,
            seed: 1,
            ..crate::generate::GenParams::default()
        });
        let sea = world.meta.sea_level as f32;
        let mut d = descend(world.dims[0].ground(), (-208, -240), sea, 60_000, 2_000_000, 1.0);
        assert!(!d.lakes.is_empty(), "lakes on its way");
        // As the River tool lays it out, 40 blocks wide and its flow all
        // the way up.
        d.course = natural_course(world.dims[0].ground(), &d.course, 120.0, &[0]).0;
        let spec = RiverSpec { width: 40.0, width_end: 200.0, depth: 10.0, depth_end: 25.0, banks: 200.0, ..RiverSpec::default() };
        let mut h = History::default();
        h.begin("spring", 0);
        dig_descent(&mut h.edit(&mut world, 0), &d, &spec);
        h.commit();
        let lakes: Vec<i16> = d.lakes.iter().map(|l| l.1.floor() as i16).collect();
        let (mut lowest, mut fed) = (i16::MAX, 0);
        for &(x, z) in &d.course {
            let (x, z) = (x.floor() as i32, z.floor() as i32);
            if world.dims[0].ground().view(TilePos::containing(x, z)).is_none() {
                continue;
            }
            let (top, fluid) = column(&world, x, z);
            if fluid == NO_FLUID || fluid as f32 <= top {
                continue;
            }
            assert!(fluid <= lowest, "the water rises from {lowest} to {fluid} at {x}, {z} (its lakes at {lakes:?})");
            lowest = fluid;
            fed += lakes.contains(&fluid) as usize;
        }
        assert!(fed > 0, "the river runs through its lakes, at {lakes:?}");
        assert_eq!(crate::water::check(&world.dims[0]).spills, 0);
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
        // Upstream, near the source: a bed under water, under the ground,
        // at its deepest across (the slope is steep there: a narrow V).
        let deepest = (40..90).min_by(|&a, &b| column(&world, 20, a).0.total_cmp(&column(&world, 20, b).0)).unwrap();
        let (top, fluid) = column(&world, 20, deepest);
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
