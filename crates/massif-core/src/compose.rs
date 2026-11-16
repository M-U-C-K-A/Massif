//! Composition of the level stack into columns.
//!
//! For one column the stack resolves to a short, sorted list of solid
//! [`Span`]s — a run-length encoding of the column along Y — plus the
//! intervals filled with fluid. The exporter turns spans into blocks, the 3D
//! preview turns them into faces, the map shows the topmost one.

use crate::coords::TilePos;
use crate::level::{Level, LevelKind, TileRef};
use crate::registry::{BiomeId, BlockId, Registry, TerrainId};
use crate::tile::{NO_FLUID, Tile};
use crate::world::Dimension;

/// Block Y just above a surface of elevation `h` (round half up).
#[inline]
pub fn block_top(h: f32) -> i32 {
    (h + 0.5).floor() as i32
}

/// A vertical run of solid blocks: `y0 <= y < y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub y0: i32,
    pub y1: i32,
    /// Terrain recipe applied downward from `y1`.
    pub terrain: TerrainId,
    /// Continuous elevation of the upper surface (for shading).
    pub top: f32,
}

struct Layer<'a> {
    kind: LevelKind,
    index: usize,
    tile: TileRef<'a>,
}

/// Every level's tile at one tile position, ready to compose columns.
pub struct TileStack<'a> {
    pub min_y: i32,
    pub max_y: i32,
    layers: Vec<Layer<'a>>,
    /// Sculpted chunks over the tile, per column of chunks.
    sculpt: Option<Vec<Vec<(i32, &'a crate::voxel::VoxelChunk)>>>,
}

impl<'a> TileStack<'a> {
    /// Gather the tiles at `pos`. Returns `None` where the map has no ground
    /// tile. Hidden levels are skipped unless `include_hidden` is set.
    pub fn new(dim: &'a Dimension, pos: TilePos, include_hidden: bool) -> Option<Self> {
        Self::new_with(dim, pos, |_, level| level.visible || include_hidden)
    }

    /// [`TileStack::new`] with only the levels `keep` accepts (given their
    /// index and the level); the ground is always there.
    pub fn new_with(dim: &'a Dimension, pos: TilePos, keep: impl Fn(usize, &Level) -> bool) -> Option<Self> {
        let ground = dim.ground().view(pos)?;
        let mut layers = vec![(0, LevelKind::Ground, ground)];
        for (index, level) in dim.levels.iter().enumerate().skip(1) {
            if keep(index, level)
                && let Some(tile) = level.view(pos)
            {
                layers.push((index, level.kind, tile));
            }
        }
        Some(Self::gather(dim.min_y, dim.max_y, &dim.voxels, pos, layers))
    }

