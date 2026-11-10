//! Turning the columns of one chunk into chunk NBT.
//!
//! The chunk layout is the one introduced by Minecraft 1.18 and still read by
//! every later version: 16³ sections holding a block-state palette with
//! bit-packed indices (Y, Z, X order, entries never straddle a `long`) and a
//! 4³ biome palette. Light is left for the game to compute (`isLightOn: 0`).

use crate::nbt::{NbtWriter, TagType};
use massif_core::compose::{Materializer, Span, TileStack, block_top};
use massif_core::coords::{TilePos, cell_index};
use massif_core::registry::{BiomeId, Registry, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::world::{Dimension, World};

/// Data version stamped on chunks and `level.dat`: Minecraft 1.18.2, the
/// oldest release with this chunk layout. Newer versions upgrade the data
/// automatically when they load it.
pub const DATA_VERSION: i32 = 2975;
pub const DATA_VERSION_NAME: &str = "1.18.2";

const SECTION_VOLUME: usize = 4096;
const UNSET: u16 = u16::MAX;

/// Everything that stays constant while a dimension is exported.
pub struct ExportContext<'a> {
    reg: &'a Registry,
    mat: Materializer<'a>,
    pub min_y: i32,
    pub max_y: i32,
    sea_level: i32,
    /// Block states split into name and properties, by `BlockId`.
    states: Vec<(String, Vec<(String, String)>)>,
    /// Biome identifiers by `BiomeId`.
    biomes: Vec<String>,
    water: u16,
    plains: u16,
    ocean: u16,
    deep_ocean: u16,
    beach: u16,
    desert: u16,
}

impl<'a> ExportContext<'a> {
    /// Fails with a description if the registry or the build limits cannot be
    /// exported.
    pub fn new(world: &'a World, dim: &Dimension) -> Result<Self, String> {
        let reg = &world.registry;
        if dim.min_y % 16 != 0 || dim.max_y % 16 != 0 || dim.max_y <= dim.min_y {
            return Err(format!(
                "build limits {}..{} are not multiples of 16",
                dim.min_y, dim.max_y
            ));
        }
        if dim.min_y < -2048 || dim.max_y > 2048 {
            return Err("build limits exceed what Minecraft supports (-2048..2048)".into());
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
        let states = reg
            .blocks
            .iter()
            .map(|b| {
                let props = b
                    .properties()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect();
                (b.name().to_owned(), props)
            })
            .collect();
        let biomes = reg
            .biomes
            .iter()
            .map(|b| {
                if b.id.is_empty() {
                    "minecraft:plains".to_owned()
                } else {
                    b.id.clone()
                }
            })
            .collect();
        let biome = |id: &str| reg.biome_by_id(id).unwrap_or(BiomeId::AUTO).0;
        Ok(Self {
            reg,
            mat: Materializer::new(reg, world.meta.seed, dim.min_y),
            min_y: dim.min_y,
            max_y: dim.max_y,
            sea_level: world.meta.sea_level,
            states,
            biomes,
            water: wk.water.0,
            plains: biome("minecraft:plains"),
            ocean: biome("minecraft:ocean"),
            deep_ocean: biome("minecraft:deep_ocean"),
            beach: biome("minecraft:beach"),
            desert: biome("minecraft:desert"),
        })
    }

    pub fn sections(&self) -> usize {
        ((self.max_y - self.min_y) / 16) as usize
    }

    pub fn materializer(&self) -> &Materializer<'a> {
        &self.mat
    }

    /// The biome of a column whose biome plane says "automatic".
    fn auto_biome(&self, terrain: TerrainId, surface: i32, fluid: i16) -> u16 {
        if fluid != NO_FLUID {
            let depth = fluid as i32 - surface;
            if depth > 24 {
                return self.deep_ocean;
            }
            if depth >= 2 {
                return self.ocean;
            }
        }
        match self.reg.terrain(terrain).biome_hint {
            // Sand along the shore is a beach, not a desert.
            Some(b) if b.0 == self.desert && surface <= self.sea_level + 4 => self.beach,
            Some(b) => b.0,
            None => self.plains,
        }
    }

