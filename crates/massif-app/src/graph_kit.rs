//! What is left of Massif's first terrain graph (`massif_core::graph`) in
//! the app, now that the Terrain Generator is its only generator: the
//! pictures of the new map's templates, the settings of a node as the
//! Reshape dialog shows them, and the blocks and levels a graph's result is
//! written with (shared with the Terrain Generator's Apply).

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::{Color32, ColorImage, Rect, RichText, Sense, Stroke, StrokeKind, TextureOptions, Ui, pos2, vec2};
use massif_core::graph::{
    AutoPalette, Bend, BlockPalette, CellStyle, Climate, ColorField, CombineOp, Extent, Field, GradientPreset,
    LayerOut, MapSource, MountainStyle, NodeKind, NoiseStyle, Plate, Progress, SampledMap, ShapeKind, Template, TintMode,
};
use massif_core::LevelKind;
use massif_core::registry::{Registry, TerrainId};
use std::sync::atomic::AtomicBool;

/// A small picture of what a template makes: its colours shaded by its
/// relief, the sea over what lies under it.
pub(crate) fn template_picture(t: Template, reg: &Registry, sea: f32, extent: &Extent, map: Option<&SampledMap>) -> Option<ColorImage> {
    let graph = t.graph(reg, sea, extent.size as f32);
    let source = map.map(|m| m as &dyn MapSource);
    let out = graph.build_on(source, extent, &AtomicBool::new(false), &Progress::default()).ok()?;
    let h = out.heights?;
    let terrain_colors: Vec<[u8; 3]> = reg.terrains.iter().map(|t| t.color).collect();
    let colors = massif_core::graph::painted_colors(&h, out.shown.as_deref(), &out.terrains, &terrain_colors);
    let n = h.n;
    let spacing = extent.spacing();
    let light = {
        let l = [-0.6f32, 0.75, -0.45];
        let k = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        l.map(|v| v / k)
    };
    let water = [0.03f32, 0.16, 0.28].map(|v| v * 0.8);
    // A surface lit from the north-west.
    let shade = |f: &Field, i: usize, j: usize| {
        let dx = (f.at(i + 1, j) - f.at(i.saturating_sub(1), j)) / (2.0 * spacing);
        let dz = (f.at(i, j + 1) - f.at(i, j.saturating_sub(1))) / (2.0 * spacing);
        let k = (dx * dx + 1.0 + dz * dz).sqrt();
        0.25 + 0.95 * ((-dx * light[0] + light[1] - dz * light[2]) / k).max(0.0)
    };
    let mut picture: Vec<[f32; 3]> = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let v = h.at(i, j);
            let base = colors.as_ref().map_or([0.4, 0.4, 0.4], |c| c.linear(j * n + i));
            let mut c = base.map(|b| b * shade(&h, i, j));
            if v < sea {
                let deep = 0.55 + 0.4 * ((sea - v) / 24.0).min(1.0);
                c = [0, 1, 2].map(|k| c[k] + (water[k] - c[k]) * deep);
            }
            picture.push(c);
        }
    }
    // Rock over the ground, seen from above, and its shadow under it.
    for layer in &out.layers {
        let there = |i: usize, j: usize| layer.top.at(i, j) - layer.bottom.at(i, j) >= 1.0;
        for j in 0..n {
            for i in 0..n {
                if i >= 2 && j >= 2 && there(i - 2, j - 2) && !there(i, j) {
                    picture[j * n + i] = picture[j * n + i].map(|v| v * 0.55);
                }
            }
        }
        for j in 0..n {
            for i in 0..n {
                if there(i, j) {
                    let base = layer.colors.as_ref().map_or([0.3, 0.42, 0.2], |(c, _)| c.linear(j * n + i));
                    picture[j * n + i] = base.map(|b| b * shade(&layer.top, i, j) * 1.1);
                }
            }
        }
    }
    let rgb: Vec<u8> = picture.into_iter().flat_map(|c| c.map(massif_core::graph::encode_srgb)).collect();
    Some(ColorImage::from_rgb([n, n], &rgb))
}

/// Turn a build's colours into terrains of real blocks: the blocks of the
/// palette (read from the game, or Massif's terrains without it), only
/// those the colours come close to, made terrains of the world.
/// The Solid levels the layers of a conversion go to, in their order: those
/// earlier conversions made, new ones for the others. Levels of layers the
/// graph no longer has are removed — unless it starts from the map, which
/// they are part of. Called at the start of the history step.
pub(crate) fn layer_levels(doc: &mut Document, layers: &[LayerOut], reads_map: bool) -> Vec<usize> {
    let find = |doc: &Document, slot: u32| {
        doc.dimension()
            .levels
            .iter()
            .position(|l| l.kind == LevelKind::Solid && LayerOut::is_level_of(&l.name, slot))
    };
    let graph_level = |name: &str| (1..=64).any(|slot| LayerOut::is_level_of(name, slot));
    let stale: Vec<usize> = doc
        .dimension()
        .levels
        .iter()
        .enumerate()
        .filter(|(_, l)| !reads_map && l.kind == LevelKind::Solid && graph_level(&l.name) && !layers.iter().any(|o| LayerOut::is_level_of(&l.name, o.slot)))
        .map(|(k, _)| k)
        .collect();
    let missing: Vec<&LayerOut> = layers.iter().filter(|l| find(doc, l.slot).is_none()).collect();
    if !stale.is_empty() || !missing.is_empty() {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        let dim = edit.structure();
        for k in stale.into_iter().rev() {
            dim.remove_level(k);
        }
        for l in missing {
            dim.add_level(l.name.clone(), LevelKind::Solid);
        }
    }
    layers.iter().map(|l| find(doc, l.slot).expect("the level was just made")).collect()
}

pub(crate) fn block_palette(doc: &mut Document, blocks: &BlockLibrary, set: BlockPalette, colors: &ColorField) -> Vec<(TerrainId, [u8; 3])> {
    match &blocks.catalogue {
        Some(cat) => {
            let wanted = set.blocks();
            let candidates: Vec<&massif_assets::Block> = if wanted.is_empty() {
                cat.blocks.iter().filter(|b| b.full_cube && b.top.is_some()).collect()
            } else {
                wanted.iter().filter_map(|id| cat.block(id)).filter(|b| b.full_cube).collect()
            };
            let rgb: Vec<[u8; 3]> = candidates.iter().map(|b| b.color).collect();
            let used = massif_core::graph::colors_in_use(colors, &rgb);
            candidates
                .into_iter()
                .zip(used)
                .filter(|(_, u)| *u)
                .map(|(b, _)| (massif_assets::block_terrain(&mut doc.world.registry, b), b.color))
                .collect()
        }
        None => doc
            .world
            .registry
            .terrains
            .iter()
            .enumerate()
            .map(|(k, t)| (TerrainId(k as u16), t.color))
            .collect(),
    }
}

