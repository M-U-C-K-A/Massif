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

/// Frames go with the graph (its own, nested), reroute points and the
/// view under its `editor`, and come back when it is read.
#[test]
fn the_layout_lives_in_the_graph() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["alps".into(), "worn".into()];
    super::canvas::frame_selection(&mut p);
    p.selected = vec!["worn".into()];
    let inner = p.frame_selection().unwrap();
    p.edit_layout().reroutes.insert("worn.height".into(), vec![[10.0, 20.0]]);
    p.edit_layout().exposed.insert("out.mask.moss".into());
    let saved = p.saved();
    assert!(!saved.editor.is_null());
    let again = Graph::from_json(&saved.to_json()).unwrap();
    assert_eq!(again.frames.len(), 2);
    let outer = again.frames[&inner].parent.clone().expect("nested");
    assert_eq!(again.frames[&outer].members, vec!["alps".to_owned()]);
    assert_eq!(again.frames[&inner].members, vec!["worn".to_owned()]);
    let l = super::panel::Layout::of(&again);
    assert_eq!(l.reroutes["worn.height"], vec![[10.0, 20.0]]);
    assert!(l.exposed.contains("out.mask.moss") && l.view.is_some());
    // The engine ignores it: same terrain.
    assert!(again.same_terrain(&example("m4-alpine-colors")));
}

/// A node gets a name to show; its id, which the wires name, stays.
#[test]
fn names_leave_the_wires_alone() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.set_name("worn", Some("Eroded".into()));
    let g = &p.graph;
    assert_eq!(g.nodes["worn"].name.as_deref(), Some("Eroded"));
    assert_eq!(g.nodes["slope"].inputs["height"], "worn");
    assert_eq!(p.display_name("worn"), "Eroded");
    p.set_name("worn", Some("  ".into()));
    assert!(p.graph.nodes["worn"].name.is_none(), "an empty name gives back the type's");
}

/// Changes undo and redo, what one gesture does (a node added and wired,
/// a slider dragged) being one step.
#[test]
fn undo_and_redo() {
    let mut p = cpu_panel();
    p.set_graph(super::TerrainPanel::starter());
    let id = p.add_node("noise.fbm", [0.0, 0.0]);
    p.connect(&id, 0, "out", "height");
    p.end_edit();
    assert_eq!(p.graph.nodes["out"].inputs["height"], id);
    for k in 0..5 {
        p.op(massif_terrain::format::Op::Set { node: id.clone(), param: "scale".into(), value: (500 + k * 10).into() }).unwrap();
    }
    p.end_edit();
    assert_eq!(p.graph.nodes[&id].params["scale"], 540);
    assert!(p.undo());
    assert!(!p.graph.nodes[&id].params.contains_key("scale"), "the slider's drag, as one step");
    assert!(p.undo());
    assert!(!p.graph.nodes.contains_key(&id) && !p.graph.nodes["out"].inputs.contains_key("height"));
    assert!(p.redo());
    assert!(p.graph.nodes.contains_key(&id));
    assert!(p.redo() && !p.redo());
    assert_eq!(p.graph.nodes[&id].params["scale"], 540);
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

/// A graph with one of Massif's mountains feeding the Map Output.
fn mountain_graph() -> Graph {
    Graph::from_json(
        r#"{ "nodes": {
            "m": { "type": "massif:mountain", "params": { "height": 220 } },
            "worn": { "type": "erosion.thermal", "inputs": { "height": "m.height" } },
            "out": { "type": "output.map", "inputs": { "height": "worn", "mask.snow": "m.snow" } }
        } }"#,
    )
    .unwrap()
}

/// A composite opens: Massif's read only, edited as a copy that the
/// document then holds; its inside edited goes back into the document; the
/// breadcrumb leads out; undo comes back through the scopes.
#[test]
fn composites_open_and_are_edited_as_a_copy() {
    let mut p = cpu_panel();
    p.set_graph(mountain_graph());
    let shape = p.shape("m").unwrap();
    assert!(shape.is_composite());
    assert_eq!(shape.outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), ["height", "snow", "flow", "wear"]);
    assert_eq!(shape.outputs[1].kind, massif_terrain::registry::PortKind::Mask);
    assert!(p.open("m"));
    assert!(p.read_only() && p.graph.nodes.contains_key("settle"));
    assert!(p.op(massif_terrain::format::Op::Remove { node: "snow".into() }).is_err(), "Massif's composites are read only");
    // Its columns stand for its inputs and outputs.
    assert!(p.ids().contains(&super::shape::IN_COLUMN.to_owned()));
    assert_eq!(p.source_of(super::shape::OUT_COLUMN, "height"), Some(("settle".to_owned(), 0)));
    let kind = p.edit_open_copy().unwrap();
    assert!(kind.starts_with("project:mountain"));
    assert!(!p.read_only() && p.scope_kind() == Some(kind.as_str()));
    let id = p.add_node("adjust.levels", [0.0, 400.0]);
    p.connect("settle", 0, &id, "in");
    p.connect(&id, 0, super::shape::OUT_COLUMN, "height");
    p.end_edit();
    p.close();
    assert!(p.outer.is_empty());
    let doc = p.document();
    assert_eq!(doc.nodes["m"].kind, kind);
    assert_eq!(doc.nodes["m"].params["height"], 220, "its settings kept");
    let c = &doc.components[&kind];
    assert_eq!(c.nodes[&id].inputs["in"], "settle");
    assert_eq!(c.outputs[0].from, id);
    // It compiles, the new node in the instance.
    let (compiled, diags) = massif_terrain::compile::compile(&doc, massif_terrain::space::Space::new(1024), 256);
    assert!(diags.iter().all(|d| d.severity != massif_terrain::compile::Severity::Error), "{diags:?}");
    assert!(compiled.node(&format!("m__{id}")).is_some());
    // The thumbnail of the instance is of what makes its first output.
    let key = super::panel::lowered(&doc, &[], &super::eval::Shown { node: "m".into(), port: None }).unwrap();
    assert_eq!(key.node, format!("m__{id}"));
    // Undo goes back through the copy to Massif's own.
    while p.undo() {}
    assert_eq!(p.document().nodes["m"].kind, "massif:mountain");
    assert!(p.document().components.is_empty());
}

