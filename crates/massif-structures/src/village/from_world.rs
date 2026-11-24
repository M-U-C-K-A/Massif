//! A village pack cut from a world save where a builder laid out their
//! builds side by side on flat ground (a "build bundle"): each build — the
//! blocks above the ground that touch, apart from the others by a gap — is
//! cut out with what lies under it down to its lowest block, sorted by what
//! it is (a house by its footprint, a prop if small, a wall if long and
//! thin, trees and rocks set aside), its entrance found by its doors and
//! turned to the south, and written as a structure `.nbt`.
//!
//! Worlds of the versions before 1.13 (numbered blocks) and after are both
//! read. The pack is written to the player's disk only: a bundle's licence
//! is the builder's.

use super::Side;
use super::buildings::turn_for;
use crate::geom::{BBox, place};
use crate::nbt::Compound;
use fastnbt::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The ground's own blocks: what a build stands on, never part of it.
fn is_ground(state: &str) -> bool {
    let n = state.split('[').next().unwrap_or(state);
    matches!(n, "minecraft:grass_block" | "minecraft:dirt" | "minecraft:bedrock" | "minecraft:coarse_dirt" | "minecraft:podzol" | "minecraft:sand" | "minecraft:water" | "minecraft:short_grass" | "minecraft:grass" | "minecraft:tall_grass" | "minecraft:snow")
}

fn is_air(state: &str) -> bool {
    state.starts_with("minecraft:air") || state.starts_with("minecraft:cave_air") || state.starts_with("minecraft:void_air")
}

/// The blocks of a chunk of the versions before 1.13 (`Blocks`, `Data`,
/// numbered), as states, by their place in the world.
fn old_chunk(level: &Compound, out: &mut impl FnMut([i32; 3], String)) {
    let (Some(Value::Int(cx)), Some(Value::Int(cz))) = (level.get("xPos"), level.get("zPos")) else { return };
    let Some(Value::List(sections)) = level.get("Sections") else { return };
    for s in sections {
        let Value::Compound(s) = s else { continue };
        let y0 = match s.get("Y") {
            Some(Value::Byte(y)) => *y as i32 * 16,
            _ => continue,
        };
        let (Some(Value::ByteArray(blocks)), Some(Value::ByteArray(data))) = (s.get("Blocks"), s.get("Data")) else { continue };
        let add = match s.get("Add") {
            Some(Value::ByteArray(a)) => Some(a),
            _ => None,
        };
        for i in 0..4096usize {
            let nib = |a: &[i8], i: usize| ((a[i / 2] as u8) >> ((i & 1) * 4)) & 15;
            let mut id = blocks[i] as u8 as u16;
            if let Some(a) = add {
                id |= (nib(a, i) as u16) << 8;
            }
            if id == 0 {
                continue;
            }
            let d = nib(data, i);
            let Some(state) = massif_objects::legacy::state(id, d) else { continue };
            let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
            out([cx * 16 + x, y0 + y, cz * 16 + z], state);
        }
    }
}

/// The blocks of a chunk of 1.13 or later, through fastanvil.
fn new_chunk(bytes: &[u8], cx: i32, cz: i32, out: &mut impl FnMut([i32; 3], String)) {
    use fastanvil::Chunk;
    let Ok(c) = fastanvil::JavaChunk::from_bytes(bytes) else { return };
    let r = c.y_range();
    for y in r {
        for z in 0..16 {
            for x in 0..16 {
                if let Some(b) = c.block(x, y, z) {
                    let e = b.encoded_description();
                    let (name, props) = e.split_once('|').unwrap_or((e, ""));
                    if is_air(name) {
                        continue;
                    }
                    let state = if props.is_empty() { name.to_owned() } else { format!("{name}[{props}]") };
                    out([cx * 16 + x as i32, y as i32, cz * 16 + z as i32], state);
                }
            }
        }
    }
}

