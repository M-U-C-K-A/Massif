//! The graph canvas: nodes with their thumbnails and typed sockets, wires
//! (a dot where a value is converted, a refusal explained where it cannot
//! go), reroute points, frames nested in frames, a minimap, the search to
//! add a node, and the breadcrumb of the composites open.

use super::panel::{Search, TerrainPanel, Wiring};
use super::shape::{self, NodeShape, Port};
use super::sockets::{self, Shape_};
use crate::theme::{self, Icon, Palette};
use egui::{Align2, Color32, CursorIcon, FontId, Key, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use massif_terrain::model::Source;
use massif_terrain::registry::{self, Conversion, Family, NodeSpec, ParamKind, PortKind};
use std::collections::HashMap;
use std::rc::Rc;

pub const WIDTH: f32 = 176.0;
pub const OUTPUT_WIDTH: f32 = 236.0;
const HEADER: f32 = 28.0;
const ROW: f32 = 20.0;
const THUMB_H: f32 = 104.0;
const PAD: f32 = 8.0;
const SOCKET_R: f32 = 4.5;
const FRAME_BAR: f32 = 28.0;

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

/// Colours a frame can take: the families' own, then greys.
pub const FRAME_COLOURS: &[(&str, &str)] = &[
    ("#56c478", "Green"),
    ("#34bab0", "Teal"),
    ("#5c92ff", "Blue"),
    ("#a87cff", "Violet"),
    ("#ec60a4", "Pink"),
    ("#f59240", "Orange"),
    ("#ecc040", "Yellow"),
    ("#f45468", "Red"),
];

/// The group a Map Output's input shows in (an input the registry adds
/// later lands in Other until it has one).
pub fn output_group(name: &str) -> &'static str {
    match name {
        "height" | "water" => "ground",
        "albedo" | "terrain" | "biome" => "surface",
        "temperature" | "humidity" => "climate",
        "vegetation" => "life",
        "caves" | "pockets" => "underground",
        "cover" | "objects" | "understory" | "annotations" | "structures" | "roads" => "life",
        n if n.starts_with("level.") || n.starts_with("layer.") => "rock",
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
    Input { name: String, label: String, kind: PortKind, form: Shape_, default: String },
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
    default != p.layout.folded.contains(&p.lkey(&format!("{id}.{key}")))
}

fn input_row(i: &Port) -> Row {
    Row::Input { name: i.name.clone(), label: i.label.clone(), kind: i.kind, form: sockets::shape(i.kind), default: i.default.clone() }
}

/// The lines of node `id`, after its header.
pub fn rows(p: &TerrainPanel, id: &str) -> Vec<Row> {
    let Some(shape) = p.shape(id) else { return Vec::new() };
    if p.collapsed(id) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if shape.is_column() {
        out.extend((0..shape.outputs.len()).map(Row::Output));
        out.extend(shape.inputs.iter().map(input_row));
        return out;
    }
    let node = &p.graph.nodes[id];
    if let Some(spec) = shape.spec().filter(|s| s.is_output()) {
        out.push(Row::Thumb);
        for (key, label) in OUTPUT_GROUPS {
            let mut inputs: Vec<Row> = shape.inputs.iter().filter(|i| output_group(&i.name) == *key).map(input_row).collect();
            if *key == "masks"
                && let Some(open) = spec.open_inputs
            {
                for name in node.inputs.keys().filter(|k| spec.open_input(k).is_some() && spec.input(k).is_none()) {
                    let label = name.strip_prefix(open.name).unwrap_or(name);
                    inputs.push(Row::Input { name: name.clone(), label: capitalise(label), kind: PortKind::Mask, form: Shape_::Circle, default: open.default.to_owned() });
                }
                let prefix = p.lkey(&format!("{id}."));
                for name in p.layout.exposed.iter().filter_map(|e| e.strip_prefix(&prefix)).filter(|n| n.starts_with(open.name) && !node.inputs.contains_key(*n)) {
                    let label = name.strip_prefix(open.name).unwrap_or(name);
                    inputs.push(Row::Input { name: name.to_owned(), label: capitalise(label), kind: PortKind::Mask, form: Shape_::Circle, default: open.default.to_owned() });
                }
            }
            if inputs.is_empty() && *key != "masks" {
                continue;
            }
            let used = inputs.iter().filter(|r| matches!(r, Row::Input { name, .. } if node.inputs.contains_key(name))).count();
            let open = group_open(p, id, key, used);
            out.push(Row::Group { key, label, open, used });
            if open {
                out.extend(inputs);
                if *key == "masks" {
                    out.push(Row::AddMask);
                }
            }
        }
        return out;
    }
    out.extend((0..shape.outputs.len()).map(Row::Output));
    out.push(Row::Thumb);
    out.extend(shape.inputs.iter().map(input_row));
    if let Some(spec) = shape.spec() {
        for prm in spec.params.iter().filter(|q| q.drivable) {
            let input = format!("@{}", prm.name);
            if node.inputs.contains_key(&input) || p.layout.exposed.contains(&p.lkey(&format!("{id}.{}", prm.name))) {
                out.push(Row::Input { name: input, label: super::text::name(prm.label).to_owned(), kind: PortKind::Mask, form: Shape_::Diamond, default: String::new() });
            }
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
    let w = if p.shape(id).is_some_and(|s| s.is_output()) { OUTPUT_WIDTH } else { WIDTH };
    let h = HEADER + rows(p, id).iter().map(Row::height).sum::<f32>() + if p.collapsed(id) { 0.0 } else { 6.0 };
    vec2(w, h)
}

/// Where everything of a node is on screen.
struct Placed {
    id: String,
    shape: Rc<NodeShape>,
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

fn frame_screen(p: &TerrainPanel, origin: Pos2, id: &str) -> Rect {
    let r = p.frame_rect(id);
    Rect::from_min_size(to_screen(p, origin, [r[0], r[1]]), vec2(r[2], r[3]) * p.view.zoom)
}

fn place(p: &TerrainPanel, origin: Pos2) -> Vec<Placed> {
    let z = p.view.zoom;
    let mut ids = p.ids();
    // The selected drawn last, over the others.
    ids.sort_by_key(|id| p.selected.contains(id));
    ids.into_iter()
        .filter(|id| p.is_column(id) || p.hidden_in(id).is_none())
        .filter_map(|id| {
            let shape = p.shape(&id)?;
            let size = node_size(p, &id);
            let rect = Rect::from_min_size(to_screen(p, origin, p.pos(&id)), size * z);
            let mut y = rect.top() + HEADER * z;
            let rows = rows(p, &id)
                .into_iter()
                .map(|r| {
                    let h = r.height() * z;
                    let rr = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), h));
                    y += h;
                    (r, rr)
                })
                .collect();
            Some(Placed { id, shape, rect, rows })
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
    to: String,
    input: String,
    key: String,
    curves: Vec<[Pos2; 4]>,
    kind: PortKind,
    target: PortKind,
}

/// Why a value of type `from` cannot feed an input of type `to`, and the
/// node that turns one into the other when there is one.
pub fn refusal(from: PortKind, to: PortKind) -> String {
    let (a, b) = (sockets::kind_label(from), sockets::kind_label(to));
    let family = |k: PortKind| {
        if k.is_geometry() {
            t!("geometry, made whole")
        } else if k.is_volume() {
            t!("a volume, made by bricks")
        } else {
            t!("samples over the terrain")
        }
    };
    let why = if from.is_raster() != to.is_raster() || from.is_geometry() != to.is_geometry() {
        t!("{a} is {fa} and {b} is {fb}: nothing turns one into the other without a node saying how.", a = a, fa = family(from), b = b, fb = family(to))
    } else {
        t!("{a} and {b} do not mean the same thing: a node has to say how one becomes the other.", a = a, b = b)
    };
    match converter(from, to) {
        Some(spec) => format!("{}\n{}", why, t!("Put a {node} node between them.", node = super::text::name(spec.label))),
        None => why,
    }
}

/// A node of the registry that takes a `from` and makes a `to`.
pub fn converter(from: PortKind, to: PortKind) -> Option<&'static NodeSpec> {
    registry::all().iter().copied().filter(|s| offered(s) && !s.is_output()).filter(|s| s.inputs.first().is_some_and(|i| i.kind == from))
        .min_by_key(|s| if s.outputs.iter().any(|o| o.kind == to) { 0 } else { 1 })
        .filter(|s| s.outputs.iter().any(|o| o.kind.conversion(to).is_some()))
}

/// What the user did on the canvas this frame, applied once it is drawn.
enum Act {
    Select(String, bool),
    Move(Vec2),
    Dropped,
    View(String),
    Open(String),
    ToggleCollapse(String),
    StartWire(String, u8),
    Detach(String, String),
    Connect(String, u8, String, String),
    Refused(String),
    Cut(String, String),
    AddReroute(String, usize, [f32; 2]),
    MoveReroute(String, usize, [f32; 2]),
    RemoveReroute(String, usize),
    OpenSearch(Pos2, Option<PortKind>, Option<(String, u8)>),
    MoveFrame(String, Vec2),
    FrameDropped(String),
    SelectFrame(String),
    ToggleFrame(String),
    RemoveFrame(String),
    FrameColour(String, Option<String>),
    ToggleGroup(String, &'static str),
    AddMask(String),
    RenameMask(String, String),
    SetActive(String),
    Bypass(Vec<String>, bool),
    ToggleBypass,
    Group,
    Ungroup(String),
    EditCopy(String),
    Duplicate,
    Remove,
    Copy,
    Arrange,
    Align(bool),
    Distribute(bool),
    Frame,
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let full = ui.max_rect();
    let now = ui.input(|i| i.time);
    let crumbs_end = breadcrumb(p, ui, full, &pal);
    // Commands at the card's top right.
    let bar = Rect::from_min_max(pos2(crumbs_end.max(full.left() + 80.0), full.top() + 4.0), pos2(full.right() - theme::PAD, full.top() + 32.0));
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
    let read_only = p.read_only();

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

    // The dotted ground, moving with the view; inside a composite, tinted.
    if !p.outer.is_empty() {
        painter.rect_filled(canvas, 0.0, theme::mix(pal.panel, pal.accent, 0.025));
    }
    let step = super::edit::GRID * 2.0 * p.view.zoom;
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

    frames_ui(p, ui, &painter, canvas, origin, &pal, &mut acts);

    // Wires, ending on the bar of a folded frame for what it hides.
    let column_src: Option<Vec<(String, String)>> = p.scope_kind().and_then(|k| massif_terrain::lower::component(&p.graph, k)).map(|c| c.outputs.iter().map(|o| (o.name.clone(), o.from.clone())).collect());
    let mut links: Vec<(String, String, String)> = Vec::new();
    for (id, n) in &p.graph.nodes {
        for (input, src) in &n.inputs {
            links.push((id.clone(), input.clone(), src.clone()));
        }
    }
    for (name, src) in column_src.iter().flatten() {
        if !src.is_empty() {
            links.push((shape::OUT_COLUMN.to_owned(), name.clone(), src.clone()));
        }
    }
    let mut wires = Vec::new();
    for (to_id, input, src) in &links {
        let s = Source::parse(src);
        let (from_node, port) = if s.node == shape::IN_COLUMN && p.is_column(shape::IN_COLUMN) {
            let k = by_id.get(shape::IN_COLUMN).and_then(|c| c.shape.output(s.output.unwrap_or(""))).unwrap_or(0);
            (shape::IN_COLUMN, k)
        } else {
            let port = p.shape(s.node).and_then(|sh| s.output.map_or(Some(0), |o| sh.output(o))).unwrap_or(0);
            (s.node, port)
        };
        let a = match by_id.get(from_node) {
            Some(n) => n.output_at(port),
            None => p.hidden_in(from_node).map(|f| frame_screen(p, origin, &f)).map(|r| pos2(r.right(), r.top() + FRAME_BAR * zoom / 2.0)),
        };
        let b = match by_id.get(to_id.as_str()) {
            Some(n) => n.input_at(input),
            None => p.hidden_in(to_id).map(|f| frame_screen(p, origin, &f)).map(|r| pos2(r.left(), r.top() + FRAME_BAR * zoom / 2.0)),
        };
        let (Some(a), Some(b)) = (a, b) else { continue };
        if p.hidden_in(from_node).is_some() && p.hidden_in(from_node) == p.hidden_in(to_id) {
            continue;
        }
        let key = p.lkey(&format!("{to_id}.{input}"));
        let via: Vec<Pos2> = p.layout.reroutes.get(&key).map(|v| v.iter().map(|g| to_screen(p, origin, *g)).collect()).unwrap_or_default();
        let kind = p.shape(from_node).and_then(|sh| sh.outputs.get(port).map(|o| o.kind)).unwrap_or(PortKind::Height);
        let target = p.shape(to_id).and_then(|sh| sh.input_kind(input)).unwrap_or(kind);
        wires.push(Wire { from: from_node.to_owned(), to: to_id.clone(), input: input.clone(), key, curves: wire_points(a, b, &via, zoom), kind, target });
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
        let bypassed = p.graph.nodes.get(&w.from).is_some_and(|n| n.bypass);
        let color = if conv.is_none() {
            pal.danger
        } else if hot {
            theme::mix(base, pal.text, 0.35)
        } else if near || p.selected.is_empty() {
            base
        } else {
            base.gamma_multiply(0.55)
        };
        let color = if bypassed { color.gamma_multiply(0.45) } else { color };
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
            painter.circle(at, 3.6 * zoom.max(0.7), sockets::color_on(w.target, &pal), Stroke::new(1.5, base));
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
        } else if w.kind.conversion(w.target).is_none() {
            background.clone().on_hover_text(refusal(w.kind, w.target));
        } else {
            background.clone().on_hover_text(t!("Double-click: add a reroute point. Right-click: cut the wire."));
        }
    }

    // The wire being drawn, where it would plug in, and why an input
    // refuses it.
    let mut drop_target: Option<(String, String)> = None;
    let mut refused_at: Option<(String, String, PortKind)> = None;
    if let (Some(Wiring::From { node, port }), Some(q)) = (&p.wiring, pointer)
        && let Some(from) = by_id.get(node.as_str())
        && let Some(a) = from.output_at(*port as usize)
    {
        let kind = from.shape.outputs.get(*port as usize).map_or(PortKind::Height, |o| o.kind);
        let mut best: Option<((String, String), f32)> = None;
        for n in placed.iter().filter(|n| n.id != *node) {
            for (r, rect) in &n.rows {
                let Row::Input { name, kind: k, .. } = r else { continue };
                let sp = pos2(n.rect.left(), rect.center().y);
                let on_row = Rect::from_min_max(pos2(n.rect.left() - 12.0, rect.top()), pos2(n.rect.center().x, rect.bottom())).contains(q);
                let d = q.distance(sp);
                if !(on_row || d < 16.0) {
                    continue;
                }
                if kind.conversion(*k).is_none() && !p.is_column(&n.id) {
                    refused_at = Some((n.id.clone(), name.clone(), *k));
                    continue;
                }
                if best.as_ref().is_none_or(|b| d < b.1) {
                    best = Some(((n.id.clone(), name.clone()), d));
                }
            }
        }
        if best.is_some() {
            refused_at = None;
        }
        let end = best.as_ref().and_then(|b| by_id.get(b.0.0.as_str()).and_then(|n| n.input_at(&b.0.1))).unwrap_or(q);
        drop_target = best.map(|b| b.0);
        let ink = if refused_at.is_some() { pal.danger } else { sockets::color_on(kind, &pal) };
        for c in wire_points(a, end, &[], zoom) {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(c, false, Color32::TRANSPARENT, Stroke::new(2.2 * zoom.max(0.6), ink)));
        }
        if let Some((_, _, to)) = &refused_at {
            explain(ui, q, &refusal(kind, *to), &pal);
        }
        if ui.input(|i| i.pointer.any_released()) {
            match (drop_target.clone(), &refused_at) {
                (Some((to, input)), _) => acts.push(Act::Connect(node.clone(), *port, to, input)),
                (None, Some((_, _, to))) => acts.push(Act::Refused(refusal(kind, *to))),
                (None, None) if canvas.contains(q) && !read_only => acts.push(Act::OpenSearch(q, Some(kind), Some((node.clone(), *port)))),
                _ => {}
            }
            p.wiring = None;
        }
    }

    // Nodes.
    let status = p.status();
    let crumbs = p.crumbs();
    let viewed = p.viewed.as_ref().filter(|v| v.0 == crumbs).map(|v| v.1.node.clone());
    let active = if crumbs.is_empty() { p.active_output() } else { None };
    let eye = viewed.or_else(|| if p.viewed.is_none() { active.clone() } else { None });
    for n in &placed {
        if !n.rect.intersects(canvas) {
            continue;
        }
        draw_node(p, ui, &painter, n, &pal, &status, eye.as_deref(), active.as_deref(), drop_target.as_ref(), refused_at.as_ref().map(|r| (r.0.as_str(), r.1.as_str())), &mut acts);
    }

    // A box to select nodes with, dragged on the ground.
    if background.drag_started_by(egui::PointerButton::Primary) && p.wiring.is_none() && hot_wire.is_none() && !ui.input(|i| i.key_down(Key::Space)) {
        p.marquee = ui.input(|i| i.pointer.press_origin());
        if !(modifiers.shift || modifiers.command) {
            p.selected.clear();
        }
    }
    if let (Some(start), Some(now_at)) = (p.marquee, pointer) {
        let area = Rect::from_two_pos(start, now_at);
        painter.rect(area, 2, pal.accent.gamma_multiply(0.08), Stroke::new(1.0, pal.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        if !ui.input(|i| i.pointer.primary_down()) {
            let hit: Vec<String> = placed.iter().filter(|n| !p.is_column(&n.id) && area.intersects(n.rect) && !p.selected.contains(&n.id)).map(|n| n.id.clone()).collect();
            p.selected.extend(hit);
            p.marquee = None;
        }
    }
    if background.clicked() && hot_wire.is_none() {
        p.selected.clear();
        p.selected_frame = None;
        p.search = None;
    }
    if background.secondary_clicked()
        && hot_wire.is_none()
        && !read_only
        && let Some(q) = background.interact_pointer_pos()
    {
        acts.push(Act::OpenSearch(q, None, None));
    }

    if p.graph.nodes.is_empty() {
        let text = if p.outer.is_empty() { t!("Right-click or press Tab to add a node, or pick a quick start in the library.") } else { t!("An empty composite: add nodes, and wire them from its inputs to its outputs.") };
        painter.text(canvas.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), pal.weak);
    }

    // Keys, while the pointer is over the canvas and nothing is typed in.
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if hovered && !typing && p.search.is_none() {
        keys(p, ui, pointer, canvas, &mut acts);
    }

    apply(p, ui, acts, origin, now);
    rename_mask_ui(p, ui, origin);
    rename_frame_ui(p, ui, origin);
    minimap(p, ui, canvas, &pal);
    notice_ui(p, ui, canvas, now, &pal);
    search_ui(p, ui, canvas, &pal);
}

/// The keys of the canvas.
fn keys(p: &mut TerrainPanel, ui: &mut Ui, pointer: Option<Pos2>, canvas: Rect, acts: &mut Vec<Act>) {
    let (del, dup, all, undo, redo, tab, fit, view, fold, frame, group, ungroup, bypass, esc) = ui.input_mut(|i| {
        (
            i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) || i.key_pressed(Key::X) && !i.modifiers.command,
            i.consume_key(egui::Modifiers::COMMAND, Key::D),
            i.consume_key(egui::Modifiers::COMMAND, Key::A),
            i.consume_key(egui::Modifiers::COMMAND, Key::Z),
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z) || i.consume_key(egui::Modifiers::COMMAND, Key::Y),
            i.key_pressed(Key::Tab) || (i.modifiers.shift && i.key_pressed(Key::A)),
            i.key_pressed(Key::F) || i.key_pressed(Key::Home),
            i.key_pressed(Key::V) && !i.modifiers.command,
            i.key_pressed(Key::H),
            i.consume_key(egui::Modifiers::COMMAND, Key::J),
            i.consume_key(egui::Modifiers::COMMAND, Key::G),
            i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::G),
            i.key_pressed(Key::B) && !i.modifiers.command,
            i.key_pressed(Key::Escape),
        )
    });
    // ⌘C, ⌘X, ⌘V come as the clipboard's events.
    let (copy, cut, paste) = ui.input(|i| {
        let mut out = (false, false, None);
        for e in &i.events {
            match e {
                egui::Event::Copy => out.0 = true,
                egui::Event::Cut => out.1 = true,
                egui::Event::Paste(t) => out.2 = Some(t.clone()),
                _ => {}
            }
        }
        out
    });
    if del {
        acts.push(Act::Remove);
    }
    if dup {
        acts.push(Act::Duplicate);
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
        acts.push(Act::Frame);
    }
    if ungroup {
        if let Some(id) = p.selected.last().cloned() {
            acts.push(Act::Ungroup(id));
        }
    } else if group && !p.selected.is_empty() {
        acts.push(Act::Group);
    }
    if bypass {
        acts.push(Act::ToggleBypass);
    }
    if esc && p.wiring.is_none() {
        if !p.selected.is_empty() {
            p.selected.clear();
        } else {
            p.close();
        }
    }
    if copy || cut {
        acts.push(Act::Copy);
    }
    if cut {
        acts.push(Act::Remove);
    }
    if let Some(text) = paste {
        let at = pointer.map(|q| to_graph(p, canvas.min, q));
        if !p.paste(&text, at) {
            p.say(t!("The clipboard holds no nodes."), 0.0);
        }
    }
}

