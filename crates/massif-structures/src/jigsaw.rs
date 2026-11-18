//! Jigsaw assembly, as the game does it: a first piece drawn from the start
//! pool, then, breadth first, a piece joined to each of its jigsaw blocks,
//! drawn from the pool that jigsaw names, turned and moved until its own
//! jigsaw meets it and its box fits in the room left — until the structure
//! is as deep as it goes, or as far from its middle as it may reach.

use crate::geom::{BBox, Free};
use crate::ground::Ground;
use crate::library::Library;
use crate::pool::{Element, Projection};
use crate::processor::Heightmap;
use crate::rng::Rng;
use crate::source::full_id;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// How the game assembles a jigsaw structure (its `worldgen/structure`
/// file).
#[derive(Debug, Clone)]
pub struct Spec {
    pub start_pool: String,
    /// The jigsaw of the first piece that goes at the start's place, if
    /// one is named (the Ancient City's `city_anchor`).
    pub start_jigsaw: Option<String>,
    /// How many joins deep it goes (its `size`).
    pub depth: u32,
    /// How far from its middle a piece may reach, in blocks.
    pub max_distance: i32,
    /// The game's fix for villages: a house's box made tall enough for
    /// what stands on it.
    pub expansion_hack: bool,
    /// Its start's height measured from the land there rather than from
    /// the bottom of the world.
    pub project: Option<Heightmap>,
    /// Blocks kept free of it at the bottom and top of the world.
    pub padding: (i32, i32),
    /// Pools named by others (Trial Chambers' spawners): each alias and
    /// the pool it stands for, drawn once for the whole structure.
    pub aliases: Vec<Alias>,
}

/// A pool alias of a structure.
#[derive(Debug, Clone)]
pub enum Alias {
    Direct { alias: String, target: String },
    Random { alias: String, targets: Vec<(String, u32)> },
    Group { groups: Vec<(Vec<Alias>, u32)> },
}

/// A piece placed.
#[derive(Debug, Clone)]
pub struct Placed {
    pub element: Arc<Element>,
    /// Where its template's corner goes.
    pub origin: [i32; 3],
    pub turn: u8,
    pub bbox: BBox,
    pub depth: u32,
    pub ground_delta: i32,
    /// The height of the land under the jigsaw a terrain-matching piece
    /// was joined at (the game's junctions), for laying it.
    pub junctions: Vec<[i32; 4]>,
}

/// The aliases drawn for one structure: alias → pool.
pub fn draw_aliases(aliases: &[Alias], rng: &mut Rng) -> HashMap<String, String> {
    let mut out = HashMap::new();
    fn add(a: &Alias, rng: &mut Rng, out: &mut HashMap<String, String>) {
        match a {
            Alias::Direct { alias, target } => {
                out.insert(full_id(alias), full_id(target));
            }
            Alias::Random { alias, targets } => {
                let total: u32 = targets.iter().map(|t| t.1).sum();
                let mut r = rng.below(total.max(1));
                for (t, w) in targets {
                    if r < *w {
                        out.insert(full_id(alias), full_id(t));
                        break;
                    }
                    r -= w;
                }
            }
            Alias::Group { groups } => {
                let total: u32 = groups.iter().map(|g| g.1).sum();
                let mut r = rng.below(total.max(1));
                for (g, w) in groups {
                    if r < *w {
                        for a in g {
                            add(a, rng, out);
                        }
                        break;
                    }
                    r -= w;
                }
            }
        }
    }
    for a in aliases {
        add(a, rng, &mut out);
    }
    out
}

