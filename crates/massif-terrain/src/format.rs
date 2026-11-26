//! The format as a model sees it: the JSON Schema of terrain graphs, the
//! catalogue of every type of node with an example, checking a graph with
//! paths to its mistakes, and editing one a step at a time.
//!
//! All of it is written from the registry, so it never says anything the
//! engine does not do. `docs/terrain-graph.schema.json` and
//! `docs/terrain-nodes.md` are [`schema`] and [`catalog_markdown`] written
//! out; a test checks they are current.

use crate::compile::{Diagnostic, Severity, compile};
use crate::model::{FORMAT, Graph, Node, NodeUi, Source, VERSION, valid_id};
use crate::registry::{self, Family, Field, Kernel, NodeSpec, ParamKind, ParamSpec, PortKind, PortSpec, Reads, Section, Unit};
use crate::space::Space;
use crate::util::closest;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What an input's value looks like: a node's name, then maybe one of its outputs.
const SOURCE_PATTERN: &str = "^[A-Za-z0-9_-]{1,64}(\\.[A-Za-z0-9_.-]{1,64})?$";

/// How to build a graph, for a model (or a person) about to write one.
pub const GUIDE: &str = "\
- A graph is a JSON object: `{ \"format\": \"massif.terrain-graph\", \"version\": 3, \"terrain\": { … }, \"nodes\": { … } }`. Only `nodes` is needed; `terrain` may give the terrain's `size` (blocks a side, the map's own when left out), its `seed` and its `sea_level`.
- Each node has an id of its own (letters, digits, `_` and `-`), by which the others refer to it and which never changes, and a `type` from this catalogue. `name` is what the editor shows, free to change. Their order does not matter.
- An input names what feeds it: a node's name (its first output), or `name.output` for another of its outputs. An optional input left out stands for what the catalogue says; a required one left out breaks the node, and what follows it.
- Settings left out take their defaults; a number outside a setting's range is brought back into it, with a warning.
- Positions (points, curves) are fractions of the terrain's side, from 0 (west, north) to 1 (east, south). Distances, widths and heights are in blocks, a block being a metre. Heights are absolute: the sea lies at the world's sea level (62 unless the map says otherwise), and Minecraft builds from y −64 to 320. A map of 16k is 16384 blocks a side.
- One `output.map` node, the Map Output, says what the map gets: its Height, and if wanted its Water, Albedo, Terrain, Biome, Temperature, Humidity, Vegetation, Caves, layers and any mask (`mask.<name>`). Each input left empty keeps what the map has, as the catalogue says.
- Sockets have types: height (blocks), mask (0 to 1), colour, vector, terrain and biome. A height and a mask feed each other as they are; a colour feeds a number by its brightness, a number feeds a colour as a grey; a terrain or a biome feeds only its own type.
- A setting marked drivable can follow a field: connect a height or a mask to the input `@<setting>`, and each place takes the value the field has there.
- Simulations (erosion, rainfall, humidity…) work over the whole terrain at the simulations' resolution; finer views of them carry the change they made.
- A seed left out comes from the node's id, so that two nodes never repeat each other; `terrain.seed` changes them all.
- A composite is a node that holds a graph: `massif:<name>` (shipped with Massif, such as `massif:mountain`) or `project:<name>` (defined under the document's `components`, each with its `label`, `inputs`, `outputs`, `exposed` settings and `nodes`; inside, `in.<name>` names its input). Its `params` are the settings it exposes. `\"bypass\": true` leaves a node out, what it feeds taking its first input.
- `frames` gather nodes under a title and a colour (`#rrggbb`), and nest (`parent`); they never change the terrain.
- `massif-cli terrain validate graph.json` says what is wrong, each mistake with its path (`nodes.hills.params.hieght`) and the name it probably meant. `massif-cli terrain patch` changes a graph a step at a time: `add`, `set`, `link`, `unlink`, `remove`, `rename` (the name shown), `bypass`, `frame`, `unframe`, `group` (nodes into a new composite), `ungroup`, `expose`, `unexpose`, `inside` (steps inside a composite), `port`, `unport`, `output`, `unoutput` (a composite's inputs and outputs), `fork` (a copy of a shipped composite to edit), or a JSON Patch.
";

/// The types of node a graph can name, every version, in the library's order.
pub fn public() -> impl Iterator<Item = &'static NodeSpec> {
    registry::all().iter().copied().filter(|s| s.family != Family::Internal)
}

/// The types new nodes take: the latest version of each.
pub fn current() -> impl Iterator<Item = &'static NodeSpec> {
    public().filter(|s| registry::latest(s.id).is_some_and(|l| l.version == s.version))
}

fn error(path: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(path, message)
}

fn is_error(d: &Diagnostic) -> bool {
    d.severity == Severity::Error
}

/// A number as JSON: `None` for what JSON cannot hold.
fn num(v: f64) -> Option<Value> {
    serde_json::Number::from_f64(v).map(Value::Number)
}

/// A number as people read it: `1500`, `0.35`.
fn show(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

fn unit_words(unit: Unit) -> &'static str {
    match unit {
        Unit::None | Unit::Ratio => "",
        Unit::Blocks => "blocks",
        Unit::Fraction => "fraction of the terrain's side",
        Unit::Degrees => "degrees",
        Unit::Count => "count",
        Unit::Celsius => "°C",
        Unit::Percent => "%",
    }
}

fn section_id(s: Section) -> &'static str {
    match s {
        Section::Main => "main",
        Section::Shape => "shape",
        Section::Detail => "detail",
        Section::Advanced => "advanced",
    }
}

fn kind_words(kind: PortKind) -> &'static str {
    match kind {
        PortKind::Height => "Heights of the ground, in blocks.",
        PortKind::Mask => "A value from 0 to 1 at each place: where something is, and how much.",
        PortKind::Color => "A colour at each place.",
        PortKind::Vector => "A direction and its strength at each place: wind, the way water flows.",
        PortKind::Terrain => "A terrain (the block material) at each place.",
        PortKind::Biome => "A biome of the game at each place.",
        PortKind::Layer => "A painted layer of the world at each place (ground cover, objects, pockets, caves, annotations), and how strongly.",
        PortKind::Points => "Where things stand: points with a turn, a scale, a kind and numbers of their own.",
        PortKind::Splines => "Lines through the terrain: roads, rivers, walls, each with a width and a kind.",
        PortKind::Footprints => "The ground things take: rectangles of columns, with the side they face and their entrance.",
        PortKind::Instances => "What stands there: a pack's building, a structure of the game, an object, each placed and turned.",
        PortKind::Density => "A field over the volume, negative inside matter: caves, arches, overhangs.",
    }
}

/// What a setting is, in a word.
fn param_kind(p: &ParamSpec) -> &'static str {
    match p.kind {
        ParamKind::Float { .. } => "number",
        ParamKind::Int { .. } => "integer",
        ParamKind::Bool { .. } => "boolean",
        ParamKind::Choice { .. } => "choice",
        ParamKind::Color { .. } => "colour",
        ParamKind::Seed => "seed",
        ParamKind::Points { .. } => "points",
        ParamKind::Curve { closed: false, .. } => "curve",
        ParamKind::Curve { closed: true, .. } => "area",
        ParamKind::Profile { .. } => "profile",
        ParamKind::Gradient { .. } => "gradient",
        ParamKind::Text { .. } => "text",
        ParamKind::File { .. } => "file",
    }
}

/// A setting's default as JSON (`None` when it has none to write).
fn param_default(p: &ParamSpec) -> Option<Value> {
    match p.kind {
        ParamKind::Float { default, .. } => num(default),
        ParamKind::Int { default, .. } => Some(json!(default)),
        ParamKind::Bool { default } => Some(json!(default)),
        ParamKind::Choice { default, .. } | ParamKind::Color { default } | ParamKind::Text { default } => Some(json!(default)),
        ParamKind::Profile { default } => Some(json!(default)),
        ParamKind::Gradient { default } => Some(Value::Array(default.iter().map(|(at, c)| json!({ "at": at, "color": c })).collect())),
        ParamKind::Points { .. } | ParamKind::Curve { .. } => Some(json!([])),
        ParamKind::Seed | ParamKind::File { .. } => None,
    }
}

