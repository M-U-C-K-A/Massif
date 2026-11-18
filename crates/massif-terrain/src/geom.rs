//! Shapes drawn on the terrain: curves through control points, and where a
//! sample lies from them.
//!
//! A curve's control points (fractions of the terrain, each with the
//! values its setting's fields name) become a smooth line through them — a
//! centripetal Catmull-Rom spline, which never loops on itself between close
//! points — cut into short segments. A kernel asks, for each sample, how
//! far the line is, how far along it the nearest point lies, on which side
//! the sample is, and the values the control points give there.

use crate::params::Point;

/// The point of a curve nearest a sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Near {
    /// Distance to the line, in blocks.
    pub dist: f64,
    /// How far along the line, in blocks from its start.
    pub along: f64,
    /// +1 on the left of the way the line runs, −1 on its right.
    pub side: f64,
    /// The direction the line runs there (a unit vector).
    pub dir: [f64; 2],
    /// The control points' values, interpolated along the line.
    pub values: Vec<f64>,
}

/// A smooth line through control points, in blocks.
#[derive(Debug, Clone)]
pub struct Curve {
    /// The line's vertices, in blocks from the terrain's corner.
    pts: Vec<[f64; 2]>,
    /// The values at each vertex.
    vals: Vec<Vec<f64>>,
    /// Arc length at each vertex.
    s: Vec<f64>,
    closed: bool,
}

/// A segment of a curve, kept by a kernel for the samples of one tile.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// Its index in the curve.
    pub k: usize,
}

