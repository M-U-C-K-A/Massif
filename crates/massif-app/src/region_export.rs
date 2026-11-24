//! File ▸ Export Region…: a part of the map written as images of 1 pixel a
//! block, to work on in another tool (World Machine, Gaea, an image
//! editor) and bring back with Edit ▸ Import mask…: a 16-bit heightmap,
//! the annotations, the water, and an image for each terrain, biome or
//! layer found there, with a `region.txt` that puts them back in their
//! place (see `massif_core::mask::export_region`).

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, row, switch, well};
use egui::{RichText, Ui};
use massif_core::BlockRect;
use massif_core::mask::RegionParts;

/// Which part of the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Selection,
    Map,
    Blocks,
}

/// The dialog's settings, kept from one opening to the next.
pub struct RegionForm {
    area: Area,
    selection: Option<BlockRect>,
    map: BlockRect,
    /// The blocks typed in: west, north, width, depth.
    x: i32,
    z: i32,
    width: i32,
    depth: i32,
    pub parts: RegionParts,
    /// Keep a version of the map and write `zone.json` beside the images,
    /// for the region to come back as a contribution to review.
    pub review: bool,
    /// Written as a `.massif` of its own rather than as images.
    pub as_map: bool,
    /// The tiles of 128 blocks added around the blocks on every side, when
    /// written as a map.
    pub border: i32,
}

pub enum Choice {
    Keep,
    Close,
    Export(BlockRect, RegionParts),
    /// As a `.massif`: the blocks, and the tiles of border.
    ExportMap(BlockRect, i32),
}

impl RegionForm {
    pub fn new(map: BlockRect, selection: Option<BlockRect>) -> Self {
        let start = selection.unwrap_or(map);
        Self {
            area: if selection.is_some() { Area::Selection } else { Area::Map },
            selection,
            map,
            x: start.x0,
            z: start.z0,
            width: start.width(),
            depth: start.depth(),
            parts: RegionParts::default(),
            review: true,
            as_map: false,
            border: 1,
        }
    }

