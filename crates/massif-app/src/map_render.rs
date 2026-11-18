//! The CPU half of the map: what each column looks like from above, coloured
//! for the active display mode and assembled into texture chunks at several
//! levels of detail.
//!
//! A chunk at level of detail `lod` is 128×128 texels, one texel per `2^lod`
//! blocks, so it covers `2^lod × 2^lod` tiles. Chunks at `lod <= 2` read
//! full-resolution [`SurfaceGrid`]s; coarser chunks read [`Summary`]s of a
//! tile's columns, 16×16 of them at `lod` 3, 8×8 at 4, down to one from 7 on,
//! so that a fully zoomed-out map keeps a few hundred bytes a tile.

use egui::{Color32, ColorImage, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode};
use massif_core::compose::{Span, TileStack, block_top};
use massif_core::coords::{TILE_AREA, TILE_SIZE, TilePos, cell_index, local_index};
use massif_core::level::LevelKind;
use massif_core::registry::Registry;
use massif_core::tile::NO_FLUID;
use massif_core::world::Dimension;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Texels per chunk edge.
pub const CHUNK_PX: usize = 128;
/// Coarsest level of detail: one texel per 1024 blocks.
pub const MAX_LOD: u8 = 10;
/// Levels of detail up to this one read full-resolution grids.
const GRID_LODS: u8 = 2;
/// Columns of a tile along each axis in its finest summary (`lod` 3).
const SUMMARY_SIZE: usize = 16;
/// Where a summary's first column is in its tile, along each axis: the
/// columns of a summary of size `n` are every `128 / n` blocks from there,
/// so that a summary holds those of every coarser one.
const SUMMARY_OFFSET: i64 = 4;
/// What summaries shrink to when there are too many: those of `lod` 5.
const SHRUNK_SIZE: usize = 4;

/// Column flag: the column has solid matter separated by air (overhang,
/// floating island or cave).
pub const FLAG_MULTI: u8 = 1;
/// Column flag: the topmost surface is not connected to the world floor.
pub const FLAG_FLOATING: u8 = 2;

/// What one column looks like from above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Elevation of the topmost solid surface; NaN for an empty column.
    pub height: f32,
    pub terrain: u16,
    /// Elevation of water lying on that surface, or [`NO_FLUID`].
    pub fluid: i16,
    /// Painted biome of the surface; 0 = automatic.
    pub biome: u16,
    /// Ground cover painted on the surface; 0 = none.
    pub cover: u16,
    /// Object layer painted on the surface; 0 = none.
    pub objects: u16,
    /// Annotation of the column (on the ground level); 0 = none.
    pub annotation: u8,
    /// Sub-canopy object layer painted on the surface; 0 = none.
    pub understory: u16,
    /// Tree size asked for on the surface; 0 = any.
    pub size: u8,
    /// Layer of underground pockets painted (on the ground level); 0 = none.
    pub pockets: u16,
    pub flags: u8,
    /// Bit `i` set when level `i` (`i < 8`) has the column.
    pub levels: u8,
}

impl Sample {
    pub const EMPTY: Sample = Sample {
        height: f32::NAN,
        terrain: 0,
        fluid: NO_FLUID,
        biome: 0,
        cover: 0,
        objects: 0,
        annotation: 0,
        understory: 0,
        size: 0,
        pockets: 0,
        flags: 0,
        levels: 0,
    };

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.height.is_nan()
    }

    /// Depth of the water on the surface, 0 if dry.
    #[inline]
    pub fn water_depth(&self) -> f32 {
        if self.fluid == NO_FLUID {
            0.0
        } else {
            (self.fluid as f32 - self.height).max(0.0)
        }
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn ground_sample(top: f32, terrain: u16, fluid: i16, biome: u16, cover: u16, objects: u16, annotation: u16, understory: u16, size: u16, pockets: u16) -> Sample {
    let fluid = if fluid != NO_FLUID && fluid as i32 > block_top(top) {
        fluid
    } else {
        NO_FLUID
    };
    Sample {
        height: top,
        terrain,
        fluid,
        biome,
        cover,
        objects,
        annotation: annotation as u8,
        understory,
        size: size as u8,
        pockets,
        flags: 0,
        levels: 1,
    }
}

/// Reusable buffers for composing columns.
#[derive(Default)]
pub struct ColumnSampler {
    spans: Vec<Span>,
    fluids: Vec<(i32, i32)>,
}

impl ColumnSampler {
    /// Compose one column of a tile stack.
    pub fn sample(&mut self, stack: &TileStack, cell: usize) -> Sample {
        if stack.is_ground_only() {
            let g = stack.ground();
            return ground_sample(
                g.top.get(cell),
                g.terrain.get(cell),
                g.fluid.get(cell),
                g.biome.get(cell),
                g.cover.get(cell),
                g.objects.get(cell),
                g.annotations.get(cell),
                g.understory.get(cell),
                g.sizes.get(cell),
                g.pockets.get(cell),
            );
        }
        stack.spans(cell, &mut self.spans);
        stack.fluids(cell, &self.spans, &mut self.fluids);

        let mut levels = 0u8;
        for (index, _, tile) in stack.layers() {
            if index < 8 && tile.present(cell) {
                levels |= 1 << index;
            }
        }
        let Some(top) = self.spans.last().copied() else {
            return Sample {
                levels,
                ..Sample::EMPTY
            };
        };

        let mut flags = 0;
        if self.spans.windows(2).any(|w| w[0].y1 != w[1].y0) {
            flags |= FLAG_MULTI;
        }
        let mut start = self.spans.len() - 1;
        while start > 0 && self.spans[start - 1].y1 == self.spans[start].y0 {
            start -= 1;
        }
        if self.spans[start].y0 > stack.min_y {
            flags |= FLAG_FLOATING;
        }

        let fluid = match self.fluids.last() {
            Some(&(f0, f1)) if f0 == top.y1 => f1.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16,
            _ => NO_FLUID,
        };

        // The biome of the level whose surface ends up on top.
        let mut biome = 0;
        for (_, kind, tile) in stack.layers() {
            if !tile.present(cell) {
                continue;
            }
            let b = tile.biome.get(cell);
            if b == 0 {
                continue;
            }
            let on_top = match kind {
                LevelKind::Ground => true,
                LevelKind::Solid => block_top(tile.top.get(cell)) == top.y1,
                LevelKind::Carve => tile
                    .bottom
                    .as_ref()
                    .is_some_and(|bottom| block_top(bottom.get(cell)) == top.y1),
            };
            if on_top {
                biome = b;
            }
        }

        Sample {
            height: top.top,
            terrain: top.terrain.0,
            fluid,
            biome,
            cover: stack.surface_cover(cell, &top),
            objects: stack.surface_objects(cell, &top),
            annotation: stack.ground().annotations.get(cell) as u8,
            understory: stack.surface_understory(cell, &top),
            size: stack.surface_size(cell, &top) as u8,
            pockets: stack.ground().pockets.get(cell),
            flags,
            levels,
        }
    }
}

/// The composed surface of every column of one tile.
pub struct SurfaceGrid(pub Vec<Sample>);

impl SurfaceGrid {
    pub fn build(dim: &Dimension, pos: TilePos) -> Option<SurfaceGrid> {
        let stack = TileStack::new(dim, pos, false)?;
        let samples = if stack.is_ground_only() {
            let g = stack.ground();
            let (top, terrain) = (g.top.dense(), g.terrain.dense());
            let (fluid, biome, cover, objects) = (g.fluid.dense(), g.biome.dense(), g.cover.dense(), g.objects.dense());
            let (annotations, understory, sizes, pockets) = (g.annotations.dense(), g.understory.dense(), g.sizes.dense(), g.pockets.dense());
            (0..TILE_AREA)
                .map(|i| ground_sample(top[i], terrain[i], fluid[i], biome[i], cover[i], objects[i], annotations[i], understory[i], sizes[i], pockets[i]))
                .collect()
        } else {
            let mut sampler = ColumnSampler::default();
            (0..TILE_AREA).map(|i| sampler.sample(&stack, i)).collect()
        };
        Some(SurfaceGrid(samples))
    }
}

/// The columns of a tile a coarse chunk reads: `size × size` of them, every
/// `128 / size` blocks from [`SUMMARY_OFFSET`] in each direction.
pub struct Summary {
    size: usize,
    samples: Box<[Sample]>,
}

/// The size of the summaries chunks at `lod` (over [`GRID_LODS`]) read: a
/// column for each of their texels, one a tile from `lod` 7 on.
fn summary_size(lod: u8) -> usize {
    (SUMMARY_SIZE >> lod.saturating_sub(3)).max(1)
}

impl Summary {
    pub fn build(dim: &Dimension, pos: TilePos, size: usize) -> Option<Summary> {
        let stack = TileStack::new(dim, pos, false)?;
        let mut sampler = ColumnSampler::default();
        let step = 128 / size;
        let at = |i: usize| i * step + SUMMARY_OFFSET as usize;
        let samples = (0..size * size).map(|k| sampler.sample(&stack, cell_index(at(k % size), at(k / size)))).collect();
        Some(Summary { size, samples })
    }

    /// The same summary with fewer columns: every `self.size / size`th.
    fn shrunk(&self, size: usize) -> Summary {
        let k = self.size / size;
        let samples = (0..size * size).map(|n| self.samples[(n / size * k) * self.size + n % size * k]).collect();
        Summary { size, samples }
    }

    /// The column at `(lx, lz)` in the tile, or the summary's nearest one
    /// before it.
    fn at(&self, lx: i64, lz: i64) -> Sample {
        self.samples[self.slot(lx, lz)]
    }

    /// Which of its columns stands for column `(lx, lz)` of the tile.
    pub(crate) fn slot(&self, lx: i64, lz: i64) -> usize {
        let step = 128 / self.size as i64;
        let index = |l: i64| (((l - SUMMARY_OFFSET).max(0) / step) as usize).min(self.size - 1);
        index(lz) * self.size + index(lx)
    }

    /// Its column `slot` (see [`Summary::slot`]), and how many it holds.
    pub(crate) fn sample(&self, slot: usize) -> Sample {
        self.samples[slot]
    }

    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }

    fn bytes(&self) -> usize {
        self.samples.len() * std::mem::size_of::<Sample>()
    }
}

// ------------------------------------------------------------------ style --

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum MapMode {
    #[default]
    Terrain,
    Height,
    Slope,
    Biome,
    Relief,
}

impl Default for MapStyle {
    fn default() -> Self {
        Self {
            mode: MapMode::Terrain,
            contours: false,
            smooth: false,
            solo: None,
            highlight: None,
            light: DEFAULT_LIGHT,
            lava: false,
            top: 320,
            seed: 0,
            annotations: true,
            sizes: false,
            pockets: false,
        }
    }
}

impl MapMode {
    pub const ALL: [MapMode; 5] = [
        MapMode::Terrain,
        MapMode::Height,
        MapMode::Slope,
        MapMode::Biome,
        MapMode::Relief,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MapMode::Terrain => tr!("Terrain"),
            MapMode::Height => tr!("Height"),
            MapMode::Slope => tr!("Slope"),
            MapMode::Biome => tr!("Biome"),
            MapMode::Relief => tr!("Relief"),
        }
    }
}

