//! Minecraft's own blocks: their ids, models, textures and colours, read
//! from the player's installation of the game.
//!
//! Nothing of the game is shipped with Massif. The client jar of a version
//! the player has installed (with the official launcher, CurseForge, Prism…)
//! is opened in place: every block state file gives a block, its model gives
//! the textures of its top, sides and bottom, and those give its real
//! colour, with grass and leaves tinted the way the game tints them.

pub mod connected;
pub mod instance;
pub mod kubejs;
pub mod materials;
pub mod true_color;
pub mod zip;

use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{0}")]
    Zip(#[from] zip::ZipError),
    #[error("this file has no Minecraft blocks in it")]
    NoBlocks,
}

/// A Minecraft client jar found on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jar {
    pub version: String,
    pub path: PathBuf,
}

/// Version numbers of a release (`1.21.11`, `26.3`), for ordering; `None`
/// for snapshots, previews and loaders.
pub fn release_numbers(version: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = version.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() >= 2)
}

/// Where applications keep their data, on each system: `%APPDATA%` on
/// Windows, Application Support on a Mac, `~/.local/share` (or
/// `$XDG_DATA_HOME`) on Linux.
fn app_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Library/Application Support"));
        dirs.push(std::env::var_os("XDG_DATA_HOME").map_or_else(|| home.join(".local/share"), PathBuf::from));
    }
    dirs
}

/// The version of the game a client jar is, from its `version.json`
/// (`"id": "1.20.1"`): for a jar kept under a folder not named by the
/// version (the Modrinth App's `1.20.1-fabric-0.16.10`, a loader's). A jar
/// without one (a loader's empty jar) is not the game's.
pub fn jar_version(jar: &Path) -> Option<String> {
    let archive = zip::Archive::open(jar).ok()?;
    let json: Value = serde_json::from_slice(&archive.read("version.json")?.ok()?).ok()?;
    let id = json.get("id").or_else(|| json.get("name"))?.as_str()?;
    // "1.20.1 / 2c7a3b..." in some old jars.
    let id = id.split([' ', '/']).next()?.to_owned();
    release_numbers(&id).is_some().then_some(id)
}

/// The client jars in a launcher's folder of versions, `<id>/<id>.jar`
/// (the official launcher, CurseForge's `Install/versions`, the Modrinth
/// App's `meta/versions`, GDLauncher's): by the folder's name when it is a
/// release, else by the version the jar says it is.
fn jars_in_versions(dir: &Path, jars: &mut Vec<Jar>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let jar = entry.path().join(format!("{name}.jar"));
        if !jar.is_file() {
            continue;
        }
        if release_numbers(&name).is_some() {
            jars.push(Jar { version: name, path: jar });
        } else if (name.chars().next().is_some_and(|c| c.is_ascii_digit()) || name.contains("fabric") || name.contains("forge") || name.contains("quilt"))
            && let Some(version) = jar_version(&jar)
        {
            jars.push(Jar { version, path: jar });
        }
    }
}

/// The client jars among a launcher's libraries: Prism's, MultiMC's and
/// GDLauncher's `com/mojang/minecraft/<v>/minecraft-<v>-client.jar`,
/// ATLauncher's and others' `net/minecraft/client/<v>/client-<v>….jar`.
fn jars_in_libraries(dir: &Path, jars: &mut Vec<Jar>) {
    for sub in ["com/mojang/minecraft", "net/minecraft/client"] {
        let Ok(read) = std::fs::read_dir(dir.join(sub)) else { continue };
        for entry in read.flatten() {
            let version = entry.file_name().to_string_lossy().into_owned();
            if release_numbers(&version).is_none() {
                continue;
            }
            let named = entry.path().join(format!("minecraft-{version}-client.jar"));
            if named.is_file() {
                jars.push(Jar { version, path: named });
                continue;
            }
            // Another name: the one that is the game's (Forge keeps its
            // srg and slim jars there too).
            let Ok(files) = std::fs::read_dir(entry.path()) else { continue };
            let mut found: Vec<PathBuf> = files.flatten().map(|f| f.path()).filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar")) && f.file_name().is_some_and(|n| n.to_string_lossy().contains("client"))).collect();
            found.sort_by_key(|f| (f.file_name().is_some_and(|n| n.to_string_lossy().contains("srg") || n.to_string_lossy().contains("slim")), f.clone()));
            if let Some(jar) = found.into_iter().find(|f| jar_version(f).is_some()) {
                jars.push(Jar { version, path: jar });
            }
        }
    }
}

/// The client jars a launcher keeps under its folder `root`, wherever it
/// keeps them: its versions, its libraries.
pub fn jars_in(root: &Path) -> Vec<Jar> {
    let mut jars = Vec::new();
    for sub in ["versions", "Install/versions", "install/versions", "meta/versions", "datastore/versions", ".minecraft/versions", "minecraft/versions"] {
        jars_in_versions(&root.join(sub), &mut jars);
    }
    for sub in ["libraries", "data/libraries", "meta/libraries", "datastore/libraries"] {
        jars_in_libraries(&root.join(sub), &mut jars);
    }
    jars
}

/// Newest first, one jar a version.
fn sorted(mut jars: Vec<Jar>) -> Vec<Jar> {
    jars.sort_by_key(|j| std::cmp::Reverse(release_numbers(&j.version)));
    jars.dedup_by(|a, b| a.version == b.version);
    jars
}

/// The client jars of the game installed beside a mod's jar or an
/// instance's folder `path`: the launcher's versions and libraries found
/// walking up from it (CurseForge keeps its instances' mods under
/// `minecraft/Instances/<name>/mods` and the game under
/// `minecraft/Install/versions`, on whatever drive it is; the Modrinth App
/// its profiles beside `meta/versions`; Prism and MultiMC their instances
/// beside `libraries`), newest first.
pub fn jars_near(path: &Path) -> Vec<Jar> {
    let mut jars = Vec::new();
    for dir in path.ancestors().skip(1).take(6) {
        jars.extend(jars_in(dir));
    }
    sorted(jars)
}

/// The folders launchers live in, on this computer.
fn launcher_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let homes: Vec<PathBuf> = ["HOME", "USERPROFILE"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect();
    for home in &homes {
        for sub in ["Library/Application Support/minecraft", ".minecraft", "Documents/curseforge/minecraft", "curseforge/minecraft", ".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher"] {
            roots.push(home.join(sub));
        }
    }
    let mut data = app_data_dirs();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        data.push(PathBuf::from(local));
    }
    for app in data {
        for sub in [".minecraft", "PrismLauncher", "MultiMC", "ModrinthApp", "com.modrinth.theseus", "ATLauncher", "atlauncher", "gdlauncher_next", "gdlauncher_carbon", "gdlauncher_carbon/data", "GDLauncher", "Programs/PrismLauncher", "Programs/MultiMC", "Programs/ATLauncher"] {
            roots.push(app.join(sub));
        }
    }
    // Windows: CurseForge and others installed on another drive.
    if cfg!(windows) {
        for letter in b'C'..=b'Z' {
            let drive = PathBuf::from(format!("{}:\\", letter as char));
            if !drive.is_dir() {
                continue;
            }
            for sub in ["curseforge/minecraft", "CurseForge/minecraft", "Games/CurseForge/minecraft", "Games/curseforge/minecraft", "minecraft", "Minecraft", "Games/Minecraft", "MultiMC", "PrismLauncher", "ATLauncher"] {
                roots.push(drive.join(sub));
            }
        }
    }
    // Launchers installed on other drives (CurseForge often is).
    for root in ["/Volumes", "/media", "/mnt"] {
        if let Ok(read) = std::fs::read_dir(root) {
            for volume in read.flatten() {
                for sub in ["minecraft", "curseforge/minecraft", "CurseForge/minecraft"] {
                    roots.push(volume.path().join(sub));
                }
            }
        }
    }
    roots
}

/// Client jars of released versions, newest first, wherever the launchers
/// of this computer keep them (see [`jars_in`]).
pub fn find_jars() -> Vec<Jar> {
    let mut jars = Vec::new();
    for root in launcher_roots() {
        jars.extend(jars_in(&root));
    }
    sorted(jars)
}

/// The number a client jar stamps on the worlds it saves (its
/// `version.json` says, from 1.14 on).
pub fn data_version(jar: &Path) -> Option<i32> {
    let archive = zip::Archive::open(jar).ok()?;
    let json: Value = serde_json::from_slice(&archive.read("version.json")?.ok()?).ok()?;
    let version = json.get("world_version")?.as_i64()?;
    i32::try_from(version).ok()
}

/// How the game colours a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    None,
    /// By the biome's grass colour (plains here).
    Grass,
    /// By the biome's foliage colour.
    Foliage,
    Water,
    Fixed([u8; 3]),
}

impl Tint {
    pub fn color(self) -> [u8; 3] {
        match self {
            Tint::None => [255, 255, 255],
            Tint::Grass => [0x91, 0xbd, 0x59],
            Tint::Foliage => [0x77, 0xab, 0x2f],
            Tint::Water => [0x3f, 0x76, 0xe4],
            Tint::Fixed(c) => c,
        }
    }
}

/// Groups of the block palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Natural,
    Stone,
    Ores,
    Wood,
    Plants,
    Building,
    Colored,
    Nether,
    End,
    Other,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Natural,
        Category::Stone,
        Category::Ores,
        Category::Wood,
        Category::Plants,
        Category::Building,
        Category::Colored,
        Category::Nether,
        Category::End,
        Category::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Category::Natural => "Soil, sand and snow",
            Category::Stone => "Stone",
            Category::Ores => "Ores and minerals",
            Category::Wood => "Wood",
            Category::Plants => "Plants and leaves",
            Category::Building => "Building blocks",
            Category::Colored => "Wool, concrete, terracotta, glass",
            Category::Nether => "Nether",
            Category::End => "End",
            Category::Other => "Other",
        }
    }

    /// The group of a block by its name, and, for a name that says nothing
    /// (a mod's `barley`, `cattail`, `aspen_fence`), by how it is drawn:
    /// a cross of two planes, a crop, leaves or a coral fan are plants
    /// (not a potted one), a block drawn with planks or a log is wood, one
    /// drawn with bricks, cobble, stone or tiles a building block.
    pub fn of_block(id: &str, models: &[String], textures: [Option<&str>; 2], full_cube: bool) -> Self {
        let n = id.rsplit(':').next().unwrap_or(id);
        // A mod's blocks of ground (`silt_grass_block`, `peat_dirt`,
        // `volcanic_ash`, and those not quite whole, `peat_dirt_path`,
        // `silt_farmland`) are soil, which their names would put among the
        // plants or nowhere; the game's keep their own groups.
        // (A part block whose name is a plant's, `sand_lily`, stays a plant.)
        if crate::namespace(id) != "minecraft" && (full_cube || Category::of(id) != Category::Plants) {
            const GROUND: [&str; 17] = ["dirt", "podzol", "mycelium", "nylium", "farmland", "mud", "sand", "gravel", "clay", "snow", "ice", "ash", "soil", "silt", "peat", "loam", "permafrost"];
            const MADE: [&str; 26] = [
                "brick", "bricks", "tile", "tiles", "polished", "chiseled", "smooth", "cut", "pillar", "sandstone", "glass", "concrete", "terracotta", "lamp", "slab", "stairs", "wall", "fence", "gate", "door", "trapdoor", "button", "plate", "sign", "potted", "pot",
            ];
            let words = || n.split('_');
            if (n.ends_with("grass_block") || words().any(|w| GROUND.contains(&w))) && !words().any(|w| MADE.contains(&w)) {
                return Category::Natural;
            }
        }
        let named = Category::of(id);
        if named != Category::Other {
            return named;
        }
        const PLANT_MODELS: [&str; 12] = [
            "block/cross", "block/tinted_cross", "block/crop", "block/leaves", "block/coral_fan", "block/coral_wall_fan", "block/template_seagrass", "block/attached_stem", "block/stem_growth0", "block/stem_growth7", "block/flowerbed_1", "block/template_azalea",
        ];
        if !n.starts_with("potted_") && models.iter().any(|m| PLANT_MODELS.contains(&m.as_str())) {
            return Category::Plants;
        }
        let drawn = |part: &str| textures.iter().flatten().any(|t| t.rsplit('/').next().is_some_and(|f| f.contains(part)) || t.contains(&format!("{part}/")));
        if drawn("planks") || drawn("_log") || drawn("stripped") || textures.iter().flatten().any(|t| t.ends_with("/log")) {
            Category::Wood
        } else if drawn("brick") || drawn("cobble") || drawn("stone") || drawn("tile") {
            Category::Building
        } else if drawn("leaves") {
            Category::Plants
        } else {
            Category::Other
        }
    }

    fn of(id: &str) -> Self {
        // A mod's blocks are sorted by their names as the game's are.
        let n = id.rsplit(':').next().unwrap_or(id);
        let has = |s: &str| n.contains(s);
        const COLORS: [&str; 16] = [
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown",
            "green", "red", "black",
        ];
        if has("wool") || has("concrete") || has("terracotta") || has("glass") || COLORS.iter().any(|c| n.starts_with(c) && (has("carpet") || has("candle") || has("bed") || has("banner") || has("shulker"))) {
            Category::Colored
        } else if has("nether") || has("crimson") || has("warped") || has("soul") || has("basalt") || has("blackstone") || has("magma") || has("glowstone") || has("shroomlight") || has("quartz") {
            Category::Nether
        } else if has("end_") || has("purpur") || has("chorus") || n == "end_stone" {
            Category::End
        } else if has("_ore") || n.starts_with("raw_") || ["coal_block", "iron_block", "gold_block", "diamond_block", "emerald_block", "lapis_block", "redstone_block", "copper_block", "netherite_block", "amethyst_block", "budding_amethyst"].contains(&n) {
            Category::Ores
        } else if has("leaves") || has("sapling") || has("flower") || has("grass") && n != "grass_block" || has("fern") || has("mushroom") || has("vine") || has("moss") && !has("cobblestone") && !has("brick") || has("azalea") || has("bush") || has("tulip") || has("orchid") || has("daisy") || has("poppy") || has("dandelion") || has("lily") || has("kelp") || has("coral") || has("bamboo") && !has("planks") && !has("mosaic") && !has("block") || has("cactus") || has("cane") || has("roots") || has("dripleaf") || has("petals") || has("weed") || has("beard") || has("hyacinth") && !has("lamp") || has("bloom") && !has("block") || has("blossom") {
            Category::Plants
        } else if has("planks") || has("_log") || has("_wood") || has("stripped") || has("bamboo") {
            Category::Wood
        } else if ["grass_block", "dirt", "coarse_dirt", "rooted_dirt", "podzol", "mycelium", "mud", "clay", "gravel", "sand", "red_sand", "snow_block", "snow", "ice", "packed_ice", "blue_ice", "powder_snow", "dirt_path", "farmland", "suspicious_sand", "suspicious_gravel", "pale_moss_block"].contains(&n) {
            Category::Natural
        } else if has("brick") || has("polished") || has("chiseled") || has("cut_") || has("smooth") || has("tiles") || has("slab") || has("stairs") || has("wall") || has("pillar") {
            Category::Building
        } else if has("stone") || has("granite") || has("diorite") || has("andesite") || has("deepslate") || has("tuff") || has("calcite") || has("dripstone") || has("obsidian") || n == "bedrock" {
            Category::Stone
        } else {
            Category::Other
        }
    }
}

/// The first frame of a texture, as RGBA.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// Mean colour of the visible pixels, tinted.
    pub fn mean(&self, tint: Tint) -> [u8; 3] {
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for p in self.rgba.as_chunks::<4>().0 {
            if p[3] > 16 {
                for k in 0..3 {
                    sum[k] += p[k] as u64;
                }
                n += 1;
            }
        }
        let t = tint.color();
        [0, 1, 2].map(|k| ((sum[k] / n.max(1)) * t[k] as u64 / 255) as u8)
    }

    /// The texture with a tint applied.
    pub fn tinted(&self, tint: Tint) -> Texture {
        if tint == Tint::None {
            return self.clone();
        }
        let t = tint.color();
        let mut rgba = self.rgba.clone();
        for p in rgba.as_chunks_mut::<4>().0 {
            for k in 0..3 {
                p[k] = (p[k] as u32 * t[k] as u32 / 255) as u8;
            }
        }
        Texture { rgba, ..*self }
    }
}

/// One block of the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Namespaced id, `minecraft:grass_block`.
    pub id: String,
    /// `Grass Block`.
    pub name: String,
    /// Texture keys (`block/grass_block_top`).
    pub top: Option<String>,
    pub side: Option<String>,
    pub bottom: Option<String>,
    /// Tints of the top and the sides.
    pub top_tint: Tint,
    pub side_tint: Tint,
    /// A whole cube: usable as ground.
    pub full_cube: bool,
    /// Drawn see-through by the game: glass, stained glass, ice (its model
    /// says so of its textures since 26.1, `force_translucent`).
    pub translucent: bool,
    /// Two blocks high, in a lower and an upper half (`half=lower`,
    /// `half=upper`): tall plants, doors.
    pub tall: bool,
    /// Real colours of the top and the sides.
    pub color: [u8; 3],
    pub side_color: [u8; 3],
    pub category: Category,
}

/// Every block of a version of the game.
#[derive(Debug, Clone)]
pub struct Catalogue {
    pub version: String,
    pub source: PathBuf,
    /// Sorted by id.
    pub blocks: Vec<Block>,
    pub textures: HashMap<String, Arc<Texture>>,
    /// The mods whose blocks were read beside the game's, and the resource
    /// packs and lists of blocks read with them, in their order.
    pub mods: Vec<ModInfo>,
    /// Mod jars that could not be read, and why: their blocks are left out.
    pub skipped: Vec<(PathBuf, String)>,
    /// What lists of blocks made for WorldPainter say of a mod's blocks
    /// (`conquest_wp.csv`): their properties, and what kind of block each
    /// is.
    pub definitions: HashMap<String, materials::Definition>,
    /// The blocks the game registers, by namespace, of the namespaces whose
    /// registry's list was read (some mods write one: `blocks-pls.txt`).
    pub registered: HashMap<String, std::collections::HashSet<String>>,
    /// The biomes the game, the mods and the data packs read define
    /// (`data/<namespace>/worldgen/biome/`), the game's first, then each
    /// source's, by name.
    pub biomes: Vec<BiomeInfo>,
    /// The biome colour maps of the game and the resource packs, and the
    /// packs' maps for blocks (OptiFine's): see [`true_color`].
    pub colors: Arc<true_color::Rules>,
    /// The connected textures (OptiFine's CTM, Fusion's), by the key of
    /// the texture they replace: see [`connected`].
    pub connected: Arc<connected::Table>,
    /// The mods' stonecutter recipes, a block in, a block out (by id):
    /// which shapes are cut from which block, for
    /// [`massif_core::block_families`].
    pub stonecutting: Vec<(String, String)>,
    /// The properties of the mods' blocks, by id, as their block state
    /// files name them (`layers`, `type`, `waterlogged`): what a mod's
    /// blocks have when no list of them says it.
    pub state_properties: HashMap<String, Vec<String>>,
}

