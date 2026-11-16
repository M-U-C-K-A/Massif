//! Turning the columns of one chunk into chunk NBT.
//!
//! The blocks of a chunk are worked out once, then written the way the
//! target version stores them (see [`crate::version`]):
//!
//! - 1.18 and later: 16³ sections holding a block-state palette with
//!   bit-packed indices (Y, Z, X order, entries never straddle a `long`)
//!   and a 4³ biome palette;
//! - 1.14 to 1.17: the same palettes under a `Level` compound, biomes as
//!   numbers, the world 256 blocks high;
//! - up to 1.12: numbered blocks, with the light of the sky worked out.
//!
//! From 1.14 on, light is left for the game to compute (`isLightOn: 0`).
//!
//! Every chunk carries the data version of the version written for, so the
//! game loads it as its own, without upgrading it. From 1.18 on it holds
//! what the game itself writes for a finished chunk but for what the game
//! works out exactly when it is missing: the heightmaps (made on load, as
//! `primeHeightmaps` does for any a chunk lacks), the light and the lists of
//! blocks to update after loading. Blending data and the `UpgradeData` are
//! only for chunks of older versions, and are left out as the game does.

use crate::nbt::{NbtWriter, TagType};
use crate::version::{Era, Target};
use massif_core::compose::{AutoBiomes, BiomeColumn, Materializer, Span, TileStack};
use massif_core::coords::{BlockRect, TilePos, cell_index};
use massif_core::objects::Placement;
use massif_core::registry::{BlockDef, Registry, is_soil, is_tall_plant};
use massif_core::world::{Dimension, World};

const SECTION_VOLUME: usize = 4096;
const UNSET: u16 = u16::MAX;
/// A block of an object the version has nothing like: left out.
const SKIP: u16 = u16::MAX;

/// Everything that stays constant while a dimension is exported.
pub struct ExportContext<'a> {
    reg: &'a Registry,
    mat: Materializer<'a>,
    pub min_y: i32,
    pub max_y: i32,
    sea_level: i32,
    /// Block states split into name and properties, by `BlockId`.
    states: Vec<(String, Vec<(String, String)>)>,
    /// Each state as an entry of a section's palette, encoded once: the
    /// inside of its compound and the end of it.
    palette_entries: Encoded,
    /// Biome identifiers by `BiomeId`.
    biomes: Vec<String>,
    /// The same, encoded as the strings of a list, then the one written
    /// for an unknown biome.
    biome_entries: Encoded,
    /// The version written for, and how it stores chunks.
    target: Target,
    era: Era,
    /// Before 1.13: the number and data of each block, by `BlockId`.
    numbered: Vec<(u8, u8)>,
    /// Before 1.18: the number of each biome, by `BiomeId`.
    biome_numbers: Vec<i32>,
    bedrock: u16,
    water: u16,
    /// The world's fluid is lava: nothing grows in it.
    lava: bool,
    /// Water and lava, for the lakes of caves, whatever the world's fluid.
    cave_fluids: [u16; 2],
    seed: u64,
    /// The plants of each cover (by number − 1), as written; `None` where
    /// the version has nothing like them.
    covers: Vec<Vec<Option<Plant>>>,
    /// Whether each block of the registry is soil that plants grow on.
    soil: Vec<bool>,
    /// The states of each object's palette as it stands each way: index
    /// `turn + 4` when mirrored, `turn` otherwise; [`SKIP`] for a block the
    /// version has nothing like. Empty for objects no layer uses.
    objects: Vec<Vec<Vec<u16>>>,
    /// The first state that is not a block of the registry: plants of
    /// covers and blocks of objects, which objects may take the place of.
    first_extra: u16,
    /// How far any object reaches from its anchor; 0 without objects.
    pub reach: i32,
    /// The ores laid in the rock, if any.
    ores: Option<massif_core::ores::Placer>,
    /// Of each ore of the settings, its states in stone and in deepslate
    /// ([`SKIP`] where the version has nothing like it).
    ore_states: Vec<[u16; 2]>,
    /// The first state of an ore: ores are ground, which objects do not
    /// take the place of.
    first_ore: u16,
    /// What each state is to an ore: 0 nothing, 1 rock of stone, 2 rock of
    /// deepslate.
    hosts: Vec<u8>,
    /// The heights where the substrate is one block in every column, and
    /// those where it is drawn block by block.
    bands: [(i32, i32, Option<u16>); 4],
    /// The biomes of columns left on automatic.
    auto: AutoBiomes,
}