/// Nodes gathered into a composite of the document and unfolded again:
/// the wires through it kept, the terrain the same.
#[test]
fn grouping_and_ungrouping() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    let before = massif_terrain::lower::lower(&p.document()).0;
    p.selected = vec!["alps".into(), "worn".into()];
    let id = p.group_selected("Relief").unwrap();
    p.end_edit();
    assert!(!p.graph.nodes.contains_key("alps") && p.shape(&id).unwrap().is_composite());
    assert_eq!(massif_terrain::model::Source::parse(&p.graph.nodes["slope"].inputs["height"]).node, id);
    let (_, diags) = massif_terrain::compile::compile(&p.document(), massif_terrain::space::Space::new(1024), 256);
    assert!(diags.iter().all(|d| d.severity != massif_terrain::compile::Severity::Error), "{diags:?}");
    // Inside, the nodes keep their places.
    assert!(p.open(&id));
    assert!(p.graph.ui.contains_key("alps") && p.graph.ui.contains_key("worn"));
    p.close();
    p.ungroup(&id).unwrap();
    p.end_edit();
    assert!(!p.graph.nodes.contains_key(&id) && p.graph.nodes.len() == example("m4-alpine-colors").nodes.len());
    let after = massif_terrain::lower::lower(&p.document()).0;
    assert_eq!(before.nodes.len(), after.nodes.len());
}

/// Nodes copied go through the clipboard as JSON, into another document
/// too, with the wires among them and the composites they are made of;
/// wires from outside are left.
#[test]
fn copy_and_paste_between_documents() {
    let mut a = cpu_panel();
    a.set_graph(example("m4-alpine-colors"));
    a.selected = vec!["alps".into()];
    a.group_selected("Relief").unwrap();
    a.end_edit();
    let relief = a.selected[0].clone();
    a.selected = vec![relief.clone(), "slope".into(), "snow".into(), "rocky".into()];
    let text = a.copy_text().unwrap();
    let mut b = cpu_panel();
    b.set_graph(super::TerrainPanel::starter());
    assert!(b.paste(&text, Some([0.0, 0.0])));
    assert_eq!(b.selected.len(), 4);
    let kind = &a.graph.nodes[&relief].kind;
    assert!(b.graph.components.contains_key(kind), "the composite comes with its node");
    let slope = b.selected.iter().find(|id| b.graph.nodes[*id].kind.starts_with("mask.slope")).unwrap().clone();
    assert!(b.graph.nodes[&slope].inputs.is_empty(), "its input came from outside the copy");
    let rocky = b.selected.iter().find(|id| b.graph.nodes[*id].kind.starts_with("color.mix")).unwrap();
    assert_eq!(b.graph.nodes[rocky].inputs["mask"], slope, "wires among the copies follow them");
    assert!(!b.paste("not nodes", None));
    // Pasted again: fresh ids.
    b.end_edit();
    assert!(b.paste(&text, None));
    assert_eq!(b.graph.nodes.len(), 1 + 8);
}

/// A node bypassed gives way to its input, in the previews too; B toggles.
#[test]
fn bypass_passes_the_input_on() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["worn".into()];
    p.toggle_bypass();
    p.end_edit();
    assert!(p.graph.nodes["worn"].bypass);
    let doc = p.document();
    let key = super::panel::lowered(&doc, &[], &super::eval::Shown { node: "worn".into(), port: None }).unwrap();
    assert_eq!(key.node, "alps", "its thumbnail shows what passes through");
    p.toggle_bypass();
    assert!(!p.graph.nodes["worn"].bypass);
    p.selected = vec!["out".into()];
    p.toggle_bypass();
    assert!(!p.graph.nodes["out"].bypass, "a Map Output is never left out");
}

