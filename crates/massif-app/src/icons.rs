//! Icons of the tools, in the Line style (the default; Settings ▸
//! Appearance ▸ Tool icons offers the Classic ones, `icons_classic`). Where
//! Lucide, the interface's icon font, has the
//! picture a tool needs (a brush, a pipette, a pickaxe), the tool shows that
//! glyph. The tools Lucide has nothing for (raising the land, smoothing it,
//! a road, a floating level) are drawn here the way Lucide draws: SVG paths
//! on a 24 × 24 grid, stroked 2 units wide with round caps and joins, in the
//! same spare style, so the two kinds sit side by side as one family.
//!
//! The paths are rasterised here, a distance to the stroke for each pixel,
//! into a mask the size of the icon in physical pixels (cached per size), so
//! they are as crisp at 1× as at 2× and their joins are round, as the font's
//! are.

use crate::theme::Icon;
use crate::tools::Tool;
use egui::{Color32, Painter, Rect, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// How a tool's icon is drawn.
enum Art {
    /// A glyph of the icon font.
    Glyph(Icon),
    /// Paths of our own on Lucide's grid.
    Paths(&'static [&'static str]),
}

// Paths on Lucide's 24 × 24 grid, its strokes 2 wide: the drawing keeps 2
// units clear of the edges, as Lucide's icons do.

/// Ground with a mound on it, an arrow up beside it: the land raised (the
/// right button lowers it).
const RAISE: &[&str] = &["M2 20h20", "M3.5 20a6.5 6.5 0 0 1 13 0", "M19 15V4", "m15.5 7.5 3.5-3.5 3.5 3.5"];
/// Rough ground over the same ground smoothed.
const SMOOTH: &[&str] = &["m3 9 3-4 3 4 3-4 3 4 3-4 3 4", "M3 19c3 0 5-6 9-6s6 6 9 6"];
/// A hill cut level, pressed down from above.
const FLATTEN: &[&str] = &["M2 20h20", "M4.5 20 8 13h8l3.5 7", "M12 3v6", "m9 6 3 3 3-3"];
/// A height measured up from the ground to a level.
const SET_HEIGHT: &[&str] = &["M2 20h20", "M5 4h14", "M12 7.5v9", "m9 10 3-3 3 3", "m9 14 3 3 3-3"];
/// Ground broken into crags.
const ROUGHEN: &[&str] = &["M2 20h20", "M2 20 5 14l2.5 3 3.5-8 3 5 2-3 3.5 6 2.5-2 1 5"];
/// A road running away between its edges, dashes down its middle.
const ROAD: &[&str] = &["M4 21 9.5 3", "M20 21 14.5 3", "M12 21v-3.5", "M12 13.5v-3", "M12 6.5V4.5"];
/// A basin, water up to its brim.
const FLOOD: &[&str] = &["M3 4v11a5 5 0 0 0 5 5h8a5 5 0 0 0 5-5V4", "M3 10c1.5 0 2.25 1.25 4.5 1.25S10.5 10 12 10s2.25 1.25 4.5 1.25S19.5 10 21 10"];
/// A floating island over the ground: a hill on top, rock tapering under
/// it.
const SLAB: &[&str] = &["M6 7c1-2 3-3 6-3s5 1 6 3", "M3 7h18l-3.5 4H15l-3 5-3-5H6.5Z", "M3 21h18"];
/// A tree, the sparkles of the automatic tools.
const AUTO_BIOME: &[&str] = &[
    "@0.82,-0.5,3.9 M8 19a4 4 0 0 1-2.24-7.32A3.5 3.5 0 0 1 9 6.03V6a3 3 0 1 1 6 0v.04a3.5 3.5 0 0 1 3.24 5.65A4 4 0 0 1 16 19Z",
    "@0.82,-0.5,3.9 M12 19v3",
    "M20 2v4",
    "M22 4h-4",
];

