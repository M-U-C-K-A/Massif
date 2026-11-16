//! The guided first start: six short steps over the editor, each a soft
//! highlight of the part of the window it talks about and a small card
//! beside it, with Back, Next and Skip.
//!
//! It shows once, the first time a map is open in Massif, and again only
//! from Help ▸ Take the tour. Escape, Skip or Done end it; with "Don't show
//! again" ticked (as it is at first) it does not come back on its own.
//!
//! The parts of the window mark where they were drawn each frame
//! ([`mark`]); the tour, drawn last, reads them back. A part not shown in
//! the workspace leaves the card in the middle of the window, with nothing
//! lit.

use crate::layout::EditorKind;
use crate::theme::{self, Palette, RichTextExt};
use egui::{Color32, Id, Key, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, pos2, vec2};

/// A part of the window a step can point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The World and Terrain switch, top left.
    Sides,
    /// The top bar's Save and Export buttons.
    SaveExport,
    /// An area showing this editor (the first one drawn).
    Editor(EditorKind),
}

struct Step {
    targets: &'static [Target],
    title: &'static str,
    body: &'static str,
}

/// How many steps the tour has.
pub const STEP_COUNT: usize = 6;

/// The steps, in the language of the interface.
fn steps() -> [Step; STEP_COUNT] {
    let mac = cfg!(target_os = "macos");
    [
        Step {
            targets: &[Target::Sides],
            title: tr!("Welcome to Massif"),
            body: tr!("Massif has two sides. World is the map itself: you sculpt it, paint it and export it. Terrain grows relief from a graph of nodes, then sends it to the map."),
        },
        Step {
            targets: &[Target::Editor(EditorKind::Tools), Target::Editor(EditorKind::ToolSettings)],
            title: tr!("Tools"),
            body: tr!("Raise, smooth, flatten, paint terrains, flood with water… Pick a tool here; its own settings show just below."),
        },
        Step {
            targets: &[Target::Editor(EditorKind::BrushSettings)],
            title: tr!("Brush Settings"),
            body: tr!("The size, strength and shape of the brush. While painting, the scroll wheel changes the size and 1 to 0 the strength."),
        },
        Step {
            targets: &[Target::Editor(EditorKind::Map)],
            title: tr!("The 2D map"),
            body: if mac {
                tr!("Paint with the left button, the inverse with the right. Pinch or ⌘-scroll to zoom; drag with the middle button or Space held to move around.")
            } else {
                tr!("Paint with the left button, the inverse with the right. Ctrl-scroll to zoom; drag with the middle button or Space held to move around.")
            },
        },
        Step {
            targets: &[Target::Editor(EditorKind::View3d)],
            title: tr!("The 3D view"),
            body: tr!("The world as it will stand in the game, redrawn as you paint. Drag to turn around it, scroll to come closer, double-click to look at a spot."),
        },
        Step {
            targets: &[Target::SaveExport],
            title: tr!("Save and export"),
            body: if mac {
                tr!("⌘S saves the map as a .massif file to come back to. Export (⌘E) writes it as a Minecraft world, ready to play.")
            } else {
                tr!("Ctrl+S saves the map as a .massif file to come back to. Export (Ctrl+E) writes it as a Minecraft world, ready to play.")
            },
        },
    ]
}

/// The width of the card.
const CARD_W: f32 = 320.0;
/// Room left around what is lit.
const HALO: f32 = 6.0;
/// Between what is lit and the card.
const GAP: f32 = 14.0;
/// Seconds the light takes to move from one part to the next.
const MOVE_SECS: f32 = 0.28;

/// The tour, while it is shown.
pub struct Tour {
    pub step: usize,
    /// "Don't show again": ticked, the tour does not come back on its own
    /// once it is closed.
    pub dont_show: bool,
    /// The card's size last frame, to place it before it is laid out.
    card: egui::Vec2,
}

impl Default for Tour {
    fn default() -> Self {
        Self {
            step: 0,
            dont_show: true,
            card: vec2(CARD_W, 168.0),
        }
    }
}

