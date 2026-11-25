//! The structures and villages as a user places them: the tool picked in
//! the toolbar, an area dragged on the map, a click, a village, one step to
//! undo.

use super::tests::{click_on, harness_with};
use super::*;
use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn map<'a>(h: &'a Harness<'_, MassifApp>) -> &'a crate::map_view::MapView {
    h.state().maps.values().next().expect("a map")
}

fn look(h: &mut Harness<'_, MassifApp>, x: f64, z: f64, zoom: f32) {
    let m = h.state_mut().maps.values_mut().next().unwrap();
    m.zoom = zoom;
    m.center = [x, z];
    h.run_steps(3);
}

fn drag(h: &mut Harness<'_, MassifApp>, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.step();
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for k in 1..=12 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 12.0)));
        h.step();
    }
    h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// Land with room round it, near the map's middle.
fn dry_spot(h: &Harness<'_, MassifApp>) -> (i32, i32) {
    let dim = h.state().doc.dimension();
    let sea = h.state().doc.world.meta.sea_level as f32;
    let height = |x: i32, z: i32| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap_or(0.0);
    let mut best = ((0, 0), 0);
    for z in (-300..300).step_by(24) {
        for x in (-300..300).step_by(24) {
            let dry = (-96..=96).step_by(24).flat_map(|dz| (-96..=96).step_by(24).map(move |dx| (dx, dz))).filter(|&(dx, dz)| height(x + dx, z + dz) > sea + 2.0).count();
            if dry > best.1 {
                best = ((x, z), dry);
            }
        }
    }
    best.0
}

