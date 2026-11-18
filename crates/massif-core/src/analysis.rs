//! A survey of a whole map, as a survival player will meet it.
//!
//! Every block the export writes is counted the way the export writes it —
//! the layers of the terrains, the rock under them, the bedrock floor, the
//! plants of the covers, the blocks of the objects — and the land measured:
//! its heights, the steps between columns a player walks, climbs or only
//! falls down, its water, its biomes, the land around the spawn point.
//! [`Analysis::findings`] reads that as a survival player would: what
//! breaks survival (riches lying around, bedrock in the way, holes into the
//! void, no wood), what is missing (ores, lava, water, grass) and what is
//! fine.
//!
//! The export writes the map's chunks finished, so Minecraft adds nothing
//! to them — no ore, no tree, no village: what the map holds is all a
//! player will find in it.

use crate::auto::Steps;
use crate::compose::{Materializer, Span, TileStack, hash2, stratum_thickness};
use crate::coords::{TILE_AREA, TILE_SIZE, TilePos, cell_index};
use crate::objects::{is_nothing, tile_placements};
use crate::registry::{BlockId, Registry, Stratum, TerrainId, is_soil, is_tall_plant, plant_color};
use crate::world::{Dimension, World};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// How far around the spawn point counts as near it, in blocks.
pub const SPAWN_REACH: i32 = 128;
/// A fall of this many blocks hurts (a fall does one point of damage a
/// block past the third).
pub const HURTING_FALL: i32 = 4;
/// A fall of this many blocks kills a player at full health, unarmoured.
pub const LETHAL_FALL: i32 = 23;
/// The layers at the bottom of the world where bedrock belongs.
pub const FLOOR: i32 = 5;

// ---------------------------------------------------------------- blocks --

/// What players mine: the families of ores, with the blocks they are
/// stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mineral {
    Coal,
    Iron,
    Copper,
    Gold,
    Redstone,
    Lapis,
    Diamond,
    Emerald,
    Quartz,
    Netherite,
}

impl Mineral {
    pub const ALL: [Mineral; 10] = [
        Mineral::Coal,
        Mineral::Iron,
        Mineral::Copper,
        Mineral::Gold,
        Mineral::Redstone,
        Mineral::Lapis,
        Mineral::Diamond,
        Mineral::Emerald,
        Mineral::Quartz,
        Mineral::Netherite,
    ];

    /// Those a survival player looks for in the overworld.
    pub const OVERWORLD: [Mineral; 8] = [
        Mineral::Coal,
        Mineral::Iron,
        Mineral::Copper,
        Mineral::Gold,
        Mineral::Redstone,
        Mineral::Lapis,
        Mineral::Diamond,
        Mineral::Emerald,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mineral::Coal => "Coal",
            Mineral::Iron => "Iron",
            Mineral::Copper => "Copper",
            Mineral::Gold => "Gold",
            Mineral::Redstone => "Redstone",
            Mineral::Lapis => "Lapis",
            Mineral::Diamond => "Diamond",
            Mineral::Emerald => "Emerald",
            Mineral::Quartz => "Quartz",
            Mineral::Netherite => "Netherite",
        }
    }

    /// What is mined, as players say it: "diamonds", "iron".
    pub fn plural(self) -> &'static str {
        match self {
            Mineral::Diamond => "diamonds",
            Mineral::Emerald => "emeralds",
            Mineral::Netherite => "ancient debris",
            _ => self.noun(),
        }
    }

    /// The word before "ore" or "block": "diamond", "iron".
    pub fn noun(self) -> &'static str {
        match self {
            Mineral::Coal => "coal",
            Mineral::Iron => "iron",
            Mineral::Copper => "copper",
            Mineral::Gold => "gold",
            Mineral::Redstone => "redstone",
            Mineral::Lapis => "lapis",
            Mineral::Diamond => "diamond",
            Mineral::Emerald => "emerald",
            Mineral::Quartz => "quartz",
            Mineral::Netherite => "netherite",
        }
    }

    /// The colour of the ore's specks.
    pub fn color(self) -> [u8; 3] {
        match self {
            Mineral::Coal => [118, 118, 124],
            Mineral::Iron => [216, 175, 147],
            Mineral::Copper => [226, 124, 82],
            Mineral::Gold => [250, 214, 70],
            Mineral::Redstone => [232, 52, 46],
            Mineral::Lapis => [62, 104, 222],
            Mineral::Diamond => [92, 219, 213],
            Mineral::Emerald => [32, 214, 104],
            Mineral::Quartz => [232, 226, 218],
            Mineral::Netherite => [128, 96, 102],
        }
    }

    /// What it is for, to a survival player.
    pub fn use_for(self) -> &'static str {
        match self {
            Mineral::Coal => "torches and smelting",
            Mineral::Iron => "tools, armour, buckets and shields",
            Mineral::Copper => "lightning rods and spyglasses",
            Mineral::Gold => "golden apples and bartering",
            Mineral::Redstone => "machines and potions",
            Mineral::Lapis => "enchanting",
            Mineral::Diamond => "the best tools and armour",
            Mineral::Emerald => "trading",
            Mineral::Quartz => "comparators and observers",
            Mineral::Netherite => "netherite gear",
        }
    }

    /// Riches a handful of which changes a survival game.
    pub fn precious(self) -> bool {
        matches!(self, Mineral::Diamond | Mineral::Emerald | Mineral::Netherite)
    }

    /// A share of the solid blocks of the map past which its ore is far
    /// beyond anything Minecraft puts underground — many times over.
    fn far_beyond(self) -> f64 {
        match self {
            Mineral::Coal => 0.04,
            Mineral::Iron => 0.02,
            Mineral::Copper => 0.015,
            Mineral::Gold => 0.004,
            Mineral::Redstone => 0.01,
            Mineral::Lapis => 0.003,
            Mineral::Diamond => 0.002,
            Mineral::Emerald => 0.001,
            Mineral::Quartz => 0.05,
            Mineral::Netherite => 0.001,
        }
    }

    /// Storage blocks anywhere, and ores and storage blocks in the open,
    /// from which on a survival game is broken.
    fn too_rich(self) -> (u64, u64) {
        if self.precious() { (4, 16) } else { (64, 256) }
    }

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// What a block is to a survival player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ore(Mineral),
    /// Nine ingots, gems or raw pieces in one block.
    Riches(Mineral),
    Bedrock,
    /// What no player can break or pick up: barriers, command blocks…
    Unbreakable,
    /// Logs and stems: what trees are made of.
    Log,
    /// Planks and bamboo: wood all the same.
    Wood,
    Leaves,
    Sapling,
    /// What stone tools and furnaces are made of, once mined.
    Stone,
    Obsidian,
    Grass,
    Water,
    Lava,
    Other,
}

/// What a block (`minecraft:oak_log`, with or without properties) is to a
/// survival player.
pub fn kind_of(state: &str) -> Kind {
    use Mineral::*;
    let name = state.split('[').next().unwrap_or(state);
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let n = name.strip_prefix("stripped_").unwrap_or(name);
    match n {
        "coal_ore" | "deepslate_coal_ore" => Kind::Ore(Coal),
        "iron_ore" | "deepslate_iron_ore" => Kind::Ore(Iron),
        "copper_ore" | "deepslate_copper_ore" => Kind::Ore(Copper),
        "gold_ore" | "deepslate_gold_ore" | "nether_gold_ore" => Kind::Ore(Gold),
        "redstone_ore" | "deepslate_redstone_ore" => Kind::Ore(Redstone),
        "lapis_ore" | "deepslate_lapis_ore" => Kind::Ore(Lapis),
        "diamond_ore" | "deepslate_diamond_ore" => Kind::Ore(Diamond),
        "emerald_ore" | "deepslate_emerald_ore" => Kind::Ore(Emerald),
        "nether_quartz_ore" => Kind::Ore(Quartz),
        "ancient_debris" => Kind::Ore(Netherite),
        "coal_block" => Kind::Riches(Coal),
        "iron_block" | "raw_iron_block" => Kind::Riches(Iron),
        "copper_block" | "raw_copper_block" => Kind::Riches(Copper),
        "gold_block" | "raw_gold_block" => Kind::Riches(Gold),
        "redstone_block" => Kind::Riches(Redstone),
        "lapis_block" => Kind::Riches(Lapis),
        "diamond_block" => Kind::Riches(Diamond),
        "emerald_block" => Kind::Riches(Emerald),
        "netherite_block" => Kind::Riches(Netherite),
        "bedrock" => Kind::Bedrock,
        "barrier" | "command_block" | "chain_command_block" | "repeating_command_block" | "structure_block" | "jigsaw" | "light" | "moving_piston"
        | "test_block" | "test_instance_block" => Kind::Unbreakable,
        "obsidian" => Kind::Obsidian,
        "water" | "bubble_column" => Kind::Water,
        "lava" => Kind::Lava,
        "grass_block" => Kind::Grass,
        "stone" | "cobblestone" | "deepslate" | "cobbled_deepslate" | "blackstone" => Kind::Stone,
        "crimson_stem" | "warped_stem" | "crimson_hyphae" | "warped_hyphae" => Kind::Log,
        "bamboo" | "bamboo_block" => Kind::Wood,
        "mangrove_propagule" | "azalea" | "flowering_azalea" => Kind::Sapling,
        _ if n.ends_with("_log") || n.ends_with("_wood") => Kind::Log,
        _ if n.ends_with("_planks") => Kind::Wood,
        _ if n.ends_with("_leaves") => Kind::Leaves,
        _ if n.ends_with("_sapling") => Kind::Sapling,
        _ => Kind::Other,
    }
}

/// `minecraft:grass_block` as people read it: "Grass Block".
pub fn pretty(id: &str) -> String {
    let name = id.split('[').next().unwrap_or(id);
    let Some((_, name)) = name.split_once(':') else { return name.to_owned() };
    let mut out = String::with_capacity(name.len());
    for (k, word) in name.split('_').filter(|w| !w.is_empty()).enumerate() {
        if k > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

/// `12345` as `12,345`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `part` of `whole` as a percentage that reads: `0.4 %`, `12 %`, `< 0.1 %`.
pub fn percent(part: u64, whole: u64) -> String {
    if part == 0 || whole == 0 {
        return "0 %".to_owned();
    }
    let p = part as f64 * 100.0 / whole as f64;
    if p < 0.1 {
        "< 0.1 %".to_owned()
    } else if p < 9.95 {
        format!("{p:.1} %")
    } else {
        format!("{p:.0} %")
    }
}

// --------------------------------------------------------------- results --

/// Blocks (or columns, or plants) of one kind: an id (`minecraft:stone`) or
/// a name, a colour, how many.
#[derive(Debug, Clone, PartialEq)]
pub struct Count {
    pub id: String,
    pub color: [u8; 3],
    pub count: u64,
}

/// One mineral over the whole map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MineralCount {
    pub mineral: Mineral,
    /// Ore blocks.
    pub ore: u64,
    /// Storage and raw blocks.
    pub riches: u64,
    /// Of both, those in the open: at the top of a column, or in objects.
    pub exposed: u64,
}

/// The land within [`SPAWN_REACH`] blocks of the spawn point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Near {
    pub columns: u64,
    /// Columns with water (lava, in a lava world) over them.
    pub wet: u64,
    /// Columns topped with grass.
    pub grass: u64,
    /// Columns a player walks across: no step of more than a block to a
    /// neighbour.
    pub walkable: u64,
    pub trees: u64,
}

/// The column of the spawn point.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnColumn {
    /// The height a player stands at.
    pub surface: i32,
    /// The block under their feet (`minecraft:grass_block`).
    pub block: String,
    /// Blocks of fluid over it: water, or lava in a lava world.
    pub depth: i32,
    /// No floor under it: open down to the void.
    pub void: bool,
}

/// Where to look on the map: of each kind of trouble, the spot nearest the
/// spawn point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spots {
    /// Ores and riches, by [`Mineral`].
    pub minerals: [Option<(i32, i32)>; 10],
    pub bedrock: Option<(i32, i32)>,
    pub unbreakable: Option<(i32, i32)>,
    pub void: Option<(i32, i32)>,
    pub ceiling: Option<(i32, i32)>,
}

