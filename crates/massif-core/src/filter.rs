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
}

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
    }

    fn needs_slope(&self) -> bool {
        self.slope_above || self.slope_below
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
        if self.needs_slope() {
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
        self.height_weight(c.height)
    }

    /// The weights of the columns of `rect` (row-major).
    pub fn mask(&self, dim: &Dimension, rect: BlockRect) -> Mask {
        let ring = if self.needs_relief() {
            self.radius.clamp(1, 64)
        } else if self.needs_slope() {
            1
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
                    let h = |dx: i32, dz: i32| surface.column(x + dx, z + dz).map_or(c.height, |n| n.height);
                    let gx = (h(1, 0) - h(-1, 0)) * 0.5;
                    let gz = (h(0, 1) - h(0, -1)) * 0.5;
                    c.slope = gx.hypot(gz).atan().to_degrees();
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
        }
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
