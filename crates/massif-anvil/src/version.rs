//! The Minecraft versions a world can be exported for.
//!
//! The way chunks are stored changed four times that matter here:
//!
//! - up to 1.12, blocks are numbers (an id and four bits of data), biomes a
//!   byte per column, and the world is 256 blocks high;
//! - 1.14 and 1.15 name their blocks in a palette per section, the indices
//!   packed end to end; 1.15 makes the biomes three-dimensional;
//! - 1.16 and 1.17 stop the indices from straddling two longs;
//! - 1.18 moves everything to the chunk's root, gives sections a palette of
//!   biomes, and digs the world down to −64 and up to 320.
//!
//! A world written for a version only uses what that version knows: blocks
//! and biomes that came later are replaced by the closest older ones.
//! (1.13 is left out: it lights chunks in a way later versions dropped.)
//!
//! From 1.18 on, a chunk is written as the version itself writes it, its
//! data version its own, so that the game loads it without upgrading it:
//! the shape of chunks has not changed since (the keys of 26.3's
//! `SerializableChunkData` are 1.18's), only the names of blocks and
//! biomes have. Every block of every release from 1.18.2 to 26.3 is listed
//! with the release that brought it (`blocks.txt`, from the client jars and
//! the wiki), and so is every biome: a block or a biome the version does not
//! have becomes the closest it has, and blocks renamed since (grass, chain)
//! take the name the version knows.

/// How a version stores its chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Era {
    /// 1.7 to 1.12: numbered blocks.
    Numbered,
    /// 1.14: palettes, packed end to end, a biome per column.
    Flat14,
    /// 1.15: the same, biomes in 4×4×4 cells.
    Flat15,
    /// 1.16 and 1.17: indices no longer straddle longs.
    Flat16,
    /// 1.18 and later.
    Modern,
}

/// A version of the game to export for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Its name, as the launcher shows it (`1.21.4`, `26.1`).
    pub name: String,
    /// The number it stamps on what it saves; 0 before 1.9, which had none.
    pub data_version: i32,
}

/// Releases and their data versions, newest first. Versions newer than
/// these are read from the game itself when it is installed.
pub const RELEASES: &[(&str, i32)] = &[
    ("26.3", 5023),
    ("26.2", 4903),
    ("26.1.2", 4790),
    ("26.1.1", 4788),
    ("26.1", 4786),
    ("1.21.11", 4671),
    ("1.21.10", 4556),
    ("1.21.9", 4554),
    ("1.21.8", 4440),
    ("1.21.7", 4438),
    ("1.21.6", 4435),
    ("1.21.5", 4325),
    ("1.21.4", 4189),
    ("1.21.3", 4082),
    ("1.21.2", 4080),
    ("1.21.1", 3955),
    ("1.21", 3953),
    ("1.20.6", 3839),
    ("1.20.5", 3837),
    ("1.20.4", 3700),
    ("1.20.3", 3698),
    ("1.20.2", 3578),
    ("1.20.1", 3465),
    ("1.20", 3463),
    ("1.19.4", 3337),
    ("1.19.3", 3218),
    ("1.19.2", 3120),
    ("1.19.1", 3117),
    ("1.19", 3105),
    ("1.18.2", 2975),
    ("1.18.1", 2865),
    ("1.18", 2860),
    ("1.17.1", 2730),
    ("1.17", 2724),
    ("1.16.5", 2586),
    ("1.16.4", 2584),
    ("1.16.3", 2580),
    ("1.16.2", 2578),
    ("1.16.1", 2567),
    ("1.16", 2566),
    ("1.15.2", 2230),
    ("1.15.1", 2227),
    ("1.15", 2225),
    ("1.14.4", 1976),
    ("1.14.3", 1968),
    ("1.14.2", 1963),
    ("1.14.1", 1957),
    ("1.14", 1952),
    ("1.12.2", 1343),
    ("1.12.1", 1241),
    ("1.12", 1139),
    ("1.11.2", 922),
    ("1.10.2", 512),
    ("1.9.4", 184),
    ("1.8.9", 0),
    ("1.7.10", 0),
];

impl Default for Target {
    /// 1.18.2: the oldest release with today's chunks, which every later
    /// one opens and upgrades by itself.
    fn default() -> Self {
        Self::release("1.18.2").expect("1.18.2 is a known release")
    }
}

impl Target {
    /// A known release, by name.
    pub fn release(name: &str) -> Option<Self> {
        RELEASES.iter().find(|r| r.0 == name).map(|&(name, data_version)| Self {
            name: name.to_owned(),
            data_version,
        })
    }

    /// Every known release, newest first.
    pub fn releases() -> Vec<Self> {
        RELEASES
            .iter()
            .map(|&(name, data_version)| Self {
                name: name.to_owned(),
                data_version,
            })
            .collect()
    }

    /// A version by its name and the data version the game itself gives
    /// (from its `version.json`): for releases newer than the list.
    pub fn installed(name: &str, data_version: i32) -> Self {
        Self {
            name: name.to_owned(),
            data_version,
        }
    }

    pub fn era(&self) -> Era {
        match self.data_version {
            ..1444 => Era::Numbered,
            1444..2200 => Era::Flat14,
            2200..2500 => Era::Flat15,
            2500..2825 => Era::Flat16,
            _ => Era::Modern,
        }
    }

    /// Whether the version is 1.7: before granite, red sandstone and the
    /// named flat-world presets.
    fn is_1_7(&self) -> bool {
        self.data_version == 0 && self.name.starts_with("1.7")
    }

