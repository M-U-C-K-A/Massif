//! The water of a map as the game will find it: where it stays still and
//! where, the first time a block beside it changes, it would run.
//!
//! Minecraft keeps the water of an exported world still until something
//! next to it is updated: a block placed or broken, a plant trampled. Then
//! every water block with open air beside it runs into that air. Water
//! that only stands next to rock or other water stays as it is. [`check`]
//! finds the water blocks with air beside them, as the export composes the
//! columns (ground, levels, carves and sculpted chunks).

use crate::compose::{Span, TileStack};
use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::world::Dimension;
use rayon::prelude::*;

/// What [`check`] found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WaterCheck {
    /// Columns holding water, and water blocks on the top of their water.
    pub columns: u64,
    /// Top water blocks with air beside them over lower water: a step
    /// down along a river, where the water runs on over the lower one.
    pub steps: u64,
    /// Top water blocks with air beside them over water more than a block
    /// lower: a fall.
    pub falls: u64,
    /// Top water blocks with air beside them over land: water that runs
    /// out of its bed.
    pub spills: u64,
    /// Water blocks with air under them (none, if the export is sound).
    pub floating: u64,
    /// Top water blocks at the edge of the map, beside nothing.
    pub edge: u64,
}

impl WaterCheck {
    fn add(mut self, o: Self) -> Self {
        self.columns += o.columns;
        self.steps += o.steps;
        self.falls += o.falls;
        self.spills += o.spills;
        self.floating += o.floating;
        self.edge += o.edge;
        self
    }

    /// The water blocks that would run once a block next to them changes.
    pub fn running(&self) -> u64 {
        self.steps + self.falls + self.spills + self.floating
    }
}

/// What lies at a height in a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Solid,
    Water,
    Air,
}

/// A column as the export writes it: its solid runs and its water runs,
/// bottom up.
#[derive(Debug, Clone, Default)]
pub struct Column {
    pub solid: Vec<(i32, i32)>,
    pub water: Vec<(i32, i32)>,
}

impl Column {
    pub fn at(&self, y: i32) -> Cell {
        if self.solid.iter().any(|s| s.0 <= y && y < s.1) {
            Cell::Solid
        } else if self.water.iter().any(|w| w.0 <= y && y < w.1) {
            Cell::Water
        } else {
            Cell::Air
        }
    }

    /// The top of the water or the ground under `y` (exclusive), the
    /// highest first: what water running over at `y` lands on.
    fn floor_under(&self, y: i32) -> Option<(i32, Cell)> {
        let s = self.solid.iter().filter(|s| s.1 <= y).map(|s| (s.1, Cell::Solid));
        let w = self.water.iter().filter(|w| w.1 <= y).map(|w| (w.1, Cell::Water));
        s.chain(w).max_by_key(|c| (c.0, c.1 == Cell::Water))
    }
}

/// The columns of the tile at `pos`, and those `margin` blocks around it,
/// as the export composes them (`None` off the map), row by row.
pub fn columns_around(dim: &Dimension, pos: TilePos, margin: i32) -> Vec<Option<Column>> {
    columns_with(dim, pos, margin, None)
}

/// The column of the ground alone: rock up to its top, its water over it.
fn ground_column(dim: &Dimension, top: i32, fluid: i16) -> Column {
    let top = top.clamp(dim.min_y, dim.max_y);
    let mut column = Column {
        solid: if top > dim.min_y { vec![(dim.min_y, top)] } else { Vec::new() },
        water: Vec::new(),
    };
    if fluid != crate::tile::NO_FLUID && fluid as i32 > top {
        column.water.push((top, (fluid as i32).min(dim.max_y)));
    }
    column
}

/// [`columns_around`], the columns of the other tiles taken from the edges
/// `planner` keeps of them where it has them.
fn columns_with(dim: &Dimension, pos: TilePos, margin: i32, planner: Option<&Planner>) -> Vec<Option<Column>> {
    columns_in(dim, pos.rect().expand(margin), Some(pos), planner)
}

/// The columns of `rect` as the export composes them (`None` off the map),
/// row by row; those of tiles other than `own` from the edges `planner`
/// keeps of them where it has them.
fn columns_in(dim: &Dimension, rect: crate::coords::BlockRect, own: Option<TilePos>, planner: Option<&Planner>) -> Vec<Option<Column>> {
    let (w, d) = (rect.width(), rect.depth());
    let mut out = vec![None; (w * d) as usize];
    let (mut spans, mut fluids): (Vec<Span>, Vec<(i32, i32)>) = (Vec::new(), Vec::new());
    let mut stacks: std::collections::HashMap<TilePos, Option<TileStack>> = std::collections::HashMap::new();
    for j in 0..d {
        for i in 0..w {
            let (x, z) = (rect.x0 + i, rect.z0 + j);
            let p = TilePos::containing(x, z);
            let cell = local_index(x, z);
            if Some(p) != own
                && let Some(ring) = planner.and_then(|pl| pl.rings.get(&p))
            {
                if let Some(&(top, fluid)) = ring.get(ring_index(cell))
                    && top != i16::MIN
                {
                    out[(j * w + i) as usize] = Some(ground_column(dim, top as i32, fluid));
                }
                continue;
            }
            let Some(stack) = stacks.entry(p).or_insert_with(|| TileStack::new(dim, p, false)) else { continue };
            if !stack.ground().present(cell) {
                continue;
            }
            if !stack.is_sculpted() && stack.layers().nth(1).is_none() {
                let g = stack.ground();
                out[(j * w + i) as usize] = Some(ground_column(dim, crate::compose::block_top(g.top.get(cell)), g.fluid.get(cell)));
                continue;
            }
            stack.spans(cell, &mut spans);
            stack.fluids(cell, &spans, &mut fluids);
            let mut column = Column {
                solid: spans.iter().map(|s| (s.y0, s.y1)).collect(),
                water: fluids.clone(),
            };
            if stack.is_sculpted() {
                let mut cave = Vec::new();
                stack.sculpted_fluids(cell, &mut cave);
                column.water.extend(cave.iter().map(|c| (c.0, c.1)));
            }
            out[(j * w + i) as usize] = Some(column);
        }
    }
    out
}

