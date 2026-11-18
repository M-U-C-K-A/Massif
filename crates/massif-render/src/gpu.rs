//! The path tracer on the graphics card: the scene's words in storage
//! buffers, the textures in an array, and a compute shader that adds a
//! sample a pixel each time it runs (see `trace.wgsl`).

use crate::camera::{Camera, Light};
use crate::sky::{Sky, SkyState};
use crate::map_source::Extent;
use crate::materials::{LAYER_BYTES, Layer, Materials, SIDE};
use crate::scene::GpuChunk;
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::num::NonZeroU64;

/// The image's format: written as sRGB values by the shader, shown
/// through an sRGB view.
pub const IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub const IMAGE_VIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Levels of detail of a texture layer: 16, 8, 4, 2 and 1 pixels across.
const MIPS: u32 = 5;
/// Levels the pyramid of heights may have (16: 2^15 chunks across).
const LEVELS: usize = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    sun: [f32; 4],
    sun_light: [f32; 4],
    air: [f32; 4],
    fog: [f32; 4],
    media: [f32; 4],
    lens: [f32; 4],
    clouds: [f32; 4],
    wind: [f32; 4],
    size: [u32; 4],
    grid: [i32; 4],
    info: [u32; 4],
    level_off: [u32; LEVELS],
    level_w: [u32; LEVELS],
}

/// For every chunk, one above its highest block a ray can reach; then for
/// every 2 × 2 of those, the highest; and so on up to one value.
struct Pyramid {
    /// Width, depth and first word of every level.
    levels: Vec<(u32, u32, u32)>,
    data: Vec<u32>,
    dirty: bool,
}

impl Pyramid {
    fn new(width: u32, depth: u32) -> Self {
        let mut levels = Vec::new();
        let (mut w, mut d, mut at) = (width.max(1), depth.max(1), 0);
        loop {
            levels.push((w, d, at));
            at += w * d;
            if (w == 1 && d == 1) || levels.len() == LEVELS {
                break;
            }
            (w, d) = (w.div_ceil(2), d.div_ceil(2));
        }
        Self { levels, data: vec![0; at as usize], dirty: true }
    }

    fn set(&mut self, cx: u32, cz: u32, top: u32) {
        let (w, _, at) = self.levels[0];
        self.data[(at + cz * w + cx) as usize] = top;
        let (mut x, mut z) = (cx, cz);
        for l in 1..self.levels.len() {
            let (pw, pd, pat) = self.levels[l - 1];
            let (lw, _, lat) = self.levels[l];
            (x, z) = (x / 2, z / 2);
            let mut high = 0;
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sz) = (x * 2 + dx, z * 2 + dz);
                if sx < pw && sz < pd {
                    high = high.max(self.data[(pat + sz * pw + sx) as usize]);
                }
            }
            self.data[(lat + z * lw + x) as usize] = high;
        }
        self.dirty = true;
    }
}

/// Where a chunk lives in the buffers.
struct Resident {
    slot: u32,
    tree_at: u32,
    tree_len: u32,
    voxel_at: u32,
    voxel_len: u32,
}

/// A storage buffer and its copy here: changes are made to the copy and
/// sent at once before a frame, one write a buffer.
struct Mirror {
    label: &'static str,
    gpu: wgpu::Buffer,
    cpu: Vec<u32>,
    /// Words changed since the last time they were sent.
    dirty: Option<(usize, usize)>,
}

impl Mirror {
    fn new(device: &wgpu::Device, label: &'static str, words: usize) -> Self {
        Self { label, gpu: storage(device, label, words as u64 * 4), cpu: vec![0; words], dirty: None }
    }

