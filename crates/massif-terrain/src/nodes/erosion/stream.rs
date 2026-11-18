//! Stream-power erosion (`erosion.stream`): rivers cut their beds by
//! `K·A^m·S^n` (A the area draining through a place, S its slope), their
//! walls fall back to the talus angle, and what they carry settles where
//! the water slows: the dendritic valleys and sharp ridges of eroded
//! mountains, with fans at their feet and deltas at the sea.
//!
//! The simulation runs coarse to fine: the drainage of the whole terrain
//! is organised on a coarse grid in few steps, then refined level by
//! level, each level taking the change the coarser made. At each level,
//! every few steps the water's way is found again ([`super::flow`]); each
//! step then lowers every sample towards its receiver, implicitly (a bed
//! never digs below where it drains to, however long the step), gathers
//! the sediment its donors sent the step before, leaves part of it, and
//! sends the rest on; loose rock steeper than the talus angle then slides
//! (the thermal passes). The graphics card runs the same steps.

use super::common::{Kit, NB, NONE, Resample, coarser, down_avg, rand01, up_add};
use super::flow::{self, FlowGpu, accumulate, outlet, rain_scale, receivers};
use super::thermal_pass;
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use rayon::prelude::*;

/// Levels go no coarser than this many samples a side.
const COARSEST: usize = 256;

/// Steps between two findings of the water's way.
const ROUTE_EVERY: usize = 4;

/// Steps between two fills made from scratch. Between them, the fill
/// starts from the last one raised by the most the relief rose since
/// (still above the fill, and still draining everywhere: water never
/// stops in a hollow), and is lowered [`WARM_STEPS`] steps.
const FILL_EVERY: usize = 32;
const WARM_STEPS: usize = 16;

/// The settings, read once.
#[derive(Clone, Copy)]
pub(super) struct P {
    /// K·T over the whole run, in blocks^(1−2m).
    pub kt: f32,
    pub m: f32,
    pub n_exp: f32,
    /// Draining area under which nothing is cut, blocks².
    pub channel: f32,
    pub deposition: f32,
    /// The share of the relief rebuilt by uplift (0–1).
    pub uplift: f32,
    /// Blocks the land rises over the run where the uplift is 1.
    pub lift_blocks: f32,
    /// The input's lowest and highest heights.
    pub lo: f32,
    /// The share of what the rivers cut that they carry as sediment: what
    /// the uplift added is carried away as fine silt, not laid down.
    pub sed_share: f32,
    pub diffusion: f32,
    /// tan of the talus angle.
    pub talus: f32,
    pub iterations: usize,
    pub seed: u32,
    pub sea: f32,
}

impl P {
    pub fn of(ctx: &Ctx, input: &[f32]) -> Self {
        let (lo, hi) = range(input);
        let m = ctx.params.f("m");
        let size = ctx.space.size as f32;
        Self {
            // Strength 1: the largest river's knickpoint travels a quarter of
            // the terrain over the run (K·T·A^m, A the whole terrain).
            kt: ctx.params.f("strength") * 50.0 * size.powf(1.0 - 2.0 * m),
            m,
            n_exp: ctx.params.f("n"),
            channel: ctx.params.f("channel").powi(2),
            deposition: ctx.params.f("deposition"),
            uplift: ctx.params.f("rebuild"),
            // Four times the relief, which the rivers then cut back: as
            // much again as they would cut without it, so that the
            // mountains rise and are carved at once.
            lift_blocks: ctx.params.f("rebuild") * 4.0 * (hi - lo),
            lo,
            sed_share: 1.0 / (1.0 + 4.0 * ctx.params.f("rebuild")),
            diffusion: ctx.params.f("diffusion"),
            talus: ctx.params.f("talus").to_radians().tan(),
            iterations: ctx.params.int("iterations").max(1) as usize,
            seed: ctx.seed,
            sea: ctx.space.sea_level,
        }
    }
}

/// The levels: samples a side of each, finest first.
fn levels(n: usize) -> Vec<usize> {
    let mut v = vec![n];
    while *v.last().unwrap() > COARSEST {
        let m = coarser(*v.last().unwrap());
        v.push(m);
    }
    v
}

