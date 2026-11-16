//! Damaged and hostile WorldPainter `.world` files: importing one returns an
//! error, never a panic or an allocation without bound. See
//! `massif-core/tests/fuzz_support`.
//!
//! The seed is the first 200 KB of a real world's serialization stream (its
//! classes, settings and first tiles). `MASSIF_FUZZ_WORLD=<a .world>` fuzzes
//! a whole real world as well.

#[path = "../../massif-core/tests/fuzz_support/mod.rs"]
mod fuzz_support;

#[global_allocator]
static ALLOC: fuzz_support::Capped = fuzz_support::Capped;

use massif_legacy::ImportProgress;
use std::io::Read;
use std::path::Path;

fn inflate(gz: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut out).unwrap();
    out
}

fn here(p: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(p)
}

fn import(bytes: &[u8]) {
    let _ = massif_legacy::worldpainter::import_stream(bytes, &ImportProgress::default());
}

#[test]
fn damaged_worldpainter_streams_are_refused_without_panicking() {
    let mut seeds = vec![inflate(&std::fs::read(here("seeds/snow-head.bin.gz")).unwrap())];
    if let Ok(path) = std::env::var("MASSIF_FUZZ_WORLD") {
        seeds.push(inflate(&std::fs::read(path).unwrap()));
    }
    fuzz_support::fuzz("world", &seeds, &here("corpus/world"), import);
}

#[test]
fn damaged_worldpainter_files_are_refused_without_panicking() {
    // The gzip layer and the picture in its header, through the file.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fuzz-world");
    std::fs::create_dir_all(&dir).unwrap();
    let seed = std::fs::read(here("seeds/snow-head.bin.gz")).unwrap();
    let path = dir.join("map.world");
    let mut rng = fuzz_support::Rng::new(7);
    let cases: Vec<Vec<u8>> = (0..40).map(|_| fuzz_support::mutate(&seed[..seed.len().min(4096)], &mut rng)).collect();
    fuzz_support::fuzz("world-file", &cases, &here("corpus/world-file"), |bytes| {
        std::fs::write(&path, bytes).unwrap();
        let _ = massif_legacy::import_world(&path, None);
        let _ = massif_legacy::worldpainter::read_thumbnail(bytes);
    });
}
