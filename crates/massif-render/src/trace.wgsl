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
    sun_light: vec4<f32>,  // irradiance / pi; w: the sun's disc, radiance
    sky_zenith: vec4<f32>, // w: brightness of the sky
    sky_horizon: vec4<f32>,
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

const UNIFORM: u32 = 0x80000000u;
const CLASS_CUTOUT: u32 = 2u;
const CLASS_WATER: u32 = 3u;
const MAX_STEPS: i32 = 1536;
const PI: f32 = 3.14159265;
const EPS: f32 = 0.004;

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
            let m = materials[mat];
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

fn sky(d: vec3<f32>) -> vec3<f32> {
    let h = d.y;
    var c: vec3<f32>;
    if (h >= 0.0) {
        c = mix(P.sky_zenith.rgb, P.sky_horizon.rgb, pow(1.0 - h, 4.0));
    } else {
        c = P.sky_horizon.rgb * mix(1.0, 0.35, min(-h * 3.0, 1.0));
    }
    // The light around the sun.
    let glow = pow(max(dot(d, P.sun.xyz), 0.0), 24.0) * 0.6;
    return (c + P.sun_light.rgb * glow * 0.15) * P.sky_zenith.w;
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

fn radiance(ro_in: vec3<f32>, rd_in: vec3<f32>, seed: ptr<function, u32>) -> vec3<f32> {
    var ro = ro_in;
    var rd = rd_in;
    var throughput = vec3<f32>(1.0);
    var sum = vec3<f32>(0.0);
    for (var bounce = 0u; bounce <= P.info.z; bounce++) {
        let hit = trace(ro, rd, 1e30);
        if (!hit.found) {
            var s = sky(rd);
            if (bounce == 0u && dot(rd, P.sun.xyz) > P.sun.w) {
                s += P.sun_light.rgb * P.sun_light.w;
            }
            sum += throughput * s;
            break;
        }
        let m = materials[hit.mat];
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
            sum += throughput * albedo * 6.0;
        }
        let pos = p + n * EPS;
        // The sun, straight.
        let l = sun_sample(seed);
        let ndl = dot(n, l);
        if (ndl > 0.0 && P.sun.y > -0.05) {
            let shadow = trace(pos, l, 1e30);
            if (!shadow.found) {
                sum += throughput * albedo * P.sun_light.rgb * ndl;
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
    let block = f32(1u << shift);
    let jitter = vec2<f32>(rand(&seed), rand(&seed)) * block;
    let ndc = (vec2<f32>(px) + jitter) / vec2<f32>(f32(w), f32(h)) * 2.0 - 1.0;
    let dir = normalize(P.forward.xyz + P.right.xyz * (ndc.x * P.eye.w * P.right.w) - P.up.xyz * (ndc.y * P.eye.w));
    // Fireflies of a rare path are clipped: the image settles sooner.
    let l = min(radiance(P.eye.xyz, dir, &seed), vec3<f32>(32.0));
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
