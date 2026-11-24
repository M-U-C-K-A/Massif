//! The Diorama area: the open map seen from straight above as the game
//! would show it, block by block, with the game's textures, the trees,
//! plants and structures the export places, and relief shading (see
//! `massif_render::diorama`). Drag to pan, the wheel to zoom down to the
//! blocks, Space to come back to the map's middle; C shows the caves under
//! a height moved with Page Up and Page Down; R turns the view a quarter.
//!
//! The tiles are built in the background, nearest the view first, and kept
//! (their floors) up to a budget of memory; an edit builds its tiles again,
//! the old ones shown until then.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::widgets;
use egui::{Align2, Color32, FontId, Key, PointerButton, Rect, Sense, Ui, pos2, vec2};
use massif_core::coords::TilePos;
use massif_render::Materials;
use massif_render::diorama::{Builder, DioramaGpu, TileFloors, View, colours, far_layout};
use massif_render::map_source::target_for;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Points a block takes on screen, at most and at least.
const MOST_ZOOM: f32 = 48.0;
const LEAST_ZOOM: f32 = 1.0 / 32.0;
/// Memory the floors of built tiles may take; the oldest seen go past it.
const FLOORS_BUDGET: usize = 384 << 20;
/// Time a frame spends taking built tiles in.
const TAKE_BUDGET: Duration = Duration::from_millis(5);
/// Time a frame spends redrawing the small images of the tiles for the
/// caves at another height.
const REDRAW_BUDGET: Duration = Duration::from_millis(6);
/// Height of the strips over the top and bottom of the view.
const STRIP: f32 = 24.0;

/// What the map's tiles are, as the Diorama has them.
struct Scene {
    gpu: DioramaGpu,
    builder: Builder,
    materials: Arc<Materials>,
    /// The floors of built tiles, and when each was last seen.
    floors: HashMap<TilePos, (Arc<TileFloors>, u64)>,
    bytes: usize,
    /// Every tile of the map, and those whose small image is on the card.
    tiles: Vec<TilePos>,
    map: HashSet<TilePos>,
    far: HashSet<TilePos>,
    /// Tiles whose small image is for another height than the view's.
    stale: HashSet<TilePos>,
    /// What the small images show: the surface, or the caves under a height.
    far_slice: Option<i32>,
    generation: u64,
    catalogue: Option<usize>,
    dim: usize,
    /// Tiles asked for and not built yet.
    asked: HashSet<TilePos>,
    started: Instant,
    /// When the first tile came, and the whole map.
    first: Option<Duration>,
    whole: Option<Duration>,
    /// Seconds of threads the tiles took to build, and how many.
    build_time: f64,
    built: usize,
    /// The map's middle, in blocks.
    middle: [f64; 2],
    min_y: i32,
    max_y: i32,
    /// The tiles last asked for, in view.
    last_wanted: Vec<TilePos>,
    frame: u64,
}

/// The least time between two starts of a scene's builder on the edits of
/// a stroke under way (the Diorama's, the Render side's): each start drops
/// what it had under way, so that starting every frame built nothing until
/// the stroke ended.
pub const RESTART_EVERY: Duration = Duration::from_millis(250);

/// The Diorama view.
pub struct Diorama {
    /// The block at the middle of the view, x and z.
    pub center: Option<[f64; 2]>,
    /// Points a block takes.
    pub zoom: f32,
    /// Quarter turns clockwise from north up.
    pub turn: u8,
    /// The caves under the slice, else the surface.
    pub caves: bool,
    /// The height the caves are seen under.
    pub slice: i32,
    /// How strong the relief's light and shadows are.
    pub relief: f32,
    scene: Option<Scene>,
    dirty: HashSet<TilePos>,
    /// When the builder last started again on edits: during a stroke, at
    /// most every [`RESTART_EVERY`].
    restarted: Option<Instant>,
    texture: Option<egui::TextureId>,
    texture_size: [u32; 2],
    /// The block under the pointer, and the height of its floor seen.
    pub hover: Option<(i32, Option<i32>, i32)>,
    /// How long the last frame's drawing took on this side, for tests.
    pub frame_time: Duration,
}

impl Default for Diorama {
    fn default() -> Self {
        Self { center: None, zoom: 2.0, turn: 0, caves: false, slice: 40, relief: 1.0, scene: None, dirty: HashSet::new(), restarted: None, texture: None, texture_size: [0; 2], hover: None, frame_time: Duration::ZERO }
    }
}

