//! The game's structures built by its code rather than from templates: the
//! desert and jungle temples, the witch hut, the stronghold. Massif builds
//! them with code of its own, after the game's: the same rooms, traps,
//! loot tables and spawners, laid out its own way.

use crate::build::{self, Builder};
use crate::geom::Dir;
use crate::ground::Ground;
use crate::nbt::Compound;
use crate::rng::Rng;
use crate::vanilla::{Adapted, Recipe, Request};
use fastnbt::Value;
use massif_core::structures::Piece;
use std::collections::HashMap;

pub fn build(ground: &dyn Ground, req: &Request, rng: &mut Rng) -> Result<crate::vanilla::Laid, String> {
    let turn = req.turn.unwrap_or_else(|| rng.below(4) as u8);
    let (x, z) = (req.at[0], req.at[2]);
    // Its box: centred on the column clicked.
    let size = match req.kind.recipe {
        Recipe::DesertPyramid => [21, 15, 21],
        Recipe::JunglePyramid => [12, 10, 15],
        Recipe::SwampHut => [7, 7, 9],
        _ => [0, 0, 0],
    };
    let (w, l) = if turn % 2 == 1 { (size[2], size[0]) } else { (size[0], size[2]) };
    let corner = [x - w / 2, 0, z - l / 2];
    let mut heights = HashMap::new();
    let surface = match req.kind.recipe {
        // On the lowest of the land under it, as the game sets them; the
        // land flattened under it, where asked.
        Recipe::DesertPyramid | Recipe::JunglePyramid => {
            let mut low = i32::MAX;
            for dz in 0..l {
                for dx in 0..w {
                    low = low.min(ground.floor(corner[0] + dx, corner[2] + dz));
                }
            }
            if req.flatten {
                for dz in -1..=l {
                    for dx in -1..=w {
                        let (cx, cz) = (corner[0] + dx, corner[2] + dz);
                        if ground.floor(cx, cz) != low {
                            heights.insert((cx, cz), low);
                        }
                    }
                }
            }
            low
        }
        _ => ground.surface(x, z),
    };
    let adapted = Adapted { inner: ground, heights };
    let mut b = Builder::new(&adapted, req.seed);
    match req.kind.recipe {
        Recipe::DesertPyramid => {
            let mut f = Frame { b: &mut b, origin: [corner[0], surface, corner[2]], turn, size };
            desert_pyramid(&mut f, rng);
        }
        Recipe::JunglePyramid => {
            let mut f = Frame { b: &mut b, origin: [corner[0], surface, corner[2]], turn, size };
            jungle_pyramid(&mut f, rng);
        }
        Recipe::SwampHut => {
            let mut f = Frame { b: &mut b, origin: [corner[0], surface, corner[2]], turn, size };
            swamp_hut(&mut f, rng);
        }
        Recipe::Stronghold => stronghold(&mut b, [x, req.at[1], z], turn, rng),
        _ => return Err(format!("{} is not built by code", req.kind.name)),
    }
    if matches!(req.kind.recipe, Recipe::DesertPyramid | Recipe::JunglePyramid | Recipe::SwampHut) {
        let (min, max) = ([corner[0], surface - 15, corner[2]], [corner[0] + w - 1, surface + size[1], corner[2] + l - 1]);
        b.pieces.push(Piece { min, max, template: String::new(), origin: [corner[0], surface, corner[2]], turn, depth: 0, ground_delta: 0, rigid: true, processors: String::new() });
    }
    let heights = adapted.heights.clone();
    Ok((b.finish(req.id, req.kind.name, req.kind.game_id, true, 0), heights))
}

/// A box of blocks laid out in its own coordinates (`0..size`), turned as a
/// whole in place, its blocks' facings turned with it.
struct Frame<'a, 'b> {
    b: &'a mut Builder<'b>,
    origin: [i32; 3],
    turn: u8,
    size: [i32; 3],
}

impl Frame<'_, '_> {
    fn world(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        let (sx, sz) = (self.size[0], self.size[2]);
        let (rx, rz) = match self.turn & 3 {
            0 => (x, z),
            1 => (sz - 1 - z, x),
            2 => (sx - 1 - x, sz - 1 - z),
            _ => (z, sx - 1 - x),
        };
        [self.origin[0] + rx, self.origin[1] + y, self.origin[2] + rz]
    }

