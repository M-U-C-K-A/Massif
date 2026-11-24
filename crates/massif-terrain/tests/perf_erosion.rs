//! Timings of the erosion nodes (ignored by default), one configuration a
//! run so that its peak memory can be measured from outside:
//!
//! `MASSIF_BENCH_NODE=erosion.stream MASSIF_BENCH_SIM=2048 [MASSIF_BENCH_CPU=1]
//! /usr/bin/time -l cargo test --release -p massif-terrain --test perf_erosion -- --ignored --nocapture`
//!
//! The terrain is 4 blocks a sample (a 2048 simulation is an 8192 map's):
//! ridged peaks over a tilted plane. The input is made first, then the
//! node alone is timed, its outputs read back to the CPU as the engine
//! gives them; the best of `MASSIF_BENCH_RUNS` runs (2 by default), each
//! with an engine of its own so that nothing is cached.

use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::gpu::Gpu;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::time::Instant;

/// The node's settings for the benchmark: as a user would leave them.
fn settings(node: &str) -> &'static str {
    match node {
        "ice.glacier" => r#"{ "ela": 140 }"#,
        "water.sea" => r#"{ "level": 110 }"#,
        _ => "{}",
    }
}

#[test]
#[ignore]
fn erosion_timings() {
    let node = std::env::var("MASSIF_BENCH_NODE").unwrap_or_else(|_| "erosion.stream".into());
    let sim: u32 = std::env::var("MASSIF_BENCH_SIM").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let cpu = std::env::var("MASSIF_BENCH_CPU").is_ok();
    let runs: usize = std::env::var("MASSIF_BENCH_RUNS").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let size = sim * 4;
    let text = format!(
        r#"{{ "terrain": {{ "size": {size}, "seed": 5 }}, "nodes": {{
            "peaks": {{ "type": "noise.fbm", "params": {{ "style": "ridged", "scale": {s1}, "height": 160, "base": 70 }} }},
            "tilt": {{ "type": "pattern.gradient", "params": {{ "height": 40 }} }},
            "land": {{ "type": "combine.blend", "inputs": {{ "a": "peaks", "b": "tilt" }}, "params": {{ "mode": "add" }} }},
            "er": {{ "type": "{node}", "inputs": {{ "height": "land" }}, "params": {p} }} }} }}"#,
        s1 = size / 5,
        p = settings(&node)
    );
    let g = Graph::from_json(&text).unwrap_or_else(|e| panic!("{e}"));
    let (c, diags) = compile(&g, Space::new(size), sim);
    assert!(diags.is_empty(), "{diags:?}");
    let lod = c.sim_lod();
    let whole = c.space.whole(lod);
    let mut best = f64::MAX;
    let mut on = String::new();
    for _ in 0..runs {
        let e = match (!cpu).then(Gpu::headless).flatten() {
            Some(g) => Engine::with_gpu(g, Settings { cache_mb: 8192 }),
            None => Engine::new(Settings { cache_mb: 8192 }),
        };
        e.region(&c, c.source("land", None).unwrap(), lod, whole, &never).unwrap();
        let started = Instant::now();
        for port in c.nodes[c.node("er").unwrap()].spec.outputs.iter().map(|p| p.name) {
            e.region(&c, c.source("er", Some(port)).unwrap(), lod, whole, &never).unwrap();
        }
        best = best.min(started.elapsed().as_secs_f64());
        let gpu = e.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed) > 0;
        on = if gpu { e.gpu_name().unwrap_or("?").to_string() } else { "the CPU".into() };
    }
    println!("BENCH {node} {sim}² on {on}: {:.0} ms", best * 1000.0);
}
