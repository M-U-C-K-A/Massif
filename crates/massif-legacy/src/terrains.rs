//! How WorldPainter terrains and biomes become Massif ones.
//!
//! Terrains Massif already has are reused by name; the others are added to
//! the registry with an equivalent block recipe. Custom terrains
//! (`CUSTOM_1` … `CUSTOM_96`) are rebuilt from the world's own mixed
//! materials once the whole file has been read.

use crate::tables::{BIOMES, TERRAINS};
use massif_core::noise::Noise;
use massif_core::registry::{BiomeDef, BlockId, Registry, Stratum, TerrainDef, TerrainId};
use std::collections::BTreeMap;

/// Marks a column of WorldPainter's "Beaches" (or "Bare Beaches") until its
/// own material is picked (see [`Beaches`]).
pub const BEACH_MARK: u16 = u16::MAX;

/// WorldPainter's noise scales (`Constants.SMALL_BLOBS`, `HUGE_BLOBS`).
const SMALL_BLOBS: f64 = 16.411;
const HUGE_BLOBS: f64 = 131.101;

/// WorldPainter's "Beaches": grass with patches of sand, gravel and clay
/// (big blobs of sand and gravel, small ones of clay), as its
/// `Terrain.BEACHES` draws them; sand all over would be a desert.
pub struct Beaches {
    sand: Noise,
    clay: Noise,
    /// Levels of the noise passed 4 % (clay), 40 % (sand) and 20 %
    /// (gravel) of the time, as WorldPainter's `getLevelForPromillage`.
    clay_level: f64,
    sand_level: f64,
    gravel_level: f64,
    /// Grass, sand, gravel and clay.
    ids: [u16; 4],
}

