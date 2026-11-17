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

/// A stroke as the app paints it: the sides kept from the strokes before
/// it, of the same gradient.
fn stroke(w: &mut World, g: &Gradient, brush: &Brush, dabs: &[(f32, f32)], sides: &mut Sides) {
    let sampler = g.sampler(&w.registry, w.meta.seed).unwrap();
    sides.begin(7);
    let mut history = History::default();
    history.begin("Gradient", 0);
    let mut edit = history.edit(w, 0);
    for (k, &(x, z)) in dabs.iter().enumerate() {
        ops::paint_gradient(&mut edit, brush, x, z, k as u64, &sampler, Layout::Across, sides);
    }
    history.commit();
    sides.end(&w.dims[0], 0);
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

/// Grass from x = 0 westward and a square of `first` (the forest)
/// eastward, as `map` lays stone; a shore of sand from x = -14 to -8, its
/// western half under the water, crossing the edge's reach from north to
/// south.
fn shore(first: &str) -> (World, TerrainId) {
    let (mut w, _, stone) = map(2);
    let reg = w.registry.clone();
    let (first, sand) = (reg.terrain_by_name(first).unwrap(), reg.terrain_by_name("Sand").unwrap());
    let mut history = History::default();
    history.begin("base", 0);
    let mut edit = history.edit(&mut w, 0);
    if first != stone {
        let square = Brush::symmetric(BrushShape::Square, Profile::Constant, 100.0);
        ops::paint_terrain(&mut edit, &square, 100.0, 0.0, first, 1);
    }
    let strip = Brush::symmetric(BrushShape::Square, Profile::Constant, 3.0);
    for z in (-200..200).step_by(4) {
        ops::paint_terrain(&mut edit, &strip, -11.0, z as f32, sand, 1);
    }
    history.commit();
    // The western half of the shore under the sea.
    for tz in -2..2 {
        let tile = w.dims[0].ground_mut().tile_mut(TilePos::new(-1, tz)).unwrap();
        for i in 0..massif_core::coords::TILE_AREA {
            let x = -128 + (i % 128) as i32;
            if (-14..-11).contains(&x) {
                tile.top.set(i, 60.0);
            }
        }
    }
    (w, sand)
}

fn snapshot(w: &World) -> Vec<u16> {
    (-80..80).flat_map(|z| (-80..80).map(move |x| (x, z))).map(|(x, z)| terrain(w, x, z).0).collect()
}

/// The columns where two maps differ: (x, z, terrain in the first, in the
/// second), the first few.
fn differ(a: &World, b: &World) -> Vec<(i32, i32, u16, u16)> {
    (-80..80).flat_map(|z| (-80..80).map(move |x| (x, z))).filter(|&(x, z)| terrain(a, x, z) != terrain(b, x, z)).map(|(x, z)| (x, z, terrain(a, x, z).0, terrain(b, x, z).0)).take(12).collect()
}

/// Clicking again over the same edge changes nothing: the boundary is found
/// between the gradient's first and last terrains, the middle ones it laid
/// being its transition, not new terrains; a terrain it does not paint (a
/// sand shore, a sea floor) is never painted. The terraformers' gradient
/// (noise, blobs, a hard cut, wander) and one with a wide wander, each
/// stroke three dabs, one, two and five strokes.
#[test]
fn clicking_again_over_the_same_edge_changes_nothing() {
    let sketch = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
    let wide = Gradient { wander: 0.8, transitions: vec![Transition::noise(8.0, 0.9, 4.0), Transition::blobs(8.0, 0.7, 5.0), Transition::noise(6.0, 0.6, 3.0)], ..sketch.clone() };
    let two = Gradient { wander: 0.5, ..Gradient::new("two", &[("Podzol", 6.0), ("Grass", 10.0)], &[Transition::noise(10.0, 0.9, 3.0)]) };
    for g in [sketch, wide, two] {
        let (base, sand) = shore(&g.bands[0].terrain);
        let dabs = [(0.0, -20.0), (0.0, 0.0), (0.0, 20.0)];
        let after = |strokes: usize| {
            let mut w = base.copy_unfrozen();
            let mut sides = Sides::new();
            for _ in 0..strokes {
                stroke(&mut w, &g, &brush(60.0), &dabs, &mut sides);
            }
            w
        };
        let (one, two, five) = (after(1), after(2), after(5));
        assert_ne!(snapshot(&one), snapshot(&base), "{}: the first stroke paints", g.name);
        assert_eq!(differ(&one, &two), [], "{}: a second stroke changes nothing", g.name);
        assert_eq!(differ(&one, &five), [], "{}: nor five", g.name);
        for z in -60..60 {
            for x in -14..-8 {
                assert_eq!(terrain(&five, x, z), sand, "{}: the shore at {x}, {z} stays sand, under the water too", g.name);
            }
        }
        // Without what the strokes before learnt (the map opened again),
        // the boundary is found between the first and last terrains alone:
        // a second stroke leaves the middle ones, the shore and all but a
        // few columns where the first terrain lies scattered past the edge
        // as they are, and never creeps.
        let mut again = one.copy_unfrozen();
        paint(&mut again, &g, Layout::Across, &brush(60.0), &dabs);
        let changed = (-80..80).flat_map(|z| (-80..80).map(move |x| (x, z))).filter(|&(x, z)| terrain(&again, x, z) != terrain(&one, x, z)).collect::<Vec<_>>();
        eprintln!("{}: {} columns changed by a stroke without memory", g.name, changed.len());
        if g.bands.len() > 2 {
            assert!(changed.is_empty(), "{}: {} columns changed", g.name, changed.len());
        }
        let reach = (g.total() - g.edge) / (1.0 - g.wander) + 2.0;
        assert!(changed.iter().all(|&(x, _)| (x as f32) > -reach && !(-14..-8).contains(&x)), "{}: {changed:?}", g.name);
    }
}

/// With a soft brush, a second stroke fills in what the first left at its
/// rim, and nothing past where the gradient ends: it does not creep.
#[test]
fn a_soft_brush_fills_in_without_creeping() {
    let g = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
    let (base, sand) = shore(&g.bands[0].terrain);
    let reg = base.registry.clone();
    let grass = reg.terrain_by_name("Grass").unwrap();
    let soft = Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, 70.0) };
    let mut w = base.copy_unfrozen();
    let mut sides = Sides::new();
    for _ in 0..5 {
        stroke(&mut w, &g, &soft, &[(0.0, 0.0)], &mut sides);
    }
    // West of the edge, farther than the gradient's length from it, the
    // grass is grass; the shore is sand.
    let reach = (g.total() - g.edge) / (1.0 - g.wander) + 2.0;
    for z in -40..40 {
        for x in -70..0 {
            let t = terrain(&w, x, z);
            if (-14..-8).contains(&x) {
                assert_eq!(t, sand, "{x}, {z}");
            } else if (-x as f32) > reach {
                assert_eq!(t, grass, "{x}, {z}: past the gradient's end");
            }
        }
    }
}

