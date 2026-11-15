//! Edit ▸ Whole map: the graph's filters run over the whole map — erosion,
//! thermal erosion, smoothing, terraces, lakes, rivers… — each in a dialog
//! with its settings and the map before and after, then in the background
//! as one step to undo.

use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui, vec2};
use massif_core::graph::{Extent, Field, NodeKind, Reshape, WHOLE_MAX, reshape_preview};
use massif_core::registry::Registry;
use massif_core::world::Dimension;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// What the Edit menu offers to do to the whole map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeOp {
    Erosion,
    Weathering,
    Smooth,
    Sharpen,
    Terraces,
    Strata,
    Lakes,
    Rivers,
    Relief,
}

impl WholeOp {
    pub const ALL: [WholeOp; 9] = [
        WholeOp::Erosion,
        WholeOp::Weathering,
        WholeOp::Rivers,
        WholeOp::Lakes,
        WholeOp::Smooth,
        WholeOp::Sharpen,
        WholeOp::Terraces,
        WholeOp::Strata,
        WholeOp::Relief,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WholeOp::Erosion => "Erosion",
            WholeOp::Weathering => "Thermal erosion",
            WholeOp::Smooth => "Smooth",
            WholeOp::Sharpen => "Sharpen",
            WholeOp::Terraces => "Terraces",
            WholeOp::Strata => "Strata",
            WholeOp::Lakes => "Lakes",
            WholeOp::Rivers => "Rivers",
            WholeOp::Relief => "Scale and raise",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            WholeOp::Erosion => Icon::CloudRain,
            WholeOp::Weathering => Icon::Mountain,
            WholeOp::Smooth => Icon::Spline,
            WholeOp::Sharpen => Icon::MountainSnow,
            WholeOp::Terraces => Icon::ChartNoAxesColumnIncreasing,
            WholeOp::Strata => Icon::Layers,
            WholeOp::Lakes => Icon::Droplets,
            WholeOp::Rivers => Icon::Waves,
            WholeOp::Relief => Icon::MoveVertical,
        }
    }

    /// What it does, in a sentence.
    pub fn about(self) -> &'static str {
        match self {
            WholeOp::Erosion => "Rain runs down the slopes and carries soil from the heights to the valleys: gullies, worn flanks, fans of silt.",
            WholeOp::Weathering => "Loose rock slides down wherever a slope is steeper than it can hold: scree at the feet of cliffs, softer ridges.",
            WholeOp::Smooth => "The relief softened: noise and small steps gone, the forms kept.",
            WholeOp::Sharpen => "The relief's detail brought out: ridges and hollows stand out more.",
            WholeOp::Terraces => "Flat steps with steep risers, over the whole height of the map.",
            WholeOp::Strata => "Layers of rock of uneven thickness: ledges along the slopes.",
            WholeOp::Lakes => "Water fills the hollows up to where they spill — or their ground rises, with Fill.",
            WholeOp::Rivers => "Rivers traced where the water of the whole map runs, dug into the valleys, with their lakes; their beds in gravel.",
            WholeOp::Relief => "The heights scaled around the sea level, then moved up or down.",
        }
    }

    /// Its settings to start from, for a map `side` blocks wide whose sea
    /// is at `sea`.
    pub fn start(self, sea: f32, side: f32) -> Reshape {
        // Features grow with the map, up to a point (as the templates do).
        let f = (side / 2048.0).clamp(0.5, 3.0);
        Reshape::Filter(match self {
            WholeOp::Erosion => NodeKind::Erosion {
                rain: 1.0,
                erosion: 0.3,
                deposition: 0.3,
                capacity: 4.0,
                evaporation: 0.02,
                radius: 4.0,
                seed: 5,
            },
            WholeOp::Weathering => NodeKind::Thermal {
                talus: 40.0,
                iterations: 40,
                strength: 0.5,
            },
            WholeOp::Smooth => NodeKind::Blur { radius: 3.0 },
            WholeOp::Sharpen => NodeKind::Sharpen { radius: 24.0, amount: 0.5 },
            WholeOp::Terraces => NodeKind::Terrace { steps: 12, strength: 0.6 },
            WholeOp::Strata => NodeKind::Stratify {
                layers: 14,
                strength: 0.6,
                tilt: 6.0,
                seed: 5,
            },
            WholeOp::Lakes => NodeKind::Lakes { amount: 0.0, sea },
            WholeOp::Rivers => NodeKind::Rivers {
                threshold: 0.5,
                depth: 4.0,
                width: 10.0 * f.clamp(1.0, 2.5),
                sea,
                banks: 1.0,
                lakes: 0.5,
            },
            WholeOp::Relief => return Reshape::Relief { scale: 1.0, shift: 0.0 },
        })
    }
}

/// Samples a side of the pictures' map.
const PREVIEW_N: usize = 192;
/// Their size, in points.
const PICTURE: f32 = 196.0;
/// Maps with more tiles than this have no pictures: even a small one reads
/// every tile.
const PREVIEW_TILES: usize = 4096;

