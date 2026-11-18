//! WorldPainter's `MappingOp` (`wp.applyHeightMap()`, `applyLayer()`,
//! `applyTerrain()`) and `DefaultFilter`, done as WorldPainter does them
//! column for column, but tile by tile on every core, skipping the tiles an
//! image maps nothing to without decoding them.
//!
//! Operations wait in [`Session::pending`] until something reads or writes
//! the map, saves or exports it, or the script ends: then the images they
//! need are decoded side by side (a batch at a time, within a memory
//! budget) and the operations applied in their order. A column sees them
//! in the order the script gave them, so the result is the same as one
//! after the other.

use crate::image::Image;
use crate::session::{Bound, Cmp, Cond, R, Reader, Session, Size, java_round, set_value, tiles_of, water_level};
use massif_core::coords::{TILE_SIZE, TilePos, local_index};
use massif_core::level::Level;
use massif_core::tile::Tile;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// How a mapping writes, as `setAlways()`, `setWhenHigher()`,
/// `setWhenLower()`, or the terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Set,
    Higher,
    Lower,
    Terrain,
}

fn hundred() -> i32 {
    100
}

fn minus_one() -> i32 {
    -1
}

/// One `go()` of a `MappingOp`, as the script set it up.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapOp {
    pub world: usize,
    #[serde(default)]
    pub dim: i32,
    #[serde(default)]
    pub ceiling: bool,
    pub height_map: Option<usize>,
    #[serde(default = "hundred")]
    pub scale: i32,
    #[serde(default)]
    pub dx: i32,
    #[serde(default)]
    pub dy: i32,
    /// Grey mappings in their order: from `[0]` to `[1]`, to `[2]`..`[3]`.
    #[serde(default)]
    pub grey: Vec<[i32; 4]>,
    /// Colour mappings: ARGB, and the value.
    #[serde(default)]
    pub colour: Vec<(i64, i32)>,
    pub layer: Option<usize>,
    #[serde(default = "minus_one")]
    pub terrain: i32,
    #[serde(default)]
    pub layer_value: i32,
    pub mode: Mode,
    pub filter: Option<usize>,
    /// The line of the script that called `go()`.
    #[serde(default)]
    pub line: u32,
    /// What the script called: `applyHeightMap`, `applyLayer`, `applyTerrain`.
    #[serde(default)]
    pub call: String,
}

/// A filter's condition as one world has it.
#[derive(Debug, Clone, Copy)]
enum Check {
    Terrain(u16),
    BitLayer(Bound),
    IntAny(Bound),
    IntEqual(Bound, i32),
    IntHigher(Bound, i32),
    IntLower(Bound, i32),
    Biome(i32),
    Water,
    Land,
    AnnotationAny,
    Annotation(i32),
}

#[derive(Debug, Clone)]
struct Filter {
    level: Option<(LevelType, i32, i32)>,
    feather: bool,
    only: Option<Check>,
    except: Option<Check>,
    slope: Option<(f32, bool)>,
    in_selection: bool,
    outside_selection: bool,
}

#[derive(Debug, Clone, Copy)]
enum LevelType {
    Above,
    Below,
    Between,
    Outside,
}

impl Session {
    fn check_of(&mut self, c: &Cond, w: usize) -> R<Check> {
        Ok(match *c {
            Cond::Terrain { index } => Check::Terrain(self.slot(w)?.terrain_id(index)?),
            Cond::Water => Check::Water,
            Cond::Land => Check::Land,
            Cond::Biome { id } => Check::Biome(id),
            Cond::AutoBiomes => Check::Biome(255),
            Cond::AutoBiome { .. } => return Err("wp.createFilter().onlyOnAutoBiome() and exceptOnAutoBiome() are not supported yet: Massif decides the automatic biomes when it exports".into()),
            Cond::Layer { layer, value, cmp } => {
                let size = self.layer(layer)?.size;
                let bound = self.bind(layer, w)?;
                match (bound, value) {
                    (Bound::Biome, Some(v)) => Check::Biome(v),
                    (Bound::Annotations, Some(v)) if cmp.is_some() => Check::Annotation(v),
                    (Bound::Annotations, _) => Check::AnnotationAny,
                    (b, _) if size.is_bit() => Check::BitLayer(b),
                    (b, None) => Check::IntAny(b),
                    (b, Some(v)) => match cmp.unwrap_or(Cmp::Equal) {
                        Cmp::Equal => Check::IntEqual(b, v),
                        Cmp::Higher => Check::IntHigher(b, v),
                        Cmp::Lower => Check::IntLower(b, v),
                    },
                }
            }
        })
    }

