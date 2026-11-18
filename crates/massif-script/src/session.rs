//! The state of a script's run: the worlds it holds, its height maps,
//! layers, filters and custom terrains, and the mapping operations waiting
//! to be applied. The JavaScript side (`js.rs`, `wp.js`, `massif.js`) only
//! holds numbers standing for these.
//!
//! Columns are read and written with WorldPainter's conventions: `(x, y)`
//! is Massif's `(x, z)`; a height is the y of the top block (Massif's
//! surface elevation less one); a water level likewise; a layer's value is
//! 0 or 1 for a layer of one bit, 0 to 15 for one of a nibble (8 is half),
//! a biome's number for the biomes (255: automatic).

use crate::access::Access;
use crate::image::{self, Image};
use massif_core::coords::{TILE_SIZE, TilePos, local_index};
use massif_core::registry::Registry;
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::treepack::TreeKind;
use massif_core::world::{DimensionKind, World};
use massif_legacy::layers::{CaveKind, Conv, Curve};
use massif_legacy::terrains::{BEACH_MARK, Beaches, CustomMaterial, Mapper};
use massif_legacy::{BIOMES, TERRAINS};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub type R<T> = Result<T, String>;

/// Caves painted, by dimension and tile: each kind's strength in every
/// column (0 to 255).
pub type Painted = HashMap<(usize, TilePos), Vec<(CaveKind, Vec<u8>)>>;

/// A line of a script's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// `print`, `console.log`.
    Out(String),
    /// `console.warn`, and what Massif does not do as WorldPainter does.
    Warn(String),
    /// `console.error`, and the error that stopped the script.
    Error(String),
}

/// How far a run is: a fraction and what it is doing.
#[derive(Debug, Default)]
pub struct Progress {
    fraction: AtomicU32,
    label: Mutex<String>,
}

impl Progress {
    pub fn set(&self, fraction: f32, label: &str) {
        self.fraction.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        if let Ok(mut l) = self.label.lock()
            && *l != label
        {
            label.clone_into(&mut l);
        }
    }

    pub fn set_fraction(&self, fraction: f32) {
        self.fraction.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.fraction.load(Ordering::Relaxed))
    }

    pub fn label(&self) -> String {
        self.label.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

/// Where a run reports and how it is stopped.
#[derive(Clone)]
pub struct Io {
    pub out: Arc<dyn Fn(Line) + Send + Sync>,
    pub progress: Arc<Progress>,
    pub cancel: Arc<AtomicBool>,
}

impl Io {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> R<()> {
        if self.cancelled() { Err(CANCELLED.into()) } else { Ok(()) }
    }
}

/// The message of a run stopped by the user.
pub const CANCELLED: &str = "Script execution interrupted by user";

/// A layer's data size, as WorldPainter's `Layer.DataSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Bit,
    BitPerChunk,
    Nibble,
    Byte,
    None,
}

impl Size {
    pub fn name(self) -> &'static str {
        match self {
            Size::Bit => "BIT",
            Size::BitPerChunk => "BIT_PER_CHUNK",
            Size::Nibble => "NIBBLE",
            Size::Byte => "BYTE",
            Size::None => "NONE",
        }
    }

    pub fn is_bit(self) -> bool {
        matches!(self, Size::Bit | Size::BitPerChunk)
    }
}

/// A layer as one world has it: what painting it there changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bound {
    Annotations,
    /// The painted biome.
    Biome,
    /// Nothing in the column, down to the floor of the world.
    Void,
    /// A ground cover, by number; `bit` for one that is on or off.
    Cover { n: u16, bit: bool },
    /// An object layer, by number: trees, objects; `under` for the
    /// sub-canopy slot.
    Objects { n: u16, curve: Curve, under: bool },
    /// Underground pockets, by number.
    Pockets(u16),
    /// A combined layer, by number: its parts painted at once.
    Combined(u16),
    /// A terrain that the layer is (a WorldPainter ground cover that takes
    /// the place of the top of the land, from a combined layer).
    Terrain { id: u16, bit: bool },
    /// A WorldPainter custom ground cover, by its index in
    /// [`Session::ground_covers`]: its blocks over the terrain under it
    /// ("River on Grass"), the ground raised by its thickness, as a
    /// `.world` painted with it imports.
    GroundCover(u16),
    /// Caves carved where painted, once the script is done.
    Caves(CaveKind),
    /// Populate, Read Only: nothing to do in Massif.
    Inert,
}

