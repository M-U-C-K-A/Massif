//! Climate: temperature, wind, rainfall, humidity, and the biomes they
//! make.
//!
//! Temperatures travel as masks from 0 (−30 °C) to 1 (+45 °C), as the Map
//! Output takes them; nodes give degrees as numbers too.

use crate::filters::{self, gradient};
use crate::kernel::{Ctx, Footprint, Grid};
use crate::noise::Fbm;
use crate::params::Params;
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use rayon::prelude::*;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&TEMPERATURE, &WIND, &RAINFALL, &HUMIDITY, &BIOMES];

/// Degrees to the 0–1 of a temperature mask, and back.
pub fn to_mask(celsius: f32) -> f32 {
    ((celsius + 30.0) / 75.0).clamp(0.0, 1.0)
}

pub fn to_celsius(mask: f32) -> f32 {
    mask * 75.0 - 30.0
}

const HEIGHT_OPT: PortSpec = PortSpec::new("height", "Height", PortKind::Height, "The relief (flat at the sea level when unconnected).").optional();

// ----------------------------------------------------------- temperature --

pub static TEMPERATURE: NodeSpec = NodeSpec {
    id: "climate.temperature",
    label: "Temperature",
    family: Family::Climate,
    summary: "How warm each place is: warmer toward one side of the terrain, colder with height, milder by the sea.",
    help: "Toward is the warm side (90 for the south, as in the northern hemisphere); the temperature goes from Cold on the far side to Warm on that one. Lapse takes degrees off for every 100 blocks over the sea, so that summits freeze; Sea moderation pulls the sea toward a mild 12 °C. Variation adds slow warmer and colder patches. Feed Biomes with it, and the Map Output's Temperature.",
    keywords: &["temperature", "heat", "climate", "latitude", "cold", "warm", "lapse"],
    inputs: &[HEIGHT_OPT],
    outputs: &[PortSpec::new("temperature", "Temperature", PortKind::Mask, "0 for −30 °C, 1 for +45 °C."), PortSpec::new("celsius", "Degrees", PortKind::Height, "The temperature in °C.")],
    params: &[
        ParamSpec::float("toward", "Toward", Unit::Degrees, 90.0, -360.0, 360.0, "The warm side, in degrees (0 east, 90 south)."),
        ParamSpec::float("warm", "Warm", Unit::Celsius, 28.0, -30.0, 45.0, "The temperature at the warm edge, in °C.").drivable(),
        ParamSpec::float("cold", "Cold", Unit::Celsius, -8.0, -30.0, 45.0, "The temperature at the cold edge, in °C.").drivable(),
        ParamSpec::float("lapse", "Lapse", Unit::Celsius, 5.0, 0.0, 30.0, "How many degrees colder for every 100 blocks over the sea."),
        ParamSpec::float("sea_moderation", "Sea moderation", Unit::Ratio, 0.3, 0.0, 1.0, "How much the sea pulls toward a mild 12 °C.").section(Section::Advanced),
        ParamSpec::float("variation", "Variation", Unit::Celsius, 2.0, 0.0, 20.0, "How much slow warmer and colder patches vary it, in °C.").section(Section::Detail),
        ParamSpec::float("variation_scale", "Variation scale", Unit::Blocks, 3000.0, 32.0, 65536.0, "The size of those patches, in blocks.").soft(200.0, 10000.0).section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: temperature_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn temperature_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let sea = ctx.space.sea_level;
    let (s, c) = p.num("toward").to_radians().sin_cos();
    let (lapse, moderate, var) = (p.f("lapse"), p.f("sea_moderation"), p.f("variation"));
    let patches = Fbm::new(p.num("variation_scale"), 3, 0.5, ctx.seed);
    let w = ctx.rect.w as usize;
    for k in 0..ctx.rect.area() {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let t = (0.5 + ((x - size * 0.5) * c + (z - size * 0.5) * s) / size).clamp(0.0, 1.0) as f32;
        let (cold, warm) = (ctx.num_at("cold", k), ctx.num_at("warm", k));
        let mut deg = cold + (warm - cold) * t + var * patches.at(x, z);
        let h = inputs[0].map_or(sea, |h| h[k]);
        if h >= sea {
            deg -= lapse * (h - sea) / 100.0;
        } else {
            deg += (12.0 - deg) * moderate;
        }
        out[0][k] = to_mask(deg);
        out[1][k] = deg;
    }
}

// ------------------------------------------------------------------ wind --

pub static WIND: NodeSpec = NodeSpec {
    id: "climate.wind",
    label: "Wind",
    family: Family::Climate,
    summary: "The prevailing wind over the terrain, bent around high ground.",
    help: "Direction is where the wind blows to (0 east, 90 south): the westerlies of Europe blow to the east. Deflection is how much the relief turns it aside instead of letting it climb. The output is a direction and a strength at each place, for dunes, rain and wind erosion.",
    keywords: &["wind", "breeze", "prevailing", "westerlies", "airflow", "vector"],
    inputs: &[HEIGHT_OPT],
    outputs: &[PortSpec::new("wind", "Wind", PortKind::Vector, "Where the wind blows, and how strongly.")],
    params: &[
        ParamSpec::float("direction", "Direction", Unit::Degrees, 0.0, -360.0, 360.0, "Where the wind blows to, in degrees (0 east, 90 south)."),
        ParamSpec::float("strength", "Strength", Unit::Ratio, 1.0, 0.0, 10.0, "How strong it is."),
        ParamSpec::float("deflection", "Deflection", Unit::Ratio, 0.5, 0.0, 1.0, "How much the relief turns it aside."),
    ],
    kernel: Kernel::Local { cpu: wind_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: 1 } },
    ..NodeSpec::BASE
};

