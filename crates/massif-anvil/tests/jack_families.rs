//! Jack's ArdaCraft map against Conquest Reforged's block families, read
//! from the player's own files, so only when asked:
//! `MASSIF_JACK_MAP=<.massif>` and `MASSIF_CONQUEST_INSTANCE=<instance dir>`.
//! Prints what the families table finds, which covers of layers can become
//! ground, and the shapes each terrain painted gets from Smooth terrain.

use massif_core::shapes::{Known, Shape, ShapeSet, family, top_block, usable};
use std::collections::HashMap;

#[test]
fn jacks_map_knows_its_families() {
    let (Ok(map), Ok(inst)) = (std::env::var("MASSIF_JACK_MAP"), std::env::var("MASSIF_CONQUEST_INSTANCE")) else {
        eprintln!("MASSIF_JACK_MAP and MASSIF_CONQUEST_INSTANCE not set: skipping");
        return;
    };
    let inst = massif_assets::instance::Instance::read(std::path::Path::new(&inst)).expect("an instance");
    let version = inst.minecraft.clone().unwrap_or_else(|| "1.20.1".to_owned());
    let jar = massif_assets::find_jars().into_iter().find(|j| j.version == version).expect("the game's jar");
    let t = std::time::Instant::now();
    let cat = massif_assets::load_with_mods(&jar.path, &jar.version, &inst.files()).expect("the catalogue");
    eprintln!("catalogue read in {:.2} s, {} stonecutter recipes", t.elapsed().as_secs_f32(), cat.stonecutting.len());
    let names: HashMap<&str, &str> = cat.blocks.iter().map(|b| (b.id.as_str(), b.name.as_str())).collect();
    let name_of = |id: &str| names.get(id).map(|n| (*n).to_owned());
    let t = std::time::Instant::now();
    let families = massif_core::block_families::BlockFamilies::from_recipes(cat.stonecutting.iter().map(|(a, b)| (a.as_str(), b.as_str())), &name_of);
    eprintln!("families: {} shapes linked in {:.1} ms", families.len(), t.elapsed().as_secs_f64() * 1000.0);
    let props = |id: &str| cat.definitions.get(id).map(|d| d.properties.iter().map(|p| p.name.clone()).collect()).or_else(|| cat.state_properties.get(id).cloned()).unwrap_or_default();
    let mut mods: HashMap<String, Vec<String>> = cat.blocks.iter().filter(|b| !b.id.starts_with("minecraft:")).map(|b| (b.id.clone(), props(&b.id))).collect();
    for id in cat.definitions.keys().filter(|id| !id.starts_with("minecraft:")) {
        mods.entry(id.clone()).or_insert_with(|| props(id));
    }
    let target = massif_anvil::Target::release(&version).unwrap();
    let vanilla = |id: &str| id.strip_prefix("minecraft:").is_some_and(|s| target.has_block(s));
    let known = Known { vanilla: &vanilla, mods: &mods, families: &families };
    for (shape, id) in [(Shape::Slab, "minecraft:stone"), (Shape::Slab, "minecraft:green_terracotta"), (Shape::Layers, "minecraft:grass_block"), (Shape::Layers, "minecraft:podzol"), (Shape::Slab, "conquest:brown_moss")] {
        let f = family(id, &known);
        eprintln!("{id}: {shape:?} {:?} (slab by eighths: {})", f.get(shape), f.slab_layers);
    }

    let t = std::time::Instant::now();
    let world = massif_format::load(std::path::Path::new(&map)).expect("the map");
    eprintln!("map read in {:.1} s", t.elapsed().as_secs_f32());
    let reg = &world.registry;
    let found = massif_core::surface_layers::surface_covers(reg, &known);
    for (k, def) in reg.covers.iter().enumerate().filter(|(_, d)| !d.removed) {
        let f = found.iter().find(|c| c.cover == k as u16 + 1);
        eprintln!("cover {:>3} {:<40} {} {:?}", k + 1, def.name, if f.is_some() { "LAYERS" } else { "      " }, def.plants.iter().map(|p| p.0.as_str()).take(4).collect::<Vec<_>>());
    }
    // The terrains painted, by columns.
    let mut counts: HashMap<u16, usize> = HashMap::new();
    let dim = &world.dims[0];
    for pos in dim.ground().positions() {
        if let Some(tile) = dim.ground().view(pos) {
            match tile.terrain.uniform_value() {
                Some(v) => *counts.entry(v).or_default() += massif_core::TILE_AREA,
                None => tile.terrain.dense().iter().for_each(|v| *counts.entry(*v).or_default() += 1),
            }
        }
    }
    let mut used: Vec<(u16, usize)> = counts.into_iter().collect();
    used.sort_by_key(|u| std::cmp::Reverse(u.1));
    let (mut smoothed, mut total) = (0usize, 0usize);
    for (t, n) in used.iter().take(60) {
        let id = massif_core::TerrainId(*t);
        let Some(b) = top_block(reg, id) else { continue };
        let f = family(reg.block(b).name(), &known);
        let all = ShapeSet::of(&Shape::ALL);
        let may = usable(world.meta.smooth.shapes_of(reg, id), &f);
        let _ = all;
        total += n;
        if may.has(Shape::Layers) || may.has(Shape::Slab) {
            smoothed += n;
        }
        eprintln!("{:>10} {:<34} {:<44} {:?} by eighths {}", n, reg.terrain(id).name, reg.block(b).state, Vec::<Shape>::from(may), f.layers.is_some() || f.slab_layers);
    }
    eprintln!("columns of the 60 first terrains smoothed by height: {:.1} %", smoothed as f64 * 100.0 / total.max(1) as f64);
}
