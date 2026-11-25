//! What a block of a world is to the import: the ground, a fluid, a plant
//! that grows on it, part of a tree, or something built.

/// A block's part in a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Air,
    Water,
    Lava,
    /// The land: its top becomes a terrain.
    Ground,
    /// What grows on the land and a ground cover can grow back: plants,
    /// snow, carpets, a mod's layer blocks.
    Plant,
    Log,
    Leaves,
    /// Anything else: buildings, roads, decorations.
    Built,
}

fn short(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

const BUILT_WORDS: &[&str] = &[
    "stairs", "slab", "wall", "fence", "gate", "door", "trapdoor", "pane", "glass", "planks", "bricks", "brick", "wool", "bed", "torch", "lantern", "chest", "barrel", "furnace", "table", "sign", "banner", "rail", "ladder", "button", "plate", "lever", "chain", "bars", "anvil", "bell",
    "campfire", "cauldron", "composter", "loom", "stonecutter", "grindstone", "lectern", "bookshelf", "head", "skull", "pot", "candle", "cake", "beacon", "hopper", "dispenser", "dropper", "observer", "piston", "repeater", "comparator", "tnt", "jukebox", "spawner", "portal",
    "polished", "smooth", "cut", "chiseled", "concrete", "glazed", "quartz", "purpur", "prismarine", "lamp", "hay", "scaffolding", "honey", "slime", "target", "lodestone", "anchor", "shulker", "conduit", "frame", "rod", "pillar", "tiles", "tile", "plaster", "beam", "arch", "window",
    "shutter", "railing", "balustrade", "column", "roof", "crate", "shelf", "workbench", "vase", "statue", "cobweb", "web", "vine", "vines", "ladder", "carpet", "terracotta_bricks", "post", "lattice", "pipe", "light", "path_slab", "block_of",
];

const PLANT_WORDS: &[&str] = &[
    "grass", "fern", "flower", "flowers", "bush", "sapling", "tulip", "orchid", "allium", "bluet", "daisy", "poppy", "dandelion", "cornflower", "lily", "rose", "petals", "mushroom", "roots", "sprouts", "seagrass", "kelp", "pickle", "coral", "reeds", "reed", "weeds", "weed", "clover",
    "heather", "shrub", "shrubs", "plant", "plants", "herb", "herbs", "cattail", "cattails", "lavender", "wildflowers", "litter", "dripleaf", "azalea", "berry", "berries", "nettle", "nettles", "thistle", "ivy", "moss_carpet", "lichen", "cane", "cactus", "bamboo", "sugar",
];

const GROUND_WORDS: &[&str] = &[
    "stone", "dirt", "grass_block", "sand", "gravel", "clay", "mud", "soil", "earth", "silt", "loam", "peat", "podzol", "mycelium", "rock", "deepslate", "tuff", "calcite", "basalt", "netherrack", "nylium", "ore", "ice", "snow_block", "moss_block", "sandstone", "granite", "diorite",
    "andesite", "terracotta", "bedrock", "obsidian", "magma", "blackstone", "dripstone", "end_stone", "farmland", "soul",
];

/// What a block state is to the import.
pub fn classify(state: &str) -> Class {
    let name = state.split('[').next().unwrap_or(state);
    let s = short(name);
    let vanilla = name.starts_with("minecraft:") || !name.contains(':');
    match s {
        "air" | "cave_air" | "void_air" | "structure_void" => return Class::Air,
        "water" | "bubble_column" => return Class::Water,
        "lava" => return Class::Lava,
        "snow" | "moss_carpet" | "pale_moss_carpet" | "leaf_litter" | "pink_petals" | "wildflowers" | "lily_pad" | "sea_pickle" | "glow_lichen" | "frogspawn" => return Class::Plant,
        "mangrove_roots" => return Class::Log,
        "mushroom_stem" | "red_mushroom_block" | "brown_mushroom_block" | "bamboo_block" => return Class::Log,
        "bee_nest" | "cocoa" => return Class::Leaves,
        "grass_block" | "dirt_path" | "farmland" | "snow_block" | "packed_ice" | "blue_ice" | "ice" | "clay" | "cobblestone" | "mossy_cobblestone" | "suspicious_sand" | "suspicious_gravel" => return Class::Ground,
        _ => {}
    }
    if massif_core::registry::plant_color(state).is_some() || massif_core::registry::is_tall_plant(state) && !s.contains("door") {
        return Class::Plant;
    }
    let words: Vec<&str> = s.split('_').collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w)) || list.iter().any(|l| l.contains('_') && s.contains(l));
    if s.ends_with("_leaves") || s == "leaves" || words.contains(&"leaves") || words.contains(&"foliage") {
        return Class::Leaves;
    }
    if s.ends_with("_log") || s.ends_with("_wood") || s.ends_with("_hyphae") || (s.ends_with("_stem") && !s.contains("mushroom") && !s.contains("melon") && !s.contains("pumpkin") && !s.contains("dripleaf")) || words.contains(&"log") || words.contains(&"trunk") {
        return if s.starts_with("stripped_") { Class::Built } else { Class::Log };
    }
    // A mod's layer blocks and carpets of the ground: covers.
    if state.contains("layers=") || s.ends_with("_layer") || s.ends_with("_carpet") && !vanilla {
        return Class::Plant;
    }
    if s.ends_with("_carpet") {
        return Class::Built;
    }
    if has(BUILT_WORDS) {
        return Class::Built;
    }
    if has(PLANT_WORDS) && !s.ends_with("_block") && !has(GROUND_WORDS) {
        return Class::Plant;
    }
    if s.ends_with("_coral_block") || s.contains("coral_fan") || s.ends_with("_coral") {
        return if s.ends_with("_block") { Class::Ground } else { Class::Plant };
    }
    if vanilla {
        // The game's natural full blocks; the rest is built.
        if has(GROUND_WORDS) || matches!(s, "glowstone" | "amethyst_block" | "budding_amethyst" | "sculk" | "crying_obsidian" | "rooted_dirt" | "coarse_dirt" | "red_sand" | "soul_sand" | "soul_soil" | "warped_wart_block" | "nether_wart_block" | "shroomlight" | "pointed_dripstone" | "bone_block" | "raw_iron_block" | "raw_copper_block" | "raw_gold_block" | "resin_clump") {
            return match s {
                "pointed_dripstone" | "resin_clump" => Class::Built,
                _ => Class::Ground,
            };
        }
        return Class::Built;
    }
    // A mod's block that names nothing built: the ground.
    Class::Ground
}