// ------------------------------------------------------------- climate --

/// The climates, each with its colour; with `written`, the biome and the
/// terrain converting writes for it.
fn climate_legend(ui: &mut Ui, written: bool) {
    let pal = Palette::of(ui);
    egui::Grid::new(("climates", written)).num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        for c in Climate::ALL {
            let [r, g, b] = c.color();
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 2, Color32::from_rgb(r, g, b));
                ui.label(RichText::new(crate::i18n::tr_name(c.label())).size(11.0));
            });
            if written {
                let biome = c.biome().trim_start_matches("minecraft:");
                ui.label(RichText::new(tr!("{biome} · {terrain}", terrain = c.terrain(), biome)).size(10.5).color(pal.weak));
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
}

/// The plates of a Tectonic node, to move: drag a plate's centre to move
/// it, the tip of its arrow to change how it moves; right-click it to make
/// it a continent or an ocean, or to remove it.
fn plate_editor(ui: &mut Ui, plates: &mut Vec<Plate>, seed: &mut u32) -> bool {
    let pal = Palette::of(ui);
    let side = ui.available_width().clamp(160.0, 280.0);
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let painter = ui.painter_at(rect);
    let land = Color32::from_rgb(132, 150, 96);
    let sea = Color32::from_rgb(48, 92, 150);
    // Who owns each spot, and how near a boundary.
    let px = 80;
    let mut pixels = Vec::with_capacity(px * px);
    for j in 0..px {
        for i in 0..px {
            let (x, z) = ((i as f32 + 0.5) / px as f32, (j as f32 + 0.5) / px as f32);
            let (mut a, mut da, mut db) = (0, f32::MAX, f32::MAX);
            for (k, p) in plates.iter().enumerate() {
                let d = (p.x - x).powi(2) + (p.z - z).powi(2);
                if d < da {
                    db = da;
                    (a, da) = (k, d);
                } else if d < db {
                    db = d;
                }
            }
            let base = plates.get(a).map_or(sea, |p| if p.continental { land } else { sea });
            let edge = db.sqrt() - da.sqrt() < 0.012;
            pixels.push(if edge { base.gamma_multiply(0.55) } else { base });
        }
    }
    let image = ColorImage {
        size: [px, px],
        pixels,
        source_size: vec2(px as f32, px as f32),
    };
    // Kept from frame to frame: a texture dropped is gone before it is drawn.
    let kept = egui::Id::unique("tectonic plates picture");
    let texture = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(kept)) {
        Some(mut t) => {
            t.set(image, TextureOptions::NEAREST);
            t
        }
        None => {
            let t = ui.ctx().load_texture("tectonic plates", image, TextureOptions::NEAREST);
            ui.ctx().data_mut(|d| d.insert_temp(kept, t.clone()));
            t
        }
    };
    painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    painter.rect_stroke(rect, 3, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let reach = side * 0.12;
    let mut changed = false;
    let mut removed = None;
    for (k, p) in plates.iter_mut().enumerate() {
        let centre = rect.min + vec2(p.x, p.z) * side;
        let tip = centre + vec2(p.dx, p.dz) * reach;
        painter.arrow(centre, tip - centre, Stroke::new(2.0, Color32::WHITE));
        let id = ui.scope_id().with(("plate", k));
        let moved = ui.interact(Rect::from_center_size(centre, vec2(16.0, 16.0)), id, Sense::click_and_drag());
        if moved.dragged()
            && let Some(at) = moved.interact_pointer_pos()
        {
            let at = (at - rect.min) / side;
            (p.x, p.z) = (at.x.clamp(0.0, 1.0), at.y.clamp(0.0, 1.0));
            changed = true;
        }
        moved.context_menu(|ui| {
            if ui.button(if p.continental { tr!("Make it an ocean") } else { tr!("Make it a continent") }).clicked() {
                p.continental = !p.continental;
                changed = true;
                ui.close();
            }
            if ui.button(tr!("Remove the plate")).clicked() {
                removed = Some(k);
                ui.close();
            }
        });
        let aimed = ui.interact(Rect::from_center_size(tip, vec2(14.0, 14.0)), id.with("tip"), Sense::drag());
        if aimed.dragged()
            && let Some(at) = aimed.interact_pointer_pos()
        {
            let d = (at - centre) / reach;
            (p.dx, p.dz) = (d.x.clamp(-1.6, 1.6), d.y.clamp(-1.6, 1.6));
            changed = true;
        }
        let fill = if p.continental { land } else { sea };
        painter.circle(centre, if moved.hovered() || moved.dragged() { 7.5 } else { 6.0 }, fill, Stroke::new(2.0, Color32::WHITE));
        painter.circle_filled(tip, if aimed.hovered() || aimed.dragged() { 4.5 } else { 3.0 }, Color32::WHITE);
        moved.on_hover_text(tr!("Drag: move · right-click: land or ocean"));
        aimed.on_hover_text(tr!("Drag: how the plate moves"));
    }
    if let Some(k) = removed {
        plates.remove(k);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui.small_button(theme::with_icon(Icon::Plus, tr!("Add a plate"))).clicked() {
            plates.push(Plate {
                x: 0.5,
                z: 0.5,
                dx: 0.6,
                dz: 0.0,
                continental: true,
            });
            changed = true;
        }
        if ui.small_button(theme::with_icon(Icon::Dices, tr!("Scatter"))).on_hover_text(tr!("New plates, as many, scattered anew")).clicked() {
            *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 100_000;
            *plates = massif_core::graph::scatter_plates(plates.len().max(2), *seed);
            changed = true;
        }
        let continents = plates.iter().filter(|p| p.continental).count();
        ui.label(RichText::new(tr!("{n} plates, {continents} continents", n = plates.len(), continents)).size(11.0).color(pal.weak));
    });
    changed
}

// ------------------------------------------------------------- settings --

