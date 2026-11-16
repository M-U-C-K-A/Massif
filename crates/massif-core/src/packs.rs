//! Packs: ready-made things to paint with, so that a new map can be dressed
//! without setting anything up, as WorldPainter Modernised's Packs panel
//! has them — the trees of the [tree pack](crate::treepack), presets of
//! plants (ground covers) and mixes of blocks for the ground, rock and
//! cliffs (terrains). Installing one adds it to the map's registry, or
//! finds it there if it was added before.

use crate::objects::guess_color;
use crate::registry::{CoverDef, Mix, Registry, Stratum, TerrainDef, TerrainId};

/// A preset of plants: a ground cover.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantPreset {
    pub name: &'static str,
    pub about: &'static str,
    pub family: &'static str,
    pub color: u32,
    /// Blocks and their weights.
    pub plants: &'static [(&'static str, u16)],
    /// Share of the ground where something grows.
    pub density: f32,
    /// Least and most blocks high (cane, bamboo).
    pub height: [u8; 2],
    pub underwater: bool,
    pub soil_only: bool,
    /// Frost: the water where it is painted turns to ice.
    pub frost: bool,
    /// Lava: the water where it is painted is lava.
    pub lava: bool,
}

const fn plants(name: &'static str, about: &'static str, family: &'static str, color: u32, plants: &'static [(&'static str, u16)], density: f32) -> PlantPreset {
    PlantPreset {
        name,
        about,
        family,
        color,
        plants,
        density,
        height: [1, 1],
        underwater: false,
        soil_only: true,
        frost: false,
        lava: false,
    }
}

pub const FLOWERS: &str = "Flowers";
pub const PLANTS: &str = "Plants & ground cover";
pub const WATER: &str = "Water plants";

/// The presets, by family.
pub const PLANT_PRESETS: &[PlantPreset] = &[
    plants("Wildflower meadow", "Grass full of small wildflowers, dandelions, poppies and cornflowers", FLOWERS, 0xf0d060, &[("minecraft:short_grass", 40), ("minecraft:wildflowers", 25), ("minecraft:dandelion", 4), ("minecraft:poppy", 4), ("minecraft:cornflower", 3), ("minecraft:oxeye_daisy", 3), ("minecraft:azure_bluet", 2)], 0.55),
    plants("Flower mix", "A colourful mix of flowers", FLOWERS, 0xe07ab8, &[("minecraft:dandelion", 10), ("minecraft:poppy", 10), ("minecraft:oxeye_daisy", 6), ("minecraft:cornflower", 6), ("minecraft:azure_bluet", 5), ("minecraft:allium", 3), ("minecraft:red_tulip", 3), ("minecraft:orange_tulip", 3), ("minecraft:white_tulip", 3), ("minecraft:pink_tulip", 3), ("minecraft:lilac", 2), ("minecraft:rose_bush", 2), ("minecraft:peony", 2)], 0.4),
    plants("Tulip field", "Red, orange, white and pink tulips", FLOWERS, 0xe8504a, &[("minecraft:red_tulip", 12), ("minecraft:orange_tulip", 10), ("minecraft:white_tulip", 8), ("minecraft:pink_tulip", 10), ("minecraft:short_grass", 10)], 0.5),
    plants("Sunflower field", "Sunflowers among grass", FLOWERS, 0xf5c518, &[("minecraft:sunflower", 25), ("minecraft:short_grass", 20), ("minecraft:tall_grass", 5)], 0.5),
    plants("Roses and lilacs", "Rose bushes, peonies and lilacs", FLOWERS, 0xc8305a, &[("minecraft:rose_bush", 10), ("minecraft:peony", 8), ("minecraft:lilac", 8), ("minecraft:poppy", 4)], 0.35),
    plants("Spring flowers", "Lilies of the valley, alliums and bluets", FLOWERS, 0xb8a0e8, &[("minecraft:lily_of_the_valley", 10), ("minecraft:allium", 6), ("minecraft:azure_bluet", 6), ("minecraft:oxeye_daisy", 4), ("minecraft:short_grass", 20)], 0.45),
    plants("Pink petals", "Carpets of pink petals, as under cherry trees", FLOWERS, 0xf4a6c8, &[("minecraft:pink_petals", 30), ("minecraft:short_grass", 10)], 0.5),
    plants("Swamp flowers", "Blue orchids among grass", FLOWERS, 0x3aa0e0, &[("minecraft:blue_orchid", 10), ("minecraft:short_grass", 20), ("minecraft:fern", 4)], 0.4),
    plants("Grass and flowers", "Short and tall grass with a few wild flowers", PLANTS, 0x86c45f, &[("minecraft:short_grass", 60), ("minecraft:tall_grass", 8), ("minecraft:dandelion", 3), ("minecraft:poppy", 3), ("minecraft:oxeye_daisy", 2), ("minecraft:cornflower", 2), ("minecraft:azure_bluet", 1)], 0.55),
    plants("Dense grass", "Short and tall grass, thick", PLANTS, 0x6aa84f, &[("minecraft:short_grass", 30), ("minecraft:tall_grass", 30), ("minecraft:fern", 4)], 0.75),
    plants("Forest ferns", "Ferns and grass for forest floors", PLANTS, 0x3f8f4f, &[("minecraft:fern", 20), ("minecraft:large_fern", 8), ("minecraft:short_grass", 10)], 0.45),
    plants("Forest floor", "Leaf litter, moss, ferns and bushes", PLANTS, 0x7a6a3a, &[("minecraft:leaf_litter", 20), ("minecraft:fern", 8), ("minecraft:moss_carpet", 6), ("minecraft:bush", 4), ("minecraft:brown_mushroom", 1)], 0.5),
    plants("Berry bushes", "Leafy bushes and berry bushes", PLANTS, 0x4d7a35, &[("minecraft:bush", 20), ("minecraft:sweet_berry_bush[age=3]", 6), ("minecraft:firefly_bush", 2)], 0.3),
    plants("Dry grass", "Short and tall dry grass for savannas and badlands", PLANTS, 0xc8b070, &[("minecraft:short_dry_grass", 30), ("minecraft:tall_dry_grass", 10), ("minecraft:dead_bush", 3)], 0.4),
    PlantPreset { soil_only: false, ..plants("Desert plants", "Dead bushes, cacti and dry grass, on sand", PLANTS, 0xa8a060, &[("minecraft:dead_bush", 20), ("minecraft:cactus", 6), ("minecraft:short_dry_grass", 10)], 0.06) },
    PlantPreset { height: [1, 4], ..plants("Jungle undergrowth", "Ferns, bamboo and melons", PLANTS, 0x2e8b2e, &[("minecraft:fern", 15), ("minecraft:large_fern", 8), ("minecraft:bamboo", 4), ("minecraft:melon", 1), ("minecraft:short_grass", 15)], 0.55) },
    plants("Pale garden", "Pale moss carpets and eyeblossoms", PLANTS, 0xb8bcb0, &[("minecraft:pale_moss_carpet", 20), ("minecraft:closed_eyeblossom", 5)], 0.5),
    PlantPreset { soil_only: false, frost: true, ..plants("Frost", "Snow on the ground and ice on the water, as WorldPainter's Frost", PLANTS, 0xe2eefa, &[("minecraft:snow[layers=1]", 1)], 1.0) },
    plants("Mushroom patches", "Red and brown mushrooms", PLANTS, 0xb05040, &[("minecraft:red_mushroom", 5), ("minecraft:brown_mushroom", 5)], 0.08),
    plants("Pumpkins and melons", "Pumpkin and melon patches among grass", PLANTS, 0xe08a2a, &[("minecraft:pumpkin", 6), ("minecraft:melon", 6), ("minecraft:short_grass", 20)], 0.3),
    PlantPreset { underwater: true, soil_only: false, ..plants("Seagrass and kelp", "Seagrass meadows and kelp under water", WATER, 0x2f7f5f, &[("minecraft:seagrass", 20), ("minecraft:tall_seagrass", 10), ("minecraft:kelp", 8)], 0.5) },
    PlantPreset { underwater: true, soil_only: false, ..plants("Coral garden", "Coral, coral fans and sea pickles in warm shallow water", WATER, 0xe07a8f, &[("minecraft:tube_coral", 6), ("minecraft:brain_coral", 6), ("minecraft:bubble_coral", 6), ("minecraft:fire_coral", 6), ("minecraft:horn_coral", 6), ("minecraft:tube_coral_fan", 4), ("minecraft:brain_coral_fan", 4), ("minecraft:bubble_coral_fan", 4), ("minecraft:fire_coral_fan", 4), ("minecraft:horn_coral_fan", 4), ("minecraft:sea_pickle", 3)], 0.4) },
    PlantPreset { soil_only: false, lava: true, ..plants("Lava", "The water where it is painted is lava, as WorldPainter's Lava layer", WATER, 0xe26018, &[], 1.0) },
    PlantPreset { height: [1, 3], soil_only: false, ..plants("Sugar cane", "Sugar cane for river and lake shores", WATER, 0x8ac86a, &[("minecraft:sugar_cane", 15)], 0.25) },
];

impl PlantPreset {
    pub fn color(&self) -> [u8; 3] {
        rgb(self.color)
    }

    /// Its cover in `reg`: the one of its name, or a new one. Returns its
    /// number.
    pub fn install(&self, reg: &mut Registry) -> u16 {
        if let Some(n) = reg.cover_by_name(self.name) {
            return n;
        }
        let mut def = CoverDef::new(self.name, self.color(), self.plants, self.density);
        def.height = self.height;
        def.underwater = self.underwater;
        def.soil_only = self.soil_only;
        def.frost = self.frost;
        def.lava = self.lava;
        reg.add_cover(def)
    }
}

/// How the blocks of a [`MaterialMix`] lie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lay {
    /// Each block on its own.
    Grain,
    /// In patches; the number is how large, from 0 to 1.
    Patches(f32),
    /// In beds, a cliff's strata.
    Beds,
}

/// A mix of blocks for natural ground, rock and cliffs: a terrain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialMix {
    pub name: &'static str,
    pub family: &'static str,
    pub color: u32,
    pub lay: Lay,
    /// Blocks (without `minecraft:`) and their weights; for beds, their
    /// thicknesses from the bottom up.
    pub blocks: &'static [(&'static str, u16)],
}

