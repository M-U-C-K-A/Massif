//! Controls of our own, where egui's look like a toolkit's.
//!
//! [`Field`] is a number as current interfaces show one: a slim track
//! filled up to its value with a thumb on it, and the value beside it —
//! pressed anywhere on the track, the thumb goes there (Shift: finely);
//! the value is dragged sideways, or clicked and typed in. [`checkbox`],
//! [`well`] and [`row`] draw the rest of a group of settings.

use crate::theme::{self, Icon, Palette};
use egui::emath::Numeric;
use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, Response, Sense, SliderClamping, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Widget, pos2, vec2};
use std::ops::RangeInclusive;

type Formatter<'a> = Box<dyn 'a + Fn(f64, RangeInclusive<usize>) -> String>;
type Parser<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;

/// A number shown as a slim slider with its value, to slide or to type.
pub struct Field<'a> {
    get_set: Box<dyn 'a + FnMut(Option<f64>) -> f64>,
    range: RangeInclusive<f64>,
    integer: bool,
    suffix: String,
    text: Option<String>,
    max_decimals: Option<usize>,
    logarithmic: bool,
    width: Option<f32>,
    clamping: SliderClamping,
    formatter: Option<Formatter<'a>>,
    parser: Option<Parser<'a>>,
}

impl<'a> Field<'a> {
    pub fn new<N: Numeric>(value: &'a mut N, range: RangeInclusive<N>) -> Self {
        Self {
            get_set: Box::new(move |v: Option<f64>| {
                if let Some(v) = v {
                    *value = N::from_f64(v);
                }
                value.to_f64()
            }),
            range: range.start().to_f64()..=range.end().to_f64(),
            integer: N::INTEGRAL,
            suffix: String::new(),
            text: None,
            max_decimals: None,
            logarithmic: false,
            width: None,
            clamping: SliderClamping::Always,
            formatter: None,
            parser: None,
        }
    }

    /// Written after the value: units.
    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }

    /// A name written in the bar, the value after it.
    pub fn text(mut self, text: impl ToString) -> Self {
        self.text = Some(text.to_string());
        self
    }

    pub fn max_decimals(mut self, n: usize) -> Self {
        self.max_decimals = Some(n);
        self
    }

    /// Spread the range evenly in ratios rather than in steps.
    pub fn logarithmic(mut self, on: bool) -> Self {
        self.logarithmic = on;
        self
    }

    /// How wide it is, rather than what the row leaves it.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Whether a value typed in may lie past the range (dragging stays in it).
    pub fn clamping(mut self, clamping: SliderClamping) -> Self {
        self.clamping = clamping;
        self
    }

    pub fn custom_formatter(mut self, f: impl 'a + Fn(f64, RangeInclusive<usize>) -> String) -> Self {
        self.formatter = Some(Box::new(f));
        self
    }

    pub fn custom_parser(mut self, f: impl 'a + Fn(&str) -> Option<f64>) -> Self {
        self.parser = Some(Box::new(f));
        self
    }

    fn get(&mut self) -> f64 {
        (self.get_set)(None)
    }

    fn set(&mut self, v: f64) {
        (self.get_set)(Some(if self.integer { v.round() } else { v }));
    }

    /// Where `v` lies along the bar, 0 to 1.
    fn fraction(&self, v: f64) -> f64 {
        let (a, b) = (*self.range.start(), *self.range.end());
        if b <= a {
            return 0.0;
        }
        let t = if self.logarithmic && a > 0.0 { (v / a).ln() / (b / a).ln() } else { (v - a) / (b - a) };
        t.clamp(0.0, 1.0)
    }

    /// The value at `t` along the bar.
    fn at(&self, t: f64) -> f64 {
        let (a, b) = (*self.range.start(), *self.range.end());
        let t = t.clamp(0.0, 1.0);
        if self.logarithmic && a > 0.0 { a * (b / a).powf(t) } else { a + (b - a) * t }
    }

    fn format(&self, v: f64) -> String {
        if let Some(f) = &self.formatter {
            let d = self.max_decimals.unwrap_or(3);
            return f(v, 0..=d);
        }
        let decimals = if self.integer {
            0
        } else {
            let auto = if v.abs() >= 100.0 { 0 } else if v.abs() >= 10.0 { 1 } else { 2 };
            self.max_decimals.map_or(auto, |m| m.min(auto.max(m.min(2))))
        };
        let mut s = format!("{v:.decimals$}");
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        if s == "-0" {
            s = "0".into();
        }
        format!("{s}{}", self.suffix)
    }

    fn parse(&self, s: &str) -> Option<f64> {
        if let Some(p) = &self.parser {
            return p(s);
        }
        let s = s.trim().trim_end_matches(self.suffix.trim()).trim();
        s.parse::<f64>().ok()
    }
}

