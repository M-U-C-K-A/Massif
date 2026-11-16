//! Damaged and hostile `.massif` files: reading, inspecting, verifying and
//! taking the picture of one returns an error, never a panic or an
//! allocation without bound. See `massif-core/tests/fuzz_support`.

#[path = "../../massif-core/tests/fuzz_support/mod.rs"]
mod fuzz_support;

#[global_allocator]
static ALLOC: fuzz_support::Capped = fuzz_support::Capped;

use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::level::LevelKind;
use massif_core::versions::SavePoint;
use massif_core::{Brush, BrushShape, Profile};
use std::path::Path;

fn brush(radius: f32) -> Brush {
    Brush { shape: BrushShape::Circle, radius, profile: Profile::Dome, strength: 1.0, rotation: 0.0, mask: None, filter: None }
}

/// A small map with every kind of section: two levels, sculpted chunks, a
/// selection and two save points.
fn seed() -> Vec<u8> {
    let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 1, landform: Landform::Hills, ..GenParams::default() });
    let mut hist = History::default();
    hist.begin("level", 0);
    let island = hist.edit(&mut world, 0).structure().add_level("Island", LevelKind::Solid);
    hist.commit();
    let point = SavePoint::new(&mut world, "before", 1);
    world.points.push(point);
    hist.begin("stamp", 0);
    massif_core::ops::stamp_slab(&mut hist.edit(&mut world, island), &brush(12.0), 5.0, 5.0, massif_core::ops::Anchor::Absolute(150.0), 10.0);
    massif_core::voxel::sculpt(&mut hist.edit(&mut world, 0), &brush(6.0), [10.5, 40.5, 10.5], 1.0, massif_core::voxel::SculptMode::Remove);
    hist.commit();
    world.dims[0].selection = Some(std::sync::Arc::new(massif_core::selection::Selection::shape(&massif_core::selection::Shape::Polygon(vec![(0.0, 0.0), (100.0, 10.0), (20.0, 90.0)]))));
    let point = SavePoint::new(&mut world, "after", 2);
    world.points.push(point);
    let mut bytes = Vec::new();
    massif_format::write(&world.snapshot(), &mut bytes).expect("a small map writes");
    bytes
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus").join(name)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// The section directory's entries: (where the entry lies, kind).
fn entries(b: &[u8]) -> Vec<(usize, [u8; 4])> {
    let (at, len) = (u64_at(b, 0x10) as usize, u32_at(b, 0x18) as usize);
    (0..len / 32).map(|i| (at + 32 * i, b[at + 32 * i..at + 32 * i + 4].try_into().unwrap())).collect()
}

/// Make every checksum and the length agree again, so that a mutation gets
/// past them to the decoders.
fn reseal(b: &mut [u8]) {
    if b.len() < 64 || b[..8] != massif_format::MAGIC {
        return;
    }
    let (dir, dir_len) = (u64_at(b, 0x10) as usize, u32_at(b, 0x18) as usize);
    if dir.checked_add(dir_len).is_none_or(|end| end > b.len()) {
        return;
    }
    for (e, _) in entries(b) {
        let (at, len) = (u64_at(b, e + 8) as usize, u64_at(b, e + 16) as usize);
        if let Some(payload) = at.checked_add(len).and_then(|end| b.get(at..end)) {
            let crc = crc32c::crc32c(payload);
            b[e + 24..e + 28].copy_from_slice(&crc.to_le_bytes());
        }
    }
    let crc = crc32c::crc32c(&b[dir..dir + dir_len]);
    b[0x1C..0x20].copy_from_slice(&crc.to_le_bytes());
    let len = b.len() as u64;
    b[0x20..0x28].copy_from_slice(&len.to_le_bytes());
    let crc = crc32c::crc32c(&b[..0x3C]);
    b[0x3C..0x40].copy_from_slice(&crc.to_le_bytes());
}

