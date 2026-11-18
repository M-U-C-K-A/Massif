//! Rectangles of columns read and written at once, as typed arrays: what
//! makes Massif's own API fast where WorldPainter's goes column by column.
//!
//! A region `[x0, x1) × [z0, z1)` is laid out row after row, `x` first:
//! column `(x, z)` at `(z - z0) * (x1 - x0) + (x - x0)`.
//!
//! | what | values | where the map has no tile | to leave a column |
//! |---|---|---|---|
//! | `height` | `Float32Array`, the y of the top block | `NaN` | `NaN` |
//! | `water` | `Float32Array`, the y of the top water block | `NaN` | `NaN` (sets no water: `-Infinity`) |
//! | `terrain`, `biome`, `annotations`, `cover`, `trees`, `understory`, `pockets`, `sizes` | `Uint16Array`, Massif's numbers | 65535 | 65535 |
//! | `selection` | `Uint16Array`, 0 to 255 | 0 | — |

use crate::session::{R, Session, tiles_of};
use massif_core::coords::{TilePos, local_index};
use massif_core::tile::{NO_FLUID, Tile};
use rayon::prelude::*;

pub enum Data {
    F32(Vec<f32>),
    U16(Vec<u16>),
    I32(Vec<i32>),
}

/// The most columns one region may hold: 16384 × 16384.
const MAX: i64 = 1 << 28;

const LEAVE: u16 = u16::MAX;

fn check(x0: i32, z0: i32, x1: i32, z1: i32) -> R<usize> {
    let n = (x1 as i64 - x0 as i64) * (z1 as i64 - z0 as i64);
    if x1 <= x0 || z1 <= z0 {
        return Err(format!("An empty region: x from {x0} to {x1}, z from {z0} to {z1}"));
    }
    if n > MAX {
        return Err(format!("A region of {n} columns: at most {MAX} at once"));
    }
    Ok(n as usize)
}

fn plane<'a>(t: &'a Tile, what: &str) -> Option<&'a massif_core::plane::Plane<u16>> {
    Some(match what {
        "terrain" => &t.terrain,
        "biome" => &t.biome,
        "annotations" => &t.annotations,
        "cover" => &t.cover,
        "trees" => &t.objects,
        "understory" => &t.understory,
        "pockets" => &t.pockets,
        "sizes" => &t.sizes,
        _ => return None,
    })
}

fn plane_mut<'a>(t: &'a mut Tile, what: &str) -> Option<&'a mut massif_core::plane::Plane<u16>> {
    Some(match what {
        "terrain" => &mut t.terrain,
        "biome" => &mut t.biome,
        "annotations" => &mut t.annotations,
        "cover" => &mut t.cover,
        "trees" => &mut t.objects,
        "understory" => &mut t.understory,
        "pockets" => &mut t.pockets,
        "sizes" => &mut t.sizes,
        _ => return None,
    })
}

const KINDS: &str = "height, water, terrain, biome, annotations, cover, trees, understory, pockets, sizes or selection";

/// Read `what` over a region.
#[allow(clippy::too_many_arguments)]
pub fn get(s: &mut Session, w: usize, dim: i32, what: &str, x0: i32, z0: i32, x1: i32, z1: i32) -> R<Data> {
    let n = check(x0, z0, x1, z1)?;
    let (slot, d) = s.column(w, dim)?;
    let width = (x1 - x0) as usize;
    if what == "selection" {
        let sel = slot.world.dims[d].selection.clone();
        let mut out = vec![0u16; n];
        if let Some(sel) = sel {
            out.par_chunks_mut(width).enumerate().for_each(|(row, line)| {
                let z = z0 + row as i32;
                for (k, v) in line.iter_mut().enumerate() {
                    *v = sel.get(x0 + k as i32, z) as u16;
                }
            });
        }
        return Ok(Data::U16(out));
    }
    let floats = matches!(what, "height" | "water");
    if !floats && !matches!(what, "terrain" | "biome" | "annotations" | "cover" | "trees" | "understory" | "pockets" | "sizes") {
        return Err(format!("Unknown kind of column data \"{what}\": {KINDS}"));
    }
    if matches!(what, "cover" | "trees" | "understory") {
        crate::session::resolve(slot);
    }
    let tiles: Vec<TilePos> = tiles_of(x0, z0, x1, z1).collect();
    let level = slot.world.dims[d].ground_mut();
    level.warm(&tiles);
    let level = slot.world.dims[d].ground();
    let mut f = if floats { vec![f32::NAN; n] } else { Vec::new() };
    let mut u = if floats { Vec::new() } else { vec![LEAVE; n] };
    // Row by row, tile by tile, the rows in parallel.
    let fill = |z: i32, fl: Option<&mut [f32]>, ul: Option<&mut [u16]>| {
        let mut fl = fl;
        let mut ul = ul;
        let mut x = x0;
        while x < x1 {
            let pos = TilePos::containing(x, z);
            let end = ((pos.x + 1) * massif_core::TILE_SIZE).min(x1);
            if let Some(t) = level.view(pos) {
                for xx in x..end {
                    let i = local_index(xx, z);
                    let k = (xx - x0) as usize;
                    if let Some(fl) = fl.as_deref_mut() {
                        fl[k] = match what {
                            "height" => t.top.get(i) - 1.0,
                            _ => match t.fluid.get(i) {
                                NO_FLUID => f32::NAN,
                                v if v as f32 <= t.top.get(i) => f32::NAN,
                                v => v as f32 - 1.0,
                            },
                        };
                    }
                    if let Some(ul) = ul.as_deref_mut() {
                        ul[k] = plane(&t, what).map_or(LEAVE, |p| p.get(i));
                    }
                }
            }
            x = end;
        }
    };
    if floats {
        f.par_chunks_mut(width).enumerate().for_each(|(row, line)| fill(z0 + row as i32, Some(line), None));
        Ok(Data::F32(f))
    } else {
        u.par_chunks_mut(width).enumerate().for_each(|(row, line)| fill(z0 + row as i32, None, Some(line)));
        Ok(Data::U16(u))
    }
}

