//! Terrain graphs: procedural terrain from a graph of nodes, in the manner of
//! World Machine or Gaea.
//!
//! Every node makes a *field*: a square grid of values over the whole map,
//! heights in blocks or masks from 0 to 1. Generators make fields from
//! nothing (noise, cells, gradients), modifiers reshape one field (levels,
//! terraces, warp, blur, hydraulic and thermal erosion), combiners mix two
//! or three, selectors turn heights into masks (by height or slope), and
//! outputs say what goes to the map: the height of the ground and where
//! each terrain is painted.
//!
//! A graph is evaluated at a resolution chosen for the job (a few hundred
//! samples per side for previews, thousands for the final build) over the
//! extent of the map, and the result is sampled per block column when it is
//! applied. Distances are in blocks, so a graph looks the same at every
//! resolution.

use crate::codec::snap_height;
use crate::compose::hash2;
use crate::coords::{BlockRect, TilePos, local_index};
use crate::history::Edit;
use crate::noise::Noise;
use crate::registry::TerrainId;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub type NodeId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseStyle {
    /// Rolling, natural relief.
    Fbm,
    /// Sharp crests and rounded valleys: mountain ranges.
    Ridged,
    /// Rounded bumps with creases between them: dunes, hills.
    Billow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellStyle {
    /// A cone at every cell: peaks, volcanoes.
    Cones,
    /// Ridges along the edges between cells: plates, mesas.
    Ridges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombineOp {
    Add,
    Subtract,
    Multiply,
    Max,
    Min,
    Average,
}

impl CombineOp {
    pub const ALL: [CombineOp; 6] = [
        CombineOp::Add,
        CombineOp::Subtract,
        CombineOp::Multiply,
        CombineOp::Max,
        CombineOp::Min,
        CombineOp::Average,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            CombineOp::Add => "Add",
            CombineOp::Subtract => "Subtract",
            CombineOp::Multiply => "Multiply",
            CombineOp::Max => "Max",
            CombineOp::Min => "Min",
            CombineOp::Average => "Average",
        }
    }
}

/// What a node does, with its settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    // Generators.
    Noise {
        style: NoiseStyle,
        /// Size of the largest features, in blocks.
        scale: f32,
        octaves: u32,
        /// How much each octave keeps of the previous one's amplitude.
        roughness: f32,
        /// Height of the relief, in blocks, on top of `base`.
        height: f32,
        base: f32,
        seed: u32,
    },
    Cells {
        style: CellStyle,
        scale: f32,
        /// How far points stray from the cell centres (0..1).
        jitter: f32,
        height: f32,
        base: f32,
        seed: u32,
    },
    /// Highest at the centre of the map, falling to the edges: islands.
    Radial {
        /// Fraction of the half-width where the fall starts and ends.
        inner: f32,
        outer: f32,
        height: f32,
        base: f32,
    },
    Constant {
        value: f32,
    },
    // Modifiers.
    Levels {
        in_min: f32,
        in_max: f32,
        out_min: f32,
        out_max: f32,
        /// Above 1 flattens the low end, below 1 the high end.
        gamma: f32,
    },
    Terrace {
        steps: u32,
        /// 0: no terraces, 1: flat steps.
        strength: f32,
    },
    Clamp {
        min: f32,
        max: f32,
    },
    Blur {
        /// In blocks.
        radius: f32,
    },
    Warp {
        /// How far the field is pushed, in blocks.
        amount: f32,
        scale: f32,
        seed: u32,
    },
    /// Water running down the slopes carries soil from the heights to the
    /// valleys: gullies, smoothed slopes, alluvial fans.
    Erosion {
        /// Raindrops per sample of the field.
        rain: f32,
        /// How much soil a drop takes, and leaves.
        erosion: f32,
        deposition: f32,
        /// How much soil a drop can carry.
        capacity: f32,
        evaporation: f32,
        /// Radius of the erosion brush, in blocks.
        radius: f32,
        seed: u32,
    },
    /// Loose material slides down where the slope is steeper than `talus`.
    Thermal {
        /// Steepest stable slope, in degrees.
        talus: f32,
        iterations: u32,
        strength: f32,
    },
    // Combiners.
    Combine {
        op: CombineOp,
        /// Weight of the second input.
        weight: f32,
    },
    /// The first input where the mask is 0, the second where it is 1.
    Blend,
    // Selectors.
    SelectHeight {
        min: f32,
        max: f32,
        /// Softness of the limits, in blocks.
        falloff: f32,
    },
    SelectSlope {
        /// Degrees.
        min: f32,
        max: f32,
        falloff: f32,
    },
    /// 1 − the input, for masks.
    Invert,
    // Outputs.
    /// The height of the ground.
    HeightOutput,
    /// Paint a terrain where the mask is high (dithered in between).
    TerrainOutput {
        terrain: u16,
    },
}

/// Families of nodes, for the editor's menus and colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Generator,
    Modifier,
    Erosion,
    Combiner,
    Selector,
    Output,
}

