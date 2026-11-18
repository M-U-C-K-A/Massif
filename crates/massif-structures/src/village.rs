//! The village brush: a selection filled with a village — a square in each
//! district, main roads that follow the land and join them, side roads off
//! them, buildings of a theme along both, their doors to the road, on
//! levelled ground, and walls round it all if asked.
//!
//! A theme is either one of the game's villages, assembled by the jigsaw
//! engine in each district ([`built_in`]), or a builder's pack: folders of
//! schematics by type of building, with a small manifest ([`read_pack`],
//! documented in `docs/village-packs.md`).

use crate::build::{self, Builder};
use crate::geom::{BBox, Dir, place};
use crate::ground::Ground;
use crate::library::Library;
use crate::nbt::Compound;
use crate::rng::Rng;
use crate::vanilla::{self, Adapted, Built};
use fastnbt::Value;
use massif_core::coords::BlockRect;
use massif_core::structures::Piece;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};

// --------------------------------------------------------------- themes --

/// The blocks of a road, with their weights, and its width.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadStyle {
    pub width: i32,
    pub blocks: Vec<(String, u32)>,
}

/// A type of building of a theme: its share of the buildings, its rules,
/// its schematics.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildingType {
    pub name: String,
    pub share: f32,
    /// At most this many in a district.
    pub max: Option<u32>,
    /// Placed near the middle of its district.
    pub centre: bool,
    /// Placed near the edge of the village.
    pub edge: bool,
    /// Placed only once a building of that type stands.
    pub near: Option<String>,
    pub files: Vec<PathBuf>,
}

/// Walls without pieces of their own, and the pieces a theme may have.
#[derive(Debug, Clone, PartialEq)]
pub struct WallStyle {
    pub height: i32,
    pub block: String,
    /// A tower every so many blocks along the wall.
    pub towers: i32,
    pub tower: Option<PathBuf>,
    pub gate: Option<PathBuf>,
}

/// A theme of village.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    /// The pack it comes from; empty for one of the game's.
    pub pack: String,
    /// One of the game's villages, by the key of its kind
    /// (`village_plains`): its districts are assembled as the game's.
    pub vanilla: Option<&'static str>,
    pub types: Vec<BuildingType>,
    pub main: RoadStyle,
    pub side: RoadStyle,
    pub walls: WallStyle,
    /// Layers of foundation under the floor of a building without a door.
    pub foundation: i32,
    /// Villagers in each house.
    pub villagers: u32,
}

fn road(blocks: &[(&str, u32)], width: i32) -> RoadStyle {
    RoadStyle { width, blocks: blocks.iter().map(|(b, w)| (b.to_string(), *w)).collect() }
}

/// The game's five villages, as themes: no pack needed.
pub fn built_in() -> Vec<Theme> {
    let theme = |name: &str, kind: &'static str, path: &[(&str, u32)], wall: &str| Theme {
        name: name.to_owned(),
        pack: String::new(),
        vanilla: Some(kind),
        types: Vec::new(),
        main: road(path, 3),
        side: road(path, 2),
        walls: WallStyle { height: 6, block: wall.to_owned(), towers: 48, tower: None, gate: None },
        foundation: 1,
        villagers: 0,
    };
    let dirt = [("minecraft:dirt_path", 4), ("minecraft:coarse_dirt", 1)];
    vec![
        theme("Plains", "village_plains", &dirt, "minecraft:cobblestone"),
        theme("Desert", "village_desert", &[("minecraft:smooth_sandstone", 3), ("minecraft:sandstone", 1)], "minecraft:cut_sandstone"),
        theme("Savanna", "village_savanna", &dirt, "minecraft:terracotta"),
        theme("Taiga", "village_taiga", &[("minecraft:dirt_path", 3), ("minecraft:podzol", 1), ("minecraft:cobblestone", 1)], "minecraft:mossy_cobblestone"),
        theme("Snowy", "village_snowy", &[("minecraft:dirt_path", 3), ("minecraft:packed_ice", 1)], "minecraft:stone_bricks"),
    ]
}

/// The default share of a type of building, by its name.
fn default_type(name: &str) -> (f32, Option<u32>, bool, bool) {
    match name {
        "house" => (0.55, None, false, false),
        "farm" | "stable" | "mill" => (0.12, None, false, true),
        "church" => (0.04, Some(1), true, false),
        "well" => (0.03, Some(1), true, false),
        "market" => (0.04, Some(1), true, false),
        "tavern" => (0.04, Some(1), false, false),
        "forge" => (0.05, Some(2), false, false),
        "watchtower" => (0.03, None, false, true),
        _ => (0.06, None, false, false),
    }
}

fn is_schematic(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| massif_objects::EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && is_schematic(p)).collect();
    out.sort();
    out
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    out.sort();
    out
}

