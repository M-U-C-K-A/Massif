//! Shows the order in which Live makes a 16k terrain: the view first,
//! nearest its centre, an overview, then the rest by distance from the view.
//!
//! `cargo run --release -p massif-terrain --example live_order -- graph.json out.png`
//!
//! The picture is the terrain at 8 blocks a sample (the view's level), each
//! tile tinted by when it came, amber first to blue last, the view outlined
//! in white; the times are printed.

use massif_terrain::engine::Engine;
use massif_terrain::kernel::{Grid, NoEnv};
use massif_terrain::live::{Live, Mode, Phase, Ready, Settings};
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space, TILE};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("usage: live_order <graph.json> <out.png>");
        std::process::exit(2);
    };
    let mut graph = Graph::from_json(&std::fs::read_to_string(input).expect("the graph reads")).expect("the graph parses");
    // Over 16384 blocks, whatever the graph says.
    graph.terrain.size = Some(16384);
    let engine = Arc::new(match massif_terrain::gpu::Gpu::headless() {
        Some(g) => Engine::with_gpu(g, Default::default()),
        None => Engine::new(Default::default()),
    });
    let settings = Settings { mode: Mode::Whole, whole_res: 2048, rest: Duration::ZERO, outputs: vec!["height".into(), "albedo".into()], ..Default::default() };
    let live = Live::new(engine.clone(), settings);
    // 4096 × 2560 blocks at 8 blocks a pixel (level 3), west of the centre.
    let view = Rect::new(3072, 6144, 4096, 2560);
    live.set_view(view, 8.0);
    let started = Instant::now();
    for d in live.set_graph(&graph, Space::new(16384), Arc::new(NoEnv), 1) {
        eprintln!("{d}");
    }
    let mut came: Vec<(Ready, f64)> = Vec::new();
    loop {
        // Idle before taking the tiles: none made after is left behind.
        let idle = live.progress().idle;
        let now = started.elapsed().as_secs_f64() * 1000.0;
        came.extend(live.tiles_ready().into_iter().map(|t| (t, now)));
        if idle {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let p = live.progress();
    if let Some(e) = &p.error {
        eprintln!("{e}");
        std::process::exit(1);
    }
    println!("Live on {}: first tile in view {:.1} ms, view complete {:.1} ms, all {:.1} ms", engine.gpu_name().unwrap_or("the CPU"), ms(p.first_visible), ms(p.visible), ms(p.finished));
    for (phase, exact) in [(Phase::Visible, false), (Phase::Overview, false), (Phase::Visible, true), (Phase::Overview, true), (Phase::Rest, true)] {
        let of: Vec<f64> = came.iter().filter(|(t, _)| t.phase == phase && t.exact == exact && t.output == 0).map(|c| c.1).collect();
        if let (Some(first), Some(last)) = (of.first(), of.last()) {
            println!("  {phase:?}{}: {} tiles from {first:.0} to {last:.0} ms", if exact { "" } else { " (quick simulations)" }, of.len());
        }
    }

    // The terrain at level 3, tiles as they last came, by when they came.
    let lod = 3;
    let n = Space::new(16384).samples(lod);
    let whole = Rect::new(0, 0, n, n);
    let mut heights = Grid::new(lod, whole, vec![0.0; whole.area()]);
    let mut colours = Grid::new(lod, whole, vec![0.0; whole.area() * 3]);
    let mut rank = vec![f32::NAN; whole.area()];
    let finals: Vec<&(Ready, f64)> = came.iter().filter(|(t, _)| t.key.lod == lod && t.exact).collect();
    let count = finals.iter().filter(|(t, _)| t.output == 0).count().max(1);
    let mut k = 0;
    for (t, _) in &finals {
        let r = Rect::tile(t.key.x, t.key.z).intersect(&whole);
        let v = t.tile.values().expect("on the CPU");
        let area = (TILE * TILE) as usize;
        for z in r.z..r.z1() {
            for x in r.x..r.x1() {
                let (i, o) = (((z - t.key.z * TILE as i32) * TILE as i32 + (x - t.key.x * TILE as i32)) as usize, (z as u32 * n + x as u32) as usize);
                if t.output == 0 {
                    heights.data[o] = v[i];
                    rank[o] = k as f32 / count as f32;
                } else {
                    for c in 0..3 {
                        colours.data[c * whole.area() + o] = v[c * area + i];
                    }
                }
            }
        }
        if t.output == 0 {
            k += 1;
        }
    }
    let has_colours = finals.iter().any(|(t, _)| t.output == 1);
    let mut rgb = massif_terrain::preview::shade_with(&heights, 62.0, has_colours.then_some(&colours));
    let ramp = |r: f32| -> [f32; 3] {
        let stops = [[236.0, 170.0, 40.0], [60.0, 190.0, 170.0], [40.0, 70.0, 200.0]];
        let s = r.clamp(0.0, 1.0) * 2.0;
        let (a, b, f) = if s < 1.0 { (stops[0], stops[1], s) } else { (stops[1], stops[2], s - 1.0) };
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
    };
    let view_at = view.at_lod(0, lod);
    for (o, px) in rgb.iter_mut().enumerate() {
        let (x, z) = ((o as u32 % n) as i32, (o as u32 / n) as i32);
        let tint = ramp(rank[o]);
        for c in 0..3 {
            px[c] = (px[c] as f32 * 0.55 + tint[c] * 0.45) as u8;
        }
        let edge = x % TILE as i32 == 0 || z % TILE as i32 == 0;
        let outline = view_at.contains(x, z) && (x - view_at.x < 4 || view_at.x1() - x <= 4 || z - view_at.z < 4 || view_at.z1() - z <= 4);
        if outline {
            *px = [255, 255, 255];
        } else if edge {
            *px = [px[0] / 2, px[1] / 2, px[2] / 2];
        }
    }
    // Half size: 1024 pixels a side.
    let half = n / 2;
    let small: Vec<u8> = (0..half * half).flat_map(|o| rgb[((o / half) * 2 * n + (o % half) * 2) as usize]).collect();
    let file = std::fs::File::create(output).expect("the picture can be written");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), half, half);
    enc.set_color(png::ColorType::Rgb);
    enc.write_header().and_then(|mut w| w.write_image_data(&small)).expect("the picture is written");
}

fn ms(d: Option<Duration>) -> f64 {
    d.map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0)
}
