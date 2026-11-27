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

/// The map made another size in place, its generation kept, as the
/// Terrain Generator's Apply to map does: the Diorama builds the new map,
/// where it showed nothing (0 chunks) before.
#[test]
fn the_diorama_follows_the_map_made_bigger() {
    let (mut h, _) = diorama(1.0);
    settle(&mut h);
    let (before, _) = h.state().diorama.scene_map().expect("a scene");
    let generation = h.state().doc.generation;
    let bigger = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 12, tiles_z: 12, landform: massif_core::generate::Landform::Hills, ..Default::default() });
    let dimension = bigger.dims.into_iter().next().unwrap();
    let count = dimension.ground().len();
    assert_ne!(count, before, "another size");
    h.state_mut().doc.replace_dimension(0, dimension, None, "Apply the terrain graph");
    assert_eq!(h.state().doc.generation, generation, "the same map");
    h.run_steps(3);
    let (tiles, middle) = h.state().diorama.scene_map().expect("a scene");
    assert_eq!(tiles, count, "the scene made for the new map");
    let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
    let want = [((lo.x + hi.x + 1) as f64) * 64.0, ((lo.z + hi.z + 1) as f64) * 64.0];
    assert_eq!(middle, want);
    assert_eq!(h.state().diorama.center, Some(want), "the view on the new map");
    settle(&mut h);
    assert!(h.state().diorama.progress().built > 0, "tiles built");
}

/// A terrain graph applied with the Diorama open, the app drawing frames
/// while the windows are written: once done, every tile it shows is the
/// new map's, as a Diorama made afresh shows it, none left as it was.
#[test]
fn the_diorama_shows_the_graph_applied_whole() {
    let (mut h, _) = diorama(1.0);
    {
        // The whole map built, not only around the view.
        let d = &mut h.state_mut().diorama;
        d.zoom = 0.3;
    }
    settle(&mut h);
    let (before, _) = h.state().diorama.surfaces();
    let graph = massif_terrain::model::Graph::from_json(r#"{ "nodes": {
        "land": { "type": "noise.fbm", "params": { "scale": 300, "height": 50, "base": 55 } },
        "out": { "type": "output.map", "inputs": { "height": "land" } } } }"#).unwrap();
    let blocks = crate::blocks::BlockLibrary::default();
    let engine = Arc::new(massif_terrain::engine::Engine::new(Default::default()));
    let env: Arc<dyn massif_terrain::kernel::Env> = Arc::new(massif_terrain::world::MapEnv::new(&h.state().doc.world, 0, 1, "."));
    let shape = (h.state().doc.dimension().bounds(), h.state().doc.dimension().ground().len());
    let mut a = crate::terrain::apply::Applying::start(engine, &graph, &mut h.state_mut().doc, env, 1, false).unwrap();
    let started = std::time::Instant::now();
    loop {
        let out = a.step(&mut h.state_mut().doc, &blocks, None);
        h.step();
        if let Some(r) = out {
            r.unwrap();
            break;
        }
        assert!(started.elapsed().as_secs() < 120, "Apply never finished");
    }
    assert_eq!((h.state().doc.dimension().bounds(), h.state().doc.dimension().ground().len()), shape, "the map kept its size");
    settle(&mut h);
    let (after, drawn) = h.state().diorama.surfaces();
    let tiles = h.state().doc.dimension().ground().len();
    // Afresh, on the map as it now is.
    h.state_mut().diorama = crate::diorama::Diorama::default();
    h.state_mut().diorama.zoom = 0.3;
    h.run_steps(3);
    settle(&mut h);
    let (fresh, _) = h.state().diorama.surfaces();
    assert_eq!(drawn, tiles, "every tile drawn");
    let old: Vec<_> = after.iter().filter(|(t, s)| fresh.get(t).is_some_and(|f| f != *s)).map(|(t, _)| *t).collect();
    assert!(old.is_empty(), "{} of {} tiles not as the new map: {:?}", old.len(), after.len(), &old[..old.len().min(8)]);
    assert!(after.iter().any(|(t, s)| before.get(t).is_some_and(|b| b != s)), "the map changed");

    // A graph larger than the map: Apply makes the map its size, and the
    // Diorama shows the new map whole (it kept the old map's tiles).
    let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
    let side = ((hi.x - lo.x + 1).max(hi.z - lo.z + 1) * massif_core::TILE_SIZE) as u32;
    let mut bigger = graph.clone();
    bigger.terrain.size = Some(side * 2);
    let engine = Arc::new(massif_terrain::engine::Engine::new(Default::default()));
    let env: Arc<dyn massif_terrain::kernel::Env> = Arc::new(massif_terrain::world::MapEnv::new(&h.state().doc.world, 0, 1, "."));
    let mut a = crate::terrain::apply::Applying::start(engine, &bigger, &mut h.state_mut().doc, env, 2, false).unwrap();
    loop {
        let out = a.step(&mut h.state_mut().doc, &blocks, None);
        h.step();
        if let Some(r) = out {
            r.unwrap();
            break;
        }
        assert!(started.elapsed().as_secs() < 240, "Apply never finished");
    }
    let tiles = h.state().doc.dimension().ground().len();
    assert!(tiles > shape.1, "the map made bigger: {tiles} tiles");
    settle(&mut h);
    let (after, drawn) = h.state().diorama.surfaces();
    assert_eq!(drawn, tiles, "every tile of the new map drawn");
    assert_eq!(h.state().diorama.scene_map().map(|m| m.0), Some(tiles));
    h.state_mut().diorama = crate::diorama::Diorama::default();
    h.state_mut().diorama.zoom = 0.3;
    h.run_steps(3);
    settle(&mut h);
    let (fresh, _) = h.state().diorama.surfaces();
    let old: Vec<_> = after.iter().filter(|(t, s)| fresh.get(t).is_some_and(|f| f != *s)).map(|(t, _)| *t).collect();
    assert!(old.is_empty(), "{} of {} tiles not as the new map", old.len(), after.len());
}