/// The colours of the tree sizes over the map, bushes to large trees: pale
/// to dark greens.
pub const SIZE_COLORS: [[u8; 3]; 4] = [[214, 226, 124], [140, 196, 86], [60, 142, 70], [22, 86, 56]];

/// Everything that changes the colour of a texel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapStyle {
    pub mode: MapMode,
    pub contours: bool,
    /// Shading and contours follow the smooth heights of the ground rather
    /// than its whole blocks: softer, without the steps the game will have.
    pub smooth: bool,
    /// Terrain shown normally while all others are faded.
    pub solo: Option<u16>,
    /// Level whose footprint is tinted.
    pub highlight: Option<u8>,
    /// Where the light of the shading comes from, in steps of 22.5°
    /// clockwise from north (14 = north-west).
    pub light: u8,
    /// The world's fluid is lava.
    pub lava: bool,
    /// The top of the world: the height colours spread up to it.
    pub top: i32,
    /// The world's seed: where the plants of covers grow.
    pub seed: i64,
    /// Annotations are drawn over the map.
    pub annotations: bool,
    /// The tree sizes are drawn over the map.
    pub sizes: bool,
    /// The underground pockets are drawn over the map.
    pub pockets: bool,
}

/// Default light direction: from the north-west, like WorldPainter.
pub const DEFAULT_LIGHT: u8 = 14;

/// Unit vector toward the light, 45° above the horizon.
fn light_vector(step: u8) -> [f32; 3] {
    let az = (step % 16) as f32 * 22.5f32.to_radians();
    let h = std::f32::consts::FRAC_1_SQRT_2;
    [az.sin() * h, h, -az.cos() * h]
}

type Rgb = [f32; 3];

fn rgb(c: [u8; 3]) -> Rgb {
    [c[0] as f32, c[1] as f32, c[2] as f32]
}

#[inline]
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[inline]
fn scale(a: Rgb, f: f32) -> Rgb {
    [a[0] * f, a[1] * f, a[2] * f]
}

#[inline]
fn to_color(c: Rgb) -> Color32 {
    let q = |x: f32| x.round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

fn ramp(stops: &[(f32, Rgb)], v: f32) -> Rgb {
    if v <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if v <= b {
            return mix(ca, cb, (v - a) / (b - a));
        }
    }
    stops[stops.len() - 1].1
}

/// Colour of an elevation relative to sea level.
pub fn height_color(height: f32, sea: f32) -> [u8; 3] {
    height_color_to(height, sea, 320.0)
}

/// [`height_color`] for a world whose top is `top`: the colours over the
/// sea spread up to it in worlds taller than the game's own.
pub fn height_color_to(height: f32, sea: f32, top: f32) -> [u8; 3] {
    let k = ((top - sea) / (320.0 - 63.0)).max(1.0);
    let d = height - sea;
    let height = if d > 0.0 { sea + d / k } else { height };
    const STOPS: [(f32, Rgb); 10] = [
        (-90.0, [10.0, 22.0, 70.0]),
        (-40.0, [22.0, 60.0, 140.0]),
        (-8.0, [46.0, 112.0, 186.0]),
        (0.0, [92.0, 160.0, 206.0]),
        (1.0, [86.0, 146.0, 70.0]),
        (25.0, [134.0, 170.0, 82.0]),
        (60.0, [204.0, 190.0, 112.0]),
        (100.0, [164.0, 118.0, 76.0]),
        (150.0, [140.0, 132.0, 126.0]),
        (205.0, [246.0, 246.0, 250.0]),
    ];
    let c = ramp(&STOPS, height - sea);
    [c[0] as u8, c[1] as u8, c[2] as u8]
}

/// Colour of a slope given in degrees.
pub fn slope_color(degrees: f32) -> [u8; 3] {
    const STOPS: [(f32, Rgb); 5] = [
        (0.0, [70.0, 168.0, 88.0]),
        (22.0, [214.0, 206.0, 70.0]),
        (38.0, [236.0, 138.0, 44.0]),
        (52.0, [206.0, 52.0, 44.0]),
        (72.0, [86.0, 26.0, 70.0]),
    ];
    let c = ramp(&STOPS, degrees);
    [c[0] as u8, c[1] as u8, c[2] as u8]
}

/// The biome the exporter picks for a column left on automatic (mirrors
/// `massif_anvil`).
pub struct AutoBiome {
    plains: u16,
    ocean: u16,
    deep_ocean: u16,
    beach: u16,
    desert: u16,
}

impl AutoBiome {
    pub fn new(reg: &Registry) -> Self {
        let id = |s: &str| reg.biome_by_id(s).map_or(0, |b| b.0);
        Self {
            plains: id("minecraft:plains"),
            ocean: id("minecraft:ocean"),
            deep_ocean: id("minecraft:deep_ocean"),
            beach: id("minecraft:beach"),
            desert: id("minecraft:desert"),
        }
    }

    pub fn resolve(&self, reg: &Registry, s: &Sample, sea: f32) -> u16 {
        if s.biome != 0 {
            return s.biome;
        }
        let depth = s.water_depth();
        if depth > 24.0 {
            return self.deep_ocean;
        }
        if depth >= 2.0 {
            return self.ocean;
        }
        match reg.terrain(massif_core::TerrainId(s.terrain)).biome_hint {
            Some(b) if b.0 == self.desert && s.height <= sea + 4.0 => self.beach,
            Some(b) => b.0,
            None => self.plains,
        }
    }
}

/// Hillshade factor: 1 on flat ground, brighter facing the light.
#[inline]
/// The light of one block's step (`dx`, `dz`: blocks up to the
/// neighbour on the side the light comes from). A step of one block stands
/// for the slope of the hillside it is on, which the smooth shading lights
/// a little at every block: it is lit by as much as that light grows with
/// the slope (its first derivative at flat ground), so that the steps of a
/// gentle hillside add up to the light it had smooth, facing the light as
/// facing away. A hillside's light lit at 45° a step (0.45 against 1.15,
/// flat 1) made the steps of gentle slopes too faint where lit, the map
/// darker the more such steps it had: 2 % over a whole real map at its
/// closest, more on its hillsides. Steeper, a column a step of two blocks
/// or more, the ground is lit as the smooth shading lights it.
fn step_shade(dx: f32, dz: f32, l: [f32; 3]) -> f32 {
    if dx.abs() > 1.0 || dz.abs() > 1.0 {
        return hillshade(dx, dz, l);
    }
    const E: f32 = 1e-3;
    let flat = hillshade(0.0, 0.0, l);
    let gx = (hillshade(E, 0.0, l) - hillshade(-E, 0.0, l)) / (2.0 * E);
    let gz = (hillshade(0.0, E, l) - hillshade(0.0, -E, l)) / (2.0 * E);
    (flat + gx * dx + gz * dz).clamp(0.3, 1.32)
}

fn hillshade(dx: f32, dz: f32, l: [f32; 3]) -> f32 {
    let inv = 1.0 / (dx * dx + 1.0 + dz * dz).sqrt();
    let dot = (-dx * l[0] + l[1] - dz * l[2]) * inv;
    (0.3 + 0.7 * dot / l[1]).clamp(0.3, 1.32)
}

const WATER: Rgb = [48.0, 96.0, 200.0];
const HIGHLIGHT: Rgb = [249.0, 115.0, 22.0];

/// Colours samples for one style.
pub struct Colorizer<'a> {
    reg: &'a Registry,
    style: MapStyle,
    sea: f32,
    terrain: Vec<Rgb>,
    biome: Vec<Rgb>,
    /// The covers (by number − 1); `None` for a removed one.
    covers: Vec<Option<massif_core::registry::CoverDef>>,
    /// Whether the top block of each terrain is soil, where plants grow.
    soil: Vec<bool>,
    /// The colour of each layer of underground pockets (by number − 1);
    /// `None` for a removed one.
    pockets: Vec<Option<[u8; 3]>>,
    auto: AutoBiome,
    light: [f32; 3],
    /// The structures placed, drawn over the land where they stand on it.
    structures: &'a [Arc<massif_core::structures::Structure>],
}

impl<'a> Colorizer<'a> {
    /// The same, drawing the structures `structures` too.
    pub fn with_structures(self, structures: &'a [Arc<massif_core::structures::Structure>]) -> Self {
        Self { structures, ..self }
    }

    pub fn new(reg: &'a Registry, style: MapStyle, sea: f32) -> Self {
        Self {
            reg,
            style,
            sea,
            light: light_vector(style.light),
            terrain: reg.terrains.iter().map(|t| rgb(t.color)).collect(),
            biome: reg.biomes.iter().map(|b| rgb(b.color)).collect(),
            covers: reg.covers.iter().map(|c| (!c.removed && !c.hidden).then(|| c.clone())).collect(),
            pockets: reg.pockets.iter().map(|p| (!p.removed).then_some(p.color)).collect(),
            soil: reg
                .terrains
                .iter()
                .map(|t| t.strata.first().is_some_and(|s| s.blocks.iter().any(|(b, _)| massif_core::registry::is_soil(&reg.block(*b).state))))
                .collect(),
            auto: AutoBiome::new(reg),
            structures: &[],
        }
    }

    /// The ground's colour `ground` with the cover of sample `c` over it:
    /// at `at`, a hint of the cover and the plant growing in that column
    /// if one does, in its own colour; from afar (`None`), the cover's
    /// colour as thick as it grows. Nothing where it does not grow: off
    /// soil, or in the air for a cover of the water.
    fn with_cover(&self, ground: Rgb, c: &Sample, at: Option<(i32, i32)>) -> Rgb {
        let Some(def) = c.cover.checked_sub(1).and_then(|k| self.covers.get(k as usize)).and_then(Option::as_ref) else { return ground };
        let wet = c.water_depth() > 0.0;
        if def.underwater != wet || (wet && self.style.lava) || (def.soil_only && !self.soil.get(c.terrain as usize).copied().unwrap_or(false)) {
            return ground;
        }
        let color = rgb(def.color);
        match at {
            Some((x, z)) => {
                let base = mix(ground, color, 0.18);
                match def.growth(x, z, self.style.seed as u64) {
                    Some(g) => {
                        let plant = def.plants.get(g.plant).and_then(|p| massif_core::registry::plant_color(&p.0)).map_or(color, rgb);
                        mix(base, plant, 0.85)
                    }
                    None => base,
                }
            }
            None => mix(ground, color, 0.25 + 0.5 * def.density.clamp(0.0, 1.0)),
        }
    }

    fn terrain_rgb(&self, id: u16) -> Rgb {
        self.terrain
            .get(id as usize)
            .copied()
            .unwrap_or([118.0, 118.0, 118.0])
    }

