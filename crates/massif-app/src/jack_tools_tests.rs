//! The tools the ArdaCraft team's terraformers asked for in 1.1, driven as
//! a user drives them: the Info panel as an inspector of the column under
//! the cursor and File ▸ World Information…, the word said when a stroke
//! falls all outside the selection, the filters' eyedropper and their
//! menu, Find on a layer, the Pencil, Alt and the wheel turning the brush,
//! and the region files' borders on the grid. With MASSIF_SNAPSHOT_DIR,
//! each is rendered at 1× and 2×.

use super::tests::harness_with;
use super::*;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use massif_core::coords::local_index;
use massif_core::filter::{Match, Painted};
use massif_core::generate::{GenParams, Landform, generate};

/// The terrain of the ground at `(x, z)`.
fn terrain_at(dim: &massif_core::Dimension, x: i32, z: i32) -> Option<u16> {
    dim.ground().view(massif_core::TilePos::containing(x, z)).map(|t| t.terrain.get(local_index(x, z)))
}

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(|d| {
        let dir = PathBuf::from(d).join("jack-tools");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// The pixel densities to test at: 1× and 2× when rendering, 1× else.
fn densities() -> Vec<f32> {
    if snapshot_dir().is_some() { vec![1.0, 2.0] } else { vec![1.0] }
}

fn save(h: &mut Harness<'_, MassifApp>, name: &str, ppp: f32) {
    if let Some(dir) = snapshot_dir() {
        h.render().unwrap().save(dir.join(format!("{name}-{ppp}x.png"))).unwrap();
    }
}

/// A map of hills, a cover, a tree layer, pockets and an annotation
/// painted at a few columns.
fn painted_world() -> massif_core::World {
    let mut world = generate(&GenParams { tiles_x: 16, tiles_z: 16, landform: Landform::Hills, ..GenParams::default() });
    let reg = &mut world.registry;
    let trees = reg.add_object_layer(massif_core::objects::ObjectLayerDef::new("Oak wood", &[], reg));
    let stone = reg.terrain_by_name("Stone").unwrap().0;
    let pockets = reg.add_pocket(massif_core::pockets::PocketDef::new("Iron veins", [180, 120, 90], &[], 0.2, 4.0, 0, 60));
    let ground = world.dims[0].ground_mut();
    for (x, z) in [(300, 300), (301, 300), (302, 300), (900, 700)] {
        let tile = ground.tile_mut(massif_core::TilePos::containing(x, z)).unwrap();
        let i = local_index(x, z);
        tile.cover.set(i, 1);
        tile.objects.set(i, trees);
        tile.pockets.set(i, pockets);
        tile.annotations.set(i, 4);
        tile.terrain.set(i, stone);
    }
    world
}

/// The whole app on a painted map, `tool` in hand.
fn app(ppp: f32, tool: Tool) -> Harness<'static, MassifApp> {
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(3);
    {
        let app = h.state_mut();
        app.doc = crate::doc::Document::new(painted_world(), None, 41);
        app.map_cache.invalidate_all();
        app.view3d.invalidate_all();
        app.tools = crate::tools::ToolState::default();
        app.tools.select(tool);
    }
    h.run_steps(5);
    h
}

fn map<'a>(h: &'a Harness<'_, MassifApp>) -> &'a crate::map_view::MapView {
    h.state().maps.values().next().expect("a map")
}

/// The map looking at `(x, z)` at `zoom` pixels a block.
fn look(h: &mut Harness<'_, MassifApp>, x: f64, z: f64, zoom: f32) {
    let m = h.state_mut().maps.values_mut().next().unwrap();
    m.zoom = zoom;
    m.center = [x, z];
    h.run_steps(3);
}

fn press(h: &mut Harness<'_, MassifApp>, p: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
    h.step();
}

// ------------------------------------------------------------ the Info box --

/// What the inspector reads of a column: its height, terrain, water,
/// biome, selection, and every layer painted there.
#[test]
fn the_inspector_reads_every_layer_of_a_column() {
    let mut doc = crate::doc::Document::new(painted_world(), None, 1);
    let info = crate::inspect::ColumnInfo::read(&doc, 300, 300).expect("on the map");
    let reg = &doc.world.registry;
    assert_eq!((info.x, info.z), (300, 300));
    assert!(info.height.is_some());
    assert_eq!(reg.terrain(info.terrain.unwrap()).name, "Stone");
    assert_eq!(info.selected, None, "no selection");
    let kinds: Vec<&str> = info.layers.iter().map(|l| l.kind).collect();
    for kind in ["Cover", "Trees", "Pockets", "Annotation"] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    assert!(info.layers.iter().any(|l| l.name == "Oak wood"));
    assert!(info.layers.iter().any(|l| l.name == "Iron veins" && l.intensity == Some(0.2)));
    // What the eyedropper offers: the terrain, water or land, the layers.
    let choices = info.choices(reg);
    assert!(choices.iter().any(|c| c.0 == Match::Terrain(reg.terrain_by_name("Stone").unwrap().0)));
    assert!(choices.iter().any(|c| c.0 == Match::Layer(Painted::Cover, 1)));
    assert!(choices.iter().any(|c| matches!(c.0, Match::Layer(Painted::Objects, _))));
    assert!(choices.iter().any(|c| c.0 == Match::Layer(Painted::Annotation, 4)));
    // A selection: said, in or out.
    doc.select("Select", Some(massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(massif_core::BlockRect::new(290, 290, 310, 310)))));
    assert_eq!(crate::inspect::ColumnInfo::read(&doc, 300, 300).unwrap().selected, Some(1.0));
    assert_eq!(crate::inspect::ColumnInfo::read(&doc, 900, 700).unwrap().selected, Some(0.0));
    assert!(crate::inspect::ColumnInfo::read(&doc, -5000, 0).is_none(), "off the map");
}

