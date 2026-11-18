//! The engine: tiles, levels of detail, the cache, simulations, the GPU.

use massif_terrain::compile::{Compiled, Severity, compile};
use massif_terrain::engine::{Engine, EvalError, Settings, never};
use massif_terrain::kernel::Grid;
use massif_terrain::model::{Graph, Node};
use massif_terrain::space::{Rect, Space};
use std::sync::atomic::{AtomicBool, Ordering};

fn engine() -> Engine {
    Engine::new(Settings { cache_mb: 512 })
}

/// Rolling hills, levelled, blurred, their steep parts, mixed with ridges.
fn hills() -> Graph {
    let mut g = Graph::default();
    g.terrain.seed = 7;
    g.nodes.insert("hills".into(), Node::new("noise.fbm").with("scale", 400).with("height", 200).with("base", 20));
    g.nodes.insert("relief".into(), Node::new("adjust.levels").input("in", "hills").with("in_low", 20).with("in_high", 220).with("out_high", 300).with("gamma", 1.5));
    g.nodes.insert("soft".into(), Node::new("adjust.blur").input("in", "relief").with("radius", 12));
    g.nodes.insert("steep".into(), Node::new("mask.slope").input("height", "soft").with("from", 10).with("to", 40));
    g.nodes.insert("ridges".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 300).with("height", 80).with("base", 0));
    g.nodes.insert("mixed".into(), Node::new("combine.blend").input("a", "soft").input("b", "ridges").input("mask", "steep"));
    g.nodes.insert("out".into(), Node::new("output.map").input("height", "mixed"));
    g
}

fn compiled(g: &Graph, size: u32, sim: u32) -> Compiled {
    let (c, diags) = compile(g, Space::new(size), sim);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn max_diff(a: &Grid, b: &Grid) -> f32 {
    assert_eq!(a.rect, b.rect);
    a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn mistakes_are_said_with_where_they_are() {
    let text = r#"{ "nodes": {
        "a": { "type": "noise.fmb" },
        "b": { "type": "noise.fbm", "params": { "hieght": 3, "style": "ridgd", "octaves": 40 } },
        "c": { "type": "adjust.levels", "inputs": { "inn": "b" } },
        "d": { "type": "adjust.levels", "inputs": { "in": "e" } },
        "e": { "type": "adjust.levels", "inputs": { "in": "d" } },
        "f": { "type": "adjust.levels", "inputs": { "in": "b.flow" } },
        "g": { "type": "adjust.levels", "inputs": { "in": "zz" } },
        "out": { "type": "output.map", "inputs": { "height": "b" } }
    } }"#;
    let g = Graph::from_json(text).unwrap();
    let (c, diags) = compile(&g, Space::new(1024), 512);
    let said = |path: &str, words: &str| diags.iter().any(|d| d.path == path && d.message.contains(words));
    for d in &diags {
        println!("{d}");
    }
    assert!(said("nodes.a.type", "did you mean “noise.fbm”"));
    assert!(said("nodes.b.params.hieght", "did you mean “height”"));
    assert!(said("nodes.b.params.style", "did you mean “ridged”"));
    assert!(diags.iter().any(|d| d.path == "nodes.b.params.octaves" && d.severity == Severity::Warning));
    assert!(said("nodes.c.inputs.inn", "did you mean “in”"));
    assert!(said("nodes.c.inputs", "needs its input “in”"));
    assert!(said("nodes.d.inputs", "loops back"));
    assert!(said("nodes.f.inputs.in", "gives height, not “flow”"));
    assert!(said("nodes.g.inputs.in", "no node is called “zz”"));
    // What is sound still evaluates; what is broken says why.
    let e = engine();
    let out = c.output("height").unwrap();
    assert!(e.region(&c, out, 2, Rect::new(0, 0, 64, 64), &never).is_ok());
    let broken = c.source("c", None).unwrap();
    assert!(matches!(e.region(&c, broken, 2, Rect::new(0, 0, 8, 8), &never), Err(EvalError::Broken { .. })));
}