/// Frames nest, move with what they hold, fold their nodes away, and take
/// a node dropped in them.
#[test]
fn frames_nest_move_and_fold() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["alps".into(), "worn".into(), "slope".into()];
    let outer = p.frame_selection().unwrap();
    p.selected = vec!["worn".into()];
    let inner = p.frame_selection().unwrap();
    p.end_edit();
    assert_eq!(p.graph.frames[&inner].parent.as_deref(), Some(outer.as_str()));
    assert_eq!(p.frame_of("worn").as_deref(), Some(inner.as_str()));
    assert_eq!(p.frame_nodes(&outer).len(), 3);
    let at = p.pos("worn");
    p.move_frame(&outer, [100.0, 50.0]);
    p.end_edit();
    assert_eq!(p.pos("worn"), [at[0] + 100.0, at[1] + 50.0], "the inner frame's node moves with the outer frame");
    p.toggle_frame(&outer);
    assert_eq!(p.hidden_in("worn").as_deref(), Some(outer.as_str()));
    assert!(p.frame_rect(&outer)[3] <= 30.0);
    p.toggle_frame(&outer);
    // A node dropped inside the inner frame joins it; dragged far away, it
    // leaves every frame.
    let r = p.frame_rect(&inner);
    p.graph.ui.get_mut("rock").unwrap().pos = [r[0] + 10.0, r[1] + 10.0];
    p.refile(&["rock".to_owned()]);
    assert_eq!(p.frame_of("rock").as_deref(), Some(inner.as_str()));
    p.graph.ui.get_mut("rock").unwrap().pos = [-5000.0, -5000.0];
    p.refile(&["rock".to_owned()]);
    assert!(p.frame_of("rock").is_none());
    // Removed, its nodes go to the frame around it.
    p.unframe(&inner);
    assert_eq!(p.frame_of("worn").as_deref(), Some(outer.as_str()));
}

/// A link between sockets of types nothing converts is refused, saying
/// why and which node to put between them.
#[test]
fn refused_links_are_explained() {
    use massif_terrain::registry::PortKind;
    let why = super::canvas::refusal(PortKind::Points, PortKind::Mask);
    assert!(why.contains("Points to mask"), "{why}");
    let why = super::canvas::refusal(PortKind::Density, PortKind::Height);
    assert!(why.contains("volume"), "{why}");
    assert!(PortKind::Height.conversion(PortKind::Mask).is_some());
    // On the canvas, the graph refuses it too.
    let mut p = cpu_panel();
    p.set_graph(super::TerrainPanel::starter());
    let pts = p.add_node("points.scatter", [0.0, 0.0]);
    p.connect(&pts, 0, "out", "height");
    p.end_edit();
    assert!(!p.graph.nodes["out"].inputs.contains_key("height"));
    assert!(p.notice.as_ref().is_some_and(|n| n.0.contains("Points to mask")));
}

/// A setting inside a composite of the document, exposed: every node of
/// it shows it, and what it is set to reaches the node inside.
#[test]
fn settings_exposed_reach_inside() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["alps".into(), "worn".into()];
    let id = p.group_selected("Relief").unwrap();
    p.end_edit();
    p.open(&id);
    p.sync_scope();
    let kind = p.scope_kind().map(|k| super::panel::base_kind(k).to_owned()).unwrap();
    let exposed = massif_terrain::model::Exposed { name: "tall".into(), label: "Height".into(), help: String::new(), targets: vec!["alps.height".into()], default: None, min: None, max: None, soft: Some([100.0, 900.0]), unit: None, control: None, category: "Shape".into(), advanced: false };
    p.op(massif_terrain::format::Op::Expose { component: kind, exposed }).unwrap();
    p.end_edit();
    p.close();
    let shape = p.shape(&id).unwrap();
    let (_, comp, editable) = shape.composite().unwrap();
    assert!(editable && comp.exposed.len() == 1);
    let (_, base) = super::shape::exposed_base(&p.graph, comp, &comp.exposed[0]).unwrap();
    let q = super::shape::exposed_spec(base, &comp.exposed[0]);
    assert_eq!(q.name, "tall");
    assert!(matches!(q.kind, massif_terrain::registry::ParamKind::Float { soft: [100.0, 900.0], .. }));
    p.op(massif_terrain::format::Op::Set { node: id.clone(), param: "tall".into(), value: 640.into() }).unwrap();
    let (flat, _) = massif_terrain::lower::lower(&p.document());
    assert_eq!(flat.nodes[&format!("{id}__alps")].params["height"], 640);
}

/// The search offers composites beside the registry's nodes.
#[test]
fn the_search_offers_composites() {
    let g = mountain_graph();
    let hits = super::canvas::find_in(&g, "range", None);
    assert!(hits.iter().any(|e| e.id == "massif:alpine-range" && e.is_composite()));
    // Mountain is superseded by Alpine Range: kept for the graphs using it,
    // not offered.
    assert!(super::canvas::find_in(&g, "mount", None).iter().all(|e| e.id != "massif:mountain"));
    let heights = super::canvas::find_in(&g, "", Some(massif_terrain::registry::PortKind::Height));
    assert!(heights.iter().any(|e| e.id == "massif:cave-system"), "it takes a relief");
}

