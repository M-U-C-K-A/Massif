//! MassifScript: the terrain graph written as text.
//!
//! ```text
//! project "Alpine valley" {
//!     seed = 4268;
//!     group "Base" {
//!         color = #347a52;
//!         base = FractalNoise(scale: 2400, height: 600, style: "ridged");
//!         worn = StreamErosion(base, strength: 0.6);
//!     }
//!     output height = worn;
//! }
//! ```
//!
//! A script is not a generator of its own: it is run once, and what it
//! makes is the steps the editor makes ([`crate::format::Op`]: add a node,
//! set a setting, link, frame, expose…), replayed on an empty graph. The
//! result is a graph like any other, which opens in the editor; ids are the
//! script's names, so making it again from the same script gives the same
//! graph. [`decompile`] writes any graph back as a script.
//!
//! The API is shaped for tools (a future MCP server): [`compile`] gives the
//! graph or errors with line and column, [`decompile`] the text,
//! [`guide`] everything a model needs to write one, from the live catalogue.

mod compile;
mod decompile;
mod guide;
pub mod names;
pub mod syntax;

pub use compile::{FileLoader, Loader, NoImports};
pub use decompile::{decompile, number};
pub use guide::{EXAMPLES, guide, reference};

use crate::format::Op;
use crate::model::{Component, Graph, NodeUi, Terrain};
use std::collections::BTreeMap;

/// Where something is written: a file (0 is the script, then the files it
/// imports), bytes, and the line and column it starts at (from 1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub file: u32,
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

impl Span {
    /// From this one's start to `other`'s end.
    pub fn to(self, other: Span) -> Span {
        if other.file != self.file || other.end < self.start {
            return self;
        }
        Span { end: other.end, ..self }
    }

    /// Just after this one.
    pub fn after(self) -> Span {
        Span { start: self.end, end: self.end, col: self.col + (self.end - self.start) as u32, ..self }
    }
}

/// A mistake in a script, or a warning.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub span: Span,
    pub message: String,
    pub warning: bool,
    /// The file it is in, when it is not the script itself.
    pub file: Option<String>,
}

impl Error {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self { span, message: message.into(), warning: false, file: None }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self { warning: true, ..Self::new(span, message) }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}:")?;
        }
        write!(f, "{}:{}: {}{}", self.span.line, self.span.col, if self.warning { "warning: " } else { "" }, self.message)
    }
}

/// One step a script makes.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// An edit, as the editor makes it.
    Op(Op),
    /// A new composite of the document, empty: its label, category,
    /// summary (no edit step makes one from nothing yet).
    Define { kind: String, component: Component },
    /// Edits inside a composite ([`Op::Inside`]), with where each is written.
    Inside { component: String, ops: Vec<Op>, spans: Vec<Span> },
}

/// What a script makes: the terrain's settings and the steps.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub terrain: Terrain,
    /// The project's name.
    pub title: Option<String>,
    pub steps: Vec<(Step, Span)>,
    /// Notes on nodes (no edit step writes one): component, node, note.
    pub notes: Vec<(Option<String>, String, String)>,
}

impl Program {
    /// The steps as the edits of [`crate::format::patch`] (a composite's
    /// definition is not one of them yet).
    pub fn ops(&self) -> Vec<Op> {
        self.steps
            .iter()
            .filter_map(|(s, _)| match s {
                Step::Op(o) => Some(o.clone()),
                Step::Inside { component, ops, .. } => Some(Op::Inside { component: component.clone(), ops: ops.clone() }),
                Step::Define { .. } => None,
            })
            .collect()
    }

    /// The steps replayed on an empty graph.
    pub fn build(&self) -> Result<Graph, Error> {
        let mut g = Graph { terrain: self.terrain.clone(), ..Graph::default() };
        for (step, span) in &self.steps {
            match step {
                Step::Op(op) => {
                    crate::format::apply(&mut g, op).map_err(|e| Error::new(*span, e))?;
                }
                Step::Define { kind, component } => {
                    g.components.insert(kind.clone(), component.clone());
                }
                Step::Inside { component, ops, spans } => {
                    let op = Op::Inside { component: component.clone(), ops: ops.clone() };
                    crate::format::apply(&mut g, &op).map_err(|e| {
                        // “step k: …” names the step inside.
                        let at = e.strip_prefix("step ").and_then(|r| r.split_once(':')).and_then(|(k, _)| k.parse::<usize>().ok());
                        let msg = e.split_once(": ").map_or(e.as_str(), |(_, m)| m).to_owned();
                        Error::new(at.and_then(|k| spans.get(k).copied()).unwrap_or(*span), msg)
                    })?;
                }
            }
        }
        for (comp, node, note) in &self.notes {
            let nodes = match comp {
                Some(c) => g.components.get_mut(c).map(|c| &mut c.nodes),
                None => Some(&mut g.nodes),
            };
            if let Some(n) = nodes.and_then(|n| n.get_mut(node)) {
                n.note = Some(note.clone());
            }
        }
        Ok(g)
    }
}

