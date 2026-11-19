//! The example graphs (also the editor's starting points) build with no
//! error and give heights within the game's; with `MASSIF_SNAPSHOT_DIR`,
//! the golden scene — the look the generator aims at — is drawn there.

use massif_terrain::compile::{Severity, compile};
use massif_terrain::engine::{Engine, never};
use massif_terrain::format;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;

#[test]
fn the_examples_build() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/graphs");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with(".terrain.json") {
            continue;
        }
        count += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let errors: Vec<_> = format::validate(&text).into_iter().filter(|d| d.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
        let g = Graph::from_json(&text).unwrap();
        let (c, _) = compile(&g, Space::new(4096), 256);
        let h = Engine::new(Default::default()).whole(&c, c.output("height").expect("a height"), 6, &never).unwrap();
        let (lo, hi) = h.range();
        assert!(lo >= -64.0 && hi <= 320.0 && hi > lo + 20.0, "{}: {lo}..{hi}", path.display());
    }
    assert!(count >= 5, "{count} examples");
}

#[test]
fn the_golden_scene_is_drawn() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/graphs/golden-scene.terrain.json");
    let g = Graph::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let (c, diags) = compile(&g, Space::new(4096), 1024);
    assert!(diags.iter().all(|d| d.severity != Severity::Error), "{diags:#?}");
    let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") else { return };
    let e = Engine::new(Default::default());
    let lod = c.space.lod_for(1024);
    let whole = c.space.whole(lod);
    let height = e.region(&c, c.output("height").unwrap(), lod, whole, &never).unwrap();
    let albedo = e.region(&c, c.output("albedo").unwrap(), lod, whole, &never).unwrap();
    let rgb: Vec<u8> = massif_terrain::preview::shade_with(&height, c.space.sea_level, Some(&albedo)).into_iter().flatten().collect();
    let out = std::path::Path::new(&dir).join("golden-scene.png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&out).unwrap()), whole.w, whole.h);
    enc.set_color(png::ColorType::Rgb);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
}
