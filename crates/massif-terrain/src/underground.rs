//! What a graph puts under the ground, written into a map: the cave layers
//! of its Cave Layer nodes, the cave biomes of its Cave Biome nodes, and
//! the volumes it carves out of the ground or adds to it — what the active
//! Map Output's `caves`, `cave_biomes`, `volume.carve` and `volume.add`
//! take. Apply calls [`apply`] after it has written the ground.
//!
//! - Caves: a Cave Layer node's settings become one of the map's cave
//!   layers, marked with the node's id ([`CaveDef::graph`]): updated in
//!   place on the next Apply, removed when the node is gone, never mixed
//!   with layers painted by hand. Columns take the layers the graph gives
//!   them ([`ops::fill_caves_from`]); where it gives none, a layer painted
//!   by hand stays.
//! - Cave biomes: painted in the cells the graph gives them, between the
//!   node's heights ([`ops::fill_cave_biomes_from`]); other cells keep
//!   theirs.
//! - Volumes: the map's sculpted chunks (16³ blocks) where the volumes
//!   reach, each starting from what the map composes there (its caves
//!   included): air where `volume.carve` is negative, rock of the column's
//!   terrain where `volume.add` is.

use crate::compile::Compiled;
use crate::engine::{Cancel, Engine, EvalError};
use crate::kernel::{BRICK, Brick, Grid};
use crate::nodes::caves::{BIOME, LAYER, MERGE, biome_of, both, def_of, key};
use crate::space::Rect;
use massif_core::caves::{CaveDef, CaveLayers};
use massif_core::history::Edit;
use massif_core::voxel::{ChunkPos, SOLID, VoxelChunk};
use massif_core::world::Dimension;
use massif_core::{BlockRect, Registry, TILE_SIZE, TilePos, ops};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What [`apply`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// The graph's cave layers now on the map.
    pub cave_layers: usize,
    /// Columns whose cave layer changed.
    pub cave_columns: usize,
    /// Cells of 4³ blocks given a cave biome.
    pub biome_cells: usize,
    /// Sculpted chunks written by the volumes.
    pub chunks: usize,
}

/// Tiles worked out at once: their samples, then the map's edit.
const BATCH: usize = 64;

/// Writes into `edit` (the ground level of a dimension of a world whose
/// registry is `reg`) what the active Map Output of `c` puts under the
/// ground, over the tiles of the map the graph covers.
pub fn apply(engine: &Engine, c: &Compiled, reg: &Registry, edit: &mut Edit, cancel: Cancel) -> Result<Applied, EvalError> {
    apply_prepared(engine, c, reg, edit, None, cancel)
}

/// What [`prepare`] worked out ahead: the samples of the caves' and cave
/// biomes' outputs over each tile, and the volumes' bricks.
#[derive(Default)]
pub struct Prepared {
    grids: HashMap<((usize, u8), TilePos), Option<Grid>>,
    bricks: HashMap<BrickAt, Arc<Brick>>,
}

/// A brick of an output: the output, the brick's corner.
type BrickAt = ((usize, u8), [i32; 3]);

/// The costly part of [`apply`], off the interface's thread: everything
/// the graph puts under the ground of `dim` (as it will be when written:
/// after the ground), evaluated; [`apply_prepared`] then only writes. What
/// it did not foresee (the ground changed meanwhile) is made while
/// writing, so the result is the same either way.
pub fn prepare(engine: &Engine, c: &Compiled, dim: &Dimension, cancel: Cancel) -> Result<Prepared, EvalError> {
    let mut p = Prepared::default();
    let tiles = tiles_of(c, dim);
    let f = Fetch { engine, prepared: None };
    for name in ["caves", "cave_biomes"] {
        let Some(src) = c.output(name) else { continue };
        for batch in tiles.chunks(BATCH) {
            for &pos in batch {
                p.grids.insert((src, pos), tile_grid(&f, c, src, pos, cancel)?);
            }
        }
    }
    let (carve, add) = (c.output("volume.carve"), c.output("volume.add"));
    if carve.is_some() || add.is_some() {
        let (carve_box, add_box) = boxes(engine, c, carve, add, cancel)?;
        for batch in tiles.chunks(BATCH) {
            let (carve_at, add_at) = brick_origins(dim, carve.is_some() && carve_box != Aabb::EMPTY, add.is_some() && add_box != Aabb::EMPTY, &carve_box, &add_box, batch);
            for (src, at) in [(carve, carve_at), (add, add_at)] {
                let Some(src) = src else { continue };
                for b in engine.bricks(c, src, &at, cancel)? {
                    p.bricks.insert((src, b.origin), b);
                }
            }
        }
    }
    Ok(p)
}