    fn biome_name(&self, id: u16) -> &str {
        self.biomes
            .get(id as usize)
            .map(String::as_str)
            .unwrap_or("minecraft:plains")
    }
}

#[derive(Default, Clone)]
struct BiomeColumn {
    base: u16,
    /// `(y0, y1, biome)` painted on slab levels, later levels last.
    overrides: Vec<(i32, i32, u16)>,
}

/// Reusable buffers for [`build_chunk`]; keep one per worker thread.
pub struct Scratch {
    blocks: Vec<u16>,
    lookup: Vec<u16>,
    palette: Vec<u16>,
    packed: Vec<i64>,
    spans: Vec<Span>,
    fluids: Vec<(i32, i32)>,
    biome_columns: Vec<BiomeColumn>,
}

impl Scratch {
    pub fn new(ctx: &ExportContext) -> Self {
        Self {
            blocks: vec![0; ctx.sections() * SECTION_VOLUME],
            lookup: vec![UNSET; ctx.states.len()],
            palette: Vec::with_capacity(64),
            packed: Vec::with_capacity(512),
            spans: Vec::with_capacity(8),
            fluids: Vec::with_capacity(4),
            biome_columns: vec![BiomeColumn::default(); 16],
        }
    }
}

#[inline]
fn bits_for(palette_len: usize) -> usize {
    (usize::BITS - (palette_len - 1).leading_zeros()) as usize
}

/// Pack `values` into longs, `bits` per entry, never crossing a long.
fn pack(values: impl Iterator<Item = u64>, count: usize, bits: usize, out: &mut Vec<i64>) {
    let per_long = 64 / bits;
    out.clear();
    out.resize(count.div_ceil(per_long), 0);
    let (mut word, mut shift, mut filled) = (0usize, 0usize, 0usize);
    for v in values {
        out[word] |= (v << shift) as i64;
        filled += 1;
        if filled == per_long {
            filled = 0;
            shift = 0;
            word += 1;
        } else {
            shift += bits;
        }
    }
}