/// The themes of the pack in folder `dir` (`<pack>/<theme>/<type>/*.schem`,
/// or `<pack>/<type>/*.schem` for a pack of one theme), its manifests read.
pub fn read_pack(dir: &Path) -> Result<Vec<Theme>, String> {
    let pack_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let manifest = |d: &Path| -> Option<serde_json::Value> { serde_json::from_slice(&std::fs::read(d.join("village.json")).ok()?).ok() };
    let top = manifest(dir);
    // A theme folder holds type folders of schematics.
    let is_theme = |d: &Path| subdirs(d).iter().any(|t| !files_in(t).is_empty());
    let theme_dirs: Vec<PathBuf> = if subdirs(dir).iter().any(|d| is_theme(d)) { subdirs(dir).into_iter().filter(|d| is_theme(d)).collect() } else if is_theme(dir) { vec![dir.to_owned()] } else { Vec::new() };
    if theme_dirs.is_empty() {
        return Err(format!("{}: no folder of schematics (theme/type/name.schem)", dir.display()));
    }
    let mut themes = Vec::new();
    for t in theme_dirs {
        let own = manifest(&t);
        // The theme's manifest over the pack's.
        let get = |key: &str| -> Option<serde_json::Value> { own.as_ref().and_then(|m| m.get(key)).or_else(|| top.as_ref().and_then(|m| m.get(key))).cloned() };
        let style = |v: Option<serde_json::Value>, width: i32| -> RoadStyle {
            let blocks: Vec<(String, u32)> = v.as_ref().and_then(|v| v.get("blocks")).and_then(|b| b.as_object()).map(|o| o.iter().map(|(k, w)| (crate::source::full_id(k), w.as_u64().unwrap_or(1).max(1) as u32)).collect()).unwrap_or_default();
            RoadStyle {
                width: v.as_ref().and_then(|v| v.get("width")).and_then(|w| w.as_i64()).map_or(width, |w| w.clamp(1, 9) as i32),
                blocks: if blocks.is_empty() { vec![("minecraft:dirt_path".into(), 1)] } else { blocks },
            }
        };
        let roads = get("roads");
        let types_json = get("types");
        let mut types = Vec::new();
        let mut tower = None;
        let mut gate = None;
        for d in subdirs(&t) {
            let name = d.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let files = files_in(&d);
            if files.is_empty() {
                continue;
            }
            if name == "walls" {
                let find = |stem: &str| files.iter().find(|f| f.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem))).cloned();
                tower = find("tower");
                gate = find("gate");
                continue;
            }
            let (share, max, centre, edge) = default_type(&name);
            let spec = types_json.as_ref().and_then(|j| j.get(&name));
            let f = |k: &str| spec.and_then(|s| s.get(k));
            types.push(BuildingType {
                share: f("share").and_then(|v| v.as_f64()).map_or(share, |v| v.max(0.0) as f32),
                max: f("max").and_then(|v| v.as_u64()).map(|v| v as u32).or(max),
                centre: f("centre").or_else(|| f("center")).and_then(|v| v.as_bool()).unwrap_or(centre),
                edge: f("edge").and_then(|v| v.as_bool()).unwrap_or(edge),
                near: f("near").and_then(|v| v.as_str()).map(str::to_owned),
                name,
                files,
            });
        }
        if types.is_empty() {
            continue;
        }
        let walls = get("walls");
        let w = |k: &str| walls.as_ref().and_then(|v| v.get(k));
        themes.push(Theme {
            name: own.as_ref().and_then(|m| m.get("name")).and_then(|n| n.as_str()).map_or_else(|| t.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), str::to_owned),
            pack: top.as_ref().and_then(|m| m.get("name")).and_then(|n| n.as_str()).map_or_else(|| pack_name.clone(), str::to_owned),
            vanilla: None,
            types,
            main: style(roads.as_ref().and_then(|r| r.get("main")).cloned(), 3),
            side: style(roads.as_ref().and_then(|r| r.get("side")).cloned(), 2),
            walls: WallStyle {
                height: w("height").and_then(|v| v.as_i64()).map_or(6, |v| v.clamp(3, 24) as i32),
                block: w("block").and_then(|v| v.as_str()).map_or_else(|| "minecraft:stone_bricks".to_owned(), crate::source::full_id),
                towers: w("towers").and_then(|v| v.as_i64()).map_or(40, |v| v.clamp(8, 400) as i32),
                tower,
                gate,
            },
            foundation: get("foundation").and_then(|v| v.as_i64()).map_or(3, |v| v.clamp(0, 16) as i32),
            villagers: get("villagers").and_then(|v| v.as_u64()).map_or(1, |v| v.min(8) as u32),
        });
    }
    if themes.is_empty() {
        return Err(format!("{}: no building in it", dir.display()));
    }
    Ok(themes)
}

// ------------------------------------------------------------- the brush --

/// What to build, and where.
pub struct Request<'a> {
    pub theme: &'a Theme,
    /// The selection: its bounds and whether a column is in it.
    pub rect: BlockRect,
    pub inside: &'a (dyn Fn(i32, i32) -> bool + Sync),
    pub seed: u64,
    pub walls: bool,
    /// How built up: 0.2 (sparse) to 1 (dense).
    pub density: f32,
    pub id: u32,
}

/// A building ready to lay: its blocks in its own box, where its door is.
struct Plan {
    size: [i32; 3],
    blocks: Vec<([i32; 3], String, Option<Compound>)>,
    /// The door's place (its lower half), or where it would be.
    door: [i32; 3],
}

/// A schematic of a pack, read as a plan.
fn plan_of(path: &Path, foundation: i32) -> Option<Plan> {
    let mut blocks = Vec::new();
    let size: [i32; 3];
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("nbt")) {
        let t = crate::template::Template::parse(&std::fs::read(path).ok()?).ok()?;
        size = t.size;
        let palette = t.palettes.first()?;
        for b in &t.blocks {
            let state = palette.get(b.state as usize)?;
            if state.starts_with("minecraft:structure_void") || state.starts_with("minecraft:jigsaw") || state.starts_with("minecraft:structure_block") {
                continue;
            }
            blocks.push((b.pos, state.clone(), b.nbt.clone()));
        }
    } else {
        let o = massif_objects::read(path).ok()?.object;
        size = o.size.map(|v| v as i32);
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let k = o.at(x as usize, y as usize, z as usize);
                    if k != 0 {
                        blocks.push(([x, y, z], o.palette[k as usize].clone(), None));
                    }
                }
            }
        }
    }
    // Its door: the lowest lower half of a door on its south side.
    let door = blocks
        .iter()
        .filter(|(_, s, _)| s.contains("_door[") && s.contains("half=lower"))
        .map(|(p, _, _)| *p)
        .max_by_key(|p| (p[2], -p[1]))
        .unwrap_or([size[0] / 2, foundation.min(size[1] - 1), size[2] - 1]);
    Some(Plan { size, blocks, door })
}

