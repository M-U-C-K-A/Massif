//! The relief bench (ignored by default: it needs the real tiles, fetched
//! once with `tools/relief_bench.sh fetch`): each real landform of
//! `bench::CASES` against Massif's nodes at the same size and range of
//! heights, measure by measure, with a board of hillshades side by side.
//!
//! `tools/relief_bench.sh run`, or
//! `MASSIF_BENCH_DIR=<tiles> cargo test --release -p massif-terrain --test bench -- --ignored --nocapture`.
//! The report (`relief-bench.md`) and the board (`relief-bench.png`) go to
//! `MASSIF_BENCH_OUT` (default the tiles' folder).

use massif_terrain::bench::{CASES, Measures, Relief, gaps, hillshade, measure, percentile};
use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

/// The terrain the nodes are worked on: as wide as a case's window.
const SIZE: u32 = 16384;
/// Its level: 32 blocks a sample, 512 samples a side.
const LOD: u8 = 5;
const N: usize = 512;

/// Massif's candidates, each a graph whose Map Output's height is
/// measured, brought to the real range of heights.
fn candidates() -> Vec<(&'static str, String)> {
    let one = |body: &str| format!(r#"{{ "nodes": {{ {body}, "out": {{ "type": "output.map", "inputs": {{ "height": "h" }} }} }} }}"#);
    vec![
        ("Mountain", one(r#""h": { "type": "mountain.single", "params": { "scale": 15000, "height": 1000, "style": "alpine", "bulk": "high" } }"#)),
        (
            "3 Mountains",
            one(r#""a": { "type": "mountain.single", "params": { "scale": 9000, "height": 1000, "style": "alpine", "bulk": "high", "seed": 1 } },
            "b": { "type": "mountain.single", "params": { "scale": 7000, "height": 800, "style": "eroded", "seed": 2 } },
            "c": { "type": "mountain.single", "params": { "scale": 8000, "height": 700, "style": "basic", "seed": 3 } },
            "tb": { "type": "adjust.transform@2", "params": { "offset_x": 0.28, "offset_z": 0.2, "rotate": 40 }, "inputs": { "in": "b.height" } },
            "tc": { "type": "adjust.transform@2", "params": { "offset_x": -0.25, "offset_z": 0.25, "rotate": -30, "deform": 300 }, "inputs": { "in": "c.height" } },
            "h": { "type": "combine.layers", "inputs": { "in.1": "a.height", "in.2": "tb", "in.3": "tc" } }"#),
        ),
        ("Alpine mountains", one(r#""h": { "type": "mountain.alpine", "params": { "height": 1000, "peak_spacing": 3000, "coverage": 1, "detail": 600 } }"#)),
        ("Rounded mountains", one(r#""h": { "type": "mountain.rounded", "params": { "height": 1000, "peak_spacing": 4000, "coverage": 1, "detail": 800 } }"#)),
        ("Eroded noise", one(r#""h": { "type": "noise.eroded", "params": { "height": 1000, "scale": 6000 } }"#)),
        ("Alpine + erosion", one(r#""m": { "type": "mountain.alpine", "params": { "height": 1000, "peak_spacing": 3000, "coverage": 1, "detail": 600 } }, "h": { "type": "erosion.stream", "inputs": { "height": "m.height" } }"#)),
        ("Canyon", one(r#""p": { "type": "pattern.constant", "params": { "value": 1000 } }, "h": { "type": "valley.canyon", "params": { "depth": 700, "width": 1800, "scale": 6000 }, "inputs": { "height": "p" } }"#)),
        ("Volcano", one(r#""h": { "type": "mountain.volcano", "params": { "volcanoes": [{ "x": 0.5, "z": 0.5, "height": 1000, "radius": 7000 }] } }"#)),
    ]
}

fn ours(json: &str) -> (Relief, f64) {
    let g = Graph::from_json(json).unwrap();
    let (c, _) = compile(&g, Space::new(SIZE), 512);
    let e = Engine::new(Settings::default());
    let t = Instant::now();
    let grid = e.whole(&c, c.output("height").unwrap(), LOD, &never).unwrap();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let n = grid.rect.w as usize;
    // The first N × N samples (the level may hold one more).
    let data = (0..N * N).map(|k| grid.data[(k / N) * n + k % N]).collect();
    (Relief { w: N, h: N, spacing: (1u32 << LOD) as f64, data }, ms)
}

fn row(name: &str, m: &Measures) -> String {
    format!(
        "| {name} | {:.0} | {:.1} | {:.1} | {} | {:.2} | {:.2} | {:.2} | {:.1} | {:.1} |",
        m.relief,
        m.slope_mean,
        m.slope_p90,
        m.slopes.iter().map(|s| format!("{:.0}", s * 100.0)).collect::<Vec<_>>().join("/"),
        m.hypsometric_integral,
        m.drainage_density,
        m.ridge_density,
        m.valley_curvature,
        m.ridge_curvature
    )
}

#[test]
#[ignore]
fn relief_bench() {
    let Some(dir) = std::env::var_os("MASSIF_BENCH_DIR").map(PathBuf::from) else {
        eprintln!("MASSIF_BENCH_DIR is not set: run tools/relief_bench.sh fetch, then tools/relief_bench.sh run");
        return;
    };
    let out = std::env::var_os("MASSIF_BENCH_OUT").map_or(dir.clone(), PathBuf::from);
    let cands = candidates();
    // Each candidate made once (they do not depend on the case), fitted
    // to each case's range.
    let made: Vec<(&str, Relief, f64)> = cands.iter().map(|(name, json)| {
        let (r, ms) = ours(json);
        (*name, r, ms)
    }).collect();
    let mut report = String::from("# Relief bench\n\nEach real landform (Copernicus GLO-30, a window 16.4 km wide resampled to 512² cells of 32 m) against Massif's nodes on a terrain as wide, their heights stretched to the same 2nd–98th percentile range. Gaps: 0 is the same; 1 is far (slopes: half the L1 distance of the slope bins; hypsometry: 4 × the mean gap of the curve; drainage, ridges, valley shape: |log₂| of the ratio).\n\n");
    let _ = writeln!(report, "Columns of measures: relief (m), mean and 90th percentile slope (°), % of the land at <5/5–15/15–25/25–35/35–45/>45°, hypsometric integral, drainage and ridge density (km/km²), curvature of valley floors and crests (1/km).\n");
    let tile = 256;
    let cols = 1 + made.len();
    let mut board = image::GrayImage::new((cols * tile) as u32, (CASES.len() * tile) as u32);
    let mut summary = Vec::new();
    for (ci, case) in CASES.iter().enumerate() {
        let path = dir.join(case.file());
        let t = match massif_core::real::read_tiff(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: {e} (run tools/relief_bench.sh fetch)", path.display());
                continue;
            }
        };
        let real = case.cut(&t.map.data, t.map.width, t.map.height, N);
        let rm = measure(&real);
        let (lo, hi) = (percentile(&real.data, 0.02), percentile(&real.data, 0.98));
        let _ = writeln!(report, "## {} — {}\n\n| | relief | slope | p90 | slope bins % | HI | drainage | ridges | valleys | crests |\n|---|---|---|---|---|---|---|---|---|---|", case.name, case.what);
        let _ = writeln!(report, "{}", row("**real**", &rm));
        let mut table = String::from("\n| Node | slopes | hypsometry | drainage | ridges | valley shape | **overall** |\n|---|---|---|---|---|---|---|\n");
        let mut best = (f32::MAX, "");
        let paste = |board: &mut image::GrayImage, r: &Relief, col: usize| {
            let shade = hillshade(r, 1.0);
            for y in 0..tile {
                for x in 0..tile {
                    let v = shade[(y * N / tile) * N + x * N / tile];
                    board.put_pixel((col * tile + x) as u32, (ci * tile + y) as u32, image::Luma([v]));
                }
            }
        };
        paste(&mut board, &real, 0);
        for (k, (name, r, _)) in made.iter().enumerate() {
            let f = r.fitted(lo, hi);
            let m = measure(&f);
            let g = gaps(&rm, &m);
            let _ = writeln!(report, "{}", row(name, &m));
            let _ = writeln!(table, "| {name} | {} |", g.iter().map(|(_, v)| format!("{v:.2}")).collect::<Vec<_>>().join(" | "));
            if g[5].1 < best.0 {
                best = (g[5].1, name);
            }
            paste(&mut board, &f, k + 1);
        }
        report.push_str(&table);
        let _ = writeln!(report, "\nClosest: {} ({:.2}).\n", best.1, best.0);
        summary.push(format!("{}: closest {} ({:.2})", case.name, best.1, best.0));
    }
    let _ = writeln!(report, "## Board\n\n`relief-bench.png`: a row per landform, the real one first, then the nodes in the order above, lit alike from the north-west.\n\n## Time to make each node (512² at 32 blocks a sample)\n");
    for (name, _, ms) in &made {
        let _ = writeln!(report, "- {name}: {ms:.0} ms");
    }
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("relief-bench.md"), &report).unwrap();
    board.save(out.join("relief-bench.png")).unwrap();
    println!("{report}");
    println!("{}", summary.join("\n"));
}
