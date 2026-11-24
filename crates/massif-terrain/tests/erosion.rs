//! The erosion nodes: what they do to a relief, the graphics card against
//! the CPU, and renders of the shapes before and after (ignored by
//! default; `MASSIF_EROSION_RENDERS=<dir> cargo test --release -p
//! massif-terrain --test erosion -- --ignored --nocapture renders`).

use massif_terrain::compile::{Compiled, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::gpu::Gpu;
use massif_terrain::kernel::Grid;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::time::Instant;

fn compiled(g: &Graph, size: u32, sim: u32) -> Compiled {
    let (c, diags) = compile(g, Space::new(size), sim);
    assert!(diags.iter().all(|d| d.severity != massif_terrain::compile::Severity::Error), "{diags:?}");
    c
}

fn engine(gpu: bool) -> Engine {
    match gpu.then(Gpu::headless).flatten() {
        Some(g) => Engine::with_gpu(g, Settings { cache_mb: 2048 }),
        None => Engine::new(Settings { cache_mb: 2048 }),
    }
}

fn shade(g: &Grid, sea: f32, mask: Option<&Grid>) -> Vec<[u8; 3]> {
    let mut rgb = massif_terrain::preview::shade(g, sea);
    if let Some(m) = mask {
        massif_terrain::preview::overlay(&mut rgb, m, [236, 192, 64], 0.75);
    }
    rgb
}

fn write(path: &std::path::Path, rgb: &[[u8; 3]], w: u32, h: u32) {
    let file = std::fs::File::create(path).expect("the picture can be written");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgb.as_flattened()).unwrap();
}

/// Pictures `n` a side laid out in `cols` columns, each halved.
fn sheet(path: &std::path::Path, pics: &[Vec<[u8; 3]>], n: u32, cols: u32) {
    let h = n / 2;
    let rows = (pics.len() as u32).div_ceil(cols);
    let (w, ht) = (h * cols, h * rows);
    let mut out = vec![[24u8; 3]; (w * ht) as usize];
    for (i, p) in pics.iter().enumerate() {
        let (ox, oz) = ((i as u32 % cols) * h, (i as u32 / cols) * h);
        for z in 0..h {
            for x in 0..h {
                let s = |dx: u32, dz: u32| p[((2 * z + dz) * n + 2 * x + dx) as usize];
                let c = [0, 1, 2].map(|q| ((s(0, 0)[q] as u32 + s(1, 0)[q] as u32 + s(0, 1)[q] as u32 + s(1, 1)[q] as u32) / 4) as u8);
                out[((oz + z) * w + ox + x) as usize] = c;
            }
        }
    }
    write(path, &out, w, ht);
}

