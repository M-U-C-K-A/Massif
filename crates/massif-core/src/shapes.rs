//! Smooth terrain: the top of the land built of partial blocks when it is
//! exported, so that it follows its precise height rather than whole
//! blocks: layers (eighths of a block, as snow's), slabs, stairs, vertical
//! slabs and corner pieces, each the shape of the block at the surface.
//!
//! One switch for the whole map ([`SmoothSettings::on`]) and, for each
//! terrain, the shapes it may use ([`SmoothSettings::shapes_of`]): set by
//! hand, or the defaults of its top block. The smoothing follows the
//! terrain painted: repainted, a column takes the shapes of its new block,
//! with nothing to update by hand. Where it is done can also be painted
//! over the map's setting, in each ground tile's [`crate::Tile::slabs`]
//! plane (the Smoothing brush): on or off here, or one slab chosen by hand.
//!
//! Which block is a block's layers, slab, stairs… is found by name among
//! the blocks known to exist (the game's for the version exported to, the
//! mods' read), from suffixes: the game's (`_slab`, `_stairs`, snow's
//! layers) and those mod profiles add as data (`profile.json` ▸ `smooth`),
//! in their own namespace or another's (a mod's `grass_block_layer` for the
//! game's grass block). No mod is named here.
//!
//! The piece of each column comes from the ground's precise height
//! ([`choose`]): its fraction of a block picks the layers' count (or a
//! slab's half), and where the land steps up a block beside it, a stair
//! faces the step, an inner corner takes an inner stair, an outer corner an
//! outer stair or a corner piece, and the foot of a cliff a vertical slab.

use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::level::TileRef;
use crate::registry::{BlockId, Registry, TerrainId};
use crate::world::Dimension;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------- shapes --

/// A kind of partial block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// Eighths of a block, as snow's layers.
    Layers,
    Slab,
    Stairs,
    VerticalSlab,
    /// A corner piece, at the outer corners of steps.
    Corner,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Layers, Shape::Slab, Shape::Stairs, Shape::VerticalSlab, Shape::Corner];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Layers => "Layers",
            Shape::Slab => "Slabs",
            Shape::Stairs => "Stairs",
            Shape::VerticalSlab => "Vertical slabs",
            Shape::Corner => "Corners",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of shapes, written as their list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "Vec<Shape>", into = "Vec<Shape>")]
pub struct ShapeSet(pub u8);

impl ShapeSet {
    pub const NONE: ShapeSet = ShapeSet(0);

    pub fn of(shapes: &[Shape]) -> Self {
        ShapeSet(shapes.iter().fold(0, |b, s| b | s.bit()))
    }

    pub fn has(self, s: Shape) -> bool {
        self.0 & s.bit() != 0
    }

    pub fn set(&mut self, s: Shape, on: bool) {
        if on {
            self.0 |= s.bit();
        } else {
            self.0 &= !s.bit();
        }
    }

