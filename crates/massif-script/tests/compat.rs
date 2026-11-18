//! WorldPainter scripts run in Massif and give WorldPainter's map.
//!
//! The scripts of `tests/wp` (WorldPainter's own scripting tests, adapted to
//! a map made from its test height map, and scripts as map makers write
//! them) were run once in the real WorldPainter; what its maps hold, as
//! Massif imports them, is kept in `tests/wp/expected` (see
//! `massif_script::testing::fingerprint`). Each test runs the script here
//! and compares. Left out: the terrains WorldPainter's `createWorld` draws
//! with noise it seeds at random (grass, permadirt, stone mix), which two
//! runs of WorldPainter do not agree on either.

use massif_script::{Ending, Line, Options, Script};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn wp_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wp")
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-script-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `source` (a file `name` in `dir`), with `args`, allowed to use
/// `allow` too: how it ended and what it printed.
fn run_in(dir: &Path, name: &str, source: Option<&str>, args: &[&str], allow: &[&Path], cancel: Arc<AtomicBool>) -> (Ending, Vec<Line>, massif_script::Outcome) {
    let path = dir.join(name);
    if let Some(src) = source {
        std::fs::write(&path, src).unwrap();
    }
    let script = Script::load(&path).unwrap();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let options = Options {
        args: args.iter().map(|a| a.to_string()).collect(),
        params: script.descriptor().defaults(),
        base: dir.to_path_buf(),
        allow: allow.iter().map(|p| p.to_path_buf()).collect(),
        world: None,
        world_path: None,
        out: Arc::new(move |line| sink.lock().unwrap().push(line)),
        progress: Default::default(),
        cancel,
    };
    let outcome = massif_script::run(&script, options);
    let printed = lines.lock().unwrap().clone();
    (outcome.ending.clone(), printed, outcome)
}

fn run(name: &str, args: &[&str], allow: &[&Path]) -> (Ending, Vec<Line>) {
    let (e, l, _) = run_in(&wp_dir(), name, None, args, allow, Default::default());
    (e, l)
}

fn out_lines(lines: &[Line]) -> Vec<String> {
    lines.iter().filter_map(|l| if let Line::Out(t) = l { Some(t.clone()) } else { None }).collect()
}

/// Compare what `map` holds with what WorldPainter's map held: every
/// count the same, but the forests' within `slack` (their strengths are
/// kept on columns of Massif's own choosing).
fn same_as_worldpainter(map: &Path, expected: &str, slack: f64) {
    let world = massif_format::load(map).expect("the script saved its map");
    let got = massif_script::testing::fingerprint(&world);
    let want: BTreeMap<String, i64> = serde_json::from_str(&std::fs::read_to_string(wp_dir().join("expected").join(expected)).unwrap()).unwrap();
    for (k, v) in &want {
        let g = got.get(k).copied().unwrap_or(0);
        if k.starts_with("trees:") || k.starts_with("understory:") {
            assert!(((g - v).abs() as f64) <= *v as f64 * slack, "{expected}: {k} is {g}, WorldPainter's {v}");
        } else {
            assert_eq!(g, *v, "{expected}: {k}");
        }
    }
    for (k, g) in &got {
        let noisy = ["terrain:Grass", "terrain:Coarse Dirt", "terrain:Stone Mix"].contains(&k.as_str());
        assert!(noisy || want.contains_key(k), "{expected}: {k} ({g} columns) is not in WorldPainter's map");
    }
}

/// Rivers dug with height maths, then the River layer and gravel: the same
/// numbers printed and the same map as WorldPainter.
#[test]
fn rivers_and_height_maths_as_worldpainter() {
    let out = temp("rivers");
    let map = out.join("rivers.world");
    let (ending, lines) = run("rivers.js", &[map.to_str().unwrap()], &[&out]);
    assert_eq!(ending, Ending::Done, "{lines:?}");
    // What WorldPainter printed.
    assert_eq!(
        out_lines(&lines),
        [
            "Map: 640 x 640 blocks (5 x 5 tiles), heights 57 to 192",
            "River: 22821 columns, beds -3134 to 81, mean -972.293, 22821 under water",
            "Saved",
        ]
    );
    same_as_worldpainter(&out.join("rivers.massif"), "rivers.json", 0.0);
    std::fs::remove_dir_all(&out).ok();
}

