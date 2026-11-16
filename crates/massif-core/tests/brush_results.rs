//! What the map brushes write, pinned: a fixed series of strokes with every
//! brush and filter, on a generated map, summed up in one checksum. A brush
//! made faster must write exactly the same heights, terrains, biomes and
//! covers; a change meant to alter what a brush does updates the checksum.

use massif_core::filter::{Filter, Match};
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::ops::{self, Surface};
use massif_core::registry::{BiomeId, TerrainId};
use massif_core::{Brush, BrushMask, BrushShape, Profile, World};
use std::sync::Arc;

/// FNV-1a over every tile of the ground, in order of position.
fn checksum(world: &World) -> u64 {
    let level = world.dims[0].ground();
    let mut positions: Vec<_> = level.positions().collect();
    positions.sort_by_key(|p| (p.z, p.x));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    };
    for pos in positions {
        let tile = level.view(pos).unwrap();
        eat(pos.x as u64 ^ (pos.z as u64) << 32);
        for v in tile.top.dense().iter() {
            eat(v.to_bits() as u64);
        }
        for plane in [&tile.terrain, &tile.biome, &tile.cover, &tile.objects] {
            for v in plane.dense().iter() {
                eat(*v as u64);
            }
        }
    }
    h
}

fn stroke(world: &mut World, history: &mut History, from: (f32, f32), to: (f32, f32), dabs: usize, mut dab: impl FnMut(&mut massif_core::Edit, f32, f32, u64)) {
    // Every other stroke on a map as just opened, its tiles compressed and
    // decoded as read: the same strokes write the same either way.
    static STROKES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if STROKES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 2 == 1 {
        for dim in &mut world.dims {
            dim.levels = dim.levels.iter_mut().map(|l| l.snapshot()).collect();
        }
    }
    history.begin("stroke", 0);
    let mut edit = history.edit(world, 0);
    for k in 0..dabs {
        let t = k as f32 / (dabs - 1).max(1) as f32;
        let (x, z) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        dab(&mut edit, x, z, k as u64 * 7 + 1);
    }
    history.commit();
}

