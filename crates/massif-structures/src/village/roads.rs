//! Stage 2, the roads: main roads that follow the land and join the
//! squares, two ways out of the village, side roads off them.

use super::site::Site;
use super::{Area, Params, Theme, dist2, height};
use crate::ground::Ground;
use crate::rng::Rng;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Columns, as a set.
pub type Cells = HashSet<(i32, i32)>;

/// What a road is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoadKind {
    /// A side road.
    Side,
    /// A main road: between squares, and out of the village.
    Main,
    /// A square, round its middle.
    Square,
}

/// A road: its middle line block by block, its kind and width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Road {
    pub points: Vec<[i32; 2]>,
    pub kind: RoadKind,
    pub width: i32,
}

/// A village's roads.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Roads {
    pub lines: Vec<Road>,
    /// Streets that are not lines: those of the game's villages.
    pub extra: Vec<(i32, i32)>,
}

impl Roads {
    /// Every column of road, with its kind (the main one where two meet),
    /// within the area.
    pub fn cells(&self, area: &Area) -> HashMap<(i32, i32), RoadKind> {
        let mut out: HashMap<(i32, i32), RoadKind> = HashMap::new();
        for c in &self.extra {
            out.entry(*c).or_insert(RoadKind::Side);
        }
        for r in &self.lines {
            let (lo, hi) = (-(r.width - 1) / 2, r.width / 2);
            for &[x, z] in &r.points {
                for dz in lo..=hi {
                    for dx in lo..=hi {
                        let c = (x + dx, z + dz);
                        if area.contains(c.0, c.1) {
                            let e = out.entry(c).or_insert(r.kind);
                            *e = (*e).max(r.kind);
                        }
                    }
                }
            }
        }
        out
    }
}

/// A way across the land between two columns, on a grid of `step`
/// blocks: gentle slopes and dry land preferred, never out of the area, nor
/// through `blocked` cells. Its columns block by block, or none.
pub fn route(ground: &dyn Ground, area: &Area, blocked: &Cells, from: [i32; 2], to: [i32; 2], step: i32) -> Vec<[i32; 2]> {
    let snap = |p: [i32; 2]| (p[0].div_euclid(step), p[1].div_euclid(step));
    let (a, b) = (snap(from), snap(to));
    let centre = |c: (i32, i32)| (c.0 * step + step / 2, c.1 * step + step / 2);
    let mut cost: HashMap<(i32, i32), i64> = HashMap::new();
    let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    cost.insert(a, 0);
    heap.push(Reverse((0i64, a)));
    let span = ((a.0 - b.0).abs() + (a.1 - b.1).abs()) as usize;
    let mut visited = 0usize;
    while let Some(Reverse((_, c))) = heap.pop() {
        if c == b {
            break;
        }
        visited += 1;
        if visited > 40_000 + span * 400 {
            return Vec::new();
        }
        let here = cost[&c];
        let (cx, cz) = centre(c);
        let (h0, _) = height(ground, cx, cz);
        for d in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let n = (c.0 + d.0, c.1 + d.1);
            let (nx, nz) = centre(n);
            if n != b && (!area.contains(nx, nz) || blocked.contains(&(nx, nz))) {
                continue;
            }
            let (h1, wet) = height(ground, nx, nz);
            let run = if d.0 != 0 && d.1 != 0 { 14 } else { 10 };
            let climb = (h1 - h0).abs() as i64;
            let step_cost = run + climb * climb * 6 + if wet { 60 } else { 0 };
            let total = here + step_cost;
            if cost.get(&n).is_none_or(|&k| total < k) {
                cost.insert(n, total);
                came.insert(n, c);
                let guess = (((n.0 - b.0).abs() + (n.1 - b.1).abs()) * 10) as i64;
                heap.push(Reverse((total + guess, n)));
            }
        }
    }
    if !came.contains_key(&b) && a != b {
        return Vec::new();
    }
    let mut cells = vec![b];
    while let Some(&p) = came.get(cells.last().expect("one")) {
        cells.push(p);
    }
    cells.reverse();
    // Block by block between the cells' middles, from `from` to `to`.
    let mut pts: Vec<(i32, i32)> = vec![(from[0], from[1])];
    pts.extend(cells.iter().skip(1).take(cells.len().saturating_sub(2)).map(|&c| centre(c)));
    pts.push((to[0], to[1]));
    let mut out: Vec<[i32; 2]> = Vec::new();
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let n = (q.0 - p.0).abs().max((q.1 - p.1).abs()).max(1);
        for k in 0..n {
            let t = k as f32 / n as f32;
            let c = [(p.0 as f32 + (q.0 - p.0) as f32 * t).round() as i32, (p.1 as f32 + (q.1 - p.1) as f32 * t).round() as i32];
            if out.last() != Some(&c) {
                out.push(c);
            }
        }
    }
    out.push(to);
    out
}

