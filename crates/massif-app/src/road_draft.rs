//! The Road tool: points clicked on the map, a smooth road through them to
//! edit (drag a point, drag the road to add one, right-click or Delete to
//! take one off), Enter to build it ([`massif_core::road`]), Escape to drop
//! it. Its settings, in Tool Settings.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Feedback, ToolState};
use egui::{Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke as Line, StrokeKind, Ui, pos2, vec2};
use massif_core::registry::{Registry, TerrainId};
use massif_core::road::{RoadPlan, RoadSpec};
use serde::{Deserialize, Serialize};

/// How near a point of the road the pointer takes hold of it, in points.
const HANDLE_REACH: f32 = 11.0;

/// The surfaces offered first, from the material mixes: added to the map
/// when a road is built with one.
pub const SURFACES: [&str; 5] = [tn!("Trodden path"), tn!("Gravel mix"), tn!("Cobblestone mix"), tn!("Stone bricks"), tn!("Wooden deck")];
/// The decks of bridges.
pub const DECKS: [&str; 2] = [tn!("Wooden deck"), tn!("Stone bricks")];

/// What a road is paved with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoadSurface {
    /// The ground left as it is.
    Keep,
    /// A terrain by name: one of the map's, or one of [`SURFACES`].
    Terrain(String),
    /// The noise of the Paint tool.
    Noise,
}

/// The settings of the Road tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoadSettings {
    /// Width of the way, in blocks.
    pub width: f32,
    /// Width of each bank, in blocks.
    pub banks: f32,
    /// The steepest it climbs, in percent (blocks up per 100 along).
    pub slope: f32,
    pub surface: RoadSurface,
    /// Cross water on a deck rather than filling it.
    pub bridges: bool,
    /// The deck's material, one of [`DECKS`].
    pub deck: String,
}

impl Default for RoadSettings {
    fn default() -> Self {
        Self {
            width: 5.0,
            banks: 6.0,
            slope: 12.0,
            surface: RoadSurface::Terrain(tn!("Trodden path").into()),
            bridges: true,
            deck: DECKS[0].into(),
        }
    }
}

/// The terrain called `name` in `reg`, the mix of that name added if the
/// map has none.
fn terrain_named(reg: &mut Registry, name: &str) -> Option<TerrainId> {
    if let Some(t) = reg.terrain_by_name(name) {
        return Some(t);
    }
    massif_core::packs::MATERIAL_MIXES.iter().find(|m| m.name == name).map(|m| m.install(reg))
}

impl RoadSettings {
    /// How the road is built, leaving the deck to [`Self::resolve`].
    pub fn spec(&self) -> RoadSpec {
        RoadSpec {
            width: self.width.max(1.0),
            banks: self.banks.max(0.0),
            grade: (self.slope / 100.0).max(0.005),
            bridges: self.bridges,
            deck: None,
        }
    }

    /// The spec with its deck, and the terrain of the surface, the mixes
    /// they name added to the map if need be.
    fn resolve(&self, reg: &mut Registry) -> (RoadSpec, Option<TerrainId>) {
        let mut spec = self.spec();
        if spec.bridges {
            spec.deck = terrain_named(reg, &self.deck);
        }
        let surface = match &self.surface {
            RoadSurface::Terrain(name) => terrain_named(reg, name),
            _ => None,
        };
        (spec, surface)
    }
}

/// A road being laid out: its points, and the road they give.
#[derive(Debug, Clone, Default)]
pub struct RoadDraft {
    pub points: Vec<(f32, f32)>,
    pub plan: RoadPlan,
    /// Where each point lies on the course (indices into it).
    pub stops: Vec<usize>,
    /// The point being dragged, and the one last taken hold of (Delete
    /// takes it off).
    pub grab: Option<usize>,
    pub picked: Option<usize>,
    planned: Option<u64>,
}

impl PartialEq for RoadDraft {
    fn eq(&self, other: &Self) -> bool {
        self.points == other.points
    }
}

/// What the pointer is over, on a road being laid out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoadHit {
    Point(usize),
    /// The road, at this point of its course.
    Course(usize),
}

