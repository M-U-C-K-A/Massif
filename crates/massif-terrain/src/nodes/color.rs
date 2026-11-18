//! Colours of the ground.

use crate::kernel::Ctx;
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&RAMP];

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
