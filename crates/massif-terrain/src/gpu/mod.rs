//! The graphics card: tiles and grids in its memory, kernels as compute
//! shaders.
//!
//! A [`Gpu`] is made on a device of its own ([`Gpu::headless`], for tests
//! and the command line) or on the app's ([`Gpu::from_device`]), so that
//! the tiles the graph makes are there for the views that draw them. Tile
//! buffers come from a pool and go back to it when a tile is dropped.
//! Kernels are compiled once, in an error scope: a kernel the device
//! refuses is remembered, and its nodes computed on the CPU instead.
//!
//! Point and local kernels are run by the engine ([`Gpu::run_tiles`]).
//! Simulations write their own passes with [`Gpu::compute`], [`GpuGrid`]
//! and the device and queue, as [`crate::kernel::GlobalGpuFn`] says.

use crate::kernel::Grid;
use crate::space::{Rect, TILE};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Bytes of a tile of `f32`.
pub const TILE_BYTES: u64 = (TILE * TILE * 4) as u64;

/// Tile buffers kept for reuse at most, per size.
const POOL_MAX: usize = 1024;

/// What a storage buffer the terrain makes can be used for.
pub const STORAGE: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC).union(wgpu::BufferUsages::COPY_DST);

/// Free tile buffers, by size.
type Pool = Arc<Mutex<HashMap<u64, Vec<wgpu::Buffer>>>>;

/// A tile in the graphics card's memory: planes of TILE² samples.
pub struct GpuTile {
    buffer: Option<wgpu::Buffer>,
    pool: Pool,
}

impl GpuTile {
    pub fn buffer(&self) -> &wgpu::Buffer {
        self.buffer.as_ref().expect("a tile has its buffer until dropped")
    }
}

impl Drop for GpuTile {
    fn drop(&mut self) {
        if let Some(b) = self.buffer.take()
            && let Ok(mut pool) = self.pool.lock()
        {
            let free = pool.entry(b.size()).or_default();
            if free.len() < POOL_MAX {
                free.push(b);
            }
        }
    }
}

impl std::fmt::Debug for GpuTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GpuTile")
    }
}

/// A grid in the graphics card's memory (a simulation's input or output
/// over the whole terrain): `planes` planes of `rect.area()` samples, row
/// by row, as [`Grid`] holds them.
#[derive(Debug)]
pub struct GpuGrid {
    pub lod: u8,
    pub rect: Rect,
    pub planes: usize,
    pub buffer: wgpu::Buffer,
}

impl GpuGrid {
    /// Samples a side (a simulation's grids are square).
    pub fn side(&self) -> u32 {
        self.rect.w
    }
}

/// One dispatch of a point or local kernel over a tile: its uniform block,
/// then the buffers bound after it (`None` binds a stand-in).
pub type Job<'a> = (Vec<u8>, Vec<Option<&'a wgpu::Buffer>>);

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    pipelines: Mutex<HashMap<(&'static str, &'static str), wgpu::ComputePipeline>>,
    /// Kernels the device refused, by name.
    refused: Mutex<HashSet<&'static str>>,
    pool: Pool,
    /// Bound for inputs left unconnected.
    stand_in: wgpu::Buffer,
    /// Uniform blocks are this many bytes apart.
    align: u64,
}

impl std::fmt::Debug for Gpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gpu({})", self.name)
    }
}

