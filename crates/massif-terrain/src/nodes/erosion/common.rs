//! What the erosion nodes share: the eight neighbours, a random number
//! per sample, coarser and finer copies of a grid (on the CPU and the
//! graphics card), and [`Kit`], which runs passes on the card.
//!
//! Grids are square, `n` samples a side, row by row. A coarser grid has
//! `n.div_ceil(2)` samples a side, its sample `I` lying on the finer one's
//! `2I`, as levels of detail do.

use crate::gpu::Gpu;
use crate::kernel::Uniforms;
use rayon::prelude::*;
use std::f32::consts::SQRT_2;

/// No neighbour (a sample whose water leaves the terrain, or stops).
pub(super) const NONE: u32 = u32::MAX;

/// The eight neighbours, in the order every kernel visits them (the WGSL
/// prelude's `OFF`).
pub(super) const NB: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// How far each neighbour is, in samples.
pub(super) const DIST: [f32; 8] = [SQRT_2, 1.0, SQRT_2, 1.0, 1.0, SQRT_2, 1.0, SQRT_2];

/// A number from 0 to 1 for sample `(x, z)` and `seed`, the same on the
/// graphics card (`rand01` of the prelude).
#[inline]
pub(super) fn rand01(x: u32, z: u32, seed: u32) -> f32 {
    (crate::noise::hash2(x, z, seed) >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Smooth noise from about −1 to 1 at `(x, z)`, in its own cells (one
/// wave a cell or so), the same as `wnoise` of the prelude.
#[inline]
pub(super) fn wnoise(x: f32, z: f32, seed: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    crate::noise::gradient(fx as i32 as u32, fz as i32 as u32, x - fx, z - fz, seed)
}

/// `t` from 0 to 1, eased (smoothstep's curve).
#[inline]
pub(super) fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Samples a side of the grid one level coarser.
#[inline]
pub(super) fn coarser(n: usize) -> usize {
    n.div_ceil(2)
}

/// Neighbour `d` of sample `k` (`(x, z)`), when inside the grid.
#[inline]
pub(super) fn neighbour(x: usize, z: usize, n: usize, d: usize) -> Option<usize> {
    let (nx, nz) = (x as i32 + NB[d].0, z as i32 + NB[d].1);
    (nx >= 0 && nz >= 0 && nx < n as i32 && nz < n as i32).then(|| nz as usize * n + nx as usize)
}

/// A grid one level coarser, each sample the 1-2-1 weighted mean of the
/// nine around its place (`down_avg` on the card).
pub(super) fn down_avg(h: &[f32], n: usize) -> Vec<f32> {
    let m = coarser(n);
    (0..m * m)
        .into_par_iter()
        .map(|k| {
            let (fx, fz) = (2 * (k % m) as i32, 2 * (k / m) as i32);
            let (mut s, mut w) = (0.0f32, 0.0f32);
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let (x, z) = (fx + dx, fz + dz);
                    if x >= 0 && z >= 0 && x < n as i32 && z < n as i32 {
                        let wt = ((2 - dx.abs()) * (2 - dz.abs())) as f32;
                        s += wt * h[z as usize * n + x as usize];
                        w += wt;
                    }
                }
            }
            s / w
        })
        .collect()
}

/// A grid one level coarser, each sample the highest of the nine around
/// its place (`down_max` on the card).
pub(super) fn down_max(h: &[f32], n: usize) -> Vec<f32> {
    let m = coarser(n);
    (0..m * m)
        .into_par_iter()
        .map(|k| {
            let (fx, fz) = (2 * (k % m) as i32, 2 * (k / m) as i32);
            let mut s = f32::MIN;
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let (x, z) = (fx + dx, fz + dz);
                    if x >= 0 && z >= 0 && x < n as i32 && z < n as i32 {
                        s = s.max(h[z as usize * n + x as usize]);
                    }
                }
            }
            s
        })
        .collect()
}

