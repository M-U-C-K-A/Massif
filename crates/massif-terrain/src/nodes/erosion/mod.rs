//! Erosion: what water, frost, ice and wind do to a relief, over the whole
//! terrain. Owned by the erosion session (M3): its nodes are simulations
//! (`Kernel::Global`), each with a CPU kernel and, where it can, a GPU one
//! ([`crate::kernel::GlobalGpuFn`]); its WGSL lives in this folder.

mod common;
mod flow;
mod stream;
mod water;

use crate::nodes::height_out;
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use rayon::prelude::*;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&STREAM, &THERMAL, &FLOW];

/// The masks every erosion node gives beside its height, named as the Map
/// Output's inputs are.
const FLOW_OUT: PortSpec = PortSpec::new("flow", "Flow", PortKind::Mask, "Where water runs: 1 in the largest rivers, fading up the smallest rills.");
const WEAR_OUT: PortSpec = PortSpec::new("wear", "Wear", PortKind::Mask, "Where the water wore the ground away, the most where it cut the most: river beds, gullies.");
const DEPOSITION_OUT: PortSpec = PortSpec::new("deposition", "Deposition", PortKind::Mask, "Where what was worn away came to rest: fans, valley floors, deltas.");
const SEDIMENT_OUT: PortSpec = PortSpec::new("sediment", "Sediment", PortKind::Mask, "How much sediment the water carries at the end: muddy rivers, gravel beds.");

const HARDNESS_IN: PortSpec = PortSpec::new("hardness", "Hardness", PortKind::Mask, "Rock that resists: 1 wears twenty times slower than 0.").or("soft rock everywhere");
const RAIN_IN: PortSpec = PortSpec::new("rain", "Rain", PortKind::Mask, "How much rain falls where: a rainfall map, a rain shadow.").or("the same rain everywhere");

pub static STREAM: NodeSpec = NodeSpec {
    id: "erosion.stream",
    label: "Stream erosion",
    family: Family::Simulate,
    summary: "Rivers cut valleys that branch up into every slope, leaving sharp ridges between them, and lay what they carry down where they slow.",
    help: "The erosion that makes mountains look like mountains: water gathers into streams, the more of it the deeper they cut (stream power), their walls fall back to the talus angle, and the sediment settles at the feet of slopes, on valley floors and at the sea. Use it after shapes and noise to turn lumps into ranges drained by real valleys; follow it with Particle erosion for small gullies and Thermal erosion for scree. Strength is how far the rivers cut back into the land; Hardness keeps some rock standing; Rain gives more water where it falls.",
    keywords: &["fluvial", "rivers", "valleys", "drainage", "dendritic", "stream power", "incision", "erosion"],
    inputs: &[
        PortSpec::new("height", "Height", PortKind::Height, "The relief to erode."),
        HARDNESS_IN,
        RAIN_IN,
        PortSpec::new("uplift", "Uplift", PortKind::Mask, "Where the land rises while the rivers cut, as much as Rebuild says.").or("the relief itself: the higher, the more"),
    ],
    outputs: &[height_out("The relief eroded."), FLOW_OUT, WEAR_OUT, DEPOSITION_OUT, SEDIMENT_OUT],
    params: &[
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 20.0, "How hard the rivers cut: higher, deeper valleys reaching further up into the land.").soft(0.0, 4.0),
        ParamSpec::float("rebuild", "Rebuild", Unit::Ratio, 1.0, 0.0, 1.0, "How much of the relief rises while the rivers cut, then is brought back to its height: 1 turns any shape into mountains carved by their rivers; 0 only cuts valleys into it, down to the sea and the low ground."),
        ParamSpec::int("iterations", "Iterations", 200, 10, 2000, "Steps at each level of the simulation: more, finer valleys reaching further, slower."),
        ParamSpec::float("talus", "Talus angle", Unit::Degrees, 55.0, 10.0, 80.0, "The steepest the valleys' walls stand: lower, wider valleys and rounder ridges."),
        ParamSpec::float("deposition", "Deposition", Unit::Ratio, 0.5, 0.0, 1.0, "How readily the water drops what it carries where it slows: fans, filled valleys, deltas."),
        ParamSpec::float("channel", "Smallest stream", Unit::Blocks, 24.0, 0.0, 512.0, "The side of the smallest area that drains into a stream: under it the ground only creeps and slides.").section(Section::Detail),
        ParamSpec::float("diffusion", "Creep", Unit::Ratio, 0.0, 0.0, 1.0, "How much soil creeps downhill and rounds the land between the streams.").section(Section::Detail),
        ParamSpec::float("m", "Area exponent", Unit::Ratio, 0.5, 0.2, 0.8, "How much more large rivers cut than small ones (m of K·A^m·S^n).").section(Section::Advanced),
        ParamSpec::float("n", "Slope exponent", Unit::Ratio, 1.0, 0.5, 2.0, "How much more steep streams cut than gentle ones (n of K·A^m·S^n).").section(Section::Advanced),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: stream::stream_cpu, gpu: Some(stream::stream_gpu), carry: &[Some(0), None, None, None, None] },
    ..NodeSpec::BASE
};

