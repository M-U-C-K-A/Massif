//! What kernels work with: the context of a computation, grids of samples,
//! and the shapes of the functions nodes provide.

use crate::params::Params;
use crate::space::{Rect, Space};
use std::sync::Arc;

/// What a kernel knows of the computation it is part of.
pub struct Ctx<'a> {
    pub space: &'a Space,
    /// The level of detail of the samples made.
    pub lod: u8,
    /// The samples made.
    pub rect: Rect,
    /// The node's own seed, mixed with the terrain's.
    pub seed: u32,
    pub params: &'a Params,
    /// For each setting (by its index), the field driving it, sample by
    /// sample over `rect`, when one does (see [`Ctx::num_at`]).
    pub driven: &'a [Option<&'a [f32]>],
    /// Where a local kernel's inputs were read: level and samples.
    pub reads: &'a [Option<(u8, Rect)>],
    /// The world outside the graph: the map, files.
    pub env: &'a dyn Env,
    /// For each input (by its index), the geometry feeding it, made whole
    /// (see [`Ctx::geo`]); empty when none of its inputs is geometry.
    pub geos: &'a [Option<Arc<Geo>>],
}

impl Ctx<'_> {
    /// The geometry feeding input `k`, when it is connected.
    pub fn geo(&self, k: usize) -> Option<&Geo> {
        self.geos.get(k).and_then(Option::as_deref)
    }

    /// Number setting `name` at sample `k` of `rect`: the field driving it
    /// there, or its value.
    #[inline]
    pub fn num_at(&self, name: &str, k: usize) -> f32 {
        match self.params.spec().param(name).and_then(|(i, _)| self.driven.get(i).copied().flatten()) {
            Some(field) => field[k],
            None => self.params.f(name),
        }
    }

    /// The field driving setting `name`, when one does.
    pub fn driven(&self, name: &str) -> Option<&[f32]> {
        self.params.spec().param(name).and_then(|(i, _)| self.driven.get(i).copied().flatten())
    }

    /// Blocks between samples.
    pub fn spacing(&self) -> f64 {
        Space::spacing(self.lod)
    }

    /// Where sample `(i, j)` lies, in blocks from the terrain's corner.
    pub fn world(&self, i: i32, j: i32) -> (f64, f64) {
        let s = self.spacing();
        (i as f64 * s, j as f64 * s)
    }
}

/// Samples of a rectangle at one level of detail, row by row; several
/// values per sample (a colour's channels) are kept as planes one after
/// the other.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub lod: u8,
    pub rect: Rect,
    pub planes: usize,
    pub data: Vec<f32>,
}

impl Grid {
    /// The planes are as many as `data` holds.
    pub fn new(lod: u8, rect: Rect, data: Vec<f32>) -> Self {
        let planes = (data.len() / rect.area().max(1)).max(1);
        debug_assert_eq!(data.len(), rect.area() * planes);
        Self { lod, rect, planes, data }
    }

    pub fn filled(lod: u8, rect: Rect, v: f32) -> Self {
        Self::new(lod, rect, vec![v; rect.area()])
    }

    pub fn plane(&self, p: usize) -> &[f32] {
        let n = self.rect.area();
        &self.data[p * n..(p + 1) * n]
    }

    /// Sample `(x, z)` of plane `p` (absolute sample coordinates at the
    /// grid's level), the nearest edge sample outside.
    #[inline]
    pub fn at_plane(&self, p: usize, x: i32, z: i32) -> f32 {
        let i = (x - self.rect.x).clamp(0, self.rect.w as i32 - 1) as usize;
        let j = (z - self.rect.z).clamp(0, self.rect.h as i32 - 1) as usize;
        self.data[p * self.rect.area() + j * self.rect.w as usize + i]
    }

    /// Sample `(x, z)` of the first plane.
    #[inline]
    pub fn at(&self, x: i32, z: i32) -> f32 {
        self.at_plane(0, x, z)
    }

    /// Bilinear sample at fractional absolute sample coordinates.
    #[inline]
    pub fn bilinear(&self, u: f64, v: f64) -> f32 {
        let (fx, fz) = (u.floor(), v.floor());
        let (x, z) = (fx as i32, fz as i32);
        let (tx, tz) = ((u - fx) as f32, (v - fz) as f32);
        let a = self.at(x, z) + (self.at(x + 1, z) - self.at(x, z)) * tx;
        let b = self.at(x, z + 1) + (self.at(x + 1, z + 1) - self.at(x, z + 1)) * tx;
        a + (b - a) * tz
    }

