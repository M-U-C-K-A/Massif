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
    sea: f64,
    flat: f64,
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
            sea: params.sea_level as f64,
            flat: params.flat_height as f64,
            base: root.derive(1),
            detail: root.derive(2),
            ridge: root.derive(3),
            warp: root.derive(4),
            mask: root.derive(5),
        }
    }

    /// Surface elevation at block column `(x, z)`.
    pub fn height(&self, x: f64, z: f64) -> f32 {
        // Domain warping bends the features so they do not look like plain
        // value noise.
        let wx = x + 60.0 * self.warp.fbm(x / 420.0, z / 420.0, 2, 2.0, 0.5);
        let wz = z + 60.0 * self.warp.fbm(x / 420.0 + 91.7, z / 420.0 - 33.1, 2, 2.0, 0.5);
        let detail = self.detail.fbm(x / 48.0, z / 48.0, 3, 2.0, 0.5);
        let sea = self.sea;
        let h = match self.landform {
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
        };
        h as f32
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
        if h < sea - 14.0 + 3.0 * jitter {
            self.gravel
        } else if h < sea + 2.2 + 0.8 * jitter && slope < 0.9 {
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
pub fn generate_tile(pos: TilePos, field: &HeightField, theme: &Theme, sea_level: i32) -> Tile {
    let n = TILE_SIZE as usize;
    let (ox, oz) = pos.origin();
    if field.landform == Landform::Flat {
        return Tile::ground(snap_height(field.flat as f32), theme.grass.0, sea_level as i16);
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
            terrain[i] = theme.terrain(h[c], slope, sea, jitter).0;
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
            let tile = generate_tile(pos, &field, &theme, params.sea_level);
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
}