/// Where the engine's results come from: worked out ahead, or made now.
struct Fetch<'a> {
    engine: &'a Engine,
    prepared: Option<&'a Prepared>,
}

impl Fetch<'_> {
    fn bricks(&self, c: &Compiled, src: (usize, u8), at: &[[i32; 3]], cancel: Cancel) -> Result<Vec<Arc<Brick>>, EvalError> {
        let Some(p) = self.prepared else { return self.engine.bricks(c, src, at, cancel) };
        let missing: Vec<[i32; 3]> = at.iter().copied().filter(|o| !p.bricks.contains_key(&(src, *o))).collect();
        let mut made = self.engine.bricks(c, src, &missing, cancel)?.into_iter();
        Ok(at.iter().map(|o| p.bricks.get(&(src, *o)).cloned().unwrap_or_else(|| made.next().expect("one each"))).collect())
    }
}

/// [`apply`] with what [`prepare`] worked out (`None`: made now).
pub fn apply_prepared(engine: &Engine, c: &Compiled, reg: &Registry, edit: &mut Edit, prepared: Option<&Prepared>, cancel: Cancel) -> Result<Applied, EvalError> {
    apply_full(engine, c, reg, edit, prepared, None, cancel)
}

/// [`apply`], keeping the blocks edited by hand that `kept` found before
/// the graph wrote anything (the ground included).
pub fn apply_keeping(engine: &Engine, c: &Compiled, reg: &Registry, edit: &mut Edit, kept: Option<HandEdits>, cancel: Cancel) -> Result<Applied, EvalError> {
    apply_full(engine, c, reg, edit, None, kept, cancel)
}

/// [`apply`] with what [`prepare`] worked out (`None`: made now), keeping
/// the blocks edited by hand that `kept` found before the graph wrote
/// anything: Apply captures them ([`HandEdits::capture`]) before it writes
/// the ground, so that what was dug, filled or flooded by hand stays where
/// it is whatever the graph changes around it. Without it (`None`), they
/// are captured here, after the ground.
pub fn apply_full(engine: &Engine, c: &Compiled, reg: &Registry, edit: &mut Edit, prepared: Option<&Prepared>, kept: Option<HandEdits>, cancel: Cancel) -> Result<Applied, EvalError> {
    let kept = kept.unwrap_or_else(|| HandEdits::capture(c, edit.dim()));
    let f = Fetch { engine, prepared };
    let engine = &f;
    let mut done = Applied::default();
    let tiles = tiles_of(c, edit.dim());
    if let Some(src) = c.output("caves") {
        let (layers, columns) = caves(engine, c, src, edit, &tiles, cancel)?;
        done.cave_layers = layers;
        done.cave_columns = columns;
    } else {
        // The graph gives no caves: none of its own stay.
        let defs = edit.dim().caves.to_vec();
        if defs.iter().any(|d| d.graph.is_some() && !d.removed) {
            edit.set_caves(CaveLayers::new(defs.into_iter().map(|d| CaveDef { removed: d.removed || d.graph.is_some(), ..d }).collect()));
        }
    }
    if let Some(src) = c.output("cave_biomes") {
        done.biome_cells = biomes(engine, c, src, reg, edit, &tiles, cancel)?;
    }
    // The sculpted chunks over what the map composes now: the graph's
    // carving of before and what followed the columns gone, hand edits
    // kept.
    let hand = kept.restore(edit);
    let (carve, add) = (c.output("volume.carve"), c.output("volume.add"));
    if carve.is_some() || add.is_some() {
        done.chunks = volumes(engine, c, carve, add, edit, &tiles, &hand, cancel)?;
    }
    Ok(done)
}

