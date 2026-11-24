//! True 3D: sculpted volumes.
//!
//! Most of a world is columns: the level stack composes every column into a
//! few solid spans, which is what keeps maps of tens of thousands of blocks
//! cheap. Where the world is sculpted in 3D — caves of any shape, overhangs
//! past the vertical, cliff edges rounded off — the dimension keeps a *voxel
//! chunk* instead: a 16³ block of densities that replaces whatever the levels
//! compose inside it. A voxel is solid when its density is at least half
//! ([`SOLID`]); the values in between let smoothing round shapes off below
//! the scale of a block.
//!
//! A chunk starts as an exact copy of what the levels compose there (with the
//! fractional height of the surface as density), so sculpting never changes
//! anything it does not touch.

use crate::brush::Brush;
use crate::codec;
use crate::compose::{Span, TileStack, block_top};
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::history::Edit;
use crate::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Edge of a voxel chunk, in blocks.
pub const SIZE: i32 = 16;
/// Voxels in a chunk.
pub const VOLUME: usize = (SIZE * SIZE * SIZE) as usize;
/// Density from which a voxel is solid.
pub const SOLID: u8 = 128;

/// The widest a brush sculpts in 3D, in blocks of radius: the work and
/// the memory of a dab grow with the cube of the brush (a brush of the
/// map's size would want terabytes), and 64 already moves a hill.
pub const MAX_SCULPT_RADIUS: f32 = 64.0;

/// `brush`, no wider than [`MAX_SCULPT_RADIUS`].
fn within_reach(brush: &Brush) -> std::borrow::Cow<'_, Brush> {
    if brush.radius > MAX_SCULPT_RADIUS {
        std::borrow::Cow::Owned(Brush {
            radius: MAX_SCULPT_RADIUS,
            ..brush.clone()
        })
    } else {
        std::borrow::Cow::Borrowed(brush)
    }
}
/// Chunks per tile side.
const PER_TILE: i32 = TILE_SIZE / SIZE;

/// Position of a voxel chunk, in chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChunkPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk holding block `(x, y, z)`.
    pub const fn containing(x: i32, y: i32, z: i32) -> Self {
        Self {
            x: x.div_euclid(SIZE),
            y: y.div_euclid(SIZE),
            z: z.div_euclid(SIZE),
        }
    }

    /// Block coordinates of the chunk's lowest corner.
    pub const fn origin(self) -> (i32, i32, i32) {
        (self.x * SIZE, self.y * SIZE, self.z * SIZE)
    }

    /// The tile whose columns the chunk lies in.
    pub const fn tile(self) -> TilePos {
        TilePos::new(self.x.div_euclid(PER_TILE), self.z.div_euclid(PER_TILE))
    }
}

/// Index of the voxel `(lx, ly, lz)` of a chunk, all in `0..16`.
#[inline]
pub const fn voxel_index(lx: i32, ly: i32, lz: i32) -> usize {
    ((ly * SIZE + lz) * SIZE + lx) as usize
}

/// The densities of a 16³ block of the world, and the terrain of the matter
/// sculpting moved there.
#[derive(Clone, PartialEq, Eq)]
pub struct VoxelChunk {
    density: Box<[u8; VOLUME]>,
    /// The terrain of each voxel whose matter was brought by sculpting —
    /// what it was made of where it came from —, or [`FOLLOW`]: as the
    /// column composes it. `None` while every voxel follows.
    pub(crate) terrain: Option<Box<[u16; VOLUME]>>,
}

/// A voxel whose terrain is the column's (see [`VoxelChunk`]).
pub(crate) const FOLLOW: u16 = u16::MAX;
/// Air voxels holding water or lava keep it in the plane of terrains (which
/// solid voxels alone read): an underground lake. Older versions read them
/// as air.
const WATER_MARK: u16 = u16::MAX - 1;
const LAVA_MARK: u16 = u16::MAX - 2;

/// What fills an air voxel of a sculpted chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Fluid {
    Water,
    Lava,
}

impl std::fmt::Debug for VoxelChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let solid = self.density.iter().filter(|&&d| d >= SOLID).count();
        write!(f, "VoxelChunk({solid} solid)")
    }
}

impl VoxelChunk {
    /// A chunk of air.
    pub fn air() -> Self {
        Self {
            density: Box::new([0; VOLUME]),
            terrain: None,
        }
    }

    /// The terrain the matter of a voxel brought with it; `None`: the
    /// column's.
    #[inline]
    pub fn terrain(&self, lx: i32, ly: i32, lz: i32) -> Option<crate::registry::TerrainId> {
        let t = self.terrain.as_ref()?[voxel_index(lx, ly, lz)];
        (t < LAVA_MARK).then_some(crate::registry::TerrainId(t))
    }

    /// The fluid an air voxel holds (none in a solid one).
    #[inline]
    pub fn fluid(&self, lx: i32, ly: i32, lz: i32) -> Option<Fluid> {
        let plane = self.terrain.as_ref()?;
        let i = voxel_index(lx, ly, lz);
        if self.density[i] >= SOLID {
            return None;
        }
        match plane[i] {
            WATER_MARK => Some(Fluid::Water),
            LAVA_MARK => Some(Fluid::Lava),
            _ => None,
        }
    }

    /// Fill a voxel with a fluid (`None`: with nothing), shown while it is
    /// air.
    pub fn set_fluid(&mut self, lx: i32, ly: i32, lz: i32, fluid: Option<Fluid>) {
        if fluid.is_none() && self.terrain.is_none() {
            return;
        }
        let plane = self.terrain.get_or_insert_with(|| Box::new([FOLLOW; VOLUME]));
        plane[voxel_index(lx, ly, lz)] = match fluid {
            Some(Fluid::Water) => WATER_MARK,
            Some(Fluid::Lava) => LAVA_MARK,
            None => FOLLOW,
        };
    }

    /// Whether some voxel holds a fluid.
    pub fn has_fluid(&self) -> bool {
        self.terrain.as_ref().is_some_and(|p| p.iter().zip(self.density.iter()).any(|(&t, &d)| d < SOLID && (t == WATER_MARK || t == LAVA_MARK)))
    }

    /// Give a voxel a terrain of its own (`None`: the column's).
    pub fn set_terrain(&mut self, lx: i32, ly: i32, lz: i32, terrain: Option<crate::registry::TerrainId>) {
        if terrain.is_none() && self.terrain.is_none() {
            return;
        }
        let plane = self.terrain.get_or_insert_with(|| Box::new([FOLLOW; VOLUME]));
        plane[voxel_index(lx, ly, lz)] = terrain.map_or(FOLLOW, |t| t.0);
    }

    #[inline]
    pub fn get(&self, lx: i32, ly: i32, lz: i32) -> u8 {
        self.density[voxel_index(lx, ly, lz)]
    }

    #[inline]
    pub fn set(&mut self, lx: i32, ly: i32, lz: i32, density: u8) {
        self.density[voxel_index(lx, ly, lz)] = density;
    }

    #[inline]
    pub fn solid(&self, lx: i32, ly: i32, lz: i32) -> bool {
        self.get(lx, ly, lz) >= SOLID
    }

    pub fn densities(&self) -> &[u8; VOLUME] {
        &self.density
    }

    pub fn from_densities(density: [u8; VOLUME]) -> Self {
        Self {
            density: Box::new(density),
            terrain: None,
        }
    }

    pub fn solid_count(&self) -> usize {
        self.density.iter().filter(|&&d| d >= SOLID).count()
    }

    /// Whether it holds nothing but `bare`, what the levels compose there
    /// (whose voxels all follow their columns): no matter of its own, no
    /// terrain, no fluid.
    pub fn is_bare(&self, bare: &VoxelChunk) -> bool {
        self.density == bare.density && self.terrain.as_ref().is_none_or(|plane| plane.iter().all(|&t| t == FOLLOW))
    }

    /// What a voxel's matter is marked with: the terrain it brought, the
    /// fluid it holds, or [`FOLLOW`] (as its column composes it).
    pub(crate) fn mark(&self, lx: i32, ly: i32, lz: i32) -> u16 {
        self.terrain.as_ref().map_or(FOLLOW, |plane| plane[voxel_index(lx, ly, lz)])
    }

    /// Mark a voxel's matter as [`VoxelChunk::mark`] reads it.
    pub(crate) fn set_mark(&mut self, lx: i32, ly: i32, lz: i32, mark: u16) {
        if mark == FOLLOW && self.terrain.is_none() {
            return;
        }
        self.terrain.get_or_insert_with(|| Box::new([FOLLOW; VOLUME]))[voxel_index(lx, ly, lz)] = mark;
    }
}

/// The sculpted chunks of a dimension.
#[derive(Debug, Clone, Default)]
pub struct VoxelLayer {
    chunks: HashMap<ChunkPos, Arc<VoxelChunk>>,
    /// The chunks over each tile, for composing a tile's columns.
    by_tile: HashMap<TilePos, BTreeSet<ChunkPos>>,
    /// Tiles whose columns changed since the last [`VoxelLayer::take_dirty`].
    dirty: HashSet<TilePos>,
}

impl VoxelLayer {
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn get(&self, pos: ChunkPos) -> Option<&VoxelChunk> {
        self.chunks.get(&pos).map(|c| &**c)
    }

    /// The shared chunk at `pos` (for undo and saving).
    pub fn get_shared(&self, pos: ChunkPos) -> Option<Arc<VoxelChunk>> {
        self.chunks.get(&pos).cloned()
    }

    /// Mutable access, copying the chunk first if it is shared.
    pub fn get_mut(&mut self, pos: ChunkPos) -> Option<&mut VoxelChunk> {
        let chunk = self.chunks.get_mut(&pos)?;
        self.dirty.insert(pos.tile());
        Some(Arc::make_mut(chunk))
    }

    pub fn insert(&mut self, pos: ChunkPos, chunk: Arc<VoxelChunk>) {
        self.chunks.insert(pos, chunk);
        self.by_tile.entry(pos.tile()).or_default().insert(pos);
        self.dirty.insert(pos.tile());
    }

    pub fn remove(&mut self, pos: ChunkPos) -> Option<Arc<VoxelChunk>> {
        let removed = self.chunks.remove(&pos)?;
        let tile = pos.tile();
        if let Some(set) = self.by_tile.get_mut(&tile) {
            set.remove(&pos);
            if set.is_empty() {
                self.by_tile.remove(&tile);
            }
        }
        self.dirty.insert(tile);
        Some(removed)
    }

    /// Put back a chunk as it was (`None`: no chunk).
    pub fn restore(&mut self, pos: ChunkPos, state: Option<Arc<VoxelChunk>>) {
        match state {
            Some(chunk) => self.insert(pos, chunk),
            None => {
                self.remove(pos);
            }
        }
    }

    /// Whether any chunk lies over the tile.
    pub fn has_tile(&self, tile: TilePos) -> bool {
        self.by_tile.contains_key(&tile)
    }

    /// The chunks over a tile, sorted by x, y, z.
    pub fn in_tile(&self, tile: TilePos) -> impl Iterator<Item = (ChunkPos, &VoxelChunk)> {
        self.by_tile
            .get(&tile)
            .into_iter()
            .flatten()
            .map(|p| (*p, &*self.chunks[p]))
    }

    pub fn iter(&self) -> impl Iterator<Item = (ChunkPos, &Arc<VoxelChunk>)> {
        self.chunks.iter().map(|(p, c)| (*p, c))
    }

    pub fn take_dirty(&mut self) -> Vec<TilePos> {
        self.dirty.drain().collect()
    }

    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Take chunks out to edit them in parallel; give them back with
    /// [`VoxelLayer::put_back`]. Missing positions are skipped.
    pub(crate) fn take_out(&mut self, positions: &[ChunkPos]) -> Vec<(ChunkPos, Arc<VoxelChunk>)> {
        positions
            .iter()
            .filter_map(|p| self.chunks.remove(p).map(|c| (*p, c)))
            .collect()
    }

    pub(crate) fn put_back(&mut self, chunks: Vec<(ChunkPos, Arc<VoxelChunk>)>) {
        for (pos, chunk) in chunks {
            self.dirty.insert(pos.tile());
            self.chunks.insert(pos, chunk);
        }
    }

