//! The blocks of the versions before 1.13 — a number and four bits of
//! data — as the block states they became. What objects are made of:
//! natural blocks, wood, stone, the usual building blocks; blocks whose
//! number means nothing here are left out.

const WOOD: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];
const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// Stairs: data 0 to 3 face east, west, south, north; 4 turns them upside
/// down.
fn stairs(kind: &str, data: u8) -> String {
    let facing = ["east", "west", "south", "north"][(data & 3) as usize];
    let half = if data & 4 != 0 { "top" } else { "bottom" };
    format!("minecraft:{kind}_stairs[facing={facing},half={half},shape=straight,waterlogged=false]")
}

/// Facing as chests, furnaces and ladders give it: 2 north, 3 south, 4
/// west, 5 east.
fn facing(data: u8) -> &'static str {
    match data & 7 {
        3 => "south",
        4 => "west",
        5 => "east",
        _ => "north",
    }
}

/// Logs: the kind in the two low bits, the axis in the next two (3: bark
/// all round).
fn log(kinds: &[&str], data: u8) -> String {
    let kind = kinds.get((data & 3) as usize).copied().unwrap_or(kinds[0]);
    match data >> 2 & 3 {
        1 => format!("minecraft:{kind}_log[axis=x]"),
        2 => format!("minecraft:{kind}_log[axis=z]"),
        3 => format!("minecraft:{kind}_wood[axis=y]"),
        _ => format!("minecraft:{kind}_log[axis=y]"),
    }
}

/// Leaves never decay once placed: what holds them up may not have come
/// with them.
fn leaves(kinds: &[&str], data: u8) -> String {
    let kind = kinds.get((data & 3) as usize).copied().unwrap_or(kinds[0]);
    format!("minecraft:{kind}_leaves[distance=1,persistent=true,waterlogged=false]")
}

fn slab(kind: &str, data: u8, double: bool) -> String {
    let half = if double {
        "double"
    } else if data & 8 != 0 {
        "top"
    } else {
        "bottom"
    };
    format!("minecraft:{kind}_slab[type={half},waterlogged=false]")
}

const STONE_SLABS: [&str; 8] = ["smooth_stone", "sandstone", "petrified_oak", "cobblestone", "brick", "stone_brick", "nether_brick", "quartz"];

fn sides(north: bool, east: bool, south: bool, west: bool) -> String {
    format!("east={east},north={north},south={south},west={west}")
}

