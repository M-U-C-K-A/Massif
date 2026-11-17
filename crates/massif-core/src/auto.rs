//! Automatic biomes and automatic painting.
//!
//! *Automatic biomes* read the land: every column is deep water, a river,
//! a beach, a plain, a snowy slope… from its water, its height over the
//! sea, its slope and what its ground is made of, and each kind of land has
//! a biome. *Automatic painting* goes the other way: every biome has a
//! ground, another for its cliffs and one for what lies under its water.
//! Both tables are settings, to be changed.

use crate::compose::{Span, TileStack};
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::filter::SLOPE_REACH;
use crate::history::Edit;
use crate::registry::{BiomeId, Registry, TerrainId};
use crate::tile::NO_FLUID;
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    /// The terrain the map was filled with (see [`base_terrain`]): read as
    /// grass, from the shape of the land, but at the water's edge.
    base: Option<u16>,
}

/// The terrain a map was filled with, when one other than grass covers
/// nearly all of its dry ground (as far as a sample of its tiles tells): a
/// ground nobody chose, which says nothing about the land. A map of sand
/// all over is no desert, nor one of stone all mountains.
pub fn base_terrain(dim: &Dimension, reg: &Registry) -> Option<TerrainId> {
    let mut tiles: Vec<TilePos> = dim.ground().positions().collect();
    if tiles.is_empty() {
        return None;
    }
    tiles.sort_unstable_by_key(|p| (p.z, p.x));
    let step = (tiles.len() / 256).max(1);
    let counts: HashMap<u16, usize> = tiles
        .par_iter()
        .step_by(step)
        .map(|&pos| {
            let mut counts: HashMap<u16, usize> = HashMap::new();
            if let Some(tile) = dim.ground().view(pos) {
                for i in (0..crate::coords::TILE_AREA).step_by(61) {
                    let fluid = tile.fluid.get(i);
                    if fluid != NO_FLUID && fluid as f32 > tile.top.get(i) {
                        continue;
                    }
                    *counts.entry(tile.terrain.get(i)).or_default() += 1;
                }
            }
            counts
        })
        .reduce(HashMap::new, |mut a, b| {
            for (k, v) in b {
                *a.entry(k).or_default() += v;
            }
            a
        });
    let total: usize = counts.values().sum();
    let (id, n) = counts.into_iter().max_by_key(|c| c.1)?;
    let grass = Ground::of_terrains(reg).get(id as usize) == Some(&Ground::Grass);
    (!grass && total >= 100 && n * 100 >= total * 85).then_some(TerrainId(id))
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
    /// Running sums over the grid (one row and one column more than it) of
    /// the dry columns and of the sandy ones among them.
    dry_sum: Vec<u32>,
    sand_sum: Vec<u32>,
    /// Running sums of what the slope is fitted from (see
    /// [`Survey::slope`]): columns, `x`, `z`, height, `x·height`,
    /// `z·height`, the coordinates from the grid's corner.
    moments: Vec<[f64; 6]>,
}