/// A cell's height (the first free block above the ground) and whether
/// water lies on it.
fn height(ground: &dyn Ground, x: i32, z: i32) -> (i32, bool) {
    let (f, s) = (ground.floor(x, z), ground.surface(x, z));
    (f, s > f)
}

/// A way across the land between two columns, on a grid of `step`
/// blocks: gentle slopes and dry land preferred, never out of the area, nor
/// through `blocked` cells. Its columns block by block, or none.
fn route(ground: &dyn Ground, inside: &dyn Fn(i32, i32) -> bool, blocked: &dyn Fn(i32, i32) -> bool, from: (i32, i32), to: (i32, i32), step: i32) -> Vec<(i32, i32)> {
    let snap = |p: (i32, i32)| (p.0.div_euclid(step), p.1.div_euclid(step));
    let (a, b) = (snap(from), snap(to));
    let centre = |c: (i32, i32)| (c.0 * step + step / 2, c.1 * step + step / 2);
    let mut cost: HashMap<(i32, i32), i64> = HashMap::new();
    let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    cost.insert(a, 0);
    heap.push(Reverse((0i64, a)));
    let span = ((a.0 - b.0).abs() + (a.1 - b.1).abs()) as usize;
    let mut visited = 0usize;
    while let Some(Reverse((_, c))) = heap.pop() {
        if c == b {
            break;
        }
        visited += 1;
        if visited > 40_000 + span * 400 {
            return Vec::new();
        }
        let here = cost[&c];
        let (cx, cz) = centre(c);
        let (h0, _) = height(ground, cx, cz);
        for d in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let n = (c.0 + d.0, c.1 + d.1);
            let (nx, nz) = centre(n);
            if n != b && (!inside(nx, nz) || blocked(nx, nz)) {
                continue;
            }
            let (h1, wet) = height(ground, nx, nz);
            let run = if d.0 != 0 && d.1 != 0 { 14 } else { 10 };
            let climb = (h1 - h0).abs() as i64;
            let step_cost = run + climb * climb * 6 + if wet { 60 } else { 0 };
            let total = here + step_cost;
            if cost.get(&n).is_none_or(|&k| total < k) {
                cost.insert(n, total);
                came.insert(n, c);
                let guess = (((n.0 - b.0).abs() + (n.1 - b.1).abs()) * 10) as i64;
                heap.push(Reverse((total + guess, n)));
            }
        }
    }
    if !came.contains_key(&b) && a != b {
        return Vec::new();
    }
    let mut cells = vec![b];
    while let Some(&p) = came.get(cells.last().expect("one")) {
        cells.push(p);
    }
    cells.reverse();
    // Block by block between the cells' middles, from `from` to `to`.
    let mut pts: Vec<(i32, i32)> = vec![from];
    pts.extend(cells.iter().skip(1).take(cells.len().saturating_sub(2)).map(|&c| centre(c)));
    pts.push(to);
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let n = (q.0 - p.0).abs().max((q.1 - p.1).abs()).max(1);
        for k in 0..n {
            let t = k as f32 / n as f32;
            let c = ((p.0 as f32 + (q.0 - p.0) as f32 * t).round() as i32, (p.1 as f32 + (q.1 - p.1) as f32 * t).round() as i32);
            if out.last() != Some(&c) {
                out.push(c);
            }
        }
    }
    out.push(to);
    out
}

