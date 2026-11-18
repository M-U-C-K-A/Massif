//! The last look at a tile's caves before the export: no water nor lava
//! may run once the game updates a block beside it.
//!
//! The caves keep their fluids sealed as they are carved (aquifer walls,
//! no fluid beside the open air of the columns around nor caves beside the
//! water over the ground, within a tile). What a tile cannot know is
//! sealed here, from the columns as the export composes them, the tile's
//! and a column around it: a cave's air or a level's (a carve, a room
//! under a ledge, a cube sculpted open) that water or lava beside it or
//! over it would run into becomes rock, and so does the block of a cave's
//! water or lava that would run into the open air. Each tile seals only
//! its own blocks, against the fluids of its own and of the columns
//! around, so that every tile of a map, done apart, seals every pair: no
//! fluid is drained, nothing spreads from one tile to the next.

use crate::compose::{Span, TileStack};
use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::voxel::Fluid;
use crate::world::Dimension;
use std::collections::HashMap;

/// The blocks of a tile to make rock, by column.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Seals {
    /// `(cell, y)`, sorted.
    blocks: Vec<(u16, i16)>,
}

impl Seals {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// The heights of column `cell` to make rock.
    pub fn column(&self, cell: usize) -> impl Iterator<Item = i32> + '_ {
        let at = self.blocks.partition_point(|b| (b.0 as usize) < cell);
        self.blocks[at..].iter().take_while(move |b| b.0 as usize == cell).map(|b| b.1 as i32)
    }
}

/// What a column holds, as the export composes it.
struct Column {
    solid: Vec<Span>,
    /// The fluids over the ground (lakes, the sea), not those of caves.
    surface: Vec<(i32, i32)>,
    caves: Vec<(i32, i32, Fluid)>,
    /// The top of its highest run: air under it is closed in, over it open.
    top: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Rock,
    Water,
    Lava,
    /// Air under the column's top: a cave, a room.
    Closed,
    /// The open air over the column.
    Open,
}

impl Column {
    fn at(&self, y: i32) -> Cell {
        if self.solid.iter().any(|s| s.y0 <= y && y < s.y1) {
            return Cell::Rock;
        }
        if let Some(&(_, _, f)) = self.caves.iter().find(|c| c.0 <= y && y < c.1) {
            return if f == Fluid::Lava { Cell::Lava } else { Cell::Water };
        }
        if self.surface.iter().any(|s| s.0 <= y && y < s.1) {
            return Cell::Water;
        }
        if y < self.top { Cell::Closed } else { Cell::Open }
    }
}