#[test]
fn a_region_is_the_same_however_it_is_asked_for() {
    let c = compiled(&hills(), 2048, 512);
    let out = c.output("height").unwrap();
    let r = Rect::new(200, 130, 400, 300);
    let first = engine().region(&c, out, 0, r, &never).unwrap();
    // Another engine, which made a larger region around it first.
    let e = engine();
    e.region(&c, out, 0, Rect::new(-100, 0, 900, 700), &never).unwrap();
    let again = e.region(&c, out, 0, r, &never).unwrap();
    assert_eq!(first, again);
}

#[test]
fn tiled_blur_matches_a_blur_of_the_whole() {
    let c = compiled(&hills(), 2048, 512);
    let e = engine();
    let relief = c.source("relief", None).unwrap();
    let soft = c.source("soft", None).unwrap();
    // A blur of 12 blocks works at the level asked for: compare it with a
    // plain 2D Gaussian over one large region.
    let r = Rect::new(240, 240, 40, 40);
    let tiled = e.region(&c, soft, 0, r, &never).unwrap();
    let big = e.region(&c, relief, 0, r.expand(20), &never).unwrap();
    let s = 12.0f64 / 3.0;
    let k: Vec<f64> = (-12..=12).map(|d: i32| (-(d * d) as f64 / (2.0 * s * s)).exp()).collect();
    let sum: f64 = k.iter().sum();
    let mut worst = 0.0f64;
    for z in r.z..r.z1() {
        for x in r.x..r.x1() {
            let mut acc = 0.0;
            for (b, kz) in k.iter().enumerate() {
                for (a, kx) in k.iter().enumerate() {
                    acc += kz * kx * big.at(x - 12 + a as i32, z - 12 + b as i32) as f64;
                }
            }
            let reference = acc / (sum * sum);
            worst = worst.max((reference - tiled.at(x, z) as f64).abs());
        }
    }
    assert!(worst < 1e-3, "{worst}");
}

#[test]
fn coarse_levels_are_samples_of_the_fine_ones() {
    let mut g = hills();
    // Point nodes only: noise, levels, blend without a mask.
    g.nodes.remove("soft");
    g.nodes.remove("steep");
    g.link("relief", "mixed", "a");
    g.nodes.get_mut("mixed").unwrap().inputs.remove("mask");
    let c = compiled(&g, 4096, 512);
    let out = c.output("height").unwrap();
    let e = engine();
    let coarse = e.region(&c, out, 2, Rect::new(0, 0, 300, 300), &never).unwrap();
    let fine = e.region(&c, out, 0, Rect::new(0, 0, 1200, 1200), &never).unwrap();
    let mut worst = 0.0f32;
    for j in 0..300 {
        for i in 0..300 {
            worst = worst.max((coarse.at(i, j) - fine.at(4 * i, 4 * j)).abs());
        }
    }
    assert!(worst < 2e-3, "{worst}");
}

#[test]
fn a_change_recomputes_only_what_follows_it() {
    let mut g = hills();
    let e = engine();
    let r = Rect::new(0, 0, 512, 512);
    let c = compiled(&g, 2048, 512);
    e.region(&c, c.output("height").unwrap(), 1, r, &never).unwrap();
    let made = |id: &str| e.counters.of(id);
    let (hills0, ridges0, relief0, mixed0) = (made("hills"), made("ridges"), made("relief"), made("mixed"));
    assert!(hills0 > 0 && relief0 > 0);

    g.set("relief", "gamma", 0.8);
    let c2 = compiled(&g, 2048, 512);
    let changed = e.region(&c2, c2.output("height").unwrap(), 1, r, &never).unwrap();
    assert_eq!(made("hills"), hills0, "the noise before it is not made again");
    assert_eq!(made("ridges"), ridges0, "nor what does not depend on it");
    assert!(made("relief") > relief0 && made("mixed") > mixed0);

    // Undo: everything is in the cache.
    let tiles = e.counters.tiles();
    let back = e.region(&c, c.output("height").unwrap(), 1, r, &never).unwrap();
    assert_eq!(e.counters.tiles(), tiles);
    assert_ne!(back, changed);
}

