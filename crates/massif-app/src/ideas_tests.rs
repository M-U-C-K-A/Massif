//! The testers' ideas of 1.1: the linked sides of a map's size, snowfall,
//! and the noise painter's gradient. The
//! whole app, at 1× (and 2× with MASSIF_SNAPSHOT_PPP=2); with
//! MASSIF_SNAPSHOT_DIR, renders of each.

use super::tests::harness_with;
use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn ppp() -> f32 {
    std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from)
}

fn save(h: &mut Harness<'_, MassifApp>, name: &str) {
    if let Some(dir) = snapshot_dir() {
        let _ = std::fs::create_dir_all(&dir);
        h.render().expect("rendered").save(dir.join(format!("{name}@{}x.png", ppp()))).unwrap();
    }
}

fn map_id(h: &Harness<'_, MassifApp>) -> AreaId {
    h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view")
}

/// The width and depth of a new map are linked: typing one sets the other,
/// unlinked they go apart.
#[test]
fn the_sides_of_a_map_are_linked_until_unlinked() {
    use crate::widgets::link_sides;
    assert_eq!(link_sides(true, (1024, 1024), (2048, 1024), false), (2048, 2048));
    assert_eq!(link_sides(true, (1024, 1024), (1024, 512), false), (512, 512));
    assert_eq!(link_sides(false, (1024, 1024), (2048, 1024), false), (2048, 1024));
    // Linked again: the depth takes the width.
    assert_eq!(link_sides(true, (2048, 1024), (2048, 1024), true), (2048, 2048));
    let mut h = harness_with(ThemeChoice::Dark, ppp());
    h.run_steps(3);
    h.state_mut().open_home(crate::home::Page::NewMap);
    h.run_steps(10);
    save(&mut h, "size-linked");
    // The chain unlinks them.
    h.get_by_label("Link the sides").click();
    h.run_steps(5);
    save(&mut h, "size-unlinked");
    let linked = h.ctx.data_mut(|m| m.get_persisted::<bool>(egui::Id::unique(("home size", "linked"))));
    assert_eq!(linked, Some(false));
}

/// Snowfall over mountains: its dialog shows the snow first, then laid,
/// it whitens the 3D view; the snow is in eighths, deeper in the lee.
#[test]
fn snowfall_lays_drifts_seen_in_3d() {
    let mut h = egui_kittest::Harness::builder()
        .with_size(vec2(1600.0, 1000.0))
        .with_pixels_per_point(ppp())
        .with_accessibility_check(false)
        .wgpu()
        .build_eframe(|cc| {
            theme::install(&cc.egui_ctx);
            let world = generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: massif_core::generate::Landform::Mountains, ..GenParams::default() });
            let mut app = MassifApp::for_tests(world);
            app.settings.theme = ThemeChoice::Dark;
            app
        });
    h.run_steps(5);
    let ctx = h.ctx.clone();
    h.state_mut().command(&ctx, Command::Snowfall);
    let mut frames = 0;
    while !matches!(&h.state().dialog, Some(Dialog::Snowfall(f)) if f.has_picture()) {
        h.step();
        frames += 1;
        assert!(frames < 200, "no preview of the snow");
    }
    h.run_steps(3);
    save(&mut h, "snowfall-dialog");
    let fall = match &h.state().dialog {
        Some(Dialog::Snowfall(f)) => f.fall.clone(),
        _ => unreachable!(),
    };
    h.state_mut().dialog = None;
    // Laid as the job lays it.
    let snow = h.state().doc.world.registry.terrain_by_name("Snow").unwrap();
    {
        let app = h.state_mut();
        let found = massif_core::graph::snow_over(&app.doc.world.dims[0], &fall, 1024, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let covers = massif_core::graph::snow_covers(&app.doc.world.registry);
        app.doc.history.begin("Snowfall", 0);
        let changes = massif_core::graph::lay_snow(&mut app.doc.history.edit(&mut app.doc.world, 0), &found, &fall, snow, &covers, &massif_core::auto::Steps::default());
        app.doc.commit();
        assert!(changes.columns > 5000, "{} columns of snow", changes.columns);
    }
    h.run_steps(5);
    save(&mut h, "snowfall-map");
    h.state_mut().layout.active = 3;
    let mut frames = 0;
    while frames < 5 || h.state().view3d.pending() > 0 {
        h.step();
        frames += 1;
        assert!(frames < 3000, "the 3D view never finished");
    }
    let top = massif_core::ops::height_at(h.state().doc.dimension(), 0, 0, 0, massif_core::ops::Surface::Top).unwrap_or(120.0);
    h.state_mut().view3d.camera = crate::view3d::Camera { target: [0.5, top as f64, 0.5], yaw: 0.6, pitch: 0.45, distance: 420.0 };
    h.run_steps(30);
    save(&mut h, "snowfall-3d");
}

