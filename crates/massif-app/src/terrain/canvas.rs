//! The graph canvas: nodes with their thumbnails and typed sockets, wires
//! (a dot where a value is converted), reroute points, frames, a minimap,
//! and the search to add a node.

use super::panel::{Frame, Search, TerrainPanel, Wiring};
use super::sockets::{self, Shape_};
use crate::theme::{self, Icon, Palette};
use egui::{Align2, Color32, CursorIcon, FontId, Key, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use massif_terrain::model::Source;
use massif_terrain::registry::{self, Conversion, Family, NodeSpec, ParamKind, PortKind};
use std::collections::HashMap;

pub const WIDTH: f32 = 176.0;
pub const OUTPUT_WIDTH: f32 = 236.0;
const HEADER: f32 = 28.0;
const ROW: f32 = 20.0;
const THUMB_H: f32 = 104.0;
const PAD: f32 = 8.0;
const SOCKET_R: f32 = 4.5;

/// The groups of a Map Output's inputs, as on the node.
pub const OUTPUT_GROUPS: &[(&str, &str)] = &[
    ("ground", "Ground"),
    ("surface", "Surface"),
    ("climate", "Climate"),
    ("life", "Life"),
    ("underground", "Underground"),
    ("rock", "Rock over the void"),
    ("masks", "Masks"),
    ("other", "Other"),
];

/// The group a Map Output's input shows in (an input the registry adds
/// later lands in Other until it has one).
pub fn output_group(name: &str) -> &'static str {
    match name {
        "height" | "water" => "ground",
        "albedo" | "terrain" | "biome" => "surface",
        "temperature" | "humidity" => "climate",
        "vegetation" => "life",
        "caves" => "underground",
        n if n.starts_with("layer.") => "rock",
        n if n.starts_with("mask.") => "masks",
        _ => "other",
    }
}

/// A line of a node, from the top.
#[derive(Debug, Clone)]
pub enum Row {
    Output(usize),
    Thumb,
    /// An input socket: its name in the graph (`height`, `mask.moss`,
    /// `@height` for a driven setting), its words, its type, its shape.
    Input { name: String, label: String, kind: PortKind, form: Shape_, default: &'static str },
    /// A Map Output's group: its key, its words, whether open, inputs
    /// connected in it.
    Group { key: &'static str, label: &'static str, open: bool, used: usize },
    AddMask,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Thumb => THUMB_H + PAD,
            Row::Group { .. } => ROW + 2.0,
            _ => ROW,
        }
    }
}

/// Whether a Map Output's group is open: Ground and Surface by default, and
/// any group with an input connected; a click turns it the other way.
pub fn group_open(p: &TerrainPanel, id: &str, key: &str, used: usize) -> bool {
    let default = matches!(key, "ground" | "surface") || used > 0;
    default != p.layout.folded.contains(&format!("{id}.{key}"))
}

/// The lines of node `id`, after its header.
pub fn rows(p: &TerrainPanel, id: &str) -> Vec<Row> {
    let Some(spec) = p.spec(id) else { return Vec::new() };
    if p.collapsed(id) {
        return Vec::new();
    }
    let node = &p.graph.nodes[id];
    let mut out = Vec::new();
    if spec.is_output() {
        out.push(Row::Thumb);
        for (key, label) in OUTPUT_GROUPS {
            let mut inputs: Vec<(String, String, PortKind, &'static str)> = spec.inputs.iter().filter(|i| output_group(i.name) == *key).map(|i| (i.name.to_owned(), super::text::name(i.label).to_owned(), i.kind, i.default)).collect();
            if *key == "masks"
                && let Some(open) = spec.open_inputs
            {
                for name in node.inputs.keys().filter(|k| spec.open_input(k).is_some() && spec.input(k).is_none()) {
                    let label = name.strip_prefix(open.name).unwrap_or(name);
                    inputs.push((name.clone(), capitalise(label), PortKind::Mask, open.default));
                }
                for name in p.layout.exposed.iter().filter_map(|e| e.strip_prefix(&format!("{id}."))).filter(|n| n.starts_with(open.name) && !node.inputs.contains_key(*n)) {
                    let label = name.strip_prefix(open.name).unwrap_or(name);
                    inputs.push((name.to_owned(), capitalise(label), PortKind::Mask, open.default));
                }
            }
            if inputs.is_empty() && *key != "masks" {
                continue;
            }
            let used = inputs.iter().filter(|i| node.inputs.contains_key(&i.0)).count();
            let open = group_open(p, id, key, used);
            out.push(Row::Group { key, label, open, used });
            if open {
                for (name, label, kind, default) in inputs {
                    out.push(Row::Input { name, label, kind, form: sockets::shape(kind), default });
                }
                if *key == "masks" {
                    out.push(Row::AddMask);
                }
            }
        }
        return out;
    }
    out.extend((0..spec.outputs.len()).map(Row::Output));
    out.push(Row::Thumb);
    for i in spec.inputs {
        out.push(Row::Input { name: i.name.to_owned(), label: super::text::name(i.label).to_owned(), kind: i.kind, form: sockets::shape(i.kind), default: i.default });
    }
    for prm in spec.params.iter().filter(|q| q.drivable) {
        let input = format!("@{}", prm.name);
        if node.inputs.contains_key(&input) || p.layout.exposed.contains(&format!("{id}.{}", prm.name)) {
            out.push(Row::Input { name: input, label: super::text::name(prm.label).to_owned(), kind: PortKind::Mask, form: Shape_::Diamond, default: "" });
        }
    }
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect::<String>().replace(['_', '-'], " "),
        None => String::new(),
    }
}

/// A node's size, in graph units.
pub fn node_size(p: &TerrainPanel, id: &str) -> Vec2 {
    let w = if p.spec(id).is_some_and(|s| s.is_output()) { OUTPUT_WIDTH } else { WIDTH };
    let h = HEADER + rows(p, id).iter().map(Row::height).sum::<f32>() + if p.collapsed(id) { 0.0 } else { 6.0 };
    vec2(w, h)
}

/// Where everything of a node is on screen.
struct Placed {
    id: String,
    spec: &'static NodeSpec,
    rect: Rect,
    rows: Vec<(Row, Rect)>,
}

impl Placed {
    fn input_at(&self, name: &str) -> Option<Pos2> {
        if self.rows.is_empty() {
            return Some(self.rect.left_center());
        }
        self.rows.iter().find_map(|(r, rect)| matches!(r, Row::Input { name: n, .. } if n == name).then(|| pos2(self.rect.left(), rect.center().y)))
    }

    fn output_at(&self, k: usize) -> Option<Pos2> {
        if self.rows.is_empty() {
            return Some(self.rect.right_center());
        }
        self.rows.iter().find_map(|(r, rect)| matches!(r, Row::Output(j) if *j == k).then(|| pos2(self.rect.right(), rect.center().y)))
    }
}

fn to_screen(p: &TerrainPanel, origin: Pos2, g: [f32; 2]) -> Pos2 {
    origin + p.view.offset + vec2(g[0], g[1]) * p.view.zoom
}

fn to_graph(p: &TerrainPanel, origin: Pos2, s: Pos2) -> [f32; 2] {
    let v = (s - origin - p.view.offset) / p.view.zoom;
    [v.x, v.y]
}

fn place(p: &TerrainPanel, origin: Pos2) -> Vec<Placed> {
    let z = p.view.zoom;
    let mut ids: Vec<&String> = p.graph.nodes.keys().collect();
    // The selected drawn last, over the others.
    ids.sort_by_key(|id| p.selected.contains(id));
    ids.into_iter()
        .filter_map(|id| {
            let spec = p.spec(id)?;
            let size = node_size(p, id);
            let rect = Rect::from_min_size(to_screen(p, origin, p.pos(id)), size * z);
            let mut y = rect.top() + HEADER * z;
            let rows = rows(p, id)
                .into_iter()
                .map(|r| {
                    let h = r.height() * z;
                    let rr = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), h));
                    y += h;
                    (r, rr)
                })
                .collect();
            Some(Placed { id: id.clone(), spec, rect, rows })
        })
        .collect()
}

