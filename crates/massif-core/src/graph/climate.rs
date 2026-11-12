//! Climate: how warm and how wet the land is, and what grows there.
//!
//! Temperature falls from a warm side of the map to a cold one and with
//! height; humidity comes from the sea and the lakes, is carried by the
//! wind and rains out on the slopes facing it, leaving the land behind
//! mountains dry. Together they choose a climate for every column — a
//! biome of the game, the colour it shows and the terrain it is painted
//! with.
//!
//! Both are masks from 0 to 1: temperature from −30 °C to +45 °C,
//! humidity from bone dry to soaked.

use super::{ColorField, Extent, Field};
use crate::noise::Noise;
use crate::registry::{BiomeId, Registry, TerrainId};
use rayon::prelude::*;

/// The temperature a mask of 0 stands for, and the span of the mask.
pub const COLDEST: f32 = -30.0;
pub const SPAN: f32 = 75.0;

/// °C to a temperature mask.
#[inline]
pub fn to_mask(celsius: f32) -> f32 {
    ((celsius - COLDEST) / SPAN).clamp(0.0, 1.0)
}

/// A temperature mask to °C.
#[inline]
pub fn to_celsius(mask: f32) -> f32 {
    COLDEST + mask * SPAN
}

/// Temperature over `heights`: `warm` °C at the sea's level on the side
/// the angle `toward` points to (degrees, 0 north, 90 east), `cold` °C on
/// the other, `lapse` °C less for every 100 blocks over the sea, with
/// `variation` °C of noise.
#[allow(clippy::too_many_arguments)]
pub fn temperature(heights: &Field, e: &Extent, warm: f32, cold: f32, toward: f32, lapse: f32, sea: f32, variation: f32, seed: u32) -> Field {
    let n = heights.n;
    let (s, c) = toward.to_radians().sin_cos();
    // North is −z.
    let dir = (s as f64, -c as f64);
    let noise = Noise::new(seed as u64 ^ 0x7e3a);
    let (cx, cz) = (e.x0 + e.size / 2.0, e.z0 + e.size / 2.0);
    let scale = (e.size / 5.0).max(64.0);
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|k| {
                let (x, z) = e.world(k % n, k / n);
                // −0.5 on the cold side, 0.5 on the warm one.
                let along = (((x - cx) * dir.0 + (z - cz) * dir.1) / e.size).clamp(-0.5, 0.5) as f32;
                let mut t = cold + (warm - cold) * (along + 0.5);
                t -= lapse * (heights.data[k] - sea).max(0.0) / 100.0;
                t += variation * noise.fbm(x / scale, z / scale, 4, 2.0, 0.5) as f32;
                to_mask(t)
            })
            .collect(),
    )
}

/// The most samples a side humidity is worked out at: it changes slowly
/// across the land, and is smoothed in between.
const HUMIDITY_MAX: usize = 512;