/// A graph showing what the editor does: one of Massif's composites, a
/// node bypassed, points scattered and drawn, frames in frames.
fn showcase() -> Graph {
    Graph::from_json(
        r##"{ "nodes": {
            "m": { "type": "massif:mountain", "params": { "height": 220 } },
            "worn": { "type": "erosion.thermal", "inputs": { "height": "m.height" } },
            "lv": { "type": "adjust.levels", "inputs": { "in": "worn" }, "bypass": true, "name": "Flatten the foot" },
            "pts": { "type": "points.scatter", "inputs": { "density": "m.snow", "height": "worn" } },
            "dots": { "type": "points.mask", "inputs": { "points": "pts" } },
            "out": { "type": "output.map", "inputs": { "height": "lv", "mask.snow": "m.snow", "mask.trees": "dots" } }
        },
        "ui": {
            "m": { "pos": [0, 0] }, "worn": { "pos": [228, 0] }, "lv": { "pos": [456, 0] },
            "pts": { "pos": [228, 300] }, "dots": { "pos": [456, 300] }, "out": { "pos": [720, 0] }
        },
        "frames": {
            "base": { "title": "Mountain Generation", "color": "#56c478", "members": ["m"], "rect": [-16, -40, 120, 60] },
            "wear": { "title": "Hydraulic Erosion", "color": "#5c92ff", "parent": "base", "members": ["worn", "lv"], "rect": [212, -40, 120, 60], "description": "Wears the mountain down." },
            "veg": { "title": "Vegetation Distribution", "color": "#34bab0", "members": ["pts", "dots"], "rect": [212, 260, 120, 60] }
        },
        "terrain": { "size": 2048 } }"##,
    )
    .unwrap()
}

/// Renders the editor as the app shows it, after `prepare`.
fn render_editor(dir: &std::path::Path, name: &str, theme: egui::Theme, ppp: f32, engine: Arc<Engine>, graph: Graph, prepare: impl FnOnce(&mut super::TerrainPanel)) {
    render_editor_then(dir, name, theme, ppp, engine, graph, prepare, |_| {});
}

/// [`render_editor`], `last` done once the previews are made.
#[allow(clippy::too_many_arguments)]
fn render_editor_then(dir: &std::path::Path, name: &str, theme: egui::Theme, ppp: f32, engine: Arc<Engine>, graph: Graph, prepare: impl FnOnce(&mut super::TerrainPanel), last: impl FnOnce(&mut super::TerrainPanel)) {
    let mut panel = super::TerrainPanel::with_graph(engine, graph);
    prepare(&mut panel);
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
    let t0 = std::time::Instant::now();
    loop {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
        if (h.state().previews_idle() && t0.elapsed().as_millis() > 400) || t0.elapsed().as_secs() > 60 {
            break;
        }
    }
    last(h.state_mut());
    h.run_steps(3);
    eprintln!("{name}: previews in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
}

/// The editor of 1.2: composites, their inside, frames, refusals.
#[test]
fn the_editor_renders() {
    let Some(dir) = snapshot_dir() else { return };
    let Some(gpu) = Gpu::headless() else { return };
    let engine = Arc::new(Engine::with_gpu(gpu, Default::default()));
    use egui::Theme::{Dark, Light};
    for (theme, tag) in [(Dark, "dark"), (Light, "light")] {
        render_editor(&dir, &format!("editor-graph-{tag}"), theme, 1.0, engine.clone(), showcase(), |p| {
            p.selected = vec!["m".into()];
        });
    }
    render_editor(&dir, "editor-graph-dark-2x", Dark, 2.0, engine.clone(), showcase(), |p| {
        p.selected = vec!["lv".into()];
    });
    // Inside Massif's mountain: read only, its columns at the edges.
    render_editor(&dir, "editor-inside-massif-dark", Dark, 1.0, engine.clone(), showcase(), |p| {
        p.open("m");
        p.selected = vec!["erode".into()];
    });
    // Inside a copy: editable, nothing selected: its interface.
    render_editor(&dir, "editor-inside-copy-light", Light, 1.0, engine.clone(), showcase(), |p| {
        p.open("m");
        p.edit_open_copy().unwrap();
        p.end_edit();
        p.selected.clear();
    });
    // A setting exposed from inside: the dialog.
    render_editor(&dir, "editor-expose-dark", Dark, 1.0, engine.clone(), showcase(), |p| {
        p.open("m");
        p.edit_open_copy().unwrap();
        p.end_edit();
        p.selected = vec!["shape".into()];
        p.exposing = Some(super::props::Exposing { target: "shape.roughness".into(), name: "rough".into(), label: "Roughness".into(), category: "Shape".into(), range: Some([0.0, 1.0]), ..Default::default() });
    });
    // Geometry and volumes previewed: a village's roads in the viewer, its
    // buildings and a hall cut under the sea on their nodes.
    let pack = test_pack("render");
    let mut g = village_and_caves(&pack);
    g.terrain.size = Some(1024);
    g.nodes.insert("hall".into(), massif_terrain::model::Node::new("volume.sphere").with("x", 0.5).with("z", 0.5).with("y", 40).with("radius", 120).with("squash", 0.5));
    g.nodes.insert("pts".into(), massif_terrain::model::Node::new("points.scatter").input("height", "land"));
    render_editor(&dir, "editor-geometry-dark", Dark, 1.0, engine.clone(), g, |p| {
        p.viewed = Some((Vec::new(), super::eval::Shown { node: "village".into(), port: Some("roads".into()) }));
        p.selected = vec!["hall".into()];
    });
    let _ = std::fs::remove_dir_all(pack.parent().unwrap());
    // A frame folded and selected; a refused link explained.
    render_editor_then(
        &dir,
        "editor-frames-light",
        Light,
        1.0,
        engine.clone(),
        showcase(),
        |p| {
            p.toggle_frame("veg");
            p.end_edit();
            p.selected_frame = Some("wear".into());
        },
        |p| p.connect("pts", 0, "worn", "height"),
    );
}

/// Inside a composite of the document, its inputs' column feeds nodes
/// (`in.<name>`), its outputs' column takes what they make, and an input
/// taken off is unlinked inside.
#[test]
fn a_composites_columns_wire_its_inside() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    p.selected = vec!["worn".into()];
    let id = p.group_selected("Wear").unwrap();
    p.end_edit();
    p.open(&id);
    let kind = p.scope_kind().unwrap().to_owned();
    let input = massif_terrain::lower::component(&p.graph, &kind).unwrap().inputs[0].name.clone();
    p.disconnect("worn", "height");
    p.end_edit();
    assert!(!p.graph.nodes["worn"].inputs.contains_key("height"));
    p.connect(super::shape::IN_COLUMN, 0, "worn", "height");
    p.end_edit();
    assert_eq!(p.graph.nodes["worn"].inputs["height"], format!("in.{input}"));
    let first = massif_terrain::lower::component(&p.graph, &kind).unwrap().outputs[0].name.clone();
    p.disconnect(super::shape::OUT_COLUMN, &first);
    assert!(p.source_of(super::shape::OUT_COLUMN, &first).is_none());
    p.connect("worn", 1, super::shape::OUT_COLUMN, &first);
    p.end_edit();
    let c = massif_terrain::lower::component(&p.graph, &kind).unwrap().into_owned();
    assert_eq!(c.outputs[0].from, "worn.talus");
    // Taken off: unlinked inside.
    p.doc_op(massif_terrain::format::Op::Unport { component: kind.clone(), name: input }).unwrap();
    p.end_edit();
    assert!(p.scope_kind() == Some(kind.as_str()), "still open");
    assert!(!p.graph.nodes["worn"].inputs.contains_key("height"));
    p.close();
    let (_, diags) = massif_terrain::lower::lower(&p.document());
    assert!(diags.iter().all(|d| d.severity != massif_terrain::compile::Severity::Error), "{diags:?}");
}

