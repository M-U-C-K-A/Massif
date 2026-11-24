//! Terrain graphs and the worlds of Massif: the graph a world keeps (its
//! graph of 1.2 migrated when it has none), the square of the world a graph
//! covers over a map, the map as the graph's Map nodes read it, and what a
//! graph makes as the map's writer takes it.

use crate::compile::{Compiled, Diagnostic};
use crate::engine::{Cancel, Engine, EvalError};
use crate::files::DiskEnv;
use crate::kernel::{Env, Grid, Image, MapLayer};
use crate::migrate;
use crate::model::Graph;
use crate::space::{Rect, Space};
use massif_core::graph::{self as v1, BiomeOut, BlockPalette, ColorField, Extent, Field, LayerOut, NO_WATER, Outputs};
use massif_core::world::Dimension;
use massif_core::{BlockRect, Registry, Span, TILE_SIZE, TerrainId, TileStack, TilePos, World};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The blocks a map covers: its tiles (a square around the origin while it
/// is empty).
pub fn map_rect(dim: &Dimension) -> BlockRect {
    match dim.bounds() {
        Some((lo, hi)) => BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE),
        None => BlockRect::new(-512, -512, 512, 512),
    }
}

/// The square of the world `graph` covers over `world`'s dimension `dim`:
/// the map, or a square of the graph's size around the map's middle, on the
/// bounds of tiles.
pub fn space_of(world: &World, dim: usize, graph: &Graph) -> Space {
    let rect = map_rect(&world.dims[dim]);
    let (x0, z0, size) = match graph.terrain.size {
        None => (rect.x0, rect.z0, rect.width().max(rect.depth()).max(1) as u32),
        Some(side) => {
            let half = side as i32 / 2;
            let corner = |mid: i32| (mid - half).div_euclid(TILE_SIZE) * TILE_SIZE;
            (corner((rect.x0 + rect.x1) / 2), corner((rect.z0 + rect.z1) / 2), side)
        }
    };
    Space { x0, z0, size, seed: world.meta.seed as u64, sea_level: world.meta.sea_level as f32 }
}

/// The terrain graph `world` keeps, or its graph of 1.2 migrated, with
/// what the migration said; `None` when it has neither.
pub fn graph_of(world: &World) -> Option<(Graph, Vec<Diagnostic>)> {
    if let Some(v) = &world.meta.terrain_graph {
        return Some(match serde_json::from_value::<Graph>(v.clone()) {
            Ok(mut g) => {
                g.upgrade();
                (g, Vec::new())
            }
            Err(e) => (Graph::default(), vec![Diagnostic::error("", format!("the world's terrain graph cannot be read: {e}"))]),
        });
    }
    let old = world.meta.graph.as_ref()?;
    let rect = map_rect(&world.dims[0]);
    let side = old.size.unwrap_or(rect.width().max(rect.depth()).max(1) as u32);
    let names = |id: u16| world.registry.terrain(TerrainId(id)).name.clone();
    Some(migrate::from_v1(old, &migrate::World { side, sea_level: world.meta.sea_level as f32, terrain_name: &names }))
}

/// A world's map as the graph reads it: its ground, water and terrains,
/// and the files beside it.
pub struct MapEnv {
    dim: Arc<Dimension>,
    revision: u64,
    /// The levels the graph writes itself, which its Map nodes do not read.
    own: Vec<String>,
    terrains: HashMap<String, u16>,
    colors: Vec<[f32; 3]>,
    files: DiskEnv,
}

impl MapEnv {
    /// The map of `dim` as it is at `revision` (a number that changes with
    /// it), files read from beside the world, in `folder`. Copies the
    /// dimension: an app keeping one open passes a snapshot to [`MapEnv::of`].
    pub fn new(world: &World, dim: usize, revision: u64, folder: impl Into<PathBuf>) -> Self {
        Self::of(world.dims[dim].clone(), &world.registry, revision, folder)
    }

