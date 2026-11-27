//! The selected node's settings, drawn from the registry: a well for each
//! section (the main settings, Shape, Detail, Advanced on demand), an
//! editor for every kind of setting, the ◇ that lets a field drive a
//! number over the map, and the help of the setting under the pointer. A
//! composite shows the settings it exposes, grouped as it says; inside an
//! opened composite, a setting can be exposed on it, and with nothing
//! selected its inputs, outputs and exposed settings show. A frame shows
//! its title, colour and note.

use super::panel::{TerrainPanel, base_kind};
use super::shape::NodeShape;
use super::sockets;
use crate::theme::{self, Icon, Palette};
use crate::widgets;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_terrain::compile::Diagnostic;
use massif_terrain::format::Op;
use massif_terrain::model::{Component, Exposed, OutputDecl, PortDecl};
use massif_terrain::params::{Params, Point, Value, color_hex, eval_gradient, eval_profile};
use massif_terrain::registry::{NodeSpec, ParamKind, ParamSpec, PortKind, Section, Unit};

/// A setting as its JSON.
pub fn encode(spec: &ParamSpec, v: &Value) -> serde_json::Value {
    match (v, spec.kind) {
        (Value::Num(x), ParamKind::Int { .. }) => serde_json::Value::from(x.round() as i64),
        (Value::Num(x), _) => serde_json::Value::from((x * 1e4).round() / 1e4),
        (Value::Bool(b), _) => serde_json::Value::from(*b),
        (Value::Choice(i), ParamKind::Choice { options, .. }) => serde_json::Value::from(options[(*i).min(options.len() - 1)].value),
        (Value::Choice(i), _) => serde_json::Value::from(*i),
        (Value::Seed(s), _) => serde_json::Value::from(*s),
        (Value::Color(c), _) => serde_json::Value::from(color_hex(*c)),
        (Value::Points(pts), ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. }) => serde_json::Value::Array(
            pts.iter()
                .map(|pt| {
                    let mut o = serde_json::Map::new();
                    o.insert("x".into(), ((pt.x * 1e4).round() / 1e4).into());
                    o.insert("z".into(), ((pt.z * 1e4).round() / 1e4).into());
                    for (f, v) in fields.iter().zip(&pt.values) {
                        o.insert(f.name.into(), (*v).into());
                    }
                    serde_json::Value::Object(o)
                })
                .collect(),
        ),
        (Value::Points(_), _) => serde_json::Value::Array(Vec::new()),
        (Value::Profile(pts), _) => serde_json::Value::Array(pts.iter().map(|p| serde_json::json!([(p[0] * 1e4).round() / 1e4, (p[1] * 1e4).round() / 1e4])).collect()),
        (Value::Gradient(stops), _) => serde_json::Value::Array(stops.iter().map(|(at, c)| serde_json::json!({ "at": (at * 1e4).round() / 1e4, "color": color_hex(*c) })).collect()),
        (Value::Text(t), _) => serde_json::Value::from(t.clone()),
    }
}

/// A node's settings with their defaults filled in.
pub fn decoded(p: &TerrainPanel, id: &str) -> Option<Params> {
    let spec = p.spec(id)?;
    let node = p.graph.nodes.get(id)?;
    let mut diags: Vec<Diagnostic> = Vec::new();
    Some(Params::decode(spec, id, &node.params, "", &mut diags))
}

/// A setting of a composite being exposed: what the dialog fills in.
#[derive(Debug, Clone, Default)]
pub struct Exposing {
    /// `node.setting` inside.
    pub target: String,
    pub name: String,
    pub label: String,
    pub help: String,
    pub category: String,
    pub advanced: bool,
    /// The range, for a number: `None` keeps the setting's own.
    pub range: Option<[f64; 2]>,
    pub error: Option<String>,
}

fn section_label(s: Section) -> &'static str {
    match s {
        Section::Main => "",
        Section::Shape => t!("Shape"),
        Section::Detail => t!("Detail"),
        Section::Advanced => t!("Advanced"),
    }
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let full = ui.max_rect();
    let foot_h = 54.0;
    let body = Rect::from_min_max(full.min + vec2(theme::PAD, 10.0), pos2(full.right() - theme::PAD, full.bottom() - foot_h));
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt("terrain props").auto_shrink([false, false]).show(&mut inner, |ui| {
        ui.set_width(body.width() - 4.0);
        let frame = p.selected_frame.clone().filter(|f| p.graph.frames.contains_key(f));
        match (p.selected.last().cloned().filter(|id| p.graph.nodes.contains_key(id)), frame) {
            (Some(id), _) => node_ui(p, ui, &id),
            (None, Some(f)) => frame_ui(p, ui, &f),
            (None, None) if !p.outer.is_empty() => composite_ui(p, ui),
            (None, None) => graph_ui(p, ui),
        }
        ui.add_space(12.0);
    });
    // The help of the setting under the pointer, at the card's foot.
    let foot = Rect::from_min_max(pos2(full.left() + theme::PAD, full.bottom() - foot_h + 6.0), pos2(full.right() - theme::PAD, full.bottom() - 8.0));
    ui.painter().hline(full.x_range().shrink(1.0), foot.top() - 4.0, Stroke::new(1.0, pal.border));
    if let Some(help) = p.hovered_help {
        let g = ui.painter().layout(super::text::name_owned(help), FontId::proportional(11.5), pal.weak, foot.width());
        ui.painter().with_clip_rect(foot).galley(foot.min, g, pal.weak);
    }
}

/// Settings of the whole graph, shown when no node is selected.
fn graph_ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    ui.label(egui::RichText::new(t!("Terrain graph")).size(14.0).strong());
    ui.add_space(2.0);
    ui.label(egui::RichText::new(t!("Select a node to edit its settings. These settings are the whole graph's.")).size(11.5).color(pal.weak));
    ui.add_space(10.0);
    let mut size = p.space.size as f64;
    let mut seed = p.graph.terrain.seed as f64;
    let mut sea = p.space.sea_level as f64;
    let mut sim = p.sim_res as f64;
    let mut changed = false;
    widgets::well(ui, |ui| {
        let top = ui.cursor().top();
        widgets::row(ui, t!("Terrain size"), |ui| {
            changed |= ui.add(widgets::Field::new(&mut size, 512.0..=32768.0).suffix(" b").logarithmic(true).width((ui.available_width() * 0.58).min(160.0))).changed();
        });
        hover_help(p, ui, top, "Side of the square of the world the graph makes, in blocks. Mountains keep their size in blocks when it changes.");
        let top = ui.cursor().top();
        widgets::row(ui, t!("Seed"), |ui| {
            if ui.button(theme::glyph(Icon::Dices).to_string()).on_hover_text(t!("Another seed")).clicked() {
                seed = (seed * 7919.0 + 104729.0) % 1_000_000.0;
                changed = true;
            }
            changed |= ui.add(widgets::Field::new(&mut seed, 0.0..=1_000_000.0).max_decimals(0).width((ui.available_width() * 0.45).min(120.0))).changed();
        });
        hover_help(p, ui, top, "The seed all of the graph's randomness starts from.");
        let top = ui.cursor().top();
        widgets::row(ui, t!("Sea level"), |ui| {
            changed |= ui.add(widgets::Field::new(&mut sea, -64.0..=320.0).suffix(" b").max_decimals(0).width((ui.available_width() * 0.58).min(160.0))).changed();
        });
        hover_help(p, ui, top, "Where the sea's surface lies, in blocks.");
        let top = ui.cursor().top();
        widgets::row(ui, t!("Simulation resolution"), |ui| {
            changed |= ui.add(widgets::Field::new(&mut sim, 256.0..=8192.0).suffix(" px").logarithmic(true).max_decimals(0).width((ui.available_width() * 0.58).min(160.0))).changed();
        });
        hover_help(p, ui, top, "Samples a side erosion and water run at in previews: finer is sharper and slower. Builds run at the map's own resolution.");
        let top = ui.cursor().top();
        widgets::row(ui, t!("Node thumbnails"), |ui| {
            let colour = p.thumb_surface == super::shade::Surface::Colour;
            if let Some(k) = widgets::button_group(ui, "terrain thumbs surface", &[t!("Clay"), t!("Colours")], &[!colour, colour]) {
                p.thumb_surface = if k == 0 { super::shade::Surface::Clay } else { super::shade::Surface::Colour };
            }
        });
        hover_help(p, ui, top, "How the nodes' thumbnails in the graph are drawn: in clay, or in the colours the graph gives the ground.");
    });
    if changed {
        let size = (size.round() as u32).clamp(512, 32768);
        p.sim_res = (sim.round() as u32).clamp(256, 8192);
        let g = p.edit();
        g.terrain.size = Some(size);
        g.terrain.seed = seed.round() as u64;
        g.terrain.sea_level = Some(sea as f32);
        p.space.size = size;
        p.space.seed = seed.round() as u64;
        p.space.sea_level = sea as f32;
    }
    let s = p.status();
    if !s.diagnostics.is_empty() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(t!("Problems")).size(12.0).strong());
        for d in s.diagnostics.iter().take(20) {
            ui.label(egui::RichText::new(d).size(11.0).color(pal.danger));
        }
    }
    if !p.notes.is_empty() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(t!("From the graph of Massif 1.2")).size(12.0).strong());
        for d in p.notes.iter().take(30) {
            ui.label(egui::RichText::new(d).size(11.0).color(pal.weak));
        }
    }
}