    fn mark(&mut self, from: usize, to: usize) {
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(from), b.max(to)),
            None => (from, to),
        });
    }

    fn set(&mut self, at: usize, words: &[u32]) {
        if self.cpu.len() < at + words.len() {
            self.cpu.resize(at + words.len(), 0);
        }
        self.cpu[at..at + words.len()].copy_from_slice(words);
        self.mark(at, at + words.len());
    }

    fn push(&mut self, words: &[u32]) -> usize {
        let at = self.cpu.len();
        self.cpu.extend_from_slice(words);
        self.mark(at, self.cpu.len());
        at
    }

    /// Send what changed, the buffer made larger first if it must be.
    /// `Err`: larger than the card can bind. `Ok(true)`: made again.
    fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, most: u64) -> Result<bool, ()> {
        let need = self.cpu.len() as u64 * 4;
        let mut remade = false;
        if need > self.gpu.size() {
            if need > most {
                return Err(());
            }
            // A quarter more, by the megabyte: room for what comes next
            // without making it again each frame.
            self.gpu = storage(device, self.label, (need + need / 4).next_multiple_of(1 << 20).min(most).max(need));
            self.dirty = Some((0, self.cpu.len()));
            remade = true;
        }
        if let Some((a, b)) = self.dirty.take()
            && b > a
        {
            queue.write_buffer(&self.gpu, a as u64 * 4, bytemuck::cast_slice(&self.cpu[a..b]));
        }
        Ok(remade)
    }
}

/// What the graphics card holds of the scene, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub scene: u64,
    pub textures: u64,
    pub image: u64,
}

impl Memory {
    pub fn total(&self) -> u64 {
        self.scene + self.textures + self.image
    }
}

struct Image {
    size: [u32; 2],
    texture: wgpu::Texture,
    storage: wgpu::TextureView,
    view: wgpu::TextureView,
    accum: wgpu::Buffer,
}

/// The renderer of one scene.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    extent: Extent,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    grid: Mirror,
    records: Mirror,
    tree: Mirror,
    voxels: Mirror,
    /// For every chunk, where its list of lights starts and how long it is
    /// (the lights of it and of the 8 chunks around: those that light
    /// it), then the lists: `[x, y, z, material]` each.
    lights: Mirror,
    /// The lights of each chunk, in the scene's blocks.
    chunk_lights: HashMap<(i32, i32), Vec<[u32; 4]>>,
    lights_dirty: bool,
    pyramid_buf: wgpu::Buffer,
    materials_buf: wgpu::Buffer,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    atlas_layers: u32,
    materials_revision: u64,
    /// Where the shaped blocks' faces start in the materials' buffer, in
    /// groups of four words.
    quad_base: u32,
    sky: Sky,
    /// The world's sea level, from which the air thins.
    pub sea_level: f64,
    image: Option<Image>,
    bind_group: Option<wgpu::BindGroup>,
    pyramid: Pyramid,
    residents: HashMap<(i32, i32), Resident>,
    free_slots: Vec<u32>,
    slots: u32,
    /// Words of the tree and voxel buffers left by chunks since replaced.
    garbage: u64,
    /// A chunk did not fit the graphics card's buffers.
    full: bool,
    /// Samples added up in the image so far.
    pub samples: u32,
    frame: u32,
    max_binding: u64,
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
}

