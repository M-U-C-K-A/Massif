//! Procedural starting terrain for a new world.

use crate::codec::snap_height;
use crate::coords::{TILE_AREA, TILE_SIZE, TilePos};
use crate::level::Blob;
use crate::noise::Noise;
use crate::plane::Plane;
use crate::registry::{Registry, TerrainId};
use crate::tile::Tile;
use crate::world::{World, WorldMeta};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Landform {
    Flat,
    Plains,
    Hills,
    Mountains,
    Islands,
}

impl Landform {
    pub const ALL: [Landform; 5] = [
        Landform::Flat,
        Landform::Plains,
        Landform::Hills,
        Landform::Mountains,
        Landform::Islands,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Landform::Flat => "Flat",
            Landform::Plains => "Plains",
            Landform::Hills => "Hills",
            Landform::Mountains => "Mountains",
            Landform::Islands => "Islands",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenParams {
    pub name: String,
    pub seed: i64,
    /// Size of the map in tiles (128 blocks each), centred on the origin.
    pub tiles_x: i32,
    pub tiles_z: i32,
    pub landform: Landform,
    /// Elevation of the water surface.
    pub sea_level: i32,
    /// Elevation of flat worlds.
    pub flat_height: f32,
    /// Base elevation of the land; `None`: the water level.
    #[serde(default)]
    pub level: Option<f32>,
    /// Height of the relief, in percent of the landform's own.
    #[serde(default = "hundred")]
    pub relief: f32,
    /// Horizontal size of the features (hills, ranges, islands), in percent.
    #[serde(default = "hundred")]
    pub feature_size: f32,
    /// Sand along the shores.
    #[serde(default = "yes")]
    pub beaches: bool,
    /// One terrain for the whole surface instead of the automatic ones
    /// (grass, sand, rock and snow by height and slope).
    #[serde(default)]
    pub surface: Option<TerrainId>,
    /// Lava instead of water.
    #[serde(default)]
    pub lava: bool,
}

fn hundred() -> f32 {
    100.0
}

fn yes() -> bool {
    true
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            name: "Generated World".into(),
            seed: 27_594_263,
            tiles_x: 8,
            tiles_z: 8,
            landform: Landform::Hills,
            sea_level: 63,
            flat_height: 64.0,
            level: None,
            relief: 100.0,
            feature_size: 100.0,
            beaches: true,
            surface: None,
            lava: false,
        }
    }
}

impl GenParams {
    /// Inclusive tile bounds of the generated map.
    pub fn bounds(&self) -> (TilePos, TilePos) {
        let (w, d) = (self.tiles_x.max(1), self.tiles_z.max(1));
        let min = TilePos::new(-(w / 2), -(d / 2));
        (min, TilePos::new(min.x + w - 1, min.z + d - 1))
    }
}

/// The height field of a landform, sampled per block column.
pub struct HeightField {
    landform: Landform,
    /// Base elevation the relief is built on.
    sea: f64,
    flat: f64,
    relief: f64,
    scale: f64,
    base: Noise,
    detail: Noise,
    ridge: Noise,
    warp: Noise,
    mask: Noise,
}

impl HeightField {
    pub fn new(params: &GenParams) -> Self {
        let root = Noise::new(params.seed as u64);
        Self {
            landform: params.landform,
            sea: params.level.unwrap_or(params.sea_level as f32) as f64,
            flat: params.level.unwrap_or(params.flat_height) as f64,
            relief: (params.relief as f64 / 100.0).max(0.0),
            scale: (params.feature_size as f64 / 100.0).max(0.05),
            base: root.derive(1),
            detail: root.derive(2),
            ridge: root.derive(3),
            warp: root.derive(4),
            mask: root.derive(5),
        }
    }

    /// Surface elevation at block column `(x, z)`.
    pub fn height(&self, x: f64, z: f64) -> f32 {
        if self.landform == Landform::Flat {
            return self.flat as f32;
        }
        let (x, z) = (x / self.scale, z / self.scale);
        let h = self.raw(x, z);
        (self.sea + (h - self.sea) * self.relief) as f32
    }

