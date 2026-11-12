//! Primitives and landforms: fields made from nothing.

use super::ops::smoothstep;
use super::{CellStyle, Extent, Field, MountainStyle, NodeKind, NoiseStyle, ShapeKind};
use crate::compose::hash2;
use crate::noise::Noise;
use rayon::prelude::*;

/// A field from a function of world position.
fn generate(e: &Extent, op: &(dyn Fn(f64, f64) -> f32 + Sync)) -> Field {
    let n = e.n;
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (x, z) = e.world(c % n, c / n);
                op(x, z)
            })
            .collect(),
    )
}

/// A random number in 0..1 from a seed and two integers.
#[inline]
fn rand(seed: u32, a: i32, b: i32) -> f64 {
    hash2(a, b, seed as u64 ^ 0x1a4d_f0e5) as f64 / u32::MAX as f64
}

/// The field of a primitive or landform node; `None` for other nodes.
pub(super) fn generate_kind(kind: &NodeKind, e: &Extent) -> Option<Field> {
    let size = e.size;
    let field = match *kind {
        NodeKind::Noise {
            style,
            scale,
            octaves,
            roughness,
            height,
            base,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x006e_6f69_7365);
            let s = scale.max(1.0) as f64;
            let oct = octaves.clamp(1, 12);
            let gain = roughness.clamp(0.05, 0.95) as f64;
            generate(e, &|x, z| {
                let v = match style {
                    NoiseStyle::Fbm => noise.fbm(x / s, z / s, oct, 2.0, gain) * 0.5 + 0.5,
                    NoiseStyle::Ridged => noise.ridged(x / s, z / s, oct, 2.0, gain),
                    NoiseStyle::Billow => noise.fbm(x / s, z / s, oct, 2.0, gain).abs(),
                };
                base + height * v.clamp(0.0, 1.0) as f32
            })
        }
        NodeKind::Cells {
            style,
            scale,
            jitter,
            height,
            base,
            seed,
        } => {
            let s = scale.max(1.0) as f64;
            let j = jitter.clamp(0.0, 1.0) as f64;
            generate(e, &|x, z| {
                let (fx, fz) = (x / s, z / s);
                let (cx, cz) = (fx.floor() as i32, fz.floor() as i32);
                let (mut d1, mut d2) = (f64::MAX, f64::MAX);
                let mut nearest = (0, 0);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let (gx, gz) = (cx + dx, cz + dz);
                        let h = hash2(gx, gz, seed as u64 ^ 0xce11);
                        let ox = 0.5 + j * ((h & 0xffff) as f64 / 65535.0 - 0.5);
                        let oz = 0.5 + j * ((h >> 16) as f64 / 65535.0 - 0.5);
                        let d = (gx as f64 + ox - fx).hypot(gz as f64 + oz - fz);
                        if d < d1 {
                            d2 = d1;
                            d1 = d;
                            nearest = (gx, gz);
                        } else if d < d2 {
                            d2 = d;
                        }
                    }
                }
                let v = match style {
                    CellStyle::Cones => (1.0 - d1 / 0.75).max(0.0),
                    CellStyle::Ridges => (1.0 - (d2 - d1) * 2.0).clamp(0.0, 1.0),
                    CellStyle::Plates => hash2(nearest.0, nearest.1, seed as u64 ^ 0x0091_a7e5) as f64 / u32::MAX as f64,
                };
                base + height * v as f32
            })
        }
        NodeKind::Radial {
            inner,
            outer,
            height,
            base,
        } => {
            let (cx, cz) = (e.x0 + size / 2.0, e.z0 + size / 2.0);
            let half = size / 2.0;
            let (a, b) = (inner as f64, outer.max(inner + 0.001) as f64);
            generate(e, &|x, z| {
                let r = (x - cx).hypot(z - cz) / half;
                let t = ((r - a) / (b - a)).clamp(0.0, 1.0);
                let v = 1.0 - t * t * (3.0 - 2.0 * t);
                base + height * v as f32
            })
        }
        NodeKind::Constant { value } => Field::new(e.n, vec![value; e.n * e.n]),
        NodeKind::LinearGradient { angle, height, base } => {
            let (s, c) = (angle as f64).to_radians().sin_cos();
            let (cx, cz) = (e.x0 + size / 2.0, e.z0 + size / 2.0);
            generate(e, &|x, z| {
                // 0 north (−z), 90 east (+x).
                let t = ((x - cx) * s - (z - cz) * c) / size + 0.5;
                base + height * t.clamp(0.0, 1.0) as f32
            })
        }
        NodeKind::Shape {
            shape,
            x,
            z,
            radius,
            falloff,
            height,
            base,
        } => {
            let (cx, cz) = e.at_fraction(x, z);
            let r = radius.max(1.0) as f64;
            let soft = falloff.clamp(0.0, 1.0) as f64;
            generate(e, &|px, pz| {
                let (dx, dz) = ((px - cx) / r, (pz - cz) / r);
                let d = match shape {
                    ShapeKind::Square => dx.abs().max(dz.abs()),
                    _ => dx.hypot(dz),
                };
                let v = match shape {
                    ShapeKind::Circle | ShapeKind::Square => {
                        if soft <= 0.0 {
                            (d <= 1.0) as u8 as f64
                        } else {
                            1.0 - smoothstep(1.0 - soft as f32, 1.0, d as f32) as f64
                        }
                    }
                    ShapeKind::Cone => (1.0 - d).max(0.0),
                    ShapeKind::Dome => (1.0 - d * d).max(0.0).sqrt(),
                };
                base + height * v as f32
            })
        }
        NodeKind::Mountain {
            style,
            x,
            z,
            radius,
            height,
            base,
            detail,
            seed,
        } => mountain(e, style, x, z, radius, height, base, detail, seed),
        NodeKind::MountainRange {
            angle,
            width,
            scale,
            height,
            base,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x5a17);
            let vary = noise.derive(3);
            let (s, c) = (angle as f64).to_radians().sin_cos();
            let (cx, cz) = (e.x0 + size / 2.0, e.z0 + size / 2.0);
            let half = (width.max(10.0) / 2.0) as f64;
            let sc = scale.max(10.0) as f64;
            generate(e, &|px, pz| {
                let (dx, dz) = (px - cx, pz - cz);
                // Along the range, and across it.
                let along = dx * s - dz * c;
                let across = dx * c + dz * s;
                // The range wanders and swells along its length.
                let wander = vary.fbm(along / (sc * 4.0), 0.5, 3, 2.0, 0.5) * half * 0.6;
                let swell = 0.65 + 0.35 * vary.fbm(along / (sc * 3.0), 7.3, 3, 2.0, 0.5);
                let t = (across - wander) / (half * swell.max(0.2));
                let band = (-t * t * 2.2).exp();
                // Ridges stretched along the range.
                let ridges = noise.ridged(along / (sc * 1.6), across / sc, 7, 2.1, 0.5);
                let v = band * (0.25 + 0.75 * ridges.powf(1.2)) * swell.max(0.0);
                base + height * v.clamp(0.0, 1.2) as f32
            })
        }
        NodeKind::Volcano {
            x,
            z,
            radius,
            height,
            base,
            crater,
            depth,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x701c);
            let (cx, cz) = e.at_fraction(x, z);
            let r0 = radius.max(10.0) as f64;
            let rc = (crater.clamp(0.02, 0.6)) as f64;
            let h = height as f64;
            generate(e, &|px, pz| {
                let (dx, dz) = (px - cx, pz - cz);
                let angle = dz.atan2(dx);
                // A slightly ragged outline.
                let ragged = 1.0 + 0.12 * noise.fbm(angle.cos() * 2.0 + 11.0, angle.sin() * 2.0, 3, 2.0, 0.5);
                let r = dx.hypot(dz) / (r0 * ragged);
                if r >= 1.0 {
                    return base;
                }
                // Concave flanks rising to the rim.
                let cone = |r: f64| (1.0 - r).max(0.0).powf(1.6);
                let rim = cone(rc);
                let mut v = if r > rc {
                    cone(r) / rim
                } else {
                    // The crater: a bowl below the rim.
                    let k = r / rc;
                    1.0 - (depth as f64 / h.max(1.0)) * (1.0 - k * k)
                };
                // Gullies down the flanks, deeper near the base.
                let gully = noise.ridged(angle * 9.0, r * 3.0, 4, 2.0, 0.5);
                v *= 1.0 - 0.18 * (1.0 - gully) * r.min(1.0) * (r > rc) as u8 as f64;
                // Lava flows and bumps.
                v += 0.04 * noise.fbm(px / (r0 * 0.15), pz / (r0 * 0.15), 4, 2.0, 0.5) * (1.0 - r);
                base + (h * v.max(0.0)) as f32
            })
        }
        NodeKind::Crater {
            x,
            z,
            radius,
            depth,
            rim,
            peak,
            base,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0xc8a7);
            let (cx, cz) = e.at_fraction(x, z);
            let r0 = radius.max(4.0) as f64;
            generate(e, &|px, pz| {
                let (dx, dz) = (px - cx, pz - cz);
                let ragged = 1.0 + 0.06 * noise.fbm(px / (r0 * 0.5), pz / (r0 * 0.5), 3, 2.0, 0.5);
                let r = dx.hypot(dz) / (r0 * ragged);
                base + crater_profile(r, depth as f64, rim as f64, peak as f64) as f32
            })
        }
        NodeKind::CraterField {
            count,
            min_radius,
            max_radius,
            depth,
            base,
            seed,
        } => crater_field(e, count.min(20_000), min_radius.max(2.0), max_radius.max(min_radius.max(2.0)), depth, base, seed),
        NodeKind::Dunes {
            angle,
            wavelength,
            height,
            base,
            sharpness,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0xd0e5);
            let (s, c) = (angle as f64).to_radians().sin_cos();
            let w = wavelength.max(4.0) as f64;
            let sharp = (0.12 + 0.5 * (1.0 - sharpness.clamp(0.0, 1.0))) as f64;
            generate(e, &|px, pz| {
                // Along the wind, and across it.
                let u = px * s - pz * c;
                let v = px * c + pz * s;
                let bend = noise.fbm(v / (w * 4.0), u / (w * 6.0), 3, 2.0, 0.5) * w * 0.9;
                let phase = (u + bend) / w + 0.25 * noise.fbm(px / (w * 2.0), pz / (w * 2.0), 2, 2.0, 0.5);
                let f = phase - phase.floor();
                // A long windward slope and a steep slip face.
                let y = if f < 1.0 - sharp { f / (1.0 - sharp) } else { (1.0 - f) / sharp };
                let y = y * y * (3.0 - 2.0 * y);
                let crest = 0.55 + 0.45 * noise.fbm(v / (w * 3.0), u / (w * 9.0) + 5.0, 3, 2.0, 0.5);
                base + height * (y * crest.max(0.1)) as f32
            })
        }
        NodeKind::Canyon {
            angle,
            width,
            depth,
            meander,
            steps,
            base,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0xca71);
            let detail = noise.derive(5);
            let (s, c) = (angle as f64).to_radians().sin_cos();
            let (cx, cz) = (e.x0 + size / 2.0, e.z0 + size / 2.0);
            let half = (width.max(8.0) / 2.0) as f64;
            let k = steps as f64;
            generate(e, &|px, pz| {
                let (dx, dz) = (px - cx, pz - cz);
                let along = dx * s - dz * c;
                let across = dx * c + dz * s;
                let wind = noise.fbm(along / (meander.max(1.0) as f64 * 3.0), 0.3, 4, 2.0, 0.5) * meander as f64 * 1.6;
                let widen = 1.0 + 0.3 * noise.fbm(along / (half * 6.0), 4.1, 3, 2.0, 0.5);
                let d = ((across - wind).abs() / (half * widen)).min(1.6);
                // A flat floor, then walls up to the plateau.
                let mut t = smoothstep(0.22, 1.0, d as f32) as f64;
                if k > 0.0 {
                    let q = t * k;
                    let f = q - q.floor();
                    let riser = smoothstep(0.55, 1.0, f as f32) as f64;
                    t = ((q.floor() + riser) / k).min(1.0);
                }
                let plateau = 4.0 * detail.fbm(px / 300.0, pz / 300.0, 4, 2.0, 0.5);
                base + (plateau - depth as f64 * (1.0 - t)) as f32
            })
        }
        NodeKind::Mesas {
            scale,
            coverage,
            height,
            base,
            steps,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x3e5a);
            let rough = noise.derive(2);
            let s = scale.max(10.0) as f64;
            let threshold = 1.0 - coverage.clamp(0.02, 0.98) as f64;
            let k = steps.max(1) as f64;
            generate(e, &|px, pz| {
                let v = noise.fbm(px / s, pz / s, 5, 2.0, 0.5) * 0.5 + 0.5;
                let t = smoothstep((threshold - 0.04) as f32, (threshold + 0.04) as f32, v as f32) as f64;
                // Stepped sides.
                let q = (v - threshold + 0.04) / 0.25 * k;
                let stepped = ((q.floor() + smoothstep(0.7, 1.0, (q - q.floor()) as f32) as f64) / k).clamp(0.0, 1.0);
                let top = t * (0.75 + 0.25 * stepped);
                let detail = 0.03 * rough.fbm(px / (s * 0.1), pz / (s * 0.1), 3, 2.0, 0.5);
                base + height * (top + detail).max(0.0) as f32
            })
        }
        NodeKind::Island {
            size: frac,
            height,
            base,
            coast,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x15a0);
            let relief = noise.derive(1);
            let (cx, cz) = (e.x0 + size / 2.0, e.z0 + size / 2.0);
            let r0 = (frac.clamp(0.05, 1.0) as f64) * size / 2.0;
            let c = coast.clamp(0.0, 1.0) as f64;
            generate(e, &|px, pz| {
                let r = (px - cx).hypot(pz - cz) / r0;
                let r = r * (1.0 + c * 0.45 * noise.fbm(px / (r0 * 0.4), pz / (r0 * 0.4), 5, 2.0, 0.5));
                let land = smoothstep(1.0, 0.25, r as f32) as f64;
                let ridges = relief.ridged(px / (r0 * 0.6), pz / (r0 * 0.6), 6, 2.0, 0.5);
                let v = land.powf(1.4) * (0.55 + 0.45 * ridges);
                base + height * v as f32
            })
        }
        NodeKind::Rocks {
            scale,
            height,
            sharpness,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x70c5);
            let s = scale.max(2.0) as f64;
            let sharp = sharpness.clamp(0.0, 1.0) as f64;
            generate(e, &|px, pz| {
                let crag = noise.ridged(px / s, pz / s, 5, 2.2, 0.5);
                let boulder = noise.fbm(px / s, pz / s, 5, 2.0, 0.5).abs();
                let v = crag * sharp + boulder * (1.0 - sharp);
                height * (v - 0.5) as f32
            })
        }
        _ => return None,
    };
    Some(field)
}

