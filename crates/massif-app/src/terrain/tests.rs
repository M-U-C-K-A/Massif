//! Renders of the panel and its previews, with MASSIF_SNAPSHOT_DIR.

use super::shade::{self, Camera, Look, Overlay, Relief, Surface};
use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, never};
use massif_terrain::gpu::Gpu;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::path::PathBuf;
use std::sync::Arc;

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from)
}

pub(super) fn example(name: &str) -> Graph {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../massif-terrain/examples/graphs").join(format!("{name}.json"));
    Graph::from_json(&std::fs::read_to_string(&path).expect("the example graph")).expect("it reads")
}

/// The lit previews of example graphs: clay and colours from above, a
/// mask over the relief, and the 3D view.
#[test]
fn lit_previews_render() {
    let Some(dir) = snapshot_dir() else { return };
    let Some(gpu) = Gpu::headless() else { return };
    let engine = Engine::with_gpu(gpu.clone(), Default::default());
    for name in ["m4-alpine-colors", "m4-climate", "m2-archipelago", "m2-mesas"] {
        let graph = example(name);
        let size = graph.terrain.size.unwrap_or(4096);
        let (c, _) = compile(&graph, Space::new(size), 1024);
        let lod = c.space.lod_for(512);
        let height = engine.whole(&c, c.output("height").unwrap(), lod, &never).unwrap();
        let albedo = c.output("albedo").map(|o| engine.whole(&c, o, lod, &never).unwrap());
        let sea = c.space.sea_level;
        let relief = Relief { heights: &height, colors: albedo.as_ref(), sea };
        let exag = shade::auto_exaggeration(&height, sea);
        let t0 = std::time::Instant::now();
        let mut shots = Vec::new();
        for (surface, tag) in [(Surface::Clay, "clay"), (Surface::Colour, "colour")] {
            let look = Look { exaggeration: exag, camera: Camera::Top { view: [0.0, 0.0, height.rect.w as f32, height.rect.h as f32] }, ..Look::default() };
            shots.push((tag, shade::shade(Some(&gpu), &relief, None, surface, &look, [512, 512])));
            let look3 = Look { exaggeration: shade::auto_exaggeration_3d(&height, sea), camera: Camera::Orbit { yaw: 20.0, pitch: 32.0, distance: 1.0 }, ..Look::default() };
            shots.push((if surface == Surface::Clay { "3d-clay" } else { "3d-colour" }, shade::shade(Some(&gpu), &relief, None, surface, &look3, [768, 512])));
        }
        if let Some(snow) = c.output("mask.snow").map(|o| engine.whole(&c, o, lod, &never).unwrap()) {
            let look = Look { exaggeration: exag, camera: Camera::Top { view: [0.0, 0.0, height.rect.w as f32, height.rect.h as f32] }, ..Look::default() };
            shots.push(("mask", shade::shade(Some(&gpu), &relief, Some(Overlay { mask: &snow, color: [236, 192, 64], opacity: 0.9 }), Surface::Clay, &look, [512, 512])));
        }
        eprintln!("{name}: {} pictures in {:.1} ms", shots.len(), t0.elapsed().as_secs_f64() * 1000.0);
        for (tag, px) in shots {
            let (w, h) = if tag.starts_with("3d") { (768, 512) } else { (512, 512) };
            image::save_buffer(dir.join(format!("lit-{name}-{tag}.png")), &px, w, h, image::ColorType::Rgba8).unwrap();
        }
    }
    let _ = Arc::strong_count(&gpu);
}

