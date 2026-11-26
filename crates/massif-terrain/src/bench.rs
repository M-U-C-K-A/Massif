//! The relief bench: measures that say how much a relief looks like real
//! land, so that the generator's work is judged by numbers and not only by
//! eye. Real landforms come from public elevation models (Copernicus
//! GLO-30, fetched by `tools/relief_bench.sh`), cut and resampled to a
//! square grid; the generator's outputs are measured the same way, at the
//! same size and range of heights.
//!
//! The measures are the geomorphologists' usual ones:
//! - the slope distribution (degrees);
//! - the hypsometric curve and integral (how the land is spread over its
//!   heights: young, deeply cut land is low on the whole, a plateau high);
//! - the drainage density (km of streams per km², streams where more than a
//!   set area drains), and the same of ridges (streams of the land upside
//!   down);
//! - the curvature of the valley floors and of the crests (V-shaped
//!   torrents are sharp, U-shaped glacial valleys broad).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Heights on a square-celled grid, row by row from the north.
#[derive(Debug, Clone)]
pub struct Relief {
    pub w: usize,
    pub h: usize,
    /// Metres between samples.
    pub spacing: f64,
    pub data: Vec<f32>,
}

impl Relief {
    #[inline]
    pub fn at(&self, x: usize, z: usize) -> f32 {
        self.data[z * self.w + x]
    }

    /// Heights stretched linearly so that their 2nd and 98th percentiles
    /// are `lo` and `hi`: another relief brought to the same range.
    pub fn fitted(&self, lo: f32, hi: f32) -> Relief {
        let (a, b) = (percentile(&self.data, 0.02), percentile(&self.data, 0.98));
        let k = if b > a { (hi - lo) / (b - a) } else { 0.0 };
        Relief { data: self.data.iter().map(|v| lo + (v - a) * k).collect(), ..self.clone() }
    }
}

/// The value below which `q` (0–1) of `v` lies.
pub fn percentile(v: &[f32], q: f64) -> f32 {
    let mut s: Vec<f32> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 0.0;
    }
    let k = ((s.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    *s.select_nth_unstable_by(k, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal)).1
}

/// The slope bins of [`Measures::slopes`], in degrees.
pub const SLOPE_BINS: [f32; 6] = [5.0, 15.0, 25.0, 35.0, 45.0, 90.0];

/// What the bench measures of a relief.
#[derive(Debug, Clone, PartialEq)]
pub struct Measures {
    /// Metres between the 2nd and the 98th percentile of the heights.
    pub relief: f32,
    /// Slope in degrees: mean, median, 90th percentile.
    pub slope_mean: f32,
    pub slope_median: f32,
    pub slope_p90: f32,
    /// The share of the land in each bin of [`SLOPE_BINS`] (up to 5°, 5–15°…).
    pub slopes: [f32; 6],
    /// The hypsometric curve: the height (0 the lowest, 1 the highest)
    /// above which 0, 10… 100 % of the land lies.
    pub hypsometry: [f32; 11],
    /// The hypsometric integral: the area under that curve.
    pub hypsometric_integral: f32,
    /// Km of streams per km² (streams where at least [`CHANNEL_AREA`] drains).
    pub drainage_density: f32,
    /// Km of ridges per km², likewise on the relief turned upside down.
    pub ridge_density: f32,
    /// The mean curvature (Laplacian, per km) along the valley floors where
    /// at least [`VALLEY_AREA`] drains: high in V-shaped valleys, low in
    /// broad U-shaped ones.
    pub valley_curvature: f32,
    /// Likewise along the main crests (negative: convex), sharp or round.
    pub ridge_curvature: f32,
}

/// The area that makes a stream, in m².
pub const CHANNEL_AREA: f64 = 200_000.0;
/// The area that makes a valley whose floor is measured, in m².
pub const VALLEY_AREA: f64 = 1_000_000.0;

