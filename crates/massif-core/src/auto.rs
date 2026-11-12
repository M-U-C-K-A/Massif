//! Automatic biomes and automatic painting.
//!
//! *Automatic biomes* read the land: every column is deep water, a river,
//! a beach, a plain, a snowy slope… from its water, its height over the
//! sea, its slope and what its ground is made of, and each kind of land has
//! a biome. *Automatic painting* goes the other way: every biome has a
//! ground, another for its cliffs and one for what lies under its water.
//! Both tables are settings, to be changed.

use crate::compose::{Span, TileStack};
use crate::coords::{BlockRect, TilePos, local_index};
use crate::filter::{SLOPE_REACH, trend_slope};
use crate::history::Edit;
use crate::registry::{BiomeId, Registry, TerrainId};
use crate::tile::NO_FLUID;
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A kind of land the automatic biomes tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Land {
    DeepWater,
    Water,
    River,
    Beach,
    StonyShore,
    SnowyShore,
    Plains,
    Hills,
    Highlands,
    Woodland,
    Jungle,
    Swamp,
    Mushroom,
    Desert,
    Badlands,
    SnowyPlains,
    SnowySlopes,
    FrozenPeaks,
    RockyHills,
    StonyPeaks,
}

impl Land {
    pub const ALL: [Land; 20] = [
        Land::DeepWater,
        Land::Water,
        Land::River,
        Land::Beach,
        Land::StonyShore,
        Land::SnowyShore,
        Land::Plains,
        Land::Hills,
        Land::Highlands,
        Land::Woodland,
        Land::Jungle,
        Land::Swamp,
        Land::Mushroom,
        Land::Desert,
        Land::Badlands,
        Land::SnowyPlains,
        Land::SnowySlopes,
        Land::FrozenPeaks,
        Land::RockyHills,
        Land::StonyPeaks,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Land::DeepWater => "Deep water",
            Land::Water => "Water",
            Land::River => "River",
            Land::Beach => "Sandy shore",
            Land::StonyShore => "Stony shore",
            Land::SnowyShore => "Snowy shore",
            Land::Plains => "Lowland",
            Land::Hills => "Hills",
            Land::Highlands => "Highlands",
            Land::Woodland => "Forest floor",
            Land::Jungle => "Moss",
            Land::Swamp => "Mud",
            Land::Mushroom => "Mycelium",
            Land::Desert => "Sand",
            Land::Badlands => "Red sand and terracotta",
            Land::SnowyPlains => "Snow, low",
            Land::SnowySlopes => "Snow, on the mountains",
            Land::FrozenPeaks => "Snow, on the peaks",
            Land::RockyHills => "Rock and cliffs, low",
            Land::StonyPeaks => "Rock and cliffs, high",
        }
    }

    /// How the land is told, in a few words.
    pub const fn hint(self) -> &'static str {
        match self {
            Land::DeepWater => "Water deeper than the deep-water depth",
            Land::Water => "Shallower water, open on at least one side",
            Land::River => "Water with banks close on both sides",
            Land::Beach => "Sand at the water's edge",
            Land::StonyShore => "Rock, gravel or a cliff at the water's edge",
            Land::SnowyShore => "Snow or ice at the water's edge",
            Land::Plains => "Grass and soil below the hills",
            Land::Hills => "Grass and soil from the hills up to the mountains",
            Land::Highlands => "Grass and soil on the mountains",
            Land::Woodland => "Podzol, coarse and rooted dirt",
            Land::Jungle => "Moss",
            Land::Swamp => "Mud",
            Land::Mushroom => "Mycelium",
            Land::Desert => "Sand and sandstone away from the water",
            Land::Badlands => "Red sand, red sandstone, terracotta",
            Land::SnowyPlains => "Snow and ice below the mountains, on gentle ground",
            Land::SnowySlopes => "Snow and ice on the mountains or on steep ground",
            Land::FrozenPeaks => "Snow and ice above the peaks' height",
            Land::RockyHills => "Stone, gravel and steep slopes below the mountains",
            Land::StonyPeaks => "Stone, gravel and steep slopes on the mountains",
        }
    }

    /// The biome it gets unless the settings say otherwise.
    pub const fn default_biome(self) -> &'static str {
        match self {
            Land::DeepWater => "minecraft:deep_ocean",
            Land::Water => "minecraft:ocean",
            Land::River => "minecraft:river",
            Land::Beach => "minecraft:beach",
            Land::StonyShore => "minecraft:stony_shore",
            Land::SnowyShore => "minecraft:snowy_beach",
            Land::Plains => "minecraft:plains",
            Land::Hills => "minecraft:forest",
            Land::Highlands => "minecraft:meadow",
            Land::Woodland => "minecraft:old_growth_pine_taiga",
            Land::Jungle => "minecraft:jungle",
            Land::Swamp => "minecraft:mangrove_swamp",
            Land::Mushroom => "minecraft:mushroom_fields",
            Land::Desert => "minecraft:desert",
            Land::Badlands => "minecraft:badlands",
            Land::SnowyPlains => "minecraft:snowy_plains",
            Land::SnowySlopes => "minecraft:snowy_slopes",
            Land::FrozenPeaks => "minecraft:frozen_peaks",
            Land::RockyHills => "minecraft:windswept_hills",
            Land::StonyPeaks => "minecraft:stony_peaks",
        }
    }
}

