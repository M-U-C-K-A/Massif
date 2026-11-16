//! The terrain graph's 3D viewport: the result of a node as a smooth relief
//! under a sky, the way Gaea shows it, or a scene rendered in Blender (no
//! blocks yet: they come when the graph is converted to the map).
//!
//! The relief is a grid mesh whose heights the vertex shader reads from a
//! texture, coloured by the graph's colours. Everything about the light is
//! worked out on the GPU, so it follows the sun as it turns: shadows cast
//! by the relief (marched toward the sun through the height texture), a
//! sky whose colours follow the sun's height (blue by day, warm at sunset)
//! with its disc, haze with distance, volumetric clouds (a layer of 3D noise
//! the view rays march through, lit by the sun, casting shadows on the
//! ground) and a sea whose colour follows its depth; how much of the sky
//! each point sees, from the heights around it; rock drawn in the world's
//! own space on steep ground, forest crowns on a forest. The sides of the
//! relief are closed down to a base, like a diorama.

use crate::theme::{self, Icon, Palette, RichTextExt};
use bytemuck::{Pod, Zeroable};
use egui::{Color32, PointerButton, Rect, RichText, Sense, Ui, pos2};
use glam::{Mat4, Vec3, Vec4};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const FOV_Y: f32 = 40.0 * std::f32::consts::PI / 180.0;
/// Views narrower than this (width over height) see as wide as it.
const WIDE: f32 = 1.9;

/// How the viewport renders the relief.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    /// Where the sun is, in degrees: 0 north, 90 east.
    pub sun_azimuth: f32,
    /// Degrees above the horizon.
    pub sun_elevation: f32,
    pub sun_intensity: f32,
    /// Heights are drawn this many times as high.
    pub exaggeration: f32,
    pub clouds: bool,
    /// Share of the sky clouds cover (0..1).
    pub cloud_cover: f32,
    /// Height of the cloud base and thickness of the layer, in blocks.
    pub cloud_base: f32,
    pub cloud_thickness: f32,
    pub cloud_density: f32,
    /// Size of the clouds, in blocks.
    pub cloud_size: f32,
    /// Moves the clouds around.
    pub cloud_seed: u32,
    /// Haze with distance (0..1).
    pub haze: f32,
    /// Show the sea at the world's sea level.
    pub sea: bool,
    pub exposure: f32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            sun_azimuth: 300.0,
            sun_elevation: 30.0,
            sun_intensity: 1.0,
            exaggeration: 1.0,
            clouds: false,
            cloud_cover: 0.45,
            cloud_base: 200.0,
            cloud_thickness: 90.0,
            cloud_density: 1.0,
            cloud_size: 500.0,
            cloud_seed: 1,
            haze: 0.3,
            sea: false,
            exposure: 1.0,
        }
    }
}

impl RenderSettings {
    /// The direction toward the sun.
    pub fn sun_dir(&self) -> Vec3 {
        let (a, e) = (self.sun_azimuth.to_radians(), self.sun_elevation.to_radians());
        // Azimuth 0: north (−z), 90: east (+x).
        Vec3::new(a.sin() * e.cos(), e.sin(), -a.cos() * e.cos()).normalize()
    }

    /// Linear colours of the sky overhead and at the horizon, and of the
    /// sunlight, for the sun's height: blue by day, warm low on the horizon,
    /// dark at night.
    fn sky(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let s = self.sun_elevation.to_radians().sin();
        let smooth = |a: f32, b: f32, x: f32| {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let day = smooth(-0.1, 0.18, s);
        let golden = 1.0 - smooth(0.0, 0.38, s);
        let mix = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t);
        let zenith = mix([0.008, 0.012, 0.03], [0.09, 0.22, 0.58], day);
        let horizon = mix([0.016, 0.02, 0.035], mix([0.52, 0.66, 0.86], [0.95, 0.52, 0.28], golden * 0.8), day);
        let sun = mix([1.0, 0.97, 0.92], [1.0, 0.52, 0.24], golden).map(|v| v * 1.6 * self.sun_intensity * smooth(-0.03, 0.06, s));
        (zenith, horizon, sun)
    }
}

/// What the viewport shows: a relief, its colours and its openness to the
/// sky.
#[derive(Debug, Clone)]
pub struct ViewData {
    /// Samples per side.
    pub n: usize,
    /// Heights in blocks.
    pub heights: Vec<f32>,
    /// sRGB colours.
    pub colors: Vec<[u8; 4]>,
    /// Unused, then openness to the sky, 0..255.
    pub light: Vec<[u8; 2]>,
    /// Blocks per side.
    pub size: f32,
    /// Vertical exaggeration the occlusion was worked out for.
    pub exaggeration: f32,
    /// Rock over the ground (floating islands, overhangs), two at most.
    pub slabs: Vec<Slab>,
    /// The level of the water of each sample, in blocks (far under the
    /// ground where there is none): rivers and lakes. Empty: no water but
    /// the sea.
    pub water: Vec<f32>,
}

/// Rock between two heights over the ground, at the relief's resolution.
#[derive(Debug, Clone)]
pub struct Slab {
    /// Top and bottom of each sample, in blocks; equal where there is none.
    pub top_bottom: Vec<[f32; 2]>,
    /// sRGB colours of its top.
    pub colors: Vec<[u8; 4]>,
}

/// How many slabs the viewport draws.
pub const MAX_SLABS: usize = 2;

/// How open to the sky every sample is: how far the ground around rises
/// above it, at a few scales.
pub fn occlusion(heights: &[f32], n: usize, spacing: f32, exaggeration: f32) -> Vec<[u8; 2]> {
    let radii = [2.0f32, 6.0, 16.0];
    let means: Vec<Vec<f32>> = radii
        .iter()
        .map(|&r| box_mean(heights, n, (r * 512.0 / n as f32).round().max(1.0) as usize))
        .collect();
    (0..n * n)
        .into_par_iter()
        .map(|c| {
            let mut open = 0.0;
            for (k, m) in means.iter().enumerate() {
                let r = radii[k] * 512.0 / n as f32 * spacing;
                let above = ((m[c] - heights[c]) * exaggeration).max(0.0);
                open += (above / r).atan() / std::f32::consts::FRAC_PI_2;
            }
            [255, ((1.0 - open / 3.0 * 2.2).clamp(0.0, 1.0) * 255.0) as u8]
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
    /// Eye; w: steps through the clouds.
    eye: [f32; 4],
    /// Toward the sun.
    sun: [f32; 4],
    /// Sunlight; w: steps of the relief's shadows.
    sun_color: [f32; 4],
    /// Samples per side, blocks per side, exaggeration, height of the base.
    grid: [f32; 4],
    /// Sky overhead; w: haze per block.
    zenith: [f32; 4],
    /// Sky at the horizon; w: exposure.
    horizon: [f32; 4],
    /// Sea level (drawn height), shown.
    sea: [f32; 4],
    /// Cloud base and top (drawn heights), cover, density.
    clouds: [f32; 4],
    /// Cloud size, offset of the seed, shown.
    clouds2: [f32; 4],
    /// tan(fov/2) × aspect, tan(fov/2).
    cam: [f32; 4],
    cam_f: [f32; 4],
    cam_s: [f32; 4],
    cam_u: [f32; 4],
    /// The mist: height of its densest (drawn), how fast it thins upward,
    /// its density there, and the haze's everywhere (per block).
    fog: [f32; 4],
    /// Directions and steps of the sky's occlusion.
    detail: [f32; 4],
}

const SHADER: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    sun: vec4<f32>,
    sun_color: vec4<f32>,
    grid: vec4<f32>,
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    sea: vec4<f32>,
    clouds: vec4<f32>,
    clouds2: vec4<f32>,
    cam: vec4<f32>,
    cam_f: vec4<f32>,
    cam_s: vec4<f32>,
    cam_u: vec4<f32>,
    fog: vec4<f32>,
    detail: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var heights: texture_2d<f32>;
@group(0) @binding(2) var colors: texture_2d<f32>;
@group(0) @binding(3) var light: texture_2d<f32>;
@group(0) @binding(4) var smooth_sampler: sampler;
@group(0) @binding(5) var slab0: texture_2d<f32>;
@group(0) @binding(6) var slab0_colors: texture_2d<f32>;
@group(0) @binding(7) var slab1: texture_2d<f32>;
@group(0) @binding(8) var slab1_colors: texture_2d<f32>;
@group(0) @binding(9) var water: texture_2d<f32>;

const EXTINCTION: f32 = 0.045;

fn height_at(i: i32, j: i32) -> f32 {
    let n = i32(u.grid.x);
    let c = clamp(vec2<i32>(i, j), vec2<i32>(0), vec2<i32>(n - 1));
    return textureLoad(heights, c, 0).r * u.grid.z;
}

/// The sample of the grid under a point of the scene.
fn cell_of(p: vec2<f32>) -> vec2<i32> {
    let spacing = u.grid.y / u.grid.x;
    let g = (p + vec2<f32>(u.grid.y * 0.5)) / spacing - 0.5;
    return vec2<i32>(i32(round(g.x)), i32(round(g.y)));
}

/// The drawn height of the ground under a point of the scene.
fn ground(p: vec2<f32>) -> f32 {
    let c = cell_of(p);
    return height_at(c.x, c.y);
}

/// The same, between the four samples around it: what shadows and the
/// sky's occlusion read, so that they do not step from one sample to the
/// next down a cliff.
fn ground_smooth(p: vec2<f32>) -> f32 {
    let spacing = u.grid.y / u.grid.x;
    let g = (p + vec2<f32>(u.grid.y * 0.5)) / spacing - 0.5;
    let i = floor(g);
    let f = g - i;
    let x = i32(i.x);
    let z = i32(i.y);
    let a = height_at(x, z);
    let b = height_at(x + 1, z);
    let c = height_at(x, z + 1);
    let d = height_at(x + 1, z + 1);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

/// Top and bottom (drawn heights) of slab `k` at a sample: rock over the
/// ground, floating islands or overhangs. Equal where there is none.
fn slab_at(k: u32, i: i32, j: i32) -> vec2<f32> {
    let n = i32(u.grid.x);
    let c = clamp(vec2<i32>(i, j), vec2<i32>(0), vec2<i32>(n - 1));
    var s = textureLoad(slab0, c, 0).rg;
    if (k == 1u) {
        s = textureLoad(slab1, c, 0).rg;
    }
    return s * u.grid.z;
}

// ---- the sky

fn sky_color(d: vec3<f32>) -> vec3<f32> {
    let up = max(d.y, 0.0);
    var c = mix(u.horizon.rgb, u.zenith.rgb, pow(up, 0.45));
    // A glow around the sun, wide and warm when it is low.
    let mu = max(dot(d, u.sun.xyz), 0.0);
    c += u.sun_color.rgb * (pow(mu, 6.0) * 0.08 + pow(mu, 48.0) * 0.18);
    if (d.y < 0.0) {
        c = mix(c, u.horizon.rgb * 0.75, clamp(-d.y * 3.0, 0.0, 1.0));
    }
    return c;
}

fn sun_disc(d: vec3<f32>) -> vec3<f32> {
    return u.sun_color.rgb * smoothstep(0.99982, 0.99994, dot(d, u.sun.xyz)) * 24.0;
}

// ---- clouds

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yxz + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let b = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let c = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let d = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(a, b, w.y), mix(c, d, w.y), w.z);
}

fn fbm3(p: vec3<f32>, octaves: i32) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    var norm = 0.0;
    for (var k = 0; k < octaves; k++) {
        s += a * noise3(q);
        norm += a;
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
        a *= 0.5;
    }
    return s / norm;
}