/// The map's sculpted chunks before a graph is applied, each with what the
/// map composed there without them: a block that differs from that, and
/// was not carved by a graph, was edited by hand.
pub struct HandEdits {
    chunks: Vec<(ChunkPos, Arc<VoxelChunk>, VoxelChunk)>,
}

/// Of each sculpted chunk left after [`HandEdits::restore`], its blocks
/// edited by hand.
pub type Hand = HashMap<ChunkPos, Vec<bool>>;

impl HandEdits {
    /// The sculpted chunks over the tiles the graph covers, as they are.
    pub fn capture(c: &Compiled, dim: &Dimension) -> Self {
        let tiles: HashSet<TilePos> = tiles_of(c, dim).into_iter().collect();
        let positions: Vec<ChunkPos> = dim.voxels.iter().map(|(p, _)| p).filter(|p| tiles.contains(&p.tile())).collect();
        let bare = massif_core::voxel::bare_densities(dim, &positions);
        let chunks = bare.into_iter().filter_map(|(p, b)| dim.voxels.get_shared(p).map(|c| (p, c, b))).collect();
        Self { chunks }
    }

    /// Each chunk made again over what the map composes now: its blocks
    /// edited by hand as they were, the others as the map composes them; a
    /// chunk with nothing edited by hand goes. Returns which blocks are
    /// hand edits.
    pub fn restore(self, edit: &mut Edit) -> Hand {
        let positions: Vec<ChunkPos> = self.chunks.iter().map(|c| c.0).collect();
        let now: HashMap<ChunkPos, VoxelChunk> = massif_core::voxel::bare_densities(edit.dim(), &positions).into_iter().collect();
        let mut hand = Hand::new();
        let mut put = Vec::new();
        for (pos, chunk, before) in self.chunks {
            let Some(now) = now.get(&pos) else { continue };
            let mut made = now.clone();
            let mut mine = vec![false; massif_core::voxel::VOLUME];
            let mut any = false;
            for ly in 0..BRICK {
                for lz in 0..BRICK {
                    for lx in 0..BRICK {
                        let same = |a: &VoxelChunk, b: &VoxelChunk| a.get(lx, ly, lz) == b.get(lx, ly, lz) && a.fluid(lx, ly, lz) == b.fluid(lx, ly, lz) && (a.get(lx, ly, lz) < SOLID || a.terrain(lx, ly, lz) == b.terrain(lx, ly, lz));
                        if chunk.carved(lx, ly, lz) || same(&chunk, &before) {
                            continue;
                        }
                        made.set(lx, ly, lz, chunk.get(lx, ly, lz));
                        made.set_fluid(lx, ly, lz, chunk.fluid(lx, ly, lz));
                        if chunk.get(lx, ly, lz) >= SOLID {
                            made.set_terrain(lx, ly, lz, chunk.terrain(lx, ly, lz));
                        }
                        mine[massif_core::voxel::voxel_index(lx, ly, lz)] = true;
                        any = true;
                    }
                }
            }
            if any {
                hand.insert(pos, mine);
                if made != *chunk {
                    put.push((pos, Arc::new(made)));
                }
            } else {
                edit.remove_voxels(pos);
            }
        }
        edit.set_voxels(put);
        hand
    }
}

/// The map's tiles under the graph's terrain.
fn tiles_of(c: &Compiled, dim: &Dimension) -> Vec<TilePos> {
    let s = &c.space;
    let rect = BlockRect::new(s.x0, s.z0, s.x0 + s.size as i32, s.z0 + s.size as i32);
    let ground = dim.ground();
    rect.tiles().filter(|t| ground.contains(*t)).collect()
}

