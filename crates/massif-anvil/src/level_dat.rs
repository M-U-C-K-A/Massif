//! `level.dat`: the world's settings, as each version writes them.
//!
//! The shape changed since 1.18.2 (read from the game's own data fixers,
//! which turn an older file into the newer one, and from 26.3's codecs):
//!
//! - 1.19 dropped the seed of each dimension's generator, and 1.19.3 added
//!   the enabled feature flags;
//! - 1.21.9 gathered the spawn point (`WorldSpawnDataFix`) and the border
//!   (`LegacyWorldBorderFix`) into compounds, and 1.21.11 moved the border
//!   into `data/world_border.dat`, its warning time in ticks
//!   (`WorldBorderWarningTimeFix`);
//! - 26.1 split the file (`LevelDatDifficultyFix`, `DayTimeToClockFix`,
//!   `LevelDatToSavedDataFileFix`, `DimensionStorageFileFix`): the
//!   difficulty became a compound, and the world's generation, weather,
//!   clock, wandering trader and border went into saved data of their own,
//!   the border one per dimension under `dimensions/minecraft/`.
//!
//! Written as the version itself writes it, the world opens without the
//! game upgrading anything.
//!
//! The game rules (`/gamerule`) were strings in a `GameRules` compound,
//! named in camel case, up to 1.21.10; 1.21.11 made them a registry
//! (`GameRuleRegistryFix`, data version 4658): a `game_rules` compound of
//! typed values under ids such as `minecraft:spawn_mobs`, `doFireTick`
//! becoming the radius fire spreads in around players; 26.1 moved that
//! compound to `data/minecraft/game_rules.dat`. Only the rules that differ
//! from the game's are written: a world with the game's rules is as it was.

use crate::version::{Era, Target};
use massif_core::rules::{GameRules, Weather};
use crate::nbt::{NbtWriter, TagType};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival = 0,
    Creative = 1,
    Adventure = 2,
    Spectator = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful = 0,
    Easy = 1,
    Normal = 2,
    Hard = 3,
}

/// The border Minecraft draws around the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorder {
    pub center: [f64; 2],
    /// Side of the square, in blocks.
    pub size: f64,
}

impl Default for WorldBorder {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            size: 60_000_000.0,
        }
    }
}

/// What Minecraft generates outside the exported map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surroundings {
    /// Normal vanilla terrain from the world seed.
    Vanilla,
    /// Nothing: an empty void.
    Void,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelSettings {
    pub name: String,
    pub seed: i64,
    pub spawn: [i32; 3],
    pub game_mode: GameMode,
    pub allow_commands: bool,
    pub surroundings: Surroundings,
    /// Whether vanilla structures generate in new chunks.
    pub generate_structures: bool,
    /// Milliseconds since the Unix epoch.
    pub last_played: i64,
    pub difficulty: Difficulty,
    pub hardcore: bool,
    pub border: WorldBorder,
    /// Data packs to enable besides vanilla, by their id (`file/name.zip`).
    pub datapacks: Vec<String>,
    /// The game rules, the weather and the time of day it starts with.
    pub rules: GameRules,
}

/// The value of a game rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleValue {
    Bool(bool),
    Int(i32),
}

/// A game rule as the versions name it: its name up to 1.21.10, its id
/// from 1.21.11, the first data version that has it, and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub name: &'static str,
    pub id: &'static str,
    pub since: i32,
    pub value: RuleValue,
}

/// From 1.21.11, the rules are a registry, typed (see the module's notes).
pub const RULE_REGISTRY: i32 = 4658;

