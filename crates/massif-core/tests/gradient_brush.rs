//! The Gradient brush on a map: laid out across a boundary, from the
//! brush's centre, in a selection; one undoable stroke, the same each time.

use massif_core::gradient::{Gradient, Layout, Sides, Transition};
use massif_core::history::History;
use massif_core::ops;
use massif_core::registry::TerrainId;
use massif_core::selection::{Selection, Shape};
use massif_core::{BlockRect, Brush, BrushShape, Profile, Tile, TilePos, World, WorldMeta};

/// A flat map of grass, `tiles` tiles each way from the origin, with a
/// square of stone from x = 0 eastward.
fn map(tiles: i32) -> (World, TerrainId, TerrainId) {
    let mut w = World::new(WorldMeta { seed: 9, ..WorldMeta::default() });
    let grass = w.registry.terrain_by_name("Grass").unwrap();
    let stone = w.registry.terrain_by_name("Stone").unwrap();
    for z in -tiles..tiles {
        for x in -tiles..tiles {
            w.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(64.0, grass.0, 63));
        }
    }
    let mut history = History::default();
    history.begin("base", 0);
    let mut edit = history.edit(&mut w, 0);
    let square = Brush::symmetric(BrushShape::Square, Profile::Constant, 100.0);
    ops::paint_terrain(&mut edit, &square, 100.0, 0.0, stone, 1);
    history.commit();
    (w, grass, stone)
}

fn terrain(w: &World, x: i32, z: i32) -> TerrainId {
    let tile = w.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
    TerrainId(tile.terrain.get(massif_core::coords::local_index(x, z)))
}

/// Stone, then Andesite and Gravel, then grass: hard cuts, 4 blocks each,
/// the boundary after the stone.
fn rings() -> Gradient {
    Gradient { edge: 4.0, wander: 0.0, ..Gradient::new("rings", &[("Stone", 4.0), ("Andesite", 4.0), ("Gravel", 4.0), ("Grass", 4.0)], &[Transition::hard(); 3]) }
}

fn paint(w: &mut World, g: &Gradient, layout: Layout, brush: &Brush, dabs: &[(f32, f32)]) -> History {
    let sampler = g.sampler(&w.registry, w.meta.seed).unwrap();
    let mut history = History::default();
    history.begin("Gradient", 0);
    let mut edit = history.edit(w, 0);
    let mut sides = Sides::new();
    for (k, &(x, z)) in dabs.iter().enumerate() {
        ops::paint_gradient(&mut edit, brush, x, z, k as u64, &sampler, layout, &mut sides);
    }
    history.commit();
    history
}

fn brush(r: f32) -> Brush {
    Brush::symmetric(BrushShape::Circle, Profile::Constant, r)
}

#[test]
fn across_a_boundary_the_bands_follow_the_distance_to_it() {
    let (mut w, grass, stone) = map(2);
    let reg = w.registry.clone();
    let (andesite, gravel) = (reg.terrain_by_name("Andesite").unwrap(), reg.terrain_by_name("Gravel").unwrap());
    // Over the west edge of the stone square, at x = 0.
    paint(&mut w, &rings(), Layout::Across, &brush(30.0), &[(0.0, 0.0), (0.0, 10.0)]);
    // The stone keeps its side; west of it, 4 blocks of andesite, then 4
    // of gravel, then grass.
    for (x, t) in [(10, stone), (0, stone), (-1, andesite), (-4, andesite), (-5, gravel), (-8, gravel), (-9, grass), (-20, grass)] {
        assert_eq!(terrain(&w, x, 5), t, "x = {x}");
    }
    // Outside the brush, nothing changed.
    assert_eq!(terrain(&w, -5, 60), grass);
}

#[test]
fn the_boundary_can_fall_in_the_middle() {
    let (mut w, grass, stone) = map(2);
    let reg = w.registry.clone();
    let (andesite, gravel) = (reg.terrain_by_name("Andesite").unwrap(), reg.terrain_by_name("Gravel").unwrap());
    let mut g = rings();
    g.edge = 8.0;
    paint(&mut w, &g, Layout::Across, &brush(30.0), &[(0.0, 0.0)]);
    for (x, t) in [(8, stone), (4, stone), (3, andesite), (0, andesite), (-1, gravel), (-4, gravel), (-5, grass)] {
        assert_eq!(terrain(&w, x, 0), t, "x = {x}");
    }
}

#[test]
fn from_the_centre_a_patch_with_rings() {
    let (mut w, _, stone) = map(2);
    let reg = w.registry.clone();
    let (andesite, gravel) = (reg.terrain_by_name("Andesite").unwrap(), reg.terrain_by_name("Gravel").unwrap());
    // 16 blocks of gradient over a radius of 32: 8 blocks a band.
    paint(&mut w, &rings(), Layout::Radial, &brush(32.0), &[(-100.0, 0.0)]);
    for (dx, t) in [(0, stone), (6, stone), (10, andesite), (18, gravel)] {
        assert_eq!(terrain(&w, -100 + dx, 0), t, "dx = {dx}");
        assert_eq!(terrain(&w, -100, -dx - 1), t, "dz = {dx}");
    }
}

