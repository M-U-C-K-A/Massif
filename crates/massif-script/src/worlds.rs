//! Whole worlds: opened from a WorldPainter `.world` or a `.massif`, made
//! from a height map as `wp.createWorld()` does, saved, exported to
//! Minecraft; WorldPainter's map formats and biome numbers.

use crate::session::{Line, R, Session, carve, java_round, massif_path};
use massif_core::coords::{TILE_AREA, TILE_SIZE, TilePos, local_index};
use massif_core::plane::Plane;
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};
use massif_legacy::BIOMES;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// WorldPainter's map formats: its id, its name, the Minecraft release
/// Massif writes for it, and its build limits.
pub const FORMATS: [(&str, &str, &str, i32, i32); 7] = [
    ("org.pepsoft.mcregion", "Minecraft 1.1", "", 0, 128),
    ("org.pepsoft.anvil", "Minecraft 1.2 - 1.12", "1.12.2", 0, 256),
    ("org.pepsoft.anvil.1.13", "Minecraft 1.15 - 1.16", "1.16.5", 0, 256),
    ("org.pepsoft.anvil.1.17", "Minecraft 1.17", "1.17.1", 0, 256),
    ("org.pepsoft.anvil.1.18", "Minecraft 1.18", "1.18.2", -64, 320),
    ("org.pepsoft.anvil.1.19", "Minecraft 1.19 - 1.20.4", "1.20.4", -64, 320),
    ("org.pepsoft.anvil.1.20.5", "Minecraft 1.20.5 or later", "1.20.5", -64, 320),
];

/// The map format `id` (WorldPainter's id, or a Minecraft release Massif
/// knows) or of display name `name`.
pub fn map_format(id: Option<&str>, name: Option<&str>) -> R<(String, String)> {
    if let Some(id) = id {
        if let Some(f) = FORMATS.iter().find(|f| f.0 == id) {
            return Ok((f.0.into(), f.1.into()));
        }
        if massif_anvil::Target::release(id).is_some() {
            return Ok((id.into(), format!("Minecraft {id}")));
        }
        return Err(format!("There is no map format with ID \"{id}\""));
    }
    let name = name.unwrap_or_default();
    FORMATS.iter().find(|f| f.1.eq_ignore_ascii_case(name)).map(|f| (f.0.into(), f.1.into())).ok_or_else(|| format!("There is no map format with name \"{name}\""))
}

/// The release Massif exports for map format `id`.
fn target(id: Option<&str>) -> R<massif_anvil::Target> {
    let Some(id) = id else { return Ok(massif_anvil::Target::default()) };
    let release = FORMATS.iter().find(|f| f.0 == id).map_or(id, |f| f.2);
    if release.is_empty() {
        return Err(format!("Exporting to {id} (Minecraft 1.1) is not supported yet"));
    }
    massif_anvil::Target::release(release).ok_or_else(|| format!("Massif does not export for {id}"))
}

/// The build limits of map format `id`.
fn limits(id: Option<&str>) -> (i32, i32) {
    match id.and_then(|id| FORMATS.iter().find(|f| f.0 == id)) {
        Some(f) => (f.3, f.4),
        None => match id.and_then(massif_anvil::Target::release) {
            Some(t) if t.data_version < 2860 => (0, 256),
            _ => (-64, 320),
        },
    }
}

/// WorldPainter's biome number for `name`: a biome's id (`minecraft:plains`,
/// `plains`) or name (`Snowy Plains`).
pub fn biome_number(name: &str) -> Option<i32> {
    let wanted = name.trim().to_ascii_lowercase().replace(' ', "_");
    let wanted = wanted.strip_prefix("minecraft:").unwrap_or(&wanted).to_owned();
    BIOMES.iter().position(|b| b.is_some_and(|b| b.strip_prefix("minecraft:") == Some(wanted.as_str()))).map(|k| k as i32)
}