pub static FLOW: NodeSpec = NodeSpec {
    id: "water.flow",
    label: "Flow",
    family: Family::Simulate,
    summary: "Where the rain runs off, as a mask: bright in the rivers, fading up every rill.",
    help: "Follows the rain down the relief, across hollows and flats to the sea or the terrain's edge, and measures how much land drains through each place. Use it to paint river beds, wet valley floors and gullies, or to drive vegetation. Small streams shows more of the rills next to the rivers.",
    keywords: &["drainage", "flow accumulation", "rivers", "watershed", "runoff"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the water runs over."), RAIN_IN],
    outputs: &[FLOW_OUT],
    params: &[ParamSpec::float("spread", "Small streams", Unit::Ratio, 0.5, 0.0, 1.0, "How much the small streams show next to the large rivers."), ParamSpec::seed()],
    kernel: Kernel::Global { cpu: water::flow_cpu, gpu: Some(water::flow_gpu), carry: &[None] },
    ..NodeSpec::BASE
};

pub static THERMAL: NodeSpec = NodeSpec {
    id: "erosion.thermal",
    label: "Thermal erosion",
    family: Family::Simulate,
    summary: "Loose rock slides down slopes steeper than its angle of repose, and gathers at their feet as scree.",
    help: "Wears cliffs back and fills the feet of slopes with talus, the way frost and gravity do: what is steeper than Talus angle sheds material to the lower neighbours until it is stable. More iterations go further towards that angle; Strength is how much moves at each. The Talus output is where material came to rest: scree, gravel, rubble.",
    keywords: &["talus", "scree", "weathering", "slope", "repose"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to wear.")],
    outputs: &[height_out("The relief worn."), PortSpec::new("talus", "Talus", PortKind::Mask, "Where material came to rest, by how much (1 for 2 blocks and more).")],
    params: &[
        ParamSpec::float("angle", "Talus angle", Unit::Degrees, 35.0, 5.0, 80.0, "The steepest slope loose material holds on."),
        ParamSpec::int("iterations", "Iterations", 50, 1, 1000, "How many times material slides: more, closer to the talus angle."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 0.5, 0.01, 1.0, "How much of what is too steep moves at each iteration.").section(Section::Detail),
    ],
    kernel: Kernel::Global { cpu: thermal_cpu, gpu: Some(thermal_gpu), carry: &[Some(0), None] },
    ..NodeSpec::BASE
};

/// One pass: each sample sheds `strength` of half its largest excess over
/// its neighbours' stable heights, shared among the neighbours below in
/// proportion to their excess; each sample then gathers what its
/// neighbours shed to it. Gathering rather than scattering keeps it
/// parallel and the same on any number of threads.
pub(super) fn thermal_pass(h: &[f32], next: &mut [f32], shed: &mut [f32], total: &mut [f32], n: usize, drop: [f32; 2], strength: f32) {
    const NB: [(i32, i32, usize); 8] = [(-1, -1, 1), (0, -1, 0), (1, -1, 1), (-1, 0, 0), (1, 0, 0), (-1, 1, 1), (0, 1, 0), (1, 1, 1)];
    let inside = |x: i32, z: i32| x >= 0 && z >= 0 && x < n as i32 && z < n as i32;
    shed.par_chunks_mut(n).zip(total.par_chunks_mut(n)).enumerate().for_each(|(z, (srow, trow))| {
        for x in 0..n {
            let c = h[z * n + x];
            let (mut sum, mut most) = (0.0f32, 0.0f32);
            for (dx, dz, d) in NB {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if !inside(nx, nz) {
                    continue;
                }
                let e = c - h[nz as usize * n + nx as usize] - drop[d];
                if e > 0.0 {
                    sum += e;
                    most = most.max(e);
                }
            }
            srow[x] = strength * 0.5 * most;
            trow[x] = sum;
        }
    });
    next.par_chunks_mut(n).enumerate().for_each(|(z, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let c = h[z * n + x];
            let mut v = c - shed[z * n + x];
            for (dx, dz, d) in NB {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if !inside(nx, nz) {
                    continue;
                }
                let k = nz as usize * n + nx as usize;
                // What the neighbour sheds to this sample.
                let e = h[k] - c - drop[d];
                if e > 0.0 && total[k] > 0.0 {
                    v += shed[k] * e / total[k];
                }
            }
            *out = v;
        }
    });
}

fn thermal_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("thermal erosion has its input");
    let n = src.rect.w as usize;
    debug_assert_eq!(src.rect.w, src.rect.h);
    let spacing = ctx.spacing() as f32;
    let slope = ctx.params.f("angle").to_radians().tan();
    let drop = [slope * spacing, slope * spacing * std::f32::consts::SQRT_2];
    let strength = ctx.params.f("strength");
    let mut h = src.data.clone();
    let mut next = vec![0.0; n * n];
    let (mut shed, mut total) = (vec![0.0; n * n], vec![0.0; n * n]);
    for _ in 0..ctx.params.int("iterations") {
        if cancel() {
            return None;
        }
        thermal_pass(&h, &mut next, &mut shed, &mut total, n, drop, strength);
        std::mem::swap(&mut h, &mut next);
    }
    let talus: Vec<f32> = h.par_iter().zip(&src.data).map(|(a, b)| ((a - b) / 2.0).clamp(0.0, 1.0)).collect();
    Some(vec![h, talus])
}

