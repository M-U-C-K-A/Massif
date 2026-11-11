//! The CPU half of the map: what each column looks like from above, coloured
//! for the active display mode and assembled into texture chunks at several
//! levels of detail.
//!
//! A chunk at level of detail `lod` is 128×128 texels, one texel per `2^lod`
//! blocks, so it covers `2^lod × 2^lod` tiles. Chunks at `lod <= 2` read
//! full-resolution [`SurfaceGrid`]s; coarser chunks read 16×16 summaries, so a
//! fully zoomed-out 16k map only ever touches a few kilobytes per tile.

use egui::{Color32, ColorImage, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode};
use massif_core::compose::{Span, TileStack, block_top};
use massif_core::coords::{TILE_AREA, TilePos, cell_index, local_index};
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
const SUMMARY_SIZE: usize = 16;
const SUMMARY_STEP: usize = 128 / SUMMARY_SIZE;

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
fn ground_sample(top: f32, terrain: u16, fluid: i16, biome: u16) -> Sample {
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
            let (fluid, biome) = (g.fluid.dense(), g.biome.dense());
            (0..TILE_AREA)
                .map(|i| ground_sample(top[i], terrain[i], fluid[i], biome[i]))
                .collect()
        } else {
            let mut sampler = ColumnSampler::default();
            (0..TILE_AREA).map(|i| sampler.sample(&stack, i)).collect()
        };
        Some(SurfaceGrid(samples))
    }
}

/// Every 8th column of a tile in each direction.
pub struct Summary(pub [Sample; SUMMARY_SIZE * SUMMARY_SIZE]);

impl Summary {
    pub fn build(dim: &Dimension, pos: TilePos) -> Option<Summary> {
        let stack = TileStack::new(dim, pos, false)?;
        let mut sampler = ColumnSampler::default();
        let mut out = [Sample::EMPTY; SUMMARY_SIZE * SUMMARY_SIZE];
        for j in 0..SUMMARY_SIZE {
            for i in 0..SUMMARY_SIZE {
                let cell = cell_index(i * SUMMARY_STEP + SUMMARY_STEP / 2, j * SUMMARY_STEP + SUMMARY_STEP / 2);
                out[j * SUMMARY_SIZE + i] = sampler.sample(&stack, cell);
            }
        }
        Some(Summary(out))
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
            solo: None,
            highlight: None,
            light: DEFAULT_LIGHT,
            lava: false,
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

    pub const fn label(self) -> &'static str {
        match self {
            MapMode::Terrain => "Terrain",
            MapMode::Height => "Height",
            MapMode::Slope => "Slope",
            MapMode::Biome => "Biome",
            MapMode::Relief => "Relief",
        }
    }
}

/// Everything that changes the colour of a texel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MapStyle {
    pub mode: MapMode,
    pub contours: bool,
    /// Terrain shown normally while all others are faded.
    pub solo: Option<u16>,
    /// Level whose footprint is tinted.
    pub highlight: Option<u8>,
    /// Where the light of the shading comes from, in steps of 22.5°
    /// clockwise from north (14 = north-west).
    pub light: u8,
    /// The world's fluid is lava.
    pub lava: bool,
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
    auto: AutoBiome,
    light: [f32; 3],
}

impl<'a> Colorizer<'a> {
    pub fn new(reg: &'a Registry, style: MapStyle, sea: f32) -> Self {
        Self {
            reg,
            style,
            sea,
            light: light_vector(style.light),
            terrain: reg.terrains.iter().map(|t| rgb(t.color)).collect(),
            biome: reg.biomes.iter().map(|b| rgb(b.color)).collect(),
            auto: AutoBiome::new(reg),
        }
    }

    fn terrain_rgb(&self, id: u16) -> Rgb {
        self.terrain
            .get(id as usize)
            .copied()
            .unwrap_or([118.0, 118.0, 118.0])
    }

