//! Why a stroke left columns looking as they were: a ground cover lying
//! over the terrain painted, a level standing over the one edited, a locked
//! layer, the brush's filter, a layer hidden on the map. A stroke notes a
//! few columns of each dab on a grid of the map ([`Probe::dab`]); once it
//! ends, [`Probe::verdict`] reads them and the map says the commonest
//! reason, as it says a stroke all outside the selection does nothing. The
//! Info panel says the same of the column under the cursor
//! ([`blocked_at`]).

use massif_core::coords::local_index;
use massif_core::filter::Filter;
use massif_core::level::LevelKind;
use massif_core::registry::{Locks, Registry, Slot, TerrainId};
use massif_core::{Dimension, TilePos};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What a stroke paints, as far as telling why it shows no change goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    /// A terrain: the one given, or (by zones, a pattern, the one put
    /// back) whichever the column gets.
    Terrain(Option<TerrainId>),
    /// A ground cover, by its number from 1.
    Cover(u16),
    /// An object layer, by its number from 1, base or sub-canopy.
    Objects { layer: u16, understory: bool },
}

/// Why a column under a stroke shows no change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blocked {
    /// The ground cover numbered so lies over the terrain painted.
    Covered(u16),
    /// The level of that index stands over the one painted.
    Level(usize),
    /// The brush's filter leaves it.
    Filter,
    /// A locked ground cover keeps its blocks.
    LockedCover(u16),
    /// A locked object layer keeps its objects.
    LockedObjects(u16),
    /// The layer painted is hidden on the map.
    Hidden,
}

/// What a column under a stroke does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It shows the paint (or is not the stroke's to paint).
    Fine,
    /// It shows no change, for this reason.
    Blocked(Blocked),
    /// It did not take the paint, for no reason of the map's own: the
    /// brush's strength, its filter, its shape.
    Unchanged,
}

/// What column `(x, z)` of level `level` does under a stroke that paints
/// `aim`, as the map stands.
pub fn blocked_at(dim: &Dimension, reg: &Registry, locks: &Locks, level: usize, aim: Aim, x: i32, z: i32) -> Outcome {
    let pos = TilePos::containing(x, z);
    let Some(edited) = dim.levels.get(level) else { return Outcome::Fine };
    let Some(tile) = edited.view(pos) else { return Outcome::Fine };
    let i = local_index(x, z);
    if edited.kind != LevelKind::Ground && !tile.present(i) {
        return Outcome::Fine;
    }
    match aim {
        Aim::Terrain(target) => {
            if target.is_some_and(|t| tile.terrain.get(i) != t.0) {
                return Outcome::Unchanged;
            }
            // A level over this one: the map and the game show its top.
            let top = massif_core::compose::block_top(tile.top.get(i));
            for (k, l) in dim.levels.iter().enumerate().skip(level + 1) {
                if l.kind == LevelKind::Solid
                    && let Some(t) = l.view(pos)
                    && t.present(i)
                    && massif_core::compose::block_top(t.top.get(i)) > top
                {
                    return Outcome::Blocked(Blocked::Level(k));
                }
            }
            // A cover that lays blocks over the ground (WorldPainter's
            // custom ground covers: sand, gravel, layers): the terrain is
            // under them, on the map and in the game. Plants only tint it.
            let n = tile.cover.get(i);
            if let Some(def) = n.checked_sub(1).and_then(|k| reg.covers.get(k as usize))
                && !def.removed
                && !def.hidden
                && def.is_laid()
            {
                return Outcome::Blocked(Blocked::Covered(n));
            }
            Outcome::Fine
        }
        Aim::Cover(c) => {
            let here = tile.cover.get(i);
            if here != c {
                let stacked = tile.stacked.has(i) && tile.stacked.entries(i).any(|e| e.kind == massif_core::layer_stack::Kind::Cover && locks.holds(Slot::Cover, e.layer));
                if locks.holds(Slot::Cover, here) || stacked {
                    let held = if locks.holds(Slot::Cover, here) { here } else { tile.stacked.entries(i).find(|e| e.kind == massif_core::layer_stack::Kind::Cover && locks.holds(Slot::Cover, e.layer)).map_or(here, |e| e.layer) };
                    return Outcome::Blocked(Blocked::LockedCover(held));
                }
                return Outcome::Unchanged;
            }
            if c != 0 && c.checked_sub(1).and_then(|k| reg.covers.get(k as usize)).is_some_and(|d| d.hidden) {
                return Outcome::Blocked(Blocked::Hidden);
            }
            Outcome::Fine
        }
        Aim::Objects { layer, understory } => {
            let here = if understory { tile.understory.get(i) } else { tile.objects.get(i) };
            if here != layer {
                if locks.holds(Slot::Objects, here) {
                    return Outcome::Blocked(Blocked::LockedObjects(here));
                }
                return Outcome::Unchanged;
            }
            if layer != 0 && reg.object_layer(layer).is_some_and(|l| l.hidden) {
                return Outcome::Blocked(Blocked::Hidden);
            }
            Outcome::Fine
        }
    }
}

