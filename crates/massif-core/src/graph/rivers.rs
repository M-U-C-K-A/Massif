//! Rivers traced over a relief, the way its water finds its way down.
//!
//! The water's way is found on a copy of the relief at most [`ROUTE_MAX`]
//! samples a side, flooded up from the sea and the map's edges lowest
//! first: every cell drains to the one the flood reached it from. Hollows
//! so spill where they overflow, and flats drain towards their outlet
//! instead of in straight parallel lines. Where enough of the land drains,
//! a river runs: a main stem from each mouth up to its farthest source,
//! tributaries off it, each joining where its water does.
//!
//! Each river then becomes a smooth line laid in the bottom of its valley
//! on the relief itself, wider and deeper as more water joins it. Its
//! water level only ever goes down, stays under its banks, meets the sea
//! at its mouth and the river it joins at its end. Its bed and banks are
//! dug at every sample of the relief — a 16k build gets rivers drawn at
//! the block — and the hollows it runs through hold lakes.

use super::ops::smoothstep;
use super::{Field, NO_WATER};
use crate::noise::Noise;
use rayon::prelude::*;
use std::cmp::Ordering as Cmp;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, Ordering};

/// The most samples a side the water's way is found at.
pub(super) const ROUTE_MAX: usize = 2048;

const NONE: u32 = u32::MAX;

const NEIGHBOURS: [(i32, i32); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];

/// What shapes the rivers.
#[derive(Debug, Clone, Copy)]
pub(super) struct RiverParams {
    /// 0..1: how much of the land must drain to a place for a river to run
    /// there (higher: fewer, larger rivers).
    pub threshold: f32,
    /// Depth of the largest river, in blocks.
    pub depth: f32,
    /// Width of the largest river, in blocks.
    pub width: f32,
    /// How far the banks slope down to the water, in river widths.
    pub banks: f32,
    /// 0..1: which hollows a river fills with a lake rather than cutting
    /// through their rim (0: none, 1: all).
    pub lakes: f32,
    /// The sea's level ([`NO_WATER`]: no sea).
    pub sea: f32,
}

/// What [`rivers`] makes, at every sample of the relief.
pub(super) struct RiverMaps {
    /// The relief with the rivers' beds and banks dug in.
    pub dug: Field,
    /// 1 in the rivers' beds, fading out over their banks.
    pub mask: Field,
    /// The level of the water of the rivers and of the lakes they run
    /// through ([`NO_WATER`] elsewhere, and where the sea is).
    pub water: Field,
    /// How many rivers run: main stems and tributaries.
    #[cfg_attr(not(test), allow(dead_code))]
    pub count: usize,
}