/// A graph making a village and caves on a map of `tiles` × `tiles` tiles.
fn village_and_caves(pack: &std::path::Path) -> Graph {
    let text = format!(
        r#"{{ "nodes": {{
        "land": {{ "type": "noise.fbm", "params": {{ "scale": 600, "height": 10, "base": 70 }} }},
        "village": {{ "type": "massif:village-generator", "inputs": {{ "height": "land" }}, "params": {{ "pack": {pack}, "houses": 18, "walls": true }} }},
        "system": {{ "type": "massif:cave-system", "inputs": {{ "height": "village.height" }} }},
        "out": {{ "type": "output.map", "inputs": {{ "height": "village.height", "structures": "village.structures", "roads": "village.roads", "caves": "system.caves", "cave_biomes": "system.biomes" }} }}
    }} }}"#,
        pack = serde_json::to_string(&pack.to_string_lossy()).unwrap()
    );
    Graph::from_json(&text).unwrap()
}

pub(crate) fn test_pack(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-app-apply-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    massif_structures::village::testpack::write_pack(&dir).unwrap()
}

/// Apply writes everything the Map Output carries in one step: the ground
/// window by window, the caves and cave biomes, the village's structures;
/// undo takes it all back, and Cancel leaves the map as it was.
#[test]
fn apply_writes_ground_caves_and_villages_in_one_step() {
    let pack = test_pack("apply");
    let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 4, tiles_z: 4, landform: massif_core::generate::Landform::Flat, ..Default::default() });
    let mut doc = crate::doc::Document::new(world, None, 1);
    let blocks = crate::blocks::BlockLibrary::default();
    let engine = Arc::new(Engine::new(Default::default()));
    let graph = village_and_caves(&pack);
    let env: Arc<dyn massif_terrain::kernel::Env> = Arc::new(massif_terrain::world::MapEnv::new(&doc.world, 0, 1, "."));
    let before_steps = doc.history.undo_count();
    let height_before = massif_core::ops::height_at(doc.dimension(), 0, 100, 100, massif_core::ops::Surface::Top);
    let mut a = super::apply::Applying::start(engine.clone(), &graph, &mut doc, env.clone(), 7).unwrap();
    let t0 = std::time::Instant::now();
    let done = loop {
        if let Some(r) = a.step(&mut doc, &blocks, None) {
            break r.unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(t0.elapsed().as_secs() < 120, "Apply never finished");
    };
    eprintln!("applied {} windows in {:.2} s: {:?}, {} villages, notes {:?}", done.windows, done.seconds, done.caves, done.villages, done.notes);
    assert_eq!(doc.history.undo_count(), before_steps + 1, "one step");
    assert!(done.caves.cave_layers >= 1 && done.caves.cave_columns > 0, "{:?}", done.caves);
    assert_eq!(done.villages, 1);
    let dim = doc.dimension();
    assert!(dim.structures.iter().any(|s| !s.source.is_user()), "the graph's village is on the map");
    assert_ne!(massif_core::ops::height_at(dim, 0, 100, 100, massif_core::ops::Surface::Top), height_before);
    // Undo: as it was.
    assert!(doc.undo().is_some());
    assert!(doc.dimension().structures.iter().all(|s| s.source.is_user()));
    assert_eq!(massif_core::ops::height_at(doc.dimension(), 0, 100, 100, massif_core::ops::Surface::Top), height_before);
    // Cancelled half way: as it was, no step.
    let steps = doc.history.undo_count();
    let mut a = super::apply::Applying::start(engine, &graph, &mut doc, env, 8).unwrap();
    let _ = a.step(&mut doc, &blocks, None);
    a.cancel(&mut doc);
    assert_eq!(doc.history.undo_count(), steps);
    assert_eq!(massif_core::ops::height_at(doc.dimension(), 0, 100, 100, massif_core::ops::Surface::Top), height_before);
    let _ = std::fs::remove_dir_all(pack.parent().unwrap());
}

