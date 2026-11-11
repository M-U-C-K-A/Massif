//! The terrain graph's 3D viewport: the result of a node as a smooth,
//! shaded relief, the way Gaea shows it (no blocks yet: they come when the
//! graph is converted to the map).
//!
//! The relief is a grid mesh whose heights the vertex shader reads from a
//! texture, coloured by the graph's colours and lit by a sun with soft cast
//! shadows and ambient occlusion (both worked out on the CPU with the
//! preview), a sky and, optionally, a sea. The sides of the relief are
//! closed down to a base, like a diorama.

use crate::theme::Palette;
use bytemuck::{Pod, Zeroable};
use egui::{Color32, PointerButton, Rect, Sense, Ui, pos2};
use glam::{Mat4, Vec3, Vec4};
use rayon::prelude::*;
use std::borrow::Cow;
use std::sync::Arc;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
const FOV_Y: f32 = 40.0 * std::f32::consts::PI / 180.0;
/// Direction the sun shines from: north-west, 35° up.
pub const SUN_AZIMUTH: f32 = 315.0;
pub const SUN_ELEVATION: f32 = 35.0;

/// What the viewport shows: a relief, its colours and its light.
#[derive(Debug, Clone)]
pub struct ViewData {
    /// Samples per side.
    pub n: usize,
    /// Heights in blocks.
    pub heights: Vec<f32>,
    /// sRGB colours.
    pub colors: Vec<[u8; 4]>,
    /// Sunlight (0: in shadow) and openness to the sky, 0..255.
    pub light: Vec<[u8; 2]>,
    /// Blocks per side.
    pub size: f32,
    /// Vertical exaggeration the light was worked out for.
    pub exaggeration: f32,
}

/// The direction toward the sun.
pub fn sun_dir() -> Vec3 {
    let (a, e) = (SUN_AZIMUTH.to_radians(), SUN_ELEVATION.to_radians());
    // Azimuth 0: north (−z), 90: east (+x).
    Vec3::new(a.sin() * e.cos(), e.sin(), -a.cos() * e.cos()).normalize()
}

/// Sunlight and sky light of a relief: soft cast shadows marched toward the
/// sun, and occlusion from how far the ground around rises above each
/// sample, at a few scales.
pub fn lighting(heights: &[f32], n: usize, spacing: f32, exaggeration: f32) -> Vec<[u8; 2]> {
    let sun = sun_dir();
    let horizontal = Vec3::new(sun.x, 0.0, sun.z).normalize_or_zero();
    let rise = sun.y / sun.x.hypot(sun.z).max(1e-4);
    let h = |i: f32, j: f32| -> f32 {
        let (i, j) = (i.clamp(0.0, (n - 1) as f32), j.clamp(0.0, (n - 1) as f32));
        let (i0, j0) = (i.floor() as usize, j.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(n - 1), (j0 + 1).min(n - 1));
        let (fi, fj) = (i - i0 as f32, j - j0 as f32);
        let a = heights[j0 * n + i0] + (heights[j0 * n + i1] - heights[j0 * n + i0]) * fi;
        let b = heights[j1 * n + i0] + (heights[j1 * n + i1] - heights[j1 * n + i0]) * fi;
        (a + (b - a) * fj) * exaggeration
    };
    // Openness: mean heights around, at three scales.
    let means: Vec<Vec<f32>> = [2usize, 6, 16]
        .iter()
        .map(|&r| box_mean(heights, n, (r as f32 * 512.0 / n as f32).round().max(1.0) as usize))
        .collect();
    (0..n * n)
        .into_par_iter()
        .map(|c| {
            let (i, j) = ((c % n) as f32, (c / n) as f32);
            let h0 = heights[c] * exaggeration + 0.05 * spacing;
            // March toward the sun; keep the narrowest clearance seen.
            let mut lit = 1.0f32;
            let mut t = 1.0f32;
            while t < n as f32 {
                let (x, z) = (i + horizontal.x * t, j + horizontal.z * t);
                if x < 0.0 || z < 0.0 || x > (n - 1) as f32 || z > (n - 1) as f32 {
                    break;
                }
                let ray = h0 + rise * t * spacing;
                let ground = h(x, z);
                // Soft: how far above the ground the ray passes, for its distance.
                lit = lit.min(((ray - ground) / (t * spacing) * 6.0 + 0.5).clamp(0.0, 1.0));
                if lit <= 0.0 {
                    break;
                }
                t += 1.0 + t * 0.03;
            }
            let mut open = 0.0;
            for (k, m) in means.iter().enumerate() {
                let r = [2.0f32, 6.0, 16.0][k] * 512.0 / n as f32 * spacing;
                let above = ((m[c] - heights[c]) * exaggeration).max(0.0);
                open += (above / r).atan() / std::f32::consts::FRAC_PI_2;
            }
            let ao = (1.0 - open / 3.0 * 2.2).clamp(0.0, 1.0);
            [(lit * 255.0) as u8, (ao * 255.0) as u8]
        })
        .collect()
}

