//! The viewer: the node viewed (or the active Map Output) lit as a render,
//! from above or in 3D, in clay or in its colours, a mask laid over the
//! relief it comes from; points and lines of the selected node edited on
//! it. Coarse first, then sharp.

use super::eval::Shown;
use super::panel::{Event, TerrainPanel};
use super::shade::Surface;
use super::sockets;
use crate::theme::{self, Icon, Palette};
use crate::widgets;
use egui::{Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_terrain::params::Value;
use massif_terrain::registry::{ParamKind, PortKind};

const BAR_H: f32 = 36.0;

/// Where the picture goes in the viewer's card.
pub fn picture_rect(card: Rect) -> Rect {
    Rect::from_min_max(pos2(card.left() + 8.0, card.top() + BAR_H), pos2(card.right() - 8.0, card.bottom() - 8.0))
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let card = ui.max_rect();
    super::panel::title(ui, card, t!("Viewer"));
    let bar = Rect::from_min_max(pos2(card.left() + 74.0, card.top() + 4.0), pos2(card.right() - theme::PAD, card.top() + BAR_H - 2.0));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    controls(p, &mut bar_ui);
    let pic = picture_rect(card);
    p.look.back = { let c = pal.well(); [c.r(), c.g(), c.b()] };
    let painter = ui.painter_at(pic);
    painter.rect_filled(pic, theme::TILE_RADIUS, pal.well());

    // 2D: the window over the terrain keeps the picture's shape.
    let aspect = pic.width() / pic.height().max(1.0);
    let (ax, az) = if aspect >= 1.0 { (aspect, 1.0) } else { (1.0, 1.0 / aspect) };
    let span = p.look.span;
    p.look.window = [p.look.center[0] - span * ax / 2.0, p.look.center[1] - span * az / 2.0, p.look.center[0] + span * ax / 2.0, p.look.center[1] + span * az / 2.0];

    let resp = ui.interact(pic, ui.scope_id().with("terrain viewer"), Sense::click_and_drag());
    let shift = ui.input(|i| i.modifiers.shift);
    let editing_points = !p.look.three_d && editable(p).is_some();
    if resp.dragged() && (shift || !editing_points || !ui.ctx().is_being_dragged(ui.scope_id().with("viewer point"))) {
        let d = resp.drag_delta();
        if shift {
            // The sun turned, as in the Render workspace.
            p.look.sun[0] = (p.look.sun[0] + d.x * 0.5).rem_euclid(360.0);
            p.look.sun[1] = (p.look.sun[1] - d.y * 0.25).clamp(4.0, 85.0);
        } else if p.look.three_d {
            p.look.yaw = (p.look.yaw - d.x * 0.4).rem_euclid(360.0);
            p.look.pitch = (p.look.pitch + d.y * 0.3).clamp(8.0, 85.0);
        } else if !ui.ctx().dragged_id().is_some_and(|i| i != resp.id) {
            let per = span / pic.height().min(pic.width()).max(1.0);
            p.look.center[0] -= d.x * per;
            p.look.center[1] -= d.y * per;
        }
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0);
        if scroll != 0.0 {
            let f = (-scroll * 0.002).exp();
            if p.look.three_d {
                p.look.distance = (p.look.distance * f).clamp(0.3, 3.0);
            } else {
                // Zoom around the pointer.
                if let Some(q) = resp.hover_pos() {
                    let w = p.look.window;
                    let at = [w[0] + (w[2] - w[0]) * (q.x - pic.left()) / pic.width(), w[1] + (w[3] - w[1]) * (q.y - pic.top()) / pic.height()];
                    let new = (span * f).clamp(0.02, 2.5);
                    let k = new / span;
                    p.look.center = [at[0] + (p.look.center[0] - at[0]) * k, at[1] + (p.look.center[1] - at[1]) * k];
                    p.look.span = new;
                }
            }
        }
        if shift {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }
    if resp.double_clicked() && !editing_points {
        if p.look.three_d {
            p.look.yaw = 25.0;
            p.look.pitch = 34.0;
            p.look.distance = 1.0;
        } else {
            p.look.center = [0.5, 0.5];
            p.look.span = 1.0;
        }
    }

    match &p.viewer {
        Some((tex, picture)) => {
            // The picture as made (the viewer may have changed size since).
            let size = vec2(picture.size[0] as f32, picture.size[1] as f32);
            let scale = (pic.width() / size.x).min(pic.height() / size.y);
            let r = Rect::from_center_size(pic.center(), size * scale);
            egui::Image::new((tex.id(), r.size())).corner_radius(theme::TILE_RADIUS).paint_at(ui, r);
            legend(ui, &painter, pic, picture, &pal);
            if !picture.sharp {
                chip(&painter, pic.right_top() + vec2(-10.0, 10.0), Align2::RIGHT_TOP, t!("refining…"));
            }
        }
        None => {
            let text = if p.graph.nodes.is_empty() {
                t!("Add nodes to see the terrain they make.")
            } else if p.previews_idle() {
                t!("Connect a node to the Map Output's Height to see the terrain.")
            } else {
                t!("Computing…")
            };
            painter.text(pic.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), pal.weak);
        }
    }
    if !p.look.three_d {
        points_overlay(p, ui, pic, &pal);
    }
    // The sun's bearing, at the top left, and how to turn it.
    let sun_at = pic.left_top() + vec2(22.0, 22.0);
    painter.circle(sun_at, 12.0, Color32::from_black_alpha(90), Stroke::NONE);
    let a = p.look.sun[0].to_radians();
    let reach = 9.0 * (1.0 - p.look.sun[1] / 100.0);
    let s = sun_at + vec2(a.sin(), -a.cos()) * reach;
    painter.circle_filled(s, 3.2, Color32::from_rgb(255, 214, 120));
    let sun_hit = ui.interact(Rect::from_center_size(sun_at, vec2(26.0, 26.0)), ui.scope_id().with("sun"), Sense::hover());
    sun_hit.on_hover_text(t!("Sun: {bearing}° bearing, {height}° high. Shift + drag on the picture turns it.", bearing = format!("{:.0}", p.look.sun[0]), height = format!("{:.0}", p.look.sun[1])));
}

