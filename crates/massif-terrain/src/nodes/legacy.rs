//! Nodes of Massif 1.2 that the terrain generator has no node of its own
//! for, run with 1.2's own code over the whole terrain at the simulations'
//! resolution, so that a graph made then keeps everything it did. Their
//! settings are 1.2's, by the same names; a graph gets them only by being
//! migrated (see [`crate::migrate`]).

use crate::kernel::{Ctx, Grid};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Unit};
use massif_core::graph::{ColorField, Extent, Field, NodeKind, Value, run_node};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub static NODES: &[&NodeSpec] = &[&FLOATING_ISLANDS, &OVERHANG, &ROCKS, &TECTONIC, &FILE_INPUT, &EROSION, &RIVERS, &LAKES, &SEA, &FLOW, &AUTO_COLOR];

const fn h(name: &'static str, label: &'static str, help: &'static str) -> PortSpec {
    PortSpec::new(name, label, PortKind::Height, help)
}

const fn m(name: &'static str, label: &'static str, help: &'static str) -> PortSpec {
    PortSpec::new(name, label, PortKind::Mask, help)
}

const RELIEF_IN: PortSpec = h("in", "In", "The relief it works on.");

static FLOATING_ISLANDS: NodeSpec = NodeSpec {
    id: "legacy.floating_islands",
    label: "Floating islands (1.2)",
    family: Family::Shape,
    summary: "Islands of rock floating in the air: a rolling top on sheer sides, over an underside hanging down to a point.",
    help: "Altitude is the height of their middles and Spread how much it varies; Size the radius of the largest; Coverage how many of the places one could float have one; Depth how far the underside hangs, as a multiple of the radius. Top and Bottom feed a layer of the Map Output.",
    keywords: &["floating", "sky", "islands", "legacy"],
    outputs: &[h("top", "Top", "The islands' top (the bottom where there is none)."), h("bottom", "Bottom", "Their underside."), m("islands", "Islands", "Where they are.")],
    params: &[
        ParamSpec::float("altitude", "Altitude", Unit::Blocks, 230.0, -64.0, 4096.0, "The height of their middles."),
        ParamSpec::float("spread", "Spread", Unit::Blocks, 25.0, 0.0, 1000.0, "How much their altitude varies."),
        ParamSpec::float("size", "Size", Unit::Blocks, 70.0, 4.0, 4000.0, "The radius of the largest."),
        ParamSpec::float("coverage", "Coverage", Unit::Ratio, 0.5, 0.0, 1.0, "How many of the places one could float have one."),
        ParamSpec::float("depth", "Depth", Unit::Ratio, 1.3, 0.0, 10.0, "How far the underside hangs, as a multiple of the radius."),
        ParamSpec::int("seed", "Seed", 11, 0, u32::MAX as i64, "Another seed places them otherwise."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None, None, None] },
    ..NodeSpec::BASE
};

static OVERHANG: NodeSpec = NodeSpec {
    id: "legacy.overhang",
    label: "Overhang (1.2)",
    family: Family::Adjust,
    summary: "Rims jutting out from the top of cliffs over the void below: a slab of rock, as its top and bottom.",
    help: "Reach is how far the rims jut out, Thickness how thick they are where they leave the cliff, Drop the smallest drop that makes a cliff, Variation how unevenly they jut out along it. Top and Bottom feed a layer of the Map Output.",
    keywords: &["overhang", "ledge", "cliff", "legacy"],
    inputs: &[RELIEF_IN],
    outputs: &[h("top", "Top", "The rims' top (the bottom where there is none)."), h("bottom", "Bottom", "Their underside.")],
    params: &[
        ParamSpec::float("reach", "Reach", Unit::Blocks, 10.0, 0.0, 200.0, "How far the rims jut out."),
        ParamSpec::float("thickness", "Thickness", Unit::Blocks, 8.0, 1.0, 200.0, "How thick a rim is where it leaves the cliff."),
        ParamSpec::float("drop", "Drop", Unit::Blocks, 24.0, 1.0, 400.0, "The smallest drop that makes a cliff."),
        ParamSpec::float("variation", "Variation", Unit::Ratio, 0.6, 0.0, 1.0, "How unevenly the rims jut out along the cliffs."),
        ParamSpec::int("seed", "Seed", 12, 0, u32::MAX as i64, "Another seed varies them otherwise."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None, None] },
    ..NodeSpec::BASE
};

