//! What is under the cursor, as WorldPainter's Info panel has it: one
//! column of the map — where, how high, under how much water, its terrain
//! and biome, whether it is selected — and every layer painted on it. The
//! Info panel shows it as the cursor moves; the filters' eyedropper offers
//! it to pick from.

use crate::map_render::{ColumnSampler, FLAG_SLABS};
use crate::theme::{self, Icon, Palette};
use egui::{RichText, Ui, vec2};
use massif_core::compose::TileStack;
use massif_core::coords::local_index;
use massif_core::filter::{Match, Painted};
use massif_core::registry::{BiomeId, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::TilePos;

/// A layer painted on the column.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerHere {
    /// What a filter matches it with; `None` for what no filter tests (a
    /// tree size, smoothed steps, a combined layer).
    pub matches: Option<Match>,
    /// What kind of layer it is, said before its name.
    pub kind: &'static str,
    pub name: String,
    pub color: [u8; 3],
    /// Its intensity, 0 to 1, where the layer has one: the share of the
    /// columns a cover grows on, of the squares a tree stands in, of the
    /// rock the pockets take.
    pub intensity: Option<f32>,
    /// What the Eyedropper tool paints with when it is picked.
    pub paint: Option<Paint>,
}

/// What the Eyedropper tool can pick to paint with: the tool it takes and
/// what that tool paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Terrain(TerrainId),
    Biome(BiomeId),
    Cover(u16),
    Objects(u16),
    Size(u16),
    Pockets(u16),
    Annotation(u16),
    Combined(u16),
}

/// One column of the map, with all it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub x: i32,
    pub z: i32,
    /// The block one stands on, `None` where the column is empty.
    pub height: Option<i32>,
    pub terrain: Option<TerrainId>,
    /// The water's surface (the block over its top one) and its depth.
    pub water: Option<(i32, i32)>,
    /// The biome painted, `None` when it is left on automatic.
    pub biome: Option<BiomeId>,
    /// On automatic, the biome the land calls for.
    pub auto_biome: Option<BiomeId>,
    /// The levels that have the column, by name.
    pub levels: Vec<String>,
    /// The layers painted on it, in the order of the Layers panel.
    pub layers: Vec<LayerHere>,
    /// How much the selection takes the column (0 to 1); `None` with no
    /// selection.
    pub selected: Option<f32>,
}

