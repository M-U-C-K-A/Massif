//! File ▸ Export WorldPainter World…: the map as a WorldPainter world
//! (`.world`), to open and go on with in WorldPainter, and what that could
//! not take (see `massif_legacy::wp_export`).

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{combo, row, well};
use egui::{RichText, Ui};
use massif_legacy::WpReport;
use std::path::{Path, PathBuf};

/// The dialog's choice: the version of Minecraft the world is for.
pub struct WpForm {
    choices: Vec<(&'static str, &'static str)>,
    chosen: usize,
}

pub enum Choice {
    Keep,
    Close,
    /// Export for this version of Minecraft.
    Export(String),
}

impl WpForm {
    /// The dialog, set for `minecraft`, the version the map is exported for.
    pub fn new(minecraft: &str) -> Self {
        let choices = massif_legacy::wp_export::platform_choices();
        let name = massif_legacy::wp_export::platform_for(minecraft).display_name;
        let chosen = choices.iter().position(|c| c.0 == name).unwrap_or(0);
        Self { choices, chosen }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(440.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Upload).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Export WorldPainter World")).size(16.0).bold());
        });
        ui.add(
            egui::Label::new(
                RichText::new(tr!("The map as a WorldPainter world (.world), to open and go on with in WorldPainter: heights, terrains (custom ones for blocks WorldPainter does not have), water and lava, biomes, ground covers, object layers with their objects, frost, pockets and annotations."))
                    .size(12.0)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        well(ui, |ui| {
            row(ui, tr!("For Minecraft"), |ui| {
                combo("wp export platform").selected_text(self.choices[self.chosen].0).show_ui(ui, |ui| {
                    for (k, (name, _)) in self.choices.iter().enumerate() {
                        ui.selectable_value(&mut self.chosen, k, *name);
                    }
                });
            });
        });
        ui.add(
            egui::Label::new(
                RichText::new(tr!("WorldPainter holds one height a column: levels over and under the ground, sculpted 3D and smooth terrain stay in the Massif map. The report says what was left out."))
                    .size(11.5)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(theme::primary_button(ui, tr!("Export…"))).clicked() {
                choice = Choice::Export(self.choices[self.chosen].1.to_owned());
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
        });
        choice
    }
}

/// What the report asks.
pub enum ReportChoice {
    Keep,
    Close,
    /// Show the file in the file manager.
    Reveal(PathBuf),
}

/// The report of an export: what went where, what was approximated, what
/// WorldPainter could not take.
pub fn report_ui(ui: &mut Ui, report: &WpReport, path: &Path) -> ReportChoice {
    let pal = Palette::of(ui);
    let mut choice = ReportChoice::Keep;
    ui.set_width(560.0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ui.label(RichText::new(tr!("Exported “{name}” for WorldPainter", name)).bold().size(16.0));
    ui.label(RichText::new(tr!("{platform} · {tiles} tiles · {size}", platform = report.platform, tiles = report.tiles, size = crate::panels::human(report.bytes))).size(11.5).color(pal.weak));
    ui.add_space(8.0);
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
        egui::Grid::new("wp export grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(RichText::new(tr!("Written")).color(pal.weak));
            ui.vertical(|ui| {
                for d in &report.dimensions {
                    ui.label(d);
                }
            });
            ui.end_row();
            let list = |ui: &mut Ui, title: &str, items: &[String]| {
                if items.is_empty() {
                    return;
                }
                ui.label(RichText::new(title).color(pal.weak));
                ui.add(egui::Label::new(RichText::new(items.join(", ")).size(11.5)).wrap());
                ui.end_row();
            };
            list(ui, tr!("Custom terrains"), &report.custom_terrains);
            list(ui, tr!("Custom biomes"), &report.custom_biomes);
            let layers: Vec<String> = report.layers.iter().map(|(n, t)| tr!("{n} ({t} tiles)", n, t)).collect();
            list(ui, tr!("Layers"), &layers);
            for (title, items) in [(tr!("Approximated"), &report.approximated), (tr!("Left out"), &report.left_out)] {
                if items.is_empty() {
                    continue;
                }
                ui.label(RichText::new(title).color(pal.weak));
                ui.vertical(|ui| {
                    for item in items.iter() {
                        ui.add(egui::Label::new(RichText::new(format!("• {item}")).size(11.5)).wrap());
                    }
                });
                ui.end_row();
            }
        });
    });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(theme::primary_button(ui, tr!("Close"))).clicked() {
                choice = ReportChoice::Close;
            }
            if ui.button(tr!("Show in Finder")).clicked() {
                choice = ReportChoice::Reveal(path.to_path_buf());
            }
        });
    });
    choice
}