    fn filter_of(&mut self, f: usize, w: usize) -> R<Filter> {
        let def = self.filters.get(f).cloned().ok_or("filter may not be null")?;
        let level = match (def.above_level, def.below_level) {
            (Some(a), Some(b)) if b >= a => Some((LevelType::Between, a, b)),
            (Some(a), Some(b)) => Some((LevelType::Outside, a, b)),
            (Some(a), None) => Some((LevelType::Above, a, 0)),
            (None, Some(b)) => Some((LevelType::Below, 0, b)),
            (None, None) => None,
        };
        let only = def.only_on.as_ref().map(|c| self.check_of(c, w)).transpose()?;
        let except = def.except_on.as_ref().map(|c| self.check_of(c, w)).transpose()?;
        let slope = def.degrees.map(|d| ((d as f64 / (180.0 / std::f64::consts::PI)).tan() as f32, def.slope_is_above));
        Ok(Filter { level, feather: def.feather, only, except, slope, in_selection: def.in_selection, outside_selection: def.outside_selection })
    }

    /// Check what can be checked now and put `op` in line.
    pub fn queue(&mut self, op: MapOp) -> R<()> {
        if op.ceiling {
            return Err("The ceiling dimensions (wp.applyHeightMap(…).applyToSurfaceCeiling() and the like) are not supported yet".into());
        }
        if op.height_map.is_none() && op.layer.is_none() && op.terrain < 0 {
            return Err("No data source (heightMap, layer or terrain) specified".into());
        }
        let slot = self.slot(op.world).map_err(|_| "world not specified".to_owned())?;
        slot.dim_index(op.dim)?;
        if op.mode == Mode::Terrain {
            if op.height_map.is_none() && op.layer.is_none() {
                self.slot(op.world)?.terrain_id(op.terrain)?;
            }
        } else if let Some(l) = op.layer {
            self.bind(l, op.world)?;
        } else {
            return Err("layer not specified".into());
        }
        if let Some(h) = op.height_map {
            self.heightmap(h)?;
        }
        if let Some(f) = op.filter {
            self.filter_of(f, op.world)?;
        }
        self.pending.push(op);
        Ok(())
    }

