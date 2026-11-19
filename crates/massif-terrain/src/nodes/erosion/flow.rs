//! The water's way over a relief, on the CPU and the graphics card alike:
//!
//! - [`fill`]: the hollows filled up to where they spill, each sample at
//!   least `eps` above the one it drains to, so that water crosses flats
//!   and lakes. Found by lowering an upper bound until nothing moves
//!   (Planchon and Darboux), the bound coming from the same fill one
//!   level coarser, so that a few dozen steps do;
//! - [`receivers`]: where each sample's water goes, its steepest way down
//!   the filled surface (none at the terrain's edge and under the sea);
//! - [`accumulate`]: how much rain drains through each sample, added up
//!   along the receivers by jumps that double each step, in whole
//!   numbers: the sums do not depend on the order of the additions, and
//!   the card and the CPU give the same.

use super::common::{DIST, Kit, NONE, Resample, coarser, down_max, neighbour, rand01};
use crate::kernel::Uniforms;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub(super) const FLOW_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("flow.wgsl"));

/// The finest grid the coarse-to-fine fill stops at.
const TOP: usize = 64;

/// Whether sample `(x, z)` lets its water out: the terrain's edge and the
/// sea.
#[inline]
pub(super) fn outlet(h: &[f32], x: usize, z: usize, n: usize, sea: f32) -> bool {
    x == 0 || z == 0 || x == n - 1 || z == n - 1 || h[z * n + x] < sea
}

/// The units of rain a sample counts for when it is all rain: as many as
/// keep the largest sum within 2³⁰.
pub(super) fn rain_scale(n: usize) -> f32 {
    ((1u64 << 30) / (n * n) as u64).clamp(1, 4096) as f32
}

/// One step of the fill: `w` lowered into `out`; whether anything moved.
fn fill_step(h: &[f32], w: &[f32], out: &mut [f32], n: usize, sea: f32, eps: f32, seed: u32) -> bool {
    let moved = AtomicBool::new(false);
    out.par_chunks_mut(n).enumerate().for_each(|(z, row)| {
        let mut any = false;
        for (x, o) in row.iter_mut().enumerate() {
            let k = z * n + x;
            if outlet(h, x, z, n, sea) {
                *o = h[k];
                continue;
            }
            let mut lo = 3.0e38f32;
            for d in 0..8 {
                if let Some(j) = neighbour(x, z, n, d) {
                    lo = lo.min(w[j]);
                }
            }
            // Each sample its own rise, so that water wanders over flats
            // instead of running in straight lines.
            let v = w[k].min(h[k].max(lo + eps * (0.25 + 1.5 * rand01(x as u32, z as u32, seed))));
            any |= v < w[k];
            *o = v;
        }
        if any {
            moved.store(true, Ordering::Relaxed);
        }
    });
    moved.into_inner()
}

/// Lowers `w` (above the fill everywhere) until nothing moves, at most
/// `steps` steps (in pairs); `None` when cancelled.
#[allow(clippy::too_many_arguments)]
pub(super) fn fill_from(h: &[f32], mut w: Vec<f32>, n: usize, sea: f32, eps: f32, seed: u32, steps: usize, cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    let mut b = vec![0.0; n * n];
    let mut done = 0;
    while done < steps {
        if done % 64 == 0 && cancel() {
            return None;
        }
        let m1 = fill_step(h, &w, &mut b, n, sea, eps, seed);
        let m2 = fill_step(h, &b, &mut w, n, sea, eps, seed);
        done += 2;
        if !m1 && !m2 {
            break;
        }
    }
    Some(w)
}

/// The filled surface of `h` (`n` a side), each sample `eps` blocks or so
/// (a quarter to 1.75 times, by `seed`) above the one it drains to.
pub(super) fn fill(h: &[f32], n: usize, sea: f32, eps: f32, seed: u32, cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    let init = if n > TOP {
        let m = coarser(n);
        let c = fill(&down_max(h, n), m, sea, eps * 4.0, seed, cancel)?;
        (0..n * n)
            .into_par_iter()
            .map(|k| {
                let (x, z) = (k % n, k / n);
                if outlet(h, x, z, n, sea) {
                    return h[k];
                }
                let (cx, cz) = (x.div_ceil(2).min(m - 1), z.div_ceil(2).min(m - 1));
                h[k].max(c[cz * m + cx] + 7.0 * eps)
            })
            .collect()
    } else {
        (0..n * n).map(|k| if outlet(h, k % n, k / n, n, sea) { h[k] } else { 1.0e30 }).collect()
    };
    fill_from(h, init, n, sea, eps, seed, 8 * n, cancel)
}

