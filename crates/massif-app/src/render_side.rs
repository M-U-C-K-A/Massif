//! The Render side: the open map path-traced on the graphics card
//! (`massif_render`), a sample a pixel a frame, added up while the view
//! stays still; the sun, the sky and the exposure beside it.
//!
//! The map's chunks are built in the background by the exporter, nearest
//! the view first, and put on the graphics card as they come, so the
//! image fills in from the middle out; an edit made on the World side
//! builds its tiles again when the Render side is shown next.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, row, well};
use egui::{Align2, Color32, FontId, PointerButton, Rect, RichText, Sense, Ui, pos2, vec2};
use massif_core::coords::TilePos;
use massif_render::map_source::{Extent, MapBuilder, target_for, tiles_by_distance};
use massif_render::{Camera, Light, Materials, Renderer};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Width of the column of settings.
const SIDEBAR: f32 = 280.0;
/// Time a frame spends putting built chunks on the graphics card.
const UPLOAD_BUDGET: Duration = Duration::from_millis(6);
/// Seconds after the view last moved that it is drawn sharp again.
const STILL_AFTER: f64 = 0.2;
/// Samples a pixel the image stops at.
const MOST_SAMPLES: u32 = 2048;
const FOV: f32 = 50.0 * std::f32::consts::PI / 180.0;

/// The camera of the Render side: turning about a point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Orbit {
    target: [f64; 3],
    yaw: f32,
    pitch: f32,
    distance: f64,
}

impl Orbit {
    fn camera(&self) -> Camera {
        Camera::orbit(self.target, self.yaw, self.pitch, self.distance, FOV)
    }
}

/// What the renderer holds of the open map.
struct Scene {
    renderer: Renderer,
    materials: Arc<Materials>,
    builder: Option<MapBuilder>,
    /// Tiles on the graphics card, and all of the map's.
    built: HashSet<TilePos>,
    tiles: usize,
    started: Instant,
    /// How long the whole map took to build, once built.
    took: Option<Duration>,
    /// The map is larger than the graphics card's buffers.
    full: bool,
    generation: u64,
    catalogue: Option<usize>,
    dim: usize,
}

pub struct RenderSide {
    scene: Option<Scene>,
    orbit: Option<Orbit>,
    light: Light,
    texture: Option<egui::TextureId>,
    /// Tiles edited since they were built.
    dirty: HashSet<TilePos>,
    moving_until: f64,
    last: Option<(Orbit, Light, [u32; 2])>,
    /// When the image last started adding samples up, and its samples
    /// then: how many it adds a second.
    since: Option<(Instant, u32)>,
}

impl Default for RenderSide {
    fn default() -> Self {
        Self { scene: None, orbit: None, light: Light::default(), texture: None, dirty: HashSet::new(), moving_until: 0.0, last: None, since: None }
    }
}

impl RenderSide {
    /// Whether the map's chunks are being built.
    #[cfg(test)]
    pub fn building(&self) -> bool {
        self.scene.as_ref().is_none_or(|s| s.builder.is_some())
    }

    /// Samples a pixel added up in the image.
    #[cfg(test)]
    pub fn samples(&self) -> u32 {
        self.scene.as_ref().map_or(0, |s| s.renderer.samples)
    }

    /// Tiles the map changed in: built again when the side is shown.
    pub fn invalidate(&mut self, tiles: &[TilePos]) {
        if self.scene.is_some() {
            self.dirty.extend(tiles.iter().copied());
        }
    }

