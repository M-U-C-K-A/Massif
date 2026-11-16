//! The Gradient brush's settings, its library and its editor: terrains in
//! bands along a bar, the stops between them dragged as in a picture
//! editor's gradient, a hard cut, noise or blobs between each two.

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::panels::{hint, section};
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::ToolState;
use egui::{Align, Color32, CursorIcon, FontId, Rect, RichText, Sense, Stroke, StrokeKind, TextureId, Ui, pos2, vec2};
use massif_core::gradient::{self, Gradient, GradientSampler, Layout, MAX_BANDS, MAX_WIDTH, Transition, TransitionKind};
use massif_core::pack_file::{self, Content, Pack};
use massif_core::registry::{Registry, TerrainId};
use serde::{Deserialize, Serialize};

/// The settings of the Gradient brush.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradientSettings {
    /// The gradient painted.
    pub gradient: Gradient,
    pub layout: Layout,
    /// Gradients saved, to paint with again in any map.
    pub saved: Vec<Gradient>,
    /// The editor, while it is open.
    #[serde(skip)]
    pub editor: Option<Editor>,
}

impl Default for GradientSettings {
    fn default() -> Self {
        Self {
            gradient: Gradient::default(),
            layout: Layout::Across,
            saved: Vec::new(),
            editor: None,
        }
    }
}

impl GradientSettings {
    /// Keep the gradient painted in the map, over one of the same name.
    pub fn keep_in(&self, doc: &mut Document) {
        let kept = &mut doc.world.meta.gradients;
        match kept.iter_mut().find(|g| g.name == self.gradient.name) {
            Some(g) if *g == self.gradient => {}
            Some(g) => *g = self.gradient.clone(),
            None => kept.push(self.gradient.clone()),
        }
    }

    /// Save `g` in the library (over one of its name) and paint with it.
    pub fn save(&mut self, g: Gradient) {
        match self.saved.iter_mut().find(|s| s.name == g.name) {
            Some(s) => *s = g.clone(),
            None => self.saved.push(g.clone()),
        }
        self.gradient = g;
    }

    /// Open the editor on a copy of the gradient painted (or a new one).
    pub fn edit(&mut self, new: bool) {
        let mut draft = self.gradient.clone();
        if new {
            draft.name = tr!("New gradient").to_owned();
        }
        self.editor = Some(Editor::new(draft));
    }
}

/// The editor of a gradient: a draft, saved or dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    pub draft: Gradient,
    /// The segment chosen along the bar: band `s / 2` when even, the
    /// transition after it when odd.
    pub selected: usize,
    /// The stop being dragged: the segments either side of it, their
    /// widths when the drag began, where it began.
    drag: Option<(usize, usize, f32, f32)>,
    /// What went wrong importing or exporting.
    pub error: Option<String>,
}

impl Editor {
    pub fn new(draft: Gradient) -> Self {
        Self { draft, selected: 0, drag: None, error: None }
    }
}

/// The width of segment `s` of `g`.
fn width(g: &Gradient, s: usize) -> f32 {
    if s.is_multiple_of(2) { g.bands[s / 2].width } else { g.transitions[s / 2].width }
}

fn set_width(g: &mut Gradient, s: usize, w: f32) {
    let w = w.clamp(if s.is_multiple_of(2) { 1.0 } else { 0.0 }, MAX_WIDTH);
    if s.is_multiple_of(2) {
        g.bands[s / 2].width = w;
    } else {
        g.transitions[s / 2].width = w;
    }
}

/// Whether segment `s` is a hard cut, which spans no blocks.
fn is_cut(g: &Gradient, s: usize) -> bool {
    s % 2 == 1 && g.transitions[s / 2].kind == TransitionKind::Hard
}

/// The colour of a terrain seen from above.
fn colour(reg: &Registry, t: TerrainId) -> Color32 {
    crate::panels::terrain_colors(reg, t.0).0
}

/// The texture of a terrain's top, when the game's textures are read.
fn texture(ui: &Ui, reg: &Registry, t: TerrainId, blocks: &mut BlockLibrary) -> Option<TextureId> {
    let cat = blocks.catalogue.clone()?;
    let block = massif_assets::terrain_block(reg, &cat, t)?;
    blocks.top(ui.ctx(), block)
}

/// Ground seen from above, a block a cell: column `i` of `cells` across is
/// `p(i)` blocks along the gradient, row `j` is `j` blocks along the edge.
/// Textured when cells are large enough to show them.
#[allow(clippy::too_many_arguments)]
fn paint_ground(ui: &Ui, rect: Rect, reg: &Registry, sampler: &GradientSampler, blocks: &mut BlockLibrary, cell: f32, p: impl Fn(f32) -> f32, z0: i32) {
    let painter = ui.painter().with_clip_rect(rect);
    let (cols, rows) = ((rect.width() / cell).ceil() as i32, (rect.height() / cell).ceil() as i32);
    let textured = cell >= 6.0;
    let mut cache: Vec<(TerrainId, Color32, Option<TextureId>)> = Vec::new();
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    for j in 0..rows {
        for i in 0..cols {
            let t = sampler.terrain(p(i as f32 + 0.5), i, z0 + j);
            let k = match cache.iter().position(|c| c.0 == t) {
                Some(k) => k,
                None => {
                    let tex = if textured { texture(ui, reg, t, blocks) } else { None };
                    cache.push((t, colour(reg, t), tex));
                    cache.len() - 1
                }
            };
            let r = Rect::from_min_size(pos2(rect.min.x + i as f32 * cell, rect.min.y + j as f32 * cell), vec2(cell, cell));
            match cache[k].2 {
                Some(tex) => {
                    painter.image(tex, r, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(r, 0.0, cache[k].1);
                }
            }
        }
    }
}

/// A small strip of a gradient's colours, for a list.
fn strip(ui: &mut Ui, g: &Gradient, reg: &Registry, size: egui::Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if let Some(sampler) = g.sampler(reg, 0) {
        let total = g.total().max(1.0);
        let painter = ui.painter().with_clip_rect(rect);
        let cell = (rect.width() / total).max(1.0);
        let n = (rect.width() / cell).ceil() as i32;
        for j in 0..(rect.height() / cell).ceil() as i32 {
            for i in 0..n {
                let p = (i as f32 + 0.5) * cell / rect.width() * total;
                let t = sampler.terrain(p, i, j);
                painter.rect_filled(Rect::from_min_size(rect.min + vec2(i as f32 * cell, j as f32 * cell), vec2(cell + 0.5, cell + 0.5)), 0.0, colour(reg, t));
            }
        }
    }
    let pal = Palette::of(ui);
    ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    response
}

/// The gradients to choose from: those of the map, those saved, those
/// built in; each name once, the first found.
fn library(tools: &ToolState, doc: &Document) -> Vec<(Gradient, Origin)> {
    let mut out: Vec<(Gradient, Origin)> = Vec::new();
    let built_in = gradient::built_in();
    let all = doc.world.meta.gradients.iter().map(|g| (g, Origin::Map)).chain(tools.gradient.saved.iter().map(|g| (g, Origin::Saved))).chain(built_in.iter().map(|g| (g, Origin::BuiltIn)));
    for (g, origin) in all {
        if !out.iter().any(|(h, _)| h.name == g.name) {
            out.push((g.clone(), origin));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Map,
    Saved,
    BuiltIn,
}

/// The Gradient brush's part of the Tool panel.
pub fn tool_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    let pal = Palette::of(ui);
    let layouts = [Layout::Across, Layout::Radial];
    let on = layouts.map(|l| tools.gradient.layout == l);
    if let Some(k) = crate::widgets::button_group(ui, "gradient layout", &[tr!("Across an edge"), tr!("From the centre")], &on) {
        tools.gradient.layout = layouts[k];
    }
    ui.add_space(2.0);
    hint(
        ui,
        match tools.gradient.layout {
            Layout::Across => tr!("Paint over the edge between two terrains, or in a selection: from its edge in."),
            Layout::Radial => tr!("A patch: the start of the gradient at the brush's centre, its end at the rim."),
        },
    );
    ui.add_space(4.0);
    let g = tools.gradient.gradient.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(&g.name).strong());
        ui.label(RichText::new(tr!("{n} blocks", n = format!("{:.0}", g.total()))).size(11.5).color(pal.weak));
    });
    strip(ui, &g, &doc.world.registry, vec2(ui.available_width().min(260.0), 18.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(theme::with_icon(Icon::Pencil, tr!("Edit…"))).clicked() {
            tools.gradient.edit(false);
        }
        if ui.button(theme::with_icon(Icon::Plus, tr!("New…"))).clicked() {
            tools.gradient.edit(true);
        }
    });
    if g.sampler(&doc.world.registry, 0).is_none() {
        hint(ui, tr!("None of its terrains is in this map."));
    }

    section(ui, Icon::Blend, tr!("Gradients"));
    let mut remove = None;
    for (item, origin) in library(tools, doc) {
        let chosen = item == tools.gradient.gradient;
        ui.horizontal(|ui| {
            let r = strip(ui, &item, &doc.world.registry, vec2(64.0, 14.0));
            let label = ui.add(egui::Label::new(RichText::new(&item.name).color(if chosen { pal.text } else { pal.weak }).strong()).truncate().sense(Sense::click()));
            if r.clicked() || label.clicked() {
                tools.gradient.gradient = item.clone();
            }
            if chosen {
                ui.label(RichText::new(theme::glyph(Icon::Check).to_string()).color(pal.accent));
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let what = match origin {
                    Origin::Map => tr!("In this map"),
                    Origin::Saved => tr!("Saved"),
                    Origin::BuiltIn => tr!("Built in"),
                };
                if origin == Origin::Saved && ui.small_button(theme::glyph(Icon::Trash).to_string()).on_hover_text(tr!("Remove it from the saved gradients")).clicked() {
                    remove = Some(item.name.clone());
                }
                ui.label(RichText::new(what).size(11.0).color(pal.weak));
            });
        });
    }
    if let Some(name) = remove {
        tools.gradient.saved.retain(|g| g.name != name);
    }
}

