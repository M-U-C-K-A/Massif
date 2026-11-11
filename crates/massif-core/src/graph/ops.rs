//! Running one node: adjustments, combinations, masks and the dispatch to
//! landforms, simulations and colours.

use super::{Bend, ColorField, CombineOp, Extent, Field, NodeKind, TintMode, Value, color, landforms, sim};
use crate::noise::Noise;
use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Run one node. `Err(true)`: an input is missing; `Err(false)`: cancelled.
pub(super) fn run(kind: &NodeKind, e: &Extent, inputs: &[Option<Value>], wanted: &[bool], cancel: &AtomicBool) -> Result<Vec<Value>, bool> {
    let n = e.n;
    // An output nobody wants: an empty field, never looked at.
    let want = |k: usize| wanted.get(k).copied().unwrap_or(true);
    let full = |f: &Field, k: usize| if want(k) { upsample(f, n) } else { Field::new(0, Vec::new()) };
    let field = |k: usize| inputs.get(k).cloned().flatten().map(|v| v.as_field()).ok_or(true);
    let colors = |k: usize| inputs.get(k).cloned().flatten().map(|v| v.as_color()).ok_or(true);
    let one = |f: Field| Ok(vec![Value::Field(Arc::new(f))]);
    let paint = |c: ColorField| Ok(vec![Value::Color(Arc::new(c))]);
    let map = |f: &Field, op: &(dyn Fn(f32) -> f32 + Sync)| Field::new(n, f.data.par_iter().map(|&v| op(v)).collect());
    if let Some(f) = landforms::generate_kind(kind, e) {
        return one(f);
    }
    match *kind {
        NodeKind::Levels {
            in_min,
            in_max,
            out_min,
            out_max,
            gamma,
        } => {
            let f = field(0)?;
            let span = (in_max - in_min).max(1e-6);
            let g = gamma.max(0.01);
            one(map(&f, &|v| {
                let t = ((v - in_min) / span).clamp(0.0, 1.0).powf(g);
                out_min + t * (out_max - out_min)
            }))
        }
        NodeKind::Terrace { steps, strength } => {
            let f = field(0)?;
            let (lo, hi) = f.range();
            let span = (hi - lo).max(1e-6);
            let k = steps.max(1) as f32;
            let s = strength.clamp(0.0, 1.0);
            one(map(&f, &|v| {
                let t = (v - lo) / span * k;
                let floor = t.floor();
                let frac = t - floor;
                // Flat steps with steep risers, softened by the strength.
                let riser = (frac - 0.85).max(0.0) / 0.15;
                let stepped = (floor + riser * riser * (3.0 - 2.0 * riser)) / k;
                let t = t / k;
                lo + (t + (stepped - t) * s) * span
            }))
        }
        NodeKind::Clamp { min, max } => {
            let f = field(0)?;
            one(map(&f, &|v| v.clamp(min, max.max(min))))
        }
        NodeKind::Blur { radius } => {
            let f = field(0)?;
            one(blur(&f, (radius / e.spacing()).round().max(1.0) as usize))
        }
        NodeKind::Warp { amount, scale, seed } => {
            let f = field(0)?;
            let nx = Noise::new(seed as u64 ^ 0x77a2);
            let nz = nx.derive(1);
            let s = scale.max(1.0) as f64;
            let a = amount as f64 / e.spacing() as f64;
            one(Field::new(
                n,
                (0..n * n)
                    .into_par_iter()
                    .map(|c| {
                        let (i, j) = (c % n, c / n);
                        let (x, z) = e.world(i, j);
                        let dx = nx.fbm(x / s, z / s, 3, 2.0, 0.5) * a;
                        let dz = nz.fbm(x / s, z / s, 3, 2.0, 0.5) * a;
                        f.sample((i as f64 + dx) as f32, (j as f64 + dz) as f32)
                    })
                    .collect(),
            ))
        }
        NodeKind::Normalize { min, max } => {
            let f = field(0)?;
            let (lo, hi) = f.range();
            let span = (hi - lo).max(1e-6);
            one(map(&f, &|v| min + (v - lo) / span * (max - min)))
        }
        NodeKind::Curve { gamma, contrast } => {
            let f = field(0)?;
            let (lo, hi) = f.range();
            let span = (hi - lo).max(1e-6);
            let g = gamma.clamp(0.05, 20.0);
            let c = contrast.clamp(-1.0, 1.0);
            one(map(&f, &|v| {
                let t = ((v - lo) / span).clamp(0.0, 1.0).powf(g);
                let curved = if c >= 0.0 {
                    t + (t * t * (3.0 - 2.0 * t) - t) * c
                } else {
                    // The inverse of the S: flatter in the middle.
                    let inv = 0.5 - ((1.0 - 2.0 * t).clamp(-1.0, 1.0).asin() / 3.0).sin();
                    t + (inv - t) * -c
                };
                lo + curved * span
            }))
        }
        NodeKind::Transform {
            scale,
            rotate,
            offset_x,
            offset_z,
        } => {
            let f = field(0)?;
            let (s, c) = (-rotate.to_radians()).sin_cos();
            let k = 1.0 / scale.max(0.01);
            one(resample(&f, &|u, v| {
                let (u, v) = (u - offset_x, v - offset_z);
                ((u * c - v * s) * k, (u * s + v * c) * k)
            }))
        }
        NodeKind::Swirl { angle, radius } => {
            let f = field(0)?;
            let r0 = radius.max(0.01);
            one(resample(&f, &|u, v| {
                let r = u.hypot(v);
                if r >= r0 {
                    return (u, v);
                }
                let t = 1.0 - r / r0;
                let (s, c) = (-(angle.to_radians() * t * t)).sin_cos();
                (u * c - v * s, u * s + v * c)
            }))
        }
        NodeKind::Fold { level, strength } => {
            let f = field(0)?;
            let s = strength.clamp(0.0, 1.0);
            one(map(&f, &|v| {
                let folded = if v > level { level - (v - level) } else { v };
                v + (folded - v) * s
            }))
        }
        NodeKind::Stratify {
            layers,
            strength,
            tilt,
            seed,
        } => {
            let f = field(0)?;
            one(stratify(&f, e, layers.clamp(1, 200), strength.clamp(0.0, 1.0), tilt, seed))
        }
        NodeKind::Sharpen { radius, amount } => {
            let f = field(0)?;
            let b = blur(&f, (radius / e.spacing()).round().max(1.0) as usize);
            one(Field::new(n, f.data.par_iter().zip(&b.data).map(|(&v, &m)| v + (v - m) * amount).collect()))
        }
        NodeKind::InvertHeight => {
            let f = field(0)?;
            let (lo, hi) = f.range();
            one(map(&f, &|v| lo + hi - v))
        }
        NodeKind::Erosion {
            rain,
            erosion,
            deposition,
            capacity,
            evaporation,
            radius,
            seed,
        } => {
            let f = field(0)?;
            let (work, we) = coarser(&f, e, SIM_MAX);
            let m = work.n;
            let params = sim::Droplets {
                drops: ((rain.max(0.0) * (m * m) as f32) as usize).min(256 << 20),
                erosion: erosion.clamp(0.0, 1.0),
                deposition: deposition.clamp(0.0, 1.0),
                capacity: capacity.max(0.0),
                evaporation: evaporation.clamp(0.0, 0.5),
                radius: (radius / we.spacing()).round().clamp(1.0, 12.0) as i32,
                seed,
            };
            let (h, flow, deposits) = sim::erode(&work, we.spacing(), &params, cancel).ok_or(false)?;
            let wear = normalized(work.data.par_iter().zip(&h.data).map(|(a, b)| (a - b).max(0.0)).collect(), m);
            let h = carry(&f, &work, &h);
            Ok([h, full(&flow, 1), full(&deposits, 2), full(&wear, 3)]
                .into_iter()
                .map(|f| Value::Field(Arc::new(f)))
                .collect())
        }
        NodeKind::Thermal {
            talus,
            iterations,
            strength,
        } => {
            let f = field(0)?;
            let (work, we) = coarser(&f, e, 2 * SIM_MAX);
            let h = sim::thermal(&work, we.spacing(), talus, iterations.min(500), strength.clamp(0.0, 1.0));
            let talus = normalized(work.data.par_iter().zip(&h.data).map(|(a, b)| (b - a).max(0.0)).collect(), work.n);
            Ok(vec![Value::Field(Arc::new(carry(&f, &work, &h))), Value::Field(Arc::new(full(&talus, 1)))])
        }
        NodeKind::Snow {
            line,
            slope,
            depth,
            softness,
        } => {
            let f = field(0)?;
            let slopes = slope_degrees(&f, e.spacing());
            let soft = softness.max(0.5);
            let mask = Field::new(
                n,
                (0..n * n)
                    .into_par_iter()
                    .map(|c| {
                        let high = smoothstep(line - soft, line + soft, f.data[c]);
                        let holds = 1.0 - smoothstep(slope - 8.0, slope + 8.0, slopes.data[c]);
                        high * holds
                    })
                    .collect(),
            );
            let mask = blur(&mask, 1);
            let h = Field::new(n, f.data.par_iter().zip(&mask.data).map(|(&v, &m)| v + depth * m).collect());
            Ok(vec![Value::Field(Arc::new(h)), Value::Field(Arc::new(mask))])
        }
        NodeKind::Rivers { threshold, depth, width } => {
            let f = field(0)?;
            let (work, we) = coarser(&f, e, SIM_MAX);
            let (h, rivers) = sim::rivers(&work, we.spacing(), threshold.clamp(0.0, 1.0), depth, width, cancel).ok_or(false)?;
            Ok(vec![Value::Field(Arc::new(carry(&f, &work, &h))), Value::Field(Arc::new(full(&rivers, 1)))])
        }
        NodeKind::Lakes { amount } => {
            let f = field(0)?;
            let (work, _) = coarser(&f, e, SIM_MAX);
            let m = work.n;
            let filled = sim::fill(&work, 0.0);
            let a = amount.clamp(0.0, 1.0);
            let h = Field::new(m, work.data.par_iter().zip(&filled).map(|(&v, &w)| v + (w - v) * a).collect());
            let lakes = Field::new(m, work.data.par_iter().zip(&filled).map(|(&v, &w)| ((w - v) * a / 1.5).clamp(0.0, 1.0)).collect());
            Ok(vec![Value::Field(Arc::new(carry(&f, &work, &h))), Value::Field(Arc::new(full(&lakes, 1)))])
        }
        NodeKind::Sea { level, shore } => {
            let f = field(0)?;
            let water = map(&f, &|v| (level - v + 0.5).clamp(0.0, 1.0));
            let s = shore.max(0.5);
            let coast = map(&f, &|v| band(v, level - s, level + s, s * 0.5));
            Ok(vec![Value::Field(f), Value::Field(Arc::new(water)), Value::Field(Arc::new(coast))])
        }
        NodeKind::Combine { op, weight } => {
            let (a, b) = (field(0)?, field(1)?);
            one(Field::new(
                n,
                a.data
                    .par_iter()
                    .zip(&b.data)
                    .map(|(&x, &raw)| {
                        let y = raw * weight;
                        match op {
                            CombineOp::Add => x + y,
                            CombineOp::Subtract => x - y,
                            CombineOp::Multiply => x * y,
                            CombineOp::Max => x.max(y),
                            CombineOp::Min => x.min(y),
                            CombineOp::Average => (x + y) * 0.5,
                            CombineOp::Lerp => x + (raw - x) * weight,
                            CombineOp::Screen => 1.0 - (1.0 - x) * (1.0 - y),
                            CombineOp::Overlay => {
                                if x < 0.5 {
                                    2.0 * x * y
                                } else {
                                    1.0 - 2.0 * (1.0 - x) * (1.0 - y)
                                }
                            }
                            CombineOp::Difference => (x - y).abs(),
                        }
                    })
                    .collect(),
            ))
        }
        NodeKind::Blend => {
            let (a, b, m) = (field(0)?, field(1)?, field(2)?);
            one(Field::new(
                n,
                (0..n * n)
                    .into_par_iter()
                    .map(|c| {
                        let t = m.data[c].clamp(0.0, 1.0);
                        a.data[c] + (b.data[c] - a.data[c]) * t
                    })
                    .collect(),
            ))
        }
        NodeKind::SelectHeight { min, max, falloff } => {
            let f = field(0)?;
            one(map(&f, &|v| band(v, min, max, falloff)))
        }
        NodeKind::SelectSlope { min, max, falloff } => {
            let f = field(0)?;
            let slopes = slope_degrees(&f, e.spacing());
            one(map(&slopes, &|d| band(d, min, max, falloff)))
        }
        NodeKind::Invert => {
            let f = field(0)?;
            one(map(&f, &|v| 1.0 - v))
        }
        NodeKind::Curvature { bend, radius, strength } => {
            let f = field(0)?;
            one(curvature(&f, e, bend, radius, strength))
        }
        NodeKind::Facing { angle, spread } => {
            let f = field(0)?;
            let s = e.spacing();
            let spread = spread.clamp(1.0, 180.0);
            one(Field::new(
                n,
                (0..n * n)
                    .into_par_iter()
                    .map(|c| {
                        let (i, j) = (c % n, c / n);
                        let (gx, gz) = gradient(&f, i, j, s);
                        let steep = gx.hypot(gz).atan().to_degrees();
                        // The way the slope faces is downhill: 0 north, 90 east.
                        let facing = (-gx).atan2(gz).to_degrees();
                        let d = ((facing - angle).rem_euclid(360.0) - 180.0).abs();
                        let d = 180.0 - d;
                        smoothstep(spread, spread * 0.4, d) * smoothstep(2.0, 12.0, steep)
                    })
                    .collect(),
            ))
        }
        NodeKind::Occlusion { radius } => {
            let f = field(0)?;
            one(occlusion(&f, e, radius))
        }
        NodeKind::Flow { spread } => {
            let f = field(0)?;
            let (work, _) = coarser(&f, e, SIM_MAX);
            let m = work.n;
            let filled = sim::fill(&work, 1e-3);
            let acc = sim::accumulate(&filled, m);
            let max = acc.par_iter().copied().reduce(|| 1.0f32, f32::max);
            let l = (1.0 + max).ln();
            let g = 3.0 + (0.6 - 3.0) * spread.clamp(0.0, 1.0);
            one(upsample(&Field::new(m, acc.par_iter().map(|&a| ((1.0 + a).ln() / l).powf(g)).collect()), n))
        }
        NodeKind::Gradient {
            preset,
            auto_range,
            min,
            max,
            reverse,
            variation,
            seed,
        } => {
            let f = field(0)?;
            let (lo, hi) = if auto_range { f.range() } else { (min, max) };
            paint(color::gradient(&f, e, preset, lo, hi, reverse, variation, seed))
        }
        NodeKind::SolidColor { color: c } => paint(ColorField { n, data: vec![c; n * n] }),
        NodeKind::MixColors => {
            let (a, b, m) = (colors(0)?, colors(1)?, field(2)?);
            paint(ColorField::build(n, |c| {
                let t = m.data[c].clamp(0.0, 1.0);
                let (x, y) = (a.linear(c), b.linear(c));
                [0, 1, 2].map(|k| x[k] + (y[k] - x[k]) * t)
            }))
        }
        NodeKind::AdjustColor {
            hue,
            saturation,
            brightness,
            contrast,
        } => {
            let a = colors(0)?;
            paint(ColorField::build(n, |c| color::adjust(a.linear(c), hue, saturation, brightness, contrast)))
        }
        NodeKind::Tint { color: tint, mode, strength } => {
            let (a, m) = (colors(0)?, field(1)?);
            let t = tint.map(color::decode);
            paint(ColorField::build(n, |c| {
                let k = (m.data[c] * strength).clamp(0.0, 1.0);
                let x = a.linear(c);
                [0, 1, 2].map(|q| match mode {
                    TintMode::Mix => x[q] + (t[q] - x[q]) * k,
                    TintMode::Multiply => x[q] * (1.0 + (t[q] - 1.0) * k),
                    TintMode::Screen => 1.0 - (1.0 - x[q]) * (1.0 - t[q] * k),
                })
            }))
        }
        NodeKind::AutoColor {
            palette,
            snow_line,
            rock_slope,
            seed,
        } => {
            let f = field(0)?;
            paint(color::auto(&f, e, palette, snow_line, rock_slope, seed))
        }
        NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } | NodeKind::ColorOutput { .. } => Ok(Vec::new()),
        // Generators and landforms were made above.
        _ => Err(true),
    }
}