    /// Apply the operations waiting, in their order.
    pub fn flush(&mut self) -> R<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let ops = std::mem::take(&mut self.pending);
        // What one batch of decoded images may take.
        const BUDGET: usize = 2 << 30;
        let total = ops.len();
        let mut k = 0;
        while k < total {
            let mut end = k;
            let mut bytes = 0usize;
            let mut seen = std::collections::HashSet::new();
            while end < total && end - k < 256 {
                if let Some(h) = ops[end].height_map
                    && seen.insert(h)
                {
                    let hd = self.heightmaps[h].header;
                    let size = hd.width as usize * hd.height as usize * if hd.bits > 8 { 2 } else { 1 };
                    if end > k && bytes + size > BUDGET {
                        break;
                    }
                    bytes += size;
                }
                end += 1;
            }
            // Decode the batch's images side by side.
            let timer = std::time::Instant::now();
            let wanted: Vec<usize> = seen.into_iter().filter(|&h| self.heightmaps[h].image.is_none()).collect();
            let decoded: Vec<(usize, R<Image>)> = wanted.par_iter().map(|&h| (h, crate::image::decode(&self.heightmaps[h].path, self.heightmaps[h].channel))).collect();
            let decoding = timer.elapsed();
            for (h, img) in decoded {
                match img {
                    Ok(img) => self.heightmaps[h].image = Some(Arc::new(img)),
                    Err(e) => {
                        let op = ops[k..end].iter().find(|o| o.height_map == Some(h)).expect("wanted by one");
                        return Err(at(op, e));
                    }
                }
            }
            let timer = std::time::Instant::now();
            for (j, op) in ops[k..end].iter().enumerate() {
                self.io.check()?;
                let label = format!("{} {} of {total}", if op.height_map.is_some() { "Applying mask" } else { "Applying" }, k + j + 1);
                self.io.progress.set((k + j) as f32 / total as f32, &label);
                self.apply(op).map_err(|e| at(op, e))?;
            }
            if std::env::var_os("MASSIF_PERF").is_some() {
                eprintln!("{} operations: {} images decoded in {:.2?}, applied in {:.2?}", end - k, wanted.len(), decoding, timer.elapsed());
            }
            for op in &ops[k..end] {
                if let Some(h) = op.height_map {
                    self.heightmaps[h].image = None;
                    self.heightmaps[h].argb = None;
                }
            }
            k = end;
        }
        self.io.progress.set(1.0, "");
        Ok(())
    }

    fn apply(&mut self, op: &MapOp) -> R<()> {
        // What each column gets, before the filter.
        enum Src {
            Grey(Arc<Image>, Vec<i16>),
            Colour(u32, u32, Arc<Vec<u32>>, HashMap<u32, i16>),
            Constant(i32),
        }
        let src = match op.height_map {
            Some(h) => {
                let hm = self.heightmap(h)?;
                if !op.colour.is_empty() {
                    let (w, ht) = (hm.header.width, hm.header.height);
                    let argb = hm.argb()?;
                    Src::Colour(w, ht, argb, op.colour.iter().map(|&(c, v)| (c as u32, v as i16)).collect())
                } else {
                    let img = hm.image()?;
                    let mut table = vec![-1i16; 65536];
                    for &[lo, hi, to_lo, to_hi] in &op.grey {
                        let (lo, hi) = (lo.clamp(0, 65535) as usize, hi.clamp(0, 65535) as usize);
                        if to_lo == to_hi || lo == hi {
                            table[lo..=hi.max(lo)].fill(to_lo as i16);
                        } else {
                            let factor = (to_hi - to_lo) as f32 / (hi - lo) as f32;
                            for (i, t) in table.iter_mut().enumerate().take(hi + 1).skip(lo) {
                                *t = (to_lo + java_round((i - lo) as f32 * factor)) as i16;
                            }
                        }
                    }
                    Src::Grey(img, table)
                }
            }
            None if op.mode != Mode::Terrain || op.layer.is_some() => Src::Constant(op.layer_value),
            None => Src::Constant(op.terrain),
        };
        let bound = match op.mode {
            Mode::Terrain => None,
            _ => Some(self.bind(op.layer.ok_or("layer not specified")?, op.world)?),
        };
        let filter = op.filter.map(|f| self.filter_of(f, op.world)).transpose()?;
        let size = op.layer.and_then(|l| self.layers.get(l)).map_or(Size::None, |l| l.size);
        let order = op.layer.and_then(|l| self.layers.get(l)).map_or(0, |l| crate::session::bucket(&l.id));
        let bit = size.is_bit();
        // WorldPainter scales a value by a feathered filter's strength when
        // a grey mapping was given (`toLevel()` makes one even for a layer).
        let smooth = !op.grey.is_empty() && op.mode != Mode::Terrain && !matches!(bound, Some(Bound::Biome | Bound::Annotations));
        let w = op.world;
        let slot = self.slot(w)?;
        let d = slot.dim_index(op.dim)?;

        // The rectangle of blocks it covers: the map's, within the image's.
        let Some((lo, hi)) = slot.world.dims[d].bounds() else { return Ok(()) };
        let (mut x0, mut z0, mut x1, mut z1) = (lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
        let s = op.scale as f32 / 100.0;
        let translate = op.scale == 100;
        let (iw, ih) = match &src {
            Src::Grey(img, _) => (img.width, img.height),
            Src::Colour(w, h, ..) => (*w, *h),
            Src::Constant(_) => (0, 0),
        };
        if !matches!(src, Src::Constant(_)) {
            let (ew, eh) = if translate { (iw as i64, ih as i64) } else { ((iw as f64 * s as f64).ceil() as i64, (ih as f64 * s as f64).ceil() as i64) };
            x0 = x0.max(op.dx);
            z0 = z0.max(op.dy);
            x1 = (x1 as i64).min(op.dx as i64 + ew) as i32;
            z1 = (z1 as i64).min(op.dy as i64 + eh) as i32;
            if x0 >= x1 || z0 >= z1 {
                return Ok(());
            }
        }
        // Image coordinates of a column, as WorldPainter's transform.
        let sx = (1.0f32 / s) as f64;
        let (mx, mz) = (-(op.dx as f64) * sx, -(op.dy as f64) * sx);
        let to_image = |x: i32, z: i32| -> (i64, i64) {
            if translate {
                ((x - op.dx) as i64, (z - op.dy) as i64)
            } else {
                ((sx * x as f64 + mx) as f32 as i64, (sx * z as f64 + mz) as f32 as i64)
            }
        };
        let inside = |ix: i64, iy: i64| ix >= 0 && iy >= 0 && ix < iw as i64 && iy < ih as i64;
        let value_at = |x: i32, z: i32| -> i16 {
            match &src {
                Src::Grey(img, table) => {
                    let (ix, iy) = to_image(x, z);
                    let v = if inside(ix, iy) { img.at(iy as usize * iw as usize + ix as usize) } else { 0.0 };
                    // Java's Math.round of a double.
                    let v = (v as f64 + 0.5).floor();
                    if !(0.0..=65535.0).contains(&v) { -1 } else { table[v as usize] }
                }
                Src::Colour(_, _, argb, map) => {
                    let (ix, iy) = if translate {
                        ((x - op.dx) as i64, (z - op.dy) as i64)
                    } else {
                        let fx = (sx * x as f64 + mx) as f32;
                        let fz = (sx * z as f64 + mz) as f32;
                        (java_round(fx) as i64, java_round(fz) as i64)
                    };
                    let c = if inside(ix, iy) { argb[iy as usize * iw as usize + ix as usize] } else { 0 };
                    map.get(&c).copied().unwrap_or(-1)
                }
                Src::Constant(v) => *v as i16,
            }
        };

        // The tiles of the map in it, and what the image gives each column.
        let level = slot.world.dims[d].ground();
        let tiles: Vec<TilePos> = tiles_of(x0, z0, x1, z1).filter(|p| level.contains(*p)).collect();
        let cancel = &self.io.cancel;
        let mut touched: Vec<(TilePos, Vec<(u16, i16)>)> = tiles
            .par_iter()
            .filter_map(|&pos| {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return None;
                }
                let r = pos.rect();
                let (cx0, cz0, cx1, cz1) = (r.x0.max(x0), r.z0.max(z0), r.x1.min(x1), r.z1.min(z1));
                let mut out = Vec::new();
                // The common case, fast: 8 bits a pixel, one for one, rows
                // of black (unmapped) skipped eight pixels at a time.
                if let (true, Src::Grey(img, table)) = (translate, &src)
                    && let crate::image::Samples::U8(samples) = &img.samples
                {
                    let skip_black = table[0] < 0;
                    for z in cz0..cz1 {
                        let start = (z - op.dy) as usize * iw as usize + (cx0 - op.dx) as usize;
                        let row = &samples[start..start + (cx1 - cx0) as usize];
                        if skip_black && all_zero(row) {
                            continue;
                        }
                        for (k, &px) in row.iter().enumerate() {
                            let v = table[px as usize];
                            if v >= 0 {
                                out.push((local_index(cx0 + k as i32, z) as u16, v));
                            }
                        }
                    }
                    return (!out.is_empty()).then_some((pos, out));
                }
                for z in cz0..cz1 {
                    for x in cx0..cx1 {
                        let v = value_at(x, z);
                        if v >= 0 {
                            out.push((local_index(x, z) as u16, v));
                        }
                    }
                }
                (!out.is_empty()).then_some((pos, out))
            })
            .collect();
        self.io.check()?;
        if touched.is_empty() {
            return Ok(());
        }
        touched.sort_unstable_by_key(|t| (t.0.z, t.0.x));

        // Resolve what the values become in this world.
        let mut warn = None;
        let slot = self.slot(w)?;
        let mut terrain_ids: HashMap<i16, u16> = HashMap::new();
        let mut biome_ids: HashMap<i16, u16> = HashMap::new();
        {
            let mut values: Vec<i16> = touched.iter().flat_map(|t| t.1.iter().map(|e| e.1)).collect();
            values.sort_unstable();
            values.dedup();
            for v in values {
                match bound {
                    None => {
                        if let Some(n) = crate::session::custom_slot(v as i32)
                            && !slot.custom.contains_key(&n)
                        {
                            warn = Some(format!("Custom terrain {n} is not installed in this world (wp.installCustomTerrain()): painted as an empty terrain, as WorldPainter does"));
                        }
                        terrain_ids.insert(v, slot.terrain_id(v as i32)?);
                    }
                    Some(Bound::Biome) => {
                        biome_ids.insert(v, slot.biome_id(v as i32)?);
                    }
                    _ => {}
                }
            }
        }
        if let Some(message) = warn {
            self.warn_once(message);
        }
        let slot = self.slot(w)?;
        // The tiles, decoded once, with their neighbours for the slope.
        let mut warm: Vec<TilePos> = touched.iter().map(|t| t.0).collect();
        if filter.as_ref().is_some_and(|f| f.slope.is_some()) {
            let around: Vec<TilePos> = warm.iter().flat_map(|p| (-1..=1).flat_map(move |dz| (-1..=1).map(move |dx| p.offset(dx, dz)))).collect();
            warm = around;
            warm.sort_unstable_by_key(|p| (p.z, p.x));
            warm.dedup();
        }
        slot.world.dims[d].ground_mut().warm(&warm);
        let selection = slot.world.dims[d].selection.clone();
        let reader = Reader::new(slot, d);
        let level = slot.world.dims[d].ground();
        let min_y = slot.world.dims[d].min_y;

        // Filter and mode, column by column, reading the map as it is.
        let writes: Vec<(TilePos, Vec<(u16, i16)>)> = touched
            .into_par_iter()
            .filter_map(|(pos, entries)| {
                let tile = level.view(pos)?;
                let (ox, oz) = pos.origin();
                let mut out = Vec::with_capacity(entries.len());
                for (i, mut v) in entries {
                    let (x, z) = (ox + (i as i32 & (TILE_SIZE - 1)), oz + (i as i32 >> 7));
                    if let Some(f) = &filter {
                        let fv = strength(f, &reader, level, &tile, pos, i as usize, x, z, selection.as_deref());
                        if fv == 0.0 {
                            continue;
                        }
                        if smooth && fv != 1.0 {
                            v = java_round(fv * v as f32) as i16;
                        }
                    }
                    match (op.mode, bound) {
                        (Mode::Terrain, _) | (Mode::Set, _) | (_, None) => {}
                        (Mode::Higher, Some(b)) => {
                            if bit {
                                if v == 0 {
                                    continue;
                                }
                            } else if reader.value(b, &tile, pos, i as usize) >= v as i32 {
                                continue;
                            }
                        }
                        (Mode::Lower, Some(b)) => {
                            if bit {
                                if v != 0 {
                                    continue;
                                }
                            } else if reader.value(b, &tile, pos, i as usize) <= v as i32 {
                                continue;
                            }
                        }
                    }
                    out.push((i, v));
                }
                (!out.is_empty()).then_some((pos, out))
            })
            .collect();
        drop(reader);

        // Write.
        let combined: Vec<Vec<massif_core::combined::Component>> = std::iter::once(Vec::new()).chain(slot.world.registry.combined.iter().map(|c| c.parts.clone())).collect();
        let beaches = terrain_ids.values().any(|&t| t == massif_legacy::terrains::BEACH_MARK).then(|| slot.beaches());
        let slot = self.slot(w)?;
        let mut kept: Vec<(TilePos, Vec<(u16, i16)>)> = Vec::new();
        for (pos, entries) in writes {
            if bound.is_some_and(crate::session::overlaid) {
                kept.push((pos, entries));
                continue;
            }
            let Some(tile) = slot.world.dims[d].ground_mut().tile_mut(pos) else { continue };
            let (ox, oz) = pos.origin();
            for (i, v) in entries {
                let i = i as usize;
                let (x, z) = (ox + (i as i32 & (TILE_SIZE - 1)), oz + (i as i32 >> 7));
                match bound {
                    None => {
                        let id = terrain_ids[&v];
                        let id = match &beaches {
                            Some(b) if id == massif_legacy::terrains::BEACH_MARK => b.at(x, z, tile.top.get(i)),
                            _ => id,
                        };
                        tile.terrain.set(i, id);
                    }
                    Some(Bound::Biome) => tile.biome.set(i, biome_ids[&v]),
                    Some(b) => set_value(b, v as i32, tile, i, x, z, min_y, &combined),
                }
            }
        }
        // Layers of a strength: kept as painted until the script is done.
        if let Some(b) = bound {
            for (pos, entries) in kept {
                for (i, v) in entries {
                    crate::session::paint(&mut slot.painted, d, pos, b, order, i as usize, v as i32);
                }
            }
        }
        slot.changed = true;
        Ok(())
    }
}

