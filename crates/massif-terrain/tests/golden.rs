//! A released node version gives the same results forever: graphs leave
//! defaults out and exports are deterministic, so a change to a released
//! kernel would change maps already made. Each version's outputs with its
//! default settings, on the CPU, are summed up in `golden.txt` (mean,
//! spread and sixteen samples); a change fails here — make a new version of
//! the node instead. `MASSIF_WRITE_GOLDEN=1` writes the file again, for new
//! node versions only.

use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, never};
use massif_terrain::format;
use massif_terrain::model::Graph;
use massif_terrain::registry::PortKind;
use massif_terrain::space::Space;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Each output of each node version, summed up.
fn fingerprints() -> BTreeMap<String, Vec<f64>> {
    let mut out = BTreeMap::new();
    for spec in format::public().filter(|s| !s.is_output()) {
        let mut inputs = Vec::new();
        for port in spec.inputs.iter().filter(|p| !p.optional) {
            let src = match port.kind {
                PortKind::Color => "paint",
                PortKind::Terrain => "lay",
                PortKind::Points => "pts",
                _ => "feed",
            };
            inputs.push(format!(r#""{}": "{src}""#, port.name));
        }
        let json = format!(
            r#"{{ "nodes": {{ "feed": {{ "type": "noise.fbm", "params": {{ "scale": 400, "height": 120, "base": 40, "seed": 1 }} }},
            "paint": {{ "type": "color.gradient", "inputs": {{ "in": "feed" }} }}, "lay": {{ "type": "terrain.layers" }}, "pts": {{ "type": "points.scatter", "inputs": {{ "height": "feed" }}, "params": {{ "spacing": 128, "seed": 2 }} }},
            "n": {{ "type": "{}", "inputs": {{ {} }}, "params": {{ "seed": 3 }} }} }} }}"#,
            spec.type_name(),
            inputs.join(", ")
        );
        let mut g = Graph::from_json(&json).unwrap();
        if !spec.has_seed() {
            g.nodes.get_mut("n").unwrap().params.clear();
        }
        let (c, _) = compile(&g, Space::new(2048), 256);
        let e = Engine::new(Default::default());
        for port in spec.outputs {
            if port.kind.is_geometry() {
                // Geometry: how many, and where they are on the whole.
                let geo = e.geometry(&c, c.source("n", Some(port.name)).unwrap(), &never).unwrap();
                let pos: Vec<[f64; 3]> = match &*geo {
                    massif_terrain::kernel::Geo::Points(p) => p.items.iter().map(|p| p.pos).collect(),
                    massif_terrain::kernel::Geo::Splines(s) => s.lines.iter().flat_map(|l| l.points.iter().copied()).collect(),
                    massif_terrain::kernel::Geo::Footprints(f) => f.items.iter().map(|f| [f.min[0] as f64, 0.0, f.min[1] as f64]).collect(),
                    massif_terrain::kernel::Geo::Instances(i) => i.items.iter().map(|i| i.origin.map(f64::from)).collect(),
                };
                let n = pos.len().max(1) as f64;
                let mut f = vec![pos.len() as f64];
                f.extend((0..3).map(|k| pos.iter().map(|p| p[k]).sum::<f64>() / n));
                out.insert(format!("{}.{}", spec.type_name(), port.name), f);
                continue;
            }
            let grid = e.whole(&c, c.source("n", Some(port.name)).unwrap(), 4, &never).unwrap();
            let d = &grid.data;
            let mean = d.iter().map(|v| *v as f64).sum::<f64>() / d.len() as f64;
            let sd = (d.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / d.len() as f64).sqrt();
            let n = grid.rect.w as usize;
            let mut f = vec![mean, sd];
            f.extend((0..16).map(|k| d[((k / 4) * n / 4 + n / 8) * n + (k % 4) * n / 4 + n / 8] as f64));
            out.insert(format!("{}.{}", spec.type_name(), port.name), f);
        }
    }
    out
}

#[test]
fn released_nodes_give_the_same_results() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden.txt");
    let now = fingerprints();
    if std::env::var_os("MASSIF_WRITE_GOLDEN").is_some() {
        let text: String = now.iter().map(|(k, f)| format!("{k} {}\n", f.iter().map(|v| format!("{v:.6}")).collect::<Vec<_>>().join(" "))).collect();
        std::fs::write(&path, text).unwrap();
    }
    let pinned: BTreeMap<String, Vec<f64>> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let key = parts.next()?.to_owned();
            Some((key, parts.map(|v| v.parse().unwrap()).collect()))
        })
        .collect();
    let mut changed = Vec::new();
    for (key, f) in &now {
        match pinned.get(key) {
            // A new node version: pinned when the file is written again.
            None => changed.push(format!("{key}: not pinned yet")),
            Some(p) => {
                if p.len() != f.len() || p.iter().zip(f).any(|(a, b)| (a - b).abs() > 2e-3 * b.abs().max(1.0)) {
                    changed.push(format!("{key}: {p:?} → {f:?}"));
                }
            }
        }
    }
    assert!(changed.is_empty(), "released nodes changed their results (make new versions instead):\n{}", changed.join("\n"));
}
