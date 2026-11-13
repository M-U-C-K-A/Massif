//! Ready-made graphs: landscapes to start from, each an ordinary graph to
//! change. They follow the world: the sea is the world's, and their
//! features grow with the map (within reason: a mountain stays a mountain).

use super::{AutoPalette, BlockPalette, CellStyle, CombineOp, GradientPreset, Graph, NodeId, NodeKind, NoiseStyle, TintMode};
use crate::registry::{Registry, TerrainId};

/// A landscape to start a graph from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Template {
    Alpine,
    Archipelago,
    KarstTowers,
    FloatingMountains,
    RiverValley,
    DesertCanyon,
    VolcanicIsland,
    DuneSea,
    RollingHills,
    /// Continents from tectonic plates, with their climates.
    Continents,
    /// Not a landscape: the map as it is, painted from its relief.
    PaintMap,
    /// Not a landscape: the map as it is, with rivers and lakes.
    MapRivers,
}

impl Template {
    /// Whether it starts from the map rather than making a landscape.
    pub const fn reads_the_map(self) -> bool {
        matches!(self, Template::PaintMap | Template::MapRivers)
    }

    pub const ALL: [Template; 12] = [
        Template::Alpine,
        Template::Archipelago,
        Template::KarstTowers,
        Template::FloatingMountains,
        Template::RiverValley,
        Template::DesertCanyon,
        Template::VolcanicIsland,
        Template::DuneSea,
        Template::RollingHills,
        Template::Continents,
        Template::PaintMap,
        Template::MapRivers,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Template::Alpine => "Alpine range",
            Template::Archipelago => "Archipelago",
            Template::KarstTowers => "Karst towers",
            Template::FloatingMountains => "Floating mountains",
            Template::RiverValley => "River valley",
            Template::DesertCanyon => "Desert canyon",
            Template::VolcanicIsland => "Volcanic island",
            Template::DuneSea => "Dune sea",
            Template::RollingHills => "Rolling hills",
            Template::Continents => "Continents",
            Template::PaintMap => "Paint this map",
            Template::MapRivers => "Rivers on this map",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Template::Alpine => "Eroded peaks over green hills, snow on the summits",
            Template::Archipelago => "Tropical islands in a shallow sea, beaches and green peaks",
            Template::KarstTowers => "Stone towers over a jungle",
            Template::FloatingMountains => "Floating mountains over stone towers",
            Template::RiverValley => "A river valley between snowy peaks",
            Template::DesertCanyon => "A winding red canyon with a river",
            Template::VolcanicIsland => "A volcano rising from the sea",
            Template::DuneSea => "Long dunes shaped by the wind, crests and slip faces",
            Template::RollingHills => "Gentle countryside: meadows, streams and ponds",
            Template::Continents => "Continents, rivers and climates, with biomes",
            Template::PaintMap => "Paints your map from its relief",
            Template::MapRivers => "Adds rivers to your map",
        }
    }

    /// The graph, for a world whose sea is at `sea` and whose map is
    /// `side` blocks wide.
    pub fn graph(self, reg: &Registry, sea: f32, side: f32) -> Graph {
        let terrain = |name: &str| reg.terrain_by_name(name).unwrap_or(TerrainId::ROCK).0;
        // Features grow with the map, up to a point.
        let f = (side / 2048.0).clamp(0.5, 3.0);
        let mut b = Builder::default();
        match self {
            Template::Alpine => {
                return Graph::starter(terrain("Grass"), TerrainId::ROCK.0, terrain("Snow"), terrain("Gravel"));
            }
            Template::Archipelago => archipelago(&mut b, sea, f, terrain("Sand")),
            Template::KarstTowers => karst(&mut b, sea, f),
            Template::FloatingMountains => floating_mountains(&mut b, sea, f),
            Template::RiverValley => valley(&mut b, sea, f, terrain("Gravel"), terrain("Snow"), TerrainId::ROCK.0),
            Template::DesertCanyon => canyon(&mut b, sea, f),
            Template::VolcanicIsland => volcano(&mut b, sea, f),
            Template::DuneSea => dunes(&mut b, sea, f),
            Template::RollingHills => hills(&mut b, sea, f),
            Template::Continents => continents(&mut b, sea, f, terrain("Gravel")),
            Template::PaintMap => paint_map(&mut b, sea, &terrain),
            Template::MapRivers => map_rivers(&mut b, sea, f, terrain("Gravel")),
        }
        b.g
    }
}

/// Lays nodes out in columns and rows, and wires them.
#[derive(Default)]
struct Builder {
    g: Graph,
}