/// On a map larger than a window, Apply writes it window by window with
/// no seam where they meet, as one window would.
#[test]
fn apply_goes_window_by_window_without_seams() {
    let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 16, tiles_z: 16, landform: massif_core::generate::Landform::Flat, ..Default::default() });
    let mut doc = crate::doc::Document::new(world, None, 1);
    let blocks = crate::blocks::BlockLibrary::default();
    let engine = Arc::new(Engine::new(Default::default()));
    let graph = Graph::from_json(r#"{ "nodes": {
        "land": { "type": "noise.fbm", "params": { "scale": 500, "height": 60, "base": 70 } },
        "worn": { "type": "erosion.thermal", "inputs": { "height": "land" } },
        "out": { "type": "output.map", "inputs": { "height": "worn" } } } }"#).unwrap();
    let env: Arc<dyn massif_terrain::kernel::Env> = Arc::new(massif_terrain::world::MapEnv::new(&doc.world, 0, 1, "."));
    let mut a = super::apply::Applying::start(engine.clone(), &graph, &mut doc, env, 1).unwrap();
    assert_eq!(a.windows, 4);
    let done = loop {
        if let Some(r) = a.step(&mut doc, &blocks, None) {
            break r.unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    eprintln!("2048² in {} windows: {:.2} s", done.windows, done.seconds);
    assert_eq!(done.windows, 4);
    // What the graph makes over the whole terrain at once, against what
    // the windows wrote, along the line where they meet.
    let space = massif_terrain::world::space_of(&doc.world, 0, &graph);
    let (c, _) = massif_terrain::compile::compile(&graph, space, space.size.min(4096));
    let whole = engine.region(&c, c.output("height").unwrap(), 0, c.space.whole(0), &massif_terrain::engine::never).unwrap();
    let mut worst = 0.0f32;
    for z in (0..2048).step_by(37) {
        for x in [1022, 1023, 1024, 1025] {
            let (bx, bz) = (c.space.x0 + x, c.space.z0 + z);
            let written = massif_core::ops::height_at(doc.dimension(), 0, bx, bz, massif_core::ops::Surface::Top).unwrap();
            worst = worst.max((written - whole.at(x, z)).abs());
        }
    }
    assert!(worst <= 1.0, "the windows meet {worst} blocks apart from the whole");
}

/// Each node shows what it last cost to make, a composite what all it
/// holds cost; a node only read from the cache keeps its cost.
#[test]
fn nodes_show_what_they_cost() {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_accessibility_check(false).build_ui_state(
        |ui, p: &mut super::TerrainPanel| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::theme::install(ui.ctx());
            }
            p.ui(ui);
        },
        super::TerrainPanel::with_graph(Arc::new(Engine::new(Default::default())), mountain_graph()),
    );
    let t0 = std::time::Instant::now();
    loop {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
        if (h.state().previews_idle() && t0.elapsed().as_millis() > 300) || t0.elapsed().as_secs() > 120 {
            break;
        }
    }
    h.run_steps(2);
    let p = h.state();
    let (ms, bytes) = p.cost("worn").expect("the erosion was made");
    assert!(ms > 0.0 && bytes > 0, "{ms} ms, {bytes} bytes");
    let (inside, _) = p.cost("m").expect("the composite's nodes were made");
    let one = p.costs.iter().filter(|(k, _)| k.starts_with("m__")).count();
    assert!(one >= 2 && inside > 0.0, "{one} nodes inside measured");
    assert!(super::canvas::cost_text(1500.0, 12 << 20) == "1.5 s · 12 MB");
}

