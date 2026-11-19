//! Exporting a Massif world as a WorldPainter world (`.world`).
//!
//! The file is what WorldPainter's own Save writes: a gzip stream of Java
//! serialisation, an `org.pepsoft.worldpainter.World2` holding its
//! dimensions, their tiles of 128 × 128 columns and the layers painted on
//! them, written by [`crate::javawrite`] from the descriptors of
//! WorldPainter 2.27 (which WorldPainter Modernised shares).
//!
//! Per column a tile holds the height (×256, so the sub-block heights of
//! Massif are kept), the terrain, the water level and layer values. Massif
//! gives each column of the ground level:
//!
//! - its height and its water (lava where Massif has lava);
//! - its terrain: one of WorldPainter's own where the Massif terrain is
//!   that terrain made of the game's blocks, otherwise a custom terrain (a
//!   mixed material of its blocks, mod blocks included), 96 at most;
//! - its biome, WorldPainter's number for it or a custom biome by name;
//! - its layers: Frost, Lava, annotations, Void, a plant layer for each
//!   ground cover of the game's plants (a ground cover layer of its blocks
//!   for others), a custom object layer for each object layer, with its
//!   objects in the file, and a layer of underground pockets for each of
//!   Massif's.
//!
//! What WorldPainter cannot hold (the levels over and under the ground,
//! sculpted voxels, smooth terrain, tree sizes) is left out, and the
//! [`WpReport`] says so, with what was approximated.

use crate::javawrite::{GzipWriter, Writer};
use crate::wp_classes::{LAYERS, LayerSpec, PLATFORMS, PlatformSpec};
use massif_core::level::LevelKind;
use massif_core::registry::{Mix, Registry, TerrainId};
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::world::{Dimension, DimensionKind, World};
use massif_core::{TILE_AREA, TilePos};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const WP: &str = "org.pepsoft.worldpainter.";
const LAYER: &str = "org.pepsoft.worldpainter.layers.Layer";
const CUSTOM_LAYER: &str = "org.pepsoft.worldpainter.layers.CustomLayer";
const TERRAIN: &str = "org.pepsoft.worldpainter.Terrain";
/// How many custom terrains a WorldPainter world has room for.
const CUSTOM_TERRAINS: usize = 96;
/// `Terrain.BARE_GRASS`: grass blocks without the flowers and tall grass
/// WorldPainter's `GRASS` adds, as Massif's Grass is.
const BARE_GRASS: u8 = 29;
/// How the description of the ground cover layer a Massif cover of plants
/// becomes ends, by which the import knows it for a cover again.
pub(crate) const COVER_MARK: &str = "(a ground cover from Massif)";
/// A biome WorldPainter lets Minecraft choose.
const AUTO_BIOME: u8 = 255;

#[derive(Debug, thiserror::Error)]
pub enum WpExportError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("the map has no dimension to export")]
    Empty,
    #[error("the export was cancelled")]
    Cancelled,
}

/// Choices for the WorldPainter world.
#[derive(Debug, Clone)]
pub struct WpOptions {
    /// The version of Minecraft the world is for ("1.21.11", "1.20.1",
    /// "1.12.2"): it picks WorldPainter's platform.
    pub minecraft: String,
    /// `SURVIVAL`, `CREATIVE`, `ADVENTURE` or `HARDCORE`.
    pub game_type: &'static str,
    /// 0 peaceful to 3 hard.
    pub difficulty: i32,
    pub cheats: bool,
}

impl Default for WpOptions {
    fn default() -> Self {
        Self { minecraft: "1.21.11".into(), game_type: "SURVIVAL", difficulty: 2, cheats: false }
    }
}

/// Progress of an export, shared with the thread running it.
#[derive(Debug, Default)]
pub struct WpProgress {
    pub tiles: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

impl WpProgress {
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 { 0.0 } else { (self.tiles.load(Ordering::Relaxed) as f32 / total as f32).min(1.0) }
    }
}

/// What was written, and what WorldPainter could not take.
#[derive(Debug, Default, Clone)]
pub struct WpReport {
    /// WorldPainter's name for the version of Minecraft.
    pub platform: String,
    pub tiles: usize,
    /// One line per dimension.
    pub dimensions: Vec<String>,
    /// The custom terrains made, by name.
    pub custom_terrains: Vec<String>,
    /// The custom biomes made, by name.
    pub custom_biomes: Vec<String>,
    /// The layers written, with the tiles holding each.
    pub layers: Vec<(String, usize)>,
    /// What WorldPainter holds differently.
    pub approximated: Vec<String>,
    /// What WorldPainter cannot hold.
    pub left_out: Vec<String>,
    /// Size of the file.
    pub bytes: u64,
}

impl WpReport {
    /// The report, a line a point.
    pub fn text(&self) -> String {
        let mut s = format!("WorldPainter world for {}: {} tiles", self.platform, self.tiles);
        for d in &self.dimensions {
            s += &format!("\n  {d}");
        }
        if !self.custom_terrains.is_empty() {
            s += &format!("\nCustom terrains ({}): {}", self.custom_terrains.len(), self.custom_terrains.join(", "));
        }
        if !self.custom_biomes.is_empty() {
            s += &format!("\nCustom biomes ({}): {}", self.custom_biomes.len(), self.custom_biomes.join(", "));
        }
        if !self.layers.is_empty() {
            let layers: Vec<String> = self.layers.iter().map(|(n, t)| format!("{n} ({t} tiles)")).collect();
            s += &format!("\nLayers: {}", layers.join(", "));
        }
        for a in &self.approximated {
            s += &format!("\nApproximated: {a}");
        }
        for l in &self.left_out {
            s += &format!("\nLeft out: {l}");
        }
        s
    }
}

/// The WorldPainter platform for a version of Minecraft (`"1.21.11"`,
/// `"26.1.2"`, `"1.12.2"`): the oldest that covers it. A version not read
/// gets the newest.
pub fn platform_for(minecraft: &str) -> &'static PlatformSpec {
    let parts: Vec<u32> = minecraft.split(['.', '-', ' ']).map_while(|p| p.parse().ok()).collect();
    let at_least = |want: &[u32]| {
        let have = |k: usize| parts.get(k).copied().unwrap_or(0);
        (0..want.len()).map(|k| (have(k), want[k])).find(|(h, w)| h != w).is_none_or(|(h, w)| h > w)
    };
    let id = if parts.is_empty() || at_least(&[26]) || (parts[0] == 1 && at_least(&[1, 22])) {
        "org.pepsoft.anvil.26.1"
    } else if at_least(&[1, 21, 11]) {
        "org.pepsoft.anvil.1.21.11"
    } else if at_least(&[1, 20, 5]) {
        "org.pepsoft.anvil.1.20.5"
    } else if at_least(&[1, 19]) {
        "org.pepsoft.anvil.1.19"
    } else if at_least(&[1, 18]) {
        "org.pepsoft.anvil.1.18"
    } else if at_least(&[1, 17]) {
        "org.pepsoft.anvil.1.17"
    } else if at_least(&[1, 13]) {
        "org.pepsoft.anvil.1.13"
    } else if at_least(&[1, 2]) {
        "org.pepsoft.anvil"
    } else {
        "org.pepsoft.mcregion"
    };
    PLATFORMS.iter().find(|p| p.id == id).expect("a known platform")
}

fn platform_int(p: &PlatformSpec, name: &str) -> i32 {
    p.ints.iter().find(|i| i.0 == name).map_or(0, |i| i.1)
}

/// The build limits WorldPainter allows on `p` nearest Massif's `min..max`:
/// the smallest that hold them, or the largest there are.
fn heights(p: &PlatformSpec, min: i32, max: i32) -> (i32, i32) {
    let maxes: Vec<i32> = if p.max_heights.is_empty() { vec![platform_int(p, "standardMaxHeight")] } else { p.max_heights.to_vec() };
    let mins: Vec<i32> = if p.min_heights.is_empty() { vec![platform_int(p, "minZ")] } else { p.min_heights.to_vec() };
    let wp_max = maxes.iter().copied().filter(|&m| m >= max).min().unwrap_or_else(|| maxes.iter().copied().max().unwrap_or(256));
    let wp_min = mins.iter().copied().filter(|&m| m <= min).max().unwrap_or_else(|| mins.iter().copied().min().unwrap_or(0));
    (wp_min, wp_max)
}

