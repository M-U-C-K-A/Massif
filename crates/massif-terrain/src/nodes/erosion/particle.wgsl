// Particle erosion, drop for drop as `particle.rs` on the CPU: every drop
// of a pass runs over the relief as it stood when the pass began, and adds
// what it wears and lays to whole-number grids.

struct U {
    n: u32,
    m: u32,
    seed: u32,
    r: i32,
    sp: f32,
    erosion: f32,
    deposition: f32,
    capacity: f32,
    evaporation: f32,
    inertia: f32,
    brush_total: f32,
    sea: f32,
    has_rain: u32,
    has_hard: u32,
    steps: u32,
    apply_scale: f32,
}

const STRIDE: f32 = 4.0;
const GRAVITY: f32 = 4.0;
const MIN_CAPACITY: f32 = 0.01;
const DELTA: f32 = 65536.0;
const WATER: f32 = 1024.0;
const SOIL: f32 = 4096.0;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> h: array<f32>;
@group(0) @binding(2) var<storage, read_write> delta: array<atomic<i32>>;
@group(0) @binding(3) var<storage, read_write> flow: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> laid: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> worn: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read> rain: array<f32>;
@group(0) @binding(7) var<storage, read> hard: array<f32>;
@group(0) @binding(8) var<storage, read_write> hw: array<f32>;
@group(0) @binding(9) var<storage, read_write> tops: array<atomic<u32>>;
@group(0) @binding(10) var<storage, read_write> o0: array<f32>;
@group(0) @binding(11) var<storage, read_write> o1: array<f32>;
@group(0) @binding(12) var<storage, read_write> o2: array<f32>;

fn fix(v: f32, unit: f32) -> i32 {
    return i32(floor(v * unit + 0.5));
}

fn at(i: u32, j: u32) -> f32 {
    return h[j * u.n + i] / u.sp;
}

// Height and slope at `(x, z)`, bilinear.
fn height_grad(x: f32, z: f32) -> vec3<f32> {
    let i = u32(x);
    let j = u32(z);
    let fu = x - f32(i);
    let fv = z - f32(j);
    let a = at(i, j);
    let b = at(i + 1u, j);
    let c = at(i, j + 1u);
    let d = at(i + 1u, j + 1u);
    let gx = (b - a) * (1.0 - fv) + (d - c) * fv;
    let gz = (c - a) * (1.0 - fu) + (d - b) * fu;
    let height = a * (1.0 - fu) * (1.0 - fv) + b * fu * (1.0 - fv) + c * (1.0 - fu) * fv + d * fu * fv;
    return vec3<f32>(height, gx, gz);
}

fn lay(cell: u32, fu: f32, fv: f32, amount: f32) {
    atomicAdd(&delta[cell], fix(amount * (1.0 - fu) * (1.0 - fv), DELTA));
    atomicAdd(&delta[cell + 1u], fix(amount * fu * (1.0 - fv), DELTA));
    atomicAdd(&delta[cell + u.n], fix(amount * (1.0 - fu) * fv, DELTA));
    atomicAdd(&delta[cell + u.n + 1u], fix(amount * fu * fv, DELTA));
    atomicAdd(&laid[cell], u32(fix(amount, SOIL)));
}

fn wear(i: u32, j: u32, amount: f32) {
    let r = u.r;
    for (var dz = -r; dz <= r; dz++) {
        let cj = i32(j) + dz;
        if cj < 0 || cj >= i32(u.n) {
            continue;
        }
        for (var dx = -r; dx <= r; dx++) {
            let ci = i32(i) + dx;
            if ci < 0 || ci >= i32(u.n) {
                continue;
            }
            let d = sqrt(f32(dx * dx + dz * dz));
            if d <= f32(r) {
                let wt = (1.0 - d / (f32(r) + 1.0)) / u.brush_total;
                atomicSub(&delta[u32(cj) * u.n + u32(ci)], fix(amount * wt, DELTA));
            }
        }
    }
    atomicAdd(&worn[j * u.n + i], u32(fix(amount, SOIL)));
}

