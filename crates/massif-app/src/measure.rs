//! The Measure tool: a line, or a path of points, across the map tells its
//! length along the map and along the ground, the heights it goes between
//! and its slopes, with the land's profile under it; an area closed around
//! a part of the map tells its size, its perimeter, its heights, the
//! blocks over and under a level (what levelling it would take away or
//! bring), and what covers it: terrains, biomes, layers.
//!
//! A block is a metre, as in the game. Measures are never saved with the
//! map: Esc or a right-click clears one, and one is shown only while the
//! tool is in hand unless it is kept on screen (pinned).
//!
//! The map and the 3D views draw it, Tool Settings holds its options, the
//! Info panel shows its results (see `inspect`).

use crate::doc::Document;
use crate::map_render::{AutoBiome, ColumnSampler};
use crate::theme::{self, Icon, Palette};
use crate::tools::ToolState;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::TilePos;
use massif_core::compose::{TileStack, block_top};
use massif_core::coords::local_index;
use massif_core::tile::NO_FLUID;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// What the tool draws: a line (or a path of points), or an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Line,
    Area,
}

/// The level an area's volume is counted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Base {
    /// The lowest ground of the area: what is over it, the blocks to take
    /// away to level the area down.
    #[default]
    Lowest,
    /// The water level of the map.
    Sea,
    /// A height given.
    Height,
}

/// Points of the screen the pointer moves at most for a press and release
/// to be a click.
const CLICK_TRAVEL: f32 = 4.0;

/// A press of the button on the map, until it is let go.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    /// How far the pointer went from where it was pressed, in points.
    travel: f32,
    /// The press began a new measure: dragged, it draws a line or an area
    /// in one go.
    new: bool,
}

/// The measure on the map: its points, in blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Measure {
    pub points: Vec<(f32, f32)>,
    /// Closed: an area.
    pub closed: bool,
    /// Finished; else the pointer is its next point.
    pub done: bool,
    /// Where the pointer is while it is being drawn.
    pub live: Option<(f32, f32)>,
    /// Kept on screen whatever the tool, until cleared.
    pub pinned: bool,
    press: Option<Press>,
}

impl Measure {
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Being drawn: the pointer adds its next point.
    pub fn drawing(&self) -> bool {
        !self.points.is_empty() && !self.done
    }

    pub fn clear(&mut self) {
        let pinned = self.pinned;
        *self = Self { pinned, ..Self::default() };
    }

    /// Its points, and while it is drawn the pointer's.
    pub fn path(&self) -> Vec<(f32, f32)> {
        let mut path = self.points.clone();
        if !self.done
            && let Some(at) = self.live
            && path.last() != Some(&at)
        {
            path.push(at);
        }
        path
    }

    /// The button pressed at `at`. `near_first`: on its first point (an
    /// area being drawn closes there).
    pub fn press(&mut self, at: (f32, f32), kind: Kind, near_first: bool) {
        if kind == Kind::Area && self.drawing() && near_first && self.points.len() >= 3 {
            self.closed = true;
            self.done = true;
            self.live = None;
            self.press = None;
            return;
        }
        let new = self.done || self.points.is_empty();
        if new {
            self.points = vec![at];
            self.closed = false;
            self.done = false;
        } else if self.points.last() != Some(&at) {
            self.points.push(at);
        }
        self.live = Some(at);
        self.press = Some(Press { travel: 0.0, new });
    }

    /// The button held, the pointer at `at`; `travel`: how far from where
    /// it was pressed, in points; `zoom`: the points a block takes.
    pub fn drag(&mut self, at: (f32, f32), travel: f32, zoom: f32, kind: Kind) {
        let Some(press) = &mut self.press else { return };
        press.travel = press.travel.max(travel);
        self.live = Some(at);
        // An area dragged: a lasso, a point every few points of the screen.
        if press.new && press.travel >= CLICK_TRAVEL && kind == Kind::Area {
            let last = *self.points.last().expect("pressed");
            if (at.0 - last.0).hypot(at.1 - last.1) * zoom >= 3.0 {
                self.points.push(at);
            }
        }
    }

    /// Let go at `at`. Dragged from a new start, the line (or the area)
    /// is done; a click goes on to the next point.
    pub fn release(&mut self, at: (f32, f32), kind: Kind) {
        let Some(press) = self.press.take() else { return };
        if press.new && press.travel >= CLICK_TRAVEL {
            match kind {
                Kind::Line => {
                    self.points.truncate(1);
                    self.points.push(at);
                    self.done = true;
                }
                Kind::Area if self.points.len() >= 3 => {
                    self.closed = true;
                    self.done = true;
                }
                Kind::Area => self.clear(),
            }
            self.live = None;
        }
    }

    /// Finished where it is (Enter, a double-click): an area closed, a
    /// line ended; too few points, cleared.
    pub fn finish(&mut self, kind: Kind) {
        self.press = None;
        self.points.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 0.5);
        match kind {
            Kind::Area if self.points.len() >= 3 => self.closed = true,
            _ if self.points.len() >= 2 => self.closed = false,
            _ => return self.clear(),
        }
        self.done = true;
        self.live = None;
    }

    /// The last point taken back (Backspace); none left, cleared.
    pub fn undo_point(&mut self) {
        self.points.pop();
        self.done = false;
        self.closed = false;
        if self.points.is_empty() {
            self.clear();
        }
    }

    /// The kind changed in Tool Settings: a finished path closes into an
    /// area, an area opens into a path.
    pub fn set_kind(&mut self, kind: Kind) {
        if self.done {
            self.closed = kind == Kind::Area && self.points.len() >= 3;
        }
    }
}

/// Whether the measure shows: with the tool in hand, or kept on screen.
pub fn shown(tools: &ToolState) -> bool {
    !tools.measure.is_empty() && (tools.tool == crate::tools::Tool::Measure || tools.measure.pinned)
}

// ---------------------------------------------------------------- ground --

/// One column as a measure reads it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Col {
    /// The ground's height (the top of the block one stands on is
    /// `block_top(height)`).
    pub height: f32,
    /// The water's surface over it.
    pub water: Option<i32>,
    pub terrain: u16,
    /// The biome painted, or the one the land calls for when `auto`.
    pub biome: u16,
    pub auto: bool,
    pub cover: u16,
    pub trees: u16,
    pub under: u16,
    pub pockets: u16,
    /// Annotations, the first four.
    pub notes: [u16; 4],
}

/// What a measure reads its columns from: the map, or a terrain made up
/// for the tests.
pub trait Ground {
    /// The column at `(x, z)`, `None` off the map or where it is empty.
    /// `full`: its terrain, biome and layers too, else its heights alone.
    fn col(&mut self, x: i32, z: i32, full: bool) -> Option<Col>;
    fn sea_level(&self) -> i32;
}

/// The map's columns, a tile's stack read once.
pub struct DocGround<'a> {
    doc: &'a Document,
    stacks: HashMap<TilePos, Option<TileStack<'a>>>,
    sampler: ColumnSampler,
    auto: AutoBiome,
}

impl<'a> DocGround<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self { doc, stacks: HashMap::new(), sampler: ColumnSampler::default(), auto: AutoBiome::new(&doc.world.registry) }
    }
}