/// What the settings of the node being edited are for, by name.
type Tips = &'static [(&'static str, &'static str)];

fn tips_id() -> egui::Id {
    egui::Id::unique("massif node setting tips")
}

/// What each setting of `kind` is for, shown over its name.
fn tips(kind: &NodeKind) -> Tips {
    match kind {
        NodeKind::Rivers { .. } => &[
            (tn!("Fewer"), tn!("Higher: fewer, bigger rivers")),
            (tn!("Depth"), tn!("Depth of the largest rivers")),
            (tn!("Width"), tn!("Width of the largest rivers")),
            (tn!("Banks"), tn!("How gently the banks slope to the water")),
            (tn!("Lakes"), tn!("How many hollows fill up as lakes")),
        ],
        NodeKind::Humidity { .. } => &[
            (
                tn!("Wind from"),
                tn!("Where the wet wind comes from: 0° north, 90° east"),
            ),
            (tn!("Inland air"), tn!("Moisture far from any sea")),
            (tn!("Reach"), tn!("How far inland the sea's moisture goes")),
            (tn!("Rain shadow"), tn!("How dry it is behind mountains")),
            (tn!("Variation"), tn!("Patches of wetter and drier land")),
        ],
        NodeKind::Temperature { .. } => &[
            (tn!("Warm side"), tn!("Temperature on the warm side of the map")),
            (tn!("Cold side"), tn!("Temperature on the cold side")),
            (tn!("Warm toward"), tn!("Where it is warm: 0° north, 90° east")),
            (tn!("Height cools"), tn!("How much colder it gets going up")),
            (tn!("Variation"), tn!("Patches of warmer and colder land")),
        ],
        NodeKind::Lakes { .. } => &[(tn!("Fill"), tn!("How much the hollows' ground is raised"))],
        _ => &[],
    }
}

/// A row of settings: a label, then the control.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    let tip = ui.data(|d| d.get_temp::<Tips>(tips_id())).and_then(|t| {
        t.iter()
            .find(|(name, _)| crate::i18n::tr_name(name) == label)
            .map(|(_, tip)| crate::i18n::tr_name(tip))
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        let mut l = ui.add_sized(
            [96.0, 18.0],
            egui::Label::new(RichText::new(label).size(12.0)).truncate(),
        );
        if let Some(tip) = tip {
            l = l.on_hover_text(tip);
        }
        add(ui).labelled_by(l.id).changed()
    })
    .inner
}

/// A distance in blocks: a plain, linear slider (type a value to go past
/// its end).
fn blocks(ui: &mut Ui, label: &str, v: &mut f32, max: f32) -> bool {
    row(ui, label, |ui| {
        ui.add(
            crate::widgets::Field::new(v, 1.0..=max.min(6000.0))
                .clamping(egui::SliderClamping::Never)
                .suffix(tr!(" blocks"))
                .max_decimals(0),
        )
    })
}

/// The build limits of the map the graph is for, as the properties set
/// them for the settings that are heights.
fn limits_id() -> egui::Id {
    egui::Id::unique("massif build limits")
}

fn limits(ui: &Ui) -> (f32, f32) {
    ui.data(|d| d.get_temp::<(f32, f32)>(limits_id())).unwrap_or((-64.0, 320.0))
}

fn height(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    let (bottom, top) = limits(ui);
    row(ui, label, |ui| {
        ui.add(
            crate::widgets::Field::new(v, bottom..=top)
                .clamping(egui::SliderClamping::Never)
                .max_decimals(0),
        )
    })
}

fn amount(ui: &mut Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> bool {
    row(ui, label, |ui| ui.add(crate::widgets::Field::new(v, range).max_decimals(2)))
}

fn fraction(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    row(ui, label, |ui| {
        ui.add(
            crate::widgets::Field::new(v, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    })
}

fn angle(ui: &mut Ui, label: &str, v: &mut f32) -> bool {
    row(ui, label, |ui| ui.add(crate::widgets::Field::new(v, 0.0..=360.0).suffix("°").max_decimals(0)))
}

fn count(ui: &mut Ui, label: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) -> bool {
    row(ui, label, |ui| ui.add(crate::widgets::Field::new(v, range)))
}

/// The sea a simulation knows of: a level, or none.
fn sea_level(ui: &mut Ui, v: &mut f32, hover: &str) -> bool {
    let mut on = *v > massif_core::graph::NO_WATER + 1.0;
    let before = *v;
    ui.horizontal(|ui| {
        crate::widgets::name_label(ui, RichText::new(tr!("Sea level")).size(12.0), vec2(96.0, 18.0));
        if crate::widgets::checkbox(ui, &mut on, "").on_hover_text(hover).changed() {
            *v = if on { 63.0 } else { massif_core::graph::NO_WATER };
        }
        if on {
            let (bottom, top) = limits(ui);
            crate::widgets::drag(ui, egui::DragValue::new(v).range(bottom..=top).speed(0.5).max_decimals(0));
        }
    });
    *v != before
}

fn seed(ui: &mut Ui, v: &mut u32) -> bool {
    ui.horizontal(|ui| {
        crate::widgets::name_label(ui, RichText::new(tr!("Seed")).size(12.0), vec2(96.0, 18.0));
        let mut changed = crate::widgets::drag(ui, egui::DragValue::new(v)).changed();
        if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text(tr!("Another seed")).clicked() {
            *v = v.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 100_000;
            changed = true;
        }
        changed
    })
    .inner
}

fn choice<T: PartialEq + Copy>(ui: &mut Ui, label: &str, v: &mut T, options: &[(T, &str)]) -> bool {
    row(ui, label, |ui| {
        let current = options.iter().find(|o| o.0 == *v).map_or("", |o| o.1);
        let mut changed = false;
        let mut r = crate::widgets::combo(label)
            .selected_text(current)
            .width(150.0)
            .show_ui(ui, |ui| {
                for (o, name) in options {
                    if ui.selectable_value(v, *o, *name).changed() {
                        changed = true;
                    }
                }
            })
            .response;
        if changed {
            r.mark_changed();
        }
        r
    })
}

fn color(ui: &mut Ui, label: &str, v: &mut [u8; 3]) -> bool {
    row(ui, label, |ui| ui.color_edit_button_srgb(v))
}

fn flag(ui: &mut Ui, label: &str, v: &mut bool) -> bool {
    row(ui, label, |ui| crate::widgets::checkbox(ui, v, ""))
}

/// Swatches of the gradients, to pick one.
fn gradient_picker(ui: &mut Ui, preset: &mut GradientPreset) -> bool {
    let mut changed = false;
    let pal = Palette::of(ui);
    let width = ((ui.available_width() - 8.0) / 2.0).max(60.0);
    egui::Grid::new("gradient presets").num_columns(2).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        for (k, p) in GradientPreset::ALL.iter().enumerate() {
            let (rect, r) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click());
            let painter = ui.painter();
            let steps = 24;
            for s in 0..steps {
                let t = s as f32 / (steps - 1) as f32;
                let c = p.at(t).map(massif_core::graph::encode_srgb);
                let x0 = rect.left() + rect.width() * s as f32 / steps as f32;
                let x1 = rect.left() + rect.width() * (s + 1) as f32 / steps as f32;
                painter.rect_filled(Rect::from_min_max(pos2(x0, rect.top()), pos2(x1 + 0.5, rect.bottom())), 0, Color32::from_rgb(c[0], c[1], c[2]));
            }
            let selected = *preset == *p;
            painter.rect_stroke(rect, 3, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { pal.accent } else { pal.border }), StrokeKind::Inside);
            painter.text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, crate::i18n::tr_name(p.label()), egui::FontId::proportional(10.5), Color32::WHITE);
            if r.clicked() {
                *preset = *p;
                changed = true;
            }
            if k % 2 == 1 {
                ui.end_row();
            }
        }
    });
    changed
}