impl ColumnInfo {
    /// The column at `(x, z)` of the dimension shown, `None` off the map.
    /// `auto`: the reader of the automatic biomes, to say which one the
    /// land calls for where none is painted.
    pub fn read(doc: &crate::doc::Document, x: i32, z: i32) -> Option<Self> {
        let dim = doc.dimension();
        let reg = &doc.world.registry;
        let stack = TileStack::new(dim, TilePos::containing(x, z), false)?;
        let cell = local_index(x, z);
        let s = ColumnSampler::default().sample(&stack, cell);
        let empty = s.is_empty();
        let height = (!empty).then(|| massif_core::compose::block_top(s.height));
        let water = (!empty && s.fluid != NO_FLUID).then(|| (s.fluid as i32, s.fluid as i32 - height.unwrap_or(0)));
        let biome = (s.biome != 0).then_some(BiomeId(s.biome));
        // As the map's Biome view and its footer have it.
        let auto_biome = (biome.is_none() && !empty).then(|| BiomeId(crate::map_render::AutoBiome::new(reg).resolve(reg, &s, doc.world.meta.sea_level as f32)));
        let levels = stack.layers().filter(|(_, _, t)| t.present(cell)).map(|(i, _, _)| dim.levels[i].name.clone()).collect();
        let mut layers = Vec::new();
        if let Some(c) = reg.cover(s.cover) {
            layers.push(LayerHere { matches: Some(Match::Layer(Painted::Cover, s.cover)), kind: tr!("Cover"), name: c.name.clone(), color: c.color, intensity: Some(c.density), paint: Some(Paint::Cover(s.cover)) });
        }
        // The covers stacked under it (a WorldPainter map's), top first.
        let top = (!empty).then(|| stack.spans_top(cell)).flatten();
        if let Some(top) = &top {
            for n in stack.surface_under(cell, top).into_iter().rev() {
                if let Some(c) = reg.cover(n) {
                    layers.push(LayerHere { matches: Some(Match::Layer(Painted::Cover, n)), kind: tr!("Cover under it"), name: c.name.clone(), color: c.color, intensity: Some(c.density), paint: Some(Paint::Cover(n)) });
                }
            }
        }
        for (plane, under, n, kind) in [(Painted::Objects, false, s.objects, tr!("Trees")), (Painted::Understory, true, s.understory, tr!("Under the trees"))] {
            // Every tree layer of the slot here, with its strength where it
            // is stacked at one (a WorldPainter map's intensity).
            let here = match &top {
                Some(top) => stack.surface_tree_layers(cell, top, under),
                None => Vec::new(),
            };
            let here = if here.is_empty() && n != 0 { vec![(n, 255)] } else { here };
            for (n, strength) in here {
                if let Some(l) = reg.object_layer(n) {
                    let intensity = if strength < 255 { strength as f32 / 255.0 } else { l.density };
                    layers.push(LayerHere { matches: Some(Match::Layer(plane, n)), kind, name: l.name.clone(), color: l.color, intensity: Some(intensity), paint: Some(Paint::Objects(n)) });
                }
            }
        }
        if let Some(size) = reg.tree_size(s.size as u16).filter(|t| !t.removed) {
            layers.push(LayerHere { matches: None, kind: tr!("Tree size"), name: crate::i18n::tr_name(&size.name).to_owned(), color: size.color, intensity: None, paint: Some(Paint::Size(s.size as u16)) });
        }
        if let Some(p) = reg.pocket(s.pockets) {
            layers.push(LayerHere { matches: Some(Match::Layer(Painted::Pockets, s.pockets)), kind: tr!("Pockets"), name: p.name.clone(), color: p.color, intensity: Some(p.share), paint: Some(Paint::Pockets(s.pockets)) });
        }
        if s.flags & FLAG_SLABS != 0 {
            layers.push(LayerHere { matches: None, kind: tr!("Smoothing"), name: tr!("Steps smoothed").to_owned(), color: [170, 170, 170], intensity: None, paint: None });
        }
        // Combined layers paint their parts into the others' planes: one
        // all of whose parts are here may have painted it.
        for (i, c) in reg.combined.iter().enumerate() {
            if c.removed || c.parts.is_empty() {
                continue;
            }
            let has = |p: &massif_core::combined::Component| {
                use massif_core::combined::Part;
                match p.part {
                    Part::Terrain => s.terrain == p.layer,
                    Part::Biome => s.biome == p.layer,
                    Part::Cover => s.cover == p.layer,
                    Part::Trees => s.objects == p.layer,
                    Part::Under => s.understory == p.layer,
                    Part::Size => s.size as u16 == p.layer,
                    Part::Pockets => s.pockets == p.layer,
                }
            };
            // A part painted over only a share of the columns may be
            // missing here: the parts painted everywhere must be here, and
            // one at least.
            if c.parts.iter().filter(|p| p.share >= 1.0).all(has) && c.parts.iter().any(has) {
                layers.push(LayerHere { matches: None, kind: tr!("Combined"), name: c.name.clone(), color: c.color, intensity: None, paint: Some(Paint::Combined(i as u16 + 1)) });
            }
        }
        if s.annotation != 0 {
            let n = s.annotation as u16;
            let color = reg.annotations.color(n).unwrap_or([128; 3]);
            layers.push(LayerHere { matches: Some(Match::Layer(Painted::Annotation, n)), kind: tr!("Annotation"), name: crate::panels::annotation_label(n, &reg.annotations), color, intensity: None, paint: Some(Paint::Annotation(n)) });
        }
        let selected = doc.selection().map(|sel| sel.weight(x, z));
        Some(Self { x, z, height, terrain: (!empty).then_some(TerrainId(s.terrain)), water, biome, auto_biome, levels, layers, selected })
    }

