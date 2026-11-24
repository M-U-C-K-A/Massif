//! Stage 5, the buildings: on each plot that has room, a building of the
//! theme — its type by the theme's rules, a variant of the size the plot
//! likes, turned so its entrance faces the road — the land levelled under
//! it, and a field beside a farm. For the game's themes, its villages
//! assembled by the jigsaw engine in each district ([`game_districts`]).

use super::pack::{BuildingType, SizeClass, Variant};
use super::plots::{Plot, class_weight};
use super::roads::{Cells, Roads};
use super::site::Site;
use super::{Area, MillMode, Params, Side, Theme, dist2, height};
use crate::build;
use crate::geom::{BBox, turn_xz};
use crate::ground::Ground;
use crate::jigsaw::Placed;
use crate::library::Library;
use crate::rng::Rng;
use crate::vanilla;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A schematic placed: what it is, where, turned how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Its type (`house`, `church`), or `prop:<name>` for a prop.
    pub kind: String,
    pub file: PathBuf,
    /// Where its corner (0, 0, 0) goes.
    pub origin: [i32; 3],
    /// Quarter turns clockwise.
    pub turn: u8,
    /// Its box on the map, the ends included.
    pub min: [i32; 3],
    pub max: [i32; 3],
    /// Its door's lower half on the map.
    pub door: [i32; 3],
    /// The side its door faces, on the map.
    pub facing: Side,
    pub size: SizeClass,
    /// Villagers to put inside its door.
    pub villagers: u32,
}

impl Placement {
    pub fn bbox(&self) -> BBox {
        BBox { min: self.min, max: self.max }
    }
}

/// A field of crops beside a farm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// x0, z0, x1, z1, the ends included.
    pub rect: [i32; 4],
    /// The height of its farmland's top + 1 (where the crops stand).
    pub floor: i32,
    pub crop: String,
    /// The farm it belongs to (its index in [`Buildings::placed`]).
    pub farm: usize,
}

/// The buildings of a village.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Buildings {
    pub placed: Vec<Placement>,
    pub fields: Vec<Field>,
    /// The land levelled under them: each column's new height (the first
    /// free block over it).
    pub heights: Vec<([i32; 2], i32)>,
    /// What could not be used, in plain words.
    pub notes: Vec<String>,
}

impl Buildings {
    pub fn height_map(&self) -> HashMap<(i32, i32), i32> {
        self.heights.iter().map(|&([x, z], h)| ((x, z), h)).collect()
    }
}

/// The game's villages assembled in each district.
pub struct Jigsaw {
    pub pieces: Vec<Vec<Placed>>,
    /// Their buildings' columns: roads keep out of them.
    pub taken: Cells,
    /// Their streets' columns.
    pub streets: Vec<(i32, i32)>,
    pub heights: HashMap<(i32, i32), i32>,
}

/// The game's village `kind` (`village_plains`) assembled at each square
/// of `site`, within the area.
pub fn game_districts(lib: &Library, ground: &dyn Ground, area: &Area, site: &Site, kind: &str, seed: u64) -> Result<Jigsaw, String> {
    let mut rng = Rng::new(seed ^ 0x7669_6c6c);
    let kind = vanilla::kind(kind).ok_or("an unknown village")?;
    let vanilla::Recipe::Jigsaw(id) = kind.recipe else { return Err("not a jigsaw village".into()) };
    let (spec, game_height, _, _) = vanilla::jigsaw_spec(lib, id).ok_or("the game's files have no villages")?;
    let mut boxes: Vec<BBox> = Vec::new();
    let mut pieces = Vec::new();
    for &c in &site.centres {
        let within = |b: &BBox| [(b.min[0], b.min[2]), (b.max[0], b.min[2]), (b.min[0], b.max[2]), (b.max[0], b.max[2])].iter().all(|&(x, z)| area.contains(x, z)) && !boxes.iter().any(|o: &BBox| o.overlaps(b));
        let mut sub = rng.fork();
        let p = crate::jigsaw::assemble_within(lib, &spec, [c[0], game_height, c[1]], None, &mut sub, ground, &within);
        boxes.extend(p.iter().map(|p| p.bbox));
        pieces.push(p);
    }
    let all: Vec<_> = pieces.iter().flatten().cloned().collect();
    let heights = build::flatten_under(&all, ground, 4);
    let mut taken = Cells::new();
    let mut streets = Vec::new();
    for p in &all {
        let street = p.element.projection() == crate::pool::Projection::TerrainMatching;
        for z in p.bbox.min[2]..=p.bbox.max[2] {
            for x in p.bbox.min[0]..=p.bbox.max[0] {
                if street {
                    streets.push((x, z));
                } else {
                    taken.insert((x, z));
                }
            }
        }
    }
    Ok(Jigsaw { pieces, taken, streets, heights })
}