/// The Info panel follows the cursor over the map, and the world's
/// settings are in File ▸ World Information… (rendered at 1× and 2×).
#[test]
fn the_info_panel_follows_the_cursor_and_the_world_has_its_dialog() {
    use egui_kittest::kittest::Queryable;
    for ppp in densities() {
        let mut h = app(ppp, Tool::Pan);
        // The default workspace shows the Info panel.
        let mut areas = Vec::new();
        h.state().layout.current().root.areas(&mut areas);
        assert!(areas.iter().any(|(_, k)| *k == EditorKind::Info));
        look(&mut h, 301.0, 300.0, 6.0);
        let p = map(&h).screen_of(301.5, 300.5);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(4);
        assert_eq!(h.state().hover, Some((301, 300)));
        // (Named in the Layers panel too.)
        let named = |h: &Harness<'_, MassifApp>, name: &str| h.query_all_by_label(name).count();
        let (oak, iron) = (named(&h, "Oak wood"), named(&h, "Iron veins"));
        assert!(oak >= 1 && iron >= 1, "the tree layer and the pockets named");
        assert!(h.query_all_by_label("301, 300").next().is_some(), "the coordinates");
        save(&mut h, "info-panel", ppp);
        // Moved to a column without layers: they go.
        let q = map(&h).screen_of(320.5, 330.5);
        h.event(egui::Event::PointerMoved(q));
        h.run_steps(4);
        assert_eq!(named(&h, "Oak wood"), oak - 1, "no longer in the Info panel");
        assert_eq!(named(&h, "Iron veins"), iron - 1);
        assert!(h.query_by_label_contains("No layer is painted here").is_some());
        // File ▸ World Information…
        let ctx = h.ctx.clone();
        h.state_mut().command(&ctx, Command::WorldInfo);
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(Dialog::WorldInfo)));
        assert!(h.query_by_label("World Information").is_some());
        assert!(h.query_by_label("Seed").is_some());
        assert!(h.query_by_label_contains("Map cache").is_some());
        save(&mut h, "world-info", ppp);
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
    }
}

