//! Climate: cold with height, a rain shadow behind a ridge, damp by the
//! sea, the right biomes.

use massif_terrain::compile::{Compiled, compile, compile_in};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::kernel::{Env, Grid};
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space};
use std::sync::Arc;

fn compiled(json: &str) -> Compiled {
    let (c, diags) = compile(&Graph::from_json(json).unwrap(), Space::new(2048), 512);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn grid(c: &Compiled, id: &str, port: Option<&str>, lod: u8) -> Grid {
    Engine::new(Settings::default()).whole(c, c.source(id, port).unwrap(), lod, &never).unwrap()
}

#[test]
fn colder_with_height_and_toward_the_cold_side() {
    let c = compiled(r#"{ "nodes": {
        "low": { "type": "pattern.constant", "params": { "value": 62 } },
        "high": { "type": "pattern.constant", "params": { "value": 162 } },
        "t_low": { "type": "climate.temperature", "inputs": { "height": "low" }, "params": { "variation": 0, "toward": 90, "warm": 30, "cold": -10 } },
        "t_high": { "type": "climate.temperature", "inputs": { "height": "high" }, "params": { "variation": 0, "toward": 90, "warm": 30, "cold": -10 } }
    } }"#);
    let (lo, hi) = (grid(&c, "t_low", Some("celsius"), 3), grid(&c, "t_high", Some("celsius"), 3));
    // 100 blocks higher, 5 °C colder.
    assert!((lo.at(100, 100) - hi.at(100, 100) - 5.0).abs() < 1e-3);
    // The south (high z) is warm, the north cold.
    assert!((lo.at(128, 255) - 30.0).abs() < 0.5 && (lo.at(128, 0) + 10.0).abs() < 0.5, "{} {}", lo.at(128, 255), lo.at(128, 0));
    let mask = grid(&c, "t_low", None, 3);
    assert!((mask.at(128, 255) - (60.0 / 75.0)).abs() < 0.01);
}

#[test]
fn a_ridge_casts_a_rain_shadow() {
    // A north–south ridge in the middle, the wind blowing east over it.
    let c = compiled(r#"{ "nodes": {
        "ridge": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "profile": [[0, 0], [0.5, 1], [1, 0]], "height": 200, "base": 70 } },
        "rain": { "type": "climate.rainfall", "inputs": { "height": "ridge" }, "params": { "direction": 0 } }
    } }"#);
    let rain = grid(&c, "rain", None, 2);
    let windward: f32 = (60..120).map(|x| rain.at(x, 256)).sum::<f32>() / 60.0;
    let lee: f32 = (392..452).map(|x| rain.at(x, 256)).sum::<f32>() / 60.0;
    assert!(windward > 2.0 * lee, "windward {windward}, lee {lee}");
    let (lo, hi) = rain.range();
    assert!(lo >= 0.0 && hi <= 1.0);
}

#[test]
fn damp_by_the_sea_dry_inland() {
    // Sea in the west, land rising to the east.
    let c = compiled(r#"{ "nodes": {
        "land": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "height": 80, "base": 30 } },
        "hum": { "type": "climate.humidity", "inputs": { "height": "land" }, "params": { "inland": 300, "variation": 0 } }
    } }"#);
    let h = grid(&c, "hum", None, 2);
    // The shore is about 820 blocks from the west edge (sample 205).
    assert!(h.at(100, 200) > 0.99, "at sea");
    assert!(h.at(215, 200) > 0.8, "by the shore: {}", h.at(215, 200));
    // 1180 blocks from the shore.
    assert!(h.at(500, 200) < 0.03, "far inland: {}", h.at(500, 200));
}