    fn raw(&self, x: f64, z: f64) -> f64 {
        // Domain warping bends the features so they do not look like plain
        // value noise.
        let wx = x + 60.0 * self.warp.fbm(x / 420.0, z / 420.0, 2, 2.0, 0.5);
        let wz = z + 60.0 * self.warp.fbm(x / 420.0 + 91.7, z / 420.0 - 33.1, 2, 2.0, 0.5);
        let detail = self.detail.fbm(x / 48.0, z / 48.0, 3, 2.0, 0.5);
        let sea = self.sea;
        match self.landform {
            Landform::Flat => self.flat,
            Landform::Plains => {
                sea + 4.0 + 7.0 * self.base.fbm(wx / 520.0, wz / 520.0, 4, 2.0, 0.5) + 1.2 * detail
            }
            Landform::Hills => {
                let n = self.base.fbm(wx / 380.0, wz / 380.0, 5, 2.0, 0.5);
                sea + 9.0 + 34.0 * n + 2.5 * detail
            }
            Landform::Mountains => {
                // Where the mask is high, ridges take over from rolling land.
                let m = (self.mask.fbm(wx / 900.0, wz / 900.0, 3, 2.0, 0.5) * 1.6 + 0.45)
                    .clamp(0.0, 1.0);
                let m = m * m * (3.0 - 2.0 * m);
                let low = 8.0 + 26.0 * self.base.fbm(wx / 420.0, wz / 420.0, 4, 2.0, 0.5);
                let r = self.ridge.ridged(wx / 520.0, wz / 520.0, 6, 2.05, 0.52);
                sea + low + m * (r.powf(1.6) * 175.0 + 12.0) + (2.0 + 5.0 * m) * detail
            }
            Landform::Islands => {
                let n = self.base.fbm(wx / 460.0, wz / 460.0, 5, 2.0, 0.52);
                // Most of the map is sea bed; land rises where the noise peaks.
                let land = ((n - 0.08) * 3.2).clamp(-1.0, 1.0);
                let shaped = if land > 0.0 {
                    land.powf(1.25) * 46.0
                } else {
                    land * 26.0
                };
                sea + 1.0 + shaped + 2.0 * detail * (0.3 + land.max(0.0))
            }
        }
    }
}

/// The terrain ids the generator paints with.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub grass: TerrainId,
    pub rock: TerrainId,
    pub rocky: TerrainId,
    pub sand: TerrainId,
    pub gravel: TerrainId,
    pub snow: TerrainId,
}

impl Theme {
    pub fn new(reg: &Registry) -> Self {
        let get = |name: &str| reg.terrain_by_name(name).unwrap_or(TerrainId::ROCK);
        Self {
            grass: get("Grass"),
            rock: TerrainId::ROCK,
            rocky: get("Rocky"),
            sand: get("Sand"),
            gravel: get("Gravel"),
            snow: get("Snow"),
        }
    }

    /// Pick a terrain from elevation and slope (blocks of rise per block).
    /// `jitter` in `-1..1` breaks up the altitude bands.
    pub fn terrain(&self, h: f32, slope: f32, sea: f32, jitter: f32) -> TerrainId {
        self.terrain_with(h, slope, sea, jitter, true)
    }

    /// [`Theme::terrain`], with or without sandy shores.
    pub fn terrain_with(&self, h: f32, slope: f32, sea: f32, jitter: f32, beaches: bool) -> TerrainId {
        if h < sea - 14.0 + 3.0 * jitter {
            self.gravel
        } else if beaches && h < sea + 2.2 + 0.8 * jitter && slope < 0.9 {
            self.sand
        } else if slope > 1.35 {
            self.rock
        } else if h > 178.0 + 12.0 * jitter {
            self.snow
        } else if h > 138.0 + 10.0 * jitter || slope > 1.05 {
            self.rocky
        } else {
            self.grass
        }
    }
}

