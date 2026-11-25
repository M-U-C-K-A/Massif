//! The village's stages turned into blocks: roads in the land's top,
//! fields, buildings and props from their schematics, Massif's own props,
//! walls, towers and gates — one structure, and the heights of the land
//! levelled under it.

use super::Stages;
use super::buildings::{Jigsaw, Placement};
use super::pack::{Plan, Variant, plan_of};
use super::props::Builtin;
use super::roads::RoadKind;
use super::walls::{Enclosure, Walls};
use super::{Params, RoadType, Side, Theme, height};
use crate::build::{self, Builder};
use crate::geom::place;
use crate::ground::Ground;
use crate::library::Library;
use crate::nbt::Compound;
use crate::rng::Rng;
use crate::vanilla::{Adapted, Built};
use fastnbt::Value;
use massif_core::structures::Piece;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The blocks of a road of `kind`, its middle and its edges, by weight.
/// Blocks with their weights.
type Weighted = Vec<(String, u32)>;

fn road_blocks(theme: &Theme, road: RoadType, kind: RoadKind) -> (Weighted, Weighted) {
    let w = |v: &[(&str, u32)]| -> Weighted { v.iter().map(|(b, w)| (format!("minecraft:{b}"), *w)).collect() };
    match road {
        RoadType::Theme => {
            let s = if kind == RoadKind::Side { &theme.side } else { &theme.main };
            (s.blocks.clone(), if s.edges.is_empty() { s.blocks.clone() } else { s.edges.clone() })
        }
        RoadType::Dirt => (w(&[("dirt_path", 4), ("coarse_dirt", 1)]), w(&[("dirt_path", 3), ("coarse_dirt", 1)])),
        RoadType::Premium => (w(&[("dirt_path", 5), ("gravel", 2)]), w(&[("coarse_dirt", 3), ("gravel", 1), ("dirt_path", 1)])),
        RoadType::Stone => (w(&[("cobblestone", 4), ("andesite", 2), ("stone_bricks", 2), ("mossy_cobblestone", 1)]), w(&[("stone_bricks", 3), ("cobblestone", 1)])),
        RoadType::Planks => (w(&[("spruce_planks", 3), ("oak_planks", 1)]), w(&[("spruce_planks", 1)])),
    }
}

/// The slab that makes a step on a road of `road`, if it has one.
fn road_slab(road: RoadType) -> Option<&'static str> {
    match road {
        RoadType::Stone => Some("minecraft:cobblestone_slab[type=bottom,waterlogged=false]"),
        RoadType::Planks => Some("minecraft:spruce_slab[type=bottom,waterlogged=false]"),
        _ => None,
    }
}

fn pick<'a>(blocks: &'a [(String, u32)], h: &mut Rng) -> &'a str {
    let total: u32 = blocks.iter().map(|w| w.1).sum::<u32>().max(1);
    let mut pick = h.below(total);
    blocks
        .iter()
        .find(|w| {
            if pick < w.1 {
                true
            } else {
                pick -= w.1;
                false
            }
        })
        .map_or("minecraft:dirt_path", |w| w.0.as_str())
}

/// Where the blocks of the schematics come from: read from the disk
/// ([`DiskPlans`]), or handed over by whoever owns them (the terrain
/// graph's engine), so that laying a village reads nothing itself.
pub trait PlanSource: Sync {
    fn plan(&self, file: &Path) -> Result<Arc<Plan>, String>;
}

/// The schematics read from the disk, each once.
#[derive(Default)]
pub struct DiskPlans(Mutex<HashMap<PathBuf, Result<Arc<Plan>, String>>>);

impl PlanSource for DiskPlans {
    fn plan(&self, file: &Path) -> Result<Arc<Plan>, String> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(file.to_owned()).or_insert_with(|| plan_of(file).map(Arc::new)).clone()
    }
}

/// The plans a village is laid from, what could not be read noted once.
struct Plans<'a> {
    source: &'a dyn PlanSource,
    failed: std::collections::HashSet<PathBuf>,
}

impl Plans<'_> {
    fn get(&mut self, file: &Path, unknown: &mut Vec<String>) -> Option<Arc<Plan>> {
        match self.source.plan(file) {
            Ok(p) => Some(p),
            Err(e) => {
                if self.failed.insert(file.to_owned()) {
                    unknown.push(format!("schematic {} (could not be read: {e})", file.display()));
                }
                None
            }
        }
    }
}

