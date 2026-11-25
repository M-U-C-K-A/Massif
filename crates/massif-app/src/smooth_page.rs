//! Edit ▸ Smooth terrain…: one switch for the whole map, and for each
//! terrain the partial blocks it may use (layers, slabs, stairs, vertical
//! slabs, corners), as its top block has them (see `massif_core::shapes`).

use crate::blocks::BlockLibrary;
use crate::theme::{self, Palette, RichTextExt};
use egui::{RichText, Sense, Ui, vec2};
use massif_core::registry::{Registry, TerrainId};
use massif_core::shapes::{Shape, ShapeSet, SmoothSettings};
use massif_core::world::Dimension;

/// What the page's buttons ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Keep,
    Apply,
    Cancel,
}

/// The page: the settings being changed, the terrains the map uses, and
/// the shapes each terrain's top block has.
pub struct SmoothForm {
    pub settings: SmoothSettings,
    /// The terrains painted on the map (on its ground), most used first.
    used: Vec<TerrainId>,
    /// Every terrain shown, not only those painted.
    all: bool,
    /// The shapes the top block of each terrain has, by `TerrainId`.
    has: Vec<ShapeSet>,
}

impl SmoothForm {
    pub fn new(settings: SmoothSettings, reg: &Registry, dim: &Dimension) -> Self {
        // The terrains of the ground, by how many tiles show them.
        let mut counts = vec![0usize; reg.terrains.len()];
        let ground = dim.ground();
        for pos in ground.positions() {
            let Some(t) = ground.view(pos) else { continue };
            match t.terrain.uniform_value() {
                Some(v) => {
                    if let Some(c) = counts.get_mut(v as usize) {
                        *c += massif_core::TILE_AREA;
                    }
                }
                None => {
                    for v in t.terrain.dense().iter() {
                        if let Some(c) = counts.get_mut(*v as usize) {
                            *c += 1;
                        }
                    }
                }
            }
        }
        let mut used: Vec<TerrainId> = (0..counts.len()).filter(|&k| counts[k] > 0).map(|k| TerrainId(k as u16)).collect();
        used.sort_by_key(|t| std::cmp::Reverse(counts[t.index()]));
        let mods = crate::blocks::mod_blocks();
        let families = crate::blocks::families();
        let known = massif_core::shapes::Known { vanilla: &crate::blocks::vanilla_exists, mods: &mods, families: &families };
        let has = (0..reg.terrains.len())
            .map(|k| massif_core::shapes::top_block(reg, TerrainId(k as u16)).map_or(ShapeSet::NONE, |b| massif_core::shapes::usable(ShapeSet::of(&Shape::ALL), &massif_core::shapes::family(reg.block(b).name(), &known))))
            .collect();
        Self { settings, used, all: false, has }
    }

    pub fn ui(&mut self, ui: &mut Ui, reg: &Registry, blocks: &mut BlockLibrary) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(560.0);
        ui.label(RichText::new(tr!("Smooth terrain")).bold().size(15.0));
        ui.add(egui::Label::new(tr!("The top of the land built of partial blocks when the map is exported, following its precise height: layers, slabs, stairs at the steps, vertical slabs at the foot of cliffs, corner pieces. Each column takes the pieces of the block painted there: repaint it, and the smoothing follows.")).wrap());
        ui.add_space(8.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Smooth the whole map"), |ui| {
                crate::widgets::switch(ui, &mut self.settings.on);
            });
            crate::widgets::row(ui, tr!("Every terrain"), |ui| {
                crate::widgets::switch(ui, &mut self.all);
            });
        });
        ui.add_space(8.0);
        // The terrains, each with its shapes: a box per shape, greyed where
        // its block has none.
        let shown: Vec<TerrainId> = if self.all { (0..reg.terrains.len() as u16).map(TerrainId).collect() } else { self.used.clone() };
        let name_w = 190.0;
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("smooth terrains").num_columns(Shape::ALL.len() + 1).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label(RichText::new(tr!("Terrain")).size(11.0).color(pal.weak));
                for s in Shape::ALL {
                    ui.label(RichText::new(crate::i18n::tr_name(s.label())).size(11.0).color(pal.weak));
                }
                ui.end_row();
                for t in shown {
                    let def = reg.terrain(t);
                    ui.horizontal(|ui| {
                        ui.set_width(name_w);
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        crate::panels::terrain_icon(ui, r, reg, t.0, blocks);
                        let custom = self.settings.terrains.contains_key(&def.name);
                        ui.add(egui::Label::new(if custom { RichText::new(&def.name).strong() } else { RichText::new(&def.name) }).truncate());
                    });
                    let has = self.has.get(t.index()).copied().unwrap_or_default();
                    let mut set = self.settings.shapes_of(reg, t);
                    let before = set;
                    for s in Shape::ALL {
                        ui.add_enabled_ui(has.has(s), |ui| {
                            let mut on = set.has(s) && has.has(s);
                            if crate::widgets::checkbox(ui, &mut on, "").on_disabled_hover_text(tr!("Its block has no such piece, in the game or the mods read")).changed() {
                                set.set(s, on);
                            }
                        });
                    }
                    if set != before {
                        self.settings.terrains.insert(def.name.clone(), set);
                    }
                    ui.end_row();
                }
            });
        });
        if self.used.is_empty() && !self.all {
            crate::panels::hint(ui, tr!("Nothing painted yet: turn on Every terrain to set them all."));
        }
        crate::panels::hint(ui, tr!("Bold: set by hand; the others take the defaults of their block, from the game and the mods' profiles. The Smoothing brush turns it on or off in places."));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if crate::widgets::button(ui, tr!("Back to the defaults")).clicked() {
                self.settings.terrains.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(theme::primary_button(ui, tr!("Apply"))).clicked() {
                    choice = Choice::Apply;
                }
                if ui.button(tr!("Cancel")).clicked() {
                    choice = Choice::Cancel;
                }
            });
        });
        choice
    }
}
