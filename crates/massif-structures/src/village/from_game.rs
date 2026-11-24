//! A village pack made of the game's own village buildings, read from the
//! player's Minecraft: real buildings to try the village with before a
//! builder's pack is ready. Written to the player's disk only — the game's
//! files are never shipped with Massif.
//!
//! Each template of a theme's houses, town centres and decor is sorted into
//! the pack's types by its name (a temple is the church, a weaponsmith a
//! forge, a meeting point the well…), turned so that the side it joins its
//! street by — its jigsaw — faces south as the pack format asks, its jigsaw
//! blocks left as what they become once joined, and written as a structure
//! `.nbt` with a `village.json` naming the doors the templates have none of.

use super::Side;
use super::buildings::turn_for;
use crate::geom::{BBox, Dir, place};
use crate::library::Library;
use crate::nbt::Compound;
use crate::pool::Element;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The game's village themes, by the name of their folder in the game.
pub const THEMES: [&str; 5] = ["plains", "desert", "savanna", "taiga", "snowy"];

/// The type of the pack a template of the game goes in, by its name, and
/// the name of the prop for decor; `None` to leave it out.
fn type_of(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    Some(if n.contains("zombie") || n.contains("street") || n.contains("terminator") {
        return None;
    } else if n.contains("temple") {
        "church"
    } else if n.contains("weaponsmith") || n.contains("tool_smith") || n.contains("toolsmith") || n.contains("armorer") {
        "forge"
    } else if n.contains("farm") {
        "farm"
    } else if n.contains("animal_pen") || n.contains("stable") {
        "stable"
    } else if n.contains("meeting_point") || n.contains("fountain") || n.contains("well") {
        "well"
    } else if n.contains("lamp") || n.contains("accessory") || n.contains("decor") || n.contains("statue") || n.contains("camel") {
        "props"
    } else {
        "house"
    })
}

/// The name of a prop of the game, for the pack's `props/` folder.
fn prop_name(name: &str) -> &'static str {
    if name.contains("lamp") { "lamppost" } else { "accessory" }
}

/// What making a pack found, template by template.
#[derive(Debug, Default)]
pub struct Made {
    pub root: PathBuf,
    /// Each template written: its id, the pack's file, the type.
    pub written: Vec<(String, String, String)>,
    /// What was left out, and why.
    pub skipped: Vec<String>,
}

/// The side a template joins its street by: its jigsaw that faces sideways
/// and leads to a street (or the only one that faces sideways).
fn entrance(t: &crate::template::Template) -> Option<&crate::template::Jigsaw> {
    let side: Vec<&crate::template::Jigsaw> = t.jigsaws.iter().filter(|j| j.front.is_horizontal()).collect();
    side.iter().copied().find(|j| j.pool.contains("street") || j.name.contains("entrance") || j.target.contains("street")).or_else(|| side.first().copied())
}

fn side_of(d: Dir) -> Side {
    match d {
        Dir::North => Side::North,
        Dir::East => Side::East,
        Dir::West => Side::West,
        _ => Side::South,
    }
}

/// A template written: its type, file, size, blocks and door.
type Written = (String, String, [i32; 3], Vec<([i32; 3], String, Option<Compound>)>, Option<[i32; 3]>);