/// A wire's curve from `a` to `b`, through reroute points.
fn wire_points(a: Pos2, b: Pos2, via: &[Pos2], zoom: f32) -> Vec<[Pos2; 4]> {
    let mut stops = vec![a];
    stops.extend_from_slice(via);
    stops.push(b);
    stops
        .windows(2)
        .map(|w| {
            let (s, e) = (w[0], w[1]);
            let d = ((e.x - s.x).abs() * 0.5).clamp(24.0 * zoom, 160.0 * zoom);
            [s, s + vec2(d, 0.0), e - vec2(d, 0.0), e]
        })
        .collect()
}

fn bezier(p: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let v = p[0].to_vec2() * u * u * u + p[1].to_vec2() * 3.0 * u * u * t + p[2].to_vec2() * 3.0 * u * t * t + p[3].to_vec2() * t * t * t;
    v.to_pos2()
}

fn distance_to(curves: &[[Pos2; 4]], q: Pos2) -> (f32, Pos2) {
    let mut best = (f32::MAX, q);
    for c in curves {
        let mut last = c[0];
        for k in 1..=24 {
            let pt = bezier(*c, k as f32 / 24.0);
            let (s, e) = (last, pt);
            let t = ((q - s).dot(e - s) / (e - s).length_sq().max(1e-6)).clamp(0.0, 1.0);
            let on = s + (e - s) * t;
            let d = q.distance(on);
            if d < best.0 {
                best = (d, on);
            }
            last = pt;
        }
    }
    best
}

/// A wire, as drawn.
struct Wire {
    from: String,
    port: u8,
    to: String,
    input: String,
    key: String,
    curves: Vec<[Pos2; 4]>,
    kind: PortKind,
    target: PortKind,
}

