//! Ores, laid in the rock as Minecraft lays them since 1.18: veins of coal,
//! iron, copper, gold, redstone, lapis, diamonds and emeralds, each kind in
//! bands of height where its veins start — fewest at the ends of a band,
//! most over its peak —, so many veins a chunk, so many blocks a vein.
//!
//! The export writes the map's chunks finished, so Minecraft adds no ore of
//! its own: these are the map's. They take the place of natural rock only
//! (stone, granite, diorite, andesite; deepslate and tuff, where they are of
//! the deepslate kind), never of a block in the open — one without solid
//! ground right above it —, and the first vein to reach a block keeps it.
//! The veins of a chunk come from the seed, the chunk and the band alone:
//! the export, the survival check and the cut through the map in the Ores
//! window all ask a [`Placer`], and agree.

use crate::analysis::Mineral;
use crate::compose::{Materializer, Span};
use crate::registry::{BlockId, Registry};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The most blocks a vein may hold.
pub const MAX_SIZE: u16 = 64;
/// The most veins a band may start in a chunk.
pub const MAX_VEINS: f32 = 200.0;

/// A band of heights where veins of one ore start: few at its ends, the
/// most over its peak — a trapezoid, a triangle when the peak is one
/// height, an even spread when it is the whole band.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    /// The lowest and the highest height a vein starts at.
    pub min: i32,
    pub max: i32,
    /// Where veins are the most common, from `peak[0]` up to `peak[1]`.
    pub peak: [i32; 2],
    /// Veins a chunk of 16 × 16 columns, on average.
    pub veins: f32,
    /// Blocks a vein.
    pub size: u16,
}

impl Band {
    pub const fn new(min: i32, peak: [i32; 2], max: i32, veins: f32, size: u16) -> Self {
        Self { min, max, peak, veins, size }
    }

    /// Its heights in order (min ≤ peak ≤ max) and its numbers within reach.
    pub fn tidy(&mut self) {
        if self.max < self.min {
            std::mem::swap(&mut self.min, &mut self.max);
        }
        if self.peak[1] < self.peak[0] {
            self.peak.swap(0, 1);
        }
        self.peak[0] = self.peak[0].clamp(self.min, self.max);
        self.peak[1] = self.peak[1].clamp(self.peak[0], self.max);
        self.veins = if self.veins.is_finite() { self.veins.clamp(0.0, MAX_VEINS) } else { 0.0 };
        self.size = self.size.clamp(1, MAX_SIZE);
    }

    /// How common veins are at height `y`, relatively: 1 over the peak,
    /// down toward its ends, 0 outside.
    pub fn weight(&self, y: i32) -> f64 {
        if y < self.min || y > self.max {
            return 0.0;
        }
        let [p0, p1] = self.peak;
        if y < p0 {
            (y - self.min + 1) as f64 / (p0 - self.min + 1) as f64
        } else if y > p1 {
            (self.max - y + 1) as f64 / (self.max - p1 + 1) as f64
        } else {
            1.0
        }
    }

    /// The sum of [`Band::weight`] over the band.
    fn total_weight(&self) -> f64 {
        (self.min..=self.max).map(|y| self.weight(y)).sum()
    }

    /// The share of its veins that start at height `y` (1 over the band).
    pub fn density(&self, y: i32) -> f64 {
        let total = self.total_weight();
        if total <= 0.0 { 0.0 } else { self.weight(y) / total }
    }

    /// How far its veins reach from where they start.
    pub fn reach(&self) -> i32 {
        reach(self.size)
    }
}

/// How far a vein of `size` blocks may reach from where it starts.
fn reach(size: u16) -> i32 {
    ((size.max(1) as f64).cbrt().ceil() as i32 + 1).min(6)
}

/// One kind of ore: its bands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OreRule {
    pub mineral: Mineral,
    #[serde(default = "yes")]
    pub on: bool,
    pub bands: Vec<Band>,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

/// The ores the export lays, saved with the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OreSettings {
    /// Whether the export lays ores at all.
    pub on: bool,
    /// Every band's veins, times this.
    #[serde(default = "one")]
    pub richness: f32,
    pub ores: Vec<OreRule>,
}

impl Default for OreSettings {
    fn default() -> Self {
        Preset::Modern.settings()
    }
}