    pub fn and(self, o: ShapeSet) -> ShapeSet {
        ShapeSet(self.0 & o.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl From<Vec<Shape>> for ShapeSet {
    fn from(v: Vec<Shape>) -> Self {
        ShapeSet::of(&v)
    }
}

impl From<ShapeSet> for Vec<Shape> {
    fn from(s: ShapeSet) -> Self {
        Shape::ALL.into_iter().filter(|&k| s.has(k)).collect()
    }
}

/// The map's Smooth terrain setting.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SmoothSettings {
    /// Smooth the whole map.
    #[serde(default)]
    pub on: bool,
    /// The shapes of the terrains set by hand, by terrain name; the others
    /// take the defaults of their top block.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terrains: BTreeMap<String, ShapeSet>,
}

impl SmoothSettings {
    pub fn is_off(&self) -> bool {
        !self.on && self.terrains.is_empty()
    }

    /// The shapes terrain `t` may use.
    pub fn shapes_of(&self, reg: &Registry, t: TerrainId) -> ShapeSet {
        let def = reg.terrain(t);
        match self.terrains.get(&def.name) {
            Some(s) => *s,
            None => top_block(reg, t).map_or(ShapeSet::NONE, |b| default_shapes(reg.block(b).name())),
        }
    }
}

/// The block at the top of a terrain: the first of its first stratum.
pub fn top_block(reg: &Registry, t: TerrainId) -> Option<BlockId> {
    reg.terrain(t).strata.first().and_then(|s| s.blocks.first()).map(|b| b.0)
}

// ------------------------------------------------- the profiles' data --

/// What a mod profile says of its partial blocks (`profile.json` ▸
/// `smooth`): the suffixes of each shape's blocks, and the shapes the
/// blocks it names by words may use by default.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct SmoothProfile {
    /// The suffixes of each shape's blocks (`"layers": ["_layer"]`), in the
    /// mod's namespace: for its own blocks and the game's.
    #[serde(default)]
    pub suffixes: BTreeMap<Shape, Vec<String>>,
    /// The shapes blocks may use by default: the first rule with a word of
    /// the block's name.
    #[serde(default)]
    pub defaults: Vec<SmoothDefault>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SmoothDefault {
    /// Words of block names (`sand`, `limestone`).
    #[serde(rename = "match")]
    pub words: Vec<String>,
    pub shapes: Vec<Shape>,
}

/// The game's own: slabs and stairs, and snow's layers (see [`family`]).
fn vanilla() -> &'static SmoothProfile {
    static V: std::sync::OnceLock<SmoothProfile> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let rock = ["stone", "cobblestone", "andesite", "diorite", "granite", "deepslate", "tuff", "blackstone", "sandstone", "bricks", "brick", "basalt", "calcite", "planks", "prismarine", "quartz", "purpur", "terracotta"];
        SmoothProfile {
            suffixes: BTreeMap::from([(Shape::Slab, vec!["_slab".into()]), (Shape::Stairs, vec!["_stairs".into()])]),
            defaults: vec![
                SmoothDefault { words: vec!["snow".into()], shapes: vec![Shape::Layers] },
                SmoothDefault { words: rock.map(String::from).to_vec(), shapes: vec![Shape::Slab, Shape::Stairs] },
            ],
        }
    })
}

/// The profiles that say something of partial blocks, with their
/// namespace: the mods' first, then the game's.
fn profiles() -> Vec<(&'static str, &'static SmoothProfile)> {
    let mut out: Vec<(&str, &SmoothProfile)> = crate::mod_profiles::all().iter().filter_map(|p| p.smooth.as_ref().map(|s| (p.namespace.as_str(), s))).collect();
    out.push(("minecraft", vanilla()));
    out
}

/// The shapes a block (without properties) may use by default.
pub fn default_shapes(name: &str) -> ShapeSet {
    let path = name.rsplit(':').next().unwrap_or(name);
    let words: Vec<&str> = path.split('_').collect();
    for (_, p) in profiles() {
        for rule in &p.defaults {
            if rule.words.iter().any(|w| words.contains(&w.as_str())) {
                return ShapeSet::of(&rule.shapes);
            }
        }
    }
    ShapeSet::NONE
}

// -------------------------------------------------------------- families --

/// What blocks are known to exist, and their properties: the game's (by
/// `vanilla`), the mods' (by id, with their properties' names).
pub struct Known<'a> {
    pub vanilla: &'a dyn Fn(&str) -> bool,
    pub mods: &'a HashMap<String, Vec<String>>,
}

impl Known<'_> {
    pub fn exists(&self, id: &str) -> bool {
        match id.strip_prefix("minecraft:") {
            Some(_) => (self.vanilla)(id),
            None => self.mods.contains_key(id),
        }
    }

    /// Whether block `id` has property `p` (the game's blocks: as the game
    /// has them, for the shapes' blocks).
    pub fn has_property(&self, id: &str, p: &str) -> bool {
        match id.strip_prefix("minecraft:") {
            Some(short) => match p {
                "layers" => short == "snow",
                "waterlogged" => short.ends_with("_slab") || short.ends_with("_stairs"),
                _ => false,
            },
            // A mod's block whose properties are not known (read from its
            // jar without a list of them): as the game's shapes have them.
            None => match self.mods.get(id) {
                Some(ps) if ps.is_empty() => p == "waterlogged" && !id.ends_with("_layer"),
                Some(ps) => ps.iter().any(|q| q == p),
                None => false,
            },
        }
    }
}