/// Mutate what a section or a tile holds once uncompressed, then write it
/// back compressed at the end of the file, the file resealed.
fn mutate_inside(seed: &[u8], rng: &mut fuzz_support::Rng) -> Vec<u8> {
    use massif_core::codec::{zstd_compress, zstd_decompress};
    let mut b = seed.to_vec();
    let sections = entries(&b);
    let (e, kind) = sections[rng.below(sections.len())];
    let (at, len) = (u64_at(&b, e + 8) as usize, u64_at(&b, e + 16) as usize);
    let zstd = u32_at(&b, e + 28) & 1 != 0;
    let payload = b[at..at + len].to_vec();
    let raw = if zstd { zstd_decompress(&payload).unwrap() } else { payload };
    if (kind == *b"TIDX" || kind == *b"VIDX") && rng.below(2) == 0 && raw.len() >= 24 {
        // A tile of this index, changed inside its frame.
        let mut index = raw;
        let t = 24 * rng.below(index.len() / 24);
        let (tat, tlen) = (u64_at(&index, t + 8) as usize, u32_at(&index, t + 16) as usize);
        let tile = zstd_decompress(&b[tat..tat + tlen]).unwrap();
        let frame = zstd_compress(&fuzz_support::mutate(&tile, rng));
        let to = b.len() as u64;
        b.extend_from_slice(&frame);
        index[t + 8..t + 16].copy_from_slice(&to.to_le_bytes());
        index[t + 16..t + 20].copy_from_slice(&(frame.len() as u32).to_le_bytes());
        index[t + 20..t + 24].copy_from_slice(&crc32c::crc32c(&frame).to_le_bytes());
        let packed = zstd_compress(&index);
        let to = b.len() as u64;
        b.extend_from_slice(&packed);
        b[e + 8..e + 16].copy_from_slice(&to.to_le_bytes());
        b[e + 16..e + 24].copy_from_slice(&(packed.len() as u64).to_le_bytes());
    } else {
        let changed = fuzz_support::mutate(&raw, rng);
        let packed = if zstd { zstd_compress(&changed) } else { changed };
        let to = b.len() as u64;
        b.extend_from_slice(&packed);
        b[e + 8..e + 16].copy_from_slice(&to.to_le_bytes());
        b[e + 16..e + 24].copy_from_slice(&(packed.len() as u64).to_le_bytes());
    }
    reseal(&mut b);
    b
}

/// Damaged files whose checksums agree: what a bug in a writer, or a file
/// changed on purpose, would give.
fn sealed_mutations(seed: &[u8]) -> Vec<Vec<u8>> {
    let n = std::env::var("MASSIF_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(300u64);
    let mut out = Vec::new();
    for i in 0..n {
        let mut rng = fuzz_support::Rng::new(0x5EA1 + i);
        out.push(mutate_inside(seed, &mut rng));
        let mut outer = fuzz_support::mutate(seed, &mut rng);
        reseal(&mut outer);
        out.push(outer);
    }
    out
}

#[test]
fn damaged_massif_files_are_refused_without_panicking() {
    let seed = seed();
    assert!(massif_format::read(&seed).is_ok());
    let mut seeds = vec![seed.clone()];
    seeds.extend(sealed_mutations(&seed));
    fuzz_support::fuzz("massif", &seeds, &corpus("massif"), |bytes| {
        if let Ok(mut world) = massif_format::read(bytes) {
            // What opening a map goes on to do: decode its tiles.
            for dim in &mut world.dims {
                for level in &mut dim.levels {
                    let at: Vec<_> = level.positions().collect();
                    for p in at {
                        let _ = level.tile_mut(p).is_some();
                    }
                }
            }
        }
        let _ = massif_format::inspect(bytes);
        let _ = massif_format::verify(bytes);
    });
}

#[test]
fn damaged_pictures_of_massif_files_are_refused_without_panicking() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fuzz-thumbnail");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.massif");
    let seed = seed();
    fuzz_support::fuzz("massif-thumbnail", &[seed], &corpus("massif-thumbnail"), |bytes| {
        let path = path.with_extension(format!("{}.massif", std::thread::current().name().unwrap_or("t").replace(':', "_")));
        std::fs::write(&path, bytes).unwrap();
        let _ = massif_format::read_thumbnail(&path);
    });
}

/// A map sculpted over thousands of chunks, its uncompressed sculpted
/// chunks well over the 16 MB a tile may take, opens again.
#[test]
fn a_map_sculpted_far_and_wide_opens_again() {
    let mut world = generate(&GenParams { tiles_x: 3, tiles_z: 3, landform: Landform::Mountains, ..GenParams::default() });
    let mut hist = History::default();
    hist.begin("dig", 0);
    for i in 0..10 {
        for k in 0..10 {
            for y in [-30.5, 10.5, 50.5, 90.5, 130.5] {
                let at = [16.5 + 36.0 * i as f32, y, 16.5 + 36.0 * k as f32];
                massif_core::voxel::sculpt(&mut hist.edit(&mut world, 0), &brush(30.0), at, 1.0, massif_core::voxel::SculptMode::Remove);
            }
        }
    }
    hist.commit();
    let chunks = world.dims[0].voxels.len();
    let mut bytes = Vec::new();
    massif_format::write(&world.snapshot(), &mut bytes).unwrap();
    let back = massif_format::read(&bytes).expect("the map opens again");
    assert_eq!(back.dims[0].voxels.len(), chunks);
    assert!(chunks * (13 + massif_core::voxel::VOLUME) > 16 << 20, "only {chunks} chunks");
}
