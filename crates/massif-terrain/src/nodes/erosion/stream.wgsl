// Stream-power erosion, step for step as `stream.rs` on the CPU.

struct U {
    n: u32,
    seed: u32,
    sp: f32,
    dt: f32,
    kt: f32,
    m: f32,
    n_exp: f32,
    ac: f32,
    deposition: f32,
    uplift: f32,
    diff: f32,
    sea: f32,
    has_hard: u32,
    has_rain: u32,
    has_lift: u32,
    scale: f32,
    sed_share: f32,
    supply: f32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> h: array<f32>;
@group(0) @binding(2) var<storage, read_write> hn: array<f32>;
@group(0) @binding(3) var<storage, read> qs: array<f32>;
@group(0) @binding(4) var<storage, read_write> qsn: array<f32>;
@group(0) @binding(5) var<storage, read> rcv: array<u32>;
@group(0) @binding(6) var<storage, read> acc: array<u32>;
@group(0) @binding(7) var<storage, read> hard: array<f32>;
@group(0) @binding(8) var<storage, read> rain_in: array<f32>;
@group(0) @binding(9) var<storage, read> lift: array<f32>;
@group(0) @binding(10) var<storage, read_write> w: array<f32>;
@group(0) @binding(16) var<storage, read_write> worn: array<f32>;
@group(0) @binding(17) var<storage, read_write> laid: array<f32>;
@group(0) @binding(15) var<storage, read_write> top: array<atomic<u32>>;
@group(0) @binding(11) var<storage, read_write> o0: array<f32>;
@group(0) @binding(12) var<storage, read_write> o1: array<f32>;
@group(0) @binding(13) var<storage, read_write> o2: array<f32>;
@group(0) @binding(14) var<storage, read_write> o3: array<f32>;

fn is_outlet(x: u32, z: u32, k: u32) -> bool {
    return x == 0u || z == 0u || x == u.n - 1u || z == u.n - 1u || h[k] < u.sea;
}

fn settles(slope: f32, area: f32) -> f32 {
    return clamp(u.deposition * u.sp / 32.0 / ((1.0 + 8.0 * slope) * (1.0 + sqrt(area) / 64.0)), 0.0, 1.0);
}

fn at(x: i32, z: i32) -> f32 {
    let cx = clamp(x, 0, i32(u.n) - 1);
    let cz = clamp(z, 0, i32(u.n) - 1);
    return h[u32(cz) * u.n + u32(cx)];
}

@compute @workgroup_size(16, 16)
fn step(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let hk = h[k];
    let r = rcv[k];
    var qin = 0.0;
    for (var d = 0u; d < 8u; d++) {
        let j = nb_of(id.x, id.y, u.n, d);
        if j != NONE && rcv[j] == k {
            qin += qs[j];
        }
    }
    var v = hk;
    var slope = 0.0;
    if r != NONE {
        let rx = r % u.n;
        let rz = r / u.n;
        var dist = u.sp;
        if rx != id.x && rz != id.y {
            dist = 1.4142135 * u.sp;
        }
        let hr = h[r];
        if hk > hr {
            slope = (hk - hr) / dist;
            let a = f32(acc[k]) / u.scale * u.sp * u.sp;
            var soft = 1.0;
            if u.has_hard != 0u {
                soft = 1.0 - 0.95 * clamp(hard[k], 0.0, 1.0);
            }
            let c = u.dt * u.kt * soft * max(pow(a, u.m) - u.ac, 0.0) * pow(slope, u.n_exp - 1.0) / dist;
            v = (hk + c * hr) / (1.0 + c);
        }
    }
    var lm = 1.0;
    if u.has_lift != 0u {
        lm = clamp(lift[k], 0.0, 1.0);
    }
    if u.supply > 0.0 && r != NONE {
        v -= u.supply * u.dt * lm * smoothstep(0.15, 0.6, slope);
    }
    let e = hk - v;
    let total = qin + e * u.sed_share;
    var f = 0.0;
    if r == NONE {
        if !is_outlet(id.x, id.y, k) {
            f = 1.0;
        } else if hk < u.sea {
            f = 0.5;
        }
    } else if hk < u.sea {
        f = 0.5;
    } else {
        f = settles(slope, f32(acc[k]) / u.scale * u.sp * u.sp);
    }
    let dep = total * f;
    v += dep;
    qsn[k] = total - dep;
    worn[k] += e;
    laid[k] += dep;
    let x = i32(id.x);
    let z = i32(id.y);
    v += u.diff * (at(x - 1, z) + at(x + 1, z) + at(x, z - 1) + at(x, z + 1) - 4.0 * hk);
    v += u.uplift * u.dt * lm;
    hn[k] = v;
}

// The rain, varied by the seed by up to a tenth, into `o2`.
@compute @workgroup_size(16, 16)
fn rain(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var r = 1.0;
    if u.has_rain != 0u {
        r = clamp(rain_in[k], 0.0, 1.0);
    }
    o2[k] = r * (0.9 + 0.2 * rand01(id.x, id.y, u.seed));
}

// `h` less `hn`, into `o0`.
@compute @workgroup_size(16, 16)
fn difference(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    o0[k] = h[k] - hn[k];
}

// Flow, wear, deposition and sediment, from the run (`acc`, `qs`, and
// what was cut and laid, bound in `hard` and `lift`), as
// `stream::outputs`.
@compute @workgroup_size(16, 16)
fn outputs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let a = f32(acc[k]) / u.scale;
    let n2 = f32(u.n) * f32(u.n);
    o0[k] = pow(clamp(log(1.0 + a) / log(1.0 + n2), 0.0, 1.0), 1.5);
    let cut = max(hard[k], 0.0);
    o1[k] = cut / (cut + u.dt);
    let put = max(lift[k], 0.0);
    o2[k] = put / (put + u.kt);
    o3[k] = clamp(log(1.0 + max(qs[k], 0.0)) / log(1000.0), 0.0, 1.0);
}

// `h` stretched by `u.uplift` from `u.sea`, into `hn`.
@compute @workgroup_size(16, 16)
fn rescale(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    hn[k] = u.sea + (h[k] - u.sea) * u.uplift;
}

// The most the relief rose since the last fill (`hn` the relief then),
// by the bits of the rise (in order for positive numbers), into `top`.
@compute @workgroup_size(16, 16)
fn rise(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    atomicMax(&top[0], bitcast<u32>(max(h[k] - hn[k], 0.0)));
}

// The last fill raised by that rise, as a start for the next.
@compute @workgroup_size(16, 16)
fn warm(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    if is_outlet(id.x, id.y, k) {
        w[k] = h[k];
    } else {
        w[k] = max(h[k], w[k] + bitcast<f32>(atomicLoad(&top[0])));
    }
}
