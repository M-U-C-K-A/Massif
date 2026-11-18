//! The graphics card: tiles in its memory, kernels as compute shaders.
//!
//! A [`Gpu`] is made on a device of its own ([`Gpu::headless`], for tests
//! and the command line) or on the app's ([`Gpu::from_device`]), so that
//! the tiles the graph makes are there for the views that draw them. Tile
//! buffers come from a pool and go back to it when a tile is dropped.
//! Every wave of dispatches runs in an error scope: a kernel the device
//! refuses is turned off and its nodes computed on the CPU instead.

use crate::kernel::GpuKernel;
use crate::space::TILE;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Bytes of a tile of `f32`.
pub const TILE_BYTES: u64 = (TILE * TILE * 4) as u64;

/// Tile buffers kept for reuse at most (256 MB).
const POOL_MAX: usize = 1024;

/// Free tile buffers, by size.
type Pool = Arc<Mutex<HashMap<u64, Vec<wgpu::Buffer>>>>;

/// A tile in the graphics card's memory.
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

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
    pipelines: Mutex<HashMap<&'static str, wgpu::ComputePipeline>>,
    /// Kernels the device refused.
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
    /// `WGPU_BACKEND` and `WGPU_ADAPTER_NAME` name, when set).
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

    /// On a device made elsewhere (the app's).
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

    /// Whether the device took `kernel` (or has not been asked yet).
    pub fn takes(&self, kernel: &GpuKernel) -> bool {
        !self.refused.lock().expect("lock").contains(kernel.name)
    }

    fn refuse(&self, kernel: &GpuKernel, why: impl std::fmt::Display) {
        log::warn!("terrain GPU: {} runs on the CPU: {why}", kernel.name);
        self.refused.lock().expect("lock").insert(kernel.name);
    }

    fn pipeline(&self, kernel: &GpuKernel) -> Option<wgpu::ComputePipeline> {
        if let Some(p) = self.pipelines.lock().expect("lock").get(kernel.name) {
            return Some(p.clone());
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let source = format!("{}\n{}", crate::nodes::WGSL_COMMON, kernel.source);
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(kernel.name), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(kernel.name),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            self.refuse(kernel, e);
            return None;
        }
        self.pipelines.lock().expect("lock").insert(kernel.name, pipeline.clone());
        Some(pipeline)
    }

    /// A tile buffer of `planes` planes.
    fn tile(&self, planes: usize) -> GpuTile {
        let size = TILE_BYTES * planes.max(1) as u64;
        let reused = self.pool.lock().expect("lock").get_mut(&size).and_then(Vec::pop);
        let buffer = reused.unwrap_or_else(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain tile"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
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
        let mut out = Vec::with_capacity(tiles.len());
        // About 16 MB a trip.
        let mut start = 0;
        while start < tiles.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < tiles.len() && (end == start || bytes + tiles[end].buffer().size() <= 64 * TILE_BYTES) {
                bytes += tiles[end].buffer().size();
                end += 1;
            }
            let chunk = &tiles[start..end];
            start = end;
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("terrain readback"), size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("terrain readback") });
            let mut at = 0;
            for t in chunk {
                let size = t.buffer().size();
                enc.copy_buffer_to_buffer(t.buffer(), 0, &staging, at, size);
                at += size;
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
                    for t in chunk {
                        let n = (t.buffer().size() / 4) as usize;
                        out.push(all[at..at + n].to_vec());
                        at += n;
                    }
                    drop(view);
                    staging.unmap();
                }
                _ => {
                    log::error!("terrain GPU: a readback failed");
                    out.extend(chunk.iter().map(|t| vec![0.0; (t.buffer().size() / 4) as usize]));
                }
            }
        }
        out
    }

    /// Runs a point kernel over tiles: for each, its uniform block and its
    /// inputs (`None` unconnected); `outputs` gives each output's planes.
    /// Gives each tile's outputs, or `None` when the device refuses the
    /// kernel.
    pub fn run_point(&self, kernel: &GpuKernel, jobs: &[(Vec<u8>, Vec<Option<&GpuTile>>)], outputs: &[usize]) -> Option<Vec<Vec<GpuTile>>> {
        if jobs.is_empty() {
            return Some(Vec::new());
        }
        let pipeline = self.pipeline(kernel)?;
        let layout = pipeline.get_bind_group_layout(0);
        let block = jobs.iter().map(|j| j.0.len() as u64).max().unwrap_or(16).max(16);
        let slot = block.div_ceil(self.align) * self.align;
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain uniforms"),
            size: slot * jobs.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut bytes = vec![0u8; (slot * jobs.len() as u64) as usize];
        for (k, (u, _)) in jobs.iter().enumerate() {
            bytes[k * slot as usize..k * slot as usize + u.len()].copy_from_slice(u);
        }
        self.queue.write_buffer(&uniforms, 0, &bytes);
        let made: Vec<Vec<GpuTile>> = jobs.iter().map(|_| outputs.iter().map(|&p| self.tile(p)).collect()).collect();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(kernel.name) });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(kernel.name), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            for (k, ((_, inputs), outs)) in jobs.iter().zip(&made).enumerate() {
                let mut entries = vec![wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &uniforms, offset: slot * k as u64, size: wgpu::BufferSize::new(block) }),
                }];
                for (i, input) in inputs.iter().enumerate() {
                    let buffer = input.map_or(&self.stand_in, |t| t.buffer());
                    entries.push(wgpu::BindGroupEntry { binding: 1 + i as u32, resource: buffer.as_entire_binding() });
                }
                for (o, t) in outs.iter().enumerate() {
                    entries.push(wgpu::BindGroupEntry { binding: 1 + (inputs.len() + o) as u32, resource: t.buffer().as_entire_binding() });
                }
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries });
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(TILE / 16, TILE / 16, 1);
            }
        }
        self.queue.submit([enc.finish()]);
        if let Some(e) = pollster::block_on(scope.pop()) {
            self.refuse(kernel, e);
            return None;
        }
        Some(made)
    }

    /// Waits for what was submitted (for timings).
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}
