//! The registry maps the small integer ids stored in tiles to their meaning:
//! Minecraft block states, terrain recipes and biomes.
//!
//! The registry is saved inside every world file, so ids never have to stay
//! stable across versions of the application.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u16);

        impl $name {
            #[inline]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(
    /// Index into [`Registry::blocks`]. `BlockId(0)` is always air.
    BlockId
);
id_type!(
    /// Index into [`Registry::terrains`]. `TerrainId(0)` is bare natural rock.
    TerrainId
);
id_type!(
    /// Index into [`Registry::biomes`]. `BiomeId(0)` means "automatic".
    BiomeId
);

impl BlockId {
    pub const AIR: BlockId = BlockId(0);
}

impl TerrainId {
    pub const ROCK: TerrainId = TerrainId(0);
}

impl BiomeId {
    pub const AUTO: BiomeId = BiomeId(0);
}

/// A Minecraft block state such as `minecraft:grass_block[snowy=false]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDef {
    /// Namespaced id with optional `[key=value,...]` properties.
    pub state: String,
    /// Colour used by the 3D preview for exposed faces.
    pub color: [u8; 3],
}

impl BlockDef {
    /// The namespaced block id without properties.
    pub fn name(&self) -> &str {
        self.state.split('[').next().unwrap_or(&self.state)
    }

    /// The `key=value` properties, in declaration order.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &str)> {
        let props = self
            .state
            .split_once('[')
            .map(|(_, rest)| rest.trim_end_matches(']'))
            .unwrap_or("");
        props
            .split(',')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.trim(), v.trim()))
    }
}

/// One layer of a terrain recipe, counted downward from the surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stratum {
    /// Weighted mix of blocks; one is picked per block from a positional hash.
    pub blocks: Vec<(BlockId, u16)>,
    /// Thickness range in blocks (inclusive); varies from column to column.
    pub depth: (u16, u16),
    /// How its blocks are laid out: each on its own, in patches, or in
    /// beds.
    #[serde(default, skip_serializing_if = "Mix::is_grain")]
    pub mix: Mix,
}

/// How the blocks of a [`Stratum`] are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Mix {
    /// Each block drawn on its own, by its weight.
    #[default]
    Grain,
    /// Patches of about `size` blocks across, each of one block drawn by
    /// its weight, their edges wavering.
    Patches { size: f32 },
    /// Horizontal beds, as in a cliff of sedimentary rock: the blocks in
    /// their order upwards, each as many blocks thick as its weight,
    /// repeating, dipping a little and wavering.
    Beds,
}

impl Mix {
    pub fn is_grain(&self) -> bool {
        *self == Mix::Grain
    }
}

/// How a painted terrain turns into blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainDef {
    pub name: String,
    /// Group shown in the terrain palette ("Grass & Soil", "Stone", ...).
    pub group: String,
    /// Map colour.
    pub color: [u8; 3],
    /// Layers from the surface down; below them comes the natural substrate
    /// (stone, then deepslate under Y = 0).
    pub strata: Vec<Stratum>,
    /// Replaces the surface block when the column is under a fluid (grass does
    /// not grow under water).
    #[serde(default)]
    pub submerged_top: Option<BlockId>,
    /// Biome used when the column's biome is left on automatic.
    #[serde(default)]
    pub biome_hint: Option<BiomeId>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BiomeDef {
    /// Namespaced biome id (`minecraft:plains`); empty for the automatic entry.
    pub id: String,
    pub name: String,
    /// Map colour.
    pub color: [u8; 3],
    /// The mod or data pack it comes from (`Regions Unexplored`); empty for
    /// the game's own and for an id typed by hand.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// The colour the game gives grass in it, once read from the game or
    /// its mod: the 3D view tints grass with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grass: Option<[u8; 3]>,
    /// The colour of leaves in it, likewise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foliage: Option<[u8; 3]>,
}

impl BiomeDef {
    /// A biome known by its id alone (typed by hand, or named by a mod's
    /// profile before its mod is read): named after its id, in a colour of
    /// its own drawn from it.
    pub fn of_id(id: &str) -> Self {
        BiomeDef { id: id.to_owned(), name: biome_display_name(id), color: biome_hash_color(id), ..Default::default() }
    }
}

/// What is wrong with a biome id typed by hand, if anything: it must be
/// `namespace:path`, of the letters the game allows (`a-z 0-9 _ - .`, and
/// `/` in the path).
pub fn check_biome_id(id: &str) -> Result<(), &'static str> {
    let Some((ns, path)) = id.split_once(':') else { return Err("namespace:biome") };
    let ok = |s: &str, slash: bool| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.') || slash && c == '/');
    if ok(ns, false) && ok(path, true) && !path.starts_with('/') && !path.ends_with('/') { Ok(()) } else { Err("a-z 0-9 _ - .") }
}

/// A biome's name from its id: `regions_unexplored:alpha_grove` is
/// `Alpha Grove`.
pub fn biome_display_name(id: &str) -> String {
    let path = id.rsplit(':').next().unwrap_or(id);
    let last = path.rsplit('/').next().unwrap_or(path);
    last.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A colour of its own for a biome the game gives none, drawn from its id:
/// a hue from its hash, of the strength of the game's biome colours.
pub fn biome_hash_color(id: &str) -> [u8; 3] {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let hue = (h % 360) as f32;
    let (s, v) = (0.45 + (h >> 16) as f32 % 30.0 / 100.0, 0.55 + (h >> 24) as f32 % 30.0 / 100.0);
    let c = v * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|k| ((k + m) * 255.0).round() as u8)
}

/// Blocks the exporter needs regardless of what terrains reference.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WellKnown {
    pub stone: BlockId,
    pub deepslate: BlockId,
    pub bedrock: BlockId,
    pub water: BlockId,
    pub lava: BlockId,
}