/// Write the game's `theme` (`plains`) as a pack into `dir`
/// (`dir/<Theme>/…`).
pub fn write_pack(lib: &Library, theme: &str, dir: &Path) -> Result<Made, String> {
    let title = {
        let mut c = theme.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let root = dir.join(&title);
    let mut made = Made { root: root.clone(), ..Made::default() };
    let mut ids: Vec<String> = Vec::new();
    for pool in ["houses", "town_centers", "decor"] {
        let id = format!("minecraft:village/{theme}/{pool}");
        let Some(p) = lib.pool(&id) else {
            made.skipped.push(format!("{id}: no such pool in this game"));
            continue;
        };
        for (e, _) in &p.elements {
            if let Element::Single { location, .. } = &**e
                && !ids.contains(location)
            {
                ids.push(location.clone());
            }
        }
    }
    if ids.is_empty() {
        return Err(format!("the game has no {theme} village"));
    }
    // Doors the templates lack, by file: their entrance's place.
    let mut doors: BTreeMap<String, BTreeMap<String, [i32; 3]>> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut files: Vec<Written> = Vec::new();
    for id in &ids {
        let short = id.rsplit('/').next().unwrap_or(id).to_owned();
        let Some(kind) = type_of(&short) else {
            made.skipped.push(format!("{id}: not a building of the pack"));
            continue;
        };
        let Some(t) = lib.template(id) else {
            made.skipped.push(format!("{id}: could not be read"));
            continue;
        };
        let palette = t.palettes.first().cloned().unwrap_or_default();
        // Its entrance to the south.
        let (turn, door) = match (kind, entrance(&t)) {
            ("props", _) => (0, None),
            (_, Some(j)) => (turn_for(side_of(j.front), Side::South), Some(j.pos)),
            (_, None) => (0, None),
        };
        let b0 = BBox::of_template(t.size, [0, 0, 0], turn);
        let origin = [-b0.min[0], -b0.min[1], -b0.min[2]];
        let size = [b0.max[0] - b0.min[0] + 1, b0.max[1] - b0.min[1] + 1, b0.max[2] - b0.min[2] + 1];
        let jig: Vec<[i32; 3]> = t.jigsaws.iter().map(|j| j.pos).collect();
        let mut blocks = Vec::new();
        for b in &t.blocks {
            let Some(state) = palette.get(b.state as usize) else { continue };
            let mut state = state.clone();
            let mut nbt = b.nbt.clone();
            if jig.contains(&b.pos) {
                // What a jigsaw becomes once joined.
                state = t.jigsaws.iter().find(|j| j.pos == b.pos).map_or("minecraft:air".into(), |j| j.final_state.clone());
                nbt = None;
            }
            // The game's villages are legacy pieces: their air leaves the
            // land as it is. Left out, it does here too.
            if state.starts_with("minecraft:air") || state.starts_with("minecraft:structure_void") || state.starts_with("minecraft:jigsaw") {
                continue;
            }
            let pos = place(b.pos, origin, turn);
            blocks.push((pos, massif_core::objects::transform_state(&state, turn, false), nbt));
        }
        let door = door.map(|d| {
            let p = place(d, origin, turn);
            // The jigsaw lies in the ground's layer: the way in is the
            // block over it.
            [p[0], p[1] + 1, p[2]]
        });
        let w = size[0].max(size[2]);
        let class = if w <= 11 { "small" } else if w <= 16 { "medium" } else if w <= 24 { "large" } else { "huge" };
        let stem = match kind {
            "house" => class.to_owned(),
            "props" => prop_name(&short).to_owned(),
            _ => kind.to_owned(),
        };
        let n = counts.entry(format!("{kind}/{stem}")).or_default();
        *n += 1;
        let file = format!("{stem}_{n}.nbt");
        files.push((kind.to_owned(), file.clone(), size, blocks, door));
        made.written.push((id.clone(), format!("{kind}/{file}"), kind.to_owned()));
    }
    for (kind, file, size, blocks, door) in &files {
        let has_door = blocks.iter().any(|(p, s, _)| (s.contains("_door[") && s.contains("half=lower")) && p[2] >= size[2] - 4);
        if let (false, Some(d), false) = (has_door, door, kind == "props") {
            doors.entry(kind.clone()).or_default().insert(file.clone(), *d);
        }
        let refs: Vec<([i32; 3], &str, Option<Compound>)> = blocks.iter().map(|(p, s, c)| (*p, s.as_str(), c.clone())).collect();
        let path = root.join(kind).join(file);
        std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
        std::fs::write(&path, crate::testing::template_bytes(*size, &refs, &[])).map_err(|e| e.to_string())?;
    }
    // The manifest: the game's farms are fields already, the well is the
    // square's, and the doors the templates have none of.
    let mut types = serde_json::Map::new();
    let has = |k: &str| files.iter().any(|f| f.0 == k);
    for (k, v) in [
        ("house", serde_json::json!({ "share": 0.6 })),
        ("church", serde_json::json!({ "share": 0.04, "max": 1, "centre": true })),
        ("forge", serde_json::json!({ "share": 0.06, "max": 2 })),
        ("farm", serde_json::json!({ "share": 0.12, "edge": true, "field": false })),
        ("stable", serde_json::json!({ "share": 0.05, "edge": true })),
        ("well", serde_json::json!({ "max": 1, "centre": true })),
    ] {
        if has(k) {
            let mut v = v;
            if let Some(d) = doors.get(k) {
                v["files"] = d.iter().map(|(f, p)| (f.clone(), serde_json::json!({ "door": p }))).collect::<serde_json::Map<_, _>>().into();
            }
            types.insert(k.into(), v);
        }
    }
    let manifest = serde_json::json!({
        "name": format!("{title} (the game's)"),
        "description": format!("The game's {theme} village buildings, read from this computer's Minecraft. For trying villages only: not to be shared."),
        "types": types,
        "props": { "along_roads": ["lamppost", "bench"], "near": { "farm": ["hay_bales"] } },
        "foundation": 1,
        "villagers": 1
    });
    std::fs::write(root.join("village.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(made)
}
