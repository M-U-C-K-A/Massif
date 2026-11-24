//! Caves as the game carves them since 1.18, from a graph: a Cave Layer
//! node holds the settings of one of the map's cave layers
//! ([`massif_core::caves::CaveDef`]: chambers, tunnels, noodles, ravines,
//! roof, surface entrances, aquifers and lava) and says where they go; a
//! Cave Biome node paints lush, dripstone or deep dark caves between two
//! heights. Both give a [`PortKind::Layer`]: plane 0 a key standing for the
//! node's settings (see [`key`]), plane 1 the strength. The Map Output's
//! `caves` and `cave_biomes` inputs take them, and Apply
//! ([`crate::underground`]) turns the keys back into the map's cave layers
//! and biomes. The caves themselves are carved as the map composes its
//! columns, as painted cave layers are: nothing is stored per block.

use crate::kernel::Ctx;
use crate::params::Params;
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::util::Fnv;
use massif_core::caves::{Aquifers, CaveDef, Feature};
use std::hash::Hasher;

pub const NODES: &[&NodeSpec] = &[&LAYER, &BIOME, &MERGE];

const fn height(name: &'static str, label: &'static str, default: f64, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Blocks, default, -2048.0, 4096.0, help).soft(-64.0, 320.0)
}

const fn share(name: &'static str, label: &'static str, default: f64, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Ratio, default, 0.0, 1.0, help)
}

const fn size(name: &'static str, label: &'static str, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Ratio, 1.0, 0.5, 2.0, help).section(Section::Shape)
}

pub static LAYER: NodeSpec = NodeSpec {
    id: "caves.layer",
    label: "Cave layer",
    family: Family::Volume,
    summary: "Caves as the game carves them since 1.18 — chambers, tunnels, noodles, ravines, water and lava — where a mask says.",
    help: "One of the map's cave layers, made by the graph: its settings are the Caves tool's, and the mask says where its caves go (they thin out where it falls). Connect it to the Map Output's Caves; Apply adds or updates the layer, marked as the graph's, and the map carves it as it carves painted caves: sealed aquifers, lava under the lava floor, cave biomes and decoration on export. Several layers go through Merge cave layers.",
    keywords: &["caves", "cave", "underground", "cheese", "spaghetti", "noodle", "ravine", "aquifer", "lava", "tunnel", "cavern"],
    inputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "Where the caves go, from 0 to 1.").or("1 everywhere")],
    outputs: &[PortSpec::new("layer", "Caves", PortKind::Layer, "The cave layer, and how much of each place it takes.")],
    params: &[
        share("amount", "Amount", 1.0, "How much of the masked area takes the layer: lower, its caves are fewer and patchier.").drivable(),
        ParamSpec::flag("cheese", "Chambers", true, "Large chambers with pillars of rock standing in them (the game's cheese caves)."),
        share("cheese_density", "Chambers density", 0.5, "How much of the rock the chambers take."),
        ParamSpec::flag("spaghetti", "Tunnels", true, "Long winding tunnels (the game's spaghetti caves)."),
        share("spaghetti_density", "Tunnels density", 0.5, "How many tunnels."),
        ParamSpec::flag("noodles", "Noodles", true, "Thin tunnels, many of them."),
        share("noodles_density", "Noodles density", 0.5, "How many noodles."),
        ParamSpec::flag("ravines", "Ravines", true, "Long, deep, narrow cuts."),
        share("ravines_density", "Ravines density", 0.3, "How many ravines."),
        height("low", "Lowest", -56.0, "The lowest height the caves reach, in blocks."),
        height("high", "Highest", 319.0, "The highest height tunnels and noodles reach; the roof keeps them under the ground."),
        height("chambers_high", "Chambers top", 54.0, "The highest height chambers and ravines reach: under the sea level, as in the game."),
        ParamSpec::int("roof", "Roof", 6, 0, 64, "Blocks of rock kept over the caves under the ground's surface, save at the entrances."),
        share("openings", "Surface entrances", 0.15, "The share of the land where caves may open to the sky."),
        size("cheese_size", "Chambers size", "How large the chambers are (1: as the game carves them)."),
        size("spaghetti_size", "Tunnels size", "How wide and long the tunnels are."),
        size("noodles_size", "Noodles size", "How wide the noodles are."),
        size("ravines_size", "Ravines size", "How long and deep the ravines are."),
        ParamSpec::flag("aquifers", "Aquifers", true, "Water and lava lakes at their own levels, sealed from each other by walls of rock as the game seals them.").section(Section::Detail),
        share("aquifer_density", "Flooded share", 0.35, "The share of the rock's zones holding water or lava.").section(Section::Detail),
        ParamSpec::float("aquifer_size", "Zone size", Unit::Ratio, 1.0, 0.5, 2.0, "How large a zone of one level is (1: 48 blocks across, 24 high).").section(Section::Detail),
        height("water_high", "Highest water", 56.0, "The highest level an aquifer fills to.").section(Section::Detail),
        share("lava", "Lava share", 0.35, "The share of the flooded zones under Lava under holding lava rather than water.").section(Section::Detail),
        height("lava_under", "Lava under", 0.0, "Lakes of lava only below this height.").section(Section::Detail),
        ParamSpec::flag("lava_floor", "Lava floor", true, "Caves under Lava floor height are full of lava, whatever their zone (the game's lava under y −54).").section(Section::Detail),
        height("lava_floor_y", "Lava floor height", -54.0, "The top of the lava floor.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: layer_cpu, gpu: None },
    ..NodeSpec::BASE
};

const BIOMES: &[Choice] = &[
    Choice { value: "lush", label: "Lush caves", help: "Moss, azaleas, cave vines with glow berries, dripleaves over clay pools, spore blossoms." },
    Choice { value: "dripstone", label: "Dripstone caves", help: "Dripstone blocks, stalactites and stalagmites." },
    Choice { value: "deep_dark", label: "Deep dark", help: "Sculk, sensors, shriekers and catalysts (Minecraft 1.19 and later)." },
];

pub static BIOME: NodeSpec = NodeSpec {
    id: "caves.biome",
    label: "Cave biome",
    family: Family::Volume,
    summary: "A cave biome — lush, dripstone or deep dark — between two heights, where a mask says.",
    help: "Paints a cave biome in the rock between Lowest and Highest where the mask says, as the Cave biome tool paints it: the caves there take its decoration on export (moss and vines, dripstone, sculk). Connect it to the Map Output's Cave biomes; several go through Merge cave layers. Apply adds the biome where the graph gives it and leaves the rest of the map's cave biomes as they are.",
    keywords: &["cave", "biome", "lush", "dripstone", "deep dark", "sculk", "moss", "underground"],
    inputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "Where the biome goes, from 0 to 1.").or("1 everywhere")],
    outputs: &[PortSpec::new("layer", "Cave biome", PortKind::Layer, "The biome, and how much of each place it takes.")],
    params: &[
        ParamSpec::choice("biome", "Biome", BIOMES, "lush", "The cave biome."),
        height("low", "Lowest", -64.0, "The lowest height it fills, in blocks."),
        height("high", "Highest", 40.0, "The highest height it fills, in blocks."),
        share("amount", "Amount", 1.0, "How much of the masked area takes it.").drivable(),
    ],
    kernel: Kernel::Point { cpu: biome_cpu, gpu: None },
    ..NodeSpec::BASE
};

