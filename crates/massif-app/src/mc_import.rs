//! File ▸ Import Minecraft World…: the dialog that says what of a world to
//! bring in — its dimension, an area picked on a map of its region files,
//! the heights, and what becomes of what stands above the ground.

use crate::theme::{self, Palette, RichTextExt};
use crate::widgets;
use egui::{Color32, RichText, Sense, Stroke, Ui};
use massif_core::world::DimensionKind;
use massif_legacy::minecraft::{Above, McWorld, Options};

/// The dialog's state.
pub struct McImportForm {
    pub world: McWorld,
    pub opts: Options,
    /// The chunks each region holds, of the dimension shown.
    presence: Vec<(i32, i32, Box<[bool; 1024]>)>,
    presence_of: Option<DimensionKind>,
    /// The chunks of the world: `[x0, z0, x1, z1]`.
    bounds: [i32; 4],
    chunks: usize,
    texture: Option<egui::TextureHandle>,
    /// Chunks per pixel of the map.
    step: i32,
    /// Where a drag over the map started, in chunks.
    drag_from: Option<[i32; 2]>,
}

pub enum McImportAction {
    Import,
    Cancel,
}

impl McImportForm {
    pub fn new(world: McWorld) -> Self {
        let kind = world.dimensions.first().map_or(DimensionKind::Overworld, |d| d.kind);
        let opts = Options::new(&world, kind);
        McImportForm { world, opts, presence: Vec::new(), presence_of: None, bounds: [0; 4], chunks: 0, texture: None, step: 1, drag_from: None }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        if self.presence_of == Some(self.opts.dimension) {
            return;
        }
        self.presence_of = Some(self.opts.dimension);
        self.presence = self.world.dimension(self.opts.dimension).map(|d| massif_legacy::minecraft::presence(&d.regions)).unwrap_or_default();
        let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        self.chunks = 0;
        for (rx, rz, p) in &self.presence {
            for (i, _) in p.iter().enumerate().filter(|c| *c.1) {
                let (x, z) = (rx * 32 + (i % 32) as i32, rz * 32 + (i / 32) as i32);
                b = [b[0].min(x), b[1].min(z), b[2].max(x), b[3].max(z)];
                self.chunks += 1;
            }
        }
        if self.chunks == 0 {
            b = [0, 0, 0, 0];
        }
        self.bounds = b;
        let (w, h) = (b[2] - b[0] + 1, b[3] - b[1] + 1);
        self.step = ((w.max(h) + 511) / 512).max(1);
        let (pw, ph) = (((w + self.step - 1) / self.step) as usize, ((h + self.step - 1) / self.step) as usize);
        let mut img = egui::ColorImage::filled([pw, ph], Color32::TRANSPARENT);
        for (rx, rz, p) in &self.presence {
            for (i, _) in p.iter().enumerate().filter(|c| *c.1) {
                let (x, z) = (rx * 32 + (i % 32) as i32 - b[0], rz * 32 + (i / 32) as i32 - b[1]);
                img[((x / self.step) as usize, (z / self.step) as usize)] = Color32::WHITE;
            }
        }
        self.texture = Some(ctx.load_texture("minecraft import map", img, egui::TextureOptions::NEAREST));
        // Another dimension: its own heights, all of it.
        (self.opts.min_y, self.opts.max_y) = self.world.heights(self.opts.dimension);
        self.opts.area = None;
        // Too much stands on a large world to keep it all.
        if self.chunks > 60_000 && self.opts.above == Above::Keep {
            self.opts.above = Above::Trees;
        }
    }