/// A graph from JSON text, with `{node}` standing for the erosion chain's
/// input.
fn graph(text: &str) -> Graph {
    Graph::from_json(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

/// Renders a graph's `before` and `after` nodes (and masks of `after`)
/// into `MASSIF_EROSION_RENDERS`, named `<name>-*.png`.
fn render(name: &str, text: &str, before: &str, after: &str, masks: &[&str]) {
    let Ok(dir) = std::env::var("MASSIF_EROSION_RENDERS") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let px: u32 = std::env::var("MASSIF_EROSION_PX").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let sim: u32 = std::env::var("MASSIF_EROSION_SIM").ok().and_then(|s| s.parse().ok()).unwrap_or(2048);
    let gpu = std::env::var("MASSIF_EROSION_CPU").is_err();
    let g = graph(text);
    let c = compiled(&g, 8192, sim);
    let e = engine(gpu);
    let lod = c.space.lod_for(px);
    let whole = c.space.whole(lod);
    let b = e.region(&c, c.source(before, None).unwrap(), lod, whole, &never).unwrap();
    let started = Instant::now();
    let a = e.region(&c, c.source(after, None).unwrap(), lod, whole, &never).unwrap();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let sea = c.space.sea_level;
    let (pb, pa) = (shade(&b, sea, None), shade(&a, sea, None));
    write(&dir.join(format!("{name}-before.png")), &pb, b.rect.w, b.rect.h);
    write(&dir.join(format!("{name}-after.png")), &pa, a.rect.w, a.rect.h);
    let mut pics = vec![pb, pa];
    for m in masks {
        if *m == "~water" {
            // The water, in blue, where it lies over the ground.
            let w = e.region(&c, c.source(after, Some("water")).unwrap(), lod, whole, &never).unwrap();
            let wet = Grid::new(lod, whole, w.data.iter().zip(&a.data).map(|(w, h)| if w - h > 0.25 { 1.0 } else { 0.0 }).collect());
            let mut rgb = shade(&a, sea, None);
            massif_terrain::preview::overlay(&mut rgb, &wet, [40, 110, 200], 0.85);
            pics.push(rgb);
            continue;
        }
        let g = e.region(&c, c.source(after, Some(m)).unwrap(), lod, whole, &never).unwrap();
        let mean = g.data.iter().sum::<f32>() / g.data.len() as f32;
        println!("  {m}: {:?}, mean {mean:.3}", g.range());
        pics.push(shade(&a, sea, Some(&g)));
    }
    sheet(&dir.join(format!("{name}-sheet.png")), &pics, a.rect.w, 2);
    let (lo, hi) = b.range();
    let (alo, ahi) = a.range();
    let on = if e.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed) > 0 { e.gpu_name().unwrap_or("?") } else { "the CPU" };
    println!("{name}: {ms:.0} ms on {on}; before {lo:.0}–{hi:.0}, after {alo:.0}–{ahi:.0}");
}

/// Extra settings for the node `er`, from `MASSIF_EROSION_PARAMS`
/// (`"strength": 2, …`), to try settings without rebuilding.
fn extra() -> String {
    std::env::var("MASSIF_EROSION_PARAMS").map(|s| format!(", \"params\": {{ {s} }}")).unwrap_or_default()
}

/// The shapes of M2 the erosion is judged on.
const ALPINE: &str = r#""shape": { "type": "mountain.alpine" }"#;
const RANGE: &str = r#""shape": { "type": "mountain.range", "params": { "spine": [ { "x": 0.1, "z": 0.7 }, { "x": 0.35, "z": 0.45, "height": 1.3 }, { "x": 0.6, "z": 0.42, "width": 1.4 }, { "x": 0.9, "z": 0.2, "height": 0.7 } ], "height": 240, "width": 1900 } }"#;
const SEA: &str = r#""coast": { "type": "coast.coastline", "params": { "land": 0.6, "scale": 7000, "height": 90 } },
    "alps": { "type": "mountain.alpine", "inputs": { "area": "inner" }, "params": { "base": 0, "height": 200 } },
    "inner": { "type": "adjust.levels", "inputs": { "in": "coast.land" }, "params": { "in_low": 0.5, "in_high": 1, "out_low": 0, "out_high": 1 } },
    "far": { "type": "adjust.blur", "inputs": { "in": "inner" }, "params": { "radius": 900 } },
    "shape": { "type": "combine.blend", "inputs": { "a": "coast", "b": "alps", "mask": "far" }, "params": { "mode": "add" } }"#;
const MESAS: &str = r#""m": { "type": "plateau.mesa", "params": { "height": 70, "scale": 1200, "base": 110 } },
    "shape": { "type": "valley.canyon", "inputs": { "height": "m" }, "params": { "depth": 40, "width": 180, "scale": 2400 } }"#;

/// Renders of every erosion node on the shapes it suits (ignored by
/// default; `MASSIF_EROSION_RENDERS=<dir>`, `MASSIF_EROSION_ONLY=<names>`).
#[test]
#[ignore]
fn node_renders() {
    let only = std::env::var("MASSIF_EROSION_ONLY").unwrap_or_default();
    // Name, shape, seed, the chain from `shape` to `er` (more nodes), the
    // node shown before, masks of `er`.
    type Case<'a> = (&'a str, &'a str, u32, &'a str, &'a str, &'a [&'a str]);
    let cases: &[Case] = &[
        ("stream-alpine", ALPINE, 13, r#""er": { "type": "erosion.stream", "inputs": { "height": "shape" } }"#, "shape", &["flow", "wear", "deposition"]),
        ("stream-range", RANGE, 11, r#""er": { "type": "erosion.stream", "inputs": { "height": "shape" } }"#, "shape", &["flow", "wear", "deposition"]),
        ("stream-sea", SEA, 18, r#""er": { "type": "erosion.stream", "inputs": { "height": "shape" } }"#, "shape", &["flow", "wear", "deposition"]),
        ("stream-mesas", MESAS, 22, r#""er": { "type": "erosion.stream", "inputs": { "height": "shape" }, "params": { "rebuild": 0, "strength": 0.05 } }"#, "shape", &["flow", "wear", "deposition"]),
        ("particle-alpine", ALPINE, 13, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "erosion.particle", "inputs": { "height": "st" } }"#, "st", &["flow", "wear", "deposition"]),
        ("particle-mesas", MESAS, 22, r#""er": { "type": "erosion.particle", "inputs": { "height": "shape" }, "params": { "rain": 2 } }"#, "shape", &["flow", "wear", "deposition"]),
        ("deposit-range", RANGE, 11, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "sediment.deposit", "inputs": { "height": "st" } }"#, "st", &["deposition", "wear", "sediment"]),
        ("glacier-alpine", ALPINE, 13, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "ice.glacier", "inputs": { "height": "st" }, "params": { "ela": 170 } }"#, "st", &["ice", "wear", "deposition"]),
        ("glacier-sea", SEA, 18, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "ice.glacier", "inputs": { "height": "st" }, "params": { "ela": 130 } }"#, "st", &["ice", "wear", "deposition"]),
        ("wind-mesas", MESAS, 22, r#""er": { "type": "wind.erosion", "inputs": { "height": "shape" }, "params": { "direction": 30, "abrasion": 1 } }"#, "shape", &["sand", "wear"]),
        ("rivers-sea", SEA, 18, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "water.rivers", "inputs": { "height": "st" } }"#, "st", &["rivers", "~water"]),
        ("lakes-alpine", ALPINE, 13, r#""er": { "type": "water.lakes", "inputs": { "height": "shape" } }"#, "shape", &["lakes", "~water"]),
        ("sea-sea", SEA, 18, r#""er": { "type": "water.sea", "inputs": { "height": "shape" }, "params": { "level": 75 } }"#, "shape", &["shore", "~water"]),
        ("thermal-mesas", MESAS, 22, r#""cap": { "type": "mask.height", "inputs": { "height": "shape" }, "params": { "from": 160, "to": 400, "softness": 4 } }, "er": { "type": "erosion.thermal", "inputs": { "height": "shape", "hardness": "cap" }, "params": { "angle": 30, "iterations": 200 } }"#, "shape", &["talus"]),
        ("strata-range", RANGE, 11, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } }, "er": { "type": "erosion.thermal", "inputs": { "height": "st" }, "params": { "angle": 32, "iterations": 300, "strata": 10 } }"#, "st", &["talus"]),
        ("alps-chain", ALPINE, 13, r#""st": { "type": "erosion.stream", "inputs": { "height": "shape" } },
            "th": { "type": "erosion.thermal", "inputs": { "height": "st" }, "params": { "angle": 34, "iterations": 200, "strata": 8 } },
            "pa": { "type": "erosion.particle", "inputs": { "height": "th" } },
            "er": { "type": "water.rivers", "inputs": { "height": "pa" }, "params": { "threshold": 0.45 } }"#, "shape", &["rivers", "~water"]),
    ];
    for (name, shape, seed, chain, before, masks) in cases {
        if !only.is_empty() && !only.split(',').any(|o| o == *name) {
            continue;
        }
        let text = format!(r#"{{ "terrain": {{ "size": 8192, "seed": {seed} }}, "nodes": {{ {shape}, {chain},
            "out": {{ "type": "output.map", "inputs": {{ "height": "er" }} }} }} }}"#);
        render(name, &text, before, "er", masks);
    }
}

#[test]
#[ignore]
fn renders() {
    let only = std::env::var("MASSIF_EROSION_ONLY").unwrap_or_default();
    let node = std::env::var("MASSIF_EROSION_NODE").unwrap_or_else(|_| "erosion.stream".into());
    let shapes: &[(&str, &str, u32)] = &[
        ("alpine", r#""shape": { "type": "mountain.alpine" }"#, 13),
        ("hills", r#""shape": { "type": "noise.fbm", "params": { "scale": 3000, "height": 250, "base": 60, "octaves": 4 } }"#, 3),
        ("range", r#""shape": { "type": "mountain.range", "params": { "spine": [ { "x": 0.1, "z": 0.7 }, { "x": 0.35, "z": 0.45, "height": 1.3 }, { "x": 0.6, "z": 0.42, "width": 1.4 }, { "x": 0.9, "z": 0.2, "height": 0.7 } ], "height": 240, "width": 1900 } }"#, 11),
        ("sea", r#""coast": { "type": "coast.coastline", "params": { "land": 0.6, "scale": 7000, "height": 90 } },
            "alps": { "type": "mountain.alpine", "inputs": { "area": "inner" }, "params": { "base": 0, "height": 200 } },
            "inner": { "type": "adjust.levels", "inputs": { "in": "coast.land" }, "params": { "in_low": 0.5, "in_high": 1, "out_low": 0, "out_high": 1 } },
            "far": { "type": "adjust.blur", "inputs": { "in": "inner" }, "params": { "radius": 900 } },
            "shape": { "type": "combine.blend", "inputs": { "a": "coast", "b": "alps", "mask": "far" }, "params": { "mode": "add" } }"#, 18),
        ("mesas", r#""m": { "type": "plateau.mesa", "params": { "height": 70, "scale": 1200, "base": 110 } },
            "shape": { "type": "valley.canyon", "inputs": { "height": "m" }, "params": { "depth": 40, "width": 180, "scale": 2400 } }"#, 22),
    ];
    for (name, nodes, seed) in shapes {
        if !only.is_empty() && !only.split(',').any(|o| o == *name) {
            continue;
        }
        let text = format!(r#"{{ "terrain": {{ "size": 8192, "seed": {seed} }}, "nodes": {{ {nodes},
            "er": {{ "type": "{node}", "inputs": {{ "height": "shape" }}{} }},
            "out": {{ "type": "output.map", "inputs": {{ "height": "er" }} }} }} }}"#, extra());
        let masks: Vec<&str> = std::env::var("MASSIF_EROSION_MASKS").map(|s| s.split(',').map(|m| &*m.to_string().leak()).collect()).unwrap_or_else(|_| vec!["flow", "wear", "deposition"]);
        render(&format!("{}-{name}", node.replace('.', "-")), &text, "shape", "er", &masks);
    }
}

/// The share of samples farther apart than `tolerance`.
fn off(a: &Grid, b: &Grid, tolerance: f32) -> f32 {
    assert_eq!(a.rect, b.rect);
    a.data.iter().zip(&b.data).filter(|(x, y)| (*x - *y).abs() > tolerance).count() as f32 / a.data.len() as f32
}

fn whole(e: &Engine, c: &Compiled, node: &str, port: Option<&str>) -> Grid {
    let lod = c.sim_lod();
    e.region(c, c.source(node, port).unwrap(), lod, c.space.whole(lod), &never).unwrap()
}

/// Ridged peaks over a tilted plane, 2048 blocks simulated at 256.
fn peaks(more: &str) -> Graph {
    graph(&format!(
        r#"{{ "terrain": {{ "size": 2048, "seed": 5 }}, "nodes": {{
            "peaks": {{ "type": "noise.fbm", "params": {{ "style": "ridged", "scale": 400, "height": 120, "base": 70 }} }},
            "tilt": {{ "type": "pattern.gradient", "params": {{ "height": 40 }} }},
            "land": {{ "type": "combine.blend", "inputs": {{ "a": "peaks", "b": "tilt" }}, "params": {{ "mode": "add" }} }}{more} }} }}"#
    ))
}

#[test]
fn flow_drains_the_whole_terrain_alike_on_the_card() {
    let g = peaks(r#", "flow": { "type": "water.flow", "inputs": { "height": "land" } }"#);
    let c = compiled(&g, 2048, 256);
    let cpu = whole(&engine(false), &c, "flow", None);
    let (lo, hi) = cpu.range();
    assert!(lo >= 0.0 && hi <= 1.0 && hi > 0.8, "{lo} {hi}");
    let gpu_engine = engine(true);
    if gpu_engine.gpu_name().is_none() {
        return;
    }
    let gpu = whole(&gpu_engine, &c, "flow", None);
    // The card's relief differs from the CPU's by 1e-4 or so, which turns
    // the water at a few flat places: the masks agree but there.
    assert!(off(&cpu, &gpu, 0.01) < 0.002, "{}", off(&cpu, &gpu, 0.01));
    assert_eq!(gpu_engine.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed), 1);
}

fn mean(g: &Grid) -> f32 {
    g.data.iter().map(|&v| v as f64).sum::<f64>() as f32 / g.data.len() as f32
}

#[test]
fn stream_erosion_rebuilds_or_cuts_and_matches_on_the_card() {
    let g = peaks(
        r#", "built": { "type": "erosion.stream", "inputs": { "height": "land" }, "params": { "iterations": 60 } },
            "cut": { "type": "erosion.stream", "inputs": { "height": "land" }, "params": { "iterations": 60, "rebuild": 0, "strength": 0.2 } }"#,
    );
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let built = whole(&cpu, &c, "built", None);
    let cut = whole(&cpu, &c, "cut", None);
    // Rebuilt, as much land stands as before; cut, less.
    assert!((mean(&built) - mean(&land)).abs() < 0.03 * mean(&land), "{} {}", mean(&built), mean(&land));
    assert!(mean(&cut) < mean(&land) - 1.0, "{} {}", mean(&cut), mean(&land));
    for port in ["flow", "wear", "deposition", "sediment"] {
        let (lo, hi) = whole(&cpu, &c, "built", Some(port)).range();
        assert!(lo >= 0.0 && hi <= 1.0 && hi > 0.3, "{port}: {lo} {hi}");
    }
    let gpu = engine(true);
    if gpu.gpu_name().is_none() {
        return;
    }
    for node in ["built", "cut"] {
        let a = whole(&cpu, &c, node, None);
        let b = whole(&gpu, &c, node, None);
        // Erosion amplifies the card's small differences in the relief it
        // starts from where water hesitates between two ways: most of the
        // terrain agrees within a block.
        let share = off(&a, &b, 1.0);
        assert!(share < 0.05, "{node}: {share} of the samples differ by more than a block");
        assert!((mean(&a) - mean(&b)).abs() < 0.5, "{node}: {} {}", mean(&a), mean(&b));
    }
    assert_eq!(gpu.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed), 2);
}

/// Evaluates `node` (port `port`) of the peaks graph extended by `more` on
/// the CPU and, when there is one, on the graphics card; the card's
/// engine is given back to check its counters.
fn both(more: &str, node: &str, port: Option<&str>) -> (Grid, Option<Grid>, Engine) {
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = whole(&engine(false), &c, node, port);
    let gpu_engine = engine(true);
    let gpu = gpu_engine.gpu_name().is_some().then(|| whole(&gpu_engine, &c, node, port));
    (cpu, gpu, gpu_engine)
}

fn ran_on_card(e: &Engine) -> bool {
    e.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed) > 0
}

#[test]
fn particles_wear_gullies_and_match_on_the_card() {
    let more = r#", "er": { "type": "erosion.particle", "inputs": { "height": "land" }, "params": { "rain": 0.5 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let er = whole(&cpu, &c, "er", None);
    // Soil moves; what drops into the sea and off the edges goes.
    assert!((mean(&er) - mean(&land)).abs() < 1.0, "{} {}", mean(&er), mean(&land));
    assert!(er.data.iter().zip(&land.data).any(|(a, b)| (a - b).abs() > 0.5), "the drops wore nothing");
    for port in ["flow", "wear", "deposition"] {
        let (lo, hi) = whole(&cpu, &c, "er", Some(port)).range();
        assert!(lo >= 0.0 && hi <= 1.0 && hi > 0.5, "{port}: {lo} {hi}");
    }
    let (a, b, e) = both(more, "er", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        let share = off(&a, &b, 0.5);
        assert!(share < 0.05, "{share} of the samples differ by more than half a block");
    }
}

#[test]
fn the_sea_comes_in_from_the_edge_only() {
    let more = r#", "sea": { "type": "water.sea", "inputs": { "height": "land" }, "params": { "level": 120 } },
        "open": { "type": "water.sea", "inputs": { "height": "land" }, "params": { "level": 120, "connected": false } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let (sea, open) = (whole(&cpu, &c, "sea", Some("water")), whole(&cpu, &c, "open", Some("water")));
    let wet = |w: &Grid| w.data.iter().zip(&land.data).filter(|(w, h)| *w > *h).count();
    assert!(wet(&sea) > 0 && wet(&sea) <= wet(&open), "{} {}", wet(&sea), wet(&open));
    for ((s, o), h) in sea.data.iter().zip(&open.data).zip(&land.data) {
        assert!(*s == 120.0 || *s <= h - 63.0);
        assert!((*h < 120.0) == (*o == 120.0));
    }
    let (a, b, e) = both(more, "sea", Some("water"));
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 1e-3) < 0.002, "{}", off(&a, &b, 1e-3));
    }
}

#[test]
fn lakes_fill_hollows_to_where_they_spill() {
    let more = r#", "lakes": { "type": "water.lakes", "inputs": { "height": "land" } },
        "full": { "type": "water.lakes", "inputs": { "height": "land" }, "params": { "amount": 1 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let (lakes, ground, full) = (whole(&cpu, &c, "lakes", Some("water")), whole(&cpu, &c, "lakes", None), whole(&cpu, &c, "full", None));
    assert_eq!(ground.data, land.data, "lakes leave the ground as it is at amount 0");
    let wet = lakes.data.iter().zip(&land.data).filter(|(w, h)| *w > *h).count();
    assert!(wet > 0, "no lake");
    // Filled with ground, the hollows are gone: the ground only rose.
    assert!(full.data.iter().zip(&land.data).all(|(f, h)| f >= h));
    assert!(mean(&full) > mean(&land));
    let (a, b, e) = both(more, "lakes", Some("water"));
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 1e-3) < 0.002, "{}", off(&a, &b, 1e-3));
    }
}

#[test]
fn rivers_dig_their_beds_and_hold_water() {
    let more = r#", "riv": { "type": "water.rivers", "inputs": { "height": "land" }, "params": { "threshold": 0.3, "width": 24, "depth": 6 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let (dug, mask, water) = (whole(&cpu, &c, "riv", None), whole(&cpu, &c, "riv", Some("rivers")), whole(&cpu, &c, "riv", Some("water")));
    assert!(dug.data.iter().zip(&land.data).all(|(d, h)| d <= h), "rivers never raise the ground");
    assert!(mask.data.iter().filter(|&&m| m >= 1.0).count() > 50, "no river");
    let wet: Vec<usize> = (0..water.data.len()).filter(|&k| water.data[k] > dug.data[k]).collect();
    assert!(wet.len() > 50, "no water in the rivers");
    let (a, b, e) = both(more, "riv", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 0.5) < 0.05, "{}", off(&a, &b, 0.5));
    }
}

