//! What a model (or a person) needs to write MassifScript: the language,
//! the catalogue of every node by its script name, examples, and the graph
//! open now. Written from the live registry, so it never goes stale.

use super::names;
use crate::model::Component;
use crate::registry::{Family, Kernel, NodeSpec, ParamKind, ParamSpec, PortKind, PortSpec, Unit};
use std::fmt::Write as _;

/// Complete scripts, each compiled by the tests.
pub const EXAMPLES: &[(&str, &str)] = &[
    (
        "Alpine mountains: noise, erosion, masks, groups and outputs",
        r##"project "Alpine Mountains" {
    seed = 4268;

    group "Base Terrain" {
        color = #347a52;
        base = FractalNoise(scale: 2600, height: 700, base: 40, octaves: 9);
        ridges = RidgedNoise(scale: 1400, height: 500, seed: 7);
        mountains = Add(base, ridges, strength: 0.8);
        lifted = Levels(mountains, in_low: 40, in_high: 1200, out_low: 30, out_high: 300, gamma: 1.3);
    }

    group "Geology" {
        color = #8a6a3a;
        warped = Warp(lifted);
        eroded = StreamErosion(warped);
    }

    group "Environment" {
        color = #3a6ea5;
        slope = SlopeMask(eroded);
        flat = InvertMask(slope);
    }

    output height = eroded;
    output mask.flat = flat;
}
"##,
    ),
    (
        "A component with exposed settings, used twice, and a loop",
        r##"project "Twin peaks" {
    seed = 12;

    component MountainGenerator(seed: seed = 42, height: float = 800, roughness: float = 0.5) {
        label = "Mountain generator";
        expose height { label: "Mountain height"; min: 0; max: 2000; unit: "blocks"; }
        expose roughness { label: "Roughness"; min: 0.05; max: 0.95; }

        base = FractalNoise(seed: seed, height: height, roughness: roughness);
        ridges = RidgedNoise(seed: seed, scale: 900, height: 300);
        terrain = Max(base, ridges);
        output terrain;
    }

    west = MountainGenerator(height: 900, seed: 1);
    east = MountainGenerator(height: 600, roughness: 0.7, seed: 2);
    both = Max(west, east);

    // Three terraced copies, each finer than the last: loops build several nodes.
    for i in 0..3 {
        step[i] = Terraces(both, steps: 4 + i * 4);
    }
    output height = step[2];
}
"##,
    ),
];

fn unit(u: Unit) -> &'static str {
    match u {
        Unit::None | Unit::Ratio => "",
        Unit::Blocks => " blocks",
        Unit::Fraction => " of the side",
        Unit::Degrees => "°",
        Unit::Count => "",
        Unit::Celsius => " °C",
        Unit::Percent => " %",
    }
}

fn num(v: f64) -> String {
    super::number(v)
}

/// A setting in one short phrase: `scale: float 2..65536 blocks = 1000`.
pub fn setting(p: &ParamSpec) -> String {
    let star = if p.drivable { "*" } else { "" };
    let kind = match p.kind {
        ParamKind::Float { min, max, default, unit: u, .. } => format!("float {}..{}{} = {}", num(min), num(max), unit(u), num(default)),
        ParamKind::Int { min, max, default } => format!("int {min}..{max} = {default}"),
        ParamKind::Bool { default } => format!("bool = {default}"),
        ParamKind::Choice { options, default } => format!("{} = \"{default}\"", options.iter().map(|o| format!("\"{}\"", o.value)).collect::<Vec<_>>().join("|")),
        ParamKind::Color { default } => format!("color = \"{default}\""),
        ParamKind::Seed => "seed (from the node's id when left out)".to_owned(),
        ParamKind::Points { fields, max } => format!("points [{{ x, z{} }}, …] (up to {max}; x, z from 0 to 1)", fields.iter().map(|f| format!(", {}", f.name)).collect::<String>()),
        ParamKind::Curve { fields, closed } => format!("{} [{{ x, z{} }}, …] (x, z from 0 to 1)", if closed { "closed curve" } else { "curve" }, fields.iter().map(|f| format!(", {}", f.name)).collect::<String>()),
        ParamKind::Profile { default } => format!("profile [[x, y], …] = {}", serde_json::to_string(default).unwrap_or_default()),
        ParamKind::Gradient { .. } => "gradient [{ at, color }, …]".to_owned(),
        ParamKind::Text { default } => format!("text = \"{default}\""),
        ParamKind::File { extensions } => format!("file ({})", extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", ")),
    };
    format!("{}{star}: {kind}", p.name)
}

fn port(p: &PortSpec) -> String {
    format!("{}{}: {}", p.name, if p.optional { "?" } else { "" }, p.kind.id())
}