impl Ground for DocGround<'_> {
    fn col(&mut self, x: i32, z: i32, full: bool) -> Option<Col> {
        let dim = self.doc.dimension();
        let pos = TilePos::containing(x, z);
        let stack = self.stacks.entry(pos).or_insert_with(|| TileStack::new(dim, pos, false)).as_ref()?;
        let cell = local_index(x, z);
        let s = self.sampler.sample(stack, cell);
        if s.is_empty() {
            return None;
        }
        let water = (s.fluid != NO_FLUID).then_some(s.fluid as i32);
        if !full {
            return Some(Col { height: s.height, water, ..Col::default() });
        }
        let reg = &self.doc.world.registry;
        let auto = s.biome == 0;
        let biome = if auto { self.auto.resolve(reg, &s, self.doc.world.meta.sea_level as f32) } else { s.biome };
        let mut notes = [0; 4];
        for (slot, n) in notes.iter_mut().zip(stack.ground().notes(cell).filter(|&n| !reg.annotations.is_removed(n))) {
            *slot = n;
        }
        Some(Col { height: s.height, water, terrain: s.terrain, biome, auto, cover: s.cover, trees: s.objects, under: s.understory, pockets: s.pockets, notes })
    }

    fn sea_level(&self) -> i32 {
        self.doc.world.meta.sea_level
    }
}

// ---------------------------------------------------------------- reports --

/// One point of the profile: how far along, the ground's height and the
/// water's surface there (`None` off the map).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfilePoint {
    pub along: f32,
    pub ground: Option<f32>,
    pub water: Option<f32>,
}

/// What a line (or the path around an area) measures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LineReport {
    /// Its length on the map, in blocks.
    pub horizontal: f64,
    /// Its length along the ground, up and down the slopes.
    pub ground: f64,
    /// The ground's height (the block one stands on) at its two ends.
    pub from: Option<i32>,
    pub to: Option<i32>,
    pub lowest: Option<i32>,
    pub highest: Option<i32>,
    /// Blocks climbed and come down along it.
    pub climb: f64,
    pub descent: f64,
    /// The mean slope (blocks up or down over blocks along) and the
    /// steepest, over a few blocks.
    pub mean_slope: f64,
    pub max_slope: f64,
    pub profile: Vec<ProfilePoint>,
    /// The points it goes through.
    pub points: usize,
}

/// A share of an area: a terrain, a biome or a layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Share {
    /// What kind of layer, for the layers ("Cover", "Trees"…).
    pub kind: &'static str,
    pub name: String,
    pub color: [u8; 3],
    /// Of the area's columns, 0 to 1.
    pub share: f32,
    /// A biome the land calls for, not painted.
    pub auto: bool,
}

/// What an area measures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AreaReport {
    /// Its columns on the map: its area in blocks², square metres.
    pub columns: f64,
    pub perimeter: f64,
    pub lowest: i32,
    pub highest: i32,
    pub mean: f64,
    /// The level the volume is counted from, and the blocks of ground
    /// over it (to take away) and the room under it (to fill).
    pub base: i32,
    pub above: f64,
    pub below: f64,
    /// The share of it under water.
    pub water: f32,
    pub terrains: Vec<Share>,
    pub biomes: Vec<Share>,
    pub layers: Vec<Share>,
    /// Every `stride`th column read, each standing for `stride²` (on an
    /// area of millions of columns).
    pub stride: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub line: LineReport,
    pub area: Option<AreaReport>,
}

/// The most points read along a line: past it, further apart than a block.
const MAX_SAMPLES: f64 = 8192.0;
/// The most columns read of an area: past it, one every few.
const MAX_COLUMNS: f64 = 1_000_000.0;
/// The run over which the steepest slope is taken, in blocks: a single
/// block's step is not a slope.
const SLOPE_RUN: f64 = 4.0;
/// The points the profile keeps at most.
const PROFILE_POINTS: usize = 400;

/// The line through `path` (closed back to its start when `closed`).
pub fn line(ground: &mut dyn Ground, path: &[(f32, f32)], closed: bool) -> LineReport {
    let mut legs: Vec<((f64, f64), (f64, f64))> = path.windows(2).map(|w| ((w[0].0 as f64, w[0].1 as f64), (w[1].0 as f64, w[1].1 as f64))).collect();
    if closed && path.len() >= 3 {
        let (a, b) = (path[path.len() - 1], path[0]);
        legs.push(((a.0 as f64, a.1 as f64), (b.0 as f64, b.1 as f64)));
    }
    let horizontal: f64 = legs.iter().map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1)).sum();
    let step = (horizontal / MAX_SAMPLES).max(1.0);
    // The ground every `step` blocks along the way, its ends included.
    let mut samples: Vec<(f64, Option<Col>)> = Vec::new();
    let mut along = 0.0;
    let at = |ground: &mut dyn Ground, x: f64, z: f64| ground.col(x.floor() as i32, z.floor() as i32, false);
    if let Some(&(x, z)) = path.first() {
        samples.push((0.0, at(ground, x as f64, z as f64)));
    }
    for (a, b) in &legs {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (len / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            samples.push((along + len * t, at(ground, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)));
        }
        along += len;
    }
    let mut r = LineReport { horizontal, points: path.len(), ..LineReport::default() };
    r.from = samples.first().and_then(|s| s.1).map(|c| block_top(c.height));
    r.to = samples.last().and_then(|s| s.1).map(|c| block_top(c.height));
    let mut ground_len = 0.0;
    let mut covered = 0.0;
    for w in samples.windows(2) {
        let ds = w[1].0 - w[0].0;
        match (w[0].1, w[1].1) {
            (Some(a), Some(b)) => {
                let dh = (b.height - a.height) as f64;
                ground_len += ds.hypot(dh);
                covered += ds;
                if dh > 0.0 {
                    r.climb += dh;
                } else {
                    r.descent -= dh;
                }
            }
            // Off the map: along the map alone.
            _ => ground_len += ds,
        }
    }
    r.ground = ground_len;
    r.mean_slope = if covered > 0.0 { (r.climb + r.descent) / covered } else { 0.0 };
    // The steepest: over SLOPE_RUN blocks, from each point.
    let run = (SLOPE_RUN / step).round().max(1.0) as usize;
    for i in 0..samples.len().saturating_sub(run) {
        if let ((d0, Some(a)), (d1, Some(b))) = (samples[i], samples[i + run]) {
            let ds = d1 - d0;
            if ds > 0.0 {
                r.max_slope = r.max_slope.max(((b.height - a.height) as f64).abs() / ds);
            }
        }
    }
    // A line shorter than the run: its ends.
    if samples.len() <= run
        && let (Some((d0, Some(a))), Some((d1, Some(b)))) = (samples.first().copied(), samples.last().copied())
        && d1 > d0
    {
        r.max_slope = ((b.height - a.height) as f64).abs() / (d1 - d0);
    }
    let tops = samples.iter().filter_map(|s| s.1).map(|c| block_top(c.height));
    r.lowest = tops.clone().min();
    r.highest = tops.max();
    // The profile, thinned to as many points as the graph can show: each
    // kept point the highest of those it stands for, so that no summit is
    // lost.
    let per = samples.len().div_ceil(PROFILE_POINTS).max(1);
    r.profile = samples
        .chunks(per)
        .map(|c| {
            let top = c.iter().filter(|s| s.1.is_some()).max_by(|a, b| a.1.unwrap().height.total_cmp(&b.1.unwrap().height)).copied().unwrap_or(c[0]);
            ProfilePoint { along: top.0 as f32, ground: top.1.map(|c| c.height), water: top.1.and_then(|c| c.water.filter(|&w| w as f32 > c.height)).map(|w| w as f32) }
        })
        .collect();
    r
}

