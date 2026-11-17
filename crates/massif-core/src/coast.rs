//! The coasts of real places, shaped from coarse data.
//!
//! The elevation tiles give the land near the sea from sources of every
//! resolution, often a 30 m or 90 m grid blown up to the tiles' pixels
//! square by square: read as it is, the land at a metre or two over the
//! sea turns to grid-aligned pools and wide flats where the coast wanders
//! between the squares. And their sea is often no depth at all (a land
//! model's 0 m), beside a bathymetry of other sources whose rectangles meet
//! at straight seams.
//!
//! [`shape`] makes a coast of them: the heights near the sea's level are
//! smoothed over the size of the squares the data comes in, so that the
//! coastline, where they cross 0 m between samples, follows a smooth curve;
//! and the sea floor deepens from the shore as far as the bathymetry says,
//! that bathymetry smoothed over its own coarse cells, so that no seam of
//! its sources shows.

use rayon::prelude::*;

/// Heights within this many metres of the sea are smoothed whole; from
/// there to [`BAND_OUT`] less and less, and not past it.
pub const BAND_IN: f32 = 2.0;
pub const BAND_OUT: f32 = 6.0;
/// The sea floor deepens by this many metres a metre away from the shore,
/// until the bathymetry says it is no deeper.
pub const SHELF: f32 = 0.04;
/// Heights within this many metres of the sea's level say nothing of which
/// side of the coast they are on (see [`shape`]).
pub const NOISE: f32 = 0.5;
/// The width of the band the data rings in along a coast, in metres: the
/// coast is smoothed over it at least.
pub const RING: f64 = 12.0;
/// The least height of the land at the coast, and depth of the sea, in
/// metres, before the sea floor is made.
pub const LAND_MIN: f32 = 0.3;
/// The sea's depth at the shore, in metres.
pub const SHORE_DEPTH: f32 = 1.0;
/// How deep the sea gets where nothing says how deep it is, in metres.
pub const UNKNOWN_DEPTH: f32 = 30.0;
/// Past this many metres from any depth the data gives, the sea is as deep
/// as where nothing says ([`UNKNOWN_DEPTH`]) at most: a bay of a land
/// model's 0 m, far from the ocean's bathymetry, takes none of its depth.
pub const FAR_KNOWN: f32 = 3000.0;
/// Depths of the data shallower than this many metres say nothing: a land
/// model's sea at 0 m, or its coast.
pub const KNOWN_DEPTH: f32 = 1.5;
/// The size of the cells of the bathymetry, smoothed over, in metres (the
/// global grids the tiles take their oceans from are 450 m to 1.8 km).
pub const BATHYMETRY_CELL: f64 = 450.0;

/// What [`shape`] found and did.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CoastReport {
    /// The size the data near the sea comes in, in samples (1 where every
    /// sample is its own).
    pub square: f32,
    /// Samples under the sea.
    pub sea: usize,
    /// Of them, those the bathymetry gave a depth.
    pub known: usize,
    /// Samples near the sea's level of a coarse fill, given the finer data
    /// around them.
    pub coarse: usize,
}

