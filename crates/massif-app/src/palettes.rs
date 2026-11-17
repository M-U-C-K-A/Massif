//! Block palettes: the window where they are made and changed.
//!
//! A palette (see `massif_core::materials`) says what the built-in
//! terrains of a map are made of: the generator's grass, rock, sand, gravel
//! and snow, the other natural terrains, and the rock under them. Those
//! that come with Massif (Vanilla, and those of the mods read whose
//! profiles bring one) are shown as they are and duplicated to be changed; the player's own are kept in the settings.
//! Each terrain is a few layers from the surface down, each of blocks with
//! their weights, picked from the game's and the mods' blocks or typed.

use crate::blocks::BlockLibrary;
use crate::theme::{self, Icon, Palette as Colors, RichTextExt};
use crate::widgets::{self, Field};
use egui::{Align, Color32, Layout, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::materials::{self, DEEP, GENERATED, Layer, Palette, Recipe};

const WIDTH: f32 = 860.0;
const LIST_W: f32 = 210.0;
/// The most blocks the picker shows at once.
const PICKS: usize = 160;

/// The palettes window.
pub struct PaletteEditor {
    /// The palettes that come with Massif, offered now.
    shipped: Vec<Palette>,
    /// The player's own palettes.
    pub own: Vec<Palette>,
    /// The one shown: an index in [`PaletteEditor::all`].
    pub selected: usize,
    /// The terrain shown (`None`: the rock).
    pub slot: Option<String>,
    /// The layer picked blocks go to.
    pub layer: usize,
    pub search: String,
    pub typed: String,
    problem: Option<String>,
    /// A palette read from the map, offered to be kept.
    of_map: Palette,
}

/// What the window asks of the application.
pub enum Choice {
    Keep,
    /// Closed: the player's palettes, to keep.
    Close(Vec<Palette>),
}

impl PaletteEditor {
    /// The window over the palettes that come with Massif `shipped` and the
    /// player's `own`, `shown` first if it is one of them; `of_map` is the
    /// map's terrains read as a palette.
    pub fn new(shipped: Vec<Palette>, own: Vec<Palette>, shown: Option<&str>, of_map: Palette) -> Self {
        let mut me = Self { shipped, own, selected: 0, slot: Some("Grass".into()), layer: 0, search: String::new(), typed: String::new(), problem: None, of_map };
        if let Some(name) = shown {
            me.selected = me.all().iter().position(|p| p.name == name).unwrap_or(0);
        }
        me
    }

    /// The palettes that come with Massif, then the player's.
    pub fn all(&self) -> Vec<Palette> {
        self.shipped.iter().chain(&self.own).cloned().collect()
    }

    fn built_ins(&self) -> usize {
        self.shipped.len()
    }

    /// The player's palette shown, to change.
    fn own_mut(&mut self) -> Option<&mut Palette> {
        let i = self.selected.checked_sub(self.built_ins())?;
        self.own.get_mut(i)
    }

    /// A name no palette has: `name`, else `name 2`, `name 3`…
    fn free_name(&self, name: &str) -> String {
        let taken = |n: &str| self.all().iter().any(|p| p.name == n);
        if !taken(name) {
            return name.to_owned();
        }
        (2..).map(|k| format!("{name} {k}")).find(|n| !taken(n)).unwrap_or_default()
    }

    /// Keep `palette` among the player's, under a free name, and show it.
    fn add(&mut self, mut palette: Palette, name: &str) {
        palette.name = self.free_name(name);
        self.own.push(palette);
        self.selected = self.built_ins() + self.own.len() - 1;
    }

    pub fn ui(&mut self, ui: &mut Ui, blocks: &mut BlockLibrary) -> Choice {
        let pal = Colors::of(ui);
        let mut choice = Choice::Keep;
        self.selected = self.selected.min(self.all().len() - 1);
        ui.set_width(WIDTH);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Palette).to_string()).size(18.0).color(pal.accent));
            ui.label(RichText::new(tr!("Block palettes")).size(19.0).bold());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::primary_button(ui, theme::with_icon(Icon::Check, tr!("Done")))).clicked() {
                    choice = Choice::Close(self.own.clone());
                }
            });
        });
        ui.label(RichText::new(tr!("What the terrains of a new map are made of: the generator's grass, rock, sand, gravel and snow by height and slope, the other natural terrains, and the rock under them. A terrain a palette leaves out is the game's.")).size(12.0).color(pal.weak));
        ui.add_space(10.0);
        let height = (ui.ctx().content_rect().height() - 220.0).clamp(320.0, 620.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(LIST_W, height), Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(vec2(LIST_W, height));
                self.list(ui);
            });
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(vec2(WIDTH - LIST_W - 12.0, height), Layout::top_down(Align::Min), |ui| {
                let w = WIDTH - LIST_W - 12.0;
                ui.set_min_size(vec2(w, height));
                ui.set_max_width(w);
                egui::ScrollArea::vertical().id_salt("palette body").auto_shrink([true, false]).max_width(w).show(ui, |ui| {
                    ui.set_width(w - 14.0);
                    self.body(ui, &pal, blocks)
                });
            });
        });
        choice
    }

    /// The palettes, and what makes new ones.
    fn list(&mut self, ui: &mut Ui) {
        let all = self.all();
        egui::ScrollArea::vertical().id_salt("palette list").max_height(ui.available_height() - 110.0).show(ui, |ui| {
            for (i, p) in all.iter().enumerate() {
                let built_in = i < self.built_ins();
                let text = if built_in { format!("{}  ·  {}", p.name, tr!("built in")) } else { p.name.clone() };
                let on = self.selected == i;
                let button = egui::Button::selectable(on, RichText::new(text).size(12.5)).min_size(vec2(LIST_W - 8.0, 26.0));
                if ui.add(button).clicked() {
                    self.selected = i;
                    self.problem = None;
                }
            }
        });
        ui.add_space(8.0);
        let shown = all[self.selected].clone();
        if ui.button(theme::with_icon(Icon::Copy, tr!("Duplicate"))).on_hover_text(tr!("A palette of your own made of the same blocks, to change")).clicked() {
            let name = tr!("{name} copy", name = shown.name);
            self.add(shown.clone(), &name);
        }
        if ui.button(theme::with_icon(Icon::Map, tr!("From this map"))).on_hover_text(tr!("A palette of your own made of the blocks of this map's terrains")).clicked() {
            let of_map = self.of_map.clone();
            self.add(of_map, tr!("This map's blocks"));
        }
        if self.selected >= self.built_ins() && ui.button(theme::with_icon(Icon::Trash2, tr!("Delete"))).clicked() {
            self.own.remove(self.selected - self.built_ins());
            self.selected -= 1;
        }
    }

    /// The palette shown: its name, a terrain and its layers, the blocks to
    /// pick from.
    fn body(&mut self, ui: &mut Ui, pal: &Colors, blocks: &mut BlockLibrary) {
        let shown = self.all()[self.selected].clone();
        let editable = self.selected >= self.built_ins();
        let missing: Vec<&str> = shown.namespaces().into_iter().filter(|ns| !blocks.catalogue.as_ref().is_some_and(|c| c.mods.iter().any(|m| m.namespaces.iter().any(|n| n == ns)))).collect();
        if editable {
            ui.horizontal(|ui| {
                ui.label(tr!("Name"));
                let mut name = shown.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0)).changed()
                    && !name.trim().is_empty()
                    && !self.all().iter().enumerate().any(|(i, p)| i != self.selected && p.name == name)
                    && let Some(p) = self.own_mut()
                {
                    p.name = name;
                }
            });
        } else {
            ui.label(RichText::new(tr!("A palette that comes with Massif: duplicate it to change it.")).size(12.0).color(pal.weak));
        }
        if !missing.is_empty() {
            ui.label(RichText::new(tr!("Uses blocks of {mods}, which are not read: add the mod in the Blocks panel to see them. They are exported all the same.", mods = missing.join(", "))).size(11.5).color(pal.weak));
        }
        ui.add_space(8.0);
        // The terrain shown.
        let slots = materials::slots();
        let label = |s: &Option<String>| match s {
            None => tr!("Rock under the terrains").to_owned(),
            Some(n) => crate::i18n::tr_name(n).to_owned(),
        };
        ui.horizontal(|ui| {
            ui.label(tr!("Terrain"));
            widgets::combo("palette slot").selected_text(label(&self.slot)).width(240.0).height(420.0).show_ui(ui, |ui| {
                let mut entry = |ui: &mut Ui, slot: Option<String>| {
                    let set = match &slot {
                        Some(n) => shown.terrains.contains_key(n),
                        None => shown.stone.is_some() || shown.deepslate.is_some(),
                    };
                    let text = if set { format!("{}  ●", label(&slot)) } else { label(&slot) };
                    if ui.selectable_label(self.slot == slot, text).clicked() {
                        self.slot = slot;
                        self.layer = 0;
                    }
                };
                ui.label(RichText::new(tr!("Painted by the generator")).size(11.0).color(pal.weak));
                for n in GENERATED {
                    entry(ui, Some(n.to_owned()));
                }
                entry(ui, None);
                ui.separator();
                ui.label(RichText::new(tr!("Other terrains")).size(11.0).color(pal.weak));
                for n in slots.iter().filter(|n| !GENERATED.contains(&n.as_str())) {
                    entry(ui, Some(n.clone()));
                }
            });
            if let Some(name) = self.slot.clone()
                && editable
                && shown.terrains.contains_key(&name)
                && ui.button(tr!("Back to the game's")).clicked()
                && let Some(p) = self.own_mut()
            {
                p.terrains.remove(&name);
            }
        });
        ui.add_space(8.0);
        let mut picked: Option<String> = None;
        match self.slot.clone() {
            Some(name) => {
                let recipe = shown.recipe(&name).unwrap_or(Recipe { strata: Vec::new(), submerged_top: None });
                let mut edited = recipe.clone();
                if !shown.terrains.contains_key(&name) {
                    ui.label(RichText::new(tr!("The game's own: change it to make it the palette's.")).size(11.5).color(pal.weak));
                }
                self.layer = self.layer.min(edited.strata.len().saturating_sub(1));
                let mut remove = None;
                for (k, layer) in edited.strata.iter_mut().enumerate() {
                    if self.layer_ui(ui, pal, blocks, k, layer, editable) {
                        remove = Some(k);
                    }
                }
                if let Some(k) = remove {
                    edited.strata.remove(k);
                }
                if editable && ui.button(theme::with_icon(Icon::Plus, tr!("Layer"))).on_hover_text(tr!("A layer under the others")).clicked() {
                    edited.strata.push(Layer { blocks: Vec::new(), depth: (2, 3), patches: None });
                    self.layer = edited.strata.len() - 1;
                }
                if editable && edited != recipe && let Some(p) = self.own_mut() {
                    p.terrains.insert(name, edited);
                }
            }
            None => {
                let mut stone = shown.stone.clone().unwrap_or_else(|| "minecraft:stone".into());
                let mut deep = shown.deepslate.clone().unwrap_or_else(|| "minecraft:deepslate[axis=y]".into());
                ui.label(RichText::new(tr!("Where caves are carved and ores laid. Pick Above or Under, then a block below.")).size(11.5).color(pal.weak));
                ui.add_space(4.0);
                widgets::well(ui, |ui| {
                    widgets::row(ui, tr!("Above Y 0"), |ui| {
                        if ui.add_enabled(editable, egui::Button::selectable(self.layer == 0, tr!("Above"))).clicked() {
                            self.layer = 0;
                        }
                        ui.label(RichText::new(block_name(blocks, &stone)).size(12.0));
                    });
                    widgets::row(ui, tr!("Under Y 0"), |ui| {
                        if ui.add_enabled(editable, egui::Button::selectable(self.layer == 1, tr!("Under"))).clicked() {
                            self.layer = 1;
                        }
                        ui.label(RichText::new(block_name(blocks, &deep)).size(12.0));
                    });
                });
                if editable {
                    let set = |stone: &mut String, deep: &mut String, layer: usize, s: String| if layer == 0 { *stone = s } else { *deep = s };
                    if let Some(s) = self.pick_ui(ui, pal, blocks) {
                        set(&mut stone, &mut deep, self.layer, s);
                    }
                    if let Some(p) = self.own_mut() {
                        p.stone = (stone != "minecraft:stone").then_some(stone);
                        p.deepslate = (deep != "minecraft:deepslate[axis=y]").then_some(deep);
                    }
                }
                return;
            }
        }
        if editable {
            picked = self.pick_ui(ui, pal, blocks);
        }
        if let (Some(state), Some(name)) = (picked, self.slot.clone()) {
            let layer = self.layer;
            if let Some(p) = self.own_mut() {
                let mut recipe = p.recipe(&name).unwrap_or(Recipe { strata: Vec::new(), submerged_top: None });
                if recipe.strata.is_empty() {
                    recipe.strata.push(Layer { blocks: Vec::new(), depth: (3, 4), patches: None });
                }
                let last = recipe.strata.len() - 1;
                let l = &mut recipe.strata[layer.min(last)];
                match l.blocks.iter_mut().find(|b| b.0 == state) {
                    Some(b) => b.1 = b.1.saturating_add(1),
                    None => l.blocks.push((state, 1)),
                }
                p.terrains.insert(name, recipe);
            }
        }
    }

    /// A layer: its depth and how its blocks mix, then its blocks with
    /// their weights. Returns whether it is to go.
    fn layer_ui(&mut self, ui: &mut Ui, pal: &Colors, blocks: &mut BlockLibrary, k: usize, layer: &mut Layer, editable: bool) -> bool {
        let mut gone = false;
        let on = self.layer == k;
        let frame = egui::Frame::new()
            .fill(pal.well())
            .stroke(Stroke::new(if on && editable { 1.5 } else { 1.0 }, if on && editable { pal.accent } else { pal.border }))
            .corner_radius(theme::GROUP_RADIUS)
            .inner_margin(egui::Margin::symmetric(12, 8));
        let response = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    let title = if k == 0 { tr!("Surface").to_owned() } else { tr!("Layer {n}", n = k + 1) };
                    ui.label(RichText::new(title).medium());
                    let deep = layer.depth.1 == DEEP;
                    if deep {
                        ui.label(RichText::new(tr!("all the way down")).size(12.0).color(pal.weak));
                    } else {
                        ui.add(Field::new(&mut layer.depth.0, 1..=64).text(tr!("Depth")).width(110.0)).on_hover_text(tr!("How many blocks thick, from column to column"));
                        ui.add(Field::new(&mut layer.depth.1, 1..=64).text(tr!("to")).width(80.0));
                    }
                    let mut patches = layer.patches.is_some();
                    widgets::checkbox(ui, &mut patches, tr!("In patches")).on_hover_text(tr!("Its blocks in patches rather than each on its own"));
                    match (patches, &mut layer.patches) {
                        (true, Some(size)) => {
                            ui.add(Field::new(size, 1.0..=64.0).text(tr!("Size")).max_decimals(0).width(100.0));
                        }
                        (true, p @ None) => *p = Some(5.0),
                        (false, p) => *p = None,
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button(theme::glyph(Icon::X).to_string()).on_hover_text(tr!("Remove the layer")).clicked() {
                            gone = true;
                        }
                    });
                });
                ui.add_space(4.0);
                let total: u32 = layer.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
                let mut remove = None;
                for (i, (state, weight)) in layer.blocks.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        block_icon(ui, blocks, state, 20.0);
                        ui.label(RichText::new(block_name(blocks, state)).size(12.5)).on_hover_text(state.as_str());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button(theme::glyph(Icon::X).to_string()).on_hover_text(tr!("Remove the block")).clicked() {
                                remove = Some(i);
                            }
                            ui.label(RichText::new(format!("{}%", *weight as u32 * 100 / total)).size(12.0).color(pal.weak));
                            ui.add(Field::new(weight, 1..=100).text(tr!("Weight")).width(110.0));
                        });
                    });
                }
                if let Some(i) = remove {
                    layer.blocks.remove(i);
                }
                if layer.blocks.is_empty() {
                    ui.label(RichText::new(tr!("No block yet: pick one below")).size(12.0).color(pal.weak));
                }
            });
        });
        if editable && response.response.interact(Sense::click()).clicked() {
            self.layer = k;
        }
        ui.add_space(6.0);
        gone
    }

    /// The blocks to pick from: the game's and the mods' found by a search,
    /// and an id typed by hand. Returns the state picked.
    fn pick_ui(&mut self, ui: &mut Ui, pal: &Colors, blocks: &mut BlockLibrary) -> Option<String> {
        let mut picked = None;
        ui.add_space(6.0);
        ui.label(RichText::new(tr!("Add blocks to the layer picked")).medium());
        ui.horizontal(|ui| {
            let typed = ui.add(egui::TextEdit::singleline(&mut self.typed).hint_text(tr!("A block id: mod:block[property=value]")).desired_width(300.0));
            let enter = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(theme::with_icon(Icon::Plus, tr!("Add"))).clicked() || enter {
                match massif_assets::parse_state(&self.typed) {
                    Ok(state) => {
                        picked = Some(state);
                        self.typed.clear();
                        self.problem = None;
                    }
                    Err(_) => self.problem = Some(tr!("“{text}” is not a block id", text = self.typed.trim())),
                }
            }
        });
        if let Some(p) = &self.problem {
            ui.label(RichText::new(p).size(11.5).color(pal.danger));
        }
        let Some(cat) = blocks.catalogue.clone() else {
            ui.label(RichText::new(tr!("The game's blocks are not read: type an id, or choose the game in the Blocks panel.")).size(11.5).color(pal.weak));
            return picked;
        };
        crate::widgets::search(ui, &mut self.search, &tr!("Search {n} blocks", n = cat.blocks.len()), 300.0);
        let needle = self.search.to_lowercase();
        let indices: Vec<u32> = blocks.sections(&needle).iter().flat_map(|s| s.blocks.iter().copied()).filter(|&i| cat.blocks[i as usize].full_cube).take(PICKS).collect();
        const CELL: f32 = 34.0;
        const GAP: f32 = 4.0;
        let width = ui.available_width();
        let cols = (((width + GAP) / (CELL + GAP)).floor() as usize).max(1);
        let rows = indices.len().div_ceil(cols);
        let (grid, _) = ui.allocate_exact_size(vec2(width, rows as f32 * (CELL + GAP)), Sense::hover());
        let ctx = ui.ctx().clone();
        for (n, &i) in indices.iter().enumerate() {
            let block = &cat.blocks[i as usize];
            let rect = egui::Rect::from_min_size(grid.min + vec2((n % cols) as f32 * (CELL + GAP), (n / cols) as f32 * (CELL + GAP)), vec2(CELL, CELL));
            let response = ui.interact(rect, ui.scope_id().with(("palette pick", i)), Sense::click());
            let fill = if response.hovered() { pal.header } else { Color32::TRANSPARENT };
            ui.painter().rect(rect, theme::TILE_RADIUS, fill, Stroke::NONE, StrokeKind::Inside);
            if let Some(texture) = blocks.icon(&ctx, block) {
                ui.painter().image(texture, rect.shrink(4.0), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if response.on_hover_text(format!("{}\n{}", block.name, block.id)).clicked() {
                picked = Some(block.id.clone());
            }
        }
        picked
    }
}

/// A block state's name as the game shows it, else the state.
fn block_name(blocks: &BlockLibrary, state: &str) -> String {
    let id = state.split('[').next().unwrap_or(state);
    blocks.catalogue.as_ref().and_then(|c| c.block(id)).map_or_else(|| state.to_owned(), |b| b.name.clone())
}

/// A block's icon, `size` points wide, when the game's blocks are read.
fn block_icon(ui: &mut Ui, blocks: &mut BlockLibrary, state: &str, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let id = state.split('[').next().unwrap_or(state);
    let ctx = ui.ctx().clone();
    let block = blocks.catalogue.clone().and_then(|c| c.block(id).cloned());
    if let Some(block) = block
        && let Some(texture) = blocks.icon(&ctx, &block)
    {
        ui.painter().image(texture, rect, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// What [`choose`] picked.
pub enum Picked {
    Use(Palette),
    /// Open the palettes window.
    Edit,
}

/// A menu of the palettes `all`, `current` shown, and a way to the window
/// where they are made.
pub fn choose(ui: &mut Ui, id: &str, all: &[Palette], current: &Palette, width: f32) -> Option<Picked> {
    let mut picked = None;
    let shown = if current.is_vanilla() { tr!("Vanilla").to_owned() } else { current.name.clone() };
    widgets::combo(id).selected_text(shown).width(width).truncate().height(360.0).show_ui(ui, |ui| {
        for p in all {
            let name = if p.is_vanilla() { tr!("Vanilla").to_owned() } else { p.name.clone() };
            // Shown picked by its name: one changed since is put on again.
            let on = p.name == current.name || (p.is_vanilla() && current.is_vanilla());
            if ui.selectable_label(on, name).clicked() && p != current {
                picked = Some(Picked::Use(p.clone()));
            }
        }
        ui.separator();
        if ui.selectable_label(false, tr!("Edit palettes…")).clicked() {
            picked = Some(Picked::Edit);
        }
    });
    picked
}

/// The game a palette's blocks are for, as the new map's summary says it.
pub fn game_of(palette: &Palette) -> String {
    if palette.is_vanilla() {
        return tr!("Minecraft Java 1.18+").to_owned();
    }
    // The version its mods' profiles name, if any.
    let version = massif_core::mod_profiles::minecraft_of(palette).unwrap_or("1.18+");
    format!("{}  ·  Minecraft Java {version}", palette.name)
}