/// The game rules `rules` sets apart from the game's, as `target` names
/// them, the ones it does not have left out.
pub fn game_rules(rules: &GameRules, target: &Target) -> Vec<Rule> {
    let game = GameRules::vanilla();
    let dv = target.data_version;
    // The versions before 1.9 have no data version: 1.8 brought the random
    // tick speed.
    let old = target.name == "1.7.10";
    let mut out = Vec::new();
    let mut bool_rule = |name, id, since, on: bool, game: bool| {
        if on != game {
            out.push(Rule { name, id, since, value: RuleValue::Bool(on) });
        }
    };
    bool_rule("doMobSpawning", "minecraft:spawn_mobs", 0, rules.spawn_mobs, game.spawn_mobs);
    // 1.15.2 brought these two rules; 1.15 the phantoms'.
    bool_rule("doPatrolSpawning", "minecraft:spawn_patrols", 2230, rules.spawn_patrols, game.spawn_patrols);
    bool_rule("doTraderSpawning", "minecraft:spawn_wandering_traders", 2230, rules.spawn_traders, game.spawn_traders);
    bool_rule("doInsomnia", "minecraft:spawn_phantoms", 2225, rules.spawn_phantoms, game.spawn_phantoms);
    bool_rule("doDaylightCycle", "minecraft:advance_time", 0, rules.advance_time, game.advance_time);
    // 1.11.
    bool_rule("doWeatherCycle", "minecraft:advance_weather", 819, rules.advance_weather, game.advance_weather);
    bool_rule("mobGriefing", "minecraft:mob_griefing", 0, rules.mob_griefing, game.mob_griefing);
    bool_rule("keepInventory", "minecraft:keep_inventory", 0, rules.keep_inventory, game.keep_inventory);
    if !rules.fire_spreads {
        // From 1.21.11, the radius around players fire spreads in, 0 for
        // none (the game's 128 when it spreads).
        let value = if dv >= RULE_REGISTRY { RuleValue::Int(0) } else { RuleValue::Bool(false) };
        out.push(Rule { name: "doFireTick", id: "minecraft:fire_spread_radius_around_player", since: 0, value });
    }
    let speed = rules.random_tick_speed.clamp(0, GameRules::MAX_TICK_SPEED);
    if speed != game.random_tick_speed && !old {
        out.push(Rule { name: "randomTickSpeed", id: "minecraft:random_tick_speed", since: 0, value: RuleValue::Int(speed) });
    }
    out.retain(|r| dv >= r.since || (dv == 0 && r.since == 0));
    out
}

/// The rules as `target` keeps them: strings in `GameRules` before
/// 1.21.11, typed values under their ids from it.
fn write_rules(w: &mut NbtWriter, rules: &[Rule], target: &Target) {
    for rule in rules {
        if target.data_version >= RULE_REGISTRY {
            match rule.value {
                RuleValue::Bool(on) => w.bool(rule.id, on),
                RuleValue::Int(n) => w.int(rule.id, n),
            }
        } else {
            match rule.value {
                RuleValue::Bool(on) => w.string(rule.name, if on { "true" } else { "false" }),
                RuleValue::Int(n) => w.string(rule.name, &n.to_string()),
            }
        }
    }
}

/// The data version from which a dimension's generator has no seed of its
/// own (1.19).
const SEEDLESS: i32 = 3105;
/// From 1.19.3, the enabled feature flags.
const FEATURE_FLAGS: i32 = 3218;
/// From 1.21.9, the spawn point and the border as compounds.
const COMPOUNDS: i32 = 4544;
/// From 1.21.11, the border in a file of its own, its warning time in ticks.
const BORDER_FILE: i32 = 4657;
/// From 1.20 (the game's `LegacyDragonFightFix`, data version 3459) the
/// game reads the dragon fight of level.dat with a codec, and logs an error
/// on every start when it is missing (`key missing: DragonFight`), until
/// 26.1 moves it to a file of the End's own.
const DRAGON_FIGHT: i32 = 3459;

fn noise_dimension(w: &mut NbtWriter, id: &str, settings: &str, seed: Option<i64>, biome_source: impl FnOnce(&mut NbtWriter)) {
    w.compound(id);
    w.string("type", id);
    w.compound("generator");
    w.string("type", "minecraft:noise");
    w.string("settings", settings);
    if let Some(seed) = seed {
        w.long("seed", seed);
    }
    w.compound("biome_source");
    biome_source(w);
    w.end();
    w.end();
    w.end();
}