    /// A stack of the given tiles — `(level index, kind, tile)`, the
    /// ground's first, bottom of the stack first — with the chunks sculpted
    /// over `pos`.
    pub fn gather(min_y: i32, max_y: i32, voxels: &'a crate::voxel::VoxelLayer, pos: TilePos, layers: Vec<(usize, LevelKind, TileRef<'a>)>) -> Self {
        debug_assert!(layers.first().is_some_and(|l| l.1 == LevelKind::Ground), "a stack starts with the ground");
        Self {
            min_y,
            max_y,
            layers: layers.into_iter().map(|(index, kind, tile)| Layer { kind, index, tile }).collect(),
            sculpt: crate::voxel::tile_columns(voxels, pos),
        }
    }

    /// The level whose surface is the top of column `cell`, `top` being
    /// its topmost span: a slab's top, the floor a carve opened, or the
    /// ground's. `None` where none is (a top that was sculpted).
    pub fn surface_level(&self, cell: usize, top: &Span) -> Option<usize> {
        let mut found = None;
        for layer in &self.layers {
            let tile = &*layer.tile;
            let on_top = match layer.kind {
                LevelKind::Ground => block_top(tile.top.get(cell)) == top.y1,
                LevelKind::Solid => tile.present(cell) && block_top(tile.top.get(cell)) == top.y1,
                LevelKind::Carve => tile.present(cell) && tile.bottom.as_ref().is_some_and(|b| block_top(b.get(cell)) == top.y1),
            };
            if on_top {
                found = Some(layer.index);
            }
        }
        found
    }

    /// The cover painted on the top of column `cell`, `top` being its
    /// topmost span: that of the level whose surface it is, the ground's
    /// where none is (a top sculpted in 3D).
    pub fn surface_cover(&self, cell: usize, top: &Span) -> u16 {
        if self.is_ground_only() {
            return self.ground().cover.get(cell);
        }
        let level = self.surface_level(cell, top).unwrap_or(0);
        self.layers.iter().find(|l| l.index == level).map_or(0, |l| l.tile.cover.get(cell))
    }

    /// The object layer painted on the top of column `cell` (see
    /// [`TileStack::surface_cover`]).
    pub fn surface_objects(&self, cell: usize, top: &Span) -> u16 {
        if self.is_ground_only() {
            return self.ground().objects.get(cell);
        }
        let level = self.surface_level(cell, top).unwrap_or(0);
        self.layers.iter().find(|l| l.index == level).map_or(0, |l| l.tile.objects.get(cell))
    }

    /// True when only the ground level has data here (the common fast path).
    pub fn is_ground_only(&self) -> bool {
        self.layers.len() == 1 && self.sculpt.is_none()
    }

    /// Whether something was sculpted in 3D over this tile.
    pub fn is_sculpted(&self) -> bool {
        self.sculpt.is_some()
    }

    pub fn ground(&self) -> &Tile {
        &self.layers[0].tile
    }

    /// `(level index, kind, tile)` for each layer present, bottom of the
    /// stack first.
    pub fn layers(&self) -> impl Iterator<Item = (usize, LevelKind, &Tile)> {
        self.layers.iter().map(|l| (l.index, l.kind, &*l.tile))
    }

    /// Resolve the solid spans of column `cell`, sorted bottom-up.
    pub fn spans(&self, cell: usize, out: &mut Vec<Span>) {
        out.clear();
        let (min_y, max_y) = (self.min_y, self.max_y);
        let ground = self.ground();
        let top = ground.top.get(cell);
        let y1 = block_top(top).clamp(min_y, max_y);
        if y1 > min_y {
            out.push(Span {
                y0: min_y,
                y1,
                terrain: TerrainId(ground.terrain.get(cell)),
                top,
            });
        }
        for layer in &self.layers[1..] {
            let tile = &*layer.tile;
            let Some(bottom) = &tile.bottom else { continue };
            let (top, bottom) = (tile.top.get(cell), bottom.get(cell));
            if top <= bottom {
                continue;
            }
            let y0 = block_top(bottom).clamp(min_y, max_y);
            let y1 = block_top(top).clamp(min_y, max_y);
            if y1 <= y0 {
                continue;
            }
            let terrain = TerrainId(tile.terrain.get(cell));
            match layer.kind {
                LevelKind::Solid => {
                    // What the slab buries is plain rock from now on.
                    cut(out, y0, y1, TerrainId::ROCK);
                    let at = out.partition_point(|s| s.y0 < y0);
                    out.insert(
                        at,
                        Span {
                            y0,
                            y1,
                            terrain,
                            top,
                        },
                    );
                }
                // The carve's terrain paints the floor it exposes.
                LevelKind::Carve => cut(out, y0, y1, terrain),
                LevelKind::Ground => {}
            }
        }
        if let Some(sculpt) = &self.sculpt {
            let (lx, lz) = ((cell % 128) as i32, (cell / 128) as i32);
            let column = &sculpt[((lz / 16) * 8 + lx / 16) as usize];
            if !column.is_empty() {
                let surface = out.last().map_or(TerrainId(ground.terrain.get(cell)), |s| s.terrain);
                crate::voxel::apply(out, column, lx % 16, lz % 16, min_y, max_y, surface);
            }
        }
    }

    /// Resolve the fluid intervals `[y0, y1)` of column `cell`, sorted and
    /// disjoint, given its solid `spans`.
    ///
    /// Ground fluid fills the air above the ground surface, a solid level's
    /// fluid sits on top of its slab, and a carve's fluid floods the carved
    /// volume. Fluid never seeps into sealed cavities.
    pub fn fluids(&self, cell: usize, spans: &[Span], out: &mut Vec<(i32, i32)>) {
        out.clear();
        for layer in &self.layers {
            let tile = &*layer.tile;
            let level = tile.fluid.get(cell);
            if level == NO_FLUID {
                continue;
            }
            let level = (level as i32).clamp(self.min_y, self.max_y);
            let top = block_top(tile.top.get(cell));
            let (lo, hi) = match (&tile.bottom, layer.kind) {
                (None, _) | (_, LevelKind::Ground) => {
                    // Water rests on the ground surface, unless a carve
                    // opened that surface to the sky: then it fills the pit
                    // down to the next solid floor. A cave with a roof stays
                    // dry because the roof still covers the surface.
                    let covered = spans.iter().any(|s| s.y0 < top && s.y1 >= top);
                    let floor = if covered {
                        top
                    } else {
                        spans
                            .iter()
                            .filter(|s| s.y1 <= top)
                            .map(|s| s.y1)
                            .max()
                            .unwrap_or(self.min_y)
                    };
                    (floor, level)
                }
                (Some(bottom), kind) => {
                    let bottom = block_top(bottom.get(cell));
                    if top <= bottom {
                        continue;
                    }
                    if kind == LevelKind::Carve {
                        (bottom, level.min(top))
                    } else {
                        (top, level)
                    }
                }
            };
            let lo = lo.clamp(self.min_y, self.max_y);
            if hi <= lo {
                continue;
            }
            // Keep only the part of [lo, hi) that is not solid.
            let mut y = lo;
            for s in spans {
                if s.y1 <= y {
                    continue;
                }
                if s.y0 >= hi {
                    break;
                }
                if s.y0 > y {
                    out.push((y, s.y0));
                }
                y = y.max(s.y1);
            }
            if y < hi {
                out.push((y, hi));
            }
        }
        if out.len() > 1 {
            out.sort_unstable();
            let mut merged = 0;
            for i in 1..out.len() {
                if out[i].0 <= out[merged].1 {
                    out[merged].1 = out[merged].1.max(out[i].1);
                } else {
                    merged += 1;
                    out[merged] = out[i];
                }
            }
            out.truncate(merged + 1);
        }
    }

    /// Elevation and terrain of the topmost surface of column `cell`, or
    /// `None` if the column is entirely empty.
    pub fn surface(&self, cell: usize, scratch: &mut Vec<Span>) -> Option<(f32, TerrainId)> {
        if self.is_ground_only() {
            let g = self.ground();
            return Some((g.top.get(cell), TerrainId(g.terrain.get(cell))));
        }
        self.spans(cell, scratch);
        scratch.last().map(|s| (s.top, s.terrain))
    }

    /// The biomes the export writes over column `cell`: the one painted on
    /// the ground, or where it is left on automatic the one `auto` picks
    /// for the column's top (`top`, asked only then: the block over its
    /// highest run and that run's terrain, `None` for a column without
    /// ground), and those painted on the levels over it, by height.
    pub fn biome_column(&self, cell: usize, reg: &Registry, auto: &AutoBiomes, top: impl FnOnce() -> Option<(i32, TerrainId)>, out: &mut BiomeColumn) {
        out.overrides.clear();
        let ground = self.ground();
        out.base = ground.biome.get(cell);
        if out.base == 0 {
            let (surface, terrain) = top().unwrap_or((auto.floor, TerrainId::ROCK));
            out.base = auto.pick(reg, terrain, surface, ground.fluid.get(cell));
        }
        for (_, _, tile) in self.layers().skip(1) {
            let biome = tile.biome.get(cell);
            if biome == 0 || !tile.present(cell) {
                continue;
            }
            if let Some(bottom) = &tile.bottom {
                out.overrides.push((block_top(bottom.get(cell)), block_top(tile.top.get(cell)), biome));
            }
        }
    }
}

