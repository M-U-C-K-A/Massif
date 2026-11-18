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

/// The time and place the sun is put by.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Time {
    /// Local solar time, hours.
    hour: f32,
    /// Day of the year.
    day: f32,
    /// Degrees, north positive.
    latitude: f32,
    /// How far the map's north is turned from the world's, degrees.
    north: f32,
}

impl Default for Time {
    fn default() -> Self {
        Self { hour: 15.0, day: 172.0, latitude: 45.0, north: 0.0 }
    }
}

/// The ready-made looks of the sky, by name.
const PRESETS: [&str; 6] = [tn!("Clear noon"), tn!("Fair weather"), tn!("Golden hour"), tn!("Blue hour"), tn!("Foggy morning"), tn!("Night")];

/// Put a ready-made look of the sky on `light`, its sun by `time` (the fog
/// and clouds from the sea level `sea`).
fn preset(k: usize, light: &mut Light, time: &mut Time, sea: f32) {
    let base = Light { fog_height: sea + 8.0, cloud_height: sea + 1200.0, aperture: light.aperture, focus: light.focus, bounces: light.bounces, ..Light::default() };
    let (hour, l) = match k {
        0 => (12.5, base),
        1 => (14.5, Light { cloud_cover: 0.42, ..base }),
        2 => (18.8, Light { haze: 4.0, cloud_cover: 0.2, exposure: 2.5, ..base }),
        3 => (20.25, Light { haze: 2.0, exposure: 7.0, ..base }),
        4 => (7.0, Light { fog: 0.02, fog_falloff: 16.0, haze: 2.0, exposure: 1.4, ..base }),
        _ => (23.0, Light { exposure: 6.0, moon: 1.0 / 150.0, ..base }),
    };
    *light = l;
    time.hour = hour;
    light.set_time(time.hour, time.day, time.latitude, time.north);
}

/// The camera of the Render side: turning about a point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Orbit {
    target: [f64; 3],
    yaw: f32,
    pitch: f32,
    distance: f64,
}