/// Build the village `req` asks for on `ground` (`lib`: the game's files,
/// for the game's themes).
pub fn build(lib: Option<&Library>, ground: &dyn Ground, req: &Request) -> Result<Built, String> {
    let mut rng = Rng::new(req.seed ^ 0x7669_6c6c);
    let theme = req.theme;
    let r = req.rect;
    // The columns of the area, every 8 blocks.
    let samples: Vec<(i32, i32)> = (r.z0..r.z1).step_by(8).flat_map(|z| (r.x0..r.x1).step_by(8).map(move |x| (x, z))).filter(|&(x, z)| (req.inside)(x, z)).collect();
    if samples.len() < 9 {
        return Err("the selection is too small for a village: 24 × 24 blocks at least".into());
    }
    let area = samples.len() as i32 * 64;
    // Districts: one for about 240 × 240 blocks, their squares spread out.
    let districts = (area / (240 * 240)).clamp(1, 48) as usize;
    let mean = samples.iter().fold((0i64, 0i64), |a, p| (a.0 + p.0 as i64, a.1 + p.1 as i64));
    let mean = ((mean.0 / samples.len() as i64) as i32, (mean.1 / samples.len() as i64) as i32);
    let first = *samples.iter().min_by_key(|p| (p.0 - mean.0).pow(2) + (p.1 - mean.1).pow(2)).expect("samples");
    let mut centres = vec![first];
    while centres.len() < districts {
        let far = *samples.iter().max_by_key(|p| centres.iter().map(|c| (p.0 - c.0).pow(2) + (p.1 - c.1).pow(2)).min().unwrap_or(0)).expect("samples");
        centres.push(far);
    }
    // Each in the middle of its district (a few rounds of k-means), on a
    // column of the area.
    for _ in 0..6 {
        let mut sums = vec![(0i64, 0i64, 0i64); centres.len()];
        for p in &samples {
            let k = centres.iter().enumerate().min_by_key(|(_, c)| (p.0 - c.0).pow(2) + (p.1 - c.1).pow(2)).map_or(0, |(k, _)| k);
            sums[k] = (sums[k].0 + p.0 as i64, sums[k].1 + p.1 as i64, sums[k].2 + 1);
        }
        for (c, s) in centres.iter_mut().zip(&sums) {
            if s.2 > 0 {
                let m = ((s.0 / s.2) as i32, (s.1 / s.2) as i32);
                *c = *samples.iter().min_by_key(|p| (p.0 - m.0).pow(2) + (p.1 - m.1).pow(2)).expect("samples");
            }
        }
    }
    // What is taken: by buildings (and their margins), roads, walls.
    let mut taken: HashSet<(i32, i32)> = HashSet::new();
    let mut roads: HashMap<(i32, i32), bool> = HashMap::new(); // column → main road
    let mut road_lines: Vec<(Vec<(i32, i32)>, bool)> = Vec::new();
    let mut pieces_boxes: Vec<BBox> = Vec::new();
    let mut heights: HashMap<(i32, i32), i32> = HashMap::new();
    let mut unknown = Vec::new();

    // The game's villages, one in each district.
    let mut jigsaw = Vec::new();
    if let Some(kind_id) = theme.vanilla {
        let lib = lib.ok_or("the game's villages need the game: no Minecraft was found")?;
        let kind = vanilla::kind(kind_id).ok_or("an unknown village")?;
        let vanilla::Recipe::Jigsaw(id) = kind.recipe else { return Err("not a jigsaw village".into()) };
        let (spec, game_height, _, _) = vanilla::jigsaw_spec(lib, id).ok_or("the game's files have no villages")?;
        let inside = req.inside;
        for &c in &centres {
            let within = |b: &BBox| [(b.min[0], b.min[2]), (b.max[0], b.min[2]), (b.min[0], b.max[2]), (b.max[0], b.max[2])].iter().all(|&(x, z)| inside(x, z)) && !pieces_boxes.iter().any(|o: &BBox| o.overlaps(b));
            let mut sub = rng.fork();
            let pieces = crate::jigsaw::assemble_within(lib, &spec, [c.0, game_height, c.1], None, &mut sub, ground, &within);
            pieces_boxes.extend(pieces.iter().map(|p| p.bbox));
            jigsaw.push(pieces);
        }
        let all: Vec<_> = jigsaw.iter().flatten().cloned().collect();
        heights.extend(build::flatten_under(&all, ground, 4));
        // Their streets are roads; their buildings are taken.
        for p in &all {
            let street = p.element.projection() == crate::pool::Projection::TerrainMatching;
            for z in p.bbox.min[2]..=p.bbox.max[2] {
                for x in p.bbox.min[0]..=p.bbox.max[0] {
                    if street {
                        roads.entry((x, z)).or_insert(false);
                    } else {
                        taken.insert((x, z));
                    }
                }
            }
        }
    }

    // Main roads: the squares joined (each to the nearest already joined),
    // and two ways out of the village.
    let inside = req.inside;
    let blocked = |x: i32, z: i32| taken.contains(&(x, z));
    let mut joined = vec![centres[0]];
    for &c in centres.iter().skip(1) {
        let near = *joined.iter().min_by_key(|j| (j.0 - c.0).pow(2) + (j.1 - c.1).pow(2)).expect("one");
        let line = route(ground, inside, &blocked, c, near, 4);
        if !line.is_empty() {
            road_lines.push((line, true));
        }
        joined.push(c);
    }
    for k in 0..2 {
        // Toward the edge of the area, on either side.
        let dir = if k == 0 { (1, 0) } else { (-1, 0) };
        let mut p = centres[0];
        while inside(p.0 + dir.0 * 8, p.1 + dir.1 * 8) {
            p = (p.0 + dir.0 * 8, p.1 + dir.1 * 8);
        }
        if (p.0 - centres[0].0).abs() > 24 {
            let line = route(ground, inside, &blocked, centres[0], p, 4);
            if !line.is_empty() {
                road_lines.push((line, true));
            }
        }
    }
    // Squares and side roads: in a pack's village only (the game's have
    // their own streets).
    if theme.vanilla.is_none() {
        for &c in &centres {
            for dz in -4..=4 {
                for dx in -4..=4 {
                    if inside(c.0 + dx, c.1 + dz) {
                        roads.insert((c.0 + dx, c.1 + dz), false);
                    }
                }
            }
        }
        let mains: Vec<Vec<(i32, i32)>> = road_lines.iter().map(|l| l.0.clone()).collect();
        for line in &mains {
            let mut k = rng.between(14, 24) as usize;
            while k < line.len() {
                let p = line[k];
                let q = line[(k + 1).min(line.len() - 1)];
                let along_x = (q.0 - p.0).abs() >= (q.1 - p.1).abs();
                for side in [-1, 1] {
                    if rng.chance(0.25) {
                        continue;
                    }
                    let n = if along_x { (0, side) } else { (side, 0) };
                    let len = rng.between(20, 46);
                    let mut end = p;
                    for s in 1..=len {
                        let c = (p.0 + n.0 * s, p.1 + n.1 * s);
                        if !inside(c.0, c.1) || taken.contains(&c) || height(ground, c.0, c.1).1 {
                            break;
                        }
                        end = c;
                    }
                    if (end.0 - p.0).abs() + (end.1 - p.1).abs() >= 10 {
                        let mut l = Vec::new();
                        let steps = (end.0 - p.0).abs().max((end.1 - p.1).abs());
                        for s in 0..=steps {
                            l.push((p.0 + n.0 * s, p.1 + n.1 * s));
                        }
                        road_lines.push((l, false));
                    }
                }
                k += rng.between(26, 42) as usize;
            }
        }
    }
    // The roads' columns.
    for (line, main) in &road_lines {
        let w = if *main { theme.main.width } else { theme.side.width };
        let (lo, hi) = (-(w - 1) / 2, w / 2);
        for &(x, z) in line {
            for dz in lo..=hi {
                for dx in lo..=hi {
                    if inside(x + dx, z + dz) && !taken.contains(&(x + dx, z + dz)) {
                        let e = roads.entry((x + dx, z + dz)).or_insert(*main);
                        *e |= *main;
                    }
                }
            }
        }
    }

    // The walls' line: the hull of the area, three blocks in.
    let mut wall_cells: Vec<(i32, i32)> = Vec::new();
    let mut corners: Vec<(i32, i32)> = Vec::new();
    if req.walls {
        let hull = hull(&samples);
        let c = mean;
        let inset: Vec<(i32, i32)> = hull
            .iter()
            .map(|&(x, z)| {
                let (dx, dz) = ((c.0 - x) as f32, (c.1 - z) as f32);
                let d = (dx * dx + dz * dz).sqrt().max(1.0);
                ((x as f32 + dx / d * 3.0).round() as i32, (z as f32 + dz / d * 3.0).round() as i32)
            })
            .collect();
        for k in 0..inset.len() {
            let (p, q) = (inset[k], inset[(k + 1) % inset.len()]);
            let n = (q.0 - p.0).abs().max((q.1 - p.1).abs()).max(1);
            for s in 0..n {
                let t = s as f32 / n as f32;
                let cell = ((p.0 as f32 + (q.0 - p.0) as f32 * t).round() as i32, (p.1 as f32 + (q.1 - p.1) as f32 * t).round() as i32);
                if wall_cells.last() != Some(&cell) {
                    wall_cells.push(cell);
                }
            }
        }
        corners = inset;
        // Nothing stands within two blocks of it.
        for &(x, z) in &wall_cells {
            for dz in -2..=2 {
                for dx in -2..=2 {
                    taken.insert((x + dx, z + dz));
                }
            }
        }
    }

    // Buildings of a pack along the roads.
    let mut placed: Vec<(BBox, Plan, u8, String, i32)> = Vec::new();
    if theme.vanilla.is_none() {
        let mut plans: HashMap<PathBuf, Option<std::sync::Arc<Plan>>> = HashMap::new();
        let mut counts: HashMap<(usize, String), u32> = HashMap::new();
        let mut lots: Vec<((i32, i32), Dir, bool)> = Vec::new();
        for (line, main) in &road_lines {
            let w = if *main { theme.main.width } else { theme.side.width };
            let mut k = 2usize;
            while k + 1 < line.len() {
                let (p, q) = (line[k], line[k + 1]);
                let along_x = (q.0 - p.0).abs() >= (q.1 - p.1).abs();
                for side in [Dir::North, Dir::South, Dir::East, Dir::West] {
                    if along_x == matches!(side, Dir::North | Dir::South) {
                        let s = side.step();
                        let edge = (w - 1) / 2 + 1 + (side == Dir::South || side == Dir::East) as i32 * ((w + 1) % 2);
                        lots.push(((p.0 + s[0] * edge, p.1 + s[2] * edge), side, *main));
                    }
                }
                k += ((rng.between(9, 15) as f32) / req.density.clamp(0.2, 1.0)) as usize;
            }
        }
        // The middles first: the buildings that keep to them take their
        // places before the others fill the streets.
        lots.sort_by_key(|(p, _, _)| centres.iter().map(|c| (p.0 - c.0).pow(2) + (p.1 - c.1).pow(2)).min().unwrap_or(0));
        let total_share: f32 = theme.types.iter().map(|t| t.share).sum::<f32>().max(0.001);
        for (door_at, side, _) in lots {
            let district = centres.iter().enumerate().min_by_key(|(_, c)| (door_at.0 - c.0).pow(2) + (door_at.1 - c.1).pow(2)).map(|(i, _)| i).unwrap_or(0);
            let dc = centres[district];
            let from_centre = (((door_at.0 - dc.0).pow(2) + (door_at.1 - dc.1).pow(2)) as f32).sqrt();
            // A type, by share, among those this lot suits.
            let mut weights: Vec<(usize, f32)> = Vec::new();
            for (i, t) in theme.types.iter().enumerate() {
                let n = counts.get(&(district, t.name.clone())).copied().unwrap_or(0);
                if t.max.is_some_and(|m| n >= m) || (t.centre && from_centre > 48.0) || (t.edge && from_centre < 40.0) {
                    continue;
                }
                if let Some(near) = &t.near
                    && !placed.iter().any(|p| &p.3 == near)
                {
                    continue;
                }
                // Those that keep to the middle come first there.
                let boost = if t.centre && n == 0 { 20.0 } else { 1.0 };
                weights.push((i, t.share / total_share * boost));
            }
            let sum: f32 = weights.iter().map(|w| w.1).sum();
            if sum <= 0.0 {
                continue;
            }
            let mut pick = rng.float() * sum;
            let ti = weights.iter().find(|w| {
                if pick < w.1 {
                    true
                } else {
                    pick -= w.1;
                    false
                }
            }).map_or(weights[0].0, |w| w.0);
            let t = &theme.types[ti];
            let Some(file) = rng.pick(&t.files).cloned() else { continue };
            let plan = plans.entry(file.clone()).or_insert_with(|| plan_of(&file, theme.foundation).map(std::sync::Arc::new)).clone();
            let Some(plan) = plan else {
                unknown.push(format!("schematic {} (could not be read)", file.display()));
                continue;
            };
            // Its door toward the road: south in the schematic.
            let turn = match side {
                Dir::North => 0,
                Dir::East => 1,
                Dir::South => 2,
                _ => 3,
            };
            let (floor, wet) = height(ground, door_at.0, door_at.1);
            if wet {
                continue;
            }
            let (dx, dz) = crate::geom::turn_xz(plan.door[0], plan.door[2], turn);
            let origin = [door_at.0 - dx, floor - plan.door[1], door_at.1 - dz];
            let b = BBox::of_template(plan.size, origin, turn);
            // On free land of the area, apart from the others.
            let margin = rng.between(1, 3);
            let mut ok = true;
            let (mut lo, mut hi) = (i32::MAX, i32::MIN);
            'cells: for z in b.min[2] - margin..=b.max[2] + margin {
                for x in b.min[0] - margin..=b.max[0] + margin {
                    let inner = x >= b.min[0] && x <= b.max[0] && z >= b.min[2] && z <= b.max[2];
                    if taken.contains(&(x, z)) || (inner && (!inside(x, z) || roads.contains_key(&(x, z)))) {
                        ok = false;
                        break 'cells;
                    }
                    if inner {
                        let (h, wet) = height(ground, x, z);
                        if wet {
                            ok = false;
                            break 'cells;
                        }
                        (lo, hi) = (lo.min(h), hi.max(h));
                    }
                }
            }
            if !ok || hi - lo > 10 {
                continue;
            }
            for z in b.min[2] - margin..=b.max[2] + margin {
                for x in b.min[0] - margin..=b.max[0] + margin {
                    taken.insert((x, z));
                }
            }
            *counts.entry((district, t.name.clone())).or_default() += 1;
            let plan = std::sync::Arc::try_unwrap(plan).unwrap_or_else(|a| Plan { size: a.size, blocks: a.blocks.clone(), door: a.door });
            placed.push((b, plan, turn, t.name.clone(), floor));
        }
        // The land under each, to its floor.
        heights.extend(level_under(placed.iter().map(|p| (p.0, p.4)), ground, 4));
    }

    // Lay it all.
    let adapted = Adapted { inner: ground, heights };
    let mut b = Builder::new(&adapted, req.seed);
    // The game's villages.
    for pieces in &jigsaw {
        let centre = build::center_of(pieces);
        for p in pieces {
            build::place_piece(lib.expect("checked"), &mut b, p, centre);
        }
    }
    // Roads on the land, a block of their style in its top; planks over
    // water.
    let mut cols: Vec<(&(i32, i32), &bool)> = roads.iter().collect();
    cols.sort();
    for (&(x, z), &main) in cols {
        if b.get([x, adapted.floor(x, z) - 1, z]).is_some() {
            continue;
        }
        let style = if main { &theme.main } else { &theme.side };
        let (floor, wet) = height(&adapted, x, z);
        if wet {
            b.set([x, adapted.surface(x, z) - 1, z], "minecraft:oak_planks", None);
            continue;
        }
        let mut h = Rng::at(req.seed, [x, 0, z]);
        let total: u32 = style.blocks.iter().map(|w| w.1).sum::<u32>().max(1);
        let mut pick = h.below(total);
        let state = style.blocks.iter().find(|w| {
            if pick < w.1 {
                true
            } else {
                pick -= w.1;
                false
            }
        }).map_or("minecraft:dirt_path", |w| w.0.as_str());
        b.set([x, floor - 1, z], state, None);
        // Clear what grows over it.
        b.set([x, floor, z], "minecraft:air", None);
    }
    // A pack's buildings.
    for (bbox, plan, turn, name, floor) in &placed {
        let origin = [bbox_origin(bbox, plan.size, *turn)[0], floor - plan.door[1], bbox_origin(bbox, plan.size, *turn)[2]];
        for (p, state, data) in &plan.blocks {
            let w = place(*p, origin, *turn);
            let state = massif_core::objects::transform_state(state, *turn, false);
            b.set(w, &state, data.clone());
        }
        // Air inside, over the levelled floor: what grew there goes.
        // Villagers in the houses, inside the door.
        if !matches!(name.as_str(), "farm" | "well" | "market" | "watchtower" | "walls") {
            let inward = Dir::North.turned(*turn).step();
            let d = place(plan.door, origin, *turn);
            for k in 0..theme.villagers {
                let at = [d[0] + inward[0] * (1 + k as i32 % 2), d[1], d[2] + inward[2] * (1 + k as i32 % 2)];
                let mut v = Compound::new();
                v.insert("id".into(), Value::String("minecraft:villager".into()));
                v.insert("PersistenceRequired".into(), Value::Byte(1));
                b.add_entity([at[0] as f64 + 0.5, at[1] as f64, at[2] as f64 + 0.5], v);
            }
        }
        b.pieces.push(Piece { min: bbox.min, max: bbox.max, template: name.clone(), origin, turn: *turn, depth: 0, ground_delta: plan.door[1], rigid: true, processors: String::new() });
    }
    // Walls, towers and gates.
    if req.walls && !wall_cells.is_empty() {
        lay_walls(&mut b, &adapted, theme, &wall_cells, &corners, &roads, &mut rng);
    }
    let name = if theme.pack.is_empty() { format!("{} village", theme.name) } else { theme.name.clone() };
    let mut s = b.finish(req.id, &name, "", true, lib.map_or(0, |l| l.data_version()));
    if s.sections.is_empty() {
        return Err("nothing could be built there: too steep, too wet, or too small".into());
    }
    s.kind = String::new();
    if let Some(l) = lib {
        unknown.extend(l.unknowns());
    }
    Ok(Built { structure: s, heights: adapted.heights.clone(), unknown })
}

