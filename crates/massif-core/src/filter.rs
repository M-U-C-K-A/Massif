//! Brush filters, as in WorldPainter: where on the map a brush acts.
//!
//! A filter turns the world under a brush into a weight per column, from 0
//! (the brush does nothing here) to 1. It looks at what the map shows: the
//! height of the topmost surface, its terrain and biome, whether it is under
//! water, its slope and its curvature (hollows and bumps). Every test that is
//! switched on must pass; the height band can fade out over a few blocks.

use crate::compose::{Span, TileStack};
use crate::coords::{BlockRect, TilePos, local_index};
use crate::tile::NO_FLUID;
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Blocks over which "fade at the limits" softens the height band.
pub const FADE: f32 = 4.0;

/// Something a column can be "on".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Match {
    /// The terrain of the surface.
    Terrain(u16),
    /// The biome painted on the column (0 = automatic).
    Biome(u16),
    /// Under water (or another fluid).
    Water,
    /// Dry land.
    Land,
}

/// The tests of a filter. The default filter lets everything through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    /// Act only at or above this height.
    pub above: Option<i32>,
    /// Act only at or below this height.
    pub below: Option<i32>,
    /// With both limits: act outside the band instead of inside it.
    pub beyond: bool,
    /// Soften the height limits over [`FADE`] blocks.
    pub fade: bool,
    /// Act only on columns matching one of these.
    pub only_on: Option<Vec<Match>>,
    /// Never act on columns matching one of these.
    pub except_on: Option<Vec<Match>>,
    /// Act only where the slope is at least / at most `slope` degrees.
    pub slope_above: bool,
    pub slope_below: bool,
    pub slope: f32,
    /// Act only in hollows / on bumps: columns at least `depth` blocks
    /// below / above the mean of the `radius` blocks around them.
    pub hollows: bool,
    pub bumps: bool,
    pub depth: f32,
    pub radius: i32,
    /// Act only on slopes that face `facing` (degrees: 0 north, 90 east),
    /// give or take `facing_spread` degrees: the side the sun lights, the
    /// side in the shade. Level ground faces nowhere.
    pub facing_on: bool,
    pub facing: f32,
    pub facing_spread: f32,
}

/// Degrees over which the facing test softens at its limits.
pub const FACING_FADE: f32 = 12.0;
/// Slopes gentler than this (degrees) face nowhere.
pub const FACING_MIN_SLOPE: f32 = 4.0;

impl Default for Filter {
    fn default() -> Self {
        Self {
            above: None,
            below: None,
            beyond: false,
            fade: false,
            only_on: None,
            except_on: None,
            slope_above: false,
            slope_below: false,
            slope: 45.0,
            hollows: false,
            bumps: false,
            depth: 1.0,
            radius: 4,
            facing_on: false,
            facing: 180.0,
            facing_spread: 60.0,
        }
    }
}

/// What a filter knows of one column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub height: f32,
    pub terrain: u16,
    pub biome: u16,
    pub wet: bool,
    /// Degrees.
    pub slope: f32,
    /// Height minus the mean height around.
    pub relief: f32,
    /// The way the slope faces, in degrees (0 north, 90 east); `None`
    /// when not worked out.
    pub aspect: Option<f32>,
}

impl Filter {
    /// Whether any test is on.
    pub fn is_active(&self) -> bool {
        self.above.is_some()
            || self.below.is_some()
            || self.only_on.is_some()
            || self.except_on.is_some()
            || self.slope_above
            || self.slope_below
            || self.hollows
            || self.bumps
            || self.facing_on
    }

    fn needs_slope(&self) -> bool {
        self.slope_above || self.slope_below || self.facing_on
    }

    /// Weight of the facing test for a slope of `slope` degrees that faces
    /// `aspect`: 1 within the spread, fading to 0 just outside it.
    pub fn facing_weight(&self, slope: f32, aspect: Option<f32>) -> f32 {
        if !self.facing_on {
            return 1.0;
        }
        let Some(aspect) = aspect.filter(|_| slope >= FACING_MIN_SLOPE) else { return 0.0 };
        let away = ((aspect - self.facing + 540.0).rem_euclid(360.0) - 180.0).abs();
        let spread = self.facing_spread.clamp(1.0, 180.0);
        ((spread - away) / FACING_FADE + 0.5).clamp(0.0, 1.0)
    }

    fn needs_relief(&self) -> bool {
        self.hollows || self.bumps
    }

