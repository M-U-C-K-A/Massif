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
    let mut done = Applied::default();
    let tiles = tiles_of(c, edit);
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
    let (carve, add) = (c.output("volume.carve"), c.output("volume.add"));
    if carve.is_some() || add.is_some() {
        done.chunks = volumes(engine, c, carve, add, edit, &tiles, cancel)?;
    }
    Ok(done)
}

/// The map's tiles under the graph's terrain.
fn tiles_of(c: &Compiled, edit: &Edit) -> Vec<TilePos> {
    let s = &c.space;
    let rect = BlockRect::new(s.x0, s.z0, s.x0 + s.size as i32, s.z0 + s.size as i32);
    let ground = edit.dim().ground();
    rect.tiles().filter(|t| ground.contains(*t)).collect()
}

/// The samples of `src` over tile `pos` (level 0), `None` where the tile
/// lies outside the graph's terrain.
fn tile_grid(engine: &Engine, c: &Compiled, src: (usize, u8), pos: TilePos, cancel: Cancel) -> Result<Option<Grid>, EvalError> {
    let (x0, z0) = pos.origin();
    let rect = Rect::new(x0 - c.space.x0, z0 - c.space.z0, TILE_SIZE as u32, TILE_SIZE as u32).intersect(&c.space.whole(0));
    if rect.is_empty() {
        return Ok(None);
    }
    engine.region(c, src, 0, rect, cancel).map(Some)
}

/// The samples of `src` over a batch of tiles, side by side.
fn grids(engine: &Engine, c: &Compiled, src: (usize, u8), batch: &[TilePos], cancel: Cancel) -> Result<HashMap<TilePos, Grid>, EvalError> {
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

fn caves(engine: &Engine, c: &Compiled, src: (usize, u8), edit: &mut Edit, tiles: &[TilePos], cancel: Cancel) -> Result<(usize, usize), EvalError> {
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

fn biomes(engine: &Engine, c: &Compiled, src: (usize, u8), reg: &Registry, edit: &mut Edit, tiles: &[TilePos], cancel: Cancel) -> Result<usize, EvalError> {
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

fn volumes(engine: &Engine, c: &Compiled, carve: Option<(usize, u8)>, add: Option<(usize, u8)>, edit: &mut Edit, tiles: &[TilePos], cancel: Cancel) -> Result<usize, EvalError> {
    let (min_y, max_y) = (edit.dim().min_y, edit.dim().max_y);
    let per = TILE_SIZE / BRICK;
    let (cy0, cy1) = (min_y.div_euclid(BRICK), (max_y - 1).div_euclid(BRICK));
    // Where the volumes can reach at all: bricks outside hold nothing,
    // and most of a map's bricks are outside a tunnel's or a hall's.
    let mut memo = HashMap::new();
    let carve_box = match carve {
        Some(src) => reach(engine, c, src.0, cancel, &mut memo)?,
        None => Aabb::EMPTY,
    };
    let add_box = match add {
        Some(src) => reach(engine, c, src.0, cancel, &mut memo)?,
        None => Aabb::EMPTY,
    };
    let mut written = 0;
    for batch in tiles.chunks(BATCH) {
        // The bricks of each tile: a carving reaches no higher than the
        // highest matter of the tile's levels (nothing to cut above);
        // added rock anywhere.
        let mut carve_at: Vec<[i32; 3]> = Vec::new();
        let mut add_at: Vec<[i32; 3]> = Vec::new();
        for &t in batch {
            let top = highest(edit, t).map_or(cy0 - 1, |h| (h.ceil() as i32).div_euclid(BRICK).min(cy1));
            let (x0, z0) = t.origin();
            for bz in 0..per {
                for bx in 0..per {
                    let (x, z) = (x0 + bx * BRICK, z0 + bz * BRICK);
                    for cy in cy0..=cy1 {
                        let o = [x, cy * BRICK, z];
                        if carve.is_some() && cy <= top && carve_box.meets(o) {
                            carve_at.push(o);
                        }
                        if add.is_some() && add_box.meets(o) {
                            add_at.push(o);
                        }
                    }
                }
            }
        }
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
            shape(chunk, carved.as_deref(), added.as_deref());
        });
        written += positions.iter().filter(|p| edit.dim().voxels.get(**p).is_some()).count();
    }
    Ok(written)
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
fn highest(edit: &Edit, t: TilePos) -> Option<f32> {
    edit.dim()
        .levels
        .iter()
        .filter_map(|l| l.view(t))
        .map(|tile| (0..massif_core::TILE_AREA).map(|i| tile.top.get(i)).fold(f32::MIN, f32::max))
        .reduce(f32::max)
}

/// A chunk cut by `carve` and filled by `add`.
fn shape(chunk: &mut VoxelChunk, carve: Option<&Brick>, add: Option<&Brick>) {
    for ly in 0..BRICK {
        for lz in 0..BRICK {
            for lx in 0..BRICK {
                let n = ((ly * BRICK + lz) * BRICK + lx) as usize;
                let mut d = chunk.get(lx, ly, lz);
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
