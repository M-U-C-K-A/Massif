// Lit previews of a terrain graph's results: the relief under a sun, with
// soft shadows, ambient occlusion and sky light, seen from above (2D) or
// from a camera orbiting it (3D). One invocation per output pixel; the
// result is packed RGBA8, sRGB. Without a graphics card, `shade.rs` falls
// back on the engine's simpler CPU shading.
//
// World units: x and z in blocks (sample × spacing), y in blocks times the
// vertical exaggeration.

struct U {
    w: u32,
    h: u32,
    ow: u32,
    oh: u32,
    spacing: f32,
    sea: f32,
    exag: f32,
    // 0 clay, 1 height tints, 2 the colours given.
    mode: u32,
    // Towards the sun (xyz), its strength (w).
    sun: vec4<f32>,
    // The overlay's colour (rgb, linear) and opacity (a).
    overlay: vec4<f32>,
    cam_pos: vec4<f32>,
    cam_fwd: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // 1 3D, 2 overlay, 4 contours, 8 water, 16 slope rock in tints.
    flags: u32,
    hmin: f32,
    hmax: f32,
    contour: f32,
    // Behind the terrain in 2D (sRGB, written as it is).
    back: vec4<f32>,
    // The 2D view: the samples the output covers (x0, z0, x1, z1).
    view: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> heights: array<f32>;
@group(0) @binding(2) var<storage, read> colors: array<f32>;
@group(0) @binding(3) var<storage, read> mask: array<f32>;
@group(0) @binding(4) var<storage, read_write> out_px: array<u32>;

fn sh_raw(x: i32, z: i32) -> f32 {
    let cx = clamp(x, 0, i32(u.w) - 1);
    let cz = clamp(z, 0, i32(u.h) - 1);
    return heights[u32(cz) * u.w + u32(cx)];
}

// Height in world units at sample coordinates p (bilinear).
fn sh_height(p: vec2<f32>) -> f32 {
    let f = floor(p);
    let t = p - f;
    let x = i32(f.x);
    let z = i32(f.y);
    let a = mix(sh_raw(x, z), sh_raw(x + 1, z), t.x);
    let b = mix(sh_raw(x, z + 1), sh_raw(x + 1, z + 1), t.x);
    return mix(a, b, t.y) * u.exag;
}

fn sh_plane(k: u32, p: vec2<f32>) -> f32 {
    let f = floor(p);
    let t = p - f;
    let x0 = u32(clamp(i32(f.x), 0, i32(u.w) - 1));
    let x1 = u32(clamp(i32(f.x) + 1, 0, i32(u.w) - 1));
    let z0 = u32(clamp(i32(f.y), 0, i32(u.h) - 1));
    let z1 = u32(clamp(i32(f.y) + 1, 0, i32(u.h) - 1));
    let base = k * u.w * u.h;
    let a = mix(colors[base + z0 * u.w + x0], colors[base + z0 * u.w + x1], t.x);
    let b = mix(colors[base + z1 * u.w + x0], colors[base + z1 * u.w + x1], t.x);
    return mix(a, b, t.y);
}

fn sh_mask(p: vec2<f32>) -> f32 {
    let f = floor(p);
    let t = p - f;
    let x0 = u32(clamp(i32(f.x), 0, i32(u.w) - 1));
    let x1 = u32(clamp(i32(f.x) + 1, 0, i32(u.w) - 1));
    let z0 = u32(clamp(i32(f.y), 0, i32(u.h) - 1));
    let z1 = u32(clamp(i32(f.y) + 1, 0, i32(u.h) - 1));
    let a = mix(mask[z0 * u.w + x0], mask[z0 * u.w + x1], t.x);
    let b = mix(mask[z1 * u.w + x0], mask[z1 * u.w + x1], t.x);
    return clamp(mix(a, b, t.y), 0.0, 1.0);
}

fn sh_normal(p: vec2<f32>) -> vec3<f32> {
    let dx = (sh_height(p + vec2<f32>(1.0, 0.0)) - sh_height(p - vec2<f32>(1.0, 0.0))) / (2.0 * u.spacing);
    let dz = (sh_height(p + vec2<f32>(0.0, 1.0)) - sh_height(p - vec2<f32>(0.0, 1.0))) / (2.0 * u.spacing);
    return normalize(vec3<f32>(-dx, 1.0, -dz));
}

// How much of the sun reaches world point q (sample coordinates p): a
// march toward the sun keeping the narrowest clearance, for a soft edge.
fn sh_shadow(p: vec2<f32>, y: f32) -> f32 {
    let l = u.sun.xyz;
    let flat_l = max(length(l.xz), 1e-4);
    var res = 1.0;
    var t = 1.0;
    for (var i = 0; i < 56; i = i + 1) {
        let q = p + l.xz / flat_l * t;
        if (q.x < -1.0 || q.y < -1.0 || q.x > f32(u.w) || q.y > f32(u.h)) {
            break;
        }
        let ry = y + l.y / flat_l * t * u.spacing;
        let d = ry - sh_height(q);
        res = min(res, 10.0 * d / (t * u.spacing));
        if (res < -0.05) {
            break;
        }
        t = t + max(0.6, t * 0.09);
    }
    return smoothstep(0.0, 1.0, clamp(res, 0.0, 1.0));
}

// Ambient occlusion: how open the sky is, from the horizon in 8 directions.
fn sh_occlusion(p: vec2<f32>, y: f32) -> f32 {
    var open = 0.0;
    for (var k = 0; k < 8; k = k + 1) {
        let a = f32(k) * 0.785398 + 0.39;
        let d = vec2<f32>(cos(a), sin(a));
        var horizon = 0.0;
        var r = 1.5;
        for (var s = 0; s < 6; s = s + 1) {
            let q = p + d * r;
            let rise = (sh_height(q) - y) / (r * u.spacing);
            horizon = max(horizon, rise);
            r = r * 2.0;
        }
        // sin of the horizon's angle.
        open = open + 1.0 - horizon / sqrt(1.0 + horizon * horizon);
    }
    return open / 8.0;
}

fn sh_ramp(hs: f32, steep: f32, rock_on_slopes: bool) -> vec3<f32> {
    // Above the sea, from 0 (the shore) to 1 (the highest).
    let t = clamp(hs, 0.0, 1.0);
    let sand = vec3<f32>(0.62, 0.55, 0.38);
    let grass = vec3<f32>(0.20, 0.30, 0.10);
    let forest = vec3<f32>(0.11, 0.19, 0.08);
    let rock = vec3<f32>(0.30, 0.27, 0.24);
    let snow = vec3<f32>(0.86, 0.88, 0.92);
    var c = mix(sand, grass, smoothstep(0.0, 0.03, t));
    c = mix(c, forest, smoothstep(0.15, 0.45, t));
    c = mix(c, rock, smoothstep(0.5, 0.75, t));
    if (rock_on_slopes) {
        c = mix(c, rock, smoothstep(0.55, 0.9, steep));
    }
    c = mix(c, snow, smoothstep(0.8, 0.9, t) * (1.0 - smoothstep(0.75, 1.0, steep)));
    return c;
}

fn sh_aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn sh_srgb(v: f32) -> f32 {
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

fn sh_pack(c: vec3<f32>) -> u32 {
    let r = u32(clamp(sh_srgb(c.x), 0.0, 1.0) * 255.0 + 0.5);
    let g = u32(clamp(sh_srgb(c.y), 0.0, 1.0) * 255.0 + 0.5);
    let b = u32(clamp(sh_srgb(c.z), 0.0, 1.0) * 255.0 + 0.5);
    return r | (g << 8u) | (b << 16u) | (255u << 24u);
}

const SUN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.94, 0.84);
// Light to the picture: flat clay in the sun comes out a mid grey.
const EXPOSURE: f32 = 0.6;
const SKY_COLOR: vec3<f32> = vec3<f32>(0.50, 0.64, 0.90);

// The ground at sample coordinates p, its height y (world units), seen
// from direction v (towards the eye): lit colour, linear.
fn sh_ground(p: vec2<f32>, y: f32, v: vec3<f32>) -> vec3<f32> {
    let n = sh_normal(p);
    let raw_h = y / u.exag;
    let steep = 1.0 - n.y;
    let range = max(u.hmax - u.sea, 1.0);
    var albedo = vec3<f32>(0.34, 0.33, 0.31);
    if (u.mode == 1u) {
        albedo = sh_ramp((raw_h - u.sea) / range, clamp(steep * 2.2, 0.0, 1.0), (u.flags & 16u) != 0u);
    } else if (u.mode == 2u) {
        albedo = max(vec3<f32>(sh_plane(0u, p), sh_plane(1u, p), sh_plane(2u, p)), vec3<f32>(0.0));
    }
    let under = (u.flags & 8u) != 0u && raw_h < u.sea;
    if (under) {
        // The sea floor, tinted by the water over it.
        let depth = clamp((u.sea - raw_h) / 40.0, 0.0, 1.0);
        albedo = mix(vec3<f32>(0.06, 0.17, 0.18), vec3<f32>(0.006, 0.028, 0.07), sqrt(depth));
    }
    let l = u.sun.xyz;
    let lambert = max(dot(n, l), 0.0);
    var sun = 0.0;
    if (lambert > 0.0) {
        sun = lambert * sh_shadow(p, y + 0.05 * u.spacing);
    }
    let ao = sh_occlusion(p, y);
    let sky = SKY_COLOR * (0.55 + 0.45 * n.y) * ao;
    // Light bounced from the sunlit ground around.
    let bounce = vec3<f32>(0.30, 0.26, 0.20) * (1.0 - n.y) * 0.35 * ao;
    var c = albedo * (SUN_COLOR * sun * u.sun.w + sky * 0.75 + bounce);
    if (under && (u.flags & 1u) == 0u) {
        // From above, a soft sheen of the sky on the water (in 3D the
        // water's surface is drawn by `sh_water`).
        let fres = 0.04 + 0.96 * pow(1.0 - clamp(v.y, 0.0, 1.0), 5.0);
        c = mix(c, SKY_COLOR * 0.9, fres * 0.6);
        let hv = normalize(l + v);
        c = c + SUN_COLOR * pow(max(hv.y, 0.0), 180.0) * 1.2 * u.sun.w;
    }
    if ((u.flags & 2u) != 0u && u.overlay.a > 0.0) {
        let m = sh_mask(p) * u.overlay.a;
        let light = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
        let tinted = u.overlay.rgb * (0.35 + 1.5 * light);
        c = mix(c * 0.55, tinted, smoothstep(0.0, 1.0, m));
    }
    if ((u.flags & 4u) != 0u && u.contour > 0.0) {
        let g = length(vec2<f32>(sh_height(p + vec2<f32>(1.0, 0.0)) - sh_height(p - vec2<f32>(1.0, 0.0)), sh_height(p + vec2<f32>(0.0, 1.0)) - sh_height(p - vec2<f32>(0.0, 1.0)))) * 0.5 / u.exag;
        let f = abs(fract(raw_h / u.contour + 0.5) - 0.5) * u.contour;
        let line = 1.0 - smoothstep(0.4, 1.2, f / max(g, 1e-3));
        c = c * (1.0 - 0.45 * line);
    }
    return c;
}

// Past the terrain in 3D: the sky, or (flag 32) the plain backdrop, which
// `shade_main` writes as it is (a negative colour marks it).
fn sh_backdrop(d: vec3<f32>) -> vec3<f32> {
    if ((u.flags & 32u) != 0u) {
        return vec3<f32>(-1.0);
    }
    return sh_sky(d);
}

// The sky without the sun, for reflections.
fn sh_sky_dome(d: vec3<f32>) -> vec3<f32> {
    let t = clamp(d.y * 1.8 + 0.25, 0.0, 1.0);
    var c = mix(vec3<f32>(0.55, 0.62, 0.70), vec3<f32>(0.62, 0.72, 0.85), smoothstep(0.0, 0.25, t));
    return mix(c, vec3<f32>(0.16, 0.32, 0.62), smoothstep(0.25, 1.0, t));
}

fn sh_sky(d: vec3<f32>) -> vec3<f32> {
    var c = sh_sky_dome(d);
    // Below the horizon, a dusk falling away from the haze, so that the
    // terrain stands out against it.
    c = mix(c, vec3<f32>(0.035, 0.045, 0.06), 1.0 - smoothstep(-0.55, -0.02, d.y));
    let s = max(dot(d, u.sun.xyz), 0.0);
    c = c + SUN_COLOR * (pow(s, 900.0) * 6.0 + pow(s, 12.0) * 0.18);
    return c;
}

fn sh_top(px: vec2<f32>) -> vec3<f32> {
    let uvx = (px.x + 0.5) / f32(u.ow);
    let uvz = (px.y + 0.5) / f32(u.oh);
    let p = vec2<f32>(mix(u.view.x, u.view.z, uvx), mix(u.view.y, u.view.w, uvz)) - vec2<f32>(0.5);
    let y = sh_height(p);
    return sh_ground(p, y, vec3<f32>(0.0, 1.0, 0.0));
}

fn sh_3d(px: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>((px.x + 0.5) / f32(u.ow) * 2.0 - 1.0, 1.0 - (px.y + 0.5) / f32(u.oh) * 2.0);
    let aspect = f32(u.ow) / f32(u.oh);
    let d = normalize(u.cam_fwd.xyz + u.cam_right.xyz * ndc.x * u.cam_up.w * aspect + u.cam_up.xyz * ndc.y * u.cam_up.w);
    let o = u.cam_pos.xyz;
    let sx = f32(u.w - 1u) * u.spacing;
    let sz = f32(u.h - 1u) * u.spacing;
    let ytop = u.hmax * u.exag + 1.0;
    let ybot = u.cam_right.w;
    // The block's box.
    let inv = 1.0 / d;
    let t0 = (vec3<f32>(0.0, ybot, 0.0) - o) * inv;
    let t1 = (vec3<f32>(sx, ytop, sz) - o) * inv;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let tn = max(max(tmin.x, tmin.y), max(tmin.z, 0.0));
    let tf = min(min(tmax.x, tmax.y), tmax.z);
    let sea_y = u.sea * u.exag;
    let water = (u.flags & 8u) != 0u && u.sea > u.hmin;
    if (tn >= tf) {
        return sh_backdrop(d);
    }
    var t = tn;
    var hit = false;
    var side = false;
    // Entering below the ground: a wall of the block.
    let e = o + d * tn;
    if (e.y < sh_height(e.xz / u.spacing) - 0.01) {
        side = true;
        hit = true;
    } else {
        let step_min = u.spacing * 0.2;
        var last = t;
        for (var i = 0; i < 320; i = i + 1) {
            let q = o + d * t;
            let gap = q.y - sh_height(q.xz / u.spacing);
            if (gap < 0.0) {
                hit = true;
                // Refine between last and t.
                var a = last;
                var b = t;
                for (var k = 0; k < 8; k = k + 1) {
                    let m = 0.5 * (a + b);
                    let qm = o + d * m;
                    if (qm.y - sh_height(qm.xz / u.spacing) < 0.0) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                t = b;
                break;
            }
            last = t;
            t = t + max(gap * 0.45, step_min);
            if (t > tf) {
                break;
            }
        }
    }
    var c: vec3<f32>;
    let q = o + d * t;
    if (!hit) {
        if (water && d.y < 0.0) {
            let tw = (sea_y - o.y) / d.y;
            if (tw > tn && tw < tf) {
                let w = o + d * tw;
                c = sh_water(w, d);
                return sh_haze(c, d, tw);
            }
        }
        return sh_backdrop(d);
    }
    if (water && q.y < sea_y && d.y < 0.0) {
        // The sea's surface, when the ray meets it inside the block (a ray
        // into a wall under the sea met the sea's plane before the block).
        let tw = (sea_y - o.y) / d.y;
        if (tw >= tn - 0.001 * u.spacing && tw < t) {
            let w = o + d * tw;
            c = sh_water(w, d);
            return sh_haze(c, d, tw);
        }
    }
    if (side) {
        // Strata of the cut: darker with depth, lit by the side it faces.
        let top = sh_height(q.xz / u.spacing);
        let depth = clamp((top - q.y) / max((ytop - ybot), 1.0), 0.0, 1.0);
        var n = vec3<f32>(0.0, 0.0, -1.0);
        if (abs(q.x) < 0.01 * u.spacing) {
            n = vec3<f32>(-1.0, 0.0, 0.0);
        } else if (abs(q.x - sx) < 0.01 * u.spacing) {
            n = vec3<f32>(1.0, 0.0, 0.0);
        } else if (abs(q.z - sz) < 0.01 * u.spacing) {
            n = vec3<f32>(0.0, 0.0, 1.0);
        }
        let band = 0.5 + 0.5 * sin(q.y / max(u.exag, 1e-3) * 0.35);
        let base = mix(vec3<f32>(0.28, 0.22, 0.17), vec3<f32>(0.16, 0.13, 0.11), depth) * (0.9 + 0.1 * band);
        let lit = max(dot(n, u.sun.xyz), 0.0) * 1.2 + 0.55;
        c = base * lit;
        if (water && q.y < sea_y) {
            c = mix(c, vec3<f32>(0.02, 0.10, 0.18), 0.55);
        }
    } else {
        c = sh_ground(q.xz / u.spacing, q.y, -d);
    }
    return sh_haze(c, d, t);
}

fn sh_water(w: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let p = w.xz / u.spacing;
    let floor_y = sh_height(p);
    let depth = clamp((w.y - floor_y) / (40.0 * u.exag), 0.0, 1.0);
    let floor_c = sh_ground(p, floor_y, -d);
    let deep = vec3<f32>(0.010, 0.050, 0.110);
    var c = mix(floor_c * vec3<f32>(0.55, 0.85, 0.9), deep, sqrt(depth));
    let v = -d;
    let fres = 0.03 + 0.97 * pow(1.0 - clamp(v.y, 0.0, 1.0), 5.0);
    let r = reflect(d, vec3<f32>(0.0, 1.0, 0.0));
    c = mix(c, sh_sky_dome(r) * 0.8, fres);
    let hv = normalize(u.sun.xyz + v);
    c = c + SUN_COLOR * pow(max(hv.y, 0.0), 2400.0) * 0.2 * u.sun.w;
    return c;
}

fn sh_haze(c: vec3<f32>, d: vec3<f32>, t: f32) -> vec3<f32> {
    // A plain backdrop: no sky to fade into.
    if ((u.flags & 32u) != 0u) {
        return c;
    }
    let size = f32(max(u.w, u.h)) * u.spacing;
    let f = 1.0 - exp(-max(t - size * 0.6, 0.0) / (size * 6.0));
    return mix(c, sh_sky_dome(vec3<f32>(d.x, max(d.y, 0.02), d.z)) * 0.9, clamp(f, 0.0, 0.3));
}

@compute @workgroup_size(16, 16)
fn shade_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.ow || id.y >= u.oh) {
        return;
    }
    let px = vec2<f32>(f32(id.x), f32(id.y));
    var c: vec3<f32>;
    if ((u.flags & 1u) != 0u) {
        c = sh_3d(px);
        if (c.x < -0.5) {
            let b = clamp(u.back.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
            out_px[id.y * u.ow + id.x] = u32(b.x * 255.0 + 0.5) | (u32(b.y * 255.0 + 0.5) << 8u) | (u32(b.z * 255.0 + 0.5) << 16u) | (255u << 24u);
            return;
        }
    } else {
        let p = vec2<f32>(mix(u.view.x, u.view.z, (px.x + 0.5) / f32(u.ow)), mix(u.view.y, u.view.w, (px.y + 0.5) / f32(u.oh))) - vec2<f32>(0.5);
        if (p.x < -0.5 || p.y < -0.5 || p.x > f32(u.w) - 0.5 || p.y > f32(u.h) - 0.5) {
            let b = clamp(u.back.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
            out_px[id.y * u.ow + id.x] = u32(b.x * 255.0 + 0.5) | (u32(b.y * 255.0 + 0.5) << 8u) | (u32(b.z * 255.0 + 0.5) << 16u) | (255u << 24u);
            return;
        }
        c = sh_top(px);
    }
    out_px[id.y * u.ow + id.x] = sh_pack(sh_aces(c * EXPOSURE));
}