impl<'a> ExportContext<'a> {
    /// Fails with a description if the registry or the build limits cannot be
    /// exported.
    pub fn new(world: &'a World, dim: &Dimension, target: &Target) -> Result<Self, String> {
        let reg = &world.registry;
        let era = target.era();
        let (min_y, max_y) = target.heights(dim.min_y, dim.max_y);
        if era == Era::Modern && (dim.min_y % 16 != 0 || dim.max_y % 16 != 0 || dim.max_y <= dim.min_y) {
            return Err(format!(
                "build limits {}..{} are not multiples of 16",
                dim.min_y, dim.max_y
            ));
        }
        if dim.min_y < -2032 || dim.max_y > 2032 || dim.max_y - dim.min_y > 4064 {
            return Err(format!(
                "build limits {}..{} exceed what Minecraft allows (−2032 to 2032, 4064 blocks at most)",
                dim.min_y, dim.max_y
            ));
        }
        if reg.blocks.is_empty() || reg.blocks.len() >= UNSET as usize {
            return Err("the block registry is empty or too large".into());
        }
        let n = reg.blocks.len();
        let wk = reg.well_known;
        for id in [wk.stone, wk.deepslate, wk.bedrock, wk.water, wk.lava] {
            if id.index() >= n {
                return Err("the registry's well-known blocks are out of range".into());
            }
        }
        for t in &reg.terrains {
            let blocks = t.strata.iter().flat_map(|s| s.blocks.iter().map(|b| b.0));
            if blocks.chain(t.submerged_top).any(|b| b.index() >= n) {
                return Err(format!("terrain \"{}\" uses an unknown block", t.name));
            }
        }
        // Blocks the version does not have yet become the closest it has.
        let mut states: Vec<(String, Vec<(String, String)>)> = reg
            .blocks
            .iter()
            .map(|b| {
                // The ground cannot have holes: stone for a block the
                // version has nothing like.
                let (name, kept) = target.block_or(b.name(), "minecraft:stone");
                let props = if kept { b.properties().map(|(k, v)| (k.to_owned(), v.to_owned())).collect() } else { Vec::new() };
                (name.to_owned(), props)
            })
            .collect();
        let mut numbered = match era {
            Era::Numbered => reg.blocks.iter().map(|b| target.numbered(b.name())).collect(),
            _ => Vec::new(),
        };
        // The plants of the covers come after the registry's blocks.
        let mut covers = Vec::with_capacity(reg.covers.len());
        for def in &reg.covers {
            let mut plants = Vec::with_capacity(def.plants.len());
            for (state, _) in &def.plants {
                let block = BlockDef { state: state.clone(), color: [0; 3] };
                let (name, kept) = target.block(block.name());
                let number = match era {
                    Era::Numbered => target.numbered_known(name),
                    _ => Some((0, 0)),
                };
                if name == "minecraft:air" || !name.contains(':') || number.is_none() {
                    plants.push(None);
                    continue;
                }
                // A tall plant is written in two halves.
                let tall = is_tall_plant(name);
                let props: Vec<(String, String)> = if kept { block.properties().filter(|(k, _)| !tall || *k != "half").map(|(k, v)| (k.to_owned(), v.to_owned())).collect() } else { Vec::new() };
                let mut add = |half: Option<&str>, number: Option<(u8, u8)>| {
                    let mut props = props.clone();
                    props.extend(half.map(|h| ("half".to_owned(), h.to_owned())));
                    states.push((name.to_owned(), props));
                    if era == Era::Numbered {
                        numbered.push(number.unwrap_or((0, 0)));
                    }
                    (states.len() - 1) as u16
                };
                let lower = add(tall.then_some("lower"), number);
                // Before 1.13 the top half of every tall plant is 175:8.
                let upper = tall.then(|| add(Some("upper"), number.map(|(id, _)| (id, 8))));
                plants.push(Some(Plant { lower, upper }));
            }
            covers.push(plants);
        }
        // The objects of the layers, each way they may stand.
        let used: std::collections::HashSet<u32> = reg.object_layers.iter().filter(|l| !l.removed).flat_map(|l| l.objects.iter().map(|o| o.0)).collect();
        let mut known: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
        let mut objects = Vec::with_capacity(reg.objects.len());
        for (k, def) in reg.objects.iter().enumerate() {
            if !used.contains(&(k as u32)) {
                objects.push(Vec::new());
                continue;
            }
            let mut ways = Vec::with_capacity(8);
            for way in 0..8u8 {
                let ids = def
                    .palette
                    .iter()
                    .enumerate()
                    .map(|(i, state)| {
                        if i == 0 {
                            return SKIP;
                        }
                        let state = massif_core::objects::transform_state(state, way & 3, way >= 4);
                        if let Some(&id) = known.get(&state) {
                            return id;
                        }
                        let block = BlockDef { state: state.clone(), color: [0; 3] };
                        let (name, kept) = target.block(block.name());
                        let number = match era {
                            // Logs lie along their axis, leaves stay.
                            Era::Numbered => target.numbered_known(name).map(|(id, data)| {
                                let axis = block.properties().find(|p| p.0 == "axis").map(|p| p.1);
                                let bits = match (name.ends_with("_log"), axis) {
                                    (true, Some("x")) => 4,
                                    (true, Some("z")) => 8,
                                    _ if name.ends_with("_leaves") => 4,
                                    _ => 0,
                                };
                                (id, data | bits)
                            }),
                            _ => Some((0, 0)),
                        };
                        let id = match number {
                            Some(number) if name != "minecraft:air" && name.contains(':') => {
                                let props = if kept { block.properties().map(|(k, v)| (k.to_owned(), v.to_owned())).collect() } else { Vec::new() };
                                states.push((name.to_owned(), props));
                                if era == Era::Numbered {
                                    numbered.push(number);
                                }
                                (states.len() - 1).min(SKIP as usize - 1) as u16
                            }
                            _ => SKIP,
                        };
                        known.insert(state, id);
                        id
                    })
                    .collect();
                ways.push(ids);
            }
            objects.push(ways);
        }
        // The ores, after everything else: in stone and in deepslate.
        let first_ore = states.len().min(SKIP as usize - 1) as u16;
        let ore_states = world
            .meta
            .ores
            .ores
            .iter()
            .map(|rule| {
                let (stone, deep) = massif_core::ores::ore_blocks(rule.mineral);
                [stone, deep].map(|state| {
                    let block = BlockDef { state: state.to_owned(), color: [0; 3] };
                    let (name, kept) = target.block(block.name());
                    let number = match era {
                        Era::Numbered => target.numbered_known(name),
                        _ => Some((0, 0)),
                    };
                    match number {
                        Some(number) if name != "minecraft:air" && name.contains(':') => {
                            let props = if kept { block.properties().map(|(k, v)| (k.to_owned(), v.to_owned())).collect() } else { Vec::new() };
                            states.push((name.to_owned(), props));
                            if era == Era::Numbered {
                                numbered.push(number);
                            }
                            (states.len() - 1).min(SKIP as usize - 1) as u16
                        }
                        _ => SKIP,
                    }
                })
            })
            .collect();
        if states.len() >= UNSET as usize {
            return Err("the covers and objects use too many blocks".into());
        }
        let mut hosts: Vec<u8> = reg.blocks.iter().map(|b| massif_core::ores::host_of(&b.state).map_or(0, |h| 1 + (h == massif_core::ores::Host::Deepslate) as u8)).collect();
        hosts.resize(states.len(), 0);
        // At least 1 with objects: 0 means none, and an object one block wide
        // reaches no further than its anchor.
        let reach = if used.is_empty() { 0 } else { massif_core::objects::max_reach(reg).max(1) };
        let soil = reg.blocks.iter().map(|b| is_soil(b.name())).collect();
        let biomes: Vec<String> = reg
            .biomes
            .iter()
            .map(|b| {
                if b.id.is_empty() {
                    "minecraft:plains".to_owned()
                } else {
                    target.biome(&b.id).to_owned()
                }
            })
            .collect();
        let biome_numbers = match era {
            Era::Modern => Vec::new(),
            _ => reg.biomes.iter().map(|b| target.biome_number(if b.id.is_empty() { "minecraft:plains" } else { &b.id })).collect(),
        };
        let palette_entries = Encoded::of(states.iter().map(|(name, props)| {
            let mut w = NbtWriter::fragment();
            w.string("Name", name);
            if !props.is_empty() {
                w.compound("Properties");
                for (k, v) in props {
                    w.string(k, v);
                }
                w.end();
            }
            w.end();
            w.into_bytes()
        }));
        let biome_entries = Encoded::of(biomes.iter().map(String::as_str).chain(["minecraft:plains"]).map(|name| {
            let mut w = NbtWriter::fragment();
            w.item_string(name);
            w.into_bytes()
        }));
        let mat = Materializer::new(reg, world.meta.seed, dim.min_y);
        let bands = mat.substrate_bands().map(|(y0, y1, b)| (y0, y1, b.map(|b| b.0)));
        Ok(Self {
            reg,
            mat,
            bands,
            min_y,
            max_y,
            sea_level: world.meta.sea_level,
            states,
            palette_entries,
            biomes,
            biome_entries,
            target: target.clone(),
            era,
            numbered,
            biome_numbers,
            bedrock: wk.bedrock.0,
            water: if world.meta.lava { wk.lava.0 } else { wk.water.0 },
            lava: world.meta.lava,
            cave_fluids: [wk.water.0, wk.lava.0],
            seed: world.meta.seed as u64,
            covers,
            soil,
            objects,
            first_extra: reg.blocks.len() as u16,
            reach,
            ores: massif_core::ores::Placer::new(&world.meta.ores, world.meta.seed),
            ore_states,
            first_ore,
            hosts,
            auto: AutoBiomes::new(reg, world.meta.sea_level, min_y),
        })
    }