/// The biomes the export gives a column left on automatic: an ocean under
/// two blocks of water or more (a deep one past 24), else the biome its
/// terrain calls for (a beach for desert sand by the sea), plains without.
#[derive(Debug, Clone, Copy)]
pub struct AutoBiomes {
    plains: u16,
    ocean: u16,
    deep_ocean: u16,
    beach: u16,
    desert: u16,
    sea_level: i32,
    /// The surface of a column without ground: the floor of the world.
    floor: i32,
}

impl AutoBiomes {
    pub fn new(reg: &Registry, sea_level: i32, floor: i32) -> Self {
        let id = |name: &str| reg.biome_by_id(name).unwrap_or(BiomeId::AUTO).0;
        Self {
            plains: id("minecraft:plains"),
            ocean: id("minecraft:ocean"),
            deep_ocean: id("minecraft:deep_ocean"),
            beach: id("minecraft:beach"),
            desert: id("minecraft:desert"),
            sea_level,
            floor,
        }
    }

    /// The biome of a column on automatic whose surface (the block over its
    /// top) is at `surface`, of terrain `terrain`, its fluid up to `fluid`
    /// ([`NO_FLUID`]: none).
    pub fn pick(&self, reg: &Registry, terrain: TerrainId, surface: i32, fluid: i16) -> u16 {
        if fluid != NO_FLUID {
            let depth = fluid as i32 - surface;
            if depth > 24 {
                return self.deep_ocean;
            }
            if depth >= 2 {
                return self.ocean;
            }
        }
        match reg.terrain(terrain).biome_hint {
            // Sand along the shore is a beach, not a desert.
            Some(b) if b.0 == self.desert && surface <= self.sea_level + 4 => self.beach,
            Some(b) => b.0,
            None => self.plains,
        }
    }
}

