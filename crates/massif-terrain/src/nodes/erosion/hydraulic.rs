//! Erosion, version 2 (`erosion.stream@2`): the rivers of the relief cut
//! their beds by stream power, carry what they cut and lay it down where
//! they slow, while the slopes between them creep and slide: dendritic
//! valleys with sharp ridges between them, flat floors and fans at their
//! feet. Clean-room from the published models:
//!
//! - incision `∂h/∂t = −K·A^m·S` (A the area draining through a sample, S
//!   its slope), solved implicitly along the drainage from the outlets up
//!   (Braun and Willett 2013), so that a step may be as long as the run
//!   needs: a bed never digs below where it drains to;
//! - deposition after Davy and Lague (2009): the sediment a river carries
//!   settles over a length that grows with the river, faster where it is
//!   gentle, so that small streams fill their valley floors and large
//!   rivers carry their load out;
//! - hillslopes: linear creep, then loose rock steeper than the talus
//!   angle sliding down (the thermal passes of [`super::thermal_pass`]).
//!
//! It runs coarse to fine: the drainage of the whole terrain is organised
//! on a grid of [`COARSEST`] samples or so, each finer level taking the
//! change the coarser made and refining it, so that a river runs on from
//! one valley to the next at every scale. Every sample's water way comes
//! from the filled relief ([`super::flow::fill`]), so that nothing stops
//! in a hollow. Everything is in sequence or in deterministic parallel
//! passes: the same input gives the same output, on any machine.

use super::common::{DIST, NONE, coarser, down_avg, rand01, up_add};
use super::flow::{self, receivers};
use super::stream::{refs, sediment_of, soft_mask};
use super::{Strata, thermal_pass};
use crate::kernel::{Ctx, Grid};
use rayon::prelude::*;

/// Levels go no coarser than this many samples a side.
const COARSEST: usize = 96;

/// The finest level simulated has at least this many samples a side.
const FINEST_SIDE: usize = 128;

/// The draining area the strength is measured against, blocks²: a river
/// draining this much cuts back about Duration × its side over the run.
const A0: f32 = 1024.0 * 1024.0;

/// The settings, read once.
#[derive(Clone, Copy, Debug)]
pub(super) struct P {
    /// K·T over the whole run, per block of √A0.
    pub kt: f32,
    pub m: f32,
    /// Draining area under which rivers do not cut, blocks².
    pub channel: f32,
    /// The width of the smallest valleys, blocks.
    pub width: f32,
    /// How readily sediment settles (0: carried out).
    pub settle: f32,
    /// Creep over the run, blocks².
    pub creep: f32,
    /// tan of the talus angle.
    pub talus: f32,
    pub steps: usize,
    pub seed: u32,
    /// Below this the water has left: the sea or the base level.
    pub base: f32,
}

