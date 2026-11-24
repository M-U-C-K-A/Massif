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
//! Nodes are keyed by their id, which never changes once the node exists
//! (renaming a node changes its `name`, what the editor shows); an input
//! names the node it comes from and, after a dot, which of its outputs (the
//! first when left out). Parameters left out take their defaults. The
//! editor's layout is kept apart, under `ui` (where nodes sit), `frames`
//! and `editor` (the rest, as the editor writes it): moving a node, a frame
//! or a colour never makes anything again. [`crate::registry`] says what
//! every type of node takes.
//!
//! Version 3 adds composite nodes ([`Component`]): a graph of its own, with
//! inputs, outputs and settings exposed, that a node of type
//! `project:<name>` (one of the document's `components`) or `massif:<name>`
//! (one shipped with Massif, see [`crate::components`]) runs as one node and
//! that the editor opens. Inside a component, `in.<name>` is its input
//! `<name>`. [`crate::lower`] unfolds them into plain nodes before compiling.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the `format` field says.
pub const FORMAT: &str = "massif.terrain-graph";
/// The version of the format this crate writes.
pub const VERSION: u32 = 3;

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
    /// Frames gathering nodes under a title, by id; never read to evaluate.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frames: BTreeMap<String, Frame>,
    /// The composite nodes this document defines, by type
    /// (`project:<name>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<String, Component>,
    /// The rest of the editor's layout (frames, points on wires, the
    /// view…), as the editor writes it; never read to evaluate the graph.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub editor: serde_json::Value,
}

impl Default for Graph {
    fn default() -> Self {
        Self { format: format_name(), version: VERSION, terrain: Terrain::default(), nodes: BTreeMap::new(), ui: BTreeMap::new(), frames: BTreeMap::new(), components: BTreeMap::new(), editor: serde_json::Value::Null }
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
    /// What the editor calls it (its type's label when left out); free to
    /// change, unlike its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Left out of the graph: what it feeds takes its first input instead
    /// (the first of the same type as its output).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
}

impl Node {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_owned(), inputs: BTreeMap::new(), params: BTreeMap::new(), note: None, name: None, bypass: false }
    }

    /// Whether its type is a composite (`project:…` or `massif:…`) rather
    /// than a node of the registry (`family.name`).
    pub fn is_component(&self) -> bool {
        is_component_type(&self.kind)
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

/// A frame around nodes in the editor: a title, a colour, a note, the
/// nodes it holds, the frame it sits in. Frames nest.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub title: String,
    /// `#rrggbb`, or the neutral grey when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The frame it sits in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The nodes it holds (moved with it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    /// In graph units: x, y, width, height.
    #[serde(default)]
    pub rect: [f32; 4],
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// A composite node: a graph of its own that runs as one node, with
/// inputs, outputs and settings exposed. It opens in the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Its version: a graph names a later one `project:name@N`; a released
    /// version keeps its results, as nodes' do.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub version: u16,
    /// What the library and the node show.
    pub label: String,
    /// Where the library shows it (`Shapes`, `Erosion`…).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// One sentence on what it makes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PortDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<OutputDecl>,
    /// The settings a node of this type shows, each driving settings of
    /// the nodes inside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed: Vec<Exposed>,
    #[serde(default)]
    pub nodes: BTreeMap<String, Node>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ui: BTreeMap<String, NodeUi>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frames: BTreeMap<String, Frame>,
}

impl Default for Component {
    fn default() -> Self {
        Self { version: 1, label: String::new(), category: String::new(), summary: String::new(), inputs: Vec::new(), outputs: Vec::new(), exposed: Vec::new(), nodes: BTreeMap::new(), ui: BTreeMap::new(), frames: BTreeMap::new() }
    }
}

fn one() -> u16 {
    1
}

fn is_one(v: &u16) -> bool {
    *v == 1
}

/// An input of a component: inside, `in.<name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// What it carries: a socket type's id (`height`, `mask`, `points`…).
    #[serde(rename = "type")]
    pub kind: String,
    /// It can be left unconnected (what reads it then reads nothing).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
}

/// An output of a component: the output of a node inside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDecl {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// `node` or `node.output`, inside the component.
    pub from: String,
}

/// A setting a component shows, standing for settings of the nodes
/// inside (`targets`, each `node.setting`, all of one kind). Its range,
/// unit and default may narrow theirs; left out, the first target's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exposed {
    /// Its name, as the node's `params` write it.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The range the slider shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<[f64; 2]>,
    /// A unit's id (`blocks`, `degrees`, `percent`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// How the panel shows it: `slider`, `field`, `switch`, `choice`…
    /// (left out: as its first target's setting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    /// The group it shows under.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// Folded away until asked for.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub advanced: bool,
}

