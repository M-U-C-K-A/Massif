//! The pictures of the manual (`docs/images/*.png`), rendered offscreen as
//! the app draws them: each sets up a state of the app (a workspace, a
//! tool in hand, a dialog open) and saves it, cropped to what the page
//! shows. Ignored: run them with
//! `MASSIF_SNAPSHOT_DIR=docs/images MASSIF_SNAPSHOT_PPP=2 cargo test -p massif-app doc_captures -- --ignored --test-threads=1`
//! (MASSIF_DOC_WORLD: a WorldPainter `.world` for the Import Report).

use super::*;
use egui::{Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use massif_core::generate::Landform;

fn ppp() -> f32 {
    std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0)
}

fn out_dir() -> PathBuf {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/images"))
}

/// The app over `world`, in the dark theme, still, in a window of `size`
/// points, the game's blocks read where the game is installed.
fn harness(world: World, size: Vec2) -> Harness<'static, MassifApp> {
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp()).with_accessibility_check(false).wgpu().build_eframe(move |cc| {
        theme::install(&cc.egui_ctx);
        let mut app = MassifApp::for_tests(world);
        app.settings.theme = ThemeChoice::Dark;
        app.settings.reduce_motion = Follow::On;
        if let Some(jar) = massif_assets::find_jars().first() {
            app.blocks.catalogue = massif_assets::load(&jar.path, &jar.version).ok().map(std::sync::Arc::new);
        }
        app
    });
    h.run_steps(5);
    h
}

fn world(landform: Landform, tiles: i32) -> World {
    generate(&GenParams { tiles_x: tiles, tiles_z: tiles, landform, ..GenParams::default() })
}

/// The rects of the areas of the workspace shown, with their editors.
fn areas(h: &Harness<'_, MassifApp>) -> Vec<(AreaId, EditorKind, Rect)> {
    let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap_or_default();
    areas.into_iter().filter_map(|(id, r)| h.state().layout.current().editor_of(id).map(|e| (id, e, r))).collect()
}

fn area(h: &Harness<'_, MassifApp>, kind: EditorKind) -> Rect {
    areas(h).into_iter().find(|a| a.1 == kind).unwrap_or_else(|| panic!("{kind:?} shown")).2
}

fn areas_of(h: &Harness<'_, MassifApp>, kinds: &[EditorKind]) -> Rect {
    kinds.iter().map(|k| area(h, *k)).reduce(|a, b| a.union(b)).unwrap()
}

/// The window over the others (a dialog, a menu): the largest of the
/// areas drawn above the panels, smaller than the screen.
fn top_window(h: &Harness<'_, MassifApp>) -> Option<Rect> {
    let screen = h.ctx.content_rect();
    h.ctx.memory(|m| {
        m.layer_ids()
            .filter(|l| matches!(l.order, egui::Order::Middle | egui::Order::Foreground) && m.areas().is_visible(l))
            .filter_map(|l| m.area_rect(l.id))
            .filter(|r| r.area() < screen.area() * 0.9 && r.width() > 60.0)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
    })
}

fn wait(h: &mut Harness<'_, MassifApp>, what: &str, done: impl Fn(&MassifApp) -> bool) {
    let started = std::time::Instant::now();
    while !done(h.state()) {
        assert!(started.elapsed().as_secs() < 180, "{what}: never came");
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(3);
}

fn settle_3d(h: &mut Harness<'_, MassifApp>) {
    let mut frames = 0;
    while frames < 5 || h.state().view3d.pending() > 0 {
        h.step();
        frames += 1;
        assert!(frames < 4000, "the 3D view never finished");
    }
    h.run_steps(20);
}

/// `img` as a PNG: in full colour, or in 256 colours (NeuQuant).
fn encode_png(img: &image::RgbaImage, quantised: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if !quantised {
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        return bytes;
    }
    let rgba = img.as_raw();
    let nq = color_quant::NeuQuant::new(10, 256, rgba);
    let indices: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| nq.index_of(p) as u8).collect();
    let map = nq.color_map_rgba();
    let palette: Vec<u8> = map.as_chunks::<4>().0.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    let alpha: Vec<u8> = map.as_chunks::<4>().0.iter().map(|c| c[3]).collect();
    let mut encoder = png::Encoder::new(&mut bytes, img.width(), img.height());
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette);
    encoder.set_trns(alpha);
    encoder.set_compression(png::Compression::High);
    encoder.write_header().unwrap().write_image_data(&indices).unwrap();
    bytes
}

