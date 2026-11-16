//! Massif's level.dat against the ones the game itself wrote on this
//! computer: the worlds of the official launcher's saves and of the
//! modpacks' instances (CurseForge, Prism, Modrinth…), read only. For each
//! version of the game found among them (from 1.20, whose reading of
//! level.dat logs an error for a key it wants and does not find), every key
//! Massif writes in `Data` is one the game writes too, and every key the
//! game writes that it reads with an error when missing is in Massif's: the
//! dragon fight, as the game has it before anyone goes to the End.
//! Nothing runs where no such world is found (not in CI).

use fastnbt::Value;
use massif_anvil::level_dat::{LevelSettings, level_nbt};
use massif_anvil::{Difficulty, GameMode, Surroundings, Target, WorldBorder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The keys the game reads from level.dat with an error in its log when
/// they are missing, from 1.20 (each read through a codec and
/// `resultOrPartial(LOGGER::error)`).
const WANTED: &[&str] = &["DragonFight", "WorldGenSettings", "DataPacks"];

/// Keys the game reads but writes only when they are not at their
/// default: the features turned on, which a world without experiments
/// leaves out (Massif writes the default, the vanilla features).
const READ_NOT_WRITTEN: &[&str] = &["enabled_features"];

fn compound(v: &Value) -> &HashMap<String, Value> {
    match v {
        Value::Compound(c) => c,
        other => panic!("expected a compound, got {other:?}"),
    }
}

/// The level.dat files of the worlds on this computer.
fn worlds() -> Vec<PathBuf> {
    let mut saves: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        saves.push(home.join("Library/Application Support/minecraft/saves"));
        saves.push(home.join(".minecraft/saves"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        saves.push(appdata.join(".minecraft/saves"));
    }
    saves.extend(massif_assets::instance::find_instance_dirs().into_iter().map(|d| d.join("saves")));
    let mut out = Vec::new();
    for dir in saves {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let level = entry.path().join("level.dat");
            if !entry.file_name().to_string_lossy().starts_with("._") && level.is_file() {
                out.push(level);
            }
        }
    }
    out
}

/// A level.dat's `Data`, when it reads.
fn data_of(path: &Path) -> Option<HashMap<String, Value>> {
    let bytes = std::fs::read(path).ok()?;
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut raw).ok()?;
    let mut root: HashMap<String, Value> = fastnbt::from_bytes(&raw).ok()?;
    match root.remove("Data")? {
        Value::Compound(data) => Some(data),
        _ => None,
    }
}

#[test]
fn level_dat_has_what_the_game_writes() {
    let settings = LevelSettings {
        name: "Massif".into(),
        seed: 7,
        spawn: [0, 80, 0],
        game_mode: GameMode::Creative,
        allow_commands: true,
        surroundings: Surroundings::Vanilla,
        generate_structures: true,
        last_played: 0,
        difficulty: Difficulty::Normal,
        hardcore: false,
        border: WorldBorder { center: [0.0, 0.0], size: 60_000_000.0 },
        datapacks: Vec::new(),
    };
    let mut checked: HashMap<String, PathBuf> = HashMap::new();
    for path in worlds() {
        let Some(game) = data_of(&path) else { continue };
        let Some(Value::Compound(version)) = game.get("Version") else { continue };
        let Some(Value::String(name)) = version.get("Name") else { continue };
        // Worlds the game saved, of a release Massif exports for, from 1.20
        // and before 26.1 splits level.dat: not those WorldPainter wrote.
        let Some(target) = Target::release(name) else { continue };
        if !(3459..4772).contains(&target.data_version) || game.get("DataVersion") != Some(&Value::Int(target.data_version)) || !game.contains_key("ServerBrands") || checked.contains_key(name) {
            continue;
        }
        let raw = level_nbt(&settings, &target);
        let ours: HashMap<String, Value> = fastnbt::from_bytes(&raw).unwrap();
        let ours = compound(&ours["Data"]);
        let foreign: Vec<&String> = ours.keys().filter(|k| !game.contains_key(*k) && !READ_NOT_WRITTEN.contains(&k.as_str())).collect();
        assert!(foreign.is_empty(), "{name}: keys the game does not write: {foreign:?} ({})", path.display());
        for key in WANTED {
            assert_eq!(ours.contains_key(*key), game.contains_key(*key), "{name}: {key} ({})", path.display());
        }
        // The dragon fight as the game has it before anyone goes to the
        // End: its gateways come when the End is first made.
        let (fight, theirs) = (compound(&ours["DragonFight"]), compound(&game["DragonFight"]));
        if theirs.get("NeedsStateScanning") == Some(&Value::Byte(1)) {
            for (k, v) in fight {
                assert_eq!(theirs.get(k), Some(v), "{name}: DragonFight.{k} ({})", path.display());
            }
        }
        eprintln!("{name}: level.dat checked against {}", path.display());
        checked.insert(name.clone(), path);
    }
    if checked.is_empty() {
        eprintln!("no world saved by Minecraft 1.20 to 1.21.11 here: nothing to compare");
    }
}
