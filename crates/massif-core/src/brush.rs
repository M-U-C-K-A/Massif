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
/// The most samples per side the outline of a brush image is traced on:
/// smaller images are traced through every pixel.
const OUTLINE_MAX: usize = 1024;

/// The greyscale image of a custom brush.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushMask {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Row-major strengths, 0..=255.
    pub data: Vec<u8>,
    /// Where the image stops painting: closed loops in image units (0..1
    /// across, y down), traced through its pixels, for drawing the brush.
    pub outline: Vec<Vec<(f32, f32)>>,
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

    /// Marching squares around the part of the image that paints, through
    /// the centres of its pixels (where the brush's strength is exactly the
    /// pixel's), joined into closed loops.
    fn trace_outline(&self) -> Vec<Vec<(f32, f32)>> {
        if self.width == 0 || self.height == 0 {
            return Vec::new();
        }
        let (nx, ny) = (self.width.clamp(2, OUTLINE_MAX), self.height.clamp(2, OUTLINE_MAX));
        let exact = nx == self.width && ny == self.height;
        let values: Vec<f32> = (0..nx * ny)
            .map(|c| {
                let (i, j) = (c % nx, c / nx);
                if exact {
                    self.data[j * self.width + i] as f32 / 255.0
                } else {
                    self.sample(i as f32 / (nx - 1) as f32, j as f32 / (ny - 1) as f32)
                }
            })
            .collect();
        trace_grid(nx, ny, &values, |i, j| (i / (nx - 1) as f32, j / (ny - 1) as f32))
    }

    /// The outline traced on squares of `step` × `step` pixels, for drawing
    /// the brush where a pixel of its image is smaller than one of the
    /// screen: a square paints if any of its pixels does, so the outline
    /// holds all the image paints, and specks closer than a square merge
    /// into one shape rather than hundreds of loops too small to see.
    /// `step` 1 is [`BrushMask::outline`].
    pub fn outline_coarse(&self, step: usize) -> Vec<Vec<(f32, f32)>> {
        if step <= 1 || self.width == 0 || self.height == 0 {
            return self.outline.clone();
        }
        let (nx, ny) = (self.width.div_ceil(step).max(2), self.height.div_ceil(step).max(2));
        let values: Vec<f32> = (0..nx * ny)
            .map(|c| {
                let (i, j) = (c % nx, c / nx);
                let (x0, y0) = ((i * step).min(self.width), (j * step).min(self.height));
                let (x1, y1) = ((x0 + step).min(self.width), (y0 + step).min(self.height));
                (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| self.data[y * self.width + x]).max().unwrap_or(0) as f32 / 255.0
            })
            .collect();
        // A square's middle, in the image's units (its pixels' centres run
        // from 0 to 1).
        let (w, h) = ((self.width - 1).max(1) as f32, (self.height - 1).max(1) as f32);
        let s = step as f32;
        trace_grid(nx, ny, &values, |i, j| (((i + 0.5) * s - 0.5) / w, ((j + 0.5) * s - 0.5) / h))
    }
}

