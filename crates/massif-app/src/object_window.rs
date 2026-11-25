//! An object layer's settings window: on the left, the tall list of its
//! objects (hundreds of schematics), searched and sorted, picked one or
//! several at a time; on the right, the layer's settings and under them
//! those of the objects picked.

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::object_list::{Picked, picked, set_picked};
use crate::panels::{hint, section};
use crate::theme::{self, Icon, Palette};
use crate::tools::ToolState;
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::registry::Registry;

/// How the list is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// As the layer has them (the order they were imported in).
    #[default]
    Layer,
    Name,
    /// Tallest first.
    Size,
    /// Most often first.
    Frequency,
}

const ROW_H: f32 = 34.0;
/// The narrowest each pane goes.
const LIST_MIN: f32 = 260.0;
const SETTINGS_MIN: f32 = 300.0;

/// The rows of layer `k` shown, in order: those `query` finds, sorted.
pub fn order(reg: &Registry, k: usize, query: &str, sort: Sort) -> Vec<usize> {
    let layer = &reg.object_layers[k];
    let needle = query.trim().to_lowercase();
    let def = |row: usize| reg.objects.get(layer.objects[row].0 as usize);
    let mut rows: Vec<usize> = (0..layer.objects.len()).filter(|&r| needle.is_empty() || def(r).is_some_and(|d| d.name.to_lowercase().contains(&needle) || d.source.to_lowercase().contains(&needle))).collect();
    match sort {
        Sort::Layer => {}
        Sort::Name => rows.sort_by_cached_key(|&r| def(r).map(|d| d.name.to_lowercase()).unwrap_or_default()),
        Sort::Size => rows.sort_by_key(|&r| std::cmp::Reverse(def(r).map_or(0, |d| d.size[1]))),
        Sort::Frequency => rows.sort_by_key(|&r| std::cmp::Reverse(layer.objects[r].1)),
    }
    rows
}

/// The whole window's content, `size` big: the list on the left, the
/// settings on the right, each scrolling on its own.
pub fn window(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, size: egui::Vec2) {
    let k = tools.object_layer.checked_sub(1).map(|k| k as usize);
    if k.and_then(|k| doc.world.registry.object_layers.get(k)).is_none_or(|l| l.removed) {
        hint(ui, tr!("No objects yet: import some to paint them."));
        return;
    }
    let k = k.expect("checked above");
    let gap = 16.0;
    let list_w = (size.x * 0.46).clamp(LIST_MIN, 520.0).min((size.x - gap - SETTINGS_MIN).max(LIST_MIN));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let left = Rect::from_min_size(rect.min, vec2(list_w, size.y));
    let right = Rect::from_min_max(pos2(left.right() + gap, rect.top()), rect.max);
    let pal = Palette::of(ui);
    ui.painter().vline(left.right() + gap / 2.0, rect.y_range(), Stroke::new(1.0, pal.border));
    let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::top_down(egui::Align::Min)));
    let before = (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len());
    let add = schematics_pane(&mut lui, &mut doc.world.registry, k);
    let mut rui = ui.new_child(egui::UiBuilder::new().max_rect(right).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("object layer settings", k)).auto_shrink([false, false]).max_height(right.height()).show(&mut rui, |ui| {
        crate::panels::object_layer_settings(ui, tools, doc, blocks, false);
        if doc.world.registry.object_layers.get(k).is_some_and(|l| !l.removed) {
            selected_settings(ui, &mut doc.world.registry, k);
        }
    });
    if (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len()) != before {
        doc.mark_changed();
    }
    if add {
        let files = rfd::FileDialog::new().set_title(tr!("Add objects")).add_filter(tr!("Schematics"), &massif_objects::EXTENSIONS).pick_files().unwrap_or_default();
        if !files.is_empty() {
            crate::panels::import_objects(doc, tools, &files, Some(k as u16 + 1), blocks.catalogue.as_deref());
        }
    }
}

/// The settings of the objects picked in the list, and taking them out of
/// the layer.
fn selected_settings(ui: &mut Ui, reg: &mut Registry, k: usize) {
    let picked = picked(ui.ctx(), k);
    let rows: Vec<usize> = picked.rows.iter().copied().filter(|&r| r < reg.object_layers[k].objects.len()).collect();
    ui.add_space(8.0);
    section(ui, Icon::Boxes, &trn!("{n} object picked", "{n} objects picked", n = rows.len()));
    if rows.is_empty() {
        hint(ui, tr!("Pick objects in the list to set how often each stands, its size, its origin and its own rules."));
        return;
    }
    crate::object_list::editor(ui, reg, k, &rows);
    ui.add_space(4.0);
    let remove = crate::widgets::button(ui, theme::with_icon(Icon::X, &trn!("Take {n} out of the layer", "Take {n} out of the layer", n = rows.len())))
        .on_hover_text(tr!("Its objects stay in the map's library; they no longer stand with this layer"))
        .clicked();
    if remove {
        let layer = &mut reg.object_layers[k];
        for &row in rows.iter().rev() {
            if row < layer.objects.len() {
                layer.objects.remove(row);
            }
        }
        set_picked(ui.ctx(), k, Picked::default());
    }
}

