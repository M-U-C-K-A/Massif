//! File ▸ Share Image…: a picture of the open map, ready to post, in a
//! click. A few ready-made compositions are drawn off screen from a copy of
//! the map — the map as a diorama block, a low-sun view of its most
//! dramatic relief, the map seen from above with a scale and a north arrow,
//! and the map beside the relief — and the dialog shows them as a carousel
//! to pick from. A slim band over the picture, which can be left out, says
//! the map's name, its size and the version of Minecraft, with Massif's
//! mark. The picture is copied to the clipboard or saved as a PNG.
//!
//! The 3D pictures come from the 3D view's renderer (`view3d::Still3d`),
//! the map from the 2D map's colours (`map_render`); the band and the marks
//! are drawn on the processor with `vello_cpu`, as egui draws its glyphs.
//! Everything is drawn on a thread of its own, which the dialog stops when
//! it closes.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::view3d::{Camera, Sky, Still3d};
use crate::widgets::{button_group, row, switch, well};
use egui::{Color32, ColorImage, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, vec2};
use image::{Rgba, RgbaImage};
use massif_core::World;
use massif_core::coords::TILE_SIZE;
use massif_core::ops::{Surface, height_at};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use vello_cpu::kurbo::{self, Shape};
use vello_cpu::{RenderContext, Resources, color, peniko};

/// The compositions, in the order the carousel shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Diorama,
    Hero,
    Top,
    Split,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Diorama, Kind::Hero, Kind::Top, Kind::Split];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Diorama => tr!("Diorama"),
            Kind::Hero => tr!("Relief"),
            Kind::Top => tr!("Map"),
            Kind::Split => tr!("Map and relief"),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Kind::Diorama => "diorama",
            Kind::Hero => "relief",
            Kind::Top => "map",
            Kind::Split => "map-and-relief",
        }
    }
}

/// The sizes a picture is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Wide,
    Square,
    Uhd,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Wide, Format::Square, Format::Uhd];

    pub fn size(self) -> [u32; 2] {
        match self {
            Format::Wide => [1920, 1080],
            Format::Square => [1080, 1080],
            Format::Uhd => [3840, 2160],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Wide => tr!("16:9"),
            Format::Square => tr!("Square"),
            Format::Uhd => "4K",
        }
    }
}

/// What the band says of the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub name: String,
    /// Blocks across and down.
    pub size: [i64; 2],
    /// The version of Minecraft the map is made for, when known.
    pub version: Option<String>,
}

impl Info {
    pub fn of(world: &World, dim: usize, version: Option<String>) -> Info {
        let size = world.dims.get(dim).and_then(|d| d.bounds()).map_or([0, 0], |(lo, hi)| [(hi.x - lo.x + 1) as i64 * TILE_SIZE as i64, (hi.z - lo.z + 1) as i64 * TILE_SIZE as i64]);
        let name = if world.meta.name.trim().is_empty() { tr!("Untitled map").to_owned() } else { world.meta.name.trim().to_owned() };
        Info { name, size, version }
    }

    /// The line under the name: its size and its game.
    fn details(&self) -> String {
        let size = tr!("{w} × {d} blocks", w = thousands(self.size[0]), d = thousands(self.size[1]));
        match &self.version {
            Some(v) => format!("{size}  ·  {}", tr!("Minecraft {v}", v)),
            None => size,
        }
    }
}

/// `n` with its thousands apart: 6 144.
fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// How the band looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Band {
    pub shown: bool,
    pub dark: bool,
}

// ------------------------------------------------------------ the land --

/// Where the land is and how high, as the compositions frame it: blocks
/// of the map's corners, and a coarse grid of the heights of its ground.
struct Land {
    x0: f64,
    z0: f64,
    x1: f64,
    z1: f64,
    /// `GRID` × `GRID` heights, row-major from the north-west; NaN where
    /// the map has no ground.
    heights: Vec<f32>,
    sea: f32,
    min_y: i32,
}

const GRID: usize = 96;

impl Land {
    fn of(world: &World, dim: usize) -> Option<Land> {
        let d = world.dims.get(dim)?;
        let (lo, hi) = d.bounds()?;
        let (x0, z0) = ((lo.x * TILE_SIZE) as f64, (lo.z * TILE_SIZE) as f64);
        let (x1, z1) = (((hi.x + 1) * TILE_SIZE) as f64, ((hi.z + 1) * TILE_SIZE) as f64);
        let heights: Vec<f32> = {
            use rayon::prelude::*;
            (0..GRID * GRID)
                .into_par_iter()
                .map(|k| {
                    let (i, j) = (k % GRID, k / GRID);
                    let x = x0 + (x1 - x0) * (i as f64 + 0.5) / GRID as f64;
                    let z = z0 + (z1 - z0) * (j as f64 + 0.5) / GRID as f64;
                    height_at(d, 0, x.floor() as i32, z.floor() as i32, Surface::Top).unwrap_or(f32::NAN)
                })
                .collect()
        };
        Some(Land { x0, z0, x1, z1, heights, sea: world.meta.sea_level as f32, min_y: d.min_y })
    }

    fn extent(&self) -> f64 {
        (self.x1 - self.x0).max(self.z1 - self.z0)
    }

