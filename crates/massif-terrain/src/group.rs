//! Composites made and unmade by editing: nodes gathered into a new
//! composite of the document (what the editor's Make a composite does),
//! an instance unfolded back into its nodes, a setting exposed.

use crate::lower::{SEP, component};
use crate::model::{Component, Exposed, Graph, INPUTS, Node, OutputDecl, PortDecl, Source, is_component_type, valid_id};
use crate::registry::{self, PortKind};
use std::collections::{BTreeMap, BTreeSet};

/// The type of what `src` (`node` or `node.output`) carries in `graph`.
pub fn kind_of(graph: &Graph, src: &str) -> Option<PortKind> {
    let s = Source::parse(src);
    let node = graph.nodes.get(s.node)?;
    if is_component_type(&node.kind) {
        let c = component(graph, &node.kind)?;
        let o = match s.output {
            Some(name) => c.outputs.iter().find(|o| o.name == name)?,
            None => c.outputs.first()?,
        };
        let inside = Graph { nodes: c.nodes.clone(), components: graph.components.clone(), ..Graph::default() };
        return kind_of(&inside, &o.from);
    }
    let spec = registry::spec(&node.kind)?;
    let port = match s.output {
        Some(name) => spec.outputs.get(spec.output(name)?)?,
        None => spec.outputs.first()?,
    };
    Some(port.kind)
}

/// An id made from `stem` that `taken` does not hold yet.
fn fresh(stem: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut stem: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
    while stem.contains(SEP) {
        stem = stem.replace(SEP, "_");
    }
    let stem = stem.trim_matches('_').to_owned();
    let stem = if stem.is_empty() { "group".to_owned() } else { stem };
    let mut id = stem.clone();
    let mut k = 2;
    while taken(&id) {
        id = format!("{stem}_{k}");
        k += 1;
    }
    id
}

/// Gathers `nodes` of `graph` into a new composite of the document,
/// labelled `label`, and puts one node of it in their place: what fed them
/// from outside becomes its inputs, what they fed outside its outputs.
/// Gives the new node's id and the composite's type.
pub fn group(graph: &mut Graph, nodes: &[String], label: &str) -> Result<(String, String), String> {
    let set: BTreeSet<&str> = nodes.iter().map(String::as_str).collect();
    if set.is_empty() {
        return Err("no node to gather".into());
    }
    for id in &set {
        let Some(n) = graph.nodes.get(*id) else { return Err(format!("the graph has no node “{id}”")) };
        if registry::spec(&n.kind).is_some_and(|s| s.is_output()) {
            return Err(format!("“{id}” is an output of the graph: it stays outside composites"));
        }
    }
    let kind = format!("project:{}", fresh(label, |k| graph.components.contains_key(&format!("project:{k}")) || crate::components::latest(&format!("massif:{k}")).is_some()));
    let id = fresh(label, |k| graph.nodes.contains_key(k));
    let mut comp = Component { label: label.to_owned(), ..Component::default() };
    // What feeds them from outside: one input each.
    let mut inputs: BTreeMap<String, String> = BTreeMap::new();
    for nid in &set {
        let n = &graph.nodes[*nid];
        for (input, src) in &n.inputs {
            let from = Source::parse(src).node;
            if set.contains(from) || inputs.contains_key(src) {
                continue;
            }
            let name = fresh(input.trim_start_matches('@'), |k| comp.inputs.iter().any(|p| p.name == k));
            let kind = kind_of(graph, src).map_or("height", |k| k.id());
            comp.inputs.push(PortDecl { name: name.clone(), label: String::new(), kind: kind.to_owned(), optional: false, help: String::new() });
            inputs.insert(src.clone(), name);
        }
    }
    // What they feed outside: one output each.
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    for (oid, n) in &graph.nodes {
        if set.contains(oid.as_str()) {
            continue;
        }
        for src in n.inputs.values() {
            let s = Source::parse(src);
            if !set.contains(s.node) || outputs.contains_key(src) {
                continue;
            }
            let stem = match s.output {
                Some(o) => format!("{}_{o}", s.node),
                None => s.node.to_owned(),
            };
            let name = fresh(&stem, |k| comp.outputs.iter().any(|o| o.name == k));
            comp.outputs.push(OutputDecl { name: name.clone(), label: String::new(), from: src.clone() });
            outputs.insert(src.clone(), name);
        }
    }
    if comp.outputs.is_empty() {
        // Nothing outside reads them yet: the last of them is the output.
        let last = set.iter().next_back().expect("not empty");
        comp.outputs.push(OutputDecl { name: "out".into(), label: String::new(), from: (*last).to_owned() });
    }
    // The nodes move inside, reading their outside inputs as `in.<name>`.
    let mut pos = [0.0f32; 2];
    let mut placed = 0.0;
    for nid in &set {
        let mut n = graph.nodes.remove(*nid).expect("checked");
        for src in n.inputs.values_mut() {
            if let Some(name) = inputs.get(src.as_str()) {
                *src = format!("{INPUTS}.{name}");
            }
        }
        comp.nodes.insert((*nid).to_owned(), n);
        if let Some(ui) = graph.ui.remove(*nid) {
            pos = [pos[0] + ui.pos[0], pos[1] + ui.pos[1]];
            placed += 1.0;
            comp.ui.insert((*nid).to_owned(), ui);
        }
    }
    for f in graph.frames.values_mut() {
        f.members.retain(|m| !set.contains(m.as_str()));
    }
    // One node of it in their place, fed as they were, read as they were.
    let mut node = Node::new(&kind);
    node.name = Some(label.to_owned());
    for (src, name) in &inputs {
        node.inputs.insert(name.clone(), src.clone());
    }
    for n in graph.nodes.values_mut() {
        for src in n.inputs.values_mut() {
            if let Some(name) = outputs.get(src.as_str()) {
                *src = format!("{id}.{name}");
            }
        }
    }
    graph.nodes.insert(id.clone(), node);
    if placed > 0.0 {
        graph.ui.insert(id.clone(), crate::model::NodeUi { pos: [pos[0] / placed, pos[1] / placed], collapsed: false });
    }
    graph.components.insert(kind.clone(), comp);
    Ok((id, kind))
}