impl RoadDraft {
    /// Work out the road again if its points, its settings or the map
    /// changed.
    pub fn plan(&mut self, doc: &Document, settings: &RoadSettings) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}{:?}", self.points, settings.spec()).hash(&mut h);
        (doc.revision, doc.dim).hash(&mut h);
        let key = h.finish();
        if self.planned == Some(key) {
            return;
        }
        self.plan = massif_core::road::plan_road(doc.dimension(), &self.points, &settings.spec());
        // Each point's place on the course, in order along it.
        let course = &self.plan.course;
        let mut from = 0;
        self.stops = self
            .points
            .iter()
            .map(|p| {
                let best = (from..course.len()).min_by(|&a, &b| {
                    let d = |k: usize| (course[k].0 - p.0).hypot(course[k].1 - p.1);
                    d(a).total_cmp(&d(b))
                });
                from = best.unwrap_or(from);
                from
            })
            .collect();
        self.planned = Some(key);
    }

    /// Its length, steepest climb and bridges, in words.
    pub fn summary(&self) -> String {
        if self.points.len() < 2 {
            return tr!("Click where the road goes next").to_owned();
        }
        let mut s = tr!("{length:.0} blocks · steepest {value:.0}%", length = self.plan.length(), value = self.plan.steepest * 100.0);
        let bridged = self.plan.bridged();
        if bridged >= 1.0 {
            s += &tr!(" · {bridged:.0} on bridges", bridged);
        }
        if !self.plan.within {
            s += tr!(" · too steep between bridges");
        }
        s
    }

    /// What the pointer at `p` is over; `screen` puts a block on the screen.
    pub fn hit(&self, screen: &dyn Fn((f32, f32)) -> Pos2, width: f32, p: Pos2) -> Option<RoadHit> {
        for (k, &a) in self.points.iter().enumerate().rev() {
            if screen(a).distance(p) <= HANDLE_REACH {
                return Some(RoadHit::Point(k));
            }
        }
        let reach = (width / 2.0).max(7.0);
        let mut best: Option<(usize, f32)> = None;
        for (i, &c) in self.plan.course.iter().enumerate() {
            let d = screen(c).distance(p);
            if d <= reach && best.is_none_or(|b| d < b.1) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| RoadHit::Course(i))
    }

    /// Take point `k` off.
    fn remove(&mut self, k: usize) {
        if k < self.points.len() {
            self.points.remove(k);
        }
        self.grab = None;
        self.picked = None;
    }
}

/// A press of a button on the map at block `at`, the pointer at `p`.
/// `zoom`: screen points a block.
pub fn press(doc: &Document, tools: &mut ToolState, at: (f32, f32), p: Pos2, secondary: bool, screen: &dyn Fn((f32, f32)) -> Pos2, zoom: f32) -> Feedback {
    let width = tools.road.width * zoom;
    let draft = tools.road_draft.get_or_insert_with(RoadDraft::default);
    let hit = draft.hit(screen, width, p);
    match (hit, secondary) {
        (Some(RoadHit::Point(k)), false) => {
            draft.grab = Some(k);
            draft.picked = Some(k);
        }
        (Some(RoadHit::Point(k)), true) => draft.remove(k),
        // The road itself, held: a point it passes, to drag.
        (Some(RoadHit::Course(i)), false) => {
            let k = draft.stops.iter().filter(|&&s| s <= i).count().clamp(1, draft.points.len());
            draft.points.insert(k, at);
            draft.grab = Some(k);
            draft.picked = Some(k);
        }
        (None, false) => {
            draft.points.push(at);
            let k = draft.points.len() - 1;
            draft.grab = Some(k);
            draft.picked = Some(k);
        }
        _ => {}
    }
    if draft.points.is_empty() {
        tools.road_draft = None;
        return None;
    }
    draft.plan(doc, &tools.road);
    None
}

/// Each frame of the Road tool: a held point follows the pointer (at block
/// `at`); Enter builds the road, Escape drops it, Delete takes off the
/// point last taken hold of.
pub fn follow(doc: &mut Document, tools: &mut ToolState, at: Option<(f32, f32)>, held: bool, ui: &Ui) -> Feedback {
    let draft = tools.road_draft.as_mut()?;
    if let Some(k) = draft.grab {
        if held {
            if let (Some(at), true) = (at, k < draft.points.len()) {
                draft.points[k] = at;
            }
            ui.ctx().request_repaint();
        } else {
            draft.grab = None;
        }
    }
    draft.plan(doc, &tools.road);
    if ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    use crate::keymap::{Action, peek};
    let (enter, escape, delete) = (ui.input(|i| i.key_pressed(egui::Key::Enter)), peek(ui.ctx(), Action::Cancel), peek(ui.ctx(), Action::Delete));
    if enter {
        build(doc, tools)
    } else if escape {
        tools.road_draft = None;
        Some(tr!("Road dropped").into())
    } else if delete {
        let k = draft.picked.unwrap_or(draft.points.len().saturating_sub(1));
        draft.remove(k);
        if draft.points.is_empty() {
            tools.road_draft = None;
        } else {
            draft.plan(doc, &tools.road);
        }
        None
    } else {
        None
    }
}

