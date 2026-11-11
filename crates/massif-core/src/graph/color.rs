//! Colours: gradients taken from nature, automatic colouring from the relief
//! alone, adjustments, and the palettes of real blocks colours turn into.

use super::ops::{blur, gradient as slope_at, smoothstep};
use super::{ColorField, Extent, Field};
use crate::noise::Noise;
use serde::{Deserialize, Serialize};

fn decode_exact(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn encode_exact(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// Steps of linear light the encoding table has.
const STEPS: usize = 8192;

static DECODE: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| std::array::from_fn(|c| decode_exact(c as u8)));
static ENCODE: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| (0..STEPS).map(|k| encode_exact(k as f32 / (STEPS - 1) as f32)).collect());

/// sRGB 8-bit to linear.
#[inline]
pub fn decode(c: u8) -> f32 {
    DECODE[c as usize]
}

/// Linear to sRGB 8-bit.
#[inline]
pub fn encode(v: f32) -> u8 {
    ENCODE[(v.clamp(0.0, 1.0) * (STEPS - 1) as f32 + 0.5) as usize]
}

/// Gradients of colours seen from above in nature, from low to high.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GradientPreset {
    Alpine,
    Temperate,
    Desert,
    RedRock,
    Arctic,
    Volcanic,
    Lunar,
    Martian,
    Tropical,
    Savanna,
    Tundra,
    Autumn,
    Badlands,
    Glacier,
    Coastal,
    Moss,
    Clay,
    Rock,
    Heat,
    Grey,
}

type Stops = &'static [(f32, [u8; 3])];