    fn known(&self) -> impl Iterator<Item = f32> + '_ {
        self.heights.iter().copied().filter(|h| h.is_finite())
    }

    fn highest(&self) -> f32 {
        self.known().fold(self.sea, f32::max)
    }

    fn lowest(&self) -> f32 {
        self.known().fold(f32::INFINITY, f32::min).min(self.sea)
    }

    fn at(&self, i: usize, j: usize) -> f32 {
        let h = self.heights[j.min(GRID - 1) * GRID + i.min(GRID - 1)];
        if h.is_finite() { h.max(self.sea) } else { self.sea }
    }

    fn block(&self, i: f64, j: f64) -> [f64; 2] {
        [self.x0 + (self.x1 - self.x0) * (i + 0.5) / GRID as f64, self.z0 + (self.z1 - self.z0) * (j + 0.5) / GRID as f64]
    }

    /// The most dramatic place of the map: the cell standing highest over
    /// the land around it, weighed by how high it is; its cell, its height
    /// and how far it stands over its surroundings, in blocks.
    fn peak(&self) -> (usize, usize, f32, f32) {
        let r = (GRID / 10).max(2) as i32;
        let mut best = (GRID / 2, GRID / 2, self.at(GRID / 2, GRID / 2), 0.0f32, f32::MIN);
        for j in 0..GRID {
            for i in 0..GRID {
                let h = self.at(i, j);
                let (mut sum, mut n) = (0.0, 0);
                for dj in (-r..=r).step_by(2) {
                    for di in (-r..=r).step_by(2) {
                        let (a, b) = (i as i32 + di, j as i32 + dj);
                        if a >= 0 && b >= 0 && (a as usize) < GRID && (b as usize) < GRID {
                            sum += self.at(a as usize, b as usize);
                            n += 1;
                        }
                    }
                }
                let rise = h - sum / n.max(1) as f32;
                // Away from the map's edges, where the view would end.
                let edge = (i.min(GRID - 1 - i).min(j).min(GRID - 1 - j) as f32 / (GRID as f32 * 0.15)).min(1.0);
                let score = (rise + (h - self.sea) * 0.35) * (0.4 + 0.6 * edge);
                if score > best.4 {
                    best = (i, j, h, rise, score);
                }
            }
        }
        (best.0, best.1, best.2, best.3.max(0.0))
    }
}

// ---------------------------------------------------------- the framing --

/// The background colour the diorama is drawn on, replaced by a gradient
/// afterwards.
const KEY: [u8; 3] = [228, 231, 236];

/// The camera that shows the whole map as a block, turned a little, in a
/// picture of `aspect`, filling `fill` of its width and height (the room
/// above the band).
fn diorama_camera(land: &Land, bottom: f64, aspect: f32, fill: [f32; 2], lift: f32) -> Camera {
    let extent = land.extent() as f32;
    let top = land.highest() as f64;
    let corners: Vec<[f64; 3]> = [land.x0, land.x1].iter().flat_map(|&x| [land.z0, land.z1].into_iter().flat_map(move |z| [bottom, top].map(|y| [x, y, z]))).collect();
    let mut camera = Camera { target: [(land.x0 + land.x1) / 2.0, (bottom + top) / 2.0, (land.z0 + land.z1) / 2.0], yaw: 0.72, pitch: 0.58, distance: extent * 1.6 };
    for _ in 0..3 {
        // The distance at which the block just fits, by halves.
        let (mut near, mut far) = (extent * 0.2, extent * 12.0);
        for _ in 0..40 {
            let mid = (near + far) / 2.0;
            let c = Camera { distance: mid, ..camera };
            let fits = corners.iter().all(|&p| Still3d::project(&c, aspect, extent, p).is_some_and(|q| q[0].abs() <= fill[0] && (q[1] + lift).abs() <= fill[1]));
            if fits { far = mid } else { near = mid }
        }
        camera.distance = far;
        // Its middle on the picture's: the target moved along the
        // picture's axes by what the block is off by.
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for &p in &corners {
            if let Some(q) = Still3d::project(&camera, aspect, extent, p) {
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
        }
        let off = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0 + lift];
        let tan = (50.0f32.to_radians() / 2.0).tan() * camera.distance;
        let (sy, cy) = camera.yaw.sin_cos();
        // The picture's right is (cos yaw, 0, -sin yaw); its up, roughly
        // the world's up turned by the pitch.
        let right = [cy as f64, 0.0, -sy as f64];
        let (sp, cp) = camera.pitch.sin_cos();
        let up = [-(sp * sy) as f64, cp as f64, -(sp * cy) as f64];
        let (dx, dy) = ((off[0] * tan * aspect) as f64, (-off[1] * tan) as f64);
        for k in 0..3 {
            camera.target[k] += right[k] * dx + up[k] * dy;
        }
    }
    camera
}

/// The camera on the map's most dramatic relief, low, the sun raking it
/// from the side; and where the sun is.
fn hero_camera(land: &Land) -> (Camera, [f32; 3]) {
    let extent = land.extent() as f32;
    let (i, j, h, rise) = land.peak();
    let [x, z] = land.block(i as f64, j as f64);
    let relief = (h - land.lowest()).max(rise).max(24.0);
    let distance = (relief * 4.2 + 160.0).clamp(180.0, extent * 0.75);
    // From where the land is lowest around it, for the view to be open.
    let mut best = (0.0f32, f32::MAX);
    for k in 0..16 {
        let yaw = k as f32 * std::f32::consts::TAU / 16.0;
        let (sy, cy) = yaw.sin_cos();
        let mut sum = 0.0;
        for s in 1..=8 {
            let d = distance * s as f32 / 8.0;
            let (bx, bz) = (x + (d * sy) as f64, z + (d * cy) as f64);
            let (gi, gj) = (((bx - land.x0) / (land.x1 - land.x0) * GRID as f64 - 0.5).round(), ((bz - land.z0) / (land.z1 - land.z0) * GRID as f64 - 0.5).round());
            // Off the map, the view is open but empty: a little worse.
            sum += if gi < 0.0 || gj < 0.0 || gi >= GRID as f64 || gj >= GRID as f64 { h } else { land.at(gi as usize, gj as usize) };
        }
        if sum < best.1 - 0.5 {
            best = (yaw, sum);
        }
    }
    let yaw = best.0;
    let target_y = (h - relief * 0.28).max(land.sea + 4.0) as f64;
    let camera = Camera { target: [x, target_y, z], yaw, pitch: 0.2, distance };
    // The sun low, ahead and to the right: long shadows across the view.
    let az = yaw + 2.2;
    let (sa, ca) = az.sin_cos();
    let elevation = 0.24f32;
    (camera, [sa * elevation.cos(), elevation.sin(), ca * elevation.cos()])
}