/// FNV-1a: stable ids from names, so the same map gives the same file.
fn hash(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for &b in *p {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A version 4 UUID made from `parts`.
fn uuid(parts: &[&[u8]]) -> u128 {
    let hi = hash(parts) as u128;
    let lo = hash(&[parts.concat().as_slice(), b"uuid"]) as u128;
    let v = (hi << 64) | lo;
    (v & !(0xF000u128 << 64) | (0x4000u128 << 64)) & !(0xC000_0000_0000_0000u128) | 0x8000_0000_0000_0000u128
}

/// A block state `minecraft:oak_log[axis=y]` as a name and its properties.
fn split_state(state: &str) -> (String, Vec<(String, String)>) {
    let (name, props) = match state.find('[') {
        Some(k) => (&state[..k], state[k + 1..].trim_end_matches(']')),
        None => (state, ""),
    };
    let name = if name.contains(':') { name.to_owned() } else { format!("minecraft:{name}") };
    let props = props
        .split(',')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    (name, props)
}

fn short_name(state: &str) -> &str {
    let name = state.split('[').next().unwrap_or(state);
    name.rsplit(':').next().unwrap_or(name)
}

fn argb(c: [u8; 3]) -> i32 {
    (0xFF00_0000u32 | (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32) as i32
}

/// How a mixed material lays its blocks (`MixedMaterial.Mode`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum MixMode {
    Simple,
    Noise,
    Blobs,
    Layered,
}

impl MixMode {
    fn name(self) -> &'static str {
        match self {
            MixMode::Simple => "SIMPLE",
            MixMode::Noise => "NOISE",
            MixMode::Blobs => "BLOBS",
            MixMode::Layered => "LAYERED",
        }
    }
}

/// A WorldPainter `MixedMaterial`: blocks with their counts.
#[derive(Debug, Clone)]
struct Mixed {
    name: String,
    rows: Vec<(String, u32)>,
    mode: MixMode,
    scale: f32,
    color: [u8; 3],
}

impl Mixed {
    /// A two-block plant over the terrain: its lower half, then its upper.
    fn tall_plant(name: &str, state: &str, color: [u8; 3]) -> Self {
        let (base, props) = split_state(state);
        let half = |h: &str| {
            let mut p: Vec<String> = props.iter().filter(|(k, _)| k != "half").map(|(k, v)| format!("{k}={v}")).collect();
            p.push(format!("half={h}"));
            p.sort();
            format!("{base}[{}]", p.join(","))
        };
        // A layered material lists its rows from the top down.
        Self { name: name.to_owned(), rows: vec![(half("upper"), 1), (half("lower"), 1)], mode: MixMode::Layered, scale: 1.0, color }
    }

    fn new(name: &str, rows: Vec<(String, u32)>, mix: Mix, color: [u8; 3]) -> Self {
        let mode = match (rows.len(), mix) {
            (0 | 1, _) => MixMode::Simple,
            (_, Mix::Grain) => MixMode::Noise,
            (_, Mix::Patches { .. }) => MixMode::Blobs,
            (_, Mix::Beds) => MixMode::Layered,
        };
        let scale = match mix {
            Mix::Patches { size } => (size / 8.0).clamp(0.1, 10.0),
            _ => 1.0,
        };
        Self { name: name.to_owned(), rows, mode, scale, color }
    }
}

/// One of the layers written.
#[derive(Debug, Clone)]
enum WpLayer {
    /// One of WorldPainter's own.
    Own(&'static LayerSpec),
    /// A plant layer: its plants by index in WorldPainter's catalogue,
    /// with their counts and growth.
    Plants { name: String, color: [u8; 3], plants: Vec<(usize, u16, i32, i32)>, valid_only: bool },
    /// A ground cover layer, so many blocks thick (into the terrain when
    /// negative).
    Cover { name: String, color: [u8; 3], material: Mixed, thickness: i32 },
    /// A two-block plant of a mod's, as a ground cover layer two blocks
    /// thick, its material laid from the terrain up.
    TallPlant { name: String, color: [u8; 3], material: Mixed },
    /// A custom object layer.
    Objects { name: String, color: [u8; 3], objects: Vec<(u32, u16)>, density: i32, grid: i32, displacement: i32, rotate: bool, underwater: bool, sink: i16 },
    /// A layer of underground pockets.
    Pockets { name: String, color: [u8; 3], material: Mixed, frequency: i32, scale: i32, low: i32, high: i32 },
}

impl WpLayer {
    fn name(&self) -> &str {
        match self {
            WpLayer::Own(s) => s.name,
            WpLayer::Plants { name, .. } | WpLayer::Cover { name, .. } | WpLayer::TallPlant { name, .. } | WpLayer::Objects { name, .. } | WpLayer::Pockets { name, .. } => name,
        }
    }

    fn custom(&self) -> bool {
        !matches!(self, WpLayer::Own(_))
    }
}

fn own_layer(class: &str) -> &'static LayerSpec {
    let class = format!("{WP}layers.{class}");
    LAYERS.iter().find(|l| l.class == class).expect("a layer of the table")
}

/// What one column of the ground of a dimension becomes, set once for the
/// whole export.
struct Plan {
    platform: &'static PlatformSpec,
    /// WorldPainter's terrain for each Massif terrain.
    terrains: Vec<u8>,
    /// The custom terrains, by slot.
    customs: Vec<Mixed>,
    /// The terrain of the tile factory: the most common.
    main_terrain: u8,
    /// WorldPainter's biome for each Massif biome.
    biomes: Vec<u8>,
    /// Custom biomes: name, number, colour.
    custom_biomes: Vec<(String, u8, [u8; 3])>,
    layers: Vec<WpLayer>,
    frost: Option<usize>,
    lava: Option<usize>,
    annotations: Option<usize>,
    void: Option<usize>,
    biome: Option<usize>,
    /// The layer of each cover, object layer and pocket layer, by number.
    covers: Vec<Option<usize>>,
    /// The ground cover layer of each terrain past WorldPainter's room for
    /// custom terrains, by terrain.
    terrain_covers: Vec<Option<usize>>,
    /// For a cover of a mod's plants, the layer of each of its plants, by
    /// the plant's place in the cover.
    cover_plants: Vec<Vec<Option<usize>>>,
    /// Whether the top of each terrain is soil, for covers that grow only
    /// on soil.
    soil: Vec<bool>,
    /// WorldPainter's terrain for the rock under the terrains.
    subsurface: &'static str,
    /// The tiles each layer was written on, for the report.
    layer_tiles: Vec<AtomicUsize>,
    objects: Vec<Option<usize>>,
    pockets: Vec<Option<usize>>,
    /// Whether the world's fluid is lava.
    lava_world: bool,
    seed: i64,
    sea: i32,
}

/// What a dimension holds, counted on all its tiles.
#[derive(Default)]
struct Survey {
    terrains: HashMap<u16, u64>,
    biomes: HashMap<u16, u64>,
    covers: HashMap<u16, u64>,
    objects: HashMap<u16, u64>,
    pockets: HashMap<u16, u64>,
    annotations: bool,
    void: bool,
    sizes: u64,
    slabs: u64,
    high: u64,
    low: u64,
}

impl Survey {
    fn merge(mut self, other: Survey) -> Survey {
        for (a, b) in [(&mut self.terrains, other.terrains), (&mut self.biomes, other.biomes), (&mut self.covers, other.covers), (&mut self.objects, other.objects), (&mut self.pockets, other.pockets)] {
            for (k, v) in b {
                *a.entry(k).or_default() += v;
            }
        }
        self.annotations |= other.annotations;
        self.void |= other.void;
        self.sizes += other.sizes;
        self.slabs += other.slabs;
        self.high += other.high;
        self.low += other.low;
        self
    }
}

fn count_runs(plane: &massif_core::plane::Plane<u16>, into: &mut HashMap<u16, u64>) {
    plane.for_each_run(|v, n| *into.entry(v).or_default() += n as u64);
}

fn survey_tile(t: &Tile, min: i32, wp_min: i32, wp_max: i32) -> Survey {
    let mut s = Survey::default();
    count_runs(&t.terrain, &mut s.terrains);
    count_runs(&t.biome, &mut s.biomes);
    count_runs(&t.cover, &mut s.covers);
    count_runs(&t.objects, &mut s.objects);
    count_runs(&t.understory, &mut s.objects);
    count_runs(&t.pockets, &mut s.pockets);
    s.annotations = t.annotations.uniform_value() != Some(0);
    t.sizes.for_each_run(|v, n| s.sizes += if v != 0 { n as u64 } else { 0 });
    t.slabs.for_each_run(|v, n| s.slabs += if v != 0 { n as u64 } else { 0 });
    t.top.for_each_run(|h, n| {
        if h <= min as f32 {
            s.void = true;
        }
        if h - 1.0 > (wp_max - 1) as f32 {
            s.high += n as u64;
        } else if h - 1.0 < wp_min as f32 && h > min as f32 {
            s.low += n as u64;
        }
    });
    s
}

/// The tiles of a dimension's ground, in order.
fn ground_tiles(dim: &Dimension) -> Vec<TilePos> {
    let mut tiles: Vec<TilePos> = dim.ground().positions().collect();
    tiles.sort_by_key(|p| (p.z, p.x));
    tiles
}

fn anchor_dim(kind: DimensionKind) -> i32 {
    match kind {
        DimensionKind::Overworld => 0,
        DimensionKind::Nether => 1,
        DimensionKind::End => 2,
    }
}

impl Plan {
    fn new(world: &World, options: &WpOptions, report: &mut WpReport) -> Plan {
        let reg = &world.registry;
        let platform = platform_for(&options.minecraft);
        report.platform = platform.display_name.to_owned();
        let mut total = Survey::default();
        for dim in &world.dims {
            let (wp_min, wp_max) = heights(platform, dim.min_y, dim.max_y);
            let ground = dim.ground();
            let tiles = ground_tiles(dim);
            let survey = tiles
                .par_iter()
                .filter_map(|&pos| ground.view(pos).map(|t| survey_tile(&t, dim.min_y, wp_min, wp_max)))
                .reduce(Survey::default, Survey::merge);
            if survey.high > 0 {
                report.approximated.push(format!("{}: {} columns higher than WorldPainter's top for {} (Y {}) were lowered to it", dim.kind.label(), survey.high, platform.display_name, wp_max - 1));
            }
            if survey.low > 0 {
                report.approximated.push(format!("{}: {} columns lower than WorldPainter's floor for {} (Y {wp_min}) were raised to it", dim.kind.label(), survey.low, platform.display_name));
            }
            if (wp_min, wp_max) != (dim.min_y, dim.max_y) {
                report.approximated.push(format!("{}: build limits Y {} to {} became WorldPainter's Y {wp_min} to {}", dim.kind.label(), dim.min_y, dim.max_y - 1, wp_max - 1));
            }
            total = total.merge(survey);
        }

        // ---- terrains ---------------------------------------------------------
        let mut terrains = vec![0u8; reg.terrains.len()];
        let mut used: Vec<(u16, u64)> = total.terrains.iter().map(|(&t, &n)| (t, n)).filter(|t| (t.0 as usize) < reg.terrains.len()).collect();
        used.sort_by_key(|&(t, n)| (std::cmp::Reverse(n), t));
        let mut customs: Vec<Mixed> = Vec::new();
        let mut assigned: Vec<(u16, u8)> = Vec::new();
        let mut over = Vec::new();
        let mut layered = Vec::new();
        for &(t, _) in &used {
            let def = reg.terrain(TerrainId(t));
            if let Some(ordinal) = builtin_ordinal(reg, t) {
                terrains[t as usize] = ordinal;
                assigned.push((t, ordinal));
            } else if customs.len() < CUSTOM_TERRAINS {
                let rows: Vec<(String, u32)> = def.strata.first().map(|s| s.blocks.iter().map(|&(b, w)| (reg.block(b).state.clone(), w.max(1) as u32)).collect()).unwrap_or_default();
                let rows = if rows.is_empty() { vec![("minecraft:stone".to_owned(), 1)] } else { rows };
                if def.strata.len() > 1 && def.strata[1..].iter().any(|s| s.blocks != def.strata[0].blocks) {
                    layered.push(def.name.clone());
                }
                let mix = def.strata.first().map_or(Mix::Grain, |s| s.mix);
                let ordinal = custom_ordinal(customs.len());
                customs.push(Mixed::new(&def.name, rows, mix, def.color));
                terrains[t as usize] = ordinal;
                assigned.push((t, ordinal));
            } else {
                over.push(t);
            }
        }
        // Past WorldPainter's room for custom terrains: the nearest by
        // colour of those given a place, under a ground cover layer of the
        // terrain's own blocks that digs as deep as its top layer (made
        // further down).
        let mut overflow: Vec<(u16, Mixed, i32)> = Vec::new();
        for &t in &over {
            let def = reg.terrain(TerrainId(t));
            let c = def.color;
            let dist = |o: u16| {
                let d = reg.terrain(TerrainId(o)).color;
                (0..3).map(|k| (c[k] as i32 - d[k] as i32).pow(2)).sum::<i32>()
            };
            let (_, ordinal) = assigned.iter().copied().min_by_key(|&(o, _)| dist(o)).unwrap_or((0, BARE_GRASS));
            terrains[t as usize] = ordinal;
            let rows: Vec<(String, u32)> = def.strata.first().map(|s| s.blocks.iter().map(|&(b, w)| (reg.block(b).state.clone(), w.max(1) as u32)).collect()).unwrap_or_default();
            let rows = if rows.is_empty() { vec![("minecraft:stone".to_owned(), 1)] } else { rows };
            let depth = def.strata.first().map_or(1, |s| ((s.depth.0 as i32 + s.depth.1 as i32) / 2).clamp(1, 8));
            overflow.push((t, Mixed::new(&def.name, rows, def.strata.first().map_or(Mix::Grain, |s| s.mix), def.color), depth));
        }
        if !over.is_empty() {
            let names: Vec<&str> = over.iter().map(|&t| reg.terrain(TerrainId(t)).name.as_str()).collect();
            report.approximated.push(format!(
                "{} terrains past WorldPainter's {CUSTOM_TERRAINS} custom terrains became ground cover layers of their blocks, digging into the nearest terrain by colour (in the game the blocks are the same; WorldPainter's map shows the nearest terrain under the layer): {}",
                over.len(),
                names.join(", ")
            ));
        }
        if !layered.is_empty() {
            report.approximated.push(format!("custom terrains keep their top blocks all the way down to WorldPainter's subsurface: {}", layered.join(", ")));
        }
        report.custom_terrains = customs.iter().map(|c| c.name.clone()).collect();
        if used.iter().any(|&(t, _)| terrains[t as usize] == BARE_GRASS) {
            report.approximated.push("Grass is WorldPainter's Bare Grass: plain grass blocks, without the flowers and tall grass its Grass adds (Massif's covers carry the plants)".into());
        }
        let main_terrain = used.first().map_or(BARE_GRASS, |&(t, _)| terrains[t as usize]);

        // ---- biomes -----------------------------------------------------------
        let mut biomes = vec![AUTO_BIOME; reg.biomes.len()];
        let mut custom_biomes = Vec::new();
        let mut free = (0..255u8).filter(|&n| crate::BIOMES[n as usize].is_none());
        let mut biome_ids: Vec<u16> = total.biomes.keys().copied().filter(|&b| b != 0 && (b as usize) < reg.biomes.len()).collect();
        biome_ids.sort_unstable();
        for b in biome_ids {
            let def = &reg.biomes[b as usize];
            let id = if def.id.contains(':') { def.id.clone() } else { format!("minecraft:{}", def.id) };
            match crate::BIOMES.iter().position(|m| *m == Some(id.as_str())) {
                Some(n) => biomes[b as usize] = n as u8,
                None => match free.next() {
                    Some(n) => {
                        biomes[b as usize] = n;
                        custom_biomes.push((id.clone(), n, def.color));
                    }
                    None => report.approximated.push(format!("biome {id} left to Minecraft (no custom biome number left)")),
                },
            }
        }
        report.custom_biomes = custom_biomes.iter().map(|b| b.0.clone()).collect();

        // ---- layers -----------------------------------------------------------
        let mut layers: Vec<WpLayer> = Vec::new();
        let mut add = |l: WpLayer| {
            layers.push(l);
            Some(layers.len() - 1)
        };
        let painted = |m: &HashMap<u16, u64>, n: u16| m.get(&n).is_some_and(|&c| c > 0);
        let biome = total.biomes.iter().any(|(&b, &n)| b != 0 && n > 0).then(|| add(WpLayer::Own(own_layer("Biome")))).flatten();
        let frost_used = total.covers.keys().any(|&n| reg.cover(n).is_some_and(|c| c.frost));
        let frost = frost_used.then(|| add(WpLayer::Own(own_layer("Frost")))).flatten();
        let lava_used = world.meta.lava || total.covers.keys().any(|&n| reg.cover(n).is_some_and(|c| c.lava));
        let lava = lava_used.then(|| add(WpLayer::Own(own_layer("FloodWithLava")))).flatten();
        let annotations = total.annotations.then(|| add(WpLayer::Own(own_layer("Annotations")))).flatten();
        let mut terrain_covers = vec![None; reg.terrains.len()];
        for (t, material, depth) in overflow {
            let def = reg.terrain(TerrainId(t));
            terrain_covers[t as usize] = add(WpLayer::Cover { name: def.name.clone(), color: def.color, material, thickness: -depth });
        }
        let void = total.void.then(|| add(WpLayer::Own(own_layer("Void")))).flatten();

        let mut covers = vec![None; reg.covers.len() + 1];
        let mut cover_plants = vec![Vec::new(); reg.covers.len() + 1];
        for n in 1..=reg.covers.len() as u16 {
            let Some(c) = reg.cover(n) else { continue };
            if !painted(&total.covers, n) || c.removed {
                continue;
            }
            if c.frost {
                covers[n as usize] = frost;
                continue;
            }
            if c.lava {
                covers[n as usize] = lava;
                continue;
            }
            let known: Vec<Option<usize>> = c.plants.iter().map(|(state, _)| plant_index(state)).collect();
            if !c.plants.is_empty() && known.iter().all(Option::is_some) {
                let plants = c
                    .plants
                    .iter()
                    .zip(&known)
                    .map(|((_, weight), k)| {
                        let k = k.expect("known");
                        let (from, to) = if crate::layers::plant_grows_tall(k) { (c.height[0].max(1) as i32, c.height[1].max(c.height[0]).max(1) as i32) } else { (PLANT_GROWTH[k], PLANT_GROWTH[k]) };
                        (k, (*weight).clamp(1, i16::MAX as u16), from, to)
                    })
                    .collect();
                covers[n as usize] = add(WpLayer::Plants { name: c.name.clone(), color: c.color, plants, valid_only: c.soil_only });
            } else {
                // Blocks WorldPainter's plants do not have: a ground cover
                // layer of the one-block plants, and one two blocks thick
                // for each two-block plant, each painted where Massif grows
                // that plant.
                let mut by_plant = vec![None; c.plants.len()];
                let singles: Vec<(String, u32)> = c.plants.iter().filter(|(s, _)| !massif_core::registry::is_tall_plant(s)).map(|(s, w)| (s.clone(), (*w).max(1) as u32)).collect();
                if !singles.is_empty() {
                    let k = add(WpLayer::Cover { name: c.name.clone(), color: c.color, material: Mixed::new(&c.name, singles, Mix::Grain, c.color), thickness: 1 });
                    for (p, (state, _)) in c.plants.iter().enumerate() {
                        if !massif_core::registry::is_tall_plant(state) {
                            by_plant[p] = k;
                        }
                    }
                }
                for (p, (state, _)) in c.plants.iter().enumerate() {
                    if massif_core::registry::is_tall_plant(state) {
                        let name = format!("{} ({})", c.name, short_name(state).replace('_', " "));
                        by_plant[p] = add(WpLayer::TallPlant { name: name.clone(), color: c.color, material: Mixed::tall_plant(&name, state, c.color) });
                    }
                }
                report.approximated.push(format!("ground cover {} became ground cover layers of its blocks (WorldPainter's plant layers have only the game's plants)", c.name));
                cover_plants[n as usize] = by_plant;
            }
        }

        let mut objects = vec![None; reg.object_layers.len() + 1];
        for n in 1..=reg.object_layers.len() as u16 {
            let Some(l) = reg.object_layer(n) else { continue };
            if !painted(&total.objects, n) || l.removed {
                continue;
            }
            let kept: Vec<(u32, u16)> = l.objects.iter().copied().filter(|&(o, _)| reg.objects.get(o as usize).is_some_and(|d| !d.blocks.is_empty())).collect();
            if kept.is_empty() {
                continue;
            }
            // WorldPainter tries every column painted (a grid of 1, so
            // that small painted spots get theirs) and places an object
            // where a draw out of 64 × density falls under the square of
            // the strength (15 here): as many as Massif, which stands one
            // in a share `density` of the squares of its spacing.
            let spacing = l.spacing.max(1) as f32;
            let density = ((226.0 * spacing * spacing / (64.0 * l.density.clamp(0.001, 1.0))).round() as i32).max(1);
            let grid = 1;
            objects[n as usize] = add(WpLayer::Objects {
                name: l.name.clone(),
                color: l.color,
                objects: kept,
                density,
                grid,
                displacement: 0,
                rotate: l.rotate,
                underwater: l.underwater,
                sink: l.sink,
            });
        }

        let mut pockets = vec![None; reg.pockets.len() + 1];
        for n in 1..=reg.pockets.len() as u16 {
            let Some(p) = reg.pocket(n) else { continue };
            if !painted(&total.pockets, n) || p.removed || p.blocks.is_empty() {
                continue;
            }
            let rows = p.blocks.iter().map(|&(b, w)| (reg.block(b).state.clone(), w.max(1) as u32)).collect();
            pockets[n as usize] = add(WpLayer::Pockets {
                name: p.name.clone(),
                color: p.color,
                material: Mixed::new(&p.name, rows, Mix::Grain, p.color),
                frequency: (p.share * 1000.0).round().clamp(1.0, 1000.0) as i32,
                scale: (p.size / 4.099 * 100.0).round().clamp(1.0, 1000.0) as i32,
                low: p.low,
                high: p.high,
            });
        }

        if total.sizes > 0 {
            report.left_out.push(format!("the tree sizes painted on {} columns: WorldPainter picks among all of a layer's objects", total.sizes));
        }
        if total.slabs > 0 || !world.meta.smooth.is_off() {
            report.left_out.push("smooth terrain (slabs and stairs on the land's steps): WorldPainter places whole blocks".into());
        }

        // Covers that grow only on soil, by the top of the terrain under
        // them: soil when most of its top blocks are.
        let soil = reg
            .terrains
            .iter()
            .map(|t| {
                let top = t.strata.first().map(|s| s.blocks.as_slice()).unwrap_or(&[]);
                let (yes, all) = top.iter().fold((0u32, 0u32), |(y, a), &(b, w)| (y + if massif_core::registry::is_soil(&reg.block(b).state) { w as u32 } else { 0 }, a + w as u32));
                all > 0 && yes * 2 >= all
            })
            .collect();
        // The rock: stone, and deepslate under Y 0 where the game has it.
        let deep = world.dims.iter().any(|d| d.min_y < 0) && platform.min_heights.iter().any(|&m| m < 0);
        let subsurface = if deep { "STONE_MIX" } else { "STONE" };
        report.approximated.push(if deep {
            "the rock under the terrains is WorldPainter's Stone Mix: stone, deepslate under Y 0, with patches of granite, diorite, andesite and tuff".to_owned()
        } else {
            "the rock under the terrains is WorldPainter's Stone".to_owned()
        });

        Plan {
            platform,
            terrains,
            customs,
            main_terrain,
            biomes,
            custom_biomes,
            layers,
            frost,
            lava,
            annotations,
            void,
            biome,
            covers,
            terrain_covers,
            cover_plants,
            soil,
            subsurface,
            layer_tiles: Vec::new(),
            objects,
            pockets,
            lava_world: world.meta.lava,
            seed: world.meta.seed,
            sea: world.meta.sea_level,
        }
    }
}

/// `Terrain.CUSTOM_n` for slot `k` (CUSTOM_1 is slot 0).
fn custom_ordinal(k: usize) -> u8 {
    let name = format!("CUSTOM_{}", k + 1);
    crate::TERRAINS.iter().position(|t| *t == name).expect("a custom terrain") as u8
}

/// WorldPainter's own terrain for Massif terrain `t`, when it is one of
/// them made of the game's blocks.
fn builtin_ordinal(reg: &Registry, t: u16) -> Option<u8> {
    let def = reg.terrain(TerrainId(t));
    let ordinal = if def.name.eq_ignore_ascii_case("Grass") { BARE_GRASS } else { crate::terrains::ordinal_of(reg, t)? };
    // A palette, or the player, may have made it of other blocks.
    match massif_core::materials::vanilla_recipe(&def.name) {
        Some(recipe) => {
            let mut ours: Vec<&str> = def.strata.first().map(|s| s.blocks.iter().map(|&(b, _)| reg.block(b).state.as_str()).collect()).unwrap_or_default();
            let mut theirs: Vec<&str> = recipe.strata.first().map(|s| s.blocks.iter().map(|(b, _)| b.as_str()).collect()).unwrap_or_default();
            ours.sort_unstable();
            theirs.sort_unstable();
            (ours == theirs).then_some(ordinal)
        }
        None => Some(ordinal),
    }
}

/// The growth WorldPainter gives each of its plants by default (fully
/// grown crops), by index in its catalogue.
const PLANT_GROWTH: [i32; 95] = {
    let mut g = [1i32; 95];
    let (mut k, list) = (0, [(27, 8), (28, 8), (29, 8), (30, 8), (31, 8), (32, 3), (33, 3), (35, 4), (36, 4), (37, 5), (48, 26), (51, 4), (55, 4), (56, 16), (64, 10), (67, 10), (72, 10), (74, 10), (76, 10), (80, 4), (81, 5), (83, 2), (86, 5), (88, 4), (89, 4)]);
    while k < list.len() {
        g[list[k].0] = list[k].1;
        k += 1;
    }
    g
};

/// The index in WorldPainter's plant catalogue of a block a cover grows.
fn plant_index(state: &str) -> Option<usize> {
    let name = short_name(state);
    let name = if name == "grass" { "short_grass" } else { name };
    if !state.split('[').next().is_some_and(|n| !n.contains(':') || n.starts_with("minecraft:")) {
        return None;
    }
    crate::layers::plant_names().position(|p| !p.is_empty() && short_name(p) == name)
}

// ---- tiles ---------------------------------------------------------------------

enum Heights {
    Int(Vec<i32>),
    Short(Vec<u16>),
}

enum Water {
    Short(Vec<u16>),
    Byte(Vec<u8>),
}

/// A tile as WorldPainter keeps it.
struct WpTile {
    x: i32,
    y: i32,
    heights: Heights,
    terrain: Vec<u8>,
    water: Water,
    /// Byte and nibble layers: the layer and its data.
    bytes: Vec<(usize, Vec<u8>)>,
    /// Bit layers.
    bits: Vec<(usize, Vec<bool>)>,
}

struct DimPlan<'w> {
    dim: &'w Dimension,
    tiles: Vec<TilePos>,
    min: i32,
    max: i32,
    tall: bool,
}

