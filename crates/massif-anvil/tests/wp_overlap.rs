//! A WorldPainter world of overlapping layers, exported by WorldPainter and
//! by Massif after importing it, compared column by column: what each layer
//! leaves in the game, alone and where it overlaps every other layer.
//!
//! The world comes from `tools/wp-layers/MakeStressWorld.java` (`stress`):
//! cells of 8 × 8 blocks, cell k painted with the layers of the bits of k.
//! WorldPainter's own export of it comes from `tools/wp-layers/ExportWorld.java`.
//! Set `MASSIF_WP_STRESS` to a folder holding `stress.world` and
//! `wp-export/<world>/`, then:
//!
//! `cargo test --release -p massif-anvil --test wp_overlap -- --nocapture`

use fastanvil::{Chunk, JavaChunk, Region};
use massif_anvil::{ExportOptions, Progress, Target, export};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The layers of the stress world, by bit.
const LAYERS: [&str; 14] = ["Grass", "Flowers", "CR grass layer", "Snow layers", "CR grass and sand", "Frost", "Mallorns", "Statues", "Deciduous", "Annotations", "Shire", "Roads to build", "Gravel pockets", "Gravel path"];
const SIDE: i32 = 1024;
const X0: i32 = -512;

fn region_dir(dir: &Path) -> PathBuf {
    let split = dir.join("dimensions/minecraft/overworld/region");
    if split.is_dir() { split } else { dir.join("region") }
}

/// A chunk as fastanvil reads it (26.3's `id`/`properties` renamed).
fn as_fastanvil_reads(data: &[u8]) -> Vec<u8> {
    use fastnbt::Value;
    let mut root: Value = fastnbt::from_bytes(data).unwrap();
    let Value::Compound(chunk) = &mut root else { panic!() };
    if !matches!(chunk.get("DataVersion"), Some(Value::Int(v)) if *v >= 5006) {
        return data.to_vec();
    }
    if let Some(Value::List(sections)) = chunk.get_mut("sections") {
        for section in sections {
            let Value::Compound(section) = section else { continue };
            let Some(Value::Compound(states)) = section.get_mut("block_states") else { continue };
            let Some(Value::List(palette)) = states.get_mut("palette") else { continue };
            for entry in palette {
                let Value::Compound(entry) = entry else { continue };
                if let Some(id) = entry.remove("id") {
                    entry.insert("Name".into(), id);
                }
                if let Some(props) = entry.remove("properties") {
                    entry.insert("Properties".into(), props);
                }
            }
        }
    }
    fastnbt::to_bytes(&root).unwrap()
}

/// What the layers leave in each 8 × 8 cell: per sign, a count.
#[derive(Default, Clone)]
struct Cell([u32; SIGNS]);

const SIGNS: usize = 14;
/// Signs: columns with short grass, a flower, conquest grass layers, snow
/// layers, grass-and-sand layers, snow or ice of any kind, mallorn log
/// blocks, statue blocks, oak or birch logs, ferns, forest floor or small
/// stones, spruce logs, gravel underground, gravel layers. Trees are counted
/// by their trunks (a log over anything but a log).
const SIGN_NAMES: [&str; SIGNS] = ["short grass", "flowers", "CR grass layer", "snow layers", "CR grass+sand", "snow/ice", "mallorn trunks", "statue blocks", "oak/birch trunks", "ferns", "forest floor", "spruce trunks", "gravel below", "gravel layer"];

fn sign_of(name: &str) -> Option<usize> {
    let short = name.split_once(':').map_or(name, |p| p.1);
    Some(match short {
        "short_grass" | "grass" => 0,
        "poppy" | "dandelion" => 1,
        "grass_block_layer" => 2,
        "snow" => 3,
        "grass_and_sand_layer" => 4,
        "ice" | "snow_block" => 5,
        "mallorn_log" => 6,
        "oak_log" | "birch_log" => 8,
        "fern" => 9,
        "fir_forest_floor_layer" | "small_stones" => 10,
        "spruce_log" => 11,
        "gravel_layer" => 13,
        _ => return None,
    })
}