#[test]
fn sediment_settles_at_the_feet_of_slopes() {
    let more = r#", "sed": { "type": "sediment.deposit", "inputs": { "height": "land" }, "params": { "amount": 10, "iterations": 60 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let sed = whole(&cpu, &c, "sed", None);
    let dep = whole(&cpu, &c, "sed", Some("deposition"));
    assert!((mean(&sed) - mean(&land)).abs() < 1.0, "{} {}", mean(&sed), mean(&land));
    assert!(dep.range().1 > 0.3, "nothing settled: {:?}", dep.range());
    let (a, b, e) = both(more, "sed", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 1.0) < 0.05, "{}", off(&a, &b, 1.0));
    }
}

#[test]
fn glaciers_grow_above_the_snow_line_and_scour() {
    let more = r#", "ice": { "type": "ice.glacier", "inputs": { "height": "land" }, "params": { "ela": 120, "iterations": 300 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    let carved = whole(&cpu, &c, "ice", None);
    let ice = whole(&cpu, &c, "ice", Some("ice"));
    assert!(ice.range().1 > 0.2, "no ice: {:?}", ice.range());
    assert!(mean(&carved) < mean(&land) + 0.5, "{} {}", mean(&carved), mean(&land));
    assert!(carved.data.iter().zip(&land.data).any(|(a, b)| b - a > 1.0), "the ice wore nothing");
    let (a, b, e) = both(more, "ice", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 1.0) < 0.05, "{}", off(&a, &b, 1.0));
    }
}

