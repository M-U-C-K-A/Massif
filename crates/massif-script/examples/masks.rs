//! Make the files of the 600-mask benchmark: a height map and `n` masks
//! (8-bit grey PNGs, black but for a few soft blobs, as a map maker draws
//! the zones of a layer), `size` pixels a side.
//!
//! `cargo run --release -p massif-script --example masks -- <dir> [n] [size]`

use std::path::Path;

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
    massif_script::testing::write_grey(&dir.join("heightmap.png"), size, &hm).expect("writable");

    use rayon::prelude::*;
    (0..n).into_par_iter().for_each(|i| {
        massif_script::testing::write_grey(&dir.join(format!("masks/mask{i}.png")), size, &massif_script::testing::mask(i, size)).expect("writable");
    });
    println!("{n} masks of {size}² and a height map in {}", dir.display());
}