/// What a pass over the map found.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub name: String,
    pub tiles: usize,
    pub columns: u64,
    /// The blocks the map's tiles span: west, north, east, south (the last
    /// two past the end).
    pub area: [i32; 4],
    pub min_y: i32,
    pub max_y: i32,
    pub sea_level: i32,
    pub spawn: [i32; 3],
    /// The world's fluid is lava.
    pub lava_world: bool,

    /// Columns by the height a player stands at on them, from `min_y` up.
    pub heights: Vec<u64>,
    /// The lowest and highest surfaces: x, y, z.
    pub lowest: [i32; 3],
    pub highest: [i32; 3],
    /// Columns by their largest step to a neighbour: none, one block, two
    /// or three (a climb), [`HURTING_FALL`] to 22 (a fall that hurts),
    /// [`LETHAL_FALL`] and more.
    pub steps: [u64; 5],
    /// Columns with a drop of [`HURTING_FALL`] blocks or more to a
    /// neighbour, and of [`LETHAL_FALL`] or more.
    pub hurting: u64,
    pub lethal: u64,
    /// The largest drop to a neighbour: blocks, and the column at its top.
    pub worst_drop: Option<(i32, i32, i32)>,
    /// Columns with more than one floor: overhangs, caves, islands.
    pub layered: u64,
    /// Air under the surface, in blocks: caves, under overhangs and islands.
    pub hollow: u64,
    /// Columns whose top is the top of the world.
    pub at_ceiling: u64,
    /// Columns with no floor: open down to the void.
    pub void: u64,

    /// Columns with fluid over them, the world's (water, or lava).
    pub wet: u64,
    /// Blocks of the world's fluid, over the land and in caves.
    pub fluid: u64,
    /// Of that, under the surface.
    pub buried_fluid: u64,
    pub depth_sum: u64,
    pub deepest: i32,
    /// Wet columns whose fluid stands above the sea: lakes and rivers.
    pub above_sea: u64,

    /// Every solid block the export writes, most first (fluids apart).
    pub volume: Vec<Count>,
    /// The block at the top of each column, most first.
    pub surface: Vec<Count>,
    /// What the covers grow, plants by kind, most first.
    pub plants: Vec<Count>,
    /// Each object, times it stands on the map, most first.
    pub objects: Vec<Count>,
    /// Columns by biome (as the export picks automatic ones), most first.
    pub biomes: Vec<Count>,
    /// Objects with logs and leaves.
    pub trees: u64,
    /// Every [`Mineral`], in its order.
    pub minerals: Vec<MineralCount>,
    /// Blocks of bedrock above the bottom [`FLOOR`] layers of the world.
    pub stray_bedrock: u64,

    pub near: Near,
    /// `None` when the spawn point is off the map.
    pub spawn_column: Option<SpawnColumn>,
    pub spots: Spots,
    /// Whether the export lays ores (Edit ▸ Ores).
    pub ores_on: bool,
    /// How long it took.
    pub seconds: f32,
}

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Breaks survival: riches lying around, bedrock in the way…
    Critical,
    /// Something survival misses: an ore, lava, grass…
    Warning,
    /// Fine.
    Good,
    /// Worth knowing.
    Info,
}

/// What a finding is about (for its icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Riches,
    Ores,
    Bedrock,
    Unbreakable,
    Void,
    Wood,
    Stone,
    Water,
    Lava,
    Nether,
    Food,
    Spawn,
    Drops,
    Ceiling,
    Caves,
    Export,
}

/// One thing the analysis has to say.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub topic: Topic,
    pub title: String,
    pub detail: String,
    /// Block ids and counts that show it.
    pub evidence: Option<String>,
    /// What to do about it.
    pub fix: Option<String>,
    /// Short labels with their colours: the ores missing…
    pub chips: Vec<(String, [u8; 3])>,
    /// Where to look on the map.
    pub spot: Option<(i32, i32)>,
    /// A window that deals with it.
    pub action: Option<Action>,
}

/// A window a finding leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Edit ▸ Ores: the ores the export lays.
    Ores,
}

impl Finding {
    fn new(severity: Severity, topic: Topic, title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            severity,
            topic,
            title: title.into(),
            detail: detail.into(),
            evidence: None,
            fix: None,
            chips: Vec::new(),
            spot: None,
            action: None,
        }
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    fn evidence(mut self, evidence: impl Into<String>) -> Self {
        let evidence = evidence.into();
        self.evidence = (!evidence.is_empty()).then_some(evidence);
        self
    }

    fn leads_to(mut self, action: Action) -> Self {
        self.action = Some(action);
        self
    }

    fn at(mut self, spot: Option<(i32, i32)>) -> Self {
        self.spot = spot;
        self
    }
}

/// A step of a survival game, and whether the map lets players take it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub name: &'static str,
    /// [`Severity::Good`], [`Severity::Warning`] or [`Severity::Critical`].
    pub state: Severity,
    pub why: String,
}

/// The analysis in one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ready,
    Gaps,
    NotReady,
}

impl Verdict {
    pub fn of(findings: &[Finding]) -> Verdict {
        if findings.iter().any(|f| f.severity == Severity::Critical) {
            Verdict::NotReady
        } else if findings.iter().any(|f| f.severity == Severity::Warning) {
            Verdict::Gaps
        } else {
            Verdict::Ready
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Verdict::Ready => "Ready for survival",
            Verdict::Gaps => "Playable, with gaps",
            Verdict::NotReady => "Not ready for survival",
        }
    }
}

// ------------------------------------------------------------------ pass --

/// Survey dimension `dim` of `world`, tile by tile on every core: a
/// snapshot, so that editing goes on meanwhile. `steps` follows it and can
/// stop it; `None` once stopped, or for a map without ground.
pub fn analyse(world: &World, dim: usize, steps: &Steps) -> Option<Analysis> {
    let started = Instant::now();
    let d = world.dims.get(dim)?;
    let mut tiles: Vec<TilePos> = d.ground().positions().collect();
    if tiles.is_empty() {
        return None;
    }
    // Row by row: neighbours are read together.
    tiles.sort_unstable_by_key(|p| (p.z, p.x));
    let ctx = Ctx::new(world, d);
    steps.done.store(0, Ordering::Relaxed);
    steps.total.store(tiles.len(), Ordering::Relaxed);
    let tally = tiles
        .par_iter()
        .fold(
            || (ctx.tally(), Scratch::default()),
            |(mut t, mut s), &pos| {
                if !steps.cancel.load(Ordering::Relaxed) {
                    ctx.tile(pos, &mut t, &mut s);
                }
                steps.done.fetch_add(1, Ordering::Relaxed);
                (t, s)
            },
        )
        .map(|(t, _)| t)
        .reduce(|| ctx.tally(), Tally::merge);
    if steps.cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(ctx.finish(tally, tiles.len(), started.elapsed().as_secs_f32()))
}

/// What a terrain's layers or an object hold that matters.
#[derive(Debug, Clone, Copy, Default)]
struct Marks {
    minerals: u16,
    bedrock: bool,
    unbreakable: bool,
}

impl Marks {
    fn add_kind(&mut self, kind: Kind) {
        match kind {
            Kind::Ore(m) | Kind::Riches(m) => self.minerals |= m.bit(),
            Kind::Bedrock => self.bedrock = true,
            Kind::Unbreakable => self.unbreakable = true,
            _ => {}
        }
    }

    fn add(&mut self, o: Marks) {
        self.minerals |= o.minerals;
        self.bedrock |= o.bedrock;
        self.unbreakable |= o.unbreakable;
    }

    fn is_empty(&self) -> bool {
        self.minerals == 0 && !self.bedrock && !self.unbreakable
    }
}

/// An object, as the pass counts it.
struct ObjectTally {
    /// Its blocks: index into [`Ctx::names`], how many.
    blocks: Vec<(usize, u64)>,
    marks: Marks,
    tree: bool,
}

/// Where trouble was seen.
#[derive(Debug, Clone, Copy)]
enum Spot {
    Mineral(Mineral),
    Bedrock,
    Unbreakable,
    Void,
    Ceiling,
}

impl Spot {
    fn index(self) -> usize {
        match self {
            Spot::Mineral(m) => m as usize,
            Spot::Bedrock => 10,
            Spot::Unbreakable => 11,
            Spot::Void => 12,
            Spot::Ceiling => 13,
        }
    }
}

const SPOTS: usize = 14;

/// What one thread has counted.
#[derive(Clone)]
struct Tally {
    columns: u64,
    /// Blocks by [`BlockId`].
    blocks: Vec<u64>,
    /// Blocks of objects, by [`Ctx::names`].
    extra: Vec<u64>,
    /// Ore blocks laid in the rock, by [`Ctx::names`].
    laid: Vec<u64>,
    /// Plants of covers, by [`Ctx::names`].
    plants: Vec<u64>,
    /// Tops of columns by [`BlockId`].
    surface: Vec<u64>,
    heights: Vec<u64>,
    /// `(y, x, z)`.
    lowest: Option<(i32, i32, i32)>,
    highest: Option<(i32, i32, i32)>,
    steps: [u64; 5],
    hurting: u64,
    lethal: u64,
    worst_drop: Option<(i32, i32, i32)>,
    layered: u64,
    hollow: u64,
    at_ceiling: u64,
    void: u64,
    wet: u64,
    fluid: u64,
    buried_fluid: u64,
    depth_sum: u64,
    deepest: i32,
    above_sea: u64,
    biomes: Vec<u64>,
    objects: Vec<u64>,
    trees: u64,
    stray_bedrock: u64,
    near: Near,
    spawn: Option<SpawnColumn>,
    /// The nearest to the spawn of each [`Spot`]: distance², x, z.
    spots: [Option<(i64, i32, i32)>; SPOTS],
}

impl Tally {
    fn spot(&mut self, spot: Spot, d: i64, x: i32, z: i32) {
        let slot = &mut self.spots[spot.index()];
        if slot.is_none_or(|s| (d, x, z) < s) {
            *slot = Some((d, x, z));
        }
    }