/// How much more a way down counts, by `seed` (0.7 to 1.3 times its
/// slope): water does not always take the very steepest way, which keeps
/// streams from lining up with the grid.
#[inline]
pub(super) fn jitter(k: usize, d: usize, seed: u32) -> f32 {
    0.7 + 0.6 * rand01(k as u32, d as u32, seed)
}

/// Where each sample's water goes: its steepest way down `w` (varied by
/// [`jitter`]), [`NONE`] at outlets and at the bottoms of hollows left.
pub(super) fn receivers(h: &[f32], w: &[f32], n: usize, sea: f32, seed: u32) -> Vec<u32> {
    (0..n * n)
        .into_par_iter()
        .map(|k| {
            let (x, z) = (k % n, k / n);
            let mut best = NONE;
            if !outlet(h, x, z, n, sea) {
                let mut s = 0.0f32;
                for (d, dist) in DIST.iter().enumerate() {
                    if let Some(j) = neighbour(x, z, n, d) {
                        let t = (w[k] - w[j]) / dist * jitter(k, d, seed);
                        if t > s {
                            s = t;
                            best = j as u32;
                        }
                    }
                }
            }
            best
        })
        .collect()
}

/// The rain draining through each sample, in units of [`rain_scale`] a
/// sample: each sample's rain (`rain`, 0–1, or all) added to every sample
/// down its way.
pub(super) fn accumulate(rcv: &[u32], rain: Option<&[f32]>, n: usize) -> Vec<u32> {
    let scale = rain_scale(n);
    let mut d: Vec<u32> = (0..n * n).into_par_iter().map(|k| (rain.map_or(1.0, |r| r[k].clamp(0.0, 1.0)) * scale).round() as u32).collect();
    let mut j = rcv.to_vec();
    loop {
        let next: Vec<AtomicU32> = d.par_iter().map(|&v| AtomicU32::new(v)).collect();
        (0..n * n).into_par_iter().for_each(|k| {
            if j[k] != NONE {
                next[j[k] as usize].fetch_add(d[k], Ordering::Relaxed);
            }
        });
        let jumped: Vec<u32> = j.par_iter().map(|&t| if t == NONE { NONE } else { j[t as usize] }).collect();
        let active = j.par_iter().any(|&t| t != NONE);
        d = next.into_iter().map(AtomicU32::into_inner).collect();
        j = jumped;
        if !active {
            break;
        }
    }
    d
}

/// A float as a whole number in the same order (to keep the least by
/// whole-number atomics), and back.
#[inline]
pub(super) fn ord(v: f32) -> u32 {
    let b = v.to_bits();
    if b & 0x8000_0000 != 0 { !b } else { b | 0x8000_0000 }
}

#[inline]
pub(super) fn unord(b: u32) -> f32 {
    f32::from_bits(if b & 0x8000_0000 != 0 { b & 0x7fff_ffff } else { !b })
}

/// The least of `values` (whole numbers, as [`ord`] makes them) over each
/// sample and all those whose water passes through it, by the jumps of
/// [`accumulate`].
pub(super) fn lowest_upstream(rcv: &[u32], values: Vec<u32>) -> Vec<u32> {
    let mut d = values;
    let mut j = rcv.to_vec();
    loop {
        let next: Vec<AtomicU32> = d.par_iter().map(|&v| AtomicU32::new(v)).collect();
        (0..d.len()).into_par_iter().for_each(|k| {
            if j[k] != NONE {
                next[j[k] as usize].fetch_min(d[k], Ordering::Relaxed);
            }
        });
        let jumped: Vec<u32> = j.par_iter().map(|&t| if t == NONE { NONE } else { j[t as usize] }).collect();
        let active = j.par_iter().any(|&t| t != NONE);
        d = next.into_iter().map(AtomicU32::into_inner).collect();
        j = jumped;
        if !active {
            break;
        }
    }
    d
}

/// The jumps of the nearest-seed search for a reach of `reach` samples:
/// halving powers of two from the reach down to 1, then 1 again.
fn jumps(reach: usize) -> Vec<usize> {
    let mut s = reach.max(1).next_power_of_two();
    let mut v = Vec::new();
    while s >= 1 {
        v.push(s);
        s /= 2;
    }
    v.push(1);
    v
}