/// The editor, while it is open: a dialog over the window.
pub fn dialog(ctx: &egui::Context, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, author: &str) {
    let Some(mut editor) = tools.gradient.editor.take() else { return };
    let mut close = false;
    let mut out = Outcome::default();
    let layout = tools.gradient.layout;
    let presets = library(tools, doc);
    let response = crate::app::modal(ctx, "gradient editor").show(ctx, |ui| editor_ui(ui, &mut editor, doc, blocks, layout, &presets, &mut out));
    if response.should_close() || out.cancel {
        close = true;
    }
    // What needs the map itself, once the dialog is drawn.
    if let Some((k, pick)) = out.picked
        && k < editor.draft.bands.len()
    {
        editor.draft.bands[k].terrain = crate::panels::picked_terrain(pick, doc);
    }
    if out.import {
        import(&mut editor, doc);
    }
    if out.export {
        export(&mut editor, &doc.world.registry, author);
    }
    if let Some(g) = out.save {
        tools.gradient.save(g);
        tools.gradient.keep_in(doc);
        doc.mark_changed();
        if tools.tool != crate::tools::Tool::Gradient {
            tools.select(crate::tools::Tool::Gradient);
        }
        close = true;
    }
    if !close {
        tools.gradient.editor = Some(editor);
    }
}

/// What the editor was asked, to be done once it is drawn.
#[derive(Default)]
struct Outcome {
    /// A terrain or a block picked for a band.
    picked: Option<(usize, crate::panels::Pick)>,
    import: bool,
    export: bool,
    save: Option<Gradient>,
    cancel: bool,
}

/// Height of the bar of bands and transitions.
const BAR_H: f32 = 44.0;
/// Height of the row of stops and midpoints under it.
const STOPS_H: f32 = 18.0;
/// Height of the row of the edge's marker over it.
const EDGE_H: f32 = 14.0;
/// The width a hard cut takes on the bar, which spans no blocks.
const CUT_W: f32 = 12.0;

/// The bar's layout: where each segment starts and ends, in points from
/// its left, and how many points a block takes.
struct Bar {
    spans: Vec<(f32, f32)>,
    scale: f32,
}

impl Bar {
    fn of(g: &Gradient, width: f32) -> Self {
        let n = g.bands.len() * 2 - 1;
        let cuts = (0..n).filter(|&s| is_cut(g, s)).count() as f32;
        let scale = ((width - cuts * CUT_W) / g.total().max(1.0)).max(0.5);
        let mut spans = Vec::with_capacity(n);
        let mut x = 0.0;
        for s in 0..n {
            let w = if is_cut(g, s) { CUT_W } else { self::width(g, s) * scale };
            spans.push((x, x + w));
            x += w;
        }
        Self { spans, scale }
    }

    /// Where position `p` (blocks from the start) falls on the bar.
    fn x(&self, g: &Gradient, p: f32) -> f32 {
        let mut at = 0.0;
        for (s, &(x0, x1)) in self.spans.iter().enumerate() {
            if is_cut(g, s) {
                if p <= at {
                    return (x0 + x1) / 2.0;
                }
                continue;
            }
            let w = width(g, s);
            if p < at + w {
                return x0 + (p - at).max(0.0) * self.scale;
            }
            at += w;
        }
        self.spans.last().map_or(0.0, |s| s.1)
    }

    /// The position (blocks from the start) at `x` points along the bar.
    fn p(&self, g: &Gradient, x: f32) -> f32 {
        let mut at = 0.0;
        for (s, &(x0, x1)) in self.spans.iter().enumerate() {
            if is_cut(g, s) {
                if x < x1 {
                    return at;
                }
                continue;
            }
            let w = width(g, s);
            if x < x1 {
                return at + ((x - x0) / self.scale).clamp(0.0, w);
            }
            at += w;
        }
        g.total()
    }

    /// Where segment `s` starts along the gradient, in blocks.
    fn start(g: &Gradient, s: usize) -> f32 {
        (0..s).filter(|&j| !is_cut(g, j)).map(|j| width(g, j)).sum()
    }
}

/// A small label in a pill, centred on `at`: a value read while dragging.
fn pill(ui: &Ui, at: egui::Pos2, text: String) {
    let pal = Palette::of(ui);
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(11.0), pal.text);
    let r = Rect::from_center_size(at, galley.size() + vec2(12.0, 4.0));
    ui.painter().rect(r, theme::PILL, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, pal.text);
}

fn blocks_text(v: f32) -> String {
    if v.fract().abs() < 0.05 { format!("{v:.0}") } else { format!("{v:.1}") }
}

