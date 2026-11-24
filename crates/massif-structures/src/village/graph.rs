//! What Apply writes of the terrain graph's villages: the Map Output's
//! structures (instances of the packs' schematics and of Massif's own
//! props and fields) and roads (the roads and the walls' lines) laid as one
//! structure per theme, each marked as the graph's at a revision, so that
//! applying the graph again replaces its own and never those placed by
//! hand.
//!
//! The map's relief is the graph's own, levelled under the buildings by
//! Village ground: lay after writing the heights, on the map as it then is
//! ([`crate::ground::MapGround`]).

use super::buildings::Placement;
use super::geometry as geo;
use super::lay::{PlanSource, lay};
use super::props::{Builtin, Props};
use super::roads::Roads;
use super::site::Site;
use super::{Area, Params, RoadType, Side, Stages, Theme};
use crate::ground::Ground;
use crate::library::Library;
use crate::vanilla::Built;
use massif_core::coords::BlockRect;
use massif_core::geometry::{Instance, Instances, Splines};
use massif_core::structures::StructureSource;
use std::path::PathBuf;
use std::sync::Arc;

/// The themes of the packs the graph's village nodes name (their Pack
/// settings, folders): every theme of each, and what could not be read.
pub fn themes_of(packs: &[PathBuf]) -> (Vec<Theme>, Vec<String>) {
    let (mut themes, mut errors) = (Vec::new(), Vec::new());
    for p in packs {
        match super::read_pack(p) {
            Ok(t) => themes.extend(t),
            Err(e) => errors.push(e),
        }
    }
    (themes, errors)
}

/// The theme an instance of a pack's schematic comes from, by its asset
/// (`pack:<pack>/<theme>/…`).
fn theme_index(i: &Instance, themes: &[Theme]) -> Option<usize> {
    themes.iter().position(|t| i.asset.starts_with(&format!("pack:{}/{}/", t.pack, t.name)))
}

fn side_of(turns: f64) -> Side {
    match turns.round() as i32 {
        1 => Side::West,
        2 => Side::North,
        3 => Side::East,
        _ => Side::South,
    }
}

/// Massif's own prop or field an instance stands for.
fn builtin(i: &Instance) -> Option<Builtin> {
    let kind = i.asset.strip_prefix("massif:")?;
    if kind == "field" {
        return None;
    }
    let rect = (kind == "fence").then_some([i.bbox.0[0], i.bbox.0[2], i.bbox.1[0], i.bbox.1[2]]);
    Some(Builtin { kind: kind.to_owned(), at: i.origin, facing: side_of(i.attrs.get("facing").copied().unwrap_or(0.0)), rect })
}

/// The stages of one theme's village, rebuilt from the map's geometry.
fn stages_of(theme: &Theme, mine: &[&Instance], fields: &Instances, roads: Roads, walls: Option<super::walls::Walls>, ground: &dyn Ground) -> Stages {
    let mut buildings = Vec::new();
    let mut placed_props: Vec<Placement> = Vec::new();
    for i in mine {
        let Some(p) = geo::placement_from(i, theme) else { continue };
        if p.kind == "market" || p.kind.starts_with("prop:") {
            placed_props.push(p);
        } else {
            buildings.push((*i).clone());
        }
    }
    let field_fp = massif_core::geometry::Footprints {
        items: fields
            .items
            .iter()
            .map(|f| massif_core::geometry::Footprint { min: [f.bbox.0[0], f.bbox.0[2]], max: [f.bbox.1[0] + 1, f.bbox.1[2] + 1], facing: None, door: None, kind: f.kind.clone(), attrs: f.attrs.clone() })
            .collect(),
    };
    let b = geo::buildings_from(&Instances { items: buildings }, Some(&field_fp), theme, ground);
    // The area: everything laid, with room round it.
    let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
    let mut grow = |x: i32, z: i32| {
        lo = [lo[0].min(x), lo[1].min(z)];
        hi = [hi[0].max(x), hi[1].max(z)];
    };
    for i in mine {
        grow(i.bbox.0[0], i.bbox.0[2]);
        grow(i.bbox.1[0], i.bbox.1[2]);
    }
    for r in &roads.lines {
        for p in &r.points {
            grow(p[0], p[1]);
        }
    }
    if let Some(w) = &walls {
        for p in &w.line {
            grow(p[0], p[1]);
        }
    }
    let rect = if lo[0] <= hi[0] { BlockRect::new(lo[0] - 16, lo[1] - 16, hi[0] + 17, hi[1] + 17) } else { BlockRect::new(0, 0, 1, 1) };
    let props = Props { placed: placed_props, builtin: mine.iter().filter_map(|i| builtin(i)).collect() };
    Stages { area: Area::full(rect), site: Site { centres: Vec::new(), reach: 0, middle: [0, 0], samples: Vec::new() }, roads, walls, plots: Vec::new(), buildings: b, props }
}

