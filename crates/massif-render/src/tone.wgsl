// The tone mapping's curves, shared by the path tracer, the denoiser and
// the 3D View's Shaders (massif-app's view3d.rs).

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