    /// What a filter can be set to from here: the terrain, the biome,
    /// water or dry land, and each layer that a filter tests, with what to
    /// call them.
    pub fn choices(&self, reg: &massif_core::registry::Registry) -> Vec<(Match, String, [u8; 3])> {
        let mut out = Vec::new();
        if let Some(t) = self.terrain {
            out.push((Match::Terrain(t.0), format!("{}: {}", tr!("Terrain"), reg.terrain(t).name), reg.terrain(t).color));
        }
        if let Some(b) = self.biome {
            out.push((Match::Biome(b.0), format!("{}: {}", tr!("Biome"), reg.biome(b).name), reg.biome(b).color));
        }
        if self.height.is_some() {
            out.push(if self.water.is_some() { (Match::Water, tr!("Water").to_owned(), [60, 110, 200]) } else { (Match::Land, tr!("Dry land").to_owned(), [150, 130, 90]) });
        }
        for l in &self.layers {
            if let Some(m) = l.matches {
                out.push((m, format!("{}: {}", l.kind, l.name), l.color));
            }
        }
        out
    }
}

impl ColumnInfo {
    /// What the Eyedropper tool can paint with from here: the terrain, the
    /// biome (painted, or the one the land calls for), and each layer
    /// painted on it, with what to call them.
    pub fn paints(&self, reg: &massif_core::registry::Registry) -> Vec<(Paint, String, [u8; 3])> {
        let mut out = Vec::new();
        if let Some(t) = self.terrain {
            out.push((Paint::Terrain(t), format!("{}: {}", tr!("Terrain"), reg.terrain(t).name), reg.terrain(t).color));
        }
        if let Some(b) = self.biome {
            out.push((Paint::Biome(b), format!("{}: {}", tr!("Biome"), reg.biome(b).name), reg.biome(b).color));
        } else if let Some(b) = self.auto_biome {
            out.push((Paint::Biome(b), format!("{}: {}{}", tr!("Biome"), reg.biome(b).name, tr!(" (automatic)")), reg.biome(b).color));
        }
        for l in &self.layers {
            if let Some(p) = l.paint {
                out.push((p, format!("{}: {}", l.kind, l.name), l.color));
            }
        }
        out
    }
}

/// The Eyedropper tool's choices at a spot: its coordinates, then each
/// thing there to paint with. Returns `Some(Some(what))` once one is
/// picked, `Some(None)` on Cancel.
pub fn paint_popup(ui: &mut Ui, info: &ColumnInfo, choices: &[(Paint, String, [u8; 3])]) -> Option<Option<Paint>> {
    let pal = Palette::of(ui);
    let mut out = None;
    ui.set_min_width(200.0);
    ui.label(RichText::new(tr!("Paint with…")).strong());
    ui.label(RichText::new(tr!("At {x}, {z}", x = info.x, z = info.z)).size(11.0).color(pal.weak));
    ui.add_space(4.0);
    for (what, name, color) in choices {
        let clicked = ui
            .horizontal(|ui| {
                swatch(ui, *color);
                ui.add(egui::Button::new(name.as_str()).frame(false)).clicked()
            })
            .inner;
        if clicked {
            out = Some(Some(*what));
        }
    }
    ui.add_space(4.0);
    if ui.button(tr!("Cancel")).clicked() {
        out = Some(None);
    }
    out
}

/// A coloured square before a name.
pub fn swatch(ui: &mut Ui, color: [u8; 3]) {
    let size = 10.0;
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, theme::TILE_RADIUS / 4, theme::rgb(color));
}