/// What the ground of a column is made of, as far as biomes care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    /// Grass, dirt and anything else that grows.
    Grass,
    ForestFloor,
    Moss,
    Mud,
    Mycelium,
    Sand,
    RedSand,
    Snow,
    Rock,
}

impl Ground {
    /// From the name of a block (`minecraft:grass_block[snowy=false]`).
    pub fn of_block(state: &str) -> Ground {
        let name = state.split('[').next().unwrap_or(state);
        let name = name.rsplit(':').next().unwrap_or(name);
        let has = |part: &str| name.contains(part);
        if has("red_sand") || has("terracotta") {
            Ground::RedSand
        } else if has("sand") && !has("soul") {
            Ground::Sand
        } else if has("snow") || has("ice") {
            Ground::Snow
        } else if has("podzol") || has("coarse_dirt") || has("rooted_dirt") {
            Ground::ForestFloor
        } else if has("moss") {
            Ground::Moss
        } else if has("mud") {
            Ground::Mud
        } else if has("mycelium") {
            Ground::Mycelium
        } else if has("grass") || has("dirt") || has("clay") || has("farmland") {
            Ground::Grass
        } else if [
            "stone", "gravel", "andesite", "diorite", "granite", "tuff", "calcite", "basalt", "deepslate", "obsidian", "bedrock", "dripstone", "rack",
            "ore", "concrete", "brick",
        ]
        .iter()
        .any(|p| has(p))
        {
            Ground::Rock
        } else {
            Ground::Grass
        }
    }

    /// Of every terrain of a registry, from the block on top of it.
    pub fn of_terrains(reg: &Registry) -> Vec<Ground> {
        reg.terrains
            .iter()
            .map(|t| {
                let top = t.strata.first().and_then(|s| s.blocks.iter().max_by_key(|b| b.1));
                match top {
                    Some((block, _)) => Ground::of_block(&reg.blocks[block.0 as usize].state),
                    None => Ground::Rock,
                }
            })
            .collect()
    }
}

/// How the land is read, and the biome of each kind of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BiomeRules {
    /// Water at least this deep (blocks) is deep water.
    pub deep_water: f32,
    /// Water with a bank on both sides within this many blocks is a river.
    pub river_width: i32,
    /// Ground at most this high over the water next to it is its shore.
    pub shore_height: f32,
    /// Heights over the sea where the hills, the mountains and the peaks
    /// begin.
    pub hills: f32,
    pub mountains: f32,
    pub peaks: f32,
    /// Slopes at least this steep (degrees) are cliffs: rock, whatever
    /// grows on them.
    pub steep: f32,
    /// The biome of each kind of land, by its id (`minecraft:plains`).
    pub biomes: Vec<(Land, String)>,
}

