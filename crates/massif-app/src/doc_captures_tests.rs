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
    top_layer(h, &[egui::Order::Middle, egui::Order::Foreground])
}

/// The largest visible area of these orders, smaller than the screen.
fn top_layer(h: &Harness<'_, MassifApp>, orders: &[egui::Order]) -> Option<Rect> {
    let screen = h.ctx.content_rect();
    h.ctx.memory(|m| {
        m.layer_ids()
            .filter(|l| orders.contains(&l.order) && m.areas().is_visible(l))
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
    while bytes.len() > 400_000 && scale > 0.25 {
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
    let mut h = harness(world(Landform::Hills, 4), vec2(1440.0, 1100.0));
    activate(&mut h, "Layout");
    h.state_mut().tools.select(Tool::Raise);
    let id = map_id(&h);
    let map = areas(&h).into_iter().find(|a| a.0 == id).unwrap().2;
    hover(&mut h, map.center());
    save(&mut h, "first-map", None);
    let gradient = h.query_all_by_label_contains("Gradient brush").map(|n| n.rect()).next();
    if let Some(r) = gradient {
        hover(&mut h, r.center());
        for _ in 0..40 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    let r = area(&h, EditorKind::Tools);
    let card = top_layer(&h, &[egui::Order::Tooltip]).map_or(r, |c| c.union(r));
    save(&mut h, "tools-panel", Some(card));
    hover(&mut h, map.center());
    h.state_mut().tools.filter.above = Some(66);
    h.state_mut().tools.filter.below = Some(90);
    h.run_steps(8);
    // Brush Settings alone in the window, Height limits in sight.
    let settings = areas(&h).into_iter().find(|a| a.1 == EditorKind::BrushSettings).unwrap().0;
    h.state_mut().layout.toggle_maximize(settings);
    h.run_steps(10);
    let r = area(&h, EditorKind::BrushSettings);
    save(&mut h, "brushes", Some(Rect::from_min_size(r.min, vec2(r.width().min(560.0), r.height().min(1000.0)))));
    h.state_mut().layout.toggle_maximize(settings);
    h.run_steps(5);
    h.state_mut().tools.filter.above = None;
    h.state_mut().tools.filter.below = None;
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
            "north": { "type": "noise.fbm", "name": "Higher in the north", "params": { "scale": 900, "height": 1, "base": 0, "octaves": 3 } },
            "range": { "type": "mountain.range", "inputs": { "@height": "north" }, "params": { "height": 180, "base": 70 } },
            "relief": { "type": "combine.blend", "inputs": { "b": "range" }, "params": { "mode": "max" } },
            "out": { "type": "output.map", "inputs": { "height": "relief" } }
        },
        "ui": { "north": { "pos": [-260, 240] }, "range": { "pos": [0, 240] }, "relief": { "pos": [260, 80] }, "out": { "pos": [500, 80] } } }"##,
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
    h.state_mut().terrain_panel.as_mut().unwrap().script.open = false;
}

/// An oak of `trunk` blocks under a crown of `crown` blocks' radius.
fn tree(name: &str, trunk: usize, crown: usize) -> massif_core::objects::ObjectDef {
    let w = 2 * crown + 1;
    let h = trunk + crown + 1;
    let palette = vec!["minecraft:air".to_owned(), "minecraft:oak_log[axis=y]".to_owned(), "minecraft:oak_leaves[distance=1,persistent=true]".to_owned()];
    let mut blocks = vec![0u16; w * h * w];
    let at = |x: usize, y: usize, z: usize| x + z * w + y * w * w;
    for y in 0..trunk {
        blocks[at(crown, y, crown)] = 1;
    }
    let c = crown as f32;
    for y in trunk - 1..h {
        for z in 0..w {
            for x in 0..w {
                let d = ((x as f32 - c).powi(2) + (z as f32 - c).powi(2) + ((y as f32 - (trunk as f32 + 0.5)) * 1.2).powi(2)).sqrt();
                if d <= c + 0.3 && blocks[at(x, y, z)] == 0 {
                    blocks[at(x, y, z)] = 2;
                }
            }
        }
    }
    massif_core::objects::ObjectDef::new(name, [w as u16, h as u16, w as u16], palette, blocks)
}

/// A map with a little of everything painted: trees, a ground cover,
/// pockets, annotations and stone, as the panels list them.
fn painted_world(tiles: i32) -> World {
    let mut world = world(Landform::Hills, tiles);
    let reg = &mut world.registry;
    let first = reg.objects.len() as u32;
    for (name, trunk, crown) in [("Oak, small", 4, 2), ("Oak, tall", 7, 3), ("Oak, broad", 5, 4)] {
        reg.objects.push(tree(name, trunk, crown));
    }
    let trees = reg.add_object_layer(massif_core::objects::ObjectLayerDef::new("Oak wood", &[first, first + 1, first + 2], reg));
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
        (Tool::River, "river-tool"),
        (Tool::Road, "road-tool"),
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
        if tool == Tool::SelectLasso {
            // A loop drawn round a hill.
            let c = map.center();
            let at = |k: usize| c + vec2((k as f32 * 0.21).cos() * (150.0 + 30.0 * (k as f32 * 0.9).sin()), (k as f32 * 0.21).sin() * (110.0 + 20.0 * (k as f32 * 1.3).cos()));
            hover(&mut h, at(0));
            press(&mut h, at(0), true);
            for k in 1..=30 {
                h.event(egui::Event::PointerMoved(at(k)));
                h.step();
            }
            press(&mut h, at(30), false);
            h.run_steps(10);
        }
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
    // Three save points, the land changed between them.
    let ctx = h.ctx.clone();
    for (k, name) in ["First sketch", "Before the rivers", "Rivers in"].iter().enumerate() {
        if k > 0 {
            let app = h.state_mut();
            app.doc.history.begin("Raise", 0);
            let rect = massif_core::BlockRect::new(-60 + 40 * k as i32, -40, 20 + 40 * k as i32, 40);
            let mut edit = app.doc.history.edit(&mut app.doc.world, 0);
            edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
                for z in clip.z0..clip.z1 {
                    for x in clip.x0..clip.x1 {
                        tile.top.set(massif_core::coords::local_index(x, z), 84.0 + 4.0 * k as f32);
                    }
                }
            });
            app.doc.commit();
        }
        h.state_mut().command(&ctx, Command::SavePoint);
        h.run_steps(3);
        if let Some(Dialog::PointName(None, n)) = &mut h.state_mut().dialog {
            *n = (*name).into();
        }
        h.run_steps(2);
        h.get_by_label("Save Point").click();
        h.run_steps(2);
        wait(&mut h, "the save point", |a| a.jobs.is_empty());
    }
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
    let png = std::env::temp_dir().join(format!("massif-doc-heightmap-{}.png", std::process::id()));
    let (w, d) = (512usize, 512usize);
    let samples: Vec<f32> = (0..w * d)
        .map(|k| {
            let (x, z) = ((k % w) as f32 / w as f32, (k / w) as f32 / d as f32);
            let ridge = (1.0 - ((x * 6.0 + (z * 9.0).sin() * 0.6).sin()).abs()).powi(2);
            (0.25 + 0.55 * ridge * (1.0 - (z - 0.5).abs()) + 0.08 * ((x * 23.0).sin() * (z * 19.0).cos())).clamp(0.0, 1.0)
        })
        .collect();
    massif_core::heightmap::write_png16(&png, w, d, &samples).unwrap();
    h.state_mut().open_home(Page::FromHeightmap);
    h.run_steps(5);
    let ctx = h.ctx.clone();
    let pick = crate::home::HeightmapPick::read(&ctx, png.clone()).expect("the height map read");
    h.state_mut().home.as_mut().unwrap().heightmap = Some(pick);
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

fn press(h: &mut Harness<'_, MassifApp>, at: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    h.step();
}

/// The window: the panel chooser, an area dragged, a reference image, the
/// settings of a ground cover and of an object layer.
#[test]
#[ignore]
fn doc_captures_window() {
    let mut h = harness(painted_world(4), vec2(1440.0, 900.0));
    activate(&mut h, "Layout");
    // The panel chooser, from the icon before the Brushes area's title,
    // the pointer over Layers.
    let brushes = area(&h, EditorKind::Brushes);
    let icon = brushes.min + vec2(22.0, 22.0);
    hover(&mut h, icon);
    press(&mut h, icon, true);
    press(&mut h, icon, false);
    h.run_steps(6);
    let popup = top_window(&h).expect("the panel chooser");
    let layers = h.query_all_by_label("Layers").map(|n| n.rect()).rfind(|r| popup.contains(r.center()));
    if let Some(r) = layers {
        hover(&mut h, r.center());
    }
    h.run_steps(4);
    let popup = top_window(&h).expect("the panel chooser");
    save(&mut h, "panel-chooser", Some(popup.expand(8.0)));
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    // The Brushes area dragged by its title over the 3D View.
    let view = area(&h, EditorKind::View3d);
    let title = brushes.min + vec2(80.0, 22.0);
    hover(&mut h, title);
    press(&mut h, title, true);
    for k in 1..=16 {
        h.event(egui::Event::PointerMoved(title + (view.center() + vec2(0.0, 60.0) - title) * (k as f32 / 16.0)));
        h.step();
    }
    h.run_steps(4);
    save(&mut h, "area-drag", None);
    // Let go where it was taken: nothing moves.
    for k in 1..=16 {
        h.event(egui::Event::PointerMoved(view.center() + (title - view.center()) * (k as f32 / 16.0)));
        h.step();
    }
    press(&mut h, title, false);
    h.run_steps(4);
    // A plan laid over the map at half opacity, its handles out.
    let dir = std::env::temp_dir().join(format!("massif-doc-window-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plan = dir.join("plan.png");
    image::RgbaImage::from_fn(600, 400, |x, y| {
        let river = ((x as f32 * 0.02).sin() * 40.0 + 200.0 - y as f32).abs() < 14.0;
        if river {
            image::Rgba([40, 110, 220, 255])
        } else if x % 60 < 5 || y % 50 < 5 {
            image::Rgba([200, 40, 40, 255])
        } else {
            image::Rgba([245, 240, 225, 255])
        }
    })
    .save(&plan)
    .unwrap();
    let id = map_id(&h);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [150.0, 100.0];
        view.zoom = 1.2;
    }
    h.state_mut().add_references(std::slice::from_ref(&plan), Some((0.0, 0.0)));
    h.run_steps(5);
    h.state_mut().doc.world.meta.references[0].opacity = 0.5;
    h.state_mut().references.placing = true;
    h.event(egui::Event::PointerGone);
    h.run_steps(10);
    let map = areas(&h).into_iter().find(|a| a.0 == id).unwrap().2;
    save(&mut h, "reference-image", Some(map));
    h.state_mut().references.placing = false;
    h.state_mut().doc.world.meta.references.clear();
    h.run_steps(5);
    // A ground cover's settings, and an object layer's.
    for (kind, number, name) in [(crate::layers::Kind::Cover, 1, "ground-cover"), (crate::layers::Kind::Objects, 1, "object-layer")] {
        h.state_mut().dialog = Some(Dialog::Layer(kind, number));
        h.run_steps(20);
        let r = top_window(&h);
        save(&mut h, name, r);
        h.state_mut().dialog = None;
        h.run_steps(3);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Caves carved and seen through the ground in the 3D View (X-ray), then a
/// lake flooded in a hollow seen in the 3D View.
#[test]
#[ignore]
fn doc_captures_3d() {
    let mut h = harness(world(Landform::Mountains, 4), vec2(1440.0, 900.0));
    h.state_mut().open_caves();
    wait(&mut h, "the caves' pictures", |a| matches!(&a.dialog, Some(Dialog::Caves(f)) if f.has_pictures()));
    let r = top_window(&h);
    save(&mut h, "caves-dialog", r);
    h.get_by_label("Paint the caves").click();
    h.run_steps(4);
    activate(&mut h, "3D");
    h.state_mut().view3d.xray = true;
    settle_3d(&mut h);
    h.event(egui::Event::PointerGone);
    h.run_steps(5);
    let r = area(&h, EditorKind::View3d);
    save(&mut h, "xray", Some(r));
    h.state_mut().view3d.xray = false;
    h.run_steps(3);
    // Flood in a hollow, from the map.
    activate(&mut h, "Layout");
    let id = map_id(&h);
    let (x, z) = {
        let dim = h.state().doc.dimension();
        let sea = h.state().doc.world.meta.sea_level as f32;
        let height = |x: i32, z: i32| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap_or(0.0);
        // The lowest dry spot whose ring stands higher round it.
        let mut best = ((0, 0), f32::MAX);
        for z in (-240..240).step_by(8) {
            for x in (-240..240).step_by(8) {
                let here = height(x, z);
                let ring = (0..16).map(|k| (k as f32 * std::f32::consts::TAU / 16.0).sin_cos()).all(|(s, c)| height(x + (c * 24.0) as i32, z + (s * 24.0) as i32) > here + 3.0);
                if here > sea + 4.0 && ring && here < best.1 {
                    best = ((x, z), here);
                }
            }
        }
        best.0
    };
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [x as f64, z as f64];
        view.zoom = 3.0;
    }
    h.state_mut().tools.select(Tool::Flood);
    h.run_steps(5);
    let at = h.state().maps[&id].screen_of(x as f64 + 0.5, z as f64 + 0.5);
    hover(&mut h, at);
    h.run_steps(5);
    let r = areas_of(&h, &[EditorKind::Tools, EditorKind::ToolSettings, EditorKind::Map]);
    save(&mut h, "flood-tool", Some(r));
    super::tests::click_on(&mut h, at, egui::PointerButton::Primary);
    wait(&mut h, "the flood", |a| a.jobs.is_empty());
    if matches!(h.state().dialog, Some(Dialog::FloodAsk(_))) {
        h.state_mut().dialog = None;
    }
    activate(&mut h, "3D");
    let top = massif_core::ops::height_at(h.state().doc.dimension(), 0, x, z, massif_core::ops::Surface::Top).unwrap_or(80.0);
    h.state_mut().view3d.camera = crate::view3d::Camera { target: [x as f64, top as f64, z as f64], yaw: 0.7, pitch: 0.5, distance: 160.0 };
    settle_3d(&mut h);
    h.event(egui::Event::PointerGone);
    h.run_steps(5);
    let r = area(&h, EditorKind::View3d);
    save(&mut h, "flood-lake", Some(r));
    // 3D Sculpt's Player mode, in Ghost, among the hills.
    activate(&mut h, "3D Sculpt");
    h.state_mut().tools.sculpt.player = true;
    h.run_steps(10);
    let view = area(&h, EditorKind::Sculpt3d);
    hover(&mut h, view.center());
    h.state_mut().sculpt3d.ghost_for_tests();
    settle_3d(&mut h);
    hover(&mut h, view.center());
    h.run_steps(10);
    save(&mut h, "player-ghost", Some(view));
    h.state_mut().tools.sculpt.player = false;
    h.run_steps(5);
}

/// The Diorama over the sample village, then its caves under a height.
#[test]
#[ignore]
fn doc_captures_diorama() {
    let mut h = harness(massif_render::diorama::sample::village(), vec2(1440.0, 900.0));
    let id = map_id(&h);
    h.state_mut().layout.current_mut().root.set_editor(id, EditorKind::Diorama);
    h.run_steps(3);
    let settle = |h: &mut Harness<'_, MassifApp>| {
        let started = std::time::Instant::now();
        while !h.state().diorama.settled() && started.elapsed().as_secs() < 150 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        h.run_steps(6);
    };
    settle(&mut h);
    {
        let d = &mut h.state_mut().diorama;
        d.center = Some([massif_render::diorama::sample::VILLAGE.0 as f64, massif_render::diorama::sample::VILLAGE.1 as f64]);
        d.zoom = 2.0;
    }
    settle(&mut h);
    h.event(egui::Event::PointerGone);
    h.run_steps(4);
    let r = area(&h, EditorKind::Diorama);
    save(&mut h, "diorama", Some(r));
    h.state_mut().diorama.caves = true;
    h.state_mut().diorama.slice = 40;
    settle(&mut h);
    let r = area(&h, EditorKind::Diorama);
    save(&mut h, "diorama-caves", Some(r));
}

/// A village placed with the Structures tool, then one of its houses
/// chosen with the Building tool.
#[test]
#[ignore]
fn doc_captures_buildings() {
    let packs = std::env::temp_dir().join(format!("massif-doc-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&packs);
    let pack = massif_structures::village::testpack::write_pack(&packs).unwrap();
    let mut h = harness(world(Landform::Hills, 4), vec2(1440.0, 900.0));
    activate(&mut h, "Layout");
    {
        let s = h.state_mut();
        s.tools.select(Tool::Structure);
        s.tools.village.packs = vec![pack.clone()];
        s.tools.village.theme = ("Plains (test boxes)".into(), "Plains".into());
        s.tools.structure.village = true;
    }
    let id = map_id(&h);
    let spot = {
        let dim = h.state().doc.dimension();
        let sea = h.state().doc.world.meta.sea_level as f32;
        let height = |x: i32, z: i32| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap_or(0.0);
        let mut best = ((0, 0), 0);
        for z in (-200..200).step_by(24) {
            for x in (-200..200).step_by(24) {
                let dry = (-96..=96).step_by(24).flat_map(|dz| (-96..=96).step_by(24).map(move |dx| (dx, dz))).filter(|&(dx, dz)| height(x + dx, z + dz) > sea + 2.0).count();
                if dry > best.1 {
                    best = ((x, z), dry);
                }
            }
        }
        best.0
    };
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [spot.0 as f64, spot.1 as f64];
        view.zoom = 1.6;
    }
    h.run_steps(4);
    let at = h.state().maps[&id].screen_of(spot.0 as f64, spot.1 as f64);
    super::tests::click_on(&mut h, at, egui::PointerButton::Primary);
    let started = std::time::Instant::now();
    loop {
        h.step();
        let busy = !h.state().jobs.is_empty() || h.state().tools.village.rt.is_busy() || h.state().tools.structure.rt.is_busy() || h.state().tools.structure.rt.regen_waiting();
        if !busy || started.elapsed().as_secs() > 120 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.event(egui::Event::PointerGone);
    h.run_steps(6);
    let r = areas_of(&h, &[EditorKind::Tools, EditorKind::ToolSettings, EditorKind::Map]);
    save(&mut h, "structures-tool", Some(r));
    // A house chosen with the Building tool.
    h.state_mut().tools.select(Tool::Building);
    h.run_steps(3);
    let house = h.state().doc.dimension().structures[0].pieces.iter().find(|p| p.template == "house" && p.key.is_some()).cloned().expect("a house");
    let (mx, mz) = ((house.min[0] + house.max[0] + 1) as f64 / 2.0, (house.min[2] + house.max[2] + 1) as f64 / 2.0);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [mx, mz];
        view.zoom = 6.0;
    }
    h.run_steps(4);
    let at = h.state().maps[&id].screen_of(mx, mz);
    super::tests::click_on(&mut h, at, egui::PointerButton::Primary);
    h.event(egui::Event::PointerGone);
    h.run_steps(6);
    let r = areas_of(&h, &[EditorKind::Tools, EditorKind::ToolSettings, EditorKind::Map]);
    save(&mut h, "building-tool", Some(r));
    let _ = std::fs::remove_dir_all(&packs);
}

/// Edit ▸ Import Mask… with two pictures, then a zone sent out and brought
/// back changed: its review on the map and in the Versions panel.
#[test]
#[ignore]
fn doc_captures_masks_and_review() {
    let mut h = harness(world(Landform::Hills, 8), vec2(1440.0, 900.0));
    activate(&mut h, "Layout");
    let ctx = h.ctx.clone();
    let dir = std::env::temp_dir().join(format!("massif-doc-review-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Two masks to paint with.
    let (w, d) = (256u32, 256u32);
    let forest = dir.join("forest.png");
    image::GrayImage::from_fn(w, d, |x, y| image::Luma([(((x as f32 * 0.05).sin() * (y as f32 * 0.04).cos() * 0.5 + 0.5) * 255.0) as u8])).save(&forest).unwrap();
    let rock = dir.join("rock.png");
    image::GrayImage::from_fn(w, d, |x, y| image::Luma([if (x as i32 - 128).pow(2) + (y as i32 - 128).pow(2) < 60 * 60 { 255 } else { 0 }])).save(&rock).unwrap();
    {
        let app = h.state_mut();
        app.jobs.push(Job::read_mask(&ctx, vec![forest, rock], None, app.doc.dim));
    }
    wait(&mut h, "the masks read", |a| a.jobs.is_empty());
    h.run_steps(10);
    let r = top_window(&h);
    save(&mut h, "import-mask", r);
    h.state_mut().dialog = None;
    h.run_steps(3);
    // A zone sent out, a hill raised in it elsewhere while the owner
    // flattened a part of it.
    let side = areas(&h).into_iter().find(|a| a.1 == EditorKind::View3d).expect("the 3D View").0;
    h.state_mut().layout.current_mut().root.set_editor(side, EditorKind::Versions);
    let out = dir.join("zone");
    let rect = massif_core::BlockRect::new(-200, -150, 100, 110);
    {
        let app = h.state_mut();
        app.doc.world.map_id();
        let snapshot = app.doc.world.snapshot();
        app.jobs.push(Job::export_region(&ctx, snapshot, app.doc.dim, rect, out.clone(), massif_core::mask::RegionParts::default(), Some((None, "Before the zone".into()))));
    }
    wait(&mut h, "the zone written", |a| a.jobs.is_empty());
    let height = out.join("height.png");
    let mut img = image::open(&height).unwrap().into_luma16();
    for z in 30..110 {
        for x in 30..130 {
            let p = img.get_pixel_mut(x, z);
            p.0[0] = p.0[0].saturating_add(8000);
        }
    }
    img.save(&height).unwrap();
    let mine = massif_core::BlockRect::new(-110, -110, -70, -70);
    {
        let app = h.state_mut();
        app.doc.history.begin("flatten", app.doc.dim);
        let mut edit = app.doc.history.edit(&mut app.doc.world, 0);
        edit.par_tiles(mine.tiles(), mine, false, |_, clip, tile| {
            for z in clip.z0..clip.z1 {
                for x in clip.x0..clip.x1 {
                    tile.top.set(massif_core::coords::local_index(x, z), 10.0);
                }
            }
        });
        app.doc.commit();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    {
        let app = h.state_mut();
        let snapshot = app.doc.world.snapshot();
        app.jobs.push(Job::read_mask(&ctx, files, Some(snapshot), app.doc.dim).on(app.doc.generation));
    }
    wait(&mut h, "the zone read", |a| a.jobs.is_empty());
    if let Some(Dialog::ImportMask(form)) = &mut h.state_mut().dialog {
        form.feather = 0;
    }
    h.run_steps(2);
    h.get_by_label("Review…").click();
    h.run_steps(2);
    wait(&mut h, "the review", |a| a.jobs.is_empty());
    let id = map_id(&h);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [-50.0, -20.0];
        view.zoom = 1.6;
    }
    h.event(egui::Event::PointerGone);
    h.run_steps(10);
    let map = areas(&h).into_iter().find(|a| a.0 == id).unwrap().2;
    let r = map.union(area(&h, EditorKind::Versions));
    save(&mut h, "review", Some(r));
    let _ = std::fs::remove_dir_all(&dir);
}
