//! What the views of the map load (Settings ▸ Graphics ▸ Load): around what
//! they show, or the whole map. On a small map the views load around them
//! and stop once what they show is built; on a real map
//! (MASSIF_MOVE_WORLD, a .massif; MASSIF_LOAD: around, whole or both) the
//! time and the processor's time each view takes to be complete, the
//! processor at rest after, the memory in use, and each frame's time while
//! the map pans, in each mode.

use super::*;
use massif_core::generate::GenParams;
use egui::vec2;
use egui_kittest::Harness;

type H = Harness<'static, MassifApp>;

fn harness(world: World) -> H {
    let mut h = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .with_step_dt(1.0 / 60.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            MassifApp::for_tests(world)
        });
    h.run_steps(3);
    h
}

/// The workspace showing the map and the 3D view, and the one showing the
/// map alone.
fn workspaces(h: &H) -> (usize, usize) {
    let spaces = &h.state().layout.workspaces;
    let both = spaces.iter().position(|w| w.root.shows(EditorKind::Map) && w.root.shows(EditorKind::View3d)).expect("a workspace with both views");
    let alone = spaces.iter().position(|w| w.root.shows(EditorKind::Map) && !w.root.shows(EditorKind::View3d) && !w.root.shows(EditorKind::Sculpt3d)).expect("a workspace with the map alone");
    (both, alone)
}

