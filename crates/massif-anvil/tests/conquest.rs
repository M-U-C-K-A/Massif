//! Conquest Reforged's blocks, exported for the version it plays (1.20.1)
//! and read back with `fastanvil`, an independent Anvil reader: every block
//! state written must be one the game has.
//!
//! Always with a fixture made as Conquest's files are (its list for
//! WorldPainter, its registry's lists). With its real instance too, when
//! `MASSIF_CONQUEST_DIR` names it or a folder of instances (the one with
//! Conquest's lists is read; the test reads the player's own game, so it
//! runs only when asked): then a hundred of its real blocks are painted as
//! terrains, mixes and patterns, its plants grown over its own ground, and
//! its shaped blocks placed in turned and mirrored objects; each state is
//! checked against the jar's block states, the mod's list for WorldPainter
//! and the registry's list of every state (`states-pls.txt`).
//! `MASSIF_CONQUEST_CHECK=<dir>` checks a world exported elsewhere (by the
//! app) the same way.

use fastanvil::Region;
use massif_anvil::{ExportOptions, Progress, Target, export};
use massif_assets::materials::{self, Definition};
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::objects::{ObjectDef, ObjectLayerDef};
use massif_core::ops;
use massif_core::pattern::Pattern;
use massif_core::registry::{CoverDef, Mix, Stratum, TerrainDef, TerrainId};
use massif_core::{Brush, BrushShape, Profile, TilePos, World};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

fn hard(radius: f32) -> Brush {
    Brush {
        shape: BrushShape::Circle,
        radius,
        profile: Profile::Constant,
        strength: 1.0,
        rotation: 0.0,
        mask: None,
        filter: None,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-anvil-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A state's name and properties: `conquest:x[a=1,b=2]`.
fn split(state: &str) -> (String, Vec<(String, String)>) {
    let (name, props) = state.split_once('[').unwrap_or((state, ""));
    let props = props.trim_end_matches(']').split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
    (name.to_owned(), props)
}

/// What the game has: Conquest's blocks with their properties, and, when
/// read, every state of each.
struct Truth {
    definitions: HashMap<String, Definition>,
    /// The blocks the game registers, of `conquest`.
    registered: HashSet<String>,
    /// Every state of the blocks used, as the registry lists them.
    states: HashMap<String, Vec<Vec<(String, String)>>>,
    /// The blocks the jars have files for.
    files: Option<HashSet<String>>,
}

impl Truth {
    /// Whether a state is the game's: its block registered, each property
    /// one of the block's with one of its values, and (when every state is
    /// listed) one of its states having them all.
    fn check(&self, name: &str, props: &[(String, String)]) -> Result<(), String> {
        if !name.starts_with("conquest:") {
            return Ok(());
        }
        if !self.registered.contains(name) {
            return Err(format!("{name} is not registered"));
        }
        if let Some(files) = &self.files
            && !files.contains(name)
        {
            return Err(format!("{name} has no block state file"));
        }
        let state = format!("{name}[{}]", props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","));
        if self.definitions.contains_key(name) {
            materials::check(&state, &self.definitions).map_err(|e| e.to_string())?;
        }
        if let Some(all) = self.states.get(name)
            && !all.iter().any(|s| props.iter().all(|p| s.contains(p)))
        {
            return Err(format!("{state} is none of the {} states of the registry", all.len()));
        }
        Ok(())
    }
}

/// Every block state of an exported world's overworld, with how many
/// sections hold it; and checks that each chunk is 1.20.1's.
fn palette_states(dir: &Path, data_version: i32) -> BTreeMap<String, usize> {
    use fastnbt::Value;
    let mut out = BTreeMap::new();
    let regions = dir.join("region");
    for entry in std::fs::read_dir(&regions).unwrap().flatten() {
        if entry.path().extension().is_none_or(|e| e != "mca") || entry.file_name().to_string_lossy().starts_with("._") {
            continue;
        }
        let mut region = Region::from_stream(std::fs::File::open(entry.path()).unwrap()).unwrap();
        for chunk in region.iter().flatten() {
            let Value::Compound(root) = fastnbt::from_bytes(&chunk.data).unwrap() else { panic!("a chunk is a compound") };
            assert_eq!(root["DataVersion"], Value::Int(data_version), "the chunk is 1.20.1's");
            let Some(Value::List(sections)) = root.get("sections") else { continue };
            for section in sections {
                let Value::Compound(section) = section else { continue };
                let Some(Value::Compound(states)) = section.get("block_states") else { continue };
                let Some(Value::List(palette)) = states.get("palette") else { continue };
                for entry in palette {
                    let Value::Compound(entry) = entry else { panic!("a palette entry is a compound") };
                    let Value::String(name) = &entry["Name"] else { panic!("a block has a name") };
                    let mut props: Vec<String> = match entry.get("Properties") {
                        Some(Value::Compound(p)) => p
                            .iter()
                            .map(|(k, v)| match v {
                                Value::String(v) => format!("{k}={v}"),
                                other => panic!("{name}: property {k} is not a string: {other:?}"),
                            })
                            .collect(),
                        None => Vec::new(),
                        Some(other) => panic!("{name}: Properties is {other:?}"),
                    };
                    props.sort();
                    let state = if props.is_empty() { name.clone() } else { format!("{name}[{}]", props.join(",")) };
                    *out.entry(state).or_insert(0) += 1;
                }
            }
        }
    }
    out
}

/// Check every Conquest state of a world against the truth; returns the
/// Conquest states written.
fn check_world(dir: &Path, truth: &Truth) -> Vec<String> {
    let target = Target::release("1.20.1").unwrap();
    let states = palette_states(dir, target.data_version);
    let mut bad = Vec::new();
    let mut conquest = Vec::new();
    for state in states.keys() {
        let (name, props) = split(state);
        if !name.contains(':') {
            bad.push(format!("{state}: no namespace"));
        }
        if name.starts_with("conquest:") {
            conquest.push(state.clone());
            if let Err(e) = truth.check(&name, &props) {
                bad.push(e);
            }
        }
    }
    assert!(bad.is_empty(), "{} states are not the game's:\n{}", bad.len(), bad.join("\n"));
    conquest
}

/// A terrain of one block, a few deep.
fn terrain(w: &mut World, name: &str, blocks: &[(&str, u16)], mix: Mix) -> TerrainId {
    let reg = &mut w.registry;
    let blocks = blocks.iter().map(|&(b, weight)| (reg.intern_block(b, [128, 128, 128]), weight)).collect();
    reg.terrains.push(TerrainDef {
        name: name.to_owned(),
        group: "Conquest Reforged".to_owned(),
        color: [128, 128, 128],
        strata: vec![Stratum { blocks, depth: (3, 5), mix }],
        submerged_top: None,
        biome_hint: None,
    });
    TerrainId((reg.terrains.len() - 1) as u16)
}

/// What a world shows of Conquest: its ground blocks, the plants grown over
/// them, and the shaped blocks of an object.
struct Palette<'a> {
    /// Whole blocks, each painted as a terrain.
    ground: Vec<&'a str>,
    /// Soil of the mod, under the plants.
    soil: Vec<&'a str>,
    /// Plants, each a cover (a tall one as its lower half).
    plants: Vec<String>,
    /// States of shaped blocks, stood in an object turned every way.
    shapes: Vec<String>,
}

