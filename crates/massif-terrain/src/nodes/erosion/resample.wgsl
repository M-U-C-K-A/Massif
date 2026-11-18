// Coarser and finer grids, as `common::{down_avg, down_max, up_add}`.

struct R {
    n: u32,
    m: u32,
    with_base: u32,
}

@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read> base: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(16, 16)
fn down_avg(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= r.m || id.y >= r.m {
        return;
    }
    let f = vec2<i32>(2 * i32(id.x), 2 * i32(id.y));
    var s = 0.0;
    var w = 0.0;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dx = -1; dx <= 1; dx++) {
            let x = f.x + dx;
            let z = f.y + dz;
            if x >= 0 && z >= 0 && x < i32(r.n) && z < i32(r.n) {
                let wt = f32((2 - abs(dx)) * (2 - abs(dz)));
                s += wt * src[u32(z) * r.n + u32(x)];
                w += wt;
            }
        }
    }
    dst[id.y * r.m + id.x] = s / w;
}

@compute @workgroup_size(16, 16)
fn down_max(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= r.m || id.y >= r.m {
        return;
    }
    let f = vec2<i32>(2 * i32(id.x), 2 * i32(id.y));
    var s = -3.0e38;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dx = -1; dx <= 1; dx++) {
            let x = f.x + dx;
            let z = f.y + dz;
            if x >= 0 && z >= 0 && x < i32(r.n) && z < i32(r.n) {
                s = max(s, src[u32(z) * r.n + u32(x)]);
            }
        }
    }
    dst[id.y * r.m + id.x] = s;
}

fn up_at(x: u32, z: u32) -> f32 {
    let x0 = min(x / 2u, r.m - 1u);
    let z0 = min(z / 2u, r.m - 1u);
    let x1 = min(x0 + 1u, r.m - 1u);
    let z1 = min(z0 + 1u, r.m - 1u);
    let tx = select(0.0, 0.5, x % 2u == 1u);
    let tz = select(0.0, 0.5, z % 2u == 1u);
    let a = src[z0 * r.m + x0] + (src[z0 * r.m + x1] - src[z0 * r.m + x0]) * tx;
    let b = src[z1 * r.m + x0] + (src[z1 * r.m + x1] - src[z1 * r.m + x0]) * tx;
    return a + (b - a) * tz;
}

@compute @workgroup_size(16, 16)
fn up_add(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= r.n || id.y >= r.n {
        return;
    }
    let k = id.y * r.n + id.x;
    var b = 0.0;
    if r.with_base != 0u {
        b = base[k];
    }
    dst[k] = up_at(id.x, id.y) + b;
}
