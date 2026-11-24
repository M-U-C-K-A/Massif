//! Stage 3, the walls: a rampart round the village, three blocks inside the
//! area's edge, towers at its corners and every so many blocks, gates
//! where the roads cross it.

use super::roads::{Cells, RoadKind, Roads};
use super::{Area, dist2};
use serde::{Deserialize, Serialize};

/// A village's ramparts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Walls {
    /// The wall's line, block by block, closed (the last joins the first).
    pub line: Vec<[i32; 2]>,
    /// Where it turns: a tower at each.
    pub corners: Vec<[i32; 2]>,
    /// The columns of the line where roads cross it, one for each gate.
    pub gates: Vec<[i32; 2]>,
}

impl Walls {
    /// The columns nothing else may take: the line and two blocks either
    /// side.
    pub fn reserved(&self) -> Cells {
        let mut out = Cells::new();
        for &[x, z] in &self.line {
            for dz in -2..=2 {
                for dx in -2..=2 {
                    out.insert((x + dx, z + dz));
                }
            }
        }
        out
    }

    /// Whether column `(x, z)` is in a gate (a road passes there).
    pub fn in_gate(&self, x: i32, z: i32) -> bool {
        self.gates.iter().any(|g| (g[0] - x).abs() <= 2 && (g[1] - z).abs() <= 2)
    }

    /// The towers: the corners, and one every `every` blocks along the
    /// line away from them and from the gates.
    pub fn towers(&self, every: i32) -> Vec<[i32; 2]> {
        let mut out = self.corners.clone();
        let every = every.max(8) as usize;
        for (k, c) in self.line.iter().enumerate() {
            if k > 0 && k % every == 0 && !out.iter().any(|t| dist2(*t, *c) < 64) && !self.in_gate(c[0], c[1]) {
                out.push(*c);
            }
        }
        out.retain(|t| !self.gates.iter().any(|g| dist2(*g, *t) < 16));
        out
    }
}

/// The convex hull of `points`, counterclockwise.
pub fn hull(points: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut p: Vec<[i32; 2]> = points.to_vec();
    p.sort();
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: [i32; 2], a: [i32; 2], b: [i32; 2]| (a[0] - o[0]) as i64 * (b[1] - o[1]) as i64 - (a[1] - o[1]) as i64 * (b[0] - o[0]) as i64;
    let mut lower: Vec<[i32; 2]> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<[i32; 2]> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Stage 3: the walls' line round the area (its hull three blocks in),
/// and a gate wherever a road crosses it.
pub fn walls(area: &Area, roads: &Roads) -> Walls {
    let samples = area.samples(8);
    let mean = samples.iter().fold((0i64, 0i64), |a, p| (a.0 + p[0] as i64, a.1 + p[1] as i64));
    let n = samples.len().max(1) as i64;
    let c = [(mean.0 / n) as i32, (mean.1 / n) as i32];
    let inset: Vec<[i32; 2]> = hull(&samples)
        .iter()
        .map(|&[x, z]| {
            let (dx, dz) = ((c[0] - x) as f32, (c[1] - z) as f32);
            let d = (dx * dx + dz * dz).sqrt().max(1.0);
            [(x as f32 + dx / d * 3.0).round() as i32, (z as f32 + dz / d * 3.0).round() as i32]
        })
        .collect();
    let mut line: Vec<[i32; 2]> = Vec::new();
    for k in 0..inset.len() {
        let (p, q) = (inset[k], inset[(k + 1) % inset.len()]);
        let n = (q[0] - p[0]).abs().max((q[1] - p[1]).abs()).max(1);
        for s in 0..n {
            let t = s as f32 / n as f32;
            let cell = [(p[0] as f32 + (q[0] - p[0]) as f32 * t).round() as i32, (p[1] as f32 + (q[1] - p[1]) as f32 * t).round() as i32];
            if line.last() != Some(&cell) {
                line.push(cell);
            }
        }
    }
    let cells = roads.cells(area);
    let mut gates: Vec<[i32; 2]> = Vec::new();
    for &[x, z] in &line {
        let on_road = (-1..=1).any(|d| cells.get(&(x + d, z)).is_some_and(|k| *k != RoadKind::Square) || cells.get(&(x, z + d)).is_some_and(|k| *k != RoadKind::Square));
        if on_road && gates.iter().all(|g| (g[0] - x).abs() + (g[1] - z).abs() > 6) {
            gates.push([x, z]);
        }
    }
    Walls { line, corners: inset, gates }
}
