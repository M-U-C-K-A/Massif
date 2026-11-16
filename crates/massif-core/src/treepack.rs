//! The tree pack: custom trees made by Massif itself, as WorldPainter
//! Modernised ships them — tall and slender spruces, firs, larches,
//! rainforest giants, big and pale oaks, birches, cherries, acacias,
//! willows, palms, redwoods, dead trees and bushes. Each kind is made, the
//! same way every time, in eight sizes and shapes, and becomes an object
//! layer that plants one of them at a time.

use crate::objects::{ObjectDef, ObjectLayerDef};
use crate::registry::Registry;
use std::collections::HashMap;
use std::f64::consts::PI;

/// A kind of tree of the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TreeKind {
    TallSpruce,
    Fir,
    Rainforest,
    BigOak,
    Birch,
    Cherry,
    Dead,
    Bush,
    Palm,
    Acacia,
    Willow,
    Azalea,
    PaleOak,
    Redwood,
    SlenderSpruce,
    Larch,
}

/// How many trees of each kind, in as many sizes and shapes.
pub const VARIANTS: usize = 8;

impl TreeKind {
    pub const ALL: [TreeKind; 16] = [
        TreeKind::TallSpruce,
        TreeKind::SlenderSpruce,
        TreeKind::Fir,
        TreeKind::Larch,
        TreeKind::Redwood,
        TreeKind::BigOak,
        TreeKind::PaleOak,
        TreeKind::Birch,
        TreeKind::Cherry,
        TreeKind::Acacia,
        TreeKind::Willow,
        TreeKind::Rainforest,
        TreeKind::Palm,
        TreeKind::Dead,
        TreeKind::Bush,
        TreeKind::Azalea,
    ];