/// What is wrong with a water block, for [`check_tile_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Step,
    Fall,
    Spill,
    Floating,
}

/// Check the water of the tile at `pos`.
pub fn check_tile(dim: &Dimension, pos: TilePos) -> WaterCheck {
    check_tile_with(dim, pos, &mut |_, _, _, _| {})
}

/// [`check_tile`], telling `found` where each problem is (x, y, z).
pub fn check_tile_with(dim: &Dimension, pos: TilePos, found: &mut dyn FnMut(i32, i32, i32, Problem)) -> WaterCheck {
    let (ox, oz) = pos.origin();
    let n = TILE_SIZE as usize + 2;
    let cols = columns_around(dim, pos, 1);
    let mut r = WaterCheck::default();
    for j in 1..n - 1 {
        for i in 1..n - 1 {
            let Some(c) = &cols[j * n + i] else { continue };
            if c.water.is_empty() {
                continue;
            }
            r.columns += 1;
            for &(y0, y1) in &c.water {
                if y0 > c.solid.first().map_or(i32::MIN, |s| s.0) && c.at(y0 - 1) == Cell::Air {
                    r.floating += 1;
                    found(ox + i as i32 - 1, y0 - 1, oz + j as i32 - 1, Problem::Floating);
                }
                let y = y1 - 1;
                // Water over water of another run is the same water.
                if c.at(y1) == Cell::Water {
                    continue;
                }
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let Some(b) = &cols[(j as i32 + dj) as usize * n + (i as i32 + di) as usize] else {
                        r.edge += 1;
                        continue;
                    };
                    if b.at(y) != Cell::Air {
                        continue;
                    }
                    match b.floor_under(y) {
                        Some((top, Cell::Water)) if top >= y - 1 => {
                            r.steps += 1;
                            found(ox + i as i32 - 1 + di, y, oz + j as i32 - 1 + dj, Problem::Step);
                        }
                        Some((_, Cell::Water)) => {
                            r.falls += 1;
                            found(ox + i as i32 - 1 + di, y, oz + j as i32 - 1 + dj, Problem::Fall);
                        }
                        _ => {
                            r.spills += 1;
                            found(ox + i as i32 - 1 + di, y, oz + j as i32 - 1 + dj, Problem::Spill);
                        }
                    }
                }
            }
        }
    }
    r
}

/// Check the water of the whole of `dim`, tile by tile on every core.
pub fn check(dim: &Dimension) -> WaterCheck {
    let tiles: Vec<TilePos> = dim.ground().positions().collect();
    tiles.par_iter().map(|&p| check_tile(dim, p)).reduce(WaterCheck::default, WaterCheck::add)
}

/// Water runs into the land lowered beside it: over `rect` of the ground
/// level, each dry column whose ground lies under the surface of the water
/// beside it takes that water, the highest first, and so on across `rect`
/// and across the land this step has lowered (a stroke that reaches the
/// water from afar), never further. Lowering the shore of a lake, or the
/// bank of a river, widens its water instead of leaving a dry hole beside
/// it, which the game would fill with running water. Returns the columns
/// that took water.
pub fn fill_lowered(edit: &mut crate::history::Edit, rect: crate::coords::BlockRect) -> usize {
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    use std::collections::HashMap;
    if rect.is_empty() || edit.level_index() != 0 {
        return 0;
    }
    // Nothing to do without water in `rect` or beside it.
    let around = crate::coords::BlockRect::new(rect.x0 - 1, rect.z0 - 1, rect.x1 + 1, rect.z1 + 1);
    if !around.tiles().any(|p| edit.view(p).is_some_and(|t| t.fluid.uniform_value() != Some(NO_FLUID))) {
        return 0;
    }
    // Each column's ground (its top block; MAX: no column), its water
    // (NO_FLUID: dry), and the ground it had when the step began: read a
    // tile at a time, as the fill reaches it.
    struct Read {
        top: Vec<i32>,
        water: Vec<i16>,
        was: Option<Vec<i32>>,
    }
    fn read(tiles: &mut HashMap<TilePos, Option<Read>>, edit: &crate::history::Edit, p: TilePos) -> bool {
        tiles
            .entry(p)
            .or_insert_with(|| {
                let t = edit.view(p)?;
                let top: Vec<i32> = (0..crate::coords::TILE_AREA).map(|i| if t.present(i) { block_top(t.top.get(i)) } else { i32::MAX }).collect();
                let water = (0..crate::coords::TILE_AREA)
                    .map(|i| {
                        let f = t.fluid.get(i);
                        if f != NO_FLUID && f as i32 > top[i] { f } else { NO_FLUID }
                    })
                    .collect();
                let was = edit.original(p).map(|o| (0..crate::coords::TILE_AREA).map(|i| block_top(o.top.get(i))).collect());
                Some(Read { top, water, was })
            })
            .is_some()
    }
    let mut tiles: HashMap<TilePos, Option<Read>> = HashMap::new();
    for p in around.tiles() {
        read(&mut tiles, edit, p);
    }
    let get = |tiles: &HashMap<TilePos, Option<Read>>, x: i32, z: i32| -> Option<(i32, i16, Option<i32>)> {
        let r = tiles.get(&TilePos::containing(x, z))?.as_ref()?;
        let i = local_index(x, z);
        Some((r.top[i], r.water[i], r.was.as_ref().map(|w| w[i])))
    };
    let inside = |x: i32, z: i32| x >= rect.x0 && x < rect.x1 && z >= rect.z0 && z < rect.z1;
    let mut heap = std::collections::BinaryHeap::new();
    for z in rect.z0..rect.z1 {
        for x in rect.x0..rect.x1 {
            let Some((top, water, _)) = get(&tiles, x, z) else { continue };
            if water != NO_FLUID || top == i32::MAX {
                continue;
            }
            let beside = [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter().filter_map(|(dx, dz)| get(&tiles, x + dx, z + dz)).map(|c| c.1).filter(|&l| l != NO_FLUID).max();
            if let Some(l) = beside
                && l as i32 > top
            {
                heap.push((l, x, z));
            }
        }
    }
    let mut taken: HashMap<TilePos, Vec<(usize, i16)>> = HashMap::new();
    let mut n = 0;
    while let Some((l, x, z)) = heap.pop() {
        let p = TilePos::containing(x, z);
        let i = local_index(x, z);
        let Some(Some(r)) = tiles.get_mut(&p) else { continue };
        if r.water[i] != NO_FLUID {
            continue;
        }
        r.water[i] = l;
        taken.entry(p).or_default().push((i, l));
        n += 1;
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, nz) = (x + dx, z + dz);
            if !read(&mut tiles, edit, TilePos::containing(nx, nz)) {
                continue;
            }
            let Some((top, water, was)) = get(&tiles, nx, nz) else { continue };
            // Into `rect`, and past it into the land this step lowered.
            let open = inside(nx, nz) || was.is_some_and(|w| w > top);
            if open && water == NO_FLUID && top != i32::MAX && l as i32 > top {
                heap.push((l, nx, nz));
            }
        }
    }
    for (pos, cells) in taken {
        if let Some(tile) = edit.tile_mut(pos) {
            for (i, l) in cells {
                tile.fluid.set(i, l);
            }
            tile.fluid.compact();
        }
    }
    n
}