impl NodeKind {
    /// A node of each kind with sensible settings, for the "add" menu.
    pub fn catalogue() -> Vec<NodeKind> {
        vec![
            NodeKind::Noise {
                style: NoiseStyle::Fbm,
                scale: 600.0,
                octaves: 6,
                roughness: 0.5,
                height: 80.0,
                base: 50.0,
                seed: 1,
            },
            NodeKind::Noise {
                style: NoiseStyle::Ridged,
                scale: 900.0,
                octaves: 7,
                roughness: 0.5,
                height: 180.0,
                base: 40.0,
                seed: 2,
            },
            NodeKind::Cells {
                style: CellStyle::Cones,
                scale: 500.0,
                jitter: 0.8,
                height: 100.0,
                base: 50.0,
                seed: 3,
            },
            NodeKind::Radial {
                inner: 0.3,
                outer: 0.95,
                height: 1.0,
                base: 0.0,
            },
            NodeKind::Constant { value: 64.0 },
            NodeKind::Levels {
                in_min: 0.0,
                in_max: 255.0,
                out_min: 0.0,
                out_max: 255.0,
                gamma: 1.0,
            },
            NodeKind::Terrace {
                steps: 8,
                strength: 0.7,
            },
            NodeKind::Clamp { min: -64.0, max: 319.0 },
            NodeKind::Blur { radius: 8.0 },
            NodeKind::Warp {
                amount: 60.0,
                scale: 400.0,
                seed: 4,
            },
            NodeKind::Erosion {
                rain: 1.0,
                erosion: 0.3,
                deposition: 0.3,
                capacity: 4.0,
                evaporation: 0.02,
                radius: 6.0,
                seed: 5,
            },
            NodeKind::Thermal {
                talus: 40.0,
                iterations: 40,
                strength: 0.5,
            },
            NodeKind::Combine {
                op: CombineOp::Add,
                weight: 1.0,
            },
            NodeKind::Blend,
            NodeKind::SelectHeight {
                min: 150.0,
                max: 400.0,
                falloff: 10.0,
            },
            NodeKind::SelectSlope {
                min: 35.0,
                max: 90.0,
                falloff: 5.0,
            },
            NodeKind::Invert,
            NodeKind::HeightOutput,
            NodeKind::TerrainOutput { terrain: 0 },
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Noise { style: NoiseStyle::Fbm, .. } => "Noise",
            NodeKind::Noise { style: NoiseStyle::Ridged, .. } => "Ridged noise",
            NodeKind::Noise { style: NoiseStyle::Billow, .. } => "Billow noise",
            NodeKind::Cells { .. } => "Cells",
            NodeKind::Radial { .. } => "Radial gradient",
            NodeKind::Constant { .. } => "Constant",
            NodeKind::Levels { .. } => "Levels",
            NodeKind::Terrace { .. } => "Terraces",
            NodeKind::Clamp { .. } => "Clamp",
            NodeKind::Blur { .. } => "Blur",
            NodeKind::Warp { .. } => "Warp",
            NodeKind::Erosion { .. } => "Erosion",
            NodeKind::Thermal { .. } => "Thermal erosion",
            NodeKind::Combine { .. } => "Combine",
            NodeKind::Blend => "Blend",
            NodeKind::SelectHeight { .. } => "Select height",
            NodeKind::SelectSlope { .. } => "Select slope",
            NodeKind::Invert => "Invert",
            NodeKind::HeightOutput => "Height output",
            NodeKind::TerrainOutput { .. } => "Terrain output",
        }
    }