fn wind_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let (s, c) = p.f("direction").to_radians().sin_cos();
    let (strength, deflect) = (p.f("strength"), p.f("deflection"));
    let n = ctx.rect.area();
    let w = ctx.rect.w as usize;
    for k in 0..n {
        let (mut wx, mut wz) = (c, s);
        if let Some(h) = inputs[0] {
            let (gx, gz) = gradient(h, ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
            let g = (gx * gx + gz * gz).sqrt();
            if g > 1e-6 {
                // Take away part of the climbing component: the air goes
                // round rather than over.
                let (ux, uz) = (gx / g, gz / g);
                let climb = wx * ux + wz * uz;
                let k2 = deflect * (g * 3.0).min(1.0);
                wx -= k2 * climb * ux;
                wz -= k2 * climb * uz;
                let l = (wx * wx + wz * wz).sqrt().max(1e-6);
                wx /= l;
                wz /= l;
            }
        }
        out[0][k] = wx * strength;
        out[0][n + k] = wz * strength;
    }
}

// -------------------------------------------------------------- rainfall --

pub static RAINFALL: NodeSpec = NodeSpec {
    id: "climate.rainfall",
    label: "Rainfall",
    family: Family::Climate,
    summary: "Where rain falls: air carries moisture from the sea downwind and loses it as it rises, so that slopes facing the wind are wet and the land behind mountains dry.",
    help: "Direction is where the wind blows to. Moisture is how wet the air comes in; Orographic how much rain rising air sheds (high: deep rain shadows behind ranges); Refill how quickly the air takes moisture back over the sea; Drying how quickly it rains out over flat land. Worked over the whole terrain at the simulations' resolution; the result is from 0 (driest) to 1 (wettest).",
    keywords: &["rain", "precipitation", "orographic", "rain shadow", "wet", "dry", "monsoon"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the air crosses.")],
    outputs: &[PortSpec::new("rain", "Rain", PortKind::Mask, "How much rain falls, from 0 to 1.")],
    params: &[
        ParamSpec::float("direction", "Direction", Unit::Degrees, 0.0, -360.0, 360.0, "Where the wind blows to, in degrees (0 east, 90 south)."),
        ParamSpec::float("moisture", "Moisture", Unit::Ratio, 1.0, 0.0, 4.0, "How wet the air comes in."),
        ParamSpec::float("orographic", "Orographic", Unit::Ratio, 1.0, 0.0, 10.0, "How much rain rising air sheds."),
        ParamSpec::float("refill", "Refill", Unit::Ratio, 0.5, 0.0, 10.0, "How quickly the air takes moisture back over the sea.").section(Section::Advanced),
        ParamSpec::float("drying", "Drying", Unit::Ratio, 0.5, 0.0, 10.0, "How quickly it rains out over flat land.").section(Section::Advanced),
    ],
    kernel: Kernel::Global { cpu: rainfall_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

fn rainfall_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let h = inputs[0].expect("rainfall has its relief");
    let p = ctx.params;
    let n = h.rect.w as usize;
    let spacing = ctx.spacing() as f32;
    let sea = ctx.space.sea_level;
    let (s, c) = p.f("direction").to_radians().sin_cos();
    let m0 = p.f("moisture");
    // Rates per block, scaled so that the defaults make sense on a map of
    // Minecraft's heights.
    let (oro, refill, dry) = (p.f("orographic") * 0.08, p.f("refill") / 400.0, p.f("drying") / 4000.0);
    // Samples by their place along the wind: each finds the air upwind
    // already done.
    let mut order: Vec<u32> = (0..(n * n) as u32).collect();
    let along = |k: u32| (k as usize % n) as f32 * c + (k as usize / n) as f32 * s;
    order.sort_by(|a, b| along(*a).total_cmp(&along(*b)));
    let mut moist = vec![0.0f32; n * n];
    let mut rain = vec![0.0f32; n * n];
    // Two samples upwind: the four samples around that point are all done.
    let step = 2.0f32;
    let dist = step * spacing;
    for (i, &k) in order.iter().enumerate() {
        if i % 65536 == 0 && cancel() {
            return None;
        }
        let k = k as usize;
        let (x, z) = ((k % n) as f32, (k / n) as f32);
        let (ux, uz) = (x - c * step, z - s * step);
        let inside = ux >= 0.0 && uz >= 0.0 && ux <= (n - 1) as f32 && uz <= (n - 1) as f32;
        let (m_up, h_up) = if inside {
            let g = |data: &[f32]| {
                let (fx, fz) = (ux.floor(), uz.floor());
                let (i0, j0) = (fx as usize, fz as usize);
                let (i1, j1) = ((i0 + 1).min(n - 1), (j0 + 1).min(n - 1));
                let (tx, tz) = (ux - fx, uz - fz);
                let a = data[j0 * n + i0] + (data[j0 * n + i1] - data[j0 * n + i0]) * tx;
                let b = data[j1 * n + i0] + (data[j1 * n + i1] - data[j1 * n + i0]) * tx;
                a + (b - a) * tz
            };
            (g(&moist), g(&h.data))
        } else {
            (m0, h.data[k])
        };
        let here = h.data[k];
        if here < sea {
            // The sea gives moisture back.
            moist[k] = m_up + (m0 - m_up) * (1.0 - (-refill * dist).exp());
            rain[k] = m_up * dry * 0.5;
        } else {
            let uplift = ((here - h_up.max(sea)) / dist).max(0.0);
            let shed = 1.0 - (-(dry + oro * uplift) * dist).exp();
            let r = m_up * shed;
            moist[k] = m_up - r;
            rain[k] = r / dist;
        }
    }
    // From 0 to 1, the wettest hundredth of land at 1.
    let mut sorted: Vec<f32> = rain.iter().zip(&h.data).filter(|(_, hh)| **hh >= sea).map(|(r, _)| *r).collect();
    let top = if sorted.is_empty() {
        1.0
    } else {
        let at = ((sorted.len() as f32 * 0.99) as usize).min(sorted.len() - 1);
        let (_, v, _) = sorted.select_nth_unstable_by(at, |a, b| a.total_cmp(b));
        v.max(1e-9)
    };
    Some(vec![rain.iter().map(|r| (r / top).clamp(0.0, 1.0)).collect()])
}

// -------------------------------------------------------------- humidity --

pub static HUMIDITY: NodeSpec = NodeSpec {
    id: "climate.humidity",
    label: "Humidity",
    family: Family::Climate,
    summary: "How humid each place is: damp by the sea, drying inland, wetter where rain falls.",
    help: "Inland is how far from the sea the air stays damp. Connect Rainfall for rain shadows and wet windward slopes; Rain weight is how much it counts against the sea's nearness. Variation adds slow patches; Irregularity lets the damp band by the coast widen and narrow along it, so that forests do not follow the shore at one width. Feed Biomes with it, and the Map Output's Humidity.",
    keywords: &["humidity", "moisture", "damp", "dry", "climate", "continentality"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief: where it is under the sea level is sea."), PortSpec::new("rainfall", "Rainfall", PortKind::Mask, "How much rain falls (the sea's nearness alone, when unconnected).").optional()],
    outputs: &[PortSpec::new("humidity", "Humidity", PortKind::Mask, "From 0 (dry) to 1 (humid).")],
    params: &[
        ParamSpec::float("inland", "Inland", Unit::Blocks, 1500.0, 1.0, 65536.0, "How far from the sea the air stays damp, in blocks.").soft(100.0, 8000.0),
        ParamSpec::float("rain_weight", "Rain weight", Unit::Ratio, 0.6, 0.0, 1.0, "How much rainfall counts against the sea's nearness."),
        ParamSpec::float("variation", "Variation", Unit::Ratio, 0.1, 0.0, 1.0, "How much slow patches vary it.").section(Section::Detail),
        ParamSpec::float("irregularity", "Irregularity", Unit::Ratio, 0.5, 0.0, 1.0, "How much the damp coastal band widens and narrows along the coast instead of keeping one width.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Global { cpu: humidity_cpu, gpu: None, carry: &[None] },
    ..NodeSpec::BASE
};

fn humidity_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let h = inputs[0].expect("humidity has its relief");
    let p = ctx.params;
    let n = h.rect.w as usize;
    let sea = ctx.space.sea_level;
    let spacing = ctx.spacing() as f32;
    let d = filters::distance(n, h.rect.h as usize, |k| h.data[k] < sea);
    let inland = p.f("inland");
    let weight = if inputs[1].is_some() { p.f("rain_weight") } else { 0.0 };
    let var = p.f("variation");
    let irregular = p.f("irregularity");
    let patches = Fbm::new(inland as f64 * 2.0, 3, 0.5, ctx.seed);
    // The band's reach varies along the coast, in broad swells and a ragged
    // edge; scaling the reach keeps the shore itself damp.
    let swell = Fbm::new(inland as f64 * 2.5, 3, 0.5, ctx.seed ^ 0x2b7e);
    let ragged = Fbm::new(inland as f64 * 0.5, 4, 0.6, ctx.seed ^ 0x9d1c);
    let out = (0..n * h.rect.h as usize)
        .into_par_iter()
        .map(|k| {
            let (x, z) = ctx.world(h.rect.x + (k % n) as i32, h.rect.z + (k / n) as i32);
            let near = if d[k] == f32::MAX {
                0.0
            } else {
                let wide = if irregular > 0.0 { (irregular * (2.4 * swell.at(x, z) + 2.0 * ragged.at(x, z))).exp() } else { 1.0 };
                (-(d[k] * spacing) / (inland * wide)).exp()
            };
            let rain = inputs[1].map_or(0.0, |r| r.data[k]);
            ((1.0 - weight) * near + weight * rain + var * patches.at(x, z)).clamp(0.0, 1.0)
        })
        .collect();
    Some(vec![out])
}

// ---------------------------------------------------------------- biomes --

/// A biome: its id in the game, and the colour maps show it in.
pub struct Biome {
    pub id: &'static str,
    pub color: [u8; 3],
}

macro_rules! biomes {
    ($($name:ident = $id:literal, [$r:literal, $g:literal, $b:literal];)*) => {
        #[allow(non_camel_case_types, dead_code)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(usize)]
        pub enum B { $($name),* }
        pub static TABLE: &[Biome] = &[$(Biome { id: $id, color: [$r, $g, $b] }),*];
    };
}

biomes! {
    ocean = "minecraft:ocean", [0, 0, 112];
    deep_ocean = "minecraft:deep_ocean", [0, 0, 48];
    frozen_ocean = "minecraft:frozen_ocean", [112, 112, 214];
    deep_frozen_ocean = "minecraft:deep_frozen_ocean", [64, 64, 144];
    cold_ocean = "minecraft:cold_ocean", [32, 32, 112];
    deep_cold_ocean = "minecraft:deep_cold_ocean", [32, 32, 56];
    lukewarm_ocean = "minecraft:lukewarm_ocean", [0, 0, 144];
    deep_lukewarm_ocean = "minecraft:deep_lukewarm_ocean", [0, 0, 64];
    warm_ocean = "minecraft:warm_ocean", [0, 0, 172];
    river = "minecraft:river", [0, 0, 255];
    frozen_river = "minecraft:frozen_river", [160, 160, 255];
    beach = "minecraft:beach", [250, 222, 85];
    snowy_beach = "minecraft:snowy_beach", [250, 240, 192];
    plains = "minecraft:plains", [141, 179, 96];
    snowy_plains = "minecraft:snowy_plains", [255, 255, 255];
    desert = "minecraft:desert", [250, 148, 24];
    swamp = "minecraft:swamp", [7, 249, 178];
    forest = "minecraft:forest", [5, 102, 33];
    birch_forest = "minecraft:birch_forest", [48, 116, 68];
    dark_forest = "minecraft:dark_forest", [64, 81, 26];
    taiga = "minecraft:taiga", [11, 102, 89];
    snowy_taiga = "minecraft:snowy_taiga", [49, 85, 74];
    old_growth_spruce_taiga = "minecraft:old_growth_spruce_taiga", [129, 142, 121];
    savanna = "minecraft:savanna", [189, 178, 95];
    badlands = "minecraft:badlands", [217, 69, 21];
    jungle = "minecraft:jungle", [83, 123, 9];
    sparse_jungle = "minecraft:sparse_jungle", [98, 139, 23];
    meadow = "minecraft:meadow", [96, 164, 69];
    grove = "minecraft:grove", [196, 214, 212];
    snowy_slopes = "minecraft:snowy_slopes", [220, 220, 220];
    frozen_peaks = "minecraft:frozen_peaks", [160, 160, 200];
    jagged_peaks = "minecraft:jagged_peaks", [230, 230, 230];
    stony_peaks = "minecraft:stony_peaks", [123, 138, 128];
}

/// The biome of a place: its temperature (°C), humidity (0–1), height
/// over the sea (blocks; `None` when unknown), whether water lies on it.
pub fn classify(t: f32, humidity: f32, over_sea: Option<f32>, water: bool, peaks: f32, beach: f32) -> B {
    if let Some(h) = over_sea {
        if h < -0.5 {
            let deep = h < -20.0;
            return match (t, deep) {
                (t, false) if t < -2.0 => B::frozen_ocean,
                (t, true) if t < -2.0 => B::deep_frozen_ocean,
                (t, false) if t < 8.0 => B::cold_ocean,
                (t, true) if t < 8.0 => B::deep_cold_ocean,
                (t, false) if t < 18.0 => B::ocean,
                (t, true) if t < 18.0 => B::deep_ocean,
                (t, false) if t < 24.0 => B::lukewarm_ocean,
                (t, true) if t < 24.0 => B::deep_lukewarm_ocean,
                _ => B::warm_ocean,
            };
        }
        if water {
            return if t < 0.0 { B::frozen_river } else { B::river };
        }
        if h <= beach {
            return if t < 0.0 { B::snowy_beach } else { B::beach };
        }
        if h > peaks {
            return if t < -2.0 { B::frozen_peaks } else if t < 3.0 { B::jagged_peaks } else { B::stony_peaks };
        }
        if h > peaks - 40.0 {
            return if t < 2.0 { B::snowy_slopes } else if t < 8.0 { B::grove } else { B::meadow };
        }
    }
    let wet = humidity;
    let low = over_sea.is_some_and(|h| h < 12.0);
    match t {
        t if t < -5.0 => {
            if wet < 0.5 {
                B::snowy_plains
            } else {
                B::snowy_taiga
            }
        }
        t if t < 6.0 => {
            if wet < 0.3 {
                B::plains
            } else if wet < 0.7 {
                B::taiga
            } else {
                B::old_growth_spruce_taiga
            }
        }
        t if t < 16.0 => {
            if wet < 0.3 {
                B::plains
            } else if wet < 0.55 {
                B::forest
            } else if wet < 0.7 {
                B::birch_forest
            } else {
                B::dark_forest
            }
        }
        t if t < 24.0 => {
            if wet < 0.25 {
                B::savanna
            } else if wet < 0.45 {
                B::plains
            } else if wet > 0.7 && low {
                B::swamp
            } else {
                B::forest
            }
        }
        _ => {
            let high = over_sea.is_some_and(|h| h > 60.0);
            if wet < 0.2 {
                if high { B::badlands } else { B::desert }
            } else if wet < 0.45 {
                B::savanna
            } else if wet < 0.6 {
                B::sparse_jungle
            } else {
                B::jungle
            }
        }
    }
}

pub static BIOMES: NodeSpec = NodeSpec {
    id: "climate.biomes",
    label: "Biomes",
    family: Family::Climate,
    summary: "The game's biomes from the climate: oceans by their warmth, beaches, rivers, peaks and meadows by height, and on land a table of temperature against humidity.",
    help: "Connect Temperature and Humidity (and Height, Water for oceans, beaches, rivers and peaks). Above Peaks blocks over the sea lie frozen, jagged or stony peaks, meadows and groves just under them; within Beach blocks of the sea level, beaches. On land: snowy plains and taiga in the cold, plains and forests in the mild, savanna and swamp in the warm, desert, badlands and jungle in the hot. The Biome output goes to the Map Output; the Colour output shows them as maps do.",
    keywords: &["biome", "whittaker", "climate", "taiga", "desert", "jungle", "forest", "minecraft"],
    inputs: &[
        PortSpec::new("temperature", "Temperature", PortKind::Mask, "The temperature, as a Temperature node gives it."),
        PortSpec::new("humidity", "Humidity", PortKind::Mask, "The humidity, from 0 to 1."),
        PortSpec::new("height", "Height", PortKind::Height, "The relief, for oceans, beaches and peaks (land everywhere when unconnected).").optional(),
        PortSpec::new("water", "Water", PortKind::Height, "The water's surface, for rivers and lakes.").optional(),
    ],
    outputs: &[PortSpec::new("biome", "Biome", PortKind::Biome, "The biome of each place."), PortSpec::new("color", "Colour", PortKind::Color, "Each biome's colour, as maps show it.")],
    params: &[
        ParamSpec::float("peaks", "Peaks", Unit::Blocks, 170.0, 0.0, 4096.0, "How high over the sea the peaks begin, in blocks.").soft(0.0, 300.0),
        ParamSpec::float("beach", "Beach", Unit::Blocks, 3.0, 0.0, 64.0, "How high over the sea beaches reach, in blocks."),
        ParamSpec::float("wiggle", "Wiggle", Unit::Ratio, 0.5, 0.0, 1.0, "How much the borders between biomes wander instead of following the climate's smooth lines.").section(Section::Detail),
        ParamSpec::float("wiggle_scale", "Wiggle scale", Unit::Blocks, 300.0, 16.0, 16000.0, "The size of the borders' wanderings, in blocks.").soft(50.0, 2000.0).section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: biomes_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn biomes_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let ids: Vec<f32> = TABLE.iter().map(|b| ctx.env.biome_id(b.id).unwrap_or(0) as f32).collect();
    let colors: Vec<[f32; 3]> = TABLE.iter().map(|b| crate::params::linear(b.color.map(|c| c as f32 / 255.0))).collect();
    let (t, hum) = (inputs[0].expect("biomes have a temperature"), inputs[1].expect("biomes have a humidity"));
    let sea = ctx.space.sea_level;
    let (peaks, beach) = (p.f("peaks"), p.f("beach"));
    let wiggle = p.f("wiggle");
    let (dt, dh) = (Fbm::new(p.num("wiggle_scale"), 3, 0.55, ctx.seed), Fbm::new(p.num("wiggle_scale"), 3, 0.55, ctx.seed ^ 0x5bd1));
    let n = ctx.rect.area();
    let w = ctx.rect.w as usize;
    for k in 0..n {
        let over = inputs[2].map(|h| h[k] - sea);
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let (jt, jh) = if wiggle > 0.0 { (wiggle * 3.0 * dt.at(x, z), wiggle * 0.08 * dh.at(x, z)) } else { (0.0, 0.0) };
        let water = match (inputs[3], inputs[2]) {
            (Some(w), Some(h)) => w[k] > h[k] + 0.5 && h[k] >= sea - 0.5,
            _ => false,
        };
        let b = classify(to_celsius(t[k]) + jt, (hum[k] + jh).clamp(0.0, 1.0), over, water, peaks, beach) as usize;
        out[0][k] = ids[b];
        for c in 0..3 {
            out[1][c * n + k] = colors[b][c];
        }
    }
}
