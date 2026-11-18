struct U {
    from: f32,
    to: f32,
    soft: f32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> in0: array<f32>;
@group(0) @binding(2) var<storage, read_write> out0: array<f32>;

fn step_up(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.y * TILE + id.x;
    let v = in0[k];
    out0[k] = step_up(u.from - u.soft, u.from + u.soft, v) * (1.0 - step_up(u.to - u.soft, u.to + u.soft, v));
}