#[test]
fn simulations_carry_their_change_to_finer_levels() {
    let mut g = Graph::default();
    g.nodes.insert("peaks".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 200).with("height", 300).with("base", 0));
    g.nodes.insert("worn".into(), Node::new("erosion.thermal").input("height", "peaks").with("angle", 30).with("iterations", 60));
    g.nodes.insert("out".into(), Node::new("output.map").input("height", "worn"));
    // 1024 blocks simulated at 128 samples: a sample every 8 blocks.
    let c = compiled(&g, 1024, 128);
    assert_eq!(c.sim_lod(), 3);
    let e = engine();
    let peaks = c.source("peaks", None).unwrap();
    let worn = c.source("worn", None).unwrap();
    let before = e.region(&c, peaks, 3, c.space.whole(3), &never).unwrap();
    let after = e.region(&c, worn, 3, c.space.whole(3), &never).unwrap();
    let steepest = |g: &Grid| {
        let mut s = 0.0f32;
        for z in 0..127 {
            for x in 0..127 {
                s = s.max((g.at(x + 1, z) - g.at(x, z)).abs()).max((g.at(x, z + 1) - g.at(x, z)).abs());
            }
        }
        s / 8.0
    };
    assert!(steepest(&after) < steepest(&before) * 0.8, "{} {}", steepest(&after), steepest(&before));
    // Mass stays: material moves, none is lost.
    let total = |g: &Grid| g.data.iter().map(|v| *v as f64).sum::<f64>();
    assert!((total(&after) - total(&before)).abs() / total(&before) < 1e-4);
    // At the finest level, where samples coincide with the simulation's,
    // the change is exactly the simulation's; between, it is interpolated.
    let fine_in = e.region(&c, peaks, 0, Rect::new(0, 0, 256, 256), &never).unwrap();
    let fine_out = e.region(&c, worn, 0, Rect::new(0, 0, 256, 256), &never).unwrap();
    for (i, j) in [(0, 0), (5, 9), (31, 20)] {
        let change = after.at(i, j) - before.at(i, j);
        let carried = fine_out.at(8 * i, 8 * j) - fine_in.at(8 * i, 8 * j);
        assert!((change - carried).abs() < 1e-3, "{change} {carried}");
    }
    assert_eq!(e.counters.globals.load(Ordering::Relaxed), 1, "the simulation ran once");
    // The talus output is a mask.
    let talus = e.region(&c, c.source("worn", Some("talus")).unwrap(), 0, Rect::new(0, 0, 256, 256), &never).unwrap();
    let (lo, hi) = talus.range();
    assert!(lo >= 0.0 && hi <= 1.0 && hi > 0.0);
}

#[test]
fn normalize_stretches_to_its_range() {
    let mut g = Graph::default();
    g.nodes.insert("n".into(), Node::new("noise.fbm").with("scale", 300).with("height", 37).with("base", 11));
    g.nodes.insert("norm".into(), Node::new("adjust.normalize").input("in", "n").with("low", 60).with("high", 260));
    let c = compiled(&g, 1024, 256);
    let e = engine();
    let all = e.region(&c, c.source("norm", None).unwrap(), 2, c.space.whole(2), &never).unwrap();
    let (lo, hi) = all.range();
    assert!((lo - 60.0).abs() < 0.5 && (hi - 260.0).abs() < 0.5, "{lo} {hi}");
}

#[test]
fn wide_blurs_work_coarse_and_come_back_smooth() {
    let mut g = hills();
    g.set("soft", "radius", 900);
    let c = compiled(&g, 4096, 512);
    let e = engine();
    let started = std::time::Instant::now();
    let soft = e.region(&c, c.source("soft", None).unwrap(), 0, Rect::new(1000, 1000, 512, 512), &never).unwrap();
    let ms = started.elapsed().as_millis();
    // Smooth: neighbouring samples barely differ.
    let mut step = 0.0f32;
    for z in 1000..1511 {
        for x in 1000..1511 {
            step = step.max((soft.at(x + 1, z) - soft.at(x, z)).abs());
        }
    }
    assert!(step < 0.2, "{step}");
    assert!(ms < 5000, "{ms} ms");
}

#[test]
fn cancelling_stops_the_work() {
    let c = compiled(&hills(), 8192, 512);
    let stop = AtomicBool::new(true);
    let r = engine().region(&c, c.output("height").unwrap(), 0, Rect::new(0, 0, 2048, 2048), &|| stop.load(Ordering::Relaxed));
    assert_eq!(r, Err(EvalError::Cancelled));
}