/// Blocks from the water within which ground can be its shore.
const SHORE_REACH: i32 = 16;
/// Sand is a desert when this share of the dry ground within this many
/// blocks is sand too: a patch of sand in a plain is no desert.
const DESERT_REACH: i32 = 48;
const DESERT_SHARE: f32 = 0.7;
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
            base: None,
        }
    }

    /// The same, the map filled with `base` (see [`base_terrain`]).
    pub fn with_base(mut self, base: Option<TerrainId>) -> Self {
        self.base = base.map(|t| t.0);
        self
    }

    /// The margin a survey needs around what it reads.
    fn margin(&self) -> i32 {
        self.rules.land.river_width.clamp(4, 64).max(SHORE_REACH).max(SLOPE_REACH).max(DESERT_REACH)
    }

    /// Read the columns of `rect`.
    pub fn survey(&self, dim: &Dimension, rect: BlockRect) -> Survey {
        Survey::gather(dim, rect, self.margin(), self.rules.land.river_width.clamp(4, 64), &self.grounds)
    }

    /// The kind of land of column `(x, z)`; `None` off the map.
    pub fn land(&self, s: &Survey, x: i32, z: i32) -> Option<Land> {
        let c = s.col(x, z)?;
        let r = &self.rules.land;
        if c.depth > 0.0 {
            // Water standing over the sea's level is a lake or a river's:
            // fresh water, not the sea.
            return Some(if s.river[s.index(x, z)] || c.level > self.sea + 0.5 {
                Land::River
            } else if c.depth >= r.deep_water {
                Land::DeepWater
            } else {
                Land::Water
            });
        }
        let own = self.grounds.get(c.terrain as usize).copied().unwrap_or(Ground::Grass);
        // The ground the map was filled with is read from the shape of the
        // land, as grass; at the water's edge it makes its own shore.
        let ground = if self.base == Some(c.terrain) { Ground::Grass } else { own };
        let slope = s.slope(x, z);
        let steep = slope >= r.steep;
        let near = s.water_near[s.index(x, z)];
        if !near.is_nan() {
            let rise = c.height - near;
            match own {
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
        let grassland = || {
            if steep {
                rocky()
            } else if over >= r.mountains {
                Land::Highlands
            } else if over >= r.hills {
                Land::Hills
            } else {
                Land::Plains
            }
        };
        Some(match ground {
            // Sand away from the water is a desert where sand is all
            // around; a patch of it in a plain is the plain.
            Ground::Sand if s.sandy(x, z) >= DESERT_SHARE => Land::Desert,
            Ground::Sand => grassland(),
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
            Ground::Grass => grassland(),
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
        let slope = s.slope(x, z);
        if slope >= self.rules.paint.steep { cliffs.or(ground) } else { ground }
    }
}

impl Survey {
    fn gather(dim: &Dimension, rect: BlockRect, margin: i32, river_width: i32, grounds: &[Ground]) -> Self {
        let outer = rect.expand(margin);
        let (w, d) = (outer.width() as usize, outer.depth() as usize);
        let mut cols: Vec<Option<Col>> = vec![None; w * d];
        // The grid in bands, one per row of tiles, filled in parallel, each
        // tile's planes read in one go.
        {
            let mut bands: Vec<(i32, &mut [Option<Col>])> = Vec::new();
            let mut rest: &mut [Option<Col>] = &mut cols;
            let mut z = outer.z0;
            while z < outer.z1 {
                let end = ((z.div_euclid(TILE_SIZE) + 1) * TILE_SIZE).min(outer.z1);
                let (band, tail) = std::mem::take(&mut rest).split_at_mut((end - z) as usize * w);
                bands.push((z, band));
                rest = tail;
                z = end;
            }
            bands.into_par_iter().for_each(|(z0, band)| {
                let tz = z0.div_euclid(TILE_SIZE);
                let mut spans: Vec<Span> = Vec::new();
                for tx in outer.x0.div_euclid(TILE_SIZE)..=(outer.x1 - 1).div_euclid(TILE_SIZE) {
                    let pos = TilePos::new(tx, tz);
                    let Some(stack) = TileStack::new(dim, pos, false) else { continue };
                    let clip = outer.intersect(&pos.rect());
                    let ground = stack.ground();
                    // The ground alone (most tiles): its planes as they are.
                    let flat = stack.is_ground_only().then(|| (ground.top.dense(), ground.terrain.dense()));
                    let (fluids, biomes) = (ground.fluid.dense(), ground.biome.dense());
                    for z in clip.z0..clip.z1 {
                        let row = &mut band[(z - z0) as usize * w..][..w];
                        for x in clip.x0..clip.x1 {
                            let cell = local_index(x, z);
                            let (height, terrain) = match &flat {
                                Some((tops, terrains)) => (tops[cell], TerrainId(terrains[cell])),
                                None => match stack.surface(cell, &mut spans) {
                                    Some(s) => s,
                                    None => continue,
                                },
                            };
                            let fluid = fluids[cell];
                            let level = if fluid == NO_FLUID { f32::NAN } else { fluid as f32 };
                            let depth = if level > height { level - height } else { 0.0 };
                            row[(x - outer.x0) as usize] = Some(Col {
                                height,
                                terrain: terrain.0,
                                biome: biomes[cell],
                                depth,
                                level,
                            });
                        }
                    }
                }
            });
        }

        // Along every row and every column of the grid: how far the nearest
        // bank is each way (for the water), and the level of the nearest
        // water (for the ground). Rows one by one; columns all together, a
        // row at a time, so that the grid is always read in order.
        const NONE: u8 = 0;
        const DRY: u8 = 1;
        const WET: u8 = 2;
        let state: Vec<u8> = cols
            .iter()
            .map(|c| match c {
                None => NONE,
                Some(c) if c.depth > 0.0 => WET,
                Some(_) => DRY,
            })
            .collect();
        let level_at = |i: usize| cols[i].map_or(f32::NAN, |c| c.level);
        let cap = river_width + 1;
        // Banks: the distances each way, summed per axis.
        let mut across = vec![0i32; w * d];
        let mut along = vec![0i32; w * d];
        let mut water_near = vec![f32::NAN; w * d];
        let mut near_dist = vec![i32::MAX; w * d];
        #[inline]
        fn step(s: u8, level: f32, bank: &mut i32, water: &mut i32, lvl: &mut f32, cap: i32) {
            *bank = if s == DRY { 0 } else { (*bank + 1).min(cap) };
            if s == WET {
                *water = 0;
                *lvl = level;
            } else {
                *water = water.saturating_add(1);
            }
        }
        across
            .par_chunks_mut(w)
            .zip(water_near.par_chunks_mut(w))
            .zip(near_dist.par_chunks_mut(w))
            .enumerate()
            .for_each(|(z, ((banks, near), dist))| {
                for backward in [false, true] {
                    let (mut bank, mut water, mut lvl) = (cap, i32::MAX, f32::NAN);
                    for k in 0..w {
                        let x = if backward { w - 1 - k } else { k };
                        let i = z * w + x;
                        step(state[i], level_at(i), &mut bank, &mut water, &mut lvl, cap);
                        banks[x] += bank;
                        if water <= SHORE_REACH && water < dist[x] {
                            dist[x] = water;
                            near[x] = lvl;
                        }
                    }
                }
            });
        for upward in [false, true] {
            let (mut bank, mut water, mut lvl) = (vec![cap; w], vec![i32::MAX; w], vec![f32::NAN; w]);
            for k in 0..d {
                let z = if upward { d - 1 - k } else { k };
                for x in 0..w {
                    let i = z * w + x;
                    step(state[i], level_at(i), &mut bank[x], &mut water[x], &mut lvl[x], cap);
                    along[i] += bank[x];
                    if water[x] <= SHORE_REACH && water[x] < near_dist[i] {
                        near_dist[i] = water[x];
                        water_near[i] = lvl[x];
                    }
                }
            }
        }
        let river = (0..w * d)
            .map(|i| state[i] == WET && (across[i] <= river_width || along[i] <= river_width))
            .collect();

        // How much of the dry ground around is sand, and what the slope is
        // fitted from: running sums, read back over any square.
        let row = w + 1;
        let mut dry_sum = vec![0u32; row * (d + 1)];
        let mut sand_sum = vec![0u32; row * (d + 1)];
        let mut moments = vec![[0f64; 6]; row * (d + 1)];
        for z in 0..d {
            let (mut dry_run, mut sand_run) = (0u32, 0u32);
            let mut run = [0f64; 6];
            for x in 0..w {
                if let Some(c) = &cols[z * w + x] {
                    if c.depth <= 0.0 {
                        dry_run += 1;
                        if grounds.get(c.terrain as usize) == Some(&Ground::Sand) {
                            sand_run += 1;
                        }
                    }
                    let (fx, fz, h) = (x as f64, z as f64, c.height as f64);
                    run[0] += 1.0;
                    run[1] += fx;
                    run[2] += fz;
                    run[3] += h;
                    run[4] += fx * h;
                    run[5] += fz * h;
                }
                let (above, at) = (z * row + x + 1, (z + 1) * row + x + 1);
                dry_sum[at] = dry_sum[above] + dry_run;
                sand_sum[at] = sand_sum[above] + sand_run;
                let m = moments[above];
                moments[at] = std::array::from_fn(|k| m[k] + run[k]);
            }
        }
        Self {
            rect,
            outer,
            cols,
            river,
            water_near,
            dry_sum,
            sand_sum,
            moments,
        }
    }

    /// The slope, in degrees, of the plane that best fits the heights within
    /// [`SLOPE_REACH`] of `(x, z)`: [`crate::filter::trend_slope`], read from running sums
    /// rather than added up column by column.
    fn slope(&self, x: i32, z: i32) -> f32 {
        let Some(c) = self.col(x, z) else { return 0.0 };
        let r = &self.outer;
        let (w, d) = (r.width(), r.depth());
        let (lx, lz) = (x - r.x0, z - r.z0);
        let x0 = (lx - SLOPE_REACH).clamp(0, w) as usize;
        let x1 = (lx + SLOPE_REACH + 1).clamp(0, w) as usize;
        let z0 = (lz - SLOPE_REACH).clamp(0, d) as usize;
        let z1 = (lz + SLOPE_REACH + 1).clamp(0, d) as usize;
        let row = w as usize + 1;
        let (a, b, e, f) = (self.moments[z1 * row + x1], self.moments[z0 * row + x0], self.moments[z0 * row + x1], self.moments[z1 * row + x0]);
        let m: [f64; 6] = std::array::from_fn(|k| a[k] + b[k] - e[k] - f[k]);
        let (cx, cz, ch) = (lx as f64, lz as f64, c.height as f64);
        // Σ dx (h − centre) over the columns there are; missing ones count
        // as level with the centre, as trend_slope has it.
        let sx = (m[4] - cx * m[3]) - ch * (m[1] - cx * m[0]);
        let sz = (m[5] - cz * m[3]) - ch * (m[2] - cz * m[0]);
        let side = (2 * SLOPE_REACH + 1) as f64;
        let weight = side * (1..=SLOPE_REACH).map(|k| 2.0 * (k * k) as f64).sum::<f64>();
        ((sx / weight).hypot(sz / weight)).atan().to_degrees() as f32
    }

    /// The share of sand in the dry ground within [`DESERT_REACH`] blocks
    /// of `(x, z)` (as far as the survey reaches).
    fn sandy(&self, x: i32, z: i32) -> f32 {
        let r = &self.outer;
        let (w, d) = (r.width(), r.depth());
        let x0 = (x - DESERT_REACH - r.x0).clamp(0, w) as usize;
        let x1 = (x + DESERT_REACH + 1 - r.x0).clamp(0, w) as usize;
        let z0 = (z - DESERT_REACH - r.z0).clamp(0, d) as usize;
        let z1 = (z + DESERT_REACH + 1 - r.z0).clamp(0, d) as usize;
        let row = w as usize + 1;
        let sum = |s: &[u32]| s[z1 * row + x1] + s[z0 * row + x0] - s[z0 * row + x1] - s[z1 * row + x0];
        let dry = sum(&self.dry_sum);
        if dry == 0 { 0.0 } else { sum(&self.sand_sum) as f32 / dry as f32 }
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

/// Tiles read together, as squares this many tiles on a side: one survey,
/// and one margin around it, for all of them.
const BLOCK: i32 = 4;
/// Squares worked on together; between two batches the map is cooled down
/// again, so that a map of any size is never decoded whole.
const BATCH: usize = 96;
/// Tiles of the edited level left decoded after each batch.
const KEEP_HOT: usize = 2048;

/// How far a pass over the whole map is, and how to stop it.
#[derive(Debug, Default)]
pub struct Steps {
    /// Tiles done, and to do.
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl Steps {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 { 0.0 } else { self.done.load(Ordering::Relaxed) as f32 / total as f32 }
    }
}

/// What a pass over the whole map changed.
#[derive(Debug, Default)]
pub struct Changes {
    /// Columns changed, and the tiles they are in.
    pub columns: usize,
    pub tiles: Vec<TilePos>,
    /// Stopped before the end.
    pub cancelled: bool,
}

/// Run `choose` over every column of the ground and write what it gives to
/// the plane `plane` picks.
fn over_the_map(
    edit: &mut Edit,
    reader: &Reader,
    steps: &Steps,
    choose: impl Fn(&Survey, i32, i32) -> Option<u16> + Sync,
    plane: impl Fn(&mut crate::tile::Tile) -> &mut crate::plane::Plane<u16> + Sync,
) -> Changes {
    // The tiles by squares, the squares row by row: neighbours are read
    // together, and each tile is decoded about once.
    let selection = edit.dim().selection.clone();
    let mut squares: HashMap<(i32, i32), Vec<TilePos>> = HashMap::new();
    for pos in edit.dim().ground().positions().filter(|p| selection.as_ref().is_none_or(|s| s.tile(*p).is_some())) {
        squares.entry((pos.x.div_euclid(BLOCK), pos.z.div_euclid(BLOCK))).or_default().push(pos);
    }
    let mut squares: Vec<((i32, i32), Vec<TilePos>)> = squares.into_iter().collect();
    squares.sort_unstable_by_key(|((x, z), _)| (*z, *x));
    steps.total.store(squares.iter().map(|s| s.1.len()).sum(), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    let mut cancelled = false;
    for batch in squares.chunks(BATCH) {
        if steps.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        // Every tile the batch reads, its margin included, decoded once.
        let reach = (reader.margin() + TILE_SIZE - 1) / TILE_SIZE;
        let mut wanted: Vec<TilePos> = Vec::new();
        for ((sx, sz), _) in batch {
            for z in sz * BLOCK - reach..(sz + 1) * BLOCK + reach {
                for x in sx * BLOCK - reach..(sx + 1) * BLOCK + reach {
                    wanted.push(TilePos::new(x, z));
                }
            }
        }
        wanted.sort_unstable_by_key(|p| (p.z, p.x));
        wanted.dedup();
        edit.warm(&wanted);
        let chosen: HashMap<TilePos, Vec<Option<u16>>> = batch
            .par_iter()
            .flat_map_iter(|((sx, sz), tiles)| {
                let (x0, z0) = (sx * BLOCK * TILE_SIZE, sz * BLOCK * TILE_SIZE);
                let side = BLOCK * TILE_SIZE;
                let survey = reader.survey(edit.dim(), BlockRect::new(x0, z0, x0 + side, z0 + side));
                let out: Vec<(TilePos, Vec<Option<u16>>)> = tiles
                    .iter()
                    .map(|&pos| {
                        let rect = pos.rect();
                        let mut out = vec![None; crate::coords::TILE_AREA];
                        for z in rect.z0..rect.z1 {
                            for x in rect.x0..rect.x1 {
                                out[local_index(x, z)] = choose(&survey, x, z);
                            }
                        }
                        (pos, out)
                    })
                    .collect();
                steps.done.fetch_add(tiles.len(), Ordering::Relaxed);
                out
            })
            .collect();
        let positions: Vec<TilePos> = batch.iter().flat_map(|s| s.1.iter().copied()).collect();
        let edited = std::sync::Mutex::new(Vec::new());
        edit.par_tiles(positions, everywhere, false, |pos, _, tile| {
            let Some(values) = chosen.get(&pos) else { return };
            let mut n = 0;
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            let (ox, oz) = pos.origin();
            for (i, v) in values.iter().enumerate() {
                if let Some(c) = cells
                    && !crate::selection::lets(c.get(i), ox + (i % TILE_SIZE as usize) as i32, oz + (i / TILE_SIZE as usize) as i32)
                {
                    continue;
                }
                if let Some(v) = v
                    && plane(tile).get(i) != *v
                {
                    plane(tile).set(i, *v);
                    n += 1;
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, Ordering::Relaxed);
                edited.lock().expect("no panic while held").push(pos);
            }
        });
        touched.extend(edited.into_inner().expect("no panic while held"));
        edit.cool(KEEP_HOT);
    }
    Changes {
        columns: changed.into_inner(),
        tiles: touched,
        cancelled,
    }
}

/// Give every column of the map the biome its land calls for. Returns how
/// many columns changed.
pub fn biomes_over_the_map(edit: &mut Edit, reader: &Reader) -> usize {
    biomes_over_the_map_with(edit, reader, &Steps::default()).columns
}

/// [`biomes_over_the_map`], telling how far it is, and stopping when asked.
pub fn biomes_over_the_map_with(edit: &mut Edit, reader: &Reader, steps: &Steps) -> Changes {
    over_the_map(edit, reader, steps, |s, x, z| reader.biome(s, x, z).map(|b| b.0), |t| &mut t.biome)
}

/// Paint every column of the map with the terrain its biome calls for.
/// Returns how many columns changed.
pub fn paint_over_the_map(edit: &mut Edit, reader: &Reader) -> usize {
    paint_over_the_map_with(edit, reader, &Steps::default()).columns
}

/// [`paint_over_the_map`], telling how far it is, and stopping when asked.
pub fn paint_over_the_map_with(edit: &mut Edit, reader: &Reader, steps: &Steps) -> Changes {
    over_the_map(edit, reader, steps, |s, x, z| reader.terrain(s, x, z).map(|t| t.0), |t| &mut t.terrain)
}

/// What an automatic tool gives the columns of the map — their biome, or
/// their terrain —, tile by tile, worked out as asked for and kept: a brush
/// going back and forth, or a preview following the pointer, reads the land
/// once. Tiles are read by squares, each with the margin around it, so that
/// a brush of any size takes little memory at a time.
#[derive(Debug, Default)]
pub struct Classes {
    biomes: bool,
    tiles: HashMap<TilePos, Box<[u16]>>,
}

/// A column the rules leave as it is.
const UNSET: u16 = u16::MAX;
/// Tiles kept (64 MiB); past them, those away from what is asked are
/// forgotten.
const KEPT: usize = 2048;

impl Classes {
    /// For Auto biome (`biomes`), or Auto paint.
    pub fn new(biomes: bool) -> Self {
        Self {
            biomes,
            tiles: HashMap::new(),
        }
    }

    pub fn biomes(&self) -> bool {
        self.biomes
    }

    /// Tiles worked out.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Work out the tiles of `rect` not known yet.
    pub fn ensure(&mut self, dim: &Dimension, reader: &Reader, rect: BlockRect) {
        let missing: Vec<TilePos> = rect.tiles().filter(|p| !self.tiles.contains_key(p) && dim.ground().contains(*p)).collect();
        if missing.is_empty() {
            return;
        }
        if self.tiles.len() + missing.len() > KEPT {
            let (lo, hi) = (TilePos::containing(rect.x0, rect.z0), TilePos::containing(rect.x1 - 1, rect.z1 - 1));
            self.tiles.retain(|p, _| (lo.x..=hi.x).contains(&p.x) && (lo.z..=hi.z).contains(&p.z));
        }
        let mut squares: HashMap<(i32, i32), Vec<TilePos>> = HashMap::new();
        for pos in missing {
            squares.entry((pos.x.div_euclid(BLOCK), pos.z.div_euclid(BLOCK))).or_default().push(pos);
        }
        let biomes = self.biomes;
        let done: Vec<(TilePos, Box<[u16]>)> = squares
            .into_par_iter()
            .flat_map_iter(|(_, tiles)| {
                // One survey for the square's missing tiles, and the margin
                // around them.
                let (x0, z0) = (tiles.iter().map(|p| p.x).min().unwrap(), tiles.iter().map(|p| p.z).min().unwrap());
                let (x1, z1) = (tiles.iter().map(|p| p.x).max().unwrap() + 1, tiles.iter().map(|p| p.z).max().unwrap() + 1);
                let survey = reader.survey(dim, BlockRect::new(x0 * TILE_SIZE, z0 * TILE_SIZE, x1 * TILE_SIZE, z1 * TILE_SIZE));
                tiles
                    .into_iter()
                    .map(|pos| {
                        let r = pos.rect();
                        let mut out = vec![UNSET; crate::coords::TILE_AREA].into_boxed_slice();
                        for z in r.z0..r.z1 {
                            for x in r.x0..r.x1 {
                                let v = if biomes { reader.biome(&survey, x, z).map(|b| b.0) } else { reader.terrain(&survey, x, z).map(|t| t.0) };
                                out[local_index(x, z)] = v.unwrap_or(UNSET);
                            }
                        }
                        (pos, out)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        self.tiles.extend(done);
    }

    /// What column `(x, z)` is to be given, if it was worked out and the
    /// rules give it something.
    #[inline]
    pub fn get(&self, x: i32, z: i32) -> Option<u16> {
        let v = self.tiles.get(&TilePos::containing(x, z))?[local_index(x, z)];
        (v != UNSET).then_some(v)
    }

    /// The values of a tile, [`u16::MAX`] where the rules leave the column
    /// as it is.
    pub fn tile(&self, pos: TilePos) -> Option<&[u16]> {
        self.tiles.get(&pos).map(|t| &t[..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::trend_slope;
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

    /// What the classes keep, tile by tile, is what a survey of the brush
    /// alone reads; on a larger map, read by squares, too.
    #[test]
    fn classes_keep_what_the_land_calls_for() {
        let world = crate::generate::generate(&crate::generate::GenParams {
            tiles_x: 10,
            tiles_z: 6,
            landform: crate::generate::Landform::Islands,
            seed: 5,
            ..Default::default()
        });
        let rules = AutoRules::default();
        let reader = Reader::new(&rules, &world.registry, world.meta.sea_level);
        let dim = &world.dims[0];
        for biomes in [true, false] {
            let mut classes = Classes::new(biomes);
            // Two rects, one across a square's edge, the second over the
            // first in part: what was read is not read again.
            for rect in [BlockRect::new(-200, -90, 140, 70), BlockRect::new(0, -300, 300, 20)] {
                classes.ensure(dim, &reader, rect);
                let survey = reader.survey(dim, rect);
                let mut seen = 0;
                for z in (rect.z0..rect.z1).step_by(3) {
                    for x in (rect.x0..rect.x1).step_by(3) {
                        let direct = if biomes { reader.biome(&survey, x, z).map(|b| b.0) } else { reader.terrain(&survey, x, z).map(|t| t.0) };
                        assert_eq!(classes.get(x, z), direct, "{} at {x}, {z}", if biomes { "biome" } else { "terrain" });
                        seen += direct.is_some() as usize;
                    }
                }
                assert!(seen > 100, "the land was read: {seen}");
            }
            assert!(!classes.is_empty());
        }
    }

    /// The slope read from running sums is the slope fitted column by
    /// column, edges of the map included.
    #[test]
    fn slopes_from_sums_are_the_fitted_ones() {
        let world = sample();
        let rules = AutoRules::default();
        let reader = Reader::new(&rules, &world.registry, 62);
        let survey = reader.survey(&world.dims[0], BlockRect::new(0, 0, 128, 128));
        for (x, z) in [(0, 0), (3, 64), (40, 10), (85, 127), (95, 20), (100, 60), (127, 127), (64, 65)] {
            let fitted = trend_slope(|dx, dz| survey.col(x + dx, z + dz).map(|n| n.height), SLOPE_REACH, 1.0);
            let summed = survey.slope(x, z);
            assert!((fitted - summed).abs() < 0.01, "at {x},{z}: {fitted} fitted, {summed} summed");
        }
    }

    /// A patch of sand in a plain is the plain; sand all around is a desert.
    #[test]
    fn a_patch_of_sand_is_no_desert() {
        let mut world = sample();
        let reg = world.registry.clone();
        let sand = reg.terrain_by_name("Sand").unwrap().0;
        {
            let tile = world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap();
            // Twelve blocks of sand on the plain, away from the water.
            for z in 20..32 {
                for x in 56..68 {
                    tile.terrain.set(local_index(x, z), sand);
                }
            }
        }
        let rules = AutoRules::default();
        let reader = Reader::new(&rules, &reg, 62);
        let survey = reader.survey(&world.dims[0], BlockRect::new(0, 0, 128, 128));
        assert_eq!(reader.land(&survey, 62, 26), Some(Land::Plains));
        // The whole tile of sand: a desert, away from the water.
        {
            let tile = world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap();
            for z in 0..128 {
                for x in 48..128 {
                    tile.terrain.set(local_index(x, z), sand);
                }
            }
        }
        let survey = reader.survey(&world.dims[0], BlockRect::new(0, 0, 128, 128));
        assert_eq!(reader.land(&survey, 90, 100), Some(Land::Desert));
    }

    /// A map filled with sand is read from the shape of its land: plains
    /// and hills, beaches by the water.
    #[test]
    fn a_map_filled_with_sand_is_no_desert() {
        let mut world = sample();
        let reg = world.registry.clone();
        let sand = reg.terrain_by_name("Sand").unwrap();
        world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap().terrain = crate::plane::Plane::Uniform(sand.0);
        let base = base_terrain(&world.dims[0], &reg);
        assert_eq!(base, Some(sand));
        let mut rules = AutoRules::default();
        rules.land.hills = 5.0;
        let reader = Reader::new(&rules, &reg, 62).with_base(base);
        let survey = reader.survey(&world.dims[0], BlockRect::new(0, 0, 128, 128));
        let land = |x: i32, z: i32| reader.land(&survey, x, z).unwrap();
        assert_eq!(land(60, 20), Land::Plains);
        assert_eq!(land(85, 20), Land::Hills);
        assert_eq!(land(42, 20), Land::Beach, "its sand by the water is a beach");
        // A map of grass has no fill to see through.
        assert_eq!(base_terrain(&sample().dims[0], &reg), None);
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