/// The most samples a side water simulations (erosion, rivers, lakes, flow)
/// work at, thermal erosion twice that: beyond, they work on a coarser copy
/// of the relief and the change they make is carried back to every sample,
/// so that a 16k build keeps all of its detail and finishes in seconds.
pub(super) const SIM_MAX: usize = 4096;

/// The field itself if it has at most `max` samples a side, otherwise a
/// box-filtered copy at `max`; with the extent it covers.
fn coarser(f: &Arc<Field>, e: &Extent, max: usize) -> (Arc<Field>, Extent) {
    if f.n <= max {
        return (f.clone(), *e);
    }
    (Arc::new(downsample(f, max)), Extent { n: max, ..*e })
}

/// A box-filtered copy of a field at `m` samples a side.
pub(super) fn downsample(f: &Field, m: usize) -> Field {
    let n = f.n;
    let k = n as f64 / m as f64;
    Field::new(
        m,
        (0..m * m)
            .into_par_iter()
            .map(|c| {
                let (i, j) = (c % m, c / m);
                let (a0, a1) = ((i as f64 * k) as usize, (((i + 1) as f64 * k).ceil() as usize).min(n));
                let (b0, b1) = ((j as f64 * k) as usize, (((j + 1) as f64 * k).ceil() as usize).min(n));
                let mut sum = 0.0f64;
                for jj in b0..b1 {
                    for ii in a0..a1 {
                        sum += f.data[jj * n + ii] as f64;
                    }
                }
                (sum / ((a1 - a0) * (b1 - b0)).max(1) as f64) as f32
            })
            .collect(),
    )
}

