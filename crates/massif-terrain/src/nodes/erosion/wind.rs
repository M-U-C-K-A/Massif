//! Wind (`wind.erosion`): sand blown into dunes, after Werner's model made
//! synchronous. At each step the wind lifts some sand wherever the surface
//! is not in the lee of a rise (the shadow of a 15° line), carries all it
//! holds one hop downwind, and lays part of it where it lands: all of it
//! in the lee, more on sand than on bare rock; sand steeper than its angle
//! of repose slides (the thermal passes, on sand only). Transverse ridges
//! form where sand is plenty, crescents where it is scarce. Bare rock
//! facing the wind is worn (Abrasion): yardangs.

use super::common::{Kit, rand01};
use super::stream::soft_mask;
use super::{Strata, THERMAL_WGSL, thermal_pass, thermal_uniform};
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use rayon::prelude::*;

/// tan of the lee's line, and of sand's angle of repose.
const SHADOW: f32 = 0.268;
const REPOSE: f32 = 0.65;

/// The settings, read once.
#[derive(Clone, Copy)]
struct P {
    /// The wind's way (towards), as a unit vector.
    dir: [f32; 2],
    /// The hop, in samples.
    hop: f32,
    /// Blocks lifted at a step from sand in the open.
    lift: f32,
    sand: f32,
    abrasion: f32,
    iterations: usize,
    /// Samples the lee is looked for upwind.
    reach: u32,
    sp: f32,
    seed: u32,
}

impl P {
    fn of(ctx: &Ctx) -> Self {
        let sp = ctx.spacing() as f32;
        let a = ctx.params.f("direction").to_radians();
        let hop = (ctx.params.f("hop") / sp).max(1.0);
        Self {
            dir: [a.cos(), a.sin()],
            hop,
            lift: 0.2 * ctx.params.f("strength").max(0.0),
            sand: ctx.params.f("sand_depth").max(0.0),
            abrasion: ctx.params.f("abrasion").max(0.0),
            iterations: ctx.params.int("iterations").max(1) as usize,
            reach: ((64.0 / sp).ceil() as u32).clamp(2, 32),
            sp,
            seed: ctx.seed,
        }
    }
}

/// The surface at `(x, z)` (nearest sample, edges clamped).
#[inline]
fn near(s: &[f32], n: usize, x: f32, z: f32) -> f32 {
    let (i, j) = (((x + 0.5).floor() as i32).clamp(0, n as i32 - 1), ((z + 0.5).floor() as i32).clamp(0, n as i32 - 1));
    s[j as usize * n + i as usize]
}

/// Whether sample `(x, z)` lies in the lee of a rise upwind.
#[inline]
fn in_lee(p: &P, s: &[f32], n: usize, x: usize, z: usize, dir: [f32; 2]) -> bool {
    let here = s[z * n + x];
    for j in 1..=p.reach {
        let t = j as f32;
        let up = near(s, n, x as f32 - dir[0] * t, z as f32 - dir[1] * t);
        if up - here > SHADOW * t * p.sp {
            return true;
        }
    }
    false
}

/// `c` at fractional `(x, z)`, bilinear, 0 outside the grid.
#[inline]
fn bilinear(c: &[f32], n: usize, x: f32, z: f32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (tx, tz) = (x - fx, z - fz);
    let at = |i: f32, j: f32| if i < 0.0 || j < 0.0 || i >= n as f32 || j >= n as f32 { 0.0 } else { c[j as usize * n + i as usize] };
    let a = at(fx, fz) + (at(fx + 1.0, fz) - at(fx, fz)) * tx;
    let b = at(fx, fz + 1.0) + (at(fx + 1.0, fz + 1.0) - at(fx, fz + 1.0)) * tx;
    a + (b - a) * tz
}

/// The wind's way at sample `k`: the vector input's, or the setting's.
#[inline]
fn dir_at(p: &P, wind: Option<&[f32]>, k: usize, n: usize) -> [f32; 2] {
    match wind {
        Some(v) => {
            let (x, z) = (v[k], v[n * n + k]);
            let l = (x * x + z * z).sqrt();
            if l > 1e-6 { [x / l, z / l] } else { p.dir }
        }
        None => p.dir,
    }
}

