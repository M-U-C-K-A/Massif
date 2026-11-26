//! A script made into a graph: evaluated at once (constants, loops and
//! conditions only decide what is built), each node, link, setting, frame
//! and composite becoming a step of [`crate::format::Op`], the steps the
//! editor and models make. The graph is those steps replayed.

use super::names::{self, Target, snake};
use super::syntax::{Arg, Attrs, Call, Expr, Param, Stmt};
use super::{Error, Program, Span, Step};
use crate::format::Op;
use crate::model::{Component, Exposed, Graph, INPUTS, Node, OutputDecl, PortDecl, Source, is_component_type, valid_id};
use crate::registry::{self, Family, NodeSpec, ParamKind, PortKind};
use crate::util::closest;
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A value while the script runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Bool(bool),
    Str(String),
    List(Vec<Val>),
    Obj(BTreeMap<String, Val>),
    /// An output of a node: `node` or `node.output` (`in.name` inside a
    /// component).
    Node(String),
    /// A setting of the component being written, standing for what its
    /// instances give.
    Setting(String),
}

impl Val {
    fn what(&self) -> &'static str {
        match self {
            Val::Num(_) => "a number",
            Val::Bool(_) => "true or false",
            Val::Str(_) => "a text",
            Val::List(_) => "a list",
            Val::Obj(_) => "an object",
            Val::Node(_) => "a node",
            Val::Setting(_) => "a setting of the component",
        }
    }
}

/// A number as JSON: whole numbers as integers.
pub fn num_json(v: f64) -> Json {
    if v.fract() == 0.0 && v.abs() < 9.0e15 { json!(v as i64) } else { serde_json::Number::from_f64(v).map_or(Json::Null, Json::Number) }
}

fn to_json(v: &Val) -> Option<Json> {
    Some(match v {
        Val::Num(n) => num_json(*n),
        Val::Bool(b) => json!(b),
        Val::Str(s) => json!(s),
        Val::List(l) => Json::Array(l.iter().map(to_json).collect::<Option<_>>()?),
        Val::Obj(o) => Json::Object(o.iter().map(|(k, v)| Some((k.clone(), to_json(v)?))).collect::<Option<Map<_, _>>>()?),
        Val::Node(_) | Val::Setting(_) => return None,
    })
}

/// The value types a component's setting can have.
const VALUE_TYPES: &[&str] = &["float", "int", "number", "bool", "string", "seed", "choice", "value"];

/// Where nodes are being made: the graph, or a component of it.
struct Scope {
    /// The component's type, or `None` for the graph.
    component: Option<String>,
    nodes: BTreeMap<String, Node>,
    /// Nodes in the order they were made.
    order: Vec<String>,
    ops: Vec<(Op, Span)>,
    /// Frames by id, in the order they were opened, with their members.
    frames: Vec<(String, FrameDraft)>,
    open: Vec<String>,
    /// The component's inputs and settings.
    inputs: Vec<PortDecl>,
    settings: Vec<SettingDraft>,
    outputs: Vec<(OutputDecl, Span)>,
    /// Notes on nodes, which no step writes.
    notes: Vec<(String, String)>,
    out_node: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct FrameDraft {
    title: String,
    color: Option<String>,
    description: String,
    parent: Option<String>,
    members: Vec<String>,
    span: Span,
}

struct SettingDraft {
    name: String,
    default: Option<Val>,
    exposed: Exposed,
    span: Span,
}

impl Scope {
    fn new(component: Option<String>) -> Self {
        Self { component, nodes: BTreeMap::new(), order: Vec::new(), ops: Vec::new(), frames: Vec::new(), open: Vec::new(), inputs: Vec::new(), settings: Vec::new(), outputs: Vec::new(), notes: Vec::new(), out_node: None }
    }
}

/// Reads the files a script imports: a path as the script writes it,
/// relative to the file importing it (`from`, `None` for the script),
/// gives the file's name and text.
pub trait Loader {
    fn load(&mut self, path: &str, from: Option<&str>) -> Result<(String, String), String>;
}

/// Imports nothing.
pub struct NoImports;

impl Loader for NoImports {
    fn load(&mut self, path: &str, _: Option<&str>) -> Result<(String, String), String> {
        Err(format!("“{path}” cannot be imported here: save the script first, beside it"))
    }
}

/// Imports from a list of files, by name (tests, the CLI's in-memory use).
impl Loader for BTreeMap<String, String> {
    fn load(&mut self, path: &str, _: Option<&str>) -> Result<(String, String), String> {
        self.get(path).map(|t| (path.to_owned(), t.clone())).ok_or_else(|| format!("there is no file “{path}”"))
    }
}

/// Imports from the disk, relative to the importing file.
pub struct FileLoader {
    /// The folder of the script itself.
    pub dir: Option<std::path::PathBuf>,
}

impl Loader for FileLoader {
    fn load(&mut self, path: &str, from: Option<&str>) -> Result<(String, String), String> {
        let base = match from {
            Some(f) => std::path::Path::new(f).parent().map(|p| p.to_path_buf()),
            None => self.dir.clone(),
        };
        let full = match base {
            Some(b) => b.join(path),
            None => std::path::PathBuf::from(path),
        };
        let text = std::fs::read_to_string(&full).map_err(|e| format!("“{}” cannot be read: {e}", full.display()))?;
        Ok((full.display().to_string(), text))
    }
}

pub struct Compiler<'l> {
    loader: &'l mut dyn Loader,
    pub files: Vec<(String, String)>,
    imported: BTreeSet<String>,
    importing: Vec<String>,
    /// Constants and variables, innermost last.
    vars: Vec<HashMap<String, (Val, bool)>>,
    scopes: Vec<Scope>,
    terrain: crate::model::Terrain,
    pub title: Option<String>,
    /// The document's components, as declared, by type.
    components: BTreeMap<String, Component>,
    /// Steps defining each component, in order.
    steps: Vec<(Step, Span)>,
    pub warnings: Vec<Error>,
    /// Notes on nodes: the component (`None`: the graph), node, note.
    notes: Vec<(Option<String>, String, String)>,
    budget: u64,
}

type R<T> = Result<T, Error>;

fn err<T>(span: Span, msg: impl Into<String>) -> R<T> {
    Err(Error::new(span, msg))
}

fn hint(name: &str, names: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let names: Vec<String> = names.into_iter().map(|n| n.as_ref().to_owned()).collect();
    closest(name, names.iter().map(String::as_str)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default()
}

/// The spec of a registry type `kind` at `version`.
fn spec_of(kind: &str) -> Option<&'static NodeSpec> {
    registry::spec(kind).filter(|s| s.family != Family::Internal)
}

/// What a socket carries, in words.
fn kind_words(k: PortKind) -> String {
    let id = k.id();
    let a = if matches!(id.as_bytes()[0], b'a' | b'e' | b'i' | b'o' | b'u') { "an" } else { "a" };
    format!("{a} {id}")
}

impl<'l> Compiler<'l> {
    pub fn new(loader: &'l mut dyn Loader) -> Self {
        Self { loader, files: Vec::new(), imported: BTreeSet::new(), importing: Vec::new(), vars: vec![HashMap::new()], scopes: vec![Scope::new(None)], terrain: Default::default(), title: None, components: BTreeMap::new(), steps: Vec::new(), warnings: Vec::new(), notes: Vec::new(), budget: 0 }
    }

