//! The zones Paint terrain paints by height with: which terrain at which
//! height above the sea, on which slope, and under the water at which
//! depth. Presets (Plains, Mountain, Mesa…) or the map's own zones, kept
//! with the map.
//!
//! The zones are read in order, the first that takes a column paints it. A
//! column under water is never given a terrain that only grows in the air
//! (grass, podzol, mycelium, snow…): the zones made for under the water
//! take it, or else sand, clay, dirt and gravel by depth, as in the game.

use serde::{Deserialize, Serialize};

use crate::coords::{TILE_SIZE, local_index};
use crate::registry::{Registry, Stratum, TerrainDef, TerrainId};

/// The sets of zones to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// As a new map is painted: grass, sand by the water, rock and snow up
    /// high.
    #[default]
    Classic,
    Plains,
    Mountain,
    Mesa,
    Desert,
    Taiga,
    Coast,
    /// The map's own zones ([`ZoneRules::custom`]).
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 8] = [Preset::Classic, Preset::Plains, Preset::Mountain, Preset::Mesa, Preset::Desert, Preset::Taiga, Preset::Coast, Preset::Custom];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Classic => "Classic",
            Preset::Plains => "Plains",
            Preset::Mountain => "Mountain",
            Preset::Mesa => "Mesa",
            Preset::Desert => "Desert",
            Preset::Taiga => "Taiga",
            Preset::Coast => "Coast",
            Preset::Custom => "Custom",
        }
    }

    /// Its zones, in the order they are read.
    pub fn zones(self) -> Vec<Zone> {
        use Water::{Dry, Under};
        let z = |terrain: &str, water: Water, (low, high): (i32, i32), (min_slope, max_slope): (f32, f32)| Zone { terrain: terrain.to_owned(), water, low, high, min_slope, max_slope };
        const ALL: (i32, i32) = (LOWEST, HIGHEST);
        const FLAT: (f32, f32) = (0.0, 90.0);
        // Under the water, as the game lays the beds of its lakes and seas.
        let bed = |shallow: &str, deep: &str| vec![z(shallow, Under, (0, 3), FLAT), z("Clay", Under, (4, 6), FLAT), z("Dirt", Under, (7, 10), FLAT), z(deep, Under, (11, HIGHEST), FLAT)];
        match self {
            Preset::Classic => vec![
                z("Sand", Under, (0, 14), FLAT),
                z("Gravel", Under, (15, HIGHEST), FLAT),
                z("Sand", Dry, (LOWEST, 2), (0.0, 42.0)),
                z("Rock", Dry, ALL, (53.0, 90.0)),
                z("Snow", Dry, (116, HIGHEST), FLAT),
                z("Rocky", Dry, (76, HIGHEST), FLAT),
                z("Rocky", Dry, ALL, (46.0, 90.0)),
                z("Grass", Dry, ALL, FLAT),
            ],
            Preset::Plains => {
                let mut v = bed("Sand", "Gravel");
                v.extend([z("Sand", Dry, (LOWEST, 1), (0.0, 30.0)), z("Rocky", Dry, ALL, (50.0, 90.0)), z("Coarse Dirt", Dry, ALL, (38.0, 50.0)), z("Grass", Dry, ALL, FLAT)]);
                v
            }
            Preset::Mountain => {
                let mut v = bed("Gravel", "Gravel");
                v.extend([
                    z("Snow", Dry, (110, HIGHEST), (0.0, 50.0)),
                    z("Stone", Dry, ALL, (50.0, 90.0)),
                    z("Gravel", Dry, ALL, (38.0, 50.0)),
                    z("Rocky", Dry, (80, HIGHEST), FLAT),
                    z("Gravel", Dry, (LOWEST, 1), (0.0, 30.0)),
                    z("Grass", Dry, ALL, FLAT),
                ]);
                v
            }
            Preset::Mesa => {
                let mut v = vec![z("Red Sand", Under, (0, 4), FLAT), z("Terracotta", Under, (5, HIGHEST), FLAT), z("Red Sand", Dry, (LOWEST, 3), (0.0, 30.0)), z("Coarse Dirt", Dry, (70, HIGHEST), (0.0, 12.0)), z("Red Sand", Dry, ALL, (0.0, 22.0))];
                // Bands of terracotta up the slopes, of uneven widths.
                let colours = ["Orange Terracotta", "Terracotta", "Yellow Terracotta", "White Terracotta", "Red Terracotta", "Terracotta", "Brown Terracotta", "Light Gray Terracotta", "Orange Terracotta", "Red Terracotta"];
                let widths = [3, 2, 4, 1, 3, 5, 2, 3, 4, 2, 1, 3];
                let mut h = 4;
                let mut k = 0;
                while h < 160 {
                    let w = widths[k % widths.len()];
                    v.push(z(colours[k % colours.len()], Dry, (h, h + w - 1), FLAT));
                    h += w;
                    k += 1;
                }
                v.push(z("Terracotta", Dry, ALL, FLAT));
                v
            }
            Preset::Desert => vec![z("Sand", Under, (0, 6), FLAT), z("Sandstone", Under, (7, HIGHEST), FLAT), z("Sandstone", Dry, ALL, (40.0, 90.0)), z("Sand", Dry, ALL, FLAT)],
            Preset::Taiga => {
                let mut v = bed("Gravel", "Gravel");
                v.extend([
                    z("Stone", Dry, ALL, (50.0, 90.0)),
                    z("Snow", Dry, (40, HIGHEST), FLAT),
                    z("Gravel", Dry, (LOWEST, 1), (0.0, 30.0)),
                    z("Coarse Dirt", Dry, ALL, (35.0, 50.0)),
                    z("Podzol", Dry, ALL, FLAT),
                ]);
                v
            }
            Preset::Coast => vec![
                z("Sand", Under, (0, 5), FLAT),
                z("Gravel", Under, (6, HIGHEST), FLAT),
                z("Stone", Dry, ALL, (45.0, 90.0)),
                z("Sand", Dry, (LOWEST, 4), (0.0, 35.0)),
                z("Gravel", Dry, (LOWEST, 8), (35.0, 45.0)),
                z("Grass", Dry, ALL, FLAT),
            ],
            Preset::Custom => Vec::new(),
        }
    }
}