    fn merge(mut self, o: Tally) -> Tally {
        fn add(a: &mut [u64], b: &[u64]) {
            for (a, b) in a.iter_mut().zip(b) {
                *a += b;
            }
        }
        self.columns += o.columns;
        add(&mut self.blocks, &o.blocks);
        add(&mut self.extra, &o.extra);
        add(&mut self.laid, &o.laid);
        add(&mut self.plants, &o.plants);
        add(&mut self.surface, &o.surface);
        add(&mut self.heights, &o.heights);
        self.lowest = match (self.lowest, o.lowest) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.highest = match (self.highest, o.highest) {
            // The highest, and of those the first by x then z.
            (Some(a), Some(b)) => Some(if (b.0, -b.1, -b.2) > (a.0, -a.1, -a.2) { b } else { a }),
            (a, b) => a.or(b),
        };
        add(&mut self.steps, &o.steps);
        self.hurting += o.hurting;
        self.lethal += o.lethal;
        self.worst_drop = match (self.worst_drop, o.worst_drop) {
            (Some(a), Some(b)) => Some(if (b.0, -b.1, -b.2) > (a.0, -a.1, -a.2) { b } else { a }),
            (a, b) => a.or(b),
        };
        self.layered += o.layered;
        self.hollow += o.hollow;
        self.at_ceiling += o.at_ceiling;
        self.void += o.void;
        self.wet += o.wet;
        self.fluid += o.fluid;
        self.buried_fluid += o.buried_fluid;
        self.depth_sum += o.depth_sum;
        self.deepest = self.deepest.max(o.deepest);
        self.above_sea += o.above_sea;
        add(&mut self.biomes, &o.biomes);
        add(&mut self.objects, &o.objects);
        self.trees += o.trees;
        self.stray_bedrock += o.stray_bedrock;
        self.near.columns += o.near.columns;
        self.near.wet += o.near.wet;
        self.near.grass += o.near.grass;
        self.near.walkable += o.near.walkable;
        self.near.trees += o.near.trees;
        self.spawn = self.spawn.or(o.spawn);
        for (a, b) in self.spots.iter_mut().zip(o.spots) {
            *a = match (*a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        self
    }
}

/// Buffers one thread reuses from tile to tile.
#[derive(Default)]
struct Scratch {
    spans: Vec<Span>,
    fluids: Vec<(i32, i32)>,
    /// The surface of each column of the tile.
    tops: Vec<i32>,
    /// The runs of every column of the tile (for the ores): where each
    /// column's start in `runs`, and how many.
    runs: Vec<Span>,
    cells: Vec<(u32, u8)>,
}

/// What every tile of the pass reads.
struct Ctx<'a> {
    reg: &'a Registry,
    dim: &'a Dimension,
    mat: Materializer<'a>,
    name: String,
    seed: u64,
    min_y: i32,
    max_y: i32,
    sea: i32,
    lava: bool,
    spawn: [i32; 3],
    kinds: Vec<Kind>,
    soil: Vec<bool>,
    bedrock: BlockId,
    stone: BlockId,
    deepslate: BlockId,
    terrains: Vec<Marks>,
    /// The underground pockets, and what each pocket layer holds.
    pockets: Option<crate::pockets::Pockets<'a>>,
    pocket_marks: Vec<Marks>,
    /// Blocks that covers and objects add, by name (no properties), and
    /// their colours.
    names: Vec<String>,
    name_colors: Vec<[u8; 3]>,
    /// Of every cover, its plants: index into `names`, whether two high.
    covers: Vec<Vec<(usize, bool)>>,
    objects: Vec<ObjectTally>,
    has_objects: bool,
    biomes: crate::compose::AutoBiomes,
    /// The ores the export lays, and what each block is to them.
    ores: Option<crate::ores::Placer>,
    hosts: Vec<Option<crate::ores::Host>>,
    /// Of every ore of the settings, its blocks in stone and in deepslate:
    /// indices into `names`.
    ore_names: Vec<[usize; 2]>,
}

impl<'a> Ctx<'a> {
    fn new(world: &'a World, dim: &'a Dimension) -> Self {
        let reg = &world.registry;
        let kinds: Vec<Kind> = reg.blocks.iter().map(|b| kind_of(&b.state)).collect();
        let terrains = reg
            .terrains
            .iter()
            .map(|t| {
                let mut m = Marks::default();
                for s in &t.strata {
                    for (b, _) in &s.blocks {
                        m.add_kind(kinds.get(b.index()).copied().unwrap_or(Kind::Other));
                    }
                }
                m
            })
            .collect();
        let pocket_marks = reg
            .pockets
            .iter()
            .map(|p| {
                let mut m = Marks::default();
                for (b, _) in &p.blocks {
                    m.add_kind(kinds.get(b.index()).copied().unwrap_or(Kind::Other));
                }
                m
            })
            .collect();
        let mut names: Vec<String> = Vec::new();
        let mut name_colors: Vec<[u8; 3]> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut intern = |state: &str, color: [u8; 3]| -> usize {
            let name = state.split('[').next().unwrap_or(state).trim();
            *index.entry(name.to_owned()).or_insert_with(|| {
                names.push(name.to_owned());
                name_colors.push(color);
                names.len() - 1
            })
        };
        let mut covers = Vec::with_capacity(reg.covers.len());
        for c in &reg.covers {
            covers.push(c.plants.iter().map(|(state, _)| (intern(state, plant_color(state).unwrap_or(c.color)), is_tall_plant(state))).collect());
        }
        let mut objects = Vec::with_capacity(reg.objects.len());
        for o in &reg.objects {
            let mut counts = vec![0u64; o.palette.len()];
            for &b in &o.blocks {
                if let Some(n) = counts.get_mut(b as usize) {
                    *n += 1;
                }
            }
            let (mut marks, mut log, mut leaves) = (Marks::default(), false, false);
            let mut blocks = Vec::new();
            for (k, &n) in counts.iter().enumerate().skip(1) {
                let state = &o.palette[k];
                if n == 0 || is_nothing(state) {
                    continue;
                }
                let kind = kind_of(state);
                marks.add_kind(kind);
                log |= kind == Kind::Log;
                leaves |= kind == Kind::Leaves;
                blocks.push((intern(state, o.color(k as u16)), n));
            }
            objects.push(ObjectTally { blocks, marks, tree: log && leaves });
        }
        // The blocks of the ores, specks of their colour on the rock.
        let ore_names = world
            .meta
            .ores
            .ores
            .iter()
            .map(|rule| {
                let (stone, deep) = crate::ores::ore_blocks(rule.mineral);
                let speck = |rock: [u8; 3]| std::array::from_fn(|k| ((rock[k] as f32) * 0.6 + rule.mineral.color()[k] as f32 * 0.4) as u8);
                [intern(stone, speck([125, 125, 125])), intern(deep, speck([80, 80, 82]))]
            })
            .collect();
        let wk = reg.well_known;
        Self {
            reg,
            dim,
            mat: Materializer::new(reg, world.meta.seed, dim.min_y),
            name: world.meta.name.clone(),
            seed: world.meta.seed as u64,
            min_y: dim.min_y,
            max_y: dim.max_y,
            sea: world.meta.sea_level,
            lava: world.meta.lava,
            spawn: world.meta.spawn,
            soil: reg.blocks.iter().map(|b| is_soil(&b.state)).collect(),
            kinds,
            bedrock: wk.bedrock,
            stone: wk.stone,
            deepslate: wk.deepslate,
            terrains,
            pockets: crate::pockets::Pockets::new(reg, world.meta.seed as u64, dim.min_y),
            pocket_marks,
            names,
            name_colors,
            covers,
            has_objects: !reg.objects.is_empty() && reg.object_layers.iter().any(|l| !l.removed),
            objects,
            ores: crate::ores::Placer::new(&world.meta.ores, world.meta.seed),
            hosts: crate::ores::hosts(reg),
            ore_names,
            biomes: crate::compose::AutoBiomes::new(reg, world.meta.sea_level, dim.min_y),
        }
    }

    fn tally(&self) -> Tally {
        Tally {
            columns: 0,
            blocks: vec![0; self.reg.blocks.len()],
            extra: vec![0; self.names.len()],
            laid: vec![0; self.names.len()],
            plants: vec![0; self.names.len()],
            surface: vec![0; self.reg.blocks.len()],
            heights: vec![0; (self.max_y - self.min_y + 1).max(1) as usize],
            lowest: None,
            highest: None,
            steps: [0; 5],
            hurting: 0,
            lethal: 0,
            worst_drop: None,
            layered: 0,
            hollow: 0,
            at_ceiling: 0,
            void: 0,
            wet: 0,
            fluid: 0,
            buried_fluid: 0,
            depth_sum: 0,
            deepest: 0,
            above_sea: 0,
            biomes: vec![0; self.reg.biomes.len()],
            objects: vec![0; self.reg.objects.len()],
            trees: 0,
            stray_bedrock: 0,
            near: Near::default(),
            spawn: None,
            spots: [None; SPOTS],
        }
    }

    /// Squared distance from the spawn point.
    fn distance(&self, x: i32, z: i32) -> i64 {
        let (dx, dz) = ((x - self.spawn[0]) as i64, (z - self.spawn[2]) as i64);
        dx * dx + dz * dz
    }

    fn near(&self, x: i32, z: i32) -> bool {
        self.distance(x, z) <= (SPAWN_REACH as i64) * (SPAWN_REACH as i64)
    }

    fn kind(&self, b: BlockId) -> Kind {
        self.kinds.get(b.index()).copied().unwrap_or(Kind::Other)
    }

    /// Note where `marks` were seen.
    fn mark(&self, t: &mut Tally, marks: Marks, x: i32, z: i32) {
        let d = self.distance(x, z);
        for m in Mineral::ALL {
            if marks.minerals & m.bit() != 0 {
                t.spot(Spot::Mineral(m), d, x, z);
            }
        }
        if marks.bedrock {
            t.spot(Spot::Bedrock, d, x, z);
        }
        if marks.unbreakable {
            t.spot(Spot::Unbreakable, d, x, z);
        }
    }

    fn tile(&self, pos: TilePos, t: &mut Tally, s: &mut Scratch) {
        let Some(stack) = TileStack::new(self.dim, pos, true) else { return };
        let (ox, oz) = pos.origin();
        let ground = stack.ground();
        let n = TILE_SIZE as usize;
        let Scratch { spans, fluids, tops, runs, cells } = s;
        tops.clear();
        tops.resize(TILE_AREA, self.min_y);
        runs.clear();
        cells.clear();
        cells.resize(TILE_AREA, (0, 0));
        for lz in 0..n {
            for lx in 0..n {
                let cell = cell_index(lx, lz);
                let (x, z) = (ox + lx as i32, oz + lz as i32);
                let near = self.near(x, z);
                t.columns += 1;
                if near {
                    t.near.columns += 1;
                }
                stack.spans(cell, spans);
                stack.fluids(cell, spans, fluids);
                if self.ores.is_some() {
                    cells[cell] = (runs.len() as u32, spans.len().min(255) as u8);
                    runs.extend(spans.iter().take(255));
                }
                let floorless = spans.first().is_none_or(|s| s.y0 > self.min_y);
                if floorless {
                    t.void += 1;
                    t.spot(Spot::Void, self.distance(x, z), x, z);
                }
                let Some(top) = spans.last().copied() else {
                    if [x, z] == [self.spawn[0], self.spawn[2]] {
                        t.spawn = Some(SpawnColumn {
                            surface: self.min_y,
                            block: "minecraft:air".to_owned(),
                            depth: 0,
                            void: true,
                        });
                    }
                    continue;
                };
                let surface = top.y1;
                tops[cell] = surface;
                t.heights[(surface - self.min_y).clamp(0, self.max_y - self.min_y) as usize] += 1;
                if t.lowest.is_none_or(|l| (surface, x, z) < l) {
                    t.lowest = Some((surface, x, z));
                }
                if t.highest.is_none_or(|h| (surface, -x, -z) > (h.0, -h.1, -h.2)) {
                    t.highest = Some((surface, x, z));
                }
                if surface >= self.max_y {
                    t.at_ceiling += 1;
                    t.spot(Spot::Ceiling, self.distance(x, z), x, z);
                }

                // Fluid over the surface, or under it in caves.
                let mut wet = None;
                for &(f0, f1) in fluids.iter() {
                    let blocks = (f1 - f0) as u64;
                    t.fluid += blocks;
                    if f0 == surface {
                        wet = Some((f0, f1));
                    } else if f1 <= top.y0 {
                        t.buried_fluid += blocks;
                    }
                }
                if let Some((f0, f1)) = wet {
                    t.wet += 1;
                    t.depth_sum += (f1 - f0) as u64;
                    t.deepest = t.deepest.max(f1 - f0);
                    if f1 > self.sea {
                        t.above_sea += 1;
                    }
                    if near {
                        t.near.wet += 1;
                    }
                }
                if spans.len() > 1 || floorless {
                    if spans.len() > 1 {
                        t.layered += 1;
                    }
                    let mut air = (spans[0].y0 - self.min_y).max(0) as u64;
                    for w in spans.windows(2) {
                        air += (w[1].y0 - w[0].y1).max(0) as u64;
                    }
                    let flooded: u64 = fluids.iter().filter(|f| f.1 <= top.y0).map(|f| (f.1 - f.0) as u64).sum();
                    t.hollow += air.saturating_sub(flooded);
                }

                // Every block of every run, as the export writes them.
                let mut marks = Marks::default();
                for span in spans.iter() {
                    let submerged = fluids.iter().any(|f| f.0 == span.y1);
                    self.count_span(span, x, z, submerged, t);
                    marks.add(self.terrains.get(span.terrain.index()).copied().unwrap_or_default());
                }
                // Underground pockets, in the place of the rock.
                let pocket = ground.pockets.get(cell);
                if let Some(col) = self.pockets.as_ref().and_then(|p| p.column(pocket, spans, stack.cave_ground(cell), &self.mat, x, z)) {
                    for y in col.low..=col.high {
                        // In rock only, not in the air of a cave.
                        if let Some(b) = col.at(x, y, z)
                            && spans.iter().any(|s| s.y0 <= y && y < s.y1)
                        {
                            t.blocks[self.mat.substrate(x, y, z).index()] -= 1;
                            t.blocks[b.index()] += 1;
                        }
                    }
                    marks.add(self.pocket_marks.get(pocket as usize - 1).copied().unwrap_or_default());
                }
                let under = self.mat.block(&top, x, surface - 1, z, wet.is_some());
                t.surface[under.index()] += 1;
                let kind = self.kind(under);
                // Bedrock at the bottom of a pit down to the floor is where it belongs.
                if kind != Kind::Bedrock || surface > self.min_y + FLOOR {
                    marks.add_kind(kind);
                }
                if !marks.is_empty() {
                    self.mark(t, marks, x, z);
                }

                // The biome, as the export picks it.
                let painted = ground.biome.get(cell);
                let biome = if painted != 0 { painted } else { self.auto_biome(top.terrain, surface, ground.fluid.get(cell)) };
                if let Some(c) = t.biomes.get_mut(biome as usize) {
                    *c += 1;
                }

                // What grows on top.
                let cover = stack.surface_cover(cell, &top);
                if cover != 0
                    && let Some(plant) = self.plant(cover, x, z, surface, under, wet)
                {
                    t.plants[plant] += 1;
                }
                if near && kind == Kind::Grass {
                    t.near.grass += 1;
                }
                if [x, z] == [self.spawn[0], self.spawn[2]] {
                    t.spawn = Some(SpawnColumn {
                        surface,
                        block: self.reg.block(under).name().to_owned(),
                        depth: wet.map_or(0, |(f0, f1)| f1 - f0),
                        void: floorless,
                    });
                }
            }
        }

        // The steps between neighbours, in the tile.
        for lz in 0..n {
            for lx in 0..n {
                let cell = cell_index(lx, lz);
                let h = tops[cell];
                let (mut step, mut drop) = (0, 0);
                let mut see = |other: usize| {
                    let d = h - tops[other];
                    step = step.max(d.abs());
                    drop = drop.max(d);
                };
                if lx > 0 {
                    see(cell - 1);
                }
                if lx + 1 < n {
                    see(cell + 1);
                }
                if lz > 0 {
                    see(cell - n);
                }
                if lz + 1 < n {
                    see(cell + n);
                }
                let class = match step {
                    0 => 0,
                    1 => 1,
                    s if s < HURTING_FALL => 2,
                    s if s < LETHAL_FALL => 3,
                    _ => 4,
                };
                t.steps[class] += 1;
                let (x, z) = (ox + lx as i32, oz + lz as i32);
                if drop >= HURTING_FALL {
                    t.hurting += 1;
                }
                if drop >= LETHAL_FALL {
                    t.lethal += 1;
                }
                if drop > 0 && t.worst_drop.is_none_or(|w| (drop, -x, -z) > (w.0, -w.1, -w.2)) {
                    t.worst_drop = Some((drop, x, z));
                }
                if step <= 1 && self.near(x, z) {
                    t.near.walkable += 1;
                }
            }
        }

        // The ores, as the export lays them in the rock: each takes the
        // place of the block counted there.
        if let Some(placer) = &self.ores {
            let n = TILE_SIZE >> 4;
            let highest = tops.iter().copied().max().unwrap_or(self.min_y);
            let run = |cell: usize| {
                let (start, len) = cells[cell];
                &runs[start as usize..start as usize + len as usize]
            };
            // The biome of each 4 × 4 as the export writes it, for the ores
            // that keep to some biomes.
            let biomes: Vec<crate::compose::BiomeColumn> = if placer.keeps_to_biomes() {
                (0..TILE_AREA / 16)
                    .map(|k| {
                        let cell = cell_index((k % 32) * 4 + 2, (k / 32) * 4 + 2);
                        let mut out = crate::compose::BiomeColumn::default();
                        stack.biome_column(cell, self.reg, &self.biomes, || run(cell).last().map(|s| (s.y1, s.terrain)), &mut out);
                        out
                    })
                    .collect()
            } else {
                Vec::new()
            };
            for chunk in placer.chunks(ox >> 4, oz >> 4, n, n, (self.min_y, highest)) {
                placer.lay(
                    &chunk,
                    |x, y, z| {
                        let cell = crate::coords::local_index(x, z);
                        let pockets = self.pockets.as_ref().and_then(|p| p.column(ground.pockets.get(cell), run(cell), stack.cave_ground(cell), &self.mat, x, z));
                        crate::ores::rock_at(run(cell), &self.mat, &self.hosts, pockets.as_ref(), x, y, z, self.max_y)
                    },
                    |x, y, z| biomes[(((z - oz) >> 2) * 32 + ((x - ox) >> 2)) as usize].of_block(y),
                    |b, (rock, host)| {
                        t.blocks[rock.index()] -= 1;
                        t.laid[self.ore_names[b.ore as usize][(host == crate::ores::Host::Deepslate) as usize]] += 1;
                    },
                );
            }
        }

        // What stands on the land.
        if self.has_objects {
            for p in tile_placements(self.reg, &self.mat, self.seed, self.lava, &stack, pos) {
                let Some(o) = self.objects.get(p.object as usize) else { continue };
                t.objects[p.object as usize] += 1;
                for &(name, blocks) in &o.blocks {
                    t.extra[name] += blocks;
                }
                if o.tree {
                    t.trees += 1;
                    if self.near(p.x, p.z) {
                        t.near.trees += 1;
                    }
                }
                if !o.marks.is_empty() {
                    self.mark(t, o.marks, p.x, p.z);
                }
            }
        }
    }