/// The list of layer `k`'s objects, as tall as the pane: a search, the
/// sort, a row each (its picture, name, size, measures and share), picked
/// as in the compact list. Returns whether Add objects… was clicked.
fn schematics_pane(ui: &mut Ui, reg: &mut Registry, k: usize) -> bool {
    let pal = Palette::of(ui);
    let query_id = egui::Id::unique(("object window query", k));
    let sort_id = egui::Id::unique(("object window sort", k));
    let mut query: String = ui.data(|d| d.get_temp(query_id)).unwrap_or_default();
    let mut sort: Sort = ui.data(|d| d.get_temp(sort_id)).unwrap_or_default();
    let count = reg.object_layers[k].objects.len();
    let mut add = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(trn!("{n} object", "{n} objects", n = count)).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add = ui.button(theme::with_icon(Icon::Plus, tr!("Add objects…"))).on_hover_text(tr!("Read more files into this layer")).clicked();
        });
    });
    ui.add_space(4.0);
    crate::widgets::search(ui, &mut query, tr!("Search objects"), ui.available_width());
    ui.add_space(4.0);
    let sorts = [Sort::Layer, Sort::Name, Sort::Size, Sort::Frequency];
    if let Some(i) = crate::widgets::button_rows(ui, "object window sort", &[tr!("Imported"), tr!("Name"), tr!("Height"), tr!("Frequency")], &sorts.map(|s| s == sort)) {
        sort = sorts[i];
    }
    ui.add_space(4.0);
    let rows = order(reg, k, &query, sort);
    let mut picked = picked(ui.ctx(), k);
    let total: f32 = reg.object_layers[k].objects.iter().map(|o| o.1 as f32).sum::<f32>().max(1.0);
    let classes = reg.object_layers[k].size_classes(reg);
    let list_id = egui::Id::unique(("object window list", k));
    let focused = ui.memory(|m| m.has_focus(list_id));
    let hint_h = 34.0;
    let height = (ui.available_height() - hint_h).max(ROW_H * 3.0);
    let frame = egui::Frame::new().fill(pal.well()).stroke(Stroke::new(1.0, if focused { pal.accent } else { pal.border })).corner_radius(theme::GROUP_RADIUS).inner_margin(4);
    let mut clicked = None;
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if rows.is_empty() {
            ui.set_min_height(height - 10.0);
            ui.label(RichText::new(if count == 0 { tr!("Nothing stands yet: add objects.") } else { tr!("No object of this name.") }).color(pal.weak));
            return;
        }
        egui::ScrollArea::vertical().id_salt(("object window scroll", k)).auto_shrink([false, false]).max_height(height - 10.0).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for &row in &rows[range] {
                let (index, weight) = reg.object_layers[k].objects[row];
                let Some(def) = reg.objects.get(index as usize) else { continue };
                let on = picked.rows.contains(&row);
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &def.name));
                let fill = if on {
                    pal.accent_soft
                } else if response.hovered() {
                    pal.header
                } else {
                    Color32::TRANSPARENT
                };
                let painter = ui.painter();
                painter.rect(rect, theme::TILE_RADIUS, fill, if on { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
                let texture = crate::panels::object_texture(ui, index as usize, def);
                let size = texture.size_vec2();
                let r = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(28.0, 28.0));
                let fit = (r.width() / size.x).min(r.height() / size.y);
                painter.image(texture.id(), Rect::from_center_size(r.center(), size * fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                // On the right: its share, its size; under its name, its
                // measures.
                let share = painter.layout_no_wrap(format!("{:.0}%", weight as f32 * 100.0 / total), egui::FontId::proportional(11.5), pal.weak);
                let class = classes.get(row).copied().unwrap_or(0);
                let class = if class == 0 { String::new() } else { crate::tree_sizes::label(reg, class as u16) };
                let class = painter.layout_no_wrap(class, egui::FontId::proportional(11.5), pal.weak);
                let share_w = 40.0;
                painter.galley(pos2(rect.right() - 6.0 - share.size().x, rect.center().y - share.size().y / 2.0), share, pal.weak);
                let class_w = class.size().x;
                painter.galley(pos2(rect.right() - 6.0 - share_w - 8.0 - class_w, rect.center().y - class.size().y / 2.0), class, pal.weak);
                let room = (rect.width() - 40.0 - share_w - class_w - 24.0).max(20.0);
                let mut job = egui::text::LayoutJob::simple_singleline(def.name.clone(), egui::FontId::proportional(12.5), pal.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let name = painter.fonts_mut(|f| f.layout_job(job));
                painter.galley(pos2(rect.left() + 38.0, rect.top() + 3.0), name, pal.text);
                painter.text(pos2(rect.left() + 38.0, rect.bottom() - 4.0), egui::Align2::LEFT_BOTTOM, format!("{} × {} × {}", def.size[0], def.size[1], def.size[2]), egui::FontId::proportional(10.5), pal.weak);
                let response = response.on_hover_text(format!("{} · {} × {} × {}\n{}", def.name, def.size[0], def.size[1], def.size[2], def.source));
                if response.clicked() {
                    clicked = Some(row);
                }
            }
        });
    });
    if let Some(row) = clicked {
        let (toggle, range) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        picked.click_in(&rows, row, toggle, range);
        ui.memory_mut(|m| m.request_focus(list_id));
    }
    if ui.memory(|m| m.has_focus(list_id)) {
        let keys = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.modifiers.shift, i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Escape)));
        if keys.0 || keys.1 {
            picked.step_in(&rows, keys.0, keys.2);
        }
        if keys.3 {
            picked.rows = rows.clone();
            picked.rows.sort_unstable();
        }
        if keys.4 {
            ui.memory_mut(|m| m.surrender_focus(list_id));
        }
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    ui.add(egui::Label::new(RichText::new(tr!("Click one to edit it; Shift+Click, Shift+Arrows or {cmd}+Click to edit several at once.", cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" })).size(11.0).color(pal.weak)));
    set_picked(ui.ctx(), k, picked);
    ui.data_mut(|d| {
        d.insert_temp(query_id, query);
        d.insert_temp(sort_id, sort);
    });
    add
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        let mut reg = Registry::vanilla();
        for (name, h) in [("Birch", 7), ("Acacia", 9), ("Oak", 12), ("Bush", 2)] {
            reg.objects.push(massif_core::objects::ObjectDef::new(name, [3, h, 3], vec!["minecraft:air".into(), "minecraft:oak_log".into()], vec![1, 1]));
        }
        let layer = massif_core::objects::ObjectLayerDef::new("Forest", &[0, 1, 2, 3], &reg);
        reg.add_object_layer(layer);
        let n = reg.object_layers.len() - 1;
        reg.object_layers[n].objects[2].1 = 40;
        reg
    }

    /// The list sorts by name, height or frequency, and the search keeps
    /// the names it finds.
    #[test]
    fn the_list_sorts_and_searches() {
        let reg = registry();
        let k = reg.object_layers.len() - 1;
        let names = |rows: Vec<usize>| rows.iter().map(|&r| reg.objects[reg.object_layers[k].objects[r].0 as usize].name.clone()).collect::<Vec<_>>();
        assert_eq!(names(order(&reg, k, "", Sort::Layer)), ["Birch", "Acacia", "Oak", "Bush"]);
        assert_eq!(names(order(&reg, k, "", Sort::Name)), ["Acacia", "Birch", "Bush", "Oak"]);
        assert_eq!(names(order(&reg, k, "", Sort::Size)), ["Oak", "Acacia", "Birch", "Bush"]);
        assert_eq!(names(order(&reg, k, "", Sort::Frequency))[0], "Oak");
        assert_eq!(names(order(&reg, k, "b", Sort::Name)), ["Birch", "Bush"]);
    }

    /// Shift+Click picks along the order shown, not the layer's.
    #[test]
    fn a_range_runs_along_the_order_shown() {
        let reg = registry();
        let k = reg.object_layers.len() - 1;
        let shown = order(&reg, k, "", Sort::Name); // Acacia 1, Birch 0, Bush 3, Oak 2
        let mut p = Picked::default();
        p.click_in(&shown, 1, false, false);
        p.click_in(&shown, 3, false, true);
        assert_eq!(p.rows, vec![0, 1, 3], "Acacia, Birch, Bush");
        p.step_in(&shown, false, true);
        assert_eq!(p.rows, vec![0, 1, 2, 3], "Shift+Down reaches Oak");
    }
}