fn catmull(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    // Centripetal parametrisation (alpha = 0.5).
    let knot = |a: [f64; 2], b: [f64; 2]| ((b[0] - a[0]).hypot(b[1] - a[1])).sqrt().max(1e-6);
    let (t0, t1) = (0.0, knot(p0, p1));
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let tt = t1 + (t2 - t1) * t;
    let lerp = |a: [f64; 2], b: [f64; 2], ta: f64, tb: f64| {
        let w = (tt - ta) / (tb - ta);
        [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
    };
    let a1 = lerp(p0, p1, t0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, t0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

impl Curve {
    /// The line through `points` on a terrain `size` blocks a side, cut
    /// into segments of about `step` blocks; `None` with fewer than two
    /// points.
    pub fn new(points: &[Point], size: f64, closed: bool, step: f64) -> Option<Self> {
        let n = points.len();
        if n < 2 {
            return None;
        }
        let ctrl: Vec<[f64; 2]> = points.iter().map(|p| [p.x * size, p.z * size]).collect();
        let at = |i: isize| -> [f64; 2] {
            if closed {
                ctrl[i.rem_euclid(n as isize) as usize]
            } else if i < 0 {
                // Mirrored ends: the line starts and ends straight.
                let (a, b) = (ctrl[0], ctrl[1]);
                [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]]
            } else if i as usize >= n {
                let (a, b) = (ctrl[n - 1], ctrl[n - 2]);
                [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]]
            } else {
                ctrl[i as usize]
            }
        };
        let value = |i: usize| -> &[f64] { &points[i % n].values };
        let spans = if closed { n } else { n - 1 };
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let mut vals: Vec<Vec<f64>> = Vec::new();
        for i in 0..spans {
            let (p1, p2) = (at(i as isize), at(i as isize + 1));
            let len = (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
            let pieces = ((len / step.max(0.5)).ceil() as usize).clamp(1, 4096);
            let (v1, v2) = (value(i), value(i + 1));
            for k in 0..pieces {
                let t = k as f64 / pieces as f64;
                pts.push(catmull(at(i as isize - 1), p1, p2, at(i as isize + 2), t));
                vals.push(v1.iter().zip(v2).map(|(a, b)| a + (b - a) * t).collect());
            }
        }
        if closed {
            pts.push(pts[0]);
            vals.push(vals[0].clone());
        } else {
            pts.push(ctrl[n - 1]);
            vals.push(points[n - 1].values.clone());
        }
        let mut s = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        for (k, p) in pts.iter().enumerate() {
            if k > 0 {
                total += (p[0] - pts[k - 1][0]).hypot(p[1] - pts[k - 1][1]);
            }
            s.push(total);
        }
        Some(Self { pts, vals, s, closed })
    }

    /// Length, in blocks.
    pub fn length(&self) -> f64 {
        *self.s.last().unwrap_or(&0.0)
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    /// The segments that can come within `reach` of the rectangle
    /// `[x0, x1] × [z0, z1]` (blocks).
    pub fn segments_near(&self, x0: f64, z0: f64, x1: f64, z1: f64, reach: f64) -> Vec<Segment> {
        let (x0, z0, x1, z1) = (x0 - reach, z0 - reach, x1 + reach, z1 + reach);
        self.pts
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0][0].max(w[1][0]) >= x0 && w[0][0].min(w[1][0]) <= x1 && w[0][1].max(w[1][1]) >= z0 && w[0][1].min(w[1][1]) <= z1)
            .map(|(k, w)| Segment { a: w[0], b: w[1], k })
            .collect()
    }

    /// For each cell of a grid over `[x0, x0 + nx·cell] × [z0, z0 + nz·cell]`
    /// (row by row), the segments that can hold the nearest point of a
    /// sample in it within `reach`, or come within `slack` blocks of it:
    /// what [`Curve::nearest_soft`] needs, and much less than all of them.
    #[allow(clippy::too_many_arguments)]
    pub fn cells(&self, segments: &[Segment], x0: f64, z0: f64, cell: f64, nx: usize, nz: usize, reach: f64, slack: f64) -> Vec<Vec<Segment>> {
        let half = cell * std::f64::consts::FRAC_1_SQRT_2;
        let mut out = Vec::with_capacity(nx * nz);
        let mut dists = Vec::with_capacity(segments.len());
        for j in 0..nz {
            for i in 0..nx {
                let (cx, cz) = (x0 + (i as f64 + 0.5) * cell, z0 + (j as f64 + 0.5) * cell);
                dists.clear();
                let mut upper = f64::MAX;
                for sg in segments {
                    let (dx, dz) = (sg.b[0] - sg.a[0], sg.b[1] - sg.a[1]);
                    let l2 = dx * dx + dz * dz;
                    let t = if l2 > 0.0 { (((cx - sg.a[0]) * dx + (cz - sg.a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    let d = (cx - sg.a[0] - dx * t).hypot(cz - sg.a[1] - dz * t);
                    upper = upper.min(d + half);
                    dists.push(d);
                }
                let keep = (upper + slack).min(reach + half);
                out.push(segments.iter().zip(&dists).filter(|(_, d)| **d - half <= keep).map(|(sg, _)| *sg).collect());
            }
        }
        out
    }

    /// The nearest point of the line among `segments`, if one lies within
    /// `reach` blocks of `(x, z)`.
    pub fn nearest(&self, segments: &[Segment], x: f64, z: f64, reach: f64) -> Option<Near> {
        let mut best: Option<(f64, usize, f64)> = None;
        for sg in segments {
            let (dx, dz) = (sg.b[0] - sg.a[0], sg.b[1] - sg.a[1]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 { (((x - sg.a[0]) * dx + (z - sg.a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let (px, pz) = (sg.a[0] + dx * t, sg.a[1] + dz * t);
            let d2 = (x - px) * (x - px) + (z - pz) * (z - pz);
            if d2 <= reach * reach && best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, sg.k, t));
            }
        }
        let (d2, k, t) = best?;
        let (a, b) = (self.pts[k], self.pts[k + 1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz).max(1e-9);
        let dir = [dx / l, dz / l];
        let cross = dir[0] * (z - a[1]) - dir[1] * (x - a[0]);
        let values = self.vals[k].iter().zip(&self.vals[k + 1]).map(|(u, v)| u + (v - u) * t).collect();
        Some(Near { dist: d2.sqrt(), along: self.s[k] + (self.s[k + 1] - self.s[k]) * t, side: if cross >= 0.0 { 1.0 } else { -1.0 }, dir, values })
    }

    /// The point `along` blocks from the line's start, and the way the line
    /// runs there.
    pub fn point_at(&self, along: f64) -> (f64, f64, [f64; 2]) {
        let along = along.clamp(0.0, self.length());
        let k = self.s.partition_point(|&s| s <= along).clamp(1, self.pts.len() - 1) - 1;
        let (a, b) = (self.pts[k], self.pts[k + 1]);
        let span = (self.s[k + 1] - self.s[k]).max(1e-9);
        let t = (along - self.s[k]) / span;
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dz).max(1e-9);
        (a[0] + dx * t, a[1] + dz * t, [dx / l, dz / l])
    }

    /// [`Curve::nearest`], with what it gives along the line (how far
    /// along, the direction, the values) blended over the segments about as
    /// near: where the line bends, the nearest point jumps from one stretch
    /// to the other, and this keeps what follows it from jumping too. The
    /// distance stays the true one. `soft` is in blocks.
    pub fn nearest_soft(&self, segments: &[Segment], x: f64, z: f64, reach: f64, soft: f64) -> Option<Near> {
        let project = |sg: &Segment| {
            let (dx, dz) = (sg.b[0] - sg.a[0], sg.b[1] - sg.a[1]);
            let l2 = dx * dx + dz * dz;
            let t = if l2 > 0.0 { (((x - sg.a[0]) * dx + (z - sg.a[1]) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
            ((x - sg.a[0] - dx * t).hypot(z - sg.a[1] - dz * t), t)
        };
        let best = segments.iter().map(|sg| project(sg).0).fold(f64::MAX, f64::min);
        if best > reach {
            return None;
        }
        let soft = soft.max(1e-3);
        let (mut wsum, mut along, mut dir, mut cross) = (0.0, 0.0, [0.0, 0.0], 0.0);
        let mut values = vec![0.0; self.vals[0].len()];
        for sg in segments {
            let (d, t) = project(sg);
            let k = sg.k;
            let excess = (d - best) / soft;
            if excess > 4.0 {
                continue;
            }
            let w = (-excess).exp();
            let (a, b) = (self.pts[k], self.pts[k + 1]);
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dz).max(1e-9);
            wsum += w;
            along += w * (self.s[k] + (self.s[k + 1] - self.s[k]) * t);
            dir[0] += w * dx / l;
            dir[1] += w * dz / l;
            cross += w * ((dx / l) * (z - a[1]) - (dz / l) * (x - a[0]));
            for (v, (u, n)) in values.iter_mut().zip(self.vals[k].iter().zip(&self.vals[k + 1])) {
                *v += w * (u + (n - u) * t);
            }
        }
        let l = dir[0].hypot(dir[1]).max(1e-9);
        values.iter_mut().for_each(|v| *v /= wsum);
        Some(Near { dist: best, along: along / wsum, side: if cross >= 0.0 { 1.0 } else { -1.0 }, dir: [dir[0] / l, dir[1] / l], values })
    }

    /// Whether `(x, z)` lies inside the closed line (by winding).
    pub fn inside(&self, x: f64, z: f64) -> bool {
        let mut winding = 0i32;
        for w in self.pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let is_left = (b[0] - a[0]) * (z - a[1]) - (x - a[0]) * (b[1] - a[1]);
            if a[1] <= z {
                if b[1] > z && is_left > 0.0 {
                    winding += 1;
                }
            } else if b[1] <= z && is_left < 0.0 {
                winding -= 1;
            }
        }
        winding != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, z: f64, v: f64) -> Point {
        Point { x, z, values: vec![v] }
    }

    #[test]
    fn a_curve_passes_through_its_points() {
        let c = Curve::new(&[pt(0.1, 0.5, 1.0), pt(0.5, 0.2, 2.0), pt(0.9, 0.5, 3.0)], 1000.0, false, 4.0).unwrap();
        let segs = c.segments_near(0.0, 0.0, 1000.0, 1000.0, 0.0);
        for (x, z, v) in [(100.0, 500.0, 1.0), (500.0, 200.0, 2.0), (900.0, 500.0, 3.0)] {
            let near = c.nearest(&segs, x, z, 10.0).unwrap();
            assert!(near.dist < 0.5, "{near:?}");
            assert!((near.values[0] - v).abs() < 0.05);
        }
        // Smooth: no sample far from both control polygon and line.
        assert!(c.length() > 800.0 && c.length() < 1100.0, "{}", c.length());
        // Side: south of the line's west end, the line runs east.
        let n = c.nearest(&segs, 100.0, 520.0, 100.0).unwrap();
        assert!(n.side != c.nearest(&segs, 100.0, 480.0, 100.0).unwrap().side);
        assert!(c.nearest(&segs, 500.0, 900.0, 100.0).is_none());
    }

    #[test]
    fn a_closed_curve_has_an_inside() {
        let c = Curve::new(&[pt(0.2, 0.2, 0.0), pt(0.8, 0.2, 0.0), pt(0.8, 0.8, 0.0), pt(0.2, 0.8, 0.0)], 100.0, true, 1.0).unwrap();
        assert!(c.inside(50.0, 50.0));
        assert!(!c.inside(5.0, 50.0) && !c.inside(95.0, 95.0));
    }
}
