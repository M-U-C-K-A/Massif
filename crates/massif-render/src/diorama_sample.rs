//! A map that looks like one, for the Diorama's tests and renders: hills
//! by the sea, woods of oaks, birches and spruces, a village of houses
//! along paths with a field, and caves under it all with lakes.

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::objects::{ObjectDef, ObjectLayerDef};
use massif_core::treepack::TreeKind;
use massif_core::{TILE_AREA, TerrainId, TilePos};

/// A house of `w × l` blocks: cobblestone floor, oak log corners, plank
/// walls with glass, and a roof of stairs up to a ridge of slabs.
fn house(name: &str, w: u16, l: u16, roof: &str) -> ObjectDef {
    let wall = 4u16;
    let h = wall + w / 2 + 2;
    let mut palette = vec!["minecraft:air".to_owned()];
    let mut id = |s: &str| {
        if let Some(k) = palette.iter().position(|p| p == s) {
            return k as u16;
        }
        palette.push(s.to_owned());
        (palette.len() - 1) as u16
    };
    let (floor, log, planks, glass, door) = (id("minecraft:cobblestone"), id("minecraft:oak_log[axis=y]"), id("minecraft:oak_planks"), id("minecraft:glass_pane"), id("minecraft:oak_door[half=lower,facing=south]"));
    let east = id(&format!("minecraft:{roof}_stairs[facing=east,half=bottom,shape=straight]"));
    let west = id(&format!("minecraft:{roof}_stairs[facing=west,half=bottom,shape=straight]"));
    let ridge = id(&format!("minecraft:{roof}_slab[type=bottom]"));
    let lantern = id("minecraft:lantern[hanging=false]");
    let mut blocks = vec![0u16; (w * l * h) as usize];
    let at = |x: u16, y: u16, z: u16| ((y * l + z) * w + x) as usize;
    for z in 0..l {
        for x in 0..w {
            blocks[at(x, 0, z)] = floor;
            let edge_x = x == 0 || x == w - 1;
            let edge_z = z == 0 || z == l - 1;
            for y in 1..wall {
                if edge_x && edge_z {
                    blocks[at(x, y, z)] = log;
                } else if edge_x || edge_z {
                    blocks[at(x, y, z)] = if y == 2 && (x % 3 == 1 || z % 3 == 1) { glass } else { planks };
                }
            }
        }
    }
    blocks[at(w / 2, 1, l - 1)] = door;
    // The roof: stairs rising from both long sides to a ridge.
    for k in 0..w.div_ceil(2) {
        let y = wall + k;
        if y >= h {
            break;
        }
        for z in 0..l {
            let (a, b) = (k, w - 1 - k);
            if a == b {
                blocks[at(a, y, z)] = ridge;
            } else if a < b {
                blocks[at(a, y, z)] = east;
                blocks[at(b, y, z)] = west;
                for x in a + 1..b {
                    if k > 0 && (z == 0 || z == l - 1) {
                        blocks[at(x, y - 1, z)] = planks;
                    }
                }
            }
        }
    }
    blocks[at(w / 2, wall + w / 2 + 1, l / 2)] = lantern;
    ObjectDef::new(name, [w, h, l], palette, blocks)
}

/// The map: hills by the sea, woods, a village with paths and a field.
pub fn village() -> massif_core::World {
    let mut world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Hills, seed: 11, ..GenParams::default() });
    let reg = &mut world.registry;
    let oaks = TreeKind::BigOak.install(reg);
    let birches = TreeKind::Birch.install(reg);
    let spruces = TreeKind::TallSpruce.install(reg);
    let first = reg.objects.len() as u32;
    reg.objects.extend([house("House", 7, 9, "spruce"), house("Long house", 9, 13, "dark_oak"), house("Hut", 5, 7, "oak")]);
    let mut village = ObjectLayerDef::new("Village", &[first, first + 1, first + 2], reg);
    village.spacing = 18;
    village.density = 0.85;
    village.rotate = true;
    village.soil_only = false;
    let houses = reg.add_object_layer(village);
    let path = {
        let b = reg.intern_block("minecraft:dirt_path", [148, 122, 72]);
        reg.terrains.push(massif_core::registry::TerrainDef { name: "Path".into(), group: "Test".into(), color: [148, 122, 72], strata: vec![massif_core::registry::Stratum { blocks: vec![(b, 1)], depth: (1, 1), mix: massif_core::registry::Mix::Grain }], submerged_top: None, biome_hint: None });
        TerrainId((reg.terrains.len() - 1) as u16)
    };
    let field = {
        let b = reg.intern_block("minecraft:farmland[moisture=7]", [110, 80, 50]);
        reg.terrains.push(massif_core::registry::TerrainDef { name: "Field".into(), group: "Test".into(), color: [110, 80, 50], strata: vec![massif_core::registry::Stratum { blocks: vec![(b, 1)], depth: (1, 1), mix: massif_core::registry::Mix::Grain }], submerged_top: None, biome_hint: None });
        TerrainId((reg.terrains.len() - 1) as u16)
    };
    let grass_cover = reg.cover_by_name("Grass").or_else(|| reg.cover_by_name("Tall grass")).unwrap_or(0);
    let (vx, vz) = VILLAGE;
    let sea = world.meta.sea_level as f32;
    let positions: Vec<TilePos> = world.dims[0].ground().positions().collect();
    for pos in positions {
        let (ox, oz) = pos.origin();
        let tile = world.dims[0].ground_mut().tile_mut(pos).unwrap();
        // Caves under it all (layer 1, below).
        tile.caves = massif_core::plane::Plane::Uniform(1);
        for i in 0..TILE_AREA {
            let (x, z) = (ox + (i % 128) as i32, oz + (i / 128) as i32);
            let top = tile.top.get(i);
            let fluid = tile.fluid.get(i);
            if top < sea + 1.0 || (fluid != massif_core::NO_FLUID && fluid as f32 > top) {
                continue;
            }
            let (dx, dz) = (x - vx, z - vz);
            let in_village = dx.abs() < 70 && dz.abs() < 70;
            let on_path = in_village && ((dx.rem_euclid(36) < 3) || (dz.rem_euclid(36) < 3));
            let in_field = (dx - 90).abs() < 18 && dz.abs() < 26;
            // Woods in patches, by a few waves.
            let w = ((x as f32 * 0.021).sin() + (z as f32 * 0.017).cos() + ((x + z) as f32 * 0.009).sin()) / 3.0;
            if on_path {
                tile.terrain.set(i, path.0);
                tile.objects.set(i, 0);
                tile.cover.set(i, 0);
            } else if in_field {
                tile.terrain.set(i, field.0);
                tile.objects.set(i, 0);
            } else if in_village {
                tile.objects.set(i, houses);
                tile.cover.set(i, grass_cover);
            } else if w > 0.15 {
                tile.objects.set(i, if top > sea + 30.0 { spruces } else if w > 0.4 { birches } else { oaks });
            } else if grass_cover != 0 && w > -0.3 {
                tile.cover.set(i, grass_cover);
            }
        }
    }
    // Caves under it all, as the game carves them, with its aquifers.
    let dim = &mut world.dims[0];
    dim.caves = massif_core::caves::CaveLayers::new(vec![massif_core::caves::CaveDef::vanilla("Caves", 11, dim.min_y, dim.max_y, sea as i32)]);
    dim.ground_mut().take_dirty();
    dim.voxels.take_dirty();
    world
}

/// Where the village is.
pub const VILLAGE: (i32, i32) = (-150, 120);

