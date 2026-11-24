//! Themes of villages: the game's five, and the builders' packs — folders of
//! schematics by type of building, with a manifest (`village.json`) that
//! says what Massif cannot guess. Documented in `docs/village-packs.md`.

use super::Side;
use crate::nbt::Compound;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The blocks of a road, with their weights, and its width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoadStyle {
    pub width: i32,
    pub blocks: Vec<(String, u32)>,
    /// The blocks of its two edges, if they differ (the premium dirt's
    /// coarse dirt).
    #[serde(default)]
    pub edges: Vec<(String, u32)>,
}

/// How large a building is, read from its footprint (its larger side).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SizeClass {
    Small,
    Medium,
    Large,
    /// Larger than a large house: a church, a castle.
    Huge,
}

impl SizeClass {
    pub const ALL: [SizeClass; 4] = [SizeClass::Small, SizeClass::Medium, SizeClass::Large, SizeClass::Huge];

    pub fn name(self) -> &'static str {
        match self {
            SizeClass::Small => "small",
            SizeClass::Medium => "medium",
            SizeClass::Large => "large",
            SizeClass::Huge => "huge",
        }
    }

    /// Where it stands from small (−1) to large (+1.5): what the *small ↔
    /// large* bias weighs.
    pub fn rank(self) -> f32 {
        match self {
            SizeClass::Small => -1.0,
            SizeClass::Medium => 0.0,
            SizeClass::Large => 1.0,
            SizeClass::Huge => 1.5,
        }
    }
}

/// The sides of the size classes, in blocks: small up to `small[1]`,
/// medium up to `medium[1]`, large up to `large[1]`, huge beyond.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sizes {
    pub small: [i32; 2],
    pub medium: [i32; 2],
    pub large: [i32; 2],
}

impl Default for Sizes {
    /// The user's Plains list: 7–11, 11–16, 16–24.
    fn default() -> Self {
        Sizes { small: [7, 11], medium: [11, 16], large: [16, 24] }
    }
}

impl Sizes {
    /// The class of a footprint of `w` × `d` blocks.
    pub fn class(&self, w: i32, d: i32) -> SizeClass {
        let side = w.max(d);
        if side <= self.small[1] {
            SizeClass::Small
        } else if side <= self.medium[1] {
            SizeClass::Medium
        } else if side <= self.large[1] {
            SizeClass::Large
        } else {
            SizeClass::Huge
        }
    }
}

/// A schematic of a type of building, what was read of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub file: PathBuf,
    /// Its size as saved (x, y, z), before any turn.
    pub size: [i32; 3],
    pub class: SizeClass,
    /// The side its entrance is on, as saved (south unless the manifest
    /// says otherwise).
    pub entrance: Side,
    /// Its door as saved (the lower half), or where it would be.
    pub door: [i32; 3],
    /// Whether a door was found on its entrance side.
    pub door_found: bool,
    pub weight: f32,
    /// A watermill: it stands by water.
    pub water: bool,
}

impl Variant {
    /// Its footprint across (x) and deep (z) once its entrance faces south.
    pub fn footprint(&self) -> [i32; 2] {
        if matches!(self.entrance, Side::East | Side::West) { [self.size[2], self.size[0]] } else { [self.size[0], self.size[2]] }
    }
}

/// A field a farm keeps beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSpec {
    /// Its side, from and to, in blocks.
    pub size: [i32; 2],
    pub crops: Vec<String>,
}

/// A type of building of a theme: its share of the buildings, its rules,
/// its schematics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildingType {
    pub name: String,
    pub share: f32,
    /// At most this many in a district.
    pub max: Option<u32>,
    /// Placed near the middle of its district.
    pub centre: bool,
    /// Placed near the edge of the village.
    pub edge: bool,
    /// Placed near a building of that type.
    pub near: Option<String>,
    /// On the highest plots (a manor).
    pub high_ground: bool,
    /// A field beside each (a farm).
    pub field: Option<FieldSpec>,
    pub variants: Vec<Variant>,
}

impl BuildingType {
    /// Whether it counts as a house for the number of houses asked for (not
    /// a well, a stall or a mill).
    pub fn is_dwelling(&self) -> bool {
        !matches!(self.name.as_str(), "well" | "market" | "mill" | "watchtower" | "walls" | "props")
    }

