//! The Spawn point tool, as a user drives it: a click on the map puts the
//! spawn on the ground there, the pin dragged moves it, the map's
//! right-click menu sets it, each as one step to undo; in the Overworld
//! only. With MASSIF_SNAPSHOT_DIR, the pin is rendered at 1× and 2×.

use super::tests::harness_with;
use super::*;
use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use massif_core::generate::{GenParams, Landform, generate};

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(|d| {
        let dir = PathBuf::from(d).join("spawn-tool");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

fn densities() -> Vec<f32> {
    if snapshot_dir().is_some() { vec![1.0, 2.0] } else { vec![1.0] }
}

fn save(h: &mut Harness<'_, MassifApp>, name: &str, ppp: f32) {
    if let Some(dir) = snapshot_dir() {
        h.render().unwrap().save(dir.join(format!("{name}-{ppp}x.png"))).unwrap();
    }
}

/// The whole app on a map of hills, `tool` in hand, the map at `(x, z)`.
fn app(ppp: f32, tool: Tool, x: f64, z: f64) -> Harness<'static, MassifApp> {
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(3);
    {
        let app = h.state_mut();
        let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Hills, ..GenParams::default() });
        app.doc = crate::doc::Document::new(world, None, 43);
        app.map_cache.invalidate_all();
        app.view3d.invalidate_all();
        app.tools = crate::tools::ToolState::default();
        app.tools.select(tool);
        let m = app.maps.values_mut().next().unwrap();
        m.zoom = 4.0;
        m.center = [x, z];
    }
    h.run_steps(5);
    h
}

fn screen_of(h: &Harness<'_, MassifApp>, x: f64, z: f64) -> Pos2 {
    h.state().maps.values().next().expect("a map").screen_of(x, z)
}

fn button(h: &mut Harness<'_, MassifApp>, p: Pos2, button: PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE });
    h.step();
}

/// Where a player stands on the column: the first block over the ground.
fn ground_spawn(h: &Harness<'_, MassifApp>, x: i32, z: i32) -> [i32; 3] {
    let p = crate::tools::probe(&h.state().doc, x, z).expect("on the map");
    assert_eq!(p.fluid, massif_core::tile::NO_FLUID, "dry land at {x}, {z}");
    [x, massif_core::compose::block_top(p.height), z]
}

#[test]
fn a_click_puts_the_spawn_on_the_ground_as_one_step() {
    for ppp in densities() {
        let mut h = app(ppp, Tool::Spawn, 200.0, 200.0);
        let before = h.state().doc.world.meta.spawn;
        let steps = h.state().doc.history.undo_count();
        { let p = screen_of(&h, 200.5, 210.5); super::tests::click_on(&mut h, p, PointerButton::Primary); }
        h.run_steps(2);
        let want = ground_spawn(&h, 200, 210);
        assert_eq!(h.state().doc.world.meta.spawn, want, "on the ground clicked");
        assert!(want[1] > h.state().doc.dimension().min_y);
        assert_eq!(h.state().doc.history.undo_count(), steps + 1, "one step");
        assert_eq!(h.state().doc.history.undo_label(), Some("Move the spawn point"));
        save(&mut h, "spawn-tool", ppp);
        h.state_mut().doc.undo();
        assert_eq!(h.state().doc.world.meta.spawn, before, "undone");
        h.state_mut().doc.redo();
        assert_eq!(h.state().doc.world.meta.spawn, want, "redone");
        // The same place again: nothing to undo more.
        { let p = screen_of(&h, 200.5, 210.5); super::tests::click_on(&mut h, p, PointerButton::Primary); }
        assert_eq!(h.state().doc.history.undo_count(), steps + 1);
    }
}

