//! Massif's own scripting API, in Lua: maps made, opened, read and written
//! by regions, changed over the whole map, saved; errors with their line;
//! the sandbox.

use massif_script::{Ending, Line, Options, Script};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-lua-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, name: &str, source: &str, args: &[&str], world: Option<massif_core::World>, cancel: Arc<AtomicBool>) -> (Ending, Vec<Line>, massif_script::Outcome) {
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    let script = Script::load(&path).unwrap();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let options = Options {
        args: args.iter().map(|a| a.to_string()).collect(),
        params: script.descriptor().defaults(),
        base: dir.to_path_buf(),
        allow: Vec::new(),
        world: world.map(|w| (w, 0)),
        world_path: None,
        out: Arc::new(move |line| sink.lock().unwrap().push(line)),
        progress: Default::default(),
        cancel,
    };
    let outcome = massif_script::run(&script, options);
    let printed = lines.lock().unwrap().clone();
    (outcome.ending.clone(), printed, outcome)
}

fn out(lines: &[Line]) -> Vec<String> {
    lines.iter().filter_map(|l| if let Line::Out(t) = l { Some(t.clone()) } else { None }).collect()
}

fn failed(e: Ending) -> massif_script::ScriptError {
    match e {
        Ending::Failed(e) => e,
        other => panic!("the script should have failed: {other:?}"),
    }
}