/// Stage 2: main roads joining the squares (each to the nearest already
/// joined) and two ways out; for a pack's village, the squares and side
/// roads off the main ones, longer as `params.spread` grows. `blocked`:
/// what roads keep out of (the game's buildings).
pub fn roads(ground: &dyn Ground, area: &Area, site: &Site, blocked: &Cells, theme: &Theme, params: &Params, seed: u64) -> Roads {
    let mut rng = Rng::new(seed ^ 0x726f_6164);
    let centres = &site.centres;
    let main_w = params.street_width.unwrap_or(theme.main.width).clamp(1, 9);
    let side_w = theme.side.width.min(main_w);
    let spread = params.spread.clamp(0.0, 1.0);
    let mut lines: Vec<Road> = Vec::new();
    let mut joined = vec![centres[0]];
    for &c in centres.iter().skip(1) {
        let near = *joined.iter().min_by_key(|j| dist2(**j, c)).expect("one");
        let line = route(ground, area, blocked, c, near, 4);
        if !line.is_empty() {
            lines.push(Road { points: line, kind: RoadKind::Main, width: main_w });
        }
        joined.push(c);
    }
    // Two ways out: along the area's longer side, either way, as far as the
    // village reaches (to the edge when it is sized by the area).
    let long_x = area.rect[2] - area.rect[0] >= area.rect[3] - area.rect[1];
    let reach = if params.houses.is_some() { (site.reach as f32 * (1.2 + spread)) as i32 } else { i32::MAX };
    for k in 0..2 {
        let s = if k == 0 { 1 } else { -1 };
        let dir = if long_x { [s, 0] } else { [0, s] };
        let mut p = centres[0];
        let mut gone = 0;
        while area.contains(p[0] + dir[0] * 8, p[1] + dir[1] * 8) && gone < reach {
            p = [p[0] + dir[0] * 8, p[1] + dir[1] * 8];
            gone += 8;
        }
        if gone > 24 {
            let line = route(ground, area, blocked, centres[0], p, 4);
            if !line.is_empty() {
                lines.push(Road { points: line, kind: RoadKind::Main, width: main_w });
            }
        }
    }
    if theme.vanilla.is_none() {
        // A square round each district's middle.
        for &c in centres {
            lines.push(Road { points: vec![c], kind: RoadKind::Square, width: 9 });
        }
        // Side roads every 26–42 blocks along the main ones, 20–46 long
        // (more and longer as the village spreads), on dry land.
        let mains: Vec<Vec<[i32; 2]>> = lines.iter().filter(|r| r.kind == RoadKind::Main).map(|r| r.points.clone()).collect();
        let (lo, hi) = ((14.0 + 12.0 * spread) as i32, (30.0 + 32.0 * spread) as i32);
        for line in &mains {
            let mut k = rng.between(14, 24) as usize;
            while k < line.len() {
                let p = line[k];
                let q = line[(k + 1).min(line.len() - 1)];
                let along_x = (q[0] - p[0]).abs() >= (q[1] - p[1]).abs();
                for side in [-1, 1] {
                    if rng.chance(0.25) {
                        continue;
                    }
                    let n = if along_x { [0, side] } else { [side, 0] };
                    let len = rng.between(lo, hi);
                    let mut end = p;
                    for s in 1..=len {
                        let c = [p[0] + n[0] * s, p[1] + n[1] * s];
                        if !area.contains(c[0], c[1]) || blocked.contains(&(c[0], c[1])) || height(ground, c[0], c[1]).1 {
                            break;
                        }
                        end = c;
                    }
                    if (end[0] - p[0]).abs() + (end[1] - p[1]).abs() >= 10 {
                        let steps = (end[0] - p[0]).abs().max((end[1] - p[1]).abs());
                        let l = (0..=steps).map(|s| [p[0] + n[0] * s, p[1] + n[1] * s]).collect();
                        lines.push(Road { points: l, kind: RoadKind::Side, width: side_w });
                    }
                }
                k += rng.between(26, 42) as usize;
            }
        }
    }
    Roads { lines, extra: Vec::new() }
}