/// The samples of `src` over tile `pos` (level 0), `None` where the tile
/// lies outside the graph's terrain.
fn tile_grid(f: &Fetch, c: &Compiled, src: (usize, u8), pos: TilePos, cancel: Cancel) -> Result<Option<Grid>, EvalError> {
    if let Some(g) = f.prepared.and_then(|p| p.grids.get(&(src, pos))) {
        return Ok(g.clone());
    }
    let engine = f.engine;
    let (x0, z0) = pos.origin();
    let rect = Rect::new(x0 - c.space.x0, z0 - c.space.z0, TILE_SIZE as u32, TILE_SIZE as u32).intersect(&c.space.whole(0));
    if rect.is_empty() {
        return Ok(None);
    }
    engine.region(c, src, 0, rect, cancel).map(Some)
}

/// The samples of `src` over a batch of tiles, side by side.
fn grids(engine: &Fetch, c: &Compiled, src: (usize, u8), batch: &[TilePos], cancel: Cancel) -> Result<HashMap<TilePos, Grid>, EvalError> {
    use rayon::prelude::*;
    let made: Vec<(TilePos, Option<Grid>)> = batch.par_iter().map(|&p| tile_grid(engine, c, src, p, cancel).map(|g| (p, g))).collect::<Result<_, _>>()?;
    Ok(made.into_iter().filter_map(|(p, g)| g.map(|g| (p, g))).collect())
}

/// Plane `p` of a layer at block `(x, z)`, 0 outside the grid.
fn layer_at(c: &Compiled, grids: &HashMap<TilePos, Grid>, p: usize, x: i32, z: i32) -> f32 {
    let Some(g) = grids.get(&TilePos::containing(x, z)) else { return 0.0 };
    let (sx, sz) = (x - c.space.x0, z - c.space.z0);
    if sx < g.rect.x || sz < g.rect.z || sx >= g.rect.x1() || sz >= g.rect.z1() {
        return 0.0;
    }
    g.at_plane(p, sx, sz)
}

/// The nodes `src` depends on, itself included.
fn upstream(c: &Compiled, src: usize) -> HashSet<usize> {
    let mut seen = HashSet::new();
    let mut todo = vec![src];
    while let Some(n) = todo.pop() {
        if seen.insert(n) {
            let node = &c.nodes[n];
            todo.extend(node.inputs.iter().flatten().map(|i| i.0));
            todo.extend(node.driven.iter().flatten().map(|i| i.0));
            todo.extend(node.open.iter().map(|o| o.1.0));
        }
    }
    seen
}

fn caves(engine: &Fetch, c: &Compiled, src: (usize, u8), edit: &mut Edit, tiles: &[TilePos], cancel: Cancel) -> Result<(usize, usize), EvalError> {
    // The graph's layers: those its Cave Layer nodes feeding the output
    // make, in place of those it made before.
    let mut defs = edit.dim().caves.to_vec();
    let mut number: HashMap<u32, u16> = HashMap::new();
    let mut kept: HashSet<usize> = HashSet::new();
    let mut ids: Vec<usize> = upstream(c, src.0).into_iter().filter(|&n| c.nodes[n].spec.id == LAYER.id && c.nodes[n].broken.is_none()).collect();
    ids.sort_by(|a, b| c.nodes[*a].id.cmp(&c.nodes[*b].id));
    for n in ids {
        let node = &c.nodes[n];
        let made = def_of(&node.params, node.seed, &node.id, &format!("Graph: {}", node.id));
        let k = match defs.iter().position(|d| d.graph.as_deref() == Some(node.id.as_str())) {
            Some(k) => {
                let old = &defs[k];
                defs[k] = CaveDef { name: old.name.clone(), color: old.color, locked: old.locked, order: old.order, ..made };
                k
            }
            None => {
                defs.push(made);
                defs.len() - 1
            }
        };
        kept.insert(k);
        number.insert(key(LAYER.id, &node.params, node.seed), (k + 1) as u16);
    }
    for (k, d) in defs.iter_mut().enumerate() {
        if d.graph.is_some() && !kept.contains(&k) {
            d.removed = true;
        }
    }
    edit.set_caves(CaveLayers::new(defs));
    let mut changed = 0;
    for batch in tiles.chunks(BATCH) {
        let grids = grids(engine, c, src, batch, cancel)?;
        changed += ops::fill_caves_from(edit, batch, |x, z| {
            let k = layer_at(c, &grids, 0, x, z) as u32;
            match number.get(&k) {
                Some(&n) => (n, layer_at(c, &grids, 1, x, z)),
                None => (0, 0.0),
            }
        });
    }
    Ok((number.len(), changed))
}

