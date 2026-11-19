// The sky's tables, filled when the sun or the air change (atmosphere.wgsl
// comes before this): the transmittance of the air from any height in any
// direction, the light scattered many times, and the sky seen from the
// observer, for a sun of illuminance 1.

struct SkyParams {
    // Toward the sun; w: unused.
    sun: vec4<f32>,
    // The air's scattering, the aerosols', their anisotropy, and the
    // observer's height, km.
    air: vec4<f32>,
}

@group(0) @binding(0) var<uniform> S: SkyParams;
@group(0) @binding(1) var transmittance_in: texture_2d<f32>;
@group(0) @binding(2) var multi_in: texture_2d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var out: texture_storage_2d<rgba16float, write>;

fn air() -> Air {
    return Air(S.air.x, S.air.y, S.air.z, S.air.w);
}

fn sun_transmittance(r: f32, mu: f32) -> vec3<f32> {
    if (hits_ground(r, mu)) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(transmittance_in, lut_sampler, transmittance_uv(r, mu), 0.0).rgb;
}

fn multi_scattering(r: f32, mu: f32) -> vec3<f32> {
    let uv = vec2<f32>(mu * 0.5 + 0.5, clamp((r - BOTTOM) / (TOP - BOTTOM), 0.0, 1.0));
    let st = (uv * (MULTI_SIZE - 1.0) + 0.5) / MULTI_SIZE;
    return textureSampleLevel(multi_in, lut_sampler, st, 0.0).rgb;
}

@compute @workgroup_size(8, 8)
fn transmittance(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= TRANSMITTANCE_SIZE.x || f32(id.y) >= TRANSMITTANCE_SIZE.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / TRANSMITTANCE_SIZE;
    let rm = transmittance_r_mu(uv);
    let a = air();
    let d = max(to_sphere(rm.x, rm.y, TOP), 0.0);
    let steps = 40;
    let dt = d / f32(steps);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let t = (f32(i) + 0.5) * dt;
        let r = sqrt(rm.x * rm.x + t * t + 2.0 * rm.x * rm.y * t);
        depth += medium_at(r - BOTTOM, a).extinction * dt;
    }
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(exp(-depth), 1.0));
}

// Light scattered twice and more, as an isotropic source (Hillaire's
// approximation): for each height and sun, the light of 64 directions.
@compute @workgroup_size(8, 8)
fn multi(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= MULTI_SIZE || f32(id.y) >= MULTI_SIZE) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / MULTI_SIZE;
    let mu_s = uv.x * 2.0 - 1.0;
    let r = BOTTOM + max(uv.y * (TOP - BOTTOM), 0.01);
    let sun = vec3<f32>(sqrt(max(1.0 - mu_s * mu_s, 0.0)), mu_s, 0.0);
    let a = air();
    var second = vec3<f32>(0.0);
    var transfer = vec3<f32>(0.0);
    for (var i = 0; i < 8; i++) {
        for (var j = 0; j < 8; j++) {
            let ct = 1.0 - 2.0 * (f32(i) + 0.5) / 8.0;
            let st = sqrt(max(1.0 - ct * ct, 0.0));
            let phi = 2.0 * PI * (f32(j) + 0.5) / 8.0;
            let dir = vec3<f32>(st * cos(phi), ct, st * sin(phi));
            let ground = hits_ground(r, ct);
            var t_max = to_sphere(r, ct, TOP);
            if (ground) {
                t_max = to_sphere(r, ct, BOTTOM);
            }
            t_max = max(t_max, 0.0);
            let steps = 20;
            let dt = t_max / f32(steps);
            var throughput = vec3<f32>(1.0);
            var lum = vec3<f32>(0.0);
            var fms = vec3<f32>(0.0);
            for (var k = 0; k < steps; k++) {
                let p = vec3<f32>(0.0, r, 0.0) + dir * ((f32(k) + 0.5) * dt);
                let rk = length(p);
                let m = medium_at(rk - BOTTOM, a);
                let step_t = exp(-m.extinction * dt);
                let ext = max(m.extinction, vec3<f32>(1e-7));
                let ts = sun_transmittance(rk, dot(p, sun) / rk);
                let s = m.scattering * ts / (4.0 * PI);
                lum += throughput * (s - s * step_t) / ext;
                fms += throughput * (m.scattering - m.scattering * step_t) / ext;
                throughput *= step_t;
            }
            if (ground) {
                let p = vec3<f32>(0.0, r, 0.0) + dir * t_max;
                let n = normalize(p);
                let ts = sun_transmittance(length(p), dot(n, sun));
                lum += throughput * ts * max(dot(n, sun), 0.0) * GROUND_ALBEDO / PI;
            }
            second += lum / 64.0;
            transfer += fms / 64.0;
        }
    }
    let psi = second / max(vec3<f32>(1.0) - transfer, vec3<f32>(1e-3));
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(psi, 1.0));
}

