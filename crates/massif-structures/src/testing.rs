//! A tiny village and the means to make templates by hand: for the tests
//! of this crate and of the app, which cannot count on the game being
//! installed.

use crate::jigsaw::Spec;
use crate::library::Library;
use crate::nbt::{self, Compound};
use crate::processor::Heightmap;
use crate::source::Source;
use fastnbt::Value;
use std::collections::HashMap;

    /// The NBT of a template of `size` holding `blocks` (`(pos, state,
/// nbt)`).
pub fn template_bytes(size: [i32; 3], blocks: &[([i32; 3], &str, Option<Compound>)], entities: &[([f64; 3], Compound)]) -> Vec<u8> {
    let mut palette: Vec<String> = Vec::new();
    let mut list = Vec::new();
    for (pos, state, data) in blocks {
        let k = palette.iter().position(|p| p == state).unwrap_or_else(|| {
            palette.push(state.to_string());
            palette.len() - 1
        });
        let mut b: Compound = Compound::new();
        b.insert("pos".into(), Value::List(pos.iter().map(|&v| Value::Int(v)).collect()));
        b.insert("state".into(), Value::Int(k as i32));
        if let Some(d) = data {
            b.insert("nbt".into(), Value::Compound(d.clone()));
        }
        list.push(Value::Compound(b));
    }
    let entry = |s: &String| {
        let (name, props) = s.split_once('[').map_or((s.as_str(), ""), |(n, p)| (n, p.trim_end_matches(']')));
        let mut c = Compound::new();
        c.insert("Name".into(), Value::String(name.to_owned()));
        let props: Compound = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_owned(), Value::String(v.to_owned()))).collect();
        if !props.is_empty() {
            c.insert("Properties".into(), Value::Compound(props));
        }
        Value::Compound(c)
    };
    let mut root = Compound::new();
    root.insert("size".into(), Value::List(size.iter().map(|&v| Value::Int(v)).collect()));
    root.insert("palette".into(), Value::List(palette.iter().map(entry).collect()));
    root.insert("blocks".into(), Value::List(list));
    root.insert(
        "entities".into(),
        Value::List(
            entities
                .iter()
                .map(|(pos, data)| {
                    let mut e = Compound::new();
                    e.insert("pos".into(), nbt::doubles(*pos));
                    e.insert("blockPos".into(), Value::List(pos.iter().map(|&v| Value::Int(v.floor() as i32)).collect()));
                    e.insert("nbt".into(), Value::Compound(data.clone()));
                    Value::Compound(e)
                })
                .collect(),
        ),
    );
    root.insert("DataVersion".into(), Value::Int(4671));
    fastnbt::to_bytes(&Value::Compound(root)).unwrap()
}

/// A jigsaw's block entity.
pub fn jig(name: &str, target: &str, pool: &str, final_state: &str) -> Option<Compound> {
    let mut c = Compound::new();
    for (k, v) in [("name", name), ("target", target), ("pool", pool), ("final_state", final_state), ("joint", "aligned"), ("id", "minecraft:jigsaw")] {
        c.insert(k.into(), Value::String(v.into()));
    }
    Some(c)
}

    /// A tiny village: a square with a jigsaw on each side, streets that