    /// The nearest sample of plane `p` to fractional coordinates.
    #[inline]
    pub fn nearest(&self, p: usize, u: f64, v: f64) -> f32 {
        self.at_plane(p, u.round() as i32, v.round() as i32)
    }

    /// Catmull-Rom sample of the first plane.
    #[inline]
    pub fn bicubic(&self, u: f64, v: f64) -> f32 {
        self.bicubic_plane(0, u, v)
    }

    /// Catmull-Rom sample of plane `p` at fractional absolute sample
    /// coordinates, kept within the four nearest samples so that it never
    /// overshoots: sharp as a cubic, without rings around cliffs.
    #[inline]
    pub fn bicubic_plane(&self, p: usize, u: f64, v: f64) -> f32 {
        let (fx, fz) = (u.floor(), v.floor());
        let (x, z) = (fx as i32, fz as i32);
        let (tx, tz) = ((u - fx) as f32, (v - fz) as f32);
        let w = |t: f32| {
            let (t2, t3) = (t * t, t * t * t);
            [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
        };
        let (wx, wz) = (w(tx), w(tz));
        let at = |x: i32, z: i32| self.at_plane(p, x, z);
        let mut sum = 0.0;
        for (b, wzb) in wz.iter().enumerate() {
            let row = z - 1 + b as i32;
            let r = wx[0] * at(x - 1, row) + wx[1] * at(x, row) + wx[2] * at(x + 1, row) + wx[3] * at(x + 2, row);
            sum += wzb * r;
        }
        let corners = [at(x, z), at(x + 1, z), at(x, z + 1), at(x + 1, z + 1)];
        let lo = corners.iter().copied().fold(f32::MAX, f32::min);
        let hi = corners.iter().copied().fold(f32::MIN, f32::max);
        sum.clamp(lo, hi)
    }

    /// The samples of `rect`, edge samples repeated outside the grid.
    pub fn crop(&self, rect: Rect) -> Grid {
        let mut data = Vec::with_capacity(rect.area() * self.planes);
        for p in 0..self.planes {
            for z in rect.z..rect.z1() {
                for x in rect.x..rect.x1() {
                    data.push(self.at_plane(p, x, z));
                }
            }
        }
        Grid::new(self.lod, rect, data)
    }

    /// The lowest and highest values of the first plane.
    pub fn range(&self) -> (f32, f32) {
        self.plane(0).iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
    }
}

/// Figures over the whole of an input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f32,
    pub max: f32,
    pub mean: f32,
}

impl Stats {
    pub fn of(data: &[f32]) -> Self {
        if data.is_empty() {
            return Self { min: 0.0, max: 0.0, mean: 0.0 };
        }
        let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0f64);
        for &v in data {
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v as f64;
        }
        Self { min: lo, max: hi, mean: (sum / data.len() as f64) as f32 }
    }
}

/// Where a local kernel reads an input: at which level of detail, and how
/// many samples around the samples it makes (at that level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footprint {
    pub lod: u8,
    pub halo: u32,
}

/// Fills the outputs (each `ctx.rect.area()` samples) from the inputs'
/// samples at the same places.
/// Geometry, as nodes make and read it (whole, never by tiles).
#[derive(Debug, Clone, PartialEq)]
pub enum Geo {
    Points(massif_core::geometry::Points),
    Splines(massif_core::geometry::Splines),
    Footprints(massif_core::geometry::Footprints),
    Instances(massif_core::geometry::Instances),
}

impl Geo {
    /// What it is, as a socket type.
    pub fn kind(&self) -> crate::registry::PortKind {
        use crate::registry::PortKind;
        match self {
            Geo::Points(_) => PortKind::Points,
            Geo::Splines(_) => PortKind::Splines,
            Geo::Footprints(_) => PortKind::Footprints,
            Geo::Instances(_) => PortKind::Instances,
        }
    }

