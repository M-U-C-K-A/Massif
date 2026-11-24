//! A Plains pack of plain boxes at the sizes of the user's list (small
//! houses 7–11, medium 11–16, manors 16–24, a church up to 24 × 40…), each
//! with a door on its south side over three layers of foundation: what the
//! village is tested with, and checked in the game with, before the real
//! schematics are ready. `massif-cli village-test-pack <dir>` writes it.

use crate::nbt::Compound;
use crate::testing::template_bytes;
use std::path::Path;

type Blocks = Vec<([i32; 3], String, Option<Compound>)>;

/// The look of a box: its walls, its roof, its trim.
struct Look {
    wall: &'static str,
    roof: &'static str,
    trim: &'static str,
}

const LOOKS: [Look; 4] = [
    Look { wall: "minecraft:oak_planks", roof: "minecraft:bricks", trim: "minecraft:stripped_oak_log[axis=y]" },
    Look { wall: "minecraft:spruce_planks", roof: "minecraft:deepslate_tiles", trim: "minecraft:spruce_log[axis=y]" },
    Look { wall: "minecraft:white_terracotta", roof: "minecraft:dark_oak_planks", trim: "minecraft:dark_oak_log[axis=y]" },
    Look { wall: "minecraft:birch_planks", roof: "minecraft:stone_bricks", trim: "minecraft:stripped_spruce_log[axis=y]" },
];

/// A house-like box `w` wide, `d` deep, its walls `wall_h` high over
/// `found` layers of foundation, a stepped roof, windows, a door in the
/// middle of the south wall at floor level.
fn building(w: i32, d: i32, wall_h: i32, found: i32, look: &Look) -> ([i32; 3], Blocks) {
    let roof_layers = (w.min(d) + 1) / 2;
    let h = found + wall_h + roof_layers;
    let mut b: Blocks = Vec::new();
    let mut set = |p: [i32; 3], s: &str| b.push((p, s.to_owned(), None));
    let door_x = w / 2;
    for y in 0..h {
        for z in 0..d {
            for x in 0..w {
                let edge = x == 0 || z == 0 || x == w - 1 || z == d - 1;
                let corner = (x == 0 || x == w - 1) && (z == 0 || z == d - 1);
                if y < found - 1 {
                    set([x, y, z], "minecraft:cobblestone");
                } else if y == found - 1 {
                    set([x, y, z], if edge { "minecraft:cobblestone" } else { "minecraft:spruce_planks" });
                } else if y < found + wall_h {
                    let wy = y - found;
                    if x == door_x && z == d - 1 && wy < 2 {
                        let half = if wy == 0 { "lower" } else { "upper" };
                        set([x, y, z], &format!("minecraft:oak_door[facing=south,half={half},hinge=left,open=false,powered=false]"));
                    } else if corner {
                        set([x, y, z], look.trim);
                    } else if edge {
                        let window = wy == 1 && wall_h >= 3 && ((x % 3 == 1 && (z == 0 || z == d - 1)) || (z % 3 == 1 && (x == 0 || x == w - 1)));
                        set([x, y, z], if window { "minecraft:glass_pane" } else { look.wall });
                    } else {
                        set([x, y, z], "minecraft:air");
                    }
                } else {
                    // The roof: each layer one block in from the one below.
                    let k = y - found - wall_h;
                    let inset = x.min(w - 1 - x).min(z.min(d - 1 - z));
                    if inset == k {
                        set([x, y, z], look.roof);
                    } else if inset > k {
                        set([x, y, z], "minecraft:air");
                    }
                }
            }
        }
    }
    ([w, h, d], b)
}

