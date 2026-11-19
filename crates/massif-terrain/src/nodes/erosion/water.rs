//! Water over a relief: where it drains (`water.flow`), the sea that the
//! terrain's edge lets in (`water.sea`), the lakes its hollows hold
//! (`water.lakes`) and the rivers it gathers into (`water.rivers`), on the
//! CPU and the graphics card alike.
//!
//! A water output is the surface of the water where there is some, and
//! [`DRY`] blocks under the ground elsewhere, so that resampled to finer
//! levels it never rises above the ground by accident.

use super::common::{Kit, NONE, wnoise};
use super::flow::{self, FlowGpu, accumulate, lowest_upstream, nearest, ord, outlet, rain_scale, receivers, unord};
use crate::gpu::{Gpu, GpuGrid};
use crate::kernel::{Ctx, Grid, Uniforms};
use rayon::prelude::*;

/// How far under the ground a water output lies where there is no water.
pub(super) const DRY: f32 = 64.0;

const WATER_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("water.wgsl"));

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

/// `v` within `[min, max]` (1), fading out over `falloff` on each side, as
/// Massif 1.2's selections.
#[inline]
fn band(v: f32, min: f32, max: f32, falloff: f32) -> f32 {
    let f = falloff.max(1e-4);
    ((v - min) / f + 0.5).clamp(0.0, 1.0).min(((max - v) / f + 0.5).clamp(0.0, 1.0))
}

// ---- the sea

/// The sea's settings: its level, the shore's width (blocks of height),
/// and whether only the water the terrain's edge lets in counts.
fn sea_settings(ctx: &Ctx) -> (f32, f32, bool) {
    (ctx.params.f("level"), ctx.params.f("shore").max(0.5), ctx.params.flag("connected"))
}

/// The sea at one sample: its water (surface or dry) and shore, from the
/// ground and the fill from the terrain's edge.
#[inline]
fn sea_at(h: f32, w: f32, level: f32, shore: f32, connected: bool) -> (f32, f32) {
    let wet = h < level && (!connected || w < level);
    (if wet { level } else { h - DRY }, band(h, level - shore, level + shore, shore * 0.5))
}

pub(super) fn sea_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("the sea has its input");
    let n = src.rect.w as usize;
    let h = src.plane(0);
    let (level, shore, connected) = sea_settings(ctx);
    // Filled from the terrain's edge alone: under the level where a way
    // to the edge stays under it.
    let w = if connected { flow::fill(h, n, f32::MIN, 0.0, 0, cancel)? } else { h.to_vec() };
    let (water, coast): (Vec<f32>, Vec<f32>) = h.par_iter().zip(&w).map(|(&v, &f)| sea_at(v, f, level, shore, connected)).unzip();
    Some(vec![h.to_vec(), water, coast])
}

pub(super) fn sea_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let (level, shore, connected) = sea_settings(ctx);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let fl = FlowGpu::new(&k)?;
    let seap = g.compute("water", WATER_WGSL, "sea")?;
    let w = if connected { fl.fill(&mut k, &src.buffer, n, f32::MIN, 0.0, 0, cancel)? } else { k.tmp(n * n) };
    let outs: Vec<GpuGrid> = (0..3).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let mut u = uniform(n, ctx.spacing() as f32);
    u.set(F_LEVEL, level).set(F_SHORE, shore).set_u(U_FLAGS, connected as u32);
    let ub = u.buffer(&k);
    let bg = k.bind(&seap, &[(0, &ub), (1, &src.buffer), (2, &w), (12, &outs[0].buffer), (8, &outs[1].buffer), (9, &outs[2].buffer)]);
    k.run(&seap, &bg, n);
    k.put(w);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("water.sea", e);
        return None;
    }
    Some(outs)
}

// ---- lakes

/// How much of each hollow is filled with ground (0: all water), and the
/// sea's level, under which nothing is a lake.
fn lake_settings(ctx: &Ctx) -> (f32, f32) {
    (ctx.params.f("amount").clamp(0.0, 1.0), ctx.params.f("sea"))
}

/// A lake at one sample: the ground raised, the lakes mask and the water,
/// from the ground and its hollows filled to where they spill.
#[inline]
fn lake_at(h: f32, w: f32, amount: f32, sea: f32) -> (f32, f32, f32) {
    let depth = (w - h).max(0.0);
    let ground = h + depth * amount;
    let held = depth * (1.0 - amount);
    let water = if held >= 1.0 && w > sea + 0.2 { w } else { ground - DRY };
    (ground, (held / 1.5).clamp(0.0, 1.0), water)
}