/// Ores to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// As Minecraft lays them since 1.18 (Caves & Cliffs): from −64 to 320.
    Modern,
    /// As before 1.18, from 0 to 128: diamonds and redstone at the bottom.
    Classic,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::Modern, Preset::Classic];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Modern => "Minecraft 1.18+",
            Preset::Classic => "Before 1.18",
        }
    }

    pub fn settings(self) -> OreSettings {
        use Mineral::*;
        let rule = |mineral: Mineral, bands: &[Band]| OreRule { mineral, on: true, bands: bands.to_vec() };
        let ores = match self {
            // Close to the game's own: its veins, sizes and heights, the
            // numbers set so that each ore's share of the rock at each
            // height comes out as in worlds the game made.
            Preset::Modern => vec![
                rule(Coal, &[Band::new(0, [96, 96], 192, 20.0, 17), Band::new(136, [136, 320], 320, 30.0, 17)]),
                rule(Iron, &[Band::new(-24, [16, 16], 56, 7.0, 9), Band::new(80, [232, 232], 384, 35.0, 9), Band::new(-64, [-64, 72], 72, 10.0, 4)]),
                rule(Copper, &[Band::new(-16, [48, 48], 112, 16.0, 10)]),
                rule(Gold, &[Band::new(-64, [-16, -16], 32, 2.5, 9), Band::new(-64, [-64, -48], -48, 0.5, 9)]),
                rule(Redstone, &[Band::new(-64, [-64, 15], 15, 2.5, 8), Band::new(-96, [-64, -64], -32, 6.0, 8)]),
                rule(Lapis, &[Band::new(-32, [0, 0], 32, 2.0, 7), Band::new(-64, [-64, 64], 64, 2.5, 7)]),
                rule(Diamond, &[Band::new(-144, [-64, -64], 16, 8.0, 4)]),
                rule(Emerald, &[Band::new(80, [232, 232], 320, 12.0, 3)]),
            ],
            Preset::Classic => vec![
                rule(Coal, &[Band::new(0, [0, 128], 128, 20.0, 17)]),
                rule(Iron, &[Band::new(0, [0, 64], 64, 20.0, 9)]),
                rule(Copper, &[Band::new(0, [48, 48], 96, 6.0, 10)]),
                rule(Gold, &[Band::new(0, [0, 32], 32, 2.0, 9)]),
                rule(Redstone, &[Band::new(0, [0, 16], 16, 8.0, 8)]),
                rule(Lapis, &[Band::new(0, [16, 16], 32, 1.0, 7)]),
                rule(Diamond, &[Band::new(0, [0, 16], 16, 1.0, 8)]),
                rule(Emerald, &[Band::new(4, [4, 32], 32, 6.0, 1)]),
            ],
        };
        OreSettings { on: true, richness: 1.0, ores }
    }
}

impl OreSettings {
    /// The preset these settings are, if they are one (richness aside).
    pub fn preset(&self) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.settings().ores == self.ores)
    }

    /// The ore blocks expected at height `y` of a chunk all of rock: veins
    /// times blocks a vein times the share of veins starting there.
    pub fn expected(&self, rule: &OreRule, y: i32) -> f64 {
        if !self.on || !rule.on {
            return 0.0;
        }
        rule.bands.iter().map(|b| b.veins as f64 * b.size as f64 * b.density(y)).sum::<f64>() * self.richness.max(0.0) as f64
    }

    /// Ore blocks a chunk, about, where it is all rock.
    pub fn per_chunk(&self, rule: &OreRule) -> f64 {
        if !self.on || !rule.on {
            return 0.0;
        }
        rule.bands.iter().map(|b| b.veins as f64 * b.size as f64).sum::<f64>() * self.richness.max(0.0) as f64
    }

    /// Keep every band tidy and the richness within reach.
    pub fn tidy(&mut self) {
        self.richness = if self.richness.is_finite() { self.richness.clamp(0.0, 10.0) } else { 1.0 };
        for rule in &mut self.ores {
            for band in &mut rule.bands {
                band.tidy();
            }
        }
    }
}