/// Whether the centre of column `(x, z)` is inside `poly` (even-odd).
#[cfg(test)]
fn inside(poly: &[(f32, f32)], x: f64, z: f64) -> bool {
    let mut on = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, zi) = (poly[i].0 as f64, poly[i].1 as f64);
        let (xj, zj) = (poly[j].0 as f64, poly[j].1 as f64);
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            on = !on;
        }
        j = i;
    }
    on
}

/// The columns inside `poly`, a row at a time: `(z, x0, x1)` runs, every
/// `stride`th column of every `stride`th row.
fn runs(poly: &[(f32, f32)], stride: i32, mut each: impl FnMut(i32, i32)) {
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, z) in poly {
        x0 = x0.min(x);
        z0 = z0.min(z);
        x1 = x1.max(x);
        z1 = z1.max(z);
    }
    let (zs, ze) = (z0.floor() as i32, z1.ceil() as i32);
    let mut z = zs;
    while z < ze {
        let zc = z as f64 + 0.5;
        // Where the row crosses the edges, in order: between each pair it
        // is inside.
        let mut cross: Vec<f64> = Vec::new();
        let mut j = poly.len() - 1;
        for i in 0..poly.len() {
            let (xi, zi) = (poly[i].0 as f64, poly[i].1 as f64);
            let (xj, zj) = (poly[j].0 as f64, poly[j].1 as f64);
            if (zi > zc) != (zj > zc) {
                cross.push((xj - xi) * (zc - zi) / (zj - zi) + xi);
            }
            j = i;
        }
        cross.sort_by(f64::total_cmp);
        for pair in cross.chunks(2) {
            if let [a, b] = pair {
                // The columns whose centre is between the two.
                let first = (a - 0.5).ceil() as i32;
                let last = (b - 0.5).ceil() as i32 - 1;
                let mut x = first + (x0.floor() as i32 - first).rem_euclid(stride);
                while x <= last {
                    each(x, z);
                    x += stride;
                }
            }
        }
        z += stride;
    }
}

/// What the area inside `poly` measures, its volume counted from `base`
/// (`level` for [`Base::Height`]).
pub fn area(ground: &mut dyn Ground, poly: &[(f32, f32)], base: Base, level: i32, full: bool) -> Option<AreaReport> {
    if poly.len() < 3 {
        return None;
    }
    let shoelace = poly.iter().zip(poly.iter().cycle().skip(1)).map(|(a, b)| a.0 as f64 * b.1 as f64 - b.0 as f64 * a.1 as f64).sum::<f64>().abs() / 2.0;
    let stride = (shoelace / MAX_COLUMNS).sqrt().ceil().max(1.0) as i32;
    let weight = (stride * stride) as f64;
    let mut tops: Vec<i32> = Vec::new();
    let mut wet = 0.0;
    let mut terrains: HashMap<u16, f64> = HashMap::new();
    let mut biomes: HashMap<(u16, bool), f64> = HashMap::new();
    let mut layers: HashMap<(u8, u16), f64> = HashMap::new();
    runs(poly, stride, |x, z| {
        let Some(c) = ground.col(x, z, full) else { return };
        let top = block_top(c.height);
        tops.push(top);
        if c.water.is_some_and(|w| w > top) {
            wet += weight;
        }
        if full {
            *terrains.entry(c.terrain).or_default() += weight;
            *biomes.entry((c.biome, c.auto)).or_default() += weight;
            for (k, n) in [(0u8, c.cover), (1, c.trees), (2, c.under), (3, c.pockets)] {
                if n != 0 {
                    *layers.entry((k, n)).or_default() += weight;
                }
            }
            for n in c.notes.into_iter().filter(|&n| n != 0) {
                *layers.entry((4, n)).or_default() += weight;
            }
        }
    });
    if tops.is_empty() {
        return None;
    }
    let columns = tops.len() as f64 * weight;
    let lowest = *tops.iter().min().unwrap();
    let highest = *tops.iter().max().unwrap();
    let mean = tops.iter().map(|&t| t as f64).sum::<f64>() / tops.len() as f64;
    let base = match base {
        Base::Lowest => lowest,
        Base::Sea => ground.sea_level(),
        Base::Height => level,
    };
    let above = tops.iter().map(|&t| (t - base).max(0) as f64).sum::<f64>() * weight;
    let below = tops.iter().map(|&t| (base - t).max(0) as f64).sum::<f64>() * weight;
    let perimeter = poly.iter().zip(poly.iter().cycle().skip(1)).map(|(a, b)| ((b.0 - a.0) as f64).hypot((b.1 - a.1) as f64)).sum();
    let share = |n: f64| (n / columns) as f32;
    let sorted = |mut v: Vec<Share>| {
        v.sort_by(|a, b| b.share.total_cmp(&a.share).then_with(|| a.name.cmp(&b.name)));
        v
    };
    let blank = |n: f64, auto: bool| Share { kind: "", name: String::new(), color: [0; 3], share: share(n), auto };
    let mut r = AreaReport { columns, perimeter, lowest, highest, mean, base, above, below, water: share(wet), stride, ..AreaReport::default() };
    r.terrains = sorted(terrains.into_iter().map(|(t, n)| blank(n, false).named(Named::Terrain(t))).collect());
    r.biomes = sorted(biomes.into_iter().map(|((b, auto), n)| blank(n, auto).named(Named::Biome(b))).collect());
    r.layers = sorted(layers.into_iter().map(|((k, l), n)| blank(n, false).named(Named::Layer(k, l))).collect());
    Some(r)
}

/// What a share is of, before it is named from the registry.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Named {
    Terrain(u16),
    Biome(u16),
    /// Cover, trees, under the trees, pockets, annotation; and which.
    Layer(u8, u16),
}

impl Share {
    /// Named later by [`name_shares`]: what it is, kept in its name.
    fn named(mut self, what: Named) -> Self {
        let (kind, id) = match what {
            Named::Terrain(t) => (0, t as u32),
            Named::Biome(b) => (1, b as u32),
            Named::Layer(k, l) => (2 + k as u32, l as u32),
        };
        self.name = format!("\u{0}{kind}:{id}");
        self
    }
}

/// The shares named and coloured as the registry has them.
fn name_shares(r: &mut AreaReport, reg: &massif_core::registry::Registry) {
    for s in r.terrains.iter_mut().chain(r.biomes.iter_mut()).chain(r.layers.iter_mut()) {
        let Some(key) = s.name.strip_prefix('\u{0}') else { continue };
        let (kind, id) = key.split_once(':').map(|(k, i)| (k.parse::<u32>().unwrap_or(0), i.parse::<u16>().unwrap_or(0))).unwrap_or_default();
        let (k, name, color): (&'static str, String, [u8; 3]) = match kind {
            0 => {
                let t = reg.terrain(massif_core::registry::TerrainId(id));
                ("", crate::i18n::tr_name(&t.name).to_owned(), t.color)
            }
            1 => {
                let b = reg.biome(massif_core::registry::BiomeId(id));
                ("", b.name.clone(), b.color)
            }
            2 => reg.cover(id).map_or((tr!("Cover"), String::new(), [128; 3]), |c| (tr!("Cover"), c.name.clone(), c.color)),
            3 => reg.object_layer(id).map_or((tr!("Trees"), String::new(), [128; 3]), |l| (tr!("Trees"), l.name.clone(), l.color)),
            4 => reg.object_layer(id).map_or((tr!("Under the trees"), String::new(), [128; 3]), |l| (tr!("Under the trees"), l.name.clone(), l.color)),
            5 => reg.pocket(id).map_or((tr!("Pockets"), String::new(), [128; 3]), |p| (tr!("Pockets"), p.name.clone(), p.color)),
            _ => {
                let kind = if reg.annotations.is_layer(id) { tr!("Annotation layer") } else { tr!("Annotation") };
                (kind, crate::panels::annotation_label(id, &reg.annotations), reg.annotations.color(id).unwrap_or([128; 3]))
            }
        };
        s.kind = k;
        s.name = name;
        s.color = color;
    }
}