static ROCKS: NodeSpec = NodeSpec {
    id: "legacy.rocks",
    label: "Rocks (1.2)",
    family: Family::Primitive,
    summary: "Rocky detail around 0, to add to a relief.",
    help: "Scale is the size of the rocks, Height how high they stand, Sharpness from rounded boulders (0) to sharp crags (1). Add it to a relief with Blend.",
    keywords: &["rocks", "boulders", "crags", "legacy"],
    outputs: &[h("out", "Out", "The rocks, around 0.")],
    params: &[
        ParamSpec::float("scale", "Scale", Unit::Blocks, 90.0, 2.0, 8000.0, "The size of the rocks."),
        ParamSpec::float("height", "Height", Unit::Blocks, 12.0, 0.0, 1000.0, "How high they stand."),
        ParamSpec::float("sharpness", "Sharpness", Unit::Ratio, 0.6, 0.0, 1.0, "From rounded boulders (0) to sharp crags (1)."),
        ParamSpec::int("seed", "Seed", 10, 0, u32::MAX as i64, "Another seed places them otherwise."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

static TECTONIC: NodeSpec = NodeSpec {
    id: "legacy.tectonic",
    label: "Tectonic plates (1.2)",
    family: Family::Shape,
    summary: "Relief from tectonic plates: continents and oceans, mountains where plates collide, rifts where they pull apart.",
    help: "Plates are 1.2's plates, as JSON. Relief is the height of the highest ranges, Width how wide the belts along the boundaries are, Roughness how much the boundaries wind, Sea the sea level the oceans lie under.",
    keywords: &["tectonic", "plates", "continents", "legacy"],
    outputs: &[h("height", "Height", "The relief."), m("ranges", "Ranges", "Where plates collide."), m("faults", "Faults", "Where they pull apart.")],
    params: &[
        ParamSpec::text("plates", "Plates", "[]", "The plates, as 1.2 wrote them (JSON)."),
        ParamSpec::float("relief", "Relief", Unit::Blocks, 170.0, 0.0, 4096.0, "The height of the highest ranges."),
        ParamSpec::float("width", "Width", Unit::Blocks, 260.0, 1.0, 20000.0, "How wide the belts along the boundaries are."),
        ParamSpec::float("roughness", "Roughness", Unit::Ratio, 0.6, 0.0, 1.0, "How much the boundaries wind."),
        ParamSpec::float("sea", "Sea", Unit::Blocks, 63.0, -64.0, 320.0, "The sea level the oceans lie under."),
        ParamSpec::int("seed", "Seed", 7, 0, u32::MAX as i64, "Another seed varies the boundaries."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None, None, None] },
    ..NodeSpec::BASE
};

static FILE_INPUT: NodeSpec = NodeSpec {
    id: "legacy.file_input",
    label: "World file (1.2)",
    family: Family::Input,
    summary: "A world brought in (WorldPainter's .world, Massif's .massif): its surface, its water and its colours.",
    help: "Path is the world's file; Fit stretches it over the terrain, or lays it at its own blocks. Low and High are the heights a height map's black and white stand for (Heightmap reads those better).",
    keywords: &["file", "world", "import", "legacy"],
    outputs: &[h("height", "Height", "The world's surface."), m("water", "Water", "Where water lies."), PortSpec::new("colour", "Colour", PortKind::Color, "The colour of its ground.")],
    params: &[
        ParamSpec::file("path", "Path", &["world", "massif", "png", "r16", "r32"], "The file."),
        ParamSpec::float("low", "Low", Unit::Blocks, 40.0, -4096.0, 4096.0, "The height black stands for."),
        ParamSpec::float("high", "High", Unit::Blocks, 220.0, -4096.0, 4096.0, "The height white stands for."),
        ParamSpec::flag("fit", "Fit", true, "Stretched over the terrain, or at its own blocks."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None, None, None] },
    reads: crate::registry::Reads::Files,
    ..NodeSpec::BASE
};

static EROSION: NodeSpec = NodeSpec {
    id: "legacy.erosion",
    label: "Erosion (1.2)",
    family: Family::Simulate,
    summary: "Water running down the slopes carries soil from the heights to the valleys: gullies, smoothed slopes, alluvial fans.",
    help: "Rain is how many drops fall on each sample; Erosion and Deposition how much soil a drop takes and leaves; Capacity how much it can carry; Evaporation how fast it dries; Radius the brush a drop wears with, in blocks.",
    keywords: &["erosion", "hydraulic", "droplets", "legacy"],
    inputs: &[RELIEF_IN],
    outputs: &[h("height", "Height", "The relief worn."), m("flow", "Flow", "Where water ran."), m("deposits", "Deposits", "Where soil settled."), m("wear", "Wear", "Where soil was taken.")],
    params: &[
        ParamSpec::float("rain", "Rain", Unit::Ratio, 1.0, 0.0, 20.0, "Drops on each sample."),
        ParamSpec::float("erosion", "Erosion", Unit::Ratio, 0.3, 0.0, 1.0, "How much soil a drop takes."),
        ParamSpec::float("deposition", "Deposition", Unit::Ratio, 0.3, 0.0, 1.0, "How much soil a drop leaves."),
        ParamSpec::float("capacity", "Capacity", Unit::Ratio, 4.0, 0.0, 64.0, "How much soil a drop can carry."),
        ParamSpec::float("evaporation", "Evaporation", Unit::Ratio, 0.02, 0.0, 0.5, "How fast a drop dries."),
        ParamSpec::float("radius", "Radius", Unit::Blocks, 6.0, 1.0, 64.0, "The brush a drop wears with."),
        ParamSpec::int("seed", "Seed", 5, 0, u32::MAX as i64, "Another seed drops the rain otherwise."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[Some(0), None, None, None] },
    ..NodeSpec::BASE
};

static RIVERS: NodeSpec = NodeSpec {
    id: "legacy.rivers",
    label: "Rivers (1.2)",
    family: Family::Simulate,
    summary: "Rivers traced where the water of the whole terrain runs, wider and deeper downstream, dug into their valleys, with lakes in the hollows they cross.",
    help: "Threshold is how much land must drain to a place for a river to run there (higher: fewer, larger rivers); Depth and Width those of the largest; Sea the level where rivers end; Banks how far the banks slope down, in river widths; Lakes which hollows hold lakes.",
    keywords: &["rivers", "streams", "water", "legacy"],
    inputs: &[RELIEF_IN, h("water_level", "Water level", "Water already there (rivers of another node).").optional()],
    outputs: &[h("height", "Height", "The relief with the rivers dug."), m("rivers", "Rivers", "Where they run."), h("water", "Water", "The water's level.")],
    params: &[
        ParamSpec::float("threshold", "Threshold", Unit::Ratio, 0.55, 0.0, 1.0, "How much land must drain to a place for a river to run there."),
        ParamSpec::float("depth", "Depth", Unit::Blocks, 5.0, 0.0, 100.0, "The depth of the largest river."),
        ParamSpec::float("width", "Width", Unit::Blocks, 14.0, 1.0, 400.0, "The width of the largest river."),
        ParamSpec::float("sea", "Sea", Unit::Blocks, 63.0, -10000.0, 320.0, "The sea level, where rivers end (−10000: no sea)."),
        ParamSpec::float("banks", "Banks", Unit::Ratio, 1.0, 0.0, 10.0, "How far the banks slope down to the water, in river widths."),
        ParamSpec::float("lakes", "Lakes", Unit::Ratio, 0.5, 0.0, 1.0, "Which hollows the rivers cross hold lakes."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

static LAKES: NodeSpec = NodeSpec {
    id: "legacy.lakes",
    label: "Lakes (1.2)",
    family: Family::Simulate,
    summary: "The hollows water would fill, filled: lakes and flat valley floors.",
    help: "Amount goes from nothing (0) to every hollow full (1); Sea is the level under which a hollow is the sea's, not a lake.",
    keywords: &["lakes", "ponds", "water", "legacy"],
    inputs: &[RELIEF_IN],
    outputs: &[h("height", "Height", "The relief."), m("lakes", "Lakes", "Where the lakes are."), h("water", "Water", "The lakes' level.")],
    params: &[
        ParamSpec::float("amount", "Amount", Unit::Ratio, 1.0, 0.0, 1.0, "From nothing to every hollow full."),
        ParamSpec::float("sea", "Sea", Unit::Blocks, 63.0, -10000.0, 320.0, "The sea level (−10000: no sea)."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

static SEA: NodeSpec = NodeSpec {
    id: "legacy.sea",
    label: "Sea (1.2)",
    family: Family::Simulate,
    summary: "A sea level: where the ground is under water, and its shore.",
    help: "Level is the sea's height; Shore how wide the shore is, in blocks of height around it.",
    keywords: &["sea", "ocean", "shore", "legacy"],
    inputs: &[RELIEF_IN],
    outputs: &[h("height", "Height", "The relief."), m("water", "Water", "Where the sea is."), m("shore", "Shore", "Its shore.")],
    params: &[
        ParamSpec::float("level", "Level", Unit::Blocks, 62.0, -64.0, 320.0, "The sea's height."),
        ParamSpec::float("shore", "Shore", Unit::Blocks, 4.0, 0.0, 100.0, "How wide the shore is, in blocks of height."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

static FLOW: NodeSpec = NodeSpec {
    id: "legacy.flow",
    label: "Flow (1.2)",
    family: Family::Simulate,
    summary: "Where the water of the terrain runs and gathers.",
    help: "Spread is how much small rills show beside the big rivers.",
    keywords: &["flow", "drainage", "water", "legacy"],
    inputs: &[RELIEF_IN],
    outputs: &[m("mask", "Mask", "Where water runs, 1 in the largest rivers.")],
    params: &[ParamSpec::float("spread", "Spread", Unit::Ratio, 0.5, 0.0, 1.0, "How much small rills show beside the big rivers.")],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

const AUTO_PALETTES: &[Choice] = &[
    Choice { value: "temperate", label: "Temperate", help: "Meadows, forests, grey rock, snow." },
    Choice { value: "alpine", label: "Alpine", help: "High meadows, scree, snow." },
    Choice { value: "desert", label: "Desert", help: "Sand and red rock." },
    Choice { value: "arctic", label: "Arctic", help: "Tundra, ice, snow." },
    Choice { value: "volcanic", label: "Volcanic", help: "Ash and basalt." },
    Choice { value: "tropical", label: "Tropical", help: "Jungle and dark rock." },
    Choice { value: "lunar", label: "Lunar", help: "Greys of the moon." },
    Choice { value: "martian", label: "Martian", help: "Rusty dust and rock." },
];

static AUTO_COLOR: NodeSpec = NodeSpec {
    id: "legacy.auto_color",
    label: "Auto colour (1.2)",
    family: Family::Color,
    summary: "Natural colours from the relief alone: valleys, meadows, rock on the steep slopes, snow on the peaks.",
    help: "Palette is the land's colours; Snow line where snow starts, in blocks; Rock slope the slope where rock shows, in degrees.",
    keywords: &["auto", "colour", "color", "legacy"],
    inputs: &[h("height", "Height", "The relief to colour.")],
    outputs: &[PortSpec::new("colour", "Colour", PortKind::Color, "Its colours.")],
    params: &[
        ParamSpec::choice("palette", "Palette", AUTO_PALETTES, "temperate", "The land's colours."),
        ParamSpec::float("snow_line", "Snow line", Unit::Blocks, 200.0, -64.0, 4096.0, "Where snow starts."),
        ParamSpec::float("rock_slope", "Rock slope", Unit::Degrees, 38.0, 0.0, 90.0, "The slope where rock shows."),
        ParamSpec::int("seed", "Seed", 1, 0, u32::MAX as i64, "Another seed varies the colours."),
    ],
    kernel: Kernel::Global { cpu: legacy_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

/// The 1.2 kind a legacy node stands for, from its settings (named as 1.2
/// named them).
fn kind_of(ctx: &Ctx) -> Option<NodeKind> {
    let spec = ctx.params.spec();
    let variant = match spec.id {
        "legacy.floating_islands" => "FloatingIslands",
        "legacy.overhang" => "Overhang",
        "legacy.rocks" => "Rocks",
        "legacy.tectonic" => "Tectonic",
        "legacy.file_input" => "FileInput",
        "legacy.erosion" => "Erosion",
        "legacy.rivers" => "Rivers",
        "legacy.lakes" => "Lakes",
        "legacy.sea" => "Sea",
        "legacy.flow" => "Flow",
        "legacy.auto_color" => "AutoColor",
        _ => return None,
    };
    let mut fields = serde_json::Map::new();
    for (name, value) in ctx.params.to_json() {
        let value = match (spec.id, name.as_str(), value) {
            ("legacy.tectonic", "plates", serde_json::Value::String(text)) => match serde_json::from_str::<serde_json::Value>(&text).ok()? {
                // None given: 1.2's nine plates.
                serde_json::Value::Array(a) if a.is_empty() => serde_json::to_value(massif_core::graph::scatter_plates(9, ctx.params.int("seed") as u32)).ok()?,
                plates => plates,
            },
            ("legacy.auto_color", "palette", serde_json::Value::String(p)) => {
                let mut c = p.chars();
                serde_json::Value::String(c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default())
            }
            (_, _, v) => v,
        };
        fields.insert(name, value);
    }
    serde_json::from_value(serde_json::json!({ variant: fields })).ok()
}

/// Runs the 1.2 node over the whole terrain: the grid its samples lie on
/// is the extent 1.2 evaluates over, sample for sample.
fn legacy_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let kind = kind_of(ctx)?;
    let n = ctx.rect.w as usize;
    let step = ctx.spacing();
    // 1.2's samples lie at the middle of n equal cells; ours at their corner.
    let extent = Extent { x0: ctx.space.x0 as f64 - step / 2.0, z0: ctx.space.z0 as f64 - step / 2.0, size: step * n as f64, n };
    let area = n * n;
    let ins: Vec<Option<Value>> = ctx
        .params
        .spec()
        .inputs
        .iter()
        .zip(inputs)
        .map(|(port, g)| {
            g.map(|g| match port.kind {
                PortKind::Color => Value::Color(Arc::new(ColorField::build(n, |c| [g.data[c], g.data[area + c], g.data[2 * area + c]]))),
                _ => Value::Field(Arc::new(Field { n, data: g.data[..area].to_vec() })),
            })
        })
        .collect();
    let stop = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    let made = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if cancel() {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        let made = run_node(&kind, &extent, &ins, &stop);
        done.store(true, Ordering::Relaxed);
        made
    })?;
    if cancel() {
        return None;
    }
    Some(
        made.iter()
            .map(|v| match v {
                Value::Field(f) if f.n == n => f.data.clone(),
                Value::Field(f) => (0..area).map(|c| f.sample(((c % n) as f32) * f.n as f32 / n as f32, ((c / n) as f32) * f.n as f32 / n as f32)).collect(),
                Value::Color(cf) => {
                    let mut out = vec![0.0; 3 * area];
                    for c in 0..area {
                        let rgb = if cf.n == n { cf.linear(c) } else { cf.sample(((c % n) as f32) * cf.n as f32 / n as f32, ((c / n) as f32) * cf.n as f32 / n as f32) };
                        for k in 0..3 {
                            out[k * area + c] = rgb[k];
                        }
                    }
                    out
                }
            })
            .collect(),
    )
}
