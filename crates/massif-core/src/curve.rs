//! A curve through points placed on the map: the line of an annotation
//! drawn as a curve, and, later, the course a River or a Road follows.
//!
//! Its points are in blocks (x, z). Between them it runs as a Catmull-Rom
//! spline (through every point), as Bézier segments (each point with a
//! tangent handle, mirrored on either side), or straight (a polyline).

use serde::{Deserialize, Serialize};

/// How a curve runs from point to point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Interp {
    /// Smoothly through every point.
    #[default]
    CatmullRom,
    /// Through every point, bent by each point's handle.
    Bezier,
    /// Straight from point to point.
    Polyline,
}

/// A point of a curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub at: [f32; 2],
    /// The handle of a Bézier curve: where the curve leaves the point
    /// toward, from it (it comes in from the opposite side). `None`: as
    /// the Catmull-Rom spline through the points would leave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<[f32; 2]>,
}

impl CurvePoint {
    pub fn new(at: [f32; 2]) -> Self {
        Self { at, handle: None }
    }
}

/// A curve on the map.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Curve {
    pub points: Vec<CurvePoint>,
    pub interp: Interp,
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn mul(a: [f32; 2], k: f32) -> [f32; 2] {
    [a[0] * k, a[1] * k]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

impl Curve {
    pub fn new(interp: Interp) -> Self {
        Self { points: Vec::new(), interp }
    }

    /// The handle the Catmull-Rom spline gives point `i`: a third of its
    /// tangent, half the way from the point before to the point after (at
    /// an end, from the point itself).
    pub fn auto_handle(&self, i: usize) -> [f32; 2] {
        let n = self.points.len();
        if n < 2 {
            return [0.0, 0.0];
        }
        let p = |k: usize| self.points[k].at;
        let (a, b) = (p(i.saturating_sub(1)), p((i + 1).min(n - 1)));
        let span = if i == 0 || i == n - 1 { 1.0 } else { 2.0 };
        mul(sub(b, a), 1.0 / (3.0 * span))
    }

    /// The handle of point `i` as the curve is drawn.
    pub fn handle(&self, i: usize) -> [f32; 2] {
        match self.interp {
            Interp::Bezier => self.points[i].handle.unwrap_or_else(|| self.auto_handle(i)),
            Interp::CatmullRom => self.auto_handle(i),
            Interp::Polyline => [0.0, 0.0],
        }
    }

    /// The point at `t` (0 to 1) along segment `i`, from point `i` to
    /// point `i + 1`.
    pub fn at(&self, i: usize, t: f32) -> [f32; 2] {
        let (a, d) = (self.points[i].at, self.points[i + 1].at);
        if self.interp == Interp::Polyline {
            return add(a, mul(sub(d, a), t));
        }
        let b = add(a, self.handle(i));
        let c = sub(d, self.handle(i + 1));
        let u = 1.0 - t;
        let (k0, k1, k2, k3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        [a[0] * k0 + b[0] * k1 + c[0] * k2 + d[0] * k3, a[1] * k0 + b[1] * k1 + c[1] * k2 + d[1] * k3]
    }

    /// Points along the curve at most `step` blocks apart, each with how
    /// far along the curve it is (0 at the first point, 1 at the last).
    pub fn sample(&self, step: f32) -> Vec<([f32; 2], f32)> {
        let step = step.max(0.05);
        let mut out: Vec<[f32; 2]> = Vec::new();
        match self.points.len() {
            0 => return Vec::new(),
            1 => return vec![(self.points[0].at, 0.0)],
            _ => {}
        }
        out.push(self.points[0].at);
        for i in 0..self.points.len() - 1 {
            // The fastest the curve runs along a step of its parameter: a
            // cubic never runs faster than three times its longest
            // control leg, so steps of that never leave a gap over `step`.
            let speed = if self.interp == Interp::Polyline {
                dist(self.points[i].at, self.points[i + 1].at)
            } else {
                let (a, d) = (self.points[i].at, self.points[i + 1].at);
                let (b, c) = (add(a, self.handle(i)), sub(d, self.handle(i + 1)));
                3.0 * dist(a, b).max(dist(b, c)).max(dist(c, d))
            };
            let n = ((speed / step).ceil() as usize).clamp(1, 100_000);
            for k in 1..=n {
                out.push(self.at(i, k as f32 / n as f32));
            }
        }
        let mut along = Vec::with_capacity(out.len());
        let mut total = 0.0;
        along.push(0.0);
        for w in out.windows(2) {
            total += dist(w[0], w[1]);
            along.push(total);
        }
        out.into_iter().zip(along).map(|(p, l)| (p, if total > 0.0 { l / total } else { 0.0 })).collect()
    }

    /// The length of the curve, in blocks.
    pub fn length(&self) -> f32 {
        let s = self.sample(0.5);
        s.windows(2).map(|w| dist(w[0].0, w[1].0)).sum()
    }

    /// The middle line of a ribbon along the curve, `width` blocks wide at
    /// its first point and `end` at its last, widening evenly along it:
    /// points at most half a block apart, each with its width.
    pub fn ribbon(&self, width: f32, end: f32) -> Vec<([f32; 2], f32)> {
        self.sample(0.5).into_iter().map(|(p, f)| (p, width + (end - width) * f)).collect()
    }

    /// Where on the curve is nearest `p`: the segment, where along it (0
    /// to 1) and how far from `p`.
    pub fn nearest(&self, p: [f32; 2]) -> Option<(usize, f32, f32)> {
        if self.points.len() < 2 {
            return None;
        }
        let mut best: Option<(usize, f32, f32)> = None;
        for i in 0..self.points.len() - 1 {
            const N: usize = 48;
            for k in 0..=N {
                let t = k as f32 / N as f32;
                let d = dist(self.at(i, t), p);
                if best.is_none_or(|b| d < b.2) {
                    best = Some((i, t, d));
                }
            }
        }
        best
    }

    /// Put a point on segment `i` at `t`, where the curve passes; returns
    /// its index. The curve keeps its course as far as it can: the new
    /// point takes no handle of its own.
    pub fn insert(&mut self, i: usize, t: f32) -> usize {
        let at = self.at(i, t);
        self.points.insert(i + 1, CurvePoint::new(at));
        i + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(interp: Interp, pts: &[[f32; 2]]) -> Curve {
        Curve { points: pts.iter().map(|&p| CurvePoint::new(p)).collect(), interp }
    }

    #[test]
    fn every_kind_passes_through_its_points() {
        for interp in [Interp::CatmullRom, Interp::Bezier, Interp::Polyline] {
            let c = curve(interp, &[[0.0, 0.0], [10.0, 5.0], [20.0, -3.0], [30.0, 0.0]]);
            for i in 0..3 {
                assert_eq!(c.at(i, 0.0), c.points[i].at, "{interp:?}");
                let end = c.at(i, 1.0);
                assert!(dist(end, c.points[i + 1].at) < 1e-4, "{interp:?}");
            }
            let s = c.sample(0.5);
            assert!(s.windows(2).all(|w| dist(w[0].0, w[1].0) <= 0.5 + 1e-3), "{interp:?}: no gap over half a block");
            assert_eq!(s.first().unwrap().1, 0.0);
            assert!((s.last().unwrap().1 - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_polyline_is_straight_and_a_spline_bends() {
        let pts = [[0.0, 0.0], [10.0, 10.0], [20.0, 0.0]];
        let line = curve(Interp::Polyline, &pts);
        assert_eq!(line.at(0, 0.5), [5.0, 5.0]);
        let spline = curve(Interp::CatmullRom, &pts);
        assert!(spline.at(0, 0.5)[1] > 5.5, "rounds over the middle point: {:?}", spline.at(0, 0.5));
        assert!(spline.length() > line.length() - 0.5 && spline.length() < line.length() + 3.0);
    }

    #[test]
    fn a_handle_bends_a_bezier_curve() {
        let mut c = curve(Interp::Bezier, &[[0.0, 0.0], [20.0, 0.0]]);
        assert!(c.at(0, 0.5)[1].abs() < 1e-4, "straight without handles");
        c.points[0].handle = Some([0.0, 12.0]);
        assert!(c.at(0, 0.5)[1] > 3.0);
        // Catmull-Rom ignores the handles.
        c.interp = Interp::CatmullRom;
        assert!(c.at(0, 0.5)[1].abs() < 1e-4);
    }

    #[test]
    fn a_point_inserted_keeps_the_course() {
        let mut c = curve(Interp::Polyline, &[[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]]);
        let (i, t, d) = c.nearest([10.0, 1.0]).unwrap();
        assert_eq!(i, 0);
        assert!((t - 0.5).abs() < 0.03 && (d - 1.0).abs() < 0.1);
        let k = c.insert(i, t);
        assert_eq!(k, 1);
        assert_eq!(c.points.len(), 4);
        assert!((c.points[1].at[0] - 10.0).abs() < 0.5);
    }

    #[test]
    fn a_ribbon_widens_from_start_to_end() {
        let c = curve(Interp::Polyline, &[[0.0, 0.0], [40.0, 0.0]]);
        let r = c.ribbon(2.0, 10.0);
        assert_eq!(r.first().unwrap().1, 2.0);
        assert!((r.last().unwrap().1 - 10.0).abs() < 1e-4);
        let mid = r.iter().find(|(p, _)| (p[0] - 20.0).abs() < 0.3).unwrap();
        assert!((mid.1 - 6.0).abs() < 0.2);
    }
}