impl Default for BiomeRules {
    fn default() -> Self {
        Self {
            deep_water: 24.0,
            river_width: 28,
            shore_height: 3.0,
            hills: 30.0,
            mountains: 80.0,
            peaks: 140.0,
            steep: 42.0,
            biomes: Land::ALL.iter().map(|l| (*l, l.default_biome().to_owned())).collect(),
        }
    }
}

impl BiomeRules {
    /// The id of the biome of a kind of land.
    pub fn biome_of(&self, land: Land) -> &str {
        self.biomes.iter().find(|(l, _)| *l == land).map_or(land.default_biome(), |(_, b)| b.as_str())
    }

    pub fn set_biome(&mut self, land: Land, id: &str) {
        match self.biomes.iter_mut().find(|(l, _)| *l == land) {
            Some(entry) => entry.1 = id.to_owned(),
            None => self.biomes.push((land, id.to_owned())),
        }
    }
}

/// What a biome is painted with. An empty name leaves the ground as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiomePaint {
    /// The biome's id (`minecraft:desert`).
    pub biome: String,
    /// Terrain of its ground, of its cliffs and of what lies under its
    /// water, by name.
    pub ground: String,
    pub cliffs: String,
    pub underwater: String,
}

/// The terrains of every biome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaintRules {
    /// Slopes at least this steep (degrees) get the biome's cliffs.
    pub steep: f32,
    /// Columns whose biome is left on automatic are painted as the biome
    /// the automatic biomes would give them.
    pub read_the_land: bool,
    pub biomes: Vec<BiomePaint>,
}

impl Default for PaintRules {
    fn default() -> Self {
        let p = |biome: &str, ground: &str, cliffs: &str, underwater: &str| BiomePaint {
            biome: format!("minecraft:{biome}"),
            ground: ground.to_owned(),
            cliffs: cliffs.to_owned(),
            underwater: underwater.to_owned(),
        };
        let mut biomes = Vec::new();
        for b in ["plains", "sunflower_plains", "meadow", "forest", "flower_forest", "birch_forest", "dark_forest", "pale_garden", "cherry_grove"] {
            biomes.push(p(b, "Grass", "Stone", "Dirt"));
        }
        biomes.extend([
            p("taiga", "Grass", "Stone", "Gravel"),
            p("old_growth_pine_taiga", "Podzol", "Stone", "Gravel"),
            p("snowy_plains", "Snow", "Stone", "Gravel"),
            p("snowy_taiga", "Snow", "Stone", "Gravel"),
            p("grove", "Snow", "Stone", "Gravel"),
            p("snowy_slopes", "Snow", "Stone", "Gravel"),
            p("frozen_peaks", "Snow", "Ice", "Gravel"),
            p("jagged_peaks", "Snow", "Stone", "Gravel"),
            p("stony_peaks", "Stone", "Calcite", "Gravel"),
            p("windswept_hills", "Grass", "Stone", "Gravel"),
            p("savanna", "Grass", "Coarse Dirt", "Dirt"),
            p("desert", "Sand", "Sandstone", "Sand"),
            p("badlands", "Red Sand", "Terracotta", "Red Sand"),
            p("jungle", "Grass", "Moss", "Dirt"),
            p("bamboo_jungle", "Grass", "Moss", "Dirt"),
            p("swamp", "Grass", "Dirt", "Mud"),
            p("mangrove_swamp", "Mud", "Mud", "Mud"),
            p("mushroom_fields", "Mycelium", "Stone", "Dirt"),
            p("beach", "Sand", "Stone", "Sand"),
            p("snowy_beach", "Snow", "Stone", "Sand"),
            p("stony_shore", "Stone", "Stone", "Gravel"),
            p("river", "Grass", "Stone", "Sand"),
            p("frozen_river", "Snow", "Stone", "Gravel"),
            p("ocean", "Sand", "Stone", "Gravel"),
            p("deep_ocean", "Sand", "Stone", "Gravel"),
            p("warm_ocean", "Sand", "Stone", "Sand"),
            p("lukewarm_ocean", "Sand", "Stone", "Sand"),
            p("cold_ocean", "Gravel", "Stone", "Gravel"),
            p("frozen_ocean", "Snow", "Stone", "Gravel"),
        ]);
        Self {
            steep: 45.0,
            read_the_land: true,
            biomes,
        }
    }
}