/// The uncompressed NBT of `level.dat`, as `target` reads it: the world's
/// generator went from a name (up to 1.15) to the settings of every
/// dimension (1.16), whose biomes changed source in 1.18.
pub fn level_nbt(s: &LevelSettings, target: &Target) -> Vec<u8> {
    let era = target.era();
    let dv = target.data_version;
    let split = target.splits_level();
    let named = era != Era::Numbered;
    let (data_version, version_name) = target.level_version();
    let mut w = NbtWriter::new();
    w.compound("Data");
    if data_version > 0 {
        w.int("DataVersion", data_version);
    }
    w.int("version", 19133);
    if named {
        w.compound("Version");
        w.int("Id", data_version);
        w.string("Name", version_name);
        if era == Era::Modern {
            w.string("Series", "main");
        }
        w.bool("Snapshot", false);
        w.end();
    }

    w.string("LevelName", &s.name);
    w.long("LastPlayed", s.last_played);
    w.int("GameType", s.game_mode as i32);
    if split {
        w.compound("difficulty_settings");
        w.string("difficulty", ["peaceful", "easy", "normal", "hard"][s.difficulty as usize]);
        w.bool("hardcore", s.hardcore);
        w.bool("locked", false);
        w.end();
    } else {
        w.byte("Difficulty", s.difficulty as i8);
        w.bool("DifficultyLocked", false);
        w.bool("hardcore", s.hardcore);
    }
    w.bool("allowCommands", s.allow_commands);
    // Tells the game the spawn point is already chosen.
    w.bool("initialized", true);
    let rules = game_rules(&s.rules, target);
    if !rules.is_empty() && !split {
        w.compound(if dv >= RULE_REGISTRY { "game_rules" } else { "GameRules" });
        write_rules(&mut w, &rules, target);
        w.end();
    }

    if dv >= COMPOUNDS {
        w.compound("spawn");
        w.string("dimension", "minecraft:overworld");
        w.int_array("pos", &s.spawn);
        w.float("yaw", 0.0);
        w.float("pitch", 0.0);
        w.end();
    } else {
        w.int("SpawnX", s.spawn[0]);
        // Before 1.18 a world ends at 255.
        w.int("SpawnY", if era == Era::Modern { s.spawn[1] } else { s.spawn[1].clamp(1, 255) });
        w.int("SpawnZ", s.spawn[2]);
    }

    w.long("Time", 0);
    if !split {
        w.long("DayTime", s.rules.time);
        w.bool("raining", s.rules.weather != Weather::Clear);
        w.int("rainTime", 0);
        w.bool("thundering", s.rules.weather == Weather::Thunder);
        w.int("thunderTime", 0);
        w.int("clearWeatherTime", 0);
    }

    if dv < COMPOUNDS {
        w.double("BorderCenterX", s.border.center[0]);
        w.double("BorderCenterZ", s.border.center[1]);
        w.double("BorderSize", s.border.size);
        w.double("BorderSizeLerpTarget", s.border.size);
        w.long("BorderSizeLerpTime", 0);
        w.double("BorderSafeZone", 5.0);
        w.double("BorderDamagePerBlock", 0.2);
        w.double("BorderWarningBlocks", 5.0);
        w.double("BorderWarningTime", 15.0);
    } else if dv < BORDER_FILE {
        w.compound("world_border");
        border(&mut w, s.border.center, s.border.size, 15);
        w.end();
    }

    if named {
        w.compound("DataPacks");
        w.list("Enabled", TagType::String, 1 + s.datapacks.len());
        w.item_string("vanilla");
        for pack in &s.datapacks {
            w.item_string(pack);
        }
        w.list("Disabled", TagType::String, 0);
        w.end();
    }
    if dv >= FEATURE_FLAGS {
        w.list("enabled_features", TagType::String, 1);
        w.item_string("minecraft:vanilla");
    }

    let void = s.surroundings == Surroundings::Void;
    match era {
        Era::Numbered => {
            w.long("RandomSeed", s.seed);
            w.string("generatorName", if void { "flat" } else { "default" });
            w.int("generatorVersion", if void { 0 } else { 1 });
            w.string("generatorOptions", if void { target.void_preset() } else { "" });
            w.bool("MapFeatures", s.generate_structures);
            w.long("SizeOnDisk", 0);
        }
        Era::Flat14 | Era::Flat15 => {
            w.long("RandomSeed", s.seed);
            w.string("generatorName", if void { "flat" } else { "default" });
            w.int("generatorVersion", if void { 0 } else { 1 });
            w.compound("generatorOptions");
            if void {
                w.string("biome", "minecraft:the_void");
                w.list("layers", TagType::Compound, 1);
                w.item_compound();
                w.string("block", "minecraft:air");
                w.byte("height", 1);
                w.end();
                w.compound("structures");
                w.end();
            }
            w.end();
            w.bool("MapFeatures", s.generate_structures);
        }
        Era::Flat16 | Era::Modern if split => w.bool("WasModded", false),
        Era::Flat16 | Era::Modern => {
            w.bool("WasModded", false);
            if dv < COMPOUNDS {
                w.float("SpawnAngle", 0.0);
            }
            w.int("WanderingTraderSpawnChance", 25);
            w.int("WanderingTraderSpawnDelay", 24000);
            w.compound("WorldGenSettings");
            world_gen_settings(&mut w, s, target);
            w.end(); // WorldGenSettings
            if dv >= DRAGON_FIGHT {
                // As the game writes a new world's, the End not visited
                // yet: the fight starts when a player gets there.
                w.compound("DragonFight");
                w.bool("NeedsStateScanning", true);
                w.bool("DragonKilled", false);
                w.bool("PreviouslyKilled", false);
                w.end();
            }
        }
    }

    w.end(); // Data
    w.finish()
}

/// The inside of a border compound, as 1.21.9 writes it: its warning time
/// in seconds before 1.21.11, in ticks from it.
fn border(w: &mut NbtWriter, center: [f64; 2], size: f64, warning_time: i32) {
    w.double("center_x", center[0]);
    w.double("center_z", center[1]);
    w.double("size", size);
    w.long("lerp_time", 0);
    w.double("lerp_target", size);
    w.double("safe_zone", 5.0);
    w.double("damage_per_block", 0.2);
    w.int("warning_blocks", 5);
    w.int("warning_time", warning_time);
}