#[allow(clippy::too_many_arguments)]
fn editor_ui(ui: &mut Ui, ed: &mut Editor, doc: &Document, blocks: &mut BlockLibrary, layout: Layout, presets: &[(Gradient, Origin)], out: &mut Outcome) {
    let pal = Palette::of(ui);
    ui.set_width(660.0);
    ed.draft.normalize();
    ed.selected = ed.selected.min(ed.draft.bands.len() * 2 - 2);

    // ---- title, presets and name ----------------------------------------
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(Icon::Blend).to_string()).size(17.0).color(pal.accent));
        ui.label(RichText::new(tr!("Gradient brush")).bold().size(15.0));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(tr!("{n} blocks in all", n = blocks_text(ed.draft.total()))).color(pal.weak));
        });
    });
    ui.add_space(8.0);
    // All of it on a tall screen; in a short window, what does not fit
    // scrolls between the title and the buttons.
    let room = (ui.ctx().content_rect().height() - 170.0).max(240.0);
    egui::ScrollArea::vertical().id_salt("gradient editor body").max_height(room).auto_shrink([false, true]).show(ui, |ui| editor_body(ui, ed, doc, blocks, layout, presets, out));

    // ---- the buttons ----------------------------------------------------
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        out.import = ui.button(theme::with_icon(Icon::Import, tr!("Import…"))).clicked();
        out.export = ui.button(theme::with_icon(Icon::Upload, tr!("Export…"))).clicked();
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let ready = !ed.draft.name.trim().is_empty();
            if ui.add_enabled(ready, theme::primary_button(ui, tr!("Save"))).clicked() {
                let mut g = ed.draft.clone();
                g.name = g.name.trim().to_owned();
                g.normalize();
                out.save = Some(g);
            }
            out.cancel = ui.button(tr!("Cancel")).clicked();
        });
    });
}

