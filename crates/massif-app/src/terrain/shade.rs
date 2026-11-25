//! Lit pictures of a relief, as Gaea shows its nodes: a sun from the
//! north-west with soft shadows, ambient occlusion and sky light, the
//! ground in clay or in its colours, a mask laid over it in its socket's
//! colour; seen from above (thumbnails, the viewer's 2D mode) or from a
//! camera orbiting it (3D). Drawn on the graphics card by `shade.wgsl`;
//! without one, by the engine's simpler CPU shading.

use massif_terrain::gpu::Gpu;
use massif_terrain::kernel::Grid;
use massif_terrain::space::{Rect, Space};

/// How the ground is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Surface {
    /// Neutral clay, as Gaea's default look: the shape alone.
    #[default]
    Clay,
    /// The colours reaching the node (its Albedo), or tints by height when
    /// there are none.
    Colour,
}

/// Where the eye is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Camera {
    /// From straight above, over `view` (in samples of the relief: x0, z0,
    /// x1, z1; the whole relief is `[0, 0, w, h]`).
    Top { view: [f32; 4] },
    /// Orbiting the terrain's centre: turned `yaw` degrees from the south,
    /// looking down `pitch` degrees, `distance` times as far as the
    /// nearest the whole terrain shows from.
    Orbit { yaw: f32, pitch: f32, distance: f32 },
}

/// What a picture shows, besides the relief.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// Degrees clockwise from the north, and above the horizon.
    pub sun: [f32; 2],
    /// How much steeper the relief is drawn than it is.
    pub exaggeration: f32,
    pub camera: Camera,
    /// Lines every this many blocks (0: none).
    pub contours: f32,
    /// Water drawn below the sea level.
    pub water: bool,
    /// Rock on steep slopes in the height tints.
    pub rock_on_slopes: bool,
    /// Beyond the terrain in the 2D view, and in 3D without the sky (sRGB).
    pub back: [u8; 3],
    /// In 3D, the sky beyond the terrain (else `back`, plain).
    pub sky: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self { sun: [315.0, 40.0], exaggeration: 1.0, camera: Camera::Top { view: [0.0, 0.0, 1.0, 1.0] }, contours: 0.0, water: true, rock_on_slopes: true, back: [0, 0, 0], sky: true }
    }
}

/// A mask over the relief.
#[derive(Debug, Clone, Copy)]
pub struct Overlay<'a> {
    pub mask: &'a Grid,
    /// sRGB.
    pub color: [u8; 3],
    pub opacity: f32,
}

/// A relief to light: heights in blocks, and the colours of its ground
/// (three planes of linear light over the same samples) when known.
pub struct Relief<'a> {
    pub heights: &'a Grid,
    pub colors: Option<&'a Grid>,
    pub sea: f32,
}

/// How much steeper than true a relief reads best: Minecraft terrains are
/// wide and low, so that a 300-block range over 8000 blocks shows as flat.
pub fn auto_exaggeration(heights: &Grid, sea: f32) -> f32 {
    let (lo, hi) = heights.range();
    let side = heights.rect.w.max(heights.rect.h) as f32 * Space::spacing(heights.lod) as f32;
    let relief = (hi - lo.min(sea)).max(8.0);
    (side * 0.045 / relief).clamp(1.0, 10.0)
}

/// Vertical exaggeration for the 3D view: the relief stands about a fifth
/// of the terrain's side.
pub fn auto_exaggeration_3d(heights: &Grid, sea: f32) -> f32 {
    let (lo, hi) = heights.range();
    let side = heights.rect.w.max(heights.rect.h) as f32 * Space::spacing(heights.lod) as f32;
    let relief = (hi - lo.min(sea)).max(8.0);
    (side * 0.09 / relief).clamp(1.0, 16.0)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    w: u32,
    h: u32,
    ow: u32,
    oh: u32,
    spacing: f32,
    sea: f32,
    exag: f32,
    mode: u32,
    sun: [f32; 4],
    overlay: [f32; 4],
    cam_pos: [f32; 4],
    cam_fwd: [f32; 4],
    /// `w`: the bottom of the block in 3D.
    cam_right: [f32; 4],
    /// `w`: tan of half the field of view.
    cam_up: [f32; 4],
    flags: u32,
    hmin: f32,
    hmax: f32,
    contour: f32,
    back: [f32; 4],
    view: [f32; 4],
}

const SHADER: &str = include_str!("shade.wgsl");

