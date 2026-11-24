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

/// The squares, as points of kind `centre`.
pub fn site_points(site: &Site, ground: &dyn Ground) -> Points {
    Points { items: site.centres.iter().enumerate().map(|(k, c)| point(ground, *c, "centre", k as u64)).collect() }
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

/// The walls: their line, closed, and their towers and gates as points.
pub fn wall_geometry(walls: &Walls, every: i32, ground: &dyn Ground) -> (Splines, Points) {
    let line = Spline { points: walls.line.iter().map(|&[x, z]| [x as f64, ground.floor(x, z) as f64, z as f64]).collect(), closed: true, width: 1.0, kind: "wall".into(), attrs: Attrs::new() };
    let mut points: Vec<Point> = walls.towers(every).iter().enumerate().map(|(k, t)| point(ground, *t, "tower", k as u64)).collect();
    let n = points.len() as u64;
    points.extend(walls.gates.iter().enumerate().map(|(k, g)| point(ground, *g, "gate", n + k as u64)));
    (Splines { lines: vec![line] }, Points { items: points })
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
        attrs: attrs(&[("floor", p.door[1] as f64), ("villagers", p.villagers as f64), ("facing", dir(p.facing).turns_from_south() as f64)]),
    }
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
        let centres = site_points(&st.site, &ground);
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
