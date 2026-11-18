//! The path tracer on the graphics card: the scene's words in storage
//! buffers, the textures in an array, and a compute shader that adds a
//! sample a pixel each time it runs (see `trace.wgsl`).

use crate::camera::{Camera, Light};
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
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
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
    voxel_at: u32,
    chunk: GpuChunk,
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
    grid: wgpu::Buffer,
    records: wgpu::Buffer,
    tree: wgpu::Buffer,
    voxels: wgpu::Buffer,
    pyramid_buf: wgpu::Buffer,
    materials_buf: wgpu::Buffer,
    atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    atlas_layers: u32,
    materials_revision: u64,
    image: Option<Image>,
    bind_group: Option<wgpu::BindGroup>,
    pyramid: Pyramid,
    residents: HashMap<(i32, i32), Resident>,
    free_slots: Vec<u32>,
    slots: u32,
    /// Words used in the tree and voxel buffers, and of them, those of
    /// chunks since replaced.
    tree_used: u32,
    voxel_used: u32,
    garbage: u64,
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
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("massif render trace"), source: wgpu::ShaderSource::Wgsl(include_str!("trace.wgsl").into()) });
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
            grid: storage(device, "massif render grid", cells * 4),
            records: storage(device, "massif render records", stride * 4 * 1024),
            tree: storage(device, "massif render tree", 4 << 20),
            voxels: storage(device, "massif render voxels", 4 << 20),
            pyramid_buf: storage(device, "massif render pyramid", pyramid.data.len() as u64 * 4),
            materials_buf: storage(device, "massif render materials", 16 * 256),
            atlas,
            atlas_view,
            atlas_layers: 1,
            materials_revision: 0,
            image: None,
            bind_group: None,
            pyramid,
            residents: HashMap::new(),
            free_slots: Vec::new(),
            slots: 0,
            tree_used: 0,
            voxel_used: 0,
            garbage: 0,
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
        let (revision, list, layers) = materials.snapshot();
        let bytes: &[u8] = bytemuck::cast_slice(&list);
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
        let tree_words = chunk.tree.len() as u32;
        let voxel_words = chunk.voxels.len().div_ceil(2) as u32;
        let slot = match self.residents.remove(&(chunk.x, chunk.z)) {
            Some(old) => {
                self.garbage += old.chunk.tree.len() as u64 + old.chunk.voxels.len().div_ceil(2) as u64;
                old.slot
            }
            None => self.free_slots.pop().unwrap_or_else(|| {
                self.slots += 1;
                self.slots - 1
            }),
        };
        if chunk.top == 0 && chunk.tree.is_empty() {
            // Nothing a ray can reach: no record.
            self.grid_write(cx as u32, cz as u32, 0);
            self.pyramid.set(cx as u32, cz as u32, 0);
            self.free_slots.push(slot);
            return true;
        }
        // Room in the buffers: grown (and packed) when full.
        let need_records = (slot as u64 + 1) * self.stride() as u64 * 4;
        if need_records > self.records.size() {
            let bytes = (need_records * 2).min(self.max_binding).max(need_records);
            if need_records > self.max_binding {
                self.free_slots.push(slot);
                return false;
            }
            self.records = storage(&self.device, "massif render records", bytes);
            self.bind_group = None;
            self.rewrite_all_records();
        }
        if (self.tree_used + tree_words) as u64 * 4 > self.tree.size() || (self.voxel_used + voxel_words) as u64 * 4 > self.voxels.size() {
            if !self.repack(tree_words as u64, voxel_words as u64) {
                self.free_slots.push(slot);
                return false;
            }
        }
        let (tree_at, voxel_at) = (self.tree_used, self.voxel_used);
        self.tree_used += tree_words;
        self.voxel_used += voxel_words;
        let resident = Resident { slot, tree_at, voxel_at, chunk };
        self.upload(&resident);
        self.grid_write(cx as u32, cz as u32, slot + 1);
        self.pyramid.set(cx as u32, cz as u32, resident.chunk.top);
        self.residents.insert((resident.chunk.x, resident.chunk.z), resident);
        true
    }

    fn grid_write(&self, cx: u32, cz: u32, value: u32) {
        let at = (cz as u64 * self.extent.width as u64 + cx as u64) * 4;
        self.queue.write_buffer(&self.grid, at, bytemuck::bytes_of(&value));
    }

    fn record(&self, r: &Resident) -> Vec<u32> {
        let mut words = Vec::with_capacity(self.stride() as usize);
        words.extend_from_slice(&[r.chunk.top, r.tree_at, r.voxel_at * 2, 0]);
        words.extend_from_slice(&r.chunk.entries);
        words
    }

    fn upload(&self, r: &Resident) {
        let stride = self.stride() as u64 * 4;
        self.queue.write_buffer(&self.records, r.slot as u64 * stride, bytemuck::cast_slice(&self.record(r)));
        if !r.chunk.tree.is_empty() {
            self.queue.write_buffer(&self.tree, r.tree_at as u64 * 4, bytemuck::cast_slice(&r.chunk.tree));
        }
        if !r.chunk.voxels.is_empty() {
            let mut words: Vec<u32> = r.chunk.voxels.chunks(2).map(|p| p[0] as u32 | (p.get(1).copied().unwrap_or(0) as u32) << 16).collect();
            words.truncate(r.chunk.voxels.len().div_ceil(2));
            self.queue.write_buffer(&self.voxels, r.voxel_at as u64 * 4, bytemuck::cast_slice(&words));
        }
    }

    fn rewrite_all_records(&self) {
        for r in self.residents.values() {
            let stride = self.stride() as u64 * 4;
            self.queue.write_buffer(&self.records, r.slot as u64 * stride, bytemuck::cast_slice(&self.record(r)));
        }
    }

    /// Lay every chunk again from the start of new buffers, large enough
    /// for them and `more` words besides. False when that is past what the
    /// graphics card can bind.
    fn repack(&mut self, more_tree: u64, more_voxels: u64) -> bool {
        let live_tree: u64 = self.residents.values().map(|r| r.chunk.tree.len() as u64).sum::<u64>() + more_tree;
        let live_voxels: u64 = self.residents.values().map(|r| r.chunk.voxels.len().div_ceil(2) as u64).sum::<u64>() + more_voxels;
        let size = |live: u64, now: u64| -> u64 {
            let want = (live * 4 * 3 / 2).max(now);
            if live * 4 * 2 < now { now } else { want.next_power_of_two() }
        };
        let (tree_bytes, voxel_bytes) = (size(live_tree, self.tree.size()).min(self.max_binding), size(live_voxels, self.voxels.size()).min(self.max_binding));
        if live_tree * 4 > tree_bytes || live_voxels * 4 > voxel_bytes {
            return false;
        }
        if tree_bytes != self.tree.size() {
            self.tree = storage(&self.device, "massif render tree", tree_bytes);
        }
        if voxel_bytes != self.voxels.size() {
            self.voxels = storage(&self.device, "massif render voxels", voxel_bytes);
        }
        self.bind_group = None;
        let (mut tree_at, mut voxel_at) = (0u32, 0u32);
        let mut keys: Vec<(i32, i32)> = self.residents.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let r = self.residents.get_mut(&key).unwrap();
            r.tree_at = tree_at;
            r.voxel_at = voxel_at;
            tree_at += r.chunk.tree.len() as u32;
            voxel_at += r.chunk.voxels.len().div_ceil(2) as u32;
        }
        for r in self.residents.values() {
            self.upload(r);
        }
        self.tree_used = tree_at;
        self.voxel_used = voxel_at;
        self.garbage = 0;
        true
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(IMAGE_VIEW_FORMAT), ..Default::default() });
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
        let image = self.image.as_ref()?;
        let [w, h] = image.size;
        if self.pyramid.dirty {
            if self.pyramid_buf.size() < self.pyramid.data.len() as u64 * 4 {
                self.pyramid_buf = storage(&self.device, "massif render pyramid", self.pyramid.data.len() as u64 * 4);
                self.bind_group = None;
            }
            self.queue.write_buffer(&self.pyramid_buf, 0, bytemuck::cast_slice(&self.pyramid.data));
            self.pyramid.dirty = false;
        }
        if self.bind_group.is_none() {
            let image = self.image.as_ref()?;
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif render"),
                layout: &self.layout,
                entries: &[
                    entry(0, &self.params),
                    entry(1, &self.grid),
                    entry(2, &self.records),
                    entry(3, &self.tree),
                    entry(4, &self.voxels),
                    entry(5, &self.pyramid_buf),
                    entry(6, &self.materials_buf),
                    entry(7, &image.accum),
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&image.storage) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&self.atlas_view) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Sampler(&self.sampler) },
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
        let sun = light.sun_direction();
        let params = Params {
            eye: [eye[0], eye[1], eye[2], tan],
            right: [basis.right.x, basis.right.y, basis.right.z, w as f32 / h as f32],
            up: [basis.up.x, basis.up.y, basis.up.z, 2.0 * tan / h as f32],
            forward: [basis.forward.x, basis.forward.y, basis.forward.z, light.exposure],
            sun: [sun.x, sun.y, sun.z, light.sun_radius.cos()],
            sun_light: [light.sun_color[0] * light.sun_strength / std::f32::consts::PI, light.sun_color[1] * light.sun_strength / std::f32::consts::PI, light.sun_color[2] * light.sun_strength / std::f32::consts::PI, 400.0],
            sky_zenith: [light.zenith[0], light.zenith[1], light.zenith[2], light.sky_strength],
            sky_horizon: [light.horizon[0], light.horizon[1], light.horizon[2], 0.0],
            size: [w, h, self.frame.wrapping_mul(2654435761).wrapping_add(1), self.samples],
            grid: [e.width as i32, e.depth as i32, e.height(), e.sections as i32],
            info: [self.stride(), self.pyramid.levels.len() as u32, light.bounces, coarse as u32],
            level_off,
            level_w,
        };
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("massif render") });
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

    /// Chunks in the scene.
    pub fn chunks(&self) -> usize {
        self.residents.len()
    }

    /// What the scene takes on the graphics card.
    pub fn memory(&self) -> Memory {
        let scene = self.grid.size() + self.records.size() + self.tree.size() + self.voxels.size() + self.pyramid_buf.size() + self.materials_buf.size();
        let textures = {
            let s = self.atlas.size();
            (s.width as u64 * s.height as u64 * 4 * s.depth_or_array_layers as u64) * 4 / 3
        };
        let image = self.image.as_ref().map_or(0, |i| i.size[0] as u64 * i.size[1] as u64 * (16 + 4));
        Memory { scene, textures, image }
    }

    /// Words of the scene in use (tree and voxels), live and replaced.
    pub fn words(&self) -> (u64, u64) {
        (self.tree_used as u64 + self.voxel_used as u64, self.garbage)
    }
}
