//! Massif core: the world model.
//!
//! A world is a set of *dimensions*. A dimension is an ordered stack of
//! *levels* (the "multi-level surface"): every level is a sparse grid of
//! 128×128 tiles holding, per column, a top height, an optional bottom height
//! and a few categorical planes (terrain, fluid, biome).
//!
//! * the **ground** level is the classic 2.5D heightmap (bottom = world floor);
//! * a **solid** level adds a slab `[bottom, top)` — floating islands, arches,
//!   overhangs;
//! * a **carve** level removes a slab `[bottom, top)` — caves, tunnels, pits.
//!
//! Composing the levels of one column yields a short list of solid spans,
//! which is exactly a run-length encoding of that column along Y. Nothing is
//! ever stored per voxel.
//!
//! Tiles live either *hot* (decoded, editable) or *cold* (one compressed
//! blob). The cold blob is byte-for-byte what the file format stores, so
//! saving unchanged tiles is a memcpy and opening a file decodes nothing.

pub mod auto;
pub mod brush;
pub mod codec;
pub mod compose;
pub mod coords;
pub mod filter;
pub mod generate;
pub mod graph;
pub mod heightmap;
pub mod history;
pub mod level;
pub mod noise;
pub mod objects;
pub mod ops;
pub mod pattern;
pub mod plane;
pub mod registry;
pub mod river;
pub mod stamps;
pub mod tile;
pub mod voxel;
pub mod world;

pub use brush::{Brush, BrushMask, BrushShape, MAX_RADIUS, Profile};
pub use compose::{Span, TileStack};
pub use coords::{BlockRect, TILE_AREA, TILE_SIZE, TilePos};
pub use history::{Edit, History, Others};
pub use level::{Blob, Level, LevelId, LevelKind, TileRef};
pub use plane::Plane;
pub use registry::{BiomeId, BlockId, Registry, TerrainId};
pub use tile::{NO_FLUID, Tile};
pub use world::{Dimension, World, WorldMeta};