/// Generate one ground tile.
pub fn generate_tile(pos: TilePos, field: &HeightField, theme: &Theme, params: &GenParams) -> Tile {
    let n = TILE_SIZE as usize;
    let (ox, oz) = pos.origin();
    let sea_level = params.sea_level;
    if field.landform == Landform::Flat {
        let terrain = params.surface.unwrap_or(theme.grass);
        return Tile::ground(snap_height(field.flat as f32), terrain.0, sea_level as i16);
    }

    // One extra column on every side for the slope estimate.
    let m = n + 2;
    let mut h = vec![0f32; m * m];
    for z in 0..m {
        for x in 0..m {
            let bx = (ox + x as i32 - 1) as f64 + 0.5;
            let bz = (oz + z as i32 - 1) as f64 + 0.5;
            h[z * m + x] = field.height(bx, bz);
        }
    }

    let mut top = vec![0f32; TILE_AREA];
    let mut terrain = vec![0u16; TILE_AREA];
    let sea = sea_level as f32;
    for z in 0..n {
        for x in 0..n {
            let c = (z + 1) * m + (x + 1);
            let dx = (h[c + 1] - h[c - 1]) * 0.5;
            let dz = (h[c + m] - h[c - m]) * 0.5;
            let slope = dx.abs().max(dz.abs());
            let jitter = field
                .detail
                .get((ox + x as i32) as f64 / 23.0 + 7.7, (oz + z as i32) as f64 / 23.0 - 3.3)
                as f32;
            let i = z * n + x;
            top[i] = snap_height(h[c]);
            terrain[i] = match params.surface {
                Some(t) => t.0,
                None => theme.terrain_with(h[c], slope, sea, jitter, params.beaches).0,
            };
        }
    }

    let mut tile = Tile::ground(0.0, 0, sea_level as i16);
    tile.top = Plane::from_vec(top);
    tile.terrain = Plane::from_vec(terrain);
    tile.compact();
    tile
}

/// Generate a whole world. Tiles are produced in parallel and stored cold, so
/// even very large maps only cost their compressed size.
pub fn generate(params: &GenParams) -> World {
    let mut world = World::new(WorldMeta {
        name: params.name.clone(),
        seed: params.seed,
        spawn: [0, 0, 0],
        sea_level: params.sea_level,
        lava: params.lava,
        graph: None,
    });
    let field = HeightField::new(params);
    let theme = Theme::new(&world.registry);
    let (min, max) = params.bounds();

    let positions: Vec<TilePos> = (min.z..=max.z)
        .flat_map(|z| (min.x..=max.x).map(move |x| TilePos::new(x, z)))
        .collect();
    let blobs: Vec<(TilePos, Blob)> = positions
        .par_iter()
        .map(|&pos| {
            let tile = generate_tile(pos, &field, &theme, params);
            (pos, Blob::from(tile.encode()))
        })
        .collect();
    let ground = world.dims[0].ground_mut();
    for (pos, blob) in blobs {
        ground.insert_blob(pos, blob);
    }

    let surface = field.height(0.5, 0.5).max(params.sea_level as f32);
    world.meta.spawn = [0, crate::compose::block_top(surface) + 1, 0];
    world
}

/// How a height map becomes a world.
#[derive(Debug, Clone, PartialEq)]
pub struct FromHeightmap {
    pub name: String,
    /// Blocks a pixel covers.
    pub scale: f32,
    /// The heights black and white stand for.
    pub low: f32,
    pub high: f32,
    pub sea_level: i32,
}