/// The seed of the water's way at step `it` of level `l`: a new one each
/// time, so that the jitter of the ways averages out.
fn route_seed(seed: u32, l: usize, it: usize) -> u32 {
    seed ^ (it as u32).wrapping_mul(0x9e37_79b9) ^ ((l as u32) << 24)
}

/// The rain at each sample, varied by the seed by up to a tenth so that
/// two seeds drain differently.
fn rain_of(rain: Option<&[f32]>, n: usize, seed: u32) -> Vec<f32> {
    (0..n * n).into_par_iter().map(|k| rain.map_or(1.0, |r| r[k].clamp(0.0, 1.0)) * (0.9 + 0.2 * rand01((k % n) as u32, (k / n) as u32, seed))).collect()
}

/// The fraction of the sediment passing a sample (`sp` blocks long,
/// draining `area` blocks²) that settles there: more where the water is
/// slow (gentle, small streams), per block of its way.
#[inline]
fn settles(p: &P, slope: f32, area: f32, sp: f32) -> f32 {
    (p.deposition * sp / 32.0 / ((1.0 + 8.0 * slope) * (1.0 + area.sqrt() / 64.0))).clamp(0.0, 1.0)
}

/// One step at a level: `h` into `out`, `qs` (sediment sent) into `qs_out`.
#[allow(clippy::too_many_arguments)]
fn step(p: &P, h: &[f32], out: &mut [f32], qs: &[f32], qs_out: &mut [f32], worn: &mut [f32], laid: &mut [f32], rcv: &[u32], acc: &[u32], hard: Option<&[f32]>, lift: Option<&[f32]>, n: usize, sp: f32, dt: f32) {
    let scale = rain_scale(n);
    let ac = p.channel.powf(p.m);
    let diff = (p.diffusion * 0.2).min(0.2);
    out.par_chunks_mut(n).zip(qs_out.par_chunks_mut(n)).zip(worn.par_chunks_mut(n).zip(laid.par_chunks_mut(n))).enumerate().for_each(|(z, ((row, qrow), (wrow, lrow)))| {
        for x in 0..n {
            let k = z * n + x;
            let hk = h[k];
            let r = rcv[k];
            let mut qin = 0.0f32;
            for (dx, dz) in NB {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx >= 0 && nz >= 0 && nx < n as i32 && nz < n as i32 {
                    let j = nz as usize * n + nx as usize;
                    if rcv[j] == k as u32 {
                        qin += qs[j];
                    }
                }
            }
            let mut v = hk;
            let mut slope = 0.0f32;
            if r != NONE {
                let (rx, rz) = (r as usize % n, r as usize / n);
                let dist = if rx != x && rz != z { std::f32::consts::SQRT_2 * sp } else { sp };
                let hr = h[r as usize];
                if hk > hr {
                    slope = (hk - hr) / dist;
                    let a = acc[k] as f32 / scale * sp * sp;
                    let soft = 1.0 - 0.95 * hard.map_or(0.0, |m| m[k].clamp(0.0, 1.0));
                    let c = dt * p.kt * soft * (a.powf(p.m) - ac).max(0.0) * slope.powf(p.n_exp - 1.0) / dist;
                    v = (hk + c * hr) / (1.0 + c);
                }
            }
            let e = hk - v;
            let total = qin + e * p.sed_share;
            let f = if r == NONE {
                if !outlet(h, x, z, n, p.sea) {
                    1.0
                } else if hk < p.sea {
                    0.5
                } else {
                    0.0
                }
            } else if hk < p.sea {
                0.5
            } else {
                settles(p, slope, acc[k] as f32 / scale * sp * sp, sp)
            };
            let dep = total * f;
            v += dep;
            qrow[x] = total - dep;
            wrow[x] += e;
            lrow[x] += dep;
            // Hillslope creep: a little of the difference to the four
            // neighbours' mean.
            let at = |dx: i32, dz: i32| {
                let (nx, nz) = ((x as i32 + dx).clamp(0, n as i32 - 1), (z as i32 + dz).clamp(0, n as i32 - 1));
                h[nz as usize * n + nx as usize]
            };
            v += diff * (at(-1, 0) + at(1, 0) + at(0, -1) + at(0, 1) - 4.0 * hk);
            v += p.lift_blocks * dt * lift.map_or(1.0, |m| m[k].clamp(0.0, 1.0));
            row[x] = v;
        }
    });
}