/// The editor between its title and its buttons: the presets, the name,
/// the bar with its stops, the chosen segment's settings and the previews.
#[allow(clippy::too_many_arguments)]
fn editor_body(ui: &mut Ui, ed: &mut Editor, doc: &Document, blocks: &mut BlockLibrary, layout: Layout, presets: &[(Gradient, Origin)], out: &mut Outcome) {
    let pal = Palette::of(ui);
    let reg = &doc.world.registry;
    let seed = doc.world.meta.seed;
    ui.label(RichText::new(tr!("Presets")).size(11.5).color(pal.weak));
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (g, origin) in presets {
            let r = strip(ui, g, reg, vec2(76.0, 22.0));
            if *g == ed.draft || g.name == ed.draft.name {
                ui.painter().rect_stroke(r.rect.expand(2.0), 4.0, Stroke::new(1.5, pal.accent), StrokeKind::Outside);
            }
            if r.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let what = match origin {
                Origin::Map => tr!("In this map"),
                Origin::Saved => tr!("Saved"),
                Origin::BuiltIn => tr!("Built in"),
            };
            if r.on_hover_text(format!("{} · {what}", g.name)).clicked() {
                ed.draft = g.clone();
                ed.selected = 0;
            }
        }
    });
    ui.add_space(8.0);
    crate::widgets::well(ui, |ui| {
        crate::widgets::row(ui, tr!("Name"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut ed.draft.name).hint_text(tr!("The gradient's name")).desired_width(f32::INFINITY).char_limit(pack_file::MAX_NAME));
        });
    });
    ui.add_space(10.0);

    let Some(sampler) = ed.draft.sampler(reg, seed) else {
        ui.label(RichText::new(tr!("None of its terrains is in this map.")).color(pal.danger));
        return;
    };
    let g = ed.draft.clone();
    let full = ui.available_width();
    let bar_of = Bar::of(&g, full);
    let spans = &bar_of.spans;
    let scale = bar_of.scale;

    // ---- the edge, over the bar -------------------------------------------
    let (edge_row, _) = ui.allocate_exact_size(vec2(full, EDGE_H), Sense::hover());
    let left = edge_row.min.x;
    let edge_x = left + bar_of.x(&g, g.edge);
    let edge_r = ui.interact(Rect::from_center_size(pos2(edge_x, edge_row.center().y), vec2(14.0, EDGE_H)), egui::Id::unique("gradient edge marker"), Sense::drag());
    if edge_r.dragged()
        && let Some(at) = edge_r.interact_pointer_pos()
    {
        ed.draft.edge = (bar_of.p(&g, at.x - left) * 2.0).round() / 2.0;
    }
    let edge_hot = edge_r.hovered() || edge_r.dragged();
    if edge_hot {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    let edge_x = left + bar_of.x(&g, ed.draft.edge);
    let tip = edge_row.max.y - 1.0;
    ui.painter().add(egui::Shape::convex_polygon(vec![pos2(edge_x, tip), pos2(edge_x + 6.0, tip - 8.0), pos2(edge_x - 6.0, tip - 8.0)], pal.accent, Stroke::NONE));
    let edge_r = edge_r.on_hover_text(tr!("The edge: where the boundary between the two terrains falls, painted across it"));

    // ---- the bar --------------------------------------------------------
    let (bar, _) = ui.allocate_exact_size(vec2(full, BAR_H), Sense::hover());
    // The ground of each segment, drawn as the brush lays it: blocks of
    // `scale` points.
    let cell = scale.max(2.0);
    for (s, &(x0, x1)) in spans.iter().enumerate() {
        let rect = Rect::from_min_max(pos2(bar.min.x + x0, bar.min.y), pos2(bar.min.x + x1, bar.max.y));
        if is_cut(&g, s) {
            // A cut: half of each side.
            let (a, b) = (sampler.terrains()[s / 2], sampler.terrains()[s / 2 + 1]);
            ui.painter().rect_filled(Rect::from_min_max(rect.min, pos2(rect.center().x, rect.max.y)), 0.0, colour(reg, a));
            ui.painter().rect_filled(Rect::from_min_max(pos2(rect.center().x, rect.min.y), rect.max), 0.0, colour(reg, b));
        } else {
            let start = Bar::start(&g, s);
            paint_ground(ui, rect, reg, &sampler, blocks, cell, |i| start + i * cell / scale, 0);
        }
        let r = ui.interact(rect, egui::Id::unique(("gradient segment", s)), Sense::click());
        if r.clicked() {
            ed.selected = s;
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            ui.painter().rect_filled(rect, 0.0, Color32::from_white_alpha(18));
        }
        // What the transition is, in its middle.
        if s % 2 == 1 {
            let icon = match g.transitions[s / 2].kind {
                TransitionKind::Hard => Icon::Minus,
                TransitionKind::Noise => Icon::Waves,
                TransitionKind::Blobs => Icon::Circle,
            };
            let c = rect.center();
            ui.painter().circle_filled(c, 9.0, pal.panel.gamma_multiply(0.92));
            ui.painter().text(c, egui::Align2::CENTER_CENTER, theme::glyph(icon), FontId::proportional(11.0), pal.text);
        }
    }
    ui.painter().vline(edge_x, bar.y_range(), Stroke::new(if edge_hot { 2.0 } else { 1.0 }, pal.accent.gamma_multiply(if edge_hot { 1.0 } else { 0.7 })));
    ui.painter().rect_stroke(bar, theme::TILE_RADIUS / 2, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    {
        let (x0, x1) = spans[ed.selected];
        let r = Rect::from_min_max(pos2(bar.min.x + x0, bar.min.y), pos2(bar.min.x + x1, bar.max.y)).shrink(1.0);
        ui.painter().rect_stroke(r, 2.0, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
    }

    // ---- the stops and the midpoints, under it ----------------------------
    let (stops, _) = ui.allocate_exact_size(vec2(full, STOPS_H), Sense::hover());
    // A click between the stops adds one there: the band under it splits.
    let row = ui.interact(stops, egui::Id::unique("gradient stops row"), Sense::click());
    if row.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Copy);
    }
    if row.clicked()
        && let Some(at) = row.interact_pointer_pos()
    {
        let x = at.x - stops.min.x;
        if let Some(s) = spans.iter().position(|&(x0, x1)| x >= x0 && x < x1).filter(|s| s.is_multiple_of(2))
            && ed.draft.split_at(s / 2, (x - spans[s].0) / scale)
        {
            ed.selected = s + 2;
        }
    }
    let row = row.on_hover_text(tr!("Click: a new stop here, the band split in two"));
    let _ = row;
    let mut reading = None;
    // Between each two segments that span blocks: a hard cut is one stop
    // between the bands either side of it.
    let n = spans.len();
    let wide: Vec<usize> = (0..n).filter(|&s| !is_cut(&g, s)).collect();
    for pair in wide.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let x = stops.min.x + spans[a].1 + if b > a + 1 { CUT_W / 2.0 } else { 0.0 };
        let handle = Rect::from_center_size(pos2(x, stops.center().y), vec2(12.0, STOPS_H));
        let r = ui.interact(handle, egui::Id::unique(("gradient stop", a, b)), Sense::drag());
        if r.drag_started() {
            ed.drag = Some((a, b, width(&g, a), width(&g, b)));
        }
        if r.dragged()
            && let Some((da, db, wa, wb)) = ed.drag
            && (da, db) == (a, b)
            && let Some(origin) = r.interact_pointer_pos().zip(ui.input(|i| i.pointer.press_origin()))
        {
            let moved = ((origin.0.x - origin.1.x) / scale).round();
            let moved = moved.clamp(-(wa - if a.is_multiple_of(2) { 1.0 } else { 0.0 }), wb - if b.is_multiple_of(2) { 1.0 } else { 0.0 });
            set_width(&mut ed.draft, a, wa + moved);
            set_width(&mut ed.draft, b, wb - moved);
        }
        if r.drag_stopped() {
            ed.drag = None;
        }
        let dragging = ed.drag.is_some_and(|d| (d.0, d.1) == (a, b));
        let hot = r.hovered() || dragging;
        if hot {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if dragging {
            reading = Some((x, blocks_text(Bar::start(&ed.draft, b))));
        }
        let top = stops.min.y + 2.0;
        let fill = if hot { pal.accent } else { pal.weak };
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(x, top), pos2(x + 5.5, top + 6.0), pos2(x + 5.5, stops.max.y - 2.0), pos2(x - 5.5, stops.max.y - 2.0), pos2(x - 5.5, top + 6.0)], fill, Stroke::NONE));
    }
    // A transition's midpoint, a diamond between its stops: where the two
    // terrains are half and half.
    for (s, &(x0, x1)) in spans.iter().enumerate() {
        if s.is_multiple_of(2) || is_cut(&g, s) || x1 - x0 < 20.0 {
            continue;
        }
        let k = s / 2;
        let m = g.transitions[k].midpoint;
        let x = stops.min.x + x0 + m * (x1 - x0);
        let r = ui.interact(Rect::from_center_size(pos2(x, stops.center().y), vec2(10.0, STOPS_H)), egui::Id::unique(("gradient midpoint", k)), Sense::drag());
        if r.dragged()
            && let Some(at) = r.interact_pointer_pos()
        {
            let m = ((at.x - stops.min.x - x0) / (x1 - x0)).clamp(gradient::MIDPOINT_RANGE.0, gradient::MIDPOINT_RANGE.1);
            ed.draft.transitions[k].midpoint = (m * 100.0).round() / 100.0;
            ed.selected = s;
        }
        let hot = r.hovered() || r.dragged();
        if hot {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if r.dragged() {
            reading = Some((x, format!("{:.0}%", ed.draft.transitions[k].midpoint * 100.0)));
        }
        let c = pos2(x, stops.center().y + 1.0);
        let d = 4.5;
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(c.x, c.y - d), pos2(c.x + d, c.y), pos2(c.x, c.y + d), pos2(c.x - d, c.y)], if hot { pal.accent } else { pal.panel }, Stroke::new(1.2, if hot { pal.accent } else { pal.weak })));
        r.on_hover_text(tr!("Midpoint: where the two terrains are half and half. Drag it towards one to let the other go further into it"));
    }
    // Each segment's width under it, or the value being dragged.
    let (labels, _) = ui.allocate_exact_size(vec2(full, 16.0), Sense::hover());
    if edge_r.dragged() {
        reading = Some((edge_x - stops.min.x + stops.min.x, blocks_text(ed.draft.edge)));
    }
    match reading {
        Some((x, text)) => pill(ui, pos2(x.clamp(labels.min.x + 20.0, labels.max.x - 20.0), labels.center().y), text),
        None => {
            for (s, &(x0, x1)) in spans.iter().enumerate() {
                if is_cut(&g, s) || x1 - x0 < 14.0 {
                    continue;
                }
                let ink = if s == ed.selected { pal.text } else { pal.weak };
                ui.painter().text(pos2(bar.min.x + (x0 + x1) / 2.0, labels.center().y), egui::Align2::CENTER_CENTER, blocks_text(width(&g, s)), FontId::proportional(11.0), ink);
            }
        }
    }
    ui.add_space(8.0);

    // ---- the selected segment, and the previews ------------------------
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(full - 176.0);
            segment_ui(ui, ed, reg, blocks, out);
            ui.add_space(8.0);
            crate::widgets::well(ui, |ui| {
                let total = ed.draft.total();
                crate::widgets::row(ui, tr!("Edge at"), |ui| {
                    ui.add(crate::widgets::Field::new(&mut ed.draft.edge, 0.0..=total).suffix(tr!(" blocks")).max_decimals(1));
                });
                crate::widgets::row(ui, tr!("Wander"), |ui| {
                    ui.add(percent(&mut ed.draft.wander, 0.0..=gradient::MAX_WANDER)).on_hover_text(tr!("How much the bands widen and narrow along the edge: a rim wide here and thin there"));
                });
                crate::widgets::row(ui, tr!("Seed"), |ui| {
                    if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text(tr!("Another layout of the same gradient")).clicked() {
                        ed.draft.seed = ed.draft.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453) % 100_000;
                    }
                    crate::widgets::drag(ui, egui::DragValue::new(&mut ed.draft.seed).range(0..=99_999));
                });
            });
            hint(ui, tr!("Across an edge, the bands before this point lie on the side of the first terrains."));
        });
        ui.vertical(|ui| {
            ui.label(RichText::new(tr!("On a patch")).size(11.5).color(pal.weak));
            sample(ui, &ed.draft, reg, seed, layout, 160.0, blocks);
        });
    });
    ui.add_space(8.0);
    ui.label(RichText::new(tr!("Along an edge: the first terrains above, the last below")).size(11.5).color(pal.weak));
    along(ui, &ed.draft, reg, seed, full, blocks);
    if let Some(e) = &ed.error {
        ui.add(egui::Label::new(RichText::new(e).color(pal.danger)).wrap());
    }
}

/// A share typed or dragged as a percentage.
fn percent(v: &mut f32, range: std::ops::RangeInclusive<f32>) -> crate::widgets::Field<'_> {
    crate::widgets::Field::new(v, range).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
}

