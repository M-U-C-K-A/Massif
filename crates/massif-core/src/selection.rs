//! Selections: the part of a map the tools may change.
//!
//! A selection gives every column a weight from 0 (outside: nothing
//! changes there) to 255 (inside), the values between along a feathered
//! edge. It is kept by tile: a tile wholly inside is one flag, a tile on the
//! edge its 16,384 weights, a tile outside nothing, so a rectangle over a
//! huge map costs a few bytes a tile. Brushes and the whole-map filters
//! multiply what they do by the weight; a map without a selection is edited
//! everywhere, exactly as before.

use crate::coords::{BlockRect, TILE_AREA, TILE_SIZE, TilePos, local_index};
use crate::filter::Filter;
use crate::world::Dimension;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The weight of a column wholly inside.
pub const IN: u8 = 255;
/// Columns weighing at least this are drawn inside the outline.
pub const HALF: u8 = 128;
/// The widest feather, in blocks: the edge fades over this many blocks on
/// either side at most.
pub const MAX_FEATHER: i32 = 64;

/// The weights of one tile.
#[derive(Debug, Clone, PartialEq)]
pub enum Cells {
    /// Every column inside.
    Full,
    /// A weight per column ([`local_index`] order).
    Part(Arc<[u8]>),
}

impl Cells {
    #[inline]
    pub fn get(&self, i: usize) -> u8 {
        match self {
            Cells::Full => IN,
            Cells::Part(w) => w[i],
        }
    }

    /// The weights as a tile holds them: `None` when all are 0.
    fn of(weights: Vec<u8>) -> Option<Cells> {
        if weights.iter().all(|&w| w == IN) {
            Some(Cells::Full)
        } else if weights.iter().all(|&w| w == 0) {
            None
        } else {
            Some(Cells::Part(weights.into()))
        }
    }

    fn to_vec(&self) -> Vec<u8> {
        match self {
            Cells::Full => vec![IN; TILE_AREA],
            Cells::Part(w) => w.to_vec(),
        }
    }
}

/// How a new shape changes the selection there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Combine {
    /// The shape becomes the selection.
    #[default]
    Replace,
    /// The shape is added to it (Shift).
    Add,
    /// The shape is taken out of it (Alt).
    Subtract,
    /// Only what both hold is kept (Shift and Alt).
    Intersect,
}

impl Combine {
    pub const ALL: [Combine; 4] = [Combine::Replace, Combine::Add, Combine::Subtract, Combine::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            Combine::Replace => "New",
            Combine::Add => "Add",
            Combine::Subtract => "Subtract",
            Combine::Intersect => "Intersect",
        }
    }

    fn cell(self, a: u8, b: u8) -> u8 {
        match self {
            Combine::Replace => b,
            Combine::Add => a.max(b),
            Combine::Subtract => ((a as u32 * (IN - b) as u32 + 127) / 255) as u8,
            Combine::Intersect => a.min(b),
        }
    }

    /// One tile of the result, from that tile of the selection (`a`) and of
    /// the shape (`b`).
    fn tile(self, a: Option<&Cells>, b: Option<&Cells>) -> Option<Cells> {
        match (self, a, b) {
            (Combine::Replace, _, b) => b.cloned(),
            (Combine::Add, Some(Cells::Full), _) | (Combine::Add, _, Some(Cells::Full)) => Some(Cells::Full),
            (Combine::Add, x, None) | (Combine::Add, None, x) => x.cloned(),
            (Combine::Subtract, None, _) | (Combine::Subtract, _, Some(Cells::Full)) => None,
            (Combine::Subtract, a, None) => a.cloned(),
            (Combine::Intersect, None, _) | (Combine::Intersect, _, None) => None,
            (Combine::Intersect, Some(Cells::Full), b) => b.cloned(),
            (Combine::Intersect, a, Some(Cells::Full)) => a.cloned(),
            (op, Some(a), Some(b)) => Cells::of((0..TILE_AREA).map(|i| op.cell(a.get(i), b.get(i))).collect()),
        }
    }
}

/// A shape drawn on the map, in blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// The columns of a rectangle.
    Rect(BlockRect),
    /// The ellipse inscribed in a rectangle.
    Ellipse(BlockRect),
    /// A closed polygon (the lasso's path), points in continuous blocks.
    Polygon(Vec<(f32, f32)>),
}