impl GradientPreset {
    pub const ALL: [GradientPreset; 20] = [
        GradientPreset::Alpine,
        GradientPreset::Temperate,
        GradientPreset::Desert,
        GradientPreset::RedRock,
        GradientPreset::Arctic,
        GradientPreset::Volcanic,
        GradientPreset::Lunar,
        GradientPreset::Martian,
        GradientPreset::Tropical,
        GradientPreset::Savanna,
        GradientPreset::Tundra,
        GradientPreset::Autumn,
        GradientPreset::Badlands,
        GradientPreset::Glacier,
        GradientPreset::Coastal,
        GradientPreset::Moss,
        GradientPreset::Clay,
        GradientPreset::Rock,
        GradientPreset::Heat,
        GradientPreset::Grey,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            GradientPreset::Alpine => "Alpine",
            GradientPreset::Temperate => "Temperate",
            GradientPreset::Desert => "Desert",
            GradientPreset::RedRock => "Red rock",
            GradientPreset::Arctic => "Arctic",
            GradientPreset::Volcanic => "Volcanic",
            GradientPreset::Lunar => "Lunar",
            GradientPreset::Martian => "Martian",
            GradientPreset::Tropical => "Tropical",
            GradientPreset::Savanna => "Savanna",
            GradientPreset::Tundra => "Tundra",
            GradientPreset::Autumn => "Autumn",
            GradientPreset::Badlands => "Badlands",
            GradientPreset::Glacier => "Glacier",
            GradientPreset::Coastal => "Coastal",
            GradientPreset::Moss => "Moss",
            GradientPreset::Clay => "Clay",
            GradientPreset::Rock => "Rock",
            GradientPreset::Heat => "Heat map",
            GradientPreset::Grey => "Grey",
        }
    }

    /// Positions (0..1) and sRGB colours.
    pub fn stops(self) -> Stops {
        match self {
            GradientPreset::Alpine => &[
                (0.0, [58, 82, 44]),
                (0.22, [86, 110, 56]),
                (0.4, [122, 120, 82]),
                (0.58, [128, 120, 108]),
                (0.75, [150, 146, 140]),
                (0.88, [196, 196, 200]),
                (1.0, [246, 248, 252]),
            ],
            GradientPreset::Temperate => &[
                (0.0, [46, 74, 38]),
                (0.3, [78, 112, 52]),
                (0.55, [128, 140, 76]),
                (0.75, [140, 118, 86]),
                (1.0, [156, 150, 140]),
            ],
            GradientPreset::Desert => &[
                (0.0, [150, 112, 74]),
                (0.3, [196, 156, 104]),
                (0.6, [222, 190, 136]),
                (0.85, [236, 214, 170]),
                (1.0, [214, 176, 128]),
            ],
            GradientPreset::RedRock => &[
                (0.0, [92, 46, 32]),
                (0.25, [150, 74, 44]),
                (0.45, [190, 104, 58]),
                (0.65, [210, 140, 86]),
                (0.85, [228, 186, 140]),
                (1.0, [240, 220, 190]),
            ],
            GradientPreset::Arctic => &[
                (0.0, [60, 66, 74]),
                (0.3, [110, 120, 134]),
                (0.6, [190, 200, 212]),
                (0.8, [232, 238, 244]),
                (1.0, [252, 253, 255]),
            ],
            GradientPreset::Volcanic => &[
                (0.0, [28, 26, 26]),
                (0.3, [52, 48, 46]),
                (0.55, [86, 70, 62]),
                (0.75, [120, 84, 64]),
                (0.9, [150, 140, 132]),
                (1.0, [180, 176, 170]),
            ],
            GradientPreset::Lunar => &[(0.0, [70, 72, 74]), (0.35, [112, 114, 116]), (0.7, [156, 158, 160]), (1.0, [206, 206, 204])],
            GradientPreset::Martian => &[
                (0.0, [90, 40, 24]),
                (0.3, [150, 74, 40]),
                (0.6, [196, 116, 66]),
                (0.85, [220, 160, 110]),
                (1.0, [232, 196, 160]),
            ],
            GradientPreset::Tropical => &[
                (0.0, [24, 60, 30]),
                (0.3, [40, 96, 40]),
                (0.6, [88, 140, 52]),
                (0.85, [150, 172, 80]),
                (1.0, [210, 196, 140]),
            ],
            GradientPreset::Savanna => &[
                (0.0, [110, 96, 50]),
                (0.35, [164, 144, 76]),
                (0.65, [196, 172, 98]),
                (0.85, [176, 120, 72]),
                (1.0, [150, 100, 66]),
            ],
            GradientPreset::Tundra => &[
                (0.0, [82, 76, 56]),
                (0.35, [110, 112, 78]),
                (0.6, [128, 130, 112]),
                (0.85, [190, 192, 190]),
                (1.0, [240, 242, 245]),
            ],
            GradientPreset::Autumn => &[
                (0.0, [40, 62, 32]),
                (0.3, [120, 104, 40]),
                (0.55, [176, 96, 36]),
                (0.75, [140, 60, 34]),
                (1.0, [110, 96, 86]),
            ],
            GradientPreset::Badlands => &[
                (0.0, [156, 84, 56]),
                (0.12, [196, 120, 80]),
                (0.22, [220, 184, 150]),
                (0.34, [170, 96, 64]),
                (0.46, [204, 140, 96]),
                (0.58, [226, 196, 160]),
                (0.7, [150, 80, 60]),
                (0.82, [196, 120, 84]),
                (1.0, [230, 204, 172]),
            ],
            GradientPreset::Glacier => &[
                (0.0, [52, 56, 62]),
                (0.35, [90, 104, 116]),
                (0.6, [120, 170, 190]),
                (0.8, [200, 230, 240]),
                (1.0, [250, 252, 255]),
            ],
            GradientPreset::Coastal => &[
                (0.0, [210, 196, 150]),
                (0.12, [196, 186, 140]),
                (0.3, [120, 150, 80]),
                (0.6, [70, 110, 50]),
                (1.0, [44, 78, 38]),
            ],
            GradientPreset::Moss => &[
                (0.0, [34, 50, 26]),
                (0.3, [64, 90, 40]),
                (0.6, [104, 112, 82]),
                (0.85, [140, 140, 132]),
                (1.0, [180, 180, 176]),
            ],
            GradientPreset::Clay => &[
                (0.0, [120, 72, 52]),
                (0.3, [162, 98, 70]),
                (0.6, [190, 132, 98]),
                (0.85, [210, 170, 140]),
                (1.0, [228, 204, 184]),
            ],
            GradientPreset::Rock => &[(0.0, [60, 58, 56]), (0.4, [110, 106, 100]), (0.75, [160, 156, 150]), (1.0, [210, 208, 204])],
            GradientPreset::Heat => &[
                (0.0, [40, 40, 160]),
                (0.25, [30, 160, 220]),
                (0.5, [60, 200, 90]),
                (0.75, [240, 220, 40]),
                (1.0, [230, 50, 30]),
            ],
            GradientPreset::Grey => &[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])],
        }
    }

    /// The linear colour at `t` (0..1).
    pub fn at(self, t: f32) -> [f32; 3] {
        sample(self.stops(), t)
    }
}

