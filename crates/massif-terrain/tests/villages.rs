//! The Village Generator: a pack's village on a terrain, stage by stage as
//! nodes inside `massif:village-generator`, its geometry made whole, its
//! levelled ground read by tiles, and the map's structures and roads fed.

use massif_terrain::compile::{Severity, compile};
use massif_terrain::engine::{Engine, never};
use massif_terrain::kernel::Geo;
use massif_terrain::model::Graph;
use massif_terrain::space::Space;

fn pack() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-terrain-villages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    massif_structures::village::testpack::write_pack(&dir).unwrap()
}

fn graph(pack: &std::path::Path, extra: &str) -> Graph {
    let text = format!(
        r#"{{ "nodes": {{
        "land": {{ "type": "noise.fbm", "params": {{ "scale": 600, "height": 10, "base": 70 }} }},
        "village": {{ "type": "massif:village-generator", "inputs": {{ "height": "land" }}, "params": {{ "pack": {pack}, "houses": 24 {extra} }} }},
        "out": {{ "type": "output.map", "inputs": {{ "height": "village.height", "structures": "village.structures", "roads": "village.roads" }} }}
    }} }}"#,
        pack = serde_json::to_string(&pack.to_string_lossy()).unwrap()
    );
    Graph::from_json(&text).unwrap()
}

#[test]
fn the_village_generator_builds_a_village_of_the_pack() {
    let pack = pack();
    let g = graph(&pack, r#", "walls": true, "road": "stone""#);
    let (c, d) = compile(&g, Space::new(512), 512);
    assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
    let e = Engine::new(Default::default());
    let all = e.geometry(&c, c.output("structures").unwrap(), &never).unwrap();
    let Geo::Instances(all) = &*all else { panic!("instances") };
    let houses = all.items.iter().filter(|i| i.kind == "house" || i.kind == "manor").count();
    assert!((12..=24).contains(&houses), "{houses} houses for 24");
    assert!(all.items.iter().any(|i| i.kind == "market"), "stalls round the square");
    assert!(all.items.iter().all(|i| i.asset.starts_with("pack:Plains (test boxes)/Plains/") || i.asset.starts_with("massif:")));
    // None over another (Massif's own props are points).
    let boxes: Vec<_> = all.items.iter().filter(|i| !i.asset.starts_with("massif:")).map(|i| i.bbox).collect();
    for (k, a) in boxes.iter().enumerate() {
        for b in &boxes[k + 1..] {
            let apart = a.1[0] < b.0[0] || b.1[0] < a.0[0] || a.1[2] < b.0[2] || b.1[2] < a.0[2];
            assert!(apart, "{a:?} over {b:?}");
        }
    }
    // Roads of stone, cut at the walls, the walls with gates.
    let roads = e.geometry(&c, c.output("roads").unwrap(), &never).unwrap();
    let Geo::Splines(roads) = &*roads else { panic!("splines") };
    assert!(roads.lines.iter().any(|l| l.kind == "main") && roads.lines.iter().all(|l| l.attrs.get("road") == Some(&3.0)));
    let village = c.node("village__walls").unwrap();
    let towers = e.geometry(&c, (village, 1), &never).unwrap();
    assert!(towers.points().unwrap().items.iter().any(|p| p.kind == "gate"));
    // The ground levelled under the buildings, read by tiles as whole.
    let out = c.output("height").unwrap();
    let whole = e.region(&c, out, 0, c.space.whole(0), &never).unwrap();
    let church = all.items.iter().find(|i| i.kind != "market" && !i.asset.starts_with("massif:")).unwrap();
    let floor = church.attrs["floor"] as f32;
    let (cx, cz) = ((church.bbox.0[0] + church.bbox.1[0]) / 2, (church.bbox.0[2] + church.bbox.1[2]) / 2);
    assert_eq!(whole.at(cx, cz), floor - 1.0, "levelled to the floor under {church:?}");
    // The same village twice; another seed, another.
    let again = Engine::new(Default::default()).geometry(&c, c.output("structures").unwrap(), &never).unwrap();
    assert_eq!(*again, Geo::Instances(all.clone()));
    let other = graph(&pack, r#", "walls": true, "seed": 9"#);
    let (c2, _) = compile(&other, Space::new(512), 512);
    let o = Engine::new(Default::default()).geometry(&c2, c2.output("structures").unwrap(), &never).unwrap();
    assert_ne!(*o, Geo::Instances(all.clone()));
    let _ = std::fs::remove_dir_all(pack.parent().unwrap());
}

#[test]
fn without_a_pack_the_village_is_empty() {
    let g = Graph::from_json(r#"{ "nodes": {
        "village": { "type": "massif:village-generator" },
        "out": { "type": "output.map", "inputs": { "height": "village.height", "structures": "village.structures" } }
    } }"#)
    .unwrap();
    let (c, _) = compile(&g, Space::new(256), 256);
    let e = Engine::new(Default::default());
    let built = e.geometry(&c, c.output("structures").unwrap(), &never).unwrap();
    assert_eq!(built.instances().unwrap().items.len(), 0);
    assert!(e.whole(&c, c.output("height").unwrap(), 0, &never).is_ok(), "the map still builds");
}