fn art(tool: Tool) -> Art {
    match tool {
        Tool::Raise => Art::Paths(RAISE),
        Tool::Smooth | Tool::Smooth3d => Art::Paths(SMOOTH),
        Tool::Flatten => Art::Paths(FLATTEN),
        Tool::SetHeight => Art::Paths(SET_HEIGHT),
        Tool::Roughen => Art::Paths(ROUGHEN),
        Tool::Road => Art::Paths(ROAD),
        Tool::River => Art::Glyph(Icon::Waves),
        Tool::Flood => Art::Paths(FLOOD),
        Tool::StampSlab => Art::Paths(SLAB),
        Tool::AutoBiome => Art::Paths(AUTO_BIOME),
        Tool::PaintTerrain => Art::Glyph(Icon::Paintbrush),
        Tool::Gradient => Art::Glyph(Icon::Blend),
        Tool::PaintBiome => Art::Glyph(Icon::TreeDeciduous),
        Tool::AutoPaint => Art::Glyph(Icon::WandSparkles),
        Tool::Cover => Art::Glyph(Icon::Sprout),
        Tool::Objects => Art::Glyph(Icon::TreePine),
        Tool::Annotate => Art::Glyph(Icon::PenLine),
        Tool::Water => Art::Glyph(Icon::Droplet),
        Tool::Drain => Art::Glyph(Icon::DropletOff),
        Tool::EraseSlab => Art::Glyph(Icon::Eraser),
        Tool::Build => Art::Glyph(Icon::Pickaxe),
        Tool::Dig => Art::Glyph(Icon::Shovel),
        Tool::Eyedropper => Art::Glyph(Icon::Pipette),
        Tool::Pan => Art::Glyph(Icon::Hand),
        Tool::SelectRect => Art::Glyph(Icon::SquareDashed),
        Tool::SelectEllipse => Art::Glyph(Icon::CircleDashed),
        Tool::SelectLasso => Art::Glyph(Icon::Lasso),
        Tool::SelectBrush => Art::Glyph(Icon::Highlighter),
    }
}

/// Which icons the tools show (Settings ▸ Appearance ▸ Tool icons).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum IconStyle {
    /// Lucide's glyphs and drawings in their style.
    #[default]
    Line,
    /// Massif's first drawings (see [`crate::icons_classic`]).
    Classic,
}

impl IconStyle {
    pub const ALL: [IconStyle; 2] = [IconStyle::Line, IconStyle::Classic];

    pub fn label(self) -> &'static str {
        match self {
            IconStyle::Line => tn!("Line"),
            IconStyle::Classic => tn!("Classic"),
        }
    }
}

fn style_key() -> egui::Id {
    egui::Id::unique("tool icon style")
}

/// Show the tools with the icons of `style` from now on.
pub fn set_style(ctx: &egui::Context, style: IconStyle) {
    ctx.data_mut(|d| d.insert_temp(style_key(), style));
}

fn style(ctx: &egui::Context) -> IconStyle {
    ctx.data(|d| d.get_temp(style_key())).unwrap_or_default()
}

/// Draw the icon of `tool` in `rect` (square), in `color`, in the style of
/// the settings.
pub fn paint(painter: &Painter, rect: Rect, tool: Tool, color: Color32) {
    match style(painter.ctx()) {
        IconStyle::Line => paint_line(painter, rect, tool, color),
        IconStyle::Classic => crate::icons_classic::paint(painter, rect, tool, color),
    }
}

