// Massif's path tracer: rays through the sparse tree of the scene's chunks
// (see scene.rs), a sample a pixel a dispatch, added up while the view
// stays still.
//
// Scene coordinates: blocks from the corner of the scene's first chunk and
// the bottom of the world, so every block of the scene is at x, y, z >= 0.

struct Params {
    eye: vec4<f32>,      // xyz; w: tan(fov / 2)
    right: vec4<f32>,    // w: width / height
    up: vec4<f32>,       // w: the angle of a pixel, radians
    forward: vec4<f32>,  // w: exposure
    sun: vec4<f32>,      // direction toward the sun; w: cos of its radius
    sun_light: vec4<f32>,  // the sun's illuminance above the air (tinted); w: the sky's strength
    air: vec4<f32>,      // the air's scattering, the haze's, its anisotropy, the eyes' height (km)
    fog: vec4<f32>,      // extinction a block at its base, base height (scene), falloff (blocks), anisotropy
    media: vec4<f32>,    // shafts on, sea level (scene), emission strength, the lens's radius
    lens: vec4<f32>,     // focus distance
    clouds: vec4<f32>,   // cover (0..1), base height (scene), thickness, extinction a block (0: none)
    wind: vec4<f32>,     // how far the wind moved them (x, z), 1 / the size of their shapes
    size: vec4<u32>,     // width, height, seed of the frame, sample (0: start over)
    grid: vec4<i32>,     // chunks across, chunks down, height in blocks, sections
    info: vec4<u32>,     // words a chunk's record, pyramid levels, bounces, shift (pixels per side of a ray: 1 << shift)
    level_off: array<vec4<u32>, 4>,
    level_w: array<vec4<u32>, 4>,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> grid: array<u32>;
@group(0) @binding(2) var<storage, read> records: array<u32>;
@group(0) @binding(3) var<storage, read> tree: array<u32>;
@group(0) @binding(4) var<storage, read> voxels: array<u32>;
@group(0) @binding(5) var<storage, read> pyramid: array<u32>;
@group(0) @binding(6) var<storage, read> materials: array<vec4<u32>>;
@group(0) @binding(7) var<storage, read_write> accum: array<vec4<f32>>;
@group(0) @binding(8) var out_image: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(9) var atlas: texture_2d_array<f32>;
@group(0) @binding(10) var atlas_sampler: sampler;
@group(0) @binding(11) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(12) var multi_lut: texture_2d<f32>;
@group(0) @binding(13) var sky_view_lut: texture_2d<f32>;
@group(0) @binding(14) var lut_sampler: sampler;
// For every chunk, where its list of lights starts and how many: those of
// it and of the 8 chunks around; then the lists, [x, y, z, material] each.
@group(0) @binding(15) var<storage, read> lights: array<u32>;

const UNIFORM: u32 = 0x80000000u;
const CLASS_CUTOUT: u32 = 2u;
const CLASS_WATER: u32 = 3u;
const MAX_STEPS: i32 = 1536;
const EPS: f32 = 0.004;
// How far a ray that leaves the scene goes through the fog.
const FAR: f32 = 30000.0;

// The sun's light at the scene (through the air), set once a pixel.
var<private> SUN_LIGHT: vec3<f32>;

struct Hit {
    found: bool,
    t: f32,
    mat: u32,
    axis: i32,
}

fn level_offset(l: u32) -> u32 {
    return P.level_off[l >> 2u][l & 3u];
}

fn level_width(l: u32) -> u32 {
    return P.level_w[l >> 2u][l & 3u];
}

// One above the highest block a ray can reach in the square of 2^l chunks
// holding chunk (cx, cz), from the bottom of the scene.
fn top_at(l: u32, cx: i32, cz: i32) -> i32 {
    return i32(pyramid[level_offset(l) + u32(cz >> l) * level_width(l) + u32(cx >> l)]);
}

fn has_bit(lo: u32, hi: u32, i: u32) -> bool {
    if (i < 32u) {
        return ((lo >> i) & 1u) != 0u;
    }
    return ((hi >> (i - 32u)) & 1u) != 0u;
}

// The bits set below bit i.
fn rank(lo: u32, hi: u32, i: u32) -> u32 {
    if (i < 32u) {
        return countOneBits(lo & ((1u << i) - 1u));
    }
    return countOneBits(lo) + countOneBits(hi & ((1u << (i - 32u)) - 1u));
}

fn voxel_material(index: u32) -> u32 {
    let w = voxels[index >> 1u];
    return (w >> ((index & 1u) * 16u)) & 0xffffu;
}

fn class_of(m: vec4<u32>) -> u32 {
    return (m.y >> 16u) & 7u;
}

// The texture of the face of material m a ray going along rd entered at p
// through a face across `axis`: its coordinates and its layer.
fn face_uv(m: vec4<u32>, p: vec3<f32>, axis: i32, rd: vec3<f32>) -> vec3<f32> {
    let f = fract(p);
    var uv: vec2<f32>;
    var up = false;
    if (axis == 1) {
        up = rd.y < 0.0;
        uv = vec2<f32>(f.x, f.z);
    } else if (axis == 0) {
        // East (+x) when the ray goes west.
        uv = vec2<f32>(select(f.z, 1.0 - f.z, rd.x < 0.0), 1.0 - f.y);
    } else {
        // South (+z) when the ray goes north.
        uv = vec2<f32>(select(1.0 - f.x, f.x, rd.z < 0.0), 1.0 - f.y);
    }
    // A log's ends are across its axis: 0 y, 1 x, 2 z.
    let log_axis = (m.y >> 19u) & 3u;
    let ends = select(select(2, 0, log_axis == 1u), 1, log_axis == 0u);
    var layer: u32;
    if (axis == ends) {
        if (axis == 1 && !up) {
            layer = m.y & 0xffffu;
        } else {
            layer = m.x & 0xffffu;
        }
    } else {
        layer = m.x >> 16u;
        if (axis != 1 && log_axis != 0u) {
            // A log lying down: its bark runs along it.
            uv = vec2<f32>(uv.y, uv.x);
        }
    }
    return vec3<f32>(uv, f32(layer));
}

// The first block along a ray from ro (inside the scene or not) that stops
// it, up to tmax.
fn trace(ro: vec3<f32>, rd: vec3<f32>, tmax: f32) -> Hit {
    var h: Hit;
    h.found = false;
    h.t = tmax;
    h.mat = 0u;
    h.axis = 1;
    let dims = vec3<i32>(P.grid.x * 16, P.grid.z, P.grid.y * 16);
    let size = vec3<f32>(dims);
    let safe = select(rd, select(vec3<f32>(-1e-12), vec3<f32>(1e-12), rd >= vec3<f32>(0.0)), abs(rd) < vec3<f32>(1e-12));
    let inv = 1.0 / safe;
    let t0s = -ro * inv;
    let t1s = (size - ro) * inv;
    let tlo = min(t0s, t1s);
    let thi = max(t0s, t1s);
    let t_in = max(max(tlo.x, tlo.y), tlo.z);
    let t_out = min(min(min(thi.x, thi.y), thi.z), tmax);
    var t = max(t_in, 0.0);
    if (t >= t_out) {
        return h;
    }
    var axis = -1;
    if (t_in > 0.0) {
        if (tlo.x >= tlo.y && tlo.x >= tlo.z) {
            axis = 0;
        } else if (tlo.y >= tlo.z) {
            axis = 1;
        } else {
            axis = 2;
        }
    }
    var v = clamp(vec3<i32>(floor(ro + rd * t)), vec3<i32>(0), dims - 1);
    for (var i = 0; i < MAX_STEPS; i++) {
        // The largest box around v with nothing to stop at, or a block.
        var lo = v;
        var hi = v + 1;
        var mat = 0u;
        let cx = v.x >> 4u;
        let cz = v.z >> 4u;
        let top0 = top_at(0u, cx, cz);
        if (v.y >= top0) {
            // Above everything there: the widest square of the pyramid
            // whose land is all below.
            var l = 0u;
            var top = top0;
            loop {
                if (l + 1u >= P.info.y) {
                    break;
                }
                let up = top_at(l + 1u, cx, cz);
                if (v.y < up) {
                    break;
                }
                l += 1u;
                top = up;
            }
            let cell = 16 << l;
            lo = vec3<i32>(((cx >> l) << l) * 16, top, ((cz >> l) << l) * 16);
            hi = vec3<i32>(lo.x + cell, dims.y, lo.z + cell);
        } else {
            let slot = grid[u32(cz) * u32(P.grid.x) + u32(cx)];
            let rec = (slot - 1u) * P.info.x;
            let entry = records[rec + 4u + u32(v.y >> 4u)];
            if (entry == 0u) {
                lo = v & vec3<i32>(-16);
                hi = lo + 16;
            } else if ((entry & UNIFORM) != 0u) {
                mat = entry & 0xffffu;
            } else {
                let base = records[rec + 1u];
                let root = base + entry - 1u;
                let b = (v & vec3<i32>(15)) >> vec3<u32>(2u);
                let bi = u32(b.x + b.z * 4 + b.y * 16);
                let mlo = tree[root];
                let mhi = tree[root + 1u];
                if (!has_bit(mlo, mhi, bi)) {
                    lo = v & vec3<i32>(-4);
                    hi = lo + 4;
                } else {
                    let brick = base + tree[root + 2u] + rank(mlo, mhi, bi) * 3u;
                    let w = v & vec3<i32>(3);
                    let vi = u32(w.x + w.z * 4 + w.y * 16);
                    let blo = tree[brick];
                    let bhi = tree[brick + 1u];
                    if (has_bit(blo, bhi, vi)) {
                        let data = tree[brick + 2u];
                        if ((data & UNIFORM) != 0u) {
                            mat = data & 0xffffu;
                        } else {
                            mat = voxel_material(records[rec + 2u] + data + rank(blo, bhi, vi));
                        }
                    }
                }
            }
        }
        if (mat != 0u && axis >= 0) {
            let m = materials[mat * 2u];
            var stops = true;
            if (class_of(m) == CLASS_CUTOUT) {
                // Through the holes of leaves and glass.
                let uvl = face_uv(m, ro + rd * t, axis, rd);
                stops = textureSampleLevel(atlas, atlas_sampler, uvl.xy, i32(uvl.z), 0.0).a >= 0.5;
            }
            if (stops) {
                h.found = true;
                h.t = t;
                h.mat = mat;
                h.axis = axis;
                return h;
            }
        }
        // Out of the box, into the next cell.
        let bound = select(vec3<f32>(lo), vec3<f32>(hi), rd > vec3<f32>(0.0));
        let tb = (bound - ro) * inv;
        var a = 0;
        var te = tb.x;
        if (tb.y < te) {
            a = 1;
            te = tb.y;
        }
        if (tb.z < te) {
            a = 2;
            te = tb.z;
        }
        t = max(t, te);
        if (t >= t_out) {
            break;
        }
        var nv = clamp(vec3<i32>(floor(ro + rd * t)), lo, hi - 1);
        if (a == 0) {
            nv.x = select(lo.x - 1, hi.x, rd.x > 0.0);
        } else if (a == 1) {
            nv.y = select(lo.y - 1, hi.y, rd.y > 0.0);
        } else {
            nv.z = select(lo.z - 1, hi.z, rd.z > 0.0);
        }
        if (any(nv < vec3<i32>(0)) || any(nv >= dims)) {
            break;
        }
        v = nv;
        axis = a;
    }
    return h;
}

// Random numbers (PCG).
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(s: ptr<function, u32>) -> f32 {
    *s = pcg(*s);
    return f32(*s >> 8u) / 16777216.0;
}

fn basis(n: vec3<f32>) -> mat3x3<f32> {
    let a = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.5);
    let b = normalize(cross(n, a));
    let t = cross(b, n);
    return mat3x3<f32>(t, b, n);
}