const fn mix(name: &'static str, family: &'static str, color: u32, lay: Lay, blocks: &'static [(&'static str, u16)]) -> MaterialMix {
    MaterialMix { name, family, color, lay, blocks }
}

pub const GRASS: &str = "Grass mixes";
pub const SOIL: &str = "Dirt & soil mixes";
pub const STONE: &str = "Stone mixes";
pub const SAND: &str = "Sand & shore mixes";
pub const CLIFFS: &str = "Cliff & strata mixes";
pub const SPECIAL: &str = "Snow, ice & volcanic mixes";
pub const BUILT: &str = "Built surfaces";

use Lay::{Beds, Grain, Patches};

/// The mixes, by family, as WorldPainter Modernised has them.
pub const MATERIAL_MIXES: &[MaterialMix] = &[
    mix("Meadow", GRASS, 0x6f9e3c, Grain, &[("grass_block", 80), ("moss_block", 10), ("coarse_dirt", 10)]),
    mix("Forest floor", GRASS, 0x4f6b2f, Patches(0.6), &[("grass_block", 45), ("podzol", 25), ("coarse_dirt", 15), ("moss_block", 15)]),
    mix("Mossy ground", GRASS, 0x5a7a32, Patches(0.5), &[("moss_block", 50), ("grass_block", 30), ("mossy_cobblestone", 10), ("coarse_dirt", 10)]),
    mix("Grass and dirt", GRASS, 0x6c8a3a, Grain, &[("grass_block", 60), ("dirt", 25), ("coarse_dirt", 15)]),
    mix("Grass and stone", GRASS, 0x6e8460, Patches(0.5), &[("grass_block", 60), ("stone", 20), ("cobblestone", 10), ("gravel", 10)]),
    mix("Lush grass", GRASS, 0x4f9a32, Patches(0.6), &[("grass_block", 70), ("moss_block", 30)]),
    mix("Dry grass", GRASS, 0x8c8a4a, Grain, &[("grass_block", 50), ("coarse_dirt", 30), ("dirt", 15), ("dirt_path", 5)]),
    mix("Mixed dirt", SOIL, 0x7a5a3c, Grain, &[("dirt", 40), ("coarse_dirt", 30), ("rooted_dirt", 15), ("podzol", 15)]),
    mix("Coarse dirt mix", SOIL, 0x6e5038, Grain, &[("coarse_dirt", 50), ("dirt", 30), ("gravel", 10), ("rooted_dirt", 10)]),
    mix("Taiga floor", SOIL, 0x5a4426, Patches(0.5), &[("podzol", 50), ("coarse_dirt", 20), ("moss_block", 15), ("dirt", 15)]),
    mix("Trodden path", SOIL, 0x947a4c, Grain, &[("dirt_path", 50), ("coarse_dirt", 30), ("gravel", 20)]),
    mix("Swamp mud", SOIL, 0x4a4232, Patches(0.5), &[("mud", 50), ("packed_mud", 15), ("muddy_mangrove_roots", 15), ("coarse_dirt", 20)]),
    mix("Mud and clay", SOIL, 0x5e5446, Patches(0.5), &[("mud", 40), ("clay", 30), ("packed_mud", 15), ("coarse_dirt", 15)]),
    mix("Alpine heath", SOIL, 0x7a4e34, Patches(0.45), &[("podzol", 30), ("coarse_dirt", 22), ("granite", 14), ("rooted_dirt", 10), ("terracotta", 8), ("grass_block", 10), ("brown_terracotta", 6)]),
    mix("Rocky ground", STONE, 0x7d7d7d, Grain, &[("stone", 40), ("andesite", 25), ("cobblestone", 15), ("gravel", 10), ("tuff", 10)]),
    mix("Limestone", STONE, 0x8e8a82, Patches(0.45), &[("stone", 35), ("andesite", 25), ("tuff", 15), ("diorite", 10), ("calcite", 10), ("cobblestone", 5)]),
    mix("Mossy rock", STONE, 0x5f7350, Patches(0.5), &[("stone", 40), ("mossy_cobblestone", 25), ("moss_block", 20), ("cobblestone", 15)]),
    mix("Alpine scree", STONE, 0x8a8a86, Patches(0.4), &[("stone", 30), ("gravel", 30), ("andesite", 20), ("cobblestone", 20)]),
    mix("Cobblestone mix", STONE, 0x7a7a7a, Grain, &[("cobblestone", 40), ("mossy_cobblestone", 20), ("stone", 20), ("gravel", 20)]),
    mix("Andesite mix", STONE, 0x888888, Patches(0.45), &[("andesite", 50), ("stone", 30), ("cobblestone", 10), ("gravel", 10)]),
    mix("Granite mix", STONE, 0x9a6a58, Patches(0.45), &[("granite", 50), ("andesite", 20), ("stone", 20), ("diorite", 10)]),
    mix("Deepslate mix", STONE, 0x4a4a50, Patches(0.45), &[("deepslate", 45), ("cobbled_deepslate", 25), ("tuff", 20), ("gravel", 10)]),
    mix("Gravel mix", STONE, 0x857f78, Grain, &[("gravel", 60), ("stone", 15), ("andesite", 15), ("cobblestone", 10)]),
    mix("Beach mix", SAND, 0xe2d6a0, Grain, &[("sand", 65), ("gravel", 15), ("sandstone", 10), ("clay", 10)]),
    mix("Sand and gravel", SAND, 0xb8ac8c, Patches(0.5), &[("sand", 55), ("gravel", 35), ("clay", 10)]),
    mix("Riverbed", SAND, 0x8b7f6a, Patches(0.4), &[("gravel", 40), ("clay", 20), ("sand", 20), ("mud", 20)]),
    mix("Coral reef", SAND, 0xe07a8f, Patches(0.35), &[("sand", 35), ("tube_coral_block", 13), ("brain_coral_block", 13), ("bubble_coral_block", 13), ("fire_coral_block", 13), ("horn_coral_block", 13)]),
    mix("Desert mix", SAND, 0xdcc58e, Patches(0.8), &[("sand", 65), ("sandstone", 20), ("red_sand", 15)]),
    mix("Red sand mix", SAND, 0xb8643a, Grain, &[("red_sand", 60), ("terracotta", 15), ("coarse_dirt", 15), ("red_sandstone", 10)]),
    mix("Red cliff strata", CLIFFS, 0xa0563a, Beds, &[("terracotta", 3), ("red_terracotta", 2), ("brown_terracotta", 2), ("orange_terracotta", 1), ("granite", 2), ("light_gray_terracotta", 1), ("red_terracotta", 3), ("white_terracotta", 1)]),
    mix("Grey cliff strata", CLIFFS, 0x6f6f6f, Beds, &[("stone", 4), ("andesite", 2), ("tuff", 2), ("deepslate", 1), ("stone", 3), ("calcite", 1), ("andesite", 2)]),
    mix("Sandstone strata", CLIFFS, 0xd8c48c, Beds, &[("sandstone", 4), ("smooth_sandstone", 2), ("terracotta", 1), ("sandstone", 3), ("yellow_terracotta", 1)]),
    mix("White chalk strata", CLIFFS, 0xe6e2da, Beds, &[("calcite", 4), ("diorite", 2), ("white_terracotta", 1), ("calcite", 2), ("stone", 1)]),
    mix("Badlands strata", CLIFFS, 0xc8743a, Beds, &[("orange_terracotta", 3), ("terracotta", 2), ("yellow_terracotta", 1), ("white_terracotta", 1), ("red_terracotta", 2), ("brown_terracotta", 1), ("light_gray_terracotta", 1)]),
    mix("Snowy rock", SPECIAL, 0xdfe6ec, Patches(0.5), &[("snow_block", 50), ("stone", 25), ("calcite", 15), ("packed_ice", 10)]),
    mix("Glacier", SPECIAL, 0xbcd9f0, Patches(0.6), &[("packed_ice", 40), ("snow_block", 35), ("blue_ice", 15), ("ice", 10)]),
    mix("Volcanic rock", SPECIAL, 0x3d3a3a, Patches(0.5), &[("basalt", 35), ("blackstone", 25), ("tuff", 20), ("smooth_basalt", 15), ("magma_block", 5)]),
    mix("Wooden deck", BUILT, 0x8a6a42, Grain, &[("spruce_planks", 70), ("oak_planks", 30)]),
    mix("Stone bricks", BUILT, 0x7a7a76, Grain, &[("stone_bricks", 70), ("mossy_stone_bricks", 15), ("cracked_stone_bricks", 15)]),
];

