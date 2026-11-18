//! How long the brushes take on a large map. Run by hand:
//! `cargo test -p massif-core --release --test perf -- --ignored --nocapture`.

use massif_core::auto::{AutoRules, Classes, Reader};
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::ops::{self, Surface};
use massif_core::registry::TerrainId;
use massif_core::{Brush, BrushShape, Profile};
use std::time::Instant;

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    eprintln!("{label:<44} {:>9.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    out
}

#[test]
#[ignore]
fn brushes_on_a_large_map() {
    let mut world = time("generate 32 × 32 tiles", || {
        generate(&GenParams {
            tiles_x: 32,
            tiles_z: 32,
            landform: Landform::Mountains,
            seed: 3,
            ..GenParams::default()
        })
    });
    let rules = AutoRules::default();
    let reg = world.registry.clone();
    let sea = world.meta.sea_level;
    let reader = Reader::new(&rules, &reg, sea);
    let mut history = History::default();
    for radius in [50.0, 200.0, 800.0, 2000.0] {
        let brush = Brush::symmetric(BrushShape::Circle, Profile::Sine, radius);
        history.begin("perf", 0);
        let mut edit = history.edit(&mut world, 0);
        time(&format!("raise r{radius}"), || ops::raise(&mut edit, &brush, 0.0, 0.0, 1.0, Surface::Top));
        time(&format!("smooth r{radius}"), || ops::smooth(&mut edit, &brush, 0.0, 0.0, Surface::Top));
        time(&format!("paint r{radius}"), || ops::paint_terrain(&mut edit, &brush, 0.0, 0.0, TerrainId(1), 7));
        world_stone(&mut edit, radius, reg.terrain_by_name("Stone").unwrap());
        // Across the edge of the patch just painted.
        let gradient = massif_core::gradient::Gradient::default().sampler(&reg, 3).unwrap();
        let mut sides = massif_core::gradient::Sides::new();
        let across = massif_core::gradient::Layout::Across;
        time(&format!("gradient r{radius}"), || ops::paint_gradient(&mut edit, &brush, radius * 0.6, 0.0, 7, &gradient, across, &mut sides));
        time(&format!("gradient again r{radius}"), || ops::paint_gradient(&mut edit, &brush, radius * 0.6 + 4.0, 0.0, 8, &gradient, across, &mut sides));
        time(&format!("survey r{radius}"), || reader.survey(edit.dim(), massif_core::coords::BlockRect::around(0.0, 0.0, radius)));
        let mut biomes = Classes::new(true);
        time(&format!("auto biome r{radius}"), || ops::auto_biome(&mut edit, &brush, 0.0, 0.0, 7, &reader, &mut biomes));
        time(&format!("auto biome again r{radius}"), || ops::auto_biome(&mut edit, &brush, 16.0, 0.0, 9, &reader, &mut biomes));
        let mut terrains = Classes::new(false);
        time(&format!("auto paint r{radius}"), || ops::auto_paint(&mut edit, &brush, 0.0, 0.0, 7, &reader, &mut terrains));
        time(&format!("auto preview r{radius}"), || ops::auto_preview(&world.dims[0], &brush, 0.0, 0.0, &reader, true));
        let step = ((2.0 * radius / 1024.0).ceil() as i32).max(1);
        time(&format!("auto preview cached, step {step}, r{radius}"), || ops::auto_preview_with(&world.dims[0], &brush, 8.0, 3.0, &biomes, step));
        time(&format!("commit r{radius}"), || history.commit());
    }
}