fn cosine_sample(n: vec3<f32>, s: ptr<function, u32>) -> vec3<f32> {
    let r1 = rand(s);
    let r2 = rand(s);
    let phi = 2.0 * PI * r1;
    let r = sqrt(r2);
    return normalize(basis(n) * vec3<f32>(r * cos(phi), r * sin(phi), sqrt(max(1.0 - r2, 0.0))));
}

// A direction toward a point of the sun's disc.
fn sun_sample(s: ptr<function, u32>) -> vec3<f32> {
    let c = 1.0 - rand(s) * (1.0 - P.sun.w);
    let st = sqrt(max(1.0 - c * c, 0.0));
    let phi = 2.0 * PI * rand(s);
    return normalize(basis(P.sun.xyz) * vec3<f32>(st * cos(phi), st * sin(phi), c));
}

fn observer_r() -> f32 {
    return BOTTOM + max(P.air.w, 0.001);
}

// The sky's light along d (the sun's disc aside), from the sky-view table.
fn sky(d: vec3<f32>) -> vec3<f32> {
    let r = observer_r();
    var cos_az = 1.0;
    let ls = length(P.sun.xz);
    let ld = length(d.xz);
    if (ls > 1e-4 && ld > 1e-4) {
        cos_az = dot(P.sun.xz / ls, d.xz / ld);
    }
    let l = textureSampleLevel(sky_view_lut, lut_sampler, sky_view_uv(r, d.y, cos_az), 0.0).rgb;
    return l * P.sun_light.rgb * P.sun_light.w;
}