    pub fn run(&mut self, name: &str, src: &str) -> R<()> {
        let file = self.files.len() as u32;
        self.files.push((name.to_owned(), src.to_owned()));
        let stmts = super::syntax::parse(src, file)?;
        self.block(&stmts)
    }

    /// The steps made, the graph's last.
    pub fn finish(mut self) -> R<Program> {
        let top = self.scopes.pop().expect("the graph's scope");
        let mut steps = std::mem::take(&mut self.steps);
        steps.extend(top.ops.iter().cloned().map(|(o, s)| (Step::Op(o), s)));
        steps.extend(Self::frame_ops(&top).into_iter().map(|(o, s)| (Step::Op(o), s)));
        let mut notes = std::mem::take(&mut self.notes);
        notes.extend(top.notes.into_iter().map(|(n, t)| (None, n, t)));
        Ok(Program { terrain: self.terrain, title: self.title, steps, notes })
    }

    fn scope(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("a scope")
    }

    fn tick(&mut self, span: Span) -> R<()> {
        self.budget += 1;
        if self.budget > 200_000 {
            return err(span, "the script runs too long: does a loop never end?");
        }
        Ok(())
    }

    // ------------------------------------------------------- statements --

    fn block(&mut self, stmts: &[Stmt]) -> R<()> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn scoped(&mut self, stmts: &[Stmt]) -> R<()> {
        self.vars.push(HashMap::new());
        let r = self.block(stmts);
        self.vars.pop();
        r
    }

    fn lookup_var(&self, name: &str) -> Option<&(Val, bool)> {
        self.vars.iter().rev().find_map(|v| v.get(name))
    }

    fn stmt(&mut self, s: &Stmt) -> R<()> {
        match s {
            Stmt::Project { name, body, span } => {
                if self.title.is_some() || self.scopes.len() > 1 {
                    return err(*span, "a script holds one project, at its top");
                }
                self.title = Some(name.clone());
                self.scoped(body)
            }
            Stmt::Group { id, title, body, span } => self.group(id.as_deref(), title, body, *span),
            Stmt::Assign { name, index, value, attrs, span } => self.assign(name, index.as_ref(), value, attrs, *span),
            Stmt::Const { name, value, span } | Stmt::Let { name, value, span } => {
                let v = self.eval(value)?;
                let mutable = matches!(s, Stmt::Let { .. });
                let top = self.vars.last_mut().expect("vars");
                if top.contains_key(name) {
                    return err(*span, format!("“{name}” is already defined here"));
                }
                top.insert(name.clone(), (v, mutable));
                Ok(())
            }
            Stmt::Output { name, value, attrs, span } => self.output(name, value.as_ref(), attrs, *span),
            Stmt::Component { name, params, body, span } => self.component(name, params, body, *span),
            Stmt::Expose { name, attrs, span } => self.expose(name, attrs, *span),
            Stmt::If { cond, then, other, span } => {
                self.tick(*span)?;
                match self.eval(cond)? {
                    Val::Bool(true) => self.scoped(then),
                    Val::Bool(false) => self.scoped(other),
                    v => err(cond.span(), format!("a condition is true or false, not {}", v.what())),
                }
            }
            Stmt::For { var, iter, body, span } => {
                let items = match self.eval(iter)? {
                    Val::List(l) => l,
                    v => return err(iter.span(), format!("a loop goes over a range (0..4) or a list, not {}", v.what())),
                };
                for item in items {
                    self.tick(*span)?;
                    self.vars.push(HashMap::from([(var.clone(), (item, false))]));
                    let r = self.block(body);
                    self.vars.pop();
                    r?;
                }
                Ok(())
            }
            Stmt::Import { path, span } => {
                let from = self.files.get(span.file as usize).map(|f| f.0.clone());
                let (name, text) = self.loader.load(path, from.as_deref()).map_err(|e| Error::new(*span, e))?;
                if self.importing.contains(&name) {
                    return err(*span, format!("“{path}” imports itself, through the files it imports"));
                }
                if !self.imported.insert(name.clone()) {
                    return Ok(());
                }
                self.importing.push(name.clone());
                let r = self.run(&name, &text);
                self.importing.pop();
                r
            }
        }
    }

    fn group(&mut self, id: Option<&str>, title: &Expr, body: &[Stmt], span: Span) -> R<()> {
        let title = match self.eval(title)? {
            Val::Str(s) => s,
            v => return err(title.span(), format!("a group's title is a text, not {}", v.what())),
        };
        let parent = self.scope().open.last().cloned();
        let sc = self.scope();
        let id = match id {
            Some(id) => {
                if !valid_id(id) {
                    return err(span, format!("“{id}” cannot name a group: letters, digits, “_” and “-”"));
                }
                id.to_owned()
            }
            None => {
                let stem = snake(&title.replace(|c: char| !c.is_ascii_alphanumeric(), " ").to_ascii_lowercase());
                let stem = if stem.is_empty() { "group".to_owned() } else { stem };
                let mut id = stem.clone();
                let mut k = 2;
                while sc.frames.iter().any(|(f, _)| *f == id) {
                    id = format!("{stem}_{k}");
                    k += 1;
                }
                id
            }
        };
        match sc.frames.iter_mut().find(|(f, _)| *f == id) {
            // Written again: the same frame, more nodes in it.
            Some((_, f)) => {
                if f.parent != parent {
                    return err(span, format!("group “{id}” sits in another group where it was first written"));
                }
                f.title = title;
            }
            None => sc.frames.push((id.clone(), FrameDraft { title, parent, span, ..Default::default() })),
        }
        sc.open.push(id);
        self.vars.push(HashMap::new());
        let r = self.block(body);
        self.vars.pop();
        self.scope().open.pop();
        r
    }

    /// `name = value;`: a node, a group's property, or a variable again.
    fn assign(&mut self, name: &str, index: Option<&Expr>, value: &Expr, attrs: &Attrs, span: Span) -> R<()> {
        if index.is_none() {
            // A component's setting may share its name with a node.
            if let Some((_, mutable)) = self.lookup_var(name).cloned().filter(|(v, _)| !matches!(v, Val::Setting(_))) {
                if !mutable {
                    return err(span, format!("“{name}” is a constant: it cannot change"));
                }
                let v = self.eval(value)?;
                for scope in self.vars.iter_mut().rev() {
                    if let Some(slot) = scope.get_mut(name) {
                        slot.0 = v;
                        break;
                    }
                }
                return Ok(());
            }
            if !matches!(value, Expr::Call(_)) && self.property(name, value, span)? {
                return Ok(());
            }
        }
        let id = match index {
            Some(i) => match self.eval(i)? {
                Val::Num(n) if n.fract() == 0.0 => format!("{name}_{}", n as i64),
                Val::Str(s) => format!("{name}_{s}"),
                v => return err(i.span(), format!("an index is a whole number or a text, not {}", v.what())),
            },
            None => name.to_owned(),
        };
        let made = match value {
            Expr::Call(call) => self.call(call, Some(&id))?,
            other => match self.eval(other)? {
                Val::Node(_) => return err(span, format!("“{id}” would be another name for a node: give it a node of its own, or use the node by its name")),
                v => return err(span, format!("“{name}” is not a property here: write “let {name} = …;” to keep {} in a variable", v.what())),
            },
        };
        let node = Source::parse(&made).node.to_owned();
        self.attrs(&node, attrs)
    }