    pub fn family(&self) -> Family {
        match self {
            NodeKind::Noise { .. } | NodeKind::Cells { .. } | NodeKind::Radial { .. } | NodeKind::Constant { .. } => Family::Generator,
            NodeKind::Levels { .. } | NodeKind::Terrace { .. } | NodeKind::Clamp { .. } | NodeKind::Blur { .. } | NodeKind::Warp { .. } => {
                Family::Modifier
            }
            NodeKind::Erosion { .. } | NodeKind::Thermal { .. } => Family::Erosion,
            NodeKind::Combine { .. } | NodeKind::Blend => Family::Combiner,
            NodeKind::SelectHeight { .. } | NodeKind::SelectSlope { .. } | NodeKind::Invert => Family::Selector,
            NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } => Family::Output,
        }
    }

    /// Names of the inputs.
    pub fn inputs(&self) -> &'static [&'static str] {
        match self.family() {
            Family::Generator => &[],
            Family::Combiner => match self {
                NodeKind::Blend => &["A", "B", "Mask"],
                _ => &["A", "B"],
            },
            Family::Output => match self {
                NodeKind::TerrainOutput { .. } => &["Mask"],
                _ => &["Height"],
            },
            _ => &["In"],
        }
    }

    /// Names of the outputs.
    pub fn outputs(&self) -> &'static [&'static str] {
        match self {
            NodeKind::Erosion { .. } => &["Height", "Flow", "Deposits"],
            NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } => &[],
            _ if self.family() == Family::Selector => &["Mask"],
            _ => &["Out"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Where the node sits in the editor.
    pub pos: [f32; 2],
}

/// `from`'s output `output` feeds `to`'s input `input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Link {
    pub from: NodeId,
    pub output: u8,
    pub to: NodeId,
    pub input: u8,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub next_id: NodeId,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GraphError {
    #[error("the graph loops back on itself")]
    Cycle,
    #[error("“{0}” is missing an input")]
    MissingInput(String),
    #[error("the build was cancelled")]
    Cancelled,
}

impl Graph {
    pub fn add(&mut self, kind: NodeKind, pos: [f32; 2]) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(Node { id, kind, pos });
        id
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn remove(&mut self, id: NodeId) {
        self.nodes.retain(|n| n.id != id);
        self.links.retain(|l| l.from != id && l.to != id);
    }

    /// Connect an output to an input, replacing what fed that input. Refused
    /// (returns false) when it would make a loop.
    pub fn connect(&mut self, from: NodeId, output: u8, to: NodeId, input: u8) -> bool {
        if from == to || self.depends_on(from, to) {
            return false;
        }
        self.links.retain(|l| !(l.to == to && l.input == input));
        self.links.push(Link { from, output, to, input });
        true
    }

    pub fn disconnect(&mut self, to: NodeId, input: u8) {
        self.links.retain(|l| !(l.to == to && l.input == input));
    }

    /// Whether `node` depends (through links) on `on`.
    pub fn depends_on(&self, node: NodeId, on: NodeId) -> bool {
        let mut stack = vec![node];
        let mut seen = HashSet::new();
        while let Some(n) = stack.pop() {
            if n == on {
                return true;
            }
            if seen.insert(n) {
                stack.extend(self.links.iter().filter(|l| l.to == n).map(|l| l.from));
            }
        }
        false
    }

    /// What feeds input `input` of `node`.
    pub fn input_of(&self, node: NodeId, input: u8) -> Option<(NodeId, u8)> {
        self.links.iter().find(|l| l.to == node && l.input == input).map(|l| (l.from, l.output))
    }

    /// The nodes `targets` need, in an order where every node comes after
    /// its inputs.
    fn order(&self, targets: &[NodeId]) -> Result<Vec<NodeId>, GraphError> {
        let mut order = Vec::new();
        let mut state: HashMap<NodeId, u8> = HashMap::new(); // 1 visiting, 2 done
        fn visit(g: &Graph, n: NodeId, state: &mut HashMap<NodeId, u8>, order: &mut Vec<NodeId>) -> Result<(), GraphError> {
            match state.get(&n) {
                Some(2) => return Ok(()),
                Some(1) => return Err(GraphError::Cycle),
                _ => {}
            }
            state.insert(n, 1);
            for l in g.links.iter().filter(|l| l.to == n) {
                visit(g, l.from, state, order)?;
            }
            state.insert(n, 2);
            order.push(n);
            Ok(())
        }
        for &t in targets {
            visit(self, t, &mut state, &mut order)?;
        }
        Ok(order)
    }

    /// A graph to start from: eroded mountains with rock on the steep
    /// slopes, snow on the peaks and gravel where the water left it.
    pub fn starter(grass: u16, rock: u16, snow: u16, gravel: u16) -> Self {
        let mut g = Graph::default();
        let base = g.add(
            NodeKind::Noise {
                style: NoiseStyle::Fbm,
                scale: 1400.0,
                octaves: 5,
                roughness: 0.5,
                height: 60.0,
                base: 50.0,
                seed: 7,
            },
            [40.0, 60.0],
        );
        let ridges = g.add(
            NodeKind::Noise {
                style: NoiseStyle::Ridged,
                scale: 900.0,
                octaves: 7,
                roughness: 0.5,
                height: 150.0,
                base: 0.0,
                seed: 11,
            },
            [40.0, 220.0],
        );
        let add = g.add(
            NodeKind::Combine {
                op: CombineOp::Add,
                weight: 1.0,
            },
            [260.0, 140.0],
        );
        let warp = g.add(
            NodeKind::Warp {
                amount: 80.0,
                scale: 500.0,
                seed: 3,
            },
            [460.0, 140.0],
        );
        let erosion = g.add(
            NodeKind::Erosion {
                rain: 1.0,
                erosion: 0.3,
                deposition: 0.3,
                capacity: 4.0,
                evaporation: 0.02,
                radius: 6.0,
                seed: 5,
            },
            [660.0, 140.0],
        );
        let out = g.add(NodeKind::HeightOutput, [900.0, 60.0]);
        // Grass everywhere first; the outputs below paint over it.
        let everywhere = g.add(NodeKind::Constant { value: 1.0 }, [900.0, -130.0]);
        let grass_out = g.add(NodeKind::TerrainOutput { terrain: grass }, [1110.0, -130.0]);
        let slope = g.add(
            NodeKind::SelectSlope {
                min: 38.0,
                max: 90.0,
                falloff: 6.0,
            },
            [900.0, 180.0],
        );
        let rock_out = g.add(NodeKind::TerrainOutput { terrain: rock }, [1110.0, 180.0]);
        let high = g.add(
            NodeKind::SelectHeight {
                min: 205.0,
                max: 400.0,
                falloff: 12.0,
            },
            [900.0, 340.0],
        );
        let snow_out = g.add(NodeKind::TerrainOutput { terrain: snow }, [1110.0, 340.0]);
        // Only where soil really piled up.
        let heaps = g.add(
            NodeKind::Levels {
                in_min: 0.6,
                in_max: 0.9,
                out_min: 0.0,
                out_max: 1.0,
                gamma: 1.0,
            },
            [900.0, 500.0],
        );
        let gravel_out = g.add(NodeKind::TerrainOutput { terrain: gravel }, [1110.0, 500.0]);
        g.connect(base, 0, add, 0);
        g.connect(ridges, 0, add, 1);
        g.connect(add, 0, warp, 0);
        g.connect(warp, 0, erosion, 0);
        g.connect(erosion, 0, out, 0);
        g.connect(everywhere, 0, grass_out, 0);
        g.connect(erosion, 0, slope, 0);
        g.connect(slope, 0, rock_out, 0);
        g.connect(erosion, 0, high, 0);
        g.connect(high, 0, snow_out, 0);
        g.connect(erosion, 2, heaps, 0);
        g.connect(heaps, 0, gravel_out, 0);
        g
    }
}

// ----------------------------------------------------------------- fields --

/// A square grid of samples over the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub n: usize,
    pub data: Vec<f32>,
}

impl Field {
    fn new(n: usize, data: Vec<f32>) -> Self {
        debug_assert_eq!(data.len(), n * n);
        Self { n, data }
    }

    #[inline]
    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.data[j.min(self.n - 1) * self.n + i.min(self.n - 1)]
    }

    /// Bilinear sample at fractional grid coordinates.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let max = (self.n - 1) as f32;
        let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (fu, fv) = (u - i as f32, v - j as f32);
        let a = self.at(i, j) + (self.at(i + 1, j) - self.at(i, j)) * fu;
        let b = self.at(i, j + 1) + (self.at(i + 1, j + 1) - self.at(i, j + 1)) * fu;
        a + (b - a) * fv
    }

    pub fn range(&self) -> (f32, f32) {
        self.data
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
    }
}

/// The square of the world a graph is evaluated over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    /// North-west corner, in blocks.
    pub x0: f64,
    pub z0: f64,
    /// Side, in blocks.
    pub size: f64,
    /// Samples per side.
    pub n: usize,
}

impl Extent {
    /// A square covering a rectangle of the world.
    pub fn covering(rect: BlockRect, n: usize) -> Self {
        let size = rect.width().max(rect.depth()).max(1) as f64;
        Self {
            x0: rect.x0 as f64,
            z0: rect.z0 as f64,
            size,
            n: n.max(2),
        }
    }

    /// Blocks between samples.
    pub fn spacing(&self) -> f32 {
        (self.size / self.n as f64) as f32
    }

    /// World position of sample `(i, j)`.
    #[inline]
    pub fn world(&self, i: usize, j: usize) -> (f64, f64) {
        let s = self.size / self.n as f64;
        (self.x0 + (i as f64 + 0.5) * s, self.z0 + (j as f64 + 0.5) * s)
    }

    /// Fractional grid coordinates of a world position.
    #[inline]
    pub fn grid(&self, x: f64, z: f64) -> (f32, f32) {
        let s = self.size / self.n as f64;
        (((x - self.x0) / s - 0.5) as f32, ((z - self.z0) / s - 0.5) as f32)
    }
}

// ------------------------------------------------------------- evaluating --