/// A box of words beside the pointer: why something cannot be done.
fn explain(ui: &Ui, at: Pos2, text: &str, pal: &Palette) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("terrain refusal")));
    let g = painter.layout(text.to_owned(), FontId::proportional(11.5), pal.text, 280.0);
    let r = Rect::from_min_size(at + vec2(16.0, 14.0), g.size() + vec2(16.0, 12.0));
    painter.rect(r, theme::TILE_RADIUS, pal.panel, Stroke::new(1.0, pal.danger), StrokeKind::Inside);
    painter.galley(r.min + vec2(8.0, 6.0), g, pal.text);
}

/// What the canvas has to say, for four seconds at its foot.
fn notice_ui(p: &mut TerrainPanel, ui: &Ui, canvas: Rect, now: f64, pal: &Palette) {
    let Some((text, since)) = p.notice.clone() else { return };
    let since = if since == 0.0 {
        p.notice = Some((text.clone(), now));
        now
    } else {
        since
    };
    if now - since > 4.0 {
        p.notice = None;
        return;
    }
    let painter = ui.painter_at(canvas);
    let g = painter.layout(text, FontId::proportional(12.0), pal.text, (canvas.width() - 80.0).min(520.0));
    let r = Rect::from_center_size(pos2(canvas.center().x, canvas.bottom() - 26.0 - g.size().y / 2.0), g.size() + vec2(24.0, 14.0));
    painter.rect(r, theme::TILE_RADIUS, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    painter.galley(r.min + vec2(12.0, 7.0), g, pal.text);
    if !theme::reduce_motion(ui.ctx()) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    } else {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((4.0 - (now - since)).max(0.1)));
    }
}

