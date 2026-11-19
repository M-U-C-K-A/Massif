// The sea, lakes and rivers, sample for sample as `water.rs` on the CPU.

struct U {
    n: u32,
    flags: u32,
    sp: f32,
    sea: f32,
    level: f32,
    shore: f32,
    amount: f32,
    lake_depth: f32,
    least: f32,
    top: f32,
    width: f32,
    depth: f32,
    banks: f32,
    meander: f32,
    meander_freq: f32,
    scale: f32,
}

const DRY: f32 = 64.0;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> h: array<f32>;
@group(0) @binding(2) var<storage, read> w: array<f32>;
@group(0) @binding(3) var<storage, read> acc: array<u32>;
@group(0) @binding(4) var<storage, read> lowest: array<u32>;
@group(0) @binding(5) var<storage, read_write> near: array<u32>;
@group(0) @binding(6) var<storage, read> water_in: array<f32>;
@group(0) @binding(7) var<storage, read_write> hp: array<f32>;
@group(0) @binding(8) var<storage, read_write> o_water: array<f32>;
@group(0) @binding(9) var<storage, read_write> o_mask: array<f32>;
@group(0) @binding(10) var<storage, read_write> ground: array<u32>;
@group(0) @binding(11) var<storage, read_write> stats: array<atomic<u32>>;
@group(0) @binding(12) var<storage, read_write> o_height: array<f32>;

fn band(v: f32, lo: f32, hi: f32, falloff: f32) -> f32 {
    let f = max(falloff, 1e-4);
    return min(clamp((v - lo) / f + 0.5, 0.0, 1.0), clamp((hi - v) / f + 0.5, 0.0, 1.0));
}

fn ord(v: f32) -> u32 {
    let b = bitcast<u32>(v);
    if (b & 0x80000000u) != 0u {
        return ~b;
    }
    return b | 0x80000000u;
}

fn unord(b: u32) -> f32 {
    if (b & 0x80000000u) != 0u {
        return bitcast<f32>(b & 0x7fffffffu);
    }
    return bitcast<f32>(~b);
}

@compute @workgroup_size(16, 16)
fn sea(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let v = h[k];
    let connected = u.flags != 0u;
    let is_wet = v < u.level && (!connected || w[k] < u.level);
    o_height[k] = v;
    o_water[k] = select(v - DRY, u.level, is_wet);
    o_mask[k] = band(v, u.level - u.shore, u.level + u.shore, u.shore * 0.5);
}

@compute @workgroup_size(16, 16)
fn lakes(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let v = h[k];
    let f = w[k];
    let depth = max(f - v, 0.0);
    let g = v + depth * u.amount;
    let held = depth * (1.0 - u.amount);
    o_height[k] = g;
    o_mask[k] = clamp(held / 1.5, 0.0, 1.0);
    o_water[k] = select(g - DRY, f, held >= 1.0 && f > u.sea + 0.2);
}

fn is_outlet(x: u32, z: u32, k: u32) -> bool {
    return x == 0u || z == 0u || x == u.n - 1u || z == u.n - 1u || hp[k] < u.sea;
}

// The ground the rivers see: where water already lies, an outlet.
@compute @workgroup_size(16, 16)
fn river_prep(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var v = h[k];
    if (u.flags & 1u) != 0u && water_in[k] > v + 0.5 {
        v = -1.0e6;
    }
    hp[k] = v;
}

// The land's samples and the largest drainage on land.
@compute @workgroup_size(16, 16)
fn river_stats(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    if !is_outlet(id.x, id.y, k) {
        atomicAdd(&stats[0], 1u);
        atomicMax(&stats[1], acc[k]);
    }
}

fn is_lake(x: u32, z: u32, k: u32) -> bool {
    return !is_outlet(x, z, k) && w[k] - h[k] >= u.lake_depth;
}

// The ground whose least upstream sets the rivers' level (a lake's at its
// surface), and the samples of rivers.
@compute @workgroup_size(16, 16)
fn river_seeds(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let lake = is_lake(id.x, id.y, k);
    ground[k] = ord(select(h[k], w[k], lake));
    let river = !is_outlet(id.x, id.y, k) && !lake && f32(acc[k]) >= u.least;
    near[k] = select(NONE, k, river);
}

// Where a sample looks for its river, moved by two noises: meanders, as
// `Rivers::wander`.
fn wander(x: f32, z: f32) -> vec2<f32> {
    if u.meander <= 0.0 {
        return vec2<f32>(x, z);
    }
    let seed = u.flags >> 1u;
    let a = x * u.meander_freq;
    let b = z * u.meander_freq;
    let last = f32(u.n - 1u);
    let wx = u.meander * wnoise(a, b, seed ^ 0x0a5a5a5au);
    let wz = u.meander * wnoise(a + 31.7, b + 17.3, seed ^ 0x05a5a5a5u);
    return vec2<f32>(clamp(x + wx, 0.0, last), clamp(z + wz, 0.0, last));
}

@compute @workgroup_size(16, 16)
fn river_carve(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let v = h[k];
    let pw = wander(f32(id.x), f32(id.y));
    let s = near[u32(floor(pw.y + 0.5)) * u.n + u32(floor(pw.x + 0.5))];
    var g = v;
    var mask = 0.0;
    var wet = false;
    var level = 0.0;
    if s != NONE {
        let dx = f32(s % u.n) - pw.x;
        let dz = f32(s / u.n) - pw.y;
        let d = sqrt(dx * dx + dz * dz) * u.sp;
        let share = clamp(f32(acc[s]) / u.top, 0.0, 1.0);
        level = unord(lowest[s]) - 0.25;
        let hw = 0.5 * max(max(u.width * sqrt(share), 0.25 * u.width), 2.0);
        let depth = max(u.depth * pow(share, 0.4), 1.0);
        let bank = max(max(u.banks * 2.0 * hw, u.sp), clamp(2.5 * (h[s] - level), 0.0, 48.0));
        if d <= hw {
            let t = d / hw;
            g = min(v, level - depth * (1.0 - t * t));
            mask = 1.0;
            wet = level > g + 0.1;
        } else if d <= hw + bank {
            let t = (d - hw) / bank;
            let sm = t * t * (3.0 - 2.0 * t);
            g = min(v, level + (v - level) * sm);
            mask = 1.0 - sm;
        }
    }
    var surface = g - DRY;
    if wet {
        surface = level;
    } else if is_lake(id.x, id.y, k) {
        surface = w[k];
    }
    if (u.flags & 1u) != 0u {
        surface = max(surface, water_in[k]);
    }
    o_height[k] = g;
    o_mask[k] = mask;
    o_water[k] = surface;
}