impl Builder {
    fn add(&mut self, kind: NodeKind, col: f32, row: f32) -> NodeId {
        self.g.add(kind, [40.0 + col * 220.0, 40.0 + row * 150.0])
    }

    /// Output `out` of `from` into input `input` of `to`.
    fn wire(&mut self, from: NodeId, out: u8, to: NodeId, input: u8) {
        self.g.connect(from, out, to, input);
    }

    fn combine(&mut self, op: CombineOp, weight: f32, a: (NodeId, u8), b: (NodeId, u8), col: f32, row: f32) -> NodeId {
        let id = self.add(NodeKind::Combine { op, weight }, col, row);
        self.wire(a.0, a.1, id, 0);
        self.wire(b.0, b.1, id, 1);
        id
    }

    /// A node with one input.
    fn then(&mut self, kind: NodeKind, from: (NodeId, u8), col: f32, row: f32) -> NodeId {
        let id = self.add(kind, col, row);
        self.wire(from.0, from.1, id, 0);
        id
    }

    fn tint(&mut self, color: [u8; 3], strength: f32, colour: NodeId, mask: (NodeId, u8), col: f32, row: f32) -> NodeId {
        let id = self.add(
            NodeKind::Tint {
                color,
                mode: TintMode::Mix,
                strength,
            },
            col,
            row,
        );
        self.wire(colour, 0, id, 0);
        self.wire(mask.0, mask.1, id, 1);
        id
    }

    /// Erosion with `rain`, `erosion` and brush `radius`.
    fn erosion(&mut self, from: NodeId, (rain, erosion, radius): (f32, f32, f32), seed: u32, col: f32, row: f32) -> NodeId {
        self.then(
            NodeKind::Erosion {
                rain,
                erosion,
                deposition: 0.32,
                capacity: 4.0,
                evaporation: 0.02,
                radius,
                seed,
            },
            (from, 0),
            col,
            row,
        )
    }
}

fn noise(style: NoiseStyle, scale: f32, octaves: u32, height: f32, base: f32, seed: u32) -> NodeKind {
    NodeKind::Noise {
        style,
        scale,
        octaves,
        roughness: 0.5,
        height,
        base,
        seed,
    }
}

fn levels(in_min: f32, in_max: f32, out_min: f32, out_max: f32, gamma: f32) -> NodeKind {
    NodeKind::Levels {
        in_min,
        in_max,
        out_min,
        out_max,
        gamma,
    }
}

fn auto(palette: AutoPalette, snow_line: f32, rock_slope: f32, seed: u32) -> NodeKind {
    NodeKind::AutoColor {
        palette,
        snow_line,
        rock_slope,
        seed,
    }
}

fn colours(palette: BlockPalette) -> NodeKind {
    NodeKind::ColorOutput { blocks: true, palette }
}

fn slope(min: f32, max: f32) -> NodeKind {
    NodeKind::SelectSlope { min, max, falloff: 6.0 }
}

