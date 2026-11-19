//! Particle erosion (`erosion.particle`): raindrops run down the relief,
//! pick up soil where they speed up and lay it down where they slow down
//! (after Hans Beyer's method, as Massif 1.2's Erosion node): gullies,
//! rills and the small sediment fans at their feet.
//!
//! The rain falls in passes. In a pass every drop runs at once over the
//! relief as it stood when the pass began, and adds what it wears away and
//! lays down to a grid of changes in whole numbers (fixed point), applied
//! at the end of the pass: sums of whole numbers do not depend on the order
//! of the additions, so the result is the same each time on the graphics
//! card, and the CPU, doing the same arithmetic, gives the same up to the
//! rounding of a few floating-point operations. Each pass rains on a
//! sixteenth of the samples, so that drops seldom meet within a pass.

use super::common::{Kit, rand01};
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use rayon::prelude::*;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// Samples between two drops of a pass (each pass rains on one sample in
/// `STRIDE²`).
const STRIDE: usize = 4;

/// Steps a raindrop lives at most.
const MAX_STEPS: u32 = 80;

const GRAVITY: f32 = 4.0;
const MIN_CAPACITY: f32 = 0.01;

/// Fixed-point units: of the changes of height (in samples' units), of the
/// water passing, and of the soil worn and laid.
const DELTA: f32 = 65536.0;
const WATER: f32 = 1024.0;
const SOIL: f32 = 4096.0;

/// The settings, read once.
#[derive(Clone, Copy)]
struct P {
    passes: usize,
    erosion: f32,
    deposition: f32,
    capacity: f32,
    evaporation: f32,
    inertia: f32,
    /// The brush's radius in samples, and the sum of its weights.
    r: i32,
    brush_total: f32,
    seed: u32,
    sea: f32,
    /// Blocks between samples.
    sp: f32,
}

impl P {
    fn of(ctx: &Ctx) -> Self {
        let sp = ctx.spacing() as f32;
        let r = (ctx.params.f("radius") / sp).round().clamp(1.0, 12.0) as i32;
        let mut total = 0.0f32;
        for dz in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                if d <= r as f32 {
                    total += 1.0 - d / (r as f32 + 1.0);
                }
            }
        }
        Self {
            passes: ((ctx.params.f("rain") * (STRIDE * STRIDE) as f32).round() as usize).max(1),
            erosion: ctx.params.f("erosion").clamp(0.0, 1.0),
            deposition: ctx.params.f("deposition").clamp(0.0, 1.0),
            capacity: ctx.params.f("capacity").max(0.0),
            evaporation: ctx.params.f("evaporation").clamp(0.0, 0.5),
            inertia: ctx.params.f("inertia").clamp(0.0, 0.95),
            r,
            brush_total: total,
            seed: ctx.seed,
            sea: ctx.space.sea_level,
            sp,
        }
    }

    fn pass_seed(&self, pass: usize) -> u32 {
        self.seed ^ (pass as u32).wrapping_mul(0x9e37_79b9)
    }
}

/// A value in fixed point, rounded as the card rounds it.
#[inline]
fn fix(v: f32, unit: f32) -> i32 {
    (v * unit + 0.5).floor() as i32
}

/// What drops write to during a pass.
struct Writes<'a> {
    delta: &'a [AtomicI32],
    flow: &'a [AtomicU32],
    laid: &'a [AtomicU32],
    worn: &'a [AtomicU32],
}

/// The relief in samples' units at `(i, j)`.
#[inline]
fn at(h: &[f32], n: usize, sp: f32, i: usize, j: usize) -> f32 {
    h[j * n + i] / sp
}

/// Height and slope at `(x, z)`, bilinear, as `height_grad` of the shader.
#[inline]
fn height_grad(h: &[f32], n: usize, sp: f32, x: f32, z: f32) -> (f32, f32, f32) {
    let (i, j) = (x as usize, z as usize);
    let (u, v) = (x - i as f32, z - j as f32);
    let (a, b, c, d) = (at(h, n, sp, i, j), at(h, n, sp, i + 1, j), at(h, n, sp, i, j + 1), at(h, n, sp, i + 1, j + 1));
    let gx = (b - a) * (1.0 - v) + (d - c) * v;
    let gz = (c - a) * (1.0 - u) + (d - b) * u;
    let height = a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
    (height, gx, gz)
}

