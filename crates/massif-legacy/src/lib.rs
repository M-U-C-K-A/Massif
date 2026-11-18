//! Import of WorldPainter `.world` files into Massif worlds.
//!
//! [`javaser`] reads Java serialization streams without a JVM;
//! [`worldpainter`] turns the `World2` object graph into a Massif world.

pub mod javaser;
pub mod layer_file;
pub mod layers;
mod tables;
pub mod terrains;
pub mod worldpainter;

pub use tables::{BIOMES, TERRAINS};
pub use worldpainter::{ImportError, ImportProgress, ImportReport, import_world, read_terrain_file};
