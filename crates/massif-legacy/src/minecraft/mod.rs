//! Importing a Minecraft Java world (its `level.dat` and region files) as a
//! Massif world, as WorldPainter's Import Minecraft map.
//!
//! Each column becomes a height (the top of its ground), a terrain (the
//! block on top of the ground), water or lava up to its level, a biome
//! (sampled at the surface) and, where a plant, snow or a carpet grows on
//! the ground, a ground cover of that block. What stands above the ground
//! — buildings, trees, overhangs — is kept block for block in sculpted
//! chunks, made object layers (trees), or dropped, as asked; caves can be
//! kept as sculpted chunks too.
//!
//! Regions are read one at a time on each worker, their tiles encoded the
//! moment they are done, so memory stays flat however large the world.

pub mod chunk;
pub mod classify;
pub mod region;

use crate::worldpainter::{ImportError, ImportProgress, ImportReport};
use chunk::{Blocks, ChunkError, Decoded};
use classify::{Class, classify};
use massif_core::level::Blob;
use massif_core::plane::Plane;
use massif_core::registry::{BiomeDef, BlockId, Mix, Registry, Stratum, TerrainDef};
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::voxel::{ChunkPos, Fluid, VoxelChunk};
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};
use massif_core::{TILE_AREA, TerrainId, TilePos};
use rayon::prelude::*;
pub use region::RegionFile;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What becomes of what stands above the ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Above {
    /// Kept block for block as sculpted chunks: buildings, trees and
    /// overhangs come back unchanged on export.
    #[default]
    Keep,
    /// Trees become object layers (of the world's own trees, by kind),
    /// the rest is dropped.
    Trees,
    /// Dropped: the bare land.
    Drop,
}

/// What to import, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub dimension: DimensionKind,
    /// The chunks to import, `[x0, z0, x1, z1]` inclusive; `None`: all.
    pub area: Option<[i32; 4]>,
    /// The heights kept: blocks from `min_y` to `max_y - 1`.
    pub min_y: i32,
    pub max_y: i32,
    pub above: Above,
    /// Plants, snow and carpets on the ground as ground covers.
    pub covers: bool,
    /// Ground of several blocks mixed together as one terrain mixing them.
    pub mixes: bool,
    pub biomes: bool,
    /// Caves and what lies under the ground kept as sculpted chunks.
    pub caves: bool,
}

impl Options {
    pub fn new(world: &McWorld, dimension: DimensionKind) -> Self {
        let (min_y, max_y) = world.heights(dimension);
        Options { dimension, area: None, min_y, max_y, above: Above::Keep, covers: true, mixes: false, biomes: true, caves: false }
    }
}

/// A world's dimension: its region files.
#[derive(Debug, Clone)]
pub struct McDimension {
    pub kind: DimensionKind,
    pub regions: Vec<RegionFile>,
}

/// A Minecraft world as found on disk, before it is imported.
#[derive(Debug, Clone, Default)]
pub struct McWorld {
    pub root: PathBuf,
    pub name: String,
    /// The game's version that last saved it (`1.21.11`).
    pub version: Option<String>,
    pub data_version: i32,
    pub seed: i64,
    pub spawn: Option<[i32; 3]>,
    pub dimensions: Vec<McDimension>,
}

impl McWorld {
    /// The heights of a dimension of this world.
    pub fn heights(&self, kind: DimensionKind) -> (i32, i32) {
        match kind {
            DimensionKind::Overworld if self.data_version >= 2825 || self.data_version == 0 => (-64, 320),
            _ => (0, 256),
        }
    }

    pub fn dimension(&self, kind: DimensionKind) -> Option<&McDimension> {
        self.dimensions.iter().find(|d| d.kind == kind)
    }
}

/// The level.dat of a world, read.
fn read_level(path: &Path) -> Option<fastnbt::Value> {
    use std::io::Read;
    let raw = std::fs::read(path).ok()?;
    let mut out = Vec::new();
    if flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut out).is_err() {
        out = raw;
    }
    fastnbt::from_bytes(&out).ok()
}

fn field<'a>(v: &'a fastnbt::Value, path: &[&str]) -> Option<&'a fastnbt::Value> {
    let mut v = v;
    for k in path {
        match v {
            fastnbt::Value::Compound(c) => v = c.get(*k)?,
            _ => return None,
        }
    }
    Some(v)
}

fn as_i64(v: &fastnbt::Value) -> Option<i64> {
    Some(match v {
        fastnbt::Value::Byte(b) => *b as i64,
        fastnbt::Value::Short(s) => *s as i64,
        fastnbt::Value::Int(i) => *i as i64,
        fastnbt::Value::Long(l) => *l,
        _ => return None,
    })
}

/// Find a world from what the user picked: its folder, its `level.dat`, a
/// `region` folder, or region files themselves.
pub fn scan(picked: &[PathBuf]) -> Result<McWorld, ImportError> {
    let first = picked.first().ok_or_else(|| ImportError::Unsupported("nothing was picked".into()))?;
    // Region files picked one by one: a world of those alone.
    if picked.iter().all(|p| region::region_coords(p).is_some()) {
        let mut regions: Vec<RegionFile> = picked.iter().filter_map(|p| region::region_coords(p).map(|(x, z)| RegionFile { x, z, path: p.clone() })).collect();
        regions.sort_by_key(|r| (r.z, r.x));
        let dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut world = world_at(dir.parent().filter(|w| w.join("level.dat").is_file()).unwrap_or(&dir));
        world.dimensions = vec![McDimension { kind: DimensionKind::Overworld, regions }];
        return Ok(world);
    }
    let mut root = first.clone();
    if root.is_file() {
        root = root.parent().map(Path::to_path_buf).unwrap_or_default();
    }
    if root.file_name().is_some_and(|n| n == "region") && !root.join("level.dat").exists() {
        root = root.parent().map(Path::to_path_buf).unwrap_or(root);
    }
    let mut world = world_at(&root);
    for (kind, dirs) in [
        (DimensionKind::Overworld, ["region", "dimensions/minecraft/overworld/region"]),
        (DimensionKind::Nether, ["DIM-1/region", "dimensions/minecraft/the_nether/region"]),
        (DimensionKind::End, ["DIM1/region", "dimensions/minecraft/the_end/region"]),
    ] {
        let regions: Vec<RegionFile> = dirs.iter().flat_map(|d| region::region_files(&root.join(d))).collect();
        if !regions.is_empty() {
            world.dimensions.push(McDimension { kind, regions });
        }
    }
    if world.dimensions.is_empty() {
        return Err(ImportError::Unsupported("no region files (.mca) were found in this folder: pick a Minecraft world's folder, its level.dat or its region files".into()));
    }
    Ok(world)
}