    fn set(&mut self, x: i32, y: i32, z: i32, state: &str) {
        let p = self.world(x, y, z);
        let state = massif_core::objects::transform_state(state, self.turn, false);
        self.b.set(p, &state, None);
    }

    fn set_with(&mut self, x: i32, y: i32, z: i32, state: &str, data: Compound) {
        let p = self.world(x, y, z);
        let state = massif_core::objects::transform_state(state, self.turn, false);
        self.b.set(p, &state, Some(data));
    }

    fn fill(&mut self, (x0, y0, z0): (i32, i32, i32), (x1, y1, z1): (i32, i32, i32), state: &str) {
        for y in y0.min(y1)..=y0.max(y1) {
            for z in z0.min(z1)..=z0.max(z1) {
                for x in x0.min(x1)..=x0.max(x1) {
                    self.set(x, y, z, state);
                }
            }
        }
    }

    /// The walls of a box in `wall`, its inside `inside`.
    fn hollow(&mut self, a: (i32, i32, i32), c: (i32, i32, i32), wall: &str, inside: &str) {
        for y in a.1..=c.1 {
            for z in a.2..=c.2 {
                for x in a.0..=c.0 {
                    let edge = x == a.0 || x == c.0 || y == a.1 || y == c.1 || z == a.2 || z == c.2;
                    self.set(x, y, z, if edge { wall } else { inside });
                }
            }
        }
    }

    /// Fill down from `y` while the land below is not solid: a building's
    /// footing.
    fn footing(&mut self, x: i32, y: i32, z: i32, state: &str) {
        for d in 0..32 {
            let p = self.world(x, y - d, z);
            if self.b.ground.block(p[0], p[1], p[2]).is_solid() && self.b.get(p).is_none() {
                break;
            }
            self.set(x, y - d, z, state);
        }
    }

    fn chest(&mut self, x: i32, y: i32, z: i32, facing: Dir, table: &str, rng: &mut Rng) {
        let state = format!("minecraft:chest[facing={},type=single,waterlogged=false]", facing.name());
        self.set_with(x, y, z, &state, build::loot("minecraft:chest", table, rng));
    }
}

/// A spawner of `mob`.
fn spawner(mob: &str) -> Compound {
    let mut entity = Compound::new();
    entity.insert("id".into(), Value::String(mob.into()));
    let mut data = Compound::new();
    data.insert("entity".into(), Value::Compound(entity));
    let mut c = Compound::new();
    c.insert("id".into(), Value::String("minecraft:mob_spawner".into()));
    c.insert("SpawnData".into(), Value::Compound(data));
    c.insert("Delay".into(), Value::Short(20));
    c
}

fn stairs(material: &str, facing: Dir, upside: bool) -> String {
    format!("minecraft:{material}_stairs[facing={},half={},shape=straight,waterlogged=false]", facing.name(), if upside { "top" } else { "bottom" })
}

// ------------------------------------------------------------ the desert --