    /// The heights a world of this version holds, given the map's own.
    pub fn heights(&self, min_y: i32, max_y: i32) -> (i32, i32) {
        match self.era() {
            Era::Modern => (min_y, max_y),
            _ => (0, 256),
        }
    }

    /// The data version and name `level.dat` is written with: the
    /// version's own, as everything else.
    pub fn level_version(&self) -> (i32, &str) {
        (self.data_version, &self.name)
    }

    /// Whether the world is laid out as 26.1 lays it (the game's
    /// `DimensionStorageFileFix` and `LevelDatToSavedDataFileFix`, data
    /// version 4772): each dimension's regions and data under
    /// `dimensions/minecraft/<dimension>/`, and what `level.dat` held of the
    /// world's generation, weather, time and border in files of their own.
    pub fn splits_level(&self) -> bool {
        self.data_version >= 4772
    }

    /// The folder of the overworld's region files, in the world's folder.
    pub fn region_dir(&self) -> &'static str {
        if self.splits_level() { "dimensions/minecraft/overworld/region" } else { "region" }
    }

    /// The keys of a block state's name and properties: `Name` and
    /// `Properties`, `id` and `properties` from 26.3 (the game's
    /// `BlockStateFieldNamesFix`, data version 5006).
    pub fn state_keys(&self) -> (&'static str, &'static str) {
        if self.data_version >= 5006 { ("id", "properties") } else { ("Name", "Properties") }
    }

    /// The format number of the version's data packs (from 1.18 on).
    pub fn pack_format(&self) -> i32 {
        // From the data version on: the format.
        const FORMATS: &[(i32, i32)] = &[
            (5023, 121),
            (4786, 101),
            (4671, 94),
            (4554, 88),
            (4438, 81),
            (4435, 80),
            (4325, 71),
            (4189, 61),
            (4080, 57),
            (3953, 48),
            (3837, 41),
            (3698, 26),
            (3578, 18),
            (3463, 15),
            (3337, 12),
            (3105, 10),
            (2975, 9),
            (2860, 8),
        ];
        FORMATS.iter().find(|(since, _)| self.data_version >= *since).map_or(8, |f| f.1)
    }

    /// The status of a finished chunk, as the version writes it.
    pub fn full_status(&self) -> &'static str {
        if self.data_version >= 3450 { "minecraft:full" } else { "full" }
    }

    /// A word on what the export loses, for the dialog; empty if nothing.
    pub fn caveat(&self, min_y: i32, max_y: i32) -> String {
        let mut notes = Vec::new();
        if self.era() != Era::Modern && (min_y < 0 || max_y > 256) {
            notes.push("worlds are 256 blocks high: the map is cut below 0 and above 255".to_owned());
        }
        match self.era() {
            Era::Numbered => notes.push("blocks are numbered: those the version does not have become the closest it has, or stone".to_owned()),
            Era::Modern => {}
            _ => notes.push("blocks and biomes added later are replaced by the closest older ones".to_owned()),
        }
        notes.join("; ")
    }

    /// The name a block goes by in this version: itself, or the closest
    /// block the version has. The second value is whether its properties
    /// still apply.
    pub fn block<'a>(&self, name: &'a str) -> (&'a str, bool) {
        self.block_or(name, "minecraft:air")
    }

    /// [`Self::block`], with `unknown` for a block the version has nothing
    /// like (from 1.18 on, where every block of every release is known).
    pub fn block_or<'a>(&self, name: &'a str, unknown: &'a str) -> (&'a str, bool) {
        let mut current = name;
        let mut kept = true;
        // A block renamed since goes by its new name in the versions after.
        if let Some(short) = name.strip_prefix("minecraft:")
            && let Some(&(_, new, since)) = RENAMED_BLOCKS.iter().find(|r| r.0 == short)
            && self.data_version >= since
        {
            current = new;
        }
        // Follow the replacements until one exists in this version.
        for _ in 0..6 {
            let short = current.strip_prefix("minecraft:").unwrap_or(current);
            match NEWER_BLOCKS.iter().find(|b| b.0 == short) {
                Some(&(_, since, older)) if self.data_version < since => {
                    // Wood for wood and leaves for leaves keep their axis
                    // and their persistence, and a block its properties
                    // under its old name.
                    let renamed = RENAMED_BLOCKS.iter().any(|r| older.strip_prefix("minecraft:") == Some(r.0) && r.1.strip_prefix("minecraft:") == Some(short));
                    let same = renamed || ["_log", "_leaves"].iter().any(|k| short.ends_with(k) && older.ends_with(k));
                    current = older;
                    kept &= same;
                }
                _ => break,
            }
        }
        if !self.lists_blocks() {
            return (current, kept);
        }
        let Some(short) = current.strip_prefix("minecraft:") else {
            // Blocks of mods are the mod's business.
            return (current, kept);
        };
        if self.has_block(short) {
            return (current, kept);
        }
        // A wood the version lacks: the same thing of the closest wood.
        for (wood, older) in OLDER_WOODS {
            if short.contains(wood) {
                let alike = short.replacen(wood, older, 1);
                if let Some((&name, _)) = blocks().get_key_value(alike.as_str())
                    && self.has_block(name)
                {
                    return (qualified(name), kept);
                }
            }
        }
        (unknown, false)
    }

    /// Whether every block and biome of the version is known: from 1.18 to
    /// the newest release listed.
    fn lists_blocks(&self) -> bool {
        self.era() == Era::Modern && self.data_version <= NEWEST_LISTED
    }

    /// Whether the version has a block, by its id without `minecraft:`
    /// (from 1.18 on).
    pub fn has_block(&self, short: &str) -> bool {
        if let Some(r) = RENAMED_BLOCKS.iter().find(|r| r.0 == short) {
            return self.data_version < r.2;
        }
        blocks().get(short).is_some_and(|&since| since <= self.data_version)
    }

    /// The biome a version writes for `id`: itself, or from 1.18 on the
    /// closest biome the version has (plains for one it has nothing like).
    pub fn biome<'a>(&self, id: &'a str) -> &'a str {
        if !self.lists_blocks() {
            return id;
        }
        let mut current = id;
        for _ in 0..4 {
            let Some(short) = current.strip_prefix("minecraft:") else {
                return current;
            };
            if BIOMES_1_18_2.contains(&short) {
                return current;
            }
            match NEWER_BIOMES.iter().find(|b| b.0 == short) {
                Some(&(_, since, _)) if self.data_version >= since => return current,
                Some(&(_, _, older)) => current = older,
                None => break,
            }
        }
        "minecraft:plains"
    }

    /// The number and data of a block before 1.13: those of the closest
    /// block the version has, or of stone.
    pub fn numbered(&self, name: &str) -> (u8, u8) {
        self.numbered_known(name).unwrap_or((1, 0))
    }

    /// [`Self::numbered`], `None` where the version has nothing close.
    pub fn numbered_known(&self, name: &str) -> Option<(u8, u8)> {
        let (name, _) = self.block(name);
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        let found = NUMBERED_BLOCKS.iter().find(|b| b.0 == short).map(|b| (b.1, b.2));
        let colour = |suffix: &str| {
            let colour = short.strip_suffix(suffix)?;
            COLOURS.iter().position(|c| *c == colour).map(|k| k as u8)
        };
        let (id, data) = found
            .or_else(|| colour("_terracotta").map(|c| (159, c)))
            .or_else(|| colour("_carpet").map(|c| (171, c)))
            .or_else(|| colour("_wool").map(|c| (35, c)))
            .or_else(|| colour("_concrete").map(|c| if self.data_version >= 1139 { (251, c) } else { (159, c) }))
            .or_else(|| colour("_concrete_powder").map(|c| if self.data_version >= 1139 { (252, c) } else { (12, 0) }))?;
        Some(match id {
            // 1.7 has neither red sandstone nor the kinds of stone.
            179 if self.is_1_7() => (24, 0),
            1 if self.is_1_7() => (1, 0),
            // Magma and bone came with 1.10.
            213 | 216 if self.data_version < 510 => (87, 0),
            _ => (id, data),
        })
    }

    /// The number of a biome before 1.18.
    pub fn biome_number(&self, id: &str) -> i32 {
        let short = id.strip_prefix("minecraft:").unwrap_or(id);
        let n = BIOME_NUMBERS.iter().find(|b| b.0 == short).map_or(1, |b| b.1);
        match n {
            // The oceans of 1.13, bamboo of 1.14, the void of 1.9.
            44..=46 if self.data_version < 1519 => 0,
            168 if self.data_version < 1952 => 21,
            127 if self.data_version < 169 => 1,
            _ => n,
        }
    }

    /// The preset of a flat world of air, before 1.13.
    pub fn void_preset(&self) -> &'static str {
        if self.is_1_7() {
            "2;0;1;"
        } else if self.data_version < 169 {
            "3;minecraft:air;1;"
        } else {
            "3;minecraft:air;127;decoration"
        }
    }
}