/// Every block of the world's region files that is not air.
fn read_world(world: &Path) -> Result<HashMap<[i32; 3], String>, String> {
    let dir = world.join("region");
    let mut blocks = HashMap::new();
    let mut put = |p: [i32; 3], s: String| {
        blocks.insert(p, s);
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "mca") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    files.sort();
    for f in files {
        let Ok(file) = std::fs::File::open(&f) else { continue };
        let Ok(mut region) = fastanvil::Region::from_stream(file) else { continue };
        let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut parts = name.split('.').skip(1).map(|v| v.parse::<i32>().unwrap_or(0));
        let (rx, rz) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        for z in 0..32usize {
            for x in 0..32usize {
                let Ok(Some(bytes)) = region.read_chunk(x, z) else { continue };
                let Ok(Value::Compound(root)) = fastnbt::from_bytes::<Value>(&bytes) else { continue };
                match root.get("Level") {
                    Some(Value::Compound(level)) if matches!(level.get("Sections"), Some(Value::List(l)) if l.iter().any(|s| matches!(s, Value::Compound(c) if c.contains_key("Blocks")))) => old_chunk(level, &mut put),
                    _ => new_chunk(&bytes, rx * 32 + x as i32, rz * 32 + z as i32, &mut put),
                }
            }
        }
    }
    // The upper halves of old doors face as their lower halves do.
    let uppers: Vec<[i32; 3]> = blocks.iter().filter(|(_, s)| s.contains("_door[") && s.contains("half=upper")).map(|(p, _)| *p).collect();
    for p in uppers {
        let below = blocks.get(&[p[0], p[1] - 1, p[2]]).cloned();
        if let (Some(below), Some(up)) = (below, blocks.get_mut(&p))
            && let Some(f) = below.split("facing=").nth(1).and_then(|r| r.split([',', ']']).next())
        {
            *up = up.replace("facing=north", &format!("facing={f}"));
        }
    }
    Ok(blocks)
}

/// A doorway on side `side` of a build: an opening two blocks high in the
/// wall that side, with a block over it. Its place (the opening's lower
/// block, in the world) and how many such openings that side has.
fn doorway(occ: &HashSet<[i32; 3]>, min: [i32; 3], max: [i32; 3], side: Side) -> Option<([i32; 3], usize)> {
    // Along the side (u), and in from it (v): the wall is the row nearest
    // the side that is most built at a man's height.
    let (us, vs): (Vec<i32>, Vec<i32>) = match side {
        Side::South => ((min[0]..=max[0]).collect(), (min[2]..=max[2]).rev().collect()),
        Side::North => ((min[0]..=max[0]).collect(), (min[2]..=max[2]).collect()),
        Side::East => ((min[2]..=max[2]).collect(), (min[0]..=max[0]).rev().collect()),
        Side::West => ((min[2]..=max[2]).collect(), (min[0]..=max[0]).collect()),
    };
    let at = |u: i32, v: i32, y: i32| -> bool {
        match side {
            Side::South | Side::North => occ.contains(&[u, y, v]),
            _ => occ.contains(&[v, y, u]),
        }
    };
    let mut best: Option<([i32; 3], usize)> = None;
    for &floor in &[min[1], min[1] + 1] {
        let wall = vs.iter().take(5).copied().max_by_key(|&v| us.iter().filter(|&&u| at(u, v, floor + 1)).count())?;
        if us.iter().filter(|&&u| at(u, wall, floor + 1)).count() * 2 < us.len() {
            continue;
        }
        let mut found = Vec::new();
        for (k, &u) in us.iter().enumerate().skip(1).take(us.len().saturating_sub(2)) {
            let open = !at(u, wall, floor) || !at(u, wall, floor + 1);
            if open && !at(u, wall, floor + 1) && !at(u, wall, floor + 2) && at(u, wall, floor + 3) && (at(us[k - 1], wall, floor + 1) || at(us[k + 1], wall, floor + 1)) {
                found.push(u);
            }
        }
        if let Some(&u) = found.get(found.len() / 2) {
            let p = match side {
                Side::South | Side::North => [u, floor + 1, wall],
                _ => [wall, floor + 1, u],
            };
            if best.as_ref().is_none_or(|b| found.len() > b.1) {
                best = Some((p, found.len()));
            }
        }
    }
    best
}