/// The desert temple: a sandstone pyramid on a platform, its two towers at
/// the front, orange and blue terracotta, and under its floor the hidden
/// room of four chests over a trap of TNT; sand that hides an
/// archaeologist's finds in the cellar beside it.
fn desert_pyramid(f: &mut Frame, rng: &mut Rng) {
    const SAND: &str = "minecraft:sandstone";
    const CUT: &str = "minecraft:cut_sandstone";
    const CARVED: &str = "minecraft:chiseled_sandstone";
    const AIR: &str = "minecraft:air";
    let (w, d) = (21, 21);
    // The platform, sunk into the sand, and footings to the land.
    f.fill((0, -4, 0), (w - 1, 0, d - 1), SAND);
    for x in 0..w {
        for z in 0..d {
            f.footing(x, -5, z, SAND);
        }
    }
    // The pyramid's steps, hollow.
    for i in 1..=9 {
        f.fill((i, i, i), (w - 1 - i, i, d - 1 - i), SAND);
        f.fill((i + 1, i, i + 1), (w - 2 - i, i, d - 2 - i), AIR);
    }
    // The hall inside, its pillars.
    f.fill((5, 1, 5), (w - 6, 4, d - 6), AIR);
    for (x, z) in [(6, 6), (6, 14), (14, 6), (14, 14)] {
        f.fill((x, 1, z), (x, 4, z), CUT);
    }
    // The two towers at the front, hollow, their stairs up.
    for x0 in [0, w - 5] {
        f.hollow((x0, 0, 0), (x0 + 4, 9, 4), SAND, AIR);
        f.fill((x0 + 1, 10, 1), (x0 + 3, 10, 3), SAND);
        f.set(x0 + 2, 10, 0, &stairs("sandstone", Dir::North, false));
        f.set(x0 + 2, 10, 4, &stairs("sandstone", Dir::South, false));
        f.set(x0, 10, 2, &stairs("sandstone", Dir::West, false));
        f.set(x0 + 4, 10, 2, &stairs("sandstone", Dir::East, false));
        // Their faces: bands of orange terracotta, a carved eye.
        for y in [3, 5, 7] {
            for x in x0 + 1..=x0 + 3 {
                f.set(x, y, 0, "minecraft:orange_terracotta");
            }
        }
        f.set(x0 + 2, 6, 0, "minecraft:blue_terracotta");
        f.set(x0 + 2, 4, 0, CARVED);
        // A way in from the hall.
        f.fill((x0 + 2, 1, 4), (x0 + 2, 2, 5), AIR);
    }
    // The gate between the towers.
    f.hollow((8, 0, 0), (12, 4, 4), SAND, AIR);
    f.fill((9, 1, 0), (11, 3, 4), AIR);
    for (x, y) in [(8, 3), (12, 3), (8, 1), (12, 1)] {
        f.set(x, y, 0, CUT);
    }
    f.set(10, 4, 0, CARVED);
    f.fill((9, 1, 5), (11, 3, 5), AIR);
    // The floor of the hall: orange terracotta round a blue heart.
    for (dx, dz) in [(-2, 0), (2, 0), (0, -2), (0, 2), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
        f.set(10 + dx, 0, 10 + dz, "minecraft:orange_terracotta");
    }
    f.set(10, 0, 10, "minecraft:blue_terracotta");
    // The hidden room under it: a shaft, the room, four chests, the trap.
    f.fill((10, -10, 10), (10, -1, 10), AIR);
    f.hollow((7, -14, 7), (13, -9, 13), CUT, AIR);
    for (x, z) in [(8, 8), (8, 12), (12, 8), (12, 12)] {
        f.fill((x, -13, z), (x, -10, z), CARVED);
    }
    f.set(10, -10, 10, AIR);
    for dir in Dir::HORIZONTAL {
        let s = dir.step();
        f.chest(10 + s[0] * 2, -13, 10 + s[2] * 2, dir.opposite(), "minecraft:chests/desert_pyramid", rng);
    }
    f.set(10, -13, 10, "minecraft:stone_pressure_plate[powered=false]");
    f.fill((9, -16, 9), (11, -15, 11), "minecraft:tnt[unstable=false]");
    f.fill((7, -17, 7), (13, -17, 13), SAND);
    // The cellar beside it, behind a sandstone wall, its suspicious sand.
    f.hollow((14, -8, 8), (18, -4, 12), CUT, AIR);
    f.fill((13, -7, 10), (14, -6, 10), AIR);
    let mut finds = 0;
    for x in 15..=17 {
        for z in 9..=11 {
            if finds < 4 && rng.chance(0.4) {
                let data = build::loot("minecraft:brushable_block", "minecraft:archaeology/desert_pyramid", rng);
                f.set_with(x, -8, z, "minecraft:suspicious_sand[dusted=0]", data);
                finds += 1;
            } else {
                f.set(x, -8, z, "minecraft:sand");
            }
        }
    }
}

// ------------------------------------------------------------- the jungle --