/// The layer cover `n` paints at column `(x, z)`, if any: only where
/// Massif grows a plant there (frost and lava on the whole of it).
#[allow(clippy::too_many_arguments)]
fn cover_layer(plan: &Plan, reg: &Registry, n: u16, x: i32, z: i32, top: f32, fluid: i16, terrain: u16) -> Option<usize> {
    let def = reg.cover(n)?;
    if def.frost || def.lava {
        return plan.covers.get(n as usize).copied().flatten();
    }
    let growth = def.growth(x, z, plan.seed as u64)?;
    let wet = fluid != NO_FLUID && fluid as f32 > top;
    if wet != def.underwater || (def.soil_only && !plan.soil.get(terrain as usize).copied().unwrap_or(false)) {
        return None;
    }
    match plan.cover_plants.get(n as usize).filter(|p| !p.is_empty()) {
        Some(plants) => plants.get(growth.plant).copied().flatten(),
        None => plan.covers.get(n as usize).copied().flatten(),
    }
}

fn convert_tile(t: &Tile, pos: TilePos, d: &DimPlan, plan: &Plan, reg: &Registry) -> WpTile {
    let (min, max) = (d.min, d.max);
    let top = t.top.dense();
    let fluid = t.fluid.dense();
    let raw_top = (max - 1 - min) * 256;
    let height = |i: usize| (((top[i] - 1.0 - min as f32) * 256.0).round() as i32).clamp(0, raw_top);
    let level = |i: usize| -> i32 {
        let f = fluid[i];
        if f == NO_FLUID { 0 } else { (f as i32 - 1 - min).clamp(0, max - 1 - min) }
    };
    let heights = if d.tall { Heights::Int((0..TILE_AREA).map(height).collect()) } else { Heights::Short((0..TILE_AREA).map(|i| height(i) as u16).collect()) };
    let water = if d.tall { Water::Short((0..TILE_AREA).map(|i| level(i) as u16).collect()) } else { Water::Byte((0..TILE_AREA).map(|i| level(i) as u8).collect()) };
    let terrain: Vec<u8> = match t.terrain.uniform_value() {
        Some(v) => vec![plan.terrains.get(v as usize).copied().unwrap_or(BARE_GRASS); TILE_AREA],
        None => t.terrain.dense().iter().map(|&v| plan.terrains.get(v as usize).copied().unwrap_or(BARE_GRASS)).collect(),
    };

    let mut bytes: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut bits: BTreeMap<usize, Vec<bool>> = BTreeMap::new();
    let mut nibbles: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut bit = |k: usize, i: usize| bits.entry(k).or_insert_with(|| vec![false; TILE_AREA])[i] = true;
    let (ox, oz) = pos.origin();
    if plan.terrain_covers.iter().any(Option::is_some) {
        for (i, &v) in t.terrain.dense().iter().enumerate() {
            if let Some(Some(k)) = plan.terrain_covers.get(v as usize).copied() {
                bit(k, i);
            }
        }
    }

    if let Some(k) = plan.biome
        && t.biome.uniform_value() != Some(0)
    {
        let b = t.biome.dense();
        bytes.push((k, b.iter().map(|&v| if v == 0 { AUTO_BIOME } else { plan.biomes.get(v as usize).copied().unwrap_or(AUTO_BIOME) }).collect()));
    }
    if let Some(k) = plan.annotations
        && t.annotations.uniform_value() != Some(0)
    {
        nibbles.insert(k, t.annotations.dense().iter().map(|&v| v.min(15) as u8).collect());
    }
    if let Some(k) = plan.void {
        for (i, &h) in top.iter().enumerate() {
            if h <= d.dim.min_y as f32 {
                bit(k, i);
            }
        }
    }
    if plan.lava_world
        && let Some(k) = plan.lava
    {
        for (i, &f) in fluid.iter().enumerate() {
            if f != NO_FLUID && f as f32 > top[i] {
                bit(k, i);
            }
        }
    }
    if t.cover.uniform_value() != Some(0) {
        let cover = t.cover.dense();
        let terrain = t.terrain.dense();
        for (i, &n) in cover.iter().enumerate() {
            if let Some(k) = cover_layer(plan, reg, n, ox + (i & 127) as i32, oz + (i >> 7) as i32, top[i], fluid[i], terrain[i]) {
                bit(k, i);
            }
        }
    }
    for plane in [&t.objects, &t.understory] {
        if plane.uniform_value() == Some(0) {
            continue;
        }
        for (i, &n) in plane.dense().iter().enumerate() {
            if let Some(Some(k)) = plan.objects.get(n as usize).copied() {
                nibbles.entry(k).or_insert_with(|| vec![0; TILE_AREA])[i] = 15;
            }
        }
    }
    if t.pockets.uniform_value() != Some(0) {
        for (i, &n) in t.pockets.dense().iter().enumerate() {
            if let Some(Some(k)) = plan.pockets.get(n as usize).copied() {
                nibbles.entry(k).or_insert_with(|| vec![0; TILE_AREA])[i] = 15;
            }
        }
    }
    for (k, values) in nibbles {
        if values.iter().all(|&v| v == 0) {
            continue;
        }
        let mut packed = vec![0u8; TILE_AREA / 2];
        for (i, &v) in values.iter().enumerate() {
            packed[i / 2] |= (v & 0x0F) << ((i % 2) * 4);
        }
        bytes.push((k, packed));
    }
    WpTile { x: pos.x, y: pos.z, heights, terrain, water, bytes, bits: bits.into_iter().collect() }
}

