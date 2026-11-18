//! Colours and terrains.

use massif_terrain::compile::{Compiled, compile_in};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::kernel::{Env, Grid, NoEnv};
use massif_terrain::model::Graph;
use massif_terrain::params::{linear, parse_color};
use massif_terrain::space::{Rect, Space};
use std::sync::Arc;

/// A world with a few terrains: their place in the list, plus one.
struct Terrains;

const NAMES: [&str; 4] = ["grass", "rock", "sand", "snow"];

impl Env for Terrains {
    fn terrain_id(&self, name: &str) -> Option<u16> {
        NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)).map(|i| i as u16 + 1)
    }
}

fn compiled(json: &str, env: Arc<dyn Env>) -> Compiled {
    let (c, diags) = compile_in(&Graph::from_json(json).unwrap(), Space::new(1024), 256, env);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn grid(c: &Compiled, id: &str, port: Option<&str>) -> Grid {
    Engine::new(Settings::default()).region(c, c.source(id, port).unwrap(), 2, Rect::new(0, 0, 256, 256), &never).unwrap()
}

fn rgb(g: &Grid, x: i32, z: i32) -> [f32; 3] {
    [g.at_plane(0, x, z), g.at_plane(1, x, z), g.at_plane(2, x, z)]
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 2e-3)
}

#[test]
fn gradients_mix_and_adjust() {
    let c = compiled(
        r##"{ "nodes": {
        "ramp": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "height": 100, "base": 50 } },
        "g": { "type": "color.gradient", "inputs": { "in": "ramp" }, "params": { "preset": "custom", "gradient": [[0, "#ff0000"], [1, "#0000ff"]], "low": 50, "high": 150, "variation": 0 } },
        "auto": { "type": "color.gradient", "inputs": { "in": "ramp" }, "params": { "preset": "custom", "gradient": [[0, "#000000"], [1, "#ffffff"]], "low": 0, "high": 1000, "auto": true, "variation": 0 } },
        "red": { "type": "color.solid", "params": { "color": "#ff0000" } },
        "white": { "type": "color.solid", "params": { "color": "#ffffff" } },
        "half": { "type": "pattern.constant", "params": { "value": 0.5 } },
        "mixed": { "type": "color.mix", "inputs": { "a": "red", "b": "white", "mask": "half" } },
        "grey": { "type": "color.adjust", "inputs": { "color": "red" }, "params": { "saturation": 0 } },
        "green": { "type": "color.adjust", "inputs": { "color": "red" }, "params": { "hue": 120 } }
    } }"##,
        Arc::new(NoEnv),
    );
    let g = grid(&c, "g", None);
    assert!(near(rgb(&g, 0, 10), linear(parse_color("#ff0000").unwrap())));
    let end = rgb(&g, 255, 10);
    assert!(end[2] > 0.99 && end[0] < 0.01, "{end:?}");
    // Auto: the ramp's own range, 50 to ~150.
    let a = grid(&c, "auto", None);
    assert!(rgb(&a, 0, 10)[0] < 0.01 && rgb(&a, 255, 10)[0] > 0.97, "{:?} {:?}", rgb(&a, 0, 10), rgb(&a, 255, 10));
    let m = rgb(&grid(&c, "mixed", None), 5, 5);
    assert!(near(m, [1.0, 0.5, 0.5]), "{m:?}");
    let grey = rgb(&grid(&c, "grey", None), 5, 5);
    assert!((grey[0] - grey[1]).abs() < 1e-4 && (grey[1] - grey[2]).abs() < 1e-4);
    let green = rgb(&grid(&c, "green", None), 5, 5);
    assert!(green[1] > 0.95 && green[0] < 0.01 && green[2] < 0.01, "{green:?}");
}

#[test]
fn terrain_layers_and_their_selection() {
    let c = compiled(
        r#"{ "nodes": {
        "left": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "profile": [[0, 1], [0.5, 1], [0.5001, 0], [1, 0]] } },
        "all": { "type": "pattern.constant", "params": { "value": 1 } },
        "layers": { "type": "terrain.layers", "inputs": { "mask_1": "left", "mask_3": "all" }, "params": { "base": "Grass", "terrain_1": "Rock", "terrain_3": "", "dither": 0 } },
        "sandy": { "type": "terrain.layers", "inputs": { "mask_1": "left", "mask_2": "all" }, "params": { "terrain_1": "Rock", "terrain_2": "Sand", "dither": 0 } },
        "rock": { "type": "terrain.select", "inputs": { "terrain": "layers" }, "params": { "name": "rock" } }
    } }"#,
        Arc::new(Terrains),
    );
    let t = grid(&c, "layers", None);
    // Rock on the left half, the base (grass) on the right; a layer with no
    // terrain named lays nothing.
    assert_eq!(t.at(10, 10), 2.0);
    assert_eq!(t.at(240, 10), 1.0);
    // A later layer covers an earlier one.
    assert_eq!(grid(&c, "sandy", None).at(10, 10), 3.0);
    let sel = grid(&c, "rock", None);
    assert_eq!((sel.at(10, 10), sel.at(240, 10)), (1.0, 0.0));
    // The preview colours are the terrains'.
    let col = grid(&c, "layers", Some("color"));
    assert_eq!(col.planes, 3);
    assert!(col.at_plane(1, 240, 10) > col.at_plane(0, 240, 10), "grass is green");
}