/// Build the road being laid out (one undoable step).
pub fn build(doc: &mut Document, tools: &mut ToolState) -> Feedback {
    let mut draft = tools.road_draft.take()?;
    draft.plan(doc, &tools.road);
    if draft.points.len() < 2 || draft.plan.course.len() < 3 {
        tools.road_draft = Some(draft);
        let note = tr!("A road needs two points at least").to_owned();
        tools.road_note = Some(note.clone());
        return Some(note);
    }
    let (spec, surface) = tools.road.resolve(&mut doc.world.registry);
    let noise = matches!(tools.road.surface, RoadSurface::Noise).then(|| tools.pattern.sampler(&doc.world.registry)).flatten();
    let paint = |x: i32, z: i32| match &noise {
        Some(n) => n.terrain(x, z),
        None => surface,
    };
    let filter = tools.filter.clone();
    doc.history.begin(tr!("Road"), doc.dim);
    let report = massif_core::road::build_road(&mut doc.history, &mut doc.world, doc.dim, &draft.plan, &spec, &paint, Some(&filter));
    doc.commit();
    let mut note = tr!("Built a road of {length:.0} blocks · {cut:.0} blocks cut, {fill:.0} filled", length = draft.plan.length(), cut = report.cut, fill = report.fill);
    if report.deck > 0 {
        note += &tr!(" · {bridged:.0} on bridges", bridged = draft.plan.bridged());
    }
    tools.road_note = Some(note.clone());
    Some(note)
}

/// Draw the road being laid out, its handles and its bar; before a first
/// point, a hint. `hover`: the pointer, when it is over the map.
#[allow(clippy::too_many_arguments)]
pub fn draw(ui: &mut Ui, painter: &egui::Painter, canvas: Rect, id: egui::Id, screen: &dyn Fn((f32, f32)) -> Pos2, zoom: f32, hover: Option<Pos2>, doc: &mut Document, tools: &mut ToolState, feedback: &mut Option<String>) {
    let Some(draft) = &tools.road_draft else {
        hint(painter, canvas, tr!("Click on the map: where the road starts"));
        return;
    };
    let settings = &tools.road;
    let width = settings.width.max(1.0) * zoom;
    let plan = &draft.plan;
    let n = plan.course.len();
    if n >= 2 {
        // Points a pixel and a half apart at least, each with whether it is
        // on a bridge.
        let mut pts: Vec<(Pos2, bool)> = Vec::with_capacity(n.min(4096));
        for (k, &c) in plan.course.iter().enumerate() {
            let p = screen(c);
            if k + 1 == n || pts.last().is_none_or(|q| q.0.distance(p) >= 1.5) {
                pts.push((p, plan.bridge.get(k).copied().unwrap_or(false)));
            }
        }
        let road = Color32::from_rgb(226, 210, 168);
        let deck = Color32::from_rgb(176, 124, 70);
        let banks = width + 2.0 * settings.banks * zoom;
        let all: Vec<Pos2> = pts.iter().map(|p| p.0).collect();
        painter.add(egui::Shape::line(all.clone(), Line::new(banks, Color32::from_white_alpha(26))));
        painter.add(egui::Shape::line(all.clone(), Line::new(width.max(2.5) + 2.0, Color32::from_black_alpha(150))));
        painter.add(egui::Shape::line(all, Line::new(width.max(2.5), road.gamma_multiply(0.92))));
        // Bridges over it, in the colour of their deck.
        let mut run: Vec<Pos2> = Vec::new();
        for (k, &(p, on)) in pts.iter().enumerate() {
            if on {
                run.push(p);
            }
            if (!on || k + 1 == pts.len()) && run.len() >= 2 {
                painter.add(egui::Shape::line(std::mem::take(&mut run), Line::new(width.max(2.5), deck)));
            } else if !on {
                run.clear();
            }
        }
    }
    let hit = match (draft.grab, hover) {
        (None, Some(p)) => draft.hit(screen, width, p),
        _ => None,
    };
    let accent = theme::accent();
    for (k, &a) in draft.points.iter().enumerate() {
        let p = screen(a);
        let lit = hit == Some(RoadHit::Point(k)) || draft.grab == Some(k);
        let grow = if lit { 2.0 } else { 0.0 };
        painter.circle_filled(p + vec2(0.0, 1.0), 8.0 + grow, Color32::from_black_alpha(70));
        if draft.picked == Some(k) {
            painter.circle(p, 6.0 + grow, accent, Line::new(2.0, Color32::WHITE));
        } else {
            painter.circle(p, 5.0 + grow, Color32::WHITE, Line::new(2.0, accent));
        }
    }
    if hover.is_some() {
        ui.ctx().set_cursor_icon(match (draft.grab, hit) {
            (Some(_), _) => CursorIcon::Grabbing,
            (None, Some(RoadHit::Point(_))) => CursorIcon::Grab,
            (None, Some(RoadHit::Course(_))) => CursorIcon::PointingHand,
            (None, None) => CursorIcon::Crosshair,
        });
    }
    match bar(ui, painter, canvas, id, &draft.summary(), draft.points.len() >= 2) {
        Some(true) => *feedback = build(doc, tools),
        Some(false) => {
            tools.road_draft = None;
            *feedback = Some(tr!("Road dropped").into());
        }
        None => {}
    }
}