/// The linear colour at `t` of a list of sRGB stops.
fn sample(stops: Stops, t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let lin = |c: [u8; 3]| c.map(decode);
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if t <= b {
            let k = ((t - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
            let (x, y) = (lin(ca), lin(cb));
            return [0, 1, 2].map(|q| x[q] + (y[q] - x[q]) * k);
        }
    }
    lin(stops.last().map_or([0; 3], |s| s.1))
}

/// Colour a field with a gradient over `lo`..`hi`.
#[allow(clippy::too_many_arguments)]
pub(super) fn gradient(f: &Field, e: &Extent, preset: GradientPreset, lo: f32, hi: f32, reverse: bool, variation: f32, seed: u32) -> ColorField {
    let n = f.n;
    let span = if (hi - lo).abs() < 1e-6 { 1e-6 } else { hi - lo };
    let noise = Noise::new(seed as u64 ^ 0x6ad1);
    let v = variation.clamp(0.0, 1.0) as f64 * 0.12;
    ColorField::build(n, |c| {
        let mut t = (f.data[c] - lo) / span;
        if v > 0.0 {
            let (x, z) = e.world(c % n, c / n);
            t += (v * noise.fbm(x / 160.0, z / 160.0, 4, 2.0, 0.5)) as f32;
        }
        if reverse {
            t = 1.0 - t;
        }
        preset.at(t)
    })
}

/// Colours of a kind of land, for automatic colouring (sRGB).
struct Land {
    valley: [u8; 3],
    meadow: [u8; 3],
    upland: [u8; 3],
    rock: [u8; 3],
    cliff: [u8; 3],
    snow: [u8; 3],
}

/// Kinds of land for automatic colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoPalette {
    Temperate,
    Alpine,
    Desert,
    Arctic,
    Volcanic,
    Tropical,
    Lunar,
    Martian,
}

impl AutoPalette {
    pub const ALL: [AutoPalette; 8] = [
        AutoPalette::Temperate,
        AutoPalette::Alpine,
        AutoPalette::Desert,
        AutoPalette::Arctic,
        AutoPalette::Volcanic,
        AutoPalette::Tropical,
        AutoPalette::Lunar,
        AutoPalette::Martian,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            AutoPalette::Temperate => "Temperate",
            AutoPalette::Alpine => "Alpine",
            AutoPalette::Desert => "Desert",
            AutoPalette::Arctic => "Arctic",
            AutoPalette::Volcanic => "Volcanic",
            AutoPalette::Tropical => "Tropical",
            AutoPalette::Lunar => "Lunar",
            AutoPalette::Martian => "Martian",
        }
    }

    fn land(self) -> Land {
        let l = |valley, meadow, upland, rock, cliff, snow| Land {
            valley,
            meadow,
            upland,
            rock,
            cliff,
            snow,
        };
        match self {
            AutoPalette::Temperate => l([52, 86, 40], [88, 124, 56], [120, 128, 72], [118, 112, 104], [88, 84, 80], [240, 242, 246]),
            AutoPalette::Alpine => l([62, 92, 46], [98, 124, 62], [132, 128, 96], [128, 124, 118], [92, 90, 88], [246, 248, 252]),
            AutoPalette::Desert => l([198, 160, 110], [220, 186, 130], [206, 150, 96], [170, 104, 70], [130, 80, 56], [236, 226, 206]),
            AutoPalette::Arctic => l([214, 222, 232], [232, 238, 244], [244, 247, 251], [96, 100, 110], [70, 74, 82], [252, 252, 255]),
            AutoPalette::Volcanic => l([60, 56, 52], [82, 74, 66], [104, 92, 82], [44, 40, 40], [30, 28, 28], [200, 196, 190]),
            AutoPalette::Tropical => l([34, 80, 36], [56, 112, 44], [92, 130, 56], [110, 100, 86], [80, 72, 64], [230, 234, 240]),
            AutoPalette::Lunar => l([96, 98, 100], [120, 122, 124], [150, 150, 152], [84, 84, 86], [60, 60, 62], [200, 200, 198]),
            AutoPalette::Martian => l([150, 74, 42], [186, 104, 60], [206, 140, 90], [120, 58, 36], [90, 44, 30], [226, 206, 184]),
        }
    }
}