/// The settings of a node. Every control reports changes through the graph
/// comparison done by the caller.
/// The settings of a node, as its properties show them, outside the graph:
/// `limits` are the map's lowest and highest heights. Whether one changed.
pub(crate) fn node_settings(ui: &mut Ui, kind: &mut NodeKind, reg: &Registry, limits: (f32, f32)) -> bool {
    ui.data_mut(|d| d.insert_temp(limits_id(), limits));
    let before = kind.clone();
    properties(ui, kind, reg);
    *kind != before
}

fn properties(ui: &mut Ui, kind: &mut NodeKind, reg: &Registry) {
    let pal = Palette::of(ui);
    let tips = tips(kind);
    ui.data_mut(|d| d.insert_temp(tips_id(), tips));
    let section = |ui: &mut Ui, name: &str| {
        ui.add_space(6.0);
        ui.label(RichText::new(name).size(11.0).color(pal.weak).medium());
    };
    match kind {
        NodeKind::Noise {
            style,
            scale,
            octaves,
            roughness,
            height: h,
            base,
            seed: s,
        } => {
            choice(ui, tr!("Style"), style, &[(NoiseStyle::Fbm, tr!("Rolling")), (NoiseStyle::Ridged, tr!("Ridged")), (NoiseStyle::Billow, tr!("Billow"))]);
            blocks(ui, tr!("Feature size"), scale, 20000.0);
            count(ui, tr!("Detail"), octaves, 1..=12);
            amount(ui, tr!("Roughness"), roughness, 0.05..=0.95);
            height(ui, tr!("Height"), h);
            height(ui, tr!("Base"), base);
            seed(ui, s);
        }
        NodeKind::Cells {
            style,
            scale,
            jitter,
            height: h,
            base,
            seed: s,
        } => {
            choice(ui, tr!("Style"), style, &[(CellStyle::Cones, tr!("Cones")), (CellStyle::Ridges, tr!("Ridges")), (CellStyle::Plates, tr!("Plates"))]);
            blocks(ui, tr!("Cell size"), scale, 20000.0);
            fraction(ui, tr!("Jitter"), jitter);
            height(ui, tr!("Height"), h);
            height(ui, tr!("Base"), base);
            seed(ui, s);
        }
        NodeKind::Radial {
            inner,
            outer,
            height: h,
            base,
        } => {
            fraction(ui, tr!("Fall starts"), inner);
            fraction(ui, tr!("Fall ends"), outer);
            amount(ui, tr!("Height"), h, -400.0..=400.0);
            amount(ui, tr!("Base"), base, limits(ui).0..=limits(ui).1);
        }
        NodeKind::Constant { value } => {
            row(ui, tr!("Value"), |ui| crate::widgets::drag(ui, egui::DragValue::new(value).speed(0.5)));
        }
        NodeKind::LinearGradient { angle: a, height: h, base } => {
            angle(ui, tr!("Rises toward"), a);
            amount(ui, tr!("Height"), h, -400.0..=400.0);
            amount(ui, tr!("Base"), base, limits(ui).0..=limits(ui).1);
        }
        NodeKind::Shape {
            shape,
            x,
            z,
            radius,
            falloff,
            height: h,
            base,
        } => {
            choice(ui, tr!("Shape"), shape, &ShapeKind::ALL.map(|s| (s, crate::i18n::tr_name(s.label()))));
            fraction(ui, tr!("Across (x)"), x);
            fraction(ui, tr!("Down (z)"), z);
            blocks(ui, tr!("Radius"), radius, 10000.0);
            fraction(ui, tr!("Soft edge"), falloff);
            amount(ui, tr!("Height"), h, -400.0..=400.0);
            amount(ui, tr!("Base"), base, limits(ui).0..=limits(ui).1);
        }
        NodeKind::Mountain {
            style,
            x,
            z,
            radius,
            height: h,
            base,
            detail,
            seed: s,
        } => {
            choice(ui, tr!("Style"), style, &MountainStyle::ALL.map(|s| (s, crate::i18n::tr_name(s.label()))));
            blocks(ui, tr!("Radius"), radius, 10000.0);
            height(ui, tr!("Height"), h);
            height(ui, tr!("Base"), base);
            fraction(ui, tr!("Detail"), detail);
            section(ui, tr!("Where"));
            fraction(ui, tr!("Across (x)"), x);
            fraction(ui, tr!("Down (z)"), z);
            seed(ui, s);
        }
        NodeKind::MountainRange {
            angle: a,
            width,
            scale,
            height: h,
            base,
            seed: s,
        } => {
            angle(ui, tr!("Runs toward"), a);
            blocks(ui, tr!("Width"), width, 10000.0);
            blocks(ui, tr!("Peak size"), scale, 5000.0);
            height(ui, tr!("Height"), h);
            height(ui, tr!("Base"), base);
            seed(ui, s);
        }
        NodeKind::Volcano {
            x,
            z,
            radius,
            height: h,
            base,
            crater,
            depth,
            seed: s,
        } => {
            blocks(ui, tr!("Radius"), radius, 10000.0);
            height(ui, tr!("Height"), h);
            height(ui, tr!("Base"), base);
            fraction(ui, tr!("Crater width"), crater);
            amount(ui, tr!("Crater depth"), depth, 0.0..=200.0);
            section(ui, tr!("Where"));
            fraction(ui, tr!("Across (x)"), x);
            fraction(ui, tr!("Down (z)"), z);
            seed(ui, s);
        }
        NodeKind::Crater {
            x,
            z,
            radius,
            depth,
            rim,
            peak,
            base,
            seed: s,
        } => {
            blocks(ui, tr!("Radius"), radius, 5000.0);
            amount(ui, tr!("Depth"), depth, 0.0..=200.0);
            amount(ui, tr!("Rim"), rim, 0.0..=100.0);
            amount(ui, tr!("Central peak"), peak, 0.0..=100.0);
            height(ui, tr!("Plain"), base);
            section(ui, tr!("Where"));
            fraction(ui, tr!("Across (x)"), x);
            fraction(ui, tr!("Down (z)"), z);
            seed(ui, s);
        }
        NodeKind::CraterField {
            count: c,
            min_radius,
            max_radius,
            depth,
            base,
            seed: s,
        } => {
            count(ui, tr!("Craters"), c, 1..=2000);
            blocks(ui, tr!("Smallest"), min_radius, 2000.0);
            blocks(ui, tr!("Largest"), max_radius, 5000.0);
            fraction(ui, tr!("Depth"), depth);
            height(ui, tr!("Plain"), base);
            seed(ui, s);
        }
        NodeKind::Dunes {
            angle: a,
            wavelength,
            height: h,
            base,
            sharpness,
            seed: s,
        } => {
            angle(ui, tr!("Wind toward"), a);
            blocks(ui, tr!("Spacing"), wavelength, 2000.0);
            amount(ui, tr!("Height"), h, 0.0..=120.0);
            height(ui, tr!("Base"), base);
            fraction(ui, tr!("Steep lee side"), sharpness);
            seed(ui, s);
        }
        NodeKind::Canyon {
            angle: a,
            width,
            depth,
            meander,
            steps,
            base,
            seed: s,
        } => {
            angle(ui, tr!("Runs toward"), a);
            blocks(ui, tr!("Width"), width, 5000.0);
            amount(ui, tr!("Depth"), depth, 0.0..=300.0);
            blocks(ui, tr!("Winding"), meander, 3000.0);
            count(ui, tr!("Steps"), steps, 0..=20);
            height(ui, tr!("Plateau"), base);
            seed(ui, s);
        }
        NodeKind::Mesas {
            scale,
            coverage,
            height: h,
            base,
            steps,
            seed: s,
        } => {
            blocks(ui, tr!("Size"), scale, 5000.0);
            fraction(ui, tr!("Coverage"), coverage);
            amount(ui, tr!("Height"), h, 0.0..=300.0);
            height(ui, tr!("Base"), base);
            count(ui, tr!("Steps"), steps, 1..=12);
            seed(ui, s);
        }
        NodeKind::Island {
            size,
            height: h,
            base,
            coast,
            seed: s,
        } => {
            fraction(ui, tr!("Size"), size);
            amount(ui, tr!("Height"), h, 0.0..=380.0);
            height(ui, tr!("Sea floor"), base);
            fraction(ui, tr!("Ragged coast"), coast);
            seed(ui, s);
        }
        NodeKind::Rocks {
            scale,
            height: h,
            sharpness,
            seed: s,
        } => {
            blocks(ui, tr!("Size"), scale, 2000.0);
            amount(ui, tr!("Height"), h, 0.0..=100.0);
            fraction(ui, tr!("Sharpness"), sharpness);
            seed(ui, s);
        }
        NodeKind::Levels {
            in_min,
            in_max,
            out_min,
            out_max,
            gamma,
        } => {
            section(ui, tr!("From"));
            height(ui, tr!("Low"), in_min);
            height(ui, tr!("High"), in_max);
            section(ui, tr!("To"));
            height(ui, tr!("Low"), out_min);
            height(ui, tr!("High"), out_max);
            amount(ui, tr!("Gamma"), gamma, 0.1..=5.0);
        }
        NodeKind::Terrace { steps, strength } => {
            count(ui, tr!("Steps"), steps, 1..=64);
            fraction(ui, tr!("Strength"), strength);
        }
        NodeKind::Clamp { min, max } => {
            height(ui, tr!("Lowest"), min);
            height(ui, tr!("Highest"), max);
        }
        NodeKind::Blur { radius } => {
            blocks(ui, tr!("Radius"), radius, 500.0);
        }
        NodeKind::Warp { amount: a, scale, seed: s } => {
            blocks(ui, tr!("Distance"), a, 2000.0);
            blocks(ui, tr!("Size"), scale, 10000.0);
            seed(ui, s);
        }
        NodeKind::Normalize { min, max } => {
            height(ui, tr!("Lowest"), min);
            height(ui, tr!("Highest"), max);
        }
        NodeKind::Curve { gamma, contrast } => {
            amount(ui, tr!("Gamma"), gamma, 0.1..=6.0);
            amount(ui, tr!("Contrast"), contrast, -1.0..=1.0);
        }
        NodeKind::Transform {
            scale,
            rotate,
            offset_x,
            offset_z,
        } => {
            amount(ui, tr!("Scale"), scale, 0.1..=8.0);
            row(ui, tr!("Turn"), |ui| ui.add(crate::widgets::Field::new(rotate, -180.0..=180.0).suffix("°").max_decimals(0)));
            amount(ui, tr!("Move across"), offset_x, -1.0..=1.0);
            amount(ui, tr!("Move down"), offset_z, -1.0..=1.0);
        }
        NodeKind::Swirl { angle: a, radius } => {
            row(ui, tr!("Turn"), |ui| ui.add(crate::widgets::Field::new(a, -720.0..=720.0).suffix("°").max_decimals(0)));
            fraction(ui, tr!("Reach"), radius);
        }
        NodeKind::Fold { level, strength } => {
            height(ui, tr!("Level"), level);
            fraction(ui, tr!("Strength"), strength);
        }
        NodeKind::Stratify {
            layers,
            strength,
            tilt,
            seed: s,
        } => {
            count(ui, tr!("Layers"), layers, 1..=80);
            fraction(ui, tr!("Strength"), strength);
            row(ui, tr!("Tilt"), |ui| ui.add(crate::widgets::Field::new(tilt, -45.0..=45.0).suffix("°").max_decimals(0)));
            seed(ui, s);
        }
        NodeKind::Sharpen { radius, amount: a } => {
            blocks(ui, tr!("Radius"), radius, 500.0);
            amount(ui, tr!("Amount"), a, 0.0..=4.0);
        }
        NodeKind::InvertHeight | NodeKind::Blend | NodeKind::Invert | NodeKind::MixColors | NodeKind::HeightOutput => {
            ui.label(RichText::new(tr!("No settings.")).size(11.5).color(pal.weak));
        }
        NodeKind::MapHeight => {
            ui.label(
                RichText::new(tr!("The open map's surface, water and colours."))
                    .size(11.5)
                    .color(pal.weak),
            );
        }
        NodeKind::FileInput {
            path,
            low,
            high,
            fit,
        } => {
            ui.horizontal(|ui| {
                let shown = std::path::Path::new(path.as_str())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(tr!("— none"));
                ui.add_sized(
                    [96.0, 18.0],
                    egui::Label::new(RichText::new(tr!("File")).size(12.0)),
                );
                ui.label(RichText::new(shown).size(12.0))
                    .on_hover_text(path.as_str());
                if ui.button(tr!("Choose…")).clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .set_title(tr!("A height map or a world"))
                        .add_filter(
                            tr!("Height maps and worlds"),
                            &["png", "jpg", "jpeg", "exr", "r16", "r32", "raw", "world", "massif"],
                        )
                        .pick_file()
                {
                    *path = p.display().to_string();
                }
            });
            let file = std::path::Path::new(path.as_str());
            let state = if path.is_empty() {
                tr!("PNG, JPEG, EXR, .r16, .r32, .world or .massif").to_owned()
            } else if massif_core::graph::is_world(file) {
                row(ui, tr!("Fit to terrain"), |ui| crate::widgets::checkbox(ui, fit, ""));
                tr!("A world: its surface, water and terrain colours.").to_owned()
            } else {
                height(ui, tr!("Black is"), low);
                height(ui, tr!("White is"), high);
                match massif_core::heightmap::cached(file) {
                    Ok(m) => tr!("{width} × {height} pixels, stretched over the terrain.", width = m.width, height = m.height),
                    Err(e) => tr!("Cannot be read: {e}", e),
                }
            };
            ui.label(RichText::new(state).size(11.0).color(pal.weak));
        }
        NodeKind::Temperature {
            warm,
            cold,
            toward,
            lapse,
            sea,
            variation,
            seed: s,
        } => {
            row(ui, tr!("Warm side"), |ui| ui.add(crate::widgets::Field::new(warm, -30.0..=45.0).suffix(" °C").max_decimals(0)));
            row(ui, tr!("Cold side"), |ui| ui.add(crate::widgets::Field::new(cold, -30.0..=45.0).suffix(" °C").max_decimals(0)));
            angle(ui, tr!("Warm toward"), toward);
            row(ui, tr!("Height cools"), |ui| ui.add(crate::widgets::Field::new(lapse, 0.0..=30.0).suffix(tr!(" °C / 100 blocks")).max_decimals(1)));
            height(ui, tr!("Sea level"), sea);
            row(ui, tr!("Variation"), |ui| ui.add(crate::widgets::Field::new(variation, 0.0..=15.0).suffix(" °C").max_decimals(1)));
            seed(ui, s);
        }
        NodeKind::Humidity {
            from,
            base,
            reach,
            shadow,
            sea,
            variation,
            seed: s,
        } => {
            angle(ui, tr!("Wind from"), from);
            fraction(ui, tr!("Inland air"), base);
            blocks(ui, tr!("Reach"), reach, 6000.0);
            fraction(ui, tr!("Rain shadow"), shadow);
            height(ui, tr!("Sea level"), sea);
            fraction(ui, tr!("Variation"), variation);
            seed(ui, s);
        }
        NodeKind::Biomes { sea } => {
            height(ui, tr!("Sea level"), sea);
            climate_legend(ui, false);
        }
        // Its settings have a panel of their own (noise_paint_properties).
        NodeKind::NoisePaint { .. } => {}
        NodeKind::BiomeOutput { sea, paint } => {
            height(ui, tr!("Sea level"), sea);
            row(ui, tr!("Paint the ground"), |ui| crate::widgets::checkbox(ui, paint, ""));
            ui.label(
                RichText::new(tr!("Writes each climate's biome and terrain:"))
                    .size(11.0)
                    .color(pal.weak),
            );
            climate_legend(ui, true);
        }
        NodeKind::Tectonic {
            plates,
            relief,
            width,
            roughness,
            sea,
            seed: s,
        } => {
            plate_editor(ui, plates, s);
            blocks(ui, tr!("Relief"), relief, 320.0);
            blocks(ui, tr!("Belt width"), width, 2000.0);
            fraction(ui, tr!("Winding"), roughness);
            height(ui, tr!("Sea level"), sea);
            seed(ui, s);
        }
        NodeKind::WaterOutput => {
            ui.label(
                RichText::new(tr!("Water up to this level, where it is over the ground."))
                    .size(11.5)
                    .color(pal.weak),
            );
        }
        NodeKind::Erosion {
            rain,
            erosion,
            deposition,
            capacity,
            evaporation,
            radius,
            seed: s,
        } => {
            amount(ui, tr!("Rain"), rain, 0.05..=8.0);
            fraction(ui, tr!("Erosion"), erosion);
            fraction(ui, tr!("Deposition"), deposition);
            amount(ui, tr!("Capacity"), capacity, 0.1..=20.0);
            amount(ui, tr!("Evaporation"), evaporation, 0.0..=0.5);
            blocks(ui, tr!("Brush"), radius, 100.0);
            seed(ui, s);
        }
        NodeKind::Thermal {
            talus,
            iterations,
            strength,
        } => {
            row(ui, tr!("Steepest"), |ui| ui.add(crate::widgets::Field::new(talus, 5.0..=85.0).suffix("°").max_decimals(0)));
            count(ui, tr!("Passes"), iterations, 1..=500);
            fraction(ui, tr!("Strength"), strength);
        }
        NodeKind::Snow {
            line,
            slope,
            depth,
            softness,
        } => {
            height(ui, tr!("Snow line"), line);
            row(ui, tr!("Steepest"), |ui| ui.add(crate::widgets::Field::new(slope, 5.0..=85.0).suffix("°").max_decimals(0)));
            amount(ui, tr!("Depth"), depth, 0.0..=20.0);
            amount(ui, tr!("Fade"), softness, 0.5..=100.0);
        }
        NodeKind::Rivers {
            threshold,
            depth,
            width,
            sea,
            banks,
            lakes,
        } => {
            fraction(ui, tr!("Fewer"), threshold);
            amount(ui, tr!("Depth"), depth, 0.5..=40.0);
            blocks(ui, tr!("Width"), width, 200.0);
            amount(ui, tr!("Banks"), banks, 0.0..=4.0);
            fraction(ui, tr!("Lakes"), lakes);
            sea_level(ui, sea, tr!("Rivers run to the sea and end there"));
        }
        NodeKind::Lakes { amount: a, sea } => {
            fraction(ui, tr!("Fill"), a);
            sea_level(ui, sea, tr!("What lies under the sea is the sea's, not a lake"));
        }
        NodeKind::Sea { level, shore } => {
            height(ui, tr!("Level"), level);
            amount(ui, tr!("Shore"), shore, 0.5..=40.0);
        }
        NodeKind::Combine { op, weight } => {
            choice(ui, tr!("Operation"), op, &CombineOp::ALL.map(|o| (o, crate::i18n::tr_name(o.label()))));
            amount(ui, tr!("Weight of B"), weight, -4.0..=4.0);
        }
        NodeKind::SelectHeight { min, max, falloff } => {
            height(ui, tr!("From"), min);
            height(ui, tr!("To"), max);
            amount(ui, tr!("Fade"), falloff, 0.1..=100.0);
        }
        NodeKind::SelectSlope { min, max, falloff } => {
            row(ui, tr!("From"), |ui| ui.add(crate::widgets::Field::new(min, 0.0..=90.0).suffix("°").max_decimals(0)));
            row(ui, tr!("To"), |ui| ui.add(crate::widgets::Field::new(max, 0.0..=90.0).suffix("°").max_decimals(0)));
            amount(ui, tr!("Fade"), falloff, 0.1..=30.0);
        }
        NodeKind::Curvature { bend, radius, strength } => {
            choice(ui, tr!("Pick"), bend, &[(Bend::Convex, tr!("Ridges")), (Bend::Concave, tr!("Hollows"))]);
            blocks(ui, tr!("Size"), radius, 1000.0);
            amount(ui, tr!("Strength"), strength, 0.1..=5.0);
        }
        NodeKind::Facing { angle: a, spread } => {
            angle(ui, tr!("Facing"), a);
            row(ui, tr!("Spread"), |ui| ui.add(crate::widgets::Field::new(spread, 5.0..=180.0).suffix("°").max_decimals(0)));
        }
        NodeKind::Occlusion { radius } => {
            blocks(ui, tr!("Radius"), radius, 1000.0);
        }
        NodeKind::Flow { spread } => {
            fraction(ui, tr!("Small rills"), spread);
        }
        NodeKind::Gradient {
            preset,
            auto_range,
            min,
            max,
            reverse,
            variation,
            seed: s,
        } => {
            gradient_picker(ui, preset);
            ui.add_space(4.0);
            flag(ui, tr!("Fit the range"), auto_range);
            if !*auto_range {
                let (bottom, top) = limits(ui);
                amount(ui, tr!("From"), min, bottom..=top);
                amount(ui, tr!("To"), max, bottom..=top);
            }
            flag(ui, tr!("Reverse"), reverse);
            fraction(ui, tr!("Variation"), variation);
            seed(ui, s);
        }
        NodeKind::SolidColor { color: c } => {
            color(ui, tr!("Colour"), c);
        }
        NodeKind::AdjustColor {
            hue,
            saturation,
            brightness,
            contrast,
        } => {
            row(ui, tr!("Hue"), |ui| ui.add(crate::widgets::Field::new(hue, -180.0..=180.0).suffix("°").max_decimals(0)));
            amount(ui, tr!("Saturation"), saturation, 0.0..=3.0);
            amount(ui, tr!("Brightness"), brightness, 0.0..=3.0);
            amount(ui, tr!("Contrast"), contrast, 0.0..=3.0);
        }
        NodeKind::Tint { color: c, mode, strength } => {
            color(ui, tr!("Colour"), c);
            choice(ui, tr!("Mode"), mode, &TintMode::ALL.map(|m| (m, crate::i18n::tr_name(m.label()))));
            fraction(ui, tr!("Strength"), strength);
        }
        NodeKind::AutoColor {
            palette,
            snow_line,
            rock_slope,
            seed: s,
        } => {
            choice(ui, tr!("Land"), palette, &AutoPalette::ALL.map(|p| (p, crate::i18n::tr_name(p.label()))));
            height(ui, tr!("Snow line"), snow_line);
            row(ui, tr!("Rock from"), |ui| ui.add(crate::widgets::Field::new(rock_slope, 5.0..=80.0).suffix("°").max_decimals(0)));
            seed(ui, s);
        }
        NodeKind::TerrainOutput { terrain } => {
            row(ui, tr!("Terrain"), |ui| {
                let mut changed = false;
                let mut r = crate::widgets::combo("output terrain")
                    .selected_text(reg.terrain(TerrainId(*terrain)).name.clone())
                    .width(170.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for (k, t) in reg.terrains.iter().enumerate() {
                            if ui.selectable_value(terrain, k as u16, &t.name).changed() {
                                changed = true;
                            }
                        }
                    })
                    .response;
                if changed {
                    r.mark_changed();
                }
                r
            });
        }
        NodeKind::FloatingIslands {
            altitude,
            spread,
            size,
            coverage,
            depth,
            seed: s,
        } => {
            height(ui, tr!("Altitude"), altitude);
            blocks(ui, tr!("Altitude spread"), spread, 150.0);
            blocks(ui, tr!("Largest radius"), size, 600.0);
            fraction(ui, tr!("Coverage"), coverage);
            amount(ui, tr!("Underside"), depth, 0.2..=3.0);
            seed(ui, s);
            ui.label(
                RichText::new(tr!("Top + Bottom → Layer output."))
                    .size(11.0)
                    .color(pal.weak),
            );
        }
        NodeKind::Overhang {
            reach,
            thickness,
            drop,
            variation,
            seed: s,
        } => {
            blocks(ui, tr!("Reach"), reach, 64.0);
            blocks(ui, tr!("Thickness"), thickness, 48.0);
            blocks(ui, tr!("Cliff drop"), drop, 160.0);
            fraction(ui, tr!("Variation"), variation);
            seed(ui, s);
            ui.label(
                RichText::new(tr!("Top + Bottom → Layer output."))
                    .size(11.0)
                    .color(pal.weak),
            );
        }
        NodeKind::LayerOutput { slot, palette } => {
            count(ui, tr!("Layer"), slot, 1..=8);
            choice(ui, tr!("Blocks from"), palette, &BlockPalette::ALL.map(|p| (p, crate::i18n::tr_name(p.label()))));
            ui.label(
                RichText::new(tr!("Rock between Top and Bottom, as a level of the map."))
                    .size(11.0)
                    .color(pal.weak),
            );
        }
        NodeKind::ColorOutput { blocks: b, palette } => {
            flag(ui, tr!("Real blocks"), b);
            if *b {
                choice(ui, tr!("From"), palette, &BlockPalette::ALL.map(|p| (p, crate::i18n::tr_name(p.label()))));
            }
            ui.label(
                RichText::new(tr!("Each column takes the closest real block."))
                    .size(11.0)
                    .color(pal.weak),
            );
        }
    }
}

