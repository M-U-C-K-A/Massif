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

/// The CPU time this process has used, user and system, in seconds.
fn cpu_seconds() -> f64 {
    // getrusage's struct rusage: two timevals first, then longs.
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
    let mut u = Rusage { user: Timeval { sec: 0, usec: 0 }, system: Timeval { sec: 0, usec: 0 }, rest: [0; 14] };
    // SAFETY: getrusage fills the struct it is given.
    if unsafe { getrusage(0, &mut u) } != 0 {
        return 0.0;
    }
    // tv_usec is 32 bits on macOS, in a 64-bit slot.
    let t = |t: &Timeval| t.sec as f64 + (t.usec & 0xffff_ffff) as f64 / 1e6;
    t(&u.user) + t(&u.system)
}

/// One brush, dab after dab for a while (MASSIF_SECONDS, 12 by default),
/// for a profiler and for timings: MASSIF_BRUSH is raise, smooth, paint,
/// height, slope or hollows, MASSIF_RADIUS its radius. Run with
/// RAYON_NUM_THREADS=1, its CPU time a dab is the work a dab takes, little
/// changed by what else the machine runs.
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
    history.begin("perf", 0);
    let mut edit = history.edit(&mut world, 0);
    let seconds: f32 = std::env::var("MASSIF_SECONDS").ok().and_then(|r| r.parse().ok()).unwrap_or(12.0);
    let (started, cpu) = (Instant::now(), cpu_seconds());
    let mut dabs = 0;
    while started.elapsed().as_secs_f32() < seconds {
        let c = (dabs % 16) as f32 * 4.0;
        match which.as_str() {
            "raise" => ops::raise(&mut edit, &brush, c, 0.0, 0.5, Surface::Top),
            "smooth" => ops::smooth(&mut edit, &brush, c, 0.0, Surface::Top),
            _ => ops::paint_terrain(&mut edit, &brush, c, 0.0, TerrainId(1), dabs as u64),
        }
        dabs += 1;
    }
    let cpu = cpu_seconds() - cpu;
    eprintln!("{which} r{radius}: {:.2} ms a dab, {:.2} ms of CPU", started.elapsed().as_secs_f64() * 1000.0 / dabs as f64, cpu * 1000.0 / dabs as f64);
}
