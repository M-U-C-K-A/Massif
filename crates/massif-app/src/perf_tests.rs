//! The performance recorder at work on the whole app, offscreen:
//!
//! - `profile_a_session` (MASSIF_PROFILE=1): a map (MASSIF_MOVE_WORLD, a
//!   .massif, else one made of MASSIF_PROFILE_TILES² tiles, 32 by default)
//!   in the Layout workspace with a Diorama and a floating 3D View beside
//!   it; then still, panned, zoomed, a big brush held and dragged, terrain
//!   painted, a large reference image added and dragged. Each stage is
//!   recorded on its own and its figures printed; MASSIF_PROFILE_OUT=<dir>
//!   keeps each stage's recording as a zip.
//! - `replay_a_recording` (MASSIF_REPLAY=<a recording's zip or folder>,
//!   MASSIF_REPLAY_MAP=<.massif>): the input of a user's recording played
//!   back frame by frame on the map, in the recording's workspaces, window
//!   size and pixels a point; the frames timed as the recorder times them.
//!
//! Both with `--release -- --nocapture`. The GPU's own time is in each
//! frame's `render` (kittest reads the image back as a window would not).

use super::*;
use egui::{Modifiers, Pos2, Vec2};
use egui_kittest::Harness;
use massif_core::generate::Landform;

type H = Harness<'static, MassifApp>;

fn harness(size: Vec2, ppp: f32) -> H {
    egui_kittest::Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .with_step_dt(1.0 / 60.0)
        .with_accessibility_check(false)
        .wgpu()
        .build_eframe(move |cc| {
            theme::install(&cc.egui_ctx);
            MassifApp::for_tests(generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() }))
        })
}