#[test]
fn wind_blows_sand_into_dunes() {
    let more = r#", "flat": { "type": "pattern.constant", "params": { "value": 80 } },
        "dunes": { "type": "wind.erosion", "inputs": { "height": "flat" }, "params": { "iterations": 200, "sand_depth": 6 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let d = whole(&cpu, &c, "dunes", None);
    let (lo, hi) = d.range();
    // Sand gathered into ridges and hollows, not left as a sheet.
    assert!(hi - lo > 3.0, "{lo} {hi}");
    let sand: f32 = d.data.iter().map(|v| v - 80.0).sum::<f32>() / d.data.len() as f32;
    assert!(sand > 2.0 && sand < 7.5, "{sand} blocks of sand left on average");
    let (a, b, e) = both(more, "dunes", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!((mean(&a) - mean(&b)).abs() < 0.2, "{} {}", mean(&a), mean(&b));
    }
}

#[test]
fn hard_rock_does_not_crumble() {
    let more = r#", "one": { "type": "pattern.constant", "params": { "value": 1 } },
        "worn": { "type": "erosion.thermal", "inputs": { "height": "land", "hardness": "one" }, "params": { "angle": 20 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let land = whole(&cpu, &c, "land", None);
    assert_eq!(whole(&cpu, &c, "worn", None).data, land.data);
    let (a, b, e) = both(more, "worn", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 0.01) == 0.0);
    }
}

/// The share of samples steeper than `degrees`.
fn steeper(g: &Grid, spacing: f32, degrees: f32) -> f32 {
    let n = g.rect.w as usize;
    let limit = degrees.to_radians().tan() * spacing;
    let mut count = 0;
    for z in 0..n - 1 {
        for x in 0..n - 1 {
            let k = z * n + x;
            if (g.data[k + 1] - g.data[k]).abs().max((g.data[k + n] - g.data[k]).abs()) > limit {
                count += 1;
            }
        }
    }
    count as f32 / ((n - 1) * (n - 1)) as f32
}

#[test]
fn strata_stand_as_cliffs_between_benches() {
    let more = r#", "plain": { "type": "erosion.thermal", "inputs": { "height": "land" }, "params": { "angle": 30, "iterations": 150 } },
        "beds": { "type": "erosion.thermal", "inputs": { "height": "land" }, "params": { "angle": 30, "iterations": 150, "strata": 12 } }"#;
    let g = peaks(more);
    let c = compiled(&g, 2048, 256);
    let cpu = engine(false);
    let (plain, beds) = (whole(&cpu, &c, "plain", None), whole(&cpu, &c, "beds", None));
    // Hard beds keep cliffs where one rock throughout wears down to its
    // talus; the soft beds between are benches.
    let (sp, sb) = (steeper(&plain, 8.0, 45.0), steeper(&beds, 8.0, 45.0));
    assert!(sb > sp * 1.5 + 0.001, "cliffs: {sb} with strata, {sp} without");
    let (a, b, e) = both(more, "beds", None);
    if let Some(b) = b {
        assert!(ran_on_card(&e));
        assert!(off(&a, &b, 0.5) < 0.02, "{}", off(&a, &b, 0.5));
    }
}
