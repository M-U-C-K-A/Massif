//! Erosion: what water, frost, ice and wind do to a relief, over the whole
//! terrain. Owned by the erosion session (M3): its nodes are simulations
//! (`Kernel::Global`), each with a CPU kernel and, where it can, a GPU one
//! ([`crate::kernel::GlobalGpuFn`]); its WGSL lives in this folder.

mod common;
mod flow;
mod glacier;
mod particle;
mod stream;
mod water;
mod wind;

use crate::nodes::height_out;
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use rayon::prelude::*;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&STREAM, &PARTICLE, &THERMAL, &DEPOSIT, &GLACIER, &WIND, &FLOW, &RIVERS, &LAKES, &SEA];

/// The masks every erosion node gives beside its height, named as the Map
/// Output's inputs are.
const FLOW_OUT: PortSpec = PortSpec::new("flow", "Flow", PortKind::Mask, "Where water runs: 1 in the largest rivers, fading up the smallest rills.");
const WEAR_OUT: PortSpec = PortSpec::new("wear", "Wear", PortKind::Mask, "Where the water wore the ground away, the most where it cut the most: river beds, gullies.");
const DEPOSITION_OUT: PortSpec = PortSpec::new("deposition", "Deposition", PortKind::Mask, "Where what was worn away came to rest: fans, valley floors, deltas.");
const SEDIMENT_OUT: PortSpec = PortSpec::new("sediment", "Sediment", PortKind::Mask, "How much sediment the water carries at the end: muddy rivers, gravel beds.");

const HARDNESS_IN: PortSpec = PortSpec::new("hardness", "Hardness", PortKind::Mask, "Rock that resists: 1 wears twenty times slower than 0.").or("soft rock everywhere");
const RAIN_IN: PortSpec = PortSpec::new("rainfall", "Rainfall", PortKind::Mask, "How much rain falls where: a rainfall map, a rain shadow.").or("the same rain everywhere");

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

