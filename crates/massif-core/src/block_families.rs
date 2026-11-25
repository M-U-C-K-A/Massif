//! Block families: which blocks are the layers, slab, stairs… of a full
//! block, as the mods themselves say it.
//!
//! The names alone ([`crate::shapes::family`]) find most of a block's
//! shapes (`stone` → `stone_slab`), but not those a mod names otherwise:
//! Conquest Reforged's `limestone_slab` is the slab of the game's `stone`,
//! its `black_hardened_clay_slab` that of `black_terracotta`, its
//! `grass_and_sand_layer` the layers of `sand`. A mod that adds shapes says
//! which block each is cut from in its stonecutter recipes (a block in, its
//! shape out: Conquest's 18,000 `*_sc.json`, and Conquest's C palette shows
//! the same groups). This table reads them, keeping only the links that
//! are a block's own shapes:
//!
//! * the shape's name is the block's with a shape's suffix (`fir_forest_floor`
//!   → `fir_forest_floor_layer`, `grass_block` → `conquest:grass_block_layer`);
//! * or its stem is the name the mod gives the block cut into it: Conquest
//!   calls the game's `stone` Limestone (its `assets/minecraft/lang`), and
//!   cuts it into `limestone_slab`; `podzol` is its Vibrant Autumnal Forest
//!   Floor, cut into `vibrant_autumnal_forest_floor_layer`.
//!
//! Recipes that turn a block into another one are left out: Conquest cuts
//! the game's grass block into taiga grass layers (the layers of
//! `taiga_grass`), and sand into grass and sand layers (sand is Sand). The table is built once for the
//! blocks read (the mods of a catalogue) and lives as long as them.

use crate::shapes::Shape;
use std::collections::HashMap;

/// The shapes of full blocks, as the mods' recipes link them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockFamilies {
    /// Of each full block, its shapes (by block id), the links by name
    /// first.
    shapes: HashMap<String, Vec<String>>,
    /// Of each shape, the full block it is of.
    full: HashMap<String, String>,
}

/// The suffixes of shapes that are none of Smooth terrain's, though they end
/// as one does (`_eighth_slab` ends in `_slab`): their stem is not a block.
const OTHER_SHAPES: [&str; 6] = ["_eighth_slab", "_quarter_slab", "_vertical_corner_slab", "_vertical_quarter", "_vertical_corner", "_corner"];

/// The shape a block is by its name, and its stem: `conquest:limestone_slab`
/// → (Slab, `limestone`). The suffixes are the mod profiles' and the game's
/// ([`crate::shapes::suffixes`]), the longest first.
pub fn shape_of(id: &str) -> Option<(Shape, &str)> {
    let path = id.rsplit(':').next().unwrap_or(id);
    if OTHER_SHAPES.iter().any(|s| path.ends_with(s)) && !path.ends_with("_vertical_slab") && !path.ends_with("_corner_slab") {
        return None;
    }
    let mut best: Option<(Shape, &str)> = None;
    for &(shape, ref suffix) in crate::shapes::suffixes() {
        if let Some(stem) = path.strip_suffix(suffix.as_str())
            && !stem.is_empty()
            && best.is_none_or(|(_, b)| stem.len() < b.len())
        {
            best = Some((shape, stem));
        }
    }
    // `_vertical_corner_slab` ends in `_corner_slab`: not a corner piece.
    if let Some((Shape::Corner, stem)) = best
        && stem.ends_with("_vertical")
    {
        return None;
    }
    best
}

impl BlockFamilies {
    /// The table of `recipes` (a block in, a block out, by id), knowing the
    /// names the blocks are given (`name_of`, by id: `Limestone`).
    pub fn from_recipes<'a>(recipes: impl IntoIterator<Item = (&'a str, &'a str)>, name_of: &dyn Fn(&str) -> Option<String>) -> Self {
        // Of each shape, the blocks cut into it.
        let mut cut: HashMap<&str, Vec<&str>> = HashMap::new();
        for (input, output) in recipes {
            if input != output && shape_of(output).is_some() {
                let list = cut.entry(output).or_default();
                if !list.contains(&input) {
                    list.push(input);
                }
            }
        }
        let mut out = BlockFamilies::default();
        let mut by_rename: Vec<(String, String)> = Vec::new();
        let snake = |name: &str| name.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("_");
        let mut cut: Vec<(&str, Vec<&str>)> = cut.into_iter().collect();
        cut.sort();
        for (shape, inputs) in cut {
            let Some((_, stem)) = shape_of(shape) else { continue };
            // By name: the stem is one of the inputs (in any namespace).
            if let Some(full) = inputs.iter().find(|i| crate::shapes::bases(i.rsplit(':').next().unwrap_or(i)).iter().any(|b| b == stem)) {
                out.link(full, shape);
                continue;
            }
            // The mod's own name for the block cut into it.
            if let Some(full) = inputs.iter().find(|i| shape_of(i).is_none() && name_of(i).is_some_and(|n| crate::shapes::bases(&snake(&n)).iter().any(|b| b == stem))) {
                by_rename.push(((*full).to_owned(), shape.to_owned()));
            }
        }
        for (full, shape) in by_rename {
            out.link(&full, &shape);
        }
        out
    }

    fn link(&mut self, full: &str, shape: &str) {
        if self.full.contains_key(shape) {
            return;
        }
        self.full.insert(shape.to_owned(), full.to_owned());
        self.shapes.entry(full.to_owned()).or_default().push(shape.to_owned());
    }

    pub fn is_empty(&self) -> bool {
        self.full.is_empty()
    }

    /// The shapes of full block `id`, of `shape`, in the table's order (by
    /// name first).
    pub fn shapes_of(&self, id: &str, shape: Shape) -> impl Iterator<Item = &str> {
        self.shapes.get(id).into_iter().flatten().map(String::as_str).filter(move |s| shape_of(s).is_some_and(|(k, _)| k == shape))
    }

    /// The full block shape `id` is of.
    pub fn full_block_of(&self, id: &str) -> Option<&str> {
        self.full.get(id).map(String::as_str)
    }

    /// How many shapes it links.
    pub fn len(&self) -> usize {
        self.full.len()
    }
}

