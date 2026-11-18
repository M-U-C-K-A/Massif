//! Working on one map together, in the Versions panel (see
//! `docs/versioning.md`): the panel's home while the map has no version,
//! the review of a contribution (its changed tiles over the 2D map, its
//! areas to accept or reject, the settings it changes), the resolver of an
//! area in conflict, and what the merge takes.
//!
//! A contribution comes from another person's copy of the map (a
//! `.massifpatch` file) or from another tool (a region exported as images,
//! edited in World Machine or Gaea and brought back): either way it is
//! classified against the map as it is (see [`massif_core::merge`]), and
//! nothing changes until the owner applies what they accepted, as one step
//! to undo and a new version.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{button_group, row, switch, well};
use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Color32, FontId, Galley, Painter, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::TilePos;
use massif_core::coords::TILE_SIZE;
use massif_core::merge::{Decision, Review, Status};
use massif_core::patch::{Kind, Patch};
use massif_core::registry::Registry;
use std::collections::HashSet;
use std::sync::Arc;

// ------------------------------------------------------------------ home --

/// What the home of the panel asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeAction {
    /// Make the map's first version.
    Start,
    /// Bring in a contribution: a `.massifpatch`, or a zone's folder.
    Import,
    /// Write a zone of the map as images, for another tool.
    ExportZone,
}

/// Text laid out at most `width` wide, its lines centred on x = 0 when
/// `centred`.
fn galley(ui: &Ui, text: RichText, width: f32, centred: bool) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    text.append_to(&mut job, ui.style(), egui::FontSelection::Default, Align::Center);
    job.wrap = TextWrapping::wrap_at_width(width);
    if centred {
        job.halign = Align::Center;
    }
    ui.painter().layout_job(job)
}

/// The colour of a card: a shade from the panel toward the header's.
fn card_fill(pal: &Palette) -> Color32 {
    pal.panel.lerp_to_gamma(pal.header, 0.5)
}

/// A small pill of text, its right side at `right`, centred on `y`.
fn badge(ui: &Ui, painter: &Painter, right: f32, y: f32, text: &str, color: Color32) -> Rect {
    let g = galley(ui, RichText::new(text).size(10.0).medium().color(color), f32::INFINITY, false);
    let size = g.size() + vec2(12.0, 4.0);
    let rect = Rect::from_min_size(pos2(right - size.x, y - size.y / 2.0), size);
    painter.rect_filled(rect, theme::PILL, color.gamma_multiply(0.16));
    painter.galley(rect.min + vec2(6.0, 2.0), g, color);
    rect
}

/// A pill button, drawn only (the illustrations are not to be clicked):
/// filled in the text colour when `primary`, outlined otherwise.
fn pill(ui: &Ui, painter: &Painter, right: f32, y: f32, text: &str, primary: bool) -> Rect {
    let pal = Palette::of(ui);
    let ink = if primary { if pal.dark { Color32::from_rgb(12, 12, 14) } else { Color32::WHITE } } else { pal.text };
    let g = galley(ui, RichText::new(text).size(10.0).medium().color(ink), f32::INFINITY, false);
    let size = g.size() + vec2(16.0, 6.0);
    let rect = Rect::from_min_size(pos2(right - size.x, y - size.y / 2.0), size);
    if primary {
        painter.rect_filled(rect, theme::PILL, pal.text);
    } else {
        painter.rect_stroke(rect, theme::PILL, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    }
    painter.galley(rect.min + vec2(8.0, 3.0), g, ink);
    rect
}

/// A name above a line of grey, the name's left at `x`, both centred on `y`.
fn two_lines(ui: &Ui, painter: &Painter, x: f32, y: f32, name: &str, meta: &str, width: f32) {
    let pal = Palette::of(ui);
    let a = galley(ui, RichText::new(name).size(11.5).medium().color(pal.text), width, false);
    let b = galley(ui, RichText::new(meta).size(10.0).color(pal.weak), width, false);
    let top = y - (a.size().y + b.size().y) / 2.0;
    let h = a.size().y;
    painter.galley(pos2(x, top), a, pal.text);
    painter.galley(pos2(x, top + h), b, pal.weak);
}

/// The first card's picture: versions in a line, a branch leaving one.
fn mini_versions(ui: &Ui, painter: &Painter, r: Rect) {
    let pal = Palette::of(ui);
    let row = (r.height() - 16.0) / 3.0;
    let y = |i: usize| r.top() + 8.0 + row * (i as f32 + 0.5);
    let (main, branch) = (r.left() + 20.0, r.left() + 34.0);
    let line = Stroke::new(1.5, pal.weak.gamma_multiply(0.7));
    painter.line_segment([pos2(main, y(1)), pos2(main, y(2))], line);
    let curve = [pos2(main, y(1)), pos2(main, y(1) - row * 0.6), pos2(branch, y(0) + row * 0.5), pos2(branch, y(0))];
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(curve, false, Color32::TRANSPARENT, Stroke::new(1.5, pal.accent)));
    painter.circle_filled(pos2(branch, y(0)), 4.0, pal.accent);
    painter.circle_filled(pos2(main, y(1)), 4.0, pal.text);
    painter.circle_filled(pos2(main, y(2)), 4.0, pal.text);
    let x = branch + 14.0;
    let room = r.right() - x - 90.0;
    two_lines(ui, painter, x, y(0), tr!("Riverlands"), tr!("Jack · 64 tiles"), room);
    two_lines(ui, painter, x, y(1), tr!("Before the rivers"), tr!("You · published"), room);
    two_lines(ui, painter, x, y(2), tr!("Base terrain"), tr!("You · 12 Nov"), room);
    badge(ui, painter, r.right() - 10.0, y(0), "jack/riverlands", pal.accent);
    badge(ui, painter, r.right() - 10.0, y(1), tr!("Published"), pal.success);
}