/// The whole panel with real graphs, at 1× and 2×, light and dark.
#[test]
fn the_panel_renders() {
    let Some(dir) = snapshot_dir() else { return };
    let Some(gpu) = Gpu::headless() else { return };
    let engine = Arc::new(Engine::with_gpu(gpu, Default::default()));
    let cases: [(&str, &str, Option<&str>, bool); 3] = [("m4-alpine-colors", "albedo", None, false), ("m4-climate", "out", Some("out"), false), ("m2-archipelago", "out", None, true)];
    let only = std::env::var("MASSIF_PANEL_PPP").ok().and_then(|v| v.parse::<f32>().ok());
    for (name, select, _view, three_d) in cases {
        for ppp in [1.0f32, 2.0] {
            if only.is_some_and(|o| o != ppp) {
                continue;
            }
            for (theme, tag) in [(egui::Theme::Dark, "dark"), (egui::Theme::Light, "light")] {
                let mut panel = super::TerrainPanel::with_graph(engine.clone(), example(name));
                panel.selected = vec![select.to_owned()];
                panel.look.three_d = three_d;
                let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui_state(
                    move |ui, p: &mut super::TerrainPanel| {
                        if ui.ctx().cumulative_pass_nr() == 0 {
                            crate::theme::install(ui.ctx());
                        }
                        ui.ctx().set_theme(theme);
                        p.ui(ui);
                    },
                    panel,
                );
                h.run_steps(3);
                let fit = h.state().graph.nodes.len();
                let _ = fit;
                // The previews, made on their thread.
                let t0 = std::time::Instant::now();
                loop {
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    if (h.state().previews_idle() && h.state().viewer.as_ref().is_some_and(|v| v.1.sharp) && t0.elapsed().as_millis() > 300) || t0.elapsed().as_secs() > 60 {
                        break;
                    }
                }
                h.run_steps(3);
                eprintln!("{name} {tag} {ppp}x: previews in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
                h.render().unwrap().save(dir.join(format!("panel-{name}-{tag}-{ppp}x.png"))).unwrap();
            }
        }
    }
}

fn cpu_panel() -> super::TerrainPanel {
    super::TerrainPanel::new(Arc::new(Engine::new(Default::default())))
}

/// Every setting of every type of node, written as the panel writes it,
/// reads back the same, without a complaint.
#[test]
fn settings_written_read_back() {
    use massif_terrain::params::{Params, Value};
    for spec in massif_terrain::registry::all() {
        let defaults = Params::defaults(spec, "n");
        for (k, (q, v)) in spec.params.iter().zip(defaults.values()).enumerate() {
            let mut given = std::collections::BTreeMap::new();
            given.insert(q.name.to_owned(), super::props::encode(q, v));
            let mut diags = Vec::new();
            let back = Params::decode(spec, "n", &given, "", &mut diags);
            assert!(diags.is_empty(), "{}.{}: {diags:?}", spec.id, q.name);
            match (&back.values()[k], v) {
                (Value::Num(a), Value::Num(b)) => assert!((a - b).abs() < 1e-3, "{}.{}", spec.id, q.name),
                (a, b) => assert_eq!(a, b, "{}.{}", spec.id, q.name),
            }
        }
    }
}

/// Frames, reroute points and the view go with the graph, under its
/// `editor`, and come back when it is read.
#[test]
fn the_layout_lives_in_the_graph() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["alps".into(), "worn".into()];
    super::canvas::frame_selection(&mut p);
    p.edit_layout().reroutes.insert("worn.height".into(), vec![[10.0, 20.0]]);
    p.edit_layout().exposed.insert("out.mask.moss".into());
    let saved = p.saved();
    assert!(!saved.editor.is_null());
    let again = Graph::from_json(&saved.to_json()).unwrap();
    let l = super::panel::Layout::of(&again);
    assert_eq!(l.frames.len(), 1);
    assert_eq!(l.reroutes["worn.height"], vec![[10.0, 20.0]]);
    assert!(l.exposed.contains("out.mask.moss") && l.view.is_some());
    // The engine ignores it: same terrain.
    assert!(again.same_terrain(&example("m4-alpine-colors")));
}

/// A node renamed keeps its wires, outputs named after the dot included.
#[test]
fn renaming_follows_the_wires() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    assert!(p.rename("worn", "eroded"));
    let g = &p.graph;
    assert!(!g.nodes.contains_key("worn") && g.nodes.contains_key("eroded"));
    assert_eq!(g.nodes["slope"].inputs["height"], "eroded");
    assert_eq!(g.nodes["terrains"].inputs["mask_2"], "eroded.talus");
    assert!(!p.rename("eroded", "slope"), "a name taken");
    assert!(!p.rename("eroded", "a.b"), "a name with a dot");
}

