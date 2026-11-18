//! Pictures of a relief: heights tinted by altitude and lit by a low sun,
//! for node thumbnails, the viewer's 2D mode and reports. Colours are sRGB
//! and lit as they are: a preview, not a render.

use crate::kernel::Grid;
use crate::space::Space;

/// Tints from the sea floor to the snow, by height above the sea.
const RAMP: [(f32, [f32; 3]); 8] = [
    (-60.0, [0.10, 0.20, 0.38]),
    (-1.0, [0.22, 0.42, 0.62]),
    (0.0, [0.80, 0.76, 0.60]),
    (4.0, [0.40, 0.55, 0.30]),
    (60.0, [0.33, 0.47, 0.25]),
    (120.0, [0.50, 0.45, 0.36]),
    (180.0, [0.56, 0.54, 0.52]),
    (235.0, [0.95, 0.96, 0.98]),
];

fn tint(h: f32) -> [f32; 3] {
    if h <= RAMP[0].0 {
        return RAMP[0].1;
    }
    for w in RAMP.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if h <= b {
            let t = (h - a) / (b - a);
            return [0, 1, 2].map(|k| ca[k] + (cb[k] - ca[k]) * t);
        }
    }
    RAMP[RAMP.len() - 1].1
}

/// The relief in `heights` as sRGB pixels, row by row: tinted by height
/// over `sea` (water below it), lit from the north-west.
pub fn shade(heights: &Grid, sea: f32) -> Vec<[u8; 3]> {
    shade_with(heights, sea, None)
}

/// The relief lit as [`shade`] lights it, its land coloured by `colors`
/// (linear light, three planes over the same samples) when given, tinted
/// by height otherwise.
pub fn shade_with(heights: &Grid, sea: f32, colors: Option<&Grid>) -> Vec<[u8; 3]> {
    // Slopes drawn steeper than they are, as relief maps do, so that a
    // gentle relief still reads.
    const EXAGGERATION: f32 = 2.5;
    let spacing = Space::spacing(heights.lod) as f32 / EXAGGERATION;
    let r = heights.rect;
    let sun = {
        let (x, y, z) = (-1.0f32, 1.1f32, -1.0f32);
        let l = (x * x + y * y + z * z).sqrt();
        [x / l, y / l, z / l]
    };
    let mut out = Vec::with_capacity(r.area());
    for z in r.z..r.z1() {
        for x in r.x..r.x1() {
            let h = heights.at(x, z);
            let dx = (heights.at(x + 1, z) - heights.at(x - 1, z)) / (2.0 * spacing);
            let dz = (heights.at(x, z + 1) - heights.at(x, z - 1)) / (2.0 * spacing);
            let (nx, ny, nz) = (-dx, 1.0, -dz);
            let l = (nx * nx + ny * ny + nz * nz).sqrt();
            let lit = ((nx * sun[0] + ny * sun[1] + nz * sun[2]) / l).max(0.0);
            let c = if h < sea {
                // Water: deeper, darker; the sea floor's relief shows faintly.
                let d = ((sea - h) / 40.0).min(1.0);
                let w = tint(-1.0 - 59.0 * d);
                w.map(|v| v * (0.85 + 0.15 * lit))
            } else {
                // Sky light from above, softer where steep, and the sun.
                let sky = 0.35 + 0.15 * (ny / l);
                let base = match colors {
                    Some(g) => [0, 1, 2].map(|q| encode(g.at_plane(q, x, z))),
                    None => tint(h - sea),
                };
                base.map(|v| v * (sky + 0.75 * lit))
            };
            out.push(c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        }
    }
    out
}

/// Lays `mask` over pixels in `color` (sRGB), as much as the mask and
/// `opacity` say: a mask seen over the relief it comes from.
pub fn overlay(pixels: &mut [[u8; 3]], mask: &Grid, color: [u8; 3], opacity: f32) {
    for (px, m) in pixels.iter_mut().zip(&mask.data) {
        let a = (m.clamp(0.0, 1.0) * opacity).clamp(0.0, 1.0);
        for q in 0..3 {
            px[q] = (px[q] as f32 + (color[q] as f32 - px[q] as f32) * a + 0.5) as u8;
        }
    }
}

/// Linear light to sRGB, both from 0 to 1.
fn encode(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// A mask as greys, 0 black and 1 white.
pub fn grey(mask: &Grid) -> Vec<[u8; 3]> {
    mask.data.iter().map(|v| [(v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8; 3]).collect()
}