const THERMAL_WGSL: &str = include_str!("thermal.wgsl");

/// Thermal erosion on the graphics card: the same passes as on the CPU,
/// ping-ponging between two grids, cancellable every 16 iterations.
fn thermal_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side();
    let spacing = ctx.spacing() as f32;
    let slope = ctx.params.f("angle").to_radians().tan();
    let u = g.uniform("thermal", &Uniforms::default().u32(n).f32(ctx.params.f("strength")).f32(slope * spacing).f32(slope * spacing * std::f32::consts::SQRT_2).finish());
    let excess = g.compute("erosion.thermal", THERMAL_WGSL, "excess")?;
    let gather = g.compute("erosion.thermal", THERMAL_WGSL, "gather")?;
    let talus = g.compute("erosion.thermal", THERMAL_WGSL, "talus")?;
    let bytes = n as u64 * n as u64 * 4;
    let (shed, total) = (g.buffer("thermal shed", bytes), g.buffer("thermal total", bytes));
    let a = g.grid(src.lod, src.rect, 1);
    let b = g.grid(src.lod, src.rect, 1);
    let device = g.device();
    let group = |p: &wgpu::ComputePipeline, h: &wgpu::Buffer, next: Option<&wgpu::Buffer>| {
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: h.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: shed.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: total.as_entire_binding() },
        ];
        if let Some(next) = next {
            entries.push(wgpu::BindGroupEntry { binding: 4, resource: next.as_entire_binding() });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &p.get_bind_group_layout(0), entries: &entries })
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // The input copied to `a`; then a → b, b → a…
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("thermal") });
    enc.copy_buffer_to_buffer(&src.buffer, 0, &a.buffer, 0, bytes);
    g.queue().submit([enc.finish()]);
    let groups = [
        (group(&excess, &a.buffer, None), group(&gather, &a.buffer, Some(&b.buffer))),
        (group(&excess, &b.buffer, None), group(&gather, &b.buffer, Some(&a.buffer))),
    ];
    let wg = n.div_ceil(16);
    let iterations = ctx.params.int("iterations") as usize;
    let mut done = 0;
    while done < iterations {
        if cancel() {
            return None;
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("thermal") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("thermal"), timestamp_writes: None });
            for it in done..(done + 16).min(iterations) {
                let (ex, ga) = &groups[it % 2];
                pass.set_pipeline(&excess);
                pass.set_bind_group(0, ex, &[]);
                pass.dispatch_workgroups(wg, wg, 1);
                pass.set_pipeline(&gather);
                pass.set_bind_group(0, ga, &[]);
                pass.dispatch_workgroups(wg, wg, 1);
            }
        }
        g.queue().submit([enc.finish()]);
        done = (done + 16).min(iterations);
    }
    let worn = if iterations.is_multiple_of(2) { a } else { b };
    // The talus, from the input and the worn relief, into `shed`.
    let out_talus = g.grid(src.lod, src.rect, 1);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("thermal talus") });
    {
        let tg = group(&talus, &src.buffer, Some(&worn.buffer));
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("thermal talus"), timestamp_writes: None });
        pass.set_pipeline(&talus);
        pass.set_bind_group(0, &tg, &[]);
        pass.dispatch_workgroups(wg, wg, 1);
    }
    enc.copy_buffer_to_buffer(&shed, 0, &out_talus.buffer, 0, bytes);
    g.queue().submit([enc.finish()]);
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("erosion.thermal", e);
        return None;
    }
    Some(vec![worn, out_talus])
}
