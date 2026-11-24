//! Villages: the stages of `massif_structures::village` as nodes, one each
//! — the site, the roads, the walls, the plots, the buildings, the props —
//! and the land levelled under them, which `massif:village-generator`
//! wires together. Each stage's result travels as geometry (points,
//! splines, footprints, instances) and is turned back into the stage's own
//! data by the next.
//!
//! The relief is read whole at the simulations' level; a village pack is
//! read from its folder (a Files setting, so the node is made again when
//! the pack changes), its schematics' blocks only when the map is applied.

use crate::kernel::{Ctx, Geo, Grid};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Reads, Section, Unit};
use massif_core::coords::BlockRect;
use massif_core::geometry::{Footprints, Instances, Points, Splines};
use massif_structures::ground::{Ground, WorldBlock};
use massif_structures::village::{self, Area, MillMode, Params, RoadType, Theme, geometry as geo};

pub const NODES: &[&NodeSpec] = &[&SITE, &ROADS, &WALLS, &PLOTS, &BUILDINGS, &PROPS, &GROUND];

// ------------------------------------------------------------ the land --

/// The relief as structures read it: a height grid of the whole terrain,
/// the sea over what lies under its level.
struct Land<'a> {
    grid: Option<&'a Grid>,
    x0: f64,
    z0: f64,
    step: f64,
    sea: f32,
}

impl<'a> Land<'a> {
    fn of(ctx: &Ctx, grid: Option<&'a Grid>) -> Self {
        Land { grid, x0: ctx.space.x0 as f64, z0: ctx.space.z0 as f64, step: ctx.spacing(), sea: ctx.space.sea_level }
    }

    fn height(&self, x: i32, z: i32) -> f32 {
        self.grid.map_or(self.sea, |g| g.bilinear((x as f64 - self.x0) / self.step, (z as f64 - self.z0) / self.step))
    }
}

impl Ground for Land<'_> {
    fn min_y(&self) -> i32 {
        -64
    }

    fn max_y(&self) -> i32 {
        320
    }

    fn surface(&self, x: i32, z: i32) -> i32 {
        self.floor(x, z).max(self.sea.floor() as i32 + 1)
    }

    fn floor(&self, x: i32, z: i32) -> i32 {
        self.height(x, z).floor() as i32 + 1
    }

    fn block(&self, x: i32, y: i32, z: i32) -> WorldBlock {
        if y < self.floor(x, z) {
            WorldBlock::Solid("minecraft:stone".into())
        } else if y < self.surface(x, z) {
            WorldBlock::Water
        } else {
            WorldBlock::Air
        }
    }
}

/// The area a village may take: where the mask is over a half (its
/// bounds), or the whole terrain.
fn area_of(ctx: &Ctx, mask: Option<&Grid>) -> Result<Area, String> {
    let step = ctx.spacing();
    let (x0, z0) = (ctx.space.x0, ctx.space.z0);
    let size = ctx.space.size as i32;
    let rect = match mask {
        Some(m) => {
            let (mut lo, mut hi) = ([i32::MAX; 2], [i32::MIN; 2]);
            for j in 0..m.rect.h as i32 {
                for i in 0..m.rect.w as i32 {
                    if m.at(m.rect.x + i, m.rect.z + j) >= 0.5 {
                        lo = [lo[0].min(i), lo[1].min(j)];
                        hi = [hi[0].max(i), hi[1].max(j)];
                    }
                }
            }
            if lo[0] > hi[0] {
                return Err("the area is empty: nowhere for the village".into());
            }
            BlockRect::new(x0 + (lo[0] as f64 * step) as i32, z0 + (lo[1] as f64 * step) as i32, x0 + ((hi[0] + 1) as f64 * step) as i32, z0 + ((hi[1] + 1) as f64 * step) as i32)
        }
        None => BlockRect::new(x0, z0, x0 + size, z0 + size),
    };
    if rect.x1 - rect.x0 > 4096 || rect.z1 - rect.z0 > 4096 {
        return Err(format!("the area is {} × {} blocks: give the village an Area of at most 4096 × 4096", rect.x1 - rect.x0, rect.z1 - rect.z0));
    }
    Ok(match mask {
        Some(m) => Area::new(rect, &|x, z| m.nearest(0, (x as f64 - x0 as f64) / step, (z as f64 - z0 as f64) / step) >= 0.5),
        None => Area::full(rect),
    })
}