/// The corner a template's box has its origin at, turned `turn`.
fn bbox_origin(b: &BBox, size: [i32; 3], turn: u8) -> [i32; 3] {
    // The box of a template at the origin, turned, and where its origin
    // lies within it.
    let at0 = BBox::of_template(size, [0, 0, 0], turn);
    [b.min[0] - at0.min[0], b.min[1] - at0.min[1], b.min[2] - at0.min[2]]
}

/// Level the land under each box to its floor (the first free block over
/// it), with a slope of `margin` blocks to the land around.
fn level_under(boxes: impl Iterator<Item = (BBox, i32)>, ground: &dyn Ground, margin: i32) -> HashMap<(i32, i32), i32> {
    let mut best: HashMap<(i32, i32), (f32, i32)> = HashMap::new();
    for (b, floor) in boxes {
        for z in b.min[2] - margin..=b.max[2] + margin {
            for x in b.min[0] - margin..=b.max[0] + margin {
                let dx = (b.min[0] - x).max(x - b.max[0]).max(0);
                let dz = (b.min[2] - z).max(z - b.max[2]).max(0);
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                let w = if d <= 0.0 { 1.0 } else { 1.0 - d / (margin as f32 + 1.0) };
                if w <= 0.0 {
                    continue;
                }
                let e = best.entry((x, z)).or_insert((0.0, floor));
                if w > e.0 {
                    *e = (w, floor);
                }
            }
        }
    }
    best.into_iter()
        .filter_map(|((x, z), (w, floor))| {
            let here = ground.floor(x, z);
            let h = if w >= 1.0 { floor } else { (here as f32 + (floor - here) as f32 * w).round() as i32 };
            (h != here).then_some(((x, z), h))
        })
        .collect()
}