    /// Every tile with chunks needs drawing again (after loading or undoing
    /// a structural change).
    pub fn mark_all_dirty(&mut self) {
        let tiles: Vec<TilePos> = self.by_tile.keys().copied().collect();
        self.dirty.extend(tiles);
    }

    /// Serialize every chunk, zstd'd: `VOX2`, then for each `x y z` (i32),
    /// the densities, and a byte saying whether the terrains of its voxels
    /// follow (16-bit each, [`FOLLOW`] for the column's).
    pub fn encode(&self) -> Vec<u8> {
        let mut positions: Vec<&ChunkPos> = self.chunks.keys().collect();
        positions.sort();
        let mut raw = Vec::with_capacity(4 + positions.len() * (13 + VOLUME));
        raw.extend_from_slice(VOXELS_2);
        for p in positions {
            let chunk = &self.chunks[p];
            raw.extend_from_slice(&p.x.to_le_bytes());
            raw.extend_from_slice(&p.y.to_le_bytes());
            raw.extend_from_slice(&p.z.to_le_bytes());
            raw.extend_from_slice(&chunk.density[..]);
            match &chunk.terrain {
                Some(t) => {
                    raw.push(1);
                    for v in t.iter() {
                        raw.extend_from_slice(&v.to_le_bytes());
                    }
                }
                None => raw.push(0),
            }
        }
        codec::zstd_compress(&raw)
    }

    /// Read what [`VoxelLayer::encode`] wrote (or, before terrains were
    /// kept, densities alone).
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let raw = codec::zstd_decompress_section(bytes).map_err(|e| e.to_string())?;
        let mut layer = Self::default();
        let position = |e: &[u8]| {
            let at = |i: usize| i32::from_le_bytes(e[i..i + 4].try_into().expect("4 bytes"));
            ChunkPos::new(at(0), at(4), at(8))
        };
        if let Some(mut rest) = raw.strip_prefix(VOXELS_2) {
            while !rest.is_empty() {
                if rest.len() < 13 + VOLUME {
                    return Err("a sculpted chunk is cut short".into());
                }
                let pos = position(rest);
                let density: [u8; VOLUME] = rest[12..12 + VOLUME].try_into().expect("a full chunk");
                let mut chunk = VoxelChunk::from_densities(density);
                let has_terrain = rest[12 + VOLUME] == 1;
                rest = &rest[13 + VOLUME..];
                if has_terrain {
                    if rest.len() < 2 * VOLUME {
                        return Err("the terrains of a sculpted chunk are cut short".into());
                    }
                    let mut plane = Box::new([FOLLOW; VOLUME]);
                    for (k, v) in plane.iter_mut().enumerate() {
                        *v = u16::from_le_bytes([rest[2 * k], rest[2 * k + 1]]);
                    }
                    chunk.terrain = Some(plane);
                    rest = &rest[2 * VOLUME..];
                }
                layer.insert(pos, Arc::new(chunk));
            }
        } else {
            let entry = 12 + VOLUME;
            if raw.len() % entry != 0 {
                return Err("the sculpted chunks have a ragged length".into());
            }
            for e in raw.chunks(entry) {
                let density: [u8; VOLUME] = e[12..].try_into().expect("a full chunk");
                layer.insert(position(e), Arc::new(VoxelChunk::from_densities(density)));
            }
        }
        layer.dirty.clear();
        Ok(layer)
    }
}

// ------------------------------------------------------------ composing --

/// The sculpted chunks of a file since terrains are kept with them.
const VOXELS_2: &[u8] = b"VOX2";

/// Mark for spans made by sculpting while a column is composed; resolved
/// before [`apply`] returns.
const SCULPTED: crate::registry::TerrainId = crate::registry::TerrainId(u16::MAX);

/// Replace what `spans` holds inside each chunk of `column` (chunk y, chunk;
/// bottom up) by the chunk's solid voxels at local column `(lx, lz)`.
///
/// Matter that sculpting brought keeps the terrain it was made of where it
/// came from. Other sculpted rock open to the sky takes the terrain of the
/// column's surface; under something else it is plain rock, as under a
/// solid level.
pub(crate) fn apply(spans: &mut Vec<Span>, column: &[(i32, &VoxelChunk)], lx: i32, lz: i32, min_y: i32, max_y: i32, surface: crate::registry::TerrainId) {
    // The exact heights of the tops the levels had, for runs that end where
    // one did and whose block above is not sculpted.
    let before: Vec<(i32, f32, crate::registry::TerrainId)> = spans.iter().map(|s| (s.y1, s.top, s.terrain)).collect();
    for (n, &(cy, chunk)) in column.iter().enumerate() {
        let base = cy * SIZE;
        let (ya, yb) = (base.max(min_y), (base + SIZE).min(max_y));
        if yb <= ya {
            continue;
        }
        crate::compose::cut(spans, ya, yb, SCULPTED);
        let mut y = ya;
        while y < yb {
            if !chunk.solid(lx, y - base, lz) {
                y += 1;
                continue;
            }
            // A run of one terrain: its own, or the column's.
            let start = y;
            let own = chunk.terrain(lx, y - base, lz);
            while y < yb && chunk.solid(lx, y - base, lz) && chunk.terrain(lx, y - base, lz) == own {
                y += 1;
            }
            // Its top at the height its densities give: the matter of its
            // last block and of the one above, as `densities_at` lays a
            // fractional surface into them — so that sculpting keeps the
            // heights it does not change, to the 1/256 of a block.
            let above = if y < base + SIZE {
                Some(chunk.get(lx, y - base, lz))
            } else {
                column.get(n + 1).filter(|(next, _)| *next == cy + 1).map(|(_, c)| c.get(lx, 0, lz))
            };
            let top = match above {
                Some(above) => (y - 1) as f32 + (chunk.get(lx, y - 1 - base, lz) as f32 + above as f32) / 255.0,
                None => before.iter().find(|b| b.0 == y).map_or(y as f32, |b| b.1),
            };
            let at = spans.partition_point(|s| s.y0 < start);
            spans.insert(
                at,
                Span {
                    y0: start,
                    y1: y,
                    terrain: own.unwrap_or(SCULPTED),
                    top: top.clamp(y as f32 - 0.5, y as f32 + 0.499),
                },
            );
        }
    }
    // Sculpted runs continue the spans they touch, and runs of one terrain
    // make one span: its layers go on down from its top.
    let mut i = 0;
    while i + 1 < spans.len() {
        let (a, b) = (spans[i], spans[i + 1]);
        if a.y1 == b.y0 && (a.terrain == SCULPTED || b.terrain == SCULPTED || a.terrain == b.terrain) {
            spans[i] = Span { y0: a.y0, ..b };
            spans.remove(i + 1);
        } else {
            i += 1;
        }
    }
    // What follows the column: the terrain of the surface it still is (the
    // ground under a ledge keeps its own), the column's surface on top,
    // rock where sculpting opened the inside of the matter.
    let last = spans.len().saturating_sub(1);
    for (i, s) in spans.iter_mut().enumerate() {
        if s.terrain == SCULPTED {
            s.terrain = match before.iter().find(|b| b.0 == s.y1) {
                Some(b) if b.2 != SCULPTED => b.2,
                _ if i == last => surface,
                _ => crate::registry::TerrainId::ROCK,
            };
        }
    }
}

/// The runs of fluid the chunks of `column` (chunk y, chunk; bottom up)
/// hold at local column `(lx, lz)`, as `(y0, y1, fluid)`, bottom up.
pub(crate) fn fluid_runs(column: &[(i32, &VoxelChunk)], lx: i32, lz: i32, min_y: i32, max_y: i32, out: &mut Vec<(i32, i32, Fluid)>) {
    out.clear();
    for &(cy, chunk) in column {
        if chunk.terrain.is_none() {
            continue;
        }
        let base = cy * SIZE;
        for ly in 0..SIZE {
            let y = base + ly;
            if y < min_y || y >= max_y {
                continue;
            }
            let Some(f) = chunk.fluid(lx, ly, lz) else { continue };
            match out.last_mut() {
                Some(last) if last.1 == y && last.2 == f => last.1 = y + 1,
                _ => out.push((y, y + 1, f)),
            }
        }
    }
}

/// The chunks over a tile, per 16 × 16 column of chunks (`lz / 16 * 8 + lx /
/// 16`), bottom up. `None` when nothing was sculpted there.
pub(crate) fn tile_columns(layer: &VoxelLayer, tile: TilePos) -> Option<Vec<Vec<(i32, &VoxelChunk)>>> {
    if !layer.has_tile(tile) {
        return None;
    }
    let mut columns: Vec<Vec<(i32, &VoxelChunk)>> = vec![Vec::new(); (PER_TILE * PER_TILE) as usize];
    for (pos, chunk) in layer.in_tile(tile) {
        let (cx, cz) = (pos.x.rem_euclid(PER_TILE), pos.z.rem_euclid(PER_TILE));
        columns[(cz * PER_TILE + cx) as usize].push((pos.y, chunk));
    }
    for c in &mut columns {
        c.sort_by_key(|(y, _)| *y);
    }
    Some(columns)
}

/// What the world holds at `pos` as densities: the sculpted chunk if there
/// is one, otherwise the levels' columns, with the fractional height of
/// each column's surface as the density of its top blocks.
pub fn densities_at(dim: &Dimension, pos: ChunkPos) -> VoxelChunk {
    if let Some(chunk) = dim.voxels.get(pos) {
        return chunk.clone();
    }
    match TileStack::new(dim, pos.tile(), false) {
        Some(stack) => densities_in(&stack, pos),
        None => VoxelChunk::air(),
    }
}

/// The sculpted chunks of `dim` that hold nothing of their own: those over
/// no land of the map, and those whose densities and terrain are what the
/// levels compose there without any sculpture. They change nothing in the
/// world, but a chunk stands for its whole 16³ cube: the ground changed
/// under it later (a brush on the map, a filter over the whole map) would
/// not show there. Strokes in 3D made them over the whole reach of their
/// brush (see [`crate::history::Edit::par_voxels`], which keeps them no
/// more).
pub fn idle_chunks(dim: &Dimension) -> Vec<ChunkPos> {
    let ground = dim.ground();
    let mut idle: Vec<ChunkPos> = dim.voxels.iter().map(|(p, _)| p).filter(|p| !ground.contains(p.tile())).collect();
    let kept: Vec<ChunkPos> = dim.voxels.iter().map(|(p, _)| p).filter(|p| ground.contains(p.tile())).collect();
    idle.extend(bare_densities(dim, &kept).into_iter().filter(|(p, bare)| dim.voxels.get(*p).is_some_and(|c| c.is_bare(bare))).map(|(p, _)| p));
    idle.sort_by_key(|p| (p.x, p.z, p.y));
    idle
}

/// What the levels of `dim` compose at each of `positions`, leaving out
/// what was sculpted: the chunks there as they would be without any.
pub fn bare_densities(dim: &Dimension, positions: &[ChunkPos]) -> Vec<(ChunkPos, VoxelChunk)> {
    let mut by_tile: HashMap<TilePos, Vec<ChunkPos>> = HashMap::new();
    for &p in positions {
        by_tile.entry(p.tile()).or_default().push(p);
    }
    let by_tile: Vec<(TilePos, Vec<ChunkPos>)> = by_tile.into_iter().collect();
    by_tile
        .par_iter()
        .flat_map_iter(|(tile, chunks)| {
            let empty = VoxelLayer::default();
            let stack = dim.ground().view(*tile).map(|ground| {
                let mut layers = vec![(0, crate::level::LevelKind::Ground, ground)];
                for (index, level) in dim.levels.iter().enumerate().skip(1) {
                    if level.visible
                        && let Some(t) = level.view(*tile)
                    {
                        layers.push((index, level.kind, t));
                    }
                }
                TileStack::gather(dim.min_y, dim.max_y, &empty, *tile, layers)
            });
            let made: Vec<(ChunkPos, VoxelChunk)> = chunks
                .par_iter()
                .map(|&p| {
                    let chunk = match &stack {
                        Some(stack) => densities_in(stack, p),
                        None => VoxelChunk::air(),
                    };
                    (p, chunk)
                })
                .collect();
            made
        })
        .collect()
}