/// Coarse grid `c` (`m` a side) at fine sample `(x, z)`, bilinear.
#[inline]
pub(super) fn up_at(c: &[f32], m: usize, x: usize, z: usize) -> f32 {
    let (x0, z0) = ((x / 2).min(m - 1), (z / 2).min(m - 1));
    let (x1, z1) = ((x0 + 1).min(m - 1), (z0 + 1).min(m - 1));
    let (tx, tz) = (if x % 2 == 1 { 0.5f32 } else { 0.0 }, if z % 2 == 1 { 0.5f32 } else { 0.0 });
    let a = c[z0 * m + x0] + (c[z0 * m + x1] - c[z0 * m + x0]) * tx;
    let b = c[z1 * m + x0] + (c[z1 * m + x1] - c[z1 * m + x0]) * tx;
    a + (b - a) * tz
}

/// `base` (`n` a side) plus the coarse grid `c` brought to it, bilinear
/// (`up_add` on the card); `base` absent, `c` alone.
pub(super) fn up_add(c: &[f32], base: Option<&[f32]>, n: usize) -> Vec<f32> {
    let m = coarser(n);
    (0..n * n).into_par_iter().map(|k| up_at(c, m, k % n, k / n) + base.map_or(0.0, |b| b[k])).collect()
}

/// Runs passes on the graphics card: dispatches recorded into one
/// command encoder, submitted every few dozen, and small counters read
/// back.
pub(super) struct Kit<'a> {
    pub g: &'a Gpu,
    enc: Option<wgpu::CommandEncoder>,
    queued: u32,
    /// Scratch buffers given back, for the next pass that needs one.
    pool: std::cell::RefCell<Vec<wgpu::Buffer>>,
    /// Uniform blocks made, by their bytes.
    uniforms: std::cell::RefCell<std::collections::HashMap<Vec<u8>, wgpu::Buffer>>,
}

impl<'a> Kit<'a> {
    pub fn new(g: &'a Gpu) -> Self {
        Self { g, enc: None, queued: 0, pool: Default::default(), uniforms: Default::default() }
    }

    /// A scratch buffer of `cells` 4-byte values, not cleared: for passes
    /// that write all of it before they read it. Give it back with
    /// [`Kit::put`].
    pub fn tmp(&self, cells: usize) -> wgpu::Buffer {
        let size = (cells as u64 * 4).max(16);
        let mut pool = self.pool.borrow_mut();
        match pool.iter().position(|b| b.size() == size) {
            Some(i) => pool.swap_remove(i),
            None => self.g.buffer("erosion scratch", size),
        }
    }

    pub fn put(&self, b: wgpu::Buffer) {
        self.pool.borrow_mut().push(b);
    }

    /// Entry `entry` of `source` (which starts with `prelude.wgsl`),
    /// compiled once under `name`.
    pub fn pipe(&self, name: &'static str, source: &'static str, entry: &'static str) -> Option<wgpu::ComputePipeline> {
        self.g.compute(name, source, entry)
    }

    /// A zeroed storage buffer of `cells` 4-byte values.
    pub fn buf(&self, label: &str, cells: usize) -> wgpu::Buffer {
        self.g.buffer(label, cells as u64 * 4)
    }

    /// A uniform block (the same bytes give the same buffer).
    pub fn uniform(&self, u: &mut Uniforms) -> wgpu::Buffer {
        let bytes = u.finish();
        self.uniforms.borrow_mut().entry(bytes).or_insert_with_key(|b| self.g.uniform("erosion", b)).clone()
    }