/// Shape the coasts of an `nx` × `nz` grid of heights in metres,
/// `metres_per_sample` apart (see the module). Samples without data
/// (`NaN`) are left so.
/// `square`: the size of the squares the data comes in, in samples, when
/// its source says; else measured on the samples.
pub fn shape(metres: &mut [f32], nx: usize, nz: usize, metres_per_sample: f64, square: Option<f32>) -> CoastReport {
    let mut report = CoastReport { square: 1.0, ..CoastReport::default() };
    if nx < 2 || nz < 2 || metres.iter().all(|m| !m.is_finite()) {
        return report;
    }
    // Work on a grid with no hole: the gaps given the heights around them.
    let holes: Vec<bool> = metres.iter().map(|m| !m.is_finite()).collect();
    let any_hole = holes.iter().any(|&h| h);
    let mut grid = metres.to_vec();
    if any_hole {
        crate::real::fill_grid(&mut grid, nx, nz);
    }

    // Low land of a coarse fill beside finer data: given the finer data's
    // continuation.
    report.coarse = mend_coarse(&mut grid, nx, nz, metres_per_sample);

    // The coast smoothed over the squares the data comes in: which side of
    // it each sample is on (land or sea, smoothed, its sign telling), and
    // the heights near the sea's level.
    let square = square.unwrap_or_else(|| square_size(&grid, nx)).max(1.0);
    report.square = square;
    let sigma = (0.6 * square).max((RING / metres_per_sample) as f32).max(1.0);
    let original = grid.clone();
    // Within half a metre of the sea's level the data rings (resampled
    // over a sharp coast, it swings either side of 0 m in a band along
    // it): such a sample says neither, and the land or the sea beyond it
    // decide. A sea of the land models is 0 m exactly.
    let mut side: Vec<f32> = grid.iter().map(|&m| if m == 0.0 || m <= -NOISE { -1.0 } else if m >= NOISE { 1.0 } else { 0.0 }).collect();
    gaussian(&mut side, nx, nz, sigma);
    let mut blurred = grid.clone();
    gaussian(&mut blurred, nx, nz, sigma);
    // How sure the side is, from -1 (the sea) to 1 (the land): near the
    // coast the heights follow it, so that the coastline, where they cross
    // 0 m between samples, is its smooth line rather than the samples'
    // squares.
    let mut rim = vec![0f32; grid.len()];
    grid.par_iter_mut().zip(rim.par_iter_mut()).zip(blurred.par_iter().zip(side.par_iter())).for_each(|((m, r), (&b, &sd))| {
        let far = m.abs().max(b.abs());
        let t = ((far - BAND_IN) / (BAND_OUT - BAND_IN)).clamp(0.0, 1.0);
        // A sample that rings says nothing of its own.
        let own = if *m == 0.0 || *m <= -NOISE { -1.0 } else if *m >= NOISE { 1.0 } else { sd };
        let v = sd + (own - sd) * t;
        let h = b + (*m - b) * t;
        *r = sd;
        *m = if v > 0.0 { h.max(LAND_MIN * v.min(1.0)).max(1e-3) } else { h.min(-LAND_MIN) };
    });
    drop((blurred, side));

    // The sea floor.
    let wet: Vec<bool> = grid.iter().map(|&m| m <= 0.0).collect();
    report.sea = wet.iter().filter(|&&w| w).count();
    if report.sea > 0 && report.sea < grid.len() {
        let dist = distance_to(&wet, nx, nz);
        let mut known: Vec<f32> = original.iter().zip(&wet).map(|(&m, &w)| if w && m < -KNOWN_DEPTH { m } else { f32::NAN }).collect();
        report.known = known.iter().filter(|m| m.is_finite()).count();
        let bathymetry = report.known > 0;
        let far_known = if bathymetry { distance_to(&known.iter().map(|m| !m.is_finite()).collect::<Vec<_>>(), nx, nz) } else { Vec::new() };
        if bathymetry {
            crate::real::fill_grid(&mut known, nx, nz);
            gaussian(&mut known, nx, nz, (BATHYMETRY_CELL / metres_per_sample).max(1.0) as f32);
        }
        let mps = metres_per_sample as f32;
        grid.par_iter_mut().enumerate().for_each(|(k, m)| {
            if !wet[k] {
                return;
            }
            let shelf = SHORE_DEPTH + dist[k] * mps * SHELF;
            let depth = if bathymetry {
                // The bathymetry, but no deeper than the sea is where
                // nothing says how deep it is, far from where something
                // does.
                let data = -known[k];
                let near = (1.0 - far_known[k] * mps / FAR_KNOWN).clamp(0.0, 1.0);
                shelf.min(data).min(UNKNOWN_DEPTH + (data - UNKNOWN_DEPTH).max(0.0) * near)
            } else {
                shelf.min(UNKNOWN_DEPTH)
            };
            let depth = depth.max(SHORE_DEPTH);
            // By the coast, shallower for as unsure as the side is.
            *m = -depth * (-rim[k]).clamp(0.02, 1.0);
        });
    } else if report.sea == grid.len() {
        // All sea: its own depths, but never dry.
        grid.iter_mut().for_each(|m| *m = m.min(-SHORE_DEPTH));
    }

    for (k, m) in metres.iter_mut().enumerate() {
        if !holes[k] {
            *m = grid[k];
        }
    }
    report
}

