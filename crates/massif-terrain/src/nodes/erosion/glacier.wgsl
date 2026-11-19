// Glaciers, step for step as `glacier.rs` on the CPU.

struct U {
    n: u32,
    has_snow: u32,
    sp: f32,
    ela: f32,
    beta: f32,
    deform: f32,
    slide: f32,
    wear: f32,
    moraines: f32,
    d_max: f32,
    wear_ref: f32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read> h: array<f32>;
@group(0) @binding(3) var<storage, read_write> b2: array<f32>;
@group(0) @binding(4) var<storage, read_write> h2: array<f32>;
@group(0) @binding(5) var<storage, read_write> d: array<f32>;
@group(0) @binding(6) var<storage, read_write> slide: array<f32>;
@group(0) @binding(7) var<storage, read_write> worn: array<f32>;
@group(0) @binding(8) var<storage, read> snow: array<f32>;
@group(0) @binding(9) var<storage, read> b0: array<f32>;
@group(0) @binding(10) var<storage, read_write> o0: array<f32>;
@group(0) @binding(11) var<storage, read_write> o1: array<f32>;
@group(0) @binding(12) var<storage, read_write> o2: array<f32>;
@group(0) @binding(13) var<storage, read_write> o3: array<f32>;

fn s_at(k: u32) -> f32 {
    return b[k] + h[k];
}

@compute @workgroup_size(16, 16)
fn diffusivity(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let x = id.x;
    let z = id.y;
    let k = z * u.n + x;
    let xl = select(x - 1u, 0u, x == 0u);
    let xr = min(x + 1u, u.n - 1u);
    let zl = select(z - 1u, 0u, z == 0u);
    let zr = min(z + 1u, u.n - 1u);
    let gx = (s_at(z * u.n + xr) - s_at(z * u.n + xl)) / (f32(max(xr - xl, 1u)) * u.sp);
    let gz = (s_at(zr * u.n + x) - s_at(zl * u.n + x)) / (f32(max(zr - zl, 1u)) * u.sp);
    let grad = sqrt(gx * gx + gz * gz);
    let g2 = grad * grad;
    let hk = h[k];
    d[k] = min(u.deform * pow(hk, 5.0) * g2 + u.slide * hk * hk * hk * grad, u.d_max);
    slide[k] = min(u.slide * hk * hk * g2, u.sp);
}

fn balance(s: f32, sn: f32) -> f32 {
    let m = u.beta * (s - u.ela);
    if m > 0.0 {
        return min(m * sn, 0.5);
    }
    return max(m, -2.0);
}

@compute @workgroup_size(16, 16)
fn step(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let s = s_at(k);
    var flux = 0.0;
    for (var i = 0; i < 4; i++) {
        var o = vec2<i32>(-1, 0);
        if i == 1 {
            o = vec2<i32>(1, 0);
        } else if i == 2 {
            o = vec2<i32>(0, -1);
        } else if i == 3 {
            o = vec2<i32>(0, 1);
        }
        let nx = i32(id.x) + o.x;
        let nz = i32(id.y) + o.y;
        if nx < 0 || nz < 0 || nx >= i32(u.n) || nz >= i32(u.n) {
            continue;
        }
        let j = u32(nz) * u.n + u32(nx);
        flux += 0.5 * (d[k] + d[j]) * (s_at(j) - s);
    }
    var sn = 1.0;
    if u.has_snow != 0u {
        sn = clamp(snow[k], 0.0, 1.0);
    }
    let ice = max(h[k] + flux / (u.sp * u.sp) + balance(s, sn), 0.0);
    let e = u.wear * slide[k] * min(h[k] / 5.0, 1.0);
    h2[k] = ice;
    b2[k] = b[k] - e;
    worn[k] += e;
}

fn margin(k: u32) -> f32 {
    let x = i32(k % u.n);
    let z = i32(k / u.n);
    var most = 0.0;
    for (var dz = -2; dz <= 2; dz++) {
        for (var dx = -2; dx <= 2; dx++) {
            let nx = x + dx;
            let nz = z + dz;
            if nx >= 0 && nz >= 0 && nx < i32(u.n) && nz < i32(u.n) {
                most = max(most, h[u32(nz) * u.n + u32(nx)]);
            }
        }
    }
    return clamp(min(most / 8.0, 1.0) - h[k] / 2.0, 0.0, 1.0);
}

// The change of the bed (moraines laid), the ice, the wear and the
// moraines, as `glacier_cpu` makes them.
@compute @workgroup_size(16, 16)
fn outputs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let laid = margin(k);
    o0[k] = b[k] - b0[k] + u.moraines * laid;
    o1[k] = clamp(h[k] / 30.0, 0.0, 1.0);
    let w = max(worn[k], 0.0);
    o2[k] = w / (w + u.wear_ref);
    o3[k] = laid;
}

// `b` plus `b0`, into `b2`.
@compute @workgroup_size(16, 16)
fn plus(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    b2[k] = b[k] + b0[k];
}