/// One line for a type of node.
pub fn node_line(name: &str, s: &NodeSpec) -> String {
    let mut ins: Vec<String> = s.inputs.iter().map(port).collect();
    for o in s.open_families() {
        ins.push(format!("{}<name>?: {}", o.name, o.kind.id()));
    }
    let outs: Vec<String> = s.outputs.iter().map(port).collect();
    let mut line = format!("- {name}({}) -> {}", ins.join(", "), if outs.is_empty() { "the map".to_owned() } else { outs.join(", ") });
    let _ = write!(line, " — {}", s.summary);
    if matches!(s.kernel, Kernel::Global { .. }) {
        line.push_str(" (simulation)");
    }
    if !s.params.is_empty() {
        let _ = write!(line, " Settings: {}.", s.params.iter().map(setting).collect::<Vec<_>>().join("; "));
    }
    line
}

/// One line for a composite Massif ships.
pub fn component_line(name: &str, c: &Component) -> String {
    let ins: Vec<String> = c.inputs.iter().map(|p| format!("{}{}: {}", p.name, if p.optional { "?" } else { "" }, p.kind)).collect();
    let outs: Vec<String> = c.outputs.iter().map(|o| o.name.clone()).collect();
    let mut line = format!("- {name}({}) -> {} — {}", ins.join(", "), outs.join(", "), c.summary);
    if !c.exposed.is_empty() {
        let set: Vec<String> = c
            .exposed
            .iter()
            .map(|e| {
                let mut s = e.name.clone();
                if let (Some(a), Some(b)) = (e.min, e.max) {
                    let _ = write!(s, " {}..{}", num(a), num(b));
                }
                if let Some(d) = &e.default {
                    let _ = write!(s, " = {d}");
                }
                s
            })
            .collect();
        let _ = write!(line, " Settings: {}.", set.join("; "));
    }
    line
}

const LANGUAGE: &str = r##"MassifScript is the text form of Massif's terrain node graph. Running a script builds exactly the nodes, links, settings, groups (frames), composites, exposed settings and outputs the visual editor makes: the result opens as an editable graph. Loops, conditions and constants only decide what is built (they run once, when the script is applied).

## Grammar

- A script is `project "Name" { … }` holding statements. `seed = 4268;` sets the terrain's seed; `size = 8192;` its side in blocks (the map's when left out); `sea_level = 62;`.
- Comments: `// to the end of the line` and `/* between */`.
- A node: `name = NodeType(input, other_input, setting: value, …);`. Inputs come first, in the order the catalogue gives, or named (`mask: m`). Settings are always named. `name` becomes the node's id: other nodes use it as an input (`eroded`, or `eroded.flow` for its output `flow`). Settings left out take their defaults.
- Nodes can be written inside an argument: `forest = Blend(a, b, mask: InvertMask(slope));`.
- Values: numbers with arithmetic (`2400 * 1.5`, `-3`, `(a + b) / 2`, `^`, `%`), functions `min max abs sqrt floor ceil round pow clamp lerp sin cos`, `pi`; texts `"ridged"`; booleans `true`/`false`; colours `#347a52`; lists `[1, 2]`; objects `{ x: 0.3, z: 0.6 }` (for points and curves, positions are fractions of the terrain from 0 to 1).
- A setting marked * in the catalogue can follow a field: give it a node (`height: mask_node`).
- `const k = 3;` and `let x = 1;` (`x = x + 1;` changes a `let`). `if cond { … } else { … }`, `for i in 0..4 { … }`, `for v in [100, 300] { … }`. In a loop, name nodes with an index: `peak[i] = …;` makes `peak_0`, `peak_1`…, used as `peak[i - 1]`.
- `group "Title" { color = #347a52; description = "…"; … }` frames the nodes written inside; groups nest.
- A node may end with attributes: `x = Blur(a) { name: "Soft hills"; note: "…"; bypass: true; };`.
- Outputs of the map: `output height = eroded;` and likewise `water`, `albedo`, `terrain` (block materials), `biome`, `temperature`, `humidity`, `cover`, `objects`, `understory`, `pockets`, `caves`, `cave_biomes`, `annotations`, `structures`, `roads`, `volume.carve`, `volume.add`, and any mask as `mask.<name>`. They are the Map Output node's inputs.
- A component is a node made of nodes: `component Name(input: height, mask_in: mask?, size: float = 800) { … output result; }`. Parameters typed as a socket (height, mask, color, vector, terrain, biome, layer, points, splines, footprints, instances, density; `?` optional) are its inputs; those typed float, int, bool, seed, choice, string or value are settings it exposes, which its nodes take as values (`height: size`). `expose size { label: "Size"; min: 0; max: 2000; unit: "blocks"; }` says how to show one. `label = "…"; category = "…"; summary = "…";` describe it. Use it like any node: `m = Name(relief, size: 900);`, `m.result` for one of its outputs.
- `import "lib.mscript";` reads another script (its components, constants and nodes).
- A type can also be called by its id: `noise.fbm(…)`, `"massif:mountain"(…)`, and an older version with `@`: `FractalNoise@1(…)`.

