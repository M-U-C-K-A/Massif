//! The terrain graph editor: nodes on a canvas, wired output to input, with
//! a live preview of every node and a build that writes the result to the
//! map, in the manner of World Machine or Gaea.
//!
//! - Right-click the canvas to add a node; drag a node by its title.
//! - Drag from an output (right) to an input (left) to wire them; click a
//!   wired input to unplug it.
//! - Scroll or drag the background with the middle button to move around,
//!   ⌘/Ctrl + scroll or pinch to zoom.
//! - The selected node's settings and a larger preview are on the right.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::{Color32, ColorImage, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use massif_core::graph::{CellStyle, CombineOp, Extent, Family, Graph, GraphError, NodeId, NodeKind, NoiseStyle, Outputs};
use massif_core::registry::{Registry, TerrainId};
use massif_core::{BlockRect, TILE_SIZE};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// Samples per side of the previews.
const PREVIEW: usize = 160;
const NODE_WIDTH: f32 = 176.0;
const HEADER: f32 = 24.0;
const ROW: f32 = 18.0;
const THUMB: f32 = 88.0;
/// Build resolutions offered, in samples per side.
pub const RESOLUTIONS: [usize; 5] = [512, 1024, 2048, 3072, 4096];

type Previews = HashMap<(NodeId, u8), ColorImage>;
/// A build on another thread: its result, and the flag that cancels it.
type Building = (Receiver<Result<Built, GraphError>>, Arc<AtomicBool>);

/// A finished build: what to write to the map.
struct Built {
    extent: Extent,
    outputs: Outputs,
}

pub struct GraphEditor {
    /// The world the graph belongs to (its generation).
    generation: Option<u64>,
    selected: Option<NodeId>,
    /// Output shown in the large preview.
    shown_output: u8,
    pan: Vec2,
    zoom: f32,
    /// A wire being drawn from an output.
    wiring: Option<(NodeId, u8)>,
    textures: HashMap<(NodeId, u8), TextureHandle>,
    /// Revision of the graph, and of the last preview asked for.
    revision: u64,
    previewed: u64,
    preview: Option<Receiver<(u64, Result<Previews, GraphError>)>>,
    pub resolution: usize,
    build: Option<Building>,
    error: Option<String>,
    /// Frame the whole graph at the next opportunity.
    fit: bool,
}

impl Default for GraphEditor {
    fn default() -> Self {
        Self {
            generation: None,
            selected: None,
            shown_output: 0,
            pan: vec2(20.0, 20.0),
            zoom: 1.0,
            wiring: None,
            textures: HashMap::new(),
            revision: 1,
            previewed: 0,
            preview: None,
            resolution: 1024,
            build: None,
            error: None,
            fit: true,
        }
    }
}

/// The square of the world the graph covers: the whole map.
fn map_extent(doc: &Document, n: usize) -> Extent {
    let rect = match doc.dimension().bounds() {
        Some((lo, hi)) => BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE),
        None => BlockRect::new(-512, -512, 512, 512),
    };
    Extent::covering(rect, n)
}

/// Is this output a mask (0..1) rather than heights? Selectors make masks,
/// and so does what reshapes or mixes a mask.
fn is_mask(graph: &Graph, id: NodeId, output: u8) -> bool {
    let Some(node) = graph.node(id) else { return false };
    match &node.kind {
        k if k.family() == Family::Selector => true,
        NodeKind::Erosion { .. } => output > 0,
        NodeKind::Radial { height, base, .. } => *height <= 1.0 && *base == 0.0,
        NodeKind::Constant { value } => (0.0..=1.0).contains(value),
        NodeKind::Levels { out_max, out_min, .. } => *out_min >= 0.0 && *out_max <= 1.0,
        NodeKind::Noise { .. } | NodeKind::Cells { .. } => false,
        _ => graph.input_of(id, 0).is_some_and(|(src, out)| src != id && is_mask(graph, src, out)),
    }
}

