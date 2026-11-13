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
    let along = |k: usize, a: f32, b: f32| a + (b - a) * k as f32 / (course.len() - 1) as f32;
    let widths: Vec<f32> = (0..course.len()).map(|k| half(along(k, spec.width, spec.width_end))).collect();
    let depths: Vec<f32> = (0..course.len()).map(|k| along(k, spec.depth, spec.depth_end).max(0.5)).collect();

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
                    water - depths[k] * (1.0 - t * t)
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
}

impl Mouth {
    pub const fn label(self) -> &'static str {
        match self {
            Mouth::Sea => "the sea",
            Mouth::Water => "the water it meets",
            Mouth::Edge => "the edge of the map",
            Mouth::Lake => "a lake",
            Mouth::Far => "as far as it may run",
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
            let water = level.round().clamp(lo, hi) as i16;
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
        }
    }
    // A point every two blocks is plenty for the curve.
    let points: Vec<(f32, f32)> = descent
        .course
        .iter()
        .enumerate()
        .filter(|(k, _)| k % 2 == 0 || *k + 1 == descent.course.len())
        .map(|(_, p)| *p)
        .collect();
    carve_river(edit, &points, spec)
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