/// Measures relief `r`.
pub fn measure(r: &Relief) -> Measures {
    let (w, h, d) = (r.w, r.h, r.spacing as f32);
    let (lo, hi) = (percentile(&r.data, 0.02), percentile(&r.data, 0.98));
    // Slopes, by central differences inside the edge.
    let mut slopes = Vec::with_capacity(w * h);
    for z in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = (r.at(x + 1, z) - r.at(x - 1, z)) / (2.0 * d);
            let gz = (r.at(x, z + 1) - r.at(x, z - 1)) / (2.0 * d);
            slopes.push(gx.hypot(gz).atan().to_degrees());
        }
    }
    let n = slopes.len().max(1) as f32;
    let mut bins = [0.0f32; 6];
    for s in &slopes {
        let b = SLOPE_BINS.iter().position(|&e| *s < e).unwrap_or(5);
        bins[b] += 1.0 / n;
    }
    // Hypsometry over the whole range.
    let (min, max) = r.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (max - min).max(1e-6);
    let mut hyps = [0.0f32; 11];
    for (k, hy) in hyps.iter_mut().enumerate() {
        *hy = (percentile(&r.data, 1.0 - k as f64 / 10.0) - min) / span;
    }
    let mean = r.data.iter().map(|v| *v as f64).sum::<f64>() / r.data.len() as f64;
    let integral = ((mean as f32) - min) / span;
    let (dd, valley) = network(r, false);
    let (rd, ridge) = network(r, true);
    Measures {
        relief: hi - lo,
        slope_mean: slopes.iter().sum::<f32>() / n,
        slope_median: percentile(&slopes, 0.5),
        slope_p90: percentile(&slopes, 0.9),
        slopes: bins,
        hypsometry: hyps,
        hypsometric_integral: integral,
        drainage_density: dd,
        ridge_density: rd,
        valley_curvature: valley,
        ridge_curvature: -ridge,
    }
}

/// The drainage network of `r` (of `-r` when `inverted`): its density in
/// km per km², and the mean curvature along its larger channels, per km.
fn network(r: &Relief, inverted: bool) -> (f32, f32) {
    let (w, h, d) = (r.w, r.h, r.spacing);
    let z0: Vec<f32> = if inverted { r.data.iter().map(|v| -v).collect() } else { r.data.clone() };
    let filled = fill(&z0, w, h);
    // D8 directions on the filled surface (pits filled with a tiny slope,
    // so that every sample drains to the edge).
    const D: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
    let mut down = vec![usize::MAX; w * h];
    let mut length = vec![0.0f64; w * h];
    for z in 0..h {
        for x in 0..w {
            let k = z * w + x;
            let mut best = (0.0f64, usize::MAX, 0.0);
            for (dx, dz) in D {
                let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                if nx < 0 || nz < 0 || nx >= w as i32 || nz >= h as i32 {
                    continue;
                }
                let j = nz as usize * w + nx as usize;
                let dist = if dx != 0 && dz != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
                let drop = (filled[k] - filled[j]) as f64 / dist;
                if drop > best.0 {
                    best = (drop, j, dist);
                }
            }
            down[k] = best.1;
            length[k] = best.2 * d;
        }
    }
    // Areas: from the highest sample down.
    let mut order: Vec<usize> = (0..w * h).collect();
    order.sort_unstable_by(|&a, &b| filled[b].partial_cmp(&filled[a]).unwrap_or(Ordering::Equal));
    let cell = d * d;
    let mut area = vec![cell; w * h];
    for &k in &order {
        if down[k] != usize::MAX {
            let a = area[k];
            area[down[k]] += a;
        }
    }
    // Streams away from the edge, where the area is known whole enough.
    let margin = 2;
    let (mut len, mut curv, mut count) = (0.0f64, 0.0f64, 0usize);
    for z in margin..h - margin {
        for x in margin..w - margin {
            let k = z * w + x;
            // Flats (the sea, a lake) carry no streams nor ridges.
            let flat = (z0[k + 1] - z0[k - 1]).abs().max((z0[k + w] - z0[k - w]).abs()) < (0.02 * d) as f32;
            if flat {
                continue;
            }
            if area[k] >= CHANNEL_AREA && down[k] != usize::MAX {
                len += length[k];
            }
            if area[k] >= VALLEY_AREA {
                let lap = (z0[k - 1] + z0[k + 1] + z0[k - w] + z0[k + w] - 4.0 * z0[k]) as f64 / (d * d);
                curv += lap;
                count += 1;
            }
        }
    }
    let inside = ((w - 2 * margin) * (h - 2 * margin)) as f64 * cell / 1e6;
    let density = (len / 1000.0 / inside.max(1e-9)) as f32;
    let curvature = if count > 0 { (curv / count as f64 * 1000.0) as f32 } else { 0.0 };
    (density, curvature)
}