#[derive(Debug, Clone)]
enum Source {
    /// One of WorldPainter's own layers, by class.
    Standard(&'static str),
    Annotations,
    Biome,
    Inert,
    Caves(CaveKind),
    Tree(TreeKind),
    /// What WorldPainter calls it, and why Massif has no such layer.
    Unsupported(&'static str),
    /// A `.layer` file's bytes.
    File(Arc<Vec<u8>>),
    /// A layer of one world.
    Bound { world: usize, bound: Bound },
}

#[derive(Debug, Clone)]
pub struct LayerDef {
    pub name: String,
    pub size: Size,
    source: Source,
}

/// WorldPainter's own layers, by the names `wp.getLayer().withName()`
/// takes.
fn standard(name: &str) -> Option<(Source, Size)> {
    use CaveKind::*;
    Some(match name {
        "Frost" => (Source::Standard("Frost"), Size::Bit),
        "Lava" => (Source::Standard("FloodWithLava"), Size::Bit),
        "Void" => (Source::Standard("Void"), Size::Bit),
        "Deciduous" => (Source::Standard("DeciduousForest"), Size::Nibble),
        "Pine" => (Source::Standard("PineForest"), Size::Nibble),
        "Jungle" => (Source::Standard("Jungle"), Size::Nibble),
        "Swamp" => (Source::Standard("SwampLand"), Size::Nibble),
        "Annotations" => (Source::Annotations, Size::Nibble),
        "Biomes" | "Biome" => (Source::Biome, Size::Byte),
        "Populate" | "Read Only" => (Source::Inert, Size::BitPerChunk),
        "Caverns" => (Source::Caves(Caverns), Size::Nibble),
        "Chasms" => (Source::Caves(Chasms), Size::Nibble),
        "Caves" => (Source::Caves(Tunnels), Size::Nibble),
        "Resources" => (Source::Unsupported("The Resources layer (Massif lays its ores over the whole map: Edit ▸ Ores)"), Size::Nibble),
        "Half Blocks" => (Source::Unsupported("The Half Blocks layer"), Size::Bit),
        "River" => (Source::Unsupported("WorldPainter's River layer (Massif's rivers: Edit ▸ Rivers)"), Size::Nibble),
        _ => {
            let kind = TreeKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name))?;
            (Source::Tree(kind), Size::Nibble)
        }
    })
}

/// A height map: an image a script loaded.
pub struct HeightMap {
    pub path: PathBuf,
    pub name: String,
    pub channel: usize,
    pub header: image::Header,
    pub image: Option<Arc<Image>>,
    pub argb: Option<Arc<Vec<u32>>>,
}

impl HeightMap {
    /// Decode it, if it is not already.
    pub fn image(&mut self) -> R<Arc<Image>> {
        if let Some(i) = &self.image {
            return Ok(i.clone());
        }
        let i = Arc::new(image::decode(&self.path, self.channel)?);
        self.image = Some(i.clone());
        Ok(i)
    }

    pub fn argb(&mut self) -> R<Arc<Vec<u32>>> {
        if let Some(a) = &self.argb {
            return Ok(a.clone());
        }
        let (_, _, argb) = image::decode_argb(&self.path)?;
        let a = Arc::new(argb);
        self.argb = Some(a.clone());
        Ok(a)
    }
}

/// How a layer condition of a filter compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cmp {
    Equal,
    Higher,
    Lower,
}

/// An "only on" or "except on" condition of a filter, as a script made it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Cond {
    Terrain { index: i32 },
    Layer { layer: usize, value: Option<i32>, cmp: Option<Cmp> },
    Biome { id: i32 },
    AutoBiome { id: i32 },
    AutoBiomes,
    Water,
    Land,
}

/// A filter, as `wp.createFilter()` made it.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterDef {
    pub above_level: Option<i32>,
    pub below_level: Option<i32>,
    #[serde(default)]
    pub feather: bool,
    pub only_on: Option<Cond>,
    pub except_on: Option<Cond>,
    /// Degrees, 0 to 90.
    pub degrees: Option<i32>,
    #[serde(default)]
    pub slope_is_above: bool,
    #[serde(default)]
    pub in_selection: bool,
    #[serde(default)]
    pub outside_selection: bool,
}

/// A world a script holds.
pub struct Slot {
    pub world: World,
    /// The file it came from.
    pub path: Option<PathBuf>,
    /// The map format a script gave it (`org.pepsoft.anvil.1.18`…).
    pub format: Option<String>,
    terrain_ids: Vec<Option<u16>>,
    biome_ids: Vec<Option<u16>>,
    beaches: Option<Arc<Beaches>>,
    /// Caves painted: strengths a column (0 to 255) by dimension and tile,
    /// carved once the script is done.
    pub caves: Painted,
    /// Custom terrain slots filled by the script: WorldPainter's slot (1 to
    /// 96) → Massif's terrain.
    pub custom: HashMap<u8, u16>,
    pub changed: bool,
    /// Tiles warmed by reads since the last trim.
    warmed: usize,
    /// The terrains made for ground covers: (cover, terrain under) → terrain.
    pub ground_made: HashMap<(u16, u16), u16>,
}