impl PaintRules {
    pub fn of(&self, biome: &str) -> Option<&BiomePaint> {
        self.biomes.iter().find(|p| p.biome == biome)
    }

    /// The paint of a biome, to change; made empty if it has none yet.
    pub fn of_mut(&mut self, biome: &str) -> &mut BiomePaint {
        let k = match self.biomes.iter().position(|p| p.biome == biome) {
            Some(k) => k,
            None => {
                self.biomes.push(BiomePaint {
                    biome: biome.to_owned(),
                    ground: String::new(),
                    cliffs: String::new(),
                    underwater: String::new(),
                });
                self.biomes.len() - 1
            }
        };
        &mut self.biomes[k]
    }
}

/// Both tables: the biome of the land, and the paint of the biomes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoRules {
    pub land: BiomeRules,
    pub paint: PaintRules,
}

/// The rules against a world's registry and sea: ready to read columns.
pub struct Reader<'a> {
    rules: &'a AutoRules,
    sea: f32,
    grounds: Vec<Ground>,
    /// Biome of each kind of land (`None`: not in this world's registry).
    biome_of: Vec<Option<BiomeId>>,
    /// Ground, cliffs and underwater terrain of each biome of the registry.
    paint_of: Vec<[Option<TerrainId>; 3]>,
}

/// One column, as the automatic tools see it.
#[derive(Debug, Clone, Copy)]
struct Col {
    height: f32,
    terrain: u16,
    biome: u16,
    /// Water over the ground, in blocks (0: dry), and its surface.
    depth: f32,
    level: f32,
}

/// The columns of a rectangle and of a margin around it, with what the
/// automatic tools need of their surroundings.
pub struct Survey {
    /// The rectangle asked for.
    pub rect: BlockRect,
    outer: BlockRect,
    cols: Vec<Option<Col>>,
    /// For wet columns: whether banks close them in on both sides.
    /// For dry ones: the level of the water a few blocks away, if any.
    river: Vec<bool>,
    water_near: Vec<f32>,
}

/// Blocks from the water within which ground can be its shore.
const SHORE_REACH: i32 = 16;
/// Sand is a beach up to this many times the shore's height: beaches are
/// wider, and rise higher, than a rocky shore.
const BEACH_RISE: f32 = 2.5;

impl<'a> Reader<'a> {
    pub fn new(rules: &'a AutoRules, reg: &Registry, sea: i32) -> Self {
        let biome_of = Land::ALL.iter().map(|l| reg.biome_by_id(rules.land.biome_of(*l))).collect();
        let terrain = |name: &str| if name.is_empty() { None } else { reg.terrain_by_name(name) };
        let paint_of = reg
            .biomes
            .iter()
            .map(|b| match rules.paint.of(&b.id) {
                Some(p) => [terrain(&p.ground), terrain(&p.cliffs), terrain(&p.underwater)],
                None => [None; 3],
            })
            .collect();
        Self {
            rules,
            sea: sea as f32,
            grounds: Ground::of_terrains(reg),
            biome_of,
            paint_of,
        }
    }

    /// The margin a survey needs around what it reads.
    fn margin(&self) -> i32 {
        self.rules.land.river_width.clamp(4, 64).max(SHORE_REACH).max(SLOPE_REACH)
    }

    /// Read the columns of `rect`.
    pub fn survey(&self, dim: &Dimension, rect: BlockRect) -> Survey {
        Survey::gather(dim, rect, self.margin(), self.rules.land.river_width.clamp(4, 64))
    }

