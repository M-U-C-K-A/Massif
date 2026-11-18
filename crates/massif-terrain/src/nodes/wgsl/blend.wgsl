struct U {
    mode: u32,
    strength: f32,
    has_mask: u32,
}

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> a_in: array<f32>;
@group(0) @binding(2) var<storage, read> b_in: array<f32>;
@group(0) @binding(3) var<storage, read> mask_in: array<f32>;
@group(0) @binding(4) var<storage, read_write> out0: array<f32>;

fn blend(mode: u32, a: f32, b: f32) -> f32 {
    switch mode {
        case 0u: { return a + b; }
        case 1u: { return a - b; }
        case 2u: { return a * b; }
        case 3u: { return max(a, b); }
        case 4u: { return min(a, b); }
        case 5u: { return (a + b) * 0.5; }
        case 6u: { return abs(a - b); }
        case 7u: { return 1.0 - (1.0 - a) * (1.0 - b); }
        case 8u: {
            if a < 0.5 {
                return 2.0 * a * b;
            }
            return 1.0 - 2.0 * (1.0 - a) * (1.0 - b);
        }
        default: { return b; }
    }
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let k = id.y * TILE + id.x;
    let a = a_in[k];
    var w = u.strength;
    if u.has_mask != 0u {
        w = w * clamp(mask_in[k], 0.0, 1.0);
    }
    out0[k] = a + (blend(u.mode, a, b_in[k]) - a) * w;
}
