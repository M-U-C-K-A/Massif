//! Objects ▸ Grow trees: the dialog of the tree generator. A kind of tree
//! to start from, a few settings, a picture of the tree they grow, then as
//! many variants as asked added to the object layer picked or to a new one.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::ToolState;
use crate::widgets::{Field, row, switch, well};
use egui::{RichText, TextureHandle, TextureOptions, Ui, vec2};
use massif_core::objects::{ObjectDef, ObjectLayerDef};
use massif_core::treegen::{Crown, TreeParams, TreePreset, grow, variants};
use crate::doc::Document;

/// The woods offered, as the game names them.
const WOODS: [&str; 9] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "pale_oak"];
/// The leaves offered.
const LEAVES: [&str; 11] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "pale_oak", "azalea", "flowering_azalea"];

/// A tree's picture, and its size in pixels.
type Picture = (TextureHandle, [usize; 2]);

/// The dialog of Grow trees.
pub struct GrowForm {
    pub preset: TreePreset,
    pub params: TreeParams,
    /// How many variants to add.
    pub count: usize,
    more: bool,
    /// The pictures of the tree of this seed and of the next ones, and the
    /// settings they were grown with.
    pictures: Option<(String, Vec<Picture>)>,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    /// Add the trees to the layer picked (`false`) or to a new one.
    Add(bool),
}

impl GrowForm {
    pub fn new() -> Self {
        let preset = TreePreset::Oak;
        Self { preset, params: preset.params(), count: 6, more: false, pictures: None }
    }