/// The second card's picture: a `.massifpatch` ready to send.
fn mini_patch(ui: &Ui, painter: &Painter, r: Rect) {
    let pal = Palette::of(ui);
    let inner = r.shrink2(vec2(12.0, 0.0));
    let total = 42.0 + 8.0 + 24.0;
    let top = r.center().y - total / 2.0;
    let chip = Rect::from_min_size(pos2(inner.left(), top), vec2(inner.width(), 42.0));
    painter.rect(chip, theme::TILE_RADIUS, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(chip.left() + 22.0, chip.center().y), vec2(24.0, 24.0));
    painter.rect_filled(icon, 6, pal.accent.gamma_multiply(0.16));
    painter.text(icon.center(), egui::Align2::CENTER_CENTER, theme::glyph(Icon::File), FontId::proportional(13.0), pal.accent);
    two_lines(ui, painter, icon.right() + 10.0, chip.center().y, "riverlands.massifpatch", tr!("64 tiles · 0.9 MB"), chip.right() - icon.right() - 20.0);
    let y = chip.bottom() + 8.0 + 12.0;
    let send = pill(ui, painter, inner.right(), y, tr!("Export"), true);
    let field = Rect::from_min_max(pos2(inner.left(), y - 12.0), pos2(send.left() - 8.0, y + 12.0));
    painter.rect(field, theme::PILL, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let text = galley(ui, RichText::new(tr!("Rivers of the northern valley")).size(10.0).color(pal.weak), field.width() - 20.0, false);
    let clip = painter.with_clip_rect(field.shrink(1.0));
    clip.galley(pos2(field.left() + 10.0, y - text.size().y / 2.0), text, pal.weak);
}

/// The third card's picture: a little map with tiles new, changed and in
/// conflict, and an area to accept.
fn mini_review(ui: &Ui, painter: &Painter, r: Rect) {
    let pal = Palette::of(ui);
    let map = Rect::from_min_max(r.min + vec2(10.0, 10.0), pos2(r.right() - 10.0, r.bottom() - 36.0));
    painter.rect_filled(map, 6, pal.canvas);
    let clip = painter.with_clip_rect(map);
    // A relief in greys, the same at every frame.
    let cell = 7.0;
    let (nx, nz) = ((map.width() / cell).ceil() as i32, (map.height() / cell).ceil() as i32);
    for j in 0..nz {
        for i in 0..nx {
            let h = massif_core::compose::hash2(i / 2, j / 2, 7) as f32 / u32::MAX as f32 * 0.5 + massif_core::compose::hash2(i, j, 9) as f32 / u32::MAX as f32 * 0.25;
            let c = pal.canvas.lerp_to_gamma(pal.weak, h * 0.55);
            clip.rect_filled(Rect::from_min_size(map.min + vec2(i as f32 * cell, j as f32 * cell), vec2(cell, cell)), 0, c);
        }
    }
    let t = 21.0;
    let tile = |x: i32, z: i32, c: Color32| {
        let rect = Rect::from_min_size(map.min + vec2(8.0 + x as f32 * t, 4.0 + z as f32 * t), vec2(t, t));
        clip.rect(rect, 0, c.gamma_multiply(0.3), Stroke::new(1.5, c), StrokeKind::Inside);
    };
    for (x, z) in [(1, 1), (2, 1), (2, 2)] {
        tile(x, z, pal.accent);
    }
    for (x, z) in [(7, 1), (7, 2), (8, 2)] {
        tile(x, z, pal.success);
    }
    tile(5, 0, pal.danger);
    let y = r.bottom() - 18.0;
    painter.circle_filled(pos2(r.left() + 16.0, y), 3.5, pal.accent);
    let name = galley(ui, RichText::new(tr!("1 · North valley")).size(10.5).medium().color(pal.text), r.width() / 2.0, false);
    painter.galley(pos2(r.left() + 26.0, y - name.size().y / 2.0), name, pal.text);
    let accept = pill(ui, painter, r.right() - 10.0, y, tr!("Accept"), true);
    pill(ui, painter, accept.left() - 6.0, y, tr!("Reject"), false);
}

/// What draws the picture of a card of the home.
type Picture = fn(&Ui, &Painter, Rect);

/// A card of the home: its picture, a title and a few lines.
fn card(ui: &mut Ui, width: f32, picture: f32, text_h: f32, title: &str, about: &str, draw: Picture) {
    let pal = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0 + picture + 14.0 + text_h + 14.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect(rect, theme::CARD_RADIUS, card_fill(&pal), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let pic = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(width - 16.0, picture));
    painter.rect(pic, theme::GROUP_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    draw(ui, &painter, pic);
    let t = galley(ui, RichText::new(title).size(13.5).bold().color(pal.text), width - 24.0, true);
    let th = t.size().y;
    painter.galley(pos2(rect.center().x, pic.bottom() + 14.0), t, pal.text);
    let d = galley(ui, RichText::new(about).size(12.0).color(pal.weak), width - 32.0, true);
    painter.galley(pos2(rect.center().x, pic.bottom() + 14.0 + th + 5.0), d, pal.weak);
}

/// The panel of a map without a version yet: what versions are for, and
/// to start one, bring in a contribution or send a zone to another tool.
/// Three cards side by side where the panel is wide, one above the other
/// where it is narrow.
pub fn home(ui: &mut Ui, busy: bool, can_edit: bool) -> Option<HomeAction> {
    let pal = Palette::of(ui);
    let mut action = None;
    let width = ui.available_width();
    let wide = width >= 760.0;
    ui.add_space(if wide { 36.0 } else { 6.0 });
    ui.vertical_centered(|ui| {
        // The label: a pill hugging its words.
        let parts = [
            galley(ui, RichText::new(theme::glyph(Icon::History).to_string()).size(12.0).color(pal.text), f32::INFINITY, false),
            galley(ui, RichText::new(tr!("Versions")).size(12.0).medium().color(pal.text), f32::INFINITY, false),
            galley(ui, RichText::new(tr!("Beta")).size(11.0).medium().color(pal.accent), f32::INFINITY, false),
        ];
        let inner = parts.iter().map(|g| g.size().x).sum::<f32>() + 10.0;
        let height = parts.iter().map(|g| g.size().y).fold(0.0, f32::max) + 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(inner + 20.0, height), Sense::hover());
        let painter = ui.painter();
        painter.rect(rect, theme::PILL, pal.header, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let mut x = rect.left() + 10.0;
        for g in parts {
            let w = g.size().x;
            painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, pal.text);
            x += w + 5.0;
        }
        ui.add_space(if wide { 16.0 } else { 12.0 });
        let room = width.min(if wide { 560.0 } else { 340.0 });
        ui.allocate_ui_with_layout(vec2(room, 0.0), egui::Layout::top_down(Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(tr!("Work on one map, together")).size(if wide { 26.0 } else { 18.0 }).medium()).halign(Align::Center).wrap());
            ui.add_space(8.0);
            let about = tr!("One person keeps the map. Others branch from a version, send their changes as a small .massifpatch file, and you review them tile by tile on the map.");
            ui.add(egui::Label::new(RichText::new(about).size(if wide { 14.0 } else { 12.5 }).color(pal.weak)).halign(Align::Center).wrap());
        });
        ui.add_space(if wide { 20.0 } else { 14.0 });
        let label = if busy { tr!("Saving a point…") } else { tr!("Start a version") };
        let start = ui.add_enabled(!busy && can_edit, theme::primary_button(ui, format!("{}  {label}", theme::glyph(Icon::Plus))));
        if start.on_hover_text(tr!("Save the map as it is as its first version, to compare with, go back to, or share")).clicked() {
            action = Some(HomeAction::Start);
        }
        ui.add_space(6.0);
        let links = [(Icon::Import, tr!("Import a contribution…"), HomeAction::Import), (Icon::ImageDown, tr!("Export a zone…"), HomeAction::ExportZone)];
        let link = |ui: &mut Ui, (icon, text, what): (Icon, &str, HomeAction), action: &mut Option<HomeAction>| {
            let b = egui::Button::new(RichText::new(format!("{}  {text}", theme::glyph(icon))).size(12.5).color(pal.weak)).frame(false);
            if ui.add_enabled(can_edit, b).clicked() {
                *action = Some(what);
            }
        };
        let measure = |text: &str| ui.painter().layout_no_wrap(format!("XX  {text}"), FontId::proportional(12.5), pal.weak).size().x + 12.0;
        let both = measure(links[0].1) + measure(links[1].1) + 8.0;
        if both <= width {
            ui.allocate_ui_with_layout(vec2(both, 22.0), egui::Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for l in links {
                    link(ui, l, &mut action);
                }
            });
        } else {
            for l in links {
                link(ui, l, &mut action);
            }
        }
    });
    ui.add_space(if wide { 36.0 } else { 16.0 });
    let cards: [(&str, &str, Picture); 3] = [
        (tr!("Branch from a version"), tr!("Contributors start from a version you published and work on a branch of their own. Your map stays yours."), mini_versions),
        (tr!("Send your changes"), tr!("Only the changed tiles travel: a small .massifpatch, 2 MB for 1,000 × 1,000 blocks, sent by hand or through a shared folder."), mini_patch),
        (tr!("Review on the map"), tr!("New, changed and conflicting tiles show on the map. Accept or reject each area; you can always go back."), mini_review),
    ];
    let gap = 16.0;
    let (per_row, card_w) = if wide { (3, ((width - 2.0 * gap) / 3.0).min(320.0)) } else { (1, width.min(380.0)) };
    let picture = if wide { 150.0 } else { 120.0 };
    // Every card as tall as the tallest text.
    let text_h = cards
        .iter()
        .map(|(t, a, _)| galley(ui, RichText::new(*t).size(13.5).bold(), card_w - 24.0, true).size().y + 5.0 + galley(ui, RichText::new(*a).size(12.0), card_w - 32.0, true).size().y)
        .fold(0.0, f32::max);
    let row_w = per_row as f32 * card_w + (per_row - 1) as f32 * gap;
    for chunk in cards.chunks(per_row) {
        ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(Align::Center), |ui| {
            ui.allocate_ui_with_layout(vec2(row_w, 0.0), egui::Layout::left_to_right(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (title, about, draw) in chunk {
                    card(ui, card_w, picture, text_h, title, about, *draw);
                }
            });
        });
        ui.add_space(gap * 0.75);
    }
    action
}