/// The lakes of a grid of heights: for each sample, the lake it is in
/// (0: none; `k`: `levels[k − 1]`).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Lakes {
    pub of: Vec<u16>,
    /// Each lake's level, in metres.
    pub levels: Vec<f32>,
}

impl Lakes {
    /// The level of the lake sample `k` is in.
    pub fn level(&self, k: usize) -> Option<f32> {
        let n = *self.of.get(k)?;
        (n > 0).then(|| self.levels[n as usize - 1])
    }
}

/// Heights within this many metres of each other are one flat: less than
/// the tiles' step (1/256 m), so that only a surface the data gives flat
/// to its last digit is, as the models give their lakes; a valley's floor,
/// however gentle, is not.
const FLAT: f32 = 0.002;
/// The smallest lake found, in square metres.
pub const LAKE_AREA: f64 = 5_000.0;
/// A lake stands at least this high over the sea, in metres (lower, it is
/// the sea's).
pub const LAKE_LEVEL: f32 = 0.5;
/// Of the samples around a flat, the share higher than it for it to be a
/// lake (a flat terrace has ground lower on one side).
const LAKE_RIM: f32 = 0.85;
/// A lake's depth a metre from its shore, in metres a metre, and the most.
const LAKE_SHELF: f32 = 0.05;
const LAKE_DEPTH: f32 = 12.0;

/// Find the lakes of a grid of heights, `metres_per_sample` apart: the
/// elevation models give a lake's surface, flat to the centimetre, over
/// the sea's level, with the shore higher all round. See [`dig_lakes`] for
/// their floors.
pub fn find_lakes(grid: &[f32], nx: usize, nz: usize, metres_per_sample: f64) -> Lakes {
    let mut lakes = Lakes::default();
    if nx < 3 || nz < 3 {
        return lakes;
    }
    let flat: Vec<bool> = (0..grid.len())
        .into_par_iter()
        .map(|k| {
            let (i, j) = (k % nx, k / nx);
            let m = grid[k];
            if !m.is_finite() || m < LAKE_LEVEL || i == 0 || j == 0 || i + 1 == nx || j + 1 == nz {
                return false;
            }
            [grid[k - 1], grid[k + 1], grid[k - nx], grid[k + nx]].iter().all(|&v| (v - m).abs() <= FLAT)
        })
        .collect();
    if !flat.iter().any(|&f| f) {
        return lakes;
    }
    let least = ((LAKE_AREA / (metres_per_sample * metres_per_sample)).ceil() as usize).max(16);
    let mut of = vec![0u16; grid.len()];
    let mut seen = vec![false; grid.len()];
    let mut stack = Vec::new();
    let mut cells = Vec::new();
    for start in 0..grid.len() {
        if !flat[start] || seen[start] {
            continue;
        }
        let level = grid[start];
        cells.clear();
        stack.push(start);
        seen[start] = true;
        let (mut rim, mut higher) = (0usize, 0usize);
        while let Some(k) = stack.pop() {
            cells.push(k);
            let (i, j) = (k % nx, k / nx);
            for (ok, o) in [(i > 0, k.wrapping_sub(1)), (i + 1 < nx, k + 1), (j > 0, k.wrapping_sub(nx)), (j + 1 < nz, k + nx)] {
                if !ok {
                    continue;
                }
                // The lake's own level, flat or at its shore.
                if (grid[o] - level).abs() <= 2.0 * FLAT {
                    if !seen[o] {
                        seen[o] = true;
                        stack.push(o);
                    }
                } else {
                    rim += 1;
                    higher += (grid[o] > level) as usize;
                }
            }
        }
        if cells.len() < least || rim == 0 || (higher as f32) < LAKE_RIM * rim as f32 || lakes.levels.len() >= u16::MAX as usize - 1 {
            continue;
        }
        lakes.levels.push(level);
        let n = lakes.levels.len() as u16;
        for &k in &cells {
            of[k] = n;
        }
    }
    if !lakes.levels.is_empty() {
        lakes.of = of;
    }
    lakes
}

