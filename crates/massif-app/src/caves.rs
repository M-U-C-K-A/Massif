//! Caves in the app: the settings of the Caves and Cave biome tools, and
//! Edit ▸ Caves… — a cave layer painted over the whole map or its selection
//! in one step to undo, with a picture of its caves (a plan at a height
//! picked, and a cut through the land) made on another thread before
//! anything is applied (see `massif_core::caves`).

use crate::doc::Document;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::ToolState;
use crate::widgets::{Field, button_group, row, switch, well};
use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui, vec2};
use massif_core::caves::{CaveDef, CaveLayers, Feature, Fill};
use massif_core::coords::{TILE_AREA, TILE_SIZE, TilePos, local_index};
use massif_core::tile::Tile;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// The game's cave biomes, which the export decorates.
pub const CAVE_BIOMES: [(&str, &str); 3] = [("minecraft:lush_caves", tn!("Lush caves")), ("minecraft:dripstone_caves", tn!("Dripstone caves")), ("minecraft:deep_dark", tn!("Deep dark"))];

/// A new layer of caves as the game's, for the document's dimension, named
/// apart from its other layers.
pub fn vanilla_layer(doc: &Document) -> CaveDef {
    let dim = doc.dimension();
    let defs = dim.caves.defs();
    let count = defs.iter().filter(|d| !d.removed).count();
    let name = if count == 0 { tr!("Caves").to_owned() } else { crate::layers::unique_name(&tr!("Caves {value}", value = count + 1), defs.iter().filter(|d| !d.removed).map(|d| d.name.as_str())) };
    let seed = (doc.world.meta.seed as u32).wrapping_add(count as u32 * 7919) % 1_000_000;
    let mut def = CaveDef::vanilla(&name, seed, dim.min_y, dim.max_y, doc.world.meta.sea_level);
    const COLORS: [[u8; 3]; 5] = [[196, 120, 60], [70, 140, 200], [150, 90, 190], [90, 170, 110], [200, 80, 90]];
    def.color = COLORS[count % COLORS.len()];
    def
}

/// The settings of the caves of a feature: on, density, size, heights.
fn feature(ui: &mut Ui, name: &str, about: &str, f: &mut Feature, (lo, hi): (i32, i32)) {
    let pal = Palette::of(ui);
    well(ui, |ui| {
        row(ui, name, |ui| {
            switch(ui, &mut f.on).on_hover_text(about);
        });
        if f.on {
            row(ui, tr!("Density"), |ui| {
                let mut percent = (f.density * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                    f.density = percent / 100.0;
                }
            });
            row(ui, tr!("Size"), |ui| {
                ui.add(Field::new(&mut f.size, 0.5..=2.0).max_decimals(2).suffix("×"));
            });
            row(ui, tr!("Highest"), |ui| {
                ui.add(Field::new(&mut f.high, lo..=hi).suffix(" y"));
            });
            row(ui, tr!("Lowest"), |ui| {
                ui.add(Field::new(&mut f.low, lo..=hi).suffix(" y"));
            });
            if f.low > f.high {
                std::mem::swap(&mut f.low, &mut f.high);
            }
        } else {
            let _ = pal;
        }
    });
}

