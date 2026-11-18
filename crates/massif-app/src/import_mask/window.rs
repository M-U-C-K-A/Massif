//! The mask window: Import mask at the size of the window. The map large,
//! the image over it live, seen as it is, alone, or as what the import
//! makes of the map (before, after, the two side by side, what changes);
//! how strongly it is laid over the map; the image's histogram with its
//! levels, midtones and blur; a legend of what each grey or colour paints;
//! and where it lands, dragged on the map. Import and Cancel as in the
//! dialog, the import one step to undo.
//!
//! What the import makes of the map is worked out from the map's columns
//! read with the image (`massif_core::mask::columns`, at most 512 across)
//! and the image's picture (at most 512 across), as the import paints them
//! (`massif_core::mask::outcome`): a few milliseconds, whatever the size of
//! the map or of the image, drawn as textures on the GPU.

use super::*;
use massif_core::mask::{Column, Columns, nearest_annotation, outcome, preview_value};
use massif_core::tile::NO_FLUID;

/// What the canvas shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum View {
    /// The map, the image over it in the colour it paints.
    #[default]
    Blend,
    /// The image alone, as its values are read.
    Mask,
    /// What the map has now.
    Before,
    /// What the import makes of it.
    After,
    /// Before on the left of a line, after on its right.
    Split,
    /// After, what changes lit, the rest dimmed.
    Changes,
}

impl View {
    const ALL: [View; 6] = [View::Blend, View::Mask, View::Before, View::After, View::Split, View::Changes];

    fn label(self) -> &'static str {
        match self {
            View::Blend => tr!("Over the map"),
            View::Mask => tr!("Mask"),
            View::Before => tr!("Before"),
            View::After => tr!("After"),
            View::Split => tr!("Before | After"),
            View::Changes => tr!("Changes"),
        }
    }
}

/// The window's own state.
pub(crate) struct Studio {
    pub view: View,
    /// How strongly the image, or what it paints, lies over the map.
    pub overlay: f32,
    /// Where the line of the split view is, across the canvas (0 to 1).
    pub split: f32,
    /// The block in the middle of the canvas and the points a block;
    /// `None`: the map and the image fitted.
    pub cam: Option<([f64; 2], f32)>,
    grid: Option<(u64, Drawn)>,
    pub(super) histogram: Option<(bool, Vec<u32>)>,
    pub(super) colours: Option<Vec<(u16, f32)>>,
}

impl Default for Studio {
    fn default() -> Self {
        Self { view: View::Blend, overlay: 0.7, split: 0.5, cam: None, grid: None, histogram: None, colours: None }
    }
}

/// What the import makes of the map's columns, as pictures of the grid.
pub(crate) struct Computed {
    /// The image's strength at each column (NaN: left alone).
    #[cfg_attr(not(test), allow(dead_code))]
    pub strength: Vec<f32>,
    pub before: Vec<Color32>,
    pub after: Vec<Color32>,
    /// Lit where the column changes.
    pub changes: Vec<Color32>,
    /// The cells that change.
    pub changed: usize,
    /// The cells the map has.
    pub cells: usize,
}

struct Drawn {
    before: egui::TextureHandle,
    after: egui::TextureHandle,
    changes: egui::TextureHandle,
    changed: usize,
    cells: usize,
}

impl MaskForm {
    #[cfg(test)]
    pub(crate) fn set_view(&mut self, view: View) {
        self.studio.view = view;
    }

    /// The cells of the map's columns that the import changes, as last
    /// drawn.
    #[cfg(test)]
    pub(crate) fn changed_columns(&self) -> Option<usize> {
        self.studio.grid.as_ref().map(|g| g.1.changed)
    }

    /// The colour a value of the target's kind shows in (transparent:
    /// nothing of it, the map shows through).
    fn colour_of(&self, target: Target, value: f32) -> Color32 {
        let rgb = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        let id = value as u16;
        match target {
            Target::Annotations(_) => if id == 0 { Color32::TRANSPARENT } else { rgb(ANNOTATION_COLORS[(id.clamp(1, 15) - 1) as usize]) },
            Target::Terrain(_) => self.terrains.iter().find(|t| t.0 == id).map_or(Color32::GRAY, |t| rgb(t.2)),
            Target::Biome(_) => if id == 0 { Color32::TRANSPARENT } else { self.biomes.iter().find(|b| b.0 == id).map_or(Color32::GRAY, |b| rgb(b.2)) },
            Target::Cover(n) | Target::Trees(n) | Target::Understory(n) | Target::Pockets(n) | Target::TreeSize(n) => {
                if id == n { rgb(self.tint()) } else { Color32::TRANSPARENT }
            }
            Target::Water(_) => if value as i16 == NO_FLUID { Color32::TRANSPARENT } else { Color32::from_rgb(64, 120, 230) },
            Target::Height { .. } => {
                let (lo, hi) = (self.heights.0 as f32, self.heights.1 as f32);
                let g = (((value - lo) / (hi - lo).max(1.0)).clamp(0.0, 1.0) * 255.0) as u8;
                Color32::from_gray(g)
            }
        }
    }