/// The sky over the relief: a warm horizon under a deep blue.
const HERO_SKY: Sky = Sky { zenith: [74, 122, 196], horizon: [232, 214, 196], mist: [222, 210, 200], haze: 0.55, murk: None };

// ------------------------------------------------------------ drawing --

/// What the pictures are drawn from.
pub struct Source {
    pub world: World,
    pub dim: usize,
    pub catalogue: Option<Arc<massif_assets::Catalogue>>,
    pub info: Info,
}

/// What the drawing thread says.
enum Message {
    /// Of all the work, how much is done (0..1).
    Progress(f32),
    Card(Kind, Arc<RgbaImage>),
    Done(Duration),
    Failed(String),
}

/// The pictures of every composition, without their band, at `size`,
/// handed to `each` as they come; `progress` told how far along it is.
pub fn draw_all(device: &wgpu::Device, queue: &wgpu::Queue, source: &Source, size: [u32; 2], cancel: &AtomicBool, progress: &dyn Fn(f32), each: &mut dyn FnMut(Kind, RgbaImage)) -> Result<(), String> {
    let land = Land::of(&source.world, source.dim).ok_or_else(|| tr!("The map is empty").to_owned())?;
    progress(0.02);
    // The map first: it comes in a moment, while the 3D view's tiles build.
    let top = top_map(source, &land, size, size[1] as f32 / 1080.0);
    each(Kind::Top, top);
    let stop = || cancel.load(Ordering::Relaxed);
    if stop() {
        return Ok(());
    }
    progress(0.12);
    let done = AtomicUsize::new(0);
    let total = source.world.dims.get(source.dim).map_or(1, |d| d.ground().positions().count()).max(1);
    let (mut still, _) = std::thread::scope(|s| {
        let built = s.spawn(|| Still3d::build(device, queue, &source.world, source.dim, source.catalogue.as_deref(), cancel, &done));
        while !built.is_finished() {
            progress(0.12 + 0.7 * (done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0));
            std::thread::sleep(Duration::from_millis(30));
        }
        built.join().ok().flatten()
    })
    .ok_or_else(|| if stop() { String::new() } else { tr!("The 3D pictures could not be drawn").to_owned() })?;
    progress(0.82);
    let extent = land.extent() as f32;
    let aspect = size[0] as f32 / size[1] as f32;
    // The block down to the bottom of the world, as the 3D view draws it.
    let bottom = land.min_y as f64;
    let camera = diorama_camera(&land, bottom, aspect, [0.84, 0.74], 0.06);
    let picture = still.draw(&camera, Sky::flat(KEY), None, extent, size).ok_or_else(|| tr!("The 3D pictures could not be drawn").to_owned())?;
    each(Kind::Diorama, studio(picture));
    progress(0.88);
    if stop() {
        return Ok(());
    }
    let (camera, sun) = hero_camera(&land);
    let hero = still.draw(&camera, HERO_SKY, Some(sun), extent, size).ok_or_else(|| tr!("The 3D pictures could not be drawn").to_owned())?;
    each(Kind::Hero, hero);
    progress(0.94);
    if stop() {
        return Ok(());
    }
    let half = [size[0] / 2, size[1]];
    let right = still.draw(&camera, HERO_SKY, Some(sun), extent, half).ok_or_else(|| tr!("The 3D pictures could not be drawn").to_owned())?;
    each(Kind::Split, split(source, &land, &right, size));
    progress(1.0);
    Ok(())
}