// ---- writing --------------------------------------------------------------------

/// Keys of the objects written once and referred to after.
const KEY_LAYER: u64 = 0x4c00_0000_0000_0000;
const KEY_MATERIAL: u64 = 0x4d00_0000_0000_0000;

fn write_material(w: &mut Writer<'_>, state: &str) -> io::Result<()> {
    let key = KEY_MATERIAL | (hash(&[state.as_bytes()]) >> 8);
    if let Some(h) = w.shared(key) {
        return w.reference(h);
    }
    let (name, props) = split_state(state);
    let h = w.object("org.pepsoft.minecraft.Material", |o| {
        o.int("blockType", -1).int("data", -1).put("identity", move |w| {
            w.object("org.pepsoft.minecraft.Material$Identity", |o| {
                o.string("name", name);
                if !props.is_empty() {
                    o.put("properties", move |w| {
                        w.hash_map(props.len(), move |w| {
                            for (k, v) in &props {
                                w.string(k)?;
                                w.string(v)?;
                            }
                            Ok(())
                        })
                        .map(drop)
                    });
                }
            })
            .map(drop)
        });
    })?;
    w.remember(key, h);
    Ok(())
}

fn write_mixed(w: &mut Writer<'_>, m: &Mixed, kind: &str) -> io::Result<()> {
    let id = uuid(&[b"mixed", kind.as_bytes(), m.name.as_bytes()]);
    let rows = m.rows.clone();
    w.object("org.pepsoft.worldpainter.MixedMaterial", |o| {
        o.int("biome", -1)
            .bool("repeat", m.mode == MixMode::Layered)
            .float("scale", m.scale)
            .put("colour", move |w| w.integer(argb(m.color)).map(drop))
            .put("id", move |w| w.uuid(id).map(drop))
            .enumeration("mode", "org.pepsoft.worldpainter.MixedMaterial$Mode", m.mode.name())
            .string("name", m.name.clone())
            .put("rows", move |w| {
                let items = rows
                    .into_iter()
                    .map(|(state, count)| -> crate::javawrite::Put<'_> {
                        Box::new(move |w: &mut Writer<'_>| {
                            w.object("org.pepsoft.worldpainter.MixedMaterial$Row", |o| {
                                o.int("occurrence", count.min(i32::MAX as u32) as i32).float("scale", 1.0).put("material", move |w| write_material(w, &state));
                            })
                            .map(drop)
                        })
                    })
                    .collect();
                w.object_array("[Lorg.pepsoft.worldpainter.MixedMaterial$Row;", items).map(drop)
            });
    })
    .map(drop)
}

/// The fields of `Layer` and `CustomLayer` of a custom layer.
fn custom_fields(o: &mut crate::javawrite::Obj<'_>, name: &str, description: &str, data_size: &'static str, priority: i32, color: [u8; 3]) {
    let id = format!("{name}.{:x}", hash(&[b"layer", name.as_bytes(), description.as_bytes()]));
    o.set_in(LAYER, "discrete", crate::javawrite::Value::Bool(false))
        .set_in(LAYER, "priority", crate::javawrite::Value::Int(priority))
        .enumeration("dataSize", "org.pepsoft.worldpainter.layers.Layer$DataSize", data_size)
        .string("description", description.to_owned())
        .string("id", id)
        .string("name", name.to_owned())
        .set_in(CUSTOM_LAYER, "biome", crate::javawrite::Value::Int(-1))
        .set_in(CUSTOM_LAYER, "colour", crate::javawrite::Value::Int(argb(color)))
        .set_in(CUSTOM_LAYER, "export", crate::javawrite::Value::Bool(true))
        .set_in(CUSTOM_LAYER, "opacity", crate::javawrite::Value::Float(1.0))
        .set_in(CUSTOM_LAYER, "version", crate::javawrite::Value::Int(4))
        .string("palette", "Massif");
}

/// One of WorldPainter's own layers, written once (by its class).
fn write_layer_spec(w: &mut Writer<'_>, s: &'static LayerSpec) -> io::Result<()> {
    let key = KEY_LAYER | 0x00F0_0000_0000_0000 | (hash(&[s.class.as_bytes()]) >> 16);
    if let Some(h) = w.shared(key) {
        return w.reference(h);
    }
    let h = w.object(s.class, |o| {
        o.bool("discrete", s.discrete)
            .int("priority", s.priority)
            .enumeration("dataSize", "org.pepsoft.worldpainter.layers.Layer$DataSize", s.data_size)
            .string("description", s.description)
            .string("id", s.id)
            .string("name", s.name);
    })?;
    w.remember(key, h);
    Ok(())
}

