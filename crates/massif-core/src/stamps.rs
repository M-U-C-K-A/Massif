//! Terrain stamps: brushes shaped like landforms (peaks, ranges, volcanoes,
//! mesas…), for sculpting a recognisable shape in one stroke with the height
//! tools. They are generated, deterministically, so they look the same every
//! time; the formulas are those of the stamps of the modernised WorldPainter.

use crate::brush::BrushMask;
use crate::noise::Noise;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// The side of the generated images, in pixels.
pub const SIZE: usize = 256;

/// Groups of stamps, in the order the brush panel shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Family {
    Mountains,
    Hills,
    Volcanoes,
    Valleys,
}

impl Family {
    pub const ALL: [Family; 4] = [Family::Mountains, Family::Hills, Family::Volcanoes, Family::Valleys];

    pub const fn label(self) -> &'static str {
        match self {
            Family::Mountains => "Mountains",
            Family::Hills => "Hills & plateaus",
            Family::Volcanoes => "Volcanoes & craters",
            Family::Valleys => "Valleys",
        }
    }
}

/// The stamps. Their order fixes their noise: add new ones at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stamp {
    Peak,
    Range,
    Massif,
    Volcano,
    Crater,
    Mesa,
    Dunes,
    Hills,
    Gullies,
    AlpinePeak,
    TwinPeaks,
    Crag,
    MountainChain,
    ErodedRidge,
    Cirque,
    Plateau,
    Butte,
    Escarpment,
    KarstTowers,
    ShieldVolcano,
    Valley,
    Terraces,
}

impl Stamp {
    pub const ALL: [Stamp; 22] = [
        Stamp::Peak,
        Stamp::Range,
        Stamp::Massif,
        Stamp::Volcano,
        Stamp::Crater,
        Stamp::Mesa,
        Stamp::Dunes,
        Stamp::Hills,
        Stamp::Gullies,
        Stamp::AlpinePeak,
        Stamp::TwinPeaks,
        Stamp::Crag,
        Stamp::MountainChain,
        Stamp::ErodedRidge,
        Stamp::Cirque,
        Stamp::Plateau,
        Stamp::Butte,
        Stamp::Escarpment,
        Stamp::KarstTowers,
        Stamp::ShieldVolcano,
        Stamp::Valley,
        Stamp::Terraces,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Stamp::Peak => "Peak",
            Stamp::Range => "Mountain range",
            Stamp::Massif => "Massif",
            Stamp::Volcano => "Volcano",
            Stamp::Crater => "Crater rim",
            Stamp::Mesa => "Mesa",
            Stamp::Dunes => "Dunes",
            Stamp::Hills => "Hills",
            Stamp::Gullies => "Eroded dome",
            Stamp::AlpinePeak => "Alpine peak",
            Stamp::TwinPeaks => "Twin peaks",
            Stamp::Crag => "Crag",
            Stamp::MountainChain => "Mountain chain",
            Stamp::ErodedRidge => "Eroded ridge",
            Stamp::Cirque => "Glacial cirque",
            Stamp::Plateau => "Plateau",
            Stamp::Butte => "Butte",
            Stamp::Escarpment => "Escarpment",
            Stamp::KarstTowers => "Karst towers",
            Stamp::ShieldVolcano => "Shield volcano",
            Stamp::Valley => "Valley (use with Lower)",
            Stamp::Terraces => "Terraced hill",
        }
    }

    pub const fn family(self) -> Family {
        match self {
            Stamp::Peak
            | Stamp::Range
            | Stamp::Massif
            | Stamp::AlpinePeak
            | Stamp::TwinPeaks
            | Stamp::Crag
            | Stamp::MountainChain
            | Stamp::ErodedRidge
            | Stamp::Cirque => Family::Mountains,
            Stamp::Mesa
            | Stamp::Dunes
            | Stamp::Hills
            | Stamp::Gullies
            | Stamp::Plateau
            | Stamp::Butte
            | Stamp::Escarpment
            | Stamp::KarstTowers
            | Stamp::Terraces => Family::Hills,
            Stamp::Volcano | Stamp::Crater | Stamp::ShieldVolcano => Family::Volcanoes,
            Stamp::Valley => Family::Valleys,
        }
    }

    fn ordinal(self) -> u64 {
        Stamp::ALL.iter().position(|s| *s == self).unwrap_or(0) as u64
    }

    /// The greyscale image of the stamp, `SIZE` pixels square, brightest
    /// at its highest point.
    pub fn render(self) -> BrushMask {
        let noise = Noise::new(self.ordinal() * 7919 + 17);
        let values: Vec<f32> = (0..SIZE * SIZE)
            .into_par_iter()
            .map(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let u = (x as f64 + 0.5) / SIZE as f64 * 2.0 - 1.0;
                let v = (y as f64 + 0.5) / SIZE as f64 * 2.0 - 1.0;
                height(self, &noise, u, v).max(0.0) as f32
            })
            .collect();
        let max = values.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        let data = values.iter().map(|v| ((v / max).min(1.0) * 255.0).round() as u8).collect();
        BrushMask::new(self.label(), SIZE, SIZE, data)
    }
}