/// A bilinear copy of a field at `n` samples a side (itself if it has them).
pub(super) fn upsample(f: &Field, n: usize) -> Field {
    let m = f.n;
    if m == n {
        return f.clone();
    }
    let k = m as f32 / n as f32;
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| f.sample(((c % n) as f32 + 0.5) * k - 0.5, ((c / n) as f32 + 0.5) * k - 0.5))
            .collect(),
    )
}

/// `heights`, changed the way its coarse copy `before` became `after`.
fn carry(heights: &Field, before: &Field, after: &Field) -> Field {
    if before.n == heights.n {
        return after.clone();
    }
    let delta = Field::new(after.n, after.data.par_iter().zip(&before.data).map(|(a, b)| a - b).collect());
    let up = upsample(&delta, heights.n);
    Field::new(heights.n, heights.data.par_iter().zip(&up.data).map(|(h, d)| h + d).collect())
}

#[inline]
pub(super) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1 inside `[min, max]`, fading to 0 over `falloff` outside.
#[inline]
pub(super) fn band(v: f32, min: f32, max: f32, falloff: f32) -> f32 {
    let f = falloff.max(1e-4);
    let lo = ((v - min) / f + 0.5).clamp(0.0, 1.0);
    let hi = ((max - v) / f + 0.5).clamp(0.0, 1.0);
    lo.min(hi)
}