/// What grows over the ground where a cover is painted: grass, flowers,
/// snow, anything chosen. A tile's cover plane holds `0` for none, or a
/// cover's number: its index in [`Registry::covers`] plus one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverDef {
    pub name: String,
    /// Map colour.
    pub color: [u8; 3],
    /// How strongly its colour shows over the ground on the map, `0..=1`:
    /// in full where something grows, lighter where the ground is bare.
    #[serde(default = "half", skip_serializing_if = "is_half")]
    pub opacity: f32,
    /// What it places, by block state (`minecraft:poppy`), with weights:
    /// one is picked per column.
    pub plants: Vec<(String, u16)>,
    /// Share of the painted columns that get something, `0..=1`.
    pub density: f32,
    /// How many blocks high it grows, least and most (cane, cactus: 1 to
    /// 3). Tall plants are always two blocks.
    #[serde(default = "one_high")]
    pub height: [u8; 2],
    /// Only on soil — grass, dirt, podzol, moss, mud… — as plants need.
    #[serde(default)]
    pub soil_only: bool,
    /// Grows under water rather than in the open air.
    #[serde(default)]
    pub underwater: bool,
    /// Frost, as WorldPainter's: where it is painted, the top of the water
    /// turns to ice; on dry ground its plants grow as any cover's (snow).
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub frost: bool,
    /// Lava, as WorldPainter's layer of that name: the water where it is
    /// painted is lava.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub lava: bool,
    /// Locked: brushes neither paint another cover over its blocks nor rub
    /// it out (see [`Locks`]).
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub locked: bool,
    /// Its place among the covers in the Layers panel (0: by its number).
    #[serde(default, skip_serializing_if = "crate::registry::is_zero")]
    pub order: u32,
    /// Removed: no longer offered, and nothing grows where it was painted.
    #[serde(default)]
    pub removed: bool,
    /// Left off the map and the 3D view, to see the ground under it; the
    /// export writes it all the same.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub hidden: bool,
    /// The palette it is sorted into in the Layers panel ("": none, it
    /// stays in its section).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub palette: String,
}

/// For `skip_serializing_if`: a flag left off is not written.
pub fn is_false(b: &bool) -> bool {
    !*b
}

/// For `skip_serializing_if`: a number left at 0 is not written.
pub fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The slots of a block that brushes paint layers in, as far as locks go
/// (see `docs/layers.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Ground covers: plants, Frost, Lava.
    Cover,
    /// Object layers, base or sub-canopy.
    Objects,
    /// Layers of underground pockets.
    Pockets,
}

/// The layers locked in a registry, by their number in their plane: a
/// brush leaves their blocks as they are.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Locks {
    covers: Vec<bool>,
    objects: Vec<bool>,
    pockets: Vec<bool>,
}

impl Locks {
    /// The layers locked in `reg`.
    pub fn of(reg: &Registry) -> Self {
        let numbered = |locked: Vec<bool>| if locked.iter().any(|&l| l) { std::iter::once(false).chain(locked).collect() } else { Vec::new() };
        Self {
            covers: numbered(reg.covers.iter().map(|c| c.locked && !c.removed).collect()),
            objects: numbered(reg.object_layers.iter().map(|l| l.locked && !l.removed).collect()),
            pockets: numbered(reg.pockets.iter().map(|p| p.locked && !p.removed).collect()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.covers.is_empty() && self.objects.is_empty() && self.pockets.is_empty()
    }

    /// Whether the layer numbered `n` in `slot`'s plane is locked.
    #[inline]
    pub fn holds(&self, slot: Slot, n: u16) -> bool {
        let list = match slot {
            Slot::Cover => &self.covers,
            Slot::Objects => &self.objects,
            Slot::Pockets => &self.pockets,
        };
        list.get(n as usize).copied().unwrap_or(false)
    }
}

fn one_high() -> [u8; 2] {
    [1, 1]
}

/// A cover's opacity on the map, by default: as every cover showed before
/// it had one.
pub(crate) fn half() -> f32 {
    0.5
}

/// For `skip_serializing_if`: an opacity left as [`half`] is not written.
pub(crate) fn is_half(v: &f32) -> bool {
    *v == half()
}

/// Plants that stand two blocks high, their halves told apart by `half`.
pub const TALL_PLANTS: [&str; 9] = [
    "tall_grass",
    "large_fern",
    "sunflower",
    "lilac",
    "rose_bush",
    "peony",
    "tall_seagrass",
    "pitcher_plant",
    "small_dripleaf",
];

/// Whether a block (`minecraft:tall_grass`, or with properties) is a plant
/// two blocks high: one of the game's, or any block given as its lower
/// half (`conquest:dead_bush_tall[half=lower]`, as a mod's tall plants are
/// kept), written as its two halves.
pub fn is_tall_plant(state: &str) -> bool {
    let name = state.split('[').next().unwrap_or(state);
    TALL_PLANTS.contains(&name.strip_prefix("minecraft:").unwrap_or(name)) || state.split_once('[').is_some_and(|(_, props)| props.trim_end_matches(']').split(',').any(|kv| kv.trim() == "half=lower"))
}

/// Soil that plants grow on (by block, with or without properties).
pub fn is_soil(state: &str) -> bool {
    let name = state.split('[').next().unwrap_or(state);
    match name.strip_prefix("minecraft:") {
        Some(short) => matches!(
            short,
            "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" | "moss_block" | "pale_moss_block" | "mud" | "muddy_mangrove_roots" | "mycelium" | "farmland" | "dirt_path"
        ),
        // A mod's ground, by its name: Conquest Reforged's grass, moss,
        // forest floors, dirts, soils, earths, silts, muds and peat — not
        // the bricks, slabs or walls made of them.
        None if name.contains(':') => {
            let short = name.rsplit(':').next().unwrap_or(name);
            const GROUND: [&str; 17] = ["grass", "grassy", "dirt", "soil", "moss", "earth", "silt", "silty", "loam", "loamy", "peat", "mud", "podzol", "mycelium", "farmland", "humus", "turf"];
            const MADE: [&str; 17] = ["brick", "bricks", "slab", "stairs", "wall", "tile", "tiles", "plaster", "pillar", "fence", "pane", "layer", "layers", "carpet", "path", "arch", "beam"];
            let words = || short.split('_');
            (words().any(|w| GROUND.contains(&w)) || short.contains("forest_floor") || short.contains("shredded_leaves")) && !words().any(|w| MADE.contains(&w))
        }
        None => false,
    }
}

/// The colour a plant shows from above; `None` for a block not known
/// here (its cover's colour stands for it).
pub fn plant_color(state: &str) -> Option<[u8; 3]> {
    let name = state.split('[').next().unwrap_or(state);
    Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
        "short_grass" | "grass" | "tall_grass" | "fern" | "large_fern" | "bush" => [88, 146, 54],
        "dandelion" | "sunflower" | "torchflower" => [236, 206, 52],
        "poppy" | "red_tulip" | "rose_bush" => [200, 44, 38],
        "cornflower" | "blue_orchid" => [72, 112, 222],
        "allium" | "lilac" | "pitcher_plant" => [178, 112, 212],
        "azure_bluet" | "oxeye_daisy" | "white_tulip" | "lily_of_the_valley" => [238, 238, 230],
        "orange_tulip" => [234, 130, 42],
        "pink_tulip" | "peony" | "pink_petals" | "wildflowers" => [238, 158, 198],
        "wither_rose" => [44, 42, 32],
        "brown_mushroom" => [152, 114, 84],
        "red_mushroom" => [202, 48, 42],
        "dead_bush" | "short_dry_grass" | "tall_dry_grass" => [152, 118, 68],
        "snow" => [244, 250, 252],
        "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant" => [42, 112, 62],
        "moss_carpet" => [89, 109, 45],
        "sweet_berry_bush" => [62, 102, 52],
        "sugar_cane" => [132, 172, 92],
        "cactus" => [62, 122, 52],
        "leaf_litter" => [160, 110, 60],
        _ => return None,
    })
}