/// Density of the clouds at a point (0: clear air).
fn cloud_density(p: vec3<f32>) -> f32 {
    let base = u.clouds.x;
    let top = u.clouds.y;
    let h = (p.y - base) / max(top - base, 1.0);
    if (h <= 0.0 || h >= 1.0) {
        return 0.0;
    }
    // Flat bottoms, rounded tops.
    let profile = smoothstep(0.0, 0.12, h) * smoothstep(1.0, 0.45, h);
    let q = p / u.clouds2.x + vec3<f32>(u.clouds2.y, 0.0, u.clouds2.y * 0.7);
    let shape = fbm3(q * vec3<f32>(1.0, 2.2, 1.0), 4);
    let cover = u.clouds.z;
    var d = (shape - (1.0 - cover) * 0.85) / max(cover, 0.05) * profile;
    // Erode the edges with finer noise.
    d -= (1.0 - clamp(d * 3.0, 0.0, 1.0)) * fbm3(q * 5.3, 2) * 0.35;
    return clamp(d, 0.0, 1.0) * u.clouds.w;
}

/// Light reaching a point of the clouds from the sun.
fn cloud_sunlight(p: vec3<f32>) -> f32 {
    let thickness = u.clouds.y - u.clouds.x;
    let step = thickness / 5.0 / max(u.sun.y, 0.25);
    var dens = 0.0;
    for (var k = 1; k <= 4; k++) {
        dens += cloud_density(p + u.sun.xyz * step * f32(k));
    }
    return exp(-dens * step * EXTINCTION);
}

fn phase(c: f32, g: f32) -> f32 {
    return (1.0 - g * g) / pow(1.0 + g * g - 2.0 * g * c, 1.5);
}

struct Clouds {
    color: vec3<f32>,
    trans: f32,
};

/// The clouds along the ray `o + t d` up to `t_max`.
fn clouds_along(o: vec3<f32>, d: vec3<f32>, t_max: f32, jitter: f32) -> Clouds {
    var res: Clouds;
    res.color = vec3<f32>(0.0);
    res.trans = 1.0;
    if (u.clouds2.z < 0.5) {
        return res;
    }
    let base = u.clouds.x;
    let top = u.clouds.y;
    var ta = 0.0;
    var tb = t_max;
    if (abs(d.y) < 1e-5) {
        if (o.y < base || o.y > top) {
            return res;
        }
    } else {
        let t0 = (base - o.y) / d.y;
        let t1 = (top - o.y) / d.y;
        ta = max(min(t0, t1), 0.0);
        tb = min(max(t0, t1), t_max);
    }
    // Clouds far beyond the relief fade into the sky.
    tb = min(tb, ta + u.grid.y * 2.0);
    if (tb <= ta) {
        return res;
    }
    let steps = i32(u.eye.w);
    let dt = (tb - ta) / f32(steps);
    let mu = dot(d, u.sun.xyz);
    let ph = mix(phase(mu, 0.65), phase(mu, -0.15), 0.35);
    let ambient = mix(u.horizon.rgb, u.zenith.rgb, 0.55);
    for (var k = 0; k < steps; k++) {
        let t = ta + (f32(k) + jitter) * dt;
        let p = o + d * t;
        let dens = cloud_density(p);
        if (dens > 0.001) {
            let a = exp(-dens * EXTINCTION * dt);
            let h = (p.y - base) / (top - base);
            let lit = u.sun_color.rgb * cloud_sunlight(p) * ph * 0.9 + ambient * (0.35 + 0.65 * h) * 1.2;
            res.color += res.trans * (1.0 - a) * lit;
            res.trans *= a;
            if (res.trans < 0.02) {
                break;
            }
        }
    }
    // Far clouds thin out into the haze.
    let fade = exp(-ta * u.zenith.w * 0.5);
    res.color *= fade;
    res.trans = mix(1.0, res.trans, fade);
    return res;
}

/// Light the clouds let through to a point of the ground.
fn cloud_shadow(p: vec3<f32>) -> f32 {
    if (u.clouds2.z < 0.5 || u.sun.y <= 0.01) {
        return 1.0;
    }
    let t0 = max((u.clouds.x - p.y) / u.sun.y, 0.0);
    let t1 = (u.clouds.y - p.y) / u.sun.y;
    if (t1 <= 0.0) {
        return 1.0;
    }
    let dt = (t1 - t0) / 6.0;
    var dens = 0.0;
    for (var k = 0; k < 6; k++) {
        dens += cloud_density(p + u.sun.xyz * (t0 + (f32(k) + 0.5) * dt));
    }
    return mix(1.0, exp(-dens * dt * EXTINCTION), 0.85);
}

// ---- the relief

/// Value noise with its gradient (Inigo Quilez's): x the value (0..1), yzw
/// how it changes along x, y and z.
fn noised(x: vec3<f32>) -> vec4<f32> {
    let i = floor(x);
    let w = fract(x);
    let s = w * w * (3.0 - 2.0 * w);
    let ds = 6.0 * w * (1.0 - w);
    let a = hash3(i);
    let b = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let c = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let d = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let e = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let f = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let g = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let h = hash3(i + vec3<f32>(1.0, 1.0, 1.0));
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + c - d + e - f - g + h;
    let v = a + k1 * s.x + k2 * s.y + k3 * s.z + k4 * s.x * s.y + k5 * s.y * s.z + k6 * s.z * s.x + k7 * s.x * s.y * s.z;
    let grad = ds * vec3<f32>(
        k1 + k4 * s.y + k6 * s.z + k7 * s.y * s.z,
        k2 + k5 * s.z + k4 * s.x + k7 * s.z * s.x,
        k3 + k6 * s.x + k5 * s.y + k7 * s.x * s.y,
    );
    return vec4<f32>(v, grad);
}

