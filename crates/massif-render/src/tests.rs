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
    // The surface only: a few words a column.
    let bytes: usize = chunks.iter().map(|c| c.bytes()).sum();
    assert!(bytes < 256 * 256 * 4, "{bytes} bytes for 65,536 columns");
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
    let tiles = map_source::tiles_by_distance(&world, 0, center[0], center[2]);
    let tile_count = tiles.len();
    let (wall, cpu) = (Instant::now(), cpu_time());
    let mut builder = MapBuilder::start(world, 0, target, materials.clone(), tiles);
    let built = builder.wait_all();
    assert!(builder.error().is_none(), "{:?}", builder.error());
    let (build_wall, build_cpu) = (wall.elapsed(), cpu_time() - cpu);
    let chunk_bytes: usize = built.iter().flat_map(|b| &b.chunks).map(|c| c.bytes()).sum();
    let chunk_count: usize = built.iter().map(|b| b.chunks.len()).sum();

    let mut r = Renderer::new(&device, &queue, extent);
    r.sync_materials(&materials);
    let started = Instant::now();
    for b in built {
        assert!(r.put(b.chunks), "the scene fits the graphics card");
    }
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
    let views = [
        ("whole", Camera::orbit([center[0], ground(center[0], center[2]), center[2]], 0.65, -0.55, span * 0.75, fov)),
        ("halfway", Camera::orbit([center[0], ground(center[0], center[2]), center[2]], 2.2, -0.35, 260.0, fov)),
        ("near", Camera::orbit([center[0] + 40.0, ground(center[0] + 40.0, center[2] - 30.0), center[2] - 30.0], 4.0, -0.22, 48.0, fov)),
    ];
    let (w, h) = (1280, 720);
    r.resize(w, h);
    let light = Light::default();
    for (view, camera) in views {
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