impl Shape {
    /// The columns the shape may cover.
    pub fn bounds(&self) -> BlockRect {
        match self {
            Shape::Rect(r) | Shape::Ellipse(r) => *r,
            Shape::Polygon(points) => {
                let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, z) in points {
                    x0 = x0.min(x);
                    z0 = z0.min(z);
                    x1 = x1.max(x);
                    z1 = z1.max(z);
                }
                if points.len() < 3 {
                    return BlockRect::new(0, 0, 0, 0);
                }
                BlockRect::new(x0.floor() as i32, z0.floor() as i32, x1.ceil() as i32, z1.ceil() as i32)
            }
        }
    }

    /// The columns `x0..x1` of row `z` the shape covers, as spans (a column
    /// is in when its centre is).
    fn spans(&self, z: i32, out: &mut Vec<(i32, i32)>) {
        out.clear();
        let zc = z as f32 + 0.5;
        // Columns whose centre lies in [a, b).
        let span = |a: f32, b: f32| ((a - 0.5).ceil() as i32, (b - 0.5).ceil() as i32);
        match self {
            Shape::Rect(r) => {
                if z >= r.z0 && z < r.z1 {
                    out.push((r.x0, r.x1));
                }
            }
            Shape::Ellipse(r) => {
                let (rx, rz) = (r.width() as f32 / 2.0, r.depth() as f32 / 2.0);
                let (cx, cz) = (r.x0 as f32 + rx, r.z0 as f32 + rz);
                let dz = (zc - cz) / rz.max(1e-6);
                if dz.abs() < 1.0 {
                    let half = rx * (1.0 - dz * dz).sqrt();
                    out.push(span(cx - half, cx + half));
                }
            }
            Shape::Polygon(points) => {
                let mut xs: Vec<f32> = Vec::new();
                for (i, &(px, pz)) in points.iter().enumerate() {
                    let (qx, qz) = points[(i + 1) % points.len()];
                    if (pz <= zc) != (qz <= zc) {
                        xs.push(px + (zc - pz) * (qx - px) / (qz - pz));
                    }
                }
                xs.sort_by(f32::total_cmp);
                for pair in xs.as_chunks::<2>().0 {
                    out.push(span(pair[0], pair[1]));
                }
            }
        }
    }
}

