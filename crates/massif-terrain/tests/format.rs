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
        assert!(g.nodes.values().any(|n| n.kind == spec.id), "{}", spec.id);
    }
}

#[test]
fn the_schema_names_every_node() {
    let schema = format::schema();
    let defs = schema["$defs"].as_object().unwrap();
    for spec in format::public() {
        let d = &defs[spec.id];
        assert_eq!(d["properties"]["type"]["const"], spec.id);
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
    assert_eq!(listed, format::public().count());
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
        { "op": "rename", "node": "hills", "to": "alps" },
        { "op": "link", "from": "fbm", "to": "alps", "input": "@height" }
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
    assert_eq!(g.nodes["alps"].params["height"], 300);
    assert_eq!(g.nodes["out"].inputs["height"], "alps", "renaming follows the links");
    assert_eq!(g.nodes["alps"].inputs["@height"], "fbm");
    // Unlinking, setting back to the default, removing with the links.
    let mut more = vec![Op::Unlink { to: "alps".into(), input: "@height".into() }, Op::Set { node: "alps".into(), param: "height".into(), value: serde_json::Value::Null }, Op::Remove { node: "alps".into() }];
    assert!(errors(&format::patch(&mut g, &more)).is_empty());
    assert!(!g.nodes.contains_key("alps") && !g.nodes["out"].inputs.contains_key("height"));
    more = vec![Op::Unlink { to: "out".into(), input: "height".into() }];
    says(&format::patch(&mut g, &more), "ops.0", "nothing feeds");
    // An operation misspelt is not taken.
    assert!(serde_json::from_str::<Op>(r#"{ "op": "set", "node": "a", "parm": "x", "value": 1 }"#).is_err());
    // What a model wrote gets places in the editor, sources to the left.
    let mut g = Graph::from_json(r#"{ "nodes": { "a": { "type": "noise.fbm" }, "b": { "type": "adjust.levels", "inputs": { "in": "a" } }, "out": { "type": "output.map", "inputs": { "height": "b" } } } }"#).unwrap();
    format::lay_out(&mut g);
    assert!(g.ui["a"].pos[0] < g.ui["b"].pos[0] && g.ui["b"].pos[0] < g.ui["out"].pos[0]);
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