/// Marching squares around the values of an `nx` × `ny` grid over
/// [`OUTLINE_THRESHOLD`], joined into closed loops; `pos` places a corner
/// of the grid (its column and row) in the image's units.
fn trace_grid(nx: usize, ny: usize, values: &[f32], pos: impl Fn(f32, f32) -> (f32, f32)) -> Vec<Vec<(f32, f32)>> {
    let t = OUTLINE_THRESHOLD;
    // Corners outside the image are a ring of zeros, so that shapes
    // touching its sides close; the outline then runs along the side.
    let at = |i: isize, j: isize| -> Option<f32> {
        (i >= 0 && j >= 0 && (i as usize) < nx && (j as usize) < ny).then(|| values[j as usize * nx + i as usize])
    };
    let pos = |i: isize, j: isize| pos(i as f32, j as f32);
    // Where the outline crosses the edge from corner a to corner b.
    let crossing = |a: (isize, isize), b: (isize, isize)| -> (f32, f32) {
        let (pa, pb) = (pos(a.0, a.1), pos(b.0, b.1));
        let k = match (at(a.0, a.1), at(b.0, b.1)) {
            (Some(va), Some(vb)) => ((t - va) / (vb - va)).clamp(0.0, 1.0),
            (None, _) => 1.0,
            (_, None) => 0.0,
        };
        (pa.0 + (pb.0 - pa.0) * k, pa.1 + (pb.1 - pa.1) * k)
    };
    // Segments between edges, an edge being its first corner and whether
    // it runs along x.
    type EdgeKey = (isize, isize, bool);
    let mut points: std::collections::HashMap<EdgeKey, (f32, f32)> = std::collections::HashMap::new();
    let mut segments: Vec<(EdgeKey, EdgeKey)> = Vec::new();
    for j in -1..ny as isize {
        for i in -1..nx as isize {
            let inside = |i: isize, j: isize| at(i, j).is_some_and(|v| v > t);
            let case = (inside(i, j) as u8)
                | (inside(i + 1, j) as u8) << 1
                | (inside(i + 1, j + 1) as u8) << 2
                | (inside(i, j + 1) as u8) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let top = (i, j, true);
            let right = (i + 1, j, false);
            let bottom = (i, j + 1, true);
            let left = (i, j, false);
            // Saddles: the middle of the cell (the mean of its corners,
            // as bilinear sampling has it) says whether the two inside
            // corners join.
            let joined = || {
                let v = |i, j| at(i, j).unwrap_or(0.0);
                (v(i, j) + v(i + 1, j) + v(i + 1, j + 1) + v(i, j + 1)) / 4.0 > t
            };
            let pairs: &[(EdgeKey, EdgeKey)] = match case {
                1 | 14 => &[(left, top)],
                2 | 13 => &[(top, right)],
                3 | 12 => &[(left, right)],
                4 | 11 => &[(right, bottom)],
                5 if joined() => &[(top, right), (bottom, left)],
                5 => &[(left, top), (right, bottom)],
                6 | 9 => &[(top, bottom)],
                7 | 8 => &[(left, bottom)],
                10 if joined() => &[(left, top), (right, bottom)],
                10 => &[(top, right), (bottom, left)],
                _ => &[],
            };
            for &(a, b) in pairs {
                for e in [a, b] {
                    points.entry(e).or_insert_with(|| {
                        let (ci, cj, along_x) = e;
                        crossing((ci, cj), if along_x { (ci + 1, cj) } else { (ci, cj + 1) })
                    });
                }
                segments.push((a, b));
            }
        }
    }
    // Every edge the outline crosses joins exactly two segments: walk
    // them into loops.
    let mut ends: std::collections::HashMap<EdgeKey, Vec<usize>> = std::collections::HashMap::new();
    for (k, &(a, b)) in segments.iter().enumerate() {
        ends.entry(a).or_default().push(k);
        ends.entry(b).or_default().push(k);
    }
    let mut used = vec![false; segments.len()];
    let mut loops = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (first, mut at_edge) = segments[start];
        let mut ring = vec![points[&first]];
        while at_edge != first {
            ring.push(points[&at_edge]);
            let Some(&next) = ends[&at_edge].iter().find(|&&k| !used[k]) else { break };
            used[next] = true;
            let (a, b) = segments[next];
            at_edge = if a == at_edge { b } else { a };
        }
        if ring.len() > 2 {
            loops.push(ring);
        }
    }
    loops
}

/// Closed loops around the cells of a `w` × `h` grid for which `inside` is
/// true, as corner coordinates (cell `(i, j)` spans `i..i + 1`, `j..j + 1`),
/// with the inside on the right of the way they run (clockwise on screen)
/// and only the corners where they turn.
pub fn trace_cells(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool) -> Vec<Vec<(i32, i32)>> {
    let is = |i: i64, j: i64| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && inside(i as usize, j as usize);
    // Boundary edges: start, direction.
    let mut edges: Vec<((i32, i32), (i32, i32))> = Vec::new();
    for j in 0..h as i64 {
        for i in 0..w as i64 {
            if !is(i, j) {
                continue;
            }
            let (x, z) = (i as i32, j as i32);
            if !is(i, j - 1) {
                edges.push(((x, z), (1, 0)));
            }
            if !is(i + 1, j) {
                edges.push(((x + 1, z), (0, 1)));
            }
            if !is(i, j + 1) {
                edges.push(((x + 1, z + 1), (-1, 0)));
            }
            if !is(i - 1, j) {
                edges.push(((x, z + 1), (0, -1)));
            }
        }
    }
    let mut from: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (k, e) in edges.iter().enumerate() {
        from.entry(e.0).or_default().push(k);
    }
    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        let mut ring: Vec<(i32, i32)> = Vec::new();
        let mut k = start;
        loop {
            used[k] = true;
            let ((x, z), d) = edges[k];
            ring.push((x, z));
            let end = (x + d.0, z + d.1);
            // Where two cells touch by a corner, turn right first so that
            // each loop goes round one piece.
            let right = (-d.1, d.0);
            let left = (d.1, -d.0);
            let Some(next) = [right, d, left]
                .iter()
                .find_map(|want| from.get(&end)?.iter().copied().find(|&n| !used[n] && edges[n].1 == *want))
            else {
                break;
            };
            k = next;
        }
        if ring.len() > 2 {
            loops.push(simplify_corners(ring));
        }
    }
    loops
}