/// A hint at the foot of the map.
fn hint(painter: &egui::Painter, canvas: Rect, text: &str) {
    let at = canvas.center_bottom() - vec2(0.0, 22.0);
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.5), Color32::from_gray(235));
    let back = Rect::from_center_size(at, galley.size() + vec2(24.0, 12.0));
    painter.rect_filled(back.translate(vec2(0.0, 2.0)), theme::TILE_RADIUS, Color32::from_black_alpha(60));
    painter.rect(back, theme::TILE_RADIUS, Color32::from_rgba_unmultiplied(20, 22, 27, 235), Line::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
    painter.galley(back.min + vec2(12.0, 6.0), galley, Color32::from_gray(235));
}

/// The bar over a road being laid out: what it gives, Build road and
/// Cancel. `Some(true)`: build it; `Some(false)`: drop it.
fn bar(ui: &mut Ui, painter: &egui::Painter, canvas: Rect, id: egui::Id, summary: &str, ready: bool) -> Option<bool> {
    let font = FontId::proportional(12.5);
    let text = painter.layout_no_wrap(summary.to_owned(), font.clone(), Color32::from_gray(225));
    let go = painter.layout_no_wrap(tr!("Build road").to_owned(), font.clone(), Color32::WHITE);
    let enter = painter.layout_no_wrap("⏎".to_owned(), FontId::proportional(11.0), Color32::from_white_alpha(170));
    let cancel = painter.layout_no_wrap(tr!("Cancel").to_owned(), font, Color32::from_gray(210));
    let esc = painter.layout_no_wrap("esc".to_owned(), FontId::proportional(10.5), Color32::from_gray(130));
    let (pad, gap, h) = (12.0, 8.0, 28.0);
    let go_w = go.size().x + enter.size().x + 26.0;
    let cancel_w = cancel.size().x + esc.size().x + 24.0;
    let width = pad + text.size().x + 16.0 + go_w + gap + cancel_w + 6.0;
    let bar = Rect::from_center_size(canvas.center_bottom() - vec2(0.0, 30.0), vec2(width, h + 12.0));
    painter.rect_filled(bar.translate(vec2(0.0, 3.0)).expand(1.0), theme::CARD_RADIUS, Color32::from_black_alpha(70));
    painter.rect(bar, theme::CARD_RADIUS, Color32::from_rgba_unmultiplied(19, 21, 26, 242), Line::new(1.0, Color32::from_white_alpha(20)), StrokeKind::Inside);
    let mid = bar.center().y;
    painter.galley(pos2(bar.min.x + pad, mid - text.size().y / 2.0), text.clone(), Color32::from_gray(225));
    let go_rect = Rect::from_min_size(pos2(bar.min.x + pad + text.size().x + 16.0, mid - h / 2.0), vec2(go_w, h));
    let cancel_rect = Rect::from_min_size(pos2(go_rect.max.x + gap, mid - h / 2.0), vec2(cancel_w, h));
    let go_hit = ui.interact(go_rect, id.with("build road"), if ready { Sense::click() } else { Sense::hover() });
    let cancel_hit = ui.interact(cancel_rect, id.with("drop road"), Sense::click());
    let accent = theme::accent();
    let fill = match (ready, go_hit.hovered()) {
        (false, _) => accent.gamma_multiply(0.4),
        (true, true) => theme::mix(accent, Color32::WHITE, 0.15),
        (true, false) => accent,
    };
    painter.rect_filled(go_rect, theme::TILE_RADIUS, fill);
    painter.galley(pos2(go_rect.min.x + 12.0, mid - go.size().y / 2.0), go, Color32::WHITE);
    painter.galley(pos2(go_rect.max.x - 12.0 - enter.size().x, mid - enter.size().y / 2.0), enter, Color32::from_white_alpha(170));
    if cancel_hit.hovered() {
        painter.rect_filled(cancel_rect, theme::TILE_RADIUS, Color32::from_white_alpha(16));
    }
    painter.galley(pos2(cancel_rect.min.x + 10.0, mid - cancel.size().y / 2.0), cancel, Color32::from_gray(210));
    painter.galley(pos2(cancel_rect.max.x - 10.0 - esc.size().x, mid - esc.size().y / 2.0 + 0.5), esc, Color32::from_gray(130));
    if (ready && go_hit.hovered()) || cancel_hit.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if go_hit.clicked() {
        Some(true)
    } else if cancel_hit.clicked() {
        Some(false)
    } else {
        None
    }
}