    /// The name of its layer.
    pub const fn name(self) -> &'static str {
        match self {
            TreeKind::TallSpruce => "Tall spruces",
            TreeKind::Fir => "Firs",
            TreeKind::Rainforest => "Rainforest giants",
            TreeKind::BigOak => "Big oaks",
            TreeKind::Birch => "Birches",
            TreeKind::Cherry => "Cherry trees",
            TreeKind::Dead => "Dead trees",
            TreeKind::Bush => "Bushes",
            TreeKind::Palm => "Palm trees",
            TreeKind::Acacia => "Acacias",
            TreeKind::Willow => "Willows",
            TreeKind::Azalea => "Azalea bushes",
            TreeKind::PaleOak => "Pale oaks",
            TreeKind::Redwood => "Giant redwoods",
            TreeKind::SlenderSpruce => "Slender spruces",
            TreeKind::Larch => "Larches",
        }
    }

    /// What they look like, in a line.
    pub const fn about(self) -> &'static str {
        match self {
            TreeKind::TallSpruce => "Long bare trunks with sparse tiers of short branches",
            TreeKind::Fir => "Dense cones",
            TreeKind::Rainforest => "Tall, thick trunks on buttress roots under wide, flat canopies",
            TreeKind::BigOak => "A few thick limbs, each ending in a round clump of leaves",
            TreeKind::Birch => "Slender, with a narrow crown",
            TreeKind::Cherry => "A short trunk forking into broad pink crowns",
            TreeKind::Dead => "Bare trunks with crooked branches",
            TreeKind::Bush => "Low rounded bushes of oak or azalea leaves",
            TreeKind::Palm => "Leaning, curving trunks with drooping fronds",
            TreeKind::Acacia => "Slanting limbs carrying flat umbrellas of leaves",
            TreeKind::Willow => "A broad crown with curtains of leaves hanging from its rim",
            TreeKind::Azalea => "Low round bushes with flowering patches",
            TreeKind::PaleOak => "Thick trunks under broad, grey-green crowns",
            TreeKind::Redwood => "Very tall, thick trunks with a narrow crown in their upper half",
            TreeKind::SlenderSpruce => "Narrow columns of short layered branches, as on steep mountain slopes",
            TreeKind::Larch => "Narrow, airy cones of light green needles",
        }
    }

    /// Its colour in the lists and on the map.
    pub const fn color(self) -> [u8; 3] {
        let c: u32 = match self {
            TreeKind::TallSpruce => 0x24452f,
            TreeKind::Fir => 0x2b5a3a,
            TreeKind::Rainforest => 0x3f7f2a,
            TreeKind::BigOak => 0x3a6b2a,
            TreeKind::Birch => 0x6e9b45,
            TreeKind::Cherry => 0xf0a3c8,
            TreeKind::Dead => 0x6b5a48,
            TreeKind::Bush => 0x4d7a35,
            TreeKind::Palm => 0x5c9a3a,
            TreeKind::Acacia => 0x8a9a3a,
            TreeKind::Willow => 0x5d8a3a,
            TreeKind::Azalea => 0x7fa04a,
            TreeKind::PaleOak => 0x9a9a8a,
            TreeKind::Redwood => 0x6b3a2a,
            TreeKind::SlenderSpruce => 0x234a32,
            TreeKind::Larch => 0x8aa64a,
        };
        [(c >> 16) as u8, (c >> 8) as u8, c as u8]
    }

    /// Its place in WorldPainter's list, which seeds its trees.
    const fn ordinal(self) -> i64 {
        match self {
            TreeKind::TallSpruce => 0,
            TreeKind::Fir => 1,
            TreeKind::Rainforest => 2,
            TreeKind::BigOak => 3,
            TreeKind::Birch => 4,
            TreeKind::Cherry => 5,
            TreeKind::Dead => 6,
            TreeKind::Bush => 7,
            TreeKind::Palm => 8,
            TreeKind::Acacia => 9,
            TreeKind::Willow => 10,
            TreeKind::Azalea => 11,
            TreeKind::PaleOak => 12,
            TreeKind::Redwood => 13,
            TreeKind::SlenderSpruce => 14,
            TreeKind::Larch => 15,
        }
    }

    /// Whether it is a conifer (drawn with a pointed icon).
    pub const fn conifer(self) -> bool {
        matches!(self, TreeKind::TallSpruce | TreeKind::Fir | TreeKind::Redwood | TreeKind::SlenderSpruce | TreeKind::Larch)
    }

    /// Its trees, in [`VARIANTS`] sizes and shapes: always the same ones.
    pub fn trees(self) -> Vec<ObjectDef> {
        let mut random = JavaRandom::new(self.ordinal() * 7919 + 17);
        (0..VARIANTS).map(|i| self.tree(i, &mut random)).collect()
    }

    fn tree(self, variant: usize, r: &mut JavaRandom) -> ObjectDef {
        match self {
            TreeKind::TallSpruce => tall_spruce(variant, r),
            TreeKind::Fir => fir(variant, r),
            TreeKind::Rainforest => rainforest(variant, r),
            TreeKind::BigOak => big_oak(variant, r),
            TreeKind::Birch => birch(variant, r),
            TreeKind::Cherry => cherry(variant, r),
            TreeKind::Dead => dead(variant, r),
            TreeKind::Bush => bush(variant, r),
            TreeKind::Palm => palm(variant, r),
            TreeKind::Acacia => acacia(variant, r),
            TreeKind::Willow => willow(variant, r),
            TreeKind::Azalea => azalea(variant, r),
            TreeKind::PaleOak => pale_oak(variant, r),
            TreeKind::Redwood => redwood(variant, r),
            TreeKind::SlenderSpruce => slender_spruce(variant, r),
            TreeKind::Larch => larch(variant, r),
        }
    }

    /// The object layer of this kind in `reg`: the one made before (by its
    /// name, its trees from the pack), or a new one with its trees. Returns
    /// its number.
    pub fn install(self, reg: &mut Registry) -> u16 {
        if let Some(k) = reg.object_layers.iter().position(|l| !l.removed && l.name == self.name() && l.objects.iter().all(|o| reg.objects.get(o.0 as usize).is_some_and(|d| d.source == SOURCE))) {
            return k as u16 + 1;
        }
        let first = reg.objects.len() as u32;
        let trees = self.trees();
        let n = trees.len() as u32;
        reg.objects.extend(trees);
        let indices: Vec<u32> = (first..first + n).collect();
        let mut layer = ObjectLayerDef::new(self.name(), &indices, reg);
        let reach = indices.iter().map(|&k| reg.objects[k as usize].reach()).max().unwrap_or(2);
        // A forest: crowns may touch, a little.
        layer.spacing = ((reach as f32 * 1.3) as u16 + 2).clamp(3, 64);
        layer.density = 0.75;
        layer.color = self.color();
        reg.add_object_layer(layer)
    }
}