/// The drop `(a, b)` of a pass, as `drop` of the shader.
#[allow(clippy::too_many_arguments)]
fn drop(p: &P, h: &[f32], n: usize, rain: Option<&[f32]>, hard: Option<&[f32]>, seed: u32, a: u32, b: u32, w: &Writes) {
    let s = STRIDE as f32;
    let mut x = a as f32 * s + rand01(a, b, seed ^ 0x6d2b_79f5) * s;
    let mut z = b as f32 * s + rand01(a, b, seed ^ 0x1b87_3593) * s;
    let last = (n - 1) as f32;
    x = x.min(last - 0.001);
    z = z.min(last - 0.001);
    if let Some(r) = rain {
        let k = z as usize * n + x as usize;
        if rand01(a, b, seed ^ 0x2545_f491) >= r[k].clamp(0.0, 1.0) {
            return;
        }
    }
    let (mut dx, mut dz) = (0.0f32, 0.0f32);
    let (mut speed, mut water, mut sediment) = (1.0f32, 1.0f32, 0.0f32);
    let sea = p.sea / p.sp;
    for _ in 0..MAX_STEPS {
        let (i, j) = (x as usize, z as usize);
        let (u, v) = (x - i as f32, z - j as f32);
        let (height, gx, gz) = height_grad(h, n, p.sp, x, z);
        let cell = j * n + i;
        if height < sea {
            // At the sea: all it carries settles, a delta.
            lay(w, cell, n, u, v, sediment);
            return;
        }
        let mut ndx = dx * p.inertia - gx * (1.0 - p.inertia);
        let mut ndz = dz * p.inertia - gz * (1.0 - p.inertia);
        let len = (ndx * ndx + ndz * ndz).sqrt();
        if len < 1e-6 {
            return;
        }
        ndx /= len;
        ndz /= len;
        let (nx, nz) = (x + ndx, z + ndz);
        if !(0.0..last).contains(&nx) || !(0.0..last).contains(&nz) {
            return;
        }
        let (new_height, _, _) = height_grad(h, n, p.sp, nx, nz);
        let dh = new_height - height;
        let cap = (-dh * speed * water * p.capacity).max(MIN_CAPACITY);
        if sediment > cap || dh > 0.0 {
            // Drop soil: all it takes to fill the hole when going up, part
            // of the excess otherwise.
            let amount = if dh > 0.0 { dh.min(sediment) } else { (sediment - cap) * p.deposition };
            sediment -= amount;
            lay(w, cell, n, u, v, amount);
        } else {
            let soft = 1.0 - 0.9 * hard.map_or(0.0, |m| m[cell].clamp(0.0, 1.0));
            let amount = ((cap - sediment) * p.erosion).min(-dh) * soft;
            wear(p, w, n, i, j, amount);
            sediment += amount;
        }
        w.flow[cell].fetch_add(fix(water, WATER) as u32, Ordering::Relaxed);
        speed = (speed * speed + dh * GRAVITY).max(0.0).sqrt();
        water *= 1.0 - p.evaporation;
        (x, z, dx, dz) = (nx, nz, ndx, ndz);
    }
}

/// `amount` laid on the four samples around a drop.
#[inline]
fn lay(w: &Writes, cell: usize, n: usize, u: f32, v: f32, amount: f32) {
    w.delta[cell].fetch_add(fix(amount * (1.0 - u) * (1.0 - v), DELTA), Ordering::Relaxed);
    w.delta[cell + 1].fetch_add(fix(amount * u * (1.0 - v), DELTA), Ordering::Relaxed);
    w.delta[cell + n].fetch_add(fix(amount * (1.0 - u) * v, DELTA), Ordering::Relaxed);
    w.delta[cell + n + 1].fetch_add(fix(amount * u * v, DELTA), Ordering::Relaxed);
    w.laid[cell].fetch_add(fix(amount, SOIL) as u32, Ordering::Relaxed);
}

/// `amount` worn off the samples within the brush around `(i, j)`.
#[inline]
fn wear(p: &P, w: &Writes, n: usize, i: usize, j: usize, amount: f32) {
    let r = p.r;
    for dz in -r..=r {
        let cj = j as i32 + dz;
        if cj < 0 || cj >= n as i32 {
            continue;
        }
        for dx in -r..=r {
            let ci = i as i32 + dx;
            if ci < 0 || ci >= n as i32 {
                continue;
            }
            let d = ((dx * dx + dz * dz) as f32).sqrt();
            if d <= r as f32 {
                let wt = (1.0 - d / (r as f32 + 1.0)) / p.brush_total;
                w.delta[cj as usize * n + ci as usize].fetch_sub(fix(amount * wt, DELTA), Ordering::Relaxed);
            }
        }
    }
    w.worn[j * n + i].fetch_add(fix(amount, SOIL) as u32, Ordering::Relaxed);
}

/// An accumulation as a mask: its logarithm over the largest's.
#[inline]
fn log_mask(v: u32, unit: f32, max: u32) -> f32 {
    let l = (1.0 + max.max(1) as f32 / unit).ln();
    ((1.0 + v as f32 / unit).ln() / l).clamp(0.0, 1.0)
}