/// Make the water of the edited level over `rect` stay where it is: no
/// water block with open land beside it lower than its surface. Where a
/// column's water stands over a dry neighbour, that neighbour's ground
/// rises to hold it, up to `rise(x, z)` (the most it may reach: the ground
/// it had before it was dug, say); where it cannot, the water goes down to
/// the neighbour's ground, and so on until it holds. Only the water of
/// the columns `mine` names is lowered: the water already on the map is
/// left as it was; and never under `floor` (the level of the sea or the
/// lake a river runs into: lower, its mouth would stand under the water it
/// meets). Water beside lower water (a step along a river) is left.
/// Returns the columns changed.
pub fn seal(edit: &mut crate::history::Edit, rect: crate::coords::BlockRect, rise: &dyn Fn(i32, i32) -> f32, mine: &dyn Fn(i32, i32) -> bool, floor: i16) -> usize {
    use crate::codec::snap_height;
    use crate::compose::block_top;
    use crate::tile::NO_FLUID;
    let (x0, z0) = (rect.x0 - 1, rect.z0 - 1);
    let (w, d) = ((rect.x1 - rect.x0 + 2) as usize, (rect.z1 - rect.z0 + 2) as usize);
    let mut top = vec![f32::NAN; w * d];
    let mut fluid = vec![NO_FLUID; w * d];
    {
        let level = edit.level();
        let mut tiles: std::collections::HashMap<TilePos, Option<crate::level::TileRef>> = std::collections::HashMap::new();
        for j in 0..d {
            for i in 0..w {
                let (x, z) = (x0 + i as i32, z0 + j as i32);
                let p = TilePos::containing(x, z);
                let Some(t) = tiles.entry(p).or_insert_with(|| level.view(p)) else { continue };
                let c = local_index(x, z);
                if t.present(c) {
                    top[j * w + i] = t.top.get(c);
                    fluid[j * w + i] = t.fluid.get(c);
                }
            }
        }
    }
    let (before_top, before_fluid) = (top.clone(), fluid.clone());
    let wet = |top: &[f32], fluid: &[i16], c: usize| fluid[c] != NO_FLUID && fluid[c] as i32 > block_top(top[c]);
    let mut queue: std::collections::VecDeque<usize> = (0..w * d).filter(|&c| wet(&top, &fluid, c)).collect();
    while let Some(c) = queue.pop_front() {
        if !wet(&top, &fluid, c) {
            continue;
        }
        let (i, j) = (c % w, c / w);
        // The inner columns only: those around are read, not changed.
        if i == 0 || j == 0 || i == w - 1 || j == d - 1 {
            continue;
        }
        let ours = mine(x0 + i as i32, z0 + j as i32);
        let f = fluid[c] as i32;
        for n in [c - 1, c + 1, c - w, c + w] {
            if top[n].is_nan() || wet(&top, &fluid, n) || block_top(top[n]) >= f {
                continue;
            }
            let (ni, nj) = (n % w, n / w);
            let inner = ni > 0 && nj > 0 && ni < w - 1 && nj < d - 1;
            let limit = if inner { rise(x0 + ni as i32, z0 + nj as i32) } else { f32::MIN };
            let want = snap_height(f as f32);
            if limit >= want - 0.5 {
                top[n] = top[n].max(want.min(limit));
            }
            if ours && block_top(top[n]) < f && f > floor as i32 {
                // It cannot hold: the water goes down to its rim, never
                // under the floor.
                let rim = block_top(top[n]).max(floor as i32);
                fluid[c] = if rim > block_top(top[c]) { rim as i16 } else { NO_FLUID };
                queue.extend([c - 1, c + 1, c - w, c + w, c]);
                break;
            }
        }
    }
    let changed: Vec<usize> = (0..w * d).filter(|&c| top[c].to_bits() != before_top[c].to_bits() || fluid[c] != before_fluid[c]).collect();
    if changed.is_empty() {
        return 0;
    }
    let (top, fluid) = (&top, &fluid);
    edit.par_tiles(rect.tiles(), rect, false, |_, clip, tile| {
        let mut any = false;
        for z in clip.z0..clip.z1 {
            for x in clip.x0..clip.x1 {
                let c = (z - z0) as usize * w + (x - x0) as usize;
                let i = local_index(x, z);
                if top[c].to_bits() != before_top[c].to_bits() {
                    tile.top.set(i, top[c]);
                    any = true;
                }
                if fluid[c] != before_fluid[c] {
                    tile.fluid.set(i, fluid[c]);
                    any = true;
                }
            }
        }
        if any {
            tile.compact();
        }
    });
    changed.len()
}