/// What a cover grows at one column: its plant, how high.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Growth {
    /// Index into [`CoverDef::plants`].
    pub plant: usize,
    pub height: u8,
}

impl CoverDef {
    /// A cover of one block, or of the blocks given with their weights.
    pub fn new(name: &str, color: [u8; 3], plants: &[(&str, u16)], density: f32) -> Self {
        Self {
            name: name.to_owned(),
            color,
            opacity: half(),
            plants: plants.iter().map(|(b, w)| ((*b).to_owned(), *w)).collect(),
            density,
            height: one_high(),
            soil_only: true,
            underwater: false,
            frost: false,
            lava: false,
            locked: false,
            order: 0,
            removed: false,
            hidden: false,
            palette: String::new(),
        }
    }

    /// Frost, as WorldPainter's layer of that name: snow on the ground, ice
    /// on the water.
    pub fn frost() -> Self {
        let mut frost = CoverDef::new("Frost", [226, 238, 250], &[("minecraft:snow[layers=1]", 1)], 1.0);
        frost.soil_only = false;
        frost.frost = true;
        frost
    }

    /// Lava, as WorldPainter's layer of that name: the water where it is
    /// painted is lava. It grows nothing.
    pub fn lava() -> Self {
        let mut lava = CoverDef::new("Lava", [226, 96, 24], &[], 1.0);
        lava.soil_only = false;
        lava.lava = true;
        lava
    }

    /// Whether it turns the water under it into something else (ice,
    /// lava).
    pub fn changes_water(&self) -> bool {
        self.frost || self.lava
    }

    /// What grows at column `(x, z)` of a world of seed `seed`, or `None`
    /// where the cover leaves the ground bare. The same everywhere it is
    /// asked: the map, the 3D view and the export agree.
    pub fn growth(&self, x: i32, z: i32, seed: u64) -> Option<Growth> {
        use crate::compose::hash2;
        if self.removed {
            return None;
        }
        let density = self.density.clamp(0.0, 1.0);
        if density < 1.0 && hash2(x, z, seed ^ 0x636f_7665_7231) as f32 >= density * u32::MAX as f32 {
            return None;
        }
        let total: u32 = self.plants.iter().map(|p| p.1 as u32).sum();
        if total == 0 {
            return None;
        }
        let mut r = hash2(x, z, seed ^ 0x636f_7665_7232) % total;
        let plant = self.plants.iter().position(|p| {
            let w = p.1 as u32;
            if r < w {
                true
            } else {
                r -= w;
                false
            }
        })?;
        let (lo, hi) = (self.height[0].max(1), self.height[1].max(1));
        let height = if hi > lo { lo + (hash2(x, z, seed ^ 0x636f_7665_7233) % (hi - lo + 1) as u32) as u8 } else { lo };
        Some(Growth { plant, height })
    }
}