/// Columns noted at most by a stroke: enough to tell a share by.
const MAX_SAMPLES: usize = 4096;
/// Columns whose filter is read at most, of those left unchanged.
const MAX_FILTERED: usize = 256;

/// The columns a stroke went over, to tell afterwards why some show no
/// change.
#[derive(Debug, Clone)]
pub struct Probe {
    aim: Aim,
    level: usize,
    filter: Option<Arc<Filter>>,
    seen: HashSet<(i32, i32)>,
}

impl Probe {
    pub fn new(aim: Aim, level: usize, filter: Option<Arc<Filter>>) -> Self {
        Self { aim, level, filter, seen: HashSet::new() }
    }

    pub fn aim(&self) -> Aim {
        self.aim
    }

    /// Note the columns well inside a dab of `radius` at `(cx, cz)`: those
    /// of a grid of the map, a few a dab whatever its size.
    pub fn dab(&mut self, cx: f32, cz: f32, radius: f32) {
        if self.seen.len() >= MAX_SAMPLES {
            return;
        }
        let step = (radius / 3.0).floor().max(1.0) as i32;
        let r = (radius * 0.7).max(0.5);
        let grid = |a: f32| (a.floor() as i32).div_euclid(step);
        for gz in grid(cz - r)..=grid(cz + r) {
            for gx in grid(cx - r)..=grid(cx + r) {
                let (x, z) = (gx * step, gz * step);
                let (dx, dz) = (x as f32 + 0.5 - cx, z as f32 + 0.5 - cz);
                if dx * dx + dz * dz <= r * r {
                    self.seen.insert((x, z));
                }
            }
        }
    }

    /// The commonest reason the stroke's columns show no change, and the
    /// share of its columns it holds, when it holds enough of them to say.
    pub fn verdict(&self, dim: &Dimension, reg: &Registry) -> Option<(Blocked, f32)> {
        let locks = Locks::of(reg);
        let mut total = 0usize;
        let mut counts: HashMap<Blocked, usize> = HashMap::new();
        let mut unchanged = Vec::new();
        for &(x, z) in &self.seen {
            // Outside the selection the stroke says so itself.
            if dim.selection.as_ref().is_some_and(|s| s.get(x, z) < massif_core::selection::HALF) {
                continue;
            }
            total += 1;
            match blocked_at(dim, reg, &locks, self.level, self.aim, x, z) {
                Outcome::Blocked(b) => *counts.entry(b).or_default() += 1,
                Outcome::Unchanged => unchanged.push((x, z)),
                Outcome::Fine => {}
            }
        }
        // Those left unchanged: by the filter where it weighs nothing (read
        // on a few of them, the share counted for all).
        if let Some(f) = &self.filter
            && !unchanged.is_empty()
        {
            let stride = unchanged.len().div_ceil(MAX_FILTERED);
            let read: Vec<_> = unchanged.iter().step_by(stride).collect();
            let out = read.iter().filter(|&&&(x, z)| f.mask(dim, massif_core::coords::BlockRect::new(x, z, x + 1, z + 1)).get(x, z) <= 0.0).count();
            let n = out * unchanged.len() / read.len();
            if n > 0 {
                counts.insert(Blocked::Filter, n);
            }
        }
        let (b, n) = counts.into_iter().max_by_key(|&(b, n)| (n, b == Blocked::Hidden))?;
        let share = n as f32 / total.max(1) as f32;
        (n >= 3 && share >= 0.03).then_some((b, share))
    }
}

