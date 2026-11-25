//! Villages: an area filled with a village — a square in each district,
//! main roads that follow the land and join them, side roads off them,
//! plots along both, buildings of a theme on them facing the road, on
//! levelled ground, props along the roads and by the buildings, and walls
//! round it all if asked.
//!
//! The work is done in six stages, each a function of plain data to plain
//! data, so that each can be tested on its own and become a node of the
//! terrain graph's Village Generator:
//!
//! 1. [`site::site`] — the squares of the districts, in the area;
//! 2. [`roads::roads`] — main roads joining them, side roads off them;
//! 3. [`walls::walls`] — the ramparts' line, towers and gates (before the
//!    plots, which keep clear of it);
//! 4. [`plots::plots`] — plots along both sides of the roads, each its size
//!    and the side its door faces;
//! 5. [`buildings::buildings`] — a building of the theme on each plot that
//!    has room, turned to face the road, the land levelled under it, a
//!    field beside a farm;
//! 6. [`props::props`] — market stalls round the square, lampposts and
//!    benches along the roads, hay and carts by the farms, fences round
//!    the fields.
//!
//! No stage reads the disk: a theme's schematics are read with its pack
//! ([`pack::read_pack`]) and their blocks handed to [`lay::lay`] through a
//! [`lay::PlanSource`], which turns the stages into blocks. [`build`] runs them all, as
//! the village brush does.
//!
//! A theme is either one of the game's villages, assembled by the jigsaw
//! engine in each district, or a builder's pack ([`pack`], documented in
//! `docs/village-packs.md`).

pub mod buildings;
pub mod from_game;
pub mod from_world;
pub mod geometry;
pub mod graph;
pub mod lay;
pub mod pack;
pub mod plots;
pub mod props;
pub mod roads;
pub mod site;
pub mod testpack;
pub mod walls;

pub use pack::{BuildingType, Problem, RoadStyle, SizeClass, Theme, Variant, WallStyle, built_in, read_pack, read_pack_checked};

use crate::geom::Dir;
use crate::ground::Ground;
use crate::library::Library;
use crate::vanilla::Built;
use massif_core::coords::BlockRect;
use serde::{Deserialize, Serialize};

/// A side of a building or of a road, on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    pub fn parse(s: &str) -> Option<Side> {
        Some(match s.to_ascii_lowercase().as_str() {
            "north" | "n" => Side::North,
            "east" | "e" => Side::East,
            "south" | "s" => Side::South,
            "west" | "w" => Side::West,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Side::North => "north",
            Side::East => "east",
            Side::South => "south",
            Side::West => "west",
        }
    }

    /// One block that way, in x and z.
    pub fn step(self) -> [i32; 2] {
        match self {
            Side::North => [0, -1],
            Side::East => [1, 0],
            Side::South => [0, 1],
            Side::West => [-1, 0],
        }
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::North => Side::South,
            Side::East => Side::West,
            Side::South => Side::North,
            Side::West => Side::East,
        }
    }

    /// Quarter turns clockwise from north.
    pub fn quarter(self) -> u8 {
        match self {
            Side::North => 0,
            Side::East => 1,
            Side::South => 2,
            Side::West => 3,
        }
    }

    pub fn dir(self) -> Dir {
        match self {
            Side::North => Dir::North,
            Side::East => Dir::East,
            Side::South => Dir::South,
            Side::West => Dir::West,
        }
    }
}

/// The area a village may take: a rectangle of the map and which of its
/// columns are in (a selection, a mask of the graph).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    /// x0, z0, x1, z1, the ends excluded.
    pub rect: [i32; 4],
    /// Row by row (z, then x), one bit a column.
    bits: Vec<u64>,
}

impl Area {
    /// The columns of `rect` for which `inside` holds.
    pub fn new(rect: BlockRect, inside: &(dyn Fn(i32, i32) -> bool + Sync)) -> Area {
        let (w, h) = ((rect.x1 - rect.x0).max(0) as usize, (rect.z1 - rect.z0).max(0) as usize);
        let mut bits = vec![0u64; (w * h).div_ceil(64)];
        for z in 0..h {
            for x in 0..w {
                if inside(rect.x0 + x as i32, rect.z0 + z as i32) {
                    let k = z * w + x;
                    bits[k / 64] |= 1 << (k % 64);
                }
            }
        }
        Area { rect: [rect.x0, rect.z0, rect.x1, rect.z1], bits }
    }

    /// The whole rectangle.
    pub fn full(rect: BlockRect) -> Area {
        Area::new(rect, &|_, _| true)
    }