/// Give the lakes floors under their surfaces, deepening from their shores.
pub fn dig_lakes(grid: &mut [f32], nx: usize, nz: usize, metres_per_sample: f64, lakes: &Lakes) {
    if lakes.levels.is_empty() {
        return;
    }
    let wet: Vec<bool> = lakes.of.iter().map(|&n| n > 0).collect();
    let dist = distance_to(&wet, nx, nz);
    let mps = metres_per_sample as f32;
    for (k, m) in grid.iter_mut().enumerate() {
        if let Some(level) = lakes.level(k) {
            let depth = (1.0 + dist[k] * mps * LAKE_SHELF).min(LAKE_DEPTH);
            *m = level - depth;
        }
    }
}

/// How far finer data is carried over the coarse fill beside it, in
/// metres: all the way within this, less and less to twice as far.
pub const MEND_REACH: f64 = 400.0;
/// A step across the edge of a coarse fill of at least this many metres is
/// a seam, smoothed away.
const SEAM: f32 = 0.75;
/// Second differences under this many metres a sample say a coarse fill:
/// heights read between the pixels of a grid far coarser than the tiles'.
const SMOOTH: f32 = 0.02;

/// The elevation tiles fill the gaps of their finer sources (the radar of
/// SRTM loses water and steep ground) from a far coarser grid, read between
/// its points: in rectangles about the finer data, low land where that data
/// has the sea, and straight coasts along the rectangles' edges. Find the
/// samples of such a fill near the sea's level (smooth to the second
/// difference, yet not flat: a sea or a lake at one height is data), and
/// give them the finer data's heights carried over them, near it; returns
/// how many were.
pub fn mend_coarse(grid: &mut [f32], nx: usize, nz: usize, metres_per_sample: f64) -> usize {
    let coarse = coarse_fill(grid, nx, nz);
    let count = coarse.iter().filter(|&&c| c).count();
    if count == 0 || count == grid.len() {
        return 0;
    }
    let reach = (MEND_REACH / metres_per_sample).max(1.0) as f32;
    let dist = distance_to(&coarse, nx, nz);
    let weight = |k: usize| (2.0 - dist[k] / reach).clamp(0.0, 1.0);
    // Its step to the finer data along its edge, carried into it less and
    // less: no straight seam where they meet, at any height.
    let mut edge = vec![false; grid.len()];
    let mut offset = vec![f32::NAN; grid.len()];
    for k in 0..grid.len() {
        if !coarse[k] {
            continue;
        }
        // The step to each neighbour of the finer data, past what the
        // fill's own slope makes (the second difference across the seam).
        let (i, j) = (k % nx, k / nx);
        let (mut sum, mut n) = (0.0, 0.0);
        for (ok, o, b) in [
            (i > 0 && i + 1 < nx, k.wrapping_sub(1), k + 1),
            (i + 1 < nx && i > 0, k + 1, k.wrapping_sub(1)),
            (j > 0 && j + 1 < nz, k.wrapping_sub(nx), k + nx),
            (j + 1 < nz && j > 0, k + nx, k.wrapping_sub(nx)),
        ] {
            if ok && !coarse[o] {
                sum += grid[o] - (2.0 * grid[k] - grid[b]);
                n += 1.0;
            }
        }
        if n > 0.0 {
            // A step the finer data's own roughness makes is no seam.
            let step = sum / n;
            offset[k] = if step.abs() >= SEAM { step } else { 0.0 };
            edge[k] = true;
        }
    }
    let inner: Vec<bool> = coarse.iter().zip(&edge).map(|(&c, &e)| c && !e).collect();
    crate::real::fill_grid(&mut offset, nx, nz);
    crate::real::relax(&mut offset, &inner, nx, nz, 48);
    // Near the sea's level, the finer data's heights themselves, carried
    // over the fill: its coast, not the fill's.
    let mut carried: Vec<f32> = grid.iter().zip(&coarse).map(|(&m, &c)| if c { f32::NAN } else { m }).collect();
    crate::real::fill_grid(&mut carried, nx, nz);
    crate::real::relax(&mut carried, &coarse, nx, nz, 48);
    let mut mended = 0;
    for k in 0..grid.len() {
        if !coarse[k] {
            continue;
        }
        let w = weight(k);
        if w <= 0.0 {
            continue;
        }
        let low = grid[k].abs() < BAND_OUT;
        grid[k] += w * offset[k];
        if low {
            grid[k] += w * (carried[k] - grid[k]);
            mended += 1;
        }
    }
    mended
}

