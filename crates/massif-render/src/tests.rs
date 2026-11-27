//! The renderer end to end: a map built into chunks, and path-traced on
//! the graphics card (MASSIF_SNAPSHOT_DIR: the images; MASSIF_PERF: the
//! timings; MASSIF_RENDER_WORLD: a map of yours, .massif or .world).

use super::*;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::world::World;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn the_open_map_builds_into_chunks() {
    let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 3, ..GenParams::default() });
    let target = map_source::target_for(None);
    let extent = Extent::of_map(&world, 0, &target).unwrap();
    assert_eq!((extent.width, extent.depth), (16, 16));
    let materials = Arc::new(Materials::new(None, 64));
    let tiles = map_source::tiles_by_distance(&world, 0, 0.0, 0.0);
    let mut builder = MapBuilder::start(world, 0, target, materials.clone(), tiles);
    let built = builder.wait_all();
    assert!(builder.error().is_none(), "{:?}", builder.error());
    assert_eq!(built.len(), 4);
    let chunks: Vec<&scene::GpuChunk> = built.iter().flat_map(|b| &b.chunks).collect();
    assert_eq!(chunks.len(), 256);
    assert!(chunks.iter().all(|c| c.top > 0), "land everywhere");
    assert!(materials.len() > 3, "stone, grass, water and more");
    // The surface only: a few words a column (and a little more where the
    // large ore veins of the modern ores reach it).
    let bytes: usize = chunks.iter().map(|c| c.bytes()).sum();
    assert!(bytes < 256 * 256 * 4 * 17 / 16, "{bytes} bytes for 65,536 columns");
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_limits: adapter.limits(), ..Default::default() })).ok()
}

fn load(path: &Path) -> World {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("world")) {
        massif_legacy::import_world(path, None).unwrap().0
    } else {
        massif_format::load(path).unwrap()
    }
}

/// The processor time of this process so far.
fn cpu_time() -> Duration {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    let u = unsafe { usage.assume_init() };
    let t = |v: libc::timeval| Duration::from_secs(v.tv_sec as u64) + Duration::from_micros(v.tv_usec as u64);
    t(u.ru_utime) + t(u.ru_stime)
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1 << 20) as f64
}