/// The breadcrumb of the composites open: Graph › Mountain › …; a click
/// goes back up. Gives where it ends on the right.
fn breadcrumb(p: &mut TerrainPanel, ui: &mut Ui, full: Rect, pal: &Palette) -> f32 {
    let y = full.top() + 18.0;
    let mut x = full.left() + theme::PAD;
    if p.outer.is_empty() {
        super::panel::title(ui, full, t!("Graph"));
        return x + 60.0;
    }
    let mut names: Vec<String> = vec![t!("Graph").to_owned()];
    for (k, o) in p.outer.iter().enumerate() {
        let scope = if k == 0 { &o.graph } else { &p.outer[k].graph };
        let name = scope.nodes.get(&o.crumb.node).and_then(|n| n.name.clone()).unwrap_or_else(|| massif_terrain::lower::component(&o.graph, &o.crumb.kind).map_or_else(|| o.crumb.kind.clone(), |c| super::text::name_owned(&c.label)));
        names.push(name);
    }
    let last = names.len() - 1;
    let mut go = None;
    for (k, name) in names.iter().enumerate() {
        if k > 0 {
            let g = ui.painter().layout_no_wrap(theme::glyph(Icon::ChevronRight).to_string(), FontId::proportional(11.0), pal.weak);
            ui.painter().galley(pos2(x, y - g.size().y / 2.0), g.clone(), pal.weak);
            x += g.size().x + 6.0;
        }
        let ink = if k == last { pal.text } else { pal.weak };
        let g = ui.painter().layout_no_wrap(name.clone(), FontId::proportional(13.0), ink);
        let r = Rect::from_min_size(pos2(x, y - g.size().y / 2.0), g.size());
        let resp = ui.interact(r.expand(3.0), ui.scope_id().with(("crumb", k)), if k == last { Sense::hover() } else { Sense::click() });
        let ink = if resp.hovered() && k != last { pal.accent } else { ink };
        ui.painter().galley(r.min, g, ink);
        if resp.clicked() {
            go = Some(k);
        }
        x = r.right() + 6.0;
    }
    if p.read_only() {
        let g = ui.painter().layout_no_wrap(format!("{}  {}", theme::glyph(Icon::Lock), t!("Read only")), FontId::proportional(11.0), pal.weak);
        ui.painter().galley(pos2(x + 4.0, y - g.size().y / 2.0), g.clone(), pal.weak);
        x += g.size().x + 10.0;
        let b = Rect::from_min_size(pos2(x, y - 11.0), vec2(ui.painter().layout_no_wrap(t!("Edit a copy").to_owned(), FontId::proportional(12.0), pal.text).size().x + 28.0, 22.0));
        let r = ui.put(b, egui::Button::new(egui::RichText::new(t!("Edit a copy")).size(12.0)).wrap_mode(egui::TextWrapMode::Extend).corner_radius(theme::PILL));
        if r.on_hover_text(t!("Make this composite the document's own, under a new name, to change it")).clicked() {
            match p.edit_open_copy() {
                Ok(_) => p.say(t!("A copy of the composite is the document's own now."), 0.0),
                Err(e) => p.say(e, 0.0),
            }
        }
        x = b.right() + 8.0;
    }
    if let Some(k) = go {
        p.close_to(k);
    }
    x
}

