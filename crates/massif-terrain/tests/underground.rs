//! What a graph puts under the ground, applied to a map: the Cave System's
//! cave layer and cave biomes, a tunnel carved along a line and rock added
//! over the ground, then the graph applied again and changed.

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::{History, TerrainId, TilePos, World};
use massif_terrain::compile::{Compiled, Severity, compile_in};
use massif_terrain::engine::{Engine, never};
use massif_terrain::model::Graph;
use massif_terrain::underground::{self, Applied};
use massif_terrain::world::{MapEnv, outputs, space_of};
use std::sync::Arc;

fn world() -> World {
    generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Flat, ..Default::default() })
}

const GRAPH: &str = r#"{ "nodes": {
    "relief": { "type": "noise.fbm", "params": { "scale": 300, "height": 140, "base": 60, "seed": 3 } },
    "system": { "type": "massif:cave-system", "inputs": { "height": "relief" }, "params": { "chambers": 0.7 } },
    "tunnel": { "type": "volume.tube", "params": { "path": [{ "x": 0.1, "z": 0.5 }, { "x": 0.9, "z": 0.5 }], "y": 30, "radius": 5, "variation": 0 } },
    "arch": { "type": "volume.box", "params": { "x": 0.25, "z": 0.25, "y": 230, "width": 40, "height": 8, "length": 12 } },
    "out": { "type": "output.map", "inputs": { "height": "relief", "caves": "system.caves", "cave_biomes": "system.biomes", "volume.carve": "tunnel", "volume.add": "arch" } } } }"#;

fn compiled(world: &World, json: &str) -> Compiled {
    let g = Graph::from_json(json).unwrap();
    let space = space_of(world, 0, &g);
    let (c, diags) = compile_in(&g, space, 512, Arc::new(MapEnv::new(world, 0, 1, ".")));
    assert!(diags.iter().all(|d| d.severity != Severity::Error), "{diags:#?}");
    c
}

/// Apply as the panel does: the ground, then what lies under it (worked
/// out ahead, off the interface's thread, when `ahead`).
fn apply(world: &mut World, c: &Compiled, e: &Engine) -> Applied {
    apply_ahead(world, c, e, false).0
}

fn apply_ahead(world: &mut World, c: &Compiled, e: &Engine, ahead: bool) -> (Applied, std::time::Duration) {
    let (extent, out) = outputs(e, c, 0, &never).unwrap();
    let (seed, sea) = (world.meta.seed as u64, world.meta.sea_level);
    let reg = world.registry.clone();
    let palette: Vec<(TerrainId, [u8; 3])> = (0..reg.terrains.len()).map(|k| (TerrainId(k as u16), reg.terrain(TerrainId(k as u16)).color)).collect();
    let tiles: Vec<TilePos> = massif_core::BlockRect::new(-512, -512, 512, 512).tiles().collect();
    let mut history = History::new(1);
    history.begin("Terrain graph", 0);
    let done = {
        let mut edit = history.edit(world, 0);
        // What was edited by hand, before the ground is written.
        let kept = underground::HandEdits::capture(c, edit.dim());
        let paint = massif_core::graph::Painting { palette: &palette, climate: None };
        let write = massif_core::graph::GroundWrite { tiles: &tiles, create: true, map: None, sea, blank: TerrainId::ROCK };
        massif_core::graph::apply(&mut edit, &extent, &out, seed, &paint, &write);
        if ahead {
            let prepared = underground::prepare(e, c, edit.dim(), &never).unwrap();
            let started = std::time::Instant::now();
            let done = underground::apply_full(e, c, &reg, &mut edit, Some(&prepared), Some(kept), &never).unwrap();
            (done, started.elapsed())
        } else {
            let started = std::time::Instant::now();
            let done = underground::apply_keeping(e, c, &reg, &mut edit, Some(kept), &never).unwrap();
            (done, started.elapsed())
        }
    };
    history.commit();
    done
}

/// Worked out ahead, then written: the same map as applied at once, the
/// writing itself short enough for the interface's thread.
#[test]
fn prepared_ahead_writes_the_same_quickly() {
    let (mut a, mut b) = (world(), world());
    let c = compiled(&a, GRAPH);
    let (once, t_once) = apply_ahead(&mut a, &c, &Engine::new(Default::default()), false);
    let (ahead, t_write) = apply_ahead(&mut b, &c, &Engine::new(Default::default()), true);
    assert_eq!(once, ahead);
    assert_eq!(a.dims[0].caves.defs(), b.dims[0].caves.defs());
    for t in a.dims[0].ground().positions() {
        assert_eq!(a.dims[0].ground().view(t).map(|x| x.caves.clone()), b.dims[0].ground().view(t).map(|x| x.caves.clone()), "{t:?}");
    }
    eprintln!("under the ground at once {t_once:?}, written after preparing {t_write:?}");
    assert!(t_write < t_once, "{t_write:?} {t_once:?}");
}