/// A map made, its heights and terrains read and written by regions, a
/// sum over a grid, a whole-map pass, the map saved.
#[test]
fn a_map_made_read_written_and_saved() {
    let dir = temp("map");
    let src = r#"
local map = massif.create{ width = 256, depth = 256, landform = "hills", seed = 3 }
local x0, z0, x1, z1 = map:bounds()
print(x1 - x0, z1 - z0)
local g = map:get("height", x0, z0, x0 + 64, z0 + 32)
print(#g, g.width, g.depth)
g:fill(100)
map:set("height", x0, z0, x0 + 64, z0 + 32, g)
print(map:height(x0 + 3, z0 + 5))
local t = map:get("terrain", x0, z0, x0 + 64, z0 + 32)
local rock = map:terrain_id("Rock")
for i = 1, #t do t[i] = rock end
map:set("terrain", x0, z0, x0 + 64, z0 + 32, t)
print(map:get("terrain", x0, z0, x0 + 64, z0 + 32):count(rock))
local tiles = 0
for tx0, tz0, tx1, tz1 in map:tiles() do tiles = tiles + 1 end
print(tiles)
map:whole_map("smooth", { radius = 2 })
map.name = "Made in Lua"
print(map.name)
map:save("made.massif")
"#;
    let (ending, lines, _) = run(&dir, "map.lua", src, &[], None, Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    assert_eq!(out(&lines), ["256\t256", "2048\t64\t32", "100", "2048", "4", "Made in Lua"]);
    let world = massif_format::load(&dir.join("made.massif")).unwrap();
    assert_eq!(world.meta.name, "Made in Lua");
    std::fs::remove_dir_all(&dir).ok();
}

/// The open map is `massif.map`, and what the script changes comes back.
#[test]
fn the_open_map_comes_back_changed() {
    let dir = temp("open");
    let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 2, tiles_z: 2, ..Default::default() });
    let src = "local m = massif.map\nlocal x0, z0, x1, z1 = m:bounds()\nm:set('height', x0, z0, x1, z1, 70)\nprint(m:get('height', x0, z0, x1, z1):max())\n";
    let (ending, lines, outcome) = run(&dir, "open.lua", src, &[], Some(world), Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    assert_eq!(out(&lines), ["70"]);
    assert!(outcome.changed);
    let world = outcome.world.unwrap();
    let t = world.dims[0].ground().view(massif_core::TilePos::new(-1, -1)).unwrap();
    assert_eq!(t.top.get(0), 71.0, "the height is the top block's y, the surface one over it");
    std::fs::remove_dir_all(&dir).ok();
}

/// Errors say their line; what Massif refuses says why.
#[test]
fn errors_say_their_line() {
    let dir = temp("errors");
    let (ending, _, _) = run(&dir, "e.lua", "local a = 1\nlocal b = nil\nprint(b.c)\n", &[], None, Default::default());
    let e = failed(ending);
    assert_eq!(e.line, Some(3), "{e:?}");
    let (ending, _, _) = run(&dir, "s.lua", "local a = 1\nlocal = 2\n", &[], None, Default::default());
    assert_eq!(failed(ending).line, Some(2));
    let (ending, _, _) = run(&dir, "k.lua", "local m = massif.create{ width = 128, depth = 128 }\n\nm:get('heights', 0, 0, 1, 1)\n", &[], None, Default::default());
    let e = failed(ending);
    assert!(e.message.contains("Unknown kind of column data \"heights\""), "{}", e.message);
    assert_eq!(e.line, Some(3));
    std::fs::remove_dir_all(&dir).ok();
}

/// No files but through Massif, no network, no programs, and the API
/// cannot be changed.
#[test]
fn the_sandbox_holds() {
    let dir = temp("sandbox");
    let src = "print(io == nil, os.execute == nil, require == nil, debug == nil or debug.sethook == nil)\nprint(pcall(function() massif.open = nil end))\nprint(pcall(function() massif.open('/etc/hosts') end))\n";
    let (ending, lines, _) = run(&dir, "sb.lua", src, &[], None, Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    let o = out(&lines);
    assert_eq!(o[0], "true\ttrue\ttrue\ttrue");
    assert!(o[1].starts_with("false"), "the API is read-only: {}", o[1]);
    assert!(o[2].starts_with("false") && o[2].contains("may only use files"), "{}", o[2]);
    std::fs::remove_dir_all(&dir).ok();
}

/// Parameters from the descriptor, arguments; a script stopped when asked.
#[test]
fn parameters_arguments_and_stop() {
    let dir = temp("params");
    let src = "-- script.name=Test\n-- script.param.count.type=integer\n-- script.param.count.default=7\nprint(massif.params.count + 1, massif.args[1], #massif.args)\n";
    let (ending, lines, _) = run(&dir, "p.lua", src, &["one", "two"], None, Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    assert_eq!(out(&lines), ["8\tone\t2"]);
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        c.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let (ending, _, _) = run(&dir, "loop.lua", "local n = 0\nwhile true do n = n + 1 end\n", &[], None, cancel);
    stopper.join().unwrap();
    assert_eq!(ending, Ending::Cancelled);
    std::fs::remove_dir_all(&dir).ok();
}

/// Masks painted with Massif's own terms: a terrain and an annotation where
/// an image is on, heights from another.
#[test]
fn masks_paint_terrains_annotations_and_heights() {
    let dir = temp("masks");
    // 256 × 256: white on the left half, a ramp of grey on the right.
    let mut img = vec![0u8; 256 * 256];
    for z in 0..256 {
        for x in 0..256 {
            img[z * 256 + x] = if x < 128 { 255 } else { 0 };
        }
    }
    massif_script::testing::write_grey(&dir.join("left.png"), 256, &img).unwrap();
    let ramp: Vec<u8> = (0..256 * 256).map(|k| (k % 256) as u8).collect();
    massif_script::testing::write_grey(&dir.join("ramp.png"), 256, &ramp).unwrap();
    let src = r#"
local map = massif.create{ width = 256, depth = 256, landform = "flat" }
local x0, z0, x1, z1 = map:bounds()
local rock = map:terrain_id("Rock")
map:import_mask("left.png", { paint = "terrain", value = rock, x = x0, z = z0 })
map:import_mask("left.png", { paint = "annotations", value = 4, invert = true, x = x0, z = z0 })
map:import_mask("ramp.png", { paint = "height", black = 40, white = 295, x = x0, z = z0 })
local t = map:get("terrain", x0, z0, x1, z1)
print(t:count(rock))
print(map:get("annotations", x0, z0, x1, z1):count(4))
print(map:height(x0, z0), map:height(x0 + 255, z0), map:height(x0 + 100, z0 + 7))
print(pcall(function() map:import_mask("left.png", { paint = "terrain" }) end))
"#;
    let (ending, lines, _) = run(&dir, "masks.lua", src, &[], None, Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    let o = out(&lines);
    assert_eq!(o[0], "32768", "the left half is rock");
    assert_eq!(o[1], "32768", "the right half annotated (the dark side)");
    assert_eq!(o[2], "40\t295\t140");
    assert!(o[3].starts_with("false") && o[3].contains("needs a value"), "{}", o[3]);
    std::fs::remove_dir_all(&dir).ok();
}

/// The first example of docs/scripting.md runs, and does what it says.
#[test]
fn the_guide_example_runs() {
    let dir = temp("guide");
    let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 4, tiles_z: 4, landform: massif_core::generate::Landform::Mountains, seed: 5, ..Default::default() });
    let src = r#"-- Frost over the high land of the open map.
local map = massif.map
local frost = map:layer("Frost")
for x0, z0, x1, z1 in map:tiles() do
    local h = map:get("height", x0, z0, x1, z1)
    local cover = map:get("cover", x0, z0, x1, z1)
    for i = 1, #h do
        if h[i] and h[i] > 150 then
            cover[i] = frost.id
        end
    end
    map:set("cover", x0, z0, x1, z1, cover)
    if massif.cancelled() then return end
end
local x0, z0, x1, z1 = map:bounds()
local h = map:get("height", x0, z0, x1, z1)
local c = map:get("cover", x0, z0, x1, z1)
local high, frosted = 0, 0
for i = 1, #h do
    if h[i] and h[i] > 150 then
        high = high + 1
        if c[i] == frost.id then frosted = frosted + 1 end
    end
end
print(high > 0, high == frosted)
"#;
    let (ending, lines, outcome) = run(&dir, "guide.lua", src, &[], Some(world), Default::default());
    assert_eq!(ending, Ending::Done, "{lines:?}");
    assert_eq!(out(&lines), ["true\ttrue"]);
    assert!(outcome.changed);
    std::fs::remove_dir_all(&dir).ok();
}