/// Frames, behind the nodes: the outermost first.
fn frames_ui(p: &mut TerrainPanel, ui: &mut Ui, painter: &Painter, canvas: Rect, origin: Pos2, pal: &Palette, acts: &mut Vec<Act>) {
    let zoom = p.view.zoom;
    for id in p.frames_in_order() {
        if p.frame_hidden(&id) {
            continue;
        }
        let f = p.graph.frames[&id].clone();
        let r = frame_screen(p, origin, &id);
        if !r.intersects(canvas) {
            continue;
        }
        let tint = f.color.as_deref().and_then(massif_terrain::params::parse_color).map(|c| Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)).unwrap_or(pal.weak);
        let tint = if pal.high { theme::legible(tint, pal.panel, 3.0) } else { tint };
        let picked = p.selected_frame.as_deref() == Some(id.as_str());
        let round = (theme::GROUP_RADIUS as f32 * zoom).clamp(3.0, theme::GROUP_RADIUS as f32);
        let fill = tint.gamma_multiply(if f.collapsed { 0.18 } else if pal.dark { 0.07 } else { 0.06 });
        painter.rect(r, round, fill, Stroke::new(if picked { 1.6 } else { 1.0 }, if picked { pal.accent } else { tint.gamma_multiply(0.5) }), StrokeKind::Inside);
        let bar = Rect::from_min_size(r.min, vec2(r.width(), FRAME_BAR * zoom));
        // Fold chevron, title, the number of nodes when folded.
        let cr = Rect::from_center_size(bar.left_center() + vec2(12.0 * zoom, 0.0), vec2(14.0, 14.0) * zoom.max(0.8));
        let chevron = ui.interact(cr, ui.scope_id().with(("frame fold", &id)), Sense::click());
        if zoom >= 0.45 {
            painter.text(cr.center(), Align2::CENTER_CENTER, theme::glyph(if f.collapsed { Icon::ChevronRight } else { Icon::ChevronDown }).to_string(), FontId::proportional(11.0 * zoom), if chevron.hovered() { pal.text } else { tint });
            if p.renaming_frame.as_deref() != Some(id.as_str()) {
                let title = if f.collapsed { format!("{}  ·  {}", f.title, p.frame_nodes(&id).len()) } else { f.title.clone() };
                painter.with_clip_rect(bar.intersect(canvas)).text(bar.left_center() + vec2(24.0 * zoom, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(12.5 * zoom), theme::mix(tint, pal.text, 0.35));
            }
        }
        if chevron.clicked() {
            acts.push(Act::ToggleFrame(id.clone()));
        }
        let handle = ui.interact(Rect::from_min_max(pos2(cr.right(), bar.top()), bar.max), ui.scope_id().with(("frame", &id)), Sense::click_and_drag());
        if handle.dragged_by(egui::PointerButton::Primary) && !p.read_only() {
            acts.push(Act::MoveFrame(id.clone(), handle.drag_delta() / zoom));
        }
        if handle.drag_stopped() {
            acts.push(Act::FrameDropped(id.clone()));
        }
        if handle.clicked() || handle.drag_started() {
            acts.push(Act::SelectFrame(id.clone()));
        }
        if handle.double_clicked() && !p.read_only() {
            p.renaming_frame = Some(id.clone());
        }
        if !f.description.is_empty() {
            handle.clone().on_hover_text(&f.description);
        }
        if !p.read_only() {
            handle.context_menu(|ui| {
                if ui.button(t!("Rename frame")).clicked() {
                    p.renaming_frame = Some(id.clone());
                    ui.close();
                }
                if ui.button(if f.collapsed { t!("Unfold") } else { t!("Fold") }).clicked() {
                    acts.push(Act::ToggleFrame(id.clone()));
                    ui.close();
                }
                ui.menu_button(t!("Colour"), |ui| {
                    if ui.button(t!("Grey")).clicked() {
                        acts.push(Act::FrameColour(id.clone(), None));
                        ui.close();
                    }
                    for (hex, name) in FRAME_COLOURS {
                        let c = massif_terrain::params::parse_color(hex).unwrap_or([0.5; 3]);
                        let swatch = Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
                        if ui.button(egui::RichText::new(format!("●  {}", super::text::name(name))).color(swatch)).clicked() {
                            acts.push(Act::FrameColour(id.clone(), Some((*hex).to_owned())));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if ui.button(t!("Remove frame")).clicked() {
                    acts.push(Act::RemoveFrame(id.clone()));
                    ui.close();
                }
            });
        }
    }
}

/// The field over a frame's title being renamed.
fn rename_frame_ui(p: &mut TerrainPanel, ui: &mut Ui, origin: Pos2) {
    let Some(id) = p.renaming_frame.clone() else { return };
    let Some(f) = p.graph.frames.get(&id).cloned() else {
        p.renaming_frame = None;
        return;
    };
    let zoom = p.view.zoom;
    let r = frame_screen(p, origin, &id);
    let field = Rect::from_min_size(r.min + vec2(22.0 * zoom, 2.0), vec2((r.width() - 30.0 * zoom).min(260.0), (FRAME_BAR - 4.0) * zoom));
    let key = ui.scope_id().with(("renaming frame", &id));
    let mut title: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| f.title.clone());
    let edit = ui.put(field, egui::TextEdit::singleline(&mut title).font(FontId::proportional(12.5 * zoom)).margin(vec2(4.0, 0.0)));
    if !edit.has_focus() && !edit.lost_focus() {
        edit.request_focus();
    }
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if edit.lost_focus() || enter || escape {
        ui.data_mut(|d| d.remove::<String>(key));
        p.renaming_frame = None;
        let title = title.trim().to_owned();
        if !escape && !title.is_empty() && title != f.title {
            p.set_frame(&id, Some(title), None, None);
            p.end_edit();
        }
    } else {
        ui.data_mut(|d| d.insert_temp(key, title));
    }
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
            let exposed = p.layout.exposed.remove(&p.lkey(&format!("{id}.{name}")));
            if let Some(src) = src {
                use massif_terrain::format::Op;
                if p.ops(vec![Op::Unlink { to: id.clone(), input: name.clone() }, Op::Link { from: src, to: id.clone(), input: to.clone() }]).is_ok()
                    && let Some(v) = p.layout.reroutes.remove(&p.lkey(&format!("{id}.{name}")))
                {
                    let k = p.lkey(&format!("{id}.{to}"));
                    p.layout.reroutes.insert(k, v);
                }
            } else if exposed {
                let k = p.lkey(&format!("{id}.{to}"));
                p.edit_layout().exposed.insert(k);
            }
        }
    } else {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_node(p: &TerrainPanel, ui: &mut Ui, painter: &Painter, n: &Placed, pal: &Palette, status: &super::eval::Status, eye: Option<&str>, active: Option<&str>, drop: Option<&(String, String)>, refused: Option<(&str, &str)>, acts: &mut Vec<Act>) {
    let z = p.view.zoom;
    let rect = n.rect;
    let column = p.is_column(&n.id);
    let picked = p.selected.contains(&n.id);
    let primary = p.selected.last() == Some(&n.id);
    let crumbs = p.crumbs();
    let broken = status.broken.get(&shape::unfolded_id(&crumbs, &n.id)).or_else(|| p.thumb_key(&n.id).and_then(|k| status.broken.get(&k.node)));
    let is_output = n.shape.is_output();
    let is_active = is_output && active == Some(n.id.as_str());
    let node = p.graph.nodes.get(&n.id);
    let bypassed = node.is_some_and(|n| n.bypass);
    let read_only = p.read_only();
    let border = if broken.is_some() {
        pal.danger
    } else if picked {
        pal.accent
    } else {
        pal.border
    };
    let round = (theme::TILE_RADIUS as f32 * z).clamp(2.0, theme::TILE_RADIUS as f32);
    let family = sockets::family(n.shape.family);
    if column {
        // A column of an opened composite: a quiet band on the ground.
        painter.rect(rect, round, theme::mix(pal.panel, family, 0.06), Stroke::new(1.0, family.gamma_multiply(0.5)), StrokeKind::Inside);
    } else {
        // A composite stands on a second card, as a stack.
        if n.shape.is_composite() {
            let back = rect.translate(vec2(4.0, 4.0) * z);
            painter.rect(back, round, if pal.dark { theme::mix(pal.panel, pal.text, 0.02) } else { theme::mix(pal.panel, pal.app_bg, 0.5) }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        }
        // A soft shadow under the node.
        painter.rect_filled(rect.translate(vec2(0.0, 2.0 * z)).expand(1.0), round, Color32::from_black_alpha(if pal.dark { 70 } else { 22 }));
        let fill = if pal.dark { theme::mix(pal.panel, pal.text, 0.035) } else { pal.panel };
        painter.rect(rect, round, fill, Stroke::new(if primary { 2.0 } else if picked { 1.5 } else { 1.0 }, border), StrokeKind::Inside);
        let band = Rect::from_min_size(rect.min, vec2(rect.width(), (3.0 * z).max(2.0)));
        painter.with_clip_rect(band.intersect(painter.clip_rect())).rect_filled(rect, round, if bypassed { pal.weak } else { family });
    }
    let text_on = z >= 0.45;
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * z));

    // Anywhere on the node selects and drags it (its sockets excepted).
    if !column {
        let handle = ui.interact(rect, ui.scope_id().with(("terrain node", &n.id)), Sense::click_and_drag());
        if handle.drag_started_by(egui::PointerButton::Primary) && !picked {
            acts.push(Act::Select(n.id.clone(), ui.input(|i| i.modifiers.shift || i.modifiers.command)));
        }
        if handle.clicked() {
            acts.push(Act::Select(n.id.clone(), ui.input(|i| i.modifiers.shift || i.modifiers.command)));
        }
        if handle.dragged_by(egui::PointerButton::Primary) && p.wiring.is_none() && !read_only {
            acts.push(Act::Move(handle.drag_delta() / z));
        }
        if handle.drag_stopped() && !read_only {
            acts.push(Act::Dropped);
        }
        if handle.double_clicked() {
            acts.push(if n.shape.is_composite() { Act::Open(n.id.clone()) } else { Act::View(n.id.clone()) });
        }
        node_menu(p, &handle, n, is_output, is_active, bypassed, acts);
    }

    if text_on {
        // Fold chevron, title, and on the right the eye of the node viewed
        // or the active output's mark.
        let mut title_left = header.left() + 10.0 * z;
        if !column {
            let chevron = if p.collapsed(&n.id) { Icon::ChevronRight } else { Icon::ChevronDown };
            let cr = Rect::from_center_size(header.left_center() + vec2(12.0 * z, 1.5 * z), vec2(14.0, 14.0) * z);
            let c = ui.interact(cr, ui.scope_id().with(("fold", &n.id)), Sense::click());
            painter.text(cr.center(), Align2::CENTER_CENTER, theme::glyph(chevron).to_string(), FontId::proportional(11.0 * z), if c.hovered() { pal.text } else { pal.weak });
            if c.clicked() {
                acts.push(Act::ToggleCollapse(n.id.clone()));
            }
            title_left = header.left() + 22.0 * z;
        }
        let title = if column { n.shape.label.clone() } else { node_title(p, &n.id, &n.shape) };
        let mut right_room = if is_output { 70.0 } else { 26.0 } * z;
        if n.shape.is_composite() {
            // Its mark, a click away from its inside.
            let at = Rect::from_center_size(header.right_center() + vec2(-14.0 * z, 1.5 * z), vec2(18.0, 18.0) * z);
            let r = ui.interact(at, ui.scope_id().with(("open", &n.id)), Sense::click());
            painter.text(at.center(), Align2::CENTER_CENTER, theme::glyph(Icon::Boxes).to_string(), FontId::proportional(11.5 * z), if r.hovered() { pal.accent } else { pal.weak });
            if r.on_hover_text(t!("Open the composite (double-click)")).clicked() {
                acts.push(Act::Open(n.id.clone()));
            }
            right_room += 18.0 * z;
        }
        let title_rect = Rect::from_min_max(pos2(title_left, header.top()), pos2(header.right() - right_room, header.bottom()));
        let ink = if bypassed || column { pal.weak } else { pal.text };
        painter.with_clip_rect(title_rect.intersect(painter.clip_rect())).text(title_rect.left_center() + vec2(0.0, 1.5 * z), Align2::LEFT_CENTER, title, FontId::proportional(12.5 * z), ink);
        if eye == Some(n.id.as_str()) {
            let at = header.right_center() + vec2(-(if is_output { 62.0 } else if n.shape.is_composite() { 30.0 } else { 12.0 }) * z, 1.5 * z);
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

    let reach = 10.0 * z.max(0.7);
    let thumb_key = p.thumb_key(&n.id).cloned();
    for (row, rr) in &n.rows {
        match row {
            Row::Output(k) => {
                let o = &n.shape.outputs[*k];
                let at = pos2(rect.right(), rr.center().y);
                if text_on {
                    painter.text(pos2(rect.right() - 12.0 * z, at.y), Align2::RIGHT_CENTER, &o.label, FontId::proportional(11.5 * z), pal.weak);
                }
                sockets::draw(painter, at, SOCKET_R * z.max(0.7), o.kind, sockets::shape(o.kind), true, pal);
                let s = ui.interact(Rect::from_center_size(at, vec2(reach, reach) * 2.0), ui.scope_id().with(("out", &n.id, *k)), Sense::drag());
                if s.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    s.clone().on_hover_text(format!("{} · {}", o.label, sockets::kind_label(o.kind)));
                }
                if s.drag_started() && !read_only {
                    acts.push(Act::StartWire(n.id.clone(), *k as u8));
                }
            }
            Row::Thumb => {
                let t = Rect::from_min_size(pos2(rect.left() + PAD * z, rr.top() + 2.0 * z), vec2(rect.width() - 2.0 * PAD * z, THUMB_H * z));
                let well_r = (theme::TILE_RADIUS as f32 - 2.0) * z;
                painter.rect_filled(t, well_r, pal.well());
                if let Some(msg) = broken {
                    if text_on {
                        let g = painter.layout(msg.clone(), FontId::proportional(10.5 * z), pal.danger, t.width() - 12.0 * z);
                        painter.galley(t.min + vec2(6.0, 6.0) * z, g, pal.danger);
                    }
                } else if let Some((tex, pic)) = thumb_key.as_ref().and_then(|k| p.thumbs.get(k)) {
                    // Cover: the square picture cut to the frame's shape.
                    let aspect = t.width() / t.height();
                    let (uw, uh) = if aspect >= 1.0 { (1.0, 1.0 / aspect) } else { (aspect, 1.0) };
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uw, uh));
                    let tint = if bypassed { Color32::from_white_alpha(110) } else { Color32::WHITE };
                    egui::Image::new((tex.id(), t.size())).uv(uv).corner_radius(well_r).tint(tint).paint_at(ui, t);
                    if text_on
                        && z >= 0.8
                        && let Some(kind) = pic.kind
                    {
                        let range = match kind {
                            PortKind::Height => format!("{:.0}–{:.0} b", pic.range.0, pic.range.1),
                            PortKind::Mask => format!("{:.2}–{:.2}", pic.range.0, pic.range.1),
                            _ => String::new(),
                        };
                        if !range.is_empty() {
                            let g = painter.layout_no_wrap(range, FontId::proportional(9.5 * z), Color32::WHITE);
                            let chip = Rect::from_min_size(t.left_bottom() + vec2(4.0, -4.0 - g.size().y - 4.0), g.size() + vec2(8.0, 4.0));
                            painter.rect_filled(chip, theme::PILL, Color32::from_black_alpha(140));
                            painter.galley(chip.min + vec2(4.0, 2.0), g, Color32::WHITE);
                        }
                    }
                } else if text_on {
                    painter.text(t.center(), Align2::CENTER_CENTER, "…", FontId::proportional(14.0 * z), pal.weak);
                }
                if bypassed && text_on {
                    let g = painter.layout_no_wrap(format!("{}  {}", theme::glyph(Icon::Ban), t!("Bypassed")), FontId::proportional(10.5 * z), pal.text);
                    let chip = Rect::from_center_size(t.center(), g.size() + vec2(14.0, 6.0) * z);
                    painter.rect(chip, theme::PILL, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
                    painter.galley(chip.center() - g.size() / 2.0, g, pal.text);
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
                if r.clicked() && !read_only {
                    acts.push(Act::AddMask(n.id.clone()));
                }
            }
            Row::Input { name, label, kind, form, default } => {
                let at = pos2(rect.left(), rr.center().y);
                let src = if column { p.source_of(&n.id, name).map(|s| s.0) } else { node.and_then(|n| n.inputs.get(name)).cloned() };
                let connected = src.is_some();
                let target = drop.is_some_and(|d| d.0 == n.id && d.1 == *name);
                let refused_here = refused.is_some_and(|r| r.0 == n.id && r.1 == name);
                if target {
                    painter.rect_filled(rr.shrink2(vec2(2.0, 1.0)), 4.0, pal.accent.gamma_multiply(0.15));
                }
                if refused_here {
                    painter.rect_filled(rr.shrink2(vec2(2.0, 1.0)), 4.0, pal.danger.gamma_multiply(0.15));
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
                        if let Some(src) = &src {
                            let s = Source::parse(src);
                            let sw = Rect::from_min_size(pos2(right - 16.0 * z, at.y - 7.0 * z), vec2(16.0, 14.0) * z);
                            match p.thumb_key(s.node).and_then(|k| p.thumbs.get(k)) {
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
                            painter.with_clip_rect(room.intersect(painter.clip_rect())).text(pos2(name_x, at.y), Align2::RIGHT_CENTER, p.display_name(s.node), FontId::proportional(10.5 * z), pal.weak);
                        } else if !default.is_empty() {
                            let label_w = painter.layout_no_wrap(label.clone(), FontId::proportional(11.5 * z), ink).size().x;
                            let room = Rect::from_min_max(pos2(lx + label_w + 10.0 * z, rr.top()), pos2(right, rr.bottom()));
                            let mut job = egui::text::LayoutJob::simple_singleline(super::text::name_owned(default), FontId::proportional(10.5 * z), theme::mix(pal.weak, pal.panel, 0.25));
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room.width().max(4.0));
                            let g = painter.layout_job(job);
                            painter.galley(pos2(right - g.size().x, at.y - g.size().y / 2.0), g, pal.weak);
                        }
                    }
                }
                // A named mask of one's own: a double-click on its name
                // renames it.
                if is_output && n.shape.spec().is_some_and(|s| s.input(name).is_none()) {
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
                    let what = if default.is_empty() { sockets::kind_label(*kind).to_owned() } else { format!("{} · {}: {}", sockets::kind_label(*kind), t!("empty"), super::text::name_owned(default)) };
                    s.clone().on_hover_text(what);
                }
                if s.drag_started() && connected && !read_only {
                    acts.push(Act::Detach(n.id.clone(), name.clone()));
                }
            }
        }
    }
    if bypassed && !column {
        // A line straight through: what it feeds takes its input.
        let y = rect.top() + HEADER * z + 8.0 * z;
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.5 * z.max(0.7), pal.weak.gamma_multiply(0.7)));
    }
    if p.collapsed(&n.id) {
        // One socket of each side on the header, for the wires.
        if let Some(o) = n.shape.outputs.first() {
            sockets::draw(painter, rect.right_center(), SOCKET_R * z.max(0.7), o.kind, sockets::shape(o.kind), true, pal);
        }
        if node.is_some_and(|n| !n.inputs.is_empty()) {
            painter.circle_filled(rect.left_center(), 3.0 * z.max(0.7), pal.weak);
        }
    }
}