/// The graph's villages laid: one structure per theme met among
/// `structures` (the Map Output's), with `lines` (its roads: roads and
/// walls), numbered from `first_id` and marked as the graph's at
/// `revision`. Instances of themes not among `themes` are skipped, and
/// said so.
#[allow(clippy::too_many_arguments)]
pub fn lay_graph(lib: Option<&Library>, plans: &dyn PlanSource, ground: &dyn Ground, structures: &Instances, lines: &Splines, themes: &[Theme], seed: u64, first_id: u32, revision: u64) -> Result<(Vec<Built>, Vec<String>), String> {
    let mut notes = Vec::new();
    let mut by_theme: Vec<Vec<&Instance>> = vec![Vec::new(); themes.len()];
    let mut fields = Instances::default();
    for i in &structures.items {
        if i.asset == "massif:field" {
            fields.items.push(i.clone());
        } else if i.asset.starts_with("massif:") {
            // Massif's own props: with the theme whose instances are
            // nearest, settled below.
        } else if let Some(k) = theme_index(i, themes) {
            by_theme[k].push(i);
        } else if i.asset.starts_with("pack:") {
            notes.push(format!("{}: its pack is not among those the graph names", i.asset));
        }
    }
    // Massif's own props and the roads go with the theme with most
    // buildings (a graph with one village has one).
    let main = (0..themes.len()).max_by_key(|&k| by_theme[k].len());
    let Some(main) = main.filter(|&k| !by_theme[k].is_empty()) else { return Ok((Vec::new(), notes)) };
    for i in structures.items.iter().filter(|i| i.asset.starts_with("massif:") && i.asset != "massif:field") {
        by_theme[main].push(i);
    }
    let road = lines.lines.iter().find_map(|l| l.attrs.get("road")).copied().unwrap_or(0.0) as usize;
    let params = Params { road: [RoadType::Theme, RoadType::Dirt, RoadType::Premium, RoadType::Stone, RoadType::Planks].get(road).copied().unwrap_or_default(), ..Params::default() };
    let mut out = Vec::new();
    for (k, theme) in themes.iter().enumerate() {
        if by_theme[k].is_empty() {
            continue;
        }
        let (roads, walls, fields) = if k == main { (geo::roads_from(lines), geo::walls_from_lines(lines), fields.clone()) } else { (Roads::default(), None, Instances::default()) };
        let stages = stages_of(theme, &by_theme[k], &fields, roads, walls, ground);
        let mut built = lay(lib, plans, ground, theme, &params, &stages, None, seed ^ k as u64, first_id + out.len() as u32)?;
        built.structure.source = StructureSource::Graph { revision };
        notes.append(&mut built.unknown);
        out.push(built);
    }
    Ok((out, notes))
}

