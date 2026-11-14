//! The automatic biomes and paint over a real map, as big as maps come:
//! every column gets its biome, and the map is never decoded whole.
//! Runs with `MASSIF_AUTO_WORLD=<a .massif>`; skipped without it.

use massif_core::auto::{AutoRules, Reader};
use massif_core::compose::TileStack;
use massif_core::coords::{TILE_AREA, local_index};
use massif_core::history::History;
use std::path::Path;

#[test]
fn automatic_biomes_cover_a_real_world() {
    let Some(path) = std::env::var_os("MASSIF_AUTO_WORLD") else {
        return;
    };
    let t = std::time::Instant::now();
    let mut world = massif_format::load(Path::new(&path)).expect("the map loads");
    let tiles = world.dims[0].ground().len();
    eprintln!("{tiles} tiles, loaded in {:.1?}", t.elapsed());
    let rules = AutoRules::default();
    let base = massif_core::auto::base_terrain(&world.dims[0], &world.registry);
    eprintln!("filled with {:?}", base.map(|t| world.registry.terrain(t).name.clone()));
    let reader = Reader::new(&rules, &world.registry, world.meta.sea_level).with_base(base);
    let mut history = History::new(10);
    history.begin("Auto biome", 0);
    let t = std::time::Instant::now();
    let changed = massif_core::auto::biomes_over_the_map(&mut history.edit(&mut world, 0), &reader);
    history.commit();
    let stats = world.dims[0].ground().stats();
    eprintln!(
        "{changed} columns changed in {:.1?}; {} hot tiles ({} MB decoded), {} MB compressed",
        t.elapsed(),
        stats.hot_tiles,
        stats.hot_bytes >> 20,
        stats.cold_bytes >> 20
    );
    // What the terrains read as, and the biomes the land got.
    let grounds = massif_core::auto::Ground::of_terrains(&world.registry);
    for (k, (t, g)) in world.registry.terrains.iter().zip(&grounds).enumerate() {
        let top = t.strata.first().map(|s| s.blocks.iter().map(|(b, w)| format!("{}×{w}", world.registry.blocks[b.0 as usize].state)).collect::<Vec<_>>());
        eprintln!("terrain {k:>2} {:<24} {:?}  {:?}", t.name, g, top);
    }
    let mut counts = std::collections::HashMap::new();
    let mut terrains = std::collections::HashMap::new();
    for pos in world.dims[0].ground().positions().step_by(7) {
        let tile = world.dims[0].ground().view(pos).unwrap();
        for i in (0..TILE_AREA).step_by(61) {
            *counts.entry(tile.biome.get(i)).or_insert(0usize) += 1;
            *terrains.entry(tile.terrain.get(i)).or_insert(0usize) += 1;
        }
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    for (b, n) in counts.iter().take(8) {
        eprintln!("biome {:<36} {n}", world.registry.biomes[*b as usize].id);
    }
    let mut terrains: Vec<_> = terrains.into_iter().collect();
    terrains.sort_by_key(|c| std::cmp::Reverse(c.1));
    for (t, n) in terrains.iter().take(8) {
        eprintln!("terrain {:<36} {n}", world.registry.terrains[*t as usize].name);
    }
    // The map is never held decoded whole.
    assert!(stats.hot_tiles <= 4096, "{} tiles left decoded", stats.hot_tiles);

    // Every column with ground has a biome now, tile after tile.
    let dim = &world.dims[0];
    let mut left = Vec::new();
    for pos in dim.ground().positions() {
        let Some(stack) = TileStack::new(dim, pos, false) else { continue };
        let ground = stack.ground();
        let mut spans = Vec::new();
        let mut automatic = 0;
        let r = pos.rect();
        for z in r.z0..r.z1 {
            for x in r.x0..r.x1 {
                let cell = local_index(x, z);
                if stack.surface(cell, &mut spans).is_some() && ground.biome.get(cell) == 0 {
                    automatic += 1;
                }
            }
        }
        if automatic > 0 {
            left.push((pos, automatic));
        }
    }
    left.sort_by_key(|(p, _)| (p.z, p.x));
    eprintln!("{} tiles with automatic columns left: {:?}", left.len(), &left[..left.len().min(20)]);
    let total: usize = left.iter().map(|(_, n)| n).sum();
    assert!(total < tiles * TILE_AREA / 1000, "{total} columns left automatic");
}