/// The lowest ground water of `cols` (`w` wide, `d` deep) may run down to: under
/// the air beside it, the ground or water lower down. `None` where no
/// water has air beside it.
fn restless(cols: &[Option<Column>], w: usize, d: usize) -> Option<i32> {
    let n = w;
    let covered = |b: &Column, y0: i32, y1: i32| {
        let mut runs: Vec<(i32, i32)> = b.solid.iter().chain(&b.water).copied().filter(|r| r.1 > y0 && r.0 < y1).collect();
        runs.sort_unstable();
        let mut y = y0;
        for r in runs {
            if r.0 > y {
                return false;
            }
            y = y.max(r.1);
        }
        y >= y1
    };
    let mut lowest: Option<i32> = None;
    for j in 0..d {
        for i in 0..n {
            let Some(c) = &cols[j * n + i] else { continue };
            for &(y0, y1) in &c.water {
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i as i32 + di, j as i32 + dj);
                    if a < 0 || b < 0 || a >= n as i32 || b >= d as i32 {
                        continue;
                    }
                    if let Some(b) = &cols[b as usize * n + a as usize]
                        && !covered(b, y0, y1)
                    {
                        let floor = b.floor_under(y1).map_or(i32::MIN, |f| f.0);
                        lowest = Some(lowest.map_or(floor, |l| l.min(floor)));
                    }
                }
            }
        }
    }
    lowest
}

/// The blocks around the tile at `pos` (`margin` blocks past it) as water
/// sees them, between a little under its lowest surface and over its
/// highest water; `None` if it holds no water.
pub fn grid_around(dim: &Dimension, pos: TilePos, margin: i32, planner: Option<&Planner>) -> Option<(crate::flow::Grid, Vec<Option<Column>>)> {
    grid_and_edges(dim, pos, margin, planner).map(|(g, c, _)| (g, c))
}

/// A box of blocks, the columns it was made from, and its water with air
/// beside it.
type GridAndEdges = (crate::flow::Grid, Vec<Option<Column>>, Vec<(i32, i32, i32)>);

/// [`grid_around`], with the water blocks that have air beside them.
fn grid_and_edges(dim: &Dimension, pos: TilePos, margin: i32, planner: Option<&Planner>) -> Option<GridAndEdges> {
    let rect = pos.rect().expand(margin);
    let cols = columns_with(dim, pos, margin, planner);
    let (g, edges) = grid_of(dim, rect, &cols)?;
    Some((g, cols, edges))
}

/// Blocks, by (x, y, z).
type Blocks = Vec<(i32, i32, i32)>;

/// The blocks of `rect` (its columns `cols`, row by row) as water sees
/// them, between a little under its lowest surface and over its highest
/// water, and its water with air beside it; `None` if it holds no water
/// that can run.
fn grid_of(dim: &Dimension, rect: crate::coords::BlockRect, cols: &[Option<Column>]) -> Option<(crate::flow::Grid, Blocks)> {
    use crate::flow::{Block, Grid};
    let (n, nd) = (rect.width() as usize, rect.depth() as usize);
    let floor = restless(cols, n, nd)?;
    let top_water = cols.iter().flatten().flat_map(|c| c.water.iter().map(|w| w.1)).max()?;
    let y1 = (top_water + 2).min(dim.max_y);
    // Water runs down to the lowest land, or the lowest water's surface.
    let lowest = cols.iter().flatten().map(|c| c.water.last().map_or_else(|| c.solid.last().map_or(dim.min_y, |s| s.1), |w| w.1 - 1)).min()?.min(floor);
    let y0 = (lowest - 1).max(y1 - 192).max(dim.min_y);
    let ny = (y1 - y0) as usize;
    let mut g = Grid::new(rect.x0, y0, rect.z0, n, ny, nd);
    let layer = n * nd;
    for (k, c) in cols.iter().enumerate() {
        let Some(c) = c else {
            for j in 0..ny {
                g.blocks[j * layer + k] = Block::Solid;
            }
            continue;
        };
        for (runs, b) in [(&c.solid, Block::Solid), (&c.water, Block::Source)] {
            for &(a, z) in runs.iter() {
                for y in a.max(y0)..z.min(y1) {
                    g.blocks[(y - y0) as usize * layer + k] = b;
                }
            }
        }
    }
    // The water with air beside it.
    let mut edges = Vec::new();
    for (k, c) in cols.iter().enumerate() {
        let Some(c) = c else { continue };
        let (i, j) = ((k % n) as i32, (k / n) as i32);
        for &(a, z) in &c.water {
            for y in a.max(y0)..z.min(y1) {
                let base = (y - y0) as usize * layer;
                if [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)].iter().any(|&(di, dj)| {
                    let (p, q) = (i + di, j + dj);
                    p >= 0 && q >= 0 && p < n as i32 && q < nd as i32 && g.blocks[base + q as usize * n + p as usize] == Block::Air
                }) {
                    edges.push((g.x0 + i, y, g.z0 + j));
                }
            }
        }
    }
    Some((g, edges))
}

/// What the game does to the water of a tile once every block of it is
/// updated, as [`crate::flow`] works it out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settled {
    /// Blocks that became water, of them over dry columns, and blocks
    /// whose water changed.
    pub added: u64,
    pub over_land: u64,
    pub changed: u64,
    /// Water blocks that became air.
    pub removed: u64,
    /// Of the blocks added: sources, falling water, flowing water.
    pub sources: u64,
    pub falling: u64,
    /// Tiles the rules did not settle within their budget.
    pub unsettled: u64,
}

