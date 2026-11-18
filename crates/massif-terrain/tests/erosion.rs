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