/// Values stretched to 0..1 on a log scale (flows, deposits).
pub(super) fn normalized(v: Vec<f32>, n: usize) -> Field {
    let max = v.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let l = (1.0 + max).ln();
    Field::new(n, v.into_iter().map(|x| (1.0 + x.max(0.0)).ln() / l).collect())
}

/// Height change per block at sample `(i, j)`, along x and z.
#[inline]
pub(super) fn gradient(f: &Field, i: usize, j: usize, spacing: f32) -> (f32, f32) {
    let n = f.n;
    let (l, r) = (i.saturating_sub(1), (i + 1).min(n - 1));
    let (u, d) = (j.saturating_sub(1), (j + 1).min(n - 1));
    let gx = (f.at(r, j) - f.at(l, j)) / ((r - l).max(1) as f32 * spacing);
    let gz = (f.at(i, d) - f.at(i, u)) / ((d - u).max(1) as f32 * spacing);
    (gx, gz)
}

/// The slope of every sample, in degrees.
pub(super) fn slope_degrees(f: &Field, spacing: f32) -> Field {
    let n = f.n;
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (gx, gz) = gradient(f, c % n, c / n, spacing);
                gx.hypot(gz).atan().to_degrees()
            })
            .collect(),
    )
}

/// Sample the field at points given, for every output sample, by `to`
/// from the sample's position around the middle of the map (−0.5..0.5).
fn resample(f: &Field, to: &(dyn Fn(f32, f32) -> (f32, f32) + Sync)) -> Field {
    let n = f.n;
    let k = n as f32;
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (u, v) = (((c % n) as f32 + 0.5) / k - 0.5, ((c / n) as f32 + 0.5) / k - 0.5);
                let (su, sv) = to(u, v);
                f.sample((su + 0.5) * k - 0.5, (sv + 0.5) * k - 0.5)
            })
            .collect(),
    )
}