// The sun's illuminance once through the air, at the eyes' height.
fn sun_through_air() -> vec3<f32> {
    let r = observer_r();
    let mu = P.sun.y;
    if (hits_ground(r, mu)) {
        return vec3<f32>(0.0);
    }
    return P.sun_light.rgb * textureSampleLevel(transmittance_lut, lut_sampler, transmittance_uv(r, mu), 0.0).rgb;
}

// Light the air scatters more than once, a unit of scattering (Hillaire's
// isotropic approximation), at the eyes' height.
fn multi_scattering() -> vec3<f32> {
    let uv = vec2<f32>(P.sun.y * 0.5 + 0.5, clamp(P.air.w / (TOP - BOTTOM), 0.0, 1.0));
    let st = (uv * (MULTI_SIZE - 1.0) + 0.5) / MULTI_SIZE;
    return textureSampleLevel(multi_lut, lut_sampler, st, 0.0).rgb * P.sun_light.rgb * P.sun_light.w;
}

// The media of the scene, each thinning exponentially with height: 0 the
// air, 1 the haze (aerosols), 2 the fog. Per block (a metre).
fn medium_base(k: i32) -> f32 {
    if (k == 2) {
        return P.fog.y;
    }
    return P.media.y;
}

fn medium_height(k: i32) -> f32 {
    if (k == 0) {
        return RAYLEIGH_HEIGHT * 1000.0;
    }
    if (k == 1) {
        return MIE_HEIGHT * 1000.0;
    }
    return P.fog.z;
}