pub(super) fn lakes_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("lakes have their input");
    let n = src.rect.w as usize;
    let h = src.plane(0);
    let (amount, sea) = lake_settings(ctx);
    let w = flow::fill(h, n, sea, 0.0, 0, cancel)?;
    let mut out = vec![Vec::with_capacity(n * n), Vec::with_capacity(n * n), Vec::with_capacity(n * n)];
    for (&v, &f) in h.iter().zip(&w) {
        let (a, b, c) = lake_at(v, f, amount, sea);
        out[0].push(a);
        out[1].push(b);
        out[2].push(c);
    }
    Some(out)
}

pub(super) fn lakes_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let (amount, sea) = lake_settings(ctx);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let fl = FlowGpu::new(&k)?;
    let lakep = g.compute("water", WATER_WGSL, "lakes")?;
    let w = fl.fill(&mut k, &src.buffer, n, sea, 0.0, 0, cancel)?;
    let outs: Vec<GpuGrid> = (0..3).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let mut u = uniform(n, ctx.spacing() as f32);
    u.set(F_AMOUNT, amount).set(F_SEA, sea);
    let ub = u.buffer(&k);
    let bg = k.bind(&lakep, &[(0, &ub), (1, &src.buffer), (2, &w), (12, &outs[0].buffer), (9, &outs[1].buffer), (8, &outs[2].buffer)]);
    k.run(&lakep, &bg, n);
    k.put(w);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("water.lakes", e);
        return None;
    }
    Some(outs)
}

// ---- rivers

/// The rivers' settings, as Massif 1.2's Rivers node has them.
#[derive(Clone, Copy)]
struct Rivers {
    threshold: f32,
    depth: f32,
    width: f32,
    sea: f32,
    banks: f32,
    /// Hollows at least this deep hold lakes (blocks; infinite: none).
    lake_depth: f32,
    sp: f32,
    seed: u32,
    /// How far the rivers wander from the water's straight way, in
    /// samples, and the waves of their meanders, per sample.
    meander: f32,
    meander_freq: f32,
}

impl Rivers {
    fn of(ctx: &Ctx) -> Self {
        let lakes = ctx.params.f("lakes");
        Self {
            threshold: ctx.params.f("threshold").clamp(0.0, 1.0),
            depth: ctx.params.f("depth").max(0.5),
            width: ctx.params.f("width").max(1.0),
            sea: ctx.params.f("sea"),
            banks: ctx.params.f("banks").max(0.0),
            lake_depth: if lakes <= 0.001 { f32::MAX } else { 1.0 + (1.0 - lakes.min(1.0)) * 24.0 },
            sp: ctx.spacing() as f32,
            // The shader keeps 31 bits of it.
            seed: ctx.seed & 0x7fff_ffff,
            meander: ctx.params.f("meander").max(0.0) * 1.2 * ctx.params.f("width").max(1.0) / ctx.spacing() as f32,
            meander_freq: ctx.spacing() as f32 / (8.0 * ctx.params.f("width")).max(32.0),
        }
    }

    /// Where sample `(x, z)` looks for its river: moved by two noises, so
    /// that the rivers' beds wander around the water's way in meanders.
    #[inline]
    fn wander(&self, x: f32, z: f32, n: usize) -> (f32, f32) {
        if self.meander <= 0.0 {
            return (x, z);
        }
        let (u, v) = (x * self.meander_freq, z * self.meander_freq);
        let last = (n - 1) as f32;
        let wx = self.meander * wnoise(u, v, self.seed ^ 0x0a5a_5a5a);
        let wz = self.meander * wnoise(u + 31.7, v + 17.3, self.seed ^ 0x05a5_a5a5);
        ((x + wx).clamp(0.0, last), (z + wz).clamp(0.0, last))
    }

    /// The least area draining to a river, in samples, on `land` samples
    /// of land, as 1.2 sets it.
    fn least(&self, land: f32) -> f32 {
        let t = self.threshold;
        let cell = self.sp * self.sp;
        (land * 10f32.powf(-3.6 + 2.6 * t)).min(10f32.powf(4.6 + 2.6 * t) / cell).max(4.0)
    }

    /// How far the carving of a river reaches, in samples.
    fn reach(&self) -> usize {
        let hw = 0.5 * self.width.max(2.0);
        ((hw + (self.banks * 2.0 * hw).max(48.0)) / self.sp + self.meander).ceil() as usize + 1
    }
}

