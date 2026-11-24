//! Relief of real places: elevation data read into heights of the map.
//!
//! The data comes as *Terrarium* tiles (the Terrain Tiles of Mapzen, kept
//! on AWS: 256 × 256 PNG tiles of the Web Mercator grid, the height in the
//! colour of each pixel, ocean floors included), or as a file the user
//! downloaded (a GeoTIFF, a 16-bit PNG, a raw grid). Fetching the tiles is
//! the app's; this module decodes them, works out which ones an area needs,
//! projects the map's columns onto the Earth and turns metres into heights
//! of blocks, then writes them into the ground.
//!
//! An area is a rectangle of the map laid on the Earth around a centre: its
//! columns are offsets east and south of it, in metres at the chosen scale,
//! made latitudes and longitudes on a sphere (exact enough for the few
//! hundred kilometres a map spans).

use crate::auto::{Changes, Steps};
use crate::codec::snap_height;
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::heightmap::Heightmap;
use crate::history::Edit;
use crate::level::Blob;
use crate::plane::Plane;
use crate::selection::IN;
use crate::tile::{NO_FLUID, Tile};
use crate::world::World;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Pixels a side of a tile.
pub const TILE_PX: usize = 256;
/// The finest zoom of the tiles.
pub const MAX_ZOOM: u8 = 15;
/// The most tiles one area reads: a coarser zoom past that (about 256 MB
/// of heights).
pub const MAX_TILES: usize = 1024;
/// The most samples a side of the grid of heights an area is read into
/// (bilinear between them): as many as the most tiles have pixels.
pub const MAX_GRID: usize = 8192;
/// The widest and deepest map made from a real place, in blocks.
pub const MAX_SIZE: i32 = 32_768;
/// The Earth's radius, in metres (the sphere of Web Mercator).
pub const EARTH_RADIUS: f64 = 6_378_137.0;
/// Where the tiles are, by `{z}`, `{x}` and `{y}`.
pub const TERRARIUM_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";

/// The credit the tiles' sources ask for (Tilezen's `attribution.md`).
pub const ATTRIBUTION: &[&str] = &[
    "Terrain Tiles by Mapzen (Tilezen), hosted on AWS Open Data",
    "ArcticDEM terrain data DEM(s) were created from DigitalGlobe, Inc., imagery and funded under National Science Foundation awards 1043681, 1559691, and 1542736",
    "Australia terrain data © Commonwealth of Australia (Geoscience Australia) 2017",
    "Austria terrain data © offene Daten Österreichs – Digitales Geländemodell (DGM) Österreich",
    "Canada terrain data contains information licensed under the Open Government Licence – Canada",
    "Europe terrain data produced using Copernicus data and information funded by the European Union - EU-DEM layers",
    "Global ETOPO1 terrain data U.S. National Oceanic and Atmospheric Administration",
    "Mexico terrain data source: INEGI, Continental relief, 2016",
    "New Zealand terrain data Copyright 2011 Crown copyright (c) Land Information New Zealand and the New Zealand Government (All rights reserved)",
    "Norway terrain data © Kartverket",
    "United Kingdom terrain data © Environment Agency copyright and/or database right 2015. All rights reserved",
    "United States 3DEP (formerly NED) and global GMTED2010 and SRTM terrain data courtesy of the U.S. Geological Survey",
];

/// A tile of the Web Mercator grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub fn url(&self) -> String {
        TERRARIUM_URL.replace("{z}", &self.z.to_string()).replace("{x}", &self.x.to_string()).replace("{y}", &self.y.to_string())
    }
}

/// The heights of a tile, in metres, row by row from the north-west.
#[derive(Debug, Clone, PartialEq)]
pub struct DemTile {
    pub metres: Vec<f32>,
}

impl DemTile {
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.metres[y * TILE_PX + x]
    }
}

/// A Terrarium pixel's height: `R × 256 + G + B / 256 − 32768` metres.
pub fn terrarium(r: u8, g: u8, b: u8) -> f32 {
    r as f32 * 256.0 + g as f32 + b as f32 / 256.0 - 32768.0
}

/// The colour a height has in a Terrarium tile (the inverse of
/// [`terrarium`], to 1/256 m): for tests and fixtures.
pub fn terrarium_rgb(metres: f32) -> [u8; 3] {
    let v = ((metres + 32768.0) * 256.0).round().clamp(0.0, 16_777_215.0) as u32;
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Decode a Terrarium tile from its PNG.
pub fn decode_terrarium(png: &[u8]) -> Result<DemTile, String> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).map_err(|e| format!("not a tile: {e}"))?.into_rgb8();
    if img.width() as usize != TILE_PX || img.height() as usize != TILE_PX {
        return Err(format!("a tile of {} × {} pixels, not {TILE_PX}", img.width(), img.height()));
    }
    Ok(DemTile {
        metres: img.pixels().map(|p| terrarium(p[0], p[1], p[2])).collect(),
    })
}

/// Encode heights as a Terrarium PNG: for tests and fixtures.
pub fn encode_terrarium(metres: &[f32]) -> Vec<u8> {
    let rgb: Vec<u8> = metres.iter().flat_map(|&m| terrarium_rgb(m)).collect();
    let img = image::RgbImage::from_raw(TILE_PX as u32, TILE_PX as u32, rgb).expect("a whole tile");
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("PNG to memory");
    out.into_inner()
}

/// Pixels across the whole world at zoom `z`.
pub fn world_px(z: u8) -> f64 {
    TILE_PX as f64 * (1u64 << z) as f64
}

/// The latitudes Web Mercator reaches.
pub const MAX_LAT: f64 = 85.051_128_78;

/// The pixel of the world at zoom `z` a latitude and longitude fall on.
pub fn to_px(lat: f64, lon: f64, z: u8) -> (f64, f64) {
    let n = world_px(z);
    let phi = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// The latitude and longitude of pixel `(x, y)` of the world at zoom `z`.
pub fn from_px(x: f64, y: f64, z: u8) -> (f64, f64) {
    let n = world_px(z);
    let lon = x / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lat, lon)
}

/// Metres on the ground a pixel covers at latitude `lat` and zoom `z`.
pub fn metres_per_px(lat: f64, z: u8) -> f64 {
    2.0 * std::f64::consts::PI * EARTH_RADIUS * lat.to_radians().cos() / world_px(z)
}

/// A rectangle of `width` × `depth` blocks laid on the Earth, centred on
/// `lat`, `lon`, at `blocks_per_metre` blocks a metre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub lat: f64,
    pub lon: f64,
    pub blocks_per_metre: f64,
    pub width: i32,
    pub depth: i32,
}

/// South, west, north and east, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl Area {
    /// Metres east and south of the centre covered, from edge to edge.
    pub fn metres(&self) -> (f64, f64) {
        (self.width as f64 / self.blocks_per_metre, self.depth as f64 / self.blocks_per_metre)
    }

    /// The latitude and longitude of the point `east` and `south` metres
    /// from the centre.
    pub fn lat_lon(&self, east: f64, south: f64) -> (f64, f64) {
        let lat = self.lat - (south / EARTH_RADIUS).to_degrees();
        let lon = self.lon + (east / (EARTH_RADIUS * self.lat.to_radians().cos().max(1e-6))).to_degrees();
        (lat.clamp(-MAX_LAT, MAX_LAT), wrap_lon(lon))
    }

    /// Metres east and south of the centre a latitude and longitude are.
    pub fn offset(&self, lat: f64, lon: f64) -> (f64, f64) {
        let east = wrap_lon(lon - self.lon).to_radians() * EARTH_RADIUS * self.lat.to_radians().cos();
        let south = (self.lat - lat).to_radians() * EARTH_RADIUS;
        (east, south)
    }

    /// The area grown to whole tiles of the map (128 blocks), about the
    /// same centre: what a new world reads, so that its edge tiles have
    /// data of their own rather than their last column stretched.
    pub fn whole_tiles(&self) -> Area {
        let up = |n: i32| (n.max(1) + TILE_SIZE - 1) / TILE_SIZE * TILE_SIZE;
        Area { width: up(self.width), depth: up(self.depth), ..*self }
    }

    pub fn bounds(&self) -> Bounds {
        let (w, d) = self.metres();
        let (north, west) = self.lat_lon(-w / 2.0, -d / 2.0);
        let (south, east) = self.lat_lon(w / 2.0, d / 2.0);
        Bounds { south, west, north, east }
    }

    /// The area over `bounds`: its centre, and its size in blocks at its
    /// scale (`keep_size`: false) or its scale for its size (true, the
    /// width fitting the bounds' width).
    pub fn fit_bounds(&self, b: Bounds, keep_size: bool) -> Area {
        let (lat, lon) = ((b.south + b.north) / 2.0, b.west + wrap_lon(b.east - b.west).abs() / 2.0);
        let mut area = Area { lat, lon: wrap_lon(lon), ..*self };
        let east = wrap_lon(b.east - b.west).abs().to_radians() * EARTH_RADIUS * lat.to_radians().cos();
        let south = (b.north - b.south).abs().to_radians() * EARTH_RADIUS;
        if keep_size {
            area.blocks_per_metre = (self.width as f64 / east.max(1.0)).clamp(MIN_SCALE, MAX_SCALE);
        } else {
            area.width = (east * self.blocks_per_metre).round().max(16.0) as i32;
            area.depth = (south * self.blocks_per_metre).round().max(16.0) as i32;
        }
        area
    }

    /// The zoom to read the area at: the coarsest whose pixels are no
    /// larger than a block, at most [`MAX_ZOOM`], and coarser while it
    /// would take more than `max_tiles` tiles.
    pub fn zoom(&self, max_tiles: usize) -> u8 {
        let block = 1.0 / self.blocks_per_metre;
        let mut z = (0..=MAX_ZOOM).find(|&z| metres_per_px(self.lat, z) <= block).unwrap_or(MAX_ZOOM);
        while z > 0 && self.tiles(z).len() > max_tiles {
            z -= 1;
        }
        z
    }

    /// The tiles of zoom `z` the area reads (with a pixel's margin, for
    /// reading between pixels).
    pub fn tiles(&self, z: u8) -> Vec<TileKey> {
        let (w, d) = self.metres();
        let corners = [(-w / 2.0, -d / 2.0), (w / 2.0, -d / 2.0), (-w / 2.0, d / 2.0), (w / 2.0, d / 2.0)];
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let (cx, _) = to_px(self.lat, self.lon, z);
        let n = world_px(z);
        for (e, s) in corners {
            let (lat, lon) = self.lat_lon(e, s);
            let (mut x, y) = to_px(lat, lon, z);
            // Across the antimeridian: on the centre's side of it.
            if x - cx > n / 2.0 {
                x -= n;
            } else if cx - x > n / 2.0 {
                x += n;
            }
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let px = TILE_PX as f64;
        let side = 1i64 << z;
        let (tx0, tx1) = (((x0 - 1.0) / px).floor() as i64, ((x1 + 1.0) / px).floor() as i64);
        let (ty0, ty1) = ((((y0 - 1.0) / px).floor() as i64).max(0), (((y1 + 1.0) / px).floor() as i64).min(side - 1));
        let mut keys = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1.min(tx0 + side - 1) {
                keys.push(TileKey { z, x: tx.rem_euclid(side) as u32, y: ty as u32 });
            }
        }
        keys
    }
}

