//! Underground pockets, as WorldPainter's layer of that name: blobs of
//! other blocks in the rock — gravel, dirt, clay, granite, a mod's stone —
//! where a pocket layer is painted. A block of the rock under the ground's
//! strata, between the layer's heights, is in a pocket where a smooth 3D
//! noise rises over a level, set so that the pockets take the layer's share
//! of the rock. [`Pockets`] asks it the same way for the export, the
//! survival check and the cut of the Ores window, so that they agree block
//! for block.

use crate::compose::{Materializer, Span, hash2};
use crate::registry::{BlockId, Registry};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The bedrock floor of the world: pockets stay over its five layers.
const FLOOR: i32 = 5;

/// A layer of underground pockets. A tile's pockets plane holds `0` for
/// none, or a layer's number: its index in
/// [`crate::registry::Registry::pockets`] plus one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PocketDef {
    pub name: String,
    /// Its colour in the lists, and over the map while it is painted.
    pub color: [u8; 3],
    /// What the pockets are made of, with weights: one is picked for each
    /// block. Blocks of the registry, as a terrain's.
    pub blocks: Vec<(BlockId, u16)>,
    /// The share of the rock the pockets take where the layer is painted,
    /// 0 to 1.
    pub share: f32,
    /// About how many blocks across a pocket is.
    pub size: f32,
    /// The heights the pockets keep to, `low..=high`.
    pub low: i32,
    pub high: i32,
    /// Removed: no longer offered, and no pocket where it was painted.
    #[serde(default)]
    pub removed: bool,
    /// Locked: brushes leave its blocks as they are.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub locked: bool,
    /// Its place among the pocket layers in the Layers panel.
    #[serde(default, skip_serializing_if = "crate::registry::is_zero")]
    pub order: u32,
}

impl PocketDef {
    /// Pockets of `blocks`, taking `share` of the rock, about `size`
    /// blocks across, from `low` to `high`.
    pub fn new(name: &str, color: [u8; 3], blocks: &[(BlockId, u16)], share: f32, size: f32, low: i32, high: i32) -> Self {
        Self {
            name: name.to_owned(),
            color,
            blocks: blocks.to_vec(),
            share,
            size,
            low,
            high,
            removed: false,
            locked: false,
            order: 0,
        }
    }

    /// The level the noise must reach for a block to be in a pocket, so
    /// that the pockets take their share of the rock.
    pub fn threshold(&self) -> f32 {
        let levels = levels();
        let k = ((1.0 - self.share.clamp(0.0, 1.0)) * (levels.len() - 1) as f32).round() as usize;
        levels[k.min(levels.len() - 1)]
    }

    /// The block (an index into [`PocketDef::blocks`]) at `(x, y, z)` in a
    /// world of seed `seed`, where the rock is in a pocket of this layer;
    /// `threshold` is [`PocketDef::threshold`], asked once.
    pub fn block_at(&self, x: i32, y: i32, z: i32, seed: u64, threshold: f32) -> Option<usize> {
        if self.removed || self.blocks.is_empty() || self.share <= 0.0 || y < self.low || y > self.high {
            return None;
        }
        if self.share < 1.0 {
            let s = self.size.max(1.0);
            if crate::caves::noise3(seed ^ 0x706f_636b_6574, x as f32 / s, y as f32 / s, z as f32 / s) < threshold {
                return None;
            }
        }
        let total: u32 = self.blocks.iter().map(|b| b.1 as u32).sum();
        if total == 0 {
            return None;
        }
        let mut r = hash2(x ^ y.wrapping_mul(73_856_093), z, seed ^ 0x0070_6f63_6b65_7432) % total;
        self.blocks.iter().position(|b| {
            if r < b.1 as u32 {
                true
            } else {
                r -= b.1 as u32;
                false
            }
        })
    }
}

/// The pocket layers of a world, ready to be asked column by column: each
/// layer's level learnt once.
pub struct Pockets<'a> {
    reg: &'a Registry,
    levels: Vec<f32>,
    seed: u64,
    min_y: i32,
}

impl<'a> Pockets<'a> {
    /// The pockets of `reg` in a world of seed `seed` whose lowest height
    /// is `min_y`; `None` when it has no pocket layer.
    pub fn new(reg: &'a Registry, seed: u64, min_y: i32) -> Option<Self> {
        reg.pockets.iter().any(|p| !p.removed).then(|| Self { reg, levels: reg.pockets.iter().map(PocketDef::threshold).collect(), seed, min_y })
    }

    /// The pockets of the column at `(x, z)`, whose pocket layer (in its
    /// tile's pockets plane) is `n` and whose solid runs are `spans`: in
    /// the rock of its lowest run, under its terrain's layers, over the
    /// bedrock floor, between the layer's heights. `None` where it has
    /// none, or its lowest run does not start at the floor of the world.
    pub fn column(&self, n: u16, spans: &[Span], mat: &Materializer, x: i32, z: i32) -> Option<Column<'a>> {
        if n == 0 {
            return None;
        }
        let def = self.reg.pocket(n)?;
        let first = spans.first().filter(|s| s.y0 <= self.min_y)?;
        let recipe = mat.recipe_depth(first.terrain, x, z);
        let split = if recipe >= (first.y1 - first.y0) as u32 { first.y0 } else { first.y1 - recipe as i32 };
        let (low, high) = (def.low.max(self.min_y + FLOOR), def.high.min(split - 1));
        (low <= high).then(|| Column { def, level: self.levels[n as usize - 1], seed: self.seed, low, high })
    }
}

/// The pockets of one column (see [`Pockets::column`]).
pub struct Column<'a> {
    pub def: &'a PocketDef,
    level: f32,
    seed: u64,
    /// The heights they may take, `low..=high`.
    pub low: i32,
    pub high: i32,
}