    /// Nothing of its kind, for a kernel that has nothing to make.
    pub fn empty(kind: crate::registry::PortKind) -> Option<Geo> {
        use crate::registry::PortKind;
        Some(match kind {
            PortKind::Points => Geo::Points(Default::default()),
            PortKind::Splines => Geo::Splines(Default::default()),
            PortKind::Footprints => Geo::Footprints(Default::default()),
            PortKind::Instances => Geo::Instances(Default::default()),
            _ => return None,
        })
    }

    pub fn points(&self) -> Option<&massif_core::geometry::Points> {
        if let Geo::Points(p) = self { Some(p) } else { None }
    }

    pub fn splines(&self) -> Option<&massif_core::geometry::Splines> {
        if let Geo::Splines(p) = self { Some(p) } else { None }
    }

    pub fn footprints(&self) -> Option<&massif_core::geometry::Footprints> {
        if let Geo::Footprints(p) = self { Some(p) } else { None }
    }

    pub fn instances(&self) -> Option<&massif_core::geometry::Instances> {
        if let Geo::Instances(p) = self { Some(p) } else { None }
    }
}

/// A geometry kernel: from its raster inputs over the whole terrain (at
/// `ctx.lod`, the simulations' level; `None` where unconnected or
/// geometry) and its geometry inputs (`ctx.geos`), one value per output, of
/// the output's type; `Err` says why it made nothing. It may stop early
/// when `cancel` says so (giving `Err`).
pub type GeometryFn = fn(&Ctx, &[Option<&Grid>], &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String>;

/// Blocks a side of a brick of a volume.
pub const BRICK: i32 = 16;

/// A brick of a volume field: 16³ values from `origin` (its west, bottom,
/// north block), `x` fastest, then `z`, then `y`.
#[derive(Debug, Clone, PartialEq)]
pub struct Brick {
    pub origin: [i32; 3],
    pub data: Vec<f32>,
}

impl Brick {
    pub fn filled(origin: [i32; 3], v: f32) -> Self {
        Self { origin, data: vec![v; (BRICK * BRICK * BRICK) as usize] }
    }

    /// The value at block `(x, y, z)` of the world, inside the brick.
    #[inline]
    pub fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        let (i, j, k) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        self.data[((j * BRICK + k) * BRICK + i) as usize]
    }
}

/// A volume kernel: the brick at `origin` of each output, from the same
/// brick of its volume inputs and, for its raster inputs, the columns under
/// it at one sample a block (with the node's halo around).
pub type VolumeFn = fn(&Ctx, [i32; 3], &[Option<&Grid>], &[Option<&Brick>], &mut [Brick]);

pub type PointFn = fn(&Ctx, &[Option<&[f32]>], &mut [Vec<f32>]);
/// Fills the outputs from the inputs read where their footprint says.
pub type LocalFn = fn(&Ctx, &[Option<&Grid>], &mut [Vec<f32>]);
/// Where a local kernel reads input `input` when it makes samples at `lod`.
pub type FootprintFn = fn(&Params, u8, usize) -> Footprint;
/// A point kernel given figures over each whole input.
pub type StatsFn = fn(&Ctx, &[Option<Stats>], &[Option<&[f32]>], &mut [Vec<f32>]);
/// Makes the outputs over the whole terrain (`ctx.rect`) from the inputs
/// over it; `None` when cancelled.
pub type GlobalFn = fn(&Ctx, &[Option<&Grid>], &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>>;
/// A global kernel on the graphics card: the inputs and outputs are grids
/// over the whole terrain in its memory. `None` when cancelled or when the
/// device refused it (the engine then runs the CPU kernel, unless
/// cancelled). Must give the same result each time on a given device: no
/// order-dependent float atomics.
pub type GlobalGpuFn = fn(&crate::gpu::Gpu, &Ctx, &[Option<&crate::gpu::GpuGrid>], &(dyn Fn() -> bool + Sync)) -> Option<Vec<crate::gpu::GpuGrid>>;

/// A point or local kernel's twin on the graphics card: a WGSL compute
/// shader with an entry point `main`.
///
/// Its bindings: `0` a uniform block (`uniforms` writes it), then one
/// read-only `array<f32>` per input (a stand-in when unconnected), then one
/// per drivable setting in order (a stand-in when not driven), then one
/// read-write `array<f32>` per output, planes one after the other. Each
/// tile is [`crate::TILE`]² samples, dispatched as 16 × 16 workgroups of
/// 16 × 16. A local kernel's inputs are the regions `Ctx::reads` gives,
/// which its uniforms must pass to the shader. `uniforms` is told which
/// inputs, then which drivable settings, are connected.
pub struct GpuKernel {
    pub name: &'static str,
    /// The kernel; [`crate::nodes::WGSL_COMMON`] is put before it.
    pub source: &'static str,
    pub uniforms: fn(&Ctx, &[bool]) -> Vec<u8>,
}

/// What the map holds, as map inputs read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapLayer {
    /// The ground's height, in blocks.
    Height,
    /// The water's surface where there is water, the ground's elsewhere.
    Water,
    /// The terrain of each column, by id.
    Terrain,
    /// The biome of each column, by id (0: automatic).
    Biome,
}