/// Step until no job is running (at most `secs`).
fn settle(h: &mut Harness<'_, MassifApp>, secs: u64) {
    let started = std::time::Instant::now();
    loop {
        h.step();
        let busy = !h.state().jobs.is_empty() || h.state().tools.village.rt.is_busy() || h.state().tools.structure.rt.is_busy() || h.state().tools.structure.rt.regen_waiting();
        if !busy || started.elapsed().as_secs() > secs {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(3);
}

/// The Structures tool as a user drives it, with the game's own villages
/// (a tiny one made by hand, as the tests have no game): its button in
/// the toolbar, a village picked in the list, a click on the map — a
/// village, no selection needed, one step to undo —; then a rectangle
/// dragged; the village clicked is selected, dragged elsewhere, its corner
/// dragged, its houses and roads changed, taken away.
#[test]
fn the_structures_tool_as_a_user_drives_it() {
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let save = |h: &mut Harness<'_, MassifApp>, name: &str| {
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join(format!("structures-{name}-{ppp}x.png"))).unwrap();
        }
    };
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(5);
    // The real game for the renders when it is installed, else a tiny
    // village made by hand.
    let real = dir.as_ref().and_then(|_| massif_assets::find_jars().into_iter().next()).and_then(|j| massif_structures::Library::open(&j.path, &j.version, &[]).ok());
    h.state_mut().tools.structure.rt.set_library(std::sync::Arc::new(real.unwrap_or_else(massif_structures::testing::village)));
    if let Some(Ok(cat)) = dir.as_ref().and_then(|_| massif_assets::load_newest()) {
        h.state_mut().blocks.catalogue = Some(std::sync::Arc::new(cat));
    }
    // Land with room round it, near the middle.
    let spot = {
        let dim = h.state().doc.dimension();
        let sea = h.state().doc.world.meta.sea_level as f32;
        let height = |x: i32, z: i32| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap_or(0.0);
        let mut best = ((0, 0), 0);
        for z in (-300..300).step_by(24) {
            for x in (-300..300).step_by(24) {
                let dry = (-96..=96).step_by(24).flat_map(|dz| (-96..=96).step_by(24).map(move |dx| (dx, dz))).filter(|&(dx, dz)| height(x + dx, z + dz) > sea + 2.0).count();
                if dry > best.1 {
                    best = ((x, z), dry);
                }
            }
        }
        best.0
    };
    look(&mut h, spot.0 as f64, spot.1 as f64, 1.6);
    h.get_by_label("Structures").click();
    h.run_steps(3);
    assert_eq!(h.state().tools.tool, Tool::Structure);
    h.get_by_label("Plains").click();
    h.run_steps(3);
    assert!(h.state().tools.structure.village);
    save(&mut h, "panel");
    // A click: a village of the size chosen, round the click.
    let p = map(&h).screen_of(spot.0 as f64, spot.1 as f64);
    click_on(&mut h, p, PointerButton::Primary);
    settle(&mut h, 120);
    let placed = h.state().doc.dimension().structures.clone();
    assert_eq!(placed.len(), 1, "a village: {:?}", h.state().tools.village.rt.notes());
    let first = placed[0].id;
    assert_eq!(h.state().tools.structure.selected, Some(first), "the village placed is selected");
    let recipe = crate::structure_tool::recipe_of(&placed[0]).expect("its recipe kept");
    assert_eq!(recipe.area().unwrap().x1 - recipe.area().unwrap().x0, h.state().tools.village.size.side());
    assert!(!placed[0].ground.is_empty(), "the land it levelled kept");
    h.run_steps(5);
    save(&mut h, "placed");
    // Dragged elsewhere: built again there, in one step to undo.
    let from = map(&h).screen_of(spot.0 as f64, spot.1 as f64);
    let to = map(&h).screen_of(spot.0 as f64 + 40.0, spot.1 as f64);
    drag(&mut h, from, to);
    settle(&mut h, 120);
    let s = h.state().doc.dimension().structures.clone();
    assert_eq!(s.len(), 1, "moved, not copied");
    let moved = crate::structure_tool::recipe_of(&s[0]).unwrap();
    assert_eq!(moved.area().unwrap().x0, recipe.area().unwrap().x0 + 40);
    // Its south-east corner dragged out: larger.
    let area = moved.area().unwrap();
    let corner = map(&h).screen_of(area.x1 as f64, area.z1 as f64);
    let out = map(&h).screen_of(area.x1 as f64 + 48.0, area.z1 as f64 + 32.0);
    drag(&mut h, corner, out);
    settle(&mut h, 120);
    let s = h.state().doc.dimension().structures.clone();
    let grown = crate::structure_tool::recipe_of(&s[0]).unwrap().area().unwrap();
    assert_eq!((grown.x1 - grown.x0, grown.z1 - grown.z0), (area.x1 - area.x0 + 48, area.z1 - area.z0 + 32));
    // At the same density, its houses follow its land.
    let massif_structures::placed::Recipe::Village { params, .. } = crate::structure_tool::recipe_of(&s[0]).unwrap() else { panic!("a village") };
    assert_eq!(params.houses, Some(massif_structures::village::houses_for(grown, params.density)));
    assert!(params.houses.unwrap() > massif_structures::village::houses_for(area, params.density), "more houses over more land");
    save(&mut h, "resized");
    // Fewer houses and stone roads, in the panel: built again once they
    // rest.
    let id = s[0].id;
    let r = crate::structure_tool::recipe_of(&s[0]).unwrap();
    let massif_structures::placed::Recipe::Village { params, .. } = &r else { panic!("a village") };
    assert_eq!(params.road, massif_structures::village::RoadType::Dirt);
    // The panel scrolled to the village's roads, as a user does.
    h.query_all_by_label("Stone").next().expect("the selected village's roads").scroll_to_me();
    h.run_steps(3);
    h.query_all_by_label("Stone").next().expect("the selected village's roads").click();
    h.run_steps(2);
    // Built again once the settings rest (on a loaded machine, maybe
    // already under way).
    assert!(h.state().tools.structure.rt.regen_waiting() || h.state().tools.village.rt.is_busy() || !h.state().jobs.is_empty() || h.state().doc.dimension().structures[0].id != id, "built again once the settings rest");
    settle(&mut h, 120);
    let s = h.state().doc.dimension().structures.clone();
    let massif_structures::placed::Recipe::Village { params, .. } = crate::structure_tool::recipe_of(&s[0]).unwrap() else { panic!("a village") };
    assert_eq!(params.road, massif_structures::village::RoadType::Stone);
    assert_ne!(s[0].id, id, "built again");
    h.run_steps(5);
    save(&mut h, "stone-roads");
    // Undone: back to dirt roads.
    h.state_mut().doc.undo();
    h.run_steps(2);
    let massif_structures::placed::Recipe::Village { params, .. } = crate::structure_tool::recipe_of(&h.state().doc.dimension().structures[0]).unwrap() else { panic!("a village") };
    assert_eq!(params.road, massif_structures::village::RoadType::Dirt);
    // Another theme picked in the list while it is selected (a pack's,
    // which needs no game files): built again of that theme, without
    // drawing it again.
    let packs = std::env::temp_dir().join(format!("massif-drive-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&packs);
    let pack = massif_structures::village::testpack::write_pack(&packs).unwrap();
    h.state_mut().tools.village.packs = vec![pack];
    let id = h.state().doc.dimension().structures[0].id;
    h.state_mut().tools.structure.selected = Some(id);
    h.run_steps(5);
    h.get_by_label("Plains (test boxes) · Plains").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Plains (test boxes) · Plains").click();
    h.run_steps(2);
    settle(&mut h, 120);
    let s = h.state().doc.dimension().structures.clone();
    let massif_structures::placed::Recipe::Village { theme, .. } = crate::structure_tool::recipe_of(&s[0]).unwrap() else { panic!("a village") };
    let massif_structures::placed::Recipe::Village { pack, .. } = crate::structure_tool::recipe_of(&s[0]).unwrap() else { panic!("a village") };
    assert_eq!((pack.as_str(), theme.as_str()), ("Plains (test boxes)", "Plains"));
    assert_ne!(s[0].id, id, "built again");
    h.run_steps(5);
    save(&mut h, "other-theme");
    h.state_mut().doc.undo();
    h.state_mut().tools.village.packs.clear();
    h.state_mut().tools.village.theme = (String::new(), "Plains".into());
    let _ = std::fs::remove_dir_all(&packs);
    h.run_steps(2);
    // A rectangle dragged on free land: a second village over it.
    let a = map(&h).screen_of(spot.0 as f64 - 260.0, spot.1 as f64 - 80.0);
    let b = map(&h).screen_of(spot.0 as f64 - 120.0, spot.1 as f64 + 40.0);
    drag(&mut h, a, b);
    settle(&mut h, 120);
    assert_eq!(h.state().doc.dimension().structures.len(), 2, "a second village: {:?}", h.state().tools.village.rt.notes());
    h.run_steps(5);
    save(&mut h, "two-villages");
    // Taken away with Delete over the map, put back with undo.
    let hover = map(&h).screen_of(spot.0 as f64 - 190.0, spot.1 as f64 - 20.0);
    h.event(egui::Event::PointerMoved(hover));
    h.step();
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert_eq!(h.state().doc.dimension().structures.len(), 1, "Delete takes the selected one away");
    h.state_mut().doc.undo();
    h.run_steps(2);
    assert_eq!(h.state().doc.dimension().structures.len(), 2);
    let last = h.state().doc.dimension().structures.last().unwrap().id;
    h.state_mut().tools.structure.selected = Some(last);
    h.run_steps(2);
    // Taken away with its button: the land it levelled given back.
    let ground = h.state().doc.dimension().structures.last().unwrap().ground.clone();
    // The panel scrolled back to the selected one's buttons.
    h.get_by_label_contains("Take it away").scroll_to_me();
    h.run_steps(3);
    h.get_by_label_contains("Take it away").click();
    h.run_steps(3);
    assert_eq!(h.state().doc.dimension().structures.len(), 1);
    let dim = h.state().doc.dimension();
    for &(x, z, before) in ground.iter().step_by(37) {
        let now = massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap();
        assert!((now - before).abs() < 0.01, "({x}, {z}): {now} for {before}");
    }
}

/// A pack's village in the panel: what it will build in a line, each type
/// with its count, the pack's buildings, the land it needs; placed, the
/// same of what it built, and a count changed builds it again.
#[test]
fn the_village_panel_says_what_a_pack_builds() {
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let packs = std::env::temp_dir().join(format!("massif-panel-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&packs);
    let pack = massif_structures::village::testpack::write_pack(&packs).unwrap();
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(5);
    {
        let s = h.state_mut();
        s.tools.select(Tool::Structure);
        s.tools.village.packs = vec![pack.clone()];
        s.tools.village.theme = ("Plains (test boxes)".into(), "Plains".into());
        s.tools.structure.village = true;
    }
    h.run_steps(5);
    assert!(h.query_by_label_contains("Medium village · 12 houses").is_some(), "the summary line");
    assert!(h.query_by_label("churches").is_some() && h.query_by_label("farms").is_some(), "each type with its count");
    assert!(h.query_by_label_contains("It needs about").is_some(), "the land it needs");
    // Scrolled down, as a user reads it: its summary and counts, then the
    // pack's buildings and the land it needs.
    let scroll = |h: &mut Harness<'_, MassifApp>, ticks: usize| {
        h.event(egui::Event::PointerMoved(egui::pos2(140.0, 560.0)));
        for _ in 0..ticks {
            h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -60.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
            h.step();
        }
        h.run_steps(4);
    };
    scroll(&mut h, 8);
    if let Some(dir) = &dir {
        h.render().unwrap().save(dir.join(format!("structures-pack-panel-{ppp}x.png"))).unwrap();
    }
    scroll(&mut h, 5);
    if let Some(dir) = &dir {
        h.render().unwrap().save(dir.join(format!("structures-pack-panel-more-{ppp}x.png"))).unwrap();
    }
    let spot = dry_spot(&h);
    look(&mut h, spot.0 as f64, spot.1 as f64, 1.6);
    let p = map(&h).screen_of(spot.0 as f64, spot.1 as f64);
    click_on(&mut h, p, PointerButton::Primary);
    settle(&mut h, 120);
    assert_eq!(h.state().doc.dimension().structures.len(), 1);
    h.run_steps(4);
    assert!(h.query_by_label_contains("Its land:").is_some(), "how its land suits it");
    if let Some(dir) = &dir {
        h.render().unwrap().save(dir.join(format!("structures-pack-selected-{ppp}x.png"))).unwrap();
    }
    let _ = std::fs::remove_dir_all(&packs);
}

/// The Tools panel: every row's tools start at the same place, the names
/// of the rows in a column as wide as the widest (in English and in French,
/// whose names are longer), at a width where the names show, rendered with
/// MASSIF_SNAPSHOT_DIR.
#[test]
fn the_tools_rows_line_up() {
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    for lang in [crate::i18n::Language::English, crate::i18n::Language::French] {
        crate::i18n::set_language(lang);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(380.0, 330.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui_state(
            |ui, tools: &mut crate::tools::ToolState| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    crate::theme::install(ui.ctx());
                }
                ui.ctx().set_theme(egui::Theme::Dark);
                egui::Frame::new().fill(crate::theme::Palette::of(ui).panel).inner_margin(8).show(ui, |ui| {
                    ui.set_width(364.0);
                    crate::panels::tools_panel(ui, tools);
                });
            },
            crate::tools::ToolState::default(),
        );
        h.run_steps(4);
        // The middle of the first tool of each row (the one chosen stands
        // out a little), past the names (not their icons alone).
        let mut lefts = Vec::new();
        for (_, _, list) in Tool::GROUPS {
            let name = crate::i18n::tr_name(list[0].label());
            if let Some(n) = h.query_all_by_label(name).next() {
                lefts.push(n.rect().center().x);
            }
        }
        assert_eq!(lefts.len(), Tool::GROUPS.len(), "{lefts:?}");
        let (lo, hi) = lefts.iter().fold((f32::MAX, f32::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
        assert!(hi - lo < 0.5, "the rows' first tools at {lefts:?}");
        assert!(lo > 85.0, "the names shown before the tools: {lefts:?}");
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join(format!("tools-panel-{}-{ppp}x.png", lang.code()))).unwrap();
        }
    }
    crate::i18n::set_language(crate::i18n::Language::English);
}

/// The village's walls: a menu of what runs round it, each with a picture,
/// rendered with MASSIF_SNAPSHOT_DIR; picking one sets it for the next
/// village.
#[test]
fn the_walls_menu_shows_each_enclosure() {
    use massif_structures::village::walls::Enclosure;
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(5);
    {
        let s = h.state_mut();
        s.tools.select(Tool::Structure);
        s.tools.structure.village = true;
        s.tools.village.walls = true;
    }
    h.run_steps(5);
    h.get_by_value("Wooden fence").scroll_to_me();
    h.run_steps(3);
    h.get_by_value("Wooden fence").click();
    h.run_steps(4);
    for e in Enclosure::ALL {
        assert!(h.query_all_by_label(e.name()).next().is_some(), "{e:?} in the menu");
    }
    if let Some(dir) = &dir {
        h.render().unwrap().save(dir.join(format!("structures-walls-menu-{ppp}x.png"))).unwrap();
    }
    h.query_all_by_label("Hedge").last().expect("the hedge").click();
    h.run_steps(4);
    assert_eq!(h.state().tools.village.enclosure, Enclosure::Hedge);
    assert!(h.state().tools.village.walls);
    let massif_structures::placed::Recipe::Village { params, .. } = h.state().tools.village.recipe(massif_core::BlockRect::new(0, 0, 100, 100)) else { panic!("a village") };
    assert!(params.walls && params.enclosure == Enclosure::Hedge);
}

/// The Building tool as a user drives it, on a village of a pack: a
/// building clicked is selected; R turns it, a drag moves it, another of
/// the pack takes its place from the menu of pictures, Delete takes it
/// away, each as one step to undo; the village keeps the changes. Rendered
/// with MASSIF_SNAPSHOT_DIR.
#[test]
fn the_building_tool_changes_one_building() {
    use massif_structures::placed::Recipe;
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let save = |h: &mut Harness<'_, MassifApp>, name: &str| {
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join(format!("building-{name}-{ppp}x.png"))).unwrap();
        }
    };
    let packs = std::env::temp_dir().join(format!("massif-building-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&packs);
    let pack = massif_structures::village::testpack::write_pack(&packs).unwrap();
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(5);
    {
        let s = h.state_mut();
        s.tools.select(Tool::Structure);
        s.tools.village.packs = vec![pack.clone()];
        s.tools.village.theme = ("Plains (test boxes)".into(), "Plains".into());
        s.tools.structure.village = true;
    }
    let spot = dry_spot(&h);
    look(&mut h, spot.0 as f64, spot.1 as f64, 1.6);
    let p = map(&h).screen_of(spot.0 as f64, spot.1 as f64);
    click_on(&mut h, p, PointerButton::Primary);
    settle(&mut h, 120);
    assert_eq!(h.state().doc.dimension().structures.len(), 1);
    // The Building tool, from the Tools panel.
    h.get_by_label("Building").click();
    h.run_steps(3);
    assert_eq!(h.state().tools.tool, Tool::Building);
    // A house of the village, clicked: selected.
    let house = |h: &Harness<'_, MassifApp>| h.state().doc.dimension().structures[0].pieces.iter().find(|p| p.template == "house" && p.key.is_some()).cloned().expect("a house");
    let first = house(&h);
    let key = first.key.unwrap();
    let middle = |p: &massif_core::structures::Piece| ((p.min[0] + p.max[0] + 1) as f64 / 2.0, (p.min[2] + p.max[2] + 1) as f64 / 2.0);
    let (mx, mz) = middle(&first);
    look(&mut h, mx, mz, 6.0);
    let at = map(&h).screen_of(mx, mz);
    click_on(&mut h, at, PointerButton::Primary);
    h.run_steps(4);
    assert_eq!(h.state().tools.building.selected, Some(key), "the house clicked is selected");
    save(&mut h, "selected");
    let edits = |h: &Harness<'_, MassifApp>| match crate::structure_tool::recipe_of(&h.state().doc.dimension().structures[0]) {
        Some(Recipe::Village { params, .. }) => params.edits,
        _ => Vec::new(),
    };
    let piece = |h: &Harness<'_, MassifApp>| crate::building_tool::find(&h.state().doc, key).map(|(_, p)| p.clone());
    // R over the map: turned a quarter, about its middle.
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.key_press(egui::Key::R);
    h.run_steps(2);
    settle(&mut h, 120);
    assert_eq!(edits(&h).first().map(|e| e.turn), Some(1), "turned");
    let turned = piece(&h).expect("still there");
    assert_ne!(turned.turn, first.turn);
    let undo_steps = h.state().doc.history.undo_count();
    // Dragged 12 blocks east: moved there.
    let (tx, tz) = middle(&turned);
    let from = map(&h).screen_of(tx, tz);
    let to = map(&h).screen_of(tx + 12.0, tz);
    drag(&mut h, from, to);
    settle(&mut h, 120);
    assert_eq!(edits(&h).first().map(|e| e.offset), Some([12, 0]), "moved");
    assert_eq!(h.state().doc.history.undo_count(), undo_steps + 1, "one step to undo");
    let moved = piece(&h).expect("still there");
    assert!((moved.min[0] - turned.min[0] - 12).abs() <= 1, "{:?} from {:?}", moved.min, turned.min);
    save(&mut h, "moved");
    // Another house of the pack, from the menu of pictures.
    h.get_by_label("Another building").scroll_to_me();
    h.run_steps(3);
    let menu = || egui_kittest::kittest::by().role(egui::accesskit::Role::ComboBox);
    let shown = h.query_all(menu()).filter_map(|n| n.value()).find(|v| v.ends_with(".nbt")).expect("the menu of the pack's buildings");
    h.query_all(menu()).find(|n| n.value().as_deref() == Some(shown.as_str())).unwrap().click();
    h.run_steps(4);
    save(&mut h, "swap-menu");
    let theme = h.state().tools.village.theme_named("Plains (test boxes)", "Plains").expect("the pack's theme");
    let other = theme.kind("house").unwrap().variants.iter().map(massif_structures::village::edits::file_name).find(|n| *n != shown).expect("another house");
    h.get_by_label(&other).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(&other).click();
    h.run_steps(2);
    settle(&mut h, 120);
    assert_eq!(edits(&h).first().and_then(|e| e.swap.clone()).map(|s| s.1), Some(other.clone()), "swapped");
    save(&mut h, "swapped");
    // Undone: the house it was.
    h.state_mut().doc.undo();
    h.run_steps(2);
    assert_eq!(edits(&h).first().and_then(|e| e.swap.clone()), None);
    // Delete: taken away; the village keeps the others.
    let before = h.state().doc.dimension().structures[0].pieces.iter().filter(|p| p.key.is_some()).count();
    h.state_mut().tools.building.selected = Some(key);
    h.run_steps(2);
    let at = map(&h).screen_of(middle(&piece(&h).unwrap()).0, middle(&piece(&h).unwrap()).1);
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    settle(&mut h, 120);
    assert!(piece(&h).is_none(), "taken away");
    assert_eq!(h.state().doc.dimension().structures[0].pieces.iter().filter(|p| p.key.is_some()).count(), before - 1);
    let _ = std::fs::remove_dir_all(&packs);
}