/// The same on a map just opened: its tiles compressed, decoded as read.
#[test]
#[ignore]
fn brushes_on_a_cold_map() {
    let mut world = generate(&GenParams {
        tiles_x: 32,
        tiles_z: 32,
        landform: Landform::Mountains,
        seed: 3,
        ..GenParams::default()
    });
    let rules = AutoRules::default();
    let reg = world.registry.clone();
    let reader = Reader::new(&rules, &reg, world.meta.sea_level);
    for radius in [200.0, 800.0] {
        let brush = Brush::symmetric(BrushShape::Circle, Profile::Sine, radius);
        let cold = |world: &mut massif_core::World| {
            for dim in &mut world.dims {
                dim.levels = dim.levels.iter_mut().map(|l| l.snapshot()).collect();
            }
        };
        let mut history = History::default();
        cold(&mut world);
        time(&format!("cold: survey r{radius}"), || reader.survey(&world.dims[0], massif_core::coords::BlockRect::around(0.0, 0.0, radius)));
        time(&format!("cold: auto preview r{radius}"), || ops::auto_preview(&world.dims[0], &brush, 0.0, 0.0, &reader, true));
        history.begin("perf", 0);
        let mut edit = history.edit(&mut world, 0);
        let mut biomes = Classes::new(true);
        time(&format!("cold: auto biome r{radius}"), || ops::auto_biome(&mut edit, &brush, 0.0, 0.0, 7, &reader, &mut biomes));
        history.commit();
        cold(&mut world);
        history.begin("perf", 0);
        let mut edit = history.edit(&mut world, 0);
        time(&format!("cold: paint r{radius}"), || ops::paint_terrain(&mut edit, &brush, 0.0, 0.0, TerrainId(1), 7));
        history.commit();
        cold(&mut world);
        history.begin("perf", 0);
        let mut edit = history.edit(&mut world, 0);
        time(&format!("cold: smooth r{radius}"), || ops::smooth(&mut edit, &brush, 0.0, 0.0, Surface::Top));
        history.commit();
    }
}

/// Brushes limited by their filters: by height, by slope.
#[test]
#[ignore]
fn filtered_brushes() {
    let mut world = generate(&GenParams {
        tiles_x: 32,
        tiles_z: 32,
        landform: Landform::Mountains,
        seed: 3,
        ..GenParams::default()
    });
    let mut history = History::default();
    for radius in [200.0, 800.0, 2000.0] {
        for (name, filter) in [
            ("height", massif_core::filter::Filter { above: Some(80), ..Default::default() }),
            ("slope", massif_core::filter::Filter { slope_above: true, slope: 30.0, ..Default::default() }),
            ("hollows", massif_core::filter::Filter { hollows: true, depth: 2.0, radius: 16, ..Default::default() }),
        ] {
            let brush = Brush { filter: Some(std::sync::Arc::new(filter)), ..Brush::symmetric(BrushShape::Circle, Profile::Sine, radius) };
            history.begin("perf", 0);
            let mut edit = history.edit(&mut world, 0);
            time(&format!("paint, {name} filter, r{radius}"), || ops::paint_terrain(&mut edit, &brush, 0.0, 0.0, TerrainId(1), 7));
            history.commit();
        }
    }
}

/// Sculpting in 3D: a dab of Build, growing a bump on flat land.
#[test]
#[ignore]
fn sculpting_in_3d() {
    let mut world = generate(&GenParams {
        tiles_x: 8,
        tiles_z: 8,
        landform: Landform::Plains,
        seed: 3,
        ..GenParams::default()
    });
    let mut history = History::default();
    for radius in [8.0, 16.0, 32.0, 48.0, 64.0, 96.0, 128.0] {
        let brush = Brush::symmetric(BrushShape::Circle, Profile::Sine, radius);
        let y = world.dims[0].ground().view(massif_core::TilePos::new(0, 0)).map_or(64.0, |t| t.top.get(0));
        history.begin("perf", 0);
        let mut edit = history.edit(&mut world, 0);
        time(&format!("3D build r{radius}"), || massif_core::voxel::displace_at(&mut edit, &brush, [0.5, y, 0.5], [0.0, 1.0, 0.0], 1.0, massif_core::voxel::SculptMode::Add));
        time(&format!("3D build again r{radius}"), || massif_core::voxel::displace_at(&mut edit, &brush, [0.5, y + 1.0, 0.5], [0.0, 1.0, 0.0], 1.0, massif_core::voxel::SculptMode::Add));
        time(&format!("3D smooth r{radius}"), || massif_core::voxel::sculpt_at(&mut edit, &brush, [0.5, y, 0.5], Some([0.0, 1.0, 0.0]), 0.5, massif_core::voxel::SculptMode::Smooth));
        history.commit();
        eprintln!("  chunks: {}", world.dims[0].voxels.len());
    }
}