#[test]
fn the_wind_goes_round_high_ground() {
    let c = compiled(r#"{ "nodes": {
        "flat": { "type": "pattern.constant", "params": { "value": 70 } },
        "cone": { "type": "pattern.gradient", "params": { "kind": "radial", "ends": [[0.5, 0.5], [0.6, 0.5]], "profile": [[0, 1], [1, 0]], "height": 150, "base": 70 } },
        "calm": { "type": "climate.wind", "inputs": { "height": "flat" }, "params": { "direction": 0, "strength": 2 } },
        "round": { "type": "climate.wind", "inputs": { "height": "cone" }, "params": { "direction": 0, "deflection": 1 } }
    } }"#);
    let calm = grid(&c, "calm", None, 3);
    assert_eq!(calm.planes, 2);
    assert!((calm.at_plane(0, 50, 50) - 2.0).abs() < 1e-5 && calm.at_plane(1, 50, 50).abs() < 1e-5);
    // North of the cone's centre, on its upwind flank, the wind turns
    // north (−z) to go round.
    let round = grid(&c, "round", None, 3);
    let (x, z) = (128 - 10, 128 - 6);
    assert!(round.at_plane(1, x, z) < -0.2, "{}", round.at_plane(1, x, z));
}

/// The ids of a world's biomes: their place in the climate table, plus one.
struct Biomes;

impl Env for Biomes {
    fn biome_id(&self, name: &str) -> Option<u16> {
        massif_terrain_biomes().iter().position(|id| *id == name).map(|i| i as u16 + 1)
    }
}

fn massif_terrain_biomes() -> Vec<&'static str> {
    vec!["minecraft:warm_ocean", "minecraft:desert", "minecraft:snowy_plains", "minecraft:jungle", "minecraft:stony_peaks", "minecraft:beach", "minecraft:badlands"]
}

#[test]
fn biomes_follow_the_climate() {
    let g = Graph::from_json(r#"{ "nodes": {
        "relief": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "height": 300, "base": 0 } },
        "hot": { "type": "pattern.constant", "params": { "value": 0.85 } },
        "cold": { "type": "pattern.constant", "params": { "value": 0.1 } },
        "dry": { "type": "pattern.constant", "params": { "value": 0.05 } },
        "wet": { "type": "pattern.constant", "params": { "value": 0.9 } },
        "desert": { "type": "climate.biomes", "inputs": { "temperature": "hot", "humidity": "dry", "height": "relief" } },
        "jungle": { "type": "climate.biomes", "inputs": { "temperature": "hot", "humidity": "wet" } },
        "tundra": { "type": "climate.biomes", "inputs": { "temperature": "cold", "humidity": "dry" } }
    } }"#)
    .unwrap();
    let (c, diags) = compile_in(&g, Space::new(2048), 512, Arc::new(Biomes));
    assert!(diags.is_empty(), "{diags:?}");
    let e = Engine::new(Settings::default());
    let id = |node: &str, x: i32| e.region(&c, c.source(node, None).unwrap(), 3, Rect::new(0, 128, 256, 1), &never).unwrap().at(x, 128) as usize;
    let name = |i: usize| if i == 0 { "unknown" } else { massif_terrain_biomes()[i - 1] };
    // The relief rises from 0 (sea) to 300 west to east; the sea level is 62.
    assert_eq!(name(id("desert", 10)), "minecraft:warm_ocean");
    assert_eq!(name(id("desert", 54)), "minecraft:beach");
    assert_eq!(name(id("desert", 80)), "minecraft:desert");
    // Hot, dry and high: badlands.
    assert_eq!(name(id("desert", 120)), "minecraft:badlands");
    assert_eq!(name(id("desert", 250)), "minecraft:stony_peaks");
    assert_eq!(name(id("jungle", 120)), "minecraft:jungle");
    assert_eq!(name(id("tundra", 120)), "minecraft:snowy_plains");
    // Unknown to the world: 0, which lets the exporter decide.
    let colour = e.region(&c, c.source("desert", Some("color")).unwrap(), 3, Rect::new(0, 128, 256, 1), &never).unwrap();
    assert_eq!(colour.planes, 3);
}
