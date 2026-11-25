//! The game's textures in the views' own look (Settings ▸ Graphics): a
//! map of the game's blocks and one of Conquest Reforged's, up close,
//! halfway and whole, in flat colours and textured, at one and two pixels a
//! point. Renders them side by side (MASSIF_SNAPSHOT_DIR) and times them
//! (MASSIF_PERF): the best and middle of 12 frames, a ray a pixel (moving)
//! and a ray a sample (still).
//!
//! The vanilla map is MASSIF_TEXTURE_WORLD (a .massif), or one made here;
//! the Conquest one MASSIF_TEXTURE_CONQUEST, or the Conquest session's test
//! map in ~/Documents, read with the instance of Conquest installed here.

use super::*;
use massif_core::World;
use massif_core::generate::{GenParams, Landform, generate};
use std::path::PathBuf;
use std::sync::Arc;

/// The ground's height at a column (the bottom of the world off the map).
fn ground(dim: &Dimension, x: i32, z: i32) -> f64 {
    dim.ground().view(TilePos::containing(x, z)).map_or(dim.min_y as f32, |v| v.top.get(local_index(x, z))) as f64
}

/// The cameras' turn, and how high and far the closest two are from what
/// they look at.
const YAW: f32 = 0.65;
const NEAR: (f32, f32) = (0.42, 28.0);
const HALFWAY: (f32, f32) = (0.45, 190.0);

/// Whether the eye of a camera looking at `at` from `pitch` and `distance`
/// sees it: the ground stays under the line between them.
fn in_sight(dim: &Dimension, at: [f64; 3], pitch: f32, distance: f32) -> bool {
    let eye = Camera { target: at, yaw: YAW, pitch, distance }.eye();
    (1..=24).all(|k| {
        let f = k as f64 / 24.0;
        let p = [at[0] + (eye[0] - at[0]) * f, at[1] + (eye[1] - at[1]) * f, at[2] + (eye[2] - at[2]) * f];
        ground(dim, p[0].floor() as i32, p[2].floor() as i32) + 1.0 < p[1]
    })
}

/// Low dry land near `(x, z)`, meadows and woods a few blocks over the sea,
/// that the closest two cameras see.
fn land_near(dim: &Dimension, sea: f64, x: i32, z: i32) -> (i32, i32) {
    let wet = |x: i32, z: i32| dim.ground().view(TilePos::containing(x, z)).is_none_or(|v| {
        let f = v.fluid.get(local_index(x, z));
        f != NO_FLUID && f as f32 > v.top.get(local_index(x, z))
    });
    for r in (0..3000).step_by(8) {
        for k in 0..(r / 8).max(1) * 8 {
            let a = k as f64 / ((r / 8).max(1) * 8) as f64 * std::f64::consts::TAU;
            let (px, pz) = (x + (a.cos() * r as f64) as i32, z + (a.sin() * r as f64) as i32);
            let h = ground(dim, px, pz);
            let at = [px as f64 + 0.5, h + 1.0, pz as f64 + 0.5];
            if !wet(px, pz) && h > sea + 3.0 && h < sea + 20.0 && in_sight(dim, at, NEAR.0, NEAR.1) && in_sight(dim, at, HALFWAY.0, HALFWAY.1) {
                return (px, pz);
            }
        }
    }
    (x, z)
}

/// An image read back: width, height, RGBA.
type Image = (u32, u32, Vec<u8>);

/// A map on the graphics card with the game's textures.
struct Scene {
    name: &'static str,
    world: World,
    tiles: Vec<TilePos>,
    renderer: Renderer3d,
    /// Where to look: a point of the ground.
    focus: [f64; 3],
    extent: f32,
}