/// The mean of the `(2r + 1)²` samples around each sample.
fn box_mean(v: &[f32], n: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; n * n];
        let lines: Vec<Vec<f32>> = (0..n)
            .into_par_iter()
            .map(|line| {
                let at = |t: usize| if horizontal { src[line * n + t] } else { src[t * n + line] };
                let mut res = vec![0f32; n];
                let (mut sum, mut count) = (0f32, 0f32);
                for t in 0..n.min(r + 1) {
                    sum += at(t);
                    count += 1.0;
                }
                for (t, o) in res.iter_mut().enumerate() {
                    *o = sum / count;
                    if t + r + 1 < n {
                        sum += at(t + r + 1);
                        count += 1.0;
                    }
                    if t >= r {
                        sum -= at(t - r);
                        count -= 1.0;
                    }
                }
                res
            })
            .collect();
        for (line, values) in lines.into_iter().enumerate() {
            for (t, val) in values.into_iter().enumerate() {
                if horizontal {
                    out[line * n + t] = val;
                } else {
                    out[t * n + line] = val;
                }
            }
        }
        out
    };
    pass(&pass(v, true), false)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    /// Samples per side, blocks per side, exaggeration, height of the base.
    grid: [f32; 4],
    /// Sky colours: zenith and horizon; sea level (w = 1 to show it).
    zenith: [f32; 4],
    horizon: [f32; 4],
    sea: [f32; 4],
    /// tan(fov/2) × aspect, tan(fov/2), unused, unused.
    cam: [f32; 4],
    cam_f: [f32; 4],
    cam_s: [f32; 4],
    cam_u: [f32; 4],
}

const SHADER: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    grid: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    sea: vec4<f32>,
    cam: vec4<f32>,
    cam_f: vec4<f32>,
    cam_s: vec4<f32>,
    cam_u: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var heights: texture_2d<f32>;
@group(0) @binding(2) var colors: texture_2d<f32>;
@group(0) @binding(3) var light: texture_2d<f32>;
@group(0) @binding(4) var smooth_sampler: sampler;

