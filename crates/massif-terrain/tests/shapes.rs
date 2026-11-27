//! Shapes: they stay within their heights, have no seams, go where they are
//! placed, and cover what they are asked to.

use massif_terrain::compile::{Compiled, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::kernel::Grid;
use massif_terrain::model::Graph;
use massif_terrain::registry::{self, Family};
use massif_terrain::space::{Rect, Space};

fn compiled(json: &str) -> Compiled {
    let g = Graph::from_json(json).unwrap();
    let (c, diags) = compile(&g, Space::new(4096), 1024);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn whole(c: &Compiled, id: &str, port: Option<&str>, lod: u8) -> Grid {
    Engine::new(Settings::default()).whole(c, c.source(id, port).unwrap(), lod, &never).unwrap()
}

/// The largest step between neighbouring samples, in blocks per block.
fn steepest_step(g: &Grid) -> f32 {
    let spacing = (1u32 << g.lod) as f32;
    let mut s = 0.0f32;
    for z in g.rect.z..g.rect.z1() - 1 {
        for x in g.rect.x..g.rect.x1() - 1 {
            s = s.max((g.at(x + 1, z) - g.at(x, z)).abs()).max((g.at(x, z + 1) - g.at(x, z)).abs());
        }
    }
    s / spacing
}

#[test]
fn every_shape_evaluates_with_its_defaults_within_its_heights() {
    for spec in registry::all().iter().filter(|s| s.family == Family::Shape) {
        let c = compiled(&format!(r#"{{ "nodes": {{ "n": {{ "type": "{}" }} }} }}"#, spec.id));
        let g = whole(&c, "n", None, 3);
        assert!(g.data.iter().all(|v| v.is_finite()), "{}", spec.id);
        let (lo, hi) = g.range();
        assert!(lo >= -1100.0 && hi <= 4096.0 + 2048.0 && hi > lo + 10.0, "{}: {lo}..{hi}", spec.id);
        for port in spec.outputs.iter().filter(|p| p.kind == registry::PortKind::Mask) {
            let m = whole(&c, "n", Some(port.name), 3);
            let (lo, hi) = m.range();
            assert!(lo >= 0.0 && hi <= 1.0 && hi > 0.0, "{}.{}: {lo}..{hi}", spec.id, port.name);
        }
    }
}

#[test]
fn a_range_has_no_seam_where_its_spine_bends() {
    // A sharp bend: the nearest point of the spine jumps from one stretch
    // to the other inside it.
    let c = compiled(r#"{ "nodes": { "r": { "type": "mountain.range", "params": {
        "spine": [{ "x": 0.15, "z": 0.8 }, { "x": 0.5, "z": 0.3 }, { "x": 0.85, "z": 0.8 }],
        "height": 200, "width": 1400, "warp": 0, "roughness": 0 } } } }"#);
    let g = whole(&c, "r", None, 1);
    // Steep in places, never a cliff: a seam would be a step of many blocks.
    let s = steepest_step(&g);
    assert!(s < 2.5, "{s}");
}

#[test]
fn peaks_stand_where_they_are_placed() {
    let c = compiled(r#"{ "nodes": { "p": { "type": "mountain.peaks", "params": {
        "peaks": [{ "x": 0.25, "z": 0.25, "height": 200, "radius": 600 }, { "x": 0.7, "z": 0.6, "height": 120, "radius": 500 }],
        "base": 70, "roughness": 0, "warp": 0 } } } }"#);
    let g = whole(&c, "p", None, 2);
    // At 4 blocks a sample, the peaks at (1024, 1024) and (2867, 2458).
    assert!((g.at(256, 256) - 270.0).abs() < 4.0, "{}", g.at(256, 256));
    assert!((g.at(717, 614) - 190.0).abs() < 4.0, "{}", g.at(717, 614));
    assert_eq!(g.at(1000, 50), 70.0, "far from them, the base");
}

#[test]
fn a_coastline_makes_as_much_land_as_asked() {
    for land in [0.3, 0.7] {
        let c = compiled(&format!(r#"{{ "nodes": {{ "c": {{ "type": "coast.coastline", "params": {{ "land": {land}, "scale": 1200 }} }} }} }}"#));
        let m = whole(&c, "c", Some("land"), 3);
        let share = m.data.iter().filter(|v| **v > 0.5).count() as f32 / m.data.len() as f32;
        assert!((share - land).abs() < 0.12, "asked {land}, got {share}");
    }
}

#[test]
fn islands_meet_without_seams() {
    let c = compiled(r#"{ "nodes": { "a": { "type": "coast.archipelago", "params": {
        "count": 0, "islands": [{ "x": 0.45, "z": 0.5, "radius": 700, "height": 60 }, { "x": 0.6, "z": 0.5, "radius": 600, "height": 30 }],
        "roughness": 0, "hills": 0 } } } }"#);
    let g = whole(&c, "a", None, 1);
    let s = steepest_step(&g);
    assert!(s < 1.5, "{s}");
    // Both islands are land, the sea around them is not.
    let land = whole(&c, "a", Some("land"), 3);
    assert!(land.at(230, 256) > 0.9 && land.at(307, 256) > 0.9 && land.at(30, 30) < 0.1);
}