impl Column<'_> {
    /// The block of a pocket at `(x, y, z)` of the column, if one is there.
    pub fn at(&self, x: i32, y: i32, z: i32) -> Option<BlockId> {
        if y < self.low || y > self.high {
            return None;
        }
        self.def.block_at(x, y, z, self.seed, self.level).map(|k| self.def.blocks[k].0)
    }
}

/// The levels of the noise at every thousandth of its spread, lowest first,
/// learnt once from many samples of it: the level a share of the blocks
/// rises over.
fn levels() -> &'static [f32] {
    static LEVELS: OnceLock<Vec<f32>> = OnceLock::new();
    LEVELS.get_or_init(|| {
        const N: usize = 200_000;
        let mut samples: Vec<f32> = (0..N)
            .map(|i| {
                let h = |k: u64| hash2(i as i32, k as i32, 0x6c65_7665_6c73) as f32 / u32::MAX as f32;
                crate::caves::noise3(0x6c65_7665_6c73, h(1) * 997.0, h(2) * 997.0, h(3) * 997.0)
            })
            .collect();
        samples.sort_by(f32::total_cmp);
        (0..=1000).map(|k| samples[(k * (N - 1)) / 1000]).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pockets take their share of the rock, in blobs of about their
    /// size, between their heights only; the same blocks every time.
    #[test]
    fn pockets_take_their_share() {
        let (gravel, andesite) = (BlockId(7), BlockId(9));
        for share in [0.05f32, 0.2, 0.5] {
            let def = PocketDef::new("Gravel", [0; 3], &[(gravel, 3), (andesite, 1)], share, 6.0, -20, 40);
            let t = def.threshold();
            let mut inside = 0;
            let mut n = 0;
            for x in 0..60 {
                for y in -20..=40 {
                    for z in 0..60 {
                        n += 1;
                        inside += def.block_at(x, y, z, 7, t).is_some() as usize;
                    }
                }
            }
            let got = inside as f32 / n as f32;
            assert!((got - share).abs() < share * 0.25 + 0.01, "share {share}: {got}");
        }
        let def = PocketDef::new("Clay", [0; 3], &[(BlockId(3), 1)], 1.0, 4.0, 0, 10);
        let t = def.threshold();
        assert!((0..=10).all(|y| def.block_at(3, y, 3, 1, t) == Some(0)), "all of the rock at a share of 1");
        assert_eq!((def.block_at(3, -1, 3, 1, t), def.block_at(3, 11, 3, 1, t)), (None, None), "only between its heights");
        let half = PocketDef::new("Mixed", [0; 3], &[(gravel, 3), (andesite, 1)], 0.3, 5.0, 0, 64);
        let t = half.threshold();
        let blocks: Vec<Option<usize>> = (0..2000).map(|i| half.block_at(i % 50, i / 50, 7, 3, t)).collect();
        assert_eq!(blocks, (0..2000).map(|i| half.block_at(i % 50, i / 50, 7, 3, t)).collect::<Vec<_>>());
        let gravel = blocks.iter().filter(|b| **b == Some(0)).count() as f32;
        let andesite = blocks.iter().filter(|b| **b == Some(1)).count() as f32;
        assert!((gravel / andesite - 3.0).abs() < 0.8, "{gravel} gravel, {andesite} andesite");
    }

    /// Smaller pockets for a smaller size: more edges between rock and
    /// pocket along a line.
    #[test]
    fn size_sets_how_big_pockets_are() {
        let edges = |size: f32| {
            let def = PocketDef::new("P", [0; 3], &[(BlockId(7), 1)], 0.3, size, 0, 64);
            let t = def.threshold();
            (0..2000).filter(|&x| def.block_at(x, 30, 5, 9, t).is_some() != def.block_at(x + 1, 30, 5, 9, t).is_some()).count()
        };
        let (small, big) = (edges(3.0), edges(12.0));
        assert!(small > big * 2, "{small} edges at size 3, {big} at 12");
    }

    /// A column's pockets keep to the rock of its lowest run: under the
    /// terrain's layers, over the bedrock floor, between the layer's
    /// heights, and only where the run starts at the floor of the world.
    #[test]
    fn a_column_keeps_its_pockets_in_the_rock() {
        let mut reg = Registry::vanilla();
        let clay = reg.intern_block("minecraft:clay", [160, 166, 179]);
        let n = reg.add_pocket(PocketDef::new("Clay", [0; 3], &[(clay, 1)], 1.0, 4.0, -100, 200));
        let terrain = reg.terrain_by_name("Grass").unwrap();
        let mat = Materializer::new(&reg, 5, -64);
        let pockets = Pockets::new(&reg, 5, -64).expect("a pocket layer");
        let run = Span { y0: -64, y1: 70, terrain, top: 70.0 };
        let col = pockets.column(n, &[run], &mat, 3, 4).expect("pockets in the column");
        let recipe = mat.recipe_depth(terrain, 3, 4) as i32;
        assert_eq!((col.low, col.high), (-59, 70 - recipe - 1), "over the floor, under the layers");
        assert!((col.low..=col.high).all(|y| col.at(3, y, 4) == Some(clay)), "all of it at a share of 1");
        assert_eq!((col.at(3, col.low - 1, 4), col.at(3, col.high + 1, 4)), (None, None));
        assert!(pockets.column(0, &[run], &mat, 3, 4).is_none(), "not painted");
        assert!(pockets.column(n, &[Span { y0: 20, ..run }], &mat, 3, 4).is_none(), "a floating island has none");
        let mut gone = reg.clone();
        gone.pockets[0].removed = true;
        assert!(Pockets::new(&gone, 5, -64).is_none(), "no layer left");
    }
}