/// The WorldPainter ordinal of terrain `CUSTOM_<n>`.
pub fn custom_ordinal(n: u8) -> Option<u8> {
    let name = format!("CUSTOM_{n}");
    TERRAINS.iter().position(|t| *t == name).map(|k| k as u8)
}

/// The custom slot (1 to 96) of a WorldPainter ordinal, if it is one.
pub fn custom_slot(ordinal: u8) -> Option<u8> {
    TERRAINS.get(ordinal as usize)?.strip_prefix("CUSTOM_")?.parse().ok()
}

/// Java's `Math.round(float)`.
#[inline]
pub fn java_round(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

impl Slot {
    pub fn new(world: World, path: Option<PathBuf>) -> Self {
        Self {
            world,
            path,
            format: None,
            terrain_ids: vec![None; TERRAINS.len()],
            biome_ids: vec![None; 256],
            beaches: None,
            caves: HashMap::new(),
            custom: HashMap::new(),
            changed: false,
            warmed: 0,
            ground_made: HashMap::new(),
        }
    }

    /// The index in `world.dims` of WorldPainter's dimension `dim` (0
    /// surface, 1 Nether, 2 End).
    pub fn dim_index(&self, dim: i32) -> R<usize> {
        let kind = match dim {
            0 => DimensionKind::Overworld,
            1 => DimensionKind::Nether,
            2 => DimensionKind::End,
            _ => return Err("Non existent dimension specified".into()),
        };
        self.world.dims.iter().position(|d| d.kind == kind).ok_or_else(|| "Non existent dimension specified".into())
    }

    fn with_mapper<T>(&mut self, f: impl FnOnce(&mut Mapper) -> T) -> T {
        let mut m = Mapper::new(std::mem::replace(&mut self.world.registry, Registry::vanilla()));
        let out = f(&mut m);
        self.world.registry = m.registry;
        out
    }

    /// Massif's terrain for WorldPainter's terrain `ordinal`, added to the
    /// map if it has none yet; [`BEACH_MARK`] for the beaches, which are
    /// grass, sand, gravel and clay by place.
    pub fn terrain_id(&mut self, ordinal: i32) -> R<u16> {
        let Some(&cached) = usize::try_from(ordinal).ok().and_then(|o| self.terrain_ids.get(o)) else {
            return Err(format!("Invalid terrain index {ordinal} specified"));
        };
        if let Some(id) = cached {
            return Ok(id);
        }
        if let Some(n) = custom_slot(ordinal as u8) {
            return self.custom.get(&n).copied().ok_or_else(|| format!("Custom terrain {n} is not installed in this world: install it first with wp.installCustomTerrain(terrain).toWorld(world).inSlot({n}).go()"));
        }
        let id = self.with_mapper(|m| m.terrain(ordinal as u8));
        self.terrain_ids[ordinal as usize] = Some(id);
        Ok(id)
    }

    /// The terrain of a column painted with terrain `id`, which
    /// [`Slot::terrain_id`] gave: the beaches resolved by place.
    pub fn place_terrain(&mut self, id: u16, x: i32, z: i32, top: f32) -> u16 {
        if id != BEACH_MARK {
            return id;
        }
        self.beaches().at(x, z, top)
    }

    pub fn beaches(&mut self) -> Arc<Beaches> {
        if self.beaches.is_none() {
            self.beaches = Some(Arc::new(Beaches::new(&self.world.registry)));
        }
        self.beaches.clone().expect("just made")
    }

    /// WorldPainter's ordinal of Massif's terrain `id`, or -1.
    pub fn ordinal_of(&self, id: u16) -> i32 {
        if let Some((&n, _)) = self.custom.iter().find(|(_, t)| **t == id) {
            return custom_ordinal(n).map_or(-1, |o| o as i32);
        }
        massif_legacy::terrains::ordinal_of(&self.world.registry, id).map_or(-1, |o| o as i32)
    }

    /// Massif's biome for WorldPainter's biome number (255: automatic, 0).
    pub fn biome_id(&mut self, number: i32) -> R<u16> {
        if number == 255 {
            return Ok(0);
        }
        let Some(&cached) = usize::try_from(number).ok().and_then(|n| self.biome_ids.get(n)) else {
            return Err(format!("Invalid biome {number} specified"));
        };
        if let Some(id) = cached {
            return Ok(id);
        }
        if BIOMES[number as usize].is_none() {
            return Err(format!("Biome {number} is not one Massif knows"));
        }
        let id = self.with_mapper(|m| m.biome(number as u8));
        self.biome_ids[number as usize] = Some(id);
        Ok(id)
    }

    /// Install `material` in custom slot `slot` (1 to 96; 0: the first
    /// free one): WorldPainter's terrain ordinal for it.
    pub fn install_terrain(&mut self, material: &CustomMaterial, slot: i32) -> R<i32> {
        let n = if slot > 0 {
            if slot > 96 {
                return Err(format!("Invalid custom terrain slot {slot} (1 to 96)"));
            }
            slot as u8
        } else {
            (1..=96u8).find(|n| !self.custom.contains_key(n)).ok_or("No free custom terrain slots")?
        };
        let ordinal = custom_ordinal(n).ok_or("No such custom terrain slot")?;
        let mut materials = vec![None; n as usize];
        materials[n as usize - 1] = Some(material.clone());
        let id = self.with_mapper(|m| {
            let id = m.terrain(ordinal);
            m.finish_custom(&materials);
            id
        });
        self.custom.insert(n, id);
        self.terrain_ids[ordinal as usize] = Some(id);
        self.changed = true;
        Ok(ordinal as i32)
    }

    /// The tile at `pos` of dimension `d`'s ground, decoded and kept so.
    fn warm(&mut self, d: usize, pos: TilePos) {
        let level = self.world.dims[d].ground_mut();
        if level.contains(pos) {
            level.warm(&[pos]);
            self.warmed += 1;
            if self.warmed > 4096 {
                // A script walking a large map column by column: keep the
                // memory of a few thousand tiles.
                self.warmed = 0;
                level.trim(2048);
            }
        }
    }

    pub fn read<T>(&mut self, d: usize, x: i32, z: i32, f: impl FnOnce(&Tile, usize) -> T) -> Option<T> {
        let pos = TilePos::containing(x, z);
        self.warm(d, pos);
        let t = self.world.dims[d].ground().view(pos)?;
        Some(f(&t, local_index(x, z)))
    }

    pub fn write<T>(&mut self, d: usize, x: i32, z: i32, f: impl FnOnce(&mut Tile, usize) -> T) -> Option<T> {
        let t = self.world.dims[d].ground_mut().tile_mut(TilePos::containing(x, z))?;
        self.changed = true;
        Some(f(t, local_index(x, z)))
    }

    /// WorldPainter's height of a column: the y of its top block;
    /// `-Float.MAX_VALUE` where the map has no tile.
    pub fn height(&mut self, d: usize, x: i32, z: i32) -> f32 {
        self.read(d, x, z, |t, i| t.top.get(i) - 1.0).unwrap_or(-f32::MAX)
    }

    pub fn water(&mut self, d: usize, x: i32, z: i32) -> i32 {
        let min = self.world.dims[d].min_y;
        self.read(d, x, z, |t, i| water_level(t, i, min)).unwrap_or(min - 1)
    }

    /// The reading context of dimension `d` for layer values.
    pub fn reader(&self, d: usize) -> Reader<'_> {
        Reader::new(self, d)
    }

    /// The biome numbers of WorldPainter by Massif biome.
    pub fn wp_biomes(&self) -> Vec<u8> {
        let reg = &self.world.registry;
        let mut out = vec![255u8; reg.biomes.len()];
        for (k, b) in BIOMES.iter().enumerate() {
            if let Some(name) = b
                && let Some(id) = reg.biome_by_id(name)
                && out[id.0 as usize] == 255
            {
                out[id.0 as usize] = k as u8;
            }
        }
        out
    }
}