    pub fn sections(&self) -> usize {
        ((self.max_y - self.min_y) / 16) as usize
    }

    pub fn materializer(&self) -> &Materializer<'a> {
        &self.mat
    }

    /// The biome `id` encoded as an item of a list of strings.
    fn biome_entry(&self, id: u16) -> &[u8] {
        self.biome_entries.get((id as usize).min(self.biomes.len()))
    }

    /// The number of a biome, for the versions that number them.
    fn biome_number(&self, id: u16) -> i32 {
        self.biome_numbers.get(id as usize).copied().unwrap_or(1)
    }
}

/// Pieces of NBT encoded once, one after the other, by number.
struct Encoded {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl Encoded {
    fn of(pieces: impl Iterator<Item = Vec<u8>>) -> Self {
        let (mut bytes, mut ends) = (Vec::new(), Vec::new());
        for piece in pieces {
            bytes.extend_from_slice(&piece);
            ends.push(bytes.len());
        }
        Self { bytes, ends }
    }

    fn get(&self, i: usize) -> &[u8] {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.bytes[start..self.ends[i]]
    }
}

/// One plant of a cover, as written: its block and, for a tall plant, the
/// block of its upper half (indices into the exported states).
#[derive(Debug, Clone, Copy)]
struct Plant {
    lower: u16,
    upper: Option<u16>,
}

/// Grow the cover numbered `cover` on the column whose topmost block is at
/// `y - 1`: in the air above it, or in the water for a cover that grows
/// under water. Nothing grows where the whole plant has no room.
/// Grow the plant of `cover` on the top of a column, at `y`. Returns the
/// height above the last block it put (`y` if none).
fn grow(ctx: &ExportContext, cover: u16, x: i32, y: i32, z: i32, blocks: &mut [u16], column: usize) -> i32 {
    let (min_y, max_y) = (ctx.min_y, ctx.max_y);
    let (Some(def), Some(plants)) = (ctx.reg.cover(cover), ctx.covers.get(cover as usize - 1)) else { return y };
    if y <= min_y || y >= max_y {
        return y;
    }
    let Some(growth) = def.growth(x, z, ctx.seed) else { return y };
    let Some(Some(plant)) = plants.get(growth.plant) else { return y };
    let at = |y: i32| (((y - min_y) as usize) << 8) | column;
    if def.soil_only && !ctx.soil.get(blocks[at(y - 1)] as usize).copied().unwrap_or(false) {
        return y;
    }
    let medium = match (def.underwater, ctx.lava) {
        (false, _) => 0,
        (true, false) => ctx.water,
        (true, true) => return y,
    };
    let height = if plant.upper.is_some() { 2 } else { growth.height as i32 };
    if y + height > max_y || (0..height).any(|k| blocks[at(y + k)] != medium) {
        return y;
    }
    for k in 0..height {
        blocks[at(y + k)] = match plant.upper {
            Some(upper) if k == 1 => upper,
            _ => plant.lower,
        };
    }
    y + height
}

/// Whether every block of a section is the same: compared 64 at a time,
/// without stopping inside a run (which the compiler turns into wide
/// comparisons).
fn uniform(section: &[u16]) -> bool {
    let first = section[0];
    let (runs, rest) = section.as_chunks::<64>();
    runs.iter().all(|run| run.iter().fold(true, |same, &b| same & (b == first))) && rest.iter().all(|&b| b == first)
}

/// Fill a column of `blocks` with the substrate from `y0` to `y1`: a band
/// at a time, a constant where it is one block everywhere. Below `laid`, the
/// constant bands are in already (see [`lay_substrate`]).
fn fill_substrate(ctx: &ExportContext, x: i32, z: i32, (y0, y1): (i32, i32), laid: i32, blocks: &mut [u16], column: usize) {
    let at = |y: i32| (((y - ctx.min_y) as usize) << 8) | column;
    for &(b0, b1, block) in &ctx.bands {
        let (lo, hi) = (b0.max(y0), b1.min(y1));
        if lo >= hi {
            continue;
        }
        match block {
            Some(b) => {
                let lo = lo.max(laid);
                if lo >= hi {
                    continue;
                }
                for v in blocks[at(lo)..].iter_mut().step_by(256).take((hi - lo) as usize) {
                    *v = b;
                }
            }
            None => {
                let substrate = ctx.mat.substrate_column(x, z);
                for y in lo..hi {
                    blocks[at(y)] = substrate.at(y).0;
                }
            }
        }
    }
}

/// Lay the constant bands of the substrate from the floor of the world up
/// to `laid`, where every column of the chunk has it: whole layers of the
/// chunk at a time, rather than a column at a time across them.
fn lay_substrate(ctx: &ExportContext, laid: i32, blocks: &mut [u16]) {
    for &(b0, b1, block) in &ctx.bands {
        let (lo, hi) = (b0.max(ctx.min_y), b1.min(laid));
        if let Some(b) = block
            && lo < hi
        {
            blocks[((lo - ctx.min_y) as usize) << 8..((hi - ctx.min_y) as usize) << 8].fill(b);
        }
    }
}

/// Reusable buffers for [`build_chunk`]; keep one per worker thread.
pub struct Scratch {
    blocks: Vec<u16>,
    /// How much of `blocks` the last chunk may have written (the rest is
    /// air): what the next one clears.
    dirty: usize,
    lookup: Vec<u16>,
    /// The place in the palette of each block of a section.
    indices: Vec<u16>,
    palette: Vec<u16>,
    packed: Vec<i64>,
    spans: Vec<Span>,
    /// The spans of every column of the chunk, one after the other, and
    /// where each column's begin (and the last ends).
    column_spans: Vec<Span>,
    span_starts: Vec<usize>,
    /// The recipe depth of each column's lowest span.
    recipes: [u32; 256],
    fluids: Vec<(i32, i32)>,
    cave_fluids: Vec<(i32, i32, massif_core::voxel::Fluid)>,
    biome_columns: Vec<BiomeColumn>,
    /// The ore blocks of each chunk of the tile being exported (see
    /// [`Scratch::lay_ores`]), row by row.
    ores: Vec<Vec<massif_core::ores::OreBlock>>,
}

impl Scratch {
    pub fn new(ctx: &ExportContext) -> Self {
        Self {
            blocks: vec![0; ctx.sections() * SECTION_VOLUME],
            dirty: 0,
            lookup: vec![UNSET; ctx.states.len()],
            indices: vec![0; SECTION_VOLUME],
            palette: Vec::with_capacity(64),
            packed: Vec::with_capacity(512),
            spans: Vec::with_capacity(8),
            column_spans: Vec::with_capacity(512),
            span_starts: vec![0; 257],
            recipes: [0; 256],
            fluids: Vec::with_capacity(4),
            cave_fluids: Vec::new(),
            biome_columns: vec![BiomeColumn::default(); 16],
            ores: Vec::new(),
        }
    }

