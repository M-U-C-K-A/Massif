// The Earth's atmosphere, after Hillaire, "A Scalable and Production Ready
// Sky and Atmosphere Rendering Technique" (2020): Rayleigh scattering by
// the air, Mie scattering by aerosols, absorption by ozone, over a planet
// of 6,360 km. Distances in kilometres. Shared by the passes that fill the
// sky's tables (sky.wgsl) and by the path tracer (trace.wgsl), which reads
// those tables.

const PI: f32 = 3.14159265;
const BOTTOM: f32 = 6360.0;
const TOP: f32 = 6460.0;
const RAYLEIGH_SCATTERING: vec3<f32> = vec3<f32>(5.802e-3, 13.558e-3, 33.1e-3);
const RAYLEIGH_HEIGHT: f32 = 8.0;
const MIE_SCATTERING: f32 = 3.996e-3;
const MIE_EXTINCTION: f32 = 4.440e-3;
const MIE_HEIGHT: f32 = 1.2;
const OZONE_ABSORPTION: vec3<f32> = vec3<f32>(0.650e-3, 1.881e-3, 0.085e-3);
const GROUND_ALBEDO: vec3<f32> = vec3<f32>(0.3);

const TRANSMITTANCE_SIZE: vec2<f32> = vec2<f32>(256.0, 64.0);
const MULTI_SIZE: f32 = 32.0;
const SKY_VIEW_SIZE: vec2<f32> = vec2<f32>(192.0, 108.0);

// What the user changes of the air: the air's own scattering (1: the
// Earth's), the aerosols' (haze), and their phase's anisotropy.
struct Air {
    rayleigh: f32,
    mie: f32,
    mie_g: f32,
    // The observer's height over the ground, km.
    altitude: f32,
}

struct Medium {
    scattering: vec3<f32>,
    extinction: vec3<f32>,
    rayleigh: vec3<f32>,
    mie: f32,
}

fn medium_at(h: f32, air: Air) -> Medium {
    let rd = exp(-max(h, 0.0) / RAYLEIGH_HEIGHT) * air.rayleigh;
    let md = exp(-max(h, 0.0) / MIE_HEIGHT) * air.mie;
    let od = max(0.0, 1.0 - abs(h - 25.0) / 15.0);
    var m: Medium;
    m.rayleigh = RAYLEIGH_SCATTERING * rd;
    m.mie = MIE_SCATTERING * md;
    m.scattering = m.rayleigh + vec3<f32>(m.mie);
    m.extinction = m.rayleigh + vec3<f32>(MIE_EXTINCTION * md) + OZONE_ABSORPTION * od;
    return m;
}

fn rayleigh_phase(c: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + c * c);
}

// Cornette-Shanks, as Hillaire's Mie phase.
fn mie_phase(c: f32, g: f32) -> f32 {
    let k = 3.0 / (8.0 * PI) * (1.0 - g * g) / (2.0 + g * g);
    return k * (1.0 + c * c) / pow(max(1.0 + g * g - 2.0 * g * c, 1e-4), 1.5);
}

fn hg_phase(c: f32, g: f32) -> f32 {
    return (1.0 - g * g) / (4.0 * PI * pow(max(1.0 + g * g - 2.0 * g * c, 1e-4), 1.5));
}

// Distance from a point at radius r along a direction of cosine mu (to
// the zenith) to the sphere of radius R, or -1 when it misses it.
fn to_sphere(r: f32, mu: f32, radius: f32) -> f32 {
    let disc = r * r * (mu * mu - 1.0) + radius * radius;
    if (disc < 0.0) {
        return -1.0;
    }
    let s = sqrt(disc);
    let near = -r * mu - s;
    if (near > 0.0) {
        return near;
    }
    let far = -r * mu + s;
    if (far > 0.0) {
        return far;
    }
    return -1.0;
}

fn hits_ground(r: f32, mu: f32) -> bool {
    return mu < 0.0 && r * r * (mu * mu - 1.0) + BOTTOM * BOTTOM >= 0.0;
}

// Bruneton's mapping of (r, mu) to the transmittance table and back.
fn transmittance_uv(r: f32, mu: f32) -> vec2<f32> {
    let h = sqrt(TOP * TOP - BOTTOM * BOTTOM);
    let rho = sqrt(max(r * r - BOTTOM * BOTTOM, 0.0));
    let disc = r * r * (mu * mu - 1.0) + TOP * TOP;
    let d = max(0.0, -r * mu + sqrt(max(disc, 0.0)));
    let d_min = TOP - r;
    let d_max = rho + h;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / h);
}

fn transmittance_r_mu(uv: vec2<f32>) -> vec2<f32> {
    let h = sqrt(TOP * TOP - BOTTOM * BOTTOM);
    let rho = h * uv.y;
    let r = sqrt(rho * rho + BOTTOM * BOTTOM);
    let d_min = TOP - r;
    let d_max = rho + h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) {
        mu = clamp((h * h - rho * rho - d * d) / (2.0 * r * d), -1.0, 1.0);
    }
    return vec2<f32>(r, mu);
}

// The sky-view table: azimuth from the sun (0..pi, the sky being the same
// on both sides), and the angle from the zenith, packed closer near the
// horizon, where the sky changes most.
fn sky_view_uv(r: f32, view_zenith_cos: f32, azimuth_cos: f32) -> vec2<f32> {
    let v_horizon = sqrt(max(r * r - BOTTOM * BOTTOM, 0.0));
    let beta = acos(clamp(v_horizon / r, -1.0, 1.0));
    let zenith_horizon = PI - beta;
    let view_zenith = acos(clamp(view_zenith_cos, -1.0, 1.0));
    var v: f32;
    if (view_zenith < zenith_horizon) {
        var c = view_zenith / zenith_horizon;
        c = 1.0 - sqrt(max(1.0 - c, 0.0));
        v = c * 0.5;
    } else {
        let c = sqrt(max((view_zenith - zenith_horizon) / beta, 0.0));
        v = c * 0.5 + 0.5;
    }
    let u = acos(clamp(azimuth_cos, -1.0, 1.0)) / PI;
    // Half a texel in from each edge.
    let size = SKY_VIEW_SIZE;
    return (vec2<f32>(u, v) * (size - 1.0) + 0.5) / size;
}