/// The biomes the export writes over a column (see
/// [`TileStack::biome_column`]): `base`, save from `y0` up to `y1` where a
/// level over it has its own, the later levels last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BiomeColumn {
    pub base: u16,
    pub overrides: Vec<(i32, i32, u16)>,
}

impl BiomeColumn {
    /// The biome at height `y`.
    pub fn at(&self, y: i32) -> u16 {
        let mut id = self.base;
        for &(y0, y1, biome) in &self.overrides {
            if y >= y0 && y < y1 {
                id = biome;
            }
        }
        id
    }

    /// The biome of the block at height `y` as the game reads it since
    /// 1.15: that of its cell of 4 × 4 × 4, taken at the cell's middle.
    pub fn of_block(&self, y: i32) -> u16 {
        self.at(y.div_euclid(4) * 4 + 2)
    }
}

/// Remove `[y0, y1)` from sorted spans. The part of a span left *below* the
/// cut gets `floor` as its new surface terrain.
pub(crate) fn cut(spans: &mut Vec<Span>, y0: i32, y1: i32, floor: TerrainId) {
    let mut i = 0;
    while i < spans.len() {
        let s = spans[i];
        if s.y1 <= y0 || s.y0 >= y1 {
            i += 1;
            continue;
        }
        let below = (s.y0 < y0).then_some(Span {
            y0: s.y0,
            y1: y0,
            terrain: floor,
            top: y0 as f32,
        });
        let above = (s.y1 > y1).then_some(Span { y0: y1, ..s });
        match (below, above) {
            (Some(b), Some(a)) => {
                spans[i] = b;
                spans.insert(i + 1, a);
                i += 2;
            }
            (Some(one), None) | (None, Some(one)) => {
                spans[i] = one;
                i += 1;
            }
            (None, None) => {
                spans.remove(i);
            }
        }
    }
}

// ------------------------------------------------------------- materials --

#[inline]
fn mix64(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// Positional hash of a column.
#[inline]
pub fn hash2(x: i32, z: i32, seed: u64) -> u32 {
    let k = ((x as u32 as u64) << 32) | (z as u32 as u64);
    (mix64(k ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15)) >> 32) as u32
}

/// Positional hash of a block.
#[inline]
pub fn hash3(x: i32, y: i32, z: i32, seed: u64) -> u32 {
    hash3_at(hash3_column(x, z, seed), y)
}