/// The settings of the selected band or transition.
fn segment_ui(ui: &mut Ui, ed: &mut Editor, reg: &Registry, blocks: &mut BlockLibrary, out: &mut Outcome) {
    let s = ed.selected;
    let g = &mut ed.draft;
    if s.is_multiple_of(2) {
        let k = s / 2;
        ui.label(RichText::new(tr!("Band {k} of {n}", k = k + 1, n = g.bands.len())).medium());
        ui.add_space(2.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Terrain"), |ui| {
                let current = g.bands[k].terrain.clone();
                let id = reg.terrain_by_name(&current);
                ui.menu_button(format!("{current}  {}", theme::glyph(Icon::ChevronDown)), |ui| {
                    if let Some(pick) = crate::panels::ground_picker(ui, egui::Id::unique(("gradient band pick", k)), reg, blocks, Some(&current)) {
                        out.picked = Some((k, pick));
                    }
                })
                .response
                .on_hover_text(tr!("A terrain of the map, or any block of the game or of its mods"));
                if let Some(id) = id {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    match texture(ui, reg, id, blocks) {
                        Some(tex) => {
                            ui.painter().image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(r, 3.0, colour(reg, id));
                        }
                    }
                }
            });
            crate::widgets::row(ui, tr!("Width"), |ui| {
                ui.add(crate::widgets::Field::new(&mut g.bands[k].width, 1.0..=MAX_WIDTH).suffix(tr!(" blocks")).max_decimals(0).logarithmic(true));
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(g.bands.len() < MAX_BANDS && g.bands[k].width >= 2.0, egui::Button::new(theme::with_icon(Icon::Plus, tr!("Add a band")))).on_hover_text(tr!("Split this band in two, a transition between")).clicked() {
                g.split(k);
                ed.selected = s + 2;
            }
            if ui.add_enabled(g.bands.len() > 1, egui::Button::new(theme::with_icon(Icon::Trash, tr!("Remove")))).clicked() {
                g.remove(k);
                ed.selected = (k * 2).min(g.bands.len() * 2 - 2);
            }
        });
    } else {
        let k = s / 2;
        let names = (g.bands[k].terrain.clone(), g.bands[k + 1].terrain.clone());
        ui.label(RichText::new(tr!("From {a} to {b}", a = names.0, b = names.1)).medium());
        ui.add_space(2.0);
        let t = &mut g.transitions[k];
        let kinds = TransitionKind::ALL;
        let on = kinds.map(|kind| t.kind == kind);
        if let Some(j) = crate::widgets::button_group(ui, "transition kind", &[tr!("Hard cut"), tr!("Noise"), tr!("Blobs")], &on) {
            let was = t.kind;
            t.kind = kinds[j];
            if t.kind == TransitionKind::Hard {
                t.width = 0.0;
            } else if was == TransitionKind::Hard && t.width < 1.0 {
                t.width = Transition::default().width;
            }
        }
        ui.add_space(4.0);
        let hard = t.kind == TransitionKind::Hard;
        ui.add_enabled_ui(!hard, |ui| {
            crate::widgets::well(ui, |ui| {
                crate::widgets::row(ui, tr!("Width"), |ui| {
                    ui.add(crate::widgets::Field::new(&mut t.width, 0.0..=MAX_WIDTH).suffix(tr!(" blocks")).max_decimals(0).logarithmic(true));
                });
                let (amount, size) = match t.kind {
                    TransitionKind::Blobs => (tr!("Density"), tr!("Blob size")),
                    _ => (tr!("Intensity"), tr!("Scale")),
                };
                crate::widgets::row(ui, amount, |ui| {
                    ui.add(percent(&mut t.amount, 0.0..=1.0));
                });
                crate::widgets::row(ui, size, |ui| {
                    ui.add(crate::widgets::Field::new(&mut t.size, 1.0..=64.0).suffix(tr!(" blocks")).max_decimals(0).logarithmic(true));
                });
                crate::widgets::row(ui, tr!("Midpoint"), |ui| {
                    ui.add(percent(&mut t.midpoint, gradient::MIDPOINT_RANGE.0..=gradient::MIDPOINT_RANGE.1));
                });
            });
        });
        ui.add_space(2.0);
        hint(
            ui,
            match t.kind {
                TransitionKind::Hard => tr!("A clean line between the two."),
                TransitionKind::Noise => tr!("A ragged edge: each terrain reaches into the other."),
                TransitionKind::Blobs => tr!("Blobs of every size, denser and denser until they join."),
            },
        );
    }
}

/// A picture of ground, a pixel a block, `colour_at(x, z)` each. One
/// picture of each `kind` is kept, made again only when `key` (what it
/// shows) changes.
fn picture(ui: &Ui, kind: &'static str, key: impl std::hash::Hash, size: [usize; 2], colour_at: impl Fn(i32, i32) -> Color32) -> egui::TextureHandle {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    (key, size).hash(&mut hasher);
    let hash = hasher.finish();
    let id = egui::Id::unique(("gradient picture", kind));
    let kept: Option<(u64, egui::TextureHandle)> = ui.ctx().data(|d| d.get_temp(id));
    if let Some((h, tex)) = &kept
        && *h == hash
    {
        return tex.clone();
    }
    let pixels = (0..size[1] as i32).flat_map(|z| (0..size[0] as i32).map(move |x| (x, z))).map(|(x, z)| colour_at(x, z)).collect();
    let image = egui::ColorImage { size, pixels, source_size: vec2(size[0] as f32, size[1] as f32) };
    let tex = match kept {
        Some((_, mut tex)) => {
            tex.set(image, egui::TextureOptions::NEAREST);
            tex
        }
        None => ui.ctx().load_texture(kind, image, egui::TextureOptions::NEAREST),
    };
    ui.ctx().data_mut(|d| d.insert_temp(id, (hash, tex.clone())));
    tex
}

/// What a picture of `g` depends on.
fn key_of(g: &Gradient, reg: &Registry, seed: i64, blocks: &BlockLibrary) -> String {
    let colours: Vec<[u8; 4]> = g.bands.iter().filter_map(|b| reg.terrain_by_name(&b.terrain)).map(|t| colour(reg, t).to_array()).collect();
    format!("{}{colours:?}{seed}{}", serde_json::to_string(g).unwrap_or_default(), blocks.catalogue.is_some())
}

/// How the terrains a gradient paints look from above: their top textures
/// (tinted, their first frame), or their colours when the game's textures
/// are not read.
struct Looks(Vec<(TerrainId, usize, Vec<Color32>)>);

impl Looks {
    fn of(sampler: &GradientSampler, reg: &Registry, blocks: &BlockLibrary) -> Self {
        let mut out: Vec<(TerrainId, usize, Vec<Color32>)> = Vec::new();
        for &t in sampler.terrains() {
            if out.iter().any(|l| l.0 == t) {
                continue;
            }
            let texels = blocks.catalogue.as_deref().and_then(|cat| {
                let block = massif_assets::terrain_block(reg, cat, t)?;
                let tex = block.top.as_ref().or(block.side.as_ref()).and_then(|k| cat.texture(k))?.tinted(block.top_tint);
                let side = tex.width as usize;
                (side > 0 && tex.rgba.len() >= side * side * 4).then(|| (side, tex.rgba[..side * side * 4].as_chunks::<4>().0.iter().map(|p| Color32::from_rgb(p[0], p[1], p[2])).collect()))
            });
            let (side, texels) = texels.unwrap_or_else(|| (1, vec![colour(reg, t)]));
            out.push((t, side, texels));
        }
        Looks(out)
    }

    /// Terrain `t` at `(u, v)` of a block's top, both in `0..1`.
    fn at(&self, t: TerrainId, u: f32, v: f32) -> Color32 {
        let Some((_, side, texels)) = self.0.iter().find(|l| l.0 == t) else { return Color32::GRAY };
        let at = |f: f32| ((f * *side as f32) as usize).min(side - 1);
        texels[at(v) * side + at(u)]
    }
}

/// A picture of ground `cols × rows` blocks, `k` pixels a block: the
/// terrain of each block from `terrain_at(x, z)`, drawn with its texture.
fn ground_picture(ui: &Ui, kind: &'static str, key: &str, looks: &Looks, (cols, rows, k): (usize, usize, usize), terrain_at: impl Fn(i32, i32) -> TerrainId) -> egui::TextureHandle {
    let grid: Vec<TerrainId> = (0..rows as i32).flat_map(|z| (0..cols as i32).map(move |x| (x, z))).map(|(x, z)| terrain_at(x, z)).collect();
    picture(ui, kind, key, [cols * k, rows * k], |px, pz| {
        let (x, z) = (px as usize / k, pz as usize / k);
        let (u, v) = (((px as usize % k) as f32 + 0.5) / k as f32, ((pz as usize % k) as f32 + 0.5) / k as f32);
        looks.at(grid[z * cols + x], u, v)
    })
}