/// Fractal value noise with its gradient, `octaves` of it, at `p` (already
/// scaled to the noise's size): x the value (about 0..1), yzw its gradient.
fn fbmd(p: vec3<f32>, octaves: i32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    var a = 0.5;
    var f = 1.0;
    var norm = 0.0;
    var q = p;
    for (var k = 0; k < octaves; k++) {
        let n = noised(q);
        sum += a * vec4<f32>(n.x, n.yzw * f);
        norm += a;
        f *= 2.03;
        a *= 0.5;
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
    }
    return sum / norm;
}

/// `n` tilted by the gradient `g` of a bump over the surface, `k` strong:
/// only the part of the gradient along the surface counts.
fn bumped(n: vec3<f32>, g: vec3<f32>, k: f32) -> vec3<f32> {
    let along = g - dot(g, n) * n;
    return normalize(n - along * k);
}

/// The crowns of a forest seen from above: in cells about `size` blocks
/// wide, the crown over a point — x how high on it (0 between crowns, 1 at
/// its top), yz its slope there, w a number of its own.
fn crowns(xz: vec2<f32>, size: f32) -> vec4<f32> {
    let p = xz / size;
    let cell = floor(p);
    var best = 1e9;
    var lean = vec2<f32>(0.0);
    var id = 0.0;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = cell + vec2<f32>(f32(dx), f32(dz));
            let h1 = hash3(vec3<f32>(c, 7.0));
            let h2 = hash3(vec3<f32>(c, 19.0));
            // Crowns of different sizes, here and there.
            let r = 0.55 + 0.4 * hash3(vec3<f32>(c, 31.0));
            let centre = c + vec2<f32>(0.15 + 0.7 * h1, 0.15 + 0.7 * h2);
            let v = (p - centre) / r;
            let d = dot(v, v);
            if (d < best) {
                best = d;
                lean = v;
                id = hash3(vec3<f32>(c, 53.0));
            }
        }
    }
    let crown = max(1.0 - best, 0.0);
    // The slope of the dome, 1 - |v|², where the point is on it.
    return vec4<f32>(crown, -lean * 2.0 * step(best, 1.0), id);
}

/// How much of the sky a point of the ground sees, from the heights around
/// it: in a few directions, the highest the ground rises over it, near and
/// far — the directions the surface faces counting the most.
fn sky_seen(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let dirs = i32(u.detail.x);
    let steps = i32(u.detail.y);
    if (dirs <= 0) {
        return 1.0;
    }
    let spacing = u.grid.y / u.grid.x;
    var total = 0.0;
    var weight = 0.0;
    for (var k = 0; k < dirs; k++) {
        let a = (f32(k) + 0.5) * 6.2831853 / f32(dirs);
        let dir = vec2<f32>(cos(a), sin(a));
        var rise = 0.0;
        var r = spacing * 1.8;
        for (var s = 0; s < steps; s++) {
            rise = max(rise, (ground_smooth(p.xz + dir * r) - p.y) / r);
            r *= 2.1;
        }
        let w = max(0.12, 0.5 + 0.5 * dot(dir, n.xz));
        total += w * (1.0 - rise / sqrt(1.0 + rise * rise));
        weight += w;
    }
    return total / weight;
}

struct Surface {
    albedo: vec3<f32>,
    n: vec3<f32>,
};

/// What a point of the relief is made of and how it faces, from the colour
/// the graph gives it (`base`, and `wide`, the same over a few samples) and
/// its smooth normal: rock where it is steep — drawn in the world's own
/// space, lumps, grain and beds, so that cliffs are neither stretched nor
/// faceted —, the graph's colours where it is not, a forest's crowns where
/// they are a forest's dark green. `footprint`: blocks a pixel covers, so
/// that what is finer fades instead of shimmering.
fn surface(base: vec3<f32>, wide: vec3<f32>, world: vec3<f32>, smooth_n: vec3<f32>, footprint: f32) -> Surface {
    var s: Surface;
    let n0 = normalize(smooth_n);
    // In blocks, whatever the exaggeration.
    let p = vec3<f32>(world.x, world.y / max(u.grid.z, 0.01), world.z);
    // While the view moves (`u.detail.z` 0), only what shows at a glance.
    let full = u.detail.z > 0.5;
    let fine = select(0.0, 1.0 - smoothstep(0.6, 2.2, footprint), full);
    let medium = 1.0 - smoothstep(3.0, 10.0, footprint);
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    // Where rock shows: steep ground, its edge ragged.
    let ragged = noise3(p * 0.09 + 5.3) - 0.5;
    let rock = smoothstep(0.24, 0.5, 1.0 - n0.y + ragged * 0.22);

    // Rock: big lumps and hollows, finer grain, and beds across the face,
    // wavering a little.
    let octaves = select(2, 3, full);
    let big = fbmd(p * 0.085, octaves);
    var grain = vec4<f32>(0.5, 0.0, 0.0, 0.0);
    if (fine > 0.001) {
        grain = fbmd(p * 0.42 + 11.0, 2);
    }
    let waver = noise3(vec3<f32>(p.x * 0.025, 0.0, p.z * 0.025)) * 10.0;
    let bed = fbmd(vec3<f32>(p.x * 0.01, (p.y + waver) * 0.32, p.z * 0.01), octaves - 1);
    var g_rock = big.yzw * 0.085 * 2.6 * medium + grain.yzw * 0.42 * 0.5 * fine;
    g_rock += vec3<f32>(0.0, bed.z * 0.32 * 1.4 * medium, 0.0);
    let stone = mix(vec3<f32>(dot(wide, lum)), wide, 0.5) * 0.85;
    let rock_albedo = stone * (0.62 + 0.66 * big.x) * mix(1.0, 0.84 + 0.32 * grain.x, fine) * (0.82 + 0.36 * bed.x);

    // Ground: the graph's colours, mottled; a forest where they are dark
    // green, its crowns standing out.
    let green = clamp((base.g - max(base.r, base.b)) / max(base.g, 1e-4) * 2.2, 0.0, 1.0);
    let forest = green * (1.0 - smoothstep(0.035, 0.12, dot(base, lum))) * (1.0 - rock);
    let mottle = fbmd(p * 0.045 + 3.1, octaves);
    // Toward a cliff's edge, the colours of the samples around rather than
    // of the one under the point: no teeth where a green top meets a wall.
    let edge = mix(base, wide, smoothstep(0.08, 0.3, 1.0 - n0.y));
    var ground = edge * (0.82 + 0.36 * mottle.x);
    if (fine > 0.001) {
        ground *= mix(1.0, 0.9 + 0.2 * noise3(p * 0.6), fine);
    }
    // Grass is never one green: drier, yellower patches here and there.
    let grassy = green * (1.0 - forest) * (1.0 - rock);
    let dry = smoothstep(0.42, 0.72, noise3(vec3<f32>(p.x * 0.006, 1.7, p.z * 0.006)) * 0.6 + mottle.x * 0.4);
    ground = mix(ground, ground * vec3<f32>(1.2, 1.04, 0.6), dry * grassy * 0.65);
    var g_ground = mottle.yzw * 0.045 * 0.6 * medium;
    // Crowns a pixel cannot hold fade into the mottle.
    let canopy = select(0.0, forest * (1.0 - smoothstep(0.5, 1.6, footprint)), full);
    if (canopy > 0.001) {
        let c = crowns(p.xz, 2.8);
        // Each crown a shade of its own, some yellower, the gaps between
        // them dark.
        let crown_tint = mix(vec3<f32>(0.72), vec3<f32>(1.25, 1.2, 0.8), c.w);
        let crowned = base * crown_tint * (0.42 + 0.72 * sqrt(c.x));
        ground = mix(ground, crowned, canopy);
        g_ground += vec3<f32>(c.y, 0.0, c.z) * 0.45 * canopy;
    }

    s.albedo = mix(ground, rock_albedo, rock);
    s.n = bumped(n0, mix(g_ground, g_rock, rock), 1.0);
    return s;
}