/// An object layer named `name` of the trees of several kinds, each kind
/// with its weight (shared among its variants): WorldPainter's own forests
/// (Deciduous: big oaks and birches) as the pack has them. The one made
/// before, by its name and its trees from the pack, or a new one. Returns
/// its number.
pub fn install_mix(name: &str, kinds: &[(TreeKind, u16)], reg: &mut Registry) -> u16 {
    if let Some(k) = reg.object_layers.iter().position(|l| !l.removed && l.name == name && l.objects.iter().all(|o| reg.objects.get(o.0 as usize).is_some_and(|d| d.source == SOURCE))) {
        return k as u16 + 1;
    }
    let mut objects = Vec::new();
    for &(kind, weight) in kinds {
        let first = reg.objects.len() as u32;
        let trees = kind.trees();
        let n = trees.len() as u32;
        reg.objects.extend(trees);
        objects.extend((first..first + n).map(|k| (k, weight.max(1))));
    }
    let indices: Vec<u32> = objects.iter().map(|o| o.0).collect();
    let mut layer = ObjectLayerDef::new(name, &indices, reg);
    layer.objects = objects;
    let reach = indices.iter().map(|&k| reg.objects[k as usize].reach()).max().unwrap_or(2);
    layer.spacing = ((reach as f32 * 1.3) as u16 + 2).clamp(3, 64);
    layer.density = 0.75;
    layer.color = kinds.first().map_or([58, 112, 52], |k| k.0.color());
    reg.add_object_layer(layer)
}

/// What the pack's trees say they were read from.
pub const SOURCE: &str = "Massif tree pack";

/// `java.util.Random`, so that the trees are those of WorldPainter
/// Modernised's pack.
struct JavaRandom(u64);

impl JavaRandom {
    const MULT: u64 = 0x5_DEEC_E66D;
    const MASK: u64 = (1 << 48) - 1;