@compute @workgroup_size(16, 16)
fn drop(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.m || id.y >= u.m {
        return;
    }
    let last = f32(u.n - 1u);
    var x = min(f32(id.x) * STRIDE + rand01(id.x, id.y, u.seed ^ 0x6d2b79f5u) * STRIDE, last - 0.001);
    var z = min(f32(id.y) * STRIDE + rand01(id.x, id.y, u.seed ^ 0x1b873593u) * STRIDE, last - 0.001);
    if u.has_rain != 0u {
        let k = u32(z) * u.n + u32(x);
        if rand01(id.x, id.y, u.seed ^ 0x2545f491u) >= clamp(rain[k], 0.0, 1.0) {
            return;
        }
    }
    var dx = 0.0;
    var dz = 0.0;
    var speed = 1.0;
    var water = 1.0;
    var sediment = 0.0;
    let sea = u.sea / u.sp;
    for (var step = 0u; step < u.steps; step++) {
        let i = u32(x);
        let j = u32(z);
        let fu = x - f32(i);
        let fv = z - f32(j);
        let hg = height_grad(x, z);
        let cell = j * u.n + i;
        if hg.x < sea {
            lay(cell, fu, fv, sediment);
            return;
        }
        var ndx = dx * u.inertia - hg.y * (1.0 - u.inertia);
        var ndz = dz * u.inertia - hg.z * (1.0 - u.inertia);
        let len = sqrt(ndx * ndx + ndz * ndz);
        if len < 1e-6 {
            return;
        }
        ndx /= len;
        ndz /= len;
        let nx = x + ndx;
        let nz = z + ndz;
        if nx < 0.0 || nx >= last || nz < 0.0 || nz >= last {
            return;
        }
        let dh = height_grad(nx, nz).x - hg.x;
        let cap = max(-dh * speed * water * u.capacity, MIN_CAPACITY);
        if sediment > cap || dh > 0.0 {
            var amount = (sediment - cap) * u.deposition;
            if dh > 0.0 {
                amount = min(dh, sediment);
            }
            sediment -= amount;
            lay(cell, fu, fv, amount);
        } else {
            var soft = 1.0;
            if u.has_hard != 0u {
                soft = 1.0 - 0.9 * clamp(hard[cell], 0.0, 1.0);
            }
            let amount = min((cap - sediment) * u.erosion, -dh) * soft;
            wear(i, j, amount);
            sediment += amount;
        }
        atomicAdd(&flow[cell], u32(fix(water, WATER)));
        speed = sqrt(max(speed * speed + dh * GRAVITY, 0.0));
        water *= 1.0 - u.evaporation;
        x = nx;
        z = nz;
        dx = ndx;
        dz = ndz;
    }
}

// The changes of the pass applied, and cleared for the next.
@compute @workgroup_size(16, 16)
fn apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    hw[k] += f32(atomicExchange(&delta[k], 0)) * u.apply_scale;
}

@compute @workgroup_size(16, 16)
fn top_of(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    atomicMax(&tops[0], atomicLoad(&flow[k]));
    atomicMax(&tops[1], atomicLoad(&worn[k]));
    atomicMax(&tops[2], atomicLoad(&laid[k]));
}

fn log_mask(v: u32, unit: f32, top: u32) -> f32 {
    let l = log(1.0 + f32(max(top, 1u)) / unit);
    return clamp(log(1.0 + f32(v) / unit) / l, 0.0, 1.0);
}

// Flow, wear and deposition, as `particle::log_mask` makes them.
@compute @workgroup_size(16, 16)
fn masks(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    o0[k] = log_mask(atomicLoad(&flow[k]), WATER, atomicLoad(&tops[0]));
    o1[k] = log_mask(atomicLoad(&worn[k]), SOIL, atomicLoad(&tops[1]));
    o2[k] = log_mask(atomicLoad(&laid[k]), SOIL, atomicLoad(&tops[2]));
}