/// The selected part of a map. See the module's notes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    tiles: HashMap<TilePos, Cells>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The weights of a tile; `None`: outside.
    #[inline]
    pub fn tile(&self, pos: TilePos) -> Option<&Cells> {
        self.tiles.get(&pos)
    }

    /// The tiles that hold some of the selection.
    pub fn positions(&self) -> impl Iterator<Item = TilePos> + '_ {
        self.tiles.keys().copied()
    }

    /// Weight of column `(x, z)`, 0 to 255.
    #[inline]
    pub fn get(&self, x: i32, z: i32) -> u8 {
        self.tiles.get(&TilePos::containing(x, z)).map_or(0, |c| c.get(local_index(x, z)))
    }

    /// Weight of column `(x, z)`, 0 to 1.
    #[inline]
    pub fn weight(&self, x: i32, z: i32) -> f32 {
        self.get(x, z) as f32 / IN as f32
    }

    /// The rectangle of blocks of the tiles holding the selection.
    pub fn bounds(&self) -> Option<BlockRect> {
        let mut it = self.tiles.keys();
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo = TilePos::new(lo.x.min(p.x), lo.z.min(p.z));
            hi = TilePos::new(hi.x.max(p.x), hi.z.max(p.z));
        }
        Some(BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE))
    }

    /// How many columns it holds, those of the edge counted by their weight.
    pub fn area(&self) -> f64 {
        self.tiles
            .values()
            .map(|c| match c {
                Cells::Full => TILE_AREA as f64,
                Cells::Part(w) => w.iter().map(|&v| v as u64).sum::<u64>() as f64 / IN as f64,
            })
            .sum()
    }

    /// The weights of the columns of `rect`, row-major; `None` when none of
    /// it is selected.
    pub fn weights(&self, rect: BlockRect) -> Option<Vec<u8>> {
        if rect.is_empty() || !rect.tiles().any(|p| self.tiles.contains_key(&p)) {
            return None;
        }
        let w = rect.width() as usize;
        let mut out = vec![0u8; w * rect.depth() as usize];
        for pos in rect.tiles() {
            let Some(cells) = self.tiles.get(&pos) else { continue };
            let clip = rect.intersect(&pos.rect());
            for z in clip.z0..clip.z1 {
                let row = &mut out[(z - rect.z0) as usize * w..][..w];
                let row = &mut row[(clip.x0 - rect.x0) as usize..(clip.x1 - rect.x0) as usize];
                match cells {
                    Cells::Full => row.fill(IN),
                    Cells::Part(c) => row.copy_from_slice(&c[local_index(clip.x0, z)..][..row.len()]),
                }
            }
        }
        Some(out)
    }

    /// The selection of the given weights, tile by tile (each tile's columns
    /// in [`local_index`] order); a tile all 0 is left out.
    pub fn from_tiles(tiles: impl IntoIterator<Item = (TilePos, Vec<u8>)>) -> Self {
        Self {
            tiles: tiles.into_iter().filter(|(_, w)| w.len() == TILE_AREA).filter_map(|(p, w)| Cells::of(w).map(|c| (p, c))).collect(),
        }
    }

    /// The selection `f` draws over `rect`: `f(z, x0, row)` sets the weights
    /// of the columns `x0..` of row `z` (0 to start with).
    pub fn rasterize(rect: BlockRect, f: impl Fn(i32, i32, &mut [u8]) + Sync) -> Self {
        let positions: Vec<TilePos> = rect.tiles().collect();
        let tiles = positions
            .into_par_iter()
            .filter_map(|pos| {
                let clip = rect.intersect(&pos.rect());
                let mut w = vec![0u8; TILE_AREA];
                for z in clip.z0..clip.z1 {
                    let at = local_index(clip.x0, z);
                    f(z, clip.x0, &mut w[at..at + clip.width() as usize]);
                }
                Cells::of(w).map(|c| (pos, c))
            })
            .collect();
        Self { tiles }
    }

    /// The columns a shape covers.
    pub fn shape(shape: &Shape) -> Self {
        let rect = shape.bounds();
        if rect.is_empty() {
            return Self::default();
        }
        Self::rasterize(rect, |z, x0, row| {
            let mut spans = Vec::new();
            shape.spans(z, &mut spans);
            let x1 = x0 + row.len() as i32;
            for &(a, b) in &spans {
                let (a, b) = (a.max(x0), b.min(x1));
                if a < b {
                    row[(a - x0) as usize..(b - x0) as usize].fill(IN);
                }
            }
        })
    }

    /// The columns of the map where `filter` lets a brush act, weighed as
    /// it weighs them: "select by" a terrain, a height, a slope…
    pub fn by_filter(dim: &Dimension, filter: &Filter) -> Self {
        let tiles: HashSet<TilePos> = dim.levels.iter().flat_map(|l| l.positions()).collect();
        // Squares of 4 × 4 tiles, as the filter works them out.
        let side = 4 * TILE_SIZE;
        let squares: HashSet<(i32, i32)> = tiles.iter().map(|p| ((p.x * TILE_SIZE).div_euclid(side), (p.z * TILE_SIZE).div_euclid(side))).collect();
        let parts: Vec<HashMap<TilePos, Cells>> = squares
            .into_par_iter()
            .map(|(sx, sz)| {
                let rect = BlockRect::new(sx * side, sz * side, (sx + 1) * side, (sz + 1) * side);
                let mask = filter.mask(dim, rect);
                rect.tiles()
                    .filter(|p| tiles.contains(p))
                    .filter_map(|pos| {
                        let (ox, oz) = pos.origin();
                        let mut w = vec![0u8; TILE_AREA];
                        for lz in 0..TILE_SIZE {
                            for lx in 0..TILE_SIZE {
                                w[local_index(ox + lx, oz + lz)] = (mask.get(ox + lx, oz + lz) * IN as f32).round() as u8;
                            }
                        }
                        Cells::of(w).map(|c| (pos, c))
                    })
                    .collect()
            })
            .collect();
        Self { tiles: parts.into_iter().flatten().collect() }
    }

    /// This selection changed by `other` as `op` says.
    pub fn combine(&self, other: &Selection, op: Combine) -> Self {
        if op == Combine::Replace {
            return other.clone();
        }
        let keys: HashSet<TilePos> = self.tiles.keys().chain(other.tiles.keys()).copied().collect();
        let tiles = keys
            .into_par_iter()
            .filter_map(|pos| op.tile(self.tiles.get(&pos), other.tiles.get(&pos)).map(|c| (pos, c)))
            .collect();
        Self { tiles }
    }

    /// Everything of the map (`domain`, its tiles) this selection leaves out.
    pub fn invert(&self, domain: impl IntoIterator<Item = TilePos>) -> Self {
        let tiles = domain
            .into_iter()
            .filter_map(|pos| {
                let cells = match self.tiles.get(&pos) {
                    None => Some(Cells::Full),
                    Some(Cells::Full) => None,
                    Some(Cells::Part(w)) => Cells::of(w.iter().map(|&v| IN - v).collect()),
                };
                cells.map(|c| (pos, c))
            })
            .collect();
        Self { tiles }
    }

    /// The selection with its edge softened: the weights fade over about
    /// `blocks` blocks on either side of it (at most [`MAX_FEATHER`]).
    pub fn feather(&self, blocks: i32) -> Self {
        let h = ((blocks.clamp(0, MAX_FEATHER) + 1) / 2) as usize;
        if h == 0 || self.is_empty() {
            return self.clone();
        }
        // Two box passes of half-width h reach 2h blocks around a column.
        let reach = 2 * h as i32;
        let mut candidates: HashSet<TilePos> = HashSet::new();
        for p in self.tiles.keys() {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    candidates.insert(p.offset(dx, dz));
                }
            }
        }
        let tiles = candidates
            .into_par_iter()
            .filter_map(|pos| {
                let around = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| pos.offset(dx, dz)));
                let near: Vec<Option<&Cells>> = around.map(|p| self.tiles.get(&p)).collect();
                if near.iter().all(|c| matches!(c, Some(Cells::Full))) {
                    return Some((pos, Cells::Full));
                }
                if near.iter().all(Option::is_none) {
                    return None;
                }
                let rect = pos.rect().expand(reach);
                let w = rect.width() as usize;
                let mut field: Vec<f32> = self.weights(rect).unwrap_or_default().into_iter().map(|v| v as f32).collect();
                let mut line = Vec::new();
                for _ in 0..2 {
                    for row in field.chunks_mut(w) {
                        box_line(row.iter_mut(), h, &mut line);
                    }
                    for x in 0..w {
                        box_line(field[x..].iter_mut().step_by(w), h, &mut line);
                    }
                }
                let (ox, oz) = pos.origin();
                let mut out = vec![0u8; TILE_AREA];
                for lz in 0..TILE_SIZE as usize {
                    let src = &field[(lz + reach as usize) * w + reach as usize..][..TILE_SIZE as usize];
                    for (lx, &v) in src.iter().enumerate() {
                        out[local_index(ox + lx as i32, oz + lz as i32)] = v.round().clamp(0.0, IN as f32) as u8;
                    }
                }
                Cells::of(out).map(|c| (pos, c))
            })
            .collect();
        Self { tiles }
    }

    /// The edges of the selection over `rect`, read every `step` blocks (a
    /// sample is in when its weight is at least [`HALF`]): segments between
    /// block corners, horizontal and vertical, joined where they run on.
    pub fn edges(&self, rect: BlockRect, step: i32) -> Vec<[(i32, i32); 2]> {
        let step = step.max(1);
        let (gx0, gz0) = (rect.x0.div_euclid(step) - 1, rect.z0.div_euclid(step) - 1);
        let (gx1, gz1) = (rect.x1.div_euclid(step) + 2, rect.z1.div_euclid(step) + 2);
        let n = (gx1 - gx0) as usize;
        let row_of = |gz: i32| -> Vec<bool> {
            let z = gz * step + step / 2;
            let mut bits = vec![false; n];
            let mut i = 0;
            while i < n {
                let x = (gx0 + i as i32) * step + step / 2;
                let pos = TilePos::containing(x, z);
                // The samples of this row in the same tile.
                let end_x = pos.origin().0 + TILE_SIZE;
                let mut j = i;
                while j < n && (gx0 + j as i32) * step + step / 2 < end_x {
                    j += 1;
                }
                match self.tiles.get(&pos) {
                    None => {}
                    Some(Cells::Full) => bits[i..j].fill(true),
                    Some(Cells::Part(w)) => {
                        for (k, b) in bits[i..j].iter_mut().enumerate() {
                            *b = w[local_index((gx0 + (i + k) as i32) * step + step / 2, z)] >= HALF;
                        }
                    }
                }
                i = j;
            }
            bits
        };
        let mut out = Vec::new();
        let mut above = row_of(gz0);
        // The vertical edges reaching the row being read, by their x.
        let mut open: HashMap<i32, usize> = HashMap::new();
        for gz in gz0 + 1..gz1 {
            let row = row_of(gz);
            let z = gz * step;
            // Horizontal edges between the row above and this one.
            let mut start = None;
            for i in 0..=n {
                let edge = i < n && row[i] != above[i];
                match (edge, start) {
                    (true, None) => start = Some(i),
                    (false, Some(s)) => {
                        out.push([((gx0 + s as i32) * step, z), ((gx0 + i as i32) * step, z)]);
                        start = None;
                    }
                    _ => {}
                }
            }
            // Vertical edges inside this row, run on from the row above.
            let mut next = HashMap::new();
            for i in 1..n {
                if row[i] != row[i - 1] {
                    let x = (gx0 + i as i32) * step;
                    let at = match open.get(&x) {
                        Some(&at) => {
                            out[at][1].1 = z + step;
                            at
                        }
                        None => {
                            out.push([(x, z), (x, z + step)]);
                            out.len() - 1
                        }
                    };
                    next.insert(x, at);
                }
            }
            open = next;
            above = row;
        }
        out
    }

    /// The selection as bytes, for the file.
    pub fn encode(&self) -> Vec<u8> {
        let mut positions: Vec<&TilePos> = self.tiles.keys().collect();
        positions.sort();
        let mut raw = Vec::with_capacity(8 + positions.len() * 9);
        raw.extend_from_slice(b"SEL1");
        raw.extend_from_slice(&(positions.len() as u32).to_le_bytes());
        for pos in positions {
            raw.extend_from_slice(&pos.x.to_le_bytes());
            raw.extend_from_slice(&pos.z.to_le_bytes());
            match &self.tiles[pos] {
                Cells::Full => raw.push(0),
                Cells::Part(w) => {
                    raw.push(1);
                    raw.extend_from_slice(w);
                }
            }
        }
        crate::codec::zstd_compress(&raw)
    }

    /// A selection from [`Selection::encode`]'s bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let raw = crate::codec::zstd_decompress_section(bytes).map_err(|e| format!("the selection: {e}"))?;
        let bad = || "the selection is damaged".to_owned();
        if raw.len() < 8 || &raw[..4] != b"SEL1" {
            return Err(bad());
        }
        let count = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let mut at = 8;
        let mut tiles = HashMap::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let head = raw.get(at..at + 9).ok_or_else(bad)?;
            let pos = TilePos::new(i32::from_le_bytes(head[0..4].try_into().unwrap()), i32::from_le_bytes(head[4..8].try_into().unwrap()));
            at += 9;
            let cells = match head[8] {
                0 => Cells::Full,
                1 => {
                    let w = raw.get(at..at + TILE_AREA).ok_or_else(bad)?;
                    at += TILE_AREA;
                    Cells::Part(w.into())
                }
                _ => return Err(bad()),
            };
            tiles.insert(pos, cells);
        }
        Ok(Self { tiles })
    }
}