/// Humidity over `heights`: moist air from the sea (under `sea`) carried by
/// a wind blowing from the angle `from` (degrees, 0 north), drying over
/// `reach` blocks of land and raining out where the ground rises into it
/// (`shadow`: how much, from 0 to 1), over a `base` humidity, with
/// `variation` of noise.
#[allow(clippy::too_many_arguments)]
pub fn humidity(heights: &Field, e: &Extent, from: f32, base: f32, reach: f32, shadow: f32, sea: f32, variation: f32, seed: u32) -> Field {
    let n = heights.n;
    let m = n.min(HUMIDITY_MAX);
    let coarse = if m == n { heights.clone() } else { super::ops::downsample(heights, m) };
    let cell = (e.size / m as f64) as f32;
    // The wind blows toward `w`; across it, `p`. North is −z.
    let (s, c) = from.to_radians().sin_cos();
    let w = (-s, c);
    let p = (-w.1, w.0);
    // A grid turned with the wind, a line of it for every crossing of the
    // map: the air moves along each line, one cell at a time.
    let half = m as f32 * std::f32::consts::FRAC_1_SQRT_2 + 2.0;
    let lines = (2.0 * half).ceil() as usize;
    let centre = (m as f32 / 2.0 - 0.5, m as f32 / 2.0 - 0.5);
    let at = |a: f32, b: f32| (centre.0 + a * w.0 + b * p.0, centre.1 + a * w.1 + b * p.1);
    let inside = |(u, v): (f32, f32)| u >= -0.5 && v >= -0.5 && u <= m as f32 - 0.5 && v <= m as f32 - 0.5;
    let decay = (-cell / reach.max(1.0)).exp();
    // About seven tenths of the air's water rains out over a climb of a
    // hundred blocks, with a full shadow.
    let wring = 0.012 * shadow.clamp(0.0, 1.0);
    let turned: Vec<f32> = (0..lines)
        .into_par_iter()
        .flat_map_iter(|row| {
            let b = row as f32 - half;
            let mut air = base.clamp(0.0, 1.0);
            let mut last: Option<f32> = None;
            let mut out = Vec::with_capacity(lines);
            for step in 0..lines {
                let a = step as f32 - half;
                let pos = at(a, b);
                if !inside(pos) {
                    out.push(air);
                    last = None;
                    continue;
                }
                let h = coarse.sample(pos.0, pos.1);
                let wet = if h < sea {
                    // Over water, the air takes up all it can.
                    air = 1.0;
                    1.0
                } else {
                    air = base + (air - base) * decay;
                    // Ground rising into the wind wrings the rain out.
                    let rise = last.map_or(0.0, |l| (h - l).max(0.0));
                    let rain = air * (1.0 - (-wring * rise).exp());
                    air -= rain;
                    (air + rain * 4.0).min(1.0)
                };
                // The sea's surface, not its floor.
                last = Some(h.max(sea));
                out.push(wet);
            }
            out
        })
        .collect();
    let read = |a: f32, b: f32| {
        let (i, j) = (a + half, b + half);
        let (i0, j0) = (i.floor().clamp(0.0, (lines - 1) as f32) as usize, j.floor().clamp(0.0, (lines - 1) as f32) as usize);
        let (i1, j1) = ((i0 + 1).min(lines - 1), (j0 + 1).min(lines - 1));
        let (fi, fj) = ((i - i0 as f32).clamp(0.0, 1.0), (j - j0 as f32).clamp(0.0, 1.0));
        let v = |i: usize, j: usize| turned[j * lines + i];
        let top = v(i0, j0) + (v(i1, j0) - v(i0, j0)) * fi;
        let bottom = v(i0, j1) + (v(i1, j1) - v(i0, j1)) * fi;
        top + (bottom - top) * fj
    };
    let field = Field::new(
        m,
        (0..m * m)
            .into_par_iter()
            .map(|k| {
                let (du, dv) = ((k % m) as f32 - centre.0, (k / m) as f32 - centre.1);
                read(du * w.0 + dv * w.1, du * p.0 + dv * p.1)
            })
            .collect(),
    );
    // The air mixes sideways a little.
    let field = super::ops::blur(&field, (m / 96).max(1));
    let noise = Noise::new(seed as u64 ^ 0x4a1d);
    let scale = (e.size / 6.0).max(64.0);
    let up = super::ops::upsample(&field, n);
    Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|k| {
                let (x, z) = e.world(k % n, k / n);
                (up.data[k] + variation * noise.fbm(x / scale, z / scale, 4, 2.0, 0.5) as f32).clamp(0.0, 1.0)
            })
            .collect(),
    )
}

/// What the land is like, from its climate and its height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Climate {
    Tundra,
    SnowyTaiga,
    Taiga,
    Steppe,
    Plains,
    Forest,
    Swamp,
    Savanna,
    Desert,
    Badlands,
    Jungle,
    Beach,
    SnowyBeach,
    StonyPeaks,
    SnowyPeaks,
    Ocean,
    ColdOcean,
    FrozenOcean,
    WarmOcean,
}

impl Climate {
    pub const ALL: [Climate; 19] = [
        Climate::Tundra,
        Climate::SnowyTaiga,
        Climate::Taiga,
        Climate::Steppe,
        Climate::Plains,
        Climate::Forest,
        Climate::Swamp,
        Climate::Savanna,
        Climate::Desert,
        Climate::Badlands,
        Climate::Jungle,
        Climate::Beach,
        Climate::SnowyBeach,
        Climate::StonyPeaks,
        Climate::SnowyPeaks,
        Climate::Ocean,
        Climate::ColdOcean,
        Climate::FrozenOcean,
        Climate::WarmOcean,
    ];