/// What the user did on the canvas this frame, applied once it is drawn.
enum Act {
    Select(String, bool),
    Move(Vec2),
    View(String),
    ToggleCollapse(String),
    StartWire(String, u8),
    Detach(String, String),
    Connect(String, u8, String, String),
    Cut(String, String),
    AddReroute(String, usize, [f32; 2]),
    MoveReroute(String, usize, [f32; 2]),
    RemoveReroute(String, usize),
    OpenSearch(Pos2, Option<PortKind>, Option<(String, u8)>),
    MoveFrame(usize, Vec2),
    ResizeFrame(usize, Vec2),
    RemoveFrame(usize),
    ToggleGroup(String, &'static str),
    AddMask(String),
    RenameMask(String, String),
    SetActive(String),
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let full = ui.max_rect();
    super::panel::title(ui, full, t!("Graph"));
    // Commands at the card's top right.
    let bar = Rect::from_min_max(pos2(full.left() + 80.0, full.top() + 4.0), pos2(full.right() - theme::PAD, full.top() + 32.0));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    header_commands(p, &mut bar_ui);
    let canvas = Rect::from_min_max(pos2(full.left() + 1.0, full.top() + 36.0), pos2(full.right() - 1.0, full.bottom() - 1.0));
    let origin = canvas.min;
    if p.fit_pending {
        p.fit_pending = false;
        fit_view(p, canvas);
    }
    let painter = ui.painter_at(canvas);
    let background = ui.interact(canvas, ui.scope_id().with("terrain canvas"), Sense::click_and_drag());
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let hovered = pointer.is_some_and(|q| canvas.contains(q)) && ui.ui_contains_pointer();
    let zoom = p.view.zoom;

    // Pan by scrolling or with the middle button; zoom by pinching or
    // with ⌘ / Ctrl and the wheel, around the pointer.
    if hovered && p.search.is_none() {
        let (scroll, zoom_delta) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        if zoom_delta != 1.0
            && let Some(q) = pointer
        {
            let before = to_graph(p, origin, q);
            p.view.zoom = (p.view.zoom * zoom_delta).clamp(0.25, 2.5);
            let after = to_screen(p, origin, before);
            p.view.offset += q - after;
        } else if scroll != Vec2::ZERO {
            p.view.offset += scroll;
        }
    }
    if background.dragged_by(egui::PointerButton::Middle) || (background.dragged_by(egui::PointerButton::Primary) && ui.input(|i| i.key_down(Key::Space))) {
        p.view.offset += background.drag_delta();
    }

    // The dotted ground, moving with the view.
    let step = 24.0 * p.view.zoom;
    if step > 6.0 {
        let dot = if pal.dark { theme::mix(pal.panel, pal.text, 0.12) } else { theme::mix(pal.panel, pal.text, 0.14) };
        let start = origin + vec2(p.view.offset.x.rem_euclid(step), p.view.offset.y.rem_euclid(step));
        let mut y = start.y;
        while y < canvas.bottom() {
            let mut x = start.x;
            while x < canvas.right() {
                painter.circle_filled(pos2(x, y), 0.9, dot);
                x += step;
            }
            y += step;
        }
    }

    let placed = place(p, origin);
    let by_id: HashMap<&str, &Placed> = placed.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut acts: Vec<Act> = Vec::new();
    let modifiers = ui.input(|i| i.modifiers);

    // Frames, behind everything.
    for (k, f) in p.layout.frames.iter().enumerate() {
        let r = Rect::from_min_size(to_screen(p, origin, [f.rect[0], f.rect[1]]), vec2(f.rect[2], f.rect[3]) * zoom);
        if !r.intersects(canvas) {
            continue;
        }
        let tint = f.color.map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(pal.weak);
        painter.rect(r, theme::GROUP_RADIUS, tint.gamma_multiply(if pal.dark { 0.07 } else { 0.06 }), Stroke::new(1.0, tint.gamma_multiply(0.45)), StrokeKind::Inside);
        let bar = Rect::from_min_size(r.min, vec2(r.width(), 26.0 * zoom));
        if p.renaming_frame == Some(k) {
            continue;
        }
        painter.with_clip_rect(bar.intersect(canvas)).text(bar.left_center() + vec2(10.0 * zoom, 0.0), Align2::LEFT_CENTER, &f.title, FontId::proportional(12.5 * zoom), pal.weak);
        let handle = ui.interact(bar, ui.scope_id().with(("frame", k)), Sense::click_and_drag());
        if handle.dragged_by(egui::PointerButton::Primary) {
            acts.push(Act::MoveFrame(k, handle.drag_delta() / zoom));
        }
        if handle.double_clicked() {
            p.renaming_frame = Some(k);
        }
        handle.context_menu(|ui| {
            if ui.button(t!("Rename frame")).clicked() {
                p.renaming_frame = Some(k);
                ui.close();
            }
            if ui.button(t!("Remove frame")).clicked() {
                acts.push(Act::RemoveFrame(k));
                ui.close();
            }
        });
        let corner = Rect::from_center_size(r.right_bottom() - vec2(6.0, 6.0), vec2(14.0, 14.0));
        let c = ui.interact(corner, ui.scope_id().with(("frame size", k)), Sense::drag());
        if c.hovered() || c.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
        }
        if c.dragged() {
            acts.push(Act::ResizeFrame(k, c.drag_delta() / zoom));
        }
    }
    if let Some(k) = p.renaming_frame
        && let Some(f) = p.layout.frames.get(k)
    {
        let r = Rect::from_min_size(to_screen(p, origin, [f.rect[0], f.rect[1]]), vec2(f.rect[2].min(260.0), 26.0) * zoom);
        let mut title = f.title.clone();
        let edit = ui.put(r.shrink2(vec2(6.0, 3.0)), egui::TextEdit::singleline(&mut title).font(FontId::proportional(12.5 * zoom)));
        edit.request_focus();
        if title != f.title {
            p.edit_layout().frames[k].title = title;
        }
        if edit.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
            p.renaming_frame = None;
        }
    }

    // Wires.
    let mut wires = Vec::new();
    for to in &placed {
        let node = &p.graph.nodes[&to.id];
        for (input, src) in &node.inputs {
            let s = Source::parse(src);
            let Some(from) = by_id.get(s.node) else { continue };
            let port = match s.output {
                Some(o) => from.spec.output(o).unwrap_or(0),
                None => 0,
            };
            let (Some(a), Some(b)) = (from.output_at(port), to.input_at(input)) else { continue };
            let key = format!("{}.{input}", to.id);
            let via: Vec<Pos2> = p.layout.reroutes.get(&key).map(|v| v.iter().map(|g| to_screen(p, origin, *g)).collect()).unwrap_or_default();
            let kind = from.spec.outputs.get(port).map_or(PortKind::Height, |o| o.kind);
            let target = input_kind(to.spec, input).unwrap_or(kind);
            wires.push(Wire { from: from.id.clone(), port: port as u8, to: to.id.clone(), input: input.clone(), key, curves: wire_points(a, b, &via, zoom), kind, target });
        }
    }
    let over_node = pointer.is_some_and(|q| placed.iter().any(|n| n.rect.expand(8.0 * zoom).contains(q)));
    let hot_wire = match (pointer, hovered, over_node, &p.wiring) {
        (Some(q), true, false, None) => wires.iter().enumerate().map(|(k, w)| (k, distance_to(&w.curves, q))).filter(|(_, d)| d.0 < 6.0).min_by(|a, b| a.1.0.total_cmp(&b.1.0)).map(|(k, d)| (k, d.1)),
        _ => None,
    };
    for (k, w) in wires.iter().enumerate() {
        let near = p.selected.contains(&w.from) || p.selected.contains(&w.to);
        let hot = hot_wire.is_some_and(|h| h.0 == k);
        let conv = w.kind.conversion(w.target);
        let base = sockets::color_on(w.kind, &pal);
        let color = if conv.is_none() {
            pal.danger
        } else if hot {
            theme::mix(base, pal.text, 0.35)
        } else if near || p.selected.is_empty() {
            base
        } else {
            base.gamma_multiply(0.55)
        };
        let width = if near || hot { 2.4 } else { 1.8 } * zoom.max(0.6);
        for c in &w.curves {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(*c, false, Color32::TRANSPARENT, Stroke::new(width, color)));
        }
        // A value converted on its way: a dot in the input's colour, as
        // Blender draws one.
        if let Some(conv) = conv
            && (conv != Conversion::AsIs || w.kind != w.target)
        {
            let mid = w.curves[w.curves.len() / 2];
            let at = bezier(mid, 0.5);
            painter.circle(at, 3.6 * zoom.max(0.7), sockets::color_on(w.target, &pal), Stroke::new(1.5, base), );
        }
        // Reroute points, dragged; right-click removes.
        if let Some(via) = p.layout.reroutes.get(&w.key) {
            for (j, g) in via.iter().enumerate() {
                let c = to_screen(p, origin, *g);
                let r = ui.interact(Rect::from_center_size(c, vec2(14.0, 14.0)), ui.scope_id().with(("reroute", &w.key, j)), Sense::click_and_drag());
                painter.circle(c, if r.hovered() { 5.0 } else { 4.0 } * zoom.max(0.7), base, Stroke::new(1.0, pal.panel));
                if r.dragged() {
                    let at = to_graph(p, origin, c + r.drag_delta());
                    acts.push(Act::MoveReroute(w.key.clone(), j, at));
                }
                if r.secondary_clicked() {
                    acts.push(Act::RemoveReroute(w.key.clone(), j));
                }
            }
        }
    }
    if let Some((k, at)) = hot_wire {
        let w = &wires[k];
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        if background.secondary_clicked() || (background.clicked() && modifiers.alt) {
            acts.push(Act::Cut(w.to.clone(), w.input.clone()));
        } else if background.double_clicked() {
            // A reroute point where the wire was clicked, in its order.
            let seg = w.curves.iter().position(|c| distance_to(&[*c], at).0 < 6.5).unwrap_or(0);
            acts.push(Act::AddReroute(w.key.clone(), seg, to_graph(p, origin, at)));
        } else {
            background.clone().on_hover_text(t!("Double-click: add a reroute point. Right-click: cut the wire."));
        }
    }

    // The wire being drawn, and where it would plug in.
    let mut drop_target: Option<(String, String)> = None;
    if let (Some(Wiring::From { node, port }), Some(q)) = (&p.wiring, pointer)
        && let Some(from) = by_id.get(node.as_str())
        && let Some(a) = from.output_at(*port as usize)
    {
        let kind = from.spec.outputs.get(*port as usize).map_or(PortKind::Height, |o| o.kind);
        let mut best: Option<((String, String), f32)> = None;
        for n in placed.iter().filter(|n| n.id != *node) {
            for (r, rect) in &n.rows {
                let Row::Input { name, kind: k, .. } = r else { continue };
                if kind.conversion(*k).is_none() {
                    continue;
                }
                let sp = pos2(n.rect.left(), rect.center().y);
                let on_row = Rect::from_min_max(pos2(n.rect.left() - 12.0, rect.top()), pos2(n.rect.center().x, rect.bottom())).contains(q);
                let d = q.distance(sp);
                if (on_row || d < 16.0) && best.as_ref().is_none_or(|b| d < b.1) {
                    best = Some(((n.id.clone(), name.clone()), d));
                }
            }
        }
        let end = best.as_ref().and_then(|b| by_id.get(b.0.0.as_str()).and_then(|n| n.input_at(&b.0.1))).unwrap_or(q);
        drop_target = best.map(|b| b.0);
        for c in wire_points(a, end, &[], zoom) {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(c, false, Color32::TRANSPARENT, Stroke::new(2.2 * zoom.max(0.6), sockets::color_on(kind, &pal))));
        }
        if ui.input(|i| i.pointer.any_released()) {
            match drop_target.clone() {
                Some((to, input)) => acts.push(Act::Connect(node.clone(), *port, to, input)),
                None if canvas.contains(q) => acts.push(Act::OpenSearch(q, Some(kind), Some((node.clone(), *port)))),
                None => {}
            }
            p.wiring = None;
        }
    }

    // Nodes.
    let status = p.status();
    let shown = p.shown();
    let active = p.active_output();
    for n in &placed {
        if !n.rect.intersects(canvas) {
            continue;
        }
        draw_node(p, ui, &painter, n, &pal, &status, shown.as_ref(), active.as_deref(), drop_target.as_ref(), &mut acts);
    }

    // A box to select nodes with, dragged on the ground.
    if background.drag_started_by(egui::PointerButton::Primary) && p.wiring.is_none() && hot_wire.is_none() && !ui.input(|i| i.key_down(Key::Space)) {
        p.marquee = ui.input(|i| i.pointer.press_origin());
        if !(modifiers.shift || modifiers.command) {
            p.selected.clear();
        }
    }
    if let (Some(start), Some(now)) = (p.marquee, pointer) {
        let area = Rect::from_two_pos(start, now);
        painter.rect(area, 2, pal.accent.gamma_multiply(0.08), Stroke::new(1.0, pal.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        if !ui.input(|i| i.pointer.primary_down()) {
            for n in &placed {
                if area.intersects(n.rect) && !p.selected.contains(&n.id) {
                    p.selected.push(n.id.clone());
                }
            }
            p.marquee = None;
        }
    }
    if background.clicked() && hot_wire.is_none() {
        p.selected.clear();
        p.search = None;
    }
    if background.secondary_clicked()
        && hot_wire.is_none()
        && let Some(q) = background.interact_pointer_pos()
    {
        acts.push(Act::OpenSearch(q, None, None));
    }

    if p.graph.nodes.is_empty() {
        painter.text(canvas.center(), Align2::CENTER_CENTER, t!("Right-click or press Tab to add a node, or pick a quick start in the library."), FontId::proportional(13.0), pal.weak);
    }

    // Keys, while the pointer is over the canvas and nothing is typed in.
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if hovered && !typing && p.search.is_none() {
        let (del, dup, all, undo, redo, tab, fit, view, fold, frame) = ui.input_mut(|i| {
            (
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) || i.key_pressed(Key::X) && !i.modifiers.command,
                i.consume_key(egui::Modifiers::COMMAND, Key::D),
                i.consume_key(egui::Modifiers::COMMAND, Key::A),
                i.consume_key(egui::Modifiers::COMMAND, Key::Z),
                i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z) || i.consume_key(egui::Modifiers::COMMAND, Key::Y),
                i.key_pressed(Key::Tab) || (i.modifiers.shift && i.key_pressed(Key::A)),
                i.key_pressed(Key::F) || i.key_pressed(Key::Home),
                i.key_pressed(Key::V),
                i.key_pressed(Key::H),
                i.consume_key(egui::Modifiers::COMMAND, Key::J),
            )
        });
        if del {
            let ids = p.selected.clone();
            p.remove_nodes(&ids);
        }
        if dup {
            let ids = p.selected.clone();
            p.duplicate(&ids);
        }
        if all {
            p.selected = p.graph.nodes.keys().cloned().collect();
        }
        if p.own_undo_keys {
            if redo {
                p.redo();
            } else if undo {
                p.undo();
            }
        }
        if tab && let Some(q) = pointer {
            acts.push(Act::OpenSearch(q, None, None));
        }
        if fit {
            fit_view(p, canvas);
        }
        if view && let Some(id) = p.selected.last().cloned() {
            acts.push(Act::View(id));
        }
        if fold {
            for id in p.selected.clone() {
                acts.push(Act::ToggleCollapse(id));
            }
        }
        if frame {
            frame_selection(p);
        }
    }

    apply(p, acts, origin);
    rename_mask_ui(p, ui, origin);
    minimap(p, ui, canvas, &pal);
    search_ui(p, ui, canvas, &pal);
}