/// The mods' blocks map `reg` uses itself (its terrains', its covers'),
/// with the properties their states name, that `mods` does not list: a map
/// of a mod's blocks knows them whether or not the mod is read (a
/// WorldPainter map of Conquest Reforged, without its jar). `None` when it
/// uses none that `mods` lacks.
pub fn with_map_blocks(reg: &Registry, mods: &HashMap<String, Vec<String>>) -> Option<HashMap<String, Vec<String>>> {
    let mut extra: HashMap<String, Vec<String>> = HashMap::new();
    let states = reg.blocks.iter().map(|b| b.state.as_str()).chain(reg.covers.iter().filter(|c| !c.removed).flat_map(|c| c.plants.iter().map(|p| p.0.as_str())));
    for state in states {
        let (name, props) = state.split_once('[').map_or((state, ""), |(n, p)| (n, p.trim_end_matches(']')));
        if name.starts_with("minecraft:") || !name.contains(':') || mods.contains_key(name) {
            continue;
        }
        let list = extra.entry(name.to_owned()).or_default();
        for key in props.split(',').filter_map(|kv| kv.split_once('=').map(|p| p.0.trim())) {
            if !list.iter().any(|k| k == key) {
                list.push(key.to_owned());
            }
        }
    }
    if extra.is_empty() {
        return None;
    }
    let mut all = mods.clone();
    all.extend(extra);
    Some(all)
}

/// The partial blocks of one block: for each shape, the block (by name).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Family {
    pub layers: Option<String>,
    pub slab: Option<String>,
    /// Its slab has a `layers` property: it goes by eighths.
    pub slab_layers: bool,
    pub stairs: Option<String>,
    pub vertical_slab: Option<String>,
    pub corner: Option<String>,
}

impl Family {
    pub fn get(&self, s: Shape) -> Option<&str> {
        match s {
            Shape::Layers => self.layers.as_deref(),
            Shape::Slab => self.slab.as_deref(),
            Shape::Stairs => self.stairs.as_deref(),
            Shape::VerticalSlab => self.vertical_slab.as_deref(),
            Shape::Corner => self.corner.as_deref(),
        }
    }

    /// The shapes it has.
    pub fn shapes(&self) -> ShapeSet {
        let mut s = ShapeSet::NONE;
        for k in Shape::ALL {
            s.set(k, self.get(k).is_some());
        }
        s
    }
}

/// The names a block's shapes may grow from: its own and shorter forms
/// (`stone_bricks` → `stone_brick`, `oak_planks` → `oak`, `quartz_block`
/// → `quartz`).
fn bases(path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        if !p.is_empty() && !out.iter().any(|o| o == p) {
            out.push(p.to_owned());
        }
    };
    push(path);
    for suffix in ["_planks", "_block", "_blocks"] {
        if let Some(p) = path.strip_suffix(suffix) {
            push(p);
        }
    }
    if let Some(p) = path.strip_suffix('s') {
        push(p);
    }
    out
}

/// The family of block `name` (without properties) among the blocks
/// `known`. A block that is itself a slab (a slab chosen by hand) is its
/// own slab, and its stairs those of its base.
pub fn family(name: &str, known: &Known) -> Family {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let path = path.strip_suffix("_slab").unwrap_or(path);
    let profiles = profiles();
    let find = |shape: Shape| -> Option<String> {
        if shape == Shape::Layers && ns == "minecraft" && (path == "snow_block" || path == "snow") {
            return Some("minecraft:snow".into());
        }
        // A block of layers (whole at eight of them: a mod's layers with no
        // full block of their own) lays its own.
        if shape == Shape::Layers && known.exists(name) && profiles.iter().filter(|(pns, _)| *pns == ns).any(|(_, p)| p.suffixes.get(&Shape::Layers).into_iter().flatten().any(|s| path.ends_with(s.as_str()))) {
            return Some(name.to_owned());
        }
        for base in bases(path) {
            // The block's own namespace first, then the others'.
            for (pns, p) in profiles.iter().filter(|(pns, _)| *pns == ns).chain(profiles.iter().filter(|(pns, _)| *pns != ns)) {
                for suffix in p.suffixes.get(&shape).into_iter().flatten() {
                    let id = format!("{pns}:{base}{suffix}");
                    if known.exists(&id) {
                        return Some(id);
                    }
                }
            }
        }
        None
    };
    let slab = if name.ends_with("_slab") && known.exists(name) { Some(name.to_owned()) } else { find(Shape::Slab) };
    let slab_layers = slab.as_deref().is_some_and(|s| known.has_property(s, "layers"));
    Family {
        layers: find(Shape::Layers),
        slab,
        slab_layers,
        stairs: find(Shape::Stairs),
        vertical_slab: find(Shape::VerticalSlab),
        corner: find(Shape::Corner),
    }
}