/// The map before and after, as the settings were.
#[derive(Default)]
struct Preview {
    made_for: Option<String>,
    rx: Option<Receiver<Option<(Extent, Field, Field)>>>,
    cancel: Arc<AtomicBool>,
    pictures: Option<(TextureHandle, TextureHandle)>,
    /// When the settings last changed (to wait until they settle) and to what.
    changed: Option<(f64, String)>,
}

/// The dialog of a whole-map pass.
pub struct ReshapeForm {
    pub what: WholeOp,
    pub op: Reshape,
    /// A cold copy of the map, for the pictures.
    map: Arc<Dimension>,
    sea: i32,
    side: f32,
    preview: Preview,
}

impl Drop for ReshapeForm {
    fn drop(&mut self) {
        self.preview.cancel.store(true, Ordering::Relaxed);
    }
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    Apply(Reshape),
}

impl ReshapeForm {
    pub fn new(what: WholeOp, map: Dimension, sea: i32) -> Self {
        let side = map.bounds().map(|(lo, hi)| ((hi.x - lo.x + 1).max(hi.z - lo.z + 1) * massif_core::coords::TILE_SIZE) as f32).unwrap_or(1024.0);
        Self {
            what,
            op: what.start(sea as f32, side),
            map: Arc::new(map),
            sea,
            side,
            preview: Preview::default(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, reg: &Registry) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(700.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(self.what.icon()).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(format!("{} over the whole map", self.what.label())).size(16.0).bold());
        });
        ui.add(egui::Label::new(RichText::new(self.what.about()).size(12.0).color(pal.weak)).wrap());
        ui.add_space(12.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(290.0);
                ui.label(RichText::new("Settings").size(11.5).color(pal.weak).medium());
                ui.add_space(4.0);
                let limits = (self.map.min_y as f32, (self.map.max_y - 1) as f32);
                match &mut self.op {
                    Reshape::Filter(kind) => {
                        crate::graph_editor::node_settings(ui, kind, reg, limits);
                    }
                    Reshape::Relief { scale, shift } => {
                        relief_settings(ui, scale, shift);
                    }
                }
                ui.add_space(8.0);
                if ui.small_button(theme::with_icon(Icon::RotateCcw, "Reset")).clicked() {
                    self.op = self.what.start(self.sea as f32, self.side);
                }
            });
            ui.add_space(12.0);
            self.pictures(ui);
        });
        ui.add_space(12.0);
        let spacing = (self.side / (self.side as usize).clamp(2, WHOLE_MAX) as f32).max(1.0);
        let read = if spacing <= 1.001 { "read at every block".to_owned() } else { format!("read a sample every {spacing:.1} blocks, the change spread over the blocks between") };
        let side = self.side as u32;
        ui.label(
            RichText::new(format!(
                "The map, {side} blocks across, {read}. Its paint, its levels and what was sculpted in 3D stay; one step to undo."
            ))
            .size(11.5)
            .color(pal.weak),
        );
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Scaled by one and raised by nothing: nothing to do.
            let idle = matches!(self.op, Reshape::Relief { scale, shift } if (scale - 1.0).abs() < 1e-4 && shift.abs() < 0.5);
            if ui.add_enabled(!idle, theme::primary_button(ui, "Apply to the whole map")).clicked() {
                choice = Choice::Apply(self.op.clone());
            }
            if ui.button("Cancel").clicked() {
                choice = Choice::Close;
            }
        });
        if !matches!(choice, Choice::Keep) {
            self.preview.cancel.store(true, Ordering::Relaxed);
        }
        choice
    }

    /// Whether the pictures of the map before and after are there (for tests).
    #[cfg(test)]
    pub fn has_pictures(&self) -> bool {
        self.preview.pictures.is_some()
    }

    /// The map before and after, made again once the settings settle.
    fn pictures(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let key = format!("{:?}", self.op);
        let p = &mut self.preview;
        let shown = self.map.ground().len() <= PREVIEW_TILES;
        if shown && p.made_for.as_deref() != Some(&key) {
            // Wait a moment for the settings to settle before working.
            match &p.changed {
                Some((_, k)) if *k == key => {}
                _ => p.changed = Some((now, key.clone())),
            }
            let since = p.changed.as_ref().map_or(0.0, |(t, _)| now - t);
            if since > 0.25 || p.made_for.is_none() {
                p.cancel.store(true, Ordering::Relaxed);
                let cancel = Arc::new(AtomicBool::new(false));
                p.cancel = cancel.clone();
                p.made_for = Some(key);
                let (tx, rx) = channel();
                p.rx = Some(rx);
                let (map, op, sea, ctx) = (self.map.clone(), self.op.clone(), self.sea, ctx.clone());
                std::thread::Builder::new()
                    .name("massif whole map preview".into())
                    .spawn(move || {
                        let _ = tx.send(reshape_preview(&map, &op, sea, PREVIEW_N, &cancel));
                        ctx.request_repaint();
                    })
                    .expect("a thread for the preview");
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
        if let Some(rx) = &p.rx
            && let Ok(made) = rx.try_recv()
        {
            p.rx = None;
            if let Some((extent, before, after)) = made {
                let range = joint_range(&before, &after);
                let spacing = extent.spacing();
                let picture = |f: &Field, name: &str| ctx.load_texture(name, relief_picture(f, spacing, range), TextureOptions::LINEAR);
                p.pictures = Some((picture(&before, "whole map before"), picture(&after, "whole map after")));
            }
        }
        let working = p.rx.is_some();
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                for (k, name) in ["Before", "After"].into_iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(name).size(11.5).color(pal.weak).medium());
                        ui.add_space(2.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(PICTURE, PICTURE), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect_filled(rect, theme::TILE_RADIUS, pal.well());
                        match &p.pictures {
                            Some(pictures) => {
                                let texture = if k == 0 { &pictures.0 } else { &pictures.1 };
                                egui::Image::new(texture).corner_radius(theme::TILE_RADIUS).paint_at(ui, rect);
                            }
                            None if !shown => {
                                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "No picture\nof a map this large", egui::FontId::proportional(11.5), pal.weak);
                            }
                            None => {}
                        }
                        painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                        if working && k == 1 {
                            let spinner = egui::Rect::from_center_size(rect.right_top() + vec2(-14.0, 14.0), vec2(14.0, 14.0));
                            ui.put(spinner, egui::Spinner::new().size(14.0));
                        }
                    });
                }
            });
            if shown {
                ui.label(RichText::new("The whole map, small: a sample every few blocks.").size(11.0).color(pal.weak));
            }
        });
    }
}