/// Pits filled (priority flood, Barnes et al. 2014), each filled sample a
/// hair over the one it drains to.
fn fill(z: &[f32], w: usize, h: usize) -> Vec<f32> {
    #[derive(PartialEq)]
    struct Cell(f32, usize);
    impl Eq for Cell {}
    impl PartialOrd for Cell {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Cell {
        fn cmp(&self, o: &Self) -> Ordering {
            o.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal).then(o.1.cmp(&self.1))
        }
    }
    let mut out = z.to_vec();
    let mut done = vec![false; w * h];
    let mut open = BinaryHeap::new();
    for z_ in 0..h {
        for x in 0..w {
            if x == 0 || z_ == 0 || x == w - 1 || z_ == h - 1 {
                let k = z_ * w + x;
                done[k] = true;
                open.push(Cell(out[k], k));
            }
        }
    }
    while let Some(Cell(v, k)) = open.pop() {
        let (x, zz) = ((k % w) as i32, (k / w) as i32);
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (nx, nz) = (x + dx, zz + dz);
            if nx < 0 || nz < 0 || nx >= w as i32 || nz >= h as i32 {
                continue;
            }
            let j = nz as usize * w + nx as usize;
            if done[j] {
                continue;
            }
            done[j] = true;
            let floor = v + 1e-3;
            if out[j] < floor {
                out[j] = floor;
            }
            open.push(Cell(out[j], j));
        }
    }
    out
}

/// How far relief `ours` lies from `real`, measure by measure, each
/// scaled so that 1 is a large gap: slope distribution (half the L1
/// distance of the bins), hypsometric curve (mean gap), drainage and ridge
/// densities and valley curvature (|log| of their ratio). With their mean.
pub fn gaps(real: &Measures, ours: &Measures) -> [(&'static str, f32); 6] {
    let ratio = |a: f32, b: f32| ((a.abs().max(1e-3)) / (b.abs().max(1e-3))).ln().abs() / 2f32.ln();
    let slope = real.slopes.iter().zip(&ours.slopes).map(|(a, b)| (a - b).abs()).sum::<f32>() * 0.5;
    let hyps = real.hypsometry.iter().zip(&ours.hypsometry).map(|(a, b)| (a - b).abs()).sum::<f32>() / 11.0 * 4.0;
    let dd = ratio(real.drainage_density, ours.drainage_density);
    let rd = ratio(real.ridge_density, ours.ridge_density);
    let vc = ratio(real.valley_curvature, ours.valley_curvature);
    let all = (slope + hyps + dd + rd + vc) / 5.0;
    [("slopes", slope), ("hypsometry", hyps), ("drainage", dd), ("ridges", rd), ("valley shape", vc), ("overall", all)]
}

/// A real landform of the bench: a window `km` wide around a place, cut
/// from one Copernicus GLO-30 tile.
#[derive(Debug, Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub what: &'static str,
    /// The tile, as Copernicus names it (`N46_00_E007_00`).
    pub tile: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub km: f64,
}

/// The bench's landforms (their tiles are the ones `tools/relief_bench.sh`
/// fetches).
pub const CASES: &[Case] = &[
    Case { name: "alps", what: "Bernese Alps: sharp crests, glacial valleys", tile: "N46_00_E007_00", lat: 46.52, lon: 7.85, km: 16.384 },
    Case { name: "pyrenees", what: "Monte Perdido: a range and its cirques", tile: "N42_00_E000_00", lat: 42.66, lon: 0.13, km: 16.384 },
    Case { name: "canyon", what: "Grand Canyon: a plateau cut in strata", tile: "N36_00_W113_00", lat: 36.12, lon: -112.15, km: 16.384 },
    Case { name: "hills", what: "Appalachian plateau: hills cut by streams", tile: "N38_00_W082_00", lat: 38.50, lon: -81.50, km: 16.384 },
    Case { name: "volcano", what: "Pico, Azores: a volcanic island", tile: "N38_00_W029_00", lat: 38.47, lon: -28.40, km: 16.384 },
];

impl Case {
    /// The tile's file name.
    pub fn file(&self) -> String {
        format!("Copernicus_DSM_COG_10_{}_DEM.tif", self.tile)
    }

    /// The window cut from the tile's heights (`w` × `h`, rows from the
    /// north, covering one degree from the tile's south-west corner),
    /// resampled to `n` × `n` square cells.
    pub fn cut(&self, tile: &[f32], w: usize, h: usize, n: usize) -> Relief {
        let (lat0, lon0) = (self.lat.floor(), self.lon.floor());
        // Metres per degree here.
        let (my, mx) = (110_574.0, 111_320.0 * self.lat.to_radians().cos());
        let spacing = self.km * 1000.0 / n as f64;
        let at = |px: f64, py: f64| -> f32 {
            let (fx, fy) = (px.clamp(0.0, (w - 1) as f64), py.clamp(0.0, (h - 1) as f64));
            let (x, y) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x + 1).min(w - 1), (y + 1).min(h - 1));
            let (tx, ty) = ((fx - x as f64) as f32, (fy - y as f64) as f32);
            let a = tile[y * w + x] + (tile[y * w + x1] - tile[y * w + x]) * tx;
            let b = tile[y1 * w + x] + (tile[y1 * w + x1] - tile[y1 * w + x]) * tx;
            a + (b - a) * ty
        };
        let mut data = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let east = (i as f64 + 0.5 - n as f64 / 2.0) * spacing;
                let north = (n as f64 / 2.0 - j as f64 - 0.5) * spacing;
                let (lat, lon) = (self.lat + north / my, self.lon + east / mx);
                let v = at((lon - lon0) * w as f64 - 0.5, (lat0 + 1.0 - lat) * h as f64 - 0.5);
                data.push(if v.is_finite() && v > -1000.0 { v } else { 0.0 });
            }
        }
        Relief { w: n, h: n, spacing, data }
    }
}