    /// Colour of the centre sample `c` given its four neighbours, `spacing`
    /// blocks apart. `at`: the column of `c`, when a texel is one or two
    /// blocks (the plants of covers show one by one). `steps`: the lines of
    /// the one-block steps across the texel, added to its light (see
    /// [`block_step_shades`]).
    #[allow(clippy::too_many_arguments)]
    pub fn pixel(
        &self,
        c: &Sample,
        at: Option<(i32, i32)>,
        l: &Sample,
        r: &Sample,
        u: &Sample,
        d: &Sample,
        spacing: f32,
        exaggeration: f32,
        contour: Option<f32>,
        trend: Option<f32>,
        steps: Option<f32>,
    ) -> Color32 {
        if c.is_empty() {
            return Color32::TRANSPARENT;
        }
        // The height of the top block, as the game will have it (and as
        // WorldPainter draws it), so that each step of a block reads; or,
        // smooth or when a texel spans more than one block (the steps would
        // only flicker in patterns), the height the ground was shaped to.
        let blocks = !self.style.smooth && spacing <= 1.0;
        let level = |v: f32| if blocks { block_top(v) as f32 } else { v };
        let h = |s: &Sample| level(if s.is_empty() { c.height } else { s.height });
        let (dx, dz) = if blocks {
            // Each column against its neighbour on the side the light comes
            // from, as the game's own maps are shaded: a step is one block
            // wide, lit on the block at its top where it faces the light, in
            // shadow on the block at its foot where it faces away, and a
            // block alone above its neighbours shows too.
            let c = h(c);
            let dx = if self.light[0] <= 0.0 { c - h(l) } else { h(r) - c };
            let dz = if self.light[2] <= 0.0 { c - h(u) } else { h(d) - c };
            (dx / spacing, dz / spacing)
        } else {
            ((h(r) - h(l)) / (2.0 * spacing), (h(d) - h(u)) / (2.0 * spacing))
        };
        let shade = if blocks { step_shade(dx, dz, self.light) } else { hillshade(dx * exaggeration, dz * exaggeration, self.light) };
        let shade = steps.map_or(shade, |line| (shade + line).clamp(0.3, 1.32));

        let mut out = match self.style.mode {
            MapMode::Terrain => {
                let ground = self.terrain_rgb(c.terrain);
                // What grows on it shows over it, as thick as it grows.
                let ground = self.with_cover(ground, c, at);
                scale(ground, shade)
            }
            MapMode::Height => scale(rgb(height_color_to(level(c.height), self.sea, self.style.top as f32)), 0.7 + 0.3 * shade),
            MapMode::Slope => {
                // The trend of the hillside when it was measured around the
                // sample, the step to the neighbours otherwise.
                let deg = trend.unwrap_or_else(|| (dx * dx + dz * dz).sqrt().atan().to_degrees());
                rgb(slope_color(deg))
            }
            MapMode::Biome => {
                let id = self.auto.resolve(self.reg, c, self.sea);
                let base = self.biome.get(id as usize).copied().unwrap_or([128.0; 3]);
                scale(base, 0.82 + 0.18 * shade)
            }
            MapMode::Relief => {
                let g = 30.0 + 190.0 * ((shade - 0.3) / 1.02);
                [g, g, g]
            }
        };

        let depth = c.water_depth();
        if depth > 0.0 && !matches!(self.style.mode, MapMode::Slope | MapMode::Biome) {
            let t = (depth / 18.0).min(1.0);
            let cover = c.cover.checked_sub(1).and_then(|k| self.covers.get(k as usize)).and_then(Option::as_ref);
            let (frozen, lava) = cover.map_or((false, false), |d| (d.frost && !self.style.lava, d.lava));
            if frozen {
                // Frost: the water is ice, white-blue, the relief under
                // it lost.
                out = scale([206.0, 224.0, 246.0], 0.9 + 0.1 * shade);
            } else if self.style.lava || lava {
                // Lava hides what lies under it: bright, with a darker
                // crust along its shores.
                let crust = (1.0 - depth / 2.5).clamp(0.0, 1.0);
                out = mix([238.0, 112.0, 22.0], [150.0, 42.0, 12.0], 0.65 * crust);
            } else {
                let water = if self.style.mode == MapMode::Relief { [70.0, 110.0, 170.0] } else { WATER };
                out = mix(out, water, 0.5 + 0.42 * t);
            }
        }

        // Underground pockets, while they are painted: hatched in their
        // colour.
        if self.style.pockets
            && let Some(color) = c.pockets.checked_sub(1).and_then(|k| self.pockets.get(k as usize)).copied().flatten()
            && at.is_none_or(|(x, z)| (x + z).rem_euclid(4) < 2)
        {
            out = mix(out, color.map(|v| v as f32), 0.55);
        }
        // Tree sizes, under the annotations.
        if self.style.sizes
            && let Some(&color) = c.size.checked_sub(1).and_then(|k| SIZE_COLORS.get(k as usize))
        {
            out = mix(out, color.map(|v| v as f32), 0.6);
        }
        // Annotations: marks of colour over everything else.
        if self.style.annotations
            && let Some(&color) = c.annotation.checked_sub(1).and_then(|k| massif_core::tile::ANNOTATION_COLORS.get(k as usize))
        {
            out = mix(out, color.map(|v| v as f32), 0.75);
        }

        if let Some(interval) = contour {
            let band = |v: f32| (level(v) / interval).floor();
            let b = band(c.height);
            if (!r.is_empty() && band(r.height) != b) || (!d.is_empty() && band(d.height) != b) {
                out = scale(out, 0.74);
            }
        }

        // Outline floating surfaces so overhangs read as such.
        let floating = |s: &Sample| !s.is_empty() && s.flags & FLAG_FLOATING != 0;
        if floating(c) && !(floating(l) && floating(r) && floating(u) && floating(d)) {
            out = scale(out, 0.55);
        }

        if let Some(t) = self.style.solo
            && c.terrain != t
        {
            out = mix(out, [196.0, 196.0, 200.0], 0.72);
        }
        if let Some(level) = self.style.highlight
            && level < 8
            && c.levels & (1 << level) != 0
        {
            out = mix(out, HIGHLIGHT, 0.42);
        }
        to_color(out)
    }
}

// ----------------------------------------------------------------- chunks --

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub lod: u8,
    pub x: i32,
    pub z: i32,
}

impl ChunkKey {
    /// Blocks covered by the chunk along each axis.
    pub fn blocks(&self) -> i64 {
        (CHUNK_PX as i64) << self.lod
    }

    /// Block coordinates of the north-west corner.
    pub fn origin(&self) -> (i64, i64) {
        (self.x as i64 * self.blocks(), self.z as i64 * self.blocks())
    }

    /// The chunk at `lod` containing tile `t`.
    pub fn of_tile(t: TilePos, lod: u8) -> ChunkKey {
        ChunkKey {
            lod,
            x: t.x >> lod,
            z: t.z >> lod,
        }
    }

    /// The chunk one level coarser that contains this one.
    pub fn parent(&self) -> ChunkKey {
        ChunkKey {
            lod: self.lod + 1,
            x: self.x >> 1,
            z: self.z >> 1,
        }
    }

    /// Block coordinates of sample `(i, j)`, which may lie one texel outside
    /// the chunk (`-1..=128`) for the shading of the border.
    #[inline]
    fn sample_point(&self, i: isize, j: isize) -> (i32, i32) {
        let step = 1i64 << self.lod;
        let (x0, z0) = self.origin();
        // The middle of the texel, or, over GRID_LODS, the column of the
        // summaries there (the same at lod 3).
        let mid = (step / 2).min(SUMMARY_OFFSET);
        let x = x0 + i as i64 * step + mid;
        let z = z0 + j as i64 * step + mid;
        (x.clamp(i32::MIN as i64, i32::MAX as i64) as i32, z.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }

    /// Tiles whose data the chunk reads.
    fn needed_tiles(&self, out: &mut HashSet<TilePos>) {
        let n = CHUNK_PX as isize;
        if self.lod <= 6 {
            // Every tile under the chunk plus a ring for the shading and
            // the slope.
            let r = slope_ring(self.lod) as isize;
            let (a, b) = (self.sample_point(-r, -r), self.sample_point(n + r - 1, n + r - 1));
            let (lo, hi) = (TilePos::containing(a.0, a.1), TilePos::containing(b.0, b.1));
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    out.insert(TilePos::new(x, z));
                }
            }
        } else {
            for j in -1..=n {
                for i in -1..=n {
                    let (x, z) = self.sample_point(i, j);
                    out.insert(TilePos::containing(x, z));
                }
            }
        }
    }
}

fn moving_id() -> egui::Id {
    egui::Id::unique("massif a view moves")
}

/// Say that a view of the map moves (the map view or the 3D view, this
/// frame): while one does, the views keep the work they do in the frame's
/// time short, so that it keeps up with the hand.
pub fn mark_moving(ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(moving_id(), now));
}

/// Whether a view of the map moved in the last quarter of a second.
pub fn moved_lately(ctx: &egui::Context) -> bool {
    let now = ctx.input(|i| i.time);
    ctx.data(|d| d.get_temp::<f64>(moving_id())).is_some_and(|t| now - t < 0.25)
}

/// What one batch of [`MapCache::update`] holds at most: chunks to draw,
/// and tiles to read that are not read yet.
const BATCH_CHUNKS: usize = 6;
const BATCH_TILES_PER_THREAD: usize = 8;

/// Read access to the cached column data while chunks are built.
struct Sources<'a> {
    grids: &'a HashMap<TilePos, GridEntry>,
    summaries: &'a HashMap<TilePos, SummaryEntry>,
}

impl Sources<'_> {
    fn sample(&self, lod: u8, x: i32, z: i32) -> Sample {
        let pos = TilePos::containing(x, z);
        if lod <= GRID_LODS {
            match self.grids.get(&pos).and_then(|g| g.grid.as_ref()) {
                Some(grid) => grid.0[local_index(x, z)],
                None => Sample::EMPTY,
            }
        } else {
            match self.summaries.get(&pos).and_then(|e| e.summary.as_ref()) {
                Some(s) => s.at(x.rem_euclid(128) as i64, z.rem_euclid(128) as i64),
                None => Sample::EMPTY,
            }
        }
    }
}

/// Contour interval at a level of detail.
fn contour_interval(lod: u8) -> f32 {
    match lod {
        0 | 1 => 8.0,
        2 => 16.0,
        3 => 32.0,
        _ => 64.0,
    }
}

/// Samples around a pixel the slope is measured over, each way, at a level
/// of detail: the same reach in blocks (`SLOPE_REACH`) whatever the zoom.
fn slope_ring(lod: u8) -> usize {
    ((massif_core::filter::SLOPE_REACH as usize) >> lod).max(1)
}

/// The levels of detail past the first whose texels are lit block by block
/// (a texel of 2 × 2 and 4 × 4 blocks; the grids hold every column up to
/// there). Further out a step's line would be an eighth of a texel or less.
const STEP_LODS: u8 = GRID_LODS;