fn natural(name: &str) -> bool {
    let short = name.split_once(':').map_or(name, |p| p.1);
    sign_of(name).is_some()
        || short.ends_with("leaves")
        || short.ends_with("_log")
        || matches!(short, "air" | "cave_air" | "water" | "stone" | "dirt" | "grass_block" | "sand" | "gravel" | "bedrock" | "deepslate" | "tall_grass" | "large_fern" | "cushion_moss_covered_loamy_dirt" | "dusty_yellow_dirt" | "coarse_dirt" | "podzol" | "clay" | "sandstone" | "vine" | "tuff" | "granite" | "diorite" | "andesite" | "lava")
        || short.ends_with("_ore")
}

/// Reads an export's cells.
fn read_cells(dir: &Path) -> Vec<Cell> {
    let per = (SIDE / 8) as usize;
    let mut cells = vec![Cell::default(); per * per];
    let mut regions: HashMap<(i32, i32), Region<std::fs::File>> = HashMap::new();
    for cz in (X0 >> 4)..((X0 + SIDE) >> 4) {
        for cx in (X0 >> 4)..((X0 + SIDE) >> 4) {
            let region = regions.entry((cx >> 5, cz >> 5)).or_insert_with(|| Region::from_stream(std::fs::File::open(region_dir(dir).join(format!("r.{}.{}.mca", cx >> 5, cz >> 5))).unwrap()).unwrap());
            let Some(data) = region.read_chunk((cx & 31) as usize, (cz & 31) as usize).unwrap() else { continue };
            let chunk = JavaChunk::from_bytes(&as_fastanvil_reads(&data)).unwrap();
            for lz in 0..16 {
                for lx in 0..16 {
                    let (x, z) = (cx * 16 + lx, cz * 16 + lz);
                    let cell = &mut cells[((z - X0) / 8) as usize * per + ((x - X0) / 8) as usize];
                    let mut seen = [false; SIGNS];
                    let mut ground = None;
                    let mut above = None;
                    for y in (-20..140).rev() {
                        let Some(b) = chunk.block(lx as usize, y as isize, lz as usize) else { continue };
                        let name = b.name();
                        if ground.is_none() && matches!(name, "minecraft:grass_block" | "minecraft:dirt" | "minecraft:stone" | "conquest:cushion_moss_covered_loamy_dirt" | "conquest:dusty_yellow_dirt" | "minecraft:sand" | "minecraft:water" | "minecraft:gravel") {
                            ground = Some(y);
                        }
                        let here = sign_of(name);
                        // A trunk's foot: a log standing on the ground or a
                        // cover's block (a branch's end, over leaves or air,
                        // is none).
                        if let Some(s @ (6 | 8 | 11)) = above
                            && here != above
                            && !name.ends_with("leaves")
                            && !matches!(name, "minecraft:air" | "minecraft:cave_air")
                            && !name.ends_with("_log")
                        {
                            cell.0[s] += 1;
                        }
                        above = here;
                        if let Some(g) = ground {
                            if name == "minecraft:gravel" && y < g - 4 {
                                cell.0[12] += 1;
                            }
                            continue;
                        }
                        match sign_of(name) {
                            // Trees: counted by their trunks' feet.
                            Some(6 | 8 | 11) => {}
                            Some(s) => seen[s] = true,
                            None if !natural(name) => cell.0[7] += 1,
                            None => {}
                        }
                    }
                    if seen.iter().any(|&s| s) && (seen[3] || seen[5]) {
                        seen[5] = true;
                    }
                    for (s, &on) in seen.iter().enumerate() {
                        cell.0[s] += on as u32;
                    }
                }
            }
        }
    }
    cells
}

/// The sign that tells layer `k` is there.
const SIGN_OF_LAYER: [Option<usize>; 14] = [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), Some(6), Some(7), Some(8), None, Some(9), None, Some(12), Some(13)];