pub(super) fn particle_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("particle erosion has its input");
    let n = src.rect.w as usize;
    let p = P::of(ctx);
    let plane = |i: usize| inputs.get(i).copied().flatten().map(|g| g.plane(0));
    let (hard, rain) = (plane(1), plane(2));
    let mut h = src.plane(0).to_vec();
    let zero_i = || (0..n * n).map(|_| AtomicI32::new(0)).collect::<Vec<_>>();
    let zero_u = || (0..n * n).map(|_| AtomicU32::new(0)).collect::<Vec<_>>();
    let (delta, flow, laid, worn) = (zero_i(), zero_u(), zero_u(), zero_u());
    let m = ((n - 1) / STRIDE) as u32;
    for pass in 0..p.passes {
        if cancel() {
            return None;
        }
        let seed = p.pass_seed(pass);
        let w = Writes { delta: &delta, flow: &flow, laid: &laid, worn: &worn };
        (0..m * m).into_par_iter().for_each(|k| drop(&p, &h, n, rain, hard, seed, k % m, k / m, &w));
        h.par_iter_mut().zip(&delta).for_each(|(v, d)| *v += d.swap(0, Ordering::Relaxed) as f32 * (p.sp / DELTA));
    }
    let max = |a: &[AtomicU32]| a.par_iter().map(|v| v.load(Ordering::Relaxed)).max().unwrap_or(0);
    let mask = |a: &[AtomicU32], unit: f32| {
        let top = max(a);
        a.par_iter().map(|v| log_mask(v.load(Ordering::Relaxed), unit, top)).collect::<Vec<f32>>()
    };
    let (f, wr, l) = (mask(&flow, WATER), mask(&worn, SOIL), mask(&laid, SOIL));
    Some(vec![h, f, wr, l])
}

const PARTICLE_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("particle.wgsl"));

pub(super) fn particle_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let p = P::of(ctx);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let pipe = |e| g.compute("erosion.particle", PARTICLE_WGSL, e);
    let (dropp, applyp, topp, maskp) = (pipe("drop")?, pipe("apply")?, pipe("top_of")?, pipe("masks")?);
    let opt = |i: usize| inputs.get(i).copied().flatten().map(|g| &g.buffer);
    let stand_in = k.buf("particle stand-in", 4);
    let (hard, rain) = (opt(1), opt(2));
    let h = g.grid(src.lod, src.rect, 1);
    k.copy(&src.buffer, &h.buffer, (n * n * 4) as u64);
    let (delta, flow, laid, worn) = (k.buf("particle changes", n * n), k.buf("particle water", n * n), k.buf("particle laid", n * n), k.buf("particle worn", n * n));
    let m = ((n - 1) / STRIDE) as u32;
    let uniform = |seed: u32| {
        let mut u = Uniforms::default();
        u.u32(n as u32).u32(m).u32(seed).i32(p.r);
        u.f32(p.sp).f32(p.erosion).f32(p.deposition).f32(p.capacity);
        u.f32(p.evaporation).f32(p.inertia).f32(p.brush_total).f32(p.sea);
        u.u32(rain.is_some() as u32).u32(hard.is_some() as u32).u32(MAX_STEPS).f32(p.sp / DELTA);
        u
    };
    let au = k.uniform(&mut uniform(0));
    let apply = k.bind(&applyp, &[(0, &au), (2, &delta), (8, &h.buffer)]);
    for pass in 0..p.passes {
        if pass % 8 == 0 && cancel() {
            return None;
        }
        let u = k.uniform(&mut uniform(p.pass_seed(pass)));
        let bg = k.bind(
            &dropp,
            &[(0, &u), (1, &h.buffer), (2, &delta), (3, &flow), (4, &laid), (5, &worn), (6, rain.unwrap_or(&stand_in)), (7, hard.unwrap_or(&stand_in))],
        );
        k.groups(&dropp, &bg, m.div_ceil(16), m.div_ceil(16));
        k.run(&applyp, &apply, n);
    }
    // The largest water, wear and soil, then the masks.
    let tops = k.buf("particle tops", 4);
    let bg = k.bind(&topp, &[(0, &au), (3, &flow), (4, &laid), (5, &worn), (9, &tops)]);
    k.run(&topp, &bg, n);
    let outs: Vec<GpuGrid> = (0..3).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let bg = k.bind(&maskp, &[(0, &au), (3, &flow), (4, &laid), (5, &worn), (9, &tops), (10, &outs[0].buffer), (11, &outs[1].buffer), (12, &outs[2].buffer)]);
    k.run(&maskp, &bg, n);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("erosion.particle", e);
        return None;
    }
    let mut v = vec![h];
    v.extend(outs);
    Some(v)
}
