//! Stage 3, the walls: a rampart round the village, three blocks inside the
//! area's edge, towers at its corners and every so many blocks, gates
//! where the roads cross it.

use super::roads::{Cells, RoadKind, Roads};
use super::site::Site;
use super::{Area, dist2};
use serde::{Deserialize, Serialize};

/// What runs round a village: a fence, a hedge, a low wall… or the
/// ramparts with towers of the theme. Each made of the game's own blocks,
/// with gates where its roads cross and a little variety along its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Enclosure {
    /// An oak fence, a post of oak now and then, some with a lantern.
    #[default]
    WoodenFence,
    /// A spruce fence on spruce posts.
    SpruceFence,
    /// Rails of birch between stripped posts, a gap under them.
    PostAndRail,
    /// A hedge of oak leaves, two blocks high, an arch over its gates.
    Hedge,
    /// A hedge of azalea, flowering here and there.
    FloweringHedge,
    /// A cobblestone wall a block high, mossy in places.
    LowStoneWall,
    /// Stones piled one or two high: cobblestone, mossy, stone, andesite.
    DryStone,
    /// A course of cobblestone with a fence on it.
    StoneAndFence,
    /// Spruce logs three or four high, a beam over its gates.
    Palisade,
    /// The theme's ramparts: battlements, towers, gatehouses.
    Ramparts,
}

impl Enclosure {
    pub const ALL: [Enclosure; 10] = [
        Enclosure::WoodenFence,
        Enclosure::SpruceFence,
        Enclosure::PostAndRail,
        Enclosure::Hedge,
        Enclosure::FloweringHedge,
        Enclosure::LowStoneWall,
        Enclosure::DryStone,
        Enclosure::StoneAndFence,
        Enclosure::Palisade,
        Enclosure::Ramparts,
    ];

    /// What villages placed before there was a choice had.
    pub fn before() -> Enclosure {
        Enclosure::Ramparts
    }