impl Settled {
    fn add(mut self, o: Self) -> Self {
        self.added += o.added;
        self.over_land += o.over_land;
        self.changed += o.changed;
        self.removed += o.removed;
        self.sources += o.sources;
        self.falling += o.falling;
        self.unsettled += o.unsettled;
        self
    }
}

/// Let the game's rules run on the water of the tile at `pos`; `steady`:
/// on the water as [`crate::flow::Grid::steady`] leaves it.
pub fn settle_tile(dim: &Dimension, pos: TilePos, steady: bool) -> Settled {
    const MARGIN: i32 = 8;
    let Some((mut g, cols, edges)) = grid_and_edges(dim, pos, MARGIN, None) else { return Settled::default() };
    let budget = 400 * g.nx * g.nz;
    let before = g.blocks.clone();
    if steady {
        if !g.steady_from(edges, budget).1 {
            if std::env::var_os("MASSIF_WATER_DEBUG").is_some() {
                eprintln!("{pos:?}: steady ran out");
            }
            return Settled { unsettled: 1, ..Settled::default() };
        }
        // Once steady, the game must leave it as it is.
        let still = g.blocks.clone();
        let mut queue = g.restless();
        g.run(&mut queue, budget);
        if g.blocks != still {
            if std::env::var_os("MASSIF_WATER_DEBUG").is_some() {
                let diff: Vec<String> = still.iter().zip(&g.blocks).enumerate().filter(|(_, (a, b))| a != b).take(4).map(|(i, (a, b))| format!("{},{},{} {a:?}->{b:?}", g.x0 + (i % g.nx) as i32, g.y0 + (i / (g.nx * g.nz)) as i32, g.z0 + ((i / g.nx) % g.nz) as i32)).collect();
                eprintln!("{pos:?}: not still: {}", diff.join("; "));
            }
            return Settled { unsettled: 1, ..Settled::default() };
        }
    }
    let n = (TILE_SIZE + 2 * MARGIN) as usize;
    let mut settled = true;
    if !steady {
        let mut queue: std::collections::VecDeque<(i32, i32, i32)> = std::collections::VecDeque::new();
        for (k, c) in cols.iter().enumerate() {
            let Some(c) = c else { continue };
            for w in &c.water {
                queue.push_back((g.x0 + (k % n) as i32, w.1 - 1, g.z0 + (k / n) as i32));
            }
        }
        settled = g.run(&mut queue, budget).1;
    }
    let mut r = Settled {
        unsettled: (!settled) as u64,
        ..Settled::default()
    };
    for (i, (a, b)) in before.iter().zip(&g.blocks).enumerate() {
        if a == b {
            continue;
        }
        let (x, z) = ((i % g.nx) as i32, ((i / g.nx) % g.nz) as i32);
        if x < MARGIN || z < MARGIN || x >= MARGIN + TILE_SIZE || z >= MARGIN + TILE_SIZE {
            continue;
        }
        if !a.is_water() {
            r.added += 1;
            match b {
                crate::flow::Block::Source => r.sources += 1,
                crate::flow::Block::Flowing { falling: true, .. } => r.falling += 1,
                _ => {}
            }
            if cols[z as usize * n + x as usize].as_ref().is_some_and(|c| c.water.is_empty()) {
                r.over_land += 1;
            }
        } else if b.is_water() {
            r.changed += 1;
        } else {
            r.removed += 1;
        }
    }
    r
}

/// [`settle_tile`] over the whole of `dim`.
pub fn settle_all(dim: &Dimension, steady: bool) -> Settled {
    let tiles: Vec<TilePos> = dim.ground().positions().collect();
    tiles.par_iter().map(|&p| settle_tile(dim, p, steady)).reduce(Settled::default, Settled::add)
}

/// A block of a tile whose water the export writes otherwise than the map
/// holds it: column (`local_index`), height, and what goes there (air, or
/// flowing water of a `level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Steady {
    pub cell: u16,
    pub y: i32,
    pub block: crate::flow::Block,
}

/// The water of the tile at `pos` as the export writes it, so that the
/// game leaves it as it is ([`crate::flow::Grid::steady`]): the blocks that
/// differ from the map's still water, by column then height. Empty where
/// all its water stands still already.
pub fn steady_tile(dim: &Dimension, pos: TilePos, planner: Option<&Planner>) -> Vec<Steady> {
    const MARGIN: i32 = RING;
    let Some((mut g, _, edges)) = grid_and_edges(dim, pos, MARGIN, planner) else { return Vec::new() };
    let before = g.blocks.clone();
    let budget = 400 * g.nx * g.nz;
    g.steady_from(edges, budget);
    let (ox, oz) = pos.origin();
    let mut out = Vec::new();
    for (i, (a, b)) in before.iter().zip(&g.blocks).enumerate() {
        if a == b {
            continue;
        }
        let (x, z) = (g.x0 + (i % g.nx) as i32, g.z0 + ((i / g.nx) % g.nz) as i32);
        if x < ox || z < oz || x >= ox + TILE_SIZE || z >= oz + TILE_SIZE {
            continue;
        }
        out.push(Steady {
            cell: local_index(x, z) as u16,
            y: g.y0 + (i / (g.nx * g.nz)) as i32,
            block: *b,
        });
    }
    out.sort_unstable_by_key(|s| (s.cell, s.y));
    out
}

/// How far around the two tiles of a seam the water is read, and how far
/// from the seam the water ticks, for [`seam_run`].
const SEAM_AROUND: i32 = 24;
const SEAM_BAND: i32 = 16;