/// Fields made by an evaluation, per node output.
#[derive(Debug, Default, Clone)]
pub struct Evaluation {
    pub fields: HashMap<(NodeId, u8), Arc<Field>>,
}

/// What a graph sends to the map.
#[derive(Debug, Default, Clone)]
pub struct Outputs {
    pub heights: Option<Arc<Field>>,
    /// Masks of the terrain outputs, in the graph's order (later ones win).
    pub terrains: Vec<(TerrainId, Arc<Field>)>,
}

impl Graph {
    /// Evaluate the nodes `targets` need over `extent`. With `keep_all`,
    /// every intermediate field is kept (for previews); otherwise fields are
    /// dropped as soon as nothing else needs them.
    pub fn evaluate(&self, extent: &Extent, targets: &[NodeId], keep_all: bool, cancel: &AtomicBool) -> Result<Evaluation, GraphError> {
        let order = self.order(targets)?;
        let mut uses: HashMap<(NodeId, u8), usize> = HashMap::new();
        for l in &self.links {
            if order.contains(&l.to) {
                *uses.entry((l.from, l.output)).or_default() += 1;
            }
        }
        let mut out = Evaluation::default();
        for id in order {
            if cancel.load(Ordering::Relaxed) {
                return Err(GraphError::Cancelled);
            }
            let node = self.node(id).expect("ordered nodes exist");
            let inputs: Vec<Option<Arc<Field>>> = (0..node.kind.inputs().len())
                .map(|k| self.input_of(id, k as u8).and_then(|src| out.fields.get(&src).cloned()))
                .collect();
            let made = run(&node.kind, extent, &inputs, cancel).map_err(|missing| {
                if missing {
                    GraphError::MissingInput(node.kind.label().to_owned())
                } else {
                    GraphError::Cancelled
                }
            })?;
            for (k, f) in made.into_iter().enumerate() {
                out.fields.insert((id, k as u8), Arc::new(f));
            }
            if !keep_all {
                for k in 0..node.kind.inputs().len() {
                    if let Some(src) = self.input_of(id, k as u8)
                        && let Some(n) = uses.get_mut(&src)
                    {
                        *n -= 1;
                        if *n == 0 && !targets.contains(&src.0) {
                            out.fields.remove(&src);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Evaluate what the outputs need, and gather it.
    pub fn build(&self, extent: &Extent, cancel: &AtomicBool) -> Result<Outputs, GraphError> {
        let outputs: Vec<&Node> = self.nodes.iter().filter(|n| n.kind.family() == Family::Output).collect();
        // Outputs have no fields of their own: evaluate what feeds them.
        let mut feeds = Vec::new();
        for n in &outputs {
            match self.input_of(n.id, 0) {
                Some(src) => feeds.push(src),
                None => return Err(GraphError::MissingInput(n.kind.label().to_owned())),
            }
        }
        let targets: Vec<NodeId> = feeds.iter().map(|f| f.0).collect();
        let eval = self.evaluate(extent, &targets, false, cancel)?;
        let mut result = Outputs::default();
        for (n, src) in outputs.iter().zip(feeds) {
            let field = eval.fields.get(&src).cloned().ok_or(GraphError::Cancelled)?;
            match n.kind {
                NodeKind::HeightOutput => result.heights = Some(field),
                NodeKind::TerrainOutput { terrain } => result.terrains.push((TerrainId(terrain), field)),
                _ => {}
            }
        }
        Ok(result)
    }
}

/// Run one node. `Err(true)`: an input is missing; `Err(false)`: cancelled.
fn run(kind: &NodeKind, e: &Extent, inputs: &[Option<Arc<Field>>], cancel: &AtomicBool) -> Result<Vec<Field>, bool> {
    let n = e.n;
    let input = |k: usize| inputs.get(k).cloned().flatten().ok_or(true);
    let map = |f: &Field, op: &(dyn Fn(f32) -> f32 + Sync)| Field::new(n, f.data.par_iter().map(|&v| op(v)).collect());
    let generate = |op: &(dyn Fn(f64, f64) -> f32 + Sync)| {
        Field::new(
            n,
            (0..n * n)
                .into_par_iter()
                .map(|c| {
                    let (x, z) = e.world(c % n, c / n);
                    op(x, z)
                })
                .collect(),
        )
    };
    let field = match *kind {
        NodeKind::Noise {
            style,
            scale,
            octaves,
            roughness,
            height,
            base,
            seed,
        } => {
            let noise = Noise::new(seed as u64 ^ 0x006e_6f69_7365);
            let s = scale.max(1.0) as f64;
            let oct = octaves.clamp(1, 12);
            let gain = roughness.clamp(0.05, 0.95) as f64;
            generate(&|x, z| {
                let v = match style {
                    NoiseStyle::Fbm => noise.fbm(x / s, z / s, oct, 2.0, gain) * 0.5 + 0.5,
                    NoiseStyle::Ridged => noise.ridged(x / s, z / s, oct, 2.0, gain),
                    NoiseStyle::Billow => noise.fbm(x / s, z / s, oct, 2.0, gain).abs(),
                };
                base + height * v.clamp(0.0, 1.0) as f32
            })
        }
        NodeKind::Cells {
            style,
            scale,
            jitter,
            height,
            base,
            seed,
        } => {
            let s = scale.max(1.0) as f64;
            let j = jitter.clamp(0.0, 1.0) as f64;
            generate(&|x, z| {
                let (fx, fz) = (x / s, z / s);
                let (cx, cz) = (fx.floor() as i32, fz.floor() as i32);
                let (mut d1, mut d2) = (f64::MAX, f64::MAX);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let (gx, gz) = (cx + dx, cz + dz);
                        let h = hash2(gx, gz, seed as u64 ^ 0xce11);
                        let ox = 0.5 + j * ((h & 0xffff) as f64 / 65535.0 - 0.5);
                        let oz = 0.5 + j * ((h >> 16) as f64 / 65535.0 - 0.5);
                        let d = (gx as f64 + ox - fx).hypot(gz as f64 + oz - fz);
                        if d < d1 {
                            d2 = d1;
                            d1 = d;
                        } else if d < d2 {
                            d2 = d;
                        }
                    }
                }
                let v = match style {
                    CellStyle::Cones => (1.0 - d1 / 0.75).max(0.0),
                    CellStyle::Ridges => (1.0 - (d2 - d1) * 2.0).clamp(0.0, 1.0),
                };
                base + height * v as f32
            })
        }
        NodeKind::Radial {
            inner,
            outer,
            height,
            base,
        } => {
            let (cx, cz) = (e.x0 + e.size / 2.0, e.z0 + e.size / 2.0);
            let half = e.size / 2.0;
            let (a, b) = (inner as f64, outer.max(inner + 0.001) as f64);
            generate(&|x, z| {
                let r = (x - cx).hypot(z - cz) / half;
                let t = ((r - a) / (b - a)).clamp(0.0, 1.0);
                let v = 1.0 - t * t * (3.0 - 2.0 * t);
                base + height * v as f32
            })
        }
        NodeKind::Constant { value } => Field::new(n, vec![value; n * n]),
        NodeKind::Levels {
            in_min,
            in_max,
            out_min,
            out_max,
            gamma,
        } => {
            let f = input(0)?;
            let span = (in_max - in_min).max(1e-6);
            let g = gamma.max(0.01);
            map(&f, &|v| {
                let t = ((v - in_min) / span).clamp(0.0, 1.0).powf(g);
                out_min + t * (out_max - out_min)
            })
        }
        NodeKind::Terrace { steps, strength } => {
            let f = input(0)?;
            let (lo, hi) = f.range();
            let span = (hi - lo).max(1e-6);
            let k = steps.max(1) as f32;
            let s = strength.clamp(0.0, 1.0);
            map(&f, &|v| {
                let t = (v - lo) / span * k;
                let floor = t.floor();
                let frac = t - floor;
                // Flat steps with steep risers, softened by the strength.
                let riser = (frac - 0.85).max(0.0) / 0.15;
                let stepped = (floor + riser * riser * (3.0 - 2.0 * riser)) / k;
                let t = t / k;
                lo + (t + (stepped - t) * s) * span
            })
        }
        NodeKind::Clamp { min, max } => {
            let f = input(0)?;
            map(&f, &|v| v.clamp(min, max.max(min)))
        }
        NodeKind::Blur { radius } => {
            let f = input(0)?;
            blur(&f, (radius / e.spacing()).round().max(1.0) as usize)
        }
        NodeKind::Warp { amount, scale, seed } => {
            let f = input(0)?;
            let nx = Noise::new(seed as u64 ^ 0x77a2);
            let nz = nx.derive(1);
            let s = scale.max(1.0) as f64;
            let a = amount as f64 / e.spacing() as f64;
            let data = (0..n * n)
                .into_par_iter()
                .map(|c| {
                    let (i, j) = (c % n, c / n);
                    let (x, z) = e.world(i, j);
                    let dx = nx.fbm(x / s, z / s, 3, 2.0, 0.5) * a;
                    let dz = nz.fbm(x / s, z / s, 3, 2.0, 0.5) * a;
                    f.sample((i as f64 + dx) as f32, (j as f64 + dz) as f32)
                })
                .collect();
            Field::new(n, data)
        }
        NodeKind::Erosion {
            rain,
            erosion,
            deposition,
            capacity,
            evaporation,
            radius,
            seed,
        } => {
            let f = input(0)?;
            let params = Droplets {
                drops: ((rain.max(0.0) * (n * n) as f32) as usize).min(64 << 20),
                erosion: erosion.clamp(0.0, 1.0),
                deposition: deposition.clamp(0.0, 1.0),
                capacity: capacity.max(0.0),
                evaporation: evaporation.clamp(0.0, 0.5),
                radius: (radius / e.spacing()).round().clamp(1.0, 12.0) as i32,
                seed,
            };
            let (h, flow, deposits) = erode(&f, e.spacing(), &params, cancel).ok_or(false)?;
            return Ok(vec![h, flow, deposits]);
        }
        NodeKind::Thermal {
            talus,
            iterations,
            strength,
        } => {
            let f = input(0)?;
            thermal(&f, e.spacing(), talus, iterations.min(500), strength.clamp(0.0, 1.0))
        }
        NodeKind::Combine { op, weight } => {
            let (a, b) = (input(0)?, input(1)?);
            let data = a
                .data
                .par_iter()
                .zip(&b.data)
                .map(|(&x, &y)| {
                    let y = y * weight;
                    match op {
                        CombineOp::Add => x + y,
                        CombineOp::Subtract => x - y,
                        CombineOp::Multiply => x * y,
                        CombineOp::Max => x.max(y),
                        CombineOp::Min => x.min(y),
                        CombineOp::Average => (x + y) * 0.5,
                    }
                })
                .collect();
            Field::new(n, data)
        }
        NodeKind::Blend => {
            let (a, b, m) = (input(0)?, input(1)?, input(2)?);
            let data = (0..n * n)
                .into_par_iter()
                .map(|c| {
                    let t = m.data[c].clamp(0.0, 1.0);
                    a.data[c] + (b.data[c] - a.data[c]) * t
                })
                .collect();
            Field::new(n, data)
        }
        NodeKind::SelectHeight { min, max, falloff } => {
            let f = input(0)?;
            map(&f, &|v| band(v, min, max, falloff))
        }
        NodeKind::SelectSlope { min, max, falloff } => {
            let f = input(0)?;
            let s = e.spacing();
            let data = (0..n * n)
                .into_par_iter()
                .map(|c| {
                    let (i, j) = (c % n, c / n);
                    let gx = (f.at(i + 1, j) - f.at(i.saturating_sub(1), j)) / (2.0 * s);
                    let gz = (f.at(i, j + 1) - f.at(i, j.saturating_sub(1))) / (2.0 * s);
                    band(gx.hypot(gz).atan().to_degrees(), min, max, falloff)
                })
                .collect();
            Field::new(n, data)
        }
        NodeKind::Invert => {
            let f = input(0)?;
            map(&f, &|v| 1.0 - v)
        }
        NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } => return Ok(Vec::new()),
    };
    Ok(vec![field])
}

/// 1 inside `[min, max]`, fading to 0 over `falloff` outside.
#[inline]
fn band(v: f32, min: f32, max: f32, falloff: f32) -> f32 {
    let f = falloff.max(1e-4);
    let lo = ((v - min) / f + 0.5).clamp(0.0, 1.0);
    let hi = ((max - v) / f + 0.5).clamp(0.0, 1.0);
    lo.min(hi)
}

/// Two passes of a separable box blur of `r` samples: close to a Gaussian.
fn blur(f: &Field, r: usize) -> Field {
    let n = f.n;
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let lines: Vec<Vec<f32>> = (0..n)
            .into_par_iter()
            .map(|line| {
                let at = |t: usize| if horizontal { src[line * n + t] } else { src[t * n + line] };
                let mut out = vec![0f32; n];
                let mut sum = 0f32;
                let mut count = 0f32;
                for t in 0..n.min(r + 1) {
                    sum += at(t);
                    count += 1.0;
                }
                for (t, o) in out.iter_mut().enumerate() {
                    *o = sum / count;
                    if t + r + 1 < n {
                        sum += at(t + r + 1);
                        count += 1.0;
                    }
                    if t >= r {
                        sum -= at(t - r);
                        count -= 1.0;
                    }
                }
                out
            })
            .collect();
        let mut out = vec![0f32; n * n];
        for (line, values) in lines.into_iter().enumerate() {
            for (t, v) in values.into_iter().enumerate() {
                if horizontal {
                    out[line * n + t] = v;
                } else {
                    out[t * n + line] = v;
                }
            }
        }
        out
    };
    let mut data = f.data.clone();
    for _ in 0..2 {
        data = pass(&data, true);
        data = pass(&data, false);
    }
    Field::new(n, data)
}

struct Droplets {
    drops: usize,
    erosion: f32,
    deposition: f32,
    capacity: f32,
    evaporation: f32,
    radius: i32,
    seed: u32,
}

/// Hydraulic erosion by simulated raindrops (after Hans Beyer's method):
/// each drop runs downhill, picks up soil where it speeds up and drops it
/// where it slows down. Returns the eroded heights, the flow of water
/// (0..1) and where soil settled (0..1).
fn erode(f: &Field, spacing: f32, p: &Droplets, cancel: &AtomicBool) -> Option<(Field, Field, Field)> {
    let n = f.n;
    // Work in units of samples so that slopes are true whatever the
    // resolution.
    let mut h: Vec<f32> = f.data.iter().map(|v| v / spacing).collect();
    let mut flow = vec![0f32; n * n];
    let mut settled = vec![0f32; n * n];
    // The erosion brush.
    let r = p.radius;
    let mut brush: Vec<(i32, i32, f32)> = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            let d = ((dx * dx + dz * dz) as f32).sqrt();
            if d <= r as f32 {
                brush.push((dx, dz, 1.0 - d / (r as f32 + 1.0)));
            }
        }
    }
    let total: f32 = brush.iter().map(|b| b.2).sum();
    for b in &mut brush {
        b.2 /= total;
    }
    let inertia = 0.05f32;
    let gravity = 4.0f32;
    let min_capacity = 0.01f32;
    let max_steps = 80;
    let mut rng = (p.seed as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    let last = (n - 1) as f32;
    let height_grad = |h: &[f32], x: f32, z: f32| -> (f32, f32, f32) {
        let (i, j) = (x as usize, z as usize);
        let (u, v) = (x - i as f32, z - j as f32);
        let at = |i: usize, j: usize| h[j.min(n - 1) * n + i.min(n - 1)];
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        let gx = (b - a) * (1.0 - v) + (d - c) * v;
        let gz = (c - a) * (1.0 - u) + (d - b) * u;
        let height = a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + c * (1.0 - u) * v + d * u * v;
        (height, gx, gz)
    };
    for drop in 0..p.drops {
        if drop % 65536 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let (mut x, mut z) = (next() * last, next() * last);
        let (mut dx, mut dz) = (0f32, 0f32);
        let (mut speed, mut water, mut sediment) = (1f32, 1f32, 0f32);
        for _ in 0..max_steps {
            let (i, j) = (x as usize, z as usize);
            let (u, v) = (x - i as f32, z - j as f32);
            let (height, gx, gz) = height_grad(&h, x, z);
            dx = dx * inertia - gx * (1.0 - inertia);
            dz = dz * inertia - gz * (1.0 - inertia);
            let len = dx.hypot(dz);
            if len < 1e-6 {
                break;
            }
            dx /= len;
            dz /= len;
            x += dx;
            z += dz;
            if !(0.0..last).contains(&x) || !(0.0..last).contains(&z) {
                break;
            }
            let (new_height, _, _) = height_grad(&h, x, z);
            let dh = new_height - height;
            let cap = (-dh * speed * water * p.capacity).max(min_capacity);
            let cell = j * n + i;
            if sediment > cap || dh > 0.0 {
                // Drop soil: all it takes to fill the hole when going up,
                // part of the excess otherwise.
                let amount = if dh > 0.0 { dh.min(sediment) } else { (sediment - cap) * p.deposition };
                sediment -= amount;
                h[cell] += amount * (1.0 - u) * (1.0 - v);
                h[cell + 1] += amount * u * (1.0 - v);
                h[cell + n] += amount * (1.0 - u) * v;
                h[cell + n + 1] += amount * u * v;
                settled[cell] += amount;
            } else {
                let amount = ((cap - sediment) * p.erosion).min(-dh);
                for &(bx, bz, w) in &brush {
                    let (ci, cj) = (i as i32 + bx, j as i32 + bz);
                    if ci < 0 || cj < 0 || ci >= n as i32 || cj >= n as i32 {
                        continue;
                    }
                    h[cj as usize * n + ci as usize] -= amount * w;
                }
                sediment += amount;
            }
            flow[cell] += water;
            speed = (speed * speed + dh * gravity).max(0.0).sqrt();
            water *= 1.0 - p.evaporation;
        }
    }
    let normalise = |v: Vec<f32>| {
        let max = v.iter().copied().fold(0.0f32, f32::max).max(1e-6);
        let l = (1.0 + max).ln();
        Field::new(n, v.into_iter().map(|x| (1.0 + x.max(0.0)).ln() / l).collect())
    };
    let heights = Field::new(n, h.into_iter().map(|v| v * spacing).collect());
    Some((heights, normalise(flow), normalise(settled)))
}

/// Thermal erosion: wherever two neighbours differ by more than the talus
/// slope allows, material slides from the higher to the lower.
fn thermal(f: &Field, spacing: f32, talus: f32, iterations: u32, strength: f32) -> Field {
    let n = f.n;
    let limit = talus.clamp(1.0, 89.0).to_radians().tan() * spacing;
    let k = strength * 0.25;
    let mut h = f.data.clone();
    for _ in 0..iterations {
        let src = &h;
        let next: Vec<f32> = (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (i, j) = (c % n, c / n);
                let mine = src[c];
                let mut change = 0.0;
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ni, nj) = (i as i32 + di, j as i32 + dj);
                    if ni < 0 || nj < 0 || ni >= n as i32 || nj >= n as i32 {
                        continue;
                    }
                    let other = src[nj as usize * n + ni as usize];
                    let d = mine - other;
                    if d > limit {
                        change -= k * (d - limit);
                    } else if -d > limit {
                        change += k * (-d - limit);
                    }
                }
                mine + change
            })
            .collect();
        h = next;
    }
    Field::new(n, h)
}