/// Two passes of a separable box blur of `r` samples: close to a Gaussian.
/// Rows are blurred in place; columns as the rows of a transposed copy, so
/// the blur needs two fields' worth of memory, not three.
pub(super) fn blur(f: &Field, r: usize) -> Field {
    let n = f.n;
    let mut a = f.data.clone();
    let mut t = vec![0f32; n * n];
    for _ in 0..2 {
        box_rows(&mut a, n, r);
        transpose(&a, &mut t, n);
        box_rows(&mut t, n, r);
        transpose(&t, &mut a, n);
    }
    Field::new(n, a)
}

/// A box blur of `r` samples along every row of an `n` × `n` grid.
fn box_rows(data: &mut [f32], n: usize, r: usize) {
    data.par_chunks_mut(n).for_each_init(
        || vec![0f32; n],
        |src, row| {
            src.copy_from_slice(row);
            let mut sum = 0f32;
            let mut count = 0f32;
            for v in src.iter().take(n.min(r + 1)) {
                sum += v;
                count += 1.0;
            }
            for (t, o) in row.iter_mut().enumerate() {
                *o = sum / count;
                if t + r + 1 < n {
                    sum += src[t + r + 1];
                    count += 1.0;
                }
                if t >= r {
                    sum -= src[t - r];
                    count -= 1.0;
                }
            }
        },
    );
}