/// What the map says of `blocked` holding `share` of a stroke.
pub fn say(blocked: Blocked, share: f32, dim: &Dimension, reg: &Registry, aim: Aim) -> String {
    let pct = ((share * 100.0).round() as i32).clamp(1, 100);
    let cover = |n: u16| n.checked_sub(1).and_then(|k| reg.covers.get(k as usize)).map_or_else(String::new, |c| crate::i18n::tr_name(&c.name).to_owned());
    match blocked {
        Blocked::Covered(n) => tr!("{pct}% of the stroke shows no change: the ground cover {name} lies over the terrain there. Rub it out with the Ground cover tool (right button), or hide it in the Layers panel to see the terrain under it.", pct, name = cover(n)),
        Blocked::Level(k) => tr!("{pct}% of the stroke shows no change: the level {name} stands over the one painted there. Pick it in the Levels panel to paint it.", pct, name = dim.levels.get(k).map_or("", |l| l.name.as_str())),
        Blocked::Filter => tr!("{pct}% of the stroke was left as it was by the brush's filter (Only on, Except on, heights, slopes): see Tool Settings.", pct),
        Blocked::LockedCover(n) => tr!("{pct}% of the stroke was left as it was: the layer {name} is locked there. Unlock it in the Layers panel to paint over it.", pct, name = cover(n)),
        Blocked::LockedObjects(n) => tr!("{pct}% of the stroke was left as it was: the layer {name} is locked there. Unlock it in the Layers panel to paint over it.", pct, name = reg.object_layer(n).map_or_else(String::new, |l| l.name.clone())),
        Blocked::Hidden => {
            let name = match aim {
                Aim::Cover(c) => cover(c),
                Aim::Objects { layer, .. } => reg.object_layer(layer).map_or_else(String::new, |l| l.name.clone()),
                Aim::Terrain(_) => String::new(),
            };
            tr!("The layer {name} is hidden on the map: it is painted all the same, and shows once shown again in the Layers panel.", name)
        }
    }
}

/// What the Info panel says of the column at `(x, z)` for the tool in
/// hand, when it would not show that tool's paint.
pub fn note_at(dim: &Dimension, reg: &Registry, level: usize, aim: Aim, x: i32, z: i32) -> Option<String> {
    let cover = |n: u16| n.checked_sub(1).and_then(|k| reg.covers.get(k as usize)).map_or_else(String::new, |c| crate::i18n::tr_name(&c.name).to_owned());
    match blocked_at(dim, reg, &Locks::of(reg), level, aim, x, z) {
        Outcome::Blocked(Blocked::Covered(n)) => Some(tr!("The ground cover {name} lies over the terrain here: a terrain painted here stays under it.", name = cover(n))),
        Outcome::Blocked(Blocked::Level(k)) => Some(tr!("The level {name} stands over the one painted here.", name = dim.levels.get(k).map_or("", |l| l.name.as_str()))),
        Outcome::Blocked(Blocked::LockedCover(n)) => Some(tr!("The layer {name} is locked here: brushes leave it as it is.", name = cover(n))),
        Outcome::Blocked(Blocked::LockedObjects(n)) => Some(tr!("The layer {name} is locked here: brushes leave it as it is.", name = reg.object_layer(n).map_or_else(String::new, |l| l.name.clone()))),
        _ => None,
    }
}