    /// Weight of the height tests at height `h`.
    pub fn height_weight(&self, h: f32) -> f32 {
        let fade = if self.fade { FADE } else { 0.0 };
        // How far inside a limit `h` is: 1 well inside, 0 outside.
        let inside = |d: f32| {
            if fade > 0.0 {
                (d / fade + 0.5).clamp(0.0, 1.0)
            } else if d >= 0.0 {
                1.0
            } else {
                0.0
            }
        };
        let lo = self.above.map(|a| inside(h - a as f32));
        let hi = self.below.map(|b| inside(b as f32 - h));
        match (lo, hi) {
            (None, None) => 1.0,
            (Some(w), None) | (None, Some(w)) => w,
            (Some(a), Some(b)) if self.beyond => (1.0 - a).max(1.0 - b),
            (Some(a), Some(b)) => a.min(b),
        }
    }

    fn matches(c: &Column, m: &Match) -> bool {
        match *m {
            Match::Terrain(t) => c.terrain == t,
            Match::Biome(b) => c.biome == b,
            Match::Water => c.wet,
            Match::Land => !c.wet,
        }
    }

    /// The weight of one column, from 0 to 1.
    pub fn weight(&self, c: &Column) -> f32 {
        if let Some(only) = &self.only_on
            && !only.iter().any(|m| Self::matches(c, m))
        {
            return 0.0;
        }
        if let Some(except) = &self.except_on
            && except.iter().any(|m| Self::matches(c, m))
        {
            return 0.0;
        }
        if self.slope_above || self.slope_below {
            let ok = (self.slope_above && c.slope >= self.slope) || (self.slope_below && c.slope <= self.slope);
            if !ok {
                return 0.0;
            }
        }
        if self.needs_relief() {
            let ok = (self.hollows && -c.relief >= self.depth) || (self.bumps && c.relief >= self.depth);
            if !ok {
                return 0.0;
            }
        }
        self.height_weight(c.height) * self.facing_weight(c.slope, c.aspect)
    }

    /// The weights of the columns of `rect` (row-major).
    pub fn mask(&self, dim: &Dimension, rect: BlockRect) -> Mask {
        let ring = if self.needs_relief() {
            self.radius.clamp(1, 64)
        } else if self.needs_slope() {
            SLOPE_REACH
        } else {
            0
        };
        let outer = rect.expand(ring);
        let surface = Surface::gather(dim, outer);
        let (w, d) = (rect.width() as usize, rect.depth() as usize);
        let relief = self.needs_relief().then(|| surface.local_means(ring));
        let weights: Vec<u8> = (0..w * d)
            .into_par_iter()
            .map(|i| {
                let (x, z) = (rect.x0 + (i % w) as i32, rect.z0 + (i / w) as i32);
                let Some(col) = surface.column(x, z) else { return 0 };
                let mut c = col;
                if self.needs_slope() {
                    let (gx, gz) = trend_gradient(|dx, dz| surface.column(x + dx, z + dz).map(|n| n.height), SLOPE_REACH, 1.0);
                    c.slope = gx.hypot(gz).atan().to_degrees();
                    c.aspect = aspect_of(gx, gz);
                }
                if let Some(means) = &relief {
                    let j = surface.index(x, z);
                    c.relief = if means[j].is_nan() { 0.0 } else { c.height - means[j] };
                }
                (self.weight(&c) * 255.0).round() as u8
            })
            .collect();
        Mask { rect, weights }
    }
}

/// Blocks around a column the slope is measured over, each way.
pub const SLOPE_REACH: i32 = 4;

/// The slope, in degrees, of the plane that best fits the heights around a
/// point (least squares over a square of `2 r + 1` samples a side): the
/// trend of the hillside there, not the steps between single blocks.
/// `h(dx, dz)` is the height `dx`, `dz` samples away (`None` off the map),
/// `spacing` the blocks between samples.
pub fn trend_slope(h: impl Fn(i32, i32) -> Option<f32>, r: i32, spacing: f32) -> f32 {
    let (gx, gz) = trend_gradient(h, r, spacing);
    gx.hypot(gz).atan().to_degrees()
}

/// The way a hillside faces, in degrees (0 north, 90 east), from the rise
/// of its heights toward the east (`gx`) and the south (`gz`): the way
/// water would run down it. `None` on level ground.
pub fn aspect_of(gx: f32, gz: f32) -> Option<f32> {
    if gx == 0.0 && gz == 0.0 {
        return None;
    }
    // Downhill is (−gx, −gz); north is −z.
    Some((-gx).atan2(gz).to_degrees().rem_euclid(360.0))
}