fn height_at(i: i32, j: i32) -> f32 {
    let n = i32(u.grid.x);
    let c = clamp(vec2<i32>(i, j), vec2<i32>(0), vec2<i32>(n - 1));
    return textureLoad(heights, c, 0).r * u.grid.z;
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) side: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> Out {
    let n = i32(u.grid.x);
    let spacing = u.grid.y / f32(n);
    var i: i32;
    var j: i32;
    var y: f32;
    var normal: vec3<f32>;
    var side = 0.0;
    let nn = u32(n * n);
    if (vi < nn) {
        i = i32(vi) % n;
        j = i32(vi) / n;
        y = height_at(i, j);
        let dx = (height_at(i + 1, j) - height_at(i - 1, j)) / (2.0 * spacing);
        let dz = (height_at(i, j + 1) - height_at(i, j - 1)) / (2.0 * spacing);
        normal = normalize(vec3<f32>(-dx, 1.0, -dz));
    } else {
        // The sides, down to the base: four edges of n top and bottom
        // vertices each.
        let k = vi - nn;
        let s = k / u32(2 * n);
        let t = i32((k % u32(2 * n)) / 2u);
        let bottom = (k % 2u) == 1u;
        switch s {
            case 0u: { i = t; j = 0; normal = vec3<f32>(0.0, 0.0, -1.0); }
            case 1u: { i = n - 1; j = t; normal = vec3<f32>(1.0, 0.0, 0.0); }
            case 2u: { i = t; j = n - 1; normal = vec3<f32>(0.0, 0.0, 1.0); }
            default: { i = 0; j = t; normal = vec3<f32>(-1.0, 0.0, 0.0); }
        }
        y = select(height_at(i, j), u.grid.w, bottom);
        side = 1.0;
    }
    let half = u.grid.y * 0.5;
    let p = vec3<f32>((f32(i) + 0.5) * spacing - half, y, (f32(j) + 0.5) * spacing - half);
    var out: Out;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.uv = (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(n);
    out.normal = normal;
    out.side = side;
    return out;
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn sky(d: vec3<f32>) -> vec3<f32> {
    let t = clamp(d.y * 1.6 + 0.1, 0.0, 1.0);
    return mix(u.horizon.rgb, u.zenith.rgb, sqrt(t));
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    var base = textureSample(colors, smooth_sampler, in.uv).rgb;
    let l = textureSample(light, smooth_sampler, in.uv).rg;
    var shadow = l.r;
    var open = l.g;
    if (in.side > 0.5) {
        // The cut through the ground: earth darkening with depth.
        let depth = clamp((in.world.y - u.grid.w) / max(u.grid.y * 0.05, 1.0), 0.0, 1.0);
        base = mix(vec3<f32>(0.10, 0.08, 0.065), base * 0.55, depth);
        shadow = 1.0;
        open = 0.8;
    }
    let sun = max(dot(n, u.sun.xyz), 0.0) * shadow;
    let sky_light = (0.55 + 0.45 * n.y) * open;
    let warm = vec3<f32>(1.0, 0.93, 0.84);
    let cool = vec3<f32>(0.62, 0.72, 0.88);
    var c = base * (warm * sun * 1.2 + cool * sky_light * 0.5);
    // Haze with distance.
    let dist = distance(in.world, u.eye.xyz);
    let f = clamp(dist / (u.grid.y * 3.5), 0.0, 1.0);
    c = mix(c, sky(normalize(in.world - u.eye.xyz)) * 0.9, f * f * 0.5);
    // A touch less saturated, as seen through air.
    c = mix(vec3<f32>(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722))), c, 0.86);
    return vec4<f32>(aces(c * 0.95), 1.0);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: SkyOut;
    out.ndc = uv * 2.0 - 1.0;
    out.clip = vec4<f32>(out.ndc, 1.0, 1.0);
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let d = normalize(u.cam_f.xyz + u.cam_s.xyz * in.ndc.x * u.cam.x + u.cam_u.xyz * in.ndc.y * u.cam.y);
    return vec4<f32>(sky(d), 1.0);
}

struct SeaOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_sea(@builtin(vertex_index) vi: u32) -> SeaOut {
    let corner = vec2<f32>(f32((0x16u >> vi) & 1u), f32((0x34u >> vi) & 1u)) - 0.5;
    let p = vec3<f32>(corner.x * u.grid.y, u.sea.x, corner.y * u.grid.y);
    var out: SeaOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    return out;
}

@fragment
fn fs_sea(in: SeaOut) -> @location(0) vec4<f32> {
    let v = normalize(u.eye.xyz - in.world);
    let fresnel = pow(1.0 - max(v.y, 0.0), 4.0);
    let deep = vec3<f32>(0.02, 0.09, 0.16);
    let c = mix(deep, sky(reflect(-v, vec3<f32>(0.0, 1.0, 0.0))), 0.25 + 0.6 * fresnel);
    return vec4<f32>(aces(c * 1.1), 0.78);
}
"#;

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    #[cfg_attr(not(test), allow(dead_code))]
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same image as plain bytes, for egui.
    display_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// The relief on the GPU.
struct Relief {
    n: usize,
    group: wgpu::BindGroup,
    indices: wgpu::Buffer,
    count: u32,
}

/// GPU side of the viewport.
pub struct TerrainRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    relief_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    sea_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    relief: Option<Relief>,
    target: Option<Target>,
}