/// WorldPainter's water level of a column: one under its fluid's surface;
/// for a column with none, one under the floor of the world.
#[inline]
pub fn water_level(t: &Tile, i: usize, min_y: i32) -> i32 {
    match t.fluid.get(i) {
        NO_FLUID => min_y - 1,
        f => f as i32 - 1,
    }
}

/// What reading a layer's value in a column needs of its world.
pub struct Reader<'a> {
    pub min_y: i32,
    pub d: usize,
    lava_world: bool,
    lava_covers: Vec<bool>,
    wp_biomes: Vec<u8>,
    caves: &'a Painted,
    /// The terrains made for each ground cover.
    ground: HashMap<u16, u16>,
}

impl<'a> Reader<'a> {
    pub fn new(slot: &'a Slot, d: usize) -> Self {
        let reg = &slot.world.registry;
        Self {
            min_y: slot.world.dims[d].min_y,
            d,
            lava_world: slot.world.meta.lava,
            lava_covers: std::iter::once(false).chain(reg.covers.iter().map(|c| c.lava && !c.removed)).collect(),
            wp_biomes: slot.wp_biomes(),
            caves: &slot.caves,
            ground: slot.ground_made.iter().map(|(&(k, _), &t)| (t, k)).collect(),
        }
    }

    /// Whether the column's fluid is lava.
    #[inline]
    pub fn lava(&self, t: &Tile, i: usize) -> bool {
        self.lava_world || self.lava_covers.get(t.cover.get(i) as usize).copied().unwrap_or(false)
    }