/// A picture of a field: shaded relief for heights, grey for masks.
fn picture(field: &massif_core::graph::Field, mask: bool, spacing: f32) -> ColorImage {
    let n = field.n;
    let (lo, hi) = field.range();
    let span = (hi - lo).max(1e-3);
    let mut pixels = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let v = field.at(i, j);
            if mask {
                let g = (v.clamp(0.0, 1.0) * 255.0) as u8;
                pixels.push(Color32::from_rgb(g, g, g));
                continue;
            }
            let t = (v - lo) / span;
            // Low green, mid brown, high grey to white.
            let ramp = [
                (0.0, [62.0, 112.0, 58.0]),
                (0.45, [128.0, 116.0, 74.0]),
                (0.75, [150.0, 146.0, 140.0]),
                (1.0, [245.0, 245.0, 248.0]),
            ];
            let mut c = ramp[0].1;
            for w in ramp.windows(2) {
                if t >= w[0].0 && t <= w[1].0 {
                    let k = (t - w[0].0) / (w[1].0 - w[0].0);
                    c = [0, 1, 2].map(|q| w[0].1[q] + (w[1].1[q] - w[0].1[q]) * k);
                }
            }
            let dx = (field.at(i + 1, j) - field.at(i.saturating_sub(1), j)) / (2.0 * spacing);
            let dz = (field.at(i, j + 1) - field.at(i, j.saturating_sub(1))) / (2.0 * spacing);
            let shade = (1.0 - (dx + dz) * 0.9).clamp(0.4, 1.4);
            pixels.push(Color32::from_rgb(
                (c[0] * shade).min(255.0) as u8,
                (c[1] * shade).min(255.0) as u8,
                (c[2] * shade).min(255.0) as u8,
            ));
        }
    }
    ColorImage::new([n, n], pixels)
}

impl GraphEditor {
    fn changed(&mut self, doc: &mut Document, graph: Graph) {
        doc.world.meta.graph = Some(graph);
        doc.mark_changed();
        self.revision += 1;
    }