impl Gpu {
    /// A device of its own on the best graphics card there is (the one
    /// `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` name, when set), with all the
    /// card allows (simulations bind grids of 256 MB and more).
    pub fn headless() -> Option<Arc<Self>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).ok()?;
        let info = adapter.get_info();
        let desc = wgpu::DeviceDescriptor { label: Some("massif-terrain"), required_limits: adapter.limits(), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc)).ok()?;
        device.on_uncaptured_error(Arc::new(|e| log::error!("terrain GPU: {e}")));
        Some(Self::from_device(device, queue, format!("{} ({:?})", info.name, info.backend)))
    }

    /// On a device made elsewhere (the app's). Its limits bound the grids
    /// simulations can bind: the app should ask its adapter for
    /// `max_storage_buffer_binding_size` and `max_buffer_size`.
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, name: String) -> Arc<Self> {
        let stand_in = device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain stand-in"), size: 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let align = device.limits().min_uniform_buffer_offset_alignment.max(16) as u64;
        Arc::new(Self { device, queue, name, pipelines: Mutex::default(), refused: Mutex::default(), pool: Pool::default(), stand_in, align })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Whether the device took the kernel named `name` (or has not been
    /// asked yet).
    pub fn takes(&self, name: &str) -> bool {
        !self.refused.lock().expect("lock").contains(name)
    }

    /// Marks a kernel as refused: its nodes run on the CPU from now on.
    pub fn refuse(&self, name: &'static str, why: impl std::fmt::Display) {
        log::warn!("terrain GPU: {name} runs on the CPU: {why}");
        self.refused.lock().expect("lock").insert(name);
    }

    /// Entry point `entry` of the WGSL `source` (with
    /// [`crate::nodes::WGSL_COMMON`] put before it), compiled once and kept
    /// under `name`; `None` when the device refuses it, which is then
    /// remembered.
    pub fn compute(&self, name: &'static str, source: &str, entry: &'static str) -> Option<wgpu::ComputePipeline> {
        if !self.takes(name) {
            return None;
        }
        if let Some(p) = self.pipelines.lock().expect("lock").get(&(name, entry)) {
            return Some(p.clone());
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let source = format!("{}\n{}", crate::nodes::WGSL_COMMON, source);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(name), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(name),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            self.refuse(name, e);
            return None;
        }
        self.pipelines.lock().expect("lock").insert((name, entry), pipeline.clone());
        Some(pipeline)
    }

    /// A storage buffer of `size` bytes, zeroed.
    pub fn buffer(&self, label: &str, size: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage: STORAGE, mapped_at_creation: false })
    }

    /// A uniform buffer holding `bytes`.
    pub fn uniform(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let b = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (bytes.len() as u64).max(16).next_multiple_of(16),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&b, 0, bytes);
        b
    }

    /// A grid of `planes` planes over `rect` at `lod`, zeroed.
    pub fn grid(&self, lod: u8, rect: Rect, planes: usize) -> GpuGrid {
        GpuGrid { lod, rect, planes, buffer: self.buffer("terrain grid", rect.area() as u64 * planes as u64 * 4) }
    }

    /// `grid` in the card's memory.
    pub fn upload_grid(&self, grid: &Grid) -> GpuGrid {
        let g = self.grid(grid.lod, grid.rect, grid.planes);
        self.queue.write_buffer(&g.buffer, 0, bytemuck::cast_slice(&grid.data));
        g
    }

    /// A grid read back.
    pub fn download_grid(&self, grid: &GpuGrid) -> Grid {
        let data = self.download_buffers(&[&grid.buffer]).pop().unwrap_or_default();
        let n = grid.rect.area() * grid.planes;
        Grid::new(grid.lod, grid.rect, data.into_iter().take(n).collect())
    }

    /// A tile buffer of `planes` planes.
    fn tile(&self, planes: usize) -> GpuTile {
        let size = TILE_BYTES * planes.max(1) as u64;
        let reused = self.pool.lock().expect("lock").get_mut(&size).and_then(Vec::pop);
        let buffer = reused.unwrap_or_else(|| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain tile"), size, usage: STORAGE, mapped_at_creation: false }));
        GpuTile { buffer: Some(buffer), pool: self.pool.clone() }
    }

    /// A tile of `data` (planes of TILE² samples) in the card's memory.
    pub fn upload(&self, data: &[f32]) -> GpuTile {
        let planes = (data.len() as u64 * 4 / TILE_BYTES) as usize;
        let t = self.tile(planes);
        self.queue.write_buffer(t.buffer(), 0, bytemuck::cast_slice(data));
        t
    }

    /// The samples of tiles, read back.
    pub fn download(&self, tiles: &[&GpuTile]) -> Vec<Vec<f32>> {
        self.download_buffers(&tiles.iter().map(|t| t.buffer()).collect::<Vec<_>>())
    }

    /// The contents of buffers, as floats, read back about 16 MB a trip.
    pub fn download_buffers(&self, buffers: &[&wgpu::Buffer]) -> Vec<Vec<f32>> {
        let mut out = Vec::with_capacity(buffers.len());
        let mut start = 0;
        while start < buffers.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < buffers.len() && (end == start || bytes + buffers[end].size() <= 64 * TILE_BYTES) {
                bytes += buffers[end].size();
                end += 1;
            }
            let chunk = &buffers[start..end];
            start = end;
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain readback"), size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("terrain readback") });
            let mut at = 0;
            for b in chunk {
                enc.copy_buffer_to_buffer(b, 0, &staging, at, b.size());
                at += b.size();
            }
            self.queue.submit([enc.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging.map_async(wgpu::MapMode::Read, .., move |r| {
                let _ = tx.send(r);
            });
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            match rx.recv() {
                Ok(Ok(())) => {
                    let view = staging.get_mapped_range(..).expect("mapped");
                    let all: &[f32] = bytemuck::cast_slice(&view);
                    let mut at = 0;
                    for b in chunk {
                        let n = (b.size() / 4) as usize;
                        out.push(all[at..at + n].to_vec());
                        at += n;
                    }
                    drop(view);
                    staging.unmap();
                }
                _ => {
                    log::error!("terrain GPU: a readback failed");
                    out.extend(chunk.iter().map(|b| vec![0.0; (b.size() / 4) as usize]));
                }
            }
        }
        out
    }

    /// Runs the point or local kernel `name` (entry `main` of `source`)
    /// over tiles: for each, its uniform block and the buffers bound after
    /// it; `outputs` gives each output's planes. Gives each tile's outputs,
    /// or `None` when the device refuses the kernel.
    pub fn run_tiles(&self, name: &'static str, source: &str, jobs: &[Job], outputs: &[usize]) -> Option<Vec<Vec<GpuTile>>> {
        if jobs.is_empty() {
            return Some(Vec::new());
        }
        let pipeline = self.compute(name, source, "main")?;
        let layout = pipeline.get_bind_group_layout(0);
        let block = jobs.iter().map(|j| j.0.len() as u64).max().unwrap_or(16).max(16);
        let slot = block.div_ceil(self.align) * self.align;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut bytes = vec![0u8; (slot * jobs.len() as u64) as usize];
        for (k, (u, _)) in jobs.iter().enumerate() {
            bytes[k * slot as usize..k * slot as usize + u.len()].copy_from_slice(u);
        }
        let uniforms = self.uniform(name, &bytes);
        let made: Vec<Vec<GpuTile>> = jobs.iter().map(|_| outputs.iter().map(|&p| self.tile(p)).collect()).collect();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(name) });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            for (k, ((_, bound), outs)) in jobs.iter().zip(&made).enumerate() {
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniforms, offset: slot * k as u64, size: wgpu::BufferSize::new(block) }),
                }];
                for (i, b) in bound.iter().enumerate() {
                    entries.push(wgpu::BindGroupEntry { binding: 1 + i as u32, resource: b.unwrap_or(&self.stand_in).as_entire_binding() });
                }
                for (o, t) in outs.iter().enumerate() {
                    entries.push(wgpu::BindGroupEntry { binding: 1 + (bound.len() + o) as u32, resource: t.buffer().as_entire_binding() });
                }
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries });
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(TILE / 16, TILE / 16, 1);
            }
        }
        self.queue.submit([enc.finish()]);
        if let Some(e) = pollster::block_on(scope.pop()) {
            self.refuse(name, e);
            return None;
        }
        Some(made)
    }

    /// Waits for what was submitted (for timings).
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}
