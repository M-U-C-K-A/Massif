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
    RiverValley,
    DesertCanyon,
    VolcanicIsland,
    DuneSea,
    RollingHills,
}

impl Template {
    pub const ALL: [Template; 8] = [
        Template::Alpine,
        Template::Archipelago,
        Template::KarstTowers,
        Template::RiverValley,
        Template::DesertCanyon,
        Template::VolcanicIsland,
        Template::DuneSea,
        Template::RollingHills,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Template::Alpine => "Alpine range",
            Template::Archipelago => "Archipelago",
            Template::KarstTowers => "Karst towers",
            Template::RiverValley => "River valley",
            Template::DesertCanyon => "Desert canyon",
            Template::VolcanicIsland => "Volcanic island",
            Template::DuneSea => "Dune sea",
            Template::RollingHills => "Rolling hills",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Template::Alpine => "Eroded peaks over green hills, snow on the summits",
            Template::Archipelago => "Tropical islands in a shallow sea, beaches and green peaks",
            Template::KarstTowers => "Sheer stone pillars over a jungle, green tops, mist between them",
            Template::RiverValley => "A wide valley between snowy mountains, a river along its floor",
            Template::DesertCanyon => "A winding canyon in a red plateau, stepped walls, buttes, a river",
            Template::VolcanicIsland => "A volcano rising from the sea, black beaches, green lower slopes",
            Template::DuneSea => "Long dunes shaped by the wind, crests and slip faces",
            Template::RollingHills => "Gentle countryside: meadows, streams and ponds",
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
            Template::RiverValley => valley(&mut b, sea, f, terrain("Gravel"), terrain("Snow"), TerrainId::ROCK.0),
            Template::DesertCanyon => canyon(&mut b, sea, f),
            Template::VolcanicIsland => volcano(&mut b, sea, f),
            Template::DuneSea => dunes(&mut b, sea, f),
            Template::RollingHills => hills(&mut b, sea, f),
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

/// Pillars with sheer walls and rounded tops, in clusters over a low,
/// wet jungle floor.
fn karst(b: &mut Builder, sea: f32, f: f32) {
    // Pillars stay pillars: they grow less than the map, and never shrink
    // to needles.
    let p = f.clamp(1.0, 2.0);
    let floor = b.add(noise(NoiseStyle::Fbm, 700.0 * f, 6, 36.0, sea - 8.0, 31), 0.0, 0.0);
    let cells = b.add(
        NodeKind::Cells {
            style: CellStyle::Cones,
            scale: 110.0 * p,
            jitter: 0.9,
            height: 1.0,
            base: 0.0,
            seed: 32,
        },
        0.0,
        1.0,
    );
    // Sheer walls, then a rounded top.
    let pillars = b.then(levels(0.55, 0.72, 0.0, 1.0, 0.12), (cells, 0), 1.0, 1.0);
    let spread = b.add(noise(NoiseStyle::Fbm, 520.0 * f, 4, 1.0, 0.0, 33), 0.0, 2.0);
    let clusters = b.then(levels(0.49, 0.6, 0.0, 1.0, 1.0), (spread, 0), 1.0, 2.0);
    let rise = b.add(noise(NoiseStyle::Fbm, 160.0 * p, 3, 1.0, 0.0, 34), 0.0, 3.0);
    let tall = b.then(levels(0.3, 0.7, 70.0, 185.0, 1.0), (rise, 0), 1.0, 3.0);
    let standing = b.combine(CombineOp::Multiply, 1.0, (pillars, 0), (clusters, 0), 2.0, 1.5);
    let towers = b.combine(CombineOp::Multiply, 1.0, (standing, 0), (tall, 0), 3.0, 2.0);
    let land = b.combine(CombineOp::Add, 1.0, (floor, 0), (towers, 0), 4.0, 0.5);
    let rocks = b.add(
        NodeKind::Rocks {
            scale: 14.0,
            height: 5.0,
            sharpness: 0.8,
            seed: 35,
        },
        4.0,
        1.5,
    );
    let rough = b.combine(CombineOp::Add, 1.0, (land, 0), (rocks, 0), 5.0, 0.5);
    let rivers = b.then(
        NodeKind::Rivers {
            threshold: 0.55,
            depth: 5.0,
            width: 6.0,
        },
        (rough, 0),
        6.0,
        0.5,
    );
    let water = b.then(NodeKind::Sea { level: sea, shore: 2.0 }, (rivers, 0), 7.0, 0.5);
    let out = b.add(NodeKind::HeightOutput, 8.0, 0.0);
    b.wire(water, 0, out, 0);
    let colour = b.then(auto(AutoPalette::Tropical, sea + 1000.0, 52.0, 4), (water, 0), 8.0, 1.0);
    let steep = b.then(slope(58.0, 90.0), (water, 0), 8.0, 2.0);
    let stone = b.tint([128, 124, 112], 0.55, colour, (steep, 0), 9.0, 1.0);
    // Moss and creepers in patches down the walls.
    let scatter = b.add(noise(NoiseStyle::Fbm, 26.0, 4, 1.0, 0.0, 36), 8.0, 5.0);
    let patches = b.then(levels(0.42, 0.62, 0.0, 1.0, 1.0), (scatter, 0), 9.0, 5.0);
    let shaded = b.combine(CombineOp::Multiply, 1.0, (steep, 0), (patches, 0), 10.0, 4.5);
    let high = b.then(
        NodeKind::SelectHeight {
            min: sea + 60.0,
            max: sea + 1000.0,
            falloff: 20.0,
        },
        (water, 0),
        8.0,
        3.0,
    );
    let flat = b.then(slope(0.0, 40.0), (water, 0), 8.0, 4.0);
    let tops = b.combine(CombineOp::Multiply, 1.0, (high, 0), (flat, 0), 9.0, 3.5);
    let lush = b.tint([46, 100, 38], 0.75, stone, (tops, 0), 10.0, 1.0);
    let mossy = b.tint([58, 92, 44], 0.65, lush, (shaded, 0), 11.0, 1.0);
    let out = b.add(colours(BlockPalette::Natural), 12.0, 1.0);
    b.wire(mossy, 0, out, 0);
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
            width: 4.0,
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
