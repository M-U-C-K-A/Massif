//! Selections on the map: the tools that draw them (rectangle, ellipse,
//! lasso, the selection brush), their options in Tool Settings, and the
//! outline the map shows, its "marching ants".
//!
//! The selection itself lives in the core (`massif_core::selection`): the
//! dimension holds it, every brush and whole-map filter keeps to it, and
//! changing it is a step to undo.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Tool, ToolState};
use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2, vec2};
use massif_core::filter::{Filter, Match};
use massif_core::selection::{Combine, Selection, Shape};
use massif_core::{BlockRect, Brush, TilePos};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// What "Select by" reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ByWhat {
    /// The terrain picked in the Terrain panel.
    #[default]
    Terrain,
    /// A band of heights.
    Height,
    /// Slopes steeper (or gentler) than an angle.
    Slope,
}

/// The settings of "Select by".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectBy {
    pub what: ByWhat,
    pub from: i32,
    pub to: i32,
    /// Degrees.
    pub slope: f32,
    /// Steeper than `slope` (else gentler).
    pub steeper: bool,
}

impl Default for SelectBy {
    fn default() -> Self {
        Self {
            what: ByWhat::Terrain,
            from: 62,
            to: 90,
            slope: 30.0,
            steeper: true,
        }
    }
}

impl SelectBy {
    /// The brush filter that picks the columns.
    pub fn filter(&self, terrain: u16) -> Filter {
        let mut f = Filter::default();
        match self.what {
            ByWhat::Terrain => f.only_on = Some(vec![Match::Terrain(terrain)]),
            ByWhat::Height => {
                f.above = Some(self.from.min(self.to));
                f.below = Some(self.from.max(self.to));
            }
            ByWhat::Slope => {
                f.slope = self.slope;
                f.slope_above = self.steeper;
                f.slope_below = !self.steeper;
            }
        }
        f
    }
}

/// What a shape does with the keys held: Shift or ⌘ (Ctrl) adds, Alt or
/// the right button takes out, both keep what both hold (as the keymap has
/// them); with neither, the mode chosen in Tool Settings.
pub fn combine_of(m: egui::Modifiers, secondary: bool, chosen: Combine) -> Combine {
    use crate::keymap::{Action, mods_hold};
    match (mods_hold(m, Action::SelectAdd), mods_hold(m, Action::SelectSubtract) || secondary) {
        (true, true) => Combine::Intersect,
        (true, false) => Combine::Add,
        (false, true) => Combine::Subtract,
        (false, false) => chosen,
    }
}

/// Every tile of the map, of every level: what "Select all" and "Invert"
/// select.
fn map_tiles(doc: &Document) -> Vec<TilePos> {
    let set: HashSet<TilePos> = doc.dimension().levels.iter().flat_map(|l| l.positions()).collect();
    set.into_iter().collect()
}

/// Select the whole map.
pub fn select_all(doc: &mut Document) -> bool {
    let all = Selection::default().invert(map_tiles(doc));
    doc.select(tr!("Select all"), Some(all))
}

/// Select what is not selected.
pub fn invert(doc: &mut Document) -> bool {
    let now = doc.selection().map(|s| (**s).clone()).unwrap_or_default();
    let inverted = now.invert(map_tiles(doc));
    doc.select(tr!("Invert selection"), Some(inverted))
}

/// Select nothing: the tools act everywhere again.
pub fn deselect(doc: &mut Document) -> bool {
    doc.select(tr!("Deselect"), None)
}

/// `shape` worked into the selection there is, as `op` says.
fn apply(doc: &mut Document, label: &str, shape: Selection, op: Combine) -> bool {
    let next = match (doc.selection(), op) {
        (Some(now), op) => now.combine(&shape, op),
        (None, Combine::Subtract | Combine::Intersect) => Selection::default(),
        (None, _) => shape,
    };
    doc.select(label, Some(next))
}

/// Points the pointer moves at most for a press and release to be a click.
const CLICK_TRAVEL: f32 = 3.0;