    /// The blocks to write.
    pub fn rect(&self) -> BlockRect {
        match self.area {
            Area::Selection => self.selection.unwrap_or(self.map),
            Area::Map => self.map,
            Area::Blocks => BlockRect::new(self.x, self.z, self.x + self.width.max(1), self.z + self.depth.max(1)),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(440.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::ImageDown).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Export Region")).size(16.0).bold());
        });
        let about = if self.as_map {
            tr!("A part of the map as a map of its own, at its place, for someone to work on in Massif: File ▸ Merge Massif Map… brings their work back, reviewed on the map before anything changes.")
        } else {
            tr!("A part of the map as images of 1 pixel a block, to work on in another tool: Edit ▸ Import Mask… puts them back in their place, with the region.txt written beside them.")
        };
        ui.add(egui::Label::new(RichText::new(about).size(12.0).color(pal.weak)).wrap());
        ui.add_space(12.0);
        ui.label(RichText::new(tr!("As")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        if let Some(i) = button_group(ui, "region as", &[tr!("Images"), tr!("Massif map")], &[!self.as_map, self.as_map]) {
            self.as_map = i == 1;
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("Where")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let mut areas = Vec::new();
        if self.selection.is_some() {
            areas.push((Area::Selection, tr!("The selection")));
        }
        areas.push((Area::Map, tr!("The whole map")));
        areas.push((Area::Blocks, tr!("These blocks")));
        let labels: Vec<&str> = areas.iter().map(|a| a.1).collect();
        let on: Vec<bool> = areas.iter().map(|a| a.0 == self.area).collect();
        if let Some(i) = button_group(ui, "region area", &labels, &on) {
            if areas[i].0 == Area::Blocks && self.area != Area::Blocks {
                let r = self.rect();
                (self.x, self.z, self.width, self.depth) = (r.x0, r.z0, r.width(), r.depth());
            }
            self.area = areas[i].0;
        }
        if self.area == Area::Blocks {
            ui.add_space(6.0);
            well(ui, |ui| {
                row(ui, tr!("Top left corner"), |ui| {
                    ui.add(Field::new(&mut self.z, -30_000_000..=30_000_000).text("z "));
                    ui.add(Field::new(&mut self.x, -30_000_000..=30_000_000).text("x "));
                });
                row(ui, tr!("Size"), |ui| {
                    ui.add(Field::new(&mut self.depth, 1..=65_535).text(tr!("depth ")));
                    ui.add(Field::new(&mut self.width, 1..=65_535).text(tr!("width ")));
                });
            });
        }
        let r = self.rect();
        if self.as_map {
            ui.add_space(10.0);
            well(ui, |ui| {
                row(ui, tr!("Border"), |ui| {
                    ui.add(Field::new(&mut self.border, 0..=16).text(tr!("tiles ")));
                });
            });
            let region = massif_core::region_map::RegionOf::around(0, r, self.border);
            let (tw, td) = region.size();
            ui.add(
                egui::Label::new(
                    RichText::new(tr!("{w} × {d} blocks from x {x}, z {z}, in {tw} × {td} tiles of 128 blocks with the border: only these tiles come back when it is merged.", w = r.width(), d = r.depth(), x = r.x0, z = r.z0, tw, td))
                        .size(11.5)
                        .color(pal.weak),
                )
                .wrap(),
            );
            ui.add(egui::Label::new(RichText::new(tr!("Keeps a version of the map as it is, the one the work is compared with when it comes back.")).size(11.5).color(pal.weak)).wrap());
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = r.width() > 0 && r.depth() > 0;
                if ui.add_enabled(ok, theme::primary_button(ui, tr!("Export…"))).clicked() {
                    choice = Choice::ExportMap(r, self.border);
                }
                if ui.button(tr!("Cancel")).clicked() {
                    choice = Choice::Close;
                }
            });
            return choice;
        }
        ui.add(egui::Label::new(RichText::new(tr!("{w} × {d} blocks from x {x}, z {z}: images of {w} × {d} pixels.", w = r.width(), d = r.depth(), x = r.x0, z = r.z0)).size(11.5).color(pal.weak)).wrap());
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("What")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let p = &mut self.parts;
        well(ui, |ui| {
            row(ui, tr!("Heightmap (16 bits)"), |ui| {
                switch(ui, &mut p.height);
            });
            row(ui, tr!("Terrains, one image each"), |ui| {
                switch(ui, &mut p.terrains);
            });
            row(ui, tr!("Layers, one image each"), |ui| {
                switch(ui, &mut p.layers);
            });
            row(ui, tr!("Biomes, one image each"), |ui| {
                switch(ui, &mut p.biomes);
            });
            row(ui, tr!("Water"), |ui| {
                switch(ui, &mut p.water);
            });
            row(ui, tr!("Annotations"), |ui| {
                switch(ui, &mut p.annotations);
            });
        });
        ui.add(egui::Label::new(RichText::new(tr!("Each terrain, layer or biome found there gets an image: white where it is, black elsewhere. The images go in a folder of their own.")).size(11.5).color(pal.weak)).wrap());
        ui.add_space(10.0);
        well(ui, |ui| {
            row(ui, tr!("Review it when it comes back"), |ui| {
                switch(ui, &mut self.review);
            });
        });
        let note = if self.review {
            tr!("Keeps a version of the map as it is and writes zone.json beside the images: brought back with Versions ▸ Import a contribution…, the zone is compared with that version and reviewed on the map before anything changes.")
        } else {
            tr!("Brought back with Edit ▸ Import Mask…, the images are painted onto the map at once.")
        };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ok = r.width() > 0 && r.depth() > 0 && r.width() <= 65_535 && r.depth() <= 65_535;
            if ui.add_enabled(ok, theme::primary_button(ui, tr!("Export…"))).clicked() {
                choice = Choice::Export(r, self.parts);
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
        });
        choice
    }
}