/// The measure's report, read from the map.
pub fn compute(doc: &Document, tools: &ToolState) -> Option<Report> {
    let m = &tools.measure;
    let path = m.path();
    if path.is_empty() {
        return None;
    }
    let mut ground = DocGround::new(doc);
    let closed = m.closed && m.done;
    let line = line(&mut ground, &path, closed);
    let mut area = if closed { area(&mut ground, &path, tools.measure_base, tools.measure_level, true) } else { None };
    if let Some(a) = &mut area {
        name_shares(a, &doc.world.registry);
    }
    Some(Report { line, area })
}

/// The measure's report, worked out again only when it or the map changes
/// (the map's edits at most twice a second): kept between frames, and
/// between the views that ask for it.
pub fn report(ctx: &egui::Context, doc: &Document, tools: &ToolState) -> Option<Arc<Report>> {
    use std::hash::{Hash, Hasher};
    if tools.measure.is_empty() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in tools.measure.path() {
        p.0.to_bits().hash(&mut h);
        p.1.to_bits().hash(&mut h);
    }
    (tools.measure.closed, tools.measure.done, tools.measure_base, tools.measure_level, doc.generation, doc.dim).hash(&mut h);
    let key = h.finish();
    let id = egui::Id::unique("measure report");
    let now = ctx.input(|i| i.time);
    let kept: Option<(u64, u64, f64, Arc<Report>)> = ctx.data(|d| d.get_temp(id));
    if let Some((k, rev, at, r)) = &kept
        && *k == key
        && (*rev == doc.revision || now - at < 0.5)
    {
        if *rev != doc.revision {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(0.5 - (now - at)));
        }
        return Some(r.clone());
    }
    let r = Arc::new(compute(doc, tools)?);
    ctx.data_mut(|d| d.insert_temp(id, (key, doc.revision, now, r.clone())));
    Some(r)
}

// ------------------------------------------------------------------ words --

/// A length in blocks (metres), in kilometres past a thousand.
pub fn length(v: f64) -> String {
    if v >= 1000.0 {
        tr!("{value:.2} km", value = v / 1000.0)
    } else if v >= 100.0 {
        tr!("{value:.0} m", value = v)
    } else {
        tr!("{value:.1} m", value = v)
    }
}

/// An area in blocks² (square metres), in km² past a million.
pub fn surface(v: f64) -> String {
    if v >= 1_000_000.0 { tr!("{value:.2} km²", value = v / 1_000_000.0) } else { tr!("{thousands} m²", thousands = crate::select::thousands(v.round() as u64)) }
}

/// A volume in blocks.
pub fn volume(v: f64) -> String {
    tr!("{thousands} blocks", thousands = crate::select::thousands(v.round() as u64))
}

/// A slope as a grade and an angle: "12 % · 6.8°".
pub fn slope(ratio: f64) -> String {
    tr!("{grade:.0} % · {angle:.1}°", grade = ratio * 100.0, angle = ratio.atan().to_degrees())
}

/// A height difference, signed.
fn rise(d: i32) -> String {
    if d > 0 { format!("+{d}") } else { d.to_string() }
}

/// What the label on the map says.
fn label(r: &Report) -> String {
    match &r.area {
        Some(a) => format!("{} · {}", surface(a.columns), length(a.perimeter)),
        None => {
            let l = &r.line;
            let mut text = length(l.horizontal);
            if let (Some(a), Some(b)) = (l.from, l.to) {
                text = format!("{text} · {} · {:.0} %", rise(b - a), l.mean_slope * 100.0);
            }
            text
        }
    }
}

// --------------------------------------------------------------- drawing --

/// A label in a pill, its left middle at `at`.
fn pill(painter: &egui::Painter, pal: &Palette, at: Pos2, text: String) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), pal.text);
    let r = Rect::from_min_size(at - vec2(0.0, galley.size().y / 2.0 + 3.0), galley.size() + vec2(14.0, 6.0));
    let r = r.translate(vec2((painter.clip_rect().right() - 4.0 - r.right()).min(0.0), 0.0));
    painter.rect(r, theme::PILL, pal.panel.gamma_multiply(0.94), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    painter.galley(r.min + vec2(7.0, 3.0), galley, pal.text);
}