// --------------------------------------------- outside the selection toast --

/// A stroke all outside the selection says why nothing happens, once; one
/// that reaches it says nothing.
#[test]
fn a_stroke_outside_the_selection_says_so_once() {
    let mut doc = crate::doc::Document::new(painted_world(), None, 1);
    doc.select("Select", Some(massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(massif_core::BlockRect::new(100, 100, 140, 140)))));
    let mut tools = crate::tools::ToolState::default();
    tools.select(Tool::PaintTerrain);
    tools.brush.radius = 4.0;
    let (stroke, _) = crate::tools::begin_stroke(&mut doc, &tools, (400.0, 400.0), false, false);
    let mut stroke = stroke.unwrap();
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (400.0, 400.0), 0.02);
    let note = stroke.take_note().expect("said");
    assert!(note.contains("selection"), "{note}");
    for k in 1..20 {
        crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (400.0 + k as f32, 400.0), 0.02);
        assert!(stroke.take_note().is_none(), "said once");
    }
    let _ = crate::tools::end_stroke(&mut doc, stroke);
    // Begun over its edge: nothing said.
    let (stroke, _) = crate::tools::begin_stroke(&mut doc, &tools, (143.0, 120.0), false, false);
    let mut stroke = stroke.unwrap();
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (143.0, 120.0), 0.02);
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (130.0, 120.0), 0.02);
    assert!(stroke.take_note().is_none());
    let _ = crate::tools::end_stroke(&mut doc, stroke);
}

/// The word at the foot of the window, a stroke on the map outside the
/// selection (rendered at 1× and 2×).
#[test]
fn the_selection_toast_shows_on_the_map() {
    for ppp in densities() {
        let mut h = app(ppp, Tool::PaintTerrain);
        h.state_mut().doc.select("Select", Some(massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(massif_core::BlockRect::new(200, 200, 260, 260)))));
        look(&mut h, 400.0, 400.0, 2.0);
        let (a, b) = (map(&h).screen_of(380.0, 400.0), map(&h).screen_of(430.0, 420.0));
        super::tests::drag_on(&mut h, a, b);
        h.run_steps(3);
        let said = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        assert!(said.contains("selection"), "{said:?}");
        save(&mut h, "selection-toast", ppp);
    }
}

// ----------------------------------------------------- filters' eyedropper --

/// The filters' menu: searched by name or kind, what the map holds first,
/// the latest picked first, or by name.
#[test]
fn the_filter_menu_sorts_and_searches() {
    use crate::panels::{FilterRow, filter_items, filter_rows};
    use crate::tools::FilterSort;
    let world = painted_world();
    let reg = &world.registry;
    let mut presence = massif_core::presence::Presence::default();
    presence.update(&world.dims[0]);
    let used = presence.used();
    let items = filter_items(reg, Some(&used));
    assert!(items.iter().any(|i| i.what == Match::Layer(Painted::Annotation, 4)), "the annotation used is offered");
    assert!(!items.iter().any(|i| i.what == Match::Layer(Painted::Annotation, 9)), "not one unused");
    let name = |row: &FilterRow| match row {
        FilterRow::Item(i) => items[*i].name.clone(),
        FilterRow::Heading(h) => format!("# {h}"),
    };
    // On the map first.
    let rows: Vec<String> = filter_rows(&items, "", FilterSort::Used, &[], Some(&used)).iter().map(name).collect();
    let (here, away) = (rows.iter().position(|r| r == "# On this map").unwrap(), rows.iter().position(|r| r == "# Not on this map").unwrap());
    let at = |n: &str| rows.iter().position(|r| r == n).unwrap_or_else(|| panic!("{n} in {rows:?}"));
    assert!(here < at("Stone") && at("Stone") < away, "Stone is on the map");
    assert!(here < at("Oak wood") && at("Oak wood") < away);
    assert!(at("Badlands") > away, "no badlands painted on these hills");
    // Searched.
    let rows: Vec<String> = filter_rows(&items, "oak", FilterSort::Name, &[], Some(&used)).iter().map(name).collect();
    assert_eq!(rows, ["Oak wood"]);
    let rows: Vec<String> = filter_rows(&items, "pockets", FilterSort::Name, &[], Some(&used)).iter().map(name).collect();
    assert_eq!(rows, ["Iron veins"], "by kind too");
    // Lately first.
    let recent = [Match::Layer(Painted::Cover, 1), Match::Water];
    let rows: Vec<String> = filter_rows(&items, "", FilterSort::Recent, &recent, Some(&used)).iter().map(name).collect();
    assert_eq!(rows[0], "# Lately");
    assert_eq!(rows[1], reg.cover(1).unwrap().name);
    assert_eq!(rows[2], "Water");
    assert_eq!(rows[3], "# All");
}