/// A schematic laid as placed.
fn lay_placement(b: &mut Builder, plans: &mut Plans, p: &Placement, villagers: u32, unknown: &mut Vec<String>) {
    let Some(plan) = plans.get(&p.file, unknown) else { return };
    for (q, state, data) in &plan.blocks {
        let w = place(*q, p.origin, p.turn);
        let state = massif_core::objects::transform_state(state, p.turn, false);
        b.set(w, &state, data.clone());
    }
    // Villagers inside the door.
    let inward = p.facing.opposite().step();
    for k in 0..villagers {
        let at = [p.door[0] + inward[0] * (1 + k as i32 % 2), p.door[1], p.door[2] + inward[1] * (1 + k as i32 % 2)];
        let mut v = Compound::new();
        v.insert("id".into(), Value::String("minecraft:villager".into()));
        v.insert("PersistenceRequired".into(), Value::Byte(1));
        b.add_entity([at[0] as f64 + 0.5, at[1] as f64, at[2] as f64 + 0.5], v);
    }
    b.pieces.push(Piece { min: p.min, max: p.max, template: p.kind.clone(), origin: p.origin, turn: p.turn, depth: 0, ground_delta: p.door[1] - p.origin[1], rigid: true, processors: String::new(), key: p.key });
}

/// A fence's state, joined to its neighbours that way.
fn fence(n: bool, e: bool, s: bool, w: bool) -> String {
    format!("minecraft:oak_fence[east={e},north={n},south={s},waterlogged=false,west={w}]")
}

/// One of Massif's own props.
fn lay_builtin(b: &mut Builder, ground: &dyn Ground, p: &Builtin, seed: u64) {
    let [x, y, z] = p.at;
    let along = if matches!(p.facing, Side::North | Side::South) { [1, 0] } else { [0, 1] };
    match p.kind.as_str() {
        "lamppost" => {
            b.set([x, y - 1, z], "minecraft:cobblestone", None);
            b.set([x, y, z], "minecraft:cobblestone_wall[east=none,north=none,south=none,up=true,waterlogged=false,west=none]", None);
            b.set([x, y + 1, z], &fence(false, false, false, false), None);
            b.set([x, y + 2, z], &fence(false, false, false, false), None);
            b.set([x, y + 3, z], "minecraft:lantern[hanging=false,waterlogged=false]", None);
        }
        "bench" => {
            // A seat of three stairs, its back away from the road.
            for k in -1..=1 {
                let c = [x + along[0] * k, y, z + along[1] * k];
                b.set(c, &format!("minecraft:spruce_stairs[facing={},half=bottom,shape=straight,waterlogged=false]", p.facing.opposite().name()), None);
            }
        }
        "hay_bales" => {
            let mut h = Rng::at(seed, p.at);
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let fy = ground.floor(x + dx, z + dz);
                b.set([x + dx, fy, z + dz], "minecraft:hay_block[axis=y]", None);
                if h.chance(0.35) {
                    b.set([x + dx, fy + 1, z + dz], if h.chance(0.5) { "minecraft:hay_block[axis=x]" } else { "minecraft:hay_block[axis=z]" }, None);
                }
            }
        }
        "cart" => {
            // A bed of slabs, a load of hay, a shaft.
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                b.set([x + dx, y, z + dz], "minecraft:spruce_slab[type=top,waterlogged=false]", None);
            }
            b.set([x, y + 1, z], "minecraft:hay_block[axis=y]", None);
            b.set([x + 1, y + 1, z + 1], "minecraft:barrel[facing=up,open=false]", None);
            let s = p.facing.step();
            b.set([x + s[0].max(0) + s[0], y, z + s[1].max(0) + s[1]], &fence(false, false, false, false), None);
        }
        "fence" => {
            let Some([x0, z0, x1, z1]) = p.rect else { return };
            let on = |x: i32, z: i32| (x == x0 || x == x1) && (z0..=z1).contains(&z) || (z == z0 || z == z1) && (x0..=x1).contains(&x);
            // The gate in the middle of the side toward the farm.
            let gate = match p.facing {
                Side::North => [(x0 + x1) / 2, z0],
                Side::South => [(x0 + x1) / 2, z1],
                Side::West => [x0, (z0 + z1) / 2],
                Side::East => [x1, (z0 + z1) / 2],
            };
            for z in z0..=z1 {
                for x in x0..=x1 {
                    if !on(x, z) {
                        continue;
                    }
                    let fy = ground.floor(x, z);
                    let state = if [x, z] == gate {
                        let f = if matches!(p.facing, Side::North | Side::South) { "north" } else { "east" };
                        format!("minecraft:oak_fence_gate[facing={f},in_wall=false,open=false,powered=false]")
                    } else {
                        fence(on(x, z - 1), on(x + 1, z), on(x, z + 1), on(x - 1, z))
                    };
                    b.set([x, fy, z], &state, None);
                }
            }
        }
        _ => {}
    }
}

