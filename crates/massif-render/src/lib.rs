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
pub mod diorama;
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

/// The threads the scene builders (the Diorama's, the Render side's) do
/// their work on: a pool of their own, two cores short of the computer's.
/// Built in rayon's global pool, they held up the interface: a brush, or
/// the 2D map's tiles, waited there for a batch of theirs to end.
pub fn builder_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).clamp(1, 16);
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("massif scene builder {i}")).build().expect("a thread pool")
    })
}

#[cfg(test)]
mod tests;