/// A biome as the game or a mod defines it.
#[derive(Debug, Clone, PartialEq)]
pub struct BiomeInfo {
    /// `regions_unexplored:alpha_grove`.
    pub id: String,
    /// `Alpha Grove`, from the language file when it names it.
    pub name: String,
    /// Where it comes from: `Minecraft`, or the mod's or data pack's name.
    pub source: String,
    /// The colours the game gives its grass, its leaves and its water.
    pub grass: [u8; 3],
    pub foliage: [u8; 3],
    pub water: [u8; 3],
    /// Its colour on the map: its grass's (its water's for a sea, a river
    /// or a lake), mixed with a colour of its own drawn from its id so
    /// that biomes of the same grass stay apart.
    pub color: [u8; 3],
    /// What the game reads to colour it: its temperature, downfall and
    /// own colours (see [`true_color`]).
    pub climate: true_color::Climate,
}

impl BiomeInfo {
    /// The registry's entry for it.
    pub fn def(&self) -> BiomeDef {
        BiomeDef {
            id: self.id.clone(),
            name: self.name.clone(),
            color: self.color,
            group: if self.source == MINECRAFT { String::new() } else { self.source.clone() },
            grass: Some(self.grass),
            foliage: Some(self.foliage),
        }
    }
}

/// The source of the game's own biomes.
pub const MINECRAFT: &str = "Minecraft";

/// The block tags whose blocks are plants (the game's: a mod adds its own
/// blocks to them), whatever their names and models say.
const PLANT_TAGS: [&str; 10] = [
    "minecraft:flowers",
    "minecraft:small_flowers",
    "minecraft:tall_flowers",
    "minecraft:saplings",
    "minecraft:leaves",
    "minecraft:crops",
    "minecraft:cave_vines",
    "minecraft:replaceable_by_trees",
    "minecraft:bee_growables",
    "minecraft:climbable",
];

/// A colour as a biome's file gives it: a number (`6975545`), or since
/// 26.1 a hex string (`"#6a7039"`).
fn rgb_of(v: Option<&Value>) -> Option<[u8; 3]> {
    let v = v?;
    let n = match v.as_i64() {
        Some(n) => n,
        None => i64::from_str_radix(v.as_str()?.strip_prefix('#')?, 16).ok()?,
    };
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// A biome read from its file (`id`, `source`), with its name and the
/// game's colour maps.
fn biome_info(id: &str, source: &str, json: &Value, name: Option<&String>, rules: &true_color::Rules) -> BiomeInfo {
    let effects = json.get("effects");
    let temperature = json.get("temperature").and_then(Value::as_f64).unwrap_or(0.8) as f32;
    let downfall = json.get("downfall").and_then(Value::as_f64).unwrap_or(0.4) as f32;
    let modifier = match effects.and_then(|e| e.get("grass_color_modifier")).and_then(Value::as_str) {
        Some("swamp") => true_color::Modifier::Swamp,
        Some("dark_forest") => true_color::Modifier::DarkForest,
        _ => true_color::Modifier::None,
    };
    let climate = true_color::Climate {
        id: id.to_owned(),
        temperature,
        downfall,
        grass: rgb_of(effects.and_then(|e| e.get("grass_color"))),
        foliage: rgb_of(effects.and_then(|e| e.get("foliage_color"))),
        dry_foliage: rgb_of(effects.and_then(|e| e.get("dry_foliage_color"))),
        water: rgb_of(effects.and_then(|e| e.get("water_color"))).unwrap_or(true_color::WATER),
        modifier,
    };
    let grass = rules.grass(&climate, 64);
    let foliage = rules.foliage(&climate, 64);
    let water = rules.water(&climate, 62);
    let path = id.rsplit(':').next().unwrap_or(id);
    let wet = path.split(['_', '/']).any(|w| ["ocean", "river", "lake", "sea", "reef", "lagoon"].contains(&w));
    let own = massif_core::registry::biome_hash_color(id);
    let base = if wet { water } else { grass };
    let color = std::array::from_fn(|k| ((base[k] as u32 * 3 + own[k] as u32 * 2) / 5) as u8);
    BiomeInfo { id: id.to_owned(), name: name.cloned().unwrap_or_else(|| massif_core::registry::biome_display_name(id)), source: source.to_owned(), grass, foliage, water, color, climate }
}

/// `data/<ns>/worldgen/biome/<path>.json` → `<ns>:<path>`.
fn biome_key(name: &str) -> Option<String> {
    let rest = name.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest.strip_prefix("worldgen/biome/")?.strip_suffix(".json")?;
    (!ns.is_empty() && !path.is_empty()).then(|| format!("{ns}:{path}"))
}

/// The names of the properties a block state file's variants or parts
/// name, in their order.
fn state_property_names(state: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |k: &str| {
        if !k.is_empty() && !out.iter().any(|o| o == k) {
            out.push(k.to_owned());
        }
    };
    if let Some(Value::Object(variants)) = state.get("variants") {
        for key in variants.keys() {
            key.split(',').filter_map(|kv| kv.split_once('=')).for_each(|(k, _)| add(k));
        }
    }
    if let Some(Value::Array(parts)) = state.get("multipart") {
        for part in parts {
            let Some(when) = part.get("when") else { continue };
            let list: Vec<&Value> = match when.get("OR").or_else(|| when.get("AND")) {
                Some(Value::Array(a)) => a.iter().collect(),
                _ => vec![when],
            };
            for w in list {
                if let Value::Object(o) = w {
                    o.keys().for_each(|k| add(k));
                }
            }
        }
    }
    out
}

/// Whether a file is a recipe: `data/<ns>/recipes/…` (`recipe/` since
/// 1.21).
fn is_recipe(name: &str) -> bool {
    name.strip_prefix("data/").and_then(|r| r.split_once('/')).is_some_and(|(_, r)| (r.starts_with("recipes/") || r.starts_with("recipe/")) && r.ends_with(".json"))
}

/// A stonecutter recipe's block in and block out, by id.
pub fn stonecutting(bytes: &[u8]) -> Option<(String, String)> {
    // Most recipes are not the stonecutter's: not parsed.
    if !bytes.windows(12).any(|w| w == b"stonecutting") {
        return None;
    }
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v.get("type")?.as_str()? != "minecraft:stonecutting" {
        return None;
    }
    let id = |s: &str| if s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") };
    let input = match v.get("ingredient")? {
        Value::Object(o) => o.get("item")?.as_str()?.to_owned(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let output = match v.get("result")? {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("item").or_else(|| o.get("id"))?.as_str()?.to_owned(),
        _ => return None,
    };
    Some((id(&input), id(&output)))
}

/// `data/<ns>/tags/block/<path>.json` (`tags/blocks/` before 1.21) →
/// `<ns>:<path>`.
fn block_tag_key(name: &str) -> Option<String> {
    let rest = name.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest.strip_prefix("tags/block/").or_else(|| rest.strip_prefix("tags/blocks/"))?.strip_suffix(".json")?;
    (!ns.is_empty() && !path.is_empty()).then(|| format!("{ns}:{path}"))
}

/// Whether a folder is KubeJS's (`kubejs` in an instance).
fn is_kubejs(path: &Path) -> bool {
    path.is_dir() && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("kubejs"))
}

/// The tile a block shows of a texture made for Fusion, from its
/// `.mcmeta`: `(x, y, width, height)` in a texture `w` × `h`. Fusion draws
/// a connected texture (`connecting`) by a block's neighbours, a
/// `continuous` one across blocks and a `random` one by position, from a
/// sheet of tiles; Massif shows a block alone, so the tile of a block with
/// no neighbours (the first, but `overlay`'s), at the size of one block.
/// `None` for a texture that is not Fusion's or does not divide.
pub fn fusion_tile(meta: &Value, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let f = meta.get("fusion")?;
    let count = |key: &str, default: u32| f.get(key).and_then(Value::as_u64).map_or(default, |v| v.clamp(1, 256) as u32);
    // Columns and rows of the sheet, and the tile shown.
    let (columns, rows, tile) = match f.get("type").and_then(Value::as_str)? {
        "continuous" | "random" => (count("columns", 1), count("rows", 1), (0, 0)),
        "connecting" => match f.get("layout").and_then(Value::as_str).unwrap_or("full") {
            "simple" => (4, 4, (0, 0)),
            "vertical" => (1, 4, (0, 0)),
            "horizontal" => (4, 1, (0, 0)),
            "compact" | "pieced" => (5, 1, (0, 0)),
            "overlay" => (6, 3, (1, 1)),
            _ => (8, 6, (0, 0)),
        },
        _ => return None,
    };
    let (tw, th) = (w / columns, h / rows);
    (tw > 0 && th > 0).then_some((tile.0 * tw, tile.1 * th, tw, th))
}

/// A file of a data pack left as a folder: its name in the pack, its bytes.
type FolderFile = (String, Vec<u8>);

/// The files of a data pack left as a folder (`datapacks/<pack>/`), by
/// their names in it (`data/<ns>/…`), read as a zip's would be: its biomes
/// and block tags.
fn folder_files(dir: &Path) -> Vec<FolderFile> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join("data")];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                if (biome_key(&name).is_some() || block_tag_key(&name).is_some() || is_recipe(&name)) && let Ok(bytes) = std::fs::read(&p) {
                    out.push((name, bytes));
                }
            }
        }
    }
    out.sort();
    out
}

/// The files of a resource pack left as a folder that say something of
/// colours (see [`true_color::wanted`]), by their names in it.
fn color_files(dir: &Path) -> Vec<FolderFile> {
    let mut out = Vec::new();
    let mut stack = vec![dir.join("assets/minecraft")];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                let name = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                if true_color::wanted(&name) && let Ok(bytes) = std::fs::read(&p) {
                    out.push((name, bytes));
                }
            }
        }
    }
    out.sort();
    out
}

/// What a file read beside the game is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A mod's jar: blocks, their models and textures.
    Mod,
    /// A resource pack: textures and models over the mods'.
    Pack,
    /// A list of blocks made for WorldPainter.
    List,
}

/// A group of blocks as a mod shows them in the game's creative inventory
/// (Conquest Reforged's tabs: Cobble and Brick, Advanced Masonry…).
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    /// Its blocks, in the mod's order.
    pub blocks: Vec<String>,
}

/// A mod read beside the game: Conquest Reforged, Chipped, Macaw's…
#[derive(Debug, Clone, PartialEq)]
pub struct ModInfo {
    /// Its name, as its jar gives it (`mods.toml`, `fabric.mod.json`), or
    /// the file's name.
    pub name: String,
    pub path: PathBuf,
    pub kind: SourceKind,
    /// The namespaces of its blocks (`conquest`), sorted.
    pub namespaces: Vec<String>,
    /// How many blocks it adds: a mod's blocks the game does not have, a
    /// list's blocks.
    pub blocks: usize,
    /// The version of the game it is made for (`1.20.1`), from what it
    /// depends on or its file's name.
    pub minecraft: Option<String>,
    /// Its creative tabs, in its order, when it gives them.
    pub groups: Vec<Group>,
    /// Block files of its jar that the game does not register (models left
    /// in it), by the registry's list beside a list of blocks: left out.
    pub unregistered: usize,
}

impl Default for ModInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: PathBuf::new(),
            kind: SourceKind::Mod,
            namespaces: Vec::new(),
            blocks: 0,
            minecraft: None,
            groups: Vec::new(),
            unregistered: 0,
        }
    }
}

/// What is wrong with a block state typed by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateSyntax {
    /// Nothing typed.
    Empty,
    /// The id has characters a block id cannot have (`a-z 0-9 _ - . /`,
    /// and one `:` after the namespace).
    BadId(String),
    /// A property is not `name=value`, or the list is not closed by `]`.
    BadProperty(String),
}

/// A block state typed by hand (`regions_unexplored:redwood_log[axis=x]`,
/// `Stone`): its id lowercased, `minecraft:` when it names no namespace,
/// and its properties as `[name=value,…]`.
pub fn parse_state(text: &str) -> Result<String, StateSyntax> {
    let text = text.trim();
    if text.is_empty() {
        return Err(StateSyntax::Empty);
    }
    let (id, props) = match text.split_once('[') {
        Some((id, rest)) => {
            let Some(inner) = rest.trim_end().strip_suffix(']') else { return Err(StateSyntax::BadProperty(rest.to_owned())) };
            (id.trim(), Some(inner))
        }
        None => (text, None),
    };
    let id = id.to_lowercase().replace(' ', "_");
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", &id));
    let ok = |part: &str, extra: &[char]| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.".contains(c) || extra.contains(&c));
    if !ok(ns, &[]) || !ok(path, &['/']) {
        return Err(StateSyntax::BadId(id.clone()));
    }
    let mut out = format!("{ns}:{path}");
    if let Some(inner) = props {
        let mut pairs = Vec::new();
        for kv in inner.split(',').map(str::trim).filter(|kv| !kv.is_empty()) {
            let Some((k, v)) = kv.split_once('=') else { return Err(StateSyntax::BadProperty(kv.to_owned())) };
            let (k, v) = (k.trim().to_lowercase(), v.trim().to_lowercase());
            if !ok(&k, &[]) || !ok(&v, &[]) {
                return Err(StateSyntax::BadProperty(kv.to_owned()));
            }
            pairs.push(format!("{k}={v}"));
        }
        if !pairs.is_empty() {
            out += &format!("[{}]", pairs.join(","));
        }
    }
    Ok(out)
}

/// The namespace of a block id: `conquest` of `conquest:old_bricks`.
pub fn namespace(id: &str) -> &str {
    id.split_once(':').map_or("minecraft", |(ns, _)| ns)
}

impl Catalogue {
    pub fn block(&self, id: &str) -> Option<&Block> {
        let id = id.split('[').next().unwrap_or(id);
        self.blocks.binary_search_by(|b| b.id.as_str().cmp(id)).ok().map(|i| &self.blocks[i])
    }

    pub fn texture(&self, key: &str) -> Option<&Arc<Texture>> {
        self.textures.get(key)
    }

    /// The mod a block comes from, when it is a mod's.
    /// A biome the game or a mod read defines, by its id.
    /// The colour of a block's top as its texture has it, before any tint
    /// (grass and leaves grey), and the tint the game gives that top: the
    /// two halves of its colour in the game (see [`true_color`]).
    pub fn true_top(&self, id: &str) -> Option<([u8; 3], Option<true_color::Kind>)> {
        let b = self.block(id)?;
        let key = b.top.as_ref().or(b.side.as_ref());
        let mean = key.and_then(|k| self.texture(k)).map_or(b.color, |t| t.mean(Tint::None));
        let tint = if b.top.is_some() { b.top_tint } else { b.side_tint };
        let kind = match tint {
            Tint::None => None,
            Tint::Fixed(c) => true_color::kind_of(id, true).filter(|k| matches!(k, true_color::Kind::Spruce | true_color::Kind::Birch)).or(Some(true_color::Kind::Fixed(c))),
            _ => true_color::kind_of(id, true),
        };
        Some((mean, kind))
    }

    /// The colour a block's faces tinted `tint` take in a biome (plains
    /// when `None`) at a height, as the game and the packs read colour
    /// maps (see [`true_color`]): white for an untinted face.
    pub fn tint_rgb(&self, id: &str, tint: Tint, biome: Option<&true_color::Climate>, y: i32) -> [u8; 3] {
        let kind = match tint {
            Tint::None => return [255; 3],
            Tint::Fixed(c) => true_color::kind_of(id, true).filter(|k| matches!(k, true_color::Kind::Spruce | true_color::Kind::Birch)).unwrap_or(true_color::Kind::Fixed(c)),
            Tint::Water => true_color::Kind::Water,
            Tint::Foliage => true_color::Kind::Foliage,
            Tint::Grass => true_color::kind_of(id, true).unwrap_or(true_color::Kind::Grass),
        };
        let plains = true_color::Climate::plains();
        self.colors.tint(id, Some(kind), biome.unwrap_or(&plains), y).unwrap_or([255; 3])
    }

    pub fn biome(&self, id: &str) -> Option<&BiomeInfo> {
        self.biomes.iter().find(|b| b.id == id)
    }

    pub fn mod_of(&self, id: &str) -> Option<&ModInfo> {
        let ns = namespace(id);
        (ns != "minecraft").then(|| self.mods.iter().find(|m| m.kind == SourceKind::Mod && m.namespaces.iter().any(|n| n == ns))).flatten()
    }

    /// Whether a state is one of the game's, by what lists of blocks say of
    /// its block: `Ok` too when no list speaks of it.
    pub fn check_state(&self, state: &str) -> Result<(), materials::StateProblem> {
        let name = state.split('[').next().unwrap_or(state);
        if self.definitions.contains_key(name) {
            return materials::check(state, &self.definitions);
        }
        let ns = namespace(name);
        // The registry's own list is the last word on which blocks exist.
        if let Some(ids) = self.registered.get(ns) {
            return if ids.contains(name) { Ok(()) } else { Err(materials::StateProblem::UnknownBlock(name.to_owned())) };
        }
        let listed = self.mods.iter().any(|m| m.kind == SourceKind::List && m.namespaces.iter().any(|n| n == ns));
        if listed { Err(materials::StateProblem::UnknownBlock(name.to_owned())) } else { Ok(()) }
    }
}

/// `grass_block` → `Grass Block`, `conquest:old_red_bricks` → `Old Red
/// Bricks`.
fn display_name(id: &str) -> String {
    id.rsplit(':')
        .next()
        .unwrap_or(id)
        .rsplit('/')
        .next()
        .unwrap_or(id)
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key of a model or texture reference: the game's without their
/// namespace (`minecraft:block/stone` → `block/stone`, as `block/stone`),
/// a mod's with it (`conquest:block/old_bricks`).
fn key(path: &str) -> String {
    // `:block/x`, with an empty namespace, is the game's (as the game reads
    // it: Conquest Reforged's models name the game's textures so).
    let path = path.trim();
    path.strip_prefix("minecraft:").or_else(|| path.strip_prefix(':')).unwrap_or(path).to_owned()
}

/// The key of a file of a jar under `assets/<namespace>/<kind>/`, without
/// its extension: `assets/conquest/models/block/x.json` → `conquest:block/x`,
/// the game's without its namespace. `None` for another file.
fn asset_key(name: &str, kind: &str, ext: &str) -> Option<String> {
    let rest = name.strip_prefix("assets/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest.strip_prefix(kind)?.strip_prefix('/')?.strip_suffix(ext)?;
    if ns.is_empty() || path.is_empty() {
        return None;
    }
    Some(if ns == "minecraft" { path.to_owned() } else { format!("{ns}:{path}") })
}

/// A mod's name as a player knows it: without the loader its jar is built
/// for, its words apart when it has none (`ConquestReforged Fabric` →
/// `Conquest Reforged`), without Minecraft's colour codes (`§6`).
fn plain_mod_name(name: &str) -> String {
    let mut clean = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            chars.next();
        } else {
            clean.push(c);
        }
    }
    let mut name = clean.trim();
    for loader in [" Fabric", " Forge", " NeoForge", " Quilt", " (Fabric)", " (Forge)", " (NeoForge)", " (Quilt)"] {
        if name.len() > loader.len() && name.is_char_boundary(name.len() - loader.len()) && name[name.len() - loader.len()..].eq_ignore_ascii_case(loader) {
            name = name[..name.len() - loader.len()].trim_end();
        }
    }
    if name.contains(' ') { name.to_owned() } else { words_apart(name) }
}