/// Whether every byte of `row` is 0, eight at a time.
#[inline]
fn all_zero(row: &[u8]) -> bool {
    let (words, rest) = row.split_at(row.len() / 8 * 8);
    words.as_chunks::<8>().0.iter().all(|w| u64::from_ne_bytes(*w) == 0) && rest.iter().all(|&b| b == 0)
}

fn at(op: &MapOp, e: String) -> String {
    if op.line > 0 {
        format!("{e} (in the {}() of line {})", if op.call.is_empty() { "mapping" } else { &op.call }, op.line)
    } else {
        e
    }
}

/// WorldPainter's height of column `(x, z)`, from `tile` when it holds it.
#[inline]
fn height(level: &Level, tile: &Tile, pos: TilePos, x: i32, z: i32) -> f32 {
    let p = TilePos::containing(x, z);
    if p == pos {
        tile.top.get(local_index(x, z)) - 1.0
    } else {
        level.view(p).map_or(-f32::MAX, |t| t.top.get(local_index(x, z)) - 1.0)
    }
}

/// WorldPainter's `Dimension.getSlope`: the steepest of four directions,
/// in blocks of rise a block.
pub fn slope(h: impl Fn(i32, i32) -> f32, x: i32, z: i32) -> f32 {
    const ROOT_EIGHT: f32 = 2.828_427_f32;
    ((h(x + 1, z) - h(x - 1, z)).abs() / 2.0)
        .max((h(x + 1, z + 1) - h(x - 1, z - 1)).abs() / ROOT_EIGHT)
        .max(((h(x, z + 1) - h(x, z - 1)).abs() / 2.0).max((h(x - 1, z + 1) - h(x + 1, z - 1)).abs() / ROOT_EIGHT))
}

