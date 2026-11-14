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
use crate::coords::{TILE_SIZE, TilePos, local_index};
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

/// The densities of a 16³ block of the world.
#[derive(Clone, PartialEq, Eq)]
pub struct VoxelChunk {
    density: Box<[u8; VOLUME]>,
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
        }
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

    /// Serialize every chunk: `x y z` (i32) then the densities, zstd'd.
    pub fn encode(&self) -> Vec<u8> {
        let mut positions: Vec<&ChunkPos> = self.chunks.keys().collect();
        positions.sort();
        let mut raw = Vec::with_capacity(positions.len() * (12 + VOLUME));
        for p in positions {
            raw.extend_from_slice(&p.x.to_le_bytes());
            raw.extend_from_slice(&p.y.to_le_bytes());
            raw.extend_from_slice(&p.z.to_le_bytes());
            raw.extend_from_slice(&self.chunks[p].density[..]);
        }
        codec::zstd_compress(&raw)
    }

    /// Read what [`VoxelLayer::encode`] wrote.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let raw = codec::zstd_decompress(bytes).map_err(|e| e.to_string())?;
        let entry = 12 + VOLUME;
        if raw.len() % entry != 0 {
            return Err("the sculpted chunks have a ragged length".into());
        }
        let mut layer = Self::default();
        for e in raw.chunks(entry) {
            let at = |i: usize| i32::from_le_bytes(e[i..i + 4].try_into().expect("4 bytes"));
            let pos = ChunkPos::new(at(0), at(4), at(8));
            let density: [u8; VOLUME] = e[12..].try_into().expect("a full chunk");
            layer.insert(pos, Arc::new(VoxelChunk::from_densities(density)));
        }
        layer.dirty.clear();
        Ok(layer)
    }
}

// ------------------------------------------------------------ composing --

/// Mark for spans made by sculpting while a column is composed; resolved
/// before [`apply`] returns.
const SCULPTED: crate::registry::TerrainId = crate::registry::TerrainId(u16::MAX);

/// Replace what `spans` holds inside each chunk of `column` (chunk y, chunk;
/// bottom up) by the chunk's solid voxels at local column `(lx, lz)`.
///
/// Sculpted rock open to the sky takes the terrain of the column's surface;
/// rock under something else is plain rock, as under a solid level.
pub(crate) fn apply(spans: &mut Vec<Span>, column: &[(i32, &VoxelChunk)], lx: i32, lz: i32, min_y: i32, max_y: i32, surface: crate::registry::TerrainId) {
    // The exact heights of the tops the levels had, for runs that end where
    // one did and whose block above is not sculpted.
    let before: Vec<(i32, f32)> = spans.iter().map(|s| (s.y1, s.top)).collect();
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
            let start = y;
            while y < yb && chunk.solid(lx, y - base, lz) {
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
                    terrain: SCULPTED,
                    top: top.clamp(y as f32 - 0.5, y as f32 + 0.499),
                },
            );
        }
    }
    // Sculpted runs continue the spans they touch.
    let mut i = 0;
    while i + 1 < spans.len() {
        let (a, b) = (spans[i], spans[i + 1]);
        if a.y1 == b.y0 && (a.terrain == SCULPTED || b.terrain == SCULPTED) {
            spans[i] = Span { y0: a.y0, ..b };
            spans.remove(i + 1);
        } else {
            i += 1;
        }
    }
    let last = spans.len().saturating_sub(1);
    for (i, s) in spans.iter_mut().enumerate() {
        if s.terrain == SCULPTED {
            s.terrain = if i == last { surface } else { crate::registry::TerrainId::ROCK };
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
    let mut chunk = VoxelChunk::air();
    let (x0, y0, z0) = pos.origin();
    let Some(stack) = TileStack::new(dim, pos.tile(), false) else {
        return chunk;
    };
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
    let normal = if n > 1e-3 {
        [-g[0] / n, -g[1] / n, -g[2] / n]
    } else {
        let (x, y, z) = hit.normal;
        [x as f32, y as f32, z as f32]
    };
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
            let values = f.values.iter().zip(&b.values).map(|(v, b)| v + by * (b - v)).collect();
            Field { values, ..f }
        };
        step(step(field, SHRINK), INFLATE)
    });

    let strength = strength.clamp(0.0, 1.0);
    let profile = brush.profile;
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
                            brush.weight(u, v) * across
                        }
                        None => {
                            let d = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() / r;
                            if d > 1.0 {
                                continue;
                            }
                            profile.apply(d).clamp(0.0, 1.0)
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
    // The matter as it is, to read it moved from.
    let field = Field::gather(edit.dim(), lo, hi, amount.ceil() as i32 + 2);
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
                    let shift = amount * brush.weight(u, v) * across * outward;
                    if shift.abs() < 0.01 {
                        continue;
                    }
                    // What lies `shift` blocks back along the normal comes here.
                    let moved = field.sample([at[0] - normal[0] * shift, at[1] - normal[1] * shift, at[2] - normal[2] * shift]);
                    let old = chunk.get(lx, ly, lz);
                    let new = if outward > 0.0 { (old as f32).max(moved) } else { (old as f32).min(moved) };
                    let new = new.round().clamp(0.0, 255.0) as u8;
                    if new != old {
                        chunk.set(lx, ly, lz, new);
                        changed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        }
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
        let filled: Vec<(ChunkPos, VoxelChunk)> = chunks.par_iter().map(|&p| (p, densities_at(dim, p))).collect();
        for (pos, chunk) in filled {
            let (x0, y0, z0) = pos.origin();
            for ly in 0..SIZE {
                for lz in 0..SIZE {
                    for lx in 0..SIZE {
                        let (x, y, z) = (x0 + lx, y0 + ly, z0 + lz);
                        if x < lo[0] || y < lo[1] || z < lo[2] || x >= hi[0] || y >= hi[1] || z >= hi[2] {
                            continue;
                        }
                        let i = ((y - lo[1]) as usize * size[2] + (z - lo[2]) as usize) * size[0] + (x - lo[0]) as usize;
                        values[i] = chunk.get(lx, ly, lz) as f32;
                    }
                }
            }
        }
        Self { lo, size, values }
    }

    /// Two passes of a separable box blur of radius `k`: close to a
    /// Gaussian, the same in every direction.
    fn blurred(mut self, k: i32) -> Self {
        let [sx, sy, sz] = self.size;
        let k = k as usize;
        let mut tmp = vec![0f32; self.values.len()];
        for _ in 0..2 {
            for axis in 0..3 {
                let (len, stride) = match axis {
                    0 => (sx, 1),
                    1 => (sy, sx * sz),
                    _ => (sz, sx),
                };
                let lines: Vec<usize> = (0..self.values.len()).filter(|&i| {
                    let x = i % sx;
                    let z = (i / sx) % sz;
                    let y = i / (sx * sz);
                    match axis {
                        0 => x == 0,
                        1 => y == 0,
                        _ => z == 0,
                    }
                }).collect();
                for start in lines {
                    let at = |t: usize| start + t * stride;
                    let mut sum = 0f32;
                    let mut count = 0f32;
                    for t in 0..len.min(k + 1) {
                        sum += self.values[at(t)];
                        count += 1.0;
                    }
                    for t in 0..len {
                        tmp[at(t)] = sum / count;
                        if t + k + 1 < len {
                            sum += self.values[at(t + k + 1)];
                            count += 1.0;
                        }
                        if t >= k {
                            sum -= self.values[at(t - k)];
                            count -= 1.0;
                        }
                    }
                }
                std::mem::swap(&mut self.values, &mut tmp);
            }
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
