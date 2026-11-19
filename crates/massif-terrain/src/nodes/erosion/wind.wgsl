// Wind and dunes, step for step as `wind.rs` on the CPU.

struct U {
    n: u32,
    has_mask: u32,
    has_wind: u32,
    reach: u32,
    dir_x: f32,
    dir_z: f32,
    hop: f32,
    lift: f32,
    sand: f32,
    abrasion: f32,
    sp: f32,
    seed: u32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    pad3: f32,
}

const SHADOW: f32 = 0.268;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read_write> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> sand: array<f32>;
@group(0) @binding(3) var<storage, read_write> fly: array<f32>;
@group(0) @binding(4) var<storage, read_write> carry: array<f32>;
@group(0) @binding(5) var<storage, read_write> lee: array<f32>;
@group(0) @binding(6) var<storage, read> mask: array<f32>;
@group(0) @binding(7) var<storage, read> wind: array<f32>;
@group(0) @binding(8) var<storage, read_write> s: array<f32>;
@group(0) @binding(9) var<storage, read_write> loose: array<f32>;
@group(0) @binding(10) var<storage, read_write> worn: array<f32>;
@group(0) @binding(11) var<storage, read> s2: array<f32>;
@group(0) @binding(12) var<storage, read_write> o0: array<f32>;
@group(0) @binding(13) var<storage, read_write> o1: array<f32>;
@group(0) @binding(14) var<storage, read_write> o2: array<f32>;

fn dir_at(k: u32) -> vec2<f32> {
    if u.has_wind != 0u {
        let v = vec2<f32>(wind[k], wind[u.n * u.n + k]);
        let l = sqrt(v.x * v.x + v.y * v.y);
        if l > 1e-6 {
            return v / l;
        }
    }
    return vec2<f32>(u.dir_x, u.dir_z);
}

fn near(x: f32, z: f32) -> f32 {
    let i = clamp(i32(floor(x + 0.5)), 0, i32(u.n) - 1);
    let j = clamp(i32(floor(z + 0.5)), 0, i32(u.n) - 1);
    return s[u32(j) * u.n + u32(i)];
}

fn carried(x: f32, z: f32) -> f32 {
    let fx = floor(x);
    let fz = floor(z);
    let tx = x - fx;
    let tz = z - fz;
    let a = at_carry(fx, fz) + (at_carry(fx + 1.0, fz) - at_carry(fx, fz)) * tx;
    let c = at_carry(fx, fz + 1.0) + (at_carry(fx + 1.0, fz + 1.0) - at_carry(fx, fz + 1.0)) * tx;
    return a + (c - a) * tz;
}

fn at_carry(i: f32, j: f32) -> f32 {
    if i < 0.0 || j < 0.0 || i >= f32(u.n) || j >= f32(u.n) {
        return 0.0;
    }
    return carry[u32(j) * u.n + u32(i)];
}

@compute @workgroup_size(16, 16)
fn init(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var m = 1.0;
    if u.has_mask != 0u {
        m = clamp(mask[k], 0.0, 1.0);
    }
    sand[k] = u.sand * m * (0.8 + 0.4 * rand01(id.x, id.y, u.seed));
}

@compute @workgroup_size(16, 16)
fn compose(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    s[k] = b[k] + sand[k];
    loose[k] = 1.0 - clamp(sand[k] / 0.5, 0.0, 1.0);
}

@compute @workgroup_size(16, 16)
fn lift(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let dir = dir_at(k);
    let here = s[k];
    var shadow = false;
    for (var j = 1u; j <= u.reach; j++) {
        let t = f32(j);
        let up = near(f32(id.x) - dir.x * t, f32(id.y) - dir.y * t);
        if up - here > SHADOW * t * u.sp {
            shadow = true;
            break;
        }
    }
    lee[k] = select(0.0, 1.0, shadow);
    var q = 0.0;
    if !shadow {
        q = min(sand[k], u.lift);
    }
    let left = sand[k] - q;
    sand[k] = left;
    carry[k] = fly[k] + q;
    if !shadow && left < 0.05 && u.abrasion > 0.0 {
        let ahead = near(f32(id.x) + dir.x, f32(id.y) + dir.y);
        let face = max((ahead - here) / u.sp, 0.0);
        let e = u.abrasion * 0.05 * (0.2 + face);
        b[k] -= e;
        worn[k] += e;
    }
}

@compute @workgroup_size(16, 16)
fn land(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let dir = dir_at(k);
    let landed = carried(f32(id.x) - dir.x * u.hop, f32(id.y) - dir.y * u.hop);
    var keep = 0.4;
    if lee[k] != 0.0 {
        keep = 1.0;
    } else if sand[k] > 0.05 {
        keep = 0.6;
    }
    let d = landed * keep;
    sand[k] += d;
    fly[k] = landed - d;
}

@compute @workgroup_size(16, 16)
fn sand_back(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    sand[k] = max(s2[k] - b[k], 0.0);
}

// The relief (rock and sand), the sand and the wear (its reference in
// `abrasion`).
@compute @workgroup_size(16, 16)
fn outputs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    o0[k] = b[k] + sand[k];
    o1[k] = clamp(sand[k] / 8.0, 0.0, 1.0);
    let w = max(worn[k], 0.0);
    o2[k] = w / (w + u.abrasion);
}
