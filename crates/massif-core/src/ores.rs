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
//! window all ask a [`Placer`], and agree. An ore may keep to some biomes
//! (or out of some): its veins are drawn all the same, and their blocks
//! laid only where the biome the export writes lets it in, so that the
//! other ores' veins stay where they were.

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

/// One kind of ore: its bands, and the biomes it keeps to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OreRule {
    /// The game's ore it is; for an ore of any block (see `custom`), a
    /// mineral the rule does not use.
    pub mineral: Mineral,
    #[serde(default = "yes")]
    pub on: bool,
    pub bands: Vec<Band>,
    #[serde(default, skip_serializing_if = "BiomeChoice::is_everywhere")]
    pub biomes: BiomeChoice,
    /// An ore of any block, a mod's (`create:zinc_ore`) or the game's,
    /// instead of the mineral's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomOre>,
}

/// The blocks of an ore of the map maker's choosing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomOre {
    /// The block it lays in stone, granite, diorite and andesite
    /// (`create:zinc_ore`, properties allowed).
    pub block: String,
    /// The one it lays in deepslate and tuff, if not the same
    /// (`create:deepslate_zinc_ore`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepslate: Option<String>,
    /// The colour of its specks in the Ores window and the survival check.
    #[serde(default = "grey")]
    pub color: [u8; 3],
}

fn grey() -> [u8; 3] {
    [160, 160, 160]
}

impl OreRule {
    pub fn new(mineral: Mineral, bands: &[Band]) -> Self {
        Self { mineral, on: true, bands: bands.to_vec(), biomes: BiomeChoice::default(), custom: None }
    }

    /// An ore of block `block` (and `deepslate` in deepslate and tuff), in
    /// one band as common as iron's around y 16 by default.
    pub fn custom(block: &str, deepslate: Option<&str>, color: [u8; 3]) -> Self {
        let custom = CustomOre { block: block.to_owned(), deepslate: deepslate.filter(|d| *d != block).map(str::to_owned), color };
        Self { mineral: Mineral::Iron, on: true, bands: vec![Band::new(-24, [16, 16], 56, 6.0, 8)], biomes: BiomeChoice::default(), custom: Some(custom) }
    }

    /// The game's ore it is, unless it is one of any block.
    pub fn vanilla(&self) -> Option<Mineral> {
        self.custom.is_none().then_some(self.mineral)
    }

    /// The blocks it lays: in stone, in deepslate.
    pub fn blocks(&self) -> (&str, &str) {
        match &self.custom {
            Some(c) => (c.block.as_str(), c.deepslate.as_deref().unwrap_or(c.block.as_str())),
            None => ore_blocks(self.mineral),
        }
    }

    /// The colour of its specks.
    pub fn color(&self) -> [u8; 3] {
        self.custom.as_ref().map_or(self.mineral.color(), |c| c.color)
    }

    /// Its name as people read it: "Diamond", "Zinc Ore".
    pub fn name(&self) -> String {
        match &self.custom {
            Some(c) => crate::analysis::pretty(c.block.split('[').next().unwrap_or(&c.block)),
            None => self.mineral.name().to_owned(),
        }
    }

    /// What tells it from the other ores: its mineral, or its block.
    pub fn key(&self) -> String {
        match &self.custom {
            Some(c) => c.block.clone(),
            None => format!("{:?}", self.mineral),
        }
    }
}

/// The biomes an ore is laid in (by their ids in the registry): only those
/// of `only` while it is set, never those of `except` while it is set — as
/// a brush keeps to what is on the map. Both unset: every biome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BiomeChoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub except: Option<Vec<u16>>,
}

impl BiomeChoice {
    /// Neither list set: the ore goes in every biome.
    pub fn is_everywhere(&self) -> bool {
        self.only.is_none() && self.except.is_none()
    }

    /// Whether the ore goes in biome `biome`.
    pub fn allows(&self, biome: u16) -> bool {
        self.only.as_ref().is_none_or(|l| l.contains(&biome)) && !self.except.as_ref().is_some_and(|l| l.contains(&biome))
    }
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
    /// The game's large veins since 1.18 (see [`Veins`]): copper in granite,
    /// iron in tuff, with blocks of raw metal. Off in maps made before them.
    #[serde(default)]
    pub veins: bool,
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

    /// The preset's ore of `mineral`, if it has one.
    pub fn rule(self, mineral: Mineral) -> Option<OreRule> {
        self.settings().ores.into_iter().find(|r| r.vanilla() == Some(mineral))
    }

    /// The preset for a world whose floor is at `min_y`: the game's since
    /// 1.18 where it goes under 0, the older one otherwise.
    pub fn for_floor(min_y: i32) -> Self {
        if min_y < 0 { Preset::Modern } else { Preset::Classic }
    }

    pub fn settings(self) -> OreSettings {
        use Mineral::*;
        let rule = OreRule::new;
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
        OreSettings { on: true, richness: 1.0, ores, veins: self == Preset::Modern }
    }
}