/// The scales a dialog offers, in blocks a metre.
pub const MIN_SCALE: f64 = 0.001;
pub const MAX_SCALE: f64 = 8.0;

fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Heights in metres, wherever an area's points are.
pub trait Elevation: Sync {
    /// The height at `east`, `south` metres from the area's centre; `None`
    /// where there is no data.
    fn metres(&self, area: &Area, east: f64, south: f64) -> Option<f32>;
}

/// Terrarium tiles of one zoom.
pub struct Tiles {
    pub z: u8,
    pub tiles: HashMap<TileKey, Arc<DemTile>>,
}

/// Heights no place on the Earth has, in metres: below the deepest trench
/// or above the highest summit, a Terrarium pixel there is no data.
pub const LOWEST: f32 = -11_500.0;
pub const HIGHEST: f32 = 9_000.0;
/// A pixel standing out of all eight around it by more than this, and more
/// than they differ among themselves, is a spike or a pit of the data.
pub const SPIKE: f32 = 25.0;

impl Tiles {
    /// The tiles, cleaned: see [`clean_tile`].
    pub fn new(z: u8, tiles: HashMap<TileKey, Arc<DemTile>>) -> Tiles {
        Tiles::patched(z, tiles, &std::collections::HashSet::new())
    }

    /// The tiles, cleaned, `patched` among them made from a coarser zoom's
    /// (see [`from_coarser`]): those are bent, edge by edge, to meet the
    /// tiles of their own beside them, so that no seam shows where they
    /// meet.
    pub fn patched(z: u8, tiles: HashMap<TileKey, Arc<DemTile>>, patched: &std::collections::HashSet<TileKey>) -> Tiles {
        let raw = Tiles { z, tiles };
        let cleaned: HashMap<TileKey, Arc<DemTile>> = raw.tiles.par_iter().map(|(k, t)| (*k, Arc::new(raw.cleaned(*k, t)))).collect();
        if patched.is_empty() {
            return Tiles { z, tiles: cleaned };
        }
        let clean = Tiles { z, tiles: cleaned };
        let met: Vec<(TileKey, Arc<DemTile>)> = patched.par_iter().filter_map(|k| clean.tiles.get(k).map(|t| (*k, Arc::new(clean.met(*k, t, patched))))).collect();
        let mut tiles = clean.tiles;
        tiles.extend(met);
        Tiles { z, tiles }
    }

    /// Tile `key` bent to meet its neighbours of their own (not in
    /// `patched`): the difference across each edge, carried into the tile
    /// less and less (a Coons patch of the four edges' differences).
    fn met(&self, key: TileKey, tile: &DemTile, patched: &std::collections::HashSet<TileKey>) -> DemTile {
        let n = TILE_PX;
        let side = 1u32 << self.z;
        let neighbour = |dx: i32, dy: i32| {
            let y = key.y as i64 + dy as i64;
            if y < 0 || y >= side as i64 {
                return None;
            }
            let k = TileKey { z: key.z, x: (key.x as i64 + dx as i64).rem_euclid(side as i64) as u32, y: y as u32 };
            if patched.contains(&k) { None } else { self.tiles.get(&k) }
        };
        let diff = |a: f32, b: f32| if a.is_finite() && b.is_finite() { a - b } else { f32::NAN };
        // North, south, west and east: the neighbour's pixel next to the
        // edge, less the tile's own on it.
        let edge = |dx: i32, dy: i32| -> Option<Vec<f32>> {
            match neighbour(dx, dy) {
                None => None,
                Some(t) => Some((0..n)
                    .map(|i| match (dx, dy) {
                        (0, -1) => diff(t.at(i, n - 1), tile.at(i, 0)),
                        (0, 1) => diff(t.at(i, 0), tile.at(i, n - 1)),
                        (-1, 0) => diff(t.at(n - 1, i), tile.at(0, i)),
                        _ => diff(t.at(0, i), tile.at(n - 1, i)),
                    })
                    .collect::<Vec<f32>>())
                .and_then(|d| {
                    // A neighbour without data along most of the edge
                    // says nothing; elsewhere, its gaps take the mean.
                    let had: Vec<f32> = d.iter().copied().filter(|v| v.is_finite()).collect();
                    (had.len() * 2 > n).then(|| {
                        let mean = had.iter().sum::<f32>() / had.len() as f32;
                        d.iter().map(|v| if v.is_finite() { *v } else { mean }).collect()
                    })
                }),
            }
        };
        let (north, south, west, east) = (edge(0, -1), edge(0, 1), edge(-1, 0), edge(1, 0));
        // An edge with no neighbour of its own goes from one corner to the
        // other as the edges beside it say, so that the corners agree.
        let end = |e: &Option<Vec<f32>>, last: bool| e.as_ref().map_or(0.0, |v| if last { v[n - 1] } else { v[0] });
        let across = |a: f32, b: f32| (0..n).map(|i| a + (b - a) * i as f32 / (n - 1) as f32).collect::<Vec<f32>>();
        let north = north.clone().unwrap_or_else(|| across(end(&west, false), end(&east, false)));
        let south = south.clone().unwrap_or_else(|| across(end(&west, true), end(&east, true)));
        let west = west.unwrap_or_else(|| across(north[0], south[0]));
        let east = east.unwrap_or_else(|| across(north[n - 1], south[n - 1]));
        let corner = |a: f32, b: f32| (a + b) / 2.0;
        let (p00, p10, p01, p11) = (corner(north[0], west[0]), corner(north[n - 1], east[0]), corner(south[0], west[n - 1]), corner(south[n - 1], east[n - 1]));
        let mut metres = tile.metres.clone();
        for y in 0..n {
            let v = y as f32 / (n - 1) as f32;
            for x in 0..n {
                let u = x as f32 / (n - 1) as f32;
                let c = (1.0 - v) * north[x] + v * south[x] + (1.0 - u) * west[y] + u * east[y] - ((1.0 - u) * (1.0 - v) * p00 + u * (1.0 - v) * p10 + (1.0 - u) * v * p01 + u * v * p11);
                metres[y * n + x] += c;
            }
        }
        DemTile { metres }
    }

    /// The size of the squares the low land near the sea comes in, in
    /// pixels (see [`crate::coast::square_size`]): the sources the tiles
    /// are made of are often coarser than their pixels, blown up square by
    /// square.
    pub fn square_px(&self) -> f32 {
        let mut runs = Vec::new();
        for t in self.tiles.values() {
            for row in t.metres.chunks(TILE_PX) {
                crate::coast::low_runs(row, &mut runs);
            }
        }
        crate::coast::median_run(&mut runs)
    }

    /// Pixels without data, over all the tiles.
    pub fn missing_px(&self) -> usize {
        self.tiles.values().map(|t| t.metres.iter().filter(|m| !m.is_finite()).count()).sum()
    }

    /// Tile `key` with its pixels out of reach of the Earth made no data,
    /// and its lone spikes and pits made the middle of the pixels around
    /// them (across the edges of the tiles, where the next ones are read).
    fn cleaned(&self, key: TileKey, tile: &DemTile) -> DemTile {
        let n = TILE_PX;
        let m = n + 2;
        let ok = |v: f32| (v.is_finite() && (LOWEST..=HIGHEST).contains(&v)).then_some(v);
        // The tile and a pixel's ring of its neighbours.
        let (bx, by) = (key.x as i64 * n as i64 - 1, key.y as i64 * n as i64 - 1);
        let padded: Vec<f32> = (0..m * m)
            .map(|k| {
                let (x, y) = (k % m, k / m);
                let v = if (1..=n).contains(&x) && (1..=n).contains(&y) { Some(tile.at(x - 1, y - 1)) } else { self.px(bx + x as i64, by + y as i64) };
                v.and_then(ok).unwrap_or(f32::NAN)
            })
            .collect();
        let mut metres = Vec::with_capacity(n * n);
        for y in 1..=n {
            for x in 1..=n {
                let v = padded[y * m + x];
                if !v.is_finite() {
                    metres.push(f32::NAN);
                    continue;
                }
                let mut around = [0f32; 8];
                let mut k = 0;
                for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let a = padded[(y as i32 + dy) as usize * m + (x as i32 + dx) as usize];
                    if a.is_finite() {
                        around[k] = a;
                        k += 1;
                    }
                }
                if k < 5 {
                    metres.push(v);
                    continue;
                }
                let around = &mut around[..k];
                around.sort_unstable_by(f32::total_cmp);
                let (lo, hi) = (around[0], around[k - 1]);
                let reach = SPIKE.max(hi - lo);
                metres.push(if v > hi + reach || v < lo - reach { around[k / 2] } else { v });
            }
        }
        DemTile { metres }
    }

    /// The height at pixel `(x, y)` of the world at the tiles' zoom.
    fn px(&self, x: i64, y: i64) -> Option<f32> {
        let side = 1i64 << self.z;
        let n = side * TILE_PX as i64;
        let (x, y) = (x.rem_euclid(n), y.clamp(0, n - 1));
        let p = TILE_PX as i64;
        let key = TileKey { z: self.z, x: (x / p) as u32, y: (y / p) as u32 };
        self.tiles.get(&key).map(|t| t.at((x % p) as usize, (y % p) as usize)).filter(|v| v.is_finite())
    }

    /// The height at a latitude and longitude, bilinear between pixels.
    pub fn at(&self, lat: f64, lon: f64) -> Option<f32> {
        let (x, y) = to_px(lat, lon, self.z);
        let (x, y) = (x - 0.5, y - 0.5);
        let (i, j) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = ((x - i as f64) as f32, (y - j as f64) as f32);
        let (a, b, c, d) = (self.px(i, j)?, self.px(i + 1, j)?, self.px(i, j + 1)?, self.px(i + 1, j + 1)?);
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        Some(top + (bottom - top) * fy)
    }
}

impl Elevation for Tiles {
    fn metres(&self, area: &Area, east: f64, south: f64) -> Option<f32> {
        let (lat, lon) = area.lat_lon(east, south);
        self.at(lat, lon)
    }
}

/// Tile `key` made from `parent`, a tile of a coarser zoom over it, its
/// pixels read between the parent's (bilinear): what stands for a tile that
/// could not be had.
pub fn from_coarser(parent: &DemTile, parent_key: TileKey, key: TileKey) -> DemTile {
    let shift = key.z.saturating_sub(parent_key.z) as u32;
    let scale = (1u32 << shift) as f64;
    let n = TILE_PX;
    // The tile's corner in the parent's pixels.
    let (ox, oy) = ((key.x as f64 - parent_key.x as f64 * scale) * n as f64 / scale, (key.y as f64 - parent_key.y as f64 * scale) * n as f64 / scale);
    let at = |x: i64, y: i64| parent.at(x.clamp(0, n as i64 - 1) as usize, y.clamp(0, n as i64 - 1) as usize);
    let metres = (0..n * n)
        .map(|k| {
            let (x, y) = (ox + ((k % n) as f64 + 0.5) / scale - 0.5, oy + ((k / n) as f64 + 0.5) / scale - 0.5);
            let (i, j) = (x.floor() as i64, y.floor() as i64);
            let (fx, fy) = ((x - i as f64) as f32, (y - j as f64) as f32);
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            let top = a + (b - a) * fx;
            let bottom = c + (d - c) * fx;
            top + (bottom - top) * fy
        })
        .collect();
    DemTile { metres }
}

