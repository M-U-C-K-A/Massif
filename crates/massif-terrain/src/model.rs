//! A terrain graph as it is written: a readable JSON document.
//!
//! ```json
//! {
//!   "format": "massif.terrain-graph",
//!   "version": 2,
//!   "terrain": { "size": 8192, "seed": 1337 },
//!   "nodes": {
//!     "hills":  { "type": "noise.fbm", "params": { "scale": 1200, "height": 180, "base": 50 } },
//!     "relief": { "type": "adjust.levels", "inputs": { "in": "hills" }, "params": { "gamma": 1.6 } },
//!     "out":    { "type": "output.map", "inputs": { "height": "relief" } }
//!   }
//! }
//! ```
//!
//! Nodes are named (the names are the ids), an input names the node it
//! comes from and, after a dot, which of its outputs (the first when left
//! out). Parameters left out take their defaults. The editor's layout is
//! kept apart, under `ui`, so that reading or writing a graph never needs
//! it. [`crate::registry`] says what every type of node takes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the `format` field says.
pub const FORMAT: &str = "massif.terrain-graph";
/// The version of the format this crate writes.
pub const VERSION: u32 = 2;

fn format_name() -> String {
    FORMAT.to_owned()
}

fn version() -> u32 {
    VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    #[serde(default = "format_name")]
    pub format: String,
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub terrain: Terrain,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    /// Where nodes sit in the editor, by node.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ui: BTreeMap<String, NodeUi>,
}

impl Default for Graph {
    fn default() -> Self {
        Self { format: format_name(), version: VERSION, terrain: Terrain::default(), nodes: BTreeMap::new(), ui: BTreeMap::new() }
    }
}

/// What the whole graph shares.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terrain {
    /// Side of the square of the world the terrain covers, in blocks
    /// (left out: the map's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// The seed all the graph's randomness starts from.
    #[serde(default)]
    pub seed: u64,
    /// The sea level, in blocks (left out: the world's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea_level: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// The type of node, as `family.name` (see the registry).
    #[serde(rename = "type")]
    pub kind: String,
    /// Each input, by name: the node it comes from, as `node` or `node.output`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    /// Settings by name; those left out take their defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    /// A note for whoever reads the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Node {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_owned(), inputs: BTreeMap::new(), params: BTreeMap::new(), note: None }
    }

    /// With a setting.
    pub fn with(mut self, param: &str, value: impl Into<serde_json::Value>) -> Self {
        self.params.insert(param.to_owned(), value.into());
        self
    }

    /// With an input coming from `source` (`node` or `node.output`).
    pub fn input(mut self, input: &str, source: &str) -> Self {
        self.inputs.insert(input.to_owned(), source.to_owned());
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeUi {
    pub pos: [f32; 2],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// An output of a node, as an input names it: `node` or `node.output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source<'a> {
    pub node: &'a str,
    pub output: Option<&'a str>,
}

impl<'a> Source<'a> {
    pub fn parse(s: &'a str) -> Self {
        match s.split_once('.') {
            Some((node, output)) => Self { node, output: Some(output) },
            None => Self { node: s, output: None },
        }
    }
}

/// Whether `id` can name a node: letters, digits, `_` and `-`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Graph {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("graphs serialise")
    }

    /// Adds a node under a fresh name made from its type (`fbm`, `fbm_2`…),
    /// and gives the name.
    pub fn add(&mut self, node: Node) -> String {
        let stem = node.kind.rsplit('.').next().unwrap_or("node").replace(|c: char| !(c.is_ascii_alphanumeric() || c == '_'), "_");
        let mut id = stem.clone();
        let mut k = 2;
        while self.nodes.contains_key(&id) {
            id = format!("{stem}_{k}");
            k += 1;
        }
        self.nodes.insert(id.clone(), node);
        id
    }

    /// Feeds `to`'s input `input` from `source` (`node` or `node.output`).
    pub fn link(&mut self, source: &str, to: &str, input: &str) {
        if let Some(n) = self.nodes.get_mut(to) {
            n.inputs.insert(input.to_owned(), source.to_owned());
        }
    }

    /// Sets a setting of a node.
    pub fn set(&mut self, node: &str, param: &str, value: impl Into<serde_json::Value>) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.params.insert(param.to_owned(), value.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes() {
        let text = r#"{
            "nodes": {
                "hills": { "type": "noise.fbm", "params": { "scale": 1200 } },
                "out": { "type": "output.map", "inputs": { "height": "hills.height" } }
            }
        }"#;
        let g = Graph::from_json(text).unwrap();
        assert_eq!(g.format, FORMAT);
        assert_eq!(g.version, VERSION);
        assert_eq!(g.nodes["out"].inputs["height"], "hills.height");
        let again = Graph::from_json(&g.to_json()).unwrap();
        assert_eq!(g, again);
        // A misspelt field is an error, not silently dropped.
        assert!(Graph::from_json(r#"{ "nodes": { "a": { "type": "noise.fbm", "param": {} } } }"#).is_err());
    }

    #[test]
    fn names_nodes() {
        let mut g = Graph::default();
        assert_eq!(g.add(Node::new("noise.fbm")), "fbm");
        assert_eq!(g.add(Node::new("noise.fbm")), "fbm_2");
        assert_eq!(Source::parse("fbm_2.flow"), Source { node: "fbm_2", output: Some("flow") });
        assert!(valid_id("fbm_2") && !valid_id("a.b") && !valid_id(""));
    }
}