/// The samples of a coarse fill (see [`mend_coarse`]): over most of the
/// square around each, the heights' second differences are tiny while the
/// heights still slope; and the finer data's along its edge.
pub fn coarse_fill(grid: &[f32], nx: usize, nz: usize) -> Vec<bool> {
    if nx < 3 || nz < 3 {
        return vec![false; grid.len()];
    }
    let raw: Vec<f32> = (0..grid.len())
        .into_par_iter()
        .map(|k| {
            let (i, j) = (k % nx, k / nx);
            let m = grid[k];
            if !m.is_finite() || i == 0 || j == 0 || i + 1 == nx || j + 1 == nz {
                return 0.0;
            }
            let (l, r, u, d) = (grid[k - 1], grid[k + 1], grid[k - nx], grid[k + nx]);
            let second = (l + r - 2.0 * m).abs().max((u + d - 2.0 * m).abs());
            let slope = (r - l).abs().max((d - u).abs());
            (second < SMOOTH && slope > 1e-3) as u8 as f32
        })
        .collect();
    // Most of the square of 9 samples around: a fill, not a lone smooth
    // sample of finer data.
    let mut share = raw;
    gaussian(&mut share, nx, nz, 3.0);
    let fill: Vec<bool> = share.iter().map(|&s| s > 0.6).collect();
    // And the samples along its edge, whose differences cross into the
    // finer data.
    let edge = distance_to(&fill.iter().map(|&f| !f).collect::<Vec<_>>(), nx, nz);
    edge.iter().map(|&d| d <= 2.5).collect()
}

/// The size, in samples, of the squares the low land near the sea comes
/// in: how far, on average, a row of it runs at one height. 1 when each
/// sample has its own.
pub fn square_size(grid: &[f32], nx: usize) -> f32 {
    let mut runs: Vec<u32> = grid.par_chunks(nx).flat_map_iter(|row| {
        let mut out = Vec::new();
        low_runs(row, &mut out);
        out
    }).collect();
    median_run(&mut runs)
}

/// The runs of low land at one height along `row`, ended by another height
/// of low land on both sides (not a flat stretch, nor the coast), added to
/// `runs`.
pub fn low_runs(row: &[f32], runs: &mut Vec<u32>) {
    let low = |m: f32| m > 0.0 && m < BAND_OUT;
    let n = row.len();
    let mut start = 0;
    for i in 1..=n {
        if i < n && row[i] == row[i - 1] {
            continue;
        }
        if start > 0 && i < n && low(row[start - 1]) && low(row[i]) && low(row[start]) {
            runs.push((i - start) as u32);
        }
        start = i;
    }
}

/// The middle run of `runs`, from 1 to 32; 1 with too few to tell.
pub fn median_run(runs: &mut [u32]) -> f32 {
    if runs.len() < 64 {
        return 1.0;
    }
    let mid = runs.len() / 2;
    let (_, median, _) = runs.select_nth_unstable(mid);
    (*median as f32).clamp(1.0, 32.0)
}