/// The diorama on a soft gradient, with a shadow under the block.
fn studio(mut img: RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let key = |p: &Rgba<u8>| (0..3).all(|k| (p[k] as i32 - KEY[k] as i32).abs() <= 2);
    // The block's shape, small, blurred and let down a little: its shadow.
    let k = 8;
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let mut mask = image::GrayImage::new(sw, sh);
    for (x, y, p) in img.enumerate_pixels() {
        if !key(p) {
            mask.put_pixel(x / k, y / k, image::Luma([255]));
        }
    }
    let shadow = image::imageops::blur(&mask, (sh as f32 * 0.025).max(1.5));
    let drop = (sh as f32 * 0.03) as u32;
    for y in 0..h {
        let t = y as f32 / h as f32;
        for x in 0..w {
            let p = img.get_pixel(x, y);
            if !key(p) {
                continue;
            }
            // Lighter at the top, a vignette towards the corners.
            let (u, v) = (x as f32 / w as f32 - 0.5, t - 0.42);
            let glow = 1.0 - (u * u * 0.9 + v * v * 1.4).min(1.0) * 0.55;
            let base = [lerp(214.0, 246.0, glow), lerp(219.0, 248.0, glow), lerp(226.0, 251.0, glow)];
            let (mx, my) = (x / k, (y / k).saturating_sub(drop));
            let dark = if my < sh { shadow.get_pixel(mx.min(sw - 1), my).0[0] as f32 / 255.0 } else { 0.0 } * 0.32;
            img.put_pixel(x, y, Rgba([(base[0] * (1.0 - dark)) as u8, (base[1] * (1.0 - dark)) as u8, (base[2] * (1.0 - dark * 0.8)) as u8, 255]));
        }
    }
    img
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The map's colours from above, `px` blocks... its picture fitting
/// `room` pixels at most.
fn map_picture(source: &Source, land: &Land, room: [u32; 2]) -> RgbaImage {
    use crate::map_render::{CHUNK_PX, MapMode, MapStyle, render_region};
    let dim = &source.world.dims[source.dim];
    let (w, d) = (land.x1 - land.x0, land.z1 - land.z0);
    let scale = (room[0] as f64 / w).min(room[1] as f64 / d);
    // The coarsest level of detail that still gives a texel a pixel.
    let mut lod = 0u8;
    while lod < 6 && (1u32 << (lod + 1)) as f64 * scale <= 1.0 {
        lod += 1;
    }
    let texel = (1i64 << lod) as f64;
    let chunk = CHUNK_PX as f64 * texel;
    let (cx0, cz0) = ((land.x0 / chunk).floor(), (land.z0 / chunk).floor());
    let (nx, nz) = (((land.x1 / chunk).ceil() - cx0) as i32, ((land.z1 / chunk).ceil() - cz0) as i32);
    let true_colour = source.catalogue.is_some();
    let style = MapStyle {
        mode: if true_colour { MapMode::TrueColour } else { MapMode::Terrain },
        smooth: true,
        lava: source.world.meta.lava,
        top: dim.max_y,
        seed: source.world.meta.seed,
        annotations: false,
        palette: if true_colour { crate::true_color::key(&source.world.registry) } else { 0 },
        ..MapStyle::default()
    };
    let image = render_region(dim, &source.world.registry, source.world.meta.sea_level as f32, style, lod, (cx0 * chunk) as i32, (cz0 * chunk) as i32, nx, nz);
    // The map's own blocks, cut out of the whole chunks.
    let (ox, oz) = (((land.x0 - cx0 * chunk) / texel) as u32, ((land.z0 - cz0 * chunk) / texel) as u32);
    let (tw, th) = ((w / texel).round() as u32, (d / texel).round() as u32);
    let mut out = RgbaImage::new(tw.max(1), th.max(1));
    for y in 0..out.height() {
        for x in 0..out.width() {
            let (sx, sy) = ((ox + x) as usize, (oz + y) as usize);
            let c = image.pixels.get(sy * image.size[0] + sx).copied().unwrap_or(Color32::TRANSPARENT);
            let c = if c.a() == 0 { Color32::from_rgb(20, 22, 26) } else { c };
            out.put_pixel(x, y, Rgba([c.r(), c.g(), c.b(), 255]));
        }
    }
    let fit = [((w * scale).round() as u32).max(1), ((d * scale).round() as u32).max(1)];
    image::imageops::resize(&out, fit[0], fit[1], if fit[0] < tw { image::imageops::FilterType::Triangle } else { image::imageops::FilterType::Nearest })
}

/// The dark slate the map is laid on.
const SLATE: [u8; 3] = [22, 25, 30];

/// The map from above on a dark ground, with its scale and north arrow,
/// clear of the band.
fn top_map(source: &Source, land: &Land, size: [u32; 2], s: f32) -> RgbaImage {
    let [w, h] = size;
    let margin = (56.0 * s) as u32;
    let band = (BAND_H * s) as u32;
    let room = [w.saturating_sub(2 * margin), h.saturating_sub(2 * margin + band)];
    let map = map_picture(source, land, room);
    let mut img = RgbaImage::from_pixel(w, h, Rgba([SLATE[0], SLATE[1], SLATE[2], 255]));
    let at = [(w - map.width()) / 2, margin + (room[1] - map.height().min(room[1])) / 2];
    lay(&mut img, &map, at, s);
    let blocks_px = map.width() as f64 / (land.x1 - land.x0);
    let mut marks = Marks::new(&img);
    marks.scale_bar([at[0] as f32, (at[1] + map.height()) as f32 + 14.0 * s], blocks_px, map.width() as f32 * 0.22, s, true);
    marks.north([(at[0] + map.width()) as f32 - 12.0 * s, (at[1] + map.height()) as f32 + 24.0 * s], s);
    marks.finish(&mut img);
    img
}

/// `pic` laid on `img` at `at`, with a soft shadow around it.
fn lay(img: &mut RgbaImage, pic: &RgbaImage, at: [u32; 2], s: f32) {
    let spread = (18.0 * s) as i64;
    let (w, h) = (pic.width() as i64, pic.height() as i64);
    for y in (at[1] as i64 - spread).max(0)..(at[1] as i64 + h + spread).min(img.height() as i64) {
        for x in (at[0] as i64 - spread).max(0)..(at[0] as i64 + w + spread).min(img.width() as i64) {
            let dx = (at[0] as i64 - x).max(x - (at[0] as i64 + w - 1)).max(0);
            let dy = (at[1] as i64 - y).max(y - (at[1] as i64 + h - 1)).max(0);
            if dx == 0 && dy == 0 {
                continue;
            }
            let d = ((dx * dx + dy * dy) as f32).sqrt() / spread as f32;
            let k = (1.0 - d).max(0.0).powi(2) * 0.5;
            let p = img.get_pixel_mut(x as u32, y as u32);
            for c in 0..3 {
                p[c] = (p[c] as f32 * (1.0 - k)) as u8;
            }
        }
    }
    image::imageops::overlay(img, pic, at[0] as i64, at[1] as i64);
}

/// The map beside the relief: the map on the left half, the relief drawn
/// for the right one.
fn split(source: &Source, land: &Land, right: &RgbaImage, size: [u32; 2]) -> RgbaImage {
    let [w, h] = size;
    let s = h as f32 / 1080.0;
    let half = w / 2;
    let mut img = RgbaImage::from_pixel(w, h, Rgba([SLATE[0], SLATE[1], SLATE[2], 255]));
    let margin = (44.0 * s) as u32;
    let band = (BAND_H * s) as u32;
    let room = [half.saturating_sub(2 * margin), h.saturating_sub(2 * margin + band)];
    let map = map_picture(source, land, room);
    let at = [(half - map.width()) / 2, margin + (room[1] - map.height().min(room[1])) / 2];
    lay(&mut img, &map, at, s);
    image::imageops::overlay(&mut img, right, half as i64, 0);
    let blocks_px = map.width() as f64 / (land.x1 - land.x0);
    let mut marks = Marks::new(&img);
    marks.scale_bar([at[0] as f32, (at[1] + map.height()) as f32 + 12.0 * s], blocks_px, map.width() as f32 * 0.3, s, true);
    marks.north([(at[0] + map.width()) as f32 - 12.0 * s, (at[1] + map.height()) as f32 + 22.0 * s], s);
    // A hairline between the two.
    marks.rect([half as f32 - 0.5 * s, 0.0, half as f32 + 0.5 * s, h as f32], [255, 255, 255, 60], 0.0);
    marks.finish(&mut img);
    img
}

// ------------------------------------------------------------- the band --

/// The band's height on a picture 1080 pixels high.
const BAND_H: f32 = 92.0;

/// The band's typeface: Geist, crisp at every size.
const FONT: &[u8] = include_bytes!("../assets/fonts/Geist.ttf");
const LOGO: &[u8] = include_bytes!("../assets/icon.png");

/// Marks drawn over a picture on the processor: text, bars and the band.
struct Marks {
    ctx: RenderContext,
    resources: Resources,
    font: peniko::FontData,
    size: [u32; 2],
    /// Logos to lay once the rest is drawn: where, and how large.
    logos: Vec<([f32; 2], f32)>,
}

impl Marks {
    fn new(img: &RgbaImage) -> Marks {
        let (w, h) = img.dimensions();
        Marks {
            ctx: RenderContext::new(w.min(u16::MAX as u32) as u16, h.min(u16::MAX as u32) as u16),
            resources: Resources::new(),
            font: peniko::FontData::new(peniko::Blob::new(Arc::new(FONT)), 0),
            size: [w, h],
            logos: Vec::new(),
        }
    }

    fn paint(&mut self, c: [u8; 4]) {
        self.ctx.set_paint(color::AlphaColor::<color::Srgb>::from_rgba8(c[0], c[1], c[2], c[3]));
    }

    /// A rectangle `[x0, y0, x1, y1]`, its corners rounded by `radius`.
    fn rect(&mut self, r: [f32; 4], c: [u8; 4], radius: f32) {
        self.paint(c);
        let path = kurbo::RoundedRect::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64, radius as f64).to_path(0.1);
        self.ctx.fill_path(&path);
    }

    /// How wide `text` is, `px` high, of weight `weight`.
    fn measure(text: &str, px: f32, weight: f32) -> f32 {
        layout(text, px, weight).1
    }

    /// `text` from `x` on the baseline `y`; its width.
    fn text(&mut self, text: &str, [x, y]: [f32; 2], px: f32, weight: f32, c: [u8; 4]) -> f32 {
        let (glyphs, width, coords) = layout(text, px, weight);
        self.paint(c);
        let font = self.font.clone();
        self.ctx.glyph_run(&mut self.resources, &font).font_size(px).hint(false).normalized_coords(&coords).fill_glyphs(glyphs.into_iter().map(|(id, gx)| vello_cpu::Glyph { id, x: x + gx, y }));
        width
    }

    /// A scale bar from `at` (its top left), about `most` pixels long at
    /// `px` pixels a block, and what it measures under it.
    fn scale_bar(&mut self, at: [f32; 2], px: f64, most: f32, s: f32, light: bool) {
        let blocks = nice((most as f64 / px).max(1.0));
        let len = (blocks * px) as f32;
        let (ink, gap) = if light { ([236, 238, 242, 235], [236, 238, 242, 70]) } else { ([24, 26, 30, 235], [24, 26, 30, 70]) };
        let t = 5.0 * s;
        self.rect([at[0], at[1], at[0] + len, at[1] + t], gap, t / 2.0);
        for k in [0, 2] {
            let (a, b) = (at[0] + len * k as f32 / 4.0, at[0] + len * (k + 1) as f32 / 4.0);
            self.rect([a, at[1], b, at[1] + t], ink, t / 2.0);
        }
        let label = tr!("{n} blocks", n = thousands(blocks as i64));
        self.text(&label, [at[0], at[1] + t + 20.0 * s], 15.0 * s, 500.0, ink);
    }

    /// A north arrow: "N" over a needle, its right end at `at[0]`, its
    /// middle at `at[1]`.
    fn north(&mut self, at: [f32; 2], s: f32) {
        let px = 15.0 * s;
        let wn = Self::measure("N", px, 650.0);
        let r = 9.0 * s;
        let (x, y) = ((at[0] - r) as f64, at[1] as f64);
        let r = r as f64;
        let mut needle = kurbo::BezPath::new();
        needle.move_to((x, y - r));
        needle.line_to((x + r * 0.62, y + r));
        needle.line_to((x, y + r * 0.5));
        needle.line_to((x - r * 0.62, y + r));
        needle.close_path();
        self.paint([236, 238, 242, 235]);
        self.ctx.fill_path(&needle);
        self.text("N", [x as f32 - r as f32 - 8.0 * s - wn, at[1] + px * 0.36 + 2.0 * s], px, 650.0, [236, 238, 242, 235]);
    }

    /// The band along the bottom: the map's name and what it is on the
    /// left, Massif's mark on the right.
    fn band(&mut self, info: &Info, dark: bool, s: f32) {
        let [w, h] = [self.size[0] as f32, self.size[1] as f32];
        let bh = BAND_H * s;
        let top = h - bh;
        let (bg, ink, weak) = if dark { ([14, 16, 20, 178], [246, 247, 249, 255], [246, 247, 249, 170]) } else { ([250, 251, 252, 200], [20, 22, 26, 255], [20, 22, 26, 150]) };
        self.rect([0.0, top, w, h], bg, 0.0);
        self.rect([0.0, top, w, top + 1.0 * s], if dark { [255, 255, 255, 26] } else { [0, 0, 0, 22] }, 0.0);
        let pad = 40.0 * s;
        let mark = tr!("Made with Massif");
        let mark_px = 19.0 * s;
        let logo = 34.0 * s;
        let mark_w = Self::measure(mark, mark_px, 560.0);
        let right = w - pad;
        // The name, cut to the room the mark leaves.
        let room = (right - mark_w - logo - 12.0 * s - 40.0 * s - pad).max(40.0 * s);
        let name_px = 30.0 * s;
        let name = fit(&info.name, name_px, 640.0, room);
        let details = fit(&info.details(), 18.0 * s, 480.0, room);
        let mid = top + bh / 2.0;
        self.text(&name, [pad, mid - 2.0 * s], name_px, 640.0, ink);
        self.text(&details, [pad, mid + 25.0 * s], 18.0 * s, 480.0, weak);
        self.text(mark, [right - mark_w, mid + mark_px * 0.36], mark_px, 560.0, ink);
        self.logos.push(([right - mark_w - 12.0 * s - logo, mid - logo / 2.0], logo));
    }

    fn finish(mut self, img: &mut RgbaImage) {
        let [w, h] = self.size;
        let mut pixmap = vello_cpu::Pixmap::new(w as u16, h as u16);
        pixmap.data_as_u8_slice_mut().copy_from_slice(img.as_raw());
        self.ctx.flush();
        self.ctx.render_with(&mut pixmap, &mut self.resources, vello_cpu::RasterizerSettings { composite_mode: vello_cpu::CompositeMode::SrcOver, ..Default::default() });
        img.copy_from_slice(pixmap.data_as_u8_slice());
        for (at, side) in self.logos {
            if let Ok(logo) = image::load_from_memory(LOGO) {
                let side = side.round().max(1.0) as u32;
                let logo = image::imageops::resize(&logo.to_rgba8(), side, side, image::imageops::FilterType::Lanczos3);
                image::imageops::overlay(img, &logo, at[0].round() as i64, at[1].round() as i64);
            }
        }
    }
}