/// The lowest and highest of `h`.
pub(super) fn range(h: &[f32]) -> (f32, f32) {
    h.par_iter().fold(|| (f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v))).reduce(|| (f32::MAX, f32::MIN), |x, y| (x.0.min(y.0), x.1.max(y.1)))
}

/// Mean height above `lo`.
fn mean_above(h: &[f32], lo: f32) -> f32 {
    (h.par_iter().map(|&v| (v - lo).max(0.0) as f64).sum::<f64>() / h.len().max(1) as f64) as f32
}

/// What the uplift adds is taken back in the end: the relief is brought
/// back to the input's mean height above its lowest point (`mean` the
/// input's, `now` the run's), so that as much land stands as before.
fn rescale_factor(p: &P, mean: f32, now: f32) -> f32 {
    if p.uplift > 0.0 && now > 1e-3 { mean / now } else { 1.0 }
}

/// The relief from its lowest, or the sea (0), to its highest (1): where
/// the land rises when no uplift is given, as much as it stands; the sea
/// floor stays.
pub(super) fn relative(h: &[f32], sea: f32) -> Vec<f32> {
    let (lo, hi) = range(h);
    let lo = lo.max(sea.min(hi - 1.0));
    let r = (hi - lo).max(1e-6);
    h.par_iter().map(|v| ((v - lo) / r).max(0.0)).collect()
}