/// Write the graph's villages into the map, as one step of `edit`: the
/// structures the graph placed before (any revision) go, `built` come in,
/// and the columns levelled under them take their heights. Structures
/// placed by hand stay.
pub fn apply_graph(edit: &mut massif_core::history::Edit, built: &[Built]) {
    let s = edit.structures_mut();
    s.retain(|x| x.source.is_user());
    for b in built {
        s.push(Arc::new(b.structure.clone()));
    }
    for b in built {
        massif_core::structures::set_heights(edit, &b.heights);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;
    use crate::village::lay::DiskPlans;
    use crate::village::{Request, read_pack, stages, testpack};

    #[test]
    fn the_graph_s_villages_are_laid_from_the_map_s_geometry_as_the_brush_lays_them() {
        let dir = std::env::temp_dir().join(format!("massif-village-graph-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = testpack::write_pack(&dir).unwrap();
        let theme = read_pack(&root).unwrap().remove(0);
        let ground = Flat { height: 64, water: None };
        let rect = BlockRect::new(0, 0, 320, 320);
        let inside = |x: i32, z: i32| rect.contains(x, z);
        let req = Request { theme: &theme, rect, inside: &inside, seed: 4, params: Params { walls: true, road: RoadType::Stone, ..Params::default() }, id: 1 };
        let (st, _) = stages(None, &ground, &req).unwrap();
        // What the Village Generator gives the Map Output.
        let mut all = geo::building_geometry(&st.buildings, &theme).0;
        all.items.extend(geo::prop_instances(&st.props, &theme).items);
        all.items.extend(geo::field_instances(&st.buildings).items);
        let mut lines = geo::road_splines(&st.roads, &ground);
        for l in &mut lines.lines {
            l.attrs.insert("road".into(), 3.0);
        }
        lines.lines.extend(geo::wall_lines(st.walls.as_ref().unwrap(), &ground));
        let (themes, errors) = themes_of(std::slice::from_ref(&root));
        assert!(errors.is_empty());
        let (built, notes) = lay_graph(None, &DiskPlans::default(), &ground, &all, &lines, &themes, 4, 7, 3).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(built.len(), 1);
        let s = &built[0].structure;
        assert_eq!(s.source, StructureSource::Graph { revision: 3 });
        assert_eq!(s.id, 7);
        // The same blocks as the brush lays from the stages themselves.
        let direct = lay(None, &DiskPlans::default(), &ground, &theme, &Params { road: RoadType::Stone, ..Params::default() }, &st, None, 4, 7).unwrap().structure;
        assert_eq!(s.pieces.len(), direct.pieces.len());
        // As many blocks, but for the edges of the roads out at the area's
        // edge, which the brush's area cut and the map's geometry does not
        // know.
        assert!(s.block_count().abs_diff(direct.block_count()) <= 16, "{} vs {}", s.block_count(), direct.block_count());
        assert_eq!(s.entities.len(), direct.entities.len());
        assert!(s.palette.iter().any(|p| p == "minecraft:farmland[moisture=7]"), "fields");
        assert!(s.palette.iter().any(|p| p == "minecraft:cobblestone"), "stone roads");
        // Applied: the graph's own replaced, those placed by hand kept.
        let mut world = massif_core::world::World::new(massif_core::world::WorldMeta::default());
        world.dims.push(massif_core::world::Dimension::new(massif_core::world::DimensionKind::Overworld, -64, 320));
        let mut history = massif_core::history::History::new(1);
        let mut hand = s.clone();
        hand.source = StructureSource::User;
        hand.id = 1;
        history.begin("Hand", 0);
        history.edit(&mut world, 0).structures_mut().push(Arc::new(hand));
        history.commit();
        for _ in 0..2 {
            history.begin("Apply", 0);
            apply_graph(&mut history.edit(&mut world, 0), &built);
            history.commit();
        }
        let placed = &world.dims[0].structures;
        assert_eq!(placed.len(), 2);
        assert_eq!(placed.iter().filter(|s| s.source.is_user()).count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