/// A text on the right of a row that leaves its name room: cut with an
/// ellipsis past six tenths of the row (whole on hover).
fn weak_text(ui: &mut Ui, text: &str, color: Color32) {
    let room = (ui.max_rect().width() * 0.6).max(40.0);
    let galley_w = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color).size().x;
    let w = galley_w.min(room);
    ui.add_sized(vec2(w, 18.0), egui::Label::new(egui::RichText::new(text).size(11.5).color(color)).truncate());
}

fn hover_help(p: &mut TerrainPanel, ui: &Ui, top: f32, help: &'static str) {
    let r = Rect::from_min_max(pos2(ui.max_rect().left(), top), pos2(ui.max_rect().right(), ui.cursor().top()));
    if ui.rect_contains_pointer(r) {
        p.hovered_help = Some(help);
    }
}

/// A help of a composite (written in the document, not known when
/// building), kept for the panel's foot.
fn leak_help(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static KEPT: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut kept = KEPT.get_or_init(Default::default).lock().expect("lock");
    if let Some(k) = kept.get(s) {
        return k;
    }
    let k: &'static str = Box::leak(s.to_owned().into_boxed_str());
    kept.insert(k);
    k
}

fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(theme::glyph(icon).to_string()).size(13.0)).frame(false)).on_hover_text(tip).clicked()
}

fn node_ui(p: &mut TerrainPanel, ui: &mut Ui, id: &str) {
    let pal = Palette::of(ui);
    let Some(shape) = p.shape(id) else { return };
    let read_only = p.read_only();
    // Title, family and what the node makes.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, sockets::family(shape.family));
        ui.add(egui::Label::new(egui::RichText::new(p.display_name(id)).size(14.0).strong()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !read_only && icon_button(ui, Icon::Trash2, t!("Remove the node")) {
                p.remove_nodes(&[id.to_owned()]);
            }
            if !read_only && icon_button(ui, Icon::Copy, t!("Duplicate (⌘D)")) {
                p.duplicate(&[id.to_owned()]);
            }
            if icon_button(ui, Icon::Eye, t!("View in the viewer (V)")) {
                p.viewed = Some((p.crumbs(), super::eval::Shown { node: id.to_owned(), port: None }));
            }
        });
    });
    if !p.graph.nodes.contains_key(id) {
        return;
    }
    ui.label(egui::RichText::new(&shape.summary).size(11.5).color(pal.weak));
    ui.add_space(6.0);
    // The name it shows (its id, by which wires and scripts name it, never
    // changes), and whether it is left out.
    let current = p.graph.nodes[id].name.clone().unwrap_or_default();
    let mut name = p.renaming_node.clone().filter(|_| ui.memory(|m| m.has_focus(ui.scope_id().with("node name")))).unwrap_or_else(|| current.clone());
    let is_output = shape.is_output();
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            let top = ui.cursor().top();
            widgets::row(ui, t!("Name"), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).id(ui.scope_id().with("node name")).hint_text(&shape.label).desired_width(150.0).font(FontId::proportional(12.0)));
                if r.changed() {
                    p.renaming_node = Some(name.clone());
                }
                if r.lost_focus() {
                    p.set_name(id, Some(name.clone()));
                    p.renaming_node = None;
                }
            });
            hover_help(p, ui, top, "The name the node shows. Wires and scripts name it by its id, which never changes.");
            widgets::row(ui, t!("Id"), |ui| {
                ui.label(egui::RichText::new(id).size(11.5).color(pal.weak).monospace());
            });
            if let Some((ms, bytes)) = p.cost(id) {
                let top = ui.cursor().top();
                widgets::row(ui, t!("Cost"), |ui| {
                    ui.label(egui::RichText::new(super::canvas::cost_text(ms, bytes)).size(11.5).color(if ms >= 1000.0 { pal.danger } else { pal.weak }));
                });
                hover_help(p, ui, top, "What making this node took the last time the graph changed: its time, and the memory of the tiles it made (a composite: all it holds). Thumbnails and the viewer ask for what they show only.");
            }
            if !is_output {
                let top = ui.cursor().top();
                let mut on = p.graph.nodes[id].bypass;
                widgets::row(ui, t!("Bypass"), |ui| {
                    if widgets::switch(ui, &mut on).changed() {
                        p.set_bypass(&[id.to_owned()], on);
                    }
                });
                hover_help(p, ui, top, "Leaves the node out: what it feeds takes its first input instead (B).");
            }
        });
    });
    let crumbs = p.crumbs();
    let broken = p.status().broken.get(&super::shape::unfolded_id(&crumbs, id)).cloned();
    if let Some(why) = broken {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(why).size(11.5).color(pal.danger));
    }
    let id = id.to_owned();
    if let Some((kind, comp, editable)) = shape.composite() {
        let (kind, comp) = (kind.to_owned(), comp.clone());
        composite_node_ui(p, ui, &id, &kind, &comp, editable);
        inputs_ui(p, ui, &id, &shape);
        return;
    }
    let Some(spec) = shape.spec() else { return };
    if spec.is_output() {
        output_ui(p, ui, &id, spec);
        return;
    }
    let Some(params) = decoded(p, &id) else { return };
    let expose_on = p.scope_kind().filter(|k| p.graph.components.contains_key(base_kind(k))).is_some() && !read_only;
    for section in [Section::Main, Section::Shape, Section::Detail, Section::Advanced] {
        let list: Vec<(usize, &'static ParamSpec)> = spec.params.iter().enumerate().filter(|(_, q)| q.section == section).collect();
        if list.is_empty() {
            continue;
        }
        ui.add_space(12.0);
        let label = section_label(section);
        if section == Section::Advanced {
            let r = ui.add(egui::Button::new(egui::RichText::new(format!("{}  {}", theme::glyph(if p.show_advanced { Icon::ChevronDown } else { Icon::ChevronRight }), label.to_uppercase())).size(10.5).color(pal.weak)).frame(false));
            if r.clicked() {
                p.show_advanced = !p.show_advanced;
            }
            if !p.show_advanced {
                continue;
            }
            ui.add_space(4.0);
        } else if !label.is_empty() {
            ui.label(egui::RichText::new(label.to_uppercase()).size(10.5).color(pal.weak));
            ui.add_space(4.0);
        }
        let mut blocks: Vec<(usize, &'static ParamSpec)> = Vec::new();
        ui.add_enabled_ui(!read_only, |ui| {
            widgets::well(ui, |ui| {
                for (k, q) in &list {
                    let top = ui.cursor().top();
                    param_row(p, ui, &id, q, &params.values()[*k], expose_on);
                    hover_help(p, ui, top, q.help);
                    if matches!(q.kind, ParamKind::Profile { .. } | ParamKind::Gradient { .. } | ParamKind::Points { .. } | ParamKind::Curve { .. }) {
                        blocks.push((*k, q));
                    }
                }
            });
            // Editors too tall for a row, under the well.
            for (k, q) in blocks {
                ui.add_space(6.0);
                let top = ui.cursor().top();
                block_editor(p, ui, &id, q, &params.values()[k]);
                hover_help(p, ui, top, q.help);
            }
        });
    }
    inputs_ui(p, ui, &id, &shape);
}

/// A composite node's settings: the exposed ones, grouped by their
/// categories (the advanced ones folded), as a node's own; and the way in.
fn composite_node_ui(p: &mut TerrainPanel, ui: &mut Ui, id: &str, kind: &str, comp: &Component, editable: bool) {
    let pal = Palette::of(ui);
    let read_only = p.read_only();
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if widgets::button(ui, format!("{}  {}", theme::glyph(Icon::Boxes), t!("Open"))).on_hover_text(t!("Edit what it holds (double-click the node)")).clicked() {
            p.open(id);
        }
        if !editable && !read_only && widgets::button(ui, t!("Edit a copy")).on_hover_text(t!("Make this composite the document's own, under a new name, to change it")).clicked() {
            match p.edit_copy(id) {
                Ok(_) => p.say(t!("A copy of the composite is the document's own now."), 0.0),
                Err(e) => p.say(e, 0.0),
            }
        }
        if !read_only && widgets::button(ui, t!("Ungroup")).on_hover_text(t!("Put its nodes back in its place (⌘⇧G)")).clicked() {
            let _ = p.ungroup(id);
        }
    });
    if !p.graph.nodes.contains_key(id) {
        return;
    }
    ui.add_space(4.0);
    let origin = if editable { t!("A composite of this document") } else { t!("A composite of Massif") };
    ui.label(egui::RichText::new(format!("{origin}  ·  {kind}")).size(11.0).color(pal.weak));
    if comp.exposed.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(t!("It exposes no setting: open it to change what it holds, or expose some of its settings.")).size(11.5).color(pal.weak));
        return;
    }
    // Groups in their first appearance's order, advanced ones last.
    let mut groups: Vec<(String, bool)> = Vec::new();
    for e in &comp.exposed {
        let g = (e.category.clone(), e.advanced);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    groups.sort_by_key(|g| g.1);
    let doc = p.graph.clone();
    let mut shown_adv_header = false;
    for (cat, adv) in groups {
        if adv && !shown_adv_header {
            shown_adv_header = true;
            ui.add_space(12.0);
            let r = ui.add(egui::Button::new(egui::RichText::new(format!("{}  {}", theme::glyph(if p.show_advanced { Icon::ChevronDown } else { Icon::ChevronRight }), t!("Advanced").to_uppercase())).size(10.5).color(pal.weak)).frame(false));
            if r.clicked() {
                p.show_advanced = !p.show_advanced;
            }
        }
        if adv && !p.show_advanced {
            continue;
        }
        ui.add_space(if adv { 4.0 } else { 12.0 });
        if !cat.is_empty() {
            ui.label(egui::RichText::new(super::text::name_owned(&cat).to_uppercase()).size(10.5).color(pal.weak));
            ui.add_space(4.0);
        }
        ui.add_enabled_ui(!read_only, |ui| {
            widgets::well(ui, |ui| {
                for e in comp.exposed.iter().filter(|e| e.category == cat && e.advanced == adv) {
                    let Some((base_spec, base_q)) = super::shape::exposed_base(&doc, comp, e) else {
                        widgets::row(ui, &e.label, |ui| {
                            ui.label(egui::RichText::new(t!("stands for nothing inside")).size(11.5).color(pal.danger));
                        });
                        continue;
                    };
                    let q = super::shape::exposed_spec(base_q, e);
                    let v = exposed_value(&doc, comp, e, &p.graph.nodes[id].params, base_spec, base_q);
                    let top = ui.cursor().top();
                    param_row(p, ui, id, q, &v, false);
                    hover_help(p, ui, top, if e.help.is_empty() { base_q.help } else { leak_help(&e.help) });
                }
            });
        });
    }
}

