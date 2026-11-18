//! Timings (ignored by default):
//! `cargo test --release -p massif-terrain --test perf -- --ignored --nocapture`.

use massif_terrain::compile::{Compiled, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::model::{Graph, Node};
use massif_terrain::space::Space;
use std::time::Instant;

/// Hills with ridges added: two noises of 8 octaves, levels, a blend.
fn graph() -> Graph {
    let mut g = Graph::default();
    g.nodes.insert("hills".into(), Node::new("noise.fbm").with("scale", 2000).with("height", 300));
    g.nodes.insert("ridges".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 800).with("height", 200).with("base", 0));
    g.nodes.insert("relief".into(), Node::new("adjust.levels").input("in", "hills").with("in_high", 340).with("out_high", 280).with("gamma", 1.4));
    g.nodes.insert("mixed".into(), Node::new("combine.blend").input("a", "relief").input("b", "ridges"));
    g.nodes.insert("out".into(), Node::new("output.map").input("height", "mixed"));
    g
}

/// Builds the whole terrain at the finest level, 8 × 8 tiles at a time as
/// a conversion would, and gives the seconds and a checksum.
fn build(e: &Engine, c: &Compiled) -> (f64, f64) {
    let out = c.output("height").unwrap();
    let n = c.space.tiles(0) as i32;
    let started = Instant::now();
    let mut sum = 0.0f64;
    for bz in (0..n).step_by(8) {
        for bx in (0..n).step_by(8) {
            let keys: Vec<(i32, i32)> = (bz..(bz + 8).min(n)).flat_map(|z| (bx..(bx + 8).min(n)).map(move |x| (x, z))).collect();
            for t in e.tiles(c, out, 0, &keys, &never).unwrap() {
                sum += t.values().unwrap()[0] as f64;
            }
        }
    }
    (started.elapsed().as_secs_f64(), sum)
}

#[test]
#[ignore]
fn build_16k() {
    let size = std::env::var("MASSIF_TERRAIN_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
    let (c, diags) = compile(&graph(), Space::new(size), 2048);
    assert!(diags.is_empty());
    let samples = (size as f64).powi(2);
    let mut best_cpu = f64::MAX;
    let mut sum_cpu = 0.0;
    for _ in 0..2 {
        let e = Engine::new(Settings { cache_mb: 1024 });
        let (s, sum) = build(&e, &c);
        best_cpu = best_cpu.min(s);
        sum_cpu = sum;
    }
    println!("{size}² on the CPU ({} threads): {best_cpu:.2} s, {:.0} M samples/s", rayon::current_num_threads(), samples / best_cpu / 1e6);
    #[cfg(feature = "gpu")]
    if let Some(gpu) = massif_terrain::gpu::Gpu::headless() {
        let mut best = f64::MAX;
        let mut sum_gpu = 0.0;
        for _ in 0..3 {
            let e = Engine::with_gpu(gpu.clone(), Settings { cache_mb: 1024 });
            let (s, sum) = build(&e, &c);
            best = best.min(s);
            sum_gpu = sum;
        }
        println!("{size}² on {}: {best:.2} s, {:.0} M samples/s ({:.1}× the CPU)", gpu.name(), samples / best / 1e6, best_cpu / best);
        assert!((sum_cpu - sum_gpu).abs() / sum_cpu.abs().max(1.0) < 1e-4, "{sum_cpu} {sum_gpu}");
    }
}

#[test]
#[ignore]
fn thermal_whole() {
    for sim in [1024u32, 2048, 4096] {
        let mut g = Graph::default();
        g.nodes.insert("peaks".into(), Node::new("noise.fbm").with("style", "ridged").with("scale", 1500).with("height", 600));
        g.nodes.insert("worn".into(), Node::new("erosion.thermal").input("height", "peaks").with("iterations", 50));
        let (c, _) = compile(&g, Space::new(16384), sim);
        let e = Engine::new(Settings::default());
        let src = c.source("worn", None).unwrap();
        let lod = c.sim_lod();
        let started = Instant::now();
        e.region(&c, src, lod, c.space.whole(lod), &never).unwrap();
        let total = started.elapsed().as_secs_f64();
        let ms = e.counters.by_node.lock().unwrap().get("worn").map_or(0.0, |v| v.1);
        println!("thermal, 50 iterations at {sim}²: {:.2} s (the noise and the rest {:.2} s)", ms / 1000.0, total - ms / 1000.0);
    }
}
