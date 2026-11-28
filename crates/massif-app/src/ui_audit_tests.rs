//! The whole interface rendered offscreen, to look over its alignment:
//! every workspace, every page of Settings and the dialogs the menus open,
//! in the normal and the compact mode. Ignored: run it with
//! `MASSIF_SNAPSHOT_DIR=<dir> MASSIF_SNAPSHOT_PPP=2 cargo test -p massif-app ui_audit -- --ignored --test-threads=1`
//! (`MASSIF_SNAPSHOT_PPP` 1 or 2; `MASSIF_AUDIT_ONLY`: a word the names
//! saved must hold, to render a few).

use super::*;
use egui_kittest::Harness;
use massif_core::generate::Landform;

fn ppp() -> f32 {
    std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn harness(size: egui::Vec2, compact: bool) -> Harness<'static, MassifApp> {
    let world = generate(&GenParams { tiles_x: 3, tiles_z: 3, landform: Landform::Hills, ..GenParams::default() });
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp()).with_accessibility_check(false).wgpu().build_eframe(move |cc| {
        theme::install(&cc.egui_ctx);
        let mut app = MassifApp::for_tests(world);
        app.settings.theme = ThemeChoice::Dark;
        app.settings.reduce_motion = Follow::On;
        app.settings.compact = compact;
        app
    });
    h.run_steps(8);
    h
}

/// The window over the others: the largest area drawn above the panels.
fn top_window(h: &Harness<'_, MassifApp>) -> Option<Rect> {
    let screen = h.ctx.content_rect();
    h.ctx.memory(|m| {
        m.layer_ids()
            .filter(|l| matches!(l.order, egui::Order::Middle | egui::Order::Foreground) && m.areas().is_visible(l))
            .filter_map(|l| m.area_rect(l.id))
            .filter(|r| r.area() < screen.area() * 0.95 && r.width() > 60.0)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
    })
}

fn wanted(name: &str) -> bool {
    std::env::var("MASSIF_AUDIT_ONLY").map_or(true, |w| w.split(',').any(|w| name.contains(w)))
}

fn save(h: &mut Harness<'_, MassifApp>, dir: &std::path::Path, name: &str, crop: Option<Rect>) {
    let image = (0..3).find_map(|_| h.render().ok()).expect("rendered");
    let image = match crop {
        Some(r) => {
            let s = ppp();
            let r = r.expand(6.0);
            let (w, hh) = (image.width() as f32, image.height() as f32);
            let x0 = (r.left() * s).floor().clamp(0.0, w - 1.0);
            let y0 = (r.top() * s).floor().clamp(0.0, hh - 1.0);
            let x1 = (r.right() * s).ceil().clamp(x0 + 1.0, w);
            let y1 = (r.bottom() * s).ceil().clamp(y0 + 1.0, hh);
            image::imageops::crop_imm(&image, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image()
        }
        None => image,
    };
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore]
fn ui_audit() {
    let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for compact in [false, true] {
        let mode = if compact { "compact" } else { "normal" };
        let mut h = harness(vec2(1440.0, 900.0), compact);
        // The workspaces, whole.
        let names: Vec<String> = h.state().layout.workspaces.iter().map(|w| w.name.clone()).collect();
        for (k, name) in names.iter().enumerate() {
            let file = format!("{mode}-workspace-{k:02}-{}", name.to_lowercase().replace(' ', "-"));
            if !wanted(&file) {
                continue;
            }
            h.state_mut().layout.activate(k);
            h.run_steps(40);
            save(&mut h, &dir, &file, None);
        }
        h.state_mut().layout.activate(0);
        h.run_steps(10);
        // Every page of Settings.
        let sections = [
            (PrefsSection::Appearance, "appearance"),
            (PrefsSection::Accessibility, "accessibility"),
            (PrefsSection::Typeface, "typeface"),
            (PrefsSection::Sounds, "sounds"),
            (PrefsSection::Notifications, "notifications"),
            (PrefsSection::Memory, "memory"),
            (PrefsSection::Autosave, "autosave"),
            (PrefsSection::Keyboard, "keyboard"),
            (PrefsSection::Updates, "updates"),
            (PrefsSection::Graphics, "graphics"),
            (PrefsSection::Export, "export"),
            (PrefsSection::Developer, "developer"),
        ];
        for (section, name) in sections {
            let file = format!("{mode}-settings-{name}");
            if !wanted(&file) {
                continue;
            }
            h.state_mut().dialog = Some(Dialog::Preferences(section));
            h.run_steps(12);
            let r = top_window(&h);
            save(&mut h, &dir, &file, r);
        }
        h.state_mut().dialog = None;
        // The dialogs of the menus that open no file picker and reach no
        // server.
        let dialogs = [
            (Command::NewWorld, "new-world"),
            (Command::WorldInfo, "world-info"),
            (Command::GameAndMods, "game-and-mods"),
            (Command::Export, "export"),
            (Command::ExportRegion, "export-region"),
            (Command::ExportSchematic, "export-schematic"),
            (Command::AutoRules, "auto-rules"),
            (Command::WholeMap(crate::reshape::WholeOp::Erosion), "erosion"),
            (Command::Snowfall, "snowfall"),
            (Command::SmoothTerrain, "smooth"),
            (Command::LayerCovers, "layer-covers"),
            (Command::ReplaceTerrain(None), "replace-terrain"),
            (Command::Caves, "caves"),
            (Command::Ores, "ores"),
            (Command::Analyse, "analysis"),
            (Command::Help, "help"),
            (Command::Shortcuts, "shortcuts"),
            (Command::About, "about"),
            (Command::ReportProblem, "report-problem"),
            (Command::RecordPerformance, "record-performance"),
        ];
        let ctx = h.ctx.clone();
        for (c, name) in dialogs {
            let file = format!("{mode}-dialog-{name}");
            if !wanted(&file) {
                continue;
            }
            h.state_mut().dialog = None;
            h.run_steps(2);
            h.state_mut().command(&ctx, c);
            h.run_steps(60);
            if name == "analysis" {
                // While it reads, then done.
                if let Some(Dialog::Analysis(v)) = &mut h.state_mut().dialog {
                    v.again(&ctx, generate(&GenParams { tiles_x: 16, tiles_z: 16, landform: Landform::Hills, ..GenParams::default() }), 0, 0);
                }
                h.run_steps(1);
                let r = top_window(&h);
                save(&mut h, &dir, &format!("{file}-reading"), r);
                let started = std::time::Instant::now();
                while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Analysis(v) if v.is_done())) {
                    assert!(started.elapsed().as_secs() < 60, "the analysis never came");
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                h.run_steps(5);
                for (k, tab) in ["survival", "relief", "blocks", "biomes"].into_iter().enumerate() {
                    if let Some(Dialog::Analysis(v)) = &mut h.state_mut().dialog {
                        v.set_tab(k);
                    }
                    h.run_steps(4);
                    let r = top_window(&h);
                    save(&mut h, &dir, &format!("{file}-{tab}"), r);
                }
                continue;
            }
            let r = top_window(&h);
            save(&mut h, &dir, &file, r);
        }
        h.state_mut().dialog = None;
        // The narrowest window.
        drop(h);
        let mut h = harness(vec2(960.0, 640.0), compact);
        let file = format!("{mode}-narrow");
        if wanted(&file) {
            h.run_steps(30);
            save(&mut h, &dir, &file, None);
        }
    }
}