/// The crop `name` at age `age` (of its seven, or three for beetroots).
fn crop(name: &str, age: u32) -> String {
    let id = match name {
        "carrots" | "carrot" => "carrots",
        "potatoes" | "potato" => "potatoes",
        "beetroots" | "beetroot" => "beetroots",
        _ => "wheat",
    };
    let age = if id == "beetroots" { age.min(3) } else { age.min(7) };
    format!("minecraft:{id}[age={age}]")
}

/// The line `line` (closed) with a column added at each diagonal step, so
/// that fences and walls along it join.
fn four_connected(line: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut out = Vec::with_capacity(line.len() * 3 / 2);
    for (k, &a) in line.iter().enumerate() {
        out.push(a);
        let b = line[(k + 1) % line.len()];
        if a[0] != b[0] && a[1] != b[1] {
            out.push([b[0], a[1]]);
        }
    }
    out
}

/// The full state of block `id` laid in an enclosure: fences and walls
/// joined to what stands beside them (`beside`, north, east, south, west),
/// gates and beams across the line (`along_x`: the line runs east–west
/// there).
fn enclosure_state(id: &str, beside: [Option<&str>; 4], along_x: bool, beam: bool) -> String {
    let joins = |b: Option<&str>| b.is_some_and(|b| !b.ends_with("lantern") && !b.ends_with("leaves"));
    let [n, e, s, w] = beside.map(joins);
    if id.ends_with("_fence_gate") {
        let facing = if along_x { "north" } else { "east" };
        format!("{id}[facing={facing},in_wall=false,open=true,powered=false]")
    } else if id.ends_with("_fence") {
        format!("{id}[east={e},north={n},south={s},waterlogged=false,west={w}]")
    } else if id.ends_with("_wall") {
        let side = |on: bool| if on { "low" } else { "none" };
        let up = !(n && s && !e && !w || e && w && !n && !s);
        format!("{id}[east={},north={},south={},up={up},waterlogged=false,west={}]", side(e), side(n), side(s), side(w))
    } else if id.ends_with("_log") {
        let axis = if !beam { "y" } else if along_x { "x" } else { "z" };
        format!("{id}[axis={axis}]")
    } else if id.ends_with("leaves") {
        format!("{id}[distance=1,persistent=true,waterlogged=false]")
    } else if id.ends_with("lantern") {
        format!("{id}[hanging=false,waterlogged=false]")
    } else {
        id.to_owned()
    }
}