/// An empty table, for what reads no mod.
pub fn none() -> &'static BlockFamilies {
    static NONE: std::sync::OnceLock<BlockFamilies> = std::sync::OnceLock::new();
    NONE.get_or_init(BlockFamilies::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Conquest Reforged's recipes as its jar has them (1.7.0).
    #[test]
    fn recipes_link_a_blocks_own_shapes() {
        let names = [("minecraft:stone", "Limestone"), ("minecraft:black_terracotta", "Black Hardened Clay"), ("minecraft:sand", "Sand"), ("minecraft:podzol", "Vibrant Autumnal Forest Floor"), ("minecraft:coarse_dirt", "Dirt Path")];
        let name_of = |id: &str| names.iter().find(|n| n.0 == id).map(|n| n.1.to_owned());
        let recipes = [
            ("conquest:fir_forest_floor", "conquest:fir_forest_floor_layer"),
            ("conquest:fir_forest_floor", "conquest:fir_forest_floor_stairs"),
            ("conquest:fir_forest_floor", "conquest:fir_forest_floor_eighth_slab"),
            ("minecraft:grass_block", "conquest:grass_block_layer"),
            ("minecraft:grass_block", "conquest:grass_block_vertical_corner_slab"),
            ("minecraft:grass_block", "conquest:taiga_grass_layer"),
            ("minecraft:grass_block", "conquest:fir_forest_floor"),
            ("minecraft:stone", "conquest:limestone_slab"),
            ("minecraft:stone", "conquest:limestone_eighth_slab"),
            ("minecraft:black_terracotta", "conquest:black_hardened_clay_slab"),
            ("minecraft:sand", "conquest:grass_and_sand_layer"),
            ("conquest:brown_moss", "conquest:brown_moss_slab"),
            ("minecraft:coarse_dirt", "conquest:grassy_dirt_layer"),
            ("minecraft:grass_block", "conquest:grassy_dirt_layer"),
            ("minecraft:podzol", "conquest:vibrant_autumnal_forest_floor_layer"),
        ];
        let t = BlockFamilies::from_recipes(recipes, &name_of);
        assert_eq!(t.full_block_of("conquest:fir_forest_floor_layer"), Some("conquest:fir_forest_floor"));
        assert_eq!(t.full_block_of("conquest:grass_block_layer"), Some("minecraft:grass_block"));
        assert_eq!(t.full_block_of("conquest:limestone_slab"), Some("minecraft:stone"), "the mod's name for stone");
        assert_eq!(t.full_block_of("conquest:black_hardened_clay_slab"), Some("minecraft:black_terracotta"));
        assert_eq!(t.full_block_of("conquest:vibrant_autumnal_forest_floor_layer"), Some("minecraft:podzol"));
        assert_eq!(t.full_block_of("conquest:grass_and_sand_layer"), None, "sand is Sand: grass and sand is another block");
        assert_eq!(t.full_block_of("conquest:brown_moss_slab"), Some("conquest:brown_moss"));
        assert_eq!(t.full_block_of("conquest:taiga_grass_layer"), None, "taiga grass exists: its layers are its own");
        assert_eq!(t.full_block_of("conquest:grassy_dirt_layer"), None, "neither block's own");
        assert_eq!(t.full_block_of("conquest:limestone_eighth_slab"), None, "not one of Smooth terrain's shapes");
        assert_eq!(t.full_block_of("conquest:fir_forest_floor"), None, "a block, not a shape");
        assert_eq!(t.shapes_of("minecraft:stone", Shape::Slab).collect::<Vec<_>>(), ["conquest:limestone_slab"]);
        assert_eq!(t.shapes_of("conquest:fir_forest_floor", Shape::Stairs).collect::<Vec<_>>(), ["conquest:fir_forest_floor_stairs"]);
        assert_eq!(shape_of("conquest:grass_block_vertical_corner_slab"), None);
        assert_eq!(shape_of("conquest:limestone_vertical_slab"), Some((Shape::VerticalSlab, "limestone")));
        assert_eq!(shape_of("conquest:limestone_corner_slab"), Some((Shape::Corner, "limestone")));
    }
}
