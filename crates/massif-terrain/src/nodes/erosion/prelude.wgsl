// What every erosion shader shares (after the terrain's common WGSL): the
// eight neighbours in the order of `common.rs`, and random numbers.

const NONE: u32 = 0xffffffffu;

var<private> OFF: array<vec2<i32>, 8> = array<vec2<i32>, 8>(
    vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0),
    vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1),
);

var<private> DIST: array<f32, 8> = array<f32, 8>(1.4142135, 1.0, 1.4142135, 1.0, 1.0, 1.4142135, 1.0, 1.4142135);

// A number from 0 to 1, as `common::rand01`.
fn rand01(x: u32, z: u32, seed: u32) -> f32 {
    return f32(hash2(x, z, seed) >> 8u) * (1.0 / 16777216.0);
}

// Neighbour `d` of `(x, z)` in a grid `n` a side, or NONE outside.
fn nb_of(x: u32, z: u32, n: u32, d: u32) -> u32 {
    let p = vec2<i32>(i32(x), i32(z)) + OFF[d];
    if p.x < 0 || p.y < 0 || p.x >= i32(n) || p.y >= i32(n) {
        return NONE;
    }
    return u32(p.y) * n + u32(p.x);
}

// Smooth noise from about -1 to 1, as `common::wnoise`.
fn wnoise(x: f32, z: f32, seed: u32) -> f32 {
    let fx = floor(x);
    let fz = floor(z);
    return gradient(bitcast<u32>(i32(fx)), bitcast<u32>(i32(fz)), x - fx, z - fz, seed);
}

fn ease(t0: f32) -> f32 {
    let t = clamp(t0, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