    /// The kind of land of column `(x, z)`; `None` off the map.
    pub fn land(&self, s: &Survey, x: i32, z: i32) -> Option<Land> {
        let c = s.col(x, z)?;
        let r = &self.rules.land;
        if c.depth > 0.0 {
            return Some(if s.river[s.index(x, z)] {
                Land::River
            } else if c.depth >= r.deep_water {
                Land::DeepWater
            } else {
                Land::Water
            });
        }
        let ground = self.grounds.get(c.terrain as usize).copied().unwrap_or(Ground::Grass);
        let slope = trend_slope(|dx, dz| s.col(x + dx, z + dz).map(|n| n.height), SLOPE_REACH, 1.0);
        let steep = slope >= r.steep;
        let near = s.water_near[s.index(x, z)];
        if !near.is_nan() {
            let rise = c.height - near;
            match ground {
                Ground::Sand if rise <= r.shore_height * BEACH_RISE => {
                    return Some(if steep { Land::StonyShore } else { Land::Beach });
                }
                Ground::Snow if rise <= r.shore_height => return Some(Land::SnowyShore),
                Ground::Rock if rise <= r.shore_height => return Some(Land::StonyShore),
                // Grass down to the water is no beach.
                _ => {}
            }
        }
        let over = c.height - self.sea;
        let rocky = || if over >= r.mountains { Land::StonyPeaks } else { Land::RockyHills };
        Some(match ground {
            Ground::Sand => Land::Desert,
            Ground::RedSand => Land::Badlands,
            Ground::Mycelium => Land::Mushroom,
            Ground::Mud => Land::Swamp,
            Ground::Moss => Land::Jungle,
            Ground::ForestFloor => Land::Woodland,
            Ground::Snow => {
                if over >= r.peaks {
                    Land::FrozenPeaks
                } else if over >= r.mountains || slope >= r.steep * 0.6 {
                    Land::SnowySlopes
                } else {
                    Land::SnowyPlains
                }
            }
            Ground::Rock => rocky(),
            Ground::Grass if steep => rocky(),
            Ground::Grass => {
                if over >= r.mountains {
                    Land::Highlands
                } else if over >= r.hills {
                    Land::Hills
                } else {
                    Land::Plains
                }
            }
        })
    }

    /// The biome the land of column `(x, z)` calls for.
    pub fn biome(&self, s: &Survey, x: i32, z: i32) -> Option<BiomeId> {
        self.biome_of[self.land(s, x, z)? as usize]
    }

    /// The terrain the biome of column `(x, z)` calls for; `None` to leave
    /// the column as it is.
    pub fn terrain(&self, s: &Survey, x: i32, z: i32) -> Option<TerrainId> {
        let c = s.col(x, z)?;
        let biome = match c.biome {
            0 if self.rules.paint.read_the_land => self.biome(s, x, z)?.0,
            0 => return None,
            b => b,
        };
        let [ground, cliffs, underwater] = *self.paint_of.get(biome as usize)?;
        if c.depth > 0.0 {
            return underwater;
        }
        let slope = trend_slope(|dx, dz| s.col(x + dx, z + dz).map(|n| n.height), SLOPE_REACH, 1.0);
        if slope >= self.rules.paint.steep { cliffs.or(ground) } else { ground }
    }
}

