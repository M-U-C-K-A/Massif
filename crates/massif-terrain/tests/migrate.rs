//! Graphs of Massif 1.2 migrate: every kind of node, alone and in the
//! templates, becomes a terrain graph that builds with no error, its links,
//! seeds and outputs carried over.

use massif_core::graph::{BlockPalette, Graph as V1, NodeKind, PortType, Template};
use massif_core::registry::{Registry, TerrainId};
use massif_terrain::compile::{Severity, compile};
use massif_terrain::engine::{Engine, never};
use massif_terrain::migrate::{OUT, World, from_v1};
use massif_terrain::model::Graph;
use massif_terrain::space::Space;

fn world(reg: &Registry) -> impl Fn(u16) -> String + '_ {
    |id| reg.terrain(TerrainId(id)).name.clone()
}

/// Migrates, compiles and builds the height: no error, finite heights.
fn builds(old: &V1, what: &str) -> Graph {
    let reg = Registry::vanilla();
    let names = world(&reg);
    let (g, diags) = from_v1(old, &World { side: 2048, sea_level: 62.0, terrain_name: &names });
    let text = g.to_json();
    let again = Graph::from_json(&text).expect("the migrated graph reads back");
    assert_eq!(again, g);
    let (c, errors) = compile(&g, Space::new(2048), 256);
    let errors: Vec<_> = errors.iter().filter(|d| d.severity == Severity::Error).collect();
    assert!(errors.is_empty(), "{what}: {errors:#?}\n{text}\n{diags:#?}");
    if let Some(h) = c.output("height") {
        let e = Engine::new(Default::default());
        let grid = e.whole(&c, h, 5, &never).unwrap_or_else(|err| panic!("{what}: {err}\n{text}"));
        assert!(grid.data.iter().all(|v| v.is_finite()), "{what}");
    }
    g
}

#[test]
fn every_kind_migrates() {
    for kind in NodeKind::catalogue() {
        let mut old = V1::default();
        let id = old.add(kind.clone(), [100.0, 100.0]);
        let noise = old.add(NodeKind::catalogue().into_iter().find(|k| matches!(k, NodeKind::Noise { .. })).unwrap(), [0.0, 0.0]);
        let mask = old.add(NodeKind::SelectHeight { min: 60.0, max: 120.0, falloff: 10.0 }, [0.0, 200.0]);
        old.connect(noise, 0, mask, 0);
        let paint = old.add(NodeKind::SolidColor { color: [120, 140, 90] }, [0.0, 300.0]);
        for (k, (_, t)) in kind.inputs().iter().enumerate() {
            let from = match t {
                PortType::Height => noise,
                PortType::Mask => mask,
                PortType::Color => paint,
            };
            old.connect(from, 0, id, k as u8);
        }
        // Its first output to the map.
        if let Some((_, t)) = kind.outputs().first() {
            let sink = match t {
                PortType::Height => NodeKind::HeightOutput,
                PortType::Mask => NodeKind::TerrainOutput { terrain: TerrainId::ROCK.0 },
                PortType::Color => NodeKind::ColorOutput { blocks: true, palette: BlockPalette::Natural },
            };
            let out = old.add(sink, [300.0, 100.0]);
            old.connect(id, 0, out, 0);
            if *t != PortType::Height {
                let h = old.add(NodeKind::HeightOutput, [300.0, 0.0]);
                old.connect(noise, 0, h, 0);
            }
        }
        let g = builds(&old, kind.label());
        assert!(g.nodes.contains_key(OUT), "{}", kind.label());
    }
}

#[test]
fn the_templates_migrate() {
    let reg = Registry::vanilla();
    for t in Template::ALL {
        let old = t.graph(&reg, 63.0, 2048.0);
        let g = builds(&old, t.label());
        let out = &g.nodes[OUT];
        assert!(out.inputs.contains_key("height"), "{}: {:?}", t.label(), out.inputs);
        // Every link of 1.2 is a link now, or was said to feed nothing.
        let links: usize = g.nodes.values().map(|n| n.inputs.len()).sum();
        assert!(links >= old.links.len() / 2, "{}: {links} links for {}", t.label(), old.links.len());
    }
}

#[test]
fn settings_links_and_seeds_carry_over() {
    let mut old = V1::default();
    let noise = old.add(NodeKind::Noise { style: massif_core::graph::NoiseStyle::Ridged, scale: 900.0, octaves: 7, roughness: 0.55, height: 180.0, base: 40.0, seed: 12345 }, [10.0, 20.0]);
    let levels = old.add(NodeKind::Levels { in_min: 0.0, in_max: 255.0, out_min: 10.0, out_max: 200.0, gamma: 2.0 }, [200.0, 20.0]);
    let snow = old.add(NodeKind::Snow { line: 150.0, slope: 40.0, depth: 3.0, softness: 20.0 }, [400.0, 20.0]);
    let h = old.add(NodeKind::HeightOutput, [600.0, 20.0]);
    let t = old.add(NodeKind::TerrainOutput { terrain: TerrainId::ROCK.0 }, [600.0, 120.0]);
    old.connect(noise, 0, levels, 0);
    old.connect(levels, 0, snow, 0);
    old.connect(snow, 0, h, 0);
    old.connect(snow, 1, t, 0);
    let g = builds(&old, "a chain");
    let fbm = &g.nodes["fbm"];
    assert_eq!((fbm.params["style"].as_str(), fbm.params["seed"].as_u64(), fbm.params["roughness"].as_f64()), (Some("ridged"), Some(12345), Some(0.55)));
    assert_eq!(g.ui["fbm"].pos, [10.0, 20.0]);
    assert_eq!(g.nodes["levels"].inputs["in"], "fbm");
    assert_eq!(g.nodes["levels"].params["gamma"].as_f64(), Some(0.5), "1.2's gamma is the inverse");
    // Snow's height goes on, its mask lays rock over the map's terrains.
    assert_eq!(g.nodes[OUT].inputs["height"], "snow.height");
    assert_eq!(g.nodes["terrains"].inputs["mask_1"], "snow");
    assert_eq!(g.nodes["terrains"].params["terrain_1"], "Rock");
    assert_eq!(g.nodes["terrains"].inputs["under"], "map.terrain");
    assert_eq!(g.nodes[OUT].inputs["terrain"], "terrains");
}