    /// Whether villagers live in it.
    pub fn has_villagers(&self) -> bool {
        !matches!(self.name.as_str(), "farm" | "well" | "market" | "watchtower" | "walls" | "mill" | "props")
    }
}

/// Walls without pieces of their own, and the pieces a theme may have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WallStyle {
    pub height: i32,
    pub block: String,
    /// A tower every so many blocks along the wall.
    pub towers: i32,
    pub tower: Option<Variant>,
    pub gate: Option<Variant>,
}

/// What a theme puts along its roads and by its buildings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PropRules {
    /// Props along the roads (`lamppost`, `bench`).
    pub along_roads: Vec<String>,
    /// Props by buildings of a type (`farm` → `hay_bales`, `cart`).
    pub near: Vec<(String, Vec<String>)>,
}

/// A theme of village.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    /// The pack it comes from; empty for one of the game's.
    pub pack: String,
    /// The folder of the theme, which its variants' files are in.
    #[serde(default)]
    pub dir: PathBuf,
    /// One of the game's villages, by the key of its kind
    /// (`village_plains`): its districts are assembled as the game's.
    pub vanilla: Option<String>,
    pub types: Vec<BuildingType>,
    pub sizes: Sizes,
    pub main: RoadStyle,
    pub side: RoadStyle,
    pub walls: WallStyle,
    /// The pack's props by name (`cart`, `lamppost`, `fence_corner`).
    pub props: Vec<(String, Vec<Variant>)>,
    pub prop_rules: PropRules,
    /// Layers of foundation under the floor of a building without a door.
    pub foundation: i32,
    /// Villagers in each house.
    pub villagers: u32,
}

impl Theme {
    pub fn kind(&self, name: &str) -> Option<&BuildingType> {
        self.types.iter().find(|t| t.name == name)
    }

    pub fn prop(&self, name: &str) -> &[Variant] {
        self.props.iter().find(|p| p.0 == name).map_or(&[], |p| p.1.as_slice())
    }
}

fn road(blocks: &[(&str, u32)], width: i32) -> RoadStyle {
    RoadStyle { width, blocks: blocks.iter().map(|(b, w)| (b.to_string(), *w)).collect(), edges: Vec::new() }
}

fn default_props() -> PropRules {
    PropRules { along_roads: vec!["lamppost".into(), "bench".into()], near: vec![("farm".into(), vec!["hay_bales".into(), "cart".into()]), ("market".into(), vec!["cart".into()])] }
}

/// The game's five villages, as themes: no pack needed.
pub fn built_in() -> Vec<Theme> {
    let theme = |name: &str, kind: &str, path: &[(&str, u32)], wall: &str| Theme {
        name: name.to_owned(),
        pack: String::new(),
        dir: PathBuf::new(),
        vanilla: Some(kind.to_owned()),
        types: Vec::new(),
        sizes: Sizes::default(),
        main: road(path, 3),
        side: road(path, 2),
        walls: WallStyle { height: 6, block: wall.to_owned(), towers: 48, tower: None, gate: None },
        props: Vec::new(),
        prop_rules: default_props(),
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

/// The rules a type has unless the manifest says otherwise: share, max,
/// centre, edge, near, high ground.
fn default_type(name: &str) -> (f32, Option<u32>, bool, bool, Option<&'static str>, bool) {
    match name {
        "house" => (0.55, None, false, false, None, false),
        "manor" => (0.05, Some(2), false, false, None, true),
        "church" => (0.04, Some(1), true, false, None, false),
        "forge" => (0.05, Some(2), false, false, Some("market"), false),
        "tavern" => (0.04, Some(1), false, false, Some("market"), false),
        "watchtower" => (0.03, Some(2), false, true, None, false),
        "farm" | "stable" => (0.12, None, false, true, None, false),
        "mill" => (0.0, Some(1), false, true, None, false),
        "well" => (0.03, Some(2), true, false, None, false),
        "market" => (0.0, Some(4), true, false, None, false),
        _ => (0.06, None, false, false, None, false),
    }
}

/// The sides a type's buildings usually have, from the user's Plains list:
/// what the pack check compares the schematics with. `(least, most)` of
/// the larger side; a church may be 40 deep.
pub fn usual_sides(name: &str) -> Option<(i32, i32)> {
    Some(match name {
        "house" => (7, 16),
        "manor" => (16, 24),
        "church" => (9, 40),
        "forge" => (9, 13),
        "tavern" => (12, 18),
        "watchtower" => (5, 9),
        "farm" => (12, 24),
        "well" | "market" => (3, 5),
        "props" => (1, 5),
        _ => return None,
    })
}

pub(crate) fn is_schematic(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| massif_objects::EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub(crate) fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_file() && is_schematic(p)).collect();
    out.sort();
    out
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    out.sort();
    out
}

/// A building ready to lay: its blocks in its own box, where its door is.
#[derive(Debug, Clone)]
pub struct Plan {
    pub size: [i32; 3],
    pub blocks: Vec<([i32; 3], String, Option<Compound>)>,
    /// Entities saved in it (which a pack should not have).
    pub entities: usize,
}

/// A schematic, read as a plan.
pub fn plan_of(path: &Path) -> Result<Plan, String> {
    let mut blocks = Vec::new();
    let size: [i32; 3];
    let mut entities = 0;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("nbt")) {
        let t = crate::template::Template::parse(&std::fs::read(path).map_err(|e| e.to_string())?)?;
        size = t.size;
        entities = t.entities.len();
        let palette = t.palettes.first().ok_or("no palette")?;
        for b in &t.blocks {
            let Some(state) = palette.get(b.state as usize) else { continue };
            if state.starts_with("minecraft:structure_void") || state.starts_with("minecraft:jigsaw") || state.starts_with("minecraft:structure_block") {
                continue;
            }
            blocks.push((b.pos, state.clone(), b.nbt.clone()));
        }
    } else {
        let o = massif_objects::read(path).map_err(|e| e.to_string())?.object;
        size = o.size.map(|v| v as i32);
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let k = o.at(x as usize, y as usize, z as usize);
                    if k != 0 {
                        let state = &o.palette[k as usize];
                        if state.starts_with("minecraft:structure_void") {
                            continue;
                        }
                        blocks.push(([x, y, z], state.clone(), None));
                    }
                }
            }
        }
    }
    Ok(Plan { size, blocks, entities })
}

