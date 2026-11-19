//! The physical sky: the tables of Hillaire's atmosphere (see
//! `atmosphere.wgsl` and `sky.wgsl`), filled on the graphics card when the
//! sun or the air change, and read by the path tracer for the sky's light,
//! the sun's colour through the air, and the light the air scatters.

use bytemuck::{Pod, Zeroable};

pub const ATMOSPHERE: &str = include_str!("atmosphere.wgsl");
const PASSES: &str = include_str!("sky.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const TRANSMITTANCE: [u32; 2] = [256, 64];
const MULTI: [u32; 2] = [32, 32];
const SKY_VIEW: [u32; 2] = [192, 108];

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct SkyParams {
    sun: [f32; 4],
    air: [f32; 4],
}

/// What the sky's tables depend on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyState {
    /// Toward the sun.
    pub sun: [f32; 3],
    /// The air's scattering (1: the Earth's), the aerosols' (haze), and
    /// their anisotropy.
    pub rayleigh: f32,
    pub mie: f32,
    pub mie_g: f32,
    /// The observer's height over sea level, km.
    pub altitude: f32,
}

struct Table {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Table {
    fn new(device: &wgpu::Device, label: &str, [w, h]: [u32; 2]) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

/// The sky's tables and the passes that fill them.
pub struct Sky {
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    transmittance: Table,
    multi: Table,
    sky_view: Table,
    pipelines: [wgpu::ComputePipeline; 3],
    groups: [wgpu::BindGroup; 3],
    state: Option<SkyState>,
}

impl Sky {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("massif render sky"), source: wgpu::ShaderSource::Wgsl(format!("{ATMOSPHERE}\n{PASSES}").into()) });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: None, module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let pipelines = [pipeline("transmittance"), pipeline("multi"), pipeline("sky_view")];
        let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("massif render sky params"), size: size_of::<SkyParams>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif render sky"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let transmittance = Table::new(device, "massif render transmittance", TRANSMITTANCE);
        let multi = Table::new(device, "massif render multiple scattering", MULTI);
        let sky_view = Table::new(device, "massif render sky view", SKY_VIEW);
        // Each pass reads the tables before it and writes its own: their
        // layouts are the shader's (only the bindings each pass uses).
        let uniform = wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() };
        let groups = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif render transmittance"),
                layout: &pipelines[0].get_bind_group_layout(0),
                entries: &[uniform.clone(), wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&transmittance.view) }],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif render multiple scattering"),
                layout: &pipelines[1].get_bind_group_layout(0),
                entries: &[
                    uniform.clone(),
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&transmittance.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&multi.view) },
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("massif render sky view"),
                layout: &pipelines[2].get_bind_group_layout(0),
                entries: &[
                    uniform,
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&transmittance.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&multi.view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&sky_view.view) },
                ],
            }),
        ];
        Self { params, sampler, transmittance, multi, sky_view, pipelines, groups, state: None }
    }

    /// Fill the tables again if `state` differs from what they hold: the
    /// passes go into `encoder`, before the frame's. True when they did.
    pub fn update(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, state: SkyState) -> bool {
        if self.state == Some(state) {
            return false;
        }
        // The air changed: all three; only the sun or the observer: the
        // last two (the transmittance does not depend on them).
        let air_changed = self.state.is_none_or(|s| (s.rayleigh, s.mie, s.mie_g) != (state.rayleigh, state.mie, state.mie_g));
        self.state = Some(state);
        let params = SkyParams { sun: [state.sun[0], state.sun[1], state.sun[2], 0.0], air: [state.rayleigh, state.mie, state.mie_g, state.altitude] };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("massif render sky"), timestamp_writes: None });
        for (k, size) in [TRANSMITTANCE, MULTI, SKY_VIEW].into_iter().enumerate() {
            if k == 0 && !air_changed {
                continue;
            }
            pass.set_pipeline(&self.pipelines[k]);
            pass.set_bind_group(0, &self.groups[k], &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        true
    }

    pub fn transmittance(&self) -> &wgpu::TextureView {
        &self.transmittance.view
    }

    pub fn multi(&self) -> &wgpu::TextureView {
        &self.multi.view
    }

    pub fn sky_view(&self) -> &wgpu::TextureView {
        &self.sky_view.view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Bytes the tables take.
    pub fn bytes(&self) -> u64 {
        [&self.transmittance, &self.multi, &self.sky_view].iter().map(|t| t.texture.size().width as u64 * t.texture.size().height as u64 * 8).sum()
    }
}