/// What the game does to the water along the seam between the tiles `a`
/// and `b` (`a` west or north of `b`), each tile's water as `steady`
/// writes it (those it lacks as the map holds it), once the water within
/// [`SEAM_BAND`] blocks of the seam with air beside it ticks, as it does
/// when a block of that air changes: whether it settled, and the blocks
/// that changed, each with what the map holds there and what it became.
#[allow(clippy::type_complexity)]
fn seam_run(dim: &Dimension, a: TilePos, b: TilePos, steady: &std::collections::HashMap<TilePos, Vec<Steady>>) -> (bool, Vec<((i32, i32, i32), crate::flow::Block, crate::flow::Block)>) {
    use crate::coords::BlockRect;
    use crate::flow::Block;
    let rect = BlockRect::new(a.x * TILE_SIZE - SEAM_AROUND, a.z * TILE_SIZE - SEAM_AROUND, (b.x + 1) * TILE_SIZE + SEAM_AROUND, (b.z + 1) * TILE_SIZE + SEAM_AROUND);
    let cols = columns_in(dim, rect, None, None);
    let Some((mut g, _)) = grid_of(dim, rect, &cols) else { return (true, Vec::new()) };
    let base = g.blocks.clone();
    // The water as the export writes it, each tile's its own.
    for t in rect.tiles() {
        let (ox, oz) = t.origin();
        for s in steady.get(&t).into_iter().flatten() {
            let (x, z) = (ox + s.cell as i32 % TILE_SIZE, oz + s.cell as i32 / TILE_SIZE);
            if x >= rect.x0 && x < rect.x1 && z >= rect.z0 && z < rect.z1 {
                g.set(x, s.y, z, s.block);
            }
        }
    }
    let written = g.blocks.clone();
    let by_seam = |x: i32, z: i32| {
        if a.x != b.x {
            (x - b.x * TILE_SIZE).abs() < SEAM_BAND && z >= a.z * TILE_SIZE && z < (a.z + 1) * TILE_SIZE
        } else {
            (z - b.z * TILE_SIZE).abs() < SEAM_BAND && x >= a.x * TILE_SIZE && x < (a.x + 1) * TILE_SIZE
        }
    };
    // The water with air beside, above or under it ticks, as it does when a
    // block of that air changes.
    let airy = |x: i32, y: i32, z: i32| [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)].iter().any(|(dx, dy, dz)| g.get(x + dx, y + dy, z + dz) == Block::Air);
    let mut queue: std::collections::VecDeque<(i32, i32, i32)> = g.restless().into_iter().filter(|&(x, y, z)| by_seam(x, z) && airy(x, y, z)).collect();
    let budget = 400 * g.nx * g.nz;
    let settled = g.run(&mut queue, budget).1;
    let mut changed = Vec::new();
    for (i, (was, now)) in written.iter().zip(&g.blocks).enumerate() {
        if was != now {
            let p = (g.x0 + (i % g.nx) as i32, g.y0 + (i / (g.nx * g.nz)) as i32, g.z0 + ((i / g.nx) % g.nz) as i32);
            changed.push((p, base[i], *now));
        }
    }
    (settled, changed)
}

/// The seams between the tiles of `tiles` and the tiles beside them, each
/// once, west or north tile first.
fn seams_of(dim: &Dimension, tiles: impl Iterator<Item = TilePos>) -> Vec<(TilePos, TilePos)> {
    let mut seams = std::collections::BTreeSet::new();
    for p in tiles {
        for (dx, dz) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
            let q = p.offset(dx, dz);
            if dim.ground().view(q).is_some() {
                seams.insert(if dx + dz > 0 { ((p.x, p.z), (q.x, q.z)) } else { ((q.x, q.z), (p.x, p.z)) });
            }
        }
    }
    seams.into_iter().map(|(a, b)| (TilePos::new(a.0, a.1), TilePos::new(b.0, b.1))).collect()
}

/// The water of every tile of `dim` the export settles, as it writes it:
/// each tile's worked out on its own ([`steady_tile`]), then made to agree
/// along the seams between tiles. Worked out alone, two tiles side by side
/// may each leave their side of a seam as it holds against what they
/// guessed of the other, and not against what the other writes: there the
/// game would move the water (flowing water evened out, a source made)
/// once a block beside it changed. Along each seam the game's rules run
/// over both tiles as written ([`seam_run`]), and what they change is
/// written instead, round after round until nothing changes. By tile, the
/// blocks written otherwise than the map holds them, by column then
/// height; a tile not there is written as the map holds it.
pub fn steady_all(dim: &Dimension, planner: &Planner) -> std::collections::HashMap<TilePos, Vec<Steady>> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let tiles: Vec<TilePos> = dim.ground().positions().filter(|&p| planner.needs(p)).collect();
    let mut steady: HashMap<TilePos, Vec<Steady>> = tiles.par_iter().map(|&p| (p, steady_tile(dim, p, Some(planner)))).collect();
    let mut seams = seams_of(dim, tiles.iter().copied());
    for _ in 0..4 {
        if seams.is_empty() {
            break;
        }
        let runs: Vec<_> = seams.par_iter().map(|&(a, b)| seam_run(dim, a, b, &steady).1).collect();
        // What the game changed, by tile: what is written there instead.
        // By column and height: what the map holds there, what it becomes.
        type Edits = BTreeMap<(u16, i32), (crate::flow::Block, crate::flow::Block)>;
        let mut edits: HashMap<TilePos, Edits> = HashMap::new();
        for ((x, y, z), base, now) in runs.into_iter().flatten() {
            edits.entry(TilePos::containing(x, z)).or_default().insert((local_index(x, z) as u16, y), (base, now));
        }
        if edits.is_empty() {
            break;
        }
        let touched: HashSet<TilePos> = edits.keys().copied().collect();
        for (t, cells) in edits {
            let list = steady.entry(t).or_default();
            let mut all: BTreeMap<(u16, i32), crate::flow::Block> = list.iter().map(|s| ((s.cell, s.y), s.block)).collect();
            for (k, (base, now)) in cells {
                if now == base {
                    all.remove(&k);
                } else {
                    all.insert(k, now);
                }
            }
            *list = all.into_iter().map(|((cell, y), block)| Steady { cell, y, block }).collect();
        }
        seams = seams_of(dim, touched.into_iter());
    }
    steady
}

