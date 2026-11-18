//! Colours of the ground, and terrains (the block materials).

use crate::kernel::{Ctx, Stats};
use crate::noise::Fbm;
use crate::params::{eval_gradient, linear, parse_color};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::util::{hash_str, mix32};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&RAMP, &GRADIENT, &SOLID, &MIX, &ADJUST, &LAYERS, &SELECT];

const COLOR_OUT: PortSpec = PortSpec::new("color", "Colour", PortKind::Color, "The colours.");

pub static RAMP: NodeSpec = NodeSpec {
    id: "color.ramp",
    label: "Colour ramp",
    family: Family::Color,
    summary: "Colours the ground by its height, from one colour at Low to another at High.",
    help: "The simplest colouring: a colour for the low ground and one for the high ground, blended between. Feed the Map Output's Albedo with it, or mix several through masks.",
    keywords: &["gradient", "colour", "color", "tint", "albedo"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to colour.")],
    outputs: &[PortSpec::new("color", "Colour", PortKind::Color, "The colours.")],
    params: &[
        ParamSpec::float("low", "Low", Unit::Blocks, 60.0, -2048.0, 4096.0, "The height that takes the low colour.").soft(-64.0, 320.0),
        ParamSpec::float("high", "High", Unit::Blocks, 250.0, -2048.0, 4096.0, "The height that takes the high colour.").soft(-64.0, 320.0),
        ParamSpec::color("color_low", "Low colour", "#5c7a3a", "The colour of the low ground."),
        ParamSpec::color("color_high", "High colour", "#e8e8ea", "The colour of the high ground."),
    ],
    kernel: Kernel::Point { cpu: ramp_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn ramp_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("the ramp has its input");
    let p = ctx.params;
    let (lo, hi) = (p.f("low"), p.f("high"));
    let span = if (hi - lo).abs() < 1e-6 { 1e-6 } else { hi - lo };
    let (a, b) = (p.color("color_low"), p.color("color_high"));
    let n = ctx.rect.area();
    for (k, &v) in h.iter().enumerate() {
        let t = ((v - lo) / span).clamp(0.0, 1.0);
        for c in 0..3 {
            out[0][c * n + k] = a[c] + (b[c] - a[c]) * t;
        }
    }
}

// -------------------------------------------------------------- gradient --

/// Gradients from nature, as sRGB stops.
const PRESETS: &[(&str, &[(f64, &str)])] = &[
    ("alpine", &[(0.0, "#3d5a2a"), (0.3, "#5f7a36"), (0.5, "#7d7259"), (0.72, "#86847f"), (0.88, "#cfd6db"), (1.0, "#f2f5f7")]),
    ("desert", &[(0.0, "#d9b98a"), (0.35, "#d6a46b"), (0.6, "#c07a48"), (0.85, "#9a5534"), (1.0, "#7d4a33")]),
    ("tropical", &[(0.0, "#e9d8a6"), (0.06, "#4f9a3c"), (0.4, "#2f7a2e"), (0.75, "#47613a"), (1.0, "#6f6f62")]),
    ("tundra", &[(0.0, "#5f6b4a"), (0.4, "#7c7f63"), (0.7, "#9c9a8e"), (0.88, "#d9dcdd"), (1.0, "#f4f6f8")]),
    ("volcanic", &[(0.0, "#2b2a2c"), (0.35, "#3d3a37"), (0.6, "#4f3a30"), (0.85, "#6d5a50"), (1.0, "#8b8079")]),
    ("badlands", &[(0.0, "#b5552d"), (0.14, "#d68a4a"), (0.28, "#e8c38a"), (0.42, "#a0432a"), (0.56, "#d9a06a"), (0.7, "#8c3d27"), (0.85, "#e3c9a0"), (1.0, "#b05a35")]),
];

const PRESET_CHOICES: &[Choice] = &[
    Choice { value: "custom", label: "Custom", help: "The stops of Gradient." },
    Choice { value: "alpine", label: "Alpine", help: "Meadow, forest, brown and grey rock, snow." },
    Choice { value: "desert", label: "Desert", help: "Pale sand to red rock." },
    Choice { value: "tropical", label: "Tropical", help: "White sand, jungle greens, dark rock." },
    Choice { value: "tundra", label: "Tundra", help: "Olive moss, grey stone, snow." },
    Choice { value: "volcanic", label: "Volcanic", help: "Black ash and basalt, rusty slopes." },
    Choice { value: "badlands", label: "Badlands strata", help: "Bands of red, orange and cream rock." },
];

pub static GRADIENT: NodeSpec = NodeSpec {
    id: "color.gradient",
    label: "Gradient",
    family: Family::Color,
    summary: "Colours from a value — a height, a mask — mapped from Low to High onto a gradient: one from nature, or one of your own.",
    help: "The input from Low to High picks a colour along the gradient (Auto takes the input's own lowest and highest values). Presets give gradients from nature; Custom uses the stops of Gradient. Feed it a height for bands of altitude, a slope mask for rock on steep ground, a strata noise for badlands. Variation jitters the colours a little so that large areas do not look flat.",
    keywords: &["gradient", "ramp", "colour", "color", "palette", "lut", "satmap", "texture"],
    inputs: &[PortSpec::new("in", "In", PortKind::Height, "The value that picks the colour: a height or a mask.")],
    outputs: &[COLOR_OUT],
    params: &[
        ParamSpec::choice("preset", "Preset", PRESET_CHOICES, "alpine", "A gradient from nature, or Custom."),
        ParamSpec::gradient("gradient", "Gradient", &[(0.0, "#3d5a2a"), (0.5, "#7d7259"), (1.0, "#f2f5f7")], "The stops, when Preset is Custom."),
        ParamSpec::float("low", "Low", Unit::Blocks, 62.0, -4096.0, 4096.0, "The value that takes the gradient's first colour.").soft(-64.0, 320.0),
        ParamSpec::float("high", "High", Unit::Blocks, 300.0, -4096.0, 4096.0, "The value that takes its last colour.").soft(-64.0, 320.0),
        ParamSpec::flag("auto", "Auto", false, "Take the input's own lowest and highest values as Low and High."),
        ParamSpec::flag("reverse", "Reverse", false, "Run the gradient the other way.").section(Section::Advanced),
        ParamSpec::float("variation", "Variation", Unit::Ratio, 0.05, 0.0, 1.0, "How much the colours are jittered.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Stats { cpu: gradient_cpu },
    ..NodeSpec::BASE
};

fn gradient_cpu(ctx: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let src = inputs[0].expect("the gradient has its input");
    let (lo, hi) = match (p.flag("auto"), stats[0]) {
        (true, Some(s)) => (s.min, s.max),
        _ => (p.f("low"), p.f("high")),
    };
    let span = if (hi - lo).abs() < 1e-6 { 1e-6 } else { hi - lo };
    let stops: Vec<(f64, [f32; 3])> = match PRESETS.iter().find(|(name, _)| *name == p.choice("preset")) {
        Some((_, stops)) => stops.iter().map(|(at, c)| (*at, parse_color(c).unwrap_or([0.5; 3]))).collect(),
        None => match p.values().iter().find_map(|v| if let crate::params::Value::Gradient(g) = v { Some(g.clone()) } else { None }) {
            Some(g) => g,
            None => vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])],
        },
    };
    let reverse = p.flag("reverse");
    let var = p.f("variation");
    let jitter = Fbm::new(24.0, 2, 0.5, ctx.seed);
    let n = ctx.rect.area();
    let w = ctx.rect.w as usize;
    for (k, &v) in src.iter().enumerate() {
        let mut t = ((v - lo) / span).clamp(0.0, 1.0);
        if reverse {
            t = 1.0 - t;
        }
        let c = eval_gradient(&stops, t as f64);
        let shade = if var > 0.0 {
            let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
            1.0 + var * jitter.at(x, z)
        } else {
            1.0
        };
        for (q, cq) in c.iter().enumerate() {
            out[0][q * n + k] = (cq * shade).max(0.0);
        }
    }
}

// --------------------------------------------------- solid, mix, adjust --

pub static SOLID: NodeSpec = NodeSpec {
    id: "color.solid",
    label: "Solid colour",
    family: Family::Color,
    summary: "One colour everywhere.",
    help: "A colour to mix in through a mask: snow white where the snow mask says, grey rock on cliffs.",
    keywords: &["colour", "color", "solid", "flat", "fill", "constant"],
    inputs: &[],
    outputs: &[COLOR_OUT],
    params: &[ParamSpec::color("color", "Colour", "#7f7f7f", "The colour.")],
    kernel: Kernel::Point { cpu: solid_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn solid_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let c = ctx.params.color("color");
    let n = ctx.rect.area();
    for (q, cq) in c.iter().enumerate() {
        out[0][q * n..(q + 1) * n].fill(*cq);
    }
}

const MIX_MODES: &[Choice] = &[
    Choice { value: "mix", label: "Mix", help: "B over A through the mask." },
    Choice { value: "multiply", label: "Multiply", help: "A darkened by B: shadows, wet ground." },
    Choice { value: "screen", label: "Screen", help: "A lightened by B: dust, frost." },
    Choice { value: "overlay", label: "Overlay", help: "B deepening A's contrast: tinting while keeping detail." },
];

pub static MIX: NodeSpec = NodeSpec {
    id: "color.mix",
    label: "Mix colours",
    family: Family::Color,
    summary: "Two colourings combined through a mask: B laid over A where the mask is 1, by a mode.",
    help: "The way to put rock colour on cliffs (a slope mask), snow on summits (a snow mask), sand on beaches (a distance mask from the sea). Strength fades the whole mix.",
    keywords: &["mix", "blend", "lerp", "colour", "color", "layer", "tint", "overlay"],
    inputs: &[
        PortSpec::new("a", "A", PortKind::Color, "The colours mixed into."),
        PortSpec::new("b", "B", PortKind::Color, "The colours mixed in."),
        PortSpec::new("mask", "Mask", PortKind::Mask, "Where B is mixed in (everywhere when unconnected).").optional(),
    ],
    outputs: &[COLOR_OUT],
    params: &[
        ParamSpec::choice("mode", "Mode", MIX_MODES, "mix", "How B combines with A."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 1.0, "How much of the mix is taken: 0 leaves A."),
    ],
    kernel: Kernel::Point { cpu: mix_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn mix_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let (a, b) = (inputs[0].expect("mix has A"), inputs[1].expect("mix has B"));
    let mode = ctx.params.choice("mode");
    let strength = ctx.params.f("strength");
    let n = ctx.rect.area();
    for k in 0..n {
        let w = strength * inputs[2].map_or(1.0, |m| m[k].clamp(0.0, 1.0));
        for q in 0..3 {
            let (x, y) = (a[q * n + k], b[q * n + k]);
            let c = match mode {
                "multiply" => x * y,
                "screen" => 1.0 - (1.0 - x) * (1.0 - y),
                "overlay" => {
                    if x < 0.5 {
                        2.0 * x * y
                    } else {
                        1.0 - 2.0 * (1.0 - x) * (1.0 - y)
                    }
                }
                _ => y,
            };
            out[0][q * n + k] = x + (c - x) * w;
        }
    }
}

pub static ADJUST: NodeSpec = NodeSpec {
    id: "color.adjust",
    label: "Adjust colours",
    family: Family::Color,
    summary: "Colours turned in hue, made more or less vivid, lighter or darker, with more or less contrast.",
    help: "Hue turns every colour around the wheel; Saturation 0 makes greys, 2 doubles the vividness; Brightness and Contrast as in a photo editor.",
    keywords: &["hue", "saturation", "brightness", "contrast", "grade", "colour", "color"],
    inputs: &[PortSpec::new("color", "Colour", PortKind::Color, "The colours to adjust.")],
    outputs: &[COLOR_OUT],
    params: &[
        ParamSpec::float("hue", "Hue", Unit::Degrees, 0.0, -180.0, 180.0, "How far every colour turns around the wheel."),
        ParamSpec::float("saturation", "Saturation", Unit::Ratio, 1.0, 0.0, 3.0, "How vivid: 0 grey, 1 as it is."),
        ParamSpec::float("brightness", "Brightness", Unit::Ratio, 1.0, 0.0, 3.0, "How light: 1 as it is."),
        ParamSpec::float("contrast", "Contrast", Unit::Ratio, 1.0, 0.0, 3.0, "How far colours spread from mid-grey: 1 as it is."),
    ],
    kernel: Kernel::Point { cpu: adjust_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn encode(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn adjust_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("adjust has its input");
    let p = ctx.params;
    let (turn, sat, bright, contrast) = (p.f("hue") / 360.0, p.f("saturation"), p.f("brightness"), p.f("contrast"));
    let n = ctx.rect.area();
    for k in 0..n {
        // In sRGB, where the eye's steps are even.
        let (r, g, b) = (encode(src[k]), encode(src[n + k]), encode(src[2 * n + k]));
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let v = max;
        let s = if max > 0.0 { (max - min) / max } else { 0.0 };
        let mut h = if max == min {
            0.0
        } else if max == r {
            ((g - b) / (max - min)).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / (max - min) + 2.0) / 6.0
        } else {
            ((r - g) / (max - min) + 4.0) / 6.0
        };
        h = (h + turn).rem_euclid(1.0);
        let s = (s * sat).clamp(0.0, 1.0);
        let v = (((v * bright) - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - ((h * 6.0).rem_euclid(2.0) - 1.0).abs());
        let (r1, g1, b1) = match (h * 6.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        let rgb = linear([r1 + m, g1 + m, b1 + m]);
        for (q, cq) in rgb.iter().enumerate() {
            out[0][q * n + k] = *cq;
        }
    }
}

// -------------------------------------------------------------- terrains --

/// The colour a terrain is shown in, by its name: the built-in terrains'
/// own, another's from its name's hash.
pub fn terrain_color(name: &str) -> [f32; 3] {
    const KNOWN: &[(&str, [u8; 3])] = &[
        ("grass", [104, 156, 64]),
        ("dirt", [134, 96, 67]),
        ("coarse dirt", [119, 85, 59]),
        ("podzol", [91, 63, 24]),
        ("mycelium", [111, 99, 105]),
        ("moss", [89, 109, 45]),
        ("mud", [60, 57, 61]),
        ("clay", [160, 166, 179]),
        ("rock", [118, 118, 118]),
        ("rocky", [110, 110, 110]),
        ("stone", [125, 125, 125]),
        ("cobblestone", [127, 127, 127]),
        ("andesite", [136, 136, 136]),
        ("diorite", [188, 188, 188]),
        ("granite", [149, 103, 85]),
        ("deepslate", [80, 80, 82]),
        ("tuff", [108, 109, 102]),
        ("calcite", [223, 224, 220]),
        ("basalt", [72, 72, 78]),
        ("blackstone", [42, 36, 41]),
        ("obsidian", [15, 10, 24]),
        ("sand", [219, 207, 163]),
        ("red sand", [190, 102, 33]),
        ("sandstone", [216, 203, 155]),
        ("gravel", [136, 126, 126]),
        ("terracotta", [152, 94, 67]),
        ("snow", [240, 250, 250]),
        ("powder snow", [248, 253, 253]),
        ("ice", [141, 180, 250]),
        ("blue ice", [116, 167, 253]),
    ];
    let lower = name.trim().to_ascii_lowercase();
    match KNOWN.iter().find(|(n, _)| *n == lower) {
        Some((_, c)) => linear(c.map(|v| v as f32 / 255.0)),
        None => {
            let h = mix32((hash_str(&lower) & 0xffff_ffff) as u32);
            linear([(h & 0xff) as f32 / 255.0, ((h >> 8) & 0xff) as f32 / 255.0, ((h >> 16) & 0xff) as f32 / 255.0])
        }
    }
}

const fn layer_mask(name: &'static str, label: &'static str) -> PortSpec {
    PortSpec::new(name, label, PortKind::Mask, "Where this layer's terrain lies.").optional()
}

const fn layer_terrain(name: &'static str, label: &'static str, default: &'static str) -> ParamSpec {
    ParamSpec::text(name, label, default, "The terrain this layer lays, by its name.")
}

pub static LAYERS: NodeSpec = NodeSpec {
    id: "terrain.layers",
    label: "Terrain layers",
    family: Family::Color,
    summary: "The terrain (block material) of each place, from masks: a base terrain, and up to six layers laid over it where their masks say.",
    help: "Base is the terrain everywhere (or Under, where connected); each layer lays its terrain where its mask passes Threshold, later layers over earlier ones (rock on slopes, then gravel on scree, then sand on beaches, then snow). Dither frays the edges so that terrains mix block by block. Terrains are named as in the world's terrain list (Grass, Rock, Gravel, Sand, Snow…). Feed the Map Output's Terrain with it.",
    keywords: &["terrain", "material", "blocks", "layers", "paint", "splat", "texture"],
    inputs: &[
        layer_mask("mask_1", "Layer 1"),
        layer_mask("mask_2", "Layer 2"),
        layer_mask("mask_3", "Layer 3"),
        layer_mask("mask_4", "Layer 4"),
        layer_mask("mask_5", "Layer 5"),
        layer_mask("mask_6", "Layer 6"),
        PortSpec::new("under", "Under", PortKind::Terrain, "The terrains where no layer is laid (the map's own, through a Map input).").or("Base everywhere"),
    ],
    outputs: &[PortSpec::new("terrain", "Terrain", PortKind::Terrain, "The terrain of each place."), PortSpec::new("color", "Colour", PortKind::Color, "Each terrain's colour, for previews.")],
    params: &[
        ParamSpec::text("base", "Base", "Grass", "The terrain everywhere no layer covers."),
        layer_terrain("terrain_1", "Layer 1", "Rock"),
        layer_terrain("terrain_2", "Layer 2", "Gravel"),
        layer_terrain("terrain_3", "Layer 3", "Sand"),
        layer_terrain("terrain_4", "Layer 4", "Snow"),
        layer_terrain("terrain_5", "Layer 5", ""),
        layer_terrain("terrain_6", "Layer 6", ""),
        ParamSpec::float("threshold", "Threshold", Unit::Ratio, 0.5, 0.0, 1.0, "Where a layer's mask lays its terrain.").section(Section::Detail),
        ParamSpec::float("dither", "Dither", Unit::Ratio, 0.3, 0.0, 1.0, "How much the edges fray, block by block.").section(Section::Detail),
    ],
    kernel: Kernel::Point { cpu: layers_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn layers_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let names: Vec<&str> = std::iter::once(p.text("base")).chain((1..=6).map(|i| p.text(["terrain_1", "terrain_2", "terrain_3", "terrain_4", "terrain_5", "terrain_6"][i - 1]))).collect();
    let ids: Vec<f32> = names.iter().map(|n| ctx.env.terrain_id(n).unwrap_or(0) as f32).collect();
    let colors: Vec<[f32; 3]> = names.iter().map(|n| terrain_color(n)).collect();
    let (threshold, dither) = (p.f("threshold"), p.f("dither"));
    let n = ctx.rect.area();
    let w = ctx.rect.w as usize;
    for k in 0..n {
        let (i, j) = (ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        // The same fray for a block at every level: hashed on its place.
        let (x, z) = ctx.world(i, j);
        let fray = (crate::noise::hash2(x as i64 as u32, z as i64 as u32, ctx.seed) & 0xffff) as f32 / 65535.0 - 0.5;
        let mut chosen = 0usize;
        for (layer, mask) in inputs[..6].iter().enumerate() {
            if let Some(m) = mask
                && !names[layer + 1].trim().is_empty()
                && m[k] > threshold + dither * fray
            {
                chosen = layer + 1;
            }
        }
        out[0][k] = match (chosen, inputs.get(6).copied().flatten()) {
            (0, Some(under)) => under[k],
            _ => ids[chosen],
        };
        for q in 0..3 {
            out[1][q * n + k] = colors[chosen][q];
        }
    }
}

pub static SELECT: NodeSpec = NodeSpec {
    id: "terrain.select",
    label: "Select terrain",
    family: Family::Color,
    summary: "A mask of where a terrain lies: 1 where the terrain is the one named.",
    help: "Picks a terrain out of a terrain map (from Terrain layers, or the map's own through a Map input), to put plants on grass only, to raise snow where there is snow.",
    keywords: &["terrain", "select", "material", "mask", "where"],
    inputs: &[PortSpec::new("terrain", "Terrain", PortKind::Terrain, "The terrains.")],
    outputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "1 where the terrain is Name.")],
    params: &[ParamSpec::text("name", "Name", "Grass", "The terrain picked, by its name.")],
    kernel: Kernel::Point { cpu: select_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn select_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let t = inputs[0].expect("select has its terrains");
    let id = ctx.env.terrain_id(ctx.params.text("name")).map(|i| i as f32);
    for (o, v) in out[0].iter_mut().zip(t) {
        *o = if Some(*v) == id { 1.0 } else { 0.0 };
    }
}