/// WorldPainter's resources of a dimension, as a new world of
/// WorldPainter's has them (`ResourcesExporterSettings.defaultSettings`):
/// everywhere, at level 8, where Massif lays ores; none where it lays
/// none.
fn write_resources(w: &mut Writer<'_>, kind: DimensionKind, on: bool, platform: &PlatformSpec) -> io::Result<()> {
    // Material, lowest and highest level (None: the world's), chance in
    // the overworld, the Nether and the End.
    #[allow(clippy::type_complexity)]
    const RESOURCES: [(&str, Option<i32>, Option<i32>, [i32; 3]); 14] = [
        ("minecraft:dirt", Some(0), None, [57, 0, 0]),
        ("minecraft:gravel", None, None, [28, 0, 0]),
        ("minecraft:gold_ore", None, Some(31), [1, 3, 0]),
        ("minecraft:iron_ore", None, None, [6, 0, 0]),
        ("minecraft:coal_ore", Some(0), None, [10, 0, 0]),
        ("minecraft:lapis_ore", None, Some(31), [1, 0, 0]),
        ("minecraft:diamond_ore", None, Some(15), [1, 0, 0]),
        ("minecraft:redstone_ore[lit=false]", None, Some(15), [8, 0, 0]),
        ("minecraft:water[level=0]", None, None, [1, 0, 0]),
        ("minecraft:lava[level=0]", None, Some(15), [2, 0, 0]),
        ("minecraft:emerald_ore", Some(64), None, [1, 0, 0]),
        ("minecraft:copper_ore", Some(0), Some(88), [6, 0, 0]),
        ("minecraft:nether_quartz_ore", None, None, [0, 7, 0]),
        ("minecraft:ancient_debris", None, None, [0, 1, 0]),
    ];
    let (min, max) = (platform_int(platform, "minZ"), platform_int(platform, "standardMaxHeight"));
    let column = match kind {
        DimensionKind::Overworld => 0,
        DimensionKind::Nether => 1,
        DimensionKind::End => 2,
    };
    // Copper came with 1.17, ancient debris with 1.16.
    let has = |state: &str| match state {
        "minecraft:copper_ore" => platform.min_heights.iter().any(|&m| m < 0) || platform.id.ends_with("1.17"),
        "minecraft:ancient_debris" => !matches!(platform.id, "org.pepsoft.mcregion" | "org.pepsoft.anvil" | "org.pepsoft.anvil.1.13"),
        _ => true,
    };
    w.object("org.pepsoft.worldpainter.layers.exporters.ResourcesExporter$ResourcesExporterSettings", |o| {
        o.int("minimumLevel", if on && kind == DimensionKind::Overworld { 8 } else { 0 }).int("version", 3).put("settings", move |w| {
            w.hash_map(RESOURCES.len(), move |w| {
                for (state, low, high, chances) in RESOURCES {
                    write_material(w, state)?;
                    let chance = if on && has(state) { chances[column] } else { 0 };
                    w.object("org.pepsoft.worldpainter.layers.exporters.ResourcesExporter$ResourceSettings", |o| {
                        o.int("chance", chance)
                            .int("minLevel", low.unwrap_or(min))
                            .int("maxLevel", high.unwrap_or(max - 1))
                            .long("seedOffset", hash(&[b"resources", state.as_bytes()]) as i64)
                            .put("material", move |w| write_material(w, state));
                    })?;
                }
                Ok(())
            })
            .map(drop)
        });
    })
    .map(drop)
}

fn write_layer(w: &mut Writer<'_>, plan: &Plan, reg: &Registry, k: usize) -> io::Result<()> {
    let key = KEY_LAYER | k as u64;
    if let Some(h) = w.shared(key) {
        return w.reference(h);
    }
    let h = match &plan.layers[k] {
        WpLayer::Own(s) => {
            write_layer_spec(w, s)?;
            let h = w.shared(KEY_LAYER | 0x00F0_0000_0000_0000 | (hash(&[s.class.as_bytes()]) >> 16)).expect("written");
            w.remember(key, h);
            return Ok(());
        }
        WpLayer::Plants { name, color, plants, valid_only } => {
            let plants = plants.clone();
            w.object("org.pepsoft.worldpainter.layers.plants.PlantLayer", |o| {
                custom_fields(o, name, "a plant layer from Massif", "BIT", 35, *color);
                o.bool("generateTilledDirt", true).bool("onlyOnValidBlocks", *valid_only).set_in("org.pepsoft.worldpainter.layers.plants.PlantLayer", "version", crate::javawrite::Value::Int(2)).put("settings", move |w| {
                    let items = (0..PLANT_GROWTH.len())
                        .map(|k| -> crate::javawrite::Put<'_> {
                            match plants.iter().find(|p| p.0 == k).copied() {
                                None => Box::new(|w: &mut Writer<'_>| w.null()),
                                Some((_, occurrence, from, to)) => Box::new(move |w: &mut Writer<'_>| {
                                    w.object("org.pepsoft.worldpainter.layers.plants.PlantLayer$PlantSettings", |o| {
                                        o.short("occurrence", occurrence as i16).byte("dataValueFrom", -1).byte("dataValueTo", -1).int("growthFrom", from).int("growthTo", to);
                                    })
                                    .map(drop)
                                }),
                            }
                        })
                        .collect();
                    w.object_array("[Lorg.pepsoft.worldpainter.layers.plants.PlantLayer$PlantSettings;", items).map(drop)
                });
            })?
        }
        WpLayer::Cover { name, color, material, thickness } => w.object("org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer", |o| {
            let description = if *thickness < 0 { format!("{name} in the top of the terrain") } else { format!("a layer of {name} on top of the terrain {COVER_MARK}") };
            custom_fields(o, name, &description, "BIT", 30, *color);
            o.set_in("org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer", "colour", crate::javawrite::Value::Int(0))
                .int("edgeWidth", 1)
                .int("thickness", *thickness)
                .enumeration("edgeShape", "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer$EdgeShape", "SHEER")
                .enumeration("layerAnchor", "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer$LayerAnchor", "BEDROCK")
                .put("mixedMaterial", move |w| write_mixed(w, material, "cover"));
        })?,
        WpLayer::TallPlant { name, color, material } => w.object("org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer", |o| {
            custom_fields(o, name, &format!("a layer of {name} on top of the terrain {COVER_MARK}"), "BIT", 30, *color);
            o.set_in("org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer", "colour", crate::javawrite::Value::Int(0))
                .int("edgeWidth", 1)
                .int("thickness", 2)
                .enumeration("edgeShape", "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer$EdgeShape", "SHEER")
                .enumeration("layerAnchor", "org.pepsoft.worldpainter.layers.groundcover.GroundCoverLayer$LayerAnchor", "TERRAIN")
                .put("mixedMaterial", move |w| write_mixed(w, material, "tall plant"));
        })?,
        WpLayer::Objects { name, color, objects, density, grid, displacement, rotate, underwater, sink } => {
            let objects = objects.clone();
            let (rotate, underwater, sink, name_owned) = (*rotate, *underwater, *sink, name.clone());
            w.object("org.pepsoft.worldpainter.layers.Bo2Layer", |o| {
                custom_fields(o, name, "objects from Massif", "NIBBLE", 50, *color);
                o.set_in("org.pepsoft.worldpainter.layers.Bo2Layer", "colour", crate::javawrite::Value::Int(0))
                    .int("density", *density)
                    .int("gridX", *grid)
                    .int("gridY", *grid)
                    .int("randomDisplacement", *displacement)
                    .put("files", |w| w.array_list(0, |_| Ok(())).map(drop))
                    .put("objectProvider", move |w| {
                        w.object("org.pepsoft.worldpainter.layers.bo2.Bo2ObjectTube", |o| {
                            o.string("name", name_owned).put("objects", move |w| {
                                w.array_list(objects.len(), move |w| {
                                    for (index, weight) in objects {
                                        let def = &reg.objects[index as usize];
                                        write_object(w, def, weight, rotate, underwater, sink)?;
                                    }
                                    Ok(())
                                })
                                .map(drop)
                            });
                        })
                        .map(drop)
                    });
            })?
        }
        WpLayer::Pockets { name, color, material, frequency, scale, low, high } => w.object("org.pepsoft.worldpainter.layers.pockets.UndergroundPocketsLayer", |o| {
            custom_fields(o, name, &format!("underground pockets of {name}"), "NIBBLE", 15, *color);
            o.int("frequency", *frequency).int("scale", *scale).int("minLevel", *low).int("maxLevel", *high).put("mixedMaterial", move |w| write_mixed(w, material, "pockets"));
        })?,
    };
    w.remember(key, h);
    Ok(())
}

