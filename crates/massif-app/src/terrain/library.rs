//! The library: every type of node by family, found by its letters, added
//! by a click or dragged onto the graph; and quick starts, example graphs
//! to begin from.

use super::panel::TerrainPanel;
use super::sockets;
use crate::theme::{self, Icon, Palette};
use crate::widgets;
use egui::{Align2, FontId, Rect, Sense, Ui, vec2};
use massif_terrain::model::Graph;
use massif_terrain::registry::{self, Family, NodeSpec};

/// Example graphs to start from: their name, and the graph.
pub fn quick_starts() -> Vec<(&'static str, &'static str)> {
    vec![
        (tn!("Alps"), include_str!("../../../massif-terrain/examples/graphs/m4-alpine-colors.json")),
        (tn!("Continent"), include_str!("../../../massif-terrain/examples/graphs/m4-climate.json")),
        (tn!("Archipelago"), include_str!("../../../massif-terrain/examples/graphs/m2-archipelago.json")),
        (tn!("Desert mesas"), include_str!("../../../massif-terrain/examples/graphs/m2-mesas.json")),
        (tn!("Canyon"), include_str!("../../../massif-terrain/examples/graphs/m2-canyon.json")),
        (tn!("Coastline"), include_str!("../../../massif-terrain/examples/graphs/m2-coastline.json")),
        (tn!("Volcano"), include_str!("../../../massif-terrain/examples/graphs/m2-volcano.json")),
        (tn!("Mountain range"), include_str!("../../../massif-terrain/examples/graphs/m2-range.json")),
    ]
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui, graph_rect: Rect) {
    let pal = Palette::of(ui);
    let card = ui.max_rect();
    super::panel::title(ui, card, t!("Library"));
    let body = Rect::from_min_max(card.min + vec2(10.0, 34.0), card.max - vec2(10.0, 8.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    widgets::search(&mut ui, &mut p.library_query, t!("Search nodes…"), body.width());
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("terrain library").auto_shrink([false, false]).show(&mut ui, |ui| {
        ui.set_width(body.width() - 2.0);
        let query = p.library_query.trim().to_owned();
        if !query.is_empty() {
            let hits = super::canvas::find(&query, None);
            for spec in hits.iter().take(40) {
                entry(p, ui, spec, graph_rect, &pal);
            }
            if hits.is_empty() {
                ui.label(egui::RichText::new(t!("No node matches.")).size(12.0).color(pal.weak));
            }
            return;
        }
        for family in Family::ALL {
            let specs: Vec<&'static NodeSpec> = registry::all().iter().copied().filter(|s| s.family == family && super::canvas::offered(s)).collect();
            if specs.is_empty() {
                continue;
            }
            let label = family.label();
            let open = p.library_open.contains(label);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, theme::TILE_RADIUS, pal.accent_soft);
            }
            let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
            ui.painter().text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, theme::glyph(chevron).to_string(), FontId::proportional(10.5), pal.weak);
            ui.painter().text(r.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, theme::glyph(sockets::family_icon(family)).to_string(), FontId::proportional(12.0), sockets::family(family));
            ui.painter().text(r.left_center() + vec2(40.0, 0.0), Align2::LEFT_CENTER, super::text::name(label), FontId::proportional(12.5), pal.text);
            ui.painter().text(r.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, specs.len().to_string(), FontId::proportional(11.0), pal.weak);
            if resp.clicked() {
                if open {
                    p.library_open.remove(label);
                } else {
                    p.library_open.insert(label);
                }
            }
            if open {
                for spec in specs {
                    entry(p, ui, spec, graph_rect, &pal);
                }
                ui.add_space(4.0);
            }
        }
        ui.add_space(12.0);
        ui.label(egui::RichText::new(t!("Quick start").to_uppercase()).size(10.5).color(pal.weak));
        ui.add_space(4.0);
        let w = (ui.available_width() - 6.0) / 2.0;
        let starts = quick_starts();
        for pair in starts.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (name, json) in pair {
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
                    let fill = if resp.hovered() { pal.accent_soft } else { pal.well() };
                    ui.painter().rect(r, theme::TILE_RADIUS, fill, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                    ui.painter().text(r.left_top() + vec2(9.0, 9.0), Align2::LEFT_TOP, theme::glyph(Icon::MountainSnow).to_string(), FontId::proportional(12.0), sockets::family(Family::Shape));
                    ui.painter().text(r.left_bottom() + vec2(9.0, -8.0), Align2::LEFT_BOTTOM, super::text::name(name), FontId::proportional(11.5), pal.text);
                    if resp.on_hover_text(t!("Start from this example: it replaces the graph (undo brings it back)")).clicked()
                        && let Ok(g) = Graph::from_json(json)
                    {
                        load_example(p, g);
                    }
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// Replaces the graph with an example, as one step that can be undone.
pub fn load_example(p: &mut TerrainPanel, g: Graph) {
    let keep_size = p.space.size;
    *p.edit() = g;
    if let Some(size) = p.graph.terrain.size {
        p.space.size = size;
    } else {
        p.space.size = keep_size;
    }
    p.space.seed = p.graph.terrain.seed;
    p.layout = Default::default();
    p.auto_place();
    p.selected.clear();
    p.viewed = None;
    p.view = Default::default();
    p.fit_pending = true;
}

fn entry(p: &mut TerrainPanel, ui: &mut Ui, spec: &'static NodeSpec, graph_rect: Rect, pal: &Palette) {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.painter().rect_filled(r, theme::TILE_RADIUS, pal.accent_soft);
    }
    ui.painter().circle_filled(r.left_center() + vec2(26.0, 0.0), 3.0, sockets::family(spec.family));
    ui.painter().with_clip_rect(r).text(r.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, super::text::name(spec.label), FontId::proportional(12.0), pal.text);
    let resp = resp.on_hover_text(super::text::name(spec.summary));
    // A click adds it in the middle of the graph's view; a drag, where it
    // is let go.
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(q) = ui.input(|i| i.pointer.hover_pos()) {
            ui.painter().text(q + vec2(12.0, 0.0), Align2::LEFT_CENTER, super::text::name(spec.label), FontId::proportional(12.0), pal.accent);
        }
    }
    let at_screen = if resp.drag_stopped() {
        ui.input(|i| i.pointer.interact_pos()).filter(|q| graph_rect.contains(*q))
    } else if resp.clicked() {
        Some(graph_rect.center())
    } else {
        None
    };
    if let Some(q) = at_screen {
        let origin = graph_rect.min + vec2(1.0, 36.0);
        let g = (q - origin - p.view.offset) / p.view.zoom;
        let n = p.selected.len() as f32;
        p.add_node(spec.id, [g.x - 80.0 + n * 0.0, g.y - 40.0]);
    }
}
