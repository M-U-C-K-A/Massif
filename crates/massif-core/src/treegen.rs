//! The tree generator: trees grown from a few settings rather than drawn
//! by hand. A trunk grows up, tapering and flaring at its foot; limbs
//! leave it in a spiral, each bending down under its weight and up towards
//! the sun, and carry smaller branches, up to three levels; leaves gather
//! in clumps where the twigs end. The skeleton is then laid in blocks:
//! thick parts as logs along their axis, thin ones as fences, the flare
//! smoothed with stairs and slabs, and every leaf persistent and within
//! reach of wood. A "survival" tree keeps to logs and leaves, the blocks a
//! tree of the game has. The result is an [`ObjectDef`], as the tree pack's
//! trees are, so object layers, tree sizes and canopies take it as it is.

use crate::objects::ObjectDef;
use std::collections::{HashMap, VecDeque};
use std::f32::consts::PI;

/// What the generator's trees say they were read from.
pub const SOURCE: &str = "Massif tree generator";

/// The shape of the crown: how long the limbs are along the trunk, and how
/// the leaves hang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Crown {
    /// Long limbs below, short ones above: firs and spruces.
    Cone,
    /// Longest in the middle.
    Round,
    /// Limbs reaching out at the top under flat clumps of leaves.
    Umbrella,
    /// Round, with curtains of leaves hanging from it.
    Weeping,
    /// A bare trunk with fronds of leaves at its top.
    Palm,
}

impl Crown {
    pub const ALL: [Crown; 5] = [Crown::Cone, Crown::Round, Crown::Umbrella, Crown::Weeping, Crown::Palm];

    pub const fn name(self) -> &'static str {
        match self {
            Crown::Cone => "Cone",
            Crown::Round => "Round",
            Crown::Umbrella => "Umbrella",
            Crown::Weeping => "Weeping",
            Crown::Palm => "Palm",
        }
    }
}

/// The settings a tree grows from: plain data and a seed, so that anything
/// may grow trees from them (a dialog today; a brush or a node later).
#[derive(Debug, Clone, PartialEq)]
pub struct TreeParams {
    /// Its height in blocks, about (each tree varies a little).
    pub height: f32,
    /// The width of its trunk at the foot, in blocks.
    pub trunk: f32,
    /// How much the trunk widens at its foot, 0 to 1.
    pub flare: f32,
    /// How far up the trunk goes before it ends in its limbs, as a share of
    /// the height: 1 for a trunk to the top.
    pub trunk_share: f32,
    /// Levels of branches, 1 to 4.
    pub levels: u8,
    /// How many limbs leave the trunk.
    pub branches: u8,
    /// Where on the trunk the first limb leaves, as a share of its length.
    pub start: f32,
    /// The angle between a branch and what it grows from, in degrees.
    pub angle: f32,
    /// The length of the longest limbs, as a share of the height.
    pub length: f32,
    /// Each level of branches as long as this share of the one before.
    pub falloff: f32,
    /// How much branches bend down under their weight, 0 to 1.
    pub gravity: f32,
    /// How much branches turn up towards the sun, 0 to 1.
    pub sun: f32,
    /// How crooked the wood grows, 0 to 1.
    pub crook: f32,
    pub crown: Crown,
    /// The radius of a clump of leaves, in blocks.
    pub leaf_size: f32,
    /// How full the clumps are, 0 to 1; 0 for a bare tree.
    pub leaf_density: f32,
    /// The wood, as the game names it: `oak` for `oak_log`, `oak_fence`…
    pub wood: String,
    /// The leaves, as the game names them: `oak` for `oak_leaves`.
    pub leaves: String,
    /// Only the blocks a tree of the game has: logs and leaves.
    pub survival: bool,
    pub seed: u64,
}

/// Kinds of trees to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreePreset {
    Oak,
    Spruce,
    Birch,
    Cherry,
    Palm,
    Willow,
    Acacia,
    Dead,
    GiantSequoia,
    GreatOak,
}

impl TreePreset {
    pub const ALL: [TreePreset; 10] = [TreePreset::Oak, TreePreset::Spruce, TreePreset::Birch, TreePreset::Cherry, TreePreset::Palm, TreePreset::Willow, TreePreset::Acacia, TreePreset::Dead, TreePreset::GiantSequoia, TreePreset::GreatOak];

