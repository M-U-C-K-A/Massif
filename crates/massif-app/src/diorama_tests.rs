//! The Diorama area driven as a user drives it, on a map that looks like
//! one: hills by the sea, woods of oaks, birches and spruces, and a village
//! of houses along paths. The area opens in place of the map, builds the
//! tiles, pans, zooms, turns, and shows the caves under a height. With the
//! game installed it draws the game's textures; with MASSIF_SNAPSHOT_DIR
//! (and MASSIF_SNAPSHOT_PPP=2) it saves the views, and MASSIF_PERF prints
//! how long the tiles took to build and a frame to draw.

use super::tests::harness_with;
use super::*;
use egui_kittest::Harness;
use std::sync::Arc;

/// The app on the village map, its map area showing the Diorama, with the
/// game's textures when the game is installed here.
fn diorama(ppp: f32) -> (Harness<'static, MassifApp>, u64) {
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(3);
    {
        let app = h.state_mut();
        app.doc = crate::doc::Document::new(massif_render::diorama::sample::village(), None, 9);
        app.map_cache.invalidate_all();
        app.view3d.invalidate_all();
        if let Some(Ok(cat)) = massif_assets::load_newest() {
            app.blocks.catalogue = Some(Arc::new(cat));
        } else {
            eprintln!("Minecraft is not installed here: flat colours");
        }
        let id = *app.maps.keys().next().expect("a map area");
        app.layout.current_mut().root.set_editor(id, EditorKind::Diorama);
    }
    h.run_steps(3);
    let id = *h.state().maps.keys().next().unwrap();
    (h, id)
}

/// Step until every tile is built and drawn (at most two minutes).
fn settle(h: &mut Harness<'_, MassifApp>) {
    let started = std::time::Instant::now();
    while !h.state().diorama.settled() && started.elapsed().as_secs() < 120 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    h.run_steps(4);
    assert!(h.state().diorama.settled(), "the Diorama built the map: {:?}", h.state().diorama.progress());
}

fn save(h: &mut Harness<'_, MassifApp>, name: &str) {
    if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) {
        let ppp = h.ctx.pixels_per_point();
        h.render().unwrap().save(dir.join(format!("diorama-{name}-{ppp}x.png"))).unwrap();
    }
}