impl P {
    pub fn of(ctx: &Ctx) -> Self {
        let down = ctx.params.f("downcutting").clamp(0.0, 1.0);
        let soft = ctx.params.f("softness").clamp(0.0, 1.0);
        let scale = ctx.params.f("scale").max(1.0);
        Self {
            kt: ctx.params.f("duration").max(0.0) * (0.25 + 1.5 * soft),
            m: 0.35 + 0.25 * down,
            channel: scale * scale,
            width: scale,
            settle: 1.0 - ctx.params.f("removal").clamp(0.0, 1.0),
            // Creep rounds what is smaller than about the erosion scale,
            // less as the rivers cut down more.
            creep: (0.5 * scale * (1.0 - down) * (0.5 + soft)).powi(2),
            talus: ctx.params.f("talus").clamp(5.0, 85.0).to_radians().tan(),
            steps: ctx.params.int("steps").max(1) as usize,
            seed: ctx.seed,
            base: ctx.params.f("base").max(ctx.space.sea_level),
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

/// The samples in an order where each comes after the one it drains to
/// (outlets first): breadth first up the drainage from every outlet.
fn stack(rcv: &[u32]) -> Vec<u32> {
    let len = rcv.len();
    let mut start = vec![0u32; len + 1];
    for &r in rcv {
        if r != NONE {
            start[r as usize + 1] += 1;
        }
    }
    for k in 0..len {
        start[k + 1] += start[k];
    }
    let mut fill = start.clone();
    let mut donors = vec![0u32; start[len] as usize];
    for (k, &r) in rcv.iter().enumerate() {
        if r != NONE {
            donors[fill[r as usize] as usize] = k as u32;
            fill[r as usize] += 1;
        }
    }
    let mut order: Vec<u32> = (0..len as u32).filter(|&k| rcv[k as usize] == NONE).collect();
    let mut i = 0;
    while i < order.len() {
        let k = order[i] as usize;
        order.extend_from_slice(&donors[start[k] as usize..start[k + 1] as usize]);
        i += 1;
    }
    order
}

/// The filled surface of `h`: hollows filled to where they spill, each
/// sample a little above the one it was reached from (`eps`, varied by
/// the seed), so that water crosses flats and lakes (priority flood,
/// Barnes et al. 2014): outlets first, then always the lowest sample
/// reached.
fn flood(h: &[f32], n: usize, base: f32, eps: f32, seed: u32) -> Vec<f32> {
    use std::cmp::Reverse;
    let mut w = vec![f32::MAX; n * n];
    let mut heap = std::collections::BinaryHeap::with_capacity(4 * n);
    for k in 0..n * n {
        if flow::outlet(h, k % n, k / n, n, base) {
            w[k] = h[k];
            heap.push(Reverse((flow::ord(h[k]), k as u32)));
        }
    }
    while let Some(Reverse((_, k))) = heap.pop() {
        let k = k as usize;
        let (x, z) = (k % n, k / n);
        for d in 0..8 {
            if let Some(j) = super::common::neighbour(x, z, n, d)
                && w[j] == f32::MAX
            {
                w[j] = h[j].max(w[k] + eps * (0.25 + 1.5 * rand01((j % n) as u32, (j / n) as u32, seed)));
                heap.push(Reverse((flow::ord(w[j]), j as u32)));
            }
        }
    }
    w
}

/// The rain at each sample, varied by the seed by up to a tenth so that
/// two seeds drain differently.
fn rain_of(rain: Option<&[f32]>, n: usize, seed: u32) -> Vec<f32> {
    (0..n * n).into_par_iter().map(|k| rain.map_or(1.0, |r| r[k].clamp(0.0, 1.0)) * (0.9 + 0.2 * rand01((k % n) as u32, (k / n) as u32, seed))).collect()
}

/// How far `r` lies from `k`, in samples.
#[inline]
fn dist(k: usize, r: usize, n: usize) -> f32 {
    if k % n != r % n && k / n != r / n { DIST[0] } else { 1.0 }
}

/// What a run gives.
pub(super) struct Run {
    pub h: Vec<f32>,
    /// Samples draining through each (rain-weighted).
    pub area: Vec<f32>,
    /// Sediment each sample sends on at the last step, blocks over a sample.
    pub qs: Vec<f32>,
    /// Blocks worn away and laid down at each sample.
    pub worn: Vec<f32>,
    pub laid: Vec<f32>,
}

/// One level of the run: `steps` steps of `dt` on `h` (`n` a side, `sp`
/// blocks between samples).
#[allow(clippy::too_many_arguments)]
fn level(p: &P, h: &mut [f32], worn: &mut [f32], laid: &mut [f32], n: usize, sp: f32, dt: f32, l: usize, hard: Option<&[f32]>, rain: &[f32], cancel: &(dyn Fn() -> bool + Sync)) -> Option<(Vec<f32>, Vec<f32>)> {
    let cells = n * n;
    let eps = 1e-3 * sp;
    let cell = sp * sp;
    let drop = [p.talus * sp, p.talus * sp * DIST[0]];
    let (mut next, mut shed, mut total) = (vec![0.0; cells], vec![0.0; cells], vec![0.0; cells]);
    let mut area = vec![0.0f32; cells];
    let mut qs = vec![0.0f32; cells];
    let mut qin = vec![0.0f32; cells];
    let mut lowest = vec![0.0f32; cells];
    // The strength at each sample once its draining area is known.
    let mut f = vec![0.0f32; cells];
    let soft: Vec<f32> = (0..cells).into_par_iter().map(|k| 1.0 - 0.95 * hard.map_or(0.0, |m| m[k].clamp(0.0, 1.0))).collect();
    let k_eff = p.kt * A0.sqrt() * dt / A0.powf(p.m);
    let ac = p.channel.powf(p.m);
    let creep = p.creep * dt / cell;
    let substeps = (creep / 0.2).ceil().max(1.0) as usize;
    let mut rcv = Vec::new();
    let mut order = Vec::new();
    for it in 0..p.steps {
        if cancel() {
            return None;
        }
        // The water's way, found again every other step (the implicit
        // solution holds on a way a step old).
        if it % 2 == 0 {
            let w = flood(h, n, p.base, eps, p.seed ^ l as u32);
            rcv = receivers(h, &w, n, p.base, p.seed ^ (it as u32).wrapping_mul(0x9e37_79b9) ^ ((l as u32) << 24));
            order = stack(&rcv);
            area.copy_from_slice(rain);
            for &k in order.iter().rev() {
                let r = rcv[k as usize];
                if r != NONE {
                    area[r as usize] += area[k as usize];
                }
            }
            f.par_iter_mut().enumerate().for_each(|(k, v)| {
                let r = rcv[k];
                *v = if r == NONE { 0.0 } else { k_eff * soft[k] * ((area[k] * cell).powf(p.m) - ac).max(0.0) / (dist(k, r as usize, n) * sp) };
            });
        }
        // Incision, outlets first: each bed lowered towards its receiver's
        // new height.
        let before = h.to_vec();
        for &k in &order {
            let k = k as usize;
            let r = rcv[k];
            if r == NONE || f[k] == 0.0 {
                continue;
            }
            let hr = h[r as usize];
            if h[k] > hr {
                h[k] = (h[k] + f[k] * hr) / (1.0 + f[k]);
            }
        }
        // Sediment, sources first: what was cut runs down and settles
        // over a length that grows with the river, at once where the
        // water stops (a hollow, the sea).
        qin.iter_mut().for_each(|v| *v = 0.0);
        lowest.iter_mut().for_each(|v| *v = f32::MAX);
        for &k in order.iter().rev() {
            let k = k as usize;
            let e = (before[k] - h[k]).max(0.0);
            worn[k] += e;
            let q = qin[k] + e;
            let r = rcv[k];
            let frac = if r == NONE {
                if flow::outlet(h, k % n, k / n, n, p.base) { if h[k] < p.base { 0.5 * p.settle } else { 0.0 } } else { 1.0 }
            } else if h[k] < p.base {
                0.5 * p.settle
            } else {
                let slope = ((h[k] - h[r as usize]) / (dist(k, r as usize, n) * sp)).max(0.0);
                let len = dist(k, r as usize, n) * sp;
                (p.settle * 4.0 * len / (area[k] * cell).sqrt() / (1.0 + slope / 0.05)).min(1.0)
            };
            // Never above where the water came from: no dams.
            let room = if r == NONE {
                f32::MAX
            } else {
                let top = if lowest[k] == f32::MAX { before[k] } else { lowest[k].max(before[k]) };
                (top - h[k]).max(0.0)
            };
            let dep = (q * frac).min(room);
            h[k] += dep;
            laid[k] += dep;
            let out = q - dep;
            qs[k] = out;
            if r != NONE {
                let r = r as usize;
                qin[r] += out;
                lowest[r] = lowest[r].min(h[k]);
            }
        }
        // Creep, then loose rock sliding.
        for _ in 0..substeps {
            let c = creep / substeps as f32;
            if c <= 0.0 {
                break;
            }
            let src: &[f32] = h;
            next.par_chunks_mut(n).enumerate().for_each(|(z, row)| {
                for (x, o) in row.iter_mut().enumerate() {
                    let at = |dx: i32, dz: i32| src[((z as i32 + dz).clamp(0, n as i32 - 1) as usize) * n + (x as i32 + dx).clamp(0, n as i32 - 1) as usize];
                    let v = src[z * n + x];
                    *o = v + c * soft[z * n + x] * (at(-1, 0) + at(1, 0) + at(0, -1) + at(0, 1) - 4.0 * v);
                }
            });
            h.copy_from_slice(&next);
        }
        thermal_pass(h, &mut next, &mut shed, &mut total, n, drop, 0.5, hard, &Strata::default());
        h.copy_from_slice(&next);
    }
    Some((area, qs))
}

/// The run over the whole terrain: `h0` (`n` a side, `sp0` blocks between
/// samples), coarse to fine.
pub(super) fn run(p: &P, h0: &[f32], n: usize, sp0: f32, hard: Option<&[f32]>, rain: Option<&[f32]>, cancel: &(dyn Fn() -> bool + Sync)) -> Option<Run> {
    let sizes = levels(n);
    let mut hin = vec![h0.to_vec()];
    let mut hards = vec![hard.map(<[f32]>::to_vec)];
    let mut rains = vec![rain.map(<[f32]>::to_vec)];
    for l in 1..sizes.len() {
        let m = sizes[l - 1];
        hin.push(down_avg(&hin[l - 1], m));
        hards.push(hards[l - 1].as_ref().map(|v| down_avg(v, m)));
        rains.push(rains[l - 1].as_ref().map(|v| down_avg(v, m)));
    }
    // The finest level simulated: as fine as the smallest valleys are
    // wide (Erosion scale), up to two levels coarser than the grid and
    // never under [`FINEST_SIDE`] samples a side, so that rivers end two
    // or more samples wide rather than notches of one.
    let mut finest = 0;
    while finest < 2 && finest + 1 < sizes.len() && sizes[finest + 1] >= FINEST_SIDE && sp0 * (2u32 << finest) as f32 <= p.width {
        finest += 1;
    }
    let dt = 1.0 / (p.steps * (sizes.len() - finest)) as f32;
    let mut h: Vec<f32> = Vec::new();
    let (mut worn, mut laid): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    let mut last = (Vec::new(), Vec::new());
    for l in (finest..sizes.len()).rev() {
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
        last = level(p, &mut h, &mut worn, &mut laid, n, sp, dt, l, hards[l].as_deref(), &rain, cancel)?;
    }
    for l in (0..finest).rev() {
        let n = sizes[l];
        let change: Vec<f32> = h.par_iter().zip(&hin[l + 1]).map(|(a, b)| a - b).collect();
        h = up_add(&change, Some(&hin[l]), n);
        (worn, laid) = (up_add(&worn, None, n), up_add(&laid, None, n));
        // A coarse sample drains four fine ones.
        last = (up_add(&last.0, None, n).into_iter().map(|a| 4.0 * a).collect(), up_add(&last.1, None, n));
    }
    Some(Run { h, area: last.0, qs: last.1, worn, laid })
}

/// The node's outputs: height, flow, wear, deposition, sediment.
fn outputs(r: Run, n: usize) -> Vec<Vec<f32>> {
    let flow: Vec<f32> = r.area.par_iter().map(|&a| flow::flow_mask(a, n, 1.5)).collect();
    let (wr, dr) = refs(&r.worn, &r.laid);
    let wear = r.worn.par_iter().map(|&d| soft_mask(d, wr)).collect();
    let dep = r.laid.par_iter().map(|&d| soft_mask(d, dr)).collect();
    let sed = r.qs.par_iter().map(|&q| sediment_of(q)).collect();
    vec![r.h, flow, wear, dep, sed]
}

pub(super) fn erosion_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let src = inputs[0].expect("erosion has its input");
    let n = src.rect.w as usize;
    let plane = |i: usize| inputs.get(i).copied().flatten().map(|g| g.plane(0));
    let r = run(&P::of(ctx), src.plane(0), n, ctx.spacing() as f32, plane(1), plane(2), cancel)?;
    Some(outputs(r, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_puts_each_sample_after_its_receiver() {
        // A 3 × 3 grid draining to its corner 0 along rows then the column.
        let rcv = vec![NONE, 0, 1, 0, 3, 4, 3, 6, 7];
        let order = stack(&rcv);
        assert_eq!(order.len(), 9);
        let pos: Vec<usize> = (0..9).map(|k| order.iter().position(|&o| o as usize == k).unwrap()).collect();
        for (k, &r) in rcv.iter().enumerate() {
            if r != NONE {
                assert!(pos[r as usize] < pos[k]);
            }
        }
    }
}