/// A setting's help, with what the schema cannot say by itself.
fn param_words(p: &ParamSpec) -> String {
    let mut s = p.help.to_owned();
    if let ParamKind::Float { soft, min, max, unit, .. } = p.kind {
        let u = unit_words(unit);
        let said = u.split(' ').next().is_some_and(|w| p.help.to_lowercase().contains(&w.to_lowercase()));
        if !u.is_empty() && !said {
            let _ = write!(s, " In {u}.");
        }
        if soft != [min, max] {
            let _ = write!(s, " Usually {} to {}.", show(soft[0]), show(soft[1]));
        }
    }
    if matches!(p.kind, ParamKind::Seed) {
        s.push_str(" Left out, it comes from the node's name.");
    }
    if p.drivable {
        let _ = write!(s, " A field connected to the input `@{}` drives it place by place.", p.name);
    }
    s
}

// ---------------------------------------------------------------- schema --

/// The JSON Schema (2020-12) of terrain graphs: every type of node with its
/// inputs and settings, their units, ranges and defaults.
pub fn schema() -> Value {
    let mut defs = Map::new();
    defs.insert(
        "source".into(),
        json!({ "type": "string", "pattern": SOURCE_PATTERN, "description": "What feeds an input: a node's name, and after a dot one of its outputs (the first when left out), as \"hills\" or \"worn.flow\"." }),
    );
    let ids: Vec<String> = public().map(|s| s.type_name()).collect();
    let mut cases = Vec::new();
    for s in public() {
        defs.insert(s.type_name(), node_schema(s));
        cases.push(json!({ "if": { "properties": { "type": { "const": s.type_name() } }, "required": ["type"] }, "then": { "$ref": format!("#/$defs/{}", s.type_name()) } }));
    }
    defs.insert(
        "node".into(),
        json!({ "description": "A node: its type, what feeds its inputs, its settings.", "type": "object", "required": ["type"], "properties": { "type": { "anyOf": [{ "enum": ids }, { "$ref": "#/$defs/composite_type" }] } }, "allOf": cases }),
    );
    defs.insert(
        "composite_type".into(),
        json!({ "type": "string", "pattern": "^(project|massif):[a-z0-9_-]+(@[0-9]+)?$", "description": "A composite: project:<name> (one of the document's components) or massif:<name> (one Massif ships), with @N for a later version. Its params are the settings it exposes, its inputs and outputs those it declares." }),
    );
    defs.insert(
        "frame".into(),
        json!({ "type": "object", "additionalProperties": false, "required": ["title"], "description": "A frame round nodes in the editor; never read to evaluate.", "properties": {
            "title": { "type": "string" }, "color": { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" }, "description": { "type": "string" },
            "parent": { "type": "string", "description": "The frame it sits in." }, "members": { "type": "array", "items": { "type": "string" }, "description": "The nodes it holds." },
            "rect": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }, "collapsed": { "type": "boolean" } } }),
    );
    defs.insert(
        "component".into(),
        json!({ "type": "object", "additionalProperties": false, "required": ["label", "nodes"], "description": "A composite node: a graph of its own that runs as one node. Inside, in.<name> names its input <name>.", "properties": {
            "version": { "type": "integer", "minimum": 1 }, "label": { "type": "string" }, "category": { "type": "string" }, "summary": { "type": "string" },
            "inputs": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["name", "type"], "properties": { "name": { "type": "string" }, "label": { "type": "string" }, "type": { "enum": PortKind::ALL.iter().map(|k| k.id()).collect::<Vec<_>>() }, "optional": { "type": "boolean" }, "help": { "type": "string" } } } },
            "outputs": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["name", "from"], "properties": { "name": { "type": "string" }, "label": { "type": "string" }, "from": { "$ref": "#/$defs/source" } } } },
            "exposed": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["name", "targets"], "properties": {
                "name": { "type": "string" }, "label": { "type": "string" }, "help": { "type": "string" }, "targets": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "The settings inside it stands for, each node.setting." },
                "default": {}, "min": { "type": "number" }, "max": { "type": "number" }, "soft": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                "unit": { "type": "string" }, "control": { "type": "string" }, "category": { "type": "string" }, "advanced": { "type": "boolean" } } } },
            "nodes": { "type": "object", "additionalProperties": { "$ref": "#/$defs/node" } },
            "ui": { "type": "object" }, "frames": { "type": "object", "additionalProperties": { "$ref": "#/$defs/frame" } } } }),
    );
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:massif:terrain-graph:3",
        "title": "Massif terrain graph",
        "description": "A terrain graph of Massif: named nodes, each of a type, its inputs naming the nodes that feed them. One output.map node says what the map gets.",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "format": { "const": FORMAT },
            "version": { "type": "integer", "minimum": 2, "maximum": VERSION, "description": "The version of the format: 3 (2 is read and upgraded)." },
            "terrain": {
                "type": "object",
                "additionalProperties": false,
                "description": "What the whole graph shares.",
                "properties": {
                    "size": { "type": "integer", "minimum": 1, "description": "Blocks a side of the square of the world the terrain covers; left out, the map's own." },
                    "seed": { "type": "integer", "minimum": 0, "description": "The seed all the graph's randomness starts from." },
                    "sea_level": { "type": "number", "description": "The sea level, in blocks; left out, the world's." }
                }
            },
            "nodes": {
                "type": "object",
                "description": "The nodes, by name.",
                "propertyNames": { "pattern": "^[A-Za-z0-9_-]{1,200}$" },
                "additionalProperties": { "$ref": "#/$defs/node" }
            },
            "ui": {
                "type": "object",
                "description": "Where nodes sit in the editor, by node; never read to evaluate the graph.",
                "additionalProperties": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["pos"],
                    "properties": {
                        "pos": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                        "collapsed": { "type": "boolean" }
                    }
                }
            },
            "frames": { "type": "object", "description": "Frames round nodes, by id; frames nest.", "additionalProperties": { "$ref": "#/$defs/frame" } },
            "components": { "type": "object", "description": "The composite nodes the document defines, by type (project:<name>).", "propertyNames": { "pattern": "^project:[a-z0-9_-]+$" }, "additionalProperties": { "$ref": "#/$defs/component" } },
            "editor": { "description": "The rest of the editor's layout (points on wires, the view); never read to evaluate the graph." }
        },
        "$defs": defs
    })
}

fn port_schema(p: &PortSpec) -> Value {
    let mut words = format!("{} ({})", p.help, p.kind.id());
    if p.optional && !p.default.is_empty() {
        let _ = write!(words, " Left empty: {}.", p.default);
    }
    json!({ "$ref": "#/$defs/source", "title": p.label, "description": words })
}

fn node_schema(s: &NodeSpec) -> Value {
    let mut inputs = Map::new();
    let mut required = Vec::new();
    for p in s.inputs {
        inputs.insert(p.name.into(), port_schema(p));
        if !p.optional {
            required.push(p.name);
        }
    }
    for p in s.params.iter().filter(|p| p.drivable) {
        inputs.insert(format!("@{}", p.name), json!({ "$ref": "#/$defs/source", "description": format!("A field driving “{}”: each place takes the value it has there.", p.label) }));
    }
    let mut ins = json!({ "type": "object", "additionalProperties": false, "properties": inputs });
    if !required.is_empty() {
        ins["required"] = json!(required);
    }
    let patterns: Map<String, Value> = s.open_families().map(|open| (format!("^{}[A-Za-z0-9_-]{{1,64}}$", open.name.replace('.', "\\.")), port_schema(open))).collect();
    if !patterns.is_empty() {
        ins["patternProperties"] = Value::Object(patterns);
    }
    let params: Map<String, Value> = s.params.iter().map(|p| (p.name.to_owned(), param_schema(p))).collect();
    let mut node = json!({
        "title": s.label,
        "description": format!("{} {}", s.summary, s.help),
        "type": "object",
        "additionalProperties": false,
        "required": ["type"],
        "properties": {
            "type": { "const": s.type_name() },
            "inputs": ins,
            "params": { "type": "object", "additionalProperties": false, "properties": params },
            "note": { "type": "string", "description": "A note for whoever reads the graph." },
            "name": { "type": "string", "description": "What the editor calls the node (its type's label when left out); its id never changes." },
            "bypass": { "type": "boolean", "description": "Left out of the graph: what it feeds takes its first input instead." }
        }
    });
    if !required.is_empty() {
        node["required"] = json!(["type", "inputs"]);
    }
    node
}

