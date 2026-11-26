//! Pictures of the tree generator's presets beside the tree pack's trees:
//! `TREEGEN_RENDER_DIR=<dir> cargo test -p massif-core --test treegen_render -- --nocapture`.

use massif_core::objects::ObjectDef;
use massif_core::treegen::{TreePreset, grow};
use massif_core::treepack::TreeKind;

/// The object seen from a corner, each block four pixels wide, as the
/// app's object pictures are.
fn picture(def: &ObjectDef) -> image::RgbaImage {
    let [w, h, l] = def.size.map(|s| s as usize);
    let (iw, ih) = ((w + l) * 2 + 2, w + l + 2 * h + 2);
    let mut img = image::RgbaImage::from_pixel(iw as u32, ih as u32, image::Rgba([235, 235, 232, 255]));
    for d in 0..w + l {
        for y in 0..h {
            for x in d.saturating_sub(l - 1)..=d.min(w - 1) {
                let z = d - x;
                let k = def.at(x, y, z);
                if k == 0 {
                    continue;
                }
                let c = def.color(k);
                let sx = (x + l - 1 - z) * 2;
                let sy = x + z + 2 * (h - 1 - y);
                for (dx, dy, f) in [(0, 0, 1.12), (1, 0, 1.12), (2, 0, 1.12), (3, 0, 1.12), (0, 1, 1.12), (1, 1, 1.12), (2, 1, 1.12), (3, 1, 1.12), (0, 2, 0.82), (1, 2, 0.82), (2, 2, 0.64), (3, 2, 0.64), (0, 3, 0.82), (1, 3, 0.82), (2, 3, 0.64), (3, 3, 0.64)] {
                    let (px, py) = ((sx + dx) as u32, (sy + dy) as u32);
                    if px < img.width() && py < img.height() {
                        let s = |v: u8| (v as f32 * f).min(255.0) as u8;
                        img.put_pixel(px, py, image::Rgba([s(c[0]), s(c[1]), s(c[2]), 255]));
                    }
                }
            }
        }
    }
    img
}

/// Pictures in a row on a light ground, scaled up.
fn row(defs: &[ObjectDef], scale: u32) -> image::RgbaImage {
    let pics: Vec<_> = defs.iter().map(picture).collect();
    let w: u32 = pics.iter().map(|p| p.width() + 8).sum();
    let h = pics.iter().map(|p| p.height()).max().unwrap_or(1);
    let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([235, 235, 232, 255]));
    let mut x = 0;
    for p in &pics {
        image::imageops::overlay(&mut out, p, x as i64, (h - p.height()) as i64);
        x += p.width() + 8;
    }
    image::imageops::resize(&out, w * scale, h * scale, image::imageops::FilterType::Nearest)
}

#[test]
fn render_presets() {
    let Ok(dir) = std::env::var("TREEGEN_RENDER_DIR") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let pack = |k: TreeKind| k.trees().into_iter().take(3).collect::<Vec<_>>();
    let pairs = [
        (TreePreset::Oak, TreeKind::BigOak),
        (TreePreset::Spruce, TreeKind::TallSpruce),
        (TreePreset::Birch, TreeKind::Birch),
        (TreePreset::Cherry, TreeKind::Cherry),
        (TreePreset::Palm, TreeKind::Palm),
        (TreePreset::Willow, TreeKind::Willow),
        (TreePreset::Acacia, TreeKind::Acacia),
        (TreePreset::Dead, TreeKind::Dead),
        (TreePreset::GiantSequoia, TreeKind::Redwood),
        (TreePreset::GreatOak, TreeKind::BigOak),
    ];
    let mut rows = Vec::new();
    for (preset, kind) in pairs {
        let mut grown = Vec::new();
        for seed in 1..4 {
            let mut p = preset.params();
            p.seed = seed;
            grown.push(grow(&p, preset.name()));
        }
        let mut p = preset.params();
        p.survival = true;
        grown.push(grow(&p, "s"));
        let (a, b) = (row(&pack(kind), 1), row(&grown, 1));
        let mut both = image::RgbaImage::from_pixel(a.width() + 40 + b.width(), a.height().max(b.height()), image::Rgba([235, 235, 232, 255]));
        let bh = both.height();
        image::imageops::overlay(&mut both, &a, 0, (bh - a.height()) as i64);
        // A rule between the pack's trees and the grown ones.
        for y in 0..both.height() {
            both.put_pixel(a.width() + 19, y, image::Rgba([160, 160, 160, 255]));
        }
        image::imageops::overlay(&mut both, &b, (a.width() + 40) as i64, (bh - b.height()) as i64);
        let name = preset.name().to_lowercase().replace(' ', "-");
        image::imageops::resize(&both, both.width() * 3, both.height() * 3, image::imageops::FilterType::Nearest).save(format!("{dir}/{name}.png")).unwrap();
        println!("{name}: {:?}, {} block states", grown[0].size, grown[0].palette.len());
        rows.push(both);
    }
    let w = rows.iter().map(|r| r.width()).max().unwrap();
    let h: u32 = rows.iter().map(|r| r.height() + 10).sum();
    let mut sheet = image::RgbaImage::from_pixel(w, h, image::Rgba([235, 235, 232, 255]));
    let mut y = 0;
    for r in &rows {
        image::imageops::overlay(&mut sheet, r, 0, y as i64);
        y += r.height() + 10;
    }
    image::imageops::resize(&sheet, w * 2, h * 2, image::imageops::FilterType::Nearest).save(format!("{dir}/all.png")).unwrap();
    // Giants beside an oak, at the same scale.
    let scale: Vec<_> = [TreePreset::Oak, TreePreset::GreatOak, TreePreset::GiantSequoia, TreePreset::Spruce].iter().map(|p| grow(&p.params(), p.name())).collect();
    row(&scale, 2).save(format!("{dir}/scale.png")).unwrap();
}