/// A shape being drawn on the map.
#[derive(Debug, Clone)]
struct Drag {
    tool: Tool,
    op: Combine,
    /// Drawn with the right button: a click alone clears the selection.
    secondary: bool,
    /// How far the pointer went from the start, in points: under a few, a
    /// click rather than a shape.
    travel: f32,
    start: (f64, f64),
    at: (f64, f64),
    /// The lasso's path.
    path: Vec<(f32, f32)>,
    /// What the selection brush has painted so far.
    painted: Selection,
    last_dab: Option<(f32, f32)>,
}

/// The outline worked out for a part of the map, kept while the view stays
/// in it.
struct Outline {
    selection: usize,
    step: i32,
    rect: BlockRect,
    edges: Vec<[(i32, i32); 2]>,
}

/// The selection's state on the map: the shape being drawn, the outline.
#[derive(Default)]
pub struct SelectView {
    drag: Option<Drag>,
    outline: Option<Outline>,
    painted_outline: Option<Outline>,
}

impl SelectView {
    pub fn is_drawing(&self) -> bool {
        self.drag.is_some()
    }

    /// Start a shape at `at` (blocks).
    pub fn begin(&mut self, tool: Tool, at: (f64, f64), op: Combine, secondary: bool, brush: &Brush) {
        let mut drag = Drag {
            tool,
            op,
            secondary,
            travel: 0.0,
            start: at,
            at,
            path: vec![(at.0 as f32, at.1 as f32)],
            painted: Selection::default(),
            last_dab: None,
        };
        if tool == Tool::SelectBrush {
            dab(&mut drag, brush, (at.0 as f32, at.1 as f32));
        }
        self.drag = Some(drag);
    }

    /// The pointer moved to `at`; `zoom`, the points a block takes.
    pub fn update(&mut self, at: (f64, f64), zoom: f32, brush: &Brush) {
        let Some(drag) = &mut self.drag else { return };
        drag.at = at;
        drag.travel = drag.travel.max(((at.0 - drag.start.0).hypot(at.1 - drag.start.1)) as f32 * zoom);
        let p = (at.0 as f32, at.1 as f32);
        match drag.tool {
            Tool::SelectLasso => {
                // A point every few points of the screen.
                let last = *drag.path.last().expect("begun with one");
                if ((p.0 - last.0).hypot(p.1 - last.1)) * zoom >= 3.0 {
                    drag.path.push(p);
                }
            }
            Tool::SelectBrush => {
                // Dabs a quarter of the brush apart along the way.
                let spacing = (brush.radius * 0.25).max(1.0);
                let from = drag.last_dab.unwrap_or(p);
                let d = (p.0 - from.0).hypot(p.1 - from.1);
                let n = (d / spacing).floor() as usize;
                for k in 1..=n {
                    let t = k as f32 * spacing / d;
                    dab(drag, brush, (from.0 + (p.0 - from.0) * t, from.1 + (p.1 - from.1) * t));
                }
            }
            _ => {}
        }
    }

    /// Let go: the shape changes the selection. Returns what to say.
    pub fn finish(&mut self, doc: &mut Document, feather: i32) -> Option<String> {
        let drag = self.drag.take()?;
        self.painted_outline = None;
        let (x0, x1) = (drag.start.0.min(drag.at.0), drag.start.0.max(drag.at.0));
        let (z0, z1) = (drag.start.1.min(drag.at.1), drag.start.1.max(drag.at.1));
        let rect = BlockRect::new(x0.floor() as i32, z0.floor() as i32, x1.ceil() as i32, z1.ceil() as i32);
        let shape = match drag.tool {
            Tool::SelectRect => Some(Shape::Rect(rect)),
            Tool::SelectEllipse => Some(Shape::Ellipse(rect)),
            Tool::SelectLasso => Some(Shape::Polygon(drag.path)),
            _ => None,
        };
        // A click (the pointer barely moved): no shape, not the block under it.
        let click = drag.tool != Tool::SelectBrush && drag.travel < CLICK_TRAVEL;
        let shape = match shape {
            _ if click => Selection::default(),
            Some(shape) => Selection::shape(&shape),
            None => drag.painted,
        };
        if shape.is_empty() {
            // A click without a shape (a new selection of nothing), or a
            // right-click: the selection is cleared.
            if (drag.op == Combine::Replace || drag.secondary) && doc.selection().is_some() {
                deselect(doc);
                return Some(tr!("Deselected").into());
            }
            return None;
        }
        let shape = if drag.tool == Tool::SelectBrush { shape } else { shape.feather(feather) };
        let label = match drag.op {
            Combine::Replace => tr!("Select"),
            Combine::Add => tr!("Add to selection"),
            Combine::Subtract => tr!("Subtract from selection"),
            Combine::Intersect => tr!("Intersect selection"),
        };
        apply(doc, label, shape, drag.op);
        Some(match doc.selection() {
            Some(s) => tr!("{thousands} blocks selected", thousands = thousands(s.area().round() as u64)),
            None => tr!("Nothing selected").into(),
        })
    }