/// The tile of zoom `z` over tile `key` (`z` no finer than its own).
pub fn parent_of(key: TileKey, z: u8) -> TileKey {
    let shift = key.z.saturating_sub(z);
    TileKey { z: key.z - shift, x: key.x >> shift, y: key.y >> shift }
}

/// A grid of heights read from a file: `metres_per_px` apart, its centre
/// on the area's.
pub struct Grid {
    pub map: Arc<Heightmap>,
    pub metres_per_px: f64,
    /// Heights the grid's 0 and 1 stand for (a GeoTIFF's are metres
    /// already: 0 and 1).
    pub low: f32,
    pub high: f32,
    /// Samples that say "no data".
    pub nodata: Option<f32>,
}

impl Elevation for Grid {
    fn metres(&self, _: &Area, east: f64, south: f64) -> Option<f32> {
        let (w, h) = (self.map.width as f64, self.map.height as f64);
        let u = east / self.metres_per_px + w / 2.0 - 0.5;
        let v = south / self.metres_per_px + h / 2.0 - 0.5;
        if u < -0.5 || v < -0.5 || u > w - 0.5 || v > h - 0.5 {
            return None;
        }
        if let Some(nodata) = self.nodata {
            let near = self.map.data[(v.round().clamp(0.0, h - 1.0) as usize) * self.map.width + u.round().clamp(0.0, w - 1.0) as usize];
            if near == nodata || !near.is_finite() {
                return None;
            }
        }
        let s = self.map.sample(u as f32, v as f32);
        s.is_finite().then_some(self.low + (self.high - self.low) * s)
    }
}

impl Grid {
    /// Metres the grid covers east and south.
    pub fn extent(&self) -> (f64, f64) {
        (self.map.width as f64 * self.metres_per_px, self.map.height as f64 * self.metres_per_px)
    }
}

/// How metres become heights of blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertical {
    /// The real height that stands at the map's sea level, in metres
    /// (0: the sea's).
    pub datum: f32,
    pub sea_level: i32,
    /// Blocks a metre upward, over those across.
    pub exaggeration: f32,
    /// The heights the ground keeps within, `low..high`.
    pub low: i32,
    pub high: i32,
    /// Squeeze what would go past the range into it (above the sea and
    /// under it apart, the sea staying where it is), rather than cut it.
    pub fit: bool,
}

/// The lowest and highest real heights of an area, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub low: f32,
    pub high: f32,
}

/// Metres to heights of blocks: see [`Vertical`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub datum: f32,
    pub sea: f32,
    /// Blocks a metre above the sea, and under it.
    pub up: f32,
    pub down: f32,
    pub low: f32,
    pub high: f32,
}

impl Vertical {
    /// The fit for an area of real heights `span`, at `blocks_per_metre`
    /// across.
    pub fn fit(&self, span: Span, blocks_per_metre: f64) -> Fit {
        let scale = (blocks_per_metre as f32 * self.exaggeration).max(1e-6);
        let sea = self.sea_level as f32;
        // A column stands on its block: the top of the ground at least a
        // block over the bottom, and a block under the top of the world.
        let (low, high) = ((self.low + 1) as f32, (self.high - 1) as f32);
        let (mut up, mut down) = (scale, scale);
        if self.fit {
            let over = (span.high - self.datum) * scale;
            if over > 0.0 && sea + over > high {
                up = ((high - sea) / (span.high - self.datum)).max(0.0);
            }
            let under = (self.datum - span.low) * scale;
            if under > 0.0 && sea - under < low {
                down = ((sea - low) / (self.datum - span.low)).max(0.0);
            }
        }
        Fit { datum: self.datum, sea, up, down, low, high }
    }
}

impl Fit {
    /// The height of the ground for a real height. The real sea (0 m or
    /// under) is a block under the water at least, where it would stand at
    /// the sea level: the data often gives the sea as 0 m flat, which would
    /// be dry ground level with the water.
    pub fn height(&self, metres: f32) -> f32 {
        let d = metres - self.datum;
        let mut y = self.sea + if d >= 0.0 { d * self.up } else { d * self.down };
        if metres <= 0.0 {
            y = y.min(self.sea - 1.0);
        }
        y.clamp(self.low, self.high.max(self.low))
    }
}

/// The real heights of an area read on a grid: `spacing` blocks apart,
/// `nx` × `nz` samples from its north-west corner, in metres (`NaN` where
/// there is no data).
pub struct Samples {
    pub spacing: f64,
    pub nx: usize,
    pub nz: usize,
    pub metres: Vec<f32>,
    /// The lakes found in the heights (see [`crate::coast::find_lakes`]).
    pub lakes: crate::coast::Lakes,
}

impl Samples {
    /// Read `area` from `source` every `spacing` blocks.
    pub fn read(area: &Area, source: &dyn Elevation, spacing: f64) -> Samples {
        let spacing = spacing.max(1e-3);
        let nx = ((area.width as f64 / spacing).ceil() as usize).max(2);
        let nz = ((area.depth as f64 / spacing).ceil() as usize).max(2);
        let (w, d) = (area.width as f64, area.depth as f64);
        let metres = (0..nx * nz)
            .into_par_iter()
            .map(|k| {
                let (i, j) = (k % nx, k / nx);
                let (bx, bz) = ((i as f64 + 0.5) * spacing - w / 2.0, (j as f64 + 0.5) * spacing - d / 2.0);
                source.metres(area, bx / area.blocks_per_metre, bz / area.blocks_per_metre).unwrap_or(f32::NAN)
            })
            .collect();
        Samples { spacing, nx, nz, metres, lakes: Default::default() }
    }

    /// The lowest and highest real heights read, `None` when none was.
    pub fn span(&self) -> Option<Span> {
        let (lo, hi) = self.metres.iter().filter(|m| m.is_finite()).fold((f32::MAX, f32::MIN), |(lo, hi), &m| (lo.min(m), hi.max(m)));
        (lo <= hi).then_some(Span { low: lo, high: hi })
    }

    /// Give the samples without data the heights around them (coarser
    /// and coarser averages of the samples that have some, pulled back
    /// over the gaps), rather than a hole: nothing changes when every
    /// sample has data, and nothing when none has.
    pub fn fill_gaps(&mut self) {
        if self.missing() == 0 || self.missing() == self.metres.len() {
            return;
        }
        let holes: Vec<bool> = self.metres.iter().map(|m| !m.is_finite()).collect();
        fill_grid(&mut self.metres, self.nx, self.nz);
        relax(&mut self.metres, &holes, self.nx, self.nz, RELAX);
    }

    /// Make the samples ready to be ground: the gaps given the heights
    /// around them ([`Samples::fill_gaps`]), and the coasts shaped (see
    /// [`crate::coast`]), the area at `blocks_per_metre`.
    ///
    /// `square_metres`: the size of the squares the data comes in, when its
    /// source says (see [`Tiles::square_px`]).
    pub fn prepare(&mut self, blocks_per_metre: f64, square_metres: Option<f64>) -> Prepared {
        let missing = self.missing();
        self.fill_gaps();
        let mps = self.spacing / blocks_per_metre.max(1e-9);
        let square = square_metres.map(|m| (m / mps) as f32);
        // Lakes found in the heights as they came, before the coasts are
        // shaped.
        self.lakes = crate::coast::find_lakes(&self.metres, self.nx, self.nz, mps);
        let coast = crate::coast::shape(&mut self.metres, self.nx, self.nz, mps, square);
        crate::coast::dig_lakes(&mut self.metres, self.nx, self.nz, mps, &self.lakes);
        Prepared { missing, coast, lakes: self.lakes.levels.len() }
    }

    /// The level of the lake at `(bx, bz)` blocks from the area's
    /// north-west corner, in metres: the nearest sample's.
    pub fn lake_at(&self, bx: f64, bz: f64) -> Option<f32> {
        if self.lakes.levels.is_empty() {
            return None;
        }
        let i = ((bx / self.spacing).floor().max(0.0) as usize).min(self.nx - 1);
        let j = ((bz / self.spacing).floor().max(0.0) as usize).min(self.nz - 1);
        self.lakes.level(j * self.nx + i)
    }

    /// Samples without data.
    pub fn missing(&self) -> usize {
        self.metres.iter().filter(|m| !m.is_finite()).count()
    }

    /// The real height at `(bx, bz)` blocks from the area's north-west
    /// corner, bilinear; `None` where there is no data.
    pub fn at(&self, bx: f64, bz: f64) -> Option<f32> {
        let u = (bx / self.spacing - 0.5).clamp(0.0, (self.nx - 1) as f64);
        let v = (bz / self.spacing - 0.5).clamp(0.0, (self.nz - 1) as f64);
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (i1, j1) = ((i + 1).min(self.nx - 1), (j + 1).min(self.nz - 1));
        let (fu, fv) = ((u - i as f64) as f32, (v - j as f64) as f32);
        let at = |i: usize, j: usize| self.metres[j * self.nx + i];
        let (a, b, c, d) = (at(i, j), at(i1, j), at(i, j1), at(i1, j1));
        if [a, b, c, d].iter().all(|m| m.is_finite()) {
            let top = a + (b - a) * fu;
            let bottom = c + (d - c) * fu;
            Some(top + (bottom - top) * fv)
        } else {
            // At the edge of the data: the nearest sample that has some.
            let near = at(u.round() as usize, v.round() as usize);
            near.is_finite().then_some(near)
        }
    }
}

/// What [`Samples::prepare`] found.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Prepared {
    /// Samples without data, filled.
    pub missing: usize,
    pub coast: crate::coast::CoastReport,
    /// Lakes found.
    pub lakes: usize,
}

/// The biomes of the sea at latitude `lat`, shallow and deep: warm in the
/// tropics, cold toward the poles (as the game's oceans are).
pub fn sea_biomes(lat: f64) -> (&'static str, &'static str) {
    match lat.abs() {
        l if l < 23.5 => ("minecraft:warm_ocean", "minecraft:deep_lukewarm_ocean"),
        l if l < 40.0 => ("minecraft:lukewarm_ocean", "minecraft:deep_lukewarm_ocean"),
        l if l < 58.0 => ("minecraft:ocean", "minecraft:deep_ocean"),
        l if l < 68.0 => ("minecraft:cold_ocean", "minecraft:deep_cold_ocean"),
        _ => ("minecraft:frozen_ocean", "minecraft:deep_frozen_ocean"),
    }
}

