//! Signed block / tile / chunk coordinates.
//!
//! Everything is `i32` and symmetric around zero. Tile and chunk indices are
//! always derived with `div_euclid` / `rem_euclid`: plain `/` and `%` truncate
//! toward zero and would fold block -1 into tile 0 with a negative remainder.

use serde::{Deserialize, Serialize};

pub const TILE_SHIFT: u32 = 7;
/// Edge length of a tile, in blocks.
pub const TILE_SIZE: i32 = 1 << TILE_SHIFT;
/// Number of columns in a tile.
pub const TILE_AREA: usize = (TILE_SIZE as usize) * (TILE_SIZE as usize);

/// Edge length of a Minecraft chunk / section, in blocks.
pub const CHUNK_SIZE: i32 = 16;
/// A region file holds 32×32 chunks, i.e. 512×512 blocks or 4×4 tiles.
pub const REGION_SIZE: i32 = 512;

/// Position of a tile in the tile grid (tile `(0, 0)` covers blocks `0..128`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TilePos {
    pub x: i32,
    pub z: i32,
}

impl TilePos {
    #[inline]
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// The tile containing block column `(bx, bz)`.
    #[inline]
    pub const fn containing(bx: i32, bz: i32) -> Self {
        Self {
            x: bx.div_euclid(TILE_SIZE),
            z: bz.div_euclid(TILE_SIZE),
        }
    }

    /// Block coordinates of the tile's north-west corner.
    #[inline]
    pub const fn origin(self) -> (i32, i32) {
        (self.x * TILE_SIZE, self.z * TILE_SIZE)
    }

    #[inline]
    pub const fn offset(self, dx: i32, dz: i32) -> Self {
        Self {
            x: self.x + dx,
            z: self.z + dz,
        }
    }

    /// The block rectangle covered by this tile.
    #[inline]
    pub const fn rect(self) -> BlockRect {
        let (x0, z0) = self.origin();
        BlockRect {
            x0,
            z0,
            x1: x0 + TILE_SIZE,
            z1: z0 + TILE_SIZE,
        }
    }
}

/// Index of block column `(bx, bz)` inside its tile, row-major (Z then X).
#[inline]
pub const fn local_index(bx: i32, bz: i32) -> usize {
    ((bz.rem_euclid(TILE_SIZE) as usize) << TILE_SHIFT) | (bx.rem_euclid(TILE_SIZE) as usize)
}

/// Index of the tile-local column `(lx, lz)`, both in `0..128`.
#[inline]
pub const fn cell_index(lx: usize, lz: usize) -> usize {
    (lz << TILE_SHIFT) | lx
}

/// A half-open rectangle of block columns: `x0 <= x < x1`, `z0 <= z < z1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRect {
    pub x0: i32,
    pub z0: i32,
    pub x1: i32,
    pub z1: i32,
}

impl BlockRect {
    pub const fn new(x0: i32, z0: i32, x1: i32, z1: i32) -> Self {
        Self { x0, z0, x1, z1 }
    }

    /// The smallest rectangle of whole columns covering a disc/square of
    /// `radius` blocks around the continuous point `(cx, cz)`.
    pub fn around(cx: f32, cz: f32, radius: f32) -> Self {
        Self {
            x0: (cx - radius).floor() as i32,
            z0: (cz - radius).floor() as i32,
            x1: (cx + radius).ceil() as i32,
            z1: (cz + radius).ceil() as i32,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.z1 <= self.z0
    }

    pub const fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub const fn depth(&self) -> i32 {
        self.z1 - self.z0
    }

    pub fn intersect(&self, o: &BlockRect) -> BlockRect {
        BlockRect {
            x0: self.x0.max(o.x0),
            z0: self.z0.max(o.z0),
            x1: self.x1.min(o.x1),
            z1: self.z1.min(o.z1),
        }
    }

    pub fn expand(&self, by: i32) -> BlockRect {
        BlockRect {
            x0: self.x0 - by,
            z0: self.z0 - by,
            x1: self.x1 + by,
            z1: self.z1 + by,
        }
    }

    /// Every tile touched by this rectangle, row by row.
    pub fn tiles(&self) -> impl Iterator<Item = TilePos> + use<> {
        let empty = self.is_empty();
        let tx0 = self.x0.div_euclid(TILE_SIZE);
        let tz0 = self.z0.div_euclid(TILE_SIZE);
        let tx1 = (self.x1 - 1).div_euclid(TILE_SIZE);
        let tz1 = (self.z1 - 1).div_euclid(TILE_SIZE);
        (tz0..=tz1)
            .flat_map(move |z| (tx0..=tx1).map(move |x| TilePos::new(x, z)))
            .filter(move |_| !empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_blocks_map_to_negative_tiles() {
        assert_eq!(TilePos::containing(0, 0), TilePos::new(0, 0));
        assert_eq!(TilePos::containing(127, 127), TilePos::new(0, 0));
        assert_eq!(TilePos::containing(128, -1), TilePos::new(1, -1));
        assert_eq!(TilePos::containing(-1, -128), TilePos::new(-1, -1));
        assert_eq!(TilePos::containing(-129, -129), TilePos::new(-2, -2));
    }

    #[test]
    fn local_index_is_always_in_range() {
        assert_eq!(local_index(0, 0), 0);
        assert_eq!(local_index(-1, 0), 127);
        assert_eq!(local_index(0, -1), 127 << 7);
        assert_eq!(local_index(-1, -1), TILE_AREA - 1);
        for b in [-1000, -129, -128, -127, -1, 0, 1, 127, 128, 5000] {
            assert!(local_index(b, b) < TILE_AREA);
            let t = TilePos::containing(b, b);
            let (ox, oz) = t.origin();
            assert!(b >= ox && b < ox + TILE_SIZE && b >= oz && b < oz + TILE_SIZE);
        }
    }

    #[test]
    fn rect_tiles_cover_the_rect() {
        let r = BlockRect::new(-130, -1, 1, 129);
        let tiles: Vec<_> = r.tiles().collect();
        assert_eq!(tiles.len(), 3 * 3);
        assert!(tiles.contains(&TilePos::new(-2, -1)));
        assert!(tiles.contains(&TilePos::new(0, 1)));
        assert_eq!(BlockRect::new(5, 5, 5, 9).tiles().count(), 0);
    }
}
