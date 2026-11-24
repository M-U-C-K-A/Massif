//! What a node shows on the canvas, whatever it is: a node of the
//! registry, a composite (`project:…` or `massif:…`) with the inputs,
//! outputs and settings it declares, or, inside an opened composite, the
//! two columns standing for its inputs and its outputs. Also where a
//! node's output is made once composites unfold, for its previews.

use massif_terrain::lower::{SEP, component};
use massif_terrain::model::{Component, Exposed, Graph, INPUTS, Node, Source, is_component_type};
use massif_terrain::registry::{self, Family, NodeSpec, ParamKind, ParamSpec, PortKind};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

/// The id of the column standing for an opened composite's inputs: what
/// its nodes name `in.<name>`.
pub const IN_COLUMN: &str = INPUTS;
/// The id of the column standing for its outputs (no node can have it).
pub const OUT_COLUMN: &str = "@outputs";

/// A socket of a node.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// As links name it (`height`, `mask.moss`, `@scale`).
    pub name: String,
    pub label: String,
    pub kind: PortKind,
    pub optional: bool,
    /// What it stands for when left empty.
    pub default: String,
    pub help: String,
}

impl Port {
    fn of(p: &registry::PortSpec) -> Self {
        Port { name: p.name.to_owned(), label: super::text::name(p.label).to_owned(), kind: p.kind, optional: p.optional, default: p.default.to_owned(), help: p.help.to_owned() }
    }
}

/// What a node is.
#[derive(Debug, Clone)]
pub enum Body {
    Native(&'static NodeSpec),
    /// A composite: its type, what it holds, whether its inside can be
    /// edited (the document's own) or only read (Massif's).
    Composite { kind: String, comp: Box<Component>, editable: bool },
    /// The column of an opened composite's inputs.
    Inputs,
    /// The column of its outputs.
    Outputs,
}

/// A node as the canvas and the settings show it.
#[derive(Debug, Clone)]
pub struct NodeShape {
    pub label: String,
    pub summary: String,
    pub family: Family,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub body: Body,
}

impl NodeShape {
    /// A node of the registry's type.
    pub fn spec(&self) -> Option<&'static NodeSpec> {
        match self.body {
            Body::Native(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_output(&self) -> bool {
        self.spec().is_some_and(|s| s.is_output())
    }

    pub fn is_composite(&self) -> bool {
        matches!(self.body, Body::Composite { .. })
    }

    /// One of an opened composite's two columns.
    pub fn is_column(&self) -> bool {
        matches!(self.body, Body::Inputs | Body::Outputs)
    }

    pub fn composite(&self) -> Option<(&str, &Component, bool)> {
        match &self.body {
            Body::Composite { kind, comp, editable } => Some((kind, comp, *editable)),
            _ => None,
        }
    }

    pub fn output(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|o| o.name == name)
    }

    /// The type an input takes: a declared one, a Map Output's named mask,
    /// or a driven setting (`@name`, a field over the map).
    pub fn input_kind(&self, name: &str) -> Option<PortKind> {
        if name.starts_with('@') {
            return Some(PortKind::Mask);
        }
        self.inputs.iter().find(|i| i.name == name).map(|i| i.kind).or_else(|| self.spec().and_then(|s| s.open_input(name)).map(|o| o.kind))
    }

    /// The registry's shape of a type of node.
    pub fn native(spec: &'static NodeSpec) -> Self {
        NodeShape {
            label: super::text::name(spec.label).to_owned(),
            summary: super::text::name(spec.summary).to_owned(),
            family: spec.family,
            inputs: spec.inputs.iter().map(Port::of).collect(),
            outputs: spec.outputs.iter().map(Port::of).collect(),
            body: Body::Native(spec),
        }
    }

    /// A composite's shape: its declared inputs, and outputs of the type
    /// what makes them inside carries.
    pub fn composite_of(doc: &Graph, kind: &str) -> Option<Self> {
        let comp = component(doc, kind)?.into_owned();
        let inputs: Vec<Port> = comp.inputs.iter().map(|p| Port { name: p.name.clone(), label: label_or(&p.label, &p.name), kind: PortKind::from_id(&p.kind).unwrap_or(PortKind::Height), optional: p.optional, default: if p.optional { "none".to_owned() } else { String::new() }, help: p.help.clone() }).collect();
        let inside = Graph { nodes: comp.nodes.clone(), components: doc.components.clone(), ..Graph::default() };
        let outputs = comp
            .outputs
            .iter()
            .map(|o| {
                let s = Source::parse(&o.from);
                let kind = if s.node == INPUTS { inputs.iter().find(|i| Some(i.name.as_str()) == s.output).map(|i| i.kind) } else { massif_terrain::group::kind_of(&inside, &o.from) };
                Port { name: o.name.clone(), label: label_or(&o.label, &o.name), kind: kind.unwrap_or(PortKind::Height), optional: false, default: String::new(), help: String::new() }
            })
            .collect();
        let family = Family::ALL.into_iter().find(|f| f.label().eq_ignore_ascii_case(&comp.category)).unwrap_or(Family::Shape);
        let editable = doc.components.contains_key(kind.split('@').next().unwrap_or(kind));
        Some(NodeShape { label: super::text::name_owned(&comp.label), summary: super::text::name_owned(&comp.summary), family, inputs, outputs, body: Body::Composite { kind: kind.to_owned(), comp: Box::new(comp), editable } })
    }

    /// The columns of an opened composite: its inputs as outputs of the
    /// left one, its outputs as inputs of the right one.
    pub fn columns(comp: &Component, scope: &Graph) -> (Self, Self) {
        let ins: Vec<Port> = comp.inputs.iter().map(|p| Port { name: p.name.clone(), label: label_or(&p.label, &p.name), kind: PortKind::from_id(&p.kind).unwrap_or(PortKind::Height), optional: p.optional, default: String::new(), help: p.help.clone() }).collect();
        let outs: Vec<Port> = comp
            .outputs
            .iter()
            .map(|o| {
                let s = Source::parse(&o.from);
                let kind = if s.node == INPUTS { ins.iter().find(|i| Some(i.name.as_str()) == s.output).map(|i| i.kind) } else { massif_terrain::group::kind_of(scope, &o.from) };
                Port { name: o.name.clone(), label: label_or(&o.label, &o.name), kind: kind.unwrap_or(PortKind::Height), optional: false, default: String::new(), help: String::new() }
            })
            .collect();
        let left = NodeShape { label: t!("Inputs").to_owned(), summary: String::new(), family: Family::Input, inputs: Vec::new(), outputs: ins, body: Body::Inputs };
        let right = NodeShape { label: t!("Outputs").to_owned(), summary: String::new(), family: Family::Output, inputs: outs, outputs: Vec::new(), body: Body::Outputs };
        (left, right)
    }

    /// The shape of a node of `doc`'s type `kind`.
    pub fn of_kind(doc: &Graph, kind: &str) -> Option<Self> {
        if is_component_type(kind) {
            return Self::composite_of(doc, kind);
        }
        registry::spec(kind).map(Self::native)
    }
}

fn label_or(label: &str, name: &str) -> String {
    if label.is_empty() {
        let mut c = name.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect::<String>().replace(['_', '-'], " ")).unwrap_or_default()
    } else {
        super::text::name_owned(label)
    }
}

/// A step into an opened composite: the node opened, in the scope around
/// it, and its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub node: String,
    pub kind: String,
}