/// The two blocks of an ore: in stone, in deepslate.
pub fn ore_blocks(m: Mineral) -> (&'static str, &'static str) {
    match m {
        Mineral::Coal => ("minecraft:coal_ore", "minecraft:deepslate_coal_ore"),
        Mineral::Iron => ("minecraft:iron_ore", "minecraft:deepslate_iron_ore"),
        Mineral::Copper => ("minecraft:copper_ore", "minecraft:deepslate_copper_ore"),
        Mineral::Gold => ("minecraft:gold_ore", "minecraft:deepslate_gold_ore"),
        Mineral::Redstone => ("minecraft:redstone_ore[lit=false]", "minecraft:deepslate_redstone_ore[lit=false]"),
        Mineral::Lapis => ("minecraft:lapis_ore", "minecraft:deepslate_lapis_ore"),
        Mineral::Diamond => ("minecraft:diamond_ore", "minecraft:deepslate_diamond_ore"),
        Mineral::Emerald => ("minecraft:emerald_ore", "minecraft:deepslate_emerald_ore"),
        Mineral::Quartz => ("minecraft:nether_quartz_ore", "minecraft:nether_quartz_ore"),
        Mineral::Netherite => ("minecraft:ancient_debris", "minecraft:ancient_debris"),
    }
}

/// The rock an ore takes the place of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Host {
    /// Stone, granite, diorite, andesite: the ore's own block.
    Stone,
    /// Deepslate and tuff: the ore's deepslate block.
    Deepslate,
}

/// What a block (`minecraft:deepslate[axis=y]`) is to an ore.
pub fn host_of(state: &str) -> Option<Host> {
    let name = state.split('[').next().unwrap_or(state);
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "stone" | "granite" | "diorite" | "andesite" => Some(Host::Stone),
        "deepslate" | "tuff" => Some(Host::Deepslate),
        _ => None,
    }
}

/// One block of a vein: where, and the ore (its index in
/// [`OreSettings::ores`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OreBlock {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub ore: u16,
}

/// A band ready to lay: its heights as a running sum to draw from.
#[derive(Debug, Clone)]
struct Ready {
    ore: u16,
    min: i32,
    /// The running sum of the weights from `min` up.
    sums: Vec<f64>,
    veins: f64,
    size: u16,
    reach: i32,
}

impl Ready {
    /// A height drawn from the band's spread.
    fn height(&self, r: u64) -> i32 {
        let total = *self.sums.last().unwrap_or(&0.0);
        let u = (r >> 11) as f64 / (1u64 << 53) as f64 * total;
        self.min + self.sums.partition_point(|&s| s <= u).min(self.sums.len().saturating_sub(1)) as i32
    }
}

/// The ores of a map, ready to lay chunk by chunk.
#[derive(Debug, Clone)]
pub struct Placer {
    seed: u64,
    bands: Vec<Ready>,
}