## Sockets

Each input and output carries a type. A height and a mask feed each other as they are; a colour feeds a number by its brightness, a number feeds a colour as a grey; every other type feeds only its own. Heights are in blocks (1 block = 1 m; Minecraft builds from y −64 to 320, the sea at 62); masks go from 0 to 1.
"##;

const ANSWER: &str = r##"## How to answer

- Answer with ONE complete script in a single ```mscript code block: `project "Name" { … }` with everything in it, not a diff. A sentence before it is welcome; nothing after it.
- Use only the node names of the catalogue above (or their aliases), with their input and setting names exactly as written. Give settings values inside their ranges.
- Give every node a short, meaningful name in snake_case; group related nodes with `group "Title" { color = #rrggbb; … }`.
- End with the outputs: at least `output height = …;`.
- To change the graph open now, start from its script below and keep the names of the nodes you keep (they are the nodes' ids).
"##;

/// The language and the catalogue, without the graph open now.
pub fn reference() -> String {
    let mut md = String::from("# MassifScript\n\n");
    md.push_str(LANGUAGE);
    md.push('\n');
    for k in PortKind::ALL {
        let feeds: Vec<&str> = PortKind::ALL.iter().filter(|to| k.conversion(**to).is_some()).map(|t| t.id()).collect();
        let _ = writeln!(md, "- `{}` feeds {}", k.id(), feeds.join(", "));
    }
    md.push_str("\n## Nodes\n\nEach line: `Name(inputs) -> outputs — what it makes. Settings: name: kind range unit = default`. `?` marks an optional input; `*` a setting a field can drive.\n");
    let cat = names::catalogue();
    for f in Family::ALL {
        let of: Vec<&(String, &NodeSpec)> = cat.iter().filter(|(_, s)| s.family == f && !s.id.starts_with("legacy.")).collect();
        if of.is_empty() {
            continue;
        }
        let _ = writeln!(md, "\n### {}\n", f.label());
        for (name, s) in of {
            let _ = writeln!(md, "{}", node_line(name, s));
        }
    }
    let legacy: Vec<&(String, &NodeSpec)> = cat.iter().filter(|(_, s)| s.id.starts_with("legacy.")).collect();
    if !legacy.is_empty() {
        md.push_str("\n### Legacy (kept for graphs of 1.2's first previews; prefer the nodes above)\n\n");
        for (name, s) in legacy {
            let _ = writeln!(md, "{}", node_line(name, s));
        }
    }
    md.push_str("\n### Composites shipped with Massif\n\n");
    for (kind, _, c) in crate::components::offered() {
        let _ = writeln!(md, "{}", component_line(&names::name_of(kind).unwrap_or_else(|| kind.to_owned()), c));
    }
    md.push_str("\n### Aliases\n\n");
    let mut aliases: Vec<String> = Vec::new();
    for n in names::all_names() {
        let Some(t) = names::lookup(n) else { continue };
        if names::name_of(&t.kind).as_deref() == Some(n) {
            continue;
        }
        let canon = names::name_of(&t.kind).unwrap_or_default();
        let preset = t.preset.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ");
        aliases.push(if preset.is_empty() { format!("`{n}` = {canon}") } else { format!("`{n}` = {canon}({preset})") });
    }
    md.push_str(&aliases.join(", "));
    md.push_str("\n\n## Examples\n");
    for (title, src) in EXAMPLES {
        let _ = write!(md, "\n{title}:\n\n```mscript\n{src}```\n");
    }
    md
}

/// The guide to paste into an AI chat: the language, the whole catalogue,
/// examples, the graph open now as a script (when given), and how to answer.
pub fn guide(current: Option<&str>) -> String {
    let mut md = String::from("You will write a MassifScript for Massif, a Minecraft map editor whose terrain is made by a node graph. Read the reference, then answer the request that follows it.\n\n");
    md.push_str(&reference());
    if let Some(script) = current {
        let _ = write!(md, "\n## The graph open now\n\n```mscript\n{}```\n", script);
    }
    md.push('\n');
    md.push_str(ANSWER);
    md.push_str("\n## Request\n\n");
    md
}
