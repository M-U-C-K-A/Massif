//! How many objects WorldPainter stands where a custom object layer is
//! painted, so that an imported layer stands as many.
//!
//! WorldPainter's exporter (`Bo2LayerExporter`) tries every column of the
//! layer's grid: it draws a number under 64 × the layer's density and
//! places an object where it is at most the square of the strength painted
//! there (0 to 15), when the object has room — none of its substantial
//! blocks over the ground may take the place of one already there, another
//! object's leaves included. How dense a forest ends up is thus set by the
//! trees' own shapes as much as by the density: trees of sparse branches
//! interleave and stand far closer than their boxes. Massif stands at most
//! one object in each square of a layer's spacing, so the import plays
//! WorldPainter's placement on a patch of flat land, its objects as they
//! are, and gives the layer the spacing and density that stand as many, at
//! each strength.

use massif_core::compose::hash2;
use massif_core::objects::{ObjectDef, is_nothing, turn_xz};

/// An object's substantial blocks over the ground, with its frequency.
type Shape = (Vec<(i32, i32, i32)>, u32);

/// The side of the patch of land played, wrapped around at its edges.
const SIDE: i32 = 128;

/// What WorldPainter stands of a layer: at each strength painted (0 to
/// 15), objects a column; and how many of each object at full strength.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    pub per_column: [f32; 16],
    pub counts: Vec<u32>,
}

/// WorldPainter's settings of a custom object layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Its density: one in 64 × this, at strength 1.
    pub density: u32,
    pub grid: [u32; 2],
    /// Objects turned and mirrored at random.
    pub rotate: bool,
}

/// Whether a block is one WorldPainter's objects pass through (its
/// "very insubstantial" materials): nothing, plants, snow, vines.
pub fn insubstantial(state: &str) -> bool {
    if is_nothing(state) || massif_core::registry::plant_color(state).is_some() {
        return true;
    }
    let name = state.split('[').next().unwrap_or(state);
    ["vine", "lichen", "hanging_roots", "torch", "cobweb", "water", "lava", "light", "fire", "carpet", "petals", "leaf_litter"].iter().any(|w| name.contains(w))
}

/// The substantial blocks of an object over the ground, from its anchor:
/// `(dx, dy, dz)` with `dy >= 0`.
fn solid_blocks(def: &ObjectDef) -> Vec<(i32, i32, i32)> {
    let [w, h, l] = def.size.map(|s| s as usize);
    let [ax, ay, az] = def.anchor.map(|a| a as i32);
    let solid: Vec<bool> = def.palette.iter().map(|s| !insubstantial(s)).collect();
    let mut out = Vec::new();
    for y in ay as usize..h {
        for z in 0..l {
            for x in 0..w {
                let b = def.blocks[x + z * w + y * w * l] as usize;
                if solid.get(b).copied().unwrap_or(false) {
                    out.push((x as i32 - ax, y as i32 - ay, z as i32 - az));
                }
            }
        }
    }
    out
}

/// A run of WorldPainter's placement at strength `strength` over the patch:
/// the objects stood, by object.
fn play(objects: &[Shape], height: i32, settings: Settings, strength: u32, salt: u64) -> Vec<u32> {
    let n = SIDE as usize;
    let mut taken = vec![0u64; (n * n * height as usize).div_ceil(64)];
    let at = |x: i32, y: i32, z: i32| (y as usize * n + z.rem_euclid(SIDE) as usize) * n + x.rem_euclid(SIDE) as usize;
    let total: u64 = objects.iter().map(|o| o.1 as u64).sum();
    let mut counts = vec![0u32; objects.len()];
    if total == 0 {
        return counts;
    }
    let draws = 64 * settings.density.max(1) as u64;
    let [gx, gz] = settings.grid.map(|g| g.max(1) as i32);
    let mut k = 0u64;
    let mut rand = |x: i32, z: i32| {
        k += 1;
        hash2(x, z, salt ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15)) as u64
    };
    // Chunk by chunk, column by column, as WorldPainter goes.
    for cz in (0..SIDE).step_by(16) {
        for cx in (0..SIDE).step_by(16) {
            for x in cx..cx + 16 {
                for z in cz..cz + 16 {
                    if x % gx != 0 || z % gz != 0 || rand(x, z) % draws > (strength * strength) as u64 {
                        continue;
                    }
                    let mut r = rand(x, z) % total;
                    let Some(pick) = objects.iter().position(|o| {
                        if r < o.1 as u64 {
                            true
                        } else {
                            r -= o.1 as u64;
                            false
                        }
                    }) else {
                        continue;
                    };
                    let (turn, mirror) = if settings.rotate { ((rand(x, z) % 4) as u8, rand(x, z) & 1 == 1) } else { (0, false) };
                    let place = |&(dx, dy, dz): &(i32, i32, i32)| {
                        let (dx, dz) = turn_xz(if mirror { -dx } else { dx }, dz, turn);
                        at(x + dx, dy, z + dz)
                    };
                    let blocks = &objects[pick].0;
                    if blocks.iter().map(place).any(|i| taken[i / 64] >> (i % 64) & 1 == 1) {
                        continue;
                    }
                    for i in blocks.iter().map(place) {
                        taken[i / 64] |= 1 << (i % 64);
                    }
                    counts[pick] += 1;
                }
            }
        }
    }
    counts
}