// ---------------------------------------------------------------- review --

/// What the owner decided for an area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pending,
    Accepted,
    Rejected,
    /// An area in conflict, settled: the tiles taken from the contribution.
    Resolved(HashSet<TilePos>),
}

/// A contribution read with the numbers of the map's registry and
/// classified against the map, off the UI thread, to review.
pub struct ReviewReady {
    pub patch: Arc<Patch>,
    pub review: Review,
    pub registry: Registry,
    pub added: Vec<String>,
    pub source: String,
    pub base_known: bool,
    pub revision: u64,
}

impl ReviewReady {
    /// `patch` read with the numbers of the registry of `map` (cold, at
    /// `revision`) and classified against it.
    pub fn of(patch: Patch, map: &massif_core::World, source: String, base_known: bool, revision: u64) -> Self {
        let mut registry = map.registry.clone();
        let (patch, remap) = patch.remapped(&mut registry);
        let review = massif_core::merge::classify(map, &patch);
        Self {
            patch: Arc::new(patch),
            review,
            registry,
            added: remap.added,
            source,
            base_known,
            revision,
        }
    }
}

/// A contribution being reviewed.
pub struct Reviewing {
    pub patch: Arc<Patch>,
    pub review: Review,
    /// By area.
    pub verdicts: Vec<Verdict>,
    /// By changed setting: taken.
    pub settings: Vec<bool>,
    /// What it came from, as the panel names it.
    pub source: String,
    /// The map's registry with what the contribution's tiles use (see
    /// [`massif_core::patch::Remap`]), and the names it gains.
    pub registry: Registry,
    pub added: Vec<String>,
    /// Whether the map has the version it starts from.
    pub base_known: bool,
    /// The map's revision it was classified at.
    pub revision: u64,
    /// The area last picked in the list.
    pub selected: Option<usize>,
    overlay: Option<(egui::TextureHandle, [i32; 4], u64)>,
}

/// What the review asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewAction {
    /// Show area `i` on the map.
    Show(usize),
    /// Open the resolver of area `i`.
    Resolve(usize),
    Apply,
    Cancel,
}

impl Reviewing {
    pub fn new(ready: ReviewReady) -> Self {
        let ReviewReady { patch, review, registry, added, source, base_known, revision } = ready;
        Self {
            verdicts: vec![Verdict::Pending; review.areas.len()],
            settings: vec![false; review.settings.len()],
            patch,
            review,
            source,
            registry,
            added,
            base_known,
            revision,
            selected: None,
            overlay: None,
        }
    }

    /// What the owner decided, for [`massif_core::merge::apply`].
    pub fn decision(&self) -> Decision {
        let mut d = Decision::default();
        for (area, verdict) in self.review.areas.iter().zip(&self.verdicts) {
            match verdict {
                Verdict::Accepted => d.take.extend(area.tiles.iter().map(|t| (area.dim, t.0))),
                Verdict::Resolved(theirs) => d.take.extend(theirs.iter().map(|&p| (area.dim, p))),
                Verdict::Pending | Verdict::Rejected => {}
            }
        }
        d.settings = self.review.settings.iter().zip(&self.settings).filter(|(_, on)| **on).map(|(s, _)| s.key.clone()).collect();
        d
    }

    /// How many areas will be merged.
    pub fn taken(&self) -> usize {
        self.verdicts.iter().filter(|v| matches!(v, Verdict::Accepted | Verdict::Resolved(_))).count()
    }

    /// Accept every area that is not in conflict and not decided yet.
    pub fn accept_clean(&mut self) {
        for (area, verdict) in self.review.areas.iter().zip(&mut self.verdicts) {
            if *verdict == Verdict::Pending && area.status() != Status::Conflict {
                *verdict = Verdict::Accepted;
            }
        }
    }

