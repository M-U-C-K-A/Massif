//! The image a 3D view draws, as egui shows it. A view draws in sRGB,
//! where colours blend as light adds up, and egui shows the image's bytes
//! as they are, since it works with gamma-encoded colours and an sRGB view
//! would decode them once more, darkening everything. Most graphics APIs
//! view the texture's bytes as Rgba8Unorm. OpenGL cannot view a texture in
//! another format, so there a small pass copies the image into a texture of
//! its own, encoding the colours back into the same bytes; and there the
//! multisampled image is kept until it is resolved.

use std::borrow::Cow;

/// The format egui reads the image's bytes in.
const PLAIN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Whether `device` draws with OpenGL, which views a texture in its own
/// format only.
fn opengl(device: &wgpu::Device) -> bool {
    device.adapter_info().backend == wgpu::Backend::Gl
}

/// The formats a view's sRGB image is made to be viewed in, besides its own.
pub fn view_formats(device: &wgpu::Device) -> &'static [wgpu::TextureFormat] {
    if opengl(device) { &[] } else { &[PLAIN] }
}

/// What becomes of a view's multisampled image once it is resolved into the
/// one egui shows: dropped, which spares memory traffic, except with
/// OpenGL, where wgpu drops it before resolving it and the image comes out
/// empty.
pub fn msaa_store(device: &wgpu::Device) -> wgpu::StoreOp {
    if opengl(device) { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard }
}

/// Wait until the graphics card is done with what it was given; whether it
/// is. OpenGL gives up waiting after a while, before a long image is drawn:
/// wait again then, up to two minutes in all.
pub fn wait(device: &wgpu::Device) -> bool {
    let started = std::time::Instant::now();
    loop {
        match device.poll(wgpu::PollType::wait_indefinitely()) {
            Ok(_) => return true,
            Err(wgpu::PollError::Timeout) if started.elapsed() < std::time::Duration::from_secs(120) => {}
            Err(_) => return false,
        }
    }
}

const SHADER: &str = r"
@group(0) @binding(0) var image: texture_2d<f32>;

// One triangle over the whole target.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

// The texel, decoded from sRGB as it is read, encoded again.
@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(image, vec2<i32>(at.xy), 0);
    let low = c.rgb * 12.92;
    let high = 1.055 * pow(c.rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(high, low, c.rgb <= vec3<f32>(0.0031308)), c.a);
}
";

/// The pass that copies an sRGB image into a plain one, on a device that
/// cannot view it so: `None` elsewhere.
pub struct Copier {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Copier {
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        opengl(device).then(|| Self::make(device))
    }

    fn make(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif plain image"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif plain image"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif plain image"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("massif plain image"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PLAIN,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }
}

/// The image egui shows of an sRGB texture.
pub struct Shown {
    /// What egui reads.
    pub view: wgpu::TextureView,
    /// Where the image is a copy: what the copy reads, and the copy.
    copy: Option<(wgpu::BindGroup, wgpu::Texture)>,
}

impl Shown {
    /// What egui shows of `image`, an sRGB texture made with
    /// [`view_formats`] and to be bound as a texture.
    pub fn of(device: &wgpu::Device, copier: Option<&Copier>, image: &wgpu::Texture) -> Self {
        let Some(copier) = copier else {
            return Self {
                view: image.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(PLAIN),
                    ..Default::default()
                }),
                copy: None,
            };
        };
        let plain = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("massif plain image"),
            size: image.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PLAIN,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif plain image"),
            layout: &copier.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&image.create_view(&wgpu::TextureViewDescriptor::default())),
            }],
        });
        Self {
            view: plain.create_view(&wgpu::TextureViewDescriptor::default()),
            copy: Some((source, plain)),
        }
    }

    /// Bring the copy up to date with the image drawn before in `encoder`,
    /// where the image is a copy.
    pub fn update(&self, copier: Option<&Copier>, encoder: &mut wgpu::CommandEncoder) {
        let (Some(copier), Some((source, _))) = (copier, &self.copy) else { return };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("massif plain image"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every texel is written.
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&copier.pipeline);
        pass.set_bind_group(0, source, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copy drawn where the device cannot view the image as plain bytes
    /// has the image's own bytes: every value of every channel, through
    /// sRGB and back. Runs on any graphics card (WGPU_BACKEND=gl tries
    /// OpenGL's), the copy made whatever the API.
    #[test]
    fn the_copy_keeps_every_byte() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipping");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).unwrap();
        let bytes: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v.wrapping_mul(7), v]).collect();
        let size = wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 };
        let image = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1024), rows_per_image: Some(1) };
        queue.write_texture(image.as_image_copy(), &bytes, layout, size);
        let copier = Copier::make(&device);
        let shown = Shown::of(&device, Some(&copier), &image);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 1024,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        shown.update(Some(&copier), &mut encoder);
        let (_, copy) = shown.copy.as_ref().expect("a copy");
        encoder.copy_texture_to_buffer(copy.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buffer, layout }, size);
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        assert!(wait(&device));
        let copied = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        let off: Vec<(usize, u8, u8)> = bytes.iter().zip(&copied).enumerate().filter(|(_, (a, b))| a != b).map(|(k, (a, b))| (k, *a, *b)).collect();
        assert!(off.is_empty(), "{} bytes differ, the first ones (index, image, copy): {:?}", off.len(), &off[..off.len().min(8)]);
    }
}
