//! Make the files of the 600-mask benchmark: a height map and `n` masks
//! (8-bit grey PNGs, black but for a few soft blobs, as a map maker draws
//! the zones of a layer), `size` pixels a side.
//!
//! `cargo run --release -p massif-script --example masks -- <dir> [n] [size]`

use std::io::BufWriter;
use std::path::Path;

fn write_grey(path: &Path, size: u32, data: &[u8]) {
    let file = std::fs::File::create(path).expect("writable");
    let mut enc = png::Encoder::new(BufWriter::new(file), size, size);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header().unwrap().write_image_data(data).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = Path::new(&args[0]);
    let n: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(600);
    let size: u32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(4096);
    std::fs::create_dir_all(dir.join("masks")).unwrap();
    let s = size as usize;

    // Hills and valleys, from 40 to 160 (the sea at 62).
    let noise = massif_core::noise::Noise::new(7);
    let mut hm = vec![0u8; s * s];
    hm.chunks_mut(s).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let h = noise.fbm(x as f64 / 900.0, y as f64 / 900.0, 6, 2.0, 0.5);
            *v = (100.0 + h * 70.0).clamp(0.0, 255.0) as u8;
        }
    });
    write_grey(&dir.join("heightmap.png"), size, &hm);

    let hash = |k: u64| massif_core::compose::hash2(k as i32, (k >> 32) as i32, 0x6d61_736b) as u64;
    use rayon::prelude::*;
    (0..n).into_par_iter().for_each(|i| {
        let mut img = vec![0u8; s * s];
        let blobs = 1 + hash(i as u64 * 7) % 4;
        for b in 0..blobs {
            let k = i as u64 * 131 + b * 17;
            let (cx, cy) = ((hash(k) % size as u64) as f64, (hash(k + 1) % size as u64) as f64);
            let r = 40.0 + (hash(k + 2) % 360) as f64;
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
        write_grey(&dir.join(format!("masks/mask{i}.png")), size, &img);
    });
    println!("{n} masks of {size}² and a height map in {}", dir.display());
}