/// The block state of block `id` with data `data`; `None` for a number
/// that means nothing here.
pub fn state(id: u16, data: u8) -> Option<String> {
    let colour = COLOURS[(data & 15) as usize];
    let wood = WOOD.get((data & 7) as usize).copied();
    let s = |name: &str| Some(format!("minecraft:{name}"));
    match id {
        0 => s("air"),
        1 => s(["stone", "granite", "polished_granite", "diorite", "polished_diorite", "andesite", "polished_andesite"].get(data as usize).copied().unwrap_or("stone")),
        2 => s("grass_block[snowy=false]"),
        3 => s(["dirt", "coarse_dirt", "podzol[snowy=false]"].get(data as usize).copied().unwrap_or("dirt")),
        4 => s("cobblestone"),
        5 => Some(format!("minecraft:{}_planks", wood.unwrap_or("oak"))),
        6 => Some(format!("minecraft:{}_sapling[stage=0]", wood.unwrap_or("oak"))),
        7 => s("bedrock"),
        8 | 9 => s("water[level=0]"),
        10 | 11 => s("lava[level=0]"),
        12 => s(if data == 1 { "red_sand" } else { "sand" }),
        13 => s("gravel"),
        14 => s("gold_ore"),
        15 => s("iron_ore"),
        16 => s("coal_ore"),
        17 => Some(log(&WOOD[..4], data)),
        18 => Some(leaves(&WOOD[..4], data)),
        19 => s("sponge"),
        20 => s("glass"),
        21 => s("lapis_ore"),
        22 => s("lapis_block"),
        24 => s(["sandstone", "chiseled_sandstone", "cut_sandstone"].get(data as usize).copied().unwrap_or("sandstone")),
        30 => s("cobweb"),
        31 => s(match data {
            0 => "dead_bush",
            2 => "fern",
            _ => "short_grass",
        }),
        32 => s("dead_bush"),
        35 => Some(format!("minecraft:{colour}_wool")),
        37 => s("dandelion"),
        38 => s(["poppy", "blue_orchid", "allium", "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip", "oxeye_daisy"].get(data as usize).copied().unwrap_or("poppy")),
        39 => s("brown_mushroom"),
        40 => s("red_mushroom"),
        41 => s("gold_block"),
        42 => s("iron_block"),
        43 => Some(slab(STONE_SLABS[(data & 7) as usize], data, true)),
        44 => Some(slab(STONE_SLABS[(data & 7) as usize], data, false)),
        45 => s("bricks"),
        46 => s("tnt[unstable=false]"),
        47 => s("bookshelf"),
        48 => s("mossy_cobblestone"),
        49 => s("obsidian"),
        50 => Some(match data {
            1 => "minecraft:wall_torch[facing=east]".to_owned(),
            2 => "minecraft:wall_torch[facing=west]".to_owned(),
            3 => "minecraft:wall_torch[facing=south]".to_owned(),
            4 => "minecraft:wall_torch[facing=north]".to_owned(),
            _ => "minecraft:torch".to_owned(),
        }),
        53 => Some(stairs("oak", data)),
        54 => Some(format!("minecraft:chest[facing={},type=single,waterlogged=false]", facing(data))),
        56 => s("diamond_ore"),
        57 => s("diamond_block"),
        58 => s("crafting_table"),
        60 => s("farmland[moisture=7]"),
        61 | 62 => Some(format!("minecraft:furnace[facing={},lit=false]", facing(data))),
        65 => Some(format!("minecraft:ladder[facing={},waterlogged=false]", facing(data))),
        67 => Some(stairs("cobblestone", data)),
        73 | 74 => s("redstone_ore[lit=false]"),
        78 => Some(format!("minecraft:snow[layers={}]", (data & 7) + 1)),
        79 => s("ice"),
        80 => s("snow_block"),
        81 => s("cactus[age=0]"),
        82 => s("clay"),
        83 => s("sugar_cane[age=0]"),
        85 => Some(format!("minecraft:oak_fence[{},waterlogged=false]", sides(false, false, false, false))),
        86 => s("carved_pumpkin[facing=south]"),
        87 => s("netherrack"),
        88 => s("soul_sand"),
        89 => s("glowstone"),
        91 => s("jack_o_lantern[facing=south]"),
        95 => Some(format!("minecraft:{colour}_stained_glass")),
        98 => s(["stone_bricks", "mossy_stone_bricks", "cracked_stone_bricks", "chiseled_stone_bricks"].get(data as usize).copied().unwrap_or("stone_bricks")),
        99 | 100 => {
            let kind = if id == 99 { "brown" } else { "red" };
            Some(match data {
                10 | 15 => format!("minecraft:mushroom_stem[down={},{},up={}]", data == 15, sides(true, true, true, true), data == 15),
                0 => format!("minecraft:{kind}_mushroom_block[down=false,{},up=false]", sides(false, false, false, false)),
                _ => format!("minecraft:{kind}_mushroom_block[down=false,{},up=true]", sides(matches!(data, 1..=3), matches!(data, 3 | 6 | 9), matches!(data, 7..=9), matches!(data, 1 | 4 | 7))),
            })
        }
        101 => Some(format!("minecraft:iron_bars[{},waterlogged=false]", sides(false, false, false, false))),
        102 => Some(format!("minecraft:glass_pane[{},waterlogged=false]", sides(false, false, false, false))),
        103 => s("melon"),
        106 => Some(format!("minecraft:vine[{},up={}]", sides(data & 4 != 0, data & 8 != 0, data & 1 != 0, data & 2 != 0), data == 0)),
        107 => Some(format!("minecraft:oak_fence_gate[facing={},in_wall=false,open=false,powered=false]", ["south", "west", "north", "east"][(data & 3) as usize])),
        108 => Some(stairs("brick", data)),
        109 => Some(stairs("stone_brick", data)),
        110 => s("mycelium[snowy=false]"),
        111 => s("lily_pad"),
        112 => s("nether_bricks"),
        113 => Some(format!("minecraft:nether_brick_fence[{},waterlogged=false]", sides(false, false, false, false))),
        114 => Some(stairs("nether_brick", data)),
        121 => s("end_stone"),
        125 => Some(slab(wood.unwrap_or("oak"), data, true)),
        126 => Some(slab(WOOD.get((data & 7) as usize).copied().unwrap_or("oak"), data, false)),
        127 => Some(format!("minecraft:cocoa[age={},facing={}]", (data >> 2).min(2), ["south", "west", "north", "east"][(data & 3) as usize])),
        128 => Some(stairs("sandstone", data)),
        129 => s("emerald_ore"),
        133 => s("emerald_block"),
        134 => Some(stairs("spruce", data)),
        135 => Some(stairs("birch", data)),
        136 => Some(stairs("jungle", data)),
        139 => Some(format!(
            "minecraft:{}cobblestone_wall[east=none,north=none,south=none,up=true,waterlogged=false,west=none]",
            if data == 1 { "mossy_" } else { "" }
        )),
        155 => s(match data {
            1 => "chiseled_quartz_block",
            2 => "quartz_pillar[axis=y]",
            3 => "quartz_pillar[axis=x]",
            4 => "quartz_pillar[axis=z]",
            _ => "quartz_block",
        }),
        156 => Some(stairs("quartz", data)),
        159 => Some(format!("minecraft:{colour}_terracotta")),
        160 => Some(format!("minecraft:{colour}_stained_glass_pane[{},waterlogged=false]", sides(false, false, false, false))),
        161 => Some(leaves(&WOOD[4..], data)),
        162 => Some(log(&WOOD[4..], data)),
        163 => Some(stairs("acacia", data)),
        164 => Some(stairs("dark_oak", data)),
        168 => s(["prismarine", "prismarine_bricks", "dark_prismarine"].get(data as usize).copied().unwrap_or("prismarine")),
        169 => s("sea_lantern"),
        170 => s(match data >> 2 & 3 {
            1 => "hay_block[axis=x]",
            2 => "hay_block[axis=z]",
            _ => "hay_block[axis=y]",
        }),
        171 => Some(format!("minecraft:{colour}_carpet")),
        172 => s("terracotta"),
        173 => s("coal_block"),
        174 => s("packed_ice"),
        175 => {
            let kind = ["sunflower", "lilac", "tall_grass", "large_fern", "rose_bush", "peony"].get((data & 7) as usize).copied().unwrap_or("tall_grass");
            let half = if data & 8 != 0 { "upper" } else { "lower" };
            Some(format!("minecraft:{kind}[half={half}]"))
        }
        179 => s(["red_sandstone", "chiseled_red_sandstone", "cut_red_sandstone"].get(data as usize).copied().unwrap_or("red_sandstone")),
        180 => Some(stairs("red_sandstone", data)),
        183..=187 => {
            let kind = ["spruce", "birch", "jungle", "dark_oak", "acacia"][(id - 183) as usize];
            Some(format!("minecraft:{kind}_fence_gate[facing={},in_wall=false,open=false,powered=false]", ["south", "west", "north", "east"][(data & 3) as usize]))
        }
        188..=192 => {
            let kind = ["spruce", "birch", "jungle", "dark_oak", "acacia"][(id - 188) as usize];
            Some(format!("minecraft:{kind}_fence[{},waterlogged=false]", sides(false, false, false, false)))
        }
        201 => s("purpur_block"),
        202 => s("purpur_pillar[axis=y]"),
        206 => s("end_stone_bricks"),
        208 => s("dirt_path"),
        213 => s("magma_block"),
        214 => s("nether_wart_block"),
        215 => s("red_nether_bricks"),
        216 => s("bone_block[axis=y]"),
        235..=250 => Some(format!("minecraft:{}_glazed_terracotta[facing=south]", COLOURS[(id - 235) as usize])),
        251 => Some(format!("minecraft:{colour}_concrete")),
        252 => Some(format!("minecraft:{colour}_concrete_powder")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::state;

    #[test]
    fn old_numbers_become_today_s_blocks() {
        assert_eq!(state(17, 0).unwrap(), "minecraft:oak_log[axis=y]");
        assert_eq!(state(17, 6).unwrap(), "minecraft:birch_log[axis=x]");
        assert_eq!(state(162, 13).unwrap(), "minecraft:dark_oak_wood[axis=y]");
        assert_eq!(state(161, 1).unwrap(), "minecraft:dark_oak_leaves[distance=1,persistent=true,waterlogged=false]");
        assert_eq!(state(31, 1).unwrap(), "minecraft:short_grass");
        assert_eq!(state(175, 10).unwrap(), "minecraft:tall_grass[half=upper]");
        assert_eq!(state(53, 6).unwrap(), "minecraft:oak_stairs[facing=south,half=top,shape=straight,waterlogged=false]");
        assert_eq!(state(35, 14).unwrap(), "minecraft:red_wool");
        assert_eq!(state(106, 5).unwrap(), "minecraft:vine[east=false,north=true,south=true,west=false,up=false]");
        assert_eq!(state(78, 2).unwrap(), "minecraft:snow[layers=3]");
        assert_eq!(state(99, 10).unwrap(), "minecraft:mushroom_stem[down=false,east=true,north=true,south=true,west=true,up=false]");
        assert!(state(255, 0).is_none());
        assert!(state(23, 0).is_none());
    }
}
