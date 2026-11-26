//! Combine: several reliefs into one.

use super::height_out;
use crate::kernel::{Ctx, GpuKernel, Uniforms};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&COMBINE, &BLEND];

const MODES: &[Choice] = &[
    Choice { value: "add", label: "Add", help: "A + B: B raised on top of A." },
    Choice { value: "subtract", label: "Subtract", help: "A − B: B dug out of A." },
    Choice { value: "multiply", label: "Multiply", help: "A × B: with a mask, A where the mask is 1 and 0 where it is 0." },
    Choice { value: "max", label: "Max", help: "The higher of A and B: two landforms side by side." },
    Choice { value: "min", label: "Min", help: "The lower of A and B: one relief cut down by another." },
    Choice { value: "average", label: "Average", help: "Halfway between A and B." },
    Choice { value: "difference", label: "Difference", help: "How far A and B lie apart." },
    Choice { value: "screen", label: "Screen", help: "1 − (1 − A)(1 − B), for masks: either one." },
    Choice { value: "overlay", label: "Overlay", help: "For masks: B deepens A's contrast." },
    Choice { value: "replace", label: "Replace", help: "B: with a mask, B where the mask is 1 and A elsewhere." },
];

pub static BLEND: NodeSpec = NodeSpec {
    id: "combine.blend",
    label: "Blend",
    family: Family::Combine,
    summary: "Combines two reliefs, A and B, by a mode, through a mask where one is given.",
    help: "The result is A where the mask is 0 (or Strength is 0), and A combined with B by the mode where it is 1. Add raises mountains on a plain, Max sets landforms side by side, Replace with a mask puts one landscape inside another.",
    keywords: &["mix", "add", "max", "lerp", "layer", "merge", "combine"],
    inputs: &[
        PortSpec::new("a", "A", PortKind::Height, "The relief combined into."),
        PortSpec::new("b", "B", PortKind::Height, "The relief combined in."),
        PortSpec::new("mask", "Mask", PortKind::Mask, "Where B is combined (everywhere when unconnected).").optional(),
    ],
    outputs: &[height_out("A combined with B.")],
    params: &[
        ParamSpec::choice("mode", "Mode", MODES, "add", "How A and B combine."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 1.0, "How much of the combination is taken: 0 leaves A."),
    ],
    kernel: Kernel::Point { cpu: blend_cpu, gpu: Some(&BLEND_GPU) },
    ..NodeSpec::BASE
};

fn mode(ctx: &Ctx) -> u32 {
    MODES.iter().position(|m| m.value == ctx.params.choice("mode")).unwrap_or(0) as u32
}

#[inline]
fn blend(mode: u32, a: f32, b: f32) -> f32 {
    match mode {
        0 => a + b,
        1 => a - b,
        2 => a * b,
        3 => a.max(b),
        4 => a.min(b),
        5 => (a + b) * 0.5,
        6 => (a - b).abs(),
        7 => 1.0 - (1.0 - a) * (1.0 - b),
        8 => {
            if a < 0.5 {
                2.0 * a * b
            } else {
                1.0 - 2.0 * (1.0 - a) * (1.0 - b)
            }
        }
        _ => b,
    }
}

fn blend_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let m = mode(ctx);
    let strength = ctx.params.f("strength");
    let (a, b) = (inputs[0].expect("blend has A"), inputs[1].expect("blend has B"));
    match inputs[2] {
        Some(mask) => {
            for (k, o) in out[0].iter_mut().enumerate() {
                let w = strength * mask[k].clamp(0.0, 1.0);
                *o = a[k] + (blend(m, a[k], b[k]) - a[k]) * w;
            }
        }
        None => {
            for (k, o) in out[0].iter_mut().enumerate() {
                *o = a[k] + (blend(m, a[k], b[k]) - a[k]) * strength;
            }
        }
    }
}

static BLEND_GPU: GpuKernel = GpuKernel {
    name: "combine.blend",
    source: include_str!("wgsl/blend.wgsl"),
    uniforms: |ctx, connected| Uniforms::default().u32(mode(ctx)).f32(ctx.params.f("strength")).u32(connected[2] as u32).finish(),
};

// ---------------------------------------------------------------- combine --