/// What a river does at a sample `d` blocks from the nearest sample of a
/// river (draining `share` of the largest river's area, its water at
/// `level`, the ground there `g`), the ground there being `h`: the ground
/// dug, the rivers mask, the water (or `None`).
#[inline]
fn river_at(p: &Rivers, h: f32, d: f32, share: f32, level: f32, g: f32) -> (f32, f32, Option<f32>) {
    let hw = 0.5 * (p.width * share.sqrt()).max(0.25 * p.width).max(2.0);
    let depth = (p.depth * share.powf(0.4)).max(1.0);
    let bank = (p.banks * 2.0 * hw).max(p.sp).max((2.5 * (g - level)).clamp(0.0, 48.0));
    if d <= hw {
        let t = d / hw;
        let bed = level - depth * (1.0 - t * t);
        let ground = h.min(bed);
        (ground, 1.0, (level > ground + 0.1).then_some(level))
    } else if d <= hw + bank {
        let t = (d - hw) / bank;
        let s = t * t * (3.0 - 2.0 * t);
        (h.min(level + (h - level) * s), 1.0 - s, None)
    } else {
        (h, 0.0, None)
    }
}

/// The ground where water already lies (an outlet), as the rivers see it.
#[inline]
fn wet(h: f32, water: Option<f32>) -> f32 {
    match water {
        Some(w) if w > h + 0.5 => -1.0e6,
        _ => h,
    }
}

pub(super) fn rivers_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("rivers have their input");
    let n = src.rect.w as usize;
    let h = src.plane(0);
    let water_in = inputs.get(1).copied().flatten().map(|g| g.plane(0));
    let p = Rivers::of(ctx);
    let hp: Vec<f32> = (0..n * n).into_par_iter().map(|k| wet(h[k], water_in.map(|w| w[k]))).collect();
    let w = flow::fill(&hp, n, p.sea, 1e-3 * p.sp, p.seed, cancel)?;
    let rcv = receivers(&hp, &w, n, p.sea, p.seed ^ 0x5bd1_e995);
    let acc = accumulate(&rcv, None, n);
    let scale = rain_scale(n);
    let is_out = |k: usize| outlet(&hp, k % n, k / n, n, p.sea);
    let land = (0..n * n).into_par_iter().filter(|&k| !is_out(k)).count() as f32;
    let top = (0..n * n).into_par_iter().filter(|&k| !is_out(k)).map(|k| acc[k]).max().unwrap_or(1).max(1) as f32 / scale;
    let least = p.least(land);
    let lake = |k: usize| !is_out(k) && w[k] - h[k] >= p.lake_depth;
    let ground: Vec<u32> = (0..n * n).into_par_iter().map(|k| ord(if lake(k) { w[k] } else { h[k] })).collect();
    let lowest = lowest_upstream(&rcv, ground);
    let seeds: Vec<u32> = (0..n * n).into_par_iter().map(|k| if !is_out(k) && !lake(k) && acc[k] as f32 / scale >= least { k as u32 } else { NONE }).collect();
    let near = nearest(seeds, n, p.reach());
    let mut out = vec![vec![0.0; n * n], vec![0.0; n * n], vec![0.0; n * n]];
    let [o0, o1, o2] = &mut out[..] else { unreachable!() };
    o0.par_iter_mut().zip(o1.par_iter_mut()).zip(o2.par_iter_mut()).enumerate().for_each(|(k, ((g0, m), wa))| {
        let (px, pz) = p.wander((k % n) as f32, (k / n) as f32, n);
        let at = (pz + 0.5).floor() as usize * n + (px + 0.5).floor() as usize;
        let (ground, mask, water) = match near[at] {
            NONE => (h[k], 0.0, None),
            s => {
                let s = s as usize;
                let (dx, dz) = ((s % n) as f32 - px, (s / n) as f32 - pz);
                let d = (dx * dx + dz * dz).sqrt() * p.sp;
                let share = (acc[s] as f32 / scale / top).clamp(0.0, 1.0);
                river_at(&p, h[k], d, share, unord(lowest[s]) - 0.25, h[s])
            }
        };
        let lake_water = lake(k).then_some(w[k]);
        let mut surface = water.or(lake_water).unwrap_or(ground - DRY);
        if let Some(wi) = water_in {
            surface = surface.max(wi[k]);
        }
        (*g0, *m, *wa) = (ground, mask, surface);
    });
    Some(out)
}