/// Drop the corners of a closed path of axis-aligned steps where it goes
/// straight on.
fn simplify_corners(ring: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let n = ring.len();
    let dir = |a: (i32, i32), b: (i32, i32)| ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    (0..n)
        .filter(|&k| {
            let (prev, here, next) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
            dir(prev, here) != dir(here, next)
        })
        .map(|k| ring[k])
        .collect()
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

    /// The outline of a custom brush, as closed loops in blocks from its
    /// centre, turned with the brush. `None` for the symmetric brushes,
    /// whose outline is their circle or square.
    pub fn outline(&self) -> Option<Vec<Vec<(f32, f32)>>> {
        let mask = self.mask.as_ref()?;
        Some(self.place_outline(&mask.outline))
    }

    /// Loops in the image's units (as [`BrushMask::outline`] or
    /// [`BrushMask::outline_coarse`] has them) placed as the brush lies:
    /// offsets in blocks from its centre, turned by its rotation.
    pub fn place_outline(&self, rings: &[Vec<(f32, f32)>]) -> Vec<Vec<(f32, f32)>> {
        let d = 2.0 * self.radius.max(0.5);
        let (s, c) = self.rotation.to_radians().sin_cos();
        let place = |&(u, v): &(f32, f32)| {
            let (x, z) = ((u - 0.5) * d, (v - 0.5) * d);
            (x * c - z * s, x * s + z * c)
        };
        rings.iter().map(|ring| ring.iter().map(place).collect()).collect()
    }

    /// Whether the brush changes the column at offset `(dx, dz)` from its
    /// centre: inside the circle or the square, or where the image paints.
    #[inline]
    pub fn covers(&self, dx: f32, dz: f32) -> bool {
        let r = self.radius.max(0.5);
        if let Some(mask) = &self.mask {
            let (dx, dz) = self.unrotate(dx, dz);
            return mask.sample(dx / (2.0 * r) + 0.5, dz / (2.0 * r) + 0.5) > OUTLINE_THRESHOLD;
        }
        match self.shape {
            BrushShape::Circle => dx * dx + dz * dz <= r * r,
            BrushShape::Square => {
                let (dx, dz) = self.unrotate(dx, dz);
                dx.abs().max(dz.abs()) <= r
            }
        }
    }

    /// Half the side of the square, centred on the brush, that holds its
    /// footprint.
    pub fn reach(&self) -> f32 {
        let r = self.radius.max(0.5);
        if self.is_square() {
            let (s, c) = self.rotation.to_radians().sin_cos();
            r * (s.abs() + c.abs())
        } else {
            r
        }
    }

    /// The exact footprint of the brush centred on `(cx, cz)`: closed loops
    /// of block corners around every column it changes (a column is changed
    /// when the brush covers its centre).
    pub fn footprint_outline(&self, cx: f32, cz: f32) -> Vec<Vec<(i32, i32)>> {
        let reach = self.reach() + 1.0;
        let (x0, z0) = ((cx - reach).floor() as i32, (cz - reach).floor() as i32);
        let (x1, z1) = ((cx + reach).ceil() as i32, (cz + reach).ceil() as i32);
        let (w, h) = ((x1 - x0).max(0) as usize, (z1 - z0).max(0) as usize);
        let cells: Vec<bool> = (0..w * h)
            .map(|c| {
                let (i, j) = ((c % w) as i32, (c / w) as i32);
                self.covers((x0 + i) as f32 + 0.5 - cx, (z0 + j) as f32 + 0.5 - cz)
            })
            .collect();
        trace_cells(w, h, |i, j| cells[j * w + i])
            .into_iter()
            .map(|ring| ring.into_iter().map(|(i, j)| (x0 + i, z0 + j)).collect())
            .collect()
    }

    /// Whether the footprint is a (possibly rotated) square rather than a
    /// circle.
    pub fn is_square(&self) -> bool {
        self.mask.is_some() || self.shape == BrushShape::Square
    }

    /// The columns `x0..x1` of row `z` that [`Brush::weight`] may give some
    /// strength, the brush centred on `(cx, cz)`: all of them but, for a
    /// circle, those well outside it, which weigh nothing.
    ///
    /// The span is wide enough for the weight of every column left out to
    /// be exactly 0 as [`Brush::weight`] works it out: worked out in f64
    /// from the very `dz` the weight is given, with a block and a ten
    /// thousandth of the radius to spare, beyond any rounding of `f32`.
    #[inline]
    pub fn row_span(&self, cx: f32, cz: f32, z: i32, x0: i32, x1: i32) -> std::ops::Range<i32> {
        if self.mask.is_some() || self.shape != BrushShape::Circle {
            return x0..x1;
        }
        let r = self.radius.max(0.5) as f64;
        let dz = (z as f32 + 0.5 - cz) as f64;
        let rem = r * r * 1.0001 - dz * dz;
        if rem < 0.0 {
            return x0..x0;
        }
        // Column x is `x + 0.5 - cx` from the centre.
        let (c, half) = (cx as f64 - 0.5, rem.sqrt() + 1.0);
        let (lo, hi) = ((c - half).floor().max(x0 as f64) as i32, ((c + half).ceil() + 1.0).min(x1 as f64) as i32);
        lo..hi.max(lo)
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

/// Steps of the profile table of a [`Weigher`].
const PROFILE_STEPS: usize = 1024;

/// [`Brush::weight`] made quick for the millions of points of a 3D dab: the
/// profiles a cosine makes (Sine, Plateau) are read from a table, within a
/// few millionths of them, instead of worked out at every point; the others
/// cost less than the table and are worked out as they are.
pub struct Weigher<'a> {
    brush: &'a Brush,
    /// The profile at `PROFILE_STEPS + 1` distances from the centre to the
    /// rim.
    table: Option<Box<[f32]>>,
    /// A round brush without an image: weighed by its distance alone.
    round: bool,
    inv_radius: f32,
}

impl Brush {
    pub fn weigher(&self) -> Weigher<'_> {
        let table = matches!(self.profile, Profile::Sine | Profile::Plateau)
            .then(|| (0..=PROFILE_STEPS).map(|k| self.profile.apply(k as f32 / PROFILE_STEPS as f32).clamp(0.0, 1.0)).collect());
        Weigher {
            brush: self,
            table,
            round: self.mask.is_none() && self.shape == BrushShape::Circle,
            inv_radius: 1.0 / self.radius.max(0.5),
        }
    }
}