fn scene(device: &wgpu::Device, queue: &wgpu::Queue, name: &'static str, mut world: World, cat: &massif_assets::Catalogue, focus: Option<(i32, i32)>) -> Scene {
    massif_assets::apply_colors(&mut world.registry, cat);
    let style = Style { mode: MapMode::Terrain, water: true, lava: world.meta.lava, notes: None, caves: massif_core::compose::Caves::Openings, xray: false, objects: true, density: false };
    let dim = &world.dims[0];
    let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
    assert_eq!(shape.stride, 1, "every block");
    let tiles: Vec<TilePos> = dim.ground().positions().collect();
    let started = Instant::now();
    let built: Vec<(TilePos, TileBuild)> = tiles.par_iter().map(|&t| (t, build_tile(dim, &world.registry, world.meta.sea_level as f32, world.meta.seed, t, style))).collect();
    let build_s = started.elapsed().as_secs_f32();
    let started = Instant::now();
    let mut renderer = Renderer3d::with_samples(device, queue, 4);
    let made_ms = started.elapsed().as_secs_f32() * 1000.0;
    renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).unwrap();
    let mut pairs = Pairs::default();
    for (t, b) in &built {
        renderer.upload_columns(*t, &pairs.pack(b));
        renderer.upload(*t, &b.faces);
    }
    renderer.flush();
    renderer.set_palette(pairs.list());
    let started = Instant::now();
    let atlas = crate::textures::atlas(&world.registry, cat, device.limits().max_texture_array_layers as usize);
    renderer.set_look(&atlas, &crate::textures::clouds(Some(cat)));
    let atlas_ms = started.elapsed().as_secs_f32() * 1000.0;
    let (lo, hi) = dim.bounds().unwrap();
    let t = TILE_SIZE;
    let (x0, z0, x1, z1) = (lo.x * t, lo.z * t, (hi.x + 1) * t, (hi.z + 1) * t);
    let sea = world.meta.sea_level as f64;
    let (fx, fz) = focus.unwrap_or_else(|| land_near(dim, sea, (x0 + x1) / 2, (z0 + z1) / 2));
    eprintln!(
        "== {name}: renderer made in {made_ms:.0} ms; {} tiles built in {build_s:.2} s; {} pairs; atlas {} layers for {} materials in {atlas_ms:.0} ms, {:.1} MB on the card; terrain {:.0} MB",
        tiles.len(),
        pairs.list().len(),
        atlas.layers.len(),
        atlas.materials.len(),
        renderer.look_bytes() as f64 / 1048576.0,
        renderer.bytes() as f64 / 1048576.0
    );
    Scene {
        name,
        focus: [fx as f64 + 0.5, ground(dim, fx, fz), fz as f64 + 0.5],
        extent: (x1 - x0).max(z1 - z0) as f32,
        world,
        tiles,
        renderer,
    }
}

impl Scene {
    /// The cameras: up close, halfway, the whole map.
    fn cameras(&self) -> [(&'static str, Camera); 3] {
        let at = self.focus;
        [
            ("near", Camera { target: at, yaw: YAW, pitch: NEAR.0, distance: NEAR.1 }),
            ("halfway", Camera { target: at, yaw: YAW, pitch: HALFWAY.0, distance: HALFWAY.1 }),
            ("far", Camera { target: at, yaw: YAW, pitch: 0.55, distance: self.extent * 0.8 }),
        ]
    }

    /// Draw a frame `size` pixels from `camera`, textured or not; with
    /// `fine`, a ray a sample. Returns the image if asked, and the best and
    /// middle times of 12 frames if timed.
    fn draw(&mut self, camera: &Camera, size: [u32; 2], textured: bool, fine: bool, image: bool, timed: bool) -> (Option<Image>, Option<(f32, f32)>) {
        let dim = &self.world.dims[0];
        self.renderer.ensure_target(size);
        set_fine(&mut self.renderer, fine);
        let (materials, fluids) = self.renderer.look_info();
        let wk = self.world.registry.well_known;
        let substrate = Substrate { stone: self.world.registry.block(wk.stone).color, deepslate: self.world.registry.block(wk.deepslate).color, whole: false };
        let mut times = Vec::new();
        let frames = if timed { 16 } else { 1 };
        let mut out = None;
        for k in 0..frames {
            let mut camera = *camera;
            if k + 1 < frames {
                camera.yaw += 0.002 * k as f32;
            }
            let origin = (camera.target[0].floor() as i32, camera.target[2].floor() as i32);
            let mut globals = camera.globals(origin, size[0] as f32 / size[1] as f32, self.extent, Sky::of(&crate::theme::Palette::DARK), size[1]);
            if textured {
                globals.look = [1.0, 0.0, 0.0, materials as f32];
                globals.rock = [wk.stone.0 as f32, wk.deepslate.0 as f32, fluids[0] as f32, fluids[1] as f32];
            }
            let m = Mat4::from_cols_array_2d(&globals.view_proj);
            let shown: Vec<TilePos> = self
                .tiles
                .iter()
                .copied()
                .filter(|t| self.renderer.chunks.contains_key(t))
                .filter(|t| {
                    let (x, z) = ((t.x * TILE_SIZE - origin.0) as f32, (t.z * TILE_SIZE - origin.1) as f32);
                    in_view(&m, Vec3::new(x, dim.min_y as f32, z), Vec3::new(x + TILE_SIZE as f32, dim.max_y as f32, z + TILE_SIZE as f32))
                })
                .collect();
            let t = Instant::now();
            self.renderer.render(&globals, origin, &shown, dim.min_y, substrate, size);
            self.renderer.wait();
            if k >= 4 && k + 1 < frames {
                times.push(t.elapsed().as_secs_f32() * 1000.0);
            }
            if image && k + 1 == frames {
                out = self.renderer.read_image();
            }
        }
        times.sort_by(f32::total_cmp);
        (out, (!times.is_empty()).then(|| (times[0], times[times.len() / 2])))
    }
}

/// How much an image changes from a pixel to the next, in its lower half
/// (the land): the mean difference of their grey levels.
fn detail(img: &Image) -> f32 {
    let (w, h, rgba) = (img.0 as usize, img.1 as usize, &img.2);
    let grey = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        (rgba[i] as f32 * 0.3 + rgba[i + 1] as f32 * 0.59 + rgba[i + 2] as f32 * 0.11) / 255.0
    };
    let (mut sum, mut n) = (0.0, 0.0);
    for y in h / 2..h - 1 {
        for x in 0..w - 1 {
            sum += (grey(x + 1, y) - grey(x, y)).abs() + (grey(x, y + 1) - grey(x, y)).abs();
            n += 2.0;
        }
    }
    100.0 * sum / n
}