/// The lines of the one-block steps over each texel of the chunk at `key`
/// (a level of detail of [`STEP_LODS`]), to add to its light. Each block is
/// shaded by its step to the neighbour on the side the light comes from,
/// as the closest level shades a texel a block; a texel takes the mean of
/// its blocks, and gives what sets it apart from the texels around: where
/// a step crosses it, the line of that step, √k as strong as its share of
/// the texel (k blocks a side: half as strong as close up at 4 × 4 blocks),
/// while the light of the hillside, from the texels' own heights, stays as
/// it was. A texel's one sample shaded from the texels around would smooth
/// the steps away, or alias them into patterns. `None` for a texel without
/// ground.
fn block_step_shades(key: ChunkKey, sources: &Sources, light: [f32; 3]) -> Vec<Option<f32>> {
    let k = 1usize << key.lod;
    // The texels of the chunk and a ring of them around; their blocks, and
    // a ring of blocks around for the neighbours.
    let t = CHUNK_PX + 2;
    let w = t * k + 2;
    let (x0, z0) = key.origin();
    let (x0, z0) = (x0 - k as i64 - 1, z0 - k as i64 - 1);
    // The top block of each column, NaN where there is no ground; read a
    // tile's row at a time.
    let mut tops = vec![f32::NAN; w * w];
    for j in 0..w {
        let z = (z0 + j as i64) as i32;
        let mut i = 0;
        while i < w {
            let x = (x0 + i as i64) as i32;
            let tile = TilePos::containing(x, z);
            let end = (((tile.x as i64 + 1) * TILE_SIZE as i64 - x0) as usize).min(w);
            if let Some(grid) = sources.grids.get(&tile).and_then(|g| g.grid.as_ref()) {
                for (ii, top) in tops[j * w + i..j * w + end].iter_mut().enumerate() {
                    let s = &grid.0[local_index(x + ii as i32, z)];
                    if !s.is_empty() {
                        *top = block_top(s.height) as f32;
                    }
                }
            }
            i = end;
        }
    }
    // The mean light of each texel's blocks, NaN without ground.
    let mut means = vec![f32::NAN; t * t];
    for (n, mean) in means.iter_mut().enumerate() {
        let (ti, tj) = (n % t, n / t);
        let (mut sum, mut count) = (0.0f32, 0usize);
        for bj in 0..k {
            let row = (tj * k + bj + 1) * w;
            for bi in 0..k {
                let at = row + ti * k + bi + 1;
                let c = tops[at];
                if c.is_nan() {
                    continue;
                }
                let h = |v: f32| if v.is_nan() { c } else { v };
                let dx = if light[0] <= 0.0 { c - h(tops[at - 1]) } else { h(tops[at + 1]) - c };
                let dz = if light[2] <= 0.0 { c - h(tops[at - w]) } else { h(tops[at + w]) - c };
                sum += step_shade(dx, dz, light);
                count += 1;
            }
        }
        if count > 0 {
            *mean = sum / count as f32;
        }
    }
    let boost = (k as f32).sqrt();
    let mut out = vec![None; CHUNK_PX * CHUNK_PX];
    for (n, line) in out.iter_mut().enumerate() {
        let (ti, tj) = (n % CHUNK_PX + 1, n / CHUNK_PX + 1);
        let m = means[tj * t + ti];
        if m.is_nan() {
            continue;
        }
        let (mut sum, mut count) = (0.0f32, 0usize);
        for dj in 0..3 {
            for di in 0..3 {
                let v = means[(tj + dj - 1) * t + ti + di - 1];
                if !v.is_nan() {
                    sum += v;
                    count += 1;
                }
            }
        }
        *line = Some((m - sum / count as f32) * boost);
    }
    out
}

fn build_chunk(key: ChunkKey, sources: &Sources, colorizer: &Colorizer) -> ColorImage {
    let slope = colorizer.style.mode == MapMode::Slope;
    let ring = if slope { slope_ring(key.lod) } else { 1 };
    let w = CHUNK_PX + 2 * ring;
    let mut samples = vec![Sample::EMPTY; w * w];
    for j in 0..w {
        for i in 0..w {
            let (x, z) = key.sample_point(i as isize - ring as isize, j as isize - ring as isize);
            samples[j * w + i] = sources.sample(key.lod, x, z);
        }
    }
    let spacing = (1u32 << key.lod) as f32;
    let exaggeration = 1.0 + 0.3 * key.lod as f32;
    let contour = colorizer.style.contours.then(|| contour_interval(key.lod));
    // A texel of 2 × 2 or 4 × 4 blocks: the lines of its blocks' steps over
    // the light of the hillside (see `block_step_shades`).
    let steps = (!colorizer.style.smooth && !slope && (1..=STEP_LODS).contains(&key.lod)).then(|| block_step_shades(key, sources, colorizer.light));
    let mut pixels = Vec::with_capacity(CHUNK_PX * CHUNK_PX);
    for j in ring..ring + CHUNK_PX {
        for i in ring..ring + CHUNK_PX {
            let at = |di: isize, dj: isize| &samples[(j as isize + dj) as usize * w + (i as isize + di) as usize];
            let trend = slope.then(|| {
                let h = |dx: i32, dz: i32| {
                    let s = at(dx as isize, dz as isize);
                    (!s.is_empty()).then_some(s.height)
                };
                massif_core::filter::trend_slope(h, ring as i32, spacing)
            });
            let column = (key.lod <= 1).then(|| key.sample_point(i as isize - ring as isize, j as isize - ring as isize));
            pixels.push(colorizer.pixel(
                at(0, 0),
                column,
                at(-1, 0),
                at(1, 0),
                at(0, -1),
                at(0, 1),
                spacing,
                exaggeration,
                contour,
                trend,
                steps.as_ref().and_then(|s| s[(j - ring) * CHUNK_PX + (i - ring)]),
            ));
        }
    }
    draw_objects(key, sources, colorizer, &mut pixels);
    draw_structures(key, sources, colorizer, &mut pixels);
    ColorImage::new([CHUNK_PX, CHUNK_PX], pixels)
}

/// The objects standing on the chunk at `key`, seen from above, over its
/// pixels: when a pixel is at most four blocks across. Where they stand is
/// asked of the same grid as the export, from the columns the map knows.
fn draw_objects(key: ChunkKey, sources: &Sources, colorizer: &Colorizer, pixels: &mut [Color32]) {
    let reg = colorizer.reg;
    if key.lod > 2 || colorizer.style.mode == MapMode::Slope || !reg.object_layers.iter().any(|l| !l.removed && !l.hidden && !l.objects.is_empty()) {
        return;
    }
    let lod = key.lod as i64;
    let (x0, z0) = key.origin();
    let span = key.blocks();
    let reach = massif_core::objects::max_reach(reg) as i64;
    let seed = colorizer.style.seed as u64;
    let mut placed = Vec::new();
    for (k, layer) in reg.object_layers.iter().enumerate() {
        if layer.removed || layer.hidden || layer.objects.is_empty() {
            continue;
        }
        let n = k as u16 + 1;
        let classes = layer.size_classes(reg);
        let s = layer.spacing.max(1) as i64;
        for j in (z0 - reach).div_euclid(s)..=(z0 + span + reach).div_euclid(s) {
            for i in (x0 - reach).div_euclid(s)..=(x0 + span + reach).div_euclid(s) {
                let Some(c) = layer.candidate(seed, n, i as i32, j as i32) else { continue };
                let Some(def) = reg.objects.get(c.object as usize) else { continue };
                let r = def.reach() as i64;
                if (c.x as i64) + r < x0 || (c.x as i64) - r >= x0 + span || (c.z as i64) + r < z0 || (c.z as i64) - r >= z0 + span {
                    continue;
                }
                let at = sources.sample(0, c.x, c.z);
                // On the base plane, or under the canopy.
                let under = at.understory == n;
                if at.is_empty() || (at.objects != n && !under) {
                    continue;
                }
                let wet = at.water_depth() > 0.0;
                if wet != layer.underwater || (wet && colorizer.style.lava) || (layer.soil_only && !colorizer.soil.get(at.terrain as usize).copied().unwrap_or(false)) {
                    continue;
                }
                let object = match at.size {
                    0 => c.object,
                    size => match layer.pick(c.pick, size as u16, &classes) {
                        Some(o) if reg.objects.get(o as usize).is_some() => o,
                        _ => continue,
                    },
                };
                placed.push(massif_core::objects::Placement { layer: n, object, x: c.x, y: 0, z: c.z, turn: c.turn, mirror: c.mirror, under });
            }
        }
    }
    // The sub-canopy first: the canopy hides it from above.
    placed.sort_unstable_by_key(|p| (!p.under, p.layer, p.x, p.z));
    let mut views: HashMap<u32, Vec<(u16, u16)>> = HashMap::new();
    // The texels of each object's top, its shadow a block to the south-east.
    let texels = |p: &massif_core::objects::Placement, views: &mut HashMap<u32, Vec<(u16, u16)>>, shift: i64| -> Vec<(usize, u16, u16)> {
        let def = &reg.objects[p.object as usize];
        let w = def.size[0] as usize;
        let tops = views.entry(p.object).or_insert_with(|| def.top_view());
        let mut out = Vec::new();
        for (c, &(k, y)) in tops.iter().enumerate() {
            if k == 0 {
                continue;
            }
            let (x, _, z) = p.world(def, (c % w) as i32, 0, (c / w) as i32);
            let (i, j) = ((x as i64 + shift - x0) >> lod, (z as i64 + shift - z0) >> lod);
            if (0..CHUNK_PX as i64).contains(&i) && (0..CHUNK_PX as i64).contains(&j) {
                out.push((j as usize * CHUNK_PX + i as usize, k, y));
            }
        }
        out
    };
    let mut shaded = vec![false; pixels.len()];
    for p in &placed {
        for (t, _, _) in texels(p, &mut views, 1 << lod) {
            if !shaded[t] {
                shaded[t] = true;
                let c = pixels[t];
                pixels[t] = Color32::from_rgb((c.r() as f32 * 0.72) as u8, (c.g() as f32 * 0.72) as u8, (c.b() as f32 * 0.72) as u8);
            }
        }
    }
    for p in &placed {
        let def = &reg.objects[p.object as usize];
        let h = def.size[1].max(1) as f32;
        // With the tree sizes shown, each tree takes the colour of its size.
        let tint = if colorizer.style.sizes {
            let at = sources.sample(0, p.x, p.z);
            at.size.checked_sub(1).and_then(|k| SIZE_COLORS.get(k as usize)).map(|c| c.map(|v| v as f32))
        } else {
            None
        };
        for (t, k, y) in texels(p, &mut views, 0) {
            // Higher up, lighter: a crown shows its round.
            let shade = 0.72 + 0.4 * (y as f32 + 1.0) / h;
            let color = scale(rgb(def.color(k)), shade);
            pixels[t] = to_color(match tint {
                Some(tint) => mix(color, tint, 0.55),
                None => color,
            });
        }
    }
}