/// A map of 256 × 256 blocks painted with a palette of Conquest: a disc of
/// each ground block, a mix in patches and one in beds, a pattern of three,
/// its plants on its soil, and an object of its shaped blocks standing
/// turned and mirrored.
fn conquest_world(p: &Palette) -> World {
    let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 11, ..GenParams::default() });
    w.meta.ores.on = false;
    let grounds: Vec<TerrainId> = p.ground.iter().map(|b| terrain(&mut w, b, &[(b, 1)], Mix::Grain)).collect();
    let soils: Vec<TerrainId> = p.soil.iter().map(|b| terrain(&mut w, b, &[(b, 1)], Mix::Grain)).collect();
    let mixed: Vec<(&str, u16)> = p.ground.iter().take(4).enumerate().map(|(k, b)| (*b, 1 + k as u16)).collect();
    let patches = terrain(&mut w, "Patches", &mixed, Mix::Patches { size: 4.0 });
    let beds = terrain(&mut w, "Beds", &mixed[..mixed.len().min(3)], Mix::Beds);
    let covers: Vec<u16> = p
        .plants
        .iter()
        .map(|plant| {
            let mut def = CoverDef::new(plant, [80, 140, 60], &[(plant.as_str(), 1)], 1.0);
            def.soil_only = true;
            w.registry.add_cover(def)
        })
        .collect();
    // The shaped blocks in a row, two blocks apart, in an object.
    let n = p.shapes.len().max(1);
    let (sx, sz) = (2 * n.min(12) - 1, 2 * n.div_ceil(12) - 1);
    let mut palette = vec!["minecraft:air".to_owned()];
    palette.extend(p.shapes.iter().cloned());
    let mut blocks = vec![0u16; sx * sz];
    for k in 0..p.shapes.len() {
        blocks[(k % 12) * 2 + (k / 12) * 2 * sx] = 1 + k as u16;
    }
    let object = w.registry.objects.len() as u32;
    w.registry.objects.push(ObjectDef::new("Conquest shapes", [sx as u16, 1, sz as u16], palette, blocks));
    let mut layer = ObjectLayerDef::new("Conquest shapes", &[object], &w.registry);
    layer.spacing = (sx.max(sz) + 4) as u16;
    layer.density = 1.0;
    layer.rotate = true;
    layer.mirror = true;
    layer.soil_only = false;
    let objects = w.registry.add_object_layer(layer);

    let mut h = History::default();
    h.begin("conquest", 0);
    {
        let mut e = h.edit(&mut w, 0);
        // Ground blocks on a grid in the north half, 12 a row.
        for (k, t) in grounds.iter().enumerate() {
            let (x, z) = (-120.0 + (k % 12) as f32 * 20.0, -120.0 + (k / 12) as f32 * 20.0);
            ops::paint_terrain(&mut e, &hard(7.0), x, z, *t, k as u64 + 1);
        }
        ops::paint_terrain(&mut e, &hard(14.0), -100.0, 40.0, patches, 900);
        ops::paint_terrain(&mut e, &hard(14.0), -60.0, 40.0, beds, 901);
        // A pattern of three ground blocks.
        let three: Vec<(TerrainId, f32)> = grounds.iter().take(3).map(|t| (*t, 1.0)).collect();
        if three.len() == 3 {
            let sampler = Pattern::default().sampler(&three);
            ops::paint_pattern(&mut e, &hard(14.0), -20.0, 40.0, 902, &sampler);
        }
        // Each plant over the mod's soil, in the south half.
        for (k, c) in covers.iter().enumerate() {
            let (x, z) = (-120.0 + (k % 12) as f32 * 20.0, 70.0 + (k / 12) as f32 * 20.0);
            if !soils.is_empty() {
                ops::paint_terrain(&mut e, &hard(7.0), x, z, soils[k % soils.len()], 1000 + k as u64);
            }
            ops::paint_cover(&mut e, &hard(6.0), x, z, *c, 2000 + k as u64);
        }
        ops::paint_objects(&mut e, &hard(30.0), 60.0, 40.0, objects, 3000);
    }
    h.commit();
    w
}