/// The settings of cave layer `def` in wells, for a dimension whose blocks
/// go from `lo` to `hi`: its kinds of caves, its roof and openings, its
/// water and lava, its seed. Whether anything changed.
pub fn layer_editor(ui: &mut Ui, def: &mut CaveDef, (lo, hi): (i32, i32)) -> bool {
    let before = def.clone();
    let pal = Palette::of(ui);
    let heading = |ui: &mut Ui, text: &str| {
        ui.add_space(6.0);
        ui.label(RichText::new(text).size(11.5).color(pal.weak).medium());
        ui.add_space(2.0);
    };
    heading(ui, tr!("Caves"));
    feature(ui, tr!("Chambers"), tr!("Large rooms with pillars of rock standing in them, the game's cheese caves"), &mut def.cheese, (lo, hi));
    ui.add_space(4.0);
    feature(ui, tr!("Tunnels"), tr!("Long winding tunnels, the game's spaghetti caves"), &mut def.spaghetti, (lo, hi));
    ui.add_space(4.0);
    feature(ui, tr!("Noodles"), tr!("Thin tunnels, many of them"), &mut def.noodles, (lo, hi));
    ui.add_space(4.0);
    feature(ui, tr!("Ravines"), tr!("Long, deep, narrow cuts"), &mut def.ravines, (lo, hi));
    heading(ui, tr!("Surface"));
    well(ui, |ui| {
        row(ui, tr!("Roof"), |ui| {
            let mut roof = def.roof as i32;
            if ui.add(Field::new(&mut roof, 1..=32).suffix(tr!(" blocks"))).on_hover_text(tr!("The rock kept over the caves, under the ground's surface")).changed() {
                def.roof = roof.clamp(1, 32) as u8;
            }
        });
        row(ui, tr!("Open to the sky"), |ui| {
            let mut percent = (def.openings * 100.0).round();
            if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text(tr!("The share of dry land where caves may open, away from water")).changed() {
                def.openings = percent / 100.0;
            }
        });
    });
    heading(ui, tr!("Water and lava"));
    let aq = &mut def.aquifers;
    well(ui, |ui| {
        row(ui, tr!("Aquifers"), |ui| {
            switch(ui, &mut aq.on).on_hover_text(tr!("Lakes of water, and of lava deep down, each at a level of its own, walled off from each other by rock"));
        });
        if aq.on {
            row(ui, tr!("Lakes"), |ui| {
                let mut percent = (aq.density * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text(tr!("The share of the caves' zones that hold water or lava")).changed() {
                    aq.density = percent / 100.0;
                }
            });
            row(ui, tr!("Size"), |ui| {
                ui.add(Field::new(&mut aq.size, 0.5..=2.0).max_decimals(2).suffix("×"));
            });
            row(ui, tr!("Highest"), |ui| {
                ui.add(Field::new(&mut aq.high, lo..=hi).suffix(" y"));
            });
            row(ui, tr!("Lowest"), |ui| {
                ui.add(Field::new(&mut aq.low, lo..=hi).suffix(" y"));
            });
            if aq.low > aq.high {
                std::mem::swap(&mut aq.low, &mut aq.high);
            }
            row(ui, tr!("Lava lakes"), |ui| {
                let mut percent = (aq.lava * 100.0).round();
                if ui.add(Field::new(&mut percent, 0.0..=100.0).max_decimals(0).suffix(" %")).on_hover_text(tr!("The share of the lakes under the height below that hold lava")).changed() {
                    aq.lava = percent / 100.0;
                }
            });
            row(ui, tr!("Lava under"), |ui| {
                ui.add(Field::new(&mut aq.lava_under, lo..=hi).suffix(" y"));
            });
            row(ui, tr!("Lava floor"), |ui| {
                let mut on = aq.lava_floor.is_some();
                if switch(ui, &mut on).on_hover_text(tr!("Caves full of lava under a height, as the game's under y −54")).changed() {
                    aq.lava_floor = on.then_some(lo + 10);
                }
                if let Some(y) = &mut aq.lava_floor {
                    ui.add(Field::new(y, lo..=hi).suffix(" y"));
                }
            });
        }
    });
    ui.add_space(6.0);
    well(ui, |ui| {
        row(ui, tr!("Seed"), |ui| {
            if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text(tr!("Another seed")).clicked() {
                def.seed = def.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 1_000_000;
            }
            ui.add(Field::new(&mut def.seed, 0..=999_999));
        });
    });
    *def != before
}

/// Tool Settings of the Caves tool: the dimension's cave layers to pick
/// from, a new one, and the settings of the one picked.
pub fn cave_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    let pal = Palette::of(ui);
    let mut defs = doc.dimension().caves.to_vec();
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, d) in defs.iter().enumerate() {
        if d.removed {
            continue;
        }
        let n = k as u16 + 1;
        let selected = tools.cave_layer == n;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
        let fill = if selected {
            pal.accent_soft
        } else if response.hovered() {
            pal.header
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, theme::TILE_RADIUS, fill, if selected { egui::Stroke::new(1.0, pal.accent) } else { egui::Stroke::NONE }, egui::StrokeKind::Inside);
        let dot = egui::Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(11.0, 11.0));
        painter.rect(dot, 3, theme::rgb(d.color), egui::Stroke::new(1.0, Color32::from_black_alpha(60)), egui::StrokeKind::Inside);
        painter.text(rect.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, &d.name, egui::FontId::proportional(12.5), pal.text);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &d.name));
        if response.clicked() {
            picked = Some(n);
        }
    }
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(n) = picked {
        tools.cave_layer = n;
    }
    let mut changed = false;
    if crate::widgets::button(ui, theme::with_icon(Icon::Plus, tr!("New cave layer"))).on_hover_text(tr!("Caves as the game's, to paint and set as you like")).clicked() {
        defs.push(vanilla_layer(doc));
        tools.cave_layer = defs.len() as u16;
        changed = true;
    }
    let (lo, hi) = (doc.dimension().min_y + 1, doc.dimension().max_y - 1);
    match tools.cave_layer.checked_sub(1).map(usize::from).filter(|&k| defs.get(k).is_some_and(|d| !d.removed)) {
        Some(k) => {
            ui.add_space(4.0);
            crate::panels::section(ui, Icon::Shovel, &defs[k].name.clone());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut defs[k].name).desired_width((ui.available_width() - 40.0).max(30.0)));
                let c = defs[k].color;
                let mut color = Color32::from_rgb(c[0], c[1], c[2]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut color, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour over the map while it is painted")).changed() {
                    defs[k].color = [color.r(), color.g(), color.b()];
                }
            });
            changed |= layer_editor(ui, &mut defs[k], (lo, hi));
        }
        None if defs.iter().all(|d| d.removed) => crate::panels::hint(ui, tr!("No caves yet: make a layer of them to paint.")),
        None => crate::panels::hint(ui, tr!("Pick a layer to paint.")),
    }
    if changed || defs != doc.dimension().caves.defs() {
        let dim = doc.dim;
        doc.world.dims[dim].caves = CaveLayers::new(defs);
        doc.caves_changed = true;
        doc.mark_changed();
    }
}