    /// The Render side, over the whole of the window under its bars.
    pub fn ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, doc: &mut Document, catalogue: Option<Arc<massif_assets::Catalogue>>) {
        let pal = Palette::of(ui);
        let whole = ui.available_rect_before_wrap();
        let view = Rect::from_min_max(whole.min, pos2((whole.right() - SIDEBAR - 8.0).max(whole.left() + 100.0), whole.bottom()));
        let side = Rect::from_min_max(pos2(view.right() + 8.0, whole.top()), whole.max);
        ui.painter().rect_filled(view, theme::CARD_RADIUS, pal.canvas);
        let Some(render_state) = frame.wgpu_render_state() else {
            ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("Rendering needs the wgpu renderer"), FontId::proportional(13.0), pal.weak);
            return;
        };
        let (device, queue) = (&render_state.device, &render_state.queue);
        let cat_key = catalogue.as_ref().map(|c| Arc::as_ptr(c) as usize);
        // A new map (or the game's textures read since): a new scene.
        if self.scene.as_ref().is_some_and(|s| s.generation != doc.generation || s.catalogue != cat_key || s.dim != doc.dim) {
            self.scene = None;
            self.orbit = None;
        }
        if self.scene.is_none() {
            let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
            let Some(extent) = Extent::of_map(&doc.world, doc.dim, &target) else {
                ui.painter().text(view.center(), Align2::CENTER_CENTER, tr!("Nothing to render: the map is empty"), FontId::proportional(13.0), pal.weak);
                return;
            };
            let orbit = *self.orbit.get_or_insert_with(|| framing(&extent, doc.world.meta.sea_level as f64));
            let materials = Arc::new(Materials::new(catalogue.clone(), device.limits().max_texture_array_layers as usize));
            let tiles = tiles_by_distance(&doc.world, doc.dim, orbit.target[0], orbit.target[2]);
            let count = tiles.len();
            let builder = MapBuilder::start(doc.world.snapshot(), doc.dim, target, materials.clone(), tiles);
            self.scene = Some(Scene {
                renderer: Renderer::new(device, queue, extent),
                materials,
                builder: Some(builder),
                built: HashSet::new(),
                tiles: count,
                started: Instant::now(),
                took: None,
                full: false,
                generation: doc.generation,
                catalogue: cat_key,
                dim: doc.dim,
            });
            self.dirty.clear();
            self.last = None;
        }
        let scene = self.scene.as_mut().expect("made above");
        let orbit = self.orbit.get_or_insert_with(|| framing(&scene.renderer.extent(), doc.world.meta.sea_level as f64));

        // Edits since: their tiles first, then those not built yet, from a
        // snapshot of the map as it is now.
        if !self.dirty.is_empty() {
            let mut first: Vec<TilePos> = self.dirty.drain().collect();
            let near = |p: &TilePos| {
                let (x, z) = ((p.x as f64 + 0.5) * 128.0 - orbit.target[0], (p.z as f64 + 0.5) * 128.0 - orbit.target[2]);
                x * x + z * z
            };
            first.sort_by(|a, b| near(a).total_cmp(&near(b)));
            let again: HashSet<TilePos> = first.iter().copied().collect();
            let rest: Vec<TilePos> = tiles_by_distance(&doc.world, doc.dim, orbit.target[0], orbit.target[2]).into_iter().filter(|p| !scene.built.contains(p) && !again.contains(p)).collect();
            for p in &first {
                scene.built.remove(p);
            }
            first.extend(rest);
            let target = target_for(catalogue.as_ref().map(|c| c.version.as_str()));
            scene.builder = Some(MapBuilder::start(doc.world.snapshot(), doc.dim, target, scene.materials.clone(), first));
            scene.took = None;
            scene.started = Instant::now();
        }

        // Built chunks onto the graphics card, a few milliseconds' worth.
        let started = Instant::now();
        let mut changed = false;
        if let Some(builder) = &mut scene.builder {
            while started.elapsed() < UPLOAD_BUDGET {
                let Some(built) = builder.next() else { break };
                scene.built.insert(built.tile);
                if !scene.renderer.put(built.chunks) {
                    scene.full = true;
                }
                changed = true;
            }
            if builder.finished() {
                if let Some(e) = builder.error() {
                    log::warn!("render scene: {e}");
                }
                scene.builder = None;
                scene.took = Some(scene.started.elapsed());
            }
            ui.ctx().request_repaint();
        }
        scene.renderer.sync_materials(&scene.materials);

        // The camera: drag to turn about the point looked at, right-drag
        // (or middle) to slide over the land, the wheel to come closer.
        let response = ui.interact(view, egui::Id::new("massif render view"), Sense::click_and_drag());
        let now = ui.input(|i| i.time);
        let extent = scene.renderer.extent();
        let span = extent.width.max(extent.depth) as f64 * 16.0;
        if response.dragged_by(PointerButton::Primary) {
            let d = response.drag_delta();
            orbit.yaw += d.x * 0.006;
            orbit.pitch = (orbit.pitch - d.y * 0.006).clamp(-1.55, 1.2);
        }
        if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
            let d = response.drag_delta();
            let k = orbit.distance * 0.0016;
            let (s, c) = (orbit.yaw.sin() as f64, orbit.yaw.cos() as f64);
            orbit.target[0] -= (c * d.x as f64 - s * d.y as f64) * k;
            orbit.target[2] -= (s * d.x as f64 + c * d.y as f64) * k;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
            if scroll != 0.0 {
                orbit.distance = (orbit.distance * (-scroll as f64 * 0.003).exp()).clamp(3.0, span * 3.0 + 2000.0);
            }
        }
        if response.double_clicked() {
            *orbit = framing(&extent, doc.world.meta.sea_level as f64);
        }

        // The image: as large as the view, in pixels.
        let ppp = ui.ctx().pixels_per_point();
        let px = [(view.width() * ppp).round().max(1.0) as u32, (view.height() * ppp).round().max(1.0) as u32];
        let had = scene.renderer.view().is_some();
        scene.renderer.resize(px[0], px[1]);
        if !had || self.last.is_none_or(|l| l.2 != px) {
            let image = scene.renderer.view().expect("made by resize");
            let mut egui_renderer = render_state.renderer.write();
            match self.texture {
                Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(device, image, wgpu::FilterMode::Linear, id),
                None => self.texture = Some(egui_renderer.register_native_texture(device, image, wgpu::FilterMode::Linear)),
            }
        }
        let shown = (*orbit, self.light, px);
        if self.last.is_some_and(|l| l.0 != shown.0) {
            self.moving_until = now + STILL_AFTER;
        }
        if self.last.is_some_and(|l| l.1 != shown.1) || changed {
            scene.renderer.restart();
        }
        self.last = Some(shown);
        let moving = now < self.moving_until;
        // With less motion (Settings ▸ Accessibility), the image settles
        // sooner and stops there.
        let most = if theme::reduce_motion(ui.ctx()) { 64 } else { MOST_SAMPLES };
        if moving || scene.renderer.samples < most {
            if let Some(cmd) = scene.renderer.render(&orbit.camera(), &self.light, moving) {
                queue.submit([cmd]);
            }
            ui.ctx().request_repaint();
        }
        let samples = scene.renderer.samples;
        if samples <= 1 || self.since.is_some_and(|(_, n)| samples < n) {
            self.since = (samples == 1).then(|| (Instant::now(), 1));
        }
        if let Some(id) = self.texture {
            ui.painter().image(id, view, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        // What it is at, over the bottom left of the image.
        let status = match (&scene.builder, scene.full) {
            (_, true) => tr!("The map is larger than the graphics card holds: part of it is missing").to_owned(),
            (Some(b), _) => {
                let (done, all) = b.progress();
                tr!("Building the scene: {done} of {all} tiles", done, all)
            }
            (None, _) => trn!("{n} sample a pixel", "{n} samples a pixel", n = scene.renderer.samples),
        };
        let at = view.left_bottom() + vec2(theme::PAD, -theme::PAD);
        let galley = ui.painter().layout_no_wrap(status, FontId::proportional(12.0), pal.text);
        let back = Rect::from_min_size(at - vec2(0.0, galley.size().y + 8.0), galley.size() + vec2(16.0, 8.0));
        ui.painter().rect(back, theme::PILL, pal.panel, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        ui.painter().galley(back.min + vec2(8.0, 4.0), galley, pal.text);

        // The settings beside.
        let mut col = ui.new_child(egui::UiBuilder::new().max_rect(side).layout(egui::Layout::top_down(egui::Align::Min)));
        col.spacing_mut().item_spacing.y = 6.0;
        col.horizontal(|ui| {
            ui.label(RichText::new(theme::with_icon(Icon::Aperture, tr!("Render"))).size(14.0).medium());
            ui.label(RichText::new(tr!("Beta")).size(11.0).strong().color(pal.accent));
        });
        col.add_space(2.0);
        let light = &mut self.light;
        let mut azimuth = light.sun_azimuth.to_degrees();
        let mut elevation = light.sun_elevation.to_degrees();
        col.label(RichText::new(tr!("Sun")).size(12.0).color(pal.weak));
        well(&mut col, |ui| {
            row(ui, tr!("Direction"), |ui| {
                ui.add(Field::new(&mut azimuth, 0.0..=360.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Height"), |ui| {
                ui.add(Field::new(&mut elevation, -10.0..=90.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Strength"), |ui| {
                ui.add(Field::new(&mut light.sun_strength, 0.0..=20.0).max_decimals(1));
            });
        });
        light.sun_azimuth = azimuth.to_radians();
        light.sun_elevation = elevation.to_radians();
        col.label(RichText::new(tr!("Image")).size(12.0).color(pal.weak));
        well(&mut col, |ui| {
            row(ui, tr!("Sky"), |ui| {
                ui.add(Field::new(&mut light.sky_strength, 0.0..=4.0).max_decimals(2));
            });
            row(ui, tr!("Bounces"), |ui| {
                ui.add(Field::new(&mut light.bounces, 0..=6));
            });
            row(ui, tr!("Exposure"), |ui| {
                ui.add(Field::new(&mut light.exposure, 0.1..=8.0).logarithmic(true).max_decimals(2));
            });
        });
        col.label(RichText::new(tr!("Scene")).size(12.0).color(pal.weak));
        let memory = scene.renderer.memory();
        let mb = |b: u64| format!("{:.0} MB", b as f64 / (1 << 20) as f64);
        well(&mut col, |ui| {
            row(ui, tr!("Tiles"), |ui| {
                ui.label(format!("{} / {}", scene.built.len(), scene.tiles));
            });
            row(ui, tr!("Chunks"), |ui| {
                ui.label(scene.renderer.chunks().to_string());
            });
            if let Some(took) = scene.took {
                row(ui, tr!("Built in"), |ui| {
                    ui.label(format!("{:.1} s", took.as_secs_f64()));
                });
            }
            row(ui, tr!("Graphics memory"), |ui| {
                ui.label(mb(memory.total()));
            });
            row(ui, tr!("Samples a second"), |ui| {
                let rate = self.since.filter(|_| samples > 4 && samples < most).map(|(t, n)| (samples - n) as f64 / t.elapsed().as_secs_f64().max(1e-3));
                ui.label(rate.map_or_else(|| "—".to_owned(), |r| format!("{r:.0}")));
            });
        });
        col.add_space(4.0);
        col.label(RichText::new(tr!("Drag: turn · right-drag: slide · wheel: closer · double-click: the whole map")).size(11.0).color(pal.weak));
    }
}

/// The whole map, seen from above its south-east, a little high.
fn framing(extent: &Extent, sea: f64) -> Orbit {
    let c = extent.center();
    let span = extent.width.max(extent.depth) as f64 * 16.0;
    Orbit { target: [c[0], sea, c[2]], yaw: 0.65, pitch: -0.55, distance: span * 0.8 }
}