/// The jungle temple: three floors of mossy cobblestone, a stair down to
/// the cellar, its lever puzzle guarding a chest, a corridor of tripwire
/// and dispensers of arrows guarding the other.
fn jungle_pyramid(f: &mut Frame, rng: &mut Rng) {
    let stone = |rng: &mut Rng| if rng.chance(0.4) { "minecraft:mossy_cobblestone" } else { "minecraft:cobblestone" };
    let (w, d) = (12, 15);
    // Footings, then the three floors, each smaller.
    for x in 0..w {
        for z in 0..d {
            let s = stone(rng);
            f.footing(x, -4, z, s);
        }
    }
    for (y0, y1, inset) in [(-4, 3, 0), (4, 6, 1), (7, 9, 2)] {
        for y in y0..=y1 {
            for z in inset..d - inset {
                for x in inset..w - inset {
                    let edge = x == inset || x == w - 1 - inset || z == inset || z == d - 1 - inset || y == y0;
                    let s = stone(rng);
                    f.set(x, y, z, if edge { s } else { "minecraft:air" });
                }
            }
        }
    }
    // Floors between, the door at the front, windows.
    for (y, inset) in [(3, 1), (6, 2)] {
        for z in inset..d - inset {
            for x in inset..w - inset {
                let s = stone(rng);
                f.set(x, y, z, s);
            }
        }
    }
    f.fill((5, 0, 0), (6, 2, 0), "minecraft:air");
    for z in [4, 8, 11] {
        f.set(0, 1, z, "minecraft:air");
        f.set(w - 1, 1, z, "minecraft:air");
    }
    f.fill((5, 4, 1), (6, 5, 1), "minecraft:air");
    f.fill((5, 7, 2), (6, 8, 2), "minecraft:air");
    // Stairs between the floors.
    for k in 0..3 {
        f.set(2, 1 + k, 4 + k, &stairs("cobblestone", Dir::South, false));
        f.set(2, 2 + k, 4 + k, "minecraft:air");
    }
    f.fill((2, 3, 6), (3, 3, 7), "minecraft:air");
    // Down to the cellar.
    for k in 0..4 {
        f.set(9, -1 - k, 3 + k, &stairs("cobblestone", Dir::South, false));
        f.fill((9, -k, 3 + k), (9, 1, 3 + k), "minecraft:air");
    }
    f.fill((1, -3, 1), (w - 2, -1, d - 2), "minecraft:air");
    // The lever puzzle: three levers on the wall; behind the wall the
    // first chest.
    for x in [3, 4, 5] {
        f.set(x, -2, 2, "minecraft:lever[face=wall,facing=south,powered=false]");
    }
    f.set(1, -3, 12, &stairs("cobblestone", Dir::South, false));
    f.chest(2, -3, 12, Dir::North, "minecraft:chests/jungle_temple", rng);
    f.fill((1, -3, 13), (3, -1, 13), "minecraft:mossy_cobblestone");
    // The corridor of the trap: tripwire across, a dispenser of arrows at
    // its end, the second chest past it.
    f.set(9, -3, 8, "minecraft:tripwire_hook[attached=true,facing=east,powered=false]");
    f.set(10, -3, 8, "minecraft:tripwire[attached=true,disarmed=false,east=true,north=false,powered=false,south=false,west=true]");
    f.set(11, -3, 8, "minecraft:tripwire_hook[attached=true,facing=west,powered=false]");
    f.set_with(10, -3, 11, "minecraft:dispenser[facing=north,triggered=false]", build::loot("minecraft:dispenser", "minecraft:chests/jungle_temple_dispenser", rng));
    f.set(10, -3, 10, "minecraft:redstone_wire[east=none,north=side,power=0,south=side,west=none]");
    f.set(10, -3, 9, "minecraft:redstone_wire[east=none,north=side,power=0,south=side,west=none]");
    f.set_with(8, -3, 13, "minecraft:dispenser[facing=north,triggered=false]", build::loot("minecraft:dispenser", "minecraft:chests/jungle_temple_dispenser", rng));
    f.chest(9, -3, 12, Dir::West, "minecraft:chests/jungle_temple", rng);
    // Carved stone and vines on its faces.
    for y in [1, 5] {
        f.set(4, y, 0, "minecraft:chiseled_stone_bricks");
        f.set(7, y, 0, "minecraft:chiseled_stone_bricks");
    }
}

// --------------------------------------------------------------- the swamp --