/// Tool Settings of the Cave biome tool: the biome, and the heights it is
/// painted between.
pub fn cave_biome_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    let reg = &doc.world.registry;
    let ids: Vec<Option<u16>> = CAVE_BIOMES.iter().map(|(id, _)| reg.biome_by_id(id).map(|b| b.0)).collect();
    if tools.cave_biome == 0 || !ids.contains(&Some(tools.cave_biome)) && reg.biomes.get(tools.cave_biome as usize).is_none() {
        tools.cave_biome = ids[0].unwrap_or(0);
    }
    let labels: Vec<&str> = CAVE_BIOMES.iter().map(|(_, name)| crate::i18n::tr_name(name)).collect();
    let on: Vec<bool> = ids.iter().map(|&id| id == Some(tools.cave_biome)).collect();
    if let Some(k) = button_group(ui, "cave biome", &labels, &on)
        && let Some(id) = ids[k]
    {
        tools.cave_biome = id;
    }
    let (lo, hi) = (doc.dimension().min_y, doc.dimension().max_y - 1);
    ui.add_space(6.0);
    well(ui, |ui| {
        row(ui, tr!("Highest"), |ui| {
            ui.add(Field::new(&mut tools.cave_high, lo..=hi).suffix(" y"));
        });
        row(ui, tr!("Lowest"), |ui| {
            ui.add(Field::new(&mut tools.cave_low, lo..=hi).suffix(" y"));
        });
    });
    crate::panels::hint(ui, tr!("The game keeps a biome every 4 blocks each way: so does the painting. The export decorates the caves of each: moss and vines, dripstone, sculk."));
}

// ------------------------------------------------------------ the dialog --

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    /// Paint cave layer `layer` (`None`: `def` as a new layer) with the
    /// settings `def`.
    Apply { layer: Option<u16>, def: CaveDef },
}

/// The tiles of the map pictured, with what the picture is made of.
struct Sample {
    /// Its ground tiles, painted with the layer where the caves would go.
    tiles: Vec<(TilePos, Tile)>,
    min_y: i32,
    max_y: i32,
}

