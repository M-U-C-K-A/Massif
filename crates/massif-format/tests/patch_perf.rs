//! What working together costs: the fingerprints of every tile of a map of
//! 16,384 × 16,384 blocks, and the size of a contribution of 1,000 × 1,000
//! blocks on a real map.
//!
//! `cargo test --release -p massif-format --test patch_perf -- --ignored --nocapture`
//! (MASSIF_PATCH_WORLD: the `.massif` of a real map; a generated one
//! otherwise.)

use massif_core::fingerprint::Prints;
use massif_core::generate::{GenParams, generate};
use massif_core::patch::{Kind, Patch};
use massif_core::versions::SavePoint;
use massif_core::{Brush, BrushShape, Profile, World};
use std::time::Instant;

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

/// The CPU time this process has used, user and system, in seconds.
fn cpu_seconds() -> f64 {
    let mut u = Rusage { user: Timeval { sec: 0, usec: 0 }, system: Timeval { sec: 0, usec: 0 }, rest: [0; 14] };
    // SAFETY: getrusage fills the struct it is given.
    if unsafe { getrusage(0, &mut u) } != 0 {
        return 0.0;
    }
    // tv_usec is 32 bits on macOS, in a 64-bit slot.
    let t = |t: &Timeval| t.sec as f64 + (t.usec & 0xffff_ffff) as f64 / 1e6;
    t(&u.user) + t(&u.system)
}

/// Fingerprints of every tile of a generated map of 128 × 128 tiles, the
/// best of three runs (wall and processor time).
#[test]
#[ignore]
fn fingerprints_of_a_16k_map() {
    let t = Instant::now();
    let mut world = generate(&GenParams { tiles_x: 128, tiles_z: 128, ..GenParams::default() });
    let world = world.snapshot();
    println!("generated 128 × 128 tiles in {:.1} s", t.elapsed().as_secs_f64());
    let blobs = world.dims[0].ground().blobs().unwrap();
    let bytes: u64 = blobs.iter().map(|(_, b)| b.len() as u64).sum();
    println!("{} tiles, {:.1} MB of tiles ({:.1} KB a tile)", blobs.len(), bytes as f64 / 1e6, bytes as f64 / blobs.len() as f64 / 1e3);
    let mut best = (f64::MAX, f64::MAX);
    for _ in 0..3 {
        let (t, cpu) = (Instant::now(), cpu_seconds());
        let prints = Prints::of(&world.dims[0]);
        let (wall, cpu) = (t.elapsed().as_secs_f64(), cpu_seconds() - cpu);
        assert_eq!(prints.len(), blobs.len());
        best = (best.0.min(wall), best.1.min(cpu));
    }
    println!("fingerprints: {:.3} s ({:.3} s of processor), {:.1} GB/s", best.0, best.1, bytes as f64 / best.0 / 1e9);
}

/// A contribution of a change over 1,000 × 1,000 blocks: built since a
/// version, written, read back and classified, with its size against the
/// map's.
#[test]
#[ignore]
fn a_contribution_on_a_real_map() {
    let mut world: World = match std::env::var_os("MASSIF_PATCH_WORLD") {
        Some(path) => massif_format::load(std::path::Path::new(&path)).unwrap(),
        None => generate(&GenParams { tiles_x: 46, tiles_z: 46, ..GenParams::default() }),
    };
    let file_len = massif_format::write(&world.snapshot(), Vec::new()).unwrap().file_len;
    let (lo, hi) = world.dims[0].bounds().unwrap();
    let t = massif_core::TILE_SIZE;
    let (cx, cz) = ((lo.x + hi.x + 1) * t / 2, (lo.z + hi.z + 1) * t / 2);
    println!("map: {} tiles, {:.1} MB on disk", world.dims[0].ground().len(), file_len as f64 / 1e6);
    let base = SavePoint::new(&mut world, "base", 1);
    world.points.push(base.clone());
    // Terraforming over 1,000 × 1,000 blocks: a hill every 100 blocks.
    let brush = Brush {
        shape: BrushShape::Circle,
        radius: 70.0,
        profile: Profile::Dome,
        strength: 1.0,
        rotation: 0.0,
        mask: None,
        filter: None,
    };
    let mut history = massif_core::history::History::new(4);
    history.begin("Raise", 0);
    for k in 0..10 {
        for j in 0..10 {
            let (x, z) = ((cx - 450 + k * 100) as f32, (cz - 450 + j * 100) as f32);
            massif_core::ops::raise(&mut history.edit(&mut world, 0), &brush, x, z, 6.0, massif_core::ops::Surface::Top);
        }
    }
    history.commit();
    let now = world.snapshot();
    let (t, cpu) = (Instant::now(), cpu_seconds());
    let patch = Patch::since(&base, &now, Kind::Contribution, "Jack", "Hills", 2).unwrap();
    let mut bytes = Vec::new();
    massif_format::write_patch(&patch, &mut bytes).unwrap();
    println!(
        "contribution: {} tiles, {:.0} KB ({:.1} KB a tile, {:.2} % of the map's file), built and written in {:.3} s ({:.3} s of processor)",
        patch.tiles(),
        bytes.len() as f64 / 1e3,
        patch.tile_bytes() as f64 / patch.tiles().max(1) as f64 / 1e3,
        bytes.len() as f64 * 100.0 / file_len as f64,
        t.elapsed().as_secs_f64(),
        cpu_seconds() - cpu
    );
    let (t, cpu) = (Instant::now(), cpu_seconds());
    let read = massif_format::read_patch(&bytes).unwrap();
    let review = massif_core::merge::classify(&base.world, &read);
    println!("read and classified in {:.3} s ({:.3} s of processor): {:?} (new, changed, conflicts), {} areas", t.elapsed().as_secs_f64(), cpu_seconds() - cpu, review.counts(), review.areas.len());
}