/// What the Diorama took to build, for the status and tests.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress {
    pub built: usize,
    pub tiles: usize,
    pub first: Option<Duration>,
    pub whole: Option<Duration>,
    /// Mean seconds of a thread a tile took.
    pub per_tile: f64,
}

impl Diorama {
    /// Tiles the map changed in: built again, the old ones shown until then.
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        if self.scene.is_some() {
            self.dirty.extend(tiles.iter().copied());
        }
    }

    /// How far the building is.
    pub fn progress(&self) -> Progress {
        let Some(s) = &self.scene else { return Progress::default() };
        Progress { built: s.built, tiles: s.tiles.len(), first: s.first, whole: s.whole, per_tile: if s.built > 0 { s.build_time / s.built as f64 } else { 0.0 } }
    }

    /// Whether every tile asked for is built and drawn.
    #[cfg(test)]
    pub fn settled(&self) -> bool {
        self.scene.as_ref().is_some_and(|s| s.builder.pending() == 0 && s.asked.is_empty() && s.stale.is_empty() && s.far.len() >= s.tiles.len())
    }

    /// The header's commands: what is shown and how it is turned.
    pub fn header(&mut self, ui: &mut Ui) {
        let labels = [tr!("Surface"), tr!("Caves")];
        if let Some(k) = widgets::button_group(ui, "diorama mode", &labels, &[!self.caves, self.caves]) {
            self.caves = k == 1;
        }
        ui.add_space(4.0);
        if ui.button(theme::glyph(Icon::RotateCw).to_string()).on_hover_text(tr!("Turn the view a quarter (R)")).clicked() {
            self.turn = (self.turn + 1) % 4;
        }
    }

    /// The view, filling `ui`.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, catalogue: Option<Arc<massif_assets::Catalogue>>) {
        let started = Instant::now();
        let pal = Palette::of(ui);
        let view = ui.available_rect_before_wrap();
        ui.painter().rect_filled(view, 0.0, pal.canvas);
        let Some(render_state) = frame.wgpu_render_state() else {
            ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("The Diorama needs the wgpu renderer"), FontId::proportional(13.0), pal.weak);
            return;
        };
        let (device, queue) = (&render_state.device, &render_state.queue);
        let cat_key = catalogue.as_ref().map(|c| Arc::as_ptr(c) as usize);
        if self.scene.as_ref().is_some_and(|s| s.generation != doc.generation || s.catalogue != cat_key || s.dim != doc.dim) {
            self.scene = None;
            self.center = None;
        }
        if self.scene.is_none() {
            match make_scene(device, queue, doc, catalogue.clone(), cat_key) {
                Ok(Some(scene)) => {
                    if self.center.is_none() {
                        self.center = Some(scene.middle);
                        self.slice = (doc.world.meta.sea_level - 8).clamp(scene.min_y + 1, scene.max_y);
                    }
                    self.dirty.clear();
                    self.scene = Some(scene);
                }
                Ok(None) => {
                    ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("Nothing to show: the map is empty"), FontId::proportional(13.0), pal.weak);
                    return;
                }
                Err(e) => {
                    ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("The Diorama could not be made: {e}", e), FontId::proportional(13.0), pal.danger);
                    return;
                }
            }
        }
        let scene = self.scene.as_mut().expect("made above");
        scene.frame += 1;
        let ppp = ui.ctx().pixels_per_point();

        // ---- input -------------------------------------------------------
        let response = ui.interact(view, ui.scope_id().with("massif diorama"), Sense::click_and_drag());
        let center = self.center.get_or_insert(scene.middle);
        let (s, c) = turn_basis(self.turn);
        // Screen points to blocks, and back.
        let to_world = |p: egui::Pos2, center: [f64; 2], zoom: f32| {
            let d = (p - view.center()) / zoom;
            [center[0] + (c.0 * d.x + s.0 * d.y) as f64, center[1] + (c.1 * d.x + s.1 * d.y) as f64]
        };
        if response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Middle) {
            let d = response.drag_delta() / self.zoom;
            center[0] -= (c.0 * d.x + s.0 * d.y) as f64;
            center[1] -= (c.1 * d.x + s.1 * d.y) as f64;
        }
        if response.hovered() {
            let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.004).exp() * zoom_delta;
            if factor != 1.0 {
                let new = (self.zoom * factor).clamp(LEAST_ZOOM, MOST_ZOOM);
                // About the pointer: the block under it stays there.
                if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                    let before = to_world(p, *center, self.zoom);
                    let after = to_world(p, *center, new);
                    center[0] += before[0] - after[0];
                    center[1] += before[1] - after[1];
                }
                self.zoom = new;
            }
        }
        let keys = response.hovered() && !ui.ctx().egui_wants_keyboard_input();
        if keys {
            ui.input(|i| {
                if i.key_pressed(Key::Space) {
                    *center = scene.middle;
                }
                if i.key_pressed(Key::C) {
                    self.caves = !self.caves;
                }
                if i.key_pressed(Key::R) {
                    self.turn = if i.modifiers.shift { (self.turn + 3) % 4 } else { (self.turn + 1) % 4 };
                }
                let step = if i.modifiers.shift { 8 } else { 1 };
                if i.key_pressed(Key::PageUp) {
                    self.slice += step;
                    self.caves = true;
                }
                if i.key_pressed(Key::PageDown) {
                    self.slice -= step;
                    self.caves = true;
                }
            });
        }
        self.slice = self.slice.clamp(scene.min_y + 1, scene.max_y);
        let center = *center;
        let slice = self.caves.then_some(self.slice);

        // ---- tiles ---------------------------------------------------------
        // Edits since: their tiles built again from the map as it is now.
        // While a stroke goes on, every RESTART_EVERY only: each start
        // drops what the builder had under way, and takes a copy of the map.
        let due = !doc.history.is_recording() || self.restarted.is_none_or(|t| t.elapsed() >= RESTART_EVERY);
        if !self.dirty.is_empty() && due {
            profiling::scope!("Diorama: edits");
            self.restarted = Some(Instant::now());
            let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
            let mut again: Vec<TilePos> = self.dirty.drain().collect();
            let near = |p: &TilePos| {
                let (x, z) = ((p.x as f64 + 0.5) * 128.0 - center[0], (p.z as f64 + 0.5) * 128.0 - center[1]);
                x * x + z * z
            };
            again.sort_by(|a, b| near(a).total_cmp(&near(b)));
            let rest: Vec<TilePos> = scene.asked.iter().copied().filter(|t| !again.contains(t)).collect();
            let copy = {
                profiling::scope!("Diorama: copy of the map");
                doc.world.copy_unfrozen()
            };
            let started = {
                profiling::scope!("Diorama: builder start");
                Builder::start(copy, doc.dim, target, scene.materials.clone(), scene.gpu.far_size())
            };
            match started {
                Ok(builder) => {
                    builder.want(&again, false);
                    builder.want(&rest, false);
                    scene.builder = builder;
                    scene.asked.extend(again);
                    scene.whole = None;
                }
                Err(e) => log::warn!("diorama: {e}"),
            }
        }
        let views = scene.materials.top_views();
        let water = water_colours(&scene.materials);
        let took = Instant::now();
        while took.elapsed() < TAKE_BUDGET {
            let Some(built) = scene.builder.take() else { break };
            scene.asked.remove(&built.tile);
            if scene.first.is_none() {
                scene.first = Some(scene.started.elapsed());
            }
            scene.build_time += built.took as f64;
            scene.built += 1;
            let image = if scene.far_slice.is_none() { built.floors.surface.clone() } else { colours(&built.floors, scene.far_slice, scene.gpu.far_size(), &views, water.0, water.1) };
            scene.gpu.put_far(built.tile, &image);
            scene.far.insert(built.tile);
            scene.stale.remove(&built.tile);
            scene.gpu.forget(&[built.tile]);
            if let Some(old) = scene.floors.insert(built.tile, (built.floors.clone(), scene.frame)) {
                scene.bytes -= old.0.bytes();
            }
            scene.bytes += built.floors.bytes();
        }
        if scene.asked.is_empty() && scene.builder.pending() == 0 && scene.whole.is_none() && scene.far.len() >= scene.tiles.len() {
            scene.whole = Some(scene.started.elapsed());
        }

        // The small images for another height: the tiles at hand drawn
        // again, nearest first, a few milliseconds a frame.
        if scene.far_slice != slice {
            scene.far_slice = slice;
            scene.stale = scene.floors.keys().copied().collect();
        }
        if !scene.stale.is_empty() {
            let redraw = Instant::now();
            let mut list: Vec<TilePos> = scene.stale.iter().copied().collect();
            let near = |p: &TilePos| {
                let (x, z) = ((p.x as f64 + 0.5) * 128.0 - center[0], (p.z as f64 + 0.5) * 128.0 - center[1]);
                x * x + z * z
            };
            list.sort_by(|a, b| near(a).total_cmp(&near(b)));
            for batch in list.chunks(32) {
                let size = scene.gpu.far_size();
                let images: Vec<(TilePos, Vec<u8>)> = batch
                    .par_iter()
                    .filter_map(|t| {
                        let f = scene.floors.get(t)?.0.clone();
                        Some((*t, if slice.is_none() { f.surface.clone() } else { colours(&f, slice, size, &views, water.0, water.1) }))
                    })
                    .collect();
                for (t, image) in images {
                    scene.gpu.put_far(t, &image);
                }
                for t in batch {
                    scene.stale.remove(t);
                }
                if redraw.elapsed() > REDRAW_BUDGET {
                    break;
                }
            }
        }

        // The tiles in view, nearest the middle first.
        let px = [(view.width() * ppp).round().max(1.0) as u32, (view.height() * ppp).round().max(1.0) as u32];
        let blocks_per_px = 1.0 / (self.zoom * ppp);
        let corners = [view.left_top(), view.right_top(), view.left_bottom(), view.right_bottom()].map(|p| to_world(p, center, self.zoom));
        let (x0, x1) = (corners.iter().map(|c| c[0]).fold(f64::MAX, f64::min), corners.iter().map(|c| c[0]).fold(f64::MIN, f64::max));
        let (z0, z1) = (corners.iter().map(|c| c[1]).fold(f64::MAX, f64::min), corners.iter().map(|c| c[1]).fold(f64::MIN, f64::max));
        let tile = |v: f64| (v / 128.0).floor() as i32;
        let mut in_view: Vec<TilePos> = Vec::new();
        for tz in tile(z0)..=tile(z1) {
            for tx in tile(x0)..=tile(x1) {
                let t = TilePos::new(tx, tz);
                if scene.map.contains(&t) {
                    in_view.push(t);
                }
            }
        }
        let near_of = |p: &TilePos| {
            let (x, z) = ((p.x as f64 + 0.5) * 128.0 - center[0], (p.z as f64 + 0.5) * 128.0 - center[1]);
            x * x + z * z
        };
        in_view.sort_by(|a, b| near_of(a).total_cmp(&near_of(b)));
        let near = blocks_per_px <= 1.0 && in_view.len() <= DioramaGpu::near_capacity();
        let mut missing = Vec::new();
        if near {
            for t in &in_view {
                if let Some(e) = scene.floors.get_mut(t) {
                    e.1 = scene.frame;
                }
            }
            let floors = &scene.floors;
            missing = scene.gpu.put_near(&in_view, &|t| floors.get(&t).map(|e| e.0.clone()), slice);
        }
        // The tiles in view first, before the rest of the map.
        if missing != scene.last_wanted {
            if !missing.is_empty() {
                scene.builder.want(&missing, false);
                scene.asked.extend(missing.iter().copied());
            }
            scene.last_wanted = missing;
        }
        // Over the budget: the floors seen the longest ago go (their small
        // images stay; they are built again when looked at up close).
        if scene.bytes > FLOORS_BUDGET {
            let mut by_age: Vec<(TilePos, u64)> = scene.floors.iter().map(|(t, e)| (*t, e.1)).collect();
            by_age.sort_by_key(|e| e.1);
            for (t, _) in by_age {
                if scene.bytes <= FLOORS_BUDGET * 9 / 10 {
                    break;
                }
                if let Some((f, _)) = scene.floors.remove(&t) {
                    scene.bytes -= f.bytes();
                }
            }
        }

        // ---- drawing -------------------------------------------------------
        scene.gpu.sync_materials(&scene.materials);
        scene.gpu.resize(px[0], px[1]);
        let bg = pal.canvas;
        let shown = View { center, blocks_per_px, turn: self.turn, slice, background: [bg.r(), bg.g(), bg.b()], relief: self.relief };
        if let Some(cmd) = scene.gpu.render(&shown, near) {
            queue.submit([cmd]);
        }
        if scene.gpu.size() != Some(self.texture_size) || self.texture.is_none() {
            let image = scene.gpu.view().expect("made by resize");
            let mut egui_renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(device, image, wgpu::FilterMode::Nearest, id),
                None => self.texture = Some(egui_renderer.register_native_texture(device, image, wgpu::FilterMode::Nearest)),
            }
            self.texture_size = scene.gpu.size().unwrap_or(px);
        }
        if let Some(id) = self.texture {
            ui.painter().image(id, view, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        let building = scene.builder.pending() > 0 || !scene.asked.is_empty() || !scene.stale.is_empty();
        if building {
            ui.ctx().request_repaint_after(Duration::from_millis(if theme::reduce_motion(ui.ctx()) { 250 } else { 16 }));
        }

        // ---- what is under the pointer ---------------------------------------
        self.hover = response.hover_pos().map(|p| {
            let w = to_world(p, center, self.zoom);
            let (x, z) = (w[0].floor() as i32, w[1].floor() as i32);
            let t = TilePos::new(x.div_euclid(128), z.div_euclid(128));
            let y = scene.floors.get(&t).and_then(|(f, _)| f.seen((z.rem_euclid(128) * 128 + x.rem_euclid(128)) as usize, slice)).map(|f| f.y as i32 + 1);
            (x, y, z)
        });

        // ---- strips over the view ---------------------------------------------
        let chunks = in_view.iter().filter(|t| scene.far.contains(t)).count() * 64;
        let top = Rect::from_min_size(view.min, vec2(view.width(), STRIP));
        let painter = ui.painter_at(view);
        let strip = if pal.high { pal.panel } else { pal.panel.gamma_multiply(0.86) };
        painter.rect_filled(top, 0.0, strip);
        painter.line_segment([top.left_bottom(), top.right_bottom()], egui::Stroke::new(1.0, pal.border));
        let mode = match slice {
            None => tr!("Surface").to_owned(),
            Some(y) => tr!("Caves · under Y {y}", y),
        };
        let at = match self.hover {
            Some((x, Some(y), z)) => format!("{x}, {y}, {z}"),
            Some((x, None, z)) => format!("{x}, –, {z}"),
            None => "–".to_owned(),
        };
        let facing = [tr!("North up"), tr!("East up"), tr!("South up"), tr!("West up")][self.turn as usize % 4];
        let mut text = format!("{mode}   ·   {at}   ·   {}   ·   {facing}", trn!("{n} chunk", "{n} chunks", n = chunks));
        if building {
            let p = Self::progress(self);
            text.push_str(&format!("   ·   {}", tr!("Building {done} of {all} tiles", done = p.built.min(p.tiles), all = p.tiles)));
        }
        painter.text(top.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(12.0), pal.text);
        // Settings, at the right of the strip.
        let button = Rect::from_min_size(pos2(top.right() - 30.0, top.top() + 2.0), vec2(26.0, STRIP - 4.0));
        let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(button).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let open = sui.button(theme::glyph(Icon::Settings2).to_string()).on_hover_text(tr!("Diorama settings"));
        egui::Popup::from_toggle_button_response(&open).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(260.0);
            widgets::well(ui, |ui| {
                widgets::row(ui, tr!("Show"), |ui| {
                    let labels = [tr!("Surface"), tr!("Caves")];
                    if let Some(k) = widgets::button_group(ui, "diorama mode settings", &labels, &[!self.caves, self.caves]) {
                        self.caves = k == 1;
                    }
                });
                widgets::row(ui, tr!("Caves under"), |ui| {
                    if let Some((lo, hi)) = self.scene.as_ref().map(|s| (s.min_y + 1, s.max_y)) {
                        ui.add(egui::Slider::new(&mut self.slice, lo..=hi).suffix(" Y"));
                    }
                });
                widgets::row(ui, tr!("Up is"), |ui| {
                    let labels = ["N", "E", "S", "W"];
                    let on: Vec<bool> = (0..4).map(|k| self.turn == k).collect();
                    if let Some(k) = widgets::button_group(ui, "diorama turn", &labels, &on) {
                        self.turn = k as u8;
                    }
                });
                widgets::row(ui, tr!("Relief"), |ui| {
                    ui.add(egui::Slider::new(&mut self.relief, 0.0..=1.0).show_value(false));
                });
            });
        });
        let bottom = Rect::from_min_max(pos2(view.left(), view.bottom() - STRIP), view.max);
        painter.rect_filled(bottom, 0.0, strip);
        painter.line_segment([bottom.left_top(), bottom.right_top()], egui::Stroke::new(1.0, pal.border));
        let hint = tr!("Drag pan · Scroll zoom · C caves · PgUp/PgDn slice · R turn · Space recenter");
        painter.text(bottom.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, hint, FontId::proportional(11.5), pal.weak);
        // North, where it is.
        let compass = pos2(view.right() - 22.0, bottom.top() - 22.0);
        let dir = [vec2(0.0, -1.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 0.0)][self.turn as usize % 4];
        painter.circle(compass, 13.0, strip, egui::Stroke::new(1.0, pal.border));
        painter.text(compass + dir * 6.5, Align2::CENTER_CENTER, "N", FontId::proportional(11.0), pal.accent);
        self.frame_time = started.elapsed();
    }
}

