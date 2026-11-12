//! Simulations: water erosion, thermal erosion, where water flows and
//! gathers, rivers and lakes.

use super::Field;
use rayon::prelude::*;
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Droplets {
    pub drops: usize,
    pub erosion: f32,
    pub deposition: f32,
    pub capacity: f32,
    pub evaporation: f32,
    pub radius: i32,
    pub seed: u32,
}

/// Steps a raindrop lives at most.
const MAX_STEPS: usize = 80;

/// The buffers raindrops write to, shared by blocks of the map far enough
/// apart that no two drops of theirs ever reach the same sample.
#[derive(Clone, Copy)]
struct Shared {
    h: *mut f32,
    flow: *mut f32,
    settled: *mut f32,
}

// Safety: blocks running at the same time touch disjoint parts of the
// buffers (see `erode`).
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// Hydraulic erosion by simulated raindrops (after Hans Beyer's method):
/// each drop runs downhill, picks up soil where it speeds up and drops it
/// where it slows down. Returns the eroded heights, the flow of water
/// (0..1) and where soil settled (0..1).
///
/// The map is cut into blocks wider than a drop can travel: a drop born in
/// a block stays within that block and its eight neighbours, so blocks
/// three apart never meet, and run side by side on every core, in nine
/// rounds. Each block draws its drops from its own seed: the result does
/// not depend on the number of cores.
pub(super) fn erode(f: &Field, spacing: f32, p: &Droplets, cancel: &AtomicBool) -> Option<(Field, Field, Field)> {
    let n = f.n;
    // Work in units of samples so that slopes are true whatever the
    // resolution.
    let mut h: Vec<f32> = f.data.par_iter().map(|v| v / spacing).collect();
    let mut flow = vec![0f32; n * n];
    let mut settled = vec![0f32; n * n];
    // The erosion brush.
    let r = p.radius.max(1);
    let mut brush: Vec<(i32, i32, f32)> = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            let d = ((dx * dx + dz * dz) as f32).sqrt();
            if d <= r as f32 {
                brush.push((dx, dz, 1.0 - d / (r as f32 + 1.0)));
            }
        }
    }
    let total: f32 = brush.iter().map(|b| b.2).sum();
    for b in &mut brush {
        b.2 /= total;
    }
    let size = MAX_STEPS + 2 * r as usize + 4;
    let blocks = n.div_ceil(size);
    let shared = Shared {
        h: h.as_mut_ptr(),
        flow: flow.as_mut_ptr(),
        settled: settled.as_mut_ptr(),
    };
    let per_sample = p.drops as f64 / (n * n) as f64;
    for round in 0..9 {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let list: Vec<(usize, usize)> = (0..blocks)
            .flat_map(|bz| (0..blocks).map(move |bx| (bx, bz)))
            .filter(|(bx, bz)| bx % 3 + 3 * (bz % 3) == round)
            .collect();
        list.par_iter().for_each(|&(bx, bz)| {
            let (x0, z0) = (bx * size, bz * size);
            let (x1, z1) = ((x0 + size).min(n), (z0 + size).min(n));
            let drops = (((x1 - x0) * (z1 - z0)) as f64 * per_sample).round() as usize;
            let seed = (p.seed as u64 ^ ((bz as u64) << 32 | bx as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1;
            // Safety: this block's drops stay within the block and its
            // neighbours, which no other block of this round reaches.
            unsafe { drops_in(shared, n, (x0, z0, x1, z1), drops, seed, p, &brush, cancel) };
        });
    }
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let normalise = |v: Vec<f32>| {
        let max = v.par_iter().copied().reduce(|| 0.0f32, f32::max).max(1e-6);
        let l = (1.0 + max).ln();
        Field::new(n, v.into_par_iter().map(|x| (1.0 + x.max(0.0)).ln() / l).collect())
    };
    let heights = Field::new(n, h.into_par_iter().map(|v| v * spacing).collect());
    Some((heights, normalise(flow), normalise(settled)))
}

/// Run `drops` raindrops born in the block `x0..x1` × `z0..z1`.
///
/// # Safety
/// No other thread may touch the samples within `MAX_STEPS` plus the brush
/// radius of the block while this runs.
#[allow(clippy::too_many_arguments)]
unsafe fn drops_in(b: Shared, n: usize, (x0, z0, x1, z1): (usize, usize, usize, usize), drops: usize, seed: u64, p: &Droplets, brush: &[(i32, i32, f32)], cancel: &AtomicBool) {
    let inertia = 0.05f32;
    let gravity = 4.0f32;
    let min_capacity = 0.01f32;
    let mut rng = seed;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    let last = (n - 1) as f32;
    let at = |i: usize, j: usize| unsafe { *b.h.add(j.min(n - 1) * n + i.min(n - 1)) };
    let height_grad = |x: f32, z: f32| -> (f32, f32, f32) {
        let (i, j) = (x as usize, z as usize);
        let (u, v) = (x - i as f32, z - j as f32);
        let (a, bb, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        let gx = (bb - a) * (1.0 - v) + (d - c) * v;
        let gz = (c - a) * (1.0 - u) + (d - bb) * u;
        let height = a * (1.0 - u) * (1.0 - v) + bb * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
        (height, gx, gz)
    };
    let (w, d) = ((x1 - x0) as f32, (z1 - z0) as f32);
    for drop in 0..drops {
        if drop % 16384 == 0 && cancel.load(Ordering::Relaxed) {
            return;
        }
        let (mut x, mut z) = ((x0 as f32 + next() * w).min(last - 0.001), (z0 as f32 + next() * d).min(last - 0.001));
        let (mut dx, mut dz) = (0f32, 0f32);
        let (mut speed, mut water, mut sediment) = (1f32, 1f32, 0f32);
        for _ in 0..MAX_STEPS {
            let (i, j) = (x as usize, z as usize);
            let (u, v) = (x - i as f32, z - j as f32);
            let (height, gx, gz) = height_grad(x, z);
            dx = dx * inertia - gx * (1.0 - inertia);
            dz = dz * inertia - gz * (1.0 - inertia);
            let len = dx.hypot(dz);
            if len < 1e-6 {
                break;
            }
            dx /= len;
            dz /= len;
            x += dx;
            z += dz;
            if !(0.0..last).contains(&x) || !(0.0..last).contains(&z) {
                break;
            }
            let (new_height, _, _) = height_grad(x, z);
            let dh = new_height - height;
            let cap = (-dh * speed * water * p.capacity).max(min_capacity);
            let cell = j * n + i;
            unsafe {
                if sediment > cap || dh > 0.0 {
                    // Drop soil: all it takes to fill the hole when going
                    // up, part of the excess otherwise.
                    let amount = if dh > 0.0 { dh.min(sediment) } else { (sediment - cap) * p.deposition };
                    sediment -= amount;
                    *b.h.add(cell) += amount * (1.0 - u) * (1.0 - v);
                    *b.h.add(cell + 1) += amount * u * (1.0 - v);
                    *b.h.add(cell + n) += amount * (1.0 - u) * v;
                    *b.h.add(cell + n + 1) += amount * u * v;
                    *b.settled.add(cell) += amount;
                } else {
                    let amount = ((cap - sediment) * p.erosion).min(-dh);
                    for &(bx, bz, wt) in brush {
                        let (ci, cj) = (i as i32 + bx, j as i32 + bz);
                        if ci < 0 || cj < 0 || ci >= n as i32 || cj >= n as i32 {
                            continue;
                        }
                        *b.h.add(cj as usize * n + ci as usize) -= amount * wt;
                    }
                    sediment += amount;
                }
                *b.flow.add(cell) += water;
            }
            speed = (speed * speed + dh * gravity).max(0.0).sqrt();
            water *= 1.0 - p.evaporation;
        }
    }
}

/// Thermal erosion: wherever two neighbours differ by more than the talus
/// slope allows, material slides from the higher to the lower.
pub(super) fn thermal(f: &Field, spacing: f32, talus: f32, iterations: u32, strength: f32) -> Field {
    let n = f.n;
    let limit = talus.clamp(1.0, 89.0).to_radians().tan() * spacing;
    let k = strength * 0.25;
    let mut h = f.data.clone();
    for _ in 0..iterations {
        let src = &h;
        let next: Vec<f32> = (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (i, j) = (c % n, c / n);
                let mine = src[c];
                let mut change = 0.0;
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ni, nj) = (i as i32 + di, j as i32 + dj);
                    if ni < 0 || nj < 0 || ni >= n as i32 || nj >= n as i32 {
                        continue;
                    }
                    let other = src[nj as usize * n + ni as usize];
                    let d = mine - other;
                    if d > limit {
                        change -= k * (d - limit);
                    } else if -d > limit {
                        change += k * (-d - limit);
                    }
                }
                mine + change
            })
            .collect();
        h = next;
    }
    Field::new(n, h)
}