/// The witch hut: spruce on stilts of oak, a cauldron, a pot with a red
/// mushroom, a crafting table, the witch and her black cat.
fn swamp_hut(f: &mut Frame, rng: &mut Rng) {
    let _ = rng;
    const PLANKS: &str = "minecraft:spruce_planks";
    // Stilts down to the ground.
    for (x, z) in [(1, 2), (5, 2), (1, 7), (5, 7)] {
        f.footing(x, 0, z, "minecraft:oak_log[axis=y]");
    }
    // The floor and the porch.
    f.fill((1, 1, 1), (5, 1, 7), PLANKS);
    // The walls, the door at the porch, windows.
    f.hollow((1, 2, 2), (5, 4, 7), PLANKS, "minecraft:air");
    f.fill((1, 1, 2), (5, 1, 7), PLANKS);
    f.fill((3, 2, 2), (3, 3, 2), "minecraft:air");
    f.set(1, 3, 4, "minecraft:air");
    f.set(5, 3, 4, "minecraft:air");
    for (x, z) in [(1, 2), (5, 2), (1, 7), (5, 7)] {
        f.fill((x, 2, z), (x, 4, z), "minecraft:oak_log[axis=y]");
    }
    f.set(1, 2, 1, "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]");
    f.set(5, 2, 1, "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]");
    // The roof: spruce stairs along its sides, planks on top.
    for z in 1..=8 {
        f.set(0, 4, z, &stairs("spruce", Dir::East, false));
        f.set(6, 4, z, &stairs("spruce", Dir::West, false));
    }
    f.fill((1, 5, 1), (5, 5, 8), PLANKS);
    // Inside.
    f.set(4, 2, 6, "minecraft:cauldron");
    f.set(2, 2, 6, "minecraft:crafting_table");
    f.set(4, 3, 3, "minecraft:potted_red_mushroom");
    // The witch and her black cat.
    let p = f.world(3, 2, 5);
    let mut witch = Compound::new();
    witch.insert("id".into(), Value::String("minecraft:witch".into()));
    witch.insert("PersistenceRequired".into(), Value::Byte(1));
    f.b.add_entity([p[0] as f64 + 0.5, p[1] as f64, p[2] as f64 + 0.5], witch);
    let mut cat = Compound::new();
    cat.insert("id".into(), Value::String("minecraft:cat".into()));
    cat.insert("variant".into(), Value::String("minecraft:all_black".into()));
    cat.insert("PersistenceRequired".into(), Value::Byte(1));
    let p = f.world(2, 2, 4);
    f.b.add_entity([p[0] as f64 + 0.5, p[1] as f64, p[2] as f64 + 0.5], cat);
}

// ---------------------------------------------------------- the stronghold --

/// What a room of a stronghold is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Room {
    Corridor,
    Crossing,
    Library,
    Prison,
    Store,
    Fountain,
    Portal,
}

