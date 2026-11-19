// The water's way over a relief, as `flow.rs` finds it on the CPU:
// hollows filled up to where they spill (each sample a little above the
// one it drains to), each sample's receiver (its steepest way down), and
// the area draining through each sample, added up along the receivers by
// jumps that double each step (whole numbers, so the order of the
// additions does not matter).

struct U {
    n: u32,
    m: u32,
    sea: f32,
    eps: f32,
    margin: f32,
    with_rain: u32,
    scale: f32,
    gamma: f32,
    seed: u32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> h: array<f32>;
@group(0) @binding(2) var<storage, read> wi: array<f32>;
@group(0) @binding(3) var<storage, read_write> wo: array<f32>;
@group(0) @binding(4) var<storage, read_write> count: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> rcv: array<u32>;
@group(0) @binding(6) var<storage, read> rain: array<f32>;
@group(0) @binding(7) var<storage, read_write> d0: array<atomic<u32>>;
@group(0) @binding(8) var<storage, read_write> d1: array<atomic<u32>>;
@group(0) @binding(9) var<storage, read_write> j0: array<u32>;
@group(0) @binding(10) var<storage, read_write> j1: array<u32>;

fn outlet(x: u32, z: u32, k: u32) -> bool {
    return x == 0u || z == 0u || x == u.n - 1u || z == u.n - 1u || h[k] < u.sea;
}

// The coarsest level: outlets at their height, the rest as high as can be.
@compute @workgroup_size(16, 16)
fn fill_top(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    wo[k] = select(1.0e30, h[k], outlet(id.x, id.y, k));
}

// A finer level from the coarser one's surface (`wi`, `m` a side): above
// what the finer one will be, which the steps then lower.
@compute @workgroup_size(16, 16)
fn fill_up(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    if outlet(id.x, id.y, k) {
        wo[k] = h[k];
        return;
    }
    let cx = min((id.x + 1u) / 2u, u.m - 1u);
    let cz = min((id.y + 1u) / 2u, u.m - 1u);
    wo[k] = max(h[k], wi[cz * u.m + cx] + u.margin);
}

@compute @workgroup_size(16, 16)
fn fill_step(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let c = wi[k];
    if outlet(id.x, id.y, k) {
        wo[k] = h[k];
        return;
    }
    var lo = 3.0e38;
    for (var d = 0u; d < 8u; d++) {
        let j = nb_of(id.x, id.y, u.n, d);
        if j != NONE {
            lo = min(lo, wi[j]);
        }
    }
    // Each sample its own rise, so that water wanders over flats instead
    // of running in straight lines.
    let v = min(c, max(h[k], lo + u.eps * (0.25 + 1.5 * rand01(id.x, id.y, u.seed))));
    if v < c {
        atomicAdd(&count[0], 1u);
    }
    wo[k] = v;
}

@compute @workgroup_size(16, 16)
fn receivers(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var best = NONE;
    if !outlet(id.x, id.y, k) {
        var s = 0.0;
        let c = wi[k];
        for (var d = 0u; d < 8u; d++) {
            let j = nb_of(id.x, id.y, u.n, d);
            if j != NONE {
                let t = (c - wi[j]) / DIST[d] * (0.7 + 0.6 * rand01(k, d, u.seed));
                if t > s {
                    s = t;
                    best = j;
                }
            }
        }
    }
    rcv[k] = best;
}

@compute @workgroup_size(16, 16)
fn acc_init(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var r = 1.0;
    if u.with_rain != 0u {
        r = clamp(rain[k], 0.0, 1.0);
    }
    atomicStore(&d0[k], u32(round(r * u.scale)));
    j0[k] = rcv[k];
}

// D1 = D0, each jump doubled; counts the samples still jumping.
@compute @workgroup_size(16, 16)
fn acc_copy(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    atomicStore(&d1[k], atomicLoad(&d0[k]));
    let j = j0[k];
    if j == NONE {
        j1[k] = NONE;
    } else {
        j1[k] = j0[j];
        atomicAdd(&count[0], 1u);
    }
}

// What reaches each sample within the next jump.
@compute @workgroup_size(16, 16)
fn acc_add(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let j = j0[k];
    if j != NONE {
        atomicAdd(&d1[j], atomicLoad(&d0[k]));
    }
}

// The draining area as a mask, as `flow::flow_mask`.
@compute @workgroup_size(16, 16)
fn flow_mask(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let a = f32(atomicLoad(&d0[k])) / u.scale;
    let n2 = f32(u.n) * f32(u.n);
    wo[k] = pow(clamp(log(1.0 + a) / log(1.0 + n2), 0.0, 1.0), u.gamma);
}

// The least, as `acc_init` starts the sums: each sample's value (`rain`
// holding whole numbers in its bits) and its receiver.
@compute @workgroup_size(16, 16)
fn min_init(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    atomicStore(&d0[k], bitcast<u32>(rain[k]));
    j0[k] = rcv[k];
}

// The least of what reaches each sample within the next jump.
@compute @workgroup_size(16, 16)
fn min_add(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let j = j0[k];
    if j != NONE {
        atomicMin(&d1[j], atomicLoad(&d0[k]));
    }
}

fn dist2(x: u32, z: u32, s: u32) -> u32 {
    if s == NONE {
        return 0xffffffffu;
    }
    let dx = i32(s % u.n) - i32(x);
    let dz = i32(s / u.n) - i32(z);
    return u32(dx * dx + dz * dz);
}

// One jump of the nearest-seed search (`u.m` the jump), `j0` into `j1`, as
// `flow::nearest`.
@compute @workgroup_size(16, 16)
fn jfa_step(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    var best = j0[k];
    var bd = dist2(id.x, id.y, best);
    let s = i32(u.m);
    for (var dz = -1; dz <= 1; dz++) {
        for (var dx = -1; dx <= 1; dx++) {
            let cx = i32(id.x) + dx * s;
            let cz = i32(id.y) + dz * s;
            if cx < 0 || cz < 0 || cx >= i32(u.n) || cz >= i32(u.n) {
                continue;
            }
            let c = j0[u32(cz) * u.n + u32(cx)];
            if c == NONE {
                continue;
            }
            let d = dist2(id.x, id.y, c);
            if d < bd || (d == bd && c < best) {
                best = c;
                bd = d;
            }
        }
    }
    j1[k] = best;
}