/// The number of a formatted value, and its unit: `133 blocks` → `133`,
/// ` blocks`.
fn split_unit(s: &str) -> (&str, &str) {
    match s.char_indices().rev().find(|(_, c)| c.is_ascii_digit()) {
        Some((i, c)) => s.split_at(i + c.len_utf8()),
        None => (s, ""),
    }
}

/// The radius of a slider's thumb.
const THUMB: f32 = 6.0;

impl Widget for Field<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let pal = Palette::of(ui);
        let height = ui.spacing().interact_size.y.clamp(20.0, 26.0);
        let width = self.width.unwrap_or_else(|| ui.available_width().min(ui.spacing().slider_width + 70.0)).max(44.0);
        let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let id = response.id;
        let value = self.get();
        let font = FontId::proportional(12.0);

        // ---- layout: [name] track value --------------------------------------
        // The value is as wide as its widest, so that the track does not
        // move as it changes. A field too narrow for a track is its value
        // alone, in a pill, dragged sideways.
        let measure = |s: &str| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), pal.text).size().x;
        let widest = [self.format(*self.range.start()), self.format(*self.range.end()), self.format(value)].iter().map(|s| measure(s)).fold(0.0f32, f32::max);
        let named = self.text.is_some();
        let inner = if named { rect.shrink2(vec2(10.0, 0.0)) } else { rect };
        let name_w = self.text.as_ref().map_or(0.0, |n| measure(n) + 10.0);
        // A field wide enough keeps a column for its value, so that the
        // tracks of a group of them end together.
        let column = if inner.width() - name_w >= 120.0 { 58.0 } else { 0.0 };
        let value_w = (widest + 2.0).max(column).min(inner.width());
        let value_rect = Rect::from_min_max(pos2(inner.right() - value_w, rect.top()), pos2(inner.right(), rect.bottom()));
        let (x0, x1) = (inner.left() + name_w + THUMB + 1.0, value_rect.left() - 10.0 - THUMB);
        let has_track = x1 - x0 >= 24.0;
        let on_value = |p: egui::Pos2| !has_track || p.x >= value_rect.left() - 6.0;

        // ---- typed in --------------------------------------------------------
        let editing: Option<String> = ui.data(|d| d.get_temp(id));
        if let Some(mut buffer) = editing {
            if has_track {
                self.paint_track(ui, &pal, rect, named, inner, x0, x1, value, false, false);
            }
            let edit_rect = if has_track { Rect::from_min_max(pos2(value_rect.right() - value_w.max(52.0) - 8.0, rect.top() + 1.0), pos2(rect.right(), rect.bottom() - 1.0)) } else { rect };
            let edit = ui.put(
                edit_rect,
                TextEdit::singleline(&mut buffer)
                    .font(FontId::proportional(12.0))
                    .align(Align2::CENTER_CENTER)
                    .margin(vec2(6.0, 2.0)),
            );
            let first = ui.data(|d| d.get_temp::<bool>(id.with("focus")).is_none());
            if first {
                edit.request_focus();
                ui.data_mut(|d| d.insert_temp(id.with("focus"), true));
            }
            let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if edit.lost_focus() || enter || escape {
                ui.data_mut(|d| {
                    d.remove::<String>(id);
                    d.remove::<bool>(id.with("focus"));
                });
                if !escape && let Some(v) = self.parse(&buffer) {
                    let v = match self.clamping {
                        SliderClamping::Never => v,
                        _ => v.clamp(*self.range.start(), *self.range.end()),
                    };
                    if v != value {
                        self.set(v);
                        response.mark_changed();
                    }
                }
            } else {
                ui.data_mut(|d| d.insert_temp(id, buffer));
            }
            return response;
        }

        // ---- the mouse ---------------------------------------------------------
        // On the track: the thumb goes where the pointer is, at once (Shift:
        // finely, from where it is). On the value: dragged sideways it
        // changes; clicked, it is typed in.
        let (origin, pointer, fine, primary) = ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos(), i.modifiers.shift, i.pointer.primary_down()));
        let grabbed = response.is_pointer_button_down_on() && primary;
        let from_value = origin.is_some_and(on_value);
        let mut sliding = false;
        if grabbed && !from_value && let Some(p) = pointer {
            sliding = true;
            let last: f64 = ui.data(|d| d.get_temp(id.with("drag"))).unwrap_or_else(|| self.fraction(value));
            let span = (x1 - x0).max(1.0) as f64;
            let t = if fine { last + ui.input(|i| i.pointer.delta().x) as f64 * 0.1 / span } else { (p.x - x0) as f64 / span }.clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(id.with("drag"), t));
            let v = self.at(t);
            if v != value {
                self.set(v);
                response.mark_changed();
            }
        } else if response.dragged() && from_value && primary {
            let dx = response.drag_delta().x as f64 * if fine { 0.1 } else { 1.0 };
            let start: f64 = if response.drag_started() { self.fraction(value) } else { ui.data(|d| d.get_temp(id.with("drag"))).unwrap_or_else(|| self.fraction(value)) };
            let t = (start + dx / 180.0).clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(id.with("drag"), t));
            let v = self.at(t);
            if v != value {
                self.set(v);
                response.mark_changed();
            }
        } else if response.clicked() && response.interact_pointer_pos().is_some_and(on_value) {
            let shown = self.format(value);
            let plain = shown.trim_end_matches(self.suffix.as_str()).trim().to_owned();
            ui.data_mut(|d| d.insert_temp(id, plain));
            ui.ctx().request_repaint();
        }
        if !grabbed && !response.dragged() {
            ui.data_mut(|d| d.remove::<f64>(id.with("drag")));
        }
        let over_value = response.hover_pos().is_some_and(on_value);
        if (response.hovered() && over_value) || (response.dragged() && from_value) {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value, self.text.as_deref().unwrap_or("")));

        // ---- drawing -----------------------------------------------------------
        if ui.is_rect_visible(rect) {
            let value = self.get();
            let painter = ui.painter();
            let hot_value = over_value && response.hovered() || response.dragged() && from_value;
            if has_track {
                self.paint_track(ui, &pal, rect, named, inner, x0, x1, value, response.hovered() && !over_value, sliding);
                if hot_value {
                    painter.rect_filled(value_rect.expand2(vec2(5.0, -3.0)), theme::PILL, theme::mix(pal.panel, pal.text, 0.09));
                }
            } else {
                let lit = response.hovered() || response.dragged();
                let fill = if lit { theme::mix(pal.control(), pal.text, 0.07) } else { pal.control() };
                painter.rect(rect, theme::PILL, fill, Stroke::new(1.0, pal.control_border()), StrokeKind::Inside);
                if let Some(name) = &self.text {
                    painter.text(inner.left_center(), Align2::LEFT_CENTER, name, font.clone(), pal.weak);
                }
            }
            // The number, and its unit after it, dimmer.
            let shown = self.format(value);
            let (number, unit) = split_unit(&shown);
            let unit = painter.layout_no_wrap(unit.to_owned(), font.clone(), pal.weak);
            let number = painter.layout_no_wrap(number.to_owned(), font, pal.text);
            let total = number.size().x + unit.size().x;
            // Against the right edge where the field is laid from the right
            // (the rows of a well) or holds its name; else just after the
            // track, where values line up under one another.
            let from_right = named || ui.layout().main_dir() == egui::Direction::RightToLeft;
            let left = if !has_track && !named {
                rect.center().x - total / 2.0
            } else if from_right || !has_track {
                value_rect.right() - total
            } else {
                value_rect.left()
            };
            let y = |g: &egui::Galley| rect.center().y - g.size().y / 2.0;
            painter.galley(pos2(left + number.size().x, y(&unit)), unit.clone(), pal.weak);
            painter.galley(pos2(left, y(&number)), number, pal.text);
        }
        response
    }
}

