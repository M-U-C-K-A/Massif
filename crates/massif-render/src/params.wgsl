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
    flags: vec4<u32>,    // the image left to the denoiser (not written here), blades of grass a block, their height (f32 bits), the view transform (see tone)
    tone: vec4<f32>,     // the display's gamma, the look's contrast, its saturation, greyscale (1)
    level_off: array<vec4<u32>, 4>,
    level_w: array<vec4<u32>, 4>,
}

// Light to the display's values, as Blender's Color Management does it:
// the view transform P.flags.w (0 Standard, 1 Khronos PBR Neutral, 2 AgX,
// 3 Filmic, 4 Filmic Log, 5 False Color, 6 Raw), its look as a contrast
// (the S curve's steepness), a saturation and greyscale (P.tone.yzw), then
// the display's gamma (P.tone.x). The same as `tone_map` in gpu.rs.
fn tone(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let x = max(c * exposure, vec3<f32>(0.0));
    var out: vec3<f32>;
    switch (P.flags.w) {
        case 0u: { out = srgb_encode(min(x, vec3<f32>(1.0))); }
        case 1u: { out = srgb_encode(pbr_neutral(x)); }
        case 3u: { out = s_curve(x, -10.0, P.tone.y, 1.08 * P.tone.z); }
        case 4u: { out = clamp((log2(max(x, vec3<f32>(1e-10))) + 10.0) / 16.5, vec3<f32>(0.0), vec3<f32>(1.0)); }
        case 5u: { out = false_color(luminance(x)); }
        case 6u: { out = min(x, vec3<f32>(1.0)); }
        default: { out = agx(x, P.tone.y, P.tone.z); }
    }
    if (P.tone.w > 0.5) {
        out = vec3<f32>(dot(out, vec3<f32>(0.2126, 0.7152, 0.0722)));
    }
    return pow(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / max(P.tone.x, 0.1)));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

// Each channel's stops, from `low` stops under 1 over 16.5 of them, along
// an S curve of steepness `contrast` that puts mid grey (0.18) at 0.46 on
// the display; colours first saturated by `saturation` (the curve greys
// them).
fn s_curve(x: vec3<f32>, low: f32, contrast: f32, saturation: f32) -> vec3<f32> {
    let l = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    let d = max(mix(vec3<f32>(l), x, saturation), vec3<f32>(0.0));
    let s = clamp((log2(max(d, vec3<f32>(1e-10))) - low) / 16.5, vec3<f32>(0.0), vec3<f32>(1.0));
    let x0 = (log2(0.18) - low) / 16.5;
    let k = pow(x0, contrast) * 0.54 / (0.46 * pow(1.0 - x0, contrast));
    let a = pow(s, vec3<f32>(contrast));
    let b = pow(1.0 - s, vec3<f32>(contrast));
    return a / max(a + k * b, vec3<f32>(1e-10));
}

// AgX as its public description goes: the colours drawn in toward white
// (inset), each channel's stops over 16.5 from 12.47 under 1 along an S
// curve, then back out (outset) and the saturation of the look; bright
// colours run to white rather than to a burnt hue.
fn agx(x: vec3<f32>, contrast: f32, saturation: f32) -> vec3<f32> {
    let y = s_curve(x, -12.47393, contrast, 0.82);
    let lin = pow(y, vec3<f32>(2.2));
    let l = dot(lin, vec3<f32>(0.2126, 0.7152, 0.0722));
    let out = max(mix(vec3<f32>(l), lin, 1.18 * saturation), vec3<f32>(0.0));
    return pow(min(out, vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
}

// Khronos PBR Neutral, by its published specification: colours kept as
// they are up to 0.76, the brighter compressed toward white.
fn pbr_neutral(c_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = min(c_in.r, min(c_in.g, c_in.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var c = c_in - offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c *= new_peak / peak;
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// The stops of light against mid grey as colours, as Blender's False Color
// shows exposure: black, purple, blue, cyan, green, grey (mid grey), the
// light greys, yellow, orange, red, white.
fn false_color(l: f32) -> vec3<f32> {
    let ev = log2(max(l, 1e-10) / 0.18);
    if (ev < -6.5) { return vec3<f32>(0.0); }
    if (ev < -4.5) { return vec3<f32>(0.4, 0.0, 0.6); }
    if (ev < -2.5) { return vec3<f32>(0.1, 0.2, 0.9); }
    if (ev < -1.5) { return vec3<f32>(0.0, 0.7, 0.8); }
    if (ev < -0.5) { return vec3<f32>(0.2, 0.75, 0.25); }
    if (ev < 0.5) { return vec3<f32>(0.5); }
    if (ev < 1.5) { return vec3<f32>(0.75); }
    if (ev < 2.5) { return vec3<f32>(0.95, 0.9, 0.2); }
    if (ev < 4.5) { return vec3<f32>(1.0, 0.55, 0.0); }
    if (ev < 6.5) { return vec3<f32>(0.95, 0.1, 0.1); }
    return vec3<f32>(1.0);
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