impl Survey {
    fn gather(dim: &Dimension, rect: BlockRect, margin: i32, river_width: i32) -> Self {
        let outer = rect.expand(margin);
        let (w, d) = (outer.width() as usize, outer.depth() as usize);
        let tiles: Vec<TilePos> = outer.tiles().collect();
        let parts: Vec<Vec<(usize, Col)>> = tiles
            .par_iter()
            .map(|&pos| {
                let Some(stack) = TileStack::new(dim, pos, false) else { return Vec::new() };
                let clip = outer.intersect(&pos.rect());
                let mut spans: Vec<Span> = Vec::new();
                let mut out = Vec::with_capacity((clip.width() * clip.depth()).max(0) as usize);
                let ground = stack.ground();
                for z in clip.z0..clip.z1 {
                    for x in clip.x0..clip.x1 {
                        let cell = local_index(x, z);
                        let Some((height, terrain)) = stack.surface(cell, &mut spans) else { continue };
                        let fluid = ground.fluid.get(cell);
                        let level = if fluid == NO_FLUID { f32::NAN } else { fluid as f32 };
                        let depth = if level > height { level - height } else { 0.0 };
                        let col = Col {
                            height,
                            terrain: terrain.0,
                            biome: ground.biome.get(cell),
                            depth,
                            level,
                        };
                        out.push(((z - outer.z0) as usize * w + (x - outer.x0) as usize, col));
                    }
                }
                out
            })
            .collect();
        let mut cols = vec![None; w * d];
        for part in parts {
            for (i, c) in part {
                cols[i] = Some(c);
            }
        }
        // Along every row and every column of the grid: how far the nearest
        // bank is each way (for the water), and the level of the nearest
        // water (for the ground).
        let wet = |c: &Option<Col>| c.is_some_and(|c| c.depth > 0.0);
        let dry = |c: &Option<Col>| c.is_some_and(|c| c.depth <= 0.0);
        let cap = river_width + 1;
        let mut banks = vec![[cap; 4]; w * d];
        let mut water_near = vec![f32::NAN; w * d];
        let mut near_dist = vec![i32::MAX; w * d];
        let mut sweep = |start: usize, step: isize, len: usize, side: usize| {
            let (mut to_bank, mut to_water, mut level) = (cap, i32::MAX, f32::NAN);
            for k in 0..len {
                let i = (start as isize + step * k as isize) as usize;
                let c = &cols[i];
                if dry(c) {
                    to_bank = 0;
                } else {
                    to_bank = (to_bank + 1).min(cap);
                }
                banks[i][side] = to_bank;
                if wet(c) {
                    to_water = 0;
                    level = c.map_or(f32::NAN, |c| c.level);
                } else {
                    to_water = to_water.saturating_add(1);
                }
                if to_water <= SHORE_REACH && to_water < near_dist[i] {
                    near_dist[i] = to_water;
                    water_near[i] = level;
                }
            }
        };
        for j in 0..d {
            sweep(j * w, 1, w, 0);
            sweep(j * w + w - 1, -1, w, 1);
        }
        for i in 0..w {
            sweep(i, w as isize, d, 2);
            sweep((d - 1) * w + i, -(w as isize), d, 3);
        }
        let river = (0..w * d)
            .map(|i| {
                let b = banks[i];
                wet(&cols[i]) && (b[0] + b[1] <= river_width || b[2] + b[3] <= river_width)
            })
            .collect();
        Self {
            rect,
            outer,
            cols,
            river,
            water_near,
        }
    }

    #[inline]
    fn index(&self, x: i32, z: i32) -> usize {
        (z - self.outer.z0) as usize * self.outer.width() as usize + (x - self.outer.x0) as usize
    }

    #[inline]
    fn col(&self, x: i32, z: i32) -> Option<Col> {
        let r = &self.outer;
        if x < r.x0 || z < r.z0 || x >= r.x1 || z >= r.z1 {
            return None;
        }
        self.cols[self.index(x, z)]
    }
}

/// Tiles worked on together, so that the map is never held twice.
const BATCH: usize = 256;

