// What every terrain kernel shares: the noise of `noise.rs`, line for line.

const TILE: u32 = 256u;

var<private> GRADS: array<vec2<f32>, 16> = array<vec2<f32>, 16>(
    vec2<f32>(0.9807853, 0.19509032),
    vec2<f32>(0.8314696, 0.55557024),
    vec2<f32>(0.55557024, 0.8314696),
    vec2<f32>(0.19509032, 0.9807853),
    vec2<f32>(-0.19509032, 0.9807853),
    vec2<f32>(-0.55557024, 0.8314696),
    vec2<f32>(-0.8314696, 0.55557024),
    vec2<f32>(-0.9807853, 0.19509032),
    vec2<f32>(-0.9807853, -0.19509032),
    vec2<f32>(-0.8314696, -0.55557024),
    vec2<f32>(-0.55557024, -0.8314696),
    vec2<f32>(-0.19509032, -0.9807853),
    vec2<f32>(0.19509032, -0.9807853),
    vec2<f32>(0.55557024, -0.8314696),
    vec2<f32>(0.8314696, -0.55557024),
    vec2<f32>(0.9807853, -0.19509032),
);

const NORM: f32 = 1.4142135;

fn mix32(x0: u32) -> u32 {
    var x = x0;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn hash2(x: u32, z: u32, seed: u32) -> u32 {
    return mix32((x * 0x8da6b343u) ^ (z * 0xd8163841u) ^ seed);
}

fn fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn corner(cx: u32, cz: u32, seed: u32, ox: f32, oz: f32) -> f32 {
    let g = GRADS[hash2(cx, cz, seed) & 15u];
    return g.x * ox + g.y * oz;
}

fn gradient(cx: u32, cz: u32, fx: f32, fz: f32, seed: u32) -> f32 {
    let n00 = corner(cx, cz, seed, fx, fz);
    let n10 = corner(cx + 1u, cz, seed, fx - 1.0, fz);
    let n01 = corner(cx, cz + 1u, seed, fx, fz - 1.0);
    let n11 = corner(cx + 1u, cz + 1u, seed, fx - 1.0, fz - 1.0);
    let ux = fade(fx);
    let uz = fade(fz);
    let a = n00 + (n10 - n00) * ux;
    let b = n01 + (n11 - n01) * ux;
    return (a + (b - a) * uz) * NORM;
}

struct Octave {
    cell: vec2<i32>,
    frac: vec2<f32>,
    step: vec4<f32>,
    amp: f32,
    seed: u32,
    pad: vec2<u32>,
}

fn octave_at(o: Octave, di: f32, dj: f32) -> f32 {
    let qx = o.frac.x + o.step.x * di + o.step.y * dj;
    let qz = o.frac.y + o.step.z * di + o.step.w * dj;
    let fx = floor(qx);
    let fz = floor(qz);
    let cx = bitcast<u32>(o.cell.x + i32(fx));
    let cz = bitcast<u32>(o.cell.y + i32(fz));
    return gradient(cx, cz, qx - fx, qz - fz, o.seed);
}