/// A village of a pack with walls, selected on the map: a frame costs
/// little, and with nothing moving the app stops drawing.
#[test]
fn a_selected_village_keeps_the_frame_light() {
    let packs = std::env::temp_dir().join(format!("massif-frame-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&packs);
    let pack = massif_structures::village::testpack::write_pack(&packs).unwrap();
    let mut h = harness_with(ThemeChoice::Dark, 1.0);
    h.run_steps(5);
    {
        let s = h.state_mut();
        s.tools.select(Tool::Structure);
        s.tools.village.packs = vec![pack.clone()];
        s.tools.village.theme = ("Plains (test boxes)".into(), "Plains".into());
        s.tools.village.walls = true;
        s.tools.village.size = crate::structure_tool::VillageSize::Large;
        s.tools.structure.village = true;
    }
    let spot = dry_spot(&h);
    look(&mut h, spot.0 as f64, spot.1 as f64, 1.0);
    let p = map(&h).screen_of(spot.0 as f64, spot.1 as f64);
    click_on(&mut h, p, PointerButton::Primary);
    settle(&mut h, 120);
    assert_eq!(h.state().doc.dimension().structures.len(), 1);
    assert!(h.state().tools.structure.selected.is_some(), "the village placed is selected");
    h.run_steps(10);
    // The pointer over the map, still: the frames as they come.
    h.event(egui::Event::PointerMoved(p + egui::vec2(30.0, 20.0)));
    let frames = std::env::var("MASSIF_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(30usize);
    let mut times: Vec<f64> = (0..frames)
        .map(|_| {
            let t = std::time::Instant::now();
            h.step();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!("selected village: frame best {:.2} ms, median {:.2} ms, worst {:.2} ms", times[0], times[frames / 2], times[frames - 1]);
    assert!(times[frames / 2] < 40.0, "a frame with the village selected: {:.1} ms", times[frames / 2]);
    // Nothing moving: the frames stop.
    assert!(h.try_run().is_ok(), "the app keeps drawing with nothing moving");
    if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) {
        h.render().unwrap().save(dir.join("structures-walled-village.png")).unwrap();
        let s = h.state().doc.dimension().structures[0].clone();
        look(&mut h, (s.min[0] + s.max[0]) as f64 / 2.0, (s.min[2] + s.max[2]) as f64 / 2.0, 3.0);
        h.run_steps(10);
        h.render().unwrap().save(dir.join("structures-walled-village-close.png")).unwrap();
    }
    let _ = std::fs::remove_dir_all(&packs);
}
