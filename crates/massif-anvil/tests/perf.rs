//! How long exporting a map takes. Run by hand:
//! `cargo test -p massif-anvil --release --test perf -- --ignored --nocapture`.
//!
//! On one thread, the best of a few runs, in processor time (the process's
//! own, so that other work on the machine counts for little), with the
//! wall-clock time beside it; each line ends with a digest of the region files (but
//! for the time they were written), to check that a change to the export
//! writes the same bytes.

use massif_anvil::{ExportOptions, Progress, Target, export};
use massif_core::generate::{GenParams, Landform, generate};
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

/// The processor time this process has used so far.
fn cpu_time() -> Duration {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let time = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    time(usage.ru_utime) + time(usage.ru_stime)
}

/// The region files of `dir`, by name, without the sector of their
/// timestamps.
fn digest(dir: &Path) -> u64 {
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    let mut h = DefaultHasher::new();
    for path in names {
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4096..8192].fill(0);
        h.write(path.file_name().unwrap().as_encoded_bytes());
        h.write(&bytes);
    }
    h.finish()
}

#[test]
#[ignore]
fn exporting_a_map() {
    let world = generate(&GenParams {
        tiles_x: 8,
        tiles_z: 8,
        landform: Landform::Islands,
        seed: 7,
        ..GenParams::default()
    });
    let dir = std::env::temp_dir().join(format!("massif-perf-export-{}", std::process::id()));
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    // Today's chunks for the versions since 1.18.2, 1.16, 1.14 and the
    // numbered blocks before 1.13.
    for version in ["26.3", "1.21.11", "1.20.4", "1.18.2", "1.16.5", "1.14.4", "1.12.2"] {
        let options = ExportOptions {
            target: Target::release(version).unwrap(),
            overwrite: true,
            ..ExportOptions::default()
        };
        let (mut best, mut wall) = (Duration::MAX, Duration::MAX);
        for _ in 0..5 {
            let (t, cpu) = (Instant::now(), cpu_time());
            pool.install(|| export(&world, &dir, &options, &Progress::default())).unwrap();
            best = best.min(cpu_time() - cpu);
            wall = wall.min(t.elapsed());
        }
        eprintln!("export 8 × 8 tiles of islands for {version:<7} {:>8.1} ms CPU {:>8.1} ms wall   {:016x}", best.as_secs_f64() * 1000.0, wall.as_secs_f64() * 1000.0, digest(&dir.join(options.target.region_dir())));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