#[test]
fn in_a_selection_it_runs_in_from_its_edge() {
    let (mut w, grass, stone) = map(2);
    let reg = w.registry.clone();
    let andesite = reg.terrain_by_name("Andesite").unwrap();
    let mut history = History::default();
    history.select(&mut w, 0, "Select", Some(Selection::shape(&Shape::Rect(BlockRect::new(-60, -60, -20, -20)))));
    paint(&mut w, &rings(), Layout::Across, &brush(60.0), &[(-40.0, -40.0)]);
    // The selection's edge is the gradient's start.
    assert_eq!(terrain(&w, -60, -40), stone);
    assert_eq!(terrain(&w, -56, -40), andesite);
    assert_eq!(terrain(&w, -40, -40), grass);
    // Outside it, nothing changed.
    assert_eq!(terrain(&w, -61, -40), grass);
    assert_eq!(terrain(&w, -10, -40), grass);
}

#[test]
fn one_stroke_is_one_undo_and_paints_the_same_each_time() {
    let (base, _, _) = map(2);
    let g = Gradient { transitions: vec![Transition::noise(8.0, 0.9, 4.0), Transition::blobs(8.0, 0.7, 5.0), Transition::noise(6.0, 0.6, 3.0)], ..Gradient::default() };
    let dabs: Vec<(f32, f32)> = (0..12).map(|k| (-2.0 + k as f32 * 0.7, -40.0 + k as f32 * 8.0)).collect();
    let soft = Brush { strength: 0.8, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, 40.0) };
    let snapshot = |w: &World| -> Vec<u16> { (-60..60).flat_map(|z| (-60..60).map(move |x| (x, z))).map(|(x, z)| terrain(w, x, z).0).collect() };
    let mut a = base.copy_unfrozen();
    let mut history = paint(&mut a, &g, Layout::Across, &soft, &dabs);
    let mut b = base.copy_unfrozen();
    paint(&mut b, &g, Layout::Across, &soft, &dabs);
    assert_eq!(snapshot(&a), snapshot(&b));
    assert_ne!(snapshot(&a), snapshot(&base));
    assert_eq!(history.undo_count(), 1);
    history.undo(&mut a);
    assert_eq!(snapshot(&a), snapshot(&base));
}

#[test]
fn the_boundary_does_not_move_under_a_stroke() {
    // Dab after dab over the same place paints what one dab does: the
    // stroke measures from the boundary as it was when it began.
    let (base, _, _) = map(2);
    let g = Gradient { edge: 6.0, ..Gradient::default() };
    let snapshot = |w: &World| -> Vec<u16> { (-30..30).flat_map(|z| (-30..30).map(move |x| (x, z))).map(|(x, z)| terrain(w, x, z).0).collect() };
    let mut once = base.copy_unfrozen();
    paint(&mut once, &g, Layout::Across, &brush(40.0), &[(0.0, 0.0)]);
    let mut many = base.copy_unfrozen();
    paint(&mut many, &g, Layout::Across, &brush(40.0), &[(0.0, 0.0); 6]);
    assert_eq!(snapshot(&once), snapshot(&many));
}

#[test]
fn wander_widens_and_narrows_the_rings_but_keeps_the_edge() {
    let (mut w, grass, stone) = map(2);
    let reg = w.registry.clone();
    let andesite = reg.terrain_by_name("Andesite").unwrap();
    let g = Gradient { wander: 0.8, ..rings() };
    let dabs: Vec<(f32, f32)> = (0..9).map(|k| (0.0, -96.0 + k as f32 * 24.0)).collect();
    paint(&mut w, &g, Layout::Across, &brush(40.0), &dabs);
    // Along the edge (x = 0), the andesite ring is wide here, thin there.
    let widths: Vec<usize> = (-90..90).map(|z| (1..30).filter(|&d| terrain(&w, -d, z) == andesite).count()).collect();
    let (min, max) = (widths.iter().min().unwrap(), widths.iter().max().unwrap());
    assert!(*max >= *min + 3, "rings of {min} to {max} blocks");
    // The edge stays: stone on its side, the rings on the other.
    for z in -90..90 {
        assert_eq!(terrain(&w, 0, z), stone, "z = {z}");
        assert_ne!(terrain(&w, -1, z), stone, "z = {z}");
        assert_ne!(terrain(&w, -1, z), grass, "z = {z}");
    }
}

#[test]
fn a_brush_filter_is_kept_to() {
    let (mut w, grass, _) = map(2);
    let only_stone = massif_core::filter::Filter { only_on: Some(vec![massif_core::filter::Match::Terrain(w.registry.terrain_by_name("Stone").unwrap().0)]), ..Default::default() };
    let filtered = Brush { filter: Some(std::sync::Arc::new(only_stone)), ..brush(30.0) };
    paint(&mut w, &rings(), Layout::Across, &filtered, &[(0.0, 0.0)]);
    // The grass side is not stone: left as it was.
    assert_eq!(terrain(&w, -3, 0), grass);
}