/// Nothing, of each output's type: what a stage gives without a pack (a
/// new Village Generator, before one is chosen), rather than breaking the
/// graph.
fn nothing(ctx: &Ctx) -> Vec<Geo> {
    ctx.params.spec().outputs.iter().filter_map(|o| Geo::empty(o.kind)).collect()
}

/// Whether the Pack setting names a pack.
fn has_pack(ctx: &Ctx) -> bool {
    !ctx.params.text("pack").trim().is_empty()
}

/// The pack the Pack setting names: its first theme, or the one Theme
/// names.
fn theme_of(ctx: &Ctx) -> Result<Theme, String> {
    let path = ctx.params.text("pack").trim();
    if path.is_empty() {
        return Err("no village pack: choose the folder of its schematics".into());
    }
    let mut themes = village::read_pack(std::path::Path::new(path))?;
    let want = ctx.params.text("theme").trim();
    let k = themes.iter().position(|t| t.name.eq_ignore_ascii_case(want)).unwrap_or(0);
    Ok(themes.swap_remove(k))
}

/// The village's settings, from those this node has.
fn params_of(ctx: &Ctx) -> Params {
    let spec = ctx.params.spec();
    let p = ctx.params;
    let has = |n: &str| spec.param(n).is_some();
    let d = Params::default();
    Params {
        houses: has("houses").then(|| p.int("houses").clamp(3, 120) as u32),
        size_bias: if has("size_bias") { p.num("size_bias") as f32 } else { d.size_bias },
        road: if has("road") { road_type(p.choice("road")) } else { d.road },
        walls: has("on") && p.flag("on"),
        density: if has("density") { p.num("density") as f32 } else { d.density },
        spread: if has("spread") { p.num("spread") as f32 } else { d.spread },
        flat: if has("flat") { p.num("flat") as f32 } else { d.flat },
        fields: if has("fields") { p.num("fields") as f32 } else { d.fields },
        props: if has("props") { p.num("props") as f32 } else { d.props },
        mill: if has("mill") {
            match p.choice("mill") {
                "wind" => MillMode::Wind,
                "water" => MillMode::Water,
                "none" => MillMode::None,
                _ => MillMode::Auto,
            }
        } else {
            d.mill
        },
        max_slope: if has("max_slope") { p.int("max_slope") as i32 } else { d.max_slope },
        margin: if has("margin") { p.int("margin") as i32 } else { d.margin },
        street_width: (has("width") && p.int("width") > 0).then(|| p.int("width") as i32),
        foundation: None,
        villagers: None,
    }
}

fn road_type(v: &str) -> RoadType {
    match v {
        "dirt" => RoadType::Dirt,
        "premium" => RoadType::Premium,
        "stone" => RoadType::Stone,
        "planks" => RoadType::Planks,
        _ => RoadType::Theme,
    }
}

fn seed(ctx: &Ctx) -> u64 {
    ctx.space.seed ^ ((ctx.seed as u64) << 17)
}

fn points(ctx: &Ctx, k: usize, what: &str) -> Result<Points, String> {
    ctx.geo(k).and_then(Geo::points).cloned().ok_or_else(|| format!("no {what}: link its input"))
}

fn splines(ctx: &Ctx, k: usize, what: &str) -> Result<Splines, String> {
    ctx.geo(k).and_then(Geo::splines).cloned().ok_or_else(|| format!("no {what}: link its input"))
}

fn footprints(ctx: &Ctx, k: usize, what: &str) -> Result<Footprints, String> {
    ctx.geo(k).and_then(Geo::footprints).cloned().ok_or_else(|| format!("no {what}: link its input"))
}

fn instances(ctx: &Ctx, k: usize, what: &str) -> Result<Instances, String> {
    ctx.geo(k).and_then(Geo::instances).cloned().ok_or_else(|| format!("no {what}: link its input"))
}

