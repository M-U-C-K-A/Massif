//! Adjust: reshaping one relief.

use crate::kernel::{Ctx, Footprint, GpuKernel, Grid, Stats, Uniforms};
use crate::noise::Fbm;
use crate::params::Params;
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::space::Space;
use rayon::prelude::*;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&LEVELS, &CURVE, &TERRACE, &STRATIFY, &CLAMP, &FOLD, &INVERT, &NORMALIZE, &BLUR, &SHARPEN, &WARP, &TRANSFORM, &SWIRL];

/// Where a reshaping applies: between heights given, or over the input's
/// own range.
const RANGES: &[Choice] = &[
    Choice { value: "given", label: "Given", help: "Between the heights set." },
    Choice { value: "own", label: "Its own", help: "From the input's lowest point to its highest, measured over the whole terrain." },
];

/// The input's range when the node works over it, else `given`.
fn range_of(p: &Params, stats: &[Option<Stats>], given: (f32, f32)) -> (f32, f32) {
    match (p.choice("range"), stats.first().copied().flatten()) {
        ("own", Some(s)) => (s.min, s.max),
        _ => given,
    }
}

const IN: PortSpec = PortSpec::new("in", "In", PortKind::Height, "The relief (or mask) to adjust.");
const OUT: PortSpec = PortSpec::new("out", "Out", PortKind::Height, "The relief adjusted.");

pub static LEVELS: NodeSpec = NodeSpec {
    id: "adjust.levels",
    label: "Levels",
    family: Family::Adjust,
    summary: "Maps one range of heights onto another, along a curve.",
    help: "What lies between In low and In high is spread between Out low and Out high; what lies outside is held at the ends. Gamma bends the way between them: above 1 lifts the low ground (rounded tops, wide valleys), below 1 flattens it (plains below sharp peaks). Works on masks too, with values from 0 to 1.",
    keywords: &["remap", "range", "gamma", "curve", "contrast"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("in_low", "In low", Unit::Blocks, 0.0, -4096.0, 4096.0, "The height taken as the bottom of the range.").soft(-64.0, 512.0),
        ParamSpec::float("in_high", "In high", Unit::Blocks, 256.0, -4096.0, 4096.0, "The height taken as the top of the range.").soft(-64.0, 512.0),
        ParamSpec::float("out_low", "Out low", Unit::Blocks, 0.0, -4096.0, 4096.0, "Where the bottom of the range goes.").soft(-64.0, 512.0),
        ParamSpec::float("out_high", "Out high", Unit::Blocks, 256.0, -4096.0, 4096.0, "Where the top of the range goes.").soft(-64.0, 512.0),
        ParamSpec::float("gamma", "Gamma", Unit::Ratio, 1.0, 0.05, 20.0, "Above 1 lifts the middle of the range, below 1 lowers it.").soft(0.2, 5.0),
    ],
    kernel: Kernel::Point { cpu: levels_cpu, gpu: Some(&LEVELS_GPU) },
    ..NodeSpec::BASE
};

/// In low, the signed span of the range (never zero), Out low, Out's span,
/// 1/gamma.
fn levels_of(p: &Params) -> [f32; 5] {
    let (lo, hi) = (p.f("in_low"), p.f("in_high"));
    let span = hi - lo;
    let span = if span.abs() < 1e-6 { 1e-6f32.copysign(span) } else { span };
    [lo, span, p.f("out_low"), p.f("out_high") - p.f("out_low"), 1.0 / p.f("gamma")]
}

fn levels_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let [lo, span, out_lo, out_span, inv_gamma] = levels_of(ctx.params);
    let src = inputs[0].expect("levels has its input");
    for (o, &v) in out[0].iter_mut().zip(src) {
        let mut t = ((v - lo) / span).clamp(0.0, 1.0);
        if inv_gamma != 1.0 {
            t = t.powf(inv_gamma);
        }
        *o = out_lo + out_span * t;
    }
}

static LEVELS_GPU: GpuKernel = GpuKernel {
    name: "adjust.levels",
    source: include_str!("wgsl/levels.wgsl"),
    uniforms: |ctx, _| {
        let mut u = Uniforms::default();
        for v in levels_of(ctx.params) {
            u.f32(v);
        }
        u.finish()
    },
};

