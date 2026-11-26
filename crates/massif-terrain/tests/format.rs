//! The format as a model sees it: the schema and the catalogue name every
//! node and are written out; examples build; checking points at mistakes;
//! edits a step at a time and JSON Patches.

use massif_terrain::compile::{Diagnostic, Severity, compile};
use massif_terrain::engine::{Engine, never};
use massif_terrain::format::{self, Op};
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::path::PathBuf;

fn errors(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags.iter().filter(|d| d.severity == Severity::Error).collect()
}

/// The error at `path`, whose message says `words`.
fn says(diags: &[Diagnostic], path: &str, words: &str) {
    assert!(diags.iter().any(|d| d.path == path && d.message.contains(words)), "no “{words}” at {path} in {diags:#?}");
}

#[test]
fn every_example_builds() {
    for spec in format::public() {
        let g = format::example(spec);
        let (c, diags) = compile(&g, Space::new(4096), 512);
        assert!(errors(&diags).is_empty(), "{}: {diags:#?}\n{}", spec.id, format::compact_json(&g));
        let e = Engine::new(Default::default());
        let h = e.whole(&c, c.output("height").expect("the example feeds the Map Output's height"), 5, &never).unwrap();
        assert!(h.data.iter().all(|v| v.is_finite()), "{}", spec.id);
        // The example uses the node.
        assert!(g.nodes.values().any(|n| n.kind == spec.type_name()), "{}", spec.type_name());
    }
}

#[test]
fn the_schema_names_every_node() {
    let schema = format::schema();
    let defs = schema["$defs"].as_object().unwrap();
    for spec in format::public() {
        let d = &defs[&spec.type_name()];
        assert_eq!(d["properties"]["type"]["const"], spec.type_name());
        for p in spec.params {
            assert!(d["properties"]["params"]["properties"].get(p.name).is_some(), "{}.{}", spec.id, p.name);
        }
    }
    assert!(defs.keys().all(|k| !k.starts_with("convert.")), "no internal nodes");
    let fbm = &defs["noise.fbm"]["properties"];
    assert_eq!(fbm["params"]["properties"]["height"]["default"], 200.0);
    assert_eq!(fbm["params"]["properties"]["height"]["x-unit"], "blocks");
    assert_eq!(fbm["params"]["properties"]["height"]["x-drivable"], true);
    assert!(fbm["inputs"]["properties"].get("@height").is_some());
    let out = &defs["output.map"]["properties"]["inputs"];
    assert!(out["patternProperties"].get("^mask\\.[A-Za-z0-9_-]{1,64}$").is_some(), "{out:#}");
    assert_eq!(defs["adjust.levels"]["properties"]["inputs"]["required"], serde_json::json!(["in"]));
    // Every node of the catalogue, by family, with an example.
    let cat = format::catalog_json();
    let listed: usize = cat["families"].as_array().unwrap().iter().map(|f| f["nodes"].as_array().unwrap().len()).sum();
    assert_eq!(listed, format::current().count());
    assert!(cat["families"][0]["nodes"][0]["example"]["nodes"].is_object());
}