    /// The title the panel gives it.
    pub fn title(&self) -> &'static str {
        match self.patch.manifest.kind {
            Kind::External => tr!("Zone from another tool"),
            Kind::Update => tr!("Update of the map"),
            Kind::Contribution => tr!("Contribution"),
        }
    }

    /// The tiles it changes over dimension `dim` of the 2D map, coloured by
    /// what they are and what was decided, and the blocks they cover.
    pub fn overlay(&mut self, ctx: &egui::Context, dim: usize) -> Option<(egui::TextureHandle, [i32; 4])> {
        let visuals = ctx.global_style().visuals.clone();
        let pal = Palette::themed(visuals.dark_mode, theme::is_high(&visuals));
        let areas: Vec<usize> = (0..self.review.areas.len()).filter(|&i| self.review.areas[i].dim == dim).collect();
        if areas.is_empty() {
            return None;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            dim.hash(&mut h);
            self.selected.hash(&mut h);
            pal.accent.to_array().hash(&mut h);
            pal.dark.hash(&mut h);
            for v in &self.verdicts {
                match v {
                    Verdict::Pending => 0u8.hash(&mut h),
                    Verdict::Accepted => 1u8.hash(&mut h),
                    Verdict::Rejected => 2u8.hash(&mut h),
                    Verdict::Resolved(t) => {
                        3u8.hash(&mut h);
                        let mut t: Vec<_> = t.iter().map(|p| (p.z, p.x)).collect();
                        t.sort_unstable();
                        t.hash(&mut h);
                    }
                }
            }
            h.finish()
        };
        if let Some((tex, rect, k)) = &self.overlay
            && *k == key
        {
            return Some((tex.clone(), *rect));
        }
        let tiles = areas.iter().flat_map(|&i| self.review.areas[i].tiles.iter().map(|t| t.0));
        let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for p in tiles {
            (x0, z0, x1, z1) = (x0.min(p.x), z0.min(p.z), x1.max(p.x), z1.max(p.z));
        }
        // Pixels a tile: its outline drawn at the edge of its square.
        let mut px = 16usize;
        while px > 4 && ((x1 - x0 + 1) as usize * px).max((z1 - z0 + 1) as usize * px) > 4096 {
            px /= 2;
        }
        let (w, h) = ((x1 - x0 + 1) as usize * px, (z1 - z0 + 1) as usize * px);
        let mut pixels = vec![Color32::TRANSPARENT; w * h];
        let grey = if pal.dark { Color32::from_gray(190) } else { Color32::from_gray(70) };
        for &i in &areas {
            let area = &self.review.areas[i];
            let verdict = &self.verdicts[i];
            let selected = self.selected == Some(i);
            for &(pos, status) in &area.tiles {
                let c = match status {
                    Status::New => pal.success,
                    Status::Changed | Status::Same => pal.accent,
                    Status::Conflict => pal.danger,
                };
                let (fill, edge) = match verdict {
                    Verdict::Pending => (c.gamma_multiply(0.22), c),
                    Verdict::Accepted => (c.gamma_multiply(0.42), c),
                    Verdict::Rejected => (grey.gamma_multiply(0.08), grey.gamma_multiply(0.6)),
                    Verdict::Resolved(theirs) if theirs.contains(&pos) => (pal.accent.gamma_multiply(0.42), pal.accent),
                    Verdict::Resolved(_) => (grey.gamma_multiply(0.12), grey.gamma_multiply(0.7)),
                };
                let edge = if selected { edge.lerp_to_gamma(pal.text, 0.35) } else { edge };
                let width = if selected { 2 } else { 1 }.min(px / 4).max(1);
                let (ox, oz) = ((pos.x - x0) as usize * px, (pos.z - z0) as usize * px);
                for j in 0..px {
                    for k in 0..px {
                        let border = j < width || k < width || j >= px - width || k >= px - width;
                        pixels[(oz + j) * w + ox + k] = if border { edge } else { fill };
                    }
                }
            }
        }
        let image = egui::ColorImage::new([w, h], pixels);
        let texture = ctx.load_texture("contribution review", image, egui::TextureOptions::NEAREST);
        let rect = [x0 * TILE_SIZE, z0 * TILE_SIZE, (x1 + 1) * TILE_SIZE, (z1 + 1) * TILE_SIZE];
        self.overlay = Some((texture.clone(), rect, key));
        Some((texture, rect))
    }

    /// The review in the panel.
    pub fn panel(&mut self, ui: &mut Ui, can_edit: bool) -> Option<ReviewAction> {
        let pal = Palette::of(ui);
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.title()).size(15.0).bold());
            beta(ui);
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new(tr!("Close")).size(12.0).color(pal.weak)).frame(false)).on_hover_text(tr!("Leave the review: the map stays as it is")).clicked() {
                    action = Some(ReviewAction::Cancel);
                }
            });
        });
        ui.add(egui::Label::new(RichText::new(&self.source).size(11.5).color(pal.weak)).truncate());
        ui.add_space(6.0);
        let m = &self.patch.manifest;
        well(ui, |ui| {
            if m.kind != Kind::External {
                let from = if m.author.is_empty() { tr!("Someone").to_owned() } else { m.author.clone() };
                row(ui, tr!("From"), |ui| {
                    ui.add(egui::Label::new(RichText::new(from).size(12.0).color(pal.weak)).truncate());
                });
            }
            row(ui, tr!("Based on"), |ui| {
                if self.base_known {
                    status_pill(ui, tr!("In this map"), pal.success);
                } else {
                    status_pill(ui, tr!("Not in this map"), pal.weak);
                }
                ui.add(egui::Label::new(RichText::new(&m.base.name).size(12.0).color(pal.weak)).truncate());
            });
            row(ui, tr!("Changes"), |ui| {
                let size = bytes(self.patch.tile_bytes());
                ui.label(RichText::new(trn!("{n} tile · {size}", "{n} tiles · {size}", n = self.patch.tiles(), size)).size(12.0).color(pal.weak));
            });
            if !m.message.is_empty() {
                row(ui, tr!("Message"), |ui| {
                    ui.add(egui::Label::new(RichText::new(&m.message).size(12.0).color(pal.weak)).truncate());
                });
            }
        });
        if !self.added.is_empty() {
            ui.add_space(4.0);
            crate::panels::hint(ui, &tr!("The map gains what it uses: {names}", names = self.added.join(", ")));
        }
        ui.add_space(8.0);
        let (new, changed, conflicts) = self.review.counts();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if new > 0 {
                legend(ui, pal.success, &trn!("{n} new", "{n} new", n = new));
            }
            if changed > 0 {
                legend(ui, pal.accent, &trn!("{n} changed", "{n} changed", n = changed));
            }
            if conflicts > 0 {
                legend(ui, pal.danger, &trn!("{n} in conflict", "{n} in conflict", n = conflicts));
            }
        });
        ui.add_space(4.0);
        if self.review.areas.is_empty() {
            crate::panels::hint(ui, tr!("Nothing in it differs from the map: it has every change already."));
        } else {
            well(ui, |ui| {
                for i in 0..self.review.areas.len() {
                    if i > 0 {
                        let r = ui.available_rect_before_wrap();
                        ui.painter().hline(r.x_range().expand(12.0), r.top(), Stroke::new(1.0, pal.border));
                    }
                    if let Some(a) = self.area_row(ui, i, can_edit) {
                        action = Some(a);
                    }
                }
            });
        }
        if !self.review.settings.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(tr!("Settings of the map it changes")).size(11.5).color(pal.weak).medium());
            ui.add_space(4.0);
            well(ui, |ui| {
                for (k, s) in self.review.settings.iter().enumerate() {
                    let label = setting_name(&s.key);
                    row(ui, label, |ui| {
                        switch(ui, &mut self.settings[k]);
                        let text = if s.conflict() {
                            tr!("{theirs} (yours: {mine})", theirs = value(&s.theirs), mine = value(&s.mine))
                        } else {
                            tr!("{base} → {theirs}", base = value(&s.base), theirs = value(&s.theirs))
                        };
                        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal.weak)).truncate());
                    });
                }
            });
            crate::panels::hint(ui, tr!("A setting changes only if you switch it on."));
        }
        ui.add_space(8.0);
        let clean = self.review.areas.iter().zip(&self.verdicts).any(|(a, v)| *v == Verdict::Pending && a.status() != Status::Conflict);
        if ui.add_enabled(clean && can_edit, egui::Button::new(tr!("Accept all without conflict")).truncate()).clicked() {
            self.accept_clean();
        }
        ui.add_space(10.0);
        crate::panels::hint(ui, tr!("Applying makes a new version of the map, so you can always go back."));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let n = self.taken();
            let taken_settings = self.settings.iter().filter(|s| **s).count();
            let label = if n == 0 && taken_settings > 0 { tr!("Apply the settings").to_owned() } else { trn!("Apply {n} area", "Apply {n} areas", n = n) };
            if ui.add_enabled(can_edit && (n > 0 || taken_settings > 0), theme::primary_button(ui, label)).clicked() {
                action = Some(ReviewAction::Apply);
            }
            if ui.add(egui::Button::new(tr!("Cancel")).truncate()).clicked() {
                action = Some(ReviewAction::Cancel);
            }
        });
        action
    }

    /// A row of the areas' list.
    fn area_row(&mut self, ui: &mut Ui, i: usize, can_edit: bool) -> Option<ReviewAction> {
        let pal = Palette::of(ui);
        let area = &self.review.areas[i];
        let status = area.status();
        let (new, changed, conflicts) = area.counts();
        let c = match status {
            Status::New => pal.success,
            Status::Conflict => pal.danger,
            _ => pal.accent,
        };
        let mut action = None;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
        if response.hovered() || self.selected == Some(i) {
            ui.painter().rect_filled(rect.expand2(vec2(6.0, -3.0)), theme::TILE_RADIUS, pal.accent_soft);
        }
        if response.clicked() {
            self.selected = Some(i);
            action = Some(ReviewAction::Show(i));
        }
        let response = response.on_hover_text(tr!("Show it on the map"));
        let _ = response;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(Align::Center)));
        let ui = &mut child;
        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
        ui.painter().circle_filled(dot.center(), 4.5, c);
        let what = if conflicts > 0 {
            trn!("{n} tile changed on both sides", "{n} tiles changed on both sides", n = conflicts)
        } else if changed > 0 && new > 0 {
            tr!("{changed} changed, {new} new", changed, new)
        } else if new > 0 {
            trn!("{n} new tile", "{n} new tiles", n = new)
        } else {
            trn!("{n} tile changed", "{n} tiles changed", n = changed)
        };
        let verdict = self.verdicts[i].clone();
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            match &verdict {
                Verdict::Pending if status == Status::Conflict => {
                    if ui.add_enabled(can_edit, egui::Button::new(RichText::new(tr!("Resolve…")).color(pal.danger)).truncate()).clicked() {
                        action = Some(ReviewAction::Resolve(i));
                    }
                }
                Verdict::Pending => {
                    if ui.add_enabled(can_edit, theme::primary_button(ui, tr!("Accept"))).clicked() {
                        self.verdicts[i] = Verdict::Accepted;
                    }
                    if ui.add_enabled(can_edit, egui::Button::new(tr!("Reject")).truncate()).clicked() {
                        self.verdicts[i] = Verdict::Rejected;
                    }
                }
                done => {
                    if ui.add(egui::Button::new(RichText::new(tr!("Undo")).size(12.0).color(pal.weak)).frame(false)).clicked() {
                        self.verdicts[i] = Verdict::Pending;
                    }
                    let (text, color) = match done {
                        Verdict::Accepted => (tr!("Accepted"), pal.success),
                        Verdict::Rejected => (tr!("Rejected"), pal.weak),
                        _ => (tr!("Resolved"), pal.accent),
                    };
                    status_pill(ui, text, color);
                    if matches!(done, Verdict::Resolved(_)) && ui.add(egui::Button::new(RichText::new(tr!("Reopen")).size(12.0).color(pal.weak)).frame(false)).clicked() {
                        action = Some(ReviewAction::Resolve(i));
                    }
                }
            }
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(egui::Label::new(RichText::new(tr!("Area {n}", n = area.number)).size(12.5).medium()).truncate());
                    ui.add(egui::Label::new(RichText::new(what).size(11.0).color(pal.weak)).truncate());
                });
            });
        });
        action
    }
}