// The sky seen from the observer: for each direction (azimuth from the
// sun, angle from the zenith), the light scattered toward it.
@compute @workgroup_size(8, 8)
fn sky_view(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= SKY_VIEW_SIZE.x || f32(id.y) >= SKY_VIEW_SIZE.y) {
        return;
    }
    let a = air();
    let r = BOTTOM + max(a.altitude, 0.001);
    // Undo sky_view_uv's mapping.
    let uv = vec2<f32>(id.xy) / (SKY_VIEW_SIZE - 1.0);
    let v_horizon = sqrt(max(r * r - BOTTOM * BOTTOM, 0.0));
    let beta = acos(clamp(v_horizon / r, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    var view_zenith: f32;
    if (uv.y < 0.5) {
        let c = 1.0 - uv.y * 2.0;
        view_zenith = zenith_horizon * (1.0 - c * c);
    } else {
        let c = uv.y * 2.0 - 1.0;
        view_zenith = zenith_horizon + beta * c * c;
    }
    let azimuth = uv.x * PI;
    let mu_s = clamp(S.sun.y, -1.0, 1.0);
    let sun = vec3<f32>(sqrt(max(1.0 - mu_s * mu_s, 0.0)), mu_s, 0.0);
    let dir = vec3<f32>(sin(view_zenith) * cos(azimuth), cos(view_zenith), sin(view_zenith) * sin(azimuth));
    let ground = hits_ground(r, dir.y);
    var t_max = to_sphere(r, dir.y, TOP);
    if (ground) {
        t_max = to_sphere(r, dir.y, BOTTOM);
    }
    t_max = max(t_max, 0.0);
    let cos_theta = dot(dir, sun);
    let pr = rayleigh_phase(cos_theta);
    let pm = mie_phase(cos_theta, a.mie_g);
    let steps = 32;
    var throughput = vec3<f32>(1.0);
    var lum = vec3<f32>(0.0);
    // Shorter steps near the observer, where the light changes most.
    var t_prev = 0.0;
    for (var k = 0; k < steps; k++) {
        let f = (f32(k) + 1.0) / f32(steps);
        let t = t_max * f * f;
        let dt = t - t_prev;
        let p = vec3<f32>(0.0, r, 0.0) + dir * (t_prev + dt * 0.5);
        t_prev = t;
        let rk = length(p);
        let m = medium_at(rk - BOTTOM, a);
        let step_t = exp(-m.extinction * dt);
        let ext = max(m.extinction, vec3<f32>(1e-7));
        let mu_k = dot(p, sun) / rk;
        let ts = sun_transmittance(rk, mu_k);
        let s = (m.rayleigh * pr + vec3<f32>(m.mie * pm)) * ts + multi_scattering(rk, mu_k) * m.scattering;
        lum += throughput * (s - s * step_t) / ext;
        throughput *= step_t;
    }
    if (ground) {
        // The planet's ground beyond the map, lit by the sun through the air.
        let p = vec3<f32>(0.0, r, 0.0) + dir * t_max;
        let n = normalize(p);
        let ts = sun_transmittance(length(p), dot(n, sun));
        lum += throughput * ts * max(dot(n, sun), 0.0) * GROUND_ALBEDO / PI;
    }
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(lum, 1.0));
}