/// The convex hull of `points`, counterclockwise.
fn hull(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut p: Vec<(i32, i32)> = points.to_vec();
    p.sort();
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: (i32, i32), a: (i32, i32), b: (i32, i32)| (a.0 - o.0) as i64 * (b.1 - o.1) as i64 - (a.1 - o.1) as i64 * (b.0 - o.0) as i64;
    let mut lower: Vec<(i32, i32)> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<(i32, i32)> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The walls along `cells`, on the land: crenellated, towers at the
/// corners and every few blocks, a gate where a road crosses.
fn lay_walls(b: &mut Builder, ground: &dyn Ground, theme: &Theme, cells: &[(i32, i32)], corners: &[(i32, i32)], roads: &HashMap<(i32, i32), bool>, rng: &mut Rng) {
    let style = &theme.walls;
    let h = style.height;
    let block = style.block.as_str();
    let tower_plan = style.tower.as_ref().and_then(|p| plan_of(p, 0));
    let gate_plan = style.gate.as_ref().and_then(|p| plan_of(p, 0));
    let mut towers: Vec<(i32, i32)> = corners.to_vec();
    for (k, c) in cells.iter().enumerate() {
        if k > 0 && k % style.towers.max(8) as usize == 0 {
            towers.push(*c);
        }
    }
    let near_tower = |x: i32, z: i32| towers.iter().any(|t| (t.0 - x).abs() <= 2 && (t.1 - z).abs() <= 2);
    let mut gates: Vec<(i32, i32)> = Vec::new();
    for (k, &(x, z)) in cells.iter().enumerate() {
        let base = ground.floor(x, z);
        let on_road = (-1..=1).any(|d| roads.contains_key(&(x + d, z)) || roads.contains_key(&(x, z + d)));
        if on_road {
            // A gate: the road passes under an arch.
            if gates.last().is_none_or(|g| (g.0 - x).abs() + (g.1 - z).abs() > 6) {
                gates.push((x, z));
            }
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
    for &(x, z) in &towers {
        let base = ground.floor(x, z);
        match &tower_plan {
            Some(plan) => {
                let origin = [x - plan.size[0] / 2, base - plan.door[1].min(1), z - plan.size[2] / 2];
                for (p, state, data) in &plan.blocks {
                    b.set(place(*p, origin, 0), state, data.clone());
                }
            }
            None => {
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
        }
    }
    if let Some(plan) = &gate_plan {
        for &(x, z) in &gates {
            let base = ground.floor(x, z);
            let origin = [x - plan.size[0] / 2, base - plan.door[1].min(1), z - plan.size[2] / 2];
            for (p, state, data) in &plan.blocks {
                b.set(place(*p, origin, 0), state, data.clone());
            }
        }
    }
    let _ = rng;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;

    fn write_schem(dir: &Path, rel: &str, size: [i32; 3], door: [i32; 3]) {
        // A box of planks with a door: written as a structure `.nbt`.
        let mut blocks: Vec<([i32; 3], &str, Option<Compound>)> = Vec::new();
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let wall = x == 0 || x == size[0] - 1 || z == 0 || z == size[2] - 1 || y == 0 || y == size[1] - 1;
                    if [x, y, z] == door {
                        blocks.push(([x, y, z], "minecraft:oak_door[facing=south,half=lower,hinge=left,open=false,powered=false]", None));
                    } else if [x, y, z] == [door[0], door[1] + 1, door[2]] {
                        blocks.push(([x, y, z], "minecraft:oak_door[facing=south,half=upper,hinge=left,open=false,powered=false]", None));
                    } else if wall {
                        blocks.push(([x, y, z], if y < door[1] { "minecraft:cobblestone" } else { "minecraft:oak_planks" }, None));
                    }
                }
            }
        }
        let bytes = crate::testing::template_bytes(size, &blocks, &[]);
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn pack() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("massif-village-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_schem(&dir, "Countryside/house/cottage.nbt", [7, 7, 7], [3, 3, 6]);
        write_schem(&dir, "Countryside/house/longhouse.nbt", [11, 6, 7], [5, 3, 6]);
        write_schem(&dir, "Countryside/church/chapel.nbt", [9, 12, 13], [4, 3, 12]);
        write_schem(&dir, "Countryside/well/well.nbt", [5, 5, 5], [2, 3, 4]);
        std::fs::write(dir.join("Countryside/village.json"), br#"{"roads": {"main": {"width": 3, "blocks": {"minecraft:gravel": 1}}}, "types": {"house": {"share": 0.8}}}"#).unwrap();
        dir
    }

    #[test]
    fn packs_are_read_and_villages_built_on_their_roads() {
        let dir = pack();
        let themes = read_pack(&dir).unwrap();
        assert_eq!(themes.len(), 1);
        let t = &themes[0];
        assert_eq!(t.name, "Countryside");
        assert_eq!(t.types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["church", "house", "well"]);
        assert_eq!(t.main.blocks, vec![("minecraft:gravel".to_owned(), 1)]);
        assert_eq!(t.types.iter().find(|t| t.name == "church").unwrap().max, Some(1));
        let ground = Flat { height: 64, water: None };
        let rect = BlockRect::new(-100, -100, 100, 100);
        let inside = |x: i32, z: i32| rect.contains(x, z);
        let req = Request { theme: t, rect, inside: &inside, seed: 3, walls: true, density: 0.8, id: 1 };
        let built = build(None, &ground, &req).unwrap();
        let s = &built.structure;
        let houses = s.pieces.iter().filter(|p| p.template == "house").count();
        assert!(houses >= 4, "{} buildings: {:?}", s.pieces.len(), s.pieces.iter().map(|p| &p.template).collect::<Vec<_>>());
        assert!(s.pieces.iter().filter(|p| p.template == "church").count() <= 1);
        // Every building inside the area, none over another.
        for (i, a) in s.pieces.iter().enumerate() {
            assert!(rect.contains(a.min[0], a.min[2]) && rect.contains(a.max[0], a.max[2]));
            for b2 in &s.pieces[i + 1..] {
                let (ba, bb) = (BBox { min: a.min, max: a.max }, BBox { min: b2.min, max: b2.max });
                assert!(!ba.overlaps(&bb), "{a:?} {b2:?}");
            }
        }
        // Doors stand at the land's level, by a road: the main ones of the
        // manifest's gravel, the side ones of the default path.
        assert!(s.palette.iter().any(|p| p == "minecraft:gravel"), "roads of the manifest's gravel");
        let road: Vec<u16> = s.palette.iter().enumerate().filter(|(_, p)| *p == "minecraft:gravel" || *p == "minecraft:dirt_path").map(|(k, _)| k as u16).collect();
        let mut column = Vec::new();
        let mut by_road = 0;
        for p in &s.pieces {
            let d = place([0, 0, 0], p.origin, p.turn);
            let _ = d;
            // Look round the box for gravel at y 63.
            let found = (p.min[0] - 2..=p.max[0] + 2).any(|x| [p.min[2] - 1, p.max[2] + 1].iter().any(|&z| {
                s.column(x, z, &mut column);
                column.iter().any(|&(y, k)| y == 63 && road.contains(&k))
            })) || (p.min[2] - 2..=p.max[2] + 2).any(|z| [p.min[0] - 1, p.max[0] + 1].iter().any(|&x| {
                s.column(x, z, &mut column);
                column.iter().any(|&(y, k)| y == 63 && road.contains(&k))
            }));
            by_road += found as usize;
        }
        assert!(by_road * 10 >= s.pieces.len() * 8, "{by_road} of {} by a road", s.pieces.len());
        // Villagers in the houses; walls round it.
        assert!(!s.entities.is_empty());
        assert!(s.palette.iter().any(|p| p == "minecraft:stone_bricks"), "walls");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_game_s_villages_fill_a_large_selection_in_districts() {
        let lib = crate::testing::village();
        let mut theme = built_in().remove(0);
        theme.vanilla = Some("village_plains");
        let ground = Flat { height: 64, water: None };
        let rect = BlockRect::new(0, 0, 600, 600);
        let inside = |x: i32, z: i32| rect.contains(x, z);
        let req = Request { theme: &theme, rect, inside: &inside, seed: 9, walls: false, density: 1.0, id: 1 };
        let built = build(Some(&lib), &ground, &req).unwrap();
        let s = &built.structure;
        // Six districts (600² / 240²), each a village round its square.
        let squares = s.pieces.iter().filter(|p| p.template == "t:square").count();
        assert!(squares >= 4, "{squares} districts");
        assert!(s.pieces.iter().all(|p| rect.contains(p.min[0], p.min[2]) && rect.contains(p.max[0], p.max[2])));
    }
}