/// [`densities_at`] for many chunks: each tile they lie in read once (not
/// once a chunk), the chunks worked out side by side.
pub fn densities_of(dim: &Dimension, positions: &[ChunkPos]) -> Vec<(ChunkPos, VoxelChunk)> {
    let mut by_tile: HashMap<TilePos, Vec<ChunkPos>> = HashMap::new();
    for &p in positions {
        by_tile.entry(p.tile()).or_default().push(p);
    }
    let by_tile: Vec<(TilePos, Vec<ChunkPos>)> = by_tile.into_iter().collect();
    by_tile
        .par_iter()
        .flat_map_iter(|(tile, chunks)| {
            // A tile's stack only for the chunks not sculpted already.
            let stack = chunks.iter().any(|p| dim.voxels.get(*p).is_none()).then(|| TileStack::new(dim, *tile, false)).flatten();
            let made: Vec<(ChunkPos, VoxelChunk)> = chunks
                .par_iter()
                .map(|&p| {
                    let chunk = match (dim.voxels.get(p), &stack) {
                        (Some(c), _) => c.clone(),
                        (None, Some(stack)) => densities_in(stack, p),
                        (None, None) => VoxelChunk::air(),
                    };
                    (p, chunk)
                })
                .collect();
            made
        })
        .collect()
}

/// The densities of the columns of `stack` (the tile of `pos`) over the
/// chunk at `pos`.
fn densities_in(stack: &TileStack, pos: ChunkPos) -> VoxelChunk {
    let mut chunk = VoxelChunk::air();
    let (x0, y0, z0) = pos.origin();
    let mut spans = Vec::new();
    for lz in 0..SIZE {
        for lx in 0..SIZE {
            let cell = local_index(x0 + lx, z0 + lz);
            stack.spans(cell, &mut spans);
            let last = spans.len().saturating_sub(1);
            for (i, s) in spans.iter().enumerate() {
                let (a, b) = (s.y0.max(y0), s.y1.min(y0 + SIZE));
                for y in a..b {
                    chunk.set(lx, y - y0, lz, 255);
                }
                // The top of the column: partly filled blocks.
                if i == last && block_top(s.top) == s.y1 {
                    for y in (s.y1 - 1)..=(s.y1) {
                        if (y0..y0 + SIZE).contains(&y) {
                            let d = ((s.top - y as f32).clamp(0.0, 1.0) * 255.0).round() as u8;
                            let d = if y < s.y1 { d.max(SOLID) } else { d.min(SOLID - 1) };
                            chunk.set(lx, y - y0, lz, d);
                        }
                    }
                }
            }
        }
    }
    chunk
}

// ------------------------------------------------------------ sculpting --

/// What a sculpting dab does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SculptMode {
    /// Add matter: build up, overhangs, arches.
    Add,
    /// Remove matter: dig caves and tunnels, carve.
    Remove,
    /// Blend toward the 3D neighbourhood: rounds cliff edges and fills
    /// sharp hollows, in every direction (not layer by layer).
    Smooth,
}

/// Sculpt a ball of `brush.radius` around `center` (block coordinates,
/// continuous). `strength` (0..1) is how far one dab goes; the brush profile
/// fades it toward the rim. Returns the number of voxels changed.
pub fn sculpt(edit: &mut Edit, brush: &Brush, center: [f32; 3], strength: f32, mode: SculptMode) -> usize {
    sculpt_at(edit, brush, center, None, strength, mode)
}

/// Where the surface is under a cursor, and which way it faces: smoothed
/// over the blocks around, so that it glides over the steps of the terrain
/// rather than jumping from the top of a block to its side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub center: [f32; 3],
    /// Outward, unit.
    pub normal: [f32; 3],
}

/// The surface where a ray from `origin` along `dir` hit it (`hit`): its
/// point and its normal, read from the densities around smoothed over five
/// blocks, the slope of the land rather than the face of a block.
pub fn surface_frame(dim: &Dimension, hit: &Hit, origin: [f64; 3], dir: [f64; 3]) -> Frame {
    let (bx, by, bz) = hit.block;
    let k = 2;
    let lo = [bx - 4, by - 4, bz - 4];
    let hi = [bx + 5, by + 5, bz + 5];
    let field = Field::gather(dim, lo, hi, k).blurred(k);
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-9);
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    let at = |t: f64| [(origin[0] + d[0] * t) as f32, (origin[1] + d[1] * t) as f32, (origin[2] + d[2] * t) as f32];
    // Where the smoothed matter becomes solid along the ray, near the hit.
    let mut center = [hit.point[0] as f32, hit.point[1] as f32, hit.point[2] as f32];
    let (t0, t1) = (hit.distance - 3.0, hit.distance + 3.0);
    let mut last: Option<(f64, f32)> = None;
    let mut t = t0.max(0.0);
    while t <= t1 {
        let v = field.sample(at(t));
        if let Some((tl, vl)) = last
            && vl < SOLID as f32
            && v >= SOLID as f32
        {
            let f = ((SOLID as f32 - vl) / (v - vl).max(1e-3)) as f64;
            center = at(tl + (t - tl) * f);
            break;
        }
        last = Some((t, v));
        t += 0.2;
    }
    // Facing away from the matter.
    let e = 1.0;
    let g = [
        field.sample([center[0] + e, center[1], center[2]]) - field.sample([center[0] - e, center[1], center[2]]),
        field.sample([center[0], center[1] + e, center[2]]) - field.sample([center[0], center[1] - e, center[2]]),
        field.sample([center[0], center[1], center[2] + e]) - field.sample([center[0], center[1], center[2] - e]),
    ];
    let n = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
    let face = {
        let (x, y, z) = hit.normal;
        [x as f32, y as f32, z as f32]
    };
    let smooth = if n > 1e-3 { [-g[0] / n, -g[1] / n, -g[2] / n] } else { face };
    // Where the smoothed slope and the face pointed at disagree — at the
    // foot of a cliff, on an edge, in a hollow — the face wins: the brush
    // works on what the pointer is on, toward the eye, never into the
    // matter nor along it.
    let agree = smooth[0] * face[0] + smooth[1] * face[1] + smooth[2] * face[2];
    let t = ((agree - 0.25) / 0.5).clamp(0.0, 1.0);
    let mixed = [face[0] + (smooth[0] - face[0]) * t, face[1] + (smooth[1] - face[1]) * t, face[2] + (smooth[2] - face[2]) * t];
    let l = (mixed[0] * mixed[0] + mixed[1] * mixed[1] + mixed[2] * mixed[2]).sqrt();
    let normal = if l > 1e-3 { [mixed[0] / l, mixed[1] / l, mixed[2] / l] } else { face };
    Frame { center, normal }
}

/// The surface under a brush of `radius` where a ray hit it: the point of
/// [`surface_frame`], facing the way the land faces over the brush's whole
/// area ([`mean_normal`]), so that the one-block steps of a slope, a riser
/// pointed at, do not turn the brush sideways.
pub fn brush_frame(dim: &Dimension, hit: &Hit, origin: [f64; 3], dir: [f64; 3], radius: f32) -> Frame {
    let near = surface_frame(dim, hit, origin, dir);
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-9);
    let toward = [(-dir[0] / len) as f32, (-dir[1] / len) as f32, (-dir[2] / len) as f32];
    Frame {
        center: near.center,
        normal: mean_normal(dim, near.center, radius, toward, near.normal),
    }
}

/// The widest a brush's surface is averaged over, in blocks of radius: wide
/// enough to see past any step, small enough to read every frame.
const MEAN_REACH: f32 = 16.0;

/// The way the matter faces around `center`, over a ball of `radius` (3 to
/// [`MEAN_REACH`] blocks): the mean of the faces between solid and air in
/// it, each counting for its area and less toward the rim — the densities'
/// differences summed over the ball. On a staircase the treads and the
/// risers add up to the slope they make. Where that mean faces away from
/// `toward` (the way to the eye: in a hollow, under an arch) or is none
/// (all rock, all air), `fallback`.
pub fn mean_normal(dim: &Dimension, center: [f32; 3], radius: f32, toward: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let w = radius.clamp(3.0, MEAN_REACH);
    let c = [center[0].floor() as i32, center[1].floor() as i32, center[2].floor() as i32];
    let r = w.ceil() as i32 + 1;
    let field = Field::gather(dim, [c[0] - r, c[1] - r, c[2] - r], [c[0] + r + 1, c[1] + r + 1, c[2] + r + 1], 0);
    let mut g = [0f64; 3];
    for y in c[1] - r..c[1] + r {
        for z in c[2] - r..c[2] + r {
            for x in c[0] - r..c[0] + r {
                let here = field.at(x, y, z);
                for (a, next) in [(0, [x + 1, y, z]), (1, [x, y + 1, z]), (2, [x, y, z + 1])] {
                    let step = field.at(next[0], next[1], next[2]) - here;
                    if step == 0.0 {
                        continue;
                    }
                    // The face between the two blocks, from the centre.
                    let mut f = [x as f32 + 0.5 - center[0], y as f32 + 0.5 - center[1], z as f32 + 0.5 - center[2]];
                    f[a] += 0.5;
                    let d = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
                    if d < w {
                        g[a] += (step * (1.0 - d / w)) as f64;
                    }
                }
            }
        }
    }
    let l = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
    if l < 1e-3 {
        return fallback;
    }
    // Out of the matter: against the densities' rise.
    let n = [(-g[0] / l) as f32, (-g[1] / l) as f32, (-g[2] / l) as f32];
    if n[0] * toward[0] + n[1] * toward[1] + n[2] * toward[2] < 0.05 { fallback } else { n }
}

/// The way a brush that raises and lowers the land works on a surface
/// facing `n`: straight up on gentle slopes (up to 25°), on the columns as
/// on the map; along the surface on steep ones (from 40°: cliffs, walls,
/// ceilings), into overhangs and caves; between the two, a blend.
pub fn upright(n: [f32; 3]) -> [f32; 3] {
    let (gentle, steep) = (25f32.to_radians().cos(), 40f32.to_radians().cos());
    let t = ((gentle - n[1]) / (gentle - steep)).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let m = [n[0] * t, 1.0 - t + n[1] * t, n[2] * t];
    let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
    if l < 1e-3 { n } else { [m[0] / l, m[1] / l, m[2] / l] }
}

/// Two directions along a surface facing `n`: the first as close as can be
/// to east, the second to south, so that a brush keeps the turn it has on
/// the map.
pub fn tangents(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f32; 3]| {
        let l = dot(a, a).sqrt().max(1e-6);
        [a[0] / l, a[1] / l, a[2] / l]
    };
    let axis = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let k = dot(axis, n);
    let t1 = unit([axis[0] - n[0] * k, axis[1] - n[1] * k, axis[2] - n[2] * k]);
    let t2 = [t1[1] * n[2] - t1[2] * n[1], t1[2] * n[0] - t1[0] * n[2], t1[0] * n[1] - t1[1] * n[0]];
    (t1, t2)
}