/// What the run gives: the relief, the drainage (whole numbers), the
/// sediment sent by each sample at the last step.
pub(super) struct Run {
    pub h: Vec<f32>,
    pub acc: Vec<u32>,
    pub qs: Vec<f32>,
    /// Blocks the rivers cut, and the sediment laid, at each sample.
    pub worn: Vec<f32>,
    pub laid: Vec<f32>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_cpu(p: &P, h0: &[f32], n: usize, sp0: f32, hard: Option<&[f32]>, rain: Option<&[f32]>, lift: Option<&[f32]>, cancel: &(dyn Fn() -> bool + Sync)) -> Option<Run> {
    let sizes = levels(n);
    // The inputs at every level, finest first.
    let mut hin = vec![h0.to_vec()];
    let mut hards = vec![hard.map(<[f32]>::to_vec)];
    let mut rains = vec![rain.map(<[f32]>::to_vec)];
    let mut lifts = vec![lift.map(<[f32]>::to_vec)];
    for l in 1..sizes.len() {
        let m = sizes[l - 1];
        hin.push(down_avg(&hin[l - 1], m));
        hards.push(hards[l - 1].as_ref().map(|v| down_avg(v, m)));
        rains.push(rains[l - 1].as_ref().map(|v| down_avg(v, m)));
        lifts.push(lifts[l - 1].as_ref().map(|v| down_avg(v, m)));
    }
    let dt = 1.0 / (p.iterations * sizes.len()) as f32;
    let mut h: Vec<f32> = Vec::new();
    let mut last = Run { h: Vec::new(), acc: Vec::new(), qs: Vec::new(), worn: Vec::new(), laid: Vec::new() };
    let (mut worn, mut laid): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    for l in (0..sizes.len()).rev() {
        let n = sizes[l];
        let sp = sp0 * (1u32 << l) as f32;
        if l + 1 == sizes.len() {
            h = hin[l].clone();
            (worn, laid) = (vec![0.0; n * n], vec![0.0; n * n]);
        } else {
            let change: Vec<f32> = h.par_iter().zip(&hin[l + 1]).map(|(a, b)| a - b).collect();
            h = up_add(&change, Some(&hin[l]), n);
            (worn, laid) = (up_add(&worn, None, n), up_add(&laid, None, n));
        }
        let rain = rain_of(rains[l].as_deref(), n, p.seed ^ l as u32);
        let eps = 1e-3 * sp;
        let drop = [p.talus * sp, p.talus * sp * std::f32::consts::SQRT_2];
        let (mut next, mut qs, mut qs_next) = (vec![0.0; n * n], vec![0.0; n * n], vec![0.0; n * n]);
        let (mut shed, mut total) = (vec![0.0; n * n], vec![0.0; n * n]);
        let (mut rcv, mut acc) = (Vec::new(), Vec::new());
        let (mut w, mut at_fill): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        for it in 0..p.iterations {
            if cancel() {
                return None;
            }
            if it % ROUTE_EVERY == 0 {
                w = if it % FILL_EVERY == 0 {
                    flow::fill(&h, n, p.sea, eps, p.seed, cancel)?
                } else {
                    let rise = h.par_iter().zip(&at_fill).map(|(a, b)| (a - b).max(0.0)).reduce(|| 0.0, f32::max);
                    let init = (0..n * n).into_par_iter().map(|k| if outlet(&h, k % n, k / n, n, p.sea) { h[k] } else { h[k].max(w[k] + rise) }).collect();
                    flow::fill_from(&h, init, n, p.sea, eps, p.seed, WARM_STEPS, cancel)?
                };
                at_fill.clone_from(&h);
                rcv = receivers(&h, &w, n, p.sea, route_seed(p.seed, l, it));
                acc = accumulate(&rcv, Some(&rain), n);
            }
            step(p, &h, &mut next, &qs, &mut qs_next, &mut worn, &mut laid, &rcv, &acc, hards[l].as_deref(), lifts[l].as_deref(), n, sp, dt);
            std::mem::swap(&mut qs, &mut qs_next);
            thermal_pass(&next, &mut h, &mut shed, &mut total, n, drop, 0.5);
        }
        last = Run { h: Vec::new(), acc, qs, worn: Vec::new(), laid: Vec::new() };
    }
    let f = rescale_factor(p, mean_above(h0, p.lo), mean_above(&h, p.lo));
    if f != 1.0 {
        h.par_iter_mut().for_each(|v| *v = p.lo + (*v - p.lo) * f);
        worn.par_iter_mut().chain(laid.par_iter_mut()).for_each(|v| *v *= f);
    }
    last.h = h;
    (last.worn, last.laid) = (worn, laid);
    Some(last)
}

/// The node's outputs from the run: height, flow, wear, deposition,
/// sediment.
pub(super) fn outputs(run: Run, n: usize) -> Vec<Vec<f32>> {
    let scale = rain_scale(n);
    let flow: Vec<f32> = run.acc.par_iter().map(|&a| flow::flow_mask(a as f32 / scale, n, 1.5)).collect();
    let (wr, dr) = refs(&run.worn, &run.laid);
    let wear: Vec<f32> = run.worn.par_iter().map(|&d| soft_mask(d, wr)).collect();
    let dep: Vec<f32> = run.laid.par_iter().map(|&d| soft_mask(d, dr)).collect();
    let sed: Vec<f32> = run.qs.par_iter().map(|&q| sediment_of(q)).collect();
    vec![run.h, flow, wear, dep, sed]
}

/// The amounts at which the wear and deposition masks reach a half: two
/// blocks, or the mean where it is more (the rivers of a rebuilt relief
/// cut much more than it stands high, everywhere).
pub(super) fn refs(worn: &[f32], laid: &[f32]) -> (f32, f32) {
    let mean = |v: &[f32]| (v.par_iter().map(|&x| x.max(0.0) as f64).sum::<f64>() / v.len().max(1) as f64) as f32;
    (mean(worn).max(2.0), mean(laid).max(1.0))
}

/// An amount as a mask: a half at `r`, nearing 1 above.
#[inline]
pub(super) fn soft_mask(d: f32, r: f32) -> f32 {
    let d = d.max(0.0);
    d / (d + r)
}

/// Sediment carried (blocks over a sample) as a mask.
#[inline]
pub(super) fn sediment_of(q: f32) -> f32 {
    ((1.0 + q.max(0.0)).ln() / 1000f32.ln()).clamp(0.0, 1.0)
}

pub(super) fn stream_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("stream erosion has its input");
    let n = src.rect.w as usize;
    let p = P::of(ctx, src.plane(0));
    let plane = |i: usize| inputs.get(i).copied().flatten().map(|g| g.plane(0));
    let own;
    let lift = match plane(3) {
        Some(l) => Some(l),
        None => {
            own = relative(src.plane(0), ctx.space.sea_level);
            Some(&own[..])
        }
    };
    let run = run_cpu(&p, src.plane(0), n, ctx.spacing() as f32, plane(1), plane(2), lift, cancel)?;
    Some(outputs(run, n))
}