/// A stronghold: corridors of stone bricks that wind and cross, a spiral
/// stair up from its first room, and off them its rooms — prison cells, a
/// fountain, a library or two, store rooms with chests, and the portal
/// room: the End portal's frames round a pool of lava, its eyes set by
/// chance, a silverfish spawner on the stairs.
fn stronghold(b: &mut Builder, start: [i32; 3], turn: u8, rng: &mut Rng) {
    // Rooms on a grid of cells 11 wide, 7 high: a walk from the start,
    // branching, each step a corridor, every few a room.
    const CELL: i32 = 11;
    let mut cells: HashMap<(i32, i32, i32), Room> = HashMap::new();
    let mut open: Vec<(i32, i32, i32)> = vec![(0, 0, 0)];
    cells.insert((0, 0, 0), Room::Crossing);
    let want = 28 + rng.below(12) as i32;
    let mut tries = 0;
    while (cells.len() as i32) < want && tries < 2000 {
        tries += 1;
        let Some(&from) = rng.pick(&open) else { break };
        let dir = Dir::HORIZONTAL[rng.below(4) as usize].step();
        let level = if rng.chance(0.15) { if rng.chance(0.5) { 1 } else { -1 } } else { 0 };
        let to = (from.0 + dir[0], (from.1 + level).clamp(-1, 1), from.2 + dir[2]);
        if cells.contains_key(&to) {
            continue;
        }
        let room = match rng.below(20) {
            0..=8 => Room::Corridor,
            9..=11 => Room::Crossing,
            12..=13 => Room::Prison,
            14..=16 => Room::Store,
            17 => Room::Fountain,
            _ => Room::Library,
        };
        cells.insert(to, room);
        open.push(to);
    }
    // The portal room: the cell farthest from the start.
    let far = *cells.keys().max_by_key(|c| c.0.abs() + c.2.abs() + c.1.abs() * 2).expect("a start");
    cells.insert(far, Room::Portal);
    // At least one library.
    if !cells.values().any(|r| *r == Room::Library) {
        let k = *cells.keys().filter(|c| **c != far && **c != (0, 0, 0)).nth(rng.below((cells.len() as u32).saturating_sub(2).max(1)) as usize).unwrap_or(&(0, 0, 0));
        if k != (0, 0, 0) {
            cells.insert(k, Room::Library);
        }
    }
    let world = |c: (i32, i32, i32)| -> [i32; 3] {
        let (rx, rz) = massif_core::objects::turn_xz(c.0 * CELL, c.2 * CELL, turn);
        [start[0] + rx, start[1] + c.1 * 7, start[2] + rz]
    };
    let mut keys: Vec<(i32, i32, i32)> = cells.keys().copied().collect();
    keys.sort();
    for &c in &keys {
        let room = cells[&c];
        let o = world(c);
        let mut f = Frame { b, origin: [o[0] - CELL / 2, o[1], o[2] - CELL / 2], turn: 0, size: [CELL, 7, CELL] };
        // Walls, floor and ceiling of stone bricks, some mossy or cracked.
        for y in 0..7 {
            for z in 0..CELL {
                for x in 0..CELL {
                    let edge = x == 0 || x == CELL - 1 || z == 0 || z == CELL - 1 || y == 0 || y == 6;
                    let state = if edge { brick(rng) } else { "minecraft:air" };
                    f.set(x, y, z, state);
                }
            }
        }
        // Doors toward the neighbouring cells.
        for d in Dir::HORIZONTAL {
            let s = d.step();
            let n = (c.0 + s[0], c.1, c.2 + s[2]);
            if cells.contains_key(&n) {
                let (dx, dz) = (CELL / 2 + s[0] * (CELL / 2), CELL / 2 + s[2] * (CELL / 2));
                let (wx, wz) = if s[0] != 0 { (0, 1) } else { (1, 0) };
                for k in -1..=1 {
                    f.fill((dx + wx * k, 1, dz + wz * k), (dx + wx * k, 3, dz + wz * k), "minecraft:air");
                }
            }
        }
        // Stairs to the cells above and below.
        let up = (c.0, c.1 + 1, c.2);
        if cells.contains_key(&up) {
            for k in 0..6 {
                f.set(2 + k, 1 + k, 2, &stairs("stone_brick", Dir::East, false));
                f.fill((2 + k, 2 + k, 2), (2 + k, 6, 2), "minecraft:air");
            }
        }
        match room {
            Room::Corridor | Room::Crossing => {
                if rng.chance(0.3) {
                    f.set(CELL / 2, 3, 1, "minecraft:wall_torch[facing=south]");
                }
            }
            Room::Prison => {
                for z in [2, 7] {
                    f.fill((2, 1, z), (8, 3, z), "minecraft:iron_bars[east=true,north=false,south=false,waterlogged=false,west=true]");
                    f.set(5, 1, z, "minecraft:iron_door[facing=south,half=lower,hinge=left,open=false,powered=false]");
                    f.set(5, 2, z, "minecraft:iron_door[facing=south,half=upper,hinge=left,open=false,powered=false]");
                }
            }
            Room::Store => {
                f.chest(1, 1, CELL / 2, Dir::East, "minecraft:chests/stronghold_corridor", rng);
                if rng.chance(0.5) {
                    f.chest(CELL - 2, 1, CELL / 2, Dir::West, "minecraft:chests/stronghold_crossing", rng);
                }
                f.set(CELL / 2, 1, CELL / 2, "minecraft:cobblestone");
            }
            Room::Fountain => {
                f.fill((4, 1, 4), (6, 1, 6), "minecraft:stone_bricks");
                f.set(5, 1, 5, "minecraft:water[level=0]");
                f.fill((5, 2, 5), (5, 4, 5), "minecraft:stone_bricks");
            }
            Room::Library => {
                for x in [1, CELL - 2] {
                    for z in 1..CELL - 1 {
                        f.fill((x, 1, z), (x, 4, z), "minecraft:bookshelf");
                    }
                }
                for z in [1, CELL - 2] {
                    for x in 2..CELL - 2 {
                        if x != CELL / 2 {
                            f.fill((x, 1, z), (x, 4, z), "minecraft:bookshelf");
                        }
                    }
                }
                f.fill((3, 1, 5), (7, 1, 5), "minecraft:oak_planks");
                f.set(5, 2, 5, "minecraft:cobweb");
                f.chest(4, 2, 5, Dir::South, "minecraft:chests/stronghold_library", rng);
            }
            Room::Portal => portal_room(&mut f, rng),
        }
    }
    // A spiral stair up from the first room, a way out toward the land.
    let o = world((0, 0, 0));
    for k in 0..12 {
        let p = [o[0] + [1, 1, -1, -1][(k % 4) as usize], o[1] + 6 + k, o[2] + [1, -1, -1, 1][(k % 4) as usize]];
        b.set(p, "minecraft:stone_brick_slab[type=bottom,waterlogged=false]", None);
        b.set([o[0], o[1] + 6 + k, o[2]], "minecraft:stone_bricks", None);
    }
    for &c in &keys {
        let p = world(c);
        b.pieces.push(Piece { min: [p[0] - CELL / 2, p[1], p[2] - CELL / 2], max: [p[0] + CELL / 2, p[1] + 6, p[2] + CELL / 2], template: String::new(), origin: p, turn, depth: 0, ground_delta: 0, rigid: true, processors: String::new() });
    }
}

