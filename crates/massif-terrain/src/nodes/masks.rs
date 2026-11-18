//! Masks: where something is, from 0 to 1.

use crate::kernel::{Ctx, Footprint, Grid};
use crate::params::Params;
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&SLOPE];

pub static SLOPE: NodeSpec = NodeSpec {
    id: "mask.slope",
    label: "Slope",
    family: Family::Mask,
    summary: "Where the ground is steep: 0 below From degrees, 1 above To, a smooth step between.",
    help: "Picks slopes by their steepness: cliffs for rock, gentle ground for grass and snow (inverted). The slope is measured between neighbouring samples, so a coarse preview shows slopes a little gentler than the final build.",
    keywords: &["steep", "cliff", "gradient", "angle"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to measure.")],
    outputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "1 where it is steep.")],
    params: &[
        ParamSpec::float("from", "From", Unit::Degrees, 25.0, 0.0, 90.0, "Slopes gentler than this are 0."),
        ParamSpec::float("to", "To", Unit::Degrees, 45.0, 0.0, 90.0, "Slopes steeper than this are 1."),
    ],
    kernel: Kernel::Local { cpu: slope_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: 1 } },
    ..NodeSpec::BASE
};

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if (b - a).abs() < 1e-6 {
        return if x >= a { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn slope_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("slope has its input");
    let (from, to) = (ctx.params.f("from"), ctx.params.f("to"));
    let inv = 1.0 / (2.0 * ctx.spacing() as f32);
    let r = ctx.rect;
    for (j, row) in out[0].chunks_mut(r.w as usize).enumerate() {
        let z = r.z + j as i32;
        for (i, o) in row.iter_mut().enumerate() {
            let x = r.x + i as i32;
            let dx = (h.at(x + 1, z) - h.at(x - 1, z)) * inv;
            let dz = (h.at(x, z + 1) - h.at(x, z - 1)) * inv;
            let deg = (dx * dx + dz * dz).sqrt().atan().to_degrees();
            *o = smoothstep(from, to, deg);
        }
    }
}