/// Export for 1.20.1 and check the world against the truth: every state
/// the game's, and what was painted, grown and placed in it.
fn export_and_check(name: &str, p: &Palette, truth: &Truth) -> Vec<String> {
    let mut w = conquest_world(p);
    let dir = temp_dir(name);
    let options = ExportOptions { target: Target::release("1.20.1").unwrap(), ..ExportOptions::default() };
    export(&w.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let written = check_world(&dir, truth);
    let names: HashSet<String> = written.iter().map(|s| split(s).0).collect();
    for b in p.ground.iter().chain(&p.soil) {
        assert!(names.contains(*b), "{b} was painted but not written");
    }
    for plant in &p.plants {
        let (name, props) = split(plant);
        assert!(names.contains(&name), "{plant} was grown but not written");
        // A tall plant stands in its two halves.
        if props.iter().any(|(k, v)| k == "half" && v == "lower") {
            for half in ["lower", "upper"] {
                assert!(written.iter().any(|s| s.starts_with(&format!("{name}[")) && s.contains(&format!("half={half}"))), "{name}: no {half} half");
            }
        }
    }
    let shapes: HashSet<String> = p.shapes.iter().map(|s| split(s).0).collect();
    let placed = shapes.iter().filter(|s| names.contains(*s)).count();
    assert!(placed * 10 >= shapes.len() * 9, "{placed} of the object's {} shaped blocks were written", shapes.len());
    let _ = std::fs::remove_dir_all(&dir);
    written
}

/// Conquest's list for WorldPainter, in small, as the mod writes it.
const FIXTURE: &str = "name,discriminator,properties,opacity,receivesLight,insubstantial,resource,tileEntity,tileEntityId,treeRelated,vegetation,blockLight,natural,watery,colour,horizontal_orientation_schemes,vertical_orientation_scheme
conquest:worn_dark_lime_mortar_masonry,,,15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:limestone_cobble,,,15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:granite_brick,,,15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:dark_fertile_soil,,,15,false,false,false,false,false,false,false,0,true,false,9923917,,
conquest:limestone_small_arch,,\"direction:e[northsouth;eastwest],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:limestone_capital,,\"facing:e[flat;north;south;east;west],type:e[top;bottom],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:limestone_slab,,\"layers:i[1-8],type:e[top;bottom],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:oak_beam,,\"shape:e[one;two_l;two_r;three_l;three_r;three_middle],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,9402184,,
conquest:dead_bush_tall,,\"half:e[upper;lower]\",0,false,true,false,false,false,false,true,0,true,false,31744,,
conquest:timothy_grass,,,0,false,true,false,false,false,false,true,0,true,false,31744,,
";

/// Every state of the blocks of a list: each value of each property, all
/// together.
fn all_states(defs: &HashMap<String, Definition>) -> HashMap<String, Vec<Vec<(String, String)>>> {
    defs.iter()
        .map(|(id, def)| {
            let mut states: Vec<Vec<(String, String)>> = vec![Vec::new()];
            for p in &def.properties {
                states = states.into_iter().flat_map(|s| p.values.iter().map(move |v| [s.clone(), vec![(p.name.clone(), v.clone())]].concat())).collect();
            }
            (id.clone(), states)
        })
        .collect()
}

/// The fixture: Conquest's ground, soil, plants (one tall) and shaped
/// blocks of every kind of property, written for 1.20.1, each the game's.
#[test]
fn conquest_fixture_exports_its_own_states() {
    let list = materials::parse(FIXTURE).unwrap();
    let definitions: HashMap<String, Definition> = list.blocks.iter().cloned().collect();
    let truth = Truth { states: all_states(&definitions), registered: definitions.keys().cloned().collect(), definitions, files: None };
    let palette = Palette {
        ground: vec!["conquest:worn_dark_lime_mortar_masonry", "conquest:limestone_cobble", "conquest:granite_brick"],
        soil: vec!["conquest:dark_fertile_soil"],
        plants: vec!["conquest:dead_bush_tall[half=lower]".to_owned(), "conquest:timothy_grass".to_owned()],
        shapes: vec![
            "conquest:limestone_small_arch[direction=northsouth,waterlogged=false]".to_owned(),
            "conquest:limestone_capital[facing=north,type=top,waterlogged=false]".to_owned(),
            "conquest:limestone_capital[facing=flat,type=bottom,waterlogged=false]".to_owned(),
            "conquest:limestone_slab[layers=5,type=bottom,waterlogged=false]".to_owned(),
            "conquest:oak_beam[shape=two_l,waterlogged=false]".to_owned(),
            "conquest:oak_beam[shape=three_middle,waterlogged=false]".to_owned(),
        ],
    };
    let written = export_and_check("conquest-fixture", &palette, &truth);
    // The object turned: its arch lies both ways, its beam's ends both
    // sides.
    for want in ["direction=northsouth", "direction=eastwest", "shape=two_l", "shape=two_r", "facing=flat"] {
        assert!(written.iter().any(|s| s.contains(want)), "no {want} among {written:?}");
    }
    // And a state no block has is caught.
    assert!(truth.check("conquest:limestone_slab", &[("layers".into(), "9".into())]).is_err());
    assert!(truth.check("conquest:oak_beam", &[("shape".into(), "east_west".into())]).is_err());
    assert!(truth.check("conquest:cheese_cut", &[]).is_err());
}

/// Conquest Reforged's instance named by MASSIF_CONQUEST_DIR (it, or the
/// best of a folder of them: the one with Conquest's lists of its blocks);
/// `None`, said, without it.
fn conquest_instance() -> Option<massif_assets::instance::Instance> {
    let Some(dir) = std::env::var_os("MASSIF_CONQUEST_DIR").map(PathBuf::from) else {
        eprintln!("MASSIF_CONQUEST_DIR=<a Conquest Reforged instance, or a folder of instances> checks Conquest's own blocks: skipping");
        return None;
    };
    let found = massif_assets::instance::with_mod_in(&dir, "conquest").into_iter().next();
    if found.is_none() {
        eprintln!("no Conquest Reforged instance in {}: skipping", dir.display());
    }
    found
}

/// The truth of the real game: the catalogue read from the instance (its
/// jar's block files, its list for WorldPainter, the registry's list) and
/// every state of the blocks used, from `states-pls.txt`.
fn real_truth(cat: &massif_assets::Catalogue, dir: &Path, used: &HashSet<String>) -> Truth {
    let mut states: HashMap<String, Vec<Vec<(String, String)>>> = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("states-pls.txt")) {
        for line in text.lines() {
            let name = line.split('[').next().unwrap_or(line);
            if used.contains(name) {
                states.entry(name.to_owned()).or_default().push(split(line.trim()).1);
            }
        }
    }
    Truth {
        definitions: cat.definitions.clone(),
        registered: cat.registered.get("conquest").cloned().unwrap_or_else(|| cat.blocks.iter().filter(|b| b.id.starts_with("conquest:")).map(|b| b.id.clone()).collect()),
        states,
        files: Some(cat.blocks.iter().filter(|b| b.id.starts_with("conquest:") && b.top.is_some()).map(|b| b.id.clone()).collect()),
    }
}