/// Draw the Line icon of `tool` in `rect` (square), in `color`.
pub fn paint_line(painter: &Painter, rect: Rect, tool: Tool, color: Color32) {
    let side = rect.width().min(rect.height());
    match art(tool) {
        Art::Glyph(icon) => glyph(painter, rect.center(), side, icon, color),
        Art::Paths(paths) => {
            // On whole pixels, so the strokes fall the same way every time.
            let ppp = painter.pixels_per_point();
            let px = (side * ppp).round().max(8.0) as u32;
            let min = ((rect.center().to_vec2() * ppp - vec2(px as f32, px as f32) / 2.0).round() / ppp).to_pos2();
            let at = Rect::from_min_size(min, vec2(px as f32, px as f32) / ppp);
            let texture = mask(painter.ctx(), tool, paths, px);
            painter.image(texture, at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
        }
    }
}

/// An icon of the font at the size of the drawn ones: the font's glyphs
/// carry the 2-unit margin of Lucide's grid, so a glyph `side` points high
/// covers what a drawn icon `side` points wide does.
pub fn glyph(painter: &Painter, center: egui::Pos2, side: f32, icon: Icon, color: Color32) {
    painter.text(center, egui::Align2::CENTER_CENTER, crate::theme::glyph(icon), egui::FontId::proportional(side / GLYPH_SCALE), color);
}

/// The size the font's tweak gives its glyphs, against the font size (see
/// `theme::install`).
const GLYPH_SCALE: f32 = 1.08;

/// An icon of a tool `size` points wide, as a widget that does nothing.
pub fn show(ui: &mut egui::Ui, tool: Tool, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect, tool, color);
    response
}

/// The masks drawn so far, by tool and size in pixels.
#[derive(Clone, Default)]
struct Masks(Arc<std::sync::Mutex<HashMap<(Tool, u32), egui::TextureHandle>>>);

fn mask(ctx: &egui::Context, tool: Tool, paths: &[&str], px: u32) -> egui::TextureId {
    let masks = ctx.data_mut(|d| d.get_temp_mut_or_default::<Masks>(egui::Id::unique("tool icon masks")).clone());
    let mut masks = masks.0.lock().unwrap();
    masks
        .entry((tool, px))
        .or_insert_with(|| {
            let image = egui::ColorImage::from_rgba_premultiplied([px as usize, px as usize], &rasterise(paths, px));
            ctx.load_texture(format!("icon {tool:?} {px}"), image, egui::TextureOptions::LINEAR)
        })
        .id()
}

/// White strokes of `paths`, 2 units wide on the 24-unit grid, on a
/// transparent square of `px` pixels: premultiplied RGBA, to tint.
fn rasterise(paths: &[&str], px: u32) -> Vec<u8> {
    let segments: Vec<[(f32, f32); 2]> = paths.iter().flat_map(|p| polylines(p)).flat_map(|line| line.windows(2).map(|w| [w[0], w[1]]).collect::<Vec<_>>()).collect();
    let scale = px as f32 / 24.0;
    // Half the stroke, in pixels, never under 0.6 of one so the lines
    // stay whole at the smallest sizes.
    let half = (1.0 * scale).max(0.6);
    let mut out = vec![0u8; (px * px * 4) as usize];
    for j in 0..px {
        for i in 0..px {
            let p = ((i as f32 + 0.5) / scale, (j as f32 + 0.5) / scale);
            let d = segments.iter().map(|&[a, b]| distance(p, a, b)).fold(f32::INFINITY, f32::min) * scale;
            let alpha = (half + 0.5 - d).clamp(0.0, 1.0);
            let v = (alpha * 255.0).round() as u8;
            let k = ((j * px + i) * 4) as usize;
            out[k..k + 4].copy_from_slice(&[v, v, v, v]);
        }
    }
    out
}

fn distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (x, y) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (x * x + y * y).sqrt()
}