/// The value an exposed setting has on an instance: its own, or the
/// composite's default, or what the setting it stands for has inside.
fn exposed_value(doc: &massif_terrain::model::Graph, comp: &Component, e: &Exposed, given: &std::collections::BTreeMap<String, serde_json::Value>, base_spec: &'static NodeSpec, base_q: &'static ParamSpec) -> Value {
    let mut inner = std::collections::BTreeMap::new();
    if let Some((n, _)) = e.targets.first().and_then(|t| t.split_once('.'))
        && let Some(node) = comp.nodes.get(n)
        && node.kind == base_spec.type_name()
    {
        inner = node.params.clone();
    }
    let _ = doc;
    if let Some(v) = given.get(&e.name).or(e.default.as_ref()) {
        inner.insert(base_q.name.to_owned(), v.clone());
    }
    let mut diags = Vec::new();
    let params = Params::decode(base_spec, "n", &inner, "", &mut diags);
    let k = base_spec.params.iter().position(|x| std::ptr::eq(x, base_q)).unwrap_or(0);
    params.values()[k].clone()
}

/// The composite open, nothing selected: its name, its inputs and outputs,
/// and the settings it exposes.
fn composite_ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let Some(kind) = p.scope_kind().map(str::to_owned) else { return };
    let Some(comp) = massif_terrain::lower::component(&p.graph, &kind).map(|c| c.into_owned()) else { return };
    let base = base_kind(&kind).to_owned();
    let read_only = p.read_only();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(theme::glyph(Icon::Boxes).to_string()).size(13.0).color(pal.weak));
        ui.label(egui::RichText::new(super::text::name_owned(&comp.label)).size(14.0).strong());
    });
    ui.add_space(2.0);
    let what = if read_only { t!("One of Massif's composites: read only. Edit a copy to change it.") } else { t!("What this composite holds. Its inputs and outputs are the columns at the canvas's edges; its exposed settings show on every node of it.") };
    ui.add(egui::Label::new(egui::RichText::new(what).size(11.5).color(pal.weak)).wrap());
    ui.add_space(10.0);
    // Its words: changed in place (one step each) until the graph has steps
    // for a composite's interface.
    let mut label = comp.label.clone();
    let mut summary = comp.summary.clone();
    let mut category = comp.category.clone();
    let mut changed = false;
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            widgets::row(ui, t!("Name"), |ui| {
                changed |= ui.add(egui::TextEdit::singleline(&mut label).desired_width(160.0)).changed();
            });
            widgets::row(ui, t!("Library group"), |ui| {
                widgets::combo("terrain composite category").width((ui.available_width() * 0.58).min(160.0)).selected_text(super::text::name_owned(&category)).show_ui(ui, |ui| {
                    for f in massif_terrain::registry::Family::ALL {
                        if f == massif_terrain::registry::Family::Internal {
                            continue;
                        }
                        if ui.selectable_label(category == f.label(), super::text::name(f.label())).clicked() {
                            category = f.label().to_owned();
                            changed = true;
                        }
                    }
                });
            });
            widgets::row(ui, t!("Summary"), |ui| {
                changed |= ui.add(egui::TextEdit::singleline(&mut summary).desired_width(160.0)).changed();
            });
        });
    });
    if changed
        && let Some(c) = p.edit().components.get_mut(&base)
    {
        c.label = label;
        c.summary = summary;
        c.category = category;
    }
    // Inputs.
    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("Inputs").to_uppercase()).size(10.5).color(pal.weak));
    ui.add_space(4.0);
    let mut remove_in = None;
    let mut add_in = None;
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            for d in &comp.inputs {
                let kind = PortKind::from_id(&d.kind).unwrap_or(PortKind::Height);
                widgets::row(ui, if d.label.is_empty() { &d.name } else { &d.label }, |ui| {
                    if icon_button(ui, Icon::X, t!("Remove the input")) {
                        remove_in = Some(d.name.clone());
                    }
                    ui.label(egui::RichText::new(sockets::kind_label(kind)).size(11.5).color(pal.weak));
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    sockets::draw(ui.painter(), r.center(), 4.0, kind, sockets::shape(kind), true, &pal);
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.menu_button(format!("{}  {}", theme::glyph(Icon::Plus), t!("Add an input")), |ui| {
                    for k in PortKind::ALL {
                        if ui.button(sockets::kind_label(k)).clicked() {
                            add_in = Some(k);
                            ui.close();
                        }
                    }
                });
            });
            ui.add_space(4.0);
        });
    });
    if let Some(name) = remove_in {
        if let Err(e) = p.doc_op(Op::Unport { component: base.clone(), name }) {
            p.say(e, 0.0);
        }
        p.end_edit();
    }
    if let Some(k) = add_in {
        let taken: Vec<String> = comp.inputs.iter().map(|d| d.name.clone()).collect();
        let mut name = k.id().to_owned();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{}_{n}", k.id());
            n += 1;
        }
        let decl = PortDecl { label: super::text::name_owned(&capital(&name)), name, kind: k.id().to_owned(), optional: true, help: String::new() };
        if let Err(e) = p.doc_op(Op::Port { component: base.clone(), decl }) {
            p.say(e, 0.0);
        }
        p.end_edit();
    }
    // Outputs.
    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("Outputs").to_uppercase()).size(10.5).color(pal.weak));
    ui.add_space(4.0);
    let mut remove_out = None;
    let mut add_out = false;
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            for o in &comp.outputs {
                widgets::row(ui, if o.label.is_empty() { &o.name } else { &o.label }, |ui| {
                    if comp.outputs.len() > 1 && icon_button(ui, Icon::X, t!("Remove the output")) {
                        remove_out = Some(o.name.clone());
                    }
                    let from = if o.from.is_empty() { t!("nothing yet").to_owned() } else { o.from.clone() };
                    weak_text(ui, &from, if o.from.is_empty() { pal.danger } else { pal.weak });
                });
            }
            ui.add_space(4.0);
            if ui.button(format!("{}  {}", theme::glyph(Icon::Plus), t!("Add an output"))).clicked() {
                add_out = true;
            }
            ui.add_space(4.0);
        });
    });
    if let Some(name) = remove_out {
        if let Err(e) = p.doc_op(Op::Unoutput { component: base.clone(), name }) {
            p.say(e, 0.0);
        }
        p.end_edit();
    }
    if add_out {
        let taken: Vec<String> = comp.outputs.iter().map(|o| o.name.clone()).collect();
        let mut name = "out".to_owned();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("out_{n}");
            n += 1;
        }
        if let Some(c) = p.edit().components.get_mut(&base) {
            c.outputs.push(OutputDecl { label: capital(&name), name, from: String::new() });
        }
        p.end_edit();
    }
    // Exposed settings.
    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("Exposed settings").to_uppercase()).size(10.5).color(pal.weak));
    ui.add_space(4.0);
    if comp.exposed.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(t!("None yet: select a node inside and click ⤴ beside one of its settings.")).size(11.5).color(pal.weak)).wrap());
        return;
    }
    let mut unexpose = None;
    let mut adv_toggle = None;
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            for e in &comp.exposed {
                let top = ui.cursor().top();
                widgets::row(ui, if e.label.is_empty() { &e.name } else { &e.label }, |ui| {
                    if icon_button(ui, Icon::X, t!("Take it off the composite")) {
                        unexpose = Some(e.name.clone());
                    }
                    let mut adv = e.advanced;
                    if widgets::checkbox(ui, &mut adv, t!("Advanced")).changed() {
                        adv_toggle = Some((e.clone(), adv));
                    }
                });
                // What it stands for inside, under the pointer.
                let r = Rect::from_min_max(pos2(ui.max_rect().left(), top), pos2(ui.max_rect().right(), ui.cursor().top()));
                if ui.rect_contains_pointer(r) {
                    p.hovered_help = Some(leak_help(&t!("Stands for {targets} inside.", targets = e.targets.join(", "))));
                }
            }
        });
    });
    if let Some(name) = unexpose
        && let Err(e) = p.doc_op(Op::Unexpose { component: base.clone(), name })
    {
        p.say(e, 0.0);
    }
    if let Some((mut e, adv)) = adv_toggle {
        e.advanced = adv;
        // The same setting again, advanced or not (Expose replaces the one
        // of that name).
        if let Err(err) = p.doc_op(Op::Expose { component: base.clone(), exposed: e }) {
            p.say(err, 0.0);
        }
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect::<String>().replace(['_', '-'], " ")).unwrap_or_default()
}