fn chip(painter: &egui::Painter, at: Pos2, align: Align2, text: &str) {
    let g = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), Color32::WHITE);
    let r = align.anchor_size(at, g.size() + vec2(14.0, 6.0));
    painter.rect_filled(r, theme::PILL, Color32::from_black_alpha(150));
    painter.galley(r.min + vec2(7.0, 3.0), g, Color32::WHITE);
}

/// What the picture shows, its kind and range, at its bottom left.
fn legend(_ui: &Ui, painter: &egui::Painter, pic: Rect, picture: &super::eval::Picture, _pal: &Palette) {
    let Some(kind) = picture.kind else { return };
    let range = match kind {
        PortKind::Height => format!("{} · {:.0} – {:.0} b", sockets::kind_label(kind), picture.range.0, picture.range.1),
        PortKind::Mask => format!("{} · {:.2} – {:.2}", sockets::kind_label(kind), picture.range.0, picture.range.1),
        _ => sockets::kind_label(kind).to_owned(),
    };
    let text = format!("{range} · {:.0} ms", picture.ms);
    let at = pic.left_bottom() + vec2(10.0, -10.0);
    let g = painter.layout_no_wrap(text, FontId::proportional(11.0), Color32::WHITE);
    let r = Align2::LEFT_BOTTOM.anchor_size(at, g.size() + vec2(26.0, 6.0));
    painter.rect_filled(r, theme::PILL, Color32::from_black_alpha(150));
    painter.circle_filled(r.left_center() + vec2(10.0, 0.0), 4.0, sockets::color(kind));
    painter.galley(r.min + vec2(19.0, 3.0), g, Color32::WHITE);
    let _ = StrokeKind::Inside;
}