#[test]
fn the_pin_dragged_moves_the_spawn_in_one_step() {
    let mut h = app(1.0, Tool::Spawn, 200.0, 200.0);
    h.state_mut().doc.world.meta.spawn = [190, 100, 195];
    h.run_steps(2);
    let steps = h.state().doc.history.undo_count();
    let from = screen_of(&h, 190.5, 195.5);
    h.event(egui::Event::PointerMoved(from));
    h.step();
    button(&mut h, from, PointerButton::Primary, true);
    for k in 1..=6 {
        let t = k as f64 / 6.0;
        h.event(egui::Event::PointerMoved(screen_of(&h, 190.5 + 20.0 * t, 195.5 + 10.0 * t)));
        h.step();
    }
    assert_eq!(h.state().doc.world.meta.spawn, [190, 100, 195], "held: not moved yet");
    assert_eq!(h.state().doc.history.undo_count(), steps, "no step while held");
    { let p = screen_of(&h, 210.5, 205.5); button(&mut h, p, PointerButton::Primary, false); }
    h.run_steps(2);
    assert_eq!(h.state().doc.world.meta.spawn, ground_spawn(&h, 210, 205), "let go: there");
    assert_eq!(h.state().doc.history.undo_count(), steps + 1, "one step for the drag");
}

#[test]
fn the_maps_menu_sets_the_spawn_here() {
    for tool in [Tool::Pan, Tool::Spawn] {
        let mut h = app(1.0, tool, 200.0, 200.0);
        let p = screen_of(&h, 215.5, 190.5);
        super::tests::click_on(&mut h, p, PointerButton::Secondary);
        h.run_steps(2);
        h.get_by_label_contains("Set spawn here").click();
        h.run_steps(3);
        assert_eq!(h.state().doc.world.meta.spawn, ground_spawn(&h, 215, 190), "{tool:?}");
        assert_eq!(h.state().tools.tool, tool, "the tool kept");
    }
    // Painting, the right button is the tool's own: no menu.
    let mut h = app(1.0, Tool::Raise, 200.0, 200.0);
    { let p = screen_of(&h, 215.5, 190.5); super::tests::click_on(&mut h, p, PointerButton::Secondary); }
    h.run_steps(2);
    assert!(h.query_by_label_contains("Set spawn here").is_none());
}

#[test]
fn the_spawn_is_in_the_overworld_only() {
    let world = generate(&GenParams { tiles_x: 4, tiles_z: 4, ..GenParams::default() });
    let mut doc = crate::doc::Document::new(world, None, 1);
    let before = doc.world.meta.spawn;
    doc.world.dims[0].kind = massif_core::world::DimensionKind::Nether;
    assert!(crate::tools::set_spawn(&mut doc, 10, 10).is_some_and(|m| m.contains("Overworld")));
    assert_eq!(doc.world.meta.spawn, before);
    doc.world.dims[0].kind = massif_core::world::DimensionKind::Overworld;
    assert!(crate::tools::set_spawn(&mut doc, 10, 10).is_some());
    assert_eq!([doc.world.meta.spawn[0], doc.world.meta.spawn[2]], [10, 10]);
    // Off the map: no ground, nothing moved.
    let now = doc.world.meta.spawn;
    assert!(crate::tools::set_spawn(&mut doc, -5000, -5000).is_some());
    assert_eq!(doc.world.meta.spawn, now);
}

/// On water, a player stands on its surface.
#[test]
fn on_water_the_spawn_stands_on_the_surface() {
    let world = generate(&GenParams { tiles_x: 4, tiles_z: 4, ..GenParams::default() });
    let mut doc = crate::doc::Document::new(world, None, 1);
    let pos = massif_core::TilePos::containing(20, 20);
    let tile = doc.world.dims[0].ground_mut().tile_mut(pos).unwrap();
    let i = massif_core::coords::local_index(20, 20);
    tile.top.set(i, 40.0);
    tile.fluid.set(i, 62);
    let spawn = crate::tools::spawn_at(&doc, 20, 20).unwrap();
    assert_eq!(spawn, [20, massif_core::compose::block_top(62.0), 20]);
}
