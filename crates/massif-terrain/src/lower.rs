//! Composites unfolded: a graph whose nodes are all of the registry, the
//! only kind [`crate::compile`] evaluates.
//!
//! A node of a composite type becomes the nodes inside it, each under the
//! id `<node>__<inside>` (so that its seed, made from its id, differs from
//! one instance to the next), its exposed settings set on the nodes they
//! stand for, its inputs wired to what feeds the node, and what read its
//! outputs wired to the nodes inside that make them. A bypassed node gives
//! way to what feeds its first input. Nothing of the editor's layout is
//! kept. Composites in composites unfold the same way, to any depth.
//!
//! The cache keys of the engine hash what a node does, not its id: two
//! instances with the same settings and inputs share their tiles, and
//! opening, closing or renaming a composite makes nothing again.

use crate::compile::Diagnostic;
use crate::model::{Component, Graph, INPUTS, Node, Source, is_component_type};
use crate::registry;
use std::collections::BTreeMap;

/// How deep composites may sit in one another.
pub const MAX_DEPTH: usize = 16;

/// What joins an instance's id to the id of a node inside it.
pub const SEP: &str = "__";

/// Whether `graph` has anything to unfold: a composite or a bypassed node.
pub fn needed(graph: &Graph) -> bool {
    graph.nodes.values().any(|n| n.is_component() || n.bypass)
}

/// The composite type `kind` names (`project:name`, `massif:name`, with
/// `@N`), from the document or from those Massif ships.
pub fn component<'a>(graph: &'a Graph, kind: &str) -> Option<std::borrow::Cow<'a, Component>> {
    let (base, version) = match kind.split_once('@') {
        Some((b, v)) => (b, v.parse::<u16>().ok()?),
        None => (kind, 1),
    };
    if let Some(c) = graph.components.get(base) {
        return (c.version == version).then_some(std::borrow::Cow::Borrowed(c));
    }
    crate::components::builtin(base, version).map(std::borrow::Cow::Owned)
}

/// `graph` with its composites unfolded and its bypassed nodes taken out;
/// the diagnostics name paths of `graph`.
pub fn lower(graph: &Graph) -> (Graph, Vec<Diagnostic>) {
    let mut out = Graph { format: graph.format.clone(), version: graph.version, terrain: graph.terrain.clone(), ..Graph::default() };
    let mut diags = Vec::new();
    let mut alias: BTreeMap<String, String> = BTreeMap::new();
    let mut l = Lowering { graph, out: &mut out, alias: &mut alias, diags: &mut diags };
    l.scope(&graph.nodes, "", "nodes", &BTreeMap::new(), &BTreeMap::new(), 0);
    // Every input follows the aliases to the node that really makes it.
    for node in out.nodes.values_mut() {
        node.inputs.retain(|_, src| match chase(&alias, src) {
            Some(s) => {
                *src = s;
                true
            }
            None => false,
        });
    }
    (out, diags)
}

/// Where `src` leads through the aliases: the source of the unfolded
/// graph that makes it; `None` for nothing (an input left empty).
fn chase(alias: &BTreeMap<String, String>, src: &str) -> Option<String> {
    let mut s = src.to_owned();
    for _ in 0..64 {
        match alias.get(&s) {
            Some(next) => s = next.clone(),
            None => break,
        }
    }
    (!s.is_empty()).then_some(s)
}

/// The source of the unfolded graph that makes `src` of `graph` (`node`,
/// `node.output`, or a path inside composites, `inst__inner.output`): what
/// the editor previews for a composite node, an output of one, or a
/// bypassed node. `None` when it makes nothing.
pub fn resolve(graph: &Graph, src: &str) -> Option<String> {
    let mut out = Graph::default();
    let mut diags = Vec::new();
    let mut alias: BTreeMap<String, String> = BTreeMap::new();
    let mut l = Lowering { graph, out: &mut out, alias: &mut alias, diags: &mut diags };
    l.scope(&graph.nodes, "", "nodes", &BTreeMap::new(), &BTreeMap::new(), 0);
    let s = chase(&alias, src)?;
    out.nodes.contains_key(Source::parse(&s).node).then_some(s)
}