    /// Count `n` blocks `b` from `y0` up, telling bedrock above the floor
    /// apart.
    fn add(&self, t: &mut Tally, b: BlockId, y0: i32, y1: i32) {
        if y1 <= y0 {
            return;
        }
        t.blocks[b.index()] += (y1 - y0) as u64;
        if b == self.bedrock {
            t.stray_bedrock += overlap(y0, y1, self.min_y + FLOOR, i32::MAX);
        }
    }

    /// The blocks of `span`, as the export writes them: the rock below the
    /// terrain's recipe, then the recipe.
    fn count_span(&self, span: &Span, x: i32, z: i32, submerged: bool, t: &mut Tally) {
        let len = (span.y1 - span.y0).max(0) as u32;
        if len == 0 {
            return;
        }
        let recipe = self.mat.recipe_depth(span.terrain, x, z);
        let split = if recipe >= len { span.y0 } else { span.y1 - recipe as i32 };
        self.count_rock(x, z, span.y0, split, t);
        self.count_recipe(span, x, z, submerged, split, t);
    }

    /// [`Materializer::substrate`] from `y0` up to `y1`: block by block
    /// only where it is drawn by chance (the bedrock floor, deepslate
    /// turning into stone), counted at once elsewhere.
    fn count_rock(&self, x: i32, z: i32, y0: i32, y1: i32, t: &mut Tally) {
        if y1 <= y0 {
            return;
        }
        let floor = self.min_y + FLOOR;
        let (mut deep, mut stone) = (overlap(y0, y1, i32::MIN, 0), overlap(y0, y1, 8, i32::MAX));
        let chance = (y0.max(self.min_y)..y1.min(floor)).chain(y0.max(0).max(floor)..y1.min(8));
        for y in chance {
            let b = self.mat.substrate(x, y, z);
            t.blocks[b.index()] += 1;
            if y < 0 {
                deep -= 1;
            } else if y >= 8 {
                stone -= 1;
            }
        }
        t.blocks[self.deepslate.index()] += deep;
        t.blocks[self.stone.index()] += stone;
    }

    /// The layers of `span`'s terrain from its top down to `lo`, as
    /// [`Materializer::block`] gives them, a layer at a time.
    fn count_recipe(&self, span: &Span, x: i32, z: i32, submerged: bool, lo: i32, t: &mut Tally) {
        if lo >= span.y1 {
            return;
        }
        let def = self.reg.terrain(span.terrain);
        let h = hash2(x, z, self.seed);
        // Bedrock still wins at the very bottom of the world.
        let floor = (span.y0 == self.min_y).then_some(self.min_y + FLOOR);
        let room = (span.y1 - lo) as u32;
        let mut depth = 0u32;
        for (i, s) in def.strata.iter().enumerate() {
            if depth >= room {
                break;
            }
            let thickness = stratum_thickness(s.depth, h, i);
            let end = depth.saturating_add(thickness).min(room);
            let (mut y0, mut y1) = (span.y1 - end as i32, span.y1 - depth as i32);
            depth = depth.saturating_add(thickness);
            if y1 <= y0 || s.blocks.is_empty() {
                continue;
            }
            if i == 0
                && y1 == span.y1
                && submerged
                && let Some(b) = def.submerged_top
            {
                self.add(t, b, y1 - 1, y1);
                y1 -= 1;
            }
            if let Some(floor) = floor
                && y0 < floor
            {
                let top = y1.min(floor);
                for y in y0..top {
                    let b = if self.mat.substrate(x, y, z) == self.bedrock { self.bedrock } else { crate::compose::stratum_block(s, x, y, z, self.seed) };
                    self.add(t, b, y, y + 1);
                }
                y0 = top;
            }
            self.count_layer(s, x, z, y0, y1, t);
        }
    }

    /// The blocks of one layer from `y0` up to `y1`.
    fn count_layer(&self, s: &Stratum, x: i32, z: i32, y0: i32, y1: i32, t: &mut Tally) {
        if y1 <= y0 {
            return;
        }
        if let [(b, _)] = s.blocks[..] {
            self.add(t, b, y0, y1);
        } else if y1 - y0 <= 64 || !s.mix.is_grain() {
            for y in y0..y1 {
                self.add(t, crate::compose::stratum_block(s, x, y, z, self.seed), y, y + 1);
            }
        } else {
            // A deep mix: its blocks by their weights, the leftover to the last.
            let n = (y1 - y0) as u64;
            let total = s.blocks.iter().map(|b| b.1 as u64).sum::<u64>().max(1);
            let mut given = 0;
            for (k, &(b, w)) in s.blocks.iter().enumerate() {
                let share = if k + 1 == s.blocks.len() { n - given } else { n * w as u64 / total };
                given += share;
                t.blocks[b.index()] += share;
                if b == self.bedrock {
                    t.stray_bedrock += share * overlap(y0, y1, self.min_y + FLOOR, i32::MAX) / n;
                }
            }
        }
    }

    /// The biome of a column whose biome is left on automatic, as the
    /// export picks it.
    fn auto_biome(&self, terrain: TerrainId, surface: i32, fluid: i16) -> u16 {
        self.biomes.pick(self.reg, terrain, surface, fluid)
    }

    /// The plant cover `cover` grows on a column whose surface is at
    /// `surface` over block `under`, as the export grows it: index into
    /// `names`.
    fn plant(&self, cover: u16, x: i32, z: i32, surface: i32, under: BlockId, wet: Option<(i32, i32)>) -> Option<usize> {
        let def = self.reg.cover(cover)?;
        if surface <= self.min_y || surface >= self.max_y {
            return None;
        }
        let growth = def.growth(x, z, self.seed)?;
        let &(name, tall) = self.covers.get(cover as usize - 1)?.get(growth.plant)?;
        if def.soil_only && !self.soil.get(under.index()).copied().unwrap_or(false) {
            return None;
        }
        let height = if tall { 2 } else { growth.height as i32 };
        if surface + height > self.max_y {
            return None;
        }
        match (def.underwater, self.lava, wet) {
            (false, _, None) => Some(name),
            (true, false, Some((f0, f1))) if f1 - f0 >= height => Some(name),
            _ => None,
        }
    }

