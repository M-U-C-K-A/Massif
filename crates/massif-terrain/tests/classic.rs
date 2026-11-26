//! A map's classic graph (Massif's generator before the Terrain
//! Generator) is converted into its terrain graph when it is opened: the
//! heights it makes compared with the classic graph's, and a graph of the
//! map's own kept to drive the map, the classic nodes beside it.

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::graph::{CombineOp, Extent, Graph as V1, NodeKind, NoiseStyle};
use massif_core::World;
use massif_terrain::compile::compile_in;
use massif_terrain::engine::{Engine, never};
use massif_terrain::model::Graph;
use massif_terrain::world::{CLASSIC_FRAME, MapEnv, absorb_classic, graph_of, map_rect, outputs, space_of};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn world() -> World {
    generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Flat, ..Default::default() })
}

/// The classic graph's heights over the map, 256 samples a side.
fn classic_heights(world: &World, g: &V1) -> Vec<f32> {
    let extent = Extent::covering(map_rect(&world.dims[0]), 256);
    let out = g.build_on(None, &extent, &AtomicBool::new(false), &Default::default()).expect("the classic graph builds");
    out.heights.expect("a height").data.clone()
}

/// The world's terrain graph's heights over the map, 256 samples a side.
fn new_heights(world: &World) -> Vec<f32> {
    let (g, _) = graph_of(world).expect("a terrain graph");
    let space = space_of(world, 0, &g);
    let (c, diags) = compile_in(&g, space, 512, Arc::new(MapEnv::new(world, 0, 1, ".")));
    assert!(diags.iter().all(|d| d.severity != massif_terrain::compile::Severity::Error), "{diags:#?}");
    let (extent, out) = outputs(&Engine::new(Default::default()), &c, 2, &never).unwrap();
    assert_eq!(extent.n, 256);
    out.heights.expect("a height").data.clone()
}

/// The largest and the mean difference, in blocks.
fn compare(a: &[f32], b: &[f32]) -> (f32, f32) {
    assert_eq!(a.len(), b.len());
    let d: Vec<f32> = a.iter().zip(b).map(|(x, y)| (x - y).abs()).collect();
    (d.iter().copied().fold(0.0, f32::max), d.iter().sum::<f32>() / d.len() as f32)
}

fn stats(h: &[f32]) -> (f32, f32, f32) {
    (h.iter().copied().fold(f32::MAX, f32::min), h.iter().copied().fold(f32::MIN, f32::max), h.iter().sum::<f32>() / h.len() as f32)
}

/// Nodes the Terrain Generator makes with 1.2's code or the same
/// arithmetic give the same heights, Stream erosion (the classic Erosion)
/// included.
#[test]
fn exact_nodes_give_the_same_heights() {
    let mut w = world();
    let mut g = V1::default();
    let base = g.add(NodeKind::Constant { value: 70.0 }, [0.0, 0.0]);
    let bump = g.add(NodeKind::Constant { value: 30.0 }, [0.0, 120.0]);
    let add = g.add(NodeKind::Combine { op: CombineOp::Add, weight: 0.5 }, [200.0, 0.0]);
    let levels = g.add(NodeKind::Levels { in_min: 0.0, in_max: 200.0, out_min: 20.0, out_max: 180.0, gamma: 1.0 }, [400.0, 0.0]);
    let clamp = g.add(NodeKind::Clamp { min: 40.0, max: 150.0 }, [600.0, 0.0]);
    let out = g.add(NodeKind::HeightOutput, [800.0, 0.0]);
    assert!(g.connect(base, 0, add, 0) && g.connect(bump, 0, add, 1) && g.connect(add, 0, levels, 0) && g.connect(levels, 0, clamp, 0) && g.connect(clamp, 0, out, 0));
    let before = classic_heights(&w, &g);
    w.meta.graph = Some(g);
    let absorbed = absorb_classic(&mut w).expect("a classic graph");
    assert!(w.meta.graph.is_none(), "the classic graph is gone");
    assert_eq!((absorbed.nodes, absorbed.approximated.len(), absorbed.beside), (6, 0, false));
    let (max, mean) = compare(&before, &new_heights(&w));
    eprintln!("exact nodes: largest difference {max:.4} blocks, mean {mean:.4}");
    assert!(max < 0.01, "largest difference {max}");
}