/// What compiling a script gives.
#[derive(Debug, Clone)]
pub struct Output {
    /// The graph, laid out, when there was no error.
    pub graph: Option<Graph>,
    pub program: Option<Program>,
    /// Errors first, then warnings.
    pub errors: Vec<Error>,
    /// The files read: the script, then those it imports.
    pub files: Vec<String>,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.graph.is_some()
    }
}

/// Makes `src` into a graph, reading what it imports through `loader`.
/// The same script always gives the same graph.
pub fn compile(src: &str, loader: &mut dyn Loader) -> Output {
    compile_named("script", src, loader)
}

/// [`compile`], the script being file `name` (what imports are relative to).
pub fn compile_named(name: &str, src: &str, loader: &mut dyn Loader) -> Output {
    let mut c = compile::Compiler::new(loader);
    let r = c.run(name, src);
    let files: Vec<String> = c.files.iter().map(|f| f.0.clone()).collect();
    let name_of = |e: &mut Error| {
        if e.span.file > 0 {
            e.file = files.get(e.span.file as usize).cloned();
        }
    };
    let mut warnings = std::mem::take(&mut c.warnings);
    warnings.iter_mut().for_each(name_of);
    let fail = |mut e: Error, warnings: Vec<Error>| {
        name_of(&mut e);
        let mut errors = vec![e];
        errors.extend(warnings);
        Output { graph: None, program: None, errors, files: files.clone() }
    };
    if let Err(e) = r {
        return fail(e, warnings);
    }
    let program = match c.finish() {
        Ok(p) => p,
        Err(e) => return fail(e, warnings),
    };
    match program.build() {
        Ok(mut g) => {
            lay_out(&mut g, None);
            Output { graph: Some(g), program: Some(program), errors: warnings, files }
        }
        Err(e) => fail(e, warnings),
    }
}

/// Makes a script with no imports into a graph, or gives its first error.
pub fn build(src: &str) -> Result<Graph, Error> {
    let mut out = compile(src, &mut NoImports);
    out.graph.take().ok_or_else(|| out.errors.into_iter().next().expect("an error"))
}

// ---------------------------------------------------------------- layout --

const COLUMN: f32 = 280.0;
const GAP: f32 = 40.0;
const PAD: f32 = 36.0;

/// A node's height in the editor, roughly.
fn node_height(g: &Graph, id: &str) -> f32 {
    let n = &g.nodes[id];
    let ports = match crate::registry::spec(&n.kind) {
        Some(s) => s.inputs.len().max(n.inputs.len()) + s.outputs.len(),
        None => crate::lower::component(g, &n.kind).map_or(2, |c| c.inputs.len() + c.outputs.len()),
    };
    70.0 + 22.0 * ports as f32
}

/// Places the nodes left to right by depth, each frame a band of its own
/// under the last (nested frames inside theirs), and the frames round
/// their nodes; inside composites too. Nodes `keep` places already stay
/// where they are.
pub fn lay_out(g: &mut Graph, keep: Option<&Graph>) {
    lay_out_one(g, keep.map(|k| (&k.ui, &k.frames)));
    let kinds: Vec<String> = g.components.keys().cloned().collect();
    for k in kinds {
        let c = g.components[&k].clone();
        let mut inner = Graph { nodes: c.nodes, frames: c.frames, components: g.components.clone(), ..Graph::default() };
        let old = keep.and_then(|kg| kg.components.get(&k)).map(|oc| (&oc.ui, &oc.frames));
        lay_out_one(&mut inner, old);
        let c = g.components.get_mut(&k).expect("there");
        c.ui = inner.ui;
        c.frames = inner.frames;
    }
}

type Kept<'a> = Option<(&'a BTreeMap<String, NodeUi>, &'a BTreeMap<String, crate::model::Frame>)>;