    fn new(seed: i64) -> Self {
        Self((seed as u64 ^ Self::MULT) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(Self::MULT).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i64 as i32
    }

    fn int(&mut self, bound: i32) -> i32 {
        if bound & (-bound) == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn double(&mut self) -> f64 {
        (((self.next(26) as i64) << 27) + self.next(27) as i64) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn boolean(&mut self) -> bool {
        self.next(1) != 0
    }
}

/// Java's `Math.round`.
fn round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

fn floor(v: f64) -> i32 {
    v.floor() as i32
}

fn log(wood: &str) -> String {
    format!("minecraft:{wood}_log[axis=y]")
}

fn leaves(wood: &str) -> String {
    format!("minecraft:{wood}_leaves[distance=1,persistent=true,waterlogged=false]")
}

fn is_log(state: &str) -> bool {
    state.split('[').next().unwrap_or(state).ends_with("_log")
}

fn is_leaves(state: &str) -> bool {
    state.split('[').next().unwrap_or(state).ends_with("_leaves")
}

/// Builds a tree around its trunk, at the origin of the horizontal
/// coordinates (`x`, `y`; `z` up, as WorldPainter has them). The object is
/// as large as what was built, so that crowns are never cut off, and
/// stands on the foot of its trunk.
struct Builder {
    name: String,
    blocks: HashMap<(i32, i32, i32), String>,
}

impl Builder {
    fn new(name: String) -> Self {
        Self { name, blocks: HashMap::new() }
    }

    /// Set a block; wood is not replaced by leaves.
    fn set(&mut self, x: i32, y: i32, z: i32, block: &str) {
        if z < 0 {
            return;
        }
        if let Some(existing) = self.blocks.get(&(x, y, z))
            && is_log(existing)
            && is_leaves(block)
        {
            return;
        }
        self.blocks.insert((x, y, z), block.to_owned());
    }

    fn has(&self, x: i32, y: i32, z: i32) -> bool {
        self.blocks.contains_key(&(x, y, z))
    }

    fn trunk(&mut self, log: &str, height: i32, thickness: i32) {
        for z in 0..height {
            for dx in 0..thickness {
                for dy in 0..thickness {
                    self.set(dx, dy, z, log);
                }
            }
        }
    }

    fn column(&mut self, block: &str, from: i32, to: i32) {
        for z in from..=to {
            self.set(0, 0, z, block);
        }
    }

    /// A horizontal branch from a corner of the trunk in one of the four
    /// directions.
    fn branch(&mut self, log: &str, z: i32, direction: i32, length: i32, offset_x: i32, offset_y: i32) {
        let dx = match direction {
            0 => 1,
            2 => -1,
            _ => 0,
        };
        let dy = match direction {
            1 => 1,
            3 => -1,
            _ => 0,
        };
        let (sx, sy) = (if dx > 0 { offset_x } else { 0 }, if dy > 0 { offset_y } else { 0 });
        let horizontal = log.replace("axis=y", if dx != 0 { "axis=x" } else { "axis=z" });
        for i in 1..=length {
            self.set(sx + dx * i, sy + dy * i, z, &horizontal);
        }
    }

    /// A line of wood between two points, for slanting limbs.
    #[allow(clippy::too_many_arguments)]
    fn line(&mut self, log: &str, x1: f64, y1: f64, z1: i32, x2: f64, y2: f64, z2: i32) {
        let steps = ((x2 - x1).hypot(y2 - y1).max((z2 - z1).abs() as f64)).ceil() as i32 * 2 + 1;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            self.set(floor(x1 + (x2 - x1) * t), floor(y1 + (y2 - y1) * t), round(z1 as f64 + (z2 - z1) as f64 * t), log);
        }
    }

    /// A flat disc of leaves, filled in less and less towards its rim.
    #[allow(clippy::too_many_arguments)]
    fn disc(&mut self, leaves: &str, cx: f64, cy: f64, z: i32, radius: f64, fill: f64, r: &mut JavaRandom) {
        let n = radius.ceil() as i32;
        for x in floor(cx) - n..=floor(cx) + n {
            for y in floor(cy) - n..=floor(cy) + n {
                let d = (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy);
                if d <= radius && (d < radius * 0.6 || r.double() < fill * (1.25 - d / (radius + 1.0))) {
                    self.set(x, y, z, leaves);
                }
            }
        }
    }

    /// A partial ring of leaves, the drooping tips of a tier.
    #[allow(clippy::too_many_arguments)]
    fn ring(&mut self, leaves: &str, cx: f64, cy: f64, z: i32, inner: f64, outer: f64, fill: f64, r: &mut JavaRandom) {
        let n = outer.ceil() as i32;
        for x in floor(cx) - n..=floor(cx) + n {
            for y in floor(cy) - n..=floor(cy) + n {
                let d = (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy);
                if d >= inner && d <= outer && r.double() < fill {
                    self.set(x, y, z, leaves);
                }
            }
        }
    }

    /// An ellipsoid of leaves, its outer layer partly filled: lumpy.
    #[allow(clippy::too_many_arguments)]
    fn blob(&mut self, leaves: &str, cx: f64, cy: f64, cz: i32, radius: f64, half_height: f64, fill: f64, r: &mut JavaRandom) {
        let (n, h) = (radius.ceil() as i32, half_height.ceil() as i32);
        for z in cz - h..=cz + h {
            for x in floor(cx) - n..=floor(cx) + n {
                for y in floor(cy) - n..=floor(cy) + n {
                    let (dx, dy, dz) = ((x as f64 + 0.5 - cx) / radius, (y as f64 + 0.5 - cy) / radius, (z - cz) as f64 / half_height);
                    let d = dx * dx + dy * dy + dz * dz;
                    if d <= 1.0 && (d < 0.6 || r.double() < fill) {
                        self.set(x, y, z, leaves);
                    }
                }
            }
        }
    }

    /// A rounded clump of a few overlapping ellipsoids: full, not a ball.
    #[allow(clippy::too_many_arguments)]
    fn cluster(&mut self, leaves: &str, cx: f64, cy: f64, cz: i32, radius: f64, half_height: f64, r: &mut JavaRandom) {
        self.blob(leaves, cx, cy, cz, radius, half_height, 0.9, r);
        let lumps = 3 + r.int(3);
        for _ in 0..lumps {
            let (angle, reach) = (r.double() * PI * 2.0, radius * (0.45 + r.double() * 0.35));
            let (x, y, z) = (cx + angle.cos() * reach, cy + angle.sin() * reach, cz + r.int(3) - 1);
            let radius = radius * (0.5 + r.double() * 0.2);
            self.blob(leaves, x, y, z, radius, half_height * 0.8, 0.85, r);
        }
    }

    /// Strands of leaves hanging from the underside of the crown's rim.
    fn hang(&mut self, leaves: &str, chance: f64, max_length: i32, r: &mut JavaRandom) {
        let mut starts: Vec<(i32, i32, i32)> = self
            .blocks
            .iter()
            .filter(|(_, b)| is_leaves(b))
            .map(|(p, _)| *p)
            .filter(|&(x, y, z)| !self.has(x, y, z - 1) && (!self.has(x + 1, y, z) || !self.has(x - 1, y, z) || !self.has(x, y + 1, z) || !self.has(x, y - 1, z)))
            .collect();
        starts.sort_unstable();
        for (x, y, z) in starts {
            if r.double() < chance {
                let length = 1 + r.int(max_length.max(1));
                let mut zz = z - 1;
                while zz >= z - length && zz >= 1 {
                    self.set(x, y, zz, leaves);
                    zz -= 1;
                }
            }
        }
    }

    /// Patches of another block at a spot and its neighbours.
    fn recolour(&mut self, from: &str, to: &str, x: i32, y: i32, z: i32) {
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(b) = self.blocks.get_mut(&(x + dx, y + dy, z))
                    && b == from
                {
                    *b = to.to_owned();
                }
            }
        }
    }