/// The rise of the best-fitting plane (see [`trend_slope`]) toward the east
/// and toward the south, in blocks per block.
pub fn trend_gradient(h: impl Fn(i32, i32) -> Option<f32>, r: i32, spacing: f32) -> (f32, f32) {
    let Some(centre) = h(0, 0) else { return (0.0, 0.0) };
    let (mut sx, mut sz, mut w) = (0.0f32, 0.0f32, 0.0f32);
    for dz in -r..=r {
        for dx in -r..=r {
            // Missing samples count as level with the centre.
            let v = h(dx, dz).unwrap_or(centre) - centre;
            sx += dx as f32 * v;
            sz += dz as f32 * v;
            w += (dx * dx) as f32;
        }
    }
    if w == 0.0 {
        return (0.0, 0.0);
    }
    (sx / w / spacing, sz / w / spacing)
}

/// The weights of a filter over a rectangle.
#[derive(Debug, Clone)]
pub struct Mask {
    pub rect: BlockRect,
    weights: Vec<u8>,
}

impl Mask {
    /// Weight of column `(x, z)`, 0 outside the rectangle.
    #[inline]
    pub fn get(&self, x: i32, z: i32) -> f32 {
        let r = &self.rect;
        if x < r.x0 || z < r.z0 || x >= r.x1 || z >= r.z1 {
            return 0.0;
        }
        self.weights[(z - r.z0) as usize * r.width() as usize + (x - r.x0) as usize] as f32 / 255.0
    }
}

/// The composed surface of a rectangle of columns.
struct Surface {
    rect: BlockRect,
    columns: Vec<Option<Column>>,
}

impl Surface {
    fn gather(dim: &Dimension, rect: BlockRect) -> Self {
        let tiles: Vec<TilePos> = rect.tiles().collect();
        let width = rect.width() as usize;
        let parts: Vec<Vec<(usize, Column)>> = tiles
            .par_iter()
            .map(|&pos| {
                let Some(stack) = TileStack::new(dim, pos, false) else { return Vec::new() };
                let clip = rect.intersect(&pos.rect());
                let mut spans: Vec<Span> = Vec::new();
                let mut out = Vec::with_capacity((clip.width() * clip.depth()).max(0) as usize);
                let ground = stack.ground();
                for z in clip.z0..clip.z1 {
                    for x in clip.x0..clip.x1 {
                        let cell = local_index(x, z);
                        let Some((height, terrain)) = stack.surface(cell, &mut spans) else { continue };
                        let fluid = ground.fluid.get(cell);
                        let column = Column {
                            height,
                            terrain: terrain.0,
                            biome: ground.biome.get(cell),
                            wet: fluid != NO_FLUID && fluid as f32 > height,
                            slope: 0.0,
                            relief: 0.0,
                            aspect: None,
                        };
                        out.push(((z - rect.z0) as usize * width + (x - rect.x0) as usize, column));
                    }
                }
                out
            })
            .collect();
        let mut columns = vec![None; width * rect.depth() as usize];
        for part in parts {
            for (i, c) in part {
                columns[i] = Some(c);
            }
        }
        Self { rect, columns }
    }

