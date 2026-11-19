//! Glaciers (`ice.glacier`): snow above the equilibrium line becomes ice,
//! the ice flows down the valleys and melts below the line, and where it
//! slides it scours its bed: U-shaped valleys, cirques at their heads,
//! hanging valleys, fjords where it reaches below the sea, moraines at its
//! edges.
//!
//! The ice is the shallow-ice approximation: its flux is `D·∇s`, `s` the
//! ice's surface, `D = Γd·H⁵·|∇s|² + Γs·H³·|∇s|` (Glen's flow law, n = 3,
//! and sliding with the basal stress squared), kept under what an explicit
//! step allows. The bed is worn in proportion to the sliding speed
//! (Hallet's abrasion). Glaciers are wide: the simulation runs at most
//! [`GLACIER_MAX`] samples a side, and the change it makes is brought back
//! to the simulation's resolution.

use super::common::{Kit, Resample, coarser, down_avg, up_add};
use super::stream::soft_mask;
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use rayon::prelude::*;

/// The most samples a side the ice is simulated at.
const GLACIER_MAX: usize = 1024;

/// The settings, read once.
#[derive(Clone, Copy)]
struct P {
    ela: f32,
    /// Ice gained per step and per block above the line (lost below).
    beta: f32,
    deform: f32,
    slide: f32,
    /// Blocks of bed worn per step at a sliding speed of 1 block a step.
    wear: f32,
    moraines: f32,
    iterations: usize,
    /// Blocks between samples where the ice runs.
    sp: f32,
}

impl P {
    fn of(ctx: &Ctx, sp: f32) -> Self {
        let flow = ctx.params.f("flow").max(0.0);
        Self {
            ela: ctx.params.f("ela"),
            beta: 0.002 * ctx.params.f("accumulation").max(0.0),
            deform: 2e-7 * flow,
            slide: 2e-3 * flow,
            wear: 0.6 * ctx.params.f("strength").max(0.0),
            moraines: ctx.params.f("moraines").max(0.0),
            iterations: ctx.params.int("iterations").max(1) as usize,
            sp,
        }
    }

    /// The largest `D` an explicit step takes.
    fn d_max(&self) -> f32 {
        0.24 * self.sp * self.sp
    }
}

/// Ice gained (lost) in a step at surface `s`, snow `snow` (0–1).
#[inline]
fn balance(p: &P, s: f32, snow: f32) -> f32 {
    let m = p.beta * (s - p.ela);
    if m > 0.0 { (m * snow).min(0.5) } else { m.max(-2.0) }
}

/// The ice's `D` at a sample, and its sliding speed (blocks a step), from
/// the ice's thickness and the slope of its surface.
#[inline]
fn diffusivity(p: &P, h: f32, grad: f32) -> (f32, f32) {
    let g2 = grad * grad;
    let d = (p.deform * h.powi(5) * g2 + p.slide * h.powi(3) * grad).min(p.d_max());
    let slide = (p.slide * h * h * g2).min(p.sp);
    (d, slide)
}

/// The slope of `s` at `(x, z)` by central differences, edges clamped.
#[inline]
fn grad_at(s: impl Fn(usize) -> f32, x: usize, z: usize, n: usize, sp: f32) -> f32 {
    let (xl, xr, zl, zr) = (x.saturating_sub(1), (x + 1).min(n - 1), z.saturating_sub(1), (z + 1).min(n - 1));
    let gx = (s(z * n + xr) - s(z * n + xl)) / ((xr - xl).max(1) as f32 * sp);
    let gz = (s(zr * n + x) - s(zl * n + x)) / ((zr - zl).max(1) as f32 * sp);
    (gx * gx + gz * gz).sqrt()
}

/// One step: the bed `b` and ice `h` into `b2` and `h2`, the bed worn
/// added to `worn`.
#[allow(clippy::too_many_arguments)]
fn step(p: &P, b: &[f32], h: &[f32], d: &mut [f32], slide: &mut [f32], b2: &mut [f32], h2: &mut [f32], worn: &mut [f32], snow: Option<&[f32]>, n: usize) {
    d.par_iter_mut().zip(slide.par_iter_mut()).enumerate().for_each(|(k, (dk, sk))| {
        let g = grad_at(|i| b[i] + h[i], k % n, k / n, n, p.sp);
        (*dk, *sk) = diffusivity(p, h[k], g);
    });
    let inv = 1.0 / (p.sp * p.sp);
    h2.par_iter_mut().zip(b2.par_iter_mut()).zip(worn.par_iter_mut()).enumerate().for_each(|(k, ((hk, bk), wk))| {
        let (x, z) = (k % n, k / n);
        let s = b[k] + h[k];
        let mut flux = 0.0f32;
        for (dx, dz) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            let (nx, nz) = (x as i32 + dx, z as i32 + dz);
            if nx < 0 || nz < 0 || nx >= n as i32 || nz >= n as i32 {
                continue;
            }
            let j = nz as usize * n + nx as usize;
            flux += 0.5 * (d[k] + d[j]) * (b[j] + h[j] - s);
        }
        let ice = (h[k] + flux * inv + balance(p, s, snow.map_or(1.0, |m| m[k].clamp(0.0, 1.0)))).max(0.0);
        let e = p.wear * slide[k] * (h[k] / 5.0).min(1.0);
        *hk = ice;
        *bk = b[k] - e;
        *wk += e;
    });
}

