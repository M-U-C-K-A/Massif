use super::*;
use crate::compile::Severity;
use crate::model::{Frame, Node};

fn ok(src: &str) -> Graph {
    let out = compile(src, &mut NoImports);
    assert!(out.ok(), "{}", out.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"));
    out.graph.unwrap()
}

fn first_error(src: &str) -> Error {
    let out = compile(src, &mut NoImports);
    assert!(!out.ok(), "compiled: {src}");
    out.errors.into_iter().find(|e| !e.warning).expect("an error")
}

/// The graph without the editor's layout (places, frames' rectangles).
fn shape(g: &Graph) -> Graph {
    let strip = |f: &BTreeMap<String, Frame>| f.iter().map(|(k, f)| (k.clone(), Frame { rect: [0.0; 4], ..f.clone() })).collect();
    let mut g = g.clone();
    g.ui.clear();
    g.frames = strip(&g.frames);
    for c in g.components.values_mut() {
        c.ui.clear();
        c.frames = strip(&c.frames);
    }
    g
}

fn no_errors(g: &Graph) {
    let d = crate::format::check(g);
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
}

#[test]
fn the_examples_build() {
    for (title, src) in EXAMPLES {
        let g = ok(src);
        no_errors(&g);
        assert!(g.nodes.contains_key("out"), "{title}");
    }
    let alpine = ok(EXAMPLES[0].1);
    assert_eq!(alpine.terrain.seed, 4268);
    assert_eq!(alpine.nodes["base"].kind, "noise.fbm");
    assert_eq!(alpine.nodes["ridges"].params["style"], "ridged");
    assert_eq!(alpine.nodes["mountains"].kind, "combine.blend");
    assert_eq!(alpine.nodes["mountains"].inputs["a"], "base");
    assert_eq!(alpine.nodes["mountains"].params["mode"], "add");
    assert_eq!(alpine.nodes["out"].inputs["height"], "eroded");
    assert_eq!(alpine.nodes["out"].inputs["mask.flat"], "flat");
    let base = &alpine.frames["base_terrain"];
    assert_eq!((base.title.as_str(), base.color.as_deref()), ("Base Terrain", Some("#347a52")));
    assert_eq!(base.members, ["base", "ridges", "mountains", "lifted"]);
    assert!(base.rect[2] > 0.0 && base.rect[3] > 0.0);
    // The component: its settings exposed, used twice, the loop's nodes.
    let twins = ok(EXAMPLES[1].1);
    let c = &twins.components["project:MountainGenerator"];
    assert_eq!(c.label, "Mountain generator");
    assert_eq!(c.exposed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["seed", "height", "roughness"]);
    assert_eq!(c.exposed[1].targets, ["base.height"]);
    assert_eq!((c.exposed[1].label.as_str(), c.exposed[1].max), ("Mountain height", Some(2000.0)));
    assert_eq!(c.exposed[0].targets, ["base.seed", "ridges.seed"]);
    assert_eq!(c.nodes["base"].params["height"], 800);
    assert_eq!(twins.nodes["west"].kind, "project:MountainGenerator");
    assert_eq!(twins.nodes["west"].params["height"], 900);
    assert_eq!(twins.nodes["step_2"].params["steps"], 12);
    assert_eq!(twins.nodes["step_1"].inputs["in"], "both");
    assert_eq!(twins.nodes["out"].inputs["height"], "step_2");
}

/// A graph from a script evaluates like one built by hand.
#[test]
fn a_script_makes_a_terrain() {
    use crate::engine::{Engine, never};
    use crate::space::Space;
    let g = ok(EXAMPLES[1].1);
    let (c, d) = crate::compile::compile(&g, Space::new(512), 128);
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    let lod = c.space.lod_for(128);
    let tile = Engine::new(Default::default()).region(&c, c.output("height").unwrap(), lod, c.space.whole(lod), &never).unwrap();
    assert!(tile.data.iter().any(|h| *h != tile.data[0]));
}

#[test]
fn the_same_script_makes_the_same_graph() {
    for (_, src) in EXAMPLES {
        assert_eq!(ok(src), ok(src));
    }
}

/// script → graph → script → graph: the same graph, the same script.
#[test]
fn round_trip() {
    for (title, src) in EXAMPLES {
        let g1 = ok(src);
        let s2 = decompile(&g1, Some("Again"));
        let g2 = ok(&s2);
        assert_eq!(shape(&g1), shape(&g2), "{title}:\n{s2}");
        assert_eq!(s2, decompile(&g2, Some("Again")), "{title}");
    }
}

/// Every type of node, in the catalogue's example of it, written as a
/// script and made again.
#[test]
fn any_graph_round_trips() {
    for s in crate::format::current() {
        let mut g = crate::format::example(s);
        g.terrain.seed = 9;
        let text = decompile(&g, None);
        let again = ok(&text);
        assert_eq!(shape(&g).nodes, shape(&again).nodes, "{}:\n{text}", s.id);
        assert_eq!(again.terrain, g.terrain, "{}", s.id);
    }
}

/// Composites of the document, frames and names survive the trip too:
/// each shipped composite copied into a document, grouped and framed.
#[test]
fn composites_round_trip() {
    for (kind, _, _) in crate::components::all() {
        let mut g = Graph::default();
        let id = g.add(Node::new(kind));
        crate::format::apply(&mut g, &crate::format::Op::Fork { node: id.clone(), label: "My copy".into() }).unwrap();
        let relief = g.add(Node::new("noise.fbm").with("scale", 700));
        crate::format::apply(&mut g, &crate::format::Op::Frame { id: Some("frame_1".into()), title: "Odd id".into(), color: Some("#123456".into()), description: "A note".into(), members: vec![relief.clone()], parent: None }).unwrap();
        g.nodes.get_mut(&relief).unwrap().name = Some("Hills".into());
        g.nodes.get_mut(&relief).unwrap().note = Some("first \"one\"".into());
        let c = g.components.values().next().unwrap().clone();
        if let Some(i) = c.inputs.first() {
            g.link(&relief, &id, &i.name);
        }
        let text = decompile(&g, Some("Copies"));
        let again = ok(&text);
        assert_eq!(shape(&g).nodes, shape(&again).nodes, "{kind}:\n{text}");
        assert_eq!(shape(&g).frames, shape(&again).frames, "{kind}:\n{text}");
        let (a, b) = (shape(&g).components, shape(&again).components);
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        for (k, ca) in &a {
            let cb = &b[k];
            assert_eq!((&ca.label, &ca.inputs, &ca.outputs, &ca.nodes), (&cb.label, &cb.inputs, &cb.outputs, &cb.nodes), "{kind}:\n{text}");
            // Targets on several nodes come in the order the nodes are written: their order means nothing.
            let sorted = |t: &Vec<String>| {
                let mut t = t.clone();
                t.sort();
                t
            };
            let names = |c: &Component| c.exposed.iter().map(|e| (e.name.clone(), sorted(&e.targets), e.label.clone(), e.min, e.max, e.unit.clone())).collect::<Vec<_>>();
            assert_eq!(names(ca), names(cb), "{kind}:\n{text}");
        }
    }
}

#[test]
fn errors_say_where_and_what() {
    let e = first_error("project \"x\" {\n    a = FractalNoize(scale: 3);\n}");
    assert_eq!((e.span.line, e.span.col), (2, 9));
    assert!(e.message.contains("unknown node “FractalNoize”") && e.message.contains("FractalNoise"), "{}", e.message);
    let e = first_error("a = FractalNoise(scael: 300);");
    assert!(e.message.contains("no setting “scael”") && e.message.contains("scale"), "{}", e.message);
    let e = first_error("a = FractalNoise();\nb = SlopeMask(a);\nc = Terraces(b, range: \"giveen\");");
    assert_eq!(e.span.line, 3);
    assert!(e.message.contains("given"), "{}", e.message);
    let e = first_error("a = FractalNoise()\nb = Blur(a);");
    assert!(e.message.contains("“;”") && e.span.line == 1, "{e}");
    let e = first_error("a = TerrainLayers();\nb = Blur(a);");
    assert!(e.message.contains("wrong type") && e.message.contains("a terrain") && e.message.contains("a height"), "{}", e.message);
    let e = first_error("a = FractalNoise();\noutput heigth = a;");
    assert!(e.message.contains("height"), "{}", e.message);
    let e = first_error("for i in 0..2 { a = FractalNoise(); }");
    assert!(e.message.contains("a[i]"), "{}", e.message);
    let e = first_error("a = FractalNoise(scale: b);");
    assert!(e.message.contains("“b” is not defined"), "{}", e.message);
    let e = first_error("a = FractalNoise(); b = Blur(a.flow);");
    assert!(e.message.contains("no output “flow”"), "{}", e.message);
    let e = first_error("/* never closed");
    assert!(e.message.contains("never closed"), "{}", e.message);
    // A warning does not stop it.
    let out = compile("a = FractalNoise(scale: 1);", &mut NoImports);
    assert!(out.ok() && out.errors.iter().any(|e| e.warning && e.message.contains("brought back")));
}

#[test]
fn language() {
    let g = ok(r##"
        // Constants, conditions, loops, lists, nested calls.
        const n = 3;
        let total = 0;
        for k in [1, 2, 3] { total = total + k; }
        base = FractalNoise(scale: 400 * n, height: total * 10 + 2 ^ 3, octaves: clamp(n * 4, 1, 10));
        if total > 5 && !false {
            soft = Blur(base) { name: "Soft"; note: "smooth"; };
        } else {
            soft = Sharpen(base);
        }
        mixed = Blend(base, soft, mask: InvertMask(SlopeMask(soft)), mode: n == 3 ? "max" : "min");
        output height = mixed;
    "##);
    assert_eq!(g.nodes["base"].params["scale"], 1200);
    assert_eq!(g.nodes["base"].params["height"], 68);
    assert_eq!(g.nodes["base"].params["octaves"], 10);
    assert_eq!(g.nodes["soft"].kind, "adjust.blur");
    assert_eq!((g.nodes["soft"].name.as_deref(), g.nodes["soft"].note.as_deref()), (Some("Soft"), Some("smooth")));
    assert_eq!(g.nodes["mixed"].params["mode"], "max");
    assert_eq!(g.nodes["mixed"].inputs["mask"], "mixed_invert_mask");
    assert_eq!(g.nodes["mixed_invert_mask"].inputs["mask"], "mixed_invert_mask_slope_mask");
    no_errors(&g);
}

#[test]
fn imports() {
    let mut files = BTreeMap::from([
        ("lib.mscript".to_owned(), "component Hills(scale: float = 500) { h = FractalNoise(scale: scale); output h; }\nconst tall = 300;".to_owned()),
        ("loop.mscript".to_owned(), "import \"loop.mscript\";".to_owned()),
    ]);
    let out = compile("import \"lib.mscript\";\nimport \"lib.mscript\";\nh = Hills(scale: tall);\noutput height = h;", &mut files);
    assert!(out.ok(), "{:?}", out.errors);
    let g = out.graph.unwrap();
    assert_eq!(g.nodes["h"].params["scale"], 300);
    assert!(g.components.contains_key("project:Hills"));
    let out = compile("import \"loop.mscript\";", &mut files);
    assert!(out.errors[0].message.contains("imports itself"), "{:?}", out.errors);
    let out = compile("import \"none.mscript\";", &mut files);
    assert!(!out.ok());
}

/// The steps a script makes are the editor's: applied one by one with
/// `format::patch`, they make the same graph.
#[test]
fn steps_are_the_editors() {
    let out = compile(EXAMPLES[0].1, &mut NoImports);
    let p = out.program.unwrap();
    let mut g = Graph { terrain: p.terrain.clone(), ..Graph::default() };
    let d = crate::format::patch(&mut g, &p.ops());
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    assert_eq!(shape(&g), shape(&out.graph.unwrap()));
}

/// The guide holds every node a script can call, and the examples.
#[test]
fn the_guide_holds_every_node() {
    let text = guide(Some("project \"Open\" { }\n"));
    for (name, s) in names::catalogue() {
        assert!(text.contains(&format!("- {name}(")), "{} ({name}) is missing", s.id);
        for p in s.params {
            assert!(text.contains(p.name), "{}.{}", s.id, p.name);
        }
    }
    for (kind, _, _) in crate::components::offered() {
        assert!(text.contains(&format!("- {}(", names::name_of(kind).unwrap())), "{kind}");
    }
    assert!(text.contains("project \"Open\"") && text.contains("```mscript") && text.contains("Twin peaks"));
}

/// Kept places stay; new nodes get one.
#[test]
fn layout_keeps_places() {
    let mut g = ok(EXAMPLES[0].1);
    g.ui.get_mut("base").unwrap().pos = [-999.0, -999.0];
    let mut again = ok(EXAMPLES[0].1);
    lay_out(&mut again, Some(&g));
    assert_eq!(again.ui["base"].pos, [-999.0, -999.0]);
    assert!(again.frames["base_terrain"].rect[0] < -999.0);
    // Left to right: what feeds a node sits left of it.
    let g = ok(EXAMPLES[0].1);
    assert!(g.ui["base"].pos[0] < g.ui["mountains"].pos[0] && g.ui["mountains"].pos[0] < g.ui["lifted"].pos[0]);
}

/// `docs/massifscript-reference.md` is the reference as the registry
/// writes it (`MASSIF_WRITE_DOCS=1` writes it again).
#[test]
fn the_reference_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/massifscript-reference.md");
    let text = format!("<!-- Written by massif-terrain (mscript::reference); do not edit by hand: `MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --lib mscript` writes it again. -->\n\n{}", reference());
    if std::env::var_os("MASSIF_WRITE_DOCS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert!(std::fs::read_to_string(&path).is_ok_and(|t| t == text), "docs/massifscript-reference.md is out of date: MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --lib mscript");
}