/// How the tour was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    /// Skip or Escape, before the end.
    Skipped,
    /// Done, at the last step.
    Finished,
}

/// The parts of the window marked this frame.
#[derive(Clone, Default)]
struct Marks {
    frame: u64,
    rects: Vec<(Target, Rect)>,
}

fn marks_id() -> Id {
    Id::unique("massif tour marks")
}

/// Say where a part of the window was drawn this frame. The first area of
/// an editor counts; marks of the same target from earlier frames are
/// forgotten.
pub fn mark(ctx: &egui::Context, target: Target, rect: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let marks = d.get_temp_mut_or_default::<Marks>(marks_id());
        if marks.frame != frame {
            marks.frame = frame;
            marks.rects.clear();
        }
        match marks.rects.iter_mut().find(|(t, _)| *t == target) {
            // The two halves of one target (Save and Export) make one.
            Some((Target::SaveExport, r)) => *r = r.union(rect),
            Some(_) => {}
            None => marks.rects.push((target, rect)),
        }
    });
}

/// The rectangle around the targets marked this frame, if any was.
fn marked(ctx: &egui::Context, targets: &[Target]) -> Option<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| {
        let marks = d.get_temp::<Marks>(marks_id())?;
        if marks.frame != frame {
            return None;
        }
        targets
            .iter()
            .filter_map(|t| marks.rects.iter().find(|(m, _)| m == t).map(|(_, r)| *r))
            .reduce(|a, b| a.union(b))
    })
}

