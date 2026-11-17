//! Block palettes: what a map's built-in terrains are made of.
//!
//! The generator paints terrains by name (Grass, Rocky, Sand, Gravel,
//! Snow… by height and slope), and so do Lakes, Rivers and the rest; what a
//! terrain is made of is its recipe in the map's registry. A palette gives
//! the built-in terrains recipes of other blocks (the game's, a mod's, a
//! resource pack's: any block state), and the rock under them where caves
//! are carved and ores laid, so that the same map is made of other blocks
//! without a column of it being painted again. A terrain a palette leaves
//! out is the game's own; terrains of the player's own are never touched.
//!
//! Two palettes come with Massif: [`Palette::vanilla`] and Conquest
//! Reforged's ([`Palette::conquest`], data in
//! `assets/conquest_terrains.json`); players make their own from them.

use crate::registry::{BlockId, Mix, Registry, Stratum, TerrainDef, TerrainId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What the built-in terrains of a map are made of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    /// Recipes of built-in terrains, by their names; one left out is the
    /// game's.
    #[serde(default)]
    pub terrains: BTreeMap<String, Recipe>,
    /// The rock under the terrains, where caves are carved and ores laid
    /// (`None`: the game's stone), and the deep rock under Y 0 (`None`:
    /// deepslate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepslate: Option<String>,
    /// Colours of its blocks (the averages of their textures), for before
    /// the game's own are read.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<String, [u8; 3]>,
}

/// What a terrain is made of, from its surface down, by block states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub strata: Vec<Layer>,
    /// The surface block under water, when the top one is not (grass).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submerged_top: Option<String>,
}

/// One layer of a [`Recipe`]: its blocks with their weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub blocks: Vec<(String, u16)>,
    /// Thickness range in blocks; [`DEEP`] for all the way down.
    pub depth: (u16, u16),
    /// Patches about this many blocks across, rather than each block on
    /// its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patches: Option<f32>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::vanilla()
    }
}

/// A layer that goes all the way down.
pub const DEEP: u16 = u16::MAX;

/// The terrains the generator paints, by height and slope, along the water
/// and under it: what a palette sets first.
pub const GENERATED: [&str; 7] = ["Grass", "Rocky", "Sand", "Gravel", "Snow", "Basalt", "Blackstone"];

/// The built-in terrains a palette may make of other blocks: all but the
/// bare rock (which is the palette's rock) and bedrock.
pub fn slots() -> Vec<String> {
    Registry::vanilla().terrains.iter().map(|t| t.name.clone()).filter(|n| n != "Rock" && n != "Bedrock").collect()
}

impl Palette {
    /// The game's own blocks.
    pub fn vanilla() -> Self {
        Self { name: "Vanilla".into(), terrains: BTreeMap::new(), stone: None, deepslate: None, colors: BTreeMap::new() }
    }

    /// Conquest Reforged's natural blocks, several mixed in patches.
    pub fn conquest() -> Self {
        static CONQUEST: std::sync::OnceLock<Palette> = std::sync::OnceLock::new();
        CONQUEST.get_or_init(|| serde_json::from_str(include_str!("../assets/conquest_terrains.json")).expect("conquest_terrains.json")).clone()
    }

    /// The palettes that come with Massif.
    pub fn built_in() -> Vec<Palette> {
        vec![Self::vanilla(), Self::conquest()]
    }

    pub fn is_vanilla(&self) -> bool {
        self.terrains.is_empty() && self.stone.is_none() && self.deepslate.is_none()
    }

    /// Whether it is one that comes with Massif, unchanged.
    pub fn is_built_in(&self) -> bool {
        Self::built_in().contains(self)
    }

    /// The recipe of the terrain `name`: its own, or the game's.
    pub fn recipe(&self, name: &str) -> Option<Recipe> {
        self.terrains.get(name).cloned().or_else(|| vanilla_recipe(name))
    }

