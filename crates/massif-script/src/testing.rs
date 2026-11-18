//! What the comparisons with WorldPainter share: the masks of the benchmark
//! and fingerprints of maps (counts of columns by terrain, layer, biome…),
//! taken of WorldPainter's result once and kept to test Massif's against.

use massif_core::World;
use massif_core::coords::TILE_AREA;
use std::collections::BTreeMap;
use std::path::Path;

/// Mask `i` of the benchmark, `size` pixels a side: black but for one to
/// four soft blobs, as a map maker draws the zones of a layer.
pub fn mask(i: usize, size: u32) -> Vec<u8> {
    let s = size as usize;
    let hash = |k: u64| massif_core::compose::hash2(k as i32, (k >> 32) as i32, 0x6d61_736b) as u64;
    let mut img = vec![0u8; s * s];
    let blobs = 1 + hash(i as u64 * 7) % 4;
    let scale = size as f64 / 4096.0;
    for b in 0..blobs {
        let k = i as u64 * 131 + b * 17;
        let (cx, cy) = ((hash(k) % size as u64) as f64, (hash(k + 1) % size as u64) as f64);
        let r = (40.0 + (hash(k + 2) % 360) as f64) * scale.max(0.25);
        let (x0, x1) = ((cx - r).max(0.0) as usize, ((cx + r) as usize).min(s - 1));
        let (y0, y1) = ((cy - r).max(0.0) as usize, ((cy + r) as usize).min(s - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() / r;
                if d < 1.0 {
                    // Solid in the middle, fading over the outer third.
                    let v = (((1.0 - d) * 3.0).min(1.0) * 255.0) as u8;
                    let p = &mut img[y * s + x];
                    *p = (*p).max(v);
                }
            }
        }
    }
    img
}

/// Write an 8-bit grey PNG.
pub fn write_grey(path: &Path, size: u32, data: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size, size);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header().map_err(std::io::Error::other)?.write_image_data(data).map_err(std::io::Error::other)
}

/// Counts of the surface's columns by what they hold: `height` (the sum of
/// the heights, in 1/16 of a block), `water` (columns under water),
/// `terrain:<name>`, `cover:<name>`, `trees:<name>`, `understory:<name>`,
/// `biome:<id>`, `annotation:<colour>` — by names, which two maps share
/// where their numbers differ.
pub fn fingerprint(world: &World) -> BTreeMap<String, i64> {
    let mut out: BTreeMap<String, i64> = BTreeMap::new();
    let reg = &world.registry;
    let ground = world.dims[0].ground();
    let mut positions: Vec<_> = ground.positions().collect();
    positions.sort_unstable_by_key(|p| (p.z, p.x));
    let mut add = |k: String, n: i64| *out.entry(k).or_default() += n;
    for pos in positions {
        let Some(t) = ground.view(pos) else { continue };
        for i in 0..TILE_AREA {
            add("height".into(), (t.top.get(i) * 16.0).round() as i64);
            if t.fluid.get(i) as f32 > t.top.get(i).round() {
                add("water".into(), 1);
            }
            add(format!("terrain:{}", reg.terrains.get(t.terrain.get(i) as usize).map_or("?", |d| d.name.as_str())), 1);
            let named = |what: &str, n: u16, name: Option<&str>| (n != 0).then(|| format!("{what}:{}", name.unwrap_or("?")));
            for k in [
                named("cover", t.cover.get(i), reg.cover(t.cover.get(i)).map(|c| c.name.as_str())),
                named("trees", t.objects.get(i), reg.object_layer(t.objects.get(i)).map(|c| c.name.as_str())),
                named("understory", t.understory.get(i), reg.object_layer(t.understory.get(i)).map(|c| c.name.as_str())),
                named("biome", t.biome.get(i), reg.biomes.get(t.biome.get(i) as usize).map(|b| b.id.as_str())),
                (t.annotations.get(i) != 0).then(|| format!("annotation:{}", t.annotations.get(i))),
            ]
            .into_iter()
            .flatten()
            {
                add(k, 1);
            }
        }
    }
    out
}

/// The entries of `fingerprint` whose keys start with one of `prefixes`
/// (all of them for none).
pub fn only(fingerprint: &BTreeMap<String, i64>, prefixes: &[&str]) -> BTreeMap<String, i64> {
    fingerprint.iter().filter(|(k, _)| prefixes.is_empty() || prefixes.iter().any(|p| k.starts_with(p))).map(|(k, v)| (k.clone(), *v)).collect()
}