impl Tour {
    /// The title of the step shown (for tests).
    #[cfg(test)]
    pub fn title(&self) -> &'static str {
        let [a, b, c, d, e, f] = steps();
        [a, b, c, d, e, f][self.step].title
    }

    /// Draw the tour over the window: the window dimmed but for what the
    /// step talks about, and the card. Returns how it was closed, if it
    /// was this frame.
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Closed> {
        let mut closed = None;
        let screen = ctx.content_rect();
        let still = theme::reduce_motion(ctx);
        let steps = steps();
        let step = &steps[self.step.min(STEP_COUNT - 1)];
        let target = marked(ctx, step.targets).map(|r| r.expand(HALO).intersect(screen.shrink(2.0)));

        // The light glides from one part to the next (not with less motion).
        let hole = target.map(|r| {
            if still {
                return r;
            }
            let glide = |k: &str, v: f32| ctx.animate_value_with_time(Id::unique(("massif tour hole", k)), v, MOVE_SECS);
            Rect::from_min_max(pos2(glide("x0", r.min.x), glide("y0", r.min.y)), pos2(glide("x1", r.max.x), glide("y1", r.max.y)))
        });
        let shown = if still { 1.0 } else { ctx.animate_bool_with_time(Id::unique("massif tour shown"), true, 0.2) };

        egui::Area::new(Id::unique("massif tour"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);
                let pal = Palette::of(ui);
                // Nothing under the tour takes clicks while it is shown.
                ui.allocate_rect(screen, Sense::click_and_drag());
                let dim = Color32::from_black_alpha((if pal.dark { 150.0 } else { 105.0 } * shown) as u8);
                match hole {
                    Some(hole) => {
                        paint_dim(ui.painter(), screen, hole, theme::CARD_RADIUS as f32 + HALO * 0.5, dim);
                        let ring = theme::mix(pal.accent, Color32::WHITE, if pal.dark { 0.15 } else { 0.0 });
                        ui.painter().rect_stroke(hole, theme::CARD_RADIUS as f32 + HALO * 0.5, Stroke::new(1.5, ring.gamma_multiply(shown)), StrokeKind::Outside);
                    }
                    None => {
                        ui.painter().rect_filled(screen, 0, dim);
                    }
                }

                let at = place_card(screen, target, self.card);
                let mut card = ui.new_child(UiBuilder::new().id_salt("massif tour card").max_rect(Rect::from_min_size(at, vec2(CARD_W, screen.height()))));
                let frame = egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(theme::POPUP_RADIUS)
                    .shadow(ui.style().visuals.popup_shadow)
                    .inner_margin(egui::Margin::same(18));
                let response = frame.show(&mut card, |ui| {
                    ui.set_width(CARD_W - 36.0);
                    if let Some(c) = self.card_ui(ui, &pal) {
                        closed = Some(c);
                    }
                });
                self.card = response.response.rect.size();
            });

        if closed.is_none() && !ctx.egui_wants_keyboard_input() {
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    closed = Some(Closed::Skipped);
                } else if i.consume_key(egui::Modifiers::NONE, Key::ArrowRight) || i.consume_key(egui::Modifiers::NONE, Key::Enter) {
                    closed = self.next();
                } else if i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                    self.step = self.step.saturating_sub(1);
                }
            });
        }
        if !still && (shown < 1.0 || hole.zip(target).is_some_and(|(h, t)| h != t)) {
            ctx.request_repaint();
        }
        closed
    }

    /// The next step, or the end after the last one.
    fn next(&mut self) -> Option<Closed> {
        if self.step + 1 >= STEP_COUNT {
            Some(Closed::Finished)
        } else {
            self.step += 1;
            None
        }
    }

    fn card_ui(&mut self, ui: &mut egui::Ui, pal: &Palette) -> Option<Closed> {
        let mut closed = None;
        let steps = steps();
        let step = &steps[self.step.min(STEP_COUNT - 1)];
        // Where the tour is, and the way out.
        ui.horizontal(|ui| {
            progress(ui, pal, self.step);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let skip = egui::Button::new(RichText::new(tr!("Skip tour")).size(12.0).color(pal.weak)).frame(false);
                if ui.add(skip).on_hover_text("Esc").clicked() {
                    closed = Some(Closed::Skipped);
                }
            });
        });
        ui.add_space(10.0);
        ui.label(RichText::new(step.title).size(16.0).medium().color(pal.text));
        ui.add_space(4.0);
        ui.label(RichText::new(step.body).size(12.5).color(theme::mix(pal.text, pal.weak, 0.45)));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            crate::widgets::checkbox(ui, &mut self.dont_show, RichText::new(tr!("Don't show again")).size(12.0).color(pal.weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let last = self.step + 1 == STEP_COUNT;
                if ui.add(theme::primary_button(ui, if last { tr!("Done") } else { tr!("Next") })).clicked() {
                    closed = self.next();
                }
                if self.step > 0 && ui.add(egui::Button::new(tr!("Back")).corner_radius(theme::PILL).min_size(vec2(0.0, 28.0))).clicked() {
                    self.step -= 1;
                }
            });
        });
        closed
    }
}

/// Where the tour is: a dot a step, the one shown long, in the accent.
fn progress(ui: &mut egui::Ui, pal: &Palette, step: usize) {
    let (w, h, gap) = (6.0, 6.0, 5.0);
    let long = 16.0;
    let width = w * (STEP_COUNT - 1) as f32 + long + gap * (STEP_COUNT - 1) as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 14.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tr!("Step {n} of {count}", n = step + 1, count = STEP_COUNT)));
    let mut x = rect.left();
    for k in 0..STEP_COUNT {
        let len = if k == step { long } else { w };
        let dot = Rect::from_min_size(pos2(x, rect.center().y - h / 2.0), vec2(len, h));
        let color = if k == step {
            pal.accent
        } else if k < step {
            theme::mix(pal.weak, pal.border, 0.2)
        } else {
            pal.border
        };
        ui.painter().rect_filled(dot, theme::PILL, color);
        x += len + gap;
    }
}