/// Open `path`, or make a map of `tiles`² tiles, and wait for it.
fn open(h: &mut H, path: Option<PathBuf>, tiles: i32) {
    h.run_steps(3);
    match path {
        Some(path) => {
            let ctx = h.ctx.clone();
            h.state_mut().jobs.push(Job::open(&ctx, path));
            while !h.state().jobs.is_empty() {
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.state_mut().dialog = None;
        }
        None => {
            let world = generate(&GenParams { tiles_x: tiles, tiles_z: tiles, landform: Landform::Islands, ..GenParams::default() });
            let generation = h.state().doc.generation + 1;
            h.state_mut().doc = Document::new(world, None, generation);
            h.state_mut().map_cache.invalidate_all();
        }
    }
    h.run_steps(3);
}

/// Time `frames` frames, `each` giving each its input: the recorder's
/// figures, the processor's time, and kittest's draw of each frame.
fn stage(h: &mut H, name: &str, frames: usize, each: &mut dyn FnMut(&mut H, usize)) {
    // MASSIF_PROFILE_STAGES=raise,paint: only those (the others still run, unrecorded).
    if let Ok(only) = std::env::var("MASSIF_PROFILE_STAGES")
        && !only.split(',').any(|s| s.trim() == name)
    {
        for k in 0..frames {
            each(h, k);
            h.step();
        }
        return;
    }
    let ctx = h.ctx.clone();
    h.state_mut().start_recording(&ctx, true);
    let cpu = crate::devtools::cpu_time().unwrap_or_default();
    let wall = std::time::Instant::now();
    let mut draws = Vec::new();
    for k in 0..frames {
        each(h, k);
        h.step();
        let t = std::time::Instant::now();
        let _ = h.render();
        draws.push(t.elapsed().as_secs_f32() * 1000.0);
    }
    let cpu = crate::devtools::cpu_time().unwrap_or_default().saturating_sub(cpu);
    let mut r = h.state_mut().recording.take().expect("recording");
    r.stop();
    let summary = r.summary();
    eprintln!(
        "\n=== {name}: {frames} frames, {:.2} s, CPU {:.2} s ({:.0} % of a core); kittest draw median {:.1} ms",
        wall.elapsed().as_secs_f32(),
        cpu.as_secs_f32(),
        cpu.as_secs_f32() / wall.elapsed().as_secs_f32() * 100.0,
        crate::perf::percentile(&draws, 0.5)
    );
    for line in summary.lines().skip(1).take_while(|l| !l.starts_with("Other threads")).filter(|l| !l.is_empty()).take(22) {
        eprintln!("{line}");
    }
    if let Some(dir) = std::env::var_os("MASSIF_PROFILE_OUT").map(PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = crate::report::save(&dir.join(format!("{name}.zip")), &r.parts(""));
        let _ = std::fs::write(dir.join(format!("{name}.txt")), &summary);
    }
}

fn map_id(h: &H) -> AreaId {
    h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view")
}

fn map_rect(h: &H) -> Rect {
    let id = map_id(h);
    let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap_or_default();
    areas.iter().find(|a| a.0 == id).map(|a| a.1).expect("the map's area")
}

fn press(h: &mut H, pos: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// A stroke going round a circle of `r` points around `c`, frame by frame.
fn circling(c: Pos2, r: f32, frames: usize) -> impl FnMut(&mut H, usize) {
    move |h: &mut H, k: usize| {
        let a = k as f32 / 40.0 * std::f32::consts::TAU;
        let p = c + vec2(a.cos(), a.sin()) * r;
        if k == 0 {
            h.event(egui::Event::PointerMoved(p));
            press(h, p, true);
        } else if k + 1 == frames {
            press(h, p, false);
        } else {
            h.event(egui::Event::PointerMoved(p));
        }
    }
}

/// Help ▸ Record Performance…: the dialog, Start, the red dot in the top
/// bar and the menu's Stop; the dot stops it, and the recording is written
/// as a zip with its files. Renders the dialog and the dot with
/// MASSIF_SNAPSHOT_DIR.
#[test]
fn recording_performance_from_the_help_menu() {
    let _turn = crate::perf::RECORDING.lock().unwrap_or_else(|e| e.into_inner());
    use egui_kittest::kittest::Queryable;
    let dir = std::env::temp_dir().join(format!("massif-perf-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::perf::TEST_DIR.with_borrow_mut(|d| *d = Some(dir.clone()));
    let snapshots = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let mut h = harness(vec2(1440.0, 900.0), 1.0);
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().command(&ctx, crate::menu::Command::RecordPerformance);
    h.run_steps(3);
    assert!(matches!(h.state().dialog, Some(Dialog::RecordPerformance)));
    if let Some(out) = &snapshots {
        h.render().unwrap().save(out.join("record-performance.png")).unwrap();
    }
    h.get_by_label_contains("Start recording").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(h.state().recording.as_ref().is_some_and(|r| r.is_on() && !r.for_report));
    assert!(h.state().menu_state().recording, "the Help menu stops it");
    // Some input, as the user's.
    let c = map_rect(&h).center();
    for k in 0..20 {
        h.event(egui::Event::PointerMoved(c + vec2(k as f32 * 3.0, 0.0)));
        h.step();
    }
    if let Some(out) = &snapshots {
        h.render().unwrap().save(out.join("record-performance-dot.png")).unwrap();
    }
    h.get_by_label("Stop Recording Performance").click();
    h.run_steps(2);
    assert!(h.state().recording.is_none());
    let started = std::time::Instant::now();
    while h.state().perf_saving.is_some() {
        assert!(started.elapsed().as_secs() < 30, "the recording is written");
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.step();
    }
    let zip = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "zip")).expect("a zip");
    assert!(zip.file_name().unwrap().to_string_lossy().starts_with("Massif performance "), "{zip:?}");
    let names: Vec<String> = crate::report::unzip(&std::fs::read(&zip).unwrap()).into_iter().map(|f| f.0).collect();
    for n in ["summary.txt", "frames.csv", "trace.json", "inputs.jsonl", "system.txt", "layout.json"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    let back = crate::perf::read_recording(&zip).unwrap();
    assert!(back.frames.len() >= 20 && back.frames.iter().any(|f| !f.events.is_empty()), "{} frames", back.frames.len());
    assert!(back.layout.is_some());
    crate::perf::TEST_DIR.with_borrow_mut(|d| *d = None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn profile_a_session() {
    if std::env::var_os("MASSIF_PROFILE").is_none() {
        return;
    }
    let _turn = crate::perf::RECORDING.lock().unwrap_or_else(|e| e.into_inner());
    let tiles = std::env::var("MASSIF_PROFILE_TILES").ok().and_then(|v| v.parse().ok()).unwrap_or(32);
    let mut h = harness(vec2(1440.0, 900.0), 2.0);
    open(&mut h, std::env::var_os("MASSIF_MOVE_WORLD").map(PathBuf::from), tiles);
    // The Layout workspace, a Diorama under the map, a 3D View floating.
    h.state_mut().layout.activate(0);
    h.run_steps(2);
    let map = map_id(&h);
    if std::env::var_os("MASSIF_PROFILE_PLAIN").is_none() {
        let layout = &mut h.state_mut().layout;
        let dio = layout.split_at(map, crate::layout::Dir::Column, false, 0.7);
        layout.current_mut().root.set_editor(dio, EditorKind::Diorama);
        let float = layout.split_at(dio, crate::layout::Dir::Row, false, 0.5);
        layout.current_mut().root.set_editor(float, EditorKind::View3d);
        layout.detach(float, [900.0, 120.0, 480.0, 360.0], false);
    }
    h.run_steps(3);
    let areas: Vec<String> = {
        let mut a = Vec::new();
        h.state().layout.current().root.areas(&mut a);
        a.iter().map(|(_, e)| format!("{e:?}")).collect()
    };
    eprintln!("areas: {areas:?}, floating {:?}", h.state().layout.current().floating.iter().map(|f| f.editor).collect::<Vec<_>>());
    // Let the views build what they show.
    let started = std::time::Instant::now();
    while (h.state().map_cache.busy() || h.state().view3d.is_building()) && started.elapsed().as_secs() < 120 {
        h.step();
    }
    eprintln!("built in {:.1} s", started.elapsed().as_secs_f32());
    h.run_steps(20);

    stage(&mut h, "still", 60, &mut |_, _| {});
    let rect = map_rect(&h);
    let c = rect.center();
    stage(&mut h, "pan", 90, &mut |h, k| {
        let p = c + vec2(k as f32 * 4.0 - 180.0, 0.0);
        match k {
            0 => {
                h.event(egui::Event::PointerMoved(p));
                h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Middle, pressed: true, modifiers: Modifiers::NONE });
            }
            89 => h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Middle, pressed: false, modifiers: Modifiers::NONE }),
            _ => h.event(egui::Event::PointerMoved(p)),
        }
    });
    stage(&mut h, "zoom", 60, &mut |h, k| {
        h.event(egui::Event::PointerMoved(c));
        let dir = if k < 30 { 1.0 } else { -1.0 };
        h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, dir), modifiers: Modifiers::CTRL, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
    });
    h.run_steps(30);
    // A big brush, raising.
    h.state_mut().tools.select(Tool::Raise);
    h.state_mut().tools.brush.radius = std::env::var("MASSIF_PROFILE_RADIUS").ok().and_then(|v| v.parse().ok()).unwrap_or(100.0);
    h.run_steps(3);
    stage(&mut h, "raise", 120, &mut circling(c, 120.0, 120));
    h.run_steps(30);
    stage(&mut h, "after the stroke", 60, &mut |_, _| {});
    h.state_mut().tools.select(Tool::PaintTerrain);
    h.run_steps(3);
    stage(&mut h, "paint", 120, &mut circling(c, 120.0, 120));
    h.run_steps(30);
    // A large reference image: added, then dragged.
    let png = std::env::temp_dir().join(format!("massif-profile-{}.png", std::process::id()));
    image::RgbaImage::from_fn(4096, 3072, |x, y| image::Rgba([(x / 16) as u8, (y / 12) as u8, ((x ^ y) / 32) as u8, 255])).save(&png).unwrap();
    let at = h.state().maps[&map].center;
    stage(&mut h, "reference added", 30, &mut |h, k| {
        if k == 0 {
            h.state_mut().add_references(std::slice::from_ref(&png), Some((at[0], at[1])));
        }
    });
    stage(&mut h, "reference dragged", 90, &mut |h, k| {
        let p = c + vec2((k as f32 * 0.2).sin() * 150.0, k as f32 * 1.5 - 60.0);
        match k {
            0 => {
                h.event(egui::Event::PointerMoved(c));
                press(h, c, true);
            }
            89 => press(h, p, false),
            _ => h.event(egui::Event::PointerMoved(p)),
        }
    });
    let _ = std::fs::remove_file(&png);
    stage(&mut h, "still at the end", 60, &mut |_, _| {});
}

