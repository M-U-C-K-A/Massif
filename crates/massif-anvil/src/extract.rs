//! The blocks of a part of the map, as the export would write them: for a
//! schematic of a mountain, a village or a forest.
//!
//! The columns are built by [`crate::chunk::chunk_blocks`], the same as the
//! world's export and the Diorama, with the water settled as the export
//! settles it: what a schematic holds is what the world exported would
//! hold there.

use crate::chunk::{CaveWork, ExportContext, Scratch, chunk_blocks, tile_placements};
use crate::export::{ExportError, Progress};
use crate::version::{Era, Target};
use massif_core::TilePos;
use massif_core::compose::TileStack;
use massif_core::coords::BlockRect;
use massif_core::objects::Placement;
use massif_core::selection::{Selection, lets};
use massif_core::world::World;
use massif_objects::write::{BlockEntity, Volume};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

/// The most blocks a box may hold: 64 million (128 MB of blocks).
pub const MAX_BLOCKS: usize = 1 << 26;
/// Past this many blocks a box is large for the tools that paste it, and
/// for Massif's own import of objects (see [`massif_objects::MAX_VOLUME`]).
pub const LARGE_BLOCKS: usize = massif_objects::MAX_VOLUME;

/// How low the box goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bottom {
    /// This many blocks under the lowest ground of the selection.
    BelowGround(i32),
    /// Down to the world's floor.
    Floor,
}

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub dimension: usize,
    pub target: Target,
    pub mod_blocks: std::sync::Arc<HashMap<String, Vec<String>>>,
    pub families: std::sync::Arc<massif_core::block_families::BlockFamilies>,
    pub bottom: Bottom,
    /// The trees and other objects of the object layers.
    pub objects: bool,
    /// The structures: villages, temples, their chests.
    pub structures: bool,
    /// Keep the biome of each block.
    pub biomes: bool,
}

/// The objects of one layer standing in the box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectCount {
    pub layer: String,
    pub object: String,
    pub count: usize,
}

/// A part of the map, built.
#[derive(Debug, Clone)]
pub struct Extracted {
    /// The blocks, the box's lowest corner at its zero; its origin at the
    /// middle of its foot.
    pub volume: Volume,
    /// Where the box's lowest corner is in the world.
    pub corner: [i32; 3],
    /// The objects standing in it, by layer and object, the most first.
    pub objects: Vec<ObjectCount>,
    /// The lowest ground of the selection.
    pub lowest_ground: i32,
}

/// The selection's columns: whether each in its bounds is taken.
fn taken(selection: &Selection, rect: BlockRect, x: i32, z: i32) -> bool {
    rect.x0 <= x && x < rect.x1 && rect.z0 <= z && z < rect.z1 && lets(selection.get(x, z), x, z)
}