impl Field<'_> {
    /// The slim track of a slider, filled up to its value, and its thumb;
    /// in a pill of its own, with its name, for a named field.
    #[allow(clippy::too_many_arguments)]
    fn paint_track(&self, ui: &Ui, pal: &Palette, rect: Rect, named: bool, inner: Rect, x0: f32, x1: f32, value: f64, hovered: bool, sliding: bool) {
        let painter = ui.painter();
        if named {
            painter.rect(rect, theme::PILL, pal.control(), Stroke::new(1.0, pal.control_border()), StrokeKind::Inside);
            if let Some(name) = &self.text {
                painter.text(inner.left_center(), Align2::LEFT_CENTER, name, FontId::proportional(12.0), pal.weak);
            }
        }
        let cy = rect.center().y;
        let t = self.fraction(value) as f32;
        let x = x0 + (x1 - x0) * t;
        let rail = theme::mix(if named { pal.control() } else { pal.panel }, pal.text, if pal.dark { 0.17 } else { 0.14 });
        let track = Rect::from_min_max(pos2(x0 - THUMB, cy - 2.0), pos2(x1 + THUMB, cy + 2.0));
        painter.rect_filled(track, 2, rail);
        if t > 0.0 {
            painter.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), 2, pal.accent);
        }
        let c = pos2(x, cy);
        let r = if sliding { THUMB + 1.0 } else { THUMB };
        if hovered || sliding {
            painter.circle_filled(c, r + 4.5, pal.accent.gamma_multiply(if sliding { 0.30 } else { 0.18 }));
        }
        painter.circle_filled(c + vec2(0.0, 1.0), r + 0.5, Color32::from_black_alpha(if pal.dark { 90 } else { 40 }));
        let (fill, edge) = if pal.dark { (Color32::from_rgb(244, 244, 246), Color32::from_black_alpha(60)) } else { (Color32::WHITE, Color32::from_black_alpha(55)) };
        painter.circle(c, r, fill, Stroke::new(1.0, edge));
    }
}