/// A word written in camel case, its words apart: a capital after a small
/// letter starts one (`ConquestReforged`), and so does the last capital of
/// a run of them before a small letter (`BiomesOPlenty` → `Biomes O
/// Plenty`, `XMLParser` → `XML Parser`).
fn words_apart(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_small = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || (prev.is_uppercase() && next_small) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// A mod's name from its jar's file name alone, for jars that do not say
/// it: the words before its first version or loader tag, apart and
/// capitalised (`ActuallyAdditions-1.12.2-r152.jar` → `Actually
/// Additions`, `sodium-fabric-0.5.13+mc1.20.1.jar` → `Sodium`,
/// `Chisel-MC1.12.2-1.0.2.45.jar` → `Chisel`).
pub fn jar_mod_name(file: &str) -> String {
    let stem = [".jar", ".zip", ".disabled"].iter().fold(file.trim(), |s, ext| if s.len() > ext.len() && s.is_char_boundary(s.len() - ext.len()) && s[s.len() - ext.len()..].eq_ignore_ascii_case(ext) { &s[..s.len() - ext.len()] } else { s });
    // Bracketed tags (`[1.12.2]`, `(Forge)`) out.
    let mut plain = String::with_capacity(stem.len());
    let mut depth = 0usize;
    for c in stem.chars() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => {
                depth = depth.saturating_sub(1);
                plain.push(' ');
            }
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let mut words: Vec<String> = Vec::new();
    for token in plain.split(['-', '_', ' ', '+']).filter(|t| !t.is_empty()) {
        let lower = token.to_ascii_lowercase();
        let after = |prefix: &str| lower.strip_prefix(prefix).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
        let version = token.starts_with(|c: char| c.is_ascii_digit()) || ["mc", "v", "r", "b", "build"].iter().any(|p| after(p)) || lower == "mc";
        let tag = matches!(lower.as_str(), "forge" | "fabric" | "neoforge" | "quilt" | "universal" | "client" | "server" | "release" | "beta" | "alpha" | "snapshot" | "final" | "deobf" | "dev" | "sources");
        if version || (tag && !words.is_empty()) {
            break;
        }
        let word = if token.chars().all(|c| !c.is_uppercase()) {
            let mut c = token.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        } else {
            words_apart(token)
        };
        words.push(word);
    }
    if words.is_empty() { stem.to_owned() } else { words.join(" ") }
}

/// What a mod's jar says of itself: its name, its id and the loader it is
/// made for, from `META-INF/neoforge.mods.toml` and `META-INF/mods.toml`
/// (NeoForge, Forge), `fabric.mod.json` and `quilt.mod.json` (Fabric,
/// Quilt) or `mcmod.info` (Forge before 1.13).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModMeta {
    pub name: Option<String>,
    pub id: Option<String>,
    pub loader: Option<&'static str>,
}

/// A value of a mod's metadata worth showing: neither empty nor a
/// placeholder of its build (`${file.jarVersion}`).
fn meta_value(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && !v.contains("${") && !v.contains("@")).then(|| v.to_owned())
}

/// The first string value of `key` in a JSON text, for `mcmod.info` files
/// that are not valid JSON (line breaks in their descriptions).
fn loose_json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))?;
    let rest = text[at + key.len() + 2..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_owned())
}

/// What a mod's jar says of itself: see [`ModMeta`].
pub fn mod_meta(archive: &zip::Archive) -> ModMeta {
    for (toml, loader) in [("META-INF/neoforge.mods.toml", "NeoForge"), ("META-INF/mods.toml", "Forge")] {
        if let Some(Ok(bytes)) = archive.read(toml) {
            let text = String::from_utf8_lossy(&bytes);
            let mut meta = ModMeta { loader: Some(loader), ..Default::default() };
            for line in text.lines() {
                if meta.name.is_none() {
                    meta.name = toml_value(line, "displayName").and_then(|v| meta_value(&v));
                }
                if meta.id.is_none() {
                    meta.id = toml_value(line, "modId").and_then(|v| meta_value(&v));
                }
            }
            return meta;
        }
    }
    for (file, loader) in [("fabric.mod.json", "Fabric"), ("quilt.mod.json", "Quilt")] {
        if let Some(Ok(bytes)) = archive.read(file) {
            let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            // Quilt's under `quilt_loader`, its name in `metadata`.
            let (id, about) = match v.get("quilt_loader") {
                Some(q) => (q.get("id"), q.get("metadata")),
                None => (v.get("id"), Some(&v)),
            };
            return ModMeta { name: about.and_then(|m| m.get("name")).and_then(Value::as_str).and_then(meta_value), id: id.and_then(Value::as_str).and_then(meta_value), loader: Some(loader) };
        }
    }
    if let Some(Ok(bytes)) = archive.read("mcmod.info") {
        let text = String::from_utf8_lossy(&bytes);
        let first = serde_json::from_str::<Value>(&text).ok().and_then(|v| match v {
            Value::Array(mut a) if !a.is_empty() => Some(a.swap_remove(0)),
            Value::Object(o) => o.get("modList").and_then(Value::as_array).and_then(|a| a.first().cloned()),
            _ => None,
        });
        let (name, id) = match &first {
            Some(m) => (m.get("name").and_then(Value::as_str).map(str::to_owned), m.get("modid").and_then(Value::as_str).map(str::to_owned)),
            None => (loose_json_string(&text, "name"), loose_json_string(&text, "modid")),
        };
        return ModMeta { name: name.and_then(|n| meta_value(&n)), id: id.and_then(|n| meta_value(&n)), loader: Some("Forge") };
    }
    ModMeta::default()
}

/// A mod's name as a player knows it, from what its jar says of itself
/// else from its file's name.
pub fn display_mod_name(meta: &ModMeta, jar: &std::path::Path) -> String {
    meta.name.as_deref().map(plain_mod_name).filter(|n| !n.is_empty()).unwrap_or_else(|| jar_mod_name(&jar.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// The value of `key` on a line of a TOML file (`displayName="Conquest
/// Reforged" #mandatory` → `Conquest Reforged`): a quoted string up to its
/// closing quote, or a bare value up to a comment.
fn toml_value(line: &str, key: &str) -> Option<String> {
    let (k, v) = line.split_once('=')?;
    if k.trim() != key {
        return None;
    }
    let v = v.trim();
    let value = match v.chars().next()? {
        q @ ('"' | '\'') => v[1..].split(q).next()?,
        _ => v.split('#').next()?.trim(),
    };
    Some(value.to_owned())
}

struct Models {
    raw: HashMap<String, Value>,
}

/// A model with its parents folded in: textures by variable, and the faces
/// of its elements (direction → texture variable, tinted?), and whether it
/// fills its whole cube.
#[derive(Default)]
struct Resolved {
    textures: HashMap<String, String>,
    /// Texture keys the model draws see-through (`force_translucent`).
    translucent: std::collections::HashSet<String>,
    faces: HashMap<String, (String, bool)>,
    full_cube: bool,
    /// The model and its parents, by key, the model first.
    chain: Vec<String>,
}

impl Models {
    fn resolve(&self, name: &str) -> Resolved {
        let mut chain = Vec::new();
        let mut keys = Vec::new();
        let mut current = key(name);
        for _ in 0..16 {
            let Some(model) = self.raw.get(&current) else { break };
            chain.push(model);
            keys.push(current.clone());
            match model.get("parent").and_then(Value::as_str) {
                Some(p) => current = key(p),
                None => break,
            }
        }
        let mut r = Resolved { chain: keys, ..Default::default() };
        // The root first, so that children override.
        for model in chain.iter().rev() {
            if let Some(t) = model.get("textures").and_then(Value::as_object) {
                for (k, v) in t {
                    // A name, or since 26.1 `{"sprite": name,
                    // "force_translucent": true}`.
                    if let Some(name) = v.as_str().or_else(|| v.get("sprite").and_then(Value::as_str)) {
                        r.textures.insert(k.clone(), name.to_owned());
                        if v.get("force_translucent").and_then(Value::as_bool) == Some(true) {
                            r.translucent.insert(key(name));
                        }
                    }
                }
            }
        }
        // Elements come from the nearest model that has them.
        if let Some(elements) = chain.iter().find_map(|m| m.get("elements").and_then(Value::as_array)) {
            for el in elements {
                let corner = |k: &str| {
                    el.get(k)
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
                        .unwrap_or_default()
                };
                if corner("from") == [0.0, 0.0, 0.0] && corner("to") == [16.0, 16.0, 16.0] {
                    r.full_cube = true;
                }
                if let Some(faces) = el.get("faces").and_then(Value::as_object) {
                    for (dir, face) in faces {
                        if let Some(t) = face.get("texture").and_then(Value::as_str) {
                            r.faces.entry(dir.clone()).or_insert((t.to_owned(), face.get("tintindex").is_some()));
                        }
                    }
                }
            }
        }
        r
    }
}

impl Resolved {
    /// Follow `#variable` references to a texture key.
    fn texture(&self, var: &str) -> Option<String> {
        let mut var = var.to_owned();
        for _ in 0..8 {
            match var.strip_prefix('#') {
                Some(name) => var = self.textures.get(name)?.clone(),
                None => return Some(key(&var)),
            }
        }
        None
    }

    fn face(&self, dirs: &[&str]) -> Option<(String, bool)> {
        dirs.iter()
            .find_map(|d| self.faces.get(*d))
            .and_then(|(v, tint)| Some((self.texture(v)?, *tint)))
    }
}

/// The first model a block state file points to.
fn first_model(state: &Value) -> Option<String> {
    let pick = |v: &Value| -> Option<String> {
        let v = if let Some(a) = v.as_array() { a.first()? } else { v };
        v.get("model").and_then(Value::as_str).map(str::to_owned)
    };
    if let Some(variants) = state.get("variants").and_then(Value::as_object) {
        return variants.values().next().and_then(pick);
    }
    state
        .get("multipart")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|part| part.get("apply").and_then(pick))
}

/// The model of a block's upper half (`half=upper`), when its states are
/// variants.
fn upper_model(state: &Value) -> Option<String> {
    let variants = state.get("variants")?.as_object()?;
    let v = variants.iter().find(|(k, _)| k.split(',').any(|kv| kv.trim() == "half=upper"))?.1;
    let v = if let Some(a) = v.as_array() { a.first()? } else { v };
    v.get("model").and_then(Value::as_str).map(str::to_owned)
}

/// Whether a block's states have a lower and an upper half (tall plants,
/// doors): `half=lower` and `half=upper` among its variants or the
/// conditions of its parts.
fn two_halves(state: &Value) -> bool {
    let text = |v: &Value| v.to_string();
    let has = |half: &str| match (state.get("variants").and_then(Value::as_object), state.get("multipart")) {
        (Some(variants), _) => variants.keys().any(|k| k.split(',').any(|kv| kv.trim() == format!("half={half}"))),
        (None, Some(parts)) => text(parts).contains(&format!("\"half\":\"{half}\"")),
        _ => false,
    };
    has("lower") && has("upper")
}

/// How the game tints a block's tinted faces.
fn tint_of(id: &str) -> Tint {
    let n = id.rsplit(':').next().unwrap_or(id);
    match n {
        "spruce_leaves" => Tint::Fixed([0x61, 0x99, 0x61]),
        "birch_leaves" => Tint::Fixed([0x80, 0xa7, 0x55]),
        // Their models are the tinted leaves', but the game gives them no
        // tint: pink and pale grey as their textures.
        "cherry_leaves" | "pale_oak_leaves" => Tint::None,
        "lily_pad" => Tint::Fixed([0x20, 0x80, 0x30]),
        "redstone_wire" => Tint::Fixed([0xd0, 0x10, 0x10]),
        _ if n.contains("water") || n == "bubble_column" => Tint::Water,
        _ if n.contains("leaves") || n == "vine" => Tint::Foliage,
        _ if n.contains("stem") => Tint::Fixed([0xe0, 0xc7, 0x1c]),
        _ => Tint::Grass,
    }
}

/// Read every block of a client jar.
pub fn load(path: &Path, version: &str) -> Result<Catalogue, AssetError> {
    load_with_mods(path, version, &[])
}

/// The version of the game a mod is made for: the first version in what
/// its `fabric.mod.json` or `mods.toml` says it depends on (`>=1.20.1`,
/// `[1.20.1,1.21)`), else in its file's name (`…-1.20.1-1.4.1.4.jar`).
fn mod_minecraft(archive: &zip::Archive, path: &Path) -> Option<String> {
    let first_version = |text: &str| -> Option<String> {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'c') {
                let end = (i..bytes.len()).find(|&j| !(bytes[j].is_ascii_digit() || bytes[j] == b'.')).unwrap_or(bytes.len());
                let candidate = text[i..end].trim_end_matches('.');
                // 1.7 to 1.21, or 26.1 and the years after.
                if release_numbers(candidate).is_some_and(|n| n.len() <= 3 && (n[0] == 1 && (7..=21).contains(&n[1]) || (24..=60).contains(&n[0]))) {
                    return Some(candidate.to_owned());
                }
                i = end;
            }
            i += 1;
        }
        None
    };
    if let Some(Ok(bytes)) = archive.read("fabric.mod.json")
        && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
        && let Some(dep) = v.pointer("/depends/minecraft")
    {
        let text = match dep {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
            other => other.as_str().unwrap_or_default().to_owned(),
        };
        if let Some(v) = first_version(&text) {
            return Some(v);
        }
    }
    for toml in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(Ok(bytes)) = archive.read(toml) {
            let text = String::from_utf8_lossy(&bytes);
            let mut in_minecraft = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_minecraft = false;
                } else if toml_value(line, "modId").as_deref() == Some("minecraft") {
                    in_minecraft = true;
                } else if in_minecraft && let Some(v) = toml_value(line, "versionRange").and_then(|r| first_version(&r)) {
                    return Some(v);
                }
            }
        }
    }
    first_version(&path.file_stem()?.to_string_lossy())
}

/// The creative tabs a mod gives as lists of blocks
/// (`assets/<ns>/groups/<tab>.txt`, as Conquest Reforged does), named by
/// its language file (`itemGroup.<ns>.<tab>`), in the order of their files.
fn mod_groups(archive: &zip::Archive, names: &[String], lang: &HashMap<String, String>) -> Vec<Group> {
    let mut files: Vec<(&str, &str, &str)> = names
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("assets/")?;
            let (ns, rest) = rest.split_once('/')?;
            let stem = rest.strip_prefix("groups/")?.strip_suffix(".txt")?;
            (!stem.contains('/') && !stem.is_empty()).then_some((ns, stem, n.as_str()))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(ns, stem, file)| {
            let text = String::from_utf8(archive.read(file)?.ok()?).ok()?;
            let blocks: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| if l.contains(':') { l.to_owned() } else { format!("{ns}:{l}") })
                .collect();
            // Without a name, the file's, less the letters that sort it
            // (`aa_advanced_masonry`).
            let bare = stem.split_once('_').filter(|(p, _)| p.len() <= 2 && p.chars().all(|c| c.is_ascii_lowercase())).map_or(stem, |(_, rest)| rest);
            let name = lang.get(&format!("itemGroup.{ns}.{stem}")).cloned().unwrap_or_else(|| display_name(bare));
            (!blocks.is_empty()).then_some(Group { name, blocks })
        })
        .collect()
}