/// Conquest Reforged itself: a hundred of its blocks from its creative tabs
/// painted, grown and placed, exported for 1.20.1, each state written the
/// game's.
#[test]
fn conquest_reforged_exports_its_own_states() {
    let Some(inst) = conquest_instance() else { return };
    // Its states are judged by the lists Conquest writes beside them
    // (conquest_wp.csv, blocks-pls.txt, states-pls.txt).
    if inst.lists.is_empty() {
        eprintln!("{} has no lists of Conquest's blocks: skipping", inst.name);
        return;
    }
    let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
    let Some(jar) = massif_assets::find_jars().into_iter().find(|j| j.version == version) else {
        eprintln!("Minecraft {version} is not installed here: skipping");
        return;
    };
    let cat = match massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files()) {
        Ok(cat) => cat,
        Err(e) => {
            eprintln!("{} could not be read ({e}): skipping", inst.name);
            return;
        }
    };
    let m = cat.mods.iter().find(|m| m.kind == massif_assets::SourceKind::Mod && m.namespaces.iter().any(|n| n == "conquest")).unwrap();
    let block = |id: &str| cat.block(id).filter(|b| b.id.starts_with("conquest:"));
    let group = |name: &str| m.groups.iter().find(|g| g.name.eq_ignore_ascii_case(name)).map(|g| g.blocks.iter().filter_map(|id| block(id)).collect::<Vec<_>>()).unwrap_or_default();
    // Whole blocks of its ground and masonry tabs, a few of each.
    let mut ground: Vec<&str> = Vec::new();
    for tab in ["Stone", "Sand And Gravel", "Cobble And Brick", "Advanced Masonry", "Plaster Stucco And Paint", "Roofing", "Planks And Beams", "Logs", "Mosaics Tiles And Floors"] {
        let cubes: Vec<&str> = group(tab).into_iter().filter(|b| b.full_cube && !massif_core::registry::is_soil(&b.id)).map(|b| b.id.as_str()).collect();
        let step = (cubes.len() / 9).max(1);
        ground.extend(cubes.iter().step_by(step).take(9));
    }
    let soil: Vec<&str> = group("Grass And Dirt").into_iter().filter(|b| b.full_cube && massif_core::registry::is_soil(&b.id)).map(|b| b.id.as_str()).take(12).collect();
    // Plants of its tabs, the tall ones as their lower halves.
    let mut plants: Vec<String> = Vec::new();
    for tab in ["Grasses And Shrubs", "Flowers", "Crops And Herbs"] {
        let list: Vec<&massif_assets::Block> = group(tab).into_iter().filter(|b| !b.full_cube).collect();
        let step = (list.len() / 8).max(1);
        plants.extend(list.iter().step_by(step).take(8).map(|b| if b.tall { format!("{}[half=lower]", b.id) } else { b.id.clone() }));
    }
    for tall in cat.blocks.iter().filter(|b| b.id.starts_with("conquest:") && b.tall && b.category == massif_assets::Category::Plants).take(4) {
        plants.push(format!("{}[half=lower]", tall.id));
    }
    // Shaped blocks of every kind of property the mod has: the first block
    // with each property, its other properties at their first value.
    let mut shapes: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut defs: Vec<(&String, &Definition)> = cat.definitions.iter().collect();
    defs.sort_by(|a, b| a.0.cmp(b.0));
    for (id, def) in defs {
        let key: Vec<String> = def.properties.iter().map(|p| format!("{}:{}", p.name, p.values.join(";"))).collect();
        if def.properties.is_empty() || def.property("half").is_some_and(|p| p.values.contains(&"lower".to_owned())) || !key.iter().any(|k| seen.insert(k.clone())) || cat.block(id).is_none() {
            continue;
        }
        let props: Vec<String> = def.properties.iter().map(|p| format!("{}={}", p.name, if p.name == "waterlogged" { "false" } else { &p.values[p.values.len() / 2] })).collect();
        shapes.push(format!("{id}[{}]", props.join(",")));
    }
    eprintln!("{} ground blocks, {} soils, {} plants, {} shaped blocks", ground.len(), soil.len(), plants.len(), shapes.len());
    assert!(ground.len() >= 60 && soil.len() >= 6 && plants.len() >= 15 && shapes.len() >= 40);
    let used: HashSet<String> = ground.iter().chain(&soil).map(|s| s.to_string()).chain(plants.iter().chain(&shapes).map(|s| split(s).0)).collect();
    let truth = real_truth(&cat, &inst.dir, &used);
    let palette = Palette { ground, soil, plants, shapes };
    let written = export_and_check("conquest-real", &palette, &truth);
    eprintln!("{} states of Conquest Reforged written, each the game's", written.len());
    if !truth.states.is_empty() {
        assert!(truth.states.len() >= used.len() * 9 / 10, "{} of {} blocks in the registry's list", truth.states.len(), used.len());
    }
}

