//! Renders a terrain graph's height output to a shaded PNG.
//!
//! `cargo run --release -p massif-terrain --example render -- graph.json out.png [pixels] [--cpu]`
//!
//! The whole terrain is drawn at the finest level of detail with at most
//! `pixels` samples a side (1024 by default).

use massif_terrain::compile::{Severity, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cpu_only = args.iter().any(|a| a == "--cpu");
    let args: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("usage: render <graph.json> <out.png> [pixels] [--cpu]");
        std::process::exit(2);
    };
    let pixels: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1024);
    let text = std::fs::read_to_string(input).expect("the graph reads");
    let graph = Graph::from_json(&text).unwrap_or_else(|e| panic!("{input}: {e}"));
    let (c, diags) = compile(&graph, Space::new(8192), 2048);
    for d in &diags {
        eprintln!("{d}");
    }
    if diags.iter().any(|d| d.severity == Severity::Error) {
        std::process::exit(1);
    }
        let engine = match (cpu_only, massif_terrain::gpu::Gpu::headless()) {
        (false, Some(gpu)) => Engine::with_gpu(gpu, Settings::default()),
        _ => Engine::new(Settings::default()),
    };
        let _ = cpu_only;
        Engine::new(Settings::default())
    };
    let out = c.output("height").expect("the graph’s Map Output has a Height");
    let lod = c.space.lod_for(pixels);
    let started = Instant::now();
    let grid = engine.region(&c, out, lod, c.space.whole(lod), &never).expect("the graph evaluates");
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let (lo, hi) = grid.range();
    eprintln!("{}² at level {lod} on {} in {ms:.0} ms; heights {lo:.0} to {hi:.0}", grid.rect.w, engine.gpu_name().unwrap_or("the CPU"));
    let mut nodes: Vec<(String, (u64, f64))> = engine.counters.by_node.lock().unwrap().iter().map(|(k, v)| (k.clone(), *v)).collect();
    nodes.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
    for (id, (tiles, ms)) in nodes {
        eprintln!("  {id}: {tiles} tiles, {ms:.0} ms");
    }
    let rgb = massif_terrain::preview::shade(&grid, c.space.sea_level);
    let file = std::fs::File::create(output).expect("the picture can be written");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), grid.rect.w, grid.rect.h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(rgb.as_flattened()).expect("png data");
}
