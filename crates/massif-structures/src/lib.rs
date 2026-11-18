//! Structures for Massif.
//!
//! The game builds its villages, Ancient Cities, Trial Chambers and
//! bastions from template pools: a first piece, then pieces joined at their
//! jigsaw blocks, drawn from the pools those name, turned and fitted so that
//! none overlaps another ([`jigsaw`]). This crate does the same with the
//! game's own files, read from the player's jar and their mods' jars
//! ([`library`]), and lays the blocks the way the game does: processors,
//! block entities (chests and their loot tables, spawners), entities
//! (villagers), the features among them (trees, sculk) — [`build`]. The
//! game's other structures, built by its code (a desert temple, a
//! stronghold), are built by Massif's own ([`vanilla`]).
//!
//! What comes out is a [`massif_core::structures::Structure`], kept with
//! the map and written by the export.

pub mod build;
pub mod code;
pub mod features;
pub mod geom;
pub mod ground;
pub mod jigsaw;
pub mod library;
pub mod nbt;
pub mod pool;
pub mod processor;
pub mod rng;
pub mod source;
pub mod template;
#[doc(hidden)]
pub mod testing;
pub mod vanilla;
pub mod village;

pub use library::Library;