/// How much of the sun a point of the ground sees: soft shadows of the
/// relief, marched toward the sun through the heights.
fn relief_shadow(p: vec3<f32>) -> f32 {
    if (u.sun.y <= 0.0) {
        return 0.0;
    }
    let spacing = u.grid.y / u.grid.x;
    let half = u.grid.y * 0.5;
    var t = spacing * 1.2;
    var lit = 1.0;
    let steps = i32(u.sun_color.w);
    for (var k = 0; k < steps; k++) {
        let q = p + u.sun.xyz * t;
        if (abs(q.x) > half || abs(q.z) > half) {
            break;
        }
        let c = cell_of(q.xz);
        var h = height_at(c.x, c.y);
        if (u.detail.z > 0.5) {
            h = ground_smooth(q.xz);
        }
        lit = min(lit, clamp((q.y - h) / t * 10.0 + 0.4, 0.0, 1.0));
        // Rock over the ground casts its shadow too.
        for (var k = 0u; k < u32(u.sea.z); k++) {
            let s = slab_at(k, c.x, c.y);
            if (s.x - s.y > 0.3) {
                let outside = max(s.y - q.y, q.y - s.x);
                lit = min(lit, clamp(outside / t * 10.0 + 0.4, 0.0, 1.0));
            }
        }
        if (lit <= 0.0) {
            break;
        }
        t = t * 1.1 + spacing * 0.6;
    }
    return lit;
}

/// Sunlight and skylight on a surface: the sun where it is seen; the sky
/// from above as much of it as the point sees, the land's own light from
/// below.
fn lighting(albedo: vec3<f32>, n: vec3<f32>, sun_seen: f32, open: f32) -> vec3<f32> {
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let direct = u.sun_color.rgb * max(dot(n, u.sun.xyz), 0.0) * sun_seen;
    // The sky's light, less blue than the sky looks: shadows stay grey.
    let sky = mix(u.horizon.rgb, u.zenith.rgb, 0.55);
    let sky_light = mix(vec3<f32>(dot(sky, lum)), sky, 0.45) * 0.52;
    // What the sunlit land throws back, warm, from below.
    let land = u.sun_color.rgb * max(u.sun.y, 0.0) * 0.07 + sky_light * 0.2;
    let ambient = mix(land, sky_light, 0.5 + 0.5 * n.y) * open;
    return albedo * (direct + ambient);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

/// A pattern of thresholds that changes every pixel, to break the steps of
/// ray marching into fine noise.
fn dither(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

/// How much air lies between the eye and a point `dist` away along `d`:
/// a thin haze everywhere, and a mist that thickens toward the lowest ground
/// (`u.fog`: its height, how fast it thins upward, its density there, the
/// haze's).
fn optical_depth(d: vec3<f32>, dist: f32) -> f32 {
    var tau = u.fog.w * dist;
    let h = max(u.fog.y, 1.0);
    // The mist at the eye and at the far end, the integral written as their
    // difference rather than as the first times its growth along the ray:
    // looking down from high above, the first is nothing and the growth
    // overflows, and 0 × ∞ painted the land black.
    let at_eye = u.fog.z * exp(min(-(u.eye.y - u.fog.x) / h, 60.0));
    let at_end = u.fog.z * exp(min(-(u.eye.y + d.y * dist - u.fog.x) / h, 60.0));
    let k = d.y / h;
    if (abs(k * dist) < 1e-4) {
        tau += at_eye * dist;
    } else {
        tau += (at_eye - at_end) / k;
    }
    return max(tau, 0.0);
}

/// The colour of the air toward `d`: the sky low over the horizon that
/// way, so that what is far melts into the sky behind it.
fn air_color(d: vec3<f32>) -> vec3<f32> {
    return sky_color(normalize(vec3<f32>(d.x, abs(d.y) * 0.25 + 0.02, d.z)));
}

/// Haze, mist and clouds between the eye and a point `dist` away along `d`.
fn atmosphere(c: vec3<f32>, d: vec3<f32>, dist: f32, pixel: vec2<f32>) -> vec3<f32> {
    let through = exp(-optical_depth(d, dist));
    let col = mix(air_color(d), c, through);
    let cl = clouds_along(u.eye.xyz, d, dist, dither(pixel));
    return col * cl.trans + cl.color;
}

struct Out {
    // The same in the pass that only lays the depth and in the one that
    // shades: see `fs_depth`.
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) side: f32,
};

// What the fragments get of `Out`: its position without the invariance,
// which OpenGL ES allows only on what a vertex gives.
struct In {
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
        // Sobel's: the slope over the eight samples around, smoother than
        // over the two beside, so that the steps of the grid along a cliff
        // do not stripe it.
        let a = height_at(i - 1, j - 1);
        let b = height_at(i, j - 1);
        let c = height_at(i + 1, j - 1);
        let d = height_at(i - 1, j);
        let f = height_at(i + 1, j);
        let g = height_at(i - 1, j + 1);
        let h = height_at(i, j + 1);
        let k = height_at(i + 1, j + 1);
        let dx = ((c + 2.0 * f + k) - (a + 2.0 * d + g)) / (8.0 * spacing);
        let dz = ((g + 2.0 * h + k) - (a + 2.0 * b + c)) / (8.0 * spacing);
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

/// The graph's colour over a few samples around `uv`: what steep rock is
/// tinted with, so that a cliff does not take the stripes of the samples
/// along its foot.
fn wide_color(tex: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let t = 1.6 / u.grid.x;
    return (textureSampleLevel(tex, smooth_sampler, uv + vec2<f32>(t, t), 0.0).rgb
        + textureSampleLevel(tex, smooth_sampler, uv + vec2<f32>(-t, t), 0.0).rgb
        + textureSampleLevel(tex, smooth_sampler, uv + vec2<f32>(t, -t), 0.0).rgb
        + textureSampleLevel(tex, smooth_sampler, uv + vec2<f32>(-t, -t), 0.0).rgb) * 0.25;
}

/// The first pass: only the depth of the relief and the slabs, so that the
/// second shades each pixel once, not every face behind another.
@fragment
fn fs_depth() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@fragment
fn fs_slab_depth(in: SlabIn) -> @location(0) vec4<f32> {
    if (in.thick < 0.3) {
        discard;
    }
    return vec4<f32>(0.0);
}

@fragment
fn fs_main(in: In) -> @location(0) vec4<f32> {
    let footprint = length(fwidth(in.world));
    var base = textureSample(colors, smooth_sampler, in.uv).rgb;
    let cpu_open = textureSample(light, smooth_sampler, in.uv).g;
    var c: vec3<f32>;
    if (in.side > 0.5) {
        // The cut through the ground: earth darkening with depth.
        let depth = clamp((in.world.y - u.grid.w) / max(u.grid.y * 0.05, 1.0), 0.0, 1.0);
        base = mix(vec3<f32>(0.10, 0.08, 0.065), base * 0.55, depth);
        c = lighting(base, normalize(in.normal), cloud_shadow(in.world), 0.8);
    } else {
        let s = surface(base, wide_color(colors, in.uv), in.world, in.normal, footprint);
        let spacing = u.grid.y / u.grid.x;
        let geo = normalize(in.normal);
        let sun_seen = relief_shadow(in.world + geo * spacing * 0.8) * cloud_shadow(in.world);
        // The occlusion worked out with the preview, sample by sample, only
        // on gentle ground: down a cliff its samples would stripe it.
        let open = sky_seen(in.world + geo * spacing * 0.5, geo) * mix(1.0, cpu_open, 0.35 * geo.y * geo.y);
        c = lighting(s.albedo, s.n, sun_seen, open);
    }
    let v = in.world - u.eye.xyz;
    let dist = length(v);
    c = atmosphere(c, v / dist, dist, in.clip.xy);
    return vec4<f32>(aces(c * u.horizon.w), 1.0);
}

struct SlabOut {
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) thick: f32,
    @location(4) under: f32,
    @location(5) @interpolate(flat) layer: u32,
};

// What the fragments get of `SlabOut` (see `In`).
struct SlabIn {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) thick: f32,
    @location(4) under: f32,
    @location(5) @interpolate(flat) layer: u32,
};

fn slab_side(k: u32, i: i32, j: i32, under: bool) -> f32 {
    let s = slab_at(k, i, j);
    return select(s.x, s.y, under);
}

