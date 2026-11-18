// Thermal erosion on the graphics card, pass for pass as `thermal_pass` on
// the CPU: `excess` finds what each sample sheds, `gather` what each
// receives. The neighbours are visited in the same order.

struct U {
    n: u32,
    strength: f32,
    drop_axis: f32,
    drop_diag: f32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> h: array<f32>;
@group(0) @binding(2) var<storage, read_write> shed: array<f32>;
@group(0) @binding(3) var<storage, read_write> total: array<f32>;
@group(0) @binding(4) var<storage, read_write> next: array<f32>;

var<private> NB: array<vec3<i32>, 8> = array<vec3<i32>, 8>(
    vec3<i32>(-1, -1, 1), vec3<i32>(0, -1, 0), vec3<i32>(1, -1, 1), vec3<i32>(-1, 0, 0),
    vec3<i32>(1, 0, 0), vec3<i32>(-1, 1, 1), vec3<i32>(0, 1, 0), vec3<i32>(1, 1, 1),
);

fn inside(x: i32, z: i32) -> bool {
    return x >= 0 && z >= 0 && x < i32(u.n) && z < i32(u.n);
}

fn drop_of(d: i32) -> f32 {
    if d == 1 {
        return u.drop_diag;
    }
    return u.drop_axis;
}

@compute @workgroup_size(16, 16)
fn excess(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let c = h[k];
    var sum = 0.0;
    var most = 0.0;
    for (var i = 0; i < 8; i++) {
        let o = NB[i];
        let x = i32(id.x) + o.x;
        let z = i32(id.y) + o.y;
        if !inside(x, z) {
            continue;
        }
        let e = c - h[u32(z) * u.n + u32(x)] - drop_of(o.z);
        if e > 0.0 {
            sum += e;
            most = max(most, e);
        }
    }
    shed[k] = u.strength * 0.5 * most;
    total[k] = sum;
}

@compute @workgroup_size(16, 16)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    let c = h[k];
    var v = c - shed[k];
    for (var i = 0; i < 8; i++) {
        let o = NB[i];
        let x = i32(id.x) + o.x;
        let z = i32(id.y) + o.y;
        if !inside(x, z) {
            continue;
        }
        let j = u32(z) * u.n + u32(x);
        let e = h[j] - c - drop_of(o.z);
        if e > 0.0 && total[j] > 0.0 {
            v += shed[j] * e / total[j];
        }
    }
    next[k] = v;
}

// The talus: where material came to rest, 1 for 2 blocks and more.
@compute @workgroup_size(16, 16)
fn talus(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.n || id.y >= u.n {
        return;
    }
    let k = id.y * u.n + id.x;
    // `h` holds the input here, `next` the worn relief, `shed` the talus
    // (and `total` is cleared, which keeps every binding in the layout).
    shed[k] = clamp((next[k] - h[k]) / 2.0, 0.0, 1.0);
    total[k] = 0.0;
}