    pub fn cancel(&mut self) {
        self.drag = None;
        self.painted_outline = None;
    }

    /// Draw the selection's outline and the shape being drawn. `land`: the
    /// painters of the map's land, which the selection's ants keep to;
    /// `to_screen` maps blocks to points; `view`, the blocks the canvas
    /// shows; `zoom`, the points a block takes.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, ui: &Ui, painter: &egui::Painter, land: &[egui::Painter], canvas: Rect, to_screen: &dyn Fn(f64, f64) -> Pos2, view: BlockRect, zoom: f32, selection: Option<&Arc<Selection>>) {
        let pal = Palette::of(ui);
        let still = theme::reduce_motion(ui.ctx());
        let time = ui.input(|i| i.time);
        // The ants march along: eight points of dash a second and a half.
        let phase = if still { 0.0 } else { (time * 12.0).rem_euclid(8.0) as f32 };
        let mut marching = false;
        if let Some(sel) = selection {
            let key = Arc::as_ptr(sel) as usize;
            if let Some(edges) = cached_edges(&mut self.outline, sel, key, view, zoom) {
                for lp in land {
                    marching |= ants(lp, canvas, to_screen, edges, phase, Color32::WHITE);
                }
            }
        }
        if let Some(drag) = &self.drag {
            let accent = pal.accent;
            let line = Stroke::new(1.5, accent);
            let under = Stroke::new(3.0, Color32::from_black_alpha(90));
            let corners = || {
                let a = to_screen(drag.start.0.floor().min(drag.at.0.floor()), drag.start.1.floor().min(drag.at.1.floor()));
                let b = to_screen(drag.start.0.ceil().max(drag.at.0.ceil()), drag.start.1.ceil().max(drag.at.1.ceil()));
                Rect::from_two_pos(a, b)
            };
            match drag.tool {
                Tool::SelectRect => {
                    let r = corners();
                    painter.rect_stroke(r, 0, under, egui::StrokeKind::Middle);
                    painter.rect_stroke(r, 0, line, egui::StrokeKind::Middle);
                }
                Tool::SelectEllipse => {
                    let r = corners();
                    let points: Vec<Pos2> = (0..72)
                        .map(|k| {
                            let t = k as f32 / 72.0 * std::f32::consts::TAU;
                            r.center() + vec2(t.cos() * r.width() / 2.0, t.sin() * r.height() / 2.0)
                        })
                        .collect();
                    painter.add(egui::Shape::closed_line(points.clone(), under));
                    painter.add(egui::Shape::closed_line(points, line));
                }
                Tool::SelectLasso => {
                    let points: Vec<Pos2> = drag.path.iter().map(|&(x, z)| to_screen(x as f64, z as f64)).chain(std::iter::once(to_screen(drag.at.0, drag.at.1))).collect();
                    if points.len() > 1 {
                        painter.add(egui::Shape::line(points.clone(), under));
                        painter.add(egui::Shape::line(points.clone(), line));
                        // Where it closes.
                        painter.add(egui::Shape::dashed_line(&[*points.last().unwrap(), points[0]], Stroke::new(1.0, accent), 4.0, 4.0));
                    }
                }
                _ => {
                    let painted = Arc::new(drag.painted.clone());
                    // Worked out again as it grows: the painted part is small.
                    self.painted_outline = None;
                    if let Some(edges) = cached_edges(&mut self.painted_outline, &painted, 0, view, zoom) {
                        ants(painter, canvas, to_screen, edges, 0.0, accent);
                    }
                }
            }
        }
        if marching && !still {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(66));
        }
    }
}