/// A graph as the classic generator's starter made it (Mountain range and
/// Noise added, warped, eroded): the noises are the Terrain Generator's
/// own, so the relief is of the same shape and range, not the same blocks.
#[test]
fn a_classic_relief_keeps_its_range() {
    let mut w = world();
    let mut g = V1::default();
    let range = g.add(NodeKind::MountainRange { angle: 30.0, width: 300.0, scale: 160.0, height: 120.0, base: 64.0, seed: 7 }, [0.0, 0.0]);
    let noise = g.add(NodeKind::Noise { style: NoiseStyle::Fbm, scale: 300.0, octaves: 5, roughness: 0.5, height: 30.0, base: 0.0, seed: 3 }, [0.0, 150.0]);
    let add = g.add(NodeKind::Combine { op: CombineOp::Add, weight: 1.0 }, [200.0, 0.0]);
    let warp = g.add(NodeKind::Warp { amount: 20.0, scale: 200.0, seed: 5 }, [400.0, 0.0]);
    let erode = g.add(NodeKind::Erosion { rain: 0.5, erosion: 0.3, deposition: 0.3, capacity: 4.0, evaporation: 0.02, radius: 2.0, seed: 1 }, [600.0, 0.0]);
    let out = g.add(NodeKind::HeightOutput, [800.0, 0.0]);
    assert!(g.connect(range, 0, add, 0) && g.connect(noise, 0, add, 1) && g.connect(add, 0, warp, 0) && g.connect(warp, 0, erode, 0) && g.connect(erode, 0, out, 0));
    let before = classic_heights(&w, &g);
    w.meta.graph = Some(g);
    let absorbed = absorb_classic(&mut w).unwrap();
    assert_eq!(absorbed.approximated, ["Mountain range", "Noise", "Warp"]);
    let after = new_heights(&w);
    let ((lo0, hi0, mean0), (lo1, hi1, mean1)) = (stats(&before), stats(&after));
    let (max, mean) = compare(&before, &after);
    eprintln!("classic relief: classic {lo0:.1}–{hi0:.1} (mean {mean0:.1}), converted {lo1:.1}–{hi1:.1} (mean {mean1:.1}); per sample largest {max:.1}, mean {mean:.1}");
    let span = hi0 - lo0;
    assert!((mean1 - mean0).abs() < 0.2 * span, "means {mean0} and {mean1}");
    assert!((hi1 - hi0).abs() < 0.35 * span && (lo1 - lo0).abs() < 0.35 * span, "ranges {lo0}–{hi0} and {lo1}–{hi1}");
}

/// A map with a terrain graph of its own keeps it to drive the map; the
/// classic nodes come beside it, unwired, in a frame of their own.
#[test]
fn a_graph_of_its_own_keeps_driving_the_map() {
    let mut w = world();
    let own = Graph::from_json(
        r#"{ "nodes": {
        "hills": { "type": "noise.fbm", "params": { "scale": 300, "height": 80, "base": 70 } },
        "out": { "type": "output.map", "inputs": { "height": "hills" } } },
        "ui": { "hills": { "pos": [0, 0] }, "out": { "pos": [300, 0] } } }"#,
    )
    .unwrap();
    w.meta.terrain_graph = Some(serde_json::to_value(&own).unwrap());
    let mut g = V1::default();
    let c = g.add(NodeKind::Constant { value: 90.0 }, [0.0, 0.0]);
    let o = g.add(NodeKind::HeightOutput, [200.0, 0.0]);
    g.connect(c, 0, o, 0);
    w.meta.graph = Some(g);
    let before = {
        let mut copy = w.clone();
        copy.meta.graph = None;
        new_heights(&copy)
    };
    let absorbed = absorb_classic(&mut w).unwrap();
    assert!(absorbed.beside);
    let (g, _) = graph_of(&w).unwrap();
    let frame = g.frames.values().find(|f| f.title == CLASSIC_FRAME).expect("the classic frame");
    assert!(frame.members.iter().all(|m| m.starts_with("classic_") && g.nodes.contains_key(m)));
    assert_eq!(g.nodes["out"].inputs["height"], "hills", "the map's own graph untouched");
    assert_eq!(g.nodes.values().filter(|n| n.kind == "output.map").count(), 2);
    assert!(frame.rect[1] > g.ui["hills"].pos[1], "below the graph's own nodes");
    // Still the map's own graph's heights.
    assert_eq!(compare(&before, &new_heights(&w)).0, 0.0);
    // A blank graph of the map's own: the converted one takes its place.
    let mut w = world();
    w.meta.terrain_graph = Some(serde_json::to_value(Graph::from_json(r#"{ "nodes": { "out": { "type": "output.map" } } }"#).unwrap()).unwrap());
    let mut g = V1::default();
    let c = g.add(NodeKind::Constant { value: 90.0 }, [0.0, 0.0]);
    let o = g.add(NodeKind::HeightOutput, [200.0, 0.0]);
    g.connect(c, 0, o, 0);
    w.meta.graph = Some(g);
    assert!(!absorb_classic(&mut w).unwrap().beside);
    assert!(new_heights(&w).iter().all(|h| (h - 90.0).abs() < 0.01));
    // Saved, the map has no classic graph.
    let text = serde_json::to_string(&w.meta).unwrap();
    assert!(!text.contains("\"graph\""), "no classic graph saved");
}