fn site_of(ctx: &Ctx, k: usize, area: &Area) -> Result<village::site::Site, String> {
    geo::site_from(&points(ctx, k, "squares")?, area).map(|s| s.0).ok_or_else(|| "no square among the points".to_owned())
}

/// The walls of input `k`, when linked and drawn.
fn walls_of(ctx: &Ctx, k: usize) -> Option<village::walls::Walls> {
    geo::walls_from(ctx.geo(k).and_then(Geo::splines)?, None)
}

// -------------------------------------------------------------- inputs --

const HEIGHT_IN: PortSpec = PortSpec::new("height", "Height", PortKind::Height, "The relief the village stands on.").or("the sea level");
const AREA_IN: PortSpec = PortSpec::new("area", "Area", PortKind::Mask, "Where the village may go: over a half. At most 4096 × 4096 blocks.").or("the whole terrain");
const SQUARES_IN: PortSpec = PortSpec::new("squares", "Squares", PortKind::Points, "The squares, from Village site.");
const ROADS_IN: PortSpec = PortSpec::new("roads", "Roads", PortKind::Splines, "The roads, from Village roads (or Village walls).");
const WALLS_IN: PortSpec = PortSpec::new("walls", "Walls", PortKind::Splines, "The walls' line, from Village walls: kept clear.").optional();
const PACK: ParamSpec = ParamSpec::text("pack", "Pack", "", "The folder of the village pack: its schematics by type (house/, church/, farm/…) and its village.json.");
const THEME: ParamSpec = ParamSpec::text("theme", "Theme", "", "Which theme of the pack, when it has several: the first one when left empty.").section(Section::Advanced);
const HOUSES: ParamSpec = ParamSpec::int("houses", "Houses", 18, 3, 120, "How many houses (not counting wells, stalls, mills and towers): the village reaches as far as it needs to hold them.");
const DENSITY: ParamSpec = ParamSpec::float("density", "Density", Unit::Ratio, 0.6, 0.2, 1.0, "From airy (0.2) to tight (1): how close the plots follow one another along the roads.");
const SPREAD: ParamSpec = ParamSpec::float("spread", "Spread", Unit::Ratio, 0.5, 0.0, 1.0, "From a compact hamlet (0) to a village strung out along its roads (1).");
const SIZE_BIAS: ParamSpec = ParamSpec::float("size_bias", "Small ↔ large", Unit::Ratio, 0.0, -1.0, 1.0, "From small houses (−1) to large houses and manors (+1).");

// ---------------------------------------------------------------- site --

pub static SITE: NodeSpec = NodeSpec {
    id: "village.site",
    label: "Village site",
    family: Family::Geometry,
    summary: "Where a village's squares go: on flat dry land of the area, one for each district.",
    help: "The first stage of a village. The squares stand where the land is flattest and driest near the middle of the area, as much as Flat ground asks; a village of more than forty houses gets more districts, each round its own square. Houses, Density and Spread set how far the village reaches, which the roads follow.",
    keywords: &["village", "site", "square", "centre", "town", "settlement"],
    inputs: &[HEIGHT_IN, AREA_IN],
    outputs: &[PortSpec::new("squares", "Squares", PortKind::Points, "The squares, the village's first.")],
    params: &[HOUSES, DENSITY, SPREAD, ParamSpec::float("flat", "Flat ground", Unit::Ratio, 0.7, 0.0, 1.0, "How much the squares prefer flat land over the area's middle."), ParamSpec::seed()],
    kernel: Kernel::Geometry { cpu: site },
    ..NodeSpec::BASE
};

fn site(ctx: &Ctx, rasters: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    let land = Land::of(ctx, rasters[0]);
    let area = area_of(ctx, rasters[1])?;
    let p = params_of(ctx);
    let s = village::site::site(&land, &area, &p, seed(ctx))?;
    Ok(vec![Geo::Points(geo::site_points(&s, true, &land))])
}

// --------------------------------------------------------------- roads --