/// Smoothing in 3D, over and over, at the largest brush: for a profiler.
#[test]
#[ignore]
fn smoothing_in_3d_again_and_again() {
    let mut world = generate(&GenParams {
        tiles_x: 8,
        tiles_z: 8,
        landform: Landform::Hills,
        seed: 3,
        ..GenParams::default()
    });
    let mut history = History::default();
    let brush = Brush::symmetric(BrushShape::Circle, Profile::Sine, 64.0);
    let y = world.dims[0].ground().view(massif_core::TilePos::new(0, 0)).map_or(64.0, |t| t.top.get(0));
    history.begin("perf", 0);
    let mut edit = history.edit(&mut world, 0);
    // Some matter sculpted first, as when smoothing what was built.
    massif_core::voxel::displace_at(&mut edit, &brush, [0.5, y, 0.5], [0.0, 1.0, 0.0], 4.0, massif_core::voxel::SculptMode::Add);
    let started = std::time::Instant::now();
    let rounds = 40;
    for k in 0..rounds {
        let c = [0.5 + k as f32 * 2.0, y, 0.5];
        massif_core::voxel::sculpt_at(&mut edit, &brush, c, Some([0.0, 1.0, 0.0]), 0.5, massif_core::voxel::SculptMode::Smooth);
    }
    eprintln!("3D smooth r64: {:.1} ms a dab", started.elapsed().as_secs_f32() * 1000.0 / rounds as f32);
    let started = std::time::Instant::now();
    for k in 0..rounds {
        let c = [0.5 + k as f32 * 2.0, y + 2.0, 0.5];
        massif_core::voxel::displace_at(&mut edit, &brush, c, [0.0, 1.0, 0.0], 1.0, massif_core::voxel::SculptMode::Add);
    }
    eprintln!("3D build r64: {:.1} ms a dab", started.elapsed().as_secs_f32() * 1000.0 / rounds as f32);
}

/// The terrain graph's erosion with its default settings, at up to 4096
/// samples a side (the most a simulation runs at), timed; with a checksum
/// of what it made, to tell a faster version from a different one.
#[test]
#[ignore]
fn graph_erosion() {
    use massif_core::graph::{Extent, Graph, NodeKind, NoiseStyle};
    use std::sync::atomic::AtomicBool;
    // MASSIF_SIZES picks the sizes, as "1024,2048".
    let sizes: Vec<usize> = std::env::var("MASSIF_SIZES").ok().map_or(vec![1024, 2048, 4096], |s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect());
    for n in sizes {
        let mut g = Graph::default();
        let hills = g.add(NodeKind::Noise { style: NoiseStyle::Ridged, scale: 900.0, octaves: 6, roughness: 0.5, height: 160.0, base: 40.0, seed: 9 }, [0.0; 2]);
        let eroded = g.add(NodeKind::Erosion { rain: 1.0, erosion: 0.3, deposition: 0.3, capacity: 4.0, evaporation: 0.02, radius: 6.0, seed: 5 }, [0.0; 2]);
        g.connect(hills, 0, eroded, 0);
        let extent = Extent { x0: -(n as f64) / 2.0, z0: -(n as f64) / 2.0, size: n as f64, n };
        let cancel = AtomicBool::new(false);
        time(&format!("noise {n}"), || g.evaluate(&extent, &[hills], false, &cancel).unwrap());
        let e = time(&format!("noise and erosion {n}"), || g.evaluate(&extent, &[eroded], false, &cancel).unwrap());
        let sum = |port: u8| e.field(eroded, port).unwrap().data.iter().fold(0u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3));
        eprintln!("  checksums {:016x} {:016x} {:016x}", sum(0), sum(1), sum(2));
    }
}

