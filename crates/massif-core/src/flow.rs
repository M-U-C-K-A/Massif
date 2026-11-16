//! Minecraft's rules for water, to know what the game does to the water of
//! a map once a block beside it changes, and to write water the game leaves
//! as it is.
//!
//! The rules are those of the game since 1.13 (`FlowingFluid`, water): a
//! source block spreads to the sides one less, flowing water keeps one less
//! than its fullest neighbour, falls into the air under it, and water with
//! two sources beside it over ground or a source becomes a source itself.
//! Flowing water over water does not spread to the sides; a source spreads
//! towards the nearest drop within four blocks, or all ways if there is
//! none.

use std::collections::VecDeque;

/// A block, as water sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Block {
    Air,
    Solid,
    Source,
    /// Flowing water: how much (1..=7, or 8 falling) and whether it falls.
    Flowing { amount: u8, falling: bool },
}

impl Block {
    pub fn is_water(self) -> bool {
        matches!(self, Block::Source | Block::Flowing { .. })
    }

    /// How much water: 8 for a source and for falling water.
    pub fn amount(self) -> u8 {
        match self {
            Block::Source => 8,
            Block::Flowing { amount, .. } => amount,
            _ => 0,
        }
    }

    /// The `level` of the block state: 0 a source, 1..=7 flowing, 8 falling.
    pub fn level(self) -> u8 {
        match self {
            Block::Flowing { falling: true, .. } => 8,
            Block::Flowing { amount, .. } => 8 - amount,
            _ => 0,
        }
    }
}

const SIDES: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

/// A box of blocks, `nx × ny × nz`, its corner at `(x0, y0, z0)`. What lies
/// outside it is solid.
#[derive(Debug, Clone)]
pub struct Grid {
    pub x0: i32,
    pub y0: i32,
    pub z0: i32,
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub blocks: Vec<Block>,
}

impl Grid {
    pub fn new(x0: i32, y0: i32, z0: i32, nx: usize, ny: usize, nz: usize) -> Self {
        Self {
            x0,
            y0,
            z0,
            nx,
            ny,
            nz,
            blocks: vec![Block::Air; nx * ny * nz],
        }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let (i, j, k) = (x - self.x0, y - self.y0, z - self.z0);
        if i < 0 || j < 0 || k < 0 || i >= self.nx as i32 || j >= self.ny as i32 || k >= self.nz as i32 {
            return None;
        }
        Some((j as usize * self.nz + k as usize) * self.nx + i as usize)
    }

    pub fn get(&self, x: i32, y: i32, z: i32) -> Block {
        self.index(x, y, z).map_or(Block::Solid, |i| self.blocks[i])
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, b: Block) {
        if let Some(i) = self.index(x, y, z) {
            self.blocks[i] = b;
        }
    }

    /// Water's way down from a block above it: into air or water.
    fn is_hole(&self, x: i32, y: i32, z: i32) -> bool {
        matches!(self.get(x, y - 1, z), Block::Air | Block::Source | Block::Flowing { .. })
    }

    /// The water the block at `(x, y, z)` would hold, from what is around it
    /// (`getNewLiquid`).
    pub fn new_liquid(&self, x: i32, y: i32, z: i32) -> Block {
        let (mut most, mut sources) = (0u8, 0);
        for (dx, dz) in SIDES {
            let b = self.get(x + dx, y, z + dz);
            if b == Block::Source {
                sources += 1;
            }
            most = most.max(b.amount());
        }
        if sources >= 2 && matches!(self.get(x, y - 1, z), Block::Solid | Block::Source) {
            return Block::Source;
        }
        if self.get(x, y + 1, z).is_water() {
            return Block::Flowing { amount: 8, falling: true };
        }
        if most <= 1 { Block::Air } else { Block::Flowing { amount: most - 1, falling: false } }
    }

    fn sources_beside(&self, x: i32, y: i32, z: i32) -> usize {
        SIDES.iter().filter(|(dx, dz)| self.get(x + dx, y, z + dz) == Block::Source).count()
    }

    /// Blocks to the nearest drop from `(x, y, z)`, within four.
    fn slope_distance(&self, x: i32, y: i32, z: i32, depth: u8, from: usize) -> u8 {
        let mut best = 100;
        for (d, (dx, dz)) in SIDES.iter().enumerate() {
            if d == from {
                continue;
            }
            let (nx, nz) = (x + dx, z + dz);
            if !matches!(self.get(nx, y, nz), Block::Air | Block::Flowing { .. }) {
                continue;
            }
            if self.is_hole(nx, y, nz) {
                return depth;
            }
            if depth < 4 {
                best = best.min(self.slope_distance(nx, y, nz, depth + 1, d ^ 1));
            }
        }
        best
    }