/// The newest release whose blocks and biomes are all listed.
const NEWEST_LISTED: i32 = 5023;

/// Every block from 1.18.2 on, by its id without `minecraft:`, with the
/// data version that brought it (0 for those of 1.18.2).
fn blocks() -> &'static std::collections::HashMap<&'static str, i32> {
    static BLOCKS: std::sync::OnceLock<std::collections::HashMap<&'static str, i32>> = std::sync::OnceLock::new();
    BLOCKS.get_or_init(|| {
        include_str!("blocks.txt")
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| match l.split_once(' ') {
                Some((name, since)) => (name, since.parse().expect("a data version")),
                None => (l, 0),
            })
            .collect()
    })
}

/// `minecraft:` and a block of the list.
fn qualified(short: &'static str) -> &'static str {
    static NAMES: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| blocks().keys().map(|&k| (k, &*format!("minecraft:{k}").leak())).collect())[short]
}

/// Blocks renamed since 1.18.2: the old name, the new one and the data
/// version of the release that renamed them (from the game's own data
/// fixers: "Rename grass block to short_grass", "Rename chain to
/// iron_chain").
const RENAMED_BLOCKS: &[(&str, &str, i32)] = &[("grass", "minecraft:short_grass", 3698), ("chain", "minecraft:iron_chain", 4554)];

/// Woods a version may not have yet, and the closest older wood: its logs,
/// planks, stairs and the rest stand for theirs.
const OLDER_WOODS: [(&str, &str); 5] = [("pale_oak_", "dark_oak_"), ("cherry_", "birch_"), ("mangrove_", "oak_"), ("bamboo_", "oak_"), ("poplar_", "birch_")];