pub static NORMALIZE: NodeSpec = NodeSpec {
    id: "adjust.normalize",
    label: "Normalize",
    family: Family::Adjust,
    summary: "Stretches a relief so that its lowest point lies at Low and its highest at High.",
    help: "Measures the whole relief, then maps its lowest and highest points to Low and High: a relief of any range becomes one of known heights, or a mask from 0 to 1. Heights are measured at the simulation's resolution, so the finest peaks may touch the ends a little early; they are held there.",
    keywords: &["stretch", "range", "fit", "rescale"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("low", "Low", Unit::Blocks, 0.0, -4096.0, 4096.0, "Where the lowest point goes (0 for a mask).").soft(-64.0, 512.0),
        ParamSpec::float("high", "High", Unit::Blocks, 1.0, -4096.0, 4096.0, "Where the highest point goes (1 for a mask).").soft(-64.0, 512.0),
    ],
    kernel: Kernel::Stats { cpu: normalize_cpu },
    ..NodeSpec::BASE
};

fn normalize_cpu(ctx: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let s = stats[0].expect("normalize has its input");
    let (low, high) = (ctx.params.f("low"), ctx.params.f("high"));
    let span = (s.max - s.min).max(1e-6);
    let src = inputs[0].expect("normalize has its input");
    for (o, &v) in out[0].iter_mut().zip(src) {
        *o = low + (high - low) * ((v - s.min) / span).clamp(0.0, 1.0);
    }
}

pub static BLUR: NodeSpec = NodeSpec {
    id: "adjust.blur",
    label: "Blur",
    family: Family::Adjust,
    summary: "Smooths a relief: the detail smaller than Radius goes, the larger shapes stay.",
    help: "A Gaussian blur Radius blocks wide. Wide blurs are made at a coarser level of detail and brought back sharp, so that a blur of a thousand blocks costs as little as one of ten. Use it to soften noise before using it as a mask, to round off terraces, or to make broad uplifts from detailed shapes.",
    keywords: &["smooth", "soften", "gaussian", "average"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[ParamSpec::float("radius", "Radius", Unit::Blocks, 32.0, 0.0, 8192.0, "How far the smoothing reaches, in blocks.").soft(1.0, 512.0)],
    kernel: Kernel::Local { cpu: blur_cpu, gpu: None, footprint: blur_footprint },
    ..NodeSpec::BASE
};

/// The blur's standard deviation in blocks.
fn sigma(p: &Params) -> f64 {
    p.num("radius") / 3.0
}

/// The level the blur works at: the finest with the deviation over 4
/// samples, or the one asked for.
fn blur_lod(p: &Params, lod: u8) -> u8 {
    let s = sigma(p);
    if s <= 8.0 {
        return lod;
    }
    lod.max(((s / 4.0).log2().floor() as u8).min(crate::space::MAX_LOD))
}

fn blur_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    let work = blur_lod(p, lod);
    let s = sigma(p) / Space::spacing(work);
    // The kernel's reach, and two samples for the cubic going back.
    let reach = (3.0 * s).ceil() as u32;
    Footprint { lod: work, halo: reach + if work > lod { 2 } else { 0 } }
}

fn blur_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("blur has its input");
    let s = sigma(ctx.params) / Space::spacing(src.lod);
    crate::filters::resample(ctx, &crate::filters::blur(src, s), &mut out[0]);
}

// ------------------------------------------------- curve, terrace, clamp --

pub static CURVE: NodeSpec = NodeSpec {
    id: "adjust.curve",
    label: "Curve",
    family: Family::Adjust,
    summary: "Reshapes a relief along a curve drawn from its low to its high values: flatten the valleys, sharpen the peaks, lift a plateau.",
    help: "The values from In low to In high go through the Curve (left: In low, right: In high; bottom: Out low, top: Out high). A curve sagging under the diagonal flattens the low ground and steepens the summits; one bulging over it rounds the tops and lifts the plains. Works on masks too, with values from 0 to 1.",
    keywords: &["curve", "remap", "tone", "profile", "transfer", "contrast", "flatten"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::profile("curve", "Curve", &[[0.0, 0.0], [0.5, 0.35], [1.0, 1.0]], "How the values are reshaped, from In low (left) to In high (right)."),
        ParamSpec::float("in_low", "In low", Unit::Blocks, 62.0, -4096.0, 4096.0, "The value at the curve's left.").soft(-64.0, 320.0),
        ParamSpec::float("in_high", "In high", Unit::Blocks, 300.0, -4096.0, 4096.0, "The value at the curve's right.").soft(-64.0, 320.0),
        ParamSpec::float("out_low", "Out low", Unit::Blocks, 62.0, -4096.0, 4096.0, "What the curve's bottom stands for.").soft(-64.0, 320.0),
        ParamSpec::float("out_high", "Out high", Unit::Blocks, 300.0, -4096.0, 4096.0, "What the curve's top stands for.").soft(-64.0, 320.0),
        ParamSpec::choice("range", "Range", RANGES, "given", "Whether the curve goes from In low to In high, or over the input's own range (its ends then stand for the input's lowest and highest points).").section(Section::Advanced),
    ],
    kernel: Kernel::Stats { cpu: curve_cpu },
    ..NodeSpec::BASE
};