    /// Run the game's water rules from the blocks queued in `queue` until
    /// nothing changes or `budget` ticks are spent. Returns the ticks spent
    /// and whether it settled.
    pub fn run(&mut self, queue: &mut VecDeque<(i32, i32, i32)>, budget: usize) -> (usize, bool) {
        let mut queued = vec![false; self.blocks.len()];
        for &(x, y, z) in queue.iter() {
            if let Some(i) = self.index(x, y, z) {
                queued[i] = true;
            }
        }
        let mut ticks = 0;
        let mut changed: Vec<(i32, i32, i32)> = Vec::new();
        while let Some((x, y, z)) = queue.pop_front() {
            let Some(i) = self.index(x, y, z) else { continue };
            queued[i] = false;
            if ticks >= budget {
                return (ticks, false);
            }
            ticks += 1;
            changed.clear();
            self.tick(x, y, z, &mut changed);
            for &(cx, cy, cz) in &changed {
                for (ax, ay, az) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                    let p = (cx + ax, cy + ay, cz + az);
                    if let Some(j) = self.index(p.0, p.1, p.2)
                        && !queued[j]
                        && self.blocks[j].is_water()
                    {
                        queued[j] = true;
                        queue.push_back(p);
                    }
                }
            }
        }
        (ticks, true)
    }

    /// One tick of the water at `(x, y, z)`; the blocks it changed go into
    /// `changed`.
    fn tick(&mut self, x: i32, y: i32, z: i32, changed: &mut Vec<(i32, i32, i32)>) {
        let mut state = self.get(x, y, z);
        if !state.is_water() {
            return;
        }
        if state != Block::Source {
            let new = self.new_liquid(x, y, z);
            if new != state {
                self.set(x, y, z, new);
                changed.push((x, y, z));
                state = new;
            }
            if !state.is_water() {
                return;
            }
        }
        // Down first.
        let below = self.get(x, y - 1, z);
        if below == Block::Air && self.index(x, y - 1, z).is_some() {
            let new = self.new_liquid(x, y - 1, z);
            if new.is_water() {
                self.set(x, y - 1, z, new);
                changed.push((x, y - 1, z));
            }
            if self.sources_beside(x, y, z) >= 3 {
                self.spread_sides(x, y, z, state, changed);
            }
        } else if state == Block::Source || !self.is_hole(x, y, z) {
            self.spread_sides(x, y, z, state, changed);
        }
    }

    fn spread_sides(&mut self, x: i32, y: i32, z: i32, state: Block, changed: &mut Vec<(i32, i32, i32)>) {
        let amount = match state {
            Block::Flowing { falling: true, .. } => 7,
            b => b.amount().saturating_sub(1),
        };
        if amount == 0 {
            return;
        }
        let mut best = u8::MAX;
        let mut ways: Vec<(i32, i32)> = Vec::with_capacity(4);
        for (d, (dx, dz)) in SIDES.iter().enumerate() {
            let (nx, nz) = (x + dx, z + dz);
            let b = self.get(nx, y, nz);
            if !matches!(b, Block::Air | Block::Flowing { .. }) {
                continue;
            }
            let dist = if self.is_hole(nx, y, nz) { 0 } else { self.slope_distance(nx, y, nz, 1, d ^ 1) };
            if dist < best {
                ways.clear();
                best = dist;
            }
            if dist <= best && b == Block::Air && self.index(nx, y, nz).is_some() {
                ways.push((nx, nz));
            }
        }
        for (nx, nz) in ways {
            let new = self.new_liquid(nx, y, nz);
            if new.is_water() {
                self.set(nx, y, nz, new);
                changed.push((nx, y, nz));
            }
        }
    }
}

impl Grid {
    /// The water of the box as the game leaves it: every source on the
    /// surface of its water with air beside it at its height becomes
    /// flowing water, which, over water, does not spread; and so do the
    /// sources beside such a block, but one, wherever two sources would
    /// make a source of it again. Then the rules run until nothing changes.
    /// Returns how many sources were made flowing, and whether it settled.
    pub fn steady(&mut self, budget: usize) -> (usize, bool) {
        let mut edge: Vec<(i32, i32, i32)> = Vec::new();
        for j in 0..self.ny as i32 {
            for k in 0..self.nz as i32 {
                for i in 0..self.nx as i32 {
                    let (x, y, z) = (self.x0 + i, self.y0 + j, self.z0 + k);
                    if self.get(x, y, z) == Block::Source && SIDES.iter().any(|(dx, dz)| self.get(x + dx, y, z + dz) == Block::Air) {
                        edge.push((x, y, z));
                    }
                }
            }
        }
        self.steady_from(edge, budget)
    }

