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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BiomeDef {
    /// Namespaced biome id (`minecraft:plains`); empty for the automatic entry.
    pub id: String,
    pub name: String,
    pub color: [u8; 3],
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
    /// Removed: no longer offered, and nothing grows where it was painted.
    #[serde(default)]
    pub removed: bool,
}

fn one_high() -> [u8; 2] {
    [1, 1]
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
/// two blocks high.
pub fn is_tall_plant(state: &str) -> bool {
    let name = state.split('[').next().unwrap_or(state);
    TALL_PLANTS.contains(&name.strip_prefix("minecraft:").unwrap_or(name))
}

/// Soil that plants grow on (by block, with or without properties).
pub fn is_soil(state: &str) -> bool {
    let name = state.split('[').next().unwrap_or(state);
    matches!(
        name.strip_prefix("minecraft:").unwrap_or(name),
        "grass_block" | "dirt" | "coarse_dirt" | "podzol" | "rooted_dirt" | "moss_block" | "pale_moss_block" | "mud" | "muddy_mangrove_roots" | "mycelium" | "farmland" | "dirt_path"
    )
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
            plants: plants.iter().map(|(b, w)| ((*b).to_owned(), *w)).collect(),
            density,
            height: one_high(),
            soil_only: true,
            underwater: false,
            removed: false,
        }
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
        };
        let mix = |blocks: &[(BlockId, u16)], lo: u16, hi: u16| Stratum {
            blocks: blocks.to_vec(),
            depth: (lo, hi),
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
