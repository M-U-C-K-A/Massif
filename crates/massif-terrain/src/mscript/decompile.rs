//! Any graph written as a script, which makes the same graph again.

use super::names::{self, snake};
use super::syntax::KEYWORDS;
use crate::model::{Component, Frame, Graph, INPUTS, Node, Source, is_component_type};
use crate::registry::{self, ParamKind};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A number as a script writes it: `800`, `0.35`, `-2.5`.
pub fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1.0e15 { format!("{}", v as i64) } else { format!("{v}") }
}

/// A name as a script writes it: as it is, or between backticks.
pub fn ident(s: &str) -> String {
    let plain = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !KEYWORDS.contains(&s);
    if plain { s.to_owned() } else { format!("`{s}`") }
}

fn string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON value as a script writes it.
pub fn value(v: &Json) -> String {
    match v {
        Json::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => string(s),
        Json::Bool(b) => b.to_string(),
        Json::Null => "0".to_owned(),
        Json::Array(a) => format!("[{}]", a.iter().map(value).collect::<Vec<_>>().join(", ")),
        Json::Object(o) if o.is_empty() => "{}".to_owned(),
        Json::Object(o) => format!("{{ {} }}", o.iter().map(|(k, v)| format!("{}: {}", if ident(k) == *k { k.clone() } else { string(k) }, value(v))).collect::<Vec<_>>().join(", ")),
    }
}

/// `noise.fbm` → `FractalNoise`, as the script can call it here.
fn callee(kind: &str, mine: &BTreeSet<String>) -> String {
    let (base, version) = match kind.split_once('@') {
        Some((b, v)) => (b, Some(v)),
        None => (kind, None),
    };
    let at = version.map(|v| format!("@{v}")).unwrap_or_default();
    if let Some(n) = base.strip_prefix("project:") {
        return format!("{}{at}", ident(n));
    }
    match names::name_of(base) {
        // A component of the document by the same name would be called instead.
        Some(n) if !mine.contains(&n) => format!("{n}{at}"),
        _ if !is_component_type(base) && base.split('.').all(|p| ident(p) == p) => format!("{base}{at}"),
        _ => format!("{}{at}", string(kind)),
    }
}

/// Where a source is read from, as a script writes it.
fn source(src: &str) -> String {
    let s = Source::parse(src);
    if s.node == INPUTS
        && let Some(o) = s.output
    {
        return ident(o);
    }
    match s.output {
        Some(o) => format!("{}.{}", ident(s.node), ident(o)),
        None => ident(s.node),
    }
}

/// What a scope (the graph, or a composite) writes from.
struct Scope<'a> {
    nodes: &'a BTreeMap<String, Node>,
    frames: &'a BTreeMap<String, Frame>,
    /// Inside a composite: its exposed settings, by `node.setting`, with
    /// the rank of the target (so that they are given in their order).
    exposed: BTreeMap<String, (String, usize)>,
    top: bool,
}

/// Writes `graph` as a script, under `title`. Compiling it gives the same
/// graph (laid out again).
pub fn decompile(graph: &Graph, title: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "project {} {{", string(title.unwrap_or("Terrain")));
    let _ = writeln!(out, "    seed = {};", graph.terrain.seed);
    if let Some(s) = graph.terrain.size {
        let _ = writeln!(out, "    size = {s};");
    }
    if let Some(s) = graph.terrain.sea_level {
        let _ = writeln!(out, "    sea_level = {};", number(s as f64));
    }
    let mine: BTreeSet<String> = graph.components.keys().filter_map(|k| k.strip_prefix("project:")).map(str::to_owned).collect();
    // Components, those used inside others first.
    let mut done: BTreeSet<String> = BTreeSet::new();
    fn visit(g: &Graph, k: &str, done: &mut BTreeSet<String>, order: &mut Vec<String>) {
        if !done.insert(k.to_owned()) {
            return;
        }
        if let Some(c) = g.components.get(k) {
            for n in c.nodes.values() {
                if g.components.contains_key(&n.kind) {
                    visit(g, &n.kind, done, order);
                }
            }
            order.push(k.to_owned());
        }
    }
    let mut order = Vec::new();
    for k in graph.components.keys() {
        visit(graph, k, &mut done, &mut order);
    }
    for k in &order {
        out.push('\n');
        component(&mut out, k, &graph.components[k], graph, &mine);
    }
    let scope = Scope { nodes: &graph.nodes, frames: &graph.frames, exposed: BTreeMap::new(), top: true };
    let body = body(&scope, graph, &mine, 1);
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
    }
    out.push_str("}\n");
    out
}