/// The field over a named mask of a Map Output being renamed.
fn rename_mask_ui(p: &mut TerrainPanel, ui: &mut Ui, origin: Pos2) {
    let Some((id, name)) = p.renaming_mask.clone() else { return };
    let placed = place(p, origin);
    let Some(n) = placed.iter().find(|n| n.id == id) else {
        p.renaming_mask = None;
        return;
    };
    let Some(rr) = n.rows.iter().find_map(|(r, rect)| matches!(r, Row::Input { name: k, .. } if *k == name).then_some(*rect)) else {
        p.renaming_mask = None;
        return;
    };
    let z = p.view.zoom;
    let key = ui.scope_id().with(("renaming mask", &id, &name));
    let mut text: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| name.trim_start_matches("mask.").to_owned());
    let field = Rect::from_min_max(pos2(n.rect.left() + 10.0 * z, rr.top() + 1.0), pos2(n.rect.right() - 10.0 * z, rr.bottom() - 1.0));
    let edit = ui.put(field, egui::TextEdit::singleline(&mut text).font(FontId::proportional(11.5 * z)).margin(vec2(4.0, 0.0)));
    if !edit.has_focus() && !edit.lost_focus() {
        edit.request_focus();
    }
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if edit.lost_focus() || enter || escape {
        ui.data_mut(|d| d.remove::<String>(key));
        p.renaming_mask = None;
        let clean: String = text.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let to = format!("mask.{clean}");
        if !escape && to != name && massif_terrain::model::valid_id(&clean) && !p.graph.nodes[&id].inputs.contains_key(&to) && p.spec(&id).is_some_and(|s| s.input(&to).is_none()) {
            let src = p.graph.nodes[&id].inputs.get(&name).cloned();
            let exposed = p.layout.exposed.remove(&format!("{id}.{name}"));
            if let Some(src) = src {
                let g = p.edit();
                if let Some(node) = g.nodes.get_mut(&id) {
                    node.inputs.remove(&name);
                    node.inputs.insert(to.clone(), src);
                }
                if let Some(v) = p.layout.reroutes.remove(&format!("{id}.{name}")) {
                    p.layout.reroutes.insert(format!("{id}.{to}"), v);
                }
            } else if exposed {
                p.edit_layout().exposed.insert(format!("{id}.{to}"));
            }
        }
    } else {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
}

fn input_kind(spec: &NodeSpec, input: &str) -> Option<PortKind> {
    if input.starts_with('@') {
        return Some(PortKind::Mask);
    }
    spec.inputs.iter().find(|i| i.name == input).map(|i| i.kind).or_else(|| spec.open_input(input).map(|o| o.kind))
}

