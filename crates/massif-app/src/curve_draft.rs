//! The Annotations tool's curves: points placed on the map, a curve through
//! them shown as the ribbon it will paint (from its start width to its end
//! width, in the colour picked), edited until it is committed: drag a point
//! (or, Bézier, a handle of the point picked) to move it, the curve to add a
//! point, right-click or Delete to take one off. Enter paints it as an
//! annotation (one undoable step, rubbed out by the same eraser, never
//! exported), Escape drops it.
//!
//! The curve itself is [`massif_core::curve::Curve`], kept apart from what
//! it is used for, so that the River and Road tools can follow one later.

use crate::doc::Document;
use crate::theme;
use crate::tools::{Feedback, ToolState};
use egui::{Color32, CursorIcon, Pos2, Rect, Stroke as Line, Ui, vec2};
use massif_core::curve::{Curve, CurvePoint, Interp};
use serde::{Deserialize, Serialize};

/// How near a point or a handle the pointer takes hold of it, in points.
const HANDLE_REACH: f32 = 10.0;

/// How the Annotations tool's curves are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurveSettings {
    pub interp: Interp,
    /// Width at the first point and at the last, in blocks.
    pub width: f32,
    pub end_width: f32,
}

impl Default for CurveSettings {
    fn default() -> Self {
        Self { interp: Interp::CatmullRom, width: 3.0, end_width: 3.0 }
    }
}

/// What is held on a curve being drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveHit {
    Point(usize),
    /// The handle of a point, on the side the curve leaves it (`true`) or
    /// comes in (`false`).
    Handle(usize, bool),
    /// The curve, on segment `i` at `t`.
    Course(usize, f32),
}

/// A curve being drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CurveDraft {
    pub curve: Curve,
    pub grab: Option<CurveHit>,
    /// The point last taken hold of: its handles show (Bézier), Delete
    /// takes it off.
    pub picked: Option<usize>,
}

impl CurveDraft {
    /// What the pointer at `p` is over; `screen` puts a block on the
    /// screen, `width` the ribbon's widest, in points.
    pub fn hit(&self, screen: &dyn Fn([f32; 2]) -> Pos2, width: f32, p: Pos2) -> Option<CurveHit> {
        let pts = &self.curve.points;
        if self.curve.interp == Interp::Bezier
            && let Some(k) = self.picked.filter(|&k| k < pts.len())
        {
            let h = self.curve.handle(k);
            for out in [true, false] {
                let s = if out { 1.0 } else { -1.0 };
                if screen([pts[k].at[0] + h[0] * s, pts[k].at[1] + h[1] * s]).distance(p) <= HANDLE_REACH {
                    return Some(CurveHit::Handle(k, out));
                }
            }
        }
        for (k, q) in pts.iter().enumerate().rev() {
            if screen(q.at).distance(p) <= HANDLE_REACH {
                return Some(CurveHit::Point(k));
            }
        }
        // The curve: nearest on screen, within half the ribbon or a few
        // points.
        let reach = (width / 2.0).max(6.0);
        let mut best: Option<(usize, f32, f32)> = None;
        for i in 0..pts.len().saturating_sub(1) {
            const N: usize = 40;
            for k in 0..=N {
                let t = k as f32 / N as f32;
                let d = screen(self.curve.at(i, t)).distance(p);
                if d <= reach && best.is_none_or(|b| d < b.2) {
                    best = Some((i, t, d));
                }
            }
        }
        best.map(|(i, t, _)| CurveHit::Course(i, t))
    }

    fn remove(&mut self, k: usize) {
        if k < self.curve.points.len() {
            self.curve.points.remove(k);
        }
        self.grab = None;
        self.picked = None;
    }

    /// Its length and points, in words.
    pub fn summary(&self) -> String {
        match self.curve.points.len() {
            0 | 1 => tr!("Click where the curve goes next").to_owned(),
            n => tr!("{length:.0} blocks · {n} points", length = self.curve.length(), n),
        }
    }
}