/// One of Massif's objects as WorldPainter's `GenericObject`: the same box
/// (WorldPainter's z is up), standing on its anchor.
fn write_object(w: &mut Writer<'_>, def: &massif_core::objects::ObjectDef, weight: u16, rotate: bool, underwater: bool, sink: i16) -> io::Result<()> {
    let [sx, sy, sz] = def.size.map(|s| s as i32);
    let [ax, ay, az] = def.anchor.map(|a| a as i32);
    w.object("org.pepsoft.worldpainter.objects.GenericObject", |o| {
        o.int("dimX", sx).int("dimY", sz).int("dimZ", sy).string("name", def.name.clone()).put("data", move |w| {
            // Massif's blocks go x, then z, then up: WorldPainter's order.
            let items = def
                .blocks
                .iter()
                .map(|&b| -> crate::javawrite::Put<'_> {
                    match def.palette.get(b as usize).filter(|_| b != 0) {
                        None => Box::new(|w: &mut Writer<'_>| w.null()),
                        Some(state) => Box::new(move |w: &mut Writer<'_>| write_material(w, state)),
                    }
                })
                .collect();
            w.object_array("[Lorg.pepsoft.minecraft.Material;", items).map(drop)
        });
        o.put("attributes", move |w| {
            w.hash_map(6, move |w| {
                w.string("WPObject.offset")?;
                w.object("javax.vecmath.Point3i", |o| {
                    o.int("x", -ax).int("y", -az).int("z", -ay - sink as i32);
                })?;
                w.string("WPObject.randomRotation")?;
                w.boolean(rotate)?;
                w.string("WPObject.frequency")?;
                w.integer(weight.max(1) as i32)?;
                w.string("WPObject.spawnInWater")?;
                w.boolean(underwater)?;
                w.string("WPObject.spawnOnLand")?;
                w.boolean(!underwater)?;
                w.string("WPObject.needsFoundation")?;
                w.boolean(false)?;
                Ok(())
            })
            .map(drop)
        });
    })
    .map(drop)
}

fn write_tile(w: &mut Writer<'_>, t: WpTile, d: &DimPlan, plan: &Plan, reg: &Registry) -> io::Result<()> {
    let WpTile { x, y, heights, terrain, water, bytes, bits } = t;
    for k in bytes.iter().map(|b| b.0).chain(bits.iter().map(|b| b.0)) {
        if let Some(c) = plan.layer_tiles.get(k) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }
    w.object("org.pepsoft.worldpainter.Tile", |o| {
        o.int("x", x).int("y", y).int("minHeight", d.min).int("maxHeight", d.max).bool("tall", d.tall);
        match heights {
            Heights::Int(v) => o.put("tallHeightMap", move |w| w.int_array(&v).map(drop)),
            Heights::Short(v) => o.put("heightMap", move |w| w.short_array(&v).map(drop)),
        };
        match water {
            Water::Short(v) => o.put("tallWaterLevel", move |w| w.short_array(&v).map(drop)),
            Water::Byte(v) => o.put("waterLevel", move |w| w.byte_array(&v).map(drop)),
        };
        o.put("terrain", move |w| w.byte_array(&terrain).map(drop));
        o.put("layerData", move |w| {
            w.hash_map(bytes.len(), move |w| {
                for (k, data) in bytes {
                    write_layer(w, plan, reg, k)?;
                    w.byte_array(&data)?;
                }
                Ok(())
            })
            .map(drop)
        });
        o.put("bitLayerData", move |w| {
            w.hash_map(bits.len(), move |w| {
                for (k, data) in bits {
                    write_layer(w, plan, reg, k)?;
                    w.bit_set(&data)?;
                }
                Ok(())
            })
            .map(drop)
        });
    })
    .map(drop)
}

fn write_anchor(w: &mut Writer<'_>, dim: i32) -> io::Result<()> {
    w.object("org.pepsoft.worldpainter.Dimension$Anchor", |o| {
        o.int("dim", dim).int("id", 0).bool("invert", false).enumeration("role", "org.pepsoft.worldpainter.Dimension$Role", "DETAIL");
    })
    .map(drop)
}

fn write_tile_factory(w: &mut Writer<'_>, d: &DimPlan, plan: &Plan) -> io::Result<()> {
    let (min, max, water, seed) = (d.min, d.max, plan.sea - 1, plan.seed);
    let terrain = crate::TERRAINS[plan.main_terrain as usize];
    w.object("org.pepsoft.worldpainter.HeightMapTileFactory", |o| {
        o.bool("floodWithLava", plan.lava_world)
            .int("maxHeight", max)
            .int("minHeight", min)
            .long("seed", seed)
            .int("version", 1)
            .int("waterHeight", -1)
            .put("tileCache", |w| {
                w.object("java.util.Collections$SynchronizedMap", |o| {
                    o.put("m", |w| w.hash_map(0, |_| Ok(())).map(drop)).put("mutex", |w| {
                        let me = w.current();
                        w.reference(me)
                    });
                })
                .map(drop)
            })
            .put("heightMap", move |w| {
                w.object("org.pepsoft.worldpainter.heightMaps.ConstantHeightMap", |o| {
                    o.double("dHeight", (water + 1) as f64).int("version", 1);
                })
                .map(drop)
            })
            .put("theme", move |w| {
                // As `SimpleTheme.createSingleTerrain`: one terrain at every
                // height.
                w.object("org.pepsoft.worldpainter.themes.SimpleTheme", |o| {
                    o.int("maxHeight", max)
                        .int("minHeight", min)
                        .long("seed", seed)
                        .int("waterHeight", water)
                        .put("terrainRanges", move |w| {
                            w.tree_map(1, move |w| {
                                w.integer(min - 1)?;
                                w.enum_const(TERRAIN, terrain).map(drop)
                            })
                            .map(drop)
                        })
                        .put("terrainRangesTable", move |w| {
                            let items = (min..max).map(|_| -> crate::javawrite::Put<'_> { Box::new(move |w: &mut Writer<'_>| w.enum_const(TERRAIN, terrain).map(drop)) }).collect();
                            w.object_array("[Lorg.pepsoft.worldpainter.Terrain;", items).map(drop)
                        });
                })
                .map(drop)
            });
    })
    .map(drop)
}

fn write_dimension(w: &mut Writer<'_>, world: &World, d: &DimPlan, plan: &Plan, world_handle: i32, progress: Option<&WpProgress>) -> io::Result<()> {
    let reg = &world.registry;
    let kind = d.dim.kind;
    let dim = anchor_dim(kind);
    let (x0, x1) = (d.tiles.iter().map(|p| p.x).min().unwrap_or(0), d.tiles.iter().map(|p| p.x).max().unwrap_or(0));
    let (y0, y1) = (d.tiles.iter().map(|p| p.z).min().unwrap_or(0), d.tiles.iter().map(|p| p.z).max().unwrap_or(0));
    let custom: Vec<usize> = (0..plan.layers.len()).filter(|&k| plan.layers[k].custom()).collect();
    let generator = match kind {
        DimensionKind::Overworld => "DEFAULT",
        DimensionKind::Nether => "NETHER",
        DimensionKind::End => "END",
    };
    let seed = plan.seed;
    let ores = world.meta.ores.on;
    let [sx, _, sz] = world.meta.spawn;
    w.object("org.pepsoft.worldpainter.Dimension", |o| {
        o.bool("biomesConverted", true)
            .int("borderLevel", plan.sea - 1)
            .int("borderSize", 2)
            .int("ceilingHeight", d.max)
            .int("contourSeparation", 10)
            .bool("contoursEnabled", true)
            .bool("coverSteepTerrain", true)
            .int("dim", dim)
            .int("gridSize", 128)
            .int("lowestX", x0)
            .int("highestX", x1)
            .int("lowestY", y0)
            .int("highestY", y1)
            .int("maxHeight", d.max)
            .int("minHeight", d.min)
            .long("minecraftSeed", seed)
            .float("scale", 1.0)
            .long("seed", seed)
            .int("topLayerMinDepth", 3)
            .int("topLayerVariation", 4)
            .int("wpVersion", 10)
            .put("anchor", move |w| write_anchor(w, dim))
            .put("customLayers", move |w| {
                w.array_list(custom.len(), move |w| {
                    for k in custom {
                        write_layer(w, plan, reg, k)?;
                    }
                    Ok(())
                })
                .map(drop)
            })
            .put("generator", move |w| {
                w.object("org.pepsoft.minecraft.SeededGenerator", |o| {
                    o.enumeration("type", "org.pepsoft.worldpainter.Generator", generator).long("seed", seed);
                })
                .map(drop)
            })
            .put("id", move |w| w.uuid(uuid(&[b"dimension", &seed.to_le_bytes(), &dim.to_le_bytes()])).map(drop))
            .put("lastViewPosition", move |w| w.point(sx, sz).map(drop))
            .put("layerSettings", move |w| {
                w.hash_map(1, move |w| {
                    write_layer_spec(w, own_layer("Resources"))?;
                    write_resources(w, kind, ores, plan.platform)
                })
                .map(drop)
            })
            .put("managedAttributes", |w| w.hash_map(0, |_| Ok(())).map(drop))
            .string("name", kind.label())
            .put("overlays", |w| w.array_list(0, |_| Ok(())).map(drop))
            .enumeration("subsurfaceLayerAnchor", "org.pepsoft.worldpainter.Dimension$LayerAnchor", "BEDROCK")
            .enumeration("subsurfaceMaterial", TERRAIN, plan.subsurface)
            .put("tileFactory", move |w| write_tile_factory(w, d, plan))
            .enumeration("topLayerAnchor", "org.pepsoft.worldpainter.Dimension$LayerAnchor", "BEDROCK")
            .reference("world", world_handle);
        if !plan.custom_biomes.is_empty() && kind == DimensionKind::Overworld {
            o.put("customBiomes", move |w| {
                w.array_list(plan.custom_biomes.len(), move |w| {
                    for (name, id, color) in &plan.custom_biomes {
                        w.object("org.pepsoft.worldpainter.biomeschemes.CustomBiome", |o| {
                            o.string("name", name.clone()).int("id", *id as i32).int("colour", argb(*color) & 0xFF_FFFF);
                        })?;
                    }
                    Ok(())
                })
                .map(drop)
            });
        }
        o.put("tiles", move |w| {
            let ground = d.dim.ground();
            w.hash_map(d.tiles.len(), move |w| {
                // Tiles are converted on the pool a batch ahead of the one
                // being written.
                std::thread::scope(|scope| {
                    let (send, receive) = std::sync::mpsc::sync_channel::<(usize, Vec<WpTile>)>(2);
                    let converter = scope.spawn(move || {
                        for batch in d.tiles.chunks(128) {
                            if progress.is_some_and(|p| p.cancel.load(Ordering::Relaxed)) {
                                return;
                            }
                            let converted = batch.par_iter().filter_map(|&pos| ground.view(pos).map(|t| convert_tile(&t, pos, d, plan, reg))).collect();
                            if send.send((batch.len(), converted)).is_err() {
                                return;
                            }
                        }
                    });
                    let mut written = 0;
                    for (count, converted) in receive {
                        for t in converted {
                            w.point(t.x, t.y)?;
                            write_tile(w, t, d, plan, reg)?;
                        }
                        written += count;
                        if let Some(p) = progress {
                            p.tiles.fetch_add(count, Ordering::Relaxed);
                        }
                    }
                    converter.join().map_err(|_| io::Error::other("the conversion failed"))?;
                    if written < d.tiles.len() {
                        return Err(io::Error::other("cancelled"));
                    }
                    Ok(())
                })
            })
            .map(drop)
        });
    })
    .map(drop)
}

fn write_platform(w: &mut Writer<'_>, p: &'static PlatformSpec) -> io::Result<()> {
    w.object("org.pepsoft.worldpainter.Platform", |o| {
        for &(name, value) in p.ints {
            o.int(name, value);
        }
        o.string("id", p.id)
            .string("displayName", p.display_name)
            .put("maxHeights", |w| w.int_array(p.max_heights).map(drop))
            .put("minHeights", |w| w.int_array(p.min_heights).map(drop))
            .put("supportedGameTypes", |w| {
                w.array_list(p.game_types.len(), |w| {
                    for &g in p.game_types {
                        w.enum_const("org.pepsoft.worldpainter.GameType", g)?;
                    }
                    Ok(())
                })
                .map(drop)
            })
            .put("supportedGenerators", |w| {
                w.array_list(p.generators.len(), |w| {
                    for &g in p.generators {
                        w.enum_const("org.pepsoft.worldpainter.Generator", g)?;
                    }
                    Ok(())
                })
                .map(drop)
            })
            .put("supportedDimensions", |w| {
                w.array_list(p.dimensions.len(), |w| {
                    for &d in p.dimensions {
                        w.integer(d)?;
                    }
                    Ok(())
                })
                .map(drop)
            })
            .put("capabilities", |w| {
                w.hash_set(p.capabilities.len(), |w| {
                    for &c in p.capabilities {
                        w.enum_const("org.pepsoft.worldpainter.Platform$Capability", c)?;
                    }
                    Ok(())
                })
                .map(drop)
            });
    })
    .map(drop)
}