/// The door of `plan` on side `entrance`: the lower half of a door (or a
/// gate) nearest that side, lowest first. `None` when it has none there.
pub fn find_door(plan: &Plan, entrance: Side) -> Option<[i32; 3]> {
    let s = plan.size;
    // How far in from the entrance side.
    let depth = |p: &[i32; 3]| match entrance {
        Side::South => s[2] - 1 - p[2],
        Side::North => p[2],
        Side::East => s[0] - 1 - p[0],
        Side::West => p[0],
    };
    plan.blocks
        .iter()
        .filter(|(_, st, _)| (st.contains("_door[") && st.contains("half=lower")) || st.contains("_fence_gate"))
        .map(|(p, _, _)| *p)
        .filter(|p| depth(p) <= 3)
        .min_by_key(|p| (depth(p), p[1]))
}

/// Where a door would be on side `entrance` of a building of `size`, over
/// `foundation` layers: the middle of that side.
pub fn default_door(size: [i32; 3], entrance: Side, foundation: i32) -> [i32; 3] {
    let y = foundation.min(size[1] - 1).max(0);
    match entrance {
        Side::South => [size[0] / 2, y, size[2] - 1],
        Side::North => [size[0] / 2, y, 0],
        Side::East => [size[0] - 1, y, size[2] / 2],
        Side::West => [0, y, size[2] / 2],
    }
}

/// What the manifest says of one file (`types.<type>.files.<name>`).
#[derive(Debug, Clone, Default)]
struct FileSpec {
    entrance: Option<Side>,
    weight: Option<f32>,
    water: Option<bool>,
}

fn file_spec(spec: Option<&serde_json::Value>, file: &Path) -> FileSpec {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let f = spec.and_then(|s| s.get("files")).and_then(|f| f.get(&name).or_else(|| f.get(&stem)));
    FileSpec {
        entrance: f.and_then(|f| f.get("entrance")).and_then(|v| v.as_str()).and_then(Side::parse),
        weight: f.and_then(|f| f.get("weight")).and_then(|v| v.as_f64()).map(|v| v.max(0.0) as f32),
        water: f.and_then(|f| f.get("water")).and_then(|v| v.as_bool()),
    }
}