#[test]
fn a_map_path_traced() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let perf = std::env::var_os("MASSIF_PERF").is_some();
    if dir.is_none() && !perf {
        return;
    }
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let cat = massif_assets::load_newest().and_then(Result::ok).map(Arc::new);
    if cat.is_none() {
        eprintln!("Minecraft is not installed here: flat colours");
    }
    let (world, name) = match std::env::var_os("MASSIF_RENDER_WORLD") {
        Some(p) => {
            let p = PathBuf::from(p);
            let name = p.file_stem().map(|s| s.to_string_lossy().replace(' ', "-")).unwrap_or_default();
            (load(&p), name)
        }
        None => (generate(&GenParams { tiles_x: 12, tiles_z: 12, landform: Landform::Hills, seed: 11, ..GenParams::default() }), "hills".to_owned()),
    };
    let sea = world.meta.sea_level as f64;
    let target = map_source::target_for(cat.as_ref().map(|c| c.version.as_str()));
    let extent = Extent::of_map(&world, 0, &target).unwrap();
    let materials = Arc::new(Materials::new(cat.clone(), device.limits().max_texture_array_layers as usize));
    let center = extent.center();
    let mut tiles = map_source::tiles_by_distance(&world, 0, center[0], center[2]);
    // MASSIF_RENDER_TILES: only the nearest tiles of a huge map.
    if let Some(n) = std::env::var("MASSIF_RENDER_TILES").ok().and_then(|v| v.parse().ok()) {
        tiles.truncate(n);
    }
    let tile_count = tiles.len();
    let (wall, cpu) = (Instant::now(), cpu_time());
    let mut builder = MapBuilder::start(world, 0, target, materials.clone(), tiles);
    let built = builder.wait_all();
    assert!(builder.error().is_none(), "{:?}", builder.error());
    let (build_wall, build_cpu) = (wall.elapsed(), cpu_time() - cpu);
    let chunk_bytes: usize = built.iter().flat_map(|b| &b.chunks).map(|c| c.bytes()).sum();
    let chunk_count: usize = built.iter().map(|b| b.chunks.len()).sum();

    let mut r = Renderer::new(&device, &queue, extent);
    r.sea_level = sea;
    r.sync_materials(&materials);
    let started = Instant::now();
    for b in built {
        assert!(r.put(b.chunks), "the scene fits the graphics card");
    }
    r.flush();
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let upload = started.elapsed();
    eprintln!(
        "{name}: {} × {} chunks, {tile_count} tiles built in {:.2} s ({:.1} s of processor), {chunk_count} chunks, {:.1} MB of trees, put on the card in {:.0} ms; {} materials",
        extent.width,
        extent.depth,
        build_wall.as_secs_f64(),
        build_cpu.as_secs_f64(),
        mb(chunk_bytes as u64),
        upload.as_secs_f64() * 1000.0,
        materials.len()
    );

    let ground = |x: f64, z: f64| r.surface(x, z).unwrap_or(sea);
    let span = extent.width.max(extent.depth) as f64 * 16.0;
    let fov = 50f32.to_radians();
    // Noon over the whole map; golden hour in haze halfway; a foggy
    // morning close up, the lens focused on the point looked at.
    let noon = Light { cloud_cover: 0.45, cloud_height: sea as f32 + 1200.0, cloud_thickness: 500.0, cloud_size: 1200.0, ..Light::default() };
    let mut golden = Light { haze: 4.0, exposure: 2.5, ..Light::default() };
    golden.set_time(18.8, 172.0, 45.0, 0.0);
    let mut morning = Light { fog: 0.02, fog_height: sea as f32 + 6.0, fog_falloff: 14.0, aperture: 0.6, focus: 48.0, ..Light::default() };
    morning.set_time(7.0, 172.0, 45.0, 0.0);
    let views = [
        ("whole", Camera::orbit([center[0], ground(center[0], center[2]), center[2]], 0.65, -0.55, span * 0.75, fov), noon),
        ("halfway", Camera::orbit([center[0], ground(center[0], center[2]), center[2]], 2.2, -0.35, 260.0, fov), golden),
        ("near", Camera::orbit([center[0] + 40.0, ground(center[0] + 40.0, center[2] - 30.0), center[2] - 30.0], 4.0, -0.22, 48.0, fov), morning),
        ("sky", Camera::orbit([center[0], ground(center[0], center[2]) + 30.0, center[2]], 2.6, 0.12, 60.0, fov), noon),
        // Low over the sea by the west edge, toward the land, late in the day.
        ("water", Camera::orbit([extent.x0 as f64 * 16.0 + span * 0.12, sea + 1.0, center[2]], 1.35, -0.12, 30.0, fov), {
            let mut l = Light { haze: 2.0, exposure: 1.4, ..Light::default() };
            l.set_time(17.5, 172.0, 45.0, 0.0);
            l
        }),
    ];
    let (w, h) = (1280, 720);
    r.resize(w, h);
    for (view, camera, light) in views {
        // The first frame waits for the shader to be made.
        r.restart();
        let frame = |r: &mut Renderer, coarse: bool| {
            let t = Instant::now();
            let cmd = r.render(&camera, &light, coarse).unwrap();
            queue.submit([cmd]);
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            t.elapsed()
        };
        frame(&mut r, false);
        let mut moving: Vec<Duration> = (0..8).map(|_| frame(&mut r, true)).collect();
        r.restart();
        let mut still: Vec<Duration> = (0..16).map(|_| frame(&mut r, false)).collect();
        moving.sort();
        still.sort();
        let spp = if perf { 256 } else { 64 };
        let t = Instant::now();
        while r.samples < spp {
            frame(&mut r, false);
        }
        let converge = t.elapsed();
        let rays = |d: Duration| (w * h) as f64 / d.as_secs_f64() / 1e6;
        eprintln!(
            "  {view}: a sample a pixel at {w}×{h}: {:.1} ms best, {:.1} ms middle ({:.0} M samples/s); moving (a quarter of the pixels): {:.1} ms; {spp} samples in {:.2} s",
            still[0].as_secs_f64() * 1000.0,
            still[still.len() / 2].as_secs_f64() * 1000.0,
            rays(still[0]),
            moving[0].as_secs_f64() * 1000.0,
            converge.as_secs_f64()
        );
        if let Some(dir) = &dir {
            let (iw, ih, rgba) = r.read_image().unwrap();
            image::save_buffer(dir.join(format!("render-{name}-{view}.png")), &rgba, iw, ih, image::ColorType::Rgba8).unwrap();
            // Something was drawn: not one colour throughout.
            let first = &rgba[..4];
            assert!(rgba.chunks(4).any(|p| p != first), "{view}: a blank image");
        }
    }
    let m = r.memory();
    eprintln!("  on the graphics card: scene {:.1} MB, textures {:.1} MB, image {:.1} MB", mb(m.scene), mb(m.textures), mb(m.image));
}

