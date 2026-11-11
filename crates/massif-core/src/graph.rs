//! Terrain graphs: procedural terrain from a graph of nodes, in the manner of
//! Gaea or World Machine.
//!
//! Every node makes *fields* — square grids of values over the whole map —
//! or *colour fields*. A field holds heights in blocks, or a mask from 0 to
//! 1; a colour field the colour of the ground. Primitives and landforms make
//! fields from nothing (noise, cells, mountains, volcanoes, craters, dunes,
//! canyons…), adjustments reshape one (levels, curves, terraces, strata,
//! warps…), simulations wear it the way nature does (water erosion, thermal
//! erosion, snow, rivers, lakes), combiners mix several, masks pick parts of
//! the relief (by height, slope, curvature, facing, occlusion, flow), colour
//! nodes paint it (gradients from nature, automatic colouring, mixes), and
//! outputs say what goes to the map: the height of the ground, where each
//! terrain is painted, and the colour the ground takes, which becomes the
//! nearest real Minecraft blocks.
//!
//! A graph is evaluated at a resolution chosen for the job (a few hundred
//! samples per side for previews, thousands for the final build) over the
//! extent of the map, and sampled per block column when it is applied.
//! Distances are in blocks, so a graph looks the same at every resolution.
//! Evaluations can keep a cache: only the nodes that changed, and those
//! after them, are computed again.

mod color;
mod landforms;
mod ops;
mod sim;

pub use color::{AutoPalette, BlockPalette, GradientPreset, decode as decode_srgb, encode as encode_srgb};

use crate::codec::snap_height;
use crate::compose::hash2;
use crate::coords::{BlockRect, TilePos, local_index};
use crate::history::Edit;
use crate::registry::TerrainId;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
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
    /// The first input, moved toward the second by the weight.
    Lerp,
    /// Brighter wherever either is: 1 − (1 − a)(1 − b), for masks.
    Screen,
    /// Contrast: multiply the low end, screen the high end, for masks.
    Overlay,
    /// How far apart they are.
    Difference,
}

impl CombineOp {
    pub const ALL: [CombineOp; 10] = [
        CombineOp::Add,
        CombineOp::Subtract,
        CombineOp::Multiply,
        CombineOp::Max,
        CombineOp::Min,
        CombineOp::Average,
        CombineOp::Lerp,
        CombineOp::Screen,
        CombineOp::Overlay,
        CombineOp::Difference,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            CombineOp::Add => "Add",
            CombineOp::Subtract => "Subtract",
            CombineOp::Multiply => "Multiply",
            CombineOp::Max => "Max",
            CombineOp::Min => "Min",
            CombineOp::Average => "Average",
            CombineOp::Lerp => "Lerp",
            CombineOp::Screen => "Screen",
            CombineOp::Overlay => "Overlay",
            CombineOp::Difference => "Difference",
        }
    }
}

/// The character of a mountain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountainStyle {
    /// Sharp ridges and deep gullies.
    Alpine,
    /// Soft, old, rounded.
    Rounded,
    /// Worn by water: branching valleys.
    Eroded,
    /// A narrow, steep peak.
    Spiky,
}

impl MountainStyle {
    pub const ALL: [MountainStyle; 4] = [MountainStyle::Alpine, MountainStyle::Rounded, MountainStyle::Eroded, MountainStyle::Spiky];

    pub const fn label(self) -> &'static str {
        match self {
            MountainStyle::Alpine => "Alpine",
            MountainStyle::Rounded => "Rounded",
            MountainStyle::Eroded => "Eroded",
            MountainStyle::Spiky => "Spiky",
        }
    }
}

/// Simple shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Circle,
    Square,
    Cone,
    Dome,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 4] = [ShapeKind::Circle, ShapeKind::Square, ShapeKind::Cone, ShapeKind::Dome];

    pub const fn label(self) -> &'static str {
        match self {
            ShapeKind::Circle => "Circle",
            ShapeKind::Square => "Square",
            ShapeKind::Cone => "Cone",
            ShapeKind::Dome => "Dome",
        }
    }
}

/// Which bends of the ground a curvature mask picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bend {
    /// Ridges, crests, edges of cliffs.
    Convex,
    /// Valleys, hollows, feet of slopes.
    Concave,
}

/// How a tint is laid over a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TintMode {
    Mix,
    Multiply,
    Screen,
}

impl TintMode {
    pub const ALL: [TintMode; 3] = [TintMode::Mix, TintMode::Multiply, TintMode::Screen];

    pub const fn label(self) -> &'static str {
        match self {
            TintMode::Mix => "Mix",
            TintMode::Multiply => "Multiply",
            TintMode::Screen => "Screen",
        }
    }
}