/// Read every block of a client jar, and of the mods' jars beside it
/// (Forge, NeoForge, Fabric or Quilt alike: their `assets/<namespace>/`),
/// of resource packs (`.zip`) and of lists of blocks made for WorldPainter
/// (`.csv`), in that order: a later archive's models, states and textures
/// replace an earlier one's.
/// A mod's models may build on the game's, and its blocks keep their own
/// ids (`conquest:old_bricks`), which the export writes as they are. A mod
/// jar that does not read is left out and named in `skipped`.
///
/// Only the models and textures the blocks show are read: Conquest
/// Reforged's jar has 94,000 models and 7,600 textures for its 17,000
/// blocks.
pub fn load_with_mods(path: &Path, version: &str, mods: &[PathBuf]) -> Result<Catalogue, AssetError> {
    let archive = zip::Archive::open(path)?;
    let mut archives: Vec<(zip::Archive, Option<PathBuf>)> = vec![(archive, None)];
    let mut skipped = Vec::new();
    let mut lists: Vec<(PathBuf, materials::List)> = Vec::new();
    // Data packs left as folders: their biomes and tags.
    let mut folders: Vec<(PathBuf, Vec<FolderFile>)> = Vec::new();
    for m in mods {
        if m.is_dir() {
            // A folder of resources reads as a pack (KubeJS's `kubejs`, a
            // resource pack left unzipped); its data, as a data pack's.
            if is_kubejs(m) {
                // KubeJS's: its assets and the blocks its scripts make.
                if let Some(a) = kubejs::archive(m) {
                    archives.push((a, Some(m.clone())));
                }
            } else if m.join("assets").is_dir() {
                match zip::Archive::folder(m, &["assets"]) {
                    Ok(a) => archives.push((a, Some(m.clone()))),
                    Err(e) => skipped.push((m.clone(), e.to_string())),
                }
            }
            if m.join("data").is_dir() {
                folders.push((m.clone(), folder_files(m)));
            }
            continue;
        }
        if materials::is_list(m) {
            match materials::read(m) {
                Ok(list) => lists.push((m.clone(), list)),
                Err(e) => skipped.push((m.clone(), e)),
            }
            continue;
        }
        match zip::Archive::open(m) {
            Ok(a) => archives.push((a, Some(m.clone()))),
            Err(e) => skipped.push((m.clone(), e.to_string())),
        }
    }
    let json = |a: &zip::Archive, name: &str| -> Option<Value> { serde_json::from_slice(&a.read(name)?.ok()?).ok() };
    // Where every model, state and texture is: the last archive's wins.
    let mut model_files: HashMap<String, (usize, String)> = HashMap::new();
    let mut state_files: HashMap<String, (usize, String)> = HashMap::new();
    let mut texture_files: HashMap<String, (usize, String)> = HashMap::new();
    let mut names_en: HashMap<String, String> = HashMap::new();
    let mut biome_names: HashMap<String, String> = HashMap::new();
    // Every biome file and every block tag file, by archive.
    let mut biome_files: Vec<(String, usize, String)> = Vec::new();
    let mut tag_files: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut recipe_files: Vec<(usize, String)> = Vec::new();
    let mut mod_infos = Vec::new();
    for (k, (a, mod_path)) in archives.iter().enumerate() {
        let names: Vec<String> = a.names().map(str::to_owned).collect();
        let mut added = 0;
        let mut namespaces = std::collections::BTreeSet::new();
        let mut lang = HashMap::new();
        for n in &names {
            if let Some(key) = asset_key(n, "models", ".json") {
                model_files.insert(key, (k, n.clone()));
            } else if let Some(key) = asset_key(n, "blockstates", ".json") {
                let id = if key.contains(':') { key } else { format!("minecraft:{key}") };
                // The game's blocks are the game's: a mod may change how one
                // looks, not add one to them (Conquest Reforged leaves the
                // templates of its models among them, `parent_cube`…).
                if k > 0 && namespace(&id) == "minecraft" && !state_files.contains_key(&id) {
                    continue;
                }
                if k > 0 {
                    namespaces.insert(namespace(&id).to_owned());
                    if !state_files.contains_key(&id) {
                        added += 1;
                    }
                }
                state_files.insert(id, (k, n.clone()));
            } else if let Some(key) = asset_key(n, "textures", ".png") {
                texture_files.insert(key, (k, n.clone()));
            } else if let Some(id) = biome_key(n) {
                biome_files.push((id, k, n.clone()));
            } else if let Some(tag) = block_tag_key(n) {
                tag_files.entry(tag).or_default().push((k, n.clone()));
            } else if k > 0 && is_recipe(n) {
                recipe_files.push((k, n.clone()));
            } else if n.starts_with("assets/") && n.ends_with("/lang/en_us.json") && n.matches('/').count() == 3 {
                // A mod's names for its blocks, its biomes and its creative
                // tabs; the game's for its biomes.
                if let Some(Value::Object(map)) = json(a, n) {
                    for (key, v) in map {
                        if let Some(v) = v.as_str() {
                            if let Some(id) = key.strip_prefix("biome.").and_then(|r| r.split_once('.')).map(|(ns, p)| format!("{ns}:{}", p.replace('.', "/"))) {
                                biome_names.insert(id, v.to_owned());
                            } else if k == 0 {
                                continue;
                            } else if let Some(id) = key.strip_prefix("block.").and_then(|r| r.split_once('.')).map(|(ns, p)| format!("{ns}:{p}")) {
                                names_en.insert(id, v.to_owned());
                            } else if key.starts_with("itemGroup.") {
                                lang.insert(key, v.to_owned());
                            }
                        }
                    }
                }
            }
        }
        if let Some(p) = mod_path {
            // A jar is a mod; a zip is a resource pack, unless it says it is
            // a mod.
            let says_mod = ["fabric.mod.json", "quilt.mod.json", "META-INF/mods.toml", "META-INF/neoforge.mods.toml", "mcmod.info"].iter().any(|f| a.contains(f));
            let jar = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar"));
            // KubeJS's folder brings the blocks its scripts make, as a mod.
            let kubejs = is_kubejs(p);
            let kind = if says_mod || jar || kubejs { SourceKind::Mod } else { SourceKind::Pack };
            let name = if kubejs { "KubeJS".to_owned() } else if kind == SourceKind::Pack { p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { display_mod_name(&mod_meta(a), p) };
            mod_infos.push(ModInfo {
                name,
                path: p.clone(),
                kind,
                namespaces: namespaces.into_iter().collect(),
                blocks: added,
                minecraft: if kind == SourceKind::Mod && !kubejs { mod_minecraft(a, p) } else { None },
                groups: mod_groups(a, &names, &lang),
                unregistered: 0,
            });
        }
    }
    // What lists of blocks say, the last list's word on a block kept.
    let mut definitions: HashMap<String, materials::Definition> = HashMap::new();
    let mut registered: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for (p, list) in &lists {
        let namespaces = list.namespaces();
        mod_infos.push(ModInfo {
            name: p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            path: p.clone(),
            kind: SourceKind::List,
            namespaces: namespaces.clone(),
            blocks: list.blocks.len(),
            ..Default::default()
        });
        if let Some(ids) = &list.registered {
            for id in ids {
                registered.entry(namespace(id).to_owned()).or_default().insert(id.clone());
            }
        }
        definitions.extend(list.blocks.iter().cloned());
    }
    // The block files the game never registers are no blocks.
    if !registered.is_empty() {
        state_files.retain(|id, (k, _)| {
            let keep = registered.get(namespace(id)).is_none_or(|ids| ids.contains(id));
            if !keep && let Some(info) = k.checked_sub(1).and_then(|i| mod_infos.get_mut(i)) {
                info.unregistered += 1;
                info.blocks = info.blocks.saturating_sub(1);
            }
            keep
        });
    }
    if state_files.is_empty() {
        return Err(AssetError::NoBlocks);
    }
    // Each block, the first model its states point to, and whether it is
    // two blocks high.
    // A tall block's upper half too: a tall plant's lower half may be a
    // model of its own where its upper one is the game's cross.
    // Its id, first model, whether it is two blocks high, its upper half's
    // model, and its properties' names.
    type StateInfo = (String, Option<String>, bool, Option<String>, Vec<String>);
    let mut states: Vec<StateInfo> = state_files
        .into_par_iter()
        .filter_map(|(id, (k, n))| {
            let state = json(&archives[k].0, &n)?;
            let tall = two_halves(&state);
            let props = if namespace(&id) == "minecraft" { Vec::new() } else { state_property_names(&state) };
            Some((id, first_model(&state), tall, tall.then(|| upper_model(&state)).flatten(), props))
        })
        .collect();
    states.sort_by(|a, b| a.0.cmp(&b.0));
    // The mods' blocks' properties, as their block states name them.
    let state_properties: HashMap<String, Vec<String>> = states.iter_mut().filter(|s| !s.4.is_empty()).map(|s| (s.0.clone(), std::mem::take(&mut s.4))).collect();
    // Those models and their parents, a generation at a time, read in
    // parallel.
    let firsts: Vec<Option<String>> = states.iter().map(|(_, first, _, _, _)| first.clone()).collect();
    let mut raw: HashMap<String, Value> = HashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut wanted: Vec<String> = firsts.iter().flatten().chain(states.iter().filter_map(|s| s.3.as_ref())).map(|m| key(m)).collect();
    while !wanted.is_empty() {
        wanted.sort();
        wanted.dedup();
        wanted.retain(|w| seen.insert(w.clone()));
        let parsed: Vec<(String, Value)> = wanted
            .par_iter()
            .filter_map(|w| {
                let (k, n) = model_files.get(w)?;
                Some((w.clone(), json(&archives[*k].0, n)?))
            })
            .collect();
        wanted = parsed.iter().filter_map(|(_, v)| v.get("parent").and_then(Value::as_str).map(key)).filter(|p| !seen.contains(p)).collect();
        raw.extend(parsed);
    }
    let models = Models { raw };

    let mut blocks: Vec<Block> = states
        .iter()
        .zip(&firsts)
        .map(|((id, _, tall, upper, _), first)| {
            let model = first.as_ref().map(|m| models.resolve(m)).unwrap_or_default();
            let particle = model.textures.get("particle").and_then(|_| model.texture("#particle"));
            let tint = tint_of(id);
            let pick = |dirs: &[&str]| -> (Option<String>, Tint) {
                match model.face(dirs) {
                    Some((t, tinted)) => (Some(t), if tinted { tint } else { Tint::None }),
                    None => (particle.clone(), Tint::None),
                }
            };
            let (top, top_tint) = pick(&["up"]);
            let (side, side_tint) = pick(&["north", "south", "east", "west"]);
            let (bottom, _) = pick(&["down"]);
            // Water and lava have no model: their colour is their texture's.
            let (top, top_tint) = match id.as_str() {
                "minecraft:water" => (Some("block/water_still".to_owned()), Tint::Water),
                "minecraft:lava" => (Some("block/lava_still".to_owned()), Tint::None),
                _ => (top, top_tint),
            };
            let mut chain = model.chain.clone();
            if let Some(upper) = upper {
                chain.extend(models.resolve(upper).chain);
            }
            let category = Category::of_block(id, &chain, [top.as_deref(), side.as_deref()], model.full_cube);
            let translucent = [&top, &side, &bottom].into_iter().flatten().any(|t| model.translucent.contains(t));
            Block {
                id: id.clone(),
                name: names_en.get(id).cloned().unwrap_or_else(|| display_name(id)),
                color: [128, 128, 128],
                side_color: [128, 128, 128],
                top,
                side,
                bottom,
                top_tint,
                side_tint,
                full_cube: model.full_cube,
                translucent,
                tall: *tall,
                category,
            }
        })
        .collect();
    // The blocks a list gives that no jar has: with the map's colour of
    // the list, without textures.
    let known: std::collections::HashSet<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
    let listed: Vec<Block> = lists
        .iter()
        .flat_map(|(_, l)| &l.blocks)
        .filter(|(id, _)| !known.contains(id.as_str()))
        .map(|(id, def)| {
            let c = def.map_color.unwrap_or([128, 128, 128]);
            let tall = def.property("half").is_some_and(|p| p.values.iter().any(|v| v == "lower") && p.values.iter().any(|v| v == "upper"));
            Block { id: id.clone(), name: names_en.get(id).cloned().unwrap_or_else(|| display_name(id)), top: None, side: None, bottom: None, top_tint: Tint::None, side_tint: Tint::None, full_cube: def.plain_cube(), translucent: false, tall, color: c, side_color: c, category: Category::of(id) }
        })
        .collect();
    blocks.extend(listed);
    blocks.sort_by(|a, b| a.id.cmp(&b.id));
    blocks.dedup_by(|a, b| a.id == b.id);

    // The textures the blocks show (block textures, and the items a few
    // blocks are drawn with), with water, lava and the sky's, decoded in
    // parallel.
    let mut needed: Vec<&str> = blocks.iter().flat_map(|b| [&b.top, &b.side, &b.bottom]).flatten().map(String::as_str).collect();
    needed.extend(["block/water_still", "block/lava_still", "environment/clouds", "environment/sun"]);
    needed.sort_unstable();
    needed.dedup();
    let decoded: Vec<(String, Arc<Texture>, Option<connected::Connected>)> = needed
        .into_par_iter()
        .filter_map(|key| {
            let (k, n) = texture_files.get(key)?;
            let bytes = archives[*k].0.read(n)?.ok()?;
            let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            // A texture of Fusion's (connected, continuous or random) is a
            // sheet of tiles: the block shows one. Animated textures are a
            // strip of square frames.
            let meta = archives[*k].0.read(&format!("{n}.mcmeta")).and_then(Result::ok).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let (x, y, tw, th) = meta.as_ref().and_then(|m| fusion_tile(m, w, h)).unwrap_or((0, 0, w, w.min(h)));
            // Fusion's sheet, for the views that join its tiles.
            let sheet_joined = meta.as_ref().filter(|m| m.get("fusion").is_some()).and_then(|m| {
                let sheet = Texture { width: w, height: h, rgba: img.as_raw().clone() };
                connected::fusion(m, &sheet, n)
            });
            let frame = image::imageops::crop_imm(&img, x, y, tw, th).to_image();
            Some((key.to_owned(), Arc::new(Texture { width: tw, height: th, rgba: frame.into_raw() }), sheet_joined))
        })
        .collect();
    let mut joined: connected::Table = HashMap::new();
    let mut textures: HashMap<String, Arc<Texture>> = HashMap::with_capacity(decoded.len());
    for (key, tex, c) in decoded {
        if let Some(c) = c {
            joined.insert(key.clone(), Arc::new(c));
        }
        textures.insert(key, tex);
    }
    // OptiFine's connected textures (read by Continuity too), the packs'
    // in order, the last word kept: by texture, or by block for its faces'
    // textures.
    for (a, _) in &archives {
        for rule in connected::read_optifine(a) {
            let c = Arc::new(rule.connected);
            for t in &rule.tiles {
                joined.insert(t.clone(), c.clone());
            }
            for id in &rule.blocks {
                if let Ok(i) = blocks.binary_search_by(|b| b.id.as_str().cmp(id)) {
                    let b = &blocks[i];
                    let faces = [(&b.top, connected::UP), (&b.side, connected::SIDES), (&b.bottom, connected::DOWN)];
                    for (t, face) in faces {
                        if let Some(t) = t
                            && c.faces & face != 0
                        {
                            joined.insert(t.clone(), c.clone());
                        }
                    }
                }
            }
        }
    }
    for b in &mut blocks {
        // A list knows a mod's plants better than their names do, and its
        // blocks' halves.
        if let Some(def) = definitions.get(&b.id) {
            if def.vegetation {
                b.category = Category::Plants;
            }
            if def.property("half").is_some_and(|p| p.values.iter().any(|v| v == "lower") && p.values.iter().any(|v| v == "upper")) {
                b.tall = true;
            }
        }
        let color_of = |t: &Option<String>, tint: Tint| t.as_ref().and_then(|k| textures.get(k)).map(|tex| tex.mean(tint));
        // A block without texture takes its colour on the game's maps, when
        // a list gives it.
        let fallback = definitions.get(&b.id).and_then(|d| d.map_color).unwrap_or(b.color);
        b.color = color_of(&b.top, b.top_tint).unwrap_or(fallback);
        b.side_color = color_of(&b.side, b.side_tint).unwrap_or(fallback);
    }

    // The block tags, every archive's and folder's files of a tag merged
    // (a file that says `replace` drops the ones before it).
    let read_file = |k: usize, n: &str| -> Option<Value> { json(&archives[k].0, n) };
    let mut tags: HashMap<String, Vec<Value>> = HashMap::new();
    for (tag, files) in &tag_files {
        let entry = tags.entry(tag.clone()).or_default();
        for (k, n) in files {
            if let Some(v) = read_file(*k, n) {
                if v.get("replace").and_then(Value::as_bool) == Some(true) {
                    entry.clear();
                }
                entry.push(v);
            }
        }
    }
    for (_, files) in &folders {
        for (n, bytes) in files {
            if let Some(tag) = block_tag_key(n)
                && let Ok(v) = serde_json::from_slice::<Value>(bytes)
            {
                tags.entry(tag).or_default().push(v);
            }
        }
    }
    // A mod's plants that neither their names nor their models tell
    // (duckweed, clover, leaf litter, ivy, a beard of moss): the game's tags
    // of plants they are in.
    let plants = tag_members(&tags, &PLANT_TAGS);
    let pots = tag_members(&tags, &["minecraft:flower_pots"]);
    for b in &mut blocks {
        let n = b.id.rsplit(':').next().unwrap_or(&b.id);
        if namespace(&b.id) != "minecraft" && b.category != Category::Plants && !b.full_cube && plants.contains(&b.id) && !pots.contains(&b.id) && !n.starts_with("potted_") && !["ladder", "scaffolding", "chain", "rope", "door", "trapdoor"].iter().any(|w| n.split('_').any(|p| p == *w)) {
            b.category = Category::Plants;
        }
    }

    // The biomes, the game's first, then each mod's and data pack's, the
    // last file of an id winning.
    // The colour maps: the game's, then each resource pack's (or mod's) in
    // order, the last word kept; packs left as folders too.
    let mut rules = true_color::Rules::default();
    for (k, (a, _)) in archives.iter().enumerate() {
        let name = if k == 0 { String::new() } else { mod_infos.get(k - 1).map_or_else(String::new, |m| m.name.clone()) };
        rules.read_archive(a, &name);
    }
    for m in mods.iter().filter(|m| m.is_dir() && m.join("assets").is_dir()) {
        rules.read_files(&color_files(m), &m.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    }
    let source_of = |k: usize| if k == 0 { MINECRAFT.to_owned() } else { mod_infos.get(k - 1).map_or_else(String::new, |m| m.name.clone()) };
    let mut biomes: Vec<(usize, BiomeInfo)> = Vec::new();
    let mut add_biome = |order: usize, id: String, source: String, v: Value| {
        let info = biome_info(&id, &source, &v, biome_names.get(&id), &rules);
        match biomes.iter().position(|(_, b)| b.id == id) {
            Some(i) => biomes[i] = (order, info),
            None => biomes.push((order, info)),
        }
    };
    for (id, k, n) in &biome_files {
        if let Some(v) = read_file(*k, n) {
            add_biome(*k, id.clone(), source_of(*k), v);
        }
    }
    for (i, (dir, files)) in folders.iter().enumerate() {
        let source = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        for (n, bytes) in files {
            if let Some(id) = biome_key(n)
                && let Ok(v) = serde_json::from_slice::<Value>(bytes)
            {
                add_biome(archives.len() + i, id, source.clone(), v);
            }
        }
    }
    biomes.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let biomes: Vec<BiomeInfo> = biomes.into_iter().map(|(_, b)| b).collect();
    for (dir, _) in &folders {
        mod_infos.push(ModInfo { name: dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), path: dir.clone(), kind: SourceKind::Pack, ..Default::default() });
    }
    // The mods' stonecutter recipes: which shapes are cut from which block.
    let mut stonecutting: Vec<(String, String)> = rayon::iter::ParallelIterator::filter_map(rayon::iter::IntoParallelRefIterator::par_iter(&recipe_files), |(k, n)| stonecutting(&archives[*k].0.read(n)?.ok()?)).collect();
    for (_, files) in &folders {
        stonecutting.extend(files.iter().filter(|(n, _)| is_recipe(n)).filter_map(|(_, bytes)| self::stonecutting(bytes)));
    }
    Ok(Catalogue {
        stonecutting,
        state_properties,
        version: version.to_owned(),
        source: path.to_owned(),
        blocks,
        textures,
        mods: mod_infos,
        skipped,
        definitions,
        registered,
        biomes,
        colors: Arc::new(rules),
        connected: Arc::new(joined),
    })
}

/// The blocks of some tags, the tags they name (`#minecraft:small_flowers`)
/// followed.
fn tag_members(tags: &HashMap<String, Vec<Value>>, roots: &[&str]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut todo: Vec<String> = roots.iter().map(|s| (*s).to_owned()).collect();
    while let Some(tag) = todo.pop() {
        if !seen.insert(tag.clone()) {
            continue;
        }
        for file in tags.get(&tag).into_iter().flatten() {
            for v in file.get("values").and_then(Value::as_array).into_iter().flatten() {
                let Some(entry) = v.as_str().or_else(|| v.get("id").and_then(Value::as_str)) else { continue };
                let full = |s: &str| if s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") };
                match entry.strip_prefix('#') {
                    Some(t) => todo.push(full(t)),
                    None => {
                        out.insert(full(entry));
                    }
                }
            }
        }
    }
    out
}

/// The newest jar found on this computer, loaded.
pub fn load_newest() -> Option<Result<Catalogue, AssetError>> {
    let jar = find_jars().into_iter().next()?;
    Some(load(&jar.path, &jar.version))
}

// --------------------------------------------------------------- registry --

use massif_core::registry::{BiomeDef, BlockId, Registry, Stratum, TerrainDef, TerrainId};

/// Give every block and terrain of a registry its real colour: the mean of
/// its top texture, tinted as in the game; a terrain takes the mix of its
/// surface layer.
pub fn apply_colors(reg: &mut Registry, cat: &Catalogue) {
    for b in reg.blocks.iter_mut().skip(1) {
        if let Some(real) = cat.block(&b.state) {
            b.color = real.color;
        }
    }
    // The biomes the game or a mod read defines: the tints of their grass
    // and leaves; a mod's, its name, its group and its colour (a biome a
    // profile named before its mod was read had its id's).
    for b in reg.biomes.iter_mut().skip(1) {
        if let Some(real) = cat.biome(&b.id) {
            b.grass = Some(real.grass);
            b.foliage = Some(real.foliage);
            if real.source != MINECRAFT {
                let def = real.def();
                b.name = def.name;
                b.group = def.group;
                b.color = def.color;
            }
        }
    }
    let blocks = reg.blocks.clone();
    for t in reg.terrains.iter_mut() {
        let Some(top) = t.strata.first() else { continue };
        let total: u32 = top.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
        let mut c = [0u32; 3];
        for &(id, w) in &top.blocks {
            let bc = blocks.get(id.index()).map_or([128; 3], |b| b.color);
            for k in 0..3 {
                c[k] += bc[k] as u32 * w as u32;
            }
        }
        t.color = c.map(|v| (v / total) as u8);
    }
}

/// The group of the terrains made of one real block.
pub const BLOCK_GROUP: &str = "Blocks";

/// A terrain made of one real block, added to the registry the first time:
/// a few blocks deep (grass, podzol and mycelium over dirt, as in the game),
/// natural rock below.
pub fn block_terrain(reg: &mut Registry, block: &Block) -> TerrainId {
    if let Some(i) = reg
        .terrains
        .iter()
        .position(|t| t.group == BLOCK_GROUP && t.strata.first().and_then(|s| s.blocks.first()).is_some_and(|b| reg.blocks[b.0.index()].name() == block.id))
    {
        return TerrainId(i as u16);
    }
    let id: BlockId = reg.intern_block(&block.id, block.color);
    let on_dirt = ["minecraft:grass_block", "minecraft:podzol", "minecraft:mycelium", "minecraft:moss_block", "minecraft:pale_moss_block"].contains(&block.id.as_str());
    let strata = if on_dirt {
        let dirt = reg.intern_block("minecraft:dirt", [134, 96, 67]);
        vec![
            Stratum {
                blocks: vec![(id, 1)],
                depth: (1, 1),
                mix: Default::default(),
            },
            Stratum {
                blocks: vec![(dirt, 1)],
                depth: (3, 4),
                mix: Default::default(),
            },
        ]
    } else {
        vec![Stratum {
            blocks: vec![(id, 1)],
            depth: (3, 5),
            mix: Default::default(),
        }]
    };
    // A mod's block named as a terrain already is (`Granite`) takes its
    // mod's name after its own, so that a terrain is found by its name.
    let mut name = block.name.clone();
    let ns = namespace(&block.id);
    if ns != "minecraft" && reg.terrains.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
        name = format!("{name} ({})", display_name(ns));
    }
    // The biome its mod's profile gives it, for Auto biome and the export.
    let biome_hint = massif_core::mod_profiles::biome_of_block(&block.id).map(|b| reg.intern_biome(BiomeDef::of_id(b)));
    reg.terrains.push(TerrainDef {
        name,
        group: BLOCK_GROUP.to_owned(),
        color: block.color,
        strata,
        submerged_top: None,
        biome_hint,
    });
    TerrainId((reg.terrains.len() - 1) as u16)
}