fn controls(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    if let Some(k) = widgets::button_group(ui, "terrain viewer mode", &[t!("2D"), t!("3D")], &[!p.look.three_d, p.look.three_d]) {
        p.look.three_d = k == 1;
    }
    ui.add_space(4.0);
    let colour = p.look.surface == Surface::Colour;
    if let Some(k) = widgets::button_group(ui, "terrain viewer surface", &[t!("Clay"), t!("Colours")], &[!colour, colour]) {
        p.look.surface = if k == 0 { Surface::Clay } else { Surface::Colour };
    }
    ui.add_space(8.0);
    // What is viewed: a node, and which of its outputs.
    let crumbs = p.crumbs();
    let viewed_here = p.viewed.clone().filter(|v| v.0 == crumbs).map(|v| v.1);
    let shown = viewed_here.clone().or_else(|| if p.viewed.is_none() && crumbs.is_empty() { p.active_output().map(|node| Shown { node, port: None }) } else { None });
    if let Some(shown) = shown.filter(|s| p.graph.nodes.contains_key(&s.node)) {
        let shape = p.shape(&shown.node);
        let mut options: Vec<(Option<String>, String)> = Vec::new();
        if let Some(shape) = &shape {
            if shape.is_output() {
                options.push((None, t!("Map").to_owned()));
                for name in p.graph.nodes[&shown.node].inputs.keys() {
                    let label = shape.inputs.iter().find(|i| i.name == *name).map_or_else(|| name.trim_start_matches("mask.").to_owned(), |i| i.label.clone());
                    options.push((Some(name.clone()), label));
                }
            } else {
                for (k, o) in shape.outputs.iter().enumerate() {
                    options.push((if k == 0 { None } else { Some(o.name.clone()) }, o.label.clone()));
                }
            }
        }
        let current = options.iter().find(|o| o.0 == shown.port).map_or(String::new(), |o| o.1.clone());
        let title = shape.map_or(shown.node.clone(), |s| super::canvas::node_title(p, &shown.node, &s));
        ui.label(egui::RichText::new(t!("Viewing")).size(12.0).color(pal.weak));
        ui.add(egui::Label::new(egui::RichText::new(&title).size(12.0)).truncate());
        if options.len() > 1 {
            let mut port = shown.port.clone();
            widgets::combo("terrain viewer output").selected_text(current).show_ui(ui, |ui| {
                for (k, label) in &options {
                    ui.selectable_value(&mut port, k.clone(), label);
                }
            });
            if port != shown.port {
                p.viewed = Some((crumbs.clone(), Shown { node: shown.node.clone(), port }));
            }
        }
    } else if let Some((_, v)) = &p.viewed {
        ui.label(egui::RichText::new(t!("Viewing")).size(12.0).color(pal.weak));
        ui.add(egui::Label::new(egui::RichText::new(&v.node).size(12.0)).truncate());
    }
    if p.viewed.is_some() && ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false)).on_hover_text(t!("Back to the map output")).clicked() {
        p.viewed = None;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(theme::primary_button(ui, t!("Apply to map"))).on_hover_text(t!("Write the graph's result into the map, in one step you can undo")).clicked() {
            p.events.push(Event::Apply);
        }
        ui.add_space(6.0);
        let mut live = p.live_on;
        if widgets::switch(ui, &mut live).on_hover_text(t!("Show the graph's result on the World map as it is edited, without writing it")).changed() {
            p.live_on = live;
            if let Some(l) = &p.live {
                if live {
                    l.set_graph(&p.document(), p.space, p.env.clone(), p.revision);
                } else {
                    l.clear();
                }
            }
        }
        ui.label(egui::RichText::new(t!("Live")).size(12.0));
        ui.add_space(8.0);
        let r = ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::SlidersHorizontal).to_string()).size(13.0)).frame(false));
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(220.0);
            widgets::well(ui, |ui| {
                widgets::row(ui, t!("Relief"), |ui| {
                    let mut v = p.look.relief as f64;
                    if ui.add(widgets::Field::new(&mut v, 0.25..=4.0).suffix("×").width(110.0)).changed() {
                        p.look.relief = v as f32;
                    }
                });
                widgets::row(ui, t!("Contour lines"), |ui| {
                    widgets::switch(ui, &mut p.look.contours);
                });
                widgets::row(ui, t!("Mask opacity"), |ui| {
                    let mut v = p.look.overlay as f64 * 100.0;
                    if ui.add(widgets::Field::new(&mut v, 0.0..=100.0).suffix(" %").max_decimals(0).width(110.0)).changed() {
                        p.look.overlay = (v / 100.0) as f32;
                    }
                });
                widgets::row(ui, t!("Sun height"), |ui| {
                    let mut v = p.look.sun[1] as f64;
                    if ui.add(widgets::Field::new(&mut v, 4.0..=85.0).suffix("°").max_decimals(0).width(110.0)).changed() {
                        p.look.sun[1] = v as f32;
                    }
                });
                widgets::row(ui, t!("Thumbnails"), |ui| {
                    let colour = p.thumb_surface == Surface::Colour;
                    if let Some(k) = widgets::button_group(ui, "terrain thumbs surface", &[t!("Clay"), t!("Colours")], &[!colour, colour]) {
                        p.thumb_surface = if k == 0 { Surface::Clay } else { Surface::Colour };
                    }
                });
            });
        });
    });
}

