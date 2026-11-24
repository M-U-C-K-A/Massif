//! Massif's terrain generator.
//!
//! A terrain graph ([`model::Graph`]) is a readable JSON document of named
//! nodes; [`registry`] says what every type of node takes and makes, in
//! words for people and models. [`compile::compile`] checks a graph and
//! orders it, [`engine::Engine`] evaluates it by tiles of [`TILE`]² samples
//! at any level of detail ([`space`]), on the graphics card where a node
//! has a kernel there ([`gpu`]) and on every core otherwise, caching each
//! tile under the hash of what made it.
//!
//! ```
//! use massif_terrain::{compile, engine, model::Graph, space::{Rect, Space}};
//! let graph = Graph::from_json(r#"{ "nodes": {
//!     "hills": { "type": "noise.fbm", "params": { "scale": 500, "height": 120 } },
//!     "out": { "type": "output.map", "inputs": { "height": "hills" } } } }"#).unwrap();
//! let (c, diags) = compile::compile(&graph, Space::new(2048), 1024);
//! assert!(diags.is_empty());
//! let e = engine::Engine::new(Default::default());
//! let out = c.output("height").unwrap();
//! let preview = e.region(&c, out, 3, Rect::new(0, 0, 256, 256), &engine::never).unwrap();
//! assert_eq!(preview.data.len(), 256 * 256);
//! ```

pub mod compile;
pub mod components;
pub mod engine;
pub mod files;
pub mod filters;
pub mod format;
pub mod geom;
pub mod gpu;
pub mod group;
pub mod kernel;
pub mod live;
pub mod lower;
pub mod migrate;
pub mod model;
pub mod noise;
mod nodes;
pub mod params;
pub mod preview;
pub mod registry;
pub mod space;
pub mod underground;
pub mod util;
pub mod world;

pub use space::TILE;