fn world_at(root: &Path) -> McWorld {
    let mut world = McWorld { root: root.to_path_buf(), name: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Minecraft world".into()), ..Default::default() };
    if let Some(level) = read_level(&root.join("level.dat")) {
        let data = |p: &[&str]| {
            let mut path = vec!["Data"];
            path.extend_from_slice(p);
            field(&level, &path).cloned()
        };
        if let Some(fastnbt::Value::String(n)) = data(&["LevelName"])
            && !n.trim().is_empty()
        {
            world.name = n.trim().to_owned();
        }
        if let Some(fastnbt::Value::String(v)) = data(&["Version", "Name"]) {
            world.version = Some(v);
        }
        world.data_version = data(&["DataVersion"]).as_ref().and_then(as_i64).unwrap_or(0) as i32;
        world.seed = data(&["WorldGenSettings", "seed"]).or_else(|| data(&["RandomSeed"])).as_ref().and_then(as_i64).unwrap_or(0);
        let c = |k: &str| data(&[k]).as_ref().and_then(as_i64).map(|v| v as i32);
        if let (Some(x), Some(y), Some(z)) = (c("SpawnX"), c("SpawnY"), c("SpawnZ")) {
            world.spawn = Some([x, y, z]);
        } else if let Some(fastnbt::Value::IntArray(p)) = data(&["spawn", "pos"])
            && p.len() == 3
        {
            world.spawn = Some([p[0], p[1], p[2]]);
        }
    }
    world
}

/// The chunks each region file holds (by `z * 32 + x`), read from their
/// tables alone: what the dialog's map shows.
pub fn presence(regions: &[RegionFile]) -> Vec<(i32, i32, Box<[bool; 1024]>)> {
    regions.par_iter().filter_map(|r| region::presence(&r.path).ok().map(|p| (r.x, r.z, p))).collect()
}

// ---- the registry, shared by the workers ---------------------------------

#[derive(Default)]
struct Species {
    name: String,
    objects: Vec<u32>,
    trees: u64,
    columns: u64,
}

struct Maps {
    reg: Registry,
    blocks: HashMap<String, BlockId>,
    /// The terrain of the registry whose top is a block, by state, then by
    /// name.
    top_state: HashMap<String, TerrainId>,
    top_name: HashMap<String, TerrainId>,
    /// The mixed terrains of the registry: the blocks of their top.
    top_sets: Vec<(TerrainId, BTreeSet<String>)>,
    ground: HashMap<String, TerrainId>,
    voxel: HashMap<String, TerrainId>,
    mixes: HashMap<Vec<String>, TerrainId>,
    biomes: HashMap<String, u16>,
    covers: HashMap<(String, u8, bool), u16>,
    species: Vec<Species>,
    species_index: HashMap<String, usize>,
    /// The terrains made for the import, and what lies under their top.
    created: Vec<TerrainId>,
    under: HashMap<TerrainId, HashMap<String, u64>>,
    columns: HashMap<TerrainId, u64>,
    dropped: HashMap<String, u64>,
    voxel_blocks: u64,
    cover_stats: HashMap<u16, (BTreeMap<String, u64>, u8, u8)>,
}

/// A block's colour on the map and in the 3D view: by its name (woods,
/// stairs, stones…), else from the table of grounds.
fn color(state: &str) -> [u8; 3] {
    match massif_core::objects::guess_color(state) {
        [125, 125, 125] => crate::terrains::block_color(state),
        c => c,
    }
}

fn base(state: &str) -> &str {
    state.split('[').next().unwrap_or(state)
}

impl Maps {
    fn new(reg: Registry) -> Self {
        let mut top_state = HashMap::new();
        let mut top_name = HashMap::new();
        let mut top_sets = Vec::new();
        for (i, t) in reg.terrains.iter().enumerate() {
            let Some(first) = t.strata.first() else { continue };
            let id = TerrainId(i as u16);
            let states: BTreeSet<String> = first.blocks.iter().map(|b| massif_core::objects::canonical(&reg.block(b.0).state)).collect();
            if states.len() == 1 {
                let s = states.iter().next().expect("one").clone();
                top_name.entry(base(&s).to_owned()).or_insert(id);
                top_state.entry(s).or_insert(id);
            } else if states.len() > 1 {
                top_sets.push((id, states));
            }
        }
        Maps {
            reg,
            blocks: HashMap::new(),
            top_state,
            top_name,
            top_sets,
            ground: HashMap::new(),
            voxel: HashMap::new(),
            mixes: HashMap::new(),
            biomes: HashMap::new(),
            covers: HashMap::new(),
            species: Vec::new(),
            species_index: HashMap::new(),
            created: Vec::new(),
            under: HashMap::new(),
            columns: HashMap::new(),
            dropped: HashMap::new(),
            voxel_blocks: 0,
            cover_stats: HashMap::new(),
        }
    }

    fn block(&mut self, state: &str) -> BlockId {
        if let Some(b) = self.blocks.get(state) {
            return *b;
        }
        let b = self.reg.intern_block(state, color(state));
        self.blocks.insert(state.to_owned(), b);
        b
    }

    fn add_terrain(&mut self, def: TerrainDef) -> TerrainId {
        self.reg.terrains.push(def);
        TerrainId((self.reg.terrains.len() - 1) as u16)
    }

    /// The terrain of ground whose top is `state`.
    fn ground(&mut self, state: &str) -> TerrainId {
        if let Some(t) = self.ground.get(state) {
            return *t;
        }
        let t = match self.top_state.get(state).or_else(|| self.top_name.get(base(state))) {
            Some(t) => *t,
            None => {
                let b = self.block(state);
                let t = self.add_terrain(TerrainDef {
                    name: classify::pretty(state),
                    group: "Imported".into(),
                    color: color(state),
                    strata: vec![Stratum { blocks: vec![(b, 1)], depth: (1, 1), mix: Mix::Grain }],
                    submerged_top: None,
                    biome_hint: None,
                });
                self.created.push(t);
                t
            }
        };
        self.ground.insert(state.to_owned(), t);
        t
    }

    /// A terrain of one block, for the blocks of sculpted chunks: the same
    /// block however deep.
    fn voxel(&mut self, state: &str) -> TerrainId {
        if let Some(t) = self.voxel.get(state) {
            return *t;
        }
        let b = self.block(state);
        let t = self.add_terrain(TerrainDef {
            name: format!("{} ·", classify::pretty(state)),
            group: BLOCKS_GROUP.into(),
            color: color(state),
            strata: vec![Stratum { blocks: vec![(b, 1)], depth: (4096, 4096), mix: Mix::Grain }],
            submerged_top: None,
            biome_hint: None,
        });
        self.voxel.insert(state.to_owned(), t);
        t
    }

    /// The terrain mixing the blocks of `set` (with their counts).
    fn mix(&mut self, set: &[(String, u32)]) -> TerrainId {
        let key: Vec<String> = set.iter().map(|s| s.0.clone()).collect();
        if let Some(t) = self.mixes.get(&key) {
            return *t;
        }
        let wanted: BTreeSet<String> = key.iter().cloned().collect();
        let found = self.top_sets.iter().filter(|(_, s)| s.is_superset(&wanted)).min_by_key(|(_, s)| s.len()).map(|(t, _)| *t);
        let t = match found {
            Some(t) => t,
            None => {
                let total: u32 = set.iter().map(|s| s.1).sum::<u32>().max(1);
                let blocks: Vec<(BlockId, u16)> = set.iter().map(|(s, n)| (self.block(s), ((n * 100 / total) as u16).max(1))).collect();
                let names: Vec<String> = set.iter().map(|s| classify::pretty(base(&s.0))).collect();
                let mut mixed = [0u32; 3];
                for (s, n) in set {
                    let c = color(s);
                    for k in 0..3 {
                        mixed[k] += c[k] as u32 * n;
                    }
                }
                let t = self.add_terrain(TerrainDef {
                    name: format!("Mix of {}", names.join(", ")),
                    group: "Imported".into(),
                    color: [(mixed[0] / total) as u8, (mixed[1] / total) as u8, (mixed[2] / total) as u8],
                    strata: vec![Stratum { blocks, depth: (1, 1), mix: Mix::Grain }],
                    submerged_top: None,
                    biome_hint: None,
                });
                self.created.push(t);
                t
            }
        };
        self.mixes.insert(key, t);
        t
    }

    fn biome(&mut self, id: &str) -> u16 {
        if let Some(b) = self.biomes.get(id) {
            return *b;
        }
        let b = self.reg.intern_biome(BiomeDef::of_id(id)).0;
        self.biomes.insert(id.to_owned(), b);
        b
    }

    /// The cover of a plant (one per plant, its states and heights
    /// gathered: see [`Maps::finish_covers`]), for `columns` columns.
    fn cover(&mut self, state: &str, height: u8, underwater: bool, columns: u64) -> u16 {
        if state == "minecraft:snow[layers=1]" {
            return self.special_cover("Frost");
        }
        let key = (base(state).to_owned(), 0, underwater);
        let n = match self.covers.get(&key) {
            Some(c) => *c,
            None => {
                let color = massif_core::registry::plant_color(state).unwrap_or_else(|| massif_core::objects::guess_color(state));
                let mut def = massif_core::registry::CoverDef::new(&classify::pretty(base(state)), color, &[(state, 1)], 1.0);
                def.soil_only = false;
                def.underwater = underwater;
                def.palette = "Imported".into();
                let n = self.reg.add_cover(def);
                self.covers.insert(key, n);
                n
            }
        };
        let stats = self.cover_stats.entry(n).or_insert_with(|| (BTreeMap::new(), u8::MAX, 0));
        *stats.0.entry(state.to_owned()).or_default() += columns;
        stats.1 = stats.1.min(height.max(1));
        stats.2 = stats.2.max(height.max(1));
        n
    }

    /// Each cover made grows its plant's states by how often each was
    /// found, as high as the plant was found.
    fn finish_covers(&mut self) {
        for (n, (states, lo, hi)) in std::mem::take(&mut self.cover_stats) {
            let most = states.values().copied().max().unwrap_or(1).max(1);
            let Some(def) = self.reg.covers.get_mut(n as usize - 1) else { continue };
            def.plants = states.into_iter().map(|(s, c)| (s, ((c * 1000).div_ceil(most)).clamp(1, 1000) as u16)).collect();
            def.height = [lo.min(hi), hi];
        }
    }

    fn special_cover(&mut self, name: &str) -> u16 {
        self.reg.cover_by_name(name).unwrap_or_else(|| {
            let def = if name == "Lava" { massif_core::registry::CoverDef::lava() } else { massif_core::registry::CoverDef::frost() };
            self.reg.add_cover(def)
        })
    }

    fn species(&mut self, name: &str) -> usize {
        if let Some(s) = self.species_index.get(name) {
            return *s;
        }
        self.species.push(Species { name: name.to_owned(), ..Default::default() });
        self.species_index.insert(name.to_owned(), self.species.len() - 1);
        self.species.len() - 1
    }
}

/// The kind of tree a log or leaves block belongs to (`oak`,
/// `conquest:pine`).
fn tree_kind(state: &str) -> String {
    let name = base(state);
    let (ns, s) = name.split_once(':').unwrap_or(("minecraft", name));
    let s = s.strip_prefix("stripped_").unwrap_or(s);
    let kind = ["_leaves", "_log", "_wood", "_stem", "_hyphae"].iter().find_map(|suffix| s.strip_suffix(suffix)).unwrap_or(s);
    let kind = kind.strip_prefix("flowering_").unwrap_or(kind);
    let kind = match kind {
        "mushroom" | "red_mushroom_block" | "brown_mushroom_block" => "mushroom",
        k => k,
    };
    if ns == "minecraft" { kind.to_owned() } else { format!("{ns}:{kind}") }
}

// ---- one region ------------------------------------------------------------

/// A tile being filled.
struct TileBuf {
    top: Vec<f32>,
    terrain: Vec<u16>,
    fluid: Vec<i16>,
    biome: Vec<u16>,
    cover: Vec<u16>,
    objects: Vec<u16>,
}

impl TileBuf {
    fn new(min_y: i32) -> Self {
        TileBuf { top: vec![min_y as f32; TILE_AREA], terrain: vec![0; TILE_AREA], fluid: vec![NO_FLUID; TILE_AREA], biome: vec![0; TILE_AREA], cover: vec![0; TILE_AREA], objects: vec![0; TILE_AREA] }
    }

    fn encode(self) -> Blob {
        let mut tile = Tile::ground(0.0, 0, NO_FLUID);
        tile.top = Plane::from_vec(self.top);
        tile.terrain = Plane::from_vec(self.terrain);
        tile.fluid = Plane::from_vec(self.fluid);
        tile.biome = Plane::from_vec(self.biome);
        tile.cover = Plane::from_vec(self.cover);
        tile.objects = Plane::from_vec(self.objects);
        tile.compact();
        Blob::from(tile.encode())
    }
}

#[derive(Default)]
struct RegionOut {
    tiles: Vec<(TilePos, Blob)>,
    voxels: Vec<(ChunkPos, VoxelChunk)>,
    chunks: usize,
    unfinished: usize,
    damaged: usize,
    /// Chunks whose blocks above the ground were left out, past
    /// [`MAX_SCULPTED`].
    over: usize,
    versions: BTreeMap<i32, usize>,
}

struct Ctx<'a> {
    opts: &'a Options,
    maps: &'a Mutex<Maps>,
    progress: &'a ImportProgress,
    /// Samples of trees wanted per kind, claimed as they are cut.
    samples: &'a Mutex<HashMap<String, usize>>,
    regions_done: &'a AtomicUsize,
    sculpted: &'a AtomicUsize,
}

