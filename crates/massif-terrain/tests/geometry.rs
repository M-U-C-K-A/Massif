//! Geometry nodes: made whole, cached, read back by raster nodes tile by
//! tile, inside composites as anywhere.

use massif_terrain::compile::{Severity, compile};
use massif_terrain::engine::{Engine, never};
use massif_terrain::kernel::Geo;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;

const GRAPH: &str = r#"{ "nodes": {
    "land": { "type": "noise.fbm", "params": { "scale": 300, "height": 60, "base": 80 } },
    "low": { "type": "mask.height", "inputs": { "height": "land" }, "params": { "from": 60, "to": 110, "softness": 0 } },
    "trees": { "type": "points.scatter", "inputs": { "density": "low", "height": "land" }, "params": { "spacing": 32 } },
    "round": { "type": "points.mask", "inputs": { "points": "trees" }, "params": { "radius": 3, "softness": 2 } },
    "out": { "type": "output.map", "inputs": { "height": "land", "mask.trees": "round" } }
} }"#;

#[test]
fn points_are_made_whole_and_read_by_tiles() {
    let g = Graph::from_json(GRAPH).unwrap();
    let (c, d) = compile(&g, Space::new(1024), 512);
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    let e = Engine::new(Default::default());
    let trees = c.node("trees").unwrap();
    let pts = e.geometry(&c, (trees, 0), &never).unwrap();
    let Geo::Points(p) = &*pts else { panic!("points") };
    // 32 × 32 squares; only where the land is low enough.
    assert!(p.items.len() > 50 && p.items.len() < 1024, "{} points", p.items.len());
    assert!(p.items.iter().all(|t| (60.0..=140.0).contains(&t.pos[1])), "standing on the land");
    // Made once: the second call is the cache's.
    let again = e.geometry(&c, (trees, 0), &never).unwrap();
    assert!(std::sync::Arc::ptr_eq(&pts, &again));
    // The same graph compiled again gives the same points.
    let e2 = Engine::new(Default::default());
    assert_eq!(*e2.geometry(&c, (trees, 0), &never).unwrap(), *pts);
    // Drawn back as a mask, tile by tile as the whole.
    let out = c.output("mask.trees").unwrap();
    let whole = e.region(&c, out, 0, c.space.whole(0), &never).unwrap();
    let ones = whole.data.iter().filter(|&&v| v >= 1.0).count();
    assert!(ones > p.items.len() * 20, "{ones} samples under {} trees", p.items.len());
    let part = e.region(&c, out, 0, massif_terrain::space::Rect::new(256, 256, 256, 256), &never).unwrap();
    for j in 0..256 {
        for i in 0..256 {
            assert_eq!(part.at(256 + i, 256 + j), whole.at(256 + i, 256 + j));
        }
    }
    // Asked by tiles, geometry says it is made whole.
    assert!(e.region(&c, (trees, 0), 0, massif_terrain::space::Rect::new(0, 0, 256, 256), &never).is_err());
}

#[test]
fn geometry_feeds_only_its_own_type() {
    let g = Graph::from_json(r#"{ "nodes": {
        "trees": { "type": "points.scatter" },
        "bad": { "type": "mask.invert", "inputs": { "mask": "trees" } },
        "out": { "type": "output.map", "inputs": { "height": "trees" } }
    } }"#).unwrap();
    let (_, d) = compile(&g, Space::new(1024), 512);
    let errors: Vec<&str> = d.iter().filter(|d| d.severity == Severity::Error).map(|d| d.path.as_str()).collect();
    assert!(errors.contains(&"nodes.bad.inputs.mask") && errors.contains(&"nodes.out.inputs.height"), "{errors:?}");
}

/// Volumes are made brick by brick from the relief under them, cached, and
/// say so when asked by tiles.
#[test]
fn volumes_are_made_by_bricks() {
    let g = Graph::from_json(r#"{ "nodes": {
        "land": { "type": "noise.fbm", "params": { "scale": 300, "height": 40, "base": 70 } },
        "solid": { "type": "volume.from_height", "inputs": { "height": "land" } }
    } }"#).unwrap();
    let (c, d) = compile(&g, Space::new(1024), 512);
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    let e = Engine::new(Default::default());
    let solid = c.node("solid").unwrap();
    let land = e.region(&c, (c.node("land").unwrap(), 0), 0, massif_terrain::space::Rect::new(0, 0, 32, 32), &never).unwrap();
    let origins = [[0, 64, 0], [16, 64, 16], [0, 48, 0]];
    let bricks = e.bricks(&c, (solid, 0), &origins, &never).unwrap();
    for (b, o) in bricks.iter().zip(origins) {
        assert_eq!(b.origin, o);
        for (x, y, z) in [(o[0], o[1], o[2]), (o[0] + 15, o[1] + 15, o[2] + 15), (o[0] + 7, o[1] + 3, o[2] + 9)] {
            let h = land.at(x, z);
            assert!((b.at(x, y, z) - (y as f32 + 0.5 - h)).abs() < 1e-4, "({x}, {y}, {z})");
        }
    }
    let again = e.bricks(&c, (solid, 0), &origins[..1], &never).unwrap();
    assert!(std::sync::Arc::ptr_eq(&again[0], &bricks[0]), "cached");
    assert!(e.bricks(&c, (solid, 0), &[[3, 0, 0]], &never).is_err(), "bricks start on multiples of 16");
    assert!(e.region(&c, (solid, 0), 0, massif_terrain::space::Rect::new(0, 0, 16, 16), &never).is_err());
}