    /// Draw the veins of the chunks of tile `tile`, whose levels are
    /// `stack` (each vein once, however many of its chunks it reaches; none
    /// up in the sky), before its chunks are built.
    pub fn lay_ores(&mut self, ctx: &ExportContext, stack: &TileStack, tile: TilePos) {
        self.ores = match &ctx.ores {
            Some(placer) => {
                let n = massif_core::TILE_SIZE / 16;
                let heights = massif_core::ores::stack_heights(stack, ctx.min_y, ctx.max_y);
                placer.chunks(tile.x * n, tile.z * n, n, n, heights)
            }
            None => Vec::new(),
        };
    }
}

#[inline]
fn bits_for(palette_len: usize) -> usize {
    (usize::BITS - (palette_len - 1).leading_zeros()) as usize
}

/// Pack `map` of each of `values` into longs, `bits` per entry, never
/// crossing a long. The width is a constant inside the loop (one copy per
/// width), so that each long is put together in a register.
fn pack(values: &[u16], map: impl Fn(u16) -> u64 + Copy, bits: usize, out: &mut Vec<i64>) {
    macro_rules! widths {
        ($($n:literal)*) => {
            match bits {
                $($n => pack_as::<$n>(values, map, out),)*
                _ => panic!("{bits} bits an entry"),
            }
        };
    }
    widths!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16)
}

#[inline(always)]
fn pack_as<const BITS: usize>(values: &[u16], map: impl Fn(u16) -> u64, out: &mut Vec<i64>) {
    out.clear();
    out.extend(values.chunks(64 / BITS).map(|run| run.iter().enumerate().fold(0u64, |word, (k, &v)| word | (map(v) << (k * BITS))) as i64));
}

/// The objects standing with their anchor in tile `pos` (see
/// [`massif_core::objects::tile_placements`]).
pub fn tile_placements(ctx: &ExportContext, world: &World, dim: &Dimension, pos: TilePos) -> Vec<Placement> {
    if ctx.reach == 0 {
        return Vec::new();
    }
    match TileStack::new(dim, pos, true) {
        Some(stack) => massif_core::objects::tile_placements(ctx.reg, &ctx.mat, world.meta.seed as u64, world.meta.lava, &stack, pos),
        None => Vec::new(),
    }
}