/// The covers of a new world.
pub fn vanilla_covers() -> Vec<CoverDef> {
    let mut snow = CoverDef::new("Snow", [242, 248, 252], &[("minecraft:snow[layers=1]", 1)], 1.0);
    snow.soil_only = false;
    let mut dead = CoverDef::new("Dead bushes", [158, 124, 76], &[("minecraft:dead_bush", 1)], 0.03);
    dead.soil_only = false;
    let mut seagrass = CoverDef::new("Seagrass", [44, 124, 92], &[("minecraft:seagrass", 5), ("minecraft:tall_seagrass", 2)], 0.4);
    seagrass.soil_only = false;
    seagrass.underwater = true;
    vec![
        CoverDef::new("Grass", [118, 172, 72], &[("minecraft:short_grass", 12), ("minecraft:tall_grass", 2), ("minecraft:fern", 1)], 0.5),
        CoverDef::new("Tall grass", [92, 152, 54], &[("minecraft:tall_grass", 4), ("minecraft:short_grass", 2), ("minecraft:large_fern", 1)], 0.7),
        CoverDef::new(
            "Flowers",
            [214, 168, 186],
            &[
                ("minecraft:short_grass", 10),
                ("minecraft:dandelion", 2),
                ("minecraft:poppy", 2),
                ("minecraft:oxeye_daisy", 1),
                ("minecraft:azure_bluet", 1),
                ("minecraft:cornflower", 1),
                ("minecraft:allium", 1),
                ("minecraft:red_tulip", 1),
                ("minecraft:white_tulip", 1),
                ("minecraft:orange_tulip", 1),
                ("minecraft:pink_tulip", 1),
            ],
            0.45,
        ),
        CoverDef::new("Ferns", [72, 130, 64], &[("minecraft:fern", 4), ("minecraft:large_fern", 2), ("minecraft:short_grass", 2)], 0.45),
        CoverDef::new("Mushrooms", [168, 112, 92], &[("minecraft:brown_mushroom", 1), ("minecraft:red_mushroom", 1)], 0.05),
        dead,
        snow,
        seagrass,
        CoverDef::frost(),
        CoverDef::lava(),
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub blocks: Vec<BlockDef>,
    pub terrains: Vec<TerrainDef>,
    pub biomes: Vec<BiomeDef>,
    pub well_known: WellKnown,
    /// Ground covers, numbered from 1 (0 in a tile: none). Worlds made
    /// before covers existed get those of a new world.
    #[serde(default = "vanilla_covers")]
    pub covers: Vec<CoverDef>,
    /// Custom objects, read from schematics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<crate::objects::ObjectDef>,
    /// Object layers, numbered from 1 (0 in a tile: none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_layers: Vec<crate::objects::ObjectLayerDef>,
    /// Layers of underground pockets, numbered from 1 (0 in a tile: none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pockets: Vec<crate::pockets::PocketDef>,
    /// Combined layers, numbered from 1: brushes that paint several layers
    /// at once (no plane of their own).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub combined: Vec<crate::combined::CombinedDef>,
    /// The palettes of the Layers panel, in their order: named groups the
    /// layers are sorted into, as WorldPainter's. A layer names its own
    /// (see [`CoverDef::palette`]); one named by a layer but missing here
    /// is shown after these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palettes: Vec<String>,
    /// The tree sizes Tree sizes paints and objects are sorted into,
    /// numbered from 1 in the `sizes` plane of a tile: the four of every
    /// map (Bushes, Small, Medium and Large trees), then the map's own.
    #[serde(default = "vanilla_tree_sizes", skip_serializing_if = "is_vanilla_tree_sizes")]
    pub tree_sizes: Vec<TreeSize>,
}

/// A size of tree: a mask Tree sizes paints, and that each object of a
/// layer belongs to (see [`crate::objects::ObjectDef::tree_size`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeSize {
    /// Its name: one of the four of every map in English (shown in the
    /// language of the interface), or the user's own.
    pub name: String,
    /// Its colour over the map.
    pub color: [u8; 3],
    /// Removed: no longer offered; where it was painted, and for the
    /// objects set to it, any size stands. (Its number stays taken, so that
    /// an undo finds what it painted.)
    #[serde(default, skip_serializing_if = "is_false")]
    pub removed: bool,
}

/// The four tree sizes of every map.
pub fn vanilla_tree_sizes() -> Vec<TreeSize> {
    [("Bushes", [214, 226, 124]), ("Small trees", [140, 196, 86]), ("Medium trees", [60, 142, 70]), ("Large trees", [22, 86, 56])].into_iter().map(|(name, color)| TreeSize { name: name.to_owned(), color, removed: false }).collect()
}

fn is_vanilla_tree_sizes(sizes: &Vec<TreeSize>) -> bool {
    *sizes == vanilla_tree_sizes()
}

impl Registry {
    pub fn block(&self, id: BlockId) -> &BlockDef {
        self.blocks.get(id.index()).unwrap_or(&self.blocks[0])
    }

    pub fn terrain(&self, id: TerrainId) -> &TerrainDef {
        self.terrains.get(id.index()).unwrap_or(&self.terrains[0])
    }

    pub fn biome(&self, id: BiomeId) -> &BiomeDef {
        self.biomes.get(id.index()).unwrap_or(&self.biomes[0])
    }

    /// The cover numbered `n` in a tile's cover plane; `None` for 0, for a
    /// number past the list, and for a removed cover.
    pub fn cover(&self, n: u16) -> Option<&CoverDef> {
        let def = self.covers.get((n as usize).checked_sub(1)?)?;
        (!def.removed).then_some(def)
    }

    /// The number of the cover of that name (not removed).
    pub fn cover_by_name(&self, name: &str) -> Option<u16> {
        self.covers.iter().position(|c| !c.removed && c.name.eq_ignore_ascii_case(name)).map(|i| i as u16 + 1)
    }

    /// Add a cover and return its number.
    pub fn add_cover(&mut self, def: CoverDef) -> u16 {
        self.covers.push(def);
        self.covers.len() as u16
    }

    /// The object layer numbered `n` in a tile's object plane; `None` for
    /// 0, for a number past the list, and for a removed layer.
    /// Tree size `n` (from 1), if the map has it (and it was not removed).
    pub fn tree_size(&self, n: u16) -> Option<&TreeSize> {
        self.tree_sizes.get((n as usize).checked_sub(1)?).filter(|t| !t.removed)
    }

    /// Size `n` as placements take it: 0 (any) for one the map does not
    /// have.
    pub fn live_size(&self, n: u16) -> u16 {
        if self.tree_size(n).is_some() { n } else { 0 }
    }

    pub fn object_layer(&self, n: u16) -> Option<&crate::objects::ObjectLayerDef> {
        let def = self.object_layers.get((n as usize).checked_sub(1)?)?;
        (!def.removed).then_some(def)
    }