/// The lowest and highest heights a zone can name: as far as it goes.
pub const LOWEST: i32 = -1024;
pub const HIGHEST: i32 = 1024;

/// Where a zone is: on dry land, or under the water.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Water {
    Dry,
    Under,
}

/// One zone: a terrain, where it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    /// The terrain, by name: a terrain of the map, or one of the game's
    /// blocks made into one.
    pub terrain: String,
    pub water: Water,
    /// On dry land, the height of the top block above the sea's level;
    /// under the water, its depth under the water's surface (1: just
    /// under). Both ends in.
    pub low: i32,
    pub high: i32,
    /// The slope it takes, in degrees, from `min_slope` up to (not
    /// including) `max_slope`; 90 takes the steepest too.
    pub min_slope: f32,
    pub max_slope: f32,
}

impl Zone {
    fn takes(&self, wet: bool, at: f32, slope: f32) -> bool {
        (self.water == Water::Under) == wet && at >= self.low as f32 - 0.5 && at < self.high as f32 + 0.5 && slope >= self.min_slope && (slope < self.max_slope || self.max_slope >= 90.0)
    }
}

/// The map's rules: a preset, or its own zones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneRules {
    pub preset: Preset,
    /// The zones of [`Preset::Custom`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<Zone>,
    /// How ragged the edges between zones are, in blocks of height.
    pub ragged: f32,
}

impl Default for ZoneRules {
    fn default() -> Self {
        Self { preset: Preset::Classic, custom: Vec::new(), ragged: 3.0 }
    }
}

impl ZoneRules {
    pub fn of(preset: Preset) -> Self {
        Self { preset, ..Default::default() }
    }

    /// The zones read, in order.
    pub fn zones(&self) -> Vec<Zone> {
        match self.preset {
            Preset::Custom => self.custom.clone(),
            p => p.zones(),
        }
    }

    /// Edit the zones: the preset's become the map's own.
    pub fn make_custom(&mut self) {
        if self.preset != Preset::Custom {
            self.custom = self.preset.zones();
            self.preset = Preset::Custom;
        }
    }

