//! Import of WorldPainter `.world` files into Massif worlds.
//!
//! [`javaser`] reads Java serialization streams without a JVM;
//! [`worldpainter`] turns the `World2` object graph into a Massif world;
//! [`javawrite`] and [`wp_export`] write a Massif world back as a `.world`.

pub mod javaser;
pub mod javawrite;
pub mod layer_file;
pub mod layers;
mod tables;
pub mod terrains;
pub mod worldpainter;
pub mod wp_classes;
pub mod wp_export;

pub use tables::{BIOMES, TERRAINS};
pub use worldpainter::{ImportError, ImportProgress, ImportReport, import_world, read_terrain_file};
pub use wp_export::{WpExportError, WpOptions, WpProgress, WpReport, export_world};