/// The point along `path` (closed when `closed`) at `along` blocks.
fn point_along(path: &[(f32, f32)], closed: bool, along: f32) -> Option<(f32, f32)> {
    let mut left = along;
    let n = path.len();
    let legs = if closed && n >= 3 { n } else { n.saturating_sub(1) };
    for i in 0..legs {
        let (a, b) = (path[i], path[(i + 1) % n]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if left <= len || i + 1 == legs {
            let t = if len > 0.0 { (left / len).clamp(0.0, 1.0) } else { 0.0 };
            return Some((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        left -= len;
    }
    path.first().copied()
}

/// Where the pointer is on the profile graph, in blocks along: the map
/// marks the spot.
fn probe_id() -> egui::Id {
    egui::Id::unique("measure probe")
}

/// The measure drawn over the map: its line, its points, the area it
/// closes, the label of what it measures. `to_screen` maps blocks to
/// points.
pub fn draw_map(ui: &Ui, painter: &egui::Painter, to_screen: &dyn Fn(f64, f64) -> Pos2, doc: &Document, tools: &ToolState) {
    if !shown(tools) {
        return;
    }
    let pal = Palette::of(ui);
    let m = &tools.measure;
    let path = m.path();
    let closed = m.closed && m.done;
    let pts: Vec<Pos2> = path.iter().map(|&(x, z)| to_screen(x as f64, z as f64)).collect();
    let under = Stroke::new(4.0, Color32::from_black_alpha(110));
    let line = Stroke::new(2.0, pal.accent);
    if closed {
        // The area, shaded: a fan from its first point, drawn even-odd by
        // its outline over a light fill of each run.
        let fill = pal.accent.gamma_multiply(0.16);
        let mut mesh = egui::Mesh::default();
        for (i, p) in pts.iter().enumerate() {
            mesh.colored_vertex(*p, fill);
            if i >= 2 {
                mesh.add_triangle(0, i as u32 - 1, i as u32);
            }
        }
        if is_convex(&pts) {
            painter.add(mesh);
        }
        painter.add(egui::Shape::closed_line(pts.clone(), under));
        painter.add(egui::Shape::closed_line(pts.clone(), line));
    } else if pts.len() > 1 {
        painter.add(egui::Shape::line(pts.clone(), under));
        painter.add(egui::Shape::line(pts.clone(), line));
        // An area being drawn: where it will close.
        if tools.measure_kind == Kind::Area && pts.len() > 2 {
            painter.add(egui::Shape::dashed_line(&[*pts.last().unwrap(), pts[0]], Stroke::new(1.0, pal.accent), 4.0, 4.0));
        }
    }
    // Its points (a lasso's are too many to mark).
    if m.points.len() <= 64 {
        for (i, p) in pts.iter().enumerate().take(m.points.len()) {
            let first_to_close = i == 0 && tools.measure_kind == Kind::Area && m.drawing() && m.points.len() >= 3;
            painter.circle(*p, if first_to_close { 5.5 } else { 3.5 }, pal.panel, Stroke::new(2.0, pal.accent));
        }
    }
    let Some(r) = report(ui.ctx(), doc, tools) else { return };
    // The spot pointed at on the profile.
    if let Some(along) = ui.ctx().data(|d| d.get_temp::<f32>(probe_id()))
        && let Some((x, z)) = point_along(&path, closed, along)
    {
        painter.circle(to_screen(x as f64, z as f64), 5.0, pal.accent, Stroke::new(2.0, Color32::WHITE));
    }
    let at = if closed {
        let n = pts.len() as f32;
        let c = pts.iter().fold(vec2(0.0, 0.0), |s, p| s + p.to_vec2()) / n;
        c.to_pos2() - vec2(30.0, 0.0)
    } else {
        *pts.last().unwrap() + vec2(12.0, -14.0)
    };
    pill(painter, &pal, at, label(&r));
}

/// Whether the polygon is convex (its fan shading then right).
fn is_convex(pts: &[Pos2]) -> bool {
    let n = pts.len();
    if n < 4 {
        return n == 3;
    }
    let mut sign = 0.0f32;
    for i in 0..n {
        let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        let cross = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
        if cross.abs() > 1e-3 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

/// The measure drawn in a 3D view, laid on the ground: `to_screen` maps a
/// point of the world (x, y, z, in blocks) to the view, `None` behind the
/// eyes.
pub fn draw_3d(ui: &Ui, painter: &egui::Painter, to_screen: &dyn Fn([f32; 3]) -> Option<Pos2>, doc: &Document, tools: &ToolState) {
    if !shown(tools) {
        return;
    }
    let pal = Palette::of(ui);
    let m = &tools.measure;
    let mut path = m.path();
    let closed = m.closed && m.done;
    if closed && let Some(&first) = path.first() {
        path.push(first);
    }
    let mut ground = DocGround::new(doc);
    let mut height = |x: f32, z: f32| ground.col(x.floor() as i32, z.floor() as i32, false).map(|c| block_top(c.height) as f32 + 0.15);
    let under = Stroke::new(4.0, Color32::from_black_alpha(110));
    let line = Stroke::new(2.0, pal.accent);
    // Each leg laid on the ground, a point every block or so (at most a
    // few hundred a leg).
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() > 1 {
            painter.add(egui::Shape::line(run.clone(), under));
            painter.add(egui::Shape::line(std::mem::take(run), line));
        }
        run.clear();
    };
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 400);
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (x, z) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            match height(x, z).and_then(|y| to_screen([x, y, z])) {
                Some(p) => run.push(p),
                None => flush(&mut run),
            }
        }
    }
    flush(&mut run);
    for &(x, z) in m.points.iter().take(64) {
        if let Some(p) = height(x, z).and_then(|y| to_screen([x, y, z])) {
            painter.circle(p, 3.5, pal.panel, Stroke::new(2.0, pal.accent));
        }
    }
    if let Some(r) = report(ui.ctx(), doc, tools)
        && let Some(&(x, z)) = m.path().last()
        && let Some(p) = height(x, z).and_then(|y| to_screen([x, y, z]))
    {
        pill(painter, &pal, p + vec2(12.0, -14.0), label(&r));
    }
}

// --------------------------------------------------------------- panels --

/// The tool's options, in Tool Settings.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &Document) {
    use crate::panels::hint;
    use crate::widgets::{Field, button_group, row, well};
    let kinds = [Kind::Line, Kind::Area];
    let labels = [tr!("Line"), tr!("Area")];
    if let Some(k) = button_group(ui, "measure kind", &labels, &kinds.map(|k| tools.measure_kind == k)) {
        tools.measure_kind = kinds[k];
        tools.measure.set_kind(kinds[k]);
    }
    ui.add_space(4.0);
    hint(
        ui,
        match tools.measure_kind {
            Kind::Line => tr!("Drag a line, or click from point to point and double-click (or Enter) to end. Esc or a right-click clears it."),
            Kind::Area => tr!("Drag around an area, or click its corners and click the first one again (or Enter) to close it. Esc or a right-click clears it."),
        },
    );
    ui.add_space(4.0);
    well(ui, |ui| {
        row(ui, tr!("Keep on screen"), |ui| {
            crate::widgets::switch(ui, &mut tools.measure.pinned).on_hover_text(tr!("The measure stays on the map and in the Info panel with the other tools, until cleared"));
        });
    });
    // An area's volume: the blocks over a level and the room under it.
    if tools.measure_kind == Kind::Area {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr!("Volume counted from")).size(11.5).color(Palette::of(ui).weak));
        ui.add_space(2.0);
        let bases = [Base::Lowest, Base::Sea, Base::Height];
        let labels = [tr!("Lowest ground"), tr!("Sea level"), tr!("A height")];
        if let Some(k) = button_group(ui, "measure base", &labels, &bases.map(|b| tools.measure_base == b)) {
            tools.measure_base = bases[k];
        }
        if tools.measure_base == Base::Height {
            ui.add_space(4.0);
            let dim = doc.dimension();
            well(ui, |ui| {
                row(ui, tr!("Height"), |ui| {
                    ui.add(Field::new(&mut tools.measure_level, dim.min_y..=dim.max_y).width(120.0));
                });
            });
        }
    }
    ui.add_space(4.0);
    let any = !tools.measure.is_empty();
    if ui.add_enabled(any, egui::Button::new(theme::with_icon(Icon::X, tr!("Clear")))).clicked() {
        tools.measure.clear();
    }
    if any {
        hint(ui, tr!("Its results are in the Info panel."));
    }
}