/// The first Points or Curve setting of the selected node.
fn editable(p: &TerrainPanel) -> Option<(String, &'static massif_terrain::registry::ParamSpec, usize)> {
    let id = p.selected.last()?;
    let spec = p.spec(id)?;
    spec.params.iter().enumerate().find(|(_, q)| matches!(q.kind, ParamKind::Points { .. } | ParamKind::Curve { .. })).map(|(k, q)| (id.clone(), q, k))
}

/// The selected node's points (or its line) over the 2D picture: dragged
/// to move them, double-click to add one, right-click to remove one.
fn points_overlay(p: &mut TerrainPanel, ui: &mut Ui, pic: Rect, pal: &Palette) {
    let Some((id, q, k)) = editable(p) else { return };
    let Some(params) = super::props::decoded(p, &id) else { return };
    let Value::Points(mut pts) = params.values()[k].clone() else { return };
    let w = p.look.window;
    // The picture is drawn to fit the card; the window maps onto it.
    let picture = p.viewer.as_ref().map(|v| vec2(v.1.size[0] as f32, v.1.size[1] as f32)).unwrap_or(pic.size());
    let scale = (pic.width() / picture.x).min(pic.height() / picture.y);
    let shown = Rect::from_center_size(pic.center(), picture * scale);
    let to_screen = |x: f64, z: f64| pos2(shown.left() + ((x as f32 - w[0]) / (w[2] - w[0])) * shown.width(), shown.top() + ((z as f32 - w[1]) / (w[3] - w[1])) * shown.height());
    let to_terrain = |s: Pos2| (((s.x - shown.left()) / shown.width() * (w[2] - w[0]) + w[0]) as f64, ((s.y - shown.top()) / shown.height() * (w[3] - w[1]) + w[1]) as f64);
    let painter = ui.painter_at(pic);
    let curve = matches!(q.kind, ParamKind::Curve { .. });
    let accent = Color32::from_rgb(255, 160, 40);
    if curve && pts.len() >= 2 {
        let line: Vec<Pos2> = pts.iter().map(|pt| to_screen(pt.x, pt.z)).collect();
        let closed = matches!(q.kind, ParamKind::Curve { closed: true, .. });
        let mut all = line.clone();
        if closed {
            all.push(line[0]);
        }
        painter.extend(egui::Shape::dashed_line(&all, Stroke::new(2.0, accent), 8.0, 5.0));
    }
    let mut changed = false;
    let mut remove = None;
    for (j, pt) in pts.iter_mut().enumerate() {
        let c = to_screen(pt.x, pt.z);
        let r = ui.interact(Rect::from_center_size(c, vec2(18.0, 18.0)), ui.scope_id().with(("viewer point", j)), Sense::click_and_drag());
        painter.circle(c, if r.hovered() || r.dragged() { 7.0 } else { 5.5 }, accent, Stroke::new(1.5, Color32::WHITE));
        if r.dragged()
            && let Some(q) = r.interact_pointer_pos()
        {
            let (x, z) = to_terrain(q);
            pt.x = x.clamp(0.0, 1.0);
            pt.z = z.clamp(0.0, 1.0);
            changed = true;
        }
        if r.secondary_clicked() {
            remove = Some(j);
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }
    if let Some(j) = remove {
        pts.remove(j);
        changed = true;
    }
    let max = match q.kind {
        ParamKind::Points { max, .. } => max,
        _ => 64,
    };
    let bg = ui.interact(pic, ui.scope_id().with("terrain viewer"), Sense::click_and_drag());
    if bg.double_clicked()
        && pts.len() < max
        && let Some(s) = bg.interact_pointer_pos()
    {
        let (x, z) = to_terrain(s);
        let fields = match q.kind {
            ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. } => fields,
            _ => &[],
        };
        let values = pts.last().map(|l| l.values.clone()).unwrap_or_else(|| fields.iter().map(|f| f.default).collect());
        pts.push(massif_terrain::params::Point { x: x.clamp(0.0, 1.0), z: z.clamp(0.0, 1.0), values });
        changed = true;
    }
    chip(&painter, pic.center_top() + vec2(0.0, 10.0), Align2::CENTER_TOP, &t!("{name}: drag the points, double-click to add one, right-click to remove one", name = super::text::name(q.label)));
    if changed {
        let j = super::props::encode(q, &Value::Points(pts));
        p.edit().set(&id, q.name, j);
    }
    let _ = pal;
}