/// The uniform block of the kernels other than `step`: the side, a
/// factor (in `uplift`) and a height (in `sea`).
fn plain_uniform(n: usize, factor: f32, height: f32) -> Uniforms {
    let mut u = Uniforms::default();
    u.u32(n as u32).u32(0);
    for _ in 0..7 {
        u.f32(0.0);
    }
    u.f32(factor).f32(0.0).f32(height).u32(0).u32(0).u32(0).f32(rain_scale(n)).u32(0);
    u
}

const STREAM_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("stream.wgsl"));
const THERMAL_WGSL: &str = include_str!("thermal.wgsl");

pub(super) fn stream_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n0 = src.side() as usize;
    let input = g.download_grid(src);
    let p = P::of(ctx, input.plane(0));
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let fl = FlowGpu::new(&k)?;
    let rs = Resample::new(&k)?;
    let pipe = |e| g.compute("erosion.stream", STREAM_WGSL, e);
    let (stepp, rainp, outp, dp) = (pipe("step")?, pipe("rain")?, pipe("outputs")?, pipe("difference")?);
    let (rescalep, warmp, risep) = (pipe("rescale")?, pipe("warm")?, pipe("rise")?);
    let tp = |e| g.compute("erosion.thermal", THERMAL_WGSL, e);
    let (excess, gather) = (tp("excess")?, tp("gather")?);
    let sizes = levels(n0);
    let opt = |i: usize| inputs.get(i).copied().flatten().map(|g| &g.buffer);
    let stand_in = k.buf("erosion stand-in", 4);
    // Inputs at every level, finest first (`None`: not connected).
    let mut hin: Vec<wgpu::Buffer> = Vec::new();
    let mut masks: Vec<[Option<wgpu::Buffer>; 3]> = Vec::new();
    {
        let first = k.buf("stream input", n0 * n0);
        k.copy(&src.buffer, &first, (n0 * n0 * 4) as u64);
        hin.push(first);
        let copy_of = |k: &mut Kit, b: Option<&wgpu::Buffer>| {
            b.map(|b| {
                let c = k.buf("stream mask", n0 * n0);
                k.copy(b, &c, (n0 * n0 * 4) as u64);
                c
            })
        };
        let lift = copy_of(&mut k, opt(3)).or_else(|| {
            // The relief itself, from 0 to 1, as the CPU makes it.
            let rel = relative(input.plane(0), ctx.space.sea_level);
            let b = k.buf("stream uplift", n0 * n0);
            g.queue().write_buffer(&b, 0, bytemuck::cast_slice(&rel));
            Some(b)
        });
        masks.push([copy_of(&mut k, opt(1)), copy_of(&mut k, opt(2)), lift]);
    }
    for l in 1..sizes.len() {
        let m = sizes[l - 1];
        let h = rs.down(&mut k, &hin[l - 1], m, false);
        let ms = [0, 1, 2].map(|i| masks[l - 1][i].as_ref().map(|b| rs.down(&mut k, b, m, false)));
        hin.push(h);
        masks.push(ms);
    }
    let dt = 1.0 / (p.iterations * sizes.len()) as f32;
    let trace = std::env::var("MASSIF_EROSION_TRACE").is_ok();
    let mut times = [0.0f64; 2];
    let started = std::time::Instant::now();
    let mut h: Option<wgpu::Buffer> = None;
    let mut last_acc = None;
    let mut last_qs = None;
    let mut acc_prev: Option<(wgpu::Buffer, wgpu::Buffer)> = None;
    for l in (0..sizes.len()).rev() {
        let n = sizes[l];
        let sp = (ctx.spacing() as f32) * (1u32 << l) as f32;
        let ha = k.buf("stream height", n * n);
        let (worn, laid) = (k.buf("stream worn", n * n), k.buf("stream laid", n * n));
        if let Some((pw, pl)) = acc_prev.take() {
            rs.up(&mut k, &pw, None, n, &worn);
            rs.up(&mut k, &pl, None, n, &laid);
        }
        match &h {
            None => k.copy(&hin[l], &ha, (n * n * 4) as u64),
            Some(prev) => {
                // The coarser level's change, brought to this one.
                let m = sizes[l + 1];
                let change = k.buf("stream change", m * m);
                let u = k.uniform(&mut plain_uniform(m, 0.0, 0.0));
                let bg = k.bind(&dp, &[(0, &u), (1, prev), (2, &hin[l + 1]), (11, &change)]);
                k.run(&dp, &bg, m);
                rs.up(&mut k, &change, Some(&hin[l]), n, &ha);
            }
        }
        let [hard, rain_in, lift] = &masks[l];
        let u = k.uniform(
            Uniforms::default()
                .u32(n as u32)
                .u32(p.seed ^ l as u32)
                .f32(sp)
                .f32(dt)
                .f32(p.kt)
                .f32(p.m)
                .f32(p.n_exp)
                .f32(p.channel.powf(p.m))
                .f32(p.deposition)
                .f32(p.lift_blocks)
                .f32((p.diffusion * 0.2).min(0.2))
                .f32(p.sea)
                .u32(hard.is_some() as u32)
                .u32(rain_in.is_some() as u32)
                .u32(lift.is_some() as u32)
                .f32(rain_scale(n))
                .f32(p.sed_share),
        );
        // The rain, varied by the seed.
        let rain = k.buf("stream rain", n * n);
        let bg = k.bind(&rainp, &[(0, &u), (8, rain_in.as_ref().unwrap_or(&stand_in)), (13, &rain)]);
        k.run(&rainp, &bg, n);
        let hb = k.buf("stream height", n * n);
        let (qa, qb) = (k.buf("stream sediment", n * n), k.buf("stream sediment", n * n));
        let (rcv, acc) = (k.buf("stream receivers", n * n), k.buf("stream drainage", n * n));
        let (shed, total) = (k.buf("thermal shed", n * n), k.buf("thermal total", n * n));
        let tu = g.uniform("thermal", &Uniforms::default().u32(n as u32).f32(0.5).f32(p.talus * sp).f32(p.talus * sp * std::f32::consts::SQRT_2).finish());
        let mk = |k: &Kit, qi: &wgpu::Buffer, qo: &wgpu::Buffer| {
            k.bind(
                &stepp,
                &[(0, &u), (1, &ha), (2, &hb), (3, qi), (4, qo), (5, &rcv), (6, &acc), (7, hard.as_ref().unwrap_or(&stand_in)), (9, lift.as_ref().unwrap_or(&stand_in)), (16, &worn), (17, &laid)],
            )
        };
        let step_groups = [mk(&k, &qa, &qb), mk(&k, &qb, &qa)];
        let ex = k.bind(&excess, &[(0, &tu), (1, &hb), (2, &shed), (3, &total)]);
        let ga = k.bind(&gather, &[(0, &tu), (1, &hb), (2, &shed), (3, &total), (4, &ha)]);
        let eps = 1e-3 * sp;
        let mut w: Option<wgpu::Buffer> = None;
        let at_fill = k.buf("stream relief at fill", n * n);
        let rise = k.buf("stream rise", 4);
        for it in 0..p.iterations {
            if it % ROUTE_EVERY == 0 {
                if cancel() {
                    return None;
                }
                let t0 = trace.then(|| {
                    k.finish();
                    std::time::Instant::now()
                });
                if it % FILL_EVERY == 0 {
                    if let Some(old) = w.take() {
                        k.put(old);
                    }
                    w = Some(fl.fill(&mut k, &ha, n, p.sea, eps, p.seed, cancel)?);
                } else {
                    let wb = w.as_ref().expect("filled at the level's first step");
                    k.clear(&rise);
                    let bg = k.bind(&risep, &[(0, &u), (1, &ha), (2, &at_fill), (15, &rise)]);
                    k.run(&risep, &bg, n);
                    let bg = k.bind(&warmp, &[(0, &u), (1, &ha), (10, wb), (15, &rise)]);
                    k.run(&warmp, &bg, n);
                    if !fl.fill_from(&mut k, &ha, wb, n, p.sea, eps, p.seed, WARM_STEPS, cancel) {
                        return None;
                    }
                }
                let wb = w.as_ref().expect("filled");
                k.copy(&ha, &at_fill, (n * n * 4) as u64);
                let t1 = t0.map(|t| {
                    k.finish();
                    times[0] += t.elapsed().as_secs_f64();
                    std::time::Instant::now()
                });
                fl.receivers(&mut k, &ha, wb, n, p.sea, route_seed(p.seed, l, it), &rcv);
                fl.accumulate(&mut k, &rcv, Some(&rain), n, &acc);
                if let Some(t) = t1 {
                    k.finish();
                    times[1] += t.elapsed().as_secs_f64();
                }
            }
            k.run(&stepp, &step_groups[it % 2], n);
            k.run(&excess, &ex, n);
            k.run(&gather, &ga, n);
        }
        h = Some(ha);
        last_acc = Some(acc);
        last_qs = Some(if p.iterations.is_multiple_of(2) { qa } else { qb });
        acc_prev = Some((worn, laid));
    }
    if trace {
        k.finish();
        eprintln!("stream: {:.0} ms, fills {:.0} ms, ways and sums {:.0} ms", started.elapsed().as_secs_f64() * 1e3, times[0] * 1e3, times[1] * 1e3);
    }
    let (h, acc, qs, (worn, laid)) = (h?, last_acc?, last_qs?, acc_prev?);
    let n = n0;
    if p.uplift > 0.0 {
        // The relief brought back, its mean measured on the CPU (the
        // same sum as there).
        k.flush();
        let now = g.download_buffers(&[&h]).pop().unwrap_or_default();
        let f = rescale_factor(&p, mean_above(input.plane(0), p.lo), mean_above(&now[..n * n], p.lo));
        let u = k.uniform(&mut plain_uniform(n, f, p.lo));
        let hb = k.buf("stream height", n * n);
        let bg = k.bind(&rescalep, &[(0, &u), (1, &h), (2, &hb)]);
        k.run(&rescalep, &bg, n);
        k.copy(&hb, &h, (n * n * 4) as u64);
        let u = k.uniform(&mut plain_uniform(n, f, 0.0));
        for b in [&worn, &laid] {
            let bg = k.bind(&rescalep, &[(0, &u), (1, b), (2, &hb)]);
            k.run(&rescalep, &bg, n);
            k.copy(&hb, b, (n * n * 4) as u64);
        }
    }
    // The masks, from the input and the run.
    let outs: Vec<GpuGrid> = (0..4).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    k.flush();
    let got = g.download_buffers(&[&worn, &laid]);
    let (wr, dr) = refs(&got[0][..n * n], &got[1][..n * n]);
    let mut uo = plain_uniform(n, 0.0, 0.0);
    // The references in `dt` and `kt`.
    uo.bytes[12..16].copy_from_slice(&wr.to_le_bytes());
    uo.bytes[16..20].copy_from_slice(&dr.to_le_bytes());
    let u = k.uniform(&mut uo);
    let bg = k.bind(&outp, &[(0, &u), (3, &qs), (6, &acc), (7, &worn), (9, &laid), (11, &outs[0].buffer), (12, &outs[1].buffer), (13, &outs[2].buffer), (14, &outs[3].buffer)]);
    k.run(&outp, &bg, n);
    let height = g.grid(src.lod, src.rect, 1);
    k.copy(&h, &height.buffer, (n * n * 4) as u64);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("erosion.stream", e);
        return None;
    }
    let mut v = vec![height];
    v.extend(outs);
    Some(v)
}
