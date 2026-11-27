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
    flags: vec4<u32>,    // the image left to the denoiser (not written here), blades of grass a block, their height (f32 bits), the look (0: filmic, 1: ACES)
    level_off: array<vec4<u32>, 4>,
    level_w: array<vec4<u32>, 4>,
}

// Light to the display's values, by the look (P.flags.w): filmic, as
// Blender's Filmic and AgX go (each channel along an S curve over 16.5
// stops of light, mid grey 0.18 to 0.46: soft shadows, highlights that
// whiten instead of burning to a hue), or ACES fitted (Narkowicz), more
// contrast and colour.
fn tone(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let x = max(c * exposure, vec3<f32>(0.0));
    if (P.flags.w == 1u) {
        let mapped = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
        return pow(mapped, vec3<f32>(1.0 / 2.2));
    }
    return filmic(x);
}

fn filmic(x: vec3<f32>) -> vec3<f32> {
    // The colours a little richer first: the curve, channel by channel,
    // greys them.
    let d = max(mix(vec3<f32>(dot(x, vec3<f32>(0.2126, 0.7152, 0.0722))), x, 1.08), vec3<f32>(0.0));
    let s = clamp((log2(max(d, vec3<f32>(1e-10))) + 10.0) / 16.5, vec3<f32>(0.0), vec3<f32>(1.0));
    let a = pow(s, vec3<f32>(2.2));
    let b = pow(1.0 - s, vec3<f32>(2.2));
    return a / max(a + FILMIC_C * b, vec3<f32>(1e-10));
}

// The S curve's constant: mid grey (0.18, at (log2(0.18) + 10) / 16.5 =
// 0.4561) to 0.46 on the display, its contrast 2.2.
const FILMIC_C: f32 = 0.797;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