    fn build(self) -> ObjectDef {
        let reach = self.blocks.keys().map(|&(x, y, _)| x.abs().max(y.abs())).max().unwrap_or(0);
        let height = self.blocks.keys().map(|&(_, _, z)| z + 1).max().unwrap_or(1).max(1);
        let width = 2 * reach + 1;
        let mut palette = vec!["minecraft:air".to_owned()];
        let mut data = vec![0u16; (width * width * height) as usize];
        let mut keys: Vec<_> = self.blocks.keys().copied().collect();
        keys.sort_unstable();
        for (x, y, z) in keys {
            let state = &self.blocks[&(x, y, z)];
            let k = match palette.iter().position(|p| p == state) {
                Some(k) => k,
                None => {
                    palette.push(state.clone());
                    palette.len() - 1
                }
            };
            // Massif's boxes: x, then z (WorldPainter's y), then y up.
            let (bx, bz, by) = ((x + reach) as usize, (y + reach) as usize, z as usize);
            data[bx + bz * width as usize + by * (width * width) as usize] = k as u16;
        }
        let size = [width as u16, height as u16, width as u16];
        let mut def = ObjectDef::new(&self.name, size, palette, data);
        def.anchor = [reach as u16, 0, reach as u16];
        def.source = SOURCE.to_owned();
        def
    }
}

fn tall_spruce(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 22 + r.int(13);
    let mut b = Builder::new(format!("Tall spruce {}", variant + 1));
    let (log, leaves) = (log("spruce"), leaves("spruce"));
    let thickness = if height > 30 { 2 } else { 1 };
    let c = (thickness - 1) as f64 / 2.0 + 0.5;
    b.trunk(&log, height, thickness);
    let first = (height as f64 * (0.28 + r.double() * 0.1)) as i32;
    for z in first..height {
        let t = (z - first) as f64 / (height - first) as f64;
        let radius = if (z - first) % 2 == 0 { thickness as f64 * 0.5 + 0.9 + (1.0 - t) * (3.0 + r.double() * 0.8) } else { thickness as f64 * 0.5 + 0.5 + (1.0 - t) * 1.6 };
        b.disc(&leaves, c, c, z, radius, 0.92, r);
        if (z - first) % 2 == 0 {
            b.ring(&leaves, c, c, z - 1, radius - 0.6, radius + 0.2, 0.35, r);
        }
    }
    b.column(&leaves, height, height + 2);
    b.build()
}

fn slender_spruce(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 9 + r.int(14);
    let mut b = Builder::new(format!("Slender spruce {}", variant + 1));
    let (log, leaves) = (log("spruce"), leaves("spruce"));
    b.trunk(&log, height, 1);
    let bottom = 1 + r.int(2);
    let width = 1.3 + r.double() * 0.7 + height as f64 * 0.03;
    for z in bottom..height {
        let t = (z - bottom) as f64 / (height - bottom) as f64;
        let radius = 0.6 + width * (1.0 - t).powf(0.6) + if (z - bottom) % 2 == 0 { 0.45 } else { 0.0 };
        b.disc(&leaves, 0.5, 0.5, z, radius, 0.9, r);
    }
    b.column(&leaves, height, height + 2);
    b.build()
}

fn larch(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 11 + r.int(10);
    let mut b = Builder::new(format!("Larch {}", variant + 1));
    let (log, leaves) = (log("spruce"), leaves("birch"));
    b.trunk(&log, height, 1);
    let bottom = 2 + r.int(3);
    for z in bottom..height {
        let t = (z - bottom) as f64 / (height - bottom) as f64;
        let radius = 0.6 + (1.0 - t) * (2.2 + r.double() * 0.4);
        b.disc(&leaves, 0.5, 0.5, z, radius, 0.72, r);
    }
    b.column(&leaves, height, height + 1);
    b.build()
}