/// Cave biomes by the key standing for them: each with its heights.
type BiomeKeys = HashMap<u32, Vec<(u16, i32, i32)>>;
/// A chunk's bricks: carved, added.
type ChunkBricks = (Option<Arc<Brick>>, Option<Arc<Brick>>);

/// The cave biomes each key the layer at `src` may hold stands for: one
/// for a Cave Biome node's, those of both for a Merge ▸ Both.
fn biome_keys(c: &Compiled, reg: &Registry, src: usize, memo: &mut HashMap<usize, BiomeKeys>) -> BiomeKeys {
    if let Some(m) = memo.get(&src) {
        return m.clone();
    }
    let node = &c.nodes[src];
    let mut out: BiomeKeys = HashMap::new();
    if node.broken.is_some() {
    } else if node.spec.id == BIOME.id {
        if let Some(biome) = reg.biome_by_id(biome_of(&node.params)) {
            let (lo, hi) = (node.params.f("low") as i32, node.params.f("high") as i32);
            out.insert(key(BIOME.id, &node.params, node.seed), vec![(biome.0, lo.min(hi), lo.max(hi) + 1)]);
        }
    } else {
        let ins: Vec<BiomeKeys> = node.inputs.iter().flatten().map(|i| biome_keys(c, reg, i.0, memo)).collect();
        for m in &ins {
            out.extend(m.iter().map(|(k, v)| (*k, v.clone())));
        }
        if node.spec.id == MERGE.id && node.params.choice("mode") == "both" && ins.len() == 2 {
            for (ka, va) in &ins[0] {
                for (kb, vb) in &ins[1] {
                    out.insert(both(*ka, *kb), va.iter().chain(vb).copied().collect());
                }
            }
        }
    }
    memo.insert(src, out.clone());
    out
}

fn biomes(engine: &Fetch, c: &Compiled, src: (usize, u8), reg: &Registry, edit: &mut Edit, tiles: &[TilePos], cancel: Cancel) -> Result<usize, EvalError> {
    let by_key = biome_keys(c, reg, src.0, &mut HashMap::new());
    if by_key.is_empty() {
        return Ok(0);
    }
    let mut changed = 0;
    for batch in tiles.chunks(BATCH) {
        let grids = grids(engine, c, src, batch, cancel)?;
        changed += ops::fill_cave_biomes_from(edit, batch, |x, z, out| match by_key.get(&(layer_at(c, &grids, 0, x, z) as u32)) {
            Some(b) => {
                out.extend_from_slice(b);
                layer_at(c, &grids, 1, x, z)
            }
            None => 0.0,
        });
    }
    Ok(changed)
}

