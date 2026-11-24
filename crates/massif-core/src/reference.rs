//! Reference images: pictures laid over or under the map to trace over, as
//! WorldPainter's overlay — a plan, a sketch, a real map. They only show:
//! they never change the land and are never exported. Each is placed by its
//! middle, its scale (blocks a pixel) and its rotation, and kept with the
//! map, by the path of its file.

use serde::{Deserialize, Serialize};

/// A picture laid on the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReferenceImage {
    /// Its file.
    pub path: String,
    /// Its size in pixels, as last read (0 until then).
    pub size: [u32; 2],
    /// Where its middle lies, in blocks.
    pub x: f64,
    pub z: f64,
    /// Blocks a pixel of it covers.
    pub scale: f32,
    /// Turned clockwise, in degrees.
    pub rotation: f32,
    /// 0 to 1.
    pub opacity: f32,
    /// Over the shaded relief (true) or under it, the relief then shown
    /// faded over it.
    pub above: bool,
    pub shown: bool,
    /// Locked: its handles do not move it.
    pub locked: bool,
    /// Where its top left corner lies, in blocks, while its size is not
    /// known (a WorldPainter overlay whose file was not found): once its
    /// size is read, its middle is set from there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<[f64; 2]>,
}

impl Default for ReferenceImage {
    fn default() -> Self {
        Self { path: String::new(), size: [0, 0], x: 0.0, z: 0.0, scale: 1.0, rotation: 0.0, opacity: 0.6, above: true, shown: true, locked: false, corner: None }
    }
}

impl ReferenceImage {
    /// A picture placed by its top left corner, at `(x, z)` in blocks, as
    /// WorldPainter places its overlays; `size` in pixels when known.
    pub fn at_corner(path: &str, x: f64, z: f64, scale: f32, size: Option<[u32; 2]>) -> Self {
        let mut r = Self { path: path.to_owned(), scale: scale.max(1e-4), corner: Some([x, z]), ..Self::default() };
        if let Some(size) = size {
            r.set_size(size);
        }
        r
    }

    /// Its size in pixels, as read: one placed by its corner gets its
    /// middle from it. True when it moved.
    pub fn set_size(&mut self, size: [u32; 2]) -> bool {
        self.size = size;
        let Some([x, z]) = self.corner.take() else { return false };
        let s = self.scale.max(1e-4) as f64;
        (self.x, self.z) = (x + size[0] as f64 * s / 2.0, z + size[1] as f64 * s / 2.0);
        true
    }

    /// The name of its file.
    pub fn name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    /// Its half size in blocks.
    fn half(&self) -> (f64, f64) {
        let s = self.scale.max(1e-4) as f64;
        (self.size[0] as f64 * s / 2.0, self.size[1] as f64 * s / 2.0)
    }

    fn axes(&self) -> ((f64, f64), (f64, f64)) {
        let (sin, cos) = (self.rotation as f64).to_radians().sin_cos();
        // Clockwise as seen from above, z going south.
        ((cos, sin), (-sin, cos))
    }

    /// The world point at `(u, v)` of it, each from 0 to 1 (0, 0: its
    /// top left corner).
    pub fn at(&self, u: f64, v: f64) -> (f64, f64) {
        let (hw, hh) = self.half();
        let (a, b) = self.axes();
        let (du, dv) = ((u * 2.0 - 1.0) * hw, (v * 2.0 - 1.0) * hh);
        (self.x + a.0 * du + b.0 * dv, self.z + a.1 * du + b.1 * dv)
    }

    /// Its corners: top left, top right, bottom right, bottom left.
    pub fn corners(&self) -> [(f64, f64); 4] {
        [self.at(0.0, 0.0), self.at(1.0, 0.0), self.at(1.0, 1.0), self.at(0.0, 1.0)]
    }

    /// Where world point `(x, z)` falls on it, `(u, v)` from 0 to 1 inside.
    pub fn uv_of(&self, x: f64, z: f64) -> (f64, f64) {
        let (hw, hh) = self.half();
        let (a, b) = self.axes();
        let (dx, dz) = (x - self.x, z - self.z);
        let du = dx * a.0 + dz * a.1;
        let dv = dx * b.0 + dz * b.1;
        (du / (2.0 * hw.max(1e-9)) + 0.5, dv / (2.0 * hh.max(1e-9)) + 0.5)
    }

