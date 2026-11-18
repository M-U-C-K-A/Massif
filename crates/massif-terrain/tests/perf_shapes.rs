//! Timings of the shapes' building blocks (ignored by default):
//! `cargo test --release -p massif-terrain --test perf_shapes -- --ignored --nocapture`.

use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::model::Graph;
use massif_terrain::noise::{Fbm, noise2};
use massif_terrain::space::{Rect, Space};
use std::time::Instant;

fn time(label: &str, n: usize, f: impl Fn(usize) -> f32) {
    let started = Instant::now();
    let mut acc = 0.0f32;
    for k in 0..n {
        acc += f(k);
    }
    let ns = started.elapsed().as_nanos() as f64 / n as f64;
    println!("{label}: {ns:.1} ns ({acc:.1})");
}

#[test]
#[ignore]
fn building_blocks() {
    let n = 1_000_000;
    let at = |k: usize| ((k % 1000) as f64 * 7.3, (k / 1000) as f64 * 7.3);
    time("noise2", n, |k| {
        let (x, z) = at(k);
        noise2(x * 0.01, z * 0.01, 7)
    });
    let f = Fbm::new(500.0, 6, 0.5, 3);
    time("fbm, 6 octaves", n, |k| {
        let (x, z) = at(k);
        f.at(x, z)
    });
    time("mountains, 6 octaves", n, |k| {
        let (x, z) = at(k);
        f.mountains(x, z, 0.7)
    });
    time("eroded, 6 octaves", n, |k| {
        let (x, z) = at(k);
        f.eroded(x, z, 1.5)
    });
    for (name, json) in [("range", include_str!("../examples/graphs/m2-range.json")), ("alpine", include_str!("../examples/graphs/m2-alpine.json"))] {
        let g = Graph::from_json(json).unwrap();
        let (c, _) = compile(&g, Space::new(8192), 2048);
        let src = c.output("height").unwrap();
        for lod in [3u8, 0] {
            let e = Engine::new(Settings::default());
            rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| {
                let started = Instant::now();
                e.region(&c, src, lod, Rect::tile(if lod == 3 { 1 } else { 12 }, if lod == 3 { 1 } else { 12 }), &never).unwrap();
                println!("{name}: one tile at level {lod} on one core: {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
            });
        }
    }
}