/// A cell waiting in the flood: lowest first; in a hollow (all of it at
/// the level it spills at) the lowest ground first, so that the way across
/// it follows the bottom of its valley; then first come.
#[derive(PartialEq)]
struct Entry {
    h: f32,
    ground: f32,
    seq: u32,
    c: u32,
}

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Cmp {
        o.h.total_cmp(&self.h).then(o.ground.total_cmp(&self.ground)).then(o.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Cmp> {
        Some(self.cmp(o))
    }
}

/// How a relief drains.
struct Drainage {
    /// The cell each cell drains to ([`NONE`]: the sea, or off the map).
    receiver: Vec<u32>,
    /// Cells in the order the flood reached them: outlets first.
    order: Vec<u32>,
    /// The relief with its hollows filled to where they spill.
    filled: Vec<f32>,
}

/// Flood `r` up from its edges and from what lies under `sea`.
fn drain(r: &[f32], m: usize, sea: f32, cancel: &AtomicBool) -> Option<Drainage> {
    let mut filled = r.to_vec();
    let mut receiver = vec![NONE; m * m];
    let mut done = vec![false; m * m];
    let mut order = Vec::with_capacity(m * m);
    let mut heap = BinaryHeap::with_capacity(8 * m);
    let mut seq = 0u32;
    for c in 0..m * m {
        let (i, j) = (c % m, c / m);
        if i == 0 || j == 0 || i == m - 1 || j == m - 1 || r[c] <= sea {
            done[c] = true;
            filled[c] = filled[c].max(sea);
            heap.push(Entry {
                h: filled[c],
                ground: r[c],
                seq,
                c: c as u32,
            });
            seq += 1;
        }
    }
    while let Some(Entry { h, c, .. }) = heap.pop() {
        order.push(c);
        if order.len() % 65_536 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (i, j) = (c as usize % m, c as usize / m);
        for (di, dj) in NEIGHBOURS {
            let (ni, nj) = (i as i32 + di, j as i32 + dj);
            if ni < 0 || nj < 0 || ni >= m as i32 || nj >= m as i32 {
                continue;
            }
            let nb = nj as usize * m + ni as usize;
            if done[nb] {
                continue;
            }
            done[nb] = true;
            receiver[nb] = c;
            filled[nb] = filled[nb].max(h);
            heap.push(Entry {
                h: filled[nb],
                ground: r[nb],
                seq,
                c: nb as u32,
            });
            seq += 1;
        }
    }
    Some(Drainage { receiver, order, filled })
}

/// A river as cells of the routing grid: from its mouth up.
struct Stem {
    cells: Vec<u32>,
    /// The river it joins and the place along it (an index of its cells).
    parent: Option<(usize, usize)>,
    /// The sea cell it flows into.
    mouth: Option<u32>,
}

/// A point along a river, in samples of the relief.
#[derive(Debug, Clone, Copy, Default)]
struct Point {
    x: f32,
    y: f32,
    /// How much of the land drains through it, in routing cells.
    area: f32,
    /// Half its width, in samples.
    hw: f32,
    /// Its depth in the middle, in blocks.
    depth: f32,
    /// The level of its water.
    level: f32,
}

/// A moving average of the points' positions, its ends left where they are.
fn smooth(points: &mut [Point], radius: usize, passes: usize) {
    let n = points.len();
    if n < 3 {
        return;
    }
    for _ in 0..passes {
        let copy: Vec<(f32, f32)> = points.iter().map(|p| (p.x, p.y)).collect();
        for i in 1..n - 1 {
            let r = radius.min(i).min(n - 1 - i);
            let (mut sx, mut sy) = (0.0, 0.0);
            for p in &copy[i - r..=i + r] {
                sx += p.0;
                sy += p.1;
            }
            let k = (2 * r + 1) as f32;
            points[i].x = sx / k;
            points[i].y = sy / k;
        }
    }
}

/// A moving average of one value of the points.
fn smooth_value(points: &mut [Point], radius: usize, get: impl Fn(&Point) -> f32, set: impl Fn(&mut Point, f32)) {
    let n = points.len();
    if n < 3 {
        return;
    }
    let copy: Vec<f32> = points.iter().map(&get).collect();
    for (i, p) in points.iter_mut().enumerate() {
        let (a, b) = (i.saturating_sub(radius), (i + radius).min(n - 1));
        set(p, copy[a..=b].iter().sum::<f32>() / (b - a + 1) as f32);
    }
}

/// Points every `step` samples along a line through `points` (Catmull-Rom),
/// with the values in between.
fn resample(points: &[Point], step: f32) -> Vec<Point> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    let mut carry = 0.0f32;
    for k in 0..points.len() - 1 {
        let (p0, p1, p2, p3) = (points[k.saturating_sub(1)], points[k], points[k + 1], points[(k + 2).min(points.len() - 1)]);
        let len = (p2.x - p1.x).hypot(p2.y - p1.y);
        // Enough sub-steps to walk the curve at about `step`.
        let subs = ((len / step).ceil() as usize).max(1) * 4;
        let mut prev = (p1.x, p1.y);
        for s in 1..=subs {
            let t = s as f32 / subs as f32;
            let cr = |a: f32, b: f32, c: f32, d: f32| {
                let (t2, t3) = (t * t, t * t * t);
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            let (x, y) = (cr(p0.x, p1.x, p2.x, p3.x), cr(p0.y, p1.y, p2.y, p3.y));
            carry += (x - prev.0).hypot(y - prev.1);
            prev = (x, y);
            if carry >= step || (k == points.len() - 2 && s == subs) {
                carry = 0.0;
                let lerp = |a: f32, b: f32| a + (b - a) * t;
                out.push(Point {
                    x,
                    y,
                    area: lerp(p1.area, p2.area),
                    ..Point::default()
                });
            }
        }
    }
    out
}

/// A piece of a river between two of its points, with what it digs.
struct Segment {
    a: Point,
    b: Point,
    /// Half the width of its banks, in samples.
    bank: f32,
}

/// Trace the rivers of relief `f` (`spacing` blocks between its samples)
/// and dig them in. `None` if cancelled.
pub(super) fn rivers(f: &Field, spacing: f32, p: RiverParams, cancel: &AtomicBool) -> Option<RiverMaps> {
    let n = f.n;
    let unchanged = || RiverMaps {
        dug: f.clone(),
        mask: Field::new(n, vec![0.0; n * n]),
        water: Field::new(n, vec![NO_WATER; n * n]),
        count: 0,
    };
    if n < 8 {
        return Some(unchanged());
    }
    // ---- the water's way, on a copy of the relief at most ROUTE_MAX a side
    let m = n.min(ROUTE_MAX);
    let k = n as f32 / m as f32;
    let coarse = if m == n { f.clone() } else { super::ops::downsample(f, m) };
    // A breath of noise, a fraction of a block, so that water finds a way
    // across flats as it would, not in straight lines.
    let noise = Noise::new(0x51ee_7a11);
    let cell = k * spacing;
    let route: Vec<f32> = (0..m * m)
        .into_par_iter()
        .map(|c| {
            let (x, z) = ((c % m) as f64 * cell as f64, (c / m) as f64 * cell as f64);
            coarse.data[c] + 0.3 * noise.fbm(x / 40.0, z / 40.0, 3, 2.0, 0.5) as f32
        })
        .collect();
    let sea = p.sea;
    let land = |c: usize| route[c] > sea;
    let d = drain(&route, m, sea, cancel)?;
    let mut area = vec![1.0f32; m * m];
    for &c in d.order.iter().rev() {
        let r = d.receiver[c as usize];
        if r != NONE {
            area[r as usize] += area[c as usize];
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let land_cells = (0..m * m).filter(|&c| land(c)).count().max(1) as f32;
    // From a 40th of a percent of the land (a dense network of streams) to
    // a tenth of it (only the great rivers).
    let a_min = (land_cells * 10f32.powf(-3.6 + 2.6 * p.threshold.clamp(0.0, 1.0))).max(4.0);
    let channel = |c: usize| land(c) && area[c] >= a_min;

    // ---- rivers: main stems from the mouths up, tributaries off them
    let mut outlets: Vec<u32> = (0..m * m)
        .filter(|&c| channel(c) && {
            let r = d.receiver[c];
            r == NONE || !land(r as usize)
        })
        .map(|c| c as u32)
        .collect();
    outlets.sort_by(|a, b| area[*a as usize].total_cmp(&area[*b as usize]));
    let mut stack: Vec<(u32, Option<(usize, usize)>)> = outlets.into_iter().map(|c| (c, None)).collect();
    let mut stems: Vec<Stem> = Vec::new();
    while let Some((start, parent)) = stack.pop() {
        let id = stems.len();
        let mut cells = Vec::new();
        let mut c = start as usize;
        loop {
            cells.push(c as u32);
            let (i, j) = (c % m, c / m);
            let mut best: Option<usize> = None;
            for (di, dj) in NEIGHBOURS {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if ni < 0 || nj < 0 || ni >= m as i32 || nj >= m as i32 {
                    continue;
                }
                let nb = nj as usize * m + ni as usize;
                if d.receiver[nb] as usize != c || !channel(nb) {
                    continue;
                }
                match best {
                    Some(b) if area[b] >= area[nb] => stack.push((nb as u32, Some((id, cells.len() - 1)))),
                    Some(b) => {
                        stack.push((b as u32, Some((id, cells.len() - 1))));
                        best = Some(nb);
                    }
                    None => best = Some(nb),
                }
            }
            match best {
                Some(b) => c = b,
                None => break,
            }
        }
        let mouth = match (parent, d.receiver[start as usize]) {
            (None, r) if r != NONE => Some(r),
            _ => None,
        };
        stems.push(Stem { cells, parent, mouth });
    }
    if stems.is_empty() {
        return Some(unchanged());
    }
    // Stubs a few cells long that nothing joins are left out.
    let mut joined = vec![0usize; stems.len()];
    for s in &stems {
        if let Some((p, _)) = s.parent {
            joined[p] += 1;
        }
    }
    let kept: Vec<bool> = stems.iter().enumerate().map(|(k, s)| s.parent.is_none() || joined[k] > 0 || s.cells.len() >= 4).collect();
    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // ---- each river as a line in the bottom of its valley
    let q_max = stems.iter().filter_map(|s| s.cells.first()).map(|&c| area[c as usize]).fold(1.0f32, f32::max);
    let at = |c: u32| -> (f32, f32) { (((c as usize % m) as f32 + 0.5) * k - 0.5, ((c as usize / m) as f32 + 0.5) * k - 0.5) };
    let coarse_at = |x: f32, y: f32| ((x + 0.5) / k - 0.5, (y + 0.5) / k - 0.5);
    let filled = Field::new(m, d.filled);
    let routed = Field::new(m, route.clone());
    // ---- the hollows the rivers run through: lakes, to where they spill
    let mut lake = vec![NO_WATER; m * m];
    let mut seen = vec![false; m * m];
    // A hollow: all of it under the level it spills at. One a river runs
    // through holds a lake if the water would stand deep enough in it;
    // through the others, the river cuts its way.
    let lake_depth = if p.lakes <= 0.001 { f32::MAX } else { 1.0 + (1.0 - p.lakes.min(1.0)) * 24.0 };
    let hollow = |c: usize| land(c) && filled.data[c] > routed.data[c] + 0.05;
    let on_river: Vec<bool> = {
        let mut on = vec![false; m * m];
        for (id, s) in stems.iter().enumerate() {
            if kept[id] {
                for &c in &s.cells {
                    on[c as usize] = true;
                }
            }
        }
        on
    };
    for start in 0..m * m {
        if seen[start] || !hollow(start) {
            continue;
        }
        let mut region = vec![start];
        seen[start] = true;
        let mut next = 0;
        let (mut river, mut deepest) = (false, 0.0f32);
        let mut spill = f32::MIN;
        while next < region.len() {
            let c = region[next];
            next += 1;
            river |= on_river[c];
            spill = spill.max(filled.data[c]);
            deepest = deepest.max(filled.data[c] - routed.data[c]);
            let (i, j) = (c % m, c / m);
            for (di, dj) in NEIGHBOURS {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if ni < 0 || nj < 0 || ni >= m as i32 || nj >= m as i32 {
                    continue;
                }
                let nb = nj as usize * m + ni as usize;
                if !seen[nb] && hollow(nb) {
                    seen[nb] = true;
                    region.push(nb);
                }
            }
        }
        if river && deepest > lake_depth && (sea == NO_WATER || spill > sea + 0.5) {
            for c in region {
                lake[c] = spill;
            }
        }
    }
    let step = (0.5 * k).clamp(0.75, 2.0);
    let bank_of = |hw: f32| (p.banks.max(0.0) * 2.0 * hw).max(1.0);
    let mut lines: Vec<Vec<Point>> = vec![Vec::new(); stems.len()];
    // Where along its parent each river ends: the index of the point.
    for (id, stem) in stems.iter().enumerate() {
        if !kept[id] {
            continue;
        }
        if id % 64 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        // Source first.
        let mut pts: Vec<Point> = stem
            .cells
            .iter()
            .rev()
            .map(|&c| {
                let (x, y) = at(c);
                Point {
                    x,
                    y,
                    area: area[c as usize],
                    ..Point::default()
                }
            })
            .collect();
        let own_area = pts.last().map_or(1.0, |p| p.area);
        // Its end: where it meets its parent, or the sea.
        let end = match stem.parent {
            Some((parent, idx)) => {
                // The parent's cells are mouth first; its line is source
                // first and denser: the nearest point to that cell.
                let (cx, cy) = at(stems[parent].cells[idx]);
                lines[parent].iter().min_by(|a, b| (a.x - cx).hypot(a.y - cy).total_cmp(&(b.x - cx).hypot(b.y - cy))).copied()
            }
            None => stem.mouth.map(|c| {
                let (x, y) = at(c);
                Point { x, y, ..Point::default() }
            }),
        };
        if let Some(q) = end {
            pts.push(Point {
                x: q.x,
                y: q.y,
                area: own_area,
                ..Point::default()
            });
        }
        smooth(&mut pts, 2, 2);
        // Into the bottom of the valley, across the river's way, on the
        // relief itself.
        let reach = k.max(1.5);
        let len = pts.len();
        let mut offsets = vec![0.0f32; len];
        for i in 1..len.saturating_sub(1) {
            let (a, b) = (pts[i - 1], pts[i + 1]);
            let (tx, ty) = (b.x - a.x, b.y - a.y);
            let tl = tx.hypot(ty).max(1e-6);
            let (nx, ny) = (-ty / tl, tx / tl);
            let (mut best, mut lowest) = (0.0, f32::MAX);
            let mut o = -reach;
            while o <= reach {
                let v = f.sample(pts[i].x + nx * o, pts[i].y + ny * o) + 0.02 * o.abs();
                if v < lowest {
                    lowest = v;
                    best = o;
                }
                o += 0.5;
            }
            offsets[i] = best;
        }
        let mut shifted: Vec<Point> = pts.iter().enumerate().map(|(i, q)| Point { hw: offsets[i], ..*q }).collect();
        smooth_value(&mut shifted, 3, |q| q.hw, |q, v| q.hw = v);
        for i in 1..len.saturating_sub(1) {
            let (a, b) = (pts[i - 1], pts[i + 1]);
            let (tx, ty) = (b.x - a.x, b.y - a.y);
            let tl = tx.hypot(ty).max(1e-6);
            pts[i].x += -ty / tl * shifted[i].hw;
            pts[i].y += tx / tl * shifted[i].hw;
        }
        smooth(&mut pts, 1, 1);
        let mut line = resample(&pts, step);
        // Wider and deeper as water joins it.
        for q in &mut line {
            let share = (q.area / q_max).clamp(0.0, 1.0);
            q.hw = (0.5 * (p.width * share.sqrt()).max(2.0) / spacing).max(0.55);
            q.depth = (p.depth * share.powf(0.4)).max(1.0);
        }
        smooth_value(&mut line, 6, |q| q.hw, |q, v| q.hw = v);
        smooth_value(&mut line, 6, |q| q.depth, |q, v| q.depth = v);
        // Its water: under the lowest ground around it, a little under its
        // banks, never going up; the lake's level in the hollows it fills.
        let mut level = f32::MAX;
        for q in &mut line {
            // Wide enough to take in every sample next to its water.
            let r = (q.hw + 2.0).min(48.0);
            let (ci, cj) = (q.x.round() as i32, q.y.round() as i32);
            let ri = r.ceil() as i32;
            let mut lowest = f32::MAX;
            for dj in -ri..=ri {
                for di in -ri..=ri {
                    if ((di * di + dj * dj) as f32) > r * r {
                        continue;
                    }
                    let (i, j) = ((ci + di).clamp(0, n as i32 - 1) as usize, (cj + dj).clamp(0, n as i32 - 1) as usize);
                    lowest = lowest.min(f.data[j * n + i]);
                }
            }
            let (u, v) = coarse_at(q.x, q.y);
            let cell = (v.round().clamp(0.0, (m - 1) as f32) as usize) * m + u.round().clamp(0.0, (m - 1) as f32) as usize;
            let freeboard = (0.25 * q.depth).clamp(0.5, 1.5);
            let raw = if lake[cell] != NO_WATER { lake[cell] } else { lowest - freeboard };
            level = level.min(raw);
            if sea != NO_WATER {
                level = level.max(sea);
            }
            q.level = level;
        }
        lines[id] = line;
    }

    // ---- digging, at every sample
    let mut segments: Vec<Segment> = Vec::new();
    for line in &lines {
        for w in line.windows(2) {
            let bank = bank_of(w[0].hw.max(w[1].hw));
            segments.push(Segment { a: w[0], b: w[1], bank });
        }
    }
    const B: usize = 32;
    let nb = n.div_ceil(B);
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); nb * nb];
    for (s, seg) in segments.iter().enumerate() {
        let r = seg.a.hw.max(seg.b.hw) + seg.bank + 1.0;
        let (x0, x1) = (seg.a.x.min(seg.b.x) - r, seg.a.x.max(seg.b.x) + r);
        let (y0, y1) = (seg.a.y.min(seg.b.y) - r, seg.a.y.max(seg.b.y) + r);
        let clampb = |v: f32| ((v.max(0.0) as usize) / B).min(nb - 1);
        for bj in clampb(y0)..=clampb(y1) {
            for bi in clampb(x0)..=clampb(x1) {
                buckets[bj * nb + bi].push(s as u32);
            }
        }
    }
    let mut dug = f.data.clone();
    let mut mask = vec![0.0f32; n * n];
    let mut water = vec![NO_WATER; n * n];
    dug.par_chunks_mut(n)
        .zip(mask.par_chunks_mut(n))
        .zip(water.par_chunks_mut(n))
        .enumerate()
        .for_each(|(j, ((drow, mrow), wrow))| {
            let py = j as f32;
            for bi in 0..nb {
                let bucket = &buckets[(j / B) * nb + bi];
                if bucket.is_empty() {
                    continue;
                }
                for i in bi * B..((bi + 1) * B).min(n) {
                    let px = i as f32;
                    let ground = f.data[j * n + i];
                    // The deepest bed of the rivers it is in, the water of
                    // the one it is most in; else the bank of the nearest.
                    let (mut bed, mut inside, mut wl, mut mk) = (f32::MAX, f32::MAX, NO_WATER, 0.0f32);
                    let mut bank: Option<(f32, f32)> = None;
                    for &s in bucket {
                        let seg = &segments[s as usize];
                        let (dx, dy) = (seg.b.x - seg.a.x, seg.b.y - seg.a.y);
                        let len2 = dx * dx + dy * dy;
                        let u = if len2 > 0.0 { (((px - seg.a.x) * dx + (py - seg.a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                        let dist = (px - seg.a.x - dx * u).hypot(py - seg.a.y - dy * u);
                        let lerp = |a: f32, b: f32| a + (b - a) * u;
                        let hw = lerp(seg.a.hw, seg.b.hw);
                        if dist >= hw + seg.bank {
                            continue;
                        }
                        let level = lerp(seg.a.level, seg.b.level);
                        if dist < hw {
                            // A bed with a flat bottom, steeper at its sides.
                            let t = dist / hw;
                            bed = bed.min(level - lerp(seg.a.depth, seg.b.depth) * (1.0 - t.powf(2.5)));
                            mk = 1.0;
                            if t < inside {
                                inside = t;
                                wl = level;
                            }
                        } else {
                            let s = (dist - hw) / seg.bank;
                            if bank.is_none_or(|b| s < b.0) {
                                bank = Some((s, level));
                            }
                            mk = mk.max(1.0 - smoothstep(0.0, 1.0, 2.0 * s));
                        }
                    }
                    let h = if bed < f32::MAX {
                        ground.min(bed)
                    } else if let Some((s, level)) = bank {
                        // Banks sloping down to a little over the water.
                        let edge = level + 0.3;
                        if ground > edge { edge + (ground - edge) * smoothstep(0.0, 1.0, s) } else { ground }
                    } else {
                        ground
                    };
                    drow[i] = h;
                    mrow[i] = mk;
                    if wl > h + 0.25 && (sea == NO_WATER || wl > sea + 0.2) {
                        wrow[i] = wl;
                    }
                }
            }
        });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    // ---- lakes, at every sample
    if lake.iter().any(|&l| l != NO_WATER) {
        // A cell more around each lake, where the relief at the block may
        // still lie under its level.
        let grown: Vec<f32> = (0..m * m)
            .into_par_iter()
            .map(|c| {
                if lake[c] != NO_WATER {
                    return lake[c];
                }
                let (i, j) = (c % m, c / m);
                let mut l = NO_WATER;
                for (di, dj) in NEIGHBOURS {
                    let (ni, nj) = (i as i32 + di, j as i32 + dj);
                    if ni >= 0 && nj >= 0 && ni < m as i32 && nj < m as i32 {
                        l = l.max(lake[nj as usize * m + ni as usize]);
                    }
                }
                l
            })
            .collect();
        water.par_chunks_mut(n).zip(dug.par_chunks(n)).enumerate().for_each(|(j, (wrow, drow))| {
            let cj = (((j as f32 + 0.5) / k) as usize).min(m - 1);
            for i in 0..n {
                let ci = (((i as f32 + 0.5) / k) as usize).min(m - 1);
                let l = grown[cj * m + ci];
                if l != NO_WATER && l > drow[i] + 0.25 {
                    wrow[i] = wrow[i].max(l);
                }
            }
        });
    }
    Some(RiverMaps {
        dug: Field::new(n, dug),
        mask: Field::new(n, mask),
        water: Field::new(n, water),
        count: kept.iter().filter(|&&k| k).count(),
    })
}