#[allow(clippy::too_many_arguments)]
fn draw_node(p: &TerrainPanel, ui: &mut Ui, painter: &Painter, n: &Placed, pal: &Palette, status: &super::eval::Status, shown: Option<&super::eval::Shown>, active: Option<&str>, drop: Option<&(String, String)>, acts: &mut Vec<Act>) {
    let z = p.view.zoom;
    let rect = n.rect;
    let picked = p.selected.contains(&n.id);
    let primary = p.selected.last() == Some(&n.id);
    let broken = status.broken.get(&n.id);
    let is_output = n.spec.is_output();
    let is_active = is_output && active == Some(n.id.as_str());
    let border = if broken.is_some() {
        pal.danger
    } else if picked {
        pal.accent
    } else {
        pal.border
    };
    let round = (theme::TILE_RADIUS as f32 * z).clamp(2.0, theme::TILE_RADIUS as f32);
    // A soft shadow under the node.
    painter.rect_filled(rect.translate(vec2(0.0, 2.0 * z)).expand(1.0), round, Color32::from_black_alpha(if pal.dark { 70 } else { 22 }));
    let fill = if pal.dark { theme::mix(pal.panel, pal.text, 0.035) } else { pal.panel };
    painter.rect(rect, round, fill, Stroke::new(if primary { 2.0 } else if picked { 1.5 } else { 1.0 }, border), StrokeKind::Inside);
    let family = sockets::family(n.spec.family);
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), (3.0 * z).max(2.0)));
    painter.with_clip_rect(band.intersect(painter.clip_rect())).rect_filled(rect, round, family);
    let text_on = z >= 0.45;
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * z));

    // Anywhere on the node selects and drags it (its sockets excepted).
    let handle = ui.interact(rect, ui.scope_id().with(("terrain node", &n.id)), Sense::click_and_drag());
    if handle.drag_started_by(egui::PointerButton::Primary) && !picked {
        acts.push(Act::Select(n.id.clone(), ui.input(|i| i.modifiers.shift || i.modifiers.command)));
    }
    if handle.clicked() {
        acts.push(Act::Select(n.id.clone(), ui.input(|i| i.modifiers.shift || i.modifiers.command)));
    }
    if handle.dragged_by(egui::PointerButton::Primary) && p.wiring.is_none() {
        acts.push(Act::Move(handle.drag_delta() / z));
    }
    if handle.double_clicked() {
        acts.push(Act::View(n.id.clone()));
    }
    handle.context_menu(|ui| {
        if ui.button(t!("View")).clicked() {
            acts.push(Act::View(n.id.clone()));
            ui.close();
        }
        if ui.button(if p.collapsed(&n.id) { t!("Expand") } else { t!("Collapse") }).clicked() {
            acts.push(Act::ToggleCollapse(n.id.clone()));
            ui.close();
        }
        if is_output && !is_active && ui.button(t!("Make active")).clicked() {
            acts.push(Act::SetActive(n.id.clone()));
            ui.close();
        }
    });

    if text_on {
        // Fold chevron, title, and on the right the eye of the node viewed
        // or the active output's mark.
        let chevron = if p.collapsed(&n.id) { Icon::ChevronRight } else { Icon::ChevronDown };
        let cr = Rect::from_center_size(header.left_center() + vec2(12.0 * z, 1.5 * z), vec2(14.0, 14.0) * z);
        let c = ui.interact(cr, ui.scope_id().with(("fold", &n.id)), Sense::click());
        painter.text(cr.center(), Align2::CENTER_CENTER, theme::glyph(chevron).to_string(), FontId::proportional(11.0 * z), if c.hovered() { pal.text } else { pal.weak });
        if c.clicked() {
            acts.push(Act::ToggleCollapse(n.id.clone()));
        }
        let title = node_title(p, &n.id, n.spec);
        let right_room = if is_output { 70.0 } else { 26.0 } * z;
        let title_rect = Rect::from_min_max(pos2(header.left() + 22.0 * z, header.top()), pos2(header.right() - right_room, header.bottom()));
        painter.with_clip_rect(title_rect.intersect(painter.clip_rect())).text(title_rect.left_center() + vec2(0.0, 1.5 * z), Align2::LEFT_CENTER, title, FontId::proportional(12.5 * z), pal.text);
        if shown.is_some_and(|s| s.node == n.id) {
            let at = header.right_center() + vec2(-(if is_output { 62.0 } else { 12.0 }) * z, 1.5 * z);
            painter.text(at, Align2::RIGHT_CENTER, theme::glyph(Icon::Eye).to_string(), FontId::proportional(11.0 * z), pal.accent);
        }
        if is_output {
            // "Active", or a ring to make this output the active one.
            let pill = Rect::from_min_max(pos2(header.right() - 58.0 * z, header.top() + 6.0 * z), pos2(header.right() - 8.0 * z, header.bottom() - 4.0 * z));
            let r = ui.interact(pill, ui.scope_id().with(("active", &n.id)), Sense::click());
            if is_active {
                painter.rect_filled(pill, theme::PILL, pal.accent);
                painter.text(pill.center(), Align2::CENTER_CENTER, t!("Active"), FontId::proportional(10.5 * z), pal.on_accent());
            } else {
                painter.rect_stroke(pill, theme::PILL, Stroke::new(1.0, if r.hovered() { pal.text } else { pal.border }), StrokeKind::Inside);
                painter.text(pill.center(), Align2::CENTER_CENTER, t!("Inactive"), FontId::proportional(10.5 * z), pal.weak);
                if r.clicked() {
                    acts.push(Act::SetActive(n.id.clone()));
                }
            }
        }
    }

    let node = &p.graph.nodes[&n.id];
    let reach = 10.0 * z.max(0.7);
    for (row, rr) in &n.rows {
        match row {
            Row::Output(k) => {
                let o = n.spec.outputs[*k];
                let at = pos2(rect.right(), rr.center().y);
                if text_on {
                    painter.text(pos2(rect.right() - 12.0 * z, at.y), Align2::RIGHT_CENTER, super::text::name(o.label), FontId::proportional(11.5 * z), pal.weak);
                }
                sockets::draw(painter, at, SOCKET_R * z.max(0.7), o.kind, sockets::shape(o.kind), true, pal);
                let s = ui.interact(Rect::from_center_size(at, vec2(reach, reach) * 2.0), ui.scope_id().with(("out", &n.id, *k)), Sense::drag());
                if s.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    s.clone().on_hover_text(format!("{} · {}", super::text::name(o.label), sockets::kind_label(o.kind)));
                }
                if s.drag_started() {
                    acts.push(Act::StartWire(n.id.clone(), *k as u8));
                }
            }
            Row::Thumb => {
                let t = Rect::from_min_size(pos2(rect.left() + PAD * z, rr.top() + 2.0 * z), vec2(rect.width() - 2.0 * PAD * z, THUMB_H * z));
                let well_r = (theme::TILE_RADIUS as f32 - 2.0) * z;
                painter.rect_filled(t, well_r, pal.well());
                let key = super::eval::Shown { node: n.id.clone(), port: None };
                if let Some(msg) = broken {
                    if text_on {
                        let g = painter.layout(msg.clone(), FontId::proportional(10.5 * z), pal.danger, t.width() - 12.0 * z);
                        painter.galley(t.min + vec2(6.0, 6.0) * z, g, pal.danger);
                    }
                } else if let Some((tex, pic)) = p.thumbs.get(&key) {
                    // Cover: the square picture cut to the frame's shape.
                    let aspect = t.width() / t.height();
                    let (uw, uh) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uw, uh));
                    egui::Image::new((tex.id(), t.size())).uv(uv).corner_radius(well_r).paint_at(ui, t);
                    if text_on && z >= 0.8 && let Some(kind) = pic.kind {
                        let range = match kind {
                            PortKind::Height => format!("{:.0}–{:.0} b", pic.range.0, pic.range.1),
                            PortKind::Mask => format!("{:.2}–{:.2}", pic.range.0, pic.range.1),
                            _ => String::new(),
                        };
                        if !range.is_empty() {
                            let g = painter.layout_no_wrap(range, FontId::proportional(9.5 * z), Color32::WHITE);
                            let chip = Rect::from_min_size(t.left_bottom() + vec2(4.0, -4.0 - g.size().y - 4.0) * 1.0, g.size() + vec2(8.0, 4.0));
                            painter.rect_filled(chip, theme::PILL, Color32::from_black_alpha(140));
                            painter.galley(chip.min + vec2(4.0, 2.0), g, Color32::WHITE);
                        }
                    }
                } else if text_on {
                    painter.text(t.center(), Align2::CENTER_CENTER, "…", FontId::proportional(14.0 * z), pal.weak);
                }
                if is_output && let Some(tex) = p.thumbs.get(&key) {
                    let _ = tex;
                }
            }
            Row::Group { key, label, open, used } => {
                let r = ui.interact(*rr, ui.scope_id().with(("group", &n.id, *key)), Sense::click());
                if text_on {
                    let chevron = if *open { Icon::ChevronDown } else { Icon::ChevronRight };
                    let ink = if r.hovered() { pal.text } else { pal.weak };
                    painter.text(pos2(rect.left() + 12.0 * z, rr.center().y), Align2::LEFT_CENTER, theme::glyph(chevron).to_string(), FontId::proportional(10.0 * z), ink);
                    painter.text(pos2(rect.left() + 26.0 * z, rr.center().y), Align2::LEFT_CENTER, t_upper(label), FontId::proportional(10.0 * z), ink);
                    if *used > 0 && !*open {
                        painter.text(pos2(rect.right() - 12.0 * z, rr.center().y), Align2::RIGHT_CENTER, used.to_string(), FontId::proportional(10.0 * z), pal.weak);
                    }
                }
                if r.clicked() {
                    acts.push(Act::ToggleGroup(n.id.clone(), key));
                }
            }
            Row::AddMask => {
                let r = ui.interact(rr.shrink2(vec2(10.0 * z, 1.0)), ui.scope_id().with(("add mask", &n.id)), Sense::click());
                if text_on {
                    let ink = if r.hovered() { pal.accent } else { pal.weak };
                    painter.text(pos2(rect.left() + 14.0 * z, rr.center().y), Align2::LEFT_CENTER, format!("{}  {}", theme::glyph(Icon::Plus), t!("Mask")), FontId::proportional(11.5 * z), ink);
                }
                if r.clicked() {
                    acts.push(Act::AddMask(n.id.clone()));
                }
            }
            Row::Input { name, label, kind, form, default } => {
                let at = pos2(rect.left(), rr.center().y);
                let src = node.inputs.get(name);
                let connected = src.is_some();
                let target = drop.is_some_and(|d| d.0 == n.id && d.1 == *name);
                if target {
                    painter.rect_filled(rr.shrink2(vec2(2.0, 1.0)), 4.0, pal.accent.gamma_multiply(0.15));
                }
                sockets::draw(painter, at, SOCKET_R * z.max(0.7), *kind, *form, connected, pal);
                if text_on {
                    let ink = if connected { pal.text } else { pal.weak };
                    let lx = rect.left() + 12.0 * z;
                    painter.text(pos2(lx, at.y), Align2::LEFT_CENTER, label, FontId::proportional(11.5 * z), ink);
                    if is_output {
                        // What arrives: a swatch of the source's picture,
                        // or, empty, what stands for it.
                        let right = rect.right() - 10.0 * z;
                        if let Some(src) = src {
                            let s = Source::parse(src);
                            let sw = Rect::from_min_size(pos2(right - 16.0 * z, at.y - 7.0 * z), vec2(16.0, 14.0) * z);
                            match p.thumbs.get(&super::eval::Shown { node: s.node.to_owned(), port: None }) {
                                Some((tex, _)) => {
                                    egui::Image::new((tex.id(), sw.size())).corner_radius(3.0 * z).paint_at(ui, sw);
                                }
                                None => {
                                    painter.rect_filled(sw, 3.0 * z, sockets::color_on(*kind, pal).gamma_multiply(0.5));
                                }
                            }
                            let name_x = sw.left() - 6.0 * z;
                            let label_w = painter.layout_no_wrap(label.clone(), FontId::proportional(11.5 * z), ink).size().x;
                            let room = Rect::from_min_max(pos2(lx + label_w + 8.0 * z, rr.top()), pos2(name_x, rr.bottom()));
                            painter.with_clip_rect(room.intersect(painter.clip_rect())).text(pos2(name_x, at.y), Align2::RIGHT_CENTER, s.node, FontId::proportional(10.5 * z), pal.weak);
                        } else if !default.is_empty() {
                            let label_w = painter.layout_no_wrap(label.clone(), FontId::proportional(11.5 * z), ink).size().x;
                            let room = Rect::from_min_max(pos2(lx + label_w + 10.0 * z, rr.top()), pos2(right, rr.bottom()));
                            let mut job = egui::text::LayoutJob::simple_singleline(super::text::name(default).to_owned(), FontId::proportional(10.5 * z), theme::mix(pal.weak, pal.panel, 0.25));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room.width().max(4.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(right - g.size().x, at.y - g.size().y / 2.0), g, pal.weak);
                        }
                    }
                }
                // A named mask of one's own: a double-click on its name
                // renames it.
                if is_output && n.spec.input(name).is_none() {
                    let label_r = Rect::from_min_max(pos2(rect.left() + 10.0 * z, rr.top()), pos2(rect.center().x, rr.bottom()));
                    let r = ui.interact(label_r, ui.scope_id().with(("mask name", &n.id, name)), Sense::click());
                    if r.double_clicked() {
                        acts.push(Act::RenameMask(n.id.clone(), name.clone()));
                    }
                }
                let hot = Rect::from_min_max(pos2(rect.left() - reach, rr.top()), pos2(rect.left() + reach, rr.bottom()));
                let s = ui.interact(hot, ui.scope_id().with(("in", &n.id, name)), Sense::drag());
                if s.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    let what = if default.is_empty() { sockets::kind_label(*kind).to_owned() } else { format!("{} · {}: {}", sockets::kind_label(*kind), t!("empty"), super::text::name(default)) };
                    s.clone().on_hover_text(what);
                }
                if s.drag_started() && connected {
                    acts.push(Act::Detach(n.id.clone(), name.clone()));
                }
            }
        }
    }
    if p.collapsed(&n.id) {
        // One socket of each side on the header, for the wires.
        if let Some(o) = n.spec.outputs.first() {
            sockets::draw(painter, rect.right_center(), SOCKET_R * z.max(0.7), o.kind, sockets::shape(o.kind), true, pal);
        }
        if !node.inputs.is_empty() {
            painter.circle_filled(rect.left_center(), 3.0 * z.max(0.7), pal.weak);
        }
    }
}

