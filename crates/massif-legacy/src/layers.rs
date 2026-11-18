//! WorldPainter's layers as Massif has them: forests and the trees of the
//! tree pack become object layers, plant layers ground covers, Frost the
//! frost cover (snow on the ground, ice on the water), custom ground cover
//! layers terrains, tunnel layers levels, and caverns, chasms and caves are
//! carved where painted. A combined layer is the layers it holds, with
//! their factors, and its terrain and biome.
//!
//! A layer is a key of a tile's `layerData` (a nibble or a byte a column)
//! or `bitLayerData` (a bit a column) map. Its strength in a column, 0 to
//! 255, becomes Massif's single layer of that column where a hash of the
//! column falls under it: a forest painted at half strength keeps about
//! that share of its columns, as WorldPainter plants about that share of
//! trees.

use crate::javaser::{Arena, Array, Handle, Value};
use crate::terrains::Mapper;
use massif_core::TilePos;
use massif_core::registry::{BlockId, CoverDef, Registry, Stratum, TerrainDef, TerrainId};
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::treepack::TreeKind;
use massif_core::world::Dimension;
use std::collections::HashMap;

const LAYERS: &str = "org.pepsoft.worldpainter.layers.";
/// What the layers of WorldPainter's tree pack say they are.
const TREE_PACK: &str = " (WorldPainter tree pack)";

/// How a layer's strength turns into the share of its columns kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// As much as its strength: objects and plants.
    Linear,
    /// The square of it, as WorldPainter's own forests plant their trees.
    Square,
}

/// What a WorldPainter layer becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conv {
    /// A ground cover, by number.
    Cover(u16),
    /// An object layer, by number; a sub-canopy one when `true`.
    Objects(u16, Curve, bool),
    /// Nothing at all in the column, down to the floor of the world.
    Void,
    /// Annotations: their colour, 1 to 15, a nibble a column.
    Annotations,
    /// Caves carved where painted, once the whole dimension is read.
    Caves(CaveKind),
    /// A custom ground cover: its index in [`Converter::ground_covers`].
    GroundCover(u16),
    /// A tunnel or floating layer: its index in [`Converter::tunnels`].
    Tunnel(u16),
    /// A terrain set where painted at half strength or more (a combined
    /// layer's).
    Terrain(u16),
    /// A biome likewise.
    Biome(u16),
    /// A layer of underground pockets, by number.
    Pockets(u16),
    /// WorldPainter's River: a river dug where painted, once the whole
    /// dimension is read.
    River,
}

impl Conv {
    /// Whether it is applied once the whole dimension is read, rather than
    /// tile by tile.
    pub fn after(self) -> bool {
        matches!(self, Conv::Caves(_) | Conv::GroundCover(_) | Conv::Tunnel(_) | Conv::River)
    }
}

/// A tile's layers applied once its whole dimension is read, each with its
/// strength in every column.
pub type After = Vec<(Conv, Vec<u8>)>;

/// What kind of caves a layer carves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaveKind {
    /// WorldPainter's Caverns: large halls.
    Caverns,
    /// Its Chasms: ravines.
    Chasms,
    /// Its Caves: winding tunnels.
    Tunnels,
}

impl CaveKind {
    /// Its name in WorldPainter.
    pub const fn name(self) -> &'static str {
        match self {
            CaveKind::Caverns => "Caverns",
            CaveKind::Chasms => "Chasms",
            CaveKind::Tunnels => "Caves",
        }
    }
}

/// A custom ground cover layer of WorldPainter: a layer of blocks over the
/// terrain, so many blocks thick (or into it, for a negative thickness).
#[derive(Debug, Clone, PartialEq)]
pub struct GroundCover {
    pub name: String,
    /// Block states and their counts.
    pub blocks: Vec<(String, u32)>,
    pub thickness: i32,
}

/// How a tunnel layer's floor or roof is placed (WorldPainter's
/// `TunnelLayer.Mode`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// At a height.
    Fixed,
    /// So deep under the ground.
    Depth,
    /// As deep under the level as the ground is over it.
    Inverted,
    /// So high over the floor (the roof only).
    AboveFloor,
}

/// Where a tunnel's floor or roof is: its mode, level and limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bound {
    pub mode: Mode,
    pub level: i32,
    pub min: i32,
    pub max: i32,
}

impl Bound {
    /// The height (of a block, as WorldPainter counts) over ground whose
    /// top block is at `terrain`, a floor at `floor`.
    fn at(&self, terrain: i32, floor: i32) -> i32 {
        let clamp = |v: i32| v.clamp(self.min, self.max.max(self.min));
        match self.mode {
            Mode::Fixed => self.level,
            Mode::Depth => clamp(terrain - self.level),
            Mode::Inverted => clamp(self.level - (terrain - self.level)),
            Mode::AboveFloor => clamp(floor + self.level),
        }
    }
}

/// A tunnel layer of WorldPainter: a void carved between a floor and a
/// roof, or, floating, land laid over the void.
#[derive(Debug, Clone, PartialEq)]
pub struct Tunnel {
    pub name: String,
    /// Land laid at the floor's height, as deep as the roof says, rather
    /// than a void carved.
    pub floating: bool,
    pub floor: Bound,
    pub roof: Bound,
    /// Water up to this height (a block's), if any.
    pub flood: Option<i32>,
}

impl Tunnel {
    /// The blocks a column whose top block is at `terrain` gets, in a world
    /// from `min` to `max`: carved, from the block over the floor up to the
    /// roof or the ground; floating, from its bottom up to the floor. As
    /// surface heights, `(bottom, top)`; empty where `top <= bottom`.
    pub fn span(&self, terrain: i32, min: i32, max: i32) -> (f32, f32) {
        let floor = self.floor.at(terrain, 0).clamp(min + 1, max - 1);
        if self.floating {
            let bottom = match self.roof.mode {
                Mode::Fixed => self.roof.level,
                _ => floor - self.roof.level,
            }
            .clamp(min, max);
            return (bottom as f32 + 1.0, floor as f32 + 1.0);
        }
        let roof = self.roof.at(terrain, floor).clamp(min + 1, max - 1).min(terrain);
        (floor as f32 + 1.0, roof as f32 + 1.0)
    }
}

/// What the layers of one import become: decided once for each layer, its
/// custom ground covers and tunnels kept aside for the end of the import.
#[derive(Debug, Default)]
pub struct Converter {
    known: HashMap<Handle, Vec<(Conv, f32)>>,
    pub ground_covers: Vec<GroundCover>,
    pub tunnels: Vec<Tunnel>,
}

impl Converter {
    /// What layer `key` (an object of the stream, `handle`) becomes, each
    /// part with the factor its strength takes; empty for a layer Massif
    /// does not convert.
    pub fn parts(&mut self, arena: &Arena, key: Value, handle: Handle, mapper: &mut Mapper) -> Vec<(Conv, f32)> {
        self.parts_at(arena, key, handle, mapper, 0)
    }

    fn parts_at(&mut self, arena: &Arena, key: Value, handle: Handle, mapper: &mut Mapper, depth: u32) -> Vec<(Conv, f32)> {
        if let Some(parts) = self.known.get(&handle) {
            return parts.clone();
        }
        let parts = self.resolve(arena, key, mapper, depth);
        self.known.insert(handle, parts.clone());
        parts
    }