/// Each template of the terrain graph built at 2048 samples a side
/// (MASSIF_SIZES for others), a block a sample: its time and its slowest
/// nodes.
#[test]
#[ignore]
fn graph_templates() {
    use massif_core::graph::{Extent, Family, Template};
    use std::sync::atomic::AtomicBool;
    let reg = massif_core::Registry::vanilla();
    let sizes: Vec<usize> = std::env::var("MASSIF_SIZES").ok().map_or(vec![2048], |s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect());
    for n in sizes {
        for t in Template::ALL {
            if matches!(t, Template::PaintMap | Template::MapRivers) {
                continue;
            }
            let side = n as f64;
            let g = t.graph(&reg, 63.0, side as f32);
            let targets: Vec<_> = g.nodes.iter().filter(|n| n.kind.family() == Family::Output).map(|n| n.id).collect();
            let extent = Extent { x0: -side / 2.0, z0: -side / 2.0, size: side, n };
            let started = Instant::now();
            let e = g.evaluate(&extent, &targets, false, &AtomicBool::new(false)).unwrap();
            let total = started.elapsed().as_secs_f32() * 1000.0;
            let mut slow: Vec<(f32, &str)> = e.timings.iter().map(|(id, ms)| (*ms, g.nodes.iter().find(|n| n.id == *id).map_or("?", |n| n.kind.label()))).collect();
            slow.sort_by(|a, b| b.0.total_cmp(&a.0));
            let top: Vec<String> = slow.iter().take(4).map(|(ms, l)| format!("{l} {ms:.0}")).collect();
            eprintln!("{t:?} {n}: {total:.0} ms ({} nodes): {}", g.nodes.len(), top.join(", "));
        }
    }
}

/// The veins of a tile's chunks, as the export draws them: how long a tile
/// takes, how many blocks it gets.
#[test]
#[ignore]
fn ores_of_a_tile() {
    use massif_core::ores::{OreSettings, Placer};
    let placer = Placer::new(&OreSettings::default(), 5).unwrap();
    let tiles = 400;
    let mut blocks = 0;
    let started = Instant::now();
    for k in 0..tiles {
        blocks += placer.chunks(k * 8, 0, 8, 8, (i32::MIN, i32::MAX)).iter().map(Vec::len).sum::<usize>();
    }
    eprintln!("ores: {:.3} ms a tile, {} blocks a tile", started.elapsed().as_secs_f64() * 1000.0 / tiles as f64, blocks / tiles as usize);
    // A tile of land up to y 100, as most of a map is.
    let (mut blocks, started) = (0, Instant::now());
    for k in 0..tiles {
        blocks += placer.chunks(k * 8, 0, 8, 8, (-64, 100)).iter().map(Vec::len).sum::<usize>();
    }
    eprintln!("ores under y 100: {:.3} ms a tile, {} blocks a tile", started.elapsed().as_secs_f64() * 1000.0 / tiles as f64, blocks / tiles as usize);
}

// getrusage's struct rusage: two timevals first, then longs (ru_maxrss
// the first of them).
#[repr(C)]
struct Timeval {
    sec: i64,
    usec: i64,
}
#[repr(C)]
struct Rusage {
    user: Timeval,
    system: Timeval,
    rest: [i64; 14],
}
unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}

fn rusage() -> Option<Rusage> {
    let mut u = Rusage { user: Timeval { sec: 0, usec: 0 }, system: Timeval { sec: 0, usec: 0 }, rest: [0; 14] };
    // SAFETY: getrusage fills the struct it is given.
    (unsafe { getrusage(0, &mut u) } == 0).then_some(u)
}

/// The CPU time this process has used, user and system, in seconds.
fn cpu_seconds() -> f64 {
    let Some(u) = rusage() else { return 0.0 };
    // tv_usec is 32 bits on macOS, in a 64-bit slot.
    let t = |t: &Timeval| t.sec as f64 + (t.usec & 0xffff_ffff) as f64 / 1e6;
    t(&u.user) + t(&u.system)
}