/// The families of the mixes, in order.
pub const MIX_FAMILIES: [&str; 7] = [GRASS, SOIL, STONE, SAND, CLIFFS, SPECIAL, BUILT];

/// The families of the plant presets, in order.
pub const PLANT_FAMILIES: [&str; 3] = [FLOWERS, PLANTS, WATER];

impl MaterialMix {
    pub fn color(&self) -> [u8; 3] {
        rgb(self.color)
    }

    /// The blocks and their shares, in a line.
    pub fn describe(&self) -> String {
        let total: u32 = self.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
        let what = match self.lay {
            Grain => "mixed block by block",
            Patches(_) => "in patches",
            Beds => "in beds",
        };
        let mut parts: Vec<(String, u32)> = Vec::new();
        for (b, w) in self.blocks {
            let name = b.replace('_', " ");
            match parts.iter_mut().find(|p| p.0 == name) {
                Some(p) => p.1 += *w as u32,
                None => parts.push((name, *w as u32)),
            }
        }
        let parts: Vec<String> = parts.into_iter().map(|(n, w)| format!("{n} {:.0}%", w as f32 * 100.0 / total as f32)).collect();
        format!("{}, {what}", parts.join(", "))
    }

    /// Its terrain in `reg`: the one of its name, or a new one, its blocks
    /// laid as it says, down through the land for beds. Grass has dirt
    /// under it, and is dirt under water.
    pub fn install(&self, reg: &mut Registry) -> TerrainId {
        if let Some(t) = reg.terrains.iter().position(|t| t.group == self.family && t.name == self.name) {
            return TerrainId(t as u16);
        }
        let blocks: Vec<_> = self
            .blocks
            .iter()
            .map(|(b, w)| {
                let state = format!("minecraft:{b}");
                (reg.intern_block(&state, guess_color(&state)), *w)
            })
            .collect();
        let grass = self.blocks.iter().any(|(b, _)| *b == "grass_block");
        let dirt = reg.intern_block("minecraft:dirt", [134, 96, 67]);
        let mix = match self.lay {
            Grain => Mix::Grain,
            Patches(scale) => Mix::Patches { size: 3.0 + scale * 8.0 },
            Beds => Mix::Beds,
        };
        let mut strata = vec![Stratum {
            blocks,
            depth: match (self.lay, grass) {
                (Beds, _) => (u16::MAX, u16::MAX),
                (_, true) => (1, 1),
                _ => (3, 5),
            },
            mix,
        }];
        if grass {
            strata.push(Stratum { blocks: vec![(dirt, 1)], depth: (3, 4), mix: Mix::Grain });
        }
        reg.terrains.push(TerrainDef {
            name: self.name.to_owned(),
            group: self.family.to_owned(),
            color: self.color(),
            strata,
            submerged_top: grass.then_some(dirt),
            biome_hint: None,
        });
        TerrainId((reg.terrains.len() - 1) as u16)
    }
}