/// A frame's settings: its title, colour and note, what it holds.
fn frame_ui(p: &mut TerrainPanel, ui: &mut Ui, id: &str) {
    let pal = Palette::of(ui);
    let Some(f) = p.graph.frames.get(id).cloned() else { return };
    let read_only = p.read_only();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(theme::glyph(Icon::Frame).to_string()).size(13.0).color(pal.weak));
        ui.add(egui::Label::new(egui::RichText::new(&f.title).size(14.0).strong()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !read_only && icon_button(ui, Icon::Trash2, t!("Remove the frame (its nodes stay)")) {
                p.unframe(id);
            }
        });
    });
    if !p.graph.frames.contains_key(id) {
        return;
    }
    let n = p.frame_nodes(id).len();
    let inner = p.frame_tree(id).len() - 1;
    let what = if inner > 0 { t!("{n} nodes, {k} frames inside", n = n, k = inner) } else { t!("{n} nodes", n = n) };
    ui.label(egui::RichText::new(what).size(11.5).color(pal.weak));
    ui.add_space(10.0);
    let mut title = f.title.clone();
    let mut note = f.description.clone();
    let mut colour: Option<Option<String>> = None;
    let mut folded = f.collapsed;
    ui.add_enabled_ui(!read_only, |ui| {
        widgets::well(ui, |ui| {
            widgets::row(ui, t!("Title"), |ui| {
                if ui.add(egui::TextEdit::singleline(&mut title).desired_width(160.0)).changed() {
                    p.set_frame(id, Some(title.clone()), None, None);
                    p.keep_editing();
                }
            });
            widgets::row(ui, t!("Colour"), |ui| {
                for (hex, name) in super::canvas::FRAME_COLOURS.iter().rev() {
                    let c = massif_terrain::params::parse_color(hex).unwrap_or([0.5; 3]);
                    let swatch = Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
                    let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    let on = f.color.as_deref() == Some(*hex);
                    ui.painter().circle(r.center(), if on { 6.5 } else { 5.5 }, swatch, Stroke::new(if on { 2.0 } else { 0.0 }, pal.text));
                    if resp.on_hover_text(super::text::name(name)).clicked() {
                        colour = Some(Some((*hex).to_owned()));
                    }
                }
                let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                ui.painter().circle(r.center(), 5.5, pal.weak.gamma_multiply(0.4), Stroke::new(if f.color.is_none() { 2.0 } else { 1.0 }, pal.weak));
                if resp.on_hover_text(t!("Grey")).clicked() {
                    colour = Some(None);
                }
            });
            widgets::row(ui, t!("Folded"), |ui| {
                if widgets::switch(ui, &mut folded).changed() {
                    p.toggle_frame(id);
                }
            });
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(t!("Note").to_uppercase()).size(10.5).color(pal.weak));
        ui.add_space(4.0);
        if ui.add(egui::TextEdit::multiline(&mut note).desired_width(ui.available_width()).desired_rows(3).hint_text(t!("What these nodes do, for whoever reads the graph"))).changed() {
            p.set_frame(id, None, None, Some(note.clone()));
            p.keep_editing();
        }
    });
    if let Some(c) = colour {
        p.set_frame(id, None, Some(c), None);
        p.end_edit();
    }
}