/// The pictures of the caves: a plan at a height, a cut across.
struct Pictures {
    plan: TextureHandle,
    cut: TextureHandle,
}

/// Edit ▸ Caves…: a cave layer, new or the map's, painted over the whole
/// map or its selection.
pub struct CaveForm {
    /// The layer: one of the map's by number, or `None` for a new one.
    pub layer: Option<u16>,
    pub def: CaveDef,
    /// The map's layers, to pick from.
    layers: Vec<(u16, CaveDef)>,
    new: CaveDef,
    heights: (i32, i32),
    selected: bool,
    side: i32,
    tiles: usize,
    /// The height of the plan.
    pub plan_y: i32,
    sample: Arc<Sample>,
    made_for: Option<String>,
    changed: Option<(f64, String)>,
    rx: Option<Receiver<(ColorImage, ColorImage)>>,
    cancel: Arc<AtomicBool>,
    pictures: Option<Pictures>,
}

/// The pictures' size: two tiles across, one down.
const PLAN: [usize; 2] = [256, 128];
const CUT_H: usize = 128;

impl CaveForm {
    pub fn new(doc: &Document, start: Option<CaveDef>) -> Self {
        let dim = doc.dimension();
        let layers: Vec<(u16, CaveDef)> = dim.caves.defs().iter().enumerate().filter(|(_, d)| !d.removed).map(|(k, d)| (k as u16 + 1, d.clone())).collect();
        let new = start.unwrap_or_else(|| vanilla_layer(doc));
        let side = dim.bounds().map_or(0, |(a, b)| (b.x - a.x + 1).max(b.z - a.z + 1) * TILE_SIZE);
        // Two tiles of the map: the middle of the selection, else of the map.
        let middle = match &dim.selection {
            Some(sel) => sel.bounds().map(|r| ((r.x0 + r.x1) / 2, (r.z0 + r.z1) / 2)),
            None => dim.bounds().map(|(a, b)| ((a.x + b.x + 1) * TILE_SIZE / 2, (a.z + b.z + 1) * TILE_SIZE / 2)),
        }
        .unwrap_or((0, 0));
        let first = TilePos::containing(middle.0 - TILE_SIZE / 2, middle.1);
        let tiles = [first, TilePos::new(first.x + 1, first.z)]
            .into_iter()
            .filter_map(|p| {
                let mut tile = Tile::clone(&*dim.ground().view(p)?);
                tile.caves = massif_core::Plane::Uniform(1);
                Some((p, tile))
            })
            .collect();
        let sample = Arc::new(Sample { tiles, min_y: dim.min_y, max_y: dim.max_y });
        let sea = doc.world.meta.sea_level;
        Self {
            layer: None,
            def: new.clone(),
            layers,
            new,
            heights: (dim.min_y, dim.max_y),
            selected: dim.selection.is_some(),
            side,
            tiles: dim.ground().len(),
            plan_y: (sea - 40).max(dim.min_y + 16),
            sample,
            made_for: None,
            changed: None,
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            pictures: None,
        }
    }