// ---------------------------------------------------------------- pieces --

/// The four sides, clockwise from north, as the game names a facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    pub const fn id(self) -> &'static str {
        match self {
            Side::North => "north",
            Side::East => "east",
            Side::South => "south",
            Side::West => "west",
        }
    }

    /// The step to the neighbour on this side.
    pub const fn step(self) -> (i32, i32) {
        match self {
            Side::North => (0, -1),
            Side::East => (1, 0),
            Side::South => (0, 1),
            Side::West => (-1, 0),
        }
    }
}

/// The shape of a stair, as the game names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StairShape {
    Straight,
    InnerRight,
    OuterRight,
}

impl StairShape {
    pub const fn id(self) -> &'static str {
        match self {
            StairShape::Straight => "straight",
            StairShape::InnerRight => "inner_right",
            StairShape::OuterRight => "outer_right",
        }
    }
}

/// A partial block, as placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `n` eighths (1 to 7).
    Layers(u8),
    /// A bottom slab, `n` eighths high (4 for a plain slab).
    Slab(u8),
    Stairs(Side, StairShape),
    /// Against the side it faces.
    VerticalSlab(Side),
    /// A corner piece, facing the higher corner (its right, as a stair's).
    Corner(Side),
}

impl Kind {
    pub fn shape(self) -> Shape {
        match self {
            Kind::Layers(_) => Shape::Layers,
            Kind::Slab(_) => Shape::Slab,
            Kind::Stairs(..) => Shape::Stairs,
            Kind::VerticalSlab(_) => Shape::VerticalSlab,
            Kind::Corner(_) => Shape::Corner,
        }
    }

    /// How high it is drawn, in eighths (the 3D View draws every piece as
    /// a box: stairs three quarters high, vertical slabs and corners half).
    pub fn eighths(self) -> u8 {
        match self {
            Kind::Layers(n) | Kind::Slab(n) => n,
            Kind::Stairs(..) => 6,
            Kind::VerticalSlab(_) | Kind::Corner(_) => 4,
        }
    }
}

/// A piece at height `y` of its column; `replaces` when it takes the place
/// of the column's top block (the land's precise height under the block's
/// middle), else it lies on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub y: i32,
    pub kind: Kind,
    pub replaces: bool,
}

/// A column to smooth, from the heights: its top (`y1`, the block over
/// its top block), its precise height in eighths over the bottom of its
/// top block (4 to 12), and the neighbours one block higher (`up`: bit k
/// for side k of [`Side::ALL`]; `diag`: bit k for the corner between side
/// k and the next clockwise), two or more higher (`cliff`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub cell: u16,
    pub y1: i32,
    pub eighths: u8,
    pub up: u8,
    pub diag: u8,
    pub cliff: u8,
    pub terrain: TerrainId,
    /// A slab chosen by hand (the Smoothing brush), whatever the block.
    pub block: Option<BlockId>,
}