/// The margins of the ice where it melts back: near ice, little of it.
#[inline]
fn margin(h: &[f32], k: usize, n: usize) -> f32 {
    let (x, z) = ((k % n) as i32, (k / n) as i32);
    let mut most = 0.0f32;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let (nx, nz) = (x + dx, z + dz);
            if nx >= 0 && nz >= 0 && nx < n as i32 && nz < n as i32 {
                most = most.max(h[nz as usize * n + nx as usize]);
            }
        }
    }
    ((most / 8.0).min(1.0) - h[k] / 2.0).clamp(0.0, 1.0)
}

/// The run at the glacier's resolution: the bed, the ice, the bed worn.
fn run_cpu(p: &P, b0: &[f32], snow: Option<&[f32]>, n: usize, cancel: &(dyn Fn() -> bool + Sync)) -> Option<(Vec<f32>, Vec<f32>, Vec<f32>)> {
    let (mut b, mut h) = (b0.to_vec(), vec![0.0f32; n * n]);
    let (mut b2, mut h2) = (vec![0.0f32; n * n], vec![0.0f32; n * n]);
    let (mut d, mut slide, mut worn) = (vec![0.0f32; n * n], vec![0.0f32; n * n], vec![0.0f32; n * n]);
    for it in 0..p.iterations {
        if it % 16 == 0 && cancel() {
            return None;
        }
        step(p, &b, &h, &mut d, &mut slide, &mut b2, &mut h2, &mut worn, snow, n);
        std::mem::swap(&mut b, &mut b2);
        std::mem::swap(&mut h, &mut h2);
    }
    Some((b, h, worn))
}

/// The levels from the simulation's grid down to the glacier's.
fn sizes(n: usize) -> Vec<usize> {
    let mut v = vec![n];
    while *v.last().unwrap() > GLACIER_MAX {
        let m = coarser(*v.last().unwrap());
        v.push(m);
    }
    v
}

pub(super) fn glacier_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("glaciers have their input");
    let n = src.rect.w as usize;
    let levels = sizes(n);
    let m = *levels.last().unwrap();
    let sp = ctx.spacing() as f32 * (1u32 << (levels.len() - 1)) as f32;
    let p = P::of(ctx, sp);
    let (mut b0, mut snow) = (src.plane(0).to_vec(), inputs.get(1).copied().flatten().map(|g| g.plane(0).to_vec()));
    for w in levels.windows(2) {
        b0 = down_avg(&b0, w[0]);
        snow = snow.map(|s| down_avg(&s, w[0]));
    }
    let (b, h, worn) = run_cpu(&p, &b0, snow.as_deref(), m, cancel)?;
    let laid: Vec<f32> = (0..m * m).into_par_iter().map(|k| margin(&h, k, m)).collect();
    let mut change: Vec<f32> = (0..m * m).into_par_iter().map(|k| b[k] - b0[k] + p.moraines * laid[k]).collect();
    let (wr, _) = super::stream::refs(&worn, &[]);
    let mut masks = [h.par_iter().map(|v| (v / 30.0).clamp(0.0, 1.0)).collect::<Vec<f32>>(), worn.par_iter().map(|&v| soft_mask(v, wr)).collect(), laid];
    for w in levels.windows(2).rev() {
        change = up_add(&change, None, w[0]);
        for mk in &mut masks {
            *mk = up_add(mk, None, w[0]);
        }
    }
    let height: Vec<f32> = src.plane(0).par_iter().zip(&change).map(|(a, c)| a + c).collect();
    let [ice, wear, dep] = masks;
    Some(vec![height, ice, wear, dep])
}

const GLACIER_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("glacier.wgsl"));