    /// Whether world point `(x, z)` is on it.
    pub fn contains(&self, x: f64, z: f64) -> bool {
        let (u, v) = self.uv_of(x, z);
        (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)
    }

    /// Scaled so that its corner at `(u, v)` goes to world point `(x, z)`,
    /// the opposite corner staying where it is: the scale it takes (its
    /// shape kept), and its new middle.
    pub fn scale_to(&mut self, corner: (f64, f64), x: f64, z: f64) {
        let opposite = (1.0 - corner.0, 1.0 - corner.1);
        let fixed = self.at(opposite.0, opposite.1);
        let (a, b) = self.axes();
        let (dx, dz) = (x - fixed.0, z - fixed.1);
        // The diagonal along the image's own axes.
        let du = (dx * a.0 + dz * a.1).abs();
        let dv = (dx * b.0 + dz * b.1).abs();
        let (w, h) = (self.size[0].max(1) as f64, self.size[1].max(1) as f64);
        self.scale = (du / w).max(dv / h).max(1e-3) as f32;
        // The opposite corner back where it was.
        let moved = self.at(opposite.0, opposite.1);
        self.x += fixed.0 - moved.0;
        self.z += fixed.1 - moved.1;
    }

    /// Fit to the rectangle `(x0, z0)`–`(x1, z1)` of the world, unturned:
    /// as large as it fits, in its middle.
    pub fn fit(&mut self, x0: f64, z0: f64, x1: f64, z1: f64) {
        let (w, h) = (self.size[0].max(1) as f64, self.size[1].max(1) as f64);
        self.scale = ((x1 - x0) / w).min((z1 - z0) / h).max(1e-3) as f32;
        self.rotation = 0.0;
        self.x = (x0 + x1) / 2.0;
        self.z = (z0 + z1) / 2.0;
    }
}

/// The size in pixels of the picture at `path`, read from its header.
pub fn image_size(path: &std::path::Path) -> Option<[u32; 2]> {
    image::image_dimensions(path).ok().map(|(w, h)| [w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_image_is_placed_scaled_and_turned_on_the_map() {
        let mut r = ReferenceImage { size: [200, 100], x: 50.0, z: 20.0, scale: 2.0, ..ReferenceImage::default() };
        // 400 × 200 blocks around its middle.
        assert_eq!(r.corners()[0], (-150.0, -80.0));
        assert_eq!(r.corners()[2], (250.0, 120.0));
        assert!(r.contains(0.0, 0.0) && !r.contains(260.0, 0.0));
        let (u, v) = r.uv_of(250.0, 120.0);
        assert!((u - 1.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9);
        // Turned a quarter clockwise: its top edge runs north to south on
        // its east side.
        r.rotation = 90.0;
        let tl = r.corners()[0];
        assert!((tl.0 - 150.0).abs() < 1e-6 && (tl.1 - -180.0).abs() < 1e-6, "{tl:?}");
        let tr = r.corners()[1];
        assert!((tr.0 - 150.0).abs() < 1e-6 && (tr.1 - 220.0).abs() < 1e-6, "{tr:?}");
        // Its bottom right corner dragged out: the top left one stays.
        r.rotation = 0.0;
        let fixed = r.corners()[0];
        r.scale_to((1.0, 1.0), 450.0, 220.0);
        assert!((r.scale - 3.0).abs() < 1e-4, "{}", r.scale);
        let now = r.corners()[0];
        assert!((now.0 - fixed.0).abs() < 1e-6 && (now.1 - fixed.1).abs() < 1e-6);
        // Fit to a map.
        r.fit(0.0, 0.0, 1024.0, 1024.0);
        assert_eq!((r.scale, r.x, r.z), (5.12, 512.0, 512.0));
    }

    #[test]
    fn reference_images_of_older_files_read_with_defaults() {
        let r: ReferenceImage = serde_json::from_str(r#"{"path": "/maps/plan.png", "x": 4.0}"#).unwrap();
        assert_eq!((r.name(), r.x, r.opacity, r.shown, r.above), ("plan.png", 4.0, 0.6, true, true));
    }
}
