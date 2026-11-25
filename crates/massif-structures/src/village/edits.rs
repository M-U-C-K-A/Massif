//! Changes made by hand to single buildings of a village — taken away,
//! moved, turned, swapped for another of the pack, drawn again —, kept with
//! the village and made again each time it is built with the same draw.

use super::buildings::{Buildings, Placement, fit, levelled};
use super::pack::Variant;
use super::roads::{Road, RoadKind, Roads};
use super::{Area, Side, Theme, height};
use crate::geom::BBox;
use crate::ground::Ground;
use crate::rng::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// What was changed by hand of one building of a village.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildingEdit {
    /// The building: the column before its door the draw gave it (its
    /// placement's `key`).
    pub at: [i32; 2],
    /// Taken away.
    pub removed: bool,
    /// Moved so many blocks east and south.
    pub offset: [i32; 2],
    /// Turned so many quarter turns clockwise, about its middle.
    pub turn: u8,
    /// Another building of the pack in its place: its type and the name of
    /// its file.
    pub swap: Option<(String, String)>,
    /// Drawn again so many times: another building of its type.
    pub draw: u32,
}

impl BuildingEdit {
    pub fn new(at: [i32; 2]) -> BuildingEdit {
        BuildingEdit { at, ..BuildingEdit::default() }
    }

    /// Whether it changes nothing.
    pub fn is_empty(&self) -> bool {
        !self.removed && self.offset == [0, 0] && self.turn.is_multiple_of(4) && self.swap.is_none() && self.draw == 0
    }
}

/// Side `s` turned `q` quarter turns clockwise.
fn turned(s: Side, q: u8) -> Side {
    [Side::North, Side::East, Side::South, Side::West][((s.quarter() + q) % 4) as usize]
}