/// Where the card goes: beside what is lit, on the side with the most
/// room (right, left, below, above), kept in the window; in the middle when
/// nothing is lit.
fn place_card(screen: Rect, target: Option<Rect>, size: egui::Vec2) -> Pos2 {
    let inner = screen.shrink(12.0);
    let clamp = |p: Pos2| {
        pos2(
            p.x.clamp(inner.left(), (inner.right() - size.x).max(inner.left())),
            p.y.clamp(inner.top(), (inner.bottom() - size.y).max(inner.top())),
        )
    };
    let Some(t) = target else {
        return clamp(screen.center() - size / 2.0);
    };
    let room = [
        (inner.right() - t.right() - GAP, size.x),
        (t.left() - inner.left() - GAP, size.x),
        (inner.bottom() - t.bottom() - GAP, size.y),
        (t.top() - inner.top() - GAP, size.y),
    ];
    let at = [
        pos2(t.right() + GAP, t.top()),
        pos2(t.left() - GAP - size.x, t.top()),
        pos2(t.left(), t.bottom() + GAP),
        pos2(t.left(), t.top() - GAP - size.y),
    ];
    // The first side it fits on, in that order; else in the middle of the
    // target, leaving its edges (a header, a bar) in sight.
    match (0..4).find(|&k| room[k].0 >= room[k].1) {
        Some(k) => clamp(at[k]),
        None => clamp(t.center() - size / 2.0),
    }
}

/// The window dimmed all around `hole`, a rounded rectangle left clear:
/// four bands around the hole's rectangle, and its corners outside their
/// arcs filled.
fn paint_dim(painter: &egui::Painter, screen: Rect, hole: Rect, radius: f32, color: Color32) {
    let r = radius.min(hole.width() / 2.0).min(hole.height() / 2.0).max(0.0);
    let bands = [
        Rect::from_min_max(screen.min, pos2(screen.right(), hole.top())),
        Rect::from_min_max(pos2(screen.left(), hole.bottom()), screen.max),
        Rect::from_min_max(pos2(screen.left(), hole.top()), pos2(hole.left(), hole.bottom())),
        Rect::from_min_max(pos2(hole.right(), hole.top()), pos2(screen.right(), hole.bottom())),
    ];
    for band in bands.into_iter().filter(|b| b.is_positive()) {
        painter.rect_filled(band, 0, color);
    }
    // Each corner: from its square point, a fan to the arc inside it.
    const SEGMENTS: usize = 10;
    let mut mesh = egui::Mesh::default();
    let corners = [
        (hole.left_top(), pos2(hole.left() + r, hole.top() + r), 1.0f32),
        (hole.right_top(), pos2(hole.right() - r, hole.top() + r), 1.5),
        (hole.right_bottom(), pos2(hole.right() - r, hole.bottom() - r), 0.0),
        (hole.left_bottom(), pos2(hole.left() + r, hole.bottom() - r), 0.5),
    ];
    for (square, centre, start) in corners {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(square, color);
        for s in 0..=SEGMENTS {
            let a = (start + 0.5 * s as f32 / SEGMENTS as f32) * std::f32::consts::PI;
            mesh.colored_vertex(centre + vec2(a.cos(), a.sin()) * r, color);
        }
        for s in 0..SEGMENTS as u32 {
            mesh.add_triangle(base, base + 1 + s, base + 2 + s);
        }
    }
    painter.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_goes_where_there_is_room() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
        let size = vec2(CARD_W, 170.0);
        // A narrow panel on the left: the card on its right.
        let at = place_card(screen, Some(Rect::from_min_size(pos2(10.0, 80.0), vec2(200.0, 300.0))), size);
        assert_eq!(at.x, 210.0 + GAP);
        // A panel on the right: on its left.
        let at = place_card(screen, Some(Rect::from_min_size(pos2(1000.0, 80.0), vec2(270.0, 300.0))), size);
        assert_eq!(at.x, 1000.0 - GAP - CARD_W);
        // Nothing lit: in the middle.
        let at = place_card(screen, None, size);
        assert_eq!(at, screen.center() - size / 2.0);
        // The whole window lit: inside it, kept in the window.
        let at = place_card(screen, Some(screen), size);
        assert!(screen.contains_rect(Rect::from_min_size(at, size)));
    }
}