/// Two images of one size side by side.
fn side_by_side(a: &Image, b: &Image) -> Image {
    let (w, h) = (a.0, a.1);
    let mut out = vec![0u8; (w * 2 * h * 4) as usize];
    for y in 0..h as usize {
        let row = w as usize * 4;
        out[y * row * 2..y * row * 2 + row].copy_from_slice(&a.2[y * row..(y + 1) * row]);
        out[y * row * 2 + row..(y + 1) * row * 2].copy_from_slice(&b.2[y * row..(y + 1) * row]);
    }
    (w * 2, h, out)
}

/// The Conquest Reforged instance installed here, read over the game it
/// plays, and its test map.
fn conquest() -> Option<(World, massif_assets::Catalogue)> {
    let instance = massif_assets::instance::find_with_mod("conquest")?;
    let jar = massif_assets::find_jars().into_iter().find(|j| Some(&j.version) == instance.minecraft.as_ref())?;
    let map = std::env::var_os("MASSIF_TEXTURE_CONQUEST").map(PathBuf::from).or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(home.join("Documents/Massif Conquest Test/Massif Conquest Test.massif")).filter(|p| p.is_file())
    })?;
    let world = massif_format::load(&map).ok()?;
    let cat = massif_assets::load_with_mods(&jar.path, &jar.version, &instance.files()).ok()?;
    Some((world, cat))
}