fn curve_cpu(ctx: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let src = inputs[0].expect("the curve has its input");
    let (lo, hi, olo, ohi) = match p.choice("range") {
        "own" => {
            let (lo, hi) = range_of(p, stats, (0.0, 1.0));
            (lo, hi, lo, hi)
        }
        _ => (p.f("in_low"), p.f("in_high"), p.f("out_low"), p.f("out_high")),
    };
    let span = if (hi - lo).abs() < 1e-6 { 1e-6 } else { hi - lo };
    for (o, &v) in out[0].iter_mut().zip(src) {
        let t = ((v - lo) / span).clamp(0.0, 1.0);
        *o = olo + (ohi - olo) * p.profile("curve", t as f64) as f32;
    }
}

pub static TERRACE: NodeSpec = NodeSpec {
    id: "adjust.terrace",
    label: "Terraces",
    family: Family::Adjust,
    summary: "Steps cut into a relief between two heights: flat ledges joined by risers, as rock beds or rice terraces make them.",
    help: "Steps is how many ledges there are between Low and High; Sharpness how abrupt the risers are (0 a gentle ramp, 1 a wall). Outside Low and High the relief is left as it is.",
    keywords: &["terrace", "steps", "strata", "ledges", "stairs", "quantize"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::int("steps", "Steps", 8, 1, 256, "How many ledges between Low and High."),
        ParamSpec::float("sharpness", "Sharpness", Unit::Ratio, 0.7, 0.0, 1.0, "How abrupt the risers are."),
        ParamSpec::float("low", "Low", Unit::Blocks, 62.0, -4096.0, 4096.0, "Where the steps begin, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("high", "High", Unit::Blocks, 300.0, -4096.0, 4096.0, "Where they end, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 1.0, "How much the steps replace the slope: 0 leaves the relief as it is."),
        ParamSpec::choice("range", "Range", RANGES, "given", "Whether the steps go from Low to High, or from the input's lowest point to its highest.").section(Section::Advanced),
    ],
    kernel: Kernel::Stats { cpu: terrace_cpu },
    ..NodeSpec::BASE
};

fn terrace_cpu(ctx: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let src = inputs[0].expect("terraces have their input");
    let (lo, hi) = range_of(p, stats, (p.f("low"), p.f("high")));
    let steps = p.int("steps").max(1) as f32;
    let strength = p.f("strength");
    // The share of each step that is riser.
    let riser = (1.0 - p.f("sharpness")).clamp(0.02, 1.0);
    for (o, &v) in out[0].iter_mut().zip(src) {
        if v <= lo || v >= hi || hi <= lo {
            *o = v;
            continue;
        }
        let t = (v - lo) / (hi - lo) * steps;
        let i = t.floor();
        let f = t - i;
        let r = ((f - (1.0 - riser)) / riser).clamp(0.0, 1.0);
        let s = r * r * (3.0 - 2.0 * r);
        let stepped = lo + (hi - lo) * (i + s) / steps;
        *o = v + (stepped - v) * strength;
    }
}