/// A schematic read as a variant (`None` with why when it cannot be
/// read).
fn variant(file: &Path, spec: Option<&serde_json::Value>, sizes: &Sizes, foundation: i32) -> Result<(Variant, Plan), String> {
    let plan = plan_of(file)?;
    let fs = file_spec(spec, file);
    let entrance = fs.entrance.or_else(|| spec.and_then(|s| s.get("entrance")).and_then(|v| v.as_str()).and_then(Side::parse)).unwrap_or(Side::South);
    let found = find_door(&plan, entrance);
    let door = found.unwrap_or_else(|| default_door(plan.size, entrance, foundation));
    let stem = file.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let v = Variant {
        file: file.to_owned(),
        size: plan.size,
        class: sizes.class(plan.size[0], plan.size[2]),
        entrance,
        door,
        door_found: found.is_some(),
        weight: fs.weight.unwrap_or(1.0),
        water: fs.water.unwrap_or(stem.contains("water")),
    };
    Ok((v, plan))
}

fn sizes_of(v: Option<&serde_json::Value>) -> Sizes {
    let d = Sizes::default();
    let pair = |k: &str, or: [i32; 2]| -> [i32; 2] {
        v.and_then(|s| s.get(k)).and_then(|p| p.as_array()).and_then(|a| Some([a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32])).unwrap_or(or)
    };
    Sizes { small: pair("small", d.small), medium: pair("medium", d.medium), large: pair("large", d.large) }
}

fn strings(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// What reading a pack met: its themes, and what is wrong in it.
#[derive(Debug, Clone, Default)]
pub struct Read {
    pub themes: Vec<Theme>,
    pub problems: Vec<Problem>,
}

/// Something wrong in a pack, in plain words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    /// The file or folder, relative to the pack.
    pub file: String,
    pub message: String,
    /// Whether the village cannot use it at all (rather than using it as it
    /// is).
    pub serious: bool,
}

/// The themes of the pack in folder `dir` (`<pack>/<theme>/<type>/*.schem`,
/// or `<pack>/<type>/*.schem` for a pack of one theme), its manifests read.
pub fn read_pack(dir: &Path) -> Result<Vec<Theme>, String> {
    let r = read_pack_checked(dir)?;
    Ok(r.themes)
}