/// The settings of [`Reshape::Relief`].
fn relief_settings(ui: &mut Ui, scale: &mut f32, shift: &mut f32) {
    for (label, tip) in [("Height", "How many times as high the relief stands over the sea"), ("Raise by", "Blocks the whole map goes up (or down, below 0)")] {
        ui.horizontal(|ui| {
            ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(label).size(12.0))).on_hover_text(tip);
            if label == "Height" {
                ui.add(crate::widgets::Field::new(scale, 0.1..=4.0).max_decimals(2).suffix("×"));
            } else {
                ui.add(
                    crate::widgets::Field::new(shift, -256.0..=256.0)
                        .clamping(egui::SliderClamping::Never)
                        .max_decimals(0)
                        .suffix(" blocks"),
                );
            }
        });
    }
}

/// The lowest and highest of two fields.
fn joint_range(a: &Field, b: &Field) -> (f32, f32) {
    let (a0, a1) = a.range();
    let (b0, b1) = b.range();
    (a0.min(b0), a1.max(b1))
}

/// Shaded relief, lit from the north-west, in greys from low to high over
/// `range`: the same for before and after, so that they compare.
fn relief_picture(f: &Field, spacing: f32, range: (f32, f32)) -> ColorImage {
    let px = (PICTURE * 2.0) as usize;
    let k = f.n as f32 / px as f32;
    let (lo, hi) = range;
    let span = (hi - lo).max(1e-3);
    let mut pixels = Vec::with_capacity(px * px);
    for j in 0..px {
        for i in 0..px {
            let (u, v) = ((i as f32 + 0.5) * k - 0.5, (j as f32 + 0.5) * k - 0.5);
            let t = (f.sample(u, v) - lo) / span;
            let dx = (f.sample(u + 1.0, v) - f.sample(u - 1.0, v)) / (2.0 * spacing);
            let dz = (f.sample(u, v + 1.0) - f.sample(u, v - 1.0)) / (2.0 * spacing);
            let shade = (1.0 + (dx + dz) * 0.9).clamp(0.3, 1.6);
            let g = ((46.0 + 168.0 * t) * shade).clamp(0.0, 255.0);
            pixels.push(Color32::from_rgb(g as u8, (g * 1.01).min(255.0) as u8, (g * 1.04).min(255.0) as u8));
        }
    }
    ColorImage::new([px, px], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::auto::Steps;

    /// Every pass over a real map, timed (MASSIF_RIVER_WORLD: a .massif).
    #[test]
    fn whole_map_passes_on_a_real_world() {
        let Some(path) = std::env::var_os("MASSIF_RIVER_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).expect("the world loads");
        let sea = world.meta.sea_level;
        let bed = world.registry.terrain_by_name("Gravel");
        let mut dim = world.dims[0].clone();
        let form = ReshapeForm::new(WholeOp::Erosion, dim.snapshot(), sea);
        for what in WholeOp::ALL {
            let mut w = world.clone();
            let started = std::time::Instant::now();
            let mut history = massif_core::history::History::new(1);
            history.begin(what.label(), 0);
            let changes = massif_core::graph::reshape_ground(&mut history.edit(&mut w, 0), &what.start(sea as f32, form.side), sea, bed, &Steps::default());
            eprintln!("{}: {} columns in {} tiles, {:.2} s", what.label(), changes.columns, changes.tiles.len(), started.elapsed().as_secs_f32());
            assert!(!changes.cancelled);
        }
    }
}