    /// The zones with their terrains found in `reg` (the game's blocks the
    /// presets name added to it the first time). Zones of terrains it has
    /// not are left out.
    pub fn resolve(&self, reg: &mut Registry, seed: i64) -> Resolved {
        let mut zones = Vec::new();
        for zone in self.zones() {
            if let Some(t) = find_or_add(reg, &zone.terrain) {
                zones.push((zone, t));
            }
        }
        let get = |reg: &mut Registry, name: &str| find_or_add(reg, name).unwrap_or(TerrainId::ROCK);
        let bed = [get(reg, "Sand"), get(reg, "Clay"), get(reg, "Dirt"), get(reg, "Gravel")];
        let air_only = reg.terrains.iter().map(air_only).collect();
        let lava = [get(reg, "Blackstone"), get(reg, "Basalt")];
        Resolved { zones, bed, lava, air_only, ragged: self.ragged.clamp(0.0, 16.0), noise: crate::noise::Noise::new(seed as u64 ^ 0x2a0e_5e1d) }
    }
}

/// Blocks the presets name that a map may not have as terrains yet: their
/// states and colours.
const BLOCKS: [(&str, &str, [u8; 3]); 7] = [
    ("White Terracotta", "minecraft:white_terracotta", [210, 178, 161]),
    ("Orange Terracotta", "minecraft:orange_terracotta", [162, 84, 38]),
    ("Yellow Terracotta", "minecraft:yellow_terracotta", [186, 133, 35]),
    ("Red Terracotta", "minecraft:red_terracotta", [143, 61, 47]),
    ("Brown Terracotta", "minecraft:brown_terracotta", [77, 51, 36]),
    ("Light Gray Terracotta", "minecraft:light_gray_terracotta", [135, 107, 98]),
    ("Smooth Sandstone", "minecraft:smooth_sandstone", [223, 214, 170]),
];

/// The group of the terrains made of one real block (as the Blocks panel
/// makes them: found again by their block).
const BLOCK_GROUP: &str = "Blocks";

/// The terrain named `name` of `reg`, or one of [`BLOCKS`] added to it.
fn find_or_add(reg: &mut Registry, name: &str) -> Option<TerrainId> {
    if let Some(t) = reg.terrain_by_name(name) {
        return Some(t);
    }
    let &(name, state, color) = BLOCKS.iter().find(|b| b.0.eq_ignore_ascii_case(name))?;
    let block = reg.intern_block(state, color);
    if let Some(i) = reg.terrains.iter().position(|t| t.group == BLOCK_GROUP && t.strata.first().and_then(|s| s.blocks.first()).is_some_and(|b| b.0 == block)) {
        return Some(TerrainId(i as u16));
    }
    reg.terrains.push(TerrainDef { name: name.to_owned(), group: BLOCK_GROUP.to_owned(), color, strata: vec![Stratum { blocks: vec![(block, 1)], depth: (3, 5), mix: Default::default() }], submerged_top: None, biome_hint: None });
    Some(TerrainId((reg.terrains.len() - 1) as u16))
}

/// Whether a terrain only grows in the open air: grass, podzol, mycelium
/// (they turn to dirt under water: [`TerrainDef::submerged_top`]), moss,
/// snow.
pub fn air_only(def: &TerrainDef) -> bool {
    if def.submerged_top.is_some() {
        return true;
    }
    let name = def.name.to_lowercase();
    ["grass", "podzol", "mycelium", "snow", "moss"].iter().any(|w| name.split(|c: char| !c.is_alphanumeric()).any(|word| word == *w)) && !name.contains("block of")
}

/// [`ZoneRules`] ready to paint with.
#[derive(Debug, Clone)]
pub struct Resolved {
    zones: Vec<(Zone, TerrainId)>,
    /// Sand, clay, dirt and gravel: under the water where no zone says.
    bed: [TerrainId; 4],
    /// Under lava: blackstone, and basalt.
    lava: [TerrainId; 2],
    air_only: Vec<bool>,
    ragged: f32,
    noise: crate::noise::Noise,
}