    /// The properties of a project, a group and a component.
    fn property(&mut self, name: &str, value: &Expr, span: Span) -> R<bool> {
        let in_group = !self.scope().open.is_empty();
        let in_component = self.scope().component.is_some();
        let v = self.eval(value)?;
        let text = |v: &Val| match v {
            Val::Str(s) => Ok(s.clone()),
            v => err(value.span(), format!("“{name}” is a text, not {}", v.what())),
        };
        let number = |v: &Val| match v {
            Val::Num(n) => Ok(*n),
            v => err(value.span(), format!("“{name}” is a number, not {}", v.what())),
        };
        if in_group {
            let sc = self.scope();
            let fid = sc.open.last().cloned().expect("a group");
            let f = &mut sc.frames.iter_mut().find(|(id, _)| *id == fid).expect("open").1;
            match name {
                "color" | "colour" => {
                    let c = text(&v)?;
                    if crate::params::parse_color(&c).is_none() {
                        return err(value.span(), format!("“{c}” is not a colour: write #rrggbb"));
                    }
                    f.color = Some(c.to_ascii_lowercase());
                }
                "description" | "note" => f.description = text(&v)?,
                "title" => f.title = text(&v)?,
                _ => return Ok(false),
            }
            return Ok(true);
        }
        if in_component {
            let kind = self.scope().component.clone().expect("a component");
            let c = self.components.get_mut(&kind).expect("declared");
            match name {
                "label" => c.label = text(&v)?,
                "category" => c.category = text(&v)?,
                "summary" => c.summary = text(&v)?,
                "version" => c.version = number(&v)? as u16,
                "hidden" => c.hidden = matches!(v, Val::Bool(true)),
                _ => return Ok(false),
            }
            return Ok(true);
        }
        match name {
            "seed" => {
                let n = number(&v)?;
                if n < 0.0 || n.fract() != 0.0 {
                    return err(value.span(), "a seed is a whole number from 0");
                }
                self.terrain.seed = n as u64;
            }
            "size" => {
                let n = number(&v)?;
                if !(16.0..=1_048_576.0).contains(&n) {
                    return err(value.span(), "the terrain's size is in blocks a side, from 16");
                }
                self.terrain.size = Some(n as u32);
            }
            "sea_level" => self.terrain.sea_level = Some(number(&v)? as f32),
            "title" | "name" => self.title = Some(text(&v)?),
            _ => {
                let _ = span;
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn attrs(&mut self, node: &str, attrs: &Attrs) -> R<()> {
        for (key, value, sp) in attrs {
            let v = self.eval(value)?;
            match (key.as_str(), v) {
                ("name" | "label", Val::Str(s)) => {
                    self.scope().nodes.get_mut(node).expect("made").name = Some(s.clone());
                    self.scope().ops.push((Op::Rename { node: node.to_owned(), to: Some(s) }, *sp));
                }
                ("note", Val::Str(s)) => {
                    // No step writes a note: the program carries it.
                    let sc = self.scope();
                    sc.nodes.get_mut(node).expect("made").note = Some(s.clone());
                    sc.notes.push((node.to_owned(), s));
                }
                ("bypass", Val::Bool(on)) => {
                    self.scope().nodes.get_mut(node).expect("made").bypass = on;
                    self.scope().ops.push((Op::Bypass { node: node.to_owned(), on }, *sp));
                }
                (k @ ("name" | "label" | "note" | "bypass"), v) => return err(*sp, format!("“{k}” takes {}, not {}", if k == "bypass" { "true or false" } else { "a text" }, v.what())),
                (k, _) => return err(*sp, format!("a node has no “{k}”: name, note or bypass{}", hint(k, ["name", "note", "bypass"]))),
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ nodes --

    /// A node made by a call, as `id` (made from the call when `None`);
    /// gives its output.
    fn call(&mut self, call: &Call, id: Option<&str>) -> R<String> {
        self.tick(call.span)?;
        let target = self.resolve(call)?;
        // Without `@`, the latest version.
        let kind = match call.version.unwrap_or_else(|| names::latest_version(&target.kind)) {
            1 => target.kind.clone(),
            v => format!("{}@{v}", target.kind),
        };
        // What it takes: inputs (name, type, optional) and settings.
        let (label, inputs, open, settings) = self.signature(&kind, call)?;
        let id = match id {
            Some(id) => id.to_owned(),
            None => {
                let stem = snake(&names::name_of(&kind).unwrap_or_else(|| "node".into()));
                let sc = self.scope();
                let mut id = stem.clone();
                let mut k = 2;
                while sc.nodes.contains_key(&id) {
                    id = format!("{stem}_{k}");
                    k += 1;
                }
                id
            }
        };
        if !valid_id(&id) || id.contains(crate::lower::SEP) || id == INPUTS {
            return err(call.span, format!("“{id}” cannot name a node: letters, digits, “_” and “-”, no “__”"));
        }
        if self.scope().nodes.contains_key(&id) {
            return err(call.span, format!("there is a node “{id}” already: in a loop, write {id}[i] = …"));
        }
        if self.scope().inputs.iter().any(|p| p.name == id) {
            return err(call.span, format!("“{id}” is an input of the component: give the node another name"));
        }
        // Arguments first, so that nested calls make their nodes before it.
        let mut links: Vec<(String, String, Span)> = Vec::new();
        let mut params: Vec<(String, Json, Span)> = target.preset.iter().map(|(k, v)| ((*k).to_owned(), v.clone(), call.name_span)).collect();
        let mut positional = 0usize;
        let mut given: BTreeSet<String> = BTreeSet::new();
        for arg in &call.args {
            let Arg { name, value, span } = arg;
            let child = match value {
                Expr::Call(c) if self.is_math(c) => self.eval(value)?,
                Expr::Call(c) => {
                    let cid = self.child_id(&id, c);
                    Val::Node(self.call(c, Some(&cid))?)
                }
                e => self.eval(e)?,
            };
            match name {
                None => {
                    // A node whose inputs grow takes as many as it is given.
                    let grown = spec_of(&kind).filter(|s| s.grows()).and_then(|s| s.open_inputs).filter(|_| positional >= inputs.len()).map(|o| (format!("{}{}", o.name, positional - inputs.len() + 1), o.kind, true));
                    let Some((input, _, _)) = inputs.get(positional).or(grown.as_ref()) else {
                        return err(*span, format!("{label} takes {} input{} in order; name the rest (scale: 900)", inputs.len(), if inputs.len() == 1 { "" } else { "s" }));
                    };
                    let input = input.clone();
                    positional += 1;
                    match child {
                        Val::Node(src) => links.push((input.clone(), src, *span)),
                        v => return err(*span, format!("{label}'s input “{input}” takes a node, not {}", v.what())),
                    }
                    given.insert(input);
                }
                Some(n) => {
                    if !given.insert(n.clone()) {
                        return err(*span, format!("“{n}” is given twice"));
                    }
                    let is_input = inputs.iter().any(|(i, _, _)| i == n) || open.iter().any(|o| n.strip_prefix(o.as_str()).is_some_and(valid_id)) || n.starts_with('@');
                    if is_input {
                        match child {
                            Val::Node(src) => links.push((n.clone(), src, *span)),
                            v => return err(*span, format!("{label}'s input “{n}” takes a node, not {}", v.what())),
                        }
                    } else if let Val::Node(src) = &child
                        && !(!n.starts_with('@') && self.scope().settings.iter().any(|d| d.name == *src))
                    {
                        if settings.iter().any(|(s, _)| s == n) {
                            // A drivable setting fed by a field.
                            match spec_of(&kind).and_then(|s| s.param(n)).map(|(_, p)| p.drivable) {
                                Some(true) => links.push((format!("@{n}"), src.clone(), *span)),
                                _ => return err(*span, format!("{label}'s setting “{n}” takes a value, not a node")),
                            }
                        } else {
                            let names = inputs.iter().map(|i| i.0.clone()).chain(settings.iter().map(|s| s.0.clone()));
                            return err(*span, format!("{label} has no input “{n}”{}", hint(n, names)));
                        }
                    } else {
                        // A node and a setting of the component by one name: the setting.
                        let child = match child {
                            Val::Node(src) if self.scope().settings.iter().any(|d| d.name == src) => Val::Setting(src),
                            c => c,
                        };
                        params.retain(|(k, _, _)| k != n);
                        params.push((n.clone(), self.setting_value(&kind, &label, &settings, n, &child, &id, *span)?, *span));
                    }
                }
            }
        }
        for (input, _, optional) in &inputs {
            if !optional && !given.contains(input) {
                self.warnings.push(Error::warning(call.span, format!("{label}'s input “{input}” is not fed: the node makes nothing until it is")));
            }
        }
        // The steps.
        let sc = self.scope();
        sc.ops.push((Op::Add { id: Some(id.clone()), kind: kind.clone(), params: BTreeMap::new(), inputs: BTreeMap::new() }, call.span));
        let mut node = Node::new(&kind);
        for (k, v, sp) in params.into_iter().filter(|p| !p.1.is_null()) {
            node.params.insert(k.clone(), v.clone());
            sc.ops.push((Op::Set { node: id.clone(), param: k, value: v }, sp));
        }
        sc.nodes.insert(id.clone(), node);
        sc.order.push(id.clone());
        if let Some(f) = sc.open.last().cloned() {
            sc.frames.iter_mut().find(|(i, _)| *i == f).expect("open").1.members.push(id.clone());
        }
        for (input, src, sp) in links {
            self.link(&id, &label, &kind, &input, &src, sp)?;
        }
        Ok(id)
    }

    /// The id of a node made inside another's arguments.
    fn child_id(&self, parent: &str, c: &Call) -> String {
        let stem = snake(&names::lookup(&c.callee).and_then(|t| names::name_of(&t.kind)).unwrap_or_else(|| c.callee.clone()));
        let stem = format!("{parent}_{}", stem.replace(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'), "_"));
        let sc = self.scopes.last().expect("scope");
        let mut id = stem.clone();
        let mut k = 2;
        while sc.nodes.contains_key(&id) {
            id = format!("{stem}_{k}");
            k += 1;
        }
        id
    }

    fn resolve(&self, call: &Call) -> R<Target> {
        let name = call.callee.as_str();
        // The document's own components first.
        if self.components.contains_key(&format!("project:{name}")) {
            return Ok(Target { kind: format!("project:{name}"), preset: vec![] });
        }
        if let Some(t) = names::lookup(name) {
            if t.kind.starts_with("project:") && !self.components.contains_key(&t.kind) {
                return err(call.name_span, format!("there is no component “{}” in this script", t.kind));
            }
            if call.version.is_some_and(|v| v != 1) && !is_component_type(&t.kind) && registry::spec(&format!("{}@{}", t.kind, call.version.unwrap_or(1))).is_none() {
                return err(call.span, format!("{name} has no version {}", call.version.unwrap_or(1)));
            }
            return Ok(t);
        }
        let mine: Vec<String> = self.components.keys().map(|k| k.trim_start_matches("project:").to_owned()).collect();
        let h = hint(name, names::all_names().map(str::to_owned).chain(mine));
        err(call.name_span, format!("unknown node “{name}”{h}"))
    }

    /// A type's label, inputs (name, type, optional), open inputs' prefixes
    /// and settings (name, kind).
    #[allow(clippy::type_complexity)]
    fn signature(&self, kind: &str, call: &Call) -> R<(String, Vec<(String, PortKind, bool)>, Vec<String>, Vec<(String, Option<ParamKind>)>)> {
        if is_component_type(kind) {
            let c = self.component_def(kind).ok_or_else(|| Error::new(call.name_span, format!("there is no composite “{kind}”")))?;
            let inputs = c.inputs.iter().map(|p| (p.name.clone(), PortKind::from_id(&p.kind).unwrap_or(PortKind::Height), p.optional)).collect();
            let settings = c.exposed.iter().map(|e| (e.name.clone(), None)).collect();
            return Ok((names::name_of(kind).unwrap_or_else(|| kind.to_owned()), inputs, Vec::new(), settings));
        }
        let spec = spec_of(kind).ok_or_else(|| Error::new(call.name_span, format!("there is no type of node “{kind}”")))?;
        let inputs = spec.inputs.iter().map(|p| (p.name.to_owned(), p.kind, p.optional)).collect();
        let settings = spec.params.iter().map(|p| (p.name.to_owned(), Some(p.kind))).collect();
        Ok((names::name_of(kind).unwrap_or_else(|| kind.to_owned()), inputs, spec.open_families().map(|o| o.name.to_owned()).collect(), settings))
    }

    fn component_def(&self, kind: &str) -> Option<Component> {
        self.components.get(kind).cloned().or_else(|| {
            let (base, v) = match kind.split_once('@') {
                Some((b, v)) => (b, v.parse().ok()?),
                None => (kind, 1),
            };
            crate::components::builtin(base, v)
        })
    }

    /// A setting's value, checked against its kind.
    #[allow(clippy::too_many_arguments)]
    fn setting_value(&mut self, kind: &str, label: &str, settings: &[(String, Option<ParamKind>)], name: &str, v: &Val, node: &str, span: Span) -> R<Json> {
        let Some((_, pk)) = settings.iter().find(|(s, _)| s == name) else {
            return err(span, format!("{label} has no setting “{name}”{}", hint(name, settings.iter().map(|s| s.0.clone()))));
        };
        if let Val::Setting(s) = v {
            // The component's setting stands for this one.
            let target = format!("{node}.{name}");
            let sc = self.scope();
            let d = sc.settings.iter_mut().find(|d| d.name == *s).expect("a setting of the component");
            d.exposed.targets.push(target);
            return match &d.default {
                Some(def) => to_json(def).ok_or_else(|| Error::new(span, "a setting's default is a value")),
                None => Ok(Json::Null),
            };
        }
        let json = to_json(v).ok_or_else(|| Error::new(span, format!("“{name}” takes a value, not {}", v.what())))?;
        let Some(pk) = pk else { return Ok(json) };
        let _ = kind;
        let bad = |want: &str| err(span, format!("{label}'s setting “{name}” takes {want}, not {}", v.what()));
        match (pk, v) {
            (ParamKind::Float { min, max, .. }, Val::Num(n)) => {
                if *n < *min || *n > *max {
                    self.warnings.push(Error::warning(span, format!("{label}'s “{name}” goes from {} to {}: {} is brought back into it", super::decompile::number(*min), super::decompile::number(*max), super::decompile::number(*n))));
                }
                Ok(json)
            }
            (ParamKind::Int { min, max, .. }, Val::Num(n)) => {
                if n.fract() != 0.0 {
                    return bad("a whole number");
                }
                if (*n as i64) < *min || (*n as i64) > *max {
                    self.warnings.push(Error::warning(span, format!("{label}'s “{name}” goes from {min} to {max}: {n} is brought back into it")));
                }
                Ok(json)
            }
            (ParamKind::Seed, Val::Num(n)) if n.fract() == 0.0 && *n >= 0.0 => Ok(json),
            (ParamKind::Float { .. }, _) | (ParamKind::Int { .. }, _) => bad("a number"),
            (ParamKind::Seed, _) => bad("a whole number from 0"),
            (ParamKind::Bool { .. }, Val::Bool(_)) => Ok(json),
            (ParamKind::Bool { .. }, _) => bad("true or false"),
            (ParamKind::Choice { options, .. }, Val::Str(s)) => {
                if options.iter().any(|o| o.value == s) {
                    Ok(json)
                } else {
                    let all = options.iter().map(|o| format!("\"{}\"", o.value)).collect::<Vec<_>>().join(", ");
                    err(span, format!("{label}'s “{name}” is one of {all}{}", hint(s, options.iter().map(|o| o.value))))
                }
            }
            (ParamKind::Choice { .. }, _) => bad("one of its choices, in quotes"),
            (ParamKind::Color { .. }, Val::Str(s)) if crate::params::parse_color(s).is_some() => Ok(json),
            (ParamKind::Color { .. }, _) => bad("a colour, #rrggbb"),
            (ParamKind::Text { .. } | ParamKind::File { .. }, Val::Str(_)) => Ok(json),
            (ParamKind::Text { .. } | ParamKind::File { .. }, _) => bad("a text"),
            (ParamKind::Points { .. } | ParamKind::Curve { .. } | ParamKind::Gradient { .. }, Val::List(l)) if l.iter().all(|p| matches!(p, Val::Obj(_))) => Ok(json),
            (ParamKind::Points { .. } | ParamKind::Curve { .. }, _) => bad("a list of points: [{ x: 0.3, z: 0.6 }, …]"),
            (ParamKind::Gradient { .. }, _) => bad("a list of stops: [{ at: 0, color: \"#203040\" }, …]"),
            (ParamKind::Profile { .. }, Val::List(l)) if l.iter().all(|p| matches!(p, Val::List(xy) if xy.len() == 2)) => Ok(json),
            (ParamKind::Profile { .. }, _) => bad("a list of points: [[0, 0], [1, 1]]"),
        }
    }

    /// The type of what `src` carries, here.
    fn source_kind(&self, src: &str) -> Option<PortKind> {
        let sc = self.scopes.last()?;
        let s = Source::parse(src);
        if s.node == INPUTS && sc.component.is_some() {
            return sc.inputs.iter().find(|p| Some(p.name.as_str()) == s.output).and_then(|p| PortKind::from_id(&p.kind));
        }
        let g = Graph { nodes: sc.nodes.clone(), components: self.components.clone(), ..Graph::default() };
        crate::group::kind_of(&g, src)
    }

    fn link(&mut self, id: &str, label: &str, kind: &str, input: &str, src: &str, span: Span) -> R<()> {
        // What the input takes.
        let want = if is_component_type(kind) {
            self.component_def(kind).and_then(|c| c.inputs.iter().find(|p| p.name == input).and_then(|p| PortKind::from_id(&p.kind)))
        } else {
            let spec = spec_of(kind).expect("checked");
            match input.strip_prefix('@') {
                Some(p) => match spec.param(p) {
                    Some((_, ps)) if ps.drivable => Some(PortKind::Mask),
                    _ => return err(span, format!("{label} has no setting “{p}” a field can drive{}", hint(p, spec.params.iter().filter(|p| p.drivable).map(|p| p.name)))),
                },
                None => spec.inputs.iter().find(|p| p.name == input).or(spec.open_input(input)).map(|p| p.kind),
            }
        };
        let s = Source::parse(src);
        // An output asked for by name exists.
        if let Some(out) = s.output
            && s.node != INPUTS
        {
            let sc = self.scopes.last().expect("scope");
            let n = sc.nodes.get(s.node).expect("made before");
            let outs: Vec<String> = if n.is_component() { self.component_def(&n.kind).map(|c| c.outputs.iter().map(|o| o.name.clone()).collect()).unwrap_or_default() } else { spec_of(&n.kind).map(|sp| sp.outputs.iter().map(|o| o.name.to_owned()).collect()).unwrap_or_default() };
            if !outs.iter().any(|o| o == out) {
                return err(span, format!("“{}” has no output “{out}”: it has {}{}", s.node, outs.join(", "), hint(out, &outs)));
            }
        }
        if let (Some(want), Some(have)) = (want, self.source_kind(src))
            && have.conversion(want).is_none()
        {
            return err(span, format!("wrong type: “{src}” gives {}, but {label}'s input “{input}” takes {}", kind_words(have), kind_words(want)));
        }
        let sc = self.scope();
        sc.nodes.get_mut(id).expect("made").inputs.insert(input.to_owned(), src.to_owned());
        sc.ops.push((Op::Link { from: src.to_owned(), to: id.to_owned(), input: input.to_owned() }, span));
        Ok(())
    }

    // ---------------------------------------------------------- outputs --

    fn output(&mut self, name: &str, value: Option<&Expr>, attrs: &Attrs, span: Span) -> R<()> {
        let src = match value {
            Some(Expr::Call(c)) => {
                let id = self.child_id(&name.replace(['.', '-'], "_"), c);
                self.call(c, Some(&id))?
            }
            Some(e) => match self.eval(e)? {
                Val::Node(s) => s,
                v => return err(e.span(), format!("an output is a node, not {}", v.what())),
            },
            None => match self.scope().nodes.contains_key(name) {
                true => name.to_owned(),
                false => return err(span, format!("“output {name};” gives the node “{name}”, which there is not: write output {name} = node;")),
            },
        };
        if self.scope().component.is_some() {
            let mut decl = OutputDecl { name: name.to_owned(), label: String::new(), from: src };
            if !valid_id(name) {
                return err(span, format!("“{name}” cannot name an output: letters, digits, “_” and “-”"));
            }
            for (k, v, sp) in attrs {
                match (k.as_str(), self.eval(v)?) {
                    ("label", Val::Str(s)) => decl.label = s,
                    _ => return err(*sp, "an output of a component takes a label: { label: \"…\" }"),
                }
            }
            let sc = self.scope();
            if sc.outputs.iter().any(|(o, _)| o.name == name) {
                return err(span, format!("the component has an output “{name}” already"));
            }
            sc.outputs.push((decl, span));
            return Ok(());
        }
        // The Map Output's input.
        let have = self.source_kind(&src);
        let spec = registry::latest("output.map").expect("the Map Output");
        let fits = |input: &str| -> bool {
            let want = spec.inputs.iter().find(|p| p.name == input).or(spec.open_input(input)).map(|p| p.kind);
            match (want, have) {
                (Some(w), Some(h)) => h.conversion(w).is_some(),
                (Some(_), None) => true,
                _ => false,
            }
        };
        let input = if fits(name) {
            name.to_owned()
        } else {
            let alias = match (name, have) {
                ("relief" | "heightmap" | "elevation" | "ground" | "terrain", Some(PortKind::Height | PortKind::Mask) | None) => "height",
                ("vegetation" | "forest" | "forests" | "trees", _) => "objects",
                ("color" | "colour" | "colors" | "colours", _) => "albedo",
                ("biomes", _) => "biome",
                ("villages" | "buildings", _) => "structures",
                ("cave", _) => "caves",
                _ => "",
            };
            if !alias.is_empty() && fits(alias) {
                alias.to_owned()
            } else if spec.inputs.iter().any(|p| p.name == name) || spec.open_input(name).is_some() {
                let want = spec.inputs.iter().find(|p| p.name == name).or(spec.open_input(name)).map(|p| p.kind).expect("an input");
                return err(span, format!("wrong type: “{src}” gives {}, but the output “{name}” takes {}", have.map_or("something else".into(), kind_words), kind_words(want)));
            } else {
                let names: Vec<&str> = spec.inputs.iter().map(|p| p.name).collect();
                return err(span, format!("the map has no output “{name}”{}: it takes {}, and mask.<name>", hint(name, &names), names.join(", ")));
            }
        };
        let out = match self.scope().out_node.clone() {
            Some(o) => o,
            None => {
                let mut id = "out".to_owned();
                let mut k = 2;
                while self.scope().nodes.contains_key(&id) {
                    id = format!("out_{k}");
                    k += 1;
                }
                let sc = self.scope();
                sc.ops.push((Op::Add { id: Some(id.clone()), kind: "output.map".into(), params: BTreeMap::new(), inputs: BTreeMap::new() }, span));
                sc.nodes.insert(id.clone(), Node::new("output.map"));
                sc.order.push(id.clone());
                if let Some(f) = sc.open.last().cloned() {
                    sc.frames.iter_mut().find(|(i, _)| *i == f).expect("open").1.members.push(id.clone());
                }
                sc.out_node = Some(id.clone());
                id
            }
        };
        if self.scope().nodes[&out].inputs.contains_key(&input) {
            return err(span, format!("the map's “{input}” is given twice"));
        }
        self.link(&out, "the map", "output.map", &input, &src, span)?;
        self.attrs(&out, attrs)
    }

    // -------------------------------------------------------- components --

    fn component(&mut self, name: &str, params: &[Param], body: &[Stmt], span: Span) -> R<()> {
        if self.scopes.len() > 1 {
            return err(span, "a component is declared at the top of the script, not inside another");
        }
        let kind = format!("project:{name}");
        if !valid_id(name) || name.contains(crate::lower::SEP) {
            return err(span, format!("“{name}” cannot name a component: letters, digits, “_” and “-”"));
        }
        if self.components.contains_key(&kind) {
            return err(span, format!("there is a component “{name}” already"));
        }
        let label = {
            let words = snake(name).replace('_', " ");
            let mut cs = words.chars();
            cs.next().map(|f| f.to_ascii_uppercase().to_string() + cs.as_str()).unwrap_or_default()
        };
        self.components.insert(kind.clone(), Component { label, ..Component::default() });
        let mut scope = Scope::new(Some(kind.clone()));
        let mut vars = HashMap::new();
        for p in params {
            if let Some(k) = PortKind::from_id(&p.ty) {
                if p.default.is_some() {
                    return err(p.span, format!("an input ({}) has no default: mark it optional with “?”", p.ty));
                }
                scope.inputs.push(PortDecl { name: p.name.clone(), label: String::new(), kind: k.id().to_owned(), optional: p.optional, help: String::new() });
                vars.insert(p.name.clone(), (Val::Node(format!("{INPUTS}.{}", p.name)), false));
            } else if VALUE_TYPES.contains(&p.ty.as_str()) {
                let default = match &p.default {
                    Some(e) => Some(self.eval(e)?),
                    None => None,
                };
                let exposed = Exposed { name: p.name.clone(), label: String::new(), help: String::new(), targets: vec![], default: None, min: None, max: None, soft: None, unit: None, control: None, category: String::new(), advanced: false };
                scope.settings.push(SettingDraft { name: p.name.clone(), default, exposed, span: p.span });
                vars.insert(p.name.clone(), (Val::Setting(p.name.clone()), false));
            } else {
                let all: Vec<&str> = PortKind::ALL.iter().map(|k| k.id()).chain(VALUE_TYPES.iter().copied()).collect();
                return err(p.span, format!("unknown type “{}”{}: inputs are {}; settings are {}", p.ty, hint(&p.ty, &all), PortKind::ALL.iter().map(|k| k.id()).collect::<Vec<_>>().join(", "), VALUE_TYPES.join(", ")));
            }
            if !valid_id(&p.name) {
                return err(p.span, format!("“{}” cannot name a parameter", p.name));
            }
        }
        self.scopes.push(scope);
        // The outside's variables stay visible (constants), its nodes do not.
        self.vars.push(vars);
        let r = self.block(body);
        self.vars.pop();
        let scope = self.scopes.pop().expect("the component's");
        r?;
        if scope.outputs.is_empty() {
            return err(span, format!("component {name} gives nothing: end it with “output name;”"));
        }
        // Its steps: the shell, its inputs, its nodes inside, its outputs,
        // its settings.
        let shell = self.components[&kind].clone();
        self.steps.push((Step::Define { kind: kind.clone(), component: Component { inputs: vec![], outputs: vec![], exposed: vec![], nodes: BTreeMap::new(), ui: BTreeMap::new(), frames: BTreeMap::new(), ..shell } }, span));
        for p in &scope.inputs {
            self.steps.push((Step::Op(Op::Port { component: kind.clone(), decl: p.clone() }), span));
        }
        let mut inside: Vec<(Op, Span)> = scope.ops.clone();
        inside.extend(Self::frame_ops(&scope));
        let spans: Vec<Span> = inside.iter().map(|(_, s)| *s).collect();
        self.steps.push((Step::Inside { component: kind.clone(), ops: inside.into_iter().map(|(o, _)| o).collect(), spans }, span));
        for (o, sp) in &scope.outputs {
            self.steps.push((Step::Op(Op::Output { component: kind.clone(), decl: o.clone() }), *sp));
        }
        let mut c = self.components.remove(&kind).expect("declared");
        for s in scope.settings {
            if s.exposed.targets.is_empty() {
                self.warnings.push(Error::warning(s.span, format!("the setting “{}” feeds no node's setting: it is left out", s.name)));
                continue;
            }
            // Settings of composites inside cannot be checked against kinds.
            self.steps.push((Step::Op(Op::Expose { component: kind.clone(), exposed: s.exposed.clone() }), s.span));
            c.exposed.push(s.exposed);
        }
        self.notes.extend(scope.notes.iter().map(|(n, t)| (Some(kind.clone()), n.clone(), t.clone())));
        c.inputs = scope.inputs;
        c.outputs = scope.outputs.into_iter().map(|(o, _)| o).collect();
        c.nodes = scope.nodes;
        self.components.insert(kind, c);
        Ok(())
    }

    fn expose(&mut self, name: &str, attrs: &Attrs, span: Span) -> R<()> {
        if self.scope().component.is_none() {
            return err(span, "“expose” is written inside a component, for one of its parameters");
        }
        let mut vals = Vec::new();
        for (k, v, sp) in attrs {
            vals.push((k.clone(), self.eval(v)?, *sp));
        }
        let sc = self.scope();
        if let Some(p) = sc.inputs.iter_mut().find(|p| p.name == name) {
            for (k, v, sp) in vals {
                match (k.as_str(), v) {
                    ("label", Val::Str(s)) => p.label = s,
                    ("help", Val::Str(s)) => p.help = s,
                    (k, _) => return err(sp, format!("an input shows a label and a help, not “{k}”")),
                }
            }
            return Ok(());
        }
        let names: Vec<String> = sc.inputs.iter().map(|p| p.name.clone()).chain(sc.settings.iter().map(|s| s.name.clone())).collect();
        let Some(s) = sc.settings.iter_mut().find(|s| s.name == name) else {
            return err(span, format!("the component has no parameter “{name}”{}", hint(name, &names)));
        };
        let e = &mut s.exposed;
        for (k, v, sp) in vals {
            let num = |v: &Val| match v {
                Val::Num(n) => Ok(*n),
                v => err(sp, format!("“{k}” is a number, not {}", v.what())),
            };
            match k.as_str() {
                "label" | "help" | "unit" | "control" | "category" => {
                    let Val::Str(t) = v else { return err(sp, format!("“{k}” is a text")) };
                    match k.as_str() {
                        "label" => e.label = t,
                        "help" => e.help = t,
                        "unit" => e.unit = Some(t),
                        "control" => e.control = Some(t),
                        _ => e.category = t,
                    }
                }
                "min" => e.min = Some(num(&v)?),
                "max" => e.max = Some(num(&v)?),
                "soft" => match &v {
                    Val::List(l) if l.len() == 2 => e.soft = Some([num(&l[0])?, num(&l[1])?]),
                    _ => return err(sp, "“soft” is the slider's range: [0, 500]"),
                },
                "advanced" => e.advanced = v == Val::Bool(true),
                "default" => e.default = to_json(&v),
                other => return err(sp, format!("an exposed setting shows label, help, min, max, soft, unit, control, category, advanced or default, not “{other}”{}", hint(other, ["label", "help", "min", "max", "soft", "unit", "control", "category", "advanced", "default"]))),
            }
        }
        Ok(())
    }

    /// One frame step for each group, in the order they were opened.
    fn frame_ops(scope: &Scope) -> Vec<(Op, Span)> {
        scope.frames.iter().map(|(id, f)| (Op::Frame { id: Some(id.clone()), title: f.title.clone(), color: f.color.clone(), description: f.description.clone(), members: f.members.clone(), parent: f.parent.clone() }, f.span)).collect()
    }

    // ------------------------------------------------------- expressions --

    fn eval(&mut self, e: &Expr) -> R<Val> {
        Ok(match e {
            Expr::Num(n, _) => Val::Num(*n),
            Expr::Str(s, _) => Val::Str(s.clone()),
            Expr::Bool(b, _) => Val::Bool(*b),
            Expr::List(items, _) => Val::List(items.iter().map(|i| self.eval(i)).collect::<R<_>>()?),
            Expr::Obj(fields, _) => Val::Obj(fields.iter().map(|(k, v)| Ok((k.clone(), self.eval(v)?))).collect::<R<_>>()?),
            Expr::Name(n, sp) => self.name(n, *sp)?,
            Expr::Field(inner, f, sp) => match self.eval(inner)? {
                Val::Node(src) if !src.contains('.') => Val::Node(format!("{src}.{f}")),
                Val::Obj(o) => o.get(f).cloned().ok_or_else(|| Error::new(*sp, format!("the object has no “{f}”{}", hint(f, o.keys()))))?,
                v => return err(*sp, format!("{} has no “{f}”", v.what())),
            },
            Expr::Index(inner, i, sp) => {
                let idx = self.eval(i)?;
                if let Expr::Name(n, _) = &**inner
                    && self.lookup_var(n).is_none()
                {
                    // `ridge[2]`: the node `ridge_2`.
                    let id = match &idx {
                        Val::Num(k) if k.fract() == 0.0 => format!("{n}_{}", *k as i64),
                        Val::Str(s) => format!("{n}_{s}"),
                        v => return err(i.span(), format!("an index is a whole number or a text, not {}", v.what())),
                    };
                    return if self.scope().nodes.contains_key(&id) { Ok(Val::Node(id)) } else { err(*sp, format!("there is no node “{id}” (yet)")) };
                }
                match (self.eval(inner)?, idx) {
                    (Val::List(l), Val::Num(k)) => {
                        let k = if k < 0.0 { l.len() as f64 + k } else { k };
                        l.get(k as usize).cloned().ok_or_else(|| Error::new(*sp, format!("index {k} is past the end of a list of {}", l.len())))?
                    }
                    (Val::Obj(o), Val::Str(k)) => o.get(&k).cloned().ok_or_else(|| Error::new(*sp, format!("the object has no “{k}”")))?,
                    (v, _) => return err(*sp, format!("{} cannot be indexed", v.what())),
                }
            }
            Expr::Call(c) => {
                if let Some(v) = self.math(c)? {
                    return Ok(v);
                }
                // A node written inside an expression.
                let id = self.child_id("", c);
                let id = id.trim_start_matches('_').to_owned();
                Val::Node(self.call(c, Some(&id))?)
            }
            Expr::Unary(op, x, sp) => match (*op, self.eval(x)?) {
                ("-", Val::Num(n)) => Val::Num(-n),
                ("!", Val::Bool(b)) => Val::Bool(!b),
                (_, Val::Setting(s)) => return err(*sp, format!("the component's setting “{s}” feeds a setting as it is, not through a calculation")),
                (op, v) => return err(*sp, format!("“{op}” does not apply to {}", v.what())),
            },
            Expr::Binary(op, a, b, sp) => {
                let (a, b) = (self.eval(a)?, self.eval(b)?);
                binary(op, a, b, *sp)?
            }
            Expr::Range(a, b, sp) => match (self.eval(a)?, self.eval(b)?) {
                (Val::Num(a), Val::Num(b)) if a.fract() == 0.0 && b.fract() == 0.0 => {
                    if b - a > 10_000.0 {
                        return err(*sp, "a range of more than 10 000 steps would make too many nodes");
                    }
                    Val::List(((a as i64)..(b as i64)).map(|k| Val::Num(k as f64)).collect())
                }
                _ => return err(*sp, "a range goes between whole numbers: 0..4"),
            },
            Expr::Cond(c, a, b, sp) => match self.eval(c)? {
                Val::Bool(true) => self.eval(a)?,
                Val::Bool(false) => self.eval(b)?,
                v => return err(*sp, format!("a condition is true or false, not {}", v.what())),
            },
        })
    }

    fn name(&mut self, n: &str, sp: Span) -> R<Val> {
        if let Some((v, _)) = self.lookup_var(n).cloned() {
            // A node of that name stands first; a setting takes it back
            // where a setting is given.
            if matches!(v, Val::Setting(_)) && self.scope().nodes.contains_key(n) {
                return Ok(Val::Node(n.to_owned()));
            }
            return Ok(v);
        }
        if self.scope().nodes.contains_key(n) {
            return Ok(Val::Node(n.to_owned()));
        }
        match n {
            "pi" | "PI" => return Ok(Val::Num(std::f64::consts::PI)),
            "seed" if self.scope().component.is_none() => return Ok(Val::Num(self.terrain.seed as f64)),
            _ => {}
        }
        let sc = self.scope();
        let mut known: Vec<String> = sc.nodes.keys().cloned().collect();
        known.extend(self.vars.iter().flat_map(|v| v.keys().cloned()));
        if names::lookup(n).is_some() {
            return err(sp, format!("“{n}” is a type of node: call it, {n}(…)"));
        }
        err(sp, format!("“{n}” is not defined{}", hint(n, &known)))
    }

    fn is_math(&self, c: &Call) -> bool {
        MATH.contains(&c.callee.as_str())
    }

    /// The functions of arithmetic, which make no node.
    fn math(&mut self, c: &Call) -> R<Option<Val>> {
        const F1: &[&str] = &["abs", "sqrt", "floor", "ceil", "round", "sin", "cos", "tan", "exp", "ln", "log10", "len", "str", "int"];
        const F2: &[&str] = &["min", "max", "pow", "atan2"];
        const F3: &[&str] = &["clamp", "lerp"];
        let f = c.callee.as_str();
        let n = if F1.contains(&f) { 1 } else if F2.contains(&f) { 2 } else if F3.contains(&f) { 3 } else { return Ok(None) };
        if c.args.len() != n || c.args.iter().any(|a| a.name.is_some()) {
            return err(c.span, format!("{f} takes {n} value{}", if n == 1 { "" } else { "s" }));
        }
        let vals: Vec<Val> = c.args.iter().map(|a| self.eval(&a.value)).collect::<R<_>>()?;
        if f == "len" {
            return match &vals[0] {
                Val::List(l) => Ok(Some(Val::Num(l.len() as f64))),
                Val::Str(s) => Ok(Some(Val::Num(s.chars().count() as f64))),
                v => err(c.span, format!("len takes a list or a text, not {}", v.what())),
            };
        }
        if f == "str" {
            return Ok(Some(Val::Str(match &vals[0] {
                Val::Num(n) => super::decompile::number(*n),
                Val::Str(s) => s.clone(),
                Val::Bool(b) => b.to_string(),
                v => return err(c.span, format!("str takes a number or a text, not {}", v.what())),
            })));
        }
        let mut x = Vec::new();
        for (v, a) in vals.iter().zip(&c.args) {
            match v {
                Val::Num(n) => x.push(*n),
                v => return err(a.span, format!("{f} takes numbers, not {}", v.what())),
            }
        }
        Ok(Some(Val::Num(match f {
            "abs" => x[0].abs(),
            "sqrt" => x[0].sqrt(),
            "floor" | "int" => x[0].floor(),
            "ceil" => x[0].ceil(),
            "round" => x[0].round(),
            "sin" => x[0].sin(),
            "cos" => x[0].cos(),
            "tan" => x[0].tan(),
            "exp" => x[0].exp(),
            "ln" => x[0].ln(),
            "log10" => x[0].log10(),
            "min" => x[0].min(x[1]),
            "max" => x[0].max(x[1]),
            "pow" => x[0].powf(x[1]),
            "atan2" => x[0].atan2(x[1]),
            "clamp" => x[0].clamp(x[1].min(x[2]), x[2].max(x[1])),
            _ => x[0] + (x[1] - x[0]) * x[2],
        })))
    }
}

/// The functions of arithmetic: a call of one makes no node.
const MATH: &[&str] = &["abs", "sqrt", "floor", "ceil", "round", "sin", "cos", "tan", "exp", "ln", "log10", "len", "str", "int", "min", "max", "pow", "atan2", "clamp", "lerp"];

fn binary(op: &str, a: Val, b: Val, sp: Span) -> R<Val> {
    if let Val::Setting(s) | Val::Node(s) = if matches!(a, Val::Setting(_) | Val::Node(_)) { &a } else { &b } {
        let what = if matches!(a, Val::Setting(_)) || matches!(b, Val::Setting(_)) { format!("the component's setting “{s}” feeds a setting as it is, not through a calculation") } else { format!("“{s}” is a node: combine nodes with a node (Add, Multiply, Blend…), not “{op}”") };
        return err(sp, what);
    }
    Ok(match (op, a, b) {
        ("+", Val::Num(x), Val::Num(y)) => Val::Num(x + y),
        ("-", Val::Num(x), Val::Num(y)) => Val::Num(x - y),
        ("*", Val::Num(x), Val::Num(y)) => Val::Num(x * y),
        ("/", Val::Num(_), Val::Num(0.0)) => return err(sp, "a division by zero"),
        ("/", Val::Num(x), Val::Num(y)) => Val::Num(x / y),
        ("%", Val::Num(x), Val::Num(y)) if y != 0.0 => Val::Num(x.rem_euclid(y)),
        ("^", Val::Num(x), Val::Num(y)) => Val::Num(x.powf(y)),
        ("+", Val::Str(x), y) => Val::Str(x + &show(&y)),
        ("+", x @ Val::Num(_), Val::Str(y)) => Val::Str(show(&x) + &y),
        ("+", Val::List(mut x), Val::List(y)) => {
            x.extend(y);
            Val::List(x)
        }
        ("==", x, y) => Val::Bool(x == y),
        ("!=", x, y) => Val::Bool(x != y),
        ("<", Val::Num(x), Val::Num(y)) => Val::Bool(x < y),
        (">", Val::Num(x), Val::Num(y)) => Val::Bool(x > y),
        ("<=", Val::Num(x), Val::Num(y)) => Val::Bool(x <= y),
        (">=", Val::Num(x), Val::Num(y)) => Val::Bool(x >= y),
        ("&&", Val::Bool(x), Val::Bool(y)) => Val::Bool(x && y),
        ("||", Val::Bool(x), Val::Bool(y)) => Val::Bool(x || y),
        (op, x, y) => return err(sp, format!("“{op}” does not apply to {} and {}", x.what(), y.what())),
    })
}

fn show(v: &Val) -> String {
    match v {
        Val::Num(n) => super::decompile::number(*n),
        Val::Str(s) => s.clone(),
        Val::Bool(b) => b.to_string(),
        other => format!("{other:?}"),
    }
}

