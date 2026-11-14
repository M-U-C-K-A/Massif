//! The 2D map editor: navigation, brush strokes and overlays.

use crate::doc::Document;
use crate::map_render::{CHUNK_PX, ChunkKey, MAX_LOD, MapCache, MapMode, MapStyle};
use crate::theme::{self, Icon, Palette};
use crate::tools::{self, Stroke, Tool, ToolState};
use egui::{
    Align2, Color32, CursorIcon, FontId, PointerButton, Pos2, Rect, Sense, Stroke as Line,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use massif_core::{Brush, BrushShape, TILE_SIZE, TerrainId};
use std::time::Duration;

/// Samples across the picture of a brush's strength.
const STRENGTH_PX: usize = 128;
/// Strengths at which the brush preview draws a line.
const STRENGTH_LEVELS: [f32; 3] = [0.25, 0.5, 0.75];

/// How strongly a brush presses over its footprint: a picture, white where
/// it presses hardest, and the lines where its strength is a quarter, a
/// half and three quarters. Made again when the brush changes shape.
struct BrushPreview {
    /// Shape, profile, rotation and image the picture was made for.
    key: (BrushShape, massif_core::Profile, u32, usize),
    texture: egui::TextureHandle,
    /// Closed lines, in fractions of the footprint's square.
    levels: [Vec<Vec<(f32, f32)>>; 3],
}

impl BrushPreview {
    fn key(brush: &Brush) -> (BrushShape, massif_core::Profile, u32, usize) {
        let image = brush.mask.as_ref().map_or(0, |m| std::sync::Arc::as_ptr(m) as usize);
        (brush.shape, brush.profile, brush.rotation.to_bits(), image)
    }

    fn new(ctx: &egui::Context, brush: &Brush) -> Self {
        let n = STRENGTH_PX;
        // The picture does not depend on the size: any radius will do.
        let mut b = brush.clone();
        b.radius = n as f32 / 2.0;
        b.filter = None;
        let reach = b.reach();
        let weights: Vec<f32> = (0..n * n)
            .map(|c| {
                let at = |k: usize| ((k as f32 + 0.5) / n as f32 - 0.5) * 2.0 * reach;
                b.weight(at(c % n), at(c / n)).clamp(0.0, 1.0)
            })
            .collect();
        let bytes: Vec<u8> = weights.iter().flat_map(|&w| [(w * 125.0) as u8; 4]).collect();
        let image = egui::ColorImage::from_rgba_premultiplied([n, n], &bytes);
        let texture = ctx.load_texture("brush strength", image, egui::TextureOptions::LINEAR);
        let levels = STRENGTH_LEVELS.map(|level| {
            massif_core::brush::trace_cells(n, n, |i, j| weights[j * n + i] >= level)
                .into_iter()
                .map(|ring| {
                    // The corners of the cells, eased into a line.
                    let k = ring.len();
                    (0..k)
                        .map(|a| {
                            let (p, q) = (ring[a], ring[(a + 1) % k]);
                            ((p.0 + q.0) as f32 / (2 * n) as f32, (p.1 + q.1) as f32 / (2 * n) as f32)
                        })
                        .collect()
                })
                .collect()
        });
        Self {
            key: Self::key(brush),
            texture,
            levels,
        }
    }
}

const MIN_ZOOM: f32 = 1.0 / 2048.0;
const MAX_ZOOM: f32 = 48.0;
const FOOTER: f32 = 28.0;
/// Points of scrolling per brush size step (about one wheel notch).
const WHEEL_STEP: f32 = 20.0;
/// Degrees the brush turns per wheel notch (one with Shift).
const TURN_STEP: f32 = 5.0;
/// Points of scrolling per line, for wheels that count in lines.
const LINE_POINTS: f32 = 40.0;
/// Screen pixels per block from which the brush is outlined block by block.
const EXACT_ZOOM: f32 = 3.0;
/// Largest brush (half its footprint, in blocks) outlined block by block.
const EXACT_REACH: f32 = 320.0;

/// Tabs for the display modes, or a drop-down when space is short.
pub fn mode_selector(ui: &mut Ui, mode: &mut MapMode, narrow: bool, id: &str) {
    if narrow {
        crate::widgets::combo(id)
            .width(84.0)
            .selected_text(mode.label())
            .show_ui(ui, |ui| {
                for m in MapMode::ALL {
                    ui.selectable_value(mode, m, m.label());
                }
            });
    } else {
        for m in MapMode::ALL {
            if ui.selectable_label(*mode == m, m.label()).clicked() {
                *mode = m;
            }
        }
    }
}

/// Per-area state of a map editor.
/// How near a handle of a river the pointer takes hold of it, in points.
const HANDLE_REACH: f32 = 11.0;

/// What the pointer is over, on a river being laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RiverHit {
    /// One of its anchors: the source, a point it passes, its end.
    Anchor(usize),
    /// The end of the way it runs by itself.
    Mouth,
    /// Its course, at this point of it.
    Course(usize),
}

/// What the automatic tools would change under the brush: the new terrain
/// (or biome) of each column, drawn over the map before painting.
struct AutoPreview {
    key: (Tool, i32, i32, u64, u64),
    /// The footprint, in blocks: x0, z0, x1, z1.
    rect: [i32; 4],
    texture: Option<egui::TextureHandle>,
}

pub struct MapView {
    /// Block coordinates at the centre of the view.
    pub center: [f64; 2],
    /// Screen points per block.
    pub zoom: f32,
    pub mode: MapMode,
    pub contours: bool,
    pub grid: bool,
    fitted: Option<u64>,
    stroke: Option<Stroke>,
    last_time: f64,
    probe: Option<((i32, i32, u64), Option<tools::Probe>)>,
    /// Scrolling not yet turned into brush size steps.
    wheel: f32,
    /// Scrolling not yet turned into brush rotation steps.
    turn: f32,
    /// Where the canvas was last drawn, for keyboard zooming.
    canvas: Rect,
    /// The picture of the brush's strength, under the pointer.
    strength: Option<BrushPreview>,
    /// The anchor of the river being laid out that is being dragged.
    river_grab: Option<usize>,
    /// Where water let go under the pointer would go (River tool, before a
    /// source is placed): for the block and the document's revision it was
    /// found for.
    spring: Option<((i32, i32, u64), massif_core::river::Descent)>,
    /// The block the pointer rests on, and since when.
    spring_rest: Option<((i32, i32, u64), f64)>,
    auto_preview: Option<AutoPreview>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 1.0,
            mode: MapMode::Terrain,
            contours: true,
            grid: false,
            fitted: None,
            stroke: None,
            last_time: 0.0,
            probe: None,
            wheel: 0.0,
            turn: 0.0,
            strength: None,
            canvas: Rect::NOTHING,
            river_grab: None,
            spring: None,
            spring_rest: None,
            auto_preview: None,
        }
    }
}