/// A shore that bends within the brush, the first click: grass with a
/// forest of `first` from x = 0 eastward, a beach of sand turning a corner
/// (down x = -14..-8, then along z = 20..26 to the forest's edge), and the
/// sea beyond it (its floor grass, under the water). The gradient paints
/// the grass in the inner corner and leaves every grain of sand and the
/// sea floor as they were, at the bend too, from the first click on.
#[test]
fn a_bend_in_the_shore_is_left_as_it_is_from_the_first_click() {
    let g = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
    let (mut w, _, stone) = map(2);
    let reg = w.registry.clone();
    let (first, sand, grass) = (reg.terrain_by_name(&g.bands[0].terrain).unwrap(), reg.terrain_by_name("Sand").unwrap(), reg.terrain_by_name("Grass").unwrap());
    assert_ne!(first, stone);
    let mut history = History::default();
    history.begin("base", 0);
    let mut edit = history.edit(&mut w, 0);
    ops::paint_terrain(&mut edit, &Brush::symmetric(BrushShape::Square, Profile::Constant, 100.0), 100.0, 0.0, first, 1);
    let cell = Brush::symmetric(BrushShape::Square, Profile::Constant, 0.5);
    let beach = |x: i32, z: i32| ((-14..-8).contains(&x) && z < 26) || ((-14..-2).contains(&x) && (20..26).contains(&z));
    let sea = |x: i32, z: i32| (x < -14 && z < 26) || (x < -2 && z >= 26);
    for z in -100..100 {
        for x in -60..0 {
            if beach(x, z) {
                ops::paint_terrain(&mut edit, &cell, x as f32 + 0.5, z as f32 + 0.5, sand, 1);
            }
        }
    }
    history.commit();
    // The sea: its floor four blocks under the water.
    for tz in -2..2 {
        for tx in -2..1 {
            let Some(tile) = w.dims[0].ground_mut().tile_mut(TilePos::new(tx, tz)) else { continue };
            for i in 0..massif_core::coords::TILE_AREA {
                let (x, z) = (tx * 128 + (i % 128) as i32, tz * 128 + (i / 128) as i32);
                if sea(x, z) {
                    tile.top.set(i, 59.0);
                }
            }
        }
    }
    let before = w.copy_unfrozen();
    // One click by the corner, the brush over the beach, the bend and the sea.
    let soft = Brush { strength: 1.0, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, 60.0) };
    let mut sides = Sides::new();
    stroke(&mut w, &g, &soft, &[(0.0, 10.0)], &mut sides);
    let mut painted = 0;
    for z in -60..80 {
        for x in -60..0 {
            let (was, now) = (terrain(&before, x, z), terrain(&w, x, z));
            if beach(x, z) || sea(x, z) {
                assert_eq!(now, was, "{x}, {z}: {} stays as it was", if beach(x, z) { "the beach" } else { "the sea floor" });
            } else if was == grass && now != grass {
                painted += 1;
            }
        }
    }
    assert!(painted > 100, "the grass in the inner corner is painted: {painted} columns");
    // A second click changes nothing but the soft rim it fills in.
    let once = w.copy_unfrozen();
    stroke(&mut w, &g, &soft, &[(0.0, 10.0)], &mut sides);
    for z in -60..80 {
        for x in -60..0 {
            if beach(x, z) || sea(x, z) {
                assert_eq!(terrain(&w, x, z), terrain(&once, x, z), "{x}, {z}");
            }
        }
    }
}