    /// The land climates, with where they sit: °C and humidity.
    const LAND: [(Climate, f32, f32); 11] = [
        (Climate::Tundra, -12.0, 0.35),
        (Climate::SnowyTaiga, -4.0, 0.7),
        (Climate::Taiga, 4.0, 0.62),
        (Climate::Steppe, 6.0, 0.15),
        (Climate::Plains, 14.0, 0.38),
        (Climate::Forest, 13.0, 0.68),
        (Climate::Swamp, 19.0, 0.95),
        (Climate::Savanna, 25.0, 0.3),
        (Climate::Desert, 27.0, 0.06),
        (Climate::Badlands, 33.0, 0.12),
        (Climate::Jungle, 28.0, 0.82),
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Climate::Tundra => "Tundra",
            Climate::SnowyTaiga => "Snowy taiga",
            Climate::Taiga => "Taiga",
            Climate::Steppe => "Steppe",
            Climate::Plains => "Plains",
            Climate::Forest => "Forest",
            Climate::Swamp => "Swamp",
            Climate::Savanna => "Savanna",
            Climate::Desert => "Desert",
            Climate::Badlands => "Badlands",
            Climate::Jungle => "Jungle",
            Climate::Beach => "Beach",
            Climate::SnowyBeach => "Snowy beach",
            Climate::StonyPeaks => "Stony peaks",
            Climate::SnowyPeaks => "Snowy peaks",
            Climate::Ocean => "Ocean",
            Climate::ColdOcean => "Cold ocean",
            Climate::FrozenOcean => "Frozen ocean",
            Climate::WarmOcean => "Warm ocean",
        }
    }

    /// The biome of the game.
    pub const fn biome(self) -> &'static str {
        match self {
            Climate::Tundra => "minecraft:snowy_plains",
            Climate::SnowyTaiga => "minecraft:snowy_taiga",
            Climate::Taiga => "minecraft:taiga",
            Climate::Steppe => "minecraft:windswept_hills",
            Climate::Plains => "minecraft:plains",
            Climate::Forest => "minecraft:forest",
            Climate::Swamp => "minecraft:swamp",
            Climate::Savanna => "minecraft:savanna",
            Climate::Desert => "minecraft:desert",
            Climate::Badlands => "minecraft:badlands",
            Climate::Jungle => "minecraft:jungle",
            Climate::Beach => "minecraft:beach",
            Climate::SnowyBeach => "minecraft:snowy_beach",
            Climate::StonyPeaks => "minecraft:stony_peaks",
            Climate::SnowyPeaks => "minecraft:frozen_peaks",
            Climate::Ocean => "minecraft:ocean",
            Climate::ColdOcean => "minecraft:cold_ocean",
            Climate::FrozenOcean => "minecraft:frozen_ocean",
            Climate::WarmOcean => "minecraft:warm_ocean",
        }
    }

    /// The terrain its ground is painted with (a name of the registry).
    pub const fn terrain(self) -> &'static str {
        match self {
            Climate::Tundra | Climate::SnowyTaiga | Climate::SnowyPeaks | Climate::SnowyBeach => "Snow",
            // Grass, coloured by its biome.
            Climate::Taiga | Climate::Steppe | Climate::Plains | Climate::Forest | Climate::Savanna | Climate::Jungle | Climate::Swamp => "Grass",
            Climate::Desert | Climate::Beach | Climate::WarmOcean => "Sand",
            Climate::Badlands => "Red Sand",
            Climate::StonyPeaks => "Rock",
            Climate::Ocean | Climate::ColdOcean | Climate::FrozenOcean => "Gravel",
        }
    }

    /// The colour the land shows in.
    pub const fn color(self) -> [u8; 3] {
        match self {
            Climate::Tundra => [226, 232, 236],
            Climate::SnowyTaiga => [150, 168, 160],
            Climate::Taiga => [62, 98, 72],
            Climate::Steppe => [160, 152, 112],
            Climate::Plains => [124, 168, 78],
            Climate::Forest => [58, 112, 46],
            Climate::Swamp => [78, 92, 54],
            Climate::Savanna => [178, 164, 92],
            Climate::Desert => [222, 202, 150],
            Climate::Badlands => [188, 100, 52],
            Climate::Jungle => [42, 108, 30],
            Climate::Beach => [226, 212, 164],
            Climate::SnowyBeach => [236, 236, 228],
            Climate::StonyPeaks => [128, 126, 120],
            Climate::SnowyPeaks => [244, 247, 250],
            Climate::Ocean => [126, 120, 104],
            Climate::ColdOcean => [112, 112, 108],
            Climate::FrozenOcean => [150, 160, 172],
            Climate::WarmOcean => [214, 200, 156],
        }
    }

    /// The climate of a column `height` high (the sea at `sea`), at
    /// `celsius` °C and `wet` humidity.
    pub fn of(height: f32, sea: f32, celsius: f32, wet: f32) -> Climate {
        if height < sea - 0.5 {
            return match celsius {
                t if t < -2.0 => Climate::FrozenOcean,
                t if t < 8.0 => Climate::ColdOcean,
                t if t > 22.0 => Climate::WarmOcean,
                _ => Climate::Ocean,
            };
        }
        if height < sea + 2.5 && wet < 0.9 {
            return if celsius < 0.0 { Climate::SnowyBeach } else { Climate::Beach };
        }
        if height > sea + 110.0 {
            return if celsius < -6.0 { Climate::SnowyPeaks } else { Climate::StonyPeaks };
        }
        Self::land(celsius, wet)
    }

    /// The land climate closest to `celsius` °C and `wet` humidity.
    fn land(celsius: f32, wet: f32) -> Climate {
        Self::LAND
            .iter()
            .min_by(|a, b| Self::far(a, celsius, wet).total_cmp(&Self::far(b, celsius, wet)))
            .map(|l| l.0)
            .unwrap_or(Climate::Plains)
    }

    fn far(&(_, t, h): &(Climate, f32, f32), celsius: f32, wet: f32) -> f32 {
        let dt = (celsius - t) / 10.0;
        let dh = (wet - h) / 0.22;
        dt * dt + dh * dh
    }

    /// The colour of land at `celsius` °C and `wet` humidity, blended from
    /// the climates around it so that one fades into the next.
    pub fn blended(celsius: f32, wet: f32) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        let mut total = 0.0;
        for l in &Self::LAND {
            let w = (-Self::far(l, celsius, wet) * 2.2).exp();
            let c = l.0.color().map(super::color::decode);
            for k in 0..3 {
                sum[k] += c[k] * w;
            }
            total += w;
        }
        sum.map(|v| v / total.max(1e-6))
    }
}