fn component(out: &mut String, kind: &str, c: &Component, g: &Graph, mine: &BTreeSet<String>) {
    let name = kind.trim_start_matches("project:");
    let inner = Graph { nodes: c.nodes.clone(), components: g.components.clone(), ..Graph::default() };
    let mut params: Vec<String> = c.inputs.iter().map(|p| format!("{}: {}{}", ident(&p.name), p.kind, if p.optional { "?" } else { "" })).collect();
    let mut exposed = BTreeMap::new();
    for e in &c.exposed {
        let targets: Vec<(&str, &str)> = e.targets.iter().filter_map(|t| t.split_once('.')).filter(|(n, _)| c.nodes.contains_key(*n)).collect();
        if targets.is_empty() {
            continue;
        }
        for (n, p) in &targets {
            let rank = exposed.len();
            exposed.insert(format!("{n}.{p}"), (e.name.clone(), rank));
        }
        let spec_kind = targets.iter().find_map(|(n, p)| registry::spec(&c.nodes[*n].kind).and_then(|s| s.param(p)).map(|(_, ps)| ps.kind));
        let ty = match spec_kind {
            Some(ParamKind::Float { .. }) => "float",
            Some(ParamKind::Int { .. }) => "int",
            Some(ParamKind::Bool { .. }) => "bool",
            Some(ParamKind::Seed) => "seed",
            Some(ParamKind::Choice { .. }) => "choice",
            Some(ParamKind::Color { .. } | ParamKind::Text { .. } | ParamKind::File { .. }) => "string",
            Some(_) => "value",
            None => "value",
        };
        let first = c.nodes[targets[0].0].params.get(targets[0].1);
        let default = first.map(|v| format!(" = {}", value(v))).unwrap_or_default();
        params.push(format!("{}: {ty}{default}", ident(&e.name)));
    }
    let _ = writeln!(out, "    component {}({}) {{", ident(name), params.join(", "));
    let auto = {
        let words = snake(name).replace('_', " ");
        let mut cs = words.chars();
        cs.next().map(|f| f.to_ascii_uppercase().to_string() + cs.as_str()).unwrap_or_default()
    };
    if c.label != auto {
        let _ = writeln!(out, "        label = {};", string(&c.label));
    }
    if !c.category.is_empty() {
        let _ = writeln!(out, "        category = {};", string(&c.category));
    }
    if !c.summary.is_empty() {
        let _ = writeln!(out, "        summary = {};", string(&c.summary));
    }
    if c.version != 1 {
        let _ = writeln!(out, "        version = {};", c.version);
    }
    if c.hidden {
        out.push_str("        hidden = true;\n");
    }
    for p in &c.inputs {
        let mut a = Vec::new();
        if !p.label.is_empty() {
            a.push(format!("label: {};", string(&p.label)));
        }
        if !p.help.is_empty() {
            a.push(format!("help: {};", string(&p.help)));
        }
        if !a.is_empty() {
            let _ = writeln!(out, "        expose {} {{ {} }}", ident(&p.name), a.join(" "));
        }
    }
    for e in &c.exposed {
        if !exposed.values().any(|(n, _)| *n == e.name) {
            continue;
        }
        let mut a = Vec::new();
        if !e.label.is_empty() {
            a.push(format!("label: {};", string(&e.label)));
        }
        if !e.help.is_empty() {
            a.push(format!("help: {};", string(&e.help)));
        }
        if let Some(m) = e.min {
            a.push(format!("min: {};", number(m)));
        }
        if let Some(m) = e.max {
            a.push(format!("max: {};", number(m)));
        }
        if let Some([lo, hi]) = e.soft {
            a.push(format!("soft: [{}, {}];", number(lo), number(hi)));
        }
        if let Some(u) = &e.unit {
            a.push(format!("unit: {};", string(u)));
        }
        if let Some(u) = &e.control {
            a.push(format!("control: {};", string(u)));
        }
        if !e.category.is_empty() {
            a.push(format!("category: {};", string(&e.category)));
        }
        if e.advanced {
            a.push("advanced: true;".to_owned());
        }
        if let Some(d) = &e.default {
            a.push(format!("default: {};", value(d)));
        }
        if !a.is_empty() {
            let _ = writeln!(out, "        expose {} {{ {} }}", ident(&e.name), a.join(" "));
        }
    }
    let scope = Scope { nodes: &c.nodes, frames: &c.frames, exposed, top: false };
    out.push_str(&body(&scope, &inner, mine, 2));
    for o in &c.outputs {
        let label = if o.label.is_empty() { String::new() } else { format!(" {{ label: {}; }}", string(&o.label)) };
        if o.from == o.name && label.is_empty() {
            let _ = writeln!(out, "        output {};", ident(&o.name));
        } else {
            let _ = writeln!(out, "        output {} = {}{label};", ident(&o.name), source(&o.from));
        }
    }
    out.push_str("    }\n");
}

