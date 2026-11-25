//! The Terrain side's node panel of 1.2 (the Terrain Generator): a graph of `massif_terrain`
//! nodes edited on a canvas (typed, colour-coded sockets, frames, reroutes,
//! a minimap, search to add), its nodes' settings drawn from the registry,
//! one Map Output with an input for everything the map takes, and previews
//! lit as a renderer lights them (node thumbnails and a 2D/3D viewer).

#![allow(dead_code)]

#[macro_use]
pub mod text;
pub mod apply;
pub mod canvas;
pub mod edit;
pub mod eval;
pub mod library;
pub mod panel;
pub mod props;
pub mod script;
pub mod shape;
pub mod shade;
pub mod sockets;
pub mod viewer;

pub use panel::TerrainPanel;

#[cfg(test)]
pub(crate) mod tests;