/// A node's right-click menu.
fn node_menu(p: &TerrainPanel, handle: &egui::Response, n: &Placed, is_output: bool, is_active: bool, bypassed: bool, acts: &mut Vec<Act>) {
    let read_only = p.read_only();
    let many = p.selected.len() > 1 && p.selected.contains(&n.id);
    handle.context_menu(|ui| {
        if ui.button(t!("View")).clicked() {
            acts.push(Act::View(n.id.clone()));
            ui.close();
        }
        if let Some((_, _, editable)) = n.shape.composite() {
            if ui.button(t!("Open")).clicked() {
                acts.push(Act::Open(n.id.clone()));
                ui.close();
            }
            if !editable && !read_only && ui.button(t!("Edit a copy")).clicked() {
                acts.push(Act::EditCopy(n.id.clone()));
                ui.close();
            }
            if !read_only && ui.button(t!("Ungroup (⌘⇧G)")).clicked() {
                acts.push(Act::Ungroup(n.id.clone()));
                ui.close();
            }
        }
        if ui.button(if p.collapsed(&n.id) { t!("Expand") } else { t!("Collapse") }).clicked() {
            acts.push(Act::ToggleCollapse(n.id.clone()));
            ui.close();
        }
        if read_only {
            return;
        }
        if is_output && !is_active && ui.button(t!("Make active")).clicked() {
            acts.push(Act::SetActive(n.id.clone()));
            ui.close();
        }
        ui.separator();
        if !is_output && ui.button(if bypassed { t!("Stop bypassing (B)") } else { t!("Bypass (B)") }).clicked() {
            let ids = if many { p.selected.clone() } else { vec![n.id.clone()] };
            acts.push(Act::Bypass(ids, !bypassed));
            ui.close();
        }
        if ui.button(t!("Duplicate (⌘D)")).clicked() {
            if !many {
                acts.push(Act::Select(n.id.clone(), false));
            }
            acts.push(Act::Duplicate);
            ui.close();
        }
        if ui.button(t!("Copy (⌘C)")).clicked() {
            if !many {
                acts.push(Act::Select(n.id.clone(), false));
            }
            acts.push(Act::Copy);
            ui.close();
        }
        if !is_output && ui.button(t!("Make a composite (⌘G)")).clicked() {
            if !many {
                acts.push(Act::Select(n.id.clone(), false));
            }
            acts.push(Act::Group);
            ui.close();
        }
        if ui.button(t!("Frame (⌘J)")).clicked() {
            if !many {
                acts.push(Act::Select(n.id.clone(), false));
            }
            acts.push(Act::Frame);
            ui.close();
        }
        if many {
            ui.menu_button(t!("Arrange"), |ui| {
                if ui.button(t!("In columns")).clicked() {
                    acts.push(Act::Arrange);
                    ui.close();
                }
                if ui.button(t!("Align left edges")).clicked() {
                    acts.push(Act::Align(true));
                    ui.close();
                }
                if ui.button(t!("Align tops")).clicked() {
                    acts.push(Act::Align(false));
                    ui.close();
                }
                if ui.button(t!("Space evenly across")).clicked() {
                    acts.push(Act::Distribute(true));
                    ui.close();
                }
                if ui.button(t!("Space evenly down")).clicked() {
                    acts.push(Act::Distribute(false));
                    ui.close();
                }
            });
        }
        ui.separator();
        if ui.button(t!("Remove")).clicked() {
            if !many {
                acts.push(Act::Select(n.id.clone(), false));
            }
            acts.push(Act::Remove);
            ui.close();
        }
    });
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

/// A node's title: its own name when it has one, else its type's name and
/// the choice that makes it what it is (Blend · Add).
pub fn node_title(p: &TerrainPanel, id: &str, shape: &NodeShape) -> String {
    let Some(node) = p.graph.nodes.get(id) else { return shape.label.clone() };
    if let Some(name) = &node.name {
        return name.clone();
    }
    let mut title = shape.label.clone();
    if let Some(spec) = shape.spec() {
        let choice = spec.params.iter().find(|q| matches!(q.kind, ParamKind::Choice { .. }) && q.section == registry::Section::Main);
        if let Some(q) = choice
            && let ParamKind::Choice { options, default } = q.kind
        {
            let v = node.params.get(q.name).and_then(|v| v.as_str()).unwrap_or(default);
            if let Some(o) = options.iter().find(|o| o.value == v) {
                title = format!("{title} · {}", super::text::name(o.label));
            }
        }
    }
    title
}

fn apply(p: &mut TerrainPanel, ui: &Ui, acts: Vec<Act>, origin: Pos2, now: f64) {
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
                p.selected_frame = None;
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
            Act::Dropped => {
                let ids = p.selected.clone();
                p.snap(&ids);
                p.refile(&ids);
                p.end_edit();
            }
            Act::View(id) => {
                p.viewed = Some((p.crumbs(), super::eval::Shown { node: id, port: None }));
            }
            Act::Open(id) => {
                p.open(&id);
            }
            Act::ToggleCollapse(id) => {
                let g = p.edit();
                if let Some(u) = g.ui.get_mut(&id) {
                    u.collapsed = !u.collapsed;
                }
                p.end_edit();
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
                    let k = p.lkey(&format!("{to}.{input}"));
                    p.layout.exposed.remove(&k);
                }
            }
            Act::Refused(why) => p.say(why, now),
            Act::Cut(to, input) => p.disconnect(&to, &input),
            Act::AddReroute(key, seg, at) => {
                let v = p.edit_layout().reroutes.entry(key).or_default();
                let k = seg.min(v.len());
                v.insert(k, at);
                p.end_edit();
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
                p.end_edit();
            }
            Act::OpenSearch(at, takes, wire) => {
                if p.read_only() {
                    continue;
                }
                let g = to_graph(p, origin, at);
                p.search = Some(Search { at: g, screen: at, query: String::new(), takes, wire, hot: 0 });
            }
            Act::MoveFrame(id, d) => p.move_frame(&id, [d.x, d.y]),
            Act::FrameDropped(id) => {
                let r = p.frame_rect(&id);
                let snapped = [(r[0] / super::edit::GRID).round() * super::edit::GRID - r[0], (r[1] / super::edit::GRID).round() * super::edit::GRID - r[1]];
                if !p.layout.no_snap && snapped != [0.0, 0.0] {
                    p.move_frame(&id, snapped);
                }
                p.refile_frame(&id);
                p.end_edit();
            }
            Act::SelectFrame(id) => {
                p.selected_frame = Some(id);
                p.selected.clear();
            }
            Act::ToggleFrame(id) => p.toggle_frame(&id),
            Act::RemoveFrame(id) => p.unframe(&id),
            Act::FrameColour(id, c) => {
                p.set_frame(&id, None, Some(c), None);
                p.end_edit();
            }
            Act::ToggleGroup(id, key) => {
                let k = p.lkey(&format!("{id}.{key}"));
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
                    if !taken.contains(&n) && !p.layout.exposed.contains(&p.lkey(&format!("{id}.{n}"))) {
                        break n;
                    }
                    k += 1;
                };
                let key = p.lkey(&format!("{id}.{name}"));
                p.edit_layout().exposed.insert(key);
                p.end_edit();
                p.renaming_mask = Some((id, name));
            }
            Act::RenameMask(id, name) => p.renaming_mask = Some((id, name)),
            Act::SetActive(id) => p.set_active_output(&id),
            Act::Bypass(ids, on) => p.set_bypass(&ids, on),
            Act::ToggleBypass => p.toggle_bypass(),
            Act::Group => match p.group_selected(t!("Group")) {
                Ok(_) => p.say(t!("Made a composite of the document: double-click it to open it."), now),
                Err(e) => p.say(e, now),
            },
            Act::Ungroup(id) => {
                if let Err(e) = p.ungroup(&id) {
                    p.say(e, now);
                }
            }
            Act::EditCopy(id) => match p.edit_copy(&id) {
                Ok(_) => p.say(t!("A copy of the composite is the document's own now."), now),
                Err(e) => p.say(e, now),
            },
            Act::Duplicate => {
                let ids = p.selected.clone();
                p.duplicate(&ids);
            }
            Act::Remove => {
                let ids = p.selected.clone();
                p.remove_nodes(&ids);
            }
            Act::Copy => {
                if let Some(text) = p.copy_text() {
                    ui.ctx().copy_text(text);
                }
            }
            Act::Arrange => p.arrange(),
            Act::Align(x) => p.align(x),
            Act::Distribute(x) => p.distribute(x),
            Act::Frame => {
                p.frame_selection();
            }
        }
    }
}