fn linear(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    v.map(|x| x / l)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Towards the sun, from its bearing and height in degrees.
pub fn sun_direction(sun: [f32; 2]) -> [f32; 3] {
    let (az, el) = (sun[0].to_radians(), sun[1].to_radians());
    normalize([az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()])
}

/// The relief lit, `size` pixels (width, height), as sRGB RGBA bytes.
pub fn shade(gpu: Option<&Gpu>, relief: &Relief, overlay: Option<Overlay>, surface: Surface, look: &Look, size: [usize; 2]) -> Vec<u8> {
    let size = [size[0].max(1), size[1].max(1)];
    if let Some(gpu) = gpu
        && let Some(px) = shade_gpu(gpu, relief, overlay, surface, look, size)
    {
        return px;
    }
    shade_cpu(relief, overlay, surface, look, size)
}

fn shade_gpu(gpu: &Gpu, relief: &Relief, overlay: Option<Overlay>, surface: Surface, look: &Look, size: [usize; 2]) -> Option<Vec<u8>> {
    let pipeline = gpu.compute("terrain panel shading", SHADER, "shade_main")?;
    let g = relief.heights;
    let (w, h) = (g.rect.w, g.rect.h);
    if w < 2 || h < 2 {
        return None;
    }
    let spacing = Space::spacing(g.lod) as f32;
    let (lo, hi) = g.range();
    let exag = look.exaggeration.max(0.01);
    let mode = match (surface, relief.colors) {
        (Surface::Clay, _) => 0,
        (Surface::Colour, None) => 1,
        (Surface::Colour, Some(_)) => 2,
    };
    let s = sun_direction(look.sun);
    let mut flags = 0u32;
    let mut cam = ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
    let mut view = [0.0, 0.0, w as f32, h as f32];
    match look.camera {
        Camera::Top { view: v } => view = v,
        Camera::Orbit { yaw, pitch, distance } => {
            flags |= 1;
            let (sx, sz) = ((w - 1) as f32 * spacing, (h - 1) as f32 * spacing);
            let side = sx.max(sz);
            let bottom = (lo.min(relief.sea) * exag) - side * 0.035;
            let top = hi.max(relief.sea) * exag;
            let fov = 0.5f32;
            let (eye, fwd, right, up) = orbit(sx, sz, bottom, top, yaw, pitch, distance, size[0] as f32 / size[1] as f32, fov);
            cam = ([eye[0], eye[1], eye[2], 0.0], [fwd[0], fwd[1], fwd[2], 0.0], [right[0], right[1], right[2], bottom], [up[0], up[1], up[2], fov]);
        }
    }
    if overlay.is_some() {
        flags |= 2;
    }
    if look.contours > 0.0 {
        flags |= 4;
    }
    if look.water {
        flags |= 8;
    }
    if look.rock_on_slopes {
        flags |= 16;
    }
    if !look.sky {
        flags |= 32;
    }
    let (ocolor, opacity) = overlay.map_or(([0.0; 3], 0.0), |o| (o.color.map(linear), o.opacity));
    let u = Uniforms {
        w,
        h,
        ow: size[0] as u32,
        oh: size[1] as u32,
        spacing,
        sea: relief.sea,
        exag,
        mode,
        sun: [s[0], s[1], s[2], 1.9],
        overlay: [ocolor[0], ocolor[1], ocolor[2], opacity],
        cam_pos: cam.0,
        cam_fwd: cam.1,
        cam_right: cam.2,
        cam_up: cam.3,
        flags,
        hmin: lo,
        hmax: hi,
        contour: look.contours,
        back: [look.back[0] as f32 / 255.0, look.back[1] as f32 / 255.0, look.back[2] as f32 / 255.0, 1.0],
        view,
    };
    let device = gpu.device();
    let ubuf = gpu.uniform("terrain shading uniforms", bytemuck::bytes_of(&u));
    let n = (w * h) as usize;
    let heights = gpu.buffer("terrain shading heights", n as u64 * 4);
    gpu.queue().write_buffer(&heights, 0, bytemuck::cast_slice(&g.data[..n]));
    let colors = match relief.colors.filter(|c| c.rect == g.rect && c.data.len() >= 3 * n) {
        Some(c) => {
            let b = gpu.buffer("terrain shading colours", 3 * n as u64 * 4);
            gpu.queue().write_buffer(&b, 0, bytemuck::cast_slice(&c.data[..3 * n]));
            b
        }
        None => gpu.buffer("terrain shading colours", 16),
    };
    let mask = match overlay.filter(|o| o.mask.rect == g.rect && o.mask.data.len() >= n) {
        Some(o) => {
            let b = gpu.buffer("terrain shading mask", n as u64 * 4);
            gpu.queue().write_buffer(&b, 0, bytemuck::cast_slice(&o.mask.data[..n]));
            b
        }
        None => gpu.buffer("terrain shading mask", 16),
    };
    let out = gpu.buffer("terrain shading pixels", (size[0] * size[1]) as u64 * 4);
    let layout = pipeline.get_bind_group_layout(0);
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain shading"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: heights.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: colors.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: mask.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: out.as_entire_binding() },
        ],
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("terrain shading") });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("terrain shading"), timestamp_writes: None });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((size[0] as u32).div_ceil(16), (size[1] as u32).div_ceil(16), 1);
    }
    gpu.queue().submit([enc.finish()]);
    let words = gpu.download_buffers(&[&out]).pop()?;
    let mut px = Vec::with_capacity(size[0] * size[1] * 4);
    for v in words.iter().take(size[0] * size[1]) {
        px.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    (px.len() == size[0] * size[1] * 4).then_some(px)
}