pub(super) fn glacier_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let levels = sizes(n);
    let m = *levels.last().unwrap();
    let sp = ctx.spacing() as f32 * (1u32 << (levels.len() - 1)) as f32;
    let p = P::of(ctx, sp);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let rs = Resample::new(&k)?;
    let pipe = |e| g.compute("ice.glacier", GLACIER_WGSL, e);
    let (dp, stepp, outp, plusp) = (pipe("diffusivity")?, pipe("step")?, pipe("outputs")?, pipe("plus")?);
    let stand_in = k.buf("glacier stand-in", 4);
    // The bed and the snow at the glacier's resolution.
    let mut b0 = k.buf("glacier bed", n * n);
    k.copy(&src.buffer, &b0, (n * n * 4) as u64);
    let mut snow = inputs.get(1).copied().flatten().map(|s| {
        let c = k.buf("glacier snow", n * n);
        k.copy(&s.buffer, &c, (n * n * 4) as u64);
        c
    });
    for w in levels.windows(2) {
        b0 = rs.down(&mut k, &b0, w[0], false);
        snow = snow.map(|s| rs.down(&mut k, &s, w[0], false));
    }
    let mut u = Uniforms::default();
    u.u32(m as u32).u32(snow.is_some() as u32).f32(p.sp).f32(p.ela);
    u.f32(p.beta).f32(p.deform).f32(p.slide).f32(p.wear);
    u.f32(p.moraines).f32(p.d_max()).f32(0.0).f32(0.0);
    let ub = k.uniform(&mut u);
    let (ba, bb, ha, hb) = (k.buf("glacier bed", m * m), k.buf("glacier bed", m * m), k.buf("glacier ice", m * m), k.buf("glacier ice", m * m));
    k.copy(&b0, &ba, (m * m * 4) as u64);
    let (d, slide, worn) = (k.buf("glacier d", m * m), k.buf("glacier slide", m * m), k.buf("glacier worn", m * m));
    let snow_b = snow.as_ref().unwrap_or(&stand_in);
    let groups = [(&ba, &ha, &bb, &hb), (&bb, &hb, &ba, &ha)].map(|(b, h, b2, h2)| {
        (k.bind(&dp, &[(0, &ub), (1, b), (2, h), (5, &d), (6, &slide)]), k.bind(&stepp, &[(0, &ub), (1, b), (2, h), (3, b2), (4, h2), (5, &d), (6, &slide), (7, &worn), (8, snow_b)]))
    });
    for it in 0..p.iterations {
        if it % 32 == 0 && cancel() {
            return None;
        }
        let (dg, sg) = &groups[it % 2];
        k.run(&dp, dg, m);
        k.run(&stepp, sg, m);
    }
    let (b, h) = if p.iterations.is_multiple_of(2) { (&ba, &ha) } else { (&bb, &hb) };
    // The wear's reference: its mean, read back.
    k.flush();
    let worn_now = g.download_buffers(&[&worn]).pop().unwrap_or_default();
    let (wr, _) = super::stream::refs(&worn_now[..m * m], &[]);
    let mut u = Uniforms::default();
    u.u32(m as u32).u32(0).f32(p.sp).f32(p.ela);
    u.f32(p.beta).f32(p.deform).f32(p.slide).f32(p.wear);
    u.f32(p.moraines).f32(p.d_max()).f32(wr).f32(0.0);
    let uo = k.uniform(&mut u);
    let outs: Vec<wgpu::Buffer> = (0..4).map(|_| k.buf("glacier out", m * m)).collect();
    let bg = k.bind(&outp, &[(0, &uo), (1, b), (2, h), (7, &worn), (9, &b0), (10, &outs[0]), (11, &outs[1]), (12, &outs[2]), (13, &outs[3])]);
    k.run(&outp, &bg, m);
    // Back to the simulation's grid: the change and the masks.
    let mut ups = outs;
    for w in levels.windows(2).rev() {
        ups = ups
            .iter()
            .map(|c| {
                let f = k.buf("glacier up", w[0] * w[0]);
                rs.up(&mut k, c, None, w[0], &f);
                f
            })
            .collect();
    }
    let grids: Vec<GpuGrid> = (0..4).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let bytes = (n * n * 4) as u64;
    let un = k.uniform(Uniforms::default().u32(n as u32).u32(0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0).f32(0.0));
    let bg = k.bind(&plusp, &[(0, &un), (1, &src.buffer), (3, &grids[0].buffer), (9, &ups[0])]);
    k.run(&plusp, &bg, n);
    for i in 1..4 {
        k.copy(&ups[i], &grids[i].buffer, bytes);
    }
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("ice.glacier", e);
        return None;
    }
    Some(grids)
}