/// The node's inputs: what feeds each, or what an empty one stands for.
fn inputs_ui(p: &mut TerrainPanel, ui: &mut Ui, id: &str, shape: &NodeShape) {
    let pal = Palette::of(ui);
    let mut names: Vec<(String, String, PortKind, String, String)> = shape.inputs.iter().map(|i| (i.name.clone(), i.label.clone(), i.kind, i.default.clone(), i.help.clone())).collect();
    if let Some(spec) = shape.spec() {
        names.extend(spec.params.iter().filter(|q| q.drivable && p.graph.nodes[id].inputs.contains_key(&format!("@{}", q.name))).map(|q| (format!("@{}", q.name), super::text::name(q.label).to_string(), PortKind::Mask, String::new(), q.help.to_owned())));
    }
    if names.is_empty() {
        return;
    }
    ui.add_space(12.0);
    ui.label(egui::RichText::new(t!("Inputs").to_uppercase()).size(10.5).color(pal.weak));
    ui.add_space(4.0);
    let mut cut = None;
    let read_only = p.read_only();
    widgets::well(ui, |ui| {
        for (name, label, kind, default, help) in &names {
            let top = ui.cursor().top();
            let src = p.graph.nodes[id].inputs.get(name).cloned();
            widgets::row(ui, label, |ui| {
                match &src {
                    Some(s) => {
                        if !read_only && ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::X).to_string()).size(11.0)).frame(false)).on_hover_text(t!("Disconnect")).clicked() {
                            cut = Some(name.clone());
                        }
                        let from = massif_terrain::model::Source::parse(s);
                        ui.label(egui::RichText::new(p.display_name(from.node)).size(12.0));
                    }
                    None => {
                        let text = if default.is_empty() { t!("required").to_owned() } else { super::text::name_owned(default) };
                        weak_text(ui, &text, if default.is_empty() { pal.danger } else { pal.weak });
                    }
                }
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                sockets::draw(ui.painter(), r.center(), 4.0, *kind, if name.starts_with('@') { sockets::Shape_::Diamond } else { sockets::shape(*kind) }, src.is_some(), &pal);
            });
            if !help.is_empty() {
                hover_help(p, ui, top, leak_help(help));
            }
        }
    });
    if let Some(name) = cut {
        p.disconnect(id, &name);
    }
}

/// The Map Output's settings: whether it is the one the map takes, and
/// each input with what feeds it or the default it stands for.
fn output_ui(p: &mut TerrainPanel, ui: &mut Ui, id: &str, spec: &'static NodeSpec) {
    let pal = Palette::of(ui);
    ui.add_space(12.0);
    let mut active = p.active_output().as_deref() == Some(id);
    widgets::well(ui, |ui| {
        let top = ui.cursor().top();
        widgets::row(ui, t!("Active"), |ui| {
            if widgets::switch(ui, &mut active).changed() && active {
                p.set_active_output(id);
            }
        });
        hover_help(p, ui, top, "Whether the map takes this output, when the graph has several.");
        // Its other settings (the blocks colours turn into…), as any node's.
        if let Some(params) = decoded(p, id) {
            for (k, q) in spec.params.iter().enumerate().filter(|(_, q)| q.name != "active") {
                let top = ui.cursor().top();
                param_row(p, ui, id, q, &params.values()[k], false);
                hover_help(p, ui, top, q.help);
            }
        }
    });
    let mut cut = None;
    for (key, label) in super::canvas::OUTPUT_GROUPS {
        let mut names: Vec<String> = spec.inputs.iter().filter(|i| super::canvas::output_group(i.name) == *key).map(|i| i.name.to_owned()).collect();
        if *key == "masks" {
            names.extend(p.graph.nodes[id].inputs.keys().filter(|k| k.starts_with("mask.") && spec.input(k).is_none()).cloned());
        }
        if names.is_empty() {
            continue;
        }
        ui.add_space(12.0);
        ui.label(egui::RichText::new(super::text::name(label).to_uppercase()).size(10.5).color(pal.weak));
        ui.add_space(4.0);
        widgets::well(ui, |ui| {
            for name in &names {
                let (label, kind, default, help) = match spec.inputs.iter().find(|i| i.name == name) {
                    Some(i) => (super::text::name(i.label).to_owned(), i.kind, i.default, i.help),
                    None => (name.strip_prefix("mask.").unwrap_or(name).replace(['_', '-'], " "), PortKind::Mask, "none", spec.open_inputs.map_or("", |o| o.help)),
                };
                let top = ui.cursor().top();
                let src = p.graph.nodes[id].inputs.get(name).cloned();
                widgets::row(ui, &label, |ui| {
                    match &src {
                        Some(s) => {
                            if ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::X).to_string()).size(11.0)).frame(false)).on_hover_text(t!("Disconnect")).clicked() {
                                cut = Some(name.clone());
                            }
                            let from = massif_terrain::model::Source::parse(s);
                            ui.label(egui::RichText::new(p.display_name(from.node)).size(12.0));
                        }
                        None => {
                            weak_text(ui, super::text::name(default), pal.weak);
                        }
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    sockets::draw(ui.painter(), r.center(), 4.0, kind, sockets::shape(kind), src.is_some(), &pal);
                });
                hover_help(p, ui, top, help);
            }
        });
    }
    // A named mask of one's own.
    ui.add_space(8.0);
    let mut new_name: String = ui.data(|d| d.get_temp(ui.scope_id().with("new mask"))).unwrap_or_default();
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut new_name).hint_text(t!("moss, sand, path…")).desired_width(140.0));
        let valid = massif_terrain::model::valid_id(new_name.trim());
        if (ui.add_enabled(valid, egui::Button::new(format!("{} {}", theme::glyph(Icon::Plus), t!("Mask")))).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid)) && valid {
            let name = format!("mask.{}", new_name.trim());
            let key = p.lkey(&format!("{id}.{name}"));
            p.edit_layout().exposed.insert(key);
            p.end_edit();
            new_name.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(ui.scope_id().with("new mask"), new_name));
    if let Some(name) = cut {
        p.disconnect(id, &name);
    }
}

/// Writes a setting, as one of the graph's steps.
fn set(p: &mut TerrainPanel, id: &str, q: &ParamSpec, v: &Value) {
    let value = encode(q, v);
    if let Err(e) = p.op(Op::Set { node: id.to_owned(), param: q.name.to_owned(), value }) {
        p.say(e, 0.0);
    }
}

/// The value a setting takes when left out.
fn default_of(q: &ParamSpec) -> Option<Value> {
    Some(match q.kind {
        ParamKind::Float { default, .. } => Value::Num(default),
        ParamKind::Int { default, .. } => Value::Num(default as f64),
        ParamKind::Bool { default } => Value::Bool(default),
        ParamKind::Choice { options, default } => Value::Choice(options.iter().position(|o| o.value == default).unwrap_or(0)),
        ParamKind::Color { default } => Value::Color(massif_terrain::params::parse_color(default)?),
        _ => return None,
    })
}

