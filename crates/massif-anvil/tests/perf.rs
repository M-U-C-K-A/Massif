//! How long exporting a map takes. Run by hand:
//! `cargo test -p massif-anvil --release --test perf -- --ignored --nocapture`.
//!
//! On one thread, the best of a few runs, so that other work on the machine
//! counts for little; each line ends with a digest of the region files (but
//! for the time they were written), to check that a change to the export
//! writes the same bytes.

use massif_anvil::{ExportOptions, Progress, Target, export};
use massif_core::generate::{GenParams, Landform, generate};
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

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
    // 1.18.2 and later, 1.16, 1.14 and the numbered blocks before 1.13.
    for version in ["1.18.2", "1.16.5", "1.14.4", "1.12.2"] {
        let options = ExportOptions {
            target: Target::release(version).unwrap(),
            overwrite: true,
            ..ExportOptions::default()
        };
        let mut best = Duration::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            pool.install(|| export(&world, &dir, &options, &Progress::default())).unwrap();
            best = best.min(t.elapsed());
        }
        eprintln!("export 8 × 8 tiles of islands for {version:<7} {:>8.1} ms   {:016x}", best.as_secs_f64() * 1000.0, digest(&dir.join("region")));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