/// The eyedropper, from Brush Settings: a click on the map shows what is
/// there by the pointer; a click on one makes it the filter, and the
/// brush is back in hand (rendered at 1× and 2×).
#[test]
fn the_filter_eyedropper_picks_on_the_map() {
    use egui_kittest::kittest::Queryable;
    for ppp in densities() {
        let mut h = app(ppp, Tool::Cover);
        look(&mut h, 300.0, 300.0, 6.0);
        h.state_mut().tools.filter_pick = Some(true);
        h.run_steps(2);
        let revision = h.state().doc.revision;
        let p = map(&h).screen_of(300.5, 300.5);
        super::tests::click_on(&mut h, p, PointerButton::Primary);
        assert_eq!(h.state().doc.revision, revision, "the tool did nothing");
        assert!(h.query_by_label_contains("Act only on").is_some(), "the choices shown");
        assert!(h.query_by_label("Trees: Oak wood").is_some());
        save(&mut h, "eyedropper-popup", ppp);
        h.get_by_label("Trees: Oak wood").click();
        h.run_steps(3);
        let tools = &h.state().tools;
        assert!(tools.filter_pick.is_none(), "one shot");
        assert!(matches!(tools.filter.only_on.as_deref(), Some([Match::Layer(Painted::Objects, _)])), "{:?}", tools.filter.only_on);
        assert_eq!(tools.tool, Tool::Cover, "the brush back in hand");
        assert!(matches!(tools.filter_recent.first(), Some(Match::Layer(Painted::Objects, _))));
        // Esc puts the eyedropper down.
        h.state_mut().tools.filter_pick = Some(false);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().tools.filter_pick.is_none());
    }
}

/// A layer as the filter: the brush acts only where it is painted.
#[test]
fn a_layer_filter_keeps_the_brush_to_it() {
    let world = painted_world();
    let dim = &world.dims[0];
    let filter = massif_core::filter::Filter { only_on: Some(vec![Match::Layer(Painted::Cover, 1)]), ..Default::default() };
    let rect = massif_core::BlockRect::new(296, 296, 306, 304);
    let mask = filter.mask(dim, rect);
    assert_eq!(mask.get(301, 300), 1.0);
    assert_eq!(mask.get(303, 300), 0.0);
    let except = massif_core::filter::Filter { except_on: Some(vec![Match::Layer(Painted::Annotation, 4)]), ..Default::default() };
    let mask = except.mask(dim, rect);
    assert_eq!(mask.get(301, 300), 0.0);
    assert_eq!(mask.get(303, 300), 1.0);
}

// ------------------------------------------------------------------- Find --