    pub fn block_rect(&self) -> BlockRect {
        BlockRect::new(self.rect[0], self.rect[1], self.rect[2], self.rect[3])
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        let [x0, z0, x1, z1] = self.rect;
        if x < x0 || z < z0 || x >= x1 || z >= z1 {
            return false;
        }
        let k = (z - z0) as usize * (x1 - x0) as usize + (x - x0) as usize;
        self.bits[k / 64] >> (k % 64) & 1 == 1
    }

    /// The columns in, every `step` blocks.
    pub fn samples(&self, step: i32) -> Vec<[i32; 2]> {
        let [x0, z0, x1, z1] = self.rect;
        (z0..z1).step_by(step as usize).flat_map(|z| (x0..x1).step_by(step as usize).map(move |x| [x, z])).filter(|&[x, z]| self.contains(x, z)).collect()
    }
}

/// The blocks roads are laid in: the theme's own, or one of four styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RoadType {
    /// The theme's (the pack's manifest, or the game's dirt path).
    #[default]
    Theme,
    /// Dirt path, a little coarse dirt.
    Dirt,
    /// Dirt path and gravel, coarse dirt on the edges.
    Premium,
    /// Cobblestone, andesite and stone bricks.
    Stone,
    /// Spruce and oak planks.
    Planks,
}

/// Which mill a village has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MillMode {
    /// A watermill when there is water near and the pack has one, else a
    /// windmill.
    #[default]
    Auto,
    Wind,
    Water,
    None,
}

/// What the village is to be like: the settings the Village Generator
/// shows (plan §7.1), with the brush's defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// How many houses (buildings that are not wells, stalls, mills or
    /// towers), 3 to 120; `None`: as many as the area holds.
    pub houses: Option<u32>,
    /// From small houses (−1) to large ones and manors (+1).
    pub size_bias: f32,
    pub road: RoadType,
    pub walls: bool,
    /// How built up: 0.2 (airy) to 1 (tight).
    pub density: f32,
    /// From a compact hamlet (0) to a village strung out along its roads
    /// (1).
    pub spread: f32,
    /// How much the squares prefer flat land (0–1).
    pub flat: f32,
    /// The share of farms that keep a field (0 for no fields), 0–1.
    pub fields: f32,
    /// How many props: none (0) to many (1).
    pub props: f32,
    pub mill: MillMode,
    /// The steepest a plot may be: its highest and lowest columns apart.
    pub max_slope: i32,
    /// Blocks kept free round each building, at least.
    pub margin: i32,
    /// The width of the main roads, if not the theme's.
    pub street_width: Option<i32>,
    /// Layers of foundation, if not the theme's.
    pub foundation: Option<i32>,
    /// Villagers in each house, if not the theme's.
    pub villagers: Option<u32>,
    /// How many buildings of each type (`church`, `farm`, `market`…) the
    /// village has, where told: those many, as far as there is room, rather
    /// than as the pack's shares draw them.
    pub counts: Vec<(String, u32)>,
    /// Whether `houses` was typed by hand (kept as the village is resized),
    /// rather than worked out from its area and density.
    pub houses_by_hand: bool,
}

/// How many houses a village of `area` holds at `density` (0.2 sparse … 1
/// tight): about one for every 1,800 blocks of its area at full density, as
/// many as its roads find room for on flat land (a square of 112 blocks at
/// 0.35, 3; of 192 at 0.6, 12; of 320 at 0.85, 48).
pub fn houses_for(area: massif_core::coords::BlockRect, density: f32) -> u32 {
    let blocks = ((area.x1 - area.x0).max(0) as f32) * ((area.z1 - area.z0).max(0) as f32);
    ((blocks * density.clamp(0.2, 1.0) / 1800.0).round() as u32).clamp(3, 120)
}

impl Default for Params {
    fn default() -> Self {
        Params { houses: None, size_bias: 0.0, road: RoadType::Theme, walls: false, density: 0.6, spread: 0.5, flat: 0.7, fields: 1.0, props: 0.5, mill: MillMode::Auto, max_slope: 10, margin: 1, street_width: None, foundation: None, villagers: None, counts: Vec::new(), houses_by_hand: false }
    }
}

/// What to build, and where: the village brush's request.
pub struct Request<'a> {
    pub theme: &'a Theme,
    /// The selection: its bounds and whether a column is in it.
    pub rect: BlockRect,
    pub inside: &'a (dyn Fn(i32, i32) -> bool + Sync),
    pub seed: u64,
    pub params: Params,
    pub id: u32,
}

/// Every stage's result, for one village.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stages {
    pub area: Area,
    pub site: site::Site,
    pub roads: roads::Roads,
    pub walls: Option<walls::Walls>,
    pub plots: Vec<plots::Plot>,
    pub buildings: buildings::Buildings,
    pub props: props::Props,
}