/// For each sample, the nearest of the seeds (samples whose `seeds` entry
/// is their own index; [`NONE`] elsewhere) within about `reach` samples,
/// found by jump flooding: each sample looks at the seeds its neighbours
/// `s` apart know, `s` halving. Ties go to the lower index, so that the
/// card finds the same.
pub(super) fn nearest(mut seeds: Vec<u32>, n: usize, reach: usize) -> Vec<u32> {
    for s in jumps(reach) {
        let prev = seeds.clone();
        seeds.par_chunks_mut(n).enumerate().for_each(|(z, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut best = prev[z * n + x];
                let mut bd = dist2(x, z, best, n);
                for dz in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (cx, cz) = (x as i64 + dx * s as i64, z as i64 + dz * s as i64);
                        if cx < 0 || cz < 0 || cx >= n as i64 || cz >= n as i64 {
                            continue;
                        }
                        let c = prev[cz as usize * n + cx as usize];
                        if c == NONE {
                            continue;
                        }
                        let d = dist2(x, z, c, n);
                        if d < bd || (d == bd && c < best) {
                            (best, bd) = (c, d);
                        }
                    }
                }
                *out = best;
            }
        });
    }
    seeds
}

/// Squared distance in samples from `(x, z)` to sample `s` ([`NONE`]: as
/// far as can be).
#[inline]
fn dist2(x: usize, z: usize, s: u32, n: usize) -> u32 {
    if s == NONE {
        return u32::MAX;
    }
    let (sx, sz) = ((s as usize % n) as i64, (s as usize / n) as i64);
    let (dx, dz) = (sx - x as i64, sz - z as i64);
    (dx * dx + dz * dz) as u32
}

/// Draining area (in samples) as a mask: its logarithm over the whole
/// terrain's, raised to `gamma` (small rills show more under 1).
#[inline]
pub(super) fn flow_mask(a: f32, n: usize, gamma: f32) -> f32 {
    ((1.0 + a).ln() / (1.0 + (n * n) as f32).ln()).clamp(0.0, 1.0).powf(gamma)
}

/// The same on the graphics card.
pub(super) struct FlowGpu {
    top: wgpu::ComputePipeline,
    up: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
    rcv: wgpu::ComputePipeline,
    init: wgpu::ComputePipeline,
    copy: wgpu::ComputePipeline,
    add: wgpu::ComputePipeline,
    mask: wgpu::ComputePipeline,
    min_init: wgpu::ComputePipeline,
    min_add: wgpu::ComputePipeline,
    jfa: wgpu::ComputePipeline,
    pub rs: Resample,
    count: wgpu::Buffer,
}

impl FlowGpu {
    pub fn new(k: &Kit) -> Option<Self> {
        let p = |e| k.pipe("erosion.flow", FLOW_WGSL, e);
        Some(Self {
            top: p("fill_top")?,
            up: p("fill_up")?,
            step: p("fill_step")?,
            rcv: p("receivers")?,
            init: p("acc_init")?,
            copy: p("acc_copy")?,
            add: p("acc_add")?,
            mask: p("flow_mask")?,
            min_init: p("min_init")?,
            min_add: p("min_add")?,
            jfa: p("jfa_step")?,
            rs: Resample::new(k)?,
            count: k.buf("erosion count", 4),
        })
    }

    fn uniform(k: &Kit, n: usize, m: usize, sea: f32, eps: f32, with_rain: bool, seed: u32) -> wgpu::Buffer {
        k.uniform(Uniforms::default().u32(n as u32).u32(m as u32).f32(sea).f32(eps).f32(7.0 * eps).u32(with_rain as u32).f32(rain_scale(n)).f32(1.0).u32(seed))
    }