/// Blur a grid with a Gaussian of `sigma` samples (three box blurs).
pub fn gaussian(grid: &mut [f32], nx: usize, nz: usize, sigma: f32) {
    if sigma < 0.5 {
        return;
    }
    // Three boxes of width w: σ² = (w² − 1) / 4.
    let w = ((4.0 * sigma * sigma + 1.0).sqrt().round() as usize).max(1) | 1;
    let r = w / 2;
    if r == 0 {
        return;
    }
    for _ in 0..3 {
        grid.par_chunks_mut(nx).for_each(|row| box_line(row, r));
        // Columns, a band of them at a time.
        const BAND: usize = 32;
        let cols: Vec<usize> = (0..nx).step_by(BAND).collect();
        let ptr = SyncPtr(grid.as_mut_ptr());
        cols.par_iter().for_each(|&c0| {
            let c1 = (c0 + BAND).min(nx);
            let mut col = vec![0f32; nz];
            for c in c0..c1 {
                // SAFETY: each band of columns is worked by one thread.
                unsafe {
                    for (j, v) in col.iter_mut().enumerate() {
                        *v = *ptr.get().add(j * nx + c);
                    }
                }
                box_line(&mut col, r);
                unsafe {
                    for (j, v) in col.iter().enumerate() {
                        *ptr.get().add(j * nx + c) = *v;
                    }
                }
            }
        });
    }
}

struct SyncPtr(*mut f32);
unsafe impl Send for SyncPtr {}
unsafe impl Sync for SyncPtr {}
impl SyncPtr {
    fn get(&self) -> *mut f32 {
        self.0
    }
}

/// A box blur of radius `r` along a line, the ends held.
fn box_line(line: &mut [f32], r: usize) {
    let n = line.len();
    if n < 2 {
        return;
    }
    let src = line.to_vec();
    let at = |i: isize| src[i.clamp(0, n as isize - 1) as usize] as f64;
    let mut sum: f64 = (-(r as isize)..=r as isize).map(at).sum();
    let w = (2 * r + 1) as f64;
    for (i, v) in line.iter_mut().enumerate() {
        *v = (sum / w) as f32;
        sum += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
    }
}