/// A group's title in the render settings: its icon and name, and a note
/// on the right, or under them where it does not fit beside.
pub fn titled(ui: &mut Ui, icon: Icon, name: &str, note: &str) {
    let pal = Palette::of(ui);
    let width = |ui: &Ui, text: &str, size: f32| ui.painter().layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), pal.text).size().x;
    let beside = note.is_empty() || width(ui, name, 13.0) + width(ui, note, 11.0) + 40.0 <= ui.available_width();
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(icon).to_string()).size(13.0).color(pal.weak));
        ui.label(RichText::new(name).size(13.0).medium());
        if beside && !note.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(note).size(11.0).color(pal.weak));
            });
        }
    });
    if !beside {
        ui.label(RichText::new(note).size(11.0).color(pal.weak));
    }
    ui.add_space(2.0);
}

/// The levels an overlay conversion writes to (what the graph digs, what it
/// raises), made if they are missing; without `wanted`, they are removed:
/// the graph is then written to the ground itself. Called at the start of
/// the history step. Returns their indices.
pub(crate) fn overlay_levels(doc: &mut Document, wanted: bool) -> Option<(usize, usize)> {
    let find = |doc: &Document, side: massif_core::graph::Overlay| doc.dimension().levels.iter().position(|l| l.kind == side.kind() && l.name == side.level_name());
    let sides = [massif_core::graph::Overlay::Dug, massif_core::graph::Overlay::Raised];
    let present = sides.map(|side| find(doc, side));
    if wanted && present.iter().any(Option::is_none) || !wanted && present.iter().any(Option::is_some) {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        let dim = edit.structure();
        for side in sides {
            let at = dim.levels.iter().position(|l| l.kind == side.kind() && l.name == side.level_name());
            match (wanted, at) {
                (true, None) => {
                    dim.add_level(side.level_name(), side.kind());
                }
                (false, Some(k)) => {
                    dim.remove_level(k);
                }
                _ => {}
            }
        }
    }
    wanted.then(|| (find(doc, massif_core::graph::Overlay::Dug).expect("just made"), find(doc, massif_core::graph::Overlay::Raised).expect("just made")))
}