/// Whether a column passes one condition, as WorldPainter's
/// `OnlyOnTerrainOrLayerFilter` (`only`) or `ExceptOnTerrainOrLayerFilter`
/// says, quirks included: an "only on" with a layer value keeps the
/// columns that do *not* have it, and an "except on" those that do.
fn passes(c: Check, only: bool, reader: &Reader, tile: &Tile, pos: TilePos, i: usize) -> bool {
    let int_height = java_round(tile.top.get(i) - 1.0);
    let water = water_level(tile, i, reader.min_y) > int_height;
    let lava = reader.lava(tile, i);
    let value = |b: Bound| reader.value(b, tile, pos, i);
    let biome = || reader.value(Bound::Biome, tile, pos, i);
    if only {
        match c {
            Check::Biome(v) => biome() == v,
            Check::BitLayer(b) => value(b) != 0,
            Check::IntAny(b) => value(b) != 0,
            Check::IntEqual(b, v) => value(b) != v,
            Check::IntHigher(b, v) => value(b) < v,
            Check::IntLower(b, v) => value(b) > v,
            Check::Terrain(t) => tile.terrain.get(i) == t,
            Check::Water => water && !lava,
            Check::Land => !water,
            Check::AnnotationAny => tile.annotations.get(i) != 0,
            Check::Annotation(v) => tile.annotations.get(i) as i32 == v,
        }
    } else {
        match c {
            Check::Biome(v) => biome() != v,
            Check::BitLayer(b) => value(b) == 0,
            Check::IntAny(b) => value(b) == 0,
            Check::IntEqual(b, v) => value(b) == v,
            Check::IntHigher(b, v) => value(b) >= v,
            Check::IntLower(b, v) => value(b) <= v,
            Check::Terrain(t) => tile.terrain.get(i) != t,
            Check::Water => !(water && !lava),
            Check::Land => water,
            Check::AnnotationAny => tile.annotations.get(i) == 0,
            Check::Annotation(v) => tile.annotations.get(i) as i32 != v,
        }
    }
}