fn medium_extinction(k: i32) -> vec3<f32> {
    if (k == 0) {
        return RAYLEIGH_SCATTERING * 0.001 * P.air.x;
    }
    if (k == 1) {
        return vec3<f32>(MIE_EXTINCTION * 0.001 * P.air.y);
    }
    return vec3<f32>(P.fog.x);
}

fn medium_scattering(k: i32) -> vec3<f32> {
    if (k == 1) {
        return vec3<f32>(MIE_SCATTERING * 0.001 * P.air.y);
    }
    return medium_extinction(k);
}

fn medium_phase(k: i32, c: f32) -> f32 {
    if (k == 0) {
        return rayleigh_phase(c);
    }
    if (k == 1) {
        return mie_phase(c, P.air.z);
    }
    return hg_phase(c, P.fog.w);
}

fn density(k: i32, y: f32) -> f32 {
    return exp(clamp(-(y - medium_base(k)) / medium_height(k), -40.0, 8.0));
}

// The integral of the density along a ray from height oy going up dy a
// block, over t blocks.
fn depth_along(k: i32, oy: f32, dy: f32, t: f32) -> f32 {
    let h = medium_height(k);
    let a = density(k, oy);
    if (abs(dy * t) < h * 1e-3) {
        return a * t;
    }
    let b = density(k, oy + dy * t);
    return h / dy * (a - b);
}

// The distance along the ray at which the integral of the density reaches
// `depth` (the inverse of depth_along).
fn distance_at(k: i32, oy: f32, dy: f32, depth: f32) -> f32 {
    let h = medium_height(k);
    let a = density(k, oy);
    if (abs(dy) < 1e-5) {
        return depth / max(a, 1e-30);
    }
    let inner = a - depth * dy / h;
    if (inner <= 0.0) {
        return 1e30;
    }
    return (medium_base(k) - oy - h * log(inner)) / dy;
}

fn medium_on(k: i32) -> bool {
    return dot(medium_extinction(k), vec3<f32>(1.0)) > 0.0;
}

