//! Import of WorldPainter `.world` files into Massif worlds.
//!
//! [`javaser`] reads Java serialization streams without a JVM;
//! [`worldpainter`] turns the `World2` object graph into a Massif world.

pub mod javaser;
mod tables;
pub mod terrains;
pub mod worldpainter;

pub use worldpainter::{ImportError, ImportProgress, ImportReport, import_world};
