//! Masks on reliefs whose answers are known: a cone, a pit, a disc.

use massif_terrain::compile::{Compiled, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::kernel::Grid;
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space};

/// A 1024-block terrain holding `extra` nodes beside a cone `cone` (apex
/// `top` over a base of 60, `radius` blocks, at the centre) and a disc.
fn graph(top: f64, radius: f64, extra: &str) -> Compiled {
    let r = radius / 1024.0;
    let json = format!(
        r#"{{ "nodes": {{
            "cone": {{ "type": "pattern.gradient", "params": {{ "kind": "radial", "ends": [[0.5, 0.5], [{}, 0.5]], "profile": [[0, 1], [1, 0]], "height": {}, "base": 60 }} }},
            "disc": {{ "type": "pattern.shape", "params": {{ "radius": 100, "softness": 0 }} }}
            {extra}
        }} }}"#,
        0.5 + r,
        top - 60.0
    );
    let (c, diags) = compile(&Graph::from_json(&json).unwrap(), Space::new(1024), 1024);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn at(c: &Compiled, id: &str, port: Option<&str>) -> Grid {
    Engine::new(Settings::default()).region(c, c.source(id, port).unwrap(), 0, Rect::new(0, 0, 1024, 1024), &never).unwrap()
}

#[test]
fn slope_facing_and_sunlight_on_a_cone() {
    let c = graph(160.0, 200.0, r#",
        "steep": { "type": "mask.slope", "inputs": { "height": "cone" }, "params": { "from": 20, "to": 30 } },
        "band": { "type": "mask.slope", "inputs": { "height": "cone" }, "params": { "from": 10, "to": 15, "upper": 20 } },
        "east": { "type": "mask.facing", "inputs": { "height": "cone" }, "params": { "direction": 0, "spread": 60 } },
        "west": { "type": "mask.facing", "inputs": { "height": "cone" }, "params": { "direction": 180, "spread": 60 } },
        "sun": { "type": "mask.exposure", "inputs": { "height": "cone" }, "params": { "azimuth": 0, "elevation": 35 } }"#);
    // The cone rises 100 blocks over 200: 26.6°.
    let steep = at(&c, "steep", None);
    assert!(steep.at(612, 512) > 0.7 && steep.at(512, 900) == 0.0, "{} {}", steep.at(612, 512), steep.at(512, 900));
    assert!(at(&c, "band", None).at(612, 512) < 0.05, "above Upper");
    // East of the apex, slopes look east.
    let (east, west) = (at(&c, "east", None), at(&c, "west", None));
    assert!(east.at(612, 512) > 0.95 && east.at(412, 512) < 0.05);
    assert!(west.at(412, 512) > 0.95 && west.at(612, 512) < 0.05);
    assert_eq!(east.at(512, 900), 0.0, "flat ground faces nowhere");
    let sun = at(&c, "sun", None);
    assert!(sun.at(612, 512) > sun.at(512, 900) && sun.at(512, 900) > sun.at(412, 512));
    assert!((sun.at(512, 900) - 35f32.to_radians().sin()).abs() < 1e-3, "flat ground takes the sine of the elevation");
}

#[test]
fn heights_curvature_and_openness() {
    let c = graph(160.0, 200.0, r#",
        "band": { "type": "mask.height", "inputs": { "height": "cone" }, "params": { "from": 100, "to": 130, "softness": 4 } },
        "ridge": { "type": "mask.curvature", "inputs": { "height": "cone" }, "params": { "mode": "convex", "scale": 24 } },
        "hollow": { "type": "mask.curvature", "inputs": { "height": "cone" }, "params": { "mode": "concave", "scale": 24 } },
        "pit": { "type": "adjust.levels", "inputs": { "in": "cone" }, "params": { "in_low": 60, "in_high": 160, "out_low": 160, "out_high": 60 } },
        "open": { "type": "mask.occlusion", "inputs": { "height": "pit" }, "params": { "radius": 120 } }"#);
    let band = at(&c, "band", None);
    // 115 blocks lies 90 blocks from the apex.
    assert!(band.at(512 + 90, 512) > 0.99 && band.at(512, 512) == 0.0 && band.at(1000, 1000) == 0.0);
    let (ridge, hollow) = (at(&c, "ridge", None), at(&c, "hollow", None));
    assert!(ridge.at(512, 512) > 0.9, "the apex is convex: {}", ridge.at(512, 512));
    assert!(hollow.at(712, 512) > 0.5, "the cone's foot is concave: {}", hollow.at(712, 512));
    assert_eq!(ridge.at(900, 900), 0.0, "flat ground is neither");
    let open = at(&c, "open", None);
    assert!(open.at(512, 512) < open.at(950, 950) - 0.2, "the bottom of a pit sees less sky: {} {}", open.at(512, 512), open.at(950, 950));
    assert!((open.at(950, 950) - 1.0).abs() < 0.02);
}

#[test]
fn distance_grow_and_shrink_from_a_disc() {
    let c = graph(160.0, 200.0, r#",
        "near": { "type": "mask.distance", "inputs": { "mask": "disc" }, "params": { "reach": 300 } },
        "grown": { "type": "mask.expand", "inputs": { "mask": "disc" }, "params": { "radius": 50 } },
        "shrunk": { "type": "mask.expand", "inputs": { "mask": "disc" }, "params": { "radius": -40 } },
        "edge": { "type": "mask.threshold", "inputs": { "mask": "near" }, "params": { "at": 0.5, "softness": 0 } },
        "out": { "type": "mask.invert", "inputs": { "mask": "disc" } }"#);
    let d = at(&c, "near", Some("distance"));
    assert!((d.at(512 + 300, 512) - 200.0).abs() < 2.0, "{}", d.at(512 + 300, 512));
    assert_eq!(d.at(512, 512), 0.0);
    let near = at(&c, "near", None);
    assert!((near.at(512 + 250, 512) - 0.5).abs() < 0.02);
    let edge = at(&c, "edge", None);
    assert!(edge.at(512 + 200, 512) > 0.5 && edge.at(512 + 300, 512) < 0.5);
    let (grown, shrunk) = (at(&c, "grown", None), at(&c, "shrunk", None));
    assert!(grown.at(512 + 140, 512) > 0.9 && grown.at(512 + 170, 512) < 0.1);
    assert!(shrunk.at(512 + 50, 512) > 0.9 && shrunk.at(512 + 70, 512) < 0.1);
    assert!((at(&c, "out", None).at(0, 0) - 1.0).abs() < 1e-6);
}

#[test]
fn snow_lies_above_its_line_but_not_on_cliffs() {
    let snow = r#",
        "snow": { "type": "mask.snow", "inputs": { "height": "cone" }, "params": { "line": 200, "softness": 10, "breakup": 0, "drift": 0, "north_bias": 0, "depth": 3 } }"#;
    // A gentle cone (7°): its top (300) is white, its foot (60) bare.
    let gentle = graph(300.0, 2000.0, snow);
    let (s, h) = (at(&gentle, "snow", None), at(&gentle, "cone", None));
    assert!(s.at(512, 512) > 0.99, "{}", s.at(512, 512));
    assert_eq!(s.at(1000, 1000), 0.0);
    let raised = at(&gentle, "snow", Some("height"));
    assert!((raised.at(512, 512) - h.at(512, 512) - 3.0).abs() < 0.05);
    // The line driven by a field: higher than the summit, no snow.
    let driven = graph(300.0, 2000.0, r#",
        "high": { "type": "pattern.constant", "params": { "value": 400 } },
        "snow": { "type": "mask.snow", "inputs": { "height": "cone", "@line": "high" }, "params": { "breakup": 0, "drift": 0 } }"#);
    assert_eq!(at(&driven, "snow", None).at(512, 512), 0.0);
    // A cliff of a cone (72°): over the line, yet bare.
    let cliff = graph(320.0, 82.0, snow);
    let (s, h) = (at(&cliff, "snow", None), at(&cliff, "cone", None));
    assert!(h.at(512 + 20, 512) > 250.0 && s.at(512 + 20, 512) < 0.05, "{} {}", h.at(512 + 20, 512), s.at(512 + 20, 512));
}

#[test]
fn the_height_band_is_the_same_on_the_graphics_card() {
    let Some(gpu) = massif_terrain::gpu::Gpu::headless() else { return };
    let c = graph(160.0, 200.0, r#",
        "band": { "type": "mask.height", "inputs": { "height": "cone" }, "params": { "from": 100, "to": 130, "softness": 12 } }"#);
    let cpu = at(&c, "band", None);
    let g = Engine::with_gpu(gpu, Settings::default()).region(&c, c.source("band", None).unwrap(), 0, Rect::new(0, 0, 1024, 1024), &never).unwrap();
    let worst = cpu.data.iter().zip(&g.data).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-4, "{worst}");
}