/// Write `what` over a region; `NaN` and 65535 leave a column as it is.
#[allow(clippy::too_many_arguments)]
pub fn set(s: &mut Session, w: usize, dim: i32, what: &str, x0: i32, z0: i32, x1: i32, z1: i32, data: Data) -> R<()> {
    let n = check(x0, z0, x1, z1)?;
    let len = match &data {
        Data::F32(v) => v.len(),
        Data::U16(v) => v.len(),
        Data::I32(v) => v.len(),
    };
    if len != n {
        return Err(format!("A region of {} × {} columns takes {n} values, not {len}", x1 - x0, z1 - z0));
    }
    let floats = matches!(what, "height" | "water");
    if !floats && !matches!(what, "terrain" | "biome" | "annotations" | "cover" | "trees" | "understory" | "pockets" | "sizes") {
        return Err(format!("Unknown kind of column data \"{what}\" to write: {}", KINDS.trim_end_matches(" or selection")));
    }
    let (slot, d) = s.column(w, dim)?;
    if matches!(what, "cover" | "trees" | "understory") {
        crate::session::resolve(slot);
    }
    let (min, max) = (slot.world.dims[d].min_y as f32, slot.world.dims[d].max_y as f32);
    // Numbers of the registry a plane may hold.
    let reg = &slot.world.registry;
    let limit = match what {
        "terrain" => reg.terrains.len(),
        "biome" => reg.biomes.len(),
        "annotations" => 16,
        "cover" => reg.covers.len() + 1,
        "trees" | "understory" => reg.object_layers.len() + 1,
        "pockets" => reg.pockets.len() + 1,
        "sizes" => massif_core::tile::SIZE_LARGE as usize + 1,
        _ => usize::MAX,
    };
    let as_f32 = |k: usize| -> f32 {
        match &data {
            Data::F32(v) => v[k],
            Data::U16(v) => v[k] as f32,
            Data::I32(v) => v[k] as f32,
        }
    };
    let as_u16 = |k: usize| -> R<u16> {
        let v = match &data {
            Data::F32(v) if v[k].is_nan() => return Ok(LEAVE),
            Data::F32(v) => v[k] as i64,
            Data::U16(v) => v[k] as i64,
            Data::I32(v) => v[k] as i64,
        };
        if v == LEAVE as i64 {
            return Ok(LEAVE);
        }
        if v < 0 || v as usize >= limit {
            return Err(format!("{v} is not a {what} of this map (0 to {})", limit - 1));
        }
        Ok(v as u16)
    };
    let tiles: Vec<TilePos> = tiles_of(x0, z0, x1, z1).filter(|p| slot.world.dims[d].ground().contains(*p)).collect();
    let level = slot.world.dims[d].ground_mut();
    for pos in tiles {
        let Some(t) = level.tile_mut(pos) else { continue };
        let r = pos.rect();
        for z in r.z0.max(z0)..r.z1.min(z1) {
            for x in r.x0.max(x0)..r.x1.min(x1) {
                let k = (z - z0) as usize * (x1 - x0) as usize + (x - x0) as usize;
                let i = local_index(x, z);
                if floats {
                    let v = as_f32(k);
                    if v.is_nan() {
                        continue;
                    }
                    if what == "height" {
                        t.top.set(i, massif_core::codec::snap_height((v + 1.0).clamp(min, max)));
                    } else if v == f32::NEG_INFINITY || v < min - 1.0 {
                        t.fluid.set(i, NO_FLUID);
                    } else {
                        t.fluid.set(i, (v.round() as i32 + 1).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16);
                    }
                } else {
                    let v = as_u16(k)?;
                    if v != LEAVE {
                        plane_mut(t, what).expect("checked").set(i, v);
                    }
                }
            }
        }
    }
    slot.changed = true;
    Ok(())
}