impl Beaches {
    pub fn new(reg: &Registry) -> Self {
        let id = |name: &str| reg.terrain_by_name(name).map_or(0, |t| t.0);
        let sand = Noise::new(26_796_036);
        let clay = Noise::new(161_603_308);
        let mut samples: Vec<f64> = (0..20_000)
            .map(|i| {
                let h = massif_core::compose::hash2(i, 0x5eed, 0xbeac);
                sand.get((h & 0xffff) as f64 * 0.731 + 0.5, (h >> 16) as f64 * 0.619 + 0.5)
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        let level = |share: f64| samples[((1.0 - share) * (samples.len() - 1) as f64) as usize];
        Self {
            sand,
            clay,
            clay_level: level(0.04),
            sand_level: level(0.40) * 1.5,
            gravel_level: level(0.20) * 1.5,
            ids: [id("Grass"), id("Sand"), id("Gravel"), id("Clay")],
        }
    }

    /// The terrain of the column at `(x, z)`, its ground at `height`.
    pub fn at(&self, x: i32, z: i32, height: f32) -> u16 {
        let (x, z) = (x as f64, z as f64);
        // WorldPainter's noise has a third dimension, the height: its
        // patches shift as the ground rises.
        let lift = height as f64 / SMALL_BLOBS;
        let (lx, lz) = (lift * 0.71, lift * 0.53);
        let [grass, sand, gravel, clay] = self.ids;
        if self.clay.get(x / SMALL_BLOBS + lx, z / SMALL_BLOBS + lz) >= self.clay_level {
            return clay;
        }
        let n = self.sand.get(x / HUGE_BLOBS + lx, z / HUGE_BLOBS + lz)
            + self.sand.get(x / SMALL_BLOBS + lx + 57.3, z / SMALL_BLOBS + lz + 91.7) / 2.0;
        if n >= self.sand_level {
            sand
        } else if -n >= self.gravel_level {
            gravel
        } else {
            grass
        }
    }
}

const DEEP: u16 = u16::MAX;

/// A block recipe: weighted blocks and a depth range.
type Layer = (Vec<(&'static str, u16)>, u16, u16);

enum Target {
    /// A terrain Massif already has, by name.
    Existing(&'static str),
    /// A terrain to add.
    New {
        name: &'static str,
        group: &'static str,
        color: [u8; 3],
        strata: Vec<Layer>,
    },
}

fn stained(name: &'static str, block: &'static str, color: [u8; 3]) -> Target {
    Target::New {
        name,
        group: "Sand & Desert",
        color,
        strata: vec![(vec![(block, 1)], 8, 14)],
    }
}

fn builtin(wp: &str) -> Target {
    use Target::*;
    match wp {
        "GRASS" | "BARE_GRASS" => Existing("Grass"),
        "DIRT" => Existing("Dirt"),
        "SAND" | "DESERT" => Existing("Sand"),
        "SANDSTONE" => Existing("Sandstone"),
        "STONE" | "RESOURCES" => Existing("Stone"),
        "ROCK" => Existing("Rocky"),
        "DEEP_SNOW" => Existing("Snow"),
        "GRAVEL" => Existing("Gravel"),
        "CLAY" => Existing("Clay"),
        "COBBLESTONE" => Existing("Cobblestone"),
        "NETHERRACK" | "NETHERLIKE" => Existing("Netherrack"),
        "SOUL_SAND" => Existing("Soul Sand"),
        "OBSIDIAN" => Existing("Obsidian"),
        "BEDROCK" => Existing("Bedrock"),
        "MYCELIUM" => Existing("Mycelium"),
        "END_STONE" => Existing("End Stone"),
        "PERMADIRT" => Existing("Coarse Dirt"),
        "PODZOL" => Existing("Podzol"),
        "RED_SAND" | "RED_DESERT" => Existing("Red Sand"),
        "HARDENED_CLAY" => Existing("Terracotta"),
        "GRANITE" => Existing("Granite"),
        "DIORITE" => Existing("Diorite"),
        "ANDESITE" => Existing("Andesite"),
        "DEEPSLATE" => Existing("Deepslate"),
        "TUFF" => Existing("Tuff"),
        "BASALT" => Existing("Basalt"),
        "BLACKSTONE" => Existing("Blackstone"),
        "CALCITE" => Existing("Calcite"),
        "MUD" => Existing("Mud"),
        "MOSS" => Existing("Moss"),
        "WATER" => New {
            name: "Water",
            group: "Other",
            color: [63, 118, 228],
            strata: vec![(vec![("minecraft:water[level=0]", 1)], 1, 1), (vec![("minecraft:gravel", 1)], 2, 3)],
        },
        "LAVA" => New {
            name: "Lava",
            group: "Other",
            color: [207, 92, 15],
            strata: vec![(vec![("minecraft:lava[level=0]", 1)], 1, 1)],
        },
        "SNOW" => New {
            name: "Snow on Rock",
            group: "Snow & Ice",
            color: [236, 244, 246],
            strata: vec![(vec![("minecraft:snow_block", 1)], 1, 1)],
        },
        "MOSSY_COBBLESTONE" => New {
            name: "Mossy Cobblestone",
            group: "Stone",
            color: [110, 118, 94],
            strata: vec![(vec![("minecraft:mossy_cobblestone", 1)], 3, 4)],
        },
        "MESA" => New {
            name: "Mesa",
            group: "Sand & Desert",
            color: [176, 92, 52],
            strata: vec![(
                vec![
                    ("minecraft:terracotta", 4),
                    ("minecraft:orange_terracotta", 3),
                    ("minecraft:yellow_terracotta", 2),
                    ("minecraft:red_terracotta", 2),
                    ("minecraft:white_terracotta", 1),
                    ("minecraft:light_gray_terracotta", 1),
                ],
                10,
                16,
            )],
        },
        "RED_SANDSTONE" => New {
            name: "Red Sandstone",
            group: "Sand & Desert",
            color: [186, 99, 29],
            strata: vec![(vec![("minecraft:red_sandstone", 1)], 6, 10)],
        },
        "STONE_MIX" => New {
            name: "Stone Mix",
            group: "Stone",
            color: [128, 124, 120],
            strata: vec![(
                vec![
                    ("minecraft:stone", 8),
                    ("minecraft:granite", 1),
                    ("minecraft:diorite", 1),
                    ("minecraft:andesite", 1),
                ],
                DEEP,
                DEEP,
            )],
        },
        "GRASS_PATH" => New {
            name: "Dirt Path",
            group: "Grass & Soil",
            color: [148, 122, 65],
            strata: vec![(vec![("minecraft:dirt_path", 1)], 1, 1), (vec![("minecraft:dirt", 1)], 2, 3)],
        },
        "MAGMA" => New {
            name: "Magma",
            group: "Other Dimensions",
            color: [142, 63, 31],
            strata: vec![(vec![("minecraft:magma_block", 1)], 3, 5)],
        },
        "SOUL_SOIL" => New {
            name: "Soul Soil",
            group: "Other Dimensions",
            color: [75, 57, 46],
            strata: vec![(vec![("minecraft:soul_soil", 1)], 3, 5)],
        },
        "WARPED_NYLIUM" => New {
            name: "Warped Nylium",
            group: "Other Dimensions",
            color: [43, 114, 101],
            strata: vec![(vec![("minecraft:warped_nylium", 1)], 1, 1), (vec![("minecraft:netherrack", 1)], DEEP, DEEP)],
        },
        "CRIMSON_NYLIUM" => New {
            name: "Crimson Nylium",
            group: "Other Dimensions",
            color: [130, 31, 31],
            strata: vec![(vec![("minecraft:crimson_nylium", 1)], 1, 1), (vec![("minecraft:netherrack", 1)], DEEP, DEEP)],
        },
        "PALE_MOSS" => New {
            name: "Pale Moss",
            group: "Grass & Soil",
            color: [106, 112, 106],
            strata: vec![(vec![("minecraft:pale_moss_block", 1)], 1, 1), (vec![("minecraft:dirt", 1)], 2, 3)],
        },
        "WHITE_STAINED_CLAY" => stained("White Terracotta", "minecraft:white_terracotta", [210, 178, 161]),
        "ORANGE_STAINED_CLAY" => stained("Orange Terracotta", "minecraft:orange_terracotta", [162, 84, 38]),
        "MAGENTA_STAINED_CLAY" => stained("Magenta Terracotta", "minecraft:magenta_terracotta", [150, 88, 109]),
        "LIGHT_BLUE_STAINED_CLAY" => stained("Light Blue Terracotta", "minecraft:light_blue_terracotta", [113, 109, 138]),
        "YELLOW_STAINED_CLAY" => stained("Yellow Terracotta", "minecraft:yellow_terracotta", [186, 133, 35]),
        "LIME_STAINED_CLAY" => stained("Lime Terracotta", "minecraft:lime_terracotta", [104, 118, 53]),
        "PINK_STAINED_CLAY" => stained("Pink Terracotta", "minecraft:pink_terracotta", [162, 78, 79]),
        "GREY_STAINED_CLAY" => stained("Gray Terracotta", "minecraft:gray_terracotta", [58, 42, 36]),
        "LIGHT_GREY_STAINED_CLAY" => stained("Light Gray Terracotta", "minecraft:light_gray_terracotta", [135, 107, 98]),
        "CYAN_STAINED_CLAY" => stained("Cyan Terracotta", "minecraft:cyan_terracotta", [87, 91, 91]),
        "PURPLE_STAINED_CLAY" => stained("Purple Terracotta", "minecraft:purple_terracotta", [118, 70, 86]),
        "BLUE_STAINED_CLAY" => stained("Blue Terracotta", "minecraft:blue_terracotta", [74, 60, 91]),
        "BROWN_STAINED_CLAY" => stained("Brown Terracotta", "minecraft:brown_terracotta", [77, 51, 36]),
        "GREEN_STAINED_CLAY" => stained("Green Terracotta", "minecraft:green_terracotta", [76, 83, 42]),
        "RED_STAINED_CLAY" => stained("Red Terracotta", "minecraft:red_terracotta", [143, 61, 47]),
        "BLACK_STAINED_CLAY" => stained("Black Terracotta", "minecraft:black_terracotta", [37, 23, 16]),
        _ => Existing("Stone"),
    }
}

/// A colour for the 3D preview of a block we know nothing else about.
pub fn block_color(state: &str) -> [u8; 3] {
    let name = state.split('[').next().unwrap_or(state);
    let name = name.trim_start_matches("minecraft:");
    const KNOWN: &[(&str, [u8; 3])] = &[
        ("stone", [125, 125, 125]),
        ("grass_block", [96, 148, 62]),
        ("dirt", [134, 96, 67]),
        ("coarse_dirt", [119, 85, 59]),
        ("rooted_dirt", [144, 103, 76]),
        ("sand", [219, 207, 163]),
        ("red_sand", [190, 102, 33]),
        ("gravel", [136, 126, 126]),
        ("clay", [160, 166, 179]),
        ("snow_block", [249, 254, 254]),
        ("snow", [249, 254, 254]),
        ("ice", [145, 183, 253]),
        ("packed_ice", [141, 180, 250]),
        ("water", [63, 118, 228]),
        ("lava", [207, 92, 15]),
        ("cobblestone", [127, 127, 127]),
        ("mossy_cobblestone", [110, 118, 94]),
        ("andesite", [136, 136, 136]),
        ("diorite", [188, 188, 188]),
        ("granite", [149, 103, 85]),
        ("deepslate", [80, 80, 82]),
        ("tuff", [108, 109, 102]),
        ("calcite", [223, 224, 220]),
        ("moss_block", [89, 109, 45]),
        ("mud", [60, 57, 61]),
        ("podzol", [91, 63, 24]),
        ("mycelium", [111, 99, 105]),
        ("terracotta", [152, 94, 67]),
        ("sandstone", [216, 203, 155]),
        ("red_sandstone", [186, 99, 29]),
        ("netherrack", [97, 38, 38]),
        ("basalt", [80, 81, 86]),
        ("blackstone", [42, 36, 41]),
        ("obsidian", [15, 10, 24]),
        ("bedrock", [85, 85, 85]),
        ("oak_leaves", [60, 120, 40]),
        ("spruce_leaves", [52, 86, 52]),
        ("birch_leaves", [96, 140, 60]),
        ("oak_log", [109, 85, 50]),
        ("spruce_log", [58, 37, 16]),
    ];
    if let Some((_, c)) = KNOWN.iter().find(|(n, _)| *n == name) {
        return *c;
    }
    // Then by what the name says.
    const HINTS: &[(&str, [u8; 3])] = &[
        ("snow", [240, 246, 248]),
        ("powder", [236, 244, 248]),
        ("blue_ice", [116, 167, 253]),
        ("ice", [145, 183, 253]),
        ("water", [63, 118, 228]),
        ("magma", [142, 63, 31]),
        ("lava", [207, 92, 15]),
        ("grass", [96, 148, 62]),
        ("moss", [89, 109, 45]),
        ("leaves", [60, 120, 40]),
        ("coral", [190, 90, 130]),
        ("red_terracotta", [143, 61, 47]),
        ("orange_terracotta", [162, 84, 38]),
        ("yellow_terracotta", [186, 133, 35]),
        ("white_terracotta", [210, 178, 161]),
        ("brown_terracotta", [77, 51, 36]),
        ("light_gray_terracotta", [135, 107, 98]),
        ("terracotta", [152, 94, 67]),
        ("red_sand", [190, 102, 33]),
        ("sandstone", [216, 203, 155]),
        ("sand", [219, 207, 163]),
        ("gravel", [136, 126, 126]),
        ("clay", [160, 166, 179]),
        ("mud", [60, 57, 61]),
        ("podzol", [91, 63, 24]),
        ("dirt", [134, 96, 67]),
        ("granite", [149, 103, 85]),
        ("diorite", [188, 188, 188]),
        ("calcite", [223, 224, 220]),
        ("quartz", [235, 229, 222]),
        ("andesite", [136, 136, 136]),
        ("tuff", [108, 109, 102]),
        ("deepslate", [80, 80, 82]),
        ("basalt", [80, 81, 86]),
        ("blackstone", [42, 36, 41]),
        ("obsidian", [15, 10, 24]),
        ("netherrack", [97, 38, 38]),
        ("nylium", [90, 60, 60]),
        ("end_stone", [219, 222, 158]),
        ("prismarine", [99, 156, 151]),
        ("log", [109, 85, 50]),
        ("wood", [109, 85, 50]),
        ("planks", [162, 130, 78]),
        ("cobble", [127, 127, 127]),
        ("brick", [150, 97, 83]),
        ("stone", [125, 125, 125]),
        ("ore", [125, 125, 125]),
    ];
    if let Some((_, c)) = HINTS.iter().find(|(k, _)| name.contains(k)) {
        return *c;
    }
    // Anything else: a stable, muted colour from the name.
    let h = name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let c = |shift: u32| 70 + ((h >> shift) & 0x7f) as u8;
    [c(0), c(8), c(16)]
}

/// A custom terrain of the world being imported.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomMaterial {
    pub name: String,
    /// Block states and their relative counts.
    pub rows: Vec<(String, u32)>,
    pub colour: Option<[u8; 3]>,
}

/// Maps WorldPainter terrain ordinals and biome numbers to Massif ids,
/// extending the registry as needed.
pub struct Mapper {
    pub registry: Registry,
    terrains: [Option<u16>; 256],
    biomes: [Option<u16>; 256],
    /// Custom terrain index → Massif terrain, to be defined once the world's
    /// mixed materials are known.
    pending_custom: BTreeMap<usize, u16>,
    /// Columns per WorldPainter terrain, for the report.
    pub terrain_use: BTreeMap<String, u64>,
    /// WorldPainter biome numbers that are none of the game's (a custom
    /// biome's), and the biome each became: named once the dimension's
    /// custom biomes are read (see [`Mapper::name_custom_biomes`]).
    pub custom_biomes: Vec<(u8, u16)>,
}

impl Mapper {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            terrains: [None; 256],
            biomes: [None; 256],
            pending_custom: BTreeMap::new(),
            terrain_use: BTreeMap::new(),
            custom_biomes: Vec::new(),
        }
    }

    fn intern(&mut self, state: &str) -> BlockId {
        let color = block_color(state);
        self.registry.intern_block(state, color)
    }

    fn add_terrain(&mut self, def: TerrainDef) -> u16 {
        if let Some(id) = self.registry.terrain_by_name(&def.name) {
            return id.0;
        }
        self.registry.terrains.push(def);
        (self.registry.terrains.len() - 1) as u16
    }

    /// The terrain of every ordinal in `raw` (whose values `counts` counts),
    /// resolved as [`Mapper::terrain`] would one column after the other.
    pub fn terrain_table(&mut self, raw: &[u8], counts: &[u64; 256]) -> [u16; 256] {
        self.table(raw, counts, |m, o| m.terrains[o as usize], Self::terrain)
    }

    /// The biome of every WorldPainter biome in `raw`, as [`Mapper::biome`].
    pub fn biome_table(&mut self, raw: &[u8], counts: &[u64; 256]) -> [u16; 256] {
        self.table(raw, counts, |m, b| m.biomes[b as usize], Self::biome)
    }

    /// New values join the registry in the order they first appear, so a
    /// tile with values not met before is walked in order; any other is
    /// looked up at once.
    fn table(&mut self, raw: &[u8], counts: &[u64; 256], known: fn(&Self, u8) -> Option<u16>, resolve: fn(&mut Self, u8) -> u16) -> [u16; 256] {
        let mut table = [0u16; 256];
        let mut all_known = true;
        for (v, &c) in counts.iter().enumerate() {
            if c > 0 {
                match known(self, v as u8) {
                    Some(id) => table[v] = id,
                    None => all_known = false,
                }
            }
        }
        if !all_known {
            let mut seen = [false; 256];
            for &v in raw {
                if !std::mem::replace(&mut seen[v as usize], true) {
                    table[v as usize] = resolve(self, v);
                }
            }
        }
        table
    }

    /// The Massif terrain for a WorldPainter terrain ordinal.
    pub fn terrain(&mut self, ordinal: u8) -> u16 {
        if let Some(id) = self.terrains[ordinal as usize] {
            return id;
        }
        let id = self.resolve_terrain(ordinal);
        self.terrains[ordinal as usize] = Some(id);
        id
    }

    pub fn count_terrain(&mut self, ordinal: u8, columns: u64) {
        let name = TERRAINS.get(ordinal as usize).copied().unwrap_or("UNKNOWN");
        *self.terrain_use.entry(name.to_owned()).or_default() += columns;
    }

    fn resolve_terrain(&mut self, ordinal: u8) -> u16 {
        let Some(&wp) = TERRAINS.get(ordinal as usize) else {
            return self.registry.terrain_by_name("Stone").map_or(0, |t| t.0);
        };
        if matches!(wp, "BEACHES" | "BARE_BEACHES") {
            return BEACH_MARK;
        }
        if let Some(n) = wp.strip_prefix("CUSTOM_").and_then(|n| n.parse::<usize>().ok()) {
            // A placeholder until the mixed materials are read.
            let id = self.add_terrain(TerrainDef {
                name: format!("Custom {n}"),
                group: "Imported".into(),
                color: [128, 128, 128],
                strata: Vec::new(),
                submerged_top: None,
                biome_hint: None,
            });
            self.pending_custom.insert(n - 1, id);
            return id;
        }
        match builtin(wp) {
            Target::Existing(name) => self.registry.terrain_by_name(name).map_or(0, |t| t.0),
            Target::New {
                name,
                group,
                color,
                strata,
            } => {
                if let Some(id) = self.registry.terrain_by_name(name) {
                    return id.0;
                }
                let strata = strata
                    .into_iter()
                    .map(|(blocks, lo, hi)| Stratum {
                        blocks: blocks.into_iter().map(|(b, w)| (self.intern(b), w)).collect(),
                        depth: (lo, hi),
                        mix: Default::default(),
                    })
                    .collect();
                self.add_terrain(TerrainDef {
                    name: name.into(),
                    group: group.into(),
                    color,
                    strata,
                    submerged_top: None,
                    biome_hint: None,
                })
            }
        }
    }

    /// Define the custom terrains used by the world from its mixed materials.
    /// Returns how many were defined.
    pub fn finish_custom(&mut self, materials: &[Option<CustomMaterial>]) -> usize {
        let pending = std::mem::take(&mut self.pending_custom);
        let mut defined = 0;
        for (index, id) in pending {
            let Some(Some(m)) = materials.get(index) else { continue };
            if m.rows.is_empty() {
                continue;
            }
            let total: u64 = m.rows.iter().map(|r| r.1 as u64).sum::<u64>().max(1);
            let blocks: Vec<(BlockId, u16)> = m
                .rows
                .iter()
                .map(|(state, count)| {
                    // Keep relative weights within u16.
                    let w = ((*count as u64 * 1000) / total).clamp(1, 1000) as u16;
                    (self.intern(state), w)
                })
                .collect();
            let color = m.colour.unwrap_or_else(|| block_color(&m.rows[0].0));
            let name = if m.name.trim().is_empty() {
                format!("Custom {}", index + 1)
            } else {
                m.name.trim().to_owned()
            };
            // Keep names unique: two custom terrains may share a name.
            let name = if self.registry.terrain_by_name(&name).is_some_and(|t| t.0 != id) {
                format!("{name} ({})", index + 1)
            } else {
                name
            };
            let def = &mut self.registry.terrains[id as usize];
            def.name = name;
            def.color = color;
            def.strata = vec![Stratum { blocks, depth: (3, 7), mix: Default::default() }];
            defined += 1;
        }
        defined
    }

    /// The Massif biome for a WorldPainter biome number (255 = automatic).
    pub fn biome(&mut self, wp: u8) -> u16 {
        if let Some(id) = self.biomes[wp as usize] {
            return id;
        }
        let id = match BIOMES[wp as usize] {
            None if wp == 255 => 0,
            // One of the world's custom biomes: a biome of its own, named
            // when the world's list of them is read.
            None => {
                let def = BiomeDef::of_id(&format!("worldpainter:custom_biome_{wp}"));
                self.registry.biomes.push(def);
                let id = (self.registry.biomes.len() - 1) as u16;
                self.custom_biomes.push((wp, id));
                id
            }
            Some(name) => match self.registry.biome_by_id(name) {
                Some(id) => id.0,
                None => {
                    let pretty = name
                        .trim_start_matches("minecraft:")
                        .split('_')
                        .map(|w| {
                            let mut c = w.chars();
                            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.registry.biomes.push(BiomeDef {
                        id: name.into(),
                        name: pretty,
                        color: block_color(name),
                        ..Default::default()
                    });
                    (self.registry.biomes.len() - 1) as u16
                }
            },
        };
        self.biomes[wp as usize] = Some(id);
        id
    }
}

impl Mapper {
    /// Name the custom biomes met (and add those never painted): from
    /// WorldPainter's list of them, its number, name and colour each. The
    /// name is the id WorldPainter exports (a data pack's or mod's,
    /// `terralith:alpine_highlands`); one that is no id is the biome's name,
    /// its id made of it. Returns their names.
    pub fn name_custom_biomes(&mut self, list: &[(u8, String, Option<[u8; 3]>)]) -> Vec<String> {
        let mut names = Vec::new();
        for (wp, name, color) in list {
            let id = match massif_core::registry::check_biome_id(name) {
                Ok(()) => name.clone(),
                Err(_) => {
                    let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
                    format!("worldpainter:{}", slug.trim_matches('_'))
                }
            };
            let display = if id == *name { massif_core::registry::biome_display_name(&id) } else { name.clone() };
            let n = match self.custom_biomes.iter().find(|c| c.0 == *wp) {
                Some(&(_, n)) => n,
                None => match self.registry.biome_by_id(&id) {
                    Some(b) => b.0,
                    None => {
                        self.registry.biomes.push(BiomeDef::of_id(&id));
                        (self.registry.biomes.len() - 1) as u16
                    }
                },
            };
            let def = &mut self.registry.biomes[n as usize];
            def.id = id;
            def.name = display.clone();
            if let Some(c) = color {
                def.color = *c;
            }
            def.group = "WorldPainter".into();
            names.push(display);
        }
        names
    }
}

/// The WorldPainter terrain (its ordinal in `Terrain.VALUES`) that
/// terrain `id` of `reg` stands for, as [`Mapper::terrain`] maps them the
/// other way: the first of WorldPainter's terrains made that terrain, by
/// name. `None` for a terrain WorldPainter does not have (custom terrains
/// included, which only their map knows).
pub fn ordinal_of(reg: &Registry, id: u16) -> Option<u8> {
    let name = &reg.terrains.get(id as usize)?.name;
    TERRAINS.iter().position(|wp| {
        !wp.starts_with("CUSTOM_")
            && match builtin(wp) {
                Target::Existing(n) => n.eq_ignore_ascii_case(name),
                Target::New { name: n, .. } => n.eq_ignore_ascii_case(name),
            }
    }).map(|k| k as u8)
}

/// The terrain id a Massif terrain name resolves to, for tests.
pub fn massif_terrain(reg: &Registry, name: &str) -> Option<TerrainId> {
    reg.terrain_by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WorldPainter's beaches are mostly grass, with big patches of sand,
    /// some gravel, a little clay.
    #[test]
    fn beaches_are_grass_with_patches() {
        let reg = Registry::vanilla();
        let beaches = Beaches::new(&reg);
        let name = |id: u16| reg.terrain(TerrainId(id)).name.clone();
        let mut counts = std::collections::BTreeMap::new();
        for z in (0..2048).step_by(3) {
            for x in (0..2048).step_by(3) {
                *counts.entry(name(beaches.at(x, z, 64.0))).or_insert(0usize) += 1;
            }
        }
        let total: usize = counts.values().sum();
        let share = |n: &str| counts.get(n).copied().unwrap_or(0) as f32 / total as f32;
        assert!(share("Grass") > 0.4, "{counts:?}");
        assert!((0.15..0.45).contains(&share("Sand")), "{counts:?}");
        assert!((0.03..0.25).contains(&share("Gravel")), "{counts:?}");
        assert!((0.01..0.08).contains(&share("Clay")), "{counts:?}");
    }

    fn ordinal(name: &str) -> u8 {
        TERRAINS.iter().position(|t| *t == name).unwrap() as u8
    }

    #[test]
    fn every_builtin_terrain_maps_to_a_usable_terrain() {
        let mut m = Mapper::new(Registry::vanilla());
        for (i, name) in TERRAINS.iter().enumerate() {
            let id = m.terrain(i as u8);
            let def = m.registry.terrain(TerrainId(id));
            if name.starts_with("CUSTOM_") {
                assert!(def.name.starts_with("Custom "), "{name}");
            } else {
                for s in &def.strata {
                    for (b, _) in &s.blocks {
                        assert!(b.index() < m.registry.blocks.len());
                    }
                }
            }
        }
        // Reuse rather than duplicate.
        let grass = m.terrain(ordinal("GRASS"));
        assert_eq!(m.registry.terrain(TerrainId(grass)).name, "Grass");
        assert_eq!(m.terrain(ordinal("BARE_GRASS")), grass);
        let mesa = m.terrain(ordinal("MESA"));
        assert_eq!(m.registry.terrain(TerrainId(mesa)).strata[0].blocks.len(), 6);
        // Names stay unique.
        let mut names: Vec<_> = m.registry.terrains.iter().map(|t| t.name.clone()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n);
    }

    #[test]
    fn custom_terrains_come_from_mixed_materials() {
        let mut m = Mapper::new(Registry::vanilla());
        let id = m.terrain(ordinal("CUSTOM_3"));
        let unused = m.terrain(ordinal("CUSTOM_7"));
        let materials = vec![
            None,
            None,
            Some(CustomMaterial {
                name: "Volcanic ash".into(),
                rows: vec![("minecraft:gray_concrete_powder".into(), 750), ("minecraft:basalt[axis=y]".into(), 250)],
                colour: Some([70, 70, 74]),
            }),
        ];
        assert_eq!(m.finish_custom(&materials), 1);
        let def = m.registry.terrain(TerrainId(id));
        assert_eq!(def.name, "Volcanic ash");
        assert_eq!(def.color, [70, 70, 74]);
        let weights: Vec<u16> = def.strata[0].blocks.iter().map(|b| b.1).collect();
        assert_eq!(weights, vec![750, 250]);
        assert_eq!(m.registry.block(def.strata[0].blocks[1].0).state, "minecraft:basalt[axis=y]");
        // A custom terrain without a definition keeps its placeholder.
        assert_eq!(m.registry.terrain(TerrainId(unused)).name, "Custom 7");
    }

    #[test]
    fn biomes_map_to_modern_ids() {
        let mut m = Mapper::new(Registry::vanilla());
        assert_eq!(m.biome(255), 0);
        let plains = m.biome(1);
        assert_eq!(m.registry.biome(massif_core::BiomeId(plains)).id, "minecraft:plains");
        let before = m.registry.biomes.len();
        let grove = m.biome(250);
        assert_eq!(m.registry.biome(massif_core::BiomeId(grove)).id, "minecraft:grove");
        let sparse = m.biome(23);
        assert_eq!(m.registry.biome(massif_core::BiomeId(sparse)).id, "minecraft:sparse_jungle");
        assert_eq!(m.registry.biome(massif_core::BiomeId(sparse)).name, "Sparse Jungle");
        assert_eq!(m.registry.biomes.len(), before + 1);
        assert_eq!(m.biome(23), sparse);
    }
}