    /// The chunks the area holds.
    fn chosen(&self) -> usize {
        match self.opts.area {
            None => self.chunks,
            Some([x0, z0, x1, z1]) => self
                .presence
                .iter()
                .map(|(rx, rz, p)| p.iter().enumerate().filter(|(i, has)| **has && (x0..=x1).contains(&(rx * 32 + (*i % 32) as i32)) && (z0..=z1).contains(&(rz * 32 + (*i / 32) as i32))).count())
                .sum(),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Option<McImportAction> {
        self.refresh(ui.ctx());
        let pal = Palette::of(ui);
        let mut action = None;
        ui.set_width(560.0);
        ui.label(RichText::new(tr!("Import a Minecraft world")).bold().size(16.0));
        let mut about = vec![self.world.name.clone()];
        if let Some(v) = &self.world.version {
            about.push(tr!("Minecraft {v}", v).to_string());
        }
        ui.label(RichText::new(about.join(" · ")).size(11.5).color(pal.weak));
        ui.add_space(8.0);

        widgets::well(ui, |ui| {
            let kinds: Vec<DimensionKind> = self.world.dimensions.iter().map(|d| d.kind).collect();
            if kinds.len() > 1 {
                widgets::row(ui, tr!("Dimension"), |ui| {
                    let labels: Vec<&str> = kinds.iter().map(|k| crate::i18n::tr_name(k.label())).collect();
                    let on: Vec<bool> = kinds.iter().map(|k| *k == self.opts.dimension).collect();
                    if let Some(k) = widgets::button_group(ui, "mc dimension", &labels, &on) {
                        self.opts.dimension = kinds[k];
                    }
                });
            }
            widgets::row(ui, tr!("Area"), |ui| {
                let on = [self.opts.area.is_none(), self.opts.area.is_some()];
                match widgets::button_group(ui, "mc area", &[tr!("The whole world"), tr!("Drawn on the map")], &on) {
                    Some(0) => self.opts.area = None,
                    Some(1) if self.opts.area.is_none() => {
                        let b = self.bounds;
                        let (cx, cz) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
                        self.opts.area = Some([(cx - 32).max(b[0]), (cz - 32).max(b[1]), (cx + 31).min(b[2]), (cz + 31).min(b[3])]);
                    }
                    _ => {}
                }
            });
            let (lo, hi) = self.world.heights(self.opts.dimension);
            widgets::row(ui, tr!("Heights"), |ui| {
                let mut max = self.opts.max_y - 1;
                ui.add(widgets::Field::new(&mut max, self.opts.min_y..=hi - 1).width(70.0));
                ui.label(RichText::new(tr!("to")).color(pal.weak));
                ui.add(widgets::Field::new(&mut self.opts.min_y, lo..=max).width(70.0));
                self.opts.max_y = max + 1;
            });
            widgets::row(ui, tr!("Above the ground"), |ui| {
                let all = [Above::Keep, Above::Trees, Above::Drop];
                let on: Vec<bool> = all.iter().map(|a| *a == self.opts.above).collect();
                if let Some(k) = widgets::button_group(ui, "mc above", &[tr!("Keep it"), tr!("Trees as layers"), tr!("Drop it")], &on) {
                    self.opts.above = all[k];
                }
            });
        });
        let says = match self.opts.above {
            Above::Keep if self.chosen() > 60_000 => tr!("Buildings, trees and overhangs are kept block for block in sculpted chunks; a map holds at most 80,000 of them, which an area this large may pass: what stands on the rest is left out."),
            Above::Keep => tr!("Buildings, trees and overhangs are kept block for block in sculpted chunks: they come back unchanged on export."),
            Above::Trees => tr!("Each kind of tree becomes an object layer of the world's own trees, painted where they stood; buildings are left out."),
            Above::Drop => tr!("Only the land comes in: its heights, terrains, water, biomes and plants."),
        };
        ui.add_space(4.0);
        ui.label(RichText::new(says).size(11.5).color(pal.weak));
        ui.add_space(8.0);

        // The map of the world's chunks; the area drawn on it.
        if let Some(tex) = &self.texture {
            let size = tex.size_vec2();
            let scale = (520.0 / size.x).min(240.0 / size.y).min(8.0);
            let (rect, response) = ui.allocate_exact_size(size * scale, Sense::click_and_drag());
            let painter = ui.painter_at(rect.expand(2.0));
            painter.rect_filled(rect, theme::GROUP_RADIUS, pal.well());
            painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), pal.weak.gamma_multiply(0.8));
            let b = self.bounds;
            let to_chunk = |p: egui::Pos2| -> [i32; 2] {
                let x = b[0] + ((p.x - rect.left()) / scale).floor() as i32 * self.step;
                let z = b[1] + ((p.y - rect.top()) / scale).floor() as i32 * self.step;
                [x.clamp(b[0], b[2]), z.clamp(b[1], b[3])]
            };
            if response.drag_started()
                && let Some(p) = response.interact_pointer_pos()
            {
                self.drag_from = Some(to_chunk(p));
            }
            if let (Some(from), Some(p)) = (self.drag_from, response.interact_pointer_pos())
                && response.dragged()
            {
                let to = to_chunk(p);
                self.opts.area = Some([from[0].min(to[0]), from[1].min(to[1]), from[0].max(to[0]) + self.step - 1, from[1].max(to[1]) + self.step - 1]);
            }
            if response.drag_stopped() {
                self.drag_from = None;
            }
            if response.clicked()
                && let Some(p) = response.interact_pointer_pos()
            {
                // A click picks the region under it.
                let [x, z] = to_chunk(p);
                let (rx, rz) = (x.div_euclid(32), z.div_euclid(32));
                self.opts.area = Some([rx * 32, rz * 32, rx * 32 + 31, rz * 32 + 31]);
            }
            if let Some([x0, z0, x1, z1]) = self.opts.area {
                let at = |x: i32, z: i32| egui::pos2(rect.left() + (x - b[0]) as f32 / self.step as f32 * scale, rect.top() + (z - b[1]) as f32 / self.step as f32 * scale);
                let r = egui::Rect::from_min_max(at(x0, z0), at(x1 + 1, z1 + 1));
                painter.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.25));
                painter.rect_stroke(r, 0.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
            }
            response.on_hover_text(tr!("Drag to draw the area to import; a click picks a region (512 × 512 blocks)."));
            let chosen = self.chosen();
            let what = match self.opts.area {
                None => trn!("{n} chunk", "{n} chunks", n = chosen).to_string(),
                Some([x0, z0, x1, z1]) => tr!("{n} chunks · blocks {bx0}, {bz0} to {bx1}, {bz1}", n = chosen, bx0 = x0 * 16, bz0 = z0 * 16, bx1 = x1 * 16 + 15, bz1 = z1 * 16 + 15).to_string(),
            };
            ui.label(RichText::new(what).size(11.5).color(pal.weak));
        }
        ui.add_space(8.0);
        widgets::checkbox(ui, &mut self.opts.covers, tr!("Plants, snow and carpets as ground covers"));
        widgets::checkbox(ui, &mut self.opts.mixes, tr!("Ground of mixed blocks as terrains that mix them"));
        widgets::checkbox(ui, &mut self.opts.biomes, tr!("Biomes"));
        widgets::checkbox(ui, &mut self.opts.caves, tr!("Caves and the underground, as sculpted chunks (takes much memory)"));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(self.chosen() > 0, theme::primary_button(ui, tr!("Import"))).clicked() {
                    action = Some(McImportAction::Import);
                }
                if ui.button(tr!("Cancel")).clicked() {
                    action = Some(McImportAction::Cancel);
                }
            });
        });
        action
    }
}
