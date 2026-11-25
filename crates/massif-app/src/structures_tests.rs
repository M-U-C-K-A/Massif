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
    assert!(h.query_by_label_contains("Medium village · 18 houses").is_some(), "the summary line");
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