/// Natural colours from the relief: valleys, meadows and uplands by height,
/// rock where it is steep, darker cliffs where it is steeper still, snow
/// above the snow line where it can hold, wetter hollows, and a little
/// variation so that nothing looks flat.
pub(super) fn auto(f: &Field, e: &Extent, palette: AutoPalette, snow_line: f32, rock_slope: f32, seed: u32) -> ColorField {
    let n = f.n;
    let land = palette.land();
    let lin = |c: [u8; 3]| c.map(decode);
    let (valley, meadow, upland, rock, cliff, snow) = (lin(land.valley), lin(land.meadow), lin(land.upland), lin(land.rock), lin(land.cliff), lin(land.snow));
    let (lo, hi) = f.range();
    let span = (hi - lo).max(1e-6);
    let spacing = e.spacing();
    let mean = blur(f, (24.0 / spacing).round().max(1.0) as usize);
    let noise = Noise::new(seed as u64 ^ 0xa070);
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t.clamp(0.0, 1.0));
    ColorField::build(n, |c| {
                let (i, j) = (c % n, c / n);
                let h = f.data[c];
                let t = (h - lo) / span;
                let (gx, gz) = slope_at(f, i, j, spacing);
                let slope = gx.hypot(gz).atan().to_degrees();
                let (x, z) = e.world(i, j);
                let wobble = noise.fbm(x / 220.0, z / 220.0, 4, 2.0, 0.5) as f32;
                // Land by height.
                let mut col = if t < 0.45 {
                    mix(valley, meadow, (t + wobble * 0.08) / 0.45)
                } else {
                    mix(meadow, upland, (t - 0.45 + wobble * 0.08) / 0.55)
                };
                // Wet hollows are a little darker and greener.
                let hollow = ((mean.data[c] - h) / 3.0).clamp(0.0, 1.0);
                col = mix(col, [col[0] * 0.75, col[1] * 0.85, col[2] * 0.72], hollow * 0.6);
                // Rock on the slopes, cliffs on the steepest.
                let r = smoothstep(rock_slope - 6.0, rock_slope + 6.0, slope + wobble * 4.0);
                col = mix(col, rock, r);
                col = mix(col, cliff, smoothstep(rock_slope + 12.0, rock_slope + 26.0, slope));
                // Snow above the line, where the slope holds it.
                let s = smoothstep(snow_line - 10.0, snow_line + 10.0, h + wobble * 12.0) * (1.0 - smoothstep(46.0, 62.0, slope));
                col = mix(col, snow, s);
                // A little variation of brightness.
                let k = 1.0 + wobble * 0.08;
                col.map(|v| v * k)
    })
}