fn write_world(w: &mut Writer<'_>, world: &World, plan: &Plan, dims: &[DimPlan], options: &WpOptions, progress: Option<&WpProgress>) -> io::Result<()> {
    let overworld = dims.iter().find(|d| d.dim.kind == DimensionKind::Overworld).unwrap_or(&dims[0]);
    let [sx, _, sz] = world.meta.spawn;
    w.object("org.pepsoft.worldpainter.World2", |o| {
        o.bool("allowCheats", options.cheats)
            .bool("allowMerging", true)
            .int("biomeAlgorithm", -1)
            .int("difficulty", options.difficulty.clamp(0, 3))
            .int("dimensionToExport", -1)
            .bool("dontAskToConvertToAnvil", true)
            .int("gameType", -1)
            .bool("mapFeatures", true)
            .int("maxheight", overworld.max)
            .int("minHeight", overworld.min)
            .int("version", -1)
            .int("wpVersion", 13)
            .put("borderSettings", |w| {
                w.object("org.pepsoft.worldpainter.World2$BorderSettings", |o| {
                    o.int("size", 60_000_000).int("safeZone", 5).int("warningBlocks", 5).int("warningTime", 15).int("sizeLerpTarget", 60_000_000).float("damagePerBlock", 0.2);
                })
                .map(drop)
            })
            .put("dimensionsByAnchor", move |w| {
                let world_handle = w.current();
                w.hash_map(dims.len(), move |w| {
                    for d in dims {
                        write_anchor(w, anchor_dim(d.dim.kind))?;
                        write_dimension(w, world, d, plan, world_handle, progress)?;
                    }
                    Ok(())
                })
                .map(drop)
            })
            .enumeration("gameTypeObj", "org.pepsoft.worldpainter.GameType", options.game_type)
            .put("history", |w| w.array_list(0, |_| Ok(())).map(drop))
            .put("mixedMaterials", move |w| {
                let items = (0..CUSTOM_TERRAINS)
                    .map(|k| -> crate::javawrite::Put<'_> {
                        match plan.customs.get(k) {
                            None => Box::new(|w: &mut Writer<'_>| w.null()),
                            Some(m) => Box::new(move |w: &mut Writer<'_>| write_mixed(w, m, "terrain")),
                        }
                    })
                    .collect();
                w.object_array("[Lorg.pepsoft.worldpainter.MixedMaterial;", items).map(drop)
            })
            .string("name", world.meta.name.clone())
            .put("platform", move |w| write_platform(w, plan.platform))
            .put("spawnPoint", move |w| w.point(sx, sz).map(drop))
            .put("spawnPointDimension", |w| write_anchor(w, 0))
            .enumeration("upIs", "org.pepsoft.minecraft.Direction", "NORTH");
    })
    .map(drop)
}

/// What WorldPainter cannot hold, by dimension.
fn left_out(world: &World, report: &mut WpReport) {
    for dim in &world.dims {
        for level in &dim.levels[1..] {
            if level.is_empty() {
                continue;
            }
            let what = match level.kind {
                LevelKind::Solid => "land over the ground (floating islands, overhangs)",
                LevelKind::Carve => "hollows under the ground (caves, tunnels dug by hand)",
                LevelKind::Ground => "ground",
            };
            report.left_out.push(format!("{}: {} level \"{}\" ({} tiles), {what}: WorldPainter holds one height per column", dim.kind.label(), level.kind.label(), level.name, level.len()));
        }
        if !dim.voxels.is_empty() {
            report.left_out.push(format!("{}: {} sculpted 3D chunks: WorldPainter holds one height per column", dim.kind.label(), dim.voxels.len()));
        }
    }
    if world.meta.graph.is_some() {
        report.left_out.push("the terrain graph: the land it made is in the world, the graph stays in the Massif map".into());
    }
    if !world.meta.ores.on {
        report.approximated.push("ores were off: WorldPainter's Resources lay none".into());
    } else {
        report.approximated.push("ores: WorldPainter's Resources lay its own default ores, dirt, gravel, water and lava pockets everywhere, rather than Massif's ore bands".into());
    }
}

/// Write `world` as a WorldPainter world at `path`.
pub fn export_world(world: &World, path: &Path, options: &WpOptions, progress: Option<&WpProgress>) -> Result<WpReport, WpExportError> {
    if world.dims.is_empty() {
        return Err(WpExportError::Empty);
    }
    let mut report = WpReport::default();
    let mut plan = Plan::new(world, options, &mut report);
    plan.layer_tiles = (0..plan.layers.len()).map(|_| AtomicUsize::new(0)).collect();
    let plan = plan;
    let dims: Vec<DimPlan> = world
        .dims
        .iter()
        .map(|dim| {
            let (min, max) = heights(plan.platform, dim.min_y, dim.max_y);
            let tiles = ground_tiles(dim);
            report.dimensions.push(format!("{} ({} tiles, Y {min} to {})", dim.kind.label(), tiles.len(), max - 1));
            DimPlan { dim, tiles, min, max, tall: max - min > 256 }
        })
        .collect();
    report.tiles = dims.iter().map(|d| d.tiles.len()).sum();
    if let Some(p) = progress {
        p.total.store(report.tiles, Ordering::Relaxed);
    }
    left_out(world, &mut report);

    let part = path.with_extension("world.part");
    let result = (|| -> io::Result<u64> {
        let file = BufWriter::with_capacity(1 << 20, File::create(&part)?);
        let mut gz = GzipWriter::new(file, 6, None)?;
        let mut w = Writer::new(&mut gz);
        write_world(&mut w, world, &plan, &dims, options, progress)?;
        w.finish()?;
        let total = gz.total_out;
        let file = gz.finish()?;
        let file = file.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        Ok(total + 8)
    })();
    match result {
        Ok(bytes) => {
            std::fs::rename(&part, path)?;
            report.bytes = bytes;
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            if progress.is_some_and(|p| p.cancel.load(Ordering::Relaxed)) {
                return Err(WpExportError::Cancelled);
            }
            return Err(e.into());
        }
    }
    report.layers = plan.layers.iter().zip(&plan.layer_tiles).map(|(l, c)| (l, c.load(Ordering::Relaxed))).filter(|(_, c)| *c > 0).map(|(l, c)| (l.name().to_owned(), c)).collect();
    let _ = (plan.frost, plan.lava);
    Ok(report)
}

/// What WorldPainter should find at `count` columns of the ground spread
/// over the map, as Massif sees them: a line a column, `dimension x z
/// height terrain water biome layers` (the layers as `name=value` pairs
/// joined by `;`, `-` for none; the water `dry` where there is none), as
/// `tools/worldpainter/WorldCheck.java` reads them.
pub fn expectations(world: &World, options: &WpOptions, count: usize) -> String {
    let mut quiet = WpReport::default();
    let plan = Plan::new(world, options, &mut quiet);
    let reg = &world.registry;
    let mut out = String::from("# dimension x z height terrain water biome layers\n");
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for dim in &world.dims {
        let (min, max) = heights(plan.platform, dim.min_y, dim.max_y);
        let tiles = ground_tiles(dim);
        if tiles.is_empty() {
            continue;
        }
        let ground = dim.ground();
        for _ in 0..count.div_ceil(world.dims.len()) {
            let pos = tiles[(next() % tiles.len() as u64) as usize];
            let Some(t) = ground.view(pos) else { continue };
            let i = (next() % TILE_AREA as u64) as usize;
            let (ox, oz) = pos.origin();
            let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
            let top = t.top.get(i);
            let height = ((top - 1.0).clamp(min as f32, (max - 1) as f32) * 256.0).round() / 256.0;
            let terrain = crate::TERRAINS[plan.terrains.get(t.terrain.get(i) as usize).copied().unwrap_or(BARE_GRASS) as usize];
            let fluid = t.fluid.get(i);
            let water = if fluid == NO_FLUID || fluid as f32 <= top { "dry".to_owned() } else { (fluid as i32 - 1).clamp(min, max - 1).to_string() };
            let b = t.biome.get(i);
            let biome = if b == 0 { AUTO_BIOME } else { plan.biomes.get(b as usize).copied().unwrap_or(AUTO_BIOME) };
            let mut layers: Vec<String> = Vec::new();
            let mut set = |k: Option<usize>, v: i32| {
                if let Some(k) = k {
                    layers.push(format!("{}={v}", plan.layers[k].name()));
                }
            };
            if top <= dim.min_y as f32 {
                set(plan.void, 1);
            }
            set(plan.terrain_covers.get(t.terrain.get(i) as usize).copied().flatten(), 1);
            if plan.lava_world && fluid != NO_FLUID && fluid as f32 > top {
                set(plan.lava, 1);
            }
            let a = t.annotations.get(i);
            if a != 0 {
                set(plan.annotations, a.min(15) as i32);
            }
            let c = t.cover.get(i);
            set(cover_layer(&plan, reg, c, x, z, top, fluid, t.terrain.get(i)), 1);
            for o in [t.objects.get(i), t.understory.get(i)] {
                set(plan.objects.get(o as usize).copied().flatten(), 15);
            }
            set(plan.pockets.get(t.pockets.get(i) as usize).copied().flatten(), 15);
            layers.sort();
            layers.dedup();
            let layers = if layers.is_empty() { "-".to_owned() } else { layers.join(";") };
            out += &format!("{}\t{x}\t{z}\t{height}\t{terrain}\t{water}\t{biome}\t{layers}\n", anchor_dim(dim.kind));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use massif_core::plane::Plane;

    #[test]
    fn platforms_follow_the_version() {
        assert_eq!(platform_for("1.21.11").id, "org.pepsoft.anvil.1.21.11");
        assert_eq!(platform_for("1.21.5").id, "org.pepsoft.anvil.1.20.5");
        assert_eq!(platform_for("1.20.1").id, "org.pepsoft.anvil.1.19");
        assert_eq!(platform_for("1.18.2").id, "org.pepsoft.anvil.1.18");
        assert_eq!(platform_for("1.16.5").id, "org.pepsoft.anvil.1.13");
        assert_eq!(platform_for("1.12.2").id, "org.pepsoft.anvil");
        assert_eq!(platform_for("26.1.2").id, "org.pepsoft.anvil.26.1");
        assert_eq!(platform_for("26.3").id, "org.pepsoft.anvil.26.1");
        assert_eq!(platform_for("?").id, "org.pepsoft.anvil.26.1");
    }

    #[test]
    fn heights_are_worldpainters_nearest() {
        let p = platform_for("1.21.11");
        assert_eq!(heights(p, -64, 320), (-64, 320));
        assert_eq!(heights(p, -64, 400), (-64, 512));
        assert_eq!(heights(p, -100, 320), (-128, 320));
        assert_eq!(heights(p, -2032, 2032), (-2032, 2032));
        assert_eq!(heights(platform_for("1.12.2"), -64, 320), (0, 256));
    }

    #[test]
    fn plants_are_found_in_worldpainters_catalogue() {
        assert_eq!(plant_index("minecraft:short_grass"), Some(0));
        assert_eq!(plant_index("minecraft:grass"), Some(0));
        assert_eq!(plant_index("minecraft:tall_grass[half=lower]"), Some(1));
        assert_eq!(plant_index("minecraft:wheat[age=7]"), Some(27));
        assert_eq!(plant_index("minecraft:tall_dry_grass"), Some(94));
        assert_eq!(plant_index("conquest:short_grass"), None);
        assert_eq!(plant_index("minecraft:vine"), None);
    }

    /// A small map with everything a column can hold: heights between
    /// blocks, the game's terrains and a mod's, water, a mod's biome,
    /// frost, plants of the game and of a mod, objects, pockets,
    /// annotations and void.
    pub(crate) fn every_kind_of_column() -> World {
        use massif_core::objects::{ObjectDef, ObjectLayerDef};
        use massif_core::pockets::PocketDef;
        use massif_core::registry::{BiomeDef, CoverDef, Stratum, TerrainDef};
        use massif_core::world::WorldMeta;
        let mut reg = Registry::vanilla();
        let grass = reg.terrain_by_name("Grass").unwrap().0;
        let sand = reg.terrain_by_name("Sand").unwrap().0;
        let slate = reg.intern_block("conquest:slate_cobble", [90, 90, 95]);
        let moss = reg.intern_block("conquest:mossy_slate", [80, 100, 80]);
        reg.terrains.push(TerrainDef { name: "Conquest Slate".into(), group: "Mods".into(), color: [90, 90, 95], strata: vec![Stratum { blocks: vec![(slate, 3), (moss, 1)], depth: (3, 4), mix: Mix::Grain }], submerged_top: None, biome_hint: None });
        let conquest = reg.terrains.len() as u16 - 1;
        let plains = reg.biome_by_id("minecraft:plains").unwrap().0;
        let alpine = reg.intern_biome(BiomeDef::of_id("terralith:alpine_grove")).0;
        let frost = crate::layers::frost_cover(&mut reg);
        let meadow = reg.add_cover(CoverDef::new("Meadow", [90, 160, 60], &[("minecraft:short_grass", 3), ("minecraft:poppy", 1)], 0.5));
        let buttercups = reg.add_cover(CoverDef::new("Buttercups", [220, 200, 40], &[("conquest:buttercups", 1)], 0.8));
        let log = "minecraft:oak_log[axis=y]".to_owned();
        let leaves = "minecraft:oak_leaves[distance=1,persistent=true]".to_owned();
        // 3 × 4 × 3: a trunk of two logs under a crown.
        let mut blocks = vec![0u16; 36];
        for y in 0..4 {
            for z in 0..3 {
                for x in 0..3 {
                    let i = x + z * 3 + y * 9;
                    blocks[i] = if x == 1 && z == 1 && y < 2 { 1 } else if y >= 2 { 2 } else { 0 };
                }
            }
        }
        let mut tree = ObjectDef::new("Little oak", [3, 4, 3], vec![String::new(), log, leaves], blocks);
        tree.anchor = [1, 0, 1];
        reg.objects.push(tree);
        let mut trees = ObjectLayerDef::new("Little oaks", &[0], &reg);
        trees.spacing = 6;
        trees.density = 0.5;
        let trees = reg.add_object_layer(trees);
        let iron = reg.intern_block("minecraft:iron_block", [220, 220, 220]);
        let pockets = reg.add_pocket(PocketDef::new("Iron", [200, 200, 200], &[(iron, 1)], 0.05, 6.0, -40, 40));

        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for pos in [TilePos::new(0, 0), TilePos::new(1, 0), TilePos::new(-1, -1)] {
            let mut t = Tile::ground(64.0, grass, NO_FLUID);
            let (ox, oz) = pos.origin();
            let mut top = vec![0f32; TILE_AREA];
            let mut fluid = vec![NO_FLUID; TILE_AREA];
            let mut terrain = vec![grass; TILE_AREA];
            let mut biome = vec![0u16; TILE_AREA];
            let mut cover = vec![0u16; TILE_AREA];
            let mut objects = vec![0u16; TILE_AREA];
            let mut pocket = vec![0u16; TILE_AREA];
            let mut notes = vec![0u16; TILE_AREA];
            for i in 0..TILE_AREA {
                let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
                let h = 66.0 + ((x as f32) * 0.07).sin() * 9.0 + ((z as f32) * 0.05).cos() * 7.0;
                top[i] = massif_core::codec::snap_height(h + ((x * 7 + z * 13) & 255) as f32 / 256.0);
                if top[i] < 64.0 {
                    fluid[i] = 66;
                    terrain[i] = sand;
                } else if x % 17 < 5 {
                    terrain[i] = conquest;
                }
                biome[i] = match (x + z).rem_euclid(4) { 0 => 0, 1 | 2 => plains, _ => alpine };
                cover[i] = match x.rem_euclid(9) { 0 => frost, 1..=3 => meadow, 4 => buttercups, _ => 0 };
                if z.rem_euclid(5) == 0 && top[i] > 66.0 {
                    objects[i] = trees;
                }
                if x.rem_euclid(11) < 3 {
                    pocket[i] = pockets;
                }
                notes[i] = ((x / 8).rem_euclid(16)) as u16;
                if x.rem_euclid(97) == 0 && z.rem_euclid(13) == 0 {
                    top[i] = -64.0;
                }
            }
            t.top = Plane::from_vec(top);
            t.fluid = Plane::from_vec(fluid);
            t.terrain = Plane::from_vec(terrain);
            t.biome = Plane::from_vec(biome);
            t.cover = Plane::from_vec(cover);
            t.objects = Plane::from_vec(objects);
            t.pockets = Plane::from_vec(pocket);
            t.annotations = Plane::from_vec(notes);
            dim.ground_mut().insert(pos, t);
        }
        World { meta: WorldMeta { name: "Every column".into(), seed: 1234, spawn: [10, 80, 20], sea_level: 66, ..Default::default() }, registry: reg, dims: vec![dim], points: Vec::new() }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("massif-wp-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_map_comes_back_from_worldpainters_format() {
        let world = every_kind_of_column();
        let path = temp("every.world");
        let report = export_world(&world, &path, &WpOptions::default(), None).unwrap();
        assert_eq!(report.tiles, 3);
        assert_eq!(report.custom_terrains, ["Conquest Slate"]);
        assert_eq!(report.custom_biomes, ["terralith:alpine_grove"]);
        // The same map, the same file.
        let again = temp("again.world");
        export_world(&world, &again, &WpOptions::default(), None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&again).unwrap());

        let (back, imported) = crate::import_world(&path, None).unwrap();
        assert!(imported.layers.is_empty(), "layers not imported: {:?}", imported.layers);
        assert_eq!(back.meta.name, "Every column");
        assert_eq!(back.meta.seed, 1234);
        assert_eq!(back.meta.sea_level, 66);
        assert_eq!((back.meta.spawn[0], back.meta.spawn[2]), (10, 20));
        let (reg, breg) = (&world.registry, &back.registry);
        let (dim, bdim) = (&world.dims[0], &back.dims[0]);
        assert_eq!((bdim.min_y, bdim.max_y), (-64, 320));
        let plan = Plan::new(&world, &WpOptions::default(), &mut WpReport::default());
        let mut checked = [0usize; 7];
        for pos in ground_tiles(dim) {
            let (t, b) = (dim.ground().view(pos).unwrap(), bdim.ground().view(pos).expect("the tile came back"));
            let (ox, oz) = pos.origin();
            for i in 0..TILE_AREA {
                let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
                let top = t.top.get(i);
                if top <= -64.0 {
                    // Void: nothing at all in the column.
                    assert!(b.top.get(i) <= -64.0, "void at {x},{z}");
                    continue;
                }
                assert_eq!(b.top.get(i), top, "height at {x},{z}");
                let name = |r: &Registry, id: u16| r.terrain(TerrainId(id)).name.clone();
                assert_eq!(name(breg, b.terrain.get(i)), name(reg, t.terrain.get(i)), "terrain at {x},{z}");
                let (f, bf) = (t.fluid.get(i), b.fluid.get(i));
                if f != NO_FLUID && f as f32 > top {
                    assert_eq!(bf, f, "water at {x},{z}");
                    checked[0] += 1;
                } else {
                    assert!(bf == NO_FLUID || bf as f32 <= top, "dry at {x},{z}");
                }
                let biome = |r: &Registry, id: u16| if id == 0 { String::new() } else { r.biomes[id as usize].id.clone() };
                assert_eq!(biome(breg, b.biome.get(i)), biome(reg, t.biome.get(i)), "biome at {x},{z}");
                // A cover comes back where it grows something: where its
                // plant comes up on soil out of the water.
                let c = t.cover.get(i);
                let grows = cover_layer(&plan, reg, c, x, z, top, t.fluid.get(i), t.terrain.get(i)).is_some();
                if reg.cover(c).is_some_and(|d| d.soil_only && d.growth(x, z, 1234).is_some()) && !grows {
                    checked[6] += 1;
                }
                let bc = b.cover.get(i);
                if grows {
                    let (d, bd) = (reg.cover(c).unwrap(), breg.cover(bc).unwrap_or_else(|| panic!("cover at {x},{z}")));
                    assert_eq!((bd.frost, &bd.plants), (d.frost, &d.plants), "cover at {x},{z}");
                    checked[1] += 1;
                } else {
                    assert_eq!(bc, 0, "no cover at {x},{z}");
                }
                assert_eq!(b.objects.get(i) != 0, t.objects.get(i) != 0, "objects at {x},{z}");
                if t.objects.get(i) != 0 {
                    let layer = breg.object_layer(b.objects.get(i)).unwrap();
                    let object = &breg.objects[layer.objects[0].0 as usize];
                    let original = &reg.objects[0];
                    assert_eq!((object.size, object.anchor, object.blocks.iter().filter(|&&v| v != 0).count()), (original.size, original.anchor, original.blocks.iter().filter(|&&v| v != 0).count()));
                    checked[2] += 1;
                }
                assert_eq!(b.pockets.get(i) != 0, t.pockets.get(i) != 0, "pockets at {x},{z}");
                if t.pockets.get(i) != 0 {
                    let p = breg.pocket(b.pockets.get(i)).unwrap();
                    assert_eq!((p.low, p.high, breg.block(p.blocks[0].0).state.as_str()), (-40, 40, "minecraft:iron_block"));
                    checked[3] += 1;
                }
                assert_eq!(b.annotations.get(i), t.annotations.get(i), "annotation at {x},{z}");
                checked[4] += (reg.terrain(TerrainId(t.terrain.get(i))).name == "Conquest Slate") as usize;
                checked[5] += 1;
            }
        }
        assert!(checked.iter().all(|&c| c > 0), "{checked:?}");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&again);
    }

    /// The real proof: WorldPainter's own code opens the file, finds in
    /// every column checked what Massif has there, saves it again and
    /// exports it as a Minecraft map. Needs WorldPainter's classes (see
    /// `tools/worldpainter/check.sh prepare`); run with
    /// `MASSIF_WP_CHECK=1 cargo test -p massif-legacy -- worldpainter_opens_it`.
    #[test]
    fn worldpainter_opens_it() {
        if std::env::var_os("MASSIF_WP_CHECK").is_none() {
            return;
        }
        let world = every_kind_of_column();
        let path = temp("checked.world");
        export_world(&world, &path, &WpOptions::default(), None).unwrap();
        let columns = temp("checked.tsv");
        std::fs::write(&columns, expectations(&world, &WpOptions::default(), 3000)).unwrap();
        let resaved = temp("resaved.world");
        let minecraft = temp("minecraft");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/worldpainter/check.sh");
        let out = std::process::Command::new(script).arg("world").arg(&path).arg("--expect").arg(&columns).arg("--save").arg(&resaved).arg("--export").arg(&minecraft).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        println!("{text}{}", String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "WorldPainter found differences");
        assert!(text.contains("checked 3000 columns: 0 different"));
        // What WorldPainter saved, Massif reads as it wrote it.
        let (back, _) = crate::import_world(&resaved, None).unwrap();
        let (t, b) = (world.dims[0].ground().view(TilePos::new(1, 0)).unwrap(), back.dims[0].ground().view(TilePos::new(1, 0)).unwrap());
        assert!((0..TILE_AREA).all(|i| t.top.get(i) <= -64.0 || b.top.get(i) == t.top.get(i)));
    }

    #[test]
    fn uuids_are_version_4() {
        let u = uuid(&[b"x"]);
        assert_eq!((u >> 76) & 0xF, 4);
        assert_eq!((u >> 62) & 0x3, 2);
    }
}