/// The biomes of 1.18.2.
const BIOMES_1_18_2: [&str; 61] = [
    "badlands", "bamboo_jungle", "basalt_deltas", "beach", "birch_forest", "cold_ocean", "crimson_forest", "dark_forest", "deep_cold_ocean", "deep_frozen_ocean", "deep_lukewarm_ocean", "deep_ocean", "desert", "dripstone_caves", "end_barrens", "end_highlands", "end_midlands", "eroded_badlands", "flower_forest", "forest", "frozen_ocean", "frozen_peaks", "frozen_river", "grove", "ice_spikes", "jagged_peaks", "jungle", "lukewarm_ocean", "lush_caves", "meadow", "mushroom_fields", "nether_wastes", "ocean", "old_growth_birch_forest", "old_growth_pine_taiga", "old_growth_spruce_taiga", "plains", "river", "savanna", "savanna_plateau", "small_end_islands", "snowy_beach", "snowy_plains", "snowy_slopes", "snowy_taiga", "soul_sand_valley", "sparse_jungle", "stony_peaks", "stony_shore", "sunflower_plains", "swamp", "taiga", "the_end", "the_void", "warm_ocean", "warped_forest", "windswept_forest", "windswept_gravelly_hills", "windswept_hills", "windswept_savanna", "wooded_badlands",
];

/// Biomes that came after 1.18.2 (from the client jars' worldgen data and
/// the wiki), the data version that brought them and the closest older
/// biome.
const NEWER_BIOMES: &[(&str, i32, &str)] = &[
    ("mangrove_swamp", 3105, "minecraft:swamp"),
    ("deep_dark", 3105, "minecraft:dripstone_caves"),
    ("cherry_grove", 3463, "minecraft:meadow"),
    ("pale_garden", 4189, "minecraft:dark_forest"),
    ("sulfur_caves", 4903, "minecraft:dripstone_caves"),
    ("dappled_forest", 5023, "minecraft:forest"),
];

/// Blocks that came after 1.14, the data version that brought them, and
/// the older block closest to them. (And the plants that came with 1.13
/// and 1.14, for the versions that number their blocks.)
const NEWER_BLOCKS: &[(&str, i32, &str)] = &[
    // 1.13
    ("seagrass", 1444, "minecraft:air"),
    ("tall_seagrass", 1444, "minecraft:air"),
    ("kelp", 1444, "minecraft:air"),
    ("kelp_plant", 1444, "minecraft:air"),
    ("sea_pickle", 1444, "minecraft:air"),
    // 1.14
    ("cornflower", 1952, "minecraft:blue_orchid"),
    ("lily_of_the_valley", 1952, "minecraft:oxeye_daisy"),
    ("wither_rose", 1952, "minecraft:poppy"),
    ("sweet_berry_bush", 1952, "minecraft:fern"),
    ("bamboo", 1952, "minecraft:sugar_cane"),
    // 1.16
    ("basalt", 2566, "minecraft:stone"),
    ("polished_basalt", 2566, "minecraft:stone"),
    ("blackstone", 2566, "minecraft:cobblestone"),
    ("soul_soil", 2566, "minecraft:soul_sand"),
    ("crying_obsidian", 2566, "minecraft:obsidian"),
    ("netherite_block", 2566, "minecraft:obsidian"),
    ("crimson_nylium", 2566, "minecraft:netherrack"),
    ("crimson_roots", 2566, "minecraft:dead_bush"),
    ("warped_roots", 2566, "minecraft:dead_bush"),
    ("nether_sprouts", 2566, "minecraft:air"),
    ("crimson_fungus", 2566, "minecraft:red_mushroom"),
    ("warped_fungus", 2566, "minecraft:brown_mushroom"),
    ("warped_nylium", 2566, "minecraft:netherrack"),
    // 1.17
    ("deepslate", 2724, "minecraft:stone"),
    ("cobbled_deepslate", 2724, "minecraft:cobblestone"),
    ("tuff", 2724, "minecraft:andesite"),
    ("calcite", 2724, "minecraft:diorite"),
    ("dripstone_block", 2724, "minecraft:granite"),
    ("smooth_basalt", 2724, "minecraft:basalt"),
    ("moss_block", 2724, "minecraft:grass_block"),
    ("rooted_dirt", 2724, "minecraft:dirt"),
    ("azalea_leaves", 2724, "minecraft:oak_leaves"),
    ("flowering_azalea_leaves", 2724, "minecraft:oak_leaves"),
    ("powder_snow", 2724, "minecraft:snow_block"),
    ("moss_carpet", 2724, "minecraft:green_carpet"),
    ("azalea", 2724, "minecraft:oak_leaves"),
    ("flowering_azalea", 2724, "minecraft:oak_leaves"),
    ("small_dripleaf", 2724, "minecraft:fern"),
    ("glow_lichen", 2724, "minecraft:air"),
    ("hanging_roots", 2724, "minecraft:air"),
    ("amethyst_block", 2724, "minecraft:purple_concrete"),
    ("raw_iron_block", 2724, "minecraft:iron_ore"),
    ("raw_copper_block", 2724, "minecraft:terracotta"),
    ("raw_gold_block", 2724, "minecraft:gold_ore"),
    ("copper_ore", 2724, "minecraft:iron_ore"),
    ("deepslate_iron_ore", 2724, "minecraft:iron_ore"),
    ("deepslate_coal_ore", 2724, "minecraft:coal_ore"),
    ("deepslate_gold_ore", 2724, "minecraft:gold_ore"),
    ("deepslate_diamond_ore", 2724, "minecraft:diamond_ore"),
    ("deepslate_copper_ore", 2724, "minecraft:iron_ore"),
    ("deepslate_redstone_ore", 2724, "minecraft:redstone_ore"),
    ("deepslate_lapis_ore", 2724, "minecraft:lapis_ore"),
    ("deepslate_emerald_ore", 2724, "minecraft:emerald_ore"),
    // 1.19
    ("mud", 3105, "minecraft:coarse_dirt"),
    ("packed_mud", 3105, "minecraft:coarse_dirt"),
    ("muddy_mangrove_roots", 3105, "minecraft:coarse_dirt"),
    ("sculk", 3105, "minecraft:black_concrete"),
    ("reinforced_deepslate", 3105, "minecraft:deepslate"),
    ("mangrove_propagule", 3105, "minecraft:oak_sapling"),
    ("mangrove_log", 3105, "minecraft:oak_log"),
    ("mangrove_leaves", 3105, "minecraft:oak_leaves"),
    // 1.20 (1.19.4 had them only as an experiment)
    ("pink_petals", 3463, "minecraft:pink_tulip"),
    ("cherry_log", 3463, "minecraft:birch_log"),
    ("cherry_leaves", 3463, "minecraft:birch_leaves"),
    ("torchflower", 3463, "minecraft:dandelion"),
    ("pitcher_plant", 3463, "minecraft:rose_bush"),
    // 1.20.3: grass the plant took a new name.
    ("short_grass", 3698, "minecraft:grass"),
    // 1.21.4
    ("pale_moss_block", 4189, "minecraft:moss_block"),
    ("pale_oak_log", 4189, "minecraft:dark_oak_log"),
    ("pale_oak_leaves", 4189, "minecraft:dark_oak_leaves"),
    ("resin_block", 4189, "minecraft:orange_terracotta"),
    ("pale_moss_carpet", 4189, "minecraft:moss_carpet"),
    ("pale_hanging_moss", 4189, "minecraft:air"),
    ("open_eyeblossom", 4189, "minecraft:oxeye_daisy"),
    ("closed_eyeblossom", 4189, "minecraft:oxeye_daisy"),
    // 1.21.5
    ("bush", 4325, "minecraft:short_grass"),
    ("firefly_bush", 4325, "minecraft:dead_bush"),
    ("short_dry_grass", 4325, "minecraft:dead_bush"),
    ("tall_dry_grass", 4325, "minecraft:dead_bush"),
    ("leaf_litter", 4325, "minecraft:air"),
    ("wildflowers", 4325, "minecraft:pink_petals"),
    ("cactus_flower", 4325, "minecraft:air"),
    // 1.21.9: chain took a new name.
    ("iron_chain", 4554, "minecraft:chain"),
    // 26.1, 26.3
    ("golden_dandelion", 4786, "minecraft:dandelion"),
    ("orange_poplar_leaves", 5023, "minecraft:birch_leaves"),
    ("red_poplar_leaves", 5023, "minecraft:birch_leaves"),
    ("yellow_poplar_leaves", 5023, "minecraft:birch_leaves"),
    ("red_shrub", 5023, "minecraft:dead_bush"),
];