#[test]
fn the_diorama_opens_builds_and_moves() {
    let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let (mut h, _) = diorama(ppp);
    assert!(h.state().layout.current().root.shows(EditorKind::Diorama));
    settle(&mut h);
    let p = h.state().diorama.progress();
    if std::env::var_os("MASSIF_PERF").is_some() {
        eprintln!("diorama: {} tiles, first after {:?}, whole after {:?}, {:.1} ms of a thread a tile", p.tiles, p.first, p.whole, p.per_tile * 1000.0);
    }
    // The village up close, then halfway, then the whole map.
    let d = &mut h.state_mut().diorama;
    d.center = Some([massif_render::diorama::sample::VILLAGE.0 as f64, massif_render::diorama::sample::VILLAGE.1 as f64]);
    d.zoom = 6.0;
    h.run_steps(6);
    settle(&mut h);
    save(&mut h, "village");
    h.state_mut().diorama.zoom = 1.5;
    h.run_steps(6);
    settle(&mut h);
    save(&mut h, "halfway");
    // Pan: a drag moves the map with the pointer.
    let before = h.state().diorama.center.unwrap();
    let view = h.state().maps.keys().next().copied();
    assert!(view.is_some());
    let mid = egui::pos2(800.0, 500.0);
    h.event(egui::Event::PointerMoved(mid));
    h.event(egui::Event::PointerButton { pos: mid, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(mid + egui::vec2(k as f32 * 6.0, 0.0)));
        h.step();
    }
    h.event(egui::Event::PointerButton { pos: mid + egui::vec2(60.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let after = h.state().diorama.center.unwrap();
    assert!(after[0] < before[0] - 20.0, "dragging right moves the view west: {before:?} → {after:?}");
    // Zoom with the wheel, about the pointer.
    let zoom = h.state().diorama.zoom;
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, 120.0), modifiers: Default::default(), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
    h.run_steps(20);
    assert!(h.state().diorama.zoom > zoom, "the wheel zooms in");
    // Space comes back to the map's middle.
    h.key_press(egui::Key::Space);
    h.run_steps(2);
    let c = h.state().diorama.center.unwrap();
    assert!(c[0].abs() < 1.0 && c[1].abs() < 1.0, "Space recentres: {c:?}");
    // The whole map from afar.
    h.state_mut().diorama.zoom = 0.6;
    h.run_steps(6);
    save(&mut h, "whole");
    // Turned: east up.
    h.key_press(egui::Key::R);
    h.run_steps(2);
    assert_eq!(h.state().diorama.turn, 1);
    h.state_mut().diorama.turn = 0;
    if std::env::var_os("MASSIF_PERF").is_some() {
        h.state_mut().diorama.center = Some([massif_render::diorama::sample::VILLAGE.0 as f64, massif_render::diorama::sample::VILLAGE.1 as f64]);
        for zoom in [6.0, 1.5, 0.6] {
            h.state_mut().diorama.zoom = zoom;
            h.run_steps(8);
            let mut best = std::time::Duration::MAX;
            for _ in 0..12 {
                let t = std::time::Instant::now();
                h.step();
                best = best.min(t.elapsed());
            }
            eprintln!("diorama frame at {zoom} points a block: best {:.2} ms (whole app, CPU side; area {:.2} ms)", best.as_secs_f64() * 1000.0, h.state().diorama.frame_time.as_secs_f64() * 1000.0);
        }
    }
}

#[test]
fn the_caves_under_a_height() {
    let (mut h, _) = diorama(1.0);
    settle(&mut h);
    let d = &mut h.state_mut().diorama;
    d.center = Some([massif_render::diorama::sample::VILLAGE.0 as f64, massif_render::diorama::sample::VILLAGE.1 as f64]);
    d.zoom = 2.0;
    h.run_steps(4);
    // C shows the caves; Page Down takes the height down.
    h.event(egui::Event::PointerMoved(egui::pos2(800.0, 500.0)));
    h.key_press(egui::Key::C);
    h.run_steps(2);
    assert!(h.state().diorama.caves, "C shows the caves");
    let slice = h.state().diorama.slice;
    h.key_press(egui::Key::PageDown);
    h.run_steps(2);
    assert_eq!(h.state().diorama.slice, slice - 1);
    h.state_mut().diorama.slice = 40;
    settle(&mut h);
    save(&mut h, "caves");
    h.key_press(egui::Key::C);
    h.run_steps(2);
    assert!(!h.state().diorama.caves);
}

/// The sky's looks on the village map, from a hillside over the valley
/// (MASSIF_SNAPSHOT_DIR: look-<n>.png, MASSIF_LOOK_TAG naming them for a
/// before and an after).
#[test]
fn the_looks_over_the_village() {
    let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else { return };
    let tag = std::env::var("MASSIF_LOOK_TAG").unwrap_or_default();
    let (mut h, _) = diorama(1.0);
    h.state_mut().layout.show_side(crate::layout::Side::Render);
    h.run_steps(3);
    let (vx, vz) = massif_render::diorama::sample::VILLAGE;
    let ground = |h: &Harness<'_, MassifApp>, x: i32, z: i32| crate::tools::probe(&h.state().doc, x, z).map_or(70.0, |p| p.height as f64);
    let eye = [vx as f64 - 140.0, ground(&h, vx - 140, vz + 120) + 40.0, vz as f64 + 120.0];
    let target = [vx as f64 + 20.0, ground(&h, vx + 20, vz), vz as f64];
    h.state_mut().render_side.set_view(eye, target);
    let started = std::time::Instant::now();
    while h.state().render_side.building() && started.elapsed().as_secs() < 300 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.state_mut().settings.reduce_motion = crate::access::Follow::On;
    for k in 0..7 {
        let app = h.state_mut();
        let was = app.render_side.sky();
        if app.render_side.sky_edit(Some(k), was, &mut app.doc) != k {
            break;
        }
        let started = std::time::Instant::now();
        while h.state().render_side.samples() < 64 && started.elapsed().as_secs() < 120 {
            h.step();
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        h.run_steps(3);
        let (w, hh, rgba) = h.state().render_side.image().unwrap();
        image::save_buffer(dir.join(format!("look-{k}{tag}.png")), &rgba, w, hh, image::ColorType::Rgba8).unwrap();
    }
}
