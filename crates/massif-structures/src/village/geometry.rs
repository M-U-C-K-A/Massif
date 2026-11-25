//! The stages' results as the terrain graph's geometry
//! ([`massif_core::geometry`]): what the Village Generator's nodes pass
//! along their wires. Squares, towers and gates are points; roads and the
//! walls' line are splines; plots and fields are footprints; buildings and
//! props are instances of a pack's schematics.

use super::buildings::{Buildings, Placement};
use super::pack::SizeClass;
use super::plots::Plot;
use super::props::Props;
use super::roads::{RoadKind, Roads};
use super::site::Site;
use super::walls::Walls;
use super::{Side, Theme};
use crate::ground::Ground;
use massif_core::geometry::{Attrs, Dir, Footprint, Footprints, Instance, Instances, Point, Points, Spline, Splines};

fn dir(s: Side) -> Dir {
    match s {
        Side::North => Dir::North,
        Side::East => Dir::East,
        Side::South => Dir::South,
        Side::West => Dir::West,
    }
}

fn attrs(pairs: &[(&str, f64)]) -> Attrs {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn point(ground: &dyn Ground, [x, z]: [i32; 2], kind: &str, id: u64) -> Point {
    Point { pos: [x as f64, ground.floor(x, z) as f64, z as f64], turn: 0.0, scale: 1.0, id, kind: kind.into(), attrs: Attrs::new() }
}

/// The squares, as points of kind `centre`, each carrying how far the
/// village reaches (`reach`), whether its size was asked for (`sized`) and
/// the area's middle (`mid_x`, `mid_z`).
pub fn site_points(site: &Site, sized: bool, ground: &dyn Ground) -> Points {
    let items = site
        .centres
        .iter()
        .enumerate()
        .map(|(k, c)| {
            let mut p = point(ground, *c, "centre", k as u64);
            p.attrs = attrs(&[("reach", site.reach as f64), ("sized", sized as u8 as f64), ("mid_x", site.middle[0] as f64), ("mid_z", site.middle[1] as f64)]);
            p
        })
        .collect();
    Points { items }
}

/// The site back from its points, in `area`; with it whether the village's
/// size was asked for.
pub fn site_from(points: &Points, area: &super::Area) -> Option<(Site, bool)> {
    let first = points.items.iter().find(|p| p.kind == "centre" || p.kind.is_empty())?;
    let a = |k: &str, or: f64| first.attrs.get(k).copied().unwrap_or(or);
    let centres: Vec<[i32; 2]> = points.items.iter().filter(|p| p.kind == "centre" || p.kind.is_empty()).map(|p| [p.pos[0].round() as i32, p.pos[2].round() as i32]).collect();
    let site = Site { reach: a("reach", 120.0) as i32, middle: [a("mid_x", first.pos[0]) as i32, a("mid_z", first.pos[2]) as i32], samples: area.samples(8), centres };
    Some((site, a("sized", 0.0) > 0.5))
}

/// The roads, as splines of kind `main`, `side` or `square`, on the land.
pub fn road_splines(roads: &Roads, ground: &dyn Ground) -> Splines {
    let lines = roads
        .lines
        .iter()
        .map(|r| Spline {
            points: r.points.iter().map(|&[x, z]| [x as f64, ground.floor(x, z) as f64, z as f64]).collect(),
            closed: false,
            width: r.width as f32,
            kind: match r.kind {
                RoadKind::Main => "main",
                RoadKind::Side => "side",
                RoadKind::Square => "square",
            }
            .into(),
            attrs: Attrs::new(),
        })
        .collect();
    Splines { lines }
}

/// The roads back from their splines (those of kind `main`, `side` or
/// `square`).
pub fn roads_from(splines: &Splines) -> Roads {
    let lines = splines
        .lines
        .iter()
        .filter_map(|l| {
            let kind = match l.kind.as_str() {
                "main" => RoadKind::Main,
                "side" => RoadKind::Side,
                "square" => RoadKind::Square,
                _ => return None,
            };
            Some(super::roads::Road { points: l.points.iter().map(|p| [p[0].round() as i32, p[2].round() as i32]).collect(), kind, width: (l.width.round() as i32).max(1) })
        })
        .collect();
    Roads { lines, extra: Vec::new() }
}

/// The walls: their line, closed, and their towers and gates as points.
pub fn wall_geometry(walls: &Walls, every: i32, ground: &dyn Ground) -> (Splines, Points) {
    let line = Spline { points: walls.line.iter().map(|&[x, z]| [x as f64, ground.floor(x, z) as f64, z as f64]).collect(), closed: true, width: 1.0, kind: "wall".into(), attrs: Attrs::new() };
    let towers = walls.towers(every);
    let mut points: Vec<Point> = towers.iter().enumerate().map(|(k, t)| point(ground, *t, if walls.corners.contains(t) { "corner" } else { "tower" }, k as u64)).collect();
    let n = points.len() as u64;
    points.extend(walls.gates.iter().enumerate().map(|(k, g)| point(ground, *g, "gate", n + k as u64)));
    (Splines { lines: vec![line] }, Points { items: points })
}

/// The walls back from their line and points (none without a line).
pub fn walls_from(line: &Splines, points: Option<&Points>) -> Option<Walls> {
    let l = line.lines.iter().find(|l| l.kind == "wall" || l.closed)?;
    let of = |kind: &str| -> Vec<[i32; 2]> { points.map(|p| p.items.iter().filter(|q| q.kind == kind).map(|q| [q.pos[0].round() as i32, q.pos[2].round() as i32]).collect()).unwrap_or_default() };
    Some(Walls { line: l.points.iter().map(|p| [p[0].round() as i32, p[2].round() as i32]).collect(), corners: of("corner"), gates: of("gate") })
}

fn side_of(d: Dir) -> Side {
    match d {
        Dir::North => Side::North,
        Dir::East => Side::East,
        Dir::South => Side::South,
        Dir::West => Side::West,
    }
}

fn class_of(name: &str) -> SizeClass {
    SizeClass::ALL.into_iter().find(|c| c.name() == name).unwrap_or(SizeClass::Small)
}

/// The plots back from their footprints (those with a door and a facing).
pub fn plots_from(f: &Footprints) -> Vec<Plot> {
    f.items
        .iter()
        .filter_map(|p| {
            let a = |k: &str| p.attrs.get(k).copied().unwrap_or(0.0);
            Some(Plot { door: p.door?, side: side_of(p.facing?).opposite(), size: class_of(&p.kind), district: a("district") as u16, from_centre: a("from_centre") as f32, main: a("main") > 0.5 })
        })
        .collect()
}

/// The plots, as footprints of the size they would like (kind: its size
/// class), from the door's column back.
pub fn plot_footprints(plots: &[Plot], theme: &Theme) -> Footprints {
    let items = plots
        .iter()
        .map(|p| {
            let side = match p.size {
                SizeClass::Small => theme.sizes.small[1],
                SizeClass::Medium => theme.sizes.medium[1],
                SizeClass::Large | SizeClass::Huge => theme.sizes.large[1],
            };
            let half = side / 2;
            // From the door, `side` deep away from the road.
            let s = p.side.step();
            let far = [p.door[0] + s[0] * (side - 1), p.door[1] + s[1] * (side - 1)];
            let (lo, hi) = if s[0] != 0 { ([p.door[0].min(far[0]), p.door[1] - half], [p.door[0].max(far[0]) + 1, p.door[1] + half + 1]) } else { ([p.door[0] - half, p.door[1].min(far[1])], [p.door[0] + half + 1, p.door[1].max(far[1]) + 1]) };
            Footprint {
                min: lo,
                max: hi,
                facing: Some(dir(p.side.opposite())),
                door: Some(p.door),
                kind: p.size.name().into(),
                attrs: attrs(&[("district", p.district as f64), ("from_centre", p.from_centre as f64), ("main", p.main as u8 as f64)]),
            }
        })
        .collect();
    Footprints { items }
}

/// The asset name of a schematic of `theme`: `pack:<pack>/<theme>/<path
/// in the theme>`.
pub fn asset(theme: &Theme, file: &std::path::Path) -> String {
    let rel = file.strip_prefix(&theme.dir).unwrap_or(file);
    format!("pack:{}/{}/{}", theme.pack, theme.name, rel.to_string_lossy().replace('\\', "/"))
}

fn instance(theme: &Theme, p: &Placement) -> Instance {
    Instance {
        asset: asset(theme, &p.file),
        origin: p.origin,
        turn: p.turn,
        mirror: false,
        bbox: (p.min, p.max),
        kind: p.kind.clone(),
        attrs: attrs(&[
            ("floor", p.door[1] as f64),
            ("door_x", p.door[0] as f64),
            ("door_z", p.door[2] as f64),
            ("villagers", p.villagers as f64),
            ("facing", dir(p.facing).turns_from_south() as f64),
            ("size", SizeClass::ALL.iter().position(|c| *c == p.size).unwrap_or(0) as f64),
        ]),
    }
}

fn dir_from_turns(t: f64) -> Side {
    match t.round() as i32 {
        1 => Side::West,
        2 => Side::North,
        3 => Side::East,
        _ => Side::South,
    }
}

/// A placement back from its instance: its file in `theme`'s folder.
pub fn placement_from(i: &Instance, theme: &Theme) -> Option<Placement> {
    let prefix = format!("pack:{}/{}/", theme.pack, theme.name);
    let rel = i.asset.strip_prefix(&prefix)?;
    let a = |k: &str| i.attrs.get(k).copied().unwrap_or(0.0);
    Some(Placement {
        kind: i.kind.clone(),
        file: theme.dir.join(rel),
        origin: i.origin,
        turn: i.turn,
        min: i.bbox.0,
        max: i.bbox.1,
        door: [a("door_x") as i32, a("floor") as i32, a("door_z") as i32],
        facing: dir_from_turns(a("facing")),
        size: SizeClass::ALL[(a("size") as usize).min(3)],
        villagers: a("villagers") as u32,
    })
}

/// The buildings back from their instances and fields, the land levelled
/// under them again.
pub fn buildings_from(i: &Instances, fields: Option<&Footprints>, theme: &Theme, ground: &dyn Ground) -> Buildings {
    let placed: Vec<Placement> = i.items.iter().filter_map(|i| placement_from(i, theme)).collect();
    let fields: Vec<super::buildings::Field> = fields
        .map(|f| {
            f.items
                .iter()
                .filter_map(|f| {
                    let crop = f.kind.strip_prefix("field:")?;
                    let a = |k: &str| f.attrs.get(k).copied().unwrap_or(0.0);
                    Some(super::buildings::Field { rect: [f.min[0], f.min[1], f.max[0] - 1, f.max[1] - 1], floor: a("floor") as i32, crop: crop.to_owned(), farm: a("farm") as usize })
                })
                .filter(|f| f.farm < placed.len())
                .collect()
        })
        .unwrap_or_default();
    let mut heights = super::buildings::level_under(placed.iter().map(|p| (p.bbox(), p.door[1])), ground, 4);
    for f in &fields {
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
    Buildings { placed, fields, heights: hv, notes: Vec::new() }
}

/// The buildings as instances, and the fields as footprints of kind
/// `field:<crop>`.
pub fn building_geometry(b: &Buildings, theme: &Theme) -> (Instances, Footprints) {
    let instances = Instances { items: b.placed.iter().map(|p| instance(theme, p)).collect() };
    let fields = Footprints {
        items: b
            .fields
            .iter()
            .map(|f| Footprint { min: [f.rect[0], f.rect[1]], max: [f.rect[2] + 1, f.rect[3] + 1], facing: None, door: None, kind: format!("field:{}", f.crop), attrs: attrs(&[("floor", f.floor as f64), ("farm", f.farm as f64)]) })
            .collect(),
    };
    (instances, fields)
}

/// The props from the pack as instances; Massif's own as instances of
/// `massif:<kind>`.
pub fn prop_instances(p: &Props, theme: &Theme) -> Instances {
    let mut items: Vec<Instance> = p.placed.iter().map(|q| instance(theme, q)).collect();
    items.extend(p.builtin.iter().map(|b| {
        let (min, max) = match b.rect {
            Some(r) => ([r[0], b.at[1], r[1]], [r[2], b.at[1], r[3]]),
            None => (b.at, b.at),
        };
        Instance { asset: format!("massif:{}", b.kind), origin: b.at, turn: 0, mirror: false, bbox: (min, max), kind: format!("prop:{}", b.kind), attrs: attrs(&[("facing", dir(b.facing).turns_from_south() as f64)]) }
    }));
    Instances { items }
}

/// The fields as instances (`massif:field`, kind `field:<crop>`), for the
/// map: what the Map Output's structures carry of them.
pub fn field_instances(b: &Buildings) -> Instances {
    let items = b
        .fields
        .iter()
        .map(|f| Instance {
            asset: "massif:field".into(),
            origin: [f.rect[0], f.floor, f.rect[1]],
            turn: 0,
            mirror: false,
            bbox: ([f.rect[0], f.floor - 1, f.rect[1]], [f.rect[2], f.floor, f.rect[3]]),
            kind: format!("field:{}", f.crop),
            attrs: attrs(&[("floor", f.floor as f64), ("farm", f.farm as f64)]),
        })
        .collect();
    Instances { items }
}

/// The walls as lines for the map: their line (`wall`, closed), and their
/// corners and gates as two more of kind `wall_corners` and `wall_gates`
/// (what the Map Output's roads carry of them), each with what runs along
/// them (`enclosure`, its place in `Enclosure::ALL`).
pub fn wall_lines(walls: &Walls, enclosure: crate::village::walls::Enclosure, ground: &dyn Ground) -> Vec<Spline> {
    let style = crate::village::walls::Enclosure::ALL.iter().position(|e| *e == enclosure).unwrap_or(0) as f64;
    let line = |pts: &[[i32; 2]], kind: &str, closed: bool| Spline { points: pts.iter().map(|&[x, z]| [x as f64, ground.floor(x, z) as f64, z as f64]).collect(), closed, width: 1.0, kind: kind.into(), attrs: attrs(&[("enclosure", style)]) };
    vec![line(&walls.line, "wall", true), line(&walls.corners, "wall_corners", false), line(&walls.gates, "wall_gates", false)]
}

/// The walls back from the lines of [`wall_lines`] (none without a line).
pub fn walls_from_lines(lines: &Splines) -> Option<Walls> {
    let of = |kind: &str| -> Vec<[i32; 2]> { lines.lines.iter().filter(|l| l.kind == kind).flat_map(|l| l.points.iter().map(|p| [p[0].round() as i32, p[2].round() as i32])).collect() };
    let line = of("wall");
    (!line.is_empty()).then(|| Walls { line, corners: of("wall_corners"), gates: of("wall_gates") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;
    use crate::village::{Params, Request, read_pack, stages, testpack};
    use massif_core::coords::BlockRect;

    #[test]
    fn the_stages_become_the_graph_s_geometry() {
        let dir = std::env::temp_dir().join(format!("massif-village-geometry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let theme = read_pack(&testpack::write_pack(&dir).unwrap()).unwrap().remove(0);
        let ground = Flat { height: 64, water: None };
        let rect = BlockRect::new(0, 0, 300, 300);
        let inside = |x: i32, z: i32| rect.contains(x, z);
        let req = Request { theme: &theme, rect, inside: &inside, seed: 2, params: Params { walls: true, ..Params::default() }, id: 1 };
        let (st, _) = stages(None, &ground, &req).unwrap();
        let centres = site_points(&st.site, false, &ground);
        assert_eq!(centres.items.len(), st.site.centres.len());
        assert!(centres.items.iter().all(|p| p.kind == "centre" && p.pos[1] == 64.0));
        let roads = road_splines(&st.roads, &ground);
        assert!(roads.lines.iter().any(|l| l.kind == "main" && l.width == 3.0));
        let (line, points) = wall_geometry(st.walls.as_ref().unwrap(), theme.walls.towers, &ground);
        assert!(line.lines[0].closed && points.items.iter().any(|p| p.kind == "gate"));
        let plots = plot_footprints(&st.plots, &theme);
        assert_eq!(plots.items.len(), st.plots.len());
        // A plot's door is in it, on its edge facing the road.
        for (f, p) in plots.items.iter().zip(&st.plots) {
            assert!(f.contains(p.door[0], p.door[1]), "{f:?} {p:?}");
        }
        let (houses, fields) = building_geometry(&st.buildings, &theme);
        assert_eq!(houses.items.len(), st.buildings.placed.len());
        assert!(houses.items.iter().all(|i| i.asset.starts_with("pack:Plains (test boxes)/Plains/") && i.asset.ends_with(".nbt")));
        assert!(fields.items.iter().all(|f| f.kind.starts_with("field:")));
        let props = prop_instances(&st.props, &theme);
        assert!(props.items.iter().any(|i| i.kind == "market"));
        // And back: the same stages.
        let (site, sized) = site_from(&centres, &st.area).unwrap();
        assert_eq!((site.centres.clone(), site.reach, sized), (st.site.centres.clone(), st.site.reach, false));
        assert_eq!(roads_from(&roads), st.roads);
        let w = walls_from(&line, Some(&points)).unwrap();
        assert_eq!((w.line.clone(), w.gates.clone()), (st.walls.as_ref().unwrap().line.clone(), st.walls.as_ref().unwrap().gates.clone()));
        assert_eq!(plots_from(&plots).iter().map(|p| (p.door, p.side, p.size)).collect::<Vec<_>>(), st.plots.iter().map(|p| (p.door, p.side, p.size)).collect::<Vec<_>>());
        let b = buildings_from(&houses, Some(&fields), &theme, &ground);
        assert_eq!(b.placed, st.buildings.placed);
        assert_eq!(b.fields, st.buildings.fields);
        assert_eq!(b.heights, st.buildings.heights);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