    pub fn ui(&mut self, ui: &mut Ui, layer: Option<&str>) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(700.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::TreeDeciduous).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Grow trees")).size(16.0).bold());
        });
        ui.add(
            egui::Label::new(
                RichText::new(tr!("Trees grown from a few settings: a trunk that flares at its foot, limbs that bend under their weight and turn to the sun, leaves where the twigs end. Each seed grows another tree."))
                    .size(12.0)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(12.0);
        let room = (ui.ctx().content_rect().height() - 290.0).max(160.0);
        egui::ScrollArea::vertical().id_salt("grow trees settings").max_height(room).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(340.0);
                    self.settings(ui);
                });
                ui.add_space(12.0);
                ui.vertical(|ui| self.preview(ui));
            });
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("Smaller trees for Tree sizes are grown with them. The trees are part of the map; nothing is written to disk.")).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = self.count;
            if ui.add(theme::primary_button(ui, trn!("Add {n} tree to a new layer", "Add {n} trees to a new layer", n = n))).clicked() {
                choice = Choice::Add(true);
            }
            if let Some(name) = layer
                && ui.button(tr!("Add to {name}", name)).clicked()
            {
                choice = Choice::Add(false);
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
            if ui.button(theme::with_icon(Icon::FileBox, &trn!("Export {n} as .schem…", "Export {n} as .schem…", n = n))).on_hover_text(tr!("Each as a Sponge schematic of its own, its anchor the origin: to paste with WorldEdit or bring into another map")).clicked() {
                let (trees, _) = trees(&self.params, self.preset.name(), n);
                let refs: Vec<&ObjectDef> = trees.iter().collect();
                if let Some(said) = crate::object_list::export_schematics(&refs) {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("grow trees schematics said"), said));
                }
            }
        });
        if let Some(said) = ui.ctx().data(|d| d.get_temp::<String>(egui::Id::unique("grow trees schematics said"))) {
            ui.label(RichText::new(said).size(11.5).color(pal.weak));
        }
        choice
    }

    fn settings(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let head = |ui: &mut Ui, name: &str| {
            ui.add_space(4.0);
            ui.label(RichText::new(name).size(11.5).color(pal.weak).medium());
            ui.add_space(2.0);
        };
        head(ui, tr!("Kind"));
        let names: Vec<&str> = TreePreset::ALL.iter().map(|p| tr(p.name())).collect();
        let on: Vec<bool> = TreePreset::ALL.iter().map(|&p| p == self.preset).collect();
        if let Some(k) = crate::widgets::button_rows(ui, "grow preset", &names, &on) {
            let seed = self.params.seed;
            let survival = self.params.survival;
            self.preset = TreePreset::ALL[k];
            self.params = self.preset.params();
            self.params.seed = seed;
            self.params.survival = survival;
        }
        let p = &mut self.params;
        head(ui, tr!("Shape"));
        well(ui, |ui| {
            row(ui, tr!("Height"), |ui| {
                ui.add(Field::new(&mut p.height, 5.0..=128.0).max_decimals(0).suffix(tr!(" blocks")));
            });
            row(ui, tr!("Trunk width"), |ui| {
                ui.add(Field::new(&mut p.trunk, 1.0..=9.0).max_decimals(1).suffix(tr!(" blocks")));
            });
            row(ui, tr!("Limbs"), |ui| {
                ui.add(Field::new(&mut p.branches, 1..=48));
            });
            row(ui, tr!("Droop"), |ui| {
                ui.add(percent(&mut p.gravity)).on_hover_text(tr!("How much branches bend down under their weight"));
            });
            row(ui, tr!("Reach for the sun"), |ui| {
                ui.add(percent(&mut p.sun)).on_hover_text(tr!("How much branches turn up towards the light"));
            });
        });
        ui.add_space(4.0);
        let crowns: Vec<&str> = Crown::ALL.iter().map(|c| tr(c.name())).collect();
        let on: Vec<bool> = Crown::ALL.iter().map(|&c| c == p.crown).collect();
        if let Some(k) = crate::widgets::button_rows(ui, "grow crown", &crowns, &on) {
            p.crown = Crown::ALL[k];
        }
        head(ui, tr!("Leaves"));
        well(ui, |ui| {
            row(ui, tr!("Clump size"), |ui| {
                ui.add(Field::new(&mut p.leaf_size, 0.0..=6.0).max_decimals(1).suffix(tr!(" blocks")));
            });
            row(ui, tr!("Fullness"), |ui| {
                ui.add(percent(&mut p.leaf_density)).on_hover_text(tr!("0% for a bare tree"));
            });
        });
        head(ui, tr!("Blocks"));
        well(ui, |ui| {
            row(ui, tr!("Wood"), |ui| {
                crate::widgets::combo("grow wood").selected_text(pretty(&p.wood)).width(150.0).show_ui(ui, |ui| {
                    for w in WOODS {
                        ui.selectable_value(&mut p.wood, w.to_owned(), pretty(w));
                    }
                });
            });
            row(ui, tr!("Leaves"), |ui| {
                crate::widgets::combo("grow leaves").selected_text(pretty(&p.leaves)).width(150.0).show_ui(ui, |ui| {
                    for l in LEAVES {
                        ui.selectable_value(&mut p.leaves, l.to_owned(), pretty(l));
                    }
                });
            });
            row(ui, tr!("Survival friendly"), |ui| {
                switch(ui, &mut p.survival).on_hover_text(tr!("Only logs and leaves, the blocks a tree of the game has: no fences, stairs or slabs"));
            });
        });
        head(ui, tr!("Variants"));
        well(ui, |ui| {
            row(ui, tr!("Seed"), |ui| {
                let mut seed = p.seed as i64;
                if ui.add(Field::new(&mut seed, 0..=999_999)).changed() {
                    p.seed = seed.max(0) as u64;
                }
                if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text(tr!("Another seed")).clicked() {
                    p.seed = (ui.ctx().input(|i| i.time) * 1000.0) as u64 % 1_000_000;
                }
            });
            row(ui, tr!("Trees"), |ui| {
                ui.add(Field::new(&mut self.count, 1..=32)).on_hover_text(tr!("How many to add, each of its own seed after this one"));
            });
        });
        ui.add_space(4.0);
        crate::widgets::checkbox(ui, &mut self.more, tr!("More settings"));
        if self.more {
            let p = &mut self.params;
            well(ui, |ui| {
                row(ui, tr!("Flare"), |ui| {
                    ui.add(percent(&mut p.flare)).on_hover_text(tr!("How much the trunk widens at its foot"));
                });
                row(ui, tr!("Trunk up to"), |ui| {
                    ui.add(percent(&mut p.trunk_share)).on_hover_text(tr!("How far up the trunk goes before it ends in its limbs"));
                });
                row(ui, tr!("First limb at"), |ui| {
                    ui.add(percent(&mut p.start)).on_hover_text(tr!("Where on the trunk the limbs begin"));
                });
                row(ui, tr!("Branch levels"), |ui| {
                    ui.add(Field::new(&mut p.levels, 1..=4));
                });
                row(ui, tr!("Angle"), |ui| {
                    ui.add(Field::new(&mut p.angle, 5.0..=120.0).max_decimals(0).suffix("°")).on_hover_text(tr!("Between a branch and what it grows from"));
                });
                row(ui, tr!("Limb length"), |ui| {
                    ui.add(percent(&mut p.length)).on_hover_text(tr!("The longest limbs, as a share of the height"));
                });
                row(ui, tr!("Each level"), |ui| {
                    ui.add(percent(&mut p.falloff)).on_hover_text(tr!("Each level of branches as long as this share of the one it grows from"));
                });
                row(ui, tr!("Crooked"), |ui| {
                    ui.add(percent(&mut p.crook));
                });
            });
            if ui.small_button(theme::with_icon(Icon::RotateCcw, tr!("Reset"))).clicked() {
                let (seed, survival) = (p.seed, p.survival);
                *p = self.preset.params();
                p.seed = seed;
                p.survival = survival;
            }
        }
    }

    /// The tree of this seed, and the next seeds' smaller under it: grown
    /// again when the settings change. Picking one takes its seed.
    fn preview(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let key = format!("{:?}", self.params);
        if self.pictures.as_ref().is_none_or(|(k, _)| *k != key) {
            let pictures = (0..1 + NEXT)
                .map(|k| {
                    let mut q = self.params.clone();
                    q.seed = q.seed.wrapping_add(k as u64);
                    let image = crate::panels::object_picture(&grow(&q, "preview"));
                    let size = image.size;
                    (ui.ctx().load_texture(format!("grown tree {k}"), image, TextureOptions::NEAREST), size)
                })
                .collect();
            self.pictures = Some((key, pictures));
        }
        let Some((_, pictures)) = &self.pictures else { return };
        let width = ui.available_width().max(300.0);
        let draw = |ui: &Ui, rect: egui::Rect, (texture, [w, h]): &Picture, hovered: bool| {
            ui.painter().rect_filled(rect, theme::TILE_RADIUS, if hovered { pal.accent_soft } else { pal.header });
            // Whole pixels where they fit.
            let fit = ((rect.width() - 12.0) / *w as f32).min((rect.height() - 12.0) / *h as f32);
            let scale = if fit >= 1.0 { fit.floor() } else { fit };
            let at = egui::Rect::from_center_size(rect.center(), vec2(*w as f32, *h as f32) * scale);
            ui.painter().image(texture.id(), at, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        };
        let (rect, _) = ui.allocate_exact_size(vec2(width, 300.0), egui::Sense::hover());
        draw(ui, rect, &pictures[0], false);
        ui.add_space(4.0);
        let side = (width - 4.0 * (NEXT as f32 - 1.0)) / NEXT as f32;
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (k, picture) in pictures.iter().enumerate().skip(1) {
                let (rect, response) = ui.allocate_exact_size(vec2(side, side * 1.2), egui::Sense::click());
                draw(ui, rect, picture, response.hovered());
                let seed = self.params.seed.wrapping_add(k as u64);
                if response.on_hover_text(tr!("Seed {seed}: click to take it", seed)).clicked() {
                    picked = Some(seed);
                }
            }
        });
        if let Some(seed) = picked {
            self.params.seed = seed;
        }
    }
}