/// A running box average of half-width `h` along a line of values, the
/// missing values past its ends counted as 0.
fn box_line<'a>(values: impl Iterator<Item = &'a mut f32>, h: usize, line: &mut Vec<f32>) {
    let mut cells: Vec<&mut f32> = values.collect();
    line.clear();
    line.extend(cells.iter().map(|v| **v));
    let n = line.len();
    let k = (2 * h + 1) as f32;
    let mut sum: f32 = line[..h.min(n)].iter().sum();
    for (i, cell) in cells.iter_mut().enumerate() {
        if i + h < n {
            sum += line[i + h];
        }
        if i > h {
            sum -= line[i - h - 1];
        }
        **cell = sum / k;
    }
}

/// Whether a paint that sets a value (a terrain, a biome) sets it at
/// `(x, z)` with weight `w`: always inside, never outside, at random on a
/// feathered edge, as many columns as the weight says.
#[inline]
pub fn lets(w: u8, x: i32, z: i32) -> bool {
    w == IN || (w > 0 && ((crate::compose::hash2(x, z, 0x005e_1ec7) >> 24) as u8) < w)
}

/// [`Cells::to_vec`] for tests and tools that want every weight.
pub fn tile_weights(cells: Option<&Cells>) -> Vec<u8> {
    cells.map_or_else(|| vec![0; TILE_AREA], Cells::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> Selection {
        Selection::shape(&Shape::Rect(BlockRect::new(x0, z0, x1, z1)))
    }

    #[test]
    fn a_rectangle_selects_its_columns() {
        let s = rect(-10, 5, 300, 20);
        assert_eq!(s.get(-10, 5), IN);
        assert_eq!(s.get(299, 19), IN);
        assert_eq!(s.get(300, 19), 0);
        assert_eq!(s.get(-11, 10), 0);
        assert_eq!(s.area(), 310.0 * 15.0);
        // A big one is mostly whole tiles.
        let big = rect(0, 0, 1024, 1024);
        assert!(big.tiles.values().all(|c| *c == Cells::Full));
        assert_eq!(big.tiles.len(), 64);
    }

    #[test]
    fn an_ellipse_and_a_lasso_select_what_they_enclose() {
        let e = Selection::shape(&Shape::Ellipse(BlockRect::new(0, 0, 100, 50)));
        assert_eq!(e.get(50, 25), IN);
        assert_eq!(e.get(1, 1), 0);
        assert_eq!(e.get(98, 25), IN);
        let area = std::f64::consts::PI * 50.0 * 25.0;
        assert!((e.area() - area).abs() / area < 0.02, "{} vs {area}", e.area());
        // A triangle drawn with the lasso.
        let t = Selection::shape(&Shape::Polygon(vec![(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)]));
        assert_eq!(t.get(10, 10), IN);
        assert_eq!(t.get(90, 90), 0);
        assert!((t.area() - 5000.0).abs() < 120.0, "{}", t.area());
    }

    #[test]
    fn shapes_combine() {
        let a = rect(0, 0, 200, 200);
        let b = rect(100, 100, 300, 300);
        let add = a.combine(&b, Combine::Add);
        assert_eq!(add.area(), 2.0 * 40_000.0 - 10_000.0);
        let sub = a.combine(&b, Combine::Subtract);
        assert_eq!(sub.area(), 30_000.0);
        assert_eq!(sub.get(150, 150), 0);
        let both = a.combine(&b, Combine::Intersect);
        assert_eq!(both.area(), 10_000.0);
        assert_eq!(a.combine(&b, Combine::Replace), b);
        // Taking all of it out leaves nothing.
        assert!(a.combine(&a, Combine::Subtract).is_empty());
    }

    #[test]
    fn inverting_keeps_to_the_map() {
        let a = rect(10, 10, 20, 20);
        let map = [TilePos::new(0, 0), TilePos::new(1, 0)];
        let inv = a.invert(map);
        assert_eq!(inv.get(15, 15), 0);
        assert_eq!(inv.get(5, 5), IN);
        assert_eq!(inv.get(200, 5), IN);
        assert_eq!(inv.get(-5, 5), 0);
        assert_eq!(inv.area(), 2.0 * TILE_AREA as f64 - 100.0);
        assert_eq!(inv.invert(map), a);
    }

    #[test]
    fn feathering_fades_the_edge() {
        let a = rect(0, 0, 400, 400);
        let f = a.feather(16);
        // Deep inside and far outside, as they were.
        assert_eq!(f.get(200, 200), IN);
        assert_eq!(f.get(-40, 200), 0);
        // About half on the edge, fading across it.
        let edge = f.get(0, 200);
        assert!((100..=160).contains(&edge), "{edge}");
        let mut last = 0;
        for x in -20..20 {
            let v = f.get(x, 200);
            assert!(v >= last, "{x}: {v} after {last}");
            last = v;
        }
        assert_eq!(f.get(-17, 200), 0);
        assert_eq!(f.get(17, 200), IN);
        // About as much selected as before.
        assert!((f.area() - a.area()).abs() / a.area() < 0.01);
        // The tiles deep inside stay whole.
        assert_eq!(f.tile(TilePos::new(1, 1)), Some(&Cells::Full));
    }

    #[test]
    fn edges_go_round_the_selection() {
        let a = rect(10, 10, 20, 30);
        let edges = a.edges(BlockRect::new(0, 0, 64, 64), 1);
        let length: i32 = edges.iter().map(|[p, q]| (q.0 - p.0).abs() + (q.1 - p.1).abs()).sum();
        assert_eq!(length, 2 * (10 + 20));
        assert_eq!(edges.len(), 4, "{edges:?}");
        // Read coarsely, about the same outline.
        let coarse = a.edges(BlockRect::new(0, 0, 64, 64), 4);
        let length: i32 = coarse.iter().map(|[p, q]| (q.0 - p.0).abs() + (q.1 - p.1).abs()).sum();
        assert!((40..=80).contains(&length), "{length}");
    }

    #[test]
    fn a_selection_round_trips_as_bytes() {
        let s = rect(0, 0, 300, 200).feather(8).combine(&Selection::shape(&Shape::Ellipse(BlockRect::new(-50, -50, 50, 50))), Combine::Add);
        assert_eq!(Selection::decode(&s.encode()).unwrap(), s);
        assert!(Selection::decode(b"nonsense").is_err());
    }
}