/// Whether a node type names a composite: `project:<name>` or
/// `massif:<name>` (with `@N` for a later version).
pub fn is_component_type(kind: &str) -> bool {
    kind.starts_with("project:") || kind.starts_with("massif:")
}

/// The name inside a component that stands for its inputs: `in.<name>`.
pub const INPUTS: &str = "in";

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
    !id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Graph {
    /// Whether `other` makes the same terrain: the same nodes and terrain
    /// settings, whatever the editor's layout.
    pub fn same_terrain(&self, other: &Graph) -> bool {
        self.terrain == other.terrain && self.nodes == other.nodes
    }

    /// The graph `text` writes, brought up to this version (see
    /// [`Graph::upgrade`]).
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut g: Graph = serde_json::from_str(text)?;
        g.upgrade();
        Ok(g)
    }

    /// Brings a graph of an earlier version up to this one, losing
    /// nothing: version 2's frames, which the editor kept under `editor`,
    /// become the graph's own, holding the nodes that sit inside them. A
    /// graph of a later version is left as it is (and not built).
    pub fn upgrade(&mut self) {
        if self.version >= 3 {
            return;
        }
        if self.frames.is_empty()
            && let Some(list) = self.editor.get("frames").and_then(|f| f.as_array())
        {
            for (k, f) in list.iter().enumerate() {
                let rect: [f32; 4] = f.get("rect").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default();
                let color = f.get("color").and_then(|c| serde_json::from_value::<[u8; 3]>(c.clone()).ok()).map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
                let inside = |p: [f32; 2]| p[0] >= rect[0] && p[1] >= rect[1] && p[0] <= rect[0] + rect[2] && p[1] <= rect[1] + rect[3];
                let members = self.ui.iter().filter(|(_, u)| inside(u.pos)).map(|(id, _)| id.clone()).collect();
                let title = f.get("title").and_then(|t| t.as_str()).unwrap_or_default().to_owned();
                self.frames.insert(format!("frame_{}", k + 1), Frame { title, color, members, rect, ..Frame::default() });
            }
        }
        self.version = VERSION;
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
        assert!(Graph::from_json(r#"{ "nodes": {}, "editr": {} }"#).is_err());
        // Nothing written for an editor's layout that is not there.
        assert!(!g.to_json().contains("editor"));
    }

    #[test]
    fn keeps_the_editors_layout() {
        let text = r##"{
            "nodes": { "hills": { "type": "noise.fbm" } },
            "ui": { "hills": { "pos": [10, 20] } },
            "editor": { "frames": [{ "title": "Coast", "rect": [0, 0, 300, 200], "color": "#3a6ea5" }], "reroutes": { "out.height": [[5, 5]] } }
        }"##;
        let g = Graph::from_json(text).unwrap();
        assert_eq!(g.editor["frames"][0]["title"], "Coast");
        let again = Graph::from_json(&g.to_json()).unwrap();
        assert_eq!(g, again);
        let mut moved = g.clone();
        moved.editor["frames"][0]["rect"][0] = 40.into();
        moved.ui.get_mut("hills").unwrap().pos = [0.0, 0.0];
        assert!(moved.same_terrain(&g) && moved != g);
        moved.set("hills", "scale", 900);
        assert!(!moved.same_terrain(&g));
    }

    #[test]
    fn version_2_comes_up_to_3() {
        let text = r#"{ "version": 2,
            "nodes": { "a": { "type": "noise.fbm" }, "b": { "type": "noise.fbm" } },
            "ui": { "a": { "pos": [10, 10] }, "b": { "pos": [500, 10] } },
            "editor": { "frames": [{ "title": "Base", "rect": [0, 0, 100, 100], "color": [52, 122, 82] }] } }"#;
        let g = Graph::from_json(text).unwrap();
        assert_eq!(g.version, VERSION);
        let f = &g.frames["frame_1"];
        assert_eq!((f.title.as_str(), f.color.as_deref(), f.members.as_slice()), ("Base", Some("#347a52"), &["a".to_owned()][..]));
        // A later version is left alone.
        let later = Graph::from_json(r#"{ "version": 9, "nodes": {} }"#).unwrap();
        assert_eq!(later.version, 9);
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