/// A column, as the zones read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Column {
    pub x: i32,
    pub z: i32,
    /// The top block's height (its Y).
    pub y: i32,
    /// The slope, in degrees.
    pub slope: f32,
    /// The fluid's level standing over it, if any.
    pub fluid: Option<i32>,
}

impl Resolved {
    /// The terrain for a column, on a map whose sea is at `sea` (of lava
    /// with `lava`). `None`: no zone takes it, it keeps its own.
    pub fn terrain(&self, c: Column, sea: i32, lava: bool) -> Option<TerrainId> {
        // The blocks of water over the top block.
        let depth = c.fluid.map(|f| f - c.y - 1).filter(|d| *d >= 1);
        let jitter = if self.ragged > 0.0 { self.noise.fbm(c.x as f64 / 23.0, c.z as f64 / 23.0, 2, 2.0, 0.5) as f32 * self.ragged } else { 0.0 };
        if let Some(d) = depth {
            if lava {
                return Some(if jitter > 0.3 * self.ragged { self.lava[1] } else { self.lava[0] });
            }
            let at = d as f32 + jitter * 0.5;
            let found = self.zones.iter().find(|(z, _)| z.takes(true, at, c.slope)).map(|(_, t)| *t);
            return Some(match found {
                Some(t) if !self.is_air_only(t) => t,
                _ => self.bed_at(at),
            });
        }
        let at = (c.y - sea) as f32 + jitter;
        self.zones.iter().find(|(z, _)| z.takes(false, at, c.slope)).map(|(_, t)| *t)
    }

    fn is_air_only(&self, t: TerrainId) -> bool {
        self.air_only.get(t.index()).copied().unwrap_or(false)
    }

    /// The game's beds by depth: sand, then clay, dirt, gravel.
    fn bed_at(&self, depth: f32) -> TerrainId {
        let k = match depth {
            d if d < 3.5 => 0,
            d if d < 6.5 => 1,
            d if d < 10.5 => 2,
            _ => 3,
        };
        self.bed[k]
    }
}

