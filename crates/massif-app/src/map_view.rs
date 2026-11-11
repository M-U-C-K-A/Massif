//! The 2D map editor: navigation, brush strokes and overlays.

use crate::doc::Document;
use crate::map_render::{CHUNK_PX, ChunkKey, MAX_LOD, MapCache, MapMode, MapStyle};
use crate::theme::{self, Icon, Palette};
use crate::tools::{self, Stroke, Tool, ToolState};
use egui::{
    Align2, Color32, CursorIcon, FontId, PointerButton, Pos2, Rect, Sense, Stroke as Line,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use massif_core::{BrushShape, TILE_SIZE, TerrainId};
use std::time::Duration;

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
        egui::ComboBox::from_id_salt(id)
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
            canvas: Rect::NOTHING,
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
            if tools.tool.is_click() {
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
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(stroke) = self.stroke.take()
        {
            // Escape abandons the stroke being drawn.
            tools::end_stroke(doc, stroke);
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
            } else if let Some(stroke) = self.stroke.take() {
                tools::end_stroke(doc, stroke);
            }
        }

        // ---- drawing -------------------------------------------------------
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0, pal.canvas);
        let dot = theme::mix(pal.canvas, pal.weak, 0.35);
        let spacing = 22.0;
        let mut y = canvas.top() + spacing / 2.0;
        while y < canvas.bottom() {
            let mut x = canvas.left() + spacing / 2.0;
            while x < canvas.right() {
                painter.circle_filled(pos2(x, y), 0.9, dot);
                x += spacing;
            }
            y += spacing;
        }

        let style = MapStyle {
            mode: self.mode,
            contours: self.contours,
            solo: solo.then_some(tools.terrain),
            highlight: (doc.active_level > 0).then_some(doc.active_level.min(7) as u8),
            light,
            lava: doc.world.meta.lava,
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
            } else {
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
                        format!("Location: {x}, {z} · height {:.1} · {terrain}{water}", p.height)
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
