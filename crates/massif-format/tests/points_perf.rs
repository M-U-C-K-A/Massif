//! Save points on a large map: how much each adds to the file, and how
//! long saving and opening take with them.
//!
//! `cargo test --release -p massif-format --test points_perf -- --ignored --nocapture`
//! (MASSIF_POINTS_TILES: tiles a side, 64 by default.)

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::versions::SavePoint;
use massif_core::{Brush, BrushShape, Profile};
use std::time::Instant;

/// The CPU time this process has used, user and system, in seconds.
fn cpu() -> f64 {
    // getrusage's struct rusage: two timevals first, then longs.
    #[repr(C)]
    struct Timeval {
        sec: i64,
        usec: i64,
    }
    #[repr(C)]
    struct Rusage {
        user: Timeval,
        system: Timeval,
        rest: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut u = Rusage { user: Timeval { sec: 0, usec: 0 }, system: Timeval { sec: 0, usec: 0 }, rest: [0; 14] };
    // SAFETY: getrusage fills the struct it is given.
    if unsafe { getrusage(0, &mut u) } != 0 {
        return 0.0;
    }
    // tv_usec is 32 bits on macOS, in a 64-bit slot.
    let t = |t: &Timeval| t.sec as f64 + (t.usec & 0xffff_ffff) as f64 / 1e6;
    t(&u.user) + t(&u.system)
}

#[test]
#[ignore]
fn save_points_on_a_large_map() {
    let n: i32 = std::env::var("MASSIF_POINTS_TILES").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let mut world = generate(&GenParams {
        tiles_x: n,
        tiles_z: n,
        landform: Landform::Mountains,
        ..GenParams::default()
    });
    let mut hist = History::default();
    let brush = Brush {
        shape: BrushShape::Circle,
        radius: 100.0,
        profile: Profile::Dome,
        strength: 1.0,
        rotation: 0.0,
        mask: None,
        filter: None,
    };
    let best = |f: &mut dyn FnMut() -> usize| -> (f64, f64, usize) {
        let mut out = (f64::MAX, f64::MAX, 0);
        for _ in 0..3 {
            let (t, c) = (Instant::now(), cpu());
            let len = f();
            out = (out.0.min(t.elapsed().as_secs_f64()), out.1.min(cpu() - c), len);
        }
        out
    };
    let mut snapshot = world.snapshot();
    let (wall, cpu0, plain) = best(&mut || {
        let mut out = Vec::new();
        massif_format::write(&snapshot, &mut out).unwrap();
        out.len()
    });
    println!("{n}×{n} tiles, no point: {:.1} MB, written in {wall:.3} s ({cpu0:.3} s of processor)", plain as f64 / 1e6);

    // 30 points, the map changed in one place (a brush of 200 blocks)
    // between each.
    let half = n as f32 * 64.0;
    for k in 0..30 {
        let point = SavePoint::new(&mut world, format!("Version {k}"), k as u64);
        world.points.push(point);
        hist.begin("raise", 0);
        let (x, z) = ((k * 397 % (n * 128)) as f32 - half, (k * 211 % (n * 128)) as f32 - half);
        massif_core::ops::raise(&mut hist.edit(&mut world, 0), &brush, x, z, 20.0, massif_core::ops::Surface::Top);
        hist.commit();
    }
    snapshot = world.snapshot();
    // MASSIF_POINTS_OUT: the map with its points, to open elsewhere (in an
    // older version, say).
    if let Some(out) = std::env::var_os("MASSIF_POINTS_OUT") {
        massif_format::save(&snapshot, std::path::Path::new(&out)).unwrap();
    }
    let mut bytes = Vec::new();
    let (wall, cpu1, with) = best(&mut || {
        bytes.clear();
        let stats = massif_format::write(&snapshot, &mut bytes).unwrap();
        println!("  frames only the points use: {:.2} MB", stats.point_bytes as f64 / 1e6);
        bytes.len()
    });
    println!("with 30 points: {:.1} MB (+{:.1} MB, {:.2} MB a point), written in {wall:.3} s ({cpu1:.3} s of processor)", with as f64 / 1e6, (with - plain) as f64 / 1e6, (with - plain) as f64 / 30e6);
    let (wall, cpu2, _) = best(&mut || massif_format::read(&bytes).unwrap().points.len());
    let mut plain_bytes = Vec::new();
    let mut alone = world.snapshot();
    alone.points.clear();
    massif_format::write(&alone, &mut plain_bytes).unwrap();
    let (pwall, pcpu, _) = best(&mut || massif_format::read(&plain_bytes).unwrap().dims.len());
    println!("read: {wall:.3} s ({cpu2:.3} s of processor) with the points, {pwall:.3} s ({pcpu:.3} s) without");
    assert!(n < 32 || with - plain < plain / 2, "30 points cost {} bytes over {}", with - plain, plain);
}