impl OreSettings {
    /// The preset these settings are, if they are one (richness aside).
    pub fn preset(&self) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.settings().ores == self.ores)
    }

    /// The ores of any block among them.
    pub fn custom(&self) -> impl Iterator<Item = &OreRule> {
        self.ores.iter().filter(|r| r.custom.is_some())
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

    /// The ores of the presets these settings have none of, in the
    /// presets' order.
    pub fn missing(&self) -> Vec<Mineral> {
        Preset::Modern.settings().ores.into_iter().map(|r| r.mineral).filter(|m| !self.ores.iter().any(|r| r.vanilla() == Some(*m))).collect()
    }

    /// Put `rule` among the ores, in the presets' order (coal first,
    /// emeralds last, the ores of any block after them); its index.
    pub fn insert(&mut self, rule: OreRule) -> usize {
        let order: Vec<Mineral> = Preset::Modern.settings().ores.into_iter().map(|r| r.mineral).collect();
        let rank = |r: &OreRule| r.vanilla().map_or(order.len() + 1, |m| order.iter().position(|&o| o == m).unwrap_or(order.len()));
        let at = self.ores.iter().position(|r| rank(r) > rank(&rule)).unwrap_or(self.ores.len());
        self.ores.insert(at, rule);
        at
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
    /// Of each ore of the settings, the biomes it is laid in.
    biomes: Vec<BiomeChoice>,
    /// Whether some ore keeps to some biomes.
    keeps: bool,
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
        let biomes: Vec<BiomeChoice> = settings.ores.iter().map(|r| r.biomes.clone()).collect();
        let keeps = settings.ores.iter().any(|r| r.on && !r.biomes.is_everywhere());
        (!bands.is_empty()).then_some(Self { seed: seed as u64, bands, biomes, keeps })
    }

    /// Whether some ore keeps to some biomes, or out of some: then where
    /// its blocks go depends on the biome there (see [`Placer::allows`]).
    pub fn keeps_to_biomes(&self) -> bool {
        self.keeps
    }

    /// Whether ore `ore` (its index in [`OreSettings::ores`]) is laid in
    /// biome `biome`.
    #[inline]
    pub fn allows(&self, ore: u16, biome: u16) -> bool {
        self.biomes.get(ore as usize).is_none_or(|c| c.allows(biome))
    }

    /// Lay the blocks of a chunk's veins (see [`Placer::chunks`]) over rock
    /// that does not change as they are laid: `rock` says what each block
    /// offers an ore (`None` where it takes nothing, or the block is in the
    /// open; see [`rock_at`]), `biome` the biome of the block (asked only
    /// for an ore that keeps to some; see
    /// [`crate::compose::BiomeColumn::of_block`]), and the first vein to
    /// reach a block it may take keeps it — as the export lays them.
    pub fn lay<R>(&self, blocks: &[OreBlock], mut rock: impl FnMut(i32, i32, i32) -> Option<R>, mut biome: impl FnMut(i32, i32, i32) -> u16, mut put: impl FnMut(OreBlock, R)) {
        let mut taken: HashSet<(i32, i32, i32)> = HashSet::with_capacity(blocks.len());
        for &b in blocks {
            if let Some(r) = rock(b.x, b.y, b.z)
                && (!self.keeps || self.biomes[b.ore as usize].is_everywhere() || self.allows(b.ore, biome(b.x, b.y, b.z)))
                && taken.insert((b.x, b.y, b.z))
            {
                put(b, r);
            }
        }
    }

    /// Every block the veins put in chunk `(cx, cz)` (in chunks), before the
    /// rock is asked, in a fixed order: the veins started in it and in the
    /// eight around it that reach into it, chunk by chunk, band by band.
    pub fn chunk(&self, cx: i32, cz: i32, mut put: impl FnMut(OreBlock)) {
        for b in self.chunks(cx, cz, 1, 1, (i32::MIN, i32::MAX)).swap_remove(0) {
            put(b);
        }
    }

    /// [`Placer::chunk`] for each chunk of the `nx` × `nz` chunks whose first
    /// is `(cx0, cz0)`, row by row — each vein drawn and grown once, however
    /// many of the chunks it reaches: the export and the survival check ask
    /// a tile's chunks at once. Veins that cannot reach a height from
    /// `heights.0` to `heights.1` are not grown: where nothing solid lies
    /// outside those heights, the ores laid are the same.
    pub fn chunks(&self, cx0: i32, cz0: i32, nx: i32, nz: i32, heights: (i32, i32)) -> Vec<Vec<OreBlock>> {
        let (nx, nz) = (nx.max(1), nz.max(1));
        let mut out = vec![Vec::new(); (nx * nz) as usize];
        let (x0, z0, x1, z1) = (cx0 * 16, cz0 * 16, (cx0 + nx) * 16, (cz0 + nz) * 16);
        let mut vein = Vein::default();
        // The chunks the veins start in, row by row: for each chunk asked,
        // its own and its eight neighbours come in the order a chunk asked
        // alone sees them.
        for sz in cz0 - 1..cz0 + nz + 1 {
            for sx in cx0 - 1..cx0 + nx + 1 {
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
                        // Too far to reach the chunks asked, or their solid
                        // ground (up in the sky, under the world): not grown.
                        if x + r < x0 || x - r >= x1 || z + r < z0 || z - r >= z1 || y + r < heights.0 || y - r > heights.1 {
                            continue;
                        }
                        vein.grow(band.size, own, r);
                        for o in &vein.blocks {
                            let (bx, by, bz) = (x + o[0] as i32, y + o[1] as i32, z + o[2] as i32);
                            if (x0..x1).contains(&bx) && (z0..z1).contains(&bz) {
                                let k = (((bz - z0) >> 4) * nx + ((bx - x0) >> 4)) as usize;
                                out[k].push(OreBlock { x: bx, y: by, z: bz, ore: band.ore });
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// [`Placer::lay`] over chunk `(cx, cz)`.
    pub fn chunk_over<R>(&self, cx: i32, cz: i32, rock: impl FnMut(i32, i32, i32) -> Option<R>, biome: impl FnMut(i32, i32, i32) -> u16, put: impl FnMut(OreBlock, R)) {
        self.lay(&self.chunks(cx, cz, 1, 1, (i32::MIN, i32::MAX))[0], rock, biome, put);
    }
}

/// A vein as it grows: its blocks from where it starts, and which of the
/// box around the start it has.
struct Vein {
    blocks: Vec<[i8; 3]>,
    has: [u64; 35],
}

impl Default for Vein {
    fn default() -> Self {
        Self { blocks: Vec::with_capacity(MAX_SIZE as usize), has: [0; 35] }
    }
}

/// The side of the box a vein grows in, its reach at most 6 either way.
const BOX: i32 = 13;

impl Vein {
    #[inline]
    fn bit(o: [i8; 3]) -> usize {
        ((o[0] as i32 + 6) + (o[1] as i32 + 6) * BOX + (o[2] as i32 + 6) * BOX * BOX) as usize
    }

    /// Grow `size` blocks from where it starts: a block at a time beside one
    /// it already has, within `reach` of the start.
    fn grow(&mut self, size: u16, seed: u64, reach: i32) {
        for &o in &self.blocks {
            let k = Self::bit(o);
            self.has[k >> 6] &= !(1 << (k & 63));
        }
        self.blocks.clear();
        self.blocks.push([0, 0, 0]);
        let k = Self::bit([0, 0, 0]);
        self.has[k >> 6] |= 1 << (k & 63);
        let mut rng = Rng(seed);
        let want = size.clamp(1, MAX_SIZE) as usize;
        let mut tries = 0;
        while self.blocks.len() < want && tries < want * 6 {
            tries += 1;
            let r = rng.next();
            let from = self.blocks[(r % self.blocks.len() as u64) as usize];
            let d = DIRECTIONS[((r >> 40) % 6) as usize];
            let next = [from[0] + d[0], from[1] + d[1], from[2] + d[2]];
            if next.iter().any(|&c| (c as i32).abs() > reach) {
                continue;
            }
            let k = Self::bit(next);
            if self.has[k >> 6] & (1 << (k & 63)) != 0 {
                continue;
            }
            self.has[k >> 6] |= 1 << (k & 63);
            self.blocks.push(next);
        }
    }
}

/// The lowest and highest heights the solid ground of a tile's stack may
/// hold, for [`Placer::chunks`]: from the floor of the world up to the
/// highest top of its levels (to the top of the world where it was sculpted).
/// What a large vein lays in a block (see [`Veins`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VeinBlock {
    CopperOre,
    RawCopper,
    Granite,
    IronOre,
    RawIron,
    Tuff,
}

impl VeinBlock {
    pub const ALL: [VeinBlock; 6] = [VeinBlock::CopperOre, VeinBlock::RawCopper, VeinBlock::Granite, VeinBlock::IronOre, VeinBlock::RawIron, VeinBlock::Tuff];

    /// Its block state.
    pub const fn state(self) -> &'static str {
        match self {
            VeinBlock::CopperOre => "minecraft:copper_ore",
            VeinBlock::RawCopper => "minecraft:raw_copper_block",
            VeinBlock::Granite => "minecraft:granite",
            VeinBlock::IronOre => "minecraft:deepslate_iron_ore",
            VeinBlock::RawIron => "minecraft:raw_iron_block",
            VeinBlock::Tuff => "minecraft:tuff",
        }
    }
}

/// The game's large ore veins since 1.18: where a slow noise rises far
/// enough from 0, ribbons where two finer noises are both near 0 — of
/// copper ore in granite between y 0 and 50 where the slow noise is over 0,
/// of iron ore in tuff between y −60 and −8 where it is under —, thinning
/// out toward the ends of their heights, an ore in a block of the ribbon
/// now and then (more toward the vein's heart), and one in fifty of those a
/// block of raw metal. As the game's: a vein takes the place of stone and
/// deepslate only.
pub struct Veins {
    seed: u64,
}

/// The heights of the copper veins and of the iron veins.
const COPPER: (i32, i32) = (0, 50);
const IRON: (i32, i32) = (-60, -8);

impl Veins {
    pub fn new(seed: i64) -> Self {
        Self { seed: seed as u64 ^ 0x0E5E_1E15 }
    }

    /// The heights veins may be at.
    pub const fn heights() -> (i32, i32) {
        (IRON.0, COPPER.1)
    }

    /// The slow noise: which metal, and whether a vein is there at all.
    #[inline]
    fn veininess(&self, x: f32, y: f32, z: f32) -> f32 {
        crate::caves::noise::gradient(self.seed, x / 170.0, y / 170.0, z / 170.0) * 1.6
    }

    /// The vein's block at `(x, y, z)` given the slow noise `v` there.
    #[inline]
    fn block(&self, x: i32, y: i32, z: i32, v: f32) -> Option<VeinBlock> {
        let copper = v > 0.0;
        let (lo, hi) = if copper { COPPER } else { IRON };
        if y < lo || y > hi {
            return None;
        }
        let edge = (y - lo).min(hi - y) as f32;
        let e = v.abs();
        if e + (-0.2 + 0.2 * (edge / 20.0).min(1.0)) < 0.4 {
            return None;
        }
        let r = |salt: u64| (crate::caves::noise::hash3(self.seed ^ salt, x, y, z) >> 40) as f32 / (1u64 << 24) as f32;
        if r(1) > 0.7 {
            return None;
        }
        let (fx, fy, fz) = (x as f32 / 32.0, y as f32 / 32.0, z as f32 / 32.0);
        let a = crate::caves::noise::gradient(self.seed ^ 0xA, fx, fy, fz);
        let b = crate::caves::noise::gradient(self.seed ^ 0xB, fx, fy, fz);
        if a.abs().max(b.abs()) - 0.08 >= 0.0 {
            return None;
        }
        let ore = 0.1 + 0.2 * ((e - 0.4) / 0.2).clamp(0.0, 1.0);
        let gap = crate::caves::noise::gradient(self.seed ^ 0x6A9, fx, fy, fz);
        Some(if r(2) < ore && gap > -0.3 {
            match (copper, r(3) < 0.02) {
                (true, false) => VeinBlock::CopperOre,
                (true, true) => VeinBlock::RawCopper,
                (false, false) => VeinBlock::IronOre,
                (false, true) => VeinBlock::RawIron,
            }
        } else if copper {
            VeinBlock::Granite
        } else {
            VeinBlock::Tuff
        })
    }

    /// The veins of the 16 × 16 columns from `(x0, z0)` between heights
    /// `y0..y1`, as `(x, y, z, block)`: the slow noise worked out every 4
    /// blocks and blended in between, the fine ones where it calls for a
    /// vein.
    pub fn chunk(&self, x0: i32, z0: i32, (y0, y1): (i32, i32), mut put: impl FnMut(i32, i32, i32, VeinBlock)) {
        let (vlo, vhi) = Self::heights();
        let (y0, y1) = (y0.max(vlo), y1.min(vhi + 1));
        if y1 <= y0 {
            return;
        }
        let ly0 = y0.div_euclid(4);
        let ny = ((y1 - 1).div_euclid(4) - ly0 + 2) as usize;
        let mut lattice = vec![0f32; 5 * 5 * ny];
        for j in 0..5 {
            for i in 0..5 {
                for k in 0..ny {
                    lattice[(j * 5 + i) * ny + k] = self.veininess((x0 + i as i32 * 4) as f32, ((ly0 + k as i32) * 4) as f32, (z0 + j as i32 * 4) as f32);
                }
            }
        }
        // The slow noise never reaches far from 0 in this chunk: no vein.
        if lattice.iter().all(|v| v.abs() < 0.3) {
            return;
        }
        for lz in 0..16 {
            for lx in 0..16 {
                let (ci, cj) = (lx as usize / 4, lz as usize / 4);
                let (fx, fz) = ((lx % 4) as f32 / 4.0, (lz % 4) as f32 / 4.0);
                for y in y0..y1 {
                    let t = (y as f32 / 4.0) - ly0 as f32;
                    let k = (t.floor() as usize).min(ny - 2);
                    let fy = t - k as f32;
                    let at = |i: usize, j: usize, k: usize| lattice[(j * 5 + i) * ny + k];
                    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
                    let v0 = lerp(lerp(at(ci, cj, k), at(ci + 1, cj, k), fx), lerp(at(ci, cj + 1, k), at(ci + 1, cj + 1, k), fx), fz);
                    let v1 = lerp(lerp(at(ci, cj, k + 1), at(ci + 1, cj, k + 1), fx), lerp(at(ci, cj + 1, k + 1), at(ci + 1, cj + 1, k + 1), fx), fz);
                    let v = lerp(v0, v1, fy);
                    if v.abs() < 0.2 {
                        continue;
                    }
                    if let Some(b) = self.block(x0 + lx, y, z0 + lz, v) {
                        put(x0 + lx, y, z0 + lz, b);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod vein_tests {
    use super::*;

    /// Copper veins in granite over y 0, iron in tuff under −8, with ores
    /// in them and a few raw blocks; nothing outside their heights.
    #[test]
    fn veins_keep_to_their_heights_and_metals() {
        let veins = Veins::new(42);
        let mut found: std::collections::HashMap<VeinBlock, usize> = std::collections::HashMap::new();
        for cz in 0..12 {
            for cx in 0..12 {
                veins.chunk(cx * 16, cz * 16, (-64, 120), |_, y, _, b| {
                    match b {
                        VeinBlock::CopperOre | VeinBlock::RawCopper | VeinBlock::Granite => assert!((0..=50).contains(&y), "{b:?} at {y}"),
                        _ => assert!((-60..=-8).contains(&y), "{b:?} at {y}"),
                    }
                    *found.entry(b).or_insert(0) += 1;
                });
            }
        }
        for b in [VeinBlock::CopperOre, VeinBlock::Granite, VeinBlock::IronOre, VeinBlock::Tuff] {
            assert!(found.get(&b).copied().unwrap_or(0) > 20, "{b:?}: {found:?}");
        }
        let ores = found.get(&VeinBlock::CopperOre).copied().unwrap_or(0) + found.get(&VeinBlock::IronOre).copied().unwrap_or(0);
        let fillers = found[&VeinBlock::Granite] + found[&VeinBlock::Tuff];
        assert!(ores * 2 < fillers, "mostly filler: {found:?}");
    }
}

pub fn stack_heights(stack: &crate::compose::TileStack, min_y: i32, max_y: i32) -> (i32, i32) {
    if stack.is_sculpted() {
        return (min_y, max_y);
    }
    let mut top = f32::MIN;
    for (_, _, tile) in stack.layers() {
        top = top.max(match tile.top.uniform_value() {
            Some(v) => v,
            None => tile.top.dense().iter().copied().fold(f32::MIN, f32::max),
        });
    }
    (min_y, (crate::compose::block_top(top) + 1).clamp(min_y, max_y))
}

/// What the blocks of a registry are to an ore, by id.
pub fn hosts(reg: &Registry) -> Vec<Option<Host>> {
    // The rock of the map's palette, whatever its blocks, takes ores.
    let wk = reg.well_known;
    reg.blocks
        .iter()
        .enumerate()
        .map(|(i, b)| match i {
            i if i == wk.stone.index() => Some(Host::Stone),
            i if i == wk.deepslate.index() => Some(Host::Deepslate),
            _ => host_of(&b.state),
        })
        .collect()
}

/// What the column whose solid runs are `spans` offers an ore at height
/// `y`: the rock there and what it is to an ore, if an ore takes its place
/// and solid ground lies right above it. `hosts` is [`hosts`] of the
/// registry; `pockets` the column's underground pockets, whose blocks take
/// the place of the rock (an ore takes theirs where they are rock too).
#[allow(clippy::too_many_arguments)]
pub fn rock_at(spans: &[Span], mat: &Materializer, hosts: &[Option<Host>], pockets: Option<&crate::pockets::Column>, x: i32, y: i32, z: i32, max_y: i32) -> Option<(BlockId, Host)> {
    let span = spans.iter().find(|s| s.y0 <= y && y < s.y1)?;
    if y + 1 >= max_y || !spans.iter().any(|s| s.y0 <= y + 1 && y + 1 < s.y1) {
        return None;
    }
    let recipe = mat.recipe_depth(span.terrain, x, z);
    let split = if recipe >= (span.y1 - span.y0) as u32 { span.y0 } else { span.y1 - recipe as i32 };
    let block = if y < split { pockets.and_then(|p| p.at(x, y, z)).unwrap_or_else(|| mat.substrate(x, y, z)) } else { mat.block(span, x, y, z, false) };
    hosts.get(block.index()).copied().flatten().map(|h| (block, h))
}

/// The rock and the solid ground of a map at each height, from `min_y`:
/// how many columns hold rock an ore may take there — the stone and
/// deepslate under the terrains' layers, and layers of such rock, but not
/// the top block of a run nor the bedrock floor —, and how many hold
/// anything solid. Read a column in sixteen (the middle of each 4 × 4,
/// where the export takes the biome) and scaled up: it is for the amounts
/// the Ores window expects, not a count.
#[derive(Debug, Clone, PartialEq)]
pub struct RockProfile {
    pub min_y: i32,
    pub rock: Vec<f64>,
    pub solid: Vec<f64>,
    /// Columns of the map.
    pub columns: u64,
    /// The rock of each biome (by id) at each height, as `rock`: empty for
    /// a biome the map has no rock in.
    pub by_biome: Vec<Vec<f64>>,
}

impl RockProfile {
    /// Rock at height `y`, in columns.
    pub fn rock_at(&self, y: i32) -> f64 {
        self.rock.get((y - self.min_y) as usize).copied().unwrap_or(0.0)
    }

    pub fn solid_at(&self, y: i32) -> f64 {
        self.solid.get((y - self.min_y) as usize).copied().unwrap_or(0.0)
    }

    /// Rock at height `y` in the biomes `choice` lets an ore into.
    pub fn rock_in(&self, choice: &BiomeChoice, y: i32) -> f64 {
        if choice.is_everywhere() {
            return self.rock_at(y);
        }
        let i = (y - self.min_y) as usize;
        self.by_biome.iter().enumerate().filter(|(b, _)| choice.allows(*b as u16)).map(|(_, rock)| rock.get(i).copied().unwrap_or(0.0)).sum()
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
    let auto = crate::compose::AutoBiomes::new(reg, world.meta.sea_level, d.min_y);
    // Terrains whose layers are all rock an ore takes.
    let rocky: Vec<bool> = reg.terrains.iter().map(|t| !t.strata.is_empty() && t.strata.iter().all(|s| s.blocks.iter().all(|b| hosts.get(b.0.index()).copied().flatten().is_some()))).collect();
    let (min_y, max_y) = (d.min_y, d.max_y);
    let h = (max_y - min_y).max(1) as usize;
    let floor = min_y + 5;
    // Rock, solid ground, the rock of each biome: the changes from one
    // height to the next.
    type Counts = (Vec<i64>, Vec<i64>, Vec<Vec<i64>>);
    let empty = || -> Counts { (vec![0i64; h + 1], vec![0i64; h + 1], Vec::new()) };
    let ((rock, solid, by_biome), _) = tiles
        .par_iter()
        .fold(
            || (empty(), (Vec::new(), crate::compose::BiomeColumn::default())),
            |((mut rock, mut solid, mut by_biome), (mut spans, mut biomes)), &pos| {
                if cancel.load(Ordering::Relaxed) {
                    return ((rock, solid, by_biome), (spans, biomes));
                }
                let Some(stack) = crate::compose::TileStack::new(d, pos, true) else { return ((rock, solid, by_biome), (spans, biomes)) };
                let (ox, oz) = pos.origin();
                let add = |list: &mut Vec<i64>, y0: i32, y1: i32| {
                    let (a, b) = ((y0.max(min_y) - min_y) as usize, (y1.min(max_y) - min_y).max(0) as usize);
                    if a < b {
                        list[a] += 1;
                        list[b] -= 1;
                    }
                };
                for lz in (STEP / 2..128).step_by(STEP) {
                    for lx in (STEP / 2..128).step_by(STEP) {
                        let cell = crate::coords::cell_index(lx, lz);
                        stack.spans(cell, &mut spans);
                        stack.biome_column(cell, reg, &auto, || spans.last().map(|s| (s.y1, s.terrain)), &mut biomes);
                        for s in &spans {
                            add(&mut solid, s.y0, s.y1);
                            let (x, z) = (ox + lx as i32, oz + lz as i32);
                            let recipe = mat.recipe_depth(s.terrain, x, z);
                            let split = if recipe >= (s.y1 - s.y0) as u32 { s.y0 } else { s.y1 - recipe as i32 };
                            let top = if rocky.get(s.terrain.index()).copied().unwrap_or(false) { s.y1 } else { split };
                            let (y0, y1) = (s.y0.max(floor), top.min(s.y1 - 1));
                            add(&mut rock, y0, y1);
                            // The rock of the biome it is in: block by
                            // block where a level over the ground has its own.
                            let mut of = |biome: u16, y0: i32, y1: i32| {
                                let k = biome as usize;
                                if by_biome.len() <= k {
                                    by_biome.resize(k + 1, Vec::new());
                                }
                                if by_biome[k].is_empty() {
                                    by_biome[k] = vec![0; h + 1];
                                }
                                add(&mut by_biome[k], y0, y1);
                            };
                            if biomes.overrides.is_empty() {
                                of(biomes.base, y0, y1);
                            } else {
                                for y in y0..y1 {
                                    of(biomes.of_block(y), y, y + 1);
                                }
                            }
                        }
                    }
                }
                ((rock, solid, by_biome), (spans, biomes))
            },
        )
        .reduce(
            || (empty(), (Vec::new(), crate::compose::BiomeColumn::default())),
            |((mut a, mut b, mut c), scratch), ((d, e, f), _)| {
                for (x, y) in a.iter_mut().zip(d) {
                    *x += y;
                }
                for (x, y) in b.iter_mut().zip(e) {
                    *x += y;
                }
                if c.len() < f.len() {
                    c.resize(f.len(), Vec::new());
                }
                for (x, y) in c.iter_mut().zip(f) {
                    if x.is_empty() {
                        *x = y;
                    } else {
                        for (v, w) in x.iter_mut().zip(y) {
                            *v += w;
                        }
                    }
                }
                ((a, b, c), scratch)
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
    let by_biome = by_biome.into_iter().map(|d| if d.is_empty() { Vec::new() } else { running(d) }).collect();
    Some(RockProfile { min_y, rock: running(rock), solid: running(solid), columns: (tiles.len() * crate::coords::TILE_AREA) as u64, by_biome })
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
            ores: vec![OreRule::new(Mineral::Coal, &[Band::new(40, [40, 40], 40, 40.0, 30)])],
            veins: false,
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

    /// A tile's chunks at once are the chunks one by one, block for block
    /// and in the same order.
    #[test]
    fn chunks_at_once_are_chunks_one_by_one() {
        let placer = Placer::new(&OreSettings::default(), 99).unwrap();
        let all = placer.chunks(-2, 5, 3, 2, (i32::MIN, i32::MAX));
        for (k, got) in all.iter().enumerate() {
            let (cx, cz) = (-2 + (k % 3) as i32, 5 + (k / 3) as i32);
            let mut alone = Vec::new();
            placer.chunk(cx, cz, |b| alone.push(b));
            assert_eq!(got, &alone, "chunk {cx} {cz}");
            assert!(!alone.is_empty());
        }
        // Bounded to some heights: the blocks there are the same, in order.
        let low = placer.chunks(-2, 5, 3, 2, (-64, 40));
        for (got, all) in low.iter().zip(&all) {
            let within = |b: &&OreBlock| (-64..=40).contains(&b.y);
            assert_eq!(got.iter().filter(within).collect::<Vec<_>>(), all.iter().filter(within).collect::<Vec<_>>());
            assert!(got.len() < all.len(), "fewer veins grown");
        }
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

    /// An ore kept to some biomes is laid only where the biome lets it in:
    /// its veins are drawn as before, the other ores' too, and a choice
    /// that lets every biome in lays every block as no choice does.
    #[test]
    fn ores_keep_to_their_biomes() {
        let all = OreSettings::default();
        let diamond = all.ores.iter().position(|o| o.mineral == Mineral::Diamond).unwrap();
        // West of x = 40 the biome is 5, east of it 9.
        let biome = |x: i32, _: i32, _: i32| if x < 40 { 5 } else { 9 };
        let lay = |settings: &OreSettings| {
            let placer = Placer::new(settings, 3).unwrap();
            let mut out = Vec::new();
            for chunk in placer.chunks(0, 0, 6, 2, (i32::MIN, i32::MAX)) {
                placer.lay(&chunk, |_, _, _| Some(()), biome, |b, _| out.push(b));
            }
            (placer.chunks(0, 0, 6, 2, (i32::MIN, i32::MAX)), out)
        };
        let (veins, every) = lay(&all);
        assert!(every.iter().any(|b| b.ore as usize == diamond && b.x < 40) && every.iter().any(|b| b.ore as usize == diamond && b.x >= 40));

        let mut open = all.clone();
        open.ores[diamond].biomes.except = Some(Vec::new());
        assert!(Placer::new(&open, 3).unwrap().keeps_to_biomes());
        assert_eq!(lay(&open), (veins.clone(), every.clone()), "a choice that lets every biome in changes nothing");

        let mut only = all.clone();
        only.ores[diamond].biomes.only = Some(vec![5]);
        let (drawn, laid) = lay(&only);
        assert_eq!(drawn, veins, "the veins are drawn as before");
        assert!(laid.iter().any(|b| b.ore as usize == diamond), "diamonds in biome 5");
        assert!(laid.iter().all(|b| b.ore as usize != diamond || b.x < 40), "no diamond in biome 9");
        let others = |list: &[OreBlock]| list.iter().filter(|b| b.ore as usize != diamond).count();
        assert!(others(&laid) >= others(&every), "the other ores keep their blocks, and get those diamonds gave up");

        let mut except = all.clone();
        except.ores[diamond].biomes.except = Some(vec![5]);
        let (_, laid) = lay(&except);
        assert!(laid.iter().any(|b| b.ore as usize == diamond) && laid.iter().all(|b| b.ore as usize != diamond || b.x >= 40), "diamonds out of biome 5");
        except.ores[diamond].biomes.only = Some(vec![5]);
        assert!(!lay(&except).1.iter().any(|b| b.ore as usize == diamond), "only in 5, but not in 5: nowhere");
    }

    /// Maps saved before biome choices load unchanged, and a rule without
    /// one is saved as before.
    #[test]
    fn biome_choices_are_saved_only_when_made() {
        let old = r#"{"mineral":"diamond","on":true,"bands":[{"min":-144,"max":16,"peak":[-64,-64],"veins":8.0,"size":4}]}"#;
        let rule: OreRule = serde_json::from_str(old).unwrap();
        assert!(rule.biomes.is_everywhere());
        assert_eq!(rule, Preset::Modern.rule(Mineral::Diamond).unwrap());
        assert_eq!(serde_json::to_string(&rule).unwrap(), old);
        let mut kept = rule.clone();
        kept.biomes.only = Some(vec![3, 7]);
        let json = serde_json::to_string(&kept).unwrap();
        assert!(json.ends_with(r#""biomes":{"only":[3,7]}}"#), "{json}");
        assert_eq!(serde_json::from_str::<OreRule>(&json).unwrap(), kept);
        let choice = BiomeChoice { only: Some(vec![3, 7]), except: Some(vec![7]) };
        assert!(choice.allows(3) && !choice.allows(7) && !choice.allows(4));
        assert!(BiomeChoice { only: None, except: Some(vec![]) }.allows(4));
        assert!(!BiomeChoice { only: Some(vec![]), except: None }.allows(4));
    }

    /// An ore removed comes back in its place among the others.
    #[test]
    fn ores_come_back_in_order() {
        let mut s = OreSettings::default();
        assert!(s.missing().is_empty());
        let gold = s.ores.remove(3);
        let iron = s.ores.remove(1);
        assert_eq!(s.missing(), vec![Mineral::Iron, Mineral::Gold]);
        assert_eq!(s.insert(gold), 2);
        assert_eq!(s.insert(iron), 1);
        assert_eq!(s, OreSettings::default());
        assert_eq!(Preset::for_floor(-64), Preset::Modern);
        assert_eq!(Preset::for_floor(0), Preset::Classic);
    }

    /// The rock of a map, biome by biome: the biomes' rock sums to the
    /// rock, and an ore kept to the biomes there has all of it.
    #[test]
    fn the_rock_of_each_biome() {
        let world = crate::generate::generate(&crate::generate::GenParams { tiles_x: 1, tiles_z: 1, ..Default::default() });
        let p = rock_profile(&world, 0, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let present: Vec<u16> = (0..p.by_biome.len()).filter(|&b| !p.by_biome[b].is_empty()).map(|b| b as u16).collect();
        assert!(!present.is_empty());
        for y in [-60, -30, 0, 40, 64] {
            let sum: f64 = p.by_biome.iter().map(|r| r.get((y - p.min_y) as usize).copied().unwrap_or(0.0)).sum();
            assert!((sum - p.rock_at(y)).abs() < 1e-6, "y {y}: {sum} of {}", p.rock_at(y));
            assert!((p.rock_in(&BiomeChoice { only: Some(present.clone()), except: None }, y) - p.rock_at(y)).abs() < 1e-6);
            assert_eq!(p.rock_in(&BiomeChoice { only: None, except: Some(present.clone()) }, y), 0.0);
        }
    }

    /// An ore of any block is saved with its blocks, comes after the
    /// game's, and does not stand for the mineral its rule carries.
    #[test]
    fn ores_of_any_block() {
        let mut s = OreSettings::default();
        let tin = OreRule::custom("examplemod:tin_ore", Some("examplemod:deepslate_tin_ore"), [1, 2, 3]);
        assert_eq!(s.insert(tin.clone()), 8, "after the game's");
        assert_eq!(s.insert(OreRule::custom("examplemod:lead_ore", Some("examplemod:lead_ore"), [0; 3])), 9);
        s.ores.remove(1);
        assert_eq!(s.missing(), vec![Mineral::Iron], "the tin is no iron");
        assert_eq!(s.insert(Preset::Modern.rule(Mineral::Iron).unwrap()), 1);
        assert_eq!((s.ores[9].blocks(), s.ores[9].name()), (("examplemod:lead_ore", "examplemod:lead_ore"), "Lead Ore".to_owned()));
        assert_eq!(tin.blocks(), ("examplemod:tin_ore", "examplemod:deepslate_tin_ore"));
        assert_eq!((tin.vanilla(), s.ores[1].vanilla()), (None, Some(Mineral::Iron)));
        let json = serde_json::to_string(&tin).unwrap();
        assert!(json.contains(r#""custom":{"block":"examplemod:tin_ore","deepslate":"examplemod:deepslate_tin_ore","color":[1,2,3]}"#), "{json}");
        assert_eq!(serde_json::from_str::<OreRule>(&json).unwrap(), tin);
        assert!(!serde_json::to_string(&OreSettings::default()).unwrap().contains("custom"), "the game's ores saved as before");
        assert_eq!(s.custom().count(), 2);
        assert_eq!(s.preset(), None);
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