/// A world whose ground is height map `map`, centred on the origin, its
/// terrains painted from height and slope as [`generate`] paints them, the
/// sea at its level.
pub fn from_heightmap(map: &crate::heightmap::Heightmap, how: &FromHeightmap) -> World {
    let mut world = World::new(WorldMeta {
        name: how.name.clone(),
        seed: 0,
        spawn: [0, 0, 0],
        sea_level: how.sea_level,
        lava: false,
        graph: None,
    });
    let theme = Theme::new(&world.registry);
    let scale = how.scale.max(0.01);
    let (w, d) = (map.width as f32 * scale, map.height as f32 * scale);
    let (tx, tz) = ((w / TILE_SIZE as f32).ceil().max(1.0) as i32, (d / TILE_SIZE as f32).ceil().max(1.0) as i32);
    let (x0, z0) = (-(tx / 2) * TILE_SIZE, -(tz / 2) * TILE_SIZE);
    // The picture's corner on the first tile's, the rest of the last tiles
    // as its edge goes on.
    let height = |bx: f32, bz: f32| how.low + (how.high - how.low) * map.sample((bx - x0 as f32) / scale - 0.5, (bz - z0 as f32) / scale - 0.5);
    let sea = how.sea_level as f32;
    let positions: Vec<TilePos> = (0..tz).flat_map(|z| (0..tx).map(move |x| TilePos::new(x0.div_euclid(TILE_SIZE) + x, z0.div_euclid(TILE_SIZE) + z))).collect();
    let blobs: Vec<(TilePos, Blob)> = positions
        .par_iter()
        .map(|&pos| {
            let (ox, oz) = pos.origin();
            let n = TILE_SIZE as usize;
            let mut top = vec![0f32; TILE_AREA];
            let mut terrain = vec![0u16; TILE_AREA];
            for z in 0..n {
                for x in 0..n {
                    let (bx, bz) = ((ox + x as i32) as f32 + 0.5, (oz + z as i32) as f32 + 0.5);
                    let h = height(bx, bz);
                    let slope = (height(bx + 1.0, bz) - height(bx - 1.0, bz)).abs().max((height(bx, bz + 1.0) - height(bx, bz - 1.0)).abs()) * 0.5;
                    top[z * n + x] = snap_height(h);
                    terrain[z * n + x] = theme.terrain(h, slope, sea, 0.0).0;
                }
            }
            let mut tile = Tile::ground(0.0, 0, how.sea_level as i16);
            tile.top = Plane::from_vec(top);
            tile.terrain = Plane::from_vec(terrain);
            tile.compact();
            (pos, Blob::from(tile.encode()))
        })
        .collect();
    let ground = world.dims[0].ground_mut();
    for (pos, blob) in blobs {
        ground.insert_blob(pos, blob);
    }
    let surface = height(0.5, 0.5).max(sea);
    world.meta.spawn = [0, crate::compose::block_top(surface) + 1, 0];
    world
}

/// A small picture of what [`generate`] will make: heights and terrains on
/// an `n` × `n` grid spread over the whole map, for previews.
pub struct Preview {
    pub n: usize,
    pub heights: Vec<f32>,
    pub terrains: Vec<TerrainId>,
    /// Blocks between two samples.
    pub spacing: f64,
}