impl TerrainRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif graph view"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let texture = |binding: u32, filterable: bool, sampled_float: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: if sampled_float {
                    wgpu::TextureSampleType::Float { filterable }
                } else {
                    wgpu::TextureSampleType::Float { filterable: false }
                },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("massif graph view"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1, false, false),
                texture(2, true, true),
                texture(3, true, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif graph view"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, vs: &str, fs: &str, blend: wgpu::BlendState, depth_write: bool, compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let relief_pipeline = pipeline("massif graph relief", "vs_main", "fs_main", wgpu::BlendState::REPLACE, true, wgpu::CompareFunction::Less);
        let sky_pipeline = pipeline("massif graph sky", "vs_sky", "fs_sky", wgpu::BlendState::REPLACE, false, wgpu::CompareFunction::LessEqual);
        let sea_pipeline = pipeline("massif graph sea", "vs_sea", "fs_sea", wgpu::BlendState::ALPHA_BLENDING, false, wgpu::CompareFunction::Less);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("massif graph view"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("massif graph view"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            relief_pipeline,
            sky_pipeline,
            sea_pipeline,
            layout,
            uniforms,
            sampler,
            relief: None,
            target: None,
        }
    }

    /// Put a relief on the GPU.
    pub fn upload(&mut self, data: &ViewData) {
        let n = data.n as u32;
        let texture = |label: &str, format: wgpu::TextureFormat, bytes: &[u8], texel: u32| {
            let t = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: n,
                    height: n,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                t.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(n * texel),
                    rows_per_image: Some(n),
                },
                wgpu::Extent3d {
                    width: n,
                    height: n,
                    depth_or_array_layers: 1,
                },
            );
            t.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let heights = texture("massif graph heights", wgpu::TextureFormat::R32Float, bytemuck::cast_slice(&data.heights), 4);
        let colors = texture("massif graph colours", wgpu::TextureFormat::Rgba8UnormSrgb, bytemuck::cast_slice(&data.colors), 4);
        let light = texture("massif graph light", wgpu::TextureFormat::Rg8Unorm, bytemuck::cast_slice(&data.light), 2);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("massif graph view"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&heights),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&colors),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&light),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        // Reuse the triangles while the size stays.
        let (indices, count) = match self.relief.take() {
            Some(r) if r.n == data.n => (r.indices, r.count),
            _ => {
                let idx = grid_indices(data.n);
                let buffer = wgpu::util::DeviceExt::create_buffer_init(
                    &self.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("massif graph triangles"),
                        contents: bytemuck::cast_slice(&idx),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                );
                (buffer, idx.len() as u32)
            }
        };
        self.relief = Some(Relief {
            n: data.n,
            group,
            indices,
            count,
        });
    }

    pub fn ensure_target(&mut self, size: [u32; 2]) -> bool {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        // The image egui shows is read as plain bytes: egui works with
        // gamma-encoded colours, and an sRGB view would decode them once
        // more, darkening everything.
        let texture = |label: &str, format, samples, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: if format == COLOR_FORMAT { &[wgpu::TextureFormat::Rgba8Unorm] } else { &[] },
            })
        };
        let msaa = texture("massif graph msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolve = texture(
            "massif graph image",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture("massif graph depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(Target {
            size,
            msaa: view(&msaa),
            resolve_view: view(&resolve),
            display_view: resolve.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8Unorm),
                ..Default::default()
            }),
            resolve,
            depth: view(&depth),
        });
        true
    }

    pub fn image_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.display_view)
    }

    fn render(&mut self, u: &Uniforms, sea: bool) {
        let Some(target) = &self.target else { return };
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(u));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("massif graph view"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("massif graph view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: u.horizon[0] as f64,
                            g: u.horizon[1] as f64,
                            b: u.horizon[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(relief) = &self.relief {
                pass.set_bind_group(0, &relief.group, &[]);
                pass.set_pipeline(&self.relief_pipeline);
                pass.set_index_buffer(relief.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..relief.count, 0, 0..1);
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
                if sea {
                    pass.set_pipeline(&self.sea_pipeline);
                    pass.draw(0..6, 0..1);
                }
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Copy the rendered image back to the CPU (RGBA, row-major). Blocks.
    #[cfg(test)]
    pub fn read_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.target.as_ref()?;
        let [w, h] = target.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.resolve,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
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
}

/// Triangles of an `n` × `n` grid, then of its four sides (see the vertex
/// shader for how side vertices are numbered).
fn grid_indices(n: usize) -> Vec<u32> {
    let n32 = n as u32;
    let mut idx = Vec::with_capacity((n - 1) * (n - 1) * 6 + 4 * (n - 1) * 6);
    for j in 0..n32 - 1 {
        for i in 0..n32 - 1 {
            let a = j * n32 + i;
            let (b, c, d) = (a + 1, a + n32, a + n32 + 1);
            // Counter-clockwise from above.
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let base = n32 * n32;
    for s in 0..4u32 {
        for t in 0..n32 - 1 {
            let top0 = base + s * 2 * n32 + t * 2;
            let (bot0, top1, bot1) = (top0 + 1, top0 + 2, top0 + 3);
            idx.extend_from_slice(&[top0, bot0, top1, top1, bot0, bot1]);
        }
    }
    idx
}

/// An orbit camera around the middle of the relief.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    /// Distance, in sides of the relief.
    pub distance: f32,
    /// Point looked at, in blocks from the middle.
    pub target: [f32; 3],
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.58,
            distance: 1.02,
            target: [0.0, 0.0, 0.0],
        }
    }
}

/// The viewport editor.
pub struct TerrainView {
    renderer: Option<TerrainRenderer>,
    texture: Option<egui::TextureId>,
    pub camera: OrbitCamera,
    /// What is shown, and its revision.
    data: Option<Arc<ViewData>>,
    uploaded: Option<usize>,
    pub exaggeration: f32,
    /// Show the sea at this level, if any.
    pub sea: Option<f32>,
    pub show_sea: bool,
    /// Range of heights shown, for framing.
    range: (f32, f32),
    last: Option<[f32; 4]>,
}

impl Default for TerrainView {
    fn default() -> Self {
        Self {
            renderer: None,
            texture: None,
            camera: OrbitCamera::default(),
            data: None,
            uploaded: None,
            exaggeration: 2.0,
            sea: None,
            show_sea: false,
            range: (0.0, 1.0),
            last: None,
        }
    }
}

impl TerrainView {
    /// Show a new relief.
    pub fn set(&mut self, data: Arc<ViewData>) {
        let (lo, hi) = data.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        self.range = (lo, hi);
        self.data = Some(data);
        self.uploaded = None;
    }

    pub fn has_data(&self) -> bool {
        self.data.is_some()
    }

    /// The last rendered image, read back from the GPU.
    #[cfg(test)]
    pub fn read_back(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.renderer.as_ref()?.read_image()
    }

    fn uniforms(&self, data: &ViewData, aspect: f32, pal: &Palette) -> Uniforms {
        let size = data.size.max(1.0);
        let ex = self.exaggeration;
        let (lo, hi) = self.range;
        let mid = (lo + hi) * 0.5 * ex;
        let target = Vec3::new(self.camera.target[0], mid + self.camera.target[1], self.camera.target[2]);
        let (sy, cy) = self.camera.yaw.sin_cos();
        let (sp, cp) = self.camera.pitch.sin_cos();
        let d = self.camera.distance * size;
        let eye = target + Vec3::new(d * cp * sy, d * sp, d * cp * cy);
        let near = (d * 0.002).max(0.5);
        let far = d + size * 4.0;
        let proj = {
            let f = 1.0 / (FOV_Y * 0.5).tan();
            let r = far / (near - far);
            Mat4::from_cols(
                Vec4::new(f / aspect, 0.0, 0.0, 0.0),
                Vec4::new(0.0, f, 0.0, 0.0),
                Vec4::new(0.0, 0.0, r, -1.0),
                Vec4::new(0.0, 0.0, r * near, 0.0),
            )
        };
        let fwd = (target - eye).normalize();
        let s = fwd.cross(Vec3::Y).normalize();
        let up = s.cross(fwd);
        let view = Mat4::from_cols(
            Vec4::new(s.x, up.x, -fwd.x, 0.0),
            Vec4::new(s.y, up.y, -fwd.y, 0.0),
            Vec4::new(s.z, up.z, -fwd.z, 0.0),
            Vec4::new(-s.dot(eye), -up.dot(eye), fwd.dot(eye), 1.0),
        );
        let sun = sun_dir();
        let lin = |c: [u8; 3]| [c[0], c[1], c[2]].map(|v| (v as f32 / 255.0).powf(2.2));
        let (zenith, horizon) = if pal.dark {
            (lin([30, 34, 42]), lin([74, 80, 92]))
        } else {
            (lin([120, 160, 214]), lin([214, 224, 238]))
        };
        let tan = (FOV_Y * 0.5).tan();
        Uniforms {
            view_proj: (proj * view).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 1.0],
            sun: [sun.x, sun.y, sun.z, 0.0],
            grid: [data.n as f32, size, ex, lo * ex - size * 0.03],
            zenith: [zenith[0], zenith[1], zenith[2], 1.0],
            horizon: [horizon[0], horizon[1], horizon[2], 1.0],
            sea: [self.sea.unwrap_or(0.0) * ex, 0.0, 0.0, (self.show_sea && self.sea.is_some()) as u8 as f32],
            cam: [tan * aspect, tan, 0.0, 0.0],
            cam_f: [fwd.x, fwd.y, fwd.z, 0.0],
            cam_s: [s.x, s.y, s.z, 0.0],
            cam_u: [up.x, up.y, up.z, 0.0],
        }
    }

    /// Draw the viewport into `rect`. Returns whether the light must be
    /// worked out again (the exaggeration changed).
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) -> bool {
        let pal = Palette::of(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let Some(state) = frame.wgpu_render_state() else {
            painter.rect_filled(rect, 0, pal.canvas);
            return false;
        };
        let mut renderer = self.renderer.take().unwrap_or_else(|| TerrainRenderer::new(&state.device, &state.queue));
        // Camera: left drag turns, right or middle drag moves, the wheel or
        // a pinch comes closer.
        if response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.006).clamp(0.05, 1.5);
        }
        if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
            let d = response.drag_delta();
            let size = self.data.as_ref().map_or(1000.0, |v| v.size);
            let k = self.camera.distance * size * 0.0016;
            let (s, c) = self.camera.yaw.sin_cos();
            self.camera.target[0] -= (d.x * c + d.y * s) * k;
            self.camera.target[2] -= (-d.x * s + d.y * c) * k;
        }
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y));
            let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
            self.camera.distance = (self.camera.distance * factor).clamp(0.05, 6.0);
        }
        if response.double_clicked() {
            self.camera = OrbitCamera {
                yaw: self.camera.yaw,
                ..OrbitCamera::default()
            };
        }
        let ppp = ui.ctx().pixels_per_point();
        let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let mut fresh = false;
        if renderer.ensure_target(px) {
            let view = renderer.image_view().expect("target exists");
            let mut egui_renderer = state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(&state.device, view, wgpu::FilterMode::Linear, id),
                None => self.texture = Some(egui_renderer.register_native_texture(&state.device, view, wgpu::FilterMode::Linear)),
            }
            fresh = true;
        }
        let Some(data) = self.data.clone() else {
            painter.rect_filled(rect, 0, pal.canvas);
            self.renderer = Some(renderer);
            return false;
        };
        if self.uploaded != Some(Arc::as_ptr(&data) as usize) {
            renderer.upload(&data);
            self.uploaded = Some(Arc::as_ptr(&data) as usize);
            fresh = true;
        }
        let u = self.uniforms(&data, rect.width() / rect.height().max(1.0), &pal);
        let key = [self.camera.yaw, self.camera.pitch, self.camera.distance, self.exaggeration + self.camera.target[0] * 1e-3 + self.camera.target[2] * 1e-6];
        if fresh || self.last != Some(key) {
            let sea = self.show_sea && self.sea.is_some();
            renderer.render(&u, sea);
            self.last = Some(key);
        }
        self.renderer = Some(renderer);
        if let Some(id) = self.texture {
            painter.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        (data.exaggeration - self.exaggeration).abs() > 1e-3
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_have_two_triangles_per_cell_and_closed_sides() {
        let idx = grid_indices(4);
        assert_eq!(idx.len(), 9 * 6 + 4 * 3 * 6);
        assert!(idx.iter().all(|&i| i < 16 + 4 * 2 * 4));
    }

    #[test]
    fn shadows_fall_away_from_the_sun() {
        // A wall across the middle, running north-south: the sun from the
        // north-west lights its west side and shades the ground east of it.
        let n = 64;
        let heights: Vec<f32> = (0..n * n).map(|c| if (30..34).contains(&(c % n)) { 60.0 } else { 0.0 }).collect();
        let light = lighting(&heights, n, 4.0, 1.0);
        let lit = |i: usize, j: usize| light[j * n + i][0];
        assert!(lit(20, 32) > 200, "west of the wall is in the sun");
        assert!(lit(36, 32) < 60, "just east of the wall is in its shadow");
        // Hollows are less open to the sky than tops.
        assert!(light[32 * n + 36][1] < light[32 * n + 31][1]);
    }
}