fn rgb(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::stratum_block;

    #[test]
    fn names_are_unique_and_families_known() {
        for mut names in [PLANT_PRESETS.iter().map(|p| p.name).collect::<Vec<_>>(), MATERIAL_MIXES.iter().map(|m| m.name).collect()] {
            let n = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), n);
        }
        assert!(PLANT_PRESETS.iter().all(|p| PLANT_FAMILIES.contains(&p.family)));
        assert!(MATERIAL_MIXES.iter().all(|m| MIX_FAMILIES.contains(&m.family)));
        // The covers a new map has keep their names: a preset of one of
        // them is that cover (Frost and Lava, for the maps made before
        // them).
        let reg = Registry::vanilla();
        for p in PLANT_PRESETS {
            if let Some(n) = reg.cover_by_name(p.name) {
                let mut fresh = Registry::vanilla();
                fresh.covers.retain(|c| c.name != p.name);
                let added = p.install(&mut fresh);
                assert_eq!(fresh.cover(added), reg.cover(n), "{} is a cover already, and another", p.name);
            }
        }
        for m in MATERIAL_MIXES {
            assert!(reg.terrain_by_name(m.name).is_none(), "{} is a terrain already", m.name);
        }
    }

    /// A preset and a mix are added once, and found again after.
    #[test]
    fn installed_once() {
        let mut reg = Registry::vanilla();
        let (covers, terrains) = (reg.covers.len(), reg.terrains.len());
        let p = &PLANT_PRESETS[0];
        let n = p.install(&mut reg);
        assert_eq!(p.install(&mut reg), n);
        assert_eq!(reg.covers.len(), covers + 1);
        assert_eq!(reg.cover(n).unwrap().plants.len(), p.plants.len());
        let m = MATERIAL_MIXES.iter().find(|m| m.name == "Forest floor").unwrap();
        let t = m.install(&mut reg);
        assert_eq!(m.install(&mut reg), t);
        assert_eq!(reg.terrains.len(), terrains + 1);
        let def = reg.terrain(t);
        assert_eq!(def.strata.len(), 2, "grass on dirt");
        assert!(matches!(def.strata[0].mix, Mix::Patches { .. }));
        assert!(def.submerged_top.is_some());
    }

    /// Patches keep the shares of their blocks, in patches; beds repeat
    /// upwards, each as thick as its weight.
    #[test]
    fn patches_and_beds() {
        let mut reg = Registry::vanilla();
        let t = MATERIAL_MIXES.iter().find(|m| m.name == "Sand and gravel").unwrap().install(&mut reg);
        let s = reg.terrain(t).strata[0].clone();
        let sand = s.blocks[0].0;
        let (mut count, mut same) = (0, 0);
        let n = 200;
        for z in 0..n {
            for x in 0..n {
                let b = stratum_block(&s, x, 64, z, 7);
                count += (b == sand) as i32;
                same += (b == stratum_block(&s, x + 1, 64, z, 7)) as i32;
            }
        }
        let share = count as f32 / (n * n) as f32;
        assert!((share - 0.55).abs() < 0.12, "sand {share}");
        assert!(same as f32 / (n * n) as f32 > 0.75, "neighbours mostly alike: patches");

        let t = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut reg);
        let s = reg.terrain(t).strata[0].clone();
        assert_eq!(reg.terrain(t).strata[0].depth.0, u16::MAX);
        let column: Vec<_> = (0..64).map(|y| stratum_block(&s, 0, y, 0, 7)).collect();
        // Beds: long runs of one block, and every block of the mix there.
        let changes = column.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes < 64 / 2, "{changes} changes");
        for (b, _) in &s.blocks {
            assert!(column.contains(b));
        }
        // Grain stays what it was: the same blocks as before mixes existed.
        let grain = Stratum { blocks: s.blocks.clone(), depth: (3, 3), mix: Mix::Grain };
        for y in 0..16 {
            assert_eq!(stratum_block(&grain, 5, y, 9, 3), crate::compose::pick(&grain.blocks, crate::compose::hash3(5, y, 9, 3)));
        }
    }

    /// A map without mixes saves its terrains as before: no `mix` written.
    #[test]
    fn grain_is_not_written() {
        let reg = Registry::vanilla();
        assert!(!serde_json::to_string(&reg.terrains).unwrap().contains("\"mix\""));
    }
}