/// Find on a layer: the map goes to the nearest place it is painted, then
/// to the next, and round again.
#[test]
fn find_goes_from_place_to_place() {
    let mut h = app(1.0, Tool::Pan);
    look(&mut h, 800.0, 650.0, 2.0);
    let ctx = h.ctx.clone();
    let what = Match::Layer(Painted::Cover, 1);
    h.state_mut().find_on_map(&ctx, what);
    let first = h.state().finding.clone().unwrap();
    assert_eq!(first.1.len(), 2, "two patches: {:?}", first.1);
    assert_eq!(first.1[0], (900, 700), "the nearest first");
    h.run_steps(60);
    let c = map(&h).center;
    assert!((c[0] - 900.5).abs() < 2.0 && (c[1] - 700.5).abs() < 2.0, "the map went there: {c:?}");
    h.state_mut().find_on_map(&ctx, what);
    assert_eq!(h.state().finding.as_ref().unwrap().2, 1);
    h.run_steps(60);
    let c = map(&h).center;
    assert!((c[0] - 300.0).abs() < 4.0 && (c[1] - 300.5).abs() < 2.0, "then the next: {c:?}");
    h.state_mut().find_on_map(&ctx, what);
    assert_eq!(h.state().finding.as_ref().unwrap().2, 0, "round again");
    // A layer painted nowhere: said.
    h.state_mut().find_on_map(&ctx, Match::Layer(Painted::Cover, 3));
    assert!(h.state().status.as_ref().unwrap().text.contains("not painted anywhere"));
}

// ----------------------------------------------------------------- Pencil --

/// The Pencil's path: every block beside the last by a side, from one end
/// to the other, at any slope.
#[test]
fn the_pencil_path_is_joined_up() {
    for to in [(0, 0), (7, 0), (0, -5), (9, 4), (-3, 11), (-13, -13), (25, -2)] {
        let path = crate::tools::pencil_path((0, 0), to);
        assert_eq!(path[0], (0, 0));
        assert_eq!(*path.last().unwrap(), to);
        assert_eq!(path.len() as i32, to.0.abs() + to.1.abs() + 1, "as few blocks as can be");
        for w in path.windows(2) {
            assert_eq!((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs(), 1, "{to:?}: {w:?} joined by a side");
        }
    }
}

/// The Pencil draws an unbroken line one block wide however far the
/// mouse jumped between two frames, at full strength.
#[test]
fn the_pencil_draws_without_gaps() {
    let mut doc = crate::doc::Document::new(painted_world(), None, 1);
    let mut tools = crate::tools::ToolState::default();
    tools.select(Tool::PaintTerrain);
    let sand = doc.world.registry.terrain_by_name("Sand").unwrap();
    tools.terrain = sand.0;
    tools.brush.strength = 0.2;
    tools.pencil = true;
    let (stroke, _) = crate::tools::begin_stroke(&mut doc, &tools, (500.5, 500.5), false, false);
    let mut stroke = stroke.unwrap();
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (500.5, 500.5), 0.02);
    // Two jumps of 40 blocks: two frames of a fast mouse.
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (540.2, 517.9), 0.02);
    crate::tools::continue_stroke(&mut doc, &tools, &mut stroke, (560.7, 550.3), 0.02);
    let _ = crate::tools::end_stroke(&mut doc, stroke);
    let dim = doc.dimension();
    let painted: std::collections::HashSet<(i32, i32)> = (480..580).flat_map(|z| (480..580).map(move |x| (x, z))).filter(|&(x, z)| terrain_at(dim, x, z) == Some(sand.0)).collect();
    let path: Vec<(i32, i32)> = crate::tools::pencil_path((500, 500), (540, 517)).into_iter().chain(crate::tools::pencil_path((540, 517), (560, 550)).into_iter().skip(1)).collect();
    for p in &path {
        assert!(painted.contains(p), "{p:?} painted, at full strength");
    }
    assert_eq!(painted.len(), path.len(), "one block wide, nothing else");
}