    pub const fn name(self) -> &'static str {
        match self {
            TreePreset::Oak => "Oak",
            TreePreset::Spruce => "Spruce",
            TreePreset::Birch => "Birch",
            TreePreset::Cherry => "Cherry",
            TreePreset::Palm => "Palm",
            TreePreset::Willow => "Willow",
            TreePreset::Acacia => "Acacia",
            TreePreset::Dead => "Dead tree",
            TreePreset::GiantSequoia => "Giant sequoia",
            TreePreset::GreatOak => "Great oak",
        }
    }

    /// Its settings.
    pub fn params(self) -> TreeParams {
        let p = |height, trunk, flare, trunk_share, levels, branches, start, angle, length, falloff, gravity, sun, crook, crown, leaf_size, leaf_density, wood: &str, leaves: &str| TreeParams {
            height,
            trunk,
            flare,
            trunk_share,
            levels,
            branches,
            start,
            angle,
            length,
            falloff,
            gravity,
            sun,
            crook,
            crown,
            leaf_size,
            leaf_density,
            wood: wood.to_owned(),
            leaves: leaves.to_owned(),
            survival: false,
            seed: 1,
        };
        match self {
            TreePreset::Oak => p(22.0, 2.0, 0.6, 0.6, 3, 6, 0.6, 45.0, 0.5, 0.55, 0.3, 0.35, 0.35, Crown::Round, 2.1, 0.65, "oak", "oak"),
            TreePreset::Spruce => p(26.0, 1.5, 0.35, 1.0, 1, 40, 0.35, 92.0, 0.24, 0.35, 0.05, 0.0, 0.05, Crown::Cone, 1.5, 0.95, "spruce", "spruce"),
            TreePreset::Birch => p(19.0, 1.0, 0.15, 0.85, 2, 6, 0.45, 30.0, 0.22, 0.5, 0.15, 0.45, 0.2, Crown::Round, 2.0, 0.75, "birch", "birch"),
            TreePreset::Cherry => p(14.0, 1.5, 0.4, 0.4, 3, 4, 0.85, 50.0, 0.55, 0.55, 0.3, 0.3, 0.3, Crown::Umbrella, 2.8, 0.8, "cherry", "cherry"),
            TreePreset::Palm => p(15.0, 1.0, 0.25, 1.0, 1, 9, 0.98, 62.0, 0.5, 0.5, 0.9, 0.0, 0.25, Crown::Palm, 1.0, 1.0, "jungle", "jungle"),
            TreePreset::Willow => p(16.0, 2.0, 0.5, 0.5, 2, 7, 0.7, 55.0, 0.5, 0.5, 0.45, 0.25, 0.3, Crown::Weeping, 2.4, 0.75, "oak", "oak"),
            TreePreset::Acacia => p(13.0, 1.0, 0.2, 0.5, 2, 3, 0.75, 42.0, 0.6, 0.45, 0.1, 0.25, 0.4, Crown::Umbrella, 3.0, 0.8, "acacia", "acacia"),
            TreePreset::GiantSequoia => p(100.0, 7.0, 0.7, 1.0, 3, 26, 0.5, 75.0, 0.14, 0.5, 0.25, 0.4, 0.2, Crown::Round, 2.4, 0.65, "mangrove", "spruce"),
            TreePreset::GreatOak => p(60.0, 7.0, 0.9, 0.4, 3, 8, 0.75, 55.0, 0.55, 0.5, 0.3, 0.3, 0.3, Crown::Round, 3.0, 0.6, "dark_oak", "dark_oak"),
            TreePreset::Dead => p(17.0, 1.0, 0.3, 0.7, 2, 6, 0.4, 45.0, 0.5, 0.55, 0.2, 0.3, 0.6, Crown::Round, 0.0, 0.0, "oak", ""),
        }
    }
}

type V = [f32; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V) -> V {
    let l = dot(a, a).sqrt();
    if l < 1e-6 { [0.0, 0.0, 1.0] } else { mul(a, 1.0 / l) }
}