/// Stand objects in the blocks of the chunk whose corner is `(x0, z0)`.
fn stand_objects(ctx: &ExportContext, objects: &[Placement], x0: i32, z0: i32, blocks: &mut [u16]) {
    let (min_y, max_y) = (ctx.min_y, ctx.max_y);
    for p in objects {
        let (Some(def), Some(ways)) = (ctx.reg.objects.get(p.object as usize), ctx.objects.get(p.object as usize)) else { continue };
        let Some(states) = ways.get((p.turn & 3) as usize + if p.mirror { 4 } else { 0 }) else { continue };
        let replace = ctx.reg.object_layer(p.layer).is_some_and(|l| l.replace);
        let [w, h, l] = def.size.map(|s| s as i32);
        let base = p.y - def.anchor[1] as i32;
        for bz in 0..l {
            for bx in 0..w {
                let (x, _, z) = p.world(def, bx, 0, bz);
                let (lx, lz) = (x - x0, z - z0);
                if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
                    continue;
                }
                let column = ((lz << 4) | lx) as usize;
                for by in 0..h {
                    let y = base + by;
                    if y < min_y || y >= max_y {
                        continue;
                    }
                    let k = def.at(bx as usize, by as usize, bz as usize);
                    let state = states.get(k as usize).copied().unwrap_or(SKIP);
                    if k == 0 || state == SKIP {
                        continue;
                    }
                    let i = (((y - min_y) as usize) << 8) | column;
                    let old = blocks[i];
                    // Unless it replaces the ground, an object only takes
                    // the place of air, water, plants and other objects.
                    if replace || old == 0 || old == ctx.water || (old >= ctx.first_extra && old < ctx.first_ore) {
                        blocks[i] = state;
                    }
                }
            }
        }
    }
}

