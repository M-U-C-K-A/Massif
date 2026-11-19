//! The Map Output: what the graph gives the map, as Blender's Material
//! Output takes what a material gives a surface.

use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&MAP];

const fn input(name: &'static str, label: &'static str, kind: PortKind, help: &'static str, default: &'static str) -> PortSpec {
    PortSpec::new(name, label, kind, help).or(default)
}

const LAYER_TOP: &str = "The top of a level of rock over the ground (a Solid level of its own): floating islands, overhangs, arches.";
const LAYER_BOTTOM: &str = "The underside of that level: where it is above the ground, the void lies between.";
const LAYER_COLOR: &str = "The colour of that level's rock: the blocks nearest to it are chosen, from the Albedo palette.";

const PALETTES: &[Choice] = &[
    Choice { value: "natural", label: "Natural", help: "Soils, stone, sand, snow: the blocks of a natural landscape." },
    Choice { value: "rock", label: "Rock", help: "Stones only." },
    Choice { value: "desert", label: "Desert", help: "Sands, sandstones, terracottas." },
    Choice { value: "snowy", label: "Snowy", help: "Snow, ice, pale stone." },
    Choice { value: "every", label: "Every block", help: "Every full block of the game." },
];

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
        input("level.1.top", "Level 1 top", PortKind::Height, LAYER_TOP, "none"),
        input("level.1.bottom", "Level 1 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("level.1.color", "Level 1 colour", PortKind::Color, LAYER_COLOR, "the colour of the rock"),
        input("level.2.top", "Level 2 top", PortKind::Height, LAYER_TOP, "none"),
        input("level.2.bottom", "Level 2 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("level.2.color", "Level 2 colour", PortKind::Color, LAYER_COLOR, "the colour of the rock"),
        input("level.3.top", "Level 3 top", PortKind::Height, LAYER_TOP, "none"),
        input("level.3.bottom", "Level 3 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("level.3.color", "Level 3 colour", PortKind::Color, LAYER_COLOR, "the colour of the rock"),
        input("level.4.top", "Level 4 top", PortKind::Height, LAYER_TOP, "none"),
        input("level.4.bottom", "Level 4 bottom", PortKind::Height, LAYER_BOTTOM, "none"),
        input("level.4.color", "Level 4 colour", PortKind::Color, LAYER_COLOR, "the colour of the rock"),
        input("mask.flow", "Flow", PortKind::Mask, "Where water runs, written to the map as the mask “flow”.", "none"),
        input("mask.wear", "Wear", PortKind::Mask, "Where the ground was worn away, written as the mask “wear”.", "none"),
        input("mask.deposition", "Deposition", PortKind::Mask, "Where sediment settled, written as the mask “deposition”.", "none"),
        input("mask.snow", "Snow", PortKind::Mask, "Where snow lies, written as the mask “snow”.", "none"),
    ],
    open_inputs: Some(input("mask.", "Mask", PortKind::Mask, "Any other mask, written to the map under its name (mask.moss writes “moss”).", "none")),
    outputs: &[],
    params: &[
        ParamSpec::flag("active", "Active", true, "Whether the map takes this output, when the graph has several."),
        ParamSpec::choice("albedo_palette", "Albedo blocks", PALETTES, "natural", "The blocks Albedo and the levels' colours turn into."),
        ParamSpec::flag("paint_biomes", "Paint biomes", false, "Paints the ground to suit the biomes: sand in the deserts, snow in the tundra."),
    ],
    kernel: Kernel::Output,
    ..NodeSpec::BASE
};