#[test]
fn replay_a_recording() {
    let (Some(rec), Some(map)) = (std::env::var_os("MASSIF_REPLAY").map(PathBuf::from), std::env::var_os("MASSIF_REPLAY_MAP").map(PathBuf::from)) else {
        return;
    };
    let recorded = crate::perf::read_recording(&rec).expect("a recording");
    let first = recorded.frames.first().cloned().unwrap_or_default();
    eprintln!("replaying {} frames ({:.1} s recorded) at {:.0}×{:.0} points, {} pixels a point", recorded.frames.len(), recorded.frames.last().map_or(0.0, |f| f.ms / 1000.0), first.window[0], first.window[1], first.ppp);
    let mut h = harness(vec2(first.window[0], first.window[1]), first.ppp.max(1.0));
    open(&mut h, Some(map), 0);
    if let Some(layout) = recorded.layout.as_deref().and_then(|l| serde_json::from_str::<crate::layout::Layout>(l).ok()) {
        h.state_mut().layout = layout;
    }
    h.run_steps(5);
    let frames = recorded.frames.clone();
    stage(&mut h, "replay", frames.len(), &mut |h, k| {
        let f = &frames[k];
        if let Some(tool) = &f.tool
            && let Ok(t) = serde_json::from_value::<Tool>(serde_json::json!(tool))
        {
            h.state_mut().tools.select(t);
        }
        for e in &f.events {
            h.event(e.clone());
        }
    });
}