/// One brush, dab after dab for a while (MASSIF_SECONDS, 12 by default),
/// for a profiler and for timings: MASSIF_BRUSH is raise, smooth, paint,
/// gradient (across the edge of a patch of stone), height, slope or hollows, MASSIF_RADIUS its radius. Run with
/// RAYON_NUM_THREADS=1, its CPU time a dab is the work a dab takes, little
/// changed by what else the machine runs. MASSIF_SELECT=1 selects a
/// feathered ellipse over half the brush first.
#[test]
#[ignore]
fn one_brush_again_and_again() {
    let mut world = generate(&GenParams { tiles_x: 32, tiles_z: 32, landform: Landform::Mountains, seed: 3, ..GenParams::default() });
    let which = std::env::var("MASSIF_BRUSH").unwrap_or_else(|_| "paint".into());
    let radius: f32 = std::env::var("MASSIF_RADIUS").ok().and_then(|r| r.parse().ok()).unwrap_or(800.0);
    let filter = match which.as_str() {
        "height" => Some(massif_core::filter::Filter { above: Some(80), ..Default::default() }),
        "slope" => Some(massif_core::filter::Filter { slope_above: true, slope: 30.0, ..Default::default() }),
        "hollows" => Some(massif_core::filter::Filter { hollows: true, depth: 2.0, radius: 16, ..Default::default() }),
        _ => None,
    };
    let brush = Brush { filter: filter.map(std::sync::Arc::new), ..Brush::symmetric(BrushShape::Circle, Profile::Sine, radius) };
    let mut history = History::default();
    if std::env::var_os("MASSIF_SELECT").is_some() {
        use massif_core::selection::{Selection, Shape};
        let r = radius as i32;
        let shape = Selection::shape(&Shape::Ellipse(massif_core::BlockRect::new(0, -r, 2 * r, r))).feather(32);
        history.select(&mut world, 0, "Select", Some(shape));
    }
    let gradient = massif_core::gradient::Gradient::default().sampler(&world.registry, 3).unwrap();
    let stone = world.registry.terrain_by_name("Stone").unwrap();
    history.begin("perf", 0);
    let mut edit = history.edit(&mut world, 0);
    if which == "gradient" {
        world_stone(&mut edit, radius, stone);
    }
    let mut sides = massif_core::gradient::Sides::new();
    let seconds: f32 = std::env::var("MASSIF_SECONDS").ok().and_then(|r| r.parse().ok()).unwrap_or(12.0);
    let (started, cpu) = (Instant::now(), cpu_seconds());
    let mut dabs = 0;
    while started.elapsed().as_secs_f32() < seconds {
        let c = (dabs % 16) as f32 * 4.0;
        match which.as_str() {
            "raise" => ops::raise(&mut edit, &brush, c, 0.0, 0.5, Surface::Top),
            "smooth" => ops::smooth(&mut edit, &brush, c, 0.0, Surface::Top),
            "gradient" => ops::paint_gradient(&mut edit, &brush, c + radius * 0.6, 0.0, dabs as u64, &gradient, massif_core::gradient::Layout::Across, &mut sides),
            _ => ops::paint_terrain(&mut edit, &brush, c, 0.0, TerrainId(1), dabs as u64),
        }
        dabs += 1;
    }
    let cpu = cpu_seconds() - cpu;
    eprintln!("{which} r{radius}: {:.2} ms a dab, {:.2} ms of CPU", started.elapsed().as_secs_f64() * 1000.0 / dabs as f64, cpu * 1000.0 / dabs as f64);
}

/// A patch of stone half the brush wide, for the gradient to blend.
fn world_stone(edit: &mut massif_core::Edit, radius: f32, stone: TerrainId) {
    ops::paint_terrain(edit, &Brush::symmetric(BrushShape::Circle, Profile::Constant, radius * 0.6), 0.0, 0.0, stone, 1);
}