    /// [`Grid::steady`], given the sources with air beside them: the
    /// game's rules run as they are, and only where they would make a new
    /// source (a pool rising to the one above it) are the sources beside it
    /// made flowing first, round after round. Where that does not settle,
    /// every source with air beside it is ([`Grid::steady_wide`]).
    pub fn steady_from(&mut self, edge: Vec<(i32, i32, i32)>, budget: usize) -> (usize, bool) {
        let original = self.blocks.clone();
        let mut held: std::collections::HashSet<(i32, i32, i32)> = std::collections::HashSet::new();
        for _ in 0..24 {
            self.blocks.clone_from(&original);
            for &(x, y, z) in &held {
                let b = if self.get(x, y + 1, z).is_water() { Block::Flowing { amount: 8, falling: true } } else { Block::Flowing { amount: 7, falling: false } };
                self.set(x, y, z, b);
            }
            let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
            let mut seen = std::collections::HashSet::new();
            for &(x, y, z) in edge.iter().chain(&held) {
                for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                    let p = (x + dx, y + dy, z + dz);
                    if self.get(p.0, p.1, p.2).is_water() && seen.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
            if !self.run(&mut queue, budget).1 {
                break;
            }
            // Sources the rules made: beside each, the map's sources are
            // held back.
            let mut more = false;
            for (i, b) in self.blocks.iter().enumerate() {
                if *b != Block::Source {
                    continue;
                }
                let p = (self.x0 + (i % self.nx) as i32, self.y0 + (i / (self.nx * self.nz)) as i32, self.z0 + ((i / self.nx) % self.nz) as i32);
                if original[i] == Block::Source && !held.contains(&p) {
                    continue;
                }
                let mut beside: Vec<(i32, i32, i32)> = SIDES.iter().map(|(dx, dz)| (p.0 + dx, p.1, p.2 + dz)).filter(|n| self.index(n.0, n.1, n.2).is_some_and(|k| original[k] == Block::Source) && !held.contains(n)).collect();
                // A source held back that two sources made a source again:
                // all but one of them are held back too (the one with the
                // most sources about it stays). A new source: all of them.
                if original[i] == Block::Source {
                    beside.sort_by_key(|n| std::cmp::Reverse(SIDES.iter().filter(|(dx, dz)| self.index(n.0 + dx, n.1, n.2 + dz).is_some_and(|k| original[k] == Block::Source)).count()));
                    if !beside.is_empty() {
                        beside.remove(0);
                    }
                }
                for n in beside {
                    held.insert(n);
                    more = true;
                }
            }
            if !more {
                let made = self.blocks.iter().zip(&original).filter(|(b, o)| **b == Block::Source && **o != Block::Source).count();
                if made == 0 {
                    return (held.len(), true);
                }
                break;
            }
        }
        self.blocks = original;
        self.steady_wide(edge, budget)
    }

    /// Every source with air beside it made flowing, and the sources beside
    /// such a block but one wherever two would make a source of it again.
    pub fn steady_wide(&mut self, edge: Vec<(i32, i32, i32)>, budget: usize) -> (usize, bool) {
        // Under water, flowing water falls.
        let edge_of = |g: &Self, x: i32, y: i32, z: i32| if g.get(x, y + 1, z).is_water() { Block::Flowing { amount: 8, falling: true } } else { Block::Flowing { amount: 7, falling: false } };
        let edges: Vec<Block> = edge.iter().map(|&(x, y, z)| edge_of(self, x, y, z)).collect();
        for (&(x, y, z), &b) in edge.iter().zip(&edges) {
            self.set(x, y, z, b);
        }
        let mut made = edge.len();
        // Two sources beside a flowing block over a source or the ground
        // make a source of it: one of them is left.
        let mut work = edge.clone();
        let mut touched = edge;
        while let Some((x, y, z)) = work.pop() {
            if !matches!(self.get(x, y, z), Block::Flowing { .. }) || !matches!(self.get(x, y - 1, z), Block::Solid | Block::Source) {
                continue;
            }
            let mut beside: Vec<(i32, i32, i32)> = SIDES.iter().map(|(dx, dz)| (x + dx, y, z + dz)).filter(|p| self.get(p.0, p.1, p.2) == Block::Source).collect();
            if beside.len() < 2 {
                continue;
            }
            // The one with the most sources about it stays.
            beside.sort_by_key(|p| std::cmp::Reverse(self.sources_beside(p.0, p.1, p.2)));
            for &p in &beside[1..] {
                let b = edge_of(self, p.0, p.1, p.2);
                self.set(p.0, p.1, p.2, b);
                made += 1;
                work.push(p);
                touched.push(p);
                for (dx, dz) in SIDES {
                    work.push((p.0 + dx, y, p.2 + dz));
                }
            }
        }
        // What changed, and the water about it.
        let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
        let mut seen = std::collections::HashSet::new();
        for (x, y, z) in touched {
            for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                let p = (x + dx, y + dy, z + dz);
                if self.get(p.0, p.1, p.2).is_water() && seen.insert(p) {
                    queue.push_back(p);
                }
            }
        }
        let (_, settled) = self.run(&mut queue, budget);
        (made, settled)
    }

