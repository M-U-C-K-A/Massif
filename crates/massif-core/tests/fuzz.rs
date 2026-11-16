//! Damaged and hostile files read by the core: shared packs, height maps
//! (PNG, JPEG, raw grids), GeoTIFFs and elevation tiles. Each returns an
//! error, never a panic or an allocation without bound. See `fuzz_support`.

mod fuzz_support;

#[global_allocator]
static ALLOC: fuzz_support::Capped = fuzz_support::Capped;

use massif_core::pack_file::Pack;
use massif_core::packs::{MATERIAL_MIXES, PLANT_PRESETS};
use massif_core::registry::Registry;
use massif_core::treepack::TreeKind;
use std::path::{Path, PathBuf};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus").join(name)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fuzz-core");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn pack_seed() -> Vec<u8> {
    let mut reg = Registry::vanilla();
    let layer = TreeKind::Fir.install(&mut reg);
    let cover = PLANT_PRESETS.iter().find(|p| p.name == "Tulip field").unwrap().install(&mut reg);
    let terrain = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut reg).0;
    let content = Pack::gather(&reg, &[cover], &[terrain], &[layer]);
    let mut pack = Pack { name: "Woods".into(), author: "Me".into(), description: "Firs".into(), made_with: "0.6.0".into(), preview: None, content };
    pack.preview = Some(pack.draw_preview());
    pack.to_bytes()
}