#[test]
fn overlapping_layers_export_as_worldpainter_exports_them() {
    let Some(dir) = std::env::var_os("MASSIF_WP_STRESS").map(PathBuf::from) else {
        eprintln!("MASSIF_WP_STRESS is not set: skipped");
        return;
    };
    let t = std::time::Instant::now();
    let (mut world, report) = massif_legacy::import_world(&dir.join("stress.world"), None).expect("imported");
    eprintln!("imported in {:.2?}: {}", t.elapsed(), report.summary());
    world.meta.ores.on = false;
    let out = std::env::temp_dir().join(format!("massif-wp-overlap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let options = ExportOptions { target: Target::release("1.20.1").unwrap(), surroundings: massif_anvil::Surroundings::Void, ..ExportOptions::default() };
    export(&world, &out, &options, &Progress::default()).expect("exported");
    let wp_dir = std::fs::read_dir(dir.join("wp-export")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_dir()).expect("WorldPainter's export");
    let (wp, ms) = (read_cells(&wp_dir), read_cells(&out));
    let _ = std::fs::remove_dir_all(&out);

    // For each layer: what its sign comes to over the cells it is painted
    // on, in each export, and where it overlaps each other layer.
    let per = (SIDE / 8) as usize;
    let mut worst: Vec<(f64, String)> = Vec::new();
    eprintln!("{:<20} {:>10} {:>10} {:>7}", "layer (sign)", "WorldPainter", "Massif", "ratio");
    for (k, name) in LAYERS.iter().enumerate() {
        let Some(s) = SIGN_OF_LAYER[k] else { continue };
        let sum = |cells: &[Cell], f: &dyn Fn(usize) -> bool| -> u64 { (0..per * per).filter(|&c| f(c % (1 << 14))).map(|c| cells[c].0[s] as u64).sum() };
        let on = |m: usize| m >> k & 1 == 1;
        let (a, b) = (sum(&wp, &on), sum(&ms, &on));
        let off = |m: usize| m >> k & 1 == 0;
        let (a0, b0) = (sum(&wp, &off), sum(&ms, &off));
        eprintln!("{:<20} {:>10} {:>10} {:>7.2}   (not painted: {a0} / {b0}) [{}]", name, a, b, b as f64 / a.max(1) as f64, SIGN_NAMES[s]);
        for (j, other) in LAYERS.iter().enumerate().filter(|(j, _)| *j != k) {
            let both = |m: usize| m >> k & 1 == 1 && m >> j & 1 == 1;
            let (a, b) = (sum(&wp, &both), sum(&ms, &both));
            if a > 200 {
                worst.push((b as f64 / a as f64, format!("{name} over {other}: {a} / {b}")));
            }
        }
    }
    // Trees where nothing else stands in their way: WorldPainter stands no
    // object where a block is already over the ground (a custom ground
    // cover's, another layer's tree), so a forest is compared on the cells
    // where it is the only thing painted over the ground.
    let in_the_way = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 10) | (1 << 13);
    for (k, s) in [(6, 6), (7, 7), (8, 8)] {
        let alone = |m: usize| m >> k & 1 == 1 && m & in_the_way & !(1 << k) == 0;
        let sum = |cells: &[Cell]| -> u64 { (0..per * per).filter(|&c| alone(c % (1 << 14))).map(|c| cells[c].0[s] as u64).sum() };
        let (a, b) = (sum(&wp), sum(&ms));
        eprintln!("{} alone: WorldPainter {a}, Massif {b}, ratio {:.2}", LAYERS[k], b as f64 / a.max(1) as f64);
    }
    worst.sort_by(|a, b| a.0.total_cmp(&b.0));
    eprintln!("\nmost lost where two layers overlap (WorldPainter / Massif):");
    for (r, w) in worst.iter().take(30) {
        eprintln!("  {r:.2}  {w}");
    }
}

/// Every layer painted on a column of the stress world is on that column
/// once imported: covers on top or stacked under it, tree layers at their
/// own strength, annotations, pockets; and the custom biomes come in as
/// biomes of the map, painted where they were.
#[test]
fn every_layer_of_every_column_is_imported() {
    use massif_core::coords::local_index;
    use massif_core::layer_stack::Kind;
    let Some(dir) = std::env::var_os("MASSIF_WP_STRESS").map(PathBuf::from) else {
        eprintln!("MASSIF_WP_STRESS is not set: skipped");
        return;
    };
    let (world, report) = massif_legacy::import_world(&dir.join("stress.world"), None).expect("imported");
    let reg = &world.registry;
    let cover = |name: &str| reg.cover_by_name(name).unwrap_or_else(|| panic!("cover {name}"));
    let layer = |name: &str| reg.object_layers.iter().position(|l| l.name == name && !l.removed).map(|k| k as u16 + 1).unwrap_or_else(|| panic!("object layer {name}"));
    let covers = [(0, cover("Grass")), (1, cover("Flowers")), (2, cover("CR grass layer")), (3, cover("Snow layers")), (4, cover("CR grass and sand")), (5, cover("Frost")), (13, cover("Gravel path"))];
    let trees = [(6, layer("Mallorns")), (7, layer("Statues")), (8, layer("Deciduous"))];
    let biome = |id: &str| reg.biome_by_id(id).unwrap_or_else(|| panic!("biome {id}: {:?}", reg.biomes.iter().map(|b| &b.id).collect::<Vec<_>>())).0;
    let (meadow, alpine, misties) = (biome("minecraft:meadow"), biome("terralith:alpine_highlands"), biome("worldpainter:misties_biome_8"));
    assert_eq!(reg.biomes[misties as usize].name, "Misties Biome 8");
    assert_eq!(report.custom_biomes.len(), 2, "{:?}", report.custom_biomes);
    let dim = &world.dims[0];
    let (mut checked, mut missing) = (0usize, Vec::new());
    for z in X0..X0 + SIDE {
        for x in X0..X0 + SIDE {
            let tile = dim.ground().view(massif_core::TilePos::containing(x, z)).unwrap();
            let i = local_index(x, z);
            let cell = (((z - X0) / 8) * (SIDE / 8) + (x - X0) / 8) as usize % (1 << 14);
            let strength = (1 + (x * 7 + z * 13).rem_euclid(15)) as u8 * 17;
            let on = |k: usize| cell >> k & 1 == 1;
            let mut lost = |what: String| {
                if missing.len() < 20 {
                    missing.push(format!("({x}, {z}) cell {cell:014b}: {what}"));
                }
            };
            for &(k, n) in &covers {
                let here = tile.cover.get(i) == n || tile.stacked.strength(i, Kind::Cover, n).is_some();
                if on(k) != here {
                    lost(format!("{} {}", LAYERS[k], if here { "where not painted" } else { "lost" }));
                }
            }
            for &(k, n) in &trees {
                let here = if tile.stacked.has(i) { tile.stacked.strength(i, Kind::Objects, n).or((tile.objects.get(i) == n).then_some(255)) } else { (tile.objects.get(i) == n).then_some(255) };
                match (on(k), here) {
                    (true, Some(s)) if s == strength => {}
                    (false, None) => {}
                    (want, got) => lost(format!("{} painted {want} at {strength}: {got:?}", LAYERS[k])),
                }
            }
            let note = tile.annotations.get(i);
            if on(9) && note != 1 + (strength / 17) as u16 % 15 || !on(9) && on(11) && note <= 15 || !on(9) && !on(11) && note != 0 {
                lost(format!("annotation {note}"));
            }
            if on(12) != (tile.pockets.get(i) != 0) {
                lost("pockets".into());
            }
            let painted = [meadow, alpine, misties, 0][(x - X0).rem_euclid(256) as usize / 64];
            if !on(10) && tile.biome.get(i) != painted {
                lost(format!("biome {} for {painted}", tile.biome.get(i)));
            }
            checked += 1;
        }
    }
    assert!(missing.is_empty(), "{} columns checked; lost:\n{}", checked, missing.join("\n"));
    eprintln!("{checked} columns: every layer at its strength, biomes painted");
}