impl Weigher<'_> {
    /// The brush's profile at `d`, the distance from its centre over its
    /// radius (`0..=1`).
    #[inline]
    pub fn profile(&self, d: f32) -> f32 {
        match &self.table {
            Some(table) => {
                let f = d.clamp(0.0, 1.0) * PROFILE_STEPS as f32;
                let i = (f as usize).min(PROFILE_STEPS - 1);
                let a = table[i];
                a + (table[i + 1] - a) * (f - i as f32)
            }
            None => self.brush.profile.apply(d).clamp(0.0, 1.0),
        }
    }

    /// As [`Brush::weight`].
    #[inline]
    pub fn weight(&self, dx: f32, dz: f32) -> f32 {
        if !self.round {
            return self.brush.weight(dx, dz);
        }
        let d = (dx * dx + dz * dz).sqrt() * self.inv_radius;
        if d > 1.0 {
            return 0.0;
        }
        self.profile(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every column a row's span leaves out weighs exactly nothing, from
    /// the smallest brush to the largest, near the origin and far out.
    #[test]
    fn row_spans_leave_out_only_what_weighs_nothing() {
        let mut seed = 7u64;
        let mut next = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let mut kept = 0usize;
        for case in 0..400 {
            let radius = [0.3, 0.5, 1.0, 2.5, 7.3, 40.0, 333.3, MAX_RADIUS][case % 8] * (0.5 + next());
            let far = [0.0, 1000.0, 120_000.0, -3_000_000.0][case % 4];
            let (cx, cz) = (far + (next() - 0.5) * 50.0, -far + (next() - 0.5) * 50.0);
            let brush = Brush::symmetric(BrushShape::Circle, Profile::ALL[case % 6], radius);
            let reach = brush.reach().ceil() as i32 + 3;
            let (x0, x1) = (cx.floor() as i32 - reach, cx.floor() as i32 + reach);
            // Rows near the rim, where rounding matters, and some others.
            let rows: Vec<i32> = (0..40).map(|_| cz.floor() as i32 + ((next() * 2.0 - 1.0) * reach as f32) as i32).chain([cz.floor() as i32 - reach + 3, cz.floor() as i32 + reach - 3]).collect();
            for z in rows {
                let span = brush.row_span(cx, cz, z, x0, x1);
                kept += span.len();
                for x in (x0..x1).filter(|x| !span.contains(x)) {
                    let w = brush.weight(x as f32 + 0.5 - cx, z as f32 + 0.5 - cz);
                    assert_eq!(w, 0.0, "r {radius} at ({cx}, {cz}): ({x}, {z}) left out with weight {w}");
                }
            }
        }
        assert!(kept > 0);
    }

    /// The weigher gives the brush's weights.
    #[test]
    fn a_weigher_weighs_as_the_brush() {
        for profile in Profile::ALL {
            for shape in [BrushShape::Circle, BrushShape::Square] {
                let brush = Brush::symmetric(shape, profile, 37.0);
                let w = brush.weigher();
                for k in 0..4000 {
                    let (dx, dz) = ((k % 89) as f32 * 0.93 - 40.0, (k / 89) as f32 * 1.71 - 38.0);
                    let (a, b) = (brush.weight(dx, dz), w.weight(dx, dz));
                    assert!((a - b).abs() < 1e-5, "{profile:?} {shape:?} at {dx} {dz}: {a} and {b}");
                }
            }
        }
    }

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
    fn a_speckled_image_is_outlined_in_closed_loops() {
        // Specks of one to three pixels, soft-edged, over a 256 × 256 image.
        let n = 256;
        let mut data = vec![0u8; n * n];
        let mut h = 12345u32;
        for _ in 0..1500 {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            let (x, y, r) = ((h % n as u32) as usize, ((h >> 8) % n as u32) as usize, (h >> 24) % 3);
            for dy in 0..=r as usize {
                for dx in 0..=r as usize {
                    if x + dx < n && y + dy < n {
                        data[(y + dy) * n + x + dx] = if dx == r as usize || dy == r as usize { 140 } else { 255 };
                    }
                }
            }
        }
        let mask = BrushMask::new("Specks", n, n, data);
        // Every loop closes on itself in steps of a pixel at most: no long
        // segment joins pieces apart.
        let check = |rings: &[Vec<(f32, f32)>], cell: f32| {
            for ring in rings {
                for k in 0..ring.len() {
                    let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                    let d = (a.0 - b.0).hypot(a.1 - b.1);
                    assert!(d <= 1.5 * cell, "a step of {d} in a loop of {} points", ring.len());
                }
            }
        };
        check(&mask.outline, 1.0 / (n - 1) as f32);
        // On squares of 8 pixels, the specks close together merge: far
        // fewer loops, still closed, and every pixel that paints inside one.
        let coarse = mask.outline_coarse(8);
        check(&coarse, 8.0 / (n - 1) as f32);
        assert!(coarse.len() * 4 < mask.outline.len(), "{} loops for {}", coarse.len(), mask.outline.len());
        let inside = |p: (f32, f32)| {
            coarse
                .iter()
                .filter(|ring| {
                    let mut hit = false;
                    for k in 0..ring.len() {
                        let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
                        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                            hit = !hit;
                        }
                    }
                    hit
                })
                .count()
                % 2
                == 1
        };
        // (Along the image's sides the outline runs through the middles of
        // the outermost squares, as the full one runs through the outermost
        // pixels' centres: the half square outside them is left out.)
        for (k, &v) in mask.data.iter().enumerate() {
            let (i, j) = (k % n, k / n);
            if v > 200 && (4..n - 4).contains(&i) && (4..n - 4).contains(&j) {
                let p = ((k % n) as f32 / (n - 1) as f32, (k / n) as f32 / (n - 1) as f32);
                assert!(inside(p), "the pixel at {p:?} paints outside the coarse outline");
            }
        }
        assert_eq!(mask.outline_coarse(1), mask.outline);
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
        // One closed loop, through the pixels at the disc's edge.
        assert_eq!(mask.outline.len(), 1);
        assert!(mask.outline[0].len() > 100);
        for &(x, y) in &mask.outline[0] {
            let r = (x - 0.5).hypot(y - 0.5);
            assert!((r - 0.4).abs() < 1.5 / n as f32, "point at {r}");
        }
        let b = Brush {
            radius: 50.0,
            mask: Some(Arc::new(mask)),
            ..Brush::default()
        };
        let outline = b.outline().unwrap();
        let (x, z) = outline[0][0];
        assert!((x.hypot(z) - 40.0).abs() < 2.0);
        assert!(Brush::default().outline().is_none());
        // A grey image converted from RGBA: half-transparent white is half strength.
        let m = BrushMask::from_rgba("Grey", 1, 1, &[255, 255, 255, 128]);
        assert_eq!(m.data, vec![128]);
        // An image painting everywhere is outlined along its sides.
        let full = BrushMask::new("Full", 4, 4, vec![255; 16]);
        assert_eq!(full.outline.len(), 1);
        for &(x, y) in &full.outline[0] {
            assert!(x == 0.0 || x == 1.0 || y == 0.0 || y == 1.0, "({x}, {y}) is on a side");
        }
        // Two blobs: two loops.
        let mut two = vec![0u8; 16 * 16];
        for (x, y) in [(3, 3), (4, 3), (3, 4), (11, 11), (12, 12)] {
            two[y * 16 + x] = 255;
        }
        assert_eq!(BrushMask::new("Two", 16, 16, two).outline.len(), 2);
    }

    /// The area enclosed by a loop of corners (shoelace formula).
    fn area(ring: &[(i32, i32)]) -> i64 {
        let n = ring.len();
        (0..n)
            .map(|k| {
                let (a, b) = (ring[k], ring[(k + 1) % n]);
                a.0 as i64 * b.1 as i64 - b.0 as i64 * a.1 as i64
            })
            .sum::<i64>()
            / 2
    }

    #[test]
    fn footprints_outline_exactly_the_blocks_changed() {
        for (shape, rotation) in [(BrushShape::Circle, 0.0), (BrushShape::Square, 0.0), (BrushShape::Square, 30.0)] {
            for (cx, cz) in [(0.0, 0.0), (10.5, -3.25), (-7.9, 4.6)] {
                let mut b = Brush::symmetric(shape, Profile::Linear, 6.0);
                b.rotation = rotation;
                let loops = b.footprint_outline(cx, cz);
                // The loops enclose as many blocks as the brush changes.
                let mut changed = 0;
                for z in -20..20 {
                    for x in -20..20 {
                        changed += b.covers(x as f32 + 0.5 - cx, z as f32 + 0.5 - cz) as i64;
                    }
                }
                let enclosed: i64 = loops.iter().map(|l| area(l)).sum();
                assert_eq!(enclosed, changed, "{shape:?} {rotation}° at ({cx}, {cz})");
                // Only corners where the outline turns, on whole blocks.
                for l in &loops {
                    let n = l.len();
                    for k in 0..n {
                        let (a, b, c) = (l[(k + n - 1) % n], l[k], l[(k + 1) % n]);
                        let d1 = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
                        let d2 = ((c.0 - b.0).signum(), (c.1 - b.1).signum());
                        assert_ne!(d1, d2);
                        assert!(d1.0 == 0 || d1.1 == 0, "steps are along the grid");
                    }
                }
            }
        }
        // Two cells touching by a corner make two loops.
        let loops = trace_cells(2, 2, |i, j| i == j);
        assert_eq!(loops.len(), 2);
        assert!(loops.iter().all(|l| l.len() == 4 && area(l) == 1));
        // A ring makes an outer loop and a hole running the other way.
        let ring = trace_cells(3, 3, |i, j| (i, j) != (1, 1));
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.iter().map(|l| area(l)).sum::<i64>(), 8);
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