/// The engine's CPU shading, resampled to `size`: a plainer picture, for
/// machines without a graphics card.
fn shade_cpu(relief: &Relief, overlay: Option<Overlay>, surface: Surface, look: &Look, size: [usize; 2]) -> Vec<u8> {
    let g = relief.heights;
    let colors = if surface == Surface::Colour { relief.colors } else { None };
    let mut px = massif_terrain::preview::shade_with(g, relief.sea, colors);
    if surface == Surface::Clay && colors.is_none() {
        // Clay: the lighting without the tints.
        let flat = Grid::filled(g.lod, g.rect, relief.sea + 30.0);
        let tint = massif_terrain::preview::shade(&flat, relief.sea);
        let base = tint.first().copied().unwrap_or([128; 3]);
        for p in px.iter_mut() {
            let l = (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / (base[0] as f32 * 0.3 + base[1] as f32 * 0.59 + base[2] as f32 * 0.11).max(1.0);
            *p = [(150.0 * l).min(255.0) as u8, (146.0 * l).min(255.0) as u8, (140.0 * l).min(255.0) as u8];
        }
    }
    if let Some(o) = overlay {
        massif_terrain::preview::overlay(&mut px, o.mask, o.color, o.opacity);
    }
    let (w, h) = (g.rect.w as usize, g.rect.h as usize);
    let view = match look.camera {
        Camera::Top { view } => view,
        Camera::Orbit { .. } => [0.0, 0.0, w as f32, h as f32],
    };
    let mut out = Vec::with_capacity(size[0] * size[1] * 4);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let sx = view[0] + (view[2] - view[0]) * (x as f32 + 0.5) / size[0] as f32;
            let sz = view[1] + (view[3] - view[1]) * (y as f32 + 0.5) / size[1] as f32;
            if sx < 0.0 || sz < 0.0 || sx >= w as f32 || sz >= h as f32 {
                out.extend_from_slice(&[look.back[0], look.back[1], look.back[2], 255]);
            } else {
                let p = px[sz as usize * w + sx as usize];
                out.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
    }
    out
}

/// The 3D viewer's camera over a block `sx` × `sz` blocks wide, from
/// `bottom` to `top`: its eye, and which ways are forward, right and up.
/// `distance` times the distance the whole block shows from at the
/// viewer's first angle (25° round, 34° up), kept as the view turns, so
/// that turning never comes closer nor goes away; `fov` the tangent of half
/// the vertical field.
#[allow(clippy::too_many_arguments)]
pub fn orbit(sx: f32, sz: f32, bottom: f32, top: f32, yaw: f32, pitch: f32, distance: f32, aspect: f32, fov: f32) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3]) {
    let side = sx.max(sz);
    let target = [sx * 0.5, (bottom + top) * 0.5, sz * 0.5];
    let dir = |yaw: f32, pitch: f32| {
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians().clamp(0.05, 1.5));
        [yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()]
    };
    let (back, reference) = (dir(yaw, pitch), dir(25.0, 34.0));
    let corners: Vec<[f32; 3]> = (0..8).map(|k| [if k & 1 == 0 { 0.0 } else { sx }, if k & 2 == 0 { bottom } else { top }, if k & 4 == 0 { 0.0 } else { sz }]).collect();
    let frame_from = |back: [f32; 3], d: f32| {
        let eye = [0, 1, 2].map(|k| target[k] + back[k] * d);
        let fwd = normalize([0, 1, 2].map(|k| target[k] - eye[k]));
        let right = normalize(cross(fwd, [0.0, 1.0, 0.0]));
        let up = cross(right, fwd);
        (eye, fwd, right, up)
    };
    let fits = |d: f32| {
        let (eye, fwd, right, up) = frame_from(reference, d);
        corners.iter().all(|c| {
            let v = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
            let z = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
            let x = (v[0] * right[0] + v[1] * right[1] + v[2] * right[2]) / (z * fov * aspect);
            let y = (v[0] * up[0] + v[1] * up[1] + v[2] * up[2]) / (z * fov);
            z > 0.0 && x.abs() <= 0.9 && y.abs() <= 0.86
        })
    };
    let (mut a, mut b) = (side * 0.05, side * 40.0);
    for _ in 0..40 {
        let m = (a + b) * 0.5;
        if fits(m) {
            b = m
        } else {
            a = m
        }
    }
    frame_from(back, b * distance.max(0.05))
}

/// The heights of a grid's rect at the same samples, for a grid whose
/// relief is unknown: flat a little above the sea.
pub fn flat_like(g: &Grid, sea: f32) -> Grid {
    Grid::filled(g.lod, Rect { ..g.rect }, sea + 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_match_the_shader_layout() {
        // WGSL lays U out with vec4s on 16 bytes: 176 bytes in all.
        assert_eq!(std::mem::size_of::<Uniforms>(), 176);
    }

    #[test]
    fn the_sun_comes_from_the_north_west() {
        let s = sun_direction([315.0, 40.0]);
        assert!(s[0] < 0.0 && s[2] < 0.0 && s[1] > 0.6);
    }
}