// How much of the sun gets down to height y through the scene's media.
fn sun_through_media(y: f32) -> vec3<f32> {
    let dy = max(P.sun.y, 0.02);
    var depth = vec3<f32>(0.0);
    for (var k = 0; k < 3; k++) {
        if (medium_on(k)) {
            depth += medium_extinction(k) * medium_height(k) / dy * density(k, y);
        }
    }
    return exp(-depth);
}

// The light the media scatter toward the eyes along a ray from ro, over
// t_end blocks (to what it hit, or out of the scene), and how much of what
// lies beyond gets through: one point of scattering, drawn in proportion
// to the media's density and transmittance (`first`: the air and the haze
// too; a ray out of the scene has them in the sky already).
struct Through {
    transmittance: vec3<f32>,
    light: vec3<f32>,
}

fn through_media(ro: vec3<f32>, rd: vec3<f32>, t_end: f32, all: bool, seed: ptr<function, u32>) -> Through {
    var out: Through;
    out.transmittance = vec3<f32>(1.0);
    out.light = vec3<f32>(0.0);
    var depth = vec3<f32>(0.0);
    var share = array<f32, 3>(0.0, 0.0, 0.0);
    var total = 0.0;
    for (var k = 0; k < 3; k++) {
        if (!medium_on(k) || (!all && k < 2)) {
            continue;
        }
        let d = depth_along(k, ro.y, rd.y, t_end);
        let tau = medium_extinction(k) * d;
        depth += tau;
        share[k] = (tau.x + tau.y + tau.z) / 3.0;
        total += share[k];
    }
    out.transmittance = exp(-depth);
    if (total < 1e-5) {
        return out;
    }
    // A medium in proportion to its depth, then a distance in it.
    let pick = rand(seed) * total;
    var k = 2;
    if (pick < share[0]) {
        k = 0;
    } else if (pick < share[0] + share[1]) {
        k = 1;
    }
    let sigma_k = (medium_extinction(k).x + medium_extinction(k).y + medium_extinction(k).z) / 3.0;
    let goal = -log(1.0 - rand(seed) * (1.0 - exp(-share[k])));
    let s = clamp(distance_at(k, ro.y, rd.y, goal / max(sigma_k, 1e-30)), 0.0, t_end);
    let x = ro + rd * s;
    // The probability of that distance, over the three ways to draw it.
    var pdf = 0.0;
    var depth_s = vec3<f32>(0.0);
    var scattering = vec3<f32>(0.0);
    var phased = vec3<f32>(0.0);
    let c = dot(rd, P.sun.xyz);
    for (var j = 0; j < 3; j++) {
        if (!medium_on(j) || (!all && j < 2)) {
            continue;
        }
        let rho = density(j, x.y);
        let dj = depth_along(j, ro.y, rd.y, s);
        depth_s += medium_extinction(j) * dj;
        let sj = (medium_extinction(j).x + medium_extinction(j).y + medium_extinction(j).z) / 3.0;
        if (share[j] > 0.0) {
            pdf += share[j] / total * sj * rho * exp(-sj * dj) / max(1.0 - exp(-share[j]), 1e-8);
        }
        scattering += medium_scattering(j) * rho;
        phased += medium_scattering(j) * rho * medium_phase(j, c);
    }
    if (pdf <= 0.0) {
        return out;
    }
    // The sun there, unless a block hides it (the shafts of light), as
    // much as the clouds let through.
    var sun = SUN_LIGHT * sun_through_media(x.y);
    if (P.media.x > 0.5 && P.sun.y > -0.02) {
        let shadow = trace(x, P.sun.xyz, 1e30);
        if (shadow.found) {
            sun = vec3<f32>(0.0);
        } else {
            sun *= cloud_shadow(x, seed);
        }
    }
    let inscatter = phased * sun + scattering * multi_scattering();
    out.light = exp(-depth_s) * inscatter / pdf;
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

fn unpack_rgb(u: u32) -> vec3<f32> {
    return srgb_to_linear(vec3<f32>(f32(u & 255u), f32((u >> 8u) & 255u), f32((u >> 16u) & 255u)) / 255.0);
}

// ACES, fitted (Narkowicz).
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = c * P.forward.w;
    let mapped = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
    return pow(mapped, vec3<f32>(1.0 / 2.2));
}

// ---- clouds ---------------------------------------------------------------