const MERGES: &[Choice] = &[
    Choice { value: "stronger", label: "Stronger", help: "Each place takes the stronger of the two, B where they are equal: for cave layers, and for cave biomes at the same heights." },
    Choice { value: "both", label: "Both", help: "Cave biomes only: where both are given, both are painted, each between its own heights (deep dark low down, lush caves above it)." },
];

pub static MERGE: NodeSpec = NodeSpec {
    id: "caves.merge",
    label: "Merge cave layers",
    family: Family::Volume,
    summary: "Two cave layers (or cave biomes) as one: each place takes the stronger, or, for cave biomes, both at their own heights.",
    help: "For a map with several kinds of caves (shallow tunnels everywhere, deep chambers under the mountains) or several cave biomes: chain as many as needed into the Map Output's Caves or Cave biomes. A column holds one cave layer; with Both, cave biomes at different heights stack in a column.",
    keywords: &["merge", "caves", "layers", "combine", "biomes", "union"],
    inputs: &[PortSpec::new("a", "A", PortKind::Layer, "A cave layer or cave biome.").optional(), PortSpec::new("b", "B", PortKind::Layer, "Another, of the same kind.").optional()],
    outputs: &[PortSpec::new("layer", "Layer", PortKind::Layer, "The stronger of the two at each place.")],
    params: &[ParamSpec::choice("mode", "Mode", MERGES, "stronger", "How two layers given at the same place merge.")],
    kernel: Kernel::Point { cpu: merge_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// The key a Cave Layer or Cave Biome node writes in its layer's first
/// plane: a hash of its type, settings and seed, between 1 and 2²⁴ (exact
/// in an `f32`). Apply finds the node again by it.
pub fn key(kind: &str, params: &Params, seed: u32) -> u32 {
    let mut h = Fnv::default();
    h.write(kind.as_bytes());
    params.hash_into(&mut h);
    h.write_u32(seed);
    let v = h.finish();
    ((v ^ (v >> 24) ^ (v >> 48)) as u32 & 0xFF_FFFF).max(1)
}

/// The cave layer a Cave Layer node's settings stand for, made by node
/// `node` (its id).
pub fn def_of(params: &Params, seed: u32, node: &str, name: &str) -> CaveDef {
    let f = |k: &str| params.f(k);
    let (low, high, top) = (f("low") as i32, f("high") as i32, f("chambers_high") as i32);
    let feature = |on: &str, dens: &str, size: &str, high: i32| Feature::new(params.flag(on), f(dens), f(size), low, high);
    CaveDef {
        name: name.to_owned(),
        color: [196, 120, 60],
        seed,
        cheese: feature("cheese", "cheese_density", "cheese_size", top.min(high)),
        spaghetti: feature("spaghetti", "spaghetti_density", "spaghetti_size", high),
        noodles: feature("noodles", "noodles_density", "noodles_size", high),
        ravines: feature("ravines", "ravines_density", "ravines_size", top.min(high)),
        roof: params.int("roof").clamp(0, 255) as u8,
        openings: f("openings"),
        aquifers: Aquifers {
            on: params.flag("aquifers"),
            density: f("aquifer_density"),
            size: f("aquifer_size"),
            low,
            high: f("water_high") as i32,
            lava: f("lava"),
            lava_under: f("lava_under") as i32,
            lava_floor: params.flag("lava_floor").then(|| f("lava_floor_y") as i32),
        },
        removed: false,
        locked: false,
        order: 0,
        graph: Some(node.to_owned()),
    }
}

/// The game's id of a Cave Biome node's biome.
pub fn biome_of(params: &Params) -> &'static str {
    match params.choice("biome") {
        "dripstone" => "minecraft:dripstone_caves",
        "deep_dark" => "minecraft:deep_dark",
        _ => "minecraft:lush_caves",
    }
}

/// A layer's planes from a mask and an amount.
fn fill(ctx: &Ctx, kind: &str, mask: Option<&[f32]>, out: &mut [f32]) {
    let n = ctx.rect.area();
    let k = key(kind, ctx.params, ctx.seed) as f32;
    let (keys, strengths) = out.split_at_mut(n);
    for (i, (key, s)) in keys.iter_mut().zip(strengths.iter_mut()).enumerate() {
        let v = (mask.map_or(1.0, |m| m[i]) * ctx.num_at("amount", i)).clamp(0.0, 1.0);
        *s = v;
        *key = if v > 0.0 { k } else { 0.0 };
    }
}

fn layer_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    fill(ctx, LAYER.id, inputs[0], &mut out[0]);
}