    /// Lowers `w` (in place) as [`fill_from`] does, `steps` at most;
    /// `false` when cancelled.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_from(&self, k: &mut Kit, h: &wgpu::Buffer, w: &wgpu::Buffer, n: usize, sea: f32, eps: f32, seed: u32, steps: usize, cancel: &(dyn Fn() -> bool + Sync)) -> bool {
        let u = Self::uniform(k, n, n, sea, eps, false, seed);
        let b = k.tmp(n * n);
        let ab = k.bind(&self.step, &[(0, &u), (1, h), (2, w), (3, &b), (4, &self.count)]);
        let ba = k.bind(&self.step, &[(0, &u), (1, h), (2, &b), (3, w), (4, &self.count)]);
        // Asked whether anything moved every so many steps: reading back
        // waits for the card.
        let every = if n >= 512 { 32 } else { 16 };
        let mut done = 0;
        while done < steps {
            if cancel() {
                k.put(b);
                return false;
            }
            k.clear(&self.count);
            for _ in 0..every / 2 {
                k.run(&self.step, &ab, n);
                k.run(&self.step, &ba, n);
            }
            done += every;
            if k.read_u32(&self.count) == 0 {
                break;
            }
        }
        k.put(b);
        true
    }

    /// The filled surface of `h`, as [`fill`] makes it, in a scratch
    /// buffer; `None` when cancelled.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(&self, k: &mut Kit, h: &wgpu::Buffer, n: usize, sea: f32, eps: f32, seed: u32, cancel: &(dyn Fn() -> bool + Sync)) -> Option<wgpu::Buffer> {
        let w = k.tmp(n * n);
        if n > TOP {
            let m = coarser(n);
            let hc = self.rs.down(k, h, n, true);
            let c = self.fill(k, &hc, m, sea, eps * 4.0, seed, cancel)?;
            let u = Self::uniform(k, n, m, sea, eps, false, seed);
            let bg = k.bind(&self.up, &[(0, &u), (1, h), (2, &c), (3, &w)]);
            k.run(&self.up, &bg, n);
            k.put(hc);
            k.put(c);
        } else {
            let u = Self::uniform(k, n, n, sea, eps, false, seed);
            let bg = k.bind(&self.top, &[(0, &u), (1, h), (3, &w)]);
            k.run(&self.top, &bg, n);
        }
        self.fill_from(k, h, &w, n, sea, eps, seed, 8 * n, cancel).then_some(w)
    }

    /// Each sample's receiver into `rcv`, as [`receivers`].
    #[allow(clippy::too_many_arguments)]
    pub fn receivers(&self, k: &mut Kit, h: &wgpu::Buffer, w: &wgpu::Buffer, n: usize, sea: f32, seed: u32, rcv: &wgpu::Buffer) {
        let u = Self::uniform(k, n, n, sea, 0.0, false, seed);
        let bg = k.bind(&self.rcv, &[(0, &u), (1, h), (2, w), (5, rcv)]);
        k.run(&self.rcv, &bg, n);
    }

    /// The rain draining through each sample, as [`accumulate`] adds it
    /// up, into `out` (whole numbers).
    pub fn accumulate(&self, k: &mut Kit, rcv: &wgpu::Buffer, rain: Option<&wgpu::Buffer>, n: usize, out: &wgpu::Buffer) {
        let u = Self::uniform(k, n, n, 0.0, 0.0, rain.is_some(), 0);
        let (d1, j0, j1) = (k.tmp(n * n), k.tmp(n * n), k.tmp(n * n));
        let bg = k.bind(&self.init, &[(0, &u), (5, rcv), (6, rain.unwrap_or(&self.count)), (7, out), (9, &j0)]);
        k.run(&self.init, &bg, n);
        self.double(k, &u, out, (d1, j0, j1), &self.add, n);
    }

    /// The lowest of `values` (as [`ord`] keeps floats in whole numbers)
    /// over each sample and all those whose water passes through it, into
    /// `out`: what a level that only goes down along the water's way is.
    pub fn lowest_upstream(&self, k: &mut Kit, rcv: &wgpu::Buffer, values: &wgpu::Buffer, n: usize, out: &wgpu::Buffer) {
        let u = Self::uniform(k, n, n, 0.0, 0.0, false, 0);
        let (d1, j0, j1) = (k.tmp(n * n), k.tmp(n * n), k.tmp(n * n));
        let bg = k.bind(&self.min_init, &[(0, &u), (5, rcv), (6, values), (7, out), (9, &j0)]);
        k.run(&self.min_init, &bg, n);
        self.double(k, &u, out, (d1, j0, j1), &self.min_add, n);
    }

    /// The doubling jumps of [`FlowGpu::accumulate`] with `add` (sums or
    /// least), from `out` and the first jumps in `j0`.
    fn double(&self, k: &mut Kit, u: &wgpu::Buffer, out: &wgpu::Buffer, (d1, j0, j1): (wgpu::Buffer, wgpu::Buffer, wgpu::Buffer), add: &wgpu::ComputePipeline, n: usize) {
        let pairs = [(out, &d1, &j0, &j1), (&d1, out, &j1, &j0)];
        let groups: Vec<_> = pairs
            .iter()
            .map(|(a, b, ja, jb)| (k.bind(&self.copy, &[(0, u), (4, &self.count), (7, a), (8, b), (9, ja), (10, jb)]), k.bind(add, &[(0, u), (7, a), (8, b), (9, ja)])))
            .collect();
        // Two steps at a time, so that the sums end in `out`; stops once no
        // sample jumps any more (a path of 2^steps samples at most).
        for _ in 0..32 {
            k.clear(&self.count);
            for (copy, adding) in &groups {
                k.run(&self.copy, copy, n);
                k.run(add, adding, n);
            }
            if k.read_u32(&self.count) == 0 {
                break;
            }
        }
        k.put(d1);
        k.put(j0);
        k.put(j1);
    }
}

