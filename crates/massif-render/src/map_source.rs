//! The open map as the renderer's chunks: built by the exporter itself,
//! tile by tile, so the image shows what the game would get (levels,
//! caves, covers, objects, ores and water), then turned into the scene's
//! trees (see [`crate::scene`]).

use crate::chunk;
use crate::materials::Materials;
use crate::scene::{GpuChunk, Patch};
use massif_anvil::Target;
use massif_anvil::chunk::{ExportContext, Scratch, build_chunk, tile_placements};
use massif_core::compose::TileStack;
use massif_core::coords::TilePos;
use massif_core::objects::Placement;
use massif_core::world::World;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Chunks on a side of a tile.
pub const TILE_CHUNKS: i32 = 8;
/// Tiles built side by side before the next ones are begun: the nearest
/// first, a batch at a time.
const BATCH: usize = 24;

/// Where a scene lies: its chunks across and down from chunk `(x0, z0)`,
/// and its sections from `min_y` up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub x0: i32,
    pub z0: i32,
    pub width: u32,
    pub depth: u32,
    pub min_y: i32,
    pub sections: u32,
}

impl Extent {
    /// The chunks of a dimension of the map, as exported for `target`.
    pub fn of_map(world: &World, dim: usize, target: &Target) -> Option<Extent> {
        let d = world.dims.get(dim)?;
        let ctx = ExportContext::new(world, d, target).ok()?;
        let mut tiles = d.ground().positions();
        let first = tiles.next()?;
        let (mut lo, mut hi) = ((first.x, first.z), (first.x, first.z));
        for p in tiles {
            lo = (lo.0.min(p.x), lo.1.min(p.z));
            hi = (hi.0.max(p.x), hi.1.max(p.z));
        }
        Some(Extent {
            x0: lo.0 * TILE_CHUNKS,
            z0: lo.1 * TILE_CHUNKS,
            width: ((hi.0 - lo.0 + 1) * TILE_CHUNKS) as u32,
            depth: ((hi.1 - lo.1 + 1) * TILE_CHUNKS) as u32,
            min_y: ctx.min_y,
            sections: ((ctx.max_y - ctx.min_y) / 16) as u32,
        })
    }

    /// Height in blocks.
    pub fn height(&self) -> i32 {
        self.sections as i32 * 16
    }

    /// The middle of the scene, in the world's blocks.
    pub fn center(&self) -> [f64; 3] {
        [(self.x0 as f64 + self.width as f64 / 2.0) * 16.0, self.min_y as f64 + self.height() as f64 / 2.0, (self.z0 as f64 + self.depth as f64 / 2.0) * 16.0]
    }
}

/// The version the map's chunks are built for: the one of the game's
/// blocks read, when it stores chunks as 1.18 does, else 1.18.2.
pub fn target_for(cat_version: Option<&str>) -> Target {
    cat_version.and_then(Target::release).filter(|t| t.era() == massif_anvil::Era::Modern).unwrap_or_else(|| Target::release(massif_anvil::version::RELEASES[0].0).unwrap_or_default())
}

/// A tile's chunks, built.
pub struct Built {
    pub tile: TilePos,
    pub chunks: Vec<GpuChunk>,
}

/// Builds tiles of a map in the background, the first asked first.
pub struct MapBuilder {
    rx: Receiver<Built>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    total: usize,
    thread: Option<JoinHandle<Result<(), String>>>,
    error: Option<String>,
}