/// The quarter turns that bring a schematic's entrance, on side
/// `entrance`, round to face `to`.
pub fn turn_for(entrance: Side, to: Side) -> u8 {
    (to.quarter() + 4 - entrance.quarter()) % 4
}

/// Where variant `v` goes so that its door stands at `door` (a column, its
/// floor `floor`) facing `facing`: its origin, turn and box.
pub fn fit(v: &Variant, door: [i32; 2], floor: i32, facing: Side) -> ([i32; 3], u8, BBox) {
    let turn = turn_for(v.entrance, facing);
    let (dx, dz) = turn_xz(v.door[0], v.door[2], turn);
    let origin = [door[0] - dx, floor - v.door[1], door[1] - dz];
    (origin, turn, BBox::of_template(v.size, origin, turn))
}

/// Level the land under each box to its floor (the first free block over
/// it), with a slope of `margin` blocks to the land around.
pub fn level_under(boxes: impl Iterator<Item = (BBox, i32)>, ground: &dyn Ground, margin: i32) -> HashMap<(i32, i32), i32> {
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

/// Whether there is water within `r` blocks of `(x, z)`.
fn water_near(ground: &dyn Ground, x: i32, z: i32, r: i32) -> bool {
    (-r..=r).step_by(2).any(|dz| (-r..=r).step_by(2).any(|dx| height(ground, x + dx, z + dz).1))
}

/// Draw one of `items` by weight.
fn draw<T>(rng: &mut Rng, items: &[(T, f32)]) -> Option<usize> {
    let sum: f32 = items.iter().map(|w| w.1).sum();
    if items.is_empty() || sum <= 0.0 {
        return None;
    }
    let mut pick = rng.float() * sum;
    for (k, w) in items.iter().enumerate() {
        if pick < w.1 {
            return Some(k);
        }
        pick -= w.1;
    }
    Some(items.len() - 1)
}

/// Stage 5: the buildings of a pack's theme on `plots`. `reserved`: what
/// they keep out of (the walls), `blocked`: what is built already.
#[allow(clippy::too_many_arguments)]
pub fn buildings(ground: &dyn Ground, area: &Area, site: &Site, roads: &Roads, reserved: &Cells, blocked: &Cells, plots: &[Plot], theme: &Theme, params: &Params, seed: u64) -> Buildings {
    let mut out = Buildings::default();
    if theme.vanilla.is_some() {
        return out;
    }
    let mut rng = Rng::new(seed ^ 0x6275_696c);
    let road_cells = roads.cells(area);
    let mut taken: Cells = reserved.union(blocked).copied().collect();
    let mut counts: HashMap<(u16, String), u32> = HashMap::new();
    let mut floors: Vec<i32> = Vec::new();
    let mut dwellings = 0u32;
    let want = params.houses.map(|n| n.clamp(3, 120));
    let bias = params.size_bias.clamp(-1.0, 1.0);
    let total_share: f32 = theme.types.iter().map(|t| t.share).sum::<f32>().max(0.001);
    let villagers = params.villagers.unwrap_or(theme.villagers);
    // The median height of the squares: what "high ground" is above.
    let median = {
        let mut h: Vec<i32> = site.centres.iter().map(|c| ground.floor(c[0], c[1])).collect();
        h.sort();
        h[h.len() / 2]
    };
    let edge_from = (site.reach as f32 * 0.45).max(24.0);
    for plot in plots {
        if want.is_some_and(|n| dwellings >= n) && theme.types.iter().all(|t| t.is_dwelling() || t.max.is_none()) {
            break;
        }
        let (floor, wet) = height(ground, plot.door[0], plot.door[1]);
        if wet {
            continue;
        }
        // A type, by share, among those this plot suits.
        let mut weights: Vec<(usize, f32)> = Vec::new();
        for (i, t) in theme.types.iter().enumerate() {
            let n = counts.get(&(plot.district, t.name.clone())).copied().unwrap_or(0);
            if t.max.is_some_and(|m| n >= m) || (t.centre && plot.from_centre > 48.0) || (t.edge && plot.from_centre < edge_from) {
                continue;
            }
            if t.is_dwelling() && want.is_some_and(|w| dwellings >= w) {
                continue;
            }
            if let Some(near) = &t.near
                && theme.kind(near).is_some()
                && !out.placed.iter().any(|p| &p.kind == near && dist2([p.door[0], p.door[2]], plot.door) < 56 * 56)
            {
                continue;
            }
            let mut share = t.share;
            if t.name == "mill" {
                if params.mill == MillMode::None || out.placed.iter().any(|p| p.kind == "mill") {
                    continue;
                }
                // One mill: as likely as a farm, once.
                share = share.max(0.08);
            }
            // Those that keep to the middle come first there.
            let mut w = share / total_share * if t.centre && n == 0 { 20.0 } else { 1.0 };
            // Manors and large buildings as the bias asks; a manor on a
            // height.
            if t.name == "manor" {
                w *= (bias * 1.2).exp();
            }
            if t.high_ground {
                w *= 1.0 + ((floor - median) as f32 / 6.0).clamp(0.0, 3.0);
            }
            if w > 0.0 {
                weights.push((i, w));
            }
        }
        let Some(k) = draw(&mut rng, &weights) else { continue };
        let t: &BuildingType = &theme.types[weights[k].0];
        // A variant: the plot's size first, then as the bias weighs.
        let water = water_near(ground, plot.door[0], plot.door[1], 10);
        let variants: Vec<(&Variant, f32)> = t
            .variants
            .iter()
            .filter(|v| {
                t.name != "mill"
                    || match params.mill {
                        MillMode::Water => v.water && water,
                        MillMode::Wind => !v.water,
                        _ => !v.water || water,
                    }
            })
            .map(|v| (v, v.weight * class_weight(v.class, bias) * if v.class == plot.size { 3.0 } else { 1.0 }))
            .collect();
        let Some(k) = draw(&mut rng, &variants) else { continue };
        let v = variants[k].0;
        let facing = plot.side.opposite();
        let (origin, turn, b) = fit(v, plot.door, floor, facing);
        // On free land of the area, apart from the others.
        let margin = params.margin.clamp(0, 8) + rng.between(0, 2);
        let mut ok = true;
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        'cells: for z in b.min[2] - margin..=b.max[2] + margin {
            for x in b.min[0] - margin..=b.max[0] + margin {
                let inner = x >= b.min[0] && x <= b.max[0] && z >= b.min[2] && z <= b.max[2];
                if taken.contains(&(x, z)) || (inner && (!area.contains(x, z) || road_cells.contains_key(&(x, z)))) {
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
        if !ok || hi - lo > params.max_slope.max(1) {
            continue;
        }
        for z in b.min[2] - margin..=b.max[2] + margin {
            for x in b.min[0] - margin..=b.max[0] + margin {
                taken.insert((x, z));
            }
        }
        // A farm's field, behind it (away from the road), on free dry land.
        if let Some(spec) = &t.field
            && rng.chance(params.fields.clamp(0.0, 1.0))
        {
            let (lo_s, hi_s) = (spec.size[0].clamp(4, 64), spec.size[1].clamp(4, 64).max(spec.size[0].clamp(4, 64)));
            let (fw, fd) = (rng.between(lo_s, hi_s), rng.between(lo_s, hi_s));
            // Behind it (away from the road), else beside it; smaller if
            // need be, down to the least size.
            let back = plot.side;
            let sides = [back, if matches!(back, Side::North | Side::South) { Side::East } else { Side::North }, if matches!(back, Side::North | Side::South) { Side::West } else { Side::South }];
            let rect_on = |side: Side, fw: i32, fd: i32| -> [i32; 4] {
                match side {
                    Side::North => [b.min[0], b.min[2] - 2 - fd, b.min[0] + fw - 1, b.min[2] - 3],
                    Side::South => [b.min[0], b.max[2] + 3, b.min[0] + fw - 1, b.max[2] + 2 + fd],
                    Side::East => [b.max[0] + 3, b.min[2], b.max[0] + 2 + fd, b.min[2] + fw - 1],
                    Side::West => [b.min[0] - 2 - fd, b.min[2], b.min[0] - 3, b.min[2] + fw - 1],
                }
            };
            let fits = |rect: [i32; 4]| -> Option<i32> {
                let mut hs = Vec::new();
                for z in rect[1] - 1..=rect[3] + 1 {
                    for x in rect[0] - 1..=rect[2] + 1 {
                        let (h, wet) = height(ground, x, z);
                        if !area.contains(x, z) || taken.contains(&(x, z)) || road_cells.contains_key(&(x, z)) || wet {
                            return None;
                        }
                        hs.push(h);
                    }
                }
                hs.sort();
                (hs[hs.len() - 1] - hs[0] <= 4).then(|| hs[hs.len() / 2])
            };
            let found = sides.iter().flat_map(|&side| [(fw, fd), ((fw + lo_s) / 2, (fd + lo_s) / 2), (lo_s, lo_s)].map(move |(w, d)| (side, w, d))).find_map(|(side, w, d)| {
                let r = rect_on(side, w, d);
                fits(r).map(|floor| (r, floor))
            });
            if let Some((rect, floor)) = found {
                for z in rect[1] - 1..=rect[3] + 1 {
                    for x in rect[0] - 1..=rect[2] + 1 {
                        taken.insert((x, z));
                    }
                }
                let crop = rng.pick(&spec.crops).cloned().unwrap_or_else(|| "wheat".into());
                out.fields.push(Field { rect, floor, crop, farm: out.placed.len() });
            }
        }
        *counts.entry((plot.district, t.name.clone())).or_default() += 1;
        if t.is_dwelling() {
            dwellings += 1;
        }
        floors.push(floor);
        let door = crate::geom::place(v.door, origin, turn);
        out.placed.push(Placement {
            kind: t.name.clone(),
            file: v.file.clone(),
            origin,
            turn,
            min: b.min,
            max: b.max,
            door,
            facing,
            size: v.class,
            villagers: if t.has_villagers() { villagers } else { 0 },
        });
    }
    // The land under each, to its floor; the fields to theirs.
    let mut heights = level_under(out.placed.iter().zip(&floors).map(|(p, f)| (p.bbox(), *f)), ground, 4);
    for f in &out.fields {
        for z in f.rect[1] - 1..=f.rect[3] + 1 {
            for x in f.rect[0] - 1..=f.rect[2] + 1 {
                if ground.floor(x, z) != f.floor {
                    heights.insert((x, z), f.floor);
                } else {
                    heights.remove(&(x, z));
                }
            }
        }
    }
    let mut hv: Vec<([i32; 2], i32)> = heights.into_iter().map(|((x, z), h)| ([x, z], h)).collect();
    hv.sort();
    out.heights = hv;
    if let Some(n) = want
        && dwellings < n
    {
        out.notes.push(format!("{dwellings} of the {n} houses asked for: the area has no room for more"));
    }
    out
}