/// Run `choose` over every column of the ground and write what it gives to
/// the plane `plane` picks. Returns how many columns changed.
fn over_the_map(
    edit: &mut Edit,
    reader: &Reader,
    choose: impl Fn(&Survey, i32, i32) -> Option<u16> + Sync,
    plane: impl Fn(&mut crate::tile::Tile) -> &mut crate::plane::Plane<u16> + Sync,
) -> usize {
    let tiles: Vec<TilePos> = edit.dim().ground().positions().collect();
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = std::sync::atomic::AtomicUsize::new(0);
    for batch in tiles.chunks(BATCH) {
        let chosen: HashMap<TilePos, Vec<Option<u16>>> = batch
            .par_iter()
            .map(|&pos| {
                let rect = pos.rect();
                let survey = reader.survey(edit.dim(), rect);
                let mut out = vec![None; (rect.width() * rect.depth()) as usize];
                for z in rect.z0..rect.z1 {
                    for x in rect.x0..rect.x1 {
                        out[local_index(x, z)] = choose(&survey, x, z);
                    }
                }
                (pos, out)
            })
            .collect();
        edit.par_tiles(batch.iter().copied(), everywhere, false, |pos, _, tile| {
            let Some(values) = chosen.get(&pos) else { return };
            let mut n = 0;
            for (i, v) in values.iter().enumerate() {
                if let Some(v) = v
                    && plane(tile).get(i) != *v
                {
                    plane(tile).set(i, *v);
                    n += 1;
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    changed.into_inner()
}

/// Give every column of the map the biome its land calls for. Returns how
/// many columns changed.
pub fn biomes_over_the_map(edit: &mut Edit, reader: &Reader) -> usize {
    over_the_map(edit, reader, |s, x, z| reader.biome(s, x, z).map(|b| b.0), |t| &mut t.biome)
}

/// Paint every column of the map with the terrain its biome calls for.
/// Returns how many columns changed.
pub fn paint_over_the_map(edit: &mut Edit, reader: &Reader) -> usize {
    over_the_map(edit, reader, |s, x, z| reader.terrain(s, x, z).map(|t| t.0), |t| &mut t.terrain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::tile::Tile;
    use crate::world::{DimensionKind, World, WorldMeta};

    /// A world of one tile at sea level 62: the sea to the west, deep then
    /// shelving up to a sand beach, a plain of grass rising to the east,
    /// gently then steeply, to a mountain of stone, and a narrow channel
    /// across the plain.
    fn sample() -> World {
        let mut world = World::new(WorldMeta::default());
        let reg = world.registry.clone();
        let t = |name: &str| reg.terrain_by_name(name).unwrap().0;
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(64.0, t("Grass"), NO_FLUID);
        for z in 0..128i32 {
            for x in 0..128i32 {
                let i = local_index(x, z);
                let (h, terrain) = match x {
                    0..=19 => (20.0, "Gravel"),
                    20..=39 => (54.0 + (x - 20) as f32 * 0.4, "Sand"),
                    40..=47 => (63.0, "Sand"),
                    48..=79 => (66.0, "Grass"),
                    80..=89 => (66.0 + (x - 80) as f32 * 0.5, "Grass"),
                    90..=99 => (71.0 + (x - 90) as f32 * 2.0, "Grass"),
                    _ => (106.0 + (x - 100) as f32 * 4.0, "Stone"),
                };
                tile.top.set(i, h);
                tile.terrain.set(i, t(terrain));
                if h < 62.0 {
                    tile.fluid.set(i, 62);
                }
            }
        }
        // A channel ten blocks wide across the plain.
        for z in 60..70 {
            for x in 50..78 {
                let i = local_index(x, z);
                tile.top.set(i, 59.0);
                tile.fluid.set(i, 62);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        world.dims = vec![dim];
        world
    }

    #[test]
    fn grounds_are_told_from_their_blocks() {
        assert_eq!(Ground::of_block("minecraft:grass_block[snowy=false]"), Ground::Grass);
        assert_eq!(Ground::of_block("minecraft:red_sandstone"), Ground::RedSand);
        assert_eq!(Ground::of_block("minecraft:sandstone"), Ground::Sand);
        assert_eq!(Ground::of_block("minecraft:packed_ice"), Ground::Snow);
        assert_eq!(Ground::of_block("minecraft:deepslate[axis=y]"), Ground::Rock);
        assert_eq!(Ground::of_block("minecraft:podzol[snowy=false]"), Ground::ForestFloor);
        let reg = Registry::vanilla();
        let grounds = Ground::of_terrains(&reg);
        assert_eq!(grounds[reg.terrain_by_name("Sand").unwrap().0 as usize], Ground::Sand);
        assert_eq!(grounds[reg.terrain_by_name("Snow").unwrap().0 as usize], Ground::Snow);
        assert_eq!(grounds[reg.terrain_by_name("Rock").unwrap().0 as usize], Ground::Rock);
    }

    #[test]
    fn the_land_is_read() {
        let world = sample();
        let mut rules = AutoRules::default();
        rules.land.hills = 5.0;
        let reader = Reader::new(&rules, &world.registry, 62);
        let survey = reader.survey(&world.dims[0], BlockRect::new(0, 0, 128, 128));
        let land = |x: i32, z: i32| reader.land(&survey, x, z).unwrap();
        assert_eq!(land(5, 20), Land::DeepWater);
        assert_eq!(land(30, 20), Land::Water);
        assert_eq!(land(42, 20), Land::Beach, "sand at the water's edge");
        assert_eq!(land(60, 20), Land::Plains);
        assert_eq!(land(64, 65), Land::River, "banks on both sides");
        assert_eq!(land(64, 20), Land::Plains);
        assert_eq!(land(85, 20), Land::Hills);
        assert_eq!(land(120, 20), Land::StonyPeaks);
        // The steep grass before the rock is a cliff.
        assert_eq!(land(95, 20), Land::RockyHills, "a slope of 63 degrees");
        assert_eq!(reader.land(&survey, 500, 500), None, "off the map");
    }

    #[test]
    fn biomes_and_paint_go_over_the_map() {
        let mut world = sample();
        let reg = world.registry.clone();
        let rules = AutoRules::default();
        let mut h = History::default();
        h.begin("auto biome", 0);
        let changed = biomes_over_the_map(&mut h.edit(&mut world, 0), &Reader::new(&rules, &reg, 62));
        h.commit();
        assert!(changed > 10_000, "{changed}");
        let biome = |world: &World, x: i32, z: i32| {
            let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
            reg.biome(BiomeId(tile.biome.get(local_index(x, z)))).id.clone()
        };
        assert_eq!(biome(&world, 5, 20), "minecraft:deep_ocean");
        assert_eq!(biome(&world, 42, 20), "minecraft:beach");
        assert_eq!(biome(&world, 64, 65), "minecraft:river");
        assert_eq!(biome(&world, 60, 20), "minecraft:plains");
        // Again: nothing left to change.
        h.begin("auto biome", 0);
        assert_eq!(biomes_over_the_map(&mut h.edit(&mut world, 0), &Reader::new(&rules, &reg, 62)), 0);
        h.commit();

        // Paint: make the plain a desert, and let the paint follow.
        let desert = reg.biome_by_id("minecraft:desert").unwrap();
        h.begin("biome", 0);
        h.edit(&mut world, 0).tile_mut(TilePos::new(0, 0)).unwrap().biome = crate::plane::Plane::Uniform(desert.0);
        h.commit();
        h.begin("auto paint", 0);
        let painted = paint_over_the_map(&mut h.edit(&mut world, 0), &Reader::new(&rules, &reg, 62));
        h.commit();
        assert!(painted > 5_000, "{painted}");
        let terrain = |world: &World, x: i32, z: i32| {
            let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
            reg.terrain(TerrainId(tile.terrain.get(local_index(x, z)))).name.clone()
        };
        assert_eq!(terrain(&world, 60, 20), "Sand", "the ground of a desert");
        assert_eq!(terrain(&world, 95, 20), "Sandstone", "its cliffs");
        assert_eq!(terrain(&world, 30, 20), "Sand", "under its water");
        // A custom rule: deserts of gravel.
        let mut custom = rules.clone();
        custom.paint.of_mut("minecraft:desert").ground = "Gravel".into();
        h.begin("auto paint", 0);
        paint_over_the_map(&mut h.edit(&mut world, 0), &Reader::new(&custom, &reg, 62));
        h.commit();
        assert_eq!(terrain(&world, 60, 20), "Gravel");
    }

    #[test]
    fn automatic_columns_are_painted_as_their_land() {
        let mut world = sample();
        let reg = world.registry.clone();
        // No biome set: grass on the mountain's rock becomes what stony
        // peaks are made of, the channel's bed what rivers flow over.
        let rules = AutoRules::default();
        let mut h = History::default();
        h.begin("auto paint", 0);
        paint_over_the_map(&mut h.edit(&mut world, 0), &Reader::new(&rules, &reg, 62));
        h.commit();
        let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        let name = |x: i32, z: i32| reg.terrain(TerrainId(tile.terrain.get(local_index(x, z)))).name.clone();
        assert_eq!(name(64, 65), "Sand", "a river's bed");
        assert_eq!(name(60, 20), "Grass");
        // With the land not read, columns on automatic are left alone.
        let mut strict = rules.clone();
        strict.paint.read_the_land = false;
        let mut again = sample();
        h.begin("auto paint", 0);
        assert_eq!(paint_over_the_map(&mut h.edit(&mut again, 0), &Reader::new(&strict, &reg, 62)), 0);
        h.commit();
    }
}