pub(super) fn wind_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("wind has its input");
    let n = src.rect.w as usize;
    let p = P::of(ctx);
    let mask = inputs.get(1).copied().flatten().map(|g| g.plane(0));
    let wind = inputs.get(2).copied().flatten().filter(|g| g.planes >= 2).map(|g| &g.data[..]);
    let mut b = src.plane(0).to_vec();
    let mut sand: Vec<f32> = (0..n * n).into_par_iter().map(|k| p.sand * mask.map_or(1.0, |m| m[k].clamp(0.0, 1.0)) * (0.8 + 0.4 * rand01((k % n) as u32, (k / n) as u32, p.seed))).collect();
    let mut fly = vec![0.0f32; n * n];
    let mut carry = vec![0.0f32; n * n];
    let mut lee = vec![0u8; n * n];
    let mut worn = vec![0.0f32; n * n];
    let (mut s, mut s2) = (vec![0.0f32; n * n], vec![0.0f32; n * n]);
    let (mut shed, mut total) = (vec![0.0f32; n * n], vec![0.0f32; n * n]);
    let mut loose = vec![0.0f32; n * n];
    let drop = [REPOSE * p.sp, REPOSE * p.sp * std::f32::consts::SQRT_2];
    for it in 0..p.iterations {
        if it % 16 == 0 && cancel() {
            return None;
        }
        s.par_iter_mut().enumerate().for_each(|(k, v)| *v = b[k] + sand[k]);
        // The lee, what is lifted, and the rock worn.
        lee.par_iter_mut().zip(carry.par_iter_mut()).zip(sand.par_iter_mut()).zip(b.par_iter_mut().zip(worn.par_iter_mut())).enumerate().for_each(|(k, (((l, c), sk), (bk, wk)))| {
            let (x, z) = (k % n, k / n);
            let dir = dir_at(&p, wind, k, n);
            let shadow = in_lee(&p, &s, n, x, z, dir);
            *l = shadow as u8;
            let q = if shadow { 0.0 } else { sk.min(p.lift) };
            *sk -= q;
            *c = fly[k] + q;
            if !shadow && *sk < 0.05 && p.abrasion > 0.0 {
                // Bare rock facing the wind.
                let ahead = near(&s, n, x as f32 + dir[0], z as f32 + dir[1]);
                let face = ((ahead - s[k]) / p.sp).max(0.0);
                let e = p.abrasion * 0.05 * (0.2 + face);
                *bk -= e;
                *wk += e;
            }
        });
        // Each sample's landing: what was carried a hop upwind of it.
        fly.par_iter_mut().zip(sand.par_iter_mut()).enumerate().for_each(|(k, (f, sk))| {
            let (x, z) = ((k % n) as f32, (k / n) as f32);
            let dir = dir_at(&p, wind, k, n);
            let landed = bilinear(&carry, n, x - dir[0] * p.hop, z - dir[1] * p.hop);
            let keep = if lee[k] != 0 { 1.0 } else if *sk > 0.05 { 0.6 } else { 0.4 };
            let d = landed * keep;
            *sk += d;
            *f = landed - d;
        });
        // Sand steeper than its repose slides; rock does not.
        s.par_iter_mut().zip(loose.par_iter_mut()).enumerate().for_each(|(k, (v, hard))| {
            *v = b[k] + sand[k];
            *hard = 1.0 - (sand[k] / 0.5).clamp(0.0, 1.0);
        });
        thermal_pass(&s, &mut s2, &mut shed, &mut total, n, drop, 0.5, Some(&loose), &Strata::default());
        sand.par_iter_mut().enumerate().for_each(|(k, v)| *v = (s2[k] - b[k]).max(0.0));
    }
    let height: Vec<f32> = b.par_iter().zip(&sand).map(|(a, c)| a + c).collect();
    let (wr, _) = super::stream::refs(&worn, &[]);
    Some(vec![height, sand.par_iter().map(|v| (v / 8.0).clamp(0.0, 1.0)).collect(), worn.par_iter().map(|&v| soft_mask(v, wr)).collect()])
}

const WIND_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("wind.wgsl"));