    /// The map of `dim` (a snapshot: `Dimension::snapshot`), its terrains
    /// named by `registry`.
    pub fn of(dim: Dimension, registry: &Registry, revision: u64, folder: impl Into<PathBuf>) -> Self {
        let count = registry.terrains.len();
        let terrains = (0..count).map(|k| (registry.terrain(TerrainId(k as u16)).name.to_ascii_lowercase(), k as u16)).collect();
        let colors = (0..count).map(|k| registry.terrain(TerrainId(k as u16)).color.map(v1::decode_srgb)).collect();
        let own = (1..=4).map(|k| LayerOut::level_name("", k)).collect();
        Self { dim: Arc::new(dim), revision, own, terrains, colors, files: DiskEnv::new(folder) }
    }
}

impl Env for MapEnv {
    fn map_revision(&self) -> u64 {
        self.revision
    }

    fn read_map(&self, layer: MapLayer, space: &Space, lod: u8, rect: Rect, out: &mut [f32]) -> bool {
        let step = Space::spacing(lod) as i64;
        let w = rect.w as usize;
        let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
        let (mut spans, mut fluids): (Vec<Span>, Vec<(i32, i32)>) = (Vec::new(), Vec::new());
        let ground = space.sea_level;
        for (k, o) in out.iter_mut().enumerate().take(rect.area()) {
            let x = (space.x0 as i64 + (rect.x as i64 + (k % w) as i64) * step) as i32;
            let z = (space.z0 as i64 + (rect.z as i64 + (k / w) as i64) * step) as i32;
            let pos = TilePos::new(x.div_euclid(TILE_SIZE), z.div_euclid(TILE_SIZE));
            let stack = stacks.entry(pos).or_insert_with(|| TileStack::new_with(&self.dim, pos, |_, level| v1::reads_level(level, &self.own)));
            let cell = massif_core::coords::cell_index(x.rem_euclid(TILE_SIZE) as usize, z.rem_euclid(TILE_SIZE) as usize);
            let surface = stack.as_ref().and_then(|s| v1::surface_of(s, cell, &mut spans, &mut fluids));
            *o = match (layer, surface) {
                (MapLayer::Height, Some((h, _, _))) => h,
                (MapLayer::Water, Some((h, level, _))) => if level > h { level } else { h },
                (MapLayer::Terrain, Some((_, _, t))) => t as f32,
                (MapLayer::Biome, _) => 0.0,
                (MapLayer::Height | MapLayer::Water, None) => ground,
                (MapLayer::Terrain, None) => 0.0,
            };
        }
        true
    }

    fn file_stamp(&self, path: &str) -> u64 {
        self.files.file_stamp(path)
    }

    fn image(&self, path: &str) -> Option<Arc<Image>> {
        self.files.image(path)
    }

    fn terrain_id(&self, name: &str) -> Option<u16> {
        self.terrains.get(&name.trim().to_ascii_lowercase()).copied()
    }

    fn terrain_color(&self, id: u16) -> Option<[f32; 3]> {
        self.colors.get(id as usize).copied()
    }
}

fn field(g: &Grid) -> Field {
    Field { n: g.rect.w as usize, data: g.data[..g.rect.area()].to_vec() }
}

fn colors(g: &Grid) -> ColorField {
    let area = g.rect.area();
    ColorField::build(g.rect.w as usize, |c| [g.data[c], g.data[area + c], g.data[2 * area + c]])
}

fn palette_of(c: &Compiled) -> BlockPalette {
    let choice = c.map_output().and_then(|o| c.nodes.get(o.node)).map(|n| n.params.choice("albedo_palette").to_owned());
    match choice.as_deref() {
        Some("rock") => BlockPalette::Rock,
        Some("desert") => BlockPalette::Desert,
        Some("snowy") => BlockPalette::Snowy,
        Some("every") => BlockPalette::Everything,
        _ => BlockPalette::Natural,
    }
}