/// The lowest and highest ground of the selected columns of `dim`, if any
/// is ground.
pub fn ground_range(world: &World, dimension: usize, selection: &Selection) -> Option<(i32, i32)> {
    let dim = world.dims.get(dimension)?;
    let rect = selection.column_bounds()?;
    (rect.z0..rect.z1)
        .into_par_iter()
        .filter_map(|z| {
            (rect.x0..rect.x1)
                .filter(|&x| taken(selection, rect, x, z))
                .filter_map(|x| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top))
                .map(|h| h.floor() as i32)
                .fold(None, |r: Option<(i32, i32)>, h| Some(r.map_or((h, h), |(lo, hi)| (lo.min(h), hi.max(h)))))
        })
        .reduce_with(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

fn too_large(total: usize, w: usize, h: usize, l: usize) -> ExportError {
    ExportError::Unsupported(format!("the box would hold {} million blocks ({w} × {h} × {l}), more than the {} million a schematic may: select less, or raise its bottom", total >> 20, MAX_BLOCKS >> 20))
}

/// The blocks of one tile's columns in the box: each column from the box's
/// bottom up to its highest block, one after the other.
struct TileColumns {
    tile: TilePos,
    /// Of each column of the tile within the box (row by row), where its
    /// blocks start in `blocks` and how many there are.
    spans: Vec<(u32, u32)>,
    blocks: Vec<u16>,
    biomes: Vec<u16>,
    objects: Vec<Placement>,
}

/// Build the blocks of the columns `selection` holds, from the bottom
/// `options` says to the highest block, as the export of `world` would.
pub fn extract(world: &World, selection: &Selection, options: &ExtractOptions, progress: &Progress) -> Result<Extracted, ExportError> {
    let dim = world.dims.get(options.dimension).ok_or_else(|| ExportError::Unsupported("no such dimension".into()))?;
    if options.target.era() != Era::Modern {
        return Err(ExportError::Unsupported("schematics are written for Minecraft 1.13 and later: pick a later version".into()));
    }
    let Some(rect) = selection.column_bounds() else { return Err(ExportError::Empty) };
    let mut ctx = ExportContext::with_mods(world, dim, &options.target, &options.mod_blocks, &options.families).map_err(ExportError::Unsupported)?;
    if !options.structures {
        ctx.drop_structures();
    }
    let (lowest, highest) = ground_range(world, options.dimension, selection).ok_or(ExportError::Empty)?;
    let bottom = match options.bottom {
        Bottom::BelowGround(depth) => (lowest - depth.max(0)).max(ctx.min_y),
        Bottom::Floor => ctx.min_y,
    }
    .min(ctx.max_y - 1);
    // Refused before it is built when even its ground would be too much.
    let (w, l) = (rect.width() as usize, (rect.z1 - rect.z0) as usize);
    let least = w.saturating_mul(l).saturating_mul((highest + 1 - bottom).max(1) as usize);
    if least > MAX_BLOCKS {
        return Err(too_large(least, w, (highest + 1 - bottom).max(1) as usize, l));
    }
    let tiles: Vec<TilePos> = rect.tiles().filter(|&t| dim.ground().contains(t) && selection.tile(t).is_some()).collect();
    if tiles.is_empty() {
        return Err(ExportError::Empty);
    }
    // The water settled as the export of these tiles settles it.
    let steady = !world.meta.lava && std::env::var_os("MASSIF_NO_STEADY").is_none();
    let around: Vec<TilePos> = if steady {
        let reach = massif_core::water::SEAM_ROUNDS;
        let ground = dim.ground();
        let near: HashSet<TilePos> = tiles.iter().flat_map(|p| (-reach..=reach).flat_map(move |dz| (-reach..=reach).map(move |dx| p.offset(dx, dz)))).filter(|&p| ground.contains(p)).collect();
        near.into_iter().collect()
    } else {
        Vec::new()
    };
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(tiles.len() + around.len(), Ordering::Relaxed);
    let settled = if steady {
        let planner = massif_core::water::Planner::within(dim, &around);
        let step = || {
            progress.done.fetch_add(1, Ordering::Relaxed);
            !progress.cancel.load(Ordering::Relaxed)
        };
        let settled = massif_core::water::steady_of(dim, &planner, &around, &step).ok_or(ExportError::Cancelled)?;
        progress.done.store(around.len(), Ordering::Relaxed);
        settled
    } else {
        HashMap::new()
    };
    let with_objects = options.objects && ctx.reach > 0;
    let ring = (ctx.reach + 127) / 128;
    let placed: HashMap<TilePos, Vec<Placement>> = if with_objects {
        let near: HashSet<TilePos> = tiles.iter().flat_map(|p| (p.z - ring..=p.z + ring).flat_map(move |tz| (p.x - ring..=p.x + ring).map(move |tx| TilePos::new(tx, tz)))).collect();
        near.into_par_iter().map(|t| (t, tile_placements(&ctx, world, dim, t))).collect()
    } else {
        HashMap::new()
    };
    let height = (ctx.max_y - bottom) as usize;
    let built: Vec<TileColumns> = tiles
        .par_iter()
        .map_init(
            || Scratch::new(&ctx),
            |scratch, &pos| -> Result<TileColumns, ExportError> {
                if progress.cancel.load(Ordering::Relaxed) {
                    return Err(ExportError::Cancelled);
                }
                let clip = pos.rect().intersect(&rect);
                let (cw, cl) = (clip.width() as usize, (clip.z1 - clip.z0) as usize);
                let mut out = TileColumns { tile: pos, spans: vec![(0, 0); cw * cl], blocks: Vec::new(), biomes: Vec::new(), objects: Vec::new() };
                let Some(stack) = TileStack::new(dim, pos, true) else {
                    progress.done.fetch_add(1, Ordering::Relaxed);
                    return Ok(out);
                };
                let mut objects = Vec::new();
                if with_objects {
                    let area = pos.rect().expand(ctx.reach);
                    for tz in pos.z - ring..=pos.z + ring {
                        for tx in pos.x - ring..=pos.x + ring {
                            if let Some(here) = placed.get(&TilePos::new(tx, tz)) {
                                objects.extend(here.iter().filter(|p| area.x0 <= p.x && p.x < area.x1 && area.z0 <= p.z && p.z < area.z1).copied());
                            }
                        }
                    }
                    objects.sort_unstable_by_key(|p| (p.layer, p.x, p.z));
                    // Those of the tile whose foot is taken, counted once.
                    out.objects = placed.get(&pos).map_or_else(Vec::new, |here| here.iter().filter(|p| p.y >= bottom && taken(selection, rect, p.x, p.z)).copied().collect());
                }
                scratch.lay_ores(&ctx, &stack, pos);
                let steady: &[massif_core::water::Steady] = settled.get(&pos).map_or(&[], |s| s.as_slice());
                let slabs = massif_core::shapes::plan_tile(dim, &world.meta.smooth, pos);
                let caves = CaveWork::of(world, dim, pos, true);
                let (ox, oz) = pos.origin();
                let mut column = vec![0u16; height];
                for cz in 0..8 {
                    for cx in 0..8 {
                        let (x0, z0) = (ox + cx * 16, oz + cz * 16);
                        let chunk = BlockRect::new(x0, z0, x0 + 16, z0 + 16).intersect(&clip);
                        if chunk.is_empty() || !(chunk.z0..chunk.z1).any(|z| (chunk.x0..chunk.x1).any(|x| taken(selection, rect, x, z))) {
                            continue;
                        }
                        chunk_blocks(&ctx, &stack, pos, cx as usize, cz as usize, &objects, steady, &slabs, &caves, scratch);
                        let (blocks, biomes) = (scratch.chunk(), scratch.chunk_biomes());
                        for z in chunk.z0..chunk.z1 {
                            for x in chunk.x0..chunk.x1 {
                                if !taken(selection, rect, x, z) {
                                    continue;
                                }
                                let (lx, lz) = ((x - x0) as usize, (z - z0) as usize);
                                let mut top = 0;
                                for (k, c) in column.iter_mut().enumerate() {
                                    let i = (bottom - ctx.min_y) as usize + k;
                                    let at = (i >> 4) * 4096 + ((i & 15) * 16 + lz) * 16 + lx;
                                    *c = blocks.get(at).copied().unwrap_or(0);
                                    if *c != 0 {
                                        top = k + 1;
                                    }
                                }
                                let local = (z - clip.z0) as usize * cw + (x - clip.x0) as usize;
                                out.spans[local] = (out.blocks.len() as u32, top as u32);
                                out.blocks.extend_from_slice(&column[..top]);
                                if options.biomes {
                                    let cell = &biomes[(lz >> 2) * 4 + (lx >> 2)];
                                    out.biomes.extend((0..top).map(|k| cell.at(bottom + k as i32)));
                                }
                            }
                        }
                    }
                }
                progress.done.fetch_add(1, Ordering::Relaxed);
                Ok(out)
            },
        )
        .collect::<Result<_, _>>()?;

    // The box: up to the highest block of any column.
    let top = built.iter().flat_map(|t| t.spans.iter().map(|s| s.1)).max().unwrap_or(0) as usize;
    if top == 0 {
        return Err(ExportError::Empty);
    }
    let total = w.saturating_mul(l).saturating_mul(top);
    if total > MAX_BLOCKS {
        return Err(too_large(total, w, top, l));
    }
    // The states used, air first.
    let mut index: Vec<u16> = vec![u16::MAX; ctx.state_count()];
    let mut palette = vec!["minecraft:air".to_owned()];
    index[0] = 0;
    let mut blocks = vec![0u16; total];
    let mut biome_ids = if options.biomes { vec![0u16; total] } else { Vec::new() };
    let mut biome_index: HashMap<u16, u16> = HashMap::new();
    let mut biome_names: Vec<String> = Vec::new();
    for t in &built {
        let clip = t.tile.rect().intersect(&rect);
        let cw = clip.width() as usize;
        for (local, &(start, len)) in t.spans.iter().enumerate() {
            let (x, z) = (clip.x0 as usize + local % cw - rect.x0 as usize, clip.z0 as usize + local / cw - rect.z0 as usize);
            for k in 0..len as usize {
                let state = t.blocks[start as usize + k];
                let slot = &mut index[state as usize];
                if *slot == u16::MAX {
                    *slot = palette.len() as u16;
                    palette.push(match ctx.state(state) {
                        Some((name, props)) if !props.is_empty() => format!("{name}[{}]", props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")),
                        Some((name, _)) => name.to_owned(),
                        None => "minecraft:air".to_owned(),
                    });
                }
                let at = x + z * w + k * w * l;
                blocks[at] = *slot;
                if options.biomes {
                    let b = t.biomes[start as usize + k];
                    let n = *biome_index.entry(b).or_insert_with(|| {
                        biome_names.push(ctx.biome_name(b).to_owned());
                        (biome_names.len() - 1) as u16
                    });
                    biome_ids[at] = n;
                }
            }
        }
    }
    if options.biomes && biome_names.is_empty() {
        biome_names.push("minecraft:plains".to_owned());
    }
    let corner = [rect.x0, bottom, rect.z0];
    let top_y = bottom + top as i32;
    let block_entities = ctx
        .block_entities_in(rect)
        .into_iter()
        .filter(|(_, p)| p[1] >= bottom && p[1] < top_y && taken(selection, rect, p[0], p[2]))
        .map(|(nbt, p)| BlockEntity { pos: [p[0] - corner[0], p[1] - corner[1], p[2] - corner[2]], nbt: nbt.to_vec() })
        .collect();
    // The objects, by layer and object.
    let mut counts: HashMap<(u16, u32), usize> = HashMap::new();
    for p in built.iter().flat_map(|t| &t.objects).filter(|p| p.y < top_y) {
        *counts.entry((p.layer, p.object)).or_default() += 1;
    }
    let reg = &world.registry;
    let mut objects: Vec<ObjectCount> = counts
        .into_iter()
        .map(|((layer, object), count)| ObjectCount {
            layer: (layer as usize).checked_sub(1).and_then(|i| reg.object_layers.get(i)).map_or_else(String::new, |l| l.name.clone()),
            object: reg.objects.get(object as usize).map_or_else(String::new, |o| o.name.clone()),
            count,
        })
        .collect();
    objects.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.layer.cmp(&b.layer)).then_with(|| a.object.cmp(&b.object)));
    let volume = Volume {
        name: world.meta.name.clone(),
        author: String::new(),
        size: [w as u32, top as u32, l as u32],
        palette,
        blocks,
        origin: [w as i32 / 2, 0, l as i32 / 2],
        block_entities,
        biomes: options.biomes.then_some((biome_names, biome_ids)),
        data_version: options.target.data_version,
    };
    Ok(Extracted { volume, corner, objects, lowest_ground: lowest })
}