/// `dst` = `src` transposed, by tiles that stay in the caches.
fn transpose(src: &[f32], dst: &mut [f32], n: usize) {
    const B: usize = 64;
    dst.par_chunks_mut(B * n).enumerate().for_each(|(k, rows)| {
        let j0 = k * B;
        let height = rows.len() / n;
        for i0 in (0..n).step_by(B) {
            for dj in 0..height {
                let j = j0 + dj;
                for i in i0..(i0 + B).min(n) {
                    rows[dj * n + i] = src[i * n + j];
                }
            }
        }
    });
}

/// Layers of rock of random thickness: within each layer the ground stays
/// nearly flat and rises quickly at its top, so slopes get steps.
fn stratify(f: &Field, e: &Extent, layers: u32, strength: f32, tilt: f32, seed: u32) -> Field {
    let n = f.n;
    let (lo, hi) = f.range();
    let span = (hi - lo).max(1e-6);
    // Boundaries of the layers, from 0 to 1, of uneven thickness.
    let mut bounds = vec![0.0f32];
    let mut total = 0.0;
    let mut thickness = Vec::new();
    for k in 0..layers {
        let r = crate::compose::hash2(k as i32, 7, seed as u64 ^ 0x57a7) as f32 / u32::MAX as f32;
        let t = 0.35 + r * r * 1.6;
        thickness.push(t);
        total += t;
    }
    let mut acc = 0.0;
    for t in thickness {
        acc += t / total;
        bounds.push(acc);
    }
    let tan = tilt.clamp(-60.0, 60.0).to_radians().tan();
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| {
                let i = c % n;
                // Tilted layers: their height shifts across the map.
                let shift = tan * ((i as f32 + 0.5) / n as f32 - 0.5) * e.size as f32 / span;
                let t = (f.data[c] - lo) / span + shift;
                let k = bounds.partition_point(|b| *b <= t).clamp(1, bounds.len() - 1);
                let (b0, b1) = (bounds[k - 1], bounds[k]);
                let u = ((t - b0) / (b1 - b0).max(1e-6)).clamp(0.0, 1.0);
                let riser = smoothstep(0.7, 1.0, u);
                let stepped = b0 + (b1 - b0) * riser;
                let t2 = t + (stepped - t) * strength;
                lo + (t2 - shift) * span
            })
            .collect(),
    )
}

