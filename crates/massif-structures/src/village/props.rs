//! Stage 6, the props: market stalls round the squares, lampposts and
//! benches along the roads, hay and carts by the farms, fences round the
//! fields. The pack's own (`props/`, `market/`) when it has them, Massif's
//! simple ones otherwise.

use super::buildings::{Buildings, Placement, fit};
use super::roads::{Cells, RoadKind, Roads};
use super::site::Site;
use super::{Area, Params, Side, Theme, height};
use crate::ground::Ground;
use crate::rng::Rng;
use crate::vanilla::Adapted;
use serde::{Deserialize, Serialize};

/// One of Massif's own props, built by [`super::lay`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Builtin {
    /// `lamppost`, `bench`, `hay_bales`, `cart`, `fence`.
    pub kind: String,
    /// The column it stands on, at its floor (the first free block).
    pub at: [i32; 3],
    pub facing: Side,
    /// For a fence: the rectangle it runs round (x0, z0, x1, z1, ends
    /// included), its gate on side `facing`.
    pub rect: Option<[i32; 4]>,
}

/// A village's props.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Props {
    /// The pack's: stalls and props from its schematics.
    pub placed: Vec<Placement>,
    pub builtin: Vec<Builtin>,
}

/// What is free for a prop: in the area, dry, off the roads, clear of the
/// buildings, the fields and the walls.
struct Free<'a> {
    area: &'a Area,
    ground: &'a dyn Ground,
    taken: Cells,
}

impl Free<'_> {
    fn ok(&self, x: i32, z: i32) -> bool {
        self.area.contains(x, z) && !self.taken.contains(&(x, z)) && !height(self.ground, x, z).1
    }

    fn rect_ok(&self, min: [i32; 3], max: [i32; 3]) -> bool {
        (min[2]..=max[2]).all(|z| (min[0]..=max[0]).all(|x| self.ok(x, z)))
    }

    fn take(&mut self, min: [i32; 3], max: [i32; 3], margin: i32) {
        for z in min[2] - margin..=max[2] + margin {
            for x in min[0] - margin..=max[0] + margin {
                self.taken.insert((x, z));
            }
        }
    }
}