    /// The pocket layer numbered `n` in a tile's pockets plane; `None` for
    /// 0, for a number past the list, and for a removed layer.
    pub fn pocket(&self, n: u16) -> Option<&crate::pockets::PocketDef> {
        self.pockets.get((n as usize).checked_sub(1)?).filter(|p| !p.removed)
    }

    /// Add a pocket layer and return its number.
    pub fn add_pocket(&mut self, def: crate::pockets::PocketDef) -> u16 {
        self.pockets.push(def);
        self.pockets.len() as u16
    }

    /// The combined layer numbered `n`; `None` for 0, a number past the
    /// list, and a removed layer.
    pub fn combined_layer(&self, n: u16) -> Option<&crate::combined::CombinedDef> {
        self.combined.get((n as usize).checked_sub(1)?).filter(|c| !c.removed)
    }

    /// Add a combined layer and return its number.
    pub fn add_combined(&mut self, def: crate::combined::CombinedDef) -> u16 {
        self.combined.push(def);
        self.combined.len() as u16
    }

    /// Add an object layer and return its number.
    pub fn add_object_layer(&mut self, def: crate::objects::ObjectLayerDef) -> u16 {
        self.object_layers.push(def);
        self.object_layers.len() as u16
    }

    /// Find a block by its exact state string, registering it if needed.
    pub fn intern_block(&mut self, state: &str, color: [u8; 3]) -> BlockId {
        if let Some(i) = self.blocks.iter().position(|b| b.state == state) {
            return BlockId(i as u16);
        }
        self.blocks.push(BlockDef {
            state: state.to_owned(),
            color,
        });
        BlockId((self.blocks.len() - 1) as u16)
    }

    pub fn terrain_by_name(&self, name: &str) -> Option<TerrainId> {
        self.terrains
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
            .map(|i| TerrainId(i as u16))
    }

    pub fn biome_by_id(&self, id: &str) -> Option<BiomeId> {
        self.biomes
            .iter()
            .position(|b| b.id == id)
            .map(|i| BiomeId(i as u16))
    }

    /// Find a biome by its id, adding `def` the first time.
    pub fn intern_biome(&mut self, def: BiomeDef) -> BiomeId {
        if let Some(b) = self.biome_by_id(&def.id) {
            return b;
        }
        self.biomes.push(def);
        BiomeId((self.biomes.len() - 1) as u16)
    }