/// A direction at `angle` (radians) from `d`, turned `azimuth` around it.
fn turn(d: V, angle: f32, azimuth: f32) -> V {
    let helper = if d[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let u = norm(cross(d, helper));
    let w = cross(d, u);
    let side = add(mul(u, azimuth.cos()), mul(w, azimuth.sin()));
    norm(add(mul(d, angle.cos()), mul(side, angle.sin())))
}

/// SplitMix64: the same numbers for the same seed, everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// In [0, 1).
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// In [-1, 1).
    fn s(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
}

/// A value in [0, 1) for a block, the same whatever order blocks are made in.
fn hash3(x: i32, y: i32, z: i32, seed: u64) -> f32 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B1) ^ (y as u64).wrapping_mul(0x85EB_CA77).rotate_left(21) ^ (z as u64).wrapping_mul(0xC2B2_AE3D).rotate_left(42);
    h = (h ^ (h >> 33)).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h = (h ^ (h >> 33)).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    ((h ^ (h >> 33)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A piece of wood between two points, as wide as its radii there.
struct Segment {
    a: V,
    b: V,
    ra: f32,
    rb: f32,
}

/// The skeleton of a tree: its wood and where its leaves gather.
#[derive(Default)]
struct Skeleton {
    wood: Vec<Segment>,
    /// Clumps of leaves: their centre and radius.
    clumps: Vec<(V, f32)>,
    /// Fronds: lines of leaves.
    fronds: Vec<Vec<V>>,
}

struct Grower<'a> {
    p: &'a TreeParams,
    rng: Rng,
    sk: Skeleton,
    height: f32,
}

impl Grower<'_> {
    /// Grow a branch of `level` (1 = a limb from the trunk) from `start`
    /// along `dir`, `length` long and `radius` wide where it starts.
    fn branch(&mut self, start: V, dir: V, length: f32, radius: f32, level: u8) {
        let p = self.p;
        if p.crown == Crown::Palm && level == 1 {
            return self.frond(start, dir, length);
        }
        let steps = length.ceil().max(1.0) as usize;
        let step = length / steps as f32;
        let tip = if p.survival { 0.5 } else { 0.25 };
        let (mut pos, mut d) = (start, dir);
        let mut path = Vec::with_capacity(steps + 1);
        path.push((pos, radius));
        for i in 0..steps {
            let t = (i as f32 + 1.0) / steps as f32;
            // Heavier the further out; the sun draws the tips most.
            let bend = [0.0, 0.0, p.sun * 0.16 - p.gravity * 0.14 * (0.25 + t) * (level as f32).sqrt()];
            let wobble = [self.rng.s(), self.rng.s(), self.rng.s() * 0.5];
            d = norm(add(add(d, bend), mul(wobble, p.crook * 0.22)));
            pos = add(pos, mul(d, step));
            let r = (radius * (1.0 - t * 0.8)).max(tip);
            path.push((pos, r));
        }
        for w in path.windows(2) {
            self.sk.wood.push(Segment { a: w[0].0, b: w[1].0, ra: w[0].1, rb: w[1].1 });
        }
        let leafy = p.leaf_density > 0.0 && p.leaf_size > 0.0;
        if level < p.levels {
            // Twigs along its outer part, in a spiral.
            let n = ((length / 2.2).round() as usize).clamp(2, 10);
            let mut azimuth = self.rng.f() * PI * 2.0;
            for k in 0..n {
                let t = 0.3 + 0.7 * (k as f32 + 0.5 + self.rng.s() * 0.3) / n as f32;
                let i = ((t * steps as f32) as usize).min(steps);
                let (at, r) = path[i];
                let along = norm(sub(path[(i + 1).min(steps)].0, path[i.saturating_sub(1)].0));
                azimuth += 2.4 + self.rng.s() * 0.4;
                let angle = (p.angle * 0.9 + self.rng.s() * 10.0).to_radians();
                let child = length * p.falloff * (1.0 - 0.45 * t) * (0.85 + self.rng.f() * 0.3);
                if child >= 1.0 {
                    self.branch(at, turn(along, angle, azimuth), child, r * 0.7, level + 1);
                }
            }
            if leafy && p.crown == Crown::Cone {
                self.leaves_along(&path, 0.4);
            }
        } else if leafy {
            // The outer part of the last twigs bears the leaves.
            self.leaves_along(&path, if p.crown == Crown::Cone { 0.0 } else { 0.7 });
        }
        if leafy {
            let end = path[steps].0;
            let r = p.leaf_size * (0.85 + self.rng.f() * 0.3);
            self.sk.clumps.push((end, r));
        }
    }

    /// Clumps of leaves along the part of `path` beyond `from` (a share of
    /// its length).
    fn leaves_along(&mut self, path: &[(V, f32)], from: f32) {
        let n = path.len() - 1;
        let gap = if self.p.crown == Crown::Cone { 1 } else { (self.p.leaf_size * 2.2).max(1.0) as usize };
        let mut i = ((n as f32 * from) as usize).max(1);
        while i < n {
            let r = self.p.leaf_size * (0.7 + self.rng.f() * 0.3);
            self.sk.clumps.push((path[i].0, r));
            i += gap;
        }
    }

    /// A palm frond: a drooping line of leaves.
    fn frond(&mut self, start: V, dir: V, length: f32) {
        let p = self.p;
        let steps = length.ceil().clamp(2.0, 8.0) as usize;
        // Two blocks of wood at its root, so that the leaves reach further.
        let stub = add(start, mul(dir, 1.5));
        self.sk.wood.push(Segment { a: start, b: stub, ra: 0.5, rb: 0.5 });
        let (mut pos, mut d) = (stub, dir);
        let mut line = vec![pos];
        for i in 0..steps {
            let t = (i as f32 + 1.0) / steps as f32;
            d = norm(add(d, [self.rng.s() * 0.05, self.rng.s() * 0.05, -p.gravity * 0.55 * t]));
            pos = add(pos, d);
            line.push(pos);
        }
        self.sk.fronds.push(line);
    }

    fn grow(mut self) -> Skeleton {
        let p = self.p;
        let h = self.height;
        let r0 = (p.trunk * 0.5).max(0.5);
        let centre = if (p.trunk.round() as i32).max(1) % 2 == 0 { 0.0 } else { 0.5 };
        // The trunk, leaning and curving a little (a palm's much more).
        let length = (h * p.trunk_share.clamp(0.1, 1.0)).max(2.0);
        let steps = length.ceil() as usize;
        let lean_angle = self.rng.f() * PI * 2.0;
        let lean = [lean_angle.cos(), lean_angle.sin(), 0.0];
        let curve = match p.crown {
            Crown::Palm => 0.02 + p.crook * 0.04,
            // A conifer's leader grows straight.
            Crown::Cone => 0.0,
            _ => p.crook * 0.015,
        };
        let sway = if p.crown == Crown::Cone { 0.0 } else { p.crook * 0.03 };
        let flare_height = 2.0 + r0 * 1.5;
        let (mut pos, mut d) = ([centre, centre, 0.0], [0.0, 0.0, 1.0]);
        let mut trunk = vec![(pos, r0 * (1.0 + p.flare * 1.3))];
        let top_r = if p.trunk_share >= 0.99 { 0.5 } else { r0 * 0.7 };
        for i in 0..steps {
            let t = (i as f32 + 1.0) / steps as f32;
            d = norm(add(add(d, mul(lean, curve)), [self.rng.s() * sway, self.rng.s() * sway, 0.0]));
            pos = add(pos, mul(d, length / steps as f32));
            let z = pos[2];
            let flare = if z < flare_height { p.flare * 1.3 * (1.0 - z / flare_height).powi(2) } else { 0.0 };
            let r = (r0 + (top_r - r0) * t) * (1.0 + flare);
            trunk.push((pos, r.max(0.5)));
        }
        for w in trunk.windows(2) {
            self.sk.wood.push(Segment { a: w[0].0, b: w[1].0, ra: w[0].1, rb: w[1].1 });
        }
        // Roots spreading at the foot of a flared trunk.
        if p.flare > 0.3 && r0 >= 0.75 {
            let n = 3 + (self.rng.f() * 3.0) as usize + (r0 as usize).saturating_sub(1);
            let a0 = self.rng.f() * PI * 2.0;
            for k in 0..n {
                let a = a0 + k as f32 * PI * 2.0 / n as f32 + self.rng.s() * 0.4;
                let out = [a.cos(), a.sin(), 0.0];
                let from = [centre, centre, 1.0 + r0 * 0.6];
                let to = add([centre, centre, 0.0], mul(out, r0 * (1.6 + p.flare) + 1.0 + self.rng.f()));
                self.sk.wood.push(Segment { a: from, b: to, ra: r0 * 0.55, rb: 0.5 });
            }
        }
        // The limbs, in a spiral up the trunk.
        // A conifer's limbs in whorls of four, the others one by one.
        let n = p.branches.max(1) as usize;
        let whorl = if p.crown == Crown::Cone { 4 } else { 1 };
        let whorls = n.div_ceil(whorl);
        let mut base = self.rng.f() * PI * 2.0;
        for k in 0..n {
            let (wk, j) = (k / whorl, k % whorl);
            let u = if whorls == 1 { 1.0 } else { wk as f32 / (whorls - 1) as f32 };
            let t = p.start + (1.0 - p.start) * u;
            let t = (t + self.rng.s() * 0.03 * (1.0 - p.start)).clamp(0.0, 1.0);
            let i = ((t * steps as f32).round() as usize).min(steps);
            let (at, r) = trunk[i];
            let along = norm(sub(trunk[(i + 1).min(steps)].0, trunk[i.saturating_sub(1)].0));
            // Fronds spread around the sky, however the trunk leans.
            let along = if p.crown == Crown::Palm { norm(add(along, [0.0, 0.0, 2.0])) } else { along };
            let azimuth = if whorl > 1 {
                if j == 0 {
                    base += 2.4;
                }
                base + j as f32 * PI * 2.0 / whorl as f32 + self.rng.s() * 0.3
            } else {
                base += 2.4 + self.rng.s() * 0.35;
                base
            };
            let shape = match p.crown {
                Crown::Cone => 0.1 + 0.9 * (1.0 - u),
                Crown::Round | Crown::Weeping => 0.55 + 0.45 * (PI * (0.25 + 0.6 * u)).sin(),
                Crown::Umbrella => 0.75 + 0.25 * u,
                Crown::Palm => 0.85 + self.rng.f() * 0.3,
            };
            let length = h * p.length * shape * (0.85 + self.rng.f() * 0.3);
            let angle = match p.crown {
                // The top limbs of a decurrent tree go up as the trunk ends.
                _ if p.trunk_share < 0.99 && i == steps => p.angle * (0.6 + self.rng.f() * 0.5),
                Crown::Palm => p.angle + self.rng.s() * 15.0,
                _ => p.angle + self.rng.s() * 8.0,
            };
            if length >= 1.0 {
                self.branch(at, turn(along, angle.to_radians(), azimuth), length, (r * 0.65).min(r0 * 0.75).max(0.5), 1);
            }
        }
        // Leaves at the top of a trunk that goes all the way.
        if p.leaf_density > 0.0 && p.leaf_size > 0.0 && p.trunk_share >= 0.99 {
            let top = trunk[steps].0;
            match p.crown {
                Crown::Palm => self.sk.clumps.push((top, 1.3)),
                // A conifer's leader: a spike of needles over the top whorl.
                Crown::Cone => {
                    for k in 0..3 {
                        self.sk.clumps.push((add(top, [0.0, 0.0, k as f32]), 0.75));
                    }
                }
                _ => self.sk.clumps.push((add(top, [0.0, 0.0, 1.0]), p.leaf_size * 0.9)),
            }
        }
        self.sk
    }
}