/// What the graph makes over its whole terrain at `lod`, as the map's
/// writer of 1.2 takes it: the extent its samples cover, and the height,
/// water, terrains (one mask each, later ones over earlier ones), colours
/// with their blocks, layers of rock and climate of the active Map Output.
pub fn outputs(engine: &Engine, c: &Compiled, lod: u8, cancel: Cancel) -> Result<(Extent, Outputs), EvalError> {
    outputs_in(engine, c, lod, c.space.whole(lod), cancel)
}

/// [`outputs`] over a square window of the terrain only (`window`: samples
/// at `lod`, its side `w`): what a map too large to hold whole is written
/// with, window after window, each holding only its own samples (and the
/// engine's cache, under its budget).
pub fn outputs_in(engine: &Engine, c: &Compiled, lod: u8, window: Rect, cancel: Cancel) -> Result<(Extent, Outputs), EvalError> {
    let whole = window;
    debug_assert_eq!(whole.w, whole.h, "a square window");
    let n = whole.w as usize;
    let step = Space::spacing(lod);
    // 1.2's samples lie at the middle of their cells; these at the corner.
    let extent = Extent { x0: c.space.x0 as f64 + whole.x as f64 * step - step / 2.0, z0: c.space.z0 as f64 + whole.z as f64 * step - step / 2.0, size: step * n as f64, n };
    // Every output the Map Output has, planned together: what they share
    // upstream is made once.
    let names: Vec<String> = ["height", "water", "terrain", "albedo", "temperature", "humidity"]
        .into_iter()
        .map(str::to_owned)
        .chain((1..=4).flat_map(|k| [format!("level.{k}.top"), format!("level.{k}.bottom"), format!("level.{k}.color")]))
        .filter(|n| c.output(n).is_some())
        .collect();
    let srcs: Vec<(usize, u8)> = names.iter().filter_map(|n| c.output(n)).collect();
    let made: std::collections::HashMap<&str, Grid> = names.iter().map(String::as_str).zip(engine.regions(c, &srcs, lod, whole, cancel)?).collect();
    let grid = |name: &str| -> Result<Option<Grid>, EvalError> { Ok(made.get(name).cloned()) };
    let mut out = Outputs::default();
    let heights = grid("height")?;
    if let Some(water) = grid("water")? {
        let data = water.data.iter().enumerate().map(|(k, &w)| if heights.as_ref().is_none_or(|h| w > h.data[k] + 0.01) { w } else { NO_WATER }).collect();
        out.water = Some(Arc::new(Field { n, data }));
    }
    out.heights = heights.as_ref().map(|h| Arc::new(field(h)));
    if let Some(t) = grid("terrain")? {
        let mut ids: Vec<u16> = t.data.iter().map(|v| *v as u16).collect();
        ids.sort_unstable();
        ids.dedup();
        out.terrains = ids.into_iter().map(|id| (TerrainId(id), Arc::new(Field { n, data: t.data.iter().map(|v| (*v as u16 == id) as u8 as f32).collect() }))).collect();
    }
    let palette = palette_of(c);
    if let Some(a) = grid("albedo")? {
        let cf = Arc::new(colors(&a));
        out.shown = Some(cf.clone());
        out.colors = Some((cf, palette));
    }
    for k in 1..=4u32 {
        let (top, bottom) = (grid(&format!("level.{k}.top"))?, grid(&format!("level.{k}.bottom"))?);
        if let (Some(top), Some(bottom)) = (top, bottom) {
            let color = grid(&format!("level.{k}.color"))?.map(|g| (Arc::new(colors(&g)), palette));
            out.layers.push(LayerOut { slot: k, name: LayerOut::level_name("Terrain graph", k), top: Arc::new(field(&top)), bottom: Arc::new(field(&bottom)), colors: color });
        }
    }
    if let (Some(h), Some(t), Some(m)) = (&out.heights, grid("temperature")?, grid("humidity")?) {
        let paint = c.map_output().and_then(|o| c.nodes.get(o.node)).is_some_and(|n| n.params.flag("paint_biomes"));
        out.biomes = Some(BiomeOut { height: h.clone(), temperature: Arc::new(field(&t)), humidity: Arc::new(field(&m)), sea: c.space.sea_level, paint });
    }
    Ok((extent, out))
}