/// Build the NBT of the chunk at chunk position `(cx, cz)` *within* the tile
/// (both in `0..8`), with the objects of `objects` that reach into it (in
/// the same order for every chunk, so that where two meet, the same one
/// wins).
pub fn build_chunk(
    ctx: &ExportContext,
    stack: &TileStack,
    tile: TilePos,
    cx: usize,
    cz: usize,
    objects: &[Placement],
    scratch: &mut Scratch,
) -> Vec<u8> {
    let (ox, oz) = tile.origin();
    let (bx0, bz0) = (ox + cx as i32 * 16, oz + cz as i32 * 16);
    let (min_y, max_y) = (ctx.min_y, ctx.max_y);
    let sections = ctx.sections();
    let Scratch {
        blocks,
        dirty,
        lookup,
        indices,
        palette,
        packed,
        spans,
        column_spans,
        span_starts,
        recipes,
        fluids,
        cave_fluids,
        biome_columns,
        ores,
    } = scratch;

    // ---- blocks ------------------------------------------------------------
    // What the last chunk wrote is cleared (above it, all is air already);
    // `top` follows the highest block this one writes.
    blocks[..*dirty].fill(0);
    // The spans of every column first: up to the lowest height where a
    // column's substrate ends (when every column has its ground from the
    // floor of the world), the substrate's constant bands are laid for the
    // whole chunk at once.
    column_spans.clear();
    let mut laid = max_y;
    for column in 0..256usize {
        let (lx, lz) = (column & 15, column >> 4);
        span_starts[column] = column_spans.len();
        stack.spans(cell_index(cx * 16 + lx, cz * 16 + lz), spans);
        column_spans.extend_from_slice(spans);
        laid = match spans.first() {
            Some(span) => {
                let recipe = ctx.mat.recipe_depth(span.terrain, bx0 + lx as i32, bz0 + lz as i32);
                recipes[column] = recipe;
                let split = if recipe >= (span.y1 - span.y0) as u32 { span.y0 } else { span.y1 - recipe as i32 };
                if span.y0 <= min_y { laid.min(split) } else { min_y }
            }
            None => min_y,
        };
    }
    span_starts[256] = column_spans.len();
    let laid = laid.clamp(min_y, max_y);
    lay_substrate(ctx, laid, blocks);
    let mut top = min_y;
    for lz in 0..16usize {
        for lx in 0..16usize {
            let cell = cell_index(cx * 16 + lx, cz * 16 + lz);
            let (x, z) = (bx0 + lx as i32, bz0 + lz as i32);
            let column = (lz << 4) | lx;
            let at = |y: i32| (((y - min_y) as usize) << 8) | column;
            let spans = &column_spans[span_starts[column]..span_starts[column + 1]];
            stack.fluids(cell, spans, fluids);
            for (k, span) in spans.iter().enumerate() {
                let submerged = fluids.iter().any(|f| f.0 == span.y1);
                let recipe = if k == 0 { recipes[column] } else { ctx.mat.recipe_depth(span.terrain, x, z) };
                let split = if recipe >= (span.y1 - span.y0) as u32 {
                    span.y0
                } else {
                    span.y1 - recipe as i32
                };
                // What lies outside the version's heights is left out.
                fill_substrate(ctx, x, z, (span.y0.max(min_y), split.min(max_y)), laid, blocks, column);
                let (lo, hi) = (split.max(min_y), span.y1.min(max_y));
                if lo < hi {
                    ctx.mat.recipe_column(span, x, z, submerged, (lo, hi), |y, b| blocks[at(y)] = b.0);
                }
                top = top.max(span.y1);
            }
            for &(y0, y1) in fluids.iter() {
                for y in y0.max(min_y)..y1.min(max_y) {
                    blocks[at(y)] = ctx.water;
                }
                top = top.max(y1);
            }
            // The lakes of caves hold their own fluid.
            if stack.is_sculpted() {
                stack.sculpted_fluids(cell, cave_fluids);
                for &(y0, y1, fluid) in cave_fluids.iter() {
                    let block = ctx.cave_fluids[fluid as usize];
                    for y in y0.max(min_y)..y1.min(max_y) {
                        blocks[at(y)] = block;
                    }
                }
            }
            // What grows on the top of the column.
            if !ctx.covers.is_empty()
                && let Some(surface) = spans.last()
            {
                let cover = stack.surface_cover(cell, surface);
                if cover != 0 {
                    top = top.max(grow(ctx, cover, x, surface.y1, z, blocks, column));
                }
            }
            // A world cut at 0 gets its floor of bedrock there.
            if ctx.era != Era::Modern && blocks[at(min_y)] != 0 {
                blocks[at(min_y)] = ctx.bedrock;
            }
        }
    }

    // ---- biomes: one sample per 4×4 column ---------------------------------
    for bz in 0..4usize {
        for bx in 0..4usize {
            let cell = cell_index(cx * 16 + bx * 4 + 2, cz * 16 + bz * 4 + 2);
            let top = || {
                stack.spans(cell, spans);
                spans.last().map(|s| (s.y1, s.terrain))
            };
            stack.biome_column(cell, ctx.reg, &ctx.auto, top, &mut biome_columns[bz * 4 + bx]);
        }
    }

    // ---- ores --------------------------------------------------------------
    // In the rock, where solid ground lies right above, and in a biome the
    // ore keeps to; the first vein to reach a block keeps it (the next finds
    // ore there, not rock).
    let keeps = ctx.ores.as_ref().is_some_and(|p| p.keeps_to_biomes());
    if let Some(chunk) = ores.get(cz * 8 + cx) {
        for b in chunk {
            if b.y < min_y || b.y + 1 >= max_y {
                continue;
            }
            let i = (((b.y - min_y) as usize) << 8) | ((((b.z - bz0) << 4) | (b.x - bx0)) as usize);
            let above = blocks[i + 256];
            let solid = above != 0 && above != ctx.water && !ctx.cave_fluids.contains(&above) && (above < ctx.first_extra || above >= ctx.first_ore);
            let host = ctx.hosts.get(blocks[i] as usize).copied().unwrap_or(0);
            if !solid || host == 0 {
                continue;
            }
            if keeps && let Some(placer) = &ctx.ores {
                let column = &biome_columns[(((b.z - bz0) >> 2) * 4 + ((b.x - bx0) >> 2)) as usize];
                // The biome the version stores for the block: by columns
                // up to 1.14, by cells of 4 × 4 × 4 after.
                let biome = if matches!(ctx.era, Era::Numbered | Era::Flat14) { column.at(ctx.sea_level) } else { column.of_block(b.y) };
                if !placer.allows(b.ore, biome) {
                    continue;
                }
            }
            let state = ctx.ore_states[b.ore as usize][host as usize - 1];
            if state != SKIP {
                blocks[i] = state;
            }
        }
    }

    // ---- objects -----------------------------------------------------------
    if !objects.is_empty() {
        let chunk = BlockRect::new(bx0, bz0, bx0 + 16, bz0 + 16);
        let near: Vec<Placement> = objects
            .iter()
            .filter(|p| ctx.reg.objects.get(p.object as usize).is_some_and(|def| !p.bounds(def).intersect(&chunk).is_empty()))
            .copied()
            .collect();
        stand_objects(ctx, &near, bx0, bz0, blocks);
        for p in &near {
            if let Some(def) = ctx.reg.objects.get(p.object as usize) {
                top = top.max(p.y - def.anchor[1] as i32 + def.size[1] as i32);
            }
        }
    }
    // The sections that hold something; those above are air.
    let filled = (((top.min(max_y) - min_y).max(0) as usize).div_ceil(16)).min(sections);
    *dirty = filled * SECTION_VOLUME;

    // ---- NBT ---------------------------------------------------------------
    match ctx.era {
        Era::Modern => {}
        Era::Numbered => return numbered_chunk(ctx, blocks, biome_columns, bx0 >> 4, bz0 >> 4),
        _ => return flat_chunk(ctx, blocks, biome_columns, lookup, palette, packed, bx0 >> 4, bz0 >> 4),
    }
    let mut w = NbtWriter::with_capacity(32 * 1024);
    w.int("DataVersion", ctx.target.data_version);
    w.int("xPos", bx0 >> 4);
    w.int("yPos", min_y >> 4);
    w.int("zPos", bz0 >> 4);
    w.string("Status", ctx.target.full_status());
    w.long("LastUpdate", 0);
    w.long("InhabitedTime", 0);
    w.bool("isLightOn", false);

    w.list("sections", TagType::Compound, sections);
    let mut biome_ids = [0u16; 64];
    let mut biome_palette: Vec<u16> = Vec::with_capacity(4);
    for s in 0..sections {
        let section = &blocks[s * SECTION_VOLUME..(s + 1) * SECTION_VOLUME];
        w.item_compound();
        w.byte("Y", ((min_y >> 4) + s as i32) as i8);

        // Block states.
        palette.clear();
        let first = section[0];
        if s >= filled {
            palette.push(0);
        } else if uniform(section) {
            palette.push(first);
        } else {
            // In the order they first appear; a block like the one before
            // it (most of them, in runs along x) is in already.
            // Each block's place in the palette as it goes, for packing: read
            // for every block, which beats guessing whether it is the one
            // before (the guess fails too often on mixed ground).
            for (&b, slot) in section.iter().zip(indices.iter_mut()) {
                let mut index = lookup[b as usize];
                if index == UNSET {
                    index = palette.len() as u16;
                    lookup[b as usize] = index;
                    palette.push(b);
                }
                *slot = index;
            }
        }
        w.compound("block_states");
        w.list("palette", TagType::Compound, palette.len());
        for &b in palette.iter() {
            w.item_compound();
            w.raw(ctx.palette_entries.get(b as usize));
        }
        if palette.len() > 1 {
            let bits = bits_for(palette.len()).max(4);
            pack(indices, |i| i as u64, bits, packed);
            w.long_array("data", packed);
            for &b in palette.iter() {
                lookup[b as usize] = UNSET;
            }
        }
        w.end();

        // Biomes: 4×4×4 cells, sampled at each cell's centre.
        biome_palette.clear();
        let section_y = min_y + s as i32 * 16;
        for by in 0..4usize {
            let y = section_y + by as i32 * 4 + 2;
            for (c, column) in biome_columns.iter().enumerate() {
                let id = column.at(y);
                let index = match biome_palette.iter().position(|&p| p == id) {
                    Some(i) => i,
                    None => {
                        biome_palette.push(id);
                        biome_palette.len() - 1
                    }
                };
                biome_ids[(by << 4) | c] = index as u16;
            }
        }
        w.compound("biomes");
        w.list("palette", TagType::String, biome_palette.len());
        for &id in &biome_palette {
            w.raw(ctx.biome_entry(id));
        }
        if biome_palette.len() > 1 {
            let bits = bits_for(biome_palette.len());
            pack(&biome_ids, |i| i as u64, bits, packed);
            w.long_array("data", packed);
        }
        w.end();

        w.end(); // section
    }

    w.list("block_entities", TagType::Compound, 0);
    w.list("block_ticks", TagType::Compound, 0);
    w.list("fluid_ticks", TagType::Compound, 0);
    w.compound("Heightmaps");
    w.end();
    w.compound("structures");
    w.compound("References");
    w.end();
    w.compound("starts");
    w.end();
    w.end();
    w.finish()
}

