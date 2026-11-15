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
    terrain: Option<Box<[u16; VOLUME]>>,
}

/// A voxel whose terrain is the column's (see [`VoxelChunk`]).
const FOLLOW: u16 = u16::MAX;

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
        (t != FOLLOW).then_some(crate::registry::TerrainId(t))
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
        let raw = codec::zstd_decompress(bytes).map_err(|e| e.to_string())?;
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
                    let w = (shape * strength).min(1.0);
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
                    let shift = amount * weigher.weight(u, v) * across * outward;
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
    edit.par_voxels(&positions, |pos, chunk| {
        let (x0, y0, z0) = pos.origin();
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let (x, z) = (x0 + lx, z0 + lz);
                if brush.weight(x as f32 + 0.5 - cx, z as f32 + 0.5 - cz) < 0.5 {
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
        if solid {
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