fn header_commands(p: &mut TerrainPanel, ui: &mut Ui) {
    let icon_button = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| -> bool {
        let r = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(theme::glyph(icon).to_string()).size(13.0)).frame(false).min_size(vec2(24.0, 24.0)));
        r.on_hover_text(tip).clicked()
    };
    let editable = !p.read_only();
    if icon_button(ui, Icon::Maximize2, t!("Frame all nodes (F)"), !p.graph.nodes.is_empty()) {
        let canvas = ui.max_rect();
        p.view.zoom = 1.0;
        let (lo, _) = bounds(p);
        p.view.offset = vec2(40.0 - lo[0], 30.0 - lo[1]);
        let _ = canvas;
    }
    let snap = !p.layout.no_snap;
    let r = ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::Magnet).to_string()).size(13.0).color(if snap { Palette::of(ui).accent } else { Palette::of(ui).weak })).frame(false).min_size(vec2(24.0, 24.0)));
    if r.on_hover_text(if snap { t!("Nodes snap to the grid: click to place them freely") } else { t!("Nodes are placed freely: click to snap them to the grid") }).clicked() {
        p.layout.no_snap = snap;
        p.dirty_layout();
    }
    if icon_button(ui, Icon::LayoutGrid, t!("Arrange the selected nodes, or all, in columns"), editable && !p.graph.nodes.is_empty()) {
        p.arrange();
    }
    if icon_button(ui, Icon::Frame, t!("Frame the selected nodes (⌘J)"), editable && !p.selected.is_empty()) {
        p.frame_selection();
    }
    if icon_button(ui, Icon::Boxes, t!("Make a composite of the selected nodes (⌘G)"), editable && !p.selected.is_empty())
        && let Err(e) = p.group_selected(t!("Group"))
    {
        p.say(e, 0.0);
    }
    if icon_button(ui, Icon::RotateCw, t!("Redo"), p.can_redo()) {
        p.redo();
    }
    if icon_button(ui, Icon::RotateCcw, t!("Undo"), p.can_undo()) {
        p.undo();
    }
    if icon_button(ui, Icon::Plus, t!("Add a node (Tab)"), editable) {
        let at = ui.max_rect().left_bottom() + vec2(60.0, 60.0);
        p.search = Some(Search { at: [40.0 - p.view.offset.x, 40.0 - p.view.offset.y], screen: at, ..Default::default() });
    }
}