/// The piece of a column, of the shapes it `may` use (those allowed and
/// its block has), and whether its slab goes by eighths.
pub fn choose(c: &Column, may: ShapeSet, slab_layers: bool) -> Option<Piece> {
    let e = c.eighths;
    // Its precise height: a part of a block over or in place of its top.
    let by_eighths = |kind: fn(u8) -> Kind| match e {
        0..=7 => Some(Piece { y: c.y1 - 1, kind: kind(e.max(1)), replaces: true }),
        8 => None,
        _ => Some(Piece { y: c.y1, kind: kind((e - 8).min(7)), replaces: false }),
    };
    let height = if may.has(Shape::Layers) {
        by_eighths(Kind::Layers)
    } else if may.has(Shape::Slab) && slab_layers {
        by_eighths(Kind::Slab)
    } else if may.has(Shape::Slab) {
        // By halves: under its middle, a slab in its place; a quarter
        // over, a slab on it.
        match e {
            0..=5 => Some(Piece { y: c.y1 - 1, kind: Kind::Slab(4), replaces: true }),
            10.. => Some(Piece { y: c.y1, kind: Kind::Slab(4), replaces: false }),
            _ => None,
        }
    } else {
        None
    };
    // A step up beside it: what fills its foot, on the column.
    let on = |kind| Some(Piece { y: c.y1, kind, replaces: false });
    let ups: Vec<usize> = (0..4).filter(|k| c.up & (1 << k) != 0).collect();
    let step = if !ups.is_empty() {
        let corner = (0..4).find(|&k| c.up & (1 << k) != 0 && c.up & (1 << ((k + 1) % 4)) != 0);
        if may.has(Shape::Stairs) {
            match (corner, ups.len()) {
                (Some(k), 2) => on(Kind::Stairs(Side::ALL[k], StairShape::InnerRight)),
                (_, 1) => on(Kind::Stairs(Side::ALL[ups[0]], StairShape::Straight)),
                _ => on(Kind::Slab(4)),
            }
        } else if may.has(Shape::Slab) {
            on(Kind::Slab(4))
        } else if may.has(Shape::Layers) {
            on(Kind::Layers(4))
        } else {
            None
        }
    } else if c.diag.count_ones() == 1 {
        // An outer corner: only the land across a corner is higher.
        let k = c.diag.trailing_zeros() as usize;
        if may.has(Shape::Corner) {
            on(Kind::Corner(Side::ALL[k]))
        } else if may.has(Shape::Stairs) {
            on(Kind::Stairs(Side::ALL[k], StairShape::OuterRight))
        } else {
            None
        }
    } else if c.cliff != 0 && may.has(Shape::VerticalSlab) {
        on(Kind::VerticalSlab(Side::ALL[c.cliff.trailing_zeros() as usize]))
    } else {
        None
    };
    // A piece of its own height wins, unless it lies on the column where a
    // step's piece would (the step shapes the foot better).
    match (height, step) {
        (Some(h), Some(s)) if !h.replaces => Some(s.max_by_height(h)),
        (Some(h), _) => Some(h),
        (None, s) => s,
    }
}

impl Piece {
    fn max_by_height(self, o: Piece) -> Piece {
        if o.kind.eighths() > self.kind.eighths() && matches!(self.kind, Kind::Slab(_) | Kind::Layers(_)) { o } else { self }
    }
}

// ----------------------------------------------------------- the brush --

/// The value of a column of the slabs plane that follows the map.
pub const FOLLOW: u16 = 0;
/// Smoothed here, whatever the map's switch.
pub const AUTO: u16 = 1;
/// Not smoothed here, whatever the map's switch.
pub const OFF: u16 = 0x7fff;
/// The bit of a slab chosen by hand that also asks for its stairs.
pub const STAIRS: u16 = 0x8000;

/// The value of a column of the slabs plane with a slab chosen by hand
/// (`block`), or smoothed here as the map's terrains ask (`None`).
pub fn value(block: Option<BlockId>, stairs: bool) -> u16 {
    match block {
        Some(b) => (b.0 + 2).min(OFF - 1) | if stairs { STAIRS } else { 0 },
        None => AUTO,
    }
}

/// What a value of the slabs plane asks for: `None` to follow the map
/// (0), `Some(None)` not to smooth, else whether to smooth and with which
/// slab chosen by hand (and its stairs).
pub fn decode(v: u16) -> Option<Option<(Option<BlockId>, bool)>> {
    match v & !STAIRS {
        FOLLOW => None,
        OFF => Some(None),
        AUTO => Some(Some((None, false))),
        n => Some(Some((Some(BlockId(n - 2)), v & STAIRS != 0))),
    }
}

