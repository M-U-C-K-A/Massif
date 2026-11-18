//! Conversions a compiled graph puts between sockets of different types:
//! the dot on the wire.

use crate::kernel::Ctx;
use crate::registry::{Family, Kernel, NodeSpec, PortKind, PortSpec};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&LUMINANCE, &GREY];

pub static LUMINANCE: NodeSpec = NodeSpec {
    id: "convert.luminance",
    label: "Brightness",
    family: Family::Internal,
    summary: "A colour's brightness, as a number from 0 to 1.",
    help: "Put by the graph where a colour feeds a height or a mask.",
    keywords: &[],
    inputs: &[PortSpec::new("color", "Colour", PortKind::Color, "The colour.")],
    outputs: &[PortSpec::new("value", "Value", PortKind::Mask, "Its brightness.")],
    params: &[],
    kernel: Kernel::Point { cpu: luminance_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn luminance_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let c = inputs[0].expect("a conversion has its input");
    let n = ctx.rect.area();
    for (k, o) in out[0].iter_mut().enumerate() {
        *o = 0.2126 * c[k] + 0.7152 * c[n + k] + 0.0722 * c[2 * n + k];
    }
}

pub static GREY: NodeSpec = NodeSpec {
    id: "convert.grey",
    label: "Grey",
    family: Family::Internal,
    summary: "A number as a grey: 0 black, 1 white.",
    help: "Put by the graph where a height or a mask feeds a colour.",
    keywords: &[],
    inputs: &[PortSpec::new("value", "Value", PortKind::Mask, "The number.")],
    outputs: &[PortSpec::new("color", "Colour", PortKind::Color, "The grey.")],
    params: &[],
    kernel: Kernel::Point { cpu: grey_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn grey_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let v = inputs[0].expect("a conversion has its input");
    let n = ctx.rect.area();
    for p in 0..3 {
        out[0][p * n..(p + 1) * n].copy_from_slice(v);
    }
}