    /// The bind group of `p` with each buffer at its binding. Only the
    /// bindings the entry point uses may be given (layouts are derived
    /// from the shader).
    pub fn bind(&self, p: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry> = entries.iter().map(|(b, buf)| wgpu::BindGroupEntry { binding: *b, resource: buf.as_entire_binding() }).collect();
        self.g.device().create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &p.get_bind_group_layout(0), entries: &entries })
    }

    fn enc(&mut self) -> &mut wgpu::CommandEncoder {
        self.enc.get_or_insert_with(|| self.g.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("erosion") }))
    }

    /// Runs `p` over an `n` × `n` grid (16 × 16 workgroups).
    pub fn run(&mut self, p: &wgpu::ComputePipeline, bg: &wgpu::BindGroup, n: usize) {
        let g = (n as u32).div_ceil(16);
        self.groups(p, bg, g, g);
    }

    pub fn groups(&mut self, p: &wgpu::ComputePipeline, bg: &wgpu::BindGroup, x: u32, y: u32) {
        {
            let mut pass = self.enc().begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("erosion"), timestamp_writes: None });
            pass.set_pipeline(p);
            pass.set_bind_group(0, bg, &[]);
            pass.dispatch_workgroups(x, y, 1);
        }
        self.queued += 1;
        if self.queued >= 48 {
            self.flush();
        }
    }

    pub fn copy(&mut self, from: &wgpu::Buffer, to: &wgpu::Buffer, bytes: u64) {
        self.enc().copy_buffer_to_buffer(from, 0, to, 0, bytes);
    }

    pub fn clear(&mut self, b: &wgpu::Buffer) {
        self.enc().clear_buffer(b, 0, None);
    }

    /// Submits what is recorded.
    pub fn flush(&mut self) {
        if let Some(enc) = self.enc.take() {
            self.g.queue().submit([enc.finish()]);
        }
        self.queued = 0;
    }

    /// The first value of `b`, as a whole number, once what is recorded
    /// has run.
    pub fn read_u32(&mut self, b: &wgpu::Buffer) -> u32 {
        self.flush();
        self.g.download_buffers(&[b]).first().and_then(|v| v.first()).map_or(0, |f| f.to_bits())
    }

    /// Waits for what is recorded (for timings).
    pub fn finish(&mut self) {
        self.flush();
        self.g.wait();
    }
}

/// The resampling kernels (`resample.wgsl`): coarser and finer grids on
/// the graphics card, as [`down_avg`], [`down_max`] and [`up_add`] make
/// them on the CPU.
pub(super) struct Resample {
    down_avg: wgpu::ComputePipeline,
    down_max: wgpu::ComputePipeline,
    up_add: wgpu::ComputePipeline,
}

const RESAMPLE_WGSL: &str = concat!(include_str!("prelude.wgsl"), include_str!("resample.wgsl"));

impl Resample {
    pub fn new(k: &Kit) -> Option<Self> {
        let p = |e| k.pipe("erosion.resample", RESAMPLE_WGSL, e);
        Some(Self { down_avg: p("down_avg")?, down_max: p("down_max")?, up_add: p("up_add")? })
    }

    fn sizes(k: &Kit, n: usize) -> wgpu::Buffer {
        k.uniform(Uniforms::default().u32(n as u32).u32(coarser(n) as u32))
    }

    /// `src` (`n` a side) one level coarser, into a scratch buffer.
    pub fn down(&self, k: &mut Kit, src: &wgpu::Buffer, n: usize, max: bool) -> wgpu::Buffer {
        let m = coarser(n);
        let dst = k.tmp(m * m);
        let u = Self::sizes(k, n);
        let p = if max { &self.down_max } else { &self.down_avg };
        let bg = k.bind(p, &[(0, &u), (1, src), (3, &dst)]);
        k.run(p, &bg, m);
        dst
    }

    /// The coarse `src` brought to `n` a side, plus `base` (when given).
    pub fn up(&self, k: &mut Kit, src: &wgpu::Buffer, base: Option<&wgpu::Buffer>, n: usize, dst: &wgpu::Buffer) {
        let u = k.uniform(Uniforms::default().u32(n as u32).u32(coarser(n) as u32).u32(base.is_some() as u32));
        let zero;
        let base = match base {
            Some(b) => b,
            None => {
                zero = k.buf("erosion zero", 4);
                &zero
            }
        };
        let bg = k.bind(&self.up_add, &[(0, &u), (1, src), (2, base), (3, dst)]);
        k.run(&self.up_add, &bg, n);
    }
}