    fn finish(&self, t: Tally, tiles: usize, seconds: f32) -> Analysis {
        let reg = self.reg;
        fn put(map: &mut HashMap<String, Count>, id: &str, color: [u8; 3], n: u64) {
            if n == 0 || is_nothing(id) {
                return;
            }
            map.entry(id.to_owned()).or_insert_with(|| Count { id: id.to_owned(), color, count: 0 }).count += n;
        }
        fn sorted(map: HashMap<String, Count>) -> Vec<Count> {
            let mut v: Vec<Count> = map.into_values().collect();
            v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));
            v
        }
        let mut volume = HashMap::new();
        let mut surface = HashMap::new();
        for (k, def) in reg.blocks.iter().enumerate() {
            put(&mut volume, def.name(), def.color, t.blocks[k]);
            put(&mut surface, def.name(), def.color, t.surface[k]);
        }
        let mut plants = HashMap::new();
        for (k, name) in self.names.iter().enumerate() {
            put(&mut volume, name, self.name_colors[k], t.extra[k] + t.laid[k]);
            put(&mut plants, name, self.name_colors[k], t.plants[k]);
        }
        let mut objects = HashMap::new();
        for (k, def) in reg.objects.iter().enumerate() {
            let color = def.top_view().first().map_or([125, 125, 125], |&(k, _)| def.color(k));
            put(&mut objects, &def.name, color, t.objects[k]);
        }
        let mut biomes = HashMap::new();
        for (k, def) in reg.biomes.iter().enumerate() {
            put(&mut biomes, &def.name, def.color, t.biomes[k]);
        }
        let volume = sorted(volume);
        let surface = sorted(surface);
        let minerals = Mineral::ALL
            .iter()
            .map(|&m| {
                let of = |list: &[Count], f: &dyn Fn(Kind) -> bool| list.iter().filter(|c| f(kind_of(&c.id))).map(|c| c.count).sum::<u64>();
                let ore = of(&volume, &|k| k == Kind::Ore(m));
                let riches = of(&volume, &|k| k == Kind::Riches(m));
                let mut exposed = of(&surface, &|k| k == Kind::Ore(m) || k == Kind::Riches(m));
                for (k, name) in self.names.iter().enumerate() {
                    if matches!(kind_of(name), Kind::Ore(x) | Kind::Riches(x) if x == m) {
                        exposed += t.extra[k];
                    }
                }
                MineralCount { mineral: m, ore, riches, exposed }
            })
            .collect();
        let spot = |s: Spot| t.spots[s.index()].map(|(_, x, z)| (x, z));
        let mut spots = Spots {
            bedrock: spot(Spot::Bedrock),
            unbreakable: spot(Spot::Unbreakable),
            void: spot(Spot::Void),
            ceiling: spot(Spot::Ceiling),
            ..Spots::default()
        };
        for m in Mineral::ALL {
            spots.minerals[m as usize] = spot(Spot::Mineral(m));
        }
        let (lo, hi) = self.dim.bounds().unwrap_or((TilePos::new(0, 0), TilePos::new(0, 0)));
        let lowest = t.lowest.map_or([0, self.min_y, 0], |(y, x, z)| [x, y, z]);
        let highest = t.highest.map_or([0, self.min_y, 0], |(y, x, z)| [x, y, z]);
        Analysis {
            name: self.name.clone(),
            tiles,
            columns: t.columns,
            area: [lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE],
            min_y: self.min_y,
            max_y: self.max_y,
            sea_level: self.sea,
            spawn: self.spawn,
            lava_world: self.lava,
            heights: t.heights,
            lowest,
            highest,
            steps: t.steps,
            hurting: t.hurting,
            lethal: t.lethal,
            worst_drop: t.worst_drop,
            layered: t.layered,
            hollow: t.hollow,
            at_ceiling: t.at_ceiling,
            void: t.void,
            wet: t.wet,
            fluid: t.fluid,
            buried_fluid: t.buried_fluid,
            depth_sum: t.depth_sum,
            deepest: t.deepest,
            above_sea: t.above_sea,
            volume,
            surface,
            plants: sorted(plants),
            objects: sorted(objects),
            biomes: sorted(biomes),
            trees: t.trees,
            minerals,
            stray_bedrock: t.stray_bedrock,
            near: t.near,
            spawn_column: t.spawn,
            spots,
            ores_on: self.ores.is_some(),
            seconds,
        }
    }
}

/// How many heights `[a0, a1)` and `[b0, b1)` share.
fn overlap(a0: i32, a1: i32, b0: i32, b1: i32) -> u64 {
    (a1.min(b1) as i64 - a0.max(b0) as i64).max(0) as u64
}

// -------------------------------------------------------------- reading --

impl Analysis {
    /// Solid blocks of the map.
    pub fn solid(&self) -> u64 {
        self.volume.iter().map(|c| c.count).sum()
    }

    /// Solid blocks that are of a kind `f` accepts.
    pub fn blocks_of(&self, f: impl Fn(Kind) -> bool) -> u64 {
        self.volume.iter().filter(|c| f(kind_of(&c.id))).map(|c| c.count).sum()
    }

    /// Plants of covers of a kind `f` accepts.
    pub fn plants_of(&self, f: impl Fn(Kind) -> bool) -> u64 {
        self.plants.iter().filter(|c| f(kind_of(&c.id))).map(|c| c.count).sum()
    }

    /// Columns whose top is of a kind `f` accepts.
    pub fn surface_of(&self, f: impl Fn(Kind) -> bool) -> u64 {
        self.surface.iter().filter(|c| f(kind_of(&c.id))).map(|c| c.count).sum()
    }

    pub fn mineral(&self, m: Mineral) -> MineralCount {
        self.minerals[m as usize]
    }

    /// Chunks of the map (16 × 16 columns).
    pub fn chunks(&self) -> f64 {
        self.columns as f64 / 256.0
    }

    /// Wood to start with: logs, planks, bamboo.
    pub fn wood(&self) -> u64 {
        self.blocks_of(|k| matches!(k, Kind::Log | Kind::Wood)) + self.plants_of(|k| k == Kind::Wood)
    }

    /// Water, blocks of it.
    pub fn water(&self) -> u64 {
        let fluid = if self.lava_world { 0 } else { self.fluid };
        fluid + self.blocks_of(|k| k == Kind::Water)
    }

    /// Lava, blocks of it.
    pub fn lava(&self) -> u64 {
        let fluid = if self.lava_world { self.fluid } else { 0 };
        fluid + self.blocks_of(|k| k == Kind::Lava)
    }

    /// Whether players can make a Nether portal: obsidian cast from lava
    /// with water carried in an iron bucket, or mined with a diamond
    /// pickaxe.
    pub fn nether(&self) -> bool {
        let has = |m: Mineral| self.mineral(m).ore + self.mineral(m).riches > 0;
        (self.lava() > 0 && self.water() > 0 && has(Mineral::Iron)) || (self.blocks_of(|k| k == Kind::Obsidian) >= 10 && has(Mineral::Diamond))
    }

    /// The mean height players stand at.
    pub fn mean_height(&self) -> f64 {
        let n: u64 = self.heights.iter().sum();
        if n == 0 {
            return self.min_y as f64;
        }
        self.heights.iter().enumerate().map(|(k, &c)| (self.min_y + k as i32) as f64 * c as f64).sum::<f64>() / n as f64
    }

    /// The height half the columns are under.
    pub fn median_height(&self) -> i32 {
        let n: u64 = self.heights.iter().sum();
        let mut seen = 0;
        for (k, &c) in self.heights.iter().enumerate() {
            seen += c;
            if seen * 2 >= n && c > 0 {
                return self.min_y + k as i32;
            }
        }
        self.min_y
    }

    /// The spread of the heights around their mean, in blocks.
    pub fn height_spread(&self) -> f64 {
        let n: u64 = self.heights.iter().sum();
        if n == 0 {
            return 0.0;
        }
        let mean = self.mean_height();
        let var = self.heights.iter().enumerate().map(|(k, &c)| ((self.min_y + k as i32) as f64 - mean).powi(2) * c as f64).sum::<f64>() / n as f64;
        var.sqrt()
    }

    /// Columns players stand on: those with a top.
    pub fn grounded(&self) -> u64 {
        self.heights.iter().sum()
    }

    /// The spawn point's column, as a spot.
    fn spawn_spot(&self) -> Option<(i32, i32)> {
        self.spawn_column.as_ref().map(|_| (self.spawn[0], self.spawn[2]))
    }

    /// The steps of a survival game the map lets players take.
    pub fn progression(&self) -> Vec<Stage> {
        let has = |m: Mineral| self.mineral(m).ore + self.mineral(m).riches;
        let wood = self.wood();
        let stone = self.blocks_of(|k| k == Kind::Stone);
        let (iron, diamonds) = (has(Mineral::Iron), has(Mineral::Diamond));
        let stage = |name, ok: bool, bad: Severity, yes: String, no: &str| Stage {
            name,
            state: if ok { Severity::Good } else { bad },
            why: if ok { yes } else { no.to_owned() },
        };
        vec![
            stage(
                "Wood",
                wood > 0,
                Severity::Critical,
                format!("{} trees, {} blocks of wood", thousands(self.trees), thousands(wood)),
                "No wood: no crafting table, no first tool",
            ),
            stage("Stone", stone > 0, Severity::Critical, format!("{} blocks that mine into cobblestone", thousands(stone)), "Nothing mines into cobblestone: no stone tools, no furnace"),
            stage("Iron", iron > 0, Severity::Warning, format!("{} blocks of iron ore", thousands(iron)), "No iron: no bucket, shield or armour"),
            stage("Diamonds", diamonds > 0, Severity::Warning, format!("{} blocks of diamond ore", thousands(diamonds)), "No diamonds: no diamond pickaxe for obsidian"),
            stage(
                "Nether",
                self.nether(),
                Severity::Warning,
                "Obsidian can be made for a portal".to_owned(),
                "No way to ten obsidian for a portal",
            ),
        ]
    }