/// Islands where a rolling sea floor rises above the sea, with ridged peaks
/// on the larger ones.
fn archipelago(b: &mut Builder, sea: f32, f: f32, sand: u16) {
    let floor = b.add(noise(NoiseStyle::Fbm, 900.0 * f, 7, 110.0, sea - 58.0, 21), 0.0, 0.0);
    let ridges = b.add(noise(NoiseStyle::Ridged, 380.0 * f, 6, 1.0, 0.0, 22), 0.0, 1.5);
    let land = b.then(
        NodeKind::SelectHeight {
            min: sea + 1.0,
            max: sea + 1000.0,
            falloff: 10.0,
        },
        (floor, 0),
        1.0,
        1.0,
    );
    let peaks = b.combine(CombineOp::Multiply, 70.0, (land, 0), (ridges, 0), 2.0, 1.0);
    let islands = b.combine(CombineOp::Add, 1.0, (floor, 0), (peaks, 0), 3.0, 0.0);
    let warp = b.then(
        NodeKind::Warp {
            amount: 40.0,
            scale: 320.0,
            seed: 5,
        },
        (islands, 0),
        4.0,
        0.0,
    );
    let eroded = b.erosion(warp, (0.9, 0.3, 4.0), 9, 5.0, 0.0);
    let water = b.then(NodeKind::Sea { level: sea, shore: 3.0 }, (eroded, 0), 6.0, 0.0);
    let out = b.add(NodeKind::HeightOutput, 7.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = b.then(auto(AutoPalette::Tropical, sea + 1000.0, 38.0, 3), (water, 0), 7.0, 1.0);
    let beach = b.tint([226, 208, 160], 0.9, colour, (water, 2), 8.0, 1.0);
    let bed = b.tint([200, 186, 146], 0.85, beach, (water, 1), 9.0, 1.0);
    let out = b.add(colours(BlockPalette::Natural), 10.0, 1.0);
    b.wire(bed, 0, out, 0);
    let sandy = b.combine(CombineOp::Max, 1.0, (water, 1), (water, 2), 7.0, 2.5);
    let out = b.add(NodeKind::TerrainOutput { terrain: sand }, 8.0, 2.5);
    b.wire(sandy, 0, out, 0);
}

/// A forest of stone towers over a jungle, as water leaves them when it
/// has worn a plateau away along its joints: every cell of a scatter holds
/// one tower or none, each of its own height, with sheer walls broken by
/// ledges, a rounded top and an outline no two share; they stand in
/// groups, with open jungle between. Returns the ground.
fn pillar_forest(b: &mut Builder, sea: f32, f: f32, top: f32) -> NodeId {
    // Towers stay towers: they grow less than the map.
    let p = f.clamp(1.0, 2.0);
    let cells = |style: CellStyle| NodeKind::Cells {
        style,
        scale: 62.0 * p,
        jitter: 0.8,
        height: 1.0,
        base: 0.0,
        seed: 32,
    };
    let floor = b.add(noise(NoiseStyle::Fbm, 700.0 * f, 6, 26.0, sea - 4.0, 31), 0.0, 0.0);
    // Around the middle of each cell, an outline pushed about by noise.
    let cones = b.add(cells(CellStyle::Cones), 0.0, 1.0);
    let outline = b.then(
        NodeKind::Warp {
            amount: 10.0 * p,
            scale: 38.0 * p,
            seed: 33,
        },
        (cones, 0),
        1.0,
        1.0,
    );
    // Sheer from the foot, rounding off toward the top, with ledges.
    let shape = b.then(levels(0.55, 0.94, 0.0, 1.0, 0.3), (outline, 0), 2.0, 1.0);
    let ledges = b.then(NodeKind::Terrace { steps: 5, strength: 0.5 }, (shape, 0), 3.0, 1.0);
    // Each cell's own number says whether a tower stands there, and how
    // high.
    let lots = b.add(cells(CellStyle::Plates), 0.0, 2.0);
    let stands = b.then(levels(0.34, 0.38, 0.0, 1.0, 1.0), (lots, 0), 1.0, 2.0);
    let tall = b.then(levels(0.34, 1.0, top * 0.4, top, 1.3), (lots, 0), 1.0, 3.0);
    // Towers come in groups.
    let spread = b.add(noise(NoiseStyle::Fbm, 520.0 * f, 4, 1.0, 0.0, 34), 0.0, 4.0);
    let groups = b.then(levels(0.44, 0.52, 0.0, 1.0, 1.0), (spread, 0), 1.0, 4.0);
    let there = b.combine(CombineOp::Multiply, 1.0, (stands, 0), (groups, 0), 2.0, 3.0);
    let height = b.combine(CombineOp::Multiply, 1.0, (there, 0), (tall, 0), 3.0, 3.0);
    let towers = b.combine(CombineOp::Multiply, 1.0, (ledges, 0), (height, 0), 4.0, 2.0);
    // Scree at their feet, where the jungle climbs.
    let fallen = b.then(NodeKind::Blur { radius: 9.0 * p }, (towers, 0), 5.0, 3.0);
    let scree = b.then(levels(0.0, top, 0.0, top * 0.45, 1.0), (fallen, 0), 6.0, 3.0);
    let standing = b.combine(CombineOp::Max, 1.0, (towers, 0), (scree, 0), 7.0, 2.0);
    let land = b.combine(CombineOp::Add, 1.0, (floor, 0), (standing, 0), 8.0, 1.0);
    let rocks = b.add(
        NodeKind::Rocks {
            scale: 11.0,
            height: 3.0,
            sharpness: 0.85,
            seed: 37,
        },
        8.0,
        2.0,
    );
    let rough = b.combine(CombineOp::Add, 1.0, (land, 0), (rocks, 0), 9.0, 1.0);
    b.then(
        NodeKind::Rivers {
            threshold: 0.55,
            depth: 5.0,
            width: 16.0,
            sea,
            banks: 1.2,
            lakes: 0.5,
        },
        (rough, 0),
        10.0,
        1.0,
    )
}

/// The colours of a jungle of stone: lush tops, pale stone walls with moss
/// and creepers in patches, a green floor. Returns the colour node.
fn jungle_colours(b: &mut Builder, sea: f32, ground: NodeId, col: f32, row: f32) -> NodeId {
    let colour = b.then(auto(AutoPalette::Tropical, sea + 1000.0, 54.0, 4), (ground, 0), col, row + 1.0);
    // The canopy: dark, in patches of darker still.
    let clumps_noise = b.add(noise(NoiseStyle::Billow, 46.0, 4, 1.0, 0.0, 39), col, row + 6.0);
    let clumps = b.then(levels(0.1, 0.5, 0.0, 1.0, 1.0), (clumps_noise, 0), col + 1.0, row + 6.0);
    let canopy = b.tint([30, 62, 30], 0.55, colour, (clumps, 0), col + 1.0, row + 1.0);
    // Walls of warm grey stone, moss and creepers in patches down them.
    let steep = b.then(slope(56.0, 90.0), (ground, 0), col, row + 2.0);
    let stone = b.tint([146, 134, 116], 0.7, canopy, (steep, 0), col + 2.0, row + 1.0);
    let scatter = b.add(noise(NoiseStyle::Fbm, 24.0, 4, 1.0, 0.0, 38), col, row + 3.0);
    let patches = b.then(levels(0.44, 0.62, 0.0, 1.0, 1.0), (scatter, 0), col + 1.0, row + 3.0);
    let mossy_walls = b.combine(CombineOp::Multiply, 1.0, (steep, 0), (patches, 0), col + 2.0, row + 2.5);
    let mossy = b.tint([52, 84, 40], 0.7, stone, (mossy_walls, 0), col + 3.0, row + 1.0);
    // Their tops and ledges, green again.
    let high = b.then(
        NodeKind::SelectHeight {
            min: sea + 34.0,
            max: sea + 1000.0,
            falloff: 12.0,
        },
        (ground, 0),
        col,
        row + 4.0,
    );
    let flat = b.then(slope(0.0, 38.0), (ground, 0), col, row + 5.0);
    let tops = b.combine(CombineOp::Multiply, 1.0, (high, 0), (flat, 0), col + 1.0, row + 4.5);
    b.tint([50, 88, 38], 0.6, mossy, (tops, 0), col + 4.0, row + 1.0)
}

/// Stone pillars over a wet jungle floor crossed by rivers.
fn karst(b: &mut Builder, sea: f32, f: f32) {
    let ground = pillar_forest(b, sea, f, 160.0);
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (ground, 0), 11.0, 1.0);
    let out = b.add(NodeKind::HeightOutput, 12.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = jungle_colours(b, sea, water, 12.0, 0.0);
    let out = b.add(colours(BlockPalette::Natural), 17.0, 1.0);
    b.wire(colour, 0, out, 0);
}

/// Mountains floating over a jungle of stone towers: rock over the void, a
/// level of the map of its own. The islands float above the towers' tops,
/// never into them.
fn floating_mountains(b: &mut Builder, sea: f32, f: f32) {
    let ground = pillar_forest(b, sea, f, 118.0);
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (ground, 0), 11.0, 1.0);
    let out = b.add(NodeKind::HeightOutput, 12.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = jungle_colours(b, sea, water, 12.0, 0.0);
    let out = b.add(colours(BlockPalette::Natural), 17.0, 1.0);
    b.wire(colour, 0, out, 0);
    // The floating mountains, high over the towers.
    let islands = b.add(
        NodeKind::FloatingIslands {
            altitude: sea + 186.0,
            spread: 14.0,
            size: 66.0 * f.clamp(0.8, 1.15),
            coverage: 0.5,
            depth: 1.2,
            seed: 92,
        },
        11.0,
        9.0,
    );
    let clearance = b.add(NodeKind::Constant { value: 14.0 }, 11.0, 10.5);
    let above = b.combine(CombineOp::Add, 1.0, (water, 0), (clearance, 0), 12.0, 10.0);
    let bottom = b.combine(CombineOp::Max, 1.0, (islands, 1), (above, 0), 13.0, 9.5);
    let green = jungle_colours(b, sea, islands, 12.0, 12.0);
    let out = b.add(
        NodeKind::LayerOutput {
            slot: 1,
            palette: BlockPalette::Natural,
        },
        15.0,
        9.5,
    );
    b.wire(islands, 0, out, 0);
    b.wire(bottom, 0, out, 1);
    b.wire(green, 0, out, 2);
}

/// A wide valley with a flat floor between ridged mountains, and a river
/// down the middle of the floor, below the sea so that it holds water.
fn valley(b: &mut Builder, sea: f32, f: f32, gravel: u16, snow: u16, rock: u16) {
    let (angle, meander, seed) = (20.0, 420.0 * f, 41);
    let carved = b.add(
        NodeKind::Canyon {
            angle,
            width: 1000.0 * f,
            depth: 100.0,
            meander,
            steps: 0,
            base: 100.0,
            seed,
        },
        0.0,
        0.0,
    );
    let walls = b.then(levels(0.0, 100.0, sea + 2.0, sea + 70.0, 1.2), (carved, 0), 1.0, 0.0);
    let open = b.then(levels(0.0, 100.0, 0.0, 1.0, 1.6), (carved, 0), 1.0, 1.0);
    let ridges = b.add(noise(NoiseStyle::Ridged, 650.0 * f, 7, 160.0, 0.0, 42), 0.0, 2.0);
    let mountains = b.combine(CombineOp::Multiply, 1.0, (open, 0), (ridges, 0), 2.0, 1.5);
    let land = b.combine(CombineOp::Add, 1.0, (walls, 0), (mountains, 0), 3.0, 0.5);
    // The same course as the valley, narrow and shallow.
    let river = b.add(
        NodeKind::Canyon {
            angle,
            width: 26.0,
            depth: 9.0,
            meander,
            steps: 0,
            base: 0.0,
            seed,
        },
        2.0,
        3.0,
    );
    let with_river = b.combine(CombineOp::Add, 1.0, (land, 0), (river, 0), 4.0, 0.5);
    let warp = b.then(
        NodeKind::Warp {
            amount: 50.0,
            scale: 600.0,
            seed: 43,
        },
        (with_river, 0),
        5.0,
        0.5,
    );
    let eroded = b.erosion(warp, (1.1, 0.35, 5.0), 44, 6.0, 0.5);
    let snowy = b.then(
        NodeKind::Snow {
            line: sea + 175.0,
            slope: 40.0,
            depth: 2.0,
            softness: 14.0,
        },
        (eroded, 0),
        7.0,
        0.5,
    );
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (snowy, 0), 8.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 9.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = b.then(auto(AutoPalette::Temperate, sea + 170.0, 36.0, 5), (water, 0), 9.0, 1.0);
    let wet = b.tint([64, 96, 50], 0.35, colour, (eroded, 1), 10.0, 1.0);
    let bed = b.tint([120, 116, 108], 0.8, wet, (water, 1), 11.0, 1.0);
    let out = b.add(colours(BlockPalette::Natural), 12.0, 1.0);
    b.wire(bed, 0, out, 0);
    let out = b.add(NodeKind::TerrainOutput { terrain: gravel }, 9.0, 2.2);
    b.wire(water, 1, out, 0);
    let cliffs = b.then(slope(44.0, 90.0), (water, 0), 9.0, 3.2);
    let out = b.add(NodeKind::TerrainOutput { terrain: rock }, 10.0, 3.2);
    b.wire(cliffs, 0, out, 0);
    let out = b.add(NodeKind::TerrainOutput { terrain: snow }, 9.0, 4.2);
    b.wire(snowy, 1, out, 0);
}

/// A stepped canyon cut into a plateau, buttes on it and in it, a river at
/// the bottom.
fn canyon(b: &mut Builder, sea: f32, f: f32) {
    let (angle, meander, seed) = (30.0, 180.0 * f, 51);
    let cut = b.add(
        NodeKind::Canyon {
            angle,
            width: 340.0 * f,
            depth: 104.0,
            meander,
            steps: 7,
            base: sea + 112.0,
            seed,
        },
        0.0,
        0.0,
    );
    let mesas = b.add(
        NodeKind::Mesas {
            scale: 520.0 * f,
            coverage: 0.22,
            height: 36.0,
            base: 0.0,
            steps: 3,
            seed: 52,
        },
        0.0,
        1.0,
    );
    let land = b.combine(CombineOp::Add, 1.0, (cut, 0), (mesas, 0), 1.0, 0.5);
    let rocks = b.add(
        NodeKind::Rocks {
            scale: 60.0,
            height: 8.0,
            sharpness: 0.7,
            seed: 53,
        },
        1.0,
        1.5,
    );
    let rough = b.combine(CombineOp::Add, 1.0, (land, 0), (rocks, 0), 2.0, 0.5);
    let river = b.add(
        NodeKind::Canyon {
            angle,
            width: 34.0,
            depth: 12.0,
            meander,
            steps: 0,
            base: 0.0,
            seed,
        },
        2.0,
        1.5,
    );
    let carved = b.combine(CombineOp::Add, 1.0, (rough, 0), (river, 0), 3.0, 0.5);
    let strata = b.then(
        NodeKind::Stratify {
            layers: 18,
            strength: 0.6,
            tilt: 4.0,
            seed: 55,
        },
        (carved, 0),
        4.0,
        0.5,
    );
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (strata, 0), 5.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 6.0, 0.0);
    b.wire(water, 0, out, 0);
    let bands = b.then(
        NodeKind::Gradient {
            preset: GradientPreset::RedRock,
            auto_range: true,
            min: 0.0,
            max: 1.0,
            reverse: false,
            variation: 0.4,
            seed: 2,
        },
        (water, 0),
        6.0,
        1.0,
    );
    let flat = b.then(slope(0.0, 14.0), (water, 0), 6.0, 2.0);
    let sand = b.tint([214, 160, 104], 0.7, bands, (flat, 0), 7.0, 1.0);
    let bed = b.tint([176, 140, 100], 0.8, sand, (water, 1), 8.0, 1.0);
    let out = b.add(colours(BlockPalette::Desert), 9.0, 1.0);
    b.wire(bed, 0, out, 0);
}