/// The water the export writes along the seams between its tiles
/// ([`steady_all`]; `joined` false: each tile's as worked out on its own,
/// [`steady_tile`]), as the game finds it ([`seam_run`]): what the game
/// would change there once a block beside it is updated (nothing, if the
/// tiles agree), and where (x, y, z), the first few of each seam.
pub fn check_seams(dim: &Dimension, joined: bool) -> (Settled, Vec<(i32, i32, i32)>) {
    let planner = Planner::new(dim);
    let steady = if joined {
        steady_all(dim, &planner)
    } else {
        dim.ground().positions().filter(|&p| planner.needs(p)).collect::<Vec<_>>().par_iter().map(|&p| (p, steady_tile(dim, p, Some(&planner)))).collect()
    };
    let seams = seams_of(dim, steady.keys().copied());
    let found: Vec<(Settled, Blocks)> = seams
        .par_iter()
        .map(|&(a, b)| {
            let (settled, changed) = seam_run(dim, a, b, &steady);
            let mut r = Settled { unsettled: (!settled) as u64, ..Settled::default() };
            let mut at = Vec::new();
            for &(p, _, now) in &changed {
                if at.len() < 8 {
                    at.push(p);
                }
                r.changed += 1;
                r.sources += (now == crate::flow::Block::Source) as u64;
            }
            (r, at)
        })
        .collect();
    let mut total = Settled::default();
    let mut at = Vec::new();
    for (r, p) in found {
        total = total.add(r);
        at.extend(p);
    }
    (total, at)
}

/// What a tile holds, for [`Planner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// No water at all.
    Dry,
    /// Under water all of it, at one level.
    Drowned(i16),
    Mixed,
}

/// Which tiles of a map have water the export must settle: those with
/// dry land and water, or water at two levels, within reach of them. The
/// rest (dry land, the open sea) the export writes as it is, without
/// working out their water.
pub struct Planner {
    kinds: std::collections::HashMap<TilePos, Kind>,
    /// The ground of the edges, [`RING`] blocks wide, of the tiles beside
    /// those to settle (of the ground alone): its top and its water by
    /// [`ring_index`]; `i16::MIN` for a column not on the map.
    rings: std::collections::HashMap<TilePos, Vec<(i16, i16)>>,
}

/// How far into the tiles beside it the water of a tile is worked out.
const RING: i32 = 24;

/// The place of a column of a tile's edge in its ring, past the end for
/// one inside.
fn ring_index(cell: usize) -> usize {
    let (x, z) = ((cell % TILE_SIZE as usize) as i32, (cell / TILE_SIZE as usize) as i32);
    let far = TILE_SIZE - RING;
    let band = (RING * TILE_SIZE) as usize;
    if z < RING {
        (z * TILE_SIZE + x) as usize
    } else if z >= far {
        band + ((z - far) * TILE_SIZE + x) as usize
    } else if x < RING {
        2 * band + ((z - RING) * RING + x) as usize
    } else if x >= far {
        2 * band + ((far - RING) * RING) as usize + ((z - RING) * RING + x - far) as usize
    } else {
        usize::MAX
    }
}