/// A cell's height (the first free block above the ground) and whether
/// water lies on it.
pub(crate) fn height(ground: &dyn Ground, x: i32, z: i32) -> (i32, bool) {
    let (f, s) = (ground.floor(x, z), ground.surface(x, z));
    (f, s > f)
}

pub(crate) fn dist2(a: [i32; 2], b: [i32; 2]) -> i64 {
    let (dx, dz) = ((a[0] - b[0]) as i64, (a[1] - b[1]) as i64);
    dx * dx + dz * dz
}

/// Run the six stages for `req` on `ground` (`lib`: the game's files, for
/// the game's themes).
pub fn stages(lib: Option<&Library>, ground: &dyn Ground, req: &Request) -> Result<(Stages, Option<buildings::Jigsaw>), String> {
    let theme = req.theme;
    let p = &req.params;
    let area = Area::new(req.rect, req.inside);
    let first = site::site(ground, &area, p, req.seed)?;
    // The game's villages: assembled in each district first, their
    // buildings then kept clear of by the roads.
    let jigsaw = match &theme.vanilla {
        Some(kind) => Some(buildings::game_districts(lib.ok_or("the game's villages need the game: no Minecraft was found")?, ground, &area, &first, kind, req.seed)?),
        None => None,
    };
    let blocked = jigsaw.as_ref().map(|j| j.taken.clone()).unwrap_or_default();
    // With a number of houses asked for, the village reaches further until
    // it holds them (or the area is full).
    let mut k = 0;
    let (site, roads, walls, plots, buildings) = loop {
        let mut site = first.clone();
        site.reach = (site.reach as f32 * 1.4f32.powi(k)) as i32;
        let mut roads = roads::roads(ground, &area, &site, &blocked, theme, p, req.seed);
        if let Some(j) = &jigsaw {
            roads.extra.extend(j.streets.iter().copied());
        }
        let walls = p.walls.then(|| walls::walls(&area, &roads, p.houses.is_some().then_some(&site)));
        if let Some(w) = &walls {
            walls::trim(&mut roads, w);
        }
        let plots = if theme.vanilla.is_some() { Vec::new() } else { plots::plots(&roads, &site, p, req.seed) };
        let reserved = walls.as_ref().map(|w| w.reserved()).unwrap_or_default();
        let within = match &walls {
            Some(w) => w.plots_within(&plots),
            None => plots.clone(),
        };
        let buildings = buildings::buildings(ground, &area, &site, &roads, &reserved, &blocked, &within, theme, p, req.seed);
        let short = buildings.notes.iter().any(|n| n.contains("houses asked for"));
        k += 1;
        if !short || k >= 4 || theme.vanilla.is_some() {
            break (site, roads, walls, plots, buildings);
        }
    };
    // Walls round a village of so many houses: drawn again round what was
    // built, so that it does not stand in a ring of empty land.
    let mut roads = roads;
    let mut walls = walls;
    if p.walls && p.houses.is_some() && theme.vanilla.is_none() {
        let mut boxes: Vec<([i32; 2], [i32; 2])> = buildings.placed.iter().map(|b| ([b.min[0], b.min[2]], [b.max[0], b.max[2]])).collect();
        boxes.extend(buildings.fields.iter().map(|f| ([f.rect[0] - 1, f.rect[1] - 1], [f.rect[2] + 1, f.rect[3] + 1])));
        if let Some(w) = walls::round_buildings(&area, &roads, &boxes) {
            walls::trim(&mut roads, &w);
            walls = Some(w);
        }
    }
    // Side roads no door opens on go (the game's villages keep their own).
    if theme.vanilla.is_none() {
        let doors: Vec<[i32; 2]> = buildings.placed.iter().map(|b| [b.door[0], b.door[2]]).collect();
        roads::prune(&mut roads, &doors);
    }
    let reserved = walls.as_ref().map(|w| w.reserved()).unwrap_or_default();
    let props = props::props(ground, &area, &site, &roads, &reserved, &buildings, theme, p, req.seed);
    Ok((Stages { area, site, roads, walls, plots, buildings, props }, jigsaw))
}

/// Build the village `req` asks for on `ground`: the six stages, laid.
pub fn build(lib: Option<&Library>, ground: &dyn Ground, req: &Request) -> Result<Built, String> {
    let (stages, jigsaw) = stages(lib, ground, req)?;
    lay::lay(lib, &lay::DiskPlans::default(), ground, req.theme, &req.params, &stages, jigsaw.as_ref(), req.seed, req.id)
}

#[cfg(test)]
mod tests;