/// A slab: its top (vertices 0..n²) and its underside (n²..2n²), meeting at
/// its edges, where its thickness falls to nothing.
@vertex
fn vs_slab(@builtin(vertex_index) vi: u32, @builtin(instance_index) k: u32) -> SlabOut {
    let n = i32(u.grid.x);
    let spacing = u.grid.y / f32(n);
    let nn = u32(n * n);
    let under = vi >= nn;
    let v = i32(select(vi, vi - nn, under));
    let i = v % n;
    let j = v / n;
    let s = slab_at(k, i, j);
    let dx = (slab_side(k, i + 1, j, under) - slab_side(k, i - 1, j, under)) / (2.0 * spacing);
    let dz = (slab_side(k, i, j + 1, under) - slab_side(k, i, j - 1, under)) / (2.0 * spacing);
    var normal = normalize(vec3<f32>(-dx, 1.0, -dz));
    if (under) {
        normal = -normal;
    }
    let half = u.grid.y * 0.5;
    let p = vec3<f32>((f32(i) + 0.5) * spacing - half, select(s.x, s.y, under), (f32(j) + 0.5) * spacing - half);
    var out: SlabOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.uv = (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(n);
    out.normal = normal;
    out.thick = s.x - s.y;
    out.under = select(0.0, 1.0, under);
    out.layer = k;
    return out;
}

@fragment
fn fs_slab(in: SlabIn) -> @location(0) vec4<f32> {
    let footprint = length(fwidth(in.world));
    let c0 = textureSample(slab0_colors, smooth_sampler, in.uv).rgb;
    let c1 = textureSample(slab1_colors, smooth_sampler, in.uv).rgb;
    if (in.thick < 0.3) {
        discard;
    }
    let top_color = select(c0, c1, in.layer == 1u);
    var wide = wide_color(slab0_colors, in.uv);
    if (in.layer == 1u) {
        wide = wide_color(slab1_colors, in.uv);
    }
    var s = surface(top_color, wide, in.world, in.normal, footprint);
    var open = 0.9;
    if (in.under > 0.5) {
        // The underside: bare rock, in the shade, lit from below by the
        // ground and the haze.
        let rock = mix(vec3<f32>(0.24, 0.215, 0.19), top_color * 0.6, 0.2);
        s = surface(rock, rock, in.world, in.normal, footprint);
        open = 0.6;
    }
    let spacing = u.grid.y / u.grid.x;
    let geo = normalize(in.normal);
    let sun_seen = relief_shadow(in.world + geo * spacing * 0.35) * cloud_shadow(in.world);
    var c = lighting(s.albedo, s.n, sun_seen, open);
    // Sunlight thrown back up by the ground below.
    c += s.albedo * u.sun_color.rgb * max(u.sun.y, 0.0) * 0.22 * max(-s.n.y, 0.0);
    let v = in.world - u.eye.xyz;
    let dist = length(v);
    c = atmosphere(c, v / dist, dist, in.clip.xy);
    return vec4<f32>(aces(c * u.horizon.w), 1.0);
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
    var c = sky_color(d) + sun_disc(d);
    let cl = clouds_along(u.eye.xyz, d, u.grid.y * 6.0, dither(in.clip.xy));
    c = c * cl.trans + cl.color;
    return vec4<f32>(aces(c * u.horizon.w), 1.0);
}

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) depth: f32,
};

/// The water of rivers and lakes: the grid again, at the water's level
/// where there is water, on the ground at its banks.
@vertex
fn vs_water(@builtin(vertex_index) vi: u32) -> WaterOut {
    let n = i32(u.grid.x);
    let spacing = u.grid.y / f32(n);
    let i = i32(vi) % n;
    let j = i32(vi) / n;
    let ground_y = height_at(i, j);
    let level = textureLoad(water, vec2<i32>(i, j), 0).r * u.grid.z;
    let depth = max(level - ground_y, 0.0) / max(u.grid.z, 0.01);
    let half = u.grid.y * 0.5;
    let p = vec3<f32>((f32(i) + 0.5) * spacing - half, max(level, ground_y), (f32(j) + 0.5) * spacing - half);
    var out: WaterOut;
    out.clip = u.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.depth = depth;
    return out;
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    if (in.depth <= 0.12) {
        discard;
    }
    let v = u.eye.xyz - in.world;
    let dist = length(v);
    let to_eye = v / dist;
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(to_eye.y, 0.0), 5.0);
    let r = reflect(-to_eye, vec3<f32>(0.0, 1.0, 0.0));
    let reflected = sky_color(r) + sun_disc(r);
    let lit = u.sun_color.rgb * max(u.sun.y, 0.0) * cloud_shadow(in.world) * relief_shadow(in.world + vec3<f32>(0.0, 0.3, 0.0)) * 0.35
        + mix(u.horizon.rgb, u.zenith.rgb, 0.7) * 0.8;
    // Rivers are shallow: clearer and greener than the sea.
    let body = mix(vec3<f32>(0.10, 0.36, 0.34), vec3<f32>(0.015, 0.09, 0.13), clamp(in.depth / 10.0, 0.0, 1.0)) * lit;
    var c = mix(body, reflected, fresnel);
    c = atmosphere(c, -to_eye, dist, in.clip.xy);
    let alpha = clamp(0.5 + in.depth / 5.0, 0.5, 0.94);
    return vec4<f32>(aces(c * u.horizon.w), max(alpha, fresnel));
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
    // Depth of the water, in blocks.
    let depth = (u.sea.x - ground(in.world.xz)) / max(u.grid.z, 0.01);
    if (depth <= 0.0) {
        discard;
    }
    let v = u.eye.xyz - in.world;
    let dist = length(v);
    let to_eye = v / dist;
    let fresnel = 0.02 + 0.98 * pow(1.0 - max(to_eye.y, 0.0), 5.0);
    let r = reflect(-to_eye, vec3<f32>(0.0, 1.0, 0.0));
    let reflected = sky_color(r) + sun_disc(r);
    let lit = u.sun_color.rgb * max(u.sun.y, 0.0) * cloud_shadow(in.world) * 0.35 + mix(u.horizon.rgb, u.zenith.rgb, 0.7) * 0.8;
    let body = mix(vec3<f32>(0.06, 0.30, 0.30), vec3<f32>(0.008, 0.045, 0.09), clamp(depth / 24.0, 0.0, 1.0)) * lit;
    var c = mix(body, reflected, fresnel);
    c = atmosphere(c, -to_eye, dist, in.clip.xy);
    let alpha = clamp(0.45 + depth / 10.0, 0.45, 0.94);
    return vec4<f32>(aces(c * u.horizon.w), max(alpha, fresnel));
}
"#;

struct Target {
    size: [u32; 2],
    /// None at one sample a pixel: drawn straight into the image.
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::Texture,
    resolve_view: wgpu::TextureView,
    /// The same image as plain bytes, for egui.
    shown: crate::view_image::Shown,
    depth: wgpu::TextureView,
}

/// The relief on the GPU.
struct Relief {
    n: usize,
    group: wgpu::BindGroup,
    indices: wgpu::Buffer,
    count: u32,
    /// The triangles of a slab (its top, then its underside), and how many
    /// slabs to draw.
    slab_indices: wgpu::Buffer,
    slab_count: u32,
    slabs: u32,
    /// Whether rivers and lakes are drawn (over the slab's top triangles).
    water: bool,
}

/// GPU side of the viewport.
pub struct TerrainRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Samples a pixel: 4 with edges smoothed, else 1.
    samples: u32,
    /// Where the device cannot view the image as plain bytes (OpenGL):
    /// the pass that copies it so.
    copier: Option<crate::view_image::Copier>,
    /// The relief's and the slabs' depth, laid first.
    relief_depth: wgpu::RenderPipeline,
    slab_depth: wgpu::RenderPipeline,
    relief_pipeline: wgpu::RenderPipeline,
    slab_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    sea_pipeline: wgpu::RenderPipeline,
    water_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    relief: Option<Relief>,
    target: Option<Target>,
    /// Half as large, drawn while the view moves.
    small: Option<Target>,
}