/// WorldPainter's `DefaultFilter.modifyStrength(x, y, 1)`.
#[allow(clippy::too_many_arguments)]
fn strength(f: &Filter, reader: &Reader, level: &Level, tile: &Tile, pos: TilePos, i: usize, x: i32, z: i32, selection: Option<&massif_core::selection::Selection>) -> f32 {
    if f.in_selection || f.outside_selection {
        let selected = selection.is_some_and(|s| massif_core::selection::lets(s.get(x, z), x, z));
        if (f.in_selection && !selected) || (f.outside_selection && selected) {
            return 0.0;
        }
    }
    if let Some(c) = f.except
        && !passes(c, false, reader, tile, pos, i)
    {
        return 0.0;
    }
    if let Some(c) = f.only
        && !passes(c, true, reader, tile, pos, i)
    {
        return 0.0;
    }
    if let Some((kind, above, below)) = f.level {
        let h = java_round(tile.top.get(i) - 1.0);
        let fade = |d: i32| 1.0 - d as f32 / 4.0;
        match kind {
            LevelType::Above if h < above => return if f.feather { fade(above - h).max(0.0) } else { 0.0 },
            LevelType::Below if h > below => return if f.feather { fade(h - below).max(0.0) } else { 0.0 },
            LevelType::Between if h < above || h > below => return if f.feather { fade(above - h).min(fade(h - below)).max(0.0) } else { 0.0 },
            LevelType::Outside if h > below && h < above => return if f.feather { fade(h - below).max(fade(above - h)).max(0.0) } else { 0.0 },
            _ => {}
        }
    }
    if let Some((tan, above)) = f.slope {
        let s = slope(|x, z| height(level, tile, pos, x, z), x, z);
        if if above { s < tan } else { s > tan } {
            return 0.0;
        }
    }
    1.0
}