/// The group of the terrains made for the blocks of sculpted chunks.
pub const BLOCKS_GROUP: &str = "Imported blocks";

/// The most sculpted chunks an import makes: a map's sculpted chunks are
/// saved in one section of at most 1 GiB (about 87,000 of them).
pub const MAX_SCULPTED: usize = 80_000;

/// The most sample trees cut of each kind.
const SAMPLES: usize = 12;

/// What one column of a chunk holds, as the import sees it.
#[derive(Clone, Copy, Default)]
struct Column {
    /// The ground's top block (`y`, local state), if any.
    ground: Option<(i32, u16)>,
    /// One above the top of the water or lava resting on the ground.
    fluid: Option<i32>,
    lava: bool,
    frozen: bool,
    /// The block under the ground's top.
    under: Option<u16>,
    /// The plant growing there: its state, height, under water.
    cover: Option<(u16, u8, bool)>,
    /// The highest block of the column kept above the ground, or of the
    /// fluid run (for the biome).
    surface: i32,
}

/// A chunk's blocks at hand by height.
struct Access<'a> {
    by_section: Vec<Option<&'a Blocks>>,
    first: i32,
}

impl<'a> Access<'a> {
    fn new(d: &'a Decoded, min_y: i32, max_y: i32) -> Self {
        let first = min_y.div_euclid(16);
        let last = (max_y - 1).div_euclid(16);
        let by_section = (first..=last).map(|sy| d.section(sy)).collect();
        Access { by_section, first }
    }