/// A terrain of one block state typed by hand (any mod's block, with its
/// properties), added the first time, a few blocks deep over the natural
/// rock, named `name`.
pub fn state_terrain(reg: &mut Registry, state: &str, name: &str, color: [u8; 3]) -> TerrainId {
    if let Some(i) = reg.terrains.iter().position(|t| t.group == BLOCK_GROUP && t.strata.len() == 1 && t.strata[0].blocks.len() == 1 && reg.blocks[t.strata[0].blocks[0].0.index()].state == state) {
        return TerrainId(i as u16);
    }
    let id = reg.intern_block(state, color);
    let mut name = name.to_owned();
    if reg.terrains.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
        name = format!("{name} ({state})");
    }
    reg.terrains.push(TerrainDef {
        name,
        group: BLOCK_GROUP.to_owned(),
        color,
        strata: vec![Stratum {
            blocks: vec![(id, 1)],
            depth: (3, 5),
            mix: Default::default(),
        }],
        submerged_top: None,
        biome_hint: None,
    });
    TerrainId((reg.terrains.len() - 1) as u16)
}

/// The block shown for a terrain: the main block of its surface layer.
pub fn terrain_block<'a>(reg: &Registry, cat: &'a Catalogue, terrain: TerrainId) -> Option<&'a Block> {
    let top = reg.terrain(terrain).strata.first()?;
    let (id, _) = top.blocks.iter().max_by_key(|b| b.1)?;
    cat.block(&reg.block(*id).state)
}

// ------------------------------------------------------------------ icons --

/// A block drawn as a small isometric cube with its real textures: the top
/// lit, the two sides in shade. `size` pixels square, RGBA, transparent
/// around the cube.
pub fn cube_icon(cat: &Catalogue, block: &Block, size: usize) -> Vec<u8> {
    let s = size as f32;
    let tex = |key: &Option<String>, tint: Tint| key.as_ref().and_then(|k| cat.texture(k)).map(|t| t.tinted(tint));
    let top = tex(&block.top, block.top_tint);
    let side = tex(&block.side, block.side_tint);
    let mut out = vec![0u8; size * size * 4];
    // Corners of the cube's outline.
    let (t, r, b, l) = ((s / 2.0, 0.0), (s, s / 4.0), (s / 2.0, s / 2.0), (0.0, s / 4.0));
    let sample = |tx: &Option<Texture>, u: f32, v: f32, fallback: [u8; 3]| -> [u8; 4] {
        match tx {
            Some(t) => {
                let x = ((u.clamp(0.0, 0.999)) * t.width as f32) as usize;
                let y = ((v.clamp(0.0, 0.999)) * t.height as f32) as usize;
                let i = (y * t.width as usize + x) * 4;
                [t.rgba[i], t.rgba[i + 1], t.rgba[i + 2], t.rgba[i + 3]]
            }
            None => [fallback[0], fallback[1], fallback[2], 255],
        }
    };
    // Solve p = o + u·a + v·b for (u, v).
    let solve = |p: (f32, f32), o: (f32, f32), a: (f32, f32), bb: (f32, f32)| {
        let (px, py) = (p.0 - o.0, p.1 - o.1);
        let det = a.0 * bb.1 - a.1 * bb.0;
        ((px * bb.1 - py * bb.0) / det, (a.0 * py - a.1 * px) / det)
    };
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = |u: f32, v: f32| (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v);
            let (u, v) = solve(p, t, (r.0 - t.0, r.1 - t.1), (l.0 - t.0, l.1 - t.1));
            let (c, shade) = if inside(u, v) {
                (sample(&top, u, v, block.color), 1.0)
            } else {
                let (u, v) = solve(p, l, (b.0 - l.0, b.1 - l.1), (0.0, s / 2.0));
                if inside(u, v) {
                    (sample(&side, u, v, block.side_color), 0.78)
                } else {
                    let (u, v) = solve(p, b, (r.0 - b.0, r.1 - b.1), (0.0, s / 2.0));
                    if inside(u, v) {
                        (sample(&side, u, v, block.side_color), 0.6)
                    } else {
                        continue;
                    }
                }
            };
            let i = (y * size + x) * 4;
            for k in 0..3 {
                out[i + k] = (c[k] as f32 * shade) as u8;
            }
            out[i + 3] = c[3];
        }
    }
    out
}