fn fbm(noise: &Noise, mut x: f64, mut y: f64, octaves: u32) -> f64 {
    let (mut sum, mut amplitude, mut total) = (0.0, 1.0, 0.0);
    for _ in 0..octaves {
        sum += noise.get(x, y) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        x *= 2.0;
        y *= 2.0;
    }
    sum / total
}

fn ridged(noise: &Noise, mut x: f64, mut y: f64, octaves: u32) -> f64 {
    let (mut sum, mut amplitude, mut total) = (0.0, 1.0, 0.0);
    for i in 0..octaves {
        let n = 1.0 - noise.get(x + 31.7 * i as f64, y - 17.3 * i as f64).abs().min(1.0);
        sum += n * n * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        x *= 2.0;
        y *= 2.0;
    }
    sum / total
}

fn smooth_step(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn pos(x: f64) -> f64 {
    x.max(0.0)
}

/// The height of a stamp at `(u, v)` in `-1..1`.
fn height(stamp: Stamp, noise: &Noise, u: f64, v: f64) -> f64 {
    let r = u.hypot(v);
    // Everything fades out towards the edge of the brush.
    let edge = smooth_step(1.0, 0.82, r);
    match stamp {
        Stamp::Peak => {
            let warped = r + 0.12 * fbm(noise, u * 2.0, v * 2.0, 3);
            let cone = pos(1.0 - warped).powf(1.6);
            cone * (0.75 + 0.35 * ridged(noise, u * 3.0, v * 3.0, 4)) * edge
        }
        Stamp::Range => {
            // A ridge along a slightly curved line, with spurs.
            let centre = 0.18 * (u * 2.4).sin() + 0.08 * fbm(noise, u * 2.0, 3.7, 2);
            let across = (v - centre).abs();
            let along = smooth_step(1.0, 0.55, u.abs());
            let profile = pos(1.0 - across / 0.42).powf(1.5);
            profile * along * (0.65 + 0.45 * ridged(noise, u * 4.0, v * 4.0, 4)) * edge
        }
        Stamp::Massif => {
            let peaks = [(-0.3, -0.2, 0.55, 1.0), (0.28, -0.1, 0.5, 0.85), (0.0, 0.32, 0.45, 0.75), (-0.05, -0.45, 0.35, 0.6)];
            let h = peaks.iter().fold(0.0f64, |h, &(px, pz, size, top)| {
                let d = (u - px).hypot(v - pz) / size;
                h.max(top * pos(1.0 - d).powf(1.4))
            });
            h * (0.8 + 0.3 * ridged(noise, u * 3.5, v * 3.5, 4)) * edge
        }
        Stamp::Volcano => {
            let cone = pos(1.0 - r).powf(1.3) * (0.92 + 0.08 * fbm(noise, u * 6.0, v * 6.0, 3));
            // A crater at the top.
            let crater = 0.42 * smooth_step(0.22, 0.08, r);
            pos(cone - crater) * edge
        }
        Stamp::Crater => {
            // A raised rim around a bowl; the Lower tool digs it.
            let rim = (-((r - 0.62) / 0.13).powi(2)).exp();
            let bowl = 0.25 * smooth_step(0.62, 0.0, r);
            (rim + bowl * 0.3) * (0.85 + 0.2 * fbm(noise, u * 5.0, v * 5.0, 3)) * edge
        }
        Stamp::Mesa => {
            let warped = r + 0.14 * fbm(noise, u * 2.5, v * 2.5, 4);
            let plateau = smooth_step(0.78, 0.58, warped);
            // Terraced flanks.
            let flank = (plateau * 4.0).floor() / 4.0 + smooth_step(0.0, 1.0, (plateau * 4.0) % 1.0) / 4.0;
            (0.85 * flank + 0.15 * plateau) * edge
        }
        Stamp::Dunes => {
            let phase = u * 3.2 + 0.35 * fbm(noise, u * 1.5, v * 1.5, 2);
            let f = phase - phase.floor();
            let profile = if f < 0.75 { smooth_step(0.0, 0.75, f) } else { smooth_step(1.0, 0.75, f) };
            profile * (0.7 + 0.3 * fbm(noise, u * 2.0, v * 2.0 + 9.0, 2)) * smooth_step(1.0, 0.4, r)
        }
        Stamp::Hills => {
            let h = 0.5 + 0.5 * fbm(noise, u * 2.2, v * 2.2, 3);
            pos(h).powf(1.8) * smooth_step(1.0, 0.3, r)
        }
        Stamp::Gullies => {
            // A dome cut by branching erosion channels.
            let dome = pos(1.0 - r).powf(0.9);
            let channels = 1.0 - (1.0 - ridged(noise, u * 3.0, v * 3.0, 5)).powi(3);
            dome * (0.55 + 0.45 * channels) * edge
        }
        Stamp::AlpinePeak => {
            // A pyramidal horn with three sharp arêtes down from the summit.
            let theta = v.atan2(u) + 0.35 * fbm(noise, u * 2.0, v * 2.0, 2);
            let arms = (0.5 + 0.5 * (3.0 * theta).cos()).powi(3);
            let cone = pos(1.0 - r).powf(1.15);
            cone * (0.6 + 0.4 * arms * smooth_step(0.0, 0.25, r) + 0.4 * smooth_step(0.25, 0.0, r))
                * (0.85 + 0.25 * ridged(noise, u * 4.0, v * 4.0, 4))
                * edge
        }
        Stamp::TwinPeaks => {
            let first = pos(1.0 - (u + 0.32).hypot(v - 0.05) / 0.58).powf(1.3);
            let second = 0.85 * pos(1.0 - (u - 0.34).hypot(v + 0.08) / 0.52).powf(1.3);
            let saddle = 0.35 * (-(v + 0.015 * u).powi(2) / 0.03).exp() * smooth_step(0.75, 0.2, u.abs());
            first.max(second).max(saddle) * (0.8 + 0.3 * ridged(noise, u * 3.5, v * 3.5, 4)) * edge
        }
        Stamp::Crag => {
            // A steep, jagged spire on a scree apron.
            let warped = r + 0.1 * fbm(noise, u * 3.0, v * 3.0, 3);
            let spire = pos(1.0 - warped / 0.5).powf(0.75) * (0.75 + 0.35 * ridged(noise, u * 6.0, v * 6.0, 4));
            let apron = 0.22 * pos(1.0 - r).powi(2);
            spire.max(apron) * edge
        }
        Stamp::MountainChain => {
            // A line of peaks of different heights joined by a lower ridge.
            let centre = 0.15 * (u * 2.0 + 0.6).sin() + 0.06 * fbm(noise, u * 2.5, 5.3, 2);
            let across = (v - centre).abs();
            let mut h = 0.45 * pos(1.0 - across / 0.3).powf(1.4) * smooth_step(1.0, 0.6, u.abs());
            for (pu, top) in [(-0.62, 0.55), (-0.3, 0.9), (0.02, 0.7), (0.32, 1.0), (0.62, 0.6)] {
                let pv = 0.15 * (pu * 2.0 + 0.6f64).sin();
                let d = (u - pu).hypot(v - pv) / 0.3;
                h = h.max(top * pos(1.0 - d).powf(1.3));
            }
            h * (0.75 + 0.35 * ridged(noise, u * 4.0, v * 4.0, 4)) * edge
        }
        Stamp::ErodedRidge => {
            let centre = 0.12 * (u * 2.8).sin() + 0.06 * fbm(noise, u * 2.0, 1.3, 2);
            let across = (v - centre).abs();
            let profile = pos(1.0 - across / 0.5).powf(1.2) * smooth_step(1.0, 0.5, u.abs());
            let channels = 1.0 - (1.0 - ridged(noise, u * 4.0, v * 4.0, 5)).powi(3);
            profile * (0.45 + 0.55 * channels) * edge
        }
        Stamp::Cirque => {
            // A horseshoe of steep walls around a glacial basin, open towards
            // the bottom of the brush.
            let ring = (-((r - 0.55) / 0.17).powi(2)).exp();
            let opening = smooth_step(0.55, -0.1, v);
            let back_wall = 0.4 * smooth_step(0.2, -0.6, v) * smooth_step(0.85, 0.5, r);
            let basin = 0.12 * smooth_step(0.5, 0.0, r);
            (ring * (0.4 + 0.6 * opening) + back_wall + basin) * (0.8 + 0.3 * ridged(noise, u * 4.0, v * 4.0, 4)) * edge
        }
        Stamp::Plateau => {
            let warped = r + 0.18 * fbm(noise, u * 2.0, v * 2.0, 4);
            let top = smooth_step(0.82, 0.66, warped);
            (0.88 * top + 0.04 * fbm(noise, u * 5.0, v * 5.0, 3) * top + 0.12 * pos(1.0 - r).powi(2)) * edge
        }
        Stamp::Butte => {
            let warped = r + 0.1 * fbm(noise, u * 3.0, v * 3.0, 3);
            let top = smooth_step(0.46, 0.36, warped);
            let talus = 0.3 * pos(1.0 - r / 0.8).powf(1.5);
            (top * (0.95 + 0.05 * fbm(noise, u * 6.0, v * 6.0, 2))).max(talus) * edge
        }
        Stamp::Escarpment => {
            // High ground on one side of a steep, irregular cliff.
            let cliff = u + 0.18 * fbm(noise, v * 2.2, 3.1, 4);
            let high = smooth_step(-0.06, 0.06, cliff);
            (0.25 + 0.7 * high + 0.05 * fbm(noise, u * 4.0, v * 4.0, 3)) * smooth_step(1.0, 0.7, r)
        }
        Stamp::KarstTowers => {
            // Steep limestone towers scattered over low ground.
            let mut h = 0.08 * (0.5 + 0.5 * fbm(noise, u * 3.0, v * 3.0, 2));
            let towers = [
                (-0.45, -0.3, 0.17, 0.85),
                (-0.1, -0.5, 0.13, 0.6),
                (0.3, -0.35, 0.16, 1.0),
                (-0.5, 0.2, 0.14, 0.7),
                (-0.05, 0.05, 0.2, 0.95),
                (0.42, 0.12, 0.13, 0.65),
                (-0.22, 0.48, 0.15, 0.8),
                (0.2, 0.5, 0.12, 0.55),
            ];
            for (tu, tv, size, top) in towers {
                let d = (u - tu).hypot(v - tv) + 0.04 * fbm(noise, u * 7.0 + tu * 9.0, v * 7.0, 2);
                h = h.max(top * smooth_step(size, size * 0.6, d) * (0.92 + 0.08 * fbm(noise, u * 9.0, v * 9.0, 2)));
            }
            h * edge
        }
        Stamp::ShieldVolcano => {
            // A broad, gentle dome with a small caldera.
            let dome = pos(1.0 - r * r).powf(1.6) * (0.95 + 0.05 * fbm(noise, u * 5.0, v * 5.0, 3));
            let caldera = 0.1 * smooth_step(0.14, 0.05, r);
            pos(dome - caldera) * edge
        }
        Stamp::Valley => {
            // A winding V-shaped valley with side gullies, for the Lower tool.
            let centre = 0.28 * (u * 2.2 + 0.4).sin() + 0.08 * fbm(noise, u * 2.0, 7.7, 2);
            let across = (v - centre).abs();
            let profile = pos(1.0 - across / 0.32).powf(1.4) * smooth_step(1.0, 0.65, u.abs());
            let gullies = 0.3 * ridged(noise, u * 5.0, v * 5.0, 3).powi(4) * smooth_step(0.55, 0.2, across);
            (profile + gullies * profile) * smooth_step(1.0, 0.85, r)
        }
        Stamp::Terraces => {
            // A hill cut into terraced fields.
            let hill = pos(1.0 - (r + 0.08 * fbm(noise, u * 2.0, v * 2.0, 3))).powf(1.1);
            let steps = 7.0;
            let level = (hill * steps).floor();
            let fraction = hill * steps - level;
            ((level + smooth_step(0.8, 1.0, fraction)) / steps) * edge
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stamp_renders_a_landform() {
        for stamp in Stamp::ALL {
            let mask = stamp.render();
            assert_eq!((mask.width, mask.height), (SIZE, SIZE));
            let max = *mask.data.iter().max().unwrap();
            assert_eq!(max, 255, "{stamp:?} is normalised");
            // The corners lie outside the disc of the brush: nothing there.
            assert_eq!(mask.data[0], 0, "{stamp:?}");
            assert_eq!(mask.data[SIZE * SIZE - 1], 0, "{stamp:?}");
            let lit = mask.data.iter().filter(|&&v| v > 0).count();
            assert!(lit > SIZE * SIZE / 20, "{stamp:?} covers too little: {lit}");
            assert!(!mask.outline.is_empty(), "{stamp:?} has an outline");
        }
        // Deterministic.
        assert_eq!(Stamp::Volcano.render().data, Stamp::Volcano.render().data);
    }

    #[test]
    fn every_family_has_stamps() {
        for family in Family::ALL {
            assert!(Stamp::ALL.iter().any(|s| s.family() == family), "{family:?}");
        }
    }
}