/// The gradient painted on a sample: a patch of its first terrains in its
/// last, as the brush lays it.
#[allow(clippy::too_many_arguments)]
fn sample(ui: &mut Ui, g: &Gradient, reg: &Registry, seed: i64, layout: Layout, side: f32, blocks: &BlockLibrary) {
    let pal = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let Some(sampler) = g.sampler(reg, seed) else { return };
    // 2 points a block: a patch some 80 blocks across.
    let cells = (side / 2.0) as usize;
    let c = cells as f32 / 2.0;
    let total = g.total().max(1.0);
    let looks = Looks::of(&sampler, reg, blocks);
    let key = format!("{}{}", key_of(g, reg, seed, blocks), layout == Layout::Radial);
    let tex = ground_picture(ui, "gradient sample", &key, &looks, (cells, cells, 2), |x, z| {
        let (dx, dz) = (x as f32 + 0.5 - c, z as f32 + 0.5 - c);
        let r = (dx * dx + dz * dz).sqrt();
        let p = match layout {
            Layout::Radial => r / (c * 0.95) * total * sampler.stretch(x, z),
            Layout::Across => {
                // A wavering patch's edge.
                let a = dz.atan2(dx);
                let edge = c * 0.5 * (1.0 + 0.16 * (3.0 * a).sin() + 0.08 * (5.0 * a + 1.0).cos());
                g.edge + (r - edge) * sampler.stretch(x, z)
            }
        };
        sampler.terrain(p, x, z)
    });
    ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, theme::TILE_RADIUS, Stroke::new(1.0, pal.border), StrokeKind::Inside);
}

/// The gradient along a long, gently winding edge: its first terrains
/// above, its last below, 4 points a block.
fn along(ui: &mut Ui, g: &Gradient, reg: &Registry, seed: i64, width: f32, blocks: &BlockLibrary) {
    let pal = Palette::of(ui);
    const MARGIN: f32 = 3.0;
    let total = g.total().max(1.0);
    let rows = (total + 2.0 * MARGIN).ceil() as usize;
    let k = (120 / rows).clamp(1, 4);
    let cols = width as usize / k;
    let (rect, _) = ui.allocate_exact_size(vec2((cols * k) as f32, (rows * k) as f32), Sense::hover());
    let Some(sampler) = g.sampler(reg, seed) else { return };
    let looks = Looks::of(&sampler, reg, blocks);
    let tex = ground_picture(ui, "gradient along an edge", &key_of(g, reg, seed, blocks), &looks, (cols, rows, k), |x, z| {
        let wave = 2.5 * (x as f32 / 23.0).sin() + 1.5 * (x as f32 / 9.0 + 1.0).sin();
        let d = z as f32 + 0.5 - MARGIN - g.edge - wave;
        sampler.terrain(g.edge + d * sampler.stretch(x, z), x, z)
    });
    ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, theme::TILE_RADIUS / 2, Stroke::new(1.0, pal.border), StrokeKind::Inside);
}

/// The help's picture of the brush: a built-in gradient along an edge, in
/// the terrains' colours.
pub fn help_picture(ui: &mut Ui) {
    static VANILLA: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    let reg = VANILLA.get_or_init(Registry::vanilla);
    let g = gradient::built_in().into_iter().find(|g| g.name == "Shore").unwrap_or_default();
    let width = ui.available_width().min(520.0);
    along(ui, &g, reg, 7, width, &BlockLibrary::default());
}

/// A pack of the draft alone, with the terrains it needs.
fn pack_of(g: &Gradient, reg: &Registry, author: &str) -> Pack {
    let mut content = Content::default();
    Pack::gather_gradients(&mut content, reg, std::slice::from_ref(g));
    let mut pack = Pack { name: g.name.clone(), author: author.to_owned(), description: String::new(), made_with: env!("CARGO_PKG_VERSION").to_owned(), preview: None, content };
    pack.preview = Some(pack.draw_preview());
    pack
}

fn export(ed: &mut Editor, reg: &Registry, author: &str) {
    let pack = pack_of(&ed.draft, reg, author);
    let Some(path) = rfd::FileDialog::new()
        .set_title(tr!("Export the gradient"))
        .add_filter(tr!("Massif pack"), &[pack_file::EXTENSION])
        .set_file_name(format!("{}.{}", pack_file::file_stem(&pack.name), pack_file::EXTENSION))
        .save_file()
    else {
        return;
    };
    let path = if crate::pack_share::is_pack_file(&path) { path } else { path.with_extension(pack_file::EXTENSION) };
    ed.error = massif_core::atomic::write(&path, &pack.to_bytes(), true).err().map(|e| massif_core::atomic::describe(&e, &path));
}

fn import(ed: &mut Editor, doc: &mut Document) {
    let Some(path) = rfd::FileDialog::new().set_title(tr!("Import a gradient")).add_filter(tr!("Massif pack"), &[pack_file::EXTENSION]).pick_file() else {
        return;
    };
    match crate::pack_share::read_file(&path) {
        Ok(pack) => ed.error = take_gradient(ed, &pack, doc),
        Err(e) => ed.error = Some(e),
    }
}

/// Put the pack's first gradient in the editor, its terrains added to the
/// map. Returns what went wrong.
pub fn take_gradient(ed: &mut Editor, pack: &Pack, doc: &mut Document) -> Option<String> {
    if pack.content.gradients.is_empty() {
        return Some(tr!("“{name}” holds no gradient.", name = pack.name));
    }
    ed.draft = pack.install_gradient(0, &mut doc.world.registry);
    ed.selected = 0;
    doc.mark_changed();
    None
}

/// What the renders of the brush share: the game's blocks (and the mod's,
/// with MASSIF_CONQUEST_JAR), a patch painted by hand, the gradient laid
/// over it, the ground seen from above with the game's textures.
#[cfg(test)]
pub(crate) mod shots {
    use super::*;

    /// The game's catalogue, and the game's with Conquest Reforged's blocks
    /// when MASSIF_CONQUEST_JAR names its jar. `None` without Minecraft.
    pub(crate) fn catalogues() -> Option<(massif_assets::Catalogue, Option<massif_assets::Catalogue>)> {
        let jar = massif_assets::find_jars().into_iter().next()?;
        let mods: Vec<std::path::PathBuf> = std::env::var_os("MASSIF_CONQUEST_JAR").map(std::path::PathBuf::from).into_iter().filter(|p| p.exists()).collect();
        let vanilla = massif_assets::load(&jar.path, &jar.version).ok()?;
        let modded = (!mods.is_empty()).then(|| massif_assets::load_with_mods(&jar.path, &jar.version, &mods).expect("the mod's blocks"));
        Some((vanilla, modded))
    }