/// The "Beta" mark beside a title.
pub fn beta(ui: &mut Ui) {
    let pal = Palette::of(ui);
    status_pill(ui, tr!("Beta"), pal.accent);
}

/// A small pill saying a state, in `color`, as tall as its words whatever
/// the room around it.
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) {
    let g = galley(ui, RichText::new(text).size(10.5).medium().color(color), f32::INFINITY, false);
    let (rect, _) = ui.allocate_exact_size(g.size() + vec2(14.0, 2.0), Sense::hover());
    ui.painter().rect_filled(rect, theme::PILL, color.gamma_multiply(0.16));
    ui.painter().galley(rect.min + vec2(7.0, 1.0), g, color);
}

/// A dot and a word, for the legend of the review.
fn legend(ui: &mut Ui, color: Color32, text: &str) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, color);
        ui.label(RichText::new(text).size(11.5).color(pal.weak));
    });
}

/// A size in kilobytes or megabytes.
pub fn bytes(n: u64) -> String {
    if n < 1_000_000 {
        tr!("{kb} KB", kb = n.div_ceil(1000))
    } else {
        tr!("{mb:.1} MB", mb = n as f64 / 1e6)
    }
}

/// How the panel names a setting of the map.
pub fn setting_name(key: &str) -> &'static str {
    match key {
        "seed" => tr!("Seed"),
        "spawn" => tr!("Spawn point"),
        "sea_level" => tr!("Sea level"),
        "lava" => tr!("Lava instead of water"),
        "graph" => tr!("Terrain graph"),
        "ores" => tr!("Ores"),
        "gradients" => tr!("Gradients"),
        "fill" => tr!("Fill terrain"),
        "palette" => tr!("Block palette"),
        "smooth" => tr!("Smooth terrain"),
        _ => tr!("Other setting"),
    }
}