/// Stage 6: the props, as many as `params.props` asks (none at 0).
#[allow(clippy::too_many_arguments)]
pub fn props(ground: &dyn Ground, area: &Area, site: &Site, roads: &Roads, reserved: &Cells, buildings: &Buildings, theme: &Theme, params: &Params, seed: u64) -> Props {
    let mut out = Props::default();
    let amount = params.props.clamp(0.0, 1.0);
    if theme.vanilla.is_some() {
        return out;
    }
    let mut rng = Rng::new(seed ^ 0x7072_6f70);
    let levelled = Adapted { inner: ground, heights: buildings.height_map() };
    let road_cells = roads.cells(area);
    let mut free = Free { area, ground: &levelled, taken: reserved.clone() };
    free.taken.extend(road_cells.keys().copied());
    for p in &buildings.placed {
        free.take(p.min, p.max, 1);
        // The way in stays clear.
        let s = p.facing.step();
        for k in 1..=2 {
            free.taken.insert((p.door[0] + s[0] * k, p.door[2] + s[1] * k));
        }
    }
    for f in &buildings.fields {
        free.take([f.rect[0], 0, f.rect[1]], [f.rect[2], 0, f.rect[3]], 1);
    }
    let floor = |x: i32, z: i32| levelled.floor(x, z);

    // Market stalls round each square, facing it (a market is the square's
    // whatever the props setting).
    if let Some(market) = theme.kind("market").filter(|t| t.share <= 0.0) {
        let max = market.max.unwrap_or(4) as usize;
        for &c in &site.centres {
            let mut n = 0;
            'sides: for side in Side::ALL {
                for along in [0, -5, 5] {
                    if n >= max {
                        break 'sides;
                    }
                    let s = side.step();
                    let door = [c[0] + s[0] * 5 + s[1].abs() * along, c[1] + s[1] * 5 + s[0].abs() * along];
                    let Some(v) = rng.pick(&market.variants) else { break 'sides };
                    let (origin, turn, b) = fit(v, door, floor(door[0], door[1]), side.opposite());
                    // Stalls stand on the square's edge, not on its roads.
                    let clear = (b.min[2]..=b.max[2]).all(|z| (b.min[0]..=b.max[0]).all(|x| area.contains(x, z) && !free.taken.contains(&(x, z)) || road_cells.get(&(x, z)) == Some(&RoadKind::Square)));
                    if !clear {
                        continue;
                    }
                    free.take(b.min, b.max, 1);
                    let d = crate::geom::place(v.door, origin, turn);
                    out.placed.push(Placement { kind: "market".into(), file: v.file.clone(), origin, turn, min: b.min, max: b.max, door: d, facing: side.opposite(), size: v.class, villagers: 0 });
                    n += 1;
                }
            }
        }
    }
    if amount <= 0.0 {
        return out;
    }
    // A prop: the pack's if it has one by that name, else Massif's.
    let put = |out: &mut Props, free: &mut Free, rng: &mut Rng, kind: &str, at: [i32; 2], facing: Side| -> bool {
        let vs = theme.prop(kind);
        if let Some(v) = rng.pick(vs) {
            let (origin, turn, b) = fit(v, at, floor(at[0], at[1]), facing);
            if !free.rect_ok(b.min, b.max) {
                return false;
            }
            free.take(b.min, b.max, 1);
            let d = crate::geom::place(v.door, origin, turn);
            out.placed.push(Placement { kind: format!("prop:{kind}"), file: v.file.clone(), origin, turn, min: b.min, max: b.max, door: d, facing, size: v.class, villagers: 0 });
            return true;
        }
        // Massif's: a lamppost of one column, a bench of three, hay and
        // carts of two by two.
        let (w, d) = match kind {
            "lamppost" => (1, 1),
            "bench" => (3, 1),
            "hay_bales" | "cart" => (2, 2),
            _ => return false,
        };
        let (w, d) = if matches!(facing, Side::East | Side::West) { (d, w) } else { (w, d) };
        let min = [at[0] - (w - 1) / 2, 0, at[1] - (d - 1) / 2];
        let max = [min[0] + w - 1, 0, min[2] + d - 1];
        if !free.rect_ok(min, max) {
            return false;
        }
        free.take(min, max, 1);
        out.builtin.push(Builtin { kind: kind.into(), at: [at[0], floor(at[0], at[1]), at[1]], facing, rect: None });
        true
    };
    // Along the roads: every 14 (many) to 40 (few) blocks, either side in
    // turn.
    let every = (40.0 - 26.0 * amount) as usize;
    let along = &theme.prop_rules.along_roads;
    if !along.is_empty() {
        for (k, r) in roads.lines.iter().filter(|r| r.kind != RoadKind::Square).enumerate() {
            let mut i = every / 2 + k % 3;
            let mut flip = rng.chance(0.5);
            while i + 1 < r.points.len() {
                let (p, q) = (r.points[i], r.points[i + 1]);
                let along_x = (q[0] - p[0]).abs() >= (q[1] - p[1]).abs();
                let side = match (along_x, flip) {
                    (true, true) => Side::North,
                    (true, false) => Side::South,
                    (false, true) => Side::West,
                    (false, false) => Side::East,
                };
                let s = side.step();
                let off = (r.width - 1) / 2 + 1 + (side == Side::South || side == Side::East) as i32 * ((r.width + 1) % 2);
                let at = [p[0] + s[0] * off, p[1] + s[1] * off];
                // Lampposts mostly, a bench now and then; facing the road.
                let kind = if along.len() > 1 && rng.chance(0.25) { &along[1 + rng.below(along.len() as u32 - 1) as usize] } else { &along[0] };
                put(&mut out, &mut free, &mut rng, kind, at, side.opposite());
                flip = !flip;
                i += every;
            }
        }
    }
    // By the buildings the rules name: one or two, beside them.
    for p in buildings.placed.clone() {
        let Some((_, kinds)) = theme.prop_rules.near.iter().find(|(t, _)| *t == p.kind) else { continue };
        if kinds.is_empty() || !rng.chance(0.4 + 0.6 * amount) {
            continue;
        }
        let n = 1 + rng.chance(amount) as usize;
        let sides: Vec<Side> = Side::ALL.into_iter().filter(|s| *s != p.facing && *s != p.facing.opposite()).collect();
        for k in 0..n {
            let side = sides[k % sides.len()];
            let s = side.step();
            // Three blocks off that side, by its middle.
            let mid = [(p.min[0] + p.max[0]) / 2, (p.min[2] + p.max[2]) / 2];
            let reach = if s[0] != 0 { (p.max[0] - p.min[0]) / 2 + 3 } else { (p.max[2] - p.min[2]) / 2 + 3 };
            let at = [mid[0] + s[0] * reach, mid[1] + s[1] * reach];
            let kind = kinds[rng.below(kinds.len() as u32) as usize].clone();
            put(&mut out, &mut free, &mut rng, &kind, at, p.facing);
        }
    }
    // Fences round the fields, their gate toward the farm.
    for f in &buildings.fields {
        let farm = &buildings.placed[f.farm];
        let mid = [(farm.min[0] + farm.max[0]) / 2, (farm.min[2] + farm.max[2]) / 2];
        let fm = [(f.rect[0] + f.rect[2]) / 2, (f.rect[1] + f.rect[3]) / 2];
        let (dx, dz) = (mid[0] - fm[0], mid[1] - fm[1]);
        let facing = if dx.abs() > dz.abs() { if dx > 0 { Side::East } else { Side::West } } else if dz > 0 { Side::South } else { Side::North };
        out.builtin.push(Builtin { kind: "fence".into(), at: [fm[0], f.floor, fm[1]], facing, rect: Some([f.rect[0] - 1, f.rect[1] - 1, f.rect[2] + 1, f.rect[3] + 1]) });
    }
    out
}