/// The Script pane: the graph as a script, a script applied as one step
/// to undo, the nodes kept where they were.
#[test]
fn a_script_makes_the_graph() {
    let mut p = cpu_panel();
    p.set_graph(example("m4-alpine-colors"));
    let before = p.document();
    p.script.open = true;
    let doc = p.document();
    let r = p.revision;
    p.script.follow(|| doc, r, true);
    assert!(p.script.text.contains("output height"), "{}", p.script.text);
    // Written again, the graph is the same.
    super::script::apply(&mut p).unwrap();
    assert!(p.document().same_terrain(&before));
    // A script of its own, applied: one step back to the graph before.
    let pos = p.document().ui.values().next().map(|u| u.pos);
    let first = p.document().ui.keys().next().cloned().unwrap();
    p.script.text = format!("{}\nextra = FractalNoise(scale: 300);\n", p.script.text.trim_end().trim_end_matches('}'));
    p.script.text.push_str("}\n");
    p.script.check();
    assert!(p.script.errors.iter().all(|e| e.warning), "{:?}", p.script.errors);
    super::script::apply(&mut p).unwrap();
    assert!(p.document().nodes.contains_key("extra"));
    assert_eq!(p.document().ui.get(&first).map(|u| u.pos), pos, "kept where it was");
    assert!(p.undo());
    assert!(!p.document().nodes.contains_key("extra"));
    // A mistake leaves the graph as it is.
    p.script.text = "a = FractalNoize();".into();
    p.script.check();
    assert!(p.script.errors[0].message.contains("FractalNoise"));
    assert!(super::script::apply(&mut p).is_err());
    assert!(super::script::guide_for(&p).contains("- FractalNoise("));
}

/// The Script pane beside the graph, with a mistake underlined.
#[test]
fn the_script_pane_renders() {
    let Some(dir) = snapshot_dir() else { return };
    let Some(gpu) = Gpu::headless() else { return };
    let engine = Arc::new(Engine::with_gpu(gpu, Default::default()));
    use egui::Theme::{Dark, Light};
    for (theme, tag) in [(Dark, "dark"), (Light, "light")] {
        render_editor(&dir, &format!("script-pane-{tag}"), theme, 1.0, engine.clone(), example("m4-alpine-colors"), |p| {
            p.script.open = true;
        });
    }
    let src = massif_terrain::mscript::EXAMPLES[0].1.replace("SlopeMask(eroded)", "SlopeMsk(eroded)");
    render_editor(&dir, "script-pane-error-dark-2x", Dark, 2.0, engine.clone(), massif_terrain::mscript::build(massif_terrain::mscript::EXAMPLES[0].1).unwrap(), move |p| {
        p.script.open = true;
        p.script.text = src.clone();
        p.script.origin = super::script::Origin::Own;
    });
}