/// A setting's value, in a few words.
fn value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => tr!("none").to_owned(),
        serde_json::Value::Bool(b) => if *b { tr!("on") } else { tr!("off") }.to_owned(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) if a.iter().all(|x| x.is_number()) && a.len() <= 3 => a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "),
        _ => tr!("changed").to_owned(),
    }
}

// -------------------------------------------------------------- resolver --

/// How a conflict is settled, for the whole area or tile by tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Mine,
    Theirs,
    TileByTile,
}

/// The resolver of an area in conflict: its tiles mine or theirs, with
/// pictures of the area as the map has it, as the contribution has it and
/// as it would be.
pub struct Resolver {
    pub area: usize,
    pub keep: Keep,
    /// The tiles in conflict, each taken from the contribution or not.
    pub tiles: Vec<(TilePos, bool)>,
    /// The area's other tiles (changed by the contribution alone) taken too.
    pub others: bool,
    /// The pictures, and the choice they were drawn for.
    pub pictures: Option<([egui::TextureHandle; 3], Vec<bool>, bool)>,
}

/// What the resolver wants done.
pub enum ResolverChoice {
    Keep,
    Cancel,
    /// The tiles of the area to take from the contribution.
    Done(HashSet<TilePos>),
}

impl Resolver {
    /// The resolver of area `area` of `reviewing`, as last decided.
    pub fn new(reviewing: &Reviewing, area: usize) -> Self {
        let a = &reviewing.review.areas[area];
        let taken = match &reviewing.verdicts[area] {
            Verdict::Resolved(t) => Some(t.clone()),
            _ => None,
        };
        let tiles: Vec<(TilePos, bool)> = a.tiles.iter().filter(|t| t.1 == Status::Conflict).map(|t| (t.0, taken.as_ref().is_some_and(|s| s.contains(&t.0)))).collect();
        let keep = match taken {
            None => Keep::Mine,
            Some(_) if tiles.iter().all(|t| t.1) => Keep::Theirs,
            Some(_) if tiles.iter().all(|t| !t.1) => Keep::Mine,
            Some(_) => Keep::TileByTile,
        };
        let others = match &reviewing.verdicts[area] {
            Verdict::Resolved(t) => a.tiles.iter().filter(|x| x.1 != Status::Conflict).all(|x| t.contains(&x.0)),
            _ => true,
        };
        Self { area, keep, tiles, others, pictures: None }
    }

    /// The tiles taken from the contribution, as chosen.
    pub fn taken(&self, reviewing: &Reviewing) -> HashSet<TilePos> {
        let a = &reviewing.review.areas[self.area];
        let mut out: HashSet<TilePos> = self
            .tiles
            .iter()
            .filter(|(_, theirs)| match self.keep {
                Keep::Mine => false,
                Keep::Theirs => true,
                Keep::TileByTile => *theirs,
            })
            .map(|t| t.0)
            .collect();
        if self.others {
            out.extend(a.tiles.iter().filter(|t| t.1 != Status::Conflict).map(|t| t.0));
        }
        out
    }

    /// What the pictures show: each tile in conflict taken or not.
    fn choice(&self) -> Vec<bool> {
        self.tiles
            .iter()
            .map(|t| match self.keep {
                Keep::Mine => false,
                Keep::Theirs => true,
                Keep::TileByTile => t.1,
            })
            .collect()
    }

    /// Whether the pictures are not drawn for the choice as it is.
    pub fn needs_pictures(&self) -> bool {
        self.pictures.as_ref().is_none_or(|(_, c, o)| *c != self.choice() || *o != self.others)
    }

    /// Draw the three pictures again if the choice changed: the area and a
    /// tile around it, from `map` (cold) as it is, with the contribution's
    /// tiles, and as chosen.
    pub fn draw_pictures(&mut self, ctx: &egui::Context, map: &massif_core::World, reviewing: &Reviewing) {
        if !self.needs_pictures() {
            return;
        }
        let choice = self.choice();
        let area = &reviewing.review.areas[self.area];
        let all: HashSet<(usize, TilePos)> = area.tiles.iter().map(|t| (area.dim, t.0)).collect();
        let chosen: HashSet<(usize, TilePos)> = self.taken(reviewing).into_iter().map(|p| (area.dim, p)).collect();
        let world_with = |take: HashSet<(usize, TilePos)>| {
            let (meta, _, dims) = massif_core::merge::apply(map, &reviewing.patch, &Decision { take, settings: HashSet::new() });
            massif_core::World { meta, registry: reviewing.registry.clone(), dims, points: Vec::new() }
        };
        let theirs = world_with(all);
        let result = world_with(chosen);
        let mine = massif_core::World { meta: map.meta.clone(), registry: map.registry.clone(), dims: map.dims.clone(), points: Vec::new() };
        let pictures = [mine, theirs, result].map(|w| {
            let crop = crop(&w, area.dim, area.rect());
            let img = crop.as_ref().and_then(|c| massif_core::thumbnail::render(c, 240));
            let image = match img {
                Some(img) => egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()),
                None => egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]),
            };
            ctx.load_texture("resolver picture", image, egui::TextureOptions::LINEAR)
        });
        self.pictures = Some((pictures, choice, self.others));
    }

    /// The dialog. `reviewing` is the review it settles an area of.
    pub fn ui(&mut self, ui: &mut Ui, reviewing: &Reviewing) -> ResolverChoice {
        let pal = Palette::of(ui);
        let mut choice = ResolverChoice::Keep;
        let area = &reviewing.review.areas[self.area];
        ui.set_width(if ui.ctx().content_rect().width() >= 900.0 { 820.0 } else { 560.0 });
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 5.0, pal.danger);
            ui.label(RichText::new(tr!("Area {n}", n = area.number)).size(16.0).bold());
            status_pill(ui, tr!("Conflict"), pal.danger);
        });
        ui.add_space(4.0);
        let about = tr!("You and the contribution both changed these tiles since “{base}”. Keep your version, theirs, or choose tile by tile.", base = reviewing.patch.manifest.base.name);
        ui.add(egui::Label::new(RichText::new(about).size(12.5).color(pal.weak)).wrap());
        ui.add_space(10.0);
        if let Some((pictures, _, _)) = &self.pictures {
            let w = ((ui.available_width() - 24.0) / 3.0).floor();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let captions = [(tr!("Mine"), pal.text), (tr!("Theirs"), pal.accent), (tr!("Result"), pal.success)];
                for (k, (caption, c)) in captions.into_iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, c);
                            ui.label(RichText::new(caption).size(12.5).medium());
                        });
                        let tex = &pictures[k];
                        let size = tex.size_vec2();
                        let scale = (w / size.x).min(180.0 / size.y);
                        let (rect, _) = ui.allocate_exact_size(vec2(w, 180.0), Sense::hover());
                        ui.painter().rect_filled(rect, theme::TILE_RADIUS, pal.canvas);
                        let r = Rect::from_center_size(rect.center(), size * scale);
                        ui.painter().image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                        if k == 2 {
                            ui.painter().rect_stroke(rect, theme::TILE_RADIUS, Stroke::new(1.5, pal.accent), StrokeKind::Inside);
                        }
                    });
                }
            });
            ui.add_space(10.0);
        }
        well(ui, |ui| {
            row(ui, tr!("Keep"), |ui| {
                let labels = [tr!("Mine"), tr!("Theirs"), tr!("Tile by tile")];
                let on = [self.keep == Keep::Mine, self.keep == Keep::Theirs, self.keep == Keep::TileByTile];
                if let Some(i) = button_group(ui, "resolver keep", &labels, &on) {
                    self.keep = [Keep::Mine, Keep::Theirs, Keep::TileByTile][i];
                }
            });
            let others = area.tiles.iter().filter(|t| t.1 != Status::Conflict).count();
            if others > 0 {
                row(ui, &trn!("The other {n} tile of the area, changed by them only", "The other {n} tiles of the area, changed by them only", n = others), |ui| {
                    switch(ui, &mut self.others);
                });
            }
            if self.keep == Keep::TileByTile {
                for (pos, theirs) in &mut self.tiles {
                    row(ui, &tr!("Tile {x}, {z}", x = pos.x, z = pos.z), |ui| {
                        let labels = [tr!("Mine"), tr!("Theirs")];
                        if let Some(i) = button_group(ui, &format!("resolver tile {} {}", pos.x, pos.z), &labels, &[!*theirs, *theirs]) {
                            *theirs = i == 1;
                        }
                    });
                }
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button(ui, tr!("Resolve area"))).clicked() {
                choice = ResolverChoice::Done(self.taken(reviewing));
            }
            if ui.add(egui::Button::new(tr!("Cancel")).truncate()).clicked() {
                choice = ResolverChoice::Cancel;
            }
        });
        choice
    }
}