    /// WorldPainter's value of layer `b` in column `i` of tile `t` (at
    /// `pos`).
    pub fn value(&self, b: Bound, t: &Tile, pos: TilePos, i: usize) -> i32 {
        let on = |yes: bool, bit: bool| if yes { if bit { 1 } else { 15 } } else { 0 };
        match b {
            Bound::Annotations => t.annotations.get(i) as i32,
            Bound::Biome => match t.biome.get(i) {
                0 => 255,
                b => self.wp_biomes.get(b as usize).copied().unwrap_or(255) as i32,
            },
            Bound::Void => on(t.top.get(i) <= self.min_y as f32, true),
            Bound::Cover { n, bit } => on(t.cover.get(i) == n, bit),
            Bound::Objects { n, under, .. } => on(if under { t.understory.get(i) } else { t.objects.get(i) } == n, false),
            Bound::Pockets(n) => on(t.pockets.get(i) == n, false),
            Bound::Terrain { id, bit } => on(t.terrain.get(i) == id, bit),
            Bound::GroundCover(k) => on(self.ground.get(&t.terrain.get(i)) == Some(&k), true),
            Bound::Caves(kind) => self.caves.get(&(self.d, pos)).and_then(|v| v.iter().find(|c| c.0 == kind)).map_or(0, |c| (c.1[i] / 17) as i32),
            Bound::Combined(_) | Bound::Inert => 0,
        }
    }
}