/// The measure's results, in the Info panel: what it measures, then the
/// profile of the land along a line, or what covers an area.
pub fn results(ui: &mut Ui, r: &Report, drawing: bool) {
    use crate::widgets::{row, well};
    let pal = Palette::of(ui);
    let mono = |ui: &mut Ui, text: String| {
        ui.label(egui::RichText::new(text).monospace());
    };
    let l = &r.line;
    match &r.area {
        Some(a) => {
            well(ui, |ui| {
                row(ui, tr!("Area"), |ui| mono(ui, surface(a.columns)));
                row(ui, tr!("Perimeter"), |ui| mono(ui, length(a.perimeter)));
                row(ui, tr!("Heights"), |ui| mono(ui, tr!("{lowest} to {highest}", lowest = a.lowest, highest = a.highest)));
                row(ui, tr!("Mean height"), |ui| mono(ui, format!("{:.1}", a.mean)));
                row(ui, &tr!("Over {base}", base = a.base), |ui| mono(ui, volume(a.above)));
                row(ui, &tr!("Under {base}", base = a.base), |ui| mono(ui, volume(a.below)));
                if a.water > 0.0 {
                    row(ui, tr!("Under water"), |ui| mono(ui, format!("{:.0} %", a.water * 100.0)));
                }
            });
            if a.stride > 1 {
                crate::panels::hint(ui, &tr!("A large area: one column in {n} read.", n = a.stride * a.stride));
            }
            for (title, shares) in [(tr!("Terrains"), &a.terrains), (tr!("Biomes"), &a.biomes), (tr!("Layers"), &a.layers)] {
                if shares.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(title).size(11.5).color(pal.weak));
                ui.add_space(2.0);
                well(ui, |ui| {
                    for s in shares.iter().take(8) {
                        shares_row(ui, s);
                    }
                    if shares.len() > 8 {
                        let rest: f32 = shares[8..].iter().map(|s| s.share).sum();
                        row(ui, &tr!("{n} more", n = shares.len() - 8), |ui| {
                            ui.label(egui::RichText::new(format!("{:.0} %", rest * 100.0)).monospace().color(pal.weak));
                        });
                    }
                });
            }
        }
        None => {
            well(ui, |ui| {
                row(ui, tr!("Distance"), |ui| mono(ui, length(l.horizontal)));
                row(ui, tr!("Along the ground"), |ui| mono(ui, length(l.ground)));
                if let (Some(a), Some(b)) = (l.from, l.to) {
                    row(ui, tr!("Height"), |ui| mono(ui, format!("{a} → {b} ({})", rise(b - a))));
                }
                if l.points > 2 && (l.climb > 0.0 || l.descent > 0.0) {
                    row(ui, tr!("Up and down"), |ui| mono(ui, format!("+{:.0} / −{:.0}", l.climb, l.descent)));
                }
                row(ui, tr!("Mean slope"), |ui| mono(ui, slope(l.mean_slope)));
                row(ui, tr!("Steepest"), |ui| mono(ui, slope(l.max_slope)));
                if l.points > 2 {
                    row(ui, tr!("Points"), |ui| mono(ui, l.points.to_string()));
                }
            });
            if l.profile.len() > 1 {
                ui.add_space(6.0);
                profile(ui, l);
            }
            if drawing {
                crate::panels::hint(ui, tr!("Being drawn: the pointer is its last point."));
            }
        }
    }
}

/// A share of an area in a row: a swatch, its name, its part.
fn shares_row(ui: &mut Ui, s: &Share) {
    let pal = Palette::of(ui);
    let name = if s.kind.is_empty() { s.name.clone() } else { format!("{} · {}", s.kind, s.name) };
    let name = if s.auto { tr!("{name} (automatic)", name) } else { name };
    // The name on the left after its swatch, its share on the right, a bar
    // of it under the row.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    let sw = Rect::from_center_size(pos2(rect.left() + 5.0, rect.center().y - 1.0), vec2(10.0, 10.0));
    p.rect_filled(sw, theme::TILE_RADIUS / 4, theme::rgb(s.color));
    let value = p.layout_no_wrap(if s.share > 0.0 && s.share < 0.005 { "<1 %".to_owned() } else { format!("{:.0} %", s.share * 100.0) }, FontId::monospace(12.0), pal.text);
    let vw = value.size().x;
    p.galley(pos2(rect.right() - vw, rect.center().y - value.size().y / 2.0 - 1.0), value, pal.text);
    let room = (rect.width() - vw - 26.0).max(10.0);
    let job = egui::text::LayoutJob { wrap: egui::text::TextWrapping::truncate_at_width(room), ..egui::text::LayoutJob::simple_singleline(name, FontId::proportional(12.5), pal.text) };
    let galley = p.layout_job(job);
    p.galley(pos2(sw.right() + 8.0, rect.center().y - galley.size().y / 2.0 - 1.0), galley, pal.text);
    let bar = Rect::from_min_size(pos2(rect.left(), rect.bottom() - 4.0), vec2(rect.width(), 2.0));
    p.rect_filled(bar, 1, pal.border);
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * s.share.clamp(0.0, 1.0), 2.0)), 1, pal.accent);
}