/// The blocks of the tile at `pos` to make rock so that no fluid runs (see
/// the module). `hidden`: whether hidden levels are exported.
pub fn seal_tile(dim: &Dimension, pos: TilePos, hidden: bool) -> Seals {
    let Some(stack) = TileStack::new(dim, pos, hidden) else { return Seals::default() };
    // Nothing of the tile may be open to a fluid but where caves, levels or
    // sculpted chunks hollow it.
    if stack.caves().is_none() && !stack.is_sculpted() && stack.layers().nth(1).is_none() {
        return Seals::default();
    }
    let n = TILE_SIZE;
    let (ox, oz) = pos.origin();
    let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
    stacks.insert(pos, Some(stack));
    // The columns of the tile and the ring around it.
    let side = (n + 2) as usize;
    let mut columns: Vec<Option<Column>> = Vec::with_capacity(side * side);
    let (mut spans, mut fluids, mut caves) = (Vec::new(), Vec::new(), Vec::new());
    for j in -1..=n {
        for i in -1..=n {
            let (x, z) = (ox + i, oz + j);
            let p = TilePos::containing(x, z);
            let Some(stack) = stacks.entry(p).or_insert_with(|| TileStack::new(dim, p, hidden)) else {
                columns.push(None);
                continue;
            };
            let cell = local_index(x, z);
            if !stack.ground().present(cell) {
                columns.push(None);
                continue;
            }
            stack.spans(cell, &mut spans);
            stack.fluids(cell, &spans, &mut fluids);
            stack.cave_fluids(cell, &mut caves);
            // The fluids that are not a cave's: over the ground.
            let mut surface = Vec::new();
            for &(a, b) in &fluids {
                let mut y = a;
                for c in caves.iter().filter(|c| c.1 > a && c.0 < b) {
                    if c.0 > y {
                        surface.push((y, c.0));
                    }
                    y = y.max(c.1);
                }
                if y < b {
                    surface.push((y, b));
                }
            }
            columns.push(Some(Column { solid: spans.clone(), surface, caves: caves.clone(), top: spans.last().map_or(dim.min_y, |s| s.y1) }));
        }
    }
    // `None` off the map, and past the ring (whose blocks are not sealed
    // here).
    let at = |i: i32, j: i32| {
        if !(-1..=n).contains(&i) || !(-1..=n).contains(&j) {
            return None;
        }
        columns[((j + 1) as usize) * side + (i + 1) as usize].as_ref()
    };
    let inside = |i: i32, j: i32| (0..n).contains(&i) && (0..n).contains(&j);
    let mut out: Vec<(u16, i16)> = Vec::new();
    let mut seal = |i: i32, j: i32, y: i32| out.push((local_index(ox + i, oz + j) as u16, y as i16));
    for j in -1..=n {
        for i in -1..=n {
            let Some(col) = at(i, j) else { continue };
            // Every block of a cave's fluid; of the fluid over the ground,
            // only along the tile's edges (inside it the caves keep away).
            let edge = !(1..n - 1).contains(&i) || !(1..n - 1).contains(&j);
            let runs: Vec<(i32, i32, Cell)> = col
                .caves
                .iter()
                .map(|&(a, b, f)| (a, b, if f == Fluid::Lava { Cell::Lava } else { Cell::Water }))
                .chain(col.surface.iter().filter(|_| edge).map(|&(a, b)| (a, b, Cell::Water)))
                .collect();
            for (a, b, fluid) in runs {
                let cave = col.caves.iter().any(|c| c.0 <= a && b <= c.1);
                for y in a..b {
                    let mut runs_off = false;
                    for (di, dj, dy) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, -1)] {
                        let (ni, nj, ny) = (i + di, j + dj, y + dy);
                        let Some(next) = at(ni, nj) else {
                            // Off the map: the game's own land is there.
                            continue;
                        };
                        match next.at(ny) {
                            Cell::Rock => {}
                            same if same == fluid => {}
                            Cell::Closed | Cell::Water | Cell::Lava => {
                                if inside(ni, nj) {
                                    seal(ni, nj, ny);
                                }
                            }
                            Cell::Open => runs_off = true,
                        }
                    }
                    if runs_off && cave && inside(i, j) {
                        seal(i, j, y);
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Seals { blocks: out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caves::{CaveDef, CaveLayers};
    use crate::tile::{NO_FLUID, Tile};
    use crate::world::{World, WorldMeta};

    /// Caves under a lake at a tile's edge and among hills: once sealed,
    /// no fluid has air beside or under it.
    #[test]
    fn sealed_tiles_hold_their_fluids() {
        let mut w = World::new(WorldMeta::default());
        let dim = &mut w.dims[0];
        dim.caves = CaveLayers::new(vec![CaveDef::vanilla("Caves", 11, dim.min_y, dim.max_y, 63)]);
        for z in 0..2 {
            for x in 0..2 {
                let mut tile = Tile::ground(90.0, 1, NO_FLUID);
                for i in 0..crate::coords::TILE_AREA {
                    let (bx, bz) = (x * 128 + (i % 128) as i32, z * 128 + (i / 128) as i32);
                    tile.caves.set(i, 1);
                    // Hills, and a lake across the tiles' border.
                    let h = 80.0 + 14.0 * ((bx as f32) * 0.07).sin() * ((bz as f32) * 0.05).cos();
                    tile.top.set(i, h.round());
                    if (bx - 128).abs() < 10 && bz < 100 {
                        tile.top.set(i, 50.0);
                        tile.fluid.set(i, 62);
                    }
                }
                dim.ground_mut().insert(TilePos::new(x, z), tile);
            }
        }
        let dim = &w.dims[0];
        let seals: HashMap<TilePos, Seals> = dim.ground().positions().map(|p| (p, seal_tile(dim, p, false))).collect();
        // The composed columns, with the seals made rock.
        let stacks: HashMap<TilePos, TileStack> = dim.ground().positions().filter_map(|p| Some((p, TileStack::new(dim, p, false)?))).collect();
        let mut columns: HashMap<(i32, i32), Column> = HashMap::new();
        for z in 0..256 {
            for x in 0..256 {
                let p = TilePos::containing(x, z);
                let stack = &stacks[&p];
                let cell = local_index(x, z);
                let (mut spans, mut fluids, mut caves) = (Vec::new(), Vec::new(), Vec::new());
                stack.spans(cell, &mut spans);
                stack.fluids(cell, &spans, &mut fluids);
                stack.cave_fluids(cell, &mut caves);
                for y in seals[&p].column(cell) {
                    spans.push(Span { y0: y, y1: y + 1, terrain: crate::registry::TerrainId::ROCK, top: (y + 1) as f32 });
                }
                let surface = fluids.iter().filter(|f| !caves.iter().any(|c| c.0 <= f.0 && f.1 <= c.1)).copied().collect();
                let top = spans.iter().map(|s| s.y1).max().unwrap();
                columns.insert((x, z), Column { solid: spans, surface, caves, top });
            }
        }
        let cell_at = |x: i32, y: i32, z: i32| columns.get(&(x, z)).map_or(Cell::Rock, |c| c.at(y));
        let mut wet = 0;
        for z in (1..255).step_by(5) {
            for x in 1..255 {
                for y in -60..90 {
                    let c = cell_at(x, y, z);
                    if c != Cell::Water && c != Cell::Lava {
                        continue;
                    }
                    wet += 1;
                    for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0)] {
                        let n = cell_at(x + dx, y + dy, z + dz);
                        assert!(n == Cell::Rock || n == c, "{c:?} at {x} {y} {z} beside {n:?} ({dx} {dy} {dz})");
                    }
                }
            }
        }
        assert!(wet > 5000, "{wet}");
        assert!(seals.values().all(|s| s.len() < 20_000), "few blocks sealed");
    }
}