/// The nodes of a scope, each after what feeds it, in its groups.
fn body(s: &Scope, g: &Graph, mine: &BTreeSet<String>, depth: usize) -> String {
    let mut out = String::new();
    // The Map Output written as `output` lines, when it is the plain one.
    let maps: Vec<&String> = s.nodes.iter().filter(|(_, n)| registry::spec(&n.kind).is_some_and(|sp| sp.id == "output.map")).map(|(id, _)| id).collect();
    let out_node = (s.top && maps.len() == 1 && maps[0] == "out").then(|| maps[0].clone()).filter(|id| {
        let n = &s.nodes[id];
        n.kind == "output.map" && n.params.is_empty() && n.name.is_none() && n.note.is_none() && !n.bypass && !n.inputs.is_empty()
    });
    // Each node's frame, and the frames' ancestry.
    let mut home: BTreeMap<&str, &str> = BTreeMap::new();
    for (fid, f) in s.frames {
        for m in &f.members {
            home.entry(m.as_str()).or_insert(fid.as_str());
        }
    }
    let path = |f: Option<&str>| -> Vec<String> {
        let mut p = Vec::new();
        let mut at = f.map(str::to_owned);
        while let Some(id) = at {
            if p.contains(&id) || p.len() > 64 {
                break;
            }
            at = s.frames.get(&id).and_then(|fr| fr.parent.clone());
            p.push(id);
        }
        p.reverse();
        p
    };
    // Kahn's order, keeping to the frame of the last node when it can.
    let deps = |id: &str| -> Vec<String> { s.nodes[id].inputs.values().map(|v| Source::parse(v).node.to_owned()).filter(|n| s.nodes.contains_key(n) && n != id).collect() };
    let mut left: BTreeSet<String> = s.nodes.keys().cloned().collect();
    let mut order: Vec<String> = Vec::new();
    let mut last_home: Option<&str> = None;
    while !left.is_empty() {
        let ready: Vec<&String> = left.iter().filter(|id| deps(id).iter().all(|d| !left.contains(d))).collect();
        let pick = if ready.is_empty() {
            // A loop: the graph cannot be built anyway; write what is left.
            left.iter().next().cloned().expect("not empty")
        } else {
            let fi = |id: &str| home.get(id).and_then(|h| s.frames.keys().position(|k| k == h)).map_or(0, |p| p + 1);
            ready.iter().find(|id| home.get(id.as_str()).copied() == last_home).map(|id| (*id).clone()).unwrap_or_else(|| (*ready.iter().min_by_key(|id| (fi(id), id.to_string())).expect("ready")).clone())
        };
        last_home = home.get(pick.as_str()).copied();
        left.remove(&pick);
        order.push(pick);
    }
    // Written group by group.
    let pad = |k: usize| "    ".repeat(k);
    let mut open: Vec<String> = Vec::new();
    let mut opened: BTreeSet<String> = BTreeSet::new();
    let mut slugs: BTreeSet<String> = BTreeSet::new();
    let go = |out: &mut String, want: &[String], open: &mut Vec<String>, opened: &mut BTreeSet<String>, slugs: &mut BTreeSet<String>| {
        let common = open.iter().zip(want).take_while(|(a, b)| a == b).count();
        while open.len() > common {
            open.pop();
            let _ = writeln!(out, "{}}}", pad(depth + open.len()));
        }
        for f in &want[common..] {
            let fr = &s.frames[f];
            let first = opened.insert(f.clone());
            // The id the script would make, to write only a different one.
            let stem = snake(&fr.title.replace(|c: char| !c.is_ascii_alphanumeric(), " ").to_ascii_lowercase());
            let stem = if stem.is_empty() { "group".to_owned() } else { stem };
            let mut auto = stem.clone();
            let mut k = 2;
            while slugs.contains(&auto) {
                auto = format!("{stem}_{k}");
                k += 1;
            }
            let id = if first && auto == *f { String::new() } else { format!("{} ", ident(f)) };
            if first {
                slugs.insert(f.clone());
            }
            let _ = writeln!(out, "{}group {id}{} {{", pad(depth + open.len()), string(&fr.title));
            if first {
                if let Some(c) = &fr.color {
                    let _ = writeln!(out, "{}color = {c};", pad(depth + open.len() + 1));
                }
                if !fr.description.is_empty() {
                    let _ = writeln!(out, "{}description = {};", pad(depth + open.len() + 1), string(&fr.description));
                }
            }
            open.push(f.clone());
        }
    };
    for id in &order {
        let want = path(home.get(id.as_str()).copied());
        go(&mut out, &want, &mut open, &mut opened, &mut slugs);
        let ind = pad(depth + open.len());
        if Some(id) == out_node.as_ref() {
            for (input, src) in &s.nodes[id].inputs {
                let _ = writeln!(out, "{ind}output {input} = {};", source(src));
            }
            continue;
        }
        let _ = writeln!(out, "{ind}{} = {};", ident(id), node(s, id, g, mine, &ind));
    }
    // Frames holding nothing.
    let empty: Vec<String> = s.frames.keys().filter(|f| !opened.contains(*f)).cloned().collect();
    for f in empty {
        let want = path(Some(&f));
        go(&mut out, &want, &mut open, &mut opened, &mut slugs);
    }
    go(&mut out, &[], &mut open, &mut opened, &mut slugs);
    out
}