#[test]
fn shapes_tile_without_seams() {
    // The same samples whether asked for in one region or tile by tile.
    let c = compiled(r#"{ "nodes": { "a": { "type": "mountain.alpine" }, "r": { "type": "mountain.range" }, "i": { "type": "coast.island" } } }"#);
    for id in ["a", "r", "i"] {
        let e = Engine::new(Settings::default());
        let src = c.source(id, None).unwrap();
        let big = e.region(&c, src, 2, Rect::new(200, 200, 400, 300), &never).unwrap();
        let fresh = Engine::new(Settings::default());
        let part = fresh.region(&c, src, 2, Rect::new(250, 260, 100, 100), &never).unwrap();
        for z in 260..360 {
            for x in 250..350 {
                assert_eq!(big.at(x, z), part.at(x, z), "{id} at {x}, {z}");
            }
        }
    }
}

#[test]
fn every_node_evaluates_with_its_defaults() {
    // Each node type, its required inputs fed by a noise.
    for spec in registry::all().iter().filter(|s| !s.is_output() && s.family != Family::Internal) {
        let mut inputs = String::new();
        for (k, port) in spec.inputs.iter().enumerate() {
            if !port.optional {
                let src = match port.kind {
                    registry::PortKind::Color => "paint",
                    registry::PortKind::Terrain => "lay",
                    registry::PortKind::Points => "pts",
                    registry::PortKind::Splines => "rd",
                    registry::PortKind::Footprints => "lt",
                    registry::PortKind::Instances => "bd",
                    registry::PortKind::Density => "vol",
                    registry::PortKind::Layer => "cav",
                    _ => "feed",
                };
                inputs.push_str(&format!(r#"{}"{}": "{}""#, if k > 0 && !inputs.is_empty() { ", " } else { "" }, port.name, src));
            }
        }
        let json = format!(
            r#"{{ "nodes": {{ "feed": {{ "type": "noise.fbm" }}, "paint": {{ "type": "color.ramp", "inputs": {{ "height": "feed" }} }}, "lay": {{ "type": "terrain.layers" }}, "pts": {{ "type": "points.scatter", "params": {{ "spacing": 256 }} }}, "rd": {{ "type": "village.roads", "inputs": {{ "squares": "pts" }} }}, "lt": {{ "type": "village.plots", "inputs": {{ "squares": "pts", "roads": "rd" }} }}, "bd": {{ "type": "village.buildings", "inputs": {{ "squares": "pts", "roads": "rd", "plots": "lt" }} }}, "vol": {{ "type": "volume.from_height", "inputs": {{ "height": "feed" }} }}, "cav": {{ "type": "caves.layer" }}, "n": {{ "type": "{}", "inputs": {{ {inputs} }} }} }} }}"#,
            spec.id
        );
        let c = compiled(&json);
        for (p, port) in spec.outputs.iter().enumerate() {
            if port.kind.is_volume() {
                assert!(Engine::new(Settings::default()).bricks(&c, c.source("n", Some(port.name)).unwrap(), &[[0, 64, 0]], &never).is_ok(), "{}.{}", spec.id, port.name);
                continue;
            }
            if port.kind.is_geometry() {
                assert!(Engine::new(Settings::default()).geometry(&c, c.source("n", Some(port.name)).unwrap(), &never).is_ok(), "{}.{}", spec.id, port.name);
                continue;
            }
            let g = whole(&c, "n", Some(port.name), 4);
            assert!(g.data.iter().all(|v| v.is_finite()), "{}.{}", spec.id, port.name);
            let _ = p;
        }
    }
}

#[test]
fn a_volcano_has_no_seam_west_of_its_centre() {
    // The gullies go round the volcano: where the angle wraps from a half
    // turn to minus a half turn, west of the centre, they drew a line.
    let c = compiled(r#"{ "nodes": { "v": { "type": "mountain.volcano", "params": { "roughness": 0 } } } }"#);
    let g = whole(&c, "v", None, 1);
    // At 2 blocks a sample, the centre at (1024, 1024), the foot 400 samples away.
    let step = |x: i32, z: i32| (g.at(x, z + 1) - g.at(x, z)).abs();
    let across = |x0: i32, x1: i32| (x0..x1).map(|x| step(x, 1023)).fold(0.0f32, f32::max);
    let (west, east) = (across(700, 950), across(1100, 1350));
    // The same steps elsewhere round it, a row away from the wrap.
    let near = (700..950).map(|x| step(x, 1030).max(step(x, 1016))).fold(0.0f32, f32::max);
    assert!(west < near.max(east) * 1.5 + 0.5, "west {west}, east {east}, near rows {near}");
}
