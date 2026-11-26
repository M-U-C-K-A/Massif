// The parameters of a frame, shared by the path tracer (trace.wgsl) and the
// denoiser (denoise.wgsl), and the tone mapping both end with.

struct Params {
    eye: vec4<f32>,      // xyz; w: tan(fov / 2)
    right: vec4<f32>,    // w: width / height
    up: vec4<f32>,       // w: the angle of a pixel, radians
    forward: vec4<f32>,  // w: exposure
    sun: vec4<f32>,      // direction toward the sun; w: cos of its radius
    sun_light: vec4<f32>,  // the sun's illuminance above the air (tinted); w: the sky's strength
    air: vec4<f32>,      // the air's scattering, the haze's, its anisotropy, the eyes' height (km)
    fog: vec4<f32>,      // extinction a block at its base, base height (scene), falloff (blocks), anisotropy
    media: vec4<f32>,    // shafts on, sea level (scene), emission strength, the lens's radius
    lens: vec4<f32>,     // focus distance, a sea around the map, where the shaped blocks' faces start, the first row drawn
    clouds: vec4<f32>,   // cover (0..1), base height (scene), thickness, extinction a block (0: none)
    wind: vec4<f32>,     // how far the wind moved them (x, z), 1 / the size of their shapes; w: the most a bounce's light brings (0: no limit)
    water: vec4<f32>,    // the waves' slope, 1 / their size, time (s), how much light water takes (0: water as the game's texture)
    wet: vec4<f32>,      // wetness, puddles, 1 / the puddles' size, a pack's LabPBR maps on
    size: vec4<u32>,     // width, height, seed of the frame, sample (0: start over)
    grid: vec4<i32>,     // chunks across, chunks down, height in blocks, sections
    info: vec4<u32>,     // words a chunk's record, pyramid levels, bounces, shift (pixels per side of a ray: 1 << shift)
    flags: vec4<u32>,    // the image left to the denoiser (not written here), blades of grass a block, their height (f32 bits); unused
    level_off: array<vec4<u32>, 4>,
    level_w: array<vec4<u32>, 4>,
}

// ACES, fitted (Narkowicz), then sRGB's gamma.
fn tone(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let x = c * exposure;
    let mapped = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
    return pow(mapped, vec3<f32>(1.0 / 2.2));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