/// The glyphs of `text` at `px` and their offsets, its width, and the
/// font's coordinates for `weight`.
fn layout(text: &str, px: f32, weight: f32) -> (Vec<(u32, f32)>, f32, Vec<i16>) {
    use skrifa::MetadataProvider;
    let Ok(font) = skrifa::FontRef::new(FONT) else { return (Vec::new(), 0.0, Vec::new()) };
    let location = font.axes().location([("wght", weight)]);
    let metrics = font.glyph_metrics(skrifa::instance::Size::new(px), &location);
    let charmap = font.charmap();
    let mut x = 0.0;
    let mut glyphs = Vec::new();
    for ch in text.chars() {
        let id = charmap.map(ch).or_else(|| charmap.map(' ')).unwrap_or_default();
        glyphs.push((id.to_u32(), x));
        x += metrics.advance_width(id).unwrap_or(px * 0.5);
    }
    let coords = location.coords().iter().map(|c| c.to_bits()).collect();
    (glyphs, x, coords)
}

/// `text`, cut with an ellipsis to `room` pixels.
fn fit(text: &str, px: f32, weight: f32, room: f32) -> String {
    if Marks::measure(text, px, weight) <= room {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let s: String = chars.iter().collect::<String>().trim_end().to_owned() + "…";
        if Marks::measure(&s, px, weight) <= room {
            return s;
        }
    }
    "…".to_owned()
}