/// The sixteen colours, in the order of their numbers before 1.13.
const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// Blocks by their number and data before 1.13.
const NUMBERED_BLOCKS: &[(&str, u8, u8)] = &[
    ("air", 0, 0),
    ("cave_air", 0, 0),
    ("stone", 1, 0),
    ("granite", 1, 1),
    ("polished_granite", 1, 2),
    ("diorite", 1, 3),
    ("polished_diorite", 1, 4),
    ("andesite", 1, 5),
    ("polished_andesite", 1, 6),
    ("grass_block", 2, 0),
    ("dirt", 3, 0),
    ("coarse_dirt", 3, 1),
    ("podzol", 3, 2),
    ("cobblestone", 4, 0),
    ("oak_planks", 5, 0),
    ("spruce_planks", 5, 1),
    ("birch_planks", 5, 2),
    ("jungle_planks", 5, 3),
    ("acacia_planks", 5, 4),
    ("dark_oak_planks", 5, 5),
    ("bedrock", 7, 0),
    ("water", 9, 0),
    ("lava", 11, 0),
    ("sand", 12, 0),
    ("red_sand", 12, 1),
    ("gravel", 13, 0),
    ("oak_sapling", 6, 0),
    ("gold_ore", 14, 0),
    ("iron_ore", 15, 0),
    ("coal_ore", 16, 0),
    ("oak_log", 17, 0),
    ("spruce_log", 17, 1),
    ("birch_log", 17, 2),
    ("jungle_log", 17, 3),
    ("oak_wood", 17, 12),
    ("spruce_wood", 17, 13),
    ("birch_wood", 17, 14),
    ("jungle_wood", 17, 15),
    ("oak_leaves", 18, 0),
    ("spruce_leaves", 18, 1),
    ("birch_leaves", 18, 2),
    ("jungle_leaves", 18, 3),
    ("sponge", 19, 0),
    ("glass", 20, 0),
    ("lapis_ore", 21, 0),
    ("sandstone", 24, 0),
    ("chiseled_sandstone", 24, 1),
    ("cut_sandstone", 24, 2),
    ("smooth_sandstone", 24, 2),
    // Grass the plant, so named from 1.13 to 1.20.2.
    ("grass", 31, 1),
    ("fern", 31, 2),
    ("dead_bush", 32, 0),
    ("dandelion", 37, 0),
    ("poppy", 38, 0),
    ("blue_orchid", 38, 1),
    ("allium", 38, 2),
    ("azure_bluet", 38, 3),
    ("red_tulip", 38, 4),
    ("orange_tulip", 38, 5),
    ("white_tulip", 38, 6),
    ("pink_tulip", 38, 7),
    ("oxeye_daisy", 38, 8),
    ("brown_mushroom", 39, 0),
    ("red_mushroom", 40, 0),
    ("gold_block", 41, 0),
    ("iron_block", 42, 0),
    ("bricks", 45, 0),
    ("mossy_cobblestone", 48, 0),
    ("obsidian", 49, 0),
    ("diamond_ore", 56, 0),
    ("redstone_ore", 73, 0),
    ("snow", 78, 0),
    ("ice", 79, 0),
    ("snow_block", 80, 0),
    ("cactus", 81, 0),
    ("clay", 82, 0),
    ("sugar_cane", 83, 0),
    ("netherrack", 87, 0),
    ("soul_sand", 88, 0),
    ("glowstone", 89, 0),
    ("stone_bricks", 98, 0),
    ("mossy_stone_bricks", 98, 1),
    ("cracked_stone_bricks", 98, 2),
    ("chiseled_stone_bricks", 98, 3),
    ("pumpkin", 86, 0),
    ("mushroom_stem", 99, 10),
    ("brown_mushroom_block", 99, 14),
    ("red_mushroom_block", 100, 14),
    ("melon", 103, 0),
    ("vine", 106, 0),
    ("mycelium", 110, 0),
    ("lily_pad", 111, 0),
    ("cocoa", 127, 0),
    ("nether_bricks", 112, 0),
    ("end_stone", 121, 0),
    ("emerald_ore", 129, 0),
    ("quartz_block", 155, 0),
    ("prismarine", 168, 0),
    ("prismarine_bricks", 168, 1),
    ("dark_prismarine", 168, 2),
    ("sea_lantern", 169, 0),
    ("hay_block", 170, 0),
    ("terracotta", 172, 0),
    ("coal_block", 173, 0),
    ("packed_ice", 174, 0),
    ("blue_ice", 174, 0),
    ("acacia_leaves", 161, 0),
    ("dark_oak_leaves", 161, 1),
    ("acacia_log", 162, 0),
    ("dark_oak_log", 162, 1),
    ("acacia_wood", 162, 12),
    ("dark_oak_wood", 162, 13),
    ("sunflower", 175, 0),
    ("lilac", 175, 1),
    ("tall_grass", 175, 2),
    ("large_fern", 175, 3),
    ("rose_bush", 175, 4),
    ("peony", 175, 5),
    ("red_sandstone", 179, 0),
    ("chiseled_red_sandstone", 179, 1),
    ("cut_red_sandstone", 179, 2),
    ("smooth_red_sandstone", 179, 2),
    ("purpur_block", 201, 0),
    ("end_stone_bricks", 206, 0),
    ("magma_block", 213, 0),
    ("nether_wart_block", 214, 0),
    ("red_nether_bricks", 215, 0),
    ("bone_block", 216, 0),
];