    /// Whether the pictures are there (for tests).
    #[cfg(test)]
    pub fn has_pictures(&self) -> bool {
        self.pictures.is_some()
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Shovel).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Caves")).size(16.0).bold());
        });
        ui.add(egui::Label::new(RichText::new(tr!("Chambers, tunnels, ravines and lakes of water and lava, as the game carves them since 1.18: a cave layer painted over the map, its settings to change at any time with the Caves tool.")).size(12.0).color(pal.weak)).wrap());
        ui.add_space(10.0);
        let (lo, hi) = (self.heights.0 + 1, self.heights.1 - 1);
        ui.horizontal_top(|ui| {
            // The settings, in a column that scrolls.
            ui.vertical(|ui| {
                ui.set_width(330.0);
                // Which layer: a new one, or one of the map's.
                let mut labels: Vec<String> = vec![tr!("New layer").to_owned()];
                labels.extend(self.layers.iter().map(|(_, d)| d.name.clone()));
                let names: Vec<&str> = labels.iter().map(String::as_str).collect();
                let on: Vec<bool> = std::iter::once(self.layer.is_none()).chain(self.layers.iter().map(|(n, _)| self.layer == Some(*n))).collect();
                if names.len() > 1
                    && let Some(k) = button_group(ui, "cave layer", &names[..names.len().min(4)], &on[..on.len().min(4)])
                {
                    if k == 0 {
                        self.layer = None;
                        self.def = self.new.clone();
                    } else {
                        let (n, d) = &self.layers[k - 1];
                        self.layer = Some(*n);
                        self.def = d.clone();
                    }
                }
                egui::ScrollArea::vertical().max_height(400.0).id_salt("cave settings").show(ui, |ui| {
                    layer_editor(ui, &mut self.def, (lo, hi));
                });
                if self.layer.is_none() {
                    self.new = self.def.clone();
                }
            });
            ui.add_space(12.0);
            self.pictures(ui, (lo, hi));
        });
        ui.add_space(10.0);
        let over = if self.selected { tr!("Over the selection, by its weight").to_owned() } else { tr!("Over the whole map, {side} blocks across ({tiles} tiles)", side = self.side, tiles = self.tiles) };
        ui.label(RichText::new(tr!("{over}; one step to undo. The caves keep a roof of rock under the ground but where they open to the sky.", over)).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(self.def.carves(), theme::primary_button(ui, tr!("Paint the caves"))).clicked() {
                choice = Choice::Apply { layer: self.layer, def: self.def.clone() };
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

    /// The plan and the cut, made again once the settings settle.
    fn pictures(&mut self, ui: &mut Ui, (lo, hi): (i32, i32)) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let key = format!("{:?}{}", self.def, self.plan_y);
        if self.made_for.as_deref() != Some(&key) && !self.sample.tiles.is_empty() {
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
                let (sample, def, y, ctx) = (self.sample.clone(), self.def.clone(), self.plan_y, ctx.clone());
                std::thread::Builder::new()
                    .name("massif caves preview".into())
                    .spawn(move || {
                        if let Some(made) = preview(&sample, &def, y, &cancel) {
                            let _ = tx.send(made);
                        }
                        ctx.request_repaint();
                    })
                    .expect("a thread for the preview");
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
        if let Some(rx) = &self.rx
            && let Ok((plan, cut)) = rx.try_recv()
        {
            self.rx = None;
            self.pictures = Some(Pictures { plan: ctx.load_texture("caves plan", plan, TextureOptions::NEAREST), cut: ctx.load_texture("caves cut", cut, TextureOptions::NEAREST) });
        }
        let working = self.rx.is_some();
        ui.vertical(|ui| {
            let width = 384.0;
            let picture = |ui: &mut Ui, title: &str, texture: Option<&TextureHandle>, h: f32| {
                ui.label(RichText::new(title).size(11.5).color(pal.weak).medium());
                ui.add_space(2.0);
                let (rect, _) = ui.allocate_exact_size(vec2(width, h), egui::Sense::hover());
                ui.painter().rect_filled(rect, theme::TILE_RADIUS, pal.well());
                if let Some(t) = texture {
                    egui::Image::new(t).corner_radius(theme::TILE_RADIUS).paint_at(ui, rect);
                }
                ui.painter().rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                rect
            };
            let y = self.plan_y;
            let rect = picture(ui, &tr!("The caves at y {y}, from above", y), self.pictures.as_ref().map(|p| &p.plan), width / 2.0);
            if working {
                let spinner = egui::Rect::from_center_size(rect.right_top() + vec2(-15.0, 15.0), vec2(16.0, 16.0));
                crate::loader::paint(ui, spinner, crate::loader::pattern(ui.ctx()));
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("Height")).size(11.5).color(pal.weak));
                ui.add(Field::new(&mut self.plan_y, lo..=hi).suffix(" y"));
            });
            ui.add_space(6.0);
            picture(ui, tr!("A cut through the land"), self.pictures.as_ref().map(|p| &p.cut), width / 2.0);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (color, name) in [(AIR, tr!("Cave")), (WATER, tr!("Water")), (LAVA, tr!("Lava")), (ROCK, tr!("Rock"))] {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2.0, color);
                    ui.label(RichText::new(name).size(11.0).color(pal.weak));
                    ui.add_space(4.0);
                }
            });
        });
    }
}