/// Parse the aliases of a structure's file.
pub fn parse_aliases(json: Option<&serde_json::Value>) -> Vec<Alias> {
    fn one(v: &serde_json::Value) -> Option<Alias> {
        let kind = v.get("type")?.as_str()?;
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
        Some(match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "direct" => Alias::Direct { alias: s("alias")?, target: s("target")? },
            "random" => Alias::Random {
                alias: s("alias")?,
                targets: v.get("targets")?.as_array()?.iter().filter_map(|t| Some((t.get("data")?.as_str()?.to_owned(), t.get("weight").and_then(|w| w.as_u64()).unwrap_or(1) as u32))).collect(),
            },
            "random_group" => Alias::Group {
                groups: v.get("groups")?.as_array()?.iter().filter_map(|g| Some((g.get("data")?.as_array()?.iter().filter_map(one).collect(), g.get("weight").and_then(|w| w.as_u64()).unwrap_or(1) as u32))).collect(),
            },
            _ => return None,
        })
    }
    json.and_then(|j| j.as_array()).map(|a| a.iter().filter_map(one).collect()).unwrap_or_default()
}

/// Assemble a structure whose start is at `start` (its height from the
/// bottom of the world, or above the land when the spec projects it),
/// turned `turn` quarter turns (drawn when `None`). Its pieces, the first
/// first; empty when the start pool has nothing to place.
pub fn assemble(lib: &Library, spec: &Spec, start: [i32; 3], turn: Option<u8>, rng: &mut Rng, ground: &dyn Ground) -> Vec<Placed> {
    assemble_within(lib, spec, start, turn, rng, ground, &|_| true)
}