/// What a map editor needs from the application for one frame.
pub struct MapInput<'a> {
    pub doc: &'a mut Document,
    pub tools: &'a mut ToolState,
    pub cache: &'a mut MapCache,
    pub solo: bool,
    pub hover: &'a mut Option<(i32, i32)>,
    pub feedback: &'a mut Option<String>,
    /// Space is held: the primary button pans.
    pub pan_key: bool,
    /// Direction of the light of the shading (see [`MapStyle::light`]).
    pub light: u8,
}

impl MapView {
    /// Display-mode tabs and toggles, shown in the area header.
    pub fn header(&mut self, ui: &mut Ui) {
        let narrow = ui.available_width() < 420.0;
        mode_selector(ui, &mut self.mode, narrow, "map mode");
        ui.separator();
        ui.toggle_value(&mut self.contours, theme::glyph(Icon::Spline).to_string())
            .on_hover_text("Contour lines");
        ui.toggle_value(&mut self.grid, theme::glyph(Icon::Grid3x3).to_string())
            .on_hover_text("Tile and chunk grid");
    }

    /// What the pointer at `p` is over, on the river being laid out.
    fn river_hit(&self, canvas: Rect, draft: &tools::RiverDraft, settings: &tools::RiverSettings, p: Pos2) -> Option<RiverHit> {
        let screen = |q: (f32, f32)| self.to_screen(canvas, q.0 as f64, q.1 as f64);
        let anchors = draft.layout.anchors();
        for (k, &a) in anchors.iter().enumerate().rev() {
            if screen(a).distance(p) <= HANDLE_REACH {
                return Some(RiverHit::Anchor(k));
            }
        }
        if draft.layout.end.is_none()
            && let Some(&m) = draft.plan.course.last()
            && screen(m).distance(p) <= HANDLE_REACH
        {
            return Some(RiverHit::Mouth);
        }
        let reach = (settings.width * (1.0 + 2.0 * settings.flow) * self.zoom / 2.0).max(7.0);
        let mut best: Option<(usize, f32)> = None;
        for (i, &c) in draft.plan.course.iter().enumerate() {
            let d = screen(c).distance(p);
            if d <= reach && best.is_none_or(|b| d < b.1) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| RiverHit::Course(i))
    }

    /// Draw a river's course on the map: its banks, its water, the way it
    /// flows, its lakes; and the handles of a river being laid out.
    fn draw_river(
        &self,
        painter: &egui::Painter,
        canvas: Rect,
        plan: &massif_core::river::Descent,
        spec: &massif_core::river::RiverSpec,
        handles: Option<(&tools::RiverDraft, Option<RiverHit>, Option<usize>)>,
    ) {
        let water = Color32::from_rgb(56, 146, 255);
        for (cells, _) in &plan.lakes {
            let mut rows: Vec<(i32, i32)> = cells.iter().map(|&(x, z)| (z, x)).collect();
            rows.sort_unstable();
            let mut k = 0;
            while k < rows.len() {
                let (z, x0) = rows[k];
                let mut x1 = x0;
                while k + 1 < rows.len() && rows[k + 1] == (z, x1 + 1) {
                    k += 1;
                    x1 += 1;
                }
                let a = self.to_screen(canvas, x0 as f64, z as f64);
                let b = self.to_screen(canvas, x1 as f64 + 1.0, z as f64 + 1.0);
                painter.rect_filled(Rect::from_min_max(a, b), 0, water.gamma_multiply(0.5));
                k += 1;
            }
        }
        let n = plan.course.len();
        if n >= 2 {
            // Points a pixel and a half apart at least, with how far down
            // the river they are.
            let mut pts: Vec<(Pos2, f32)> = Vec::with_capacity(n.min(4096));
            for (k, &(x, z)) in plan.course.iter().enumerate() {
                let p = self.to_screen(canvas, x as f64, z as f64);
                if k + 1 == n || pts.last().is_none_or(|q| q.0.distance(p) >= 1.5) {
                    pts.push((p, k as f32 / (n - 1) as f32));
                }
            }
            let width = |t: f32| (spec.width + (spec.width_end - spec.width) * t) * self.zoom;
            // In pieces of steady width, the width growing downstream.
            let pieces: Vec<&[(Pos2, f32)]> = pts.chunks(24).collect();
            let joined = |k: usize| -> Vec<Pos2> {
                let mut v: Vec<Pos2> = pieces[k].iter().map(|p| p.0).collect();
                if let Some(next) = pieces.get(k + 1).and_then(|p| p.first()) {
                    v.push(next.0);
                }
                v
            };
            for (layer, color) in [
                (0, water.gamma_multiply(0.14)),
                (1, Color32::from_rgba_unmultiplied(6, 24, 56, 170)),
                (2, water.gamma_multiply(0.92)),
            ] {
                for k in 0..pieces.len() {
                    let t = pieces[k][pieces[k].len() / 2].1;
                    let w = width(t);
                    let stroke = match layer {
                        0 => w + 2.0 * spec.banks * self.zoom,
                        1 => w.max(2.5) + 2.0,
                        _ => w.max(2.5),
                    };
                    let line = joined(k);
                    if line.len() >= 2 {
                        painter.add(egui::Shape::line(line, Line::new(stroke, color)));
                    }
                }
            }
            // Which way it flows: a chevron every so often.
            let mut run = 0.0;
            for w in pts.windows(2) {
                let (a, b) = (w[0].0, w[1].0);
                let d = a.distance(b);
                run += d;
                if run >= 120.0 && d > 0.5 {
                    run = 0.0;
                    let dir = (b - a) / d;
                    let side = vec2(-dir.y, dir.x);
                    let tip = b + dir * 3.0;
                    let ink = Line::new(1.6, Color32::from_white_alpha(210));
                    painter.line_segment([tip - dir * 6.0 + side * 4.5, tip], ink);
                    painter.line_segment([tip - dir * 6.0 - side * 4.5, tip], ink);
                }
            }
        }
        let Some((draft, hover, grab)) = handles else {
            if let Some(&(x, z)) = plan.course.first() {
                let p = self.to_screen(canvas, x as f64, z as f64);
                painter.circle(p, 6.0, water, Line::new(2.0, Color32::WHITE));
            }
            return;
        };
        let lit = |k: usize| hover == Some(RiverHit::Anchor(k)) || grab == Some(k);
        let anchors = draft.layout.anchors();
        let last = anchors.len() - 1;
        if draft.layout.end.is_none()
            && let Some(&(x, z)) = plan.course.last()
        {
            // Where it runs to by itself: a ring to take hold of.
            let p = self.to_screen(canvas, x as f64, z as f64);
            let r = if hover == Some(RiverHit::Mouth) { 7.5 } else { 5.5 };
            painter.circle_stroke(p, r + 1.0, Line::new(3.0, Color32::from_black_alpha(90)));
            painter.circle_stroke(p, r, Line::new(2.0, Color32::WHITE));
        }
        for (k, &(x, z)) in anchors.iter().enumerate() {
            let p = self.to_screen(canvas, x as f64, z as f64);
            let grow = if lit(k) { 2.0 } else { 0.0 };
            painter.circle_filled(p + vec2(0.0, 1.0), 8.0 + grow, Color32::from_black_alpha(70));
            if k == 0 {
                // The source.
                painter.circle(p, 7.0 + grow, water, Line::new(2.0, Color32::WHITE));
                painter.circle_filled(p, 2.2, Color32::WHITE);
            } else if k == last && draft.layout.end.is_some() {
                // The end it was given.
                painter.circle(p, 6.5 + grow, Color32::WHITE, Line::new(2.5, water));
                painter.circle_filled(p, 2.5, water);
            } else {
                painter.circle(p, 5.0 + grow, Color32::WHITE, Line::new(2.0, water));
            }
        }
    }