const DIRECTIONS: [[i8; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A small generator of numbers (SplitMix64).
struct Rng(u64);

impl Rng {
    #[inline]
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }
}

impl Placer {
    /// The ores of `settings` for a world of seed `seed`; `None` when there
    /// is nothing to lay.
    pub fn new(settings: &OreSettings, seed: i64) -> Option<Self> {
        if !settings.on {
            return None;
        }
        let richness = if settings.richness.is_finite() { settings.richness.clamp(0.0, 10.0) as f64 } else { 1.0 };
        let mut bands = Vec::new();
        for (k, rule) in settings.ores.iter().enumerate() {
            if !rule.on {
                continue;
            }
            for band in &rule.bands {
                let mut band = *band;
                band.tidy();
                let veins = band.veins as f64 * richness;
                if veins <= 0.0 {
                    continue;
                }
                let mut sum = 0.0;
                let sums = (band.min..=band.max)
                    .map(|y| {
                        sum += band.weight(y);
                        sum
                    })
                    .collect();
                bands.push(Ready { ore: k as u16, min: band.min, sums, veins, size: band.size, reach: band.reach() });
            }
        }
        (!bands.is_empty()).then_some(Self { seed: seed as u64, bands })
    }

    /// Every block the veins put in chunk `(cx, cz)` (in chunks), before the
    /// rock is asked, in a fixed order: the veins started in it and in the
    /// eight around it that reach into it, chunk by chunk, band by band.
    pub fn chunk(&self, cx: i32, cz: i32, mut put: impl FnMut(OreBlock)) {
        let (x0, z0) = (cx * 16, cz * 16);
        let mut vein: Vec<[i8; 3]> = Vec::with_capacity(MAX_SIZE as usize);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (sx, sz) = (cx + dx, cz + dz);
                let key = ((sx as u32 as u64) << 32) | sz as u32 as u64;
                for (b, band) in self.bands.iter().enumerate() {
                    let mut rng = Rng(mix(self.seed ^ mix(key ^ (b as u64).wrapping_mul(0x2545_f491_4f6c_dd1d))));
                    let whole = band.veins.floor();
                    let draw = (rng.next() >> 11) as f64 / (1u64 << 53) as f64;
                    let count = whole as u32 + (draw < band.veins - whole) as u32;
                    for _ in 0..count {
                        let at = rng.next();
                        let (x, z) = (sx * 16 + (at & 15) as i32, sz * 16 + ((at >> 4) & 15) as i32);
                        let y = band.height(rng.next());
                        let own = rng.next();
                        let r = band.reach;
                        // Too far to reach this chunk: not grown at all.
                        if x + r < x0 || x - r >= x0 + 16 || z + r < z0 || z - r >= z0 + 16 {
                            continue;
                        }
                        grow(&mut vein, band.size, own, r);
                        for o in &vein {
                            let (bx, by, bz) = (x + o[0] as i32, y + o[1] as i32, z + o[2] as i32);
                            if (x0..x0 + 16).contains(&bx) && (z0..z0 + 16).contains(&bz) {
                                put(OreBlock { x: bx, y: by, z: bz, ore: band.ore });
                            }
                        }
                    }
                }
            }
        }
    }

    /// [`Placer::chunk`] over rock that does not change as veins are laid:
    /// `rock` says what each block offers an ore (`None` where it takes
    /// nothing, or the block is in the open; see [`rock_at`]), and the first
    /// vein to reach a block keeps it — as the export lays them.
    pub fn chunk_over<R>(&self, cx: i32, cz: i32, mut rock: impl FnMut(i32, i32, i32) -> Option<R>, mut put: impl FnMut(OreBlock, R)) {
        let mut taken: HashSet<(i32, i32, i32)> = HashSet::new();
        self.chunk(cx, cz, |b| {
            if let Some(r) = rock(b.x, b.y, b.z)
                && taken.insert((b.x, b.y, b.z))
            {
                put(b, r);
            }
        });
    }
}

/// Grow a vein of `size` blocks from where it starts: a block at a time
/// beside one it already has, within `reach` of the start.
fn grow(out: &mut Vec<[i8; 3]>, size: u16, seed: u64, reach: i32) {
    out.clear();
    out.push([0, 0, 0]);
    let mut rng = Rng(seed);
    let want = size.clamp(1, MAX_SIZE) as usize;
    let mut tries = 0;
    while out.len() < want && tries < want * 6 {
        tries += 1;
        let r = rng.next();
        let from = out[(r % out.len() as u64) as usize];
        let d = DIRECTIONS[((r >> 40) % 6) as usize];
        let next = [from[0] + d[0], from[1] + d[1], from[2] + d[2]];
        if next.iter().any(|&c| (c as i32).abs() > reach) || out.contains(&next) {
            continue;
        }
        out.push(next);
    }
}

/// What the blocks of a registry are to an ore, by id.
pub fn hosts(reg: &Registry) -> Vec<Option<Host>> {
    reg.blocks.iter().map(|b| host_of(&b.state)).collect()
}

/// What the column whose solid runs are `spans` offers an ore at height
/// `y`: the rock there and what it is to an ore, if an ore takes its place
/// and solid ground lies right above it. `hosts` is [`hosts`] of the
/// registry.
pub fn rock_at(spans: &[Span], mat: &Materializer, hosts: &[Option<Host>], x: i32, y: i32, z: i32, max_y: i32) -> Option<(BlockId, Host)> {
    let span = spans.iter().find(|s| s.y0 <= y && y < s.y1)?;
    if y + 1 >= max_y || !spans.iter().any(|s| s.y0 <= y + 1 && y + 1 < s.y1) {
        return None;
    }
    let recipe = mat.recipe_depth(span.terrain, x, z);
    let split = if recipe >= (span.y1 - span.y0) as u32 { span.y0 } else { span.y1 - recipe as i32 };
    let block = if y < split { mat.substrate(x, y, z) } else { mat.block(span, x, y, z, false) };
    hosts.get(block.index()).copied().flatten().map(|h| (block, h))
}