/// [`read_pack`], and what is wrong in the pack: the pack check.
pub fn read_pack_checked(dir: &Path) -> Result<Read, String> {
    let pack_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut problems = Vec::new();
    let rel = |p: &Path| p.strip_prefix(dir).unwrap_or(p).display().to_string();
    let manifest = |d: &Path, problems: &mut Vec<Problem>| -> Option<serde_json::Value> {
        let bytes = std::fs::read(d.join("village.json")).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(v) => Some(v),
            Err(e) => {
                problems.push(Problem { file: rel(&d.join("village.json")), message: format!("cannot be read, so it is left out: {e}"), serious: true });
                None
            }
        }
    };
    let top = manifest(dir, &mut problems);
    // A theme folder holds type folders of schematics.
    let is_theme = |d: &Path| subdirs(d).iter().any(|t| !files_in(t).is_empty());
    let theme_dirs: Vec<PathBuf> = if subdirs(dir).iter().any(|d| is_theme(d)) { subdirs(dir).into_iter().filter(|d| is_theme(d)).collect() } else if is_theme(dir) { vec![dir.to_owned()] } else { Vec::new() };
    if theme_dirs.is_empty() {
        return Err(format!("{}: no folder of schematics (theme/type/name.schem)", dir.display()));
    }
    let mut themes = Vec::new();
    for t in theme_dirs {
        let own = if t == dir { None } else { manifest(&t, &mut problems) };
        // The theme's manifest over the pack's.
        let get = |key: &str| -> Option<serde_json::Value> { own.as_ref().and_then(|m| m.get(key)).or_else(|| top.as_ref().and_then(|m| m.get(key))).cloned() };
        let style = |v: Option<serde_json::Value>, width: i32| -> RoadStyle {
            let blocks_of = |k: &str| -> Vec<(String, u32)> { v.as_ref().and_then(|v| v.get(k)).and_then(|b| b.as_object()).map(|o| o.iter().map(|(k, w)| (crate::source::full_id(k), w.as_u64().unwrap_or(1).max(1) as u32)).collect()).unwrap_or_default() };
            let blocks = blocks_of("blocks");
            RoadStyle {
                width: v.as_ref().and_then(|v| v.get("width")).and_then(|w| w.as_i64()).map_or(width, |w| w.clamp(1, 9) as i32),
                blocks: if blocks.is_empty() { vec![("minecraft:dirt_path".into(), 1)] } else { blocks },
                edges: blocks_of("edges"),
            }
        };
        let sizes = sizes_of(get("sizes").as_ref());
        let foundation = get("foundation").and_then(|v| v.as_i64()).map_or(3, |v| v.clamp(0, 16) as i32);
        let roads = get("roads");
        let types_json = get("types");
        let mut types = Vec::new();
        let mut props: Vec<(String, Vec<Variant>)> = Vec::new();
        let mut tower = None;
        let mut gate = None;
        let mut seen: HashMap<u64, String> = HashMap::new();
        for d in subdirs(&t) {
            let name = d.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let files = files_in(&d);
            if files.is_empty() {
                continue;
            }
            let spec = types_json.as_ref().and_then(|j| j.get(&name));
            let mut variants = Vec::new();
            for f in &files {
                match variant(f, spec, &sizes, if name == "props" { 0 } else { foundation }) {
                    Ok((v, plan)) => {
                        check_variant(&name, &v, &plan, &rel(f), foundation, &mut problems);
                        // The same build twice.
                        let hash = {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            plan.size.hash(&mut h);
                            for b in &plan.blocks {
                                b.0.hash(&mut h);
                                b.1.hash(&mut h);
                            }
                            h.finish()
                        };
                        if let Some(other) = seen.get(&hash) {
                            problems.push(Problem { file: rel(f), message: format!("is the same build as {other}: it only makes that one come up twice as often"), serious: false });
                        } else {
                            seen.insert(hash, rel(f));
                        }
                        variants.push(v);
                    }
                    Err(e) => problems.push(Problem { file: rel(f), message: format!("cannot be read, so it is left out: {e}"), serious: true }),
                }
            }
            if variants.is_empty() {
                continue;
            }
            match name.as_str() {
                "walls" => {
                    let find = |stem: &str| variants.iter().find(|v| v.file.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem))).cloned();
                    tower = find("tower");
                    gate = find("gate");
                    continue;
                }
                "props" => {
                    // By name, the number after the last _ dropped: fence_1,
                    // fence_2 are both fences.
                    for v in variants {
                        let stem = v.file.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                        let key = match stem.rsplit_once('_') {
                            Some((k, n)) if n.chars().all(|c| c.is_ascii_digit()) => k.to_owned(),
                            _ => stem,
                        };
                        match props.iter_mut().find(|p| p.0 == key) {
                            Some(p) => p.1.push(v),
                            None => props.push((key, vec![v])),
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let (share, max, centre, edge, near, high) = default_type(&name);
            let f = |k: &str| spec.and_then(|s| s.get(k));
            let field = f("field").map(|v| FieldSpec {
                size: v.get("size").and_then(|p| p.as_array()).and_then(|a| Some([a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32])).unwrap_or([12, 24]),
                crops: {
                    let c = strings(v.get("crops"));
                    if c.is_empty() { vec!["wheat".into(), "carrots".into(), "potatoes".into()] } else { c }
                },
            });
            let field = field.or_else(|| (name == "farm").then(|| FieldSpec { size: [12, 24], crops: vec!["wheat".into(), "carrots".into(), "potatoes".into()] }));
            types.push(BuildingType {
                share: f("share").and_then(|v| v.as_f64()).map_or(share, |v| v.max(0.0) as f32),
                max: f("max").and_then(|v| v.as_u64()).map(|v| v as u32).or(max),
                centre: f("centre").or_else(|| f("center")).and_then(|v| v.as_bool()).unwrap_or(centre),
                edge: f("edge").and_then(|v| v.as_bool()).unwrap_or(edge),
                near: f("near").and_then(|v| v.as_str()).map(str::to_owned).or(near.map(str::to_owned)),
                high_ground: f("prefer").and_then(|v| v.as_str()).map_or(high, |p| p == "high_ground"),
                field,
                name,
                variants,
            });
        }
        // Types the manifest names that have no folder.
        if let Some(obj) = types_json.as_ref().and_then(|j| j.as_object()) {
            for k in obj.keys() {
                if !types.iter().any(|t| &t.name == k) && k != "props" && k != "walls" {
                    problems.push(Problem { file: rel(&t), message: format!("the manifest speaks of \"{k}\", but there is no {k}/ folder of schematics"), serious: false });
                }
            }
        }
        if !types.iter().any(|t| t.name == "house") {
            problems.push(Problem { file: rel(&t), message: "has no house/ folder: the village has no houses".into(), serious: false });
        }
        if types.is_empty() {
            continue;
        }
        let walls = get("walls");
        let w = |k: &str| walls.as_ref().and_then(|v| v.get(k));
        let pr = get("props");
        let prop_rules = match pr {
            Some(p) => PropRules {
                along_roads: strings(p.get("along_roads")),
                near: p.get("near").and_then(|n| n.as_object()).map(|o| o.iter().map(|(k, v)| (k.clone(), strings(Some(v)))).collect()).unwrap_or_default(),
            },
            None => default_props(),
        };
        themes.push(Theme {
            name: own.as_ref().and_then(|m| m.get("name")).and_then(|n| n.as_str()).map_or_else(|| t.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), str::to_owned),
            pack: top.as_ref().and_then(|m| m.get("name")).and_then(|n| n.as_str()).map_or_else(|| pack_name.clone(), str::to_owned),
            dir: t.clone(),
            vanilla: None,
            types,
            sizes,
            main: style(roads.as_ref().and_then(|r| r.get("main")).cloned(), 3),
            side: style(roads.as_ref().and_then(|r| r.get("side")).cloned(), 2),
            walls: WallStyle {
                height: w("height").and_then(|v| v.as_i64()).map_or(6, |v| v.clamp(3, 24) as i32),
                block: w("block").and_then(|v| v.as_str()).map_or_else(|| "minecraft:stone_bricks".to_owned(), crate::source::full_id),
                towers: w("towers").and_then(|v| v.as_i64()).map_or(40, |v| v.clamp(8, 400) as i32),
                tower,
                gate,
            },
            props,
            prop_rules,
            foundation,
            villagers: get("villagers").and_then(|v| v.as_u64()).map_or(1, |v| v.min(8) as u32),
        });
    }
    if themes.is_empty() {
        return Err(format!("{}: no building in it", dir.display()));
    }
    Ok(Read { themes, problems })
}