/// The noise painter's gradient: dirt, coarse dirt, grass blended over
/// the whole map, then the same mix as a node of the graph.
#[test]
fn noise_gradient_blends_its_blocks_in_order() {
    let mut h = harness_with(ThemeChoice::Dark, ppp());
    h.run_steps(5);
    {
        let app = h.state_mut();
        app.tools.select(Tool::PaintTerrain);
        let ps = &mut app.tools.pattern;
        ps.on = true;
        ps.pattern = massif_core::pattern::Pattern { scale: 40.0, softness: 0.0, blend: 1.0, ..Default::default() };
        ps.terrains = vec![("Dirt".into(), 0.3), ("Coarse Dirt".into(), 0.3), ("Grass".into(), 0.4)];
        let sampler = ps.sampler(&app.doc.world.registry).unwrap();
        app.doc.history.begin("Noise", 0);
        massif_core::pattern::paint_over_the_map(&mut app.doc.history.edit(&mut app.doc.world, 0), &sampler, None, &massif_core::auto::Steps::default());
        app.doc.commit();
        // Never dirt beside grass: the blend goes through coarse dirt.
        let reg = &app.doc.world.registry;
        let (dirt, grass) = (reg.terrain_by_name("Dirt").unwrap(), reg.terrain_by_name("Grass").unwrap());
        // Where the noise itself jumps, a few may.
        let (mut touching, mut edges) = (0, 0);
        for z in 0..300 {
            for x in 0..300 {
                let (a, b) = (sampler.terrain(x, z), sampler.terrain(x + 1, z));
                edges += (a != b) as usize;
                touching += ((a == Some(dirt) && b == Some(grass)) || (a == Some(grass) && b == Some(dirt))) as usize;
            }
        }
        assert!(touching * 20 < edges, "{touching} of {edges} edges between dirt and grass");
    }
    let id = map_id(&h);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [100.0, 100.0];
        view.zoom = 3.0;
    }
    h.run_steps(30);
    save(&mut h, "noise-gradient");
    // The node, with the brush's presets.
    {
        let app = h.state_mut();
        app.tools.pattern.save("Dirt to grass");
        let reg = &app.doc.world.registry;
        let ids: Vec<(u16, f32)> = ["Dirt", "Coarse Dirt", "Grass"].iter().zip([0.3, 0.3, 0.4]).map(|(n, w)| (reg.terrain_by_name(n).unwrap().0, w)).collect();
        let mut graph = massif_core::graph::Graph::default();
        let node = graph.add(massif_core::graph::NodeKind::NoisePaint { pattern: app.tools.pattern.pattern.clone(), terrains: ids }, [200.0, 200.0]);
        app.doc.world.meta.graph = Some(graph);
        app.graph.graph_replaced();
        app.graph.selected = Some(node);
        app.layout.switch_side(true);
    }
    let started = std::time::Instant::now();
    h.run_steps(3);
    while !h.state().graph.is_current() && started.elapsed().as_secs() < 90 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    h.run_steps(10);
    save(&mut h, "noise-paint-node");
}