/// A fence, hedge or low wall along the walls' line, as `style` makes it:
/// each column on the land, gates where the roads cross, nothing over
/// water.
fn lay_enclosure(b: &mut Builder, ground: &dyn Ground, style: Enclosure, walls: &Walls, seed: u64) {
    let line = four_connected(&walls.line);
    let on: std::collections::HashSet<(i32, i32)> = line.iter().map(|p| (p[0], p[1])).collect();
    let mut cols: HashMap<(i32, i32), (i32, Vec<&'static str>, bool)> = HashMap::new();
    for (k, &[x, z]) in line.iter().enumerate() {
        let (base, wet) = height(ground, x, z);
        if wet || cols.contains_key(&(x, z)) {
            continue;
        }
        let gate = walls.in_gate(x, z);
        let edge = !gate && [[1, 0], [-1, 0], [0, 1], [0, -1]].iter().any(|d| on.contains(&(x + d[0], z + d[1])) && walls.in_gate(x + d[0], z + d[1]));
        let r = Rng::at(seed ^ 0x656e_636c, [x, 0, z]).next_u64() as u32;
        cols.insert((x, z), (base, style.column(k, r, gate, edge), gate));
    }
    let at = |x: i32, y: i32, z: i32| -> Option<&'static str> {
        let (base, col, _) = cols.get(&(x, z))?;
        let i = usize::try_from(y - base).ok()?;
        col.get(i).copied().filter(|s| !s.is_empty())
    };
    for (&(x, z), (base, col, gate)) in &cols {
        let along_x = on.contains(&(x + 1, z)) || on.contains(&(x - 1, z));
        for (i, id) in col.iter().enumerate() {
            if id.is_empty() {
                continue;
            }
            let y = base + i as i32;
            let beside = [at(x, y, z - 1), at(x + 1, y, z), at(x, y, z + 1), at(x - 1, y, z)];
            b.set([x, y, z], &enclosure_state(id, beside, along_x, *gate), None);
        }
    }
}

/// The walls along their line, on the land: crenellated, towers at the
/// corners and every few blocks, a gate where a road crosses.
fn lay_walls(b: &mut Builder, ground: &dyn Ground, theme: &Theme, walls: &Walls, plans: &mut Plans, unknown: &mut Vec<String>) {
    let style = &theme.walls;
    let h = style.height;
    let block = style.block.as_str();
    let towers = walls.towers(style.towers);
    let near_tower = |x: i32, z: i32| towers.iter().any(|t| (t[0] - x).abs() <= 2 && (t[1] - z).abs() <= 2);
    for (k, &[x, z]) in walls.line.iter().enumerate() {
        let base = ground.floor(x, z);
        if walls.in_gate(x, z) {
            // A gate: the road passes under an arch.
            for y in base + 4..base + h + 1 {
                b.set([x, y, z], block, None);
            }
            continue;
        }
        if near_tower(x, z) {
            continue;
        }
        for y in base - 1..base + h {
            b.set([x, y, z], block, None);
        }
        // Battlements: every other block, one more.
        if k % 2 == 0 {
            b.set([x, base + h, z], block, None);
        }
    }
    let piece = |b: &mut Builder, plans: &mut Plans, unknown: &mut Vec<String>, v: &Variant, x: i32, z: i32| {
        let base = ground.floor(x, z);
        if let Some(plan) = plans.get(&v.file, unknown) {
            let origin = [x - plan.size[0] / 2, base - v.door[1].min(1), z - plan.size[2] / 2];
            for (p, state, data) in &plan.blocks {
                b.set(place(*p, origin, 0), state, data.clone());
            }
        }
    };
    for &[x, z] in &towers {
        let base = ground.floor(x, z);
        if let Some(v) = &style.tower {
            piece(b, plans, unknown, v, x, z);
            continue;
        }
        for dz in -2..=2i32 {
            for dx in -2..=2i32 {
                let wall = dx.abs() == 2 || dz.abs() == 2;
                for y in base - 1..base + h + 4 {
                    b.set([x + dx, y, z + dz], if wall || y == base - 1 { block } else { "minecraft:air" }, None);
                }
                if wall && (dx + dz) % 2 == 0 {
                    b.set([x + dx, base + h + 4, z + dz], block, None);
                }
            }
        }
        // A floor on top, a ladder up.
        for dz in -1..=1 {
            for dx in -1..=1 {
                b.set([x + dx, base + h + 3, z + dz], "minecraft:spruce_planks", None);
            }
        }
        for y in base..base + h + 3 {
            b.set([x + 1, y, z + 1], "minecraft:ladder[facing=north,waterlogged=false]", None);
        }
    }
    if let Some(v) = &style.gate {
        for &[x, z] in &walls.gates {
            piece(b, plans, unknown, v, x, z);
        }
    }
}