    #[inline]
    fn at(&self, x: usize, y: i32, z: usize) -> u16 {
        match self.by_section.get((y.div_euclid(16) - self.first) as usize).copied().flatten() {
            Some(b) => b.get(((y.rem_euclid(16) as usize) * 16 + z) * 16 + x),
            None => u16::MAX,
        }
    }
}

fn region_work(file: &RegionFile, ctx: &Ctx) -> RegionOut {
    let mut out = RegionOut::default();
    let Ok(region) = region::Region::open(file) else {
        out.damaged += 1;
        return out;
    };
    let opts = ctx.opts;
    let (min_y, max_y) = (opts.min_y, opts.max_y);
    let mut tiles: HashMap<TilePos, TileBuf> = HashMap::new();
    // Trees cut out of the region: their blocks, and where trunks stand.
    let mut tree_blocks: HashMap<[i32; 3], u32> = HashMap::new();
    let mut tree_states: Vec<String> = Vec::new();
    let mut tree_state_index: HashMap<String, u32> = HashMap::new();
    let mut trunks: Vec<([i32; 3], String)> = Vec::new();
    let mut tree_columns: Vec<(TilePos, usize, String)> = Vec::new();
    let legacy = |id: u16, d: u8| massif_objects::legacy::state(id, d);
    let biome_name = |n: i32| massif_anvil::version::biome_of_number(n).map_or_else(|| format!("minecraft:unknown_{n}"), |s| format!("minecraft:{s}"));

    for cz in 0..32usize {
        for cx in 0..32usize {
            if !region.has(cx, cz) {
                continue;
            }
            let (wx, wz) = (file.x * 32 + cx as i32, file.z * 32 + cz as i32);
            if let Some([x0, z0, x1, z1]) = opts.area
                && (wx < x0 || wx > x1 || wz < z0 || wz > z1)
            {
                continue;
            }
            if ctx.progress.cancel.load(Ordering::Relaxed) {
                return out;
            }
            ctx.progress.read.fetch_add(1, Ordering::Relaxed);
            let nbt = match region.chunk(cx, cz) {
                Ok(Some(n)) => n,
                Ok(None) => continue,
                Err(_) => {
                    out.damaged += 1;
                    continue;
                }
            };
            let d = match chunk::decode(&nbt, &legacy, &biome_name) {
                Ok(d) => d,
                Err(ChunkError::Unfinished) => {
                    out.unfinished += 1;
                    continue;
                }
                Err(ChunkError::Damaged(_)) => {
                    out.damaged += 1;
                    continue;
                }
            };
            drop(nbt);
            out.chunks += 1;
            *out.versions.entry(d.data_version).or_default() += 1;
            let (wx, wz) = (d.x, d.z);
            let classes: Vec<Class> = d.states.iter().map(|s| classify(s)).collect();
            let class = |i: u16| if i == u16::MAX { Class::Air } else { classes[i as usize] };
            let state = |i: u16| d.states[i as usize].as_str();
            let access = Access::new(&d, min_y, max_y);
            let is_ice = |i: u16| i != u16::MAX && base(state(i)) == "minecraft:ice";
            let wet = |i: u16| i != u16::MAX && (classes[i as usize] == Class::Water || classify::waterlogged(state(i)) || classify::aquatic(state(i)));

            // ---- the columns ----
            let mut columns = [Column::default(); 256];
            // What is kept above the ground of each column (Keep): its
            // blocks and fluids by height.
            let mut kept: Vec<(usize, i32, u16)> = Vec::new();
            let mut dropped: HashMap<u16, u64> = HashMap::new();
            for z in 0..16usize {
                for x in 0..16usize {
                    let col = &mut columns[z * 16 + x];
                    // The Nether's roof is no ground: start under it.
                    let mut y = max_y - 1;
                    if opts.dimension == DimensionKind::Nether {
                        y = y.min(127);
                        while y >= min_y && class(access.at(x, y, z)) != Class::Air {
                            y -= 1;
                        }
                    }
                    let mut above: Vec<(i32, u16)> = Vec::new();
                    while y >= min_y {
                        let b = access.at(x, y, z);
                        match class(b) {
                            Class::Air => {}
                            Class::Ground if is_ice(b) && y > min_y && wet(access.at(x, y - 1, z)) => above.push((y, b)),
                            Class::Ground => {
                                col.ground = Some((y, b));
                                break;
                            }
                            _ => above.push((y, b)),
                        }
                        y -= 1;
                    }
                    let gy = col.ground.map_or(min_y - 1, |g| g.0);
                    if col.ground.is_some() && gy > min_y {
                        let u = access.at(x, gy - 1, z);
                        if class(u) == Class::Ground {
                            col.under = Some(u);
                        }
                    }
                    above.reverse();
                    // The water (or lava) resting on the ground.
                    // Water rises to its last block over the ground with no
                    // air between: what was built in it (a ruin, steps)
                    // stands in the water.
                    let mut j = 0;
                    let mut next = gy + 1;
                    let mut end = 0;
                    let mut aquatic: Vec<u16> = Vec::new();
                    while j < above.len() && above[j].0 == next {
                        let b = above[j].1;
                        if class(b) == Class::Lava {
                            col.lava = true;
                            end = j + 1;
                        } else if is_ice(b) {
                            col.frozen = true;
                            end = j + 1;
                        } else if class(b) == Class::Water || classify::aquatic(state(b)) {
                            end = j + 1;
                        }
                        if classify::aquatic(state(b)) && aquatic.len() == j {
                            aquatic.push(b);
                        }
                        next += 1;
                        j += 1;
                        if col.frozen {
                            break;
                        }
                    }
                    let fluid_top = (end > 0).then(|| above[end - 1].0 + 1);
                    col.fluid = fluid_top;
                    aquatic.truncate(end);
                    // What stands in the water, then what stands over it.
                    let mut rest: Vec<(i32, u16)> = above[..end].iter().copied().filter(|&(_, b)| !matches!(class(b), Class::Water | Class::Lava) && !is_ice(b) && !classify::aquatic(state(b))).collect();
                    rest.extend_from_slice(&above[end..]);
                    // A plant on the ground: a cover.
                    if opts.covers {
                        if !aquatic.is_empty() && (rest.is_empty() || opts.above != Above::Keep) {
                            col.cover = Some((aquatic[0], aquatic.len().min(255) as u8, true));
                            aquatic.clear();
                        } else if fluid_top.is_none() && let Some(&(y0, b0)) = rest.first() && y0 == gy + 1 && class(b0) == Class::Plant {
                            let plant = classify::cover_state(state(b0));
                            let mut n = 0;
                            while n < rest.len() && rest[n].0 == gy + 1 + n as i32 && class(rest[n].1) == Class::Plant && base(&classify::cover_state(state(rest[n].1))) == base(&plant) {
                                n += 1;
                            }
                            let tall = massif_core::registry::is_tall_plant(state(b0));
                            if tall && n == 1 && rest.get(1).is_some_and(|r| r.0 == gy + 2 && massif_core::registry::is_tall_plant(state(r.1))) {
                                n = 2;
                            }
                            let alone = n == rest.len() || opts.above != Above::Keep;
                            if alone && (!tall || n == 2) {
                                col.cover = Some((b0, if tall { 1 } else { n.min(255) as u8 }, false));
                            }
                        }
                    }
                    let covered = col.cover.map_or(0, |c| if c.2 { 0 } else if massif_core::registry::is_tall_plant(state(c.0)) { 2 } else { c.1 as usize });
                    col.surface = fluid_top.map_or(gy, |t| t - 1);
                    let rest = &rest[covered.min(rest.len())..];
                    if opts.above == Above::Keep {
                        kept.extend(aquatic.iter().enumerate().map(|(i, &b)| (z * 16 + x, gy + 1 + i as i32, b)));
                    }
                    for &(y, b) in rest {
                        match opts.above {
                            Above::Keep => kept.push((z * 16 + x, y, b)),
                            Above::Trees if matches!(class(b), Class::Log | Class::Leaves) => {
                                let s = state(b);
                                let id = *tree_state_index.entry(s.to_owned()).or_insert_with(|| {
                                    tree_states.push(s.to_owned());
                                    (tree_states.len() - 1) as u32
                                });
                                tree_blocks.insert([wx * 16 + x as i32, y, wz * 16 + z as i32], id);
                                if class(b) == Class::Log && y == gy + 1 + covered as i32 {
                                    trunks.push(([wx * 16 + x as i32, y, wz * 16 + z as i32], tree_kind(s)));
                                }
                            }
                            _ => *dropped.entry(b).or_default() += 1,
                        }
                    }
                    if opts.above == Above::Trees
                        && let Some(&(_, b)) = rest.iter().rev().find(|r| matches!(class(r.1), Class::Log | Class::Leaves))
                    {
                        let (bx, bz) = (wx * 16 + x as i32, wz * 16 + z as i32);
                        let pos = TilePos::new(bx.div_euclid(128), bz.div_euclid(128));
                        tree_columns.push((pos, (bz.rem_euclid(128) * 128 + bx.rem_euclid(128)) as usize, tree_kind(state(b))));
                    }
                }
            }

            // ---- the registry: terrains, biomes, covers ----
            let mut mix_of: HashMap<u16, Vec<(String, u32)>> = HashMap::new();
            if opts.mixes {
                let mut counts: HashMap<u16, u32> = HashMap::new();
                for c in &columns {
                    if let Some((_, b)) = c.ground {
                        *counts.entry(b).or_default() += 1;
                    }
                }
                let mut set: Vec<(String, u32)> = counts.iter().filter(|(_, n)| **n >= 26).map(|(b, n)| (state(*b).to_owned(), *n)).collect();
                if set.len() >= 2 {
                    set.sort();
                    for (b, _) in counts.iter().filter(|(_, n)| **n >= 26) {
                        mix_of.insert(*b, set.clone());
                    }
                }
            }
            let mut terrain_of: HashMap<u16, u16> = HashMap::new();
            let mut biome_of: HashMap<String, u16> = HashMap::new();
            let mut cover_of: HashMap<(u16, u8, bool), u16> = HashMap::new();
            let mut voxel_of: HashMap<u16, u16> = HashMap::new();
            let mut cover_count: HashMap<(u16, u8, bool), u64> = HashMap::new();
            let (frost, lava_cover);
            {
                let mut m = ctx.maps.lock().expect("no panic holds the maps");
                for c in &columns {
                    if let Some((_, b)) = c.ground
                        && !terrain_of.contains_key(&b)
                    {
                        let t = match mix_of.get(&b) {
                            Some(set) => m.mix(set),
                            None => m.ground(state(b)),
                        };
                        terrain_of.insert(b, t.0);
                    }
                    if let Some(g) = c.ground {
                        let t = TerrainId(terrain_of[&g.1]);
                        *m.columns.entry(t).or_default() += 1;
                        if let Some(u) = c.under
                            && m.created.contains(&t)
                        {
                            *m.under.entry(t).or_default().entry(state(u).to_owned()).or_default() += 1;
                        }
                    }
                    if let Some(cv) = c.cover {
                        *cover_count.entry(cv).or_default() += 1;
                    }
                }
                for (cv, count) in &cover_count {
                    let n = m.cover(&classify::cover_state(state(cv.0)), cv.1, cv.2, *count);
                    cover_of.insert(*cv, n);
                }
                if opts.biomes {
                    for z in 0..16 {
                        for x in 0..16 {
                            if let Some(b) = d.biome_at(x, columns[z * 16 + x].surface, z)
                                && !biome_of.contains_key(b)
                            {
                                biome_of.insert(b.to_owned(), m.biome(b));
                            }
                        }
                    }
                }
                for &(_, _, b) in &kept {
                    if !matches!(class(b), Class::Water | Class::Lava) && !voxel_of.contains_key(&b) {
                        voxel_of.insert(b, m.voxel(state(b)).0);
                    }
                }
                m.voxel_blocks += kept.len() as u64;
                for (b, n) in dropped {
                    *m.dropped.entry(base(state(b)).to_owned()).or_default() += n;
                }
                frost = columns.iter().any(|c| c.frozen).then(|| m.special_cover("Frost"));
                lava_cover = columns.iter().any(|c| c.lava).then(|| m.special_cover("Lava"));
            }

            // ---- the tile ----
            let (bx0, bz0) = (wx * 16, wz * 16);
            let pos = TilePos::new(bx0.div_euclid(128), bz0.div_euclid(128));
            let tile = tiles.entry(pos).or_insert_with(|| TileBuf::new(min_y));
            for z in 0..16usize {
                for x in 0..16usize {
                    let c = &columns[z * 16 + x];
                    let i = ((bz0 + z as i32).rem_euclid(128) * 128 + (bx0 + x as i32).rem_euclid(128)) as usize;
                    let gy = c.ground.map_or(min_y - 1, |g| g.0);
                    tile.top[i] = (gy + 1) as f32;
                    tile.terrain[i] = c.ground.map_or(0, |g| terrain_of[&g.1]);
                    tile.fluid[i] = c.fluid.map_or(NO_FLUID, |f| f as i16);
                    if opts.biomes
                        && let Some(b) = d.biome_at(x, c.surface, z)
                    {
                        tile.biome[i] = biome_of[b];
                    }
                    tile.cover[i] = match (c.lava, c.frozen, c.cover) {
                        (true, _, _) => lava_cover.unwrap_or(0),
                        (_, true, _) => frost.unwrap_or(0),
                        (_, _, Some(cv)) => cover_of[&cv],
                        _ => 0,
                    };
                }
            }

            // ---- sculpted chunks: what stands above the ground, caves ----
            if (!kept.is_empty() || opts.caves) && ctx.sculpted.load(Ordering::Relaxed) >= MAX_SCULPTED {
                out.over += 1;
                let mut m = ctx.maps.lock().expect("no panic holds the maps");
                m.voxel_blocks -= kept.len() as u64;
                for &(_, _, b) in &kept {
                    *m.dropped.entry(base(state(b)).to_owned()).or_default() += 1;
                }
                kept.clear();
            }
            if !kept.is_empty() || opts.caves {
                let ground_top: Vec<i32> = columns.iter().map(|c| c.ground.map_or(min_y, |g| g.0 + 1)).collect();
                // The ground in a sculpted chunk has its column's terrain
                // (one that followed the column would run on into the
                // block set on it).
                let ground_terrain: Vec<Option<TerrainId>> = columns.iter().map(|c| c.ground.map(|g| TerrainId(terrain_of[&g.1]))).collect();
                let fluid_run: Vec<(i32, i32)> = columns.iter().map(|c| (c.ground.map_or(min_y, |g| g.0 + 1), c.fluid.unwrap_or(i32::MIN))).collect();
                let mut cubes: BTreeMap<i32, Vec<(usize, i32, u16)>> = BTreeMap::new();
                for &(col, y, b) in &kept {
                    cubes.entry(y.div_euclid(16)).or_default().push((col, y, b));
                }
                let mut cave_cubes: BTreeSet<i32> = BTreeSet::new();
                if opts.caves {
                    for z in 0..16usize {
                        for x in 0..16usize {
                            let top = ground_top[z * 16 + x];
                            for y in min_y..top - 1 {
                                if matches!(class(access.at(x, y, z)), Class::Air | Class::Water | Class::Lava) {
                                    cave_cubes.insert(y.div_euclid(16));
                                }
                            }
                        }
                    }
                }
                let all: BTreeSet<i32> = cubes.keys().copied().chain(cave_cubes.iter().copied()).collect();
                ctx.sculpted.fetch_add(all.len(), Ordering::Relaxed);
                let mut cave_terrains: HashMap<u16, u16> = HashMap::new();
                for cy in all {
                    let mut v = VoxelChunk::air();
                    let y0 = cy * 16;
                    for lz in 0..16i32 {
                        for lx in 0..16i32 {
                            let col = (lz * 16 + lx) as usize;
                            let top = ground_top[col];
                            for ly in 0..16i32 {
                                let y = y0 + ly;
                                if y >= max_y || y < min_y {
                                    continue;
                                }
                                if y < top {
                                    if cave_cubes.contains(&cy) {
                                        let b = access.at(lx as usize, y, lz as usize);
                                        match class(b) {
                                            Class::Air => {}
                                            Class::Water => v.set_fluid(lx, ly, lz, Some(Fluid::Water)),
                                            Class::Lava => v.set_fluid(lx, ly, lz, Some(Fluid::Lava)),
                                            _ => {
                                                v.set(lx, ly, lz, 255);
                                                let t = if y >= top - 6 {
                                                    ground_terrain[col]
                                                } else {
                                                    Some(TerrainId(match cave_terrains.get(&b) {
                                                        Some(t) => *t,
                                                        None => {
                                                            let t = ctx.maps.lock().expect("no panic holds the maps").voxel(state(b)).0;
                                                            cave_terrains.insert(b, t);
                                                            t
                                                        }
                                                    }))
                                                };
                                                v.set_terrain(lx, ly, lz, t);
                                            }
                                        }
                                    } else {
                                        v.set(lx, ly, lz, 255);
                                        v.set_terrain(lx, ly, lz, ground_terrain[col]);
                                    }
                                }
                            }
                        }
                    }
                    for &(col, y, b) in cubes.get(&cy).map(Vec::as_slice).unwrap_or(&[]) {
                        let (lx, lz, ly) = ((col % 16) as i32, (col / 16) as i32, y - y0);
                        match class(b) {
                            Class::Water if y >= fluid_run[col].1 || y < fluid_run[col].0 => v.set_fluid(lx, ly, lz, Some(Fluid::Water)),
                            Class::Lava => v.set_fluid(lx, ly, lz, Some(Fluid::Lava)),
                            Class::Water => {}
                            _ => {
                                v.set(lx, ly, lz, 255);
                                v.set_terrain(lx, ly, lz, Some(TerrainId(voxel_of[&b])));
                            }
                        }
                    }
                    out.voxels.push((ChunkPos::new(wx, cy, wz), v));
                }
            }
        }
    }

    // ---- trees: their kinds, painted where they stood; samples cut ----
    if opts.above == Above::Trees && (!trunks.is_empty() || !tree_columns.is_empty()) {
        let mut per_kind: HashMap<String, (u64, u64)> = HashMap::new();
        for (_, k) in &trunks {
            per_kind.entry(k.clone()).or_default().0 += 1;
        }
        for (_, _, k) in &tree_columns {
            per_kind.entry(k.clone()).or_default().1 += 1;
        }
        // Samples: whole trees well inside the region.
        let (rx0, rz0) = (file.x * 512, file.z * 512);
        let mut cut: Vec<(String, massif_core::objects::ObjectDef)> = Vec::new();
        for (base_at, kind) in &trunks {
            let inside = base_at[0] - rx0 >= 12 && base_at[0] - rx0 < 500 && base_at[2] - rz0 >= 12 && base_at[2] - rz0 < 500;
            if !inside {
                continue;
            }
            {
                let mut s = ctx.samples.lock().expect("no panic holds the samples");
                let n = s.entry(kind.clone()).or_default();
                if *n >= SAMPLES {
                    continue;
                }
                *n += 1;
            }
            if let Some(def) = cut_tree(*base_at, &tree_blocks, &tree_states, kind) {
                cut.push((kind.clone(), def));
            }
        }
        let mut m = ctx.maps.lock().expect("no panic holds the maps");
        for (kind, def) in cut {
            let s = m.species(&kind);
            m.reg.objects.push(def);
            let k = (m.reg.objects.len() - 1) as u32;
            m.species[s].objects.push(k);
        }
        for (kind, (trees, columns)) in per_kind {
            let s = m.species(&kind);
            m.species[s].trees += trees;
            m.species[s].columns += columns;
        }
        let kinds: Vec<(TilePos, usize, usize)> = tree_columns.into_iter().map(|(p, i, k)| (p, i, m.species(&k))).collect();
        drop(m);
        // Object layers are numbered once every region is read: the
        // plane holds the kind + 1 until then.
        for (p, i, s) in kinds {
            if let Some(t) = tiles.get_mut(&p) {
                t.objects[i] = (s + 1) as u16;
            }
        }
    }

    out.tiles = tiles.into_iter().map(|(p, t)| (p, t.encode())).collect();
    ctx.progress.tiles.fetch_add(out.tiles.len(), Ordering::Relaxed);
    ctx.regions_done.fetch_add(1, Ordering::Relaxed);
    out
}