/// WorldPainter's colourmappingtest.js: biomes from a colour map.
#[test]
fn biomes_from_a_colour_map_as_worldpainter() {
    let out = temp("colours");
    let (ending, lines) = run("colours.js", &[out.join("c.massif").to_str().unwrap()], &[&out]);
    assert_eq!(ending, Ending::Done, "{lines:?}");
    same_as_worldpainter(&out.join("c.massif"), "colours.json", 0.0);
    std::fs::remove_dir_all(&out).ok();
}

/// WorldPainter's slopetest.js, and forests through a feathered filter of
/// slope, heights and land.
#[test]
fn filters_of_slope_height_and_land_as_worldpainter() {
    let out = temp("slopes");
    let (ending, lines) = run("slopes.js", &[out.join("s.massif").to_str().unwrap()], &[&out]);
    assert_eq!(ending, Ending::Done, "{lines:?}");
    same_as_worldpainter(&out.join("s.massif"), "slopes.json", 0.0);
    std::fs::remove_dir_all(&out).ok();
}

/// Sixty masks onto terrains, forests, frost and annotations, with filters.
#[test]
fn masks_onto_terrains_and_layers_as_worldpainter() {
    let out = temp("masks");
    for i in 0..60 {
        massif_script::testing::write_grey(&out.join(format!("mask{i}.png")), 640, &massif_script::testing::mask(i, 640)).unwrap();
    }
    let started = std::time::Instant::now();
    let (ending, lines) = run("masks.js", &[out.to_str().unwrap(), "60", out.join("m.massif").to_str().unwrap()], &[&out]);
    assert_eq!(ending, Ending::Done, "{lines:?}");
    eprintln!("60 masks of 640² in {:.2?}", started.elapsed());
    same_as_worldpainter(&out.join("m.massif"), "masks.json", 0.01);
    std::fs::remove_dir_all(&out).ok();
}

fn failed(ending: Ending) -> massif_script::ScriptError {
    match ending {
        Ending::Failed(e) => e,
        other => panic!("the script should have failed, it ended {other:?}"),
    }
}

/// What WorldPainter's scripting context says when `go()` is forgotten,
/// and the line it says it on.
#[test]
fn a_forgotten_go_is_said_with_its_line() {
    let dir = temp("go");
    let (ending, _, _) = run_in(&dir, "go.js", Some("var frost = wp.getLayer().withName('Frost');\n\nvar again = wp.getLayer().withName('Frost').go();\n"), &[], &[], Default::default());
    let e = failed(ending);
    assert_eq!(e.message, "You forgot to invoke go() on the getLayer() operation");
    assert_eq!(e.line, Some(3));
    let (ending, _, _) = run_in(&dir, "end.js", Some("wp.getLayer().withName('Frost');\n"), &[], &[], Default::default());
    assert_eq!(failed(ending).message, "You forgot to invoke go() on the getLayer() operation");
    std::fs::remove_dir_all(&dir).ok();
}

