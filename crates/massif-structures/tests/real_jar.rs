//! The game's own structures, assembled from a real jar: run with
//! `MASSIF_JAR=<…/1.21.11.jar> cargo test -p massif-structures --test real_jar -- --ignored --nocapture`
//! (or with a game found on this computer).

use massif_structures::Library;
use massif_structures::ground::Flat;
use massif_structures::vanilla::{self, Height, KINDS, Request};
use std::path::PathBuf;

fn library() -> Option<Library> {
    let (path, version) = match std::env::var_os("MASSIF_JAR") {
        Some(p) => {
            let p = PathBuf::from(p);
            let v = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            (p, v)
        }
        None => {
            let jar = massif_assets::find_jars().into_iter().next()?;
            (jar.path, jar.version)
        }
    };
    Library::open(&path, &version, &[]).ok()
}

#[test]
#[ignore]
fn every_structure_of_the_game_is_assembled() {
    let Some(lib) = library() else {
        eprintln!("no game jar: skipped");
        return;
    };
    let ground = Flat { height: 64, water: None };
    let sea = Flat { height: 40, water: Some(63) };
    for kind in KINDS {
        let water = kind.group == vanilla::Group::Water;
        let g: &dyn massif_structures::ground::Ground = if water { &sea } else { &ground };
        let y = match kind.height {
            Height::Depth(y) => y,
            _ => 64,
        };
        for seed in [1u64, 2, 3] {
            let t = std::time::Instant::now();
            let req = Request { kind, at: [0, y, 0], turn: None, seed, flatten: kind.flattens, id: 1 };
            match vanilla::build(&lib, g, &req) {
                Ok(built) => {
                    let s = &built.structure;
                    let shriekers = s.palette.iter().enumerate().filter(|(_, p)| p.starts_with("minecraft:sculk_shrieker[can_summon=true")).map(|(k, _)| k as u16).collect::<Vec<_>>();
                    let n_shriekers: usize = s.sections.iter().map(|sec| sec.blocks.iter().filter(|b| shriekers.contains(&b.1)).count()).sum();
                    let loot = s.block_entities.iter().filter(|b| String::from_utf8_lossy(&b.nbt).contains("LootTable")).count();
                    println!(
                        "{:<18} seed {seed}: {:>4} pieces, {:>8} blocks, y {}..{}, {:>3} block entities ({loot} with loot), {:>2} entities, {n_shriekers} shriekers, {} columns flattened, {:.0} ms",
                        kind.id,
                        s.pieces.len(),
                        s.block_count(),
                        s.min[1],
                        s.max[1],
                        s.block_entities.len(),
                        s.entities.len(),
                        built.heights.len(),
                        t.elapsed().as_secs_f64() * 1000.0
                    );
                    if kind.id == "ancient_city" {
                        assert!(n_shriekers > 0, "an Ancient City calls its warden");
                    }
                    assert!(s.block_count() > 50, "{}", kind.id);
                }
                Err(e) => println!("{:<18} seed {seed}: {e}", kind.id),
            }
        }
    }
    println!("unknown: {:#?}", lib.unknowns());
}

#[test]
#[ignore]
fn ancient_city_details() {
    let Some(lib) = library() else { return };
    let ground = Flat { height: 64, water: None };
    let kind = vanilla::kind("ancient_city").unwrap();
    let req = Request { kind, at: [0, -27, 0], turn: Some(0), seed: 1, flatten: false, id: 1 };
    let built = vanilla::build(&lib, &ground, &req).unwrap();
    let s = &built.structure;
    let mut counts = std::collections::BTreeMap::new();
    for sec in &s.sections {
        for b in &sec.blocks {
            *counts.entry(s.palette[b.1 as usize].split('[').next().unwrap().to_owned()).or_insert(0usize) += 1;
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.1));
    for (k, n) in v.iter().take(25) {
        println!("{n:>8} {k}");
    }
    let mut templates = std::collections::BTreeMap::new();
    for p in &s.pieces {
        *templates.entry(p.template.clone()).or_insert(0) += 1;
    }
    println!("{templates:#?}");
    println!("unknown: {:#?}", lib.unknowns());
}
