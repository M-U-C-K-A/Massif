//! The light primitives: the Mountain, Transform at full detail, and the
//! Combine whose inputs grow as they are connected.

use massif_terrain::compile::{Compiled, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::kernel::Grid;
use massif_terrain::model::Graph;
use massif_terrain::mscript;
use massif_terrain::space::{Rect, Space};
use std::time::Instant;

fn compiled(json: &str, size: u32) -> Compiled {
    let g = Graph::from_json(json).unwrap();
    let (c, diags) = compile(&g, Space::new(size), 512);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn whole(c: &Compiled, id: &str, lod: u8) -> Grid {
    Engine::new(Settings::default()).whole(c, c.source(id, None).unwrap(), lod, &never).unwrap()
}

/// Five reliefs, each a noise of its own.
const FIVE: &str = r#""a": { "type": "noise.fbm", "params": { "seed": 1, "scale": 300 } },
    "b": { "type": "noise.fbm", "params": { "seed": 2, "scale": 500 } },
    "c": { "type": "noise.fbm", "params": { "seed": 3, "scale": 700 } },
    "d": { "type": "noise.fbm", "params": { "seed": 4, "scale": 900 } },
    "e": { "type": "noise.fbm", "params": { "seed": 5, "scale": 1100 } },
    "m": { "type": "pattern.gradient" }"#;

#[test]
fn combine_takes_five_inputs() {
    let json = format!(r#"{{ "nodes": {{ {FIVE},
        "max": {{ "type": "combine.layers", "inputs": {{ "in.1": "a", "in.2": "b", "in.3": "c", "in.4": "d", "in.10": "e" }} }},
        "avg": {{ "type": "combine.layers", "params": {{ "mode": "average" }}, "inputs": {{ "in.1": "a", "in.2": "b", "in.3": "c", "in.4": "d", "in.5": "e" }} }},
        "lay": {{ "type": "combine.layers", "params": {{ "mode": "blend" }}, "inputs": {{ "in.1": "a", "in.2": "b", "mask.2": "m", "in.3": "c", "mask.3": "m" }} }} }} }}"#);
    let c = compiled(&json, 2048);
    let ins: Vec<Grid> = ["a", "b", "c", "d", "e"].iter().map(|id| whole(&c, id, 3)).collect();
    let (max, avg, lay, m) = (whole(&c, "max", 3), whole(&c, "avg", 3), whole(&c, "lay", 3), whole(&c, "m", 3));
    for k in 0..max.data.len() {
        let want = ins.iter().map(|g| g.data[k]).fold(f32::MIN, f32::max);
        assert!((max.data[k] - want).abs() < 1e-3, "max at {k}: {} vs {want}", max.data[k]);
        let mean = ins.iter().map(|g| g.data[k]).sum::<f32>() / 5.0;
        assert!((avg.data[k] - mean).abs() < 1e-2, "average at {k}");
        // Each laid over the one before through the mask.
        let w = m.data[k].clamp(0.0, 1.0);
        let ab = ins[0].data[k] + (ins[1].data[k] - ins[0].data[k]) * w;
        let want = ab + (ins[2].data[k] - ab) * w;
        assert!((lay.data[k] - want).abs() < 1e-2, "blend at {k}");
    }
}

#[test]
fn combine_inputs_go_by_number() {
    // in.10 comes after in.2, whatever the names' order as text.
    let json = r#"{ "nodes": { "lo": { "type": "pattern.constant", "params": { "value": 10 } }, "hi": { "type": "pattern.constant", "params": { "value": 50 } },
        "n": { "type": "combine.layers", "params": { "mode": "subtract" }, "inputs": { "in.10": "lo", "in.2": "hi" } } } }"#;
    let g = whole(&compiled(json, 1024), "n", 4);
    assert!(g.data.iter().all(|v| (v - 40.0).abs() < 1e-4), "{:?}", &g.data[..4]);
}

#[test]
fn combine_in_massifscript() {
    let src = "a = FractalNoise(seed: 1);\nb = FractalNoise(seed: 2);\nc = FractalNoise(seed: 3);\nm = Gradient();\nall = Mixer(a, b, c, `mask.2`: m, mode: \"add\");\noutput height = all;\n";
    let out = mscript::compile(src, &mut mscript::NoImports);
    assert!(out.ok(), "{:?}", out.errors);
    let g = out.graph.unwrap();
    let n = &g.nodes["all"];
    assert_eq!(n.kind, "combine.layers");
    assert_eq!((n.inputs["in.1"].as_str(), n.inputs["in.2"].as_str(), n.inputs["in.3"].as_str(), n.inputs["mask.2"].as_str()), ("a", "b", "c", "m"));
    // Written back as it was given.
    let text = mscript::decompile(&g, None);
    assert!(text.contains("Mixer(a, b, c, `mask.2`: m, mode: \"add\")"), "{text}");
    let again = mscript::compile(&text, &mut mscript::NoImports);
    assert!(again.ok(), "{:?}", again.errors);
    assert_eq!(again.graph.unwrap().nodes["all"].inputs, n.inputs);
}

#[test]
fn transform_moves_at_full_detail() {
    // Moved a quarter of the terrain east: each place shows the relief a
    // quarter west of it, sample for sample at level 0.
    let json = r#"{ "nodes": { "r": { "type": "noise.fbm", "params": { "seed": 7, "scale": 60 } },
        "t": { "type": "adjust.transform@2", "params": { "offset_x": 0.25 }, "inputs": { "in": "r" } } } }"#;
    let c = compiled(json, 1024);
    let e = Engine::new(Settings::default());
    let rect = Rect::new(512, 300, 256, 64);
    let moved = e.region(&c, c.source("t", None).unwrap(), 0, rect, &never).unwrap();
    let src = e.region(&c, c.source("r", None).unwrap(), 0, Rect::new(256, 300, 256, 64), &never).unwrap();
    let worst = moved.data.iter().zip(&src.data).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-3, "{worst}");
}