/// A tower of `side` blocks, `wall_h` high: a watchtower, a windmill (with
/// sails on its south face), a wall's tower.
fn tower(side: i32, wall_h: i32, found: i32, sails: bool, door: bool) -> ([i32; 3], Blocks) {
    let look = Look { wall: "minecraft:stone_bricks", roof: "minecraft:spruce_planks", trim: "minecraft:cobblestone" };
    let (size, mut b) = building(side, side, wall_h, found, &look);
    if !door {
        b.retain(|(_, s, _)| !s.contains("_door"));
        for y in found..found + 2 {
            b.push(([side / 2, y, side - 1], look.wall.to_owned(), None));
        }
    }
    if sails {
        let (cx, cy) = (side / 2, found + wall_h - 2);
        let mut extra: Blocks = Vec::new();
        for k in -4..=4i32 {
            if k != 0 {
                extra.push(([cx + k, cy, side], "minecraft:white_wool".into(), None));
                extra.push(([cx, cy + k, side], "minecraft:white_wool".into(), None));
            }
        }
        // The sails stand out of the box: it grows a block south, and the
        // door's side stays the south one, behind them.
        let size = [size[0], size[1].max(cy + 5), size[2] + 1];
        b.extend(extra.into_iter().filter(|(p, _, _)| p[0] >= 0 && p[0] < size[0] && p[1] >= 0 && p[1] < size[1]));
        return (size, b);
    }
    (size, b)
}

/// A well: a ring of cobblestone round water, posts, a roof of slabs.
fn well(side: i32, found: i32) -> ([i32; 3], Blocks) {
    let h = found + 4;
    let mut b: Blocks = Vec::new();
    for z in 0..side {
        for x in 0..side {
            for y in 0..found {
                b.push(([x, y, z], "minecraft:cobblestone".into(), None));
            }
            let rim = x == 0 || z == 0 || x == side - 1 || z == side - 1;
            b.push(([x, found - 1, z], if rim { "minecraft:cobblestone".into() } else { "minecraft:water[level=0]".into() }, None));
            if rim {
                b.push(([x, found, z], "minecraft:cobblestone_wall[east=none,north=none,south=none,up=true,waterlogged=false,west=none]".into(), None));
            }
            let post = (x == 0 || x == side - 1) && (z == 0 || z == side - 1);
            if post {
                for y in found + 1..found + 3 {
                    b.push(([x, y, z], "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]".into(), None));
                }
            }
            b.push(([x, found + 3, z], "minecraft:spruce_slab[type=bottom,waterlogged=false]".into(), None));
        }
    }
    ([side, h, side], b)
}

/// A market stall: posts, a counter of barrels, a canopy of wool.
fn stall(w: i32, d: i32, wool: &str) -> ([i32; 3], Blocks) {
    let found = 1;
    let mut b: Blocks = Vec::new();
    for z in 0..d {
        for x in 0..w {
            b.push(([x, 0, z], "minecraft:spruce_planks".into(), None));
            let post = (x == 0 || x == w - 1) && (z == 0 || z == d - 1);
            if post {
                for y in found..found + 3 {
                    b.push(([x, y, z], "minecraft:spruce_fence[east=false,north=false,south=false,waterlogged=false,west=false]".into(), None));
                }
            } else if z == d - 1 && x > 0 && x < w - 1 {
                b.push(([x, found, z], "minecraft:barrel[facing=up,open=false]".into(), None));
            }
            b.push(([x, found + 3, z], wool.to_owned(), None));
        }
    }
    ([w, found + 4, d], b)
}