    /// The bar over a river being laid out: how long it is and where it
    /// goes, and the buttons to dig it or drop it.
    #[allow(clippy::too_many_arguments)]
    fn river_bar(&mut self, ui: &mut Ui, painter: &egui::Painter, canvas: Rect, id: egui::Id, doc: &mut Document, tools: &mut ToolState, feedback: &mut Option<String>) {
        let Some(draft) = &tools.river_draft else { return };
        let font = FontId::proportional(12.5);
        let text = painter.layout_no_wrap(draft.summary(), font.clone(), Color32::from_gray(225));
        let dig = painter.layout_no_wrap("Dig river".to_owned(), FontId::proportional(12.5), Color32::WHITE);
        let enter = painter.layout_no_wrap("⏎".to_owned(), FontId::proportional(11.0), Color32::from_white_alpha(170));
        let cancel = painter.layout_no_wrap("Cancel".to_owned(), font, Color32::from_gray(210));
        let esc = painter.layout_no_wrap("esc".to_owned(), FontId::proportional(10.5), Color32::from_gray(130));
        let (pad, gap, h) = (12.0, 8.0, 28.0);
        let dig_w = dig.size().x + enter.size().x + 26.0;
        let cancel_w = cancel.size().x + esc.size().x + 24.0;
        let width = pad + text.size().x + 16.0 + dig_w + gap + cancel_w + 6.0;
        let bar = Rect::from_center_size(canvas.center_bottom() - vec2(0.0, 30.0), vec2(width, h + 12.0));
        painter.rect_filled(bar.translate(vec2(0.0, 3.0)).expand(1.0), 12, Color32::from_black_alpha(70));
        painter.rect(bar, 12, Color32::from_rgba_unmultiplied(19, 21, 26, 242), Line::new(1.0, Color32::from_white_alpha(20)), StrokeKind::Inside);
        let mid = bar.center().y;
        painter.galley(pos2(bar.min.x + pad, mid - text.size().y / 2.0), text.clone(), Color32::from_gray(225));
        let dig_rect = Rect::from_min_size(pos2(bar.min.x + pad + text.size().x + 16.0, mid - h / 2.0), vec2(dig_w, h));
        let cancel_rect = Rect::from_min_size(pos2(dig_rect.max.x + gap, mid - h / 2.0), vec2(cancel_w, h));
        let dig_hit = ui.interact(dig_rect, id.with("dig river"), Sense::click()).on_hover_text("Dig it into the map (Enter)");
        let cancel_hit = ui.interact(cancel_rect, id.with("drop river"), Sense::click()).on_hover_text("Drop this river (Esc)");
        let accent = theme::accent();
        painter.rect_filled(dig_rect, 8, if dig_hit.hovered() { theme::mix(accent, Color32::WHITE, 0.15) } else { accent });
        painter.galley(pos2(dig_rect.min.x + 12.0, mid - dig.size().y / 2.0), dig, Color32::WHITE);
        painter.galley(pos2(dig_rect.max.x - 12.0 - enter.size().x, mid - enter.size().y / 2.0), enter, Color32::from_white_alpha(170));
        if cancel_hit.hovered() {
            painter.rect_filled(cancel_rect, 8, Color32::from_white_alpha(16));
        }
        painter.galley(pos2(cancel_rect.min.x + 10.0, mid - cancel.size().y / 2.0), cancel, Color32::from_gray(210));
        painter.galley(pos2(cancel_rect.max.x - 10.0 - esc.size().x, mid - esc.size().y / 2.0 + 0.5), esc, Color32::from_gray(130));
        if dig_hit.hovered() || cancel_hit.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if dig_hit.clicked() {
            *feedback = tools::dig_draft(doc, tools);
            self.river_grab = None;
        } else if cancel_hit.clicked() {
            tools.river_draft = None;
            self.river_grab = None;
            *feedback = Some("River dropped".into());
        }
    }