fn hash3(p: vec3<i32>) -> f32 {
    let h = pcg(u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u);
    return f32(h >> 8u) / 16777216.0;
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<i32>(1, 0, 0)), u.x);
    let b = mix(hash3(i + vec3<i32>(0, 1, 0)), hash3(i + vec3<i32>(1, 1, 0)), u.x);
    let c = mix(hash3(i + vec3<i32>(0, 0, 1)), hash3(i + vec3<i32>(1, 0, 1)), u.x);
    let d = mix(hash3(i + vec3<i32>(0, 1, 1)), hash3(i + vec3<i32>(1, 1, 1)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var norm = 0.0;
    var q = p;
    for (var k = 0; k < octaves; k++) {
        sum += amp * value_noise(q);
        norm += amp;
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.3);
        amp *= 0.5;
    }
    return sum / norm;
}

fn clouds_on() -> bool {
    return P.clouds.w > 0.0 && P.clouds.x > 0.0;
}

// The clouds' extinction at x: billows of noise, flat underneath and
// rounded on top, as many as the cover asks for.
fn cloud_density(x: vec3<f32>, octaves: i32) -> f32 {
    let h = (x.y - P.clouds.y) / P.clouds.z;
    if (h <= 0.0 || h >= 1.0) {
        return 0.0;
    }
    let profile = smoothstep(0.0, 0.12, h) * smoothstep(1.0, 0.35, h);
    let q = (x + vec3<f32>(P.wind.x, 0.0, P.wind.y)) * P.wind.z;
    let shape = fbm(vec3<f32>(q.x, q.y * 2.0, q.z), octaves);
    let cover = P.clouds.x;
    let d = clamp((shape * mix(0.6, 1.0, profile) - (1.0 - cover)) / max(cover, 0.05), 0.0, 1.0) * profile;
    return d * P.clouds.w;
}

// Where a ray from ro along rd is between the clouds' base and top.
fn cloud_span(ro: vec3<f32>, rd: vec3<f32>) -> vec2<f32> {
    let lo = P.clouds.y;
    let hi = P.clouds.y + P.clouds.z;
    if (abs(rd.y) < 1e-5) {
        if (ro.y > lo && ro.y < hi) {
            return vec2<f32>(0.0, FAR);
        }
        return vec2<f32>(1.0, 0.0);
    }
    let a = (lo - ro.y) / rd.y;
    let b = (hi - ro.y) / rd.y;
    return vec2<f32>(max(min(a, b), 0.0), min(max(a, b), FAR));
}

// How much of the sun gets through the clouds to x (a few steps toward it,
// their shapes coarser).
fn cloud_shadow(x: vec3<f32>, seed: ptr<function, u32>) -> f32 {
    if (!clouds_on() || P.sun.y <= 0.0) {
        return 1.0;
    }
    let span = cloud_span(x, P.sun.xyz);
    if (span.y <= span.x) {
        return 1.0;
    }
    let steps = 4;
    let dt = (span.y - span.x) / f32(steps);
    let j = rand(seed);
    var depth = 0.0;
    for (var k = 0; k < steps; k++) {
        depth += cloud_density(x + P.sun.xyz * (span.x + (f32(k) + j) * dt), 3) * dt;
    }
    return exp(-depth);
}

// A ray meeting the clouds before t_end: where, and the light they send
// back there (the sun through them, scattered many times as octaves of
// fading phases, and the sky's light); t < 0: it went through.
struct CloudHit {
    t: f32,
    light: vec3<f32>,
}