impl TerrainRenderer {
    /// The same, `samples` a pixel (1 or 4).
    pub fn with_samples(device: &wgpu::Device, queue: &wgpu::Queue, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("massif graph view"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let texture = |binding: u32, filterable: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
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
                texture(1, false),
                texture(2, true),
                texture(3, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(5, false),
                texture(6, true),
                texture(7, false),
                texture(8, true),
                texture(9, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("massif graph view"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |label: &str, vs: &str, fs: &str, blend: wgpu::BlendState, depth_write: bool, compare: wgpu::CompareFunction, color: bool| {
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
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(blend),
                        write_mask: if color { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // The relief and the slabs in two passes: their depth, then their
        // colours where they are seen.
        let replace = wgpu::BlendState::REPLACE;
        let prepass = !cfg!(target_os = "macos");
        let relief_depth = pipeline("massif graph relief depth", "vs_main", "fs_depth", replace, true, wgpu::CompareFunction::Less, false);
        let slab_depth = pipeline("massif graph slabs depth", "vs_slab", "fs_slab_depth", replace, true, wgpu::CompareFunction::Less, false);
        let shaded = if prepass { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Less };
        let relief_pipeline = pipeline("massif graph relief", "vs_main", "fs_main", replace, !prepass, shaded, true);
        let slab_pipeline = pipeline("massif graph slabs", "vs_slab", "fs_slab", replace, !prepass, shaded, true);
        let sky_pipeline = pipeline("massif graph sky", "vs_sky", "fs_sky", replace, false, wgpu::CompareFunction::LessEqual, true);
        let sea_pipeline = pipeline("massif graph sea", "vs_sea", "fs_sea", wgpu::BlendState::ALPHA_BLENDING, false, wgpu::CompareFunction::Less, true);
        let water_pipeline = pipeline("massif graph water", "vs_water", "fs_water", wgpu::BlendState::ALPHA_BLENDING, false, wgpu::CompareFunction::Less, true);
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
            samples,
            relief_depth,
            slab_depth,
            relief_pipeline,
            slab_pipeline,
            sky_pipeline,
            sea_pipeline,
            water_pipeline,
            layout,
            uniforms,
            sampler,
            relief: None,
            target: None,
            small: None,
            copier: crate::view_image::Copier::new(device),
        }
    }

    /// Put a relief on the GPU.
    pub fn upload(&mut self, data: &ViewData) {
        let texture = |label: &str, format: wgpu::TextureFormat, bytes: &[u8], texel: u32| {
            // Empty: a texture of one sample, never read.
            let n = if bytes.is_empty() { 1 } else { data.n as u32 };
            let blank = vec![0u8; texel as usize];
            let bytes = if bytes.is_empty() { &blank[..] } else { bytes };
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
        let slab = |k: usize| {
            let s = data.slabs.get(k);
            let shape = s.map_or(&[][..], |s| bytemuck::cast_slice(&s.top_bottom));
            let colors = s.map_or(&[][..], |s| bytemuck::cast_slice(&s.colors));
            (
                texture("massif graph slab", wgpu::TextureFormat::Rg32Float, shape, 8),
                texture("massif graph slab colours", wgpu::TextureFormat::Rgba8UnormSrgb, colors, 4),
            )
        };
        let (slab0, slab0_colors) = slab(0);
        let (slab1, slab1_colors) = slab(1);
        let water = texture("massif graph water", wgpu::TextureFormat::R32Float, bytemuck::cast_slice(&data.water), 4);
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
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&slab0),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&slab0_colors),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&slab1),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&slab1_colors),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&water),
                },
            ],
        });
        // Reuse the triangles while the size stays.
        let buffer = |label: &str, idx: &[u32]| {
            wgpu::util::DeviceExt::create_buffer_init(
                &self.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(idx),
                    usage: wgpu::BufferUsages::INDEX,
                },
            )
        };
        let (indices, count, slab_indices, slab_count) = match self.relief.take() {
            Some(r) if r.n == data.n => (r.indices, r.count, r.slab_indices, r.slab_count),
            _ => {
                let idx = grid_indices(data.n);
                let slab = slab_indices(data.n);
                (
                    buffer("massif graph triangles", &idx),
                    idx.len() as u32,
                    buffer("massif graph slab triangles", &slab),
                    slab.len() as u32,
                )
            }
        };
        self.relief = Some(Relief {
            n: data.n,
            group,
            indices,
            count,
            slab_indices,
            slab_count,
            slabs: data.slabs.len().min(MAX_SLABS) as u32,
            water: data.water.len() == data.n * data.n,
        });
    }

    pub fn ensure_target(&mut self, size: [u32; 2]) -> bool {
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        self.target = Some(self.make_target(size));
        true
    }

    /// The same for the image drawn while the view moves.
    pub fn ensure_small(&mut self, size: [u32; 2]) -> bool {
        let size = [size[0].clamp(1, 4096), size[1].clamp(1, 4096)];
        if self.small.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        self.small = Some(self.make_target(size));
        true
    }

    fn make_target(&self, size: [u32; 2]) -> Target {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        // The image egui shows is read as plain bytes (see `view_image`).
        let texture = |label: &str, format, samples, usage, view_formats| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let msaa = (self.samples > 1).then(|| texture("massif graph msaa", COLOR_FORMAT, self.samples, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]));
        let resolve = texture(
            "massif graph image",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            crate::view_image::view_formats(&self.device),
        );
        let depth = texture("massif graph depth", DEPTH_FORMAT, self.samples, wgpu::TextureUsages::RENDER_ATTACHMENT, &[]);
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        Target {
            size,
            msaa: msaa.as_ref().map(view),
            resolve_view: view(&resolve),
            shown: crate::view_image::Shown::of(&self.device, self.copier.as_ref(), &resolve),
            resolve,
            depth: view(&depth),
        }
    }

    pub fn image_view(&self, small: bool) -> Option<&wgpu::TextureView> {
        if small { self.small.as_ref() } else { self.target.as_ref() }.map(|t| &t.shown.view)
    }

    fn render_to(&self, target: &Target, u: &Uniforms, sea: bool) {
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(u));
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("massif graph view"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("massif graph view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.msaa.as_ref().unwrap_or(&target.resolve_view),
                    depth_slice: None,
                    resolve_target: target.msaa.as_ref().map(|_| &target.resolve_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: u.horizon[0] as f64,
                            g: u.horizon[1] as f64,
                            b: u.horizon[2] as f64,
                            a: 1.0,
                        }),
                        store: if target.msaa.is_some() { crate::view_image::msaa_store(&self.device) } else { wgpu::StoreOp::Store },
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
                // Apple's GPUs leave out what is hidden by themselves: the
                // pass of depth first would only cost them.
                let passes = [(&self.relief_depth, &self.slab_depth), (&self.relief_pipeline, &self.slab_pipeline)];
                for (relief_pipeline, slab_pipeline) in passes.into_iter().skip(if cfg!(target_os = "macos") { 1 } else { 0 }) {
                    pass.set_pipeline(relief_pipeline);
                    pass.set_index_buffer(relief.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..relief.count, 0, 0..1);
                    if relief.slabs > 0 {
                        pass.set_pipeline(slab_pipeline);
                        pass.set_index_buffer(relief.slab_indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..relief.slab_count, 0, 0..relief.slabs);
                    }
                }
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
                if sea {
                    pass.set_pipeline(&self.sea_pipeline);
                    pass.draw(0..6, 0..1);
                }
                if relief.water {
                    // The first half of a slab's triangles: one grid.
                    pass.set_pipeline(&self.water_pipeline);
                    pass.set_index_buffer(relief.slab_indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..relief.slab_count / 2, 0, 0..1);
                }
            }
        }
        target.shown.update(self.copier.as_ref(), &mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    fn render(&mut self, u: &Uniforms, sea: bool, small: bool) {
        if let Some(target) = if small { &self.small } else { &self.target } {
            self.render_to(target, u, sea);
        }
    }

    /// Copy an image back to the CPU (RGBA, row-major). Blocks.
    fn read(&self, target: &Target) -> Option<(u32, u32, Vec<u8>)> {
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
        if !crate::view_image::wait(&self.device) {
            return None;
        }
        rx.recv().ok()?.ok()?;
        let data = buffer.get_mapped_range(..).ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]);
        }
        Some((w, h, out))
    }

    /// The image shown, read back (RGBA). Blocks.
    #[cfg(test)]
    pub fn read_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.read(self.target.as_ref()?)
    }
}

/// Triangles of a slab: its top, an `n` × `n` grid, then its underside, the
/// same grid over the next n² vertices.
fn slab_indices(n: usize) -> Vec<u32> {
    let n32 = n as u32;
    let mut idx = Vec::with_capacity((n - 1) * (n - 1) * 12);
    for base in [0, n32 * n32] {
        for j in 0..n32 - 1 {
            for i in 0..n32 - 1 {
                let a = base + j * n32 + i;
                let (b, c, d) = (a + 1, a + n32, a + n32 + 1);
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    idx
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
            distance: 1.3,
            target: [0.0, 0.0, 0.0],
        }
    }
}

/// How finely the view is marched: lighter while it moves.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Quality {
    Moving,
    Still,
    /// For saved images.
    Final,
}

/// The viewport editor.
pub struct TerrainView {
    renderer: Option<TerrainRenderer>,
    texture: Option<egui::TextureId>,
    pub camera: OrbitCamera,
    pub render: RenderSettings,
    /// The world's sea level, in blocks.
    pub sea_level: f32,
    /// What is shown.
    data: Option<Arc<ViewData>>,
    uploaded: Option<usize>,
    /// Range of heights shown, for framing.
    range: (f32, f32),
    /// What the last image was made with.
    last: Option<(Vec<u8>, bool)>,
    /// The view moved lately: an image at full quality follows.
    moved: Option<Instant>,
    /// egui shows the image drawn while the view moves.
    shows_small: bool,
}

impl Default for TerrainView {
    fn default() -> Self {
        Self {
            renderer: None,
            texture: None,
            camera: OrbitCamera::default(),
            render: RenderSettings::default(),
            sea_level: 62.0,
            data: None,
            uploaded: None,
            range: (0.0, 1.0),
            last: None,
            moved: None,
            shows_small: false,
        }
    }
}