fn fir(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 12 + r.int(9);
    let mut b = Builder::new(format!("Fir {}", variant + 1));
    let (log, leaves) = (log("spruce"), leaves("spruce"));
    b.trunk(&log, height, 1);
    let bottom = 2 + r.int(2);
    for z in bottom..height {
        let t = (z - bottom) as f64 / (height - bottom) as f64;
        let radius = 0.7 + (1.0 - t) * 3.8 + if (z - bottom) % 3 == 0 { 0.7 } else { 0.0 };
        b.disc(&leaves, 0.5, 0.5, z, radius, 0.95, r);
    }
    b.column(&leaves, height, height + 1);
    b.build()
}

fn rainforest(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 18 + r.int(11);
    let spread = 5.5 + r.double() * 3.0;
    let mut b = Builder::new(format!("Rainforest giant {}", variant + 1));
    let (log, leaves) = (log("jungle"), leaves("jungle"));
    b.trunk(&log, height, 2);
    for d in 0..4 {
        b.branch(&log, 0, d, 2, 1, 1);
        b.branch(&log, 1, d, 1, 1, 1);
    }
    let limbs = 4 + r.int(3);
    for i in 0..limbs {
        let angle = (i as f64 + r.double() * 0.6) * PI * 2.0 / limbs as f64;
        let reach = spread * (0.55 + r.double() * 0.35);
        let z = height - 1 + r.int(3);
        let (x, y) = (1.0 + angle.cos() * reach, 1.0 + angle.sin() * reach);
        let from = height - 4 - r.int(3);
        b.line(&log, 1.0, 1.0, from, x, y, z);
        let radius = 2.6 + r.double();
        b.cluster(&leaves, x, y, z + 1, radius, 1.6, r);
    }
    b.blob(&leaves, 1.0, 1.0, height + 1, spread * 0.6, 2.0, 0.9, r);
    b.build()
}

fn big_oak(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 8 + r.int(6);
    let thickness = if height >= 11 { 2 } else { 1 };
    let c = (thickness - 1) as f64 / 2.0 + 0.5;
    let mut b = Builder::new(format!("Big oak {}", variant + 1));
    let (log, leaves) = (log("oak"), leaves("oak"));
    b.trunk(&log, height, thickness);
    let radius = 3.2 + r.double() * 0.6;
    b.cluster(&leaves, c, c, height + 1, radius, 2.6, r);
    let limbs = 4 + r.int(3);
    for i in 0..limbs {
        let angle = (i as f64 + r.double() * 0.7) * PI * 2.0 / limbs as f64;
        let reach = 3.0 + r.double() * 2.5;
        let from = (height as f64 * (0.5 + r.double() * 0.3)) as i32;
        let z = from + 2 + r.int(3);
        let (x, y) = (c + angle.cos() * reach, c + angle.sin() * reach);
        b.line(&log, c, c, from, x, y, z);
        let radius = 2.3 + r.double() * 0.8;
        b.cluster(&leaves, x, y, z + 1, radius, 2.0, r);
    }
    b.build()
}

fn birch(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 9 + r.int(6);
    let mut b = Builder::new(format!("Birch {}", variant + 1));
    b.trunk(&log("birch"), height, 1);
    let radius = 2.3 + r.double();
    b.blob(&leaves("birch"), 0.5, 0.5, height - 2, radius, 3.6, 0.92, r);
    b.column(&leaves("birch"), height, height + 1);
    b.build()
}

fn cherry(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 5 + r.int(3);
    let mut b = Builder::new(format!("Cherry tree {}", variant + 1));
    let (log, leaves) = (log("cherry"), leaves("cherry"));
    b.trunk(&log, height, 1);
    let forks = 3 + r.int(2);
    for i in 0..forks {
        let angle = (i as f64 + r.double() * 0.6) * PI * 2.0 / forks as f64;
        let reach = 2.2 + r.double() * 2.0;
        let (x, y) = (0.5 + angle.cos() * reach, 0.5 + angle.sin() * reach);
        let z = height + 1 + r.int(2);
        b.line(&log, 0.5, 0.5, height - 1, x, y, z);
        let radius = 2.8 + r.double();
        b.cluster(&leaves, x, y, z + 1, radius, 1.9, r);
    }
    b.blob(&leaves, 0.5, 0.5, height + 2, 2.8, 1.8, 0.9, r);
    b.hang(&leaves, 0.35, 2, r);
    b.build()
}

fn dead(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 6 + r.int(7);
    let mut b = Builder::new(format!("Dead tree {}", variant + 1));
    let log = if r.boolean() { log("oak") } else { log("spruce") };
    b.trunk(&log, height, 1);
    let branches = 2 + r.int(3);
    for _ in 0..branches {
        let (angle, reach) = (r.double() * PI * 2.0, 2.0 + r.double() * 2.5);
        let z = height / 2 + r.int((height / 2).max(1));
        let top = z + 1 + r.int(2);
        b.line(&log, 0.5, 0.5, z, 0.5 + angle.cos() * reach, 0.5 + angle.sin() * reach, top);
    }
    b.build()
}