/// Changes undo and redo, a drag or a slider being one step.
#[test]
fn undo_and_redo() {
    let mut p = cpu_panel();
    p.set_graph(super::TerrainPanel::starter());
    let id = p.add_node("noise.fbm", [0.0, 0.0]);
    p.connect(&id, 0, "out", "height");
    assert_eq!(p.graph.nodes["out"].inputs["height"], id, "one edit so far: the same step");
    assert!(p.undo());
    assert!(!p.graph.nodes.contains_key(&id) && !p.graph.nodes["out"].inputs.contains_key("height"));
    assert!(p.redo());
    assert!(p.graph.nodes.contains_key(&id));
    assert!(!p.redo());
}

/// The search finds nodes by the start of their words, by their letters
/// in order, and, for a wire let go, only those that take what it carries.
#[test]
fn the_search_finds_nodes() {
    use massif_terrain::registry::PortKind;
    assert_eq!(super::canvas::find("alp", None)[0].id, "mountain.alpine");
    assert_eq!(super::canvas::find("therm", None)[0].id, "erosion.thermal");
    assert!(super::canvas::find("mtnrange", None).iter().any(|s| s.id == "mountain.range"));
    let takes_terrain = super::canvas::find("", Some(PortKind::Terrain));
    assert!(takes_terrain.iter().any(|s| s.id == "output.map"));
    assert!(takes_terrain.iter().all(|s| s.inputs.iter().any(|i| PortKind::Terrain.conversion(i.kind).is_some()) || s.open_inputs.is_some()));
    assert!(super::canvas::find("zzqx", None).is_empty());
}

/// The document keeps the graph the panel edits: written when changed,
/// read back when another world is opened, the map's square around it.
#[test]
fn the_document_keeps_the_graph() {
    let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 4, tiles_z: 2, landform: massif_core::generate::Landform::Flat, ..Default::default() });
    let mut doc = crate::doc::Document::new(world, None, 1);
    let mut p = cpu_panel();
    p.sync(&mut doc);
    assert!(p.graph.nodes.contains_key("out"), "a world without a graph starts from a Map Output");
    assert_eq!(p.space.size, 512, "the map's 4 × 2 tiles, as a square");
    p.commit(&mut doc);
    assert!(doc.world.meta.terrain_graph.is_none(), "nothing written before an edit");
    let id = p.add_node("noise.fbm", [0.0, 0.0]);
    p.commit(&mut doc);
    let stored: Graph = serde_json::from_value(doc.world.meta.terrain_graph.clone().unwrap()).unwrap();
    assert!(stored.nodes.contains_key(&id));
    // Another world opened: its graph replaces the panel's.
    doc.world.meta.terrain_graph = Some(serde_json::to_value(example("m2-volcano")).unwrap());
    p.sync(&mut doc);
    assert!(!p.graph.nodes.contains_key(&id) && !p.can_undo());
}

/// A world with a graph of Massif 1.2 and none of 1.3 opens with its graph
/// migrated, kept in the world without the world counting as changed; a
/// graph with a Map node reads the map, again once the map changes.
#[test]
fn a_world_of_1_2_opens_migrated() {
    let mut world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 2, tiles_z: 2, landform: massif_core::generate::Landform::Flat, ..Default::default() });
    let get = |name: &str| world.registry.terrain_by_name(name).unwrap_or(massif_core::registry::TerrainId::ROCK).0;
    world.meta.graph = Some(massif_core::graph::Graph::starter(get("Grass"), massif_core::registry::TerrainId::ROCK.0, get("Snow"), get("Gravel")));
    let mut doc = crate::doc::Document::new(world, None, 1);
    let mut p = cpu_panel();
    p.sync(&mut doc);
    assert!(p.graph.nodes.values().any(|n| n.kind == "output.map"), "{:?}", p.graph.nodes.keys().collect::<Vec<_>>());
    assert!(doc.world.meta.terrain_graph.is_some() && !doc.is_modified());
    // A Map node: the environment reads the map at the panel's revision.
    let id = p.add_node("input.map", [0.0, 0.0]);
    p.commit(&mut doc);
    p.sync(&mut doc);
    assert!(p.graph.nodes.contains_key(&id));
    assert_eq!(p.env.map_revision(), p.map_revision);
    let before = p.map_revision;
    doc.mark_changed();
    p.sync(&mut doc);
    assert_eq!(p.map_revision, before + 1, "a change of the map is seen");
}