/// A decoded image, as file inputs read it: rows from the north,
/// `channels` values per pixel, each from 0 to 1 (or as stored, for
/// floating-point images).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub data: Vec<f32>,
}

/// The world outside the graph, as the host (the app, a test) gives it.
/// Every method has a default: a graph evaluated with [`NoEnv`] has no map
/// and no files.
pub trait Env: Send + Sync {
    /// Changes whenever the map changes.
    fn map_revision(&self) -> u64 {
        0
    }

    /// Fills `out` (`rect` at `lod` of `space`, row by row) with the map's
    /// `layer`; `false` when there is no map.
    fn read_map(&self, layer: MapLayer, space: &Space, lod: u8, rect: Rect, out: &mut [f32]) -> bool {
        let _ = (layer, space, lod, rect, out);
        false
    }

    /// Changes whenever the file at `path` changes.
    fn file_stamp(&self, path: &str) -> u64 {
        let _ = path;
        0
    }

    /// The image at `path`, decoded (and kept by the host between calls).
    fn image(&self, path: &str) -> Option<Arc<Image>> {
        let _ = path;
        None
    }

    /// The id of the terrain named `name` (Massif's `TerrainId`).
    fn terrain_id(&self, name: &str) -> Option<u16> {
        let _ = name;
        None
    }

    /// The id of the biome named `name` (`minecraft:plains`; Massif's
    /// `BiomeId`, 0 being automatic).
    fn biome_id(&self, name: &str) -> Option<u16> {
        let _ = name;
        None
    }

    /// The colour terrain `id` is shown in (linear light), for previews of
    /// the map's own terrains.
    fn terrain_color(&self, id: u16) -> Option<[f32; 3]> {
        let _ = id;
        None
    }
}

/// No map, no files.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEnv;

impl Env for NoEnv {}

/// Writes a WGSL uniform block field by field, with its alignments.
#[derive(Default)]
pub struct Uniforms {
    pub bytes: Vec<u8>,
}

impl Uniforms {
    pub fn align(&mut self, to: usize) -> &mut Self {
        while !self.bytes.len().is_multiple_of(to) {
            self.bytes.push(0);
        }
        self
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.align(4).bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.align(4).bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.align(4).bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// The block's end, rounded as WGSL rounds a struct (16 bytes).
    pub fn finish(&mut self) -> Vec<u8> {
        self.align(16);
        std::mem::take(&mut self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling() {
        let g = Grid::new(0, Rect::new(10, 20, 3, 2), vec![0.0, 1.0, 2.0, 10.0, 11.0, 12.0]);
        assert_eq!(g.at(11, 21), 11.0);
        assert_eq!(g.at(-5, 100), 10.0);
        assert!((g.bilinear(10.5, 20.5) - 5.5).abs() < 1e-6);
        // Linear data stays linear under the cubic, and it never overshoots.
        let line = Grid::new(0, Rect::new(0, 0, 5, 1), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert!((line.bicubic(1.25, 0.0) - 1.25).abs() < 1e-5);
        let step = Grid::new(0, Rect::new(0, 0, 4, 1), vec![0.0, 0.0, 10.0, 10.0]);
        for k in 0..=10 {
            let v = step.bicubic(1.0 + k as f64 / 10.0, 0.0);
            assert!((0.0..=10.0).contains(&v));
        }
    }
}