fn lay_out_one(g: &mut Graph, keep: Kept) {
    // Depth from the sources.
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    fn depth_of(g: &Graph, id: &str, memo: &mut BTreeMap<String, usize>, seen: &mut Vec<String>) -> usize {
        if let Some(d) = memo.get(id) {
            return *d;
        }
        if seen.iter().any(|s| s == id) || !g.nodes.contains_key(id) {
            return 0;
        }
        seen.push(id.to_owned());
        let d = g.nodes[id].inputs.values().map(|s| crate::model::Source::parse(s).node).filter(|n| g.nodes.contains_key(*n)).map(|n| depth_of(g, n, memo, seen) + 1).max().unwrap_or(0);
        seen.pop();
        memo.insert(id.to_owned(), d);
        d
    }
    let ids: Vec<String> = g.nodes.keys().cloned().collect();
    for id in &ids {
        depth_of(g, id, &mut depth, &mut Vec::new());
    }
    // Each node's frame (the first holding it), and each frame's children.
    let mut home: BTreeMap<String, String> = BTreeMap::new();
    for (fid, f) in &g.frames {
        for m in &f.members {
            home.entry(m.clone()).or_insert_with(|| fid.clone());
        }
    }
    let kids = |parent: Option<&str>| -> Vec<String> { g.frames.iter().filter(|(_, f)| f.parent.as_deref() == parent).map(|(id, _)| id.clone()).collect() };
    let mut ui: BTreeMap<String, NodeUi> = BTreeMap::new();
    let mut rects: BTreeMap<String, [f32; 4]> = BTreeMap::new();
    // Lays out a band (a frame's own nodes, then its frames) from `y`;
    // gives where it ends.
    #[allow(clippy::too_many_arguments)]
    fn band(g: &Graph, frame: Option<&str>, y: f32, depth: &BTreeMap<String, usize>, home: &BTreeMap<String, String>, kids: &dyn Fn(Option<&str>) -> Vec<String>, ui: &mut BTreeMap<String, NodeUi>, rects: &mut BTreeMap<String, [f32; 4]>) -> f32 {
        let top = if frame.is_some() { y + PAD + 24.0 } else { y };
        let own: Vec<&String> = g.nodes.keys().filter(|id| home.get(*id).map(String::as_str) == frame).collect();
        let mut rows: BTreeMap<usize, f32> = BTreeMap::new();
        let mut bottom = top;
        for id in own {
            let d = depth[id];
            let row = rows.entry(d).or_insert(top);
            ui.insert(id.clone(), NodeUi { pos: [d as f32 * COLUMN, *row], collapsed: false });
            *row += node_height(g, id) + GAP;
            bottom = bottom.max(*row);
        }
        let mut y2 = bottom;
        for k in kids(frame) {
            y2 = band(g, Some(&k), y2, depth, home, kids, ui, rects) + GAP;
        }
        let end = if frame.is_some() { y2.max(top + 60.0) - GAP + PAD } else { y2 };
        if let Some(f) = frame {
            rects.insert(f.to_owned(), [0.0, y, 0.0, end - y]);
        }
        end
    }
    let mut y = 0.0;
    // Loose nodes first, then each top frame.
    let loose = band(g, None, 0.0, &depth, &home, &|p| if p.is_none() { Vec::new() } else { kids(p) }, &mut ui, &mut rects);
    y += loose + if loose > 0.0 { GAP * 2.0 } else { 0.0 };
    for k in kids(None) {
        y = band(g, Some(&k), y, &depth, &home, &kids, &mut ui, &mut rects) + GAP * 2.0;
    }
    // Kept places win.
    if let Some((old, _)) = keep {
        for (id, u) in old {
            if let Some(slot) = ui.get_mut(id) {
                *slot = u.clone();
            }
        }
    }
    // Frames round what they hold (their frames' too).
    fn bounds(g: &Graph, f: &str, ui: &BTreeMap<String, NodeUi>) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        let mut grow = |r: [f32; 4]| {
            b = Some(match b {
                None => r,
                Some(o) => [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])],
            })
        };
        for m in &g.frames[f].members {
            if let Some(u) = ui.get(m) {
                grow([u.pos[0], u.pos[1], u.pos[0] + 200.0, u.pos[1] + node_height(g, m)]);
            }
        }
        for (k, _) in g.frames.iter().filter(|(_, c)| c.parent.as_deref() == Some(f)) {
            if let Some(r) = bounds(g, k, ui) {
                grow([r[0] - PAD, r[1] - PAD - 24.0, r[2] + PAD, r[3] + PAD]);
            }
        }
        b
    }
    let fids: Vec<String> = g.frames.keys().cloned().collect();
    for f in fids {
        let r = bounds(g, &f, &ui).map(|b| [b[0] - PAD, b[1] - PAD - 24.0, b[2] - b[0] + 2.0 * PAD, b[3] - b[1] + 2.0 * PAD + 24.0]).or_else(|| rects.get(&f).copied()).unwrap_or_default();
        g.frames.get_mut(&f).expect("there").rect = r;
    }
    g.ui = ui;
}

#[cfg(test)]
mod tests;
