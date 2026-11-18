//! Draws each template of Massif 1.2 as 1.2 builds it (left) and migrated
//! to a terrain graph (right), and prints what the migration said.
//!
//! `cargo run --release -p massif-terrain --example migrate_templates -- out.png [side]`

use massif_core::graph::{Extent, Template};
use massif_core::registry::{Registry, TerrainId};
use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, never};
use massif_terrain::kernel::Grid;
use massif_terrain::migrate::{World, from_v1};
use massif_terrain::space::{Rect, Space};
use std::sync::atomic::AtomicBool;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().cloned().unwrap_or_else(|| "templates.png".into());
    let px: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(256);
    let side = 2048u32;
    let reg = Registry::vanilla();
    let names = |id: u16| reg.terrain(TerrainId(id)).name.clone();
    let engine = Engine::new(Default::default());
    let gpu = massif_terrain::gpu::Gpu::headless();
    let engine = match gpu {
        Some(g) => Engine::with_gpu(g, Default::default()),
        None => engine,
    };
    let rows = Template::ALL.len();
    let (w, h) = (2 * px + 8, rows * (px + 8));
    let mut img = vec![[24u8, 24, 28]; w * h];
    for (r, t) in Template::ALL.iter().enumerate() {
        let old = t.graph(&reg, 63.0, side as f32);
        // 1.2's build.
        let extent = Extent { x0: 0.0, z0: 0.0, size: side as f64, n: px };
        let started = std::time::Instant::now();
        let built = old.build(&extent, &AtomicBool::new(false));
        let v1_ms = started.elapsed().as_secs_f64() * 1000.0;
        let left: Option<Vec<[u8; 3]>> = built.ok().and_then(|o| {
            let heights = o.heights?;
            let (n, mean) = (heights.data.len() as f64, heights.data.iter().map(|v| *v as f64).sum::<f64>() / heights.data.len() as f64);
            let sd = (heights.data.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
            println!("  1.2 heights {:?} mean {mean:.1} sd {sd:.1}", heights.range());
            let grid = Grid::new(0, Rect::new(0, 0, px as u32, px as u32), heights.data.clone());
            let colours = o.shown.map(|c| Grid::new(0, Rect::new(0, 0, px as u32, px as u32), (0..3).flat_map(|k| (0..px * px).map(move |i| (k, i))).map(|(k, i)| c.linear(i)[k]).collect()));
            Some(massif_terrain::preview::shade_with(&grid, 63.0, colours.as_ref()))
        });
        // Migrated.
        let (g, diags) = from_v1(&old, &World { side, sea_level: 63.0, terrain_name: &names });
        if std::env::var("MASSIF_DUMP").is_ok_and(|d| t.label().starts_with(&d)) {
            for n in &old.nodes {
                println!("v1 #{} {:?}", n.id, n.kind);
            }
            for l in &old.links {
                println!("v1 link {}.{} -> {}.{}", l.from, l.output, l.to, l.input);
            }
            println!("{}", massif_terrain::format::compact_json(&g));
        }
        let (c, errors) = compile(&g, Space::new(side), 1024);
        let lod = c.space.lod_for(px as u32);
        let whole = c.space.whole(lod);
        let started = std::time::Instant::now();
        let right = c.output("height").and_then(|src| engine.region(&c, src, lod, whole, &never).ok()).map(|grid| {
            let (n, mean) = (grid.data.len() as f64, grid.data.iter().map(|v| *v as f64).sum::<f64>() / grid.data.len() as f64);
            let sd = (grid.data.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
            println!("  1.3 heights {:?} mean {mean:.1} sd {sd:.1}", grid.range());
            let albedo = c.output("albedo").and_then(|a| engine.region(&c, a, lod, whole, &never).ok());
            massif_massif(&grid, albedo.as_ref())
        });
        let v2_ms = started.elapsed().as_secs_f64() * 1000.0;
        println!("{:<22} 1.2 {:>6.0} ms, migrated {:>6.0} ms, {} nodes, {} errors", t.label(), v1_ms, v2_ms, g.nodes.len(), errors.len());
        for d in diags.iter().chain(&errors) {
            println!("    {d}");
        }
        for (col, picture) in [(0, left), (1, right)] {
            let Some(p) = picture else { continue };
            let n = (p.len() as f64).sqrt() as usize;
            for y in 0..px {
                for x in 0..px {
                    let s = p[(y * n / px) * n + x * n / px];
                    img[(r * (px + 8) + y) * w + col * (px + 8) + x] = s;
                }
            }
        }
    }
    let flat: Vec<u8> = img.into_iter().flatten().collect();
    image::RgbImage::from_raw(w as u32, h as u32, flat).expect("pixels").save(&out).expect("the picture is written");
}

fn massif_massif(grid: &Grid, albedo: Option<&Grid>) -> Vec<[u8; 3]> {
    massif_terrain::preview::shade_with(grid, 63.0, albedo)
}