/// The Pencil on the map: lines drawn with fast moves of the mouse, one
/// and three blocks wide (rendered at 1× and 2×).
#[test]
fn pencil_lines_on_the_map() {
    for ppp in densities() {
        let mut h = app(ppp, Tool::PaintTerrain);
        let sand = h.state().doc.world.registry.terrain_by_name("Sand").unwrap().0;
        h.state_mut().tools.terrain = sand;
        h.state_mut().tools.pencil = true;
        look(&mut h, 520.0, 520.0, 8.0);
        for (width, z) in [(1u16, 510.0), (3, 530.0)] {
            h.state_mut().tools.pencil_width = width;
            let (a, b) = (map(&h).screen_of(500.5, z), map(&h).screen_of(545.5, z + 9.0));
            h.event(egui::Event::PointerMoved(a));
            h.step();
            press(&mut h, a, true, Modifiers::NONE);
            // Three frames for 45 blocks: fifteen blocks a frame.
            for k in 1..=3 {
                h.event(egui::Event::PointerMoved(a + (b - a) * (k as f32 / 3.0)));
                h.step();
            }
            press(&mut h, b, false, Modifiers::NONE);
            h.run_steps(3);
        }
        let dim = h.state().doc.dimension();
        for x in 501..545 {
            assert!((509..=520).any(|z| terrain_at(dim, x, z) == Some(sand)), "column {x} of the thin line");
        }
        h.event(egui::Event::PointerGone);
        h.run_steps(8);
        save(&mut h, "pencil-lines", ppp);
    }
}

// --------------------------------------------------------- Alt and wheel --

/// Alt and the wheel turn the brush, as in WorldPainter, rather than zoom.
#[test]
fn alt_and_the_wheel_turn_the_brush() {
    let mut h = app(1.0, Tool::Raise);
    look(&mut h, 400.0, 400.0, 2.0);
    let p = map(&h).screen_of(400.0, 400.0);
    h.event(egui::Event::PointerMoved(p));
    h.step();
    let (a0, z0) = (h.state().tools.brush.rotation, map(&h).zoom);
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, 80.0), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: Modifiers::ALT });
    h.run_steps(10);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.state().tools.brush.rotation, (a0 + 20.0).rem_euclid(360.0));
    assert_eq!(map(&h).zoom, z0, "no zoom");
}

// ------------------------------------------------------------ region grid --

#[test]
fn regions_are_named_as_the_game_names_their_files() {
    assert_eq!(crate::map_view::region_name(0, 0), "r.0.0.mca");
    assert_eq!(crate::map_view::region_name(-1, 2), "r.-1.2.mca");
    assert_eq!((-1i32).div_euclid(crate::map_view::REGION_SIZE), -1);
    assert_eq!(511i32.div_euclid(crate::map_view::REGION_SIZE), 0);
}

/// The grid with the region files' borders and names, near and far
/// (rendered at 1× and 2×).
#[test]
fn the_grid_shows_the_region_files() {
    use egui_kittest::kittest::Queryable;
    for ppp in densities() {
        let mut h = app(ppp, Tool::Pan);
        h.state_mut().maps.values_mut().next().unwrap().grid = true;
        look(&mut h, 512.0, 512.0, 0.6);
        let p = map(&h).screen_of(700.0, 300.0);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(8);
        assert!(h.query_by_label_contains("r.1.0.mca").is_some(), "the footer names the region");
        save(&mut h, "region-grid", ppp);
        look(&mut h, 512.0, 512.0, 2.0);
        h.run_steps(8);
        save(&mut h, "region-grid-near", ppp);
    }
}

// ------------------------------------------------- Test Export by regions --

/// A press takes a region, or leaves one taken; a drag goes on the way the
/// press went.
#[test]
fn regions_are_picked_by_click_and_drag() {
    let mut pick = crate::tools::RegionPick::default();
    pick.press(10, 10);
    pick.drag = None;
    assert!(pick.regions.contains(&(0, 0)));
    pick.press(-1, 600);
    for x in [100, 600, 1100] {
        pick.press(x, 600);
    }
    pick.drag = None;
    assert_eq!(pick.regions.iter().copied().collect::<Vec<_>>(), [(-1, 1), (0, 0), (0, 1), (1, 1), (2, 1)]);
    // Pressed on a region taken: the drag leaves them.
    pick.press(700, 700);
    pick.press(1100, 700);
    pick.drag = None;
    assert_eq!(pick.regions.iter().copied().collect::<Vec<_>>(), [(-1, 1), (0, 0), (0, 1)]);
    assert_eq!(super::regions_of(massif_core::BlockRect::new(-10, 0, 520, 100)).into_iter().collect::<Vec<_>>(), [(-1, 0), (0, 0), (1, 0)]);
}