pub(super) fn wind_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let p = P::of(ctx);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let pipe = |e| g.compute("wind.erosion", WIND_WGSL, e);
    let (initp, liftp, landp, composep, sandp, outp) = (pipe("init")?, pipe("lift")?, pipe("land")?, pipe("compose")?, pipe("sand_back")?, pipe("outputs")?);
    let tp = |e| g.compute("erosion.thermal", THERMAL_WGSL, e);
    let (excess, gather) = (tp("excess")?, tp("gather")?);
    let stand_in = k.buf("wind stand-in", 4);
    let mask = inputs.get(1).copied().flatten().map(|g| &g.buffer);
    let wind = inputs.get(2).copied().flatten().filter(|g| g.planes >= 2).map(|g| &g.buffer);
    let mut u = Uniforms::default();
    u.u32(n as u32).u32(mask.is_some() as u32).u32(wind.is_some() as u32).u32(p.reach);
    u.f32(p.dir[0]).f32(p.dir[1]).f32(p.hop).f32(p.lift);
    u.f32(p.sand).f32(p.abrasion).f32(p.sp).u32(p.seed);
    u.f32(0.0).f32(0.0).f32(0.0).f32(0.0);
    let ub = k.uniform(&mut u);
    let b = k.buf("wind rock", n * n);
    k.copy(&src.buffer, &b, (n * n * 4) as u64);
    let (sand, fly, carry, lee, worn) = (k.buf("wind sand", n * n), k.buf("wind flying", n * n), k.buf("wind carried", n * n), k.buf("wind lee", n * n), k.buf("wind worn", n * n));
    let (s, s2, shed, total, loose) = (k.buf("wind surface", n * n), k.buf("wind surface", n * n), k.buf("thermal shed", n * n), k.buf("thermal total", n * n), k.buf("wind loose", n * n));
    let bg = k.bind(&initp, &[(0, &ub), (2, &sand), (6, mask.unwrap_or(&stand_in))]);
    k.run(&initp, &bg, n);
    let tu = thermal_uniform(g, n as u32, 0.5, [REPOSE * p.sp, REPOSE * p.sp * std::f32::consts::SQRT_2], true, &Strata::default());
    let wind_b = wind.unwrap_or(&stand_in);
    let compose = k.bind(&composep, &[(0, &ub), (1, &b), (2, &sand), (8, &s), (9, &loose)]);
    let lift = k.bind(&liftp, &[(0, &ub), (1, &b), (2, &sand), (3, &fly), (4, &carry), (5, &lee), (7, wind_b), (8, &s), (10, &worn)]);
    let land = k.bind(&landp, &[(0, &ub), (2, &sand), (3, &fly), (4, &carry), (5, &lee), (7, wind_b)]);
    let ex = k.bind(&excess, &[(0, &tu), (1, &s), (2, &shed), (3, &total), (5, &loose)]);
    let ga = k.bind(&gather, &[(0, &tu), (1, &s), (2, &shed), (3, &total), (4, &s2)]);
    let back = k.bind(&sandp, &[(0, &ub), (1, &b), (2, &sand), (11, &s2)]);
    for it in 0..p.iterations {
        if it % 32 == 0 && cancel() {
            return None;
        }
        k.run(&composep, &compose, n);
        k.run(&liftp, &lift, n);
        k.run(&landp, &land, n);
        k.run(&composep, &compose, n);
        k.run(&excess, &ex, n);
        k.run(&gather, &ga, n);
        k.run(&sandp, &back, n);
    }
    k.flush();
    let worn_now = g.download_buffers(&[&worn]).pop().unwrap_or_default();
    let (wr, _) = super::stream::refs(&worn_now[..n * n], &[]);
    let mut uo = Uniforms::default();
    uo.u32(n as u32).u32(0).u32(0).u32(0);
    uo.f32(0.0).f32(0.0).f32(0.0).f32(0.0);
    uo.f32(0.0).f32(wr).f32(p.sp).u32(0);
    uo.f32(0.0).f32(0.0).f32(0.0).f32(0.0);
    let uo = k.uniform(&mut uo);
    let outs: Vec<GpuGrid> = (0..3).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let bg = k.bind(&outp, &[(0, &uo), (1, &b), (2, &sand), (10, &worn), (12, &outs[0].buffer), (13, &outs[1].buffer), (14, &outs[2].buffer)]);
    k.run(&outp, &bg, n);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("wind.erosion", e);
        return None;
    }
    Some(outs)
}