impl FlowGpu {
    /// The nearest of the samples `seeds` holds (its own index, or [`NONE`])
    /// for each sample within `reach` samples, in `seeds`, as [`nearest`].
    pub fn nearest(&self, k: &mut Kit, seeds: &wgpu::Buffer, n: usize, reach: usize) {
        let other = k.tmp(n * n);
        let mut from_seeds = true;
        for s in jumps(reach) {
            let u = k.uniform(Uniforms::default().u32(n as u32).u32(s as u32).f32(0.0).f32(0.0).f32(0.0).u32(0).f32(0.0).f32(0.0).u32(0));
            let (a, b) = if from_seeds { (seeds, &other) } else { (&other, seeds) };
            let bg = k.bind(&self.jfa, &[(0, &u), (9, a), (10, b)]);
            k.run(&self.jfa, &bg, n);
            from_seeds = !from_seeds;
        }
        if !from_seeds {
            k.copy(&other, seeds, (n * n * 4) as u64);
        }
        k.put(other);
    }

    /// The draining area `acc` as a mask into `out`, as [`flow_mask`].
    pub fn mask(&self, k: &mut Kit, acc: &wgpu::Buffer, n: usize, gamma: f32, out: &wgpu::Buffer) {
        let u = k.uniform(Uniforms::default().u32(n as u32).u32(n as u32).f32(0.0).f32(0.0).f32(0.0).u32(0).f32(rain_scale(n)).f32(gamma).u32(0));
        let bg = k.bind(&self.mask, &[(0, &u), (3, out), (7, acc)]);
        k.run(&self.mask, &bg, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Gpu;

    fn relief(n: usize) -> Vec<f32> {
        (0..n * n).map(|k| 70.0 + 40.0 * super::super::common::rand01((k % n) as u32 / 3, (k / n) as u32 / 3, 9) + (k % n) as f32 * 0.05).collect()
    }

    #[test]
    fn the_card_routes_as_the_cpu() {
        let Some(g) = Gpu::headless() else { return };
        let n = 300;
        let h = relief(n);
        let never = || false;
        let w = fill(&h, n, 62.0, 1e-3, 3, &never).unwrap();
        let rcv = receivers(&h, &w, n, 62.0, 4);
        let acc = accumulate(&rcv, None, n);
        assert_eq!(acc.iter().map(|&a| a as u64).filter(|_| true).count(), n * n);
        let mut k = Kit::new(&g);
        let fl = FlowGpu::new(&k).unwrap();
        let hb = k.buf("h", n * n);
        g.queue().write_buffer(&hb, 0, bytemuck::cast_slice(&h));
        let wb = fl.fill(&mut k, &hb, n, 62.0, 1e-3, 3, &never).unwrap();
        let (rb, ab) = (k.buf("r", n * n), k.buf("a", n * n));
        fl.receivers(&mut k, &hb, &wb, n, 62.0, 4, &rb);
        fl.accumulate(&mut k, &rb, None, n, &ab);
        k.flush();
        let got = g.download_buffers(&[&wb, &rb, &ab]);
        let wd = w.iter().zip(&got[0]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        let rd = rcv.iter().zip(&got[1]).filter(|(a, b)| **a != b.to_bits()).count();
        let ad = acc.iter().zip(&got[2]).filter(|(a, b)| **a != b.to_bits()).count();
        // The card may round a jittered slope differently at a near tie:
        // a receiver or two, and the sums below it.
        assert!(wd == 0.0 && rd <= n * n / 1000 && ad <= n * n / 100, "fill {wd}, receivers {rd}, sums {ad}");
    }
}