const LAYER_MODES: &[Choice] = &[
    Choice { value: "blend", label: "Blend", help: "Each input laid over the ones before, through its mask: where the mask is 1, the input." },
    Choice { value: "add", label: "Add", help: "Each input raised on top of the ones before." },
    Choice { value: "subtract", label: "Subtract", help: "Each input dug out of the ones before." },
    Choice { value: "max", label: "Max", help: "The highest of the inputs: landforms side by side." },
    Choice { value: "min", label: "Min", help: "The lowest of the inputs: one relief cut down by the others." },
    Choice { value: "multiply", label: "Multiply", help: "The inputs multiplied: masks cutting each other." },
    Choice { value: "average", label: "Average", help: "The mean of the inputs, each weighted by its mask." },
    Choice { value: "difference", label: "Difference", help: "How far each input lies from the ones before." },
    Choice { value: "screen", label: "Screen", help: "1 − (1 − A)(1 − B), for masks: any of them." },
    Choice { value: "overlay", label: "Overlay", help: "For masks: each input deepens the contrast of the ones before." },
    Choice { value: "power", label: "Power", help: "For masks: the ones before raised to the power of each input, which darkens them where it is high." },
];

pub static COMBINE: NodeSpec = NodeSpec {
    id: "combine.layers",
    label: "Combine",
    family: Family::Combine,
    summary: "Combines any number of reliefs or masks into one by a mode, each input through its own mask.",
    help: "Connect the reliefs to In 1, In 2 and on: a new empty input appears as soon as the last one is connected. In 1 is the base; each input after it combines with what is above it by the mode, as much as its mask says (everywhere when its mask is unconnected) and Strength. A Constant mask gives an input a weight of its own. Max sets landforms side by side, Add raises them on a plain, Blend with masks lays one landscape into another.",
    keywords: &["combine", "mix", "mixer", "layers", "merge", "add", "max", "blend", "stack"],
    open_inputs: Some(PortSpec::new("in.", "In", PortKind::Height, "A relief or a mask combined in, in the order of the numbers.").optional()),
    open_masks: Some(PortSpec::new("mask.", "Mask", PortKind::Mask, "Where the input of the same number is combined, and how much (everywhere when unconnected).").optional()),
    outputs: &[height_out("The inputs combined.")],
    params: &[
        ParamSpec::choice("mode", "Mode", LAYER_MODES, "max", "How each input combines with the ones before it."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 1.0, "How much of each input after the first is taken: 0 leaves In 1."),
    ],
    kernel: Kernel::Point { cpu: combine_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// Mode `m`, by its place in [`LAYER_MODES`].
#[inline]
fn layer(m: u8, a: f32, b: f32) -> f32 {
    match m {
        1 => a + b,
        2 => a - b,
        3 => a.max(b),
        4 => a.min(b),
        5 => a * b,
        7 => (a - b).abs(),
        8 => 1.0 - (1.0 - a) * (1.0 - b),
        9 => {
            if a < 0.5 {
                2.0 * a * b
            } else {
                1.0 - 2.0 * (1.0 - a) * (1.0 - b)
            }
        }
        10 => a.max(0.0).powf(b.max(0.0)),
        _ => b,
    }
}

/// Its inputs come in pairs, in order: each relief, then its mask.
fn combine_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let mode = ctx.params.choice("mode");
    let m = LAYER_MODES.iter().position(|c| c.value == mode).unwrap_or(0) as u8;
    let strength = ctx.params.f("strength");
    let layers: Vec<(&[f32], Option<&[f32]>)> = inputs.chunks(2).filter_map(|p| p[0].map(|v| (v, p.get(1).copied().flatten()))).collect();
    let o = &mut out[0];
    let Some(((base, _), rest)) = layers.split_first() else {
        o.fill(0.0);
        return;
    };
    o.copy_from_slice(base);
    if mode == "average" {
        let mut weight = vec![1.0f32; o.len()];
        for (v, mask) in rest {
            for (k, (o, w)) in o.iter_mut().zip(weight.iter_mut()).enumerate() {
                let m = strength * mask.map_or(1.0, |m| m[k].clamp(0.0, 1.0));
                *o += v[k] * m;
                *w += m;
            }
        }
        for (o, w) in o.iter_mut().zip(&weight) {
            *o /= w;
        }
        return;
    }
    for (v, mask) in rest {
        match mask {
            Some(w) => {
                for (k, o) in o.iter_mut().enumerate() {
                    *o += (layer(m, *o, v[k]) - *o) * strength * w[k].clamp(0.0, 1.0);
                }
            }
            None => {
                for (k, o) in o.iter_mut().enumerate() {
                    *o += (layer(m, *o, v[k]) - *o) * strength;
                }
            }
        }
    }
}