/// What a build is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sort {
    House,
    Manor,
    Hall,
    Prop,
    Wall,
    Tree,
    Rock,
    Other,
}

impl Sort {
    /// The pack's folder for it; `None` for what is set aside.
    fn folder(self) -> Option<&'static str> {
        Some(match self {
            Sort::House => "house",
            Sort::Manor => "manor",
            Sort::Hall => "church",
            Sort::Prop => "props",
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Sort::House => "house",
            Sort::Manor => "manor",
            Sort::Hall => "hall",
            Sort::Prop => "prop",
            Sort::Wall => "wall",
            Sort::Tree => "tree",
            Sort::Rock => "rock",
            Sort::Other => "other",
        }
    }
}

/// A build cut out.
#[derive(Debug, Clone)]
pub struct Build {
    /// Its box in the world, the ends included.
    pub min: [i32; 3],
    pub max: [i32; 3],
    pub sort: Sort,
    /// The side its doors are on, when it has some.
    pub entrance: Option<Side>,
    pub file: String,
    pub blocks: usize,
    /// Its three commonest blocks.
    pub made_of: Vec<String>,
}

/// What cutting a world found.
#[derive(Debug, Default)]
pub struct Cut {
    pub root: PathBuf,
    pub ground: i32,
    pub builds: Vec<Build>,
}

/// The blocks of builds, column by column.
type ByColumn<'a> = HashMap<(i32, i32), Vec<([i32; 3], &'a String)>>;

