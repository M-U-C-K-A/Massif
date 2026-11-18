//! Adjust: reshaping one relief.

use crate::kernel::{Ctx, Footprint, GpuKernel, Grid, Stats, Uniforms};
use crate::params::Params;
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Unit};
use crate::space::Space;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&LEVELS, &NORMALIZE, &BLUR];

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
