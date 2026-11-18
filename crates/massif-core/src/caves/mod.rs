//! Caves, as a layer painted on the map and carved as the columns are
//! composed, the way Minecraft carves them since 1.18.
//!
//! A cave layer ([`CaveDef`]) says what its caves are: large chambers with
//! pillars standing in them (the game's "cheese" caves), long winding
//! tunnels ("spaghetti"), thin ones ("noodles") and ravines, each with its
//! density, size and the heights it keeps to, a roof of rock kept under the
//! ground's surface save where caves open to the sky, and aquifers — water
//! and lava lakes at their own levels, sealed from each other by walls of
//! rock as the game seals them. The dimension holds its layers
//! ([`CaveLayers`]); the ground's tiles hold, per column, the layer painted
//! there (see [`crate::tile::Tile::caves`]).
//!
//! Nothing is stored per block: [`TileStack`](crate::compose::TileStack)
//! asks [`tile_caves`] for the caves of a tile — runs of air, water and
//! lava per column, worked out from the layers, the mask and the ground,
//! kept in a cache while their inputs stay the same — and cuts them out of
//! the ground before the levels over it apply. Settings changed later
//! change the caves everywhere they are painted; a cube sculpted in 3D
//! keeps the caves it was sculpted from.

mod aquifer;
pub mod noise;
mod old;
mod ravines;
mod tile;

pub use old::{CaveSettings, Carved, carve, noise3};
pub use tile::{Fill, Run, TileCaves, cache_bytes, tile_caves};

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One kind of cave of a layer: whether it is carved, how much of the rock
/// it takes, how large, and between which heights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub on: bool,
    /// 0 to 1: the share of the rock it takes, from none to the most.
    pub density: f32,
    /// 0.5 to 2: how wide and long (1: as the game carves them).
    pub size: f32,
    /// The heights it keeps to, `low..=high`.
    pub low: i32,
    pub high: i32,
}

impl Feature {
    pub const fn new(on: bool, density: f32, size: f32, low: i32, high: i32) -> Self {
        Self { on, density, size, low, high }
    }

    /// Whether it carves anything.
    pub fn carves(&self) -> bool {
        self.on && self.density > 0.0 && self.high > self.low
    }
}

/// Underground water and lava: the rock is cut into zones some tens of
/// blocks across, each dry or holding water (deep down, lava) up to a level
/// of its own; where zones of different levels meet, a wall of rock stays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aquifers {
    pub on: bool,
    /// The share of zones holding water or lava, 0 to 1.
    pub density: f32,
    /// How large a zone is, 0.5 to 2 (1: 48 blocks across, 24 high).
    pub size: f32,
    /// The heights zones may hold fluid between.
    pub low: i32,
    pub high: i32,
    /// The share of the zones holding fluid under `lava_under` that hold
    /// lava rather than water, 0 to 1.
    pub lava: f32,
    pub lava_under: i32,
    /// Caves under this height are full of lava, whatever their zone (the
    /// game's lava under y −54); `None`: no such floor.
    pub lava_floor: Option<i32>,
}

/// A layer of caves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaveDef {
    pub name: String,
    /// Its colour in the lists, and over the map while it is painted.
    pub color: [u8; 3],
    pub seed: u32,
    /// Large chambers with pillars of rock standing in them.
    pub cheese: Feature,
    /// Long winding tunnels.
    pub spaghetti: Feature,
    /// Thin tunnels, many of them.
    pub noodles: Feature,
    /// Long, deep, narrow cuts.
    pub ravines: Feature,
    /// Blocks of rock kept over the caves, under the ground's surface,
    /// save where they open to the sky.
    pub roof: u8,
    /// The share of the land where caves may open to the sky, 0 to 1.
    pub openings: f32,
    pub aquifers: Aquifers,
    /// Removed: no longer offered, and no cave where it was painted.
    #[serde(default)]
    pub removed: bool,
    /// Locked: brushes leave where it is painted as it is.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub locked: bool,
    /// Its place among the cave layers in the Layers panel.
    #[serde(default, skip_serializing_if = "crate::registry::is_zero")]
    pub order: u32,
}

impl CaveDef {
    /// Caves as the game carves them, for a dimension from `min_y` to
    /// `max_y` whose sea is at `sea`.
    pub fn vanilla(name: &str, seed: u32, min_y: i32, max_y: i32, sea: i32) -> Self {
        let deep = min_y + 8;
        let top = max_y - 1;
        Self {
            name: name.to_owned(),
            color: [196, 120, 60],
            seed,
            cheese: Feature::new(true, 0.5, 1.0, deep, (sea - 8).max(deep + 16)),
            spaghetti: Feature::new(true, 0.5, 1.0, deep, top),
            noodles: Feature::new(true, 0.5, 1.0, min_y + 4, top),
            ravines: Feature::new(true, 0.3, 1.0, deep, (sea - 4).max(deep + 16)),
            roof: 6,
            openings: 0.15,
            aquifers: Aquifers {
                on: true,
                density: 0.35,
                size: 1.0,
                low: min_y,
                high: sea - 6,
                lava: 0.35,
                lava_under: min_y + 64,
                lava_floor: Some(min_y + 10),
            },
            removed: false,
            locked: false,
            order: 0,
        }
    }

    /// Whether it carves anything at all.
    pub fn carves(&self) -> bool {
        !self.removed && [&self.cheese, &self.spaghetti, &self.noodles, &self.ravines].iter().any(|f| f.carves())
    }

    /// The lowest and highest heights its caves reach.
    pub fn span(&self) -> (i32, i32) {
        let on = [&self.cheese, &self.spaghetti, &self.noodles, &self.ravines].into_iter().filter(|f| f.carves());
        on.fold((i32::MAX, i32::MIN), |(lo, hi), f| (lo.min(f.low), hi.max(f.high)))
    }
}

/// The cave layers of a dimension, numbered from 1 (0 in a tile: none).
/// Shared: a dimension cloned for a job keeps the same.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaveLayers(Arc<Vec<CaveDef>>);

impl CaveLayers {
    pub fn new(defs: Vec<CaveDef>) -> Self {
        Self(Arc::new(defs))
    }

    /// The layer numbered `n` in a tile's caves plane; `None` for 0, for a
    /// number past the list, and for a removed layer.
    pub fn get(&self, n: u16) -> Option<&CaveDef> {
        self.0.get((n as usize).checked_sub(1)?).filter(|d| !d.removed)
    }

    pub fn defs(&self) -> &[CaveDef] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|d| d.removed)
    }

    /// The list, to change: the layers numbered as before (a removed one
    /// keeps its place).
    pub fn to_vec(&self) -> Vec<CaveDef> {
        self.0.as_ref().clone()
    }

    /// The layers as JSON, as the file keeps them.
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self.0.as_ref()).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice::<Vec<CaveDef>>(bytes).map(Self::new).map_err(|e| format!("the cave layers cannot be read: {e}"))
    }

    /// The same list (not merely equal ones).
    pub fn same(&self, other: &CaveLayers) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