struct Lowering<'a> {
    graph: &'a Graph,
    out: &'a mut Graph,
    /// A source in the unfolded graph that stands for another: an
    /// instance's output for the node inside that makes it, a bypassed node
    /// for what feeds it; "" for nothing (an input left empty).
    alias: &'a mut BTreeMap<String, String>,
    diags: &'a mut Vec<Diagnostic>,
}

impl Lowering<'_> {
    /// The nodes of one graph (the document's, or a component's inside an
    /// instance): `prefix` before their ids, `path` before the paths of
    /// diagnostics, `bound` what each `in.<name>` stands for, `set` the
    /// settings the instance's exposed ones give, by node then setting.
    fn scope(&mut self, nodes: &BTreeMap<String, Node>, prefix: &str, path: &str, bound: &BTreeMap<String, String>, set: &BTreeMap<String, BTreeMap<String, serde_json::Value>>, depth: usize) {
        // Where a source of this scope goes in the unfolded graph.
        let flat = |src: &str| -> String {
            let s = Source::parse(src);
            if s.node == INPUTS && !prefix.is_empty() {
                return bound.get(s.output.unwrap_or("")).cloned().unwrap_or_default();
            }
            match s.output {
                Some(o) => format!("{prefix}{}.{o}", s.node),
                None => format!("{prefix}{}", s.node),
            }
        };
        for (id, node) in nodes {
            let here = format!("{path}.{id}");
            let me = format!("{prefix}{id}");
            if id.contains(SEP) {
                self.diags.push(Diagnostic::error(here.clone(), format!("a node's id may not hold “{SEP}”")));
            }
            let inputs: BTreeMap<String, String> = node.inputs.iter().map(|(k, v)| (k.clone(), flat(v))).collect();
            let mut params = node.params.clone();
            if let Some(s) = set.get(id) {
                params.extend(s.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            if node.bypass {
                // What it feeds takes its first input of the type it makes.
                let through = self.bypassed(node, &inputs);
                self.alias.insert(me.clone(), through.clone().unwrap_or_default());
                for o in self.outputs_of(node) {
                    self.alias.insert(format!("{me}.{o}"), through.clone().unwrap_or_default());
                }
                if through.is_none() {
                    self.diags.push(Diagnostic::warning(here, "bypassed with nothing to pass on: what it fed reads nothing"));
                }
                continue;
            }
            if !is_component_type(&node.kind) {
                self.out.nodes.insert(me, Node { kind: node.kind.clone(), inputs, params, note: None, name: None, bypass: false });
                continue;
            }
            let Some(comp) = component(self.graph, &node.kind) else {
                self.diags.push(Diagnostic::error(format!("{here}.type"), format!("no composite is called “{}”", node.kind)));
                continue;
            };
            if depth >= MAX_DEPTH {
                self.diags.push(Diagnostic::error(here, format!("composites sit more than {MAX_DEPTH} deep in one another (one holds itself?)")));
                continue;
            }
            // Its inputs, by name, for `in.<name>` inside.
            let mut inner_bound = BTreeMap::new();
            for p in &comp.inputs {
                inner_bound.insert(p.name.clone(), inputs.get(&p.name).cloned().unwrap_or_default());
            }
            for k in inputs.keys().filter(|k| !comp.inputs.iter().any(|p| &p.name == *k)) {
                self.diags.push(Diagnostic::error(format!("{here}.inputs.{k}"), format!("{} has no input “{k}”", comp.label)));
            }
            // Its exposed settings, onto the nodes they stand for.
            let mut inner_set: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
            for e in &comp.exposed {
                let Some(value) = params.get(&e.name).or(e.default.as_ref()) else { continue };
                for t in &e.targets {
                    if let Some((n, p)) = t.split_once('.') {
                        inner_set.entry(n.to_owned()).or_default().insert(p.to_owned(), value.clone());
                    }
                }
            }
            for k in params.keys().filter(|k| !comp.exposed.iter().any(|e| &e.name == *k)) {
                self.diags.push(Diagnostic::error(format!("{here}.params.{k}"), format!("{} shows no setting “{k}”", comp.label)));
            }
            let inner = format!("{me}{SEP}");
            self.scope(&comp.nodes, &inner, &format!("{here}<{}>", node.kind), &inner_bound, &inner_set, depth + 1);
            // Its outputs: the nodes inside that make them.
            for (k, o) in comp.outputs.iter().enumerate() {
                let s = Source::parse(&o.from);
                let target = if s.node == INPUTS { inner_bound.get(s.output.unwrap_or("")).cloned().unwrap_or_default() } else { format!("{inner}{}", o.from) };
                self.alias.insert(format!("{me}.{}", o.name), target.clone());
                if k == 0 {
                    self.alias.insert(me.clone(), target);
                }
            }
        }
    }

    /// The names of `node`'s outputs (a node of the registry or a
    /// composite).
    fn outputs_of(&self, node: &Node) -> Vec<String> {
        if is_component_type(&node.kind) {
            return component(self.graph, &node.kind).map(|c| c.outputs.iter().map(|o| o.name.clone()).collect()).unwrap_or_default();
        }
        registry::spec(&node.kind).map(|s| s.outputs.iter().map(|p| p.name.to_owned()).collect()).unwrap_or_default()
    }

    /// What a bypassed node passes on: its first connected input of the
    /// type of its first output, else its first connected input.
    fn bypassed(&self, node: &Node, inputs: &BTreeMap<String, String>) -> Option<String> {
        if let Some(spec) = registry::spec(&node.kind) {
            let want = spec.outputs.first().map(|o| o.kind);
            let first = |same: bool| spec.inputs.iter().filter(|p| !same || Some(p.kind) == want).find_map(|p| inputs.get(p.name).filter(|s| !s.is_empty()).cloned());
            return first(true).or_else(|| first(false));
        }
        if let Some(c) = component(self.graph, &node.kind) {
            return c.inputs.iter().find_map(|p| inputs.get(&p.name).filter(|s| !s.is_empty()).cloned());
        }
        inputs.values().find(|s| !s.is_empty()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(text: &str) -> Graph {
        Graph::from_json(text).unwrap()
    }

    #[test]
    fn composites_unfold_into_their_nodes() {
        let g = graph(
            r#"{
            "components": { "project:hills": {
                "label": "Hills",
                "inputs": [{ "name": "base", "type": "height", "optional": true }],
                "outputs": [{ "name": "height", "from": "mix" }],
                "exposed": [{ "name": "tall", "label": "Height", "targets": ["noise.height"], "default": 90 }],
                "nodes": {
                    "noise": { "type": "noise.fbm", "params": { "scale": 400 } },
                    "mix": { "type": "combine.blend", "inputs": { "a": "in.base", "b": "noise" }, "params": { "mode": "max" } }
                } } },
            "nodes": {
                "flat": { "type": "noise.fbm", "params": { "height": 5 } },
                "one": { "type": "project:hills", "inputs": { "base": "flat" }, "params": { "tall": 150 } },
                "two": { "type": "project:hills", "name": "Other hills" },
                "out": { "type": "output.map", "inputs": { "height": "one.height" } }
            } }"#,
        );
        let (l, d) = lower(&g);
        assert!(d.is_empty(), "{d:#?}");
        let ids: Vec<&str> = l.nodes.keys().map(String::as_str).collect();
        assert_eq!(ids, ["flat", "one__mix", "one__noise", "out", "two__mix", "two__noise"]);
        assert_eq!(l.nodes["out"].inputs["height"], "one__mix");
        assert_eq!(l.nodes["one__mix"].inputs["a"], "flat");
        assert_eq!(l.nodes["one__mix"].inputs["b"], "one__noise");
        assert!(!l.nodes["two__mix"].inputs.contains_key("a"), "an input left empty");
        assert_eq!(l.nodes["one__noise"].params["height"], 150);
        assert_eq!(l.nodes["two__noise"].params["height"], 90, "the exposed setting's default");
        assert_eq!(l.nodes["two__noise"].params["scale"], 400);
        // An optional input of a composite left empty, where a node inside
        // needs it, is that node's error, under its unfolded id.
        let (_, diags) = crate::compile::compile(&g, crate::space::Space::new(1024), 256);
        let errors: Vec<&str> = diags.iter().filter(|d| d.severity == crate::compile::Severity::Error).map(|d| d.path.as_str()).collect();
        assert_eq!(errors, ["nodes.two__mix.inputs"]);
    }

    #[test]
    fn sources_resolve_to_the_nodes_that_make_them() {
        let g = graph(
            r#"{
            "components": { "project:c": { "label": "C", "outputs": [{ "name": "h", "from": "n" }, { "name": "m", "from": "s" }],
                "nodes": { "n": { "type": "noise.fbm" }, "s": { "type": "mask.slope", "inputs": { "height": "n" } } } } },
            "nodes": { "x": { "type": "project:c" }, "b": { "type": "adjust.levels", "inputs": { "in": "x.h" }, "bypass": true }, "e": { "type": "adjust.levels" , "bypass": true } } }"#,
        );
        assert_eq!(resolve(&g, "x").as_deref(), Some("x__n"));
        assert_eq!(resolve(&g, "x.m").as_deref(), Some("x__s"));
        assert_eq!(resolve(&g, "x__s.mask").as_deref(), Some("x__s.mask"));
        assert_eq!(resolve(&g, "b").as_deref(), Some("x__n"));
        assert_eq!(resolve(&g, "e"), None);
    }

    #[test]
    fn bypassed_nodes_pass_their_input_on() {
        let g = graph(
            r#"{ "nodes": {
                "a": { "type": "noise.fbm" },
                "b": { "type": "adjust.levels", "inputs": { "in": "a" }, "bypass": true },
                "out": { "type": "output.map", "inputs": { "height": "b" } }
            } }"#,
        );
        let (l, d) = lower(&g);
        assert!(d.is_empty(), "{d:#?}");
        assert!(!l.nodes.contains_key("b"));
        assert_eq!(l.nodes["out"].inputs["height"], "a");
    }

    #[test]
    fn composites_nest_and_say_what_is_wrong() {
        let g = graph(
            r#"{
            "components": {
                "project:inner": { "label": "Inner", "outputs": [{ "name": "h", "from": "n" }], "exposed": [{ "name": "s", "targets": ["n.scale"] }],
                    "nodes": { "n": { "type": "noise.fbm" } } },
                "project:outer": { "label": "Outer", "outputs": [{ "name": "h", "from": "i.h" }], "exposed": [{ "name": "size", "targets": ["i.s"] }],
                    "nodes": { "i": { "type": "project:inner" } } },
                "project:loop": { "label": "Loop", "outputs": [{ "name": "h", "from": "me" }], "nodes": { "me": { "type": "project:loop" } } }
            },
            "nodes": {
                "x": { "type": "project:outer", "params": { "size": 777, "nope": 1 } },
                "y": { "type": "project:loop" },
                "z": { "type": "project:missing" },
                "out": { "type": "output.map", "inputs": { "height": "x" } }
            } }"#,
        );
        let (l, d) = lower(&g);
        assert_eq!(l.nodes["x__i__n"].params["scale"], 777);
        assert_eq!(l.nodes["out"].inputs["height"], "x__i__n");
        let paths: Vec<&str> = d.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"nodes.x.params.nope"), "{paths:?}");
        assert!(paths.contains(&"nodes.z.type"), "{paths:?}");
        assert!(paths.iter().any(|p| p.starts_with("nodes.y")), "{paths:?}");
    }
}