fn cloud_hit(ro: vec3<f32>, rd: vec3<f32>, t_end: f32, seed: ptr<function, u32>) -> CloudHit {
    var out: CloudHit;
    out.t = -1.0;
    out.light = vec3<f32>(0.0);
    if (!clouds_on()) {
        return out;
    }
    let span = cloud_span(ro, rd);
    let t1 = min(span.y, t_end);
    if (t1 <= span.x) {
        return out;
    }
    // Delta tracking: tentative collisions at the densest the clouds get,
    // each kept in proportion to the density there.
    let most = P.clouds.w;
    var t = span.x;
    for (var k = 0; k < 96; k++) {
        t += -log(1.0 - rand(seed)) / most;
        if (t >= t1) {
            return out;
        }
        let x = ro + rd * t;
        let d = cloud_density(x, 5);
        if (rand(seed) * most < d) {
            let c = dot(rd, P.sun.xyz);
            let through = cloud_shadow(x, seed);
            var sun = vec3<f32>(0.0);
            var a = 1.0;
            var b = 1.0;
            for (var o = 0; o < 4; o++) {
                let g = 0.8 * b;
                let phase = mix(hg_phase(c, g), hg_phase(c, -0.2 * b), 0.3);
                sun += a * phase * pow(through, b);
                a *= 0.5;
                b *= 0.5;
            }
            let h = clamp((x.y - P.clouds.y) / P.clouds.z, 0.0, 1.0);
            let sky_light = sky(vec3<f32>(0.0, 1.0, 0.0)) * mix(0.35, 1.0, h);
            out.t = t;
            out.light = 0.96 * (SUN_LIGHT * sun * 4.0 * PI * 0.25 + sky_light);
            return out;
        }
    }
    return out;
}

