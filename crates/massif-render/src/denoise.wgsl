// The denoiser of the path tracer's image: an edge-avoiding à-trous wavelet
// filter (Dammertz, Sewtz, Hanika and Lensch, 2010), its edges found by what
// the eyes' rays first met (the distance and the normal) and by the light's
// own variance, as in spatiotemporal variance-guided filtering (Schied et
// al., 2017), spatial only: the samples added up stand for its history.
//
// The light is filtered divided by the albedo met first (demodulated), so
// that the textures stay as sharp as they are, and multiplied by it again
// at the end. The more samples, the lower the variance of their mean and
// the narrower the edges: the filter fades by itself as the image settles.
//
// Passes: 0 prepares (the light over the albedo, and its variance); then
// up to five of the filter, their taps 1 pixel apart, then 2, 4, 8 and 16,
// fewer the more samples (the kernel narrower: less of the light's soft
// changes smoothed away once the noise is small); 6 multiplies the albedo
// back in and writes the image. Ping-ponged between two buffers of halves.

@group(0) @binding(0) var<uniform> P: Params;
// The samples added up and the guides, two entries a pixel (trace.wgsl).
@group(0) @binding(1) var<storage, read> accum: array<vec4<u32>>;
// The guides as the filter reads them, made by the first pass: the normal
// (octahedral, halves) and the distance along the view's axis (negative:
// the sky).
@group(0) @binding(2) var<storage, read_write> geo: array<vec2<u32>>;
@group(0) @binding(3) var<storage, read> src: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> dst: array<vec2<u32>>;
@group(0) @binding(5) var out_image: texture_storage_2d<rgba8unorm, write>;
// The pass, the step between taps (pixels), the denoised image's share of
// the shown one.
@group(0) @binding(6) var<uniform> D: vec4<u32>;

const SKY: f32 = 15000.0;

fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        n = vec3<f32>((1.0 - abs(n.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0)), n.z);
    }
    return normalize(n);
}

fn samples(i: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(accum[i * 2u]);
}

fn albedo_at(i: u32) -> vec3<f32> {
    let g = accum[i * 2u + 1u];
    return max(vec3<f32>(unpack2x16float(g.x), unpack2x16float(g.y).x), vec3<f32>(0.02));
}

fn demodulated(i: u32) -> vec3<f32> {
    let a = samples(i);
    return a.rgb / max(a.w, 1.0) / albedo_at(i);
}

fn load(i: u32) -> vec4<f32> {
    let v = src[i];
    return vec4<f32>(unpack2x16float(v.x), unpack2x16float(v.y));
}

fn store(i: u32, c: vec4<f32>) {
    let v = min(c, vec4<f32>(60000.0));
    dst[i] = vec2<u32>(pack2x16float(v.xy), pack2x16float(v.zw));
}