#[test]
fn the_games_textures_in_the_views_look() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let perf = std::env::var_os("MASSIF_PERF").is_some();
    if dir.is_none() && !perf {
        return;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).unwrap();
    let Some(Ok(vanilla)) = massif_assets::load_newest() else {
        eprintln!("Minecraft is not installed here: skipping");
        return;
    };
    let vanilla = Arc::new(vanilla);
    let world = match std::env::var_os("MASSIF_TEXTURE_WORLD") {
        Some(p) => massif_format::load(std::path::Path::new(&p)).unwrap(),
        None => generate(&GenParams { tiles_x: 12, tiles_z: 12, landform: Landform::Hills, seed: 11, ..GenParams::default() }),
    };
    let mut scenes = vec![scene(&device, &queue, "vanilla", world, &vanilla, None)];
    match conquest() {
        Some((world, cat)) => {
            // The plaza of Conquest's blocks, in the middle of its map.
            let focus = std::env::var("MASSIF_TEXTURE_FOCUS").ok().and_then(|v| {
                let (x, z) = v.split_once(',')?;
                Some((x.trim().parse().ok()?, z.trim().parse().ok()?))
            });
            scenes.push(scene(&device, &queue, "conquest", world, &cat, focus));
        }
        None => eprintln!("Conquest Reforged or its test map is not here: vanilla only"),
    }
    for sc in &mut scenes {
        for (cam, camera) in sc.cameras() {
            for ppp in [1u32, 2] {
                let size = [1600 * ppp, 1000 * ppp];
                // Still: a ray a sample, as the app draws once nothing moves.
                let flat = sc.draw(&camera, size, false, true, true, false).0.unwrap();
                let textured = sc.draw(&camera, size, true, true, true, false).0.unwrap();
                if let Some(dir) = &dir {
                    let (w, h, rgba) = side_by_side(&flat, &textured);
                    image::save_buffer(dir.join(format!("textures-{}-{cam}-{ppp}x.png", sc.name)), &rgba, w, h, image::ColorType::Rgba8).unwrap();
                }
                // Up close the blocks show their textures: much more
                // detail than flat colours. Where a block is smaller than a
                // pixel, the land stays as smooth as it was: no grain or
                // moiré added far off.
                let (a, b) = (detail(&flat), detail(&textured));
                let tiny = camera.distance * FOV_Y / size[1] as f32 > 1.0;
                eprintln!("{} {cam} {ppp}x: detail {a:.2} flat, {b:.2} textured", sc.name);
                if cam == "near" {
                    assert!(b > a * 1.3, "{} {cam}: textures show ({a:.2} → {b:.2})", sc.name);
                }
                if tiny {
                    assert!(b < a * 1.25, "{} {cam}: as smooth far off ({a:.2} → {b:.2})", sc.name);
                }
                if perf && ppp == 1 {
                    for (look, textured) in [("flat", false), ("textured", true)] {
                        let (_, moving) = sc.draw(&camera, [2560, 1440], textured, false, false, true);
                        let (_, still) = sc.draw(&camera, [2560, 1440], textured, true, false, true);
                        let (m, s) = (moving.unwrap(), still.unwrap());
                        eprintln!("{} {cam}, {look}, 2560 × 1440: moving best {:.2} ms (middle {:.2}), still best {:.2} ms (middle {:.2})", sc.name, m.0, m.1, s.0, s.1);
                    }
                }
            }
        }
    }
}

/// The map's sides (MASSIF_SNAPSHOT_DIR): a grassy map of mountains seen
/// whole, from as far as the 3D view frames it, and one of its sides up
/// close, flat and textured, at one and two pixels a point. A side is the
/// section through the land: its top block, the soil under it, the rock
/// down to the bottom of the world (no streaks of the tops' colours).
#[test]
fn the_maps_sides() {
    let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
        return;
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).unwrap();
    let Some(Ok(cat)) = massif_assets::load_newest() else {
        eprintln!("Minecraft is not installed here: skipping");
        return;
    };
    let world = generate(&GenParams { tiles_x: 6, tiles_z: 6, landform: Landform::Mountains, seed: 3, ..GenParams::default() });
    let mut sc = scene(&device, &queue, "sides", world, &cat, Some((0, 0)));
    let dim = &sc.world.dims[0];
    let (lo, hi) = dim.bounds().unwrap();
    let (x0, z1) = (lo.x * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
    let x1 = (hi.x + 1) * TILE_SIZE;
    let cx = (x0 + x1) / 2;
    let top = ground(dim, cx, z1 - 1);
    let whole = Camera { target: [cx as f64, 64.0, ((lo.z * TILE_SIZE + z1) / 2) as f64], yaw: 0.65, pitch: 0.42, distance: 1250.0 };
    // Outside the south side, looking at it, a little from one side.
    let side = Camera { target: [cx as f64, (top + dim.min_y as f64) / 2.0, z1 as f64], yaw: 0.35, pitch: 0.12, distance: 170.0 };
    for (name, camera) in [("whole", whole), ("side", side)] {
        for ppp in [1u32, 2] {
            let size = [1600 * ppp, 1000 * ppp];
            let flat = sc.draw(&camera, size, false, true, true, false).0.unwrap();
            let textured = sc.draw(&camera, size, true, true, true, false).0.unwrap();
            let (w, h, rgba) = side_by_side(&flat, &textured);
            image::save_buffer(dir.join(format!("sides-{name}-{ppp}x.png")), &rgba, w, h, image::ColorType::Rgba8).unwrap();
        }
    }
}