const ROAD_TYPES: &[Choice] = &[
    Choice { value: "theme", label: "Pack's", help: "The blocks the pack's village.json gives its roads." },
    Choice { value: "dirt", label: "Dirt path", help: "Dirt path and a little coarse dirt." },
    Choice { value: "premium", label: "Gravel", help: "Dirt path and gravel, coarse dirt on the edges." },
    Choice { value: "stone", label: "Stone", help: "Cobblestone, andesite and stone bricks, slabs where they climb." },
    Choice { value: "planks", label: "Planks", help: "Spruce and oak planks." },
];

pub static ROADS: NodeSpec = NodeSpec {
    id: "village.roads",
    label: "Village roads",
    family: Family::Geometry,
    summary: "A village's roads: main roads that follow the land between its squares and out of it, side roads off them.",
    help: "Main roads find their way between the squares and toward the area's edge, choosing gentle slopes and dry land; side roads branch off them every few dozen blocks, longer as Spread grows. Type is the blocks they are laid in when the map is applied; Width the main roads' width (0: the pack's).",
    keywords: &["village", "roads", "streets", "paths", "network"],
    inputs: &[HEIGHT_IN, AREA_IN, SQUARES_IN],
    outputs: &[PortSpec::new("roads", "Roads", PortKind::Splines, "The roads: main, side, and the squares.")],
    params: &[
        PACK,
        ParamSpec::choice("road", "Type", ROAD_TYPES, "theme", "The blocks the roads are laid in."),
        SPREAD,
        ParamSpec::int("width", "Width", 0, 0, 9, "The main roads' width in blocks; 0 keeps the pack's.").section(Section::Advanced),
        THEME,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Geometry { cpu: roads },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn roads(ctx: &Ctx, rasters: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    if !has_pack(ctx) {
        return Ok(nothing(ctx));
    }
    let land = Land::of(ctx, rasters[0]);
    let area = area_of(ctx, rasters[1])?;
    let (site, sized) = geo::site_from(&points(ctx, 2, "squares")?, &area).ok_or("no square among the points")?;
    let theme = theme_of(ctx)?;
    let mut p = params_of(ctx);
    // The houses the site was sized for: the roads then stop at its reach.
    p.houses = sized.then_some(18);
    let r = village::roads::roads(&land, &area, &site, &Default::default(), &theme, &p, seed(ctx));
    let mut s = geo::road_splines(&r, &land);
    // The road type travels with the roads, for the map to lay them.
    let road = ROAD_TYPES.iter().position(|c| c.value == ctx.params.choice("road")).unwrap_or(0) as f64;
    for l in &mut s.lines {
        l.attrs.insert("road".into(), road);
    }
    Ok(vec![Geo::Splines(s)])
}

// --------------------------------------------------------------- walls --

pub static WALLS: NodeSpec = NodeSpec {
    id: "village.walls",
    label: "Village walls",
    family: Family::Geometry,
    summary: "Ramparts round a village, three blocks inside its area: towers at the corners and along them, a gate where a main road crosses.",
    help: "The wall's line follows the area's outline; side roads stop short of it, main roads pass through its gates (Roads out is the roads cut so). With Walls off, it draws no wall and lets the roads through as they are.",
    keywords: &["village", "walls", "ramparts", "towers", "gates", "fortification"],
    inputs: &[
        HEIGHT_IN,
        AREA_IN,
        ROADS_IN,
        PortSpec::new("squares", "Squares", PortKind::Points, "The squares, from Village site: the walls go round the village's reach of them.").or("round the whole area"),
        PortSpec::new("buildings", "Buildings", PortKind::Instances, "The buildings, from Village buildings: the walls go round them, eight blocks out.").optional(),
        PortSpec::new("fields", "Fields", PortKind::Footprints, "The fields, kept within the walls too.").optional(),
    ],
    outputs: &[
        PortSpec::new("walls", "Walls", PortKind::Splines, "The wall's line, closed."),
        PortSpec::new("towers", "Towers", PortKind::Points, "Its corners, towers and gates."),
        PortSpec::new("roads", "Roads", PortKind::Splines, "The roads, side roads cut at the wall, and the wall's lines: what the Map Output's roads take."),
    ],
    params: &[ParamSpec::flag("on", "Walls", true, "Whether the village has walls."), PACK, THEME],
    kernel: Kernel::Geometry { cpu: walls },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn walls(ctx: &Ctx, rasters: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    let roads_in = splines(ctx, 2, "roads")?;
    if !ctx.params.flag("on") {
        return Ok(vec![Geo::Splines(Splines::default()), Geo::Points(Points::default()), Geo::Splines(roads_in)]);
    }
    let land = Land::of(ctx, rasters[0]);
    let area = area_of(ctx, rasters[1])?;
    let every = theme_of(ctx).map_or(40, |t| t.walls.towers);
    let mut roads = geo::roads_from(&roads_in);
    // Round the buildings when they are given, else round the village's
    // reach, else the area.
    let mut boxes: Vec<([i32; 2], [i32; 2])> = ctx.geo(4).and_then(Geo::instances).map(|i| i.items.iter().filter(|i| !i.asset.starts_with("massif:")).map(|i| ([i.bbox.0[0], i.bbox.0[2]], [i.bbox.1[0], i.bbox.1[2]])).collect()).unwrap_or_default();
    if let Some(f) = ctx.geo(5).and_then(Geo::footprints) {
        boxes.extend(f.items.iter().map(|f| ([f.min[0] - 1, f.min[1] - 1], [f.max[0], f.max[1]])));
    }
    let site = ctx.geo(3).and_then(Geo::points).and_then(|p| geo::site_from(p, &area)).filter(|s| s.1).map(|s| s.0);
    let w = match village::walls::round_buildings(&area, &roads, &boxes) {
        Some(w) => w,
        None => village::walls::walls(&area, &roads, site.as_ref()),
    };
    village::walls::trim(&mut roads, &w);
    let (line, towers) = geo::wall_geometry(&w, every, &land);
    // The roads cut, keeping what the splines carried (their type).
    let mut out = geo::road_splines(&roads, &land);
    let attrs = roads_in.lines.first().map(|l| l.attrs.clone()).unwrap_or_default();
    for l in &mut out.lines {
        l.attrs = attrs.clone();
    }
    // With the walls' line, corners and gates, for the map to build them.
    out.lines.extend(geo::wall_lines(&w, &land));
    Ok(vec![Geo::Splines(line), Geo::Points(towers), Geo::Splines(out)])
}

// --------------------------------------------------------------- plots --

pub static PLOTS: NodeSpec = NodeSpec {
    id: "village.plots",
    label: "Village plots",
    family: Family::Geometry,
    summary: "Plots along both sides of a village's roads, each the size of building it would like and its door to the road.",
    help: "A plot every 9 to 15 blocks along each road, fewer as Density falls; nearer the squares first. Their sizes lean small or large as Small ↔ large asks, and a little larger away from the squares.",
    keywords: &["village", "plots", "lots", "parcels", "footprints"],
    inputs: &[SQUARES_IN, ROADS_IN],
    outputs: &[PortSpec::new("plots", "Plots", PortKind::Footprints, "The plots: each its door, the way it faces, its size class.")],
    params: &[DENSITY, SIZE_BIAS, PACK, THEME, ParamSpec::seed()],
    kernel: Kernel::Geometry { cpu: plots },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn plots(ctx: &Ctx, _: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    if !has_pack(ctx) {
        return Ok(nothing(ctx));
    }
    let roads = geo::roads_from(&splines(ctx, 1, "roads")?);
    let sq = points(ctx, 0, "squares")?;
    let reach = sq.items.first().and_then(|p| p.attrs.get("reach")).copied().unwrap_or(120.0) as i32;
    let site = village::site::Site { centres: sq.items.iter().map(|p| [p.pos[0].round() as i32, p.pos[2].round() as i32]).collect(), reach, middle: [0, 0], samples: Vec::new() };
    if site.centres.is_empty() {
        return Err("no square among the points".into());
    }
    let theme = theme_of(ctx)?;
    let plots = village::plots::plots(&roads, &site, &params_of(ctx), seed(ctx));
    Ok(vec![Geo::Footprints(geo::plot_footprints(&plots, &theme))])
}

// ----------------------------------------------------------- buildings --

const MILLS: &[Choice] = &[
    Choice { value: "auto", label: "Auto", help: "A watermill when water is near and the pack has one, else a windmill." },
    Choice { value: "wind", label: "Windmill", help: "A windmill, toward the edge." },
    Choice { value: "water", label: "Watermill", help: "A watermill by the water." },
    Choice { value: "none", label: "None", help: "No mill." },
];

pub static BUILDINGS: NodeSpec = NodeSpec {
    id: "village.buildings",
    label: "Village buildings",
    family: Family::Geometry,
    summary: "A building of the pack on each plot with room for it, its door to the road: houses, the church by the square, farms with their fields toward the edge.",
    help: "Each plot draws a type by the pack's shares and rules (the church and wells near the square, forges and taverns near the market, farms and towers toward the edge, manors on the heights), then a schematic of the size the plot likes, turned to face the road; a smaller one when it has not the room. Farms keep a field behind them, as often as Fields asks. It stops at Houses houses.",
    keywords: &["village", "buildings", "houses", "farms", "fields", "church", "instances"],
    inputs: &[HEIGHT_IN, AREA_IN, SQUARES_IN, ROADS_IN, WALLS_IN, PortSpec::new("plots", "Plots", PortKind::Footprints, "The plots, from Village plots.")],
    outputs: &[
        PortSpec::new("buildings", "Buildings", PortKind::Instances, "The buildings: the pack's schematics placed and turned."),
        PortSpec::new("fields", "Fields", PortKind::Footprints, "The farms' fields, each its crop."),
        PortSpec::new("roads", "Roads", PortKind::Splines, "The roads, side roads cut after their last building (no door opens on the rest)."),
    ],
    params: &[
        PACK,
        HOUSES,
        SIZE_BIAS,
        ParamSpec::float("fields", "Fields", Unit::Ratio, 1.0, 0.0, 1.0, "The share of farms that keep a field."),
        ParamSpec::choice("mill", "Mill", MILLS, "auto", "Which mill the village has."),
        ParamSpec::int("max_slope", "Steepest plot", 10, 1, 32, "The most a plot's land may rise across it, in blocks.").section(Section::Advanced),
        ParamSpec::int("margin", "Margin", 1, 0, 8, "Blocks kept free round each building, at least.").section(Section::Advanced),
        THEME,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Geometry { cpu: buildings },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn buildings(ctx: &Ctx, rasters: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    if !has_pack(ctx) {
        return Ok(nothing(ctx));
    }
    let land = Land::of(ctx, rasters[0]);
    let area = area_of(ctx, rasters[1])?;
    let site = site_of(ctx, 2, &area)?;
    let lines = splines(ctx, 3, "roads")?;
    let mut roads = geo::roads_from(&lines);
    let walls = walls_of(ctx, 4);
    let reserved = walls.as_ref().map(|w| w.reserved()).unwrap_or_default();
    let mut plots = geo::plots_from(&footprints(ctx, 5, "plots")?);
    // Within the walls only.
    if let Some(w) = &walls {
        plots = w.plots_within(&plots);
    }
    let theme = theme_of(ctx)?;
    let b = village::buildings::buildings(&land, &area, &site, &roads, &reserved, &Default::default(), &plots, &theme, &params_of(ctx), seed(ctx));
    let (i, f) = geo::building_geometry(&b, &theme);
    // The side roads no door opens on go; the other lines (the walls')
    // pass as they came, the roads keeping what their splines carried.
    let doors: Vec<[i32; 2]> = b.placed.iter().map(|p| [p.door[0], p.door[2]]).collect();
    village::roads::prune(&mut roads, &doors);
    let attrs = lines.lines.iter().find(|l| matches!(l.kind.as_str(), "main" | "side" | "square")).map(|l| l.attrs.clone()).unwrap_or_default();
    let mut out = geo::road_splines(&roads, &land);
    for l in &mut out.lines {
        l.attrs = attrs.clone();
    }
    out.lines.extend(lines.lines.iter().filter(|l| !matches!(l.kind.as_str(), "main" | "side" | "square")).cloned());
    Ok(vec![Geo::Instances(i), Geo::Footprints(f), Geo::Splines(out)])
}

// --------------------------------------------------------------- props --

pub static PROPS: NodeSpec = NodeSpec {
    id: "village.props",
    label: "Village props",
    family: Family::Geometry,
    summary: "A village's props: market stalls round the square, lampposts and benches along the roads, hay and carts by the farms, fences round the fields.",
    help: "The pack's own props (its props/ and market/ folders) where it has them, Massif's simple ones otherwise; Props sets how many, the market's stalls standing whatever it says. Everything is the buildings and the props together, for the map.",
    keywords: &["village", "props", "lampposts", "benches", "market", "stalls", "decoration"],
    inputs: &[
        HEIGHT_IN,
        AREA_IN,
        SQUARES_IN,
        ROADS_IN,
        WALLS_IN,
        PortSpec::new("buildings", "Buildings", PortKind::Instances, "The buildings, from Village buildings."),
        PortSpec::new("fields", "Fields", PortKind::Footprints, "The fields, from Village buildings.").optional(),
    ],
    outputs: &[PortSpec::new("props", "Props", PortKind::Instances, "The props."), PortSpec::new("all", "Everything", PortKind::Instances, "The buildings, the props and the fields: what the Map Output's structures take.")],
    params: &[PACK, ParamSpec::float("props", "Props", Unit::Ratio, 0.5, 0.0, 1.0, "How many props: none (0) to many (1)."), THEME, ParamSpec::seed()],
    kernel: Kernel::Geometry { cpu: props },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn props(ctx: &Ctx, rasters: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    if !has_pack(ctx) {
        return Ok(nothing(ctx));
    }
    let land = Land::of(ctx, rasters[0]);
    let area = area_of(ctx, rasters[1])?;
    let site = site_of(ctx, 2, &area)?;
    let roads = geo::roads_from(&splines(ctx, 3, "roads")?);
    let reserved = walls_of(ctx, 4).map(|w| w.reserved()).unwrap_or_default();
    let theme = theme_of(ctx)?;
    let built = instances(ctx, 5, "buildings")?;
    let b = geo::buildings_from(&built, ctx.geo(6).and_then(Geo::footprints), &theme, &land);
    let p = village::props::props(&land, &area, &site, &roads, &reserved, &b, &theme, &params_of(ctx), seed(ctx));
    let props = geo::prop_instances(&p, &theme);
    let mut all = built;
    all.items.extend(props.items.iter().cloned());
    // The fields too, for the map to plant them.
    all.items.extend(geo::field_instances(&b).items);
    Ok(vec![Geo::Instances(props), Geo::Instances(all)])
}

// -------------------------------------------------------------- ground --

pub static GROUND: NodeSpec = NodeSpec {
    id: "village.ground",
    label: "Village ground",
    family: Family::Geometry,
    summary: "The relief levelled under a village's buildings and fields, and a mask of what the village takes.",
    help: "Each building's ground is cut or raised to its floor, sloping back to the land over four blocks; each field to its own. The mask is 1 under the buildings, the fields and the roads: keep the forest and the ores' veins out of the village with it.",
    keywords: &["village", "level", "flatten", "ground", "mask", "footprint"],
    inputs: &[
        PortSpec::new("height", "Height", PortKind::Height, "The relief.").or("the sea level"),
        PortSpec::new("buildings", "Buildings", PortKind::Instances, "The buildings (and props), from the village."),
        PortSpec::new("fields", "Fields", PortKind::Footprints, "The fields.").optional(),
        PortSpec::new("roads", "Roads", PortKind::Splines, "The roads.").optional(),
    ],
    outputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief levelled under the village."), PortSpec::new("mask", "Taken", PortKind::Mask, "1 where the village stands.")],
    params: &[],
    kernel: Kernel::Point { cpu: ground, gpu: None },
    ..NodeSpec::BASE
};

fn ground(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let step = ctx.spacing();
    let r = ctx.rect;
    let (x0, z0) = (ctx.space.x0 as f64 + r.x as f64 * step, ctx.space.z0 as f64 + r.z as f64 * step);
    let n = (r.w * r.h) as usize;
    let base: Vec<f32> = inputs.first().copied().flatten().map_or_else(|| vec![ctx.space.sea_level; n], |h| h.to_vec());
    let mut weight = vec![0.0f32; n];
    let mut target = vec![0.0f32; n];
    let mut taken = vec![0.0f32; n];
    // Over a box of blocks (ends included), with a slope of `margin`.
    let level = |lo: [i32; 2], hi: [i32; 2], floor: f32, margin: i32, weight: &mut [f32], target: &mut [f32], taken: &mut [f32]| {
        let (bx0, bz0, bx1, bz1) = ((lo[0] - margin) as f64, (lo[1] - margin) as f64, (hi[0] + margin + 1) as f64, (hi[1] + margin + 1) as f64);
        if bx1 < x0 || bz1 < z0 || bx0 > x0 + r.w as f64 * step || bz0 > z0 + r.h as f64 * step {
            return;
        }
        let i0 = ((bx0 - x0) / step).floor().max(0.0) as usize;
        let i1 = ((bx1 - x0) / step).ceil().clamp(0.0, r.w as f64) as usize;
        let j0 = ((bz0 - z0) / step).floor().max(0.0) as usize;
        let j1 = ((bz1 - z0) / step).ceil().clamp(0.0, r.h as f64) as usize;
        for j in j0..j1 {
            for i in i0..i1 {
                let (x, z) = ((x0 + i as f64 * step).floor() as i32, (z0 + j as f64 * step).floor() as i32);
                let dx = (lo[0] - x).max(x - hi[0]).max(0);
                let dz = (lo[1] - z).max(z - hi[1]).max(0);
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                let w = if d <= 0.0 { 1.0 } else { 1.0 - d / (margin as f32 + 1.0) };
                let k = j * r.w as usize + i;
                if d <= 0.0 {
                    taken[k] = 1.0;
                }
                if w > weight[k] {
                    weight[k] = w;
                    target[k] = floor;
                }
            }
        }
    };
    if let Some(b) = ctx.geo(1).and_then(Geo::instances) {
        for i in &b.items {
            // Massif's own props stand on the land as it is.
            if i.asset.starts_with("massif:") {
                continue;
            }
            let floor = i.attrs.get("floor").copied().unwrap_or(i.origin[1] as f64) as f32;
            level([i.bbox.0[0], i.bbox.0[2]], [i.bbox.1[0], i.bbox.1[2]], floor - 1.0, 4, &mut weight, &mut target, &mut taken);
        }
    }
    if let Some(f) = ctx.geo(2).and_then(Geo::footprints) {
        for f in &f.items {
            let floor = f.attrs.get("floor").copied().unwrap_or(0.0) as f32;
            level([f.min[0] - 1, f.min[1] - 1], [f.max[0], f.max[1]], floor - 1.0, 0, &mut weight, &mut target, &mut taken);
        }
    }
    for k in 0..n {
        let h = base[k];
        out[0][k] = if weight[k] > 0.0 { h + (target[k] - h) * weight[k] } else { h };
    }
    if let Some(s) = ctx.geo(3).and_then(Geo::splines) {
        for l in &s.lines {
            let half = (l.width as f64 / 2.0).max(0.5);
            for p in &l.points {
                let i0 = ((p[0] - half - x0) / step).floor().max(0.0) as usize;
                let i1 = ((p[0] + half - x0) / step).ceil().clamp(0.0, r.w as f64) as usize;
                let j0 = ((p[2] - half - z0) / step).floor().max(0.0) as usize;
                let j1 = ((p[2] + half - z0) / step).ceil().clamp(0.0, r.h as f64) as usize;
                for j in j0..j1 {
                    for i in i0..i1 {
                        taken[j * r.w as usize + i] = 1.0;
                    }
                }
            }
        }
    }
    out[1].copy_from_slice(&taken);
}