/// Pack `values` into longs, `bits` per entry, end to end: an entry may
/// begin in one long and end in the next (1.14 and 1.15).
fn pack_spanning(values: impl Iterator<Item = u64>, count: usize, bits: usize, out: &mut Vec<i64>) {
    out.clear();
    out.resize((count * bits).div_ceil(64), 0);
    for (i, v) in values.enumerate() {
        let (word, shift) = (i * bits / 64, i * bits % 64);
        out[word] |= (v << shift) as i64;
        if shift + bits > 64 {
            out[word + 1] |= (v >> (64 - shift)) as i64;
        }
    }
}

/// The biome of the 4-block cell `c` (of the chunk's 16) at height `y`.
fn biome_at(columns: &[BiomeColumn], c: usize, y: i32) -> u16 {
    columns[c].at(y)
}

/// A chunk as 1.14 to 1.17 store it: everything under `Level`, sections
/// with a palette of block states, biomes as numbers, 256 blocks high.
#[allow(clippy::too_many_arguments)]
fn flat_chunk(
    ctx: &ExportContext,
    blocks: &[u16],
    biome_columns: &[BiomeColumn],
    lookup: &mut [u16],
    palette: &mut Vec<u16>,
    packed: &mut Vec<i64>,
    chunk_x: i32,
    chunk_z: i32,
) -> Vec<u8> {
    let mut w = NbtWriter::with_capacity(32 * 1024);
    w.int("DataVersion", ctx.target.data_version);
    w.compound("Level");
    w.int("xPos", chunk_x);
    w.int("zPos", chunk_z);
    w.string("Status", "full");
    w.long("LastUpdate", 0);
    w.long("InhabitedTime", 0);
    w.bool("isLightOn", false);

    // Biomes: a number per column in 1.14, per 4×4×4 cell after.
    let biomes: Vec<i32> = if ctx.era == Era::Flat14 {
        (0..256).map(|i| ctx.biome_number(biome_at(biome_columns, ((i >> 4) / 4) * 4 + (i & 15) / 4, ctx.sea_level))).collect()
    } else {
        (0..1024)
            .map(|i| {
                let (y4, z4, x4) = (i >> 4, (i >> 2) & 3, i & 3);
                ctx.biome_number(biome_at(biome_columns, z4 * 4 + x4, ctx.min_y + y4 as i32 * 4 + 2))
            })
            .collect()
    };
    w.int_array("Biomes", &biomes);

    let sections: Vec<usize> = (0..ctx.sections()).filter(|s| blocks[s * SECTION_VOLUME..(s + 1) * SECTION_VOLUME].iter().any(|&b| b != 0)).collect();
    w.list("Sections", TagType::Compound, sections.len());
    for s in sections {
        let section = &blocks[s * SECTION_VOLUME..(s + 1) * SECTION_VOLUME];
        w.item_compound();
        w.byte("Y", ((ctx.min_y >> 4) + s as i32) as i8);
        palette.clear();
        for &b in section {
            if lookup[b as usize] == UNSET {
                lookup[b as usize] = palette.len() as u16;
                palette.push(b);
            }
        }
        w.list("Palette", TagType::Compound, palette.len());
        for &b in palette.iter() {
            w.item_compound();
            w.raw(ctx.palette_entries.get(b as usize));
        }
        let bits = bits_for(palette.len().max(2)).max(4);
        if ctx.era == Era::Flat16 {
            pack(section, |b| lookup[b as usize] as u64, bits, packed);
        } else {
            pack_spanning(section.iter().map(|&b| lookup[b as usize] as u64), SECTION_VOLUME, bits, packed);
        }
        w.long_array("BlockStates", packed);
        for &b in palette.iter() {
            lookup[b as usize] = UNSET;
        }
        w.end();
    }

    w.list("Entities", TagType::Compound, 0);
    w.list("TileEntities", TagType::Compound, 0);
    w.list("TileTicks", TagType::Compound, 0);
    w.list("LiquidTicks", TagType::Compound, 0);
    w.compound("Heightmaps");
    w.end();
    w.compound("Structures");
    w.compound("References");
    w.end();
    w.compound("Starts");
    w.end();
    w.end();
    w.end(); // Level
    w.finish()
}

/// How much of the sky's light a numbered block lets through: all of it
/// (air), some (water and ice dim it by three), none.
fn sky_dimming(id: u8) -> u8 {
    match id {
        // Air, and the plants and snow that let the light through.
        0 | 6 | 31 | 32 | 37..=40 | 78 | 81 | 83 | 106 | 111 | 171 | 175 => 0,
        18 | 161 => 1,
        8 | 9 | 79 => 3,
        _ => 15,
    }
}