/// How long Apply takes on a map of 4096² blocks with the Diorama open,
/// against with the 2D map: the Diorama's builder must not hold it up.
#[test]
#[ignore]
fn apply_with_the_diorama_open() {
    let graph = massif_terrain::model::Graph::from_json(r#"{ "nodes": {
        "land": { "type": "noise.fbm", "params": { "scale": 600, "height": 60, "base": 60 } },
        "out": { "type": "output.map", "inputs": { "height": "land" } } } }"#).unwrap();
    let blocks = crate::blocks::BlockLibrary::default();
    for open in [true, false] {
        let (mut h, id) = diorama(1.0);
        h.state_mut().doc = crate::doc::Document::new(massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 32, tiles_z: 32, landform: massif_core::generate::Landform::Flat, ..Default::default() }), None, 12);
        if !open {
            h.state_mut().layout.current_mut().root.set_editor(id, EditorKind::Map);
        }
        h.state_mut().diorama.zoom = 0.1;
        h.run_steps(5);
        if open {
            settle(&mut h);
        }
        let engine = Arc::new(massif_terrain::engine::Engine::new(Default::default()));
        let env: Arc<dyn massif_terrain::kernel::Env> = Arc::new(massif_terrain::world::MapEnv::new(&h.state().doc.world, 0, 1, "."));
        let t0 = std::time::Instant::now();
        let mut a = crate::terrain::apply::Applying::start(engine, &graph, &mut h.state_mut().doc, env, 1, false).unwrap();
        let (mut frames, mut slowest) = (0, std::time::Duration::ZERO);
        let done = loop {
            let out = a.step(&mut h.state_mut().doc, &blocks, None);
            let f = std::time::Instant::now();
            h.step();
            slowest = slowest.max(f.elapsed());
            frames += 1;
            if let Some(r) = out {
                break r.unwrap();
            }
        };
        eprintln!("APPLY with the {}: {:.2} s ({:.2} s by Apply), {frames} frames, slowest frame {:.0} ms", if open { "Diorama open" } else { "2D map" }, t0.elapsed().as_secs_f64(), done.seconds, slowest.as_secs_f64() * 1000.0);
    }
}