/// What a node does, with its settings. Positions (`x`, `z`) are fractions
/// of the map (0.5: its middle); sizes and heights are in blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    // ---- primitives
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
    /// A ramp across the map.
    LinearGradient {
        /// Direction it rises toward, in degrees (0: north, 90: east).
        angle: f32,
        height: f32,
        base: f32,
    },
    /// One shape somewhere on the map.
    Shape {
        shape: ShapeKind,
        x: f32,
        z: f32,
        radius: f32,
        /// Softness of the edge, as a fraction of the radius.
        falloff: f32,
        height: f32,
        base: f32,
    },
    // ---- landforms
    Mountain {
        style: MountainStyle,
        x: f32,
        z: f32,
        radius: f32,
        height: f32,
        base: f32,
        /// Fine detail on the flanks (0..1).
        detail: f32,
        seed: u32,
    },
    /// A chain of ridged peaks across the map.
    MountainRange {
        /// Direction the range runs, in degrees.
        angle: f32,
        /// How wide the range is, in blocks.
        width: f32,
        /// Size of its peaks and valleys.
        scale: f32,
        height: f32,
        base: f32,
        seed: u32,
    },
    Volcano {
        x: f32,
        z: f32,
        radius: f32,
        height: f32,
        base: f32,
        /// Width of the crater, as a fraction of the radius.
        crater: f32,
        /// Depth of the crater below the rim, in blocks.
        depth: f32,
        seed: u32,
    },
    Crater {
        x: f32,
        z: f32,
        radius: f32,
        depth: f32,
        /// Height of the rim above the plain.
        rim: f32,
        /// Height of a central peak (0: none).
        peak: f32,
        base: f32,
        seed: u32,
    },
    /// Craters of all sizes over the whole map, the way meteorites leave
    /// them on a moon.
    CraterField {
        count: u32,
        min_radius: f32,
        max_radius: f32,
        /// Depth of a crater as a fraction of its radius.
        depth: f32,
        base: f32,
        seed: u32,
    },
    Dunes {
        /// Direction the wind blows toward, in degrees.
        angle: f32,
        /// Distance between crests, in blocks.
        wavelength: f32,
        height: f32,
        base: f32,
        /// How steep the lee side is (0..1).
        sharpness: f32,
        seed: u32,
    },
    /// A winding river canyon cut into a plateau, with stepped walls.
    Canyon {
        angle: f32,
        width: f32,
        depth: f32,
        /// How far it winds, in blocks.
        meander: f32,
        /// Steps of its walls (0: smooth).
        steps: u32,
        /// Height of the plateau.
        base: f32,
        seed: u32,
    },
    /// Flat-topped hills with steep, stepped sides.
    Mesas {
        scale: f32,
        /// How much of the map they cover (0..1).
        coverage: f32,
        height: f32,
        base: f32,
        steps: u32,
        seed: u32,
    },
    Island {
        /// Size, as a fraction of the map.
        size: f32,
        height: f32,
        /// Height of the sea floor around it.
        base: f32,
        /// How ragged the coast is (0..1).
        coast: f32,
        seed: u32,
    },
    /// Rocky detail around 0, to add to a relief.
    Rocks {
        scale: f32,
        height: f32,
        /// 0: rounded boulders, 1: sharp crags.
        sharpness: f32,
        seed: u32,
    },
    // ---- adjustments
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
    /// Stretch the field to a range of heights.
    Normalize {
        min: f32,
        max: f32,
    },
    /// Reshape the heights: a gamma and an S-curve, over the field's range.
    Curve {
        /// Above 1: lower ground widens; below 1: higher ground does.
        gamma: f32,
        /// −1 (flatter middle) to 1 (steeper middle).
        contrast: f32,
    },
    /// Scale, turn and move the field.
    Transform {
        scale: f32,
        /// Degrees.
        rotate: f32,
        /// Fractions of the map.
        offset_x: f32,
        offset_z: f32,
    },
    /// Twist around the middle of the map.
    Swirl {
        /// Degrees of turn at the middle.
        angle: f32,
        /// Fraction of the map it reaches.
        radius: f32,
    },
    /// Mirror what rises over `level` back down: ridges and folds.
    Fold {
        level: f32,
        strength: f32,
    },
    /// Layers of rock of uneven thickness: steps on the slopes.
    Stratify {
        layers: u32,
        strength: f32,
        /// Tilt of the layers, in degrees.
        tilt: f32,
        seed: u32,
    },
    Sharpen {
        radius: f32,
        amount: f32,
    },
    /// Flip the heights upside down (within their range).
    InvertHeight,
    // ---- simulations
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
    /// Snow settles above a line, on slopes gentle enough to hold it.
    Snow {
        /// Height of the snow line.
        line: f32,
        /// Steepest slope that holds snow, in degrees.
        slope: f32,
        /// Depth of the snow, in blocks.
        depth: f32,
        /// How gradually the snow line fades, in blocks.
        softness: f32,
    },
    /// Rivers dug where the water of the whole map gathers.
    Rivers {
        /// How much water makes a river (0..1: more, fewer rivers).
        threshold: f32,
        depth: f32,
        width: f32,
    },
    /// Fill the hollows water would fill: lakes and flat valley floors.
    Lakes {
        /// 0: nothing, 1: every hollow full.
        amount: f32,
    },
    /// A sea level: where the ground is under water, and its shore.
    Sea {
        level: f32,
        /// Width of the shore, in blocks of height around the level.
        shore: f32,
    },
    // ---- combining
    Combine {
        op: CombineOp,
        /// Weight of the second input.
        weight: f32,
    },
    /// The first input where the mask is 0, the second where it is 1.
    Blend,
    // ---- masks
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
    Curvature {
        bend: Bend,
        /// Size of the bends looked at, in blocks.
        radius: f32,
        strength: f32,
    },
    /// Slopes facing a direction (snow on the shady side, moss…).
    Facing {
        /// Degrees: 0 north, 90 east.
        angle: f32,
        /// How far around it, in degrees.
        spread: f32,
    },
    /// How open to the sky the ground is: dark in hollows and crevices.
    Occlusion {
        radius: f32,
    },
    /// Where the water of the map runs and gathers.
    Flow {
        /// 0..1: how much small rills show next to big rivers.
        spread: f32,
    },
    // ---- colour
    /// Colour a field with a gradient taken from nature.
    Gradient {
        preset: GradientPreset,
        /// Use the field's own range rather than `min`..`max`.
        auto_range: bool,
        min: f32,
        max: f32,
        reverse: bool,
        /// Breaks the bands up a little (0..1).
        variation: f32,
        seed: u32,
    },
    SolidColor {
        color: [u8; 3],
    },
    /// The first colour where the mask is 0, the second where it is 1.
    MixColors,
    AdjustColor {
        /// Degrees around the colour wheel.
        hue: f32,
        saturation: f32,
        brightness: f32,
        contrast: f32,
    },
    /// Lay a colour over another where a mask says.
    Tint {
        color: [u8; 3],
        mode: TintMode,
        strength: f32,
    },
    /// Natural colours from the relief alone: valleys, meadows, rock on the
    /// steep slopes, snow on the peaks.
    AutoColor {
        palette: AutoPalette,
        /// Where snow starts, in blocks.
        snow_line: f32,
        /// Slope where rock shows, in degrees.
        rock_slope: f32,
        seed: u32,
    },
    // ---- outputs
    /// The height of the ground.
    HeightOutput,
    /// Paint a terrain where the mask is high (dithered in between).
    TerrainOutput {
        terrain: u16,
    },
    /// The colour of the ground: shown in the preview, and, if `blocks`,
    /// turned into the real blocks of the game closest to it.
    ColorOutput {
        blocks: bool,
        palette: BlockPalette,
    },
}

/// What flows along a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortType {
    /// Heights, in blocks.
    Height,
    /// 0 to 1.
    Mask,
    /// A colour per sample.
    Color,
}

/// Families of nodes, for the editor's menus and colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Generator,
    Landform,
    Modifier,
    Erosion,
    Combiner,
    Selector,
    Color,
    Output,
}

impl Family {
    pub const ALL: [Family; 8] = [
        Family::Landform,
        Family::Generator,
        Family::Modifier,
        Family::Erosion,
        Family::Combiner,
        Family::Selector,
        Family::Color,
        Family::Output,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Family::Generator => "Primitives",
            Family::Landform => "Landforms",
            Family::Modifier => "Adjust",
            Family::Erosion => "Simulate",
            Family::Combiner => "Combine",
            Family::Selector => "Masks",
            Family::Color => "Colour",
            Family::Output => "Output",
        }
    }
}

