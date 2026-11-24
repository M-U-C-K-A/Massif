//! The node set: one module per family, each listing its node types in
//! `NODES`. A session adding nodes touches only its family's files (and
//! `erosion/` belongs to the erosion work); the list below is the only
//! shared line per family.

mod adjust;
mod climate;
mod color;
mod combine;
mod convert;
mod erosion;
mod geometry;
mod inputs;
mod legacy;
mod masks;
mod output;
mod primitives;
mod shapes;

use crate::registry::{NodeSpec, PortKind, PortSpec};
use std::sync::OnceLock;

/// The WGSL every kernel starts with: the noise, the tile size.
pub const WGSL_COMMON: &str = include_str!("wgsl/common.wgsl");

/// Every type of node, family by family in the library's order, then the
/// conversions the compiler puts on wires.
pub fn all() -> &'static [&'static NodeSpec] {
    static ALL: OnceLock<Vec<&'static NodeSpec>> = OnceLock::new();
    ALL.get_or_init(|| {
        [inputs::NODES, primitives::NODES, shapes::NODES, adjust::NODES, erosion::NODES, climate::NODES, masks::NODES, color::NODES, combine::NODES, geometry::NODES, legacy::NODES, output::NODES, convert::NODES].concat()
    })
}

/// The common words of a node's height output.
pub(crate) const fn height_out(help: &'static str) -> PortSpec {
    PortSpec::new("height", "Height", PortKind::Height, help)
}