fn number_schema(min: f64, max: f64, default: Option<f64>) -> Value {
    let mut v = json!({ "type": "number" });
    if let Some(m) = num(min).filter(|_| min.is_finite()) {
        v["minimum"] = m;
    }
    if let Some(m) = num(max).filter(|_| max.is_finite()) {
        v["maximum"] = m;
    }
    if let Some(d) = default.and_then(num) {
        v["default"] = d;
    }
    v
}

fn point_schema(fields: &[Field]) -> Value {
    let x = json!({ "type": "number", "description": "West to east, as a fraction of the terrain's side (0 to 1)." });
    let z = json!({ "type": "number", "description": "North to south, as a fraction of the terrain's side (0 to 1)." });
    let field = |f: &Field| {
        let mut v = number_schema(f.min, f.max, Some(f.default));
        let u = unit_words(f.unit);
        v["title"] = json!(f.label);
        v["description"] = json!(if u.is_empty() { f.help.to_owned() } else { format!("{} In {u}.", f.help) });
        v
    };
    let mut props = Map::new();
    props.insert("x".into(), x.clone());
    props.insert("z".into(), z.clone());
    for f in fields {
        props.insert(f.name.into(), field(f));
    }
    let mut items = vec![x, z];
    items.extend(fields.iter().map(field));
    json!({ "anyOf": [
        { "type": "object", "additionalProperties": false, "required": ["x", "z"], "properties": props },
        { "type": "array", "minItems": 2, "maxItems": 2 + fields.len(), "prefixItems": items }
    ] })
}

fn param_schema(p: &ParamSpec) -> Value {
    let unit01 = json!({ "type": "number", "minimum": 0, "maximum": 1 });
    let mut v = match p.kind {
        ParamKind::Float { min, max, soft, default, unit } => {
            let mut v = number_schema(min, max, Some(default));
            v["x-unit"] = json!(unit.id());
            v["x-soft"] = json!([soft[0], soft[1]]);
            v
        }
        ParamKind::Int { min, max, default } => json!({ "type": "integer", "minimum": min, "maximum": max, "default": default }),
        ParamKind::Bool { default } => json!({ "type": "boolean", "default": default }),
        ParamKind::Choice { options, default } => json!({
            "enum": options.iter().map(|o| o.value).collect::<Vec<_>>(),
            "default": default,
            "x-options": options.iter().map(|o| json!({ "value": o.value, "label": o.label, "help": o.help })).collect::<Vec<_>>()
        }),
        ParamKind::Color { default } => json!({
            "anyOf": [{ "type": "string", "pattern": "^#[0-9A-Fa-f]{6}$" }, { "type": "array", "items": unit01, "minItems": 3, "maxItems": 3 }],
            "default": default
        }),
        ParamKind::Seed => json!({ "type": "integer", "minimum": 0, "maximum": u32::MAX }),
        ParamKind::Points { fields, max } => json!({ "type": "array", "maxItems": max, "items": point_schema(fields), "default": [] }),
        ParamKind::Curve { fields, closed } => json!({ "type": "array", "items": point_schema(fields), "default": [], "x-closed": closed }),
        ParamKind::Profile { default } => json!({
            "type": "array",
            "minItems": 2,
            "items": { "type": "array", "prefixItems": [unit01, unit01], "minItems": 2, "maxItems": 2 },
            "default": default
        }),
        ParamKind::Gradient { .. } => json!({
            "type": "array",
            "minItems": 1,
            "items": { "anyOf": [
                { "type": "object", "additionalProperties": false, "required": ["at", "color"], "properties": { "at": unit01, "color": { "type": "string", "pattern": "^#[0-9A-Fa-f]{6}$" } } },
                { "type": "array", "prefixItems": [unit01, { "type": "string", "pattern": "^#[0-9A-Fa-f]{6}$" }], "minItems": 2, "maxItems": 2 }
            ] },
            "default": param_default(p)
        }),
        ParamKind::Text { default } => json!({ "type": "string", "default": default }),
        ParamKind::File { extensions } => json!({ "type": "string", "x-extensions": extensions }),
    };
    v["title"] = json!(p.label);
    v["description"] = json!(param_words(p));
    v["x-section"] = json!(section_id(p.section));
    if p.drivable {
        v["x-drivable"] = json!(true);
    }
    v
}

// ------------------------------------------------------------- catalogue --

fn port_json(p: &PortSpec) -> Value {
    let mut v = json!({ "name": p.name, "label": p.label, "type": p.kind.id(), "help": p.help });
    if p.optional {
        v["optional"] = json!(true);
        if !p.default.is_empty() {
            v["left_empty"] = json!(p.default);
        }
    }
    v
}

fn param_json(p: &ParamSpec) -> Value {
    let mut v = json!({ "name": p.name, "label": p.label, "kind": param_kind(p), "help": param_words(p), "section": section_id(p.section) });
    if let Some(d) = param_default(p) {
        v["default"] = d;
    }
    match p.kind {
        ParamKind::Float { min, max, soft, unit, .. } => {
            v["unit"] = json!(unit.id());
            v["min"] = num(min).unwrap_or(Value::Null);
            v["max"] = num(max).unwrap_or(Value::Null);
            v["usual"] = json!([soft[0], soft[1]]);
        }
        ParamKind::Int { min, max, .. } => {
            v["min"] = json!(min);
            v["max"] = json!(max);
        }
        ParamKind::Choice { options, .. } => v["options"] = Value::Array(options.iter().map(|o| json!({ "value": o.value, "label": o.label, "help": o.help })).collect()),
        ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. } => {
            v["fields"] = Value::Array(fields.iter().map(|f| json!({ "name": f.name, "label": f.label, "unit": f.unit.id(), "default": f.default, "min": f.min, "max": f.max, "help": f.help })).collect());
            if let ParamKind::Points { max, .. } = p.kind {
                v["max_points"] = json!(max);
            }
        }
        ParamKind::File { extensions } => v["extensions"] = json!(extensions),
        _ => {}
    }
    if p.drivable {
        v["drivable"] = json!(true);
    }
    v
}

fn node_json(s: &'static NodeSpec) -> Value {
    let mut v = json!({
        "type": s.type_name(),
        "label": s.label,
        "summary": s.summary,
        "help": s.help,
        "keywords": s.keywords,
        "inputs": s.inputs.iter().map(port_json).collect::<Vec<_>>(),
        "outputs": s.outputs.iter().map(port_json).collect::<Vec<_>>(),
        "settings": s.params.iter().map(param_json).collect::<Vec<_>>(),
        "example": serde_json::to_value(example(s)).expect("graphs serialise"),
    });
    if let Some(open) = &s.open_inputs {
        v["open_inputs"] = port_json(open);
    }
    if let Some(open) = &s.open_masks {
        v["open_masks"] = port_json(open);
    }
    if matches!(s.kernel, Kernel::Global { .. }) {
        v["simulates"] = json!(true);
    }
    if matches!(s.kernel, Kernel::Geometry { .. }) {
        v["whole"] = json!(true);
    }
    match s.reads {
        Reads::Map => v["reads"] = json!("map"),
        Reads::Files => v["reads"] = json!("files"),
        Reads::Nothing => {}
    }
    v
}