/// The Road tool's part of Tool Settings.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use crate::panels::{hint, section};
    use crate::widgets::{Field, checkbox, combo, row, well};
    let pal = Palette::of(ui);
    let r = &mut tools.road;
    well(ui, |ui| {
        row(ui, tr!("Width"), |ui| {
            ui.add(Field::new(&mut r.width, 1.0..=64.0).suffix(tr!(" blocks")).max_decimals(0).width((ui.available_width() * 0.55).min(120.0)));
        });
        row(ui, tr!("Banks"), |ui| {
            ui.add(Field::new(&mut r.banks, 0.0..=64.0).suffix(tr!(" blocks")).max_decimals(0).width((ui.available_width() * 0.55).min(120.0)));
        });
        row(ui, tr!("Steepest slope"), |ui| {
            ui.add(Field::new(&mut r.slope, 1.0..=100.0).suffix("%").max_decimals(0).width((ui.available_width() * 0.55).min(120.0)));
        });
        row(ui, tr!("Surface"), |ui| {
            let shown = match &r.surface {
                RoadSurface::Keep => tr!("As it is").to_owned(),
                RoadSurface::Noise => tr!("Paint tool's noise").to_owned(),
                RoadSurface::Terrain(name) => name.clone(),
            };
            combo("road surface").selected_text(shown).width((ui.available_width() * 0.55).min(150.0)).truncate().show_ui(ui, |ui| {
                let mut pick = |ui: &mut Ui, s: RoadSurface, label: &str| {
                    if ui.selectable_label(r.surface == s, label).clicked() {
                        r.surface = s;
                    }
                };
                for name in SURFACES {
                    pick(ui, RoadSurface::Terrain(name.into()), name);
                }
                pick(ui, RoadSurface::Noise, tr!("Paint tool's noise"));
                pick(ui, RoadSurface::Keep, tr!("As it is"));
                ui.separator();
                for t in &doc.world.registry.terrains {
                    if !SURFACES.contains(&t.name.as_str()) {
                        pick(ui, RoadSurface::Terrain(t.name.clone()), &t.name);
                    }
                }
            });
        });
    });
    ui.add_space(6.0);
    checkbox(ui, &mut r.bridges, tr!("Bridges over water"));
    if r.bridges {
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, tr!("Deck"), |ui| {
                combo("bridge deck").selected_text(r.deck.clone()).width((ui.available_width() * 0.55).min(150.0)).truncate().show_ui(ui, |ui| {
                    for d in DECKS {
                        if ui.selectable_label(r.deck == d, d).clicked() {
                            r.deck = d.into();
                        }
                    }
                });
            });
        });
        hint(ui, tr!("A deck a block over the water, on a level of its own (Bridges); the water stays under it."));
    } else {
        hint(ui, tr!("Over water, the road is built up a block over it."));
    }
    if tools.filter.is_active() {
        hint(ui, tr!("The brush filters choose where its surface is painted."));
    }
    if doc.selection().is_some() {
        hint(ui, tr!("It changes only what is selected."));
    }

    section(ui, Icon::Signpost, tr!("Road"));
    if let Some(draft) = &mut tools.road_draft {
        draft.plan(doc, &tools.road);
    }
    match &tools.road_draft {
        None => hint(ui, tr!("Click on the map where the road starts, then where it goes.")),
        Some(draft) => {
            ui.label(egui::RichText::new(draft.summary()).strong().color(pal.text));
            hint(ui, tr!("Drag a point to move it · drag the road to add one · right-click or Delete: take one off."));
            let ready = draft.points.len() >= 2;
            ui.horizontal(|ui| {
                if ui.add_enabled(ready, egui::Button::new(theme::with_icon(Icon::Signpost, tr!("Build road")))).on_hover_text(tr!("Enter")).clicked() {
                    crate::road_draft::build(doc, tools);
                }
                if ui.button(tr!("Cancel")).on_hover_text(tr!("Esc")).clicked() {
                    tools.road_draft = None;
                }
            });
        }
    }
    if let Some(note) = &tools.road_note {
        hint(ui, note);
    }
}