fn param_row(p: &mut TerrainPanel, ui: &mut Ui, id: &str, q: &'static ParamSpec, v: &Value, expose_on: bool) {
    let pal = Palette::of(ui);
    let label = super::text::name_owned(q.label);
    let driven = p.graph.nodes[id].inputs.get(&format!("@{}", q.name)).cloned();
    let key = p.lkey(&format!("{id}.{}", q.name));
    let exposed = p.layout.exposed.contains(&key);
    widgets::row(ui, &label, |ui| {
        // ⤴: shown on the composite open, standing for this setting.
        if expose_on && ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::ArrowUpRight).to_string()).size(12.0).color(pal.weak)).frame(false)).on_hover_text(t!("Expose on the composite: its nodes show this setting")).clicked() {
            let mut name: String = q.name.to_owned();
            if let Some(c) = p.scope_kind().and_then(|k| p.graph.components.get(base_kind(k))) {
                let mut k = 2;
                while c.exposed.iter().any(|e| e.name == name) {
                    name = format!("{}_{k}", q.name);
                    k += 1;
                }
            }
            let range = match q.kind {
                ParamKind::Float { soft, .. } => Some(soft),
                _ => None,
            };
            p.exposing = Some(Exposing { target: format!("{id}.{}", q.name), name, label: label.clone(), help: String::new(), category: section_label(q.section).to_owned(), advanced: q.section == Section::Advanced, range, error: None });
        }
        // ◇: driven by a field over the map.
        if q.drivable {
            let on = driven.is_some() || exposed;
            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            let c = r.center();
            let s = 5.0;
            let pts = vec![pos2(c.x, c.y - s), pos2(c.x + s, c.y), pos2(c.x, c.y + s), pos2(c.x - s, c.y)];
            let ink = if on { pal.accent } else if resp.hovered() { pal.text } else { pal.weak };
            ui.painter().add(egui::Shape::convex_polygon(pts, if driven.is_some() { ink } else { Color32::TRANSPARENT }, Stroke::new(1.3, ink)));
            let tip = if driven.is_some() { t!("Driven by a field: click to stop") } else if exposed { t!("Shown as a socket on the node: connect a mask to drive it, or click to hide") } else { t!("Drive this setting by a field over the map: adds a socket on the node") };
            if resp.on_hover_text(tip).clicked() {
                if driven.is_some() {
                    p.disconnect(id, &format!("@{}", q.name));
                    p.edit_layout().exposed.remove(&key);
                } else if exposed {
                    p.edit_layout().exposed.remove(&key);
                } else {
                    p.edit_layout().exposed.insert(key.clone());
                }
                p.end_edit();
            }
        } else {
            // Its place kept, so that every row's slider starts at the same
            // place.
            ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        }
        if let Some(src) = &driven {
            ui.label(egui::RichText::new(t!("from {node}", node = p.display_name(massif_terrain::model::Source::parse(src).node))).size(12.0).color(pal.accent));
            return;
        }
        // The reset dot, when changed from the default.
        let def = default_of(q);
        let given = p.graph.nodes[id].params.contains_key(q.name);
        if given && def.as_ref().is_some_and(|d| d != v) {
            let (r, resp) = ui.allocate_exact_size(vec2(12.0, 18.0), Sense::click());
            ui.painter().circle_filled(r.center(), 2.6, if resp.hovered() { pal.text } else { pal.accent });
            if resp.on_hover_text(t!("Back to the default")).clicked() {
                let _ = p.op(Op::Set { node: id.to_owned(), param: q.name.to_owned(), value: serde_json::Value::Null });
                return;
            }
        } else {
            ui.allocate_exact_size(vec2(12.0, 18.0), Sense::hover());
        }
        // As wide on every row of the well: the room the name column leaves.
        let width = (ui.available_width() - 70.0).clamp(90.0, 170.0);
        match (q.kind, v) {
            (ParamKind::Float { min, max, soft, unit, .. }, Value::Num(x)) => {
                let mut x = *x;
                let decimals = if unit == Unit::Blocks || unit == Unit::Degrees || soft[1] - soft[0] >= 200.0 { 0 } else if soft[1] - soft[0] >= 10.0 { 1 } else { 2 };
                let r = ui.add(widgets::Field::new(&mut x, soft[0]..=soft[1]).suffix(unit.suffix()).max_decimals(decimals).clamping(egui::SliderClamping::Never).width(width));
                if r.changed() {
                    set(p, id, q, &Value::Num(x.clamp(min, max)));
                }
            }
            (ParamKind::Int { min, max, .. }, Value::Num(x)) => {
                let mut x = *x;
                let hi = (max as f64).min(min as f64 + 64.0).max(min as f64 + 1.0);
                let r = ui.add(widgets::Field::new(&mut x, min as f64..=hi).max_decimals(0).clamping(egui::SliderClamping::Never).width(width));
                if r.changed() {
                    set(p, id, q, &Value::Num(x.round().clamp(min as f64, max as f64)));
                }
            }
            (ParamKind::Bool { .. }, Value::Bool(b)) => {
                let mut b = *b;
                if widgets::switch(ui, &mut b).changed() {
                    set(p, id, q, &Value::Bool(b));
                }
            }
            (ParamKind::Choice { options, .. }, Value::Choice(i)) => {
                let labels: Vec<&str> = options.iter().map(|o| super::text::name(o.label)).collect();
                let short = labels.len() <= 3 && labels.iter().map(|l| l.chars().count()).sum::<usize>() <= 22;
                if short {
                    let on: Vec<bool> = (0..labels.len()).map(|k| k == *i).collect();
                    if let Some(k) = widgets::button_group(ui, &format!("{id}.{}", q.name), &labels, &on) {
                        set(p, id, q, &Value::Choice(k));
                    }
                } else {
                    let mut k = *i;
                    widgets::combo(format!("terrain {id}.{}", q.name)).width(width).selected_text(labels.get(*i).copied().unwrap_or("")).show_ui(ui, |ui| {
                        for (j, l) in labels.iter().enumerate() {
                            let r = ui.selectable_value(&mut k, j, *l);
                            if !options[j].help.is_empty() {
                                r.on_hover_text(super::text::name(options[j].help));
                            }
                        }
                    });
                    if k != *i {
                        set(p, id, q, &Value::Choice(k));
                    }
                }
            }
            (ParamKind::Color { .. }, Value::Color(c)) => {
                let mut rgb = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                    set(p, id, q, &Value::Color(rgb.map(|b| b as f32 / 255.0)));
                }
                ui.label(egui::RichText::new(color_hex(*c)).size(11.5).color(pal.weak).monospace());
            }
            (ParamKind::Seed, Value::Seed(s)) => {
                if ui.add(egui::Button::new(theme::glyph(Icon::Dices).to_string()).frame(false)).on_hover_text(t!("Another seed")).clicked() {
                    let next = s.wrapping_mul(2_654_435_761).wrapping_add(97) % 100_000;
                    set(p, id, q, &Value::Seed(next));
                }
                let mut x = *s as f64;
                if ui.add(widgets::Field::new(&mut x, 0.0..=99_999.0).max_decimals(0).clamping(egui::SliderClamping::Never).width(width - 30.0)).changed() {
                    set(p, id, q, &Value::Seed(x.max(0.0) as u32));
                }
            }
            (ParamKind::Text { .. }, Value::Text(s)) => {
                let mut s = s.clone();
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(width)).changed() {
                    set(p, id, q, &Value::Text(s));
                }
            }
            (ParamKind::File { extensions }, Value::Text(s)) => {
                if widgets::button(ui, t!("Choose…")).clicked() {
                    let mut d = rfd::FileDialog::new();
                    if !extensions.is_empty() {
                        d = d.add_filter(t!("Files"), extensions);
                    }
                    if let Some(path) = d.pick_file() {
                        set(p, id, q, &Value::Text(path.to_string_lossy().into_owned()));
                    }
                }
                let shown = if s.is_empty() { t!("none").to_owned() } else { std::path::Path::new(s).file_name().map_or(s.clone(), |f| f.to_string_lossy().into_owned()) };
                // An EXR's own lowest and highest values (read as 0 and 1):
                // on a Heightmap, a click makes them its Low and High.
                let file = std::path::Path::new(s);
                let range = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("exr")).then(|| massif_core::heightmap::cached(file).ok()).flatten().and_then(|m| m.range);
                if let Some([lo, hi]) = range {
                    let node: Option<&'static NodeSpec> = p.graph.nodes.get(id).and_then(|n| massif_terrain::registry::spec(&n.kind)).filter(|sp| sp.id == "input.heightmap");
                    let spec = |name: &str| node.and_then(|sp| sp.params.iter().find(|q| q.name == name));
                    let text = format!("{lo:.1} – {hi:.1}");
                    if let (Some(low), Some(high)) = (spec("low"), spec("high")) {
                        if ui.add(egui::Button::new(egui::RichText::new(text).size(12.0).color(pal.accent)).frame(false)).on_hover_text(t!("The file's lowest and highest values: click to make them Low and High")).clicked() {
                            set(p, id, low, &Value::Num(lo as f64));
                            set(p, id, high, &Value::Num(hi as f64));
                        }
                    } else {
                        ui.label(egui::RichText::new(text).size(12.0).color(pal.weak));
                    }
                }
                ui.add(egui::Label::new(egui::RichText::new(shown).size(12.0).color(pal.weak)).truncate());
            }
            (ParamKind::Points { .. } | ParamKind::Curve { .. }, Value::Points(pts)) => {
                let text = trn_points(pts.len());
                ui.label(egui::RichText::new(format!("{text} · {}", t!("edit on the viewer"))).size(12.0).color(pal.accent));
            }
            (ParamKind::Profile { .. }, _) => {
                ui.label(egui::RichText::new(t!("drag the points")).size(11.5).color(pal.weak));
            }
            (ParamKind::Gradient { .. }, _) => {
                ui.label(egui::RichText::new(t!("click a stop to colour it")).size(11.5).color(pal.weak));
            }
            _ => {}
        }
    });
}