/// The Info panel: the column under the cursor, all it holds.
pub fn panel(ui: &mut Ui, info: Option<&ColumnInfo>, reg: &massif_core::registry::Registry) {
    let pal = Palette::of(ui);
    let Some(c) = info else {
        crate::panels::section(ui, Icon::Crosshair, tr!("Under the cursor"));
        crate::panels::hint(ui, tr!("Hover the map or the 3D view: what is there shows here."));
        crate::panels::hint(ui, tr!("The world's name, seed, size and memory are in File ▸ World Information…"));
        return;
    };
    crate::panels::section(ui, Icon::Crosshair, tr!("Under the cursor"));
    crate::widgets::well(ui, |ui| {
        crate::widgets::row(ui, tr!("Coordinates"), |ui| {
            ui.label(RichText::new(format!("{}, {}", c.x, c.z)).monospace());
        });
        crate::widgets::row(ui, tr!("Height"), |ui| {
            ui.label(RichText::new(c.height.map_or_else(|| "—".to_owned(), |h| h.to_string())).monospace());
        });
        crate::widgets::row(ui, tr!("Water"), |ui| {
            let text = match c.water {
                Some((top, depth)) => tr!("{top}, {depth} deep", top, depth),
                None => tr!("None").to_owned(),
            };
            ui.label(text);
        });
        crate::widgets::row(ui, tr!("Terrain"), |ui| match c.terrain {
            Some(t) => {
                ui.label(&reg.terrain(t).name);
                swatch(ui, reg.terrain(t).color);
            }
            None => {
                ui.label("—");
            }
        });
        crate::widgets::row(ui, tr!("Biome"), |ui| match (c.biome, c.auto_biome) {
            (Some(b), _) => {
                ui.label(&reg.biome(b).name);
            }
            (None, Some(b)) => {
                ui.label(RichText::new(tr!("{name} (automatic)", name = reg.biome(b).name)));
            }
            (None, None) => {
                ui.label(tr!("Automatic"));
            }
        });
        crate::widgets::row(ui, tr!("Selection"), |ui| {
            let text = match c.selected {
                None => tr!("No selection").to_owned(),
                Some(w) if w >= 1.0 => tr!("Selected").to_owned(),
                Some(w) if w > 0.0 => tr!("{value:.0} % selected", value = w * 100.0),
                Some(_) => tr!("Not selected").to_owned(),
            };
            ui.label(text);
        });
        if c.levels.len() > 1 {
            crate::widgets::row(ui, tr!("Levels"), |ui| {
                ui.label(c.levels.join(", "));
            });
        }
    });
    crate::panels::section(ui, Icon::Layers, tr!("Layers here"));
    if c.layers.is_empty() {
        crate::panels::hint(ui, tr!("No layer is painted here."));
        return;
    }
    crate::widgets::well(ui, |ui| {
        for l in &c.layers {
            crate::widgets::row(ui, l.kind, |ui| {
                if let Some(v) = l.intensity {
                    ui.label(RichText::new(format!("{:.0} %", v * 100.0)).color(pal.weak).monospace());
                }
                ui.label(&l.name);
                swatch(ui, l.color);
            });
        }
    });
}

/// What the filters' eyedropper offers at a spot: its coordinates, then
/// each thing there a filter can be on. Returns `Some(Some(what))` once one
/// is picked, `Some(None)` on Cancel.
pub fn pick_popup(ui: &mut Ui, info: &ColumnInfo, choices: &[(Match, String, [u8; 3])], only: bool) -> Option<Option<Match>> {
    let pal = Palette::of(ui);
    let mut out = None;
    ui.set_min_width(200.0);
    ui.label(RichText::new(if only { tr!("Act only on…") } else { tr!("Act everywhere but on…") }).strong());
    ui.label(RichText::new(tr!("At {x}, {z}", x = info.x, z = info.z)).size(11.0).color(pal.weak));
    ui.add_space(4.0);
    if choices.is_empty() {
        ui.label(RichText::new(tr!("Nothing a filter can be on here")).color(pal.weak));
    }
    for (what, name, color) in choices {
        let clicked = ui
            .horizontal(|ui| {
                swatch(ui, *color);
                ui.add(egui::Button::new(name.as_str()).frame(false)).clicked()
            })
            .inner;
        if clicked {
            out = Some(Some(*what));
        }
    }
    ui.add_space(4.0);
    if ui.button(tr!("Cancel")).clicked() {
        out = Some(None);
    }
    out
}
