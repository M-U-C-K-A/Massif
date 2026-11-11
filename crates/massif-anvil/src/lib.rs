//! Minecraft Java Edition export: NBT, Anvil region files and `level.dat`.
//!
//! The exporter composes every column of the map from the level stack (see
//! `massif_core::compose`), turns the solid spans into blocks with the
//! terrain recipes, and writes one region file per 512×512 blocks. Regions
//! are built in parallel.

pub mod chunk;
pub mod export;
pub mod level_dat;
pub mod nbt;
pub mod region;

pub use chunk::{DATA_VERSION, DATA_VERSION_NAME};
pub use export::{
    ExportError, ExportOptions, ExportStats, Progress, default_saves_dir, export, unique_dir,
};
pub use level_dat::{Difficulty, GameMode, Surroundings, WorldBorder};