/// Hue (degrees), saturation and brightness (factors) and contrast of a
/// linear colour, worked on in sRGB as the eye sees it.
pub(super) fn adjust(c: [f32; 3], hue: f32, saturation: f32, brightness: f32, contrast: f32) -> [f32; 3] {
    let s = c.map(|v| encode(v) as f32 / 255.0);
    let (r, g, b) = (s[0], s[1], s[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let sat = if max == 0.0 { 0.0 } else { d / max };
    h = (h + hue).rem_euclid(360.0);
    let sat = (sat * saturation).clamp(0.0, 1.0);
    let val = (max * brightness).clamp(0.0, 1.0);
    let cc = val * sat;
    let x = cc * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = val - cc;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (cc, x, 0.0),
        1 => (x, cc, 0.0),
        2 => (0.0, cc, x),
        3 => (0.0, x, cc),
        4 => (x, 0.0, cc),
        _ => (cc, 0.0, x),
    };
    [r1 + m, g1 + m, b1 + m].map(|v| {
        let v = ((v - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        decode((v * 255.0).round() as u8)
    })
}

/// Sets of real blocks the colour of the ground becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BlockPalette {
    /// Soils, sands, stones, snow: what natural ground is made of.
    Natural,
    /// Stones and gravel only.
    Rock,
    /// Sands, sandstones and terracottas.
    Desert,
    /// Snow, ice and stone.
    Snowy,
    /// Every whole block the game has.
    Everything,
}

impl BlockPalette {
    pub const ALL: [BlockPalette; 5] = [
        BlockPalette::Natural,
        BlockPalette::Rock,
        BlockPalette::Desert,
        BlockPalette::Snowy,
        BlockPalette::Everything,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            BlockPalette::Natural => "Natural ground",
            BlockPalette::Rock => "Rock",
            BlockPalette::Desert => "Desert",
            BlockPalette::Snowy => "Snow and ice",
            BlockPalette::Everything => "Every block",
        }
    }

    /// The blocks, by Minecraft id; empty for every whole block.
    pub fn blocks(self) -> &'static [&'static str] {
        const NATURAL: &[&str] = &[
            "minecraft:grass_block",
            "minecraft:dirt",
            "minecraft:coarse_dirt",
            "minecraft:podzol",
            "minecraft:rooted_dirt",
            "minecraft:mud",
            "minecraft:moss_block",
            "minecraft:sand",
            "minecraft:red_sand",
            "minecraft:sandstone",
            "minecraft:gravel",
            "minecraft:stone",
            "minecraft:cobblestone",
            "minecraft:mossy_cobblestone",
            "minecraft:andesite",
            "minecraft:diorite",
            "minecraft:granite",
            "minecraft:tuff",
            "minecraft:calcite",
            "minecraft:deepslate",
            "minecraft:smooth_basalt",
            "minecraft:snow_block",
            "minecraft:packed_ice",
            "minecraft:clay",
            "minecraft:terracotta",
            "minecraft:brown_terracotta",
            "minecraft:dripstone_block",
            "minecraft:mycelium",
        ];
        const ROCK: &[&str] = &[
            "minecraft:stone",
            "minecraft:cobblestone",
            "minecraft:andesite",
            "minecraft:diorite",
            "minecraft:granite",
            "minecraft:tuff",
            "minecraft:calcite",
            "minecraft:deepslate",
            "minecraft:cobbled_deepslate",
            "minecraft:smooth_basalt",
            "minecraft:basalt",
            "minecraft:blackstone",
            "minecraft:gravel",
            "minecraft:dripstone_block",
        ];
        const DESERT: &[&str] = &[
            "minecraft:sand",
            "minecraft:red_sand",
            "minecraft:sandstone",
            "minecraft:red_sandstone",
            "minecraft:terracotta",
            "minecraft:white_terracotta",
            "minecraft:orange_terracotta",
            "minecraft:yellow_terracotta",
            "minecraft:brown_terracotta",
            "minecraft:red_terracotta",
            "minecraft:light_gray_terracotta",
            "minecraft:coarse_dirt",
            "minecraft:granite",
        ];
        const SNOWY: &[&str] = &[
            "minecraft:snow_block",
            "minecraft:powder_snow",
            "minecraft:packed_ice",
            "minecraft:blue_ice",
            "minecraft:ice",
            "minecraft:stone",
            "minecraft:andesite",
            "minecraft:calcite",
            "minecraft:gravel",
            "minecraft:deepslate",
            "minecraft:tuff",
        ];
        match self {
            BlockPalette::Natural => NATURAL,
            BlockPalette::Rock => ROCK,
            BlockPalette::Desert => DESERT,
            BlockPalette::Snowy => SNOWY,
            BlockPalette::Everything => &[],
        }
    }
}