    /// What the analysis has to say, the worst first.
    pub fn findings(&self) -> Vec<Finding> {
        use Severity::*;
        let mut out = Vec::new();
        let solid = self.solid().max(1);
        let chunks = self.chunks().max(1.0 / 256.0);
        let grounded = self.grounded().max(1);

        // ---- riches lying around, ores far too common
        for m in Mineral::ALL {
            let c = self.mineral(m);
            if c.ore + c.riches == 0 {
                continue;
            }
            let (stash, open) = m.too_rich();
            let share = c.ore as f64 / solid as f64;
            let flood = share > m.far_beyond();
            let severe = c.riches >= stash || c.exposed >= open || flood;
            if !severe && c.riches == 0 && c.exposed == 0 {
                continue;
            }
            let mut held = Vec::new();
            if c.riches > 0 {
                held.push(format!("{} {} blocks", thousands(c.riches), m.noun()));
            }
            if c.ore > 0 {
                held.push(format!("{} {} ores", thousands(c.ore), m.noun()));
            }
            let held = held.join(" and ");
            // Riches first, then ores.
            let mut shown: Vec<&Count> = self.volume.iter().filter(|v| matches!(kind_of(&v.id), Kind::Ore(x) | Kind::Riches(x) if x == m)).collect();
            shown.sort_by_key(|v| (!matches!(kind_of(&v.id), Kind::Riches(_)), std::cmp::Reverse(v.count)));
            let evidence = shown.iter().map(|v| format!("{} × {}", v.id, thousands(v.count))).collect::<Vec<_>>().join(" · ");
            let open_part = if c.exposed > 0 { format!(", {} of them in the open", thousands(c.exposed)) } else { String::new() };
            let finding = if !severe {
                Finding::new(
                    Warning,
                    Topic::Riches,
                    format!("A few {} in the open", m.plural()),
                    format!("The map holds {held}{open_part}: fine as a reward or in a build, a head start if players find them on the first day."),
                )
                .fix("Keep riches in chests and rewards, out of the ground players start on.")
            } else if flood && c.riches < stash && c.exposed < open {
                Finding::new(
                    Critical,
                    Topic::Ores,
                    format!("Far too much {}", m.noun()),
                    format!(
                        "{} ore makes up {} of the ground ({held}, about {} a chunk): many times what Minecraft puts underground. Mining loses its point.",
                        m.name(),
                        percent(c.ore, solid),
                        thousands((c.ore as f64 / chunks).round() as u64)
                    ),
                )
                .fix("Use far fewer ore blocks in the layers of the terrains that hold them: a vein here and there.")
            } else {
                let why = if m.precious() {
                    format!("A player with {} on the first day skips most of the game.", m.plural())
                } else {
                    format!("{} for the taking cuts the game short.", capitalised(m.plural()))
                };
                Finding::new(Critical, Topic::Riches, format!("{} lying around", capitalised(m.plural())), format!("The map holds {held}{open_part}. {why}"))
                    .fix("Paint those columns with a natural terrain, and keep riches for chests and rewards.")
            };
            out.push(finding.evidence(evidence).at(self.spots.minerals[m as usize]));
        }

        // ---- bedrock out of place
        if self.stray_bedrock > 0 {
            let on_top = self.surface_of(|k| k == Kind::Bedrock);
            let severe = on_top > 0 || self.stray_bedrock >= 64;
            let title = if on_top > 0 { "Bedrock where players walk" } else { "Bedrock above the floor" };
            let at_top = if on_top > 0 { format!(", {} columns of it at the surface", thousands(on_top)) } else { String::new() };
            out.push(
                Finding::new(
                    if severe { Critical } else { Warning },
                    Topic::Bedrock,
                    title,
                    format!(
                        "{} blocks of bedrock stand above the bottom {FLOOR} layers of the world{at_top}. No player can break bedrock: there it makes walls and dead ends.",
                        thousands(self.stray_bedrock)
                    ),
                )
                .evidence(format!("minecraft:bedrock × {} above y {}", thousands(self.stray_bedrock), self.min_y + FLOOR - 1))
                .fix("Paint those columns with Stone or another terrain: bedrock belongs at the very bottom only.")
                .at(self.spots.bedrock),
            );
        }

        // ---- blocks no one can break
        let unbreakable: Vec<&Count> = self.volume.iter().filter(|c| kind_of(&c.id) == Kind::Unbreakable).collect();
        if !unbreakable.is_empty() {
            let n: u64 = unbreakable.iter().map(|c| c.count).sum();
            out.push(
                Finding::new(
                    Critical,
                    Topic::Unbreakable,
                    "Blocks no player can break",
                    format!("{} barriers, command blocks or the like: in survival they can't be mined or picked up, and stay in the way for good.", thousands(n)),
                )
                .evidence(unbreakable.iter().map(|c| format!("{} × {}", c.id, thousands(c.count))).collect::<Vec<_>>().join(" · "))
                .fix("Take them out of the objects or terrains that hold them.")
                .at(self.spots.unbreakable),
            );
        }

        // ---- holes into the void
        if self.void > 0 {
            out.push(
                Finding::new(
                    Critical,
                    Topic::Void,
                    "Holes into the void",
                    format!("{} columns have no floor: a player who falls in dies and loses everything they carried.", thousands(self.void)),
                )
                .evidence(format!("{} columns open below y {}", thousands(self.void), self.min_y))
                .fix("Raise the ground there, or stop carving before the bottom of the world.")
                .at(self.spots.void),
            );
        }

        // ---- the spawn
        match &self.spawn_column {
            None => out.push(
                Finding::new(
                    Warning,
                    Topic::Spawn,
                    "Spawn off the map",
                    format!(
                        "The spawn point ({}, {}) is outside the map: players start in Minecraft's own land, or over the void.",
                        self.spawn[0], self.spawn[2]
                    ),
                )
                .fix("Set the spawn point on the map (Export ▸ Spawn)."),
            ),
            Some(c) if c.void => out.push(
                Finding::new(Critical, Topic::Spawn, "Spawn over the void", "The spawn point has no ground under it: players fall out of the world as they arrive.")
                    .fix("Move the spawn point onto solid ground (Export ▸ Spawn).")
                    .at(self.spawn_spot()),
            ),
            Some(c) if self.lava_world && c.depth > 0 => out.push(
                Finding::new(Critical, Topic::Spawn, "Spawn in lava", format!("The spawn point is under {} blocks of lava: players burn as they arrive.", c.depth))
                    .fix("Move the spawn point onto dry ground (Export ▸ Spawn).")
                    .at(self.spawn_spot()),
            ),
            Some(c) if c.depth >= 2 => out.push(
                Finding::new(Warning, Topic::Spawn, "Spawn in water", format!("The spawn point is under {} blocks of water: players start by swimming for a shore.", c.depth))
                    .fix("Move the spawn point onto dry ground (Export ▸ Spawn).")
                    .at(self.spawn_spot()),
            ),
            Some(c) => out.push(
                Finding::new(
                    Good,
                    Topic::Spawn,
                    format!("Spawn on {} at y {}", pretty(&c.block).to_lowercase(), c.surface),
                    format!(
                        "Within {SPAWN_REACH} blocks: {} trees, water on {}, grass on {}, {} of the land walkable.",
                        thousands(self.near.trees),
                        percent(self.near.wet, self.near.columns),
                        percent(self.near.grass, self.near.columns),
                        percent(self.near.walkable, self.near.columns)
                    ),
                )
                .at(self.spawn_spot()),
            ),
        }

        // ---- wood
        let wood = self.wood();
        if wood == 0 {
            out.push(
                Finding::new(
                    Critical,
                    Topic::Wood,
                    "No wood anywhere",
                    "Not a tree, log, plank or stem of bamboo on the map. Without wood there is no crafting table and no first tool: survival can't begin.",
                )
                .fix("Stand trees with Objects (a schematic of a tree), or grow bamboo with a ground cover. Past the map's edge, Minecraft's own land has trees when the export keeps Surroundings on Vanilla."),
            );
        } else {
            let renewable = self.blocks_of(|k| matches!(k, Kind::Leaves | Kind::Sapling)) + self.plants_of(|k| matches!(k, Kind::Sapling | Kind::Wood)) > 0;
            if !renewable {
                out.push(
                    Finding::new(Warning, Topic::Wood, "Wood won't grow back", "The map's wood has no leaves to drop saplings, and no bamboo: once it is cut, it is gone.")
                        .fix("Use trees with their leaves."),
                );
            }
            if self.spawn_column.is_some() && self.near.trees < 3 {
                out.push(
                    Finding::new(
                        Warning,
                        Topic::Wood,
                        "Few trees near the spawn",
                        format!("Only {} trees stand within {SPAWN_REACH} blocks of the spawn: the first minutes are a long walk for wood.", self.near.trees),
                    )
                    .fix("Stand a few trees around the spawn point.")
                    .at(self.spawn_spot()),
                );
            }
            let title = if self.trees > 0 { format!("{} trees", thousands(self.trees)) } else { "Wood to start with".to_owned() };
            out.push(Finding::new(
                Good,
                Topic::Wood,
                title,
                format!("{} blocks of wood, about {:.1} trees a chunk.", thousands(wood), self.trees as f64 / chunks),
            ));
        }

        // ---- stone
        if self.blocks_of(|k| k == Kind::Stone) == 0 {
            out.push(
                Finding::new(Critical, Topic::Stone, "No stone for tools", "Nothing on the map mines into cobblestone: no stone tools, no furnace, no way past wood.")
                    .fix("Use Stone (or deepslate, blackstone) under the terrains."),
            );
        }

        // ---- ores
        let has = |m: Mineral| self.mineral(m).ore + self.mineral(m).riches > 0;
        // With ores laid, copper and emeralds may lack rock at their heights
        // (emeralds keep to mountains): only the others are missed.
        let wanted: &[Mineral] = if self.ores_on {
            &[Mineral::Coal, Mineral::Iron, Mineral::Gold, Mineral::Redstone, Mineral::Lapis, Mineral::Diamond]
        } else {
            &Mineral::OVERWORLD
        };
        let missing: Vec<Mineral> = wanted.iter().copied().filter(|&m| !has(m)).collect();
        if !missing.is_empty() {
            let title = if missing.len() == Mineral::OVERWORLD.len() {
                "No ores at all".to_owned()
            } else if missing.len() <= 2 {
                format!("No {}", missing.iter().map(|m| m.plural()).collect::<Vec<_>>().join(" or "))
            } else {
                format!("{} kinds of ore missing", missing.len())
            };
            let (detail, fix) = if !self.ores_on {
                (
                    "Ores are turned off for the export, and Minecraft adds none to the map's finished chunks: nothing to mine for torches, tools, armour or enchanting.".to_owned(),
                    "Turn ores on in Edit ▸ Ores…: the export lays them in the rock, as Minecraft does.",
                )
            } else {
                (
                    format!(
                        "Nothing to mine for {}: turned off, or no rock at their heights on this map.",
                        missing.iter().map(|m| m.use_for()).collect::<Vec<_>>().join(", ")
                    ),
                    "Turn them on or move their heights in Edit ▸ Ores….",
                )
            };
            let mut f = Finding::new(Warning, Topic::Ores, title, detail).fix(fix).leads_to(Action::Ores);
            f.chips = missing.iter().map(|m| (m.name().to_owned(), m.color())).collect();
            out.push(f);
        }
        // Those lying around in heaps are no ores to mine.
        let heaped: Vec<Mineral> = out
            .iter()
            .filter(|f| f.severity == Critical && matches!(f.topic, Topic::Riches | Topic::Ores))
            .filter_map(|f| f.spot.and_then(|s| Mineral::ALL.into_iter().find(|&m| self.spots.minerals[m as usize] == Some(s))))
            .collect();
        let present: Vec<Mineral> = Mineral::ALL.into_iter().filter(|&m| has(m) && !heaped.contains(&m)).collect();
        if !present.is_empty() {
            let ores: u64 = present.iter().map(|&m| self.mineral(m).ore).sum();
            let mut f = Finding::new(
                Good,
                Topic::Ores,
                "Ores to mine",
                format!("{} ore blocks, about {} a chunk.", thousands(ores), thousands((ores as f64 / chunks).round() as u64)),
            );
            f.chips = present.iter().map(|&m| (format!("{} {}", m.name(), thousands(self.mineral(m).ore + self.mineral(m).riches)), m.color())).collect();
            out.push(f.leads_to(Action::Ores));
        }

        // ---- water and lava
        let (water, lava) = (self.water(), self.lava());
        if self.lava_world {
            out.push(
                Finding::new(
                    Warning,
                    Topic::Lava,
                    "Seas of lava",
                    "The map's seas, lakes and rivers are lava: deadly to fall in, and no water to farm, fish or make obsidian with.",
                )
                .fix("Fine for a hardcore or nether-like map; for plain survival, make the world's fluid water."),
            );
        } else if water == 0 {
            out.push(
                Finding::new(Warning, Topic::Water, "No water", "No water anywhere: no farms, no fishing, nothing to turn lava into obsidian.")
                    .fix("Flood low ground (Flood tool) or dig a river (River tool)."),
            );
        } else {
            out.push(Finding::new(
                Good,
                Topic::Water,
                format!("Water on {} of the map", percent(self.wet, self.columns)),
                if self.above_sea > 0 {
                    format!(
                        "{} blocks of water, {} blocks deep at most; lakes and rivers above the sea on {} columns.",
                        thousands(water),
                        self.deepest,
                        thousands(self.above_sea)
                    )
                } else {
                    format!("{} blocks of water, {} blocks deep at most.", thousands(water), self.deepest)
                },
            ));
        }
        if self.nether() {
            out.push(Finding::new(Good, Topic::Nether, "A way to the Nether", "Lava, water and iron for a bucket, or obsidian and diamonds: players can make their portal."));
        } else {
            let mut lacks = Vec::new();
            if lava == 0 {
                lacks.push("lava");
            }
            if water == 0 {
                lacks.push("water");
            }
            if !has(Mineral::Iron) {
                lacks.push("iron for a bucket");
            }
            let title = if lava == 0 { "No lava for a Nether portal" } else { "No way to the Nether" };
            out.push(
                Finding::new(
                    Warning,
                    Topic::Nether,
                    title,
                    format!(
                        "A portal takes ten obsidian: lava turned by water carried in an iron bucket, or obsidian mined with a diamond pickaxe. Here there is no {}. The Nether holds the blaze rods that lead to the End.",
                        lacks.join(", no ")
                    ),
                )
                .fix("Put lava where players can reach it (a pool, a cave), or ten obsidian for a portal."),
            );
        }

        // ---- food
        let grass = self.surface_of(|k| k == Kind::Grass);
        if grass == 0 {
            out.push(
                Finding::new(Warning, Topic::Food, "No grass", "Animals only spawn on grass blocks: without grass there's no meat, leather or wool, and no seeds from grass plants.")
                    .fix("Paint Grass where animals should live."),
            );
        } else if grass * 50 < grounded {
            out.push(
                Finding::new(Warning, Topic::Food, "Little grass", format!("Grass covers only {} of the surface: animals will be rare.", percent(grass, grounded)))
                    .fix("Paint more Grass, around the spawn first."),
            );
        } else {
            out.push(Finding::new(
                Good,
                Topic::Food,
                format!("Grass on {} of the surface", percent(grass, grounded)),
                "Animals will spawn there over time: the export places none.",
            ));
        }

        // ---- the relief
        if self.lethal > 0 {
            let worst = self.worst_drop.map_or(0, |w| w.0);
            out.push(
                Finding::new(
                    if self.lethal * 200 >= self.columns { Warning } else { Info },
                    Topic::Drops,
                    "Deadly drops",
                    format!(
                        "{} columns drop {LETHAL_FALL} blocks or more straight down: a fall there kills a player at full health. The highest drop is {worst} blocks.",
                        thousands(self.lethal)
                    ),
                )
                .fix("Soften the cliffs where players will walk (Smooth, or Edit ▸ Whole map ▸ Thermal erosion).")
                .at(self.worst_drop.map(|w| (w.1, w.2))),
            );
        }
        if self.at_ceiling > 0 {
            out.push(
                Finding::new(
                    Warning,
                    Topic::Ceiling,
                    "Peaks cut off at the top of the world",
                    format!("{} columns reach y {}: their tops are flat, and nothing can be built over them.", thousands(self.at_ceiling), self.max_y),
                )
                .fix("Lower the relief (Edit ▸ Whole map ▸ Scale and raise), or make the world taller.")
                .at(self.spots.ceiling),
            );
        }
        if self.void == 0 {
            out.push(Finding::new(Good, Topic::Void, "A floor everywhere", "Bedrock lies under every column: no way out of the world."));
        }
        out.push(if self.layered == 0 && self.hollow == 0 {
            Finding::new(Info, Topic::Caves, "No caves", "The ground is solid all the way down: players dig their own way to the deep.")
        } else {
            Finding::new(
                Info,
                Topic::Caves,
                "Caves and overhangs",
                format!("{} columns have more than one floor, over {} blocks of air under the surface.", thousands(self.layered), thousands(self.hollow)),
            )
        });
        out.push(Finding::new(
            Info,
            Topic::Export,
            "Minecraft adds nothing inside the map",
            "The export writes every chunk finished: no village, mineshaft, stronghold or ruined portal will appear in it, and animals come only slowly, on grass. Monsters still come at night.",
        ));
        out.sort_by_key(|f| f.severity);
        out
    }