/// The nodes of the scope `crumbs` leads to.
pub fn scope_nodes<'a>(doc: &'a Graph, crumbs: &[Crumb]) -> Option<std::borrow::Cow<'a, BTreeMap<String, Node>>> {
    match crumbs.last() {
        None => Some(std::borrow::Cow::Borrowed(&doc.nodes)),
        Some(c) => component(doc, &c.kind).map(|c| std::borrow::Cow::Owned(c.into_owned().nodes)),
    }
}

/// The unfolded id of node `id` of the scope `crumbs` leads to (its
/// errors are reported under it).
pub fn unfolded_id(crumbs: &[Crumb], id: &str) -> String {
    let prefix: String = crumbs.iter().map(|c| format!("{}{SEP}", c.node)).collect();
    format!("{prefix}{id}")
}

/// The setting of the registry an exposed setting stands for in the end
/// (through composites inside composites), with the type of node it is a
/// setting of.
pub fn exposed_base(doc: &Graph, comp: &Component, e: &Exposed) -> Option<(&'static NodeSpec, &'static ParamSpec)> {
    exposed_base_in(doc, comp, e, 0)
}

fn exposed_base_in(doc: &Graph, comp: &Component, e: &Exposed, depth: usize) -> Option<(&'static NodeSpec, &'static ParamSpec)> {
    if depth > 16 {
        return None;
    }
    let (n, p) = e.targets.first()?.split_once('.')?;
    let node = comp.nodes.get(n)?;
    if is_component_type(&node.kind) {
        let inner = component(doc, &node.kind)?;
        let ie = inner.exposed.iter().find(|x| x.name == p)?.clone();
        return exposed_base_in(doc, &inner, &ie, depth + 1);
    }
    let spec = registry::spec(&node.kind)?;
    let (_, q) = spec.param(p)?;
    // `param` gives a borrow of the static spec's slice: find it again there.
    let q: &'static ParamSpec = spec.params.iter().find(|x| std::ptr::eq(*x, q))?;
    Some((spec, q))
}

/// An exposed setting as a setting of the registry: the one it stands
/// for, with the exposed one's name, words, range and unit. Made once
/// for each different exposed setting and kept.
pub fn exposed_spec(base: &'static ParamSpec, e: &Exposed) -> &'static ParamSpec {
    static MADE: OnceLock<Mutex<HashMap<String, &'static ParamSpec>>> = OnceLock::new();
    let key = format!("{:p}|{}", base, serde_json::to_string(e).unwrap_or_default());
    let mut made = MADE.get_or_init(Default::default).lock().expect("lock");
    if let Some(q) = made.get(&key) {
        return q;
    }
    let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let mut q = *base;
    q.name = leak(&e.name);
    if !e.label.is_empty() {
        q.label = leak(&e.label);
    }
    if !e.help.is_empty() {
        q.help = leak(&e.help);
    }
    q.section = registry::Section::Main;
    q.drivable = false;
    if let ParamKind::Float { min, max, soft, default, unit } = q.kind {
        let lo = e.min.unwrap_or(min);
        let hi = e.max.unwrap_or(max);
        let soft = e.soft.unwrap_or([soft[0].max(lo), soft[1].min(hi)]);
        let default = e.default.as_ref().and_then(|v| v.as_f64()).unwrap_or(default);
        let unit = e.unit.as_deref().and_then(unit_of).unwrap_or(unit);
        q.kind = ParamKind::Float { min: lo, max: hi, soft, default, unit };
    }
    let q: &'static ParamSpec = Box::leak(Box::new(q));
    made.insert(key, q);
    q
}

fn unit_of(id: &str) -> Option<registry::Unit> {
    use registry::Unit;
    [Unit::None, Unit::Blocks, Unit::Fraction, Unit::Degrees, Unit::Ratio, Unit::Count, Unit::Celsius, Unit::Percent].into_iter().find(|u| u.id() == id)
}