/// A block of the tree as it is laid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum B {
    /// A log along an axis: 0 x, 1 y, 2 up.
    Log(u8),
    /// A wall, between logs and fences as branches thin.
    Wall,
    Fence,
    Leaves,
    /// A stair whose high side faces (dx, dy).
    Stair(i8, i8),
    Slab,
}

impl B {
    fn wood(self) -> bool {
        matches!(self, B::Log(_) | B::Wall | B::Fence)
    }
}

const SIDES: [(i32, i32, i32); 6] = [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

struct Voxels {
    m: HashMap<(i32, i32, i32), B>,
}

impl Voxels {
    fn get(&self, x: i32, y: i32, z: i32) -> Option<B> {
        self.m.get(&(x, y, z)).copied()
    }

    /// Lay `b`: wood over leaves, the thicker over the thinner.
    fn put(&mut self, x: i32, y: i32, z: i32, b: B) {
        if z < 0 {
            return;
        }
        let rank = |b: B| match b {
            B::Log(_) => 3,
            B::Wall => 2,
            B::Fence => 1,
            _ => 0,
        };
        if self.m.get(&(x, y, z)).is_none_or(|&old| rank(b) > rank(old) || (rank(old) == 0 && rank(b) == 0)) {
            self.m.insert((x, y, z), b);
        }
    }

    /// The blocks on the line from `a` to `b`, each touching the one before
    /// by a face.
    fn line(&mut self, a: V, b: V, block: B) {
        let mut c = [a[0].floor() as i32, a[1].floor() as i32, a[2].floor() as i32];
        let end = [b[0].floor() as i32, b[1].floor() as i32, b[2].floor() as i32];
        let d = sub(b, a);
        let step = [d[0].signum() as i32, d[1].signum() as i32, d[2].signum() as i32];
        let next = |i: usize| -> f32 {
            if d[i].abs() < 1e-6 {
                f32::INFINITY
            } else {
                let edge = if step[i] > 0 { c[i] as f32 + 1.0 } else { c[i] as f32 };
                (edge - a[i]) / d[i]
            }
        };
        let mut t_max = [next(0), next(1), next(2)];
        let t_delta = [1.0 / d[0].abs().max(1e-6), 1.0 / d[1].abs().max(1e-6), 1.0 / d[2].abs().max(1e-6)];
        self.put(c[0], c[1], c[2], block);
        for _ in 0..256 {
            if c == end {
                break;
            }
            let i = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
            if t_max[i] > 1.0 {
                break;
            }
            c[i] += step[i];
            t_max[i] += t_delta[i];
            self.put(c[0], c[1], c[2], block);
        }
    }
}

/// The axis a piece of wood along `d` lies on.
fn axis(d: V) -> u8 {
    let a = [d[0].abs(), d[1].abs(), d[2].abs()];
    if a[2] >= a[0] * 0.8 && a[2] >= a[1] * 0.8 {
        2
    } else if a[0] >= a[1] {
        0
    } else {
        1
    }
}

/// Grow a tree. The same settings and seed give the same tree.
pub fn grow(p: &TreeParams, name: &str) -> ObjectDef {
    let mut rng = Rng(p.seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0x5EED);
    let height = (p.height * (0.88 + rng.f() * 0.24)).max(4.0);
    let sk = Grower { p, rng, sk: Skeleton::default(), height }.grow();
    let mut v = Voxels { m: HashMap::new() };

    // Wood: thick parts filled round, thin ones as a line.
    for s in &sk.wood {
        let dir = sub(s.b, s.a);
        let ax = axis(dir);
        let r = s.ra.max(s.rb);
        // Logs, then walls, then fences as the wood thins.
        let mean = (s.ra + s.rb) * 0.5;
        let line_block = match mean {
            _ if p.survival => B::Log(ax),
            m if m < 0.3 => B::Fence,
            m if m < 0.42 => B::Wall,
            _ => B::Log(ax),
        };
        v.line(s.a, s.b, line_block);
        if r >= 0.7 {
            let len2 = dot(dir, dir).max(1e-6);
            let lo = [s.a[0].min(s.b[0]) - r, s.a[1].min(s.b[1]) - r, s.a[2].min(s.b[2]) - r];
            let hi = [s.a[0].max(s.b[0]) + r, s.a[1].max(s.b[1]) + r, s.a[2].max(s.b[2]) + r];
            for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
                for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                    for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                        let c = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
                        let t = (dot(sub(c, s.a), dir) / len2).clamp(0.0, 1.0);
                        let rr = s.ra + (s.rb - s.ra) * t;
                        let q = sub(c, add(s.a, mul(dir, t)));
                        // Trunks are round in plan, whatever their slope.
                        if dot(q, q) <= rr * rr {
                            v.put(x, y, z, B::Log(ax));
                        }
                    }
                }
            }
        }
    }

    // A conifer's branches under a coat of needles (not its roots).
    if p.crown == Crown::Cone && p.leaf_density > 0.0 {
        let reach = p.trunk * 0.5 + 0.6;
        let tops: Vec<(i32, i32, i32)> = v.m.iter().filter(|(k, b)| b.wood() && k.2 as f32 > height * p.start * 0.8 && ((k.0 as f32 + 0.5).powi(2) + (k.1 as f32 + 0.5).powi(2)).sqrt() > reach).map(|(k, _)| *k).collect();
        for (x, y, z) in tops {
            v.put(x, y, z + 1, B::Leaves);
        }
    }

    // Leaves: lumpy clumps, flatter under an umbrella; fronds.
    let squash = match p.crown {
        Crown::Umbrella => 0.45,
        Crown::Cone => 0.5,
        _ => 0.7,
    };
    let seed = p.seed;
    // Holes as large as the clumps make them: wider in a giant's crown.
    let cell = ((p.leaf_size * 0.6).round() as i32).max(2);
    for &(c, r) in &sk.clumps {
        let hz = (r * squash).max(0.8);
        let n = r.ceil() as i32 + 1;
        let (cx, cy, cz) = (c[0].floor() as i32, c[1].floor() as i32, c[2].floor() as i32);
        for x in cx - n..=cx + n {
            for y in cy - n..=cy + n {
                for z in cz - n..=cz + n {
                    let q = [(x as f32 + 0.5 - c[0]) / r, (y as f32 + 0.5 - c[1]) / r, (z as f32 + 0.5 - c[2]) / hz];
                    let d = dot(q, q);
                    // Full inside, ragged at the rim, with holes the wood
                    // shows through (not in a conifer's needles).
                    let hole = p.crown != Crown::Cone && hash3(x.div_euclid(cell), y.div_euclid(cell), z.div_euclid(cell), seed ^ 0x51) < 0.2;
                    if d <= 1.0 && !hole && (d < 0.45 || hash3(x, y, z, seed) < p.leaf_density * (1.35 - d)) {
                        v.put(x, y, z, B::Leaves);
                    }
                }
            }
        }
    }
    for line in &sk.fronds {
        for w in line.windows(2) {
            v.line(w[0], w[1], B::Leaves);
        }
        // Wider in the middle of the frond.
        let n = line.len();
        for (i, &q) in line.iter().enumerate().skip(1) {
            let t = i as f32 / n as f32;
            if (0.15..0.65).contains(&t) && hash3(i as i32, n as i32, q[2] as i32, seed) < 0.7 {
                let d = norm(sub(line[i], line[i - 1]));
                let side = norm(cross(d, [0.0, 0.0, 1.0]));
                for s in [-1.0, 1.0] {
                    let b = add(q, mul(side, s));
                    v.put(b[0].floor() as i32, b[1].floor() as i32, b[2].floor() as i32, B::Leaves);
                }
            }
        }
    }

    // A weeping crown's curtains, from the underside of its rim.
    if p.crown == Crown::Weeping && p.leaf_density > 0.0 {
        let mut starts: Vec<(i32, i32, i32)> = v.m.iter().filter(|(_, b)| **b == B::Leaves).map(|(k, _)| *k).filter(|&(x, y, z)| v.get(x, y, z - 1).is_none()).collect();
        starts.sort_unstable();
        let reach = starts.iter().map(|&(x, y, _)| ((x * x + y * y) as f32).sqrt()).fold(0.0f32, f32::max);
        for (x, y, z) in starts {
            let out = ((x * x + y * y) as f32).sqrt() / reach.max(1.0);
            // Strands a block apart, longer towards the rim.
            if out > 0.3 && (x + y).rem_euclid(2) == 0 && hash3(x, y, z, seed ^ 7) < 0.6 {
                let length = 2 + (hash3(x, y, z, seed ^ 11) * height * 0.55 * out) as i32;
                for zz in (z - length).max(1)..z {
                    if v.get(x, y, zz).is_none() {
                        v.put(x, y, zz, B::Leaves);
                    }
                }
            }
        }
    }

    // The flare smoothed: stairs on its steps, slabs on its root tips.
    if !p.survival && p.flare > 0.0 {
        let flare_top = (2.0 + p.trunk * 1.2).ceil() as i32;
        let mut stairs = Vec::new();
        let mut slabs = Vec::new();
        for (&(x, y, z), &b) in &v.m {
            if !matches!(b, B::Log(_)) || z > flare_top || v.get(x, y, z + 1).is_some() {
                continue;
            }
            let (cx, cy) = (x as f32 + 0.5 - p.trunk * 0.25, y as f32 + 0.5 - p.trunk * 0.25);
            let (dx, dy) = if cx.abs() >= cy.abs() { (-(cx.signum() as i8), 0) } else { (0, -(cy.signum() as i8)) };
            if matches!(v.get(x + dx as i32, y + dy as i32, z + 1), Some(B::Log(_))) {
                stairs.push(((x, y, z + 1), B::Stair(dx, dy)));
            } else if z == 0 && SIDES[..4].iter().filter(|s| v.get(x + s.0, y + s.1, z).is_some_and(B::wood)).count() == 1 {
                slabs.push((x, y, z));
            }
        }
        for (k, b) in stairs {
            v.m.insert(k, b);
        }
        for k in slabs {
            v.m.insert(k, B::Slab);
        }
    }

    // Leaves within reach of wood (the game's seven blocks), and nothing
    // left floating.
    let mut dist: HashMap<(i32, i32, i32), u8> = HashMap::new();
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    let mut keys: Vec<_> = v.m.keys().copied().collect();
    keys.sort_unstable();
    for &k in &keys {
        if v.m[&k].wood() {
            dist.insert(k, 0);
            queue.push_back(k);
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        let d = dist[&(x, y, z)];
        if d >= reach(p.crown) {
            continue;
        }
        for s in SIDES {
            let n = (x + s.0, y + s.1, z + s.2);
            if v.m.get(&n) == Some(&B::Leaves) && !dist.contains_key(&n) {
                dist.insert(n, d + 1);
                queue.push_back(n);
            }
        }
    }
    v.m.retain(|k, b| *b != B::Leaves || dist.contains_key(k));
    let mut seen: HashMap<(i32, i32, i32), ()> = HashMap::new();
    for &k in &keys {
        if k.2 == 0 && v.m.get(&k).is_some_and(|b| b.wood() || matches!(b, B::Slab)) {
            seen.insert(k, ());
            queue.push_back(k);
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        for s in SIDES {
            let n = (x + s.0, y + s.1, z + s.2);
            if v.m.contains_key(&n) && !seen.contains_key(&n) {
                seen.insert(n, ());
                queue.push_back(n);
            }
        }
    }
    v.m.retain(|k, _| seen.contains_key(k));

    build(&v, &dist, p, name)
}

/// How far leaves may be from wood, counted through leaves: within the
/// game's reach, but for a palm's long fronds (whose leaves are persistent,
/// as all the generator's are).
pub fn reach(crown: Crown) -> u8 {
    if crown == Crown::Palm { 12 } else { 6 }
}

/// The block state of each block, and the object they make.
fn build(v: &Voxels, dist: &HashMap<(i32, i32, i32), u8>, p: &TreeParams, name: &str) -> ObjectDef {
    let wood = if p.wood.is_empty() { "oak" } else { p.wood.as_str() };
    let stem = matches!(wood, "crimson" | "warped");
    let log = if stem { format!("minecraft:{wood}_stem") } else { format!("minecraft:{wood}_log") };
    let leaves = match p.leaves.as_str() {
        "" => "minecraft:oak_leaves".to_owned(),
        l if l.contains(':') => l.to_owned(),
        l => format!("minecraft:{l}_leaves"),
    };
    let trim = trim(wood);
    // Massif's directions: x east, y south.
    let facing = |dx: i8, dy: i8| match (dx, dy) {
        (1, _) => "east",
        (-1, _) => "west",
        (_, 1) => "south",
        _ => "north",
    };
    let state = |k: (i32, i32, i32), b: B| -> String {
        match b {
            B::Log(a) => format!("{log}[axis={}]", ["x", "z", "y"][a as usize]),
            B::Leaves => format!("{leaves}[distance={},persistent=true,waterlogged=false]", dist.get(&k).copied().unwrap_or(1).clamp(1, 7)),
            B::Fence => {
                let (x, y, z) = k;
                let joins = |dx: i32, dy: i32| v.get(x + dx, y + dy, z).is_some_and(|b| !matches!(b, B::Stair(..) | B::Slab | B::Wall));
                format!("minecraft:{trim}_fence[east={},north={},south={},waterlogged=false,west={}]", joins(1, 0), joins(0, -1), joins(0, 1), joins(-1, 0))
            }
            B::Wall => {
                let (x, y, z) = k;
                let side = |dx: i32, dy: i32| if v.get(x + dx, y + dy, z).is_some_and(|b| matches!(b, B::Log(_) | B::Wall | B::Leaves)) { "low" } else { "none" };
                format!("minecraft:{}_wall[east={},north={},south={},up=true,waterlogged=false,west={}]", wall(wood), side(1, 0), side(0, -1), side(0, 1), side(-1, 0))
            }
            B::Stair(dx, dy) => format!("minecraft:{trim}_stairs[facing={},half=bottom,shape=straight,waterlogged=false]", facing(dx, dy)),
            B::Slab => format!("minecraft:{trim}_slab[type=bottom,waterlogged=false]"),
        }
    };
    let reach = v.m.keys().map(|&(x, y, _)| x.abs().max(y.abs())).max().unwrap_or(0);
    let height = v.m.keys().map(|&(_, _, z)| z + 1).max().unwrap_or(1).max(1);
    let width = 2 * reach + 1;
    let mut palette = vec!["minecraft:air".to_owned()];
    let mut index: HashMap<String, u16> = HashMap::new();
    let mut data = vec![0u16; (width * width * height) as usize];
    let mut keys: Vec<_> = v.m.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        let s = state(k, v.m[&k]);
        let n = *index.entry(s.clone()).or_insert_with(|| {
            palette.push(s);
            (palette.len() - 1) as u16
        });
        let (bx, bz, by) = ((k.0 + reach) as usize, (k.1 + reach) as usize, k.2 as usize);
        data[bx + bz * width as usize + by * (width * width) as usize] = n;
    }
    let mut def = ObjectDef::new(name, [width as u16, height as u16, width as u16], palette, data);
    def.anchor = [reach as u16, 0, reach as u16];
    def.source = SOURCE.to_owned();
    def
}

