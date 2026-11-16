//! Edit ▸ Caves: tunnels, ravines and caverns carved into the map's matter,
//! over the whole map or its selection, in the background as one step to
//! undo (see `massif_core::caves`).

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, row, switch, well};
use egui::{RichText, Ui};
use massif_core::caves::CaveSettings;
use massif_core::voxel::Fluid;

/// The dialog's settings, kept from one opening to the next.
pub struct CaveForm {
    pub settings: CaveSettings,
    /// The heights the map has, `min_y..max_y`.
    heights: (i32, i32),
    /// Whether the map has a selection to keep to.
    selected: bool,
    /// Its side, in blocks, and its tiles.
    side: i32,
    tiles: usize,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    Carve(CaveSettings),
}

impl CaveForm {
    pub fn new(settings: CaveSettings, heights: (i32, i32), selected: bool, side: i32, tiles: usize) -> Self {
        Self { settings, heights, selected, side, tiles }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(420.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Shovel).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new("Caves").size(16.0).bold());
        });
        ui.add(
            egui::Label::new(
                RichText::new("Winding tunnels with their chambers, ravines and large caverns, carved into the rock as in 3D sculpting: they show in the 3D views and are exported as air.")
                    .size(12.0)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        let (lo, hi) = (self.heights.0 + 1, self.heights.1 - 1);
        let s = &mut self.settings;
        ui.label(RichText::new("What").size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, "Tunnels and chambers", |ui| {
                switch(ui, &mut s.tunnels);
            });
            row(ui, "Ravines", |ui| {
                switch(ui, &mut s.ravines);
            });
            row(ui, "Caverns", |ui| {
                switch(ui, &mut s.caverns);
            });
        });
        ui.add_space(10.0);
        ui.label(RichText::new("How").size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, "Density", |ui| {
                let mut percent = (s.density * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                    s.density = percent / 100.0;
                }
            });
            row(ui, "Size", |ui| {
                ui.add(Field::new(&mut s.size, 0.5..=2.0).max_decimals(2).suffix("×"));
            });
            row(ui, "Highest", |ui| {
                ui.add(Field::new(&mut s.high, lo..=hi).suffix(" y"));
            });
            row(ui, "Lowest", |ui| {
                ui.add(Field::new(&mut s.low, lo..=hi).suffix(" y"));
            });
            row(ui, "Open to the surface", |ui| {
                let mut percent = (s.openings * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text("The share of tunnels and ravines that reach up to the sky").changed() {
                    s.openings = percent / 100.0;
                }
            });
            row(ui, "Seed", |ui| {
                if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text("Another seed").clicked() {
                    s.seed = s.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 1_000_000;
                }
                ui.add(Field::new(&mut s.seed, 0..=999_999));
            });
        });
        if s.low > s.high {
            std::mem::swap(&mut s.low, &mut s.high);
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Lakes in the caverns").size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, "Fill with", |ui| {
                let choices = [None, Some(Fluid::Water), Some(Fluid::Lava)];
                let on = choices.map(|c| c == s.lake);
                if let Some(k) = crate::widgets::button_group(ui, "cave lake", &["Nothing", "Water", "Lava"], &on) {
                    s.lake = choices[k];
                }
            });
            row(ui, "Up to", |ui| {
                ui.add_enabled(s.lake.is_some() && s.caverns, Field::new(&mut s.lake_level, lo..=hi).suffix(" y"));
            });
        });
        ui.add_space(12.0);
        let over = if self.selected { "Over the selection only, by its weight".to_owned() } else { format!("Over the whole map, {} blocks across ({} tiles)", self.side, self.tiles) };
        ui.label(RichText::new(format!("{over}. Caves keep a roof of 5 blocks under the ground except where they open; one step to undo.")).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(self.settings.carves(), theme::primary_button(ui, "Carve")).clicked() {
                choice = Choice::Carve(self.settings.clone());
            }
            if ui.button("Cancel").clicked() {
                choice = Choice::Close;
            }
        });
        choice
    }
}
