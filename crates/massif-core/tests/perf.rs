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
