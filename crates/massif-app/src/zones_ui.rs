//! Paint terrain's zones, shown and edited in Tool Settings: which terrain
//! at which height, on which slope, under the water. A preset (Plains,
//! Mountain, Mesa…) or the map's own zones, kept with the map; zones of
//! one's own saved under a name for other maps.

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::panels::{hint, section};
use crate::theme::{self, Icon, Palette};
use crate::tools::{SavedZones, ToolState};
use egui::{RichText, Sense, Ui, vec2};
use massif_core::paint_rules::{HIGHEST, LOWEST, Preset, Water, Zone, ZoneRules};

/// The name of a preset, in the language shown.
pub fn preset_name(p: Preset) -> &'static str {
    match p {
        Preset::Classic => tr!("Classic"),
        Preset::Plains => tr!("Plains"),
        Preset::Mountain => tr!("Mountain"),
        Preset::Mesa => tr!("Mesa"),
        Preset::Desert => tr!("Desert"),
        Preset::Taiga => tr!("Taiga"),
        Preset::Coast => tr!("Coast"),
        Preset::Custom => tr!("Custom"),
    }
}

/// Where a zone is, said in a few words: its heights (Y on the map) or its
/// depth under the water, and its slope.
pub fn zone_summary(z: &Zone, sea: i32) -> String {
    let (lo, hi) = (z.low <= LOWEST, z.high >= HIGHEST);
    let mut out = match z.water {
        Water::Dry => match (lo, hi) {
            (true, true) => tr!("any height").to_owned(),
            (true, false) => tr!("up to y {y}", y = sea + z.high),
            (false, true) => tr!("from y {y}", y = sea + z.low),
            (false, false) => tr!("y {a}–{b}", a = sea + z.low, b = sea + z.high),
        },
        Water::Under => match (z.low <= 1, hi) {
            (true, true) => tr!("under water").to_owned(),
            (true, false) => tr!("under water, up to {d} deep", d = z.high),
            (false, true) => tr!("under water, {d} deep and more", d = z.low),
            (false, false) => tr!("under water, {a}–{b} deep", a = z.low, b = z.high),
        },
    };
    let (a, b) = (z.min_slope.round() as i32, z.max_slope.round() as i32);
    let slope = match (a <= 0, b >= 90) {
        (true, true) => None,
        (true, false) => Some(tr!("under {b}°", b)),
        (false, true) => Some(tr!("over {a}°", a)),
        (false, false) => Some(tr!("{a}–{b}°", a, b)),
    };
    if let Some(s) = slope {
        out.push_str(" · ");
        out.push_str(&s);
    }
    out
}