    /// The built-in registry of a new world.
    pub fn vanilla() -> Self {
        let mut reg = Registry {
            blocks: vec![BlockDef {
                state: "minecraft:air".into(),
                color: [0, 0, 0],
            }],
            terrains: Vec::new(),
            biomes: Vec::new(),
            covers: vanilla_covers(),
            objects: Vec::new(),
            object_layers: Vec::new(),
            pockets: Vec::new(),
            combined: Vec::new(),
            palettes: Vec::new(),
            tree_sizes: vanilla_tree_sizes(),
            well_known: WellKnown {
                stone: BlockId(0),
                deepslate: BlockId(0),
                bedrock: BlockId(0),
                water: BlockId(0),
                lava: BlockId(0),
            },
        };

        // ---- biomes (index 0 = automatic) --------------------------------
        const BIOMES: &[(&str, &str, [u8; 3])] = &[
            ("", "Automatic", [128, 128, 128]),
            ("minecraft:plains", "Plains", [141, 179, 96]),
            ("minecraft:sunflower_plains", "Sunflower Plains", [181, 219, 136]),
            ("minecraft:meadow", "Meadow", [131, 187, 109]),
            ("minecraft:forest", "Forest", [5, 102, 33]),
            ("minecraft:flower_forest", "Flower Forest", [45, 142, 73]),
            ("minecraft:birch_forest", "Birch Forest", [48, 116, 68]),
            ("minecraft:dark_forest", "Dark Forest", [64, 81, 26]),
            ("minecraft:pale_garden", "Pale Garden", [150, 156, 150]),
            ("minecraft:cherry_grove", "Cherry Grove", [237, 164, 205]),
            ("minecraft:taiga", "Taiga", [11, 102, 89]),
            ("minecraft:old_growth_pine_taiga", "Old Growth Pine Taiga", [89, 102, 81]),
            ("minecraft:snowy_plains", "Snowy Plains", [235, 240, 245]),
            ("minecraft:snowy_taiga", "Snowy Taiga", [49, 85, 74]),
            ("minecraft:grove", "Grove", [198, 214, 208]),
            ("minecraft:snowy_slopes", "Snowy Slopes", [214, 226, 234]),
            ("minecraft:frozen_peaks", "Frozen Peaks", [176, 198, 222]),
            ("minecraft:jagged_peaks", "Jagged Peaks", [202, 208, 218]),
            ("minecraft:stony_peaks", "Stony Peaks", [136, 136, 132]),
            ("minecraft:windswept_hills", "Windswept Hills", [96, 104, 96]),
            ("minecraft:savanna", "Savanna", [189, 178, 95]),
            ("minecraft:desert", "Desert", [250, 148, 24]),
            ("minecraft:badlands", "Badlands", [217, 69, 21]),
            ("minecraft:jungle", "Jungle", [83, 123, 9]),
            ("minecraft:bamboo_jungle", "Bamboo Jungle", [118, 142, 20]),
            ("minecraft:swamp", "Swamp", [7, 249, 178]),
            ("minecraft:mangrove_swamp", "Mangrove Swamp", [103, 165, 91]),
            ("minecraft:mushroom_fields", "Mushroom Fields", [255, 0, 255]),
            ("minecraft:beach", "Beach", [250, 222, 85]),
            ("minecraft:snowy_beach", "Snowy Beach", [250, 240, 192]),
            ("minecraft:stony_shore", "Stony Shore", [162, 162, 132]),
            ("minecraft:river", "River", [60, 100, 255]),
            ("minecraft:frozen_river", "Frozen River", [160, 160, 255]),
            ("minecraft:ocean", "Ocean", [20, 40, 160]),
            ("minecraft:deep_ocean", "Deep Ocean", [10, 20, 100]),
            ("minecraft:warm_ocean", "Warm Ocean", [20, 150, 200]),
            ("minecraft:lukewarm_ocean", "Lukewarm Ocean", [20, 100, 180]),
            ("minecraft:cold_ocean", "Cold Ocean", [32, 32, 112]),
            ("minecraft:frozen_ocean", "Frozen Ocean", [112, 112, 214]),
            ("minecraft:dripstone_caves", "Dripstone Caves", [140, 110, 80]),
            ("minecraft:lush_caves", "Lush Caves", [120, 170, 40]),
            ("minecraft:deep_dark", "Deep Dark", [12, 34, 44]),
            ("minecraft:the_void", "The Void", [30, 30, 30]),
        ];
        for (id, name, color) in BIOMES {
            reg.biomes.push(BiomeDef {
                id: (*id).into(),
                name: (*name).into(),
                color: *color,
                ..Default::default()
            });
        }

        // ---- blocks -------------------------------------------------------
        let mut b = |state: &str, color: [u8; 3]| reg.intern_block(state, color);
        let stone = b("minecraft:stone", [125, 125, 125]);
        let deepslate = b("minecraft:deepslate[axis=y]", [80, 80, 82]);
        let bedrock = b("minecraft:bedrock", [85, 85, 85]);
        let water = b("minecraft:water[level=0]", [63, 118, 228]);
        let lava = b("minecraft:lava[level=0]", [207, 92, 15]);
        let grass = b("minecraft:grass_block[snowy=false]", [96, 148, 62]);
        let dirt = b("minecraft:dirt", [134, 96, 67]);
        let coarse_dirt = b("minecraft:coarse_dirt", [119, 85, 59]);
        let podzol = b("minecraft:podzol[snowy=false]", [91, 63, 24]);
        let mycelium = b("minecraft:mycelium[snowy=false]", [111, 99, 105]);
        let moss = b("minecraft:moss_block", [89, 109, 45]);
        let mud = b("minecraft:mud", [60, 57, 61]);
        let clay = b("minecraft:clay", [160, 166, 179]);
        let sand = b("minecraft:sand", [219, 207, 163]);
        let sandstone = b("minecraft:sandstone", [216, 203, 155]);
        let red_sand = b("minecraft:red_sand", [190, 102, 33]);
        let red_sandstone = b("minecraft:red_sandstone", [186, 99, 29]);
        let gravel = b("minecraft:gravel", [136, 126, 126]);
        let cobblestone = b("minecraft:cobblestone", [127, 127, 127]);
        let mossy_cobblestone = b("minecraft:mossy_cobblestone", [110, 118, 94]);
        let andesite = b("minecraft:andesite", [136, 136, 136]);
        let diorite = b("minecraft:diorite", [188, 188, 188]);
        let granite = b("minecraft:granite", [149, 103, 85]);
        let calcite = b("minecraft:calcite", [223, 224, 220]);
        let tuff = b("minecraft:tuff", [108, 109, 102]);
        let basalt = b("minecraft:smooth_basalt", [72, 72, 78]);
        let blackstone = b("minecraft:blackstone", [42, 36, 41]);
        let obsidian = b("minecraft:obsidian", [15, 10, 24]);
        let terracotta = b("minecraft:terracotta", [152, 94, 67]);
        let snow = b("minecraft:snow_block", [249, 254, 254]);
        let powder_snow = b("minecraft:powder_snow", [248, 253, 253]);
        let ice = b("minecraft:packed_ice", [141, 180, 250]);
        let blue_ice = b("minecraft:blue_ice", [116, 167, 253]);
        let netherrack = b("minecraft:netherrack", [97, 38, 38]);
        let soul_sand = b("minecraft:soul_sand", [81, 62, 50]);
        let end_stone = b("minecraft:end_stone", [219, 222, 158]);
        reg.well_known = WellKnown {
            stone,
            deepslate,
            bedrock,
            water,
            lava,
        };

        // ---- terrains (index 0 = bare natural rock) -----------------------
        let biome = |id: &str| reg.biome_by_id(id);
        let plains = biome("minecraft:plains");
        let desert = biome("minecraft:desert");
        let badlands = biome("minecraft:badlands");
        let snowy = biome("minecraft:snowy_plains");
        let stony = biome("minecraft:stony_peaks");
        let frozen = biome("minecraft:frozen_peaks");
        let mushroom = biome("minecraft:mushroom_fields");
        let taiga = biome("minecraft:old_growth_pine_taiga");
        let swamp = biome("minecraft:swamp");

        let one = |block: BlockId, lo: u16, hi: u16| Stratum {
            blocks: vec![(block, 1)],
            depth: (lo, hi),
            mix: Mix::Grain,
        };
        let mix = |blocks: &[(BlockId, u16)], lo: u16, hi: u16| Stratum {
            blocks: blocks.to_vec(),
            depth: (lo, hi),
            mix: Mix::Grain,
        };
        const DEEP: u16 = u16::MAX;

        let mut t = |name: &str,
                     group: &str,
                     color: [u8; 3],
                     strata: Vec<Stratum>,
                     submerged_top: Option<BlockId>,
                     biome_hint: Option<BiomeId>| {
            reg.terrains.push(TerrainDef {
                name: name.into(),
                group: group.into(),
                color,
                strata,
                submerged_top,
                biome_hint,
            });
        };

        t("Rock", "Stone", [118, 118, 118], vec![], None, stony);
        t(
            "Grass",
            "Grass & Soil",
            [96, 148, 62],
            vec![one(grass, 1, 1), one(dirt, 2, 4)],
            Some(dirt),
            plains,
        );
        t("Dirt", "Grass & Soil", [134, 96, 67], vec![one(dirt, 3, 5)], None, plains);
        t(
            "Coarse Dirt",
            "Grass & Soil",
            [119, 85, 59],
            vec![one(coarse_dirt, 1, 2), one(dirt, 2, 3)],
            None,
            plains,
        );
        t(
            "Podzol",
            "Grass & Soil",
            [91, 63, 24],
            vec![one(podzol, 1, 1), one(dirt, 2, 4)],
            Some(dirt),
            taiga,
        );
        t(
            "Mycelium",
            "Grass & Soil",
            [111, 99, 105],
            vec![one(mycelium, 1, 1), one(dirt, 2, 4)],
            Some(dirt),
            mushroom,
        );
        t(
            "Moss",
            "Grass & Soil",
            [89, 109, 45],
            vec![one(moss, 1, 1), one(dirt, 2, 3)],
            None,
            plains,
        );
        t("Mud", "Grass & Soil", [60, 57, 61], vec![one(mud, 2, 4)], None, swamp);
        t("Clay", "Grass & Soil", [160, 166, 179], vec![one(clay, 2, 4)], None, None);

        t("Stone", "Stone", [125, 125, 125], vec![one(stone, DEEP, DEEP)], None, stony);
        t(
            "Rocky",
            "Stone",
            [122, 124, 118],
            vec![mix(
                &[(stone, 6), (cobblestone, 3), (andesite, 2), (mossy_cobblestone, 1)],
                3,
                5,
            )],
            None,
            stony,
        );
        t("Cobblestone", "Stone", [127, 127, 127], vec![one(cobblestone, 3, 4)], None, stony);
        t("Andesite", "Stone", [136, 136, 136], vec![one(andesite, 6, 10)], None, stony);
        t("Diorite", "Stone", [188, 188, 188], vec![one(diorite, 6, 10)], None, stony);
        t("Granite", "Stone", [149, 103, 85], vec![one(granite, 6, 10)], None, stony);
        t(
            "Deepslate",
            "Stone",
            [80, 80, 82],
            vec![one(deepslate, DEEP, DEEP)],
            None,
            stony,
        );
        t("Tuff", "Stone", [108, 109, 102], vec![one(tuff, 6, 10)], None, stony);
        t("Calcite", "Stone", [223, 224, 220], vec![one(calcite, 4, 8)], None, stony);
        t("Basalt", "Stone", [72, 72, 78], vec![one(basalt, 6, 10)], None, stony);
        t("Blackstone", "Stone", [42, 36, 41], vec![one(blackstone, 6, 10)], None, stony);
        t("Obsidian", "Stone", [15, 10, 24], vec![one(obsidian, 2, 4)], None, None);
        t("Bedrock", "Stone", [60, 60, 60], vec![one(bedrock, DEEP, DEEP)], None, None);

        t(
            "Sand",
            "Sand & Desert",
            [219, 207, 163],
            vec![one(sand, 3, 5), one(sandstone, 3, 6)],
            None,
            desert,
        );
        t(
            "Red Sand",
            "Sand & Desert",
            [190, 102, 33],
            vec![one(red_sand, 3, 5), one(red_sandstone, 3, 6)],
            None,
            badlands,
        );
        t(
            "Sandstone",
            "Sand & Desert",
            [216, 203, 155],
            vec![one(sandstone, 6, 10)],
            None,
            desert,
        );
        t("Gravel", "Sand & Desert", [136, 126, 126], vec![one(gravel, 3, 5)], None, None);
        t(
            "Terracotta",
            "Sand & Desert",
            [152, 94, 67],
            vec![one(terracotta, 8, 14)],
            None,
            badlands,
        );

        t("Snow", "Snow & Ice", [240, 250, 250], vec![one(snow, 2, 4)], None, snowy);
        t(
            "Powder Snow",
            "Snow & Ice",
            [228, 240, 246],
            vec![one(powder_snow, 1, 2), one(snow, 2, 3)],
            None,
            snowy,
        );
        t("Ice", "Snow & Ice", [141, 180, 250], vec![one(ice, 3, 6)], None, frozen);
        t("Blue Ice", "Snow & Ice", [116, 167, 253], vec![one(blue_ice, 3, 6)], None, frozen);

        t(
            "Netherrack",
            "Other Dimensions",
            [97, 38, 38],
            vec![one(netherrack, DEEP, DEEP)],
            None,
            None,
        );
        t(
            "Soul Sand",
            "Other Dimensions",
            [81, 62, 50],
            vec![one(soul_sand, 3, 5), one(netherrack, DEEP, DEEP)],
            None,
            None,
        );
        t(
            "End Stone",
            "Other Dimensions",
            [219, 222, 158],
            vec![one(end_stone, DEEP, DEEP)],
            None,
            None,
        );

        reg
    }
}