/// What is wrong with one schematic of type `kind`, in plain words.
fn check_variant(kind: &str, v: &Variant, plan: &Plan, file: &str, foundation: i32, out: &mut Vec<Problem>) {
    let side = v.entrance.name();
    let [w, d] = [v.size[0], v.size[2]];
    if plan.blocks.is_empty() {
        out.push(Problem { file: file.into(), message: "is empty: no block in it".into(), serious: true });
        return;
    }
    if !v.door_found && !matches!(kind, "walls" | "props" | "well" | "market" | "farm" | "mill") {
        out.push(Problem {
            file: file.into(),
            message: format!("has no door on its {side} side: Massif cannot tell where its entrance is, and puts it in the middle of that side, {foundation} blocks up. Put a door on the {side} wall at floor level, or name the side in village.json (\"entrance\")"),
            serious: false,
        });
    }
    if v.door_found && v.door[1] == 0 && kind != "props" {
        out.push(Problem { file: file.into(), message: "has its door on its lowest layer: no foundation under its floor, so on a slope it floats. Add 3 to 4 layers of stone under the floor".into(), serious: false });
    }
    if let Some((lo, hi)) = usual_sides(kind) {
        let big = w.max(d);
        let small = w.min(d);
        let deep_church = kind == "church" && small <= 24 && big <= 40;
        if (big > hi && !deep_church) || big < lo {
            out.push(Problem {
                file: file.into(),
                message: format!("is {w} × {d} blocks: {} than a {kind} usually is ({lo} × {lo} to {hi} × {hi}). It is used as it is", if big > hi { "larger" } else { "smaller" }),
                serious: false,
            });
        }
    }
    if plan.entities > 0 {
        out.push(Problem { file: file.into(), message: format!("holds {} entities: villagers and animals come from village.json, and these are left out", plan.entities), serious: false });
    }
    // Air in its four bottom corners at ground level: it carves the land
    // round the building, where structure voids would let it show.
    let corners = [[0, v.door[1], 0], [w - 1, v.door[1], 0], [0, v.door[1], d - 1], [w - 1, v.door[1], d - 1]];
    let air_corners = corners.iter().filter(|c| plan.blocks.iter().any(|(p, s, _)| p == *c && s.starts_with("minecraft:air"))).count();
    if air_corners >= 3 {
        out.push(Problem { file: file.into(), message: "has air around the building where structure voids belong: the air carves into the land round it".into(), serious: false });
    }
}