    /// Gradients of Conquest Reforged's blocks, after the terraformers'
    /// examples and sketch: (file, gradient, ground around, patch).
    pub(crate) fn conquest() -> Vec<(&'static str, Gradient, &'static str, &'static str)> {
        let (hard, noise, blobs) = (Transition::hard(), Transition::noise(6.0, 0.8, 4.0), Transition::blobs(8.0, 0.7, 6.0));
        vec![
            ("conquest-hard-cut", Gradient::new("Hard cuts", &[("Gray Limestone", 4.0), ("Rocky Soil", 4.0), ("Clover Covered Grass", 4.0)], &[hard, hard]), "Clover Covered Grass", "Gray Limestone"),
            ("conquest-noise", Gradient::new("Noise", &[("Gray Limestone", 4.0), ("Rocky Soil", 4.0), ("Clover Covered Grass", 4.0)], &[noise, noise]), "Clover Covered Grass", "Gray Limestone"),
            ("conquest-blobs", Gradient::new("Blobs", &[("Gray Limestone", 4.0), ("Rocky Soil", 4.0), ("Clover Covered Grass", 4.0)], &[blobs, blobs]), "Clover Covered Grass", "Gray Limestone"),
            (
                "conquest-rock-gold-green-rim",
                Gradient::new(
                    "Limestone, a gold-green rim, meadow",
                    &[("Gray Limestone", 4.0), ("Mossy Limestone", 3.0), ("Mossy Lorien Forest Floor", 3.0), ("Clover Covered Grass", 4.0)],
                    &[Transition::noise(3.0, 0.7, 3.0), Transition::noise(3.0, 0.7, 3.0), Transition::blobs(5.0, 0.65, 4.0)],
                ),
                "Clover Covered Grass",
                "Gray Limestone",
            ),
            (
                "conquest-dark-pit-two-greys",
                Gradient::new(
                    "Dark pit, two greys, grass",
                    &[("Dark Gravel", 4.0), ("Gray Gravel", 3.0), ("Light Gravel", 3.0), ("Taiga Grass", 4.0)],
                    &[Transition::hard(), Transition::noise(3.0, 0.6, 3.0), Transition::blobs(6.0, 0.7, 4.0)],
                ),
                "Taiga Grass",
                "Dark Gravel",
            ),
            (
                "conquest-sketch-30-blocks",
                Gradient::new(
                    "Deciduous forest to grass",
                    &[("Light Green Sphagnum Moss Block", 4.0), ("Ochre Soil", 5.0), ("Rocky Soil", 6.0), ("Taiga Grass", 4.0)],
                    &[Transition::noise(6.0, 0.7, 5.0), Transition::blobs(5.0, 0.6, 5.0), Transition::hard()],
                ),
                "Taiga Grass",
                "Light Green Sphagnum Moss Block",
            ),
        ]
    }

    /// The ground of a world seen from above, `k` pixels a block, with the
    /// game's textures: blocks `x0..x0 + w`, `z0..z0 + h`.
    pub(crate) fn ground_png(world: &massif_core::World, cat: &massif_assets::Catalogue, (x0, z0, w, h): (i32, i32, i32, i32), k: u32) -> image::RgbImage {
        let reg = &world.registry;
        let mut texels: std::collections::HashMap<u16, (usize, Vec<[u8; 3]>)> = std::collections::HashMap::new();
        let mut img = image::RgbImage::new(w as u32 * k, h as u32 * k);
        let level = world.dims[0].ground();
        for z in 0..h {
            for x in 0..w {
                let (bx, bz) = (x0 + x, z0 + z);
                let Some(tile) = level.view(massif_core::TilePos::containing(bx, bz)) else { continue };
                let t = tile.terrain.get(massif_core::coords::local_index(bx, bz));
                let (side, tex) = texels.entry(t).or_insert_with(|| {
                    let id = TerrainId(t);
                    massif_assets::terrain_block(reg, cat, id)
                        .and_then(|b| Some((b.top.as_ref().or(b.side.as_ref()).and_then(|key| cat.texture(key))?.tinted(b.top_tint), b)))
                        .map(|(tex, _)| (tex.width as usize, tex.rgba.as_chunks::<4>().0.iter().take(tex.width as usize * tex.width as usize).map(|p| [p[0], p[1], p[2]]).collect()))
                        .unwrap_or_else(|| (1, vec![reg.terrain(id).color]))
                });
                for py in 0..k {
                    for px in 0..k {
                        let (u, v) = ((px as usize * *side) / k as usize, (py as usize * *side) / k as usize);
                        img.put_pixel(x as u32 * k + px, z as u32 * k + py, image::Rgb(tex[v * *side + u]));
                    }
                }
            }
        }
        img
    }

    /// Two pictures side by side, a gap between.
    pub(crate) fn side_by_side(a: &image::RgbImage, b: &image::RgbImage) -> image::RgbImage {
        let gap = 16;
        let mut out = image::RgbImage::from_pixel(a.width() + gap + b.width(), a.height().max(b.height()), image::Rgb([250, 250, 250]));
        image::imageops::replace(&mut out, a, 0, 0);
        image::imageops::replace(&mut out, b, (a.width() + gap) as i64, 0);
        out
    }

    /// A world of `around`, with an irregular patch of `patch` in its
    /// middle some 70 blocks across, as painted before the gradient.
    /// Terrain `name` of the map, made from the game's (or the mod's) block
    /// of that name when the map has none, as the band picker does.
    pub(crate) fn terrain(reg: &mut Registry, cat: &massif_assets::Catalogue, name: &str) -> TerrainId {
        match reg.terrain_by_name(name) {
            Some(t) => t,
            None => {
                let block = cat.blocks.iter().find(|b| b.name == name).unwrap_or_else(|| panic!("no block {name}"));
                massif_assets::block_terrain(reg, block)
            }
        }
    }

