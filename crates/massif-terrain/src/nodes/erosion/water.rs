//! Water over a relief: where it drains (`water.flow`), and later the
//! rivers, lakes and sea it makes.

use super::common::Kit;
use super::flow::{self, FlowGpu, accumulate, rain_scale, receivers};
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid};
use rayon::prelude::*;

/// The flow mask's exponent for a Small streams setting.
fn gamma(spread: f32) -> f32 {
    3.0 + (0.6 - 3.0) * spread.clamp(0.0, 1.0)
}

pub(super) fn flow_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("flow has its input");
    let n = src.rect.w as usize;
    let h = src.plane(0);
    let sea = ctx.space.sea_level;
    let w = flow::fill(h, n, sea, 1e-3 * ctx.spacing() as f32, ctx.seed, cancel)?;
    let rcv = receivers(h, &w, n, sea, ctx.seed ^ 0x5bd1_e995);
    let acc = accumulate(&rcv, inputs.get(1).copied().flatten().map(|g| g.plane(0)), n);
    let (scale, g) = (rain_scale(n), gamma(ctx.params.f("spread")));
    Some(vec![acc.par_iter().map(|&a| flow::flow_mask(a as f32 / scale, n, g)).collect()])
}

pub(super) fn flow_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let sea = ctx.space.sea_level;
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let fl = FlowGpu::new(&k)?;
    let w = fl.fill(&mut k, &src.buffer, n, sea, 1e-3 * ctx.spacing() as f32, ctx.seed, cancel)?;
    let (rcv, acc) = (k.buf("flow receivers", n * n), k.buf("flow drainage", n * n));
    fl.receivers(&mut k, &src.buffer, &w, n, sea, ctx.seed ^ 0x5bd1_e995, &rcv);
    fl.accumulate(&mut k, &rcv, inputs.get(1).copied().flatten().map(|g| &g.buffer), n, &acc);
    let out = g.grid(src.lod, src.rect, 1);
    fl.mask(&mut k, &acc, n, gamma(ctx.params.f("spread")), &out.buffer);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("water.flow", e);
        return None;
    }
    Some(vec![out])
}