/// Test Export ▸ Regions: the dialog makes way for the map, whole regions
/// are picked on it with a click and a drag, Done comes back to the
/// dialog, which lists the region files; the test world has those files
/// and no others. Snap to regions rounds the view out to them (rendered at
/// 1× and 2×).
#[test]
fn test_export_writes_the_regions_picked() {
    use egui_kittest::kittest::Queryable;
    for ppp in densities() {
        let out = std::env::temp_dir().join(format!("massif-region-test-{}-{ppp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let mut h = app(ppp, Tool::Raise);
        look(&mut h, 0.0, 0.0, 0.35);
        h.state_mut().open_export_dialog();
        if let Some(Dialog::Export(form)) = &mut h.state_mut().dialog {
            form.parent = out.clone();
            form.name = "Regions".into();
            form.testing = true;
            form.test_area = TestArea::Regions;
        }
        h.run_steps(12);
        assert!(h.query_all_by_label_contains("No region picked yet").next().is_some());
        h.query_all_by_label_contains("Pick on the map…").next().expect("the button").click();
        h.run_steps(4);
        assert!(h.state().dialog.is_none(), "the map is free to click");
        assert!(h.state().tools.region_pick.is_some());
        // A click on r.0.0, then a drag over r.-1.-1 and r.0.-1.
        let revision = h.state().doc.revision;
        let p = map(&h).screen_of(200.0, 200.0);
        super::tests::click_on(&mut h, p, PointerButton::Primary);
        let (a, b) = (map(&h).screen_of(-300.0, -300.0), map(&h).screen_of(300.0, -300.0));
        super::tests::drag_on(&mut h, a, b);
        assert_eq!(h.state().doc.revision, revision, "the tool did nothing");
        assert_eq!(h.state().tools.region_pick.as_ref().unwrap().regions.iter().copied().collect::<Vec<_>>(), [(-1, -1), (0, -1), (0, 0)]);
        h.event(egui::Event::PointerMoved(map(&h).screen_of(-200.0, 300.0)));
        h.run_steps(6);
        assert!(h.query_by_label_contains("3 regions picked").is_some());
        save(&mut h, "test-export-pick-regions", ppp);
        h.get_by_label("Done").click();
        h.run_steps(12);
        let Some(Dialog::Export(form)) = &h.state().dialog else { panic!("back to the dialog") };
        assert_eq!(form.test_area, TestArea::Regions);
        let region_dir = form.target.region_dir();
        assert!(h.query_all_by_label_contains("r.-1.-1.mca  r.0.-1.mca  r.0.0.mca").next().is_some(), "the files listed");
        assert!(h.query_all_by_label_contains("3 whole region files").next().is_some());
        save(&mut h, "test-export-regions", ppp);
        // Snap to regions, around the view: the 3 × 3 tiles round out to
        // the four regions they reach.
        if let Some(Dialog::Export(form)) = &mut h.state_mut().dialog {
            form.test_area = TestArea::View;
            form.test_centre = (0, 0);
            form.snap_regions = true;
            assert_eq!(form.test_region_files().into_iter().collect::<Vec<_>>(), [(-1, -1), (-1, 0), (0, -1), (0, 0)]);
            form.snap_regions = false;
            form.test_area = TestArea::Regions;
        }
        h.run_steps(4);
        h.get_by_label("Export the test").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            h.run_steps(1);
        }
        let world = out.join("Regions test");
        let mut files: Vec<String> = std::fs::read_dir(world.join(region_dir)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".mca")).collect();
        files.sort();
        assert_eq!(files, ["r.-1.-1.mca", "r.0.-1.mca", "r.0.0.mca"], "those picked and no others");
        let _ = std::fs::remove_dir_all(&out);
    }
}