/// The roundest length of 1, 2 or 5 times a power of ten up to `most`.
fn nice(most: f64) -> f64 {
    let p = 10f64.powf(most.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|k| k * p).find(|&v| v <= most).unwrap_or(p)
}

/// The picture of `kind` as it is shared: with its band, when shown.
pub fn finished(picture: &RgbaImage, info: &Info, band: Band) -> RgbaImage {
    let mut img = picture.clone();
    if band.shown {
        let s = img.height() as f32 / 1080.0;
        // Frosted: what lies under the band, blurred.
        let bh = (BAND_H * s) as u32;
        let (w, h) = img.dimensions();
        let under = image::imageops::crop_imm(&img, 0, h - bh, w, bh).to_image();
        let blurred = image::imageops::fast_blur(&under, 14.0 * s);
        image::imageops::replace(&mut img, &blurred, 0, (h - bh) as i64);
        let mut marks = Marks::new(&img);
        marks.band(info, band.dark, s);
        marks.finish(&mut img);
    }
    img
}

// ------------------------------------------------------------ the dialog --

struct Job {
    messages: Receiver<Message>,
    cancel: Arc<AtomicBool>,
    progress: f32,
    started: Instant,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A composition drawn: its picture without the band, and what the
/// dialog shows of it with its band.
struct Card {
    kind: Kind,
    picture: Arc<RgbaImage>,
    shown: Option<(Band, TextureHandle)>,
}

/// The dialog.
pub struct ShareForm {
    source: Arc<Source>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    pub format: Format,
    pub band: Band,
    pub picked: usize,
    cards: Vec<Card>,
    job: Option<Job>,
    /// How long the last drawing took, or why it failed.
    took: Option<Result<Duration, String>>,
    /// What the last Copy or Save did.
    said: Option<String>,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
}

impl ShareForm {
    pub fn new(source: Source, gpu: Option<(wgpu::Device, wgpu::Queue)>) -> Self {
        let mut form = ShareForm { source: Arc::new(source), gpu, format: Format::Wide, band: Band { shown: true, dark: true }, picked: 0, cards: Vec::new(), job: None, took: None, said: None };
        form.start();
        form
    }