pub static DEPOSIT: NodeSpec = NodeSpec {
    id: "sediment.deposit",
    label: "Sediment",
    family: Family::Simulate,
    summary: "Loose rock and soil shed by steep slopes run down with the water and settle where it slows: fans at the feet of mountains, filled valleys and basins, deltas at the sea.",
    help: "Lays down what the mountains shed, without cutting anything: the steep slopes of Source give Amount blocks of material over the run, the water carries it down its ways and drops it where they flatten, spreading alluvial fans at the mouths of valleys, flat floors in the valleys and basins, and deltas where rivers meet the sea. Use it after Stream erosion, or on any range, to soften its feet into the plains.",
    keywords: &["sediment", "alluvial fan", "deposition", "valley fill", "delta", "basin", "bajada"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief that sheds and gathers."), PortSpec::new("source", "Source", PortKind::Mask, "Where loose material comes from (still only from its steep slopes).").or("every steep slope")],
    outputs: &[height_out("The relief with its sediment laid down."), DEPOSITION_OUT, PortSpec::new("wear", "Source", PortKind::Mask, "Where the material came from."), SEDIMENT_OUT],
    params: &[
        ParamSpec::float("amount", "Amount", Unit::Blocks, 6.0, 0.0, 200.0, "Blocks of material the steepest slopes shed over the run.").soft(0.0, 40.0),
        ParamSpec::float("deposition", "Deposition", Unit::Ratio, 0.7, 0.0, 1.0, "How readily the water drops what it carries: higher, shorter and steeper fans; lower, material carried far down the valleys and into the sea."),
        ParamSpec::int("iterations", "Iterations", 150, 10, 2000, "Steps at each level of the simulation: more, fans spreading further."),
        ParamSpec::float("creep", "Creep", Unit::Ratio, 0.05, 0.0, 1.0, "How much the loose ground creeps and smooths.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: stream::deposit_cpu, gpu: Some(stream::deposit_gpu), carry: &[Some(0), None, None, None] },
    ..NodeSpec::BASE
};

pub static GLACIER: NodeSpec = NodeSpec {
    id: "ice.glacier",
    label: "Glaciers",
    family: Family::Simulate,
    summary: "Ice gathers above the snow line, flows down the valleys and scours them into U-shapes, with cirques at their heads, fjords below the sea and moraines at their ends.",
    help: "Snow above the equilibrium line (ELA) turns to ice, which flows downhill and melts below the line. Where it slides it wears its bed: V-shaped river valleys become broad U-shaped troughs, their heads armchair cirques, side valleys left hanging, and valleys the ice carried below the sea become fjords. Moraines are laid along its edges and where it ends. Lower the line or raise Accumulation for larger glaciers; Strength is how deep they cut.",
    keywords: &["glacier", "ice", "U-valley", "cirque", "fjord", "moraine", "ice age", "glacial"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the ice covers."), PortSpec::new("snow", "Snow", PortKind::Mask, "Where snow falls (a snow or rainfall map): 0 no ice grows.").or("the same snow everywhere")],
    outputs: &[height_out("The relief scoured by the ice, its moraines laid."), PortSpec::new("ice", "Ice", PortKind::Mask, "Where the ice lies at the end, by how thick (1 for 30 blocks and more)."), WEAR_OUT, PortSpec::new("deposition", "Moraines", PortKind::Mask, "Where the ice laid its moraines, along its edges and where it ends.")],
    params: &[
        ParamSpec::float("ela", "Snow line", Unit::Blocks, 170.0, -64.0, 320.0, "The equilibrium line: above it snow gathers into ice, below it the ice melts.").soft(60.0, 300.0),
        ParamSpec::float("accumulation", "Accumulation", Unit::Ratio, 1.0, 0.0, 10.0, "How fast ice gathers above the line, and melts below: higher, thicker glaciers reaching further down.").soft(0.0, 4.0),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 10.0, "How deep the ice scours its bed.").soft(0.0, 4.0),
        ParamSpec::int("iterations", "Iterations", 800, 10, 10000, "Steps of the simulation: more, an older ice age, deeper troughs."),
        ParamSpec::float("moraines", "Moraines", Unit::Blocks, 3.0, 0.0, 32.0, "How high the ridges of rubble along the ice's edges stand.").section(Section::Detail),
        ParamSpec::float("flow", "Flow", Unit::Ratio, 1.0, 0.0, 10.0, "How readily the ice flows and slides: higher, thinner and faster glaciers.").section(Section::Advanced),
    ],
    kernel: Kernel::Global { cpu: glacier::glacier_cpu, gpu: Some(glacier::glacier_gpu), carry: &[Some(0), None, None, None] },
    ..NodeSpec::BASE
};

pub static WIND: NodeSpec = NodeSpec {
    id: "wind.erosion",
    label: "Wind",
    family: Family::Simulate,
    summary: "The wind blows loose sand into dunes, ridged where sand is plenty and crescents where it is scarce, and wears bare rock that faces it.",
    help: "Spreads Sand blocks of loose sand (where the Sand mask says) and lets the wind work it: it lifts sand in the open, carries it a hop downwind, and drops it in the lee of every rise, so that dunes grow with gentle backs to the wind and steep slip faces away from it. Plenty of sand makes transverse ridges; little makes barchans. Abrasion wears rock facing the wind into streamlined ridges (yardangs). Connect a wind vector (Climate's wind) to make the wind turn over the land.",
    keywords: &["wind", "dunes", "sand", "desert", "barchan", "aeolian", "yardang", "erg"],
    inputs: &[
        PortSpec::new("height", "Height", PortKind::Height, "The ground under the sand."),
        PortSpec::new("sand", "Sand", PortKind::Mask, "Where loose sand lies at first.").or("sand everywhere"),
        PortSpec::new("wind", "Wind", PortKind::Vector, "The wind's way at each place.").or("the Direction setting everywhere"),
    ],
    outputs: &[height_out("The ground with its dunes."), PortSpec::new("sand", "Sand", PortKind::Mask, "How deep the sand lies (1 for 8 blocks and more)."), WEAR_OUT],
    params: &[
        ParamSpec::float("direction", "Direction", Unit::Degrees, 0.0, 0.0, 360.0, "Where the wind blows towards: 0° east, 90° south."),
        ParamSpec::float("sand_depth", "Sand", Unit::Blocks, 6.0, 0.0, 64.0, "Blocks of loose sand to blow around: plenty, transverse ridges; little, crescents.").soft(0.0, 24.0),
        ParamSpec::float("hop", "Hop", Unit::Blocks, 12.0, 1.0, 128.0, "How far a grain flies in one hop: longer, larger dunes further apart.").soft(4.0, 48.0),
        ParamSpec::int("iterations", "Iterations", 400, 10, 10000, "Steps of the simulation: more, dunes grown larger and more regular."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 5.0, "How much sand the wind lifts at each step.").section(Section::Detail),
        ParamSpec::float("abrasion", "Abrasion", Unit::Ratio, 0.0, 0.0, 5.0, "How fast blown sand wears the bare rock that faces the wind.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: wind::wind_cpu, gpu: Some(wind::wind_gpu), carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

pub static PARTICLE: NodeSpec = NodeSpec {
    id: "erosion.particle",
    label: "Particle erosion",
    family: Family::Simulate,
    summary: "Raindrops run down the slopes, wearing gullies and rills where they speed up and laying soil where they slow down.",
    help: "The fine detail of erosion: each drop of rain runs downhill, picks up soil where it gathers speed and drops it where it slows, in hollows and at the feet of slopes. Use it after Stream erosion, or on its own on hills and cliffs, for gullies, rills, scree fans and soft valley floors. More Rain cuts deeper; Capacity is how much a drop carries; Radius how wide it wears.",
    keywords: &["hydraulic", "droplets", "rain", "gullies", "rills", "erosion", "sediment"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to erode."), HARDNESS_IN, RAIN_IN],
    outputs: &[height_out("The relief eroded."), FLOW_OUT, WEAR_OUT, DEPOSITION_OUT],
    params: &[
        ParamSpec::float("rain", "Rain", Unit::Ratio, 1.0, 0.05, 16.0, "Raindrops for each sample of the simulation: more, deeper gullies, slower.").soft(0.1, 4.0),
        ParamSpec::float("erosion", "Erosion", Unit::Ratio, 0.3, 0.0, 1.0, "How fast a drop picks up soil where it can carry more."),
        ParamSpec::float("deposition", "Deposition", Unit::Ratio, 0.3, 0.0, 1.0, "How fast a drop lays down what it carries beyond what it can."),
        ParamSpec::float("capacity", "Capacity", Unit::Ratio, 4.0, 0.0, 32.0, "How much soil a fast drop on a steep slope can carry.").soft(0.5, 16.0),
        ParamSpec::float("evaporation", "Evaporation", Unit::Ratio, 0.02, 0.0, 0.5, "How much of its water a drop loses at each step: higher, shorter gullies.").section(Section::Detail),
        ParamSpec::float("radius", "Radius", Unit::Blocks, 6.0, 1.0, 64.0, "How wide a drop wears the ground around it.").section(Section::Detail),
        ParamSpec::float("inertia", "Inertia", Unit::Ratio, 0.05, 0.0, 0.95, "How much a drop keeps its way rather than turning downhill: higher, straighter and longer gullies.").section(Section::Advanced),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: particle::particle_cpu, gpu: Some(particle::particle_gpu), carry: &[Some(0), None, None, None] },
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
    help: "Wears cliffs back and fills the feet of slopes with talus, the way frost and gravity do: what is steeper than Talus angle sheds material to the lower neighbours until it is stable. More iterations go further towards that angle; Strength is how much moves at each; Hardness keeps cliffs of hard rock standing while soft beds crumble. The Talus output is where material came to rest: scree, gravel, rubble.",
    keywords: &["talus", "scree", "weathering", "slope", "repose"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to wear."), PortSpec::new("hardness", "Hardness", PortKind::Mask, "Rock that holds: 1 never crumbles, 0 crumbles as Strength says.").or("the same rock everywhere")],
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
#[allow(clippy::too_many_arguments)]
pub(super) fn thermal_pass(h: &[f32], next: &mut [f32], shed: &mut [f32], total: &mut [f32], n: usize, drop: [f32; 2], strength: f32, hard: Option<&[f32]>) {
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
            let s = strength * (1.0 - hard.map_or(0.0, |m| m[z * n + x].clamp(0.0, 1.0)));
            srow[x] = s * 0.5 * most;
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
        thermal_pass(&h, &mut next, &mut shed, &mut total, n, drop, strength, inputs.get(1).copied().flatten().map(|g| g.plane(0)));
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
    let hard = inputs.get(1).copied().flatten().map(|g| &g.buffer);
    let u = g.uniform("thermal", &Uniforms::default().u32(n).f32(ctx.params.f("strength")).f32(slope * spacing).f32(slope * spacing * std::f32::consts::SQRT_2).u32(hard.is_some() as u32).finish());
    let stand_in = g.buffer("thermal stand-in", 16);
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
        } else if std::ptr::eq(p, &excess) {
            // What `excess` alone reads: the hardness.
            entries.push(wgpu::BindGroupEntry { binding: 5, resource: hard.unwrap_or(&stand_in).as_entire_binding() });
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

/// The water output's words: a surface where there is water.
const WATER_OUT: PortSpec = PortSpec::new("water", "Water", PortKind::Height, "The surface of the water where there is some, in blocks; well under the ground elsewhere.");

pub static RIVERS: NodeSpec = NodeSpec {
    id: "water.rivers",
    label: "Rivers",
    family: Family::Simulate,
    summary: "Rivers where enough of the land drains, their beds and banks dug into the relief, wider and deeper downstream, holding lakes in deep hollows.",
    help: "Follows the rain to the sea or the terrain's edge and lays a river wherever enough land drains: a thread at its source, wider and deeper as water joins it. Its water only ever goes down, cutting through the rims of shallow hollows and filling deep ones with lakes. Threshold sets how many rivers there are; Width and Depth those of the largest; Banks how gently the land slopes down to them. Water already on the map (wired to Water) is kept, and rivers end where they reach it.",
    keywords: &["river", "stream", "channel", "riverbed", "drainage", "water", "lakes"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the rivers run over."), PortSpec::new("water", "Water", PortKind::Height, "Water already there (a map's rivers and lakes): kept, and rivers end in it.").or("no water yet")],
    outputs: &[height_out("The relief with the rivers' beds and banks dug in."), PortSpec::new("rivers", "Rivers", PortKind::Mask, "1 in the rivers' beds, fading out over their banks."), WATER_OUT],
    params: &[
        ParamSpec::float("threshold", "Threshold", Unit::Ratio, 0.55, 0.0, 1.0, "How much land must drain to a place for a river to run there: higher, fewer and larger rivers."),
        ParamSpec::float("width", "Width", Unit::Blocks, 14.0, 1.0, 256.0, "The width of the largest river; the smallest are a quarter of it.").soft(2.0, 64.0),
        ParamSpec::float("depth", "Depth", Unit::Blocks, 5.0, 0.5, 64.0, "The depth of the largest river.").soft(1.0, 16.0),
        ParamSpec::float("banks", "Banks", Unit::Ratio, 1.0, 0.0, 8.0, "How far the banks slope down to the water, in river widths.").soft(0.0, 4.0),
        ParamSpec::float("lakes", "Lakes", Unit::Ratio, 0.5, 0.0, 1.0, "Which hollows hold lakes rather than being cut through: 0 none, 1 every hollow a block deep."),
        ParamSpec::float("sea", "Sea level", Unit::Blocks, 62.0, -64.0, 320.0, "Rivers end where the ground goes under it.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: water::rivers_cpu, gpu: Some(water::rivers_gpu), carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

pub static LAKES: NodeSpec = NodeSpec {
    id: "water.lakes",
    label: "Lakes",
    family: Family::Simulate,
    summary: "Every hollow fills up to where it spills: with water, or with ground.",
    help: "Finds every hollow of the relief, where rain would gather, and fills it to the level where it overflows. Amount 0 makes them lakes; 1 fills them with ground, as sediment would, smoothing the land; between, both. What lies under the sea level is the sea's, not a lake.",
    keywords: &["lake", "pond", "hollow", "depression", "basin", "fill", "water"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief whose hollows fill.")],
    outputs: &[height_out("The relief, its hollows filled with ground as much as Amount says."), PortSpec::new("lakes", "Lakes", PortKind::Mask, "Where the lakes are, by how deep (1 for a block and a half and more)."), WATER_OUT],
    params: &[
        ParamSpec::float("amount", "Amount", Unit::Ratio, 0.0, 0.0, 1.0, "How much of each hollow fills with ground rather than water: 0, lakes; 1, flat ground."),
        ParamSpec::float("sea", "Sea level", Unit::Blocks, 62.0, -64.0, 320.0, "Under it the hollows are the sea's: hollows spill into it."),
    ],
    kernel: Kernel::Global { cpu: water::lakes_cpu, gpu: Some(water::lakes_gpu), carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};

pub static SEA: NodeSpec = NodeSpec {
    id: "water.sea",
    label: "Sea",
    family: Family::Simulate,
    summary: "The sea up to a level, where the terrain's edge lets it in, and the shore around it.",
    help: "Floods the relief up to Level from the terrain's edge: low ground behind higher ground stays dry unless Connected is off. Its Shore mask follows the coast, a band of height around the level, for beaches and cliffs' feet.",
    keywords: &["sea", "ocean", "coast", "shore", "beach", "water level"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the sea floods.")],
    outputs: &[height_out("The relief, as it is."), WATER_OUT, PortSpec::new("shore", "Shore", PortKind::Mask, "A band along the coast, Shore blocks of height either side of the level.")],
    params: &[
        ParamSpec::float("level", "Level", Unit::Blocks, 62.0, -64.0, 320.0, "The sea's surface."),
        ParamSpec::float("shore", "Shore", Unit::Blocks, 4.0, 0.5, 64.0, "How many blocks of height either side of the level the shore takes.").soft(0.5, 16.0),
        ParamSpec::flag("connected", "Connected", true, "Only where the sea comes in from the terrain's edge; off, every place under the level is sea."),
    ],
    kernel: Kernel::Global { cpu: water::sea_cpu, gpu: Some(water::sea_gpu), carry: &[Some(0), None, None] },
    ..NodeSpec::BASE
};