/// Caves over a large generated map (MASSIF_TILES a side, 32 by default),
/// a layer as the game's painted over all of it: the time to work out the
/// caves of every tile (CPU and wall clock), the blocks carved and flooded,
/// the memory the results take, and the time to paint the layer.
#[test]
#[ignore]
fn caves_over_a_large_map() {
    use massif_core::caves::{CaveDef, CaveLayers, tile_caves};
    use rayon::prelude::*;
    let n: i32 = std::env::var("MASSIF_TILES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let mut world = time(&format!("generate {n} × {n} tiles"), || generate(&GenParams { tiles_x: n, tiles_z: n, landform: Landform::Mountains, seed: 3, ..GenParams::default() }));
    let sea = world.meta.sea_level;
    let dim = &mut world.dims[0];
    dim.caves = CaveLayers::new(vec![CaveDef::vanilla("Caves", 3, dim.min_y, dim.max_y, sea)]);
    let mut history = History::new(1);
    history.begin("caves", 0);
    let painted = time("paint the layer over the whole map", || ops::fill_caves(&mut history.edit(&mut world, 0), 1));
    history.commit();
    eprintln!("{painted} columns painted");
    let dim = &world.dims[0];
    let tiles: Vec<_> = dim.ground().positions().collect();
    for pass in 0..2 {
        let cpu = cpu_seconds();
        let started = Instant::now();
        let made: Vec<(usize, usize, usize)> = tiles
            .par_iter()
            .map(|&p| {
                let ground = dim.ground().view(p).unwrap();
                let c = tile_caves(&dim.caves, &ground, p, dim.min_y, dim.max_y, &[]).unwrap();
                let (all, wet) = c.count();
                (all, wet, c.bytes())
            })
            .collect();
        let wall = started.elapsed().as_secs_f64();
        let (all, wet, bytes) = made.iter().fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
        eprintln!(
            "pass {pass}: {:.2} s, CPU {:.1} s ({:.1} ms a tile), {all} blocks carved, {wet} flooded, {:.0} MB of runs ({:.0} KB a tile), cache {:.0} MB",
            wall,
            cpu_seconds() - cpu,
            (cpu_seconds() - cpu) * 1000.0 / tiles.len() as f64,
            bytes as f64 / 1e6,
            bytes as f64 / 1e3 / tiles.len() as f64,
            massif_core::caves::cache_bytes() as f64 / 1e6
        );
    }
}

/// The most memory this process has held, in megabytes.
fn peak_mb() -> f64 {
    // Bytes on macOS, kilobytes on Linux.
    rusage().map_or(0.0, |u| u.rest[0] as f64 / if cfg!(target_os = "macos") { 1048576.0 } else { 1024.0 })
}

/// A new world from a real place MASSIF_REAL_SIZE blocks a side (16384 by
/// default), at a block a metre, from made-up tiles of hills by a sea: the
/// time and the memory each step of the job takes.
#[test]
#[ignore]
fn a_real_place_of_every_size() {
    use massif_core::real::{Area, DemTile, MAX_TILES, Samples, TILE_PX, Tiles, Vertical, from_px, metres_per_px, spacing};
    use std::sync::Arc;
    let size: i32 = std::env::var("MASSIF_REAL_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
    let started = Instant::now();
    let area = Area { lat: 45.0, lon: 6.0, blocks_per_metre: 1.0, width: size, depth: size }.whole_tiles();
    let z = area.zoom(MAX_TILES);
    let keys = area.tiles(z);
    let tiles = time(&format!("{} tiles of zoom {z} made up", keys.len()), || {
        keys.iter()
            .map(|&k| {
                let metres = (0..TILE_PX * TILE_PX)
                    .map(|i| {
                        let (lat, lon) = from_px(k.x as f64 * 256.0 + (i % TILE_PX) as f64, k.y as f64 * 256.0 + (i / TILE_PX) as f64, z);
                        let (e, s) = area.offset(lat, lon);
                        (e / 30.0 + 200.0 * (e / 900.0).sin() * (s / 1300.0).cos() + 80.0 * (s / 211.0).sin()) as f32
                    })
                    .collect();
                (k, Arc::new(DemTile { metres }))
            })
            .collect()
    });
    let peak = peak_mb();
    let tiles = time("tiles cleaned", || Tiles::new(z, tiles));
    let mut samples = time("samples read", || Samples::read(&area, &tiles, spacing(&area, metres_per_px(area.lat, z))));
    drop(tiles);
    time("gaps filled", || samples.fill_gaps());
    let fit = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: true }.fit(samples.span().unwrap(), 1.0);
    let mut world = time("world made", || massif_core::real::new_world("real", &samples, &fit, &area, 62, -64, 320));
    eprintln!("samples {} × {} every {} blocks", samples.nx, samples.nz, samples.spacing);
    drop(samples);
    let rules = AutoRules::default();
    let mut history = History::new(1);
    history.begin("automatic", 0);
    let base = massif_core::auto::base_terrain(&world.dims[0], &world.registry);
    let reader = Reader::new(&rules, &world.registry, 62).with_base(base);
    time("painted", || massif_core::auto::paint_over_the_map(&mut history.edit(&mut world, 0), &reader));
    drop(history);
    eprintln!(
        "{size} blocks: {:.1} s, {:.1} s of CPU, peak {:.0} MB ({:.0} MB before the work), {} tiles of the map",
        started.elapsed().as_secs_f64(),
        cpu_seconds(),
        peak_mb(),
        peak,
        world.dims[0].ground().positions().count()
    );
}
