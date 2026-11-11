//! Brushes, the WorldPainter way.
//!
//! A brush is either one of the twelve symmetric brushes (a circle or a
//! square with one of six profiles: Constant, Linear, Sine, Plateau, Spike,
//! Dome — the same formulas as WorldPainter) or a custom brush: a greyscale
//! image stretched over the brush's diameter. Every brush has a radius, an
//! intensity and a rotation.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The largest radius a brush can have, in blocks.
pub const MAX_RADIUS: f32 = 4096.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushShape {
    Circle,
    Square,
}

/// How the strength falls off from the centre to the rim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Profile {
    /// Full strength everywhere.
    Constant,
    /// Straight down to the rim.
    Linear,
    /// Half a cosine: soft at both ends.
    Sine,
    /// Full strength over the inner half, then soft.
    Plateau,
    /// A sharp peak.
    Spike,
    /// A quarter circle.
    Dome,
}

impl Profile {
    pub const ALL: [Profile; 6] = [
        Profile::Constant,
        Profile::Linear,
        Profile::Sine,
        Profile::Plateau,
        Profile::Spike,
        Profile::Dome,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Profile::Constant => "Constant",
            Profile::Linear => "Linear",
            Profile::Sine => "Sine",
            Profile::Plateau => "Plateau",
            Profile::Spike => "Spike",
            Profile::Dome => "Dome",
        }
    }

    /// Strength at relative distance `dr` (0 at the centre, 1 at the rim).
    #[inline]
    pub fn apply(self, dr: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            Profile::Constant => 1.0,
            Profile::Linear => 1.0 - dr,
            Profile::Sine => (dr * PI).cos() / 2.0 + 0.5,
            Profile::Plateau => {
                if dr <= 0.5 {
                    1.0
                } else {
                    ((dr - 0.5) * 2.0 * PI).cos() / 2.0 + 0.5
                }
            }
            Profile::Spike => (1.0 - dr) * (1.0 - dr),
            Profile::Dome => (1.0 - dr * dr).max(0.0).sqrt(),
        }
    }
}

/// Strength under which a brush image does not paint, for its outline.
const OUTLINE_THRESHOLD: f32 = 0.02;
/// Cells across the grid the outline of a brush image is traced on.
const OUTLINE_CELLS: usize = 96;

/// The greyscale image of a custom brush.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushMask {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Row-major strengths, 0..=255.
    pub data: Vec<u8>,
    /// Where the image stops painting, as segments in image units (0..1
    /// across, y down), for drawing the brush on the map.
    pub outline: Vec<[(f32, f32); 2]>,
}

impl BrushMask {
    /// A mask from row-major greyscale values.
    pub fn new(name: impl Into<String>, width: usize, height: usize, data: Vec<u8>) -> Self {
        assert_eq!(data.len(), width * height, "a mask needs width × height values");
        let mut mask = Self {
            name: name.into(),
            width,
            height,
            data,
            outline: Vec::new(),
        };
        mask.outline = mask.trace_outline();
        mask
    }

    /// A mask from row-major RGBA pixels: white paints, black does not, and
    /// transparent pixels count as black, as in WorldPainter.
    pub fn from_rgba(name: impl Into<String>, width: usize, height: usize, rgba: &[u8]) -> Self {
        let data = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let luma = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
                (luma * p[3] as f32 / 255.0).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Self::new(name, width, height, data)
    }