#[test]
fn the_map_output_takes_typed_inputs() {
    let text = r##"{ "nodes": {
        "hills": { "type": "noise.fbm", "params": { "scale": 300, "height": 200, "base": 0 } },
        "paint": { "type": "color.ramp", "inputs": { "height": "hills" }, "params": { "low": 0, "high": 200, "color_low": "#000000", "color_high": "#ffffff" } },
        "bright": { "type": "adjust.levels", "inputs": { "in": "paint" }, "params": { "in_low": 0, "in_high": 1, "out_low": 0, "out_high": 100 } },
        "steep": { "type": "mask.slope", "inputs": { "height": "hills" } },
        "out": { "type": "output.map", "inputs": { "height": "hills", "albedo": "paint", "mask.snow": "steep", "mask.moss": "steep" } },
        "draft": { "type": "output.map", "inputs": { "height": "bright" }, "params": { "active": false } }
    } }"##;
    let g = Graph::from_json(text).unwrap();
    let (c, diags) = compile(&g, Space::new(1024), 256);
    assert!(diags.is_empty(), "{diags:?}");
    let out = c.map_output().unwrap();
    assert_eq!(c.nodes[out.node].id, "out", "the active output");
    let names: Vec<&str> = out.inputs.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(names, ["height", "albedo", "mask.snow", "mask.moss"]);
    assert_eq!(c.output("mask.moss"), c.source("steep", None));
    // A colour feeding a height goes through its brightness, a node the
    // compiled graph adds.
    assert!(c.nodes.iter().any(|n| n.is_internal() && n.spec.id == "convert.luminance"));
    let e = engine();
    let r = Rect::new(0, 0, 64, 64);
    let colour = e.region(&c, c.output("albedo").unwrap(), 1, r, &never).unwrap();
    assert_eq!(colour.planes, 3);
    let bright = e.region(&c, c.source("bright", None).unwrap(), 1, r, &never).unwrap();
    for (x, z) in [(3, 4), (40, 17)] {
        let lum = 0.2126 * colour.at_plane(0, x, z) + 0.7152 * colour.at_plane(1, x, z) + 0.0722 * colour.at_plane(2, x, z);
        assert!((bright.at(x, z) - 100.0 * lum).abs() < 1e-3);
    }
    // What would need a node to say how is refused, with the reason.
    let bad = Graph::from_json(r#"{ "nodes": { "n": { "type": "noise.fbm" }, "out": { "type": "output.map", "inputs": { "terrain": "n", "mask.Bad name": "n" } } } }"#).unwrap();
    let (_, diags) = compile(&bad, Space::new(1024), 256);
    assert!(diags.iter().any(|d| d.path == "nodes.out.inputs.terrain" && d.message.contains("cannot take")), "{diags:?}");
    assert!(diags.iter().any(|d| d.path == "nodes.out.inputs.mask.Bad name"), "{diags:?}");
}

#[test]
fn fields_drive_settings() {
    let text = r#"{ "nodes": {
        "zero": { "type": "pattern.constant", "params": { "value": 0 } },
        "lift": { "type": "pattern.constant", "params": { "value": 100 } },
        "flat": { "type": "noise.fbm", "inputs": { "@height": "zero", "@base": "lift" } },
        "half": { "type": "noise.fbm", "inputs": { "@height": "lift" }, "params": { "seed": 5, "base": 0 } },
        "same": { "type": "noise.fbm", "params": { "seed": 5, "base": 0, "height": 100 } }
    } }"#;
    let g = Graph::from_json(text).unwrap();
    let (c, diags) = compile(&g, Space::new(1024), 256);
    assert!(diags.is_empty(), "{diags:?}");
    let e = engine();
    let r = Rect::new(0, 0, 300, 300);
    let flat = e.region(&c, c.source("flat", None).unwrap(), 1, r, &never).unwrap();
    assert!(flat.data.iter().all(|v| *v == 100.0));
    // Driven by a field of 100 everywhere is the same as set to 100.
    let half = e.region(&c, c.source("half", None).unwrap(), 1, r, &never).unwrap();
    let same = e.region(&c, c.source("same", None).unwrap(), 1, r, &never).unwrap();
    assert_eq!(half, same);
    // Only numbers made to be driven can be, and a near miss is named.
    let bad = Graph::from_json(r#"{ "nodes": { "z": { "type": "pattern.constant" }, "n": { "type": "noise.fbm", "inputs": { "@style": "z", "@heigth": "z" } } } }"#).unwrap();
    let (_, diags) = compile(&bad, Space::new(1024), 256);
    assert!(diags.iter().any(|d| d.path == "nodes.n.inputs.@style" && d.message.contains("cannot be driven")), "{diags:?}");
    assert!(diags.iter().any(|d| d.path == "nodes.n.inputs.@heigth" && d.message.contains("“@height”")), "{diags:?}");
}