/// How a template's landscape is best seen: its sun, sky, clouds and sea.
pub(crate) fn template_render(t: Template, sea: f32, exaggeration: f32) -> crate::terrain_view::RenderSettings {
    let d = crate::terrain_view::RenderSettings {
        exaggeration,
        ..crate::terrain_view::RenderSettings::default()
    };
    let clouds = |cover: f32, base: f32, thickness: f32, size: f32| crate::terrain_view::RenderSettings {
        clouds: true,
        cloud_cover: cover,
        cloud_base: sea + base,
        cloud_thickness: thickness,
        cloud_size: size,
        ..d
    };
    match t {
        Template::Alpine => clouds(0.38, 170.0, 80.0, 300.0),
        Template::Archipelago => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 210.0,
            sun_elevation: 52.0,
            haze: 0.35,
            ..clouds(0.32, 110.0, 50.0, 200.0)
        },
        Template::KarstTowers => crate::terrain_view::RenderSettings {
            sea: true,
            cloud_density: 1.3,
            sun_azimuth: 115.0,
            sun_elevation: 32.0,
            haze: 0.7,
            ..clouds(0.5, 55.0, 45.0, 150.0)
        },
        Template::FloatingMountains => crate::terrain_view::RenderSettings {
            sea: true,
            cloud_density: 1.25,
            sun_azimuth: 120.0,
            sun_elevation: 30.0,
            haze: 0.6,
            ..clouds(0.5, 70.0, 45.0, 160.0)
        },
        Template::RiverValley => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 140.0,
            sun_elevation: 26.0,
            haze: 0.4,
            ..clouds(0.36, 190.0, 70.0, 280.0)
        },
        Template::DesertCanyon => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 250.0,
            sun_elevation: 9.0,
            haze: 0.4,
            ..d
        },
        Template::VolcanicIsland => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 300.0,
            sun_elevation: 30.0,
            haze: 0.45,
            ..clouds(0.42, 150.0, 70.0, 240.0)
        },
        Template::DuneSea => crate::terrain_view::RenderSettings {
            sun_azimuth: 100.0,
            sun_elevation: 12.0,
            haze: 0.5,
            ..d
        },
        Template::RollingHills => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 200.0,
            sun_elevation: 45.0,
            ..clouds(0.45, 130.0, 70.0, 260.0)
        },
        Template::Continents => crate::terrain_view::RenderSettings {
            sea: true,
            sun_azimuth: 160.0,
            sun_elevation: 34.0,
            haze: 0.45,
            ..clouds(0.34, 150.0, 60.0, 320.0)
        },
        Template::PaintMap | Template::MapRivers => crate::terrain_view::RenderSettings { sea: true, ..d },
    }
}