/// The catalogue as JSON: the guide, the sockets, and every type of node by
/// family, with its inputs, outputs, settings and an example graph.
pub fn catalog_json() -> Value {
    json!({
        "format": FORMAT,
        "version": VERSION,
        "guide": GUIDE,
        "sockets": PortKind::ALL.iter().map(|k| json!({
            "type": k.id(),
            "carries": kind_words(*k),
            "feeds": PortKind::ALL.iter().filter(|to| k.conversion(**to).is_some()).map(|to| to.id()).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "families": Family::ALL.iter().map(|f| json!({
            "family": f.label(),
            "nodes": current().filter(|s| s.family == *f).map(node_json).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

/// JSON on one line, spaced for reading: `{ "a": [1, 2] }`.
fn inline(v: &Value) -> String {
    match v {
        Value::Object(o) if o.is_empty() => "{}".to_owned(),
        Value::Object(o) => format!("{{ {} }}", o.iter().map(|(k, v)| format!("{}: {}", Value::String(k.clone()), inline(v))).collect::<Vec<_>>().join(", ")),
        Value::Array(a) => format!("[{}]", a.iter().map(inline).collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}

fn node_line(n: &Node) -> String {
    let mut parts = vec![format!("\"type\": {}", Value::String(n.kind.clone()))];
    if !n.inputs.is_empty() {
        parts.push(format!("\"inputs\": {}", inline(&json!(n.inputs))));
    }
    if !n.params.is_empty() {
        parts.push(format!("\"params\": {}", inline(&json!(n.params))));
    }
    if let Some(note) = &n.note {
        parts.push(format!("\"note\": {}", Value::String(note.clone())));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// A graph as the catalogue shows it: one node a line.
pub fn compact_json(g: &Graph) -> String {
    let mut parts = Vec::new();
    if g.terrain != Default::default() {
        parts.push(format!("  \"terrain\": {}", inline(&serde_json::to_value(&g.terrain).expect("serialises"))));
    }
    let nodes: Vec<String> = g.nodes.iter().map(|(id, n)| format!("    {}: {}", Value::String(id.clone()), node_line(n))).collect();
    parts.push(format!("  \"nodes\": {{\n{}\n  }}", nodes.join(",\n")));
    format!("{{\n{}\n}}", parts.join(",\n"))
}

/// The catalogue as Markdown, for people and models alike.
pub fn catalog_markdown() -> String {
    let mut md = String::new();
    md.push_str("# Terrain graph nodes\n\n<!-- Written by massif-terrain (format::catalog_markdown); do not edit by hand: `MASSIF_WRITE_DOCS=1 cargo test -p massif-terrain --test format` writes it again. -->\n\n");
    md.push_str("Every type of node a terrain graph of Massif can use, with its inputs, outputs, settings and an example. The JSON Schema of graphs is `terrain-graph.schema.json`, beside this file.\n\n## How a graph is built\n\n");
    md.push_str(GUIDE);
    md.push_str("\n## Sockets\n\n| Type | Carries | Feeds |\n|---|---|---|\n");
    for k in PortKind::ALL {
        let feeds: Vec<String> = PortKind::ALL.iter().filter(|to| k.conversion(**to).is_some()).map(|to| format!("`{}`", to.id())).collect();
        let _ = writeln!(md, "| `{}` | {} | {} |", k.id(), kind_words(k), feeds.join(", "));
    }
    for f in Family::ALL {
        let _ = write!(md, "\n## {}\n", f.label());
        for s in current().filter(|s| s.family == f) {
            node_markdown(&mut md, s);
        }
    }
    md
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn ports_markdown(md: &mut String, title: &str, ports: &[PortSpec]) {
    if ports.is_empty() {
        return;
    }
    let _ = writeln!(md, "\n{title}:\n");
    for p in ports {
        let mut line = format!("- `{}` ({}{}) — {}", p.name, p.kind.id(), if p.optional { ", optional" } else { "" }, p.help);
        if p.optional && !p.default.is_empty() {
            let _ = write!(line, " Left empty: {}.", p.default);
        }
        let _ = writeln!(md, "{line}");
    }
}

fn node_markdown(md: &mut String, s: &'static NodeSpec) {
    let _ = writeln!(md, "\n### {} — `{}`\n\n{} {}", s.label, s.type_name(), s.summary, s.help);
    if matches!(s.kernel, Kernel::Global { .. }) {
        md.push_str("\nA simulation: it works over the whole terrain at the simulations' resolution.\n");
    }
    if matches!(s.kernel, Kernel::Geometry { .. }) {
        md.push_str("\nIt makes geometry over the whole terrain at once, reading reliefs and masks at the simulations' resolution.\n");
    }
    match s.reads {
        Reads::Map => md.push_str("\nIt reads the map as it is, and follows it when it changes.\n"),
        Reads::Files => md.push_str("\nIt reads the files its settings name, and follows them when they change.\n"),
        Reads::Nothing => {}
    }
    ports_markdown(md, "Inputs", s.inputs);
    for open in s.open_families() {
        let _ = writeln!(md, "- `{}<name>` ({}, optional) — {}", open.name, open.kind.id(), open.help);
    }
    ports_markdown(md, "Outputs", s.outputs);
    if !s.params.is_empty() {
        md.push_str("\n| Setting | Kind | Default | Range | What it does |\n|---|---|---|---|---|\n");
        for p in s.params {
            let default = match param_default(p) {
                Some(Value::String(s)) => format!("`{s}`"),
                Some(Value::Array(a)) if a.is_empty() => "none".to_owned(),
                Some(v @ Value::Array(_)) => format!("`{v}`"),
                Some(Value::Number(n)) => show(n.as_f64().unwrap_or(0.0)),
                Some(v) => v.to_string(),
                None => "—".to_owned(),
            };
            let range = match p.kind {
                ParamKind::Float { min, max, unit, .. } => {
                    let u = unit_words(unit);
                    format!("{} to {}{}{}", show(min), show(max), if u.is_empty() { "" } else { " " }, u)
                }
                ParamKind::Int { min, max, .. } => format!("{min} to {max}"),
                ParamKind::Choice { options, .. } => options.iter().map(|o| format!("`{}`", o.value)).collect::<Vec<_>>().join(", "),
                ParamKind::Points { fields, max } => format!("up to {max}; each `x`, `z`{}", fields.iter().map(|f| format!(", `{}`", f.name)).collect::<String>()),
                ParamKind::Curve { fields, .. } => format!("each `x`, `z`{}", fields.iter().map(|f| format!(", `{}`", f.name)).collect::<String>()),
                ParamKind::File { extensions } => extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", "),
                ParamKind::Seed => "0 to 4294967295".to_owned(),
                _ => String::new(),
            };
            let mut words = param_words(p);
            if let ParamKind::Choice { options, .. } = p.kind {
                for o in options {
                    let _ = write!(words, " `{}`: {}", o.value, o.help);
                }
            }
            let _ = writeln!(md, "| `{}` | {} | {} | {} | {} |", p.name, param_kind(p), cell(&default), cell(&range), cell(&words));
        }
    }
    let _ = writeln!(md, "\n```json\n{}\n```", compact_json(&example(s)));
}

// -------------------------------------------------------------- examples --

/// A node feeding what a socket of `kind` takes, in an example.
fn feeder(g: &mut Graph, kind: PortKind) -> String {
    let relief = || Node::new("noise.fbm").with("scale", 1500).with("height", 250);
    if !g.nodes.contains_key("relief") {
        g.nodes.insert("relief".into(), relief());
    }
    let fed = |id: &str| {
        let s = registry::spec(id).expect("a type of the registry");
        let mut n = Node::new(id);
        for p in s.inputs.iter().filter(|p| !p.optional) {
            n.inputs.insert(p.name.into(), "relief".into());
        }
        n
    };
    let (name, node) = match kind {
        PortKind::Mask => ("slopes", fed("mask.slope")),
        PortKind::Color => ("colours", fed("color.gradient")),
        PortKind::Terrain => ("terrains", Node::new("terrain.layers")),
        PortKind::Points => ("places", fed("points.scatter").input("height", "relief").with("spacing", 256)),
        // A village's stages feed one another (they need a pack to make
        // anything, which an example has not: they only show the wiring).
        PortKind::Splines => {
            let places = feeder(g, PortKind::Points);
            ("streets", Node::new("village.roads").input("height", "relief").input("squares", &places))
        }
        PortKind::Footprints => {
            let (places, streets) = (feeder(g, PortKind::Points), feeder(g, PortKind::Splines));
            ("lots", Node::new("village.plots").input("squares", &places).input("roads", &streets))
        }
        PortKind::Instances => {
            let (places, streets, lots) = (feeder(g, PortKind::Points), feeder(g, PortKind::Splines), feeder(g, PortKind::Footprints));
            ("houses", Node::new("village.buildings").input("height", "relief").input("squares", &places).input("roads", &streets).input("plots", &lots))
        }
        PortKind::Density => ("ground", fed("volume.from_height")),
        PortKind::Layer => ("caves", Node::new("caves.layer")),
        _ => return "relief".into(),
    };
    g.nodes.entry(name.into()).or_insert(node);
    name.into()
}

/// Values for settings drawn on the terrain, so that an example shows
/// something.
fn sample(p: &ParamSpec) -> Option<Value> {
    match p.kind {
        ParamKind::Points { .. } => Some(json!([{ "x": 0.35, "z": 0.4 }, { "x": 0.65, "z": 0.6 }])),
        ParamKind::Curve { closed: false, .. } => Some(json!([{ "x": 0.2, "z": 0.55 }, { "x": 0.5, "z": 0.45 }, { "x": 0.8, "z": 0.5 }])),
        ParamKind::Curve { closed: true, .. } => Some(json!([{ "x": 0.5, "z": 0.25 }, { "x": 0.74, "z": 0.42 }, { "x": 0.65, "z": 0.7 }, { "x": 0.35, "z": 0.7 }, { "x": 0.26, "z": 0.42 }])),
        ParamKind::File { extensions } => Some(json!(if extensions.contains(&"png") { "heightmap.png".to_owned() } else { format!("file.{}", extensions.first().copied().unwrap_or("dat")) })),
        _ => None,
    }
}

/// A small graph using a node of type `spec`, as the catalogue shows it:
/// what it needs fed by a noise, its result to the Map Output.
pub fn example(spec: &'static NodeSpec) -> Graph {
    let mut g = Graph::default();
    g.terrain.size = Some(4096);
    let mut out = Node::new("output.map");
    if spec.is_output() {
        let relief = feeder(&mut g, PortKind::Height);
        let colours = feeder(&mut g, PortKind::Color);
        let slopes = feeder(&mut g, PortKind::Mask);
        out = out.input("height", &relief).input("albedo", &colours).input("mask.rock", &slopes);
        g.nodes.insert("out".into(), out);
        return g;
    }
    let mut node = Node::new(&spec.type_name());
    for p in spec.params {
        if let Some(v) = sample(p) {
            node.params.insert(p.name.into(), v);
        }
    }
    for port in spec.inputs.iter().filter(|p| !p.optional) {
        node.inputs.insert(port.name.into(), feeder(&mut g, port.kind));
    }
    // A node whose inputs grow: two of them, the second through a mask.
    if spec.grows()
        && let Some(open) = spec.open_inputs
    {
        for k in 1..=2 {
            node.inputs.insert(format!("{}{k}", open.name), feeder(&mut g, open.kind));
        }
        if let Some(mask) = spec.open_masks {
            node.inputs.insert(format!("{}2", mask.name), feeder(&mut g, mask.kind));
        }
    }
    let stem = spec.id.rsplit('.').next().unwrap_or("node");
    let name = if g.nodes.contains_key(stem) || stem == "out" { format!("{stem}_1") } else { stem.to_owned() };
    let first = &spec.outputs[0];
    let map = registry::spec("output.map").expect("the Map Output");
    let into = match map.inputs.iter().find(|i| i.name == first.name && first.kind.conversion(i.kind).is_some()) {
        Some(i) => Some(i.name.to_owned()),
        None => match first.kind {
            PortKind::Height => Some("height".to_owned()),
            PortKind::Mask => Some(format!("mask.{}", first.name)),
            PortKind::Color => Some("albedo".to_owned()),
            PortKind::Terrain => Some("terrain".to_owned()),
            PortKind::Biome => Some("biome".to_owned()),
            PortKind::Instances => Some("structures".to_owned()),
            PortKind::Density => Some("volume.carve".to_owned()),
            PortKind::Layer if spec.id == "caves.biome" => Some("cave_biomes".to_owned()),
            PortKind::Layer => Some("caves".to_owned()),
            _ => None,
        },
    };
    if let Some(into) = &into {
        out.inputs.insert(into.clone(), name.clone());
    }
    if into.as_deref() != Some("height") {
        out.inputs.insert("height".into(), feeder(&mut g, PortKind::Height));
    }
    g.nodes.insert(name, node);
    g.nodes.insert("out".into(), out);
    g
}

// -------------------------------------------------------------- checking --

/// Keys of `o` that are not among `known`, as errors with the name probably
/// meant.
fn unknown_keys(o: &Map<String, Value>, path: &str, known: &[&str], what: &str, diags: &mut Vec<Diagnostic>) {
    for key in o.keys().filter(|k| !known.contains(&k.as_str())) {
        let hint = closest(key, known.iter().copied()).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
        let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
        diags.push(error(here, format!("{what} has no field “{key}”{hint}")));
    }
}

/// What is wrong with the shape of a graph's JSON, before its nodes are
/// looked at.
fn shape(v: &Value) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let Some(top) = v.as_object() else {
        return vec![error("", "a terrain graph is a JSON object")];
    };
    unknown_keys(top, "", &["format", "version", "terrain", "nodes", "ui", "frames", "components", "editor"], "a graph", &mut d);
    if let Some(f) = top.get("format")
        && f.as_str() != Some(FORMAT)
    {
        d.push(error("format", format!("a terrain graph's format is “{FORMAT}”, not {f}")));
    }
    if let Some(ver) = top.get("version") {
        match ver.as_u64() {
            Some(n) if n <= VERSION as u64 => {}
            Some(n) => d.push(error("version", format!("version {n} is newer than this Massif reads ({VERSION})"))),
            None => d.push(error("version", format!("the version is a whole number, not {ver}"))),
        }
    }
    if let Some(t) = top.get("terrain") {
        match t.as_object() {
            Some(o) => {
                unknown_keys(o, "terrain", &["size", "seed", "sea_level"], "the terrain", &mut d);
                if o.get("size").is_some_and(|s| s.as_u64().is_none_or(|s| s == 0 || s > u32::MAX as u64)) {
                    d.push(error("terrain.size", "the size is a whole number of blocks, from 1"));
                }
                if o.get("seed").is_some_and(|s| s.as_u64().is_none()) {
                    d.push(error("terrain.seed", "the seed is a whole number, from 0"));
                }
                if o.get("sea_level").is_some_and(|s| s.as_f64().is_none()) {
                    d.push(error("terrain.sea_level", "the sea level is a number of blocks"));
                }
            }
            None => d.push(error("terrain", "the terrain is an object: { \"size\": 8192, \"seed\": 1 }")),
        }
    }
    match top.get("nodes").map(Value::as_object) {
        None => {}
        Some(None) => d.push(error("nodes", "the nodes are an object, by name: { \"hills\": { \"type\": \"noise.fbm\" } }")),
        Some(Some(nodes)) => {
            for (id, node) in nodes {
                let path = format!("nodes.{id}");
                if !valid_id(id) {
                    d.push(error(path.clone(), "a node's name is made of letters, digits, “_” and “-”, at most 64"));
                }
                let Some(o) = node.as_object() else {
                    d.push(error(path, "a node is an object: { \"type\": \"noise.fbm\" }"));
                    continue;
                };
                unknown_keys(o, &path, &["type", "inputs", "params", "note", "name", "bypass"], "a node", &mut d);
                match o.get("type") {
                    None => d.push(error(format!("{path}.type"), "a node needs a type")),
                    Some(t) if !t.is_string() => d.push(error(format!("{path}.type"), "a node's type is a name, as “noise.fbm”")),
                    _ => {}
                }
                match o.get("inputs").map(Value::as_object) {
                    None => {}
                    Some(None) => d.push(error(format!("{path}.inputs"), "the inputs are an object: { \"height\": \"hills\" }")),
                    Some(Some(ins)) => {
                        for (k, s) in ins.iter().filter(|(_, s)| !s.is_string()) {
                            d.push(error(format!("{path}.inputs.{k}"), format!("an input names the node feeding it, as “hills” or “hills.flow”, not {s}")));
                        }
                    }
                }
                if o.get("params").is_some_and(|p| !p.is_object()) {
                    d.push(error(format!("{path}.params"), "the settings are an object: { \"height\": 300 }"));
                }
                if o.get("note").is_some_and(|n| !n.is_string()) {
                    d.push(error(format!("{path}.note"), "a note is text"));
                }
                if o.get("name").is_some_and(|n| !n.is_string()) {
                    d.push(error(format!("{path}.name"), "a node's name is text"));
                }
                if o.get("bypass").is_some_and(|n| !n.is_boolean()) {
                    d.push(error(format!("{path}.bypass"), "bypass is true or false"));
                }
            }
        }
    }
    match top.get("ui").map(Value::as_object) {
        None => {}
        Some(None) => d.push(error("ui", "the layout is an object, by node: { \"hills\": { \"pos\": [0, 0] } }")),
        Some(Some(ui)) => {
            for (id, at) in ui {
                let path = format!("ui.{id}");
                match at.as_object() {
                    Some(o) => {
                        unknown_keys(o, &path, &["pos", "collapsed"], "a node's place", &mut d);
                        if o.get("pos").and_then(Value::as_array).is_none_or(|p| p.len() != 2 || !p.iter().all(Value::is_number)) {
                            d.push(error(format!("{path}.pos"), "a node's place is two numbers, [x, y]"));
                        }
                    }
                    None => d.push(error(path, "a node's place is an object: { \"pos\": [0, 0] }")),
                }
            }
        }
    }
    d
}

/// What compiling a graph finds, with no map and no files.
pub fn check(graph: &Graph) -> Vec<Diagnostic> {
    compile(graph, Space::new(4096), 1024).1
}

/// Everything wrong with a graph's text: its JSON, its shape, then its
/// nodes, inputs and settings, each with its path. A graph with no errors
/// can be built; warnings say what was taken otherwise.
pub fn validate(text: &str) -> Vec<Diagnostic> {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return vec![error("", format!("this is not JSON: {e}"))],
    };
    let mut diags = shape(&value);
    if diags.iter().any(is_error) {
        return diags;
    }
    match serde_json::from_value::<Graph>(value) {
        Ok(mut g) => {
            g.upgrade();
            diags.extend(check(&g))
        }
        Err(e) => diags.push(error("", e.to_string())),
    }
    diags
}

// --------------------------------------------------------------- editing --

/// One step of an edit, as a model writes it, for example
/// `{ "op": "add", "type": "noise.fbm", "id": "hills", "params": { "scale": 900 } }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Adds a node, named after its type when `id` is left out.
    Add {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "type")]
        kind: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, Value>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        inputs: BTreeMap<String, String>,
    },
    /// Sets a setting of a node; `null` gives it back its default.
    Set { node: String, param: String, value: Value },
    /// Feeds input `input` of node `to` from `from` (`node` or `node.output`).
    Link { from: String, to: String, input: String },
    /// Leaves an input empty.
    Unlink { to: String, input: String },
    /// Removes a node, and the links from it.
    Remove { node: String },
    /// Gives a node the name the editor shows (`null`: its type's); its id,
    /// by which the others refer to it, never changes.
    Rename { node: String, to: Option<String> },
    /// Leaves a node out (`true`) or puts it back: what it feeds takes its
    /// first input instead.
    Bypass { node: String, on: bool },
    /// Adds a frame round nodes, or changes one (`id` given): its title,
    /// colour (`#rrggbb`), note, nodes and the frame it sits in.
    Frame {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        description: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        members: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
    /// Removes a frame, leaving its nodes (and the frames in it, which go
    /// to its own parent).
    Unframe { id: String },
    /// Gathers nodes into a new composite of the document, one node of it
    /// in their place.
    Group { nodes: Vec<String>, label: String },
    /// Puts a composite node's nodes back in its place.
    Ungroup { node: String },
    /// Shows a setting on a composite of the document, standing for
    /// settings of its nodes (`targets`: `node.setting`).
    Expose {
        component: String,
        #[serde(flatten)]
        exposed: crate::model::Exposed,
    },
    /// Takes an exposed setting off a composite of the document.
    Unexpose { component: String, name: String },
    /// Edits inside a composite of the document: the same steps, on its
    /// nodes (`in.<name>` names its inputs).
    Inside { component: String, ops: Vec<Op> },
    /// Adds an input to a composite of the document, or changes the one of
    /// that name.
    Port { component: String, decl: crate::model::PortDecl },
    /// Takes an input off a composite of the document, and the links from
    /// it inside.
    Unport { component: String, name: String },
    /// Adds an output to a composite of the document (`from`: `node` or
    /// `node.output` inside), or changes the one of that name.
    Output { component: String, decl: crate::model::OutputDecl },
    /// Takes an output off a composite of the document, and what read it.
    Unoutput { component: String, name: String },
    /// Edit a copy: the shipped composite of node `node` (`massif:…`)
    /// copied into the document as a new `project:` composite labelled
    /// `label`, the node now of that type, its settings kept. Gives the
    /// new type.
    Fork { node: String, label: String },
}

fn no_node(graph: &Graph, id: &str) -> String {
    let hint = closest(id, graph.nodes.keys().map(String::as_str)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
    format!("the graph has no node “{id}”{hint}")
}

/// Whether `kind` names a composite this document or Massif has.
fn composite_known(graph: &Graph, kind: &str) -> Result<(), String> {
    match crate::lower::component(graph, kind) {
        Some(_) => Ok(()),
        None => Err(format!("there is no composite “{kind}”")),
    }
}

fn type_of(kind: &str) -> Result<&'static NodeSpec, String> {
    match registry::spec(kind).filter(|s| s.family != Family::Internal) {
        Some(s) => Ok(s),
        None => {
            let hint = closest(kind, public().map(|s| s.id)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
            Err(format!("there is no type of node “{kind}”{hint}"))
        }
    }
}

/// Applies one step: gives the node it named or added, or why it cannot be
/// done, the graph then unchanged.
pub fn apply(graph: &mut Graph, op: &Op) -> Result<String, String> {
    apply_in(graph, op, &[])
}

/// [`apply`] inside a composite whose inputs are `inputs` (`in.<name>`).
fn apply_in(graph: &mut Graph, op: &Op, inputs: &[String]) -> Result<String, String> {
    match op {
        Op::Add { id, kind, params, inputs } => {
            if crate::model::is_component_type(kind) {
                composite_known(graph, kind)?;
            } else {
                type_of(kind)?;
            }
            let node = Node { kind: kind.clone(), inputs: inputs.clone(), params: params.clone(), ..Node::new("") };
            match id {
                Some(id) if !valid_id(id) => Err(format!("“{id}” cannot name a node: letters, digits, “_” and “-”, at most 64")),
                Some(id) if graph.nodes.contains_key(id) => Err(format!("the graph has a node “{id}” already")),
                Some(id) => {
                    graph.nodes.insert(id.clone(), node);
                    Ok(id.clone())
                }
                None => Ok(graph.add(node)),
            }
        }
        Op::Set { node, param, value } => {
            if !graph.nodes.contains_key(node) {
                return Err(no_node(graph, node));
            }
            if let Some(c) = graph.nodes.get(node).filter(|n| n.is_component()).and_then(|n| crate::lower::component(graph, &n.kind))
                && !c.exposed.iter().any(|e| &e.name == param)
            {
                let hint = closest(param, c.exposed.iter().map(|e| e.name.as_str())).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                return Err(format!("{} shows no setting “{param}”{hint}", c.label));
            }
            let n = graph.nodes.get_mut(node).expect("checked");
            if let Ok(spec) = type_of(&n.kind)
                && spec.param(param).is_none()
            {
                let hint = closest(param, spec.params.iter().map(|p| p.name)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                return Err(format!("{} has no setting “{param}”{hint}", spec.id));
            }
            if value.is_null() {
                n.params.remove(param);
            } else {
                n.params.insert(param.clone(), value.clone());
            }
            Ok(node.clone())
        }
        Op::Link { from, to, input } => {
            let src = Source::parse(from);
            let inside = src.node == crate::model::INPUTS && src.output.is_some_and(|o| inputs.iter().any(|i| i == o));
            if src.node == crate::model::INPUTS && !inside && !inputs.is_empty() {
                return Err(format!("the composite has no input “{}”", src.output.unwrap_or("")));
            }
            let feeding = graph.nodes.get(src.node);
            if feeding.is_none() && !inside {
                return Err(no_node(graph, src.node));
            }
            if let (Some(Ok(spec)), Some(port)) = (feeding.map(|f| type_of(&f.kind)), src.output)
                && spec.output(port).is_none()
            {
                let hint = closest(port, spec.outputs.iter().map(|p| p.name)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                return Err(format!("{} has no output “{port}”{hint}", spec.id));
            }
            let Some(n) = graph.nodes.get(to) else { return Err(no_node(graph, to)) };
            if let Ok(spec) = type_of(&n.kind) {
                let driven = input.strip_prefix('@').is_some_and(|p| spec.param(p).is_some_and(|(_, p)| p.drivable));
                if spec.input(input).is_none() && spec.open_input(input).is_none() && !driven {
                    let names = spec.inputs.iter().map(|p| p.name.to_owned()).chain(spec.params.iter().filter(|p| p.drivable).map(|p| format!("@{}", p.name))).collect::<Vec<_>>();
                    let hint = closest(input, names.iter().map(String::as_str)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                    return Err(format!("{} has no input “{input}”{hint}", spec.id));
                }
            }
            graph.link(from, to, input);
            Ok(to.clone())
        }
        Op::Unlink { to, input } => {
            if !graph.nodes.contains_key(to) {
                return Err(no_node(graph, to));
            }
            let n = graph.nodes.get_mut(to).expect("checked");
            match n.inputs.remove(input) {
                Some(_) => Ok(to.clone()),
                None => Err(format!("nothing feeds “{to}”'s input “{input}”")),
            }
        }
        Op::Remove { node } => {
            if graph.nodes.remove(node).is_none() {
                return Err(no_node(graph, node));
            }
            graph.ui.remove(node);
            for n in graph.nodes.values_mut() {
                n.inputs.retain(|_, s| Source::parse(s).node != node.as_str());
            }
            for f in graph.frames.values_mut() {
                f.members.retain(|m| m != node);
            }
            Ok(node.clone())
        }
        Op::Rename { node, to } => {
            let Some(n) = graph.nodes.get_mut(node) else { return Err(no_node(graph, node)) };
            n.name = to.clone().filter(|s| !s.trim().is_empty());
            Ok(node.clone())
        }
        Op::Bypass { node, on } => {
            let Some(n) = graph.nodes.get_mut(node) else { return Err(no_node(graph, node)) };
            n.bypass = *on;
            Ok(node.clone())
        }
        Op::Frame { id, title, color, description, members, parent } => {
            if let Some(m) = members.iter().find(|m| !graph.nodes.contains_key(*m)) {
                return Err(no_node(graph, m));
            }
            if let Some(c) = color
                && crate::params::parse_color(c).is_none()
            {
                return Err(format!("“{c}” is not a colour: #rrggbb"));
            }
            let id = match id {
                Some(id) if !valid_id(id) => return Err(format!("“{id}” cannot name a frame: letters, digits, “_” and “-”")),
                Some(id) => id.clone(),
                None => {
                    let mut k = graph.frames.len() + 1;
                    while graph.frames.contains_key(&format!("frame_{k}")) {
                        k += 1;
                    }
                    format!("frame_{k}")
                }
            };
            if let Some(p) = parent {
                // A frame sits in a frame that exists, and never in itself.
                let mut at = Some(p.clone());
                while let Some(f) = at {
                    if f == id {
                        return Err(format!("frame “{id}” cannot sit in itself"));
                    }
                    let Some(fr) = graph.frames.get(&f) else { return Err(format!("the graph has no frame “{f}”")) };
                    at = fr.parent.clone();
                }
            }
            let f = graph.frames.entry(id.clone()).or_default();
            f.title = title.clone();
            f.color = color.clone();
            f.description = description.clone();
            f.members = members.clone();
            f.parent = parent.clone();
            Ok(id)
        }
        Op::Unframe { id } => {
            let Some(f) = graph.frames.remove(id) else { return Err(format!("the graph has no frame “{id}”")) };
            for g in graph.frames.values_mut().filter(|g| g.parent.as_deref() == Some(id)) {
                g.parent = f.parent.clone();
            }
            Ok(id.clone())
        }
        Op::Group { nodes, label } => crate::group::group(graph, nodes, label).map(|(id, _)| id),
        Op::Ungroup { node } => crate::group::ungroup(graph, node).map(|_| node.clone()),
        Op::Expose { component, exposed } => crate::group::expose(graph, component, exposed.clone()).map(|_| component.clone()),
        Op::Unexpose { component, name } => {
            let Some(c) = graph.components.get_mut(component) else { return Err(format!("the document has no composite “{component}”")) };
            let before = c.exposed.len();
            c.exposed.retain(|e| &e.name != name);
            if c.exposed.len() == before {
                return Err(format!("{} shows no setting “{name}”", c.label));
            }
            Ok(component.clone())
        }
        Op::Inside { component, ops } => {
            let Some(c) = graph.components.get(component) else { return Err(format!("the document has no composite “{component}” (those of Massif are edited as a copy)")) };
            // Its nodes as a graph of their own (the document's composites
            // known), edited, then put back: all the steps, or none.
            let mut inner = Graph { nodes: c.nodes.clone(), ui: c.ui.clone(), frames: c.frames.clone(), components: graph.components.clone(), ..Graph::default() };
            let names: Vec<String> = c.inputs.iter().map(|p| p.name.clone()).collect();
            for (k, op) in ops.iter().enumerate() {
                apply_in(&mut inner, op, &names).map_err(|e| format!("step {k}: {e}"))?;
            }
            let c = graph.components.get_mut(component).expect("checked");
            c.nodes = inner.nodes;
            c.ui = inner.ui;
            c.frames = inner.frames;
            Ok(component.clone())
        }
        Op::Port { component, decl } => {
            let c = own_component(graph, component)?;
            if !valid_id(&decl.name) {
                return Err(format!("“{}” cannot name an input: letters, digits, “_” and “-”", decl.name));
            }
            if PortKind::from_id(&decl.kind).is_none() {
                let hint = closest(&decl.kind, PortKind::ALL.iter().map(|k| k.id())).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                return Err(format!("“{}” is not a type of socket{hint}", decl.kind));
            }
            match c.inputs.iter_mut().find(|p| p.name == decl.name) {
                Some(p) => *p = decl.clone(),
                None => c.inputs.push(decl.clone()),
            }
            Ok(component.clone())
        }
        Op::Unport { component, name } => {
            let c = own_component(graph, component)?;
            let before = c.inputs.len();
            c.inputs.retain(|p| &p.name != name);
            if c.inputs.len() == before {
                return Err(format!("{} has no input “{name}”", c.label));
            }
            let gone = format!("{}.{name}", crate::model::INPUTS);
            for n in c.nodes.values_mut() {
                n.inputs.retain(|_, s| *s != gone);
            }
            for n in graph.nodes.values_mut().filter(|n| &n.kind == component) {
                n.inputs.remove(name);
            }
            Ok(component.clone())
        }
        Op::Output { component, decl } => {
            let c = own_component(graph, component)?;
            if !valid_id(&decl.name) {
                return Err(format!("“{}” cannot name an output: letters, digits, “_” and “-”", decl.name));
            }
            let src = Source::parse(&decl.from);
            let made = c.nodes.contains_key(src.node) || (src.node == crate::model::INPUTS && src.output.is_some_and(|o| c.inputs.iter().any(|p| p.name == o)));
            if !made {
                return Err(format!("{} has no node “{}” to give its output", c.label, src.node));
            }
            match c.outputs.iter_mut().find(|o| o.name == decl.name) {
                Some(o) => *o = decl.clone(),
                None => c.outputs.push(decl.clone()),
            }
            Ok(component.clone())
        }
        Op::Unoutput { component, name } => {
            let c = own_component(graph, component)?;
            let before = c.outputs.len();
            c.outputs.retain(|o| &o.name != name);
            if c.outputs.len() == before {
                return Err(format!("{} has no output “{name}”", c.label));
            }
            let instances: Vec<String> = graph.nodes.iter().filter(|(_, n)| &n.kind == component).map(|(id, _)| id.clone()).collect();
            for n in graph.nodes.values_mut() {
                n.inputs.retain(|_, s| {
                    let src = Source::parse(s);
                    !(instances.iter().any(|i| i == src.node) && src.output == Some(name.as_str()))
                });
            }
            Ok(component.clone())
        }
        Op::Fork { node, label } => {
            let Some(n) = graph.nodes.get(node) else { return Err(no_node(graph, node)) };
            if !n.kind.starts_with("massif:") {
                return Err(format!("“{node}” is not a composite shipped with Massif (those of the document are edited as they are)"));
            }
            let Some(mut copy) = crate::lower::component(graph, &n.kind).map(|c| c.into_owned()) else { return Err(format!("there is no composite “{}”", n.kind)) };
            let stem: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '_' }).collect();
            let stem = stem.trim_matches('_').replace("__", "_");
            let stem = if stem.is_empty() { "copy".to_owned() } else { stem };
            let mut kind = format!("project:{stem}");
            let mut k = 2;
            while graph.components.contains_key(&kind) {
                kind = format!("project:{stem}_{k}");
                k += 1;
            }
            copy.version = 1;
            copy.label = label.clone();
            graph.components.insert(kind.clone(), copy);
            graph.nodes.get_mut(node).expect("checked").kind = kind.clone();
            Ok(kind)
        }
    }
}

/// A composite of the document, to edit (those Massif ships are edited as
/// a copy, see [`Op::Fork`]).
fn own_component<'a>(graph: &'a mut Graph, kind: &str) -> Result<&'a mut crate::model::Component, String> {
    graph.components.get_mut(kind).ok_or_else(|| format!("the document has no composite “{kind}” (those of Massif are edited as a copy)"))
}

/// Applies steps in order, each that can be done; gives why each other
/// could not (at `ops.<k>`), then what checking the graph finds.
pub fn patch(graph: &mut Graph, ops: &[Op]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (k, op) in ops.iter().enumerate() {
        if let Err(why) = apply(graph, op) {
            diags.push(error(format!("ops.{k}"), why));
        }
    }
    diags.extend(check(graph));
    diags
}

/// Applies a JSON Patch (RFC 6902) to the whole graph: all of it, or nothing
/// when an operation fails or the result is not a graph. Gives what was
/// wrong, then what checking the graph finds.
pub fn json_patch(graph: &mut Graph, ops: &Value) -> Vec<Diagnostic> {
    let Some(list) = ops.as_array() else {
        return vec![error("", "a JSON Patch is a list of operations: [{ \"op\": \"replace\", \"path\": \"/nodes/hills/params/height\", \"value\": 300 }]")];
    };
    let mut doc = serde_json::to_value(&*graph).expect("graphs serialise");
    for (k, op) in list.iter().enumerate() {
        if let Err(why) = rfc6902(&mut doc, op) {
            return vec![error(format!("{k}"), why)];
        }
    }
    let diags = shape(&doc);
    if diags.iter().any(is_error) {
        return diags;
    }
    match serde_json::from_value::<Graph>(doc) {
        Ok(g) => {
            *graph = g;
            let mut diags = diags;
            diags.extend(check(graph));
            diags
        }
        Err(e) => vec![error("", e.to_string())],
    }
}

/// The tokens of a JSON Pointer (RFC 6901).
fn pointer(path: &str) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = path.strip_prefix('/') else { return Err(format!("“{path}” is not a JSON Pointer: it starts with “/”")) };
    Ok(rest.split('/').map(|t| t.replace("~1", "/").replace("~0", "~")).collect())
}

fn get<'a>(doc: &'a Value, tokens: &[String]) -> Option<&'a Value> {
    tokens.iter().try_fold(doc, |v, t| match v {
        Value::Object(o) => o.get(t),
        Value::Array(a) => t.parse::<usize>().ok().and_then(|i| a.get(i)),
        _ => None,
    })
}

fn get_mut<'a>(doc: &'a mut Value, tokens: &[String]) -> Option<&'a mut Value> {
    tokens.iter().try_fold(doc, |v, t| match v {
        Value::Object(o) => o.get_mut(t),
        Value::Array(a) => t.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
        _ => None,
    })
}

fn add(doc: &mut Value, tokens: &[String], value: Value) -> Result<(), String> {
    let Some((last, parent)) = tokens.split_last() else {
        *doc = value;
        return Ok(());
    };
    match get_mut(doc, parent) {
        Some(Value::Object(o)) => {
            o.insert(last.clone(), value);
            Ok(())
        }
        Some(Value::Array(a)) => {
            let i = if last == "-" { a.len() } else { last.parse::<usize>().map_err(|_| format!("“{last}” is not an index"))? };
            if i > a.len() {
                return Err(format!("index {i} is past the end of a list of {}", a.len()));
            }
            a.insert(i, value);
            Ok(())
        }
        _ => Err(format!("nothing at /{} to add to", parent.join("/"))),
    }
}

fn remove(doc: &mut Value, tokens: &[String]) -> Result<Value, String> {
    let Some((last, parent)) = tokens.split_last() else { return Err("the whole document cannot be removed".to_owned()) };
    let missing = || format!("nothing at /{} to remove", tokens.join("/"));
    match get_mut(doc, parent) {
        Some(Value::Object(o)) => o.remove(last).ok_or_else(missing),
        Some(Value::Array(a)) => match last.parse::<usize>() {
            Ok(i) if i < a.len() => Ok(a.remove(i)),
            _ => Err(missing()),
        },
        _ => Err(missing()),
    }
}

/// One operation of a JSON Patch.
fn rfc6902(doc: &mut Value, op: &Value) -> Result<(), String> {
    let field = |name: &str| op.get(name).and_then(Value::as_str).ok_or_else(|| format!("an operation needs “{name}”"));
    let value = || op.get("value").cloned().ok_or_else(|| "this operation needs “value”".to_owned());
    let path = pointer(field("path")?)?;
    match field("op")? {
        "add" => add(doc, &path, value()?),
        "remove" => remove(doc, &path).map(|_| ()),
        "replace" => match get_mut(doc, &path) {
            Some(v) => {
                *v = value()?;
                Ok(())
            }
            None => Err(format!("nothing at {} to replace", field("path")?)),
        },
        "move" => {
            let from = pointer(field("from")?)?;
            if path.len() > from.len() && path[..from.len()] == from[..] {
                return Err("a value cannot move into itself".to_owned());
            }
            let v = remove(doc, &from)?;
            add(doc, &path, v)
        }
        "copy" => {
            let v = get(doc, &pointer(field("from")?)?).cloned().ok_or_else(|| "nothing to copy there".to_owned())?;
            add(doc, &path, v)
        }
        "test" => match get(doc, &path) {
            Some(v) if *v == value()? => Ok(()),
            _ => Err(format!("the test of {} failed", field("path")?)),
        },
        other => Err(format!("“{other}” is not an operation of JSON Patch: add, remove, replace, move, copy, test")),
    }
}

/// Places for nodes that have none, in columns by depth from the sources,
/// for graphs a model wrote.
pub fn lay_out(graph: &mut Graph) {
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    fn depth_of(g: &Graph, id: &str, memo: &mut BTreeMap<String, usize>, seen: &mut Vec<String>) -> usize {
        if let Some(d) = memo.get(id) {
            return *d;
        }
        if seen.iter().any(|s| s == id) {
            return 0;
        }
        seen.push(id.to_owned());
        let d = g.nodes.get(id).map_or(0, |n| n.inputs.values().map(|s| depth_of(g, Source::parse(s).node, memo, seen) + 1).max().unwrap_or(0));
        seen.pop();
        memo.insert(id.to_owned(), d);
        d
    }
    let ids: Vec<String> = graph.nodes.keys().cloned().collect();
    for id in &ids {
        depth_of(graph, id, &mut depth, &mut Vec::new());
    }
    let mut rows: BTreeMap<usize, f32> = BTreeMap::new();
    for id in ids {
        let d = depth[&id];
        let row = rows.entry(d).or_insert(0.0);
        graph.ui.entry(id).or_insert(NodeUi { pos: [d as f32 * 260.0, *row], collapsed: false });
        *row += 160.0;
    }
}