    /// Colour of the centre sample `c` given its four neighbours, `spacing`
    /// blocks apart.
    #[allow(clippy::too_many_arguments)]
    pub fn pixel(
        &self,
        c: &Sample,
        l: &Sample,
        r: &Sample,
        u: &Sample,
        d: &Sample,
        spacing: f32,
        exaggeration: f32,
        contour: Option<f32>,
    ) -> Color32 {
        if c.is_empty() {
            return Color32::TRANSPARENT;
        }
        let h = |s: &Sample| if s.is_empty() { c.height } else { s.height };
        let dx = (h(r) - h(l)) / (2.0 * spacing);
        let dz = (h(d) - h(u)) / (2.0 * spacing);
        let shade = hillshade(dx * exaggeration, dz * exaggeration, self.light);

        let mut out = match self.style.mode {
            MapMode::Terrain => scale(self.terrain_rgb(c.terrain), shade),
            MapMode::Height => scale(rgb(height_color(c.height, self.sea)), 0.7 + 0.3 * shade),
            MapMode::Slope => {
                let deg = (dx * dx + dz * dz).sqrt().atan().to_degrees();
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
            let water = if self.style.lava {
                [214.0, 96.0, 18.0]
            } else if self.style.mode == MapMode::Relief {
                [70.0, 110.0, 170.0]
            } else {
                WATER
            };
            out = mix(out, water, 0.5 + 0.42 * t);
        }

        if let Some(interval) = contour {
            let band = |v: f32| (v / interval).floor();
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
        let x = x0 + i as i64 * step + step / 2;
        let z = z0 + j as i64 * step + step / 2;
        (x.clamp(i32::MIN as i64, i32::MAX as i64) as i32, z.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }

    /// Tiles whose data the chunk reads.
    fn needed_tiles(&self, out: &mut HashSet<TilePos>) {
        let n = CHUNK_PX as isize;
        if self.lod <= 6 {
            // Every tile under the chunk plus a ring for the shading.
            let (a, b) = (self.sample_point(-1, -1), self.sample_point(n, n));
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

/// Read access to the cached column data while chunks are built.
struct Sources<'a> {
    grids: &'a HashMap<TilePos, GridEntry>,
    summaries: &'a HashMap<TilePos, Option<Arc<Summary>>>,
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
            match self.summaries.get(&pos).and_then(Option::as_ref) {
                Some(s) => {
                    let i = x.rem_euclid(128) as usize / SUMMARY_STEP;
                    let j = z.rem_euclid(128) as usize / SUMMARY_STEP;
                    s.0[j * SUMMARY_SIZE + i]
                }
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

fn build_chunk(key: ChunkKey, sources: &Sources, colorizer: &Colorizer) -> ColorImage {
    const W: usize = CHUNK_PX + 2;
    let mut samples = vec![Sample::EMPTY; W * W];
    for j in 0..W {
        for i in 0..W {
            let (x, z) = key.sample_point(i as isize - 1, j as isize - 1);
            samples[j * W + i] = sources.sample(key.lod, x, z);
        }
    }
    let spacing = (1u32 << key.lod) as f32;
    let exaggeration = 1.0 + 0.3 * key.lod as f32;
    let contour = colorizer.style.contours.then(|| contour_interval(key.lod));
    let mut pixels = Vec::with_capacity(CHUNK_PX * CHUNK_PX);
    for j in 1..=CHUNK_PX {
        for i in 1..=CHUNK_PX {
            let at = |di: isize, dj: isize| &samples[(j as isize + dj) as usize * W + (i as isize + di) as usize];
            pixels.push(colorizer.pixel(
                at(0, 0),
                at(-1, 0),
                at(1, 0),
                at(0, -1),
                at(0, 1),
                spacing,
                exaggeration,
                contour,
            ));
        }
    }
    ColorImage::new([CHUNK_PX, CHUNK_PX], pixels)
}

// ------------------------------------------------------------------ cache --

struct GridEntry {
    grid: Option<Arc<SurfaceGrid>>,
    used: u64,
}

pub struct ChunkEntry {
    pub texture: TextureHandle,
    pub stale: bool,
    used: u64,
}

/// Map textures and the column data they are built from.
pub struct MapCache {
    generation: u64,
    clock: u64,
    grids: HashMap<TilePos, GridEntry>,
    summaries: HashMap<TilePos, Option<Arc<Summary>>>,
    chunks: HashMap<(MapStyle, ChunkKey), ChunkEntry>,
    /// Grids kept around (≈ 190 KiB each).
    pub max_grids: usize,
    /// Chunk textures kept around (64 KiB each).
    pub max_chunks: usize,
}

impl Default for MapCache {
    fn default() -> Self {
        Self {
            generation: u64::MAX,
            clock: 0,
            grids: HashMap::new(),
            summaries: HashMap::new(),
            chunks: HashMap::new(),
            max_grids: 384,
            max_chunks: 1400,
        }
    }
}

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
        let mut affected = HashSet::new();
        for &t in tiles {
            self.grids.remove(&t);
            self.summaries.remove(&t);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = t.offset(dx, dz);
                    for lod in 0..=MAX_LOD {
                        affected.insert(ChunkKey::of_tile(n, lod));
                    }
                }
            }
        }
        for ((_, key), entry) in self.chunks.iter_mut() {
            if affected.contains(key) {
                entry.stale = true;
            }
        }
    }

    /// Everything is out of date (level visibility or order changed).
    pub fn invalidate_all(&mut self) {
        self.grids.clear();
        self.summaries.clear();
        for entry in self.chunks.values_mut() {
            entry.stale = true;
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
        let started = Instant::now();
        let todo: Vec<ChunkKey> = wanted
            .iter()
            .copied()
            .filter(|k| !self.is_ready(style, *k))
            .collect();
        if todo.is_empty() {
            return false;
        }
        let colorizer = Colorizer::new(reg, style, sea);
        let ground = dim.ground();
        let mut cursor = 0;
        while cursor < todo.len() {
            if started.elapsed() > budget {
                return true;
            }
            // Pick a batch whose data stays within the per-batch tile limits.
            let mut batch = Vec::new();
            let mut tiles = HashSet::new();
            while cursor < todo.len() && batch.len() < 32 {
                let key = todo[cursor];
                let mut needed = HashSet::new();
                key.needed_tiles(&mut needed);
                needed.retain(|t| ground.contains(*t));
                let fresh = needed.difference(&tiles).count();
                if !batch.is_empty() && tiles.len() + fresh > 2048 {
                    break;
                }
                tiles.extend(needed);
                batch.push(key);
                cursor += 1;
            }

            // Column data first, decoded in parallel.
            self.clock += 1;
            let clock = self.clock;
            let (fine, coarse): (Vec<ChunkKey>, Vec<ChunkKey>) = batch.iter().copied().partition(|k| k.lod <= GRID_LODS);
            if !fine.is_empty() {
                let mut need = HashSet::new();
                for k in &fine {
                    k.needed_tiles(&mut need);
                }
                let missing: Vec<TilePos> = need
                    .into_iter()
                    .filter(|t| ground.contains(*t))
                    .filter(|t| match self.grids.get_mut(t) {
                        Some(e) => {
                            e.used = clock;
                            false
                        }
                        None => true,
                    })
                    .collect();
                let built: Vec<(TilePos, Option<Arc<SurfaceGrid>>)> = missing
                    .par_iter()
                    .map(|&t| (t, SurfaceGrid::build(dim, t).map(Arc::new)))
                    .collect();
                for (t, grid) in built {
                    self.grids.insert(t, GridEntry { grid, used: clock });
                }
            }
            if !coarse.is_empty() {
                let mut need = HashSet::new();
                for k in &coarse {
                    k.needed_tiles(&mut need);
                }
                let missing: Vec<TilePos> = need
                    .into_iter()
                    .filter(|t| ground.contains(*t) && !self.summaries.contains_key(t))
                    .collect();
                let built: Vec<(TilePos, Option<Arc<Summary>>)> = missing
                    .par_iter()
                    .map(|&t| (t, Summary::build(dim, t).map(Arc::new)))
                    .collect();
                self.summaries.extend(built);
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
    let colorizer = Colorizer::new(reg, style, sea);
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
            .map(|&t| (t, Summary::build(dim, t).map(Arc::new)))
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
    use massif_core::ops::{self, Anchor};
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
        let summary = Summary::build(dim, TilePos::new(0, -1)).unwrap();
        // Summary sample (7, 7) is column (60, -68 + 128 = 60 within the tile).
        let s = summary.0[7 * SUMMARY_SIZE + 7];
        assert!(s.flags & FLAG_FLOATING != 0, "{s:?}");
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
}