/// What WorldPainter stands of a layer of `objects` (each with its
/// frequency) with these settings, on flat land.
pub fn coverage(objects: &[(&ObjectDef, u32)], settings: Settings) -> Coverage {
    let shapes: Vec<Shape> = objects.iter().map(|(d, f)| (solid_blocks(d), *f)).collect();
    let height = shapes.iter().flat_map(|s| s.0.iter().map(|b| b.1 + 1)).max().unwrap_or(1).max(1);
    let area = (SIDE * SIDE) as f32;
    let mut per_column = [0.0f32; 16];
    let mut counts = vec![0u32; objects.len()];
    // Played at a few strengths, the others between them.
    const PLAYED: [u32; 6] = [1, 3, 6, 9, 12, 15];
    let runs: Vec<Vec<u32>> = PLAYED.iter().map(|&s| play(&shapes, height, settings, s, 0x7770_6f62 ^ s as u64)).collect();
    for (k, &s) in PLAYED.iter().enumerate() {
        per_column[s as usize] = runs[k].iter().sum::<u32>() as f32 / area;
    }
    for s in 1..16usize {
        if PLAYED.contains(&(s as u32)) {
            continue;
        }
        let lo = *PLAYED.iter().rev().find(|&&p| p < s as u32).unwrap() as usize;
        let hi = *PLAYED.iter().find(|&&p| p > s as u32).unwrap() as usize;
        let t = (s - lo) as f32 / (hi - lo) as f32;
        per_column[s] = per_column[lo] + (per_column[hi] - per_column[lo]) * t;
    }
    for (c, n) in counts.iter_mut().zip(&runs[PLAYED.len() - 1]) {
        *c = *n;
    }
    Coverage { per_column, counts }
}

/// The spacing and density of a Massif layer that stands `per_column`
/// objects a column, its spacing at most `spacing`: the widest square that
/// holds no more than one.
pub fn spacing_for(per_column: f32, spacing: u16) -> (u16, f32) {
    if per_column <= 0.0 {
        return (spacing, 0.0);
    }
    let s = if per_column * (spacing as f32).powi(2) <= 1.0 { spacing } else { (1.0 / per_column.sqrt()).floor().max(1.0) as u16 };
    (s, (per_column * (s as f32).powi(2)).clamp(0.0, 1.0))
}

/// A tree of a trunk and a round crown of leaves, `r` across from its
/// trunk: for tests.
#[cfg(test)]
pub(crate) fn tree(r: i32, trunk: i32) -> ObjectDef {
    let side = (2 * r + 1) as usize;
    let h = (trunk + r + 1) as usize;
    let mut blocks = vec![0u16; side * side * h];
    for y in 0..h as i32 {
        for z in -r..=r {
            for x in -r..=r {
                let i = (x + r) as usize + (z + r) as usize * side + y as usize * side * side;
                if x == 0 && z == 0 && y < trunk {
                    blocks[i] = 1;
                } else if y >= trunk - 1 && x * x + z * z + (y - trunk) * (y - trunk) <= r * r {
                    blocks[i] = 2;
                }
            }
        }
    }
    let mut def = ObjectDef::new("tree", [side as u16, h as u16, side as u16], vec!["minecraft:air".into(), "minecraft:oak_log".into(), "minecraft:oak_leaves".into()], blocks);
    def.anchor = [r as u16, 0, r as u16];
    def
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Objects of one block hardly ever meet: as many stand as are drawn,
    /// 226 in 64 × the density at full strength, and the square of the
    /// strength fewer at less.
    #[test]
    fn single_blocks_stand_as_often_as_drawn() {
        let mut stone = ObjectDef::new("stone", [1, 1, 1], vec!["minecraft:air".into(), "minecraft:stone".into()], vec![1]);
        stone.anchor = [0, 0, 0];
        let c = coverage(&[(&stone, 1)], Settings { density: 20, grid: [1, 1], rotate: false });
        let expected = 226.0 / 1280.0;
        assert!((c.per_column[15] / expected - 1.0).abs() < 0.08, "{} vs {expected}", c.per_column[15]);
        assert!((c.per_column[6] / (37.0 / 1280.0) - 1.0).abs() < 0.25, "{:?}", c.per_column);
        assert!(c.per_column.windows(2).all(|w| w[0] <= w[1] + 0.01), "{:?}", c.per_column);
    }

    /// Trees stand as close as their crowns let them: far fewer than are
    /// drawn, but more than one a box (crowns are round).
    #[test]
    fn trees_stand_as_close_as_their_crowns_let_them() {
        let oak = tree(3, 5);
        let c = coverage(&[(&oak, 1)], Settings { density: 5, grid: [1, 1], rotate: true });
        let boxes = 1.0 / 49.0;
        assert!(c.per_column[15] < 226.0 / 320.0 / 4.0, "{:?}", c.per_column);
        assert!(c.per_column[15] > boxes, "{} a column, a box is {boxes}", c.per_column[15]);
        // A grid of 4 leaves a sixteenth of the columns to try.
        let sparse = coverage(&[(&oak, 1)], Settings { density: 20, grid: [4, 4], rotate: true });
        assert!(sparse.per_column[15] < c.per_column[15], "{:?}", sparse.per_column);
    }

    #[test]
    fn spacing_holds_one_a_square() {
        assert_eq!(spacing_for(0.001, 16), (16, 0.256));
        let (s, d) = spacing_for(0.05, 16);
        assert_eq!(s, 4);
        assert!((d - 0.8).abs() < 1e-6);
        assert_eq!(spacing_for(0.0, 9), (9, 0.0));
    }
}