/// A small prop of the user's list (1 × 1 to 5 × 5), standing on the
/// ground (no foundation).
fn prop(kind: &str) -> ([i32; 3], Blocks) {
    let mut b: Blocks = Vec::new();
    let fence = "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]";
    let size = match kind {
        "lamppost_2" => {
            b.push(([0, 0, 0], "minecraft:stone_bricks".into(), None));
            for y in 1..4 {
                b.push(([0, y, 0], "minecraft:cobblestone_wall[east=none,north=none,south=none,up=true,waterlogged=false,west=none]".into(), None));
            }
            b.push(([0, 4, 0], "minecraft:lantern[hanging=false,waterlogged=false]".into(), None));
            [1, 5, 1]
        }
        "hay_bales_2" => {
            for (x, z) in [(0, 0), (1, 0), (2, 0)] {
                b.push(([x, 0, z], "minecraft:hay_block[axis=x]".into(), None));
            }
            b.push(([1, 1, 0], "minecraft:hay_block[axis=x]".into(), None));
            [3, 2, 1]
        }
        "lamppost" | "lamppost_1" => {
            for y in 0..3 {
                b.push(([0, y, 0], fence.into(), None));
            }
            b.push(([0, 3, 0], "minecraft:lantern[hanging=false,waterlogged=false]".into(), None));
            [1, 4, 1]
        }
        "bench" | "bench_1" => {
            for x in 0..3 {
                b.push(([x, 0, 0], "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]".into(), None));
            }
            [3, 1, 1]
        }
        "hay_bales" | "hay_bales_1" => {
            for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                b.push(([x, 0, z], "minecraft:hay_block[axis=y]".into(), None));
            }
            b.push(([0, 1, 0], "minecraft:hay_block[axis=x]".into(), None));
            [2, 2, 2]
        }
        "cart" | "cart_1" => {
            for x in 0..2 {
                for z in 0..3 {
                    b.push(([x, 0, z], "minecraft:spruce_slab[type=top,waterlogged=false]".into(), None));
                }
            }
            b.push(([0, 1, 1], "minecraft:hay_block[axis=y]".into(), None));
            b.push(([1, 1, 0], "minecraft:barrel[facing=up,open=false]".into(), None));
            [2, 2, 3]
        }
        _ => {
            // A pile of crates.
            for (x, z) in [(0, 0), (1, 0), (0, 1)] {
                b.push(([x, 0, z], "minecraft:barrel[facing=up,open=false]".into(), None));
            }
            b.push(([0, 1, 0], "minecraft:barrel[facing=up,open=false]".into(), None));
            [2, 2, 2]
        }
    };
    (size, b)
}

fn write(dir: &Path, rel: &str, (size, blocks): ([i32; 3], Blocks)) -> std::io::Result<()> {
    let refs: Vec<([i32; 3], &str, Option<Compound>)> = blocks.iter().map(|(p, s, c)| (*p, s.as_str(), c.clone())).collect();
    let path = dir.join(rel);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, template_bytes(size, &refs, &[]))
}

/// The manifest of the test pack: the plan's (§7.2), as a builder would
/// write it.
pub const MANIFEST: &str = r#"{
  "name": "Plains (test boxes)",
  "author": "Massif",
  "description": "Boxes at the sizes of the Plains list, to try villages before the real builds are ready.",
  "sizes": { "small": [7, 11], "medium": [11, 16], "large": [16, 24] },
  "types": {
    "house":      { "share": 0.55 },
    "manor":      { "share": 0.05, "max": 2, "prefer": "high_ground" },
    "church":     { "share": 0.04, "max": 1, "centre": true },
    "forge":      { "share": 0.05, "max": 2, "near": "market" },
    "tavern":     { "share": 0.04, "max": 1, "near": "market" },
    "watchtower": { "share": 0.03, "max": 2, "edge": true },
    "farm":       { "share": 0.12, "edge": true, "field": { "size": [12, 24], "crops": ["wheat", "carrots", "potatoes"] } },
    "mill":       { "max": 1, "edge": true, "files": { "watermill.nbt": { "water": true } } },
    "well":       { "max": 2, "centre": true },
    "market":     { "max": 4, "centre": true }
  },
  "props": { "along_roads": ["lamppost", "bench"], "near": { "farm": ["hay_bales", "cart"], "market": ["cart", "crates"] } },
  "roads": {
    "main": { "width": 3, "blocks": { "minecraft:dirt_path": 4, "minecraft:coarse_dirt": 1 } },
    "side": { "width": 2, "blocks": { "minecraft:dirt_path": 3, "minecraft:grass_block": 1 } }
  },
  "foundation": 3,
  "villagers": 1,
  "walls": { "height": 6, "block": "minecraft:stone_bricks", "towers": 40 }
}
"#;