pub static CLAMP: NodeSpec = NodeSpec {
    id: "adjust.clamp",
    label: "Clamp",
    family: Family::Adjust,
    summary: "A relief held between two heights: nothing under Min, nothing over Max.",
    help: "Cuts the tops off at Max (flat plateaus, a mesa's cap) and fills the bottoms up to Min (a sea floor, a valley's flat).",
    keywords: &["clamp", "limit", "cap", "floor", "ceiling", "cut"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("min", "Min", Unit::Blocks, -64.0, -4096.0, 4096.0, "The lowest the relief goes, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("max", "Max", Unit::Blocks, 320.0, -4096.0, 4096.0, "The highest it goes, in blocks.").soft(-64.0, 320.0),
    ],
    kernel: Kernel::Point { cpu: clamp_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn clamp_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("clamp has its input");
    let (lo, hi) = (ctx.params.f("min"), ctx.params.f("max"));
    for (o, &v) in out[0].iter_mut().zip(src) {
        *o = v.max(lo).min(hi.max(lo));
    }
}

// ------------------------------------------------------- fold, invert --

pub static FOLD: NodeSpec = NodeSpec {
    id: "adjust.fold",
    label: "Fold",
    family: Family::Adjust,
    summary: "Mirrors what rises over a level back down: ridges folded into crests and troughs.",
    help: "Everything above Level is reflected under it, as if the relief were folded there: a smooth hill becomes a ring of ridges, noise becomes sharp crests. Strength mixes the fold with the relief as it was.",
    keywords: &["fold", "mirror", "reflect", "ridges"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("level", "Level", Unit::Blocks, 120.0, -4096.0, 4096.0, "Where the relief is folded, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 1.0, "How much of the fold is taken."),
    ],
    kernel: Kernel::Point { cpu: fold_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn fold_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("fold has its input");
    let (level, s) = (ctx.params.f("level"), ctx.params.f("strength"));
    for (o, &v) in out[0].iter_mut().zip(src) {
        let folded = if v > level { 2.0 * level - v } else { v };
        *o = v + (folded - v) * s;
    }
}

pub static INVERT: NodeSpec = NodeSpec {
    id: "adjust.invert",
    label: "Invert",
    family: Family::Adjust,
    summary: "Turns a relief upside down within its own range: its highest point where its lowest was.",
    help: "Valleys become ridges and peaks become pits, between the same lowest and highest heights, measured over the whole terrain. For a mask, Invert in Masks is 1 − the mask.",
    keywords: &["invert", "flip", "upside down", "negative"],
    inputs: &[IN],
    outputs: &[OUT],
    kernel: Kernel::Stats { cpu: invert_cpu },
    ..NodeSpec::BASE
};

fn invert_cpu(_: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let s = stats[0].expect("invert has its input");
    let src = inputs[0].expect("invert has its input");
    for (o, &v) in out[0].iter_mut().zip(src) {
        *o = s.min + s.max - v;
    }
}

// -------------------------------------------------------------- stratify --

pub static STRATIFY: NodeSpec = NodeSpec {
    id: "adjust.stratify",
    label: "Strata",
    family: Family::Adjust,
    summary: "Layers of rock of uneven thickness cut into a relief: ledges on the slopes, as sedimentary rock weathers.",
    help: "Layers is how many beds the relief's range is cut into, of thicknesses that vary; Strength how flat their ledges are; Tilt leans the beds across the terrain, from west to east. Works over the input's own range.",
    keywords: &["strata", "layers", "sedimentary", "ledges", "beds"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::int("layers", "Layers", 14, 1, 200, "How many beds the range is cut into."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 0.7, 0.0, 1.0, "How flat their ledges are."),
        ParamSpec::float("tilt", "Tilt", Unit::Degrees, 6.0, -60.0, 60.0, "How much the beds lean, from west to east."),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Stats { cpu: stratify_cpu },
    ..NodeSpec::BASE
};

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn stratify_cpu(ctx: &Ctx, stats: &[Option<Stats>], inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let s = stats[0].expect("strata have their input");
    let src = inputs[0].expect("strata have their input");
    let (lo, span) = (s.min, (s.max - s.min).max(1e-6));
    let layers = p.int("layers").clamp(1, 200) as u32;
    // Bed boundaries from 0 to 1, of uneven thickness.
    let thick: Vec<f32> = (0..layers)
        .map(|k| {
            let r = crate::util::mix32(k.wrapping_mul(0x9e37_79b9) ^ ctx.seed) as f32 / u32::MAX as f32;
            0.35 + r * r * 1.6
        })
        .collect();
    let total: f32 = thick.iter().sum();
    let mut bounds = vec![0.0f32];
    let mut acc = 0.0;
    for t in &thick {
        acc += t / total;
        bounds.push(acc);
    }
    let strength = p.f("strength");
    let tan = p.f("tilt").to_radians().tan();
    let size = ctx.space.size as f32;
    let w = ctx.rect.w as usize;
    for (k, (o, &v)) in out[0].iter_mut().zip(src).enumerate() {
        let (x, _) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z);
        let shift = tan * (x as f32 / size - 0.5) * size / span;
        let t = (v - lo) / span + shift;
        let i = bounds.partition_point(|b| *b <= t).clamp(1, bounds.len() - 1);
        let (b0, b1) = (bounds[i - 1], bounds[i]);
        let u = ((t - b0) / (b1 - b0).max(1e-6)).clamp(0.0, 1.0);
        let stepped = b0 + (b1 - b0) * smoothstep(0.7, 1.0, u);
        *o = lo + (t + (stepped - t) * strength - shift) * span;
    }
}

// ------------------------------------------------------ sharpen, warp --

pub static SHARPEN: NodeSpec = NodeSpec {
    id: "adjust.sharpen",
    label: "Sharpen",
    family: Family::Adjust,
    summary: "Brings out a relief's detail: what stands above its surroundings rises, what lies below sinks.",
    help: "The relief minus its blur over Radius, times Amount, is added back: crests and gullies about Radius wide stand out more. Large amounts make a relief look carved.",
    keywords: &["sharpen", "unsharp", "detail", "crisp", "enhance"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("radius", "Radius", Unit::Blocks, 24.0, 1.0, 1024.0, "The size of the detail brought out, in blocks.").soft(2.0, 256.0),
        ParamSpec::float("amount", "Amount", Unit::Ratio, 0.8, 0.0, 8.0, "How much it is brought out.").soft(0.0, 3.0),
    ],
    kernel: Kernel::Local { cpu: sharpen_cpu, gpu: None, footprint: sharpen_footprint },
    ..NodeSpec::BASE
};

fn sharpen_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    let s = sigma(p) / Space::spacing(lod);
    Footprint { lod, halo: (3.0 * s).ceil() as u32 + 1 }
}

fn sharpen_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("sharpen has its input");
    let s = sigma(ctx.params) / Space::spacing(src.lod);
    let blurred = crate::filters::blur(src, s);
    let amount = ctx.params.f("amount");
    let w = ctx.rect.w as usize;
    for (k, o) in out[0].iter_mut().enumerate() {
        let (x, z) = (ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let v = src.at(x, z);
        *o = v + (v - blurred.at(x, z)) * amount;
    }
}

pub static WARP: NodeSpec = NodeSpec {
    id: "adjust.warp",
    label: "Warp",
    family: Family::Adjust,
    summary: "Pushes a relief around by a noise: straight lines wander, round shapes twist, everything looks less made.",
    help: "Each place takes the height found up to Amount blocks away, in a direction that changes smoothly over Scale. Small amounts take the regularity off shapes; large ones swirl them like rock under pressure.",
    keywords: &["warp", "distort", "displace", "domain", "twist"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("amount", "Amount", Unit::Blocks, 60.0, 0.0, 4096.0, "How far the relief is pushed, in blocks.").soft(0.0, 600.0),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 400.0, 4.0, 65536.0, "The size of the pushes, in blocks.").soft(32.0, 4000.0),
        ParamSpec::int("octaves", "Detail", 3, 1, 8, "How much finer turns the pushes have.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Local { cpu: warp_cpu, gpu: None, footprint: warp_footprint },
    ..NodeSpec::BASE
};

fn warp_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    Footprint { lod, halo: (p.num("amount") / Space::spacing(lod)).ceil() as u32 + 2 }
}

fn warp_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let src = inputs[0].expect("warp has its input");
    let amount = p.num("amount");
    let octaves = p.int("octaves").clamp(1, 8) as u32;
    let (nx, nz) = (Fbm::new(p.num("scale"), octaves, 0.5, ctx.seed), Fbm::new(p.num("scale"), octaves, 0.5, ctx.seed ^ 0x5f3c_1a2b));
    let spacing = ctx.spacing();
    let w = ctx.rect.w as usize;
    out[0].par_iter_mut().enumerate().for_each(|(k, o)| {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let dx = (nx.at(x, z) as f64 * 1.6).clamp(-1.0, 1.0) * amount;
        let dz = (nz.at(x, z) as f64 * 1.6).clamp(-1.0, 1.0) * amount;
        *o = src.bilinear((x + dx) / spacing, (z + dz) / spacing);
    });
}

// ----------------------------------------------------- transform, swirl --

pub static TRANSFORM: NodeSpec = NodeSpec {
    id: "adjust.transform",
    label: "Transform",
    family: Family::Adjust,
    summary: "Scales, turns and moves a relief over the terrain.",
    help: "Scale above 1 enlarges the relief around the terrain's middle; Rotate turns it, in degrees clockwise; Offset moves it, as fractions of the terrain. Worked over the whole terrain at the simulations' resolution.",
    keywords: &["transform", "scale", "rotate", "move", "offset", "zoom"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("scale", "Scale", Unit::Ratio, 1.0, 0.01, 100.0, "How much larger the relief is drawn.").soft(0.25, 4.0),
        ParamSpec::float("rotate", "Rotate", Unit::Degrees, 0.0, -360.0, 360.0, "How far it is turned, clockwise."),
        ParamSpec::float("offset_x", "Offset east", Unit::Fraction, 0.0, -1.0, 1.0, "How far it is moved east, as a fraction of the terrain."),
        ParamSpec::float("offset_z", "Offset south", Unit::Fraction, 0.0, -1.0, 1.0, "How far it is moved south, as a fraction of the terrain."),
    ],
    kernel: Kernel::Global { cpu: transform_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

/// The whole input resampled: each sample takes the input at `from(u, v)`,
/// positions as fractions of the terrain from its middle.
fn remap(ctx: &Ctx, src: &Grid, from: &(dyn Fn(f64, f64) -> (f64, f64) + Sync)) -> Vec<f32> {
    let w = ctx.rect.w as usize;
    let (size, spacing) = (ctx.space.size as f64, ctx.spacing());
    (0..ctx.rect.area())
        .into_par_iter()
        .map(|k| {
            let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
            let (u, v) = from(x / size - 0.5, z / size - 0.5);
            src.bicubic((u + 0.5) * size / spacing, (v + 0.5) * size / spacing)
        })
        .collect()
}

fn transform_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("transform has its input");
    let p = ctx.params;
    let (s, c) = (-p.num("rotate").to_radians()).sin_cos();
    let k = 1.0 / p.num("scale").max(0.01);
    let (ox, oz) = (p.num("offset_x"), p.num("offset_z"));
    Some(vec![remap(ctx, src, &|u, v| {
        let (u, v) = (u - ox, v - oz);
        ((u * c - v * s) * k, (u * s + v * c) * k)
    })])
}

pub static SWIRL: NodeSpec = NodeSpec {
    id: "adjust.swirl",
    label: "Swirl",
    family: Family::Adjust,
    summary: "Twists a relief around a point, most at the middle, not at all at Radius.",
    help: "Angle is how far the middle turns, in degrees; Radius how far the twist reaches, as a fraction of the terrain; X and Z where its middle is. Worked over the whole terrain at the simulations' resolution.",
    keywords: &["swirl", "twist", "vortex", "spiral"],
    inputs: &[IN],
    outputs: &[OUT],
    params: &[
        ParamSpec::float("angle", "Angle", Unit::Degrees, 120.0, -1080.0, 1080.0, "How far the middle turns."),
        ParamSpec::float("radius", "Radius", Unit::Fraction, 0.5, 0.01, 2.0, "How far the twist reaches, as a fraction of the terrain."),
        ParamSpec::float("x", "X", Unit::Fraction, 0.5, -1.0, 2.0, "Where its middle is, west to east."),
        ParamSpec::float("z", "Z", Unit::Fraction, 0.5, -1.0, 2.0, "Where its middle is, north to south."),
    ],
    kernel: Kernel::Global { cpu: swirl_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

fn swirl_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("swirl has its input");
    let p = ctx.params;
    let (angle, r0) = (p.num("angle").to_radians(), p.num("radius").max(0.01));
    let (cx, cz) = (p.num("x") - 0.5, p.num("z") - 0.5);
    Some(vec![remap(ctx, src, &|u, v| {
        let (u, v) = (u - cx, v - cz);
        let r = u.hypot(v);
        if r >= r0 {
            return (u + cx, v + cz);
        }
        let t = 1.0 - r / r0;
        let (s, c) = (-(angle * t * t)).sin_cos();
        (u * c - v * s + cx, u * s + v * c + cz)
    })])
}