/// The colours of the pictures: the game's blocks, not the theme's.
const ROCK: Color32 = Color32::from_rgb(128, 128, 126);
const DEEP: Color32 = Color32::from_rgb(78, 78, 84);
const AIR: Color32 = Color32::from_rgb(26, 24, 30);
const WATER: Color32 = Color32::from_rgb(52, 98, 200);
const LAVA: Color32 = Color32::from_rgb(226, 104, 36);
const SKY: Color32 = Color32::from_rgb(150, 186, 220);

/// The plan at height `y` and the cut through the middle of `sample`, its
/// tiles painted with `def`; `None` once cancelled.
fn preview(sample: &Sample, def: &CaveDef, y: i32, cancel: &AtomicBool) -> Option<(ColorImage, ColorImage)> {
    let layers = CaveLayers::new(vec![def.clone()]);
    let mut plan = ColorImage::new(PLAN, vec![Color32::BLACK; PLAN[0] * PLAN[1]]);
    let top = sample.tiles.iter().map(|(_, t)| (0..TILE_AREA).map(|i| massif_core::compose::block_top(t.top.get(i))).max().unwrap_or(0)).max().unwrap_or(0);
    let (y0, y1) = (sample.min_y, (top + 8).min(sample.max_y));
    let mut cut = ColorImage::new([PLAN[0], CUT_H], vec![SKY; PLAN[0] * CUT_H]);
    let row = 64;
    for (k, (pos, tile)) in sample.tiles.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let caves = massif_core::caves::tile_caves(&layers, tile, *pos, sample.min_y, sample.max_y, &[]);
        let column = |cell: usize| caves.as_ref().map_or(&[][..], |c| c.column(cell));
        for lz in 0..TILE_SIZE {
            for lx in 0..TILE_SIZE {
                let cell = local_index(lx, lz);
                let ground = massif_core::compose::block_top(tile.top.get(cell));
                let runs = column(cell);
                let color = if y >= ground {
                    SKY
                } else {
                    match runs.iter().find(|r| (r.y0 as i32..r.y1 as i32).contains(&y)) {
                        Some(r) => {
                            // Darker where the cave goes deep under the plan.
                            let depth = (y - r.y0 as i32).clamp(0, 24) as f32 / 24.0;
                            match r.fill {
                                Fill::Air => theme::mix(Color32::from_rgb(70, 64, 78), AIR, depth),
                                Fill::Water => WATER,
                                Fill::Lava => LAVA,
                            }
                        }
                        None => theme::mix(ROCK, DEEP, ((ground - y) as f32 / 120.0).clamp(0.0, 1.0)),
                    }
                };
                plan.pixels[lz as usize * PLAN[0] + k * TILE_SIZE as usize + lx as usize] = color;
            }
        }
        // The cut, along the middle row.
        for lx in 0..TILE_SIZE {
            let cell = local_index(lx, row);
            let ground = massif_core::compose::block_top(tile.top.get(cell));
            let runs = column(cell);
            for py in 0..CUT_H {
                let yy = y1 - 1 - ((py as i32) * (y1 - y0) / CUT_H as i32);
                let color = if yy >= ground {
                    SKY
                } else {
                    match runs.iter().find(|r| (r.y0 as i32..r.y1 as i32).contains(&yy)) {
                        Some(r) => match r.fill {
                            Fill::Air => AIR,
                            Fill::Water => WATER,
                            Fill::Lava => LAVA,
                        },
                        None => theme::mix(ROCK, DEEP, ((ground - yy) as f32 / 160.0).clamp(0.0, 1.0)),
                    }
                };
                cut.pixels[py * PLAN[0] + k * TILE_SIZE as usize + lx as usize] = color;
            }
        }
    }
    // The height of the plan, marked across the cut.
    if (y0..y1).contains(&y) {
        let py = (((y1 - 1 - y) as i64 * CUT_H as i64) / (y1 - y0) as i64).clamp(0, CUT_H as i64 - 1) as usize;
        for x in (0..PLAN[0]).step_by(2) {
            cut.pixels[py * PLAN[0] + x] = Color32::from_rgb(250, 210, 80);
        }
    }
    Some((plan, cut))
}