#[allow(clippy::too_many_arguments)]
fn mountain(e: &Extent, style: MountainStyle, x: f32, z: f32, radius: f32, height: f32, base: f32, detail: f32, seed: u32) -> Field {
    let noise = Noise::new(seed as u64 ^ 0x6d07);
    let warp = noise.derive(1);
    let fine = noise.derive(2);
    let (cx, cz) = e.at_fraction(x, z);
    let r0 = radius.max(10.0) as f64;
    let d = detail.clamp(0.0, 1.0) as f64;
    generate(e, &|px, pz| {
        // An irregular outline: the distance is warped.
        let wx = warp.fbm(px / (r0 * 0.7), pz / (r0 * 0.7), 3, 2.0, 0.5) * r0 * 0.28;
        let wz = warp.fbm(px / (r0 * 0.7) + 9.1, pz / (r0 * 0.7), 3, 2.0, 0.5) * r0 * 0.28;
        let r = (px + wx - cx).hypot(pz + wz - cz) / r0;
        if r >= 1.15 {
            return base;
        }
        let bell = {
            let t = (1.0 - (r / 1.15).powi(2)).max(0.0);
            t * t
        };
        let s = r0 * 0.55;
        let v = match style {
            MountainStyle::Alpine => {
                let ridges = noise.ridged((px + wx) / s, (pz + wz) / s, 8, 2.1, 0.5);
                bell * (0.32 + 0.68 * ridges.powf(1.15))
            }
            MountainStyle::Rounded => {
                let soft = noise.fbm(px / s, pz / s, 5, 2.0, 0.45) * 0.5 + 0.5;
                bell.powf(0.8) * (0.7 + 0.3 * soft)
            }
            MountainStyle::Eroded => {
                // Valleys branching from the summit: ridged noise warped
                // around the centre.
                let a = (pz - cz).atan2(px - cx);
                let radial = noise.ridged(a * 3.0 + wx / r0, r * 4.0, 6, 2.0, 0.55);
                let ridges = noise.ridged((px + wx * 2.0) / s, (pz + wz * 2.0) / s, 7, 2.0, 0.5);
                bell.powf(1.15) * (0.35 + 0.35 * ridges + 0.3 * radial)
            }
            MountainStyle::Spiky => {
                let ridges = noise.ridged((px + wx) / (s * 0.7), (pz + wz) / (s * 0.7), 8, 2.2, 0.5);
                let sharp = (1.0 - r / 1.15).max(0.0).powf(2.2);
                (sharp * (0.4 + 0.6 * ridges.powf(1.6)) * 1.25).min(1.0)
            }
        };
        let grain = fine.fbm(px / (r0 * 0.06), pz / (r0 * 0.06), 4, 2.0, 0.5) * 0.06 * d * bell;
        base + height * (v + grain).max(0.0) as f32
    })
}