/// What Massif does not do says so, never nothing.
#[test]
fn what_is_not_supported_says_so() {
    let dir = temp("unsupported");
    for (src, said) in [
        ("wp.mergeWorld().go();", "wp.mergeWorld() is not supported yet"),
        ("var f = new java.io.File('x');", "The Java class java.io.File is not supported yet"),
        ("var r = wp.getLayer().withName('Resources').go();\nvar h = wp.getHeightMap().fromFile('m.png').go();", "File m.png not found"),
        ("wp.getLayer().withName('Resources').go().getPriority();", "layer.getPriority() is not supported yet"),
        ("Java.type('org.pepsoft.minecraft.Material');", "The Java class org.pepsoft.minecraft.Material is not supported yet"),
    ] {
        let (ending, _, _) = run_in(&dir, "u.js", Some(src), &[], &[], Default::default());
        let e = failed(ending);
        assert!(e.message.contains(said), "{src}: {}", e.message);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A layer Massif has no such thing as fails where it is applied, by name.
#[test]
fn an_unsupported_layer_fails_where_it_is_applied() {
    let dir = temp("resources");
    std::fs::copy(wp_dir().join("heightmap.png"), dir.join("heightmap.png")).unwrap();
    let src = "var hm = wp.getHeightMap().fromFile('heightmap.png').go();\nvar world = wp.createWorld().fromHeightMap(hm).go();\nvar r = wp.getLayer().withName('Resources').go();\nwp.applyLayer(r).toWorld(world).go();\n";
    let (ending, _, _) = run_in(&dir, "r.js", Some(src), &[], &[], Default::default());
    let e = failed(ending);
    assert!(e.message.contains("The Resources layer") && e.message.contains("is not supported yet"), "{}", e.message);
    assert_eq!(e.line, Some(4));
    std::fs::remove_dir_all(&dir).ok();
}

/// A syntax error's line and column; an error thrown in a function, the
/// line it was thrown on.
#[test]
fn errors_name_their_line() {
    let dir = temp("syntax");
    let (ending, _, _) = run_in(&dir, "s.js", Some("var a = 1;\nvar b = 2;\nvar c = ;\n"), &[], &[], Default::default());
    assert_eq!(failed(ending).line, Some(3));
    let (ending, _, _) = run_in(&dir, "t.js", Some("function f() {\n    throw new Error('no');\n}\nf();\n"), &[], &[], Default::default());
    let e = failed(ending);
    assert_eq!((e.message.as_str(), e.line), ("no", Some(2)));
    std::fs::remove_dir_all(&dir).ok();
}

/// Files outside the script's folder are refused unless picked.
#[test]
fn files_outside_the_script_folder_are_refused() {
    let dir = temp("outside");
    let other = temp("outside-other");
    std::fs::copy(wp_dir().join("heightmap.png"), other.join("hm.png")).unwrap();
    let src = format!("var h = wp.getHeightMap().fromFile('{}').go();\nprint(h.getWidth());\n", other.join("hm.png").display());
    let (ending, _, _) = run_in(&dir, "o.js", Some(&src), &[], &[], Default::default());
    assert!(failed(ending).message.contains("may only use files in the script's folder"));
    let (ending, lines, _) = run_in(&dir, "o.js", Some(&src), &[], &[&other.join("hm.png")], Default::default());
    assert_eq!((ending, out_lines(&lines)), (Ending::Done, vec!["640".to_owned()]));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&other).ok();
}

/// `arguments`, `argv`, `argc`, `params` and `exit()`, as WorldPainter
/// gives them.
#[test]
fn arguments_params_and_exit() {
    let dir = temp("args");
    let src = "// script.param.count.type=integer\n// script.param.count.default=7\n// script.param.name.type=string\n// script.param.name.default=Zhyrr\nprint(arguments.length + ' ' + arguments[0] + ' ' + argv[0] + ' ' + argc);\nprint(params['count'] + 1);\nprint(params['name']);\nexit(3);\nprint('not here');\n";
    let (ending, lines, _) = run_in(&dir, "a.js", Some(src), &["one", "two"], &[], Default::default());
    assert_eq!(ending, Ending::Exit(3));
    assert_eq!(out_lines(&lines), ["2 one a.js 3", "8", "Zhyrr"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A script that runs on is stopped when asked.
#[test]
fn a_script_is_stopped_when_asked() {
    let dir = temp("cancel");
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let (ending, _, _) = run_in(&dir, "loop.js", Some("var n = 0;\nwhile (true) { n++; }\n"), &[], &[], cancel);
    stopper.join().unwrap();
    assert_eq!(ending, Ending::Cancelled);
    std::fs::remove_dir_all(&dir).ok();
}

/// Terrains are numbered as WorldPainter's Terrain.VALUES: the beaches are
/// 36, grass 0, and `ordinal()` is the enum's.
#[test]
fn terrains_are_numbered_as_worldpainter_numbers_them() {
    let dir = temp("terrains");
    let src = "var T = org.pepsoft.worldpainter.Terrain;\nprint(T.VALUES.length + ' ' + T.VALUES[36].name() + ' ' + T.VALUES[0].getName() + ' ' + T.GRASS.ordinal() + ' ' + T.STONE_MIX.ordinal() + ' ' + (T.VALUES[3] === T.PERMADIRT));\n";
    let (ending, lines, _) = run_in(&dir, "t.js", Some(src), &[], &[], Default::default());
    assert_eq!(ending, Ending::Done);
    assert_eq!(out_lines(&lines), ["161 BEACHES Grass 0 75 true"]);
    std::fs::remove_dir_all(&dir).ok();
}
