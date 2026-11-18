//! Inputs: heightmaps and images from disk, the map through the host.

use massif_terrain::compile::{Compiled, compile_in};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::files::DiskEnv;
use massif_terrain::kernel::{Env, Grid, MapLayer};
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space};
use std::path::PathBuf;
use std::sync::Arc;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-terrain-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &std::path::Path, w: u32, h: u32, color: png::ColorType, depth: png::BitDepth, data: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    enc.write_header().unwrap().write_image_data(data).unwrap();
}

fn compiled(json: &str, env: Arc<dyn Env>) -> Compiled {
    let (c, diags) = compile_in(&Graph::from_json(json).unwrap(), Space::new(1024), 256, env);
    assert!(diags.is_empty(), "{diags:?}");
    c
}

fn grid(c: &Compiled, id: &str, port: Option<&str>, lod: u8) -> Grid {
    Engine::new(Settings::default()).whole(c, c.source(id, port).unwrap(), lod, &never).unwrap()
}

#[test]
fn a_16_bit_heightmap_reads_back_to_within_half_a_block() {
    let dir = folder("ramp");
    // A ramp west to east over 256 pixels, 16-bit greys, big-endian.
    let mut data = Vec::new();
    for _ in 0..256 {
        for x in 0..256u32 {
            let v = (x as f64 / 255.0 * 65535.0).round() as u16;
            data.extend_from_slice(&v.to_be_bytes());
        }
    }
    write_png(&dir.join("ramp.png"), 256, 256, png::ColorType::Grayscale, png::BitDepth::Sixteen, &data);
    let env = Arc::new(DiskEnv::new(&dir));
    let c = compiled(r#"{ "nodes": { "hm": { "type": "input.heightmap", "params": { "file": "ramp.png", "low": 40, "high": 200 } } } }"#, env.clone());
    let g = grid(&c, "hm", None, 2);
    // Sample i lies at block 4i, at pixel i − 0.5.
    for i in [1, 40, 128, 200, 254] {
        let expected = 40.0 + 160.0 * ((i as f32 - 0.5) / 255.0);
        assert!((g.at(i, 100) - expected).abs() < 0.5, "{i}: {} vs {expected}", g.at(i, 100));
    }
    let m = grid(&c, "hm", Some("mask"), 2);
    assert!(m.data.iter().all(|v| *v == 1.0));
    // Laid over the terrain's east half only: Low in the west.
    let half = compiled(r#"{ "nodes": { "hm": { "type": "input.heightmap", "params": { "file": "ramp.png", "low": 40, "high": 200, "x": 0.5, "width": 0.5 } } } }"#, env.clone());
    let (g, m) = (grid(&half, "hm", None, 2), grid(&half, "hm", Some("mask"), 2));
    assert_eq!((g.at(50, 50), m.at(50, 50)), (40.0, 0.0));
    assert!(m.at(200, 50) == 1.0 && g.at(255, 50) > 195.0);
    // A changed file changes the node's hash: nothing stale is kept.
    let before = c.nodes[c.node("hm").unwrap()].hash;
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_png(&dir.join("ramp.png"), 256, 256, png::ColorType::Grayscale, png::BitDepth::Sixteen, &vec![0u8; 256 * 256 * 2]);
    let again = compiled(r#"{ "nodes": { "hm": { "type": "input.heightmap", "params": { "file": "ramp.png", "low": 40, "high": 200 } } } }"#, env);
    assert_ne!(again.nodes[again.node("hm").unwrap()].hash, before);
    assert_eq!(grid(&again, "hm", None, 2).at(200, 100), 40.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_image_gives_a_channel_as_a_mask_and_its_colours() {
    let dir = folder("image");
    // Red rising west to east, green constant.
    let mut data = Vec::new();
    for _ in 0..64 {
        for x in 0..64u32 {
            data.extend_from_slice(&[(x * 255 / 63) as u8, 128, 0]);
        }
    }
    write_png(&dir.join("map.png"), 64, 64, png::ColorType::Rgb, png::BitDepth::Eight, &data);
    let c = compiled(
        r#"{ "nodes": {
        "red": { "type": "input.image", "params": { "file": "map.png", "channel": "red" } },
        "warm": { "type": "input.image", "params": { "file": "map.png", "channel": "red", "low": 0.5, "high": 1, "invert": true } }
    } }"#,
        Arc::new(DiskEnv::new(&dir)),
    );
    let red = grid(&c, "red", None, 2);
    assert!(red.at(2, 10) < 0.05 && red.at(253, 10) > 0.95);
    let warm = grid(&c, "warm", None, 2);
    assert_eq!(warm.at(20, 10), 1.0, "under Low, inverted");
    let colour = grid(&c, "red", Some("color"), 2);
    assert!((colour.at_plane(1, 100, 10) - massif_terrain::params::linear([0.5; 3])[0]).abs() < 0.01);
    let _ = std::fs::remove_dir_all(dir);
}

/// A map 1024 blocks a side: the ground rises east, a lake in the west.
struct World;

impl Env for World {
    fn map_revision(&self) -> u64 {
        7
    }

    fn read_map(&self, layer: MapLayer, _: &Space, lod: u8, rect: Rect, out: &mut [f32]) -> bool {
        let s = (1u32 << lod) as f32;
        for (k, o) in out.iter_mut().enumerate() {
            let x = (rect.x + (k % rect.w as usize) as i32) as f32 * s;
            let ground = 60.0 + x * 0.1;
            *o = match layer {
                MapLayer::Height => ground,
                MapLayer::Water => if x < 100.0 { 75.0 } else { ground },
                MapLayer::Terrain => 3.0,
                MapLayer::Biome => 2.0,
            };
        }
        true
    }

    fn terrain_color(&self, id: u16) -> Option<[f32; 3]> {
        (id == 3).then_some([0.1, 0.2, 0.3])
    }
}

#[test]
fn the_map_is_read_through_the_host() {
    let c = compiled(r#"{ "nodes": { "m": { "type": "input.map" } } }"#, Arc::new(World));
    let h = grid(&c, "m", None, 2);
    assert!((h.at(100, 5) - 100.0).abs() < 1e-4);
    let water = grid(&c, "m", Some("water"), 2);
    assert_eq!((water.at(10, 5), water.at(100, 5)), (1.0, 0.0));
    assert_eq!(grid(&c, "m", Some("terrain"), 2).at(3, 3), 3.0);
    assert_eq!(grid(&c, "m", Some("color"), 2).at_plane(2, 3, 3), 0.3);
}

#[test]
fn curves_terraces_and_clamps() {
    let c = compiled(
        r#"{ "nodes": {
        "ramp": { "type": "pattern.gradient", "params": { "ends": [[0, 0.5], [1, 0.5]], "height": 100, "base": 100 } },
        "curve": { "type": "adjust.curve", "inputs": { "in": "ramp" }, "params": { "curve": [[0, 0], [1, 1]], "in_low": 100, "in_high": 200, "out_low": 0, "out_high": 10 } },
        "steps": { "type": "adjust.terrace", "inputs": { "in": "ramp" }, "params": { "steps": 4, "sharpness": 1, "low": 100, "high": 200 } },
        "cut": { "type": "adjust.clamp", "inputs": { "in": "ramp" }, "params": { "min": 120, "max": 180 } }
    } }"#,
        Arc::new(massif_terrain::kernel::NoEnv),
    );
    let ramp = grid(&c, "ramp", None, 2);
    let curve = grid(&c, "curve", None, 2);
    assert!((curve.at(128, 5) - (ramp.at(128, 5) - 100.0) / 10.0).abs() < 1e-3);
    // Four flat ledges: 100, 125, 150, 175.
    let steps = grid(&c, "steps", None, 2);
    for (x, ledge) in [(20, 100.0), (80, 125.0), (140, 150.0), (200, 175.0)] {
        assert!((steps.at(x, 5) - ledge).abs() < 1e-3, "{x}: {}", steps.at(x, 5));
    }
    let cut = grid(&c, "cut", None, 2);
    assert_eq!((cut.at(0, 5), cut.at(255, 5)), (120.0, 180.0));
}