use PortType::{Color as C, Height as H, Mask as M};

impl NodeKind {
    /// A node of each kind with sensible settings, for the "add" menu.
    pub fn catalogue() -> Vec<NodeKind> {
        vec![
            // Landforms.
            NodeKind::Mountain {
                style: MountainStyle::Alpine,
                x: 0.5,
                z: 0.5,
                radius: 900.0,
                height: 220.0,
                base: 40.0,
                detail: 0.5,
                seed: 1,
            },
            NodeKind::MountainRange {
                angle: 30.0,
                width: 1200.0,
                scale: 700.0,
                height: 200.0,
                base: 40.0,
                seed: 2,
            },
            NodeKind::Volcano {
                x: 0.5,
                z: 0.5,
                radius: 800.0,
                height: 200.0,
                base: 40.0,
                crater: 0.18,
                depth: 40.0,
                seed: 3,
            },
            NodeKind::Crater {
                x: 0.5,
                z: 0.5,
                radius: 400.0,
                depth: 50.0,
                rim: 20.0,
                peak: 0.0,
                base: 80.0,
                seed: 4,
            },
            NodeKind::CraterField {
                count: 160,
                min_radius: 20.0,
                max_radius: 450.0,
                depth: 0.25,
                base: 90.0,
                seed: 5,
            },
            NodeKind::Dunes {
                angle: 60.0,
                wavelength: 120.0,
                height: 20.0,
                base: 64.0,
                sharpness: 0.7,
                seed: 6,
            },
            NodeKind::Canyon {
                angle: 20.0,
                width: 500.0,
                depth: 90.0,
                meander: 300.0,
                steps: 6,
                base: 150.0,
                seed: 7,
            },
            NodeKind::Mesas {
                scale: 500.0,
                coverage: 0.4,
                height: 60.0,
                base: 70.0,
                steps: 3,
                seed: 8,
            },
            NodeKind::Island {
                size: 0.75,
                height: 140.0,
                base: 20.0,
                coast: 0.6,
                seed: 9,
            },
            NodeKind::Rocks {
                scale: 90.0,
                height: 12.0,
                sharpness: 0.6,
                seed: 10,
            },
            // Primitives.
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
            NodeKind::Noise {
                style: NoiseStyle::Billow,
                scale: 500.0,
                octaves: 5,
                roughness: 0.5,
                height: 60.0,
                base: 50.0,
                seed: 3,
            },
            NodeKind::Cells {
                style: CellStyle::Cones,
                scale: 500.0,
                jitter: 0.8,
                height: 100.0,
                base: 50.0,
                seed: 3,
            },
            NodeKind::Cells {
                style: CellStyle::Ridges,
                scale: 500.0,
                jitter: 0.8,
                height: 100.0,
                base: 50.0,
                seed: 4,
            },
            NodeKind::Radial {
                inner: 0.3,
                outer: 0.95,
                height: 1.0,
                base: 0.0,
            },
            NodeKind::LinearGradient {
                angle: 90.0,
                height: 100.0,
                base: 40.0,
            },
            NodeKind::Shape {
                shape: ShapeKind::Dome,
                x: 0.5,
                z: 0.5,
                radius: 600.0,
                falloff: 0.5,
                height: 1.0,
                base: 0.0,
            },
            NodeKind::Constant { value: 64.0 },
            // Adjustments.
            NodeKind::Levels {
                in_min: 0.0,
                in_max: 255.0,
                out_min: 0.0,
                out_max: 255.0,
                gamma: 1.0,
            },
            NodeKind::Normalize { min: 40.0, max: 260.0 },
            NodeKind::Curve { gamma: 1.4, contrast: 0.2 },
            NodeKind::Terrace {
                steps: 8,
                strength: 0.7,
            },
            NodeKind::Stratify {
                layers: 14,
                strength: 0.7,
                tilt: 6.0,
                seed: 5,
            },
            NodeKind::Clamp { min: -64.0, max: 319.0 },
            NodeKind::Blur { radius: 8.0 },
            NodeKind::Sharpen { radius: 24.0, amount: 0.8 },
            NodeKind::Warp {
                amount: 60.0,
                scale: 400.0,
                seed: 4,
            },
            NodeKind::Transform {
                scale: 1.0,
                rotate: 0.0,
                offset_x: 0.0,
                offset_z: 0.0,
            },
            NodeKind::Swirl { angle: 120.0, radius: 0.5 },
            NodeKind::Fold { level: 120.0, strength: 1.0 },
            NodeKind::InvertHeight,
            // Simulations.
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
            NodeKind::Snow {
                line: 190.0,
                slope: 40.0,
                depth: 3.0,
                softness: 20.0,
            },
            NodeKind::Rivers {
                threshold: 0.55,
                depth: 6.0,
                width: 12.0,
            },
            NodeKind::Lakes { amount: 1.0 },
            NodeKind::Sea { level: 62.0, shore: 4.0 },
            // Combining.
            NodeKind::Combine {
                op: CombineOp::Add,
                weight: 1.0,
            },
            NodeKind::Blend,
            // Masks.
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
            NodeKind::Curvature {
                bend: Bend::Concave,
                radius: 24.0,
                strength: 1.0,
            },
            NodeKind::Facing { angle: 0.0, spread: 60.0 },
            NodeKind::Occlusion { radius: 32.0 },
            NodeKind::Flow { spread: 0.5 },
            NodeKind::Invert,
            // Colour.
            NodeKind::AutoColor {
                palette: AutoPalette::Temperate,
                snow_line: 200.0,
                rock_slope: 38.0,
                seed: 1,
            },
            NodeKind::Gradient {
                preset: GradientPreset::Alpine,
                auto_range: true,
                min: 0.0,
                max: 255.0,
                reverse: false,
                variation: 0.15,
                seed: 1,
            },
            NodeKind::SolidColor { color: [124, 150, 92] },
            NodeKind::MixColors,
            NodeKind::Tint {
                color: [70, 60, 50],
                mode: TintMode::Multiply,
                strength: 0.5,
            },
            NodeKind::AdjustColor {
                hue: 0.0,
                saturation: 1.0,
                brightness: 1.0,
                contrast: 1.0,
            },
            // Outputs.
            NodeKind::HeightOutput,
            NodeKind::TerrainOutput { terrain: 0 },
            NodeKind::ColorOutput {
                blocks: true,
                palette: BlockPalette::Natural,
            },
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Noise { style: NoiseStyle::Fbm, .. } => "Noise",
            NodeKind::Noise { style: NoiseStyle::Ridged, .. } => "Ridged noise",
            NodeKind::Noise { style: NoiseStyle::Billow, .. } => "Billow noise",
            NodeKind::Cells { style: CellStyle::Cones, .. } => "Cells",
            NodeKind::Cells { style: CellStyle::Ridges, .. } => "Cell ridges",
            NodeKind::Radial { .. } => "Radial gradient",
            NodeKind::Constant { .. } => "Constant",
            NodeKind::LinearGradient { .. } => "Linear gradient",
            NodeKind::Shape { .. } => "Shape",
            NodeKind::Mountain { .. } => "Mountain",
            NodeKind::MountainRange { .. } => "Mountain range",
            NodeKind::Volcano { .. } => "Volcano",
            NodeKind::Crater { .. } => "Crater",
            NodeKind::CraterField { .. } => "Crater field",
            NodeKind::Dunes { .. } => "Dunes",
            NodeKind::Canyon { .. } => "Canyon",
            NodeKind::Mesas { .. } => "Mesas",
            NodeKind::Island { .. } => "Island",
            NodeKind::Rocks { .. } => "Rocks",
            NodeKind::Levels { .. } => "Levels",
            NodeKind::Terrace { .. } => "Terraces",
            NodeKind::Clamp { .. } => "Clamp",
            NodeKind::Blur { .. } => "Blur",
            NodeKind::Warp { .. } => "Warp",
            NodeKind::Normalize { .. } => "Normalize",
            NodeKind::Curve { .. } => "Curve",
            NodeKind::Transform { .. } => "Transform",
            NodeKind::Swirl { .. } => "Swirl",
            NodeKind::Fold { .. } => "Fold",
            NodeKind::Stratify { .. } => "Stratify",
            NodeKind::Sharpen { .. } => "Sharpen",
            NodeKind::InvertHeight => "Invert height",
            NodeKind::Erosion { .. } => "Erosion",
            NodeKind::Thermal { .. } => "Thermal erosion",
            NodeKind::Snow { .. } => "Snow",
            NodeKind::Rivers { .. } => "Rivers",
            NodeKind::Lakes { .. } => "Lakes",
            NodeKind::Sea { .. } => "Sea",
            NodeKind::Combine { .. } => "Combine",
            NodeKind::Blend => "Blend",
            NodeKind::SelectHeight { .. } => "Select height",
            NodeKind::SelectSlope { .. } => "Select slope",
            NodeKind::Invert => "Invert",
            NodeKind::Curvature { .. } => "Curvature",
            NodeKind::Facing { .. } => "Facing",
            NodeKind::Occlusion { .. } => "Occlusion",
            NodeKind::Flow { .. } => "Flow",
            NodeKind::Gradient { .. } => "Gradient",
            NodeKind::SolidColor { .. } => "Solid colour",
            NodeKind::MixColors => "Mix colours",
            NodeKind::AdjustColor { .. } => "Adjust colour",
            NodeKind::Tint { .. } => "Tint",
            NodeKind::AutoColor { .. } => "Auto colour",
            NodeKind::HeightOutput => "Height output",
            NodeKind::TerrainOutput { .. } => "Terrain output",
            NodeKind::ColorOutput { .. } => "Colour output",
        }
    }