/// Frames until `done` (at most `cap` seconds): the seconds, the processor's
/// seconds (every thread) and the frames.
fn until(h: &mut H, cap: f32, done: &dyn Fn(&MassifApp) -> bool) -> (f32, f32, usize) {
    let (started, cpu0) = (std::time::Instant::now(), crate::devtools::cpu_time().unwrap_or_default());
    let mut frames = 0;
    let mut work = Vec::new();
    loop {
        let t = std::time::Instant::now();
        h.step();
        work.push(t.elapsed().as_secs_f32() * 1000.0);
        frames += 1;
        if (frames >= 3 && done(h.state())) || started.elapsed().as_secs_f32() > cap {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    work.sort_by(f32::total_cmp);
    let n = work.len();
    if std::env::var_os("MASSIF_MOVE_WORLD").is_some() {
        eprintln!("    (a frame's work: middle {:.1} ms, p95 {:.1}, worst {:.1})", work[n / 2], work[(n * 95 / 100).min(n - 1)], work[n - 1]);
    }
    (started.elapsed().as_secs_f32(), (crate::devtools::cpu_time().unwrap_or_default() - cpu0).as_secs_f32(), frames)
}

/// `seconds` as the app runs when nothing is touched: a frame only when
/// one was asked for. The processor's share of a core over that time, and
/// the frames drawn.
fn at_rest(h: &mut H, seconds: f32) -> (f32, usize) {
    let (started, cpu0) = (std::time::Instant::now(), crate::devtools::cpu_time().unwrap_or_default());
    let mut frames = 0;
    while started.elapsed().as_secs_f32() < seconds {
        if h.ctx.has_requested_repaint() {
            h.step();
            frames += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let cpu = (crate::devtools::cpu_time().unwrap_or_default() - cpu0).as_secs_f32();
    (100.0 * cpu / started.elapsed().as_secs_f32(), frames)
}

fn set_whole(h: &mut H, whole: bool) {
    h.state_mut().settings.load_whole = whole;
    crate::graphics::set_loading(crate::graphics::Loading { whole, distance: h.state().settings.view_distance });
}

/// On a map of 24 × 24 tiles, a view distance of 1,024 blocks: the 3D view
/// builds the tiles around its camera and stops; the whole map builds
/// every tile.
#[test]
fn the_3d_view_builds_around_its_camera() {
    let all = 24 * 24;
    for whole in [false, true] {
        let mut h = harness(generate(&GenParams { tiles_x: 24, tiles_z: 24, ..GenParams::default() }));
        h.state_mut().settings.view_distance = 1024;
        set_whole(&mut h, whole);
        let (both, _) = workspaces(&h);
        h.state_mut().layout.active = both;
        until(&mut h, 120.0, &|app| app.view3d.pending() == 0 && !app.view3d.building());
        let built = h.state().view3d.cells_built();
        if whole {
            assert_eq!(built, all, "the whole map built");
        } else {
            // A disc of 1,024 blocks (8 tiles) and a half diagonal more.
            assert!(built > 0 && built < all / 2, "{built} of {all} tiles built around the camera");
            // The camera taken to a corner: the corner built, then rest.
            let (_, hi) = h.state().doc.dimension().bounds().expect("a map");
            let corner = (hi.x * 128 + 28, hi.z * 128 + 28);
            assert!(!h.state().view3d.built_at(corner.0, corner.1));
            h.state_mut().view3d.camera.target = [corner.0 as f64, 80.0, corner.1 as f64];
            until(&mut h, 120.0, &|app| app.view3d.pending() == 0 && !app.view3d.building());
            assert!(h.state().view3d.built_at(corner.0, corner.1), "the corner built once the camera is there");
            assert!(h.state().view3d.cells_built() < all, "still not the whole map");
        }
        // Nothing more to do: no frame asked for at once.
        h.run_steps(10);
        let asked = (0..60).filter(|_| {
            h.step();
            h.ctx.requested_repaint_last_pass_for(&h.ctx.viewport_id())
        });
        assert_eq!(asked.count(), 0, "the 3D view rests once built");
    }
}

/// The Diorama, around the view, builds the tiles in view and a margin,
/// not the map's every tile; with the whole map, every tile.
#[test]
fn the_diorama_builds_around_the_view() {
    for whole in [false, true] {
        let mut h = harness(generate(&GenParams { tiles_x: 24, tiles_z: 24, ..GenParams::default() }));
        h.state_mut().settings.view_distance = 1024;
        set_whole(&mut h, whole);
        let id = *h.state().maps.keys().next().expect("a map area");
        h.state_mut().layout.current_mut().root.set_editor(id, EditorKind::Diorama);
        h.state_mut().diorama.zoom = 2.0;
        until(&mut h, 120.0, &|app| app.diorama.settled());
        let p = h.state().diorama.progress();
        assert!(h.state().diorama.settled(), "{p:?}");
        if whole {
            assert!(p.tiles == 24 * 24 && p.built >= p.tiles, "{p:?}");
        } else {
            assert!(p.built > 0 && p.built < 24 * 24 / 2, "{p:?}");
            assert!(p.tiles <= 24 * 24 / 2, "{p:?}");
        }
    }
}

/// A real map, around the view and whole: see the module's comment.
#[test]
fn loading_a_real_world_around_the_view() {
    let Some(path) = std::env::var_os("MASSIF_MOVE_WORLD").map(PathBuf::from) else {
        return;
    };
    let modes: Vec<bool> = match std::env::var("MASSIF_LOAD").as_deref() {
        Ok("around") => vec![false],
        Ok("whole") => vec![true],
        _ => vec![false, true],
    };
    let cap = std::env::var("MASSIF_MOVE_SECONDS").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(180.0);
    for whole in modes {
        let name = if whole { "whole map" } else { "around the view" };
        let mut h = harness(generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() }));
        set_whole(&mut h, whole);
        let (both, alone) = workspaces(&h);
        h.state_mut().layout.active = alone;
        let ctx = h.ctx.clone();
        let started = std::time::Instant::now();
        h.state_mut().jobs.push(Job::open(&ctx, path.clone()));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.state_mut().dialog = None;
        let open = started.elapsed().as_secs_f32();
        let m0 = crate::devtools::memory_in_use().unwrap_or(0) >> 20;
        eprintln!("== {name}: opened in {open:.2} s, {m0} MB in use");
        let (t, cpu, n) = until(&mut h, cap, &|app| !app.map_cache.busy());
        eprintln!("2D map, the whole map in view: complete in {t:.2} s, CPU {cpu:.1} s, {n} frames");
        h.state_mut().layout.active = both;
        let (t, cpu, n) = until(&mut h, cap, &|app| app.view3d.pending() == 0 && !app.view3d.building() && !app.map_cache.busy());
        eprintln!("3D view beside it: complete in {t:.2} s, CPU {cpu:.1} s, {n} frames, {} cells shown, {} built for good, {} still to build", h.state().view3d.tiles_built(), h.state().view3d.cells_built(), h.state().view3d.pending());
        let (cpu, frames) = at_rest(&mut h, 10.0);
        eprintln!("at rest, map and 3D view: {cpu:.1} % of a core, {frames} frames in 10 s");
        // The map panned close up (a block a pixel), a screen's 32nd a
        // frame, as a hand drags, for two seconds; then until it is drawn.
        for m in h.state_mut().maps.values_mut() {
            m.zoom = 0.5;
        }
        until(&mut h, cap, &|app| !app.map_cache.busy());
        let mut work = Vec::new();
        for k in 0..120 {
            let step = if k < 60 { 1.0 } else { -1.0 };
            for m in h.state_mut().maps.values_mut() {
                m.center[0] += step * 1440.0 / 32.0 / m.zoom as f64;
            }
            let t = std::time::Instant::now();
            h.step();
            work.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        work.sort_by(f32::total_cmp);
        eprintln!("panning the 2D map close up: frame middle {:.1} ms, p95 {:.1}, worst {:.1}", work[60], work[114], work[119]);
        let (t, cpu, _) = until(&mut h, cap, &|app| app.view3d.pending() == 0 && !app.view3d.building() && !app.map_cache.busy());
        eprintln!("after the pan, both views complete in {t:.2} s, CPU {cpu:.1} s");
        // The 3D camera taken 10,000 blocks east.
        h.state_mut().view3d.camera.target[0] += 10_000.0;
        let (t, cpu, _) = until(&mut h, cap, &|app| app.view3d.pending() == 0 && !app.view3d.building());
        eprintln!("3D camera moved 10,000 blocks: complete in {t:.2} s, CPU {cpu:.1} s, {} cells built for good", h.state().view3d.cells_built());
        // The Diorama in the map's place.
        let id = *h.state().maps.keys().next().expect("a map area");
        h.state_mut().layout.current_mut().root.set_editor(id, EditorKind::Diorama);
        for k in 0..3 {
            let t = std::time::Instant::now();
            h.step();
            eprintln!("    Diorama's frame {k}: {:.1} ms, of which the Diorama {:.1} ms", t.elapsed().as_secs_f32() * 1000.0, h.state().diorama.frame_time.as_secs_f32() * 1000.0);
        }
        let (t, cpu, n) = until(&mut h, cap, &|app| app.diorama.settled());
        eprintln!("Diorama: {} in {t:.2} s, CPU {cpu:.1} s, {n} frames; {:?}", if h.state().diorama.settled() { "complete" } else { "NOT complete" }, h.state().diorama.progress());
        let (cpu, frames) = at_rest(&mut h, 10.0);
        eprintln!("at rest, Diorama and 3D view: {cpu:.1} % of a core, {frames} frames in 10 s");
        // The Render in the Diorama's place.
        h.state_mut().layout.current_mut().root.set_editor(id, EditorKind::RenderView);
        let (t, cpu, n) = until(&mut h, cap, &|app| !app.render_side.building());
        eprintln!("Render: {} in {t:.2} s, CPU {cpu:.1} s, {n} frames; tiles {:?}", if h.state().render_side.building() { "NOT built" } else { "built" }, h.state().render_side.tiles_built());
        let (cpu, frames) = at_rest(&mut h, 10.0);
        eprintln!("at rest, Render and 3D view: {cpu:.1} % of a core, {frames} frames in 10 s");
        let (cpu, frames) = at_rest(&mut h, 50.0);
        eprintln!("a minute after: {cpu:.1} % of a core, {frames} frames in 50 s; {} MB in use", crate::devtools::memory_in_use().unwrap_or(0) >> 20);
    }
}

/// The pixels of `image` near black: the 3D view's land drawn without its
/// colours shows as such.
fn dark_pixels(image: &image::RgbaImage) -> usize {
    image.pixels().filter(|p| p.0[0] < 12 && p.0[1] < 12 && p.0[2] < 12).count()
}

/// A graphics preset picked while the 3D view is shown (Low draws without
/// smoothed edges, Medium and High with): the view draws the land as
/// before, in its colours, not black.
#[test]
fn the_3d_view_keeps_its_colours_through_a_graphics_preset() {
    use crate::graphics::{Graphics, Preset};
    let mut h = harness(generate(&GenParams { tiles_x: 4, tiles_z: 4, ..GenParams::default() }));
    crate::graphics::set(Graphics::of(Preset::Medium));
    let (both, _) = workspaces(&h);
    h.state_mut().layout.active = both;
    let settle = |h: &mut H| {
        until(h, 120.0, &|app| app.view3d.pending() == 0 && !app.view3d.building());
        h.run_steps(10);
        h.render().unwrap()
    };
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let save = |k: usize, image: &image::RgbaImage| {
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
            image.save(dir.join(format!("graphics-preset-{k}.png"))).unwrap();
        }
    };
    let first = settle(&mut h);
    save(0, &first);
    let dark0 = dark_pixels(&first);
    for (k, preset) in [Preset::Low, Preset::Medium, Preset::High, Preset::Low].into_iter().enumerate() {
        crate::graphics::set(Graphics::of(preset));
        let image = settle(&mut h);
        save(k + 1, &image);
        let dark = dark_pixels(&image);
        assert!(dark <= dark0 + (first.width() * first.height()) as usize / 200, "{preset:?}: {dark} pixels near black, {dark0} before");
    }
}