/// A checkbox as current interfaces draw them: a rounded square, filled
/// with the accent and ticked when on.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: impl Into<egui::WidgetText>) -> Response {
    const BOX: f32 = 16.0;
    let pal = Palette::of(ui);
    let text: egui::WidgetText = text.into();
    let gap = if text.is_empty() { 0.0 } else { 8.0 };
    let galley = (!text.is_empty()).then(|| text.into_galley(ui, None, ui.available_width() - BOX - gap, egui::TextStyle::Button));
    let text_size = galley.as_ref().map_or(egui::Vec2::ZERO, |g| g.size());
    let size = vec2(BOX + gap + text_size.x, text_size.y.max(BOX).max(ui.spacing().interact_size.y.min(22.0)));
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let label = galley.as_ref().map(|g| g.text().to_owned()).unwrap_or_default();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, &label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id.with("on"), *on, 0.1);
        let square = Rect::from_center_size(pos2(rect.left() + BOX / 2.0, rect.center().y), egui::Vec2::splat(BOX));
        let painter = ui.painter();
        let edge = theme::mix(pal.control_border(), pal.text, if response.hovered() { 0.4 } else { 0.2 });
        painter.rect(square, 5, if pal.dark { pal.well() } else { Color32::WHITE }, Stroke::new(1.2, edge), StrokeKind::Inside);
        if t > 0.0 {
            painter.rect_filled(square, 5, pal.accent.gamma_multiply(t));
            let at = |x: f32, y: f32| square.min + vec2(x, y) * BOX;
            painter.line(vec![at(0.27, 0.52), at(0.44, 0.68), at(0.74, 0.34)], Stroke::new(1.8, pal.on_accent().gamma_multiply(t)));
        }
        if let Some(galley) = galley {
            painter.galley(pos2(square.right() + gap, rect.center().y - galley.size().y / 2.0), galley, ui.visuals().text_color());
        }
    }
    response
}

