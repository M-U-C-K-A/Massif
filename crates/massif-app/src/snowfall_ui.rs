//! Edit ▸ Whole map ▸ Snowfall: the dialog of the snow simulation, its
//! settings beside a picture of the map with the snow it would lay, then
//! the snow laid in the background as one step to undo.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, row, switch, well};
use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui, vec2};
use massif_core::graph::{Field as Grid, Snowfall, snow_over};
use massif_core::world::Dimension;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// Samples a side of the preview.
const PREVIEW_N: usize = 256;
/// The picture's side, in points.
const PICTURE: f32 = 300.0;
/// Maps with more tiles than this have no picture.
const PREVIEW_TILES: usize = 4096;

/// What the preview found: the picture, and how much of the land is white.
type Found = (ColorImage, f32, f32);

/// The dialog of Snowfall.
pub struct SnowfallForm {
    pub fall: Snowfall,
    start: Snowfall,
    map: Arc<Dimension>,
    made_for: Option<String>,
    changed: Option<(f64, String)>,
    rx: Option<Receiver<Option<Found>>>,
    cancel: Arc<AtomicBool>,
    picture: Option<(TextureHandle, f32, f32)>,
}

impl Drop for SnowfallForm {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    Apply(Snowfall),
}

impl SnowfallForm {
    pub fn new(map: Dimension) -> Self {
        // The land's heights, from a coarse look at it.
        let (lo, hi) = land_range(&map);
        let start = Snowfall::for_land(lo, hi, (map.max_y - 1) as f32);
        Self {
            fall: start.clone(),
            start,
            map: Arc::new(map),
            made_for: None,
            changed: None,
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            picture: None,
        }
    }

    /// Whether the preview is there (for tests).
    #[cfg(test)]
    pub fn has_picture(&self) -> bool {
        self.picture.is_some()
    }