fn t_upper(s: &str) -> String {
    super::text::name(match s {
        "Ground" => "Ground",
        "Surface" => "Surface",
        "Climate" => "Climate",
        "Life" => "Life",
        "Underground" => "Underground",
        "Rock over the void" => "Rock over the void",
        "Masks" => "Masks",
        "Other" => "Other",
        _ => "",
    })
    .to_uppercase()
}

/// A node's title: its type's name, and the choice that makes it what it
/// is (Blend · Add), or its own name when renamed from the type's.
pub fn node_title(p: &TerrainPanel, id: &str, spec: &NodeSpec) -> String {
    let base = super::text::name(spec.label).to_owned();
    let node = &p.graph.nodes[id];
    let choice = spec.params.iter().find(|q| matches!(q.kind, ParamKind::Choice { .. }) && q.section == registry::Section::Main);
    let mut title = base;
    if let Some(q) = choice
        && let ParamKind::Choice { options, default } = q.kind
    {
        let v = node.params.get(q.name).and_then(|v| v.as_str()).unwrap_or(default);
        if let Some(o) = options.iter().find(|o| o.value == v) {
            title = format!("{title} · {}", super::text::name(o.label));
        }
    }
    let stem = spec.id.rsplit('.').next().unwrap_or("");
    let auto = id == stem || id.strip_prefix(stem).is_some_and(|r| r.starts_with('_') && r[1..].chars().all(|c| c.is_ascii_digit()));
    if !auto {
        title = format!("{id} · {title}");
    }
    title
}

fn apply(p: &mut TerrainPanel, acts: Vec<Act>, origin: Pos2) {
    let _ = origin;
    for a in acts {
        match a {
            Act::Select(id, add) => {
                if add {
                    if let Some(k) = p.selected.iter().position(|s| *s == id) {
                        p.selected.remove(k);
                    } else {
                        p.selected.push(id);
                    }
                } else if !p.selected.contains(&id) || p.selected.len() > 1 && p.selected.last() != Some(&id) {
                    if !p.selected.contains(&id) {
                        p.selected = vec![id];
                    } else {
                        p.selected.retain(|s| *s != id);
                        p.selected.push(id);
                    }
                }
                p.search = None;
            }
            Act::Move(d) => {
                if d != Vec2::ZERO {
                    let ids = p.selected.clone();
                    let g = p.edit();
                    for id in ids {
                        if let Some(u) = g.ui.get_mut(&id) {
                            u.pos[0] += d.x;
                            u.pos[1] += d.y;
                        }
                    }
                }
            }
            Act::View(id) => {
                p.viewed = Some(super::eval::Shown { node: id, port: None });
            }
            Act::ToggleCollapse(id) => {
                let g = p.edit();
                if let Some(u) = g.ui.get_mut(&id) {
                    u.collapsed = !u.collapsed;
                }
            }
            Act::StartWire(node, port) => p.wiring = Some(Wiring::From { node, port }),
            Act::Detach(to, input) => {
                if let Some((node, port)) = p.source_of(&to, &input) {
                    p.disconnect(&to, &input);
                    p.wiring = Some(Wiring::From { node, port });
                }
            }
            Act::Connect(node, port, to, input) => {
                p.connect(&node, port, &to, &input);
                if p.spec(&to).is_some_and(|s| s.is_output()) && input.starts_with("mask.") {
                    p.layout.exposed.remove(&format!("{to}.{input}"));
                }
            }
            Act::Cut(to, input) => p.disconnect(&to, &input),
            Act::AddReroute(key, seg, at) => {
                let v = p.edit_layout().reroutes.entry(key).or_default();
                let k = seg.min(v.len());
                v.insert(k, at);
            }
            Act::MoveReroute(key, j, at) => {
                if let Some(v) = p.edit_layout().reroutes.get_mut(&key)
                    && let Some(q) = v.get_mut(j)
                {
                    *q = at;
                }
            }
            Act::RemoveReroute(key, j) => {
                let l = p.edit_layout();
                if let Some(v) = l.reroutes.get_mut(&key) {
                    if j < v.len() {
                        v.remove(j);
                    }
                    if v.is_empty() {
                        l.reroutes.remove(&key);
                    }
                }
            }
            Act::OpenSearch(at, takes, wire) => {
                let g = to_graph(p, origin, at);
                p.search = Some(Search { at: g, screen: at, query: String::new(), takes, wire, hot: 0 });
            }
            Act::MoveFrame(k, d) => {
                let Some(f) = p.layout.frames.get(k).cloned() else { continue };
                let inside: Vec<String> = p.graph.nodes.keys().filter(|id| {
                    let pos = p.pos(id);
                    pos[0] >= f.rect[0] && pos[1] >= f.rect[1] && pos[0] <= f.rect[0] + f.rect[2] && pos[1] <= f.rect[1] + f.rect[3]
                }).cloned().collect();
                let g = p.edit();
                for id in inside {
                    if let Some(u) = g.ui.get_mut(&id) {
                        u.pos[0] += d.x;
                        u.pos[1] += d.y;
                    }
                }
                let fr = &mut p.layout.frames[k];
                fr.rect[0] += d.x;
                fr.rect[1] += d.y;
            }
            Act::ResizeFrame(k, d) => {
                if let Some(f) = p.edit_layout().frames.get_mut(k) {
                    f.rect[2] = (f.rect[2] + d.x).max(120.0);
                    f.rect[3] = (f.rect[3] + d.y).max(60.0);
                }
            }
            Act::RemoveFrame(k) => {
                let l = p.edit_layout();
                if k < l.frames.len() {
                    l.frames.remove(k);
                }
            }
            Act::ToggleGroup(id, key) => {
                let k = format!("{id}.{key}");
                let l = &mut p.layout;
                if !l.folded.remove(&k) {
                    l.folded.insert(k);
                }
            }
            Act::AddMask(id) => {
                let taken: Vec<String> = p.graph.nodes.get(&id).map(|n| n.inputs.keys().cloned().collect()).unwrap_or_default();
                let mut k = 1;
                let name = loop {
                    let n = if k == 1 { "mask.custom".to_owned() } else { format!("mask.custom_{k}") };
                    if !taken.contains(&n) && !p.layout.exposed.contains(&format!("{id}.{n}")) {
                        break n;
                    }
                    k += 1;
                };
                p.edit_layout().exposed.insert(format!("{id}.{name}"));
                p.renaming_mask = Some((id, name));
            }
            Act::RenameMask(id, name) => p.renaming_mask = Some((id, name)),
            Act::SetActive(id) => p.set_active_output(&id),
        }
    }
}