/// The screen's axes as the world's, for `turn` quarter turns: how a step
/// right (`c`) and down (`s`) on screen move in x and z.
fn turn_basis(turn: u8) -> ((f32, f32), (f32, f32)) {
    // (down → world, right → world)
    match turn % 4 {
        0 => ((0.0, 1.0), (1.0, 0.0)),
        1 => ((-1.0, 0.0), (0.0, 1.0)),
        2 => ((0.0, -1.0), (-1.0, 0.0)),
        _ => ((1.0, 0.0), (0.0, -1.0)),
    }
}

/// The mean colours of water and lava from above.
fn water_colours(m: &Materials) -> ([u8; 4], [u8; 4]) {
    let view = |name: &str| m.top_view(m.id(name, &[("level", "0")])).mean;
    (view("minecraft:water"), view("minecraft:lava"))
}

fn make_scene(device: &wgpu::Device, queue: &wgpu::Queue, doc: &mut Document, catalogue: Option<Arc<massif_assets::Catalogue>>, cat_key: Option<usize>) -> Result<Option<Scene>, String> {
    let Some(d) = doc.world.dims.get(doc.dim) else { return Ok(None) };
    let tiles: Vec<TilePos> = d.ground().positions().collect();
    let Some(first) = tiles.first() else { return Ok(None) };
    let (mut lo, mut hi) = (*first, *first);
    for t in &tiles {
        lo = TilePos::new(lo.x.min(t.x), lo.z.min(t.z));
        hi = TilePos::new(hi.x.max(t.x), hi.z.max(t.z));
    }
    let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
    let materials = Arc::new(Materials::new(catalogue, device.limits().max_texture_array_layers as usize));
    // The probes of the shader, first: water and lava.
    let _ = (materials.id("minecraft:water", &[("level", "0")]), materials.id("minecraft:lava", &[("level", "0")]));
    let builder = Builder::start(doc.world.copy_unfrozen(), doc.dim, target, materials.clone(), far_layout(tiles.len()).0 as usize)?;
    let gpu = DioramaGpu::new(device, queue, lo, hi, tiles.len(), builder.min_y);
    let middle = [((lo.x + hi.x + 1) as f64) * 64.0, ((lo.z + hi.z + 1) as f64) * 64.0];
    let mut order = tiles.clone();
    let key = |p: &TilePos| {
        let (x, z) = ((p.x as f64 + 0.5) * 128.0 - middle[0], (p.z as f64 + 0.5) * 128.0 - middle[1]);
        x * x + z * z
    };
    order.sort_by(|a, b| key(a).total_cmp(&key(b)));
    builder.want(&order, true);
    let (min_y, max_y) = (builder.min_y, builder.max_y);
    Ok(Some(Scene {
        gpu,
        builder,
        materials,
        floors: HashMap::new(),
        bytes: 0,
        asked: order.iter().copied().collect(),
        map: tiles.iter().copied().collect(),
        tiles,
        far: HashSet::new(),
        stale: HashSet::new(),
        far_slice: None,
        generation: doc.generation,
        catalogue: cat_key,
        dim: doc.dim,
        started: Instant::now(),
        first: None,
        whole: None,
        build_time: 0.0,
        built: 0,
        middle,
        min_y,
        max_y,
        last_wanted: Vec::new(),
        frame: 0,
    }))
}