/// The zones' settings: the preset, the zones in order, editing one's own,
/// saving them.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    section(ui, Icon::Mountain, tr!("Zones"));
    let mut rules = doc.world.meta.paint_rules.clone().unwrap_or_default();
    let before = rules.clone();
    // The presets, in as few rows as the panel takes.
    let labels: Vec<&str> = Preset::ALL.iter().map(|p| preset_name(*p)).collect();
    let on: Vec<bool> = Preset::ALL.iter().map(|p| rules.preset == *p).collect();
    if let Some(i) = crate::widgets::button_rows(ui, "zone presets", &labels, &on) {
        if Preset::ALL[i] == Preset::Custom {
            rules.make_custom();
        } else {
            rules.preset = Preset::ALL[i];
        }
    }
    saved_zones(ui, tools, &mut rules);
    ui.add_space(4.0);
    let sea = doc.world.meta.sea_level;
    let zones = rules.zones();
    let custom = rules.preset == Preset::Custom;
    let selected_id = egui::Id::unique("zone selected");
    let mut selected: Option<usize> = ui.data(|d| d.get_temp(selected_id)).filter(|k| custom && *k < zones.len());
    egui::ScrollArea::vertical().id_salt("zones list").max_height(230.0).show(ui, |ui| {
        crate::widgets::well(ui, |ui| {
            if zones.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(tr!("No zones yet: add one below.")).color(pal.weak));
                ui.add_space(8.0);
            }
            for (k, z) in zones.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), if custom { Sense::click() } else { Sense::hover() });
                if k > 0 {
                    ui.painter().hline(rect.x_range().expand(12.0), rect.top(), egui::Stroke::new(1.0, pal.border));
                }
                let on = selected == Some(k);
                if on || (custom && response.hovered()) {
                    ui.painter().rect_filled(rect.expand2(vec2(12.0, 0.0)), 0.0, if on { pal.accent_soft } else { theme::mix(pal.panel, pal.text, 0.05) });
                }
                let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 9.0, rect.center().y), vec2(18.0, 18.0));
                match doc.world.registry.terrain_by_name(&z.terrain) {
                    Some(t) => crate::panels::terrain_icon(ui, icon, &doc.world.registry, t.0, blocks),
                    None => {
                        ui.painter().rect_stroke(icon, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                    }
                }
                let name = crate::i18n::tr_name(&z.terrain);
                let summary = zone_summary(z, sea);
                let font = egui::FontId::proportional(12.5);
                let small = egui::FontId::proportional(11.5);
                let right = ui.painter().layout_no_wrap(summary.clone(), small, pal.weak);
                let room = (rect.width() - 30.0 - right.size().x - 8.0).max(20.0);
                let left = ui.painter().layout(name.to_owned(), font, pal.text, room);
                let left = if left.rows.len() > 1 { ui.painter().layout_no_wrap(format!("{}…", name.chars().take(14).collect::<String>()), egui::FontId::proportional(12.5), pal.text) } else { left };
                ui.painter().galley(egui::pos2(rect.left() + 26.0, rect.center().y - left.size().y / 2.0), left, pal.text);
                ui.painter().galley(egui::pos2(rect.right() - right.size().x, rect.center().y - right.size().y / 2.0), right, pal.weak);
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, format!("{name}: {summary}")));
                if response.clicked() {
                    selected = if on { None } else { Some(k) };
                }
            }
        });
    });
    ui.add_space(4.0);
    if custom {
        if let Some(k) = selected {
            zone_editor(ui, tools, doc, blocks, &mut rules, k, &mut selected);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(theme::with_icon(Icon::Plus, tr!("Add a zone"))).clicked() {
                let terrain = doc.world.registry.terrain(massif_core::registry::TerrainId(tools.terrain)).name.clone();
                rules.custom.insert(0, Zone { terrain, water: Water::Dry, low: 0, high: 20, min_slope: 0.0, max_slope: 90.0 });
                selected = Some(0);
            }
        });
    } else if ui.button(theme::with_icon(Icon::Pencil, tr!("Edit these zones"))).on_hover_text(tr!("Makes them the map's own, to change, add or take out")).clicked() {
        rules.make_custom();
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        crate::widgets::name_label(ui, tr!("Ragged edges"), vec2(90.0, 18.0));
        ui.add(crate::widgets::Field::new(&mut rules.ragged, 0.0..=16.0).suffix(tr!(" blocks")).max_decimals(0));
    });
    hint(ui, tr!("Read from the top: the first zone that takes a column paints it, a column no zone takes keeps its terrain. Heights are the top block's Y, slopes in degrees. Under the water, grass, snow and the like never go: the zones made for it, or else sand, clay, dirt and gravel by depth, as in the game."));
    ui.data_mut(|d| {
        if let Some(k) = selected {
            d.insert_temp(selected_id, k);
        } else {
            d.remove::<usize>(selected_id);
        }
    });
    if rules != before {
        doc.world.meta.paint_rules = Some(rules);
        doc.mark_changed();
    }
}

/// The zone chosen of the map's own: its terrain, where, its slope; moved
/// up or down the list, or taken out.
#[allow(clippy::too_many_arguments)]
fn zone_editor(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, rules: &mut ZoneRules, k: usize, selected: &mut Option<usize>) {
    let sea = doc.world.meta.sea_level;
    let (min_y, max_y) = (doc.dimension().min_y, doc.dimension().max_y);
    let n = rules.custom.len();
    let mut terrain = None;
    crate::widgets::well(ui, |ui| {
        crate::widgets::row(ui, tr!("Terrain"), |ui| {
            let z = &rules.custom[k];
            let t = doc.world.registry.terrain_by_name(&z.terrain);
            let label = crate::i18n::tr_name(&z.terrain).to_owned();
            let chip = crate::panels::terrain_chip(ui, &doc.world.registry, t.map(|t| t.0), &label, blocks, 150.0);
            terrain = crate::panels::terrain_popup(&chip, egui::Id::unique(("zone terrain", k)), tools, doc, blocks, Some(&label));
        });
        let z = &mut rules.custom[k];
        crate::widgets::row(ui, tr!("Where"), |ui| {
            let on = [z.water == Water::Dry, z.water == Water::Under];
            if let Some(i) = crate::widgets::button_group(ui, "zone water", &[tr!("Dry land"), tr!("Under water")], &on) {
                let water = if i == 0 { Water::Dry } else { Water::Under };
                if water != z.water {
                    z.water = water;
                    (z.low, z.high) = if water == Water::Under { (1, HIGHEST) } else { (LOWEST, HIGHEST) };
                }
            }
        });
        match z.water {
            Water::Dry => {
                let range = (min_y - sea)..=(max_y - sea);
                let mut lo = z.low.clamp(*range.start(), *range.end()) + sea;
                let mut hi = z.high.clamp(*range.start(), *range.end()) + sea;
                crate::widgets::row(ui, tr!("From y"), |ui| {
                    if crate::widgets::drag(ui, egui::DragValue::new(&mut lo).range(min_y..=max_y)).changed() {
                        z.low = if lo <= min_y { LOWEST } else { lo - sea };
                        z.high = z.high.max(z.low);
                    }
                });
                crate::widgets::row(ui, tr!("To y"), |ui| {
                    if crate::widgets::drag(ui, egui::DragValue::new(&mut hi).range(min_y..=max_y)).changed() {
                        z.high = if hi >= max_y { HIGHEST } else { hi - sea };
                        z.low = z.low.min(z.high);
                    }
                });
            }
            Water::Under => {
                let mut lo = z.low.clamp(1, 256);
                let mut hi = z.high.clamp(1, 256);
                crate::widgets::row(ui, tr!("From depth"), |ui| {
                    if crate::widgets::drag(ui, egui::DragValue::new(&mut lo).range(1..=256)).changed() {
                        z.low = lo;
                        z.high = z.high.max(lo);
                    }
                });
                crate::widgets::row(ui, tr!("To depth"), |ui| {
                    if crate::widgets::drag(ui, egui::DragValue::new(&mut hi).range(1..=256)).changed() {
                        z.high = if hi >= 256 { HIGHEST } else { hi };
                        z.low = z.low.min(z.high);
                    }
                });
            }
        }
        crate::widgets::row(ui, tr!("Slope from"), |ui| {
            ui.add(crate::widgets::Field::new(&mut z.min_slope, 0.0..=90.0).suffix("°").max_decimals(0));
        });
        crate::widgets::row(ui, tr!("Slope to"), |ui| {
            ui.add(crate::widgets::Field::new(&mut z.max_slope, 0.0..=90.0).suffix("°").max_decimals(0));
        });
        if z.max_slope < z.min_slope {
            z.max_slope = z.min_slope;
        }
    });
    if let Some(t) = terrain {
        rules.custom[k].terrain = doc.world.registry.terrain(t).name.clone();
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(k > 0, egui::Button::new(theme::with_icon(Icon::ArrowUp, tr!("Up")))).on_hover_text(tr!("Read before the zone above")).clicked() {
            rules.custom.swap(k, k - 1);
            *selected = Some(k - 1);
        }
        if ui.add_enabled(k + 1 < n, egui::Button::new(theme::with_icon(Icon::ArrowDown, tr!("Down")))).clicked() {
            rules.custom.swap(k, k + 1);
            *selected = Some(k + 1);
        }
        if ui.button(theme::with_icon(Icon::Trash, tr!("Remove"))).clicked() {
            rules.custom.remove(k);
            *selected = None;
        }
    });
    ui.add_space(4.0);
}