/// The height of a row of a well.
const ROW_H: f32 = 32.0;

/// A well: settings that go together, a shade under the card, a hairline
/// around them. Fill it with [`row`]s.
pub fn well<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = Palette::of(ui);
    egui::Frame::new()
        .fill(pal.well())
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(theme::GROUP_RADIUS)
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// A row of a [`well`]: its name on the left, its control on the right, a
/// hairline over it from edge to edge unless it is the first.
pub fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    let first = ui.cursor().top() <= ui.max_rect().top() + 0.5;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    if !first {
        ui.painter().hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.label(egui::RichText::new(label).size(12.5));
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
}

/// A combo box with a chevron rather than egui's triangle.
pub fn combo(id: impl egui::AsIdSalt) -> egui::ComboBox<'static> {
    egui::ComboBox::from_id_salt(id).icon(chevron)
}

fn chevron(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _open: bool) {
    let c = rect.center();
    let s = rect.height().min(rect.width()) * 0.28;
    let stroke = Stroke::new(1.4, visuals.fg_stroke.color.gamma_multiply(0.75));
    ui.painter().line(vec![c + vec2(-s, -s * 0.45), c + vec2(0.0, s * 0.55), c + vec2(s, -s * 0.45)], stroke);
}

// --------------------------------------------------------------- groups --
//
// As shadcn's input and button groups: the parts of one setting joined in
// one outline, a hairline between them, what is not to be edited muted.

const GROUP_H: f32 = 30.0;
const GROUP_R: u8 = 15;

fn group_fill(pal: &Palette) -> Color32 {
    let c = pal.control();
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 235)
}

/// The rounding of part `k` of `n` joined parts.
fn corners(k: usize, n: usize) -> CornerRadius {
    let (first, last) = (k == 0, k + 1 == n);
    CornerRadius {
        nw: if first { GROUP_R } else { 0 },
        sw: if first { GROUP_R } else { 0 },
        ne: if last { GROUP_R } else { 0 },
        se: if last { GROUP_R } else { 0 },
    }
}

/// A child of `ui` over `rect` whose widgets draw no frame of their own.
fn bare(ui: &mut Ui, rect: Rect) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)));
    let v = child.visuals_mut();
    v.extreme_bg_color = Color32::TRANSPARENT;
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.bg_fill = Color32::TRANSPARENT;
        w.weak_bg_fill = Color32::TRANSPARENT;
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
    }
    child
}

