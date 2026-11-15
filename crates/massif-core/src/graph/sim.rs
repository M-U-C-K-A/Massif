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
/// not depend on the number of cores. `progress` hears how much of the
/// rain has fallen (0..1), round by round.
pub(super) fn erode(f: &Field, spacing: f32, p: &Droplets, cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Option<(Field, Field, Field)> {
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
    let brush = BrushRows::new(&brush);
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
        let rains: Vec<Rain> = (0..blocks)
            .flat_map(|bz| (0..blocks).map(move |bx| (bx, bz)))
            .filter(|(bx, bz)| bx % 3 + 3 * (bz % 3) == round)
            .map(|(bx, bz)| {
                let (x0, z0) = (bx * size, bz * size);
                let (x1, z1) = ((x0 + size).min(n), (z0 + size).min(n));
                let drops = (((x1 - x0) * (z1 - z0)) as f64 * per_sample).round() as usize;
                let seed = (p.seed as u64 ^ ((bz as u64) << 32 | bx as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1;
                Rain::new(seed, (x0, z0, x1, z1), drops)
            })
            .collect();
        // Blocks of a round never meet: a thread lets the rain of two or
        // four fall a step at a time in turn (the same as one after the
        // other), when that still leaves work for every core.
        let threads = rayon::current_num_threads();
        let lanes = if rains.len() >= 8 * threads { 4 } else if rains.len() >= 4 * threads { 2 } else { 1 };
        // Safety: each block's drops stay within the block and its
        // neighbours, which no other block of this round reaches.
        match lanes {
            4 => rains.par_chunks(4).for_each(|group| unsafe { fall::<4>(shared, n, group, p, &brush, cancel) }),
            2 => rains.par_chunks(2).for_each(|group| unsafe { fall::<2>(shared, n, group, p, &brush, cancel) }),
            _ => rains.par_chunks(1).for_each(|group| unsafe { fall::<1>(shared, n, group, p, &brush, cancel) }),
        }
        progress((round + 1) as f32 / 9.0);
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

/// The erosion brush as runs of cells along x: each worn down in one go,
/// a run cut once at the edges of the map rather than every cell checked.
struct BrushRows {
    /// Offset in z, first offset in x and the weights of the run's cells.
    rows: Vec<(i32, i32, Vec<f32>)>,
}

impl BrushRows {
    /// The runs of `cells` (offset in x, offset in z, weight), in their
    /// order.
    fn new(cells: &[(i32, i32, f32)]) -> Self {
        let mut rows: Vec<(i32, i32, Vec<f32>)> = Vec::new();
        for &(dx, dz, w) in cells {
            match rows.last_mut() {
                Some((rz, rx, ws)) if *rz == dz && *rx + ws.len() as i32 == dx => ws.push(w),
                _ => rows.push((dz, dx, vec![w])),
            }
        }
        Self { rows }
    }

    /// Wear `amount` off the samples `h` (a map `n` a side) around `(i,
    /// j)`.
    ///
    /// # Safety
    /// `h` holds `n × n` samples, and no other thread touches those around
    /// `(i, j)` within the brush.
    #[inline]
    unsafe fn wear(&self, h: *mut f32, n: usize, i: usize, j: usize, amount: f32) {
        for (dz, dx0, weights) in &self.rows {
            let cj = j as i32 + dz;
            if cj < 0 || cj >= n as i32 {
                continue;
            }
            let start = i as i32 + dx0;
            let (a, b) = (start.max(0), (start + weights.len() as i32).min(n as i32));
            if a >= b {
                continue;
            }
            let weights = &weights[(a - start) as usize..(b - start) as usize];
            // Safety: within the map (cut above), touched by this thread
            // alone (the caller's promise).
            let cells = unsafe { std::slice::from_raw_parts_mut(h.add(cj as usize * n + a as usize), weights.len()) };
            for (c, &w) in cells.iter_mut().zip(weights) {
                *c -= amount * w;
            }
        }
    }
}

/// The rain of a block: the drop falling now, and those still to fall,
/// drawn from the block's own seed.
#[derive(Clone, Copy)]
struct Rain {
    rng: u64,
    /// The block's corner and size.
    x0: f32,
    z0: f32,
    w: f32,
    d: f32,
    /// Drops still to be born, and born so far.
    left: usize,
    born: usize,
    /// The drop falling now: where, which way, how fast, with how much
    /// water and soil, for how many steps.
    x: f32,
    z: f32,
    dx: f32,
    dz: f32,
    speed: f32,
    water: f32,
    sediment: f32,
    steps: usize,
    falling: bool,
}

const INERTIA: f32 = 0.05;
const GRAVITY: f32 = 4.0;
const MIN_CAPACITY: f32 = 0.01;

impl Rain {
    fn new(seed: u64, (x0, z0, x1, z1): (usize, usize, usize, usize), drops: usize) -> Self {
        Self {
            rng: seed,
            x0: x0 as f32,
            z0: z0 as f32,
            w: (x1 - x0) as f32,
            d: (z1 - z0) as f32,
            left: drops,
            born: 0,
            x: 0.0,
            z: 0.0,
            dx: 0.0,
            dz: 0.0,
            speed: 0.0,
            water: 0.0,
            sediment: 0.0,
            steps: 0,
            falling: false,
        }
    }

    /// No rain: what fills a group of blocks short of one.
    const NONE: Rain = Rain {
        rng: 1,
        x0: 0.0,
        z0: 0.0,
        w: 0.0,
        d: 0.0,
        left: 0,
        born: 0,
        x: 0.0,
        z: 0.0,
        dx: 0.0,
        dz: 0.0,
        speed: 0.0,
        water: 0.0,
        sediment: 0.0,
        steps: 0,
        falling: false,
    };

    #[inline]
    fn next(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Let the next drop fall; false when the block's rain is over.
    #[inline]
    fn spawn(&mut self, last: f32) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        self.born += 1;
        self.x = (self.x0 + self.next() * self.w).min(last - 0.001);
        self.z = (self.z0 + self.next() * self.d).min(last - 0.001);
        (self.dx, self.dz) = (0.0, 0.0);
        (self.speed, self.water, self.sediment) = (1.0, 1.0, 0.0);
        self.steps = 0;
        self.falling = true;
        true
    }

    /// One step of the falling drop (after Hans Beyer's method): it runs
    /// downhill, picks up soil where it speeds up and drops it where it
    /// slows down.
    ///
    /// # Safety
    /// As [`fall`].
    #[inline(always)]
    unsafe fn step(&mut self, b: Shared, n: usize, last: f32, p: &Droplets, brush: &BrushRows) {
        // A drop stays within `0..last` on both axes: its cell and the next
        // are in the map.
        let at = |i: usize, j: usize| unsafe { *b.h.add(j * n + i) };
        let height_grad = |x: f32, z: f32| -> (f32, f32, f32) {
            let (i, j) = (x as usize, z as usize);
            let (u, v) = (x - i as f32, z - j as f32);
            let (a, bb, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            let gx = (bb - a) * (1.0 - v) + (d - c) * v;
            let gz = (c - a) * (1.0 - u) + (d - bb) * u;
            let height = a * (1.0 - u) * (1.0 - v) + bb * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
            (height, gx, gz)
        };
        let (x, z) = (self.x, self.z);
        let (i, j) = (x as usize, z as usize);
        let (u, v) = (x - i as f32, z - j as f32);
        let (height, gx, gz) = height_grad(x, z);
        let mut dx = self.dx * INERTIA - gx * (1.0 - INERTIA);
        let mut dz = self.dz * INERTIA - gz * (1.0 - INERTIA);
        let len = dx.hypot(dz);
        if len < 1e-6 {
            self.falling = false;
            return;
        }
        dx /= len;
        dz /= len;
        let (nx, nz) = (x + dx, z + dz);
        if !(0.0..last).contains(&nx) || !(0.0..last).contains(&nz) {
            self.falling = false;
            return;
        }
        let (new_height, _, _) = height_grad(nx, nz);
        let dh = new_height - height;
        let cap = (-dh * self.speed * self.water * p.capacity).max(MIN_CAPACITY);
        let cell = j * n + i;
        unsafe {
            if self.sediment > cap || dh > 0.0 {
                // Drop soil: all it takes to fill the hole when going up,
                // part of the excess otherwise.
                let amount = if dh > 0.0 { dh.min(self.sediment) } else { (self.sediment - cap) * p.deposition };
                self.sediment -= amount;
                *b.h.add(cell) += amount * (1.0 - u) * (1.0 - v);
                *b.h.add(cell + 1) += amount * u * (1.0 - v);
                *b.h.add(cell + n) += amount * (1.0 - u) * v;
                *b.h.add(cell + n + 1) += amount * u * v;
                *b.settled.add(cell) += amount;
            } else {
                let amount = ((cap - self.sediment) * p.erosion).min(-dh);
                brush.wear(b.h, n, i, j, amount);
                self.sediment += amount;
            }
            *b.flow.add(cell) += self.water;
        }
        self.speed = (self.speed * self.speed + dh * GRAVITY).max(0.0).sqrt();
        self.water *= 1.0 - p.evaporation;
        (self.x, self.z, self.dx, self.dz) = (nx, nz, dx, dz);
        self.steps += 1;
        if self.steps == MAX_STEPS {
            self.falling = false;
        }
    }
}

/// Let the rain of the blocks of `group` (at most `L`) fall, their drops a
/// step at a time in turn: the processor works on one drop's step while it
/// waits on another's, and as the blocks never meet, each falls as it would
/// alone.
///
/// # Safety
/// No other thread may touch the samples within `MAX_STEPS` plus the brush
/// radius of these blocks while this runs.
unsafe fn fall<const L: usize>(b: Shared, n: usize, group: &[Rain], p: &Droplets, brush: &BrushRows, cancel: &AtomicBool) {
    let mut rains = [Rain::NONE; L];
    rains[..group.len()].copy_from_slice(group);
    let last = (n - 1) as f32;
    loop {
        let mut any = false;
        for rain in &mut rains {
            if !rain.falling {
                if rain.born % 16384 == 0 && rain.left > 0 && cancel.load(Ordering::Relaxed) {
                    return;
                }
                if !rain.spawn(last) {
                    continue;
                }
            }
            any = true;
            unsafe { rain.step(b, n, last, p, brush) };
        }
        if !any {
            return;
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