impl MapBuilder {
    /// Build `tiles` of dimension `dim` of `world` (a snapshot the builder
    /// keeps), in that order.
    pub fn start(world: World, dim: usize, target: Target, materials: Arc<Materials>, tiles: Vec<TilePos>) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let total = tiles.len();
        let (stop, count) = (cancel.clone(), done.clone());
        let thread = std::thread::Builder::new()
            .name("massif render scene".into())
            .spawn(move || -> Result<(), String> {
                let d = world.dims.get(dim).ok_or("no such dimension")?;
                let ctx = ExportContext::new(&world, d, &target)?;
                let sections = ((ctx.max_y - ctx.min_y) / 16) as usize;
                let planner = (!world.meta.lava).then(|| massif_core::water::Planner::new(d));
                let placed: Mutex<HashMap<TilePos, Arc<Vec<Placement>>>> = Mutex::new(HashMap::new());
                let ring = (ctx.reach + 127) / 128;
                for batch in tiles.chunks(BATCH) {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    batch
                        .par_iter()
                        .map_init(
                            || (Scratch::new(&ctx), crate::materials::Lookup::default()),
                            |(scratch, lookup), &pos| {
                                if stop.load(Ordering::Relaxed) {
                                    return None;
                                }
                                let stack = TileStack::new(d, pos, true)?;
                                let mut objects = Vec::new();
                                if ctx.reach > 0 {
                                    let area = pos.rect().expand(ctx.reach);
                                    for tz in pos.z - ring..=pos.z + ring {
                                        for tx in pos.x - ring..=pos.x + ring {
                                            let here = TilePos::new(tx, tz);
                                            let cached = placed.lock().unwrap().get(&here).cloned();
                                            let list = cached.unwrap_or_else(|| {
                                                let list = Arc::new(tile_placements(&ctx, &world, d, here));
                                                placed.lock().unwrap().insert(here, list.clone());
                                                list
                                            });
                                            objects.extend(list.iter().filter(|p| area.x0 <= p.x && p.x < area.x1 && area.z0 <= p.z && p.z < area.z1).copied());
                                        }
                                    }
                                    objects.sort_unstable_by_key(|p| (p.layer, p.x, p.z));
                                }
                                scratch.lay_ores(&ctx, &stack, pos);
                                // Each tile's water settled on its own (the export also
                                // settles it along the seams between tiles: the
                                // first image comes sooner, and seams barely differ).
                                let steady = if planner.as_ref().is_some_and(|p| p.needs(pos)) { massif_core::water::steady_tile(d, pos, planner.as_ref()) } else { Vec::new() };
                                let slabs = massif_core::shapes::plan_tile(d, &world.meta.smooth, pos);
                                let mut patch = Patch::new(pos.x * TILE_CHUNKS, pos.z * TILE_CHUNKS, TILE_CHUNKS as usize, TILE_CHUNKS as usize, sections);
                                patch.edges = edges(d, pos, ctx.min_y, sections);
                                for cz in 0..TILE_CHUNKS as usize {
                                    for cx in 0..TILE_CHUNKS as usize {
                                        let nbt = build_chunk(&ctx, &stack, pos, cx, cz, &objects, &steady, &slabs, scratch);
                                        if let Some(c) = chunk::decode(&nbt, &materials, lookup, ctx.min_y, sections) {
                                            patch.put(c);
                                        }
                                    }
                                }
                                Some(Built { tile: pos, chunks: patch.build(&materials.classes(), &materials.lights()) })
                            },
                        )
                        .for_each_with(tx.clone(), |tx, built| {
                            if let Some(built) = built {
                                let _ = tx.send(built);
                            }
                            count.fetch_add(1, Ordering::Relaxed);
                        });
                }
                Ok(())
            })
            .expect("a thread for the scene");
        Self { rx, cancel, done, total, thread: Some(thread), error: None }
    }

    /// A tile built, if one is ready.
    pub fn take(&mut self) -> Option<Built> {
        match self.rx.try_recv() {
            Ok(b) => Some(b),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                if let Some(t) = self.thread.take()
                    && let Ok(Err(e)) = t.join()
                {
                    self.error = Some(e);
                }
                None
            }
        }
    }

    /// Tiles done (built or found empty), and all of them.
    pub fn progress(&self) -> (usize, usize) {
        (self.done.load(Ordering::Relaxed), self.total)
    }

    /// Whether every tile is built and handed over.
    pub fn finished(&self) -> bool {
        self.thread.is_none() || (self.done.load(Ordering::Relaxed) == self.total && self.thread.as_ref().is_some_and(JoinHandle::is_finished))
    }

    /// Why the builder stopped, if it failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Wait for every tile, for tests and stills: all of them at once.
    pub fn wait_all(&mut self) -> Vec<Built> {
        let mut out = Vec::new();
        while let Ok(b) = self.rx.recv() {
            out.push(b);
        }
        if let Some(t) = self.thread.take()
            && let Ok(Err(e)) = t.join()
        {
            self.error = Some(e);
        }
        out
    }
}

impl Drop for MapBuilder {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// What the tiles around `pos` hold just beside it, from their solid
/// spans (the levels, what they carve and the sculpted volumes): the
/// faces of the tile's sides against their ground are hidden, and not
/// kept as walls between the tiles. The objects and the caves the export
/// adds are not in the spans: the faces against them are only kept where
/// the spans have air.
fn edges(d: &massif_core::world::Dimension, pos: TilePos, min_y: i32, sections: usize) -> [Option<Vec<u8>>; 4] {
    let h = sections * 16;
    let mut spans = Vec::new();
    let mut out = [None, None, None, None];
    for (side, (dx, dz)) in [(-1, 0), (1, 0), (0, -1), (0, 1)].into_iter().enumerate() {
        let Some(stack) = TileStack::new(d, TilePos::new(pos.x + dx, pos.z + dz), true) else { continue };
        let mut edge = vec![0u8; 128 * h];
        for along in 0..128usize {
            let (x, z) = match side {
                0 => (127, along),
                1 => (0, along),
                2 => (along, 127),
                _ => (along, 0),
            };
            stack.spans(massif_core::coords::cell_index(x, z), &mut spans);
            for s in &spans {
                let (y0, y1) = ((s.y0 - min_y).clamp(0, h as i32) as usize, (s.y1 - min_y).clamp(0, h as i32) as usize);
                edge[along * h + y0..along * h + y1].fill(1);
            }
        }
        out[side] = Some(edge);
    }
    out
}

/// The tiles of a dimension, nearest `(x, z)` (blocks) first.
pub fn tiles_by_distance(world: &World, dim: usize, x: f64, z: f64) -> Vec<TilePos> {
    let Some(d) = world.dims.get(dim) else { return Vec::new() };
    let mut tiles: Vec<TilePos> = d.ground().positions().collect();
    let size = (TILE_CHUNKS * 16) as f64;
    let key = |p: &TilePos| {
        let (cx, cz) = ((p.x as f64 + 0.5) * size - x, (p.z as f64 + 0.5) * size - z);
        cx * cx + cz * cz
    };
    tiles.sort_by(|a, b| key(a).total_cmp(&key(b)));
    tiles
}