    /// The block states it uses, each once, in order.
    pub fn states(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let recipes = self.terrains.values().flat_map(|t| t.strata.iter().flat_map(|s| s.blocks.iter().map(|(b, _)| b.as_str())).chain(t.submerged_top.as_deref()));
        for s in recipes.chain(self.stone.as_deref()).chain(self.deepslate.as_deref()) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// The namespaces of the mods whose blocks it uses (`conquest`).
    pub fn namespaces(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.states().into_iter().map(|s| s.split_once(':').map_or("minecraft", |(ns, _)| ns)).filter(|ns| *ns != "minecraft").collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The game's recipe of the built-in terrain `name`.
pub fn vanilla_recipe(name: &str) -> Option<Recipe> {
    let reg = Registry::vanilla();
    let t = reg.terrain(reg.terrain_by_name(name)?);
    let state = |b: BlockId| reg.block(b).state.clone();
    Some(Recipe {
        strata: t
            .strata
            .iter()
            .map(|s| Layer {
                blocks: s.blocks.iter().map(|&(b, w)| (state(b), w)).collect(),
                depth: s.depth,
                patches: match s.mix {
                    Mix::Patches { size } => Some(size),
                    _ => None,
                },
            })
            .collect(),
        submerged_top: t.submerged_top.map(state),
    })
}

/// Give the built-in terrains of `reg` the recipes of `palette` (the
/// game's where it has none), and its rock. Their map colours follow
/// their top blocks. Returns how many terrains and rocks changed.
pub fn apply(reg: &mut Registry, palette: &Palette) -> usize {
    let vanilla = Registry::vanilla();
    let block = |reg: &mut Registry, state: &str| {
        let color = palette.colors.get(state).copied().or_else(|| reg.blocks.iter().chain(&vanilla.blocks).find(|b| b.state == state).map(|b| b.color)).unwrap_or([128, 128, 128]);
        reg.intern_block(state, color)
    };
    let mut changed = 0;
    for name in slots() {
        let Some(id) = reg.terrain_by_name(&name) else { continue };
        let Some(recipe) = palette.recipe(&name) else { continue };
        let strata = recipe
            .strata
            .iter()
            .filter(|l| !l.blocks.is_empty())
            .map(|l| Stratum {
                blocks: l.blocks.iter().map(|(s, w)| (block(reg, s), (*w).max(1))).collect(),
                depth: (l.depth.0.min(l.depth.1), l.depth.1.max(l.depth.0)),
                mix: l.patches.filter(|s| *s > 0.0).map_or(Mix::Grain, |size| Mix::Patches { size }),
            })
            .collect();
        let submerged_top = recipe.submerged_top.as_deref().map(|s| block(reg, s));
        let mut def = TerrainDef { strata, submerged_top, ..reg.terrain(id).clone() };
        // The game's terrains keep their own colours on the map.
        def.color = match (palette.terrains.contains_key(&name), vanilla.terrain_by_name(&name)) {
            (false, Some(v)) => vanilla.terrain(v).color,
            _ => top_color(reg, &def).unwrap_or(def.color),
        };
        if *reg.terrain(id) != def {
            reg.terrains[id.index()] = def;
            changed += 1;
        }
    }
    let stone = block(reg, palette.stone.as_deref().unwrap_or("minecraft:stone"));
    let deepslate = block(reg, palette.deepslate.as_deref().unwrap_or("minecraft:deepslate[axis=y]"));
    changed += (reg.well_known.stone != stone) as usize + (reg.well_known.deepslate != deepslate) as usize;
    reg.well_known.stone = stone;
    reg.well_known.deepslate = deepslate;
    changed
}

/// The colour of a terrain's top: its top blocks' by their weights.
fn top_color(reg: &Registry, def: &TerrainDef) -> Option<[u8; 3]> {
    let top = def.strata.first()?;
    let total: u32 = top.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
    let mut c = [0u32; 3];
    for &(id, w) in &top.blocks {
        let bc = reg.blocks.get(id.index()).map_or([128; 3], |b| b.color);
        for k in 0..3 {
            c[k] += bc[k] as u32 * w as u32;
        }
    }
    Some(c.map(|v| (v / total) as u8))
}

/// The palette the built-in terrains of `reg` are made of now, read back
/// from them, named `name`: to make one of a map's terrains.
pub fn read(reg: &Registry, name: &str) -> Palette {
    let mut palette = Palette::vanilla();
    palette.name = name.to_owned();
    let state = |b: BlockId| reg.block(b).state.clone();
    for slot in slots() {
        let Some(id) = reg.terrain_by_name(&slot) else { continue };
        if id == TerrainId::ROCK {
            continue;
        }
        let t = reg.terrain(id);
        let recipe = Recipe {
            strata: t
                .strata
                .iter()
                .map(|s| Layer {
                    blocks: s.blocks.iter().map(|&(b, w)| (state(b), w)).collect(),
                    depth: s.depth,
                    patches: match s.mix {
                        Mix::Patches { size } => Some(size),
                        _ => None,
                    },
                })
                .collect(),
            submerged_top: t.submerged_top.map(state),
        };
        if vanilla_recipe(&slot).as_ref() != Some(&recipe) {
            for s in recipe.strata.iter().flat_map(|l| l.blocks.iter().map(|b| &b.0)) {
                if let Some(b) = reg.blocks.iter().find(|b| b.state == *s) {
                    palette.colors.insert(s.clone(), b.color);
                }
            }
            palette.terrains.insert(slot, recipe);
        }
    }
    let (stone, deep) = (state(reg.well_known.stone), state(reg.well_known.deepslate));
    palette.stone = (stone != "minecraft:stone").then_some(stone);
    palette.deepslate = (deep != "minecraft:deepslate[axis=y]").then_some(deep);
    palette
}

#[cfg(test)]
mod tests {
    use super::*;

    type Strata = Vec<(Vec<(String, u16)>, (u16, u16), Mix)>;

    fn states(r: &Registry, d: &TerrainDef) -> Strata {
        d.strata.iter().map(|s| (s.blocks.iter().map(|b| (r.block(b.0).state.clone(), b.1)).collect(), s.depth, s.mix)).collect()
    }

    #[test]
    fn conquest_reads_and_names_built_in_terrains() {
        let reg = Registry::vanilla();
        let conquest = Palette::conquest();
        assert_eq!(conquest.name, "Conquest Reforged");
        let slots = slots();
        for name in conquest.terrains.keys() {
            assert!(slots.contains(name), "{name} is not a terrain a palette sets");
        }
        assert!(conquest.states().iter().filter(|s| s.starts_with("conquest:")).count() >= 20);
        assert_eq!(conquest.namespaces(), ["conquest"]);
        // Several blocks on the surface of the generator's terrains, in
        // patches.
        for name in ["Grass", "Rocky", "Sand", "Gravel", "Snow"] {
            let top = &conquest.terrains[name].strata[0];
            assert!(top.blocks.len() >= 2 && top.patches.is_some(), "{name}");
        }
        assert!(reg.terrain_by_name("Rock").is_some() && !slots.contains(&"Rock".to_owned()));
        assert!(Palette::vanilla().is_vanilla() && Palette::conquest().is_built_in());
    }

    #[test]
    fn a_palette_and_back_again() {
        let vanilla = Registry::vanilla();
        let mut reg = vanilla.clone();
        assert_eq!(apply(&mut reg, &Palette::vanilla()), 0, "a new map is the game's");
        assert_eq!(read(&reg, "Vanilla"), Palette::vanilla());
        let conquest = Palette::conquest();
        let n = apply(&mut reg, &conquest);
        assert_eq!(n, conquest.terrains.len());
        let grass = reg.terrain(reg.terrain_by_name("Grass").unwrap());
        assert!(grass.strata[0].blocks.iter().any(|b| reg.block(b.0).state.starts_with("conquest:")));
        assert!(matches!(grass.strata[0].mix, Mix::Patches { .. }));
        // Read back, it is the same palette (but its colours).
        let back = read(&reg, "Conquest Reforged");
        assert_eq!(back.terrains, conquest.terrains);
        // Applied twice, nothing more changes.
        assert_eq!(apply(&mut reg, &conquest), 0);
        // Back to the game's blocks: the terrains are the vanilla ones again
        // (the blocks they no longer use stay registered).
        assert_eq!(apply(&mut reg, &Palette::vanilla()), n);
        for t in &vanilla.terrains {
            let mine = reg.terrain(reg.terrain_by_name(&t.name).unwrap());
            assert_eq!(states(&reg, mine), states(&vanilla, t), "{}", t.name);
            assert_eq!(mine.color, t.color, "{}", t.name);
        }
        assert_eq!(reg.block(reg.well_known.stone).state, "minecraft:stone");
    }

    #[test]
    fn a_palette_of_ones_own() {
        let mut reg = Registry::vanilla();
        let mut mine = Palette::vanilla();
        mine.name = "Basalt coast".into();
        mine.terrains.insert("Sand".into(), Recipe { strata: vec![Layer { blocks: vec![("minecraft:gravel".into(), 2), ("othermod:black_sand".into(), 1)], depth: (2, 4), patches: Some(5.0) }], submerged_top: None });
        mine.stone = Some("minecraft:tuff".into());
        assert_eq!(mine.namespaces(), ["othermod"]);
        assert_eq!(apply(&mut reg, &mine), 2, "sand, and the rock");
        let sand = reg.terrain(reg.terrain_by_name("Sand").unwrap());
        assert_eq!(states(&reg, sand)[0].0, [("minecraft:gravel".to_owned(), 2), ("othermod:black_sand".to_owned(), 1)]);
        assert_eq!(reg.block(reg.well_known.stone).state, "minecraft:tuff");
        // Grass, which it leaves out, is the game's.
        let v = Registry::vanilla();
        assert_eq!(states(&reg, reg.terrain(reg.terrain_by_name("Grass").unwrap())), states(&v, v.terrain(v.terrain_by_name("Grass").unwrap())));
        // Saved and read again.
        let text = serde_json::to_string(&mine).unwrap();
        assert_eq!(serde_json::from_str::<Palette>(&text).unwrap(), mine);
        assert_eq!(read(&reg, "Basalt coast").terrains, mine.terrains);
    }
}