/// The colours of climate: the land's from its temperature and humidity,
/// beaches by the sea, bare rock and snow on the peaks, sea floor under the
/// water.
pub fn colors(heights: &Field, temperature: &Field, humidity: &Field, sea: f32) -> ColorField {
    let n = heights.n;
    let sample = |f: &Field, k: usize| if f.n == n { f.data[k] } else { f.sample(((k % n) as f32 + 0.5) * f.n as f32 / n as f32 - 0.5, ((k / n) as f32 + 0.5) * f.n as f32 / n as f32 - 0.5) };
    ColorField::build(n, |k| {
        let h = heights.data[k];
        let (t, w) = (to_celsius(sample(temperature, k)), sample(humidity, k));
        let land = Climate::blended(t, w);
        let lerp = |a: [f32; 3], b: [f32; 3], s: f32| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * s.clamp(0.0, 1.0));
        let decode = |c: [u8; 3]| c.map(super::color::decode);
        let mut c = land;
        // Peaks: rock, then snow where it is cold.
        let peak = ((h - sea - 95.0) / 30.0).clamp(0.0, 1.0);
        let top = if t < -6.0 { decode(Climate::SnowyPeaks.color()) } else { decode(Climate::StonyPeaks.color()) };
        c = lerp(c, top, peak);
        // Beaches and the sea floor.
        let beach = if t < 0.0 { decode(Climate::SnowyBeach.color()) } else { decode(Climate::Beach.color()) };
        c = lerp(c, beach, 1.0 - ((h - sea - 1.0) / 2.5).clamp(0.0, 1.0));
        let floor = decode(Climate::of(sea - 1.0, sea, t, w).color());
        lerp(c, floor, ((sea - 0.5 - h) / 3.0).clamp(0.0, 1.0))
    })
}

/// What converting a graph's climate writes, by climate: its biome and the
/// terrain its ground is painted with, as the world's registry has them.
#[derive(Debug, Clone)]
pub struct ClimateTable {
    pub biomes: Vec<BiomeId>,
    pub terrains: Vec<Option<TerrainId>>,
    /// Rivers', and frozen rivers'.
    pub rivers: [BiomeId; 2],
}