    pub fn ui(&mut self, ui: &mut Ui, has_snow: bool) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(700.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Snowflake).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Snowfall over the whole map")).size(16.0).bold());
        });
        ui.add(
            egui::Label::new(
                RichText::new(tr!("Snow falls over a band of heights; steep slopes shed it, the wind strips the slopes facing it and piles drifts in the lee of ridges and in hollows. It is laid in snow layers, then snow blocks under them."))
                    .size(12.0)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        // Its settings scroll in a small window; the buttons stay in it.
        let room = (ui.ctx().content_rect().height() - 290.0).max(160.0);
        egui::ScrollArea::vertical().id_salt("snowfall settings").max_height(room).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(330.0);
                    self.settings(ui);
                    ui.add_space(6.0);
                    if ui.small_button(theme::with_icon(Icon::RotateCcw, tr!("Reset"))).clicked() {
                        self.fall = self.start.clone();
                    }
                });
                ui.add_space(12.0);
                self.preview(ui);
            });
        });
        ui.add_space(10.0);
        let note = if has_snow {
            tr!("Snow replaces the top blocks of the ground under it; its columns are smoothed so that the export writes the layers. Within the selection when there is one; one step to undo.")
        } else {
            tr!("This map has no Snow terrain to lay.")
        };
        ui.label(RichText::new(note).size(11.5).color(if has_snow { pal.weak } else { pal.danger }));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(has_snow && self.fall.amount > 0.0, theme::primary_button(ui, tr!("Let it snow"))).clicked() {
                choice = Choice::Apply(self.fall.clone());
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
        });
        if !matches!(choice, Choice::Keep) {
            self.cancel.store(true, Ordering::Relaxed);
        }
        choice
    }

    fn settings(&mut self, ui: &mut Ui) {
        let f = &mut self.fall;
        let pal = Palette::of(ui);
        let head = |ui: &mut Ui, name: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(name).size(11.5).color(pal.weak).medium());
            ui.add_space(2.0);
        };
        head(ui, tr!("Snow"));
        well(ui, |ui| {
            row(ui, tr!("Intensity"), |ui| {
                ui.add(Field::new(&mut f.amount, 0.0..=8.0).max_decimals(2).suffix(tr!(" blocks"))).on_hover_text(tr!("How much snow falls, before the wind and the slopes move it"));
            });
            row(ui, tr!("Lowest"), |ui| {
                ui.add(Field::new(&mut f.low, -64.0..=2032.0).max_decimals(0).suffix(" y"));
            });
            row(ui, tr!("Highest"), |ui| {
                ui.add(Field::new(&mut f.high, -64.0..=2032.0).max_decimals(0).suffix(" y"));
            });
            row(ui, tr!("Fade"), |ui| {
                ui.add(Field::new(&mut f.fade, 0.0..=200.0).max_decimals(0).suffix(tr!(" blocks"))).on_hover_text(tr!("How far past each end of the band the snow thins out"));
            });
            row(ui, tr!("Deepest"), |ui| {
                ui.add(Field::new(&mut f.most, 1..=128).suffix(tr!(" layers"))).on_hover_text(tr!("The most snow anywhere, in snow layers: 8 make a block"));
            });
            row(ui, tr!("Accumulation"), |ui| {
                ui.add(Field::new(&mut f.settle, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))).on_hover_text(tr!("How much of the snow settles where it falls; the rest drifts with the wind"));
            });
            row(ui, tr!("Steepest slope"), |ui| {
                ui.add(Field::new(&mut f.slope, 10.0..=85.0).max_decimals(0).suffix("°")).on_hover_text(tr!("Slopes steeper than this shed their snow"));
            });
        });
        head(ui, tr!("Wind"));
        well(ui, |ui| {
            row(ui, tr!("Strength"), |ui| {
                ui.add(Field::new(&mut f.wind, 0.0..=96.0).max_decimals(0).suffix(tr!(" blocks"))).on_hover_text(tr!("How far the wind carries the snow: 0, no wind"));
            });
            row(ui, tr!("From"), |ui| {
                ui.add(Field::new(&mut f.direction, 0.0..=359.0).max_decimals(0).suffix("°")).on_hover_text(tr!("Where the wind blows from: 0° north, 90° east"));
            });
            row(ui, tr!("Iterations"), |ui| {
                ui.add(Field::new(&mut f.iterations, 1..=60)).on_hover_text(tr!("Passes of the simulation: more, smoother drifts"));
            });
        });
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, tr!("Only on Frost and snow covers"), |ui| {
                switch(ui, &mut f.only_covers);
            });
        });
        if f.high < f.low {
            f.high = f.low;
        }
    }

    /// The map seen from above with the snow, made again once the settings
    /// settle.
    fn preview(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        // Only what changes the snow: whether it keeps to the covers is
        // read on the map when laid.
        let key = format!("{:?}", Snowfall { only_covers: false, ..self.fall.clone() });
        let shown = self.map.ground().len() <= PREVIEW_TILES;
        if shown && self.made_for.as_deref() != Some(&key) {
            match &self.changed {
                Some((_, k)) if *k == key => {}
                _ => self.changed = Some((now, key.clone())),
            }
            let since = self.changed.as_ref().map_or(0.0, |(t, _)| now - t);
            if since > 0.25 || self.made_for.is_none() {
                self.cancel.store(true, Ordering::Relaxed);
                let cancel = Arc::new(AtomicBool::new(false));
                self.cancel = cancel.clone();
                self.made_for = Some(key);
                let (tx, rx) = channel();
                self.rx = Some(rx);
                let (map, fall, ctx) = (self.map.clone(), self.fall.clone(), ctx.clone());
                let work = move || {
                    let found = snow_over(&map, &fall, PREVIEW_N, &cancel).map(|s| picture(&s.heights, &s.depth, s.extent.spacing()));
                    let _ = tx.send(found);
                    ctx.request_repaint();
                };
                if cfg!(test) {
                    work();
                } else {
                    std::thread::Builder::new().name("massif snowfall preview".into()).spawn(work).expect("a thread for the preview");
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
        if let Some(rx) = &self.rx
            && let Ok(found) = rx.try_recv()
        {
            self.rx = None;
            if let Some((image, white, deepest)) = found {
                self.picture = Some((ctx.load_texture("snowfall preview", image, TextureOptions::LINEAR), white, deepest));
            }
        }
        let working = self.rx.is_some();
        ui.vertical(|ui| {
            ui.label(RichText::new(tr!("Preview")).size(11.5).color(pal.weak).medium());
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(vec2(PICTURE, PICTURE), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, theme::TILE_RADIUS, pal.well());
            match &self.picture {
                Some((texture, ..)) => {
                    egui::Image::new(texture).corner_radius(theme::TILE_RADIUS).paint_at(ui, rect);
                }
                None if !shown => {
                    painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr!("No picture\nof a map this large"), egui::FontId::proportional(11.5), pal.weak);
                }
                None => {}
            }
            painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            if working {
                let spinner = egui::Rect::from_center_size(rect.right_top() + vec2(-15.0, 15.0), vec2(16.0, 16.0));
                crate::loader::paint(ui, spinner, crate::loader::pattern(ui.ctx()));
            }
            if let Some((_, white, deepest)) = &self.picture {
                let layers = (deepest * 8.0).round() as u32;
                ui.label(RichText::new(tr!("Snow over {share:.0}% of the land, {layers} layers at the deepest.", share = white * 100.0, layers)).size(11.0).color(pal.weak));
            }
        });
    }
}

/// The lowest and highest ground of a map, from a coarse look at it.
fn land_range(map: &Dimension) -> (f32, f32) {
    let ground = map.ground();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for pos in ground.positions() {
        if let Some(t) = ground.view(pos) {
            for i in (0..(massif_core::TILE_SIZE * massif_core::TILE_SIZE) as usize).step_by(97) {
                let h = t.top.get(i);
                lo = lo.min(h);
                hi = hi.max(h);
            }
        }
    }
    if lo > hi { (60.0, 120.0) } else { (lo, hi) }
}

/// The relief shaded in greys, white where the snow lies (whiter the
/// deeper), a little blue in the deepest drifts; with the share of the land
/// under snow and the deepest snow, in blocks.
fn picture(heights: &Grid, depth: &Grid, spacing: f32) -> Found {
    let px = (PICTURE * 2.0) as usize;
    let k = heights.n as f32 / px as f32;
    let (lo, hi) = heights.range();
    let span = (hi - lo).max(1e-3);
    let deepest = depth.data.iter().copied().fold(0.0f32, f32::max);
    let white = depth.data.iter().filter(|&&d| d >= 1.0 / 16.0).count() as f32 / depth.data.len().max(1) as f32;
    let mut pixels = Vec::with_capacity(px * px);
    for j in 0..px {
        for i in 0..px {
            let (u, v) = ((i as f32 + 0.5) * k - 0.5, (j as f32 + 0.5) * k - 0.5);
            let h = heights.sample(u, v);
            let d = depth.sample(u, v);
            let t = (h - lo) / span;
            let top = |a: f32, b: f32| heights.sample(a, b) + depth.sample(a, b);
            let dx = (top(u + 1.0, v) - top(u - 1.0, v)) / (2.0 * spacing);
            let dz = (top(u, v + 1.0) - top(u, v - 1.0)) / (2.0 * spacing);
            let shade = (1.0 + (dx + dz) * 0.9).clamp(0.3, 1.5);
            let ground = [86.0 + 90.0 * t, 92.0 + 80.0 * t, 70.0 + 70.0 * t];
            let snow = if d >= 1.0 / 16.0 { (0.55 + d * 0.6).min(1.0) } else { 0.0 };
            let deep = (d / 3.0).clamp(0.0, 1.0) * 0.12;
            let c = [0, 1, 2].map(|k| {
                let lit = [238.0 - 30.0 * deep, 242.0 - 18.0 * deep, 250.0][k];
                ((ground[k] + (lit - ground[k]) * snow) * shade).clamp(0.0, 255.0) as u8
            });
            pixels.push(Color32::from_rgb(c[0], c[1], c[2]));
        }
    }
    (ColorImage::new([px, px], pixels), white, deepest)
}