/// A volcano on an island: dark rock and black sand, green where the lower
/// slopes are gentle.
fn volcano(b: &mut Builder, sea: f32, f: f32) {
    let island = b.add(
        NodeKind::Island {
            size: 1.0,
            height: 64.0,
            base: sea - 26.0,
            coast: 0.55,
            seed: 61,
        },
        0.0,
        0.0,
    );
    let cone = b.add(
        NodeKind::Volcano {
            x: 0.52,
            z: 0.48,
            radius: 600.0 * f,
            height: 200.0,
            base: 0.0,
            crater: 0.16,
            depth: 55.0,
            seed: 62,
        },
        0.0,
        1.0,
    );
    let land = b.combine(CombineOp::Add, 1.0, (island, 0), (cone, 0), 1.0, 0.5);
    let warp = b.then(
        NodeKind::Warp {
            amount: 30.0,
            scale: 300.0,
            seed: 63,
        },
        (land, 0),
        2.0,
        0.5,
    );
    let eroded = b.erosion(warp, (1.0, 0.32, 4.0), 64, 3.0, 0.5);
    let water = b.then(NodeKind::Sea { level: sea, shore: 3.0 }, (eroded, 0), 4.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 5.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = b.then(auto(AutoPalette::Volcanic, sea + 1000.0, 34.0, 6), (water, 0), 5.0, 1.0);
    let low = b.then(
        NodeKind::SelectHeight {
            min: sea + 3.0,
            max: sea + 95.0,
            falloff: 18.0,
        },
        (water, 0),
        5.0,
        2.0,
    );
    let gentle = b.then(slope(0.0, 32.0), (water, 0), 5.0, 3.0);
    let growing = b.combine(CombineOp::Multiply, 1.0, (low, 0), (gentle, 0), 6.0, 2.5);
    let green = b.tint([66, 106, 44], 0.85, colour, (growing, 0), 6.0, 1.0);
    let beach = b.tint([46, 44, 46], 0.9, green, (water, 2), 7.0, 1.0);
    let out = b.add(colours(BlockPalette::Natural), 8.0, 1.0);
    b.wire(beach, 0, out, 0);
}

/// Dunes on a broad swell, with small ripples over them.
fn dunes(b: &mut Builder, sea: f32, f: f32) {
    let big = b.add(
        NodeKind::Dunes {
            angle: 65.0,
            wavelength: 150.0 * f,
            height: 26.0,
            base: sea + 18.0,
            sharpness: 0.75,
            seed: 71,
        },
        0.0,
        0.0,
    );
    let swell = b.add(noise(NoiseStyle::Billow, 1500.0 * f, 4, 34.0, 0.0, 72), 0.0, 1.0);
    let land = b.combine(CombineOp::Add, 1.0, (big, 0), (swell, 0), 1.0, 0.5);
    let ripples = b.add(
        NodeKind::Dunes {
            angle: 40.0,
            wavelength: 34.0,
            height: 3.0,
            base: 0.0,
            sharpness: 0.4,
            seed: 73,
        },
        1.0,
        1.5,
    );
    let sand = b.combine(CombineOp::Add, 1.0, (land, 0), (ripples, 0), 2.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 3.0, 0.0);
    b.wire(sand, 0, out, 0);
    let colour = b.then(
        NodeKind::Gradient {
            preset: GradientPreset::Desert,
            auto_range: true,
            min: 0.0,
            max: 1.0,
            reverse: false,
            variation: 0.25,
            seed: 3,
        },
        (sand, 0),
        3.0,
        1.0,
    );
    let out = b.add(colours(BlockPalette::Desert), 4.0, 1.0);
    b.wire(colour, 0, out, 0);
}

/// Low hills worn by water, with streams and, where they dip under the sea
/// level, ponds.
fn hills(b: &mut Builder, sea: f32, f: f32) {
    let rolling = b.add(noise(NoiseStyle::Fbm, 1000.0 * f, 7, 92.0, sea - 30.0, 81), 0.0, 0.0);
    let bumps = b.add(noise(NoiseStyle::Billow, 260.0 * f, 4, 10.0, 0.0, 82), 0.0, 1.0);
    let land = b.combine(CombineOp::Add, 1.0, (rolling, 0), (bumps, 0), 1.0, 0.5);
    let eroded = b.erosion(land, (1.0, 0.25, 5.0), 83, 2.0, 0.5);
    let streams = b.then(
        NodeKind::Rivers {
            threshold: 0.6,
            depth: 3.0,
            width: 8.0,
            sea,
            banks: 0.5,
            lakes: 0.3,
        },
        (eroded, 0),
        3.0,
        0.5,
    );
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (streams, 0), 4.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 5.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = b.then(auto(AutoPalette::Temperate, sea + 1000.0, 34.0, 7), (water, 0), 5.0, 1.0);
    let wet = b.tint([58, 92, 44], 0.4, colour, (eroded, 1), 6.0, 1.0);
    let shore = b.tint([200, 186, 140], 0.6, wet, (water, 2), 7.0, 1.0);
    let out = b.add(colours(BlockPalette::Natural), 8.0, 1.0);
    b.wire(shore, 0, out, 0);
}

/// The map, with rivers dug where its water gathers and water in them and
/// in its hollows: what to start from to work on a map that exists.
/// Continents and oceans from tectonic plates — ranges where they collide —
/// worn by rain, rivers to the sea, and the climate: warm in the south,
/// rain from the west, biomes from tundra to jungle.
fn continents(b: &mut Builder, sea: f32, f: f32, gravel: u16) {
    let plates = b.add(
        NodeKind::Tectonic {
            plates: super::tectonics::scatter(9, 11),
            relief: 165.0,
            width: 240.0 * f,
            roughness: 0.65,
            sea,
            seed: 11,
        },
        0.0,
        0.0,
    );
    let hills = b.add(noise(NoiseStyle::Fbm, 380.0 * f, 6, 34.0, -17.0, 12), 0.0, 1.0);
    let rough = b.combine(CombineOp::Add, 1.0, (plates, 0), (hills, 0), 1.0, 0.5);
    let worn = b.erosion(rough, (1.0, 0.3, 4.0), 13, 2.0, 0.5);
    let rivers = b.then(
        NodeKind::Rivers {
            threshold: 0.55,
            depth: 4.0,
            width: 12.0 * f.clamp(1.0, 2.5),
            sea,
            banks: 1.0,
            lakes: 0.5,
        },
        (worn, 0),
        3.0,
        0.5,
    );
    let out = b.add(NodeKind::HeightOutput, 5.0, 0.0);
    b.wire(rivers, 0, out, 0);
    let out = b.add(NodeKind::WaterOutput, 5.0, 1.0);
    b.wire(rivers, 2, out, 0);
    let warmth = b.then(
        NodeKind::Temperature {
            warm: 30.0,
            cold: -4.0,
            toward: 180.0,
            lapse: 11.0,
            sea,
            variation: 3.0,
            seed: 14,
        },
        (rivers, 0),
        4.0,
        2.0,
    );
    let wet = b.then(
        NodeKind::Humidity {
            from: 270.0,
            base: 0.22,
            reach: 1600.0 * f,
            shadow: 0.75,
            sea,
            variation: 0.1,
            seed: 15,
        },
        (rivers, 0),
        4.0,
        3.0,
    );
    let colours = b.add(NodeKind::Biomes { sea }, 5.0, 2.0);
    let biomes = b.add(NodeKind::BiomeOutput { sea, paint: true }, 5.0, 3.0);
    for to in [colours, biomes] {
        b.wire(rivers, 0, to, 0);
        b.wire(warmth, 0, to, 1);
        b.wire(wet, 0, to, 2);
    }
    let out = b.add(
        NodeKind::ColorOutput {
            blocks: false,
            palette: BlockPalette::Natural,
        },
        6.0,
        2.0,
    );
    b.wire(colours, 0, out, 0);
    let out = b.add(NodeKind::TerrainOutput { terrain: gravel }, 5.0, 4.0);
    b.wire(rivers, 1, out, 0);
}

/// The map as it is, its relief left alone, painted from it: grass, worn
/// ground on the steeper slopes, rock on the cliffs, gravel where the water
/// runs and in the beds of the map's water, sand on the shores, snow on
/// the heights that hold it. Each paint its own terrain output, later ones
/// over earlier ones: take one away, the map keeps its own paint there.
fn paint_map(b: &mut Builder, sea: f32, terrain: &dyn Fn(&str) -> u16) {
    let map = b.add(NodeKind::MapHeight, 0.0, 0.0);
    let out = b.add(NodeKind::HeightOutput, 4.0, 0.0);
    b.wire(map, 0, out, 0);
    // The map's own colours under the paint, to see it by.
    let shown = b.add(
        NodeKind::ColorOutput {
            blocks: false,
            palette: BlockPalette::Natural,
        },
        4.0,
        1.0,
    );
    b.wire(map, 2, shown, 0);
    let mut row = 2.0;
    let mut paint = |b: &mut Builder, mask: (NodeId, u8), name: &str| {
        let out = b.add(NodeKind::TerrainOutput { terrain: terrain(name) }, 4.0, row);
        b.wire(mask.0, mask.1, out, 0);
        row += 1.0;
    };
    let everywhere = b.then(
        NodeKind::SelectHeight {
            min: -10_000.0,
            max: 10_000.0,
            falloff: 0.0,
        },
        (map, 0),
        1.0,
        2.0,
    );
    paint(b, (everywhere, 0), "Grass");
    let worn = b.then(
        NodeKind::SelectSlope {
            min: 27.0,
            max: 41.0,
            falloff: 4.0,
        },
        (map, 0),
        1.0,
        3.0,
    );
    paint(b, (worn, 0), "Coarse Dirt");
    let cliffs = b.then(
        NodeKind::SelectSlope {
            min: 41.0,
            max: 90.0,
            falloff: 5.0,
        },
        (map, 0),
        1.0,
        4.0,
    );
    paint(b, (cliffs, 0), "Rock");
    let flow = b.then(NodeKind::Flow { spread: 0.35 }, (map, 0), 1.0, 5.0);
    let gullies = b.then(
        NodeKind::Levels {
            in_min: 0.5,
            in_max: 0.72,
            out_min: 0.0,
            out_max: 1.0,
            gamma: 1.0,
        },
        (flow, 0),
        2.0,
        5.0,
    );
    paint(b, (gullies, 0), "Gravel");
    // Under the map's water, gravel; on the shores of the sea, sand.
    paint(b, (map, 1), "Gravel");
    let shore = b.then(NodeKind::Sea { level: sea, shore: 2.5 }, (map, 0), 1.0, 7.0);
    paint(b, (shore, 2), "Sand");
    let snow = b.then(
        NodeKind::Snow {
            line: sea + 120.0,
            slope: 38.0,
            depth: 1.0,
            softness: 14.0,
        },
        (map, 0),
        1.0,
        8.0,
    );
    paint(b, (snow, 1), "Snow");
}

fn map_rivers(b: &mut Builder, sea: f32, f: f32, gravel: u16) {
    let map = b.add(NodeKind::MapHeight, 0.0, 0.0);
    let rivers = b.then(
        NodeKind::Rivers {
            threshold: 0.5,
            depth: 4.0,
            width: 10.0 * f.clamp(1.0, 2.5),
            sea,
            banks: 1.0,
            lakes: 0.5,
        },
        (map, 0),
        1.0,
        0.0,
    );
    // The water already on the map: rivers keep to it, and it stays.
    b.wire(map, 3, rivers, 1);
    // The lakes are the rivers' own: the hollows they run through. (A
    // Lakes node would fill every hollow of the map, the smallest too.)
    let out = b.add(NodeKind::HeightOutput, 3.0, 0.0);
    b.wire(rivers, 0, out, 0);
    let out = b.add(NodeKind::WaterOutput, 3.0, 1.0);
    b.wire(rivers, 2, out, 0);
    let out = b.add(NodeKind::TerrainOutput { terrain: gravel }, 2.0, 1.5);
    b.wire(rivers, 1, out, 0);
    // The map's own colours, to see it by: they are not converted.
    let out = b.add(
        NodeKind::ColorOutput {
            blocks: false,
            palette: BlockPalette::Natural,
        },
        1.0,
        1.5,
    );
    b.wire(map, 2, out, 0);
}
