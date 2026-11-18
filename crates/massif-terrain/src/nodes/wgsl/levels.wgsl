struct U {
    in_lo: f32,
    span: f32,
    out_lo: f32,
    out_span: f32,
    inv_gamma: f32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> in0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out0: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.y * TILE + id.x;
    var t = clamp((in0[k] - u.in_lo) / u.span, 0.0, 1.0);
    if u.inv_gamma != 1.0 {
        t = pow(t, u.inv_gamma);
    }
    out0[k] = u.out_lo + u.out_span * t;
}