// The light of one of the glowing blocks near p (those of its chunk and
// the 8 around), drawn at random among them: a point in the block, a
// shadow ray, and the falloff with distance.
fn block_light(p: vec3<f32>, n: vec3<f32>, albedo: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    if (P.media.z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let c = clamp(vec2<i32>(floor(p.xz / 16.0)), vec2<i32>(0), vec2<i32>(P.grid.x - 1, P.grid.y - 1));
    let cell = u32(c.y * P.grid.x + c.x) * 2u;
    let count = lights[cell + 1u];
    if (count == 0u) {
        return vec3<f32>(0.0);
    }
    let k = lights[cell] + min(u32(rand(seed) * f32(count)), count - 1u);
    let at = u32(P.grid.x * P.grid.y) * 2u + k * 4u;
    let jitter = vec3<f32>(rand(seed), rand(seed), rand(seed)) * 0.5 + 0.25;
    let q = vec3<f32>(f32(lights[at]), f32(lights[at + 1u]), f32(lights[at + 2u])) + jitter;
    let glow = materials[lights[at + 3u] * 2u + 1u].x;
    let to = q - p;
    let d2 = max(dot(to, to), 0.25);
    let d = sqrt(d2);
    let l = to / d;
    let ndl = dot(n, l);
    if (ndl <= 0.0) {
        return vec3<f32>(0.0);
    }
    // Up to the light's own block, which it lights from inside.
    let shadow = trace(p, l, max(d - 0.75, 0.0));
    if (shadow.found) {
        return vec3<f32>(0.0);
    }
    let level = f32(glow >> 24u) / 15.0;
    let intensity = level * level * 24.0 * P.media.z;
    return albedo / PI * unpack_rgb(glow) * intensity * ndl / d2 * f32(count);
}

fn radiance(ro_in: vec3<f32>, rd_in: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    var ro = ro_in;
    var rd = rd_in;
    var throughput = vec3<f32>(1.0);
    var sum = vec3<f32>(0.0);
    var last_chunk = vec2<i32>(-1000000);
    for (var bounce = 0u; bounce <= P.info.z; bounce++) {
        let hit = trace(ro, rd, 1e30);
        if (bounce == 0u) {
            var t_end = hit.t;
            if (!hit.found) {
                t_end = FAR;
            }
            // A cloud before what the eyes would see: its light ends the path.
            let cloud = cloud_hit(ro, rd, t_end, seed);
            if (cloud.t >= 0.0) {
                let m = through_media(ro, rd, cloud.t, true, seed);
                return m.light + m.transmittance * cloud.light;
            }
            // The air, the haze and the fog between the eyes and what they see.
            let m = through_media(ro, rd, t_end, hit.found, seed);
            sum += m.light;
            throughput *= m.transmittance;
        }
        if (!hit.found) {
            var s = sky(rd);
            if (bounce == 0u && dot(rd, P.sun.xyz) > P.sun.w) {
                // The sun's disc, bright enough to bloom, not to stay a
                // firefly once added up.
                s += SUN_LIGHT * 40.0;
            }
            sum += throughput * s;
            break;
        }
        let m = materials[hit.mat * 2u];
        let glow = materials[hit.mat * 2u + 1u].x;
        let p = ro + rd * hit.t;
        var n = vec3<f32>(0.0);
        n[hit.axis] = -sign(rd[hit.axis]);
        let uvl = face_uv(m, p, hit.axis, rd);
        // Coarser textures the smaller a block is on screen.
        let lod = clamp(log2(max(hit.t * P.up.w * 16.0, 1e-6)), 0.0, 4.0);
        let tex = textureSampleLevel(atlas, atlas_sampler, uvl.xy, i32(uvl.z), lod);
        let top = hit.axis == 1 && rd.y < 0.0;
        let albedo = tex.rgb * unpack_rgb(select(m.w, m.z, top));
        if (((m.y >> 21u) & 1u) != 0u) {
            // A glowing block seen: by the eyes, or by a bounce from too far
            // for its light to be among those drawn there (block_light).
            let c = vec2<i32>(floor(p.xz / 16.0));
            if (bounce == 0u || any(abs(c - last_chunk) > vec2<i32>(1))) {
                let level = f32(glow >> 24u) / 15.0;
                sum += throughput * albedo * (8.0 * level * P.media.z);
            }
        }
        let pos = p + n * EPS;
        // The glowing blocks around, one drawn among them.
        sum += throughput * block_light(pos, n, albedo, seed);
        last_chunk = vec2<i32>(floor(pos.xz / 16.0));
        // The sun, straight.
        let l = sun_sample(seed);
        let ndl = dot(n, l);
        if (ndl > 0.0 && P.sun.y > -0.05) {
            let shadow = trace(pos, l, 1e30);
            if (!shadow.found) {
                // The clouds' shadows on what the eyes see (not on the bounces).
                var clouds = 1.0;
                if (bounce == 0u) {
                    clouds = cloud_shadow(pos, seed);
                }
                sum += throughput * albedo / PI * SUN_LIGHT * sun_through_media(pos.y) * clouds * ndl;
            }
        }
        if (bounce == P.info.z) {
            break;
        }
        // The light of the sky and of the land around: one bounce more.
        throughput *= albedo;
        if (bounce >= 1u) {
            let q = clamp(max(throughput.r, max(throughput.g, throughput.b)), 0.05, 1.0);
            if (rand(seed) > q) {
                break;
            }
            throughput /= q;
        }
        ro = pos;
        rd = cosine_sample(n, seed);
    }
    return sum;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let shift = P.info.w;
    let px = gid.xy << vec2<u32>(shift);
    let w = P.size.x;
    let h = P.size.y;
    if (px.x >= w || px.y >= h) {
        return;
    }
    var seed = pcg(gid.x * 1973u + gid.y * 9277u + P.size.z * 26699u) | 1u;
    SUN_LIGHT = sun_through_air();
    let block = f32(1u << shift);
    let jitter = vec2<f32>(rand(&seed), rand(&seed)) * block;
    let ndc = (vec2<f32>(px) + jitter) / vec2<f32>(f32(w), f32(h)) * 2.0 - 1.0;
    var dir = normalize(P.forward.xyz + P.right.xyz * (ndc.x * P.eye.w * P.right.w) - P.up.xyz * (ndc.y * P.eye.w));
    var eye = P.eye.xyz;
    if (P.media.w > 0.0) {
        // A thin lens: the rays of a pixel cross at the distance in focus,
        // from points spread over the opening.
        let focus = eye + dir * (P.lens.x / max(dot(dir, P.forward.xyz), 1e-3));
        let r = sqrt(rand(&seed)) * P.media.w;
        let a = 2.0 * PI * rand(&seed);
        eye += P.right.xyz * (r * cos(a)) + P.up.xyz * (r * sin(a));
        dir = normalize(focus - eye);
    }
    // Fireflies of a rare path are clipped: the image settles sooner.
    let l = min(radiance(eye, dir, &seed), vec3<f32>(32.0));
    let n = 1u << shift;
    for (var dy = 0u; dy < n; dy++) {
        for (var dx = 0u; dx < n; dx++) {
            let q = px + vec2<u32>(dx, dy);
            if (q.x >= w || q.y >= h) {
                continue;
            }
            let i = q.y * w + q.x;
            var a = vec4<f32>(l, 1.0);
            if (P.size.w != 0u) {
                a += accum[i];
            }
            accum[i] = a;
            textureStore(out_image, vec2<i32>(q), vec4<f32>(tonemap(a.rgb / a.w), 1.0));
        }
    }
}
