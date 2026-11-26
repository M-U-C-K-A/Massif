//! The composite nodes Massif ships (`massif:<name>`): graphs of plain
//! nodes, written in JSON beside the crate, that a graph uses as one node
//! and that the editor opens read-only (Edit a copy makes a `project:`
//! composite of the document from one).
//!
//! A released version keeps its results, as a node's does: changing what
//! one makes is a new file, `<name>@2.json`, listed below beside the old.

use crate::model::Component;
use std::sync::OnceLock;

/// Each composite shipped: its type, version and JSON.
const SHIPPED: &[(&str, u16, &str)] = &[("massif:mountain", 1, include_str!("../components/mountain.json")), ("massif:alpine-range", 1, include_str!("../components/alpine-range.json")), ("massif:alpine-range", 2, include_str!("../components/alpine-range-2.json")), ("massif:village-generator", 1, include_str!("../components/village-generator.json")), ("massif:cave-system", 1, include_str!("../components/cave-system.json"))];

/// Every composite shipped, parsed once: type, version, definition.
pub fn all() -> &'static [(&'static str, u16, Component)] {
    static ALL: OnceLock<Vec<(&'static str, u16, Component)>> = OnceLock::new();
    ALL.get_or_init(|| {
        SHIPPED
            .iter()
            .map(|&(kind, version, text)| {
                let mut c: Component = serde_json::from_str(text).unwrap_or_else(|e| panic!("{kind}: {e}"));
                c.version = version;
                (kind, version, c)
            })
            .collect()
    })
}

/// The composite shipped as `kind` (`massif:name`) at `version`.
pub fn builtin(kind: &str, version: u16) -> Option<Component> {
    all().iter().find(|(k, v, _)| *k == kind && *v == version).map(|(_, _, c)| c.clone())
}

/// The composites the library offers: the latest version of each shipped
/// one not hidden, by type.
pub fn offered() -> Vec<(&'static str, u16, &'static Component)> {
    let mut out: Vec<(&'static str, u16, &'static Component)> = Vec::new();
    for (kind, version, c) in all() {
        if let Some((latest, _)) = latest(kind)
            && latest == *version
            && !c.hidden
        {
            out.push((kind, *version, c));
        }
    }
    out
}

/// The latest version of the composite shipped as `kind`.
pub fn latest(kind: &str) -> Option<(u16, &'static Component)> {
    all().iter().filter(|(k, _, _)| *k == kind).max_by_key(|(_, v, _)| *v).map(|(_, v, c)| (*v, c))
}

#[cfg(test)]
mod tests {
    use crate::compile::{Severity, compile};
    use crate::model::{Graph, Node};
    use crate::space::Space;

    /// Every composite shipped compiles with its defaults, its exposed
    /// settings each standing for settings that exist.
    #[test]
    fn the_library_offers_what_is_not_hidden() {
        let kinds: Vec<&str> = super::offered().iter().map(|(k, _, _)| *k).collect();
        assert!(kinds.contains(&"massif:alpine-range") && !kinds.contains(&"massif:mountain"), "{kinds:?}");
        assert!(super::builtin("massif:mountain", 1).is_some(), "hidden, still there for the graphs using it");
    }

    #[test]
    fn shipped_composites_hold_together() {
        for (kind, version, c) in super::all() {
            assert!(!c.label.is_empty() && !c.summary.is_empty() && !c.outputs.is_empty(), "{kind}");
            for e in &c.exposed {
                for t in &e.targets {
                    let (n, p) = t.split_once('.').unwrap_or_else(|| panic!("{kind}: {t}"));
                    let node = c.nodes.get(n).unwrap_or_else(|| panic!("{kind}: no node {n}"));
                    let spec = crate::registry::spec(&node.kind).unwrap_or_else(|| panic!("{kind}: {}", node.kind));
                    assert!(spec.param(p).is_some(), "{kind}: {t}");
                }
            }
            let mut g = Graph::default();
            let id = g.add(Node::new(&if *version == 1 { kind.to_string() } else { format!("{kind}@{version}") }));
            let out = g.add(Node::new("output.map"));
            // Its inputs that must be fed, by a relief; its first output to
            // the Map Output's input of its type.
            let relief = g.add(Node::new("noise.fbm"));
            for i in c.inputs.iter().filter(|i| !i.optional) {
                g.link(&relief, &id, &i.name);
            }
            let first = &c.outputs[0];
            let from = c.nodes.get(first.from.split('.').next().unwrap_or("")).and_then(|n| crate::registry::spec(&n.kind)).and_then(|s| s.outputs.iter().find(|o| Some(o.name) == first.from.split('.').nth(1)));
            let into = match from.map(|o| o.kind) {
                Some(crate::registry::PortKind::Layer) => "caves",
                Some(crate::registry::PortKind::Mask) => "mask.out",
                _ => "height",
            };
            g.link(&format!("{id}.{}", first.name), &out, into);
            let (_, d) = compile(&g, Space::new(1024), 256);
            assert!(d.iter().all(|d| d.severity != Severity::Error), "{kind}: {d:#?}");
        }
    }
}