/// A press of a button on the map at block `at`, the pointer at `p`.
/// `zoom`: screen points a block.
pub fn press(tools: &mut ToolState, at: [f32; 2], p: Pos2, secondary: bool, screen: &dyn Fn([f32; 2]) -> Pos2, zoom: f32) -> Feedback {
    let width = tools.curve.width.max(tools.curve.end_width) * zoom;
    let interp = tools.curve.interp;
    let draft = tools.curve_draft.get_or_insert_with(|| CurveDraft { curve: Curve::new(interp), ..Default::default() });
    draft.curve.interp = interp;
    match (draft.hit(screen, width, p), secondary) {
        (Some(CurveHit::Handle(k, out)), false) => {
            // Taken hold of, a handle is the point's own from then on.
            let h = draft.curve.handle(k);
            draft.curve.points[k].handle = Some(h);
            draft.grab = Some(CurveHit::Handle(k, out));
        }
        (Some(CurveHit::Point(k)), false) => {
            draft.grab = Some(CurveHit::Point(k));
            draft.picked = Some(k);
        }
        (Some(CurveHit::Point(k)), true) => draft.remove(k),
        (Some(CurveHit::Course(i, t)), false) => {
            let k = draft.curve.insert(i, t);
            draft.grab = Some(CurveHit::Point(k));
            draft.picked = Some(k);
        }
        (None, false) => {
            draft.curve.points.push(CurvePoint::new(at));
            let k = draft.curve.points.len() - 1;
            draft.grab = Some(CurveHit::Point(k));
            draft.picked = Some(k);
        }
        _ => {}
    }
    if draft.curve.points.is_empty() {
        tools.curve_draft = None;
    }
    None
}

/// Each frame of a curve being drawn: what is held follows the pointer (at
/// block `at`); Enter paints the curve, Escape drops it, Delete takes off
/// the point picked.
pub fn follow(doc: &mut Document, tools: &mut ToolState, at: Option<[f32; 2]>, held: bool, ui: &Ui) -> Feedback {
    let interp = tools.curve.interp;
    let draft = tools.curve_draft.as_mut()?;
    draft.curve.interp = interp;
    if let Some(grab) = draft.grab {
        if held {
            if let Some(at) = at {
                match grab {
                    CurveHit::Point(k) if k < draft.curve.points.len() => draft.curve.points[k].at = at,
                    CurveHit::Handle(k, out) if k < draft.curve.points.len() => {
                        let a = draft.curve.points[k].at;
                        let s = if out { 1.0 } else { -1.0 };
                        draft.curve.points[k].handle = Some([(at[0] - a[0]) * s, (at[1] - a[1]) * s]);
                    }
                    _ => {}
                }
            }
            ui.ctx().request_repaint();
        } else {
            draft.grab = None;
        }
    }
    if ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    use crate::keymap::{Action, peek};
    let (enter, escape, delete) = (ui.input(|i| i.key_pressed(egui::Key::Enter)), peek(ui.ctx(), Action::Cancel), peek(ui.ctx(), Action::Delete));
    if enter {
        commit(doc, tools)
    } else if escape {
        tools.curve_draft = None;
        Some(tr!("Curve dropped").into())
    } else if delete {
        let k = draft.picked.unwrap_or(draft.curve.points.len().saturating_sub(1));
        draft.remove(k);
        if draft.curve.points.is_empty() {
            tools.curve_draft = None;
        }
        None
    } else {
        None
    }
}

/// Paint the curve being drawn as an annotation, in the colour picked (one
/// undoable step): from then on it is an annotation like any other.
pub fn commit(doc: &mut Document, tools: &mut ToolState) -> Feedback {
    let draft = tools.curve_draft.take()?;
    if draft.curve.points.len() < 2 {
        tools.curve_draft = Some(draft);
        return Some(tr!("A curve needs two points at least").into());
    }
    let ribbon = draft.curve.ribbon(tools.curve.width.max(0.0), tools.curve.end_width.max(0.0));
    doc.history.begin(tr!("Annotation curve"), doc.dim);
    massif_core::ops::paint_annotation_ribbon(&mut doc.history.edit(&mut doc.world, 0), &ribbon, tools.annotation.max(1));
    doc.commit();
    Some(tr!("Painted a curve of {length:.0} blocks", length = draft.curve.length()))
}