#[cfg(test)]
mod tests {

    /// Biomes of mods and typed by hand: added once by their ids, named and
    /// coloured from them, and files of before them still read.
    #[test]
    fn biomes_are_interned_by_id() {
        let mut reg = Registry::vanilla();
        let n = reg.biomes.len();
        let plains = reg.biome_by_id("minecraft:plains").unwrap();
        assert_eq!(reg.intern_biome(BiomeDef::of_id("minecraft:plains")), plains, "the game's kept");
        let b = reg.intern_biome(BiomeDef::of_id("regions_unexplored:alpha_grove"));
        assert_eq!((b.index(), reg.biomes.len()), (n, n + 1));
        assert_eq!(reg.intern_biome(BiomeDef::of_id("regions_unexplored:alpha_grove")), b);
        assert_eq!(reg.biome(b).name, "Alpha Grove");
        assert_eq!(biome_display_name("terralith:cave/frostfire_caves"), "Frostfire Caves");
        assert_eq!(biome_hash_color("a:b"), biome_hash_color("a:b"));
        assert_ne!(biome_hash_color("a:b"), biome_hash_color("a:c"));
        for id in ["regions_unexplored:bayou", "terralith:cave/deep_caves", "my-pack:glade.2"] {
            assert_eq!(check_biome_id(id), Ok(()), "{id}");
        }
        for id in ["bayou", "Mod:Bayou", ":bayou", "mod:", "mod:a b", "mod:/a", "a:b:c"] {
            assert!(check_biome_id(id).is_err(), "{id}");
        }
        // A biome of before groups and tints reads, and writes as it did.
        let old: BiomeDef = serde_json::from_str(r#"{"id":"minecraft:plains","name":"Plains","color":[1,2,3]}"#).unwrap();
        assert_eq!((old.group.as_str(), old.grass, old.foliage), ("", None, None));
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"id":"minecraft:plains","name":"Plains","color":[1,2,3]}"#);
    }