/// The automatic rules for a real place at latitude `lat`: its sea's
/// biomes by [`sea_biomes`], where the rules keep the game's plain ocean.
pub fn rules_at(rules: &crate::auto::AutoRules, lat: f64) -> crate::auto::AutoRules {
    use crate::auto::Land;
    let mut rules = rules.clone();
    let (shallow, deep) = sea_biomes(lat);
    for (land, biome) in [(Land::Water, shallow), (Land::DeepWater, deep)] {
        if rules.land.biome_of(land) == land.default_biome() {
            rules.land.set_biome(land, biome);
        }
    }
    rules
}

/// Fill the `NaN`s of a `w` × `h` grid by push-pull: the grid halved
/// (each cell the mean of the finite cells under it), filled the same way,
/// and its cells read back, bilinear, where this one has none.
pub fn fill_grid(grid: &mut [f32], w: usize, h: usize) {
    if w <= 1 && h <= 1 {
        if !grid[0].is_finite() {
            grid[0] = 0.0;
        }
        return;
    }
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    let mut half = vec![f32::NAN; hw * hh];
    for j in 0..hh {
        for i in 0..hw {
            let (mut sum, mut n) = (0.0, 0);
            for (x, z) in [(2 * i, 2 * j), (2 * i + 1, 2 * j), (2 * i, 2 * j + 1), (2 * i + 1, 2 * j + 1)] {
                if x < w && z < h && grid[z * w + x].is_finite() {
                    sum += grid[z * w + x];
                    n += 1;
                }
            }
            if n > 0 {
                half[j * hw + i] = sum / n as f32;
            }
        }
    }
    if half.iter().any(|m| !m.is_finite()) {
        fill_grid(&mut half, hw, hh);
    }
    for z in 0..h {
        for x in 0..w {
            if grid[z * w + x].is_finite() {
                continue;
            }
            // The cell's centre on the half grid.
            let (u, v) = (((x as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hw - 1) as f32), ((z as f32 + 0.5) / 2.0 - 0.5).clamp(0.0, (hh - 1) as f32));
            let (i, j) = (u.floor() as usize, v.floor() as usize);
            let (i1, j1) = ((i + 1).min(hw - 1), (j + 1).min(hh - 1));
            let (fu, fv) = (u - i as f32, v - j as f32);
            let at = |i: usize, j: usize| half[j * hw + i];
            let top = at(i, j) + (at(i1, j) - at(i, j)) * fu;
            let bottom = at(i, j1) + (at(i1, j1) - at(i, j1)) * fu;
            grid[z * w + x] = top + (bottom - top) * fv;
        }
    }
}

/// Rounds of [`relax`] over the gaps filled.
const RELAX: usize = 96;

/// Smooth the cells of `holes` of a `w` × `h` grid toward the mean of
/// their four neighbours (over-relaxed, every other cell at a time), the
/// others held: a gap filled by [`fill_grid`] meets the data around it with
/// no step at its edge.
pub fn relax(grid: &mut [f32], holes: &[bool], w: usize, h: usize, rounds: usize) {
    // The cells, row by row: a row's cells of one parity read only cells of
    // the other, so the rows are worked at once.
    let rows: Vec<Vec<u32>> = (0..h).into_par_iter().map(|z| (0..w).filter(|&x| holes[z * w + x]).map(|x| x as u32).collect()).collect();
    if rows.iter().all(|r| r.is_empty()) {
        return;
    }
    const OMEGA: f32 = 1.85;
    struct Cells(*mut f32);
    unsafe impl Send for Cells {}
    unsafe impl Sync for Cells {}
    let cells = Cells(grid.as_mut_ptr());
    let cells = &cells;
    for _ in 0..rounds {
        for parity in 0..2 {
            rows.par_iter().enumerate().for_each(|(z, row)| {
                let g = cells.0;
                for &x in row {
                    let x = x as usize;
                    if (x + z) % 2 != parity {
                        continue;
                    }
                    let k = z * w + x;
                    // SAFETY: this pass writes only cells of its parity and
                    // reads only cells of the other; each cell is in one row.
                    unsafe {
                        let (mut sum, mut n) = (0.0, 0.0);
                        if x > 0 {
                            sum += *g.add(k - 1);
                            n += 1.0;
                        }
                        if x + 1 < w {
                            sum += *g.add(k + 1);
                            n += 1.0;
                        }
                        if z > 0 {
                            sum += *g.add(k - w);
                            n += 1.0;
                        }
                        if z + 1 < h {
                            sum += *g.add(k + w);
                            n += 1.0;
                        }
                        let v = g.add(k);
                        *v += OMEGA * (sum / n - *v);
                    }
                }
            });
        }
    }
}

/// How far apart to read an area for a map: a sample a block, or as far
/// apart as the data's pixels are, and no more than [`MAX_GRID`] a side.
pub fn spacing(area: &Area, data_metres: f64) -> f64 {
    let data = (data_metres * area.blocks_per_metre).floor().max(1.0);
    let most = (area.width.max(area.depth) as f64 / MAX_GRID as f64).ceil().max(1.0);
    data.max(most)
}

/// How far from the water a beach goes, in metres.
pub const BEACH_METRES: f64 = 30.0;

/// About where the snow stays all year at latitude `lat`, in metres: some
/// 5200 m in the tropics, lower toward the poles (some 3000 m in the Alps,
/// 1800 m at 60°, the sea's level past 80°).
pub fn snow_line(lat: f64) -> f32 {
    (5200.0 - 85.0 * (lat.abs() - 20.0).max(0.0)).max(0.0) as f32
}

/// The ground a real place gives its land, from its real heights: gravel
/// deep under the water, sand only along the water (as far as a beach goes
/// on the Earth, about 30 m, and no more than 12 blocks), rock on steep slopes, snow above the
/// [`snow_line`] of its latitude and stony ground under it, grass
/// elsewhere. The samples' north-west corner lies at block `origin`.
struct Ground<'a> {
    samples: &'a Samples,
    fit: &'a Fit,
    theme: crate::generate::Theme,
    sea: f32,
    snow: f32,
    origin: (i32, i32),
    /// The heights a column may take, in blocks.
    limits: (f32, f32),
    /// Whether water is near each sample.
    shore: Vec<bool>,
}

impl<'a> Ground<'a> {
    fn new(samples: &'a Samples, fit: &'a Fit, area: &Area, theme: crate::generate::Theme, sea: f32, origin: (i32, i32), limits: (f32, f32)) -> Self {
        use crate::generate::{BEACH_REACH, within_reach};
        let wet: Vec<bool> = samples.metres.iter().map(|&m| m.is_finite() && fit.height(m) < sea).collect();
        let blocks = (BEACH_METRES * area.blocks_per_metre).clamp(2.0, 2.0 * BEACH_REACH as f64);
        let reach = (blocks / samples.spacing).ceil().max(1.0) as usize;
        let shore = within_reach(&wet, samples.nx, reach);
        Self { samples, fit, theme, sea, snow: snow_line(area.lat), origin, limits, shore }
    }

    /// The real height of column `(bx, bz)` (`f32::MIN` where there is no
    /// data), and its height in blocks.
    fn column(&self, bx: i32, bz: i32) -> (f32, f32) {
        let metres = self.samples.at((bx - self.origin.0) as f64 + 0.5, (bz - self.origin.1) as f64 + 0.5);
        let mut h = metres.map_or(self.fit.sea - 1.0, |m| self.fit.height(m)).clamp(self.limits.0, self.limits.1);
        if let Some(level) = self.lake(bx, bz) {
            h = h.min(level as f32 - 1.0);
        }
        (metres.unwrap_or(f32::MIN), h)
    }

    /// The water level of the lake column `(bx, bz)` is in, in blocks.
    fn lake(&self, bx: i32, bz: i32) -> Option<i16> {
        let level = self.samples.lake_at((bx - self.origin.0) as f64 + 0.5, (bz - self.origin.1) as f64 + 0.5)?;
        Some(lake_level(self.fit, level))
    }

    fn near_water(&self, bx: i32, bz: i32) -> bool {
        let s = self.samples;
        let (i, j) = ((((bx - self.origin.0) as f64 + 0.5) / s.spacing) as usize, (((bz - self.origin.1) as f64 + 0.5) / s.spacing) as usize);
        self.shore[j.min(s.nz - 1) * s.nx + i.min(s.nx - 1)]
    }

    /// The terrain of column `(bx, bz)`, `metres` high on the Earth and `h`
    /// in blocks, its slope `slope` blocks a block.
    fn terrain(&self, bx: i32, bz: i32, metres: f32, h: f32, slope: f32) -> crate::registry::TerrainId {
        let t = &self.theme;
        if h < self.sea - 14.0 {
            t.gravel
        } else if h < self.sea + 2.2 && slope < 0.9 && self.near_water(bx, bz) {
            t.sand
        } else if slope > 1.35 {
            t.rock
        } else if metres > self.snow {
            t.snow
        } else if metres > self.snow - 300.0 || slope > 1.05 {
            t.rocky
        } else {
            t.grass
        }
    }
}

/// The surface of a lake `metres` high, in blocks: the water's level.
pub fn lake_level(fit: &Fit, metres: f32) -> i16 {
    fit.height(metres).round().max(fit.sea) as i16
}

