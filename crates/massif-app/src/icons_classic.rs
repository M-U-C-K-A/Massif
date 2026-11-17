//! The Classic tool icons (Settings ▸ Appearance ▸ Tool icons), Massif's
//! first drawings of the tools: cuts through the ground for the sculpting
//! tools, a block of terrain for painting, a basin for water, a slab for
//! levels, drawn as strokes on a 24 × 24 grid.

use crate::tools::Tool;
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// A pen on the 24 × 24 grid of an icon, drawn into a rectangle.
struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    stroke: Stroke,
    color: Color32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.origin.x + x * self.scale, self.origin.y + y * self.scale)
    }

    fn points(&self, pts: &[(f32, f32)]) -> Vec<Pos2> {
        pts.iter().map(|&(x, y)| self.at(x, y)).collect()
    }

    fn line(&self, pts: &[(f32, f32)]) {
        self.painter.add(Shape::line(self.points(pts), self.stroke));
        // Round ends.
        let r = self.stroke.width / 2.0;
        for p in [pts[0], pts[pts.len() - 1]] {
            self.painter.circle_filled(self.at(p.0, p.1), r, self.color);
        }
    }

    fn faint(&self, pts: &[(f32, f32)]) {
        let stroke = Stroke::new(self.stroke.width * 0.8, self.color.gamma_multiply(0.42));
        self.painter.add(Shape::line(self.points(pts), stroke));
    }

    fn dashed(&self, pts: &[(f32, f32)]) {
        let stroke = Stroke::new(self.stroke.width * 0.85, self.color);
        self.painter.extend(Shape::dashed_line(&self.points(pts), stroke, 2.0 * self.scale, 1.8 * self.scale));
    }

    fn closed(&self, pts: &[(f32, f32)]) {
        self.painter.add(Shape::closed_line(self.points(pts), self.stroke));
    }

    /// A shape filled lightly, outlined.
    fn tinted(&self, pts: &[(f32, f32)], alpha: f32) {
        self.painter.add(Shape::convex_polygon(self.points(pts), self.color.gamma_multiply(alpha), Stroke::NONE));
        self.closed(pts);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.at(x, y), r * self.scale, self.color);
    }

    fn ring(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.at(x, y), r * self.scale, self.stroke);
    }

    /// A curve through `f(x)` from `x0` to `x1`.
    fn graph(&self, x0: f32, x1: f32, f: impl Fn(f32) -> f32) -> Vec<(f32, f32)> {
        let n = 24;
        (0..=n).map(|k| x0 + (x1 - x0) * k as f32 / n as f32).map(|x| (x, f(x))).collect()
    }

    /// An arrow from `a` to `b`, its head at `b`.
    fn arrow(&self, a: (f32, f32), b: (f32, f32)) {
        self.line(&[a, b]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / l, dy / l);
        let head = 3.2;
        let left = (b.0 - ux * head - uy * head * 0.85, b.1 - uy * head + ux * head * 0.85);
        let right = (b.0 - ux * head + uy * head * 0.85, b.1 - uy * head - ux * head * 0.85);
        self.line(&[left, b, right]);
    }

    /// A four-pointed sparkle.
    fn sparkle(&self, x: f32, y: f32, r: f32) {
        let w = r * 0.28;
        let pts = [(x, y - r), (x + w, y - w), (x + r, y), (x + w, y + w), (x, y + r), (x - w, y + w), (x - r, y), (x - w, y - w)];
        self.painter.add(Shape::convex_polygon(self.points(&pts[..]), self.color, Stroke::NONE));
    }

    /// A block seen from above and aside, its top `top` (0 none … 1 full)
    /// filled.
    fn block(&self, cx: f32, cy: f32, w: f32, h: f32, top: f32) {
        let (a, b) = (w / 2.0, w / 4.0);
        let face = [(cx, cy - b), (cx + a, cy), (cx, cy + b), (cx - a, cy)];
        if top > 0.0 {
            self.tinted(&face, top);
        } else {
            self.closed(&face);
        }
        self.line(&[(cx - a, cy), (cx - a, cy + h), (cx, cy + b + h), (cx + a, cy + h), (cx + a, cy)]);
        self.line(&[(cx, cy + b), (cx, cy + b + h)]);
    }

    /// A basin cut through the ground.
    fn basin(&self) {
        self.line(&[(2.0, 8.0), (5.5, 8.0), (8.0, 18.5), (16.0, 18.5), (18.5, 8.0), (22.0, 8.0)]);
    }

    fn tree(&self, x: f32, ground: f32, size: f32) {
        let top = ground - size;
        self.closed(&[(x, top), (x + size * 0.42, ground - size * 0.22), (x - size * 0.42, ground - size * 0.22)]);
        self.line(&[(x, ground - size * 0.22), (x, ground)]);
    }
}

