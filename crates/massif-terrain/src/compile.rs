//! A graph made ready to evaluate: checked, its nodes in order, their
//! settings decoded and each with the hash of everything its result
//! depends on.
//!
//! A graph with mistakes still compiles: what is wrong is said in
//! diagnostics with the path to it (`nodes.alps.params.height`), and the
//! nodes it breaks, with those after them, cannot be evaluated while the
//! rest can. This is what lets the editor show a graph being built, and a
//! model correct a graph from the messages.
//!
//! Where an output feeds a socket of another type, the compiled graph puts
//! the conversion between them as a node of its own (the dot Blender
//! draws on such a wire), shared by every input it feeds.

use crate::kernel::{Ctx, Env, NoEnv};
use crate::model::{Graph, Source, valid_id};
use crate::params::Params;
use crate::registry::{self, Conversion, Kernel, NodeSpec, ParamKind, PortKind, PortSpec, Reads};
use crate::space::{Rect, Space};
use crate::util::{Fnv, closest, mix32};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::Hasher;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Something wrong in a graph, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Where, as a path into the JSON: `nodes.alps.params.height`.
    pub path: String,
    pub message: String,
}

impl Diagnostic {
    pub fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, path: path.into(), message: message.into() }
    }

    pub fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, path: path.into(), message: message.into() }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{what} at {}: {}", self.path, self.message)
    }
}

/// A node ready to evaluate.
#[derive(Debug)]
pub struct CNode {
    /// Its name in the graph (conversions the compiled graph added are
    /// named `#source.output:conversion`).
    pub id: String,
    pub spec: &'static NodeSpec,
    pub params: Params,
    /// What feeds each of its type's inputs: a node (by index) and its output.
    pub inputs: Vec<Option<(usize, u8)>>,
    /// Its open family of inputs (an output's named masks), by name.
    pub open: Vec<(String, (usize, u8))>,
    /// For each setting (by index), the field driving it (input `@name`).
    pub driven: Vec<Option<(usize, u8)>>,
    /// Everything the node's results depend on, hashed: its type, settings,
    /// seed, inputs' hashes, the terrain.
    pub hash: u64,
    /// The seed its kernel works with.
    pub seed: u32,
    /// Why it cannot be evaluated.
    pub broken: Option<String>,
}

impl CNode {
    pub fn is_global(&self) -> bool {
        matches!(self.spec.kernel, Kernel::Global { .. })
    }

    /// Whether the compiled graph added it.
    pub fn is_internal(&self) -> bool {
        self.id.starts_with('#')
    }
}

/// What the map takes from the graph: the active Map Output's inputs that
/// are connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOutput {
    /// The Map Output node.
    pub node: usize,
    /// Each connected input: its name (`height`, `mask.snow`…), its type,
    /// and the node output feeding it.
    pub inputs: Vec<(String, PortKind, (usize, u8))>,
}

impl MapOutput {
    pub fn get(&self, name: &str) -> Option<(usize, u8)> {
        self.inputs.iter().find(|i| i.0 == name).map(|i| i.2)
    }
}

/// A graph ready to evaluate.
#[derive(Debug)]
pub struct Compiled {
    pub space: Space,
    /// Samples a side the simulations run at, at most.
    pub sim_res: u32,
    /// In an order where every node comes after its inputs.
    pub nodes: Vec<CNode>,
    index: HashMap<String, usize>,
    output: Option<MapOutput>,
    env: Arc<dyn Env>,
}

impl std::fmt::Debug for dyn Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Env")
    }
}