impl Orbit {
    fn camera(&self, fov: f32) -> Camera {
        Camera::orbit(self.target, self.yaw, self.pitch, self.distance, fov.to_radians())
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
    time: Time,
    /// The vertical field of view, degrees.
    fov: f32,
    preset: usize,
    texture: Option<egui::TextureId>,
    /// Tiles edited since they were built.
    dirty: HashSet<TilePos>,
    moving_until: f64,
    last: Option<(Orbit, Light, [u32; 2], f32)>,
    /// When the image last started adding samples up, and its samples
    /// then: how many it adds a second.
    since: Option<(Instant, u32)>,
}

impl Default for RenderSide {
    fn default() -> Self {
        let mut light = Light::default();
        let time = Time::default();
        light.set_time(time.hour, time.day, time.latitude, time.north);
        Self { scene: None, orbit: None, light, time, fov: 50.0, preset: usize::MAX, texture: None, dirty: HashSet::new(), moving_until: 0.0, last: None, since: None }
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
            // The fog and the clouds where they belong on this map.
            let sea = doc.world.meta.sea_level as f32;
            self.light.fog_height = sea + 8.0;
            self.light.cloud_height = sea + 1200.0;
        }
        let scene = self.scene.as_mut().expect("made above");
        scene.renderer.sea_level = doc.world.meta.sea_level as f64;
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
                let Some(built) = builder.take() else { break };
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
        let response = ui.interact(view, egui::Id::unique("massif render view"), Sense::click_and_drag());
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
        // Shift-click: in focus there.
        if response.clicked()
            && ui.input(|i| i.modifiers.shift)
            && let Some(at) = response.interact_pointer_pos()
        {
            let camera = orbit.camera(self.fov);
            let b = camera.basis();
            let tan = (camera.fov_y * 0.5).tan();
            let ndc = [((at.x - view.left()) / view.width()) * 2.0 - 1.0, ((at.y - view.top()) / view.height()) * 2.0 - 1.0];
            let dir = b.forward + b.right * (ndc[0] * tan * view.width() / view.height()) - b.up * (ndc[1] * tan);
            let d = dir.normalize();
            if let Some(t) = scene.renderer.pick(camera.eye, [d.x as f64, d.y as f64, d.z as f64], 50000.0) {
                self.light.focus = (t * d.dot(b.forward) as f64) as f32;
                if self.light.aperture == 0.0 {
                    self.light.aperture = 0.4;
                }
            }
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
        let shown = (*orbit, self.light, px, self.fov);
        if self.last.is_some_and(|l| l.0 != shown.0 || l.3 != shown.3) {
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
            if let Some(cmd) = scene.renderer.render(&orbit.camera(self.fov), &self.light, moving) {
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

        // The settings beside, in a column that scrolls.
        let mut outer = ui.new_child(egui::UiBuilder::new().max_rect(side).layout(egui::Layout::top_down(egui::Align::Min)));
        outer.horizontal(|ui| {
            ui.label(RichText::new(theme::with_icon(Icon::Aperture, tr!("Render"))).size(14.0).medium());
            ui.label(RichText::new(tr!("Beta")).size(11.0).strong().color(pal.accent));
        });
        outer.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut outer, |col| {
        col.spacing_mut().item_spacing.y = 6.0;
        let sea = doc.world.meta.sea_level as f32;
        let (light, time) = (&mut self.light, &mut self.time);
        let mut picked = None;
        col.label(RichText::new(tr!("Sun and sky")).size(12.0).color(pal.weak));
        let before = *time;
        well(col, |ui| {
            row(ui, tr!("Look"), |ui| {
                let name = PRESETS.get(self.preset).map_or(tr!("Custom"), |n| crate::i18n::tr_name(n));
                crate::widgets::combo("render preset").selected_text(name).show_ui(ui, |ui| {
                    for (k, n) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(self.preset == k, crate::i18n::tr_name(n)).clicked() {
                            picked = Some(k);
                        }
                    }
                });
            });
            row(ui, tr!("Time"), |ui| {
                ui.add(Field::new(&mut time.hour, 0.0..=24.0).suffix(" h").max_decimals(2));
            });
            row(ui, tr!("Day"), |ui| {
                ui.add(Field::new(&mut time.day, 1.0..=365.0).max_decimals(0));
            });
            row(ui, tr!("Latitude"), |ui| {
                ui.add(Field::new(&mut time.latitude, -66.0..=66.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("North"), |ui| {
                ui.add(Field::new(&mut time.north, 0.0..=360.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Sun"), |ui| {
                ui.add(Field::new(&mut light.sun_strength, 0.0..=20.0).max_decimals(1));
            });
            row(ui, tr!("Sky"), |ui| {
                ui.add(Field::new(&mut light.sky_strength, 0.0..=4.0).max_decimals(2));
            });
        });
        if *time != before {
            light.set_time(time.hour, time.day, time.latitude, time.north);
        }
        col.label(RichText::new(tr!("Atmosphere")).size(12.0).color(pal.weak));
        well(col, |ui| {
            row(ui, tr!("Air"), |ui| {
                ui.add(Field::new(&mut light.air, 0.0..=4.0).max_decimals(2));
            });
            row(ui, tr!("Haze"), |ui| {
                ui.add(Field::new(&mut light.haze, 0.0..=30.0).logarithmic(false).max_decimals(1));
            });
            row(ui, tr!("Fog"), |ui| {
                ui.add(Field::new(&mut light.fog, 0.0..=0.1).max_decimals(3));
            });
            row(ui, tr!("Fog height"), |ui| {
                ui.add(Field::new(&mut light.fog_height, sea - 64.0..=sea + 256.0).max_decimals(0));
            });
            row(ui, tr!("Fog fade"), |ui| {
                ui.add(Field::new(&mut light.fog_falloff, 2.0..=200.0).suffix(" blocks").max_decimals(0));
            });
            row(ui, tr!("Shafts of light"), |ui| {
                crate::widgets::switch(ui, &mut light.shafts);
            });
            row(ui, tr!("Sea around the map"), |ui| {
                crate::widgets::switch(ui, &mut light.sea);
            });
        });
        col.label(RichText::new(tr!("Clouds")).size(12.0).color(pal.weak));
        well(col, |ui| {
            row(ui, tr!("Cover"), |ui| {
                ui.add(Field::new(&mut light.cloud_cover, 0.0..=1.0).max_decimals(2));
            });
            row(ui, tr!("Height"), |ui| {
                ui.add(Field::new(&mut light.cloud_height, sea..=sea + 4000.0).max_decimals(0));
            });
            row(ui, tr!("Thickness"), |ui| {
                ui.add(Field::new(&mut light.cloud_thickness, 50.0..=2000.0).max_decimals(0));
            });
            row(ui, tr!("Density"), |ui| {
                ui.add(Field::new(&mut light.cloud_density, 0.002..=0.1).logarithmic(true).max_decimals(3));
            });
            row(ui, tr!("Size"), |ui| {
                ui.add(Field::new(&mut light.cloud_size, 200.0..=6000.0).logarithmic(true).max_decimals(0));
            });
            row(ui, tr!("Wind"), |ui| {
                ui.add(Field::new(&mut light.wind[0], 0.0..=20000.0).max_decimals(0));
            });
        });
        col.label(RichText::new(tr!("Lens")).size(12.0).color(pal.weak));
        well(col, |ui| {
            row(ui, tr!("Angle"), |ui| {
                ui.add(Field::new(&mut self.fov, 10.0..=100.0).suffix("°").max_decimals(0));
            });
            row(ui, tr!("Aperture"), |ui| {
                ui.add(Field::new(&mut light.aperture, 0.0..=4.0).suffix(" blocks").max_decimals(2));
            });
            row(ui, tr!("Focus"), |ui| {
                ui.add(Field::new(&mut light.focus, 1.0..=5000.0).logarithmic(true).suffix(" blocks").max_decimals(0));
            });
        });
        col.label(RichText::new(tr!("Image")).size(12.0).color(pal.weak));
        well(col, |ui| {
            row(ui, tr!("Glow"), |ui| {
                ui.add(Field::new(&mut light.emission, 0.0..=8.0).max_decimals(2));
            });
            row(ui, tr!("Bounces"), |ui| {
                ui.add(Field::new(&mut light.bounces, 0..=6));
            });
            row(ui, tr!("Exposure"), |ui| {
                ui.add(Field::new(&mut light.exposure, 0.1..=16.0).logarithmic(true).max_decimals(2));
            });
        });
        if let Some(k) = picked {
            self.preset = k;
            preset(k, light, time, sea);
        }
        col.label(RichText::new(tr!("Scene")).size(12.0).color(pal.weak));
        let memory = scene.renderer.memory();
        let mb = |b: u64| format!("{:.0} MB", b as f64 / (1 << 20) as f64);
        well(col, |ui| {
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
        col.label(RichText::new(tr!("Drag: turn · right-drag: slide · wheel: closer · shift-click: focus there · double-click: the whole map")).size(11.0).color(pal.weak));
        });
    }
}

/// The whole map, seen from above its south-east, a little high.
fn framing(extent: &Extent, sea: f64) -> Orbit {
    let c = extent.center();
    let span = extent.width.max(extent.depth) as f64 * 16.0;
    Orbit { target: [c[0], sea, c[2]], yaw: 0.65, pitch: -0.55, distance: span * 0.8 }
}