/// A tree cut out of the world from the foot of its trunk: its logs and
/// the leaves near them, as an object.
fn cut_tree(at: [i32; 3], blocks: &HashMap<[i32; 3], u32>, states: &[String], kind: &str) -> Option<massif_core::objects::ObjectDef> {
    let mut seen: HashMap<[i32; 3], u32> = HashMap::new();
    let mut stack = vec![at];
    let is_log = |s: &str| classify(s) == Class::Log;
    while let Some(p) = stack.pop() {
        if seen.contains_key(&p) || (p[0] - at[0]).abs() > 12 || (p[2] - at[2]).abs() > 12 || p[1] < at[1] || p[1] > at[1] + 48 {
            continue;
        }
        let Some(&b) = blocks.get(&p) else { continue };
        seen.insert(p, b);
        if seen.len() > 20_000 {
            return None;
        }
        // From a log, every neighbour; from leaves, only on along leaves.
        let from_log = is_log(&states[b as usize]);
        for d in [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]] {
            let q = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
            if let Some(&nb) = blocks.get(&q)
                && (from_log || !is_log(&states[nb as usize]) || d[1] != 0)
            {
                stack.push(q);
            }
        }
    }
    if seen.len() < 3 {
        return None;
    }
    let min = [seen.keys().map(|p| p[0]).min()?, at[1], seen.keys().map(|p| p[2]).min()?];
    let max = [seen.keys().map(|p| p[0]).max()?, seen.keys().map(|p| p[1]).max()?, seen.keys().map(|p| p[2]).max()?];
    let size = [(max[0] - min[0] + 1) as u16, (max[1] - min[1] + 1) as u16, (max[2] - min[2] + 1) as u16];
    let mut palette = vec!["minecraft:air".to_owned()];
    let mut index: HashMap<u32, u16> = HashMap::new();
    let mut cells = vec![0u16; size[0] as usize * size[1] as usize * size[2] as usize];
    for (p, b) in &seen {
        let k = *index.entry(*b).or_insert_with(|| {
            palette.push(states[*b as usize].clone());
            (palette.len() - 1) as u16
        });
        let (x, y, z) = ((p[0] - min[0]) as usize, (p[1] - min[1]) as usize, (p[2] - min[2]) as usize);
        cells[(y * size[2] as usize + z) * size[0] as usize + x] = k;
    }
    let mut def = massif_core::objects::ObjectDef::new(&format!("{} tree at {} {} {}", classify::pretty(kind), at[0], at[1], at[2]), size, palette, cells);
    def.anchor = [(at[0] - min[0]) as u16, 0, (at[2] - min[2]) as u16];
    def.source = "Imported from the Minecraft world".into();
    Some(def)
}