/// A new world of the area: its ground at the heights `fit` gives, the sea
/// at its level, the world's build limits `min_y..max_y`. The map is whole
/// tiles, its north-west corner on the area's (read the area grown with
/// [`Area::whole_tiles`], so that every column has data of its own), and
/// centred on the origin. Its terrains are painted from the real place:
/// sand only along the water, rock on steep slopes, snow above the
/// [`snow_line`] of its latitude (not by the heights of blocks, which the
/// scale and the exaggeration move); the caller paints them again with the
/// automatic paint.
pub fn new_world(name: &str, samples: &Samples, fit: &Fit, area: &Area, sea_level: i32, min_y: i32, max_y: i32) -> World {
    use crate::generate::{Theme, build_limits};
    use crate::world::WorldMeta;
    let mut world = World::new(WorldMeta {
        name: name.to_owned(),
        seed: 0,
        spawn: [0, 0, 0],
        sea_level,
        lava: false,
        graph: None,
        ores: Default::default(),
        gradients: Vec::new(),
        fill: None,
        paint_rules: None,
        palette: Default::default(),
        smooth: Default::default(),
        ..Default::default()
    });
    let (bottom, ceiling) = build_limits(min_y, max_y);
    world.dims[0].min_y = bottom;
    world.dims[0].max_y = ceiling;
    let ts = TILE_SIZE as f64;
    let (w, d) = (samples.nx as f64 * samples.spacing, samples.nz as f64 * samples.spacing);
    let (tx, tz) = (((w / ts).floor() as i32).max(1), ((d / ts).floor() as i32).max(1));
    let (x0, z0) = (-(tx / 2) * TILE_SIZE, -(tz / 2) * TILE_SIZE);
    let ground = Ground::new(samples, fit, area, Theme::new(&world.registry), sea_level as f32, (x0, z0), (bottom as f32 + 1.0, ceiling as f32 - 1.0));
    let column = |bx: i32, bz: i32| ground.column(bx, bz);
    let positions: Vec<TilePos> = (0..tz).flat_map(|z| (0..tx).map(move |x| TilePos::new(x0.div_euclid(TILE_SIZE) + x, z0.div_euclid(TILE_SIZE) + z))).collect();
    let blobs: Vec<(TilePos, Blob)> = positions
        .par_iter()
        .map(|&pos| {
            let (ox, oz) = pos.origin();
            let n = TILE_SIZE as usize;
            // The tile's heights with a column's margin, for the slopes.
            let m = n + 2;
            let columns: Vec<(f32, f32)> = (0..m * m).map(|k| column(ox - 1 + (k % m) as i32, oz - 1 + (k / m) as i32)).collect();
            let mut top = vec![0f32; n * n];
            let mut terrain = vec![0u16; n * n];
            let mut fluid = vec![sea_level as i16; n * n];
            for z in 0..n {
                for x in 0..n {
                    let at = |dx: usize, dz: usize| columns[(z + dz) * m + x + dx].1;
                    let (metres, h) = columns[(z + 1) * m + x + 1];
                    let slope = (at(2, 1) - at(0, 1)).abs().max((at(1, 2) - at(1, 0)).abs()) * 0.5;
                    top[z * n + x] = snap_height(h);
                    terrain[z * n + x] = ground.terrain(ox + x as i32, oz + z as i32, metres, h, slope).0;
                    if let Some(level) = ground.lake(ox + x as i32, oz + z as i32) {
                        fluid[z * n + x] = level;
                    }
                }
            }
            let mut tile = Tile::ground(0.0, 0, sea_level as i16);
            tile.top = Plane::from_vec(top);
            tile.terrain = Plane::from_vec(terrain);
            tile.fluid = Plane::from_vec(fluid);
            tile.compact();
            (pos, Blob::from(tile.encode()))
        })
        .collect();
    let ground = world.dims[0].ground_mut();
    for (pos, blob) in blobs {
        ground.insert_blob(pos, blob);
    }
    let surface = column(0, 0).1.max(sea_level as f32);
    world.meta.spawn = [0, crate::compose::block_top(surface) + 1, 0];
    world
}