/// Puts the nodes of composite node `id` back in its place, under ids
/// `<id>_<inside>`, with the settings its exposed ones gave them. The
/// composite stays among the document's for its other nodes.
pub fn ungroup(graph: &mut Graph, id: &str) -> Result<Vec<String>, String> {
    let Some(node) = graph.nodes.get(id).cloned() else { return Err(format!("the graph has no node “{id}”")) };
    let Some(comp) = component(graph, &node.kind).map(|c| c.into_owned()) else { return Err(format!("“{id}” is not a composite")) };
    let ids: BTreeMap<String, String> = comp.nodes.keys().map(|k| (k.clone(), fresh(&format!("{id}_{k}"), |x| graph.nodes.contains_key(x) && x != id))).collect();
    let inside = |src: &str| -> Option<String> {
        let s = Source::parse(src);
        if s.node == INPUTS {
            return node.inputs.get(s.output.unwrap_or("")).cloned();
        }
        let to = ids.get(s.node)?;
        Some(match s.output {
            Some(o) => format!("{to}.{o}"),
            None => to.clone(),
        })
    };
    let mut set: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
    for e in &comp.exposed {
        let Some(v) = node.params.get(&e.name).or(e.default.as_ref()) else { continue };
        for t in &e.targets {
            if let Some((n, p)) = t.split_once('.') {
                set.entry(n.to_owned()).or_default().insert(p.to_owned(), v.clone());
            }
        }
    }
    let at = graph.ui.get(id).map_or([0.0, 0.0], |u| u.pos);
    graph.nodes.remove(id);
    graph.ui.remove(id);
    let mut made = Vec::new();
    for (k, n) in &comp.nodes {
        let mut n = n.clone();
        n.inputs = n.inputs.iter().filter_map(|(i, s)| inside(s).map(|s| (i.clone(), s))).collect();
        if let Some(s) = set.get(k) {
            n.params.extend(s.clone());
        }
        let to = ids[k].clone();
        if let Some(ui) = comp.ui.get(k) {
            graph.ui.insert(to.clone(), crate::model::NodeUi { pos: [at[0] + ui.pos[0], at[1] + ui.pos[1]], collapsed: ui.collapsed });
        }
        graph.nodes.insert(to.clone(), n);
        made.push(to);
    }
    // What read its outputs reads the nodes that made them.
    let outs: BTreeMap<String, String> = comp.outputs.iter().filter_map(|o| inside(&o.from).map(|s| (o.name.clone(), s))).collect();
    let first = comp.outputs.first().and_then(|o| outs.get(&o.name)).cloned();
    for n in graph.nodes.values_mut() {
        n.inputs.retain(|_, src| {
            let s = Source::parse(src);
            if s.node != id {
                return true;
            }
            let to = match s.output {
                Some(o) => outs.get(o).cloned(),
                None => first.clone(),
            };
            match to {
                Some(t) => {
                    *src = t;
                    true
                }
                None => false,
            }
        });
    }
    Ok(made)
}