    /// Draw every composition again, at the format picked.
    fn start(&mut self) {
        self.job = None;
        self.cards.clear();
        self.took = None;
        let Some((device, queue)) = self.gpu.clone() else {
            self.took = Some(Err(tr!("Pictures need a graphics card Massif can draw with").to_owned()));
            return;
        };
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (source, size, stop) = (self.source.clone(), self.format.size(), cancel.clone());
        let started = Instant::now();
        let spawned = std::thread::Builder::new().name("share image".into()).spawn(move || {
            let progress = |p: f32| {
                let _ = tx.send(Message::Progress(p));
            };
            let mut each = |kind: Kind, img: RgbaImage| {
                let _ = tx.send(Message::Card(kind, Arc::new(img)));
            };
            let outcome = draw_all(&device, &queue, &source, size, &stop, &progress, &mut each);
            let _ = tx.send(match outcome {
                Ok(()) => Message::Done(started.elapsed()),
                Err(e) => Message::Failed(e),
            });
        });
        match spawned {
            Ok(_) => self.job = Some(Job { messages: rx, cancel, progress: 0.0, started }),
            Err(e) => self.took = Some(Err(e.to_string())),
        }
    }

    /// Take in what the drawing thread said.
    pub fn poll(&mut self) {
        let Some(job) = &mut self.job else { return };
        let mut ended = None;
        while let Ok(m) = job.messages.try_recv() {
            match m {
                Message::Progress(p) => job.progress = p,
                Message::Card(kind, picture) => {
                    self.cards.retain(|c| c.kind != kind);
                    self.cards.push(Card { kind, picture, shown: None });
                    self.cards.sort_by_key(|c| Kind::ALL.iter().position(|k| *k == c.kind));
                }
                Message::Done(t) => ended = Some(Ok(t)),
                Message::Failed(e) => ended = Some(Err(e)),
            }
        }
        if ended.is_none() && job.started.elapsed() > Duration::from_secs(600) {
            ended = Some(Err(tr!("The pictures took too long").to_owned()));
        }
        if let Some(e) = ended {
            if let Ok(t) = &e {
                log::info!("share image: {} pictures of {:?} in {:.2} s", self.cards.len(), self.format.size(), t.as_secs_f64());
            }
            self.took = Some(e);
            self.job = None;
        }
    }

    /// Whether every composition is drawn.
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        self.job.is_none() && self.cards.len() == Kind::ALL.len()
    }

    /// The picture picked, as it is shared.
    pub fn picture(&self) -> Option<RgbaImage> {
        let card = self.cards.get(self.picked)?;
        Some(finished(&card.picture, &self.source.info, self.band))
    }

    /// The picture of each composition drawn, as shared.
    #[cfg(test)]
    pub fn pictures(&self) -> Vec<(Kind, RgbaImage)> {
        self.cards.iter().map(|c| (c.kind, finished(&c.picture, &self.source.info, self.band))).collect()
    }