fn palm(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 7 + r.int(6);
    let mut b = Builder::new(format!("Palm tree {}", variant + 1));
    let (log, leaves) = (log("jungle"), leaves("jungle"));
    let (angle, lean) = (r.double() * PI * 2.0, 2.0 + r.double() * 2.5);
    let (mut top_x, mut top_y) = (0.0, 0.0);
    for z in 0..height {
        let t = z as f64 / height as f64;
        top_x = angle.cos() * lean * t * t;
        top_y = angle.sin() * lean * t * t;
        b.set(floor(top_x + 0.5), floor(top_y + 0.5), z, &log);
    }
    let (cx, cy, cz) = (floor(top_x + 0.5), floor(top_y + 0.5), height);
    b.disc(&leaves, cx as f64 + 0.5, cy as f64 + 0.5, cz, 1.5, 1.0, r);
    b.set(cx, cy, cz + 1, &leaves);
    let fronds = 7 + r.int(3);
    for f in 0..fronds {
        let a = f as f64 * PI * 2.0 / fronds as f64 + r.double() * 0.4;
        let length = 5 + r.int(2);
        for i in 1..=length {
            let droop = if i <= 2 { 0.0 } else { -0.3 * ((i - 2) * (i - 2)) as f64 };
            let (x, y, z) = (cx + round(a.cos() * i as f64), cy + round(a.sin() * i as f64), cz + round(droop));
            b.set(x, y, z, &leaves);
            if i >= 2 && i <= length - 2 {
                b.set(cx + round((a + 0.25).cos() * i as f64), cy + round((a + 0.25).sin() * i as f64), z, &leaves);
            }
            if i == length {
                b.set(x, y, z - 1, &leaves);
            }
        }
    }
    b.build()
}

fn bush(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let mut b = Builder::new(format!("Bush {}", variant + 1));
    let leaves = if variant.is_multiple_of(3) { leaves("azalea") } else { leaves("oak") };
    let radius = 1.6 + r.double() * 1.2;
    b.blob(&leaves, 0.5, 0.5, 1, radius, 1.4, 0.92, r);
    b.set(0, 0, 0, &log("oak"));
    b.build()
}

fn acacia(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 6 + r.int(4);
    let mut b = Builder::new(format!("Acacia {}", variant + 1));
    let (log, leaves) = (log("acacia"), leaves("acacia"));
    b.trunk(&log, height - 2, 1);
    let forks = 2 + r.int(2);
    for i in 0..forks {
        let angle = i as f64 * PI * 2.0 / forks as f64 + r.double() * 0.8;
        let reach = 2.0 + r.double() * 2.0;
        let (x, y) = (0.5 + angle.cos() * reach, 0.5 + angle.sin() * reach);
        let z = height + r.int(2);
        b.line(&log, 0.5, 0.5, height - 3, x, y, z);
        let radius = 3.0 + r.double();
        b.disc(&leaves, x, y, z + 1, radius, 0.95, r);
        b.disc(&leaves, x, y, z + 2, radius - 1.3, 0.95, r);
    }
    b.build()
}

fn willow(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 7 + r.int(4);
    let mut b = Builder::new(format!("Willow {}", variant + 1));
    let (log, leaves) = (log("oak"), leaves("mangrove"));
    b.trunk(&log, height, 1);
    for i in 0..4 {
        let a = i as f64 * PI / 2.0 + 0.4;
        b.line(&log, 0.5, 0.5, height - 3, 0.5 + a.cos() * 2.5, 0.5 + a.sin() * 2.5, height);
    }
    let radius = 4.3 + r.double() * 0.6;
    b.cluster(&leaves, 0.5, 0.5, height + 1, radius, 2.4, r);
    b.hang(&leaves, 0.65, height - 2, r);
    b.build()
}

fn azalea(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 2 + r.int(2);
    let mut b = Builder::new(format!("Azalea {}", variant + 1));
    let (log, leaves, flowering) = (log("oak"), leaves("azalea"), leaves("flowering_azalea"));
    b.trunk(&log, height, 1);
    let radius = 2.4 + r.double() * 0.6;
    b.blob(&leaves, 0.5, 0.5, height + 1, radius, 1.7, 0.9, r);
    for _ in 0..10 {
        let (angle, reach) = (r.double() * PI * 2.0, r.double() * 2.5);
        let z = height + r.int(3);
        b.recolour(&leaves, &flowering, floor(0.5 + angle.cos() * reach), floor(0.5 + angle.sin() * reach), z);
    }
    b.build()
}