/// The file name of `v`.
pub fn file_name(v: &Variant) -> String {
    v.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The middle of box `b`, on the map.
fn middle(b: &BBox) -> [i32; 2] {
    [(b.min[0] + b.max[0]).div_euclid(2), (b.min[2] + b.max[2]).div_euclid(2)]
}

/// Make `edits` on `b`, the buildings a village's draw placed: each
/// building an edit names is taken away, or placed again as it says (moved,
/// turned about its middle, another variant), a path laid from its door to
/// the nearest road when it no longer opens on one; the land levelled under
/// them all again. What could not be done, in plain words.
#[allow(clippy::too_many_arguments)]
pub fn apply(edits: &[BuildingEdit], ground: &dyn Ground, area: &Area, roads: &mut Roads, theme: &Theme, villagers: u32, seed: u64, b: &mut Buildings) -> Vec<String> {
    let mut notes = Vec::new();
    if edits.is_empty() {
        return notes;
    }
    let by_key: HashMap<[i32; 2], &BuildingEdit> = edits.iter().map(|e| (e.at, e)).collect();
    let found: HashSet<[i32; 2]> = b.placed.iter().filter_map(|p| p.key).collect();
    let lost = edits.iter().filter(|e| !found.contains(&e.at)).count();
    if lost > 0 {
        notes.push(format!("{lost} buildings changed by hand are no longer in this draw of the village: their changes wait for it"));
    }
    let mut keep: Vec<bool> = vec![true; b.placed.len()];
    let mut moved: Vec<usize> = Vec::new();
    for (i, p) in b.placed.iter_mut().enumerate() {
        let Some(e) = p.key.and_then(|k| by_key.get(&k)) else { continue };
        if e.removed {
            keep[i] = false;
            continue;
        }
        if e.is_empty() {
            continue;
        }
        // Which building: the one swapped in, another of its type drawn,
        // or its own.
        let own = |kind: &str, file: &std::path::Path| theme.kind(kind).and_then(|t| t.variants.iter().find(|v| v.file == file).map(|v| (t, v)));
        let swapped = e.swap.as_ref().and_then(|(kind, name)| theme.kind(kind).and_then(|t| t.variants.iter().find(|v| file_name(v) == *name).map(|v| (t, v))));
        if let Some((kind, name)) = &e.swap
            && swapped.is_none()
        {
            notes.push(format!("{kind} {name} is no longer in the pack: the building drawn there stays"));
        }
        let Some((t, mut v)) = swapped.or_else(|| own(&p.kind, &p.file)) else { continue };
        if swapped.is_none() && e.draw > 0 && t.variants.len() > 1 {
            let others: Vec<&Variant> = t.variants.iter().filter(|o| o.file != p.file).collect();
            let mut rng = Rng::new(seed ^ (e.at[0] as u64).wrapping_mul(0x9e37_79b9) ^ (e.at[1] as u64).wrapping_mul(0x85eb_ca6b) ^ (e.draw as u64) << 40);
            v = others[rng.below(others.len() as u32) as usize];
        }
        let facing = turned(p.facing, e.turn % 4);
        let key = p.key.unwrap_or([p.door[0], p.door[2]]);
        let (front, floor) = if e.offset == [0, 0] && e.turn.is_multiple_of(4) {
            // In its place: its door on the same road.
            (key, p.door[1])
        } else {
            // Its middle where it stood, moved; its door wherever the turn
            // puts it round that middle.
            let goal = middle(&p.bbox());
            let goal = [goal[0] + e.offset[0], goal[1] + e.offset[1]];
            let front = [key[0] + e.offset[0], key[1] + e.offset[1]];
            let (_, _, first) = fit(v, front, p.door[1], facing);
            let m = middle(&first);
            let front = [front[0] + goal[0] - m[0], front[1] + goal[1] - m[1]];
            (front, height(ground, front[0], front[1]).0)
        };
        let (origin, turn, bx) = fit(v, front, floor, facing);
        *p = Placement {
            kind: t.name.clone(),
            file: v.file.clone(),
            origin,
            turn,
            min: bx.min,
            max: bx.max,
            door: crate::geom::place(v.door, origin, turn),
            facing,
            size: v.class,
            villagers: if t.has_villagers() { villagers } else { 0 },
            key: p.key,
        };
        moved.push(i);
    }
    // A farm taken away takes its field.
    let mut index = Vec::with_capacity(keep.len());
    let mut n = 0;
    for k in &keep {
        index.push(n);
        n += *k as usize;
    }
    b.fields.retain(|f| keep.get(f.farm).copied().unwrap_or(true));
    for f in &mut b.fields {
        f.farm = index.get(f.farm).copied().unwrap_or(f.farm);
    }
    let mut k = 0;
    let moved_keys: HashSet<Option<[i32; 2]>> = moved.iter().map(|&i| b.placed[i].key).collect();
    b.placed.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    // A path from each door moved off its road to the nearest one.
    let boxes: Vec<BBox> = b.placed.iter().map(|p| p.bbox()).collect();
    for p in b.placed.iter().filter(|p| moved_keys.contains(&p.key)) {
        let cells = roads.cells(area);
        let front = [p.door[0] + p.facing.step()[0], p.door[2] + p.facing.step()[1]];
        if [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]].iter().any(|d| cells.contains_key(&(front[0] + d[0], front[1] + d[1]))) {
            continue;
        }
        match path(front, &cells, &boxes, area, ground) {
            Some(mut points) => {
                // From the road to the door: what keeps it when side
                // roads are cut after their last door.
                points.reverse();
                roads.lines.push(Road { points, kind: RoadKind::Side, width: 1 });
            }
            None => notes.push(format!("No way was found from the door of the {} at {}, {} to a road", p.kind, front[0], front[1])),
        }
    }
    b.heights = levelled(&b.placed, &b.fields, ground);
    notes
}

/// The shortest way from `from` to a road (`cells`), around the buildings
/// (`boxes`, with a block to spare), over dry land of the area not too
/// steep, within 64 blocks: its columns, `from` first.
fn path(from: [i32; 2], cells: &HashMap<(i32, i32), RoadKind>, boxes: &[BBox], area: &Area, ground: &dyn Ground) -> Option<Vec<[i32; 2]>> {
    let blocked = |x: i32, z: i32| boxes.iter().any(|b| x >= b.min[0] - 1 && x <= b.max[0] + 1 && z >= b.min[2] - 1 && z <= b.max[2] + 1);
    let mut prev: HashMap<[i32; 2], [i32; 2]> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    prev.insert(from, from);
    while let Some(c) = queue.pop_front() {
        if cells.contains_key(&(c[0], c[1])) {
            let mut out = vec![c];
            let mut at = c;
            while at != from {
                at = prev[&at];
                out.push(at);
            }
            out.reverse();
            // The road's own column is the road's.
            out.pop();
            return (!out.is_empty()).then_some(out);
        }
        let (h, _) = height(ground, c[0], c[1]);
        for d in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
            let n = [c[0] + d[0], c[1] + d[1]];
            if prev.contains_key(&n) || (n[0] - from[0]).abs() > 64 || (n[1] - from[1]).abs() > 64 || !area.contains(n[0], n[1]) {
                continue;
            }
            let road = cells.contains_key(&(n[0], n[1]));
            let (nh, wet) = height(ground, n[0], n[1]);
            if !road && (blocked(n[0], n[1]) || wet || (nh - h).abs() > 1) {
                continue;
            }
            prev.insert(n, c);
            queue.push_back(n);
        }
    }
    None
}
