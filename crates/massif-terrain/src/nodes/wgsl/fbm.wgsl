struct U {
    style: u32,
    count: u32,
    height: f32,
    base: f32,
    oct: array<Octave, 16>,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read_write> out0: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let di = f32(id.x);
    let dj = f32(id.y);
    var sum = 0.0;
    var norm = 0.0;
    var weight = 1.0;
    for (var k = 0u; k < u.count; k++) {
        let o = u.oct[k];
        let n = octave_at(o, di, dj);
        var v = n;
        if u.style == 2u {
            v = 2.0 * abs(n) - 1.0;
        } else if u.style == 1u {
            var r = 1.0 - abs(n);
            r = r * r;
            r = r * weight;
            weight = clamp(r * 2.0, 0.0, 1.0);
            v = r;
        }
        sum += v * o.amp;
        norm += o.amp;
    }
    var v = sum / max(norm, 1e-6);
    if u.style != 1u {
        v = 0.5 + 0.5 * v;
    }
    out0[id.y * TILE + id.x] = u.base + u.height * v;
}