/// The slope of a tile's column `i`, in degrees: from its neighbours in the
/// tile (on its edge, from the one inside).
pub fn slope_in(top: &crate::plane::Plane<f32>, x: i32, z: i32) -> f32 {
    let (lx, lz) = (x.rem_euclid(TILE_SIZE), z.rem_euclid(TILE_SIZE));
    let at = |dx: i32, dz: i32| {
        let (ax, az) = ((lx + dx).clamp(0, TILE_SIZE - 1), (lz + dz).clamp(0, TILE_SIZE - 1));
        top.get(local_index(ax, az))
    };
    let span = |a: i32, b: i32| (b - a).max(1) as f32;
    let (x0, x1) = ((lx - 1).max(0) - lx, (lx + 1).min(TILE_SIZE - 1) - lx);
    let (z0, z1) = ((lz - 1).max(0) - lz, (lz + 1).min(TILE_SIZE - 1) - lz);
    let gx = (at(x1, 0) - at(x0, 0)) / span(x0, x1);
    let gz = (at(0, z1) - at(0, z0)) / span(z0, z1);
    (gx * gx + gz * gz).sqrt().atan().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(y: i32, slope: f32, fluid: Option<i32>) -> Column {
        Column { x: 10, z: 20, y, slope, fluid }
    }

    /// No preset, and no zone of one's own, puts grass, snow or podzol
    /// under the water: the beds of the game take it.
    #[test]
    fn nothing_of_the_open_air_under_water() {
        let mut reg = Registry::vanilla();
        let custom = ZoneRules { preset: Preset::Custom, custom: vec![Zone { terrain: "Grass".into(), water: Water::Under, low: LOWEST, high: HIGHEST, min_slope: 0.0, max_slope: 90.0 }], ragged: 3.0 };
        for rules in Preset::ALL.iter().map(|p| ZoneRules::of(*p)).chain([custom]) {
            let r = rules.resolve(&mut reg, 7);
            for y in 20..200 {
                for depth in 1..40 {
                    for slope in [0.0, 30.0, 60.0] {
                        let t = r.terrain(Column { x: y * 7, z: depth * 13, ..column(y, slope, Some(y + 1 + depth)) }, 62, false);
                        if let Some(t) = t {
                            assert!(!air_only(reg.terrain(t)), "{:?}: {} under {depth} blocks of water", rules.preset, reg.terrain(t).name);
                        }
                    }
                }
            }
        }
    }

    /// Grass in a lake above the sea used to be painted: by height alone,
    /// it was dry land.
    #[test]
    fn a_lake_above_the_sea_has_a_bed() {
        let mut reg = Registry::vanilla();
        let r = ZoneRules { ragged: 0.0, ..Default::default() }.resolve(&mut reg, 1);
        let grass = reg.terrain_by_name("Grass").unwrap();
        assert_eq!(r.terrain(column(90, 0.0, None), 62, false), Some(grass));
        assert_eq!(r.terrain(column(90, 0.0, Some(93)), 62, false), reg.terrain_by_name("Sand"));
        assert_eq!(r.terrain(column(70, 0.0, Some(93)), 62, false), reg.terrain_by_name("Gravel"));
        assert_eq!(r.terrain(column(90, 0.0, Some(93)), 62, true), reg.terrain_by_name("Blackstone"), "under lava");
    }

    /// The presets paint what they say: the mountain's snow up high, stone
    /// on its cliffs; the mesa's bands; the desert's sand.
    #[test]
    fn presets_paint_their_zones() {
        let mut reg = Registry::vanilla();
        let name = |reg: &Registry, t: Option<TerrainId>| t.map(|t| reg.terrain(t).name.clone()).unwrap_or_default();
        let r = ZoneRules { ragged: 0.0, ..ZoneRules::of(Preset::Mountain) }.resolve(&mut reg, 1);
        assert_eq!(name(&reg, r.terrain(column(62 + 130, 10.0, None), 62, false)), "Snow");
        assert_eq!(name(&reg, r.terrain(column(62 + 30, 60.0, None), 62, false)), "Stone");
        assert_eq!(name(&reg, r.terrain(column(62 + 30, 40.0, None), 62, false)), "Gravel");
        assert_eq!(name(&reg, r.terrain(column(62 + 30, 10.0, None), 62, false)), "Grass");
        let r = ZoneRules { ragged: 0.0, ..ZoneRules::of(Preset::Mesa) }.resolve(&mut reg, 1);
        let bands: std::collections::HashSet<String> = (66..130).map(|y| name(&reg, r.terrain(column(y, 40.0, None), 62, false))).collect();
        assert!(bands.len() >= 6 && bands.iter().all(|b| b.contains("Terracotta")), "{bands:?}");
        assert_eq!(name(&reg, r.terrain(column(80, 5.0, None), 62, false)), "Red Sand");
        let r = ZoneRules { ragged: 0.0, ..ZoneRules::of(Preset::Desert) }.resolve(&mut reg, 1);
        assert_eq!(name(&reg, r.terrain(column(90, 5.0, None), 62, false)), "Sand");
        assert_eq!(name(&reg, r.terrain(column(90, 60.0, None), 62, false)), "Sandstone");
        // Each preset's terrains are in the map, or made from the game's.
        for p in Preset::ALL {
            for z in p.zones() {
                assert!(find_or_add(&mut reg, &z.terrain).is_some(), "{p:?}: {}", z.terrain);
            }
        }
    }

    /// A map's own zones are read in order, and saved with it.
    #[test]
    fn custom_zones_in_order_and_saved() {
        let mut rules = ZoneRules::of(Preset::Desert);
        rules.make_custom();
        assert_eq!(rules.custom, Preset::Desert.zones());
        rules.custom.insert(0, Zone { terrain: "Grass".into(), water: Water::Dry, low: 10, high: 20, min_slope: 0.0, max_slope: 90.0 });
        rules.ragged = 0.0;
        let back: ZoneRules = serde_json::from_str(&serde_json::to_string(&rules).unwrap()).unwrap();
        assert_eq!(back, rules);
        let mut reg = Registry::vanilla();
        let r = back.resolve(&mut reg, 1);
        assert_eq!(r.terrain(column(62 + 15, 0.0, None), 62, false), reg.terrain_by_name("Grass"));
        assert_eq!(r.terrain(column(62 + 25, 0.0, None), 62, false), reg.terrain_by_name("Sand"));
    }
}