/// Ridges (convex) or hollows (concave): how far the ground stands above or
/// below the mean around it, normalised by how much it does on average.
fn curvature(f: &Field, e: &Extent, bend: Bend, radius: f32, strength: f32) -> Field {
    let n = f.n;
    let b = blur(f, (radius / e.spacing()).round().max(1.0) as usize);
    let lap: Vec<f32> = b.data.par_iter().zip(&f.data).map(|(m, v)| m - v).collect();
    let scale = (lap.iter().map(|v| v.abs()).sum::<f32>() / lap.len() as f32 * 2.5).max(1e-4);
    let sign = if bend == Bend::Concave { 1.0 } else { -1.0 };
    Field::new(n, lap.par_iter().map(|v| (v * sign / scale * strength.max(0.0)).clamp(0.0, 1.0)).collect())
}

/// How open to the sky the ground is: at a few scales, the angle the mean
/// ground around stands above the sample.
fn occlusion(f: &Field, e: &Extent, radius: f32) -> Field {
    let n = f.n;
    let s = e.spacing();
    let mut total = vec![0f32; n * n];
    let scales = [0.25f32, 0.5, 1.0];
    for k in scales {
        let r_blocks = (radius * k).max(s);
        let b = blur(f, (r_blocks / s).round().max(1.0) as usize);
        total
            .par_iter_mut()
            .zip(b.data.par_iter().zip(&f.data))
            .for_each(|(t, (m, v))| *t += ((m - v).max(0.0) / r_blocks).atan());
    }
    let k = 1.0 / scales.len() as f32 / std::f32::consts::FRAC_PI_2;
    Field::new(n, total.into_iter().map(|t| (1.0 - t * k * 2.5).clamp(0.0, 1.0)).collect())
}