/// The columns of the ground tile at `pos` to smooth, by cell, as the map
/// (`settings`) and the brush ask, from the ground's heights, its own and
/// its neighbours' (across tiles). Empty where nothing is smoothed.
pub fn plan_tile(dim: &Dimension, settings: &SmoothSettings, pos: TilePos) -> Vec<Column> {
    let ground = dim.ground();
    let Some(here) = ground.view(pos) else { return Vec::new() };
    if !settings.on && here.slabs.uniform_value() == Some(FOLLOW) {
        return Vec::new();
    }
    // The tiles around, read when a column at an edge needs one.
    let mut near: HashMap<(i32, i32), Option<TileRef>> = HashMap::new();
    let mut out = Vec::new();
    for lz in 0..TILE_SIZE {
        for lx in 0..TILE_SIZE {
            let i = local_index(lx, lz);
            let block = match decode(here.slabs.get(i)) {
                None if settings.on => None,
                None | Some(None) => continue,
                Some(Some((b, _))) => b,
            };
            let h = here.top.get(i);
            let y1 = crate::compose::block_top(h);
            let eighths = ((h - (y1 - 1) as f32) * 8.0).round().clamp(4.0, 12.0) as u8;
            let (mut up, mut diag, mut cliff) = (0u8, 0u8, 0u8);
            for (k, side) in Side::ALL.into_iter().enumerate() {
                let (dx, dz) = side.step();
                match neighbour_top(ground, &here, pos, &mut near, lx + dx, lz + dz) {
                    Some(t) if t == y1 + 1 => up |= 1 << k,
                    Some(t) if t >= y1 + 2 => cliff |= 1 << k,
                    _ => {}
                }
                let (ex, ez) = Side::ALL[(k + 1) % 4].step();
                if neighbour_top(ground, &here, pos, &mut near, lx + dx + ex, lz + dz + ez) == Some(y1 + 1) {
                    diag |= 1 << k;
                }
            }
            out.push(Column { cell: i as u16, y1, eighths, up, diag, cliff, terrain: TerrainId(here.terrain.get(i)), block });
        }
    }
    out
}

/// The block top of the ground at `(lx, lz)` from tile `pos`'s corner,
/// in `here` or a tile around it (read once into `near`).
fn neighbour_top<'a>(ground: &'a crate::level::Level, here: &crate::tile::Tile, pos: TilePos, near: &mut HashMap<(i32, i32), Option<TileRef<'a>>>, lx: i32, lz: i32) -> Option<i32> {
    if (0..TILE_SIZE).contains(&lx) && (0..TILE_SIZE).contains(&lz) {
        return Some(crate::compose::block_top(here.top.get(local_index(lx, lz))));
    }
    let (tx, tz) = (lx.div_euclid(TILE_SIZE), lz.div_euclid(TILE_SIZE));
    let tile = near.entry((tx, tz)).or_insert_with(|| ground.view(pos.offset(tx, tz)));
    let (ox, oz) = pos.origin();
    tile.as_ref().map(|t| crate::compose::block_top(t.top.get(local_index(ox + lx, oz + lz))))
}