fn biome_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    fill(ctx, BIOME.id, inputs[0], &mut out[0]);
}

/// The key of two keys given together (Merge ▸ Both), between 1 and 2²⁴.
pub fn both(a: u32, b: u32) -> u32 {
    let mut h = Fnv::default();
    h.write_u32(a);
    h.write_u32(b);
    let v = h.finish();
    ((v ^ (v >> 24) ^ (v >> 48)) as u32 & 0xFF_FFFF).max(1)
}

fn merge_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let n = ctx.rect.area();
    let stack = ctx.params.choice("mode") == "both";
    let o = &mut out[0];
    for i in 0..n {
        let (ka, sa) = inputs[0].map_or((0.0, 0.0), |a| (a[i], a[n + i]));
        let (kb, sb) = inputs[1].map_or((0.0, 0.0), |b| (b[i], b[n + i]));
        let (k, s) = if stack && ka > 0.0 && kb > 0.0 {
            (both(ka as u32, kb as u32) as f32, sa.max(sb))
        } else if sb >= sa && kb > 0.0 {
            (kb, sb)
        } else if ka > 0.0 {
            (ka, sa)
        } else {
            (0.0, 0.0)
        };
        o[i] = k;
        o[n + i] = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_differ_with_settings_and_seed_and_stay_exact() {
        let mut p = Params::defaults(&LAYER, "n");
        let a = key(LAYER.id, &p, 1);
        assert_eq!(a, key(LAYER.id, &p, 1));
        assert_ne!(a, key(LAYER.id, &p, 2));
        assert_ne!(a, key(BIOME.id, &p, 1));
        p.set_num("cheese_density", 0.7);
        assert_ne!(a, key(LAYER.id, &p, 1));
        assert!((1..1 << 24).contains(&a) && (a as f32) as u32 == a);
    }

    #[test]
    fn the_default_layer_is_the_games() {
        let d = def_of(&Params::defaults(&LAYER, "n"), 5, "n", "Caves");
        let v = CaveDef::vanilla("Caves", 5, -64, 320, 62);
        assert_eq!((d.cheese.low, d.cheese.high, d.spaghetti.high, d.ravines.high), (v.cheese.low, v.cheese.high, v.spaghetti.high, d.cheese.high));
        assert_eq!((d.roof, d.openings, d.aquifers.lava_floor), (v.roof, v.openings, v.aquifers.lava_floor));
        assert_eq!(d.graph.as_deref(), Some("n"));
    }
}