#[test]
fn checking_points_at_mistakes() {
    let ok = r#"{ "nodes": { "hills": { "type": "noise.fbm", "params": { "height": 300 } }, "out": { "type": "output.map", "inputs": { "height": "hills" } } } }"#;
    assert!(errors(&format::validate(ok)).is_empty());
    let d = format::validate(r#"{ "nodes": { "hills": { "type": "noise.fbm", "params": { "hieght": 300 } } } }"#);
    says(&d, "nodes.hills.params.hieght", "did you mean “height”");
    let d = format::validate(r#"{ "nods": {} }"#);
    says(&d, "nods", "did you mean “nodes”");
    let d = format::validate(r#"{ "nodes": { "a": { "type": "noise.fbm", "param": {} } } }"#);
    says(&d, "nodes.a.param", "did you mean “params”");
    let d = format::validate(r#"{ "nodes": { "a": { "type": "noise.fmb" } } }"#);
    assert!(d.iter().any(|d| d.path.starts_with("nodes.a") && d.message.contains("noise.fbm")), "{d:#?}");
    let d = format::validate(r#"{ "nodes": { "a": { "type": "noise.fbm", "inputs": { "x": 3 } } } }"#);
    says(&d, "nodes.a.inputs.x", "names the node feeding it");
    says(&format::validate("{ nodes: }"), "", "not JSON");
    says(&format::validate(r#"{ "version": 9, "nodes": {} }"#), "version", "newer");
    says(&format::validate(r#"{ "format": "something", "nodes": {} }"#), "format", "massif.terrain-graph");
    // Out of range: taken, with a warning.
    let d = format::validate(r#"{ "nodes": { "a": { "type": "noise.fbm", "params": { "height": 99999 } } } }"#);
    assert!(errors(&d).is_empty() && d.iter().any(|d| d.path == "nodes.a.params.height"), "{d:#?}");
}

#[test]
fn edits_a_step_at_a_time() {
    let ops: Vec<Op> = serde_json::from_str(
        r#"[
        { "op": "add", "type": "noise.fbm", "id": "hills", "params": { "scale": 900 } },
        { "op": "add", "type": "noise.fbm" },
        { "op": "add", "type": "output.map", "id": "out" },
        { "op": "link", "from": "hills", "to": "out", "input": "height" },
        { "op": "set", "node": "hills", "param": "height", "value": 300 },
        { "op": "set", "node": "hills", "param": "hieght", "value": 300 },
        { "op": "link", "from": "hills", "to": "out", "input": "hieght" },
        { "op": "link", "from": "hill", "to": "out", "input": "height" },
        { "op": "add", "type": "noise.fmb" },
        { "op": "add", "type": "noise.fbm", "id": "hills" },
        { "op": "rename", "node": "hills", "to": "Alps" },
        { "op": "link", "from": "fbm", "to": "hills", "input": "@height" }
    ]"#,
    )
    .unwrap();
    let mut g = Graph::default();
    let d = format::patch(&mut g, &ops);
    says(&d, "ops.5", "did you mean “height”");
    says(&d, "ops.6", "did you mean “height”");
    says(&d, "ops.7", "did you mean “hills”");
    says(&d, "ops.8", "did you mean “noise.fbm”");
    says(&d, "ops.9", "already");
    assert_eq!(errors(&d).len(), 5, "{d:#?}");
    assert_eq!(g.nodes["hills"].params["height"], 300);
    assert_eq!(g.nodes["hills"].name.as_deref(), Some("Alps"), "renamed, its id kept");
    assert_eq!(g.nodes["out"].inputs["height"], "hills");
    assert_eq!(g.nodes["hills"].inputs["@height"], "fbm");
    // Unlinking, setting back to the default, removing with the links.
    let mut more = vec![Op::Unlink { to: "hills".into(), input: "@height".into() }, Op::Set { node: "hills".into(), param: "height".into(), value: serde_json::Value::Null }, Op::Remove { node: "hills".into() }];
    assert!(errors(&format::patch(&mut g, &more)).is_empty());
    assert!(!g.nodes.contains_key("hills") && !g.nodes["out"].inputs.contains_key("height"));
    more = vec![Op::Unlink { to: "out".into(), input: "height".into() }];
    says(&format::patch(&mut g, &more), "ops.0", "nothing feeds");
    // An operation misspelt is not taken.
    assert!(serde_json::from_str::<Op>(r#"{ "op": "set", "node": "a", "parm": "x", "value": 1 }"#).is_err());
    // What a model wrote gets places in the editor, sources to the left.
    let mut g = Graph::from_json(r#"{ "nodes": { "a": { "type": "noise.fbm" }, "b": { "type": "adjust.levels", "inputs": { "in": "a" } }, "out": { "type": "output.map", "inputs": { "height": "b" } } } }"#).unwrap();
    format::lay_out(&mut g);
    assert!(g.ui["a"].pos[0] < g.ui["b"].pos[0] && g.ui["b"].pos[0] < g.ui["out"].pos[0]);
}

/// Version 3's steps: frames, bypass, composites made, shown, edited inside
/// and unmade.
#[test]
fn edits_frames_and_composites() {
    let mut g = Graph::from_json(
        r#"{ "nodes": {
            "hills": { "type": "noise.fbm", "params": { "height": 120 } },
            "lift": { "type": "adjust.levels", "inputs": { "in": "hills" } },
            "out": { "type": "output.map", "inputs": { "height": "lift" } }
        } }"#,
    )
    .unwrap();
    let ops: Vec<Op> = serde_json::from_str(
        r##"[
        { "op": "frame", "title": "Base terrain", "color": "#347a52", "members": ["hills", "lift"] },
        { "op": "frame", "id": "outer", "title": "All", "members": ["out"] },
        { "op": "frame", "id": "frame_1", "title": "Base terrain", "members": ["hills", "lift"], "parent": "outer" },
        { "op": "frame", "id": "outer", "title": "All", "parent": "frame_1" },
        { "op": "bypass", "node": "lift", "on": true },
        { "op": "bypass", "node": "lift", "on": false },
        { "op": "group", "nodes": ["hills", "lift"], "label": "My hills" },
        { "op": "expose", "component": "project:my_hills", "name": "tall", "label": "Height", "targets": ["hills.height"], "unit": "blocks" },
        { "op": "set", "node": "my_hills", "param": "tall", "value": 200 },
        { "op": "set", "node": "my_hills", "param": "tal", "value": 200 },
        { "op": "inside", "component": "project:my_hills", "ops": [{ "op": "set", "node": "lift", "param": "gamma", "value": 1.3 }] },
        { "op": "add", "type": "massif:mountain", "id": "alps", "params": { "height": 400 } },
        { "op": "add", "type": "massif:nothing" }
    ]"##,
    )
    .unwrap();
    let d = format::patch(&mut g, &ops);
    says(&d, "ops.3", "cannot sit in itself");
    says(&d, "ops.9", "did you mean “tall”");
    says(&d, "ops.12", "no composite");
    assert_eq!(errors(&d).len(), 3, "{d:#?}");
    assert_eq!(g.frames["frame_1"].parent.as_deref(), Some("outer"));
    assert!(g.frames["frame_1"].members.is_empty(), "its nodes went into the composite");
    assert_eq!(g.nodes["my_hills"].params["tall"], 200);
    assert_eq!(g.components["project:my_hills"].nodes["lift"].params["gamma"], 1.3);
    assert_eq!(g.nodes["out"].inputs["height"], "my_hills.lift");
    // The document reads back as it was written.
    assert_eq!(Graph::from_json(&g.to_json()).unwrap(), g);
    assert!(format::validate(&g.to_json()).iter().all(|d| d.severity != Severity::Error), "{:#?}", format::validate(&g.to_json()));
    // A composite's interface edited; inside, `in.<name>` links.
    let ops: Vec<Op> = serde_json::from_str(
        r#"[
        { "op": "port", "component": "project:my_hills", "decl": { "name": "base", "type": "height", "optional": true } },
        { "op": "inside", "component": "project:my_hills", "ops": [{ "op": "link", "from": "in.base", "to": "lift", "input": "in" }] },
        { "op": "inside", "component": "project:my_hills", "ops": [{ "op": "link", "from": "in.nope", "to": "lift", "input": "in" }] },
        { "op": "output", "component": "project:my_hills", "decl": { "name": "noise", "from": "hills" } },
        { "op": "port", "component": "project:my_hills", "decl": { "name": "x", "type": "heigth" } },
        { "op": "fork", "node": "alps", "label": "My mountain" }
    ]"#,
    )
    .unwrap();
    let d = format::patch(&mut g, &ops);
    says(&d, "ops.2", "no input “nope”");
    says(&d, "ops.4", "did you mean “height”");
    assert_eq!(errors(&d).iter().filter(|e| e.path.starts_with("ops.")).count(), 2, "{d:#?}");
    assert_eq!(g.components["project:my_hills"].nodes["lift"].inputs["in"], "in.base");
    assert_eq!(g.nodes["alps"].kind, "project:my_mountain");
    assert_eq!(g.components["project:my_mountain"].label, "My mountain");
    let d = format::patch(&mut g, &[Op::Unport { component: "project:my_hills".into(), name: "base".into() }, Op::Unoutput { component: "project:my_hills".into(), name: "lift".into() }]);
    assert!(errors(&d).iter().all(|e| !e.path.starts_with("ops.")), "{d:#?}");
    assert!(!g.components["project:my_hills"].nodes["lift"].inputs.contains_key("in"));
    assert!(!g.nodes["out"].inputs.contains_key("height"), "what read the output is unlinked");
    let d = format::patch(&mut g, &[Op::Ungroup { node: "my_hills".into() }, Op::Unframe { id: "outer".into() }]);
    assert!(errors(&d).iter().all(|e| !e.path.starts_with("ops.")), "{d:#?}");
    assert_eq!(g.nodes["my_hills_hills"].params["height"], 200);
    assert_eq!(g.frames["frame_1"].parent, None);
}

#[test]
fn json_patches_apply_whole_or_not_at_all() {
    let mut g = Graph::from_json(r#"{ "nodes": { "hills": { "type": "noise.fbm", "params": { "height": 100 } } } }"#).unwrap();
    let d = format::json_patch(
        &mut g,
        &serde_json::json!([
            { "op": "test", "path": "/nodes/hills/params/height", "value": 100 },
            { "op": "replace", "path": "/nodes/hills/params/height", "value": 250 },
            { "op": "add", "path": "/nodes/out", "value": { "type": "output.map", "inputs": { "height": "hills" } } },
            { "op": "copy", "from": "/nodes/hills", "path": "/nodes/more" },
            { "op": "move", "from": "/nodes/more", "path": "/nodes/other~1name" }
        ]),
    );
    // “other/name” cannot name a node: nothing was applied.
    says(&d, "nodes.other/name", "letters, digits");
    assert_eq!(g.nodes["hills"].params["height"], 100);
    assert!(!g.nodes.contains_key("out"));
    let d = format::json_patch(
        &mut g,
        &serde_json::json!([
            { "op": "replace", "path": "/nodes/hills/params/height", "value": 250 },
            { "op": "add", "path": "/nodes/out", "value": { "type": "output.map", "inputs": { "height": "hills" } } },
            { "op": "copy", "from": "/nodes/hills", "path": "/nodes/more" },
            { "op": "remove", "path": "/nodes/more/params" }
        ]),
    );
    assert!(errors(&d).is_empty(), "{d:#?}");
    assert_eq!(g.nodes["hills"].params["height"], 250);
    assert!(g.nodes["more"].params.is_empty() && g.nodes["out"].inputs["height"] == "hills");
    let d = format::json_patch(&mut g, &serde_json::json!([{ "op": "test", "path": "/nodes/hills/params/height", "value": 1 }]));
    says(&d, "0", "failed");
    let d = format::json_patch(&mut g, &serde_json::json!([{ "op": "remove", "path": "/nodes/nothing" }]));
    says(&d, "0", "nothing at");
}

#[test]
fn the_docs_are_current() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let schema = serde_json::to_string_pretty(&format::schema()).unwrap() + "\n";
    let catalog = format::catalog_markdown();
    let files = [("terrain-graph.schema.json", schema), ("terrain-nodes.md", catalog)];
    if std::env::var_os("MASSIF_WRITE_DOCS").is_some() {
        for (name, text) in &files {
            std::fs::write(dir.join(name), text).unwrap();
        }
    }
    for (name, text) in &files {
        let on_disk = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        assert!(on_disk == *text, "docs/{name} is out of date: MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --test format");
    }
}