/// A grey hillshade of `r`, lit from the north-west 45° up, heights
/// exaggerated by `exaggeration`: the board's pictures, all lit alike.
pub fn hillshade(r: &Relief, exaggeration: f32) -> Vec<u8> {
    let (w, h, d) = (r.w, r.h, r.spacing as f32);
    let light = {
        let (az, alt) = (315f32.to_radians(), 45f32.to_radians());
        [alt.cos() * az.sin(), alt.sin(), -alt.cos() * az.cos()]
    };
    let mut out = vec![0u8; w * h];
    for z in 0..h {
        for x in 0..w {
            let hx = |x: usize| r.at(x.min(w - 1), z);
            let hz = |zz: usize| r.at(x, zz.min(h - 1));
            let gx = (hx(x + 1) - hx(x.saturating_sub(1))) / (2.0 * d) * exaggeration;
            let gz = (hz(z + 1) - hz(z.saturating_sub(1))) / (2.0 * d) * exaggeration;
            let nrm = [-gx, 1.0, -gz];
            let len = (nrm[0] * nrm[0] + 1.0 + nrm[2] * nrm[2]).sqrt();
            let lit = ((nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]) / len).max(0.0);
            out[z * w + x] = (30.0 + 225.0 * lit).min(255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relief(n: usize, spacing: f64, f: impl Fn(f64, f64) -> f64) -> Relief {
        let data = (0..n * n).map(|k| f((k % n) as f64 * spacing, (k / n) as f64 * spacing) as f32).collect();
        Relief { w: n, h: n, spacing, data }
    }

    #[test]
    fn a_plane_has_its_slope() {
        // Rising 0.9 in 1 to the east: 42° everywhere.
        let m = measure(&relief(64, 30.0, |x, _| x * 0.9));
        let deg = 0.9f32.atan().to_degrees();
        assert!((m.slope_mean - deg).abs() < 0.1 && (m.slope_p90 - deg).abs() < 0.1, "{m:?}");
        assert!(m.slopes[4] > 0.99, "{m:?}");
        assert!((m.hypsometric_integral - 0.5).abs() < 0.02, "{m:?}");
    }

    #[test]
    fn a_v_valley_drains_and_curves() {
        // A valley along z, sloping down to the north: one stream down
        // its middle, its floor concave.
        let n = 128;
        let m = measure(&relief(n, 30.0, |x, z| (x - 64.0 * 30.0).abs() * 0.6 + z * 0.05));
        assert!(m.drainage_density > 0.1, "{m:?}");
        assert!(m.valley_curvature > 1.0, "{m:?}");
        // A cone: no valley anywhere, low on the whole.
        let cone = measure(&relief(n, 30.0, |x, z| 2000.0 - ((x - 1920.0).powi(2) + (z - 1920.0).powi(2)).sqrt()));
        assert!(cone.hypsometric_integral < 0.5 && cone.drainage_density == 0.0, "{cone:?}");
    }

    #[test]
    fn the_windows_stay_in_their_tiles() {
        for c in CASES {
            let half = c.km * 500.0;
            let (dlat, dlon) = (half / 110_574.0, half / (111_320.0 * c.lat.to_radians().cos()));
            let (lat0, lon0) = (c.lat.floor(), c.lon.floor());
            assert!(c.lat - dlat > lat0 && c.lat + dlat < lat0 + 1.0 && c.lon - dlon > lon0 && c.lon + dlon < lon0 + 1.0, "{}", c.name);
            let t = &c.tile;
            assert!(t.starts_with(if lat0 >= 0.0 { 'N' } else { 'S' }) && t.contains(&format!("{:03}_00", lon0.abs() as i32)), "{}", c.name);
        }
    }

    #[test]
    fn the_same_relief_has_no_gap() {
        let r = relief(96, 30.0, |x, z| (x * 0.002).sin() * 300.0 + (z * 0.003).cos() * 200.0);
        let m = measure(&r);
        assert!(gaps(&m, &m).iter().all(|(_, g)| *g < 1e-4));
        let f = r.fitted(0.0, 1000.0);
        assert!((percentile(&f.data, 0.98) - percentile(&f.data, 0.02) - 1000.0).abs() < 1.0);
    }
}
