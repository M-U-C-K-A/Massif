//! The land a structure is placed in: its heights, where it is water, what
//! block is where. The map's own ([`MapGround`]), or a flat one for tests.

use massif_core::compose::{Materializer, Span, TileStack};
use massif_core::coords::{TilePos, local_index};
use massif_core::world::Dimension;
use std::collections::HashMap;
use std::sync::Mutex;

/// What the world has at a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldBlock {
    Air,
    Water,
    Lava,
    Solid(String),
}

impl WorldBlock {
    /// Its block state, as rules match it.
    pub fn state(&self) -> &str {
        match self {
            WorldBlock::Air => "minecraft:air",
            WorldBlock::Water => "minecraft:water[level=0]",
            WorldBlock::Lava => "minecraft:lava[level=0]",
            WorldBlock::Solid(s) => s,
        }
    }

    pub fn is_solid(&self) -> bool {
        matches!(self, WorldBlock::Solid(_))
    }
}

/// The land, as structures read it.
pub trait Ground: Sync {
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
    /// The first free block above the topmost block that is not air, water
    /// included (the game's `WORLD_SURFACE_WG`).
    fn surface(&self, x: i32, z: i32) -> i32;
    /// The first free block above the topmost solid block (the game's
    /// `OCEAN_FLOOR_WG`).
    fn floor(&self, x: i32, z: i32) -> i32;
    fn block(&self, x: i32, y: i32, z: i32) -> WorldBlock;

    /// Whether the block at `(x, y, z)` is air (quicker than asking which
    /// block it is).
    fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        self.block(x, y, z) == WorldBlock::Air
    }
}

/// Flat land, for tests: stone up to `height` (exclusive), water up to
/// `water` above it.
pub struct Flat {
    pub height: i32,
    pub water: Option<i32>,
}

impl Ground for Flat {
    fn min_y(&self) -> i32 {
        -64
    }

    fn max_y(&self) -> i32 {
        320
    }

    fn surface(&self, _: i32, _: i32) -> i32 {
        self.water.map_or(self.height, |w| w.max(self.height))
    }

    fn floor(&self, _: i32, _: i32) -> i32 {
        self.height
    }

    fn block(&self, _: i32, y: i32, _: i32) -> WorldBlock {
        if y < self.height {
            WorldBlock::Solid(if y < 0 { "minecraft:deepslate".into() } else { "minecraft:stone".into() })
        } else if self.water.is_some_and(|w| y < w) {
            WorldBlock::Water
        } else {
            WorldBlock::Air
        }
    }
}

/// A column of the map as structures read it: its solid spans and the
/// water over them.
#[derive(Debug, Clone, Default)]
struct Column {
    spans: Vec<Span>,
    fluids: Vec<(i32, i32)>,
}

/// The map's land: what the export would write, read column by column
/// (each column composed once).
pub struct MapGround<'a> {
    dim: &'a Dimension,
    mat: Materializer<'a>,
    reg: &'a massif_core::registry::Registry,
    lava: bool,
    columns: Mutex<HashMap<(i32, i32), std::sync::Arc<Column>>>,
}

impl<'a> MapGround<'a> {
    pub fn new(world: &'a massif_core::world::World, dim: usize) -> Self {
        let d = &world.dims[dim];
        Self {
            dim: d,
            mat: Materializer::new(&world.registry, world.meta.seed, d.min_y),
            reg: &world.registry,
            lava: world.meta.lava,
            columns: Mutex::new(HashMap::new()),
        }
    }

    fn column(&self, x: i32, z: i32) -> std::sync::Arc<Column> {
        if let Some(c) = self.columns.lock().unwrap_or_else(|e| e.into_inner()).get(&(x, z)) {
            return c.clone();
        }
        let pos = TilePos::containing(x, z);
        let mut col = Column::default();
        if let Some(stack) = TileStack::new(self.dim, pos, true) {
            let cell = local_index(x, z);
            stack.spans(cell, &mut col.spans);
            stack.fluids(cell, &col.spans, &mut col.fluids);
        }
        let col = std::sync::Arc::new(col);
        let mut cache = self.columns.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > 1 << 20 {
            cache.clear();
        }
        cache.insert((x, z), col.clone());
        col
    }

    fn top(&self, col: &Column) -> i32 {
        col.spans.last().map_or(self.dim.min_y, |s| s.y1)
    }
}

impl Ground for MapGround<'_> {
    fn min_y(&self) -> i32 {
        self.dim.min_y
    }

    fn max_y(&self) -> i32 {
        self.dim.max_y
    }

    fn surface(&self, x: i32, z: i32) -> i32 {
        let col = self.column(x, z);
        let water = col.fluids.iter().map(|f| f.1).max().unwrap_or(i32::MIN);
        self.top(&col).max(water)
    }

    fn floor(&self, x: i32, z: i32) -> i32 {
        self.top(&self.column(x, z))
    }

    fn block(&self, x: i32, y: i32, z: i32) -> WorldBlock {
        let col = self.column(x, z);
        for span in &col.spans {
            if y >= span.y0 && y < span.y1 {
                let wet = col.fluids.iter().any(|f| f.0 == span.y1);
                return WorldBlock::Solid(self.reg.block(self.mat.block(span, x, y, z, wet)).state.clone());
            }
        }
        if col.fluids.iter().any(|f| y >= f.0 && y < f.1) {
            return if self.lava { WorldBlock::Lava } else { WorldBlock::Water };
        }
        WorldBlock::Air
    }

    fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        let col = self.column(x, z);
        !col.spans.iter().any(|s| y >= s.y0 && y < s.y1) && !col.fluids.iter().any(|f| y >= f.0 && y < f.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, generate};

    #[test]
    fn the_map_is_read_as_the_export_writes_it() {
        let w = generate(&GenParams { tiles_x: 2, tiles_z: 2, seed: 4, ..GenParams::default() });
        let g = MapGround::new(&w, 0);
        let mut seen_water = false;
        for x in (-120..120).step_by(7) {
            for z in (-120..120).step_by(11) {
                let (s, f) = (g.surface(x, z), g.floor(x, z));
                assert!(s >= f, "{x} {z}");
                assert!(g.block(x, f - 1, z).is_solid(), "{x} {z}: the floor stands on something");
                assert!(!g.block(x, f, z).is_solid());
                if s > f {
                    assert_eq!(g.block(x, s - 1, z), WorldBlock::Water);
                    seen_water = true;
                }
                assert_eq!(g.block(x, s, z), WorldBlock::Air);
            }
        }
        let _ = seen_water;
    }
}