/// The world's seed, structures and the generator of each dimension (from
/// 1.16).
fn world_gen_settings(w: &mut NbtWriter, s: &LevelSettings, target: &Target) {
    let era = target.era();
    let dv = target.data_version;
    let void = s.surroundings == Surroundings::Void;
    // Seeds of their own before 1.19.
    let seed = (dv < SEEDLESS).then_some(s.seed);
    w.long("seed", s.seed);
    // Renamed with the split of 26.1.
    w.bool(if target.splits_level() { "generate_structures" } else { "generate_features" }, s.generate_structures);
    w.bool("bonus_chest", false);
    w.compound("dimensions");
    if void {
        w.compound("minecraft:overworld");
        w.string("type", "minecraft:overworld");
        w.compound("generator");
        w.string("type", "minecraft:flat");
        w.compound("settings");
        w.string("biome", "minecraft:the_void");
        w.bool("features", false);
        w.bool("lakes", false);
        w.list("layers", TagType::Compound, 1);
        w.item_compound();
        w.string("block", "minecraft:air");
        w.int("height", 1);
        w.end();
        if era == Era::Flat16 {
            // 1.16 and 1.17 want to be told there are no structures.
            w.compound("structures");
            w.compound("structures");
            w.end();
            w.end();
        }
        w.end();
        w.end();
        w.end();
    } else {
        noise_dimension(w, "minecraft:overworld", "minecraft:overworld", seed, |w| {
            if era == Era::Flat16 {
                w.string("type", "minecraft:vanilla_layered");
                w.long("seed", s.seed);
                w.bool("large_biomes", false);
            } else {
                w.string("type", "minecraft:multi_noise");
                w.string("preset", "minecraft:overworld");
            }
        });
    }
    noise_dimension(w, "minecraft:the_nether", "minecraft:nether", seed, |w| {
        w.string("type", "minecraft:multi_noise");
        if era == Era::Flat16 {
            w.long("seed", s.seed);
        }
        w.string("preset", "minecraft:nether");
    });
    noise_dimension(w, "minecraft:the_end", "minecraft:end", seed, |w| {
        w.string("type", "minecraft:the_end");
        if let Some(seed) = seed {
            w.long("seed", seed);
        }
    });
    w.end(); // dimensions
}

/// A file of saved data: `data` and the data version, gzipped.
fn saved_data(target: &Target, content: impl FnOnce(&mut NbtWriter)) -> io::Result<Vec<u8>> {
    let mut w = NbtWriter::new();
    w.compound("data");
    content(&mut w);
    w.end();
    w.int("DataVersion", target.data_version);
    gzip(&w.finish())
}

fn gzip(nbt: &[u8]) -> io::Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(nbt)?;
    gz.finish()
}

/// Every file of the world's settings, by its path in the world's folder:
/// `level.dat`, and the saved data the version keeps apart from it.
pub fn world_files(s: &LevelSettings, target: &Target) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = vec![("level.dat".to_owned(), level_dat(s, target)?)];
    let dv = target.data_version;
    if target.splits_level() {
        let data = |name: &str| format!("data/minecraft/{name}.dat");
        files.push((data("world_gen_settings"), saved_data(target, |w| world_gen_settings(w, s, target))?));
        files.push((
            data("weather"),
            saved_data(target, |w| {
                w.int("clear_weather_time", 0);
                w.int("rain_time", 0);
                w.int("thunder_time", 0);
                w.bool("raining", s.rules.weather != Weather::Clear);
                w.bool("thundering", s.rules.weather == Weather::Thunder);
            })?,
        ));
        files.push((
            data("world_clocks"),
            saved_data(target, |w| {
                w.compound("minecraft:overworld");
                w.long("total_ticks", s.rules.time);
                w.bool("paused", false);
                w.end();
            })?,
        ));
        let rules = game_rules(&s.rules, target);
        if !rules.is_empty() {
            files.push((data("game_rules"), saved_data(target, |w| write_rules(w, &rules, target))?));
        }
        files.push((
            data("wandering_trader"),
            saved_data(target, |w| {
                w.int("spawn_delay", 24000);
                w.int("spawn_chance", 25);
            })?,
        ));
        // The border of each dimension, the nether's centre where the
        // overworld's comes to there.
        for (dimension, scale) in [("overworld", 1.0), ("the_nether", 8.0), ("the_end", 1.0)] {
            let center = s.border.center.map(|c| c / scale);
            files.push((format!("dimensions/minecraft/{dimension}/data/minecraft/world_border.dat"), saved_data(target, |w| border(w, center, s.border.size, 300))?));
        }
    } else if dv >= BORDER_FILE {
        files.push(("data/world_border.dat".to_owned(), saved_data(target, |w| border(w, s.border.center, s.border.size, 300))?));
    }
    Ok(files)
}