/// Zones saved under a name, to load on any map; saving the map's own.
fn saved_zones(ui: &mut Ui, tools: &mut ToolState, rules: &mut ZoneRules) {
    let custom = rules.preset == Preset::Custom;
    if tools.saved_zones.is_empty() && !custom {
        return;
    }
    let (mut load, mut forget) = (None, None);
    ui.horizontal(|ui| {
        crate::widgets::name_label(ui, tr!("Saved"), vec2(72.0, 18.0));
        let width = (ui.available_width() - 40.0).clamp(90.0, 190.0);
        let current = tools.saved_zones.iter().find(|s| s.rules == *rules).map(|s| s.name.clone());
        crate::widgets::combo("saved zones").selected_text(current.clone().unwrap_or_else(|| tr!("None").to_owned())).width(width).show_ui(ui, |ui| {
            if tools.saved_zones.is_empty() {
                ui.label(RichText::new(tr!("Nothing saved yet")).color(Palette::of(ui).weak));
            }
            for (k, s) in tools.saved_zones.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(current.as_deref() == Some(s.name.as_str()), &s.name).clicked() {
                        load = Some(s.rules.clone());
                    }
                    if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Forget it")).clicked() {
                        forget = Some(k);
                    }
                });
            }
        });
        if custom {
            let name_id = egui::Id::unique("zones save name");
            ui.menu_button(theme::glyph(Icon::Save).to_string(), |ui| {
                ui.set_min_width(220.0);
                ui.label(RichText::new(tr!("Save these zones as")).size(12.0));
                let mut name: String = ui.ctx().data(|d| d.get_temp(name_id)).unwrap_or_else(|| current.clone().unwrap_or_default());
                let edit = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Its name")).desired_width(200.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let taken = tools.saved_zones.iter().position(|s| s.name.eq_ignore_ascii_case(name.trim()));
                let label = if taken.is_some() { tr!("Replace it") } else { tr!("Save") };
                if (ui.add_enabled(!name.trim().is_empty(), egui::Button::new(label)).clicked() || enter) && !name.trim().is_empty() {
                    let saved = SavedZones { name: name.trim().to_owned(), rules: rules.clone() };
                    match taken {
                        Some(k) => tools.saved_zones[k] = saved,
                        None => tools.saved_zones.push(saved),
                    }
                    ui.ctx().data_mut(|d| d.remove::<String>(name_id));
                    ui.close();
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(name_id, name));
                }
            })
            .response
            .on_hover_text(tr!("Save these zones in the settings, to paint other maps with them"));
        }
    });
    if let Some(r) = load {
        *rules = r;
    }
    if let Some(k) = forget {
        tools.saved_zones.remove(k);
    }
}