/// A world exported elsewhere (by the app: `MASSIF_CONQUEST_CHECK=<dir>`,
/// with MASSIF_CONQUEST_DIR), checked against the real game the same way.
#[test]
fn an_exported_conquest_world_is_the_games() {
    let Some(dir) = std::env::var_os("MASSIF_CONQUEST_CHECK").map(PathBuf::from) else { return };
    let Some(inst) = conquest_instance() else { return };
    let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
    let jar = massif_assets::find_jars().into_iter().find(|j| j.version == version).expect("Minecraft 1.20.1");
    let cat = massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files()).unwrap();
    let states = palette_states(&dir, Target::release("1.20.1").unwrap().data_version);
    let used: HashSet<String> = states.keys().map(|s| split(s).0).filter(|n| n.starts_with("conquest:")).collect();
    let truth = real_truth(&cat, &inst.dir, &used);
    let written = check_world(&dir, &truth);
    let blocks: HashSet<String> = written.iter().map(|s| split(s).0).collect();
    eprintln!("{}: {} states of {} Conquest Reforged blocks, each the game's", dir.display(), written.len(), blocks.len());
}

/// Every block of an exported world, column by column of its sections'
/// palettes: the files compared state for state, whatever order the blocks
/// were registered in.
fn region_bytes(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir.join("region"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "mca"))
        .map(|e| {
            // The second sector holds when each chunk was saved: the time of
            // the export, which two exports need not share.
            let mut bytes = std::fs::read(e.path()).unwrap();
            bytes[4096..8192].fill(0);
            (e.file_name().to_string_lossy().into_owned(), bytes)
        })
        .collect();
    out.sort();
    out
}