/// The generator's previews and viewer made.
fn settle_generator(h: &mut Harness<'_, MassifApp>) {
    h.event(egui::Event::PointerGone);
    let started = std::time::Instant::now();
    let mut quiet = 0;
    while quiet < 40 && started.elapsed().as_secs() < 150 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let busy = h.state().terrain_panel.as_ref().is_some_and(|p| p.status().busy);
        quiet = if busy { 0 } else { quiet + 1 };
    }
    h.run_steps(10);
}

/// Saves the window as `docs/images/<name>.png`, cropped to `crop` (in
/// points; the whole window when `None`).
fn save(h: &mut Harness<'_, MassifApp>, name: &str, crop: Option<Rect>) {
    let image = (0..3).find_map(|_| h.render().ok()).expect("rendered");
    let image = match crop {
        Some(r) => {
            let s = ppp();
            let (w, hh) = (image.width() as f32, image.height() as f32);
            let x0 = (r.left() * s).floor().clamp(0.0, w - 1.0);
            let y0 = (r.top() * s).floor().clamp(0.0, hh - 1.0);
            let x1 = (r.right() * s).ceil().clamp(x0 + 1.0, w);
            let y1 = (r.bottom() * s).ceil().clamp(y0 + 1.0, hh);
            image::imageops::crop_imm(&image, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image()
        }
        None => image,
    };
    let dir = out_dir();
    let _ = std::fs::create_dir_all(&dir);
    // Kept light for the pages: a picture of more than 400 KB is written
    // in 256 colours, and made smaller, a tenth at a time, while it is
    // still heavier.
    let (w0, h0) = (image.width() as f32, image.height() as f32);
    let mut bytes = encode_png(&image, false);
    let mut scale = 1.0;
    if bytes.len() > 400_000 {
        bytes = encode_png(&image, true);
    }
    while bytes.len() > 400_000 && scale > 0.55 {
        scale -= 0.1;
        let small = image::imageops::resize(&image, (w0 * scale) as u32, (h0 * scale) as u32, image::imageops::FilterType::Lanczos3);
        bytes = encode_png(&small, true);
    }
    std::fs::write(dir.join(format!("{name}.png")), &bytes).unwrap();
    eprintln!("saved {name}.png ({:.0}×{:.0}, {} KB)", w0 * scale, h0 * scale, bytes.len() / 1000);
}

fn activate(h: &mut Harness<'_, MassifApp>, name: &str) {
    let index = h.state().layout.workspaces.iter().position(|w| w.name == name).unwrap_or_else(|| panic!("workspace {name}"));
    h.state_mut().layout.activate(index);
    h.run_steps(8);
}

fn map_id(h: &Harness<'_, MassifApp>) -> AreaId {
    h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view")
}

fn hover(h: &mut Harness<'_, MassifApp>, at: Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(4);
}

fn command(h: &mut Harness<'_, MassifApp>, c: Command) {
    let ctx = h.ctx.clone();
    h.state_mut().command(&ctx, c);
    h.run_steps(8);
}

/// Settings, the Appearance (home background), Graphics and Keyboard
/// sections.
#[test]
#[ignore]
fn doc_captures_settings() {
    let mut h = harness(world(Landform::Hills, 2), vec2(1280.0, 1700.0));
    h.state_mut().settings.backdrop.style = crate::backdrop::Style::Waves;
    for (section, name) in [(PrefsSection::Appearance, "settings-appearance"), (PrefsSection::Graphics, "settings-graphics"), (PrefsSection::Keyboard, "settings-keyboard")] {
        h.state_mut().dialog = Some(Dialog::Preferences(section));
        h.run_steps(12);
        if section == PrefsSection::Appearance {
            h.get_by_label("Waves").scroll_to_me();
            h.run_steps(12);
        }
        let r = top_window(&h);
        save(&mut h, name, r);
    }
}

/// The dialogs of Edit ▸ Whole map, Caves, Ores and the survival check
/// over a mountain map.
#[test]
#[ignore]
fn doc_captures_whole_map_dialogs() {
    let mut h = harness(world(Landform::Mountains, 4), vec2(1440.0, 940.0));
    command(&mut h, Command::WholeMap(crate::reshape::WholeOp::Erosion));
    h.run_steps(60);
    let r = top_window(&h);
    save(&mut h, "erosion-dialog", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::Snowfall);
    wait(&mut h, "the snow's picture", |a| matches!(&a.dialog, Some(Dialog::Snowfall(f)) if f.has_picture()));
    let r = top_window(&h);
    save(&mut h, "snowfall-dialog", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::Caves);
    h.run_steps(60);
    let r = top_window(&h);
    save(&mut h, "caves-dialog", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::Ores);
    h.run_steps(60);
    let r = top_window(&h);
    save(&mut h, "ores-dialog", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::Analyse);
    h.run_steps(120);
    let r = top_window(&h);
    save(&mut h, "survival-check", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::ReportProblem);
    h.run_steps(20);
    let r = top_window(&h);
    save(&mut h, "report-problem", r);
    h.state_mut().dialog = None;
    command(&mut h, Command::GameAndMods);
    h.run_steps(20);
    let r = top_window(&h);
    save(&mut h, "game-and-mods", r);
}

/// The panels of the World side's workspaces.
#[test]
#[ignore]
fn doc_captures_panels() {
    let mut h = harness(world(Landform::Hills, 4), vec2(1440.0, 900.0));
    activate(&mut h, "Layout");
    h.state_mut().tools.select(Tool::Raise);
    let id = map_id(&h);
    let map = areas(&h).into_iter().find(|a| a.0 == id).unwrap().2;
    hover(&mut h, map.center());
    save(&mut h, "first-map", None);
    let r = area(&h, EditorKind::Tools);
    save(&mut h, "tools-panel", Some(r));
    let r = areas_of(&h, &[EditorKind::Brushes, EditorKind::BrushSettings]);
    save(&mut h, "brushes", Some(r));
    activate(&mut h, "Painting");
    h.state_mut().tools.select(Tool::PaintTerrain);
    h.run_steps(8);
    save(&mut h, "painting-workspace", None);
    activate(&mut h, "3D");
    settle_3d(&mut h);
    save(&mut h, "view3d", None);
}

/// File ▸ Run Script… after a run, the window that offers a backup after
/// a forced quit, and the export dialog's Rules and test pages.
#[test]
#[ignore]
fn doc_captures_files() {
    let mut h = harness(world(Landform::Mountains, 4), vec2(1280.0, 900.0));
    let dir = std::env::temp_dir().join(format!("massif-doc-captures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A script with a parameter, run.
    let script = dir.join("land-above.lua");
    std::fs::write(&script, "-- script.name=Land above\n-- script.param.above.type=integer\n-- script.param.above.default=90\n-- script.param.above.description=Height to count from\nlocal map, above = massif.map, massif.params.above\nlocal high, all = 0, 0\nfor x0, z0, x1, z1 in map:tiles() do\n    local h = map:get(\"height\", x0, z0, x1, z1)\n    for i = 1, #h do\n        if h[i] then\n            all += 1\n            if h[i] > above then high += 1 end\n        end\n    end\nend\nprint(string.format(\"%.1f %% of the land is above y %d\", 100 * high / all, above))\n").unwrap();
    let mut form = crate::run_script::ScriptForm::new(script);
    form.start(h.state().doc.world.copy_unfrozen(), 0, None);
    h.state_mut().dialog = Some(Dialog::Script(Box::new(form)));
    wait(&mut h, "the script's run", |a| matches!(&a.dialog, Some(Dialog::Script(f)) if !f.running()));
    h.run_steps(10);
    let r = top_window(&h);
    save(&mut h, "run-script", r);
    h.state_mut().dialog = None;
    // A backup left by a forced quit.
    let mut lost = world(Landform::Hills, 2);
    lost.meta.name = "Northern valleys".into();
    let taken = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - 600;
    let info = crate::autosave::Info { session: "1-1".into(), generation: 1, name: "Northern valleys".into(), path: Some(PathBuf::from("/Users/me/Maps/Northern valleys.massif")), taken, revision: 3 };
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    crate::autosave::write(&backups, lost.copy_unfrozen(), info).unwrap();
    h.state_mut().autosave = crate::autosave::Autosave::open(backups);
    h.state_mut().dialog = Some(Dialog::Recover);
    h.run_steps(10);
    let r = top_window(&h);
    save(&mut h, "restore-unsaved", r);
    h.state_mut().dialog = None;
    // The export dialog: its Rules, Creative / Frozen chosen.
    command(&mut h, Command::Export);
    fn export<'a>(h: &'a mut Harness<'_, MassifApp>) -> &'a mut ExportForm {
        match &mut h.state_mut().dialog {
            Some(Dialog::Export(form)) => form,
            _ => panic!("the export dialog"),
        }
    }
    export(&mut h).parent = PathBuf::from("/Users/me/Library/Application Support/minecraft/saves");
    export(&mut h).name = "Northern valleys".into();
    export(&mut h).tab = ExportTab::Rules;
    export(&mut h).target = massif_anvil::Target::release("1.16.5").unwrap();
    h.run_steps(6);
    h.get_by_label("Creative / Frozen").hover();
    h.run_steps(2);
    h.get_by_label("Creative / Frozen").click();
    h.run_steps(6);
    let r = top_window(&h);
    save(&mut h, "export-rules", r);
    export(&mut h).tab = ExportTab::General;
    export(&mut h).target = massif_anvil::Target::release("1.21.11").unwrap();
    export(&mut h).testing = true;
    h.event(egui::Event::PointerGone);
    export(&mut h).test_area = TestArea::Regions;
    export(&mut h).test_regions = [(0, 0), (-1, 0), (0, -1)].into_iter().collect();
    h.run_steps(10);
    let r = top_window(&h);
    save(&mut h, "export-test-regions", r);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Terrain Generator over the Alpine range sample: its workspace, the
/// add-node search, the composite opened, a node's settings, the viewer,
/// the Script pane.
#[test]
#[ignore]
fn doc_captures_generator() {
    let mut h = harness(world(Landform::Hills, 8), vec2(1440.0, 900.0));
    h.state_mut().layout.show_generator();
    h.run_steps(10);
    let graph = massif_terrain::model::Graph::from_json(
        r##"{ "nodes": {
            "range": { "type": "mountain.range", "params": { "height": 180, "base": 70 } },
            "relief": { "type": "combine.blend", "inputs": { "b": "range" }, "params": { "mode": "max" } },
            "out": { "type": "output.map", "inputs": { "height": "relief" } }
        },
        "ui": { "range": { "pos": [0, 240] }, "relief": { "pos": [260, 80] }, "out": { "pos": [500, 80] } } }"##,
    )
    .unwrap();
    let composite = {
        let p = h.state_mut().terrain_panel.as_mut().expect("the generator");
        p.set_graph(graph);
        // As from the library: the latest Alpine Range.
        let alps = p.add_node("massif:alpine-range@2", [0.0, 0.0]);
        p.connect(&alps, 0, "relief", "a");
        p.connect(&alps, 1, "out", "mask.snow");
        p.fit_pending = true;
        p.look.three_d = true;
        alps
    };
    h.run_steps(10);
    h.state_mut().terrain_panel.as_mut().unwrap().selected = vec![composite.clone()];
    settle_generator(&mut h);
    save(&mut h, "generator-workspace", None);
    let viewer = area(&h, EditorKind::GenViewer);
    save(&mut h, "generator-viewer", Some(viewer));
    h.state_mut().terrain_panel.as_mut().unwrap().selected = vec!["range".into()];
    h.run_steps(20);
    let props = area(&h, EditorKind::GenProps);
    save(&mut h, "generator-node-settings", Some(props));
    h.state_mut().terrain_panel.as_mut().unwrap().selected = vec![composite.clone()];
    h.run_steps(10);
    // The add-node search.
    let graph = area(&h, EditorKind::GenGraph);
    hover(&mut h, graph.center());
    h.state_mut().terrain_panel.as_mut().unwrap().search = Some(crate::terrain::panel::Search { at: [260.0, 300.0], screen: graph.center() - vec2(120.0, 140.0), query: "therm".into(), ..Default::default() });
    h.run_steps(8);
    save(&mut h, "generator-add-node", Some(graph));
    h.state_mut().terrain_panel.as_mut().unwrap().search = None;
    h.event(egui::Event::PointerGone);
    // The composite opened.
    h.state_mut().terrain_panel.as_mut().unwrap().open(&composite);
    settle_generator(&mut h);
    save(&mut h, "generator-composite", Some(graph));
    h.state_mut().terrain_panel.as_mut().unwrap().close_to(0);
    h.run_steps(5);
    // The Script pane.
    h.state_mut().terrain_panel.as_mut().unwrap().script.open = true;
    h.run_steps(20);
    save(&mut h, "generator-script", Some(graph));
}

/// A map with a little of everything painted: trees, a ground cover,
/// pockets, annotations and stone, as the panels list them.
fn painted_world(tiles: i32) -> World {
    let mut world = world(Landform::Hills, tiles);
    let reg = &mut world.registry;
    let trees = reg.add_object_layer(massif_core::objects::ObjectLayerDef::new("Oak wood", &[], reg));
    let pockets = reg.add_pocket(massif_core::pockets::PocketDef::new("Iron veins", [180, 120, 90], &[], 0.2, 4.0, 0, 60));
    let ground = world.dims[0].ground_mut();
    for z in -40..40 {
        for x in -40..40 {
            let Some(tile) = ground.tile_mut(massif_core::TilePos::containing(x, z)) else { continue };
            let i = massif_core::coords::local_index(x, z);
            tile.objects.set(i, trees);
            if (x + z).rem_euclid(5) == 0 {
                tile.cover.set(i, 1);
            }
            if x > 20 {
                tile.pockets.set(i, pockets);
            }
            if z < -30 {
                tile.annotations.set(i, 4);
            }
        }
    }
    world
}

/// Each tool in hand over the map (Layout workspace): its Tool Settings
/// beside the map.
#[test]
#[ignore]
fn doc_captures_tools() {
    let mut h = harness(painted_world(4), vec2(1440.0, 900.0));
    activate(&mut h, "Layout");
    let id = map_id(&h);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [0.0, 0.0];
        view.zoom = 1.5;
    }
    h.run_steps(5);
    let map = areas(&h).into_iter().find(|a| a.0 == id).unwrap().2;
    // The map's header, its modes and overlays.
    let header = Rect::from_min_size(map.min, vec2(map.width(), 44.0));
    save(&mut h, "map-header", Some(header));
    // The Info panel, the pointer over the trees.
    hover(&mut h, map.center() + vec2(-10.0, -10.0));
    let r = area(&h, EditorKind::Info);
    save(&mut h, "info-panel", Some(r));
    for (tool, name) in [
        (Tool::Annotate, "annotations-tool"),
        (Tool::Flood, "flood-tool"),
        (Tool::River, "river-tool"),
        (Tool::Road, "road-tool"),
        (Tool::Structure, "structures-tool"),
        (Tool::Gradient, "gradient-brush"),
        (Tool::AutoBiome, "auto-biome"),
        (Tool::SelectLasso, "select-tools"),
        (Tool::Layer, "layer-tool"),
    ] {
        h.state_mut().tools.select(tool);
        if let Some(view) = h.state_mut().maps.get_mut(&id) {
            view.cut_y = 62;
        }
        h.run_steps(6);
        hover(&mut h, map.center() + vec2(60.0, 40.0));
        h.run_steps(6);
        let r = areas_of(&h, &[EditorKind::Tools, EditorKind::ToolSettings, EditorKind::Map]);
        save(&mut h, name, Some(r));
    }
    // Paint terrain in Noise mode.
    {
        let app = h.state_mut();
        app.tools.select(Tool::PaintTerrain);
        let ps = &mut app.tools.pattern;
        ps.on = true;
        ps.terrains = vec![("Grass".into(), 0.5), ("Coarse Dirt".into(), 0.3), ("Podzol".into(), 0.2)];
    }
    h.run_steps(10);
    let r = areas_of(&h, &[EditorKind::Tools, EditorKind::ToolSettings, EditorKind::Map]);
    save(&mut h, "noise-painting", Some(r));
    h.state_mut().tools.pattern.on = false;
    h.state_mut().tools.select(Tool::Raise);
    h.event(egui::Event::PointerGone);
    h.run_steps(5);
    // The panels of the right-hand column, one at a time in the 3D View's
    // place.
    let side = areas(&h).into_iter().find(|a| a.1 == EditorKind::View3d).expect("the 3D View").0;
    for (kind, name) in [(EditorKind::Layers, "layers-panel"), (EditorKind::Packs, "packs-panel"), (EditorKind::Levels, "levels-panel"), (EditorKind::Blocks, "blocks-panel"), (EditorKind::Versions, "versions-panel")] {
        h.state_mut().layout.current_mut().root.set_editor(side, kind);
        h.run_steps(15);
        let r = area(&h, kind);
        save(&mut h, name, Some(r));
    }
    // The block palettes.
    h.state_mut().open_palettes(None);
    h.run_steps(10);
    let r = top_window(&h);
    save(&mut h, "palettes", r);
    h.state_mut().dialog = None;
    // The Render side's Still workspace, its picture settled.
    h.state_mut().layout.show_side(crate::layout::Side::Render);
    h.run_steps(5);
    let started = std::time::Instant::now();
    while (h.state().render_side.building() || h.state().render_side.samples() < 64) && started.elapsed().as_secs() < 150 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    h.run_steps(4);
    save(&mut h, "render-still", None);
}

/// The home screen's page of a height map, and the report of a
/// WorldPainter world imported (MASSIF_DOC_WORLD: a `.world`).
#[test]
#[ignore]
fn doc_captures_import() {
    let mut h = harness(world(Landform::Hills, 2), vec2(1440.0, 900.0));
    h.state_mut().open_home(Page::FromHeightmap);
    h.run_steps(15);
    save(&mut h, "from-height-map", None);
    h.state_mut().home = None;
    h.run_steps(5);
    if let Some(path) = std::env::var_os("MASSIF_DOC_WORLD") {
        let (world, report) = massif_legacy::worldpainter::import_world(std::path::Path::new(&path), None).expect("imported");
        {
            let app = h.state_mut();
            app.doc = crate::doc::Document::new(world, None, 41);
            app.doc.world.meta.import_report = serde_json::to_value(&report).ok();
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
        }
        h.run_steps(5);
        command(&mut h, Command::ImportReport);
        h.run_steps(10);
        let r = top_window(&h);
        save(&mut h, "import-report", r);
    }
}