/// How far each `true` cell is from the nearest `false` one, in cells
/// (chamfer distances, 0 off the set); the set's cells far from any other
/// are as far as the grid is wide.
pub fn distance_to(set: &[bool], nx: usize, nz: usize) -> Vec<f32> {
    const A: f32 = 1.0;
    const B: f32 = std::f32::consts::SQRT_2;
    let far = (nx + nz) as f32;
    let mut d: Vec<f32> = set.iter().map(|&s| if s { far } else { 0.0 }).collect();
    for j in 0..nz {
        for i in 0..nx {
            let k = j * nx + i;
            if d[k] == 0.0 {
                continue;
            }
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + A);
            }
            if j > 0 {
                v = v.min(d[k - nx] + A);
                if i > 0 {
                    v = v.min(d[k - nx - 1] + B);
                }
                if i + 1 < nx {
                    v = v.min(d[k - nx + 1] + B);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..nz).rev() {
        for i in (0..nx).rev() {
            let k = j * nx + i;
            if d[k] == 0.0 {
                continue;
            }
            let mut v = d[k];
            if i + 1 < nx {
                v = v.min(d[k + 1] + A);
            }
            if j + 1 < nz {
                v = v.min(d[k + nx] + A);
                if i + 1 < nx {
                    v = v.min(d[k + nx + 1] + B);
                }
                if i > 0 {
                    v = v.min(d[k + nx - 1] + B);
                }
            }
            d[k] = v;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A coast of land 1 m high and sea at 0 m, given in squares of 8
    /// samples: a coastline going round a circle, the squares' corners
    /// around it.
    fn blocky(n: usize, square: usize) -> Vec<f32> {
        let c = n as f32 / 2.0;
        (0..n * n)
            .map(|k| {
                let (i, j) = ((k % n) / square * square + square / 2, (k / n) / square * square + square / 2);
                let r = ((i as f32 - c).powi(2) + (j as f32 - c).powi(2)).sqrt();
                if r < n as f32 * 0.3 { 0.5 + ((i * 7 + j * 3) % 11) as f32 * 0.1 } else { 0.0 }
            })
            .collect()
    }

    #[test]
    fn squares_of_data_are_measured() {
        let g = blocky(256, 8);
        let s = square_size(&g, 256);
        assert!(s > 6.0, "{s}");
        let fine: Vec<f32> = (0..256 * 256).map(|k| ((k % 256) as f32 * 0.37).sin()).collect();
        assert!(square_size(&fine, 256) < 1.5);
    }

    #[test]
    fn a_coast_of_squares_is_made_round() {
        let n = 256;
        let mut g = blocky(n, 8);
        let report = shape(&mut g, n, n, 2.0, None);
        assert!(report.square > 6.0);
        // The coastline along a row through the middle and one through a
        // square's corner cross the land once each way, and the land
        // reaches as far as the circle does, within a square.
        let c = n as f32 / 2.0;
        for j in [n / 2, n / 2 + 37, n / 2 + 61] {
            let row = &g[j * n..(j + 1) * n];
            let crossings = row.windows(2).filter(|w| (w[0] > 0.0) != (w[1] > 0.0)).count();
            assert_eq!(crossings, 2, "row {j}");
            let first = row.iter().position(|&m| m > 0.0).unwrap() as f32;
            let r = n as f32 * 0.3;
            let edge = c - (r * r - (j as f32 - c).powi(2)).max(0.0).sqrt();
            assert!((first - edge).abs() < 8.0, "row {j}: land from {first}, the circle from {edge}");
        }
        // Squares inside a square: the sea deepens away from the shore.
        assert!(g[4 * n + 4] < g[(n / 2) * n + (n / 2 - (0.3 * n as f32) as usize - 4)]);
    }

    #[test]
    fn a_coarse_fill_takes_the_finer_coast() {
        // Finer data in a rectangle: an island of 10 m, rough, in a sea at
        // -5 m. Around it, a coarse fill sloping gently from 1 m to 3 m:
        // land the finer data says is sea.
        let n = 256;
        let mut g: Vec<f32> = (0..n * n)
            .map(|k| {
                let (i, j) = (k % n, k / n);
                if (64..192).contains(&i) && (64..192).contains(&j) {
                    let r = ((i as f32 - 128.0).powi(2) + (j as f32 - 128.0).powi(2)).sqrt();
                    let rough = ((i * 7 + j * 13) % 5) as f32 * 0.4;
                    if r < 40.0 { 10.0 + rough } else { -5.0 + rough * 0.2 }
                } else {
                    1.0 + 2.0 * i as f32 / n as f32
                }
            })
            .collect();
        let coarse = coarse_fill(&g, n, n);
        assert!(coarse[10 * n + 10] && coarse[128 * n + 30]);
        assert!(!coarse[128 * n + 128] && !coarse[100 * n + 80]);
        let report = shape(&mut g, n, n, 10.0, None);
        assert!(report.coarse > 0);
        // Just outside the rectangle, by the finer sea: the sea now, and no
        // straight coast along the rectangle's edge.
        for j in [80, 128, 170] {
            assert!(g[j * n + 60] <= 0.0, "row {j}: {}", g[j * n + 60]);
        }
        // The island stays.
        assert!(g[128 * n + 128] > 5.0);
    }

    #[test]
    fn the_sea_floor_has_no_seam() {
        // Sea west of x = 40, land east; the bathymetry's two sources meet
        // at z = 128: 0 m (a land model's sea) north of it, 20 m deep under.
        let n = 256;
        let mut g: Vec<f32> = (0..n * n)
            .map(|k| {
                let (i, j) = (k % n, k / n);
                if i >= 200 { 30.0 } else if j < 128 { 0.0 } else { -20.0 }
            })
            .collect();
        shape(&mut g, n, n, 30.0, None);
        // No step across the seam anywhere along it.
        for i in 0..200 {
            let step = (g[128 * n + i] - g[127 * n + i]).abs();
            assert!(step < 0.6, "a step of {step} m at x {i}");
        }
        // Deeper away from the shore, at most what the bathymetry says.
        assert!(g[200 * n + 10] < g[200 * n + 190]);
        assert!(g[200 * n + 10] >= -20.5);
        assert!(g.iter().all(|m| *m >= -20.5));
        assert!(g[200 * n + 199] < 0.0 && g[200 * n + 196] <= -SHORE_DEPTH);
    }
}