/// The profile of the land along a line: its heights over the distance,
/// the water over them; under the pointer, the spot, marked on the map
/// too.
pub fn profile(ui: &mut Ui, l: &LineReport) {
    let pal = Palette::of(ui);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 110.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect(rect, theme::GROUP_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let plot = Rect::from_min_max(rect.min + vec2(34.0, 10.0), rect.max - vec2(10.0, 18.0));
    let heights = l.profile.iter().filter_map(|q| q.ground.map(|g| g.max(q.water.unwrap_or(g))));
    let (lo, hi) = heights.clone().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(h), b.max(h)));
    if lo > hi {
        return;
    }
    // A little room over and under, at least 8 blocks shown.
    let pad = ((hi - lo) * 0.1).max((8.0 - (hi - lo)).max(0.0) / 2.0);
    let (lo, hi) = (lo - pad, hi + pad);
    let total = l.profile.last().map_or(1.0, |q| q.along).max(1e-3);
    let x_of = |d: f32| plot.left() + d / total * plot.width();
    let y_of = |h: f32| plot.bottom() - (h - lo) / (hi - lo) * plot.height();
    // The grid: the lowest and highest heights, the length.
    let weak = pal.weak;
    for h in [lo + pad, hi - pad] {
        let y = y_of(h);
        p.hline(plot.x_range(), y, Stroke::new(1.0, pal.border));
        p.text(pos2(plot.left() - 6.0, y), egui::Align2::RIGHT_CENTER, format!("{}", h.round() as i32), FontId::monospace(10.0), weak);
    }
    p.text(pos2(plot.left(), rect.bottom() - 4.0), egui::Align2::LEFT_BOTTOM, "0", FontId::monospace(10.0), weak);
    p.text(pos2(plot.right(), rect.bottom() - 4.0), egui::Align2::RIGHT_BOTTOM, length(total as f64), FontId::monospace(10.0), weak);
    // The ground: a line over a shade, in runs where the map has it.
    let fill = pal.accent.gamma_multiply(0.18);
    let water = Color32::from_rgb(60, 110, 200).gamma_multiply(0.55);
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() > 1 {
            let mut mesh = egui::Mesh::default();
            for (i, q) in run.iter().enumerate() {
                mesh.colored_vertex(*q, fill);
                mesh.colored_vertex(pos2(q.x, plot.bottom()), fill);
                if i > 0 {
                    let k = i as u32 * 2;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            p.add(mesh);
            p.add(egui::Shape::line(run.clone(), Stroke::new(1.5, pal.accent)));
        }
        run.clear();
    };
    for w in l.profile.windows(2) {
        // Water over the ground, in a band.
        if let (Some(g0), Some(w0), Some(g1)) = (w[0].ground, w[0].water, w[1].ground) {
            let w1 = w[1].water.unwrap_or(w0);
            let quad = vec![pos2(x_of(w[0].along), y_of(w0)), pos2(x_of(w[1].along), y_of(w1)), pos2(x_of(w[1].along), y_of(g1)), pos2(x_of(w[0].along), y_of(g0))];
            p.add(egui::Shape::convex_polygon(quad, water, Stroke::NONE));
        }
    }
    for q in &l.profile {
        match q.ground {
            Some(g) => run.push(pos2(x_of(q.along), y_of(g))),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    // Under the pointer: the spot, its distance and height.
    let mut probe = None;
    if let Some(at) = response.hover_pos().filter(|a| plot.x_range().contains(a.x)) {
        let d = (at.x - plot.left()) / plot.width() * total;
        if let Some(q) = l.profile.iter().min_by(|a, b| (a.along - d).abs().total_cmp(&(b.along - d).abs()))
            && let Some(g) = q.ground
        {
            let x = x_of(q.along);
            p.vline(x, plot.y_range(), Stroke::new(1.0, pal.text.gamma_multiply(0.5)));
            p.circle(pos2(x, y_of(g)), 3.5, pal.accent, Stroke::new(1.5, pal.panel));
            let text = format!("{} · {}", length(q.along as f64), block_top(g));
            let anchor = if x > plot.center().x { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
            p.text(pos2(x + if x > plot.center().x { -6.0 } else { 6.0 }, plot.top()), anchor, text, FontId::proportional(11.0), pal.text);
            probe = Some(q.along);
        }
    }
    ui.ctx().data_mut(|d| match probe {
        Some(v) => {
            d.insert_temp(probe_id(), v);
        }
        None => d.remove::<f32>(probe_id()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terrain made up of its heights alone.
    struct Heights<F: FnMut(i32, i32) -> Option<f32>>(F);

    impl<F: FnMut(i32, i32) -> Option<f32>> Ground for Heights<F> {
        fn col(&mut self, x: i32, z: i32, _full: bool) -> Option<Col> {
            (self.0)(x, z).map(|height| Col { height, ..Col::default() })
        }
        fn sea_level(&self) -> i32 {
            62
        }
    }

    fn close(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    /// On flat ground the distance along the ground is the distance on the
    /// map, and there is no slope.
    #[test]
    fn a_line_on_flat_ground() {
        let mut g = Heights(|_, _| Some(64.0));
        let r = line(&mut g, &[(0.5, 0.5), (30.5, 40.5)], false);
        assert!(close(r.horizontal, 50.0, 1e-9), "{}", r.horizontal);
        assert!(close(r.ground, 50.0, 1e-9), "{}", r.ground);
        assert_eq!((r.from, r.to, r.lowest, r.highest), (Some(64), Some(64), Some(64), Some(64)));
        assert_eq!((r.mean_slope, r.max_slope, r.climb, r.descent), (0.0, 0.0, 0.0, 0.0));
        assert!(r.profile.len() >= 50 && r.profile.len() <= PROFILE_POINTS);
    }

    /// Up a ramp rising half a block a block: 50 blocks up over 100, a
    /// slope of 50 % (26.6°) all along, √1.25 as long on the ground.
    #[test]
    fn a_line_up_a_ramp() {
        let mut g = Heights(|x, _| Some(64.0 + x as f32 * 0.5));
        let r = line(&mut g, &[(10.5, 5.5), (110.5, 5.5)], false);
        assert!(close(r.horizontal, 100.0, 1e-9));
        assert!(close(r.ground, 100.0 * 1.25f64.sqrt(), 1e-3), "{}", r.ground);
        assert_eq!((r.from, r.to), (Some(69), Some(119)));
        assert!(close(r.climb, 50.0, 1e-3) && r.descent == 0.0);
        assert!(close(r.mean_slope, 0.5, 1e-6), "{}", r.mean_slope);
        assert!(close(r.max_slope, 0.5, 1e-6), "{}", r.max_slope);
        assert!(close(r.max_slope.atan().to_degrees(), 26.565, 1e-2));
        // Back down the other way: the same slopes, come down.
        let back = line(&mut g, &[(110.5, 5.5), (10.5, 5.5)], false);
        assert!(close(back.descent, 50.0, 1e-3) && back.climb == 0.0);
        assert!(close(back.mean_slope, 0.5, 1e-6));
    }

    /// Over a cliff, the steepest slope is the cliff's over a few blocks,
    /// the mean the drop over the length.
    #[test]
    fn a_line_over_a_cliff() {
        let mut g = Heights(|x, _| Some(if x < 50 { 100.0 } else { 60.0 }));
        let r = line(&mut g, &[(0.5, 0.5), (100.5, 0.5)], false);
        assert!(close(r.descent, 40.0, 1e-6));
        assert!(close(r.mean_slope, 0.4, 1e-6));
        assert!(close(r.max_slope, 40.0 / SLOPE_RUN, 1e-6), "{}", r.max_slope);
        assert_eq!((r.lowest, r.highest), (Some(60), Some(100)));
    }

    /// A path of three points: its length is its legs', its height
    /// difference its ends'.
    #[test]
    fn a_path_of_points() {
        let mut g = Heights(|x, z| Some(64.0 + (x + z) as f32 * 0.0));
        let r = line(&mut g, &[(0.5, 0.5), (30.5, 0.5), (30.5, 40.5)], false);
        assert!(close(r.horizontal, 70.0, 1e-9));
        assert_eq!(r.points, 3);
        // Closed: the way back too, 50 more.
        let r = line(&mut g, &[(0.5, 0.5), (30.5, 0.5), (30.5, 40.5)], true);
        assert!(close(r.horizontal, 120.0, 1e-9));
    }

    /// Off the map the line goes on, without heights.
    #[test]
    fn a_line_off_the_map() {
        let mut g = Heights(|x, _| (x < 20).then_some(70.0));
        let r = line(&mut g, &[(0.5, 0.5), (40.5, 0.5)], false);
        assert!(close(r.horizontal, 40.0, 1e-9));
        assert_eq!((r.from, r.to), (Some(70), None));
        assert!(r.profile.iter().any(|q| q.ground.is_none()));
    }

    /// A square of 10 × 10 on flat ground at 64, a block of 5 × 5 standing
    /// 10 high in it: 100 blocks², 40 round, 250 blocks over the lowest.
    #[test]
    fn an_area_with_a_block_on_it() {
        let mut g = Heights(|x, z| Some(if (12..17).contains(&x) && (12..17).contains(&z) { 74.0 } else { 64.0 }));
        let square = [(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)];
        let a = area(&mut g, &square, Base::Lowest, 0, false).unwrap();
        assert_eq!(a.columns, 100.0);
        assert!(close(a.perimeter, 40.0, 1e-9));
        assert_eq!((a.lowest, a.highest, a.base, a.stride), (64, 74, 64, 1));
        assert!(close(a.mean, 66.5, 1e-9), "{}", a.mean);
        assert_eq!((a.above, a.below), (250.0, 0.0));
        // From the sea, at 62: two blocks more under every column.
        let a = area(&mut g, &square, Base::Sea, 0, false).unwrap();
        assert_eq!((a.base, a.above, a.below), (62, 450.0, 0.0));
        // From 70: the block's 4 over it, the rest 6 under it.
        let a = area(&mut g, &square, Base::Height, 70, false).unwrap();
        assert_eq!((a.above, a.below), (100.0, 75.0 * 6.0));
    }

    /// A triangle's columns are those whose centre is inside it: about
    /// its area; a circle's, about π r².
    #[test]
    fn areas_of_shapes() {
        let mut g = Heights(|_, _| Some(64.0));
        let a = area(&mut g, &[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)], Base::Lowest, 0, false).unwrap();
        assert!((a.columns - 5000.0).abs() <= 100.0, "{}", a.columns);
        let circle: Vec<(f32, f32)> = (0..360).map(|k| (k as f32).to_radians()).map(|t| (200.0 + 50.0 * t.cos(), 200.0 + 50.0 * t.sin())).collect();
        let a = area(&mut g, &circle, Base::Lowest, 0, false).unwrap();
        let pi_r2 = std::f64::consts::PI * 2500.0;
        assert!((a.columns - pi_r2).abs() / pi_r2 < 0.01, "{}", a.columns);
        assert!((a.perimeter - 2.0 * std::f64::consts::PI * 50.0).abs() < 0.1);
        // The runs agree with the test of each column.
        let mut n = 0;
        runs(&circle, 1, |x, z| {
            assert!(inside(&circle, x as f64 + 0.5, z as f64 + 0.5));
            n += 1;
        });
        assert_eq!(n as f64, a.columns);
    }

    /// An area of millions of columns is read one column in several, each
    /// standing for as many: its area stays right.
    #[test]
    fn a_large_area_is_read_sparsely() {
        let mut g = Heights(|_, _| Some(64.0));
        let a = area(&mut g, &[(0.0, 0.0), (3000.0, 0.0), (3000.0, 3000.0), (0.0, 3000.0)], Base::Height, 60, false).unwrap();
        assert!(a.stride > 1);
        assert!((a.columns - 9_000_000.0).abs() / 9e6 < 0.01, "{}", a.columns);
        assert!((a.above - 36_000_000.0).abs() / 36e6 < 0.01);
    }

    /// Drawn as a user draws: a drag is a line; clicks are points, a
    /// double-click ends them; clicking the first point closes an area.
    #[test]
    fn drawing_a_measure() {
        let mut m = Measure::default();
        m.press((0.0, 0.0), Kind::Line, false);
        m.drag((5.0, 0.0), 20.0, 4.0, Kind::Line);
        m.drag((10.0, 0.0), 40.0, 4.0, Kind::Line);
        m.release((10.0, 0.0), Kind::Line);
        assert!(m.done && !m.closed);
        assert_eq!(m.points, vec![(0.0, 0.0), (10.0, 0.0)]);
        // A click after: a new one.
        m.press((50.0, 50.0), Kind::Line, false);
        m.release((50.0, 50.0), Kind::Line);
        assert!(m.drawing());
        m.press((60.0, 50.0), Kind::Line, false);
        m.release((60.0, 50.0), Kind::Line);
        m.live = Some((60.0, 70.0));
        assert_eq!(m.path().len(), 3, "the pointer its last point");
        // A double-click: the second press on the same spot, then finish.
        m.press((60.0, 70.0), Kind::Line, false);
        m.press((60.0, 70.0), Kind::Line, false);
        m.finish(Kind::Line);
        assert!(m.done);
        assert_eq!(m.points, vec![(50.0, 50.0), (60.0, 50.0), (60.0, 70.0)]);
        // An area by its corners, closed on its first one.
        let mut m = Measure::default();
        for p in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)] {
            m.press(p, Kind::Area, false);
            m.release(p, Kind::Area);
        }
        m.press((0.2, 0.1), Kind::Area, true);
        assert!(m.done && m.closed && m.points.len() == 3);
        // An area dragged: a lasso.
        let mut m = Measure::default();
        m.press((0.0, 0.0), Kind::Area, false);
        for (k, p) in [(10.0, 0.0), (10.0, 10.0), (0.0, 10.0)].into_iter().enumerate() {
            m.drag(p, 40.0 * (k + 1) as f32, 4.0, Kind::Area);
        }
        m.release((0.0, 10.0), Kind::Area);
        assert!(m.closed && m.points.len() == 4);
        // The kind changed: an area opens, a path closes.
        m.set_kind(Kind::Line);
        assert!(!m.closed);
        m.set_kind(Kind::Area);
        assert!(m.closed);
        // Pinned, cleared: still pinned for the next one.
        m.pinned = true;
        m.clear();
        assert!(m.is_empty() && m.pinned);
    }

    /// On a real map: a measure reads the map's heights, and an area what
    /// covers it.
    #[test]
    fn an_area_on_the_map_says_what_covers_it() {
        use massif_core::generate::{GenParams, Landform, generate};
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Flat, flat_height: 64.0, ..GenParams::default() });
        let stone = world.registry.terrain_by_name("Stone").unwrap().0;
        let grass = world.registry.terrain_by_name("Grass").unwrap().0;
        let ground = world.dims[0].ground_mut();
        for x in 0..20 {
            for z in 0..20 {
                let tile = ground.tile_mut(TilePos::containing(x, z)).unwrap();
                let i = local_index(x, z);
                tile.terrain.set(i, if x < 10 { stone } else { grass });
                if x < 5 {
                    tile.cover.set(i, 1);
                }
            }
        }
        let doc = Document::new(world, None, 1);
        let mut tools = ToolState { measure_kind: Kind::Area, ..ToolState::default() };
        tools.measure.points = vec![(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)];
        tools.measure.closed = true;
        tools.measure.done = true;
        let r = compute(&doc, &tools).unwrap();
        let a = r.area.unwrap();
        assert_eq!(a.columns, 400.0);
        let share = |v: &[Share], name: &str| v.iter().find(|s| s.name == name).map(|s| s.share);
        assert_eq!(share(&a.terrains, "Stone"), Some(0.5), "{:?}", a.terrains);
        assert_eq!(share(&a.terrains, "Grass"), Some(0.5));
        assert!(a.layers.iter().any(|s| s.kind == "Cover" && s.share == 0.25), "{:?}", a.layers);
        assert!(!a.biomes.is_empty() && a.biomes[0].auto);
        assert!(close(r.line.horizontal, 80.0, 1e-9), "the perimeter");
    }

    /// The Info panel with a measure: where the cursor is, its ground and
    /// layers, then the measure's results, a line's with its profile, an
    /// area's with what covers it. Rendered with MASSIF_SNAPSHOT_DIR, as
    /// narrow as the sidebar goes, light and dark.
    #[test]
    fn the_info_panel_with_a_measure() {
        use massif_core::generate::{GenParams, Landform, generate};
        let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Hills, ..GenParams::default() });
        let doc = Document::new(world, None, 1);
        let mut tools = ToolState::default();
        tools.measure.points = vec![(-300.0, -100.0), (-120.0, 40.0), (150.0, 60.0)];
        tools.measure.done = true;
        let line = compute(&doc, &tools).unwrap();
        assert!(line.line.horizontal > 400.0 && line.line.profile.len() > 100);
        tools.measure.points = vec![(-200.0, -200.0), (100.0, -180.0), (160.0, 120.0), (-180.0, 90.0)];
        tools.measure.closed = true;
        tools.measure_base = Base::Sea;
        let area = compute(&doc, &tools).unwrap();
        let a = area.area.as_ref().unwrap();
        assert!(a.columns > 80_000.0 && !a.terrains.is_empty() && !a.biomes.is_empty());
        let info = crate::inspect::ColumnInfo::read(&doc, -120, 40).unwrap();
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(|d| std::path::PathBuf::from(d).join("measure")) else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        for (name, report) in [("line", line), ("area", area)] {
            for dark in [true, false] {
                let (info, reg, report) = (info.clone(), doc.world.registry.clone(), report.clone());
                let mut h = egui_kittest::Harness::builder().with_size(vec2(272.0, 900.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui(move |ui| {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                    let pal = Palette::of(ui);
                    egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                        ui.set_width(248.0);
                        crate::inspect::panel(ui, Some(&info), &reg, None, Some((&report, false)));
                    });
                });
                h.run_steps(3);
                let theme = if dark { "dark" } else { "light" };
                h.render().unwrap().save(dir.join(format!("info-{name}-{theme}.png"))).unwrap();
            }
        }
    }
}