impl Compiled {
    /// The context of node `n`'s kernel making `rect` at `lod`, with the
    /// fields driving its settings and where its inputs were read.
    pub fn ctx<'a>(&'a self, n: usize, lod: u8, rect: Rect, driven: &'a [Option<&'a [f32]>], reads: &'a [Option<(u8, Rect)>]) -> Ctx<'a> {
        let node = &self.nodes[n];
        Ctx { space: &self.space, lod, rect, seed: node.seed, params: &node.params, driven, reads, env: &*self.env }
    }

    /// The world outside the graph it was compiled with.
    pub fn env(&self) -> &dyn Env {
        &*self.env
    }

    pub fn node(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// Output `output` (the first when `None`) of node `id`; for a Map
    /// Output, what feeds its Height.
    pub fn source(&self, id: &str, output: Option<&str>) -> Option<(usize, u8)> {
        let n = self.node(id)?;
        let node = &self.nodes[n];
        if node.spec.is_output() {
            return node.inputs.first().copied().flatten();
        }
        match output {
            None => (!node.spec.outputs.is_empty()).then_some((n, 0)),
            Some(name) => node.spec.output(name).map(|k| (n, k as u8)),
        }
    }

    /// The active Map Output, with what is connected to it.
    pub fn map_output(&self) -> Option<&MapOutput> {
        self.output.as_ref()
    }

    /// What feeds the active Map Output's input `name` (`height`,
    /// `albedo`, `mask.snow`…).
    pub fn output(&self, name: &str) -> Option<(usize, u8)> {
        self.output.as_ref().and_then(|o| o.get(name))
    }

    /// The type of what an output of a node carries.
    pub fn kind(&self, (n, port): (usize, u8)) -> PortKind {
        self.nodes[n].spec.outputs[port as usize].kind
    }

    /// The level of detail simulations run at.
    pub fn sim_lod(&self) -> u8 {
        self.space.lod_for(self.sim_res)
    }
}

/// An input as written: the node and output it comes from, and what goes
/// between.
#[derive(Debug, Clone)]
struct Wire {
    from: String,
    port: u8,
    conversion: Conversion,
}

/// Checks and orders `graph` over `space` (whose size, seed and sea level
/// the graph's own `terrain` settings override), simulations running at
/// up to `sim_res` samples a side, with no map and no files.
pub fn compile(graph: &Graph, space: Space, sim_res: u32) -> (Compiled, Vec<Diagnostic>) {
    compile_in(graph, space, sim_res, Arc::new(NoEnv))
}