    fn texture(&mut self, ctx: &egui::Context, k: usize) -> Option<TextureHandle> {
        let band = self.band;
        let info = self.source.info.clone();
        let card = self.cards.get_mut(k)?;
        if let Some((b, t)) = &card.shown
            && *b == band
        {
            return Some(t.clone());
        }
        // Shown at most 1280 pixels across: the dialog is smaller.
        let pic = finished(&card.picture, &info, band);
        let small = if pic.width() > 1280 { image::imageops::resize(&pic, 1280, pic.height() * 1280 / pic.width(), image::imageops::FilterType::Triangle) } else { pic };
        let image = ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
        let t = ctx.load_texture(format!("share {}", card.kind.slug()), image, TextureOptions::LINEAR);
        card.shown = Some((band, t.clone()));
        Some(t)
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        self.poll();
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(720.0f32.min(ui.ctx().content_rect().width() - 80.0));
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Share2).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Share Image")).size(16.0).bold());
        });
        ui.label(RichText::new(tr!("A picture of the map, ready to post. Pick one, then copy it or save it.")).size(12.0).color(pal.weak));
        ui.add_space(10.0);
        let width = ui.available_width();
        let aspect = { let [w, h] = self.format.size(); h as f32 / w as f32 };
        let preview_h = (width * aspect).min(420.0);
        let preview_w = preview_h / aspect;
        // The picture picked, between arrows.
        let (rect, _) = ui.allocate_exact_size(vec2(width, preview_h), Sense::hover());
        let frame = Rect::from_center_size(rect.center(), vec2(preview_w, preview_h));
        let n = Kind::ALL.len();
        self.picked = self.picked.min(n - 1);
        let picked_kind = Kind::ALL[self.picked];
        let k = self.cards.iter().position(|c| c.kind == picked_kind);
        let texture = k.and_then(|k| self.texture(ui.ctx(), k));
        let painter = ui.painter_at(rect);
        painter.rect_filled(frame, theme::TILE_RADIUS, pal.well());
        if let Some(t) = &texture {
            // Slid in from the side it came from, unless motion is reduced.
            let t_in = ui.ctx().animate_value_with_time(egui::Id::unique("share slide"), self.picked as f32, if theme::reduce_motion(ui.ctx()) { 0.0 } else { 0.18 });
            let off = (t_in - self.picked as f32) * 24.0;
            egui::Image::new(t).corner_radius(theme::TILE_RADIUS).paint_at(ui, frame.translate(vec2(off, 0.0)));
        } else {
            let p = self.job.as_ref().map_or(0.0, |j| j.progress);
            let bar = Rect::from_center_size(frame.center() + vec2(0.0, 14.0), vec2(frame.width() * 0.4, 4.0));
            painter.text(frame.center() - vec2(0.0, 6.0), egui::Align2::CENTER_CENTER, tr!("Drawing the pictures…"), egui::FontId::proportional(13.0), pal.weak);
            painter.rect_filled(bar, theme::PILL, theme::mix(pal.well(), pal.text, 0.12));
            painter.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * p, bar.height())), theme::PILL, pal.accent);
            if self.job.is_some() && !theme::reduce_motion(ui.ctx()) {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        if self.job.is_some() {
            // The thread says when a picture comes.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        for (dir, icon, x) in [(-1i32, Icon::ChevronLeft, rect.left() + 18.0), (1, Icon::ChevronRight, rect.right() - 18.0)] {
            let r = Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(32.0, 32.0));
            let name = if dir < 0 { tr!("Previous picture") } else { tr!("Next picture") };
            let resp = ui.interact(r, egui::Id::unique(("share arrow", dir)), Sense::click()).on_hover_text(name);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            let fill = if resp.hovered() { theme::mix(pal.control(), pal.text, 0.1) } else { pal.control() };
            ui.painter().circle(r.center(), 16.0, fill, egui::Stroke::new(1.0, pal.control_border()));
            crate::icons::glyph(ui.painter(), r.center(), 16.0, icon, pal.text);
            if resp.clicked() {
                self.picked = (self.picked as i32 + dir).rem_euclid(n as i32) as usize;
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.picked = (self.picked + n - 1) % n;
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.picked = (self.picked + 1) % n;
        }
        ui.add_space(8.0);
        // The thumbnails, their names under them.
        let thumb_w = ((width - 12.0 * (n as f32 - 1.0)) / n as f32).min(132.0);
        let thumb = vec2(thumb_w, thumb_w * aspect);
        ui.horizontal(|ui| {
            let used = n as f32 * thumb_w + (n as f32 - 1.0) * 12.0;
            ui.add_space((width - used) / 2.0);
            ui.spacing_mut().item_spacing.x = 12.0;
            for (i, kind) in Kind::ALL.into_iter().enumerate() {
                let k = self.cards.iter().position(|c| c.kind == kind);
                let texture = k.and_then(|k| self.texture(ui.ctx(), k));
                ui.vertical(|ui| {
                    let (r, resp) = ui.allocate_exact_size(thumb, Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, i == self.picked, kind.label()));
                    match &texture {
                        Some(t) => egui::Image::new(t).corner_radius(theme::TILE_RADIUS).paint_at(ui, r),
                        None => {
                            ui.painter().rect_filled(r, theme::TILE_RADIUS, pal.well());
                        }
                    }
                    let edge = if i == self.picked { egui::Stroke::new(2.0, pal.accent) } else if resp.hovered() { egui::Stroke::new(1.0, pal.control_border()) } else { egui::Stroke::new(1.0, pal.border) };
                    ui.painter().rect_stroke(r, theme::TILE_RADIUS, edge, egui::StrokeKind::Outside);
                    if resp.clicked() {
                        self.picked = i;
                    }
                    ui.add_space(2.0);
                    ui.allocate_ui(vec2(thumb_w, 16.0), |ui| ui.centered_and_justified(|ui| ui.label(RichText::new(kind.label()).size(11.5).color(if i == self.picked { pal.text } else { pal.weak }))));
                });
            }
        });
        ui.add_space(10.0);
        well(ui, |ui| {
            row(ui, tr!("Size"), |ui| {
                let labels: Vec<&str> = Format::ALL.iter().map(|f| f.label()).collect();
                let on: Vec<bool> = Format::ALL.iter().map(|f| *f == self.format).collect();
                if let Some(k) = button_group(ui, "share format", &labels, &on)
                    && Format::ALL[k] != self.format
                {
                    self.format = Format::ALL[k];
                    self.said = None;
                    self.start();
                }
            });
            row(ui, tr!("Band with the map's name"), |ui| {
                switch(ui, &mut self.band.shown);
            });
            if self.band.shown {
                row(ui, tr!("Band"), |ui| {
                    if let Some(k) = button_group(ui, "share band", &[tr!("Light"), tr!("Dark")], &[!self.band.dark, self.band.dark]) {
                        self.band.dark = k == 1;
                    }
                });
            }
        });
        ui.add_space(4.0);
        let [w, h] = self.format.size();
        let status = match (&self.said, &self.took, &self.job) {
            (Some(s), _, _) => s.clone(),
            (None, _, Some(j)) => tr!("Drawing… {p} %", p = (j.progress * 100.0).round() as u32),
            (None, Some(Err(e)), _) if !e.is_empty() => e.clone(),
            (None, Some(Ok(t)), _) => tr!("{w} × {h} pixels, drawn in {s:.1} s", w, h, s = t.as_secs_f32()),
            _ => String::new(),
        };
        ui.label(RichText::new(status).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let can = self.cards.iter().any(|c| c.kind == Kind::ALL[self.picked]);
            if ui.add_enabled(can, egui::Button::new(theme::with_icon(Icon::Copy, tr!("Copy")))).clicked()
                && let Some(img) = self.picture()
            {
                ui.ctx().copy_image(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()));
                self.said = Some(tr!("Copied: paste it into Discord, Reddit or anywhere").to_owned());
            }
            if ui.add_enabled(can, egui::Button::new(theme::with_icon(Icon::ImageDown, tr!("Save PNG…")))).clicked() {
                let name = format!("{} {}.png", self.source.info.name, Kind::ALL[self.picked].label().to_lowercase());
                if let Some(path) = rfd::FileDialog::new().set_title(tr!("Save the picture")).add_filter("PNG", &["png"]).set_file_name(name).save_file() {
                    self.said = Some(match self.save(&path) {
                        Ok(()) => tr!("Saved {name}", name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                        Err(e) => e,
                    });
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr!("Close")).clicked() {
                    choice = Choice::Close;
                }
            });
        });
        choice
    }

    /// Save the picture picked as a PNG at `path`.
    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let img = self.picture().ok_or_else(|| tr!("The picture is not drawn yet").to_owned())?;
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        massif_core::atomic::write(path, &png, true).map_err(|e| massif_core::atomic::describe(&e, path))
    }
}

#[cfg(test)]
#[path = "share_tests.rs"]
mod tests;