    /// The findings in a few sentences: what breaks survival, what is
    /// missing, what holds up.
    pub fn summary(findings: &[Finding]) -> String {
        let titles = |s: Severity| findings.iter().filter(|f| f.severity == s).map(|f| uncapitalised(&f.title)).collect::<Vec<_>>();
        let list = |v: &[String]| match v {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        let (bad, missing, fine) = (titles(Severity::Critical), titles(Severity::Warning), titles(Severity::Good));
        let mut parts = Vec::new();
        if !bad.is_empty() {
            parts.push(format!("Breaks survival: {}.", list(&bad)));
        }
        if !missing.is_empty() {
            parts.push(format!("Gaps: {}.", list(&missing)));
        }
        if bad.is_empty() && missing.is_empty() {
            parts.push("Nothing breaks survival and nothing is missing.".to_owned());
        }
        if !fine.is_empty() {
            parts.push(format!("Holding up: {}.", list(&fine)));
        }
        parts.join(" ")
    }

    /// The analysis as Markdown, to paste where people talk about the map.
    pub fn report(&self) -> String {
        use std::fmt::Write;
        let findings = self.findings();
        let verdict = Verdict::of(&findings);
        let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
        let mut md = String::new();
        let _ = writeln!(md, "# Survival check: {}\n", self.name);
        let _ = writeln!(
            md,
            "**{}** — {}, {}.\n",
            verdict.label(),
            plural(count(Severity::Critical) as u64, "problem"),
            plural(count(Severity::Warning) as u64, "warning")
        );
        let path = self.progression().iter().map(|s| format!("{} {}", mark(s.state), s.name)).collect::<Vec<_>>().join(" → ");
        let _ = writeln!(md, "{path}\n");
        let _ = writeln!(md, "{}\n", Analysis::summary(&findings));
        for (severity, heading) in [(Severity::Critical, "Problems"), (Severity::Warning, "Warnings"), (Severity::Good, "Fine"), (Severity::Info, "Worth knowing")] {
            let list: Vec<&Finding> = findings.iter().filter(|f| f.severity == severity).collect();
            if list.is_empty() {
                continue;
            }
            let _ = writeln!(md, "## {heading}\n");
            for f in list {
                let _ = write!(md, "- {} **{}** — {}", mark(f.severity), f.title, f.detail);
                if !f.chips.is_empty() {
                    let _ = write!(md, " ({})", f.chips.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join(", "));
                }
                if let Some(fix) = &f.fix {
                    let _ = write!(md, " *Fix: {fix}*");
                }
                if let Some((x, z)) = f.spot {
                    let _ = write!(md, " (at {x}, {z})");
                }
                let _ = writeln!(md);
            }
            let _ = writeln!(md);
        }
        let (w, d) = (self.area[2] - self.area[0], self.area[3] - self.area[1]);
        let _ = writeln!(md, "## The map\n");
        let _ = writeln!(md, "| | |\n|---|---|");
        let _ = writeln!(md, "| Size | {} × {} blocks, {} tiles, {} chunks |", thousands(w as u64), thousands(d as u64), thousands(self.tiles as u64), thousands(self.chunks().round() as u64));
        let _ = writeln!(md, "| Build limits | {} to {} |", self.min_y, self.max_y);
        let _ = writeln!(md, "| Surface | {} to {}, mean {:.0}, median {} |", self.lowest[1], self.highest[1], self.mean_height(), self.median_height());
        let _ = writeln!(md, "| Highest point | y {} at {}, {} |", self.highest[1], self.highest[0], self.highest[2]);
        let _ = writeln!(md, "| Sea level | {} |", self.sea_level);
        let _ = writeln!(md, "| Under water | {} |", percent(self.wet, self.columns));
        let _ = writeln!(md, "| Walkable | {} of the columns |", percent(self.steps[0] + self.steps[1], self.columns));
        let _ = writeln!(md, "| Solid blocks | {} |", thousands(self.solid()));
        let _ = writeln!(md, "| Trees | {} |", thousands(self.trees));
        let _ = writeln!(md, "| Spawn | {}, {}, {} |\n", self.spawn[0], self.spawn[1], self.spawn[2]);
        let _ = writeln!(md, "## Ores\n");
        let _ = writeln!(md, "| | Ore | Blocks | In the open |\n|---|---|---|---|");
        for c in &self.minerals {
            let _ = writeln!(md, "| {} | {} | {} | {} |", c.mineral.name(), thousands(c.ore), thousands(c.riches), thousands(c.exposed));
        }
        let _ = writeln!(md);
        for (heading, list, whole) in [("At the surface", &self.surface, self.grounded()), ("In the ground", &self.volume, self.solid()), ("Biomes", &self.biomes, self.columns)] {
            let _ = writeln!(md, "## {heading}\n");
            for c in list.iter().take(12) {
                let _ = writeln!(md, "- {} — {}", pretty(&c.id), percent(c.count, whole));
            }
            let _ = writeln!(md);
        }
        let _ = writeln!(md, "*Massif, every block counted as the export writes it, in {:.1} s.*", self.seconds);
        md
    }
}

fn mark(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "🔴",
        Severity::Warning => "🟠",
        Severity::Good => "🟢",
        Severity::Info => "⚪",
    }
}

/// `n` things, the word in the plural unless there is one: "1 problem",
/// "3 warnings".
pub fn plural(n: u64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{} {word}s", thousands(n)) }
}