#[test]
fn a_graph_puts_caves_biomes_and_volumes_under_the_ground() {
    let mut world = world();
    // A layer painted by hand, on a tile the graph will give no caves to.
    let mut defs = world.dims[0].caves.to_vec();
    defs.push(massif_core::caves::CaveDef::vanilla("By hand", 9, -64, 320, 62));
    world.dims[0].caves = massif_core::caves::CaveLayers::new(defs);
    let hand = world.dims[0].caves.defs().len() as u16;
    let e = Engine::new(Default::default());
    // The graph's caves only east of the middle: west keeps what is painted.
    let json = GRAPH.replace(r#""inputs": { "height": "relief" }"#, r#""inputs": { "height": "relief", "area": "east" }"#).replace(r#""out":"#, r#""ramp": { "type": "pattern.gradient", "params": { "ends": [{ "x": 0, "z": 0.5 }, { "x": 1, "z": 0.5 }] } }, "east": { "type": "mask.threshold", "inputs": { "mask": "ramp" }, "params": { "at": 0.5, "softness": 0 } }, "out":"#);
    let c = compiled(&world, &json);
    {
        let mut h = History::new(1);
        h.begin("hand", 0);
        massif_core::ops::fill_caves_from(&mut h.edit(&mut world, 0), &[TilePos::new(-4, 0)], |_, _| (hand, 1.0));
        h.commit();
    }
    let done = apply(&mut world, &c, &e);
    assert_eq!(done.cave_layers, 1, "{done:?}");
    assert!(done.cave_columns > 100_000, "{done:?}");
    assert!(done.biome_cells > 1000, "{done:?}");
    assert!(done.chunks > 20, "{done:?}");
    let dim = &world.dims[0];
    let graph: Vec<_> = dim.caves.defs().iter().enumerate().filter(|(_, d)| d.graph.is_some()).collect();
    assert_eq!(graph.len(), 1);
    let (k, def) = graph[0];
    assert!((def.cheese.density - 0.7).abs() < 1e-6, "the exposed setting reached the layer");
    // East: the graph's layer; west: still the hand-painted one.
    let east = dim.ground().view(TilePos::new(2, 0)).unwrap();
    assert!((0..massif_core::TILE_AREA).filter(|&i| east.caves.get(i) == (k + 1) as u16).count() > 15_000);
    let west = dim.ground().view(TilePos::new(-4, 0)).unwrap();
    assert!((0..massif_core::TILE_AREA).all(|i| west.caves.get(i) == hand));
    drop((east, west));
    // Cave biomes: deep dark low down, lush or dripstone above, somewhere.
    let reg = &world.registry;
    let ids = ["minecraft:deep_dark", "minecraft:lush_caves", "minecraft:dripstone_caves"].map(|b| reg.biome_by_id(b).unwrap().0);
    let mut seen = [0usize; 3];
    for t in dim.ground().positions() {
        let tile = dim.ground().view(t).unwrap();
        for used in tile.cave_biomes.used() {
            if let Some(k) = ids.iter().position(|&b| b == used) {
                seen[k] += 1;
            }
        }
    }
    assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    // The tunnel: air along its line, under the ground.
    let voxel = |x: i32, y: i32, z: i32| massif_core::voxel::densities_at(dim, massif_core::voxel::ChunkPos::containing(x, y, z)).solid(x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16));
    let z = 0; // the middle of the map, north to south
    let solid_beside = voxel(-100, 30, z + 20);
    assert!(!voxel(-100, 30, z), "the tunnel is carved");
    assert!(!voxel(200, 31, z) && !voxel(0, 29, z));
    // The relief is 60 to 200 high: rock beside the tunnel at its height.
    assert!(solid_beside || dim.ground().view(TilePos::containing(-100, 20)).is_some());
    // The arch: rock in the air at 230.
    assert!(voxel(-256, 230, -256), "the added rock is there");
    assert!(!voxel(-256, 240, -256));
    // Applied again: nothing new.
    let again = apply(&mut world, &c, &e);
    assert_eq!((again.cave_layers, again.cave_columns), (1, 0), "{again:?}");
    assert_eq!(world.dims[0].caves.defs().iter().filter(|d| d.graph.is_some()).count(), 1);
    // Without caves in the graph, its layer goes, the hand-painted stays.
    let bare = compiled(&world, &json.replace(r#""caves": "system.caves", "#, ""));
    apply(&mut world, &bare, &e);
    let dim = &world.dims[0];
    assert!(dim.caves.get((k + 1) as u16).is_none(), "the graph's layer is removed");
    assert!(dim.caves.get(hand).is_some());
}

#[test]
fn a_volume_cut_shows_where_it_reaches() {
    let world = world();
    let c = compiled(&world, GRAPH);
    let e = Engine::new(Default::default());
    let src = c.output("volume.carve").unwrap();
    let whole = c.space.whole(0);
    let cut = underground::slice(&e, &c, src, 30, massif_terrain::space::Rect::new(0, 480, whole.w, 64), &never).unwrap();
    // The tunnel crosses the strip west to east, 10 blocks wide.
    let inside = cut.data.iter().filter(|&&v| v > 0.5).count();
    assert!(inside > 700 * 8 && inside < 900 * 12, "{inside}");
    let above = underground::slice(&e, &c, src, 60, massif_terrain::space::Rect::new(0, 480, whole.w, 64), &never).unwrap();
    assert!(above.data.iter().all(|&v| v == 0.0));
}

/// Blocks dug, filled or flooded by hand on a layer stay when the graph is
/// applied again, changed: the graph's own carving of before fills back.
#[test]
fn hand_edits_stay_when_the_graph_changes() {
    let mut world = world();
    let tube = |z: f64| {
        format!(
            r#"{{ "nodes": {{
            "relief": {{ "type": "noise.fbm", "params": {{ "scale": 300, "height": 60, "base": 120, "seed": 3 }} }},
            "tunnel": {{ "type": "volume.tube", "params": {{ "path": [{{ "x": 0.1, "z": {z} }}, {{ "x": 0.9, "z": {z} }}], "y": 30, "radius": 4, "variation": 0 }} }},
            "out": {{ "type": "output.map", "inputs": {{ "height": "relief", "volume.carve": "tunnel" }} }} }} }}"#
        )
    };
    let e = Engine::new(Default::default());
    let first = compiled(&world, &tube(0.5));
    apply(&mut world, &first, &e);
    let solid = |w: &World, x: i32, y: i32, z: i32| massif_core::voxel::densities_at(&w.dims[0], massif_core::voxel::ChunkPos::containing(x, y, z)).solid(x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16));
    assert!(!solid(&world, -200, 30, 0) && solid(&world, -200, 30, 100));
    // By hand: a hall dug at y 50 north of the tunnel, a plug of granite in
    // the tunnel, water poured beside it.
    let brush = massif_core::Brush { radius: 5.0, ..Default::default() };
    let granite = world.registry.terrain_by_name("Granite").unwrap_or(TerrainId::ROCK);
    {
        let mut h = History::new(1);
        h.begin("dig", 0);
        let mut edit = h.edit(&mut world, 0);
        use massif_core::ops::{SliceFill, paint_slice};
        assert!(paint_slice(&mut edit, &brush, -100.5, -150.5, 50, (2, 2), true, SliceFill::Air) > 100);
        assert!(paint_slice(&mut edit, &brush, 100.5, 0.5, 30, (0, 0), false, SliceFill::Rock(Some(granite))) > 20);
        assert!(paint_slice(&mut edit, &brush, 300.5, -150.5, 50, (0, 0), false, SliceFill::Water) > 20);
        h.commit();
    }
    assert!(!solid(&world, -101, 50, -151) && solid(&world, 100, 30, 0));
    // The tunnel moves south and the graph is applied again.
    let second = compiled(&world, &tube(0.7));
    apply(&mut world, &second, &e);
    assert!(solid(&world, -200, 30, 0), "the old tunnel is filled back");
    assert!(!solid(&world, -200, 30, 204), "the new one is dug");
    assert!(!solid(&world, -101, 50, -151), "the hall dug by hand stays");
    assert!(solid(&world, 100, 30, 0), "the granite stays");
    let at = |w: &World, x: i32, y: i32, z: i32| massif_core::voxel::densities_at(&w.dims[0], massif_core::voxel::ChunkPos::containing(x, y, z));
    assert_eq!(at(&world, 100, 30, 0).terrain(100i32.rem_euclid(16), 30i32.rem_euclid(16), 0), Some(granite));
    assert_eq!(at(&world, 300, 50, -151).fluid(300i32.rem_euclid(16), 50i32.rem_euclid(16), (-151i32).rem_euclid(16)), Some(massif_core::voxel::Fluid::Water));
    // Without the tunnel, no chunk of it is left: only those edited by hand.
    let none = compiled(&world, &tube(0.7).replace(r#", "volume.carve": "tunnel""#, ""));
    apply(&mut world, &none, &e);
    assert!(solid(&world, -200, 30, 204));
    let chunks = world.dims[0].voxels.len();
    assert!(chunks > 0 && chunks < 20, "{chunks} sculpted chunks left");
}