    /// Start previewing the current graph on another thread.
    fn request_preview(&mut self, ctx: &egui::Context, graph: Graph, extent: Extent) {
        let (tx, rx) = channel();
        let revision = self.revision;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let ids: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
            let result = graph
                .evaluate(&extent, &ids, true, &AtomicBool::new(false))
                .or_else(|e| {
                    // Evaluate what can be: nodes whose inputs are all there.
                    let ok: Vec<NodeId> = ids
                        .iter()
                        .copied()
                        .filter(|&id| graph.evaluate(&extent, &[id], false, &AtomicBool::new(false)).is_ok())
                        .collect();
                    if ok.is_empty() { Err(e) } else { graph.evaluate(&extent, &ok, true, &AtomicBool::new(false)) }
                })
                .map(|eval| {
                    eval.fields
                        .iter()
                        .map(|(&(id, out), f)| {
                            ((id, out), picture(f, is_mask(&graph, id, out), extent.spacing()))
                        })
                        .collect()
                });
            let _ = tx.send((revision, result));
            ctx.request_repaint();
        });
        self.preview = Some(rx);
        self.previewed = revision;
    }

    pub fn ui(&mut self, ui: &mut Ui, doc: &mut Document) -> Option<String> {
        let mut feedback = None;
        let pal = Palette::of(ui);
        // A new world: its graph, or one to start from.
        if self.generation != Some(doc.generation) {
            self.generation = Some(doc.generation);
            self.textures.clear();
            self.selected = None;
            self.revision += 1;
            self.fit = true;
        }
        let mut graph = match &doc.world.meta.graph {
            Some(g) => g.clone(),
            None => {
                let reg = &doc.world.registry;
                let get = |name: &str| reg.terrain_by_name(name).unwrap_or(TerrainId::ROCK).0;
                Graph::starter(get("Grass"), TerrainId::ROCK.0, get("Snow"), get("Gravel"))
            }
        };
        let before = graph.clone();

        // Previews come back from their thread.
        if let Some(rx) = &self.preview
            && let Ok((revision, result)) = rx.try_recv()
        {
            self.preview = None;
            match result {
                Ok(images) => {
                    self.error = None;
                    self.textures.retain(|k, _| images.contains_key(k));
                    for (key, image) in images {
                        let name = format!("graph {key:?}");
                        match self.textures.get_mut(&key) {
                            Some(t) => t.set(image, TextureOptions::LINEAR),
                            None => {
                                self.textures.insert(key, ui.ctx().load_texture(name, image, TextureOptions::LINEAR));
                            }
                        }
                    }
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            let _ = revision;
        }
        if self.preview.is_none() && self.previewed != self.revision {
            self.request_preview(ui.ctx(), graph.clone(), map_extent(doc, PREVIEW));
        }
        // A build finished: write it to the map.
        if let Some((rx, _)) = &self.build
            && let Ok(result) = rx.try_recv()
        {
            self.build = None;
            match result {
                Ok(built) => {
                    doc.history.begin("Terrain graph", doc.dim);
                    let seed = doc.world.meta.seed as u64;
                    massif_core::graph::apply(&mut doc.history.edit(&mut doc.world, 0), &built.extent, &built.outputs, seed);
                    doc.commit();
                    feedback = Some(format!(
                        "Built the terrain graph at {} × {} and applied it to the map",
                        built.extent.n, built.extent.n
                    ));
                }
                Err(GraphError::Cancelled) => feedback = Some("Build cancelled".to_owned()),
                Err(e) => feedback = Some(format!("The graph cannot be built: {e}")),
            }
        }

        // ---- toolbar ---------------------------------------------------------
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let building = self.build.is_some();
            if building {
                ui.spinner();
                ui.label("Building…");
                if ui.button("Cancel").clicked()
                    && let Some((_, cancel)) = &self.build
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            } else if ui
                .add(theme::primary_button(ui, format!("{}  Build and apply", theme::glyph(Icon::Play))))
                .on_hover_text("Build the graph at the chosen resolution over the whole map, and write the heights and terrains to the ground (undoable)")
                .clicked()
            {
                let (tx, rx) = channel();
                let cancel = Arc::new(AtomicBool::new(false));
                let flag = cancel.clone();
                let extent = map_extent(doc, self.resolution);
                let g = graph.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = g.build(&extent, &flag).map(|outputs| Built { extent, outputs });
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                self.build = Some((rx, cancel));
            }
            ui.label(RichText::new("at").color(pal.weak));
            egui::ComboBox::from_id_salt("graph resolution")
                .selected_text(format!("{0} × {0}", self.resolution))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for r in RESOLUTIONS {
                        ui.selectable_value(&mut self.resolution, r, format!("{r} × {r}"));
                    }
                });
            let blocks = map_extent(doc, self.resolution);
            ui.label(RichText::new(format!("{:.1} blocks per sample", blocks.spacing())).size(11.5).color(pal.weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Start over").on_hover_text("Replace the graph by the starter one").clicked() {
                    doc.world.meta.graph = None;
                    self.revision += 1;
                    self.selected = None;
                    self.fit = true;
                }
                if ui.small_button(theme::with_icon(Icon::Maximize, "Fit")).on_hover_text("Show the whole graph").clicked() {
                    self.fit = true;
                }
                if self.preview.is_some() {
                    ui.spinner();
                }
                if let Some(e) = &self.error {
                    ui.colored_label(pal.danger, e);
                }
            });
        });
        ui.separator();

        // ---- canvas and inspector ---------------------------------------------
        let full = ui.available_rect_before_wrap();
        let side = if full.width() > 640.0 { 280.0 } else { 0.0 };
        let canvas = Rect::from_min_max(full.min, pos2(full.max.x - side, full.max.y));
        let inspector = Rect::from_min_max(pos2(canvas.max.x, full.min.y), full.max);
        self.canvas(ui, canvas, &mut graph, &doc.world.registry);
        if side > 0.0 {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inspector.shrink2(vec2(10.0, 6.0))).layout(egui::Layout::top_down(egui::Align::Min)));
            egui::ScrollArea::vertical().id_salt("graph inspector").show(&mut child, |ui| {
                self.inspector(ui, &mut graph, &doc.world.registry);
            });
        }
        ui.allocate_rect(full, Sense::hover());
        if graph != before {
            self.changed(doc, graph);
        }
        feedback
    }

    fn to_screen(&self, canvas: Rect, p: [f32; 2]) -> Pos2 {
        canvas.min + self.pan + vec2(p[0], p[1]) * self.zoom
    }

    fn node_rect(&self, canvas: Rect, kind: &NodeKind, pos: [f32; 2]) -> Rect {
        let rows = kind.inputs().len().max(kind.outputs().len()) as f32;
        let h = HEADER + rows * ROW + if kind.outputs().is_empty() { 6.0 } else { THUMB + 10.0 };
        Rect::from_min_size(self.to_screen(canvas, pos), vec2(NODE_WIDTH, h) * self.zoom)
    }

    fn input_pos(&self, rect: Rect, k: usize) -> Pos2 {
        pos2(rect.left(), rect.top() + (HEADER + (k as f32 + 0.5) * ROW) * self.zoom)
    }

    fn output_pos(&self, rect: Rect, k: usize) -> Pos2 {
        pos2(rect.right(), rect.top() + (HEADER + (k as f32 + 0.5) * ROW) * self.zoom)
    }

    /// Zoom and move so that every node is in view.
    fn frame(&mut self, canvas: Rect, graph: &Graph) {
        if graph.nodes.is_empty() {
            return;
        }
        let (mut lo, mut hi) = (vec2(f32::MAX, f32::MAX), vec2(f32::MIN, f32::MIN));
        for n in &graph.nodes {
            let rows = n.kind.inputs().len().max(n.kind.outputs().len()) as f32;
            let size = vec2(NODE_WIDTH, HEADER + rows * ROW + THUMB + 10.0);
            lo = lo.min(vec2(n.pos[0], n.pos[1]));
            hi = hi.max(vec2(n.pos[0], n.pos[1]) + size);
        }
        let span = hi - lo + vec2(60.0, 60.0);
        self.zoom = (canvas.width() / span.x).min(canvas.height() / span.y).clamp(0.3, 1.2);
        let centre = (lo + hi) * 0.5;
        self.pan = canvas.size() * 0.5 - centre * self.zoom;
    }

    fn canvas(&mut self, ui: &mut Ui, canvas: Rect, graph: &mut Graph, reg: &Registry) {
        let pal = Palette::of(ui);
        if self.fit && canvas.width() > 50.0 {
            self.frame(canvas, graph);
            self.fit = false;
        }
        let background = ui.interact(canvas, ui.scope_id().with("graph canvas"), Sense::click_and_drag());
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0, pal.canvas);
        // Dot grid.
        let step = 24.0 * self.zoom;
        if step > 6.0 {
            let start = canvas.min + vec2(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
            let mut y = start.y;
            while y < canvas.max.y {
                let mut x = start.x;
                while x < canvas.max.x {
                    painter.circle_filled(pos2(x, y), 0.8, pal.border);
                    x += step;
                }
                y += step;
            }
        }
        // Moving and zooming.
        if background.dragged_by(egui::PointerButton::Middle) || background.dragged_by(egui::PointerButton::Primary) {
            self.pan += background.drag_delta();
        }
        if background.hovered() || ui.rect_contains_pointer(canvas) {
            let (scroll, zoom, cmd) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.command || i.modifiers.ctrl));
            let pointer = ui.input(|i| i.pointer.hover_pos()).unwrap_or(canvas.center());
            let factor = if zoom != 1.0 {
                zoom
            } else if cmd && scroll.y != 0.0 {
                (scroll.y * 0.004).exp()
            } else {
                1.0
            };
            if factor != 1.0 {
                let new_zoom = (self.zoom * factor).clamp(0.3, 2.5);
                let anchor = pointer - canvas.min - self.pan;
                self.pan -= anchor * (new_zoom / self.zoom - 1.0);
                self.zoom = new_zoom;
            } else if scroll != Vec2::ZERO && ui.rect_contains_pointer(canvas) {
                self.pan += scroll;
            }
        }
        // Add a node where the menu was opened.
        let menu_at = ui.ctx().memory(|m| m.data.get_temp::<Pos2>(ui.scope_id().with("graph menu at")));
        background.context_menu(|ui| {
            let at = menu_at.unwrap_or(canvas.center());
            let pos = (at - canvas.min - self.pan) / self.zoom;
            ui.label(RichText::new("Add a node").bold());
            for family in [Family::Generator, Family::Modifier, Family::Erosion, Family::Combiner, Family::Selector, Family::Output] {
                let name = match family {
                    Family::Generator => "Generators",
                    Family::Modifier => "Modifiers",
                    Family::Erosion => "Erosion",
                    Family::Combiner => "Combiners",
                    Family::Selector => "Selectors",
                    Family::Output => "Outputs",
                };
                ui.menu_button(name, |ui| {
                    for kind in NodeKind::catalogue().into_iter().filter(|k| k.family() == family) {
                        if ui.button(kind.label()).clicked() {
                            let id = graph.add(kind, [pos.x, pos.y]);
                            self.selected = Some(id);
                            ui.close();
                        }
                    }
                });
            }
        });
        if background.secondary_clicked()
            && let Some(p) = background.interact_pointer_pos()
        {
            ui.ctx().memory_mut(|m| m.data.insert_temp(ui.scope_id().with("graph menu at"), p));
        }
        if background.clicked() {
            self.selected = None;
        }

        // Wires.
        let rects: HashMap<NodeId, Rect> = graph.nodes.iter().map(|n| (n.id, self.node_rect(canvas, &n.kind, n.pos))).collect();
        let wire = |painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32| {
            let d = ((b.x - a.x).abs() * 0.5).max(30.0 * self.zoom);
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(
                [a, a + vec2(d, 0.0), b - vec2(d, 0.0), b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0 * self.zoom.max(0.6), color),
            );
            painter.add(curve);
        };
        for l in &graph.links {
            let (Some(a), Some(b)) = (rects.get(&l.from), rects.get(&l.to)) else { continue };
            let color = if self.selected == Some(l.from) || self.selected == Some(l.to) { pal.accent } else { pal.weak };
            wire(&painter, self.output_pos(*a, l.output as usize), self.input_pos(*b, l.input as usize), color);
        }

        // Nodes.
        let mut drop_on: Option<(NodeId, u8)> = None;
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.any_released());
        let mut remove = None;
        let ids: Vec<NodeId> = graph.nodes.iter().map(|n| n.id).collect();
        for id in ids {
            let node = graph.node(id).expect("listed").clone();
            let rect = rects[&id];
            let family_color = match node.kind.family() {
                Family::Generator => Color32::from_rgb(52, 199, 120),
                Family::Modifier => Color32::from_rgb(76, 141, 255),
                Family::Erosion => Color32::from_rgb(245, 140, 60),
                Family::Combiner => Color32::from_rgb(168, 120, 255),
                Family::Selector => Color32::from_rgb(240, 196, 60),
                Family::Output => Color32::from_rgb(244, 84, 120),
            };
            let selected = self.selected == Some(id);
            // A card with a coloured dot for its family, as on node canvases
            // of current tools.
            painter.rect(
                rect,
                10,
                pal.header,
                Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { pal.accent } else { pal.border }),
                StrokeKind::Inside,
            );
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER * self.zoom));
            painter.hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0, pal.border));
            painter.circle_filled(header.left_center() + vec2(11.0 * self.zoom, 0.0), 3.5 * self.zoom, family_color);
            let title = match &node.kind {
                NodeKind::TerrainOutput { terrain } => format!("{} · {}", node.kind.label(), reg.terrain(TerrainId(*terrain)).name),
                NodeKind::Combine { op, .. } => format!("{} · {}", node.kind.label(), op.label()),
                _ => node.kind.label().to_owned(),
            };
            painter.text(
                header.left_center() + vec2(20.0 * self.zoom, 0.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(12.0 * self.zoom),
                pal.text,
            );
            // Drag the node by its header; click it to select it.
            let handle = ui.interact(header, ui.scope_id().with(("graph node", id)), Sense::click_and_drag());
            if handle.dragged()
                && let Some(n) = graph.node_mut(id)
            {
                let d = handle.drag_delta() / self.zoom;
                n.pos = [n.pos[0] + d.x, n.pos[1] + d.y];
            }
            if handle.clicked() || handle.drag_started() {
                self.selected = Some(id);
            }
            handle.context_menu(|ui| {
                if ui.button(theme::with_icon(Icon::Trash2, "Delete node")).clicked() {
                    remove = Some(id);
                    ui.close();
                }
            });
            // Ports.
            let r = 5.0 * self.zoom;
            for (k, name) in node.kind.inputs().iter().enumerate() {
                let p = self.input_pos(rect, k);
                let wired = graph.input_of(id, k as u8).is_some();
                painter.circle(p, r, if wired { pal.text } else { pal.header }, Stroke::new(1.5, pal.weak));
                painter.text(p + vec2(9.0 * self.zoom, 0.0), egui::Align2::LEFT_CENTER, *name, egui::FontId::proportional(11.0 * self.zoom), pal.text);
                let port = ui.interact(Rect::from_center_size(p, vec2(16.0, 16.0) * self.zoom.max(0.7)), ui.scope_id().with(("in", id, k)), Sense::click());
                if port.clicked() && wired {
                    graph.disconnect(id, k as u8);
                }
                if self.wiring.is_some() && pointer.is_some_and(|q| q.distance(p) < 14.0 * self.zoom.max(0.7)) {
                    painter.circle_stroke(p, r + 3.0, Stroke::new(2.0, pal.accent));
                    if released {
                        drop_on = Some((id, k as u8));
                    }
                }
            }
            for (k, name) in node.kind.outputs().iter().enumerate() {
                let p = self.output_pos(rect, k);
                painter.circle(p, r, pal.text, Stroke::new(1.5, pal.weak));
                painter.text(p - vec2(9.0 * self.zoom, 0.0), egui::Align2::RIGHT_CENTER, *name, egui::FontId::proportional(11.0 * self.zoom), pal.text);
                let port = ui.interact(Rect::from_center_size(p, vec2(16.0, 16.0) * self.zoom.max(0.7)), ui.scope_id().with(("out", id, k)), Sense::drag());
                if port.drag_started() {
                    self.wiring = Some((id, k as u8));
                }
            }
            // Thumbnail of the first output.
            if !node.kind.outputs().is_empty() {
                let rows = node.kind.inputs().len().max(node.kind.outputs().len()) as f32;
                let top = rect.top() + (HEADER + rows * ROW + 4.0) * self.zoom;
                let side = THUMB * self.zoom;
                let thumb = Rect::from_min_size(pos2(rect.center().x - side / 2.0, top), vec2(side, side));
                match self.textures.get(&(id, 0)) {
                    Some(t) => {
                        painter.image(t.id(), thumb, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(thumb, 4, pal.header);
                    }
                }
            }
        }
        if let Some(id) = remove {
            graph.remove(id);
            if self.selected == Some(id) {
                self.selected = None;
            }
        }
        // The wire being drawn.
        if let Some((from, output)) = self.wiring {
            if let (Some(a), Some(p)) = (rects.get(&from), pointer) {
                wire(&painter, self.output_pos(*a, output as usize), p, pal.accent);
            }
            if released {
                if let Some((to, input)) = drop_on
                    && !graph.connect(from, output, to, input)
                {
                    ui.ctx().debug_text("That would make a loop");
                }
                self.wiring = None;
            }
        }
        // Delete the selected node with the keyboard.
        if let Some(id) = self.selected
            && ui.rect_contains_pointer(canvas)
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            graph.remove(id);
            self.selected = None;
        }
        if graph.nodes.is_empty() {
            painter.text(canvas.center(), egui::Align2::CENTER_CENTER, "Right-click to add a node", egui::FontId::proportional(14.0), pal.weak);
        }
    }

    fn inspector(&mut self, ui: &mut Ui, graph: &mut Graph, reg: &Registry) {
        let pal = Palette::of(ui);
        let Some(id) = self.selected.filter(|id| graph.node(*id).is_some()) else {
            ui.label(RichText::new("Terrain graph").bold().size(14.0));
            ui.label(
                RichText::new(
                    "Build terrain from nodes, as in World Machine or Gaea: generators make relief, modifiers and erosion shape it, \
                     selectors make masks, outputs send heights and terrains to the map. Select a node to edit it; right-click the canvas to add one.",
                )
                .size(11.5)
                .color(pal.weak),
            );
            return;
        };
        let node = graph.node_mut(id).expect("checked");
        ui.label(RichText::new(node.kind.label()).bold().size(14.0));
        ui.add_space(4.0);
        edit_kind(ui, &mut node.kind, reg);
        let outputs = node.kind.outputs();
        if !outputs.is_empty() {
            ui.add_space(8.0);
            if outputs.len() > 1 {
                ui.horizontal(|ui| {
                    for (k, name) in outputs.iter().enumerate() {
                        ui.selectable_value(&mut self.shown_output, k as u8, *name);
                    }
                });
            }
            let k = self.shown_output.min(outputs.len() as u8 - 1);
            let side = ui.available_width().min(260.0);
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            match self.textures.get(&(id, k)) {
                Some(t) => {
                    ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().rect_filled(rect, 4, pal.header);
                }
            }
            ui.label(RichText::new("The whole map, seen from above.").size(11.0).color(pal.weak));
        }
        ui.add_space(8.0);
        if ui.button(theme::with_icon(Icon::Trash2, "Delete node")).clicked() {
            graph.remove(id);
            self.selected = None;
        }
    }
}