fn uncapitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::block_top;
    use crate::coords::local_index;
    use crate::generate::{GenParams, generate};
    use crate::level::LevelKind;
    use crate::objects::{ObjectDef, ObjectLayerDef};
    use crate::tile::{NO_FLUID, Tile};

    fn small(p: GenParams) -> World {
        generate(&GenParams {
            tiles_x: 1,
            tiles_z: 1,
            ..p
        })
    }

    fn run(world: &World) -> Analysis {
        analyse(world, 0, &Steps::default()).expect("the map has ground")
    }

    fn by_name(list: &[Count]) -> HashMap<String, u64> {
        list.iter().map(|c| (c.id.clone(), c.count)).collect()
    }

    /// Block by block, as the export writes them: the runs of every column
    /// into a grid, then the ores laid over it as the export lays them
    /// (over the blocks as they stand, each vein seeing the ones before).
    fn every_block(world: &World) -> HashMap<String, u64> {
        const AIR: u16 = u16::MAX;
        const FLUID: u16 = u16::MAX - 1;
        const ORE: u16 = u16::MAX - 2;
        let d = &world.dims[0];
        let reg = &world.registry;
        let mat = Materializer::new(reg, world.meta.seed, d.min_y);
        let (min_y, max_y) = (d.min_y, d.max_y);
        let h = (max_y - min_y) as usize;
        let placer = crate::ores::Placer::new(&world.meta.ores, world.meta.seed);
        let hosts = crate::ores::hosts(reg);
        let pockets = crate::pockets::Pockets::new(reg, world.meta.seed as u64, d.min_y);
        let mut out: HashMap<String, u64> = HashMap::new();
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        for pos in d.ground().positions() {
            let stack = TileStack::new(d, pos, true).unwrap();
            let (ox, oz) = pos.origin();
            let mut grid = vec![AIR; TILE_AREA * h];
            let at = |cell: usize, y: i32| cell * h + (y - min_y) as usize;
            for cell in 0..TILE_AREA {
                let (x, z) = (ox + (cell % 128) as i32, oz + (cell / 128) as i32);
                stack.spans(cell, &mut spans);
                stack.fluids(cell, &spans, &mut fluids);
                for span in &spans {
                    let submerged = fluids.iter().any(|f| f.0 == span.y1);
                    let recipe = mat.recipe_depth(span.terrain, x, z);
                    let split = if recipe >= (span.y1 - span.y0) as u32 { span.y0 } else { span.y1 - recipe as i32 };
                    for y in span.y0..span.y1 {
                        grid[at(cell, y)] = if y < split { mat.substrate(x, y, z) } else { mat.block(span, x, y, z, submerged) }.0;
                    }
                }
                if let Some(col) = pockets.as_ref().and_then(|p| p.column(stack.ground().pockets.get(cell), &spans, stack.cave_ground(cell), &mat, x, z)) {
                    for y in col.low..=col.high {
                        if let Some(b) = col.at(x, y, z)
                            && grid[at(cell, y)] != AIR
                        {
                            grid[at(cell, y)] = b.0;
                        }
                    }
                }
                for &(f0, f1) in &fluids {
                    for y in f0..f1 {
                        grid[at(cell, y)] = FLUID;
                    }
                }
            }
            let mut ores: HashMap<usize, &'static str> = HashMap::new();
            if let Some(placer) = &placer {
                for cz in (oz >> 4)..(oz >> 4) + 8 {
                    for cx in (ox >> 4)..(ox >> 4) + 8 {
                        placer.chunk(cx, cz, |b| {
                            if b.y < min_y || b.y + 1 >= max_y {
                                return;
                            }
                            let cell = local_index(b.x, b.z);
                            let (here, above) = (grid[at(cell, b.y)], grid[at(cell, b.y + 1)]);
                            if above == AIR || above == FLUID {
                                return;
                            }
                            let Some(host) = hosts.get(here as usize).copied().flatten() else { return };
                            let (stone, deep) = crate::ores::ore_blocks(world.meta.ores.ores[b.ore as usize].mineral);
                            ores.insert(at(cell, b.y), if host == crate::ores::Host::Deepslate { deep } else { stone });
                            grid[at(cell, b.y)] = ORE;
                        });
                    }
                }
            }
            for (i, &b) in grid.iter().enumerate() {
                let name = match b {
                    AIR | FLUID => continue,
                    ORE => ores[&i].split('[').next().unwrap(),
                    b => reg.block(BlockId(b)).name(),
                };
                *out.entry(name.to_owned()).or_default() += 1;
            }
        }
        out
    }

    /// A map with every terrain of the registry, heights down to the floor
    /// and up to the top, water over some, a floating slab and a cave.
    fn varied() -> World {
        let mut world = small(GenParams::default());
        let terrains = world.registry.terrains.len() as u16;
        let d = &mut world.dims[0];
        let (min_y, max_y) = (d.min_y, d.max_y);
        let pos = TilePos::new(0, 0);
        let tile = d.ground_mut().tile_mut(pos).unwrap();
        for cell in 0..TILE_AREA {
            let k = cell as i32;
            tile.terrain.set(cell, (k % terrains as i32) as u16);
            let h = match k % 7 {
                0 => min_y as f32 + 2.0,
                1 => min_y as f32 + 6.0,
                2 => (max_y + 4) as f32,
                _ => 40.0 + (k % 61) as f32,
            };
            tile.top.set(cell, h);
            tile.fluid.set(cell, if k % 3 == 0 { 70 } else { NO_FLUID });
        }
        let slab = d.add_level("Island", LevelKind::Solid);
        let t = d.levels[slab].tile_mut_or_insert(pos, Tile::empty_slab);
        for cell in (0..TILE_AREA).step_by(5) {
            t.top.set(cell, 150.0);
            t.bottom.as_mut().unwrap().set(cell, 120.0);
            t.terrain.set(cell, (cell % terrains as usize) as u16);
        }
        let cave = d.add_level("Cave", LevelKind::Carve);
        let t = d.levels[cave].tile_mut_or_insert(pos, Tile::empty_slab);
        for cell in (0..TILE_AREA).step_by(11) {
            t.top.set(cell, 30.0);
            t.bottom.as_mut().unwrap().set(cell, 10.0);
        }
        world
    }

    /// The analysis counts the blocks the export writes, one for one.
    #[test]
    fn counts_every_block_as_the_export_writes_it() {
        let world = varied();
        let a = run(&world);
        assert_eq!(by_name(&a.volume), every_block(&world));
        assert_eq!(a.columns, TILE_AREA as u64);
        assert_eq!((a.grounded(), a.void), (a.columns, 0), "every column has ground");
        assert!(a.layered > 0 && a.hollow > 0, "the slab and the cave");
        assert!(a.at_ceiling > 0, "columns up to the top of the world");
    }

    /// Underground pockets are counted where the export lays them, in the
    /// place of the rock, with the ores in pockets of rock; pockets of
    /// diamond blocks break survival, as diamond blocks anywhere.
    #[test]
    fn pockets_are_counted_as_the_export_writes_them() {
        let mut world = varied();
        world.meta.ores = Default::default();
        let reg = &mut world.registry;
        let (andesite, gravel, diamond) = (reg.intern_block("minecraft:andesite", [136, 136, 136]), reg.intern_block("minecraft:gravel", [136, 126, 118]), reg.intern_block("minecraft:diamond_block", [98, 237, 228]));
        let rock = reg.add_pocket(crate::pockets::PocketDef::new("Rock", [0; 3], &[(andesite, 3), (gravel, 1)], 0.4, 5.0, -64, 100));
        let riches = reg.add_pocket(crate::pockets::PocketDef::new("Riches", [0; 3], &[(diamond, 1)], 0.02, 3.0, -64, 100));
        let tile = world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap();
        for cell in 0..TILE_AREA {
            tile.pockets.set(cell, if cell % 128 < 64 { rock } else { riches });
        }
        let a = run(&world);
        let counts = by_name(&a.volume);
        assert_eq!(counts, every_block(&world));
        assert!(counts["minecraft:andesite"] > 10_000 && counts["minecraft:gravel"] > 3_000 && counts["minecraft:diamond_block"] > 100, "{counts:?}");
        let f = a.findings();
        assert!(f.iter().any(|f| f.title == "Diamonds lying around"), "{:?}", f.iter().map(|f| &f.title).collect::<Vec<_>>());
    }

    /// Bedrock and diamond blocks painted on the surface break survival;
    /// the spot shown is where they are.
    #[test]
    fn riches_and_bedrock_in_the_open_are_red() {
        let mut world = small(GenParams::default());
        let reg = &mut world.registry;
        let bedrock = reg.terrain_by_name("Bedrock").unwrap();
        let diamond = reg.intern_block("minecraft:diamond_block", [98, 237, 228]);
        reg.terrains.push(crate::registry::TerrainDef {
            name: "Diamond Block".into(),
            group: "Blocks".into(),
            color: [98, 237, 228],
            strata: vec![Stratum { blocks: vec![(diamond, 1)], depth: (3, 5), mix: Default::default() }],
            submerged_top: None,
            biome_hint: None,
        });
        let diamonds = TerrainId(reg.terrains.len() as u16 - 1);
        let tile = world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap();
        for x in 10..20 {
            tile.terrain.set(local_index(x, 30), bedrock.0);
            tile.terrain.set(local_index(x, 40), diamonds.0);
        }
        let a = run(&world);
        let f = a.findings();
        let red: Vec<&str> = f.iter().filter(|f| f.severity == Severity::Critical).map(|f| f.title.as_str()).collect();
        assert!(red.contains(&"Diamonds lying around"), "{red:?}");
        assert!(red.contains(&"Bedrock where players walk"), "{red:?}");
        let diamonds = f.iter().find(|f| f.title == "Diamonds lying around").unwrap();
        assert_eq!(diamonds.spot.map(|s| s.1), Some(40));
        assert!(diamonds.evidence.as_deref().unwrap().starts_with("minecraft:diamond_block × "));
        let bedrock = f.iter().find(|f| f.title == "Bedrock where players walk").unwrap();
        assert_eq!(bedrock.spot.map(|s| s.1), Some(30));
        assert!(a.stray_bedrock > 0);
        assert_eq!(Verdict::of(&f), Verdict::NotReady);
    }

    /// A plain generated map: no ores (orange), no wood (red), water and
    /// grass (fine), a floor everywhere.
    #[test]
    fn a_plain_map_misses_ores_and_wood() {
        let mut world = small(GenParams::default());
        world.meta.ores.on = false;
        let a = run(&world);
        let f = a.findings();
        let find = |t: &str| f.iter().find(|f| f.title == t).unwrap_or_else(|| panic!("no “{t}” in {:?}", f.iter().map(|f| &f.title).collect::<Vec<_>>()));
        assert_eq!(find("No ores at all").severity, Severity::Warning);
        assert_eq!(find("No ores at all").chips.len(), 8);
        assert_eq!(find("No wood anywhere").severity, Severity::Critical);
        assert_eq!(find("A floor everywhere").severity, Severity::Good);
        assert_eq!(find("No lava for a Nether portal").severity, Severity::Warning);
        assert!(f.iter().any(|f| f.topic == Topic::Water && f.severity == Severity::Good));
        assert!(a.spawn_column.is_some(), "the spawn is on the map");
        // The worst first.
        assert!(f.windows(2).all(|w| w[0].severity <= w[1].severity));
        let stages = a.progression();
        assert_eq!(stages.iter().map(|s| s.state).collect::<Vec<_>>(), [Severity::Critical, Severity::Good, Severity::Warning, Severity::Warning, Severity::Warning]);
        let summary = Analysis::summary(&f);
        assert!(summary.starts_with("Breaks survival: no wood anywhere."), "{summary}");
        assert!(summary.contains("Gaps: no ores at all"), "{summary}");
        let md = a.report();
        assert!(md.starts_with("# Survival check: Generated World"), "{md}");
        assert!(md.contains("No ores at all"));
        assert_eq!(find("No ores at all").action, Some(Action::Ores));
    }

    /// With ores laid, as a new map has them: ores to mine at their
    /// heights, iron and diamonds on the way, none in the open.
    #[test]
    fn ores_are_laid_in_the_rock() {
        let world = small(GenParams::default());
        assert!(world.meta.ores.on, "a new map lays ores");
        let a = run(&world);
        let f = a.findings();
        assert!(!f.iter().any(|f| f.topic == Topic::Ores && f.severity == Severity::Warning), "{:?}", f.iter().map(|f| &f.title).collect::<Vec<_>>());
        assert!(f.iter().any(|f| f.title == "Ores to mine"));
        for m in [Mineral::Coal, Mineral::Iron, Mineral::Diamond, Mineral::Redstone] {
            let c = a.mineral(m);
            assert!(c.ore > 0 && c.exposed == 0, "{m:?}: {c:?}");
        }
        let names = by_name(&a.volume);
        // Diamonds deep down, most of them in deepslate; none high up.
        let (deep, stone) = (names.get("minecraft:deepslate_diamond_ore").copied().unwrap_or(0), names.get("minecraft:diamond_ore").copied().unwrap_or(0));
        assert!(deep > 10 * stone && deep > 0, "{names:?}");
        let stages = a.progression();
        assert_eq!(stages[2].state, Severity::Good, "iron");
        assert_eq!(stages[3].state, Severity::Good, "diamonds");
    }

    /// Carving down through the floor opens holes into the void.
    #[test]
    fn carving_through_the_floor_opens_the_void() {
        let mut world = small(GenParams::default());
        let d = &mut world.dims[0];
        let (min_y, pos) = (d.min_y, TilePos::new(0, 0));
        let cave = d.add_level("Pit", LevelKind::Carve);
        let t = d.levels[cave].tile_mut_or_insert(pos, Tile::empty_slab);
        for x in 0..4 {
            t.top.set(local_index(x, 7), 400.0);
            t.bottom.as_mut().unwrap().set(local_index(x, 7), (min_y - 1) as f32);
        }
        let a = run(&world);
        assert_eq!(a.void, 4);
        let f = a.findings();
        let void = f.iter().find(|f| f.title == "Holes into the void").expect("holes found");
        assert_eq!(void.severity, Severity::Critical);
        assert_eq!(void.spot.map(|s| s.1), Some(7));
        assert!(!f.iter().any(|f| f.title == "A floor everywhere"));
    }

    /// Trees of an object layer count, near the spawn too, and give wood.
    #[test]
    fn trees_are_counted() {
        let mut world = small(GenParams { landform: crate::generate::Landform::Flat, ..GenParams::default() });
        let (w, h, l) = (3usize, 5usize, 3usize);
        let palette = vec!["minecraft:air".to_owned(), "minecraft:oak_log[axis=y]".to_owned(), "minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]".to_owned()];
        let mut blocks = vec![0u16; w * h * l];
        for y in 0..h {
            for z in 0..l {
                for x in 0..w {
                    let i = x + z * w + y * w * l;
                    if x == 1 && z == 1 && y < 4 {
                        blocks[i] = 1;
                    } else if y >= 3 {
                        blocks[i] = 2;
                    }
                }
            }
        }
        let tree = ObjectDef::new("Oak", [w as u16, h as u16, l as u16], palette, blocks);
        world.registry.objects.push(tree);
        let mut layer = ObjectLayerDef::new("Oaks", &[0], &world.registry);
        layer.spacing = 8;
        layer.density = 1.0;
        layer.soil_only = false;
        let n = world.registry.add_object_layer(layer);
        let tile = world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap();
        tile.objects = crate::plane::Plane::Uniform(n);
        // The flat land a little over the sea, dry.
        tile.top = crate::plane::Plane::Uniform(70.0);
        tile.fluid = crate::plane::Plane::Uniform(NO_FLUID);
        world.meta.spawn = [64, block_top(70.0), 64];
        let a = run(&world);
        assert!(a.trees > 50, "{} trees", a.trees);
        assert_eq!(a.objects.first().map(|c| c.count), Some(a.trees));
        assert!(a.near.trees > 0);
        let logs = by_name(&a.volume).get("minecraft:oak_log").copied().unwrap_or(0);
        assert_eq!(logs, a.trees * 4);
        let f = a.findings();
        assert!(!f.iter().any(|f| f.title == "No wood anywhere"));
        assert!(!f.iter().any(|f| f.title == "Wood won't grow back"));
        assert_eq!(a.progression()[0].state, Severity::Good);
    }

    /// A lava world: the spawn under lava is red, the seas of lava orange.
    #[test]
    fn a_lava_world() {
        let mut world = small(GenParams { lava: true, landform: crate::generate::Landform::Flat, flat_height: 50.0, ..GenParams::default() });
        world.meta.spawn = [5, 60, 5];
        let a = run(&world);
        assert!(a.lava_world && a.lava() > 0 && a.water() == 0);
        let f = a.findings();
        assert!(f.iter().any(|f| f.title == "Spawn in lava" && f.severity == Severity::Critical), "{:?}", f.iter().map(|f| &f.title).collect::<Vec<_>>());
        assert!(f.iter().any(|f| f.title == "Seas of lava" && f.severity == Severity::Warning));
    }

    /// A stop asked for stops the pass.
    #[test]
    fn it_can_be_stopped() {
        let world = small(GenParams::default());
        let steps = Steps::default();
        steps.cancel.store(true, Ordering::Relaxed);
        assert!(analyse(&world, 0, &steps).is_none());
    }

    #[test]
    fn blocks_are_told_apart() {
        assert_eq!(kind_of("minecraft:deepslate_diamond_ore"), Kind::Ore(Mineral::Diamond));
        assert_eq!(kind_of("minecraft:stripped_oak_log[axis=x]"), Kind::Log);
        assert_eq!(kind_of("minecraft:mushroom_stem"), Kind::Other);
        assert_eq!(kind_of("minecraft:raw_iron_block"), Kind::Riches(Mineral::Iron));
        assert_eq!(kind_of("minecraft:bamboo[age=0]"), Kind::Wood);
        assert_eq!(pretty("minecraft:grass_block[snowy=false]"), "Grass Block");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(12), "12");
        assert_eq!(percent(1, 3), "33 %");
        assert_eq!(percent(1, 10000), "< 0.1 %");
        assert_eq!(percent(5, 100), "5.0 %");
    }
}
