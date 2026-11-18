//! Graphs and worlds: a world's graph of 1.2 is found migrated; a graph is
//! built into a world as Convert writes it, and the map it made reads back.

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::graph::Template;
use massif_core::{History, TerrainId};
use massif_terrain::compile::{Severity, compile_in};
use massif_terrain::engine::{Engine, never};
use massif_terrain::kernel::{Env, MapLayer};
use massif_terrain::model::Graph;
use massif_terrain::space::Rect;
use massif_terrain::world::{MapEnv, graph_of, outputs, space_of};
use std::sync::Arc;

#[test]
fn a_graph_builds_into_a_world() {
    let mut world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Flat, ..Default::default() });
    // A world of 1.2: its graph migrated when asked for.
    world.meta.graph = Some(Template::Alpine.graph(&world.registry, world.meta.sea_level as f32, 1024.0));
    let (g, diags) = graph_of(&world).expect("the world has a graph");
    assert!(diags.iter().all(|d| d.severity != Severity::Error), "{diags:#?}");
    assert!(g.nodes.values().any(|n| n.kind == "output.map"));
    // A graph of its own wins over 1.2's.
    let own = Graph::from_json(
        r#"{ "nodes": {
        "hills": { "type": "noise.fbm", "params": { "scale": 300, "height": 80, "base": 70 } },
        "steep": { "type": "mask.slope", "inputs": { "height": "hills" }, "params": { "from": 20, "to": 30 } },
        "lay": { "type": "terrain.layers", "inputs": { "mask_1": "steep" }, "params": { "base": "Grass", "terrain_1": "Rock" } },
        "out": { "type": "output.map", "inputs": { "height": "hills", "terrain": "lay" } } } }"#,
    )
    .unwrap();
    world.meta.terrain_graph = Some(serde_json::to_value(&own).unwrap());
    let (g, _) = graph_of(&world).unwrap();
    assert_eq!(g, own);
    // Built over the map, one sample a block.
    let space = space_of(&world, 0, &g);
    assert_eq!((space.x0, space.z0, space.size), (-512, -512, 1024));
    let env = Arc::new(MapEnv::new(&world, 0, 1, "."));
    let (c, diags) = compile_in(&g, space, 512, env);
    assert!(diags.is_empty(), "{diags:#?}");
    let e = Engine::new(Default::default());
    let (extent, out) = outputs(&e, &c, 0, &never).unwrap();
    assert_eq!(extent.n, 1024);
    let heights = out.heights.clone().expect("a height");
    assert!(out.terrains.iter().any(|(t, _)| *t == world.registry.terrain_by_name("Rock").unwrap()));
    let (seed, sea) = (world.meta.seed as u64, world.meta.sea_level);
    let reg = world.registry.clone();
    let palette: Vec<(TerrainId, [u8; 3])> = (0..reg.terrains.len()).map(|k| (TerrainId(k as u16), reg.terrain(TerrainId(k as u16)).color)).collect();
    let tiles: Vec<_> = massif_core::BlockRect::new(-512, -512, 512, 512).tiles().collect();
    let mut history = History::new(1);
    history.begin("Terrain graph", 0);
    {
        let paint = massif_core::graph::Painting { palette: &palette, climate: None };
        let write = massif_core::graph::GroundWrite { tiles: &tiles, create: true, map: None, sea, blank: TerrainId::ROCK };
        massif_core::graph::apply(&mut history.edit(&mut world, 0), &extent, &out, seed, &paint, &write);
    }
    // The map now has the graph's heights and terrains.
    let after = MapEnv::new(&world, 0, 2, ".");
    let mut read = vec![0.0f32; 64 * 64];
    assert!(after.read_map(MapLayer::Height, &c.space, 4, Rect::new(0, 0, 64, 64), &mut read));
    let mut off = 0.0f32;
    for (k, v) in read.iter().enumerate() {
        let (i, j) = ((k % 64) * 16, (k / 64) * 16);
        off = off.max((v - heights.data[j * 1024 + i]).abs());
    }
    assert!(off <= 1.0, "the map's heights are the graph's, to the block: {off}");
    let mut terrains = vec![0.0f32; 64 * 64];
    after.read_map(MapLayer::Terrain, &c.space, 4, Rect::new(0, 0, 64, 64), &mut terrains);
    let rock = world.registry.terrain_by_name("Rock").unwrap().0 as f32;
    assert!(terrains.contains(&rock) && terrains.iter().any(|t| *t != rock));
}