/// The rock and the solid ground of a map at each height, from `min_y`:
/// how many columns hold rock an ore may take there — the stone and
/// deepslate under the terrains' layers, and layers of such rock, but not
/// the top block of a run nor the bedrock floor —, and how many hold
/// anything solid. Read a column in sixteen and scaled up: it is for the
/// amounts the Ores window expects, not a count.
#[derive(Debug, Clone, PartialEq)]
pub struct RockProfile {
    pub min_y: i32,
    pub rock: Vec<f64>,
    pub solid: Vec<f64>,
    /// Columns of the map.
    pub columns: u64,
}

impl RockProfile {
    /// Rock at height `y`, in columns.
    pub fn rock_at(&self, y: i32) -> f64 {
        self.rock.get((y - self.min_y) as usize).copied().unwrap_or(0.0)
    }

    pub fn solid_at(&self, y: i32) -> f64 {
        self.solid.get((y - self.min_y) as usize).copied().unwrap_or(0.0)
    }
}

/// [`RockProfile`] of dimension `dim` of `world`; `None` once `cancel` is
/// set, or for a map without ground.
pub fn rock_profile(world: &crate::world::World, dim: usize, cancel: &std::sync::atomic::AtomicBool) -> Option<RockProfile> {
    use rayon::prelude::*;
    use std::sync::atomic::Ordering;
    const STEP: usize = 4;
    let d = world.dims.get(dim)?;
    let reg = &world.registry;
    let tiles: Vec<crate::coords::TilePos> = d.ground().positions().collect();
    if tiles.is_empty() {
        return None;
    }
    let mat = Materializer::new(reg, world.meta.seed, d.min_y);
    let hosts = hosts(reg);
    // Terrains whose layers are all rock an ore takes.
    let rocky: Vec<bool> = reg.terrains.iter().map(|t| !t.strata.is_empty() && t.strata.iter().all(|s| s.blocks.iter().all(|b| hosts.get(b.0.index()).copied().flatten().is_some()))).collect();
    let (min_y, max_y) = (d.min_y, d.max_y);
    let h = (max_y - min_y).max(1) as usize;
    let floor = min_y + 5;
    let (rock, solid) = tiles
        .par_iter()
        .fold(
            || (vec![0i64; h + 1], vec![0i64; h + 1], Vec::new()),
            |(mut rock, mut solid, mut spans), &pos| {
                if cancel.load(Ordering::Relaxed) {
                    return (rock, solid, spans);
                }
                let Some(stack) = crate::compose::TileStack::new(d, pos, true) else { return (rock, solid, spans) };
                let (ox, oz) = pos.origin();
                let add = |list: &mut Vec<i64>, y0: i32, y1: i32| {
                    let (a, b) = ((y0.max(min_y) - min_y) as usize, (y1.min(max_y) - min_y).max(0) as usize);
                    if a < b {
                        list[a] += 1;
                        list[b] -= 1;
                    }
                };
                for lz in (0..128).step_by(STEP) {
                    for lx in (0..128).step_by(STEP) {
                        let cell = crate::coords::cell_index(lx, lz);
                        stack.spans(cell, &mut spans);
                        for s in &spans {
                            add(&mut solid, s.y0, s.y1);
                            let (x, z) = (ox + lx as i32, oz + lz as i32);
                            let recipe = mat.recipe_depth(s.terrain, x, z);
                            let split = if recipe >= (s.y1 - s.y0) as u32 { s.y0 } else { s.y1 - recipe as i32 };
                            let top = if rocky.get(s.terrain.index()).copied().unwrap_or(false) { s.y1 } else { split };
                            add(&mut rock, s.y0.max(floor), top.min(s.y1 - 1));
                        }
                    }
                }
                (rock, solid, spans)
            },
        )
        .map(|(r, s, _)| (r, s))
        .reduce(
            || (vec![0i64; h + 1], vec![0i64; h + 1]),
            |(mut a, mut b), (c, d)| {
                for (x, y) in a.iter_mut().zip(c) {
                    *x += y;
                }
                for (x, y) in b.iter_mut().zip(d) {
                    *x += y;
                }
                (a, b)
            },
        );
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let scale = (STEP * STEP) as f64;
    let running = |d: Vec<i64>| {
        let mut sum = 0i64;
        d.iter().take(h).map(|v| {
            sum += v;
            sum as f64 * scale
        }).collect::<Vec<f64>>()
    };
    Some(RockProfile { min_y, rock: running(rock), solid: running(solid), columns: (tiles.len() * crate::coords::TILE_AREA) as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same chunk always gets the same veins; another seed, others.
    #[test]
    fn veins_come_from_the_seed() {
        let settings = OreSettings::default();
        let list = |seed: i64, cx: i32, cz: i32| {
            let placer = Placer::new(&settings, seed).unwrap();
            let mut out = Vec::new();
            placer.chunk(cx, cz, |b| out.push(b));
            out
        };
        let a = list(7, 3, -2);
        assert!(a.len() > 500, "{} blocks", a.len());
        assert_eq!(a, list(7, 3, -2));
        assert_ne!(a, list(8, 3, -2));
        assert!(a.iter().all(|b| (48..64).contains(&b.x) && (-32..-16).contains(&b.z)), "inside the chunk");
    }

    /// A vein that starts near a chunk's edge reaches into its neighbour,
    /// whole: the blocks two chunks get of it are those of one vein.
    #[test]
    fn veins_cross_chunk_edges() {
        let settings = OreSettings {
            on: true,
            richness: 1.0,
            ores: vec![OreRule { mineral: Mineral::Coal, on: true, bands: vec![Band::new(40, [40, 40], 40, 40.0, 30)] }],
        };
        let placer = Placer::new(&settings, 1).unwrap();
        let mut got = HashSet::new();
        for cx in -2..2 {
            for cz in -2..2 {
                placer.chunk(cx, cz, |b| {
                    got.insert((b.x, b.y, b.z));
                });
            }
        }
        // Blocks right on both sides of the edge x = 0.
        let across = got.iter().filter(|b| b.0 == -1 && got.contains(&(0, b.1, b.2))).count();
        assert!(across > 20, "{across} blocks meet across the edge");
        // Veins grow around their start: never farther than their reach.
        assert!(got.iter().all(|b| (b.1 - 40).abs() <= reach(30)));
    }

    /// Heights are drawn as the band says: none outside it, the most over
    /// its peak.
    #[test]
    fn heights_follow_the_band() {
        let band = Band::new(-64, [0, 0], 64, 1.0, 1);
        let mut sum = 0.0;
        let ready = Ready {
            ore: 0,
            min: band.min,
            sums: (band.min..=band.max)
                .map(|y| {
                    sum += band.weight(y);
                    sum
                })
                .collect(),
            veins: 1.0,
            size: 1,
            reach: 1,
        };
        let mut rng = Rng(5);
        let mut counts = std::collections::HashMap::new();
        for _ in 0..40_000 {
            let y = ready.height(rng.next());
            assert!((-64..=64).contains(&y));
            *counts.entry(y / 16).or_insert(0) += 1;
        }
        assert!(counts[&0] > counts[&-3] * 2 && counts[&0] > counts[&3] * 2, "{counts:?}");
        assert!((band.density(0) - 1.0 / 65.0).abs() < 1e-3);
        let total: f64 = (-64..=64).map(|y| band.density(y)).sum();
        assert!((total - 1.0).abs() < 1e-9);
    }

    /// The rock of a generated map: none above its land, most under it,
    /// none in the bedrock floor.
    #[test]
    fn the_rock_of_a_map() {
        let world = crate::generate::generate(&crate::generate::GenParams { tiles_x: 1, tiles_z: 1, ..Default::default() });
        let p = rock_profile(&world, 0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!(p.columns, 16384);
        assert_eq!(p.rock_at(-64), 0.0, "bedrock");
        assert!((p.rock_at(-30) - 16384.0).abs() < 1.0, "all rock deep down: {}", p.rock_at(-30));
        assert!((p.solid_at(-30) - 16384.0).abs() < 1.0);
        assert_eq!(p.rock_at(300), 0.0, "air up high");
    }

    #[test]
    fn bands_stay_tidy() {
        let mut b = Band::new(50, [80, -10], 10, -3.0, 500);
        b.tidy();
        assert_eq!((b.min, b.peak, b.max, b.veins, b.size), (10, [10, 50], 50, 0.0, MAX_SIZE));
        assert_eq!(host_of("minecraft:deepslate[axis=y]"), Some(Host::Deepslate));
        assert_eq!(host_of("minecraft:dirt"), None);
        assert_eq!(OreSettings::default().preset(), Some(Preset::Modern));
    }
}