/// The height of a crater's surface relative to the plain, at `r` radii from
/// its centre: a bowl, a raised rim, ejecta fading outside, a central peak.
fn crater_profile(r: f64, depth: f64, rim: f64, peak: f64) -> f64 {
    let mut v = 0.0;
    if r < 1.0 {
        // A bowl rising steeply to the rim.
        v += -depth * (1.0 - r.powf(2.4)) + rim * r.powi(6);
    } else {
        // The rim, falling off outside through the ejecta.
        let t = (r - 1.0) / 0.9;
        v += rim * (-t * t * 3.0).exp();
    }
    if peak > 0.0 {
        v += peak * (-(r / 0.16).powi(2)).exp();
    }
    v
}

fn crater_field(e: &Extent, count: u32, min_r: f32, max_r: f32, depth: f32, base: f32, seed: u32) -> Field {
    // Sizes follow a power law: many small craters, few big ones. Big ones
    // first, so that small ones land on top of them.
    let mut craters: Vec<(f64, f64, f64)> = (0..count as i32)
        .map(|k| {
            let u = rand(seed, k, 1);
            let ratio = (max_r / min_r) as f64;
            let r = min_r as f64 * ratio.powf(u.powf(2.6));
            let x = e.x0 + rand(seed, k, 2) * e.size;
            let z = e.z0 + rand(seed, k, 3) * e.size;
            (x, z, r)
        })
        .collect();
    craters.sort_by(|a, b| b.2.total_cmp(&a.2));
    // Buckets of craters, to look only at those near each sample.
    let cell = (max_r as f64 * 2.5).max(1.0);
    let mut buckets: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (k, &(x, z, r)) in craters.iter().enumerate() {
        let reach = r * 2.0;
        let (i0, i1) = (((x - reach) / cell).floor() as i32, ((x + reach) / cell).floor() as i32);
        let (j0, j1) = (((z - reach) / cell).floor() as i32, ((z + reach) / cell).floor() as i32);
        for j in j0..=j1 {
            for i in i0..=i1 {
                buckets.entry((i, j)).or_default().push(k);
            }
        }
    }
    let noise = Noise::new(seed as u64 ^ 0xf1e1d);
    let d = depth.clamp(0.02, 1.0) as f64;
    generate(e, &|px, pz| {
        let mut h = 2.5 * noise.fbm(px / 300.0, pz / 300.0, 5, 2.0, 0.5);
        if let Some(list) = buckets.get(&((px / cell).floor() as i32, (pz / cell).floor() as i32)) {
            for &k in list {
                let (x, z, r) = craters[k];
                let q = (px - x).hypot(pz - z) / r;
                if q < 2.0 {
                    // Craters land on what is there: a small one in a big
                    // one's bowl is dug from its floor.
                    let dep = r * d;
                    h += crater_profile(q, dep, dep * 0.3, if r > max_r as f64 * 0.6 { dep * 0.35 } else { 0.0 });
                }
            }
        }
        base + h as f32
    })
}