pub(super) fn rivers_gpu(g: &Gpu, ctx: &Ctx, inputs: &[Option<&GpuGrid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<GpuGrid>> {
    let src = inputs[0]?;
    let n = src.side() as usize;
    let p = Rivers::of(ctx);
    let water_in = inputs.get(1).copied().flatten().map(|g| &g.buffer);
    let mut k = Kit::new(g);
    let scope = g.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let fl = FlowGpu::new(&k)?;
    let pipe = |e| g.compute("water", WATER_WGSL, e);
    let (prepp, statsp, seedsp, carvep) = (pipe("river_prep")?, pipe("river_stats")?, pipe("river_seeds")?, pipe("river_carve")?);
    let stand_in = k.buf("rivers stand-in", 4);
    let mut u = uniform(n, p.sp);
    u.set(F_SEA, p.sea).set(F_LAKE_DEPTH, p.lake_depth).set(F_WIDTH, p.width).set(F_DEPTH, p.depth).set(F_BANKS, p.banks).set(F_MEANDER, p.meander).set(F_MEANDER_FREQ, p.meander_freq);
    u.set_u(U_FLAGS, water_in.is_some() as u32 | (p.seed << 1));
    let ub = u.buffer(&k);
    let hp = k.buf("rivers ground", n * n);
    let bg = k.bind(&prepp, &[(0, &ub), (1, &src.buffer), (6, water_in.unwrap_or(&stand_in)), (7, &hp)]);
    k.run(&prepp, &bg, n);
    let w = fl.fill(&mut k, &hp, n, p.sea, 1e-3 * p.sp, p.seed, cancel)?;
    let (rcv, acc) = (k.buf("rivers receivers", n * n), k.buf("rivers drainage", n * n));
    fl.receivers(&mut k, &hp, &w, n, p.sea, p.seed ^ 0x5bd1_e995, &rcv);
    fl.accumulate(&mut k, &rcv, None, n, &acc);
    // The land and the largest river, read back for the least river.
    let stats = k.buf("rivers stats", 4);
    let bg = k.bind(&statsp, &[(0, &ub), (3, &acc), (7, &hp), (11, &stats)]);
    k.run(&statsp, &bg, n);
    k.flush();
    let got = g.download_buffers(&[&stats]).pop().unwrap_or_default();
    let (land, top) = (got[0].to_bits() as f32, got[1].to_bits().max(1) as f32);
    u.set(F_LEAST, p.least(land) * rain_scale(n)).set(F_TOP, top);
    let ub = u.buffer(&k);
    let (ground, near) = (k.buf("rivers ground order", n * n), k.buf("rivers nearest", n * n));
    let bg = k.bind(&seedsp, &[(0, &ub), (1, &src.buffer), (2, &w), (3, &acc), (5, &near), (7, &hp), (10, &ground)]);
    k.run(&seedsp, &bg, n);
    let lowest = k.buf("rivers level", n * n);
    fl.lowest_upstream(&mut k, &rcv, &ground, n, &lowest);
    fl.nearest(&mut k, &near, n, p.reach());
    let outs: Vec<GpuGrid> = (0..3).map(|_| g.grid(src.lod, src.rect, 1)).collect();
    let bg = k.bind(
        &carvep,
        &[(0, &ub), (1, &src.buffer), (2, &w), (3, &acc), (4, &lowest), (5, &near), (6, water_in.unwrap_or(&stand_in)), (7, &hp), (12, &outs[0].buffer), (9, &outs[1].buffer), (8, &outs[2].buffer)],
    );
    k.run(&carvep, &bg, n);
    k.put(w);
    k.flush();
    if let Some(e) = pollster::block_on(scope.pop()) {
        g.refuse("water.rivers", e);
        return None;
    }
    Some(outs)
}

// ---- the uniform block of `water.wgsl`

const U_FLAGS: usize = 1;
const F_SEA: usize = 3;
const F_LEVEL: usize = 4;
const F_SHORE: usize = 5;
const F_AMOUNT: usize = 6;
const F_LAKE_DEPTH: usize = 7;
const F_LEAST: usize = 8;
const F_TOP: usize = 9;
const F_WIDTH: usize = 10;
const F_DEPTH: usize = 11;
const F_BANKS: usize = 12;
const F_MEANDER: usize = 13;
const F_MEANDER_FREQ: usize = 14;

/// The uniform block: sixteen 4-byte fields (`n`, flags, spacing, then the
/// settings at the indices above, then the rain scale).
struct Block {
    bytes: Vec<u8>,
}

impl Block {
    fn set(&mut self, i: usize, v: f32) -> &mut Self {
        self.bytes[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        self
    }

    fn set_u(&mut self, i: usize, v: u32) -> &mut Self {
        self.bytes[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        self
    }

    fn buffer(&self, k: &Kit) -> wgpu::Buffer {
        k.uniform(&mut Uniforms { bytes: self.bytes.clone() })
    }
}

fn uniform(n: usize, sp: f32) -> Block {
    let mut u = Uniforms::default();
    u.u32(n as u32).u32(0).f32(sp);
    for _ in 3..15 {
        u.f32(0.0);
    }
    u.f32(rain_scale(n));
    Block { bytes: u.finish() }
}
