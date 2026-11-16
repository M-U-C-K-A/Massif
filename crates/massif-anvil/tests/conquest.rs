//! Conquest Reforged's blocks, exported for the version it plays (1.20.1)
//! and read back with `fastanvil`, an independent Anvil reader: every block
//! state written must be one the game has.
//!
//! Always with a fixture made as Conquest's files are (its list for
//! WorldPainter, its registry's lists). With its real instance too, when it
//! is installed here (`MASSIF_CONQUEST_INSTANCE`, or found among the
//! launchers' folders): then a hundred of its real blocks are painted as
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
use massif_core::{Brush, BrushShape, Profile, World};
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

/// Conquest Reforged's instance, when it is installed here.
fn conquest_instance() -> Option<massif_assets::instance::Instance> {
    std::env::var_os("MASSIF_CONQUEST_INSTANCE")
        .map(PathBuf::from)
        .and_then(|d| massif_assets::instance::Instance::read(&d))
        .or_else(|| massif_assets::instance::find_with_mod("conquest"))
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
    let Some(inst) = conquest_instance() else {
        eprintln!("Conquest Reforged is not installed here: skipping");
        return;
    };
    let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
    let Some(jar) = massif_assets::find_jars().into_iter().find(|j| j.version == version) else {
        eprintln!("Minecraft {version} is not installed here: skipping");
        return;
    };
    let cat = massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files()).unwrap();
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

/// A world exported elsewhere (by the app: `MASSIF_CONQUEST_CHECK=<dir>`),
/// checked against the real game the same way.
#[test]
fn an_exported_conquest_world_is_the_games() {
    let Some(dir) = std::env::var_os("MASSIF_CONQUEST_CHECK").map(PathBuf::from) else { return };
    let inst = conquest_instance().expect("Conquest Reforged's instance");
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
