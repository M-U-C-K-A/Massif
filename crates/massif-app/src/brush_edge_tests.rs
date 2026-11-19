//! The brush's previews at the edge of the map: the fills, the hatching
//! and the colours of a noise stop at the map's edge, and only the brush's
//! outline goes on, faintly, over the void around the map; the selection's
//! ants keep to the map. The whole app, the pointer over the map near its
//! bottom edge, at 1× and 2×; with MASSIF_SNAPSHOT_DIR, drawn as a grid.

use super::tests::{drag_on, harness_with};
use super::*;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use massif_core::TILE_SIZE;

/// Blocks a side of the brush, and how far over the edge the pointer is.
const RADIUS: f32 = 100.0;
const ABOVE: f64 = 60.0;
const ZOOM: f32 = 1.5;

fn map_view<'a>(h: &'a Harness<'_, MassifApp>) -> &'a crate::map_view::MapView {
    h.state().maps.values().next().expect("a map")
}

/// The window drawn, cut to `r` (in points).
fn shot(image: &image::RgbaImage, ppp: f32, r: Rect) -> image::RgbaImage {
    let (w, hh) = (image.width() as f32, image.height() as f32);
    let (x0, y0) = ((r.min.x * ppp).clamp(0.0, w - 1.0), (r.min.y * ppp).clamp(0.0, hh - 1.0));
    let (x1, y1) = ((r.max.x * ppp).clamp(x0 + 1.0, w), (r.max.y * ppp).clamp(y0 + 1.0, hh));
    image::imageops::crop_imm(image, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image()
}

/// Frames in rows of `columns`, each `width` pixels wide.
fn save_grid(frames: &[image::RgbaImage], columns: usize, width: u32, path: &std::path::Path) {
    let scaled: Vec<image::RgbaImage> = frames.iter().map(|f| image::imageops::resize(f, width, (f.height() as f32 * width as f32 / f.width() as f32).round().max(1.0) as u32, image::imageops::FilterType::Triangle)).collect();
    let gap = 12;
    let row_h = scaled.iter().map(|f| f.height()).max().unwrap_or(1);
    let rows = scaled.len().div_ceil(columns) as u32;
    let mut grid = image::RgbaImage::from_pixel(columns as u32 * (width + gap) + gap, rows * (row_h + gap) + gap, image::Rgba([24, 24, 27, 255]));
    for (k, f) in scaled.iter().enumerate() {
        let (c, r) = ((k % columns) as u32, (k / columns) as u32);
        image::imageops::overlay(&mut grid, f, (gap + c * (width + gap)) as i64, (gap + r * (row_h + gap)) as i64);
    }
    grid.save(path).unwrap();
}

/// Pixels drawn over the void under the map's bottom edge (at `edge`, in
/// points) within `reach` points of `p`, but for those `band` points or
/// less from the circle of radius `ring` round `p` (its outline): how many
/// differ from the void itself.
fn over_the_void(image: &image::RgbaImage, ppp: f32, p: Pos2, reach: f32, edge: f32, ring: Option<(f32, f32)>) -> usize {
    let px = |q: Pos2| image.get_pixel(((q.x * ppp) as u32).min(image.width() - 1), ((q.y * ppp) as u32).min(image.height() - 1)).0;
    let void = px(pos2(p.x + reach + 40.0, edge + 40.0));
    let mut n = 0;
    let mut y = edge + 3.0;
    while y < p.y + reach + 4.0 {
        let mut x = p.x - reach - 4.0;
        while x < p.x + reach + 4.0 {
            let q = pos2(x, y);
            let d = q.distance(p);
            if ring.is_none_or(|(r, band)| (d - r).abs() > band) {
                let c = px(q);
                if (0..3).any(|k| (c[k] as i32 - void[k] as i32).abs() > 10) {
                    n += 1;
                }
            }
            x += 1.0;
        }
        y += 1.0;
    }
    n
}

/// Every brush tool hovered over the map near its bottom edge, the brush
/// reaching 40 blocks past it: its fill, its hatching or its noise drawn
/// over the land only, its outline faint over the void; a selection
/// dragged past the edge, its ants on the map only. Before the fix, the
/// hatching of Auto biome covered the void.
#[test]
fn brush_previews_stop_at_the_map_edge() {
    let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
    let mut bad = Vec::new();
    for ppp in [1.0, 2.0] {
        let mut h = harness_with(ThemeChoice::Dark, ppp);
        h.run_steps(5);
        let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
        let t = TILE_SIZE as f64;
        let bottom = (hi.z + 1) as f64 * t;
        let middle = ((lo.x + hi.x + 1) as f64 * t) / 2.0;
        {
            let m = h.state_mut().maps.values_mut().next().unwrap();
            m.zoom = ZOOM;
            m.center = [middle, bottom - ABOVE];
        }
        h.run_steps(3);
        let canvas = map_view(&h).canvas();
        let p = canvas.center();
        let edge = map_view(&h).screen_of(middle, bottom).y;
        let r = RADIUS * ZOOM;
        let around = Rect::from_center_size(p, vec2(2.0 * r + 60.0, 2.0 * r + 60.0)).intersect(canvas);
        let mut frames = Vec::new();
        let tools: [(&str, Tool, bool); 9] = [
            ("Auto biome", Tool::AutoBiome, false),
            ("Auto paint", Tool::AutoPaint, false),
            ("Paint terrain", Tool::PaintTerrain, false),
            ("Paint terrain, noise", Tool::PaintTerrain, true),
            ("Raise / Lower", Tool::Raise, false),
            ("Smooth", Tool::Smooth, false),
            ("Water brush", Tool::Water, false),
            ("Gradient", Tool::Gradient, false),
            ("Cover", Tool::Cover, false),
        ];
        for (name, tool, noise) in tools {
            {
                let tools = &mut h.state_mut().tools;
                tools.select(tool);
                tools.brush.radius = RADIUS;
                tools.brush.strength = 1.0;
                tools.brush.shape = massif_core::BrushShape::Circle;
                tools.brush.mask = None;
                tools.pattern.on = noise;
            }
            h.event(egui::Event::PointerMoved(p + vec2(0.0, 1.0)));
            h.run_steps(2);
            h.event(egui::Event::PointerMoved(p));
            h.run_steps(4);
            let image = h.render().unwrap();
            let n = over_the_void(&image, ppp, p, r, edge, Some((r, 5.0)));
            eprintln!("{ppp}x {name}: {n} points drawn over the void");
            if n > 30 {
                bad.push(format!("{ppp}x {name}: {n}"));
            }
            frames.push(shot(&image, ppp, around));
        }
        // A selection dragged from the land past the edge: its ants keep to
        // the map.
        h.state_mut().tools.select(Tool::SelectRect);
        h.run_steps(2);
        drag_on(&mut h, p - vec2(r, r * 0.6), p + vec2(r * 0.8, r));
        h.event(egui::Event::PointerMoved(canvas.left_top() + vec2(30.0, 30.0)));
        h.run_steps(4);
        let image = h.render().unwrap();
        let n = over_the_void(&image, ppp, p, r, edge, None);
        eprintln!("{ppp}x selection: {n} points drawn over the void");
        if n > 30 {
            bad.push(format!("{ppp}x selection: {n}"));
        }
        frames.push(shot(&image, ppp, around));
        if let Some(dir) = &dir {
            save_grid(&frames, 5, 480, &dir.join(format!("brush-edge-{ppp}x.png")));
        }
    }
    assert!(bad.is_empty(), "drawn over the void: {bad:#?}");
}