/// Draw the curve being drawn: its ribbon in its colour, its middle line,
/// its points and (Bézier) the handles of the point picked, and its bar;
/// before a first point, a hint. `hover`: the pointer, when over the map.
#[allow(clippy::too_many_arguments)]
pub fn draw(ui: &mut Ui, painter: &egui::Painter, canvas: Rect, id: egui::Id, screen: &dyn Fn([f32; 2]) -> Pos2, zoom: f32, hover: Option<Pos2>, doc: &mut Document, tools: &mut ToolState, feedback: &mut Option<String>) {
    let Some(draft) = &tools.curve_draft else {
        crate::road_draft::hint(painter, canvas, tr!("Click on the map: where the curve starts"));
        return;
    };
    let palette = &doc.world.registry.annotations;
    let n = tools.annotation.max(1);
    let rgb = palette.color(n).unwrap_or([255, 255, 255]);
    // The band at its real width, see-through enough for the land under
    // it to show.
    let alpha = (palette.opacity(n) as f32 / 100.0 * 0.6).clamp(0.15, 0.6);
    let fill = Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8);
    let curve = &draft.curve;
    if curve.points.len() >= 2 {
        // The ribbon: its edges either side of the middle line, as a strip
        // of triangles.
        let ribbon = curve.ribbon(tools.curve.width.max(0.0), tools.curve.end_width.max(0.0));
        let mut mid: Vec<(Pos2, f32)> = Vec::with_capacity(ribbon.len());
        for (k, &(p, w)) in ribbon.iter().enumerate() {
            let s = screen(p);
            if k + 1 == ribbon.len() || mid.last().is_none_or(|q| q.0.distance(s) >= 1.5) {
                mid.push((s, (w * zoom / 2.0).max(1.0)));
            }
        }
        if mid.len() >= 2 {
            let mut mesh = egui::Mesh::default();
            for k in 0..mid.len() {
                let (a, b) = (mid[k.saturating_sub(1)].0, mid[(k + 1).min(mid.len() - 1)].0);
                let d = (b - a).normalized();
                let normal = vec2(-d.y, d.x) * mid[k].1;
                mesh.colored_vertex(mid[k].0 + normal, fill);
                mesh.colored_vertex(mid[k].0 - normal, fill);
                if k > 0 {
                    let i = (2 * k) as u32;
                    mesh.add_triangle(i - 2, i - 1, i);
                    mesh.add_triangle(i - 1, i, i + 1);
                }
            }
            // Its edges, so that its width reads at any zoom: a dark line
            // in a light one along each side, and a hairline down its
            // middle.
            let edge = |side: f32| -> Vec<Pos2> {
                (0..mid.len())
                    .map(|k| {
                        let (a, b) = (mid[k.saturating_sub(1)].0, mid[(k + 1).min(mid.len() - 1)].0);
                        let d = (b - a).normalized();
                        mid[k].0 + vec2(-d.y, d.x) * mid[k].1 * side
                    })
                    .collect()
            };
            painter.add(egui::Shape::mesh(mesh));
            for side in [1.0, -1.0] {
                let e = edge(side);
                painter.add(egui::Shape::line(e.clone(), Line::new(2.5, Color32::from_black_alpha(120))));
                painter.add(egui::Shape::line(e, Line::new(1.0, Color32::from_white_alpha(230))));
            }
            let line: Vec<Pos2> = mid.iter().map(|m| m.0).collect();
            painter.add(egui::Shape::line(line, Line::new(1.0, Color32::from_white_alpha(110))));
        }
    }
    let hit = match (draft.grab, hover) {
        (None, Some(p)) => draft.hit(screen, tools.curve.width.max(tools.curve.end_width) * zoom, p),
        _ => None,
    };
    let accent = theme::accent();
    // The handles of the point picked, Bézier.
    if curve.interp == Interp::Bezier
        && let Some(k) = draft.picked.filter(|&k| k < curve.points.len())
    {
        let (a, h) = (curve.points[k].at, curve.handle(k));
        let ends = [screen([a[0] + h[0], a[1] + h[1]]), screen([a[0] - h[0], a[1] - h[1]])];
        painter.line_segment(ends, Line::new(1.0, Color32::WHITE));
        for (j, e) in ends.into_iter().enumerate() {
            let lit = matches!(hit.or(draft.grab), Some(CurveHit::Handle(i, out)) if i == k && out == (j == 0));
            painter.rect(Rect::from_center_size(e, vec2(8.0, 8.0) * if lit { 1.3 } else { 1.0 }), theme::TILE_RADIUS / 4, Color32::WHITE, Line::new(1.5, accent), egui::StrokeKind::Inside);
        }
    }
    for (k, q) in curve.points.iter().enumerate() {
        let p = screen(q.at);
        let lit = hit == Some(CurveHit::Point(k)) || draft.grab == Some(CurveHit::Point(k));
        let grow = if lit { 2.0 } else { 0.0 };
        painter.circle_filled(p + vec2(0.0, 1.0), 7.0 + grow, Color32::from_black_alpha(70));
        if draft.picked == Some(k) {
            painter.circle(p, 5.5 + grow, accent, Line::new(2.0, Color32::WHITE));
        } else {
            painter.circle(p, 4.5 + grow, Color32::WHITE, Line::new(2.0, accent));
        }
    }
    if hover.is_some() {
        ui.ctx().set_cursor_icon(match (draft.grab, hit) {
            (Some(_), _) => CursorIcon::Grabbing,
            (None, Some(CurveHit::Point(_) | CurveHit::Handle(..))) => CursorIcon::Grab,
            (None, Some(CurveHit::Course(..))) => CursorIcon::PointingHand,
            (None, None) => CursorIcon::Crosshair,
        });
    }
    match crate::road_draft::draft_bar(ui, painter, canvas, id, &draft.summary(), tr!("Paint curve"), curve.points.len() >= 2) {
        Some(true) => *feedback = commit(doc, tools),
        Some(false) => {
            tools.curve_draft = None;
            *feedback = Some(tr!("Curve dropped").into());
        }
        None => {}
    }
}