/// Write the test pack into `dir` (`dir/Plains/…`): its folder.
pub fn write_pack(dir: &Path) -> std::io::Result<std::path::PathBuf> {
    let root = dir.join("Plains");
    let f = 3;
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("village.json"), MANIFEST)?;
    // Houses: eight small, six medium.
    for (k, (w, d)) in [(7, 7), (8, 7), (9, 8), (9, 9), (10, 8), (10, 10), (11, 9), (11, 11)].iter().enumerate() {
        write(&root, &format!("house/small_{}.nbt", k + 1), building(*w, *d, 4, f, &LOOKS[k % 4]))?;
    }
    for (k, (w, d)) in [(12, 12), (13, 11), (14, 12), (14, 14), (16, 13), (16, 16)].iter().enumerate() {
        write(&root, &format!("house/medium_{}.nbt", k + 1), building(*w, *d, 5, f, &LOOKS[(k + 1) % 4]))?;
    }
    write(&root, "manor/farmhouse_1.nbt", building(18, 16, 5, f, &LOOKS[2]))?;
    write(&root, "manor/manor_1.nbt", building(22, 20, 7, f, &LOOKS[3]))?;
    write(&root, "manor/manor_2.nbt", building(24, 24, 7, f, &LOOKS[1]))?;
    let church = Look { wall: "minecraft:stone_bricks", roof: "minecraft:deepslate_tiles", trim: "minecraft:chiseled_stone_bricks" };
    write(&root, "church/chapel.nbt", building(13, 19, 8, f, &church))?;
    write(&root, "church/church.nbt", building(22, 36, 10, f, &church))?;
    let forge = Look { wall: "minecraft:cobblestone", roof: "minecraft:blackstone", trim: "minecraft:polished_blackstone" };
    write(&root, "forge/blacksmith_1.nbt", building(9, 11, 4, f, &forge))?;
    write(&root, "forge/blacksmith_2.nbt", building(13, 13, 5, f, &forge))?;
    write(&root, "tavern/inn_1.nbt", building(12, 14, 6, f, &LOOKS[0]))?;
    write(&root, "tavern/inn_2.nbt", building(16, 18, 7, f, &LOOKS[2]))?;
    write(&root, "watchtower/tower_1.nbt", tower(5, 12, f, false, true))?;
    write(&root, "watchtower/tower_2.nbt", tower(8, 14, f, false, true))?;
    let barn = Look { wall: "minecraft:red_terracotta", roof: "minecraft:dark_oak_planks", trim: "minecraft:dark_oak_log[axis=y]" };
    for (k, (w, d)) in [(12, 12), (16, 14), (20, 16), (24, 20)].iter().enumerate() {
        write(&root, &format!("farm/barn_{}.nbt", k + 1), building(*w, *d, 5, f, &barn))?;
    }
    write(&root, "mill/windmill.nbt", tower(9, 14, f, true, true))?;
    write(&root, "mill/watermill.nbt", building(11, 9, 5, f, &LOOKS[1]))?;
    write(&root, "well/well_1.nbt", well(3, f))?;
    write(&root, "well/well_2.nbt", well(5, f))?;
    for (k, (w, d, wool)) in [(3, 3, "red"), (4, 3, "yellow"), (5, 4, "blue"), (5, 5, "white")].iter().enumerate() {
        write(&root, &format!("market/stall_{}.nbt", k + 1), stall(*w, *d, &format!("minecraft:{wool}_wool")))?;
    }
    for name in ["lamppost_1", "lamppost_2", "bench_1", "hay_bales_1", "hay_bales_2", "cart_1", "crates_1"] {
        write(&root, &format!("props/{name}.nbt"), prop(name))?;
    }
    write(&root, "walls/tower.nbt", tower(7, 9, 1, false, false))?;
    let gate = Look { wall: "minecraft:stone_bricks", roof: "minecraft:stone_brick_slab[type=bottom,waterlogged=false]", trim: "minecraft:chiseled_stone_bricks" };
    let (size, mut blocks) = building(7, 5, 6, 1, &gate);
    // An opening three wide through it, north to south.
    blocks.retain(|(p, _, _)| !(p[0] >= 2 && p[0] <= 4 && p[1] >= 1 && p[1] <= 4));
    blocks.extend((1..5).flat_map(|y| (2..5).flat_map(move |x| (0..5).map(move |z| ([x, y, z], "minecraft:air".to_owned(), None)))));
    write(&root, "walls/gate.nbt", (size, blocks))?;
    Ok(root)
}