/// A night lit by its own blocks: a courtyard of 4 × 4 chunks, glowstone
/// and a sea lantern on the ground, torches on the walls (not drawn yet,
/// their light only), a redstone torch (MASSIF_SNAPSHOT_DIR: the image).
#[test]
fn a_night_lit_by_its_blocks() {
    use crate::chunk::{Chunk, Section, VOLUME};
    use crate::scene::Patch;
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    if dir.is_none() && std::env::var_os("MASSIF_PERF").is_none() {
        return;
    }
    let Some((device, queue)) = device() else { return };
    let cat = massif_assets::load_newest().and_then(Result::ok).map(Arc::new);
    let materials = Materials::new(cat, device.limits().max_texture_array_layers as usize);
    let id = |name: &str, props: &[(&str, &str)]| materials.id(name, props);
    let (grass, bricks, glowstone, lantern, torch, redstone, cobble) = (
        id("minecraft:grass_block", &[]),
        id("minecraft:stone_bricks", &[]),
        id("minecraft:glowstone", &[]),
        id("minecraft:sea_lantern", &[]),
        id("minecraft:wall_torch", &[("facing", "south")]),
        id("minecraft:redstone_torch", &[("lit", "true")]),
        id("minecraft:cobblestone", &[]),
    );
    // Scene y: ground up to 16, walls 4 high around the middle.
    let mut patch = Patch::new(0, 0, 4, 4, 4);
    for cz in 0..4 {
        for cx in 0..4 {
            let mut lower = Box::new([cobble; VOLUME]);
            for z in 0..16 {
                for x in 0..16 {
                    lower[(15 * 16 + z) * 16 + x] = grass;
                }
            }
            let mut upper = Box::new([crate::materials::AIR; VOLUME]);
            let mut put = |x: i32, y: i32, z: i32, m: u16| {
                let (lx, lz) = (x - cx * 16, z - cz * 16);
                if (0..16).contains(&lx) && (0..16).contains(&lz) {
                    upper[((y as usize) * 16 + lz as usize) * 16 + lx as usize] = m;
                }
            };
            for k in 20..44 {
                for y in 0..4 {
                    // Walls around a yard from 20 to 43, a gap in the south.
                    put(k, y, 20, bricks);
                    put(20, y, k, bricks);
                    put(43, y, k, bricks);
                    if !(30..34).contains(&k) {
                        put(k, y, 43, bricks);
                    }
                }
            }
            put(26, 0, 26, glowstone);
            put(37, 0, 30, lantern);
            put(32, 2, 21, torch);
            put(21, 2, 36, torch);
            put(40, 0, 38, redstone);
            patch.put(Chunk { x: cx, z: cz, sections: vec![Section::Mixed(lower), Section::Mixed(upper), Section::EMPTY, Section::EMPTY] });
        }
    }
    let built = patch.build(&materials.classes(), &materials.lights());
    let extent = Extent { x0: 0, z0: 0, width: 4, depth: 4, min_y: 0, sections: 4 };
    let mut r = Renderer::new(&device, &queue, extent);
    r.sea_level = 0.0;
    r.sync_materials(&materials);
    assert!(r.put(built));
    r.flush();
    assert_eq!(r.light_count(), 5, "the glowstone, the lantern, two torches, the redstone torch");
    let mut night = Light { exposure: 1.6, ..Light::default() };
    night.set_time(23.0, 172.0, 45.0, 0.0);
    let camera = Camera::orbit([32.0, 16.5, 31.0], 3.4, -0.55, 30.0, 55f32.to_radians());
    r.resize(960, 600);
    let t = Instant::now();
    while r.samples < 256 {
        let cmd = r.render(&camera, &night, false).unwrap();
        queue.submit([cmd]);
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    eprintln!("night: 256 samples at 960×600 in {:.2} s", t.elapsed().as_secs_f64());
    if let Some(dir) = &dir {
        let (w, h, rgba) = r.read_image().unwrap();
        image::save_buffer(dir.join("render-night.png"), &rgba, w, h, image::ColorType::Rgba8).unwrap();
    }
}

/// Blocks drawn by their models, up close in the afternoon: stairs, slabs,
/// a fence, a wall, panes, a door, plants, torches, a lantern, snow
/// layers, a carpet, rails, a potted flower, a campfire
/// (MASSIF_SNAPSHOT_DIR: the image).
#[test]
fn models_up_close() {
    use crate::chunk::{Chunk, Section, VOLUME};
    use crate::scene::Patch;
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    if dir.is_none() && std::env::var_os("MASSIF_PERF").is_none() {
        return;
    }
    let Some((device, queue)) = device() else { return };
    let Some(Ok(cat)) = massif_assets::load_newest() else {
        eprintln!("Minecraft is not installed here: skipping");
        return;
    };
    let materials = Materials::new(Some(Arc::new(cat)), device.limits().max_texture_array_layers as usize);
    let id = |name: &str, props: &[(&str, &str)]| materials.id(&format!("minecraft:{name}"), props);
    let grass = id("grass_block", &[("snowy", "false")]);
    let bricks = id("stone_bricks", &[]);
    let mut blocks: Vec<(i32, i32, i32, u16)> = Vec::new();
    let mut put = |x: i32, y: i32, z: i32, m: u16| blocks.push((x, y, z, m));
    for x in 4..9 {
        put(x, 0, 4, id("oak_stairs", &[("facing", "north"), ("half", "bottom"), ("shape", "straight"), ("waterlogged", "false")]));
        put(x, 0, 6, id("stone_slab", &[("type", "bottom"), ("waterlogged", "false")]));
        put(x, 1, 7, id("stone_slab", &[("type", "top"), ("waterlogged", "false")]));
    }
    put(9, 0, 4, id("oak_stairs", &[("facing", "west"), ("half", "bottom"), ("shape", "outer_right"), ("waterlogged", "false")]));
    for x in 11..17 {
        let (w, e) = (if x > 11 { "true" } else { "false" }, if x < 16 { "true" } else { "false" });
        put(x, 0, 4, id("oak_fence", &[("east", e), ("west", w), ("north", "false"), ("south", "false"), ("waterlogged", "false")]));
    }
    for z in 8..14 {
        let (n, s) = (if z > 8 { "low" } else { "none" }, if z < 13 { "low" } else { "none" });
        let up = if z == 8 || z == 13 { "true" } else { "false" };
        put(17, 0, z, id("cobblestone_wall", &[("north", n), ("south", s), ("east", "none"), ("west", "none"), ("up", up), ("waterlogged", "false")]));
    }
    for x in 4..10 {
        let (w, e) = (if x > 4 { "true" } else { "false" }, if x < 9 { "true" } else { "false" });
        put(x, 0, 12, id("glass_pane", &[("east", e), ("west", w), ("north", "false"), ("south", "false"), ("waterlogged", "false")]));
        put(x, 1, 12, id("glass_pane", &[("east", e), ("west", w), ("north", "false"), ("south", "false"), ("waterlogged", "false")]));
    }
    for y in 0..3 {
        put(12, y, 14, bricks);
        put(13, y, 14, bricks);
    }
    put(12, 1, 13, id("wall_torch", &[("facing", "north")]));
    put(13, 0, 13, id("ladder", &[("facing", "north"), ("waterlogged", "false")]));
    put(13, 1, 13, id("ladder", &[("facing", "north"), ("waterlogged", "false")]));
    put(10, 0, 8, id("torch", &[]));
    put(14, 0, 10, id("lantern", &[("hanging", "false"), ("waterlogged", "false")]));
    put(15, 0, 6, id("oak_door", &[("facing", "east"), ("half", "lower"), ("hinge", "left"), ("open", "false"), ("powered", "false")]));
    put(15, 1, 6, id("oak_door", &[("facing", "east"), ("half", "upper"), ("hinge", "left"), ("open", "false"), ("powered", "false")]));
    for (x, z) in [(2, 14), (3, 14), (2, 15), (3, 15)] {
        put(x, 0, z, id("snow", &[("layers", "3")]));
    }
    for (x, z) in [(6, 14), (7, 14), (6, 15), (7, 15)] {
        put(x, 0, z, id("red_carpet", &[]));
    }
    for z in 1..7 {
        put(1, 0, z, id("rail", &[("shape", "north_south"), ("waterlogged", "false")]));
    }
    put(10, 0, 11, id("potted_poppy", &[]));
    put(3, 0, 10, id("campfire", &[("facing", "north"), ("lit", "true"), ("signal_fire", "false"), ("waterlogged", "false")]));
    let plants = ["short_grass", "poppy", "dandelion", "cornflower", "fern", "short_grass", "oxeye_daisy", "short_grass"];
    for k in 0..40 {
        let (x, z) = ((k * 7 + 3) % 18 + 1, (k * 11 + 5) % 4 + 17);
        put(x, 0, z, id(plants[k as usize % plants.len()], &[]));
    }
    put(9, 0, 18, id("tall_grass", &[("half", "lower")]));
    put(9, 1, 18, id("tall_grass", &[("half", "upper")]));
    // 2 × 2 chunks: grass up to scene y 15, the blocks from 16.
    let mut patch = Patch::new(0, 0, 2, 2, 2);
    for cz in 0..2 {
        for cx in 0..2 {
            let mut lower = Box::new([grass; VOLUME]);
            for k in 0..VOLUME - 256 {
                lower[k] = id("dirt", &[]);
            }
            let mut upper = Box::new([crate::materials::AIR; VOLUME]);
            for &(x, y, z, m) in &blocks {
                let (lx, lz) = (x - cx * 16, z - cz * 16);
                if (0..16).contains(&lx) && (0..16).contains(&lz) {
                    upper[((y as usize) * 16 + lz as usize) * 16 + lx as usize] = m;
                }
            }
            patch.put(Chunk { x: cx, z: cz, sections: vec![Section::Mixed(lower), Section::Mixed(upper)] });
        }
    }
    let built = patch.build(&materials.classes(), &materials.lights());
    let extent = Extent { x0: 0, z0: 0, width: 2, depth: 2, min_y: 0, sections: 2 };
    let mut r = Renderer::new(&device, &queue, extent);
    r.sea_level = 0.0;
    r.sync_materials(&materials);
    assert!(r.put(built));
    r.flush();
    let mut light = Light { haze: 2.0, ..Light::default() };
    light.set_time(16.5, 172.0, 45.0, 0.0);
    let (w, h) = (1280, 800);
    r.resize(w, h);
    let mut rain = Light { wetness: 0.8, puddles: 0.55, puddle_size: 5.0, haze: 3.0, exposure: 1.6, ..Light::default() };
    rain.set_time(18.6, 172.0, 45.0, 0.0);
    for (name, camera, light) in [
        ("models", Camera::orbit([10.0, 16.5, 10.0], 3.0, -0.5, 17.0, 55f32.to_radians()), light),
        ("models-plants", Camera::orbit([9.0, 16.4, 18.0], 0.4, -0.25, 7.0, 55f32.to_radians()), light),
        ("models-rain", Camera::orbit([9.0, 16.2, 9.0], 3.6, -0.22, 13.0, 55f32.to_radians()), rain),
    ] {
        r.restart();
        let t = Instant::now();
        while r.samples < 256 {
            let cmd = r.render(&camera, &light, false).unwrap();
            queue.submit([cmd]);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        eprintln!("{name}: 256 samples at {w}×{h} in {:.2} s", t.elapsed().as_secs_f64());
        if let Some(dir) = &dir {
            let (iw, ih, rgba) = r.read_image().unwrap();
            image::save_buffer(dir.join(format!("render-{name}.png")), &rgba, iw, ih, image::ColorType::Rgba8).unwrap();
        }
    }
}

/// A zip of `(name, bytes)` stored as they are (no compression, no CRC:
/// the reader needs neither), for a resource pack made in a test.
fn stored_zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let at = out.len() as u32;
        le32(&mut out, 0x0403_4b50);
        for x in [20u16, 0, 0, 0, 0] {
            le16(&mut out, x);
        }
        for x in [0u32, data.len() as u32, data.len() as u32] {
            le32(&mut out, x);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        le32(&mut central, 0x0201_4b50);
        for x in [20u16, 20, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        for x in [0u32, data.len() as u32, data.len() as u32] {
            le32(&mut central, x);
        }
        for x in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, 0);
        le32(&mut central, at);
        central.extend_from_slice(name.as_bytes());
    }
    let (dir_at, dir_len, count) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    for x in [0u16, 0, count, count] {
        le16(&mut out, x);
    }
    le32(&mut out, dir_len);
    le32(&mut out, dir_at);
    le16(&mut out, 0);
    out
}

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
    let mut bytes = std::io::Cursor::new(Vec::new());
    img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Shine and relief from a LabPBR pack, metals and polished stone, wet
/// ground at sunset (MASSIF_SNAPSHOT_DIR: the image).
#[test]
fn pbr_and_metals() {
    use crate::chunk::{Chunk, Section, VOLUME};
    use crate::scene::Patch;
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    if dir.is_none() && std::env::var_os("MASSIF_PERF").is_none() {
        return;
    }
    let Some((device, queue)) = device() else { return };
    let Some(Ok(cat)) = massif_assets::load_newest() else { return };
    // Stone bricks with mortar: bricks of 8 × 4 pixels, their edges bent
    // out, smooth bricks and rough mortar.
    let mortar = |x: u32, y: u32| y % 4 == 3 || (x + if (y / 4).is_multiple_of(2) { 0 } else { 4 }) % 8 == 7;
    let normal = png(16, 16, |x, y| {
        if mortar(x, y) {
            [128, 128, 200, 255]
        } else {
            let (dx, dy) = (if x % 8 == 0 { -40i32 } else { 0 }, if y % 4 == 0 { -40i32 } else { 0 });
            [(128 + dx) as u8, (128 + dy) as u8, 255, 255]
        }
    });
    let specular = png(16, 16, |x, y| if mortar(x, y) { [20, 10, 0, 255] } else { [210, 18, 0, 255] });
    let pack = stored_zip(&[("assets/minecraft/textures/block/stone_bricks_n.png", normal), ("assets/minecraft/textures/block/stone_bricks_s.png", specular)]);
    let archives = vec![massif_assets::zip::Archive::open(&cat.source).unwrap(), massif_assets::zip::Archive::from_bytes(pack).unwrap()];
    let library = crate::models::Library::of_archives(archives);
    let materials = Materials::with_library(Some(Arc::new(cat)), Some(library), device.limits().max_texture_array_layers as usize);
    let id = |name: &str| materials.id(&format!("minecraft:{name}"), &[]);
    let (floor, bricks, gold, iron, copper, glass, quartz) = (id("polished_andesite"), id("stone_bricks"), id("gold_block"), id("iron_block"), id("copper_block"), id("glass"), id("quartz_block"));
    assert!(materials.pbr().is_some(), "the pack's maps were read");
    let mut patch = Patch::new(0, 0, 2, 2, 2);
    for cz in 0..2 {
        for cx in 0..2 {
            let lower = Box::new([floor; VOLUME]);
            let mut upper = Box::new([crate::materials::AIR; VOLUME]);
            let mut put = |x: i32, y: i32, z: i32, m: u16| {
                let (lx, lz) = (x - cx * 16, z - cz * 16);
                if (0..16).contains(&lx) && (0..16).contains(&lz) {
                    upper[((y as usize) * 16 + lz as usize) * 16 + lx as usize] = m;
                }
            };
            for k in 4..28 {
                for y in 0..5 {
                    put(k, y, 26, bricks);
                    put(26, y, k, bricks);
                }
            }
            for (k, m) in [gold, iron, copper, quartz, glass].into_iter().enumerate() {
                put(8 + k as i32 * 3, 0, 18, m);
                put(8 + k as i32 * 3, 1, 18, m);
            }
            patch.put(Chunk { x: cx, z: cz, sections: vec![Section::Mixed(lower), Section::Mixed(upper)] });
        }
    }
    let built = patch.build(&materials.classes(), &materials.lights());
    let mut r = Renderer::new(&device, &queue, Extent { x0: 0, z0: 0, width: 2, depth: 2, min_y: 0, sections: 2 });
    r.sea_level = 0.0;
    r.sync_materials(&materials);
    assert!(r.put(built));
    r.flush();
    let mut light = Light { haze: 3.0, exposure: 1.8, wetness: 0.35, ..Light::default() };
    light.set_time(18.9, 172.0, 45.0, 120.0);
    r.resize(1280, 800);
    let camera = Camera::orbit([18.0, 17.5, 20.0], 3.6, -0.2, 16.0, 55f32.to_radians());
    let t = Instant::now();
    while r.samples < 384 {
        let cmd = r.render(&camera, &light, false).unwrap();
        queue.submit([cmd]);
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    eprintln!("pbr: 384 samples at 1280×800 in {:.2} s", t.elapsed().as_secs_f64());
    if let Some(dir) = &dir {
        let (w, h, rgba) = r.read_image().unwrap();
        image::save_buffer(dir.join("render-pbr.png"), &rgba, w, h, image::ColorType::Rgba8).unwrap();
    }
}

/// The Diorama of the sample village, drawn straight from the floors
/// (MASSIF_SNAPSHOT_DIR: the images at several zooms, the surface and the
/// caves; MASSIF_PERF: the times to build and draw).
#[test]
fn the_diorama_draws_the_village() {
    use crate::diorama::{Builder, DioramaGpu, View, colours, far_layout, sample};
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let cat = massif_assets::load_newest().and_then(Result::ok).map(Arc::new);
    if cat.is_none() {
        eprintln!("Minecraft is not installed here: flat colours");
    }
    let world = std::env::var_os("MASSIF_DIORAMA_WORLD").map_or_else(sample::village, |p| load(Path::new(&p)));
    let tiles: Vec<massif_core::TilePos> = world.dims[0].ground().positions().collect();
    let (lo, hi) = tiles.iter().fold((tiles[0], tiles[0]), |(lo, hi), t| (massif_core::TilePos::new(lo.x.min(t.x), lo.z.min(t.z)), massif_core::TilePos::new(hi.x.max(t.x), hi.z.max(t.z))));
    let materials = Arc::new(Materials::new(cat.clone(), device.limits().max_texture_array_layers as usize));
    let size = far_layout(tiles.len()).0 as usize;
    let started = Instant::now();
    let cpu = cpu_time();
    let mut builder = Builder::start(world, 0, map_source::target_for(cat.as_ref().map(|c| c.version.as_str())), materials.clone(), size).unwrap();
    builder.want(&tiles, true);
    let mut built = Vec::new();
    while built.len() < tiles.len() {
        built.push(builder.wait().expect("a tile"));
    }
    let (wall, thread) = (started.elapsed(), cpu_time() - cpu);
    let floors: usize = built.iter().map(|b| b.floors.len()).sum();
    eprintln!("diorama: {} tiles in {:.0} ms ({:.0} ms of processor, {:.1} a tile), {:.2} floors a column", tiles.len(), wall.as_secs_f64() * 1000.0, thread.as_secs_f64() * 1000.0, thread.as_secs_f64() * 1000.0 / tiles.len() as f64, floors as f64 / (tiles.len() * 16384) as f64);
    let mut gpu = DioramaGpu::new(&device, &queue, lo, hi, tiles.len(), builder.min_y);
    gpu.sync_materials(&materials);
    let views = materials.top_views();
    let water = materials.top_view(materials.id("minecraft:water", &[("level", "0")])).mean;
    let lava = materials.top_view(materials.id("minecraft:lava", &[("level", "0")])).mean;
    let by_tile: std::collections::HashMap<_, _> = built.iter().map(|b| (b.tile, b.floors.clone())).collect();
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let (vx, vz) = sample::VILLAGE;
    let shots: [(&str, [f64; 2], f32, Option<i32>); 6] = [
        ("village", [vx as f64, vz as f64], 1.0 / 8.0, None),
        ("houses", [vx as f64 - 20.0, vz as f64 - 20.0], 1.0 / 20.0, None),
        ("halfway", [vx as f64, vz as f64], 0.5, None),
        ("whole", [0.0, 0.0], 1.4, None),
        ("caves", [vx as f64, vz as f64], 1.0 / 3.0, Some(40)),
        ("caves-deep", [0.0, 0.0], 1.0, Some(10)),
    ];
    for (name, center, blocks_per_px, slice) in shots {
        for t in &tiles {
            let img = if slice.is_none() { by_tile[t].surface.clone() } else { colours(&by_tile[t], slice, size, &views, water, lava) };
            gpu.put_far(*t, &img);
        }
        let [w, h] = [1600u32, 1000];
        gpu.resize(w, h);
        let view = View { center, blocks_per_px, turn: 0, slice, background: [24, 24, 27], relief: 1.0 };
        let near = blocks_per_px <= 1.0;
        if near {
            let cx = |v: f64| (v / 128.0).floor() as i32;
            let (rx, rz) = ((w as f64 * blocks_per_px as f64 / 256.0).ceil() as i32 + 1, (h as f64 * blocks_per_px as f64 / 256.0).ceil() as i32 + 1);
            let mut wanted = Vec::new();
            for tz in cx(center[1]) - rz..=cx(center[1]) + rz {
                for tx in cx(center[0]) - rx..=cx(center[0]) + rx {
                    if by_tile.contains_key(&massif_core::TilePos::new(tx, tz)) {
                        wanted.push(massif_core::TilePos::new(tx, tz));
                    }
                }
            }
            for _ in 0..8 {
                gpu.put_near(&wanted, &|t| by_tile.get(&t).cloned(), slice);
            }
        }
        let mut best = Duration::MAX;
        for _ in 0..6 {
            let t = Instant::now();
            queue.submit([gpu.render(&view, near).unwrap()]);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            best = best.min(t.elapsed());
        }
        eprintln!("diorama {name}: {:.2} ms to draw {w} × {h} (best of 6)", best.as_secs_f64() * 1000.0);
        let (w, h, rgba) = gpu.read_image().unwrap();
        let mean: f64 = rgba.chunks(4).map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).sum::<f64>() / (w * h) as f64;
        eprintln!("diorama {name}: mean brightness {mean:.0}");
        if let Some(dir) = &dir {
            image::save_buffer(dir.join(format!("diorama-gpu-{name}.png")), &rgba, w, h, image::ColorType::Rgba8).unwrap();
        }
    }
}

/// The denoiser against the samples alone: the error of each image to a
/// reference of 1024 samples a pixel, after 1, 4, 16, 64 and 256 samples,
/// denoised and not; how many samples each takes to come within the error
/// of 256 raw samples; and what a denoising costs at 1920 × 1080
/// (MASSIF_SNAPSHOT_DIR: the images; MASSIF_PERF: the timings).
#[test]
fn the_denoiser_settles_the_image_sooner() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let perf = std::env::var_os("MASSIF_PERF").is_some();
    if dir.is_none() && !perf {
        return;
    }
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let cat = massif_assets::load_newest().and_then(Result::ok).map(Arc::new);
    let world = generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: Landform::Hills, seed: 11, ..GenParams::default() });
    let sea = world.meta.sea_level as f64;
    let target = map_source::target_for(cat.as_ref().map(|c| c.version.as_str()));
    let extent = Extent::of_map(&world, 0, &target).unwrap();
    let materials = Arc::new(Materials::new(cat.clone(), device.limits().max_texture_array_layers as usize));
    let center = extent.center();
    let tiles = map_source::tiles_by_distance(&world, 0, center[0], center[2]);
    let mut builder = MapBuilder::start(world, 0, target, materials.clone(), tiles);
    let built = builder.wait_all();
    let mut r = Renderer::new(&device, &queue, extent);
    r.sea_level = sea;
    r.sync_materials(&materials);
    for b in built {
        assert!(r.put(b.chunks));
    }
    let ground = r.surface(center[0], center[2]).unwrap_or(sea);
    // Among the hills late in the day, in haze: long shadows lit by the sky
    // and the land around, where the grain shows most.
    let camera = Camera::orbit([center[0], ground, center[2]], 2.2, -0.25, 60.0, 50f32.to_radians());
    let mut light = Light { haze: 3.0, exposure: 2.5, ..Light::default() };
    light.set_time(18.6, 172.0, 45.0, 0.0);
    let (w, h) = (960, 540);
    r.resize(w, h);
    let wait = |cmd: wgpu::CommandBuffer| {
        queue.submit([cmd]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    };
    // Linear light of an image's sRGB bytes.
    let linear = |rgba: &[u8]| -> Vec<f32> { rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).map(|v| (v as f32 / 255.0).powf(2.2)).collect() };
    let error = |a: &[f32], b: &[f32]| (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / a.len() as f32).sqrt();
    let save = |r: &Renderer, name: &str| {
        if let Some(dir) = &dir {
            let (iw, ih, rgba) = r.read_image().unwrap();
            image::save_buffer(dir.join(format!("denoise-{name}.png")), &rgba, iw, ih, image::ColorType::Rgba8).unwrap();
        }
    };
    r.restart();
    while r.samples < 1024 {
        let cmd = r.render(&camera, &light, false).unwrap();
        wait(cmd);
    }
    let reference = linear(&r.read_image().unwrap().2);
    save(&r, "reference-1024");
    // Every sample's error, raw and denoised.
    r.restart();
    let mut raw = Vec::new();
    let mut clean = Vec::new();
    // Open Image Denoise, where it is here: its error at the milestones.
    let mut intel = Vec::new();
    while r.samples < 256 {
        let cmd = r.render(&camera, &light, false).unwrap();
        wait(cmd);
        let n = r.samples;
        raw.push(error(&linear(&r.read_image().unwrap().2), &reference));
        if [1, 4, 16, 64, 256].contains(&n) {
            save(&r, &format!("{n:04}-raw"));
        }
        let cmd = r.denoise(1.0).unwrap();
        wait(cmd);
        clean.push(error(&linear(&r.read_image().unwrap().2), &reference));
        if [1, 4, 16, 64, 256].contains(&n) {
            save(&r, &format!("{n:04}-denoised"));
            if oidn::available() {
                let mut s = r.read_samples().unwrap();
                oidn::denoise(&mut s.color, Some(&s.albedo), Some(&s.normal), s.width as usize, s.height as usize).unwrap();
                r.show_linear(&s.color, s.exposure);
                intel.push((n, error(&linear(&r.read_image().unwrap().2), &reference)));
                save(&r, &format!("{n:04}-oidn"));
            }
        }
    }
    for (n, e) in &intel {
        eprintln!("  {n:>3} samples: error {e:.4} with Open Image Denoise");
    }
    for n in [1usize, 4, 16, 64, 256] {
        eprintln!("  {n:>3} samples: error {:.4} raw, {:.4} denoised", raw[n - 1], clean[n - 1]);
    }
    // How many samples, denoised, come within the error of so many raw.
    let first = |e: &[f32], goal: f32| e.iter().position(|&x| x <= goal).map_or(0, |k| k + 1);
    for n in [4usize, 16, 64] {
        eprintln!("  the error of {n} raw samples ({:.4}) reached after {} samples denoised", raw[n - 1], first(&clean, raw[n - 1]));
    }
    assert!(clean[3] < raw[3], "4 samples denoised are nearer the reference than raw");
    assert!(clean[15] < raw[15], "so are 16");
    // The cost of a denoising at 1920 × 1080, best of 20, at 1 sample a
    // pixel (the most passes); and of an empty submission, to take off.
    r.resize(1920, 1080);
    let cmd = r.render(&camera, &light, false).unwrap();
    wait(cmd);
    let empty = (0..20)
        .map(|_| {
            let t = Instant::now();
            wait(device.create_command_encoder(&Default::default()).finish());
            t.elapsed()
        })
        .min()
        .unwrap();
    let mut times: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            let cmd = r.denoise(1.0).unwrap();
            wait(cmd);
            t.elapsed().saturating_sub(empty)
        })
        .collect();
    let trace: Vec<Duration> = (0..10)
        .map(|_| {
            let t = Instant::now();
            let cmd = r.render(&camera, &light, false).unwrap();
            wait(cmd);
            t.elapsed()
        })
        .collect();
    times.sort();
    let best_trace = trace.iter().min().unwrap();
    if oidn::available() {
        let mut s = r.read_samples().unwrap();
        let t = Instant::now();
        let _ = r.read_samples().unwrap();
        let read = t.elapsed();
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let mut c = s.color.clone();
            let t = Instant::now();
            oidn::denoise(&mut c, Some(&s.albedo), Some(&s.normal), 1920, 1080).unwrap();
            best = best.min(t.elapsed());
        }
        oidn::denoise(&mut s.color, Some(&s.albedo), Some(&s.normal), 1920, 1080).unwrap();
        eprintln!("  at 1920 × 1080: Open Image Denoise {:.0} ms best of 5, reading the samples back {:.0} ms", best.as_secs_f64() * 1000.0, read.as_secs_f64() * 1000.0);
    }
    eprintln!("  at 1920 × 1080: a denoising {:.2} ms best, {:.2} ms middle; a sample {:.1} ms best", times[0].as_secs_f64() * 1000.0, times[10].as_secs_f64() * 1000.0, best_trace.as_secs_f64() * 1000.0);
}