    /// What the import makes of the map's columns `cols`, with the form's
    /// settings.
    pub(crate) fn compute(&self, cols: &Columns, accent: Color32) -> Computed {
        let target = self.target();
        let p = self.placement();
        let (w, h) = (self.info.header.width as f64, self.info.header.height as f64);
        let pv = &self.info.preview;
        let colours = target.by_colour();
        let at = |k: usize, base: i32| (base + k as i32 * cols.step + cols.step / 2) as f64 + 0.5;
        let n = cols.w * cols.h;
        let mut strength = vec![f32::NAN; n];
        let mut colour = vec![0u16; if colours { n } else { 0 }];
        for j in 0..cols.h {
            let v = (at(j, cols.z0) - p.z as f64) / p.scale as f64;
            if v < 0.0 || v >= h || pv.height == 0 {
                continue;
            }
            let pj = ((v / h * pv.height as f64) as usize).min(pv.height - 1);
            for i in 0..cols.w {
                let u = (at(i, cols.x0) - p.x as f64) / p.scale as f64;
                if u < 0.0 || u >= w {
                    continue;
                }
                let k = pj * pv.width + ((u / w * pv.width as f64) as usize).min(pv.width - 1);
                strength[j * cols.w + i] = preview_value(&self.info, &self.mapping, k).0;
                if colours && !strength[j * cols.w + i].is_nan() {
                    colour[j * cols.w + i] = nearest_annotation(pv.rgb[k]);
                }
            }
        }
        if !colours && self.mapping.blur > 0.0 {
            let r = (self.mapping.blur / cols.step as f32).round() as usize;
            massif_core::mask::blur_grid(&mut strength, cols.w, r);
        }
        let heights = (self.heights.0 as f32, self.heights.1 as f32);
        let (mut before, mut after, mut changes) = (vec![Color32::TRANSPARENT; n], vec![Color32::TRANSPARENT; n], vec![Color32::TRANSPARENT; n]);
        let (mut changed, mut cells) = (0, 0);
        let lit = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 170);
        let dim = Color32::from_black_alpha(120);
        for j in 0..cols.h {
            for i in 0..cols.w {
                let k = j * cols.w + i;
                let Some(col): Option<Column> = cols.cells[k] else { continue };
                cells += 1;
                let old = col.value(target);
                let c = if colours { colour[k] } else { 0 };
                let new = outcome(&col, strength[k], c, target, self.mode, &self.mapping, at(i, cols.x0) as i32, at(j, cols.z0) as i32, heights);
                before[k] = self.colour_of(target, old);
                after[k] = self.colour_of(target, new);
                if new != old {
                    changed += 1;
                    changes[k] = lit;
                } else {
                    changes[k] = dim;
                }
            }
        }
        Computed { strength, before, after, changes, changed, cells }
    }

    /// The pictures of what the import makes, worked out again when a
    /// setting changes.
    fn drawn(&mut self, ctx: &egui::Context, accent: Color32) -> Option<&Drawn> {
        use std::hash::{Hash, Hasher};
        let cols = self.columns.clone()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}{:?}{:?}{:?}{:?}{:?}{:?}", self.target(), self.mapping, self.mode, self.placement(), self.tint(), accent, self.path).hash(&mut hasher);
        let key = hasher.finish();
        if self.studio.grid.as_ref().is_none_or(|g| g.0 != key) {
            let c = self.compute(&cols, accent);
            let size = [cols.w.max(1), cols.h.max(1)];
            let texture = |name: &str, pixels: Vec<Color32>| {
                let pixels = if pixels.len() == size[0] * size[1] { pixels } else { vec![Color32::TRANSPARENT; size[0] * size[1]] };
                ctx.load_texture(name, egui::ColorImage::new(size, pixels), egui::TextureOptions::NEAREST)
            };
            let drawn = Drawn {
                before: texture("mask window before", c.before),
                after: texture("mask window after", c.after),
                changes: texture("mask window changes", c.changes),
                changed: c.changed,
                cells: c.cells,
            };
            self.studio.grid = Some((key, drawn));
        }
        self.studio.grid.as_ref().map(|g| &g.1)
    }

    /// The window: the canvas on the left, the settings on the right, the
    /// buttons under them.
    pub(crate) fn window_ui(&mut self, ui: &mut Ui, choice: &mut Choice) {
        let pal = Palette::of(ui);
        let room = ui.ctx().content_rect();
        let (width, height) = ((room.width() - 2.0 * 22.0 - 40.0).max(600.0), (room.height() - 2.0 * 22.0 - 48.0).max(400.0));
        ui.set_width(width);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::ImageUp).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Import Mask")).size(16.0).bold());
            let h = &self.info.header;
            let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.label(RichText::new(tr!("{name}: {w} × {h} pixels", name, w = h.width, h = h.height)).size(12.0).color(pal.weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(theme::with_icon(Icon::Minimize2, tr!("Smaller"))).on_hover_text(tr!("Back to the dialog")).clicked() {
                    self.big = Some(false);
                }
            });
        });
        ui.add_space(8.0);
        let side = 460.0f32.min(width * 0.42);
        let body = height - 30.0 - 8.0 - 44.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(width - side - 16.0);
                self.canvas_bar(ui);
                ui.add_space(6.0);
                self.canvas(ui, vec2(width - side - 16.0, body - 34.0));
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(side);
                egui::ScrollArea::vertical().id_salt("mask window side").max_height(body).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(side - 10.0);
                    self.images_ui(ui, choice);
                    self.target_ui(ui);
                    ui.add_space(10.0);
                    self.legend_ui(ui);
                    ui.add_space(10.0);
                    self.levels_ui(ui);
                    ui.add_space(10.0);
                    self.values_ui(ui);
                    ui.add_space(10.0);
                    self.placement_fields(ui);
                    ui.add_space(10.0);
                    self.reference_ui(ui);
                    if self.zone.is_some() {
                        ui.add_space(10.0);
                        self.zone_ui(ui);
                    }
                });
            });
        });
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.buttons(ui, choice);
            if let Some(d) = self.studio.grid.as_ref().map(|g| &g.1) {
                let step = self.columns.as_ref().map_or(1, |c| c.step as usize);
                let columns = d.changed * step * step;
                let share = if d.cells > 0 { d.changed as f32 / d.cells as f32 * 100.0 } else { 0.0 };
                ui.add_space(12.0);
                ui.label(RichText::new(tr!("About {n} columns change, {p} % of the map", n = columns, p = format!("{share:.1}"))).size(12.0).color(pal.weak));
            }
        });
    }

    /// Over the canvas: what it shows, how strongly, and Fit.
    fn canvas_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let views: Vec<View> = View::ALL.iter().copied().filter(|v| self.columns.is_some() || matches!(v, View::Blend | View::Mask)).collect();
            let labels: Vec<&str> = views.iter().map(|v| v.label()).collect();
            let on: Vec<bool> = views.iter().map(|&v| v == self.studio.view).collect();
            if let Some(i) = button_group(ui, "mask window view", &labels, &on) {
                self.studio.view = views[i];
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(theme::with_icon(Icon::Maximize, tr!("Fit"))).clicked() {
                    self.studio.cam = None;
                }
                let mut percent = (self.studio.overlay * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %").text(tr!("Overlay")).width(190.0)).changed() {
                    self.studio.overlay = percent / 100.0;
                }
            });
        });
    }

    /// The map large and the image over it: the left button drags the
    /// image (or the split line), the right or middle one pans, the wheel
    /// zooms about the pointer, the arrow keys nudge a block (Shift: 16).
    fn canvas(&mut self, ui: &mut Ui, size: egui::Vec2) {
        let pal = Palette::of(ui);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let id = response.id;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::GROUP_RADIUS, if pal.dark { Color32::from_gray(14) } else { Color32::from_gray(225) });
        let [x0, z0, x1, z1] = self.bounds;
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        // The camera: the map and the image fitted, until moved.
        let (centre, s) = self.studio.cam.unwrap_or_else(|| {
            let (wx0, wz0, wx1, wz1) = (x0.min(r.x0) as f64, z0.min(r.z0) as f64, x1.max(r.x1) as f64, z1.max(r.z1) as f64);
            let inner = rect.shrink(24.0);
            let s = (inner.width() as f64 / (wx1 - wx0).max(1.0)).min(inner.height() as f64 / (wz1 - wz0).max(1.0)) as f32;
            ([(wx0 + wx1) / 2.0, (wz0 + wz1) / 2.0], s)
        });
        let at = |x: f64, z: f64| pos2(rect.center().x + ((x - centre[0]) as f32) * s, rect.center().y + ((z - centre[1]) as f32) * s);
        let block = |p: egui::Pos2| [centre[0] + ((p.x - rect.center().x) / s) as f64, centre[1] + ((p.y - rect.center().y) / s) as f64];
        // Zoom about the pointer.
        if let Some(p) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
            if scroll != 0.0 {
                let k = (scroll * 0.0025).exp();
                let s2 = (s * k).clamp(0.002, 64.0);
                let b = block(p);
                let c2 = [b[0] - ((p.x - rect.center().x) / s2) as f64, b[1] - ((p.y - rect.center().y) / s2) as f64];
                self.studio.cam = Some((c2, s2));
            }
        }
        let secondary = ui.input(|i| i.pointer.secondary_down() || i.pointer.middle_down());
        let split_x = rect.left() + rect.width() * self.studio.split;
        if response.drag_started() {
            let on_line = self.studio.view == View::Split && response.interact_pointer_pos().is_some_and(|p| (p.x - split_x).abs() < 8.0);
            let what = if secondary { 2u8 } else if on_line { 1 } else { 0 };
            ui.data_mut(|d| d.insert_temp(id.with("drag"), (what, self.x, self.z, egui::Vec2::ZERO)));
            response.request_focus();
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.dragged()
            && let Some((what, sx, sz, moved)) = ui.data(|d| d.get_temp::<(u8, i32, i32, egui::Vec2)>(id.with("drag")))
        {
            let delta = response.drag_delta();
            match what {
                1 => {
                    if let Some(p) = response.interact_pointer_pos() {
                        self.studio.split = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    }
                }
                2 => {
                    self.studio.cam = Some(([centre[0] - (delta.x / s) as f64, centre[1] - (delta.y / s) as f64], s));
                }
                _ => {
                    let moved = moved + delta;
                    ui.data_mut(|d| d.insert_temp(id.with("drag"), (what, sx, sz, moved)));
                    let (dx, dz) = drag_blocks(moved, s);
                    // The view stays where it is while the image moves.
                    self.studio.cam = Some((centre, s));
                    self.nudge(sx + dx - self.x, sz + dz - self.z);
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<(u8, i32, i32, egui::Vec2)>(id.with("drag")));
        }
        if response.has_focus() || response.hovered() {
            let (dx, dz) = ui.input(|i| {
                let step = if i.modifiers.shift { 16 } else { 1 };
                let k = |key| i.key_pressed(key) as i32 * step;
                (k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft), k(egui::Key::ArrowDown) - k(egui::Key::ArrowUp))
            });
            if (dx, dz) != (0, 0) {
                self.studio.cam = Some((centre, s));
                self.nudge(dx, dz);
            }
        }
        if response.hovered() {
            let near_line = self.studio.view == View::Split && response.hover_pos().is_some_and(|p| (p.x - split_x).abs() < 8.0);
            ui.ctx().set_cursor_icon(if near_line {
                egui::CursorIcon::ResizeHorizontal
            } else if response.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }

        // ---- drawing ------------------------------------------------------
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        let map = Rect::from_min_max(at(x0 as f64, z0 as f64), at(x1 as f64, z1 as f64));
        let img = Rect::from_min_max(at(r.x0 as f64, r.z0 as f64), at(r.x1 as f64, r.z1 as f64));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let alpha = |a: f32| Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0) as u8);
        if let Some((image, _)) = self.map_picture.take() {
            self.map_texture = Some(ui.ctx().load_texture("import mask map", image, egui::TextureOptions::LINEAR));
        }
        let view = self.studio.view;
        let overlay = self.studio.overlay;
        let base = if view == View::Mask { 0.3 } else { 1.0 };
        match &self.map_texture {
            Some(tex) => {
                painter.image(tex.id(), map, uv, alpha(base));
            }
            None => {
                painter.rect_filled(map, 0.0, theme::scale(pal.border, 0.6));
            }
        }
        let grid_rect = self.columns.as_ref().map(|c| Rect::from_min_max(at(c.x0 as f64, c.z0 as f64), at((c.x0 + c.w as i32 * c.step) as f64, (c.z0 + c.h as i32 * c.step) as f64)));
        match view {
            View::Blend => {
                let tex = self.preview_texture(ui.ctx());
                painter.image(tex, img, uv, alpha(overlay));
            }
            View::Mask => {
                let tex = self.mask_texture(ui.ctx());
                painter.image(tex, img, uv, Color32::WHITE);
            }
            View::Before | View::After | View::Changes | View::Split => {
                let accent = pal.accent;
                if let (Some(grid), Some(d)) = (grid_rect, self.drawn(ui.ctx(), accent)) {
                    match view {
                        View::Before => {
                            painter.image(d.before.id(), grid, uv, alpha(overlay));
                        }
                        View::After => {
                            painter.image(d.after.id(), grid, uv, alpha(overlay));
                        }
                        View::Changes => {
                            painter.image(d.after.id(), grid, uv, alpha(overlay));
                            painter.image(d.changes.id(), grid, uv, Color32::WHITE);
                        }
                        _ => {
                            let left = Rect::from_min_max(rect.min, pos2(split_x, rect.max.y));
                            let right = Rect::from_min_max(pos2(split_x, rect.min.y), rect.max);
                            painter.with_clip_rect(left.intersect(rect)).image(d.before.id(), grid, uv, alpha(overlay));
                            painter.with_clip_rect(right.intersect(rect)).image(d.after.id(), grid, uv, alpha(overlay));
                        }
                    }
                }
            }
        }
        painter.rect_stroke(map, 0.0, Stroke::new(1.0, theme::scale(pal.border, 0.85)), StrokeKind::Outside);
        painter.rect_stroke(img, 0.0, Stroke::new(1.5, pal.accent), StrokeKind::Outside);
        if view == View::Split {
            painter.vline(split_x, rect.y_range(), Stroke::new(2.0, pal.text));
            let font = egui::FontId::proportional(11.5);
            let label = |x: f32, align: egui::Align2, text: &str| {
                let g = painter.layout_no_wrap(text.to_owned(), font.clone(), pal.text);
                let r = align.anchor_size(pos2(x, rect.top() + 10.0), g.size()).expand2(vec2(8.0, 3.0));
                painter.rect_filled(r, theme::PILL, theme::scale(pal.panel, 0.95));
                painter.galley(r.min + vec2(8.0, 3.0), g, pal.text);
            };
            label(split_x - 8.0, egui::Align2::RIGHT_TOP, tr!("Before"));
            label(split_x + 8.0, egui::Align2::LEFT_TOP, tr!("After"));
        }
        let stroke = if response.has_focus() { pal.accent } else { pal.border };
        painter.rect_stroke(rect, theme::GROUP_RADIUS, Stroke::new(1.0, stroke), StrokeKind::Inside);
        if self.columns.is_none() && matches!(view, View::Before | View::After | View::Split | View::Changes) {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr!("The map's columns are not read: add the image again"), egui::FontId::proportional(12.0), pal.weak);
        }
    }

    /// The image alone, in grey as its values are read (levels and all),
    /// transparent where it leaves the map as it is.
    fn mask_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("mask{:?}{:?}{:?}", self.mapping, self.target(), self.path).hash(&mut h);
        let key = h.finish();
        if let Some((tex, k)) = &self.mask_preview
            && *k == key
        {
            return tex.id();
        }
        let p = &self.info.preview;
        let target = self.target();
        let mut v: Vec<f32> = (0..p.width * p.height).map(|k| preview_value(&self.info, &self.mapping, k).0).collect();
        if self.mapping.blur > 0.0 && !target.by_colour() {
            // The picture's pixels stand for this many blocks each.
            let blocks = self.scale as f64 * self.info.header.width as f64 / p.width.max(1) as f64;
            massif_core::mask::blur_grid(&mut v, p.width, (self.mapping.blur as f64 / blocks.max(1e-6)).round() as usize);
        }
        let threshold = self.mapping.threshold.filter(|_| !matches!(target, Target::Height { .. }) && !target.by_colour());
        let pixels: Vec<Color32> = v
            .iter()
            .enumerate()
            .map(|(k, &v)| {
                if v.is_nan() {
                    return Color32::TRANSPARENT;
                }
                if target.by_colour() {
                    let c = ANNOTATION_COLORS[(nearest_annotation(p.rgb[k]).clamp(1, 15) - 1) as usize];
                    return Color32::from_rgb(c[0], c[1], c[2]);
                }
                let v = match threshold {
                    Some(t) => (v >= t) as u8 as f32,
                    None => v,
                };
                Color32::from_gray((v.clamp(0.0, 1.0) * 255.0) as u8)
            })
            .collect();
        let tex = ctx.load_texture("mask window mask", egui::ColorImage::new([p.width.max(1), p.height.max(1)], pixels), egui::TextureOptions::NEAREST);
        let id = tex.id();
        self.mask_preview = Some((tex, key));
        id
    }

    /// The image's greys, as many as there are of each, from black to
    /// white of its range (64 bars).
    pub(crate) fn histogram(&mut self) -> &[u32] {
        let full = self.mapping.full_range;
        if self.studio.histogram.as_ref().is_none_or(|h| h.0 != full) {
            let mut bins = vec![0u32; 64];
            let m = Mapping { levels: [0.0, 1.0], gamma: 1.0, black_is_none: false, invert: false, ..self.mapping };
            for k in 0..self.info.preview.grey.len() {
                let g = preview_value(&self.info, &m, k).1;
                if !g.is_nan() {
                    bins[((g * 63.999) as usize).min(63)] += 1;
                }
            }
            self.studio.histogram = Some((full, bins));
        }
        &self.studio.histogram.as_ref().expect("just made").1
    }

    /// The histogram with the levels over it, and the levels, midtones and
    /// blur.
    fn levels_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        if self.kind == Kind::Annotations && self.info.colour && self.by_colour {
            return;
        }
        ui.label(RichText::new(tr!("Levels")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let bins = self.histogram().to_vec();
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::GROUP_RADIUS, pal.well());
        let inner = rect.shrink2(vec2(10.0, 8.0));
        let most = bins.iter().copied().max().unwrap_or(1).max(1) as f32;
        let bar = inner.width() / bins.len() as f32;
        for (k, &n) in bins.iter().enumerate() {
            let t = (n as f32 / most).sqrt();
            if t > 0.0 {
                let x = inner.left() + k as f32 * bar;
                painter.rect_filled(Rect::from_min_max(pos2(x, inner.bottom() - t * inner.height()), pos2(x + bar - 0.5, inner.bottom())), 0.0, theme::scale(pal.weak, 0.8));
            }
        }
        // The curve the levels and midtones make.
        let curve: Vec<egui::Pos2> = (0..=64)
            .map(|k| {
                let x = k as f32 / 64.0;
                let y = self.mapping.shape(x);
                pos2(inner.left() + x * inner.width(), inner.bottom() - y * inner.height())
            })
            .collect();
        painter.add(egui::Shape::line(curve, Stroke::new(1.5, pal.accent)));
        let marker = |t: f32| inner.left() + t * inner.width();
        for (t, c) in [(self.mapping.levels[0], Color32::BLACK), (self.mapping.levels[1], Color32::WHITE)] {
            let x = marker(t);
            painter.vline(x, inner.y_range(), Stroke::new(1.0, pal.accent));
            painter.circle(pos2(x, inner.bottom()), 4.5, c, Stroke::new(1.0, pal.accent));
        }
        // The black and white points are dragged on it.
        if let Some(p) = response.interact_pointer_pos()
            && (response.dragged() || response.clicked())
        {
            let t = ((p.x - inner.left()) / inner.width()).clamp(0.0, 1.0);
            let which: usize = ui.data(|d| d.get_temp(response.id)).filter(|_| !response.drag_started() && !response.clicked()).unwrap_or_else(|| ((t - self.mapping.levels[0]).abs() > (t - self.mapping.levels[1]).abs()) as usize);
            ui.data_mut(|d| d.insert_temp(response.id, which));
            if which == 0 {
                self.mapping.levels[0] = t.min(self.mapping.levels[1] - 0.01).max(0.0);
            } else {
                self.mapping.levels[1] = t.max(self.mapping.levels[0] + 0.01).min(1.0);
            }
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        ui.add_space(6.0);
        well(ui, |ui| {
            let [mut a, mut b] = self.mapping.levels.map(|v| (v * 100.0).round());
            row(ui, tr!("Black point"), |ui| {
                if ui.add(Field::new(&mut a, 0.0..=99.0).max_decimals(0).suffix(" %")).changed() {
                    self.mapping.levels[0] = (a / 100.0).min(self.mapping.levels[1] - 0.01);
                }
            });
            row(ui, tr!("White point"), |ui| {
                if ui.add(Field::new(&mut b, 1.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                    self.mapping.levels[1] = (b / 100.0).max(self.mapping.levels[0] + 0.01);
                }
            });
            row(ui, tr!("Midtones"), |ui| {
                ui.add(Field::new(&mut self.mapping.gamma, 0.1..=10.0).max_decimals(2).logarithmic(true));
            });
            row(ui, tr!("Blur"), |ui| {
                ui.add(Field::new(&mut self.mapping.blur, 0.0..=64.0).max_decimals(0).suffix(tr!(" blocks")));
            });
            if self.mapping.adjusted() {
                row(ui, "", |ui| {
                    if ui.button(tr!("Reset the levels")).clicked() {
                        (self.mapping.levels, self.mapping.gamma, self.mapping.blur) = ([0.0, 1.0], 1.0, 0.0);
                    }
                });
            }
        });
    }

    /// The name of what the image paints, and its colour.
    fn target_name(&self) -> (String, Color32) {
        let t = self.tint();
        let c = Color32::from_rgb(t[0], t[1], t[2]);
        let name = match self.kind {
            Kind::Annotations => tr!("Annotations").to_owned(),
            Kind::Layer => self.layers.get(self.layer).map_or_else(String::new, |l| l.name.clone()),
            Kind::Terrain => self.terrains.iter().find(|x| x.0 == self.terrain).map_or_else(String::new, |x| crate::i18n::tr_name(&x.1).to_owned()),
            Kind::Biome => self.biomes.iter().find(|x| x.0 == self.biome).map_or_else(String::new, |x| crate::i18n::tr_name(&x.1).to_owned()),
            Kind::Water => tr!("Water up to y {y}", y = self.water),
            Kind::Height => tr!("Height").to_owned(),
        };
        (name, c)
    }

    /// What each grey or colour of the image paints.
    fn legend_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("What each value paints")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let swatch = |ui: &mut Ui, fill: Color32| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            ui.painter().rect(r, theme::TILE_RADIUS as f32 / 2.0, fill, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        };
        let ramp = |ui: &mut Ui, from: Color32, to: Color32| {
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 12.0), Sense::hover());
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(r.left_top(), from);
            mesh.colored_vertex(r.right_top(), to);
            mesh.colored_vertex(r.right_bottom(), to);
            mesh.colored_vertex(r.left_bottom(), from);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            ui.painter().add(mesh);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        };
        let (name, colour) = self.target_name();
        let colours = self.kind == Kind::Annotations && self.info.colour && self.by_colour;
        if colours && self.studio.colours.is_none() {
            let mut counts = [0u32; 16];
            for (g, c) in self.info.preview.grey.iter().zip(&self.info.preview.rgb) {
                if !g.is_nan() && *c != [0, 0, 0] {
                    counts[nearest_annotation(*c) as usize] += 1;
                }
            }
            let total = self.info.preview.grey.len().max(1) as f32;
            let mut list: Vec<(u16, f32)> = (1..16u16).filter(|&k| counts[k as usize] > 0).map(|k| (k, counts[k as usize] as f32 / total)).collect();
            list.sort_by(|a, b| b.1.total_cmp(&a.1));
            self.studio.colours = Some(list);
        }
        let mapping = self.mapping;
        let mode = self.mode;
        well(ui, |ui| {
            // The swatch, then what it paints, from the left.
            let line = |ui: &mut Ui, left: &dyn Fn(&mut Ui), text: String| {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
                left(&mut child);
                child.add_space(8.0);
                child.add(egui::Label::new(RichText::new(text).size(12.0)).truncate());
            };
            if colours {
                for &(k, share) in self.studio.colours.as_deref().unwrap_or_default() {
                    let c = ANNOTATION_COLORS[(k - 1) as usize];
                    line(ui, &|ui| swatch(ui, Color32::from_rgb(c[0], c[1], c[2])), tr!("Annotation {n}, {p} % of the image", n = k, p = format!("{:.1}", share * 100.0)));
                }
                line(ui, &|ui| swatch(ui, Color32::BLACK), tr!("Black: left as it is").to_owned());
                return;
            }
            match (self.kind, mapping.threshold) {
                (Kind::Height, _) => {
                    let (b, w) = if mapping.invert { (self.white, self.black) } else { (self.black, self.white) };
                    line(ui, &|ui| ramp(ui, Color32::BLACK, Color32::WHITE), tr!("Black y {b} to white y {w}", b = format!("{b:.0}"), w = format!("{w:.0}")));
                }
                (_, Some(t)) => {
                    let from = format!("{:.0}", t * 100.0);
                    line(ui, &|ui| swatch(ui, colour), tr!("From {t} %: {name}", t = from, name = name));
                    let off = match mode {
                        Mode::Replace if self.kind != Kind::Terrain => tr!("Under {t} %: rubbed out", t = from),
                        Mode::Min => tr!("Under {t} %: rubbed out", t = from),
                        _ => tr!("Under {t} %: left as it is", t = from),
                    };
                    line(ui, &|ui| swatch(ui, Color32::TRANSPARENT), off);
                }
                (_, None) => {
                    line(ui, &|ui| ramp(ui, Color32::TRANSPARENT, colour), tr!("{name}, as dense as the grey is bright", name = name));
                }
            }
            if mapping.black_is_none {
                line(ui, &|ui| swatch(ui, Color32::BLACK), tr!("Black: left as it is").to_owned());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::form;
    use super::*;
    use massif_core::mask::Column;

    /// A map of 1,024 blocks from -512, a column every 8 blocks, grass
    /// (terrain 0) without biome, cover or water.
    fn columns() -> Columns {
        let col = Column { present: true, top: 64.0, fluid: NO_FLUID, ..Column::default() };
        Columns { x0: -512, z0: -512, step: 8, w: 128, h: 128, cells: vec![Some(col); 128 * 128] }
    }

    #[test]
    fn the_preview_shows_what_changes_where_the_image_lands() {
        // A white image of 512 × 512 at a block a pixel, from x 0, z 0: a
        // quarter of the map.
        let mut f = form(512, 512);
        f.kind = Kind::Biome;
        f.biome = 3;
        f.mapping.black_is_none = false;
        let cols = columns();
        let c = f.compute(&cols, Color32::RED);
        assert_eq!(c.cells, 128 * 128);
        assert_eq!(c.changed, 64 * 64, "the quarter under the image");
        let k = |x: i32, z: i32| ((z + 512) / 8 * 128 + (x + 512) / 8) as usize;
        assert_eq!(c.before[k(100, 100)], Color32::TRANSPARENT, "no biome before");
        assert_ne!(c.after[k(100, 100)], Color32::TRANSPARENT, "the biome after");
        assert_eq!(c.after[k(-100, 100)], Color32::TRANSPARENT, "outside the image, nothing");
        assert!(c.changes[k(100, 100)].r() > 0 && c.changes[k(100, 100)].g() == 0, "lit in the accent");
        assert_eq!(c.changes[k(-100, 100)], Color32::from_black_alpha(120), "the rest dimmed");
        assert!(c.strength[k(-100, 100)].is_nan() && c.strength[k(100, 100)] == 1.0);
        // Moved west by 256 blocks, it still covers a quarter of the map.
        f.nudge(-256, 0);
        assert_eq!(f.compute(&cols, Color32::RED).changed, 64 * 64);
        // Levels that take its white for black paint nothing.
        f.mapping.levels = [1.0, 1.0];
        f.mapping.invert = true;
        assert_eq!(f.compute(&cols, Color32::RED).changed, 0);
    }

    #[test]
    fn the_histogram_counts_the_picture() {
        let mut f = form(4, 4);
        let bins = f.histogram();
        assert_eq!(bins.len(), 64);
        assert_eq!(bins[63], 1, "the one white pixel of the picture");
        assert_eq!(bins.iter().sum::<u32>(), 1);
    }

    /// How long the views take to work out for a mask of 4,096 × 4,096 on
    /// a map read 512 × 512 columns (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn the_views_are_quick() {
        let mut f = form(4096, 4096);
        f.kind = Kind::Terrain;
        f.mapping.threshold = None;
        f.mapping.blur = 24.0;
        let col = Column { present: true, top: 64.0, fluid: NO_FLUID, ..Column::default() };
        let cols = Columns { x0: -2048, z0: -2048, step: 8, w: 512, h: 512, cells: vec![Some(col); 512 * 512] };
        f.bounds = [-2048, -2048, 2048, 2048];
        f.set_origin(Origin::Fit);
        let best = (0..5)
            .map(|_| {
                let t = std::time::Instant::now();
                std::hint::black_box(f.compute(&cols, Color32::RED));
                t.elapsed().as_secs_f64() * 1000.0
            })
            .fold(f64::MAX, f64::min);
        println!("views of 512 × 512 columns: {best:.1} ms");
    }
}
