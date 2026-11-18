//! Massif's renderer: a path tracer of Minecraft worlds on the graphics
//! card, for the Render side.
//!
//! The scene is Minecraft chunks, whatever they come from: the open map's,
//! built by the exporter as the game would get them ([`map_source`]), or
//! a world's region files. Their blocks become materials (the game's
//! textures, from its jar, mods and resource packs: [`materials`]); the
//! blocks a ray can reach go into a sparse tree per chunk ([`scene`]); and
//! a compute shader traces rays through it, adding samples up while the
//! view stays still ([`gpu`]).

pub mod camera;
pub mod chunk;
pub mod gpu;
pub mod map_source;
pub mod materials;
pub mod models;
pub mod scene;
pub mod sky;

pub use camera::{Camera, Light};
pub use gpu::{Memory, Renderer};
pub use map_source::{Built, Extent, MapBuilder};
pub use materials::Materials;

#[cfg(test)]
mod tests;