/// Paint the whole map's slabs plane with `value` (FOLLOW clears it),
/// within the selection when there is one.
pub fn fill_slabs(edit: &mut crate::history::Edit, value: u16) {
    let positions: Vec<TilePos> = edit.level().positions().collect();
    let selection = edit.dim().selection.clone();
    let rect = positions.iter().fold(crate::coords::BlockRect::new(0, 0, 0, 0), |r, p| {
        let t = p.rect();
        if r.is_empty() { t } else { crate::coords::BlockRect::new(r.x0.min(t.x0), r.z0.min(t.z0), r.x1.max(t.x1), r.z1.max(t.z1)) }
    });
    edit.par_tiles(positions, rect, false, |pos, _, tile| {
        if tile.bottom.is_some() {
            return;
        }
        let (ox, oz) = pos.origin();
        match &selection {
            None => tile.slabs = crate::plane::Plane::Uniform(value),
            Some(sel) => {
                for lz in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        if sel.get(ox + lx, oz + lz) >= crate::selection::HALF {
                            tile.slabs.set(local_index(lx, lz), value);
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::tile::Tile;
    use crate::world::{World, WorldMeta};

    fn mods() -> HashMap<String, Vec<String>> {
        let props = |p: &str| p.split(',').map(String::from).collect::<Vec<_>>();
        HashMap::from([
            ("conquest:grass_block_layer".into(), props("layers")),
            ("conquest:sand_layer".into(), props("layers")),
            ("conquest:gray_limestone_slab".into(), props("layers,type,waterlogged")),
            ("conquest:gray_limestone_stairs".into(), props("facing,half,shape,waterlogged")),
            ("conquest:gray_limestone_vertical_slab".into(), props("extension_toggle,facing,layer,waterlogged")),
            ("conquest:gray_limestone_corner_slab".into(), props("facing,type,waterlogged")),
        ])
    }

    fn vanilla(id: &str) -> bool {
        ["minecraft:stone_slab", "minecraft:stone_stairs", "minecraft:oak_slab", "minecraft:oak_stairs", "minecraft:stone_brick_slab", "minecraft:snow"].contains(&id)
    }

    fn fake_profile() -> SmoothProfile {
        serde_json::from_str(r#"{"suffixes": {"layers": ["_layer"], "slab": ["_slab"], "stairs": ["_stairs"], "vertical_slab": ["_vertical_slab"], "corner": ["_corner_slab"]}, "defaults": [{"match": ["grass", "sand"], "shapes": ["layers"]}]}"#).unwrap()
    }

    #[test]
    fn families_are_found_by_name_in_every_namespace() {
        let m = mods();
        let known = Known { vanilla: &vanilla, mods: &m };
        let f = family("minecraft:stone", &known);
        assert_eq!((f.slab.as_deref(), f.stairs.as_deref(), f.layers.as_deref()), (Some("minecraft:stone_slab"), Some("minecraft:stone_stairs"), None));
        assert_eq!(family("minecraft:oak_planks", &known).slab.as_deref(), Some("minecraft:oak_slab"));
        assert_eq!(family("minecraft:stone_bricks", &known).slab.as_deref(), Some("minecraft:stone_brick_slab"));
        assert_eq!(family("minecraft:snow_block", &known).layers.as_deref(), Some("minecraft:snow"));
        assert!(family("minecraft:dirt", &known).shapes().is_empty());
        // A slab chosen by hand is its own slab, with its stairs.
        let oak = family("minecraft:oak_slab", &known);
        assert_eq!((oak.slab.as_deref(), oak.stairs.as_deref()), (Some("minecraft:oak_slab"), Some("minecraft:oak_stairs")));
    }

    #[test]
    fn the_shape_follows_the_precise_height() {
        let c = |e: u8, up: u8, diag: u8, cliff: u8| Column { cell: 0, y1: 64, eighths: e, up, diag, cliff, terrain: TerrainId(0), block: None };
        let layers = ShapeSet::of(&[Shape::Layers]);
        // Three eighths under the top block's top: three eighths in its place.
        assert_eq!(choose(&c(5, 0, 0, 0), layers, false), Some(Piece { y: 63, kind: Kind::Layers(5), replaces: true }));
        assert_eq!(choose(&c(8, 0, 0, 0), layers, false), None);
        assert_eq!(choose(&c(11, 0, 0, 0), layers, false), Some(Piece { y: 64, kind: Kind::Layers(3), replaces: false }));
        // Slabs by halves, or by eighths when the slab has layers.
        let slab = ShapeSet::of(&[Shape::Slab]);
        assert_eq!(choose(&c(5, 0, 0, 0), slab, false).map(|p| (p.y, p.kind)), Some((63, Kind::Slab(4))));
        assert_eq!(choose(&c(7, 0, 0, 0), slab, false), None);
        assert_eq!(choose(&c(10, 0, 0, 0), slab, false).map(|p| (p.y, p.kind)), Some((64, Kind::Slab(4))));
        assert_eq!(choose(&c(6, 0, 0, 0), slab, true).map(|p| p.kind), Some(Kind::Slab(6)));
        // Steps: a stair facing the step, an inner stair in an inner corner,
        // an outer one (or a corner piece) at an outer corner, a vertical
        // slab at a cliff's foot.
        let all = ShapeSet::of(&Shape::ALL);
        let rock = ShapeSet::of(&[Shape::Slab, Shape::Stairs]);
        assert_eq!(choose(&c(8, 0b0001, 0, 0), rock, false).map(|p| p.kind), Some(Kind::Stairs(Side::North, StairShape::Straight)));
        assert_eq!(choose(&c(8, 0b0011, 0, 0), rock, false).map(|p| p.kind), Some(Kind::Stairs(Side::North, StairShape::InnerRight)));
        assert_eq!(choose(&c(8, 0, 0b0010, 0), rock, false).map(|p| p.kind), Some(Kind::Stairs(Side::East, StairShape::OuterRight)));
        assert_eq!(choose(&c(8, 0, 0b0010, 0), all, false).map(|p| p.kind), Some(Kind::Corner(Side::East)));
        assert_eq!(choose(&c(8, 0, 0, 0b0100), ShapeSet::of(&[Shape::VerticalSlab]), false).map(|p| p.kind), Some(Kind::VerticalSlab(Side::South)));
        // Grass: layers at the foot of a step, when its height asks none.
        assert_eq!(choose(&c(8, 0b0001, 0, 0), layers, false).map(|p| p.kind), Some(Kind::Layers(4)));
        assert_eq!(choose(&c(8, 0b0001, 0, 0), ShapeSet::NONE, false), None);
    }

    #[test]
    fn profiles_add_suffixes_and_defaults_as_data() {
        let p = fake_profile();
        assert_eq!(p.suffixes[&Shape::Layers], ["_layer"]);
        assert_eq!(ShapeSet::of(&p.defaults[0].shapes), ShapeSet::of(&[Shape::Layers]));
        // Rock takes slabs and stairs (and more, of a mod's profile, where
        // the mod has them), snow layers; a block no rule names, none.
        let rock = default_shapes("minecraft:stone");
        assert!(rock.has(Shape::Slab) && rock.has(Shape::Stairs), "{rock:?}");
        assert!(default_shapes("minecraft:snow_block").has(Shape::Layers));
        assert!(default_shapes("minecraft:glowstone").is_empty());
        // Written as their names.
        let json = serde_json::to_string(&SmoothSettings { on: true, terrains: BTreeMap::from([("Grass".into(), ShapeSet::of(&[Shape::Layers, Shape::Stairs]))]) }).unwrap();
        assert_eq!(json, r#"{"on":true,"terrains":{"Grass":["layers","stairs"]}}"#);
        let back: SmoothSettings = serde_json::from_str(&json).unwrap();
        assert!(back.on && back.terrains["Grass"].has(Shape::Stairs));
    }

    #[test]
    fn a_tile_plans_its_columns_across_its_edges() {
        let mut world = World::new(WorldMeta::default());
        let dim = &mut world.dims[0];
        // Two tiles side by side: the east one a block higher; the west
        // one at 63.7 (its top block's middle at 63.5: 5 eighths over it).
        let (a, b) = (TilePos::new(0, 0), TilePos::new(1, 0));
        dim.ground_mut().insert(a, Tile::ground(63.7, 1, crate::NO_FLUID));
        dim.ground_mut().insert(b, Tile::ground(65.0, 1, crate::NO_FLUID));
        let settings = SmoothSettings { on: true, ..Default::default() };
        let plan = plan_tile(&world.dims[0], &settings, a);
        assert_eq!(plan.len(), (TILE_SIZE * TILE_SIZE) as usize);
        let edge = plan.iter().find(|c| c.cell as usize == local_index(TILE_SIZE - 1, 5)).unwrap();
        assert_eq!((edge.y1, edge.eighths, edge.up, edge.diag), (64, 6, 0b0010, 0b0011));
        // Off by the brush, or the map's switch off: nothing.
        let mut h = History::new(4);
        h.begin("off", 0);
        fill_slabs(&mut h.edit(&mut world, 0), OFF);
        h.commit();
        assert!(plan_tile(&world.dims[0], &settings, a).is_empty());
        assert!(plan_tile(&world.dims[0], &SmoothSettings::default(), b).is_empty());
    }
}