fn brick(rng: &mut Rng) -> &'static str {
    match rng.below(10) {
        0 | 1 => "minecraft:mossy_stone_bricks",
        2 | 3 => "minecraft:cracked_stone_bricks",
        4 if rng.chance(0.2) => "minecraft:infested_stone_bricks",
        _ => "minecraft:stone_bricks",
    }
}

/// The portal room: twelve frames round a 3 × 3 well (each frame's eye
/// set one time in ten, as in the game), a pool of lava under it, stairs
/// up to it and the silverfish spawner on them.
fn portal_room(f: &mut Frame, rng: &mut Rng) {
    // The pool of lava, walled.
    f.fill((3, 1, 3), (7, 1, 7), "minecraft:stone_bricks");
    f.fill((4, 1, 4), (6, 1, 6), "minecraft:lava[level=0]");
    // The frames round the well, one block up, facing in.
    let frame = |f: &mut Frame, x: i32, z: i32, facing: Dir, rng: &mut Rng| {
        let eye = rng.chance(0.1);
        f.set(x, 2, z, &format!("minecraft:end_portal_frame[eye={eye},facing={}]", facing.name()));
    };
    for k in 4..=6 {
        frame(f, k, 3, Dir::South, rng);
        frame(f, k, 7, Dir::North, rng);
        frame(f, 3, k, Dir::East, rng);
        frame(f, 7, k, Dir::West, rng);
    }
    // Stairs up to it from the door, and the spawner on the landing.
    for k in 0..2 {
        f.set(5, 1 + k, 8 + k, &stairs("stone_brick", Dir::North, false));
    }
    f.set_with(5, 3, 9, "minecraft:spawner", spawner("minecraft:silverfish"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;
    use crate::vanilla::kind;
    use massif_core::structures::Structure;

    fn built(id: &str, seed: u64) -> Structure {
        let ground = Flat { height: 64, water: None };
        let k = kind(id).unwrap();
        let y = match k.height {
            crate::vanilla::Height::Depth(d) => d,
            _ => 64,
        };
        let req = Request { kind: k, at: [0, y, 0], turn: None, seed, flatten: true, id: 1 };
        build(&ground, &req, &mut Rng::new(seed)).unwrap().0
    }

    fn count(s: &Structure, prefix: &str) -> usize {
        s.sections.iter().map(|sec| sec.blocks.iter().filter(|b| s.palette[b.1 as usize].starts_with(prefix)).count()).sum()
    }

    #[test]
    fn the_temples_hut_and_stronghold_are_built() {
        for seed in 0..4 {
            let desert = built("desert_pyramid", seed);
            assert_eq!(count(&desert, "minecraft:chest"), 4, "four chests in the hidden room");
            assert!(count(&desert, "minecraft:tnt") >= 9);
            assert!(desert.block_entities.iter().filter(|b| String::from_utf8_lossy(&b.nbt).contains("desert_pyramid")).count() >= 4);
            let jungle = built("jungle_pyramid", seed);
            assert_eq!(count(&jungle, "minecraft:chest"), 2);
            assert_eq!(count(&jungle, "minecraft:dispenser"), 2);
            let hut = built("swamp_hut", seed);
            assert_eq!(hut.entities.len(), 2, "the witch and her cat");
            let s = built("stronghold", seed);
            assert_eq!(count(&s, "minecraft:end_portal_frame"), 12, "the portal's twelve frames");
            assert_eq!(count(&s, "minecraft:spawner"), 1);
            assert!(count(&s, "minecraft:bookshelf") > 20, "a library");
            assert!(s.block_entities.iter().any(|b| String::from_utf8_lossy(&b.nbt).contains("stronghold_")));
        }
    }
}