/// Shows setting `exposed.name` on composite `kind` of the document,
/// standing for the settings `exposed.targets` of its nodes (replacing
/// one of that name).
pub fn expose(graph: &mut Graph, kind: &str, exposed: Exposed) -> Result<(), String> {
    let Some(comp) = graph.components.get(kind) else { return Err(format!("the document has no composite “{kind}” (those of Massif are edited as a copy)")) };
    if !valid_id(&exposed.name) {
        return Err(format!("“{}” cannot name a setting: letters, digits, “_” and “-”", exposed.name));
    }
    if exposed.targets.is_empty() {
        return Err("an exposed setting stands for at least one setting inside".into());
    }
    let mut first = None;
    for t in &exposed.targets {
        let Some((n, p)) = t.split_once('.') else { return Err(format!("“{t}” is not node.setting")) };
        let Some(node) = comp.nodes.get(n) else { return Err(format!("{} has no node “{n}”", comp.label)) };
        if is_component_type(&node.kind) {
            // A setting that a composite inside shows.
            let inner = component(graph, &node.kind).ok_or_else(|| format!("no composite is called “{}”", node.kind))?;
            if !inner.exposed.iter().any(|e| e.name == p) {
                return Err(format!("{} shows no setting “{p}”", inner.label));
            }
            continue;
        }
        let Some(spec) = registry::spec(&node.kind) else { return Err(format!("“{}” is not a type of node", node.kind)) };
        let Some((_, ps)) = spec.param(p) else { return Err(format!("{} has no setting “{p}”", spec.id)) };
        let k = std::mem::discriminant(&ps.kind);
        if first.is_some_and(|f| f != k) {
            return Err(format!("“{t}” is not the same kind of setting as the first"));
        }
        first = Some(k);
    }
    let comp = graph.components.get_mut(kind).expect("checked");
    comp.exposed.retain(|e| e.name != exposed.name);
    comp.exposed.push(exposed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Severity, compile};
    use crate::engine::{Engine, never};
    use crate::space::Space;

    fn chain() -> Graph {
        Graph::from_json(
            r#"{ "nodes": {
                "hills": { "type": "noise.fbm", "params": { "scale": 600, "height": 120 } },
                "lift": { "type": "adjust.levels", "inputs": { "in": "hills" }, "params": { "gamma": 1.4 } },
                "worn": { "type": "erosion.thermal", "inputs": { "height": "lift" } },
                "slope": { "type": "mask.slope", "inputs": { "height": "worn" } },
                "out": { "type": "output.map", "inputs": { "height": "worn.height", "mask.steep": "slope" } }
            }, "ui": { "lift": { "pos": [100, 0] }, "worn": { "pos": [300, 0] } } }"#,
        )
        .unwrap()
    }

    fn heights(g: &Graph) -> Vec<f32> {
        let (c, d) = compile(g, Space::new(1024), 256);
        assert!(d.iter().all(|d| d.severity != Severity::Error), "{d:#?}");
        let e = Engine::new(Default::default());
        let lod = c.space.lod_for(256);
        e.region(&c, c.output("height").unwrap(), lod, c.space.whole(lod), &never).unwrap().data
    }

    /// Gathering nodes into a composite, and unfolding it again, changes
    /// nothing the graph makes.
    #[test]
    fn grouping_keeps_the_terrain() {
        let before = heights(&chain());
        let mut g = chain();
        let (id, kind) = group(&mut g, &["lift".into(), "worn".into()], "Worn hills").unwrap();
        assert_eq!((id.as_str(), kind.as_str()), ("worn_hills", "project:worn_hills"));
        let c = &g.components[&kind];
        assert_eq!(c.inputs.len(), 1);
        assert_eq!(c.inputs[0].kind, "height");
        assert_eq!(c.nodes["lift"].inputs["in"], "in.in");
        assert_eq!(c.outputs.iter().map(|o| o.from.as_str()).collect::<Vec<_>>(), ["worn.height", "worn"]);
        assert_eq!(g.nodes[&id].inputs["in"], "hills");
        assert!(g.nodes["slope"].inputs["height"].starts_with("worn_hills."));
        assert_eq!(g.ui[&id].pos, [200.0, 0.0]);
        assert_eq!(heights(&g), before);
        // Exposing a setting, then setting it on the node.
        expose(&mut g, &kind, Exposed { name: "gamma".into(), label: "Gamma".into(), help: String::new(), targets: vec!["lift.gamma".into()], default: None, min: None, max: None, soft: None, unit: None, control: None, category: String::new(), advanced: false }).unwrap();
        let bad = Exposed { name: "bad".into(), targets: vec!["lift.nope".into()], ..g.components[&kind].exposed[0].clone() };
        assert!(expose(&mut g, &kind, bad).is_err());
        g.set(&id, "gamma", 1.4);
        assert_eq!(heights(&g), before);
        // Unfolded again: its nodes back, the setting given them.
        let made = ungroup(&mut g, &id).unwrap();
        assert_eq!(made, ["worn_hills_lift", "worn_hills_worn"]);
        assert_eq!(g.nodes["worn_hills_lift"].params["gamma"], 1.4);
        assert_eq!(g.nodes["out"].inputs["height"], "worn_hills_worn.height");
        assert_eq!(heights(&g), before);
    }
}
