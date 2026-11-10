//! `level.dat`: the world's settings.
//!
//! Written in the Minecraft 1.18.2 layout, which every later version upgrades
//! on load (spawn point, world generation settings and so on are migrated by
//! the game's own data fixers).

use crate::chunk::{DATA_VERSION, DATA_VERSION_NAME};
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
}

fn noise_dimension(w: &mut NbtWriter, id: &str, settings: &str, seed: i64, biome_source: impl FnOnce(&mut NbtWriter)) {
    w.compound(id);
    w.string("type", id);
    w.compound("generator");
    w.string("type", "minecraft:noise");
    w.string("settings", settings);
    w.long("seed", seed);
    w.compound("biome_source");
    biome_source(w);
    w.end();
    w.end();
    w.end();
}

/// The uncompressed NBT of `level.dat`.
pub fn level_nbt(s: &LevelSettings) -> Vec<u8> {
    let mut w = NbtWriter::new();
    w.compound("Data");
    w.int("DataVersion", DATA_VERSION);
    w.int("version", 19133);
    w.compound("Version");
    w.int("Id", DATA_VERSION);
    w.string("Name", DATA_VERSION_NAME);
    w.string("Series", "main");
    w.bool("Snapshot", false);
    w.end();

    w.string("LevelName", &s.name);
    w.long("LastPlayed", s.last_played);
    w.int("GameType", s.game_mode as i32);
    w.byte("Difficulty", 2);
    w.bool("DifficultyLocked", false);
    w.bool("hardcore", false);
    w.bool("allowCommands", s.allow_commands);
    // Tells the game the spawn point is already chosen.
    w.bool("initialized", true);
    w.bool("WasModded", false);

    w.int("SpawnX", s.spawn[0]);
    w.int("SpawnY", s.spawn[1]);
    w.int("SpawnZ", s.spawn[2]);
    w.float("SpawnAngle", 0.0);

    w.long("Time", 0);
    w.long("DayTime", 1000);
    w.bool("raining", false);
    w.int("rainTime", 0);
    w.bool("thundering", false);
    w.int("thunderTime", 0);
    w.int("clearWeatherTime", 0);

    w.double("BorderCenterX", 0.0);
    w.double("BorderCenterZ", 0.0);
    w.double("BorderSize", 60_000_000.0);
    w.double("BorderSizeLerpTarget", 60_000_000.0);
    w.long("BorderSizeLerpTime", 0);
    w.double("BorderSafeZone", 5.0);
    w.double("BorderDamagePerBlock", 0.2);
    w.double("BorderWarningBlocks", 5.0);
    w.double("BorderWarningTime", 15.0);

    w.int("WanderingTraderSpawnChance", 25);
    w.int("WanderingTraderSpawnDelay", 24000);

    w.compound("DataPacks");
    w.list("Enabled", TagType::String, 1);
    w.item_string("vanilla");
    w.list("Disabled", TagType::String, 0);
    w.end();

    w.compound("WorldGenSettings");
    w.long("seed", s.seed);
    w.bool("generate_features", s.generate_structures);
    w.bool("bonus_chest", false);
    w.compound("dimensions");
    match s.surroundings {
        Surroundings::Vanilla => {
            noise_dimension(&mut w, "minecraft:overworld", "minecraft:overworld", s.seed, |w| {
                w.string("type", "minecraft:multi_noise");
                w.string("preset", "minecraft:overworld");
            });
        }
        Surroundings::Void => {
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
            w.end();
            w.end();
            w.end();
        }
    }
    noise_dimension(&mut w, "minecraft:the_nether", "minecraft:nether", s.seed, |w| {
        w.string("type", "minecraft:multi_noise");
        w.string("preset", "minecraft:nether");
    });
    noise_dimension(&mut w, "minecraft:the_end", "minecraft:end", s.seed, |w| {
        w.string("type", "minecraft:the_end");
        w.long("seed", s.seed);
    });
    w.end(); // dimensions
    w.end(); // WorldGenSettings

    w.end(); // Data
    w.finish()
}

/// The gzip-compressed bytes of `level.dat`.
pub fn level_dat(settings: &LevelSettings) -> io::Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&level_nbt(settings))?;
    gz.finish()
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
        }
    }

    #[test]
    fn level_dat_is_gzipped_nbt_with_the_expected_fields() {
        let bytes = level_dat(&settings(Surroundings::Vanilla)).unwrap();
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
    }

    #[test]
    fn void_surroundings_use_an_empty_flat_generator() {
        let root: HashMap<String, Value> =
            fastnbt::from_bytes(&level_nbt(&settings(Surroundings::Void))).unwrap();
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