/// Set WorldPainter's value `v` of layer `b` in column `i` of tile `t`, at
/// `(x, z)`: what Massif's single layer of the slot becomes. A strength
/// keeps the layer in that share of the columns, the same ones for the
/// same strength, as a `.world` imports. `combined` holds the parts of the
/// combined layers by number.
#[allow(clippy::too_many_arguments)]
pub fn set_value(b: Bound, v: i32, t: &mut Tile, i: usize, x: i32, z: i32, min_y: i32, combined: &[Vec<massif_core::combined::Component>]) {
    let strength = (v.clamp(0, 15) * 17) as u8;
    let place = |plane: &mut massif_core::plane::Plane<u16>, n: u16, on: bool| {
        if on {
            plane.set(i, n);
        } else if plane.get(i) == n {
            plane.set(i, 0);
        }
    };
    match b {
        Bound::Annotations => t.annotations.set(i, (v & 15) as u16),
        // Biomes are set by the caller, which maps the number.
        Bound::Biome => {}
        Bound::Void => {
            if v != 0 {
                t.top.set(i, min_y as f32);
                t.fluid.set(i, NO_FLUID);
            }
        }
        Bound::Cover { n, bit } => {
            let on = if bit { v != 0 } else { v > 0 && massif_legacy::layers::kept(strength, Curve::Linear, x, z, n as u64) };
            place(&mut t.cover, n, on);
        }
        Bound::Objects { n, curve, under } => {
            let on = v > 0 && massif_legacy::layers::kept(strength, curve, x, z, 0x10000 | n as u64);
            place(if under { &mut t.understory } else { &mut t.objects }, n, on);
        }
        Bound::Pockets(n) => place(&mut t.pockets, n, v > 0),
        Bound::Terrain { id, .. } => {
            if v != 0 {
                t.terrain.set(i, id);
            }
        }
        Bound::Combined(n) => {
            if v > 0 && massif_legacy::layers::kept(strength, Curve::Linear, x, z, 0x636f_6d62 ^ n as u64) {
                use massif_core::combined::Part;
                for c in combined.get(n as usize).map(Vec::as_slice).unwrap_or(&[]) {
                    let salt = 0x636f_6d62_696e_6564 ^ (c.part as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    if c.share < 1.0 && (massif_core::compose::hash2(x, z, salt) as f32) >= c.share * u32::MAX as f32 {
                        continue;
                    }
                    match c.part {
                        Part::Terrain => t.terrain.set(i, c.layer),
                        Part::Biome => t.biome.set(i, c.layer),
                        Part::Cover => t.cover.set(i, c.layer),
                        Part::Trees => t.objects.set(i, c.layer),
                        Part::Under => t.understory.set(i, c.layer),
                        Part::Size => t.sizes.set(i, c.layer.min(massif_core::tile::SIZE_LARGE)),
                        Part::Pockets => t.pockets.set(i, c.layer),
                    }
                }
            }
        }
        // Caves are kept by the caller until the script is done; ground
        // covers make terrains, which the caller does.
        Bound::Caves(_) | Bound::GroundCover(_) | Bound::Inert => {}
    }
}

/// Paint ground cover `k` on column `i` of `t`: its terrain over the one
/// there, made in `registry` once, the ground raised by its thickness.
pub fn paint_ground_cover(cover: &massif_legacy::layers::GroundCover, k: u16, t: &mut Tile, i: usize, registry: &mut Registry, made: &mut HashMap<(u16, u16), u16>) {
    let base = t.terrain.get(i);
    if made.values().any(|&m| m == base) {
        // Painted over itself: nothing more.
        return;
    }
    let id = *made.entry((k, base)).or_insert_with(|| massif_legacy::layers::ground_terrain(registry, cover, base));
    t.terrain.set(i, id);
    if cover.thickness > 0 {
        t.top.set(i, t.top.get(i) + cover.thickness as f32);
    }
}

/// A run's state.
pub struct Session {
    pub io: Io,
    pub access: Access,
    pub worlds: Vec<Option<Slot>>,
    pub heightmaps: Vec<HeightMap>,
    pub layers: Vec<LayerDef>,
    layer_files: HashMap<PathBuf, usize>,
    bound: HashMap<(usize, usize), Bound>,
    pub filters: Vec<FilterDef>,
    pub materials: Vec<CustomMaterial>,
    /// WorldPainter custom ground covers met in `.layer` files.
    pub ground_covers: Vec<massif_legacy::layers::GroundCover>,
    pub pending: Vec<crate::apply::MapOp>,
    warned: HashSet<String>,
    /// The world the script was given (the open map), if any.
    pub open: Option<usize>,
}

impl Session {
    pub fn new(io: Io, access: Access) -> Self {
        Self {
            io,
            access,
            worlds: Vec::new(),
            heightmaps: Vec::new(),
            layers: Vec::new(),
            layer_files: HashMap::new(),
            bound: HashMap::new(),
            filters: Vec::new(),
            materials: Vec::new(),
            ground_covers: Vec::new(),
            pending: Vec::new(),
            warned: HashSet::new(),
            open: None,
        }
    }

    pub fn print(&self, line: Line) {
        (self.io.out)(line);
    }

    /// Say `message` once a run.
    pub fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            self.print(Line::Warn(message));
        }
    }

    pub fn add_world(&mut self, world: World, path: Option<PathBuf>) -> usize {
        self.worlds.push(Some(Slot::new(world, path)));
        self.worlds.len() - 1
    }

    pub fn slot(&mut self, w: usize) -> R<&mut Slot> {
        self.worlds.get_mut(w).and_then(Option::as_mut).ok_or_else(|| "world may not be null".into())
    }

    /// A world's slot and dimension `dim`, once the operations waiting
    /// have been applied: what reading or writing a column needs.
    pub fn column(&mut self, w: usize, dim: i32) -> R<(&mut Slot, usize)> {
        self.flush()?;
        let slot = self.slot(w)?;
        let d = slot.dim_index(dim)?;
        Ok((slot, d))
    }

    // Layers.

    pub fn layer(&self, l: usize) -> R<&LayerDef> {
        self.layers.get(l).ok_or_else(|| "layer may not be null".into())
    }

    fn add_layer(&mut self, def: LayerDef) -> usize {
        self.layers.push(def);
        self.layers.len() - 1
    }

    /// `wp.getLayer().withName(name)`: one of WorldPainter's own layers.
    pub fn standard_layer(&mut self, name: &str) -> R<usize> {
        if let Some(k) = self.layers.iter().position(|l| l.name == name && !matches!(l.source, Source::File(_) | Source::Bound { .. })) {
            return Ok(k);
        }
        let (source, size) = standard(name).ok_or_else(|| format!("No default layer named \"{name}\" exists and no world specified"))?;
        let name = match &source {
            Source::Biome => "Biome".to_owned(),
            Source::Tree(kind) => kind.name().to_owned(),
            _ => name.to_owned(),
        };
        Ok(self.add_layer(LayerDef { name, size, source }))
    }

    /// `wp.getLayer().fromFile(path)`: a `.layer` file.
    pub fn layer_file(&mut self, file: &str) -> R<usize> {
        let path = self.readable(file)?;
        if let Some(&k) = self.layer_files.get(&path) {
            return Ok(k);
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("I/O error while loading layer {file}: {e}"))?;
        let mut scratch = Registry::vanilla();
        let read = massif_legacy::layer_file::read_layer_file(&bytes, &mut scratch).map_err(|e| format!("{file}: {e}"))?;
        let size = if read.class.contains("GroundCover") || read.class.contains("Tunnel") { Size::Bit } else { Size::Nibble };
        let k = self.add_layer(LayerDef { name: read.name, size, source: Source::File(Arc::new(bytes)) });
        self.layer_files.insert(path, k);
        Ok(k)
    }

    /// `wp.getLayer().fromWorld(world).withName(name)`: a layer the world
    /// has, by name: its ground covers, object layers, pockets and
    /// combined layers, then WorldPainter's own.
    pub fn world_layer(&mut self, w: usize, name: &str) -> R<usize> {
        let slot = self.slot(w)?;
        let reg = &slot.world.registry;
        let eq = |n: &str| n.eq_ignore_ascii_case(name);
        let found = if let Some(n) = reg.cover_by_name(name) {
            let def = reg.cover(n).expect("named");
            let bit = def.frost || def.lava;
            Some((Bound::Cover { n, bit }, if bit { Size::Bit } else { Size::Nibble }))
        } else if let Some(k) = reg.object_layers.iter().position(|l| !l.removed && eq(&l.name)) {
            let n = k as u16 + 1;
            let under = reg.object_layers[k].understory;
            let curve = if TreeKind::ALL.iter().any(|t| eq(t.name())) { Curve::Linear } else { Curve::Square };
            Some((Bound::Objects { n, curve, under }, Size::Nibble))
        } else if let Some(k) = reg.pockets.iter().position(|p| !p.removed && eq(&p.name)) {
            Some((Bound::Pockets(k as u16 + 1), Size::Nibble))
        } else {
            reg.combined.iter().position(|c| !c.removed && eq(&c.name)).map(|k| (Bound::Combined(k as u16 + 1), Size::Nibble))
        };
        match found {
            Some((bound, size)) => Ok(self.add_layer(LayerDef { name: name.to_owned(), size, source: Source::Bound { world: w, bound } })),
            None => self.standard_layer(name).map_err(|_| format!("World contains no layer named \"{name}\"")),
        }
    }

    /// Layer `l` as world `w` has it, added to the world's layers if it
    /// is new there.
    pub fn bind(&mut self, l: usize, w: usize) -> R<Bound> {
        if let Some(&b) = self.bound.get(&(l, w)) {
            return Ok(b);
        }
        let def = self.layer(l)?.clone();
        let size = def.size;
        let bound = match &def.source {
            Source::Annotations => Bound::Annotations,
            Source::Biome => Bound::Biome,
            Source::Caves(kind) => Bound::Caves(*kind),
            Source::Inert => {
                self.warn_once(format!("Layer {} has no effect in Massif, which writes finished chunks itself", def.name));
                Bound::Inert
            }
            Source::Unsupported(what) => return Err(format!("{what} is not supported yet")),
            Source::Bound { world, bound } => {
                if *world != w {
                    return Err(format!("Layer {} belongs to another world: get it from this one with wp.getLayer().fromWorld(world).withName(\"{}\")", def.name, def.name));
                }
                *bound
            }
            Source::Standard(class) => {
                let reg = &mut self.slot(w)?.world.registry;
                match massif_legacy::layers::standard(class, reg) {
                    Some(Conv::Cover(n)) => Bound::Cover { n, bit: size.is_bit() },
                    Some(Conv::Objects(n, curve, under)) => Bound::Objects { n, curve, under },
                    Some(Conv::Void) => Bound::Void,
                    Some(Conv::Annotations) => Bound::Annotations,
                    _ => return Err(format!("Layer {} is not supported yet", def.name)),
                }
            }
            Source::Tree(kind) => {
                let reg = &mut self.slot(w)?.world.registry;
                let n = kind.install(reg);
                Bound::Objects { n, curve: Curve::Linear, under: reg.object_layer(n).is_some_and(|l| l.understory) }
            }
            Source::File(bytes) if massif_legacy::layer_file::read_ground_cover(bytes).is_some() => {
                let cover = massif_legacy::layer_file::read_ground_cover(bytes).expect("just read");
                let k = match self.ground_covers.iter().position(|c| *c == cover) {
                    Some(k) => k,
                    None => {
                        self.ground_covers.push(cover);
                        self.ground_covers.len() - 1
                    }
                };
                Bound::GroundCover(k as u16)
            }
            Source::File(bytes) => {
                let reg = &mut self.slot(w)?.world.registry;
                let read = massif_legacy::layer_file::read_layer_file(bytes, reg).map_err(|e| format!("Layer {}: {e}", def.name))?;
                let left: Vec<String> = read.left_out.iter().map(|l| format!("{l:?}")).collect();
                let made = read.made.first().copied();
                if !left.is_empty() {
                    self.warn_once(format!("Layer {}: left out of Massif: {}", def.name, left.join(", ")));
                }
                use massif_legacy::layer_file::Made;
                match made {
                    Some(Made::Cover(n)) => Bound::Cover { n, bit: size.is_bit() },
                    Some(Made::Objects(n, _)) => {
                        let reg = &self.slot(w)?.world.registry;
                        Bound::Objects { n, curve: Curve::Square, under: reg.object_layer(n).is_some_and(|l| l.understory) }
                    }
                    Some(Made::Pockets(n)) => Bound::Pockets(n),
                    Some(Made::Combined(n)) => Bound::Combined(n),
                    Some(Made::Terrain(id)) => Bound::Terrain { id, bit: size.is_bit() },
                    None => return Err(format!("Layer {} ({}) is not supported yet", def.name, read.class)),
                }
            }
        };
        self.slot(w)?.changed = true;
        self.bound.insert((l, w), bound);
        Ok(bound)
    }

    // Files.

    pub fn readable(&self, name: &str) -> R<PathBuf> {
        let path = self.access.check(name, false)?;
        if !path.exists() {
            return Err(format!("File {name} not found"));
        }
        if !path.is_file() {
            return Err(format!("File {name} is not a file"));
        }
        Ok(path)
    }

    // Height maps.

    /// `wp.getHeightMap().fromFile(file)`, optionally one channel: its
    /// header is read now, the image when it is used.
    pub fn load_heightmap(&mut self, file: &str, channel: Option<usize>) -> R<usize> {
        let path = self.readable(file)?;
        let header = image::header(&path)?;
        if channel.is_some() && header.grey {
            return Err("Colour channel selected for grey scale image".into());
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.heightmaps.push(HeightMap { path, name, channel: channel.unwrap_or(0), header, image: None, argb: None });
        Ok(self.heightmaps.len() - 1)
    }

    pub fn heightmap(&mut self, h: usize) -> R<&mut HeightMap> {
        self.heightmaps.get_mut(h).ok_or_else(|| "heightMap may not be null".into())
    }

    // Custom terrains.

    pub fn terrain_file(&mut self, file: &str) -> R<usize> {
        let path = self.readable(file)?;
        let bytes = std::fs::read(&path).map_err(|e| format!("I/O error while loading terrain {file}: {e}"))?;
        let m = massif_legacy::read_terrain_file(&bytes).map_err(|e| format!("{file} is {e}"))?;
        self.materials.push(m);
        Ok(self.materials.len() - 1)
    }

    pub fn finish(&mut self) -> R<()> {
        self.flush()?;
        for slot in self.worlds.iter_mut().flatten() {
            carve(slot);
        }
        Ok(())
    }
}

