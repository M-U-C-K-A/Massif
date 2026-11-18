//! Primitives: what makes relief from nothing.

use super::height_out;
use crate::kernel::{Ctx, GpuKernel, Uniforms};
use crate::noise::{Fractal, MAX_OCTAVES, Octave, Style, fractal};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, Section, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&CONSTANT, &FBM];

pub static CONSTANT: NodeSpec = NodeSpec {
    id: "pattern.constant",
    label: "Constant",
    family: Family::Primitive,
    summary: "The same height everywhere.",
    help: "A flat ground at one height: a start to build on, or a level to combine with another relief.",
    keywords: &["flat", "value", "plane"],
    inputs: &[],
    outputs: &[height_out("The height, everywhere.")],
    params: &[ParamSpec::float("value", "Height", Unit::Blocks, 64.0, -2048.0, 4096.0, "The height of the ground everywhere, in blocks.").soft(-64.0, 320.0)],
    kernel: Kernel::Point { cpu: constant_cpu, gpu: Some(&CONSTANT_GPU) },
    ..NodeSpec::BASE
};

fn constant_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    out[0].fill(ctx.params.f("value"));
}

static CONSTANT_GPU: GpuKernel = GpuKernel {
    name: "pattern.constant",
    source: include_str!("wgsl/constant.wgsl"),
    uniforms: |ctx, _| Uniforms::default().f32(ctx.params.f("value")).finish(),
};

const STYLES: &[Choice] = &[
    Choice { value: "fbm", label: "Rolling", help: "Octaves summed: smooth hills and valleys." },
    Choice { value: "ridged", label: "Ridged", help: "Octaves folded into sharp crests, with finer ridges branching where the coarser ones are high: mountain ranges." },
    Choice { value: "billow", label: "Billowy", help: "Octaves folded the other way: puffy, rounded mounds with creases between them." },
];

pub static FBM: NodeSpec = NodeSpec {
    id: "noise.fbm",
    label: "Noise",
    family: Family::Primitive,
    summary: "Fractal noise: relief at every scale, from its largest features down to fine detail.",
    help: "The base of most landscapes. Scale is the size of the largest features; each octave adds detail half as large (lacunarity) and weaker (roughness). Rolling makes hills, Ridged mountain crests, Billowy rounded mounds. Height is how far the lowest and highest points lie apart, Base where the lowest lies.",
    keywords: &["fbm", "perlin", "fractal", "ridged", "billow", "hills", "random"],
    inputs: &[],
    outputs: &[height_out("The relief, from Base to Base + Height.")],
    params: &[
        ParamSpec::choice("style", "Style", STYLES, "fbm", "How the octaves add up."),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 1000.0, 2.0, 65536.0, "Blocks across the largest features.").soft(16.0, 8000.0),
        ParamSpec::float("height", "Height", Unit::Blocks, 200.0, 0.0, 4096.0, "How far the lowest and the highest points lie apart, in blocks.").soft(0.0, 1000.0).drivable(),
        ParamSpec::float("base", "Base", Unit::Blocks, 40.0, -2048.0, 4096.0, "The height of the lowest points, in blocks.").soft(-64.0, 320.0).drivable(),
        ParamSpec::int("octaves", "Detail", 8, 1, MAX_OCTAVES as i64, "Octaves summed: each adds detail twice as fine.").section(Section::Detail),
        ParamSpec::float("roughness", "Roughness", Unit::Ratio, 0.5, 0.05, 0.95, "How strong each octave is next to the one before: higher, rougher.").section(Section::Detail),
        ParamSpec::float("lacunarity", "Lacunarity", Unit::Ratio, 2.0, 1.2, 4.0, "How much finer each octave is than the one before.").section(Section::Advanced),
        ParamSpec::float("offset_x", "Offset X", Unit::Blocks, 0.0, -1.0e6, 1.0e6, "Moves the noise east, in blocks.").soft(-10000.0, 10000.0).section(Section::Advanced),
        ParamSpec::float("offset_z", "Offset Z", Unit::Blocks, 0.0, -1.0e6, 1.0e6, "Moves the noise south, in blocks.").soft(-10000.0, 10000.0).section(Section::Advanced),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: fbm_cpu, gpu: Some(&FBM_GPU) },
    ..NodeSpec::BASE
};

fn style(ctx: &Ctx) -> Style {
    match ctx.params.choice("style") {
        "ridged" => Style::Ridged,
        "billow" => Style::Billow,
        _ => Style::Fbm,
    }
}

fn octaves(ctx: &Ctx) -> Vec<Octave> {
    let p = ctx.params;
    let f = Fractal {
        scale: p.num("scale"),
        octaves: p.int("octaves") as u32,
        lacunarity: p.num("lacunarity"),
        gain: p.num("roughness"),
        offset: (p.num("offset_x"), p.num("offset_z")),
        seed: ctx.seed,
    };
    f.octaves(ctx.world(ctx.rect.x, ctx.rect.z), ctx.spacing())
}

fn fbm_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let octs = octaves(ctx);
    let style = style(ctx);
    let w = ctx.rect.w as usize;
    for (j, row) in out[0].chunks_mut(w).enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            let k = j * w + i;
            *v = ctx.num_at("base", k) + ctx.num_at("height", k) * fractal(&octs, style, i as f32, j as f32);
        }
    }
}

static FBM_GPU: GpuKernel = GpuKernel {
    name: "noise.fbm",
    source: include_str!("wgsl/fbm.wgsl"),
    uniforms: |ctx, driven| {
        let octs = octaves(ctx);
        let mut u = Uniforms::default();
        u.u32(style(ctx) as u32).u32(octs.len() as u32).f32(ctx.params.f("height")).f32(ctx.params.f("base"));
        u.u32(driven[0] as u32).u32(driven[1] as u32);
        for k in 0..MAX_OCTAVES {
            octs.get(k).copied().unwrap_or_default().write(&mut u);
        }
        u.finish()
    },
};