/// A cell of the priority queue of [`fill`]: lowest first.
#[derive(PartialEq)]
struct Low(f32, usize);

impl Eq for Low {}

impl PartialOrd for Low {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for Low {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

/// Fill every hollow up to its spill point, flooding from the edges of the
/// map inward, lowest first ("priority flood"). With `epsilon` > 0 the
/// filled hollows keep a tiny slope toward their outlet, so that water can
/// still find its way across them.
pub(super) fn fill(f: &Field, epsilon: f32) -> Vec<f32> {
    let n = f.n;
    let mut h = f.data.clone();
    let mut done = vec![false; n * n];
    let mut queue = BinaryHeap::with_capacity(4 * n);
    for c in 0..n * n {
        let (i, j) = (c % n, c / n);
        if i == 0 || j == 0 || i == n - 1 || j == n - 1 {
            done[c] = true;
            queue.push(Low(h[c], c));
        }
    }
    while let Some(Low(level, c)) = queue.pop() {
        let (i, j) = ((c % n) as i64, (c / n) as i64);
        for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= n as i64 || nj >= n as i64 {
                continue;
            }
            let k = nj as usize * n + ni as usize;
            if done[k] {
                continue;
            }
            done[k] = true;
            if h[k] <= level + epsilon {
                h[k] = level + epsilon;
            }
            queue.push(Low(h[k], k));
        }
    }
    h
}

/// How much water runs through every cell when it rains one unit on every
/// cell and each sends its water to its lowest neighbour (D8).
pub(super) fn accumulate(h: &[f32], n: usize) -> Vec<f32> {
    let mut order: Vec<usize> = (0..n * n).collect();
    order.par_sort_unstable_by(|&a, &b| h[b].total_cmp(&h[a]));
    let mut acc = vec![1.0f32; n * n];
    for c in order {
        let (i, j) = ((c % n) as i64, (c / n) as i64);
        let mut best = None;
        let mut drop = 0.0f32;
        for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (ni, nj) = (i + di, j + dj);
            if ni < 0 || nj < 0 || ni >= n as i64 || nj >= n as i64 {
                continue;
            }
            let k = nj as usize * n + ni as usize;
            let d = (h[c] - h[k]) / if di != 0 && dj != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            if d > drop {
                drop = d;
                best = Some(k);
            }
        }
        if let Some(k) = best {
            acc[k] += acc[c];
        }
    }
    acc
}
