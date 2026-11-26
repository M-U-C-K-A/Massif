//! The pictures to share, drawn off screen (needs a GPU), and the dialog.

use super::*;
use std::path::PathBuf;

pub(super) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// The world to draw: MASSIF_SHARE_WORLD (a .massif), else hills.
fn world() -> World {
    if let Some(path) = std::env::var_os("MASSIF_SHARE_WORLD") {
        return massif_format::load(std::path::Path::new(&path)).expect("the map");
    }
    use massif_core::generate::{GenParams, Landform, generate};
    generate(&GenParams { tiles_x: 4, tiles_z: 4, landform: Landform::Hills, seed: 7, ..GenParams::default() })
}

#[test]
fn cards_render() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let world = world();
    let info = Info::of(&world, 0, Some("1.21.4".into()));
    let source = Source { world, dim: 0, catalogue: None, info };
    let size: [u32; 2] = std::env::var("MASSIF_SHARE_SIZE").ok().and_then(|s| s.split_once('x').map(|(a, b)| [a.parse().unwrap(), b.parse().unwrap()])).unwrap_or([480, 270]);
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut cards = Vec::new();
    let last = std::sync::Mutex::new(-1.0f32);
    let progress = |p: f32| {
        let mut l = last.lock().unwrap();
        // The steps between phases, not the build's ticks.
        if p >= *l + 0.05 || p >= 1.0 {
            eprintln!("  {:>5.2} s: {:.0} %", started.elapsed().as_secs_f64(), p * 100.0);
            *l = p;
        }
    };
    draw_all(&device, &queue, &source, size, &cancel, &progress, &mut |k, img| cards.push((k, img))).unwrap();
    eprintln!("share: {} cards of {size:?} in {:.2} s", cards.len(), started.elapsed().as_secs_f64());
    assert_eq!(cards.len(), 4);
    for (k, img) in &cards {
        assert_eq!(img.dimensions(), (size[0], size[1]), "{k:?}");
    }
    if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
        for (k, img) in &cards {
            for dark in [true, false] {
                finished(img, &source.info, Band { shown: true, dark }).save(dir.join(format!("share-{}-{}.png", k.slug(), if dark { "dark" } else { "light" }))).unwrap();
            }
        }
    }
}