fn bounds(p: &TerrainPanel) -> ([f32; 2], [f32; 2]) {
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for id in p.ids() {
        let pos = p.pos(&id);
        let s = node_size(p, &id);
        lo = [lo[0].min(pos[0]), lo[1].min(pos[1])];
        hi = [hi[0].max(pos[0] + s.x), hi[1].max(pos[1] + s.y)];
    }
    for id in p.graph.frames.keys() {
        let r = p.frame_rect(id);
        lo = [lo[0].min(r[0]), lo[1].min(r[1])];
        hi = [hi[0].max(r[0] + r[2]), hi[1].max(r[1] + r[3])];
    }
    if lo[0] > hi[0] {
        return ([0.0; 2], [1.0; 2]);
    }
    (lo, hi)
}

/// Zooms and moves the view so that every node shows in `canvas`.
pub fn fit_view(p: &mut TerrainPanel, canvas: Rect) {
    if p.graph.nodes.is_empty() && p.outer.is_empty() {
        return;
    }
    let (lo, hi) = bounds(p);
    let (w, h) = (hi[0] - lo[0] + 60.0, hi[1] - lo[1] + 60.0);
    // Not so small that the nodes' words cannot be read: past that, the
    // graph's end (where its Map Output is) shows, and the minimap the rest.
    p.view.zoom = (canvas.width() / w).min(canvas.height() / h).clamp(0.6, 1.15);
    let size = vec2(hi[0] - lo[0], hi[1] - lo[1]) * p.view.zoom;
    p.view.offset = (canvas.size() - size) / 2.0 - vec2(lo[0], lo[1]) * p.view.zoom;
    if size.x > canvas.width() - 40.0 {
        p.view.offset.x = canvas.width() - 30.0 - hi[0] * p.view.zoom;
    }
    if size.y > canvas.height() - 40.0 {
        p.view.offset.y = 24.0 - lo[1] * p.view.zoom;
    }
}