/// The detailed grass: the village map's meadows path-traced with the
/// game's grass, then with blades of grass, at the grass's height and from
/// higher up, with what a sample costs each way at 1280 × 720
/// (MASSIF_SNAPSHOT_DIR: the images; MASSIF_PERF: the timings).
#[test]
fn blades_of_grass() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
    let perf = std::env::var_os("MASSIF_PERF").is_some();
    if dir.is_none() && !perf {
        return;
    }
    let Some((device, queue)) = device() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let cat = massif_assets::load_newest().and_then(Result::ok).map(Arc::new);
    let world = diorama::sample::village();
    let sea = world.meta.sea_level as f64;
    let target = map_source::target_for(cat.as_ref().map(|c| c.version.as_str()));
    let extent = Extent::of_map(&world, 0, &target).unwrap();
    let (vx, vz) = diorama::sample::VILLAGE;
    let (w, h) = (1280, 720);
    for blades in [false, true] {
        let materials = Arc::new(Materials::new(cat.clone(), device.limits().max_texture_array_layers as usize));
        materials.set_blades(blades);
        let tiles = map_source::tiles_by_distance(&world, 0, vx as f64, vz as f64);
        let mut builder = MapBuilder::start(world.copy_unfrozen(), 0, target.clone(), materials.clone(), tiles);
        let built = builder.wait_all();
        let mut r = Renderer::new(&device, &queue, extent);
        r.sea_level = sea;
        r.sync_materials(&materials);
        for b in built {
            assert!(r.put(b.chunks));
        }
        let mut light = Light { haze: 1.5, ..Light::default() };
        light.set_time(15.5, 172.0, 45.0, 0.0);
        if let Some(n) = std::env::var("MASSIF_BLADES").ok().and_then(|v| v.parse().ok()) {
            light.grass_blades = n;
        }
        if let Some(n) = std::env::var("MASSIF_BOUNCES").ok().and_then(|v| v.parse().ok()) {
            light.bounces = n;
        }
        r.resize(w, h);
        let (mx, mz) = (vx as f64 + 80.0, vz as f64 + 60.0);
        let ground = r.surface(mx, mz).unwrap_or(sea);
        let views = [
            ("low", Camera::orbit([mx, ground + 1.4, mz], 0.8, -0.05, 18.0, 55f32.to_radians())),
            ("high", Camera::orbit([mx, ground, mz], 0.8, -0.45, 45.0, 50f32.to_radians())),
        ];
        for (view, camera) in views {
            r.restart();
            let mut times = Vec::new();
            while r.samples < 64 {
                let t = Instant::now();
                let cmd = r.render(&camera, &light, false).unwrap();
                queue.submit([cmd]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                times.push(t.elapsed());
            }
            times.sort();
            let name = if blades { "blades" } else { "blocks" };
            eprintln!("  grass as {name}, {view}: a sample {:.1} ms best, {:.1} ms middle", times[0].as_secs_f64() * 1000.0, times[times.len() / 2].as_secs_f64() * 1000.0);
            if let Some(dir) = &dir {
                let cmd = r.denoise(1.0).unwrap();
                queue.submit([cmd]);
                let (iw, ih, rgba) = r.read_image().unwrap();
                image::save_buffer(dir.join(format!("grass-{view}-{name}.png")), &rgba, iw, ih, image::ColorType::Rgba8).unwrap();
            }
        }
    }
}