/// The part of dimension `dim` of `world` over `rect` (blocks) and a tile
/// around it, as a world of its own, for a picture; `None` if nothing is
/// there.
fn crop(world: &massif_core::World, dim: usize, rect: [i32; 4]) -> Option<massif_core::World> {
    let d = world.dims.get(dim)?;
    let (x0, z0) = (rect[0].div_euclid(TILE_SIZE) - 1, rect[1].div_euclid(TILE_SIZE) - 1);
    let (x1, z1) = ((rect[2] - 1).div_euclid(TILE_SIZE) + 1, (rect[3] - 1).div_euclid(TILE_SIZE) + 1);
    let inside = |p: &TilePos| p.x >= x0 && p.x <= x1 && p.z >= z0 && p.z <= z1;
    let mut levels = Vec::new();
    for level in &d.levels {
        let mut l = massif_core::level::Level::new(level.id, level.name.clone(), level.kind);
        l.visible = level.visible;
        for (pos, blob) in level.blobs()?.into_iter().filter(|(p, _)| inside(p)) {
            l.insert_blob(pos, blob);
        }
        levels.push(l);
    }
    if levels.first().is_none_or(|g| g.is_empty()) {
        return None;
    }
    let mut out = massif_core::world::Dimension::from_levels(d.kind, d.min_y, d.max_y, levels);
    for (pos, chunk) in d.voxels.iter().filter(|(p, _)| inside(&p.tile())) {
        out.voxels.insert(pos, chunk.clone());
    }
    Some(massif_core::World { meta: world.meta.clone(), registry: world.registry.clone(), dims: vec![out], points: Vec::new() })
}

// ---------------------------------------------------------------- export --

/// The dialog writing the map's changes since a version as a
/// `.massifpatch`.
pub struct ExportForm {
    /// The version they are counted from: an index in the map's points.
    pub base: usize,
    /// The shared folder's name, when the map has one, and whether the
    /// contribution goes into it (rather than to a file chosen).
    pub folder: Option<String>,
    pub into_folder: bool,
    /// Each version's name, and who made it when.
    versions: Vec<(String, String)>,
    pub author: String,
    pub message: String,
}

/// What the export dialog wants done.
pub enum ExportChoice {
    Keep,
    Close,
    Export,
}

impl ExportForm {
    /// The dialog for `world`, by `me`: counted from the last version
    /// someone else made, or else from the first.
    pub fn new(world: &massif_core::World, me: &str) -> Self {
        let now = massif_core::versions::now();
        let base = world.points.iter().rposition(|p| !p.author.is_empty() && p.author != me).unwrap_or(0);
        let versions = world
            .points
            .iter()
            .map(|p| {
                let when = crate::versions::when(p.created, now);
                let meta = if p.author.is_empty() { when } else { format!("{} · {when}", p.author) };
                (p.name.clone(), meta)
            })
            .collect();
        Self {
            base,
            versions,
            author: me.to_owned(),
            message: String::new(),
            folder: None,
            into_folder: false,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> ExportChoice {
        let pal = Palette::of(ui);
        let mut choice = ExportChoice::Keep;
        ui.set_width(460.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::FileOutput).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Export my changes")).size(16.0).bold());
            beta(ui);
        });
        ui.add(egui::Label::new(RichText::new(tr!("The tiles you changed since a version, each with what it was, in a small .massifpatch file: the map's owner brings it in with Versions ▸ Import a contribution… and reviews it on their map.")).size(12.0).color(pal.weak)).wrap());
        ui.add_space(12.0);
        well(ui, |ui| {
            row(ui, tr!("Since"), |ui| {
                let shown = self.versions.get(self.base).map_or_else(String::new, |v| v.0.clone());
                crate::widgets::combo("export since").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                    for (i, (name, meta)) in self.versions.iter().enumerate().rev() {
                        ui.selectable_value(&mut self.base, i, format!("{name}  ·  {meta}"));
                    }
                });
            });
            row(ui, tr!("Your name"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.author).hint_text(tr!("Jack")).desired_width(220.0));
            });
            row(ui, tr!("Message"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.message).hint_text(tr!("Rivers of the northern valley")).desired_width(220.0));
            });
            if let Some(folder) = &self.folder {
                row(ui, &tr!("Into the shared folder “{folder}”", folder), |ui| {
                    switch(ui, &mut self.into_folder);
                });
            }
        });
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let label = if self.folder.is_some() && self.into_folder { tr!("Send") } else { tr!("Export…") };
            if ui.add_enabled(self.base < self.versions.len(), theme::primary_button(ui, label)).clicked() {
                choice = ExportChoice::Export;
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = ExportChoice::Close;
            }
        });
        choice
    }
}