/// One dab of the selection brush, added to what it painted.
fn dab(drag: &mut Drag, brush: &Brush, at: (f32, f32)) {
    let rect = BlockRect::around(at.0, at.1, brush.reach());
    let w = brush.weigher();
    let spot = Selection::rasterize(rect, |z, x0, row| {
        for (k, v) in row.iter_mut().enumerate() {
            let x = x0 + k as i32;
            *v = (w.weight(x as f32 + 0.5 - at.0, z as f32 + 0.5 - at.1).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    drag.painted = drag.painted.combine(&spot, Combine::Add);
    drag.last_dab = Some(at);
}

/// The edges of `sel` around `view`, read about a block a point, worked
/// out again only when the view leaves what was worked out, or the zoom or
/// the selection changes.
fn cached_edges<'a>(cache: &'a mut Option<Outline>, sel: &Selection, key: usize, view: BlockRect, zoom: f32) -> Option<&'a [[(i32, i32); 2]]> {
    let bounds = sel.bounds()?;
    // A sample every `step` blocks: about one a point.
    let mut step = 1;
    while (step as f32) * zoom < 1.0 && step < 1 << 12 {
        step *= 2;
    }
    let view = view.intersect(&bounds.expand(step));
    if view.is_empty() {
        return None;
    }
    let fresh = cache.as_ref().is_some_and(|c| c.selection == key && c.step == step && c.rect.intersect(&view) == view);
    if !fresh {
        // Half a view more on every side, rounded to the samples.
        let (w, d) = (view.width() / 2 + step, view.depth() / 2 + step);
        let round = |v: i32| v.div_euclid(step) * step;
        let rect = BlockRect::new(round(view.x0 - w), round(view.z0 - d), round(view.x1 + w) + step, round(view.z1 + d) + step).intersect(&bounds.expand(2 * step));
        let edges = sel.edges(rect, step);
        *cache = Some(Outline { selection: key, step, rect, edges });
    }
    cache.as_ref().map(|c| c.edges.as_slice())
}

/// Marching ants along `edges`: a dark line, light dashes over it moved by
/// `phase` points. Returns whether any was drawn.
fn ants(painter: &egui::Painter, canvas: Rect, to_screen: &dyn Fn(f64, f64) -> Pos2, edges: &[[(i32, i32); 2]], phase: f32, ink: Color32) -> bool {
    let under = Stroke::new(1.0, Color32::from_black_alpha(200));
    let dash = Stroke::new(1.0, ink);
    let area = canvas.expand(2.0);
    // Past this many, the dashes would cost more than they show.
    let dashed = edges.len() <= 20_000;
    let mut drawn = false;
    for &[a, b] in edges {
        let (p, q) = (to_screen(a.0 as f64, a.1 as f64), to_screen(b.0 as f64, b.1 as f64));
        if !area.intersects(Rect::from_two_pos(p, q)) {
            continue;
        }
        let (p, q) = (pos2(p.x.round() + 0.5, p.y.round() + 0.5), pos2(q.x.round() + 0.5, q.y.round() + 0.5));
        painter.line_segment([p, q], under);
        if dashed {
            // The pattern runs on from one edge to the next: it is set by
            // where on the screen the edge starts.
            let along = if a.1 == b.1 { p.x } else { p.y };
            let offset = (along + phase).rem_euclid(8.0);
            painter.extend(egui::Shape::dashed_line_with_offset(&[p, q], dash, &[4.0], &[4.0], offset));
        }
        drawn = true;
    }
    drawn
}

/// 12345678 as "12,345,678".
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Set in the context's data when Export as Schematic is clicked in Tool
/// Settings, for the app to open its dialog.
pub const EXPORT_SCHEMATIC: &str = "export the selection as a schematic";

/// The options of the selection tools, in Tool Settings.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut crate::blocks::BlockLibrary) {
    use crate::panels::{hint, section};
    use crate::widgets::{Field, button_group, row, well};
    let modes = Combine::ALL;
    let labels = modes.map(|m| crate::i18n::tr_name(m.label()));
    let on = modes.map(|m| tools.select_mode == m);
    let any = doc.selection().is_some();
    if let Some(k) = button_group(ui, "selection mode", &labels, &on) {
        tools.select_mode = modes[k];
    }
    // Clearing it, right under what shapes do: where it is looked for.
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        // Each button whole: one that does not fit goes to the next line,
        // rather than fold its words inside it (Invert in Russian, in a
        // wide typeface, at the narrowest sidebar).
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if ui
            .add_enabled(any, egui::Button::new(theme::with_icon(Icon::SquareX, tr!("Clear"))))
            .on_hover_text(if cfg!(target_os = "macos") { tr!("Clear the selection: the tools act on the whole map again (Esc, a right-click, ⌘D)") } else { tr!("Clear the selection: the tools act on the whole map again (Esc, a right-click, Ctrl+D)") })
            .clicked()
        {
            deselect(doc);
        }
        if crate::widgets::button(ui, theme::with_icon(Icon::SquareDashedMousePointer, tr!("All"))).clicked() {
            select_all(doc);
        }
        if ui.add_enabled(any, egui::Button::new(theme::with_icon(Icon::SquaresExclude, tr!("Invert")))).clicked() {
            invert(doc);
        }
    });
    hint(
        ui,
        if cfg!(target_os = "macos") {
            tr!("Shift or ⌘ + drag adds to the selection, Alt + drag takes out of it, both keep what both hold. A right-click or Esc clears it.")
        } else {
            tr!("Shift or Ctrl + drag adds to the selection, Alt + drag takes out of it, both keep what both hold. A right-click or Esc clears it.")
        },
    );
    ui.add_space(4.0);
    if tools.tool != Tool::SelectBrush {
        well(ui, |ui| {
            row(ui, tr!("Feather"), |ui| {
                ui.add(Field::new(&mut tools.select_feather, 0..=massif_core::selection::MAX_FEATHER).suffix(tr!(" blocks")).width(120.0));
            });
        });
        hint(ui, tr!("New shapes fade over this many blocks on either side of their edge."));
    } else {
        hint(ui, tr!("The brush's soft edge feathers what it paints."));
    }

    section(ui, Icon::SquareDashed, tr!("Selection"));
    match doc.selection() {
        Some(s) => {
            let area = s.area();
            hint(ui, &tr!("{thousands} blocks selected. The tools act only inside, fading along a feathered edge.", thousands = thousands(area.round() as u64)));
        }
        None => hint(ui, tr!("Nothing selected: the tools act on the whole map.")),
    }
    if any && tools.tool != Tool::SelectBrush {
        let n = tools.select_feather;
        let label = if n > 0 { tr!("Feather it by {n}", n) } else { tr!("Feather it").to_owned() };
        let feather = ui.add_enabled(n > 0, egui::Button::new(theme::with_icon(Icon::Feather, &label))).on_disabled_hover_text(tr!("Set Feather above first"));
        if feather.clicked()
            && let Some(s) = doc.selection().cloned()
        {
            doc.select(tr!("Feather selection"), Some(s.feather(n)));
        }
    }
    if any && crate::widgets::button(ui, theme::with_icon(Icon::Box, tr!("Export as Schematic…"))).on_hover_text(tr!("The selected part of the map as a WorldEdit or Litematica schematic, with the materials it holds")).clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(EXPORT_SCHEMATIC), true));
    }

    section(ui, Icon::ScanLine, tr!("Select by"));
    let by = &mut tools.select_by;
    let whats = [ByWhat::Terrain, ByWhat::Height, ByWhat::Slope];
    if let Some(k) = button_group(ui, "select by", &[tr!("Terrain"), tr!("Height"), tr!("Slope")], &whats.map(|w| by.what == w)) {
        by.what = whats[k];
    }
    ui.add_space(4.0);
    let (lo, hi) = (doc.dimension().min_y, doc.dimension().max_y - 1);
    well(ui, |ui| match by.what {
        ByWhat::Terrain => {
            let name = doc.world.registry.terrain(massif_core::TerrainId(tools.terrain)).name.clone();
            row(ui, tr!("Terrain"), |ui| {
                ui.label(egui::RichText::new(name).strong());
                let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::hover());
                crate::panels::terrain_icon(ui, r, &doc.world.registry, tools.terrain, blocks);
            });
        }
        ByWhat::Height => {
            row(ui, tr!("From"), |ui| {
                ui.add(Field::new(&mut by.from, lo..=hi).width(120.0));
            });
            row(ui, tr!("To"), |ui| {
                ui.add(Field::new(&mut by.to, lo..=hi).width(120.0));
            });
        }
        ByWhat::Slope => {
            row(ui, tr!("Slopes"), |ui| {
                let on = [by.steeper, !by.steeper];
                if let Some(k) = button_group(ui, "steeper or gentler", &[tr!("Steeper"), tr!("Gentler")], &on) {
                    by.steeper = k == 0;
                }
            });
            row(ui, tr!("Than"), |ui| {
                ui.add(Field::new(&mut by.slope, 0.0..=89.0).suffix("°").max_decimals(0).width(120.0));
            });
        }
    });
    if by.what == ByWhat::Terrain {
        hint(ui, tr!("The terrain picked in Terrain or Blocks."));
    }
    ui.add_space(4.0);
    if crate::widgets::button(ui, theme::with_icon(Icon::ScanLine, tr!("Select"))).on_hover_text(tr!("Select the columns of the whole map that match, as the mode above says")).clicked() {
        let filter = by.filter(tools.terrain);
        let shape = Selection::by_filter(doc.dimension(), &filter);
        apply(doc, tr!("Select by"), shape, tools.select_mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        let world = massif_core::generate::generate(&massif_core::generate::GenParams {
            tiles_x: 2,
            tiles_z: 2,
            seed: 3,
            ..Default::default()
        });
        Document::new(world, None, 1)
    }

    #[test]
    fn shapes_drawn_on_the_map_select_and_undo() {
        let mut d = doc();
        let mut view = SelectView::default();
        let brush = Brush::default();
        view.begin(Tool::SelectRect, (-10.2, -10.0), Combine::Replace, false, &brush);
        view.update((30.0, 20.5), 1.0, &brush);
        assert!(view.finish(&mut d, 0).is_some());
        assert_eq!(d.selection().unwrap().area(), 41.0 * 31.0);
        // Shift adds an ellipse; Alt takes a lasso's triangle out.
        view.begin(Tool::SelectEllipse, (100.0, 0.0), combine_of(egui::Modifiers::SHIFT, false, Combine::Replace), false, &brush);
        view.update((140.0, 40.0), 1.0, &brush);
        view.finish(&mut d, 0);
        let both = d.selection().unwrap().area();
        assert!(both > 41.0 * 31.0 + 1000.0, "{both}");
        view.begin(Tool::SelectLasso, (-20.0, -20.0), combine_of(egui::Modifiers::ALT, false, Combine::Replace), false, &brush);
        for p in [(10.0, -20.0), (10.0, 10.0), (-20.0, 10.0)] {
            view.update(p, 1.0, &brush);
        }
        view.finish(&mut d, 0);
        assert!(d.selection().unwrap().get(0, 0) == 0);
        assert!(d.selection().unwrap().get(20, 15) > 0);
        // Inverted, then deselected; each a step to undo.
        assert!(invert(&mut d));
        assert_eq!(d.selection().unwrap().get(0, 0), 255);
        assert!(deselect(&mut d));
        assert!(d.selection().is_none());
        d.undo();
        d.undo();
        assert_eq!(d.selection().unwrap().get(0, 0), 0);
        // A click with nothing drawn deselects.
        view.begin(Tool::SelectRect, (5.0, 5.0), Combine::Replace, false, &brush);
        view.finish(&mut d, 0);
        assert!(d.selection().is_none());
    }

    /// ⌘ (Ctrl) adds as Shift does, Alt takes out, and a right-click with
    /// nothing drawn clears the selection, as a left click in the New mode
    /// does; a right-click in the Add mode too.
    #[test]
    fn command_adds_alt_subtracts_a_right_click_clears() {
        let mut d = doc();
        let mut view = SelectView::default();
        let brush = Brush::default();
        let rect = |view: &mut SelectView, d: &mut Document, from: (f64, f64), to: (f64, f64), mods: egui::Modifiers, secondary: bool| {
            view.begin(Tool::SelectRect, from, combine_of(mods, secondary, Combine::Replace), secondary, &brush);
            view.update(to, 1.0, &brush);
            view.finish(d, 0)
        };
        rect(&mut view, &mut d, (0.0, 0.0), (20.0, 20.0), egui::Modifiers::NONE, false);
        assert_eq!(d.selection().unwrap().area(), 400.0);
        rect(&mut view, &mut d, (40.0, 0.0), (60.0, 20.0), egui::Modifiers::COMMAND, false);
        assert_eq!(d.selection().unwrap().area(), 800.0, "⌘ adds");
        rect(&mut view, &mut d, (0.0, 0.0), (10.0, 20.0), egui::Modifiers::ALT, false);
        assert_eq!(d.selection().unwrap().area(), 600.0, "Alt takes out");
        // A right-drag takes out too; a right-click alone clears.
        rect(&mut view, &mut d, (40.0, 0.0), (50.0, 20.0), egui::Modifiers::NONE, true);
        assert_eq!(d.selection().unwrap().area(), 400.0);
        assert_eq!(rect(&mut view, &mut d, (5.0, 5.0), (5.0, 5.0), egui::Modifiers::NONE, true).as_deref(), Some("Deselected"));
        assert!(d.selection().is_none());
        // Undone, it comes back.
        d.undo();
        assert_eq!(d.selection().unwrap().area(), 400.0);
        // In the Add mode a left click keeps the selection; a right-click clears it.
        view.begin(Tool::SelectRect, (5.0, 5.0), combine_of(egui::Modifiers::NONE, false, Combine::Add), false, &brush);
        view.finish(&mut d, 0);
        assert!(d.selection().is_some());
        view.begin(Tool::SelectRect, (5.0, 5.0), combine_of(egui::Modifiers::NONE, true, Combine::Add), true, &brush);
        view.finish(&mut d, 0);
        assert!(d.selection().is_none());
    }

    #[test]
    fn the_selection_brush_paints_it_softly() {
        let mut d = doc();
        let mut view = SelectView::default();
        let brush = Brush {
            radius: 10.0,
            ..Brush::default()
        };
        view.begin(Tool::SelectBrush, (0.0, 0.0), Combine::Add, false, &brush);
        view.update((60.0, 0.0), 1.0, &brush);
        view.finish(&mut d, 0);
        let s = d.selection().unwrap();
        // Full along the stroke, fading to its sides, nothing past them.
        assert!(s.get(30, 0) >= 245, "{}", s.get(30, 0));
        assert!(s.get(30, 6) < s.get(30, 0));
        assert_eq!(s.get(30, 30), 0);
        assert!(select_all(&mut d));
        assert_eq!(d.selection().unwrap().area(), 4.0 * 128.0 * 128.0);
    }

    #[test]
    fn select_by_a_height_band() {
        let mut d = doc();
        let by = SelectBy {
            what: ByWhat::Height,
            from: 60,
            to: 70,
            ..SelectBy::default()
        };
        let shape = Selection::by_filter(d.dimension(), &by.filter(1));
        apply(&mut d, "Select by", shape, Combine::Replace);
        let dim = d.dimension();
        let s = dim.selection.as_ref().unwrap();
        for (x, z) in [(-100, -100), (10, 30), (100, 50), (-30, 90)] {
            let h = massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top).unwrap();
            let y = massif_core::compose::block_top(h);
            assert_eq!(s.get(x, z) == 255, (60..=70).contains(&y), "{x}, {z}: {y}");
        }
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(12_345_678), "12,345,678");
    }
}