fn header_commands(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let icon_button = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| -> bool {
        let r = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(theme::glyph(icon).to_string()).size(13.0)).frame(false).min_size(vec2(24.0, 24.0)));
        r.on_hover_text(tip).clicked()
    };
    if icon_button(ui, Icon::Maximize2, t!("Frame all nodes (F)"), !p.graph.nodes.is_empty()) {
        let rect = ui.max_rect();
        let canvas = Rect::from_min_max(pos2(rect.left() - 2000.0, rect.bottom()), pos2(rect.right(), rect.bottom() + 600.0));
        let _ = canvas;
        p.view.zoom = 1.0;
        p.view.offset = vec2(40.0, 40.0);
        let (lo, _) = bounds(p);
        p.view.offset = vec2(40.0 - lo[0], 30.0 - lo[1]);
    }
    if icon_button(ui, Icon::Frame, t!("Frame the selected nodes (⌘J)"), !p.selected.is_empty()) {
        frame_selection(p);
    }
    if icon_button(ui, Icon::RotateCw, t!("Redo"), p.can_redo()) {
        p.redo();
    }
    if icon_button(ui, Icon::RotateCcw, t!("Undo"), p.can_undo()) {
        p.undo();
    }
    if icon_button(ui, Icon::Plus, t!("Add a node (Tab)"), true) {
        let at = ui.max_rect().left_bottom() + vec2(60.0, 60.0);
        p.search = Some(Search { at: [40.0 - p.view.offset.x, 40.0 - p.view.offset.y], screen: at, ..Default::default() });
    }
    let _ = pal;
}

fn bounds(p: &TerrainPanel) -> ([f32; 2], [f32; 2]) {
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for id in p.graph.nodes.keys() {
        let pos = p.pos(id);
        let s = node_size(p, id);
        lo = [lo[0].min(pos[0]), lo[1].min(pos[1])];
        hi = [hi[0].max(pos[0] + s.x), hi[1].max(pos[1] + s.y)];
    }
    for f in &p.layout.frames {
        lo = [lo[0].min(f.rect[0]), lo[1].min(f.rect[1])];
        hi = [hi[0].max(f.rect[0] + f.rect[2]), hi[1].max(f.rect[1] + f.rect[3])];
    }
    if lo[0] > hi[0] {
        return ([0.0; 2], [1.0; 2]);
    }
    (lo, hi)
}

/// Zooms and moves the view so that every node shows in `canvas`.
pub fn fit_view(p: &mut TerrainPanel, canvas: Rect) {
    if p.graph.nodes.is_empty() {
        return;
    }
    let (lo, hi) = bounds(p);
    let (w, h) = (hi[0] - lo[0] + 60.0, hi[1] - lo[1] + 60.0);
    // Not so small that the nodes' words cannot be read: past that, the
    // graph's end (where its Map Output is) shows, and the minimap the rest.
    p.view.zoom = (canvas.width() / w).min(canvas.height() / h).clamp(0.7, 1.15);
    let size = vec2(hi[0] - lo[0], hi[1] - lo[1]) * p.view.zoom;
    p.view.offset = (canvas.size() - size) / 2.0 - vec2(lo[0], lo[1]) * p.view.zoom;
    if size.x > canvas.width() - 40.0 {
        p.view.offset.x = canvas.width() - 30.0 - hi[0] * p.view.zoom;
    }
    if size.y > canvas.height() - 40.0 {
        p.view.offset.y = 24.0 - lo[1] * p.view.zoom;
    }
}

/// A frame around the selected nodes.
pub fn frame_selection(p: &mut TerrainPanel) {
    if p.selected.is_empty() {
        return;
    }
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for id in &p.selected {
        let pos = p.pos(id);
        let s = node_size(p, id);
        lo = [lo[0].min(pos[0]), lo[1].min(pos[1])];
        hi = [hi[0].max(pos[0] + s.x), hi[1].max(pos[1] + s.y)];
    }
    let title = t!("Frame").to_owned();
    p.edit_layout().frames.push(Frame { title, rect: [lo[0] - 20.0, lo[1] - 44.0, hi[0] - lo[0] + 40.0, hi[1] - lo[1] + 64.0], color: None });
    p.renaming_frame = Some(p.layout.frames.len() - 1);
}