    fn resolve(&mut self, arena: &Arena, key: Value, mapper: &mut Mapper, depth: u32) -> Vec<(Conv, f32)> {
        let Some(class) = arena.class_name(key).map(str::to_owned) else { return Vec::new() };
        let Some(short) = class.strip_prefix(LAYERS) else { return Vec::new() };
        let name = arena.field(key, "name").and_then(|v| arena.string(v)).unwrap_or(short).to_owned();
        let int = |n: &str, default: i64| arena.field(key, n).and_then(|v| arena.int(v)).unwrap_or(default);
        match short {
            "Caverns" => vec![(Conv::Caves(CaveKind::Caverns), 1.0)],
            "Chasms" => vec![(Conv::Caves(CaveKind::Chasms), 1.0)],
            "Caves" => vec![(Conv::Caves(CaveKind::Tunnels), 1.0)],
            "River" => vec![(Conv::River, 1.0)],
            "groundcover.GroundCoverLayer" => {
                let blocks = match arena.field(key, "mixedMaterial").filter(|m| *m != Value::Null).and_then(|m| crate::worldpainter::read_mixed(arena, m)) {
                    Some(mixed) if !mixed.rows.is_empty() => mixed.rows,
                    _ => match arena.field(key, "material").filter(|m| *m != Value::Null) {
                        Some(m) => vec![(crate::worldpainter::material_state(arena, m), 1)],
                        None => return Vec::new(),
                    },
                };
                let thickness = int("thickness", 1).clamp(-64, 64) as i32;
                if thickness == 0 {
                    return Vec::new();
                }
                self.ground_covers.push(GroundCover { name, blocks, thickness });
                vec![(Conv::GroundCover(self.ground_covers.len() as u16 - 1), 1.0)]
            }
            "pockets.UndergroundPocketsLayer" => {
                // Its blocks: a mixed material, a material, or a terrain's.
                let intern = |reg: &mut Registry, state: &str| reg.intern_block(state, crate::terrains::block_color(state));
                let blocks: Vec<(BlockId, u16)> = match arena.field(key, "mixedMaterial").filter(|m| *m != Value::Null).and_then(|m| crate::worldpainter::read_mixed(arena, m)) {
                    Some(mixed) if !mixed.rows.is_empty() => mixed.rows.iter().map(|(s, n)| (intern(&mut mapper.registry, s), (*n).clamp(1, 1000) as u16)).collect(),
                    _ => match arena.field(key, "material").filter(|m| *m != Value::Null) {
                        Some(m) => vec![(intern(&mut mapper.registry, &crate::worldpainter::material_state(arena, m)), 1)],
                        None => {
                            let terrain = arena.field(key, "terrain").and_then(|t| arena.enum_name(t)).and_then(|t| crate::tables::TERRAINS.iter().position(|n| *n == t));
                            let Some(ordinal) = terrain else { return Vec::new() };
                            let id = mapper.terrain(ordinal as u8);
                            mapper.registry.terrain(TerrainId(id)).strata.first().map(|s| s.blocks.iter().map(|&(b, w)| (b, w.max(1))).collect()).unwrap_or_default()
                        }
                    },
                };
                if blocks.is_empty() {
                    return Vec::new();
                }
                // Its frequency is per thousand of the rock, its scale a
                // hundredth of blobs of 4.1 blocks.
                let share = (int("frequency", 50) as f32 / 1000.0).clamp(0.001, 1.0);
                let size = (4.099 * int("scale", 100) as f32 / 100.0).max(1.0);
                let (low, high) = (int("minLevel", i32::MIN as i64) as i32, int("maxLevel", i32::MAX as i64) as i32);
                let color = arena.field(key, "colour").and_then(|c| arena.int(c)).map_or([150, 150, 150], |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                let def = massif_core::pockets::PocketDef::new(&name, color, &blocks, share, size, low, high);
                vec![(Conv::Pockets(mapper.registry.add_pocket(def)), 1.0)]
            }
            "tunnel.TunnelLayer" => {
                let mode = |n: &str, default: Mode| match arena.field(key, n).and_then(|v| arena.enum_name(v)) {
                    Some("FIXED_HEIGHT") => Mode::Fixed,
                    Some("CONSTANT_DEPTH") => Mode::Depth,
                    Some("INVERTED_DEPTH") => Mode::Inverted,
                    Some("FIXED_HEIGHT_ABOVE_FLOOR") => Mode::AboveFloor,
                    // A floor of a dimension of its own: at its level.
                    _ => default,
                };
                let bound = |prefix: &str, default: Mode, level: i64| Bound {
                    mode: mode(&format!("{prefix}Mode"), default),
                    level: int(&format!("{prefix}Level"), level) as i32,
                    min: int(&format!("{prefix}Min"), i32::MIN as i64) as i32,
                    max: int(&format!("{prefix}Max"), i32::MAX as i64) as i32,
                };
                let flood = int("floodLevel", i32::MIN as i64) as i32;
                let floating = arena.field(key, "mode").and_then(|v| arena.enum_name(v)) == Some("FLOATING");
                self.tunnels.push(Tunnel {
                    name,
                    floating,
                    floor: bound("floor", Mode::Fixed, 32),
                    roof: bound("roof", Mode::AboveFloor, 16),
                    flood: (flood != i32::MIN).then_some(flood),
                });
                vec![(Conv::Tunnel(self.tunnels.len() as u16 - 1), 1.0)]
            }
            "CombinedLayer" if depth < 4 => {
                let mut parts = Vec::new();
                let factors = arena.field(key, "factors").and_then(|f| arena.map_entries(f)).unwrap_or_default();
                for layer in arena.field(key, "layers").and_then(|l| arena.list_items(l)).unwrap_or_default() {
                    let Value::Ref(handle) = layer else { continue };
                    let factor = factors.iter().find(|(k, _)| *k == layer).and_then(|(_, f)| arena.float(*f)).unwrap_or(1.0) as f32;
                    for (conv, f) in self.parts_at(arena, layer, handle, mapper, depth + 1) {
                        parts.push((conv, f * factor));
                    }
                }
                let applies = arena.field(key, "applyTerrainAndBiomeOnExport").and_then(|v| arena.bool(v)).unwrap_or(true);
                if applies {
                    let terrain = arena.field(key, "terrain").and_then(|t| arena.enum_name(t)).and_then(|t| crate::tables::TERRAINS.iter().position(|n| *n == t));
                    if let Some(ordinal) = terrain {
                        parts.push((Conv::Terrain(mapper.terrain(ordinal as u8)), 1.0));
                    }
                    let biome = int("biome", -1);
                    if (0..255).contains(&biome) {
                        parts.push((Conv::Biome(mapper.biome(biome as u8)), 1.0));
                    }
                }
                // The combined layer itself, to paint on with in Massif: the
                // first layer of each slot it holds, at its factor (caves,
                // tunnels and custom ground covers are applied as read).
                use massif_core::combined::{CombinedDef, Part};
                let color = arena.field(key, "colour").and_then(|c| arena.int(c)).map_or([96, 132, 62], |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
                let mut def = CombinedDef::new(&name, color);
                for (conv, f) in &parts {
                    let (part, layer) = match conv {
                        Conv::Terrain(t) => (Part::Terrain, *t),
                        Conv::Biome(b) => (Part::Biome, *b),
                        Conv::Cover(n) => (Part::Cover, *n),
                        Conv::Objects(n, _, under) => (if *under { Part::Under } else { Part::Trees }, *n),
                        Conv::Pockets(n) => (Part::Pockets, *n),
                        _ => continue,
                    };
                    if def.get(part).is_none() {
                        def.set(part, Some((layer, f.min(1.0))));
                    }
                }
                if !def.parts.is_empty() {
                    mapper.registry.add_combined(def);
                }
                parts
            }
            _ => convert(arena, key, &class, &mut mapper.registry).map(|c| vec![(c, 1.0)]).unwrap_or_default(),
        }
    }
}

/// One layer of a tile: what it became, and its strength in each column
/// (0 to 255).
pub struct Painted {
    pub conv: Conv,
    pub strength: Vec<u8>,
    /// Painted before the others (0: Frost and Lava, whose snow, ice and
    /// lava take the place of plants), as the covers of a column are one.
    pub priority: u8,
}

/// What layer `key` (of class `class`) becomes in `reg`, adding what it
/// needs there; `None` for a layer Massif does not convert.
pub fn convert(arena: &Arena, key: Value, class: &str, reg: &mut Registry) -> Option<Conv> {
    let short = class.strip_prefix(LAYERS)?;
    let string = |name: &str| arena.field(key, name).and_then(|v| arena.string(v)).map(str::to_owned);
    let objects = |n: u16, curve: Curve, reg: &Registry| Conv::Objects(n, curve, reg.object_layer(n).is_some_and(|l| l.understory));
    if let Some(conv) = standard(short, reg) {
        return Some(conv);
    }
    match short {
        "Bo2Layer" => {
            // The trees of the tree pack (which WorldPainter Modernised
            // describes as such, and older versions only name, its trees
            // made by WorldPainter): Massif has the same, made the same
            // way. A layer of one's own objects may have the same name.
            let name = string("name")?;
            let made_by_worldpainter = || {
                let items = arena.field(key, "objectProvider").and_then(|p| arena.field(p, "objects")).and_then(|o| arena.list_items(o)).unwrap_or_default();
                items.iter().all(|&i| arena.class_name(i).is_some_and(|c| c.ends_with(".GenericObject")))
            };
            match TreeKind::ALL.into_iter().find(|k| (k.name() == name && made_by_worldpainter()) || string("description").is_some_and(|d| d.ends_with(TREE_PACK) && d.starts_with(k.name()))) {
                Some(kind) => {
                    let n = kind.install(reg);
                    Some(objects(n, Curve::Linear, reg))
                }
                // A layer of the user's own objects, which the world holds.
                None => object_layer(arena, key, &name, reg).map(|n| objects(n, Curve::Square, reg)),
            }
        }
        "plants.PlantLayer" => plant_cover(arena, key, reg).map(Conv::Cover),
        _ => None,
    }
}

/// What one of WorldPainter's own layers (its class, after
/// `org.pepsoft.worldpainter.layers.`) becomes in `reg`, adding what it
/// needs there, as a `.world`'s: Frost, Lava (`FloodWithLava`), the four
/// forests, Void and Annotations. `None` for any other.
pub fn standard(class: &str, reg: &mut Registry) -> Option<Conv> {
    let forest = |name: &str, kinds: &[(TreeKind, u16)], reg: &mut Registry| {
        let n = massif_core::treepack::install_mix(name, kinds, reg);
        Some(Conv::Objects(n, Curve::Square, reg.object_layer(n).is_some_and(|l| l.understory)))
    };
    match class {
        "Frost" => Some(Conv::Cover(frost_cover(reg))),
        "FloodWithLava" => Some(Conv::Cover(match reg.covers.iter().position(|c| c.lava && !c.removed) {
            Some(k) => k as u16 + 1,
            None => reg.add_cover(CoverDef::lava()),
        })),
        "DeciduousForest" => forest("Deciduous", &[(TreeKind::BigOak, 3), (TreeKind::Birch, 2)], reg),
        "PineForest" => forest("Pine", &[(TreeKind::TallSpruce, 2), (TreeKind::SlenderSpruce, 2), (TreeKind::Fir, 1)], reg),
        "Jungle" => forest("Jungle", &[(TreeKind::Rainforest, 3), (TreeKind::Bush, 1)], reg),
        "SwampLand" => forest("Swamp", &[(TreeKind::Willow, 1)], reg),
        "Void" => Some(Conv::Void),
        "Annotations" => Some(Conv::Annotations),
        _ => None,
    }
}

/// A custom object layer as an object layer of Massif: its objects read
/// from what the world holds of them (Sponge schematics, MCEdit
/// schematics, structures, Bo2 objects), placed as WorldPainter places
/// them — about as many, standing on their offset, turned and mirrored if
/// they were. `None` when none of its objects could be read.
fn object_layer(arena: &Arena, key: Value, name: &str, reg: &mut Registry) -> Option<u16> {
    let provider = arena.field(key, "objectProvider")?;
    let items = arena.field(provider, "objects").and_then(|o| arena.list_items(o))?;
    let first = reg.objects.len() as u32;
    let mut weights = Vec::new();
    let (mut rotate, mut water, mut land) = (false, false, false);
    for item in items {
        let Some((mut def, shift)) = wp_object(arena, item) else { continue };
        let attributes: Vec<(String, Value)> = arena
            .field(item, "attributes")
            .and_then(|a| arena.map_entries(a))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| Some((arena.string(k)?.to_owned(), v)))
            .collect();
        let attribute = |name: &str| attributes.iter().find(|a| a.0 == name).map(|a| a.1);
        let flag = |name: &str, default: bool| attribute(name).and_then(|v| arena.bool(v)).unwrap_or(default);
        // The offset: where the object stands from the spot it is placed
        // on, the block over the ground (WorldPainter's z is up). Objects
        // of blocks by their place stand on their place 0 without one.
        let offset = attribute("WPObject.offset").map(|o| {
            let c = |n: &str| arena.field(o, n).and_then(|v| arena.int(v)).unwrap_or(0);
            [c("x"), c("z"), c("y")]
        });
        if let Some(anchor) = anchor(offset, shift, def.size) {
            def.anchor = anchor;
        }
        rotate |= flag("WPObject.randomRotation", true);
        water |= flag("WPObject.spawnInWater", false);
        land |= flag("WPObject.spawnOnLand", true);
        let frequency = attribute("WPObject.frequency").and_then(|v| arena.int(v)).unwrap_or(100).clamp(1, u16::MAX as i64) as u16;
        def.source = format!("WorldPainter layer {name}");
        reg.objects.push(def);
        weights.push(frequency);
    }
    if weights.is_empty() {
        return None;
    }
    let indices: Vec<u32> = (first..first + weights.len() as u32).collect();
    let mut layer = massif_core::objects::ObjectLayerDef::new(name, &indices, reg);
    layer.objects = indices.iter().copied().zip(weights).collect();
    layer.rotate = rotate;
    layer.mirror = rotate;
    layer.underwater = water && !land;
    layer.soil_only = false;
    if let Some(c) = arena.field(key, "colour").and_then(|c| arena.int(c)).filter(|&c| c != 0) {
        layer.color = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    }
    // WorldPainter tries every column of its grid, and places an object
    // where a draw out of 64 × density falls under the square of the
    // strength: at full strength, 226 in 64 × density. Massif places one in
    // a share of the squares of its spacing, so as many stand.
    let int = |n: &str, default: i64| arena.field(key, n).and_then(|v| arena.int(v)).unwrap_or(default).max(1) as f32;
    let per_column = 226.0 / (64.0 * int("density", 20) * int("gridX", 1) * int("gridY", 1));
    layer.density = (per_column * (layer.spacing as f32).powi(2)).clamp(0.02, 1.0);
    Some(reg.add_object_layer(layer))
}

/// Where an object stands, in its box: the block WorldPainter puts on the
/// spot over the ground, from the object's offset (in Massif's order: x,
/// up, z) and, for an object of blocks by their place, where its own
/// coordinates start in the box (without an offset, it stands on its place
/// 0). `None` to keep the anchor Massif finds, or for a place outside the
/// box.
fn anchor(offset: Option<[i64; 3]>, shift: Option<[i64; 3]>, size: [u16; 3]) -> Option<[u16; 3]> {
    let offset = offset.or(shift.map(|_| [0; 3]))?;
    let shift = shift.unwrap_or([0; 3]);
    let anchor: [i64; 3] = std::array::from_fn(|k| -offset[k] - shift[k]);
    anchor.iter().zip(size).all(|(&a, s)| (0..s as i64).contains(&a)).then(|| anchor.map(|a| a as u16))
}

/// One of WorldPainter's objects as Massif's: its blocks in a box, `None`
/// for a kind not read here or one too large. With it, for an object of
/// blocks by their place, where its own coordinates (from which its offset
/// counts) start in the box, in Massif's order (x, up, z).
fn wp_object(arena: &Arena, object: Value) -> Option<(massif_core::objects::ObjectDef, Option<[i64; 3]>)> {
    use massif_core::objects::ObjectDef;
    let class = arena.class_name(object)?;
    let name = arena.field(object, "name").and_then(|n| arena.string(n)).unwrap_or("Object").to_owned();
    let int = |n: &str| arena.field(object, n).and_then(|v| arena.int(v)).filter(|&v| v > 0).map(|v| v as usize);
    let state = |material: Value| -> String {
        // A material of before 1.13 is known by its number.
        if arena.field(material, "identity").is_none_or(|i| i == Value::Null) {
            let n = |f: &str| arena.field(material, f).and_then(|v| arena.int(v)).unwrap_or(-1);
            let (id, data) = (n("blockType"), n("data"));
            if id >= 0 {
                return massif_objects::legacy::state(id as u16, data.max(0) as u8).unwrap_or_else(|| "minecraft:air".to_owned());
            }
        }
        crate::worldpainter::material_state(arena, material)
    };
    let fits = |w: usize, h: usize, l: usize| w * h * l <= massif_objects::MAX_VOLUME;
    match class.rsplit('.').next()? {
        // Sponge schematics: a palette and a block of it each, x first,
        // then WorldPainter's y (Massif's z), then up — as Massif lays
        // them.
        "Schem" => {
            let (w, h, l) = (int("width")?, int("height")?, int("length")?);
            let palette: Vec<String> = match arena.field(object, "palette").and_then(|p| arena.array(p))? {
                Array::Object(items) => items.iter().map(|&m| if m == Value::Null { "minecraft:air".to_owned() } else { state(m) }).collect(),
                _ => return None,
            };
            let blocks: Vec<u16> = match arena.field(object, "blocks").and_then(|b| arena.array(b))? {
                Array::Int(b) if b.len() == w * h * l => b.iter().map(|&k| if (k as usize) < palette.len() { k as u16 } else { 0 }).collect(),
                _ => return None,
            };
            fits(w, h, l).then(|| (ObjectDef::new(&name, [w as u16, h as u16, l as u16], palette, blocks), None))
        }
        // MCEdit schematics: numbered blocks, in the same order.
        "Schematic" => {
            let (w, h, l) = (int("width")?, int("height")?, int("length")?);
            let bytes = |n: &str| match arena.field(object, n).and_then(|b| arena.array(b)) {
                Some(Array::Byte(b)) => Some(b.as_slice()),
                _ => None,
            };
            let (ids, data, add) = (bytes("blocks")?, bytes("data")?, bytes("addBlocks"));
            if ids.len() != w * h * l || data.len() != ids.len() || !fits(w, h, l) {
                return None;
            }
            let mut palette = vec!["minecraft:air".to_owned()];
            let mut index: std::collections::HashMap<(u16, u8), u16> = std::collections::HashMap::new();
            let blocks = (0..ids.len())
                .map(|i| {
                    let high = add.and_then(|a| a.get(i >> 1)).map_or(0, |&a| if i & 1 == 0 { a & 15 } else { a >> 4 }) as u16;
                    let key = ((high << 8) | ids[i] as u16, data[i] & 15);
                    *index.entry(key).or_insert_with(|| {
                        palette.push(massif_objects::legacy::state(key.0, key.1).unwrap_or_else(|| "minecraft:air".to_owned()));
                        (palette.len() - 1) as u16
                    })
                })
                .collect();
            Some((ObjectDef::new(&name, [w as u16, h as u16, l as u16], palette, blocks), None))
        }
        // Structures and Bo2 objects: blocks by their place.
        "Structure" | "Bo2Object" => {
            let entries = arena.field(object, "blocks").and_then(|b| arena.map_entries(b))?;
            let mut placed = Vec::with_capacity(entries.len());
            for (point, value) in entries {
                let c = |n: &str| arena.field(point, n).and_then(|v| arena.int(v));
                let material = if class.ends_with("Bo2Object") { arena.field(value, "material")? } else { value };
                placed.push(([c("x")?, c("z")?, c("y")?], state(material)));
            }
            let lo: [i64; 3] = std::array::from_fn(|k| placed.iter().map(|p| p.0[k]).min().unwrap_or(0));
            let hi: [i64; 3] = std::array::from_fn(|k| placed.iter().map(|p| p.0[k]).max().unwrap_or(0));
            let size: [usize; 3] = std::array::from_fn(|k| (hi[k] - lo[k] + 1) as usize);
            if placed.is_empty() || !fits(size[0], size[1], size[2]) {
                return None;
            }
            let mut palette = vec!["minecraft:air".to_owned()];
            let mut blocks = vec![0u16; size[0] * size[1] * size[2]];
            for (p, s) in placed {
                let k = palette.iter().position(|q| *q == s).unwrap_or_else(|| {
                    palette.push(s);
                    palette.len() - 1
                });
                let [x, y, z] = [(p[0] - lo[0]) as usize, (p[1] - lo[1]) as usize, (p[2] - lo[2]) as usize];
                blocks[x + z * size[0] + y * size[0] * size[2]] = k as u16;
            }
            let def = ObjectDef::new(&name, size.map(|s| s as u16), palette, blocks);
            // A Bo2 object's own coordinates are its blocks' places moved
            // by its origin.
            let origin = arena.field(object, "origin").filter(|o| *o != Value::Null).map_or([0; 3], |o| {
                let c = |n: &str| arena.field(o, n).and_then(|v| arena.int(v)).unwrap_or(0);
                [c("x"), c("z"), c("y")]
            });
            Some((def, Some(std::array::from_fn(|k| lo[k] + origin[k]))))
        }
        _ => None,
    }
}

/// The layers of a dimension applied once all of it is read, from what
/// `painted` holds of them, tile by tile, with their strength in each
/// column: custom ground covers (a terrain of the cover's blocks over the
/// ground's, the ground raised by the cover's thickness), tunnels (a level
/// each: Carve, or Solid when floating) and caves carved where painted, as
/// deep as the strength (Edit ▸ Caves over a selection of those weights).
pub fn apply_after(dim: &mut Dimension, painted: &[(TilePos, After)], conv: &Converter, reg: &mut Registry, seed: i64, sea: i32) {
    use massif_core::compose::block_top;
    use massif_core::plane::Plane;
    const AREA: usize = massif_core::TILE_AREA;
    let column = |pos: TilePos, i: usize| {
        let (ox, oz) = pos.origin();
        (ox + (i & 127) as i32, oz + (i >> 7) as i32)
    };
    let of = |target: Conv| painted.iter().filter_map(move |(pos, parts)| parts.iter().find(|(c, _)| *c == target).map(|(_, s)| (*pos, s)));

    // Custom ground covers.
    let mut made: HashMap<(u16, u16), u16> = HashMap::new();
    for (k, cover) in conv.ground_covers.iter().enumerate() {
        for (pos, strength) in of(Conv::GroundCover(k as u16)) {
            let Some(tile) = dim.ground_mut().tile_mut(pos) else { continue };
            for (i, &s) in strength.iter().enumerate() {
                let (x, z) = column(pos, i);
                if s == 0 || !kept(s, Curve::Linear, x, z, 0x6763_0000 | k as u64) {
                    continue;
                }
                let base = tile.terrain.get(i);
                let t = *made.entry((k as u16, base)).or_insert_with(|| ground_terrain(reg, cover, base));
                tile.terrain.set(i, t);
                if cover.thickness > 0 {
                    tile.top.set(i, tile.top.get(i) + cover.thickness as f32);
                }
            }
            tile.compact();
        }
    }

    // Rivers, dug into the ground where painted.
    let rivers: Vec<(TilePos, &Vec<u8>)> = of(Conv::River).collect();
    if !rivers.is_empty() {
        dig_painted_rivers(dim, &rivers, reg, seed);
    }

    // Tunnels: a level each.
    for (k, tunnel) in conv.tunnels.iter().enumerate() {
        let tiles: Vec<(TilePos, &Vec<u8>)> = of(Conv::Tunnel(k as u16)).collect();
        if tiles.is_empty() {
            continue;
        }
        let kind = if tunnel.floating { massif_core::level::LevelKind::Solid } else { massif_core::level::LevelKind::Carve };
        let level = dim.add_level(tunnel.name.clone(), kind);
        let (min, max) = (dim.min_y, dim.max_y);
        for (pos, strength) in tiles {
            let Some(ground) = dim.ground_mut().tile_mut(pos) else { continue };
            let (top, terrain) = (ground.top.dense().to_vec(), ground.terrain.dense().to_vec());
            let (mut tops, mut bottoms, mut terrains, mut fluids) = (vec![0.0f32; AREA], vec![0.0f32; AREA], vec![0u16; AREA], vec![NO_FLUID; AREA]);
            for i in 0..AREA {
                let (x, z) = column(pos, i);
                if strength[i] == 0 || !kept(strength[i], Curve::Linear, x, z, 0x7475_0000 | k as u64) {
                    continue;
                }
                let (bottom, roof) = tunnel.span(block_top(top[i]) - 1, min, max);
                if roof <= bottom {
                    continue;
                }
                (bottoms[i], tops[i], terrains[i]) = (bottom, roof, terrain[i]);
                if let Some(f) = tunnel.flood.filter(|_| !tunnel.floating) {
                    let surface = (f + 1) as f32;
                    if surface > bottom {
                        fluids[i] = surface.min(roof) as i16;
                    }
                }
            }
            let mut tile = Tile::empty_slab();
            tile.top = Plane::from_vec(tops);
            tile.bottom = Some(Plane::from_vec(bottoms));
            tile.terrain = Plane::from_vec(terrains);
            tile.fluid = Plane::from_vec(fluids);
            tile.compact();
            if !tile.is_empty_slab() {
                dim.levels[level].insert(pos, tile);
            }
        }
        dim.levels[level].take_dirty();
    }

    // Caves: a cave layer for each mix of WorldPainter's cave layers
    // painted together on a column — Caverns as chambers, Chasms as
    // ravines, Caves as tunnels and noodles —, each kind as dense as the
    // strength it was painted at on average, the ground's caves plane
    // painted with it where it was painted (Edit ▸ Caves, or the Caves
    // tool, change them after).
    let kinds = [CaveKind::Caverns, CaveKind::Chasms, CaveKind::Tunnels];
    let painted_kinds: Vec<HashMap<TilePos, &Vec<u8>>> = kinds.iter().map(|&k| of(Conv::Caves(k)).collect()).collect();
    if painted_kinds.iter().any(|p| !p.is_empty()) {
        // The share each kind takes where painted, from its strength.
        let share: Vec<f32> = painted_kinds
            .iter()
            .map(|tiles| {
                let (n, sum) = tiles.values().flat_map(|s| s.iter()).filter(|&&v| v > 0).fold((0u64, 0u64), |(n, sum), &v| (n + 1, sum + v as u64));
                if n == 0 { 0.0 } else { sum as f32 / n as f32 / 255.0 }
            })
            .collect();
        let mut defs = dim.caves.to_vec();
        let (min_y, max_y) = (dim.min_y, dim.max_y);
        let mut layer_of: HashMap<u8, u16> = HashMap::new();
        let mut tiles: Vec<TilePos> = painted_kinds.iter().flat_map(|p| p.keys().copied()).collect();
        tiles.sort();
        tiles.dedup();
        for pos in tiles {
            let Some(tile) = dim.ground_mut().tile_mut(pos) else { continue };
            for i in 0..AREA {
                let mut mix = 0u8;
                for (k, p) in painted_kinds.iter().enumerate() {
                    if p.get(&pos).is_some_and(|s| s[i] > 0) {
                        mix |= 1 << k;
                    }
                }
                if mix == 0 {
                    continue;
                }
                let n = *layer_of.entry(mix).or_insert_with(|| {
                    let names: Vec<&str> = kinds.iter().enumerate().filter(|(k, _)| mix & (1 << k) != 0).map(|(_, k)| k.name()).collect();
                    let mut def = massif_core::caves::CaveDef::vanilla(&names.join(" + "), (seed as u32).wrapping_add(mix as u32), min_y, max_y, sea);
                    let on = |k: usize| mix & (1 << k) != 0;
                    (def.cheese.on, def.cheese.density) = (on(0), share[0]);
                    (def.ravines.on, def.ravines.density) = (on(1), share[1]);
                    (def.spaghetti.on, def.spaghetti.density) = (on(2), share[2]);
                    (def.noodles.on, def.noodles.density) = (on(2), share[2] * 0.5);
                    def.openings = 0.0;
                    def.aquifers.on = false;
                    defs.push(def);
                    defs.len() as u16
                });
                tile.caves.set(i, n);
            }
            tile.compact();
        }
        dim.caves = massif_core::caves::CaveLayers::new(defs);
    }
    dim.ground_mut().take_dirty();
    dim.voxels.take_dirty();
    // What was changed here is kept as it will be saved.
    for level in &mut dim.levels {
        level.freeze();
    }
}

/// WorldPainter's River layer, dug as its exporter digs it (RiverExporter,
/// whose settings are fixed): the water's surface two blocks under the
/// ground, and the bed four to seven blocks under it, its depth wandering
/// with a fine noise and shallower towards the edges of what is painted,
/// as many blocks deep as the column lies from the nearest column not
/// painted. In water already there (a lake painted over), only the bed is
/// dug, under that water. Where a river comes within ten blocks of water
/// that was there before (a lake, the sea) lying higher than its own, its
/// water rises to that water's level, so that it joins it without a wall.
/// Its bed is Massif's rivers': gravel, sand where the water is under two
/// blocks deep, clay in patches (WorldPainter's own riverbed is a mix of
/// grass, gravel, clay and stone); the river biome where painted.
pub fn dig_painted_rivers(dim: &mut Dimension, tiles: &[(TilePos, &Vec<u8>)], reg: &mut Registry, seed: i64) {
    use massif_core::compose::block_top;
    use massif_core::coords::local_index;
    const SHORE: i32 = 2;
    const MAX_DEPTH: i32 = SHORE + 7;
    const VARIATION: i32 = 7 - 4;
    const REACH: i32 = 10;
    const N: i32 = 128 + 2 * REACH;
    let painted: HashMap<TilePos, &Vec<u8>> = tiles.iter().map(|(p, s)| (*p, *s)).collect();
    let river = |x: i32, z: i32| painted.get(&TilePos::containing(x, z)).is_some_and(|s| s[local_index(x, z)] > 0);
    let (gravel, sand, clay) = (reg.terrain_by_name("Gravel"), reg.terrain_by_name("Sand"), reg.terrain_by_name("Clay"));
    let biome = reg.biome_by_id("minecraft:river");
    let floor = massif_core::noise::Noise::new((seed as u64) ^ 0x7269_7665);
    let patches = massif_core::noise::Noise::new((seed as u64) ^ 0x636c_6179);
    // What each painted tile becomes, worked out from the map as it was.
    type Dug = (TilePos, Vec<(usize, f32, i16, Option<u16>)>);
    let ground = dim.ground();
    let mut views: HashMap<TilePos, Option<massif_core::level::TileRef>> = HashMap::new();
    let mut work: Vec<Dug> = Vec::new();
    for &(pos, _) in tiles {
        let (ox, oz) = pos.origin();
        let (x0, z0) = (ox - REACH, oz - REACH);
        let mut column = |x: i32, z: i32| -> Option<(f32, i16)> {
            let p = TilePos::containing(x, z);
            let t = views.entry(p).or_insert_with(|| ground.view(p)).as_ref()?;
            let i = local_index(x, z);
            t.present(i).then(|| (t.top.get(i), t.fluid.get(i)))
        };
        // Blocks to the nearest column not painted, up to the deepest a
        // river digs (two passes of a chamfer: 1 along, 1.4 across).
        let (n, big) = (N as usize, MAX_DEPTH as f32 + 1.0);
        let mut dist = vec![0.0f32; n * n];
        for j in 0..n {
            for i in 0..n {
                if river(x0 + i as i32, z0 + j as i32) {
                    dist[j * n + i] = big;
                }
            }
        }
        for j in 0..n {
            for i in 0..n {
                let c = j * n + i;
                if dist[c] == 0.0 {
                    continue;
                }
                let mut d = dist[c];
                if i > 0 { d = d.min(dist[c - 1] + 1.0); } else { d = d.min(1.0); }
                if j > 0 { d = d.min(dist[c - n] + 1.0); } else { d = d.min(1.0); }
                if i > 0 && j > 0 { d = d.min(dist[c - n - 1] + 1.4); }
                if i + 1 < n && j > 0 { d = d.min(dist[c - n + 1] + 1.4); }
                dist[c] = d;
            }
        }
        for j in (0..n).rev() {
            for i in (0..n).rev() {
                let c = j * n + i;
                if dist[c] == 0.0 {
                    continue;
                }
                let mut d = dist[c];
                if i + 1 < n { d = d.min(dist[c + 1] + 1.0); } else { d = d.min(1.0); }
                if j + 1 < n { d = d.min(dist[c + n] + 1.0); } else { d = d.min(1.0); }
                if i + 1 < n && j + 1 < n { d = d.min(dist[c + n + 1] + 1.4); }
                if i > 0 && j + 1 < n { d = d.min(dist[c + n - 1] + 1.4); }
                dist[c] = d;
            }
        }
        // The water standing there before, not painted: what a river within
        // ten blocks joins.
        let mut body = vec![NO_FLUID; n * n];
        for j in 0..n {
            for i in 0..n {
                let (x, z) = (x0 + i as i32, z0 + j as i32);
                if !river(x, z)
                    && let Some((top, fluid)) = column(x, z)
                    && fluid != NO_FLUID
                    && fluid as i32 > block_top(top)
                {
                    body[j * n + i] = fluid;
                }
            }
        }
        let mut cells = Vec::new();
        for lz in 0..128 {
            for lx in 0..128 {
                let (x, z) = (ox + lx, oz + lz);
                if !river(x, z) {
                    continue;
                }
                let Some((top, fluid)) = column(x, z) else { continue };
                let c = (lz + REACH) as usize * n + (lx + REACH) as usize;
                let wander = ((floor.get(x as f64 / 4.099, z as f64 / 4.099) * 0.5 + 0.5).clamp(0.0, 1.0) * VARIATION as f64).round() as i32;
                let depth = (dist[c] as i32).max(1).min(MAX_DEPTH - wander);
                let height = block_top(top) - 1;
                let flooded = fluid != NO_FLUID && fluid as i32 > block_top(top);
                let (mut new_top, mut new_fluid) = if flooded {
                    // Already under water: the bed dug under it.
                    (((fluid as i32 - depth) as f32).min(top), fluid)
                } else {
                    let bed = (height - depth + 1) as f32;
                    let water = height - SHORE + 1;
                    (bed.min(top), if water > block_top(bed) { water as i16 } else { NO_FLUID })
                };
                // Water there before, higher, within ten blocks: joined.
                let mut joined = NO_FLUID;
                for dz in -REACH..=REACH {
                    for dx in -REACH..=REACH {
                        if dx * dx + dz * dz > REACH * REACH {
                            continue;
                        }
                        let b = body[(lz + REACH + dz) as usize * n + (lx + REACH + dx) as usize];
                        if b != NO_FLUID && (joined == NO_FLUID || b > joined) {
                            joined = b;
                        }
                    }
                }
                if joined != NO_FLUID && joined as i32 > block_top(new_top) && (new_fluid == NO_FLUID || joined > new_fluid) {
                    new_fluid = joined;
                }
                new_top = new_top.min(top);
                let terrain = (new_fluid != NO_FLUID && new_fluid as i32 > block_top(new_top)).then(|| {
                    let deep = new_fluid as i32 - block_top(new_top);
                    if deep < 2 {
                        sand.or(gravel)
                    } else if patches.fbm(x as f64 / 9.0, z as f64 / 9.0, 2, 2.0, 0.5) > 0.35 {
                        clay.or(gravel)
                    } else {
                        gravel
                    }
                });
                cells.push((local_index(x, z), new_top, new_fluid, terrain.flatten().map(|t| t.0)));
            }
        }
        work.push((pos, cells));
    }
    drop(views);
    // The ground of the columns dug, as it was: their banks may rise back
    // to it to hold the water.
    let mut dug: HashMap<(i32, i32), f32> = HashMap::new();
    for (pos, cells) in work {
        let Some(tile) = dim.ground_mut().tile_mut(pos) else { continue };
        let (ox, oz) = pos.origin();
        for (i, top, fluid, terrain) in cells {
            dug.insert((ox + (i % 128) as i32, oz + (i / 128) as i32), tile.top.get(i));
            tile.top.set(i, top);
            tile.fluid.set(i, fluid);
            if let Some(t) = terrain {
                tile.terrain.set(i, t);
            }
            if let Some(b) = biome {
                tile.biome.set(i, b.0);
            }
        }
        tile.compact();
    }
    // Down a slope, the water two blocks under each column's ground stands
    // over the lower bank beside it, and the game would run it out over the
    // land: there the bank rises back to the ground it had, or the river's
    // water goes down to what holds it.
    let (lo, hi) = tiles.iter().fold((TilePos::new(i32::MAX, i32::MAX), TilePos::new(i32::MIN, i32::MIN)), |(lo, hi), (p, _)| (TilePos::new(lo.x.min(p.x), lo.z.min(p.z)), TilePos::new(hi.x.max(p.x), hi.z.max(p.z))));
    if lo.x <= hi.x {
        let rect = massif_core::coords::BlockRect::new(lo.x * 128 - 1, lo.z * 128 - 1, (hi.x + 1) * 128 + 1, (hi.z + 1) * 128 + 1);
        massif_core::water::seal_level(dim.ground_mut(), rect, &|x, z| dug.get(&(x, z)).copied().unwrap_or(f32::MIN), &|x, z| river(x, z), i16::MIN);
    }
}

/// The terrain of a custom ground cover over terrain `base`: the cover's
/// blocks, as thick as it is, over the strata of `base`; made once.
pub fn ground_terrain(reg: &mut Registry, cover: &GroundCover, base: u16) -> u16 {
    let under = reg.terrain(TerrainId(base)).clone();
    let name = format!("{} on {}", cover.name, under.name);
    if let Some(t) = reg.terrain_by_name(&name) {
        return t.0;
    }
    let total: u64 = cover.blocks.iter().map(|b| b.1 as u64).sum::<u64>().max(1);
    let blocks = cover.blocks.iter().map(|(state, n)| (reg.intern_block(state, crate::terrains::block_color(state)), ((*n as u64 * 1000) / total).clamp(1, 1000) as u16)).collect();
    let depth = cover.thickness.unsigned_abs().min(64) as u16;
    let mut strata = vec![Stratum { blocks, depth: (depth, depth), mix: Default::default() }];
    strata.extend(under.strata.iter().cloned());
    reg.terrains.push(TerrainDef {
        name,
        group: "Imported".into(),
        color: crate::terrains::block_color(&cover.blocks[0].0),
        strata,
        submerged_top: None,
        biome_hint: under.biome_hint,
    });
    (reg.terrains.len() - 1) as u16
}

/// The frost cover of `reg`, added if it has none.
pub fn frost_cover(reg: &mut Registry) -> u16 {
    match reg.covers.iter().position(|c| c.frost && !c.removed) {
        Some(k) => k as u16 + 1,
        None => reg.add_cover(CoverDef::frost()),
    }
}

/// The strength of a layer in each column of a tile, from its data: a
/// nibble or a byte a column, or a `java.util.BitSet`.
pub fn strengths(arena: &Arena, value: Value) -> Option<Vec<u8>> {
    const AREA: usize = massif_core::TILE_AREA;
    if let Some(array) = arena.array(value) {
        return match array {
            Array::Byte(raw) if raw.len() == AREA / 2 => Some((0..AREA).map(|i| ((raw[i / 2] >> ((i & 1) * 4)) & 15) * 17).collect()),
            Array::Byte(raw) if raw.len() == AREA => Some(raw.clone()),
            _ => None,
        };
    }
    let words = match arena.field(value, "bits").and_then(|b| arena.array(b)) {
        Some(Array::Long(words)) => words,
        _ => return None,
    };
    Some((0..AREA).map(|i| if words.get(i >> 6).is_some_and(|w| (w >> (i & 63)) & 1 != 0) { 255 } else { 0 }).collect())
}

/// Whether a column painted at `strength` keeps the layer.
pub fn kept(strength: u8, curve: Curve, x: i32, z: i32, salt: u64) -> bool {
    let share = match curve {
        Curve::Linear => strength as u32,
        Curve::Square => (strength as u32 * strength as u32) / 255,
    };
    share >= 255 || (share > 0 && massif_core::compose::hash2(x, z, 0x7770_6c61_7965_7273 ^ salt) % 255 < share)
}

/// A plant layer as a ground cover: its plants with their occurrences.
fn plant_cover(arena: &Arena, key: Value, reg: &mut Registry) -> Option<u16> {
    let name = arena.field(key, "name").and_then(|v| arena.string(v)).unwrap_or("Plants").to_owned();
    let settings = match arena.field(key, "settings").and_then(|s| arena.array(s)) {
        Some(Array::Object(items)) => items.clone(),
        _ => return None,
    };
    let mut plants: Vec<(String, u16)> = Vec::new();
    let (mut wet, mut dry) = (0u32, 0u32);
    let mut height = [u8::MAX, 0u8];
    for (k, item) in settings.iter().enumerate() {
        let Some(occurrence) = arena.field(*item, "occurrence").and_then(|o| arena.int(o)) else { continue };
        let Some(&(state, kind)) = PLANTS.get(k) else { continue };
        if occurrence <= 0 || state.is_empty() {
            continue;
        }
        let weight = occurrence.min(u16::MAX as i64) as u16;
        match kind {
            Kind::Water => wet += weight as u32,
            _ => dry += weight as u32,
        }
        if kind == Kind::Tall {
            let int = |n: &str| arena.field(*item, n).and_then(|v| arena.int(v)).unwrap_or(1).clamp(1, 16) as u8;
            height = [height[0].min(int("growthFrom")), height[1].max(int("growthTo"))];
        } else {
            height = [1, height[1].max(1)];
        }
        plants.push((format!("minecraft:{state}"), weight));
    }
    if plants.is_empty() {
        return None;
    }
    // A cover grows in the air or in the water: the plants of the other
    // side are left out.
    let underwater = wet > dry;
    plants.retain(|(state, _)| PLANTS.iter().any(|(s, kind)| state.ends_with(s) && !s.is_empty() && (*kind == Kind::Water) == underwater));
    let color = arena.field(key, "colour").and_then(|c| arena.int(c)).map_or([96, 160, 72], |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    let soil_only = arena.field(key, "onlyOnValidBlocks").and_then(|v| arena.bool(v)).unwrap_or(true) && !underwater;
    let mut def = CoverDef::new(&name, color, &[], 1.0);
    def.plants = plants;
    def.underwater = underwater;
    def.soil_only = soil_only;
    def.height = if height[0] == u8::MAX { [1, 1] } else { [height[0], height[1].max(height[0])] };
    // A cover of a new world with that name gives its place to the
    // layer's own plants: the map keeps the name it had.
    match reg.cover_by_name(&name) {
        Some(n) => {
            reg.covers[n as usize - 1] = def;
            Some(n)
        }
        None => Some(reg.add_cover(def)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// One block or a two-block plant.
    Plain,
    /// Grows some blocks high (cactus, cane, bamboo).
    Tall,
    /// Grows under water.
    Water,
}

use Kind::{Plain, Tall, Water};

/// WorldPainter's plants (`Plants.ALL_PLANTS`, in its order, the index of a
/// plant layer's settings) as blocks; an empty name for a plant that hangs
/// or climbs, which a cover does not grow.
const PLANTS: &[(&str, Kind)] = &[
    ("short_grass", Plain),
    ("tall_grass", Plain),
    ("fern", Plain),
    ("large_fern", Plain),
    ("dead_bush", Plain),
    ("dandelion", Plain),
    ("poppy", Plain),
    ("blue_orchid", Plain),
    ("allium", Plain),
    ("azure_bluet", Plain),
    ("red_tulip", Plain),
    ("orange_tulip", Plain),
    ("white_tulip", Plain),
    ("pink_tulip", Plain),
    ("oxeye_daisy", Plain),
    ("sunflower", Plain),
    ("lilac", Plain),
    ("rose_bush", Plain),
    ("peony", Plain),
    ("oak_sapling", Plain),
    ("dark_oak_sapling", Plain),
    ("spruce_sapling", Plain),
    ("birch_sapling", Plain),
    ("jungle_sapling", Plain),
    ("acacia_sapling", Plain),
    ("red_mushroom", Plain),
    ("brown_mushroom", Plain),
    ("wheat[age=7]", Plain),
    ("carrots[age=7]", Plain),
    ("potatoes[age=7]", Plain),
    ("pumpkin_stem[age=7]", Plain),
    ("melon_stem[age=7]", Plain),
    ("cactus", Tall),
    ("sugar_cane", Tall),
    ("lily_pad", Plain),
    ("beetroots[age=3]", Plain),
    ("nether_wart[age=3]", Plain),
    ("", Plain), // chorus plant
    ("tube_coral", Water),
    ("brain_coral", Water),
    ("bubble_coral", Water),
    ("fire_coral", Water),
    ("horn_coral", Water),
    ("tube_coral_fan", Water),
    ("brain_coral_fan", Water),
    ("bubble_coral_fan", Water),
    ("fire_coral_fan", Water),
    ("horn_coral_fan", Water),
    ("kelp", Water),
    ("seagrass", Water),
    ("tall_seagrass", Water),
    ("sea_pickle", Water),
    ("cornflower", Plain),
    ("lily_of_the_valley", Plain),
    ("wither_rose", Plain),
    ("sweet_berry_bush[age=3]", Plain),
    ("bamboo", Tall),
    ("azalea", Plain),
    ("flowering_azalea", Plain),
    ("crimson_fungus", Plain),
    ("warped_fungus", Plain),
    ("crimson_roots", Plain),
    ("warped_roots", Plain),
    ("nether_sprouts", Plain),
    ("", Plain), // twisting vines
    ("", Plain), // glow lichen
    ("moss_carpet", Plain),
    ("", Plain), // big dripleaf
    ("pumpkin", Plain),
    ("melon", Plain),
    ("carved_pumpkin", Plain),
    ("jack_o_lantern", Plain),
    ("", Plain), // vine
    ("", Plain), // spore blossom
    ("", Plain), // weeping vines
    ("", Plain), // hanging roots
    ("", Plain), // glow berries
    ("", Plain), // small dripleaf
    ("", Plain), // mangrove propagule
    ("cherry_sapling", Plain),
    ("pink_petals", Plain),
    ("", Plain), // pitcher pod
    ("pitcher_plant", Plain),
    ("", Plain), // torchflower seed
    ("torchflower", Plain),
    ("closed_eyeblossom", Plain),
    ("", Plain), // pale hanging moss
    ("pale_moss_carpet", Plain),
    ("wildflowers", Plain),
    ("leaf_litter", Plain),
    ("bush", Plain),
    ("firefly_bush", Plain),
    ("cactus_flower", Plain),
    ("short_dry_grass", Plain),
    ("tall_dry_grass", Plain),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plants_are_wordpainters_in_its_order() {
        // Plants.ALL_PLANTS has 95 plants.
        assert_eq!(PLANTS.len(), 95);
        assert_eq!(PLANTS[0].0, "short_grass");
        assert_eq!(PLANTS[33].0, "sugar_cane");
        assert_eq!(PLANTS[48].0, "kelp");
        assert_eq!(PLANTS[52].0, "cornflower");
        assert_eq!(PLANTS[80].0, "pink_petals");
        assert_eq!(PLANTS[94].0, "tall_dry_grass");
    }

    /// A Sponge schematic and an MCEdit schematic as WorldPainter keeps them
    /// in a world: their blocks in Massif's order (x, then WorldPainter's y
    /// that is Massif's z, then up), their states read.
    #[test]
    fn objects_are_read_from_the_world() {
        use crate::javaser::tests::StreamWriter;
        use crate::javaser::*;
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        let mut w = StreamWriter::new();
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.bo2.Schem",
            SC_SERIALIZABLE,
            &[
                (b'I', "height", None),
                (b'I', "length", None),
                (b'I', "width", None),
                (b'[', "blocks", Some("[I")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'[', "palette", Some("[Lorg/pepsoft/minecraft/Material;")),
            ],
            null,
        );
        w.handle();
        w.i32(4).i32(3).i32(3);
        // A trunk 3 high in the middle, a layer of leaves on it.
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(36);
        for i in 0..36 {
            let (x, y, z) = (i % 3, i / 3 % 3, i / 9);
            w.i32(if z == 3 { 2 } else if x == 1 && y == 1 { 1 } else { 0 });
        }
        w.string("Little tree");
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.minecraft.Material;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(3);
        let mut descs = None;
        for name in ["minecraft:air", "minecraft:oak_log", "minecraft:oak_leaves"] {
            match descs {
                None => {
                    let material = w.next;
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material",
                        SC_SERIALIZABLE,
                        &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
                        null,
                    );
                    w.handle();
                    w.i32(-1).i32(-1);
                    let identity = w.next;
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material$Identity",
                        SC_SERIALIZABLE,
                        &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
                        null,
                    );
                    descs = Some((material, identity));
                }
                Some((material, identity)) => {
                    w.u8(TC_OBJECT).reference(material);
                    w.handle();
                    w.i32(-1).i32(-1);
                    w.u8(TC_OBJECT).reference(identity);
                }
            }
            w.handle();
            w.string(name);
            w.u8(TC_NULL);
        }
        // An MCEdit schematic: a log (17) under leaves (18), numbered.
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.bo2.Schematic",
            SC_SERIALIZABLE,
            &[
                (b'S', "height", None),
                (b'S', "length", None),
                (b'S', "width", None),
                (b'[', "blocks", Some("[B")),
                (b'[', "data", Some("[B")),
                (b'L', "name", Some("Ljava/lang/String;")),
            ],
            null,
        );
        w.handle();
        w.u16(2).u16(1).u16(1);
        let bytes = w.next;
        w.u8(TC_ARRAY).class("[B", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(2).u8(17).u8(18);
        w.u8(TC_ARRAY).reference(bytes);
        w.handle();
        w.i32(2).u8(0).u8(0);
        w.string("Old bush");

        let mut parser = Parser::new(&w.out[..]).unwrap();
        let (schem, schematic) = (parser.next_object().unwrap().unwrap(), parser.next_object().unwrap().unwrap());
        let arena = &parser.arena;
        let (tree, shift) = wp_object(arena, schem).unwrap();
        assert_eq!(shift, None);
        assert_eq!((tree.name.as_str(), tree.size), ("Little tree", [3, 4, 3]));
        let at = |x: usize, y: usize, z: usize| tree.palette[tree.blocks[x + z * 3 + y * 9] as usize].as_str();
        assert_eq!(at(1, 0, 1), "minecraft:oak_log");
        assert_eq!(at(1, 2, 1), "minecraft:oak_log");
        assert_eq!(at(0, 0, 0), "minecraft:air");
        assert_eq!(at(2, 3, 0), "minecraft:oak_leaves");
        // Its offset, (-1, -1, 0) as WorldPainter writes it (z up): the
        // foot of its trunk on the spot.
        assert_eq!(anchor(Some([-1, 0, -1]), None, tree.size), Some([1, 0, 1]));
        assert_eq!(anchor(None, None, tree.size), None);
        assert_eq!(anchor(Some([-5, 0, 0]), None, tree.size), None, "outside its box");
        // Blocks by their place, from (-2, 0, -2): on their place 0.
        assert_eq!(anchor(None, Some([-2, 0, -2]), [5, 6, 5]), Some([2, 0, 2]));

        let (bush, _) = wp_object(arena, schematic).unwrap();
        assert_eq!(bush.size, [1, 2, 1]);
        assert!(bush.palette[bush.blocks[0] as usize].starts_with("minecraft:oak_log"), "{:?}", bush.palette);
        assert!(bush.palette[bush.blocks[1] as usize].starts_with("minecraft:oak_leaves"), "{:?}", bush.palette);
    }

    /// Where WorldPainter's tunnel modes put a floor and a roof, over ground
    /// whose top block is at 70, in a world from -64 to 320.
    #[test]
    fn tunnels_find_their_floor_and_roof() {
        let bound = |mode, level| Bound { mode, level, min: i32::MIN, max: i32::MAX };
        let tunnel = |floor, roof, floating| Tunnel { name: "T".into(), floating, floor, roof, flood: None };
        // The default: a floor at 32, a roof 16 over it.
        let t = tunnel(bound(Mode::Fixed, 32), bound(Mode::AboveFloor, 16), false);
        assert_eq!(t.span(70, -64, 320), (33.0, 49.0));
        // 10 under the ground, a roof 4 under it: carved from 61 to 66.
        let t = tunnel(bound(Mode::Depth, 10), bound(Mode::Depth, 4), false);
        assert_eq!(t.span(70, -64, 320), (61.0, 67.0));
        // A roof over the ground: open to the sky, up to the ground.
        let t = tunnel(bound(Mode::Depth, 3), bound(Mode::AboveFloor, 20), false);
        assert_eq!(t.span(70, -64, 320), (68.0, 71.0));
        // As deep under 40 as the ground is over it: 10.
        let t = tunnel(bound(Mode::Inverted, 40), bound(Mode::AboveFloor, 5), false);
        assert_eq!(t.span(70, -64, 320), (11.0, 16.0));
        // Limits keep the floor in its range.
        let t = tunnel(Bound { mode: Mode::Depth, level: 100, min: 0, max: 50 }, bound(Mode::AboveFloor, 5), false);
        assert_eq!(t.span(70, -64, 320).0, 1.0);
        // Floating: land from 12 under the floor up to it.
        let t = tunnel(bound(Mode::Fixed, 150), bound(Mode::AboveFloor, 12), true);
        assert_eq!(t.span(70, -64, 320), (139.0, 151.0));
    }

    /// A tunnel layer, a custom ground cover layer and a combined layer of
    /// Frost at half strength, that ground cover, sand and a forest, as
    /// WorldPainter keeps them in a world.
    #[test]
    fn tunnels_ground_covers_and_combined_layers_are_read() {
        use crate::javaser::tests::StreamWriter;
        use crate::javaser::*;
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        let enum_class = |w: &mut StreamWriter, class: &str| {
            w.u8(TC_ENUM).class(class, SC_SERIALIZABLE | 0x10, &[], |w| {
                w.class("java.lang.Enum", SC_SERIALIZABLE | 0x10, &[], |w| {
                    w.u8(TC_NULL);
                });
            });
        };
        let mut w = StreamWriter::new();
        // TunnelLayer { I floodLevel, I floorLevel, I roofLevel, L floorMode, L mode, L name, L roofMode }
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.tunnel.TunnelLayer",
            SC_SERIALIZABLE,
            &[
                (b'I', "floodLevel", None),
                (b'I', "floorLevel", None),
                (b'I', "roofLevel", None),
                (b'L', "floorMode", Some("Lorg/pepsoft/worldpainter/layers/tunnel/TunnelLayer$Mode;")),
                (b'L', "mode", Some("Lorg/pepsoft/worldpainter/layers/tunnel/TunnelLayer$LayerMode;")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'L', "roofMode", Some("Lorg/pepsoft/worldpainter/layers/tunnel/TunnelLayer$Mode;")),
            ],
            null,
        );
        let tunnel = w.handle();
        w.i32(55).i32(10).i32(5);
        let mode = w.next;
        enum_class(&mut w, "org.pepsoft.worldpainter.layers.tunnel.TunnelLayer$Mode");
        w.handle();
        w.string("CONSTANT_DEPTH");
        enum_class(&mut w, "org.pepsoft.worldpainter.layers.tunnel.TunnelLayer$LayerMode");
        w.handle();
        w.string("CAVE");
        w.string("Mine");
        w.u8(TC_ENUM).reference(mode);
        w.handle();
        w.string("FIXED_HEIGHT_ABOVE_FLOOR");
        // GroundCoverLayer { I thickness, L material, L name }
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer",
            SC_SERIALIZABLE,
            &[(b'I', "thickness", None), (b'L', "material", Some("Lorg/pepsoft/minecraft/Material;")), (b'L', "name", Some("Ljava/lang/String;"))],
            null,
        );
        let cover = w.handle();
        w.i32(2);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material",
            SC_SERIALIZABLE,
            &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
            null,
        );
        w.handle();
        w.i32(-1).i32(-1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material$Identity",
            SC_SERIALIZABLE,
            &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
            null,
        );
        w.handle();
        w.string("minecraft:snow_block");
        w.u8(TC_NULL);
        w.string("Snow Block");
        // CombinedLayer { I biome, [ layers, L factors, L name, L terrain }:
        // Frost ×0.5 and the ground cover, sand, a forest (4).
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.CombinedLayer",
            SC_SERIALIZABLE,
            &[
                (b'I', "biome", None),
                (b'[', "layers", Some("[Lorg/pepsoft/worldpainter/layers/Layer;")),
                (b'L', "factors", Some("Ljava/util/Map;")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'L', "terrain", Some("Lorg/pepsoft/worldpainter/Terrain;")),
            ],
            null,
        );
        w.handle();
        w.i32(4);
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.worldpainter.layers.Layer;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(2);
        w.u8(TC_OBJECT).class("org.pepsoft.worldpainter.layers.Frost", SC_SERIALIZABLE, &[(b'L', "name", Some("Ljava/lang/String;"))], null);
        let frost = w.handle();
        w.string("Frost");
        w.u8(TC_REFERENCE).i32(0x7E_0000 + cover);
        w.u8(TC_OBJECT).class("java.util.Collections$SingletonMap", SC_SERIALIZABLE, &[(b'L', "k", Some("Ljava/lang/Object;")), (b'L', "v", Some("Ljava/lang/Object;"))], null);
        w.handle();
        w.u8(TC_REFERENCE).i32(0x7E_0000 + frost);
        w.u8(TC_OBJECT).class("java.lang.Float", SC_SERIALIZABLE, &[(b'F', "value", None)], |w| {
            w.class("java.lang.Number", SC_SERIALIZABLE, &[], |w| {
                w.u8(TC_NULL);
            });
        });
        w.handle();
        w.i32(0.5f32.to_bits() as i32);
        w.string("Snowy sand");
        enum_class(&mut w, "org.pepsoft.worldpainter.Terrain");
        w.handle();
        w.string("SAND");

        let mut parser = Parser::new(&w.out[..]).unwrap();
        let values: Vec<Value> = (0..3).map(|_| parser.next_object().unwrap().unwrap()).collect();
        let arena = &parser.arena;
        let mut mapper = Mapper::new(Registry::vanilla());
        let mut conv = Converter::default();
        let handle = |v: Value| match v {
            Value::Ref(h) => h,
            _ => panic!(),
        };
        assert_eq!(handle(values[0]), tunnel as usize);
        assert_eq!(handle(values[1]), cover as usize);
        assert_eq!(conv.parts(arena, values[0], handle(values[0]), &mut mapper), [(Conv::Tunnel(0), 1.0)]);
        let t = &conv.tunnels[0];
        assert_eq!((t.name.as_str(), t.floating, t.flood), ("Mine", false, Some(55)));
        assert_eq!((t.floor.mode, t.floor.level, t.roof.mode, t.roof.level), (Mode::Depth, 10, Mode::AboveFloor, 5));
        assert_eq!(t.span(70, -64, 320), (61.0, 66.0));
        assert_eq!(conv.parts(arena, values[1], handle(values[1]), &mut mapper), [(Conv::GroundCover(0), 1.0)]);
        assert_eq!(conv.ground_covers[0], GroundCover { name: "Snow Block".into(), blocks: vec![("minecraft:snow_block".into(), 1)], thickness: 2 });
        let parts = conv.parts(arena, values[2], handle(values[2]), &mut mapper);
        let reg = &mapper.registry;
        let frost_cover = reg.covers.iter().position(|c| c.frost).unwrap() as u16 + 1;
        let sand = reg.terrain_by_name("Sand").unwrap().0;
        let forest = reg.biome_by_id("minecraft:forest").unwrap().0;
        assert_eq!(parts, [(Conv::Cover(frost_cover), 0.5), (Conv::GroundCover(0), 1.0), (Conv::Terrain(sand), 1.0), (Conv::Biome(forest), 1.0)]);
        assert_eq!(conv.ground_covers.len(), 1, "read once");
        // And a combined layer to paint on with: its frost at its factor,
        // its terrain and biome (the custom ground cover is no slot).
        use massif_core::combined::Part;
        let combined = &mapper.registry.combined;
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].name, "Snowy sand");
        assert_eq!(combined[0].parts.iter().map(|c| (c.part, c.layer, c.share)).collect::<Vec<_>>(), [(Part::Terrain, sand, 1.0), (Part::Biome, forest, 1.0), (Part::Cover, frost_cover, 0.5)]);
    }

    /// An underground pockets layer becomes a pocket layer of the same
    /// block, its frequency a share of the rock in thousandths, its scale
    /// the size of its blobs, between the same heights, in its colour.
    #[test]
    fn underground_pockets_are_read() {
        use crate::javaser::tests::StreamWriter;
        use crate::javaser::*;
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        let mut w = StreamWriter::new();
        // UndergroundPocketsLayer { I colour, I frequency, I maxLevel, I minLevel, I scale, L material, L name }
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.pockets.UndergroundPocketsLayer",
            SC_SERIALIZABLE,
            &[
                (b'I', "colour", None),
                (b'I', "frequency", None),
                (b'I', "maxLevel", None),
                (b'I', "minLevel", None),
                (b'I', "scale", None),
                (b'L', "material", Some("Lorg/pepsoft/minecraft/Material;")),
                (b'L', "name", Some("Ljava/lang/String;")),
            ],
            null,
        );
        let pockets = w.handle();
        w.i32(0x80_6040).i32(200).i32(60).i32(-30).i32(150);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material",
            SC_SERIALIZABLE,
            &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
            null,
        );
        w.handle();
        w.i32(-1).i32(-1);
        w.u8(TC_OBJECT).class(
            "org.pepsoft.minecraft.Material$Identity",
            SC_SERIALIZABLE,
            &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
            null,
        );
        w.handle();
        w.string("minecraft:gravel");
        w.u8(TC_NULL);
        w.string("Gravel pockets");

        let mut parser = Parser::new(&w.out[..]).unwrap();
        let value = parser.next_object().unwrap().unwrap();
        let arena = &parser.arena;
        let Value::Ref(h) = value else { panic!() };
        assert_eq!(h, pockets as usize);
        let mut mapper = Mapper::new(Registry::vanilla());
        let mut conv = Converter::default();
        assert_eq!(conv.parts(arena, value, h, &mut mapper), [(Conv::Pockets(1), 1.0)]);
        let reg = &mapper.registry;
        let def = reg.pocket(1).unwrap();
        assert_eq!((def.name.as_str(), def.color), ("Gravel pockets", [0x80, 0x60, 0x40]));
        assert_eq!(def.blocks.iter().map(|&(b, w)| (reg.block(b).state.as_str(), w)).collect::<Vec<_>>(), [("minecraft:gravel", 1)]);
        assert_eq!((def.share, def.low, def.high), (0.2, -30, 60));
        assert!((def.size - 4.099 * 1.5).abs() < 1e-4, "{}", def.size);
    }

    /// WorldPainter's River layer dug as WorldPainter's exporter digs it:
    /// across a band painted 20 blocks wide over flat ground at 80, the
    /// water two blocks under the ground in the middle over a bed six to
    /// nine blocks down, dry and shallow at its edges, gravel or sand under
    /// the water and the river biome where painted; within ten blocks of a
    /// lake standing higher, the river's water rises to the lake's.
    #[test]
    fn worldpainters_river_layer_is_dug() {
        use massif_core::coords::local_index;
        use massif_core::world::{DimensionKind, World, WorldMeta};
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let grass = world.registry.terrain_by_name("Grass").unwrap().0;
        let mut tile = Tile::ground(81.0, grass, NO_FLUID);
        // A lake at 90 over ground at 79 in the corner x, z < 8.
        for z in 0..8 {
            for x in 0..8 {
                tile.top.set(local_index(x, z), 79.0);
                tile.fluid.set(local_index(x, z), 90);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        // A band 20 wide along z = 50..70, and a short one by the lake.
        let painted_at = |x: i32, z: i32| (50..70).contains(&z) || ((10..16).contains(&x) && z < 20);
        let band: Vec<u8> = (0..massif_core::TILE_AREA).map(|i| if painted_at((i & 127) as i32, (i >> 7) as i32) { 255 } else { 0 }).collect();
        let painted = vec![(TilePos::new(0, 0), vec![(Conv::River, band)])];
        apply_after(&mut dim, &painted, &Converter::default(), &mut world.registry, 7, 63);
        let reg = &world.registry;
        let t = dim.ground().view(TilePos::new(0, 0)).unwrap();
        let column = |x: i32, z: i32| (massif_core::compose::block_top(t.top.get(local_index(x, z))) - 1, t.fluid.get(local_index(x, z)));
        // In the middle: the water's top block at 78, the bed 6 to 9 down.
        let (bed, fluid) = column(64, 60);
        assert_eq!(fluid, 79, "the water two blocks under the ground");
        assert!((71..=74).contains(&bed), "bed at {bed}");
        let terrain = reg.terrain(TerrainId(t.terrain.get(local_index(64, 60)))).name.clone();
        assert!(terrain == "Gravel" || terrain == "Clay", "{terrain}");
        assert_eq!(reg.biome(massif_core::registry::BiomeId(t.biome.get(local_index(64, 60)))).id, "minecraft:river");
        // At its edge: a block or two dug, no water.
        let (edge, dry) = column(64, 50);
        assert!((78..=79).contains(&edge) && dry == NO_FLUID, "{edge} {dry}");
        // Not painted: as it was.
        assert_eq!(column(64, 40), (80, NO_FLUID));
        // By the lake: its water at the lake's level.
        assert_eq!(column(13, 10).1, 90);
    }

    /// WorldPainter's River layer painted down a slope and across one: its
    /// water stands beside no lower land and over no air, and once
    /// exported the game leaves it as it is.
    #[test]
    fn worldpainters_river_layer_holds_on_a_slope() {
        use massif_core::coords::local_index;
        use massif_core::world::{DimensionKind, World, WorldMeta};
        for across in [false, true] {
            let mut world = World::new(WorldMeta::default());
            let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
            let grass = world.registry.terrain_by_name("Grass").unwrap().0;
            for tx in 0..2 {
                let mut tile = Tile::ground(81.0, grass, NO_FLUID);
                for z in 0..128 {
                    for x in 0..128 {
                        // Down 1 in 8 to the east, and 1 in 4 to the south.
                        let bx = tx * 128 + x;
                        tile.top.set(local_index(bx, z), 120.0 - bx as f32 / 8.0 - z as f32 / 4.0);
                    }
                }
                dim.ground_mut().insert(TilePos::new(tx, 0), tile);
            }
            // A band 16 wide down the slope (along x), or across it (along z).
            let painted_at = |x: i32, z: i32| if across { (100..116).contains(&x) } else { (56..72).contains(&z) };
            type Painted = Vec<(TilePos, Vec<(Conv, Vec<u8>)>)>;
            let painted: Painted = (0..2)
                .map(|tx| (TilePos::new(tx, 0), vec![(Conv::River, (0..massif_core::TILE_AREA).map(|i| if painted_at(tx * 128 + (i & 127) as i32, (i >> 7) as i32) { 255 } else { 0 }).collect())]))
                .collect();
            apply_after(&mut dim, &painted, &Converter::default(), &mut world.registry, 7, 63);
            let held = massif_core::water::check(&dim);
            assert!(held.columns > 500, "across {across}: {held:?}");
            assert_eq!((held.spills, held.floating), (0, 0), "across {across}: {held:?}");
            assert_eq!(massif_core::water::settle_all(&dim, true).unsettled, 0, "across {across}");
        }
    }

    /// Custom ground covers, tunnels and caves applied to a dimension:
    /// the cover's blocks over the ground's terrain (raised by its
    /// thickness, or in place of the top of it), a level of a tunnel
    /// flooded to its level, and caves carved only where painted.
    #[test]
    fn layers_after_the_tiles() {
        use massif_core::world::{DimensionKind, World, WorldMeta};
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let grass = world.registry.terrain_by_name("Grass").unwrap().0;
        for z in -1..1 {
            for x in -1..1 {
                dim.ground_mut().insert(TilePos::new(x, z), Tile::ground(71.0, grass, NO_FLUID));
            }
        }
        let mut conv = Converter::default();
        conv.ground_covers.push(GroundCover { name: "Snow Block".into(), blocks: vec![("minecraft:snow_block".into(), 1)], thickness: 2 });
        conv.ground_covers.push(GroundCover { name: "Gravel".into(), blocks: vec![("minecraft:gravel".into(), 1)], thickness: -3 });
        let bound = |mode, level| Bound { mode, level, min: i32::MIN, max: i32::MAX };
        conv.tunnels.push(Tunnel { name: "Tunnels".into(), floating: false, floor: bound(Mode::Depth, 12), roof: bound(Mode::AboveFloor, 6), flood: Some(60) });
        let row = |r: usize| -> Vec<u8> { (0..massif_core::TILE_AREA).map(|i| if i >> 7 == r { 255 } else { 0 }).collect() };
        let everywhere = vec![255u8; massif_core::TILE_AREA];
        let painted = vec![
            (TilePos::new(0, 0), vec![(Conv::GroundCover(0), row(1)), (Conv::GroundCover(1), row(2)), (Conv::Tunnel(0), row(3))]),
            (TilePos::new(-1, -1), vec![(Conv::Caves(CaveKind::Caverns), everywhere)]),
        ];
        apply_after(&mut dim, &painted, &conv, &mut world.registry, 7, 63);

        let reg = &world.registry;
        let tile = dim.ground().view(TilePos::new(0, 0)).unwrap();
        let snow = reg.terrain(TerrainId(tile.terrain.get(128 + 5)));
        assert_eq!(snow.name, "Snow Block on Grass");
        assert_eq!(reg.block(snow.strata[0].blocks[0].0).state, "minecraft:snow_block");
        assert_eq!(snow.strata[0].depth, (2, 2));
        assert_eq!(snow.strata[1..], reg.terrain(TerrainId(grass)).strata[..]);
        assert_eq!(tile.top.get(128 + 5), 73.0, "raised by its thickness");
        let gravel = reg.terrain(TerrainId(tile.terrain.get(256 + 5)));
        assert_eq!((gravel.name.as_str(), gravel.strata[0].depth), ("Gravel on Grass", (3, 3)));
        assert_eq!(tile.top.get(256 + 5), 71.0, "in place of the top of the ground");
        assert_eq!((tile.terrain.get(5), tile.top.get(5)), (grass, 71.0));
        drop(tile);

        // The tunnel: carved from the block over its floor (70 - 12 = 58) up
        // to its roof (64), water to 60.
        let level = dim.levels.iter().find(|l| l.name == "Tunnels").unwrap();
        assert_eq!(level.kind, massif_core::level::LevelKind::Carve);
        let carve = level.view(TilePos::new(0, 0)).unwrap();
        let i = 3 * 128 + 9;
        assert_eq!((carve.bottom.as_ref().unwrap().get(i), carve.top.get(i), carve.fluid.get(i)), (59.0, 65.0, 61));
        assert!(!carve.present(2 * 128 + 9) && !carve.present(4 * 128 + 9));
        drop(carve);

        // Caverns: a layer of chambers, painted in the tile painted, nowhere
        // else.
        assert_eq!(dim.caves.defs().len(), 1);
        let caverns = &dim.caves.defs()[0];
        assert_eq!(caverns.name, "Caverns");
        assert!(caverns.cheese.on && !caverns.ravines.on && !caverns.spaghetti.on && caverns.cheese.density == 1.0);
        assert_eq!(dim.ground().view(TilePos::new(-1, -1)).unwrap().caves.uniform_value(), Some(1));
        assert_eq!(dim.ground().view(TilePos::new(0, 0)).unwrap().caves.uniform_value(), Some(0));
        assert!(dim.voxels.is_empty(), "nothing sculpted");
        assert!(dim.selection.is_none());
    }

    #[test]
    fn strength_keeps_its_share_of_columns() {
        let count = |s: u8, curve| (0..10_000).filter(|&i| kept(s, curve, i % 100, i / 100, 1)).count();
        assert_eq!(count(255, Curve::Linear), 10_000);
        assert_eq!(count(0, Curve::Square), 0);
        let half = count(128, Curve::Linear);
        assert!((4_700..5_300).contains(&half), "{half}");
        let quarter = count(128, Curve::Square);
        assert!((2_200..2_800).contains(&quarter), "{quarter}");
    }
}