/// The part of [`hash3`] that depends on the column alone, for the blocks
/// of a column one after the other: [`hash3_at`] finishes it.
#[inline]
pub fn hash3_column(x: i32, z: i32, seed: u64) -> u64 {
    let k = ((x as u32 as u64) << 32) | (z as u32 as u64);
    mix64(k ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// [`hash3`] at height `y` of the column of [`hash3_column`].
#[inline]
pub fn hash3_at(column: u64, y: i32) -> u32 {
    (mix64(column ^ (y as u32 as u64)) >> 32) as u32
}

/// [`Materializer::substrate`] down one column (see
/// [`Materializer::substrate_column`]).
pub struct SubstrateColumn {
    min_y: i32,
    /// The column's hashes for bedrock at the floor of the world and for
    /// deepslate turning into stone.
    floor: u64,
    deep: u64,
    /// Bedrock, deepslate, stone.
    blocks: [BlockId; 3],
}

impl SubstrateColumn {
    #[inline]
    pub fn at(&self, y: i32) -> BlockId {
        let above_floor = y - self.min_y;
        if above_floor < 5 && (above_floor <= 0 || (hash3_at(self.floor, y) % 5) as i32 >= above_floor) {
            return self.blocks[0];
        }
        if y < 0 || (y < 8 && (hash3_at(self.deep, y) % 8) as i32 >= y) { self.blocks[1] } else { self.blocks[2] }
    }
}

/// Turns spans into concrete blocks using the terrain recipes.
pub struct Materializer<'a> {
    reg: &'a Registry,
    seed: u64,
    min_y: i32,
}

impl<'a> Materializer<'a> {
    pub fn new(reg: &'a Registry, seed: i64, min_y: i32) -> Self {
        Self {
            reg,
            seed: seed as u64,
            min_y,
        }
    }

    /// Natural rock at a position: bedrock floor, deepslate below zero with a
    /// dithered transition up to Y = 8, stone above.
    #[inline]
    /// [`Materializer::substrate`] all along the column at `(x, z)`, its
    /// hashes begun once.
    pub fn substrate_column(&self, x: i32, z: i32) -> SubstrateColumn {
        let wk = &self.reg.well_known;
        SubstrateColumn {
            min_y: self.min_y,
            floor: hash3_column(x, z, self.seed ^ 0xbed),
            deep: hash3_column(x, z, self.seed ^ 0xdee9),
            blocks: [wk.bedrock, wk.deepslate, wk.stone],
        }
    }

    pub fn substrate(&self, x: i32, y: i32, z: i32) -> BlockId {
        let wk = &self.reg.well_known;
        let above_floor = y - self.min_y;
        if above_floor < 5
            && (above_floor <= 0
                || (hash3(x, y, z, self.seed ^ 0xbed) % 5) as i32 >= above_floor)
        {
            return wk.bedrock;
        }
        if y < 0 || (y < 8 && (hash3(x, y, z, self.seed ^ 0xdee9) % 8) as i32 >= y) {
            wk.deepslate
        } else {
            wk.stone
        }
    }

    /// The bands of heights `[y0, y1)` where [`Materializer::substrate`]
    /// gives the same block in every column, with that block; `None` where
    /// it is drawn by chance (bedrock at the floor of the world, deepslate
    /// turning into stone). Together they cover every height.
    pub fn substrate_bands(&self) -> [(i32, i32, Option<BlockId>); 4] {
        let wk = &self.reg.well_known;
        let floor = self.min_y + 5;
        [
            (i32::MIN, floor, None),
            (floor, 0, Some(wk.deepslate)),
            (floor.max(0), 8, None),
            (floor.max(8), i32::MAX, Some(wk.stone)),
        ]
    }

    /// Total thickness of the terrain recipe in column `(x, z)`: blocks deeper
    /// than this below the span's top are plain substrate.
    pub fn recipe_depth(&self, terrain: TerrainId, x: i32, z: i32) -> u32 {
        let def = self.reg.terrain(terrain);
        let h = hash2(x, z, self.seed);
        let mut total = 0u32;
        for (i, s) in def.strata.iter().enumerate() {
            total = total.saturating_add(stratum_thickness(s.depth, h, i));
            if s.depth.0 == u16::MAX {
                return u32::MAX;
            }
        }
        total
    }

    /// [`Materializer::block`] at every height of `span` from `lo` up to
    /// `hi` (both within the span), handed to `put`: the column's hash and
    /// strata worked out once rather than for every block.
    pub fn recipe_column(&self, span: &Span, x: i32, z: i32, submerged: bool, (lo, hi): (i32, i32), mut put: impl FnMut(i32, BlockId)) {
        let def = self.reg.terrain(span.terrain);
        let h = hash2(x, z, self.seed);
        let column = hash3_column(x, z, self.seed);
        let bedrock = self.reg.well_known.bedrock;
        let mut strata = def.strata.iter().enumerate();
        // The stratum the depth is in, and the depth where it ends.
        let mut current = None;
        let mut end = 0u32;
        for y in (lo..hi).rev() {
            let depth = (span.y1 - 1 - y).max(0) as u32;
            while depth >= end {
                match strata.next() {
                    Some((i, s)) => {
                        end = end.saturating_add(stratum_thickness(s.depth, h, i));
                        current = Some((i, s));
                    }
                    None => {
                        current = None;
                        end = u32::MAX;
                    }
                }
            }
            let block = match current {
                Some((i, s)) => {
                    if i == 0
                        && depth == 0
                        && submerged
                        && let Some(b) = def.submerged_top
                    {
                        b
                    } else if y - self.min_y < 5 && span.y0 == self.min_y && self.substrate(x, y, z) == bedrock {
                        bedrock
                    } else if s.blocks.len() == 1 {
                        s.blocks[0].0
                    } else {
                        pick(&s.blocks, hash3_at(column, y))
                    }
                }
                None => self.substrate(x, y, z),
            };
            put(y, block);
        }
    }

    /// The block at `y` inside `span`. `submerged` tells whether fluid lies
    /// directly on top of the span.
    pub fn block(&self, span: &Span, x: i32, y: i32, z: i32, submerged: bool) -> BlockId {
        let depth = (span.y1 - 1 - y).max(0) as u32;
        let def = self.reg.terrain(span.terrain);
        let h = hash2(x, z, self.seed);
        let mut acc = 0u32;
        for (i, s) in def.strata.iter().enumerate() {
            acc = acc.saturating_add(stratum_thickness(s.depth, h, i));
            if depth < acc {
                if i == 0
                    && depth == 0
                    && submerged
                    && let Some(b) = def.submerged_top
                {
                    return b;
                }
                // Bedrock still wins at the very bottom of the world.
                if y - self.min_y < 5 && span.y0 == self.min_y {
                    let b = self.substrate(x, y, z);
                    if b == self.reg.well_known.bedrock {
                        return b;
                    }
                }
                return pick(&s.blocks, hash3(x, y, z, self.seed));
            }
        }
        self.substrate(x, y, z)
    }
}

#[inline]
pub(crate) fn stratum_thickness(depth: (u16, u16), column_hash: u32, index: usize) -> u32 {
    let (lo, hi) = (depth.0 as u32, depth.1 as u32);
    if lo == u16::MAX as u32 {
        return u32::MAX;
    }
    if hi <= lo {
        return lo;
    }
    lo + column_hash.rotate_right(index as u32 * 7) % (hi - lo + 1)
}

#[inline]
pub(crate) fn pick(blocks: &[(BlockId, u16)], hash: u32) -> BlockId {
    if blocks.len() == 1 {
        return blocks[0].0;
    }
    let total: u32 = blocks.iter().map(|b| b.1 as u32).sum();
    let mut r = hash % total.max(1);
    for &(b, w) in blocks {
        if r < w as u32 {
            return b;
        }
        r -= w as u32;
    }
    blocks[0].0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::cell_index;
    use crate::level::LevelKind;
    use crate::world::DimensionKind;

    /// A column worked out at once is the same as block by block, for
    /// every terrain, over the sea or under it, down to the floor.
    #[test]
    fn a_recipe_column_is_its_blocks() {
        let reg = Registry::vanilla();
        let m = Materializer::new(&reg, 1234, -64);
        for (t, _) in reg.terrains.iter().enumerate() {
            for (x, z) in [(0, 0), (17, -40), (-300, 1201)] {
                for (y0, y1) in [(-64, 70), (40, 64), (-64, -60)] {
                    for submerged in [false, true] {
                        let span = Span { y0, y1, top: y1 as f32, terrain: TerrainId(t as u16) };
                        let mut got = Vec::new();
                        m.recipe_column(&span, x, z, submerged, (y0, y1), |y, b| got.push((y, b)));
                        let want: Vec<(i32, BlockId)> = (y0..y1).rev().map(|y| (y, m.block(&span, x, y, z, submerged))).collect();
                        assert_eq!(got, want, "terrain {t} at {x} {z}, {y0}..{y1}");
                    }
                }
            }
        }
    }

    const POS: TilePos = TilePos::new(0, 0);

    fn dim() -> Dimension {
        let mut d = Dimension::new(DimensionKind::Overworld, -64, 320);
        d.ground_mut().insert(POS, Tile::ground(64.0, 1, 63));
        d
    }

    fn add_slab(d: &mut Dimension, kind: LevelKind, cell: usize, bottom: f32, top: f32, terrain: u16) -> usize {
        let idx = d.add_level("slab", kind);
        let t = d.levels[idx].tile_mut_or_insert(POS, Tile::empty_slab);
        t.top.set(cell, top);
        t.bottom.as_mut().unwrap().set(cell, bottom);
        t.terrain.set(cell, terrain);
        idx
    }

    fn spans_at(d: &Dimension, cell: usize) -> Vec<(i32, i32, u16)> {
        let stack = TileStack::new(d, POS, true).unwrap();
        let mut out = Vec::new();
        stack.spans(cell, &mut out);
        out.iter().map(|s| (s.y0, s.y1, s.terrain.0)).collect()
    }

    #[test]
    fn rounding_is_half_up_on_both_sides_of_zero() {
        assert_eq!(block_top(64.0), 64);
        assert_eq!(block_top(63.5), 64);
        assert_eq!(block_top(63.49), 63);
        assert_eq!(block_top(-0.5), 0);
        assert_eq!(block_top(-0.51), -1);
        assert_eq!(block_top(-63.5), -63);
    }

    #[test]
    fn ground_only_column_is_one_span() {
        let d = dim();
        assert_eq!(spans_at(&d, 0), vec![(-64, 64, 1)]);
        assert!(TileStack::new(&d, TilePos::new(5, 5), true).is_none());
    }

    #[test]
    fn floating_slab_adds_a_second_surface() {
        let mut d = dim();
        let c = cell_index(3, 4);
        add_slab(&mut d, LevelKind::Solid, c, 90.0, 100.0, 5);
        assert_eq!(spans_at(&d, c), vec![(-64, 64, 1), (90, 100, 5)]);
        // Other columns are untouched.
        assert_eq!(spans_at(&d, c + 1), vec![(-64, 64, 1)]);
    }

    #[test]
    fn carve_splits_the_ground_into_floor_and_roof() {
        let mut d = dim();
        let c = cell_index(10, 10);
        add_slab(&mut d, LevelKind::Carve, c, 20.0, 30.0, 7);
        // Floor takes the carve's terrain, the roof keeps the grass on top.
        assert_eq!(spans_at(&d, c), vec![(-64, 20, 7), (30, 64, 1)]);
    }

    #[test]
    fn carve_open_to_the_sky_makes_a_pit() {
        let mut d = dim();
        let c = 0;
        add_slab(&mut d, LevelKind::Carve, c, 50.0, 80.0, 0);
        assert_eq!(spans_at(&d, c), vec![(-64, 50, 0)]);
    }

    #[test]
    fn later_levels_win() {
        let mut d = dim();
        let c = 9;
        add_slab(&mut d, LevelKind::Solid, c, 60.0, 100.0, 5);
        add_slab(&mut d, LevelKind::Carve, c, 70.0, 80.0, 0);
        add_slab(&mut d, LevelKind::Solid, c, 74.0, 76.0, 9);
        assert_eq!(
            spans_at(&d, c),
            vec![(-64, 60, 0), (60, 70, 0), (74, 76, 9), (80, 100, 5)]
        );
    }

    #[test]
    fn spans_respect_build_limits_and_visibility() {
        let mut d = dim();
        let c = 1;
        let idx = add_slab(&mut d, LevelKind::Solid, c, 300.0, 400.0, 5);
        assert_eq!(spans_at(&d, c), vec![(-64, 64, 1), (300, 320, 5)]);
        d.levels[idx].visible = false;
        let stack = TileStack::new(&d, POS, false).unwrap();
        let mut out = Vec::new();
        stack.spans(c, &mut out);
        assert_eq!(out.len(), 1);
        assert!(stack.is_ground_only());
    }

    #[test]
    fn fluid_fills_open_air_only() {
        let mut d = dim();
        // Sea bed at 50, sea level 63.
        d.ground_mut().tile_mut(POS).unwrap().top.fill(50.0);
        let c = 2;
        // A sealed cave under the sea bed stays dry...
        add_slab(&mut d, LevelKind::Carve, c, 10.0, 20.0, 0);
        // ...and a rock shelf inside the water column displaces water.
        add_slab(&mut d, LevelKind::Solid, c, 55.0, 58.0, 0);
        let stack = TileStack::new(&d, POS, true).unwrap();
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        stack.spans(c, &mut spans);
        stack.fluids(c, &spans, &mut fluids);
        assert_eq!(fluids, vec![(50, 55), (58, 63)]);

        // A pit carved open to the sky fills with sea water down to its floor.
        let pit = 3;
        let carve = d.level_index(d.levels[1].id).unwrap();
        {
            let t = d.levels[carve].tile_mut(POS).unwrap();
            t.top.set(pit, 80.0);
            t.bottom.as_mut().unwrap().set(pit, 30.0);
        }
        let stack = TileStack::new(&d, POS, true).unwrap();
        stack.spans(pit, &mut spans);
        assert_eq!(spans.iter().map(|s| (s.y0, s.y1)).collect::<Vec<_>>(), vec![(-64, 30)]);
        stack.fluids(pit, &spans, &mut fluids);
        assert_eq!(fluids, vec![(30, 63)]);

        // Flooding the cave is explicit: set the carve level's own fluid.
        d.levels[1].tile_mut(POS).unwrap().fluid.fill(15);
        let stack = TileStack::new(&d, POS, true).unwrap();
        stack.spans(c, &mut spans);
        stack.fluids(c, &spans, &mut fluids);
        assert_eq!(fluids, vec![(10, 15), (50, 55), (58, 63)]);
    }

    #[test]
    fn materials_follow_the_recipe() {
        let reg = Registry::vanilla();
        let grass = reg.terrain_by_name("Grass").unwrap();
        let m = Materializer::new(&reg, 42, -64);
        let span = Span {
            y0: -64,
            y1: 64,
            terrain: grass,
            top: 64.0,
        };
        let name = |y: i32, sub: bool| reg.block(m.block(&span, 5, y, 7, sub)).name().to_owned();
        assert_eq!(name(63, false), "minecraft:grass_block");
        assert_eq!(name(63, true), "minecraft:dirt");
        assert_eq!(name(62, false), "minecraft:dirt");
        assert_eq!(name(40, false), "minecraft:stone");
        assert_eq!(name(-20, false), "minecraft:deepslate");
        assert_eq!(name(-64, false), "minecraft:bedrock");
        let depth = m.recipe_depth(grass, 5, 7);
        assert!((3..=5).contains(&depth));
        assert_eq!(m.recipe_depth(TerrainId::ROCK, 5, 7), 0);
        assert_eq!(m.recipe_depth(reg.terrain_by_name("Stone").unwrap(), 5, 7), u32::MAX);
    }
}
