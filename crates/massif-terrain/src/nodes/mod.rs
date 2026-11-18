//! The node set: one module per family.

mod adjust;
mod color;
mod combine;
mod convert;
mod masks;
mod output;
mod primitives;
mod simulate;

use crate::registry::NodeSpec;

/// The WGSL every kernel starts with: the noise, the tile size.
pub const WGSL_COMMON: &str = include_str!("wgsl/common.wgsl");

/// Every type of node, in the library's order.
pub static ALL: &[&NodeSpec] = &[
    &primitives::CONSTANT,
    &primitives::FBM,
    &adjust::LEVELS,
    &adjust::NORMALIZE,
    &adjust::BLUR,
    &simulate::THERMAL,
    &masks::SLOPE,
    &combine::BLEND,
    &color::RAMP,
    &output::MAP,
    &convert::LUMINANCE,
    &convert::GREY,
];

/// The common words of a node's height output.
pub(crate) const fn height_out(help: &'static str) -> crate::registry::PortSpec {
    crate::registry::PortSpec::new("height", "Height", crate::registry::PortKind::Height, help)
}