/// The polylines of an SVG path (M, L, H, V, C, S, Q, A and Z, absolute or
/// relative), its curves cut into short lines.
///
/// A path may start with "@s,dx,dy ": its points scaled by `s` and moved
/// by (`dx`, `dy`), to draw one of Lucide's icons smaller beside a mark.
fn polylines(d: &str) -> Vec<Vec<(f32, f32)>> {
    if let Some(rest) = d.strip_prefix('@') {
        let (head, path) = rest.split_once(' ').expect("a path after the transform");
        let v: Vec<f32> = head.split(',').map(|x| x.parse().unwrap()).collect();
        return polylines(path).into_iter().map(|line| line.into_iter().map(|(x, y)| (x * v[0] + v[1], y * v[0] + v[2])).collect()).collect();
    }
    let tokens = tokens(d);
    let mut lines: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut k = 0;
    let mut cmd = 'M';
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    // The last control point of a cubic, for S.
    let mut last_ctrl: Option<(f32, f32)> = None;
    let num = |k: &mut usize| -> f32 {
        let Token::Num(v) = tokens[*k] else { panic!("a number in the icon path {d:?}") };
        *k += 1;
        v
    };
    while k < tokens.len() {
        if let Token::Cmd(c) = tokens[k] {
            cmd = c;
            k += 1;
            if c == 'Z' || c == 'z' {
                cur = start;
                if let Some(line) = lines.last_mut() {
                    line.push(start);
                }
                last_ctrl = None;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let off = |p: (f32, f32), cur: (f32, f32)| if rel { (p.0 + cur.0, p.1 + cur.1) } else { p };
        let mut ctrl = None;
        match cmd.to_ascii_uppercase() {
            'M' => {
                let p = off((num(&mut k), num(&mut k)), cur);
                lines.push(vec![p]);
                cur = p;
                start = p;
                // Further pairs are lines.
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                cur = off((num(&mut k), num(&mut k)), cur);
                lines.last_mut().unwrap().push(cur);
            }
            'H' => {
                let x = num(&mut k);
                cur = (if rel { cur.0 + x } else { x }, cur.1);
                lines.last_mut().unwrap().push(cur);
            }
            'V' => {
                let y = num(&mut k);
                cur = (cur.0, if rel { cur.1 + y } else { y });
                lines.last_mut().unwrap().push(cur);
            }
            'C' | 'S' => {
                let c1 = if cmd.eq_ignore_ascii_case(&'C') {
                    off((num(&mut k), num(&mut k)), cur)
                } else {
                    last_ctrl.map_or(cur, |c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1))
                };
                let c2 = off((num(&mut k), num(&mut k)), cur);
                let p = off((num(&mut k), num(&mut k)), cur);
                let line = lines.last_mut().unwrap();
                for s in 1..=16 {
                    let t = s as f32 / 16.0;
                    let u = 1.0 - t;
                    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                    line.push((w[0] * cur.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * p.0, w[0] * cur.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * p.1));
                }
                ctrl = Some(c2);
                cur = p;
            }
            'Q' => {
                let c = off((num(&mut k), num(&mut k)), cur);
                let p = off((num(&mut k), num(&mut k)), cur);
                let line = lines.last_mut().unwrap();
                for s in 1..=12 {
                    let t = s as f32 / 12.0;
                    let u = 1.0 - t;
                    line.push((u * u * cur.0 + 2.0 * u * t * c.0 + t * t * p.0, u * u * cur.1 + 2.0 * u * t * c.1 + t * t * p.1));
                }
                cur = p;
            }
            'A' => {
                let (rx, ry, rot) = (num(&mut k), num(&mut k), num(&mut k));
                let (large, sweep) = (num(&mut k) != 0.0, num(&mut k) != 0.0);
                let p = off((num(&mut k), num(&mut k)), cur);
                let line = lines.last_mut().unwrap();
                line.extend(arc(cur, p, rx, ry, rot, large, sweep));
                cur = p;
            }
            other => panic!("the command {other} in the icon path {d:?}"),
        }
        last_ctrl = ctrl;
    }
    lines
}

/// An SVG arc from `a` to `b` as points after `a` (SVG 1.1, F.6.5).
fn arc(a: (f32, f32), b: (f32, f32), rx: f32, ry: f32, rot: f32, large: bool, sweep: bool) -> Vec<(f32, f32)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        return vec![b];
    }
    let (s, c) = rot.to_radians().sin_cos();
    let (hx, hy) = ((a.0 - b.0) / 2.0, (a.1 - b.1) / 2.0);
    let (x1, y1) = (c * hx + s * hy, -s * hx + c * hy);
    let l = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if l > 1.0 {
        rx *= l.sqrt();
        ry *= l.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut f = (num / den).max(0.0).sqrt();
    if large == sweep {
        f = -f;
    }
    let (cx1, cy1) = (f * rx * y1 / ry, -f * ry * x1 / rx);
    let (cx, cy) = (c * cx1 - s * cy1 + (a.0 + b.0) / 2.0, s * cx1 + c * cy1 + (a.1 + b.1) / 2.0);
    let angle = |ux: f32, uy: f32| uy.atan2(ux);
    let t0 = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - t0;
    let tau = std::f32::consts::TAU;
    if sweep && dt < 0.0 {
        dt += tau;
    } else if !sweep && dt > 0.0 {
        dt -= tau;
    }
    let n = ((dt.abs() / tau * 48.0).ceil() as usize).max(4);
    (1..=n)
        .map(|i| {
            let t = t0 + dt * i as f32 / n as f32;
            let (x, y) = (rx * t.cos(), ry * t.sin());
            if i == n { b } else { (c * x - s * y + cx, s * x + c * y + cy) }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Token {
    Cmd(char),
    Num(f32),
}

/// The commands and numbers of a path ("m9 6 3 3-3.5.5" is m, 9, 6, 3, 3,
/// -3.5, .5).
fn tokens(d: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let ch = b[i] as char;
        if ch.is_ascii_alphabetic() {
            out.push(Token::Cmd(ch));
            i += 1;
        } else if ch == '-' || ch == '.' || ch.is_ascii_digit() {
            let start = i;
            i += 1;
            let mut dot = ch == '.';
            while i < b.len() {
                let c = b[i] as char;
                if c.is_ascii_digit() || (c == '.' && !dot) {
                    dot |= c == '.';
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token::Num(d[start..i].parse().unwrap()));
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tool, in the order of the Tools panel.
    pub(super) const ALL: [Tool; 28] = [
        Tool::Raise,
        Tool::Smooth,
        Tool::Flatten,
        Tool::SetHeight,
        Tool::Roughen,
        Tool::Road,
        Tool::PaintTerrain,
        Tool::Gradient,
        Tool::PaintBiome,
        Tool::AutoPaint,
        Tool::AutoBiome,
        Tool::Cover,
        Tool::Objects,
        Tool::River,
        Tool::Flood,
        Tool::Drain,
        Tool::Water,
        Tool::StampSlab,
        Tool::EraseSlab,
        Tool::Build,
        Tool::Dig,
        Tool::Smooth3d,
        Tool::SelectRect,
        Tool::SelectEllipse,
        Tool::SelectLasso,
        Tool::SelectBrush,
        Tool::Eyedropper,
        Tool::Pan,
    ];

    #[test]
    fn the_style_follows_the_settings() {
        let ctx = egui::Context::default();
        assert_eq!(style(&ctx), IconStyle::Line, "Line unless set");
        set_style(&ctx, IconStyle::Classic);
        assert_eq!(style(&ctx), IconStyle::Classic);
    }

    #[test]
    fn paths_parse_as_lucide_writes_them() {
        assert_eq!(tokens("m9 6 3 3-3.5.5").len(), 7);
        // A semicircle of radius 6.5 on the ground reaches 6.5 up.
        let line = &polylines("M3.5 20a6.5 6.5 0 0 1 13 0")[0];
        let top = line.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        assert!((top - 13.5).abs() < 0.05, "{top}");
        assert_eq!(*line.last().unwrap(), (16.5, 20.0));
        // Every drawn icon parses and keeps within the grid.
        for tool in ALL {
            if let Art::Paths(paths) = art(tool) {
                for p in paths {
                    for line in polylines(p) {
                        assert!(line.iter().all(|&(x, y)| (1.0..=23.0).contains(&x) && (1.0..=23.0).contains(&y)), "{tool:?}: {p}");
                    }
                }
            }
        }
    }

    /// Every tool's icon, large and at the size of the tools panel, in both
    /// themes. Renders with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn the_icons_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        for dark in [true, false] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(ALL.len() as f32 * 60.0 + 20.0, 150.0))
                .with_pixels_per_point(2.0)
                .wgpu()
                .build_ui(move |ui| {
                    crate::theme::install(ui.ctx());
                    ui.ctx().set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
                    let (bg, ink) = if dark { (Color32::from_rgb(20, 20, 21), Color32::from_rgb(237, 237, 239)) } else { (Color32::from_rgb(250, 250, 251), Color32::from_rgb(23, 23, 25)) };
                    ui.painter().rect_filled(ui.max_rect().expand(20.0), 0, bg);
                    for (k, tool) in ALL.iter().enumerate() {
                        let x = 10.0 + k as f32 * 60.0;
                        paint(ui.painter(), Rect::from_min_size(pos2(x, 10.0), egui::vec2(48.0, 48.0)), *tool, ink);
                        paint(ui.painter(), Rect::from_min_size(pos2(x + 15.0, 80.0), egui::vec2(18.0, 18.0)), *tool, ink);
                        paint(ui.painter(), Rect::from_min_size(pos2(x + 17.0, 112.0), egui::vec2(14.0, 14.0)), *tool, Color32::from_rgb(249, 115, 22));
                    }
                });
            h.run();
            h.render().unwrap().save(dir.join(if dark { "icons-dark.png" } else { "icons-light.png" })).unwrap();
        }
    }
}

#[cfg(test)]
mod sheet {
    use super::*;

    /// Classic and Line icons side by side, named, on tiles as in the Tools
    /// panel, at 1× and 2×, dark and light. Renders with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn contact_sheet() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let rows = tests::ALL.len();
        let (col_w, row_h) = (420.0, 40.0);
        for ppp in [1.0f32, 2.0] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(col_w * 2.0, rows as f32 * row_h + 40.0))
                .with_pixels_per_point(ppp)
                .wgpu()
                .build_ui(move |ui| {
                    crate::theme::install(ui.ctx());
                    for (side, dark) in [(0.0, true), (1.0, false)] {
                        let pal = if dark { crate::theme::Palette::dark() } else { crate::theme::Palette::light() };
                        let x0 = side * col_w;
                        let p = ui.painter();
                        p.rect_filled(Rect::from_min_size(pos2(x0, 0.0), egui::vec2(col_w, rows as f32 * row_h + 40.0)), 0, pal.panel);
                        let font = egui::FontId::proportional(12.0);
                        for (x, t) in [(10.0, "Tool"), (150.0, "Classic"), (200.0, "Line"), (265.0, "Classic"), (330.0, "Line")] {
                            p.text(pos2(x0 + x, 14.0), egui::Align2::LEFT_CENTER, t, font.clone(), pal.weak);
                        }
                        for (k, tool) in tests::ALL.iter().enumerate() {
                            let y = 30.0 + k as f32 * row_h;
                            p.text(pos2(x0 + 10.0, y + row_h / 2.0), egui::Align2::LEFT_CENTER, tool.label(), font.clone(), pal.text);
                            // Large, then on a 30-point tile as the panel shows them (20 points).
                            crate::icons_classic::paint(p, Rect::from_min_size(pos2(x0 + 150.0, y + 8.0), egui::vec2(24.0, 24.0)), *tool, pal.text);
                            paint_line(p, Rect::from_min_size(pos2(x0 + 200.0, y + 8.0), egui::vec2(24.0, 24.0)), *tool, pal.text);
                            for (x, old) in [(265.0, true), (325.0, false)] {
                                let tile = Rect::from_min_size(pos2(x0 + x, y + 5.0), egui::vec2(30.0, 30.0));
                                let sel = k % 5 == 0;
                                if sel {
                                    p.rect(tile, crate::theme::TILE_RADIUS, pal.accent_soft, egui::Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                                }
                                let icon = Rect::from_center_size(tile.center(), egui::vec2(20.0, 20.0));
                                let c = if sel { pal.accent } else { pal.text };
                                if old {
                                    crate::icons_classic::paint(p, icon, *tool, c);
                                } else {
                                    paint_line(p, icon, *tool, c);
                                }
                            }
                            // A chip's 16 points.
                            paint_line(p, Rect::from_min_size(pos2(x0 + 375.0, y + 12.0), egui::vec2(16.0, 16.0)), *tool, pal.weak);
                        }
                    }
                });
            h.run();
            h.render().unwrap().save(dir.join(format!("icon-sheet-{ppp}x.png"))).unwrap();
        }
    }
}