    /// Bilinear sample at `(u, v)` in `0..=1`, returning `0..=1`.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let x = u * (self.width - 1) as f32;
        let y = v * (self.height - 1) as f32;
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.data[y * self.width + x] as f32;
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        (top + (bottom - top) * fy) / 255.0
    }

    /// Marching squares around the part of the image that paints.
    fn trace_outline(&self) -> Vec<[(f32, f32); 2]> {
        let n = OUTLINE_CELLS.min(self.width.max(self.height)).max(2) as isize;
        let t = OUTLINE_THRESHOLD;
        // Corner values, with a ring of zeros around the image so that shapes
        // touching its sides close.
        let at = |i: isize, j: isize| -> f32 {
            if i < 0 || j < 0 || i > n || j > n {
                0.0
            } else {
                self.sample(i as f32 / n as f32, j as f32 / n as f32)
            }
        };
        let mut segments = Vec::new();
        for j in -1..=n {
            for i in -1..=n {
                // Top left, top right, bottom right, bottom left.
                let c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
                let case = c.iter().enumerate().fold(0, |acc, (k, v)| acc | (((*v > t) as u8) << k));
                if case == 0 || case == 15 {
                    continue;
                }
                let cross = |a: f32, b: f32| ((t - a) / (b - a)).clamp(0.0, 1.0);
                let (x, y) = (i as f32, j as f32);
                let top = (x + cross(c[0], c[1]), y);
                let right = (x + 1.0, y + cross(c[1], c[2]));
                let bottom = (x + cross(c[3], c[2]), y + 1.0);
                let left = (x, y + cross(c[0], c[3]));
                type Point = (f32, f32);
                let pairs: &[(Point, Point)] = match case {
                    1 | 14 => &[(left, top)],
                    2 | 13 => &[(top, right)],
                    3 | 12 => &[(left, right)],
                    4 | 11 => &[(right, bottom)],
                    5 => &[(left, top), (right, bottom)],
                    6 | 9 => &[(top, bottom)],
                    7 | 8 => &[(left, bottom)],
                    10 => &[(top, right), (bottom, left)],
                    _ => &[],
                };
                let unit = |(x, y): (f32, f32)| (x / n as f32, y / n as f32);
                segments.extend(pairs.iter().map(|&(a, b)| [unit(a), unit(b)]));
            }
        }
        segments
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brush {
    pub shape: BrushShape,
    pub profile: Profile,
    /// Radius in blocks.
    pub radius: f32,
    /// Intensity (0..1), WorldPainter's "level".
    pub strength: f32,
    /// Rotation in degrees, clockwise; changes squares and custom brushes.
    #[serde(default)]
    pub rotation: f32,
    /// The image of a custom brush; replaces shape and profile.
    #[serde(skip)]
    pub mask: Option<Arc<BrushMask>>,
    /// Where on the map the brush acts (WorldPainter's brush options).
    #[serde(skip)]
    pub filter: Option<Arc<crate::filter::Filter>>,
}

impl Default for Brush {
    fn default() -> Self {
        Self {
            shape: BrushShape::Circle,
            profile: Profile::Sine,
            radius: 24.0,
            strength: 0.5,
            rotation: 0.0,
            mask: None,
            filter: None,
        }
    }
}

impl Brush {
    /// A symmetric brush.
    pub fn symmetric(shape: BrushShape, profile: Profile, radius: f32) -> Self {
        Self {
            shape,
            profile,
            radius,
            ..Self::default()
        }
    }

    /// The display name, as in WorldPainter ("Sine Circle", a file name...).
    pub fn name(&self) -> String {
        match &self.mask {
            Some(m) => m.name.clone(),
            None => format!(
                "{} {}",
                self.profile.label(),
                match self.shape {
                    BrushShape::Circle => "Circle",
                    BrushShape::Square => "Square",
                }
            ),
        }
    }

    /// Undo the brush rotation on an offset from its centre.
    #[inline]
    fn unrotate(&self, dx: f32, dz: f32) -> (f32, f32) {
        if self.rotation == 0.0 {
            return (dx, dz);
        }
        let (s, c) = (-self.rotation.to_radians()).sin_cos();
        (dx * c - dz * s, dx * s + dz * c)
    }

    /// Turn the brush by `degrees`, keeping the angle in `0..360`.
    pub fn rotate(&mut self, degrees: f32) {
        self.rotation = (self.rotation + degrees).rem_euclid(360.0);
    }

    /// The corners of the brush's square footprint around `(cx, cz)`.
    pub fn square_corners(&self, cx: f32, cz: f32) -> [(f32, f32); 4] {
        let r = self.radius;
        let (s, c) = self.rotation.to_radians().sin_cos();
        [(-r, -r), (r, -r), (r, r), (-r, r)].map(|(x, z)| (cx + x * c - z * s, cz + x * s + z * c))
    }

    /// The outline of a custom brush, as segments in blocks from its centre,
    /// turned with the brush. `None` for the symmetric brushes, whose
    /// outline is their circle or square.
    pub fn outline(&self) -> Option<Vec<[(f32, f32); 2]>> {
        let mask = self.mask.as_ref()?;
        let d = 2.0 * self.radius;
        let (s, c) = self.rotation.to_radians().sin_cos();
        let place = |(u, v): (f32, f32)| {
            let (x, z) = ((u - 0.5) * d, (v - 0.5) * d);
            (x * c - z * s, x * s + z * c)
        };
        Some(mask.outline.iter().map(|[a, b]| [place(*a), place(*b)]).collect())
    }

    /// Whether the footprint is a (possibly rotated) square rather than a
    /// circle.
    pub fn is_square(&self) -> bool {
        self.mask.is_some() || self.shape == BrushShape::Square
    }

    /// Strength in `0..=1` at offset `(dx, dz)` from the brush centre,
    /// without the intensity. Zero outside the footprint.
    #[inline]
    pub fn weight(&self, dx: f32, dz: f32) -> f32 {
        let r = self.radius.max(0.5);
        if let Some(mask) = &self.mask {
            let (dx, dz) = self.unrotate(dx, dz);
            return mask.sample(dx / (2.0 * r) + 0.5, dz / (2.0 * r) + 0.5);
        }
        let dr = match self.shape {
            BrushShape::Circle => (dx * dx + dz * dz).sqrt(),
            BrushShape::Square => {
                let (dx, dz) = self.unrotate(dx, dz);
                dx.abs().max(dz.abs())
            }
        } / r;
        if dr > 1.0 {
            return 0.0;
        }
        self.profile.apply(dr).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_match_worldpainter() {
        use std::f32::consts::PI;
        for p in Profile::ALL {
            assert!((p.apply(0.0) - 1.0).abs() < 1e-6, "{p:?}");
        }
        assert_eq!(Profile::Linear.apply(0.25), 0.75);
        assert!((Profile::Sine.apply(0.5) - 0.5).abs() < 1e-6);
        assert_eq!(Profile::Plateau.apply(0.4), 1.0);
        assert!((Profile::Plateau.apply(0.75) - ((0.25 * 2.0 * PI).cos() / 2.0 + 0.5)).abs() < 1e-6);
        assert_eq!(Profile::Spike.apply(0.5), 0.25);
        assert!((Profile::Dome.apply(0.6) - 0.8).abs() < 1e-6);
        // Everything but Constant and Dome reaches zero at the rim.
        for p in [Profile::Linear, Profile::Sine, Profile::Plateau, Profile::Spike] {
            assert!(p.apply(1.0).abs() < 1e-6, "{p:?}");
        }
    }

    #[test]
    fn weight_is_one_at_centre_and_zero_outside() {
        for profile in Profile::ALL {
            for shape in [BrushShape::Circle, BrushShape::Square] {
                let b = Brush::symmetric(shape, profile, 10.0);
                assert_eq!(b.weight(0.0, 0.0), 1.0);
                assert_eq!(b.weight(10.5, 0.0), 0.0);
                assert_eq!(b.weight(0.0, -11.0), 0.0);
                let mid = b.weight(7.0, 0.0);
                assert!((0.0..=1.0).contains(&mid));
            }
        }
        let circle = Brush::symmetric(BrushShape::Circle, Profile::Constant, 10.0);
        let square = Brush::symmetric(BrushShape::Square, Profile::Constant, 10.0);
        // The corner of the square lies outside the circle.
        assert_eq!(circle.weight(8.0, 8.0), 0.0);
        assert_eq!(square.weight(8.0, 8.0), 1.0);
    }

    #[test]
    fn rotation_turns_square_brushes() {
        let mut b = Brush::symmetric(BrushShape::Square, Profile::Constant, 10.0);
        assert_eq!(b.weight(9.0, 9.0), 1.0);
        b.rotate(45.0);
        assert_eq!(b.weight(9.0, 9.0), 0.0);
        assert_eq!(b.weight(13.0, 0.0), 1.0);
        b.rotate(-90.0);
        assert_eq!(b.rotation, 315.0);
        let corners = b.square_corners(0.0, 0.0);
        assert!((corners[0].0.hypot(corners[0].1) - 200f32.sqrt()).abs() < 1e-4);
        let c = Brush { shape: BrushShape::Circle, ..b };
        assert_eq!(c.weight(0.0, 9.0), 1.0);
    }

    #[test]
    fn custom_brushes_stretch_their_image() {
        // A 3×3 image: bright left column, dark elsewhere.
        let mask = BrushMask::new("Stripe", 3, 3, vec![255, 0, 0, 255, 0, 0, 255, 0, 0]);
        let mut b = Brush {
            radius: 100.0,
            mask: Some(Arc::new(mask)),
            ..Brush::default()
        };
        assert_eq!(b.name(), "Stripe");
        assert!(b.is_square());
        assert_eq!(b.weight(-100.0, 0.0), 1.0);
        assert_eq!(b.weight(100.0, 0.0), 0.0);
        assert!((b.weight(-50.0, 30.0) - 0.5).abs() < 1e-6);
        assert_eq!(b.weight(-150.0, 0.0), 0.0);
        // Turned a quarter, the bright side is at the top.
        b.rotate(90.0);
        assert!(b.weight(0.0, -100.0) > 0.99);
        assert!(b.weight(-100.0, 0.0) < 0.01);
    }

    #[test]
    fn custom_brushes_have_an_outline() {
        // A filled disc of radius 0.4 (in image units) in a 64×64 image.
        let n = 64;
        let data = (0..n * n)
            .map(|i| {
                let (x, y) = ((i % n) as f32 / (n - 1) as f32 - 0.5, (i / n) as f32 / (n - 1) as f32 - 0.5);
                if x.hypot(y) < 0.4 { 255 } else { 0 }
            })
            .collect();
        let mask = BrushMask::new("Disc", n, n, data);
        assert!(mask.outline.len() > 20);
        for [a, b] in &mask.outline {
            for (x, y) in [a, b] {
                let r = (x - 0.5).hypot(y - 0.5);
                assert!((r - 0.4).abs() < 0.04, "point at {r}");
            }
        }
        let b = Brush {
            radius: 50.0,
            mask: Some(Arc::new(mask)),
            ..Brush::default()
        };
        let outline = b.outline().unwrap();
        let (x, z) = outline[0][0];
        assert!((x.hypot(z) - 40.0).abs() < 4.0);
        assert!(Brush::default().outline().is_none());
        // A grey image converted from RGBA: half-transparent white is half strength.
        let m = BrushMask::from_rgba("Grey", 1, 1, &[255, 255, 255, 128]);
        assert_eq!(m.data, vec![128]);
    }

    #[test]
    fn weight_decreases_monotonically() {
        for profile in Profile::ALL {
            let b = Brush::symmetric(BrushShape::Circle, profile, 20.0);
            let mut prev = 1.0;
            for i in 0..=20 {
                let w = b.weight(i as f32, 0.0);
                assert!(w <= prev + 1e-6, "{profile:?} at {i}");
                prev = w;
            }
        }
    }
}