    /// Its name, in English (the app translates it).
    pub fn name(self) -> &'static str {
        match self {
            Enclosure::WoodenFence => "Wooden fence",
            Enclosure::SpruceFence => "Spruce fence",
            Enclosure::PostAndRail => "Post and rail",
            Enclosure::Hedge => "Hedge",
            Enclosure::FloweringHedge => "Flowering hedge",
            Enclosure::LowStoneWall => "Low stone wall",
            Enclosure::DryStone => "Dry-stone wall",
            Enclosure::StoneAndFence => "Stone and fence",
            Enclosure::Palisade => "Palisade",
            Enclosure::Ramparts => "Ramparts",
        }
    }

    /// Its key in a script or on the command line (`wooden-fence`).
    pub fn key(self) -> String {
        self.name().to_lowercase().replace(' ', "-")
    }

    /// The one of key or name `s`.
    pub fn parse(s: &str) -> Option<Enclosure> {
        let s = s.trim().to_lowercase().replace(['_', ' '], "-");
        Enclosure::ALL.into_iter().find(|e| e.key() == s)
    }

    /// The blocks of one column of its run, from the ground up (`""`: left
    /// as it is): column `k` of the run, `r` a random number of its own;
    /// `gate`, a road passes there; `edge`, beside a gate.
    pub fn column(self, k: usize, r: u32, gate: bool, edge: bool) -> Vec<&'static str> {
        let chance = |n: u32| r % 100 < n;
        let post = |every: usize| k.is_multiple_of(every) || edge;
        let lantern = |p: u32| if edge || chance(p) { "minecraft:lantern" } else { "" };
        match self {
            Enclosure::WoodenFence => match (gate, post(7 + (r % 3) as usize)) {
                (true, _) => vec!["minecraft:oak_fence_gate"],
                (false, true) => vec!["minecraft:oak_log", lantern(30)],
                _ => vec!["minecraft:oak_fence"],
            },
            Enclosure::SpruceFence => match (gate, post(6)) {
                (true, _) => vec!["minecraft:spruce_fence_gate"],
                (false, true) => vec!["minecraft:spruce_log", if edge || chance(40) { "minecraft:spruce_fence" } else { "" }, lantern(0)],
                _ => vec!["minecraft:spruce_fence"],
            },
            Enclosure::PostAndRail => match (gate, post(4)) {
                (true, _) => vec!["", "minecraft:birch_fence_gate"],
                (false, true) => vec!["minecraft:stripped_birch_log", "minecraft:stripped_birch_log", if edge { "minecraft:lantern" } else { "" }],
                _ => vec!["", "minecraft:birch_fence"],
            },
            Enclosure::Hedge | Enclosure::FloweringHedge => {
                let leaf = || match self {
                    Enclosure::Hedge => {
                        if chance(12) {
                            "minecraft:azalea_leaves"
                        } else {
                            "minecraft:oak_leaves"
                        }
                    }
                    _ => {
                        if chance(35) {
                            "minecraft:flowering_azalea_leaves"
                        } else {
                            "minecraft:azalea_leaves"
                        }
                    }
                };
                if gate {
                    // An arch of leaves over the road.
                    vec!["", "", "", leaf()]
                } else if edge {
                    vec![leaf(), leaf(), leaf(), leaf()]
                } else if chance(25) {
                    vec![leaf(), leaf(), leaf()]
                } else {
                    vec![leaf(), leaf()]
                }
            }
            Enclosure::LowStoneWall => {
                if gate {
                    vec![]
                } else if post(6) {
                    vec!["minecraft:cobblestone", lantern(35)]
                } else if chance(30) {
                    vec!["minecraft:mossy_cobblestone_wall"]
                } else {
                    vec!["minecraft:cobblestone_wall"]
                }
            }
            Enclosure::DryStone => {
                const STONES: [&str; 5] = ["minecraft:cobblestone", "minecraft:cobblestone", "minecraft:mossy_cobblestone", "minecraft:stone", "minecraft:andesite"];
                let stone = |n: u32| STONES[((r >> n) % STONES.len() as u32) as usize];
                if gate {
                    vec![]
                } else if edge || chance(40) {
                    vec![stone(0), stone(8)]
                } else {
                    vec![stone(0)]
                }
            }
            Enclosure::StoneAndFence => {
                let base = if chance(25) { "minecraft:mossy_cobblestone" } else { "minecraft:cobblestone" };
                match (gate, post(6)) {
                    (true, _) => vec!["", "minecraft:oak_fence_gate"],
                    (false, true) => vec![base, "minecraft:cobblestone", lantern(40)],
                    _ => vec![base, "minecraft:oak_fence"],
                }
            }
            Enclosure::Palisade => {
                let log = if chance(20) { "minecraft:stripped_spruce_log" } else { "minecraft:spruce_log" };
                if gate {
                    // A beam over the road.
                    vec!["", "", "", "", "minecraft:spruce_log"]
                } else if edge {
                    vec![log, log, log, log, log, "minecraft:lantern"]
                } else if chance(50) {
                    vec![log, log, log, log]
                } else {
                    vec![log, log, log]
                }
            }
            Enclosure::Ramparts => {
                // The look of them, for a picture: the theme's walls lay
                // them.
                if gate { vec!["", "", "", "", "minecraft:stone_bricks"] } else if k.is_multiple_of(2) { vec!["minecraft:stone_bricks"; 5] } else { vec!["minecraft:stone_bricks"; 4] }
            }
        }
    }

    /// A straight run of `len` columns with a gate of three in the middle,
    /// as [`Enclosure::column`] gives them: for a picture of it.
    pub fn sample(self, len: usize) -> Vec<Vec<&'static str>> {
        let mid = len / 2;
        let mut rng = crate::rng::Rng::new(0x656e_636c ^ self as u64);
        (0..len)
            .map(|k| {
                let gate = (mid - 1..=mid + 1).contains(&k);
                let edge = k + 2 == mid || k == mid + 2;
                self.column(k, rng.next_u64() as u32, gate, edge)
            })
            .collect()
    }
}

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

    /// Whether column `(x, z)` lies within the walls (their line, closed).
    pub fn contains(&self, x: i32, z: i32) -> bool {
        let l = &self.line;
        if l.len() < 3 {
            return true;
        }
        let (px, pz) = (x as f64 + 0.5, z as f64 + 0.5);
        let mut inside = false;
        let mut j = l.len() - 1;
        for i in 0..l.len() {
            let (xi, zi, xj, zj) = (l[i][0] as f64, l[i][1] as f64, l[j][0] as f64, l[j][1] as f64);
            if (zi > pz) != (zj > pz) && px < (xj - xi) * (pz - zi) / (zj - zi) + xi {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// The plots within the walls: those outside would stand beyond them.
    pub fn plots_within(&self, plots: &[super::plots::Plot]) -> Vec<super::plots::Plot> {
        plots.iter().filter(|p| self.contains(p.door[0], p.door[1])).cloned().collect()
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

/// The roads cut where the walls are: side roads stop short of them,
/// main roads pass through their gates.
pub fn trim(roads: &mut Roads, walls: &Walls) {
    let reserved = walls.reserved();
    for r in roads.lines.iter_mut().filter(|r| r.kind == RoadKind::Side) {
        let reach = r.width / 2 + 1;
        if let Some(k) = r.points.iter().position(|p| (-reach..=reach).any(|d| reserved.contains(&(p[0] + d, p[1])) || reserved.contains(&(p[0], p[1] + d)))) {
            r.points.truncate(k);
        }
    }
    roads.lines.retain(|r| r.kind != RoadKind::Side || r.points.len() >= 6);
}

/// Stage 3: the walls' line round the village — round the area (its hull
/// three blocks in), or, when `site` says how far the village reaches,
/// round that reach of each square within the area — and a gate wherever
/// a main road crosses it.
pub fn walls(area: &Area, roads: &Roads, site: Option<&Site>) -> Walls {
    let samples: Vec<[i32; 2]> = match site {
        Some(s) => {
            let r = s.reach as f32;
            let ring: Vec<[i32; 2]> = s.centres.iter().flat_map(|c| (0..32).map(move |k| {
                let a = k as f32 / 32.0 * std::f32::consts::TAU;
                [c[0] + (a.cos() * r) as i32, c[1] + (a.sin() * r) as i32]
            })).filter(|p| area.contains(p[0], p[1])).collect();
            if ring.len() >= 3 { ring.into_iter().chain(s.centres.iter().copied()).collect() } else { area.samples(8) }
        }
        None => area.samples(8),
    };
    around(area, roads, &samples)
}

/// Walls round a village already built: round its buildings' boxes, eight
/// blocks out, within the area.
pub fn round_buildings(area: &Area, roads: &Roads, boxes: &[([i32; 2], [i32; 2])]) -> Option<Walls> {
    let pts: Vec<[i32; 2]> = boxes.iter().flat_map(|(lo, hi)| [[lo[0] - 8, lo[1] - 8], [hi[0] + 8, lo[1] - 8], [lo[0] - 8, hi[1] + 8], [hi[0] + 8, hi[1] + 8]]).filter(|p| area.contains(p[0], p[1])).collect();
    (pts.len() >= 3).then(|| around(area, roads, &pts))
}

/// Walls three blocks inside the hull of `samples`, gates where main roads
/// cross.
fn around(area: &Area, roads: &Roads, samples: &[[i32; 2]]) -> Walls {
    let mean = samples.iter().fold((0i64, 0i64), |a, p| (a.0 + p[0] as i64, a.1 + p[1] as i64));
    let n = samples.len().max(1) as i64;
    let c = [(mean.0 / n) as i32, (mean.1 / n) as i32];
    let inset: Vec<[i32; 2]> = hull(samples)
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
    // A gate where a main road crosses: on the line, and on both sides of
    // it (a road beside the wall is no way through).
    let cells = roads.cells(area);
    let main = |x: i32, z: i32| (-1..=1).any(|d| cells.get(&(x + d, z)) == Some(&RoadKind::Main) || cells.get(&(x, z + d)) == Some(&RoadKind::Main));
    let mut gates: Vec<[i32; 2]> = Vec::new();
    for &[x, z] in &line {
        if !main(x, z) {
            continue;
        }
        let (dx, dz) = ((c[0] - x) as f32, (c[1] - z) as f32);
        let d = (dx * dx + dz * dz).sqrt().max(1.0);
        let (ux, uz) = (dx / d, dz / d);
        let at = |k: f32| ((x as f32 + ux * k).round() as i32, (z as f32 + uz * k).round() as i32);
        let inward = (2..=4).any(|k| {
            let (px, pz) = at(k as f32);
            main(px, pz)
        });
        let outward = (2..=4).any(|k| {
            let (px, pz) = at(-(k as f32));
            main(px, pz) || !area.contains(px, pz)
        });
        if inward && outward && gates.iter().all(|g| (g[0] - x).abs() + (g[1] - z).abs() > 6) {
            gates.push([x, z]);
        }
    }
    Walls { line, corners: inset, gates }
}
