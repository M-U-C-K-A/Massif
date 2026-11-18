//! The Map Output: what the graph gives the map, as Blender's Material
//! Output takes what a material gives a surface.

use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&MAP];

const fn input(name: &'static str, label: &'static str, kind: PortKind, help: &'static str, default: &'static str) -> PortSpec {
    PortSpec::new(name, label, kind, help).or(default)
}

const LAYER_TOP: &str = "The top of a layer of rock over the ground: floating islands, overhangs, arches.";
const LAYER_BOTTOM: &str = "The underside of that layer: where it is above the ground, the void lies between.";

pub static MAP: NodeSpec = NodeSpec {
    id: "output.map",
    label: "Map output",
    family: Family::Output,
    summary: "What the graph gives the map: its ground, water, surface, climate, life, underground and masks, each input optional.",
    help: "The graph's final node, like a material output in Blender: one node with an input for each thing the map can take. Connect only what the graph makes; an input left empty keeps what the map has (or takes the default it shows). A graph can keep several Map Outputs to compare variants: the one marked Active is used.",
    keywords: &["output", "result", "final", "material", "map", "ground", "terrain"],
    inputs: &[
        input("height", "Height", PortKind::Height, "The ground's height, in blocks.", "the map's ground as it is"),
        input("water", "Water", PortKind::Height, "The surface of rivers and lakes, in blocks: where it lies above the ground, water fills up to it.", "none: the world's sea only"),
        input("albedo", "Albedo", PortKind::Color, "The colour of the ground: the blocks nearest to it are chosen when Terrain is left empty.", "none"),
        input("terrain", "Terrain", PortKind::Terrain, "The terrain (the block material) of each column.", "from Albedo when it is given, else the map's terrains"),
        input("biome", "Biome", PortKind::Biome, "The game's biome of each column.", "from Temperature and Humidity when given, else the map's biomes"),
        input("temperature", "Temperature", PortKind::Mask, "The climate's temperature: 0 is −30 °C, 1 is +45 °C.", "none"),
        input("humidity", "Humidity", PortKind::Mask, "How wet the climate is, from 0 to 1.", "none"),
        input("vegetation", "Vegetation", PortKind::Mask, "How densely trees and plants grow, from 0 to 1.", "none: the graph places no plants"),
        input("caves", "Caves", PortKind::Mask, "Where caves form, and how many, from 0 to 1.", "the map's cave settings"),
        input("layer.1.top", "Layer 1 top", PortKind::Height, LAYER_TOP, "none"),
        input("layer.1.bottom", "Layer 1 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("layer.2.top", "Layer 2 top", PortKind::Height, LAYER_TOP, "none"),
        input("layer.2.bottom", "Layer 2 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("mask.flow", "Flow", PortKind::Mask, "Where water runs, written to the map as the mask “flow”.", "none"),
        input("mask.wear", "Wear", PortKind::Mask, "Where the ground was worn away, written as the mask “wear”.", "none"),
        input("mask.deposition", "Deposition", PortKind::Mask, "Where sediment settled, written as the mask “deposition”.", "none"),
        input("mask.snow", "Snow", PortKind::Mask, "Where snow lies, written as the mask “snow”.", "none"),
    ],
    open_inputs: Some(input("mask.", "Mask", PortKind::Mask, "Any other mask, written to the map under its name (mask.moss writes “moss”).", "none")),
    outputs: &[],
    params: &[ParamSpec::flag("active", "Active", true, "Whether the map takes this output, when the graph has several.")],
    kernel: Kernel::Output,
    ..NodeSpec::BASE
};