/// Carve the caves painted in `slot`, as a `.world`'s are.
pub fn carve(slot: &mut Slot) {
    if slot.caves.is_empty() {
        return;
    }
    let caves = std::mem::take(&mut slot.caves);
    let mut by_dim: HashMap<usize, Vec<(TilePos, massif_legacy::layers::After)>> = HashMap::new();
    for ((d, pos), painted) in caves {
        let after = painted.into_iter().map(|(kind, s)| (Conv::Caves(kind), s)).collect();
        by_dim.entry(d).or_default().push((pos, after));
    }
    let seed = slot.world.meta.seed;
    let sea = slot.world.meta.sea_level - 1;
    for (d, painted) in by_dim {
        let conv = massif_legacy::layers::Converter::default();
        let dim = &mut slot.world.dims[d];
        massif_legacy::layers::apply_after(dim, &painted, &conv, &mut slot.world.registry, seed, sea);
    }
    slot.changed = true;
}

/// The tiles of a rectangle of blocks `[x0, x1) × [z0, z1)`.
pub fn tiles_of(x0: i32, z0: i32, x1: i32, z1: i32) -> impl Iterator<Item = TilePos> {
    let (tx0, tz0) = (x0.div_euclid(TILE_SIZE), z0.div_euclid(TILE_SIZE));
    let (tx1, tz1) = ((x1 - 1).div_euclid(TILE_SIZE), (z1 - 1).div_euclid(TILE_SIZE));
    (tz0..=tz1).flat_map(move |z| (tx0..=tx1).map(move |x| TilePos::new(x, z)))
}

/// The path of a world file next to `path` with Massif's extension.
pub fn massif_path(path: &Path) -> PathBuf {
    path.with_extension(massif_format::EXTENSION)
}