/// carry on, houses at their ends.
pub fn village() -> Library {
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut add = |path: &str, bytes: Vec<u8>| {
        files.insert(path.to_owned(), bytes);
    };
    // The square: 9 × 9, a jigsaw on the middle of each side at y 0.
    let mut square = Vec::new();
    for x in 0..9 {
        for z in 0..9 {
            square.push(([x, 0, z], "minecraft:cobblestone", None));
        }
    }
    square.retain(|b| !matches!(b.0, [4, 0, 0] | [8, 0, 4] | [4, 0, 8] | [0, 0, 4]));
    for (pos, facing) in [([4, 0, 0], "north"), ([8, 0, 4], "east"), ([4, 0, 8], "south"), ([0, 0, 4], "west")] {
        square.push((pos, Box::leak(format!("minecraft:jigsaw[orientation={facing}_up]").into_boxed_str()) as &str, jig("t:street", "t:street", "t:streets", "minecraft:cobblestone")));
    }
    square.push(([4, 1, 4], "minecraft:bell[attachment=floor,facing=north,powered=false]", None));
    add("data/t/structure/square.nbt", template_bytes([9, 3, 9], &square, &[]));
    // A street: 3 wide, 12 long along z, joining at its north end and
    // carrying on south; a house's jigsaw on its east side.
    let mut street = Vec::new();
    for x in 0..3 {
        for z in 0..12 {
            street.push(([x, 0, z], "minecraft:dirt_path", None));
        }
    }
    street.retain(|b| !matches!(b.0, [1, 0, 0] | [1, 0, 11] | [2, 0, 6]));
    street.push(([1, 0, 0], "minecraft:jigsaw[orientation=north_up]", jig("t:street", "t:street", "t:streets", "minecraft:dirt_path")));
    street.push(([1, 0, 11], "minecraft:jigsaw[orientation=south_up]", jig("t:street", "t:street", "t:streets", "minecraft:dirt_path")));
    street.push(([2, 0, 6], "minecraft:jigsaw[orientation=east_up]", jig("t:house", "t:door", "t:houses", "minecraft:dirt_path")));
    add("data/t/structure/street.nbt", template_bytes([3, 1, 12], &street, &[]));
    // A house: 5 × 5, 4 high, its door on its west side at y 1.
    let mut house = Vec::new();
    for x in 0..5 {
        for z in 0..5 {
            house.push(([x, 0, z], "minecraft:cobblestone", None));
            for y in 1..4 {
                if x == 0 || x == 4 || z == 0 || z == 4 {
                    house.push(([x, y, z], "minecraft:oak_planks", None));
                } else {
                    house.push(([x, y, z], "minecraft:air", None));
                }
            }
        }
    }
    house.retain(|b| b.0 != [0, 1, 2]);
    house.push(([0, 1, 2], "minecraft:jigsaw[orientation=west_up]", jig("t:door", "t:house", "minecraft:empty", "minecraft:air")));
    house.push((
        [2, 1, 2],
        "minecraft:chest[facing=north,type=single,waterlogged=false]",
        Some([("LootTable".to_owned(), fastnbt::Value::String("minecraft:chests/village/village_plains_house".into())), ("id".to_owned(), fastnbt::Value::String("minecraft:chest".into()))].into_iter().collect()),
    ));
    let mut villager = crate::nbt::Compound::new();
    villager.insert("id".into(), fastnbt::Value::String("minecraft:villager".into()));
    villager.insert("Rotation".into(), fastnbt::Value::List(vec![fastnbt::Value::Float(0.0), fastnbt::Value::Float(0.0)]));
    add("data/t/structure/house.nbt", template_bytes([5, 4, 5], &house, &[([1.5, 1.0, 1.5], villager)]));
    let pool = |elements: &[(&str, &str, u32)], fallback: &str| {
        let list: Vec<String> = elements
            .iter()
            .map(|(loc, proj, w)| format!(r#"{{"weight": {w}, "element": {{"element_type": "minecraft:legacy_single_pool_element", "location": "{loc}", "projection": "{proj}", "processors": {{"processors": []}}}}}}"#))
            .collect();
        format!(r#"{{"fallback": "{fallback}", "elements": [{}]}}"#, list.join(",")).into_bytes()
    };
    add("data/t/worldgen/template_pool/centers.json", pool(&[("t:square", "rigid", 1)], "minecraft:empty"));
    add("data/t/worldgen/template_pool/streets.json", pool(&[("t:street", "terrain_matching", 1)], "t:ends"));
    add("data/t/worldgen/template_pool/ends.json", br#"{"fallback": "minecraft:empty", "elements": [{"weight": 1, "element": {"element_type": "minecraft:empty_pool_element"}}]}"#.to_vec());
    add("data/t/worldgen/template_pool/houses.json", pool(&[("t:house", "rigid", 1)], "minecraft:empty"));
    // As the game's plains villages, so that the one-click kind finds it.
    add("data/minecraft/worldgen/structure/village_plains.json", br#"{"type": "minecraft:jigsaw", "start_pool": "t:centers", "size": 4, "max_distance_from_center": 60, "project_start_to_heightmap": "WORLD_SURFACE_WG", "start_height": {"absolute": 0}, "use_expansion_hack": true, "terrain_adaptation": "beard_thin"}"#.to_vec());
    Library::new(Source::Memory(files), "test")
}

pub fn village_spec() -> Spec {
    Spec { start_pool: "t:centers".into(), start_jigsaw: None, depth: 4, max_distance: 60, expansion_hack: true, project: Some(Heightmap::WorldSurface), padding: (0, 0), aliases: Vec::new() }
}

