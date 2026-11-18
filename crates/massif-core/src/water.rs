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
    let n = TILE_SIZE + 2 * margin;
    let mut out = vec![None; (n * n) as usize];
    let (ox, oz) = pos.origin();
    let (mut spans, mut fluids): (Vec<Span>, Vec<(i32, i32)>) = (Vec::new(), Vec::new());
    let mut stacks: std::collections::HashMap<TilePos, Option<TileStack>> = std::collections::HashMap::new();
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (ox + i - margin, oz + j - margin);
            let p = TilePos::containing(x, z);
            let cell = local_index(x, z);
            if p != pos
                && let Some(ring) = planner.and_then(|pl| pl.rings.get(&p))
            {
                if let Some(&(top, fluid)) = ring.get(ring_index(cell))
                    && top != i16::MIN
                {
                    out[(j * n + i) as usize] = Some(ground_column(dim, top as i32, fluid));
                }
                continue;
            }
            let Some(stack) = stacks.entry(p).or_insert_with(|| TileStack::from_above(dim, p, false)) else { continue };
            if !stack.ground().present(cell) {
                continue;
            }
            if !stack.is_sculpted() && stack.caves().is_none() && stack.layers().nth(1).is_none() {
                let g = stack.ground();
                out[(j * n + i) as usize] = Some(ground_column(dim, crate::compose::block_top(g.top.get(cell)), g.fluid.get(cell)));
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
            out[(j * n + i) as usize] = Some(column);
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

/// The lowest ground water of `cols` (`n` a side) may run down to: under
/// the air beside it, the ground or water lower down. `None` where no
/// water has air beside it.
fn restless(cols: &[Option<Column>], n: usize) -> Option<i32> {
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
    for j in 0..n {
        for i in 0..n {
            let Some(c) = &cols[j * n + i] else { continue };
            for &(y0, y1) in &c.water {
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i as i32 + di, j as i32 + dj);
                    if a < 0 || b < 0 || a >= n as i32 || b >= n as i32 {
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
    use crate::flow::{Block, Grid};
    let cols = columns_with(dim, pos, margin, planner);
    let floor = restless(&cols, (TILE_SIZE + 2 * margin) as usize)?;
    let top_water = cols.iter().flatten().flat_map(|c| c.water.iter().map(|w| w.1)).max()?;
    let y1 = (top_water + 2).min(dim.max_y);
    // Water runs down to the lowest land, or the lowest water's surface.
    let lowest = cols.iter().flatten().map(|c| c.water.last().map_or_else(|| c.solid.last().map_or(dim.min_y, |s| s.1), |w| w.1 - 1)).min()?.min(floor);
    let y0 = (lowest - 1).max(y1 - 192).max(dim.min_y);
    let n = (TILE_SIZE + 2 * margin) as usize;
    let (ox, oz) = pos.origin();
    let ny = (y1 - y0) as usize;
    let mut g = Grid::new(ox - margin, y0, oz - margin, n, ny, n);
    let layer = n * n;
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
                    p >= 0 && q >= 0 && p < n as i32 && q < n as i32 && g.blocks[base + q as usize * n + p as usize] == Block::Air
                }) {
                    edges.push((g.x0 + i, y, g.z0 + j));
                }
            }
        }
    }
    Some((g, cols, edges))
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
const RING: i32 = 8;

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
                let kind = match TileStack::from_above(dim, p, false) {
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