/// [`assemble`], each piece's box also asked of `within` (a village kept to
/// a selection).
pub fn assemble_within(lib: &Library, spec: &Spec, start: [i32; 3], turn: Option<u8>, rng: &mut Rng, ground: &dyn Ground, within: &dyn Fn(&BBox) -> bool) -> Vec<Placed> {
    let aliases = draw_aliases(&spec.aliases, rng);
    let alias = |pool: &str| -> String {
        let id = full_id(pool);
        aliases.get(&id).cloned().unwrap_or(id)
    };
    let turn = turn.unwrap_or_else(|| rng.below(4) as u8);
    let Some(pool) = lib.pool(&alias(&spec.start_pool)) else {
        lib.unknown(format!("pool {} (missing)", spec.start_pool));
        return Vec::new();
    };
    let Some(element) = pool.random(rng) else { return Vec::new() };
    if element.is_empty() {
        return Vec::new();
    }
    // The start's place is where the named jigsaw goes, if one is named.
    let mut origin = start;
    if let Some(name) = &spec.start_jigsaw {
        let jigsaws: Vec<_> = element.jigsaws(lib, [0, 0, 0], turn, rng).into_iter().filter(|j| full_id(&j.name) == full_id(name)).collect();
        match rng.pick(&jigsaws) {
            Some(j) => origin = [start[0] - j.pos[0], start[1] - j.pos[1], start[2] - j.pos[2]],
            None => {
                lib.unknown(format!("jigsaw {name} (not in the start piece)"));
                return Vec::new();
            }
        }
    }
    let Some(mut bbox) = element.bbox(lib, origin, turn) else { return Vec::new() };
    let delta = element.ground_level_delta();
    let (cx, cz) = ((bbox.min[0] + bbox.max[0]) / 2, (bbox.min[2] + bbox.max[2]) / 2);
    // Its height: from the bottom of the world, or above the land at its
    // middle.
    let cy = match spec.project {
        Some(h) => start[1] + h.height(ground, cx, cz),
        None => origin[1],
    };
    let shift = cy - (bbox.min[1] + delta);
    origin[1] += shift;
    bbox = bbox.moved([0, shift, 0]);
    let y = cy + (start[1] - (origin[1] - shift));
    if !within(&bbox) {
        return Vec::new();
    }
    let mut pieces = vec![Placed { element, origin, turn, bbox, depth: 0, ground_delta: delta, junctions: Vec::new() }];
    if spec.depth == 0 {
        return pieces;
    }
    let reach = spec.max_distance;
    let bounds = BBox {
        min: [cx - reach, (y - reach).max(ground.min_y() + spec.padding.0), cz - reach],
        max: [cx + reach, (y + reach).min(ground.max_y() - 1 - spec.padding.1), cz + reach],
    };
    let mut outer = Free::new(bounds);
    outer.take(bbox);
    // The rooms left: the outer one, and each piece's own box for the
    // pieces joined inside it.
    let mut frees: Vec<Free> = vec![outer];
    // The pieces waiting to have their jigsaws joined: by priority, the
    // first come first within one.
    let mut queue: BTreeMap<(std::cmp::Reverse<i32>, u64), (usize, usize)> = BTreeMap::new();
    let mut order = 0u64;
    queue.insert((std::cmp::Reverse(0), order), (0, 0));
    let mut steps = 0usize;
    while let Some((_, (index, free))) = queue.pop_first() {
        steps += 1;
        // A structure of mods may loop without end: a bound, generous.
        if steps > 20_000 || pieces.len() > 10_000 {
            lib.unknown("a structure that does not end (stopped at 10,000 pieces)".to_owned());
            break;
        }
        let parent = pieces[index].clone();
        let rigid = parent.element.projection() == Projection::Rigid;
        let min_y = parent.bbox.min[1];
        let mut inner: Option<usize> = None;
        let mut surface: Option<i32> = None;
        'jigsaws: for jigsaw in parent.element.jigsaws(lib, parent.origin, parent.turn, rng) {
            let attached = jigsaw.attached();
            let jy = jigsaw.pos[1] - min_y;
            let Some(pool) = lib.pool(&alias(&jigsaw.pool)) else {
                lib.unknown(format!("pool {} (missing)", jigsaw.pool));
                continue;
            };
            if pool.is_empty() && full_id(&pool.name) != "minecraft:empty" {
                continue;
            }
            let fallback = lib.pool(&alias(&pool.fallback));
            // A piece joined inside its parent's box takes room in that box;
            // one outside, in the room the parent was given.
            let room = if parent.bbox.contains(attached) {
                *inner.get_or_insert_with(|| {
                    frees.push(Free::new(parent.bbox));
                    frees.len() - 1
                })
            } else {
                free
            };
            let mut candidates: Vec<Arc<Element>> = Vec::new();
            // As deep as it goes, only the fallback's pieces: street ends.
            if parent.depth != spec.depth {
                candidates.extend(pool.shuffled(rng));
            }
            if let Some(f) = &fallback {
                candidates.extend(f.shuffled(rng));
            }
            let priority = jigsaw.placement_priority;
            for candidate in candidates {
                if candidate.is_empty() {
                    break;
                }
                let mut turns = [0u8, 1, 2, 3];
                rng.shuffle(&mut turns);
                for turn in turns {
                    let joints = candidate.jigsaws(lib, [0, 0, 0], turn, rng);
                    let Some(local_box) = candidate.bbox(lib, [0, 0, 0], turn) else { continue };
                    // Villages: a house's box raised to hold what its roof's
                    // jigsaws would join (the game's expansion hack).
                    let expansion = if spec.expansion_hack && local_box.span(1) <= 16 {
                        joints
                            .iter()
                            .filter(|j| local_box.contains(j.attached()))
                            .filter_map(|j| {
                                let p = lib.pool(&alias(&j.pool))?;
                                let f = lib.pool(&alias(&p.fallback));
                                let tallest = |p: &crate::pool::Pool| p.elements.iter().filter_map(|(e, _)| e.bbox(lib, [0, 0, 0], 0).map(|b| b.span(1))).max().unwrap_or(0);
                                Some(tallest(&p).max(f.as_deref().map_or(0, tallest)))
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for joint in &joints {
                        if !jigsaw.can_attach(joint) {
                            continue;
                        }
                        let offset = [attached[0] - joint.pos[0], attached[1] - joint.pos[1], attached[2] - joint.pos[2]];
                        let child_box = local_box.moved(offset);
                        let child_min_y = child_box.min[1];
                        let child_rigid = candidate.projection() == Projection::Rigid;
                        let cjy = joint.pos[1];
                        let dy = jy - cjy + jigsaw.front.step()[1];
                        let new_min_y = if rigid && child_rigid {
                            min_y + dy
                        } else {
                            let s = *surface.get_or_insert_with(|| ground.surface(jigsaw.pos[0], jigsaw.pos[2]));
                            s - cjy
                        };
                        let delta_y = new_min_y - child_min_y;
                        let mut placed_box = child_box.moved([0, delta_y, 0]);
                        let child_origin = [offset[0], offset[1] + delta_y, offset[2]];
                        if expansion > 0 {
                            let top = (expansion + 1).max(placed_box.max[1] - placed_box.min[1]);
                            placed_box.max[1] = placed_box.max[1].max(placed_box.min[1] + top);
                        }
                        if !frees[room].fits(&placed_box) || !within(&placed_box) {
                            continue;
                        }
                        frees[room].take(placed_box);
                        let ground_delta = if child_rigid { parent.ground_delta - dy } else { candidate.ground_level_delta() };
                        let junction_y = if rigid {
                            min_y + jy
                        } else if child_rigid {
                            new_min_y + cjy
                        } else {
                            surface.unwrap_or_else(|| ground.surface(jigsaw.pos[0], jigsaw.pos[2])) + dy / 2
                        };
                        pieces[index].junctions.push([attached[0], junction_y - jy + parent.ground_delta, attached[2], dy]);
                        let child = Placed {
                            element: candidate.clone(),
                            origin: child_origin,
                            turn,
                            bbox: placed_box,
                            depth: parent.depth + 1,
                            ground_delta,
                            junctions: vec![[jigsaw.pos[0], junction_y - cjy + ground_delta, jigsaw.pos[2], -dy]],
                        };
                        pieces.push(child);
                        if parent.depth < spec.depth {
                            // The room the child's own children take from:
                            // the same room as itself.
                            order += 1;
                            queue.insert((std::cmp::Reverse(priority), order), (pieces.len() - 1, room));
                        }
                        continue 'jigsaws;
                    }
                }
            }
        }
    }
    pieces
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ground::Flat;
    pub use crate::testing::{village, village_spec};

    #[test]
    fn pieces_join_at_their_jigsaws_without_overlapping() {
        let lib = village();
        let ground = Flat { height: 64, water: None };
        let pieces = assemble(&lib, &village_spec(), [100, 0, 200], Some(0), &mut Rng::new(3), &ground);
        assert!(lib.unknowns().is_empty(), "{:?}", lib.unknowns());
        let names: Vec<&str> = pieces.iter().map(|p| p.element.location().unwrap_or("")).collect();
        assert_eq!(names[0], "t:square");
        let streets = names.iter().filter(|n| **n == "t:street").count();
        let houses = names.iter().filter(|n| **n == "t:house").count();
        assert!(streets >= 4, "{names:?}");
        assert!(houses >= 2, "{names:?}");
        // The square stands on the land, its floor one above the bottom of
        // its box.
        assert_eq!(pieces[0].bbox.min[1], 63);
        // No two pieces overlap (beyond touching).
        for (i, a) in pieces.iter().enumerate() {
            for b in &pieces[i + 1..] {
                let mut a2 = a.bbox;
                let mut b2 = b.bbox;
                // The expansion hack raises boxes: compare their floors.
                a2.max[1] = a2.min[1];
                b2.max[1] = b2.min[1];
                assert!(!a2.overlaps(&b2) || a.bbox.min[1] != b.bbox.min[1], "{:?} {:?}", a.bbox, b.bbox);
            }
            // Within reach of the middle.
            assert!(a.bbox.min[0] >= 100 - 64 && a.bbox.max[0] <= 100 + 64, "{:?}", a.bbox);
        }
        // Houses stand with their door on the street's level: the floor of
        // a house (its box's bottom plus one) is the land's surface.
        for p in pieces.iter().filter(|p| p.element.location() == Some("t:house")) {
            assert_eq!(p.bbox.min[1], 63, "{:?}", p.bbox);
            assert!(p.depth >= 2);
        }
        // The same draw, the same structure.
        let again = assemble(&lib, &village_spec(), [100, 0, 200], Some(0), &mut Rng::new(3), &ground);
        assert_eq!(again.iter().map(|p| (p.origin, p.turn)).collect::<Vec<_>>(), pieces.iter().map(|p| (p.origin, p.turn)).collect::<Vec<_>>());
    }
}