/// The structures placed over the chunk at `key`, seen from above: their
/// topmost blocks where they stand over the land (a village's roofs and
/// paths; an Ancient City, under it, shows nothing), when a pixel is at
/// most four blocks across.
fn draw_structures(key: ChunkKey, sources: &Sources, colorizer: &Colorizer, pixels: &mut [Color32]) {
    if key.lod > 2 || colorizer.structures.is_empty() || colorizer.style.mode == MapMode::Slope {
        return;
    }
    let lod = key.lod as i64;
    let (x0, z0) = key.origin();
    let span = key.blocks();
    let area = massif_core::BlockRect::new(x0 as i32, z0 as i32, (x0 + span) as i32, (z0 + span) as i32);
    // Each pixel's highest block of a structure: its height, its colour.
    let mut tops: Vec<Option<(i32, [u8; 3])>> = vec![None; CHUNK_PX * CHUNK_PX];
    let mut any = false;
    for s in colorizer.structures {
        if s.columns().intersect(&area).is_empty() {
            continue;
        }
        for sec in &s.sections {
            let (sx, sz) = (sec.pos[0] as i64 * 16, sec.pos[2] as i64 * 16);
            if sx + 16 <= x0 || sx >= x0 + span || sz + 16 <= z0 || sz >= z0 + span {
                continue;
            }
            for &(i, k) in &sec.blocks {
                if s.is_air(k) {
                    continue;
                }
                let [x, y, z] = sec.block(i);
                let (px, pz) = ((x as i64 - x0) >> lod, (z as i64 - z0) >> lod);
                if !(0..CHUNK_PX as i64).contains(&px) || !(0..CHUNK_PX as i64).contains(&pz) {
                    continue;
                }
                let t = &mut tops[pz as usize * CHUNK_PX + px as usize];
                if t.is_none_or(|(ty, _)| y > ty) {
                    *t = Some((y, s.color(k)));
                    any = true;
                }
            }
        }
    }
    if !any {
        return;
    }
    for (t, top) in tops.iter().enumerate() {
        let Some((y, color)) = *top else { continue };
        let (x, z) = ((x0 + (((t % CHUNK_PX) as i64) << lod)) as i32, (z0 + (((t / CHUNK_PX) as i64) << lod)) as i32);
        let at = sources.sample(0, x, z);
        let land = if at.is_empty() { i32::MIN } else { block_top(at.height) };
        // Under the land (and its water): hidden. A path one block down,
        // in the land's top, shows.
        let water = if at.fluid != NO_FLUID { at.fluid as i32 } else { i32::MIN };
        if y + 1 < land.max(water) {
            continue;
        }
        // Higher over the land, lighter: a roof shows over its walls.
        let shade = 0.82 + 0.05 * ((y - land + 1).clamp(0, 6) as f32);
        pixels[t] = to_color(scale(rgb(color), shade));
    }
}

// ------------------------------------------------------------------ cache --

struct GridEntry {
    grid: Option<Arc<SurfaceGrid>>,
    used: u64,
}

/// Bytes a tile's grid of columns takes.
pub const GRID_BYTES: usize = TILE_AREA * std::mem::size_of::<Sample>();
/// Bytes a chunk's texture takes on the graphics card.
pub const CHUNK_BYTES: usize = CHUNK_PX * CHUNK_PX * 4;

struct SummaryEntry {
    summary: Option<Arc<Summary>>,
    used: u64,
}

impl SummaryEntry {
    fn bytes(&self) -> usize {
        self.summary.as_ref().map_or(0, |s| s.bytes())
    }

    /// Whether it holds the columns of a summary of `size` (or the tile is
    /// empty).
    fn holds(&self, size: usize) -> bool {
        self.summary.as_ref().is_none_or(|s| s.size >= size)
    }
}

pub struct ChunkEntry {
    pub texture: TextureHandle,
    pub stale: bool,
    /// When it went stale: those waiting longest are redrawn first.
    stale_since: u64,
    used: u64,
}

/// Map textures and the column data they are built from.
pub struct MapCache {
    generation: u64,
    clock: u64,
    grids: HashMap<TilePos, GridEntry>,
    summaries: HashMap<TilePos, SummaryEntry>,
    /// Bytes the summaries take.
    summary_bytes: usize,
    chunks: HashMap<(MapStyle, ChunkKey), ChunkEntry>,
    /// Grids kept around ([`GRID_BYTES`] each).
    pub max_grids: usize,
    /// Bytes of summaries kept around at full size: beyond, those not used
    /// lately shrink to the size of `lod` 5.
    pub max_summary_bytes: usize,
    /// Chunk textures kept around (64 KiB each).
    pub max_chunks: usize,
    busy: bool,
}

impl MapCache {
    /// The summaries of the tiles the map has read, for the map of
    /// generation `generation`: what the 3D view shows of a large map
    /// before it reads every column (see `view3d::build_glimpse`).
    pub fn summaries_of(&self, generation: u64) -> HashMap<TilePos, Arc<Summary>> {
        if self.generation != generation {
            return HashMap::new();
        }
        self.summaries.iter().filter_map(|(t, e)| Some((*t, e.summary.clone()?))).collect()
    }
}

impl Default for MapCache {
    fn default() -> Self {
        Self {
            generation: u64::MAX,
            clock: 0,
            grids: HashMap::new(),
            summaries: HashMap::new(),
            summary_bytes: 0,
            chunks: HashMap::new(),
            max_grids: 384,
            max_summary_bytes: DEFAULT_SUMMARY_BYTES,
            max_chunks: 1400,
            busy: false,
        }
    }
}

/// Summaries kept at full size by default.
pub const DEFAULT_SUMMARY_BYTES: usize = 128 << 20;

pub const TEXTURE_OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

impl MapCache {
    /// Drop everything when a different world is shown.
    pub fn sync_generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.grids.clear();
            self.summaries.clear();
            self.summary_bytes = 0;
            self.chunks.clear();
            self.generation = generation;
        }
    }

    /// Forget the column data of `tiles` and mark the chunks showing them
    /// (or their shading neighbours) stale. Stale chunks keep being drawn
    /// until they are rebuilt, so edits never flash.
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        if tiles.is_empty() {
            return;
        }
        // A pass over a whole big map: everything, at once.
        if tiles.len() > 4096 {
            self.invalidate_all();
            return;
        }
        let mut affected = HashSet::new();
        for &t in tiles {
            self.grids.remove(&t);
            if let Some(e) = self.summaries.remove(&t) {
                self.summary_bytes -= e.bytes();
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = t.offset(dx, dz);
                    for lod in 0..=MAX_LOD {
                        affected.insert(ChunkKey::of_tile(n, lod));
                    }
                }
            }
        }
        self.clock += 1;
        let clock = self.clock;
        for ((_, key), entry) in self.chunks.iter_mut() {
            if affected.contains(key) && !entry.stale {
                entry.stale = true;
                entry.stale_since = clock;
            }
        }
    }

    /// Everything is out of date (level visibility or order changed).
    pub fn invalidate_all(&mut self) {
        self.grids.clear();
        self.summaries.clear();
        self.summary_bytes = 0;
        self.clock += 1;
        let clock = self.clock;
        for entry in self.chunks.values_mut() {
            if !entry.stale {
                entry.stale = true;
                entry.stale_since = clock;
            }
        }
    }

    /// The texture of a chunk, if one was ever built (possibly stale).
    pub fn texture(&mut self, style: MapStyle, key: ChunkKey) -> Option<&TextureHandle> {
        self.clock += 1;
        let clock = self.clock;
        self.chunks.get_mut(&(style, key)).map(|e| {
            e.used = clock;
            &e.texture
        })
    }

    pub fn is_ready(&self, style: MapStyle, key: ChunkKey) -> bool {
        self.chunks.get(&(style, key)).is_some_and(|e| !e.stale)
    }

    /// Build or refresh the `wanted` chunks (most important first) for about
    /// `budget`. Returns whether work remains.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        dim: &Dimension,
        reg: &Registry,
        sea: f32,
        style: MapStyle,
        wanted: &[ChunkKey],
        budget: Duration,
    ) -> bool {
        // What was used before this update may shrink.
        let first = self.clock + 1;
        self.busy = self.build(ctx, dim, reg, sea, style, wanted, budget);
        self.trim_summaries(first);
        self.busy
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &mut self,
        ctx: &egui::Context,
        dim: &Dimension,
        reg: &Registry,
        sea: f32,
        style: MapStyle,
        wanted: &[ChunkKey],
        budget: Duration,
    ) -> bool {
        let started = Instant::now();
        let mut todo: Vec<ChunkKey> = wanted
            .iter()
            .copied()
            .filter(|k| !self.is_ready(style, *k))
            .collect();
        // Those never drawn first, in the order wanted, then the stale ones,
        // those waiting longest first: a brush held over the map marks the
        // same chunks stale every frame, and in the order wanted alone those
        // a frame has no time for would never be redrawn while it is held.
        todo.sort_by_key(|k| self.chunks.get(&(style, *k)).map_or(0, |e| e.stale_since));
        if todo.is_empty() {
            return false;
        }
        let colorizer = Colorizer::new(reg, style, sea).with_structures(&dim.structures);
        let ground = dim.ground();
        let mut cursor = 0;
        // The chunk whose tiles are being read: the tiles it needs, and those
        // still to read.
        let mut current: Option<(usize, Vec<TilePos>, Vec<TilePos>)> = None;
        let capacity = BATCH_TILES_PER_THREAD * rayon::current_num_threads().max(1);
        let mut first = true;
        while cursor < todo.len() {
            // One batch at least, however slow the frame: what is drawn
            // keeps up, a little at a time.
            if !first && started.elapsed() > budget {
                return true;
            }
            first = false;
            // A batch: the tiles still to read for the next chunks, about a
            // third of a millisecond of reading for each thread, and the
            // chunks whose tiles are then all read, at most BATCH_CHUNKS, so
            // that the frame keeps to its budget. A chunk that needs more
            // tiles than a batch holds has them read over several batches
            // (worked out once), and is drawn once they all are.
            self.clock += 1;
            let clock = self.clock;
            // The tiles to read, with the size of the summaries wanted.
            let (mut fine_reads, mut coarse_reads): (HashMap<TilePos, usize>, HashMap<TilePos, usize>) = (HashMap::new(), HashMap::new());
            let mut batch = Vec::new();
            let mut room = capacity;
            while cursor < todo.len() && batch.len() < BATCH_CHUNKS && room > 0 {
                let key = todo[cursor];
                let fine = key.lod <= GRID_LODS;
                let size = if fine { 0 } else { summary_size(key.lod) };
                let (needed, fresh) = match &mut current {
                    Some((at, needed, fresh)) if *at == cursor => (needed, fresh),
                    _ => {
                        let mut needed = HashSet::new();
                        key.needed_tiles(&mut needed);
                        let needed: Vec<TilePos> = needed.into_iter().filter(|t| ground.contains(*t)).collect();
                        let fresh = needed
                            .iter()
                            .copied()
                            .filter(|t| if fine { !self.grids.contains_key(t) } else { !self.summaries.get(t).is_some_and(|e| e.holds(size)) })
                            .collect();
                        let (_, needed, fresh) = current.insert((cursor, needed, fresh));
                        (needed, fresh)
                    }
                };
                let reads = if fine { &mut fine_reads } else { &mut coarse_reads };
                fresh.retain(|t| reads.get(t).is_none_or(|&s| s < size));
                // A grid takes about twice the time of a summary.
                let cost = if fine { 2 } else { 1 };
                let take = fresh.len().min(room / cost);
                reads.extend(fresh.drain(fresh.len() - take..).map(|t| (t, size)));
                room -= take * cost;
                if !fresh.is_empty() {
                    break;
                }
                for t in needed.iter() {
                    if let Some(e) = self.grids.get_mut(t).filter(|_| fine) {
                        e.used = clock;
                    } else if let Some(e) = self.summaries.get_mut(t).filter(|_| !fine) {
                        e.used = clock;
                    }
                }
                batch.push(key);
                cursor += 1;
                current = None;
            }
            // Column data first, read in parallel.
            if !fine_reads.is_empty() {
                let built: Vec<(TilePos, Option<Arc<SurfaceGrid>>)> = fine_reads
                    .par_iter()
                    .map(|(&t, _)| (t, SurfaceGrid::build(dim, t).map(Arc::new)))
                    .collect();
                for (t, grid) in built {
                    self.grids.insert(t, GridEntry { grid, used: clock });
                }
            }
            if !coarse_reads.is_empty() {
                let built: Vec<(TilePos, Option<Arc<Summary>>)> = coarse_reads
                    .par_iter()
                    .map(|(&t, &size)| (t, Summary::build(dim, t, size).map(Arc::new)))
                    .collect();
                for (t, summary) in built {
                    let entry = SummaryEntry { summary, used: clock };
                    self.summary_bytes += entry.bytes();
                    if let Some(old) = self.summaries.insert(t, entry) {
                        self.summary_bytes -= old.bytes();
                    }
                }
            }
            if batch.is_empty() {
                continue;
            }

            // Then the images.
            let sources = Sources {
                grids: &self.grids,
                summaries: &self.summaries,
            };
            let images: Vec<(ChunkKey, ColorImage)> = batch
                .par_iter()
                .map(|&key| (key, build_chunk(key, &sources, &colorizer)))
                .collect();
            for (key, image) in images {
                match self.chunks.get_mut(&(style, key)) {
                    Some(entry) => {
                        entry.texture.set(image, TEXTURE_OPTIONS);
                        entry.stale = false;
                        entry.stale_since = 0;
                        entry.used = clock;
                    }
                    None => {
                        let name = format!("map {:?} {} {} {}", style.mode, key.lod, key.x, key.z);
                        let texture = ctx.load_texture(name, image, TEXTURE_OPTIONS);
                        self.chunks.insert(
                            (style, key),
                            ChunkEntry {
                                texture,
                                stale: false,
                                stale_since: 0,
                                used: clock,
                            },
                        );
                    }
                }
            }
            self.trim_grids();
        }
        self.trim_chunks();
        false
    }

    fn trim_grids(&mut self) {
        if self.grids.len() <= self.max_grids {
            return;
        }
        let mut by_age: Vec<(u64, TilePos)> = self.grids.iter().map(|(t, e)| (e.used, *t)).collect();
        by_age.sort_unstable();
        for (_, t) in by_age.iter().take(self.grids.len() - self.max_grids) {
            self.grids.remove(t);
        }
    }

    /// Shrink the summaries not used since `first` (the start of this
    /// update), least lately used first, while they take more than
    /// `max_summary_bytes`.
    fn trim_summaries(&mut self, first: u64) {
        if self.summary_bytes <= self.max_summary_bytes {
            return;
        }
        let mut by_age: Vec<(u64, TilePos)> = self
            .summaries
            .iter()
            .filter(|(_, e)| e.used < first && e.summary.as_ref().is_some_and(|s| s.size > SHRUNK_SIZE))
            .map(|(t, e)| (e.used, *t))
            .collect();
        by_age.sort_unstable();
        for (_, t) in by_age {
            if self.summary_bytes <= self.max_summary_bytes * 3 / 4 {
                break;
            }
            let e = self.summaries.get_mut(&t).expect("listed above");
            let s = e.summary.as_ref().expect("listed above");
            let small = s.shrunk(SHRUNK_SIZE);
            self.summary_bytes -= s.bytes() - small.bytes();
            e.summary = Some(Arc::new(small));
        }
    }

    fn trim_chunks(&mut self) {
        if self.chunks.len() <= self.max_chunks {
            return;
        }
        let mut by_age: Vec<(u64, (MapStyle, ChunkKey))> =
            self.chunks.iter().map(|(k, e)| (e.used, *k)).collect();
        by_age.sort_unstable_by_key(|(used, _)| *used);
        for (_, k) in by_age.iter().take(self.chunks.len() - self.max_chunks) {
            self.chunks.remove(k);
        }
    }

    /// Whether the last update left chunks to build.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Bytes the column data takes, and the chunks' textures on the
    /// graphics card.
    pub fn bytes(&self) -> (usize, usize) {
        (self.grids.len() * GRID_BYTES + self.summary_bytes, self.chunks.len() * CHUNK_BYTES)
    }

    pub fn stats(&self) -> (usize, usize, usize) {
        (self.chunks.len(), self.grids.len(), self.summaries.len())
    }
}