/// A new map made of Conquest Reforged's palette exports Conquest's own
/// blocks on its surface, beaches, beds and snow, each state one the game
/// has; put back to the game's palette, it exports exactly as a map made
/// vanilla from the start. With MASSIF_CONQUEST_DIR, each state is checked
/// against Conquest's real lists too.
#[test]
fn a_new_map_of_conquest_blocks_exports_them() {
    use massif_core::materials::{self as palettes, Palette as BlockPalette};
    let params = GenParams { tiles_x: 3, tiles_z: 3, landform: Landform::Islands, relief: 400.0, seed: 5, ..GenParams::default() };
    let options = ExportOptions { target: Target::release("1.20.1").unwrap(), ..ExportOptions::default() };
    let mut conquest = generate(&GenParams { palette: BlockPalette::of_mod("conquest").unwrap(), ..params.clone() });
    assert_eq!(conquest.meta.palette.name, "Conquest Reforged");
    let dir = temp_dir("palette-conquest");
    export(&conquest.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let states = palette_states(&dir, options.target.data_version);
    let written: Vec<&String> = states.keys().filter(|s| s.starts_with("conquest:")).collect();
    // Grass, rock, sand, gravel and snow of Conquest's.
    for want in ["conquest:clover_covered_grass", "conquest:gray_limestone", "conquest:tan_sand", "conquest:gray_gravel", "conquest:icy_limestone"] {
        assert!(written.iter().any(|s| s.starts_with(want)), "no {want} among {written:?}");
    }
    // Conquest's own states, by its lists: the fixture's, or its real ones.
    let fixture: HashMap<String, Definition> = materials::parse(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/conquest_wp_palette.csv")).unwrap()).unwrap().blocks.into_iter().collect();
    for s in &written {
        let (name, props) = split(s);
        let state = format!("{name}[{}]", props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(","));
        materials::check(&state, &fixture).unwrap_or_else(|e| panic!("{s}: {e}"));
    }
    if let Some(inst) = conquest_instance()
        && let Some(jar) = massif_assets::find_jars().into_iter().find(|j| Some(&j.version) == inst.minecraft.as_ref())
        && let Ok(cat) = massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files())
    {
        let used: HashSet<String> = written.iter().map(|s| split(s).0).collect();
        let truth = real_truth(&cat, &inst.dir, &used);
        let checked = check_world(&dir, &truth);
        eprintln!("{} Conquest states checked against {}", checked.len(), inst.name);
        // Each a whole block, its colour (for before the game is read) its
        // texture's.
        let conquest = BlockPalette::of_mod("conquest").unwrap();
        let mut colors = BTreeMap::new();
        for state in conquest.states() {
            let id = state.split('[').next().unwrap();
            let block = cat.block(id).filter(|b| b.full_cube).unwrap_or_else(|| panic!("{id} is not a whole block of Conquest's"));
            colors.insert(state.to_owned(), block.color);
        }
        let off: Vec<&String> = colors.iter().filter(|(s, c)| conquest.colors.get(*s).is_none_or(|k| k.iter().zip(c.iter()).any(|(a, b)| a.abs_diff(*b) > 8))).map(|(s, _)| s).collect();
        let listed: Vec<String> = colors.iter().map(|(s, c)| format!("\"{s}\": [{}, {}, {}]", c[0], c[1], c[2])).collect();
        assert!(off.is_empty(), "colours to keep in conquest_terrains.json: {{{}}}", listed.join(", "));
    }
    let _ = std::fs::remove_dir_all(&dir);

    // Back to the game's blocks: the same export as a vanilla map's.
    let mut history = History::default();
    let mut registry = conquest.registry.clone();
    assert!(palettes::apply(&mut registry, &BlockPalette::vanilla()) > 0);
    let meta = massif_core::WorldMeta { palette: BlockPalette::vanilla(), ..conquest.meta.clone() };
    history.replace_settings(&mut conquest, meta, registry, "Blocks: Vanilla");
    let back = temp_dir("palette-back");
    export(&conquest.snapshot(), &back, &options, &Progress::default()).unwrap();
    let mut vanilla = generate(&params);
    let plain = temp_dir("palette-vanilla");
    export(&vanilla.snapshot(), &plain, &options, &Progress::default()).unwrap();
    assert!(palette_states(&back, options.target.data_version).keys().all(|s| !s.starts_with("conquest:")));
    assert_eq!(palette_states(&back, options.target.data_version), palette_states(&plain, options.target.data_version));
    assert!(region_bytes(&back) == region_bytes(&plain), "the regions differ from a vanilla map's");
    // Undone, the map is Conquest's again.
    history.undo(&mut conquest);
    assert_eq!(conquest.meta.palette.name, "Conquest Reforged");
    let _ = (std::fs::remove_dir_all(&back), std::fs::remove_dir_all(&plain));
}

/// Every mod profile's palette (`massif_core::mod_profiles`), whatever the
/// mod: a new map made of it exports for the version its mod is made for,
/// and writes the mod's blocks of the palette, each as the palette states
/// it, and no other block of the mod.
#[test]
fn every_mod_profile_palette_exports_its_blocks() {
    use massif_core::materials::Palette as BlockPalette;
    for profile in massif_core::mod_profiles::all() {
        let Some(palette) = &profile.palette else { continue };
        let version = profile.minecraft.as_deref().unwrap_or("1.21.1");
        let options = ExportOptions { target: Target::release(version).unwrap(), ..ExportOptions::default() };
        let params = GenParams { tiles_x: 3, tiles_z: 3, landform: Landform::Islands, relief: 400.0, seed: 5, palette: BlockPalette::of_mod(&profile.namespace).unwrap(), ..GenParams::default() };
        let mut world = generate(&params);
        assert_eq!(&world.meta.palette, palette);
        let dir = temp_dir(&format!("profile-{}", profile.namespace));
        export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
        let states = palette_states(&dir, options.target.data_version);
        let prefix = format!("{}:", profile.namespace);
        let written: Vec<&String> = states.keys().filter(|s| s.starts_with(&prefix)).collect();
        let wanted: HashSet<(String, Vec<(String, String)>)> = palette.states().iter().map(|s| split(s)).collect();
        assert!(written.len() >= 2, "{}: {written:?}", profile.name);
        for s in &written {
            let (id, mut props) = split(s);
            props.sort();
            assert!(wanted.iter().any(|(w, p)| *w == id && { let mut p = p.clone(); p.sort(); p == props }), "{}: {s} is not of its palette", profile.name);
        }
        eprintln!("{}: {} states of its blocks written for {version}, of the {} its palette uses", profile.name, written.len(), wanted.iter().filter(|(id, _)| id.starts_with(&prefix)).count());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Smooth terrain on Conquest Reforged itself (MASSIF_CONQUEST_DIR): hills
/// of its rock, its soils and the game's grass and sand, smoothed, exported
/// for 1.20.1; every piece written is a state of the game's — layers,
/// slabs by eighths, stairs, vertical slabs and corners —, of its own
/// blocks and for the game's (`grass_block_layer`).
#[test]
fn conquest_smooth_terrain_writes_the_games_states() {
    let Some(inst) = conquest_instance() else { return };
    if inst.lists.is_empty() {
        eprintln!("{} has no lists of Conquest's blocks: skipping", inst.name);
        return;
    }
    let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
    let Some(jar) = massif_assets::find_jars().into_iter().find(|j| j.version == version) else {
        eprintln!("Minecraft {version} is not installed here: skipping");
        return;
    };
    let Ok(cat) = massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files()) else { return };
    let mods: HashMap<String, Vec<String>> = cat
        .definitions
        .iter()
        .filter(|(id, _)| id.starts_with("conquest:"))
        .map(|(id, d)| (id.clone(), d.properties.iter().map(|p| p.name.clone()).collect()))
        .collect();
    let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 11, ..GenParams::default() });
    w.meta.ores.on = false;
    w.meta.smooth.on = true;
    let blocks = ["conquest:gray_limestone", "conquest:mossy_limestone", "minecraft:grass_block", "minecraft:sand", "minecraft:gravel", "minecraft:stone"];
    let terrains: Vec<TerrainId> = blocks.iter().map(|b| terrain(&mut w, b, &[(b, 1)], Mix::Grain)).collect();
    // Bands of each across the hills, and a cliff for vertical slabs.
    let positions: Vec<TilePos> = w.dims[0].ground().positions().collect();
    for pos in positions {
        let (ox, _) = pos.origin();
        let tile = w.dims[0].ground_mut().tile_mut(pos).unwrap();
        for i in 0..massif_core::TILE_AREA {
            let x = ox + (i % 128) as i32;
            tile.terrain.set(i, terrains[((x + 128) / 43).clamp(0, 5) as usize].0);
            tile.cover.set(i, 0);
            tile.objects.set(i, 0);
            if (x + 128) % 43 == 20 {
                tile.top.set(i, tile.top.get(i) + 3.0);
            }
        }
    }
    let dir = temp_dir("conquest-smooth");
    let options = ExportOptions { target: Target::release("1.20.1").unwrap(), mod_blocks: std::sync::Arc::new(mods), ..ExportOptions::default() };
    export(&w.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let used: HashSet<String> = palette_states(&dir, options.target.data_version).keys().map(|s| split(s).0).filter(|n| n.starts_with("conquest:")).collect();
    let truth = real_truth(&cat, &inst.dir, &used);
    let written = check_world(&dir, &truth);
    let kinds = ["_layer[", "_slab[", "_stairs[", "_vertical_slab[", "_corner_slab["];
    let count = |k: &str| written.iter().filter(|s| s.contains(k)).count();
    eprintln!("{} Conquest states written, each the game's: {:?}", written.len(), kinds.map(|k| (k, count(k))));
    for k in ["_layer[", "_slab[", "_stairs["] {
        assert!(count(k) > 0, "no {k} among {written:?}");
    }
    assert!(written.iter().any(|s| s.starts_with("conquest:grass_block_layer[")), "the game's grass takes Conquest's layers");
    let _ = std::fs::remove_dir_all(&dir);
}
