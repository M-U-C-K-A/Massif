//! Slabs: the one-block steps of the land smoothed when it is exported, as
//! WorldPainter's Half Blocks did in the fork: at the foot of each step, a
//! slab of the block at the surface makes a half step, and, if asked, a
//! stair fills each inner corner, where the land steps up on two sides.
//!
//! Where it is done is painted (the Slabs tool, Edit ▸ Whole map ▸ Slabs)
//! into each ground tile's [`crate::Tile::slabs`] plane: 0 for none, else a
//! value of [`value`], which says which slab (the one that matches the
//! block at the surface, or a block chosen by hand) and whether stairs go
//! in the corners.
//!
//! Which slab matches a block is found by name, as the Blocks panel finds
//! the shapes of a block (`stone_bricks` → `stone_brick_slab`,
//! `oak_planks` → `oak_slab`, `conquest:old_bricks` →
//! `conquest:old_bricks_slab`), among the blocks known to exist: the
//! game's for the version exported to, and the mods' read. A block
//! without one (grass, dirt, sand) stays whole.

use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::level::TileRef;
use crate::registry::{BlockId, Registry};
use crate::world::Dimension;

/// The value of a column smoothed with the slab matching its surface.
pub const AUTO: u16 = 1;
/// The bit of a value asking for stairs in the inner corners.
pub const STAIRS: u16 = 0x8000;

/// The value a column of the slabs plane takes: the slab matching the
/// block at its surface (`block` `None`), or the slab `block` of the
/// registry; with stairs in the inner corners or not.
pub fn value(block: Option<BlockId>, stairs: bool) -> u16 {
    let slab = match block {
        Some(b) => (b.0 + 2).min(STAIRS - 1),
        None => AUTO,
    };
    slab | if stairs { STAIRS } else { 0 }
}

/// What a value of the slabs plane asks for: `None` for 0, else the slab
/// chosen by hand (`None`: the matching one) and whether stairs go in the
/// corners.
pub fn decode(v: u16) -> Option<(Option<BlockId>, bool)> {
    let slab = v & !STAIRS;
    match slab {
        0 => None,
        AUTO => Some((None, v & STAIRS != 0)),
        n => Some((Some(BlockId(n - 2)), v & STAIRS != 0)),
    }
}

/// The four sides, clockwise from north, as the game names a stair's
/// facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    pub const fn name(self) -> &'static str {
        match self {
            Side::North => "north",
            Side::East => "east",
            Side::South => "south",
            Side::West => "west",
        }
    }

    /// The step to the neighbour on this side.
    pub const fn step(self) -> (i32, i32) {
        match self {
            Side::North => (0, -1),
            Side::East => (1, 0),
            Side::South => (0, 1),
            Side::West => (-1, 0),
        }
    }

    const fn clockwise(self) -> Side {
        match self {
            Side::North => Side::East,
            Side::East => Side::South,
            Side::South => Side::West,
            Side::West => Side::North,
        }
    }
}

/// What smooths the foot of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// A bottom slab.
    Slab,
    /// A bottom stair in an inner corner: its back toward `facing`, and its
    /// extra quarter on its left (`left`, counter-clockwise of `facing`) or
    /// right, the two sides the land steps up on.
    Corner { facing: Side, left: bool },
}

impl Shape {
    /// The block state's `shape` of a stair (`inner_left`, `inner_right`).
    pub const fn stair_shape(self) -> &'static str {
        match self {
            Shape::Corner { left: true, .. } => "inner_left",
            _ => "inner_right",
        }
    }
}

/// What smooths the column whose top block is at `y1 - 1`, given the same
/// of its neighbours north, east, south and west (`None` where there is
/// no land): a slab where a neighbour is one block higher, a stair where
/// two neighbours side by side are, if `stairs`.
pub fn shape_at(y1: i32, sides: [Option<i32>; 4], stairs: bool) -> Option<Shape> {
    let up = sides.map(|s| s == Some(y1 + 1));
    if !up.iter().any(|&u| u) {
        return None;
    }
    if stairs && up.iter().filter(|&&u| u).count() == 2 {
        for (k, side) in Side::ALL.into_iter().enumerate() {
            let next = (k + 1) % 4;
            if up[k] && up[next] {
                // The step up on `side` and on the side clockwise of it:
                // the stair faces `side`, its extra quarter on its right.
                debug_assert_eq!(Side::ALL[next], side.clockwise());
                return Some(Shape::Corner { facing: side, left: false });
            }
        }
    }
    Some(Shape::Slab)
}