/// Lay `stages` on `ground`: the village's structure, numbered `id`.
#[allow(clippy::too_many_arguments)]
pub fn lay(lib: Option<&Library>, plans: &dyn PlanSource, ground: &dyn Ground, theme: &Theme, params: &Params, stages: &Stages, jigsaw: Option<&Jigsaw>, seed: u64, id: u32) -> Result<Built, String> {
    let mut unknown: Vec<String> = stages.buildings.notes.clone();
    let mut heights = stages.buildings.height_map();
    if let Some(j) = jigsaw {
        for (k, v) in &j.heights {
            heights.entry(*k).or_insert(*v);
        }
    }
    let adapted = Adapted { inner: ground, heights };
    let mut b = Builder::new(&adapted, seed);
    let mut plans = Plans { source: plans, failed: Default::default() };
    // The game's villages.
    if let Some(j) = jigsaw {
        for pieces in &j.pieces {
            let centre = build::center_of(pieces);
            for p in pieces {
                build::place_piece(lib.ok_or("the game's villages need the game")?, &mut b, p, centre);
            }
        }
    }
    // Roads on the land, a block of their style in its top (their edges'
    // where they have their own); planks over water; slabs where they
    // climb, if their blocks have them.
    let road = stages.roads.cells(&stages.area);
    let mut inside: Vec<((i32, i32), RoadKind)> = road.iter().map(|(k, v)| (*k, *v)).collect();
    inside.sort();
    let slab = road_slab(params.road);
    for &((x, z), kind) in &inside {
        if b.get([x, adapted.floor(x, z) - 1, z]).is_some() {
            continue;
        }
        let (floor, wet) = height(&adapted, x, z);
        if wet {
            b.set([x, adapted.surface(x, z) - 1, z], "minecraft:oak_planks", None);
            continue;
        }
        let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dz)| !road.contains_key(&(x + dx, z + dz)));
        let (middle, edges) = road_blocks(theme, params.road, kind);
        let mut h = Rng::at(seed, [x, 0, z]);
        let state = pick(if edge { &edges } else { &middle }, &mut h).to_owned();
        b.set([x, floor - 1, z], &state, None);
        // A step up to a higher neighbour.
        let climbs = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|(dx, dz)| road.contains_key(&(x + dx, z + dz)) && adapted.floor(x + dx, z + dz) == floor + 1);
        match slab {
            Some(s) if climbs => b.set([x, floor, z], s, None),
            // Clear what grows over it.
            _ => b.set([x, floor, z], "minecraft:air", None),
        }
    }
    // Fields: farmland and crops, a row of water every nine.
    for f in &stages.buildings.fields {
        let mut h = Rng::at(seed, [f.rect[0], 1, f.rect[1]]);
        let wide = f.rect[2] - f.rect[0] >= f.rect[3] - f.rect[1];
        for z in f.rect[1]..=f.rect[3] {
            for x in f.rect[0]..=f.rect[2] {
                let along = if wide { z - f.rect[1] } else { x - f.rect[0] };
                if along % 9 == 4 {
                    b.set([x, f.floor - 1, z], "minecraft:water[level=0]", None);
                    b.set([x, f.floor, z], "minecraft:air", None);
                } else {
                    b.set([x, f.floor - 1, z], "minecraft:farmland[moisture=7]", None);
                    b.set([x, f.floor, z], &crop(&f.crop, 3 + h.below(5)), None);
                }
            }
        }
    }
    // The buildings, then the pack's props.
    for p in &stages.buildings.placed {
        lay_placement(&mut b, &mut plans, p, p.villagers, &mut unknown);
    }
    for p in &stages.props.placed {
        lay_placement(&mut b, &mut plans, p, 0, &mut unknown);
    }
    for p in &stages.props.builtin {
        lay_builtin(&mut b, &adapted, p, seed);
    }
    // Walls, towers and gates.
    if let Some(w) = &stages.walls
        && !w.line.is_empty()
    {
        if params.enclosure == Enclosure::Ramparts {
            lay_walls(&mut b, &adapted, theme, w, &mut plans, &mut unknown);
        } else {
            lay_enclosure(&mut b, &adapted, params.enclosure, w, seed);
        }
    }
    let name = if theme.pack.is_empty() { format!("{} village", theme.name) } else { theme.name.clone() };
    let mut s = b.finish(id, &name, "", true, lib.map_or(0, |l| l.data_version()));
    if s.sections.is_empty() {
        return Err("nothing could be built there: too steep, too wet, or too small".into());
    }
    s.kind = String::new();
    if let Some(l) = lib {
        unknown.extend(l.unknowns());
    }
    Ok(Built { structure: s, heights: adapted.heights.clone(), unknown })
}