/// A frame around the selected nodes (kept for the tests' name).
pub fn frame_selection(p: &mut TerrainPanel) {
    p.frame_selection();
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
    let covers = |r: Rect| {
        p.graph.nodes.keys().any(|id| {
            let pos = p.pos(id);
            let at = canvas.min + p.view.offset + vec2(pos[0], pos[1]) * p.view.zoom;
            Rect::from_min_size(at, node_size(p, id) * p.view.zoom).intersects(r)
        })
    };
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
    for id in p.graph.frames.keys() {
        if p.frame_hidden(id) {
            continue;
        }
        let f = p.frame_rect(id);
        painter.rect_stroke(Rect::from_min_max(at([f[0], f[1]]), at([f[0] + f[2], f[1] + f[3]])), 2.0, Stroke::new(1.0, pal.weak.gamma_multiply(0.5)), StrokeKind::Inside);
    }
    for id in p.graph.nodes.keys() {
        if p.hidden_in(id).is_some() {
            continue;
        }
        let pos = p.pos(id);
        let sz = node_size(p, id);
        let r = Rect::from_min_max(at(pos), at([pos[0] + sz.x, pos[1] + sz.y]));
        let family = p.shape(id).map_or(Family::Internal, |s| s.family);
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

/// Something the search or the library offers to add: a node of the
/// registry, or a composite (Massif's or the document's).
#[derive(Debug, Clone)]
pub struct Entry {
    /// The type added (`noise.fbm`, `massif:mountain`, `project:hills`).
    pub id: String,
    pub label: String,
    pub summary: String,
    pub family: Family,
    pub keywords: Vec<String>,
    pub spec: Option<&'static NodeSpec>,
    /// The types its inputs take.
    pub takes: Vec<PortKind>,
}

impl Entry {
    fn native(s: &'static NodeSpec) -> Self {
        let mut takes: Vec<PortKind> = s.inputs.iter().map(|i| i.kind).collect();
        if let Some(o) = s.open_inputs {
            takes.push(o.kind);
        }
        Entry { id: s.id.to_owned(), label: super::text::name(s.label).to_owned(), summary: super::text::name(s.summary).to_owned(), family: s.family, keywords: s.keywords.iter().map(|k| (*k).to_owned()).collect(), spec: Some(s), takes }
    }

    fn composite(doc: &massif_terrain::model::Graph, kind: &str) -> Option<Self> {
        let shape = NodeShape::composite_of(doc, kind)?;
        let english = massif_terrain::lower::component(doc, kind).map(|c| c.label.to_lowercase()).unwrap_or_default();
        Some(Entry { id: kind.to_owned(), label: shape.label.clone(), summary: shape.summary.clone(), family: shape.family, keywords: vec![english, t!("composite").to_owned()], spec: None, takes: shape.inputs.iter().map(|i| i.kind).collect() })
    }

    pub fn is_composite(&self) -> bool {
        self.spec.is_none()
    }
}

/// Every type of node the library and the search offer: the registry's,
/// then the composites Massif ships and those of the document.
pub fn entries(doc: &massif_terrain::model::Graph) -> Vec<Entry> {
    let mut out: Vec<Entry> = registry::all().iter().copied().filter(|s| offered(s)).map(Entry::native).collect();
    let mut kinds: Vec<String> = massif_terrain::components::all().iter().map(|(k, _, _)| (*k).to_owned()).collect();
    kinds.sort();
    kinds.dedup();
    kinds.extend(doc.components.keys().cloned());
    out.extend(kinds.iter().filter_map(|k| Entry::composite(doc, k)));
    out
}

/// Types of node matching a search, best first.
pub fn find_in(doc: &massif_terrain::model::Graph, query: &str, takes: Option<PortKind>) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    let mut hits: Vec<(i32, Entry)> = entries(doc)
        .into_iter()
        .filter(|e| takes.is_none_or(|k| e.takes.iter().any(|t| k.conversion(*t).is_some())))
        .filter_map(|e| {
            if q.is_empty() {
                return Some((0, e));
            }
            let label = e.label.to_lowercase();
            let english = e.spec.map_or_else(|| label.clone(), |s| s.label.to_lowercase());
            let score = if label.starts_with(&q) || english.starts_with(&q) {
                100
            } else if label.split(' ').any(|w| w.starts_with(&q)) || english.split(' ').any(|w| w.starts_with(&q)) {
                80
            } else if label.contains(&q) || english.contains(&q) || e.id.contains(&q) {
                60
            } else if e.keywords.iter().any(|k| k.starts_with(&q)) {
                50
            } else if fuzzy(&q, &english) || fuzzy(&q, &label) {
                20
            } else {
                return None;
            };
            Some((score, e))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.family.cmp(&b.1.family)).then(a.1.is_composite().cmp(&b.1.is_composite())));
    hits.into_iter().map(|h| h.1).collect()
}

/// Types of node of the registry matching a search, best first.
pub fn find(query: &str, takes: Option<PortKind>) -> Vec<&'static NodeSpec> {
    find_in(&massif_terrain::model::Graph::default(), query, takes).into_iter().filter_map(|e| e.spec).collect()
}

/// Whether the library and the search offer a type of node: not the
/// compiler's own, nor the nodes of 1.2 that only migrated graphs hold.
pub fn offered(s: &NodeSpec) -> bool {
    s.family != Family::Internal && !s.id.starts_with("legacy.") && registry::latest(s.id).is_some_and(|l| l.version == s.version)
}

/// Whether the letters of `q` come in order in `s`.
fn fuzzy(q: &str, s: &str) -> bool {
    let mut it = s.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|d| d == c))
}

fn search_ui(p: &mut TerrainPanel, ui: &mut Ui, canvas: Rect, pal: &Palette) {
    let Some(mut s) = p.search.take() else { return };
    let width = 320.0;
    let pos = pos2(s.screen.x.min(canvas.right() - width - 8.0).max(canvas.left() + 8.0), s.screen.y.min(canvas.bottom() - 340.0).max(canvas.top() + 8.0));
    let hits = find_in(&p.graph, &s.query, s.takes);
    let mut chosen: Option<Entry> = None;
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
                chosen = Some(h.clone());
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
                for (k, e) in hits.iter().enumerate().take(80) {
                    if s.query.is_empty() && last_family != Some(e.family) {
                        last_family = Some(e.family);
                        ui.add_space(if k == 0 { 0.0 } else { 6.0 });
                        ui.label(egui::RichText::new(super::text::name(e.family.label()).to_uppercase()).size(10.0).color(pal.weak));
                    }
                    let hot = k == s.hot;
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                    if hot || r.hovered() {
                        ui.painter().rect_filled(rect, theme::TILE_RADIUS, pal.accent_soft);
                    }
                    if e.is_composite() {
                        ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::CENTER_CENTER, theme::glyph(Icon::Boxes).to_string(), FontId::proportional(11.0), sockets::family(e.family));
                    } else {
                        ui.painter().circle_filled(rect.left_center() + vec2(10.0, 0.0), 4.0, sockets::family(e.family));
                    }
                    ui.painter().text(rect.left_center() + vec2(22.0, 0.0), Align2::LEFT_CENTER, &e.label, FontId::proportional(12.5), pal.text);
                    let tag = if e.is_composite() { t!("Composite").to_owned() } else { super::text::name(e.family.label()).to_owned() };
                    ui.painter().with_clip_rect(rect).text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, tag, FontId::proportional(10.5), pal.weak);
                    if hot && down | up {
                        r.scroll_to_me(None);
                    }
                    let r = if e.summary.is_empty() { r } else { r.on_hover_text(&e.summary) };
                    if r.clicked() {
                        chosen = Some(e.clone());
                    }
                }
                if hits.is_empty() {
                    ui.label(egui::RichText::new(t!("No node matches.")).color(pal.weak));
                }
            });
        });
    });
    let clicked_out = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && ui.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
    if let Some(e) = chosen {
        let id = p.add_node(&e.id, s.at);
        if !id.is_empty()
            && let Some((from, port)) = s.wire.clone()
        {
            let kind = p.out_kind(&from, port).unwrap_or(PortKind::Height);
            let shape = p.shape(&id);
            let input = shape.as_ref().and_then(|sh| sh.inputs.iter().find(|i| i.kind == kind).or_else(|| sh.inputs.iter().find(|i| kind.conversion(i.kind).is_some())).map(|i| i.name.clone())).or_else(|| e.spec.and_then(|sp| sp.open_inputs).map(|_| "mask.custom".to_owned()));
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