/// The biome numbered `n` before 1.18, by its name of today (without
/// `minecraft:`): the first listed of that number.
pub fn biome_of_number(n: i32) -> Option<&'static str> {
    BIOME_NUMBERS.iter().find(|b| b.1 == n).map(|b| b.0)
}

/// Biomes by their number before 1.18; later ones take the number of the
/// closest biome of old.
const BIOME_NUMBERS: &[(&str, i32)] = &[
    ("ocean", 0),
    ("plains", 1),
    ("desert", 2),
    ("windswept_hills", 3),
    ("stony_peaks", 3),
    ("forest", 4),
    ("taiga", 5),
    ("swamp", 6),
    ("mangrove_swamp", 6),
    ("river", 7),
    ("frozen_ocean", 10),
    ("frozen_river", 11),
    ("snowy_plains", 12),
    ("snowy_slopes", 13),
    ("frozen_peaks", 13),
    ("jagged_peaks", 13),
    ("mushroom_fields", 14),
    ("beach", 16),
    ("jungle", 21),
    ("deep_ocean", 24),
    ("stony_shore", 25),
    ("snowy_beach", 26),
    ("birch_forest", 27),
    ("dark_forest", 29),
    ("pale_garden", 29),
    ("snowy_taiga", 30),
    ("grove", 30),
    ("old_growth_pine_taiga", 32),
    ("savanna", 35),
    ("badlands", 37),
    ("warm_ocean", 44),
    ("lukewarm_ocean", 45),
    ("cold_ocean", 46),
    ("the_void", 127),
    ("sunflower_plains", 129),
    ("flower_forest", 132),
    ("cherry_grove", 132),
    ("meadow", 1),
    ("bamboo_jungle", 168),
    ("dripstone_caves", 1),
    ("lush_caves", 1),
    ("deep_dark", 1),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_know_their_format() {
        let era = |name: &str| Target::release(name).unwrap().era();
        assert_eq!(era("1.7.10"), Era::Numbered);
        assert_eq!(era("1.12.2"), Era::Numbered);
        assert_eq!(era("1.14.4"), Era::Flat14);
        assert_eq!(era("1.15.2"), Era::Flat15);
        assert_eq!(era("1.16.5"), Era::Flat16);
        assert_eq!(era("1.17.1"), Era::Flat16);
        assert_eq!(era("1.18"), Era::Modern);
        assert_eq!(era("1.21.11"), Era::Modern);
        assert_eq!(Target::installed("27.1", 6000).era(), Era::Modern);
        assert_eq!(era("26.3"), Era::Modern);
        // level.dat is the version's own too.
        assert_eq!(Target::release("26.3").unwrap().level_version(), (5023, "26.3"));
        assert_eq!(Target::release("26.3").unwrap().region_dir(), "dimensions/minecraft/overworld/region");
        assert_eq!(Target::release("26.1").unwrap().region_dir(), "dimensions/minecraft/overworld/region");
        assert_eq!(Target::release("1.21.11").unwrap().region_dir(), "region");
        assert_eq!(Target::release("26.2").unwrap().state_keys(), ("Name", "Properties"));
        assert_eq!(Target::release("26.3").unwrap().state_keys(), ("id", "properties"));
        assert_eq!(Target::release("1.18.1").unwrap().level_version(), (2865, "1.18.1"));
        assert_eq!(Target::release("1.16.5").unwrap().level_version(), (2586, "1.16.5"));
        assert_eq!(Target::default().name, "1.18.2");
        // Newest first, every name once.
        let all = Target::releases();
        assert!(all.windows(2).all(|w| w[0].data_version >= w[1].data_version));
        assert_eq!(Target::release("1.19.4").unwrap().full_status(), "full");
        assert_eq!(Target::release("1.20.1").unwrap().full_status(), "minecraft:full");
        assert_eq!(Target::release("1.16.5").unwrap().heights(-64, 320), (0, 256));
        assert_eq!(Target::release("1.20.1").unwrap().heights(-64, 320), (-64, 320));
    }

    #[test]
    fn later_blocks_fall_back_to_older_ones() {
        let v = |name: &str| Target::release(name).unwrap();
        assert_eq!(v("1.21.4").block("minecraft:deepslate"), ("minecraft:deepslate", true));
        // The tree pack's woods: wood for wood, leaves for leaves, their
        // axis and persistence kept.
        assert_eq!(v("1.19.2").block("minecraft:cherry_leaves"), ("minecraft:birch_leaves", true));
        assert_eq!(v("1.16.5").block("minecraft:mangrove_log"), ("minecraft:oak_log", true));
        assert_eq!(v("1.21.1").block("minecraft:pale_oak_leaves"), ("minecraft:dark_oak_leaves", true));
        assert_eq!(v("1.16.5").block("minecraft:flowering_azalea_leaves"), ("minecraft:oak_leaves", true));
        assert_eq!(v("1.16.5").block("minecraft:azalea"), ("minecraft:oak_leaves", false));
        assert_eq!(v("1.16.5").block("minecraft:deepslate"), ("minecraft:stone", false));
        assert_eq!(v("1.16.5").block("minecraft:grass_block"), ("minecraft:grass_block", true));
        // Smooth basalt is basalt in 1.16, stone before.
        assert_eq!(v("1.16.5").block("minecraft:smooth_basalt").0, "minecraft:basalt");
        assert_eq!(v("1.15.2").block("minecraft:smooth_basalt").0, "minecraft:stone");
        assert_eq!(v("1.18.2").block("minecraft:mud").0, "minecraft:coarse_dirt");
        assert_eq!(v("1.19").block("minecraft:mud").0, "minecraft:mud");
        // Numbers, before 1.13.
        assert_eq!(v("1.12.2").numbered("minecraft:grass_block"), (2, 0));
        // Plants: grass the plant by both its names, flowers of 1.14 as the
        // closest of old, tall plants by their kind.
        assert_eq!(v("1.12.2").numbered("minecraft:short_grass"), (31, 1));
        assert_eq!(v("1.12.2").numbered("minecraft:cornflower"), (38, 1));
        assert_eq!(v("1.12.2").numbered("minecraft:large_fern"), (175, 3));
        assert_eq!(v("1.12.2").numbered("minecraft:moss_carpet"), (171, 13));
        assert_eq!(v("1.12.2").numbered_known("minecraft:seagrass"), Some((0, 0)));
        assert_eq!(v("1.12.2").numbered_known("minecraft:spore_blossom"), None);
        assert_eq!(v("1.20.2").block("minecraft:short_grass"), ("minecraft:grass", true));
        assert_eq!(v("1.20.4").block("minecraft:short_grass"), ("minecraft:short_grass", true));
        assert_eq!(v("1.21.4").block("minecraft:bush"), ("minecraft:short_grass", false));
        assert_eq!(v("1.19.2").block("minecraft:bush"), ("minecraft:grass", false));
        // A mod's blocks are written by their own names; before 1.13,
        // where blocks are numbered, they are stone.
        assert_eq!(v("1.21.4").block("conquest:old_red_bricks"), ("conquest:old_red_bricks", true));
        assert_eq!(v("1.12.2").numbered_known("conquest:old_red_bricks"), None);
        assert_eq!(v("1.12.2").numbered("conquest:old_red_bricks"), (1, 0));
        assert_eq!(v("1.12.2").numbered("minecraft:podzol"), (3, 2));
        assert_eq!(v("1.12.2").numbered("minecraft:deepslate"), (1, 0));
        assert_eq!(v("1.12.2").numbered("minecraft:moss_block"), (2, 0));
        assert_eq!(v("1.12.2").numbered("minecraft:orange_terracotta"), (159, 1));
        assert_eq!(v("1.12.2").numbered("minecraft:red_concrete"), (251, 14));
        assert_eq!(v("1.10.2").numbered("minecraft:red_concrete"), (159, 14));
        assert_eq!(v("1.12.2").numbered("minecraft:red_sandstone"), (179, 0));
        assert_eq!(v("1.7.10").numbered("minecraft:red_sandstone"), (24, 0));
        assert_eq!(v("1.12.2").numbered("minecraft:some_block_of_the_future"), (1, 0));
    }

    #[test]
    fn modern_versions_know_their_blocks_and_biomes() {
        let v = |name: &str| Target::release(name).unwrap();
        assert!(!v("1.18.2").has_block("mud") && v("1.19").has_block("mud"));
        assert!(v("1.21.8").has_block("chain") && !v("1.21.8").has_block("iron_chain"));
        assert!(!v("1.21.9").has_block("chain") && v("1.21.9").has_block("iron_chain"));
        assert!(!v("1.20.2").has_block("short_grass") && v("1.20.2").has_block("grass") && !v("1.20.3").has_block("grass"));
        assert!(!v("1.20.6").has_block("crafter") && v("1.21").has_block("crafter"));
        assert!(!v("26.1.2").has_block("sulfur") && v("26.2").has_block("sulfur") && !v("26.2").has_block("poplar_log"));
        // Renamed blocks go by the version's name, their properties kept.
        assert_eq!(v("1.21.11").block("minecraft:grass"), ("minecraft:short_grass", true));
        assert_eq!(v("1.21.11").block("minecraft:chain"), ("minecraft:iron_chain", true));
        assert_eq!(v("1.21.8").block("minecraft:iron_chain"), ("minecraft:chain", true));
        assert_eq!(v("1.21.8").block("minecraft:chain"), ("minecraft:chain", true));
        // A wood the version lacks is the closest wood it has.
        assert_eq!(v("1.21.1").block("minecraft:pale_oak_planks"), ("minecraft:dark_oak_planks", true));
        assert_eq!(v("1.19.4").block("minecraft:stripped_cherry_log"), ("minecraft:stripped_birch_log", true));
        assert_eq!(v("1.21.11").block("minecraft:poplar_fence"), ("minecraft:birch_fence", true));
        assert_eq!(v("1.18.2").block("minecraft:mangrove_planks"), ("minecraft:oak_planks", true));
        // Nothing alike: left out, or stone in the ground.
        assert_eq!(v("1.21.11").block("minecraft:sulfur"), ("minecraft:air", false));
        assert_eq!(v("1.21.11").block_or("minecraft:sulfur", "minecraft:stone"), ("minecraft:stone", false));
        assert_eq!(v("1.21.11").block("minecraft:bamboo_mosaic"), ("minecraft:bamboo_mosaic", true));
        assert_eq!(v("1.19.4").block("minecraft:bamboo_mosaic"), ("minecraft:air", false));
        assert_eq!(v("26.3").block("minecraft:sulfur"), ("minecraft:sulfur", true));
        // Blocks of mods, and versions newer than the lists, as they are.
        assert_eq!(v("1.18.2").block("create:andesite_casing"), ("create:andesite_casing", true));
        assert_eq!(Target::installed("26.4", 5200).block("minecraft:something_new"), ("minecraft:something_new", true));
        // Biomes.
        assert_eq!(v("1.18.2").biome("minecraft:mangrove_swamp"), "minecraft:swamp");
        assert_eq!(v("1.19").biome("minecraft:mangrove_swamp"), "minecraft:mangrove_swamp");
        assert_eq!(v("1.19.4").biome("minecraft:cherry_grove"), "minecraft:meadow");
        assert_eq!(v("1.20").biome("minecraft:cherry_grove"), "minecraft:cherry_grove");
        assert_eq!(v("1.21.1").biome("minecraft:pale_garden"), "minecraft:dark_forest");
        assert_eq!(v("1.21.11").biome("minecraft:sulfur_caves"), "minecraft:dripstone_caves");
        assert_eq!(v("26.2").biome("minecraft:dappled_forest"), "minecraft:forest");
        assert_eq!(v("1.21.11").biome("minecraft:no_such_biome"), "minecraft:plains");
        assert_eq!(v("1.16.5").biome("minecraft:cherry_grove"), "minecraft:cherry_grove");
    }

    /// For every release from 1.18.2 on, every block of every release is
    /// written as one the version has (or left out), and so is every biome.
    #[test]
    fn every_block_becomes_one_the_version_has() {
        for target in Target::releases().into_iter().filter(|t| t.lists_blocks()) {
            for (&short, &since) in blocks() {
                assert!(since <= NEWEST_LISTED && since == 0 || since > 2975, "{short} {since}");
                let name = format!("minecraft:{short}");
                let (got, _) = target.block(&name);
                let got = got.strip_prefix("minecraft:").unwrap();
                assert!(got == "air" || target.has_block(got), "{}: {short} became {got}", target.name);
            }
            for &(old, ..) in RENAMED_BLOCKS {
                let old = format!("minecraft:{old}");
                let (got, kept) = target.block(&old);
                assert!(kept && target.has_block(got.strip_prefix("minecraft:").unwrap()), "{}: {old} became {got}", target.name);
            }
            for id in BIOMES_1_18_2.iter().chain(NEWER_BIOMES.iter().map(|b| &b.0)) {
                let got = target.biome(&format!("minecraft:{id}")).strip_prefix("minecraft:").unwrap().to_owned();
                let since = NEWER_BIOMES.iter().find(|b| b.0 == got).map_or(0, |b| b.1);
                assert!(BIOMES_1_18_2.contains(&got.as_str()) || since <= target.data_version, "{}: {id} became {got}", target.name);
            }
        }
    }

    #[test]
    fn biomes_have_their_old_numbers() {
        let v = |name: &str| Target::release(name).unwrap();
        assert_eq!(v("1.16.5").biome_number("minecraft:plains"), 1);
        assert_eq!(v("1.16.5").biome_number("minecraft:warm_ocean"), 44);
        assert_eq!(v("1.12.2").biome_number("minecraft:warm_ocean"), 0);
        assert_eq!(v("1.16.5").biome_number("minecraft:bamboo_jungle"), 168);
        assert_eq!(v("1.12.2").biome_number("minecraft:bamboo_jungle"), 21);
        assert_eq!(v("1.16.5").biome_number("minecraft:meadow"), 1);
        assert_eq!(v("1.12.2").void_preset(), "3;minecraft:air;127;decoration");
        assert_eq!(v("1.7.10").void_preset(), "2;0;1;");
    }
}
