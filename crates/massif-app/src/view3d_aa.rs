//! A coast seen at a player's eyes, as in the 3D view, in both looks and
//! at each preset's samples: what aliases far off (water, the steps of
//! slopes) and what smoothing it costs. Renders it (MASSIF_SNAPSHOT_DIR)
//! and times it (MASSIF_PERF): the best and middle of 16 frames.

use super::*;
use massif_core::generate::{GenParams, Landform, generate};

/// Where to stand: dry land a few blocks over the sea, the sea ahead.
fn shore(dim: &Dimension, sea: f32) -> ([f64; 3], f32) {
    // The ground's height and whether water lies over it.
    let top = |x: i32, z: i32| {
        dim.ground().view(TilePos::containing(x, z)).map(|v| {
            let (h, f) = (v.top.get(local_index(x, z)), v.fluid.get(local_index(x, z)));
            (h, f != NO_FLUID && f as f32 > h)
        })
    };
    let (lo, hi) = dim.bounds().unwrap();
    let t = TILE_SIZE;
    let (mx, mz) = ((lo.x + hi.x + 1) * t / 2, (lo.z + hi.z + 1) * t / 2);
    let mut spots: Vec<(i32, i32)> = (lo.z * t + 170..(hi.z + 1) * t - 170).step_by(8).flat_map(|z| (lo.x * t + 170..(hi.x + 1) * t - 170).step_by(8).map(move |x| (x, z))).collect();
    spots.sort_by_key(|&(x, z)| (x - mx).abs() + (z - mz).abs());
    for (x, z) in spots {
        let Some((h, f)) = top(x, z) else { continue };
        if f || h < sea + 2.0 || h > sea + 30.0 {
            continue;
        }
        // The sea a good way off in one of the four directions, land behind.
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let wet = (40..160).step_by(10).filter(|k| top(x + dx * k, z + dz * k).is_some_and(|(_, f)| f)).count();
            if wet >= 8 && top(x - dx * 30, z - dz * 30).is_some_and(|(_, f)| !f) {
                let yaw = (-(dx as f32)).atan2(-(dz as f32));
                return ([x as f64 + 0.5, h as f64 + 1.62, z as f64 + 0.5], yaw);
            }
        }
    }
    panic!("no shore on the map");
}

#[test]
fn a_coast_far_off_does_not_alias() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let perf = std::env::var_os("MASSIF_PERF").is_some();
    if dir.is_none() && !perf {
        return;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let world = generate(&GenParams { tiles_x: 16, tiles_z: 16, landform: Landform::Islands, seed: 7, ..GenParams::default() });
    let dim = &world.dims[0];
    let sea = world.meta.sea_level as f32;
    let (eye, yaw) = shore(dim, sea);
    eprintln!("eye at {eye:?}, yaw {yaw:.2}");
    let style = Style { mode: MapMode::Terrain, water: true, lava: false };
    let shape = GridShape::of(dim, PAGE_BUDGET).unwrap();
    let tiles: Vec<TilePos> = dim.ground().positions().collect();
    let built: Vec<(TilePos, TileBuild)> = tiles.iter().map(|&t| (t, build_tile(dim, &world.registry, sea, world.meta.seed, t, style))).collect();
    let substrate = Substrate { stone: [128; 3], deepslate: [80; 3], whole: false };
    // Low: one sample, two thirds of the pixels; Medium and High: four
    // samples, the traced ground once a pixel while moving, once a sample
    // still.
    let size = [1600u32, 1000];
    let presets: [(&str, u32, f32, bool); 4] = [("low", 1, 0.67, false), ("moving", 4, 1.0, false), ("still", 4, 1.0, true), ("full", 1, 1.0, false)];
    for (name, samples, scale, fine) in presets {
        let mut renderer = Renderer3d::with_samples(&device, &queue, samples);
        renderer.set_grid(shape.stride, shape.page0, shape.pages, &shape.present).unwrap();
        let mut pairs = Pairs::default();
        for (t, b) in &built {
            renderer.upload_columns(*t, &pairs.pack(b));
            renderer.upload(*t, &b.faces);
        }
        renderer.flush();
        renderer.set_palette(pairs.list());
        set_fine(&mut renderer, fine);
        let region = [(size[0] as f32 * scale).round() as u32, (size[1] as f32 * scale).round() as u32];
        renderer.ensure_target(region);
        for game in [false, true] {
            let mut times = Vec::new();
            let mut image = None;
            for k in 0..20 {
                let camera = Camera { target: eye, yaw: yaw + if k == 19 { 0.0 } else { 0.002 * k as f32 }, pitch: 0.06, distance: 1.0 };
                let origin = (eye[0].floor() as i32, eye[2].floor() as i32);
                let mut globals = camera.globals(origin, size[0] as f32 / size[1] as f32, 2048.0, if game { Sky::GAME } else { Sky::of(&crate::theme::Palette::DARK) }, region[1]);
                if game {
                    globals.look = [0.0, 1.0, 0.0, 0.0];
                    globals.fog = [192.0, 1536.0, globals.fog[2], globals.fog[3]];
                }
                let t = Instant::now();
                renderer.render(&globals, origin, &tiles, dim.min_y, substrate, region);
                renderer.wait();
                if (4..19).contains(&k) {
                    times.push(t.elapsed().as_secs_f32() * 1000.0);
                }
                if k == 19 {
                    image = renderer.read_image();
                }
            }
            times.sort_by(f32::total_cmp);
            let look = if game { "game" } else { "views" };
            eprintln!("{name} ({samples} samples, scale {scale}, {} a pixel), {look} look: best {:.2} ms, middle {:.2} ms", if fine { "a ray a sample" } else { "a ray" }, times[0], times[times.len() / 2]);
            if let (Some(dir), Some((w, h, rgba))) = (&dir, image) {
                image::save_buffer(dir.join(format!("coast-{look}-{name}.png")), &rgba, w, h, image::ColorType::Rgba8).unwrap();
            }
        }
    }
}