#[test]
fn brushes_write_what_they_wrote() {
    let mut world = generate(&GenParams {
        tiles_x: 6,
        tiles_z: 6,
        landform: Landform::Mountains,
        seed: 11,
        ..GenParams::default()
    });
    let mut history = History::default();
    let mut sums = Vec::new();
    let sine = |r| Brush { strength: 0.7, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, r) };
    let filtered = |r, f: Filter| Brush { filter: Some(Arc::new(f)), ..sine(r) };
    // From the map's edge (it spans −384..384) across tile borders.
    let (a, b) = ((-420.0, -150.3), (260.7, 90.2));
    let square = Brush { rotation: 30.0, strength: 0.6, ..Brush::symmetric(BrushShape::Square, Profile::Plateau, 70.0) };
    let image = {
        let (w, h) = (37, 23);
        let data = (0..w * h).map(|i| ((i * 97 + (i / w) * 31) % 256) as u8).collect();
        Brush { mask: Some(Arc::new(BrushMask::new("pinned", w, h, data))), rotation: 75.0, strength: 0.8, ..Brush::symmetric(BrushShape::Circle, Profile::Constant, 90.0) }
    };
    let profiles = Profile::ALL.map(|p| Brush { strength: 0.5, ..Brush::symmetric(BrushShape::Circle, p, 45.0) });

    stroke(&mut world, &mut history, a, b, 12, |e, x, z, _| ops::raise(e, &sine(120.0), x, z, 3.0, Surface::Top));
    sums.push(checksum(&world));
    stroke(&mut world, &mut history, b, a, 9, |e, x, z, _| ops::raise(e, &square, x, z, -2.5, Surface::Top));
    sums.push(checksum(&world));
    for (k, p) in profiles.iter().enumerate() {
        let y = -300.0 + k as f32 * 100.0;
        stroke(&mut world, &mut history, (-350.0, y), (350.0, y + 20.0), 8, |e, x, z, _| ops::raise(e, p, x, z, 1.7, Surface::Top));
    }
    sums.push(checksum(&world));
    stroke(&mut world, &mut history, a, b, 10, |e, x, z, _| ops::smooth(e, &sine(80.0), x, z, Surface::Top));
    stroke(&mut world, &mut history, (0.0, -380.0), (10.0, 380.0), 10, |e, x, z, _| ops::smooth(e, &sine(30.0), x, z, Surface::Top));
    sums.push(checksum(&world));
    stroke(&mut world, &mut history, (-200.0, 200.0), (200.0, -200.0), 8, |e, x, z, _| ops::flatten(e, &sine(60.0), x, z, 70.0, Surface::Top));
    stroke(&mut world, &mut history, (-100.0, 0.0), (100.0, 30.0), 4, |e, x, z, _| ops::set_height(e, &square, x, z, 82.5, Surface::Top));
    stroke(&mut world, &mut history, (150.0, -300.0), (-150.0, 300.0), 8, |e, x, z, s| ops::roughen(e, &sine(70.0), x, z, 4.0, s, Surface::Top));
    sums.push(checksum(&world));
    stroke(&mut world, &mut history, b, a, 10, |e, x, z, _| ops::raise(e, &image, x, z, 2.0, Surface::Top));
    stroke(&mut world, &mut history, a, b, 10, |e, x, z, s| ops::paint_terrain(e, &image, x, z, TerrainId(3), s));
    sums.push(checksum(&world));
    stroke(&mut world, &mut history, a, b, 14, |e, x, z, s| ops::paint_terrain(e, &sine(100.0), x, z, TerrainId(2), s));
    stroke(&mut world, &mut history, (-380.0, 380.0), (380.0, -380.0), 14, |e, x, z, s| ops::paint_biome(e, &square, x, z, BiomeId(5), s));
    stroke(&mut world, &mut history, (0.0, -380.0), (0.0, 380.0), 14, |e, x, z, s| ops::paint_cover(e, &sine(50.0), x, z, 2, s));
    stroke(&mut world, &mut history, (-380.0, 0.0), (380.0, 0.0), 14, |e, x, z, s| ops::paint_objects(e, &sine(50.0), x, z, 1, s));
    stroke(&mut world, &mut history, a, b, 6, |e, x, z, s| ops::paint_terrain_with(e, &sine(90.0), x, z, s, |h| TerrainId(if h > 75.0 { 4 } else { 1 })));
    sums.push(checksum(&world));

    // The filters, alone and together, sharp and faded.
    let filters = [
        Filter { above: Some(72), ..Default::default() },
        Filter { above: Some(60), below: Some(90), fade: true, ..Default::default() },
        Filter { above: Some(65), below: Some(85), beyond: true, ..Default::default() },
        Filter { only_on: Some(vec![Match::Terrain(2), Match::Water]), ..Default::default() },
        Filter { except_on: Some(vec![Match::Biome(5), Match::Terrain(3)]), ..Default::default() },
        Filter { slope_above: true, slope: 25.0, ..Default::default() },
        Filter { slope_below: true, slope: 12.0, above: Some(64), ..Default::default() },
        Filter { facing_on: true, facing: 135.0, facing_spread: 50.0, ..Default::default() },
        Filter { facing_on: true, facing: 300.0, facing_spread: 20.0, slope_above: true, slope: 10.0, ..Default::default() },
        Filter { hollows: true, depth: 1.5, radius: 12, ..Default::default() },
        Filter { bumps: true, depth: 1.0, radius: 40, below: Some(95), fade: true, ..Default::default() },
    ];
    for (k, f) in filters.into_iter().enumerate() {
        let brush = filtered(110.0, f);
        let y = -330.0 + k as f32 * 60.0;
        stroke(&mut world, &mut history, (-400.0, y), (400.0, -y * 0.5), 9, |e, x, z, s| ops::paint_terrain(e, &brush, x, z, TerrainId(5 + k as u16), s));
        stroke(&mut world, &mut history, (400.0, y), (-400.0, y + 40.0), 7, |e, x, z, _| ops::raise(e, &brush, x, z, 2.0, Surface::Top));
        stroke(&mut world, &mut history, (-400.0, y + 20.0), (400.0, y), 5, |e, x, z, _| ops::smooth(e, &brush, x, z, Surface::Top));
        sums.push(checksum(&world));
    }

    let expected: [u64; 18] = [
        0x679ec8a6280429bf, 0x9905114175f4b7c4, 0x9734540aa0f93437,
        0xc8bc705c11ee2dbd, 0x59ff57767ae848bd, 0xb073c4dc9bc69612,
        0xa9e738daece510e7, 0xa370b5397ead4b31, 0x401f5237e7884af3,
        0x539acb32f146ebc0, 0xd4ec0f1398205287, 0xfea1b67766677d2a,
        0x984ddbe071c6d5a6, 0x058cce81bd4d74b0, 0x507e5ec1e737b67c,
        0x866e9784634bae73, 0x444ca4e0cfc7cdca, 0x27e21b16706b426e,
    ];
    let got: Vec<String> = sums.iter().map(|s| format!("0x{s:016x}")).collect();
    assert_eq!(sums, expected, "got [{}]", got.join(", "));
}