/// Build the NBT of the chunk at chunk position `(cx, cz)` *within* the tile
/// (both in `0..8`).
pub fn build_chunk(
    ctx: &ExportContext,
    stack: &TileStack,
    tile: TilePos,
    cx: usize,
    cz: usize,
    scratch: &mut Scratch,
) -> Vec<u8> {
    let (ox, oz) = tile.origin();
    let (bx0, bz0) = (ox + cx as i32 * 16, oz + cz as i32 * 16);
    let min_y = ctx.min_y;
    let sections = ctx.sections();
    let Scratch {
        blocks,
        lookup,
        palette,
        packed,
        spans,
        fluids,
        biome_columns,
    } = scratch;

    // ---- blocks ------------------------------------------------------------
    blocks.fill(0);
    for lz in 0..16usize {
        for lx in 0..16usize {
            let cell = cell_index(cx * 16 + lx, cz * 16 + lz);
            let (x, z) = (bx0 + lx as i32, bz0 + lz as i32);
            let column = (lz << 4) | lx;
            let at = |y: i32| (((y - min_y) as usize) << 8) | column;
            stack.spans(cell, spans);
            stack.fluids(cell, spans, fluids);
            for span in spans.iter() {
                let submerged = fluids.iter().any(|f| f.0 == span.y1);
                let recipe = ctx.mat.recipe_depth(span.terrain, x, z);
                let split = if recipe >= (span.y1 - span.y0) as u32 {
                    span.y0
                } else {
                    span.y1 - recipe as i32
                };
                for y in span.y0..split {
                    blocks[at(y)] = ctx.mat.substrate(x, y, z).0;
                }
                for y in split..span.y1 {
                    blocks[at(y)] = ctx.mat.block(span, x, y, z, submerged).0;
                }
            }
            for &(y0, y1) in fluids.iter() {
                for y in y0..y1 {
                    blocks[at(y)] = ctx.water;
                }
            }
        }
    }

    // ---- biomes: one sample per 4×4 column ---------------------------------
    for bz in 0..4usize {
        for bx in 0..4usize {
            let cell = cell_index(cx * 16 + bx * 4 + 2, cz * 16 + bz * 4 + 2);
            let column = &mut biome_columns[bz * 4 + bx];
            column.overrides.clear();
            let ground = stack.ground();
            column.base = ground.biome.get(cell);
            if column.base == 0 {
                stack.spans(cell, spans);
                let (surface, terrain) = spans
                    .last()
                    .map_or((min_y, TerrainId::ROCK), |s| (s.y1, s.terrain));
                column.base = ctx.auto_biome(terrain, surface, ground.fluid.get(cell));
            }
            for (_, _, tile) in stack.layers().skip(1) {
                let biome = tile.biome.get(cell);
                if biome == 0 || !tile.present(cell) {
                    continue;
                }
                if let Some(bottom) = &tile.bottom {
                    let y0 = block_top(bottom.get(cell));
                    let y1 = block_top(tile.top.get(cell));
                    column.overrides.push((y0, y1, biome));
                }
            }
        }
    }

    // ---- NBT ---------------------------------------------------------------
    let mut w = NbtWriter::with_capacity(32 * 1024);
    w.int("DataVersion", DATA_VERSION);
    w.int("xPos", bx0 >> 4);
    w.int("yPos", min_y >> 4);
    w.int("zPos", bz0 >> 4);
    w.string("Status", "full");
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
        if section.iter().all(|&b| b == first) {
            palette.push(first);
        } else {
            for &b in section {
                if lookup[b as usize] == UNSET {
                    lookup[b as usize] = palette.len() as u16;
                    palette.push(b);
                }
            }
        }
        w.compound("block_states");
        w.list("palette", TagType::Compound, palette.len());
        for &b in palette.iter() {
            let (name, props) = &ctx.states[b as usize];
            w.item_compound();
            w.string("Name", name);
            if !props.is_empty() {
                w.compound("Properties");
                for (k, v) in props {
                    w.string(k, v);
                }
                w.end();
            }
            w.end();
        }
        if palette.len() > 1 {
            let bits = bits_for(palette.len()).max(4);
            pack(
                section.iter().map(|&b| lookup[b as usize] as u64),
                SECTION_VOLUME,
                bits,
                packed,
            );
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
                let mut id = column.base;
                for &(y0, y1, biome) in &column.overrides {
                    if y >= y0 && y < y1 {
                        id = biome;
                    }
                }
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
            w.item_string(ctx.biome_name(id));
        }
        if biome_palette.len() > 1 {
            let bits = bits_for(biome_palette.len());
            pack(biome_ids.iter().map(|&i| i as u64), 64, bits, packed);
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

#[cfg(test)]
mod tests {
    use super::*;

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
            let mut out = Vec::new();
            pack(values.iter().copied(), 4096, bits, &mut out);
            assert_eq!(out.len(), 4096usize.div_ceil(64 / bits), "{bits} bits");
            assert_eq!(unpack(&out, 4096, bits), values, "{bits} bits");
        }
        // Minecraft's own sizes: 4 bits -> 256 longs, 5 bits -> 342 longs.
        let mut out = Vec::new();
        pack(std::iter::repeat_n(1, 4096), 4096, 4, &mut out);
        assert_eq!(out.len(), 256);
        pack(std::iter::repeat_n(1, 4096), 4096, 5, &mut out);
        assert_eq!(out.len(), 342);
        pack(std::iter::repeat_n(1, 64), 64, 3, &mut out);
        assert_eq!(out.len(), 4);
    }
}