/// How much of a block a volume's value fills, as a voxel's density: all
/// of it half a block inside, none half a block outside.
#[inline]
pub fn fill(d: f32) -> u8 {
    ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Whether a brick reaches anything.
fn reaches(b: &Brick) -> bool {
    b.data.iter().any(|&v| fill(v) > 0)
}

#[allow(clippy::too_many_arguments)]
fn volumes(engine: &Fetch, c: &Compiled, carve: Option<(usize, u8)>, add: Option<(usize, u8)>, edit: &mut Edit, tiles: &[TilePos], hand: &Hand, cancel: Cancel) -> Result<usize, EvalError> {
    let (carve_box, add_box) = boxes(engine.engine, c, carve, add, cancel)?;
    let mut written = 0;
    for batch in tiles.chunks(BATCH) {
        let (carve_at, add_at) = brick_origins(edit.dim(), carve.is_some(), add.is_some(), &carve_box, &add_box, batch);
        let mut bricks: HashMap<ChunkPos, ChunkBricks> = HashMap::new();
        for (src, at, slot) in [(carve, &carve_at, 0), (add, &add_at, 1)] {
            let Some(src) = src else { continue };
            for b in engine.bricks(c, src, at, cancel)? {
                if reaches(&b) {
                    let [x, y, z] = b.origin;
                    let e = bricks.entry(ChunkPos::containing(x, y, z)).or_default();
                    if slot == 0 {
                        e.0 = Some(b);
                    } else {
                        e.1 = Some(b);
                    }
                }
            }
        }
        if bricks.is_empty() {
            continue;
        }
        let mut positions: Vec<ChunkPos> = bricks.keys().copied().collect();
        positions.sort_by_key(|p| (p.x, p.z, p.y));
        edit.par_voxels(&positions, |pos, chunk| {
            let (carved, added) = &bricks[&pos];
            shape(chunk, carved.as_deref(), added.as_deref(), hand.get(&pos));
        });
        written += positions.iter().filter(|p| edit.dim().voxels.get(**p).is_some()).count();
    }
    Ok(written)
}

/// Where the volumes carving (`carve`) and adding (`add`) can reach at all:
/// bricks outside hold nothing, and most of a map's bricks are outside a
/// tunnel's or a hall's.
fn boxes(engine: &Engine, c: &Compiled, carve: Option<(usize, u8)>, add: Option<(usize, u8)>, cancel: Cancel) -> Result<(Aabb, Aabb), EvalError> {
    let mut memo = HashMap::new();
    let carve_box = match carve {
        Some(src) => reach(engine, c, src.0, cancel, &mut memo)?,
        None => Aabb::EMPTY,
    };
    let add_box = match add {
        Some(src) => reach(engine, c, src.0, cancel, &mut memo)?,
        None => Aabb::EMPTY,
    };
    Ok((carve_box, add_box))
}

/// The bricks of each tile of `batch` to make: a carving reaches no
/// higher than the highest matter of the tile's levels (nothing to cut
/// above); added rock anywhere.
fn brick_origins(dim: &Dimension, carve: bool, add: bool, carve_box: &Aabb, add_box: &Aabb, batch: &[TilePos]) -> (Vec<[i32; 3]>, Vec<[i32; 3]>) {
    let (min_y, max_y) = (dim.min_y, dim.max_y);
    let per = TILE_SIZE / BRICK;
    let (cy0, cy1) = (min_y.div_euclid(BRICK), (max_y - 1).div_euclid(BRICK));
    let mut carve_at: Vec<[i32; 3]> = Vec::new();
    let mut add_at: Vec<[i32; 3]> = Vec::new();
    for &t in batch {
        let top = highest(dim, t).map_or(cy0 - 1, |h| (h.ceil() as i32).div_euclid(BRICK).min(cy1));
        let (x0, z0) = t.origin();
        for bz in 0..per {
            for bx in 0..per {
                let (x, z) = (x0 + bx * BRICK, z0 + bz * BRICK);
                for cy in cy0..=cy1 {
                    let o = [x, cy * BRICK, z];
                    if carve && cy <= top && carve_box.meets(o) {
                        carve_at.push(o);
                    }
                    if add && add_box.meets(o) {
                        add_at.push(o);
                    }
                }
            }
        }
    }
    (carve_at, add_at)
}

/// A box of the world a volume reaches no further than (its value is
/// positive outside); infinite where it cannot say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

impl Aabb {
    pub const ALL: Aabb = Aabb { lo: [f32::NEG_INFINITY; 3], hi: [f32::INFINITY; 3] };
    pub const EMPTY: Aabb = Aabb { lo: [f32::INFINITY; 3], hi: [f32::NEG_INFINITY; 3] };

    fn around(c: [f32; 3], r: [f32; 3]) -> Self {
        Aabb { lo: [c[0] - r[0], c[1] - r[1], c[2] - r[2]], hi: [c[0] + r[0], c[1] + r[1], c[2] + r[2]] }
    }

    fn union(self, o: Aabb) -> Self {
        Aabb { lo: [0, 1, 2].map(|k| self.lo[k].min(o.lo[k])), hi: [0, 1, 2].map(|k| self.hi[k].max(o.hi[k])) }
    }

    fn meet(self, o: Aabb) -> Self {
        Aabb { lo: [0, 1, 2].map(|k| self.lo[k].max(o.lo[k])), hi: [0, 1, 2].map(|k| self.hi[k].min(o.hi[k])) }
    }

    fn grow(self, m: f32) -> Self {
        Aabb { lo: self.lo.map(|v| v - m), hi: self.hi.map(|v| v + m) }
    }

    /// Whether it meets the brick at `o`.
    pub fn meets(&self, o: [i32; 3]) -> bool {
        (0..3).all(|k| self.lo[k] <= (o[k] + BRICK) as f32 && self.hi[k] >= o[k] as f32)
    }
}

/// How far the volume node `n` reaches, from its settings and inputs.
fn reach(engine: &Engine, c: &Compiled, n: usize, cancel: Cancel, memo: &mut HashMap<usize, Aabb>) -> Result<Aabb, EvalError> {
    if let Some(b) = memo.get(&n) {
        return Ok(*b);
    }
    let node = &c.nodes[n];
    let p = &node.params;
    let space = &c.space;
    let at = |fx: &str, fz: &str| [space.x0 as f32 + p.f(fx) * space.size as f32, space.z0 as f32 + p.f(fz) * space.size as f32];
    let geo = |k: usize| -> Result<Option<Arc<crate::kernel::Geo>>, EvalError> { node.inputs.get(k).copied().flatten().map(|src| engine.geometry(c, src, cancel)).transpose() };
    let input = |k: usize, memo: &mut HashMap<usize, Aabb>| -> Result<Aabb, EvalError> {
        match node.inputs.get(k).copied().flatten() {
            Some(src) if c.kind(src).is_volume() => reach(engine, c, src.0, cancel, memo),
            _ => Ok(Aabb::EMPTY),
        }
    };
    let b = match node.spec.id {
        "volume.sphere" => {
            let (r, squash) = (p.f("radius").max(0.5), p.f("squash").max(0.1));
            match geo(0)?.as_deref().and_then(crate::kernel::Geo::points) {
                Some(points) => points.items.iter().fold(Aabb::EMPTY, |b, q| {
                    let r = r * q.scale.max(0.01);
                    b.union(Aabb::around([q.pos[0] as f32, q.pos[1] as f32 - p.f("depth"), q.pos[2] as f32], [r, r * squash, r]))
                }),
                None => {
                    let [x, z] = at("x", "z");
                    Aabb::around([x, p.f("y"), z], [r, r * squash, r])
                }
            }
            .grow(1.0)
        }
        "volume.box" => {
            let [x, z] = at("x", "z");
            let (w, h, l) = (p.f("width") / 2.0, p.f("height") / 2.0, p.f("length") / 2.0);
            let xz = w.hypot(l);
            Aabb::around([x, p.f("y"), z], [xz, h, xz]).grow(1.0)
        }
        "volume.tube" => {
            let var = p.f("variation").clamp(0.0, 1.0);
            let depth = p.f("depth");
            let swell = 1.0 + 1.5 * var;
            match geo(0)?.as_deref().and_then(crate::kernel::Geo::splines) {
                Some(lines) => lines.lines.iter().fold(Aabb::EMPTY, |b, l| {
                    let r = if p.f("radius") > 0.0 { p.f("radius") } else { l.width / 2.0 }.max(0.5) * swell;
                    l.points.iter().fold(b, |b, q| b.union(Aabb::around([q[0] as f32, q[1] as f32 - depth, q[2] as f32], [r; 3])))
                }),
                None => {
                    let r = p.f("radius").max(0.5) * swell;
                    p.points("path").iter().fold(Aabb::EMPTY, |b, q| {
                        b.union(Aabb::around([space.x0 as f32 + q.x as f32 * space.size as f32, p.f("y") - depth, space.z0 as f32 + q.z as f32 * space.size as f32], [r; 3]))
                    })
                }
            }
            .grow(1.0)
        }
        "volume.caves" => Aabb { lo: [f32::NEG_INFINITY, p.f("low") - 1.0, f32::NEG_INFINITY], hi: [f32::INFINITY, p.f("high") + 1.0, f32::INFINITY] },
        "volume.combine" => {
            let (a, b) = (input(0, memo)?, input(1, memo)?);
            let k = p.f("smooth").max(0.0);
            match p.choice("op") {
                "subtract" => a,
                "intersect" => a.meet(b).grow(k),
                _ => a.union(b).grow(k),
            }
        }
        "volume.shape" => input(0, memo)?.grow(p.f("grow").max(0.0) + p.f("roughness").max(0.0) + 1.0),
        "volume.limit" => {
            let fade = p.f("fade").max(0.0);
            input(0, memo)?.meet(Aabb { lo: [f32::NEG_INFINITY, p.f("low") - fade, f32::NEG_INFINITY], hi: [f32::INFINITY, p.f("high") + fade, f32::INFINITY] })
        }
        _ => Aabb::ALL,
    };
    memo.insert(n, b);
    Ok(b)
}

/// The highest top of the levels over tile `t`.
fn highest(dim: &Dimension, t: TilePos) -> Option<f32> {
    dim.levels
        .iter()
        .filter_map(|l| l.view(t))
        .map(|tile| (0..massif_core::TILE_AREA).map(|i| tile.top.get(i)).fold(f32::MIN, f32::max))
        .reduce(f32::max)
}

/// A chunk cut by `carve` and filled by `add`.
fn shape(chunk: &mut VoxelChunk, carve: Option<&Brick>, add: Option<&Brick>, hand: Option<&Vec<bool>>) {
    for ly in 0..BRICK {
        for lz in 0..BRICK {
            for lx in 0..BRICK {
                let n = ((ly * BRICK + lz) * BRICK + lx) as usize;
                // What was edited by hand stays as it is.
                if hand.is_some_and(|h| h[n]) {
                    continue;
                }
                let was = chunk.get(lx, ly, lz);
                let mut d = was;
                if let Some(c) = carve {
                    d = d.min(255 - fill(c.data[n]));
                }
                if let Some(a) = add {
                    let f = fill(a.data[n]);
                    if f > d {
                        d = f;
                        if f >= SOLID {
                            chunk.set_fluid(lx, ly, lz, None);
                        }
                    }
                }
                if d != was {
                    // Known as the graph's, so that the next Apply may
                    // change it back.
                    chunk.set_carved(lx, ly, lz);
                }
                chunk.set(lx, ly, lz, d);
            }
        }
    }
}

/// A volume cut at height `y` over `rect` (samples of level 0 from the
/// terrain's corner), as a mask: 1 inside, 0 outside, the edge in between —
/// the panel's preview of a volume, and a 2D cut of it.
pub fn slice(engine: &Engine, c: &Compiled, src: (usize, u8), y: i32, rect: Rect, cancel: Cancel) -> Result<Grid, EvalError> {
    let rect = rect.intersect(&c.space.whole(0));
    let mut out = Grid::filled(0, rect, 0.0);
    if rect.is_empty() {
        return Ok(out);
    }
    let by = y.div_euclid(BRICK) * BRICK;
    let (wx0, wz0) = (c.space.x0 + rect.x, c.space.z0 + rect.z);
    let (wx1, wz1) = (wx0 + rect.w as i32, wz0 + rect.h as i32);
    let mut origins = Vec::new();
    for bz in wz0.div_euclid(BRICK)..=(wz1 - 1).div_euclid(BRICK) {
        for bx in wx0.div_euclid(BRICK)..=(wx1 - 1).div_euclid(BRICK) {
            origins.push([bx * BRICK, by, bz * BRICK]);
        }
    }
    for b in engine.bricks(c, src, &origins, cancel)? {
        for k in 0..BRICK {
            for i in 0..BRICK {
                let (x, z) = (b.origin[0] + i, b.origin[2] + k);
                if x < wx0 || x >= wx1 || z < wz0 || z >= wz1 {
                    continue;
                }
                let v = b.at(x, y, z);
                out.data[((z - wz0) as u32 * rect.w + (x - wx0) as u32) as usize] = fill(v) as f32 / 255.0;
            }
        }
    }
    Ok(out)
}