    use super::*;

    #[test]
    fn vanilla_registry_is_consistent() {
        let reg = Registry::vanilla();
        assert_eq!(reg.block(BlockId::AIR).state, "minecraft:air");
        assert_eq!(reg.terrain(TerrainId::ROCK).name, "Rock");
        assert_eq!(reg.biome(BiomeId::AUTO).id, "");
        assert_eq!(reg.block(reg.well_known.stone).name(), "minecraft:stone");
        assert_eq!(reg.block(reg.well_known.water).name(), "minecraft:water");
        for t in &reg.terrains {
            for s in &t.strata {
                assert!(!s.blocks.is_empty(), "{}", t.name);
                assert!(s.depth.0 >= 1 && s.depth.0 <= s.depth.1, "{}", t.name);
                for (b, w) in &s.blocks {
                    assert!(b.index() < reg.blocks.len() && *w > 0, "{}", t.name);
                }
            }
            if let Some(b) = t.biome_hint {
                assert!(b.index() < reg.biomes.len());
            }
        }
        // Names are unique so that lookups by name are unambiguous.
        let mut names: Vec<_> = reg.terrains.iter().map(|t| t.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), reg.terrains.len());
    }

    #[test]
    fn covers_grow_their_plants_at_their_density() {
        let reg = Registry::vanilla();
        assert!(reg.cover(0).is_none());
        let n = reg.cover_by_name("Grass").unwrap();
        let grass = reg.cover(n).unwrap();
        assert_eq!(grass.name, "Grass");
        let mut grown = 0;
        let mut kinds = [0usize; 3];
        for x in 0..100 {
            for z in 0..100 {
                if let Some(g) = grass.growth(x, z, 7) {
                    grown += 1;
                    kinds[g.plant] += 1;
                    assert_eq!(g.height, 1);
                    // The same column always grows the same.
                    assert_eq!(grass.growth(x, z, 7), Some(g));
                }
            }
        }
        // Half of the columns, mostly short grass.
        assert!((4500..5500).contains(&grown), "{grown}");
        assert!(kinds[0] > kinds[1] * 3 && kinds[1] > kinds[2], "{kinds:?}");
        let snow = reg.cover(reg.cover_by_name("snow").unwrap()).unwrap();
        assert!((0..50).all(|x| snow.growth(x, 3, 1).is_some()));
        let mut gone = snow.clone();
        gone.removed = true;
        assert!(gone.growth(0, 0, 1).is_none());
        let mut cane = CoverDef::new("Cane", [0; 3], &[("minecraft:sugar_cane", 1)], 1.0);
        cane.height = [1, 3];
        let heights: std::collections::BTreeSet<u8> = (0..200).filter_map(|x| cane.growth(x, 0, 1)).map(|g| g.height).collect();
        assert_eq!(heights.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(is_tall_plant("minecraft:tall_grass") && is_tall_plant("minecraft:peony[half=lower]"));
        assert!(!is_tall_plant("minecraft:short_grass") && !is_tall_plant("minecraft:tall_dry_grass"));
        // A mod's tall plant, kept as its lower half.
        assert!(is_tall_plant("conquest:dead_bush_tall[half=lower]") && !is_tall_plant("conquest:dead_bush_tall") && !is_tall_plant("conquest:oak_stairs[half=bottom]"));
        // A mod's ground is soil by its name; what is made of it is not.
        for ground in ["conquest:grass_covered_limestone", "conquest:clover_covered_grass", "conquest:green_sphagnum_moss_block", "conquest:autumnal_forest_floor", "conquest:loamy_dirt_with_bones", "conquest:dark_fertile_soil", "conquest:red_earth", "conquest:light_silt", "conquest:thick_mud", "conquest:peat", "conquest:shredded_leaves"] {
            assert!(is_soil(ground), "{ground}");
        }
        for made in ["conquest:mud_brick", "conquest:dirt_path_slab", "conquest:mossy_cobblestone_wall", "conquest:mossy_cobblestone", "conquest:limestone", "conquest:old_red_bricks", "conquest:oak_parquet_floor", "conquest:earthenware_pot"] {
            assert!(!is_soil(made), "{made}");
        }
        assert!(is_soil("minecraft:grass_block[snowy=false]") && !is_soil("minecraft:stone") && !is_soil("minecraft:clay"));
    }

    #[test]
    fn worlds_from_before_covers_get_the_usual_ones() {
        let mut json = serde_json::to_value(Registry::vanilla()).unwrap();
        json.as_object_mut().unwrap().remove("covers");
        let reg: Registry = serde_json::from_value(json).unwrap();
        assert_eq!(reg.covers, vanilla_covers());
    }

    #[test]
    fn block_state_parsing() {
        let b = BlockDef {
            state: "minecraft:grass_block[snowy=false,foo=bar]".into(),
            color: [0; 3],
        };
        assert_eq!(b.name(), "minecraft:grass_block");
        let props: Vec<_> = b.properties().collect();
        assert_eq!(props, vec![("snowy", "false"), ("foo", "bar")]);
        let b = BlockDef {
            state: "minecraft:stone".into(),
            color: [0; 3],
        };
        assert_eq!(b.name(), "minecraft:stone");
        assert_eq!(b.properties().count(), 0);
    }
}