#[test]
fn the_graphics_card_computes_what_the_cpu_does() {
    use massif_terrain::gpu::Gpu;
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no graphics card: skipped");
        return;
    };
    eprintln!("on {}", gpu.name());
    let mut g = Graph::default();
    g.terrain.seed = 99;
    g.nodes.insert("flat".into(), Node::new("pattern.constant").with("value", 70));
    g.nodes.insert("rolling".into(), Node::new("noise.fbm").with("scale", 700).with("height", 250).with("offset_x", 31_000.5));
    g.nodes.insert("ridges".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 90).with("octaves", 12));
    g.nodes.insert("puffs".into(), Node::new("noise.fbm").with("style", "billow").with("scale", 40).with("height", 1));
    g.nodes.insert("lev".into(), Node::new("adjust.levels").input("in", "rolling").with("in_low", 40).with("in_high", 290).with("out_low", 10).with("out_high", 400).with("gamma", 2.2));
    g.nodes.insert("sum".into(), Node::new("combine.blend").input("a", "lev").input("b", "ridges").input("mask", "puffs").with("mode", "max"));
    g.nodes.insert("over".into(), Node::new("combine.blend").input("a", "sum").input("b", "flat").with("mode", "overlay").with("strength", 0.3));
    g.nodes.insert("driven".into(), Node::new("noise.fbm").input("@height", "puffs").input("@base", "lev").with("scale", 150));
    g.nodes.insert("out".into(), Node::new("output.map").input("height", "over"));
    let c = compiled(&g, 65536, 2048);
    let cpu = engine();
    let gpu = Engine::with_gpu(gpu, Settings { cache_mb: 512 });
    for (lod, rect) in [(0, Rect::new(30_000, 40_000, 600, 300)), (4, Rect::new(0, 0, 512, 512))] {
        for id in ["flat", "rolling", "ridges", "puffs", "lev", "sum", "over", "driven"] {
            let src = c.source(id, None).unwrap();
            let a = cpu.region(&c, src, lod, rect, &never).unwrap();
            let b = gpu.region(&c, src, lod, rect, &never).unwrap();
            let d = max_diff(&a, &b);
            let (lo, hi) = a.range();
            assert!(d <= 1e-4 * (hi - lo).abs().max(1.0), "{id} at level {lod}: {d} over {lo}..{hi}");
        }
    }
    assert!(gpu.counters.gpu_tiles.load(Ordering::Relaxed) > 0);
}

#[test]
fn simulations_on_the_graphics_card_match_the_cpu() {
    use massif_terrain::gpu::Gpu;
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no graphics card: skipped");
        return;
    };
    let mut g = Graph::default();
    g.nodes.insert("peaks".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 200).with("height", 300).with("base", 0));
    g.nodes.insert("worn".into(), Node::new("erosion.thermal").input("height", "peaks").with("angle", 30).with("iterations", 41));
    let c = compiled(&g, 1024, 256);
    let cpu = engine();
    let on_gpu = Engine::with_gpu(gpu, Settings { cache_mb: 512 });
    for (port, tolerance) in [(None, 0.05), (Some("talus"), 0.03)] {
        let src = c.source("worn", port).unwrap();
        for lod in [2, 0] {
            let r = Rect::new(100, 100, 200, 200);
            let a = cpu.region(&c, src, lod, r, &never).unwrap();
            let b = on_gpu.region(&c, src, lod, r, &never).unwrap();
            let d = max_diff(&a, &b);
            assert!(d < tolerance, "{port:?} at level {lod}: {d}");
        }
    }
    assert_eq!(on_gpu.counters.gpu_globals.load(Ordering::Relaxed), 1);
}