impl Planner {
    pub fn new(dim: &Dimension) -> Self {
        use crate::compose::block_top;
        use crate::tile::NO_FLUID;
        let tiles: Vec<TilePos> = dim.ground().positions().collect();
        let kinds = tiles
            .par_iter()
            .map(|&p| {
                let kind = match TileStack::new(dim, p, false) {
                    Some(stack) if !stack.is_sculpted() && stack.layers().count() == 1 => {
                        let t = stack.ground();
                        match t.fluid.uniform_value() {
                            Some(NO_FLUID) => Kind::Dry,
                            Some(f) if (0..crate::coords::TILE_AREA).all(|i| !t.present(i) || block_top(t.top.get(i)) < f as i32) => Kind::Drowned(f),
                            // Water nowhere over the ground (a level kept
                            // under it, as some generated maps keep the
                            // sea's): dry.
                            Some(f) if (0..crate::coords::TILE_AREA).all(|i| !t.present(i) || block_top(t.top.get(i)) >= f as i32) => Kind::Dry,
                            None => {
                                let (top, fluid) = (t.top.dense(), t.fluid.dense());
                                let wet = |i: usize| t.present(i) && fluid[i] != NO_FLUID && fluid[i] as i32 > block_top(top[i]);
                                if (0..crate::coords::TILE_AREA).any(wet) { Kind::Mixed } else { Kind::Dry }
                            }
                            _ => Kind::Mixed,
                        }
                    }
                    _ => Kind::Mixed,
                };
                (p, kind)
            })
            .collect();
        let mut planner = Self { kinds, rings: std::collections::HashMap::new() };
        // The edges of the tiles beside those to settle, read once.
        let near: std::collections::HashSet<TilePos> = tiles.iter().filter(|&&p| planner.needs(p)).flat_map(|p| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| p.offset(dx, dz)))).collect();
        let near: Vec<TilePos> = near.into_iter().filter(|p| planner.kinds.get(p).is_some_and(|&k| k != Kind::Mixed)).collect();
        let ground = dim.ground();
        planner.rings = near
            .par_iter()
            .filter_map(|&p| {
                let t = ground.view(p)?;
                let mut ring = vec![(i16::MIN, NO_FLUID); (4 * RING * (TILE_SIZE - RING)) as usize];
                for cell in 0..crate::coords::TILE_AREA {
                    let k = ring_index(cell);
                    if k < ring.len() && t.present(cell) {
                        ring[k] = (block_top(t.top.get(cell)).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16, t.fluid.get(cell));
                    }
                }
                Some((p, ring))
            })
            .collect();
        planner
    }

    /// Whether the water of the tile at `pos` may need settling.
    pub fn needs(&self, pos: TilePos) -> bool {
        let Some(&here) = self.kinds.get(&pos) else { return false };
        if here == Kind::Mixed {
            return true;
        }
        (-1..=1).any(|dz| (-1..=1).any(|dx| self.kinds.get(&pos.offset(dx, dz)).is_some_and(|&k| k != here)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::BlockRect;
    use crate::history::History;
    use crate::tile::{NO_FLUID, Tile};
    use crate::world::{DimensionKind, World, WorldMeta};

    /// Water at 80 over ground at 75 for x < 64, dry ground at 76 past it.
    fn shelf() -> World {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        let mut tile = Tile::ground(76.0, 1, NO_FLUID);
        for z in 0..128 {
            for x in 0..64 {
                tile.top.set(local_index(x, z), 75.0);
                tile.fluid.set(local_index(x, z), 80);
            }
        }
        dim.ground_mut().insert(TilePos::new(0, 0), tile);
        world.dims = vec![dim];
        world
    }

    /// A river dug with the River tool across the seams of a hilly map:
    /// each tile's water, worked out on its own, disagrees with the next
    /// one's along their seams (the game evens the flowing water out and
    /// makes sources there once a block beside it changes); made to agree,
    /// the game leaves all of it as it is.
    #[test]
    fn the_tiles_agree_along_their_seams() {
        let mut world = crate::generate::generate(&crate::generate::GenParams { tiles_x: 4, tiles_z: 4, landform: crate::generate::Landform::Hills, seed: 1, ..crate::generate::GenParams::default() });
        let sea = world.meta.sea_level as f32;
        let level = world.dims[0].ground();
        let mut d = crate::river::descend(level, (-208, -240), sea, 60_000, 2_000_000, 13.0);
        d.course = crate::river::natural_course(level, &d.course, 24.0, &[0]).0;
        let reg = world.registry.clone();
        let spec = crate::river::RiverSpec {
            width: 12.0,
            width_end: 36.0,
            depth: 4.0,
            depth_end: 7.0,
            banks: 26.4,
            water: true,
            bed: reg.terrain_by_name("Gravel"),
            shore: reg.terrain_by_name("Sand"),
            clay: reg.terrain_by_name("Clay"),
            sea: None,
            biome: reg.biome_by_id("minecraft:river"),
        };
        let mut h = History::default();
        h.begin("river", 0);
        crate::river::dig_descent(&mut h.edit(&mut world, 0), &d, &spec);
        h.commit();
        let (alone, at) = check_seams(&world.dims[0], false);
        assert!(alone.changed > 0, "the tiles alone disagree: {alone:?}");
        let (joined, at_joined) = check_seams(&world.dims[0], true);
        assert_eq!(joined.changed, 0, "{joined:?} at {at_joined:?} (alone: {} at {:?})", alone.changed, &at[..at.len().min(4)]);
    }

    #[test]
    fn water_over_lower_land_spills_and_settles() {
        let world = shelf();
        let c = check(&world.dims[0]);
        // A row of 128 columns, four blocks of water over the land.
        assert_eq!(c.spills, 128, "{c:?}");
        let s = settle_tile(&world.dims[0], TilePos::new(0, 0), false);
        assert!(s.over_land > 0, "{s:?}");
        let s = settle_tile(&world.dims[0], TilePos::new(0, 0), true);
        assert_eq!(s.unsettled, 0, "{s:?}");
    }

    /// A tile whose water level lies under its ground everywhere (as some
    /// generated maps keep the sea's) has no water to settle.
    #[test]
    fn water_buried_under_the_ground_is_not_settled() {
        let mut world = World::new(WorldMeta::default());
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tx in 0..2 {
            let mut tile = Tile::ground(80.0, 1, 63);
            tile.top.set(local_index(5, 5), 90.0);
            dim.ground_mut().insert(TilePos::new(tx, 0), tile);
        }
        world.dims = vec![dim];
        let planner = Planner::new(&world.dims[0]);
        assert!(!planner.needs(TilePos::new(0, 0)) && !planner.needs(TilePos::new(1, 0)));
        assert!(steady_tile(&world.dims[0], TilePos::new(0, 0), Some(&planner)).is_empty());
    }

    #[test]
    fn seal_lowers_only_its_own_water() {
        let rect = BlockRect::new(0, 0, 128, 128);
        // Water already there stays as it was.
        let mut world = shelf();
        let mut h = History::default();
        h.begin("seal", 0);
        assert_eq!(seal(&mut h.edit(&mut world, 0), rect, &|_, _| f32::MIN, &|_, _| false, i16::MIN), 0);
        h.commit();
        assert_eq!(check(&world.dims[0]).spills, 128);
        // Its own goes down to the rim of the land beside it.
        let mut h = History::default();
        h.begin("seal", 0);
        assert!(seal(&mut h.edit(&mut world, 0), rect, &|_, _| f32::MIN, &|_, _| true, i16::MIN) > 0);
        h.commit();
        // The edge goes down to the land's rim, and the rest falls into it.
        let c = check(&world.dims[0]);
        assert_eq!((c.spills, c.falls), (0, 128), "{c:?}");
        let t = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!(t.fluid.get(local_index(63, 10)), 76);
        assert_eq!(t.fluid.get(local_index(10, 10)), 80);
        // Or the land it dug rises back to hold it.
        let mut world = shelf();
        let mut h = History::default();
        h.begin("seal", 0);
        seal(&mut h.edit(&mut world, 0), rect, &|_, _| 90.0, &|_, _| true, i16::MIN);
        h.commit();
        let t = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!(t.fluid.get(local_index(10, 10)), 80);
        assert!(crate::compose::block_top(t.top.get(local_index(64, 10))) >= 80);
        assert_eq!(check(&world.dims[0]).spills, 0);
    }
}