/// The pack with its entries stored rather than deflated, so that
/// mutations reach the JSON and the blocks; their CRCs are set right after
/// each mutation by [`fix_stored_crcs`].
fn stored(zip: &[u8]) -> Vec<u8> {
    // Rebuild with method 0: read every entry back through the public
    // reader would need its API, so the test writes a zip of its own.
    let names = ["manifest.json", "content.json", "preview.png"];
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut at = 0;
    while at + 30 <= zip.len() && zip[at..at + 4] == [0x50, 0x4b, 3, 4] {
        let u16_at = |i: usize| u16::from_le_bytes([zip[i], zip[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes(zip[i..i + 4].try_into().unwrap()) as usize;
        let (method, compressed, name_len, extra_len) = (u16_at(at + 8), u32_at(at + 18), u16_at(at + 26), u16_at(at + 28));
        let name = String::from_utf8(zip[at + 30..at + 30 + name_len].to_vec()).unwrap();
        let start = at + 30 + name_len + extra_len;
        let raw = &zip[start..start + compressed];
        let data = if method == 8 {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(raw), &mut out).unwrap();
            out
        } else {
            raw.to_vec()
        };
        files.push((name, data));
        at = start + compressed;
    }
    assert!(names.iter().all(|n| files.iter().any(|(f, _)| f == n)));
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in &files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let header = out.len() as u32;
        let mut local = vec![0x50, 0x4b, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        local.extend_from_slice(&crc.sum().to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        let mut c = vec![0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        c.extend_from_slice(&crc.sum().to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(data.len() as u32).to_le_bytes());
        c.extend_from_slice(&(name.len() as u16).to_le_bytes());
        c.extend_from_slice(&[0; 12]);
        c.extend_from_slice(&header.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&c);
    }
    let dir = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&dir.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn damaged_packs_are_refused_without_panicking() {
    let deflated = pack_seed();
    let plain = stored(&deflated);
    assert!(Pack::from_bytes(&deflated).is_ok());
    assert!(Pack::from_bytes(&plain).is_ok(), "{:?}", Pack::from_bytes(&plain).err());
    // Changes inside the stored entries, their checksums made right: the
    // JSON and the blocks are what is tried.
    let mut seeds = vec![deflated];
    let mut rng = fuzz_support::Rng::new(77);
    let entries = |b: &[u8]| -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 30 <= b.len() && b[at..at + 4] == [0x50, 0x4b, 3, 4] {
            let n = u16::from_le_bytes([b[at + 26], b[at + 27]]) as usize;
            let len = u32::from_le_bytes(b[at + 18..at + 22].try_into().unwrap()) as usize;
            out.push((String::from_utf8_lossy(&b[at + 30..at + 30 + n]).into_owned(), b[at + 30 + n..at + 30 + n + len].to_vec()));
            at += 30 + n + len;
        }
        out
    };
    let files = entries(&plain);
    for _ in 0..std::env::var("MASSIF_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(300) {
        let k = rng.below(files.len());
        let mut changed = files.clone();
        changed[k].1 = fuzz_support::mutate(&files[k].1, &mut rng);
        seeds.push(rezip(&changed));
    }
    seeds.push(plain);
    fuzz_support::fuzz("pack", &seeds, &corpus("pack"), |bytes| {
        if let Ok(pack) = Pack::from_bytes(bytes) {
            // What the library and the panel go on to do with one.
            let _ = pack.draw_preview();
            let mut reg = Registry::vanilla();
            let items: Vec<_> = pack.items().map(|(k, i, _)| (k, i)).collect();
            for (k, i) in items {
                pack.install(k, i, &mut reg);
            }
        }
    });
}

/// Stored entries back into a zip.
fn rezip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut local = Vec::new();
    for (name, data) in files {
        local.extend_from_slice(&[0x50, 0x4b, 3, 4]);
        local.extend_from_slice(&[0; 14]);
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        local.extend_from_slice(data);
    }
    // Let `stored` write the CRCs and the directory.
    stored(&local)
}

#[test]
fn a_pack_bomb_is_refused() {
    // One entry saying it is small but inflating to a gigabyte.
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    let zeros = vec![b' '; 1 << 24];
    for _ in 0..64 {
        std::io::Write::write_all(&mut deflate, &zeros).unwrap();
    }
    let data = deflate.finish().unwrap();
    let name = b"content.json";
    let mut zip = vec![0x50, 0x4b, 3, 4, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
    zip.extend_from_slice(&100u32.to_le_bytes());
    zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
    zip.extend_from_slice(&0u16.to_le_bytes());
    zip.extend_from_slice(name);
    zip.extend_from_slice(&data);
    assert!(Pack::from_bytes(&zip).is_err());
}

fn png(w: u32, h: u32, sixteen: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let cursor = &mut std::io::Cursor::new(&mut out);
    if sixteen {
        image::ImageBuffer::<image::Luma<u16>, _>::from_fn(w, h, |x, y| image::Luma([(x * 1000 + y * 7) as u16])).write_to(cursor, image::ImageFormat::Png).unwrap();
    } else {
        image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 9])).write_to(cursor, image::ImageFormat::Png).unwrap();
    }
    out
}

#[test]
fn damaged_height_maps_are_refused_without_panicking() {
    let mut jpeg = Vec::new();
    image::RgbImage::from_fn(24, 16, |x, y| image::Rgb([x as u8 * 9, y as u8 * 11, 0])).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let r16: Vec<u8> = (0..16 * 16u16).flat_map(|v| (v * 200).to_le_bytes()).collect();
    let seeds = vec![png(20, 12, true), png(16, 16, false), jpeg, r16];
    let exts = ["png", "png", "jpg", "r16", "r32", "raw", "tif", "bmp"];
    fuzz_support::fuzz("heightmap", &seeds, &corpus("heightmap"), |bytes| {
        for ext in exts {
            let path = scratch(&format!("map-{:?}.{ext}", std::thread::current().id()).replace(['(', ')'], ""));
            std::fs::write(&path, bytes).unwrap();
            if let Ok(map) = massif_core::heightmap::read(&path) {
                let _ = map.sample(0.5, 3.0);
                let _ = map.sample(1e9, -1e9);
            }
        }
        let _ = massif_core::real::decode_terrarium(bytes);
    });
}

fn tiff(kind: u8) -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype};
    let mut out = std::io::Cursor::new(Vec::new());
    let mut enc = TiffEncoder::new(&mut out).unwrap();
    let (w, h) = (12u32, 9u32);
    match kind {
        0 => {
            let data: Vec<f32> = (0..w * h).map(|i| i as f32 * 1.5 - 20.0).collect();
            let mut image = enc.new_image::<colortype::Gray32Float>(w, h).unwrap();
            image.encoder().write_tag(tiff::tags::Tag::Unknown(33550), &[30.0f64, 30.0, 0.0][..]).unwrap();
            image.encoder().write_tag(tiff::tags::Tag::Unknown(33922), &[0.0f64, 0.0, 0.0, 6.8, 45.8, 0.0][..]).unwrap();
            image.encoder().write_tag(tiff::tags::Tag::Unknown(34735), &[1u16, 1, 0, 1, 1024, 0, 1, 2][..]).unwrap();
            image.encoder().write_tag(tiff::tags::Tag::Unknown(42113), "-9999").unwrap();
            image.write_data(&data).unwrap();
        }
        1 => {
            let data: Vec<i16> = (0..w * h).map(|i| i as i16 * 30 - 900).collect();
            enc.write_image::<colortype::GrayI16>(w, h, &data).unwrap();
        }
        _ => {
            let data: Vec<u8> = (0..w * h * 3).map(|i| i as u8).collect();
            enc.write_image::<colortype::RGB8>(w, h, &data).unwrap();
        }
    }
    out.into_inner()
}

#[test]
fn damaged_geotiffs_are_refused_without_panicking() {
    let seeds = vec![tiff(0), tiff(1), tiff(2)];
    for s in &seeds {
        let path = scratch("seed.tif");
        std::fs::write(&path, s).unwrap();
        assert!(massif_core::real::read_tiff(&path).is_ok());
    }
    fuzz_support::fuzz("geotiff", &seeds, &corpus("geotiff"), |bytes| {
        let path = scratch(&format!("map-{:?}.tif", std::thread::current().id()).replace(['(', ')'], ""));
        std::fs::write(&path, bytes).unwrap();
        if let Ok(t) = massif_core::real::read_tiff(&path) {
            let _ = t.map.sample(2.0, 2.0);
        }
    });
}

#[test]
fn images_that_claim_to_be_huge_are_refused() {
    // A PNG header of 100,000 × 100,000 pixels in front of little data.
    let mut big = png(4, 4, true);
    big[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    big[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    let crc = {
        let mut c = flate2::Crc::new();
        c.update(&big[12..29]);
        c.sum()
    };
    big[29..33].copy_from_slice(&crc.to_be_bytes());
    let path = scratch("huge.png");
    std::fs::write(&path, &big).unwrap();
    assert!(massif_core::heightmap::read(&path).is_err());
}