impl ClimateTable {
    pub fn new(reg: &Registry) -> Self {
        let biome = |id: &str| reg.biome_by_id(id).unwrap_or(BiomeId(0));
        Self {
            biomes: Climate::ALL.iter().map(|c| biome(c.biome())).collect(),
            terrains: Climate::ALL.iter().map(|c| reg.terrain_by_name(c.terrain())).collect(),
            rivers: [biome("minecraft:river"), biome("minecraft:frozen_river")],
        }
    }

    /// The biome of a river, frozen or not.
    pub fn river(&self, frozen: bool) -> BiomeId {
        self.rivers[frozen as usize]
    }

    fn index(c: Climate) -> usize {
        Climate::ALL.iter().position(|&a| a == c).unwrap_or(0)
    }

    pub fn biome(&self, c: Climate) -> BiomeId {
        self.biomes.get(Self::index(c)).copied().unwrap_or(BiomeId(0))
    }

    pub fn terrain(&self, c: Climate) -> Option<TerrainId> {
        self.terrains.get(Self::index(c)).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(n: usize) -> Extent {
        Extent {
            x0: 0.0,
            z0: 0.0,
            size: 2048.0,
            n,
        }
    }

    #[test]
    fn warm_side_low_ground_is_warm() {
        let n = 64;
        let e = ext(n);
        // Flat at the sea, but a mountain in the middle.
        let h = Field::new(n, (0..n * n).map(|k| if (k % n).abs_diff(32) < 4 && (k / n).abs_diff(32) < 4 { 263.0 } else { 70.0 }).collect());
        let t = temperature(&h, &e, 30.0, -10.0, 180.0, 10.0, 63.0, 0.0, 1);
        let c = |i: usize, j: usize| to_celsius(t.at(i, j));
        assert!(c(10, 60) > c(10, 4) + 30.0, "the south is warm: {} vs {}", c(10, 60), c(10, 4));
        assert!(c(32, 32) < c(20, 32) - 15.0, "the mountain is cold: {} vs {}", c(32, 32), c(20, 32));
    }

    #[test]
    fn rain_falls_on_the_windward_side() {
        let n = 128;
        let e = ext(n);
        // Sea in the west, then land with a ridge across it at i = 64.
        let h = Field::new(
            n,
            (0..n * n)
                .map(|k| {
                    let i = (k % n) as f32;
                    if i < 20.0 { 40.0 } else { 70.0 + 120.0 * (-((i - 64.0) / 6.0).powi(2)).exp() }
                })
                .collect(),
        );
        // Wind from the west.
        let w = humidity(&h, &e, 270.0, 0.15, 900.0, 1.0, 63.0, 0.0, 1);
        let (windward, lee, coast) = (w.at(58, 64), w.at(80, 64), w.at(24, 64));
        assert!(coast > 0.8, "by the sea: {coast}");
        assert!(windward > lee + 0.2, "a rain shadow: {windward} against {lee}");
    }

    #[test]
    fn climates_fall_where_they_should() {
        assert_eq!(Climate::of(80.0, 63.0, 30.0, 0.05), Climate::Desert);
        assert_eq!(Climate::of(80.0, 63.0, 28.0, 0.85), Climate::Jungle);
        assert_eq!(Climate::of(80.0, 63.0, -12.0, 0.3), Climate::Tundra);
        assert_eq!(Climate::of(80.0, 63.0, 13.0, 0.7), Climate::Forest);
        assert_eq!(Climate::of(80.0, 63.0, 14.0, 0.4), Climate::Plains);
        assert_eq!(Climate::of(30.0, 63.0, 14.0, 0.4), Climate::Ocean);
        assert_eq!(Climate::of(64.0, 63.0, 14.0, 0.4), Climate::Beach);
        assert_eq!(Climate::of(200.0, 63.0, -10.0, 0.4), Climate::SnowyPeaks);
        let reg = Registry::vanilla();
        let table = ClimateTable::new(&reg);
        for c in Climate::ALL {
            assert_ne!(table.biome(c), BiomeId(0), "{} has its biome", c.label());
            assert!(table.terrain(c).is_some(), "{} has its terrain", c.label());
        }
    }
}