// The eyes' ray through the middle of pixel `q`, one long along the view's
// axis: times the distance along it, the point seen.
fn ray(q: vec2<i32>) -> vec3<f32> {
    let ndc = (vec2<f32>(q) + 0.5) / vec2<f32>(f32(P.size.x), f32(P.size.y)) * 2.0 - 1.0;
    return P.forward.xyz + P.right.xyz * (ndc.x * P.eye.w * P.right.w) - P.up.xyz * (ndc.y * P.eye.w);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = P.size.x;
    let h = P.size.y;
    let p = vec2<i32>(gid.xy);
    if (u32(p.x) >= w || u32(p.y) >= h) {
        return;
    }
    let i = u32(p.y) * w + u32(p.x);
    let kind = D.x;
    if (kind == 0u) {
        // The guides as the filter reads them; the light over the albedo,
        // and the variance of its mean: from the samples' own spread once
        // there are a few, from the pixels around before (a single sample
        // has none).
        let g = accum[i * 2u + 1u];
        let depth = unpack2x16float(g.y).y;
        let r = ray(p);
        let along = select(depth / length(r), -1.0, depth >= SKY);
        geo[i] = vec2<u32>(g.w, bitcast<u32>(along));
        let c = demodulated(i);
        let n = max(samples(i).w, 1.0);
        let l = luminance(c);
        var variance = max(bitcast<f32>(g.z) - l * l, 0.0) / n;
        if (n < 4.0) {
            var s1 = 0.0;
            var s2 = 0.0;
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let q = clamp(p + vec2<i32>(dx, dy), vec2<i32>(0), vec2<i32>(i32(w) - 1, i32(h) - 1));
                    let lq = luminance(demodulated(u32(q.y) * w + u32(q.x)));
                    s1 += lq;
                    s2 += lq * lq;
                }
            }
            let mean = s1 / 9.0;
            variance = max(variance, max(s2 / 9.0 - mean * mean, 0.0) / n);
        }
        store(i, vec4<f32>(c, variance));
        return;
    }
    let centre = load(i);
    if (kind == 6u) {
        // The albedo back in; shown as much as asked, over the raw image.
        let raw = samples(i);
        let noisy = raw.rgb / max(raw.w, 1.0);
        let clean = centre.rgb * albedo_at(i);
        let k = f32(D.z) / 65535.0;
        textureStore(out_image, p, vec4<f32>(tone(mix(noisy, clean, k), P.forward.w), 1.0));
        return;
    }
    // A pass of the filter: 3 × 3 taps `step` pixels apart, each weighed by
    // a binomial kernel and by how alike it is to the middle.
    let step = i32(D.y);
    // The variance at the middle, a little blurred (3 × 3), so that its
    // own noise does not decide the edges.
    var vblur = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let q = clamp(p + vec2<i32>(dx, dy), vec2<i32>(0), vec2<i32>(i32(w) - 1, i32(h) - 1));
            let k = select(0.25, 0.5, dx == 0) * select(0.25, 0.5, dy == 0);
            vblur += load(u32(q.y) * w + u32(q.x)).w * k;
        }
    }
    let sigma_l = 4.0 * sqrt(max(vblur, 0.0)) + 1e-4;
    let l_p = luminance(centre.rgb);
    let g_p = geo[i];
    let z_p = bitcast<f32>(g_p.y);
    let sky_p = z_p < 0.0;
    let n_p = oct_decode(unpack2x16snorm(g_p.x));
    let pos_p = ray(p) * z_p;
    // How large a pixel is at the middle's distance, in blocks.
    let pixel = z_p * P.up.w;
    let kernel = array<f32, 2>(0.5, 0.25);
    var sum = centre.rgb * 0.25;
    var var_sum = centre.w * 0.0625;
    var weights = 0.25;
    for (var ky = -1; ky <= 1; ky++) {
        for (var kx = -1; kx <= 1; kx++) {
            if (kx == 0 && ky == 0) {
                continue;
            }
            let q = p + vec2<i32>(kx, ky) * step;
            if (q.x < 0 || q.y < 0 || q.x >= i32(w) || q.y >= i32(h)) {
                continue;
            }
            let j = u32(q.y) * w + u32(q.x);
            let g_q = geo[j];
            let z_q = bitcast<f32>(g_q.y);
            if ((z_q < 0.0) != sky_p) {
                continue;
            }
            let c = load(j);
            var wgt = kernel[abs(kx)] * kernel[abs(ky)] * exp(-abs(luminance(c.rgb) - l_p) / sigma_l);
            if (!sky_p) {
                // Facing the same way, and on the middle's plane.
                let nd = max(dot(n_p, oct_decode(unpack2x16snorm(g_q.x))), 0.0);
                let n2 = nd * nd;
                let n4 = n2 * n2;
                let n16 = n4 * n4 * n4 * n4;
                let off = abs(dot(n_p, ray(q) * z_q - pos_p));
                let reach = pixel * f32(step) * length(vec2<f32>(f32(kx), f32(ky)));
                wgt *= n16 * n16 * n16 * n16 * exp(-off / (reach + 0.05));
            }
            sum += c.rgb * wgt;
            var_sum += c.w * wgt * wgt;
            weights += wgt;
        }
    }
    store(i, vec4<f32>(sum / weights, var_sum / (weights * weights)));
}