/// Whether a state is waterlogged: water around it.
pub fn waterlogged(state: &str) -> bool {
    state.contains("waterlogged=true")
}

/// Plants that grow under water.
pub fn aquatic(state: &str) -> bool {
    let s = short(state.split('[').next().unwrap_or(state));
    matches!(s, "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant" | "sea_pickle") || s.contains("coral")
}

/// The block a cover writes for a plant seen in the world: the lower half
/// of a tall plant without its `half`, a stack's top as kelp grows.
pub fn cover_state(state: &str) -> String {
    let name = state.split('[').next().unwrap_or(state);
    if short(name) == "kelp_plant" {
        return "minecraft:kelp".into();
    }
    if massif_core::registry::is_tall_plant(state) {
        let kept: Vec<&str> = state.split_once('[').map(|(_, p)| p.trim_end_matches(']').split(',').filter(|kv| !kv.starts_with("half=")).collect()).unwrap_or_default();
        return if kept.is_empty() { name.to_owned() } else { format!("{name}[{}]", kept.join(",")) };
    }
    state.to_owned()
}

/// A readable name of a block: `minecraft:oak_stairs[facing=east]` →
/// `Oak stairs (facing=east)`, a mod's block with its mod.
pub fn pretty(state: &str) -> String {
    let (name, props) = state.split_once('[').map_or((state, ""), |(n, p)| (n, p.trim_end_matches(']')));
    let (ns, s) = name.split_once(':').unwrap_or(("minecraft", name));
    let mut words = s.replace('_', " ");
    if let Some(f) = words.get(..1) {
        words = f.to_uppercase() + &words[1..];
    }
    let mut out = if ns == "minecraft" { words } else { format!("{}: {words}", ns) };
    if !props.is_empty() {
        out += &format!(" ({props})");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        for (s, c) in [
            ("minecraft:grass_block[snowy=false]", Class::Ground),
            ("minecraft:stone", Class::Ground),
            ("minecraft:deepslate[axis=y]", Class::Ground),
            ("minecraft:iron_ore", Class::Ground),
            ("minecraft:sand", Class::Ground),
            ("minecraft:oak_log[axis=y]", Class::Log),
            ("minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]", Class::Leaves),
            ("minecraft:short_grass", Class::Plant),
            ("minecraft:tall_grass[half=lower]", Class::Plant),
            ("minecraft:poppy", Class::Plant),
            ("minecraft:snow[layers=1]", Class::Plant),
            ("minecraft:oak_planks", Class::Built),
            ("minecraft:stone_bricks", Class::Built),
            ("minecraft:cobblestone_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]", Class::Built),
            ("minecraft:glass", Class::Built),
            ("minecraft:torch", Class::Built),
            ("minecraft:water[level=0]", Class::Water),
            ("minecraft:cave_air", Class::Air),
            ("conquest:mossy_loamy_dirt", Class::Ground),
            ("conquest:muddy_autumnal_forest_floor_layer[layers=8]", Class::Plant),
            ("conquest:oak_beam", Class::Built),
            ("conquest:white_plaster_wall", Class::Built),
            ("minecraft:polished_andesite", Class::Built),
            ("minecraft:spruce_wood[axis=y]", Class::Log),
            ("minecraft:stripped_oak_log[axis=y]", Class::Built),
        ] {
            assert_eq!(classify(s), c, "{s}");
        }
    }

    #[test]
    fn cover_states() {
        assert_eq!(cover_state("minecraft:tall_grass[half=lower]"), "minecraft:tall_grass");
        assert_eq!(cover_state("minecraft:kelp_plant"), "minecraft:kelp");
        assert_eq!(pretty("minecraft:oak_stairs[facing=east]"), "Oak stairs (facing=east)");
    }
}