/// A node's call, with its attributes.
fn node(s: &Scope, id: &str, g: &Graph, mine: &BTreeSet<String>, ind: &str) -> String {
    let n = &s.nodes[id];
    let mut args: Vec<String> = Vec::new();
    let (inputs, params): (Vec<String>, Vec<String>) = match registry::spec(&n.kind) {
        Some(spec) => (spec.inputs.iter().map(|p| p.name.to_owned()).collect(), spec.params.iter().map(|p| p.name.to_owned()).collect()),
        None => match crate::lower::component(g, &n.kind) {
            Some(c) => (c.inputs.iter().map(|p| p.name.clone()).collect(), c.exposed.iter().map(|e| e.name.clone()).collect()),
            None => (Vec::new(), Vec::new()),
        },
    };
    let mut used: BTreeSet<&str> = BTreeSet::new();
    let mut positional = true;
    for i in &inputs {
        match n.inputs.get(i) {
            Some(src) if positional => {
                args.push(source(src));
                used.insert(i);
            }
            Some(src) => {
                args.push(format!("{i}: {}", source(src)));
                used.insert(i);
            }
            None => positional = false,
        }
    }
    // A node whose inputs grow: its open inputs in order, after the
    // declared ones, while they are numbered one after the other.
    if positional
        && let Some(open) = registry::spec(&n.kind).filter(|s| s.grows()).and_then(|s| s.open_inputs)
    {
        for k in 1.. {
            let Some((name, src)) = n.inputs.get_key_value(&format!("{}{k}", open.name)) else { break };
            args.push(source(src));
            used.insert(name);
        }
    }
    for (i, src) in &n.inputs {
        if !used.contains(i.as_str()) {
            let name = if i.starts_with('@') || i.split('.').all(|w| ident(w) == *w) { i.clone() } else { format!("`{i}`") };
            args.push(format!("{name}: {}", source(src)));
        }
    }
    // Its own settings in the type's order, then those standing for the
    // component's, in the order the component lists them (with a value of
    // their own or not).
    let mut set: Vec<((usize, usize), String)> = Vec::new();
    for (k, v) in &n.params {
        let order = params.iter().position(|p| p == k).unwrap_or(usize::MAX);
        match s.exposed.get(&format!("{id}.{k}")) {
            Some((e, rank)) => set.push(((rank + 1, 0), format!("{k}: {}", ident(e)))),
            None if !v.is_null() => set.push(((0, order), format!("{k}: {}", value(v)))),
            None => {}
        }
    }
    for (t, (e, rank)) in &s.exposed {
        if let Some(p) = t.strip_prefix(id).and_then(|r| r.strip_prefix('.'))
            && !n.params.contains_key(p)
        {
            set.push(((rank + 1, 0), format!("{p}: {}", ident(e))));
        }
    }
    set.sort();
    args.extend(set.into_iter().map(|(_, a)| a));
    let one_line = format!("{}({})", callee(&n.kind, mine), args.join(", "));
    let call = if one_line.len() + ind.len() > 120 { format!("{}(\n{ind}    {}\n{ind})", callee(&n.kind, mine), args.join(&format!(",\n{ind}    "))) } else { one_line };
    let mut attrs = Vec::new();
    if let Some(name) = &n.name {
        attrs.push(format!("name: {};", string(name)));
    }
    if let Some(note) = &n.note {
        attrs.push(format!("note: {};", string(note)));
    }
    if n.bypass {
        attrs.push("bypass: true;".to_owned());
    }
    if attrs.is_empty() { call } else { format!("{call} {{ {} }}", attrs.join(" ")) }
}