    /// A brush in the shape of a patch painted by hand: an outline that
    /// wanders, `radius` blocks out at most.
    pub(crate) fn patch_brush(radius: f32, seed: u64) -> massif_core::Brush {
        let n = 160;
        let noise = massif_core::noise::Noise::new(seed);
        let data = (0..n * n)
            .map(|i| {
                let (x, z) = ((i % n) as f64 / n as f64 * 2.0 - 1.0, (i / n) as f64 / n as f64 * 2.0 - 1.0);
                let r = (x * x * 1.0 + z * z * 1.6).sqrt();
                let wobble = noise.fbm(x * 2.2, z * 2.2, 3, 2.0, 0.5) * 0.35;
                if r + wobble < 0.78 { 255 } else { 0 }
            })
            .collect();
        massif_core::Brush { mask: Some(std::sync::Arc::new(massif_core::BrushMask::new("patch", n, n, data))), strength: 1.0, ..massif_core::Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, radius) }
    }

    pub(crate) fn patch_world(cat: &massif_assets::Catalogue, around: &str, patch: &str, g: &Gradient) -> massif_core::World {
        let mut world = massif_core::World::new(massif_core::WorldMeta { seed: 5, ..Default::default() });
        for name in g.terrains() {
            terrain(&mut world.registry, cat, name);
        }
        let (around, patch) = (terrain(&mut world.registry, cat, around), terrain(&mut world.registry, cat, patch));
        for z in -2..2 {
            for x in -2..2 {
                world.dims[0].ground_mut().insert(massif_core::TilePos::new(x, z), massif_core::Tile::ground(64.0, around.0, 63));
            }
        }
        let brush = patch_brush(48.0, 17);
        let mut history = massif_core::History::default();
        history.begin("patch", 0);
        let mut edit = history.edit(&mut world, 0);
        massif_core::ops::paint_terrain(&mut edit, &brush, 0.0, 0.0, patch, 1);
        history.commit();
        world
    }

    /// The gradient painted over the whole patch's edge with one click of a
    /// big brush.
    pub(crate) fn paint_over(world: &mut massif_core::World, g: &Gradient, at: (f32, f32), radius: f32) {
        let sampler = g.sampler(&world.registry, world.meta.seed).expect("its terrains");
        let brush = massif_core::Brush { strength: 1.0, ..massif_core::Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, radius) };
        let mut history = massif_core::History::default();
        history.begin("Gradient brush", 0);
        let mut edit = history.edit(world, 0);
        massif_core::ops::paint_gradient(&mut edit, &brush, at.0, at.1, 7, &sampler, Layout::Across, &mut massif_core::gradient::Sides::new());
        history.commit();
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Document {
        let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 2, tiles_z: 2, landform: massif_core::generate::Landform::Flat, ..Default::default() });
        Document::new(world, None, 1)
    }

    /// The tool's settings with the gradients to choose from, and the help's
    /// topic with its picture. Renders with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn the_settings_and_the_help_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
        let topic = crate::help::topics().iter().find(|t| t.title == "Painting").expect("the Painting topic");
        let start = topic.blocks.iter().position(|b| matches!(b, crate::help::Block::Heading(h) if h == "Gradient brush")).expect("its Gradient brush part");
        let end = topic.blocks[start + 1..].iter().position(|b| matches!(b, crate::help::Block::Heading(_))).map_or(topic.blocks.len(), |k| start + 1 + k);
        assert!(topic.blocks[start..end].iter().any(|b| matches!(b, crate::help::Block::Picture(p) if p == "gradient")));
        for (theme, suffix) in [(egui::Theme::Light, "light"), (egui::Theme::Dark, "dark")] {
            let mut state = (ToolState::default(), flat());
            state.0.select(crate::tools::Tool::Gradient);
            state.1.world.meta.gradients.push(Gradient::new("Dark pit", &[("Blackstone", 4.0), ("Andesite", 3.0), ("Stone", 3.0), ("Grass", 4.0)], &[Transition::hard(), Transition::noise(3.0, 0.6, 3.0), Transition::blobs(5.0, 0.7, 4.0)]));
            let mut h = egui_kittest::Harness::builder().with_size(vec2(320.0, 640.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui_state(
                move |ui, s: &mut (ToolState, Document)| {
                    if ui.ctx().cumulative_pass_nr() == 0 {
                        crate::theme::install(ui.ctx());
                    }
                    ui.ctx().set_theme(theme);
                    egui::Frame::new().fill(Palette::of(ui).panel).inner_margin(12).show(ui, |ui| {
                        ui.set_width(296.0);
                        tool_settings(ui, &mut s.0, &mut s.1);
                    });
                },
                state,
            );
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("gradient-settings-{suffix}.png"))).unwrap();
            let blocks: Vec<&crate::help::Block> = topic.blocks[start..end].iter().collect();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(560.0, 560.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui(move |ui| {
                if ui.ctx().cumulative_pass_nr() == 0 {
                    crate::theme::install(ui.ctx());
                }
                ui.ctx().set_theme(theme);
                egui::Frame::new().fill(Palette::of(ui).panel).inner_margin(16).show(ui, |ui| {
                    ui.set_width(528.0);
                    let owned: Vec<crate::help::Block> = blocks.iter().map(|b| (*b).clone()).collect();
                    crate::help::blocks_ui(ui, &owned);
                });
            });
            h.run_steps(3);
            h.render().unwrap().save(dir.join(format!("gradient-help-{suffix}.png"))).unwrap();
        }
    }

    /// Before and after: a patch's edge painted with a hard cut, a noise
    /// and blobs, the terraformers' two examples, and a gradient of
    /// Conquest Reforged's blocks (with MASSIF_CONQUEST_JAR, the mod's jar):
    /// the ground seen from above with the game's textures. Runs with
    /// MASSIF_SNAPSHOT_DIR when a Minecraft jar is installed.
    #[test]
    fn boundaries_painted_before_and_after() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let Some((vanilla, modded)) = shots::catalogues() else {
            eprintln!("no Minecraft installation: skipping");
            return;
        };
        let view = (-66, -45, 132, 90);
        let mut shots: Vec<(String, &str, &str, Gradient)> = Vec::new();
        for (name, tr) in [("hard", Transition::hard()), ("noise", Transition::noise(6.0, 0.8, 4.0)), ("blobs", Transition::blobs(8.0, 0.7, 6.0))] {
            let g = Gradient::new(name, &[("Stone", 4.0), ("Coarse Dirt", 4.0), ("Grass", 4.0)], &[tr, tr]);
            shots.push((format!("gradient-{name}"), "Grass", "Stone", g));
        }
        for (file, name, around, patch) in [("gradient-rock-to-grass", "Rock to grass", "Grass", "Stone"), ("gradient-pit-to-grass", "Pit to grass", "Grass", "Deepslate"), ("gradient-forest-to-grass", "Deciduous forest to grass", "Grass", "Podzol")] {
            let g = gradient::built_in().into_iter().find(|g| g.name == name).unwrap();
            shots.push((file.to_owned(), around, patch, g));
        }
        if modded.is_some() {
            for (file, g, around, patch) in shots::conquest() {
                shots.push((format!("gradient-{file}"), around, patch, g));
            }
        }
        for (file, around, patch, g) in shots {
            let cat = if file.starts_with("gradient-conquest") { modded.as_ref().unwrap() } else { &vanilla };
            let mut world = shots::patch_world(cat, around, patch, &g);
            let before = shots::ground_png(&world, cat, view, 8);
            shots::paint_over(&mut world, &g, (0.0, 0.0), 90.0);
            let after = shots::ground_png(&world, cat, view, 8);
            assert_ne!(before, after, "{file}: the gradient changed the ground");
            if file == "gradient-conquest-rock-gold-green-rim" {
                // The rim is the mod's blocks, as the export writes them.
                let reg = &world.registry;
                let level = world.dims[0].ground();
                let states: std::collections::HashSet<String> = (-60..60)
                    .flat_map(|z| (-60..60).map(move |x| (x, z)))
                    .filter_map(|(x, z)| {
                        let t = level.view(massif_core::TilePos::containing(x, z))?.terrain.get(massif_core::coords::local_index(x, z));
                        Some(reg.block(reg.terrain(TerrainId(t)).strata[0].blocks[0].0).state.clone())
                    })
                    .collect();
                for state in ["conquest:gray_limestone", "conquest:mossy_limestone", "conquest:mossy_lorien_forest_floor", "conquest:clover_covered_grass"] {
                    assert!(states.iter().any(|s| s.starts_with(state)), "{state} in {states:?}");
                }
            }
            shots::side_by_side(&before, &after).save(dir.join(format!("{file}.png"))).unwrap();
        }
    }

    #[test]
    fn a_stop_moves_blocks_from_one_segment_to_its_neighbour() {
        let mut g = Gradient::default();
        let total = g.total();
        set_width(&mut g, 0, 0.0);
        assert_eq!(g.bands[0].width, 1.0, "a band keeps a block");
        set_width(&mut g, 1, 9.0);
        assert_eq!(g.transitions[0].width, 9.0);
        assert!(g.total() != total);
    }

    #[test]
    fn the_map_keeps_the_gradients_it_is_painted_with() {
        let mut doc = flat();
        let mut s = GradientSettings::default();
        s.keep_in(&mut doc);
        s.keep_in(&mut doc);
        assert_eq!(doc.world.meta.gradients.len(), 1);
        s.gradient.bands[0].width = 9.0;
        s.keep_in(&mut doc);
        assert_eq!(doc.world.meta.gradients, [s.gradient.clone()]);
    }

    #[test]
    fn a_pack_of_a_gradient_takes_it_into_another_map() {
        let doc = flat();
        let g = Gradient::default();
        let pack = Pack::from_bytes(&pack_of(&g, &doc.world.registry, "Fornad").to_bytes()).unwrap();
        let mut other = flat();
        let mut ed = Editor::new(Gradient::new("x", &[("Sand", 3.0)], &[]));
        assert_eq!(take_gradient(&mut ed, &pack, &mut other), None);
        assert_eq!(ed.draft, g);
    }
}