/// The settings of a node.
fn edit_kind(ui: &mut Ui, kind: &mut NodeKind, reg: &Registry) {
    let row = |ui: &mut Ui, label: &str, add: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal(|ui| {
            ui.add_sized([96.0, 18.0], egui::Label::new(label));
            add(ui);
        });
    };
    fn drag(v: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>) -> egui::DragValue<'_> {
        egui::DragValue::new(v).speed(speed).range(range)
    }
    match kind {
        NodeKind::Noise {
            style,
            scale,
            octaves,
            roughness,
            height,
            base,
            seed,
        } => {
            row(ui, "Style", &mut |ui| {
                ui.selectable_value(style, NoiseStyle::Fbm, "Rolling");
                ui.selectable_value(style, NoiseStyle::Ridged, "Ridged");
                ui.selectable_value(style, NoiseStyle::Billow, "Billow");
            });
            row(ui, "Feature size", &mut |ui| {
                ui.add(drag(scale, 4.0, 8.0..=20000.0).suffix(" blocks"));
            });
            row(ui, "Detail", &mut |ui| {
                ui.add(egui::DragValue::new(octaves).range(1..=12).suffix(" octaves"));
            });
            row(ui, "Roughness", &mut |ui| {
                ui.add(drag(roughness, 0.01, 0.05..=0.95));
            });
            row(ui, "Height", &mut |ui| {
                ui.add(drag(height, 1.0, -400.0..=400.0).suffix(" blocks"));
            });
            row(ui, "Base", &mut |ui| {
                ui.add(drag(base, 1.0, -64.0..=319.0));
            });
            seed_row(ui, seed);
        }
        NodeKind::Cells {
            style,
            scale,
            jitter,
            height,
            base,
            seed,
        } => {
            row(ui, "Style", &mut |ui| {
                ui.selectable_value(style, CellStyle::Cones, "Cones");
                ui.selectable_value(style, CellStyle::Ridges, "Ridges");
            });
            row(ui, "Cell size", &mut |ui| {
                ui.add(drag(scale, 4.0, 8.0..=20000.0).suffix(" blocks"));
            });
            row(ui, "Jitter", &mut |ui| {
                ui.add(drag(jitter, 0.01, 0.0..=1.0));
            });
            row(ui, "Height", &mut |ui| {
                ui.add(drag(height, 1.0, -400.0..=400.0).suffix(" blocks"));
            });
            row(ui, "Base", &mut |ui| {
                ui.add(drag(base, 1.0, -64.0..=319.0));
            });
            seed_row(ui, seed);
        }
        NodeKind::Radial { inner, outer, height, base } => {
            row(ui, "Inner", &mut |ui| {
                ui.add(drag(inner, 0.01, 0.0..=1.5));
            });
            row(ui, "Outer", &mut |ui| {
                ui.add(drag(outer, 0.01, 0.0..=1.5));
            });
            row(ui, "Height", &mut |ui| {
                ui.add(drag(height, 0.5, -400.0..=400.0));
            });
            row(ui, "Base", &mut |ui| {
                ui.add(drag(base, 0.5, -400.0..=400.0));
            });
            hint(ui, "Height 1 and base 0 make a mask for Blend: 1 in the middle of the map, 0 at its edges.");
        }
        NodeKind::Constant { value } => row(ui, "Value", &mut |ui| {
            ui.add(drag(value, 0.5, -400.0..=400.0));
        }),
        NodeKind::Levels {
            in_min,
            in_max,
            out_min,
            out_max,
            gamma,
        } => {
            row(ui, "From", &mut |ui| {
                ui.add(drag(in_min, 0.5, -1000.0..=1000.0));
                ui.add(drag(in_max, 0.5, -1000.0..=1000.0));
            });
            row(ui, "To", &mut |ui| {
                ui.add(drag(out_min, 0.5, -1000.0..=1000.0));
                ui.add(drag(out_max, 0.5, -1000.0..=1000.0));
            });
            row(ui, "Gamma", &mut |ui| {
                ui.add(drag(gamma, 0.01, 0.05..=8.0));
            });
        }
        NodeKind::Terrace { steps, strength } => {
            row(ui, "Steps", &mut |ui| {
                ui.add(egui::DragValue::new(steps).range(1..=64));
            });
            row(ui, "Strength", &mut |ui| {
                ui.add(drag(strength, 0.01, 0.0..=1.0));
            });
        }
        NodeKind::Clamp { min, max } => {
            row(ui, "Between", &mut |ui| {
                ui.add(drag(min, 0.5, -1000.0..=1000.0));
                ui.add(drag(max, 0.5, -1000.0..=1000.0));
            });
        }
        NodeKind::Blur { radius } => row(ui, "Radius", &mut |ui| {
            ui.add(drag(radius, 0.5, 1.0..=512.0).suffix(" blocks"));
        }),
        NodeKind::Warp { amount, scale, seed } => {
            row(ui, "Amount", &mut |ui| {
                ui.add(drag(amount, 1.0, 0.0..=2000.0).suffix(" blocks"));
            });
            row(ui, "Feature size", &mut |ui| {
                ui.add(drag(scale, 4.0, 8.0..=20000.0).suffix(" blocks"));
            });
            seed_row(ui, seed);
        }
        NodeKind::Erosion {
            rain,
            erosion,
            deposition,
            capacity,
            evaporation,
            radius,
            seed,
        } => {
            row(ui, "Rain", &mut |ui| {
                ui.add(drag(rain, 0.05, 0.0..=10.0).suffix(" drops / sample"));
            });
            row(ui, "Erosion", &mut |ui| {
                ui.add(drag(erosion, 0.01, 0.0..=1.0));
            });
            row(ui, "Deposition", &mut |ui| {
                ui.add(drag(deposition, 0.01, 0.0..=1.0));
            });
            row(ui, "Capacity", &mut |ui| {
                ui.add(drag(capacity, 0.05, 0.0..=32.0));
            });
            row(ui, "Evaporation", &mut |ui| {
                ui.add(drag(evaporation, 0.001, 0.0..=0.5));
            });
            row(ui, "Brush radius", &mut |ui| {
                ui.add(drag(radius, 0.5, 1.0..=64.0).suffix(" blocks"));
            });
            seed_row(ui, seed);
            hint(ui, "Outputs the eroded heights, the flow of water and where soil settled (masks).");
        }
        NodeKind::Thermal { talus, iterations, strength } => {
            row(ui, "Talus", &mut |ui| {
                ui.add(drag(talus, 0.5, 1.0..=89.0).suffix("°"));
            });
            row(ui, "Iterations", &mut |ui| {
                ui.add(egui::DragValue::new(iterations).range(1..=500));
            });
            row(ui, "Strength", &mut |ui| {
                ui.add(drag(strength, 0.01, 0.0..=1.0));
            });
        }
        NodeKind::Combine { op, weight } => {
            row(ui, "Operation", &mut |ui| {
                egui::ComboBox::from_id_salt("combine op").selected_text(op.label()).show_ui(ui, |ui| {
                    for o in CombineOp::ALL {
                        ui.selectable_value(op, o, o.label());
                    }
                });
            });
            row(ui, "Weight of B", &mut |ui| {
                ui.add(drag(weight, 0.01, -10.0..=10.0));
            });
        }
        NodeKind::Blend => hint(ui, "A where the mask is 0, B where it is 1."),
        NodeKind::SelectHeight { min, max, falloff } => {
            row(ui, "Between", &mut |ui| {
                ui.add(drag(min, 0.5, -64.0..=1000.0));
                ui.add(drag(max, 0.5, -64.0..=1000.0));
            });
            row(ui, "Falloff", &mut |ui| {
                ui.add(drag(falloff, 0.5, 0.0..=200.0).suffix(" blocks"));
            });
        }
        NodeKind::SelectSlope { min, max, falloff } => {
            row(ui, "Between", &mut |ui| {
                ui.add(drag(min, 0.5, 0.0..=90.0).suffix("°"));
                ui.add(drag(max, 0.5, 0.0..=90.0).suffix("°"));
            });
            row(ui, "Falloff", &mut |ui| {
                ui.add(drag(falloff, 0.5, 0.0..=45.0).suffix("°"));
            });
        }
        NodeKind::Invert => hint(ui, "1 − the input: turns a mask inside out."),
        NodeKind::HeightOutput => hint(ui, "What reaches this node becomes the height of the ground."),
        NodeKind::TerrainOutput { terrain } => {
            row(ui, "Terrain", &mut |ui| {
                egui::ComboBox::from_id_salt("graph terrain")
                    .selected_text(&reg.terrain(TerrainId(*terrain)).name)
                    .height(300.0)
                    .show_ui(ui, |ui| {
                        for (i, t) in reg.terrains.iter().enumerate() {
                            ui.selectable_value(terrain, i as u16, &t.name);
                        }
                    });
            });
            hint(ui, "Painted where the mask is 1, scattered where it is between 0 and 1. Later outputs paint over earlier ones.");
        }
    }
}

fn seed_row(ui: &mut Ui, seed: &mut u32) {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 18.0], egui::Label::new("Seed"));
        ui.add(egui::DragValue::new(seed));
        if ui.small_button(theme::glyph(Icon::Dice5).to_string()).on_hover_text("Another seed").clicked() {
            *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 100_000;
        }
    });
}

fn hint(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(text).size(11.5).color(pal.weak));
}