/// How many of the next seeds' trees are shown under the tree.
const NEXT: usize = 4;

fn tr(s: &'static str) -> &'static str {
    crate::i18n::tr(s)
}

fn percent(v: &mut f32) -> Field<'_> {
    Field::new(v, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
}

/// `dark_oak` as `Dark oak`.
fn pretty(id: &str) -> String {
    let s = id.replace('_', " ");
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// The trees asked for, and their smaller ones for Tree sizes: as many
/// bushes (two fifths of the height) and small trees (two thirds), grown
/// rather than shrunk.
pub fn trees(p: &TreeParams, name: &str, count: usize) -> (Vec<ObjectDef>, Vec<ObjectDef>) {
    let full = variants(p, name, count);
    let half = count.div_ceil(2);
    let small = (0..count)
        .map(|k| {
            let mut q = p.clone();
            q.seed = p.seed.wrapping_add(k as u64);
            let (scale, what) = if k < half { (0.4, "bush") } else { (0.66, "small") };
            q.height = (p.height * scale).max(4.0);
            q.trunk = (p.trunk * scale.sqrt()).max(1.0);
            q.leaf_size = p.leaf_size * scale.sqrt();
            grow(&q, &format!("{name} {} ({what})", k + 1))
        })
        .collect();
    (full, small)
}

/// Add the trees of `form` to the object layer `into` (its number), or to a
/// new layer when `None`; the layer they went to is picked.
pub fn add(form: &GrowForm, into: Option<u16>, doc: &mut Document, tools: &mut ToolState) {
    let name = form.preset.name();
    let (full, small) = trees(&form.params, name, form.count);
    let reg = &mut doc.world.registry;
    let first = reg.objects.len() as u32;
    let (n_full, n_small) = (full.len() as u32, small.len() as u32);
    reg.objects.extend(full);
    reg.objects.extend(small);
    // The smaller ones only where Tree sizes ask for them.
    let objects: Vec<(u32, u16)> = (first..first + n_full).map(|k| (k, 1)).chain((first + n_full..first + n_full + n_small).map(|k| (k, 0))).collect();
    let layer = into.and_then(|n| n.checked_sub(1)).map(|k| k as usize).filter(|&k| reg.object_layers.get(k).is_some_and(|l| !l.removed));
    let n = match layer {
        Some(k) => {
            reg.object_layers[k].objects.extend(objects);
            k as u16 + 1
        }
        None => {
            let indices: Vec<u32> = objects.iter().map(|o| o.0).collect();
            let mut layer = ObjectLayerDef::new(&format!("{name} (grown)"), &indices, reg);
            layer.objects = objects;
            let reach = (first..first + n_full).map(|k| reg.objects[k as usize].reach()).max().unwrap_or(2);
            layer.spacing = ((reach as f32 * 1.3) as u16 + 2).clamp(3, 256);
            layer.density = 0.75;
            let leaves = if form.params.leaf_density > 0.0 { format!("minecraft:{}_leaves", form.params.leaves) } else { format!("minecraft:{}_log", form.params.wood) };
            layer.color = massif_core::objects::guess_color(&leaves);
            reg.add_object_layer(layer)
        }
    };
    tools.object_layer = n;
    tools.objects_note = Some(trn!("Grew {n} tree", "Grew {n} trees", n = form.count));
    doc.mark_changed();
}