/// Write the area's heights into the ground of the map over `rect` (the
/// area's north-west corner on its own): the tiles of the selection when
/// there is one, weighed by it, else those of the map in `rect`. Columns
/// under the sea level get the sea, those over it lose their water; where
/// there is no data the map stays as it is. With `repaint` (the area, and
/// the terrains of the map), the land is painted again from the real
/// place as a new world's is (see [`Ground`]), its painted biomes, ground
/// covers and object layers taken off, so that the automatic paint reads
/// the new land rather than what the map had: where the selection lets
/// it. Stops between batches when `steps` is cancelled.
pub fn write_ground(edit: &mut Edit, samples: &Samples, fit: &Fit, rect: BlockRect, repaint: Option<(&Area, crate::generate::Theme)>, steps: &Steps) -> Changes {
    const BATCH: usize = 256;
    const KEEP_HOT: usize = 2048;
    let selection = edit.dim().selection.clone();
    let positions: Vec<TilePos> = edit
        .level()
        .positions()
        .filter(|p| !rect.intersect(&p.rect()).is_empty())
        .filter(|p| selection.as_ref().is_none_or(|s| s.tile(*p).is_some()))
        .collect();
    steps.total.store(positions.len().max(1), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let (min_y, max_y) = (edit.dim().min_y as f32 + 1.0, (edit.dim().max_y - 1) as f32);
    let sea = fit.sea.round() as i16;
    let ground = repaint.map(|(area, theme)| Ground::new(samples, fit, area, theme, fit.sea, (rect.x0, rect.z0), (min_y, max_y)));
    let changed = AtomicUsize::new(0);
    let mut touched = Vec::new();
    let mut cancelled = false;
    for batch in positions.chunks(BATCH) {
        if steps.cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        edit.warm(batch);
        let edited = Mutex::new(Vec::new());
        edit.par_tiles(batch.iter().copied(), rect, false, |pos, clip, tile| {
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            // The new heights of the clip with a column's margin, for the
            // slopes the paint reads.
            let (m, heights) = match &ground {
                Some(g) => {
                    let m = (clip.width() + 2) as usize;
                    let rows = (clip.depth() + 2) as usize;
                    (m, (0..m * rows).map(|k| g.column(clip.x0 - 1 + (k % m) as i32, clip.z0 - 1 + (k / m) as i32)).collect())
                }
                None => (0, Vec::new()),
            };
            let mut n = 0;
            for z in clip.z0..clip.z1 {
                for x in clip.x0..clip.x1 {
                    let i = local_index(x, z);
                    let weight = cells.map_or(IN, |c| c.get(i));
                    if weight == 0 {
                        continue;
                    }
                    let Some(metres) = samples.at((x - rect.x0) as f64 + 0.5, (z - rect.z0) as f64 + 0.5) else { continue };
                    let lake = samples.lake_at((x - rect.x0) as f64 + 0.5, (z - rect.z0) as f64 + 0.5).map(|m| lake_level(fit, m));
                    let target = match lake {
                        Some(level) => fit.height(metres).min(level as f32 - 1.0),
                        None => fit.height(metres),
                    };
                    let old = tile.top.get(i);
                    let new = if weight == IN { target } else { old + (target - old) * weight as f32 / IN as f32 };
                    let new = snap_height(new.clamp(min_y, max_y));
                    let mut moved = false;
                    if (new - old).abs() > 1e-3 {
                        tile.top.set(i, new);
                        moved = true;
                    }
                    if crate::selection::lets(weight, x, z) {
                        let fluid = match lake {
                            Some(level) if new < level as f32 => level,
                            _ if new < fit.sea => sea,
                            _ => NO_FLUID,
                        };
                        if tile.fluid.get(i) != fluid {
                            tile.fluid.set(i, fluid);
                            moved = true;
                        }
                        if let Some(g) = &ground {
                            let (cx, cz) = ((x - clip.x0 + 1) as usize, (z - clip.z0 + 1) as usize);
                            let at = |dx: usize, dz: usize| heights[(cz + dz - 1) * m + cx + dx - 1].1;
                            let slope = (at(2, 1) - at(0, 1)).abs().max((at(1, 2) - at(1, 0)).abs()) * 0.5;
                            let terrain = g.terrain(x, z, metres, new, slope).0;
                            for (plane, value) in [(&mut tile.terrain, terrain), (&mut tile.biome, 0), (&mut tile.cover, 0), (&mut tile.objects, 0)] {
                                if plane.get(i) != value {
                                    plane.set(i, value);
                                    moved = true;
                                }
                            }
                        }
                    }
                    n += moved as usize;
                }
            }
            if n > 0 {
                tile.compact();
                changed.fetch_add(n, Ordering::Relaxed);
                edited.lock().expect("no panic while held").push(pos);
            }
            steps.done.fetch_add(1, Ordering::Relaxed);
        });
        touched.extend(edited.into_inner().expect("no panic while held"));
        edit.cool(KEEP_HOT);
    }
    let _ = TILE_SIZE;
    Changes {
        columns: changed.into_inner(),
        tiles: touched,
        cancelled,
    }
}

/// A GeoTIFF's heights, and how far apart its pixels are when it says
/// (metres; degrees made metres at its middle latitude).
pub struct Tiff {
    pub map: Heightmap,
    pub metres_per_px: Option<f64>,
    pub nodata: Option<f32>,
    /// Its middle's latitude and longitude, when it says.
    pub centre: Option<(f64, f64)>,
}

/// Read a GeoTIFF (or any one-channel TIFF) of heights: 8, 16 or 32-bit
/// integers or floats, the values kept as they are (metres).
pub fn read_tiff(path: &std::path::Path) -> Result<Tiff, String> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::Tag;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = Decoder::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let (w, h) = dec.dimensions().map_err(|e| e.to_string())?;
    let scale = dec.get_tag_f64_vec(Tag::Unknown(33550)).ok();
    let tie = dec.get_tag_f64_vec(Tag::Unknown(33922)).ok();
    let nodata = dec.get_tag_ascii_string(Tag::Unknown(42113)).ok().and_then(|s| s.trim().trim_end_matches('\0').parse::<f32>().ok());
    // Geographic (degrees) unless its keys say projected: key 1024, the
    // model's type, 1 projected and 2 geographic.
    let keys = dec.get_tag_u16_vec(Tag::Unknown(34735)).ok();
    let model = keys.as_ref().and_then(|k| k.as_chunks::<4>().0.iter().skip(1).find(|e| e[0] == 1024).map(|e| e[3]));
    let geographic = model != Some(1);
    let image = dec.read_image().map_err(|e| e.to_string())?;
    let n = w as usize * h as usize;
    let data: Vec<f32> = match image {
        DecodingResult::U8(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::U16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::U32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I8(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I32(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => return Err("a TIFF of a kind of samples not read".into()),
    };
    if data.len() < n {
        return Err("a TIFF of more than one channel: heights are one".into());
    }
    // One channel: every `len / n`-th sample.
    let step = data.len() / n;
    let data: Vec<f32> = if step == 1 { data } else { data.into_iter().step_by(step).collect() };
    let mut centre = None;
    let mut metres_per_px = None;
    if let Some(s) = &scale
        && s.len() >= 2
        && s[0] > 0.0
    {
        if let Some(t) = &tie
            && t.len() >= 6
        {
            let (x, y) = (t[3] + (w as f64 / 2.0 - t[0]) * s[0], t[4] - (h as f64 / 2.0 - t[1]) * s[1]);
            if geographic {
                centre = Some((y, x));
            }
        }
        metres_per_px = Some(if geographic {
            let lat = centre.map_or(0.0, |c| c.0);
            s[0].to_radians() * EARTH_RADIUS * lat.to_radians().cos()
        } else {
            s[0]
        });
    }
    Ok(Tiff {
        map: Heightmap {
            width: w as usize,
            height: h as usize,
            data,
        },
        metres_per_px,
        nodata,
        centre,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_pixels_decode() {
        assert_eq!(terrarium(128, 0, 0), 0.0);
        assert_eq!(terrarium(128, 1, 128), 1.5);
        assert_eq!(terrarium(127, 255, 0), -1.0);
        for m in [-10_994.0, -1.25, 0.0, 8848.5, 4808.75] {
            let [r, g, b] = terrarium_rgb(m);
            assert!((terrarium(r, g, b) - m).abs() < 1.0 / 256.0, "{m}");
        }
        // A whole tile, out as PNG and back.
        let metres: Vec<f32> = (0..TILE_PX * TILE_PX).map(|k| (k % TILE_PX) as f32 * 10.0 - (k / TILE_PX) as f32 * 3.5).collect();
        let tile = decode_terrarium(&encode_terrarium(&metres)).unwrap();
        assert_eq!(tile.at(255, 0), 2550.0);
        assert_eq!(tile.at(0, 10), -35.0);
        assert!(decode_terrarium(b"not a png").is_err());
    }

    #[test]
    fn mercator_goes_there_and_back() {
        assert_eq!(to_px(0.0, 0.0, 0), (128.0, 128.0));
        let (x, y) = to_px(45.8326, 6.8652, 12);
        let (lat, lon) = from_px(x, y, 12);
        assert!((lat - 45.8326).abs() < 1e-9 && (lon - 6.8652).abs() < 1e-9);
        // The top of the grid is about 85.05°.
        assert!((from_px(0.0, 0.0, 3).0 - MAX_LAT).abs() < 1e-6);
        // About 156 km a pixel at zoom 0 on the equator, half that at 60°.
        assert!((metres_per_px(0.0, 0) - 156_543.03).abs() < 0.1);
        assert!((metres_per_px(60.0, 0) - 78_271.5).abs() < 0.1);
    }

    #[test]
    fn an_area_on_the_earth() {
        let area = Area { lat: 45.0, lon: 6.0, blocks_per_metre: 1.0, width: 2000, depth: 1000 };
        let b = area.bounds();
        // 500 m north: 500 / 6378137 rad ≈ 0.00449°.
        assert!((b.north - 45.0 - 0.004_492).abs() < 1e-5, "{b:?}");
        assert!((b.east - 6.0 - 0.012_704).abs() < 1e-5, "{b:?}");
        let (e, s) = area.offset(b.south, b.east);
        assert!((e - 1000.0).abs() < 0.01 && (s - 500.0).abs() < 0.01);
        // Bounds back to an area: the same, at the scale or the size.
        let same = area.fit_bounds(b, false);
        assert!((same.width - 2000).abs() <= 1 && (same.depth - 1000).abs() <= 1 && (same.lat - 45.0).abs() < 1e-9);
        let scaled = Area { width: 4000, ..area }.fit_bounds(b, true);
        assert!((scaled.blocks_per_metre - 2.0).abs() < 1e-3, "{scaled:?}");
        // A block a metre wants zoom 15 at 45° (3.4 m a pixel at 15: the
        // finest there is).
        assert_eq!(area.zoom(MAX_TILES), MAX_ZOOM);
        // A block every 30 m: zoom 12 (27.6 m a pixel).
        let coarse = Area { blocks_per_metre: 1.0 / 30.0, ..area };
        assert_eq!(coarse.zoom(MAX_TILES), 12);
        let tiles = coarse.tiles(12);
        // 60 × 30 km in tiles of about 7 km: 10 × 5.
        assert_eq!(tiles.len(), 50);
        // Too many tiles: coarser.
        let huge = Area { width: 30_000, depth: 30_000, ..area };
        let z = huge.zoom(64);
        assert!(z < MAX_ZOOM && huge.tiles(z).len() <= 64);
        // Across the antimeridian, tiles of both sides.
        let fiji = Area { lat: -17.0, lon: 179.99, blocks_per_metre: 0.05, width: 2000, depth: 2000 };
        let xs: Vec<u32> = fiji.tiles(8).iter().map(|k| k.x).collect();
        assert!(xs.contains(&0) && xs.contains(&255), "{xs:?}");
    }

    /// Tiles of a slope rising 1 m a pixel eastward, around `area`.
    fn ramp(area: &Area, z: u8) -> Tiles {
        let tiles = area
            .tiles(z)
            .into_iter()
            .map(|k| {
                let metres = (0..TILE_PX * TILE_PX).map(|i| (k.x as usize * TILE_PX + i % TILE_PX) as f32 - 100_000.0).collect();
                (k, Arc::new(DemTile { metres }))
            })
            .collect();
        Tiles { z, tiles }
    }

    #[test]
    fn tiles_are_read_between_pixels() {
        let area = Area { lat: 10.0, lon: 20.0, blocks_per_metre: 0.2, width: 512, depth: 512 };
        let z = 12;
        let tiles = ramp(&area, z);
        let (x, _) = to_px(10.0, 20.0, z);
        let at = tiles.metres(&area, 0.0, 0.0).unwrap();
        assert!((at - (x - 0.5) as f32 + 100_000.0).abs() < 0.05, "{at} at pixel {x}");
        // A pixel east, a metre higher.
        let east = tiles.metres(&area, metres_per_px(10.0, z), 0.0).unwrap();
        assert!((east - at - 1.0).abs() < 0.02, "{east} {at}");
        // Off the tiles read: no data.
        assert_eq!(tiles.metres(&area, 1.0e6, 0.0), None);
        let samples = Samples::read(&area, &tiles, 4.0);
        assert_eq!((samples.nx, samples.nz), (128, 128));
        assert_eq!(samples.missing(), 0);
        let span = samples.span().unwrap();
        // 512 blocks at 0.2 a metre: 2560 m, 2560 / 36.6 m ≈ 70 pixels.
        let wide = 2560.0 / metres_per_px(10.0, z) as f32;
        assert!((span.high - span.low - wide).abs() < 2.0, "{span:?} {wide}");
    }

    #[test]
    fn spikes_pits_and_holes_are_cleaned() {
        // Two tiles side by side of gentle ground, 1 m a pixel eastward.
        let z = 10;
        let (a, b) = (TileKey { z, x: 500, y: 300 }, TileKey { z, x: 501, y: 300 });
        let ground = |k: TileKey| (0..TILE_PX * TILE_PX).map(|i| ((k.x as usize - 500) * TILE_PX + i % TILE_PX) as f32 * 0.5).collect::<Vec<f32>>();
        let (mut ma, mut mb) = (ground(a), ground(b));
        let base = |k: TileKey, x: usize| ((k.x as usize - 500) * TILE_PX + x) as f32 * 0.5;
        ma[40 * TILE_PX + 40] += 900.0; // A spike,
        ma[80 * TILE_PX + 90] -= 300.0; // a pit,
        ma[10 * TILE_PX + 255] += 500.0; // a spike on the edge of the tile,
        mb[20 * TILE_PX + 20] = -32768.0; // no data,
        mb[100 * TILE_PX + 100] = f32::NAN;
        // and a real ridge one pixel wide, 40 m over the ground (8 m over
        // its neighbours' spread), kept.
        for y in 120..140 {
            mb[y * TILE_PX + 60] += 40.0;
        }
        let tiles = Tiles::new(z, [(a, Arc::new(DemTile { metres: ma })), (b, Arc::new(DemTile { metres: mb }))].into_iter().collect());
        let t = |k: TileKey| tiles.tiles[&k].clone();
        assert_eq!(t(a).at(40, 40), base(a, 40));
        assert_eq!(t(a).at(90, 80), base(a, 90));
        assert_eq!(t(a).at(255, 10), base(a, 255), "across the tiles' edge");
        assert!(t(b).at(20, 20).is_nan() && t(b).at(100, 100).is_nan(), "no data is no data");
        assert_eq!(t(b).at(60, 130), base(b, 60) + 40.0, "a ridge stays");
        assert_eq!(t(b).at(61, 130), base(b, 61));
        // No data read between pixels: none; on samples, filled from around.
        let (x, y) = ((501 * 256 + 20) as f64 + 0.5, (300 * 256 + 20) as f64 + 0.5);
        let (lat, lon) = from_px(x, y, z);
        assert_eq!(tiles.at(lat, lon), None);
        let mut s = Samples { spacing: 1.0, nx: 4, nz: 3, metres: vec![1.0, 2.0, 3.0, 4.0, 1.0, f32::NAN, f32::NAN, 4.0, 1.0, 2.0, 3.0, 4.0], lakes: Default::default() };
        s.fill_gaps();
        assert_eq!(s.missing(), 0);
        assert!(s.metres[5] > 1.0 && s.metres[5] < 3.5 && s.metres[6] > 2.0 && s.metres[6] < 4.0, "{:?}", s.metres);
        // A grid without any data is left as it is.
        let mut none = Samples { spacing: 1.0, nx: 2, nz: 2, metres: vec![f32::NAN; 4], lakes: Default::default() };
        none.fill_gaps();
        assert_eq!(none.missing(), 4);
    }

    /// A new world of a slope rising eastward from a sea at 0 m to hills,
    /// `size` blocks a side.
    fn coast(size: i32) -> (Area, Samples, Fit, World) {
        let area = Area { lat: 46.0, lon: 7.0, blocks_per_metre: 1.0, width: size, depth: size }.whole_tiles();
        struct Coast;
        impl Elevation for Coast {
            fn metres(&self, _: &Area, east: f64, _: f64) -> Option<f32> {
                Some((east as f32 * 0.25).max(0.0))
            }
        }
        let samples = Samples::read(&area, &Coast, 2.0);
        let fit = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: true }.fit(samples.span().unwrap(), 1.0);
        let world = new_world("coast", &samples, &fit, &area, 62, -64, 320);
        (area, samples, fit, world)
    }

    #[test]
    fn a_new_world_has_data_to_its_edges() {
        let (area, _, fit, world) = coast(300);
        // Grown to whole tiles: 384 blocks, three tiles a side.
        assert_eq!((area.width, area.depth), (384, 384));
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        assert_eq!((hi.x - lo.x + 1, hi.z - lo.z + 1), (3, 3));
        let top = |x: i32, z: i32| {
            let t = dim.ground().view(TilePos::containing(x, z)).unwrap();
            t.top.get(local_index(x, z))
        };
        let (x0, x1, z1) = (lo.x * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
        // Every column of the last ones rises as the slope does: none
        // repeats the one before it (the stretched edge of 0.6).
        for x in x1 - 40..x1 {
            assert!(top(x, z1 - 1) > top(x - 2, z1 - 1) + 0.3, "column {x}: {} after {}", top(x, z1 - 1), top(x - 2, z1 - 1));
        }
        // The east edge as high as its real height says.
        let east = (x1 - x0) as f32 - 0.5 - area.width as f32 / 2.0;
        assert!((top(x1 - 1, 0) - fit.height(east * 0.25)).abs() < 1.0, "{} {}", top(x1 - 1, 0), fit.height(east * 0.25));
    }

    #[test]
    fn the_sea_is_wet_and_sand_keeps_to_it() {
        let (_, _, _, world) = coast(1024);
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        let x0 = lo.x * TILE_SIZE;
        let sand = world.registry.terrain_by_name("Sand").unwrap().0;
        let col = |x: i32| {
            let t = dim.ground().view(TilePos::containing(x, 0)).unwrap();
            let i = local_index(x, 0);
            (t.top.get(i), t.fluid.get(i), t.terrain.get(i))
        };
        // The sea at 0 m: a block under the water, not level with it.
        let (top, fluid, _) = col(x0 + 10);
        assert!(top <= 61.0 && fluid == 62, "{top} {fluid}");
        // Where the land rises out of it, sand; low ground farther from
        // the water, none (it was sand as far as it lay low, a desert).
        let shore = (x0..(hi.x + 1) * TILE_SIZE).find(|&x| col(x).0 >= 62.0).unwrap();
        assert_eq!(col(shore + 2).2, sand);
        let far = (BEACH_METRES as i32) + 10;
        assert_ne!(col(shore + far).2, sand);
        assert!(col(shore + far).0 < 62.0 + 2.2 + 30.0);
    }

    #[test]
    fn snow_lies_by_latitude() {
        assert_eq!(snow_line(0.0), 5200.0);
        assert!((2900.0..3100.0).contains(&snow_line(46.0)));
        assert!(snow_line(-60.0) < 2000.0);
        assert_eq!(snow_line(85.0), 0.0);
    }

    #[test]
    fn heights_fit_the_range() {
        let v = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: false };
        // A block a metre: 100 m is 100 blocks over the sea.
        let f = v.fit(Span { low: -50.0, high: 100.0 }, 1.0);
        assert_eq!(f.height(100.0), 162.0);
        assert_eq!(f.height(-50.0), 12.0);
        // Twice as tall.
        let f = Vertical { exaggeration: 2.0, ..v }.fit(Span { low: -50.0, high: 100.0 }, 1.0);
        assert_eq!(f.height(100.0), 262.0);
        // Too tall for the world: cut at its top without fitting...
        let alps = Span { low: 400.0, high: 4808.0 };
        let f = v.fit(alps, 0.1);
        assert_eq!(f.height(4808.0), 319.0);
        assert_eq!(f.height(2000.0), 262.0);
        // ...squeezed into it with: the summit at the top, the sea where it is.
        let f = Vertical { fit: true, ..v }.fit(alps, 0.1);
        assert!((f.height(4808.0) - 319.0).abs() < 1e-3);
        assert_eq!(f.height(0.0), 61.0, "the sea at 0 m under the water");
        assert!((f.height(2404.0) - (62.0 + 257.0 / 2.0)).abs() < 1e-2);
        // Deep sea squeezed above the bottom of the world, apart from land.
        let f = Vertical { fit: true, ..v }.fit(Span { low: -4000.0, high: 50.0 }, 0.5);
        assert!((f.height(-4000.0) - (-63.0)).abs() < 1e-3);
        assert_eq!(f.height(50.0), 87.0);
        // The real height at the sea level moved: an inland valley at 400 m
        // on the sea.
        let f = Vertical { datum: 400.0, ..v }.fit(alps, 0.1);
        assert_eq!(f.height(400.0), 62.0);
        assert_eq!(f.height(1400.0), 162.0);
    }

    #[test]
    fn a_grid_from_a_file() {
        let map = Heightmap {
            width: 3,
            height: 3,
            data: vec![0.0, 0.5, 1.0, 0.0, 0.5, 1.0, -9999.0, 0.5, 1.0],
        };
        let grid = Grid { map: Arc::new(map), metres_per_px: 10.0, low: 100.0, high: 300.0, nodata: Some(-9999.0) };
        let area = Area { lat: 0.0, lon: 0.0, blocks_per_metre: 1.0, width: 30, depth: 30 };
        assert_eq!(grid.metres(&area, 0.0, 0.0), Some(200.0));
        assert_eq!(grid.metres(&area, 10.0, 0.0), Some(300.0));
        assert_eq!(grid.metres(&area, -10.0, 10.0), None, "no data there");
        assert_eq!(grid.metres(&area, 40.0, 0.0), None, "off the grid");
    }

    #[test]
    fn spacing_follows_the_data() {
        let area = Area { lat: 0.0, lon: 0.0, blocks_per_metre: 1.0, width: 2048, depth: 2048 };
        assert_eq!(spacing(&area, 3.0), 3.0);
        assert_eq!(spacing(&Area { blocks_per_metre: 0.1, ..area }, 3.0), 1.0);
        assert_eq!(spacing(&Area { width: 32768, ..area }, 1.0), 4.0);
    }

    #[test]
    fn a_geotiff_of_heights() {
        use tiff::encoder::{TiffEncoder, colortype::Gray32Float};
        use tiff::tags::Tag;
        let dir = std::env::temp_dir().join(format!("massif-geotiff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dem.tif");
        let (w, h) = (40u32, 20u32);
        let data: Vec<f32> = (0..w * h).map(|k| (k % w) as f32 * 10.0 - 50.0).collect();
        {
            let mut enc = TiffEncoder::new(std::fs::File::create(&path).unwrap()).unwrap();
            let mut img = enc.new_image::<Gray32Float>(w, h).unwrap();
            // A pixel of 1/3600° (an arc second), from 46°N, 7°E.
            img.encoder().write_tag(Tag::Unknown(33550), &[1.0 / 3600.0, 1.0 / 3600.0, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::Unknown(33922), &[0.0, 0.0, 0.0, 7.0, 46.0, 0.0][..]).unwrap();
            img.encoder().write_tag(Tag::Unknown(42113), "-9999").unwrap();
            img.write_data(&data).unwrap();
        }
        let t = read_tiff(&path).unwrap();
        assert_eq!((t.map.width, t.map.height), (40, 20));
        assert_eq!(t.map.data[39], 340.0);
        assert_eq!(t.nodata, Some(-9999.0));
        let (lat, lon) = t.centre.unwrap();
        assert!((lat - (46.0 - 10.0 / 3600.0)).abs() < 1e-9 && (lon - (7.0 + 20.0 / 3600.0)).abs() < 1e-9);
        // An arc second east at 46°: about 21.5 m.
        let m = t.metres_per_px.unwrap();
        assert!((m - 21.48).abs() < 0.05, "{m}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn written_into_the_selection_by_its_weight() {
        use crate::generate::{GenParams, generate};
        use crate::selection::{Selection, Shape};
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let (lo, hi) = world.dims[0].bounds().unwrap();
        let rect = BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
        // The west half selected.
        let west = BlockRect::new(rect.x0, rect.z0, rect.x0 + rect.width() / 2, rect.z1);
        world.dims[0].selection = Some(Arc::new(Selection::shape(&Shape::Rect(west))));
        let before = world.clone();
        // Everywhere 30 m under the sea, at a block a metre.
        let area = Area { lat: 0.0, lon: 0.0, blocks_per_metre: 1.0, width: rect.width(), depth: rect.depth() };
        struct Flat;
        impl Elevation for Flat {
            fn metres(&self, _: &Area, _: f64, _: f64) -> Option<f32> {
                Some(-30.0)
            }
        }
        let samples = Samples::read(&area, &Flat, 4.0);
        let fit = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: true }.fit(samples.span().unwrap(), 1.0);
        let mut history = crate::history::History::new(4);
        history.begin("real", 0);
        let changes = write_ground(&mut history.edit(&mut world, 0), &samples, &fit, rect, None, &Steps::default());
        history.commit();
        assert!(changes.columns > 0 && !changes.cancelled);
        let column = |w: &World, x: i32, z: i32| {
            let tile = w.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
            let i = local_index(x, z);
            (tile.top.get(i), tile.fluid.get(i))
        };
        let (x, z) = (rect.x0 + 5, rect.z0 + 5);
        assert_eq!(column(&world, x, z), (32.0, 62), "under the sea, and the sea over it");
        let east = (rect.x1 - 5, rect.z0 + 5);
        assert_eq!(column(&world, east.0, east.1), column(&before, east.0, east.1), "outside the selection, as it was");
        history.undo(&mut world);
        assert_eq!(column(&world, x, z), column(&before, x, z));
    }

    /// A map of waste ground with pits of sand, its biomes and covers
    /// painted, gets a real place over it: repainted, its land is painted
    /// from the new relief alone (sand only along the new sea, grass inland,
    /// snow over the snow line), its painted biomes, covers and objects
    /// taken off; kept, the sand of the old map lies where it was, on the
    /// new land.
    #[test]
    fn written_over_the_map_repainted_or_kept() {
        use crate::generate::{GenParams, Theme, generate};
        let mut map = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let theme = Theme::new(&map.registry);
        let (lo, hi) = map.dims[0].bounds().unwrap();
        let rect = BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
        // Sand everywhere but for a pit of grass in each tile, painted
        // biomes, covers and object layers.
        for pos in rect.tiles() {
            let ground = map.dims[0].ground_mut();
            let tile = ground.tile_mut(pos).unwrap();
            for i in 0..crate::coords::TILE_AREA {
                tile.terrain.set(i, if i % 97 == 0 { theme.grass.0 } else { theme.sand.0 });
                tile.biome.set(i, 3);
                tile.cover.set(i, 1);
                tile.objects.set(i, 1);
            }
        }
        // The sea in the west, the land rising to 4,000 m in the east, at
        // a block every 50 m.
        let area = Area { lat: 45.0, lon: 7.0, blocks_per_metre: 0.02, width: rect.width(), depth: rect.depth() };
        struct Slope;
        impl Elevation for Slope {
            fn metres(&self, area: &Area, east: f64, _: f64) -> Option<f32> {
                let (w, _) = area.metres();
                Some(((east / w + 0.5) * 4300.0 - 300.0 ) as f32)
            }
        }
        let samples = Samples::read(&area, &Slope, 2.0);
        let fit = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: true }.fit(samples.span().unwrap(), 0.02);
        let terrain_at = |w: &World, x: i32| {
            let z = rect.z0 + 70;
            let tile = w.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
            let i = local_index(x, z);
            (tile.terrain.get(i), tile.biome.get(i), tile.cover.get(i), tile.objects.get(i))
        };
        let inland = rect.x0 + rect.width() / 2;
        let summit = rect.x1 - 3;
        for repaint in [true, false] {
            let mut world = map.clone();
            let mut history = crate::history::History::new(4);
            history.begin("real", 0);
            let changes = write_ground(&mut history.edit(&mut world, 0), &samples, &fit, rect, repaint.then_some((&area, theme)), &Steps::default());
            history.commit();
            assert!(changes.columns > 0);
            if repaint {
                assert_eq!(terrain_at(&world, inland), (theme.grass.0, 0, 0, 0), "inland: grass, nothing painted left");
                assert_eq!(terrain_at(&world, summit).0, theme.snow.0, "over the snow line");
                let sand = (rect.x0..rect.x1).filter(|&x| terrain_at(&world, x).0 == theme.sand.0).count();
                assert!(sand > 0 && sand < rect.width() as usize / 8, "sand along the water only: {sand} columns");
            } else {
                assert_eq!(terrain_at(&world, inland), (theme.sand.0, 3, 1, 1), "the old paint kept");
            }
            history.undo(&mut world);
            assert_eq!(terrain_at(&world, inland), terrain_at(&map, inland), "one undo");
        }
    }

    /// Heights from a function of metres east and south of the centre.
    struct Field<F: Fn(f64, f64) -> Option<f32> + Sync>(F);
    impl<F: Fn(f64, f64) -> Option<f32> + Sync> Elevation for Field<F> {
        fn metres(&self, _: &Area, east: f64, south: f64) -> Option<f32> {
            (self.0)(east, south)
        }
    }

    /// The largest step between two samples side by side.
    fn largest_step(s: &Samples) -> f32 {
        let mut most = 0f32;
        for j in 0..s.nz {
            for i in 0..s.nx {
                let m = s.metres[j * s.nx + i];
                if i + 1 < s.nx {
                    most = most.max((s.metres[j * s.nx + i + 1] - m).abs());
                }
                if j + 1 < s.nz {
                    most = most.max((s.metres[(j + 1) * s.nx + i] - m).abs());
                }
            }
        }
        most
    }

    #[test]
    fn a_tile_without_data_is_filled_with_no_seam() {
        // Gentle hills over tiles of zoom 12; one tile in the middle all no
        // data (as the server's -32768), another lost and made from the tile
        // over it two zooms coarser, that one 6 m off (another source).
        let area = Area { lat: 45.0, lon: 6.0, blocks_per_metre: 0.1, width: 2048, depth: 2048 };
        let z = 12;
        let hill = |x: f64, y: f64| (300.0 + 40.0 * (x / 97.0).sin() + 30.0 * (y / 131.0).cos()) as f32;
        let tile = |k: TileKey, off: f32, scale: f64| {
            let metres = (0..TILE_PX * TILE_PX).map(|i| hill((k.x as f64 * 256.0 + (i % TILE_PX) as f64 + 0.5) * scale, (k.y as f64 * 256.0 + (i / TILE_PX) as f64 + 0.5) * scale) + off).collect();
            DemTile { metres }
        };
        let keys = area.tiles(z);
        let mut sorted = keys.clone();
        sorted.sort();
        let (hole, lost) = (sorted[sorted.len() / 2], sorted[sorted.len() / 2 + 1]);
        let mut tiles: HashMap<TileKey, Arc<DemTile>> = keys.iter().map(|&k| (k, Arc::new(tile(k, 0.0, 1.0)))).collect();
        tiles.insert(hole, Arc::new(DemTile { metres: vec![-32768.0; TILE_PX * TILE_PX] }));
        let parent = parent_of(lost, z - 2);
        let coarse = tile(parent, 6.0, 4.0);
        tiles.insert(lost, Arc::new(from_coarser(&coarse, parent, lost)));
        let patched: std::collections::HashSet<TileKey> = [lost].into();
        let source = Tiles::patched(z, tiles, &patched);
        assert_eq!(source.missing_px(), TILE_PX * TILE_PX);
        let mut samples = Samples::read(&area, &source, 4.0);
        let gaps = samples.missing();
        assert!(gaps > 1000, "{gaps}");
        let ready = samples.prepare(area.blocks_per_metre, None);
        assert_eq!((ready.missing, ready.lakes), (gaps, 0));
        assert_eq!(samples.missing(), 0);
        // The hills step at most so much from one sample to the next; the
        // filled tile and the one made from coarser data no more than that,
        // nowhere a seam.
        let whole = Samples::read(&area, &Field(|e, s| {
            let (lat, lon) = area.lat_lon(e, s);
            let (x, y) = to_px(lat, lon, z);
            Some(hill(x, y))
        }), 4.0);
        let natural = largest_step(&whole);
        let step = largest_step(&samples);
        assert!(step < natural * 1.6 + 0.5, "a step of {step} m, the hills' own at most {natural} m");
    }

    /// A low coast as the land models give it: the sea at 0 m exactly, the
    /// land rising from 0.5 m, and the band between them ringing either
    /// side of 0 m in squares; inland, a lagoon at 0 m with a ringing shore,
    /// and a lake 20 m up, flat. At 2 m a block.
    fn low_coast() -> (Area, World) {
        let area = Area { lat: 45.4, lon: 12.3, blocks_per_metre: 0.5, width: 1024, depth: 1024 }.whole_tiles();
        let ring = |e: f64, s: f64| if (((e / 7.0).floor() + (s / 7.0).floor()) as i64).rem_euclid(2) == 0 { 0.35 } else { -0.3 };
        let field = Field(move |e: f64, s: f64| {
            // The coast at 600 m west; land rising eastward.
            let land = e + 600.0;
            let lagoon = ((e - 300.0).powi(2) + s.powi(2)).sqrt() - 250.0;
            let lake = ((e - 300.0).powi(2) + (s - 650.0).powi(2)).sqrt() - 150.0;
            Some(if land < -30.0 {
                0.0
            } else if land < 0.0 {
                ring(e, s)
            } else if lagoon < -20.0 {
                0.0
            } else if lagoon < 0.0 {
                ring(e, s)
            } else if lake < 0.0 {
                20.0
            } else {
                (0.5 + land * 0.01).min(0.5 + lagoon * 0.02 + 0.5).max(0.5) as f32 + 0.05 * ((e / 3.1).sin() * (s / 4.7).cos()) as f32 + if lake < 40.0 { 20.0 * (1.0 - lake / 40.0) as f32 } else { 0.0 }
            })
        });
        let mut samples = Samples::read(&area, &field, 2.0);
        samples.prepare(area.blocks_per_metre, None);
        let fit = Vertical { datum: 0.0, sea_level: 62, exaggeration: 1.0, low: -64, high: 320, fit: false }.fit(samples.span().unwrap(), area.blocks_per_metre);
        assert_eq!(samples.lakes.levels, vec![20.0]);
        let mut world = new_world("low coast", &samples, &fit, &area, 62, -64, 320);
        let rules = rules_at(&crate::auto::AutoRules::default(), area.lat);
        let mut history = crate::history::History::new(1);
        history.begin("automatic", 0);
        let base = crate::auto::base_terrain(&world.dims[0], &world.registry);
        let reader = crate::auto::Reader::new(&rules, &world.registry, 62).with_base(base);
        crate::auto::paint_over_the_map(&mut history.edit(&mut world, 0), &reader);
        crate::auto::biomes_over_the_map(&mut history.edit(&mut world, 0), &reader);
        drop(history);
        (area, world)
    }

    fn column(world: &World, x: i32, z: i32) -> (f32, i16, u16, u16) {
        let t = world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
        let i = local_index(x, z);
        (t.top.get(i), t.fluid.get(i), t.terrain.get(i), t.biome.get(i))
    }

    #[test]
    fn a_lagoon_has_water_where_the_source_says_water() {
        let (_, world) = low_coast();
        let sand = world.registry.terrain_by_name("Sand").unwrap().0;
        // The lagoon, 300 m east of the middle and 250 m across: water all
        // over, not sand, its floor under it.
        for (e, s) in [(300.0, 0.0), (300.0, 150.0), (150.0, -100.0), (450.0, 100.0)] {
            let (x, z) = ((e * 0.5) as i32, (s * 0.5) as i32);
            let (top, fluid, terrain, _) = column(&world, x, z);
            assert!(fluid == 62 && top < 62.0, "({e}, {s}) m: top {top}, water {fluid}");
            assert_ne!(terrain, sand, "({e}, {s}) m");
        }
        // The lake 20 m up: water at its own level, 10 blocks over the sea.
        let (top, fluid, _, _) = column(&world, 150, 325);
        assert!(fluid == 72 && top < 72.0, "the lake: top {top}, water {fluid}");
        // The sea west of the coast: water, deepening away from it.
        let (near, far) = (column(&world, (-640.0f64 * 0.5) as i32, 0), column(&world, (-1000.0f64 * 0.5) as i32, 0));
        assert!(near.1 == 62 && far.1 == 62 && far.0 < near.0, "{near:?} {far:?}");
        // No pool on the land: between the sea and the lagoon every column
        // is dry, and sand only along the water (a narrow beach).
        let (mut pools, mut sandy) = (0, 0);
        for e in (-560..0).step_by(4) {
            for s in (-300..300).step_by(8) {
                let (top, fluid, terrain, _) = column(&world, (e as f64 * 0.5) as i32, (s as f64 * 0.5) as i32);
                pools += (fluid != NO_FLUID && fluid as f32 > top) as usize;
                sandy += (terrain == sand) as usize;
            }
        }
        assert_eq!(pools, 0, "pools on the land");
        assert!(sandy * 10 < 140 * 75, "sand on {sandy} of {} columns", 140 * 75);
        // The coast follows a line, not the squares of the ringing data: along
        // it, the shore stays within a few blocks of 600 m west.
        for s in (-400..400).step_by(16) {
            let z = (s as f64 * 0.5) as i32;
            let shore = (-400..0).find(|&x| { let (top, fluid, _, _) = column(&world, x, z); fluid == NO_FLUID || fluid as f32 <= top }).unwrap();
            assert!((shore + 300).abs() <= 12, "at {s} m south the shore is at {shore}");
        }
    }

    /// The water of a real place holds: the sea, a lagoon behind its low
    /// coast and a lake up in the land stand beside no lower land and over
    /// no air, and the game leaves the export's water as it is.
    #[test]
    fn a_real_places_water_holds() {
        let (_, low) = low_coast();
        let (_, _, _, steep) = coast(512);
        for (name, world) in [("low coast", &low), ("coast", &steep)] {
            let dim = &world.dims[0];
            let held = crate::water::check(dim);
            assert!(held.columns > 1000, "{name}: water: {held:?}");
            assert_eq!((held.spills, held.floating), (0, 0), "{name}: {held:?}");
            let settled = crate::water::settle_all(dim, true);
            assert_eq!(settled.unsettled, 0, "{name}: {settled:?}");
        }
    }

    #[test]
    fn the_waters_biomes_are_ocean_river_or_beach() {
        let (area, world) = low_coast();
        let reg = &world.registry;
        let id = |b: u16| reg.biome(crate::registry::BiomeId(b)).id.clone();
        let (shallow, deep) = sea_biomes(area.lat);
        let mut seen = std::collections::BTreeMap::new();
        let dim = &world.dims[0];
        let (lo, hi) = dim.bounds().unwrap();
        for z in (lo.z * TILE_SIZE..(hi.z + 1) * TILE_SIZE).step_by(3) {
            for x in (lo.x * TILE_SIZE..(hi.x + 1) * TILE_SIZE).step_by(3) {
                let (top, fluid, _, biome) = column(&world, x, z);
                if fluid == NO_FLUID || fluid as f32 <= top {
                    continue;
                }
                let b = id(biome);
                assert_ne!(b, "minecraft:plains", "water at ({x}, {z}) is plains");
                *seen.entry(b).or_insert(0usize) += 1;
            }
        }
        // The sea is the latitude's ocean, the lake over the sea's level is
        // a river's.
        assert!(seen.contains_key(shallow) || seen.contains_key(deep), "{seen:?}");
        assert!(seen.contains_key("minecraft:river"), "{seen:?}");
        assert!(seen.keys().all(|b| b == shallow || b == deep || b == "minecraft:river" || b == "minecraft:beach"), "{seen:?}");
        // The coast's sand is beach.
        let beach = (-310..-280).any(|x| id(column(&world, x, 0).3) == "minecraft:beach");
        assert!(beach);
        assert_eq!(sea_biomes(10.0).0, "minecraft:warm_ocean");
        assert_eq!(sea_biomes(-75.0).1, "minecraft:deep_frozen_ocean");
    }
}