/// The planks whose fences, stairs and slabs look most like the bark of
/// `wood`: the game has no bark stairs.
pub fn trim(wood: &str) -> &str {
    match wood {
        "oak" | "jungle" | "mangrove" | "acacia" => "spruce",
        "spruce" | "dark_oak" | "cherry" | "pale_oak" => "dark_oak",
        w => w,
    }
}

/// The wall that looks most like the bark of `wood`, for branches too thin
/// for logs: the game has no wooden walls.
pub fn wall(wood: &str) -> &'static str {
    match wood {
        "birch" => "diorite",
        "acacia" => "andesite",
        "spruce" | "dark_oak" | "cherry" | "pale_oak" => "polished_blackstone",
        _ => "mud_brick",
    }
}

/// `n` trees of the same settings, each of its own seed: `seed`, `seed + 1`…
pub fn variants(p: &TreeParams, name: &str, n: usize) -> Vec<ObjectDef> {
    (0..n)
        .map(|k| {
            let mut q = p.clone();
            q.seed = p.seed.wrapping_add(k as u64);
            grow(&q, &format!("{name} {}", k + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(def: &ObjectDef) -> Vec<((usize, usize, usize), &str)> {
        let [w, h, l] = def.size.map(|s| s as usize);
        let mut out = Vec::new();
        for y in 0..h {
            for z in 0..l {
                for x in 0..w {
                    let k = def.at(x, y, z);
                    if k != 0 {
                        out.push(((x, y, z), def.palette[k as usize].as_str()));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_same_seed_grows_the_same_tree() {
        for preset in TreePreset::ALL {
            let p = preset.params();
            let (a, b) = (grow(&p, "a"), grow(&p, "a"));
            assert_eq!(a.blocks, b.blocks, "{}", preset.name());
            assert_eq!(a.palette, b.palette);
            let mut q = p.clone();
            q.seed += 1;
            assert_ne!(grow(&q, "a").blocks, a.blocks, "{}: another seed, another tree", preset.name());
        }
    }

    #[test]
    fn every_block_holds_to_the_foot_and_leaves_stay_near_wood() {
        for preset in TreePreset::ALL {
            for seed in 0..if preset.params().height > 50.0 { 2 } else { 6 } {
                for survival in [false, true] {
                    let mut p = preset.params();
                    p.seed = seed;
                    p.survival = survival;
                    let def = grow(&p, "t");
                    let all = blocks(&def);
                    assert!(all.len() > 20, "{}: a tree", preset.name());
                    let set: HashMap<(i32, i32, i32), &str> = all.iter().map(|&((x, y, z), s)| ((x as i32, y as i32, z as i32), s)).collect();
                    // Connected by faces to the blocks on the ground.
                    let mut seen: Vec<(i32, i32, i32)> = set.keys().copied().filter(|k| k.1 == 0).collect();
                    assert!(!seen.is_empty());
                    let mut done: std::collections::HashSet<(i32, i32, i32)> = seen.iter().copied().collect();
                    while let Some((x, y, z)) = seen.pop() {
                        for s in SIDES {
                            let n = (x + s.0, y + s.1, z + s.2);
                            if set.contains_key(&n) && done.insert(n) {
                                seen.push(n);
                            }
                        }
                    }
                    assert_eq!(done.len(), set.len(), "{} seed {seed}: nothing floats", preset.name());
                    // Every leaf within reach of wood.
                    let r = reach(p.crown) as i32;
                    for (&(x, y, z), s) in &set {
                        if s.contains("_leaves") {
                            let near = (-r..=r).any(|dx: i32| (-r..=r).any(|dy: i32| (-r..=r).any(|dz: i32| dx.abs() + dy.abs() + dz.abs() <= r && set.get(&(x + dx, y + dy, z + dz)).is_some_and(|b| b.contains("_log") || b.contains("_fence") || b.contains("_wall")))));
                            assert!(near, "{} seed {seed}: a leaf far from wood", preset.name());
                        }
                    }
                    if survival {
                        for s in set.values() {
                            assert!(s.contains("_log") || s.contains("_leaves"), "{}: survival has only logs and leaves, not {s}", preset.name());
                            assert!(!s.contains("_wall") && !s.contains("_fence"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_giant_grows_fast_and_whole() {
        let mut p = TreePreset::GiantSequoia.params();
        p.height = 120.0;
        let started = std::time::Instant::now();
        let def = grow(&p, "giant");
        let took = started.elapsed();
        println!("a {:?} giant in {took:?}", def.size);
        assert!(def.size[1] >= 100, "{:?}", def.size);
        assert!(took.as_secs_f32() < 3.0, "{took:?}");
        // A trunk of seven blocks or more at its foot.
        let [ax, _, az] = def.anchor.map(|a| a as usize);
        let wide = (0..def.size[0] as usize).filter(|&x| def.palette[def.at(x, 3, az) as usize].contains("_log")).count();
        assert!(wide >= 7, "{wide} blocks of trunk across");
        assert!(def.palette[def.at(ax, 0, az) as usize].contains("_log"));
    }

    #[test]
    fn variants_differ_and_stand_on_their_trunk() {
        let defs = variants(&TreePreset::Oak.params(), "Oak", 4);
        assert_eq!(defs.len(), 4);
        assert_eq!(defs[3].name, "Oak 4");
        assert!(defs.windows(2).all(|w| w[0].blocks != w[1].blocks));
        for d in &defs {
            assert_eq!(d.source, SOURCE);
            let [ax, _, az] = d.anchor.map(|a| a as usize);
            assert!(d.palette[d.at(ax, 0, az) as usize].contains("_log"), "the anchor is the foot of the trunk");
        }
    }
}