/// The canvas zooms around the pointer with a mouse wheel or a pinch, over
/// a wide range, and slides with two fingers on a trackpad; ⌘+ and ⌘−
/// zoom, F and a double-click on the ground show the whole graph; the
/// minimap shows when the graph does not fit, follows the view, and a click
/// in it centres the view there.
#[test]
fn the_canvas_zooms_and_its_minimap_moves_it() {
    use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, TouchPhase, pos2, vec2};
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1600.0, 1000.0)).with_accessibility_check(false).build_ui_state(
        |ui, p: &mut super::TerrainPanel| {
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::theme::install(ui.ctx());
            }
            p.ui(ui);
        },
        super::TerrainPanel::with_graph(Arc::new(Engine::new(Default::default())), example("m4-alpine-colors")),
    );
    h.run_steps(3);
    let canvas = h.state().canvas_rect;
    let at = canvas.center() + vec2(-120.0, 40.0);
    let graph_at = |p: &super::TerrainPanel| (at - canvas.min - p.view.offset) / p.view.zoom;
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(1);
    let (z0, under) = (h.state().view.zoom, graph_at(h.state()));
    // A mouse wheel: closer, the point under the pointer staying there.
    for _ in 0..12 {
        h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta: vec2(0.0, 3.0), phase: TouchPhase::Move, source: MouseWheelSource::Wheel, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    let p = h.state();
    assert!(p.view.zoom > z0 * 2.5 && p.view.zoom <= super::canvas::ZOOM_MAX, "{z0} → {}", p.view.zoom);
    assert!((graph_at(p) - under).length() < 1.0, "the point under the pointer stays");
    // Two fingers on a trackpad slide, never zoom.
    let (z1, o1) = (p.view.zoom, p.view.offset);
    h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(30.0, -50.0), phase: TouchPhase::Move, source: MouseWheelSource::Trackpad, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(h.state().view.zoom, z1);
    assert_eq!(h.state().view.offset, o1 + vec2(30.0, -50.0));
    // Close in, part of the graph is out of view: the minimap, whose click
    // centres the view on what it shows there.
    let mini = super::canvas::minimap_rect(h.state(), canvas).expect("the minimap shows");
    assert!(mini.left() < canvas.center().x && mini.bottom() <= canvas.bottom(), "at the bottom left");
    let before = h.state().view.offset;
    h.input_mut().events.push(Event::PointerMoved(mini.center()));
    h.input_mut().events.push(Event::PointerButton { pos: mini.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerButton { pos: mini.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_ne!(h.state().view.offset, before, "the view moved there");
    assert_eq!(h.state().view.zoom, z1, "the click did not zoom");
    // The wheel over the minimap zooms the canvas (around its middle).
    h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta: vec2(0.0, -3.0), phase: TouchPhase::Move, source: MouseWheelSource::Wheel, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().view.zoom < z1);
    // ⌘− from the keyboard; F shows it all, as far out as it takes, and the
    // minimap goes.
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(1);
    let z2 = h.state().view.zoom;
    h.input_mut().events.push(Event::Key { key: egui::Key::Minus, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run_steps(1);
    assert!((h.state().view.zoom - z2 * 0.8).abs() < 1e-3, "{z2} → {}", h.state().view.zoom);
    h.input_mut().events.push(Event::Key { key: egui::Key::F, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(super::canvas::minimap_rect(h.state(), canvas).is_none(), "all of it in view");
    // Far out, a double-click on the ground frames it again.
    for _ in 0..30 {
        h.input_mut().events.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta: vec2(0.0, -3.0), phase: TouchPhase::Move, source: MouseWheelSource::Wheel, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    assert!((h.state().view.zoom - super::canvas::ZOOM_MIN).abs() < 1e-4, "as far out as it goes: {}", h.state().view.zoom);
    let ground = pos2(canvas.right() - 30.0, canvas.top() + 30.0);
    for _ in 0..2 {
        h.input_mut().events.push(Event::PointerButton { pos: ground, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.input_mut().events.push(Event::PointerButton { pos: ground, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
    assert!(h.state().view.zoom > super::canvas::ZOOM_MIN * 2.0, "framed: {}", h.state().view.zoom);
}

/// The 3D viewer's camera keeps its distance as it turns: turning never
/// comes closer nor goes away, only the wheel does.
#[test]
fn the_3d_viewer_keeps_its_distance_as_it_turns() {
    let target = [600.0, 50.0, 400.0];
    let dist = |yaw: f32, pitch: f32, d: f32| {
        let (eye, ..) = super::shade::orbit(1200.0, 800.0, 0.0, 100.0, yaw, pitch, d, 1.6, 0.5);
        ((eye[0] - target[0]).powi(2) + (eye[1] - target[1]).powi(2) + (eye[2] - target[2]).powi(2)).sqrt()
    };
    let d0 = dist(25.0, 34.0, 1.0);
    for (yaw, pitch) in [(0.0, 34.0), (90.0, 34.0), (200.0, 15.0), (310.0, 70.0)] {
        assert!((dist(yaw, pitch, 1.0) - d0).abs() < 1e-2 * d0, "{yaw}°, {pitch}°");
    }
    assert!((dist(90.0, 50.0, 0.5) - d0 * 0.5).abs() < 1e-2 * d0, "the wheel's distance alone");
}

/// While a wire is drawn near the Map Output, its folded groups holding an
/// input that takes it open (Underground for caves), and fold again once it
/// goes; folded, a group says how many inputs it holds.
#[test]
fn groups_open_for_a_wire_drawn_near() {
    use super::canvas::Row;
    let mut p = cpu_panel();
    p.set_graph(super::TerrainPanel::starter());
    let open = |p: &super::TerrainPanel, key: &str| super::canvas::rows(p, "out").iter().find_map(|r| match r {
        Row::Group { key: k, open, total, .. } if *k == key => Some((*open, *total)),
        _ => None,
    });
    let (folded, total) = open(&p, "underground").expect("an Underground group");
    assert!(!folded && total >= 2, "folded, with its inputs counted: {total}");
    p.magnet = Some(("out".into(), massif_terrain::registry::PortKind::Layer));
    assert!(open(&p, "underground").unwrap().0, "a layer drawn near: Underground opens");
    assert!(!open(&p, "climate").unwrap().0, "Climate takes no layer: it stays folded");
    p.magnet = None;
    assert!(!open(&p, "underground").unwrap().0, "folded again once the wire goes");
}

/// A graph with no Map Output gets one from the warning's button, its
/// Height fed by the rightmost node making heights.
#[test]
fn a_map_output_is_added_when_missing() {
    let mut p = cpu_panel();
    let mut g = example("m4-alpine-colors");
    g.nodes.remove("out");
    g.ui.remove("out");
    p.set_graph(g);
    assert!(!p.graph.nodes.values().any(|n| n.kind.starts_with("output.map")));
    let id = super::canvas::add_map_output(&mut p);
    assert!(p.graph.nodes[&id].kind.starts_with("output.map"));
    assert!(p.graph.nodes[&id].inputs.contains_key("height"), "its Height fed");
    assert!(p.pos(&id)[0] > p.pos("alps")[0], "right of the graph");
}