/// Render a rectangle of the map to an image without any caching: one texel
/// per `2^lod` blocks, `width × height` texels from block `(x0, z0)`. Used by
/// tests and snapshots.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn render_region(
    dim: &Dimension,
    reg: &Registry,
    sea: f32,
    style: MapStyle,
    lod: u8,
    x0: i32,
    z0: i32,
    chunks_x: i32,
    chunks_z: i32,
) -> ColorImage {
    let colorizer = Colorizer::new(reg, style, sea).with_structures(&dim.structures);
    let first = ChunkKey {
        lod,
        x: x0.div_euclid((CHUNK_PX as i32) << lod),
        z: z0.div_euclid((CHUNK_PX as i32) << lod),
    };
    let keys: Vec<ChunkKey> = (0..chunks_z)
        .flat_map(|j| (0..chunks_x).map(move |i| ChunkKey { lod, x: first.x + i, z: first.z + j }))
        .collect();
    let mut tiles = HashSet::new();
    for k in &keys {
        k.needed_tiles(&mut tiles);
    }
    let tiles: Vec<TilePos> = tiles.into_iter().filter(|t| dim.ground().contains(*t)).collect();
    let mut grids = HashMap::new();
    let mut summaries = HashMap::new();
    if lod <= GRID_LODS {
        grids = tiles
            .par_iter()
            .map(|&t| {
                (
                    t,
                    GridEntry {
                        grid: SurfaceGrid::build(dim, t).map(Arc::new),
                        used: 0,
                    },
                )
            })
            .collect();
    } else {
        summaries = tiles
            .par_iter()
            .map(|&t| (t, SummaryEntry { summary: Summary::build(dim, t, summary_size(lod)).map(Arc::new), used: 0 }))
            .collect();
    }
    let sources = Sources {
        grids: &grids,
        summaries: &summaries,
    };
    let (w, h) = (chunks_x as usize * CHUNK_PX, chunks_z as usize * CHUNK_PX);
    let mut out = ColorImage::filled([w, h], Color32::TRANSPARENT);
    for (n, key) in keys.iter().enumerate() {
        let image = build_chunk(*key, &sources, &colorizer);
        let (ci, cj) = (n % chunks_x as usize, n / chunks_x as usize);
        for y in 0..CHUNK_PX {
            let dst = (cj * CHUNK_PX + y) * w + ci * CHUNK_PX;
            out.pixels[dst..dst + CHUNK_PX].copy_from_slice(&image.pixels[y * CHUNK_PX..(y + 1) * CHUNK_PX]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, Landform, generate};
    use massif_core::history::History;
    use massif_core::coords::BlockRect;
    use massif_core::ops::{self, Anchor, Surface};
    use massif_core::{Brush, BrushShape, Profile, World};

    fn world() -> World {
        let mut w = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Islands,
            seed: 11,
            ..GenParams::default()
        });
        let mut h = History::default();
        h.begin("level", 0);
        let lvl = h.edit(&mut w, 0).structure().add_level("Island", LevelKind::Solid);
        h.commit();
        h.begin("stamp", 0);
        let brush = Brush {
            shape: BrushShape::Circle,
            radius: 40.0,
            profile: Profile::Dome,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        };
        ops::stamp_slab(&mut h.edit(&mut w, lvl), &brush, 60.0, -60.0, Anchor::Absolute(140.0), 30.0);
        h.commit();
        w
    }

    fn save(name: &str, image: &ColorImage) {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR") else {
            return;
        };
        let path = std::path::PathBuf::from(dir).join(name);
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        image::save_buffer(&path, &rgba, image.width() as u32, image.height() as u32, image::ColorType::Rgba8)
            .unwrap();
    }

    /// Close up, each block is shaded against its neighbour on the side
    /// the light comes from: a step is one block wide, ground a fraction of
    /// a block higher looks the same, and a block alone above the others
    /// shows. Smooth, a step spreads over both blocks and the fraction shows.
    #[test]
    fn close_up_each_block_shows_its_own_step() {
        let w = world();
        let at = |height: f32| Sample { height, levels: 1, ..Sample::EMPTY };
        for smooth in [false, true] {
            // The light from the north-west, the default.
            let colorizer = Colorizer::new(&w.registry, MapStyle { smooth, ..MapStyle::default() }, 63.0);
            // Block `k` of a row from west to east, its west neighbour `lift` blocks higher.
            let pixel = |row: &[f32], k: usize, lift: f32| {
                colorizer.pixel(&at(row[k]), None, &at(row[k - 1] + lift), &at(row[k + 1]), &at(row[k]), &at(row[k]), 1.0, 1.0, None, None, None)
            };
            let flat = pixel(&[70.0; 3], 1, 0.0);
            let step = [70.0, 70.0, 71.0, 71.0];
            let (below, top) = (pixel(&step, 1, 0.0), pixel(&step, 2, 0.0));
            if smooth {
                assert!(below != flat && top != flat, "smooth: both sides of the step shaded");
                assert_ne!(pixel(&step, 1, 0.3), below, "smooth: a fraction of a block shows");
            } else {
                assert_eq!(below, flat, "the block at the foot of the step, before it, stays flat");
                assert!(top.r() > flat.r(), "the block at the top of the step, facing the light, is lit");
                assert_eq!(pixel(&step, 1, 0.3), below, "a fraction of a block does not show");
                let bump = [70.0, 71.0, 70.0, 70.0];
                assert!(pixel(&bump, 1, 0.0).r() > flat.r(), "a block alone above the others is lit");
                assert!(pixel(&bump, 2, 0.0).r() < flat.r(), "the block east of it is in its shadow");
            }
        }
    }

    /// Out to a texel of 4 × 4 blocks, a one-block step keeps its line: a
    /// texel holding a step facing the light is lit, its neighbours without
    /// one are not, where the light worked out from the texels around (the
    /// smooth way, as before) is the same over the whole hillside. The mean
    /// light of the hillside stays what it was.
    #[test]
    fn a_texel_of_several_blocks_keeps_its_steps() {
        let w = world();
        for (lod, every) in [(1u8, 4i32), (2, 8)] {
            // Ground rising a block every `every` blocks eastward.
            let mut grids = HashMap::new();
            for tz in -1..=4 {
                for tx in -1..=4 {
                    let samples = (0..TILE_AREA)
                        .map(|i| {
                            let x = tx * TILE_SIZE + (i % TILE_SIZE as usize) as i32;
                            Sample { height: 64.0 + x.div_euclid(every) as f32, levels: 1, ..Sample::EMPTY }
                        })
                        .collect();
                    grids.insert(TilePos::new(tx, tz), GridEntry { grid: Some(Arc::new(SurfaceGrid(samples))), used: 0 });
                }
            }
            let summaries = HashMap::new();
            let sources = Sources { grids: &grids, summaries: &summaries };
            let key = ChunkKey { lod, x: 0, z: 0 };
            let row = |smooth: bool| -> Vec<f32> {
                let colorizer = Colorizer::new(&w.registry, MapStyle { smooth, ..MapStyle::default() }, 63.0);
                let image = build_chunk(key, &sources, &colorizer);
                (8..40).map(|i| image.pixels[64 * CHUNK_PX + i].r() as f32).collect()
            };
            let (steps, smooth) = (row(false), row(true));
            let spread = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
            assert!(spread(&smooth) <= 1.0, "lod {lod}: smooth, the hillside is one shade: {smooth:?}");
            assert!(spread(&steps) >= 8.0, "lod {lod}: block by block, the steps show: {steps:?}");
            // Every other texel holds a step: lit, then not.
            assert!(steps.windows(2).all(|p| (p[0] - p[1]).abs() >= 8.0), "lod {lod}: {steps:?}");
            let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
            assert!((mean(&steps) - mean(&smooth)).abs() <= 3.0, "lod {lod}: the hillside's mean light: {} against {}", mean(&steps), mean(&smooth));
            eprintln!("lod {lod}: steps {:?}, smooth {:?}", &steps[..4], &smooth[..4]);
        }
    }

    /// Block by block, a gentle hillside is as bright on the whole as it
    /// is shaded smooth, facing the light or away from it, and its steps
    /// still show.
    #[test]
    fn steps_keep_the_map_as_bright() {
        for step in [0u8, 3, 8] {
            let l = light_vector(step);
            let flat = hillshade(0.0, 0.0, l);
            for every in [4.0f32, 8.0] {
                for (ux, uz) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    // A step every `every` blocks: the smooth slope 1 / every.
                    let s = 1.0 / every;
                    let smooth = hillshade(ux * s, uz * s, l);
                    let lit = step_shade(ux, uz, l);
                    if lit >= 1.32 || lit <= 0.3 {
                        // A light low enough that a step reaches the
                        // brightest or darkest a slope gets.
                        continue;
                    }
                    let steps = (lit + (every - 1.0) * flat) / every;
                    // (Within the curve of the smooth light, which a step
                    // of a block the same everywhere cannot follow.)
                    assert!((steps - smooth).abs() < 0.025, "light {step}, every {every}, way {ux},{uz}: steps {steps}, smooth {smooth}");
                    if (ux * l[0] + uz * l[2]).abs() > 0.3 {
                        assert!((step_shade(ux, uz, l) - flat).abs() > 0.2, "the step shows");
                    }
                }
            }
        }
    }

    #[test]
    fn samples_see_the_level_stack() {
        let w = world();
        let dim = &w.dims[0];
        let grid = SurfaceGrid::build(dim, TilePos::new(0, -1)).unwrap();
        let island = grid.0[local_index(60, -60)];
        assert!((island.height - 155.0).abs() < 0.01, "{}", island.height);
        assert_eq!(island.flags & FLAG_FLOATING, FLAG_FLOATING);
        assert_eq!(island.levels, 0b11);
        assert_eq!(island.fluid, NO_FLUID);
        let plain = grid.0[local_index(5, -120)];
        assert_eq!(plain.levels, 0b01);
        assert_eq!(plain.flags, 0);
        assert!(SurfaceGrid::build(dim, TilePos::new(10, 10)).is_none());
        let summary = Summary::build(dim, TilePos::new(0, -1), SUMMARY_SIZE).unwrap();
        // Summary sample (7, 7) is column (60, -68 + 128 = 60 within the tile).
        let s = summary.samples[7 * SUMMARY_SIZE + 7];
        assert!(s.flags & FLAG_FLOATING != 0, "{s:?}");
        assert_eq!(summary.at(60, 60), s);
        // A smaller summary holds some of the same columns, read the same.
        for size in [8, 4, 2, 1] {
            let small = Summary::build(dim, TilePos::new(0, -1), size).unwrap();
            let shrunk = summary.shrunk(size);
            assert_eq!(small.samples, shrunk.samples, "size {size}");
            let step = 128 / size as i64;
            for l in [4, 4 + step * (size as i64 - 1)] {
                assert_eq!(small.at(l, 4), summary.at(l, 4), "size {size}, column {l}");
            }
        }
    }

    #[test]
    fn chunks_render_without_seams_at_every_level_of_detail() {
        let w = world();
        let dim = &w.dims[0];
        for (lod, chunks) in [(0u8, 4), (2, 1), (3, 1)] {
            let style = MapStyle {
                contours: true,
                ..MapStyle::default()
            };
            let img = render_region(dim, &w.registry, 63.0, style, lod, -256, -256, chunks, chunks);
            save(&format!("map-terrain-lod{lod}.png"), &img);
            let size = img.width();
            // Inside the map every texel is opaque; outside, transparent.
            let span = (CHUNK_PX as i32) << lod;
            let origin = (-256i32).div_euclid(span) * span;
            let inside = ((-100 - origin) >> lod) as usize;
            let outside = ((-300 - origin).max(0) >> lod) as usize;
            assert_eq!(img.pixels[inside * size + inside].a(), 255, "lod {lod}");
            if -300 >= origin {
                assert_eq!(img.pixels[outside * size + outside].a(), 0, "lod {lod}");
            }
            if lod == 0 {
                // Tile borders at x = 128 * k must not show a shading seam:
                // compare the average jump across a border with the average
                // jump between two inner columns.
                let jump = |x: usize| -> f32 {
                    (0..size)
                        .map(|y| {
                            let a = img.pixels[y * size + x - 1];
                            let b = img.pixels[y * size + x];
                            (a.r() as f32 - b.r() as f32).abs()
                        })
                        .sum::<f32>()
                        / size as f32
                };
                let border = (jump(128) + jump(256) + jump(384)) / 3.0;
                let inner = (jump(64) + jump(200) + jump(330)) / 3.0;
                assert!(border < inner * 2.0 + 2.0, "border {border} vs inner {inner}");
            }
        }
        for mode in MapMode::ALL {
            let style = MapStyle {
                mode,
                highlight: Some(1),
                ..MapStyle::default()
            };
            let img = render_region(dim, &w.registry, 63.0, style, 1, -256, -256, 2, 2);
            save(&format!("map-{}.png", mode.label().to_lowercase()), &img);
            let distinct: HashSet<Color32> = img.pixels.iter().copied().collect();
            assert!(distinct.len() > 50, "{mode:?} has only {} colours", distinct.len());
        }
        // Outside the map nothing is drawn.
        let img = render_region(dim, &w.registry, 63.0, MapStyle::default(), 0, 4096, 4096, 1, 1);
        assert!(img.pixels.iter().all(|c| c.a() == 0));
    }

    /// Underground pockets show over the map, hatched in their layer's
    /// colour, only while they are painted (`MapStyle::pockets`), and only
    /// where they are.
    #[test]
    fn pockets_show_hatched_while_painted() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 7, ..GenParams::default() });
        let granite = w.registry.intern_block("minecraft:granite", [149, 103, 85]);
        let red = w.registry.add_pocket(massif_core::pockets::PocketDef::new("Red", [220, 30, 30], &[(granite, 1)], 0.3, 4.0, -64, 60));
        let hard = Brush { shape: BrushShape::Circle, radius: 20.0, profile: Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let mut h = History::default();
        h.begin("paint", 0);
        ops::paint_pockets(&mut h.edit(&mut w, 0), &hard, -40.0, -40.0, red, 1);
        h.commit();
        let dim = &w.dims[0];
        let reddish = |img: &ColorImage, x0: i32, z0: i32| {
            let mut n = 0;
            for z in z0..z0 + 8 {
                for x in x0..x0 + 8 {
                    let c = img.pixels[(z + 128) as usize * img.width() + (x + 128) as usize];
                    n += (c.r() as i32 > c.g() as i32 + 50) as usize;
                }
            }
            n
        };
        let shown = render_region(dim, &w.registry, 63.0, MapStyle { pockets: true, ..MapStyle::default() }, 0, -128, -128, 2, 2);
        save("map-pockets.png", &shown);
        let plain = render_region(dim, &w.registry, 63.0, MapStyle::default(), 0, -128, -128, 2, 2);
        let (inside, outside, hidden) = (reddish(&shown, -44, -44), reddish(&shown, 40, 40), reddish(&plain, -44, -44));
        assert!((16..=48).contains(&inside), "hatched: about half of the pixels, {inside} of 64");
        assert_eq!((outside, hidden), (0, 0), "nothing where they are not painted, nor while they are not");
    }

    /// WorldPainter's Frost, Lava and annotations as the map shows them:
    /// ice over the water painted with Frost, lava where Lava is painted,
    /// open water around; the colour of an annotation over the land, gone
    /// when annotations are hidden.
    #[test]
    fn frost_lava_and_annotations_show_on_the_map() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 7, ..GenParams::default() });
        let hard = |radius: f32| Brush { shape: BrushShape::Circle, radius, profile: Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let reg = &w.registry;
        let (frost, lava) = (reg.cover_by_name("Frost").unwrap(), reg.cover_by_name("Lava").unwrap());
        let mut h = History::default();
        h.begin("paint", 0);
        {
            let mut e = h.edit(&mut w, 0);
            ops::set_height(&mut e, &hard(30.0), -60.0, -60.0, 50.0, Surface::Top);
            ops::set_height(&mut e, &hard(20.0), 60.0, 60.0, 90.0, Surface::Top);
            ops::paint_cover(&mut e, &hard(10.0), -70.0, -60.0, frost, 1);
            ops::paint_cover(&mut e, &hard(8.0), -45.0, -60.0, lava, 2);
            ops::paint_annotation(&mut e, &hard(8.0), 60.0, 60.0, 14, 3);
        }
        h.commit();
        let dim = &w.dims[0];
        let at = |img: &ColorImage, x: i32, z: i32| img.pixels[(z + 128) as usize * img.width() + (x + 128) as usize];
        let img = render_region(dim, &w.registry, 63.0, MapStyle::default(), 0, -128, -128, 2, 2);
        save("map-frost-lava-annotations.png", &img);
        let (ice, lava, water, mark) = (at(&img, -70, -60), at(&img, -45, -60), at(&img, -60, -40), at(&img, 60, 60));
        assert!(ice.r() > 170 && ice.b() > 200 && ice.b() >= ice.r(), "ice {ice:?}");
        assert!(lava.r() > 180 && lava.b() < 90, "lava {lava:?}");
        assert!(water.b() > water.r() + 40 && water.r() < 120, "water {water:?}");
        // Red over the land.
        assert!(mark.r() > mark.g() + 30 && mark.r() > mark.b() + 30, "annotation {mark:?}");
        let hidden = render_region(dim, &w.registry, 63.0, MapStyle { annotations: false, ..MapStyle::default() }, 0, -128, -128, 2, 2);
        let land = at(&hidden, 60, 60);
        assert!(land.g() >= land.r(), "the land without its annotation {land:?}");
        assert_eq!(at(&hidden, -70, -60), ice, "only the annotations go");
    }

    #[test]
    fn colour_ramps_are_continuous() {
        let mut prev = height_color(-200.0, 63.0);
        for h in -199..400 {
            let c = height_color(h as f32, 63.0);
            let jump = (0..3).map(|i| (c[i] as i32 - prev[i] as i32).abs()).max().unwrap();
            // The shoreline is the one intended discontinuity.
            if h != 64 {
                assert!(jump <= 12, "jump of {jump} at {h}");
            }
            prev = c;
        }
        assert_eq!(slope_color(0.0), [70, 168, 88]);
        assert_eq!(slope_color(90.0), [86, 26, 70]);
    }

    #[test]
    fn the_light_can_turn() {
        let nw = light_vector(DEFAULT_LIGHT);
        assert!((nw[0] + 0.5).abs() < 1e-6 && (nw[2] + 0.5).abs() < 1e-6);
        let south = light_vector(8);
        assert!(south[0].abs() < 1e-6 && (south[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // A slope facing north is lit from the north and dark from the south.
        let (dx, dz) = (0.0, 1.0);
        assert!(hillshade(dx, dz, light_vector(0)) > 1.0);
        assert!(hillshade(dx, dz, light_vector(8)) < 1.0);
        assert_eq!(hillshade(0.0, 0.0, south), 1.0);
    }

    #[test]
    fn chunk_keys() {
        assert_eq!(ChunkKey::of_tile(TilePos::new(-1, 5), 0), ChunkKey { lod: 0, x: -1, z: 5 });
        assert_eq!(ChunkKey::of_tile(TilePos::new(-1, 5), 2), ChunkKey { lod: 2, x: -1, z: 1 });
        assert_eq!(ChunkKey::of_tile(TilePos::new(-4, -5), 2), ChunkKey { lod: 2, x: -1, z: -2 });
        let k = ChunkKey { lod: 3, x: -2, z: 1 };
        assert_eq!(k.blocks(), 1024);
        assert_eq!(k.origin(), (-2048, 1024));
        assert_eq!(k.parent(), ChunkKey { lod: 4, x: -1, z: 0 });
    }

    /// What reading a huge map's tiles for the map view costs, one thread
    /// (MASSIF_HUGE_WORLD: a .massif): a summary and a grid of columns a
    /// tile, best of three over tiles spread over the map.
    #[test]
    fn reading_a_huge_map() {
        let Some(path) = std::env::var_os("MASSIF_HUGE_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let dim = &world.dims[0];
        let tiles: Vec<TilePos> = dim.ground().positions().step_by(97).take(500).collect();
        let time = |f: &dyn Fn(TilePos)| {
            (0..3)
                .map(|_| {
                    let t = Instant::now();
                    for &p in &tiles {
                        f(p);
                    }
                    t.elapsed().as_secs_f64() * 1000.0 / tiles.len() as f64
                })
                .fold(f64::MAX, f64::min)
        };
        eprintln!("{} tiles: summary {:.3} ms a tile", tiles.len(), time(&|p| drop(Summary::build(dim, p, SUMMARY_SIZE))));
        eprintln!("grid {:.3} ms a tile", time(&|p| drop(SurfaceGrid::build(dim, p))));
        eprintln!("decode only {:.3} ms a tile", time(&|p| drop(TileStack::new(dim, p, false))));
        // A chunk's image, from column data already read.
        for (smooth, lod) in [false, true].into_iter().flat_map(|s| [0u8, 1, 2, 4].map(|l| (s, l))) {
            let style = MapStyle { contours: true, smooth, ..MapStyle::default() };
            let colorizer = Colorizer::new(&world.registry, style, world.meta.sea_level as f32);
            let key = ChunkKey::of_tile(tiles[tiles.len() / 2], lod);
            let mut needed = HashSet::new();
            key.needed_tiles(&mut needed);
            let grids = needed.iter().map(|&t| (t, GridEntry { grid: SurfaceGrid::build(dim, t).map(Arc::new), used: 0 })).collect();
            let summaries = needed.iter().map(|&t| (t, SummaryEntry { summary: Summary::build(dim, t, summary_size(lod)).map(Arc::new), used: 0 })).collect();
            let sources = Sources { grids: &grids, summaries: &summaries };
            let best = (0..5)
                .map(|_| {
                    let t = Instant::now();
                    drop(build_chunk(key, &sources, &colorizer));
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .fold(f64::MAX, f64::min);
            eprintln!("chunk at lod {lod}{}: {best:.3} ms", if smooth { ", smooth" } else { "" });
            if !smooth && (1..=STEP_LODS).contains(&lod) {
                let best = (0..20)
                    .map(|_| {
                        let t = Instant::now();
                        drop(block_step_shades(key, &sources, colorizer.light));
                        t.elapsed().as_secs_f64() * 1000.0
                    })
                    .fold(f64::MAX, f64::min);
                eprintln!("  of which the block steps: {best:.3} ms");
            }
        }
    }

    /// The huge map (MASSIF_HUGE_WORLD) at every coarse level of detail,
    /// around its middle, in every mode, saved to MASSIF_SNAPSHOT_DIR to
    /// compare renders before and after a change.
    #[test]
    fn a_huge_map_at_every_level() {
        let Some(path) = std::env::var_os("MASSIF_HUGE_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        let mid = ((lo.x + hi.x) / 2 * 128, (lo.z + hi.z) / 2 * 128);
        for mode in MapMode::ALL {
            for lod in 0..=8u8 {
                let style = MapStyle { mode, ..MapStyle::default() };
                let n = 2;
                let span = (CHUNK_PX << lod) as i32;
                let image = render_region(dim, &world.registry, world.meta.sea_level as f32, style, lod, mid.0 - span, mid.1 - span, n, n);
                save(&format!("huge-{mode:?}-lod{lod}.png"), &image);
            }
        }
    }

    /// A brush held in place redraws every part of the map under it, not
    /// only those nearest the middle of the view: each frame marks them all
    /// out of date again, and a frame's time draws only some, so those
    /// waiting longest go first. Before, the ones farthest from the middle
    /// of the view stayed as they were for as long as the brush was held,
    /// squares of the land of before inside the brush.
    #[test]
    fn a_held_brush_redraws_everything_under_it() {
        let mut w = generate(&GenParams {
            tiles_x: 16,
            tiles_z: 16,
            landform: Landform::Mountains,
            seed: 5,
            ..GenParams::default()
        });
        let ctx = egui::Context::default();
        let style = MapStyle::default();
        // The whole map at lod 1, nearest the middle of the view first.
        let lod = 1;
        let (lo, hi) = w.dims[0].bounds().expect("a map");
        let mut keys: Vec<ChunkKey> = (lo.z >> lod..=hi.z >> lod).flat_map(|z| (lo.x >> lod..=hi.x >> lod).map(move |x| ChunkKey { lod, x, z })).collect();
        keys.sort_by_key(|k| (k.x * 2 + 1).pow(2) + (k.z * 2 + 1).pow(2));
        let mut cache = MapCache::default();
        while cache.update(&ctx, &w.dims[0], &w.registry, 63.0, style, &keys, Duration::from_secs(1)) {}
        w.dims[0].levels[0].take_dirty();
        let brush = Brush {
            shape: BrushShape::Circle,
            radius: 506.0,
            profile: Profile::Sine,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        };
        // Held off the middle, a frame's time drawing one batch at most.
        let (cx, cz) = (380.0, 250.0);
        let under: Vec<ChunkKey> = keys.iter().copied().filter(|k| BlockRect::around(cx, cz, 506.0).tiles().any(|t| ChunkKey::of_tile(t, lod) == *k)).collect();
        assert!(under.len() > BATCH_CHUNKS * 2, "more than a frame draws: {}", under.len());
        let mut hist = History::default();
        hist.begin("raise", 0);
        let frames = 40;
        let mut drawn = HashMap::new();
        for frame in 0..frames {
            ops::raise(&mut hist.edit(&mut w, 0), &brush, cx, cz, 0.2, Surface::Top);
            let dirty = w.dims[0].levels[0].take_dirty();
            cache.invalidate(&dirty);
            cache.update(&ctx, &w.dims[0], &w.registry, 63.0, style, &keys, Duration::from_nanos(1));
            for k in &under {
                if cache.is_ready(style, *k) {
                    drawn.insert(*k, frame);
                }
            }
        }
        hist.commit();
        let late: Vec<_> = under.iter().filter(|k| drawn.get(k).is_none_or(|&f| f + 3 * under.len().div_ceil(BATCH_CHUNKS) < frames)).collect();
        assert!(late.is_empty(), "{} of {} parts under the brush not redrawn lately: {late:?}", late.len(), under.len());
    }

    /// A map (MASSIF_CLOSE_WORLD) close up, at the block MASSIF_CLOSE_AT
    /// (`x,z`, its middle by default), at the four finest levels of detail,
    /// with block steps and smooth, saved to MASSIF_SNAPSHOT_DIR.
    #[test]
    fn a_map_close_up() {
        let Some(path) = std::env::var_os("MASSIF_CLOSE_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        let at = std::env::var("MASSIF_CLOSE_AT").ok().and_then(|v| v.split_once(',').map(|(x, z)| (x.trim().parse().unwrap(), z.trim().parse().unwrap())));
        let (x, z) = at.unwrap_or(((lo.x + hi.x) / 2 * 128, (lo.z + hi.z) / 2 * 128));
        // The whole map, a texel per 32 blocks, to find a place.
        let (x0, z0) = (lo.x * 128, lo.z * 128);
        let n = |a: i32, b: i32| ((b - a + 1) as usize * 128).div_ceil(CHUNK_PX << 5) as i32;
        eprintln!("map from {x0},{z0}, 32 blocks a pixel");
        save("close-whole.png", &render_region(dim, &world.registry, world.meta.sea_level as f32, MapStyle::default(), 5, x0, z0, n(lo.x, hi.x), n(lo.z, hi.z)));
        for smooth in [false, true] {
            for lod in 0..=3u8 {
                let style = MapStyle { contours: true, smooth, ..MapStyle::default() };
                let span = (CHUNK_PX << lod) as i32;
                let image = render_region(dim, &world.registry, world.meta.sea_level as f32, style, lod, x - span, z - span, 2, 2);
                save(&format!("close-{}-lod{lod}.png", if smooth { "smooth" } else { "steps" }), &image);
                if lod == 0 {
                    // The middle, 8 pixels a block, as the map close up.
                    let (n, k) = (64, 8);
                    let pixels = (0..n * k * n * k).map(|p| image.pixels[(96 + p / (n * k) / k) * 256 + 96 + p % (n * k) / k]).collect();
                    save(&format!("close-{}-zoomed.png", if smooth { "smooth" } else { "steps" }), &ColorImage::new([n * k, n * k], pixels));
                }
            }
        }
    }
}