/// A flat picture of a block that is not a cube (flowers, torches…): its
/// side texture, as the game shows it in the inventory.
pub fn flat_icon(cat: &Catalogue, block: &Block, size: usize) -> Vec<u8> {
    let tex = block
        .side
        .as_ref()
        .or(block.top.as_ref())
        .and_then(|k| cat.texture(k))
        .map(|t| t.tinted(block.side_tint));
    let mut out = vec![0u8; size * size * 4];
    if let Some(t) = tex {
        for y in 0..size {
            for x in 0..size {
                let sx = x * t.width as usize / size;
                let sy = y * t.height as usize / size;
                let si = (sy * t.width as usize + sx) * 4;
                out[(y * size + x) * 4..][..4].copy_from_slice(&t.rgba[si..si + 4]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// A mod chosen in a CurseForge instance on any drive: the game it
    /// plays on is found beside it, under the launcher's Install/versions,
    /// newest first; snapshots and other folders are left.
    #[test]
    fn the_game_is_found_beside_a_mods_jar() {
        let root = std::env::temp_dir().join(format!("massif-jars-near-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mc = root.join("curseforge/minecraft");
        for v in ["1.20.1", "1.21.1", "24w14a"] {
            std::fs::create_dir_all(mc.join("Install/versions").join(v)).unwrap();
            std::fs::write(mc.join("Install/versions").join(v).join(format!("{v}.jar")), b"").unwrap();
        }
        let mods = mc.join("Instances/Conquest/mods");
        std::fs::create_dir_all(&mods).unwrap();
        let found: Vec<String> = super::jars_near(&mods.join("conquest.jar")).into_iter().map(|j| j.version).collect();
        assert_eq!(found, ["1.21.1", "1.20.1"]);
        assert!(super::jars_near(&root.join("elsewhere/mod.jar")).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The game's jar of 1.20.1 as each launcher keeps it, found from an
    /// instance's mods folder, as on a Windows PC's drives: the official
    /// launcher's `versions`, CurseForge on another drive (its Fabric
    /// loader's empty jar left out), the Modrinth App under a loader's
    /// name (its version read from the jar), Prism's and GDLauncher's
    /// libraries, ATLauncher's (its srg jar left out).
    #[test]
    fn every_launcher_keeps_its_game_where_it_is_found() {
        use crate::zip::tests_support::tiny_zip;
        let root = std::env::temp_dir().join(format!("massif-launchers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = tiny_zip(&[("version.json", br#"{"id":"1.20.1","world_version":3465}"#, true), ("assets/minecraft/blockstates/stone.json", b"{}", true)]);
        let put = |path: &str, bytes: &[u8]| {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        };
        let layouts: [(&str, &[(&str, bool)]); 6] = [
            ("AppData/Roaming/.minecraft/mods", &[("AppData/Roaming/.minecraft/versions/1.20.1/1.20.1.jar", true), ("AppData/Roaming/.minecraft/versions/fabric-loader-0.16.10-1.20.1/fabric-loader-0.16.10-1.20.1.jar", false)]),
            ("D/Games/CurseForge/minecraft/Instances/ArdaCraft/mods", &[("D/Games/CurseForge/minecraft/Install/versions/1.20.1/1.20.1.jar", true), ("D/Games/CurseForge/minecraft/Install/versions/fabric-loader-0.16.10-1.20.1/fabric-loader-0.16.10-1.20.1.jar", false)]),
            ("AppData/Roaming/ModrinthApp/profiles/ArdaCraft 2.7.4/mods", &[("AppData/Roaming/ModrinthApp/meta/versions/1.20.1-fabric-0.16.10/1.20.1-fabric-0.16.10.jar", true)]),
            ("AppData/Roaming/PrismLauncher/instances/ArdaCraft/.minecraft/mods", &[("AppData/Roaming/PrismLauncher/libraries/com/mojang/minecraft/1.20.1/minecraft-1.20.1-client.jar", true)]),
            ("AppData/Roaming/ATLauncher/instances/ArdaCraft/mods", &[("AppData/Roaming/ATLauncher/libraries/net/minecraft/client/1.20.1/client-1.20.1-official.jar", true), ("AppData/Roaming/ATLauncher/libraries/net/minecraft/client/1.20.1/client-1.20.1-srg.jar", false)]),
            ("AppData/Roaming/gdlauncher_carbon/data/instances/ArdaCraft/instance/mods", &[("AppData/Roaming/gdlauncher_carbon/data/libraries/com/mojang/minecraft/1.20.1/minecraft-1.20.1-client.jar", true)]),
        ];
        for (mods, files) in layouts {
            let empty = tiny_zip(&[("META-INF/MANIFEST.MF", b"", true)]);
            for (file, real) in files {
                put(file, if *real { &game } else { &empty });
            }
            std::fs::create_dir_all(root.join(mods)).unwrap();
            let found = super::jars_near(&root.join(mods).join("arda.jar"));
            let want = root.join(files[0].0);
            assert_eq!(found.iter().map(|j| (j.version.as_str(), j.path.clone())).collect::<Vec<_>>(), [("1.20.1", want)], "{mods}");
            // Its launcher's folder alone gives it too.
            let launcher = files[0].0.split("/versions/").next().unwrap().split("/libraries/").next().unwrap();
            let launcher = launcher.strip_suffix("/Install").or_else(|| launcher.strip_suffix("/meta")).or_else(|| launcher.strip_suffix("/data")).unwrap_or(launcher);
            assert!(super::jars_in(&root.join(launcher)).iter().any(|j| j.version == "1.20.1"), "{launcher}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;

    /// Mods' names from their jars' file names, as modpacks of every era
    /// write them: Forge 1.12.2's, Fabric's, NeoForge's.
    #[test]
    fn jar_names_cleaned() {
        for (jar, name) in [
            ("ActuallyAdditions-1.12.2-r152.jar", "Actually Additions"),
            ("AppliedIntegrations-1.12.2-e2884e6f.jar", "Applied Integrations"),
            ("Botania r1.10-364.4.jar", "Botania"),
            ("Ceramics-1.12-1.3.7b.jar", "Ceramics"),
            ("ChanceCubes-1.12.2-5.0.2.385.jar", "Chance Cubes"),
            ("ChickenChunks-1.12.2-2.4.2.74-universal.jar", "Chicken Chunks"),
            ("Chisel-MC1.12.2-1.0.2.45.jar", "Chisel"),
            ("ColossalChests-1.12.2-1.7.3.jar", "Colossal Chests"),
            ("CookingForBlockheads_1.12.2-6.5.0.jar", "Cooking For Blockheads"),
            ("DarkUtils-1.12.2-1.8.230.jar", "Dark Utils"),
            ("Decocraft-2.6.3.7_1.12.2.jar", "Decocraft"),
            ("Draconic-Evolution-1.12.2-2.3.28.354-universal.jar", "Draconic Evolution"),
            ("EnderIO-1.12.2-5.3.70.jar", "Ender IO"),
            ("EquivalentEnergistics-1.12.2-PE_1.12.2-AE2_RV6-0.8.3.jar", "Equivalent Energistics"),
            ("ExtraBitManipulation-1.12.2-3.4.1.jar", "Extra Bit Manipulation"),
            ("ForgeMultipart-1.12.2-2.6.2.83-universal.jar", "Forge Multipart"),
            ("Growable Cells-1.12.2-3.1.0.jar", "Growable Cells"),
            ("BiomesOPlenty-1.12.2-7.0.1.2444-universal.jar", "Biomes O Plenty"),
            ("[1.12.2]bspkrsCore-universal-7.6.jar", "bspkrs Core"),
            ("ConquestReforged-fabric-1.20.1-1.4.1.4.jar", "Conquest Reforged"),
            ("sodium-fabric-0.5.13+mc1.20.1.jar", "Sodium"),
            ("create-1.20.1-0.5.1.f.jar", "Create"),
            ("jei-1.20.1-forge-15.2.0.27.jar", "Jei"),
            ("fabric-api-0.92.2+1.20.1.jar", "Fabric Api"),
            ("geckolib-neoforge-1.21.1-4.7.jar", "Geckolib"),
            ("mekanism-1.21.1-10.7.0.55.jar", "Mekanism"),
            ("Xaeros_Minimap_24.0.0_Forge_1.20.jar", "Xaeros Minimap"),
            ("supplementaries-1.20-2.8.17.jar", "Supplementaries"),
            ("macawsbridges-3.0.0-1.20.1-forge.jar", "Macawsbridges"),
            ("1.20.1-something.jar", "1.20.1-something"),
        ] {
            assert_eq!(jar_mod_name(jar), name, "{jar}");
        }
        assert_eq!(plain_mod_name("\u{a7}6Chisel"), "Chisel", "colour codes out");
        assert_eq!(plain_mod_name("BiomesOPlenty"), "Biomes O Plenty");
        assert_eq!(plain_mod_name("Biomes O' Plenty"), "Biomes O' Plenty", "written by hand: kept");
    }

    /// What a jar says of itself, whatever its loader.
    #[test]
    fn mods_metadata_read() {
        use crate::zip::tests_support::tiny_zip;
        let meta = |files: &[(&str, &[u8], bool)]| mod_meta(&zip::Archive::from_bytes(tiny_zip(files)).unwrap());
        let forge = meta(&[("META-INF/mods.toml", b"modLoader=\"javafml\"\n[[mods]]\nmodId=\"supplementaries\"\nversion=\"${file.jarVersion}\"\ndisplayName=\"Supplementaries\"\n", true)]);
        assert_eq!(forge, ModMeta { name: Some("Supplementaries".into()), id: Some("supplementaries".into()), loader: Some("Forge") });
        let neo = meta(&[("META-INF/neoforge.mods.toml", b"[[mods]]\nmodId=\"mekanism\"\ndisplayName=\"${mod_name}\"\n", true)]);
        assert_eq!(neo, ModMeta { name: None, id: Some("mekanism".into()), loader: Some("NeoForge") }, "placeholders left out");
        let fabric = meta(&[("fabric.mod.json", br#"{"id":"conquest","name":"ConquestReforged Fabric"}"#, true)]);
        assert_eq!((fabric.name.as_deref(), fabric.id.as_deref(), fabric.loader), (Some("ConquestReforged Fabric"), Some("conquest"), Some("Fabric")));
        let quilt = meta(&[("quilt.mod.json", br#"{"quilt_loader":{"id":"qbits","metadata":{"name":"Quilted Bits"}}}"#, false)]);
        assert_eq!((quilt.name.as_deref(), quilt.id.as_deref(), quilt.loader), (Some("Quilted Bits"), Some("qbits"), Some("Quilt")));
        // Forge 1.12.2's mcmod.info: a list, a modList, or not quite JSON.
        let old = meta(&[("mcmod.info", br#"[{"modid":"actuallyadditions","name":"Actually Additions","version":"r152"}]"#, true)]);
        assert_eq!((old.name.as_deref(), old.id.as_deref(), old.loader), (Some("Actually Additions"), Some("actuallyadditions"), Some("Forge")));
        let v2 = meta(&[("mcmod.info", br#"{"modListVersion":2,"modList":[{"modid":"chisel","name":"Chisel"}]}"#, true)]);
        assert_eq!((v2.name.as_deref(), v2.id.as_deref()), (Some("Chisel"), Some("chisel")));
        let loose = meta(&[("mcmod.info", b"[{\n \"modid\": \"botania\",\n \"name\": \"Botania\",\n \"description\": \"two\nlines\"\n}]", true)]);
        assert_eq!((loose.name.as_deref(), loose.id.as_deref()), (Some("Botania"), Some("botania")));
        assert_eq!(meta(&[("assets/x/blockstates/a.json", b"{}", false)]), ModMeta::default());
        assert_eq!(display_mod_name(&old, std::path::Path::new("ActuallyAdditions-1.12.2-r152.jar")), "Actually Additions");
        assert_eq!(display_mod_name(&ModMeta::default(), std::path::Path::new("mods/ChanceCubes-1.12.2-5.0.2.385.jar")), "Chance Cubes");
    }


    /// The games installed here say which data version they write (only
    /// checked where a game is installed).
    #[test]
    fn installed_games_tell_their_data_version() {
        for jar in find_jars() {
            let version = data_version(&jar.path);
            eprintln!("{}: data version {version:?}", jar.version);
            if release_numbers(&jar.version).is_some_and(|n| n >= vec![1, 14]) {
                assert!(version.is_some_and(|v| v >= 1952), "{}: {version:?}", jar.version);
            }
        }
    }

    /// Leaves take the colours the game gives them: the biome's, a fixed
    /// one, or none at all (cherry and pale oak, whose models are the
    /// tinted leaves' all the same).
    #[test]
    fn leaves_are_tinted_as_in_the_game() {
        assert_eq!(tint_of("minecraft:oak_leaves"), Tint::Foliage);
        assert_eq!(tint_of("minecraft:mangrove_leaves"), Tint::Foliage);
        assert!(matches!(tint_of("minecraft:spruce_leaves"), Tint::Fixed(_)));
        assert_eq!(tint_of("minecraft:cherry_leaves"), Tint::None);
        assert_eq!(tint_of("minecraft:pale_oak_leaves"), Tint::None);
    }

    #[test]
    fn versions_order_like_the_game() {
        let mut v = ["1.20.4", "26.3", "1.21.11", "1.21.5", "26.1.2"].map(|s| release_numbers(s).unwrap());
        v.sort();
        assert_eq!(v.last().unwrap(), &vec![26, 3]);
        assert_eq!(v[0], vec![1, 20, 4]);
        assert!(release_numbers("25w16a").is_none());
        assert!(release_numbers("neoforge-21.5.38-beta").is_none());
        assert_eq!(display_name("minecraft:grass_block"), "Grass Block");
    }

    #[test]
    fn states_typed_by_hand() {
        assert_eq!(parse_state("regions_unexplored:redwood_log[axis=x]").as_deref(), Ok("regions_unexplored:redwood_log[axis=x]"));
        assert_eq!(parse_state("  Stone ").as_deref(), Ok("minecraft:stone"));
        assert_eq!(parse_state("Mod:Tall Grass[ Half = Lower , ]").as_deref(), Ok("mod:tall_grass[half=lower]"));
        assert_eq!(parse_state("biomeswevegone:block/odd.name-2[]").as_deref(), Ok("biomeswevegone:block/odd.name-2"));
        assert_eq!(parse_state(""), Err(StateSyntax::Empty));
        assert!(matches!(parse_state("mod:a:b"), Err(StateSyntax::BadId(_))));
        assert!(matches!(parse_state("mod:é"), Err(StateSyntax::BadId(_))));
        assert!(matches!(parse_state("mod:x[axis]"), Err(StateSyntax::BadProperty(_))));
        assert!(matches!(parse_state("mod:x[axis=y"), Err(StateSyntax::BadProperty(_))));
        // A terrain of it, once, by its state.
        let mut reg = Registry::vanilla();
        let t = state_terrain(&mut reg, "regions_unexplored:redwood_log[axis=x]", "Redwood Log", [120, 60, 40]);
        assert_eq!(state_terrain(&mut reg, "regions_unexplored:redwood_log[axis=x]", "Redwood Log", [120, 60, 40]), t);
        assert_eq!(reg.block(reg.terrain(t).strata[0].blocks[0].0).state, "regions_unexplored:redwood_log[axis=x]");
        let y = state_terrain(&mut reg, "regions_unexplored:redwood_log[axis=y]", "Redwood Log", [120, 60, 40]);
        assert_ne!(y, t);
        assert_eq!(reg.terrain(y).name, "Redwood Log (regions_unexplored:redwood_log[axis=y])", "named apart");
    }

    #[test]
    fn categories() {
        assert_eq!(Category::of("minecraft:grass_block"), Category::Natural);
        assert_eq!(Category::of("minecraft:stone"), Category::Stone);
        assert_eq!(Category::of("minecraft:stone_bricks"), Category::Building);
        assert_eq!(Category::of("minecraft:diamond_ore"), Category::Ores);
        assert_eq!(Category::of("minecraft:oak_planks"), Category::Wood);
        assert_eq!(Category::of("minecraft:oak_leaves"), Category::Plants);
        assert_eq!(Category::of("minecraft:red_wool"), Category::Colored);
        assert_eq!(Category::of("minecraft:netherrack"), Category::Nether);
        assert_eq!(Category::of("minecraft:end_stone"), Category::End);
        // A mod's names that say nothing, by how the blocks are drawn.
        let models = |m: &[&str]| m.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Category::of_block("biomeswevegone:amaranth", &models(&["biomeswevegone:block/amaranth", "block/cross"]), [Some("biomeswevegone:block/amaranth"), None], false), Category::Plants);
        assert_eq!(Category::of_block("biomesoplenty:barley", &models(&["biomesoplenty:block/barley_bottom", "block/tinted_cross"]), [None, None], false), Category::Plants);
        assert_eq!(Category::of_block("mod:potted_amaranth", &models(&["block/flower_pot_cross", "block/cross"]), [None, None], false), Category::Other);
        assert_eq!(Category::of_block("biomeswevegone:aspen_fence", &models(&["biomeswevegone:block/aspen_fence_post"]), [Some("biomeswevegone:block/aspen/planks"), None], false), Category::Wood);
        assert_eq!(Category::of_block("biomeswevegone:aspen_hanging_sign", &[], [Some("biomeswevegone:block/aspen/stripped_log"), None], false), Category::Wood);
        assert_eq!(Category::of_block("mod:mystery", &[], [Some("mod:block/mystery_bricks"), None], true), Category::Building);
        assert_eq!(Category::of_block("mod:mystery", &[], [Some("mod:block/mystery"), None], true), Category::Other);
        assert_eq!(Category::of_block("minecraft:oak_planks", &[], [None, None], true), Category::Wood, "a name that says is kept");
        // A mod's ground is soil, whole; what is made of it is not.
        for ground in ["regions_unexplored:silt_grass_block", "regions_unexplored:peat_dirt", "regions_unexplored:silt_mud", "regions_unexplored:volcanic_ash", "regions_unexplored:cobalt_nylium", "biomesoplenty:white_sand", "regions_unexplored:peat_podzol"] {
            assert_eq!(Category::of_block(ground, &[], [None, None], true), Category::Natural, "{ground}");
        }
        assert_eq!(Category::of_block("regions_unexplored:silt_grass_block", &[], [None, None], false), Category::Plants, "not whole: by its name");
        assert_eq!(Category::of_block("biomesoplenty:white_sandstone", &[], [None, None], true), Category::Stone, "made of sand, not sand");
        assert_eq!(Category::of_block("mod:mud_bricks", &[], [None, None], true), Category::Building);
        assert_eq!(Category::of_block("minecraft:soul_sand", &[], [None, None], true), Category::Nether, "the game's keep their groups");
    }

    /// A tiny jar: a block state, its model chain and a texture.
    #[test]
    fn blocks_resolve_through_their_models() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 200, 200, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let files: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/grass_block.json", br#"{"variants":{"snowy=false":[{"model":"minecraft:block/grass_block"}]}}"#, true),
            ("assets/minecraft/models/block/block.json", br#"{"textures":{}}"#, true),
            (
                "assets/minecraft/models/block/grass_block.json",
                br##"{"parent":"block/block","textures":{"particle":"block/dirt","top":"block/grass_block_top","side":"block/dirt"},
                "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#top","tintindex":0},"north":{"texture":"#side"},"down":{"texture":"#side"}}}]}"##,
                true,
            ),
            ("assets/minecraft/textures/block/grass_block_top.png", &png, false),
        ];
        let zip = zip::tests_support::tiny_zip(&files);
        let dir = std::env::temp_dir().join(format!("massif-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.jar");
        std::fs::write(&path, zip).unwrap();
        let cat = load(&path, "1.21").unwrap();
        let grass = cat.block("minecraft:grass_block[snowy=false]").unwrap();
        assert_eq!(grass.name, "Grass Block");
        assert!(grass.full_cube);
        assert_eq!(grass.top.as_deref(), Some("block/grass_block_top"));
        assert_eq!(grass.side.as_deref(), Some("block/dirt"));
        assert_eq!(grass.top_tint, Tint::Grass);
        // Grey tinted by the plains grass colour.
        assert_eq!(grass.color, [0x91 * 200 / 255, 0xbd * 200 / 255, 0x59 * 200 / 255].map(|v| v as u8));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// What the game's jars named in MASSIF_DIGEST_JARS (`;` apart) read
    /// to: per jar, its blocks, those with a texture, those see-through,
    /// and a digest of every block's textures, tints, shape, category and
    /// colours, to compare two versions of the reader.
    #[test]
    fn jars_digest() {
        use std::hash::{Hash, Hasher};
        let Ok(jars) = std::env::var("MASSIF_DIGEST_JARS") else { return };
        for jar in jars.split(';').filter(|j| !j.is_empty()) {
            let cat = load(std::path::Path::new(jar), "digest").unwrap();
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for b in &cat.blocks {
                (&b.id, &b.top, &b.side, &b.bottom, format!("{:?}{:?}{:?}", b.top_tint, b.side_tint, b.category), b.full_cube, b.tall, b.color, b.side_color).hash(&mut h);
            }
            let textured = cat.blocks.iter().filter(|b| b.top.is_some() && cat.textures.contains_key(b.top.as_deref().unwrap())).count();
            let see_through = cat.blocks.iter().filter(|b| b.translucent).count();
            eprintln!("{jar}: {} blocks, {textured} with a top texture, {see_through} see-through, digest {:016x}", cat.blocks.len(), h.finish());
        }
    }

    /// Since Minecraft 26.1 a model's texture may be an object,
    /// `{"sprite": name, "force_translucent": true}` (glass, stained glass):
    /// read as the name, the block see-through; a plain name as before.
    #[test]
    fn textures_written_as_sprites_are_read() {
        let png = |c: [u8; 4]| {
            let mut out = Vec::new();
            image::RgbaImage::from_pixel(16, 16, image::Rgba(c)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        let (glass_png, stone_png) = (png([200, 220, 230, 90]), png([120, 120, 120, 255]));
        let files: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/glass.json", br#"{"variants":{"":{"model":"minecraft:block/glass"}}}"#, true),
            ("assets/minecraft/blockstates/stone.json", br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#, true),
            ("assets/minecraft/models/block/block.json", br#"{"textures":{}}"#, true),
            (
                "assets/minecraft/models/block/cube.json",
                br##"{"parent":"block/block","elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#up"},"down":{"texture":"#down"},"north":{"texture":"#north"}}}]}"##,
                true,
            ),
            ("assets/minecraft/models/block/cube_all.json", br##"{"parent":"block/cube","textures":{"particle":"#all","up":"#all","down":"#all","north":"#all"}}"##, true),
            // As 26.3 writes glass.
            ("assets/minecraft/models/block/glass.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":{"force_translucent":true,"sprite":"minecraft:block/glass"}}}"#, true),
            ("assets/minecraft/models/block/stone.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#, true),
            ("assets/minecraft/textures/block/glass.png", &glass_png, false),
            ("assets/minecraft/textures/block/stone.png", &stone_png, false),
        ];
        let zip = zip::tests_support::tiny_zip(&files);
        let dir = std::env::temp_dir().join(format!("massif-assets-sprites-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.jar");
        std::fs::write(&path, zip).unwrap();
        let cat = load(&path, "26.3").unwrap();
        let glass = cat.block("minecraft:glass").unwrap();
        assert_eq!(glass.top.as_deref(), Some("block/glass"));
        assert_eq!(glass.side.as_deref(), Some("block/glass"));
        assert!(cat.textures.contains_key("block/glass"), "its texture read");
        assert!(glass.full_cube && glass.translucent);
        assert_eq!(glass.color, [200, 220, 230], "its texture's colour, not grey");
        let stone = cat.block("minecraft:stone").unwrap();
        assert_eq!(stone.top.as_deref(), Some("block/stone"));
        assert!(!stone.translucent);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The biomes of the game, of a mod and of a data pack left as a folder:
    /// each with its name (its language file's, else its id's), its source,
    /// its grass and leaves (its own colours, else the game's colour maps at
    /// its temperature) and a colour on the map; a mod's plant that neither
    /// its name nor its model tells, by the game's tags of plants it is in.
    #[test]
    fn mods_define_biomes_and_plants() {
        let mut map = Vec::new();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([10, 200, 30, 255])).write_to(&mut std::io::Cursor::new(&mut map), image::ImageFormat::Png).unwrap();
        let game: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/stone.json", br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#, true),
            ("assets/minecraft/lang/en_us.json", br#"{"biome.minecraft.plains":"Plains","block.minecraft.stone":"Stone"}"#, true),
            ("assets/minecraft/textures/colormap/grass.png", &map, false),
            ("data/minecraft/worldgen/biome/plains.json", br#"{"temperature":0.8,"downfall":0.4,"effects":{"water_color":4159204}}"#, true),
            ("data/minecraft/tags/block/flowers.json", br##"{"values":["#minecraft:small_flowers"]}"##, true),
            ("data/minecraft/tags/block/small_flowers.json", br#"{"values":["minecraft:poppy"]}"#, true),
        ];
        let mod_files: Vec<(&str, &[u8], bool)> = vec![
            ("META-INF/mods.toml", b"modLoader=\"javafml\"\n[[mods]]\nmodId=\"testmod\"\ndisplayName=\"Test Mod\"\n", true),
            ("assets/testmod/lang/en_us.json", br#"{"biome.testmod.bayou":"Bayou of Tests"}"#, true),
            // A flat plant drawn with a model of its own, called a flower by
            // the game's tag; a pot of it is no plant.
            ("assets/testmod/blockstates/duckmat.json", br#"{"variants":{"":{"model":"testmod:block/duckmat"}}}"#, true),
            ("assets/testmod/models/block/duckmat.json", br##"{"textures":{"particle":"testmod:block/duckmat"},"elements":[{"from":[0,0,0],"to":[16,0.25,16],"faces":{"up":{"texture":"#particle"}}}]}"##, true),
            ("assets/testmod/blockstates/potted_duckmat.json", br#"{"variants":{"":{"model":"testmod:block/duckmat"}}}"#, true),
            ("data/minecraft/tags/block/small_flowers.json", br#"{"values":["testmod:duckmat",{"id":"testmod:potted_duckmat","required":false}]}"#, true),
            ("data/testmod/worldgen/biome/bayou.json", br#"{"temperature":0.9,"downfall":0.9,"effects":{"grass_color":1122867,"foliage_color":4478310,"water_color":30583}}"#, true),
            ("data/testmod/worldgen/biome/sea/murky_lagoon.json", br#"{"temperature":0.5,"downfall":0.5,"effects":{"water_color":30583,"grass_color_modifier":"swamp"}}"#, true),
        ];
        let dir = std::env::temp_dir().join(format!("massif-assets-biomes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Misty Pack/data/misty/worldgen/biome")).unwrap();
        std::fs::write(dir.join("Misty Pack/pack.mcmeta"), b"{}").unwrap();
        std::fs::write(dir.join("Misty Pack/data/misty/worldgen/biome/glade.json"), br#"{"temperature":0.2,"downfall":0.0,"effects":{"water_color":1}}"#).unwrap();
        let (game_jar, mod_jar) = (dir.join("1.21.jar"), dir.join("testmod.jar"));
        std::fs::write(&game_jar, zip::tests_support::tiny_zip(&game)).unwrap();
        std::fs::write(&mod_jar, zip::tests_support::tiny_zip(&mod_files)).unwrap();
        let cat = load_with_mods(&game_jar, "1.21", &[mod_jar.clone(), dir.join("Misty Pack")]).unwrap();
        let ids: Vec<&str> = cat.biomes.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["minecraft:plains", "testmod:bayou", "testmod:sea/murky_lagoon", "misty:glade"], "the game's, then the mod's by name, then the pack's");
        let plains = cat.biome("minecraft:plains").unwrap();
        assert_eq!((plains.source.as_str(), plains.name.as_str()), (MINECRAFT, "Plains"));
        assert_eq!(plains.grass, [10, 200, 30], "from the colour map");
        assert_eq!(plains.foliage, Tint::Foliage.color(), "no colour map: the plains'");
        let bayou = cat.biome("testmod:bayou").unwrap();
        assert_eq!((bayou.source.as_str(), bayou.name.as_str()), ("Test Mod", "Bayou of Tests"));
        assert_eq!((bayou.grass, bayou.foliage, bayou.water), ([0x11, 0x22, 0x33], [0x44, 0x55, 0x66], [0, 0x77, 0x77]));
        let lagoon = cat.biome("testmod:sea/murky_lagoon").unwrap();
        assert_eq!((lagoon.name.as_str(), lagoon.grass), ("Murky Lagoon", [0x6a, 0x70, 0x39]));
        assert!(lagoon.color[2] > lagoon.color[0], "a sea is drawn in its water's colour: {:?}", lagoon.color);
        assert_eq!(cat.biome("misty:glade").unwrap().source, "Misty Pack");
        assert!(cat.mods.iter().any(|m| m.name == "Misty Pack" && m.kind == SourceKind::Pack));
        let def = bayou.def();
        assert_eq!((def.group.as_str(), def.grass, def.foliage), ("Test Mod", Some(bayou.grass), Some(bayou.foliage)));
        assert_eq!(plains.def().group, "", "the game's have no group");
        assert_eq!(cat.block("testmod:duckmat").unwrap().category, Category::Plants);
        assert_ne!(cat.block("testmod:potted_duckmat").unwrap().category, Category::Plants);

        // In a map: the biomes it uses take their colours and tints once the
        // game is read.
        let mut reg = Registry::vanilla();
        let b = reg.intern_biome(BiomeDef::of_id("testmod:bayou"));
        apply_colors(&mut reg, &cat);
        assert_eq!(reg.biome(b), &def);
        let p = reg.biome_by_id("minecraft:plains").unwrap();
        assert_eq!((reg.biome(p).name.as_str(), reg.biome(p).grass), ("Plains", Some([10, 200, 30])));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A terrain made of a mod's block takes the biome the mod's profile
    /// gives that block (Regions Unexplored's peat mud: its bayou), named
    /// from its id before the mod is read.
    #[test]
    fn profiles_give_mod_terrains_their_biomes() {
        let mut reg = Registry::vanilla();
        let block = |id: &str| Block { id: id.into(), name: display_name(id), top: None, side: None, bottom: None, top_tint: Tint::None, side_tint: Tint::None, full_cube: true, translucent: false, tall: false, color: [80, 60, 50], side_color: [80, 60, 50], category: Category::Natural };
        let t = block_terrain(&mut reg, &block("regions_unexplored:peat_mud"));
        let b = reg.terrain(t).biome_hint.expect("a biome");
        assert_eq!((reg.biome(b).id.as_str(), reg.biome(b).name.as_str()), ("regions_unexplored:bayou", "Bayou"));
        let silt = block_terrain(&mut reg, &block("regions_unexplored:silt_dirt"));
        assert_eq!(reg.terrain(silt).biome_hint, None, "a block its profile names no biome for");
    }

    /// A mod beside the game, as Conquest Reforged is made: its block states
    /// and models under its own namespace, a model built on the game's
    /// `cube_all`, its textures, its name in `mods.toml`. Its blocks keep
    /// their ids and are drawn with their textures; a jar that is not one is
    /// left out, the game's blocks still read.
    #[test]
    fn mods_add_their_blocks() {
        let png = |rgb: [u8; 3]| {
            let mut out = Vec::new();
            let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        let (grey, red) = (png([120, 120, 120]), png([180, 40, 40]));
        let game: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/stone.json", br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#, true),
            ("assets/minecraft/models/block/cube.json", br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#up"},"north":{"texture":"#north"},"down":{"texture":"#down"}}}]}"##, true),
            ("assets/minecraft/models/block/cube_all.json", br##"{"parent":"block/cube","textures":{"particle":"#all","up":"#all","north":"#all","down":"#all"}}"##, true),
            ("assets/minecraft/models/block/stone.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#, true),
            ("assets/minecraft/textures/block/stone.png", &grey, false),
        ];
        let conquest: Vec<(&str, &[u8], bool)> = vec![
            ("META-INF/mods.toml", b"modLoader=\"javafml\"\n[[mods]]\nmodId=\"conquest\"\ndisplayName=\"Conquest Reforged\"\n", true),
            ("assets/conquest/blockstates/old_red_bricks.json", br#"{"variants":{"":{"model":"conquest:block/old_red_bricks"}}}"#, true),
            ("assets/conquest/models/block/old_red_bricks.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":"conquest:block/old_red_bricks"}}"#, true),
            ("assets/conquest/textures/block/old_red_bricks.png", &red, false),
            // A block of its own namespace whose model is missing still lists.
            ("assets/conquest/blockstates/thatch_slab.json", br#"{"variants":{"type=bottom":{"model":"conquest:block/thatch_slab"}}}"#, true),
        ];
        let dir = std::env::temp_dir().join(format!("massif-assets-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (game_jar, mod_jar, broken) = (dir.join("1.21.jar"), dir.join("conquest-reforged.jar"), dir.join("broken.jar"));
        std::fs::write(&game_jar, zip::tests_support::tiny_zip(&game)).unwrap();
        std::fs::write(&mod_jar, zip::tests_support::tiny_zip(&conquest)).unwrap();
        std::fs::write(&broken, b"not a jar").unwrap();
        let cat = load_with_mods(&game_jar, "1.21", &[mod_jar.clone(), broken.clone()]).unwrap();
        let bricks = cat.block("conquest:old_red_bricks").unwrap();
        assert_eq!(bricks.name, "Old Red Bricks");
        assert!(bricks.full_cube, "built on the game's cube_all");
        assert_eq!(bricks.top.as_deref(), Some("conquest:block/old_red_bricks"));
        assert_eq!(bricks.color, [180, 40, 40]);
        assert_eq!(bricks.category, Category::Building);
        assert!(cat.block("conquest:thatch_slab").is_some());
        assert_eq!(cat.block("minecraft:stone").unwrap().color, [120, 120, 120]);
        assert_eq!(cat.mods.len(), 1);
        assert_eq!(cat.mods[0].name, "Conquest Reforged");
        assert_eq!(cat.mods[0].namespaces, ["conquest"]);
        assert_eq!(cat.mods[0].blocks, 2);
        assert_eq!(cat.skipped.len(), 1);
        assert_eq!(cat.skipped[0].0, broken);
        assert_eq!(namespace("conquest:old_red_bricks"), "conquest");
        assert_eq!(namespace("stone"), "minecraft");
        // The game alone has none of them.
        assert!(load(&game_jar, "1.21").unwrap().block("conquest:old_red_bricks").is_none());
        // A resource pack after the mod gives its blocks their textures.
        let blue = png([40, 60, 200]);
        let pack: Vec<(&str, &[u8], bool)> = vec![
            ("pack.mcmeta", br#"{"pack":{"pack_format":15,"description":"Conquest textures"}}"#, true),
            ("assets/conquest/textures/block/old_red_bricks.png", &blue, false),
        ];
        let pack_zip = dir.join("conquest-textures.zip");
        std::fs::write(&pack_zip, zip::tests_support::tiny_zip(&pack)).unwrap();
        let packed = load_with_mods(&game_jar, "1.21", &[mod_jar.clone(), pack_zip.clone()]).unwrap();
        assert_eq!(packed.block("conquest:old_red_bricks").unwrap().color, [40, 60, 200]);
        assert_eq!(packed.mods[1].blocks, 0, "a resource pack adds textures, not blocks");
        // A terrain of a mod's block keeps its id, for the export.
        let mut reg = Registry::vanilla();
        let t = block_terrain(&mut reg, bricks);
        let top = reg.terrain(t).strata[0].blocks[0].0;
        assert_eq!(reg.block(top).state, "conquest:old_red_bricks");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// An instance whose KubeJS scripts make blocks: its `kubejs` folder
    /// is read as a mod beside its jars, and a texture of its made for
    /// Fusion (a sheet of 2 × 2 tiles, continuous) shows one tile.
    #[test]
    fn kubejs_blocks_and_fusion_textures_load() {
        let png = |w: u32, h: u32| {
            let mut out = Vec::new();
            // Each tile of 16 its own colour, so that the tile shown tells.
            let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x / 16 * 100) as u8, (y / 16 * 100) as u8, 50, 255]));
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        let stone = png(16, 16);
        let game: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/stone.json", br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#, true),
            ("assets/minecraft/models/block/cube.json", br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#up"},"north":{"texture":"#north"},"down":{"texture":"#down"}}}]}"##, true),
            ("assets/minecraft/models/block/cube_all.json", br##"{"parent":"block/cube","textures":{"particle":"#all","up":"#all","north":"#all","down":"#all"}}"##, true),
            ("assets/minecraft/models/block/stone.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#, true),
            ("assets/minecraft/textures/block/stone.png", &stone, false),
        ];
        let dir = std::env::temp_dir().join(format!("massif-assets-kubejs-{}", std::process::id()));
        let game_dir = dir.join("instance/minecraft");
        let assets = game_dir.join("kubejs/assets/kubejs");
        for sub in ["blockstates", "models/block", "textures/block/rock"] {
            std::fs::create_dir_all(assets.join(sub)).unwrap();
        }
        std::fs::create_dir_all(game_dir.join("mods")).unwrap();
        std::fs::write(assets.join("blockstates/pavement.json"), br#"{"variants":{"":{"model":"kubejs:block/pavement"}}}"#).unwrap();
        std::fs::write(assets.join("models/block/pavement.json"), br#"{"loader":"fusion:model","type":"base","parent":"minecraft:block/cube_all","textures":{"all":"kubejs:block/rock/pavement"}}"#).unwrap();
        std::fs::write(assets.join("textures/block/rock/pavement.png"), png(32, 32)).unwrap();
        std::fs::write(assets.join("textures/block/rock/pavement.png.mcmeta"), br#"{"fusion":{"type":"continuous","rows":2,"columns":2}}"#).unwrap();
        // A block a startup script makes, without files of its own: KubeJS
        // would write them, from its texture by default.
        std::fs::create_dir_all(game_dir.join("kubejs/startup_scripts")).unwrap();
        std::fs::write(game_dir.join("kubejs/startup_scripts/blocks.js"), "StartupEvents.registry('block', event => {\n  event.create('marble').displayName('Marble')\n})\n").unwrap();
        std::fs::write(assets.join("textures/block/marble.png"), png(16, 16)).unwrap();
        let game_jar = dir.join("1.21.jar");
        std::fs::write(&game_jar, zip::tests_support::tiny_zip(&game)).unwrap();

        let instance = instance::Instance::read(&dir.join("instance")).unwrap();
        assert_eq!(instance.mod_list.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["KubeJS"]);
        assert_eq!(instance.namespaces, ["kubejs"]);
        let cat = load_with_mods(&game_jar, "1.21", &instance.files()).unwrap();
        let pavement = cat.block("kubejs:pavement").unwrap();
        assert_eq!(pavement.top.as_deref(), Some("kubejs:block/rock/pavement"));
        let tile = cat.texture("kubejs:block/rock/pavement").unwrap();
        assert_eq!((tile.width, tile.height), (16, 16), "one tile of the sheet");
        assert_eq!(&tile.rgba[..4], &[0, 0, 50, 255], "the first");
        let marble = cat.block("kubejs:marble").expect("the script's block");
        assert_eq!(marble.top.as_deref(), Some("kubejs:block/marble"));
        assert_eq!(marble.name, "Marble");
        assert_eq!(cat.mods.len(), 1);
        assert_eq!((cat.mods[0].name.as_str(), cat.mods[0].kind, cat.mods[0].blocks), ("KubeJS", SourceKind::Mod, 2));
        assert!(cat.skipped.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fusion_sheets_show_one_tile() {
        let meta = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"continuous","rows":8,"columns":8}}"#), 1024, 1024), Some((0, 0, 128, 128)));
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"random","rows":5,"columns":6}}"#), 512, 384), Some((0, 0, 85, 76)));
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"connecting","layout":"pieced"}}"#), 80, 16), Some((0, 0, 16, 16)));
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"connecting","layout":"full"}}"#), 128, 96), Some((0, 0, 16, 16)));
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"connecting"}}"#), 128, 96), Some((0, 0, 16, 16)), "full by default");
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"connecting","layout":"overlay"}}"#), 96, 48), Some((16, 16, 16, 16)));
        // Not Fusion's, of a type it does not know, or too small to divide.
        assert_eq!(fusion_tile(&meta(r#"{"animation":{"frametime":2}}"#), 16, 64), None);
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"other"}}"#), 16, 16), None);
        assert_eq!(fusion_tile(&meta(r#"{"fusion":{"type":"continuous","rows":64,"columns":64}}"#), 16, 16), None);
    }

    /// Conquest Reforged as its jar is made, in small: a block whose states
    /// name a model per layer, models built on `block/block` naming the
    /// game's textures with an empty namespace (`:block/basalt_side`), a
    /// template of its models among the game's block states, a block file
    /// the game never registers, its creative tabs and their names, its
    /// `fabric.mod.json`; beside it its list for WorldPainter and the
    /// registry's list. Every real block reads with its textures, its
    /// name and its tab; the template and the stray file are no blocks.
    #[test]
    fn conquest_fixture_loads() {
        let png = |rgb: [u8; 3]| {
            let mut out = Vec::new();
            let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
            out
        };
        let (basalt_side, basalt_top, masonry) = (png([80, 80, 84]), png([60, 60, 64]), png([150, 140, 120]));
        let game: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/basalt.json", br#"{"variants":{"axis=y":{"model":"minecraft:block/basalt"}}}"#, true),
            ("assets/minecraft/models/block/block.json", br#"{"display":{}}"#, true),
            ("assets/minecraft/models/block/cube.json", br##"{"parent":"block/block","elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#up"},"north":{"texture":"#north"},"down":{"texture":"#down"}}}]}"##, true),
            ("assets/minecraft/models/block/cube_all.json", br##"{"parent":"block/cube","textures":{"particle":"#all","up":"#all","north":"#all","down":"#all"}}"##, true),
            ("assets/minecraft/models/block/basalt.json", br#"{"parent":"block/cube_all","textures":{"all":"block/basalt_side"}}"#, true),
            ("assets/minecraft/textures/block/basalt_side.png", &basalt_side, false),
            ("assets/minecraft/textures/block/basalt_top.png", &basalt_top, false),
        ];
        let conquest: Vec<(&str, &[u8], bool)> = vec![
            ("fabric.mod.json", br#"{"schemaVersion":1,"id":"conquest","name":"ConquestReforged Fabric","depends":{"fabricloader":">=0.14.10","minecraft":">=1.20.1"}}"#, true),
            ("assets/conquest/blockstates/basalt_pillar.json", br#"{"variants":{"layer=1":{"model":"conquest:block/basalt_pillar_2","uvlock":true},"layer=2":{"model":"conquest:block/basalt_pillar_4"}}}"#, true),
            ("assets/conquest/models/block/basalt_pillar_2.json", br##"{"parent":"block/block","textures":{"texture":":block/basalt_side","top":":block/basalt_top","particle":":block/basalt_side"},"elements":[{"from":[6,0,6],"to":[10,16,10],"faces":{"north":{"texture":"#texture"},"up":{"texture":"#top"}}}]}"##, true),
            ("assets/conquest/blockstates/worn_dark_lime_mortar_masonry.json", br#"{"variants":{"":{"model":"conquest:block/worn_dark_lime_mortar_masonry"}}}"#, true),
            ("assets/conquest/models/block/worn_dark_lime_mortar_masonry.json", br#"{"parent":"minecraft:block/cube_all","textures":{"all":"conquest:block/worn_dark_lime_mortar_masonry"}}"#, true),
            ("assets/conquest/textures/block/worn_dark_lime_mortar_masonry.png", &masonry, false),
            ("assets/conquest/blockstates/worn_dark_lime_mortar_masonry_slab.json", br#"{"variants":{"layers=1,type=bottom":{"model":"conquest:block/worn_dark_lime_mortar_masonry"}}}"#, true),
            // Never registered by the game: left in the jar.
            ("assets/conquest/blockstates/cheese_cut.json", br#"{"variants":{"":{"model":"conquest:block/cheese_cut"}}}"#, true),
            // A template of its models, among the game's states.
            ("assets/minecraft/blockstates/parent_cube.json", br#"{"variants":{"":{"model":"minecraft:block/cube_all"}}}"#, true),
            ("assets/conquest/blockstates/dead_bush_tall.json", br#"{"variants":{"half=lower":{"model":"conquest:block/dead_bush_tall_bottom"},"half=upper":{"model":"conquest:block/dead_bush_tall_top"}}}"#, true),
            ("assets/conquest/groups/k_stone.txt", b"conquest:worn_dark_lime_mortar_masonry\nminecraft:basalt\n\n", true),
            ("assets/conquest/groups/aa_advanced_masonry.txt", b"conquest:basalt_pillar\n", true),
            ("assets/conquest/lang/en_us.json", br#"{"block.conquest.worn_dark_lime_mortar_masonry":"Worn Dark Lime Mortar Masonry","block.conquest.basalt_pillar":"Basalt Pillar","itemGroup.conquest.k_stone":"Stone"}"#, true),
        ];
        let dir = std::env::temp_dir().join(format!("massif-assets-conquest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (game_jar, mod_jar) = (dir.join("1.20.1.jar"), dir.join("ConquestReforged-fabric-1.20.1-1.4.1.4.jar"));
        std::fs::write(&game_jar, zip::tests_support::tiny_zip(&game)).unwrap();
        std::fs::write(&mod_jar, zip::tests_support::tiny_zip(&conquest)).unwrap();
        let list = dir.join("conquest_wp.csv");
        std::fs::write(&list, materials::tests::CONQUEST).unwrap();
        std::fs::write(dir.join(materials::REGISTRY_LIST), "conquest:basalt_pillar\nconquest:worn_dark_lime_mortar_masonry\nconquest:worn_dark_lime_mortar_masonry_slab\nconquest:worn_dark_lime_mortar_masonry_small_arch\nconquest:dead_bush_tall\nconquest:candle_lantern\n").unwrap();
        let cat = load_with_mods(&game_jar, "1.20.1", &[mod_jar.clone(), list.clone()]).unwrap();

        let pillar = cat.block("conquest:basalt_pillar").unwrap();
        assert_eq!(pillar.top.as_deref(), Some("block/basalt_top"), "`:block/…` is the game's");
        assert_eq!(pillar.side.as_deref(), Some("block/basalt_side"));
        assert_eq!(pillar.color, [60, 60, 64]);
        assert!(!pillar.full_cube);
        let masonry_block = cat.block("conquest:worn_dark_lime_mortar_masonry").unwrap();
        assert!(masonry_block.full_cube);
        assert_eq!(masonry_block.name, "Worn Dark Lime Mortar Masonry");
        assert_eq!(masonry_block.color, [150, 140, 120]);
        assert!(cat.block("conquest:cheese_cut").is_none(), "not a block of the game");
        assert!(cat.block("minecraft:parent_cube").is_none(), "a template, not a block");
        // Blocks of the list without a file in the jar: with their map colour.
        let lantern = cat.block("conquest:candle_lantern").unwrap();
        assert_eq!(lantern.color, [128, 128, 128], "no texture, no map colour: grey");
        assert_eq!(cat.block("conquest:dead_bush_tall").unwrap().category, Category::Plants, "the list says it is a plant");
        assert!(cat.block("conquest:dead_bush_tall").unwrap().tall, "two halves");
        assert!(!pillar.tall && !masonry_block.tall);
        assert_eq!(cat.block("conquest:dead_bush_tall").unwrap().color, [0, 0x7c, 0]);
        // Only the textures the blocks show are read.
        assert!(cat.texture("conquest:block/worn_dark_lime_mortar_masonry").is_some());
        assert_eq!(cat.textures.len(), 3);

        let m = &cat.mods[0];
        assert_eq!((m.name.as_str(), m.kind, m.minecraft.as_deref()), ("Conquest Reforged", SourceKind::Mod, Some("1.20.1")), "its name without its loader, its words apart");
        assert_eq!(m.namespaces, ["conquest"]);
        assert_eq!((m.blocks, m.unregistered), (4, 1));
        assert_eq!(m.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Advanced Masonry", "Stone"], "in the order of their files, named by the language file or the file");
        assert_eq!(m.groups[1].blocks, ["conquest:worn_dark_lime_mortar_masonry", "minecraft:basalt"]);
        assert_eq!(cat.mods[1].kind, SourceKind::List);
        assert_eq!(cat.mods[1].blocks, 5);
        assert_eq!(cat.mod_of("conquest:basalt_pillar").unwrap().name, "Conquest Reforged");
        assert_eq!(plain_mod_name("Macaw's Bridges"), "Macaw's Bridges");
        // Forge's mods.toml, comments and all (Conquest Reforged 1.7.0's).
        assert_eq!(toml_value("displayName=\"Conquest Reforged\" #mandatory", "displayName").as_deref(), Some("Conquest Reforged"));
        assert_eq!(toml_value("   modId=\"minecraft\" #mandatory", "modId").as_deref(), Some("minecraft"));
        assert_eq!(toml_value("versionRange='[1.20.1,1.21)'", "versionRange").as_deref(), Some("[1.20.1,1.21)"));
        assert_eq!(toml_value("mandatory=true # a comment", "mandatory").as_deref(), Some("true"));
        assert_eq!(toml_value("displayNameX=\"no\"", "displayName"), None);
        assert_eq!(plain_mod_name("Chipped (Forge)"), "Chipped");
        assert!(cat.mod_of("minecraft:basalt").is_none());

        // States, by the list and the registry's list.
        assert_eq!(cat.check_state("conquest:worn_dark_lime_mortar_masonry_slab[layers=3,type=top,waterlogged=false]"), Ok(()));
        assert!(cat.check_state("conquest:worn_dark_lime_mortar_masonry_slab[layers=0]").is_err());
        assert_eq!(cat.check_state("conquest:basalt_pillar[layer=2]"), Ok(()), "registered, not in the list: not judged");
        assert!(matches!(cat.check_state("conquest:cheese_cut"), Err(materials::StateProblem::UnknownBlock(_))));
        assert_eq!(cat.check_state("minecraft:basalt[axis=x]"), Ok(()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Conquest Reforged's own instances, when MASSIF_CONQUEST_DIR names
    /// one or a folder of them (CurseForge's `Instances`: its Fabric and its
    /// Forge modpacks): read with the game each plays, more than 17,000
    /// blocks in its tabs, each drawn with its textures. With Conquest's
    /// lists, its blocks are exactly the registry's and their states those
    /// its list for WorldPainter gives; without them, the only blocks
    /// without textures are the block files of the jar whose model the jar
    /// does not have. It reads the player's own game, so it runs only when
    /// asked, and says why it reads nothing.
    #[test]
    fn conquest_reforged_loads() {
        let Some(dir) = std::env::var_os("MASSIF_CONQUEST_DIR").map(PathBuf::from) else {
            eprintln!("MASSIF_CONQUEST_DIR=<a Conquest Reforged instance, or a folder of instances> reads Conquest's own: skipping");
            return;
        };
        let instances = instance::with_mod_in(&dir, "conquest");
        if instances.is_empty() {
            eprintln!("no Conquest Reforged instance in {}: skipping", dir.display());
        }
        for inst in instances {
            let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
            let Some(jar) = find_jars().into_iter().find(|j| j.version == version) else {
                eprintln!("{}: Minecraft {version} is not installed here: skipping", inst.name);
                continue;
            };
            let started = std::time::Instant::now();
            let cat = match load_with_mods(&jar.path, &jar.version, &inst.files()) {
                Ok(cat) => cat,
                Err(e) => {
                    eprintln!("{}: could not be read ({e}): skipping", inst.name);
                    continue;
                }
            };
            let took = started.elapsed();
            let conquest: Vec<&Block> = cat.blocks.iter().filter(|b| namespace(&b.id) == "conquest").collect();
            eprintln!("{}: {} blocks ({} of Conquest Reforged), {} textures, read in {took:?}", inst.name, cat.blocks.len(), conquest.len(), cat.textures.len());
            assert!(conquest.len() > 17_000, "{}: {}", inst.name, conquest.len());
            let m = cat.mods.iter().find(|m| m.namespaces.iter().any(|n| n == "conquest") && m.kind == SourceKind::Mod).unwrap();
            assert_eq!((m.name.as_str(), m.minecraft.as_deref()), ("Conquest Reforged", Some("1.20.1")), "{}", inst.name);
            assert!(m.groups.len() >= 30, "{}: {} tabs", inst.name, m.groups.len());
            assert!(conquest.iter().filter(|b| b.full_cube).count() > 1_000, "{}", inst.name);
            assert!(cat.blocks.iter().all(|b| !b.id.starts_with("minecraft:parent_")), "{}", inst.name);
            // Every block drawn with its own textures.
            let bare: Vec<&str> = conquest.iter().filter(|b| [&b.top, &b.side].iter().any(|t| t.as_ref().is_none_or(|k| cat.texture(k).is_none()))).map(|b| b.id.as_str()).collect();
            let Some(ids) = cat.registered.get("conquest") else {
                // Without Conquest's lists: only files of the jar that are
                // no blocks lack textures, those whose model is missing.
                let archives: Vec<zip::Archive> = std::iter::once(jar.path.clone()).chain(inst.mods.iter().cloned()).filter_map(|p| zip::Archive::open(&p).ok()).collect();
                let model_file = |m: &str| {
                    let k = key(m);
                    let (ns, path) = k.split_once(':').unwrap_or(("minecraft", &k));
                    format!("assets/{ns}/models/{path}.json")
                };
                for id in &bare {
                    let path = id.strip_prefix("conquest:").unwrap_or(id);
                    let state = archives.iter().find_map(|a| a.read(&format!("assets/conquest/blockstates/{path}.json"))?.ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
                    let model = state.as_ref().and_then(first_model);
                    assert!(model.as_ref().is_none_or(|m| !archives.iter().any(|a| a.contains(&model_file(m)))), "{}: {id} has its model but no textures", inst.name);
                }
                eprintln!("{}: no lists of Conquest's blocks; {} block files without a model: {bare:?}", inst.name, bare.len());
                continue;
            };
            assert!(bare.is_empty(), "{}: {} blocks without their textures: {:?}", inst.name, bare.len(), &bare[..bare.len().min(20)]);
            // With its lists: exactly the blocks the game registers.
            let ours: std::collections::HashSet<&str> = conquest.iter().map(|b| b.id.as_str()).collect();
            let missing: Vec<&String> = ids.iter().filter(|id| !ours.contains(id.as_str())).collect();
            assert!(missing.is_empty(), "{missing:?}");
            assert_eq!(ours.len(), ids.len(), "none the game does not register");
            // Every value of every property stays one of the block's when
            // an object made of it turns or is mirrored.
            let mut turned = 0;
            for (id, def) in &cat.definitions {
                for p in &def.properties {
                    for v in &p.values {
                        let state = format!("{id}[{}={v}]", p.name);
                        assert_eq!(cat.check_state(&state), Ok(()));
                        for (turn, mirror) in [(1, false), (2, false), (3, false), (0, true), (1, true)] {
                            let t = massif_core::objects::transform_state(&state, turn, mirror);
                            assert_eq!(cat.check_state(&t), Ok(()), "{state} turned {turn}, mirrored {mirror}: {t}");
                            turned += 1;
                        }
                    }
                }
            }
            eprintln!("{}: {turned} turned states checked", inst.name);
        }
    }

    /// Mods' jars named by MASSIF_MOD_JARS (separated by `;`), each read
    /// over the version of the game it is made for when it is installed:
    /// every block state file of the mod gives a block, nearly every one
    /// with its textures, and a block's family read from its name or its
    /// drawing (its ground with the soils, its plants with the plants).
    /// Prints how many show at first, and how many each family has.
    #[test]
    fn mod_jars_give_every_block() {
        let Some(list) = std::env::var_os("MASSIF_MOD_JARS") else {
            eprintln!("MASSIF_MOD_JARS=<jar;jar…> reads mods' jars: skipping");
            return;
        };
        for jar in std::env::split_paths(&list).chain(list.to_string_lossy().split(';').map(PathBuf::from)).filter(|p| p.is_file()).collect::<std::collections::BTreeSet<_>>() {
            let Ok(names) = zip::Archive::list(&jar) else { continue };
            let Some(version) = zip::Archive::open(&jar).ok().and_then(|a| mod_minecraft(&a, &jar)) else { continue };
            // Its version, else the newest installed of the same series
            // (`[1.21,1.21.2)` is read as 1.21; 1.21.1 plays it).
            let series = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
            let jars = find_jars();
            let Some(game) = jars.iter().find(|j| j.version == version).or_else(|| jars.iter().find(|j| series(&j.version) == series(&version))).cloned() else {
                eprintln!("{}: Minecraft {version} is not installed here: skipping", jar.display());
                continue;
            };
            let cat = load_with_mods(&game.path, &game.version, std::slice::from_ref(&jar)).unwrap();
            let m = cat.mods.iter().find(|m| m.kind == SourceKind::Mod).unwrap();
            let spaces: Vec<&str> = m.namespaces.iter().map(String::as_str).filter(|n| *n != "minecraft").collect();
            let files = names.iter().filter(|n| asset_key(n, "blockstates", ".json").is_some_and(|k| spaces.contains(&namespace(&k)))).count();
            let mine: Vec<&Block> = cat.blocks.iter().filter(|b| spaces.contains(&namespace(&b.id))).collect();
            let bare = mine.iter().filter(|b| b.top.as_ref().is_none_or(|k| cat.texture(k).is_none())).count();
            let first = mine.iter().filter(|b| b.full_cube || matches!(b.category, Category::Plants | Category::Natural)).count();
            let mut families: std::collections::BTreeMap<String, usize> = Default::default();
            for b in &mine {
                *families.entry(format!("{:?}", b.category)).or_default() += 1;
            }
            eprintln!("{}: {} blocks of {} block files, {bare} without textures, {first} shown at first; {families:?}", m.name, mine.len(), files);
            let biomes: Vec<&BiomeInfo> = cat.biomes.iter().filter(|b| b.source == m.name).collect();
            eprintln!("{}: {} biomes, {} of the game", m.name, biomes.len(), cat.biomes.iter().filter(|b| b.source == MINECRAFT).count());
            assert_eq!(biomes.len(), names.iter().filter(|n| biome_key(n).is_some_and(|k| spaces.contains(&namespace(&k)))).count(), "{}: every biome file of its namespaces gives a biome", m.name);
            if let Some(dump) = std::env::var_os("MASSIF_MOD_DUMP") {
                let text: String = mine.iter().map(|b| format!("{}\t{:?}\t{}\t{}\t{:?}\t{:?}\n", b.id, b.category, b.full_cube, b.tall, b.top, b.side)).collect();
                std::fs::write(dump, text).unwrap();
            }
            assert_eq!(mine.len(), files, "{}: every block file gives a block", m.name);
            assert!(bare * 100 <= mine.len(), "{}: {bare} blocks without textures", m.name);
        }
    }

    /// The mod profiles (`massif_core::mod_profiles`) against the mods'
    /// own jars, named by MASSIF_MOD_JARS (separated by `;`) and read over
    /// the game installed nearest their version: each profile's name is its
    /// mod's, every block of its palette is a whole block of the mod or the
    /// game with its textures and a state the mod gives, and the colours
    /// the palette keeps for before the game is read are its textures'.
    /// MASSIF_PROFILE_CANDIDATES=1 prints each mod's whole blocks of
    /// ground and stone with their colours, to make a profile from.
    #[test]
    fn mod_profiles_match_their_mods() {
        let Some(list) = std::env::var_os("MASSIF_MOD_JARS") else {
            eprintln!("MASSIF_MOD_JARS=<jar;jar…> checks the mod profiles against their mods: skipping");
            return;
        };
        let series = |v: &str| v.split('.').take(2).collect::<Vec<_>>().join(".");
        let jars = find_jars();
        for jar in list.to_string_lossy().split(';').map(PathBuf::from).filter(|p| p.is_file()) {
            let Some(version) = zip::Archive::open(&jar).ok().and_then(|a| mod_minecraft(&a, &jar)) else { continue };
            let Some(game) = jars.iter().find(|j| j.version == version).or_else(|| jars.iter().find(|j| series(&j.version) == series(&version))).or(jars.first()).cloned() else {
                eprintln!("no Minecraft installed: skipping");
                return;
            };
            let cat = load_with_mods(&game.path, &game.version, std::slice::from_ref(&jar)).unwrap();
            let m = cat.mods.iter().find(|m| m.kind == SourceKind::Mod).unwrap().clone();
            if std::env::var_os("MASSIF_PROFILE_CANDIDATES").is_some() {
                for b in cat.blocks.iter().filter(|b| m.namespaces.contains(&namespace(&b.id).to_owned()) && b.full_cube && matches!(b.category, Category::Natural | Category::Stone)) {
                    eprintln!("{} {:?} {:?} {:?}", b.id, b.category, b.color, b.name);
                }
            }
            for ns in &m.namespaces {
                let Some(profile) = massif_core::mod_profiles::of(ns) else { continue };
                eprintln!("{}: profile {} against {} (Minecraft {}), {} blocks", jar.display(), profile.namespace, m.name, game.version, cat.blocks.iter().filter(|b| namespace(&b.id) == ns).count());
                assert_eq!(profile.name, m.name, "the profile names the mod as its jar does");
                let Some(palette) = &profile.palette else { continue };
                let mut off = Vec::new();
                for state in palette.states() {
                    let id = state.split('[').next().unwrap();
                    let block = cat.block(id).unwrap_or_else(|| panic!("{state}: no such block"));
                    assert!(block.full_cube, "{state}: not a whole block");
                    assert!(block.top.as_ref().is_some_and(|t| cat.texture(t).is_some()), "{state}: no texture");
                    assert_eq!(cat.check_state(state), Ok(()), "{state}");
                    if namespace(id) != "minecraft" && palette.colors.get(state).is_none_or(|c| c.iter().zip(block.color).any(|(a, b)| a.abs_diff(b) > 8)) {
                        off.push(format!("\"{state}\": [{}, {}, {}]", block.color[0], block.color[1], block.color[2]));
                    }
                }
                assert!(off.is_empty(), "{}: colours to keep in its palette.json: {{{}}}", profile.namespace, off.join(", "));
            }
        }
    }

    /// The jars of this computer, when there are any (not in CI).
    #[test]
    fn real_jars_load() {
        let Some(result) = load_newest() else { return };
        let cat = result.unwrap();
        eprintln!("{} blocks, {} textures from {}", cat.blocks.len(), cat.textures.len(), cat.source.display());
        assert!(cat.blocks.len() > 800);
        let grass = cat.block("minecraft:grass_block").unwrap();
        assert!(grass.full_cube && grass.top_tint == Tint::Grass);
        assert!(grass.color[1] > grass.color[0] && grass.color[1] > grass.color[2], "grass is green: {:?}", grass.color);
        let stone = cat.block("minecraft:stone").unwrap();
        assert!(stone.full_cube);
        let log = cat.block("minecraft:oak_log").unwrap();
        assert_ne!(log.top, log.side);
        let flower = cat.block("minecraft:poppy").unwrap();
        assert!(!flower.full_cube);

        // Real colours in the registry, and a terrain of any block.
        let mut reg = Registry::vanilla();
        apply_colors(&mut reg, &cat);
        let grass_terrain = reg.terrain_by_name("Grass").unwrap();
        let t = reg.terrain(grass_terrain).color;
        assert!(t[1] > t[0] && t[1] > t[2], "the grass terrain is green: {t:?}");
        let bricks = cat.block("minecraft:stone_bricks").unwrap();
        let id = block_terrain(&mut reg, bricks);
        assert_eq!(block_terrain(&mut reg, bricks), id, "made once");
        assert_eq!(reg.terrain(id).name, "Stone Bricks");
        assert_eq!(terrain_block(&reg, &cat, id).unwrap().id, "minecraft:stone_bricks");
        let icon = cube_icon(&cat, grass, 32);
        assert_eq!(icon.len(), 32 * 32 * 4);
        assert_eq!(icon[3], 0, "transparent corner");
        assert_eq!(icon[(16 * 32 + 16) * 4 + 3], 255, "opaque middle");
    }

    /// Biome colours as numbers (before 26.1) and as hex strings (26.1 and
    /// after) read the same.
    #[test]
    fn biome_colours_as_numbers_and_hex_strings() {
        assert_eq!(rgb_of(Some(&serde_json::json!(6975545))), Some([0x6a, 0x70, 0x39]));
        assert_eq!(rgb_of(Some(&serde_json::json!("#6a7039"))), Some([0x6a, 0x70, 0x39]));
        assert_eq!(rgb_of(Some(&serde_json::json!("6a7039"))), None, "the game writes its #");
        assert_eq!(rgb_of(Some(&serde_json::json!("#zz0000"))), None);
        let swamp = serde_json::json!({"temperature": 0.8, "downfall": 0.9, "effects": {"foliage_color": "#6a7039", "grass_color_modifier": "swamp", "water_color": "#617b64"}});
        let b = biome_info("minecraft:swamp", MINECRAFT, &swamp, None, &true_color::Rules::default());
        assert_eq!((b.grass, b.foliage, b.water), ([0x6a, 0x70, 0x39], [0x6a, 0x70, 0x39], [0x61, 0x7b, 0x64]));
    }

    /// The colours of the game's own biomes, from the newest jar on this
    /// computer (skipped without one), against the game's: those a biome
    /// names (swamp, badlands, warm ocean) and those its colour maps give.
    #[test]
    fn biome_colours_match_the_game() {
        let Some(jar) = find_jars().into_iter().find(|j| release_numbers(&j.version).is_some_and(|n| n >= vec![1, 19])) else {
            eprintln!("no game jar: skipped");
            return;
        };
        let cat = load(&jar.path, &jar.version).unwrap();
        let hex = |c: [u8; 3]| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
        let check = |id: &str, grass: u32, foliage: u32, water: u32| {
            let b = cat.biome(id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!((hex(b.grass), hex(b.foliage), hex(b.water)), (grass, foliage, water), "{id} in {}", jar.version);
        };
        check("minecraft:plains", 0x91bd59, 0x77ab2f, 0x3f76e4);
        check("minecraft:forest", 0x79c05a, 0x59ae30, 0x3f76e4);
        check("minecraft:jungle", 0x59c93c, 0x30bb0b, 0x3f76e4);
        check("minecraft:desert", 0xbfb755, 0xaea42a, 0x3f76e4);
        check("minecraft:swamp", 0x6a7039, 0x6a7039, 0x617b64);
        check("minecraft:badlands", 0x90814d, 0x9e814d, 0x3f76e4);
        assert_eq!(hex(cat.biome("minecraft:warm_ocean").unwrap().water), 0x43d5ee);
    }
}