/// Apply to map as the user does it, from the Terrain Generator's panel
/// through the app's frames, with two Diorama areas open: once done, each
/// draws the new map, every tile as a Diorama made afresh draws it, of the
/// same size and of a larger one.
#[test]
fn both_dioramas_follow_apply_from_the_panel() {
    use crate::layout::{Dir, Node};
    let (mut h, _) = diorama(1.0);
    let ids = [9201, 9202];
    let area = |k: usize| Box::new(Node::Area { id: ids[k], editor: EditorKind::Diorama });
    {
        let app = h.state_mut();
        let root = Node::Split { dir: Dir::Row, ratio: 0.5, a: area(0), b: area(1) };
        app.layout.workspaces.push(crate::layout::Workspace { name: "Two dioramas".into(), root, maximized: None, terrain: false, render: false, floating: Vec::new() });
        let layout = std::mem::take(&mut app.layout).sanitize();
        app.layout = layout;
        app.layout.activate(app.layout.workspaces.len() - 1);
    }
    h.run_steps(4);
    assert_eq!(h.state().extra_dioramas.len(), 1, "two Dioramas");
    let all_settled = |h: &Harness<'_, MassifApp>| h.state().diorama.settled() && h.state().extra_dioramas[0].settled();
    let wait = |h: &mut Harness<'_, MassifApp>| {
        let started = std::time::Instant::now();
        while !all_settled(h) && started.elapsed().as_secs() < 240 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        h.run_steps(4);
        assert!(all_settled(h), "both built: {:?} {:?}", h.state().diorama.progress(), h.state().extra_dioramas[0].progress());
    };
    for d in std::iter::once(&mut h.state_mut().diorama) {
        d.zoom = 0.3;
    }
    h.state_mut().extra_dioramas[0].zoom = 0.3;
    wait(&mut h);
    let (before, _) = h.state().diorama.surfaces();
    let graph = massif_terrain::model::Graph::from_json(r#"{ "nodes": {
        "land": { "type": "noise.fbm", "params": { "scale": 300, "height": 50, "base": 55 } },
        "out": { "type": "output.map", "inputs": { "height": "land" } } } }"#).unwrap();
    let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
    let side = ((hi.x - lo.x + 1).max(hi.z - lo.z + 1) * massif_core::TILE_SIZE) as u32;
    let mut bigger = graph.clone();
    bigger.terrain.size = Some(side * 2);
    for (k, g) in [graph, bigger].into_iter().enumerate() {
        {
            let mut p = crate::terrain::TerrainPanel::with_graph(Arc::new(massif_terrain::engine::Engine::new(Default::default())), g);
            p.apply_asked = true;
            h.state_mut().terrain_panel = Some(p);
        }
        let started = std::time::Instant::now();
        while h.state().terrain_panel.as_ref().is_some_and(|p| p.applying.is_some() || p.apply_asked) {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(2));
            assert!(started.elapsed().as_secs() < 240, "Apply never finished");
        }
        wait(&mut h);
        let tiles = h.state().doc.dimension().ground().len();
        let shown = [h.state().diorama.surfaces(), h.state().extra_dioramas[0].surfaces()];
        if k == 0 {
            assert!(shown[0].0.iter().any(|(t, s)| before.get(t).is_some_and(|b| b != s)), "the first Diorama shows the map changed");
        }
        // Afresh, on the map as it now is.
        let mut fresh = crate::diorama::Diorama::default();
        fresh.zoom = 0.3;
        let kept = std::mem::replace(&mut h.state_mut().diorama, fresh);
        h.run_steps(3);
        settle(&mut h);
        let (fresh, _) = h.state().diorama.surfaces();
        h.state_mut().diorama = kept;
        for (n, (after, drawn)) in shown.iter().enumerate() {
            assert_eq!(*drawn, tiles, "Apply {k}: Diorama {n} drew every tile");
            let old: Vec<_> = after.iter().filter(|(t, s)| fresh.get(t).is_none_or(|f| f != *s)).map(|(t, _)| *t).collect();
            assert!(old.is_empty(), "Apply {k}: Diorama {n}: {} of {} tiles not as the new map", old.len(), after.len());
        }
    }
}