// --------------------------------------------------------------- applying --

/// Write a build to the ground level: heights sampled per column from the
/// height field, terrains painted where their masks are high (dithered in
/// between, later outputs over earlier ones). Recorded for undo.
pub fn apply(edit: &mut Edit, extent: &Extent, outputs: &Outputs, seed: u64) {
    let (lo, hi) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
    let tiles: Vec<TilePos> = edit.level().positions().collect();
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    edit.par_tiles(tiles, everywhere, false, |pos, _, tile| {
        let (ox, oz) = pos.origin();
        let n = crate::coords::TILE_SIZE;
        for lz in 0..n {
            for lx in 0..n {
                let (x, z) = (ox + lx, oz + lz);
                let (u, v) = extent.grid(x as f64 + 0.5, z as f64 + 0.5);
                let i = local_index(x, z);
                if let Some(h) = &outputs.heights {
                    tile.top.set(i, snap_height(h.sample(u, v).clamp(lo, hi)));
                }
                for (k, (terrain, mask)) in outputs.terrains.iter().enumerate() {
                    let m = mask.sample(u, v);
                    let dither = hash2(x, z, seed ^ (k as u64 + 1).wrapping_mul(0x51ed)) as f32 / u32::MAX as f32;
                    if m > dither {
                        tile.terrain.set(i, terrain.0);
                    }
                }
            }
        }
        tile.compact();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(n: usize) -> Extent {
        Extent {
            x0: -512.0,
            z0: -512.0,
            size: 1024.0,
            n,
        }
    }

    fn eval_one(g: &Graph, id: NodeId, n: usize) -> Arc<Field> {
        let e = g.evaluate(&extent(n), &[id], true, &AtomicBool::new(false)).unwrap();
        e.fields[&(id, 0)].clone()
    }

    #[test]
    fn generators_stay_in_their_range_and_are_deterministic() {
        for kind in NodeKind::catalogue().into_iter().filter(|k| k.family() == Family::Generator) {
            let mut g = Graph::default();
            let id = g.add(kind.clone(), [0.0; 2]);
            let f = eval_one(&g, id, 64);
            let (lo, hi) = f.range();
            let (base, height) = match kind {
                NodeKind::Noise { base, height, .. } | NodeKind::Cells { base, height, .. } | NodeKind::Radial { base, height, .. } => {
                    (base, height)
                }
                NodeKind::Constant { value } => (value, 0.0),
                _ => unreachable!(),
            };
            assert!(lo >= base - 1e-3 && hi <= base + height + 1e-3, "{}: {lo}..{hi}", kind.label());
            assert_eq!(eval_one(&g, id, 64).data, f.data, "{}", kind.label());
        }
    }

    #[test]
    fn the_same_graph_looks_the_same_at_every_resolution() {
        let mut g = Graph::default();
        let id = g.add(NodeKind::catalogue()[0].clone(), [0.0; 2]);
        let coarse = eval_one(&g, id, 64);
        let fine = eval_one(&g, id, 256);
        // Sample (10, 20) of the coarse grid is sample (41.5, 81.5) of the fine one.
        let (x, z) = extent(64).world(10, 20);
        let (u, v) = extent(256).grid(x, z);
        assert!((coarse.at(10, 20) - fine.sample(u, v)).abs() < 1.0);
    }

    #[test]
    fn links_refuse_loops_and_missing_inputs_are_reported() {
        let mut g = Graph::default();
        let a = g.add(NodeKind::Constant { value: 1.0 }, [0.0; 2]);
        let b = g.add(NodeKind::Blur { radius: 4.0 }, [0.0; 2]);
        let c = g.add(NodeKind::Blur { radius: 4.0 }, [0.0; 2]);
        assert!(g.connect(a, 0, b, 0));
        assert!(g.connect(b, 0, c, 0));
        assert!(!g.connect(c, 0, b, 0), "a loop");
        assert!(g.depends_on(c, a));
        let lone = g.add(NodeKind::Invert, [0.0; 2]);
        let err = g.evaluate(&extent(16), &[lone], true, &AtomicBool::new(false)).unwrap_err();
        assert_eq!(err, GraphError::MissingInput("Invert".into()));
        // Reconnecting an input replaces the old link.
        let d = g.add(NodeKind::Constant { value: 5.0 }, [0.0; 2]);
        assert!(g.connect(d, 0, c, 0));
        assert_eq!(g.input_of(c, 0), Some((d, 0)));
        g.remove(d);
        assert_eq!(g.input_of(c, 0), None);
    }

    #[test]
    fn combiners_selectors_and_modifiers() {
        let mut g = Graph::default();
        let a = g.add(NodeKind::Constant { value: 10.0 }, [0.0; 2]);
        let b = g.add(NodeKind::Constant { value: 4.0 }, [0.0; 2]);
        for (op, want) in [
            (CombineOp::Add, 14.0),
            (CombineOp::Subtract, 6.0),
            (CombineOp::Multiply, 40.0),
            (CombineOp::Max, 10.0),
            (CombineOp::Min, 4.0),
            (CombineOp::Average, 7.0),
        ] {
            let c = g.add(NodeKind::Combine { op, weight: 1.0 }, [0.0; 2]);
            g.connect(a, 0, c, 0);
            g.connect(b, 0, c, 1);
            assert_eq!(eval_one(&g, c, 8).data[0], want, "{op:?}");
        }
        let sel = g.add(
            NodeKind::SelectHeight {
                min: 5.0,
                max: 20.0,
                falloff: 1.0,
            },
            [0.0; 2],
        );
        g.connect(a, 0, sel, 0);
        assert_eq!(eval_one(&g, sel, 8).data[0], 1.0);
        g.connect(b, 0, sel, 0);
        assert_eq!(eval_one(&g, sel, 8).data[0], 0.0);
        let levels = g.add(
            NodeKind::Levels {
                in_min: 0.0,
                in_max: 20.0,
                out_min: 100.0,
                out_max: 200.0,
                gamma: 1.0,
            },
            [0.0; 2],
        );
        g.connect(a, 0, levels, 0);
        assert_eq!(eval_one(&g, levels, 8).data[0], 150.0);
        // A slope: steep where a radial gradient falls.
        let radial = g.add(
            NodeKind::Radial {
                inner: 0.2,
                outer: 0.6,
                height: 300.0,
                base: 0.0,
            },
            [0.0; 2],
        );
        let steep = g.add(
            NodeKind::SelectSlope {
                min: 20.0,
                max: 90.0,
                falloff: 1.0,
            },
            [0.0; 2],
        );
        g.connect(radial, 0, steep, 0);
        let f = eval_one(&g, steep, 64);
        assert_eq!(f.at(32, 32), 0.0, "flat top");
        assert_eq!(f.at(0, 0), 0.0, "flat plain");
        assert!(f.at(32 + 13, 32) > 0.9, "the flank");
    }

    #[test]
    fn erosion_carves_and_settles_and_thermal_limits_slopes() {
        let mut g = Graph::default();
        let hills = g.add(
            NodeKind::Noise {
                style: NoiseStyle::Ridged,
                scale: 400.0,
                octaves: 5,
                roughness: 0.5,
                height: 120.0,
                base: 40.0,
                seed: 9,
            },
            [0.0; 2],
        );
        let eroded = g.add(
            NodeKind::Erosion {
                rain: 2.0,
                erosion: 0.3,
                deposition: 0.3,
                capacity: 4.0,
                evaporation: 0.02,
                radius: 12.0,
                seed: 1,
            },
            [0.0; 2],
        );
        g.connect(hills, 0, eroded, 0);
        let e = g.evaluate(&extent(128), &[eroded], true, &AtomicBool::new(false)).unwrap();
        let before = &e.fields[&(hills, 0)];
        let after = &e.fields[&(eroded, 0)];
        let flow = &e.fields[&(eroded, 1)];
        let deposits = &e.fields[&(eroded, 2)];
        let changed = before.data.iter().zip(&after.data).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
        assert!(changed > 128 * 128 / 4, "erosion changed only {changed} samples");
        // Peaks wear down: the highest point is lower.
        assert!(after.range().1 < before.range().1);
        let (flo, fhi) = flow.range();
        assert!(flo >= 0.0 && fhi <= 1.0 && fhi > 0.9);
        assert!(deposits.range().1 > 0.5);
        // Water gathers in valleys: more flow where it is lower.
        let mean = |pick: &dyn Fn(f32) -> bool| {
            let v: Vec<f32> = (0..128 * 128).filter(|&c| pick(after.data[c])).map(|c| flow.data[c]).collect();
            v.iter().sum::<f32>() / v.len().max(1) as f32
        };
        let (lo, hi) = after.range();
        let mid = (lo + hi) / 2.0;
        assert!(mean(&|h| h < mid) > mean(&|h| h > mid));

        // Thermal erosion: a cliff becomes a slope no steeper than the talus.
        let step = g.add(
            NodeKind::Radial {
                inner: 0.5,
                outer: 0.505,
                height: 100.0,
                base: 0.0,
            },
            [0.0; 2],
        );
        let slid = g.add(
            NodeKind::Thermal {
                talus: 45.0,
                iterations: 400,
                strength: 1.0,
            },
            [0.0; 2],
        );
        g.connect(step, 0, slid, 0);
        let f = eval_one(&g, slid, 64);
        let spacing = extent(64).spacing();
        let steepest = (1..64)
            .map(|i| (f.at(i, 32) - f.at(i - 1, 32)).abs() / spacing)
            .fold(0.0f32, f32::max);
        assert!(steepest < 1.6, "steepest slope {steepest}");
    }

    #[test]
    fn the_starter_graph_builds_and_applies() {
        use crate::generate::{GenParams, Landform, generate};
        use crate::history::History;
        let mut world = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Flat,
            ..GenParams::default()
        });
        let reg = &world.registry;
        let (rock, snow, gravel) = (
            TerrainId::ROCK.0,
            reg.terrain_by_name("Snow").unwrap().0,
            reg.terrain_by_name("Gravel").unwrap().0,
        );
        let grass = reg.terrain_by_name("Grass").unwrap().0;
        let g = Graph::starter(grass, rock, snow, gravel);
        let rect = BlockRect::new(-256, -256, 256, 256);
        let ext = Extent::covering(rect, 128);
        let outputs = g.build(&ext, &AtomicBool::new(false)).unwrap();
        assert!(outputs.heights.is_some());
        assert_eq!(outputs.terrains.len(), 4);
        let mut h = History::default();
        h.begin("graph", 0);
        apply(&mut h.edit(&mut world, 0), &ext, &outputs, 1);
        h.commit();
        let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        let heights: Vec<f32> = (0..128 * 128).map(|i| tile.top.get(i)).collect();
        let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 20.0, "the flat world got relief: {lo}..{hi}");
        // Undo puts the flat world back.
        h.undo(&mut world);
        let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!(tile.top.get(0), 64.0);
        // A graph survives serialisation (it is saved with the editor).
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Graph>(&json).unwrap(), g);
        // Cancelling stops a build.
        assert_eq!(g.build(&ext, &AtomicBool::new(true)).unwrap_err(), GraphError::Cancelled);
    }
}