/// Import a dimension of a Minecraft world.
pub fn import(world: &McWorld, opts: &Options, progress: Option<&ImportProgress>) -> Result<(World, ImportReport), ImportError> {
    let local = ImportProgress::default();
    let progress = progress.unwrap_or(&local);
    let dim = world.dimension(opts.dimension).ok_or_else(|| ImportError::Unsupported(format!("the world has no {}", opts.dimension.label())))?;
    if opts.max_y <= opts.min_y {
        return Err(ImportError::Unsupported("the heights to import are empty".into()));
    }
    // Only the regions the area reaches.
    let regions: Vec<&RegionFile> = dim
        .regions
        .iter()
        .filter(|r| opts.area.is_none_or(|[x0, z0, x1, z1]| r.x * 32 + 31 >= x0 && r.x * 32 <= x1 && r.z * 32 + 31 >= z0 && r.z * 32 <= z1))
        .collect();
    let chunks: usize = regions
        .par_iter()
        .map(|r| {
            region::presence(&r.path).map_or(0, |p| {
                p.iter().enumerate().filter(|(i, has)| **has && opts.area.is_none_or(|[x0, z0, x1, z1]| (x0..=x1).contains(&(r.x * 32 + (*i % 32) as i32)) && (z0..=z1).contains(&(r.z * 32 + (*i / 32) as i32)))).count()
            })
        })
        .sum();
    progress.total.store(chunks.max(1) as u64, Ordering::Relaxed);
    let maps = Mutex::new(Maps::new(Registry::vanilla()));
    let samples = Mutex::new(HashMap::new());
    let regions_done = AtomicUsize::new(0);
    let sculpted = AtomicUsize::new(0);
    let ctx = Ctx { opts, maps: &maps, progress, samples: &samples, regions_done: &regions_done, sculpted: &sculpted };
    let started = std::time::Instant::now();
    let outs: Vec<RegionOut> = regions.par_iter().map(|r| region_work(r, &ctx)).collect();
    if progress.cancel.load(Ordering::Relaxed) {
        return Err(ImportError::Cancelled);
    }
    let mut maps = maps.into_inner().expect("workers finished");

    // What lies under the top of the terrains made for the import.
    for t in maps.created.clone() {
        let Some(under) = maps.under.get(&t) else { continue };
        let total: u64 = under.values().sum();
        let Some((state, n)) = under.iter().max_by_key(|u| *u.1).map(|(s, n)| (s.clone(), *n)) else { continue };
        let top: Vec<String> = maps.reg.terrain(t).strata[0].blocks.iter().map(|b| maps.reg.block(b.0).state.clone()).collect();
        if n * 2 >= total && !top.contains(&state) && !matches!(base(&state), "minecraft:stone" | "minecraft:deepslate") {
            let b = maps.block(&state);
            maps.reg.terrains[t.0 as usize].strata.push(Stratum { blocks: vec![(b, 1)], depth: (3, 4), mix: Mix::Grain });
        } else if top.len() == 1 && top[0] == state {
            maps.reg.terrains[t.0 as usize].strata[0].depth = (4, 5);
        }
    }

    maps.finish_covers();

    // Trees: an object layer of each kind, of its trees cut out.
    let mut converted: Vec<(String, usize)> = Vec::new();
    let mut layer_of_kind: Vec<u16> = vec![0; maps.species.len()];
    let species = std::mem::take(&mut maps.species);
    for (k, s) in species.iter().enumerate() {
        if s.objects.is_empty() || s.columns == 0 {
            continue;
        }
        let mut layer = massif_core::objects::ObjectLayerDef::new(&format!("{} trees", classify::pretty(&s.name)), &s.objects, &maps.reg);
        layer.soil_only = false;
        layer.palette = "Imported".into();
        layer.color = s.objects.first().and_then(|o| maps.reg.objects.get(*o as usize)).map_or([58, 112, 52], |o| {
            o.palette.iter().find(|p| classify(p) == Class::Leaves).map_or([58, 112, 52], |p| massif_core::objects::guess_color(p))
        });
        let squares = s.columns as f32 / (layer.spacing as f32 * layer.spacing as f32);
        layer.density = (s.trees as f32 / squares.max(1.0)).clamp(0.05, 1.0);
        layer_of_kind[k] = maps.reg.add_object_layer(layer);
        converted.push((format!("{} trees: {} trees, {} of them cut out", classify::pretty(&s.name), s.trees, s.objects.len()), 0));
    }

    // ---- the dimension ------------------------------------------------------
    let mut dimension = Dimension::new(opts.dimension, opts.min_y, opts.max_y);
    let mut report = ImportReport { name: world.name.clone(), platform: world.version.as_ref().map(|v| format!("Minecraft {v}")), minecraft: true, ..Default::default() };
    let (mut chunks, mut unfinished, mut damaged, mut voxels, mut over) = (0, 0, 0, 0usize, 0usize);
    let mut versions: BTreeMap<i32, usize> = BTreeMap::new();
    let mut sculpted = Vec::new();
    let ground = dimension.ground_mut();
    let mut tile_count = 0;
    let renumber = opts.above == Above::Trees && layer_of_kind.iter().enumerate().any(|(k, l)| *l != k as u16 + 1);
    for out in outs {
        chunks += out.chunks;
        unfinished += out.unfinished;
        damaged += out.damaged;
        over += out.over;
        for (v, n) in out.versions {
            *versions.entry(v).or_default() += n;
        }
        for (pos, blob) in out.tiles {
            let blob = if renumber {
                let mut tile = Tile::decode(&blob).map_err(|e| ImportError::Unsupported(e.to_string()))?;
                let objects: Vec<u16> = tile.objects.dense().iter().map(|&o| if o == 0 { 0 } else { layer_of_kind.get(o as usize - 1).copied().unwrap_or(0) }).collect();
                tile.objects = Plane::from_vec(objects);
                tile.compact();
                Blob::from(tile.encode())
            } else {
                blob
            };
            ground.insert_blob(pos, blob);
            tile_count += 1;
        }
        sculpted.extend(out.voxels);
    }
    ground.take_dirty();
    for (pos, v) in sculpted {
        dimension.voxels.insert(pos, Arc::new(v));
        voxels += 1;
    }
    dimension.voxels.take_dirty();
    for level in &mut dimension.levels {
        level.freeze();
    }
    dimension.trim(0);
    if tile_count == 0 {
        return Err(ImportError::Unsupported(if unfinished > 0 { "the world's chunks are not generated far enough to have their surface yet".into() } else { "there is nothing to import in the area chosen".into() }));
    }
    let secs = started.elapsed().as_secs_f32();
    report.tiles = tile_count;
    report.dimensions.push(format!("{} ({chunks} chunks in {tile_count} tiles, Y {} to {}, {secs:.1} s)", opts.dimension.label(), opts.min_y, opts.max_y - 1));
    if voxels > 0 {
        report.kept.push(format!("{} blocks above the ground in {voxels} sculpted chunks", maps.voxel_blocks));
    }
    if opts.caves {
        report.kept.push("Caves and the underground in sculpted chunks".into());
    }
    let mut covers: Vec<(String, usize)> = maps.covers.values().collect::<BTreeSet<_>>().into_iter().filter_map(|n| maps.reg.cover(*n).map(|c| (c.name.clone(), 0))).collect();
    covers.sort();
    report.converted = converted;
    report.converted.extend(covers.into_iter().map(|(n, _)| (format!("{n} (ground cover)"), 0)));
    report.ground_covers = maps.covers.values().copied().collect::<BTreeSet<u16>>().into_iter().collect();
    let mut dropped: Vec<(String, u64)> = maps.dropped.into_iter().collect();
    dropped.sort_by_key(|d| std::cmp::Reverse(d.1));
    report.dropped = dropped.into_iter().map(|(n, c)| (classify::pretty(&n), c)).collect();
    report.created_terrains = maps.created.iter().map(|t| maps.reg.terrain(*t).name.clone()).collect();
    report.custom_terrains = report.created_terrains.len();
    let mut terrains: Vec<(String, u64)> = maps.columns.iter().map(|(t, n)| (maps.reg.terrain(*t).name.clone(), *n)).collect();
    terrains.sort_by_key(|t| std::cmp::Reverse(t.1));
    report.terrains = terrains;
    if unfinished > 0 {
        report.skipped_dimensions.push(format!("{unfinished} chunks not generated far enough (the world's edge)"));
    }
    if over > 0 {
        report.skipped_dimensions.push(format!("what stands above the ground in {over} chunks: a map holds at most {MAX_SCULPTED} sculpted chunks (import a smaller area to keep it all, or its trees as layers)"));
    }
    if damaged > 0 {
        report.skipped_dimensions.push(format!("{damaged} damaged chunks"));
    }
    if let Some((&dv, _)) = versions.iter().max_by_key(|v| v.1)
        && report.platform.is_none()
    {
        report.platform = Some(format!("Minecraft (data version {dv})"));
    }

    let sea_level = match opts.dimension {
        DimensionKind::Overworld => 63,
        DimensionKind::Nether => 32,
        DimensionKind::End => 0,
    };
    let spawn = world.spawn.filter(|_| opts.dimension == DimensionKind::Overworld).unwrap_or_else(|| {
        let (lo, hi) = dimension.bounds().unwrap_or((TilePos::new(0, 0), TilePos::new(0, 0)));
        let (x, z) = ((lo.x + hi.x + 1) * 64, (lo.z + hi.z + 1) * 64);
        [x, massif_core::ops::height_at(&dimension, 0, x, z, massif_core::ops::Surface::Top).map_or(sea_level + 1, massif_core::compose::block_top), z]
    });
    let world = World {
        // The report kept with the map, so that File ▸ Import Report… shows it again.
        meta: WorldMeta { name: world.name.clone(), seed: world.seed, spawn, sea_level, lava: false, import_report: serde_json::to_value(&report).ok(), ..Default::default() },
        registry: maps.reg,
        dims: vec![dimension],
        points: Vec::new(),
    };
    Ok((world, report))
}
