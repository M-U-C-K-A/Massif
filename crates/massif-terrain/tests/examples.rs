//! The example graphs (also the editor's starting points) build with no
//! error and give heights within the game's.

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