pub fn preview(params: &GenParams, reg: &Registry, n: usize) -> Preview {
    let field = HeightField::new(params);
    let theme = Theme::new(reg);
    let (min, max) = params.bounds();
    let (x0, z0) = (min.x as f64 * TILE_SIZE as f64, min.z as f64 * TILE_SIZE as f64);
    let width = ((max.x - min.x + 1) as f64 * TILE_SIZE as f64).max((max.z - min.z + 1) as f64 * TILE_SIZE as f64);
    let spacing = width / n as f64;
    let heights: Vec<f32> = (0..n * n)
        .into_par_iter()
        .map(|i| field.height(x0 + ((i % n) as f64 + 0.5) * spacing, z0 + ((i / n) as f64 + 0.5) * spacing))
        .collect();
    let sea = params.sea_level as f32;
    let terrains = (0..n * n)
        .map(|i| {
            if let Some(t) = params.surface {
                return t;
            }
            let (x, z) = (i % n, i / n);
            let at = |x: usize, z: usize| heights[z.min(n - 1) * n + x.min(n - 1)];
            let dx = (at(x + 1, z) - at(x.saturating_sub(1), z)) / (2.0 * spacing as f32);
            let dz = (at(x, z + 1) - at(x, z.saturating_sub(1))) / (2.0 * spacing as f32);
            theme.terrain_with(heights[i], dx.abs().max(dz.abs()), sea, 0.0, params.beaches)
        })
        .collect();
    Preview {
        n,
        heights,
        terrains,
        spacing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::local_index;

    fn small(landform: Landform) -> GenParams {
        GenParams {
            tiles_x: 4,
            tiles_z: 3,
            landform,
            ..GenParams::default()
        }
    }

    #[test]
    fn bounds_are_centred_on_the_origin() {
        let p = small(Landform::Hills);
        assert_eq!(p.bounds(), (TilePos::new(-2, -1), TilePos::new(1, 1)));
        let w = generate(&p);
        assert_eq!(w.dims[0].ground().len(), 12);
        assert_eq!(w.dims[0].bounds(), Some(p.bounds()));
        // Everything is stored cold.
        assert_eq!(w.dims[0].ground().stats().hot_tiles, 0);
    }

    #[test]
    fn generation_is_deterministic_and_seamless() {
        for landform in Landform::ALL {
            let p = small(landform);
            let a = generate(&p);
            let b = generate(&p);
            let ga = a.dims[0].ground();
            let gb = b.dims[0].ground();
            let mut steps = Vec::new();
            for z in -128..256 {
                let left = ga.view(TilePos::new(-1, TilePos::containing(0, z).z)).unwrap();
                let right = ga.view(TilePos::new(0, TilePos::containing(0, z).z)).unwrap();
                let hl = left.top.get(local_index(-1, z));
                let hr = right.top.get(local_index(0, z));
                steps.push((hl - hr).abs());
                let right_b = gb.view(TilePos::new(0, TilePos::containing(0, z).z)).unwrap();
                assert_eq!(hr, right_b.top.get(local_index(0, z)));
            }
            // Across a tile border the terrain is as smooth as anywhere else.
            let worst = steps.iter().cloned().fold(0.0, f32::max);
            assert!(worst < 6.0, "{landform:?}: step of {worst} across a tile border");
        }
    }

    #[test]
    fn landforms_have_plausible_relief() {
        let range = |landform| {
            let p = GenParams {
                tiles_x: 12,
                tiles_z: 12,
                landform,
                ..GenParams::default()
            };
            let field = HeightField::new(&p);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for z in (-768..768).step_by(8) {
                for x in (-768..768).step_by(8) {
                    let h = field.height(x as f64, z as f64);
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
            (lo, hi)
        };
        let (lo, hi) = range(Landform::Flat);
        assert_eq!((lo, hi), (64.0, 64.0));
        let (lo, hi) = range(Landform::Plains);
        assert!(lo > 50.0 && hi < 85.0, "plains {lo}..{hi}");
        let (lo, hi) = range(Landform::Hills);
        assert!(lo > 20.0 && hi < 125.0 && hi - lo > 25.0, "hills {lo}..{hi}");
        let (lo, hi) = range(Landform::Mountains);
        assert!(lo > 0.0 && hi < 300.0 && hi > 140.0, "mountains {lo}..{hi}");
        let (lo, hi) = range(Landform::Islands);
        assert!(lo < 55.0 && lo > 20.0 && hi > 66.0 && hi < 130.0, "islands {lo}..{hi}");
    }

    #[test]
    fn theme_bands() {
        let reg = Registry::vanilla();
        let t = Theme::new(&reg);
        assert_eq!(t.terrain(40.0, 0.1, 63.0, 0.0), t.gravel);
        assert_eq!(t.terrain(62.0, 0.1, 63.0, 0.0), t.sand);
        assert_eq!(t.terrain(64.0, 0.1, 63.0, 0.0), t.sand);
        assert_eq!(t.terrain(80.0, 0.2, 63.0, 0.0), t.grass);
        assert_eq!(t.terrain(80.0, 2.0, 63.0, 0.0), t.rock);
        assert_eq!(t.terrain(150.0, 0.2, 63.0, 0.0), t.rocky);
        assert_eq!(t.terrain(200.0, 0.2, 63.0, 0.0), t.snow);
        assert_ne!(t.grass, t.rock);
    }
    #[test]
    fn level_relief_size_beaches_and_surface() {
        let base = GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Hills,
            ..GenParams::default()
        };
        let range = |p: &GenParams| {
            let field = HeightField::new(p);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for z in (-256..256).step_by(4) {
                for x in (-256..256).step_by(4) {
                    let h = field.height(x as f64, z as f64);
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
            (lo, hi)
        };
        let (lo, hi) = range(&base);
        // Half the relief: half the spread around the same base.
        let (lo2, hi2) = range(&GenParams { relief: 50.0, ..base.clone() });
        assert!(((hi2 - lo2) - (hi - lo) * 0.5).abs() < 1.0, "{lo}..{hi} vs {lo2}..{hi2}");
        // A higher level lifts everything.
        let (lo3, _) = range(&GenParams { level: Some(100.0), relief: 0.0, ..base.clone() });
        assert_eq!(lo3, 100.0);
        // One terrain everywhere.
        let rock = GenParams { surface: Some(TerrainId::ROCK), ..base.clone() };
        let w = generate(&rock);
        let tile = w.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert!((0..TILE_AREA).all(|i| tile.terrain.get(i) == TerrainId::ROCK.0));
        // Lava worlds remember it.
        assert!(generate(&GenParams { lava: true, ..base.clone() }).meta.lava);
        // Previews cover the map.
        let p = preview(&base, &Registry::vanilla(), 32);
        assert_eq!(p.heights.len(), 32 * 32);
        assert_eq!(p.spacing, 512.0 / 32.0);
        // Bigger features vary more slowly.
        let field = HeightField::new(&GenParams { feature_size: 400.0, ..base.clone() });
        let small = HeightField::new(&base);
        let wiggle = |f: &HeightField| (0..64).map(|i| (f.height(i as f64 * 4.0, 0.0) - f.height(i as f64 * 4.0 + 4.0, 0.0)).abs()).sum::<f32>();
        assert!(wiggle(&field) < wiggle(&small));
    }

}

/// Very large maps, as WorldPainter's: how long they take and how much they
/// weigh. Runs with MASSIF_GIANT (the size in blocks, e.g. "60000x50000");
/// run it with `--release`.
#[cfg(test)]
mod giant {
    use super::*;
    use crate::history::History;

    #[test]
    fn giant_maps() {
        let Ok(size) = std::env::var("MASSIF_GIANT") else { return };
        let (w, d) = size.split_once('x').map(|(w, d)| (w.parse::<i32>().unwrap(), d.parse::<i32>().unwrap())).expect("WxD");
        let t = std::time::Instant::now();
        let mut world = generate(&GenParams {
            tiles_x: (w + 127) / 128,
            tiles_z: (d + 127) / 128,
            landform: Landform::Hills,
            ..GenParams::default()
        });
        let tiles = world.dims[0].ground().len();
        eprintln!("generated {tiles} tiles in {:.1} s", t.elapsed().as_secs_f32());
        let bytes: usize = {
            let ground = world.dims[0].ground();
            ground.positions().map(|p| ground.blob_len(p)).sum()
        };
        eprintln!("compressed: {:.2} GB", bytes as f64 / 1e9);
        // The graph's Map node over the whole map, at a preview's size.
        let t = std::time::Instant::now();
        let ext = crate::graph::Extent::covering(world.dims[0].bounds().map(|(lo, hi)| crate::coords::BlockRect::new(lo.x * 128, lo.z * 128, (hi.x + 1) * 128, (hi.z + 1) * 128)).unwrap(), 512);
        let read = crate::graph::sample_map(&world.dims[0], &ext, 0.0, &[], &[]);
        eprintln!("the map read at 512 × 512 in {:.1} s ({} high)", t.elapsed().as_secs_f32(), read.heights.range().1);
        // A brush stroke somewhere.
        let t = std::time::Instant::now();
        let mut h = History::default();
        h.begin("raise", 0);
        let brush = crate::brush::Brush {
            radius: 200.0,
            ..Default::default()
        };
        crate::ops::raise(&mut h.edit(&mut world, 0), &brush, 1000.0, 1000.0, 5.0, crate::ops::Surface::Top);
        h.commit();
        eprintln!("a brush of 400 blocks in {:.3} s", t.elapsed().as_secs_f32());
    }

    #[test]
    fn a_world_is_made_from_a_height_map() {
        // A ramp from black in the west to white in the east, 300 × 200.
        let map = crate::heightmap::Heightmap {
            width: 300,
            height: 200,
            data: (0..300 * 200).map(|k| (k % 300) as f32 / 299.0).collect(),
        };
        let how = FromHeightmap {
            name: "Ramp".into(),
            scale: 2.0,
            low: 40.0,
            high: 200.0,
            sea_level: 63,
        };
        let world = from_heightmap(&map, &how);
        let (lo, hi) = world.dims[0].bounds().unwrap();
        // 600 × 400 blocks: 5 × 4 tiles.
        assert_eq!((hi.x - lo.x + 1, hi.z - lo.z + 1), (5, 4));
        let at = |x: i32, z: i32| world.dims[0].ground().view(TilePos::containing(x, z)).unwrap().top.get(crate::coords::local_index(x, z));
        let (x0, z0) = (lo.x * 128, lo.z * 128);
        assert!((at(x0, z0 + 10) - 40.0).abs() < 1.0, "black is low: {}", at(x0, z0 + 10));
        assert!((at(x0 + 599, z0 + 10) - 200.0).abs() < 1.0, "white is high: {}", at(x0 + 599, z0 + 10));
        assert!((at(x0 + 300, z0 + 10) - 120.0).abs() < 2.0, "in between: {}", at(x0 + 300, z0 + 10));
    }
}