/// A column the export smooths: the cell in its tile, the height of the
/// slab or stair (the block over the column's top), what it is, and the
/// slab chosen by hand (`None`: the one matching the surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub cell: u16,
    pub y: i32,
    pub shape: Shape,
    pub block: Option<BlockId>,
}

/// The columns of the ground tile at `pos` the export smooths, by cell:
/// from the ground's heights, its own and its neighbours' (across tiles).
/// Empty where nothing asks for slabs.
pub fn plan_tile(dim: &Dimension, pos: TilePos) -> Vec<Placed> {
    let ground = dim.ground();
    let Some(here) = ground.view(pos) else { return Vec::new() };
    if here.slabs.uniform_value() == Some(0) {
        return Vec::new();
    }
    let (ox, oz) = pos.origin();
    // The tiles around, read when a column at an edge needs one.
    let mut near: [Option<Option<TileRef>>; 4] = [None, None, None, None];
    let mut out = Vec::new();
    for lz in 0..TILE_SIZE {
        for lx in 0..TILE_SIZE {
            let i = local_index(lx, lz);
            let Some((block, stairs)) = decode(here.slabs.get(i)) else { continue };
            let y1 = crate::compose::block_top(here.top.get(i));
            let mut sides = [None; 4];
            for (k, side) in Side::ALL.into_iter().enumerate() {
                let (dx, dz) = side.step();
                let (nx, nz) = (lx + dx, lz + dz);
                sides[k] = if (0..TILE_SIZE).contains(&nx) && (0..TILE_SIZE).contains(&nz) {
                    Some(crate::compose::block_top(here.top.get(local_index(nx, nz))))
                } else {
                    let tile = near[k].get_or_insert_with(|| ground.view(pos.offset(dx, dz)));
                    tile.as_ref().map(|t| crate::compose::block_top(t.top.get(local_index(ox + nx, oz + nz))))
                };
            }
            if let Some(shape) = shape_at(y1, sides, stairs) {
                out.push(Placed { cell: i as u16, y: y1, shape, block });
            }
        }
    }
    out
}

/// The names a block's slab may have, the likeliest first: `{base}_slab`
/// for the block's name and its shorter forms (`stone_bricks` →
/// `stone_brick`, `oak_planks` → `oak`, `quartz_block` → `quartz`).
fn bases(name: &str) -> Vec<String> {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        if !p.is_empty() && !out.iter().any(|o| o == p) {
            out.push(p.to_owned());
        }
    };
    push(path);
    for suffix in ["_planks", "_block", "_blocks"] {
        if let Some(p) = path.strip_suffix(suffix) {
            push(p);
        }
    }
    if let Some(p) = path.strip_suffix('s') {
        push(p);
    }
    out.into_iter().map(|p| format!("{ns}:{p}")).collect()
}

/// The slab of the block `name` (without properties), if one exists by
/// `exists`; a slab is its own.
pub fn find_slab(name: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    if is_slab(name) {
        return Some(name.to_owned());
    }
    bases(name).into_iter().map(|b| format!("{b}_slab")).find(|s| exists(s))
}

/// The stairs of the block `name`, or of the block a slab is of
/// (`oak_slab` → `oak_stairs`), if they exist by `exists`.
pub fn find_stairs(name: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let base = name.strip_suffix("_slab").unwrap_or(name);
    if let Some(s) = name.strip_suffix("_slab").map(|b| format!("{b}_stairs")).filter(|s| exists(s)) {
        return Some(s);
    }
    // A slab of planks has stairs of the same wood (`oak_slab` →
    // `oak_stairs` above), a slab of bricks those of the bricks.
    bases(base).into_iter().map(|b| format!("{b}_stairs")).find(|s| exists(s))
}