    /// The water blocks a tick may change: flowing water, and sources with
    /// air or flowing water above, under or beside them. (A source among
    /// sources and solid blocks does nothing when it ticks.)
    pub fn restless(&self) -> VecDeque<(i32, i32, i32)> {
        let mut queue = VecDeque::new();
        for (i, b) in self.blocks.iter().enumerate() {
            let (x, y, z) = (self.x0 + (i % self.nx) as i32, self.y0 + (i / (self.nx * self.nz)) as i32, self.z0 + ((i / self.nx) % self.nz) as i32);
            let restless = match b {
                Block::Flowing { .. } => true,
                Block::Source => [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)].iter().any(|(dx, dy, dz)| matches!(self.get(x + dx, y + dy, z + dz), Block::Air | Block::Flowing { .. })),
                _ => false,
            };
            if restless {
                queue.push_back((x, y, z));
            }
        }
        queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(g: &mut Grid, x0: i32, x1: i32, z0: i32, z1: i32, floor: i32, top: i32) {
        for z in z0..z1 {
            for x in x0..x1 {
                for y in g.y0..floor {
                    g.set(x, y, z, Block::Solid);
                }
                for y in floor..top {
                    g.set(x, y, z, Block::Source);
                }
            }
        }
    }

    fn settle(g: &mut Grid) -> bool {
        let mut q: VecDeque<_> = (0..g.blocks.len())
            .filter(|&i| g.blocks[i].is_water())
            .map(|i| {
                let x = (i % g.nx) as i32 + g.x0;
                let z = ((i / g.nx) % g.nz) as i32 + g.z0;
                let y = (i / (g.nx * g.nz)) as i32 + g.y0;
                (x, y, z)
            })
            .collect();
        g.run(&mut q, 10_000_000).1
    }

    #[test]
    fn still_water_in_a_basin_stays() {
        let mut g = Grid::new(0, 0, 0, 10, 10, 10);
        pool(&mut g, 0, 10, 0, 10, 5, 5);
        pool(&mut g, 2, 8, 2, 8, 2, 5);
        let before = g.blocks.clone();
        assert!(settle(&mut g));
        assert_eq!(g.blocks, before);
    }

    #[test]
    fn a_source_on_flat_ground_spreads_seven() {
        let mut g = Grid::new(0, 0, 0, 21, 4, 21);
        pool(&mut g, 0, 21, 0, 21, 1, 1);
        g.set(10, 1, 10, Block::Source);
        assert!(settle(&mut g));
        assert_eq!(g.get(10, 1, 10), Block::Source);
        assert_eq!(g.get(11, 1, 10), Block::Flowing { amount: 7, falling: false });
        assert_eq!(g.get(17, 1, 10), Block::Flowing { amount: 1, falling: false });
        assert_eq!(g.get(18, 1, 10), Block::Air);
        assert_eq!(g.get(13, 1, 13), Block::Flowing { amount: 2, falling: false });
    }

    #[test]
    fn a_step_with_a_straight_edge_runs_one_row() {
        // Upstream at 5, downstream at 4, an edge along z.
        let mut g = Grid::new(0, 0, 0, 12, 8, 6);
        pool(&mut g, 0, 12, 0, 6, 1, 1);
        pool(&mut g, 0, 6, 1, 5, 1, 6);
        pool(&mut g, 6, 12, 1, 5, 1, 5);
        // Banks that hold the upper water.
        for x in 0..12 {
            for y in 1..7 {
                g.set(x, y, 0, Block::Solid);
                g.set(x, y, 5, Block::Solid);
            }
        }
        assert!(settle(&mut g));
        for z in 1..5 {
            assert_eq!(g.get(6, 5, z), Block::Flowing { amount: 7, falling: false });
            assert_eq!(g.get(7, 5, z), Block::Air);
        }
    }

    #[test]
    fn a_corner_in_a_step_makes_sources() {
        // An L-shaped upper pool: the lower water rises where two sources
        // meet over it.
        let mut g = Grid::new(0, 0, 0, 10, 8, 10);
        pool(&mut g, 0, 10, 0, 10, 1, 4);
        pool(&mut g, 0, 5, 0, 10, 1, 5);
        pool(&mut g, 5, 10, 0, 5, 1, 5);
        assert!(settle(&mut g));
        assert_eq!(g.get(5, 4, 5), Block::Source);
    }
}