/// The whole graph small, at the canvas's bottom right: a click or a drag
/// moves the view there.
fn minimap(p: &mut TerrainPanel, ui: &mut Ui, canvas: Rect, pal: &Palette) {
    if p.graph.nodes.len() < 2 || canvas.width() < 420.0 || canvas.height() < 220.0 {
        return;
    }
    // Only when some of the graph is out of view.
    {
        let (lo, hi) = bounds(p);
        let a = pos2(lo[0], lo[1]) * p.view.zoom + p.view.offset;
        let b = pos2(hi[0], hi[1]) * p.view.zoom + p.view.offset;
        if a.x >= 0.0 && a.y >= 0.0 && b.x <= canvas.width() && b.y <= canvas.height() {
            return;
        }
    }
    let size = vec2(168.0, 104.0);
    // At the bottom right, or at the bottom left when a node is there.
    let right = Rect::from_min_size(canvas.right_bottom() - size - vec2(10.0, 10.0), size);
    let left = Rect::from_min_size(pos2(canvas.left() + 10.0, canvas.bottom() - size.y - 10.0), size);
    let covers = |r: Rect| p.graph.nodes.keys().any(|id| {
        let pos = p.pos(id);
        let at = canvas.min + p.view.offset + vec2(pos[0], pos[1]) * p.view.zoom;
        Rect::from_min_size(at, node_size(p, id) * p.view.zoom).intersects(r)
    });
    let rect = if covers(right) && !covers(left) { left } else { right };
    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect(rect, theme::TILE_RADIUS, theme::mix(pal.panel, pal.app_bg, 0.5).gamma_multiply(0.96), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let (lo, hi) = bounds(p);
    // The view's rectangle, in graph units.
    let v0 = [(-p.view.offset.x) / p.view.zoom, (-p.view.offset.y) / p.view.zoom];
    let v1 = [v0[0] + canvas.width() / p.view.zoom, v0[1] + canvas.height() / p.view.zoom];
    let (lo, hi) = ([lo[0].min(v0[0]), lo[1].min(v0[1])], [hi[0].max(v1[0]), hi[1].max(v1[1])]);
    let inner = rect.shrink(8.0);
    let s = (inner.width() / (hi[0] - lo[0]).max(1.0)).min(inner.height() / (hi[1] - lo[1]).max(1.0));
    let at = |g: [f32; 2]| inner.min + vec2((g[0] - lo[0]) * s, (g[1] - lo[1]) * s);
    for f in &p.layout.frames {
        painter.rect_stroke(Rect::from_min_max(at([f.rect[0], f.rect[1]]), at([f.rect[0] + f.rect[2], f.rect[1] + f.rect[3]])), 2.0, Stroke::new(1.0, pal.weak.gamma_multiply(0.5)), StrokeKind::Inside);
    }
    for id in p.graph.nodes.keys() {
        let pos = p.pos(id);
        let sz = node_size(p, id);
        let r = Rect::from_min_max(at(pos), at([pos[0] + sz.x, pos[1] + sz.y]));
        let family = p.spec(id).map_or(Family::Internal, |s| s.family);
        let c = sockets::family(family);
        painter.rect_filled(r, 1.5, if p.selected.contains(id) { c } else { c.gamma_multiply(0.6) });
    }
    painter.rect_stroke(Rect::from_min_max(at(v0), at(v1)), 2.0, Stroke::new(1.2, pal.accent), StrokeKind::Inside);
    let r = ui.interact(rect, ui.scope_id().with("terrain minimap"), Sense::click_and_drag());
    if (r.clicked() || r.dragged())
        && let Some(q) = r.interact_pointer_pos()
    {
        let g = [lo[0] + (q.x - inner.min.x) / s, lo[1] + (q.y - inner.min.y) / s];
        p.view.offset = canvas.size() / 2.0 - vec2(g[0], g[1]) * p.view.zoom;
    }
}

/// Node types matching a search, best first.
pub fn find(query: &str, takes: Option<PortKind>) -> Vec<&'static NodeSpec> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<(i32, &'static NodeSpec)> = registry::all()
        .iter()
        .copied()
        .filter(|s| offered(s))
        .filter(|s| takes.is_none_or(|k| s.inputs.iter().any(|i| k.conversion(i.kind).is_some()) || s.open_inputs.is_some_and(|o| k.conversion(o.kind).is_some())))
        .filter_map(|s| {
            if q.is_empty() {
                return Some((0, s));
            }
            let label = super::text::name(s.label).to_lowercase();
            let english = s.label.to_lowercase();
            let score = if label.starts_with(&q) || english.starts_with(&q) {
                100
            } else if label.split(' ').any(|w| w.starts_with(&q)) || english.split(' ').any(|w| w.starts_with(&q)) {
                80
            } else if label.contains(&q) || english.contains(&q) || s.id.contains(&q) {
                60
            } else if s.keywords.iter().any(|k| k.starts_with(&q)) {
                50
            } else if fuzzy(&q, &english) || fuzzy(&q, &label) {
                20
            } else {
                return None;
            };
            Some((score, s))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.family.cmp(&b.1.family)));
    hits.into_iter().map(|h| h.1).collect()
}

/// Whether the library and the search offer a type of node: not the
/// compiler's own, nor the nodes of 1.2 that only migrated graphs hold.
pub fn offered(s: &NodeSpec) -> bool {
    s.family != Family::Internal && !s.id.starts_with("legacy.")
}

/// Whether the letters of `q` come in order in `s`.
fn fuzzy(q: &str, s: &str) -> bool {
    let mut it = s.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|d| d == c))
}

fn search_ui(p: &mut TerrainPanel, ui: &mut Ui, canvas: Rect, pal: &Palette) {
    let Some(mut s) = p.search.take() else { return };
    let width = 300.0;
    let pos = pos2(s.screen.x.min(canvas.right() - width - 8.0).max(canvas.left() + 8.0), s.screen.y.min(canvas.bottom() - 340.0).max(canvas.top() + 8.0));
    let hits = find(&s.query, s.takes);
    let mut chosen: Option<&'static NodeSpec> = None;
    let mut close = false;
    let area = egui::Area::new(ui.scope_id().with("terrain search")).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).corner_radius(theme::POPUP_RADIUS).inner_margin(8).show(ui, |ui| {
            ui.set_width(width);
            let r = crate::widgets::search(ui, &mut s.query, t!("Search nodes…"), width);
            if ui.ctx().memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            if r.changed() {
                s.hot = 0;
            }
            let (down, up, enter, esc) = ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
            if down {
                s.hot = (s.hot + 1).min(hits.len().saturating_sub(1));
            }
            if up {
                s.hot = s.hot.saturating_sub(1);
            }
            if enter && let Some(h) = hits.get(s.hot) {
                chosen = Some(h);
            }
            if esc {
                close = true;
            }
            if let Some(k) = s.takes {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(t!("Nodes that take a {kind}", kind = sockets::kind_label(k).to_lowercase())).size(11.0).color(pal.weak));
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                let mut last_family = None;
                for (k, spec) in hits.iter().enumerate().take(60) {
                    if s.query.is_empty() && last_family != Some(spec.family) {
                        last_family = Some(spec.family);
                        ui.add_space(if k == 0 { 0.0 } else { 6.0 });
                        ui.label(egui::RichText::new(super::text::name(spec.family.label()).to_uppercase()).size(10.0).color(pal.weak));
                    }
                    let hot = k == s.hot;
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                    if hot || r.hovered() {
                        ui.painter().rect_filled(rect, theme::TILE_RADIUS, pal.accent_soft);
                    }
                    ui.painter().circle_filled(rect.left_center() + vec2(10.0, 0.0), 4.0, sockets::family(spec.family));
                    ui.painter().text(rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, super::text::name(spec.label), FontId::proportional(12.5), pal.text);
                    ui.painter().with_clip_rect(rect).text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, super::text::name(spec.family.label()), FontId::proportional(10.5), pal.weak);
                    if hot && down | up {
                        r.scroll_to_me(None);
                    }
                    let r = r.on_hover_text(super::text::name(spec.summary));
                    if r.clicked() {
                        chosen = Some(spec);
                    }
                }
                if hits.is_empty() {
                    ui.label(egui::RichText::new(t!("No node matches.")).color(pal.weak));
                }
            });
        });
    });
    let clicked_out = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
    if let Some(spec) = chosen {
        let id = p.add_node(spec.id, s.at);
        if let Some((from, port)) = s.wire.clone() {
            let kind = p.out_kind(&from, port).unwrap_or(PortKind::Height);
            let input = spec.inputs.iter().find(|i| i.kind == kind).or_else(|| spec.inputs.iter().find(|i| kind.conversion(i.kind).is_some())).map(|i| i.name.to_owned()).or_else(|| spec.open_inputs.map(|_| "mask.custom".to_owned()));
            if let Some(input) = input {
                p.connect(&from, port, &id, &input);
            }
        }
        return;
    }
    if close || clicked_out {
        return;
    }
    p.search = Some(s);
}