/// Whether a block (without properties) is a slab, by its name.
pub fn is_slab(name: &str) -> bool {
    name.ends_with("_slab")
}

/// The slab and stairs (block names) of each block of a registry, by
/// [`BlockId`]: `None` for a block with no slab.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Family {
    pub of: Vec<Option<(String, Option<String>)>>,
}

impl Family {
    /// The slab and stairs of every block of `reg`, among the blocks that
    /// `exists`.
    pub fn new(reg: &Registry, exists: &dyn Fn(&str) -> bool) -> Self {
        let of = reg
            .blocks
            .iter()
            .map(|b| {
                let name = b.name();
                if name == "minecraft:air" || name == "minecraft:water" || name == "minecraft:lava" {
                    return None;
                }
                find_slab(name, exists).map(|slab| {
                    let stairs = find_stairs(&slab, exists).or_else(|| find_stairs(name, exists));
                    (slab, stairs)
                })
            })
            .collect();
        Self { of }
    }

    /// Whether the block `id` has a slab.
    pub fn has_slab(&self, id: BlockId) -> bool {
        self.of.get(id.index()).is_some_and(Option::is_some)
    }
}

/// Smooth the whole map's steps with `value` (0 smooths none), within the
/// selection when there is one.
pub fn fill_slabs(edit: &mut crate::history::Edit, value: u16) {
    let positions: Vec<TilePos> = edit.level().positions().collect();
    let selection = edit.dim().selection.clone();
    let rect = positions.iter().fold(crate::coords::BlockRect::new(0, 0, 0, 0), |r, p| {
        let t = p.rect();
        if r.is_empty() { t } else { crate::coords::BlockRect::new(r.x0.min(t.x0), r.z0.min(t.z0), r.x1.max(t.x1), r.z1.max(t.z1)) }
    });
    edit.par_tiles(positions, rect, false, |pos, _, tile| {
        if tile.bottom.is_some() {
            return;
        }
        let (ox, oz) = pos.origin();
        match &selection {
            None => tile.slabs = crate::plane::Plane::Uniform(value),
            Some(sel) => {
                for lz in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        if sel.get(ox + lx, oz + lz) >= crate::selection::HALF {
                            tile.slabs.set(local_index(lx, lz), value);
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::tile::Tile;
    use crate::world::{World, WorldMeta};

    const VANILLA: &[&str] = &[
        "minecraft:stone_slab",
        "minecraft:stone_stairs",
        "minecraft:stone_brick_slab",
        "minecraft:stone_brick_stairs",
        "minecraft:oak_slab",
        "minecraft:oak_stairs",
        "minecraft:quartz_slab",
        "minecraft:quartz_stairs",
        "minecraft:smooth_stone_slab",
        "minecraft:sandstone_slab",
        "minecraft:sandstone_stairs",
        "conquest:old_bricks_slab",
        "conquest:old_bricks_stairs",
        "conquest:worn_dark_lime_mortar_masonry_slab",
    ];

    fn known(name: &str) -> bool {
        VANILLA.contains(&name)
    }

    #[test]
    fn a_block_finds_its_slab_and_stairs_by_name() {
        let slab = |n: &str| find_slab(n, &known);
        let stairs = |n: &str| find_stairs(n, &known);
        assert_eq!(slab("minecraft:stone").as_deref(), Some("minecraft:stone_slab"));
        assert_eq!(slab("minecraft:stone_bricks").as_deref(), Some("minecraft:stone_brick_slab"));
        assert_eq!(slab("minecraft:oak_planks").as_deref(), Some("minecraft:oak_slab"));
        assert_eq!(slab("minecraft:quartz_block").as_deref(), Some("minecraft:quartz_slab"));
        assert_eq!(slab("minecraft:smooth_stone").as_deref(), Some("minecraft:smooth_stone_slab"));
        assert_eq!(slab("minecraft:grass_block"), None, "grass has no slab: it stays whole");
        assert_eq!(slab("minecraft:dirt"), None);
        // A mod's block, by the same names.
        assert_eq!(slab("conquest:old_bricks").as_deref(), Some("conquest:old_bricks_slab"));
        assert_eq!(slab("conquest:worn_dark_lime_mortar_masonry").as_deref(), Some("conquest:worn_dark_lime_mortar_masonry_slab"));
        // Stairs, of a block or of its slab.
        assert_eq!(stairs("minecraft:oak_slab").as_deref(), Some("minecraft:oak_stairs"));
        assert_eq!(stairs("minecraft:stone_brick_slab").as_deref(), Some("minecraft:stone_brick_stairs"));
        assert_eq!(stairs("minecraft:smooth_stone_slab"), None);
        assert_eq!(stairs("conquest:old_bricks_slab").as_deref(), Some("conquest:old_bricks_stairs"));
        // A slab chosen by hand is its own slab.
        assert_eq!(slab("minecraft:oak_slab").as_deref(), Some("minecraft:oak_slab"));
    }

    #[test]
    fn values_say_which_slab_and_whether_stairs() {
        assert_eq!(decode(0), None);
        assert_eq!(decode(value(None, false)), Some((None, false)));
        assert_eq!(decode(value(None, true)), Some((None, true)));
        assert_eq!(decode(value(Some(BlockId(7)), true)), Some((Some(BlockId(7)), true)));
        assert_eq!(decode(value(Some(BlockId(0)), false)), Some((Some(BlockId(0)), false)));
    }

    #[test]
    fn slabs_at_the_foot_of_steps_and_stairs_in_inner_corners() {
        let n = |v: [i32; 4]| v.map(Some);
        // Flat: nothing.
        assert_eq!(shape_at(64, n([64; 4]), true), None);
        // A step up to the north: a slab.
        assert_eq!(shape_at(64, n([65, 64, 64, 64]), true), Some(Shape::Slab));
        // A cliff (two blocks or more) is no step.
        assert_eq!(shape_at(64, n([66, 64, 64, 64]), true), None);
        // Steps up north and east: an inner corner, a stair facing north
        // with its extra quarter east (its right) — or a slab without
        // stairs.
        assert_eq!(shape_at(64, n([65, 65, 64, 64]), true), Some(Shape::Corner { facing: Side::North, left: false }));
        assert_eq!(shape_at(64, n([65, 65, 64, 64]), false), Some(Shape::Slab));
        // West and north: facing west, its right is north.
        assert_eq!(shape_at(64, n([65, 64, 64, 65]), true), Some(Shape::Corner { facing: Side::West, left: false }));
        // Opposite sides: a slab in the trough.
        assert_eq!(shape_at(64, n([65, 64, 65, 64]), true), Some(Shape::Slab));
        // A step down is the other column's.
        assert_eq!(shape_at(64, n([63, 64, 64, 64]), true), None);
        assert_eq!(shape_at(64, [None, Some(65), None, None], true), Some(Shape::Slab));
    }

    #[test]
    fn a_tile_plans_its_steps_across_its_edges() {
        let mut world = World::new(WorldMeta::default());
        let dim = &mut world.dims[0];
        // Two tiles side by side: the east one a block higher.
        let a = TilePos::new(0, 0);
        let b = TilePos::new(1, 0);
        dim.ground_mut().insert(a, Tile::ground(64.0, 1, crate::NO_FLUID));
        dim.ground_mut().insert(b, Tile::ground(65.0, 1, crate::NO_FLUID));
        let mut hist = History::new(10);
        hist.begin("slabs", 0);
        fill_slabs(&mut hist.edit(&mut world, 0), value(None, false));
        hist.commit();
        let dim = &world.dims[0];
        let plan = plan_tile(dim, a);
        // The east column of tile a, at the foot of tile b.
        assert_eq!(plan.len(), TILE_SIZE as usize);
        assert!(plan.iter().all(|p| p.cell as usize % TILE_SIZE as usize == TILE_SIZE as usize - 1 && p.y == 64 && p.shape == Shape::Slab));
        assert!(plan_tile(dim, b).is_empty(), "the top of the step has nothing");
    }
}