impl Session {
    /// `wp.getWorld().fromFile(file)`: a WorldPainter world (imported as
    /// Massif imports one) or a Massif map.
    pub fn open_world(&mut self, file: &str) -> R<usize> {
        let path = self.readable(file)?;
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        self.io.progress.set(0.0, &format!("Opening {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        let world = if ext == massif_format::EXTENSION {
            massif_format::load(&path).map_err(|e| format!("I/O error while loading world {file}: {e}"))?
        } else {
            let (world, report) = massif_legacy::import_world(&path, None).map_err(|e| format!("Unloadable world {file} (not a WorldPainter world?): {e}"))?;
            let lost: Vec<String> = report.layers.iter().filter(|(name, _)| !report.converted.iter().any(|(c, _)| c == name)).map(|(n, _)| n.clone()).collect();
            if !lost.is_empty() {
                self.print(Line::Warn(format!("{file}: layers Massif does not import: {}", lost.join(", "))));
            }
            world
        };
        Ok(self.add_world(world, Some(path)))
    }

    /// `wp.saveWorld(world).toFile(file)`: Massif saves `.massif` maps, so
    /// `x.world` is written as `x.massif`, which it says.
    pub fn save_world(&mut self, w: usize, file: &str) -> R<PathBuf> {
        self.flush()?;
        let mut path = self.access.check(file, false)?;
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case(massif_format::EXTENSION)) {
            let to = massif_path(&path);
            self.print(Line::Warn(format!("Massif saves its own maps: {} written as {}", path.display(), to.display())));
            path = to;
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !dir.is_dir() {
            return Err(format!("Destination directory {} does not exist or is not a directory", dir.display()));
        }
        if path.is_file() {
            // As WorldPainter keeps backups of what it overwrites.
            let backup = path.with_extension(format!("{}.bak", massif_format::EXTENSION));
            std::fs::rename(&path, &backup).map_err(|e| format!("Could not move {} to {}: {e}", path.display(), backup.display()))?;
        }
        self.io.progress.set(0.0, "Saving");
        let slot = self.slot(w)?;
        carve(slot);
        // Written from a cold copy: what is decoded stays so for the script.
        let copy = slot.world.copy_unfrozen().snapshot();
        massif_format::save(&copy, &path).map_err(|e| format!("I/O error saving file (message: {e})"))?;
        Ok(path)
    }

    /// `wp.exportWorld(world).toDirectory(dir)`: the map exported into a
    /// folder named after it in `dir` (by default Minecraft's saves); a
    /// map of that name already there is moved aside first.
    pub fn export_world(&mut self, w: usize, dir: Option<&str>) -> R<PathBuf> {
        self.flush()?;
        let base = match dir {
            Some(d) => self.access.check(d, true)?,
            None => self.access.saves.clone().filter(|s| s.is_dir()).ok_or("No directory given, and Minecraft's saves folder was not found: use exportWorld(world).toDirectory(…)")?,
        };
        if !base.is_dir() {
            return Err(format!("Directory {} does not exist or is not a directory", base.display()));
        }
        let slot = self.slot(w)?;
        carve(slot);
        let name = slot.world.meta.name.clone();
        let clean: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
        let to = base.join(clean.trim());
        if to.exists() {
            let aside = massif_anvil::export::unique_dir(&base, &format!("{} backup", clean.trim()));
            std::fs::rename(&to, &aside).map_err(|e| format!("I/O error while exporting world: could not move {} aside: {e}", to.display()))?;
            self.print(Line::Warn(format!("The map already in {} was moved to {}", to.display(), aside.display())));
        }
        let io = self.io.clone();
        let slot = self.slot(w)?;
        let options = massif_anvil::ExportOptions { target: target(slot.format.as_deref())?, ..Default::default() };
        let others = slot.world.dims.iter().skip(1).count();
        let progress = massif_anvil::Progress::default();
        let world = &slot.world;
        let result = std::thread::scope(|s| {
            let job = s.spawn(|| massif_anvil::export(world, &to, &options, &progress));
            while !job.is_finished() {
                if io.cancelled() {
                    progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                io.progress.set(progress.fraction(), "Exporting");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            job.join().expect("the export does not panic")
        });
        result.map_err(|e| format!("I/O error while exporting world: {e}"))?;
        if others > 0 {
            self.print(Line::Warn("Only the surface is exported: Massif exports the Nether and the End separately".into()));
        }
        Ok(to)
    }

    /// `wp.createWorld().fromHeightMap(…)…go()`, as WorldPainter's
    /// `HeightMapImporter` does it: levels one for one by default (each
    /// block 0.4375 lower, as WorldPainter makes it for its smooth snow),
    /// else mapped linearly; the water at its level everywhere; outside the
    /// image, low beaches; terrains by WorldPainter's default theme —
    /// beaches from two under the water to one over, then grass, permadirt,
    /// stone mix and deep snow by height, Frost high up — the heights
    /// jittered by Massif's noise (WorldPainter's own is seeded at random).
    pub fn create_world(&mut self, spec: CreateSpec) -> R<usize> {
        let (min, max) = limits(spec.format.as_deref());
        let min = spec.lower_build_limit.unwrap_or(min);
        let max = spec.upper_build_limit.unwrap_or(max);
        if min >= max {
            return Err(format!("Lower build limit {min} not under upper build limit {max}"));
        }
        let hm = self.heightmap(spec.height_map)?;
        let name = Path::new(&hm.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
        let img = hm.image()?;
        let (iw, ih) = (img.width as usize, img.height as usize);
        let (image_low, image_high) = (spec.image_low.unwrap_or(0.0), spec.image_high.unwrap_or(255.0));
        let (world_low, world_high) = (spec.world_low.unwrap_or(0.0), spec.world_high.unwrap_or(255.0));
        let water = spec.water_level;
        if world_low.min(water as f64) < min as f64 {
            return Err(format!("Map format not deep enough to accommodate minimum terrain height of {world_low} or water level of {water}"));
        }
        let scaled = spec.scale != 100;
        let one_on_one = world_low == image_low && world_high == image_high;
        let high_res = image_high >= max as f64 && world_high < max as f64;
        let level_scale = (world_high - world_low) / (image_high - image_low);
        let height_of = |level: f64| -> f32 {
            let h = if high_res {
                (level - image_low) * level_scale + world_low
            } else if one_on_one {
                if scaled { level } else { level - 0.4375 }
            } else {
                (level - image_low) * level_scale + world_low
            };
            (h as f32).clamp(min as f32, (max - 1) as f32)
        };
        let scale = spec.scale as f64 / 100.0;
        let (ew, eh) = ((iw as f64 * scale).round().max(1.0) as i32, (ih as f64 * scale).round().max(1.0) as i32);
        // The image's level under a column, `None` off the image.
        let level_at = |x: i32, z: i32| -> Option<f64> {
            if x < spec.dx || z < spec.dy || x >= spec.dx + ew || z >= spec.dy + eh {
                return None;
            }
            let sample = |i: f64, j: f64| img.at(j.clamp(0.0, (ih - 1) as f64) as usize * iw + i.clamp(0.0, (iw - 1) as f64) as usize) as f64;
            Some(if !scaled {
                sample((x - spec.dx) as f64, (z - spec.dy) as f64)
            } else {
                let u = ((x - spec.dx) as f64 + 0.5) / scale - 0.5;
                let v = ((z - spec.dy) as f64 + 0.5) / scale - 0.5;
                let (i0, j0) = (u.floor(), v.floor());
                let (fu, fv) = (u - i0, v - j0);
                let top = sample(i0, j0) * (1.0 - fu) + sample(i0 + 1.0, j0) * fu;
                let bottom = sample(i0, j0 + 1.0) * (1.0 - fu) + sample(i0 + 1.0, j0 + 1.0) * fu;
                top * (1.0 - fv) + bottom * fv
            })
        };
        let mut world = World::new(WorldMeta { name, seed: rand_seed(), sea_level: water + 1, ..Default::default() });
        world.dims = vec![Dimension::new(DimensionKind::Overworld, min, max)];
        let seed = world.meta.seed;
        let w = self.add_world(world, None);
        let slot = self.slot(w)?;
        slot.format = spec.format.clone();
        let ordinal = |n: &str| crate::session::VALUES.iter().position(|t| *t == n).expect("a terrain of WorldPainter's") as i32;
        let (grass, permadirt, stone_mix, deep_snow, beaches) = (slot.terrain_id(ordinal("GRASS"))?, slot.terrain_id(ordinal("PERMADIRT"))?, slot.terrain_id(ordinal("STONE_MIX"))?, slot.terrain_id(ordinal("DEEP_SNOW"))?, slot.beaches());
        let frost = massif_legacy::layers::frost_cover(&mut slot.world.registry);
        // WorldPainter's default theme.
        let factor = max.min(320) as f32 / 128.0;
        let (k_permadirt, k_stone, k_snow, k_frost) = ((32.0 * factor) as i32 + water, (48.0 * factor) as i32 + water, (80.0 * factor) as i32 + water, (64.0 * factor) as i32 + water);
        let noise = massif_core::noise::Noise::new(seed as u64);
        let (small, tiny) = (noise.derive(1), noise.derive(2));
        let floor = (water - 20).max(min);
        let variation = 15.min((water - floor) / 2) as f64;
        let medium = noise.derive(3);
        let tiles: Vec<TilePos> = crate::session::tiles_of(spec.dx, spec.dy, spec.dx + ew, spec.dy + eh).collect();
        let made: Vec<(TilePos, Tile)> = tiles
            .par_iter()
            .map(|&pos| {
                let (ox, oz) = pos.origin();
                let mut top = vec![0f32; TILE_AREA];
                let mut terrain = vec![0u16; TILE_AREA];
                let mut fluid = vec![NO_FLUID; TILE_AREA];
                let mut cover = vec![0u16; TILE_AREA];
                for z in oz..oz + TILE_SIZE {
                    for x in ox..ox + TILE_SIZE {
                        let i = local_index(x, z);
                        let (h, beach) = match level_at(x, z) {
                            Some(level) => (height_of(level), false),
                            None => ((floor as f64 + (medium.get(x as f64 / 32.771, z as f64 / 32.771) * 0.5 + 0.5) * variation) as f32, true),
                        };
                        let t = massif_core::codec::snap_height(h + 1.0);
                        top[i] = t;
                        let int_height = java_round(h).clamp(min, max - 1);
                        terrain[i] = if beach || (int_height >= water - 2 && int_height <= water + 1) {
                            beaches.at(x, z, t)
                        } else {
                            let jitter = (small.get(x as f64 / 16.411, z as f64 / 16.411) * 0.5 + tiny.get(x as f64 / 4.099, z as f64 / 4.099) * 0.5) * 5.0;
                            let jh = int_height + jitter as i32;
                            if jh > k_snow {
                                deep_snow
                            } else if jh > k_stone {
                                stone_mix
                            } else if jh > k_permadirt {
                                permadirt
                            } else {
                                grass
                            }
                        };
                        if int_height >= k_frost {
                            cover[i] = frost;
                        }
                        if water > int_height {
                            fluid[i] = (water + 1) as i16;
                        }
                    }
                }
                let mut tile = Tile::ground(0.0, 0, NO_FLUID);
                tile.top = Plane::from_vec(top);
                tile.terrain = Plane::from_vec(terrain);
                tile.fluid = Plane::from_vec(fluid);
                tile.cover = Plane::from_vec(cover);
                tile.compact();
                (pos, tile)
            })
            .collect();
        let slot = self.slot(w)?;
        for (pos, tile) in made {
            slot.world.dims[0].ground_mut().insert(pos, tile);
        }
        let (cx, cz) = (spec.dx + ew / 2, spec.dy + eh / 2);
        let h = level_at(cx, cz).map_or(water as f32, height_of);
        slot.world.meta.spawn = [cx, java_round(h) + 1, cz];
        slot.changed = true;
        Ok(w)
    }
}

fn rand_seed() -> i64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    massif_core::compose::hash2(t as i32, (t >> 32) as i32, 0x7365_6564) as i64 | ((t as i64) << 32)
}

/// `wp.createWorld()`'s settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSpec {
    pub height_map: usize,
    #[serde(default = "hundred")]
    pub scale: i32,
    #[serde(default)]
    pub dx: i32,
    #[serde(default)]
    pub dy: i32,
    pub image_low: Option<f64>,
    pub image_high: Option<f64>,
    pub world_low: Option<f64>,
    pub world_high: Option<f64>,
    #[serde(default = "sixty_two")]
    pub water_level: i32,
    pub format: Option<String>,
    pub lower_build_limit: Option<i32>,
    pub upper_build_limit: Option<i32>,
}

fn hundred() -> i32 {
    100
}

fn sixty_two() -> i32 {
    62
}