    #[inline]
    fn index(&self, x: i32, z: i32) -> usize {
        (z - self.rect.z0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize
    }

    fn column(&self, x: i32, z: i32) -> Option<Column> {
        let r = &self.rect;
        if x < r.x0 || z < r.z0 || x >= r.x1 || z >= r.z1 {
            return None;
        }
        self.columns[self.index(x, z)]
    }

    /// Mean height of the `(2k + 1)²` columns around each one (NaN where
    /// none is on the map).
    fn local_means(&self, k: i32) -> Vec<f32> {
        let (w, d) = (self.rect.width() as usize, self.rect.depth() as usize);
        let k = k as usize;
        let mut sum: Vec<f32> = self.columns.iter().map(|c| c.map_or(0.0, |c| c.height)).collect();
        let mut count: Vec<f32> = self.columns.iter().map(|c| c.is_some() as u8 as f32).collect();
        for buf in [&mut sum, &mut count] {
            let mut tmp = vec![0f32; w * d];
            // Rows, then columns: a box filter in two passes.
            for z in 0..d {
                let mut acc = 0.0;
                for x in 0..w.min(k + 1) {
                    acc += buf[z * w + x];
                }
                for x in 0..w {
                    tmp[z * w + x] = acc;
                    if x + k + 1 < w {
                        acc += buf[z * w + x + k + 1];
                    }
                    if x >= k {
                        acc -= buf[z * w + x - k];
                    }
                }
            }
            for x in 0..w {
                let mut acc = 0.0;
                for z in 0..d.min(k + 1) {
                    acc += tmp[z * w + x];
                }
                for z in 0..d {
                    buf[z * w + x] = acc;
                    if z + k + 1 < d {
                        acc += tmp[(z + k + 1) * w + x];
                    }
                    if z >= k {
                        acc -= tmp[(z - k) * w + x];
                    }
                }
            }
        }
        sum.iter().zip(&count).map(|(s, c)| if *c > 0.0 { s / c } else { f32::NAN }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slopes_follow_the_hillside_not_the_steps() {
        // A staircase of one block every two: 26.6° on the whole, though
        // every step is either flat or a vertical block.
        let stairs = |dx: i32, _dz: i32| Some((dx.div_euclid(2)) as f32);
        let s = trend_slope(stairs, SLOPE_REACH, 1.0);
        assert!((s - 26.57).abs() < 3.0, "{s}");
        // Flat ground, and a 45° plane, whatever the spacing.
        assert_eq!(trend_slope(|_, _| Some(64.0), 4, 1.0), 0.0);
        let plane = |dx: i32, dz: i32| Some((dx + dz) as f32 * 4.0);
        let s = trend_slope(plane, 2, 4.0);
        assert!((s - 54.74).abs() < 0.1, "{s}");
        // Off the map counts as level.
        assert_eq!(trend_slope(|dx, dz| (dx == 0 && dz == 0).then_some(10.0), 3, 1.0), 0.0);
    }
    use crate::tile::Tile;
    use crate::world::DimensionKind;

    fn column(height: f32) -> Column {
        Column {
            height,
            terrain: 1,
            biome: 0,
            wet: false,
            slope: 10.0,
            relief: 0.0,
            aspect: None,
        }
    }

    #[test]
    fn slopes_face_the_way_water_runs() {
        // Rising toward the south: the hillside faces north.
        assert_eq!(aspect_of(0.0, 1.0), Some(0.0));
        // Rising toward the east: it faces west.
        assert_eq!(aspect_of(1.0, 0.0), Some(270.0));
        assert_eq!(aspect_of(-1.0, 0.0), Some(90.0));
        assert_eq!(aspect_of(0.0, -1.0), Some(180.0));
        assert_eq!(aspect_of(0.0, 0.0), None);
        let south = Filter {
            facing_on: true,
            facing: 180.0,
            facing_spread: 45.0,
            ..Filter::default()
        };
        assert!(south.is_active());
        assert_eq!(south.facing_weight(30.0, Some(180.0)), 1.0);
        assert_eq!(south.facing_weight(30.0, Some(200.0)), 1.0);
        assert_eq!(south.facing_weight(30.0, Some(0.0)), 0.0, "the north side");
        assert_eq!(south.facing_weight(30.0, Some(270.0)), 0.0);
        assert_eq!(south.facing_weight(1.0, Some(180.0)), 0.0, "level ground faces nowhere");
        // The limit is soft.
        let edge = south.facing_weight(30.0, Some(225.0));
        assert!((edge - 0.5).abs() < 1e-4, "{edge}");
        // Across north: 350° is 20° from 10°.
        let north = Filter {
            facing_on: true,
            facing: 10.0,
            facing_spread: 30.0,
            ..Filter::default()
        };
        assert_eq!(north.facing_weight(20.0, Some(350.0)), 1.0);
    }

    #[test]
    fn the_facing_test_reads_the_map() {
        // A pyramid: each of its four sides faces its own way.
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(60.0, 1, NO_FLUID);
        for z in 0..128i32 {
            for x in 0..128i32 {
                let d = (x - 64).abs().max((z - 64).abs());
                tile.top.set(local_index(x, z), 60.0 + (50 - d).max(0) as f32);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        let rect = BlockRect::new(0, 0, 128, 128);
        let facing = |degrees: f32| {
            Filter {
                facing_on: true,
                facing: degrees,
                facing_spread: 40.0,
                ..Filter::default()
            }
            .mask(&dim, rect)
        };
        // North is −z: the north side is above the middle on the map.
        let north = facing(0.0);
        assert_eq!(north.get(64, 34), 1.0);
        assert_eq!(north.get(64, 94), 0.0);
        assert_eq!(north.get(34, 64), 0.0);
        let east = facing(90.0);
        assert_eq!(east.get(94, 64), 1.0);
        assert_eq!(east.get(34, 64), 0.0);
        let south = facing(180.0);
        assert_eq!(south.get(64, 94), 1.0);
        // The plain around faces nowhere.
        assert_eq!(south.get(5, 120), 0.0);
    }

    #[test]
    fn the_default_filter_lets_everything_through() {
        let f = Filter::default();
        assert!(!f.is_active());
        assert_eq!(f.weight(&column(-30.0)), 1.0);
    }

    #[test]
    fn height_bands_with_and_without_fade() {
        let mut f = Filter {
            above: Some(60),
            below: Some(100),
            ..Filter::default()
        };
        assert_eq!(f.height_weight(59.0), 0.0);
        assert_eq!(f.height_weight(60.0), 1.0);
        assert_eq!(f.height_weight(100.0), 1.0);
        assert_eq!(f.height_weight(101.0), 0.0);
        f.beyond = true;
        assert_eq!(f.height_weight(80.0), 0.0);
        assert_eq!(f.height_weight(30.0), 1.0);
        assert_eq!(f.height_weight(130.0), 1.0);
        f.beyond = false;
        f.fade = true;
        assert_eq!(f.height_weight(60.0), 0.5);
        assert!(f.height_weight(57.0) < 0.01);
        assert!(f.height_weight(63.0) > 0.99);
        let only_above = Filter {
            above: Some(70),
            ..Filter::default()
        };
        assert_eq!(only_above.height_weight(500.0), 1.0);
        assert_eq!(only_above.height_weight(69.9), 0.0);
    }

    #[test]
    fn only_on_except_on_slope_and_relief() {
        let mut c = column(64.0);
        let only_grass = Filter {
            only_on: Some(vec![Match::Terrain(1), Match::Water]),
            ..Filter::default()
        };
        assert_eq!(only_grass.weight(&c), 1.0);
        c.terrain = 5;
        assert_eq!(only_grass.weight(&c), 0.0);
        c.wet = true;
        assert_eq!(only_grass.weight(&c), 1.0);
        let not_wet = Filter {
            except_on: Some(vec![Match::Water]),
            ..Filter::default()
        };
        assert_eq!(not_wet.weight(&c), 0.0);
        c.wet = false;
        let steep = Filter {
            slope_above: true,
            slope: 30.0,
            ..Filter::default()
        };
        assert_eq!(steep.weight(&c), 0.0);
        c.slope = 40.0;
        assert_eq!(steep.weight(&c), 1.0);
        let hollows = Filter {
            hollows: true,
            depth: 2.0,
            ..Filter::default()
        };
        c.relief = -3.0;
        assert_eq!(hollows.weight(&c), 1.0);
        c.relief = 3.0;
        assert_eq!(hollows.weight(&c), 0.0);
    }

    #[test]
    fn masks_read_the_map() {
        // A tile at 60 with a 10-block-high square plateau in it, half of it
        // under water.
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(60.0, 1, NO_FLUID);
        for z in 40..60 {
            for x in 40..60 {
                tile.top.set(local_index(x, z), 70.0);
            }
        }
        for z in 0..128 {
            for x in 0..20 {
                tile.fluid.set(local_index(x, z), 63);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        let rect = BlockRect::new(0, 0, 128, 128);

        let high = Filter {
            above: Some(65),
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(high.get(50, 50), 1.0);
        assert_eq!(high.get(10, 10), 0.0);
        assert_eq!(high.get(500, 10), 0.0, "outside the mask");

        let dry = Filter {
            except_on: Some(vec![Match::Water]),
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(dry.get(5, 5), 0.0);
        assert_eq!(dry.get(30, 5), 1.0);

        // The plateau's edge is steep, its middle and the plain are flat.
        let steep = Filter {
            slope_above: true,
            slope: 30.0,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(steep.get(40, 50), 1.0);
        assert_eq!(steep.get(50, 50), 0.0);
        assert_eq!(steep.get(90, 90), 0.0);

        // Next to the plateau, the plain is a hollow; on its rim, a bump.
        let hollows = Filter {
            hollows: true,
            depth: 1.0,
            radius: 4,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(hollows.get(38, 50), 1.0);
        assert_eq!(hollows.get(90, 90), 0.0);
        let bumps = Filter {
            bumps: true,
            depth: 1.0,
            radius: 4,
            ..Filter::default()
        }
        .mask(&dim, rect);
        assert_eq!(bumps.get(40, 50), 1.0);
        assert_eq!(bumps.get(50, 50), 0.0);
    }
}