/// [`sculpt`], laid along a surface facing `normal` when there is one: the
/// brush's shape (a custom image too) on the slope, fading off it.
pub fn sculpt_at(edit: &mut Edit, brush: &Brush, center: [f32; 3], normal: Option<[f32; 3]>, strength: f32, mode: SculptMode) -> usize {
    let brush = &*within_reach(brush);
    let r = brush.radius.max(0.5);
    let reach = brush.reach().max(r);
    let frame = normal.map(|n| (n, tangents(n)));
    let (min_y, max_y) = (edit.dim().min_y, edit.dim().max_y);
    let lo = [
        (center[0] - reach).floor() as i32,
        ((center[1] - reach).floor() as i32).max(min_y),
        (center[2] - reach).floor() as i32,
    ];
    let hi = [
        (center[0] + reach).ceil() as i32,
        ((center[1] + reach).ceil() as i32).min(max_y),
        (center[2] + reach).ceil() as i32,
    ];
    if hi[1] <= lo[1] {
        return 0;
    }
    // Only inside the map.
    let ground = edit.dim().ground();
    let mut positions = Vec::new();
    let (c0, c1) = (ChunkPos::containing(lo[0], lo[1], lo[2]), ChunkPos::containing(hi[0] - 1, hi[1] - 1, hi[2] - 1));
    for cy in c0.y..=c1.y {
        for cz in c0.z..=c1.z {
            for cx in c0.x..=c1.x {
                let pos = ChunkPos::new(cx, cy, cz);
                if ground.contains(pos.tile()) {
                    positions.push(pos);
                }
            }
        }
    }
    if positions.is_empty() {
        return 0;
    }

    // Smoothing reads the neighbourhood before anything changes, and aims
    // at it smoothed in two steps (Taubin's): blurred, then pushed back
    // out a little, so that bumps and steps melt away while the land keeps
    // its matter — a hill is not worn down, a hollow not filled in.
    let k = ((r * 0.25).round() as i32).clamp(1, 4);
    let blurred = (mode == SculptMode::Smooth).then(|| {
        const SHRINK: f32 = 0.8;
        const INFLATE: f32 = -0.85;
        let field = Field::gather(edit.dim(), lo, hi, 2 * k);
        let step = |f: Field, by: f32| {
            let b = f.clone().blurred(k);
            let values = f.values.par_iter().zip(b.values.par_iter()).map(|(v, b)| v + by * (b - v)).collect();
            Field { values, ..f }
        };
        step(step(field, SHRINK), INFLATE)
    });

    let strength = strength.clamp(0.0, 1.0);
    let weigher = brush.weigher();
    let changed = std::sync::atomic::AtomicUsize::new(0);
    // Only the columns of the map's selection, by their weight.
    let selection = edit.dim().selection.clone();
    edit.par_voxels(&positions, |pos, chunk| {
        let (x0, y0, z0) = pos.origin();
        for ly in 0..SIZE {
            let y = y0 + ly;
            if y < lo[1] || y >= hi[1] {
                continue;
            }
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let (x, z) = (x0 + lx, z0 + lz);
                    let p = [x as f32 + 0.5 - center[0], y as f32 + 0.5 - center[1], z as f32 + 0.5 - center[2]];
                    let shape = match &frame {
                        // Along the surface: the brush's own shape, fading
                        // off the slope, as deep as the brush is wide.
                        Some((n, (t1, t2))) => {
                            let w = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
                            let across = 1.0 - (w / r).powi(2);
                            if across <= 0.0 {
                                continue;
                            }
                            let u = p[0] * t1[0] + p[1] * t1[1] + p[2] * t1[2];
                            let v = p[0] * t2[0] + p[1] * t2[1] + p[2] * t2[2];
                            weigher.weight(u, v) * across
                        }
                        None => {
                            let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() / r;
                            if d > 1.0 {
                                continue;
                            }
                            weigher.profile(d)
                        }
                    };
                    let mut w = (shape * strength).min(1.0);
                    if let Some(s) = &selection {
                        w *= s.weight(x, z);
                    }
                    if w <= 0.0 {
                        continue;
                    }
                    let old = chunk.get(lx, ly, lz) as f32;
                    let new = match mode {
                        SculptMode::Add => old + (255.0 - old) * w,
                        SculptMode::Remove => old * (1.0 - w),
                        SculptMode::Smooth => {
                            let target = blurred.as_ref().map_or(old, |f| f.at(x, y, z));
                            // Once smooth enough, left as it is.
                            if (target - old).abs() < 0.5 { old } else { old + (target - old) * w }
                        }
                    };
                    let new = new.round().clamp(0.0, 255.0) as u8;
                    if new != old as u8 {
                        chunk.set(lx, ly, lz, new);
                        changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
    });
    changed.into_inner()
}

/// Push the surface at `center`, facing `normal`, out (adding matter) or in
/// (removing it) by up to `amount` blocks: the brush's shape (a custom
/// image too) laid on the slope sets how far each point goes, fading off
/// the slope. Only matter within `amount` of the surface moves: a dab adds
/// a layer, it does not fill the brush's whole volume. Returns the number
/// of voxels changed.
pub fn displace_at(edit: &mut Edit, brush: &Brush, center: [f32; 3], normal: [f32; 3], amount: f32, mode: SculptMode) -> usize {
    let outward = match mode {
        SculptMode::Add => 1.0f32,
        SculptMode::Remove => -1.0,
        SculptMode::Smooth => return 0,
    };
    let amount = amount.clamp(0.0, 64.0);
    if amount <= 0.0 {
        return 0;
    }
    let brush = &*within_reach(brush);
    let r = brush.radius.max(0.5);
    let reach = brush.reach().max(r) + amount + 1.0;
    let (t1, t2) = tangents(normal);
    let (min_y, max_y) = (edit.dim().min_y, edit.dim().max_y);
    let lo = [(center[0] - reach).floor() as i32, ((center[1] - reach).floor() as i32).max(min_y), (center[2] - reach).floor() as i32];
    let hi = [(center[0] + reach).ceil() as i32, ((center[1] + reach).ceil() as i32).min(max_y), (center[2] + reach).ceil() as i32];
    if hi[1] <= lo[1] {
        return 0;
    }
    let ground = edit.dim().ground();
    let mut positions = Vec::new();
    let (c0, c1) = (ChunkPos::containing(lo[0], lo[1], lo[2]), ChunkPos::containing(hi[0] - 1, hi[1] - 1, hi[2] - 1));
    for cy in c0.y..=c1.y {
        for cz in c0.z..=c1.z {
            for cx in c0.x..=c1.x {
                let pos = ChunkPos::new(cx, cy, cz);
                if ground.contains(pos.tile()) {
                    positions.push(pos);
                }
            }
        }
    }
    if positions.is_empty() {
        return 0;
    }
    // The matter as it is, to read it moved from; when adding, what it is
    // made of too, so that it brings its terrain along.
    let reach_back = amount.ceil() as i32 + 2;
    let field = Field::gather(edit.dim(), lo, hi, reach_back);
    let terrains = (outward > 0.0).then(|| Terrains::gather(edit.dim(), [lo[0] - reach_back, lo[1] - reach_back, lo[2] - reach_back], [hi[0] + reach_back, hi[1] + reach_back, hi[2] + reach_back]));
    let weigher = brush.weigher();
    let changed = std::sync::atomic::AtomicUsize::new(0);
    let selection = edit.dim().selection.clone();
    edit.par_voxels(&positions, |pos, chunk| {
        let (x0, y0, z0) = pos.origin();
        for ly in 0..SIZE {
            let y = y0 + ly;
            if y < lo[1] || y >= hi[1] {
                continue;
            }
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let (x, z) = (x0 + lx, z0 + lz);
                    let at = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
                    let p = [at[0] - center[0], at[1] - center[1], at[2] - center[2]];
                    let w = p[0] * normal[0] + p[1] * normal[1] + p[2] * normal[2];
                    let across = 1.0 - (w / r).powi(2);
                    if across <= 0.0 {
                        continue;
                    }
                    let u = p[0] * t1[0] + p[1] * t1[1] + p[2] * t1[2];
                    let v = p[0] * t2[0] + p[1] * t2[1] + p[2] * t2[2];
                    let shift = amount * weigher.weight(u, v) * across * outward * selection.as_ref().map_or(1.0, |s| s.weight(x, z));
                    if shift.abs() < 0.01 {
                        continue;
                    }
                    // What lies `shift` blocks back along the normal comes here.
                    let from = [at[0] - normal[0] * shift, at[1] - normal[1] * shift, at[2] - normal[2] * shift];
                    let moved = field.sample(from);
                    let old = chunk.get(lx, ly, lz);
                    let new = if outward > 0.0 { (old as f32).max(moved) } else { (old as f32).min(moved) };
                    let new = new.round().clamp(0.0, 255.0) as u8;
                    if new != old {
                        chunk.set(lx, ly, lz, new);
                        if new >= SOLID && old < SOLID {
                            // New matter: made of what it came from (or of
                            // what lies a block further back, past the air
                            // of a partly filled surface block).
                            let back = |k: f32| [from[0] - normal[0] * k, from[1] - normal[1] * k, from[2] - normal[2] * k];
                            let source = terrains.as_ref().and_then(|t| t.at(from).or_else(|| t.at(back(1.0))));
                            chunk.set_terrain(lx, ly, lz, source);
                        } else if new < SOLID && old >= SOLID {
                            chunk.set_terrain(lx, ly, lz, None);
                        }
                        changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
    });
    changed.into_inner()
}

/// What every block of a box is made of, as the columns compose it (with
/// the terrains sculpted matter keeps); air has none.
struct Terrains {
    lo: [i32; 3],
    size: [usize; 3],
    values: Vec<u16>,
}

impl Terrains {
    fn gather(dim: &Dimension, lo: [i32; 3], hi: [i32; 3]) -> Self {
        let size = [(hi[0] - lo[0]).max(0) as usize, (hi[1] - lo[1]).max(0) as usize, (hi[2] - lo[2]).max(0) as usize];
        let mut values = vec![FOLLOW; size[0] * size[1] * size[2]];
        let rect = BlockRect::new(lo[0], lo[2], hi[0], hi[2]);
        // The columns' spans, a tile at a time side by side (composing
        // them is the work); then laid into the box.
        let tiles: Vec<TilePos> = rect.tiles().collect();
        let columns: Vec<(i32, i32, Span)> = tiles
            .par_iter()
            .flat_map_iter(|&tile| {
                let mut out = Vec::new();
                if let Some(stack) = TileStack::new(dim, tile, false) {
                    let clip = rect.intersect(&tile.rect());
                    let mut spans = Vec::new();
                    for z in clip.z0..clip.z1 {
                        for x in clip.x0..clip.x1 {
                            stack.spans(local_index(x, z), &mut spans);
                            out.extend(spans.iter().filter(|s| s.y1 > lo[1] && s.y0 < hi[1]).map(|s| (x, z, *s)));
                        }
                    }
                }
                out
            })
            .collect();
        for (x, z, s) in columns {
            for y in s.y0.max(lo[1])..s.y1.min(hi[1]) {
                let i = ((y - lo[1]) as usize * size[2] + (z - lo[2]) as usize) * size[0] + (x - lo[0]) as usize;
                values[i] = s.terrain.0;
            }
        }
        Self { lo, size, values }
    }

    /// The terrain of the block holding point `p`; `None` for air or
    /// outside the box.
    fn at(&self, p: [f32; 3]) -> Option<crate::registry::TerrainId> {
        let [x, y, z] = [0, 1, 2].map(|k| p[k].floor() as i32 - self.lo[k]);
        if x < 0 || y < 0 || z < 0 || x as usize >= self.size[0] || y as usize >= self.size[1] || z as usize >= self.size[2] {
            return None;
        }
        let v = self.values[(y as usize * self.size[2] + z as usize) * self.size[0] + x as usize];
        (v != FOLLOW).then_some(crate::registry::TerrainId(v))
    }
}

/// Paint the sculpted matter in the columns under `brush` at `(cx, cz)`:
/// `terrain(x, z)` gives what each column is painted with (`None`: the
/// column's own, again). The levels are painted apart; this reaches the
/// matter sculpting moved, which keeps its own terrain.
pub fn paint_sculpted(edit: &mut Edit, brush: &Brush, cx: f32, cz: f32, terrain: impl Fn(i32, i32) -> Option<crate::registry::TerrainId> + Sync) -> usize {
    let footprint = BlockRect::around(cx, cz, brush.reach().max(brush.radius));
    let positions: Vec<ChunkPos> = footprint.tiles().flat_map(|t| edit.dim().voxels.in_tile(t).map(|(p, _)| p).collect::<Vec<_>>()).collect();
    if positions.is_empty() {
        return 0;
    }
    let changed = std::sync::atomic::AtomicUsize::new(0);
    let selection = edit.dim().selection.clone();
    edit.par_voxels(&positions, |pos, chunk| {
        let (x0, y0, z0) = pos.origin();
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let (x, z) = (x0 + lx, z0 + lz);
                if brush.weight(x as f32 + 0.5 - cx, z as f32 + 0.5 - cz) < 0.5 || selection.as_ref().is_some_and(|s| s.get(x, z) < crate::selection::HALF) {
                    continue;
                }
                let t = terrain(x, z);
                for ly in 0..SIZE {
                    if chunk.solid(lx, ly, lz) && chunk.terrain(lx, ly, lz) != t {
                        chunk.set_terrain(lx, ly, lz, t);
                        changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
        let _ = y0;
    });
    changed.into_inner()
}

/// Densities of a box of the world, for smoothing.
#[derive(Clone)]
struct Field {
    lo: [i32; 3],
    size: [usize; 3],
    values: Vec<f32>,
}

impl Field {
    /// The densities of `[lo - k, hi + k)`.
    fn gather(dim: &Dimension, lo: [i32; 3], hi: [i32; 3], k: i32) -> Self {
        let lo = [lo[0] - k, lo[1] - k, lo[2] - k];
        let hi = [hi[0] + k, hi[1] + k, hi[2] + k];
        let size = [(hi[0] - lo[0]) as usize, (hi[1] - lo[1]) as usize, (hi[2] - lo[2]) as usize];
        let mut values = vec![0f32; size[0] * size[1] * size[2]];
        let (c0, c1) = (ChunkPos::containing(lo[0], lo[1], lo[2]), ChunkPos::containing(hi[0] - 1, hi[1] - 1, hi[2] - 1));
        let mut chunks = Vec::new();
        for cy in c0.y..=c1.y {
            for cz in c0.z..=c1.z {
                for cx in c0.x..=c1.x {
                    chunks.push(ChunkPos::new(cx, cy, cz));
                }
            }
        }
        let filled: HashMap<ChunkPos, VoxelChunk> = densities_of(dim, &chunks).into_iter().collect();
        // A layer of the box at a time, side by side.
        values.par_chunks_mut(size[0] * size[2]).enumerate().for_each(|(j, layer)| {
            let y = lo[1] + j as i32;
            let (cy, ly) = (y.div_euclid(SIZE), y.rem_euclid(SIZE));
            for cz in c0.z..=c1.z {
                for cx in c0.x..=c1.x {
                    let Some(chunk) = filled.get(&ChunkPos::new(cx, cy, cz)) else { continue };
                    let (x0, _, z0) = ChunkPos::new(cx, cy, cz).origin();
                    for lz in 0..SIZE {
                        let z = z0 + lz;
                        if z < lo[2] || z >= hi[2] {
                            continue;
                        }
                        let row = &mut layer[(z - lo[2]) as usize * size[0]..][..size[0]];
                        for lx in 0..SIZE {
                            let x = x0 + lx;
                            if x >= lo[0] && x < hi[0] {
                                row[(x - lo[0]) as usize] = chunk.get(lx, ly, lz) as f32;
                            }
                        }
                    }
                }
            }
        });
        Self { lo, size, values }
    }

    /// Two passes of a separable box blur of radius `k`: close to a
    /// Gaussian, the same in every direction. Each pass sums the `2k + 1`
    /// neighbours along x within rows, along z and y within and between
    /// layers, the layers in parallel; at the edges, over those there are.
    fn blurred(mut self, k: i32) -> Self {
        let [sx, sy, sz] = self.size;
        let k = k.max(0) as usize;
        let layer = sx * sz;
        let mut tmp = vec![0f32; self.values.len()];
        for _ in 0..2 {
            // Along x: a running sum over each row.
            tmp.par_chunks_mut(sx).zip(self.values.par_chunks(sx)).for_each(|(out, row)| {
                let (mut sum, mut count) = (0f32, 0f32);
                for v in &row[..sx.min(k + 1)] {
                    sum += v;
                    count += 1.0;
                }
                for t in 0..sx {
                    out[t] = sum / count;
                    if t + k + 1 < sx {
                        sum += row[t + k + 1];
                        count += 1.0;
                    }
                    if t >= k {
                        sum -= row[t - k];
                        count -= 1.0;
                    }
                }
            });
            std::mem::swap(&mut self.values, &mut tmp);
            // Along z: rows of the same layer.
            tmp.par_chunks_mut(layer).zip(self.values.par_chunks(layer)).for_each(|(out, src)| {
                for z in 0..sz {
                    let (a, b) = (z.saturating_sub(k), (z + k + 1).min(sz));
                    let row = &mut out[z * sx..][..sx];
                    row.fill(0.0);
                    for zz in a..b {
                        for (o, v) in row.iter_mut().zip(&src[zz * sx..][..sx]) {
                            *o += v;
                        }
                    }
                    let inv = 1.0 / (b - a) as f32;
                    row.iter_mut().for_each(|o| *o *= inv);
                }
            });
            std::mem::swap(&mut self.values, &mut tmp);
            // Along y: the same place in the layers around.
            let src = &self.values;
            tmp.par_chunks_mut(layer).enumerate().for_each(|(y, out)| {
                let (a, b) = (y.saturating_sub(k), (y + k + 1).min(sy));
                out.fill(0.0);
                for yy in a..b {
                    for (o, v) in out.iter_mut().zip(&src[yy * layer..][..layer]) {
                        *o += v;
                    }
                }
                let inv = 1.0 / (b - a) as f32;
                out.iter_mut().for_each(|o| *o *= inv);
            });
            std::mem::swap(&mut self.values, &mut tmp);
        }
        self
    }

    fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        let (i, j, k) = ((x - self.lo[0]) as usize, (y - self.lo[1]) as usize, (z - self.lo[2]) as usize);
        self.values[(j * self.size[2] + k) * self.size[0] + i]
    }

    /// The density at a point (block coordinates, continuous), between the
    /// centres of the blocks; the edge of the box beyond it.
    fn sample(&self, p: [f32; 3]) -> f32 {
        let mut i = [0usize; 3];
        let mut f = [0f32; 3];
        for a in 0..3 {
            let v = (p[a] - 0.5 - self.lo[a] as f32).clamp(0.0, (self.size[a] - 1) as f32 - 1e-3);
            i[a] = v.floor() as usize;
            f[a] = v - i[a] as f32;
        }
        let get = |x: usize, y: usize, z: usize| {
            let (x, y, z) = (x.min(self.size[0] - 1), y.min(self.size[1] - 1), z.min(self.size[2] - 1));
            self.values[(y * self.size[2] + z) * self.size[0] + x]
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let c00 = lerp(get(i[0], i[1], i[2]), get(i[0] + 1, i[1], i[2]), f[0]);
        let c10 = lerp(get(i[0], i[1] + 1, i[2]), get(i[0] + 1, i[1] + 1, i[2]), f[0]);
        let c01 = lerp(get(i[0], i[1], i[2] + 1), get(i[0] + 1, i[1], i[2] + 1), f[0]);
        let c11 = lerp(get(i[0], i[1] + 1, i[2] + 1), get(i[0] + 1, i[1] + 1, i[2] + 1), f[0]);
        lerp(lerp(c00, c10, f[1]), lerp(c01, c11, f[1]), f[2])
    }
}

// ------------------------------------------------------------ ray casts --

/// Where a ray meets the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The solid block hit.
    pub block: (i32, i32, i32),
    /// Outward normal of the face entered.
    pub normal: (i32, i32, i32),
    /// The point on that face.
    pub point: [f64; 3],
    pub distance: f64,
}

/// Cast a ray from `origin` along `dir` and return the first solid block
/// within `max_distance`, as composed (levels and sculpted chunks).
pub fn raycast(dim: &Dimension, origin: [f64; 3], dir: [f64; 3], max_distance: f64) -> Option<Hit> {
    cast(dim, origin, dir, max_distance, false)
}

/// [`raycast`] seeing through the ground `origin` is in, as the game's
/// spectator mode does: the first solid block entered once out of it (a
/// cave's wall seen from the rock around it), or as [`raycast`] from air.
pub fn raycast_out(dim: &Dimension, origin: [f64; 3], dir: [f64; 3], max_distance: f64) -> Option<Hit> {
    cast(dim, origin, dir, max_distance, true)
}

fn cast(dim: &Dimension, origin: [f64; 3], dir: [f64; 3], max_distance: f64, through: bool) -> Option<Hit> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len == 0.0 {
        return None;
    }
    let d = [dir[0] / len, dir[1] / len, dir[2] / len];
    // Start where the ray enters the height range of the world.
    let (min_y, max_y) = (dim.min_y as f64, dim.max_y as f64);
    let mut t = 0.0f64;
    let o = origin;
    if o[1] > max_y {
        if d[1] >= 0.0 {
            return None;
        }
        t = (o[1] - max_y) / -d[1];
    } else if o[1] < min_y {
        if d[1] <= 0.0 {
            return None;
        }
        t = (min_y - o[1]) / d[1];
    }
    let p = |t: f64| [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
    let start = p(t + 1e-6);
    let mut cell = [start[0].floor() as i32, start[1].floor() as i32, start[2].floor() as i32];
    let step = [d[0].signum() as i32, d[1].signum() as i32, d[2].signum() as i32];
    let next = |i: usize, c: i32| -> f64 {
        if d[i] > 0.0 {
            ((c + 1) as f64 - o[i]) / d[i]
        } else if d[i] < 0.0 {
            (c as f64 - o[i]) / d[i]
        } else {
            f64::INFINITY
        }
    };
    let mut t_max = [next(0, cell[0]), next(1, cell[1]), next(2, cell[2])];
    let t_delta = [
        if d[0] != 0.0 { 1.0 / d[0].abs() } else { f64::INFINITY },
        if d[1] != 0.0 { 1.0 / d[1].abs() } else { f64::INFINITY },
        if d[2] != 0.0 { 1.0 / d[2].abs() } else { f64::INFINITY },
    ];
    let mut normal = (0, 0, 0);
    let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
    let mut column: Option<((i32, i32), Vec<Span>)> = None;
    let mut spans = Vec::new();
    let mut entered = t;
    // Out of the ground the ray starts in (always, but seeing through it).
    let mut out = !through;
    while entered <= max_distance {
        let (x, y, z) = (cell[0], cell[1], cell[2]);
        if (y as f64) < min_y - 1.0 && step[1] <= 0 || (y as f64) >= max_y && step[1] >= 0 {
            return None;
        }
        if column.as_ref().is_none_or(|(c, _)| *c != (x, z)) {
            let tile = TilePos::containing(x, z);
            let stack = stacks.entry(tile).or_insert_with(|| TileStack::new(dim, tile, false));
            spans.clear();
            if let Some(stack) = stack {
                stack.spans(local_index(x, z), &mut spans);
            }
            column = Some(((x, z), std::mem::take(&mut spans)));
        }
        let solid = column.as_ref().is_some_and(|(_, s)| s.iter().any(|s| s.y0 <= y && y < s.y1));
        if !solid {
            out = true;
        } else if out {
            return Some(Hit {
                block: (x, y, z),
                normal,
                point: p(entered),
                distance: entered,
            });
        }
        // Step to the next block.
        let axis = if t_max[0] < t_max[1] {
            if t_max[0] < t_max[2] { 0 } else { 2 }
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        entered = t_max[axis];
        t_max[axis] += t_delta[axis];
        cell[axis] += step[axis];
        normal = match axis {
            0 => (-step[0], 0, 0),
            1 => (0, -step[1], 0),
            _ => (0, 0, -step[2]),
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{BrushShape, Profile};
    use crate::history::History;
    use crate::tile::Tile;
    use crate::world::{World, WorldMeta};

    fn world() -> World {
        let mut w = World::new(WorldMeta::default());
        for z in -2..2 {
            for x in -2..2 {
                w.dims[0].ground_mut().insert(TilePos::new(x, z), Tile::ground(64.0, 1, crate::tile::NO_FLUID));
            }
        }
        w
    }

    fn brush(radius: f32) -> Brush {
        Brush {
            shape: BrushShape::Circle,
            profile: Profile::Constant,
            radius,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        }
    }

    fn spans_at(w: &World, x: i32, z: i32) -> Vec<Span> {
        let stack = TileStack::new(&w.dims[0], TilePos::containing(x, z), false).unwrap();
        let mut spans = Vec::new();
        stack.spans(local_index(x, z), &mut spans);
        spans
    }

    /// The top of column `(x, z)` as composed.
    fn top_at(w: &World, x: i32, z: i32) -> i32 {
        spans_at(w, x, z).iter().map(|s| s.y1).max().unwrap()
    }

    /// Smoothing in 3D keeps only the chunks it changed: those it made over
    /// the reach of its brush and left as the levels compose them go, and
    /// so do they from a map that holds some (`idle_chunks`).
    #[test]
    fn strokes_keep_only_the_chunks_they_change() {
        let mut w = world();
        let mut h = History::default();
        // A bump, then smoothing dabs high in the air and deep in the rock,
        // far from it: nothing to smooth there, nothing kept (each made a
        // dozen chunks over the reach of its brush).
        h.begin("Build", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(4.0), [10.5, 66.5, 10.5], 1.0, SculptMode::Add);
        h.commit();
        let bump = w.dims[0].voxels.len();
        assert!(bump > 0);
        for y in [200.0, 0.0] {
            h.begin("Smooth", 0);
            sculpt_at(&mut h.edit(&mut w, 0), &brush(12.0), [-100.5, y, -100.5], Some([0.0, 1.0, 0.0]), 1.0, SculptMode::Smooth);
            h.commit();
        }
        assert_eq!(w.dims[0].voxels.len(), bump, "no chunk kept in the air or the rock");
        assert!(idle_chunks(&w.dims[0]).is_empty());
        // A chunk as the levels compose it, put there as older versions
        // did, and one over no land: both idle.
        let at = ChunkPos::containing(-200, 40, -200);
        let base = densities_at(&w.dims[0], at);
        w.dims[0].voxels.insert(at, Arc::new(base));
        let off = ChunkPos::containing(5000, 40, 5000);
        w.dims[0].voxels.insert(off, Arc::new(VoxelChunk::air()));
        let idle = idle_chunks(&w.dims[0]);
        assert_eq!(idle.len(), 2, "{idle:?}");
        assert!(idle.contains(&at) && idle.contains(&off));
    }

    /// A brush on the map changes the ground under sculpted chunks too, and
    /// what was sculpted there rides on it: raised by 20, every column of
    /// the bump and around it stands 20 higher (one step to undo), where the
    /// chunks used to keep the old ground, a wall at their edge, and then
    /// gave way to the new one, the bump lost.
    #[test]
    fn the_ground_moved_under_sculpted_chunks_shows() {
        let mut w = world();
        let mut h = History::default();
        // A bump sculpted at (10, 10), on ground at 64.
        h.begin("Build", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(3.0), [10.5, 64.5, 10.5], 1.0, SculptMode::Add);
        h.commit();
        assert!(top_at(&w, 10, 10) > 64);
        let around: Vec<(i32, i32)> = (0..32).flat_map(|z| (0..32).map(move |x| (x, z))).collect();
        let was: Vec<Vec<Span>> = around.iter().map(|&(x, z)| spans_at(&w, x, z)).collect();
        // The ground raised by 20 over a wide area: everywhere, the bump's
        // columns too.
        h.begin("Raise", 0);
        crate::ops::raise(&mut h.edit(&mut w, 0), &Brush { radius: 40.0, ..brush(40.0) }, 10.5, 10.5, 20.0, crate::ops::Surface::Top);
        h.commit();
        for (&(x, z), was) in around.iter().zip(&was) {
            let is = spans_at(&w, x, z);
            assert_eq!(is.len(), was.len(), "({x}, {z})");
            assert_eq!(is.last().unwrap().y1, was.last().unwrap().y1 + 20, "the bump and the ground around it 20 higher at ({x}, {z})");
        }
        // One step: undone, the bump is back on the old ground.
        h.undo(&mut w);
        for (&(x, z), was) in around.iter().zip(&was) {
            assert_eq!(spans_at(&w, x, z).last().unwrap().y1, was.last().unwrap().y1, "({x}, {z})");
        }
    }

    /// An overhang sculpted in 3D keeps its shape when the brushes of the
    /// map move the land around and under it, lightly or a lot (a user's
    /// overhang went whole at one light click of Raise beside it): Raise
    /// and Lower, Smooth, and Set height wide over it, each one step to
    /// undo.
    #[test]
    fn an_overhang_keeps_its_shape_under_the_brushes_of_the_map() {
        let mut w = world();
        let mut h = History::default();
        // A hill on the map, and a ledge sculpted over its side: 3 blocks
        // thick, 6 over the ground below it.
        h.begin("Hill", 0);
        crate::ops::raise(&mut h.edit(&mut w, 0), &Brush { profile: Profile::Sine, ..brush(24.0) }, 40.5, 40.5, 16.0, crate::ops::Surface::Top);
        h.commit();
        let ledge: Vec<(i32, i32)> = (48..56).flat_map(|x| (36..44).map(move |z| (x, z))).collect();
        h.begin("Build", 0);
        {
            let mut e = h.edit(&mut w, 0);
            let tops: Vec<i32> = ledge.iter().map(|&(x, z)| crate::compose::block_top(crate::ops::height_at(e.dim(), 0, x, z, crate::ops::Surface::Top).unwrap())).collect();
            let bottom = tops.iter().max().unwrap() + 6;
            for &(x, z) in &ledge {
                for y in bottom..bottom + 3 {
                    let pos = ChunkPos::containing(x, y, z);
                    let (x0, y0, z0) = pos.origin();
                    e.voxel_chunk_mut(pos).set(x - x0, y - y0, z - z0, 255);
                }
            }
        }
        h.commit();
        // The ledge: a run of 3 blocks over air in each of its columns.
        let shape = |w: &World| -> Vec<Option<(i32, i32)>> {
            ledge
                .iter()
                .map(|&(x, z)| {
                    let spans = spans_at(w, x, z);
                    (spans.len() == 2).then(|| (spans[1].y0, spans[1].y1 - spans[1].y0))
                })
                .collect()
        };
        let built = shape(&w);
        assert!(built.iter().all(|s| s.is_some_and(|(_, thick)| thick == 3)), "{built:?}");
        let start: Vec<Vec<Span>> = ledge.iter().map(|&(x, z)| spans_at(&w, x, z)).collect();
        type Stroke = Box<dyn Fn(&mut Edit<'_>)>;
        let light = || Brush { strength: 0.2, profile: Profile::Sine, ..brush(10.0) };
        let strokes: Vec<(&str, Stroke)> = vec![
            ("Raise", Box::new(move |e: &mut Edit<'_>| crate::ops::raise(e, &light(), 50.5, 46.5, 0.4, crate::ops::Surface::Top))),
            ("Lower", Box::new(move |e: &mut Edit<'_>| crate::ops::raise(e, &light(), 50.5, 46.5, -0.4, crate::ops::Surface::Top))),
            (
                "Smooth",
                Box::new(|e: &mut Edit<'_>| {
                    for _ in 0..8 {
                        crate::ops::smooth(e, &Brush { profile: Profile::Sine, ..brush(12.0) }, 52.5, 40.5, crate::ops::Surface::Top);
                    }
                }),
            ),
            ("Raise a lot", Box::new(|e: &mut Edit<'_>| crate::ops::raise(e, &brush(30.0), 52.5, 40.5, 5.0, crate::ops::Surface::Top))),
            ("Set height", Box::new(|e: &mut Edit<'_>| crate::ops::set_height(e, &brush(30.0), 52.5, 40.5, 70.0, crate::ops::Surface::Top))),
        ];
        for (name, stroke) in &strokes {
            let before = shape(&w);
            let ground: Vec<f32> = ledge.iter().map(|&(x, z)| crate::ops::height_at(&w.dims[0], 0, x, z, crate::ops::Surface::Top).unwrap()).collect();
            h.begin(*name, 0);
            stroke(&mut h.edit(&mut w, 0));
            h.commit();
            let after = shape(&w);
            let moved = ledge.iter().zip(&ground).filter(|&(&(x, z), g)| crate::ops::height_at(&w.dims[0], 0, x, z, crate::ops::Surface::Top).unwrap() != *g).count();
            assert!(moved > 0, "{name} moved the ground under the ledge");
            for (k, (&(x, z), is)) in ledge.iter().zip(&after).enumerate() {
                let (y, thick) = is.unwrap_or_else(|| panic!("{name}: the ledge kept at ({x}, {z}): {:?}", spans_at(&w, x, z)));
                assert_eq!(thick, 3, "{name} at ({x}, {z})");
                // It goes up or down with the ground under it, by whole
                // blocks: not at all for a light stroke.
                let shifted = crate::compose::block_top(crate::ops::height_at(&w.dims[0], 0, x, z, crate::ops::Surface::Top).unwrap()) - crate::compose::block_top(ground[k]);
                assert_eq!(y, before[k].unwrap().0 + shifted, "{name} at ({x}, {z})");
            }
        }
        // Each stroke one step: undone, the ledge and the hill as built.
        for _ in &strokes {
            h.undo(&mut w);
        }
        let back: Vec<Vec<Span>> = ledge.iter().map(|&(x, z)| spans_at(&w, x, z)).collect();
        assert_eq!(format!("{back:?}"), format!("{start:?}"));
    }

    #[test]
    fn chunks_start_as_what_the_levels_compose() {
        let w = world();
        let chunk = densities_at(&w.dims[0], ChunkPos::containing(5, 60, 5));
        // Ground at 64: blocks up to 63 solid, 64 and above air.
        assert!(chunk.solid(5, 63 - 48, 5));
        assert_eq!(chunk.solid_count(), VOLUME);
        let above = densities_at(&w.dims[0], ChunkPos::containing(5, 64, 5));
        assert_eq!(above.solid_count(), 0);
        // Sculpting nothing changes nothing: a copied chunk composes the same.
        let mut w2 = world();
        let pos = ChunkPos::containing(5, 60, 5);
        w2.dims[0].voxels.insert(pos, Arc::new(chunk));
        assert_eq!(spans_at(&w2, 5, 5), spans_at(&w, 5, 5));
    }

    /// On a slope of blocks, the frame under the cursor faces the slope,
    /// whether the ray hits the top of a step or its side, and glides along
    /// it.
    #[test]
    fn the_surface_frame_follows_the_slope_not_the_block() {
        let mut w = World::new(WorldMeta::default());
        let mut tile = Tile::ground(64.0, 1, crate::tile::NO_FLUID);
        // Rising one block per block eastward: a 45° staircase.
        for z in 0..128 {
            for x in 0..128 {
                tile.top.set(local_index(x, z), 40.0 + x as f32);
            }
        }
        w.dims[0].ground_mut().insert(TilePos::new(0, 0), tile);
        let dim = &w.dims[0];
        let mut normals = Vec::new();
        let mut centers = Vec::new();
        for k in 0..12 {
            // Looking down at the slope from the west, the ray landing a
            // little further east each time: tops and sides of steps alike.
            let o = [20.0 + k as f64 * 0.37, 120.0, 64.5];
            let d = [0.6, -1.0, 0.0];
            let hit = raycast(dim, o, d, 400.0).expect("the slope");
            let f = surface_frame(dim, &hit, o, d);
            normals.push(f.normal);
            centers.push(f.center);
        }
        let s = std::f32::consts::FRAC_1_SQRT_2;
        for n in &normals {
            assert!((n[0] + s).abs() < 0.12 && (n[1] - s).abs() < 0.12 && n[2].abs() < 0.08, "faces the slope: {n:?}");
        }
        // The centre lies on the slope (y - x the same), moving smoothly.
        for c in &centers {
            assert!((c[1] - c[0] - 40.0).abs() < 1.2, "on the slope: {c:?}");
        }
    }

    /// A square brush laid flat digs a square pit, not a ball.
    #[test]
    fn a_brush_keeps_its_shape_on_the_surface() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Dig", 0);
        let square = Brush {
            shape: BrushShape::Square,
            ..brush(6.0)
        };
        sculpt_at(&mut h.edit(&mut w, 0), &square, [10.5, 64.0, 10.5], Some([0.0, 1.0, 0.0]), 1.0, SculptMode::Remove);
        h.commit();
        let top = |x: i32, z: i32| spans_at(&w, x, z).last().map_or(0, |s| s.y1);
        // The corners of the square are dug as deep as its middle edges.
        assert!(top(6, 6) < 64 && top(15, 15) < 64, "corners dug: {} {}", top(6, 6), top(15, 15));
        assert_eq!(top(18, 18), 64, "outside untouched");
    }

    #[test]
    fn digging_makes_a_cave_and_undo_fills_it() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Dig", 0);
        // A cave 20 blocks under the surface: the ground above stays.
        let n = sculpt(&mut h.edit(&mut w, 0), &brush(6.0), [10.5, 40.5, 10.5], 1.0, SculptMode::Remove);
        assert!(n > 500, "{n} voxels changed");
        assert!(h.commit());
        let spans = spans_at(&w, 10, 10);
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(spans[1].y1, 64, "the surface is untouched");
        assert!(spans[0].y1 <= 35 && spans[1].y0 >= 46, "{spans:?}");
        // The cave's floor is rock, the surface keeps its terrain.
        assert_eq!(spans[0].terrain, crate::registry::TerrainId::ROCK);
        assert_eq!(spans[1].terrain, crate::registry::TerrainId(1));
        assert!(!w.dims[0].voxels.is_empty());
        let dirty = w.dims[0].voxels.take_dirty();
        assert!(dirty.contains(&TilePos::new(0, 0)));
        // Undo removes the chunks again.
        h.undo(&mut w);
        assert!(w.dims[0].voxels.is_empty());
        assert_eq!(spans_at(&w, 10, 10).len(), 1);
        h.redo(&mut w);
        assert_eq!(spans_at(&w, 10, 10).len(), 2);
    }

    #[test]
    fn building_makes_overhangs() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Build", 0);
        // A ball floating above the ground: matter over air.
        sculpt(&mut h.edit(&mut w, 0), &brush(5.0), [20.5, 90.5, 20.5], 1.0, SculptMode::Add);
        h.commit();
        let spans = spans_at(&w, 20, 20);
        assert_eq!(spans.len(), 2, "{spans:?}");
        // Blocks whose centre lies within the radius.
        assert_eq!((spans[1].y0, spans[1].y1), (85, 96));
        // Its top is open to the sky: it takes the surface's terrain.
        assert_eq!(spans[1].terrain, crate::registry::TerrainId(1));
    }

    /// Smoothing in 3D as the Sculpt editor does — dabs held at one spot
    /// for two seconds, laid along the ground — melts bumps away without
    /// wearing the land down, and only under the brush.
    #[test]
    fn smoothing_keeps_the_land() {
        use crate::ops::{self, Surface};
        for case in ["flat", "rough", "hill"] {
            let mut w = world();
            let mut h = History::default();
            h.begin("shape", 0);
            {
                let mut e = h.edit(&mut w, 0);
                match case {
                    "rough" => ops::roughen(&mut e, &brush(30.0), 0.5, 0.5, 4.0, 7, Surface::Top),
                    "hill" => {
                        let mut b = brush(14.0);
                        b.profile = Profile::Linear;
                        ops::raise(&mut e, &b, 0.5, 0.5, 12.0, Surface::Top)
                    }
                    _ => {}
                }
            }
            h.commit();
            let tops = |w: &World| -> (f32, f32, f32) {
                let (mut inside, mut outside, mut ni, mut no, mut rough) = (0.0, 0.0, 0.0, 0.0, 0.0);
                for z in -20..20 {
                    for x in -20..20 {
                        let s = spans_at(w, x, z);
                        let top = s.last().map_or(0.0, |s| s.top);
                        let d = ((x as f32 + 0.5).powi(2) + (z as f32 + 0.5).powi(2)).sqrt();
                        if d < 6.0 {
                            inside += top;
                            ni += 1.0;
                            let east = spans_at(w, x + 1, z).last().map_or(0.0, |s| s.top);
                            rough += (east - top).abs();
                        } else if d > 10.0 { outside += top; no += 1.0 }
                    }
                }
                (inside / ni, outside / no, rough / ni)
            };
            let before = tops(&w);
            let y = spans_at(&w, 0, 0).last().unwrap().top;
            h.begin("Smooth", 0);
            for _ in 0..120 {
                sculpt_at(&mut h.edit(&mut w, 0), &brush(8.0), [0.5, y, 0.5], Some([0.0, 1.0, 0.0]), 0.1, SculptMode::Smooth);
            }
            h.commit();
            let after = tops(&w);
            // Nothing moves away from the brush (but for the 1/255 of a
            // block densities hold heights to).
            assert!((before.1 - after.1).abs() < 0.005, "{case}: outside {before:?} -> {after:?}");
            match case {
                "flat" => assert!((after.0 - before.0).abs() < 0.01, "{case}: {before:?} -> {after:?}"),
                "rough" => {
                    assert!((after.0 - before.0).abs() < 0.12, "{case}: sank {before:?} -> {after:?}");
                    assert!(after.2 < before.2 * 0.6, "{case}: as rough {before:?} -> {after:?}");
                }
                _ => assert!(before.0 - after.0 < 0.5, "{case}: worn down {before:?} -> {after:?}"),
            }
        }
    }

    /// The Sculpt editor's dabs push the surface by what they are given, a
    /// layer at a time, and only under the brush.
    #[test]
    fn dabs_add_a_layer_not_a_mountain() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Sculpt", 0);
        // A brush 20 blocks across, half a block a dab, four dabs.
        for _ in 0..4 {
            displace_at(&mut h.edit(&mut w, 0), &brush(10.0), [0.5, 64.0, 0.5], [0.0, 1.0, 0.0], 0.5, SculptMode::Add);
        }
        h.commit();
        let top = |w: &World, x: i32, z: i32| spans_at(w, x, z).last().unwrap().top;
        // Two blocks up under the brush, nothing beyond it.
        assert!((top(&w, 0, 0) - 66.0).abs() < 0.6, "{}", top(&w, 0, 0));
        assert!((top(&w, 6, 3) - 66.0).abs() < 0.6, "{}", top(&w, 6, 3));
        assert_eq!(top(&w, 14, 0), 64.0);
        // Digging takes it back down, and on.
        h.begin("Dig", 0);
        for _ in 0..8 {
            displace_at(&mut h.edit(&mut w, 0), &brush(10.0), [0.5, 66.0, 0.5], [0.0, 1.0, 0.0], 0.5, SculptMode::Remove);
        }
        h.commit();
        assert!((top(&w, 0, 0) - 62.0).abs() < 0.6, "{}", top(&w, 0, 0));
        assert_eq!(top(&w, 14, 0), 64.0);
    }

    /// A sculpted chunk keeps the exact heights of the columns it does not
    /// change: sculpting one spot does not move the rest of its 16 × 16.
    #[test]
    fn sculpted_chunks_keep_their_heights() {
        let mut w = World::new(WorldMeta::default());
        w.dims[0].ground_mut().insert(TilePos::new(0, 0), Tile::ground(64.4, 1, crate::tile::NO_FLUID));
        let mut h = History::default();
        h.begin("Build", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(2.0), [3.5, 70.5, 3.5], 1.0, SculptMode::Add);
        h.commit();
        assert!(!w.dims[0].voxels.is_empty());
        // Far from the dab, in the same chunk: the same height as before.
        let top = spans_at(&w, 13, 12).last().unwrap().top;
        assert!((top - 64.4).abs() < 0.01, "{top}");
    }

    /// At the foot of a cliff, the ground pointed at faces up: building
    /// there raises it, rather than pushing it sideways off the cliff.
    #[test]
    fn building_at_the_foot_of_a_cliff_raises_the_ground() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Cliff", 0);
        {
            let mut e = h.edit(&mut w, 0);
            for x in 0..12 {
                for z in -6..18 {
                    for y in 64..84 {
                        let pos = ChunkPos::containing(x, y, z);
                        let (x0, y0, z0) = pos.origin();
                        e.voxel_chunk_mut(pos).set(x - x0, y - y0, z - z0, 255);
                    }
                }
            }
        }
        h.commit();
        // Two blocks from the cliff's foot, from above and from aside.
        for (origin, dir) in [([14.5, 100.0, 6.5], [0.0, -1.0, 0.0]), ([40.0, 80.0, 6.5], [-0.69, -0.42, 0.0])] {
            let hit = raycast(&w.dims[0], origin, dir, 200.0).expect("the ground is hit");
            assert_eq!(hit.normal, (0, 1, 0), "{hit:?}");
            let frame = surface_frame(&w.dims[0], &hit, origin, dir);
            assert!(frame.normal[1] > 0.8, "the ground faces up: {:?}", frame.normal);
        }
        let hit = raycast(&w.dims[0], [14.5, 100.0, 6.5], [0.0, -1.0, 0.0], 200.0).unwrap();
        let frame = surface_frame(&w.dims[0], &hit, [14.5, 100.0, 6.5], [0.0, -1.0, 0.0]);
        h.begin("Build", 0);
        for _ in 0..4 {
            displace_at(&mut h.edit(&mut w, 0), &brush(3.0), frame.center, frame.normal, 1.0, SculptMode::Add);
        }
        h.commit();
        let top = spans_at(&w, 14, 6).last().unwrap().y1;
        assert!(top >= 66, "the ground rose: top {top}");
        // On the cliff's face, from aside: it grows out toward the eye.
        let (origin, dir) = ([40.0, 74.5, 2.5], [-1.0, 0.0, 0.0]);
        let hit = raycast(&w.dims[0], origin, dir, 200.0).unwrap();
        assert_eq!(hit.normal, (1, 0, 0));
        let frame = surface_frame(&w.dims[0], &hit, origin, dir);
        assert!(frame.normal[0] > 0.8, "{:?}", frame.normal);
        h.begin("Build out", 0);
        for _ in 0..6 {
            displace_at(&mut h.edit(&mut w, 0), &brush(3.0), frame.center, frame.normal, 1.0, SculptMode::Add);
        }
        h.commit();
        let out = spans_at(&w, 13, 2);
        assert!(out.iter().any(|s| s.y0 <= 74 && s.y1 >= 75), "the face grew out to x = 13: {out:?}");
        // Held still, as long as the button is down: every frame aims again
        // at the surface as it now is, and it keeps growing.
        for (name, origin, dir) in [("down", [30.5, 140.0, 30.5], [0.0, -1.0, 0.0]), ("aslant", [40.0, 110.0, 10.5], [-0.6, -0.8, 0.0])] {
            let first = raycast(&w.dims[0], origin, dir, 400.0).unwrap();
            h.begin("Hold", 0);
            for _ in 0..120 {
                let hit = raycast(&w.dims[0], origin, dir, 400.0).unwrap();
                let frame = surface_frame(&w.dims[0], &hit, origin, dir);
                displace_at(&mut h.edit(&mut w, 0), &brush(3.0), frame.center, frame.normal, 16.0 / 60.0, SculptMode::Add);
            }
            h.commit();
            let last = raycast(&w.dims[0], origin, dir, 400.0).unwrap();
            eprintln!("{name}: grew {:.1} blocks in two seconds", first.distance - last.distance);
            assert!(first.distance - last.distance >= 8.0, "{name}: grew {:.1} blocks in two seconds", first.distance - last.distance);
        }
    }

    /// A grassy slope of one-block steps (2.5 blocks deep, 21.8°): the brush
    /// faces the slope wherever it points — a tread, a riser, from above or
    /// aslant — within a few degrees, where the face under the cursor
    /// turned it sideways; and a Raise there works straight up, on the
    /// columns the map's Raise takes.
    #[test]
    fn a_brush_faces_a_stepped_slope_not_its_steps() {
        let mut w = world();
        let step = |x: i32| 64 + (0.4 * x.clamp(-60, 60) as f32).floor() as i32;
        for z in -2..2 {
            for x in -2..2 {
                let pos = TilePos::new(x, z);
                let tile = w.dims[0].ground_mut().tile_mut(pos).unwrap();
                for lz in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        tile.top.set(local_index(lx, lz), step(pos.x * TILE_SIZE + lx) as f32);
                    }
                }
            }
        }
        let slope = {
            let l = (0.4f32 * 0.4 + 1.0).sqrt();
            [-0.4 / l, 1.0 / l, 0.0]
        };
        let angle = |n: [f32; 3]| (n[0] * slope[0] + n[1] * slope[1] + n[2] * slope[2]).clamp(-1.0, 1.0).acos().to_degrees();
        let (mut worst, mut worst_face, mut risers) = (0f32, 0f32, 0);
        for k in 0..40 {
            let x = -20.0 + k as f64;
            for dir in [[0.0, -1.0, 0.0], [0.8, -0.6, 0.0], [0.6, -0.5, 0.62], [0.95, -0.3, 0.0]] {
                let origin = [x + 0.5 - dir[0] * 60.0, 64.0 + 0.4 * x - dir[1] * 60.0, 4.5 - dir[2] * 60.0];
                let Some(hit) = raycast(&w.dims[0], origin, dir, 300.0) else { continue };
                risers += (hit.normal.1 == 0) as usize;
                for radius in [4.0, 8.0, 24.0] {
                    worst = worst.max(angle(brush_frame(&w.dims[0], &hit, origin, dir, radius).normal));
                }
                worst_face = worst_face.max(angle(surface_frame(&w.dims[0], &hit, origin, dir).normal));
            }
        }
        eprintln!("stepped slope: {risers} risers pointed at; the brush's normal at most {worst:.1}° off the slope (the face's, smoothed over 5 blocks: {worst_face:.1}°)");
        assert!(risers > 10, "risers pointed at: {risers}");
        assert!(worst < 6.0, "{worst:.1}° off the slope");
        // A Raise of 8 on a tread: straight up, on the columns of the map's.
        let origin = [3.5, 120.0, 4.5];
        let hit = raycast(&w.dims[0], origin, [0.0, -1.0, 0.0], 300.0).unwrap();
        let frame = brush_frame(&w.dims[0], &hit, origin, [0.0, -1.0, 0.0], 8.0);
        let up = upright(frame.normal);
        assert!(up[1] > 0.999, "straight up on a gentle slope: {up:?}");
        let tops = |w: &World| -> HashMap<(i32, i32), i32> { (-12..20).flat_map(|x| (-8..18).map(move |z| (x, z))).map(|(x, z)| ((x, z), spans_at(w, x, z).last().unwrap().y1)).collect() };
        let before = tops(&w);
        let mut map = w.clone();
        let mut h = History::default();
        h.begin("Raise 3D", 0);
        displace_at(&mut h.edit(&mut w, 0), &brush(8.0), frame.center, up, 2.0, SculptMode::Add);
        h.commit();
        h.begin("Raise", 0);
        crate::ops::raise(&mut h.edit(&mut map, 0), &brush(8.0), frame.center[0], frame.center[2], 2.0, crate::ops::Surface::Top);
        h.commit();
        let changed = |w: &World| -> HashSet<(i32, i32)> { tops(w).into_iter().filter(|(k, v)| before[k] != *v).map(|(k, _)| k).collect() };
        let (in_3d, on_map) = (changed(&w), changed(&map));
        let rim = |&(x, z): &(i32, i32)| ((x as f32 + 0.5 - frame.center[0]).hypot(z as f32 + 0.5 - frame.center[2]) - 8.0).abs() <= 1.0;
        let apart: Vec<_> = in_3d.symmetric_difference(&on_map).filter(|c| !rim(c)).collect();
        eprintln!("raise of 8: {} columns in 3D, {} on the map, {} apart off the rim", in_3d.len(), on_map.len(), apart.len());
        assert!(on_map.len() > 150 && apart.is_empty(), "{apart:?}");
    }

    /// Matter pushed out of a wall is made of what the wall was made of:
    /// a sand wall grows a sand ledge, not rock nor the ground's grass; and
    /// ground raised stays one span, its layers going on from its new top.
    #[test]
    fn added_matter_keeps_its_terrain() {
        let mut w = world();
        let reg = w.registry.clone();
        let sand = reg.terrain_by_name("Sand").unwrap();
        let mut h = History::default();
        h.begin("Wall", 0);
        {
            let mut e = h.edit(&mut w, 0);
            for x in 0..8 {
                for z in 0..16 {
                    for y in 64..84 {
                        let pos = ChunkPos::containing(x, y, z);
                        let (x0, y0, z0) = pos.origin();
                        let c = e.voxel_chunk_mut(pos);
                        c.set(x - x0, y - y0, z - z0, 255);
                        c.set_terrain(x - x0, y - y0, z - z0, Some(sand));
                    }
                }
            }
        }
        h.commit();
        // Out of its face, half way up.
        let (origin, dir) = ([40.0, 76.5, 8.5], [-1.0, 0.0, 0.0]);
        let hit = raycast(&w.dims[0], origin, dir, 200.0).unwrap();
        let frame = surface_frame(&w.dims[0], &hit, origin, dir);
        h.begin("Ledge", 0);
        for _ in 0..8 {
            displace_at(&mut h.edit(&mut w, 0), &brush(3.0), frame.center, frame.normal, 1.0, SculptMode::Add);
        }
        h.commit();
        let ledge = spans_at(&w, 9, 8);
        let up_there: Vec<&Span> = ledge.iter().filter(|s| s.y0 > 64).collect();
        assert!(!up_there.is_empty(), "a ledge grew: {ledge:?}");
        for s in up_there {
            assert_eq!(s.terrain, sand, "the ledge is sand: {ledge:?}");
        }
        // The ground under the ledge is still the ground, not rock.
        let ground = spans_at(&w, 30, 8).last().unwrap().terrain;
        assert_eq!(ledge[0].terrain, ground, "{ledge:?}");
        // The ground raised by a few blocks: one span still, of grass.
        let grass = spans_at(&w, 30, 30).last().unwrap().terrain;
        let (origin, dir) = ([30.5, 120.0, 30.5], [0.0, -1.0, 0.0]);
        h.begin("Mound", 0);
        for _ in 0..6 {
            let hit = raycast(&w.dims[0], origin, dir, 200.0).unwrap();
            let frame = surface_frame(&w.dims[0], &hit, origin, dir);
            displace_at(&mut h.edit(&mut w, 0), &brush(3.0), frame.center, frame.normal, 1.0, SculptMode::Add);
        }
        h.commit();
        let mound = spans_at(&w, 30, 30);
        assert_eq!(mound.len(), 1, "{mound:?}");
        assert_eq!(mound[0].terrain, grass);
        assert!(mound[0].y1 > 66, "{mound:?}");
        // Kept in the file, as they were.
        let bytes = w.dims[0].voxels.encode();
        let back = VoxelLayer::decode(&bytes).unwrap();
        for (pos, chunk) in w.dims[0].voxels.iter() {
            assert_eq!(**chunk, *back.get(pos).unwrap(), "{pos:?}");
        }
    }

    /// Files from before terrains were kept still read: their sculpted
    /// matter follows its columns.
    #[test]
    fn old_sculpted_chunks_still_read() {
        let mut raw = Vec::new();
        for (x, y, z) in [(0i32, 4i32, 0i32), (1, 4, 0)] {
            raw.extend_from_slice(&x.to_le_bytes());
            raw.extend_from_slice(&y.to_le_bytes());
            raw.extend_from_slice(&z.to_le_bytes());
            raw.extend((0..VOLUME).map(|i| if i % 3 == 0 { 255u8 } else { 0 }));
        }
        let layer = VoxelLayer::decode(&codec::zstd_compress(&raw)).unwrap();
        assert_eq!(layer.len(), 2);
        let chunk = layer.get(ChunkPos::new(1, 4, 0)).unwrap();
        assert_eq!(chunk.get(0, 0, 0), 255);
        assert_eq!(chunk.terrain(0, 0, 0), None);
    }

    #[test]
    fn smoothing_rounds_a_cliff_edge() {
        let mut w = world();
        let mut h = History::default();
        // A sharp-edged block: a 12 × 12 tower 20 blocks high.
        h.begin("Tower", 0);
        {
            let mut e = h.edit(&mut w, 0);
            for x in 0..12 {
                for z in 0..12 {
                    for y in 64..84 {
                        let pos = ChunkPos::containing(x, y, z);
                        let (x0, y0, z0) = pos.origin();
                        e.voxel_chunk_mut(pos).set(x - x0, y - y0, z - z0, 255);
                    }
                }
            }
        }
        h.commit();
        assert_eq!(spans_at(&w, 11, 11)[0].y1, 84);
        // Smooth on the top corner of the tower.
        h.begin("Smooth", 0);
        for _ in 0..3 {
            sculpt(&mut h.edit(&mut w, 0), &brush(6.0), [11.5, 83.5, 11.5], 1.0, SculptMode::Smooth);
        }
        h.commit();
        // The corner column lost its top; the middle of the tower did not.
        let corner = spans_at(&w, 11, 11).last().unwrap().y1;
        let middle = spans_at(&w, 5, 5).last().unwrap().y1;
        assert!(corner < 84, "corner top {corner}");
        assert_eq!(middle, 84);
        // Smoothing in the middle of the wall leaves it vertical: a flat
        // face stays where it is, only edges and hollows round off.
        h.begin("Smooth wall", 0);
        for _ in 0..3 {
            sculpt(&mut h.edit(&mut w, 0), &brush(6.0), [11.9, 74.0, 6.0], 1.0, SculptMode::Smooth);
        }
        h.commit();
        for y in 70..78 {
            let solid = |x: i32| spans_at(&w, x, 6).iter().any(|s| s.y0 <= y && y < s.y1);
            assert!(solid(11) && !solid(12), "the wall moved at y = {y}");
        }
    }

    #[test]
    fn rays_hit_the_surface_and_sculpted_matter() {
        let mut w = world();
        let hit = raycast(&w.dims[0], [10.5, 100.0, 10.5], [0.0, -1.0, 0.0], 500.0).unwrap();
        assert_eq!(hit.block, (10, 63, 10));
        assert_eq!(hit.normal, (0, 1, 0));
        assert!((hit.point[1] - 64.0).abs() < 1e-6);
        // Slanted, from above the world.
        let hit = raycast(&w.dims[0], [-100.0, 400.0, 0.5], [1.0, -1.0, 0.0], 2000.0).unwrap();
        assert_eq!(hit.block.1, 63);
        // A floating ball is hit first.
        let mut h = History::default();
        h.begin("Build", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(4.0), [10.5, 90.5, 10.5], 1.0, SculptMode::Add);
        h.commit();
        let hit = raycast(&w.dims[0], [10.5, 100.0, 10.5], [0.0, -1.0, 0.0], 500.0).unwrap();
        assert_eq!(hit.block.1, 94);
        // Rays that leave the world miss.
        assert!(raycast(&w.dims[0], [0.0, 100.0, 0.0], [0.0, 1.0, 0.0], 500.0).is_none());
        assert!(raycast(&w.dims[0], [1000.0, 70.0, 0.0], [1.0, 0.0, 0.0], 500.0).is_none());
    }

    #[test]
    fn rays_from_inside_the_ground_see_through_it() {
        let mut w = world();
        // A cave dug at y 50, the eye in the rock beside it.
        let mut h = History::default();
        h.begin("Dig", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(4.0), [20.5, 50.5, 10.5], 1.0, SculptMode::Remove);
        h.commit();
        let (eye, east) = ([5.5, 50.5, 10.5], [1.0, 0.0, 0.0]);
        // A plain ray stops in the block of the eye.
        assert_eq!(raycast(&w.dims[0], eye, east, 100.0).unwrap().block, (5, 50, 10));
        // Seen through, the far wall of the cave, entered from it.
        let hit = raycast_out(&w.dims[0], eye, east, 100.0).unwrap();
        assert!(hit.block.0 > 20 && hit.block.0 < 26 && hit.normal == (-1, 0, 0), "{hit:?}");
        // From air, both agree; through nothing but rock, nothing.
        let above = [10.5, 100.0, 10.5];
        assert_eq!(raycast_out(&w.dims[0], above, [0.0, -1.0, 0.0], 500.0).unwrap().block, (10, 63, 10));
        assert!(raycast_out(&w.dims[0], [5.5, 30.5, 10.5], [0.0, -1.0, 0.0], 500.0).is_none());
    }

    #[test]
    fn layers_survive_encoding() {
        let mut w = world();
        let mut h = History::default();
        h.begin("Dig", 0);
        sculpt(&mut h.edit(&mut w, 0), &brush(5.0), [3.5, 50.5, 3.5], 1.0, SculptMode::Remove);
        h.commit();
        let bytes = w.dims[0].voxels.encode();
        let back = VoxelLayer::decode(&bytes).unwrap();
        assert_eq!(back.len(), w.dims[0].voxels.len());
        for (pos, chunk) in w.dims[0].voxels.iter() {
            assert_eq!(back.get(pos).unwrap(), &**chunk);
        }
        assert!(VoxelLayer::decode(&[1, 2, 3]).is_err());
    }
}