/// The dialog that exposes a setting of the composite open: its name, its
/// words, its group, its range.
pub fn expose_dialog(p: &mut TerrainPanel, ui: &mut Ui) {
    let Some(mut x) = p.exposing.take() else { return };
    let ctx = ui.ctx().clone();
    let pal = Palette::of(ui);
    let mut done = None;
    let response = crate::app::modal(&ctx, "terrain expose").show(&ctx, |ui| {
        ui.set_width(420.0);
        ui.label(egui::RichText::new(t!("Expose a setting")).size(15.0).strong());
        ui.add_space(2.0);
        ui.label(egui::RichText::new(t!("Every node of this composite shows it, standing for {target} inside.", target = x.target)).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        widgets::well(ui, |ui| {
            widgets::row(ui, t!("Label"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut x.label).desired_width(200.0));
            });
            widgets::row(ui, t!("Name"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut x.name).desired_width(200.0).font(egui::TextStyle::Monospace));
            });
            widgets::row(ui, t!("Group"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut x.category).desired_width(200.0).hint_text(t!("Shape, Erosion…")));
            });
            widgets::row(ui, t!("Advanced"), |ui| {
                widgets::switch(ui, &mut x.advanced);
            });
            if let Some(r) = &mut x.range {
                widgets::row(ui, t!("Range"), |ui| {
                    ui.add(widgets::Field::new(&mut r[1], -1e6..=1e6).text(t!("to")).width(96.0));
                    ui.add(widgets::Field::new(&mut r[0], -1e6..=1e6).text(t!("from")).width(96.0));
                });
            }
        });
        ui.add_space(8.0);
        ui.add(egui::TextEdit::multiline(&mut x.help).desired_width(ui.available_width()).desired_rows(2).hint_text(t!("What it changes, for whoever uses the composite")));
        if let Some(e) = &x.error {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(e).size(11.5).color(pal.danger));
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(theme::primary_button(ui, t!("Expose"))).clicked() {
                done = Some(true);
            }
            if widgets::button(ui, t!("Cancel")).clicked() {
                done = Some(false);
            }
        });
    });
    if response.should_close() && done.is_none() {
        done = Some(false);
    }
    match done {
        Some(true) => {
            let Some(kind) = p.scope_kind().map(|k| base_kind(k).to_owned()) else { return };
            let exposed = Exposed {
                name: x.name.trim().to_owned(),
                label: x.label.trim().to_owned(),
                help: x.help.trim().to_owned(),
                targets: vec![x.target.clone()],
                default: None,
                min: None,
                max: None,
                soft: x.range.map(|r| [r[0].min(r[1]), r[0].max(r[1])]),
                unit: None,
                control: None,
                category: x.category.trim().to_owned(),
                advanced: x.advanced,
            };
            if let Err(e) = p.doc_op(Op::Expose { component: kind, exposed }) {
                x.error = Some(e);
                p.exposing = Some(x);
            } else {
                p.say(t!("Exposed: every node of this composite shows it."), 0.0);
            }
        }
        Some(false) => {}
        None => p.exposing = Some(x),
    }
}

fn trn_points(n: usize) -> String {
    if n == 1 { t!("1 point").to_owned() } else { t!("{n} points", n = n) }
}

/// The editors of profiles, gradients and lists of points, as wide as the
/// panel.
fn block_editor(p: &mut TerrainPanel, ui: &mut Ui, id: &str, q: &'static ParamSpec, v: &Value) {
    match (q.kind, v) {
        (ParamKind::Profile { .. }, Value::Profile(pts)) => {
            let mut pts = pts.clone();
            if profile_editor(ui, &format!("{id}.{}", q.name), &mut pts) {
                set(p, id, q, &Value::Profile(pts));
            }
        }
        (ParamKind::Gradient { .. }, Value::Gradient(stops)) => {
            let mut stops = stops.clone();
            if gradient_editor(ui, &format!("{id}.{}", q.name), &mut stops) {
                set(p, id, q, &Value::Gradient(stops));
            }
        }
        (ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. }, Value::Points(pts)) => {
            let mut pts = pts.clone();
            if points_table(ui, &format!("{id}.{}", q.name), fields, &mut pts, max_of(q), p.space.size as f64) {
                set(p, id, q, &Value::Points(pts));
            }
        }
        _ => {}
    }
}

fn max_of(q: &ParamSpec) -> usize {
    match q.kind {
        ParamKind::Points { max, .. } => max,
        ParamKind::Curve { .. } => 64,
        _ => 0,
    }
}