/// Cut the builds of `world` into a pack named `name` in `dir`
/// (`dir/<name>/…`, what is set aside in `dir/<name> (set aside)/…`).
pub fn write_pack(world: &Path, name: &str, dir: &Path) -> Result<Cut, String> {
    let all = read_world(world)?;
    if all.is_empty() {
        return Err(format!("{}: no blocks in its region files", world.display()));
    }
    // The ground's level: the highest layer that is (almost) full — a
    // flat world's ground, whatever it is made of (grass, a floor of
    // terracotta, paths). Builds are what stands on it.
    let mut per_y: HashMap<i32, usize> = HashMap::new();
    for p in all.keys() {
        *per_y.entry(p[1]).or_default() += 1;
    }
    let fullest = per_y.values().copied().max().unwrap_or(0);
    let ground = per_y.iter().filter(|(_, n)| **n * 10 >= fullest * 9).map(|(y, _)| *y).max().unwrap_or(0);
    let mine: HashMap<[i32; 3], &String> = all.iter().filter(|(p, s)| p[1] > ground && !is_ground(s)).map(|(p, s)| (*p, s)).collect();
    // Columns, joined where they touch (diagonals too).
    let cols: HashSet<(i32, i32)> = mine.keys().map(|p| (p[0], p[2])).collect();
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut groups: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut sorted: Vec<&(i32, i32)> = cols.iter().collect();
    sorted.sort();
    for &&c in &sorted {
        if !seen.insert(c) {
            continue;
        }
        let mut g = vec![c];
        let mut k = 0;
        while k < g.len() {
            let (x, z) = g[k];
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = (x + dx, z + dz);
                    if cols.contains(&n) && seen.insert(n) {
                        g.push(n);
                    }
                }
            }
            k += 1;
        }
        groups.push(g);
    }
    let by_col: ByColumn = {
        let mut m: ByColumn = HashMap::new();
        for (p, s) in &mine {
            m.entry((p[0], p[2])).or_default().push((*p, s));
        }
        m
    };
    let root = dir.join(name);
    let aside = dir.join(format!("{name} (set aside)"));
    let mut cut = Cut { root: root.clone(), ground, builds: Vec::new() };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    // The doorways of builds without doors, by type and file: the
    // manifest names them.
    let mut openings: BTreeMap<String, BTreeMap<String, [i32; 3]>> = BTreeMap::new();
    for g in groups {
        let blocks: Vec<([i32; 3], &String)> = g.iter().flat_map(|c| by_col[c].iter().copied()).collect();
        if blocks.len() < 2 {
            continue;
        }
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for (p, _) in &blocks {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        let size = [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1];
        let share = |f: &dyn Fn(&str) -> bool| blocks.iter().filter(|(_, s)| f(s)).count() as f32 / blocks.len() as f32;
        let woody = share(&|s| s.contains("_log") || s.contains("_leaves") || s.contains("_wood["));
        // What builders build with: wood worked, glass, stairs, slabs,
        // fences, doors. Rocks and ruins have little of it.
        let built = share(&|s| ["_planks", "_stairs", "_slab", "_fence", "glass", "_door", "_trapdoor", "stripped_", "_wall["].iter().any(|k| s.contains(k)));
        let rocky = share(&|s| ["stone", "andesite", "diorite", "granite", "cobblestone", "mossy_cobblestone", "gravel", "coal_ore", "iron_ore", "gold_ore", "diamond_ore", "emerald_ore", "lapis_ore", "redstone_ore", "dirt", "coarse_dirt", "grass_block"].iter().any(|r| s.starts_with(&format!("minecraft:{r}"))));
        // Doors: on which side of the build they open.
        let doors: Vec<[i32; 3]> = blocks.iter().filter(|(_, s)| s.contains("_door[") && s.contains("half=lower")).map(|(p, _)| *p).collect();
        let entrance = (!doors.is_empty()).then(|| {
            // The side nearest the doors, the doors' own facing breaking
            // ties.
            let mut best = (i32::MAX, Side::South);
            for side in Side::ALL {
                let d: i32 = doors.iter().map(|p| match side {
                    Side::North => p[2] - min[2],
                    Side::South => max[2] - p[2],
                    Side::West => p[0] - min[0],
                    Side::East => max[0] - p[0],
                }).min().unwrap_or(i32::MAX);
                if d < best.0 {
                    best = (d, side);
                }
            }
            best.1
        });
        // Without doors, a doorway: the side with an opening in its wall
        // (the longer sides first, as a house's front is).
        let occ: HashSet<[i32; 3]> = blocks.iter().filter(|(_, s)| !s.contains("_door[")).map(|(p, _)| *p).collect();
        let mut opening: Option<[i32; 3]> = None;
        let entrance = entrance.or_else(|| {
            if woody > 0.5 || size[1] < 4 {
                return None;
            }
            let mut sides = Side::ALL.to_vec();
            sides.sort_by_key(|s| -(if matches!(s, Side::North | Side::South) { size[0] } else { size[2] }));
            let found = sides.into_iter().filter_map(|s| doorway(&occ, min, max, s).map(|d| (s, d))).max_by_key(|(_, d)| d.1)?;
            opening = Some(found.1.0);
            Some(found.0)
        });
        let has_entrance = entrance.is_some();
        let (w, d) = (size[0].max(size[2]), size[0].min(size[2]));
        let sort = if woody > 0.5 && doors.is_empty() {
            Sort::Tree
        } else if doors.is_empty() && (rocky > 0.6 || (rocky > 0.4 && built < 0.05)) {
            Sort::Rock
        } else if !has_entrance && w >= 3 * d.max(1) && w >= 6 {
            Sort::Wall
        } else if w <= 5 && (doors.is_empty() || size[1] <= 4) {
            Sort::Prop
        } else if !has_entrance || w < 7 || d < 5 || size[1] < 5 {
            // Too small or too low for a house: an interior's piece, a
            // stall, a ruin.
            Sort::Other
        } else if w <= 16 {
            Sort::House
        } else if w <= 24 {
            Sort::Manor
        } else {
            Sort::Hall
        };
        // Turned so that its entrance faces south.
        let turn = entrance.map_or(0, |e| turn_for(e, Side::South));
        let b0 = BBox::of_template(size, [0, 0, 0], turn);
        let origin = [-b0.min[0], -b0.min[1], -b0.min[2]];
        let tsize = [b0.max[0] - b0.min[0] + 1, size[1], b0.max[2] - b0.min[2] + 1];
        let local: Vec<([i32; 3], String, Option<Compound>)> = blocks.iter().map(|(p, s)| (place([p[0] - min[0], p[1] - min[1], p[2] - min[2]], origin, turn), massif_core::objects::transform_state(s, turn, false), None)).collect();
        let class = if w <= 11 { "small" } else if w <= 16 { "medium" } else if w <= 24 { "large" } else { "huge" };
        let stem = match sort {
            Sort::House => class.to_owned(),
            Sort::Prop if blocks.iter().any(|(_, s)| s.contains("torch") || s.contains("lantern") || s.contains("glowstone")) => "lamppost".into(),
            Sort::Prop if blocks.iter().any(|(_, s)| s.contains("hay_block")) => "hay_bales".into(),
            Sort::Prop => "decor".into(),
            s => s.name().to_owned(),
        };
        let folder = sort.folder().map_or_else(|| aside.join(sort.name()), |f| root.join(f));
        let n = counts.entry(format!("{}/{stem}", folder.display())).or_default();
        *n += 1;
        let file = folder.join(format!("{stem}_{n}.nbt"));
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let refs: Vec<([i32; 3], &str, Option<Compound>)> = local.iter().map(|(p, s, c)| (*p, s.as_str(), c.clone())).collect();
        std::fs::write(&file, crate::testing::template_bytes(tsize, &refs, &[])).map_err(|e| e.to_string())?;
        if let (Some(o), Some(t)) = (opening, sort.folder())
            && doors.is_empty()
        {
            let p = place([o[0] - min[0], o[1] - min[1], o[2] - min[2]], origin, turn);
            openings.entry(t.to_owned()).or_default().insert(file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p);
        }
        let made_of = {
            let mut m: HashMap<&str, usize> = HashMap::new();
            for (_, s) in &blocks {
                *m.entry(s.split('[').next().unwrap_or(s).trim_start_matches("minecraft:")).or_default() += 1;
            }
            let mut v: Vec<(&str, usize)> = m.into_iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1));
            v.into_iter().take(3).map(|(k, _)| k.to_owned()).collect()
        };
        cut.builds.push(Build { min, max, sort, entrance, file: file.strip_prefix(dir).unwrap_or(&file).display().to_string(), blocks: blocks.len(), made_of });
    }
    // The manifest: the bundle's halls are its church, its manors stand
    // on the heights.
    let mut types = serde_json::json!({
        "house": { "share": 0.7 },
        "manor": { "share": 0.08, "max": 2, "prefer": "high_ground" },
        "church": { "share": 0.04, "max": 1, "centre": true }
    });
    for (t, files) in &openings {
        types[t.as_str()]["files"] = files.iter().map(|(f, p)| (f.clone(), serde_json::json!({ "door": p }))).collect::<serde_json::Map<_, _>>().into();
    }
    let manifest = serde_json::json!({
        "name": name,
        "description": "Builds cut from a world save, for trying villages only: the builds are their author's.",
        "types": types,
        "foundation": 1,
        "villagers": 1
    });
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    std::fs::write(root.join("village.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(cut)
}

/// Each height, and its commonest blocks with how many.
pub type Layers = Vec<(i32, Vec<(String, usize)>)>;

/// How many of each block there are at each height, from `y0` to `y1`:
/// what the ground of a world is made of.
pub fn layers(world: &Path, y0: i32, y1: i32) -> Result<Layers, String> {
    let all = read_world(world)?;
    let mut by: BTreeMap<i32, HashMap<String, usize>> = BTreeMap::new();
    for (p, s) in &all {
        if (y0..=y1).contains(&p[1]) {
            *by.entry(p[1]).or_default().entry(s.split('[').next().unwrap_or(s).to_owned()).or_default() += 1;
        }
    }
    Ok(by.into_iter().map(|(y, m)| {
        let mut v: Vec<(String, usize)> = m.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v.truncate(6);
        (y, v)
    }).collect())
}