/// A chunk as the versions up to 1.12 store it: numbered blocks with four
/// bits of data, a biome per column, and the light of the sky falling
/// straight down (the game spreads it sideways when it loads the chunk).
fn numbered_chunk(ctx: &ExportContext, blocks: &[u16], biome_columns: &[BiomeColumn], chunk_x: i32, chunk_z: i32) -> Vec<u8> {
    let height = (ctx.max_y - ctx.min_y) as usize;
    let numbered = |i: usize| ctx.numbered.get(blocks[i] as usize).copied().unwrap_or((1, 0));
    // Light of the sky and height of every column, from the top down.
    let mut sky = vec![0u8; height * 256];
    let mut heights = [0i32; 256];
    for (column, top) in heights.iter_mut().enumerate() {
        let mut light = 15u8;
        for y in (0..height).rev() {
            let i = (y << 8) | column;
            let dim = sky_dimming(numbered(i).0);
            if dim > 0 && *top == 0 {
                *top = y as i32 + 1;
            }
            light = light.saturating_sub(dim);
            sky[i] = light;
        }
    }

    let mut w = NbtWriter::with_capacity(48 * 1024);
    if ctx.target.data_version > 0 {
        w.int("DataVersion", ctx.target.data_version);
    }
    w.compound("Level");
    w.int("xPos", chunk_x);
    w.int("zPos", chunk_z);
    w.long("LastUpdate", 0);
    w.long("InhabitedTime", 0);
    w.bool("TerrainPopulated", true);
    w.bool("LightPopulated", false);
    let biomes: Vec<u8> = (0..256).map(|i| ctx.biome_number(biome_at(biome_columns, ((i >> 4) / 4) * 4 + (i & 15) / 4, ctx.sea_level)) as u8).collect();
    w.byte_array("Biomes", &biomes);
    w.int_array("HeightMap", &heights);

    let sections: Vec<usize> = (0..ctx.sections()).filter(|s| blocks[s * SECTION_VOLUME..(s + 1) * SECTION_VOLUME].iter().any(|&b| b != 0)).collect();
    w.list("Sections", TagType::Compound, sections.len());
    let mut ids = vec![0u8; SECTION_VOLUME];
    let mut data = vec![0u8; SECTION_VOLUME / 2];
    let mut sky_light = vec![0u8; SECTION_VOLUME / 2];
    let block_light = vec![0u8; SECTION_VOLUME / 2];
    for s in sections {
        data.fill(0);
        sky_light.fill(0);
        for k in 0..SECTION_VOLUME {
            let i = s * SECTION_VOLUME + k;
            let (id, d) = numbered(i);
            ids[k] = id;
            // Two blocks to a byte, the first in the low half.
            data[k / 2] |= (d & 15) << ((k & 1) * 4);
            sky_light[k / 2] |= sky[i] << ((k & 1) * 4);
        }
        w.item_compound();
        w.byte("Y", s as i8);
        w.byte_array("Blocks", &ids);
        w.byte_array("Data", &data);
        w.byte_array("BlockLight", &block_light);
        w.byte_array("SkyLight", &sky_light);
        w.end();
    }
    w.list("Entities", TagType::Compound, 0);
    w.list("TileEntities", TagType::Compound, 0);
    w.end(); // Level
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanning_packing_runs_end_to_end() {
        for bits in [4usize, 5, 6, 7, 9, 13] {
            let mask = (1u64 << bits) - 1;
            let values: Vec<u64> = (0..4096u64).map(|i| (i * 2_654_435_761) & mask).collect();
            let mut out = Vec::new();
            pack_spanning(values.iter().copied(), 4096, bits, &mut out);
            assert_eq!(out.len(), (4096 * bits).div_ceil(64), "{bits} bits");
            let read = |i: usize| {
                let (word, shift) = (i * bits / 64, i * bits % 64);
                let mut v = (out[word] as u64) >> shift;
                if shift + bits > 64 {
                    v |= (out[word + 1] as u64) << (64 - shift);
                }
                v & mask
            };
            assert!((0..4096).all(|i| read(i) == values[i]), "{bits} bits");
        }
    }

    fn unpack(longs: &[i64], count: usize, bits: usize) -> Vec<u64> {
        let per_long = 64 / bits;
        let mask = (1u64 << bits) - 1;
        (0..count)
            .map(|i| ((longs[i / per_long] as u64) >> ((i % per_long) * bits)) & mask)
            .collect()
    }

    #[test]
    fn bit_widths() {
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(3), 2);
        assert_eq!(bits_for(4), 2);
        assert_eq!(bits_for(5), 3);
        assert_eq!(bits_for(16), 4);
        assert_eq!(bits_for(17), 5);
        assert_eq!(bits_for(256), 8);
        assert_eq!(bits_for(257), 9);
    }

    #[test]
    fn packing_never_straddles_a_long() {
        for bits in [1usize, 2, 3, 4, 5, 6, 7, 9, 13] {
            let mask = (1u64 << bits) - 1;
            let values: Vec<u64> = (0..4096u64).map(|i| (i * 2_654_435_761) & mask).collect();
            let small: Vec<u16> = values.iter().map(|&v| v as u16).collect();
            let mut out = Vec::new();
            pack(&small, |v| v as u64, bits, &mut out);
            assert_eq!(out.len(), 4096usize.div_ceil(64 / bits), "{bits} bits");
            assert_eq!(unpack(&out, 4096, bits), values, "{bits} bits");
        }
        // Minecraft's own sizes: 4 bits -> 256 longs, 5 bits -> 342 longs.
        let mut out = Vec::new();
        pack(&[1; 4096], |v| v as u64, 4, &mut out);
        assert_eq!(out.len(), 256);
        pack(&[1; 4096], |v| v as u64, 5, &mut out);
        assert_eq!(out.len(), 342);
        pack(&[1; 64], |v| v as u64, 3, &mut out);
        assert_eq!(out.len(), 4);
    }
}