/// The gzip-compressed bytes of `level.dat`.
pub fn level_dat(settings: &LevelSettings, target: &Target) -> io::Result<Vec<u8>> {
    gzip(&level_nbt(settings, target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;
    use std::collections::HashMap;
    use std::io::Read;

    fn compound(v: &Value) -> &HashMap<String, Value> {
        match v {
            Value::Compound(c) => c,
            other => panic!("expected a compound, got {other:?}"),
        }
    }

    fn settings(surroundings: Surroundings) -> LevelSettings {
        LevelSettings {
            name: "Île d'été".into(),
            seed: -42,
            spawn: [12, 80, -7],
            game_mode: GameMode::Creative,
            allow_commands: true,
            surroundings,
            generate_structures: false,
            last_played: 1_793_000_000_000,
            difficulty: Difficulty::Hard,
            hardcore: true,
            border: WorldBorder {
                center: [100.0, -50.0],
                size: 4096.0,
            },
            datapacks: vec!["file/rivers.zip".into()],
            rules: Default::default(),
        }
    }

    #[test]
    fn level_dat_is_gzipped_nbt_with_the_expected_fields() {
        let bytes = level_dat(&settings(Surroundings::Vanilla), &Target::default()).unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut raw).unwrap();
        let root: HashMap<String, Value> = fastnbt::from_bytes(&raw).unwrap();
        let data = compound(&root["Data"]);
        assert_eq!(data["LevelName"], Value::String("Île d'été".into()));
        assert_eq!(data["DataVersion"], Value::Int(2975));
        assert_eq!(data["version"], Value::Int(19133));
        assert_eq!(data["GameType"], Value::Int(1));
        assert_eq!(data["initialized"], Value::Byte(1));
        assert_eq!(
            (&data["SpawnX"], &data["SpawnY"], &data["SpawnZ"]),
            (&Value::Int(12), &Value::Int(80), &Value::Int(-7))
        );
        let wgs = compound(&data["WorldGenSettings"]);
        assert_eq!(wgs["seed"], Value::Long(-42));
        assert_eq!(wgs["generate_features"], Value::Byte(0));
        let dims = compound(&wgs["dimensions"]);
        assert_eq!(dims.len(), 3);
        let overworld = compound(&dims["minecraft:overworld"]);
        assert_eq!(overworld["type"], Value::String("minecraft:overworld".into()));
        let generator = compound(&overworld["generator"]);
        assert_eq!(generator["type"], Value::String("minecraft:noise".into()));
        assert_eq!(generator["settings"], Value::String("minecraft:overworld".into()));
        let source = compound(&generator["biome_source"]);
        assert_eq!(source["preset"], Value::String("minecraft:overworld".into()));
        let end = compound(&compound(&dims["minecraft:the_end"])["generator"]);
        assert_eq!(compound(&end["biome_source"])["seed"], Value::Long(-42));
        assert_eq!(data["Difficulty"], Value::Byte(3));
        assert_eq!(data["hardcore"], Value::Byte(1));
        assert_eq!(data["BorderCenterX"], Value::Double(100.0));
        assert_eq!(data["BorderSize"], Value::Double(4096.0));
        match &compound(&data["DataPacks"])["Enabled"] {
            Value::List(packs) => assert_eq!(packs, &vec![Value::String("vanilla".into()), Value::String("file/rivers.zip".into())]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn void_surroundings_use_an_empty_flat_generator() {
        let root: HashMap<String, Value> =
            fastnbt::from_bytes(&level_nbt(&settings(Surroundings::Void), &Target::default())).unwrap();
        let wgs = compound(&compound(&root["Data"])["WorldGenSettings"]);
        let overworld = compound(&compound(&wgs["dimensions"])["minecraft:overworld"]);
        let generator = compound(&overworld["generator"]);
        assert_eq!(generator["type"], Value::String("minecraft:flat".into()));
        let flat = compound(&generator["settings"]);
        assert_eq!(flat["biome"], Value::String("minecraft:the_void".into()));
        match &flat["layers"] {
            Value::List(layers) => {
                assert_eq!(layers.len(), 1);
                assert_eq!(compound(&layers[0])["block"], Value::String("minecraft:air".into()));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(test)]
mod digests {
    use super::*;
    use fastnbt::Value;
    use std::collections::HashMap;

    pub(super) fn settings(surroundings: Surroundings) -> LevelSettings {
        LevelSettings {
            name: "Île d'été".into(),
            seed: -42,
            spawn: [12, 80, -7],
            game_mode: GameMode::Creative,
            allow_commands: true,
            surroundings,
            generate_structures: false,
            last_played: 1_793_000_000_000,
            difficulty: Difficulty::Hard,
            hardcore: true,
            border: WorldBorder { center: [100.0, -50.0], size: 4096.0 },
            datapacks: vec!["file/rivers.zip".into()],
            rules: Default::default(),
        }
    }

    pub(super) fn digest(bytes: &[u8]) -> u64 {
        // FNV-1a: the same on every platform and Rust version.
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
    }

    /// Up to 1.18.2, level.dat is byte for byte what Massif 0.6 wrote.
    #[test]
    fn older_versions_keep_their_bytes() {
        for (name, surroundings, want) in [
            ("1.18.2", Surroundings::Vanilla, 0xab5f400b0351fe70),
            ("1.18.2", Surroundings::Void, 0x5466117cb6480d66),
            ("1.18", Surroundings::Vanilla, 0x01dc200b324c9b58),
            ("1.18", Surroundings::Void, 0x078ae21881282c4e),
            ("1.17.1", Surroundings::Vanilla, 0xd719c3f8081d1ff3),
            ("1.17.1", Surroundings::Void, 0x340603a530a6f738),
            ("1.16.5", Surroundings::Vanilla, 0xe4aff40de923986a),
            ("1.16.5", Surroundings::Void, 0x40dd7e56c7f1f42f),
            ("1.15.2", Surroundings::Vanilla, 0xb77f7349f165d34c),
            ("1.15.2", Surroundings::Void, 0x2d46c7dc0604f511),
            ("1.14.4", Surroundings::Vanilla, 0x11561a2355dfedb5),
            ("1.14.4", Surroundings::Void, 0xa2dd7042f10edcf6),
            ("1.12.2", Surroundings::Vanilla, 0x5bb45b772e131088),
            ("1.12.2", Surroundings::Void, 0xc7d3abab6813ba5d),
            ("1.8.9", Surroundings::Vanilla, 0x418a9f1d21110ec4),
            ("1.8.9", Surroundings::Void, 0x01861b5394732f42),
        ] {
            assert_eq!(digest(&level_nbt(&settings(surroundings), &crate::Target::release(name).unwrap())), want, "{name} {surroundings:?}");
        }
    }

    fn read(bytes: &[u8]) -> HashMap<String, Value> {
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(bytes), &mut raw).unwrap();
        fastnbt::from_bytes(&raw).unwrap()
    }

    fn compound(v: &Value) -> &HashMap<String, Value> {
        match v {
            Value::Compound(c) => c,
            other => panic!("expected a compound, got {other:?}"),
        }
    }

    fn files(name: &str, surroundings: Surroundings) -> HashMap<String, HashMap<String, Value>> {
        world_files(&settings(surroundings), &crate::Target::release(name).unwrap()).unwrap().into_iter().map(|(path, bytes)| (path, read(&bytes))).collect()
    }

    /// The game rules, as each version keeps them: strings named in camel
    /// case in level.dat up to 1.21.10, typed values under their ids from
    /// 1.21.11, in a file of their own from 26.1; the rules a version lacks
    /// left out, and nothing at all for the game's own rules. The weather
    /// and the time of day follow the rules.
    #[test]
    fn the_game_rules_are_written_as_each_version_keeps_them() {
        use massif_core::rules::{GameRules, Preset, Weather};
        let frozen = Preset::Frozen.rules();
        let read_rules = |name: &str, rules: GameRules| -> (HashMap<String, Value>, Option<HashMap<String, Value>>) {
            let target = crate::Target::release(name).unwrap();
            let files: HashMap<String, HashMap<String, Value>> = world_files(&LevelSettings { rules, ..settings(Surroundings::Vanilla) }, &target).unwrap().into_iter().map(|(path, bytes)| (path, read(&bytes))).collect();
            let data = compound(&files["level.dat"]["Data"]).clone();
            let rules = files.get("data/minecraft/game_rules.dat").map(|f| compound(&f["data"]).clone()).or_else(|| data.get("game_rules").map(|c| compound(c).clone())).or_else(|| data.get("GameRules").map(|c| compound(c).clone()));
            (data, rules)
        };
        let s = |v: &str| Value::String(v.into());
        // Up to 1.21.10: strings.
        let (data, rules) = read_rules("1.21.1", frozen);
        let rules = rules.expect("GameRules");
        assert!(data.contains_key("GameRules"));
        for (name, value) in [("doMobSpawning", "false"), ("doWeatherCycle", "false"), ("doDaylightCycle", "false"), ("randomTickSpeed", "0"), ("doFireTick", "false"), ("mobGriefing", "false"), ("doPatrolSpawning", "false"), ("doTraderSpawning", "false"), ("doInsomnia", "false")] {
            assert_eq!(rules[name], s(value), "1.21.1 {name}");
        }
        assert!(!rules.contains_key("keepInventory"), "the game's own value is not written");
        assert_eq!(data["DayTime"], Value::Long(6000));
        // 1.14: no patrol, trader nor phantom rule yet.
        let (_, rules) = read_rules("1.14.4", frozen);
        let rules = rules.unwrap();
        assert!(rules.contains_key("doMobSpawning") && !rules.contains_key("doPatrolSpawning") && !rules.contains_key("doInsomnia"));
        // 1.21.11: the registry, typed, in level.dat.
        let (data, rules) = read_rules("1.21.11", GameRules { keep_inventory: true, ..frozen });
        let rules = rules.unwrap();
        assert!(data.contains_key("game_rules") && !data.contains_key("GameRules"));
        for (id, value) in [("minecraft:spawn_mobs", Value::Byte(0)), ("minecraft:advance_weather", Value::Byte(0)), ("minecraft:advance_time", Value::Byte(0)), ("minecraft:random_tick_speed", Value::Int(0)), ("minecraft:fire_spread_radius_around_player", Value::Int(0)), ("minecraft:mob_griefing", Value::Byte(0)), ("minecraft:spawn_patrols", Value::Byte(0)), ("minecraft:spawn_wandering_traders", Value::Byte(0)), ("minecraft:spawn_phantoms", Value::Byte(0)), ("minecraft:keep_inventory", Value::Byte(1))] {
            assert_eq!(rules[id], value, "1.21.11 {id}");
        }
        // 26.1 on: a file of their own; the weather and the clock apart.
        for name in ["26.1", "26.3"] {
            let target = crate::Target::release(name).unwrap();
            let files: HashMap<String, HashMap<String, Value>> = world_files(&LevelSettings { rules: GameRules { weather: Weather::Thunder, ..frozen }, ..settings(Surroundings::Vanilla) }, &target).unwrap().into_iter().map(|(path, bytes)| (path, read(&bytes))).collect();
            let rules = compound(&files["data/minecraft/game_rules.dat"]["data"]);
            assert_eq!(rules["minecraft:spawn_mobs"], Value::Byte(0), "{name}");
            assert_eq!(files["data/minecraft/game_rules.dat"]["DataVersion"], Value::Int(target.data_version));
            assert!(!compound(&files["level.dat"]["Data"]).contains_key("game_rules"), "{name}");
            let weather = compound(&files["data/minecraft/weather.dat"]["data"]);
            assert_eq!((&weather["raining"], &weather["thundering"]), (&Value::Byte(1), &Value::Byte(1)), "{name}");
            assert_eq!(compound(&compound(&files["data/minecraft/world_clocks.dat"]["data"])["minecraft:overworld"])["total_ticks"], Value::Long(6000), "{name}");
        }
        // The game's own rules: nothing written, the files as they were.
        for name in ["1.12.2", "1.21.1", "1.21.11", "26.3"] {
            let (data, rules) = read_rules(name, GameRules::vanilla());
            assert!(rules.is_none() && data.get("DayTime").is_none_or(|t| *t == Value::Long(1000)), "{name}");
        }
        // Fire alone, from 1.21.11: the radius it spreads in.
        let (_, rules) = read_rules("1.21.11", Preset::SurvivalSafe.rules());
        assert_eq!(rules.unwrap().len(), 2);
    }

    /// From 1.19 on, each version's level.dat and saved data hold what the
    /// version writes itself, as its data fixers make them of 1.18.2's.
    #[test]
    fn each_version_gets_its_own_level_dat() {
        for name in ["1.19.2", "1.19.4", "1.20", "1.20.1", "1.20.4", "1.21.5", "1.21.8", "1.21.9", "1.21.10", "1.21.11", "26.1", "26.1.2", "26.2", "26.3"] {
            let target = crate::Target::release(name).unwrap();
            let dv = target.data_version;
            let files = files(name, Surroundings::Vanilla);
            let data = compound(&files["level.dat"]["Data"]);
            assert_eq!(data["DataVersion"], Value::Int(dv), "{name}");
            let version = compound(&data["Version"]);
            assert_eq!((&version["Id"], &version["Name"]), (&Value::Int(dv), &Value::String(name.into())), "{name}");
            // The seed is the world's alone from 1.19.
            let has = |key: &str| data.contains_key(key);
            assert_eq!(has("enabled_features"), dv >= 3218, "{name}");
            // The dragon fight from 1.20 to 1.21.11, as a new world has it.
            assert_eq!(has("DragonFight"), (3459..4772).contains(&dv), "{name}");
            if has("DragonFight") {
                let fight = compound(&data["DragonFight"]);
                assert_eq!(fight.len(), 3, "{name}");
                assert_eq!((&fight["NeedsStateScanning"], &fight["DragonKilled"], &fight["PreviouslyKilled"]), (&Value::Byte(1), &Value::Byte(0), &Value::Byte(0)), "{name}");
            }
            // The spawn point and the border as compounds from 1.21.9.
            assert_eq!(has("SpawnX"), dv < 4544, "{name}");
            assert_eq!(has("BorderSize"), dv < 4544, "{name}");
            if dv >= 4544 {
                let spawn = compound(&data["spawn"]);
                assert_eq!(spawn["pos"], Value::IntArray(fastnbt::IntArray::new(vec![12, 80, -7])), "{name}");
                assert_eq!(spawn["dimension"], Value::String("minecraft:overworld".into()), "{name}");
                assert!(!has("SpawnAngle"), "{name}");
            }
            // The border: in level.dat for 1.21.9 and 1.21.10, in a file of
            // its own from 1.21.11, one a dimension from 26.1.
            assert_eq!(has("world_border"), (4544..4657).contains(&dv), "{name}");
            let border = match dv {
                ..4544 => None,
                4544..4657 => Some(compound(&data["world_border"])),
                4657..4772 => Some(compound(&files["data/world_border.dat"]["data"])),
                _ => Some(compound(&files["dimensions/minecraft/overworld/data/minecraft/world_border.dat"]["data"])),
            };
            if let Some(border) = border {
                assert_eq!(border["size"], Value::Double(4096.0), "{name}");
                assert_eq!(border["center_x"], Value::Double(100.0), "{name}");
                assert_eq!(border["warning_time"], Value::Int(if dv >= 4657 { 300 } else { 15 }), "{name}");
            }
            let split = dv >= 4772;
            assert_eq!(files.len(), match dv { ..4657 => 1, 4657..4772 => 2, _ => 8 }, "{name}: {:?}", files.keys());
            // 26.1 splits level.dat.
            for key in ["WorldGenSettings", "DayTime", "raining", "clearWeatherTime", "WanderingTraderSpawnChance", "Difficulty", "DifficultyLocked", "hardcore"] {
                assert_eq!(has(key), !split, "{name}: {key}");
            }
            let worldgen = if split {
                let difficulty = compound(&data["difficulty_settings"]);
                assert_eq!((&difficulty["difficulty"], &difficulty["hardcore"]), (&Value::String("hard".into()), &Value::Byte(1)), "{name}");
                for file in ["weather", "world_clocks", "wandering_trader", "world_gen_settings"] {
                    assert_eq!(files[&format!("data/minecraft/{file}.dat")]["DataVersion"], Value::Int(dv), "{name}: {file}");
                }
                let clock = compound(&compound(&files["data/minecraft/world_clocks.dat"]["data"])["minecraft:overworld"]);
                assert_eq!(clock["total_ticks"], Value::Long(1000), "{name}");
                let nether = compound(&files["dimensions/minecraft/the_nether/data/minecraft/world_border.dat"]["data"]);
                assert_eq!(nether["center_x"], Value::Double(12.5), "{name}: the nether's border is centred eight times nearer");
                compound(&files["data/minecraft/world_gen_settings.dat"]["data"])
            } else {
                compound(&data["WorldGenSettings"])
            };
            assert_eq!(worldgen["seed"], Value::Long(-42), "{name}");
            assert_eq!(worldgen[if split { "generate_structures" } else { "generate_features" }], Value::Byte(0), "{name}");
            let dims = compound(&worldgen["dimensions"]);
            for (id, preset) in [("minecraft:overworld", Some("minecraft:overworld")), ("minecraft:the_nether", Some("minecraft:nether")), ("minecraft:the_end", None)] {
                let generator = compound(&compound(&dims[id])["generator"]);
                assert!(!generator.contains_key("seed"), "{name}: {id}");
                let source = compound(&generator["biome_source"]);
                assert!(!source.contains_key("seed"), "{name}: {id}");
                if let Some(preset) = preset {
                    assert_eq!(source["preset"], Value::String(preset.into()), "{name}: {id}");
                }
            }
        }
        // A void around the map, in 26.3's own file.
        let files = files("26.3", Surroundings::Void);
        let worldgen = compound(&files["data/minecraft/world_gen_settings.dat"]["data"]);
        let generator = compound(&compound(&compound(&worldgen["dimensions"])["minecraft:overworld"])["generator"]);
        assert_eq!(generator["type"], Value::String("minecraft:flat".into()));
        assert_eq!(compound(&generator["settings"])["biome"], Value::String("minecraft:the_void".into()));
    }
}