#[test]
fn transform_turns_and_scales() {
    // A gradient west to east, turned 90° clockwise, runs north to south.
    let json = r#"{ "nodes": { "g": { "type": "pattern.gradient" },
        "t": { "type": "adjust.transform@2", "params": { "rotate": 90 }, "inputs": { "in": "g" } },
        "s": { "type": "adjust.transform@2", "params": { "scale": 0.25, "deform": 40 }, "inputs": { "in": "g" } } } }"#;
    let c = compiled(json, 1024);
    let (g, t) = (whole(&c, "g", 2), whole(&c, "t", 2));
    let n = g.rect.w as i32;
    for k in [10, 60, 120, 200] {
        let (a, b) = (g.at(k, n / 2), t.at(n / 2, k));
        assert!((a - b).abs() < 0.03, "{k}: {a} vs {b}");
    }
    let s = whole(&c, "s", 2);
    assert!(s.data.iter().all(|v| v.is_finite()));
}

#[test]
fn the_mountain_rises_in_the_middle() {
    for style in ["basic", "eroded", "old", "alpine", "strata"] {
        for bulk in ["low", "medium", "high"] {
            let json = format!(r#"{{ "nodes": {{ "m": {{ "type": "mountain.single", "params": {{ "style": "{style}", "bulk": "{bulk}", "scale": 1600, "height": 300 }} }} }} }}"#);
            let g = whole(&compiled(&json, 2048), "m", 2);
            let (lo, hi) = g.range();
            assert!(lo >= 0.0 && hi <= 300.0 && hi > 150.0, "{style} {bulk}: {lo}..{hi}");
            // The summit near the middle, nothing past its foot.
            let n = g.rect.w as i32;
            let (mut best, mut at) = (f32::MIN, (0, 0));
            for z in 0..n {
                for x in 0..n {
                    if g.at(x, z) > best {
                        best = g.at(x, z);
                        at = (x, z);
                    }
                }
            }
            let d = (((at.0 - n / 2).pow(2) + (at.1 - n / 2).pow(2)) as f64).sqrt() * 4.0;
            assert!(d < 450.0, "{style} {bulk}: summit {d} blocks from the middle");
            assert_eq!(g.at(4, 4), 0.0, "{style} {bulk}");
        }
    }
}

/// One tile at full detail on one core, the best of a few runs, in ms.
fn tile_ms(json: &str, id: &str) -> f64 {
    let c = compiled(json, 4096);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    (0..3)
        .map(|_| {
            let e = Engine::new(Settings::default());
            pool.install(|| {
                let t = Instant::now();
                e.region(&c, c.source(id, None).unwrap(), 0, Rect::tile(7, 7), &never).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
        })
        .fold(f64::MAX, f64::min)
}

/// `cargo test --release -p massif-terrain --test primitives -- --ignored --nocapture`
#[test]
#[ignore]
fn timings() {
    let alpine = tile_ms(r#"{ "nodes": { "m": { "type": "mountain.alpine" } } }"#, "m");
    let mountain = tile_ms(r#"{ "nodes": { "m": { "type": "mountain.single", "params": { "scale": 3000 } } } }"#, "m");
    let reduced = tile_ms(r#"{ "nodes": { "m": { "type": "mountain.single", "params": { "scale": 3000, "reduce_details": true } } } }"#, "m");
    println!("one tile at level 0 on one core: alpine mountains {alpine:.1} ms, Mountain {mountain:.1} ms, reduced details {reduced:.1} ms");
    let five = format!(r#"{{ "nodes": {{ {FIVE}, "n": {{ "type": "combine.layers", "inputs": {{ "in.1": "a", "in.2": "b", "in.3": "c", "in.4": "d", "in.5": "e" }} }} }} }}"#);
    let blends = format!(r#"{{ "nodes": {{ {FIVE}, "ab": {{ "type": "combine.blend", "params": {{ "mode": "max" }}, "inputs": {{ "a": "a", "b": "b" }} }}, "abc": {{ "type": "combine.blend", "params": {{ "mode": "max" }}, "inputs": {{ "a": "ab", "b": "c" }} }}, "abcd": {{ "type": "combine.blend", "params": {{ "mode": "max" }}, "inputs": {{ "a": "abc", "b": "d" }} }}, "n": {{ "type": "combine.blend", "params": {{ "mode": "max" }}, "inputs": {{ "a": "abcd", "b": "e" }} }} }} }}"#);
    println!("five reliefs combined: one Combine {:.1} ms, four Blends {:.1} ms", tile_ms(&five, "n"), tile_ms(&blends, "n"));
    // A preview: the whole of a 4096 terrain at level 3, all cores.
    for (label, t) in [("Transform 1", "adjust.transform"), ("Transform 2", "adjust.transform@2")] {
        let json = format!(r#"{{ "nodes": {{ "m": {{ "type": "mountain.single", "params": {{ "scale": 2500 }} }}, "t": {{ "type": "{t}", "params": {{ "rotate": 30, "scale": 1.2, "offset_x": 0.1 }}, "inputs": {{ "in": "m" }} }} }} }}"#);
        let c = compiled(&json, 4096);
        let best = (0..3)
            .map(|_| {
                let e = Engine::new(Settings::default());
                let t = Instant::now();
                e.whole(&c, c.source("t", None).unwrap(), 3, &never).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .fold(f64::MAX, f64::min);
        println!("{label}: a 4096 terrain at level 3 (512²): {best:.1} ms");
    }
}