/// [`compile`] in a world: the map and files `env` gives.
pub fn compile_in(graph: &Graph, mut space: Space, sim_res: u32, env: Arc<dyn Env>) -> (Compiled, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    if graph.format != crate::model::FORMAT {
        diags.push(Diagnostic::warning("format", format!("expected “{}”", crate::model::FORMAT)));
    }
    // A document of a later format is not half-understood: nothing of it is
    // evaluated (and it is saved back as it was).
    let newer = graph.version > crate::model::VERSION;
    if newer {
        diags.push(Diagnostic::error("version", format!("made with a newer Massif (format version {}); this one reads up to {}", graph.version, crate::model::VERSION)));
    }
    if let Some(size) = graph.terrain.size {
        space.size = size.clamp(16, 1 << 17);
    }
    space.seed = graph.terrain.seed;
    if let Some(sea) = graph.terrain.sea_level {
        space.sea_level = sea;
    }

    // Each node's type, settings and inputs.
    struct Pending {
        spec: Option<&'static NodeSpec>,
        params: Option<Params>,
        inputs: Vec<Option<Wire>>,
        open: Vec<(String, Wire)>,
        driven: Vec<Option<Wire>>,
        broken: Option<String>,
    }
    let ids: Vec<&str> = graph.nodes.keys().map(String::as_str).collect();
    let mut pending: BTreeMap<&str, Pending> = BTreeMap::new();
    for (id, node) in &graph.nodes {
        let path = format!("nodes.{id}");
        let mut broken = None;
        if !valid_id(id) {
            diags.push(Diagnostic::error(path.clone(), "a node's name is made of letters, digits, “_” and “-”"));
        }
        let spec = registry::spec(&node.kind).filter(|s| s.family != registry::Family::Internal);
        if spec.is_none() {
            let base = node.kind.split('@').next().unwrap_or("");
            if let Some(known) = registry::latest(base).filter(|s| s.family != registry::Family::Internal) {
                diags.push(Diagnostic::error(format!("{path}.type"), format!("“{}” is a version of {} newer than this Massif knows (up to {}): it was made with a newer Massif", node.kind, known.id, known.version)));
            } else {
                let hint = closest(&node.kind, registry::all().iter().filter(|s| s.family != registry::Family::Internal).map(|s| s.id)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                diags.push(Diagnostic::error(format!("{path}.type"), format!("no type of node is called “{}”{hint}", node.kind)));
            }
            broken = Some(format!("its type “{}” is unknown", node.kind));
        }
        if newer {
            broken = Some("the graph was made with a newer Massif".to_owned());
        }
        let params = spec.map(|s| Params::decode(s, id, &node.params, &format!("{path}.params"), &mut diags));
        let mut inputs: Vec<Option<Wire>> = vec![None; spec.map_or(0, |s| s.inputs.len())];
        let mut open = Vec::new();
        let mut driven: Vec<Option<Wire>> = vec![None; spec.map_or(0, |s| s.params.len())];
        if let Some(spec) = spec {
            for (input, source) in &node.inputs {
                let here = format!("{path}.inputs.{input}");
                // `@name`: a field driving setting `name`.
                let drives = match input.strip_prefix('@') {
                    None => None,
                    Some(param) => match spec.param(param) {
                        Some((i, p)) if p.drivable => Some(i),
                        Some(_) => {
                            diags.push(Diagnostic::error(here, format!("{}'s setting “{param}” cannot be driven by a field", spec.id)));
                            continue;
                        }
                        None => {
                            let can: Vec<&str> = spec.params.iter().filter(|p| p.drivable).map(|p| p.name).collect();
                            let hint = closest(param, can.iter().copied()).map(|c| format!(" (did you mean “@{c}”?)")).unwrap_or_default();
                            diags.push(Diagnostic::error(here, format!("{} has no setting “{param}” a field can drive{hint}", spec.id)));
                            continue;
                        }
                    },
                };
                const DRIVE: PortSpec = PortSpec::new("@", "Field", PortKind::Mask, "");
                let (slot, socket): (Option<usize>, &PortSpec) = match (drives, spec.input(input)) {
                    (Some(_), _) => (None, &DRIVE),
                    (None, Some(k)) => (Some(k), &spec.inputs[k]),
                    (None, None) => match spec.open_input(input) {
                        Some(o) => (None, o),
                        None => {
                            let names = spec.inputs.iter().map(|p| p.name);
                            let hint = closest(input, names.clone()).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                            let mut list: Vec<String> = names.map(str::to_owned).collect();
                            if let Some(o) = &spec.open_inputs {
                                list.push(format!("{}<name>", o.name));
                            }
                            let has = if list.is_empty() { "no inputs".to_owned() } else { format!("the inputs {}", list.join(", ")) };
                            diags.push(Diagnostic::error(here, format!("{} has {has}, not “{input}”{hint}", spec.id)));
                            continue;
                        }
                    },
                };
                let src = Source::parse(source);
                let Some(from) = graph.nodes.get(src.node) else {
                    let hint = closest(src.node, ids.iter().copied()).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                    diags.push(Diagnostic::error(here, format!("no node is called “{}”{hint}", src.node)));
                    continue;
                };
                let Some(from_spec) = registry::spec(&from.kind) else {
                    // Its own diagnostic says why; what it feeds is broken.
                    let w = Wire { from: src.node.to_owned(), port: 0, conversion: Conversion::AsIs };
                    match (drives, slot) {
                        (Some(i), _) => driven[i] = Some(w),
                        (None, Some(k)) => inputs[k] = Some(w),
                        (None, None) => open.push((input.clone(), w)),
                    }
                    continue;
                };
                let port = match src.output {
                    None if from_spec.outputs.is_empty() => {
                        diags.push(Diagnostic::error(here, format!("“{}” gives nothing to other nodes", src.node)));
                        continue;
                    }
                    None => 0,
                    Some(name) => match from_spec.output(name) {
                        Some(p) => p,
                        None => {
                            let outs = from_spec.outputs.iter().map(|p| p.name).collect::<Vec<_>>().join(", ");
                            diags.push(Diagnostic::error(here, format!("“{}” ({}) gives {outs}, not “{name}”", src.node, from_spec.id)));
                            continue;
                        }
                    },
                };
                let from_kind = from_spec.outputs[port].kind;
                let Some(conversion) = from_kind.conversion(socket.kind) else {
                    diags.push(Diagnostic::error(
                        here,
                        format!("“{source}” gives a {}, which “{input}” (a {}) cannot take: a node must say how to turn one into the other", from_kind.id(), socket.kind.id()),
                    ));
                    continue;
                };
                let w = Wire { from: src.node.to_owned(), port: port as u8, conversion };
                match (drives, slot) {
                    (Some(i), _) => driven[i] = Some(w),
                    (None, Some(k)) => inputs[k] = Some(w),
                    (None, None) => open.push((input.clone(), w)),
                }
            }
            for (k, p) in spec.inputs.iter().enumerate() {
                if inputs[k].is_none() && !p.optional && broken.is_none() {
                    diags.push(Diagnostic::error(format!("{path}.inputs"), format!("needs its input “{}”", p.name)));
                    broken = Some(format!("needs its input “{}”", p.name));
                }
            }
        }
        pending.insert(id.as_str(), Pending { spec, params, inputs, open, driven, broken });
    }
    for id in graph.ui.keys() {
        if !graph.nodes.contains_key(id) {
            diags.push(Diagnostic::warning(format!("ui.{id}"), "lays out a node the graph does not have"));
        }
    }

    // Order: every node after its inputs (Kahn's, by name among equals, so
    // that the order is the same each time).
    let mut waiting: BTreeMap<&str, usize> = BTreeMap::new();
    let mut users: HashMap<&str, Vec<&str>> = HashMap::new();
    for (id, p) in &pending {
        let mut n = 0;
        for w in p.inputs.iter().flatten().chain(p.open.iter().map(|o| &o.1)).chain(p.driven.iter().flatten()) {
            if pending.contains_key(w.from.as_str()) {
                n += 1;
                users.entry(w.from.as_str()).or_default().push(id);
            }
        }
        waiting.insert(id, n);
    }
    let mut ready: BTreeSet<&str> = waiting.iter().filter(|(_, n)| **n == 0).map(|(id, _)| *id).collect();
    let mut order: Vec<&str> = Vec::with_capacity(pending.len());
    while let Some(id) = ready.pop_first() {
        order.push(id);
        for u in users.get(id).into_iter().flatten() {
            let n = waiting.get_mut(u).expect("users are nodes");
            *n -= 1;
            if *n == 0 {
                ready.insert(u);
            }
        }
    }
    for (id, n) in &waiting {
        if *n > 0 {
            diags.push(Diagnostic::error(format!("nodes.{id}.inputs"), "the graph loops back on itself through this node"));
            order.push(id);
            pending.get_mut(id).expect("waiting nodes are pending").broken = Some("the graph loops back on itself through it".to_owned());
        }
    }

    // Built in order, with hashes and the conversions between them.
    let mut b = Builder { space, sim_res, env: env.clone(), nodes: Vec::with_capacity(order.len()), index: HashMap::new(), terrain_seed: mix32((space.seed as u32) ^ mix32((space.seed >> 32) as u32)) };
    for id in order {
        let p = pending.remove(id).expect("ordered nodes are pending");
        let Some(spec) = p.spec else {
            // An unknown type: kept out, what uses it is broken.
            continue;
        };
        let mut broken = p.broken;
        let mut resolve = |name: &str, w: &Wire, broken: &mut Option<String>| -> Option<(usize, u8)> {
            let Some(&n) = b.index.get(w.from.as_str()) else {
                if broken.is_none() {
                    *broken = Some(format!("its input “{name}” comes from “{}”, which cannot be evaluated", w.from));
                }
                return None;
            };
            if b.nodes[n].broken.is_some() && broken.is_none() {
                *broken = Some(format!("its input “{name}” comes from “{}”, which cannot be evaluated", w.from));
            }
            Some(b.convert((n, w.port), w.conversion))
        };
        let inputs: Vec<Option<(usize, u8)>> = p.inputs.iter().enumerate().map(|(k, w)| w.as_ref().and_then(|w| resolve(spec.inputs[k].name, w, &mut broken))).collect();
        let open: Vec<(String, (usize, u8))> = p.open.iter().filter_map(|(name, w)| resolve(name, w, &mut broken).map(|s| (name.clone(), s))).collect();
        let driven: Vec<Option<(usize, u8)>> = p.driven.iter().enumerate().map(|(i, w)| w.as_ref().and_then(|w| resolve(&format!("@{}", spec.params[i].name), w, &mut broken))).collect();
        let params = p.params.expect("known types have settings");
        b.push(id.to_owned(), spec, params, Links { inputs, open, driven }, broken);
    }

    // The active Map Output.
    let mut outputs: Vec<&CNode> = b.nodes.iter().filter(|n| n.spec.id == "output.map").collect();
    outputs.sort_by(|a, b| a.id.cmp(&b.id));
    let active: Vec<&CNode> = outputs.iter().copied().filter(|n| n.params.flag("active")).collect();
    if active.len() > 1 {
        diags.push(Diagnostic::warning(format!("nodes.{}.params.active", active[1].id), format!("the graph has several active Map Outputs; “{}”, the first by name, is used", active[0].id)));
    }
    let chosen = active.first().or(outputs.first()).map(|n| n.id.clone());
    let output = chosen.map(|id| {
        let n = b.index[&id];
        let node = &b.nodes[n];
        let mut inputs: Vec<(String, PortKind, (usize, u8))> = Vec::new();
        for (k, src) in node.inputs.iter().enumerate() {
            if let Some(src) = src {
                inputs.push((node.spec.inputs[k].name.to_owned(), node.spec.inputs[k].kind, *src));
            }
        }
        let open_kind = node.spec.open_inputs.map_or(PortKind::Mask, |o| o.kind);
        for (name, src) in &node.open {
            inputs.push((name.clone(), open_kind, *src));
        }
        MapOutput { node: n, inputs }
    });
    let Builder { space, nodes, index, .. } = b;
    (Compiled { space, sim_res: sim_res.max(16), nodes, index, output, env }, diags)
}

/// What feeds a node.
struct Links {
    inputs: Vec<Option<(usize, u8)>>,
    open: Vec<(String, (usize, u8))>,
    driven: Vec<Option<(usize, u8)>>,
}

struct Builder {
    space: Space,
    sim_res: u32,
    env: Arc<dyn Env>,
    nodes: Vec<CNode>,
    index: HashMap<String, usize>,
    terrain_seed: u32,
}

impl Builder {
    fn push(&mut self, id: String, spec: &'static NodeSpec, params: Params, links: Links, broken: Option<String>) -> usize {
        let Links { inputs, open, driven } = links;
        let seed = mix32(params.seed().unwrap_or(0) ^ self.terrain_seed);
        let mut h = Fnv::default();
        h.write(spec.id.as_bytes());
        params.hash_into(&mut h);
        h.write_u32(seed);
        let feeds: Vec<Option<(usize, u8)>> = inputs.iter().copied().chain(open.iter().map(|o| Some(o.1))).chain(driven.iter().copied()).collect();
        for i in &feeds {
            match i {
                Some((n, port)) => {
                    h.write_u64(self.nodes[*n].hash);
                    h.write_u8(*port);
                }
                None => h.write_u8(0xff),
            }
        }
        h.write_u32(self.space.size);
        h.write_u32(self.space.sea_level.to_bits());
        if matches!(spec.kernel, Kernel::Global { .. } | Kernel::Stats { .. }) {
            h.write_u32(self.sim_res);
        }
        match spec.reads {
            Reads::Nothing => {}
            Reads::Map => {
                h.write_u64(self.env.map_revision());
                h.write_i32(self.space.x0);
                h.write_i32(self.space.z0);
            }
            Reads::Files => {
                for (p, v) in spec.params.iter().zip(params.values()) {
                    if let (ParamKind::File { .. }, crate::params::Value::Text(path)) = (p.kind, v) {
                        h.write_u64(self.env.file_stamp(path));
                    }
                }
            }
        }
        let n = self.nodes.len();
        self.index.insert(id.clone(), n);
        let driven = if driven.iter().any(Option::is_some) { driven } else { Vec::new() };
        self.nodes.push(CNode { id, spec, params, inputs, open, driven, hash: h.finish(), seed, broken });
        n
    }

    /// `src` through `conversion`: itself, or the conversion node fed by it
    /// (made once, shared by all it feeds).
    fn convert(&mut self, src: (usize, u8), conversion: Conversion) -> (usize, u8) {
        let kind = match conversion {
            Conversion::AsIs => return src,
            Conversion::Luminance => "convert.luminance",
            Conversion::Grey => "convert.grey",
        };
        let id = format!("#{}.{}:{kind}", self.nodes[src.0].id, src.1);
        if let Some(&n) = self.index.get(&id) {
            return (n, 0);
        }
        let spec = registry::spec(kind).expect("conversions are registered");
        let broken = self.nodes[src.0].broken.as_ref().map(|_| format!("it converts “{}”, which cannot be evaluated", self.nodes[src.0].id));
        let n = self.push(id.clone(), spec, Params::defaults(spec, &id), Links { inputs: vec![Some(src)], open: Vec::new(), driven: Vec::new() }, broken);
        (n, 0)
    }
}
