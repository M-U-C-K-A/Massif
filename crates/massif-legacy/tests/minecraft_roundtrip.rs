//! A Massif map exported, the export imported, the import exported again:
//! the two exports have the same heights, top blocks, water and biomes,
//! and (kept above the ground) the same trees and plants, block for block.

use massif_anvil::{ExportOptions, Progress, Target, export};
use massif_core::World;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::world::DimensionKind;
use massif_legacy::minecraft::classify::{Class, classify};
use massif_legacy::minecraft::{Above, Options, chunk, region, scan};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What a column of an exported world shows: its top, the blocks from the
/// ground's top up, the biome at the ground.
#[derive(Debug, Clone, PartialEq)]
struct Col {
    ground: i32,
    blocks: Vec<String>,
    biome: String,
}

fn columns(dir: &Path) -> HashMap<(i32, i32), Col> {
    let world = scan(&[dir.to_path_buf()]).expect("a world");
    let dim = world.dimension(DimensionKind::Overworld).expect("an overworld");
    let mut out = HashMap::new();
    let legacy = |id: u16, d: u8| massif_objects::legacy::state(id, d);
    let biome = |n: i32| format!("#{n}");
    let (min_y, max_y) = world.heights(DimensionKind::Overworld);
    for r in &dim.regions {
        let region = region::Region::open(r).unwrap();
        for cz in 0..32 {
            for cx in 0..32 {
                let Some(nbt) = region.chunk(cx, cz).unwrap() else { continue };
                let d = chunk::decode(&nbt, &legacy, &biome).unwrap_or_else(|e| panic!("chunk {cx} {cz}: {e:?}"));
                let at = |x: usize, y: i32, z: usize| -> &str {
                    d.section(y.div_euclid(16)).map_or("minecraft:air", |s| {
                        let i = s.get(((y.rem_euclid(16) as usize) * 16 + z) * 16 + x);
                        if i == u16::MAX { "minecraft:air" } else { d.states[i as usize].as_str() }
                    })
                };
                for z in 0..16 {
                    for x in 0..16 {
                        let mut y = max_y - 1;
                        while y > min_y && classify(at(x, y, z)) != Class::Ground {
                            y -= 1;
                        }
                        let mut top = max_y - 1;
                        while top > y && classify(at(x, top, z)) == Class::Air {
                            top -= 1;
                        }
                        let blocks = (y..=top).map(|k| at(x, k, z).to_owned()).collect();
                        let b = d.biome_at(x, y, z).unwrap_or("").to_owned();
                        out.insert((d.x * 16 + x as i32, d.z * 16 + z as i32), Col { ground: y, blocks, biome: b });
                    }
                }
            }
        }
    }
    out
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("massif-mc-roundtrip-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A map with hills, a sea, ground covers and trees.
fn map() -> World {
    let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 11, ..GenParams::default() });
    let grass = w.registry.cover_by_name("Grass").unwrap();
    let oaks = massif_core::treepack::TreeKind::BigOak.install(&mut w.registry);
    let dim = &mut w.dims[0];
    let positions: Vec<_> = dim.ground().positions().collect();
    for (k, p) in positions.into_iter().enumerate() {
        let t = dim.ground_mut().tile_mut(p).unwrap();
        for i in 0..massif_core::TILE_AREA {
            t.cover.set(i, grass);
            if k == 0 {
                t.objects.set(i, oaks);
            }
        }
    }
    w
}

fn round_trip(target: &str, above: Above) -> (usize, usize, usize) {
    let w = map();
    let a = tmp(&format!("{target}-a"));
    let b = tmp(&format!("{target}-b"));
    let options = ExportOptions { target: Target::release(target).unwrap(), ..ExportOptions::default() };
    export(&w.clone().snapshot(), &a, &options, &Progress::default()).unwrap();
    let world = scan(std::slice::from_ref(&a)).unwrap();
    let mut opts = Options::new(&world, DimensionKind::Overworld);
    opts.above = above;
    let started = std::time::Instant::now();
    let (mut imported, report) = massif_legacy::minecraft::import(&world, &opts, None).unwrap();
    eprintln!("{target}: imported in {:.2} s: {}", started.elapsed().as_secs_f32(), report.summary());
    export(&imported.snapshot(), &b, &options, &Progress::default()).unwrap();
    let (ca, cb) = (columns(&a), columns(&b));
    assert!(!ca.is_empty());
    assert_eq!(ca.len(), cb.len(), "{target}: the same columns");
    let (mut heights, mut tops, mut biomes, mut above_diff) = (0, 0, 0, 0);
    for (k, x) in &ca {
        let y = &cb[k];
        heights += (x.ground != y.ground) as usize;
        tops += (x.blocks.first() != y.blocks.first()) as usize;
        biomes += (x.biome != y.biome) as usize;
        if x.blocks != y.blocks {
            above_diff += 1;
            if above_diff <= 3 {
                eprintln!("{target} {k:?}: {:?}\n   != {:?}", x.blocks, y.blocks);
            }
        }
    }
    eprintln!("{target} {above:?}: {} columns; {heights} heights, {tops} tops, {biomes} biomes, {above_diff} columns above the ground differ", ca.len());
    std::fs::remove_dir_all(&a).ok();
    std::fs::remove_dir_all(&b).ok();
    assert_eq!((heights, tops, biomes), (0, 0, 0), "{target}: heights, top blocks and biomes the same");
    (heights, tops, above_diff)
}

#[test]
fn round_trip_keeps_everything_on_1_21() {
    let (_, _, above) = round_trip("1.21.11", Above::Keep);
    assert_eq!(above, 0, "what stands on the ground comes back block for block");
}

#[test]
fn round_trip_of_every_era() {
    for target in ["26.3", "1.18.2", "1.16.5", "1.14.4"] {
        assert_eq!(round_trip(target, Above::Keep).2, 0, "{target}: what stands on the ground the same");
    }
    // Before 1.13 the export writes blocks by number without their states:
    // the land comes back, the upper halves of tall plants kept as
    // sculpted blocks do not.
    round_trip("1.12.2", Above::Keep);
}

#[test]
fn trees_become_object_layers() {
    let w = map();
    let a = tmp("trees");
    export(&w.clone().snapshot(), &a, &ExportOptions { target: Target::release("1.21.11").unwrap(), ..ExportOptions::default() }, &Progress::default()).unwrap();
    let world = scan(std::slice::from_ref(&a)).unwrap();
    let mut opts = Options::new(&world, DimensionKind::Overworld);
    opts.above = Above::Trees;
    let (imported, report) = massif_legacy::minecraft::import(&world, &opts, None).unwrap();
    std::fs::remove_dir_all(&a).ok();
    assert!(!imported.registry.object_layers.is_empty(), "{report:?}");
    assert!(imported.registry.object_layers.iter().any(|l| l.name.contains("Oak")), "{:?}", imported.registry.object_layers.iter().map(|l| &l.name).collect::<Vec<_>>());
    assert!(imported.dims[0].voxels.is_empty(), "nothing sculpted");
}
