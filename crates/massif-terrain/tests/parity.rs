//! How the card's simulations compare with the CPU's on the same input (a
//! shape made on the CPU), and whether each repeats itself exactly. Run
//! with `--ignored --nocapture`: thermal erosion and flow agree within
//! 1e-4; stream erosion, chaotic where water hesitates between two ways,
//! differs (a fifth of the samples by more than a block at 1024²) but
//! each backend gives the same result run after run.
use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::gpu::Gpu;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;

#[test]
#[ignore]
fn simulations_on_the_card_and_the_cpu() {
    for (name, er) in [("stream", r#"{ "type": "erosion.stream", "inputs": { "height": "land" }, "params": { "iterations": 120 } }"#), ("thermal", r#"{ "type": "erosion.thermal", "inputs": { "height": "land" } }"#), ("flow", r#"{ "type": "water.flow", "inputs": { "height": "land" } }"#)] {
        let g = Graph::from_json(&format!(r#"{{ "terrain": {{ "size": 4096, "seed": 3 }}, "nodes": {{ "land": {{ "type": "mountain.alpine" }}, "er": {er} }} }}"#)).unwrap();
        let (c, _) = compile(&g, Space::new(4096), 1024);
        let lod = c.sim_lod();
        let cpu = Engine::new(Settings::default());
        let gpu = Engine::with_gpu(Gpu::headless().unwrap(), Settings::default());
        let src = c.source("er", None).unwrap();
        let land = c.source("land", None).unwrap();
        let a = cpu.region(&c, src, lod, c.space.whole(lod), &never).unwrap();
        let b = gpu.region(&c, src, lod, c.space.whole(lod), &never).unwrap();
        let gpu2 = Engine::with_gpu(Gpu::headless().unwrap(), Settings::default());
        let b2 = gpu2.region(&c, src, lod, c.space.whole(lod), &never).unwrap();
        let cpu2 = Engine::new(Settings::default());
        let a2 = cpu2.region(&c, src, lod, c.space.whole(lod), &never).unwrap();
        eprintln!("{name}: gpu twice equal: {}, cpu twice equal: {}", b2.data == b.data, a2.data == a.data);
        let la = cpu.region(&c, land, lod, c.space.whole(lod), &never).unwrap();
        let lb = gpu.region(&c, land, lod, c.space.whole(lod), &never).unwrap();
        let input = la.data.iter().zip(&lb.data).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        let diffs: Vec<f32> = a.data.iter().zip(&b.data).map(|(x, y)| (x - y).abs()).collect();
        let max = diffs.iter().cloned().fold(0.0f32, f32::max);
        let over = |t: f32| diffs.iter().filter(|d| **d > t).count() as f32 / diffs.len() as f32;
        eprintln!("{name}: input max diff {input:e}; output max {max:e}, >1e-3: {:.4}, >0.1: {:.4}, >1: {:.4}; gpu globals {}", over(1e-3), over(0.1), over(1.0), gpu.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed));
    }
}