impl TerrainView {
    /// Show a new relief.
    pub fn set(&mut self, data: Arc<ViewData>) {
        let (lo, mut hi) = data.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        for slab in &data.slabs {
            for &[top, bottom] in &slab.top_bottom {
                if top - bottom > 0.5 {
                    hi = hi.max(top);
                }
            }
        }
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

    fn uniforms(&self, data: &ViewData, aspect: f32, quality: Quality) -> Uniforms {
        let r = &self.render;
        let size = data.size.max(1.0);
        let ex = r.exaggeration;
        let (lo, hi) = self.range;
        let mid = (lo + hi) * 0.5 * ex;
        let target = Vec3::new(self.camera.target[0], mid + self.camera.target[1], self.camera.target[2]);
        let (sy, cy) = self.camera.yaw.sin_cos();
        let (sp, cp) = self.camera.pitch.sin_cos();
        let d = self.camera.distance * size;
        let eye = target + Vec3::new(d * cp * sy, d * sp, d * cp * cy);
        // The vertical field of view, widened on narrow views so that they
        // keep the width of a wide one.
        let tan = (FOV_Y * 0.5).tan() * (WIDE / aspect.max(0.1)).max(1.0);
        let near = (d * 0.002).max(0.5);
        let far = d + size * 8.0;
        let proj = {
            let f = 1.0 / tan;
            let q = far / (near - far);
            Mat4::from_cols(
                Vec4::new(f / aspect, 0.0, 0.0, 0.0),
                Vec4::new(0.0, f, 0.0, 0.0),
                Vec4::new(0.0, 0.0, q, -1.0),
                Vec4::new(0.0, 0.0, q * near, 0.0),
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
        let sun = r.sun_dir();
        let (zenith, horizon, sun_color) = r.sky();
        // Clouds and shadows marched, and the directions and steps the sky's
        // occlusion is looked for in.
        // Settings ▸ Graphics: Low takes the steps of a moving view still
        // too, High more than Medium's; saved images always the most.
        let light = crate::graphics::get().light;
        let (cloud_steps, shadow_steps, sky_dirs, sky_steps) = match (quality, light) {
            (Quality::Final, _) => (110.0, 96.0, 12.0, 8.0),
            (Quality::Moving, _) | (Quality::Still, crate::graphics::Light::Low) => (14.0, 24.0, 4.0, 4.0),
            (Quality::Still, crate::graphics::Light::Medium) => (40.0, 48.0, 8.0, 7.0),
            (Quality::Still, crate::graphics::Light::High) => (64.0, 72.0, 10.0, 8.0),
        };
        // Haze as a share of the map, whatever its size: at the usual 0.3,
        // an eighth of the light lost across it, so that the far side
        // stands back. Over the lowest ground, a mist thinning upward over
        // about a sixth of the relief: seen between the mountains, not from
        // above them.
        let thick = r.haze.clamp(0.0, 1.0);
        let haze = thick * 0.45 / size;
        let fog = [lo * ex, ((hi - lo) * ex).max(8.0) * 0.16, thick * 1.0 / size, haze];
        Uniforms {
            view_proj: (proj * view).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, cloud_steps],
            sun: [sun.x, sun.y, sun.z, 0.0],
            sun_color: [sun_color[0], sun_color[1], sun_color[2], shadow_steps],
            grid: [data.n as f32, size, ex, lo * ex - size * 0.03],
            zenith: [zenith[0], zenith[1], zenith[2], haze],
            horizon: [horizon[0], horizon[1], horizon[2], r.exposure.max(0.05)],
            sea: [self.sea_level * ex, r.sea as u8 as f32, data.slabs.len().min(MAX_SLABS) as f32, 0.0],
            clouds: [r.cloud_base * ex, (r.cloud_base + r.cloud_thickness.max(1.0)) * ex, r.cloud_cover.clamp(0.0, 1.0), r.cloud_density.max(0.0)],
            // Clouds left out by Settings ▸ Graphics, except in saved images.
            clouds2: [r.cloud_size.max(10.0), (r.cloud_seed % 1000) as f32 * 7.31, (r.clouds && (quality == Quality::Final || crate::graphics::get().clouds)) as u8 as f32, 0.0],
            cam: [tan * aspect, tan, 0.0, 0.0],
            cam_f: [fwd.x, fwd.y, fwd.z, 0.0],
            cam_s: [s.x, s.y, s.z, 0.0],
            cam_u: [up.x, up.y, up.z, 0.0],
            fog,
            detail: [sky_dirs, sky_steps, if quality == Quality::Moving { 0.0 } else { 1.0 }, 0.0],
        }
    }

    /// Draw the viewport into the available rectangle. Returns whether the
    /// relief's occlusion must be worked out again (the exaggeration
    /// changed).
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) -> bool {
        let pal = Palette::of(ui);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let Some(state) = frame.wgpu_render_state() else {
            painter.rect_filled(rect, 0, pal.canvas);
            return false;
        };
        let graphics = crate::graphics::get();
        // Edges smoothed or not (Settings ▸ Graphics): another renderer.
        if self.renderer.as_ref().is_some_and(|r| r.samples != graphics.samples()) {
            self.renderer = None;
            self.uploaded = None;
            self.last = None;
        }
        let mut renderer = self.renderer.take().unwrap_or_else(|| TerrainRenderer::with_samples(&state.device, &state.queue, graphics.samples()));
        // Left drag turns the view, Shift + left drag turns the sun, right
        // or middle drag moves, the wheel or a pinch comes closer.
        let mut moving = false;
        let shift = ui.input(|i| i.modifiers.shift);
        if response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            if shift {
                self.render.sun_azimuth = (self.render.sun_azimuth - d.x * 0.4).rem_euclid(360.0);
                self.render.sun_elevation = (self.render.sun_elevation - d.y * 0.25).clamp(1.0, 89.0);
            } else {
                self.camera.yaw -= d.x * 0.008;
                self.camera.pitch = (self.camera.pitch + d.y * 0.006).clamp(0.02, 1.5);
            }
            moving = d != egui::Vec2::ZERO;
        }
        if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
            let d = response.drag_delta();
            let size = self.data.as_ref().map_or(1000.0, |v| v.size);
            let k = self.camera.distance * size * 0.0016;
            let (s, c) = self.camera.yaw.sin_cos();
            self.camera.target[0] -= (d.x * c + d.y * s) * k;
            self.camera.target[2] -= (-d.x * s + d.y * c) * k;
            moving |= d != egui::Vec2::ZERO;
        }
        if response.hovered() {
            let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.y));
            let factor = if zoom != 1.0 { 1.0 / zoom } else { (-scroll * 0.003).exp() };
            if factor != 1.0 {
                self.camera.distance = (self.camera.distance * factor).clamp(0.03, 6.0);
                moving = true;
            }
        }
        if response.double_clicked() {
            self.camera = OrbitCamera {
                yaw: self.camera.yaw,
                ..OrbitCamera::default()
            };
        }
        if moving {
            self.moved = Some(Instant::now());
        }
        // While it moves the view is drawn lighter, then once at full
        // quality.
        let settled = self.moved.is_some_and(|t| t.elapsed() > Duration::from_millis(160));
        let quality = if self.moved.is_some() && !settled { Quality::Moving } else { Quality::Still };
        if self.moved.is_some() && !settled {
            ui.ctx().request_repaint_after(Duration::from_millis(170));
        } else if settled {
            self.moved = None;
        }
        // Drawn smaller and stretched by Settings ▸ Graphics' render scale.
        let ppp = ui.ctx().pixels_per_point() * graphics.render_scale;
        let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        // While it moves the view is drawn at half the size and shown
        // stretched (four times fewer pixels to light), sharp once it stops.
        let small = quality == Quality::Moving;
        let mut fresh = renderer.ensure_target(px);
        if small {
            fresh |= renderer.ensure_small([(px[0] / 2).max(1), (px[1] / 2).max(1)]);
        }
        if fresh || self.texture.is_none() || self.shows_small != small {
            let view = renderer.image_view(small).expect("target exists");
            let mut egui_renderer = state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(&state.device, view, wgpu::FilterMode::Linear, id),
                None => self.texture = Some(egui_renderer.register_native_texture(&state.device, view, wgpu::FilterMode::Linear)),
            }
            self.shows_small = small;
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
        let u = self.uniforms(&data, rect.width() / rect.height().max(1.0), quality);
        let key = (bytemuck::bytes_of(&u).to_vec(), self.render.sea);
        if fresh || self.last.as_ref() != Some(&key) {
            renderer.render(&u, self.render.sea, small);
            crate::graphics::drew_3d();
            self.last = Some(key);
        }
        self.renderer = Some(renderer);
        if let Some(id) = self.texture {
            painter.image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        (data.exaggeration - self.render.exaggeration).abs() > 1e-3
    }

    /// Render the view at `size` pixels, at full quality, as RGBA.
    pub fn render_image(&mut self, size: [u32; 2]) -> Option<(u32, u32, Vec<u8>)> {
        self.render_at(size, Quality::Final)
    }

    /// How long the view takes to draw at `size` while it moves, still,
    /// and for a saved image, in milliseconds (the image read back
    /// included).
    #[cfg(test)]
    pub fn time_qualities(&mut self, size: [u32; 2]) -> [f32; 3] {
        self.render_at(size, Quality::Final);
        let data = self.data.clone().expect("a relief");
        let aspect = size[0] as f32 / size[1] as f32;
        let all = [Quality::Moving, Quality::Still, Quality::Final].map(|q| self.uniforms(&data, aspect, q));
        let sea = self.render.sea;
        let renderer = self.renderer.as_mut().expect("a renderer");
        let target = renderer.make_target(size);
        all.map(|u| {
            renderer.render_to(&target, &u, sea);
            renderer.read(&target);
            // Ten frames, then the image once: what the card takes a frame.
            let started = Instant::now();
            for _ in 0..10 {
                renderer.render_to(&target, &u, sea);
            }
            renderer.read(&target);
            started.elapsed().as_secs_f32() * 1000.0 / 10.0
        })
    }

    /// How long a still frame takes at `size` with the graphics settings
    /// now (their samples a pixel, light and clouds; the render scale
    /// applied to `size`), in milliseconds: the best of ten.
    #[cfg(test)]
    pub fn time_still(&mut self, size: [u32; 2]) -> f32 {
        let g = crate::graphics::get();
        let size = [(size[0] as f32 * g.render_scale) as u32, (size[1] as f32 * g.render_scale) as u32];
        let data = self.data.clone().expect("a relief");
        if self.renderer.as_ref().is_some_and(|r| r.samples != g.samples()) {
            let r = self.renderer.take().expect("a renderer");
            self.renderer = Some(TerrainRenderer::with_samples(&r.device, &r.queue, g.samples()));
        }
        let renderer = self.renderer.as_mut().expect("a renderer");
        renderer.upload(&data);
        self.uploaded = Some(Arc::as_ptr(&data) as usize);
        let u = self.uniforms(&data, size[0] as f32 / size[1] as f32, Quality::Still);
        let sea = self.render.sea;
        let renderer = self.renderer.as_mut().expect("a renderer");
        let target = renderer.make_target(size);
        renderer.render_to(&target, &u, sea);
        renderer.read(&target);
        let mut best = f32::MAX;
        for _ in 0..10 {
            let started = Instant::now();
            renderer.render_to(&target, &u, sea);
            renderer.read(&target);
            best = best.min(started.elapsed().as_secs_f32() * 1000.0);
        }
        self.last = None;
        best
    }

    fn render_at(&mut self, size: [u32; 2], quality: Quality) -> Option<(u32, u32, Vec<u8>)> {
        let data = self.data.clone()?;
        let renderer = self.renderer.as_mut()?;
        if self.uploaded != Some(Arc::as_ptr(&data) as usize) {
            renderer.upload(&data);
            self.uploaded = Some(Arc::as_ptr(&data) as usize);
        }
        let u = self.uniforms(&data, size[0] as f32 / size[1].max(1) as f32, quality);
        let renderer = self.renderer.as_mut()?;
        let target = renderer.make_target(size);
        renderer.render_to(&target, &u, self.render.sea);
        let image = renderer.read(&target);
        // The viewport's own image must be drawn again.
        self.last = None;
        image
    }

    /// The render settings, as a panel. Returns whether they changed.
    pub fn settings_ui(&mut self, ui: &mut Ui) -> bool {
        use crate::widgets::{Field, checkbox, row, well};
        let pal = Palette::of(ui);
        let before = self.render;
        let r = &mut self.render;
        // Each group of settings: its name, then its rows in a well.
        let section = |ui: &mut Ui, icon: Icon, name: &str, note: &str| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(theme::glyph(icon).to_string()).size(13.0).color(pal.weak));
                ui.label(RichText::new(name).size(13.0).medium());
                if !note.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(note).size(11.0).color(pal.weak));
                    });
                }
            });
            ui.add_space(2.0);
        };
        ui.spacing_mut().slider_width = (ui.available_width() - 190.0).clamp(70.0, 220.0);
        section(ui, Icon::Sun, tr!("Sun"), tr!("Shift + drag in the view turns it"));
        well(ui, |ui| {
            row(ui, tr!("Direction"), |ui| {
                ui.add(Field::new(&mut r.sun_azimuth, 0.0..=360.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Height"), |ui| {
                ui.add(Field::new(&mut r.sun_elevation, 1.0..=89.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Strength"), |ui| {
                ui.add(Field::new(&mut r.sun_intensity, 0.0..=3.0).max_decimals(2));
            });
        });
        ui.add_space(10.0);
        section(ui, Icon::Cloud, tr!("Clouds"), "");
        well(ui, |ui| {
            row(ui, tr!("Show"), |ui| {
                checkbox(ui, &mut r.clouds, "");
            });
            if r.clouds {
                row(ui, tr!("Cover"), |ui| {
                    ui.add(Field::new(&mut r.cloud_cover, 0.0..=1.0).max_decimals(2));
                });
                row(ui, tr!("Base"), |ui| {
                    ui.add(Field::new(&mut r.cloud_base, -64.0..=600.0).suffix(tr!(" blocks")).max_decimals(0));
                });
                row(ui, tr!("Thickness"), |ui| {
                    ui.add(Field::new(&mut r.cloud_thickness, 5.0..=400.0).suffix(tr!(" blocks")).max_decimals(0));
                });
                row(ui, tr!("Density"), |ui| {
                    ui.add(Field::new(&mut r.cloud_density, 0.05..=3.0).max_decimals(2));
                });
                row(ui, tr!("Size"), |ui| {
                    ui.add(Field::new(&mut r.cloud_size, 20.0..=3000.0).suffix(tr!(" blocks")).max_decimals(0));
                });
                row(ui, tr!("Shape"), |ui| {
                    crate::widgets::drag(ui, egui::DragValue::new(&mut r.cloud_seed).range(0..=999));
                });
            }
        });
        ui.add_space(10.0);
        section(ui, Icon::Mountain, tr!("Scene"), "");
        well(ui, |ui| {
            row(ui, tr!("Height ×"), |ui| {
                ui.add(Field::new(&mut r.exaggeration, 1.0..=8.0).max_decimals(1));
            });
            row(ui, tr!("Haze"), |ui| {
                ui.add(Field::new(&mut r.haze, 0.0..=1.0).max_decimals(2));
            });
            row(ui, tr!("Sea"), |ui| {
                checkbox(ui, &mut r.sea, "");
            });
            row(ui, tr!("Exposure"), |ui| {
                ui.add(Field::new(&mut r.exposure, 0.2..=3.0).max_decimals(2));
            });
        });
        ui.add_space(10.0);
        if ui.button(tr!("Reset")).on_hover_text(tr!("Back to the usual sun, clouds and scene")).clicked() {
            *r = RenderSettings::default();
        }
        self.render != before
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
    fn hollows_are_less_open_to_the_sky_than_tops() {
        let n = 64;
        let heights: Vec<f32> = (0..n * n).map(|c| if (30..34).contains(&(c % n)) { 60.0 } else { 0.0 }).collect();
        let light = occlusion(&heights, n, 4.0, 1.0);
        assert!(light[32 * n + 36][1] < light[32 * n + 31][1]);
    }

    #[test]
    fn the_sky_follows_the_sun() {
        let mut r = RenderSettings {
            sun_elevation: 60.0,
            ..Default::default()
        };
        let (zenith, _, noon) = r.sky();
        r.sun_elevation = 3.0;
        let (_, horizon, sunset) = r.sky();
        assert!(zenith[2] > zenith[0], "a blue sky by day");
        assert!(sunset[0] > sunset[2] * 2.0 && noon[2] > sunset[2], "a warm sun at sunset");
        assert!(horizon[0] > horizon[2], "a warm horizon at sunset");
        // The sun is where the settings say: high and to the east.
        r.sun_azimuth = 90.0;
        r.sun_elevation = 45.0;
        let d = r.sun_dir();
        assert!(d.x > 0.6 && d.y > 0.6 && d.z.abs() < 1e-4);
    }
}