    /// One line on what the node does, for the library and tooltips.
    pub fn description(&self) -> &'static str {
        match self {
            NodeKind::Noise { .. } => "Fractal noise: rolling, ridged or billowy relief at every scale",
            NodeKind::Cells { .. } => "Voronoi cells: cones at their centres or ridges along their edges",
            NodeKind::Radial { .. } => "High in the middle of the map, falling to its edges",
            NodeKind::Constant { .. } => "One value everywhere",
            NodeKind::LinearGradient { .. } => "A ramp rising across the map",
            NodeKind::Shape { .. } => "A circle, square, cone or dome",
            NodeKind::Mountain { .. } => "One mountain, alpine, rounded, eroded or spiky",
            NodeKind::MountainRange { .. } => "A chain of ridged peaks across the map",
            NodeKind::Volcano { .. } => "A cone with a crater and gullies down its flanks",
            NodeKind::Crater { .. } => "One impact crater: bowl, rim and central peak",
            NodeKind::CraterField { .. } => "Craters of every size over the whole map",
            NodeKind::Dunes { .. } => "Sand dunes shaped by the wind",
            NodeKind::Canyon { .. } => "A winding canyon cut into a plateau",
            NodeKind::Mesas { .. } => "Flat-topped hills with steep, stepped sides",
            NodeKind::Island { .. } => "An island rising from the sea floor",
            NodeKind::Rocks { .. } => "Rocky detail to add to a relief",
            NodeKind::Levels { .. } => "Map one range of heights onto another",
            NodeKind::Terrace { .. } => "Flat steps with steep risers",
            NodeKind::Clamp { .. } => "Keep heights between two limits",
            NodeKind::Blur { .. } => "Smooth the field",
            NodeKind::Warp { .. } => "Push the field around with noise",
            NodeKind::Normalize { .. } => "Stretch the field to a range of heights",
            NodeKind::Curve { .. } => "Reshape heights with a gamma and an S-curve",
            NodeKind::Transform { .. } => "Scale, turn and move the field",
            NodeKind::Swirl { .. } => "Twist the field around the middle",
            NodeKind::Fold { .. } => "Fold heights over a level: ridges",
            NodeKind::Stratify { .. } => "Rock layers of uneven thickness",
            NodeKind::Sharpen { .. } => "Bring out the detail",
            NodeKind::InvertHeight => "Valleys become ridges and ridges valleys",
            NodeKind::Erosion { .. } => "Rain wears the slopes: gullies, fans, flow and deposits",
            NodeKind::Thermal { .. } => "Loose rock slides down steep slopes",
            NodeKind::Snow { .. } => "Snow above a line, on slopes that hold it",
            NodeKind::Rivers { .. } => "Rivers dug where the water gathers",
            NodeKind::Lakes { .. } => "Water fills the hollows: lakes, flat valley floors",
            NodeKind::Sea { .. } => "A sea level, with masks of the water and the shore",
            NodeKind::Combine { .. } => "Add, subtract, multiply… two fields",
            NodeKind::Blend => "Two fields mixed by a mask",
            NodeKind::SelectHeight { .. } => "Mask of a band of heights",
            NodeKind::SelectSlope { .. } => "Mask of a band of slopes",
            NodeKind::Invert => "1 − the mask",
            NodeKind::Curvature { .. } => "Mask of ridges or of hollows",
            NodeKind::Facing { .. } => "Mask of slopes facing a direction",
            NodeKind::Occlusion { .. } => "How open to the sky the ground is",
            NodeKind::Flow { .. } => "Where water runs and gathers",
            NodeKind::Gradient { .. } => "Colour a field with a gradient from nature",
            NodeKind::SolidColor { .. } => "One colour everywhere",
            NodeKind::MixColors => "Two colours mixed by a mask",
            NodeKind::AdjustColor { .. } => "Hue, saturation, brightness and contrast",
            NodeKind::Tint { .. } => "Lay a colour where a mask says",
            NodeKind::AutoColor { .. } => "Natural colours from the relief alone",
            NodeKind::HeightOutput => "The height of the ground of the map",
            NodeKind::TerrainOutput { .. } => "Paint a terrain where the mask says",
            NodeKind::ColorOutput { .. } => "The colour of the ground, turned into real blocks",
        }
    }

    pub fn family(&self) -> Family {
        match self {
            NodeKind::Noise { .. }
            | NodeKind::Cells { .. }
            | NodeKind::Radial { .. }
            | NodeKind::Constant { .. }
            | NodeKind::LinearGradient { .. }
            | NodeKind::Shape { .. } => Family::Generator,
            NodeKind::Mountain { .. }
            | NodeKind::MountainRange { .. }
            | NodeKind::Volcano { .. }
            | NodeKind::Crater { .. }
            | NodeKind::CraterField { .. }
            | NodeKind::Dunes { .. }
            | NodeKind::Canyon { .. }
            | NodeKind::Mesas { .. }
            | NodeKind::Island { .. }
            | NodeKind::Rocks { .. } => Family::Landform,
            NodeKind::Levels { .. }
            | NodeKind::Terrace { .. }
            | NodeKind::Clamp { .. }
            | NodeKind::Blur { .. }
            | NodeKind::Warp { .. }
            | NodeKind::Normalize { .. }
            | NodeKind::Curve { .. }
            | NodeKind::Transform { .. }
            | NodeKind::Swirl { .. }
            | NodeKind::Fold { .. }
            | NodeKind::Stratify { .. }
            | NodeKind::Sharpen { .. }
            | NodeKind::InvertHeight => Family::Modifier,
            NodeKind::Erosion { .. }
            | NodeKind::Thermal { .. }
            | NodeKind::Snow { .. }
            | NodeKind::Rivers { .. }
            | NodeKind::Lakes { .. }
            | NodeKind::Sea { .. } => Family::Erosion,
            NodeKind::Combine { .. } | NodeKind::Blend => Family::Combiner,
            NodeKind::SelectHeight { .. }
            | NodeKind::SelectSlope { .. }
            | NodeKind::Invert
            | NodeKind::Curvature { .. }
            | NodeKind::Facing { .. }
            | NodeKind::Occlusion { .. }
            | NodeKind::Flow { .. } => Family::Selector,
            NodeKind::Gradient { .. }
            | NodeKind::SolidColor { .. }
            | NodeKind::MixColors
            | NodeKind::AdjustColor { .. }
            | NodeKind::Tint { .. }
            | NodeKind::AutoColor { .. } => Family::Color,
            NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } | NodeKind::ColorOutput { .. } => Family::Output,
        }
    }

    /// Names and types of the inputs.
    pub fn inputs(&self) -> &'static [(&'static str, PortType)] {
        match self {
            k if k.family() == Family::Generator || k.family() == Family::Landform => &[],
            NodeKind::SolidColor { .. } => &[],
            NodeKind::Combine { .. } => &[("A", H), ("B", H)],
            NodeKind::Blend => &[("A", H), ("B", H), ("Mask", M)],
            NodeKind::Invert => &[("Mask", M)],
            NodeKind::Gradient { .. } => &[("In", H)],
            NodeKind::MixColors => &[("A", C), ("B", C), ("Mask", M)],
            NodeKind::AdjustColor { .. } => &[("Colour", C)],
            NodeKind::Tint { .. } => &[("Colour", C), ("Mask", M)],
            NodeKind::AutoColor { .. } => &[("Height", H)],
            NodeKind::TerrainOutput { .. } => &[("Mask", M)],
            NodeKind::ColorOutput { .. } => &[("Colour", C)],
            _ => &[("In", H)],
        }
    }

    /// Names and types of the outputs.
    pub fn outputs(&self) -> &'static [(&'static str, PortType)] {
        match self {
            NodeKind::Erosion { .. } => &[("Height", H), ("Flow", M), ("Deposits", M), ("Wear", M)],
            NodeKind::Thermal { .. } => &[("Height", H), ("Talus", M)],
            NodeKind::Snow { .. } => &[("Height", H), ("Snow", M)],
            NodeKind::Rivers { .. } => &[("Height", H), ("Rivers", M)],
            NodeKind::Lakes { .. } => &[("Height", H), ("Lakes", M)],
            NodeKind::Sea { .. } => &[("Height", H), ("Water", M), ("Shore", M)],
            NodeKind::HeightOutput | NodeKind::TerrainOutput { .. } | NodeKind::ColorOutput { .. } => &[],
            k if k.family() == Family::Selector => &[("Mask", M)],
            k if k.family() == Family::Color => &[("Colour", C)],
            _ => &[("Out", H)],
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

    /// The type of what an output makes. Masks pass through adjustments,
    /// so a blurred mask is still a mask.
    pub fn output_type(&self, id: NodeId, output: u8) -> PortType {
        let mut seen = HashSet::new();
        self.output_type_inner(id, output, &mut seen)
    }

    fn output_type_inner(&self, id: NodeId, output: u8, seen: &mut HashSet<NodeId>) -> PortType {
        let Some(node) = self.node(id) else { return H };
        let declared = node.kind.outputs().get(output as usize).map_or(H, |o| o.1);
        if declared != H || !seen.insert(id) {
            return declared;
        }
        let follows = |g: &Graph, k: u8, seen: &mut HashSet<NodeId>| {
            g.input_of(id, k).map(|(src, out)| g.output_type_inner(src, out, seen))
        };
        match &node.kind {
            NodeKind::Radial { height, base, .. } if *height <= 1.0 && *base == 0.0 => M,
            NodeKind::Shape { height, base, .. } if *height <= 1.0 && *base == 0.0 => M,
            NodeKind::Constant { value } if (0.0..=1.0).contains(value) => M,
            NodeKind::Levels { out_min, out_max, .. } if *out_min >= 0.0 && *out_max <= 1.0 => M,
            NodeKind::Normalize { min, max } if *min >= 0.0 && *max <= 1.0 => M,
            k if k.family() == Family::Modifier => follows(self, 0, seen).unwrap_or(H),
            NodeKind::Combine { .. } | NodeKind::Blend => {
                let a = follows(self, 0, seen);
                let b = follows(self, 1, seen);
                if a == Some(M) && b.is_none_or(|b| b == M) { M } else { H }
            }
            _ => H,
        }
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
            let mut inputs: Vec<&Link> = g.links.iter().filter(|l| l.to == n).collect();
            inputs.sort_by_key(|l| l.input);
            for l in inputs {
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

    /// A fingerprint of what a node computes: its settings, and those of
    /// everything before it. Two evaluations of nodes with the same
    /// fingerprint over the same extent give the same fields.
    fn fingerprint(&self, id: NodeId, memo: &mut HashMap<NodeId, u64>) -> u64 {
        if let Some(&f) = memo.get(&id) {
            return f;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(node) = self.node(id) {
            format!("{:?}", node.kind).hash(&mut h);
            for k in 0..node.kind.inputs().len() {
                match self.input_of(id, k as u8) {
                    Some((src, out)) => (k, self.fingerprint(src, memo), out).hash(&mut h),
                    None => (k, u64::MAX).hash(&mut h),
                }
            }
        }
        let f = h.finish();
        memo.insert(id, f);
        f
    }

    /// The output nodes, in the graph's order (later terrains win).
    fn outputs_of(&self) -> Vec<&Node> {
        self.nodes.iter().filter(|n| n.kind.family() == Family::Output).collect()
    }

    /// A graph to start from: an eroded mountain range with natural colours
    /// that turn into real blocks, snow on the peaks, rock on the cliffs.
    pub fn starter(grass: u16, rock: u16, snow: u16, gravel: u16) -> Self {
        let _ = (grass, gravel);
        let mut g = Graph::default();
        let range = g.add(
            NodeKind::MountainRange {
                angle: 25.0,
                width: 2600.0,
                scale: 800.0,
                height: 170.0,
                base: 0.0,
                seed: 11,
            },
            [40.0, 40.0],
        );
        let hills = g.add(
            NodeKind::Noise {
                style: NoiseStyle::Fbm,
                scale: 1400.0,
                octaves: 6,
                roughness: 0.5,
                height: 60.0,
                base: 52.0,
                seed: 7,
            },
            [40.0, 240.0],
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
        let out = g.add(NodeKind::HeightOutput, [920.0, 40.0]);
        let color = g.add(
            NodeKind::AutoColor {
                palette: AutoPalette::Alpine,
                snow_line: 190.0,
                rock_slope: 36.0,
                seed: 1,
            },
            [920.0, 160.0],
        );
        let wet = g.add(
            NodeKind::Tint {
                color: [96, 112, 70],
                mode: TintMode::Mix,
                strength: 0.35,
            },
            [1140.0, 160.0],
        );
        let color_out = g.add(
            NodeKind::ColorOutput {
                blocks: true,
                palette: BlockPalette::Natural,
            },
            [1360.0, 160.0],
        );
        let slope = g.add(
            NodeKind::SelectSlope {
                min: 42.0,
                max: 90.0,
                falloff: 6.0,
            },
            [920.0, 340.0],
        );
        let rock_out = g.add(NodeKind::TerrainOutput { terrain: rock }, [1140.0, 340.0]);
        let high = g.add(
            NodeKind::SelectHeight {
                min: 205.0,
                max: 400.0,
                falloff: 12.0,
            },
            [920.0, 480.0],
        );
        let snow_out = g.add(NodeKind::TerrainOutput { terrain: snow }, [1140.0, 480.0]);
        g.connect(range, 0, add, 0);
        g.connect(hills, 0, add, 1);
        g.connect(add, 0, warp, 0);
        g.connect(warp, 0, erosion, 0);
        g.connect(erosion, 0, out, 0);
        g.connect(erosion, 0, color, 0);
        g.connect(color, 0, wet, 0);
        g.connect(erosion, 1, wet, 1);
        g.connect(wet, 0, color_out, 0);
        g.connect(erosion, 0, slope, 0);
        g.connect(slope, 0, rock_out, 0);
        g.connect(erosion, 0, high, 0);
        g.connect(high, 0, snow_out, 0);
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
    pub(crate) fn new(n: usize, data: Vec<f32>) -> Self {
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

/// A square grid of colours over the map, kept as sRGB bytes — what colour
/// maps hold, and a quarter of the memory of floats, which counts at 16k.
/// Colours are worked on in linear light: [`ColorField::at`] and
/// [`ColorField::sample`] give linear values, [`ColorField::build`] takes
/// them.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorField {
    pub n: usize,
    /// sRGB, row-major.
    pub data: Vec<[u8; 3]>,
}

impl ColorField {
    /// A field whose sample `c` (row-major) has the linear colour `f(c)`.
    pub fn build(n: usize, f: impl Fn(usize) -> [f32; 3] + Sync) -> Self {
        Self {
            n,
            data: (0..n * n).into_par_iter().map(|c| f(c).map(color::encode)).collect(),
        }
    }

    /// The linear colour of sample `c` (row-major).
    #[inline]
    pub fn linear(&self, c: usize) -> [f32; 3] {
        self.data[c].map(color::decode)
    }

    #[inline]
    pub fn at(&self, i: usize, j: usize) -> [f32; 3] {
        self.linear(j.min(self.n - 1) * self.n + i.min(self.n - 1))
    }

    /// Bilinear sample at fractional grid coordinates, in linear light.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let max = (self.n - 1) as f32;
        let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (fu, fv) = (u - i as f32, v - j as f32);
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t);
        let top = lerp(self.at(i, j), self.at(i + 1, j), fu);
        let bottom = lerp(self.at(i, j + 1), self.at(i + 1, j + 1), fu);
        lerp(top, bottom, fv)
    }
}

/// What a node output holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Field(Arc<Field>),
    Color(Arc<ColorField>),
}

impl Value {
    pub fn field(&self) -> Option<&Arc<Field>> {
        match self {
            Value::Field(f) => Some(f),
            Value::Color(_) => None,
        }
    }

    pub fn color(&self) -> Option<&Arc<ColorField>> {
        match self {
            Value::Color(c) => Some(c),
            Value::Field(_) => None,
        }
    }

    /// As a field: colours become their brightness.
    pub fn as_field(&self) -> Arc<Field> {
        match self {
            Value::Field(f) => f.clone(),
            Value::Color(c) => Arc::new(Field::new(
                c.n,
                (0..c.n * c.n)
                    .into_par_iter()
                    .map(|k| {
                        let p = c.linear(k);
                        0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
                    })
                    .collect(),
            )),
        }
    }

    /// As colours: fields become greys over their range.
    pub fn as_color(&self) -> Arc<ColorField> {
        match self {
            Value::Color(c) => c.clone(),
            Value::Field(f) => {
                let (lo, hi) = f.range();
                let span = (hi - lo).max(1e-6);
                Arc::new(ColorField::build(f.n, |c| [(f.data[c] - lo) / span; 3]))
            }
        }
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

    /// World position of a point given as fractions of the extent.
    #[inline]
    pub fn at_fraction(&self, x: f32, z: f32) -> (f64, f64) {
        (self.x0 + x as f64 * self.size, self.z0 + z as f64 * self.size)
    }

    /// Fractional grid coordinates of a world position.
    #[inline]
    pub fn grid(&self, x: f64, z: f64) -> (f32, f32) {
        let s = self.size / self.n as f64;
        (((x - self.x0) / s - 0.5) as f32, ((z - self.z0) / s - 0.5) as f32)
    }

    fn key(&self) -> [u64; 4] {
        [self.x0.to_bits(), self.z0.to_bits(), self.size.to_bits(), self.n as u64]
    }
}

// ------------------------------------------------------------- evaluating --

/// What an evaluation made, per node output.
#[derive(Debug, Default, Clone)]
pub struct Evaluation {
    pub values: HashMap<(NodeId, u8), Value>,
    /// Nodes that could not be computed, and why.
    pub failed: HashMap<NodeId, GraphError>,
    /// Milliseconds each node computed took (not those from a cache).
    pub timings: HashMap<NodeId, f32>,
}

impl Evaluation {
    pub fn field(&self, id: NodeId, output: u8) -> Option<&Arc<Field>> {
        self.values.get(&(id, output)).and_then(Value::field)
    }
}

/// Outputs of nodes already computed, kept between evaluations so that
/// only what changed is computed again.
#[derive(Debug, Default)]
pub struct EvalCache {
    extent: Option<[u64; 4]>,
    entries: HashMap<NodeId, (u64, Vec<Value>)>,
}

impl EvalCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// How far a build got.
#[derive(Debug, Default)]
pub struct Progress {
    /// Nodes done, and to do.
    pub done: std::sync::atomic::AtomicUsize,
    pub total: std::sync::atomic::AtomicUsize,
    /// The node being computed.
    pub node: std::sync::atomic::AtomicU32,
}

/// What a graph sends to the map.
#[derive(Debug, Default, Clone)]
pub struct Outputs {
    pub heights: Option<Arc<Field>>,
    /// Masks of the terrain outputs, in the graph's order (later ones win).
    pub terrains: Vec<(TerrainId, Arc<Field>)>,
    /// The colour of the ground, to turn into blocks of a palette.
    pub colors: Option<(Arc<ColorField>, BlockPalette)>,
}

impl Graph {
    /// Evaluate the nodes `targets` need over `extent`. With `keep_all`,
    /// every intermediate field is kept (for previews); otherwise fields are
    /// dropped as soon as nothing else needs them.
    pub fn evaluate(&self, extent: &Extent, targets: &[NodeId], keep_all: bool, cancel: &AtomicBool) -> Result<Evaluation, GraphError> {
        self.evaluate_with(extent, targets, keep_all, None, None, cancel, None)
    }

    /// Evaluate with a cache: nodes whose settings and inputs did not change
    /// since the cache saw them are not computed again.
    pub fn evaluate_cached(&self, extent: &Extent, targets: &[NodeId], cache: &mut EvalCache, cancel: &AtomicBool) -> Result<Evaluation, GraphError> {
        self.evaluate_with(extent, targets, true, Some(cache), None, cancel, None)
    }

    /// Evaluate. Without `keep_all`, a field is dropped as soon as nothing
    /// needs it any more: an output no link uses at once, unless `keep`
    /// names it (or, without `keep`, its node is a target).
    #[allow(clippy::too_many_arguments)]
    fn evaluate_with(
        &self,
        extent: &Extent,
        targets: &[NodeId],
        keep_all: bool,
        mut cache: Option<&mut EvalCache>,
        keep: Option<&HashSet<(NodeId, u8)>>,
        cancel: &AtomicBool,
        progress: Option<&Progress>,
    ) -> Result<Evaluation, GraphError> {
        let order = self.order(targets)?;
        if let Some(p) = progress {
            p.total.store(order.len(), Ordering::Relaxed);
            p.done.store(0, Ordering::Relaxed);
        }
        let kept = |key: &(NodeId, u8)| match keep {
            Some(set) => set.contains(key),
            None => targets.contains(&key.0),
        };
        if let Some(c) = cache.as_deref_mut()
            && c.extent != Some(extent.key())
        {
            c.entries.clear();
            c.extent = Some(extent.key());
        }
        let mut uses: HashMap<(NodeId, u8), usize> = HashMap::new();
        for l in &self.links {
            if order.contains(&l.to) {
                *uses.entry((l.from, l.output)).or_default() += 1;
            }
        }
        let mut memo = HashMap::new();
        let mut out = Evaluation::default();
        for id in order {
            if cancel.load(Ordering::Relaxed) {
                return Err(GraphError::Cancelled);
            }
            let node = self.node(id).expect("ordered nodes exist");
            if let Some(p) = progress {
                p.node.store(id, Ordering::Relaxed);
            }
            let fingerprint = self.fingerprint(id, &mut memo);
            let cached = cache
                .as_deref()
                .and_then(|c| c.entries.get(&id))
                .filter(|(f, _)| *f == fingerprint)
                .map(|(_, v)| v.clone());
            let made = match cached {
                Some(v) => v,
                None => {
                    let inputs: Vec<Option<Value>> = (0..node.kind.inputs().len())
                        .map(|k| self.input_of(id, k as u8).and_then(|src| out.values.get(&src).cloned()))
                        .collect();
                    // Outputs nothing will use are not even made.
                    let wanted: Vec<bool> = (0..node.kind.outputs().len())
                        .map(|k| {
                            let key = (id, k as u8);
                            keep_all || kept(&key) || uses.get(&key).is_some_and(|u| *u > 0)
                        })
                        .collect();
                    let started = std::time::Instant::now();
                    let result = ops::run(&node.kind, extent, &inputs, &wanted, cancel);
                    out.timings.insert(id, started.elapsed().as_secs_f32() * 1000.0);
                    match result {
                        Ok(v) => {
                            if let Some(c) = cache.as_deref_mut() {
                                c.entries.insert(id, (fingerprint, v.clone()));
                            }
                            v
                        }
                        Err(missing) => {
                            if !missing {
                                return Err(GraphError::Cancelled);
                            }
                            let e = GraphError::MissingInput(node.kind.label().to_owned());
                            if keep_all {
                                // Carry on with the rest: what depends on
                                // this node fails in turn.
                                out.failed.insert(id, e);
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
            };
            for (k, v) in made.into_iter().enumerate() {
                let key = (id, k as u8);
                // An output nothing uses is not kept (erosion's masks, say).
                if keep_all || kept(&key) || uses.get(&key).is_some_and(|u| *u > 0) {
                    out.values.insert(key, v);
                }
            }
            if !keep_all {
                for k in 0..node.kind.inputs().len() {
                    if let Some(src) = self.input_of(id, k as u8)
                        && let Some(n) = uses.get_mut(&src)
                    {
                        *n -= 1;
                        if *n == 0 && !kept(&src) {
                            out.values.remove(&src);
                        }
                    }
                }
            }
            if let Some(p) = progress {
                p.done.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(c) = cache {
            c.entries.retain(|id, _| self.node(*id).is_some());
        }
        Ok(out)
    }

    /// Evaluate what the outputs need, and gather it.
    pub fn build(&self, extent: &Extent, cancel: &AtomicBool) -> Result<Outputs, GraphError> {
        self.build_with_progress(extent, cancel, &Progress::default())
    }

    /// [`Graph::build`], saying how far it got in `progress`.
    pub fn build_with_progress(&self, extent: &Extent, cancel: &AtomicBool, progress: &Progress) -> Result<Outputs, GraphError> {
        let outputs = self.outputs_of();
        // Outputs have no fields of their own: evaluate what feeds them.
        let mut feeds = Vec::new();
        for n in &outputs {
            match self.input_of(n.id, 0) {
                Some(src) => feeds.push(src),
                None => return Err(GraphError::MissingInput(n.kind.label().to_owned())),
            }
        }
        let targets: Vec<NodeId> = feeds.iter().map(|f| f.0).collect();
        let keep: HashSet<(NodeId, u8)> = feeds.iter().copied().collect();
        let eval = self.evaluate_with(extent, &targets, false, None, Some(&keep), cancel, Some(progress))?;
        let mut result = Outputs::default();
        for (n, src) in outputs.iter().zip(feeds) {
            let value = eval.values.get(&src).cloned().ok_or(GraphError::Cancelled)?;
            match n.kind {
                NodeKind::HeightOutput => result.heights = Some(value.as_field()),
                NodeKind::TerrainOutput { terrain } => result.terrains.push((TerrainId(terrain), value.as_field())),
                NodeKind::ColorOutput { blocks: true, palette } => result.colors = Some((value.as_color(), palette)),
                _ => {}
            }
        }
        Ok(result)
    }
}

// --------------------------------------------------------------- applying --

/// Picks, for any colour, the closest of a few: a table over the colour
/// cube, so that millions of columns are matched quickly.
struct Nearest {
    table: Vec<u16>,
}

const LUT: usize = 32;

impl Nearest {
    fn new(colors: &[[u8; 3]]) -> Self {
        // A distance close to how different colours look ("redmean").
        let distance = |a: [f32; 3], b: [u8; 3]| {
            let (r1, g1, b1) = (a[0], a[1], a[2]);
            let (r2, g2, b2) = (b[0] as f32, b[1] as f32, b[2] as f32);
            let rm = (r1 + r2) / 2.0;
            let (dr, dg, db) = (r1 - r2, g1 - g2, b1 - b2);
            (2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db
        };
        let table = (0..LUT * LUT * LUT)
            .map(|k| {
                let c = [k % LUT, (k / LUT) % LUT, k / (LUT * LUT)].map(|v| (v as f32 + 0.5) * 256.0 / LUT as f32);
                (0..colors.len())
                    .min_by(|&a, &b| distance(c, colors[a]).total_cmp(&distance(c, colors[b])))
                    .unwrap_or(0) as u16
            })
            .collect();
        Self { table }
    }

    #[inline]
    fn get(&self, c: [f32; 3]) -> usize {
        let q = |v: f32| ((v.clamp(0.0, 255.0) * LUT as f32 / 256.0) as usize).min(LUT - 1);
        self.table[q(c[2]) * LUT * LUT + q(c[1]) * LUT + q(c[0])] as usize
    }
}

/// Which colours of `palette` a colour field comes close enough to that
/// [`apply`] may pick them (allowing for its dithering): the blocks worth
/// making terrains of.
pub fn colors_in_use(field: &ColorField, palette: &[[u8; 3]]) -> Vec<bool> {
    let mut used = vec![false; palette.len()];
    if palette.is_empty() {
        return used;
    }
    let table = Nearest::new(palette);
    for c in &field.data {
        let s = c.map(|v| v as f32);
        used[table.get(s)] = true;
        for k in 0..3 {
            for d in [-9.0, 9.0] {
                let mut t = s;
                t[k] += d;
                used[table.get(t)] = true;
            }
        }
    }
    used
}

/// Write a build to the ground level: heights sampled per column from the
/// height field, the colour of the ground turned into the terrains of
/// `palette` (the closest colour, slightly dithered so that two blocks mix
/// where the colour lies between them), then terrains painted where their
/// masks are high (dithered in between, later outputs over earlier ones).
/// Recorded for undo.
pub fn apply(edit: &mut Edit, extent: &Extent, outputs: &Outputs, seed: u64, palette: &[(TerrainId, [u8; 3])]) {
    let (lo, hi) = (edit.dim().min_y as f32, (edit.dim().max_y - 1) as f32);
    let tiles: Vec<TilePos> = edit.level().positions().collect();
    let everywhere = BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let colors: Vec<[u8; 3]> = palette.iter().map(|p| p.1).collect();
    let nearest = (outputs.colors.is_some() && !palette.is_empty()).then(|| Nearest::new(&colors));
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
                if let (Some(table), Some((field, _))) = (&nearest, &outputs.colors) {
                    let c = field.sample(u, v);
                    let jitter = |k: u64| (hash2(x, z, seed ^ k) as f32 / u32::MAX as f32 - 0.5) * 18.0;
                    let srgb = [0, 1, 2].map(|k| color::encode(c[k]) as f32 + jitter(0xc0 + k as u64));
                    tile.terrain.set(i, palette[table.get(srgb)].0.0);
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
mod tests;