    /// Draw what Auto paint or Auto biome would change under the brush at
    /// `at`: the columns that would change, in their new colour.
    fn show_auto_preview(&mut self, ctx: &egui::Context, painter: &egui::Painter, canvas: Rect, doc: &Document, tools: &ToolState, at: (f64, f64)) {
        use std::hash::{Hash, Hasher};
        let biomes = tools.tool == Tool::AutoBiome;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", tools.auto).hash(&mut h);
        BrushPreview::key(&tools.brush).hash(&mut h);
        tools.brush.radius.to_bits().hash(&mut h);
        tools.brush.strength.to_bits().hash(&mut h);
        let key = (tools.tool, at.0.floor() as i32, at.1.floor() as i32, doc.revision, h.finish());
        if self.auto_preview.as_ref().is_none_or(|a| a.key != key) {
            let reader = massif_core::auto::Reader::new(&tools.auto, &doc.world.registry, doc.world.meta.sea_level);
            let brush = tools.stroke_brush();
            let (rect, cols) = massif_core::ops::auto_preview(doc.dimension(), &brush, at.0 as f32, at.1 as f32, &reader, biomes);
            let reg = &doc.world.registry;
            // What changes in its new colour; what stays, darkened.
            let colour = |p: &massif_core::ops::Preview| match p {
                massif_core::ops::Preview::Outside => Color32::TRANSPARENT,
                massif_core::ops::Preview::Same => Color32::from_black_alpha(120),
                massif_core::ops::Preview::Becomes(id) => {
                    let c = if biomes { reg.biomes.get(*id as usize).map(|b| b.color) } else { reg.terrains.get(*id as usize).map(|t| t.color) };
                    let [r, g, b] = c.unwrap_or([255, 0, 255]);
                    Color32::from_rgb(r, g, b)
                }
            };
            let texture = cols.iter().any(|p| *p != massif_core::ops::Preview::Outside).then(|| {
                let pixels = cols.iter().map(colour).collect();
                let image = egui::ColorImage::new([rect.width() as usize, rect.depth() as usize], pixels);
                ctx.load_texture("auto preview", image, egui::TextureOptions::NEAREST)
            });
            self.auto_preview = Some(AutoPreview {
                key,
                rect: [rect.x0, rect.z0, rect.x1, rect.z1],
                texture,
            });
        }
        if let Some(AutoPreview { rect, texture: Some(texture), .. }) = &self.auto_preview {
            let min = self.to_screen(canvas, rect[0] as f64, rect[1] as f64);
            let max = self.to_screen(canvas, rect[2] as f64, rect[3] as f64);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture.id(), Rect::from_min_max(min, max), uv, Color32::WHITE);
        }
    }

    /// Where block coordinates `(x, z)` were on the screen, last frame.
    #[cfg(test)]
    pub fn screen_of(&self, x: f64, z: f64) -> Pos2 {
        self.to_screen(self.canvas, x, z)
    }

    fn to_screen(&self, canvas: Rect, x: f64, z: f64) -> Pos2 {
        let c = canvas.center();
        pos2(
            c.x + ((x - self.center[0]) * self.zoom as f64) as f32,
            c.y + ((z - self.center[1]) * self.zoom as f64) as f32,
        )
    }

    fn to_world(&self, canvas: Rect, p: Pos2) -> (f64, f64) {
        let c = canvas.center();
        (
            self.center[0] + ((p.x - c.x) / self.zoom) as f64,
            self.center[1] + ((p.y - c.y) / self.zoom) as f64,
        )
    }

    fn zoom_at(&mut self, canvas: Rect, p: Pos2, factor: f32) {
        let before = self.to_world(canvas, p);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let c = canvas.center();
        self.center[0] = before.0 - ((p.x - c.x) / self.zoom) as f64;
        self.center[1] = before.1 - ((p.y - c.y) / self.zoom) as f64;
    }

    /// Zoom around the centre of the view (keyboard zooming).
    pub fn zoom_by(&mut self, factor: f32) {
        let canvas = self.canvas;
        if canvas.is_positive() {
            self.zoom_at(canvas, canvas.center(), factor);
        } else {
            self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// One block per point.
    pub fn zoom_reset(&mut self) {
        self.zoom = 1.0;
    }

    /// Frame the whole map.
    pub fn fit(&mut self, doc: &Document, canvas: Rect) {
        let Some((lo, hi)) = doc.dimension().bounds() else {
            self.center = [0.0, 0.0];
            self.zoom = 1.0;
            return;
        };
        let t = TILE_SIZE as f64;
        let (x0, z0) = (lo.x as f64 * t, lo.z as f64 * t);
        let (x1, z1) = ((hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t);
        self.center = [(x0 + x1) / 2.0, (z0 + z1) / 2.0];
        let fit = (canvas.width() / (x1 - x0) as f32).min(canvas.height() / (z1 - z0) as f32);
        self.zoom = (fit * 0.92).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn ui(&mut self, ui: &mut Ui, input: MapInput) {
        let pal = Palette::of(ui);
        let full = ui.available_rect_before_wrap();
        let canvas = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - FOOTER));
        let footer = Rect::from_min_max(pos2(full.min.x, canvas.max.y), full.max);
        let response = ui.allocate_rect(canvas, Sense::click_and_drag());
        ui.advance_cursor_after_rect(full);
        let MapInput {
            doc,
            tools,
            cache,
            solo,
            hover,
            feedback,
            pan_key,
            light,
        } = input;

        if self.fitted != Some(doc.generation) && canvas.width() > 10.0 {
            self.fit(doc, canvas);
            self.fitted = Some(doc.generation);
        }

        // ---- navigation, the WorldPainter way ----------------------------
        // The wheel sizes the brush (Shift: one block at a time); with ⌘ (the
        // Windows key of a PC keyboard) it turns the brush, with Ctrl or Alt,
        // or with a pinch, it zooms. The middle button (or Space, or the Pan
        // tool) moves the map.
        let (time, modifiers, pointer) = ui.input(|i| (i.time, i.modifiers, i.pointer.interact_pos()));
        let (pressed, down) = ui.input(|i| {
            let p = &i.pointer;
            (
                [p.button_pressed(PointerButton::Primary), p.button_pressed(PointerButton::Secondary)],
                [p.button_down(PointerButton::Primary), p.button_down(PointerButton::Secondary)],
            )
        });
        let dt = (time - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = time;
        self.canvas = canvas;
        let panning = tools.tool == Tool::Pan || pan_key;
        if response.dragged_by(PointerButton::Middle)
            || (panning && response.dragged_by(PointerButton::Primary) && self.stroke.is_none())
        {
            let d = response.drag_delta();
            self.center[0] -= (d.x / self.zoom) as f64;
            self.center[1] -= (d.y / self.zoom) as f64;
        }
        if response.hovered()
            && let Some(p) = response.hover_pos()
        {
            if turns_brush(modifiers) && tools.tool.uses_brush() {
                // egui reads ⌘ and the wheel as a zoom: take the wheel itself.
                let height = canvas.height();
                self.turn += ui.input(|i| {
                    i.events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::MouseWheel { unit, delta, .. } => {
                                let k = match unit {
                                    egui::MouseWheelUnit::Point => 1.0,
                                    egui::MouseWheelUnit::Line => LINE_POINTS,
                                    egui::MouseWheelUnit::Page => height,
                                };
                                Some(if delta.y != 0.0 { delta.y } else { delta.x } * k)
                            }
                            _ => None,
                        })
                        .sum::<f32>()
                });
                let step = if modifiers.shift && cfg!(target_os = "macos") { 1.0 } else { TURN_STEP };
                while self.turn.abs() >= WHEEL_STEP {
                    // Scrolling up turns clockwise, like a knob.
                    let up = self.turn > 0.0;
                    self.turn -= WHEEL_STEP.copysign(self.turn);
                    tools.brush.rotate(if up { step } else { -step });
                }
            } else {
                self.turn = 0.0;
                let (zoom_delta, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
                if zoom_delta != 1.0 {
                    self.zoom_at(canvas, p, zoom_delta);
                } else if scroll != Vec2::ZERO {
                    // Shift turns vertical scrolling into horizontal on some systems.
                    let amount = if scroll.y != 0.0 { scroll.y } else { scroll.x };
                    if modifiers.ctrl || modifiers.alt || modifiers.command {
                        self.zoom_at(canvas, p, (amount * 0.004).exp());
                    } else if tools.tool.uses_brush() {
                        self.wheel += amount;
                        while self.wheel.abs() >= WHEEL_STEP {
                            // Scrolling up makes the brush smaller, as in WorldPainter.
                            let up = self.wheel > 0.0;
                            self.wheel -= WHEEL_STEP.copysign(self.wheel);
                            let r = tools.brush.radius;
                            let next = match (modifiers.shift, up) {
                                (true, true) => r - 1.0,
                                (true, false) => r + 1.0,
                                (false, true) => (r / 1.1).min(r - 1.0),
                                (false, false) => (r * 1.1).max(r + 1.0),
                            };
                            tools.brush.radius = next.round().clamp(1.0, massif_core::MAX_RADIUS);
                        }
                    }
                }
            }
        }

        // ---- editing -------------------------------------------------------
        // The left button applies the tool, the right button (or Alt with the
        // left one) its inverse: lower instead of raise, erase instead of
        // paint, drain instead of flood.
        let world_pointer = pointer.map(|p| self.to_world(canvas, p));
        if let Some((x, z)) = world_pointer.filter(|_| response.hovered()) {
            *hover = Some((x.floor() as i32, z.floor() as i32));
        }
        let button = if pressed[0] && !panning {
            Some(PointerButton::Primary)
        } else if pressed[1] {
            Some(PointerButton::Secondary)
        } else {
            None
        };
        if let Some(button) = button
            && response.is_pointer_button_down_on()
            && self.stroke.is_none()
            && let Some((x, z)) = world_pointer
        {
            let secondary = button == PointerButton::Secondary;
            let inverse = secondary || modifiers.alt;
            let (bx, bz) = (x.floor() as i32, z.floor() as i32);
            if tools.tool == Tool::River {
                let at = (x as f32, z as f32);
                let hit = match (&tools.river_draft, pointer) {
                    (Some(draft), Some(p)) => self.river_hit(canvas, draft, &tools.river, p),
                    _ => None,
                };
                match (&mut tools.river_draft, hit, secondary) {
                    // The first click: where it springs (dragged, it moves).
                    (None, _, false) => {
                        *feedback = tools::start_river(doc, tools, at.0, at.1);
                        if tools.river_draft.is_some() {
                            self.river_grab = Some(0);
                        }
                    }
                    (Some(_), Some(RiverHit::Anchor(k)), false) => self.river_grab = Some(k),
                    // A point it passes, or its chosen end, taken away.
                    (Some(draft), Some(RiverHit::Anchor(k)), true) if k > 0 => {
                        if k <= draft.layout.through.len() {
                            draft.layout.through.remove(k - 1);
                        } else {
                            draft.layout.end = None;
                        }
                        draft.plan(doc, &tools.river, None);
                    }
                    // Where it runs to by itself, held: its end, to move.
                    (Some(draft), Some(RiverHit::Mouth), false) => {
                        draft.layout.end = draft.plan.course.last().copied();
                        self.river_grab = Some(draft.layout.anchors().len() - 1);
                    }
                    // The course itself, held: a point it must pass, to drag.
                    (Some(draft), Some(RiverHit::Course(i)), false) => {
                        let leg = draft.stops.iter().rposition(|&s| s <= i).unwrap_or(0);
                        let k = leg.min(draft.layout.through.len());
                        draft.layout.through.insert(k, at);
                        self.river_grab = Some(k + 1);
                    }
                    // Anywhere else: that is where it ends.
                    (Some(draft), None, false) => {
                        draft.layout.end = Some(at);
                        self.river_grab = Some(draft.layout.anchors().len() - 1);
                    }
                    _ => {}
                }
            } else if tools.tool.is_click() {
                *feedback = tools::click(doc, tools, bx, bz, inverse);
            } else if tools.tool.uses_brush() {
                let (stroke, msg) = tools::begin_stroke(doc, tools, (x as f32, z as f32), inverse, secondary);
                *feedback = msg.or(feedback.take());
                if let Some(mut stroke) = stroke {
                    tools::continue_stroke(doc, tools, &mut stroke, (x as f32, z as f32), 1.0 / 60.0);
                    self.stroke = Some(stroke);
                }
            }
        }
        // A held anchor of the river follows the pointer; let go, its
        // course is worked out finely.
        if let Some(k) = self.river_grab {
            match tools.river_draft.as_mut() {
                Some(draft) if tools.tool == Tool::River => {
                    if down[0] {
                        if let Some((x, z)) = world_pointer {
                            draft.layout.set(k, (x as f32, z as f32));
                            draft.plan(doc, &tools.river, Some(k));
                        }
                        ui.ctx().request_repaint();
                    } else {
                        self.river_grab = None;
                        draft.plan(doc, &tools.river, None);
                    }
                }
                _ => self.river_grab = None,
            }
        }
        if tools.tool == Tool::River
            && tools.river_draft.is_some()
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if enter {
                *feedback = tools::dig_draft(doc, tools);
            } else if escape {
                tools.river_draft = None;
                self.river_grab = None;
                *feedback = Some("River dropped".into());
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(stroke) = self.stroke.take()
        {
            // Escape abandons the stroke being drawn.
            let _ = tools::end_stroke(doc, stroke);
            doc.undo();
            *feedback = Some("Stroke cancelled".into());
        }
        if let Some(stroke) = &mut self.stroke {
            let held = if stroke.secondary() { down[1] } else { down[0] };
            if held {
                if let Some((x, z)) = world_pointer {
                    tools::continue_stroke(doc, tools, stroke, (x as f32, z as f32), dt);
                }
                ui.ctx().request_repaint();
            } else if let Some(stroke) = self.stroke.take()
                && let Some(note) = tools::end_stroke(doc, stroke)
            {
                *feedback = Some(note);
            }
        }

        // ---- drawing -------------------------------------------------------
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0, pal.canvas);
        // The map lies on the canvas, a soft shadow under it.
        if let Some((lo, hi)) = doc.dimension().bounds() {
            let t = TILE_SIZE as f64;
            let under = Rect::from_min_max(
                self.to_screen(canvas, lo.x as f64 * t, lo.z as f64 * t),
                self.to_screen(canvas, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t),
            );
            let shadow = egui::epaint::Shadow {
                offset: [0, 8],
                blur: 36,
                spread: 2,
                color: Color32::from_black_alpha(if pal.dark { 170 } else { 70 }),
            };
            painter.add(shadow.as_shape(under, 3));
        }

        let style = MapStyle {
            mode: self.mode,
            contours: self.contours,
            solo: solo.then_some(tools.terrain),
            highlight: (doc.active_level > 0).then_some(doc.active_level.min(7) as u8),
            light,
            lava: doc.world.meta.lava,
            top: doc.dimension().max_y,
            seed: doc.world.meta.seed,
        };
        let ppp = ui.ctx().pixels_per_point();
        let blocks_per_px = 1.0 / (self.zoom * ppp);
        let lod = if blocks_per_px <= 1.0 {
            0
        } else {
            (blocks_per_px.log2().floor() as u8).min(MAX_LOD)
        };

        let dim = doc.dimension();
        if let Some((lo, hi)) = dim.bounds() {
            let span = (CHUNK_PX as f64) * (1u64 << lod) as f64;
            let (wx0, wz0) = self.to_world(canvas, canvas.min);
            let (wx1, wz1) = self.to_world(canvas, canvas.max);
            let cx0 = ((wx0 / span).floor() as i64).max((lo.x >> lod) as i64);
            let cz0 = ((wz0 / span).floor() as i64).max((lo.z >> lod) as i64);
            let cx1 = ((wx1 / span).floor() as i64).min((hi.x >> lod) as i64);
            let cz1 = ((wz1 / span).floor() as i64).min((hi.z >> lod) as i64);
            let mut keys = Vec::new();
            for cz in cz0..=cz1 {
                for cx in cx0..=cx1 {
                    keys.push(ChunkKey {
                        lod,
                        x: cx as i32,
                        z: cz as i32,
                    });
                }
            }
            let mid = ((self.center[0] / span) - 0.5, (self.center[1] / span) - 0.5);
            keys.sort_by(|a, b| {
                let d = |k: &ChunkKey| (k.x as f64 - mid.0).powi(2) + (k.z as f64 - mid.1).powi(2);
                d(a).total_cmp(&d(b))
            });
            if keys.len() <= 4096 {
                let more = cache.update(
                    ui.ctx(),
                    dim,
                    &doc.world.registry,
                    doc.world.meta.sea_level as f32,
                    style,
                    &keys,
                    Duration::from_millis(12),
                );
                if more {
                    ui.ctx().request_repaint();
                }
            }
            for key in &keys {
                let (ox, oz) = key.origin();
                let b = key.blocks() as f64;
                let rect = Rect::from_min_max(
                    self.to_screen(canvas, ox as f64, oz as f64),
                    self.to_screen(canvas, ox as f64 + b, oz as f64 + b),
                );
                if let Some(tex) = cache.texture(style, *key) {
                    painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    continue;
                }
                // Fall back on a coarser chunk while this one is built.
                let mut parent = *key;
                for up in 1..=4u8 {
                    if parent.lod >= MAX_LOD {
                        break;
                    }
                    parent = parent.parent();
                    if let Some(tex) = cache.texture(style, parent) {
                        let n = (1i32 << up) as f32;
                        let u = (key.x - (parent.x << up)) as f32 / n;
                        let v = (key.z - (parent.z << up)) as f32 / n;
                        let uv = Rect::from_min_size(pos2(u, v), vec2(1.0 / n, 1.0 / n));
                        painter.image(tex.id(), rect, uv, Color32::WHITE);
                        break;
                    }
                }
            }

            // Map outline.
            let t = TILE_SIZE as f64;
            let outline = Rect::from_min_max(
                self.to_screen(canvas, lo.x as f64 * t, lo.z as f64 * t),
                self.to_screen(canvas, (hi.x + 1) as f64 * t, (hi.z + 1) as f64 * t),
            );
            painter.rect_stroke(outline, 0, Line::new(1.0, theme::scale(pal.border, 0.85)), StrokeKind::Outside);
        }

        if self.grid {
            self.draw_grid(&painter, canvas, pal);
        }

        // Origin marker.
        let o = self.to_screen(canvas, 0.0, 0.0);
        if canvas.contains(o) {
            let red = Color32::from_rgb(230, 40, 40);
            painter.line_segment([o - vec2(0.0, 7.0), o + vec2(0.0, 7.0)], Line::new(1.5, red));
            painter.line_segment([o - vec2(4.0, 0.0), o + vec2(4.0, 0.0)], Line::new(1.5, red));
        }

        // The river being laid out, with its handles; before a source is
        // placed, where water let go under the pointer would go.
        if tools.tool == Tool::River {
            let spec = tools.river.spring_spec(&doc.world.registry);
            if let Some(draft) = &tools.river_draft {
                self.spring = None;
                let hover = match (self.river_grab, pointer.filter(|_| response.hovered())) {
                    (None, Some(p)) => self.river_hit(canvas, draft, &tools.river, p),
                    _ => None,
                };
                self.draw_river(&painter, canvas, &draft.plan, &spec, Some((draft, hover, self.river_grab)));
                if response.hovered() {
                    ui.ctx().set_cursor_icon(match (self.river_grab, hover) {
                        (Some(_), _) => CursorIcon::Grabbing,
                        (None, Some(RiverHit::Anchor(_) | RiverHit::Mouth)) => CursorIcon::Grab,
                        (None, Some(RiverHit::Course(_))) => CursorIcon::PointingHand,
                        (None, None) => CursorIcon::Crosshair,
                    });
                }
            } else {
                // Worked out once the pointer rests, the way the river goes.
                if let Some((x, z)) = world_pointer.filter(|_| response.hovered()) {
                    let key = (x.floor() as i32, z.floor() as i32, doc.revision ^ (tools.river.lakes.to_bits() as u64) << 32);
                    let now = ui.input(|i| i.time);
                    match self.spring_rest {
                        Some((k, since)) if k == key => {
                            if now - since > 0.12 && self.spring.as_ref().is_none_or(|s| s.0 != key) {
                                let sea = doc.world.meta.sea_level as f32;
                                let d = massif_core::river::descend(doc.dimension().ground(), (key.0, key.1), sea, 30_000, 600_000, tools.river.lake_depth());
                                self.spring = Some((key, d));
                            }
                        }
                        _ => self.spring_rest = Some((key, now)),
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(130));
                }
                if let Some((_, d)) = &self.spring {
                    self.draw_river(&painter, canvas, d, &spec, None);
                }
                let hint = match self.spring.as_ref().map(|s| &s.1) {
                    Some(d) if d.course.len() >= 3 => format!("Click: a river springs here and runs to {}", d.mouth.map_or("", |m| m.label())),
                    _ => "Click on land: where the river springs".to_owned(),
                };
                let at = canvas.center_bottom() - vec2(0.0, 22.0);
                let galley = painter.layout_no_wrap(hint, FontId::proportional(12.5), Color32::from_gray(235));
                let back = Rect::from_center_size(at, galley.size() + vec2(24.0, 12.0));
                painter.rect_filled(back.translate(vec2(0.0, 2.0)), 9, Color32::from_black_alpha(60));
                painter.rect(back, 9, Color32::from_rgba_unmultiplied(20, 22, 27, 235), Line::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
                painter.galley(back.min + vec2(12.0, 6.0), galley, Color32::from_gray(235));
            }
        } else {
            self.spring = None;
        }
        if tools.tool == Tool::River && tools.river_draft.is_some() {
            self.river_bar(ui, &painter, canvas, response.id, doc, tools, feedback);
        }

        // Brush outline.
        if let Some(p) = pointer.filter(|p| canvas.contains(*p))
            && (response.hovered() || self.stroke.is_some())
        {
            if panning {
                ui.ctx().set_cursor_icon(if down[0] { CursorIcon::Grabbing } else { CursorIcon::Grab });
            } else if tools.tool.uses_brush() {
                let brush = &tools.brush;
                let shadow = Line::new(3.0, Color32::from_black_alpha(90));
                let line = Line::new(1.4, Color32::WHITE);
                // The automatic tools show what they would change.
                if matches!(tools.tool, Tool::AutoPaint | Tool::AutoBiome)
                    && !modifiers.alt
                    && self.stroke.is_none()
                    && let Some((x, z)) = world_pointer
                {
                    self.show_auto_preview(ui.ctx(), &painter, canvas, doc, tools, (x, z));
                }
                // How hard the brush presses: lighter where it presses
                // more, with a line at a quarter, a half and three quarters
                // of its strength.
                let key = BrushPreview::key(brush);
                if self.strength.as_ref().is_none_or(|s| s.key != key) {
                    self.strength = Some(BrushPreview::new(ui.ctx(), brush));
                }
                let automatic = matches!(tools.tool, Tool::AutoPaint | Tool::AutoBiome);
                if let Some(preview) = self.strength.as_ref().filter(|_| !automatic) {
                    let side = 2.0 * brush.reach() * self.zoom;
                    if side >= 12.0 {
                        let square = Rect::from_center_size(p, Vec2::splat(side));
                        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                        painter.image(preview.texture.id(), square, uv, Color32::WHITE);
                        for (k, rings) in preview.levels.iter().enumerate() {
                            let ink = Line::new(1.1, Color32::from_white_alpha(105 + 45 * k as u8));
                            let under = Line::new(2.4, Color32::from_black_alpha(55));
                            for ring in rings {
                                let points = thin(ring.iter().map(|&(u, v)| square.min + vec2(u, v) * side), 1.5);
                                if points.len() > 2 {
                                    painter.add(egui::Shape::closed_line(points.clone(), under));
                                    painter.add(egui::Shape::closed_line(points, ink));
                                }
                            }
                        }
                    }
                }
                let mut rings: Vec<Vec<Pos2>> = Vec::new();
                if let Some((x, z)) = world_pointer
                    && self.zoom * ppp >= EXACT_ZOOM
                    && brush.reach() <= EXACT_REACH
                {
                    // Close enough to see the blocks: exactly the columns the
                    // brush changes.
                    for ring in brush.footprint_outline(x as f32, z as f32) {
                        rings.push(ring.iter().map(|&(bx, bz)| self.to_screen(canvas, bx as f64, bz as f64)).collect());
                    }
                } else if let Some(outline) = brush.outline() {
                    // A custom brush: where its image paints, pixel by pixel.
                    for ring in outline {
                        rings.push(thin(ring.iter().map(|&(dx, dz)| p + vec2(dx * self.zoom, dz * self.zoom)), 0.75));
                    }
                } else {
                    match brush.shape {
                        BrushShape::Circle => {
                            let r = brush.radius * self.zoom;
                            painter.circle_stroke(p, r, shadow);
                            painter.circle_stroke(p, r, line);
                        }
                        BrushShape::Square => rings.push(
                            brush
                                .square_corners(0.0, 0.0)
                                .iter()
                                .map(|(x, z)| p + vec2(x * self.zoom, z * self.zoom))
                                .collect(),
                        ),
                    }
                }
                rings.retain(|r| r.len() > 2);
                for ring in &rings {
                    painter.add(egui::Shape::closed_line(ring.clone(), shadow));
                }
                for ring in rings {
                    painter.add(egui::Shape::closed_line(ring, line));
                }
                painter.circle_filled(p, 1.5, Color32::WHITE);
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            } else if tools.tool != Tool::River || tools.river_draft.is_none() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
        }

        if doc.dimension().bounds().is_none() {
            painter.text(
                canvas.center(),
                Align2::CENTER_CENTER,
                "This world has no map yet: create one with File ▸ New World",
                FontId::proportional(14.0),
                pal.weak,
            );
        }

        self.draw_scale(&painter, canvas, pal);
        self.footer(ui, footer, doc, pal, *hover);
    }

    fn draw_grid(&self, painter: &egui::Painter, canvas: Rect, pal: Palette) {
        let (wx0, wz0) = self.to_world(canvas, canvas.min);
        let (wx1, wz1) = self.to_world(canvas, canvas.max);
        let lines = |step: f64, color: Color32| {
            if step * self.zoom as f64 * 1.0 < 12.0 {
                return;
            }
            let mut x = (wx0 / step).floor() * step;
            while x <= wx1 {
                let a = self.to_screen(canvas, x, wz0);
                painter.vline(a.x, canvas.y_range(), Line::new(1.0, color));
                x += step;
            }
            let mut z = (wz0 / step).floor() * step;
            while z <= wz1 {
                let a = self.to_screen(canvas, wx0, z);
                painter.hline(canvas.x_range(), a.y, Line::new(1.0, color));
                z += step;
            }
        };
        let base = if pal.dark { Color32::WHITE } else { Color32::BLACK };
        lines(16.0, base.gamma_multiply(0.08));
        lines(TILE_SIZE as f64, base.gamma_multiply(0.22));
    }

    fn draw_scale(&self, painter: &egui::Painter, canvas: Rect, pal: Palette) {
        // A round length close to 110 points.
        let target = 110.0 / self.zoom;
        let pow = 10f32.powf(target.log10().floor());
        let len = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|m| m * pow)
            .rfind(|l| *l <= target)
            .unwrap_or(pow);
        let px = len * self.zoom;
        let base = pos2(canvas.left() + 12.0, canvas.bottom() - 12.0);
        let bg = Rect::from_min_max(base - vec2(4.0, 30.0), base + vec2(px + 40.0, 6.0));
        painter.rect_filled(bg, 6, pal.panel.gamma_multiply(0.86));
        // North arrow.
        painter.text(base + vec2(6.0, -17.0), Align2::CENTER_CENTER, "N", FontId::proportional(10.0), pal.text);
        let tip = base + vec2(6.0, -9.0);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, tip + vec2(4.0, 8.0), tip + vec2(-4.0, 8.0)],
            pal.accent,
            Line::NONE,
        ));
        let x0 = base.x + 22.0;
        let y = base.y - 2.0;
        let line = Line::new(1.5, pal.text);
        painter.hline(x0..=x0 + px, y, line);
        painter.vline(x0, (y - 5.0)..=y, line);
        painter.vline(x0 + px, (y - 5.0)..=y, line);
        let label = if len >= 1000.0 {
            format!("{} km", len / 1000.0)
        } else {
            format!("{len} blocks")
        };
        painter.text(pos2(x0 + 4.0, y - 4.0), Align2::LEFT_BOTTOM, label, FontId::proportional(10.5), pal.text);
    }

    fn footer(&mut self, ui: &mut Ui, rect: Rect, doc: &Document, pal: Palette, hover: Option<(i32, i32)>) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, pal.header);
        painter.hline(rect.x_range(), rect.top(), Line::new(1.0, pal.border));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(8.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let location = match hover {
            Some((x, z)) => {
                let key = (x, z, doc.revision ^ doc.generation.rotate_left(32));
                let probe = match self.probe {
                    Some((k, p)) if k == key => p,
                    _ => {
                        let p = tools::probe(doc, x, z);
                        self.probe = Some((key, p));
                        p
                    }
                };
                match probe {
                    Some(p) => {
                        let terrain = &doc.world.registry.terrain(TerrainId(p.terrain)).name;
                        let water = if p.fluid != massif_core::NO_FLUID {
                            format!(" · water {}", p.fluid)
                        } else {
                            String::new()
                        };
                        let cover = doc.world.registry.cover(p.cover).map(|c| format!(" · {}", c.name)).unwrap_or_default();
                        format!("Location: {x}, {z} · height {:.1} · {terrain}{cover}{water}", p.height)
                    }
                    None => format!("Location: {x}, {z}"),
                }
            }
            None => "Location: –".to_owned(),
        };
        child.label(egui::RichText::new(location).size(11.5).color(pal.weak));
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(theme::glyph(Icon::Maximize2).to_string()).on_hover_text("Fit the whole map").clicked() {
                self.fitted = None;
            }
            let canvas = Rect::from_min_max(rect.min - vec2(0.0, 600.0), rect.max);
            if ui.small_button("+").clicked() {
                let c = canvas.center();
                self.zoom_at(canvas, c, 2.0);
            }
            ui.label(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).size(11.5));
            if ui.small_button("−").clicked() {
                let c = canvas.center();
                self.zoom_at(canvas, c, 0.5);
            }
            let mut log = self.zoom.log2();
            ui.spacing_mut().interact_size.y = 16.0;
            let slider = egui::Slider::new(&mut log, MIN_ZOOM.log2()..=MAX_ZOOM.log2()).show_value(false);
            if ui.add_sized([90.0, 16.0], slider).changed() {
                self.zoom = log.exp2();
            }
        });
    }
}

/// Whether the wheel turns the brush: with ⌘ on a Mac (the Windows key of a
/// PC keyboard), with Ctrl + Shift elsewhere, where the Windows key does not
/// reach applications.
fn turns_brush(m: egui::Modifiers) -> bool {
    m.mac_cmd || (!cfg!(target_os = "macos") && m.ctrl && m.shift)
}

/// The points of a path, without those closer than `min` to the last kept.
fn thin(points: impl Iterator<Item = Pos2>, min: f32) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::new();
    for p in points {
        if out.last().is_none_or(|q| q.distance(p) >= min) {
            out.push(p);
        }
    }
    out
}
