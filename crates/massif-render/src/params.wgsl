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