fn pale_oak(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 7 + r.int(5);
    let mut b = Builder::new(format!("Pale oak {}", variant + 1));
    let (log, leaves) = (log("pale_oak"), leaves("pale_oak"));
    b.trunk(&log, height, 2);
    let radius = 4.0 + r.double();
    b.cluster(&leaves, 1.0, 1.0, height + 1, radius, 2.4, r);
    let limbs = 4 + r.int(2);
    for i in 0..limbs {
        let angle = (i as f64 + r.double() * 0.6) * PI * 2.0 / limbs as f64;
        let reach = 3.0 + r.double() * 1.8;
        let z = height - 1 + r.int(2);
        let (x, y) = (1.0 + angle.cos() * reach, 1.0 + angle.sin() * reach);
        b.line(&log, 1.0, 1.0, z - 2, x, y, z);
        let radius = 2.6 + r.double() * 0.6;
        b.cluster(&leaves, x, y, z + 1, radius, 1.9, r);
    }
    b.hang(&leaves, 0.15, 2, r);
    b.build()
}

fn redwood(variant: usize, r: &mut JavaRandom) -> ObjectDef {
    let height = 30 + r.int(14);
    let mut b = Builder::new(format!("Giant redwood {}", variant + 1));
    let (log, leaves) = (log("spruce"), leaves("spruce"));
    b.trunk(&log, height, 2);
    let first = (height as f64 * (0.45 + r.double() * 0.1)) as i32;
    for z in first..height {
        let t = (z - first) as f64 / (height - first) as f64;
        let radius = if (z - first) % 3 == 0 { 2.4 + (1.0 - t) * 2.8 } else { 1.9 + (1.0 - t) * 1.8 };
        b.disc(&leaves, 1.0, 1.0, z, radius, 0.92, r);
        if (z - first) % 3 == 0 && t < 0.8 {
            let d = r.int(4);
            b.branch(&log, z, d, round(radius) - 1, 1, 1);
        }
    }
    b.blob(&leaves, 1.0, 1.0, height, 1.8, 1.8, 0.95, r);
    b.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Java's `Random` gives the numbers Java gives.
    #[test]
    fn java_random() {
        let mut r = JavaRandom::new(42);
        assert_eq!(r.int(10), 0);
        assert_eq!(r.int(10), 3);
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next(32), -1170105035);
        let mut r = JavaRandom::new(0);
        assert!((r.double() - 0.730967787376657).abs() < 1e-15);
    }

    /// Every kind gives eight trees standing on their trunk, the same
    /// every time, none cut off.
    #[test]
    fn trees_are_made_whole_and_the_same() {
        for kind in TreeKind::ALL {
            let trees = kind.trees();
            assert_eq!(trees.len(), VARIANTS);
            assert_eq!(trees, kind.trees(), "{kind:?} always the same");
            for t in &trees {
                let [w, h, l] = t.size;
                assert_eq!(w, l);
                assert!(h >= 2, "{} is {h} high", t.name);
                let foot = t.at(t.anchor[0] as usize, 0, t.anchor[2] as usize);
                assert!(t.palette[foot as usize].ends_with("_log[axis=y]"), "{} stands on {}", t.name, t.palette[foot as usize]);
                assert!(t.count() > 3, "{}", t.name);
            }
        }
        let redwoods = TreeKind::Redwood.trees();
        assert!(redwoods.iter().all(|t| t.size[1] >= 30));
        let bushes = TreeKind::Bush.trees();
        assert!(bushes.iter().all(|t| t.size[1] <= 5));
    }

    /// A kind is installed once: asked again, its layer is found.
    #[test]
    fn install_once() {
        let mut reg = Registry::vanilla();
        let n = TreeKind::Fir.install(&mut reg);
        assert_eq!(TreeKind::Fir.install(&mut reg), n);
        assert_eq!(reg.objects.len(), VARIANTS);
        let layer = reg.object_layer(n).unwrap();
        assert_eq!((layer.name.as_str(), layer.objects.len()), ("Firs", VARIANTS));
        assert!(layer.spacing >= 3);
        let m = TreeKind::Birch.install(&mut reg);
        assert_ne!(m, n);
        assert_eq!(reg.objects.len(), 2 * VARIANTS);
    }
}