/// Draw the Classic icon of `tool` in `rect` (square), in `color`.
pub fn paint(painter: &Painter, rect: Rect, tool: Tool, color: Color32) {
    let side = rect.width().min(rect.height());
    let scale = side / 24.0;
    let pen = Pen {
        painter,
        origin: rect.center() - egui::vec2(12.0 * scale, 12.0 * scale),
        scale,
        stroke: Stroke::new((2.0 * scale).max(1.25), color),
        color,
    };
    let bump = |x: f32, at: f32, width: f32, height: f32, base: f32| base - height * (-((x - at) / width).powi(2)).exp();
    match tool {
        // Ground, a hill rising from it, an arrow up.
        Tool::Raise => {
            pen.line(&pen.graph(2.0, 22.0, |x| bump(x, 9.5, 4.2, 9.5, 20.0)));
            pen.arrow((18.5, 15.0), (18.5, 4.0));
        }
        // The rough hill it was, faint; the smooth one it is.
        Tool::Smooth => {
            pen.faint(&[(2.5, 18.0), (5.0, 13.0), (7.0, 15.5), (9.5, 8.0), (12.0, 12.5), (14.0, 6.5), (16.5, 12.0), (19.0, 10.0), (21.5, 16.0)]);
            pen.line(&pen.graph(2.0, 22.0, |x| bump(x, 12.0, 5.0, 10.5, 20.0)));
        }
        // A hill cut level, the level beyond it.
        Tool::Flatten => {
            pen.line(&[(2.0, 20.0), (6.5, 10.0), (17.5, 10.0), (22.0, 20.0)]);
            pen.dashed(&[(1.5, 10.0), (5.0, 10.0)]);
            pen.dashed(&[(19.0, 10.0), (22.5, 10.0)]);
        }
        // Uneven ground, a height over it, the way to it.
        Tool::SetHeight => {
            pen.line(&[(2.0, 19.0), (6.0, 16.0), (10.0, 18.0), (14.0, 14.5), (18.0, 17.0), (22.0, 15.5)]);
            pen.line(&[(3.5, 6.0), (20.5, 6.0)]);
            pen.line(&[(3.5, 4.0), (3.5, 8.0)]);
            pen.line(&[(20.5, 4.0), (20.5, 8.0)]);
            pen.arrow((12.0, 15.0), (12.0, 8.5));
        }
        // Ground broken into crags, a few stones thrown up.
        // A road winding away between its edges, dashes down its middle.
        Tool::Road => {
            pen.line(&[(3.0, 21.5), (7.5, 13.0), (10.0, 6.5), (10.5, 2.5)]);
            pen.line(&[(21.0, 21.5), (16.5, 13.0), (14.0, 6.5), (13.5, 2.5)]);
            pen.dashed(&[(12.0, 20.5), (12.0, 13.0), (12.0, 4.5)]);
        }
        Tool::Roughen => {
            pen.line(&[(2.0, 19.0), (4.5, 14.0), (6.5, 16.5), (9.0, 9.5), (11.0, 14.0), (13.0, 7.0), (15.5, 13.5), (17.5, 11.0), (19.5, 15.5), (22.0, 13.0)]);
            pen.dot(5.0, 7.0, 1.1);
            pen.dot(19.5, 6.0, 1.1);
        }
        // A block of ground, its top painted.
        Tool::PaintTerrain => {
            pen.block(12.0, 8.0, 16.0, 9.0, 0.55);
        }
        // Ground in bands: one terrain giving way to the next.
        Tool::Gradient => {
            pen.line(&[(2.0, 20.5), (22.0, 20.5)]);
            pen.line(&[(2.0, 13.0), (22.0, 13.0)]);
            pen.dashed(&[(8.0, 13.0), (9.5, 16.5), (8.0, 20.5)]);
            pen.dashed(&[(15.5, 13.0), (14.0, 16.5), (15.5, 20.5)]);
            pen.dot(5.0, 16.8, 1.0);
            pen.dot(12.0, 15.0, 0.9);
            pen.dot(11.5, 18.5, 0.9);
            pen.dot(19.0, 16.8, 1.0);
        }
        // Trees of a climate on the ground.
        Tool::PaintBiome => {
            pen.line(&[(2.0, 20.5), (22.0, 20.5)]);
            pen.ring(8.0, 10.0, 4.6);
            pen.line(&[(8.0, 14.6), (8.0, 20.5)]);
            pen.tree(16.5, 20.5, 12.0);
        }
        // A painted block, a sparkle: paint as the land calls for.
        Tool::AutoPaint => {
            pen.block(10.0, 10.0, 14.0, 8.0, 0.55);
            pen.sparkle(19.5, 5.0, 3.6);
        }
        // A tree, a sparkle: biomes as the land calls for.
        Tool::AutoBiome => {
            pen.line(&[(2.0, 20.5), (17.0, 20.5)]);
            pen.tree(9.0, 20.5, 14.0);
            pen.sparkle(18.5, 6.0, 3.6);
        }
        // Grass grown over the ground, a flower.
        Tool::Cover => {
            pen.line(&[(2.0, 20.5), (22.0, 20.5)]);
            for (x, h) in [(5.5, 7.0), (11.0, 9.0), (17.0, 6.5)] {
                pen.line(&[(x - 2.2, 20.5 - h * 0.75), (x, 20.5)]);
                pen.line(&[(x, 20.5 - h), (x, 20.5)]);
                pen.line(&[(x + 2.2, 20.5 - h * 0.7), (x, 20.5)]);
            }
            pen.ring(19.5, 7.0, 1.8);
        }
        // Layers stacked, painted at once.
        Tool::Combined => {
            pen.line(&[(3.0, 9.0), (12.0, 4.5), (21.0, 9.0), (12.0, 13.5), (3.0, 9.0)]);
            pen.line(&[(3.0, 13.0), (12.0, 17.5), (21.0, 13.0)]);
            pen.line(&[(3.0, 17.0), (12.0, 21.5), (21.0, 17.0)]);
        }
        // The ground, and blobs in the rock under it.
        Tool::Pockets => {
            pen.line(&[(2.0, 6.5), (22.0, 6.5)]);
            pen.ring(8.0, 13.0, 2.6);
            pen.ring(16.0, 17.0, 3.2);
            pen.ring(17.5, 10.5, 1.6);
        }
        // Three trees, small to large, on the ground.
        Tool::TreeSize => {
            pen.line(&[(2.0, 20.5), (22.0, 20.5)]);
            pen.tree(5.0, 20.5, 6.0);
            pen.tree(11.0, 20.5, 10.0);
            pen.tree(18.0, 20.5, 15.0);
        }
        // A pen and the mark it draws over the map.
        Tool::Annotate => {
            pen.line(&[(2.0, 21.0), (5.0, 17.5), (8.0, 19.5), (11.5, 16.0)]);
            pen.closed(&[(11.5, 16.0), (19.0, 3.5), (22.0, 5.5), (14.5, 18.0)]);
            pen.line(&[(17.5, 6.0), (20.5, 8.0)]);
        }
        // A tree and a rock standing on the ground: objects placed.
        Tool::Objects => {
            pen.line(&[(2.0, 20.5), (22.0, 20.5)]);
            pen.tree(8.5, 20.5, 16.0);
            pen.closed(&[(14.0, 20.5), (15.0, 15.5), (18.5, 13.5), (21.5, 16.0), (21.5, 20.5)]);
        }
        // A stream winding between its banks.
        Tool::River => {
            let left = [(4.0, 2.5), (8.5, 7.0), (6.0, 12.5), (10.0, 18.0), (14.5, 21.5)];
            let right = [(9.5, 2.5), (13.5, 7.0), (11.5, 12.0), (15.0, 16.0), (20.5, 19.0)];
            pen.line(&left);
            pen.line(&right);
            pen.dot(10.5, 10.0, 0.9);
        }
        // A basin, water up to a level in it, more falling in.
        Tool::Flood => {
            pen.basin();
            pen.line(&[(6.5, 12.5), (9.0, 11.5), (11.5, 12.5), (14.0, 11.5), (17.5, 12.5)]);
            pen.arrow((12.0, 2.0), (12.0, 8.0));
        }
        // A drop of water over a ripple.
        Tool::Water => {
            // Its round bottom, from the upper right round to the upper
            // left, and the point above.
            let mut outline = vec![(12.0, 2.5)];
            outline.extend((0..=24).map(|k| {
                let a = std::f32::consts::PI * (-0.25 + 1.5 * k as f32 / 24.0);
                (12.0 + 5.2 * a.cos(), 12.0 + 5.2 * a.sin())
            }));
            outline.push((12.0, 2.5));
            pen.line(&outline);
            pen.line(&[(3.0, 21.0), (6.5, 19.5), (10.0, 21.0), (13.5, 19.5), (17.0, 21.0), (20.5, 19.5)]);
        }
        // A slab of rock held over the ground.
        Tool::StampSlab => {
            pen.block(12.0, 6.5, 18.0, 4.0, 0.3);
            pen.dashed(&[(2.0, 20.5), (22.0, 20.5)]);
            pen.arrow((12.0, 19.5), (12.0, 14.5));
        }
        // A slab rubbed out.
        Tool::EraseSlab => {
            pen.dashed(&[(3.0, 9.0), (12.0, 4.5), (21.0, 9.0), (12.0, 13.5), (3.0, 9.0)]);
            pen.dashed(&[(3.0, 9.0), (3.0, 12.5), (12.0, 17.0), (21.0, 12.5), (21.0, 9.0)]);
            pen.line(&[(7.0, 21.0), (17.0, 21.0)]);
        }
        // Blocks stacked, one more on them.
        Tool::Build => {
            pen.block(8.0, 13.0, 9.0, 5.0, 0.0);
            pen.block(16.0, 13.0, 9.0, 5.0, 0.0);
            pen.block(12.0, 6.0, 9.0, 5.0, 0.45);
        }
        // A block with a bite dug out of it.
        Tool::Dig => {
            pen.line(&[(4.0, 9.0), (12.0, 5.0), (20.0, 9.0), (20.0, 17.0), (12.0, 21.0), (4.0, 17.0), (4.0, 9.0)]);
            pen.tinted(&[(9.0, 9.5), (13.0, 7.5), (16.5, 9.5), (12.5, 11.5)], 0.6);
            pen.line(&[(12.0, 13.0), (12.0, 21.0)]);
            pen.line(&[(4.0, 9.0), (12.0, 13.0), (20.0, 9.0)]);
        }
        // Blocks melted into a round.
        Tool::Smooth3d => {
            pen.faint(&[(3.0, 19.0), (3.0, 14.0), (7.0, 14.0), (7.0, 9.0), (12.0, 9.0), (12.0, 5.0), (17.0, 5.0), (17.0, 11.0), (21.0, 11.0), (21.0, 19.0)]);
            pen.line(&pen.graph(2.0, 22.0, |x| bump(x, 13.0, 6.0, 13.5, 19.5)));
            pen.line(&[(2.0, 19.5), (22.0, 19.5)]);
        }
        Tool::Eyedropper | Tool::Pan | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, crate::theme::glyph(tool.icon()), egui::FontId::proportional(side), color);
        }
    }
}