/// A transfer curve from 0–1 to 0–1: drag its points, double-click to add
/// one, right-click to remove one (two at least stay).
pub fn profile_editor(ui: &mut Ui, id: &str, pts: &mut Vec<[f64; 2]>) -> bool {
    let pal = Palette::of(ui);
    let w = ui.available_width();
    let (rect, bg) = ui.allocate_exact_size(vec2(w, 120.0), Sense::click());
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect(rect, theme::GROUP_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let inner = rect.shrink(10.0);
    let at = |x: f64, y: f64| pos2(inner.left() + x as f32 * inner.width(), inner.bottom() - y as f32 * inner.height());
    for k in 1..4 {
        let f = k as f32 / 4.0;
        painter.hline(inner.x_range(), inner.bottom() - f * inner.height(), Stroke::new(1.0, pal.border.gamma_multiply(0.6)));
        painter.vline(inner.left() + f * inner.width(), inner.y_range(), Stroke::new(1.0, pal.border.gamma_multiply(0.6)));
    }
    let line: Vec<egui::Pos2> = (0..=96).map(|k| {
        let x = k as f64 / 96.0;
        at(x, eval_profile(pts, x).clamp(0.0, 1.0))
    }).collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, pal.accent)));
    let mut changed = false;
    let mut remove = None;
    let n = pts.len();
    for k in 0..n {
        let c = at(pts[k][0], pts[k][1]);
        let r = ui.interact(egui::Rect::from_center_size(c, vec2(14.0, 14.0)), ui.scope_id().with((id, "pt", k)), Sense::click_and_drag());
        painter.circle(c, if r.hovered() || r.dragged() { 5.5 } else { 4.5 }, pal.panel, Stroke::new(1.8, pal.accent));
        if r.dragged()
            && let Some(q) = r.interact_pointer_pos()
        {
            let lo = if k == 0 { 0.0 } else { pts[k - 1][0] + 0.01 };
            let hi = if k + 1 == n { 1.0 } else { pts[k + 1][0] - 0.01 };
            let x = (((q.x - inner.left()) / inner.width()) as f64).clamp(lo, hi.max(lo));
            let y = (((inner.bottom() - q.y) / inner.height()) as f64).clamp(0.0, 1.0);
            pts[k] = [x, y];
            changed = true;
        }
        if r.secondary_clicked() && n > 2 {
            remove = Some(k);
        }
    }
    if let Some(k) = remove {
        pts.remove(k);
        changed = true;
    }
    if bg.double_clicked()
        && let Some(q) = bg.interact_pointer_pos()
    {
        let x = (((q.x - inner.left()) / inner.width()) as f64).clamp(0.0, 1.0);
        let y = (((inner.bottom() - q.y) / inner.height()) as f64).clamp(0.0, 1.0);
        let k = pts.iter().position(|p| p[0] > x).unwrap_or(pts.len());
        pts.insert(k, [x, y]);
        changed = true;
    }
    changed
}

/// Colour stops along 0–1: drag a stop, click it to choose its colour,
/// double-click the bar to add one, right-click a stop to remove it.
pub fn gradient_editor(ui: &mut Ui, id: &str, stops: &mut Vec<(f64, [f32; 3])>) -> bool {
    let pal = Palette::of(ui);
    let w = ui.available_width();
    let (rect, bg) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
    let bar = Rect::from_min_size(rect.min, vec2(w, 22.0));
    let painter = ui.painter_at(rect.expand(6.0));
    let steps = (bar.width() as usize / 3).max(8);
    for k in 0..steps {
        let t0 = k as f64 / steps as f64;
        let c = eval_gradient(stops, t0 + 0.5 / steps as f64);
        let r = Rect::from_min_max(pos2(bar.left() + t0 as f32 * bar.width(), bar.top()), pos2(bar.left() + (k + 1) as f32 / steps as f32 * bar.width() + 0.5, bar.bottom()));
        painter.rect_filled(r, 0.0, Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
    }
    painter.rect_stroke(bar, 4.0, Stroke::new(1.0, pal.border), StrokeKind::Outside);
    let mut changed = false;
    let mut remove = None;
    let open_key = ui.scope_id().with((id, "open stop"));
    let mut open: Option<usize> = ui.data(|d| d.get_temp(open_key));
    for k in 0..stops.len() {
        let x = bar.left() + stops[k].0 as f32 * bar.width();
        let c = pos2(x, bar.bottom() + 9.0);
        let r = ui.interact(Rect::from_center_size(c, vec2(14.0, 18.0)), ui.scope_id().with((id, "stop", k)), Sense::click_and_drag());
        let col = stops[k].1.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
        let tri = vec![pos2(x, bar.bottom() + 1.0), pos2(x + 6.0, bar.bottom() + 8.0), pos2(x + 6.0, bar.bottom() + 17.0), pos2(x - 6.0, bar.bottom() + 17.0), pos2(x - 6.0, bar.bottom() + 8.0)];
        painter.add(egui::Shape::convex_polygon(tri, Color32::from_rgb(col[0], col[1], col[2]), Stroke::new(if r.hovered() || open == Some(k) { 2.0 } else { 1.2 }, pal.text)));
        if r.dragged()
            && let Some(q) = r.interact_pointer_pos()
        {
            stops[k].0 = (((q.x - bar.left()) / bar.width()) as f64).clamp(0.0, 1.0);
            changed = true;
        }
        if r.clicked() {
            open = if open == Some(k) { None } else { Some(k) };
        }
        if r.secondary_clicked() && stops.len() > 2 {
            remove = Some(k);
        }
    }
    if let Some(k) = remove {
        stops.remove(k);
        open = None;
        changed = true;
    }
    if bg.double_clicked()
        && let Some(q) = bg.interact_pointer_pos()
    {
        let t = (((q.x - bar.left()) / bar.width()) as f64).clamp(0.0, 1.0);
        let c = eval_gradient(stops, t);
        stops.push((t, c));
        changed = true;
    }
    if let Some(k) = open.filter(|k| *k < stops.len()) {
        let mut rgb = stops[k].1.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t!("Stop at {at} %", at = format!("{:.0}", stops[k].0 * 100.0))).size(11.5));
            if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                stops[k].1 = rgb.map(|b| b as f32 / 255.0);
                changed = true;
            }
        });
    }
    ui.data_mut(|d| match open {
        Some(k) => {
            d.insert_temp(open_key, k);
        }
        None => {
            d.remove::<usize>(open_key);
        }
    });
    if changed {
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let _ = Align2::LEFT_TOP;
    changed
}

/// Points placed on the terrain, as a table: position in blocks and each
/// value, a point added or removed.
fn points_table(ui: &mut Ui, id: &str, fields: &'static [massif_terrain::registry::Field], pts: &mut Vec<Point>, max: usize, size: f64) -> bool {
    let pal = Palette::of(ui);
    let mut changed = false;
    let mut remove = None;
    widgets::well(ui, |ui| {
        for (k, pt) in pts.iter_mut().enumerate() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{}", k + 1)).size(11.0).color(pal.weak));
                let mut x = pt.x * size;
                let mut z = pt.z * size;
                changed |= ui.add(widgets::Field::new(&mut x, 0.0..=size).text("x").max_decimals(0).width(92.0)).changed();
                changed |= ui.add(widgets::Field::new(&mut z, 0.0..=size).text("z").max_decimals(0).width(92.0)).changed();
                pt.x = x / size;
                pt.z = z / size;
                if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false)).on_hover_text(t!("Remove the point")).clicked() {
                    remove = Some(k);
                }
            });
            for (j, f) in fields.iter().enumerate() {
                if pt.values.len() <= j {
                    pt.values.push(f.default);
                }
                let label = super::text::name(f.label);
                widgets::row(ui, label, |ui| {
                    let mut v = pt.values[j];
                    if ui.add(widgets::Field::new(&mut v, f.min..=f.max).suffix(f.unit.suffix()).width(150.0)).changed() {
                        pt.values[j] = v;
                        changed = true;
                    }
                });
            }
            ui.add_space(4.0);
        }
        if pts.len() < max {
            ui.add_space(4.0);
            if ui.button(format!("{} {}", theme::glyph(Icon::Plus), t!("Add a point"))).clicked() {
                let last = pts.last().map_or((0.5, 0.5), |p| (p.x, p.z));
                pts.push(Point { x: (last.0 + 0.08).min(0.95), z: last.1, values: fields.iter().map(|f| f.default).collect() });
                changed = true;
            }
            ui.add_space(6.0);
        }
    });
    if let Some(k) = remove {
        pts.remove(k);
        changed = true;
    }
    let _ = id;
    changed
}