fn storage(device: &wgpu::Device, label: &str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes.max(16).next_multiple_of(4), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false })
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, extent: Extent) -> Self {
        let source = format!("{}\n{}", crate::sky::ATMOSPHERE, include_str!("trace.wgsl"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("massif render trace"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let lut = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let buffer = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif render"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: NonZeroU64::new(size_of::<Params>() as u64) },
                    count: None,
                },
                buffer(1, true),
                buffer(2, true),
                buffer(3, true),
                buffer(4, true),
                buffer(5, true),
                buffer(6, true),
                buffer(7, false),
                buffer(15, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: IMAGE_FORMAT, view_dimension: wgpu::TextureViewDimension::D2 },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 10, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                lut(11),
                lut(12),
                lut(13),
                wgpu::BindGroupLayoutEntry { binding: 14, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("massif render"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("massif render trace"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif render params"), size: size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif render atlas"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let cells = extent.width as u64 * extent.depth as u64;
        let pyramid = Pyramid::new(extent.width, extent.depth);
        let stride = 4 + extent.sections as u64;
        let max_binding = device.limits().max_storage_buffer_binding_size.min(device.limits().max_buffer_size);
        let (atlas, atlas_view) = Self::make_atlas(device, 1);
        let r = Self {
            device: device.clone(),
            queue: queue.clone(),
            extent,
            pipeline,
            layout,
            params,
            sampler,
            grid: Mirror::new(device, "massif render grid", cells as usize),
            records: Mirror::new(device, "massif render records", stride as usize * 64),
            tree: Mirror::new(device, "massif render tree", 0),
            voxels: Mirror::new(device, "massif render voxels", 0),
            lights: Mirror::new(device, "massif render lights", cells as usize * 2),
            chunk_lights: HashMap::new(),
            lights_dirty: false,
            pyramid_buf: storage(device, "massif render pyramid", pyramid.data.len() as u64 * 4),
            materials_buf: storage(device, "massif render materials", 16 * 256),
            atlas,
            atlas_view,
            atlas_layers: 1,
            materials_revision: 0,
            quad_base: 0,
            sky: Sky::new(device),
            sea_level: 63.0,
            image: None,
            bind_group: None,
            pyramid,
            residents: HashMap::new(),
            free_slots: Vec::new(),
            slots: 0,
            garbage: 0,
            full: false,
            samples: 0,
            frame: 0,
            max_binding,
        };
        r.write_layer(0, &[255; LAYER_BYTES]);
        r
    }

    pub fn extent(&self) -> Extent {
        self.extent
    }

    fn make_atlas(device: &wgpu::Device, layers: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif render atlas"),
            size: wgpu::Extent3d { width: SIDE as u32, height: SIDE as u32, depth_or_array_layers: layers.max(1) },
            mip_level_count: MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        (texture, view)
    }

    /// One layer and its smaller levels (alpha-weighted averages, so that
    /// holes do not darken the colours around them).
    fn write_layer(&self, layer: u32, rgba: &Layer) {
        let mut level = rgba.to_vec();
        let mut side = SIDE;
        for mip in 0..MIPS {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
                &level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side as u32 * 4), rows_per_image: Some(side as u32) },
                wgpu::Extent3d { width: side as u32, height: side as u32, depth_or_array_layers: 1 },
            );
            if side == 1 {
                break;
            }
            let half = side / 2;
            let mut next = vec![0u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    let (mut sum, mut alpha) = ([0u32; 3], 0u32);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = &level[((y * 2 + dy) * side + x * 2 + dx) * 4..][..4];
                        for k in 0..3 {
                            sum[k] += p[k] as u32 * p[3] as u32;
                        }
                        alpha += p[3] as u32;
                    }
                    let o = &mut next[(y * half + x) * 4..][..4];
                    for k in 0..3 {
                        o[k] = sum[k].checked_div(alpha).unwrap_or(0) as u8;
                    }
                    o[3] = (alpha / 4) as u8;
                }
            }
            level = next;
            side = half;
        }
    }

    /// The materials and textures met since the last time, to the card.
    pub fn sync_materials(&mut self, materials: &Materials) {
        if materials.revision() == self.materials_revision {
            return;
        }
        let (revision, list, quads, layers) = materials.snapshot();
        // The materials, then the faces of the shaped ones, in one buffer.
        let mut words: Vec<u32> = bytemuck::cast_slice(&list).to_vec();
        self.quad_base = (words.len() / 4) as u32;
        words.extend_from_slice(bytemuck::cast_slice(&quads));
        let bytes: &[u8] = bytemuck::cast_slice(&words);
        if bytes.len() as u64 > self.materials_buf.size() {
            self.materials_buf = storage(&self.device, "massif render materials", (bytes.len() as u64 * 2).next_power_of_two());
            self.bind_group = None;
        }
        self.queue.write_buffer(&self.materials_buf, 0, bytes);
        let have = self.atlas.size().depth_or_array_layers;
        if layers.len() as u32 > have {
            let most = self.device.limits().max_texture_array_layers;
            let room = (layers.len() as u32).next_power_of_two().clamp(16, most.max(1));
            let (atlas, view) = Self::make_atlas(&self.device, room);
            self.atlas = atlas;
            self.atlas_view = view;
            self.atlas_layers = 0;
            self.bind_group = None;
        }
        let room = self.atlas.size().depth_or_array_layers as usize;
        for (k, layer) in layers.iter().enumerate().take(room).skip(self.atlas_layers as usize) {
            self.write_layer(k as u32, layer);
        }
        self.atlas_layers = layers.len().min(room) as u32;
        self.materials_revision = revision;
        self.samples = 0;
    }

    fn stride(&self) -> u32 {
        4 + self.extent.sections
    }

    /// Put built chunks in the scene, in place of what was there. False when
    /// the scene no longer fits the graphics card's buffers.
    pub fn put(&mut self, chunks: Vec<GpuChunk>) -> bool {
        let mut fits = true;
        for chunk in chunks {
            fits &= self.put_one(chunk);
        }
        self.samples = 0;
        fits
    }

    fn put_one(&mut self, chunk: GpuChunk) -> bool {
        let e = self.extent;
        let (cx, cz) = (chunk.x - e.x0, chunk.z - e.z0);
        if cx < 0 || cz < 0 || cx >= e.width as i32 || cz >= e.depth as i32 || chunk.entries.len() != e.sections as usize {
            return true;
        }
        let cell = cz as usize * e.width as usize + cx as usize;
        // Its lights, in the scene's blocks.
        let lights: Vec<[u32; 4]> = chunk.lights.iter().map(|&[p, m]| [(cx as u32) * 16 + (p & 15), p >> 8, (cz as u32) * 16 + ((p >> 4) & 15), m]).collect();
        if !lights.is_empty() || self.chunk_lights.contains_key(&(chunk.x, chunk.z)) {
            self.lights_dirty = true;
            if lights.is_empty() {
                self.chunk_lights.remove(&(chunk.x, chunk.z));
            } else {
                self.chunk_lights.insert((chunk.x, chunk.z), lights);
            }
        }
        let slot = match self.residents.remove(&(chunk.x, chunk.z)) {
            Some(old) => {
                self.garbage += old.tree_len as u64 + old.voxel_len as u64;
                old.slot
            }
            None => self.free_slots.pop().unwrap_or_else(|| {
                self.slots += 1;
                self.slots - 1
            }),
        };
        if chunk.top == 0 {
            // Nothing a ray can reach: no record.
            self.grid.set(cell, &[0]);
            self.pyramid.set(cx as u32, cz as u32, 0);
            self.free_slots.push(slot);
            return true;
        }
        let voxel_words: Vec<u32> = chunk.voxels.chunks(2).map(|p| p[0] as u32 | (p.get(1).copied().unwrap_or(0) as u32) << 16).collect();
        let most = (self.max_binding / 4) as usize;
        if self.tree.cpu.len() + chunk.tree.len() > most || self.voxels.cpu.len() + voxel_words.len() > most {
            self.pack();
            if self.tree.cpu.len() + chunk.tree.len() > most || self.voxels.cpu.len() + voxel_words.len() > most {
                self.free_slots.push(slot);
                self.grid.set(cell, &[0]);
                self.pyramid.set(cx as u32, cz as u32, 0);
                self.full = true;
                return false;
            }
        }
        let tree_at = self.tree.push(&chunk.tree) as u32;
        let voxel_at = self.voxels.push(&voxel_words) as u32;
        let stride = self.stride() as usize;
        let mut record = Vec::with_capacity(stride);
        record.extend_from_slice(&[chunk.top, tree_at, voxel_at * 2, 0]);
        record.extend_from_slice(&chunk.entries);
        self.records.set(slot as usize * stride, &record);
        self.grid.set(cell, &[slot + 1]);
        self.pyramid.set(cx as u32, cz as u32, chunk.top);
        self.residents.insert((chunk.x, chunk.z), Resident { slot, tree_at, tree_len: chunk.tree.len() as u32, voxel_at, voxel_len: voxel_words.len() as u32 });
        if self.garbage > 1 << 20 && self.garbage * 2 > (self.tree.cpu.len() + self.voxels.cpu.len()) as u64 {
            self.pack();
        }
        true
    }

    /// Lay every chunk's words again one after the other, leaving out
    /// those of chunks replaced since.
    fn pack(&mut self) {
        if self.garbage == 0 {
            return;
        }
        let mut keys: Vec<(i32, i32)> = self.residents.keys().copied().collect();
        keys.sort_unstable();
        let (mut tree, mut voxels) = (Vec::with_capacity(self.tree.cpu.len()), Vec::with_capacity(self.voxels.cpu.len()));
        let stride = self.stride() as usize;
        for key in keys {
            let r = self.residents.get_mut(&key).expect("listed");
            let (t, v) = (tree.len() as u32, voxels.len() as u32);
            tree.extend_from_slice(&self.tree.cpu[r.tree_at as usize..(r.tree_at + r.tree_len) as usize]);
            voxels.extend_from_slice(&self.voxels.cpu[r.voxel_at as usize..(r.voxel_at + r.voxel_len) as usize]);
            r.tree_at = t;
            r.voxel_at = v;
            let at = r.slot as usize * stride;
            self.records.cpu[at + 1] = t;
            self.records.cpu[at + 2] = v * 2;
        }
        self.tree.cpu = tree;
        self.voxels.cpu = voxels;
        let (t, v, r) = (self.tree.cpu.len(), self.voxels.cpu.len(), self.records.cpu.len());
        self.tree.mark(0, t);
        self.voxels.mark(0, v);
        self.records.mark(0, r);
        self.garbage = 0;
    }

    /// The lists of lights again: each chunk's, then for each chunk those
    /// of it and of the 8 around it.
    fn index_lights(&mut self) {
        let e = self.extent;
        let (w, d) = (e.width as i64, e.depth as i64);
        let cells = (w * d) as usize;
        let mut count = vec![0u32; cells];
        let near = |x: i32, z: i32| {
            let (cx, cz) = ((x - e.x0) as i64, (z - e.z0) as i64);
            (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| (cx + dx, cz + dz))).filter(move |&(x, z)| x >= 0 && z >= 0 && x < w && z < d).map(move |(x, z)| (z * w + x) as usize)
        };
        for (&(x, z), list) in &self.chunk_lights {
            for c in near(x, z) {
                count[c] += list.len() as u32;
            }
        }
        let mut words = vec![0u32; cells * 2];
        let mut at = 0u32;
        for c in 0..cells {
            words[c * 2] = at;
            at += count[c];
        }
        let mut entries = vec![0u32; at as usize * 4];
        let mut fill = vec![0u32; cells];
        for (&(x, z), list) in &self.chunk_lights {
            for c in near(x, z) {
                for l in list {
                    let k = (words[c * 2] + fill[c]) as usize * 4;
                    entries[k..k + 4].copy_from_slice(l);
                    fill[c] += 1;
                }
            }
        }
        for c in 0..cells {
            words[c * 2 + 1] = fill[c];
        }
        words.extend_from_slice(&entries);
        let n = words.len();
        self.lights.cpu = words;
        self.lights.mark(0, n);
        self.lights_dirty = false;
    }

    /// Lights in the scene.
    pub fn light_count(&self) -> usize {
        self.chunk_lights.values().map(Vec::len).sum()
    }

    /// Send the scene's changes to the graphics card.
    pub fn flush(&mut self) {
        if self.lights_dirty {
            self.index_lights();
        }
        let most = self.max_binding;
        let (device, queue) = (&self.device, &self.queue);
        let mut remade = false;
        for m in [&mut self.grid, &mut self.records, &mut self.tree, &mut self.voxels, &mut self.lights] {
            match m.flush(device, queue, most) {
                Ok(r) => remade |= r,
                Err(()) => self.full = true,
            }
        }
        if self.pyramid.dirty {
            if self.pyramid_buf.size() < self.pyramid.data.len() as u64 * 4 {
                self.pyramid_buf = storage(device, "massif render pyramid", self.pyramid.data.len() as u64 * 4);
                remade = true;
            }
            queue.write_buffer(&self.pyramid_buf, 0, bytemuck::cast_slice(&self.pyramid.data));
            self.pyramid.dirty = false;
        }
        if remade {
            self.bind_group = None;
        }
    }

    /// Whether part of the scene did not fit the graphics card.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// The image the renderer draws into, `width` × `height`: its texture
    /// (to show through [`Self::view`]) made again when the size changes.
    pub fn resize(&mut self, width: u32, height: u32) {
        let size = [width.max(1), height.max(1)];
        if self.image.as_ref().is_some_and(|i| i.size == size) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif render image"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: IMAGE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[IMAGE_VIEW_FORMAT],
        });
        let storage_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Shown through an sRGB view of the same pixels, which only samples.
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(IMAGE_VIEW_FORMAT), usage: Some(wgpu::TextureUsages::TEXTURE_BINDING), ..Default::default() });
        let accum = storage(&self.device, "massif render accumulation", size[0] as u64 * size[1] as u64 * 16);
        self.image = Some(Image { size, texture, storage: storage_view, view, accum });
        self.bind_group = None;
        self.samples = 0;
    }

    /// The image, as an sRGB view to show.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.image.as_ref().map(|i| &i.view)
    }

    /// Start adding samples up again (the view or the light changed).
    pub fn restart(&mut self) {
        self.samples = 0;
    }

    /// Add a sample a pixel of the view from `camera` under `light`: every
    /// pixel (`coarse` false), or one sample for each 2 × 2 pixels, which
    /// starts the image over, while the view moves.
    pub fn render(&mut self, camera: &Camera, light: &Light, coarse: bool) -> Option<wgpu::CommandBuffer> {
        self.flush();
        let image = self.image.as_ref()?;
        let [w, h] = image.size;
        if self.bind_group.is_none() {
            let image = self.image.as_ref()?;
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif render"),
                layout: &self.layout,
                entries: &[
                    entry(0, &self.params),
                    entry(1, &self.grid.gpu),
                    entry(2, &self.records.gpu),
                    entry(3, &self.tree.gpu),
                    entry(4, &self.voxels.gpu),
                    entry(5, &self.pyramid_buf),
                    entry(6, &self.materials_buf),
                    entry(7, &image.accum),
                    entry(15, &self.lights.gpu),
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&image.storage) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(self.sky.transmittance()) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(self.sky.multi()) },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(self.sky.sky_view()) },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::Sampler(self.sky.sampler()) },
                ],
            }));
        }
        if coarse {
            self.samples = 0;
        }
        let e = self.extent;
        let origin = [e.x0 as f64 * 16.0, e.min_y as f64, e.z0 as f64 * 16.0];
        let basis = camera.basis();
        let eye = [(camera.eye[0] - origin[0]) as f32, (camera.eye[1] - origin[1]) as f32, (camera.eye[2] - origin[2]) as f32];
        let tan = (camera.fov_y * 0.5).tan();
        let mut level_off = [0u32; LEVELS];
        let mut level_w = [0u32; LEVELS];
        for (l, &(lw, _, at)) in self.pyramid.levels.iter().enumerate() {
            level_off[l] = at;
            level_w[l] = lw;
        }
        // The sun, or the moon at night.
        let (sun, strength, tint) = light.source();
        let sea = (self.sea_level - origin[1]) as f32;
        // The sky as seen from the eyes' height over the sea, by the 10 m:
        // its tables are filled again only when that changes.
        let altitude = ((camera.eye[1] - self.sea_level).max(0.0) / 1000.0 * 100.0).round() as f32 / 100.0 + 0.02;
        let state = SkyState { sun: sun.to_array(), rayleigh: light.air, mie: light.haze, mie_g: light.haze_g, altitude };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif render") });
        if self.sky.update(&self.queue, &mut encoder, state) && !coarse {
            self.samples = 0;
        }
        let tint = tint.map(|c| c * strength);
        let params = Params {
            eye: [eye[0], eye[1], eye[2], tan],
            right: [basis.right.x, basis.right.y, basis.right.z, w as f32 / h as f32],
            up: [basis.up.x, basis.up.y, basis.up.z, 2.0 * tan / h as f32],
            forward: [basis.forward.x, basis.forward.y, basis.forward.z, light.exposure],
            sun: [sun.x, sun.y, sun.z, light.sun_radius.cos()],
            sun_light: [tint[0], tint[1], tint[2], light.sky_strength],
            air: [light.air, light.haze, light.haze_g, altitude],
            fog: [light.fog, light.fog_height - origin[1] as f32, light.fog_falloff.max(0.5), light.fog_g],
            media: [light.shafts as u8 as f32, sea, light.emission, light.aperture],
            lens: [light.focus.max(0.1), light.sea as u8 as f32, self.quad_base as f32, 0.0],
            clouds: [light.cloud_cover, light.cloud_height - origin[1] as f32, light.cloud_thickness.max(1.0), if light.cloud_cover > 0.0 { light.cloud_density } else { 0.0 }],
            wind: [light.wind[0], light.wind[1], 1.0 / light.cloud_size.max(1.0), 0.0],
            size: [w, h, self.frame.wrapping_mul(2654435761).wrapping_add(1), self.samples],
            grid: [e.width as i32, e.depth as i32, e.height(), e.sections as i32],
            info: [self.stride(), self.pyramid.levels.len() as u32, light.bounces, coarse as u32],
            level_off,
            level_w,
        };
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("massif render trace"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, self.bind_group.as_ref(), &[]);
            let shift = coarse as u32;
            pass.dispatch_workgroups((w >> shift).max(1).div_ceil(8) + shift, (h >> shift).max(1).div_ceil(8) + shift, 1);
        }
        self.frame = self.frame.wrapping_add(1);
        self.samples = if coarse { 0 } else { self.samples + 1 };
        Some(encoder.finish())
    }

    /// The image last drawn, RGBA in sRGB, for tests and stills.
    pub fn read_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        let image = self.image.as_ref()?;
        let [w, h] = image.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif render readback"), size: row as u64 * h as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif render readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &image.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = buffer.get_mapped_range(..).ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]);
        }
        Some((w, h, out))
    }

    /// The height of the highest block a ray can reach in the chunk of
    /// `(x, z)` (the world's blocks), if any.
    pub fn surface(&self, x: f64, z: f64) -> Option<f64> {
        let e = self.extent;
        let (cx, cz) = ((x / 16.0).floor() as i64 - e.x0 as i64, (z / 16.0).floor() as i64 - e.z0 as i64);
        if cx < 0 || cz < 0 || cx >= e.width as i64 || cz >= e.depth as i64 {
            return None;
        }
        let top = self.pyramid.data[(cz * e.width as i64 + cx) as usize];
        (top > 0).then(|| (e.min_y + top as i32) as f64)
    }

    /// The distance from `from` (the world's blocks) along `dir` to the
    /// first block a ray stops at, if within `most` blocks: for the focus
    /// picked on the image. Walks the scene's copy here, block by block.
    pub fn pick(&self, from: [f64; 3], dir: [f64; 3], most: f64) -> Option<f64> {
        let e = self.extent;
        let origin = [e.x0 as f64 * 16.0, e.min_y as f64, e.z0 as f64 * 16.0];
        let p = [from[0] - origin[0], from[1] - origin[1], from[2] - origin[2]];
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len == 0.0 {
            return None;
        }
        let d = dir.map(|c| c / len);
        let dims = [e.width as i64 * 16, e.height() as i64, e.depth as i64 * 16];
        let stride = self.stride() as usize;
        let occupied = |v: [i64; 3]| -> bool {
            if (0..3).any(|a| v[a] < 0 || v[a] >= dims[a]) {
                return false;
            }
            let (cx, cz) = (v[0] >> 4, v[2] >> 4);
            let slot = self.grid.cpu[(cz * e.width as i64 + cx) as usize];
            if slot == 0 {
                return false;
            }
            let rec = (slot - 1) as usize * stride;
            let entry = self.records.cpu[rec + 4 + (v[1] >> 4) as usize];
            if entry == 0 {
                return false;
            }
            if entry & crate::scene::UNIFORM != 0 {
                return true;
            }
            let root = (self.records.cpu[rec + 1] + entry - 1) as usize;
            let mask = self.tree.cpu[root] as u64 | (self.tree.cpu[root + 1] as u64) << 32;
            let b = [(v[0] & 15) >> 2, (v[1] & 15) >> 2, (v[2] & 15) >> 2];
            let bi = (b[0] + b[2] * 4 + b[1] * 16) as u32;
            if mask >> bi & 1 == 0 {
                return false;
            }
            let brick = self.records.cpu[rec + 1] as usize + self.tree.cpu[root + 2] as usize + (mask & ((1u64 << bi) - 1)).count_ones() as usize * 3;
            let bm = self.tree.cpu[brick] as u64 | (self.tree.cpu[brick + 1] as u64) << 32;
            let w = [v[0] & 3, v[1] & 3, v[2] & 3];
            bm >> (w[0] + w[2] * 4 + w[1] * 16) & 1 != 0
        };
        // A plain walk from block to block (Amanatides and Woo).
        let mut v = p.map(|c| c.floor() as i64);
        let step = d.map(|c| if c > 0.0 { 1 } else { -1 });
        let mut next = [0.0; 3];
        let mut delta = [0.0; 3];
        for a in 0..3 {
            delta[a] = if d[a] == 0.0 { f64::INFINITY } else { (1.0 / d[a]).abs() };
            let edge = if d[a] > 0.0 { v[a] as f64 + 1.0 } else { v[a] as f64 };
            next[a] = if d[a] == 0.0 { f64::INFINITY } else { (edge - p[a]) / d[a] };
        }
        let mut t = 0.0;
        while t < most {
            let a = (0..3).min_by(|&i, &j| next[i].total_cmp(&next[j])).expect("three axes");
            t = next[a];
            v[a] += step[a];
            next[a] += delta[a];
            if occupied(v) {
                return Some(t);
            }
        }
        None
    }

    /// Chunks in the scene.
    pub fn chunks(&self) -> usize {
        self.residents.len()
    }

    /// What the scene takes on the graphics card.
    pub fn memory(&self) -> Memory {
        let scene = self.grid.gpu.size() + self.records.gpu.size() + self.tree.gpu.size() + self.voxels.gpu.size() + self.lights.gpu.size() + self.pyramid_buf.size() + self.materials_buf.size();
        let textures = {
            let s = self.atlas.size();
            (s.width as u64 * s.height as u64 * 4 * s.depth_or_array_layers as u64) * 4 / 3
        };
        let image = self.image.as_ref().map_or(0, |i| i.size[0] as u64 * i.size[1] as u64 * (16 + 4));
        Memory { scene, textures: textures + self.sky.bytes(), image }
    }

    /// Words of the scene's trees and voxels, and of them, those of chunks
    /// replaced since.
    pub fn words(&self) -> (u64, u64) {
        ((self.tree.cpu.len() + self.voxels.cpu.len()) as u64, self.garbage)
    }
}