/// One part of an input group.
pub enum Part<'a> {
    /// Muted words: a unit, a sign, a name.
    Text(&'a str),
    /// A whole number to type or drag: its range, how fast it drags.
    Int(&'a mut i32, RangeInclusive<i32>, f64),
    /// A line of text, this wide.
    Edit(&'a mut String, f32),
    /// A button showing an icon, with what it does.
    Button(Icon, &'a str),
}

/// Parts joined in one outline, as one control. Returns whether a value
/// changed, and the index of a button clicked.
pub fn input_group(ui: &mut Ui, id: &str, parts: &mut [Part]) -> (bool, Option<usize>) {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), pal.weak).size().x + 24.0,
            Part::Int(..) => 66.0,
            Part::Edit(_, w) => *w,
            Part::Button(..) => 32.0,
        })
        .collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum(), GROUP_H), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, GROUP_R, group_fill(&pal));
    let n = parts.len();
    let (mut changed, mut clicked) = (false, None);
    let mut x = rect.min.x;
    for (k, (part, w)) in parts.iter_mut().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.min.y), vec2(*w, GROUP_H));
        match part {
            Part::Text(t) => {
                painter.rect_filled(seg, corners(k, n), theme::mix(pal.control(), pal.text, 0.045));
                painter.text(seg.center(), Align2::CENTER_CENTER, *t, font.clone(), pal.weak);
            }
            Part::Int(v, range, speed) => {
                let mut child = bare(ui, seg.shrink2(vec2(4.0, 3.0)));
                changed |= child.add(egui::DragValue::new(*v).range(range.clone()).speed(*speed)).changed();
            }
            Part::Edit(text, _) => {
                let mut child = bare(ui, seg.shrink2(vec2(2.0, 3.0)));
                changed |= child
                    .add(
                        TextEdit::singleline(*text)
                            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 0)))
                            .font(font.clone())
                            .align(Align2::LEFT_CENTER),
                    )
                    .changed();
            }
            Part::Button(icon, tip) => {
                let r = ui.interact(seg, egui::Id::unique((id, k)), Sense::click()).on_hover_text(*tip);
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *tip));
                if r.hovered() {
                    painter.rect_filled(seg, corners(k, n), theme::mix(pal.control(), pal.text, 0.08));
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                painter.text(seg.center(), Align2::CENTER_CENTER, theme::glyph(*icon).to_string(), FontId::proportional(13.5), pal.text);
                if r.clicked() {
                    clicked = Some(k);
                }
            }
        }
        if k > 0 {
            painter.vline(x, seg.y_range(), Stroke::new(1.0, pal.border));
        }
        x += w;
    }
    painter.rect_stroke(rect, GROUP_R, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    (changed, clicked)
}

/// A segmented control: choices side by side in a pill, those `on` raised
/// in a pill of their own. Returns the one clicked.
pub fn button_group(ui: &mut Ui, id: &str, labels: &[&str], on: &[bool]) -> Option<usize> {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let galleys: Vec<_> = labels.iter().map(|t| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), pal.text)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 26.0).collect();
    const INSET: f32 = 3.0;
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 2.0 * INSET, GROUP_H), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(rect, GROUP_R, if pal.dark { pal.well() } else { pal.header }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.min.x + INSET;
    for (k, (galley, w)) in galleys.into_iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.min.y + INSET), vec2(*w, GROUP_H - 2.0 * INSET));
        let pressed = on.get(k).copied().unwrap_or(false);
        let r = ui.interact(seg, egui::Id::unique((id, k)), Sense::click());
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, pressed, labels[k]));
        if pressed {
            let lifted = if pal.dark { theme::mix(pal.control(), pal.text, 0.07) } else { Color32::WHITE };
            painter.rect(seg, GROUP_R, lifted, Stroke::new(1.0, if pal.dark { pal.control_border() } else { pal.border }), StrokeKind::Inside);
        } else if r.hovered() {
            painter.rect_filled(seg, GROUP_R, theme::mix(pal.well(), pal.text, 0.06));
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let ink = if pressed { pal.text } else { pal.weak };
        painter.galley_with_override_text_color(seg.center() - galley.size() / 2.0, galley, ink);
        if r.clicked() {
            clicked = Some(k);
        }
        x += w;
    }
    clicked
}