// ----------------------------------------------------------------- share --

/// What the panel's Share part asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareAction {
    /// Write (or send) the map's changes since a version.
    ExportChanges,
    Import,
    ExportZone,
    /// Choose the map's shared folder (or make one).
    ChooseFolder,
    /// Publish the map into its shared folder.
    Publish,
    /// Review the contribution waiting at that place in the list.
    Review(usize),
    /// Set that contribution aside without merging it.
    SetAside(usize),
}

/// The map's shared folder: the depot, whether this person keeps the map,
/// and the contributions waiting in it.
pub struct DepotState {
    pub depot: massif_format::depot::FolderDepot,
    pub owner: bool,
    pub waiting: Vec<massif_format::depot::Waiting>,
    read_at: Option<std::time::Instant>,
}

impl DepotState {
    pub fn new(depot: massif_format::depot::FolderDepot, owner: bool) -> Self {
        let mut state = Self { depot, owner, waiting: Vec::new(), read_at: None };
        state.refresh();
        state
    }

    /// Read the contributions waiting again (the owner's only).
    pub fn refresh(&mut self) {
        use massif_format::depot::Depot;
        if self.owner {
            self.waiting = self.depot.waiting().unwrap_or_default();
        }
        self.read_at = Some(std::time::Instant::now());
    }

    /// Whether the list is old enough to be read again: files come in
    /// through the sync service while the panel shows.
    pub fn due(&self) -> bool {
        self.read_at.is_none_or(|t| t.elapsed().as_secs_f32() > 5.0)
    }
}

/// The panel's Share part: the shared folder, publishing, sending and
/// bringing in changes, the contributions waiting.
pub fn share(ui: &mut Ui, depot: Option<&DepotState>, can_edit: bool) -> Option<ShareAction> {
    use massif_format::depot::Depot;
    let pal = Palette::of(ui);
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("Sharing")).size(11.5).color(pal.weak).medium());
        beta(ui);
    });
    ui.add_space(4.0);
    well(ui, |ui| {
        row(ui, tr!("Shared folder"), |ui| {
            let label = if depot.is_some() { tr!("Change…") } else { tr!("Choose…") };
            if ui
                .add_enabled(can_edit, egui::Button::new(label).truncate())
                .on_hover_text(tr!("A folder shared with Google Drive, Dropbox, iCloud or Git, where the map and its contributions are put"))
                .clicked()
            {
                action = Some(ShareAction::ChooseFolder);
            }
            if let Some(d) = depot {
                ui.add(egui::Label::new(RichText::new(d.depot.describe()).size(12.0).color(pal.weak)).truncate());
            }
        });
        if depot.is_some_and(|d| d.owner) {
            row(ui, tr!("Publish the map"), |ui| {
                if ui.add_enabled(can_edit, egui::Button::new(tr!("Publish")).truncate()).on_hover_text(tr!("Write the map, with its versions, into the shared folder: contributors start from it")).clicked() {
                    action = Some(ShareAction::Publish);
                }
            });
        }
        let (label, button, tip) = if depot.is_some() {
            (tr!("Send my changes"), tr!("Send…"), tr!("The tiles changed since a version, as a .massifpatch put in the shared folder for the map's owner"))
        } else {
            (tr!("Export my changes"), tr!("Export…"), tr!("The tiles changed since a version, as a .massifpatch file to send to the map's owner"))
        };
        row(ui, label, |ui| {
            if ui.add_enabled(can_edit, egui::Button::new(button).truncate()).on_hover_text(tip).clicked() {
                action = Some(ShareAction::ExportChanges);
            }
        });
        row(ui, tr!("Import a contribution"), |ui| {
            if ui.add_enabled(can_edit, egui::Button::new(tr!("Import…")).truncate()).on_hover_text(tr!("A .massifpatch, or the zone.json of a zone brought back from another tool, to review on the map")).clicked() {
                action = Some(ShareAction::Import);
            }
        });
        row(ui, tr!("Export a zone for another tool"), |ui| {
            if ui.add_enabled(can_edit, egui::Button::new(tr!("Export…")).truncate()).on_hover_text(tr!("A part of the map as a heightmap and masks, to edit in World Machine or Gaea and bring back as a contribution")).clicked() {
                action = Some(ShareAction::ExportZone);
            }
        });
    });
    if let Some(d) = depot {
        let info = d.depot.info();
        let keeper = if d.owner {
            tr!("You keep the map.").to_owned()
        } else if info.owner.is_empty() {
            tr!("Someone else keeps the map.").to_owned()
        } else {
            tr!("{owner} keeps the map.", owner = info.owner)
        };
        let published = match &info.published {
            Some(v) => tr!("Published: “{name}”.", name = v.name),
            None => tr!("Not published yet.").to_owned(),
        };
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(format!("{keeper} {published}")).size(11.5).color(pal.weak)).wrap());
    }
    if let Some(d) = depot.filter(|d| d.owner && !d.waiting.is_empty()) {
        ui.add_space(8.0);
        ui.label(RichText::new(trn!("{n} contribution waiting", "{n} contributions waiting", n = d.waiting.len())).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            let now = massif_core::versions::now();
            for (i, w) in d.waiting.iter().enumerate() {
                if i > 0 {
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range().expand(12.0), r.top(), Stroke::new(1.0, pal.border));
                }
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(Align::Center)));
                let ui = &mut child;
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui.add_enabled(can_edit, egui::Button::new(tr!("Review")).truncate()).clicked() {
                    action = Some(ShareAction::Review(i));
                }
                if ui.add_enabled(can_edit, egui::Button::new(RichText::new(tr!("Set aside")).size(12.0).color(pal.weak)).frame(false)).on_hover_text(tr!("Leave it out of the list without merging it")).clicked() {
                    action = Some(ShareAction::SetAside(i));
                }
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let m = &w.manifest;
                        let who = if m.author.is_empty() { tr!("Someone").to_owned() } else { m.author.clone() };
                        let title = if m.message.is_empty() { who } else { format!("{who} · {}", m.message) };
                        ui.add(egui::Label::new(RichText::new(title).size(12.5).medium()).truncate());
                        let meta = format!("{} · {}", crate::versions::when(m.created, now), bytes(w.bytes));
                        ui.add(egui::Label::new(RichText::new(meta).size(11.0).color(pal.weak)).truncate());
                    });
                });
            }
        });
    }
    action
}
