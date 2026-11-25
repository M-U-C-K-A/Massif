//! Controls of our own, where egui's look like a toolkit's.
//!
//! [`Field`] is a number as current interfaces show one: a slim track
//! filled up to its value with a thumb on it, and the value beside it —
//! pressed anywhere on the track, the thumb goes there (Shift: finely);
//! the value is dragged sideways, or clicked and typed in. [`checkbox`],
//! [`well`] and [`row`] draw the rest of a group of settings.

use crate::theme::{self, Icon, Palette};
use egui::emath::Numeric;
use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, Response, Sense, SliderClamping, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, Widget, pos2, vec2};
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
        // The widest of values along the whole range (with all their
        // decimals), not only the one shown: the track then stays put while
        // the value is dragged, and fields of a range line up.
        let along = [0.0, 0.123_456, 0.555_555, 0.876_543, 1.0].map(|t| self.format(self.at(t)));
        let past = (!self.range.contains(&value)).then(|| self.format(value));
        let widest = along.iter().chain(past.iter()).map(|s| measure(s)).fold(0.0f32, f32::max);
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
                // The caret where the number was clicked, as in any text:
                // a digit typed goes there, nothing is replaced.
                let at: Option<egui::Pos2> = ui.data(|d| d.get_temp(id.with("click")));
                // Measured against the number as it was drawn when clicked.
                let galley = ui.painter().layout_no_wrap(buffer.clone(), FontId::proportional(12.0), pal.text);
                let drawn: Option<f32> = ui.data(|d| d.get_temp(id.with("number at")));
                let left = drawn.unwrap_or(edit_rect.center().x - galley.size().x / 2.0);
                let caret = match at {
                    Some(p) => galley.cursor_from_pos(vec2(p.x - left, galley.size().y / 2.0)),
                    None => galley.end(),
                };
                let mut state = TextEdit::load_state(ui.ctx(), edit.id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(caret)));
                state.store(ui.ctx(), edit.id);
            }
            let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if edit.lost_focus() || enter || escape {
                ui.data_mut(|d| {
                    d.remove::<String>(id);
                    d.remove::<bool>(id.with("focus"));
                    d.remove::<egui::Pos2>(id.with("click"));
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
        } else if response.clicked()
            && let Some(at) = response.interact_pointer_pos().filter(|p| on_value(*p))
        {
            let shown = self.format(value);
            let plain = shown.trim_end_matches(self.suffix.as_str()).trim().to_owned();
            ui.data_mut(|d| {
                d.insert_temp(id, plain);
                d.insert_temp(id.with("click"), at);
            });
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
            // Where the number starts, for a click that types into it.
            ui.data_mut(|d| d.insert_temp(id.with("number at"), left));
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
        crate::sounds::play(crate::sounds::Cue::Toggle);
    }
    let label = galley.as_ref().map(|g| g.text().to_owned()).unwrap_or_default();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, &label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id.with("on"), *on, if theme::reduce_motion(ui.ctx()) { 0.0 } else { 0.1 });
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

/// A switch: on or off, its knob sliding over, the accent when on.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let pal = Palette::of(ui);
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
        crate::sounds::play(crate::sounds::Cue::Toggle);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), *on, ""));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id.with("on"), *on, if theme::reduce_motion(ui.ctx()) { 0.0 } else { 0.12 });
        let off = if pal.dark { theme::mix(pal.well(), pal.text, 0.18) } else { theme::mix(Color32::WHITE, pal.text, 0.16) };
        let track = theme::mix(off, pal.accent, t);
        ui.painter().rect_filled(rect, theme::PILL, track);
        let r = rect.height() / 2.0 - 2.5;
        let x = egui::lerp(rect.left() + 2.5 + r..=rect.right() - 2.5 - r, t);
        let knob = pos2(x, rect.center().y);
        ui.painter().circle_filled(knob + vec2(0.0, 0.8), r, Color32::from_black_alpha(50));
        ui.painter().circle_filled(knob, r, Color32::WHITE);
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
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
    // The control first, from the right; the name in the room it leaves,
    // cut with an ellipsis (whole on hover) in a narrow panel.
    let mut control = ui.new_child(UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    add(&mut control);
    let used = control.min_rect();
    let right = if used.width() > 0.0 { used.left() - ui.spacing().item_spacing.x } else { rect.right() };
    let room = Rect::from_min_max(rect.left_top(), egui::pos2(right.max(rect.left()), rect.bottom()));
    let mut name = ui.new_child(UiBuilder::new().max_rect(room).layout(egui::Layout::left_to_right(egui::Align::Center)));
    name.add(egui::Label::new(egui::RichText::new(label).size(12.5)).truncate());
}

/// The name of a setting on the left of its control, in a column `size`
/// wide: at the start of it, as the names of a well's rows and the
/// headings over them (not centred in it, as `ui.add_sized` puts a label);
/// cut with an ellipsis, whole on hover, when longer than the column.
pub fn name_label(ui: &mut Ui, text: impl Into<egui::WidgetText>, size: Vec2) -> Response {
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(size);
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

/// A button whose words stay on one line: cut with an ellipsis (whole on
/// hover) where the panel is narrower than they are, rather than folded
/// onto two lines inside it.
pub fn button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Button::new(text).truncate())
}

/// A button opening a menu, its words on one line as [`button`]'s.
pub fn menu_button(ui: &mut Ui, text: impl Into<egui::WidgetText>, add: impl FnOnce(&mut Ui)) -> Response {
    egui::containers::menu::MenuButton::from_button(egui::Button::new(text).truncate()).ui(ui, add).0
}

/// A search field: a pill with a magnifier at its left, the text after it,
/// `hint` in it while empty, centred top to bottom. `width` at most (as
/// wide as the room with `f32::INFINITY`). The text is laid inside the
/// pill's straight part, so that, scrolled to its end as it is typed, its
/// start never shows under the rounded edge. The text field's response.
pub fn search(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    search_with(ui, text, hint, width, |edit| edit)
}

/// A [`search`] field whose text field `more` sets up further (the keys it
/// takes, say).
pub fn search_with<'t>(ui: &mut Ui, text: &'t mut String, hint: &str, width: f32, more: impl FnOnce(TextEdit<'t>) -> TextEdit<'t>) -> Response {
    let pal = Palette::of(ui);
    let height = ui.spacing().interact_size.y.max(28.0);
    let w = width.min(ui.available_width()).max(height * 2.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let back = ui.painter().add(egui::Shape::Noop);
    let icon_x = rect.left() + height * 0.5;
    // The text 9 points clear of the magnifier (7 points across from its
    // middle).
    let inner = Rect::from_min_max(egui::pos2(icon_x + 16.0, rect.top()), egui::pos2(rect.right() - height * 0.45, rect.bottom()));
    // Cut to its own room: scrolled, the text never runs over the magnifier
    // or the pill's ends.
    let mut room = ui.new_child(UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    room.set_clip_rect(inner.intersect(ui.clip_rect()));
    let edit = room.put(
        inner,
        more(
            TextEdit::singleline(text)
                .hint_text(hint)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(inner.size())
                .desired_width(inner.width()),
        ),
    );
    let visuals = ui.visuals();
    let stroke = if edit.has_focus() {
        visuals.selection.stroke
    } else if edit.hovered() {
        visuals.widgets.hovered.bg_stroke
    } else {
        visuals.widgets.inactive.bg_stroke
    };
    let fill = visuals.text_edit_bg_color();
    ui.painter().set(back, egui::Shape::Rect(egui::epaint::RectShape::new(rect, theme::PILL, fill, stroke, StrokeKind::Inside)));
    ui.painter().text(egui::pos2(icon_x, rect.center().y), egui::Align2::CENTER_CENTER, theme::glyph(theme::Icon::Search), FontId::proportional(14.0), pal.weak);
    edit
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
    /// The same, made one of the values it may be once typed in: the
    /// nearest (`snap`), and why, shown a moment under it when it changed.
    Snapped(&'a mut i32, RangeInclusive<i32>, f64, fn(i32) -> i32, &'a str),
    /// A line of text, this wide.
    Edit(&'a mut String, f32),
    /// A button showing an icon, with what it does.
    Button(Icon, &'a str),
    /// A chain between two values: linked, they stay equal.
    Link(&'a mut bool),
}

/// `ui.add(drag)`, but a number clicked to be typed in gets the caret at
/// its end instead of being selected whole: a digit typed adds to it rather
/// than replacing it (a double-click, or ⌘A, still selects it all). What is
/// typed counts once it is done (Enter, or the number left): typing 1024
/// does not stop at 1, below the least the number may be, and become that
/// least before the rest is typed.
pub fn drag(ui: &mut Ui, drag: egui::DragValue<'_>) -> Response {
    let response = ui.add(drag.update_while_editing(false));
    let id = response.id;
    let key = id.with("caret since");
    if ui.memory(|m| m.has_focus(id)) {
        let frame = ui.ctx().cumulative_frame_nr();
        let since = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || frame));
        // egui selects everything over the first frames of the focus.
        if frame <= since + 2
            && let Some(mut state) = TextEdit::load_state(ui.ctx(), id)
            && let Some(range) = state.cursor.char_range()
            && range.primary != range.secondary
        {
            let end = if range.primary.index >= range.secondary.index { range.primary } else { range.secondary };
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), id);
            ui.ctx().request_repaint();
        }
    } else {
        ui.data_mut(|d| d.remove::<u64>(key));
    }
    response
}

/// [`drag`], greyed out unless `enabled`.
pub fn drag_enabled(ui: &mut Ui, enabled: bool, d: egui::DragValue<'_>) -> Response {
    ui.add_enabled_ui(enabled, |ui| drag(ui, d)).inner
}

/// The nearest value a number may be, and why.
type Snap<'a> = (fn(i32) -> i32, &'a str);

/// How long what a number became is shown under it.
const NOTE_SECS: f64 = 2.6;

/// A whole number typed as written: what is typed, a key at a time, stays
/// as it is typed (no completion, no bound, no rounding) until it is done
/// — Enter, Tab, or a click elsewhere; Escape drops it. Only then is it
/// read, kept in `range`, and made the nearest value `snap` allows; when
/// that is not what was typed, a note under it says what it became, a
/// moment. Dragged sideways, it changes by `speed` a point. Returns
/// whether the value changed.
fn number(ui: &mut Ui, rect: Rect, id: egui::Id, value: &mut i32, range: RangeInclusive<i32>, speed: f64, snap: Option<Snap>) -> bool {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let edit_id = id.with("edit");
    let mut changed = false;
    let fit = |v: i64| -> i32 {
        let v = v.clamp(*range.start() as i64, *range.end() as i64) as i32;
        snap.map_or(v, |(s, _)| s(v).clamp(*range.start(), *range.end()))
    };
    let editing: Option<String> = ui.data(|d| d.get_temp(id));
    if let Some(mut buffer) = editing {
        let mut child = bare(ui, rect);
        let edit = child.add(TextEdit::singleline(&mut buffer).id(edit_id).frame(egui::Frame::NONE).font(font.clone()).align(Align2::CENTER_CENTER).desired_width(rect.width()));
        if ui.data(|d| d.get_temp::<bool>(id.with("focus")).is_none()) {
            edit.request_focus();
            ui.data_mut(|d| d.insert_temp(id.with("focus"), true));
            // The caret where the number was clicked; all of it chosen
            // when come to with Tab, so that what is typed replaces it.
            let at: Option<egui::Pos2> = ui.data(|d| d.get_temp(id.with("click")));
            let galley = ui.painter().layout_no_wrap(buffer.clone(), font.clone(), pal.text);
            let range = match at {
                Some(p) => egui::text::CCursorRange::one(galley.cursor_from_pos(vec2(p.x - (rect.center().x - galley.size().x / 2.0), galley.size().y / 2.0))),
                None => egui::text::CCursorRange::two(galley.begin(), galley.end()),
            };
            let mut state = TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), edit_id);
        }
        let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if edit.lost_focus() || enter || escape {
            // The key ends the typing, and does nothing else (Enter does
            // not also make the map).
            ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
            ui.data_mut(|d| {
                d.remove::<String>(id);
                d.remove::<bool>(id.with("focus"));
                d.remove::<egui::Pos2>(id.with("click"));
            });
            if !escape && let Some(typed) = parse_whole(&buffer) {
                let v = fit(typed);
                if v as i64 != typed {
                    let why = if typed > *range.end() as i64 && v == *range.end() {
                        tr!("the most it may be").to_owned()
                    } else if typed < *range.start() as i64 && v == *range.start() {
                        tr!("the least it may be").to_owned()
                    } else {
                        snap.map_or(String::new(), |(_, why)| why.to_owned())
                    };
                    let now = ui.input(|i| i.time);
                    ui.data_mut(|d| d.insert_temp(id.with("note"), (now, format!("{typed} → {v}  ·  {why}"))));
                }
                if v != *value {
                    *value = v;
                    changed = true;
                }
            }
        } else {
            ui.data_mut(|d| d.insert_temp(id, buffer));
        }
    } else {
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if response.drag_started() {
            ui.data_mut(|d| d.insert_temp(id.with("drag"), *value as f64));
        }
        if response.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let raw: f64 = ui.data(|d| d.get_temp(id.with("drag"))).unwrap_or(*value as f64) + response.drag_delta().x as f64 * speed * if fine { 0.1 } else { 1.0 };
            let raw = raw.clamp(*range.start() as f64, *range.end() as f64);
            ui.data_mut(|d| d.insert_temp(id.with("drag"), raw));
            let v = fit(raw.round() as i64);
            if v != *value {
                *value = v;
                changed = true;
            }
        } else if response.clicked() || (response.has_focus() && !response.dragged()) {
            // Clicked, or come to with Tab: typed in.
            let at = response.interact_pointer_pos().filter(|_| response.clicked());
            ui.data_mut(|d| {
                d.insert_temp(id, value.to_string());
                if let Some(p) = at {
                    d.insert_temp(id.with("click"), p);
                }
            });
            ui.ctx().request_repaint();
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        response.widget_info(|| egui::WidgetInfo::drag_value(ui.is_enabled(), *value as f64));
        if ui.is_rect_visible(rect) {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, value.to_string(), font, if ui.is_enabled() { pal.text } else { pal.weak });
        }
    }
    // What a number typed became, a moment, under it.
    if let Some((at, note)) = ui.data(|d| d.get_temp::<(f64, String)>(id.with("note"))) {
        let age = ui.input(|i| i.time) - at;
        if age >= NOTE_SECS {
            ui.data_mut(|d| d.remove::<(f64, String)>(id.with("note")));
        } else {
            let still = theme::reduce_motion(ui.ctx());
            let fade = if still { 1.0 } else { ((NOTE_SECS - age) / 0.4).min(1.0) as f32 };
            let layer = egui::LayerId::new(egui::Order::Tooltip, id.with("note layer"));
            let painter = ui.ctx().layer_painter(layer);
            let galley = painter.layout_no_wrap(note, FontId::proportional(11.5), pal.text);
            let pill = Rect::from_min_size(pos2(rect.left() - 4.0, rect.bottom() + 8.0), galley.size() + vec2(18.0, 10.0));
            let back = theme::mix(pal.panel.to_opaque(), pal.text, if pal.dark { 0.08 } else { 0.0 });
            painter.rect_filled(pill.translate(vec2(0.0, 2.0)).expand(1.0), theme::PILL, Color32::from_black_alpha(if pal.dark { 90 } else { 28 }).gamma_multiply(fade));
            painter.rect(pill, theme::PILL, back.gamma_multiply(fade), Stroke::new(1.0, pal.control_border().gamma_multiply(fade)), StrokeKind::Inside);
            painter.galley(pill.min + vec2(9.0, 5.0), galley, pal.text.gamma_multiply(fade));
            if still {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(NOTE_SECS - age));
            } else if age < NOTE_SECS - 0.4 {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(NOTE_SECS - 0.4 - age));
            } else {
                ui.ctx().request_repaint();
            }
        }
    }
    changed
}

/// A whole number as people write it: `1024`, `1 024`, `1,024`, `-64`, or
/// `1024.4` (rounded).
fn parse_whole(s: &str) -> Option<i64> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && !matches!(c, ',' | '_' | '\'' | '\u{202f}')).collect();
    s.parse::<i64>().ok().or_else(|| s.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.round() as i64))
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
            Part::Int(..) | Part::Snapped(..) => 66.0,
            Part::Edit(_, w) => *w,
            Part::Button(..) | Part::Link(_) => 32.0,
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
                changed |= number(ui, seg.shrink2(vec2(4.0, 3.0)), egui::Id::unique((id, k, "number")), v, range.clone(), *speed, None);
            }
            Part::Snapped(v, range, speed, snap, why) => {
                changed |= number(ui, seg.shrink2(vec2(4.0, 3.0)), egui::Id::unique((id, k, "number")), v, range.clone(), *speed, Some((*snap, *why)));
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
            Part::Link(on) => {
                let tip = if **on { tr!("Linked: both sides stay equal") } else { tr!("Unlinked: each side set apart") };
                let r = ui.interact(seg, egui::Id::unique((id, k, "link")), Sense::click()).on_hover_text(tip);
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, **on, tr!("Link the sides")));
                if r.hovered() {
                    painter.rect_filled(seg, corners(k, n), theme::mix(pal.control(), pal.text, 0.08));
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                let (icon, ink) = if **on { (Icon::Link, theme::accent()) } else { (Icon::Unlink, pal.weak) };
                painter.text(seg.center(), Align2::CENTER_CENTER, theme::glyph(icon).to_string(), FontId::proportional(13.5), ink);
                if r.clicked() {
                    **on = !**on;
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

/// A size of two sides in blocks, `w × d`, with a chain between them:
/// linked (as it is at first, remembered under `id`), typing one sets the
/// other to it. `snap` makes each side one it may be. True when either
/// changed.
pub fn size_pair(ui: &mut Ui, id: &str, w: &mut i32, d: &mut i32, range: RangeInclusive<i32>, snap: fn(i32) -> i32, why: &str) -> bool {
    let key = egui::Id::unique((id, "linked"));
    let mut linked = ui.data_mut(|m| *m.get_persisted_mut_or(key, true));
    let (was_w, was_d) = (*w, *d);
    let (mut nw, mut nd) = (*w, *d);
    let (changed, clicked) = input_group(
        ui,
        id,
        &mut [
            Part::Snapped(&mut nw, range.clone(), 64.0, snap, why),
            Part::Link(&mut linked),
            Part::Snapped(&mut nd, range, 64.0, snap, why),
            Part::Text(tr!("blocks")),
        ],
    );
    ui.data_mut(|m| m.insert_persisted(key, linked));
    let (w2, d2) = link_sides(linked, (was_w, was_d), (nw, nd), clicked == Some(1));
    *w = w2;
    *d = d2;
    changed || (w2, d2) != (was_w, was_d)
}

/// Two sides after an edit: linked, the side that changed sets the other
/// (the width when both did); linking them makes the depth the width.
pub fn link_sides(linked: bool, was: (i32, i32), now: (i32, i32), just_linked: bool) -> (i32, i32) {
    if !linked {
        return now;
    }
    if now.0 != was.0 || just_linked {
        (now.0, now.0)
    } else if now.1 != was.1 {
        (now.1, now.1)
    } else {
        now
    }
}

/// A segmented control: choices side by side in a pill, those `on` raised
/// in a pill of their own; one under the other where they are too wide for
/// the room there is. Returns the one clicked.
pub fn button_group(ui: &mut Ui, id: &str, labels: &[&str], on: &[bool]) -> Option<usize> {
    button_group_named(ui, id, labels, labels, on)
}

/// A [`button_group`] of many choices: in one row where it fits, else in
/// rows of as many as fit (the same number in each but the last), rather
/// than all stacked. The choice clicked, by its index in `labels`.
pub fn button_rows(ui: &mut Ui, id: &str, labels: &[&str], on: &[bool]) -> Option<usize> {
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = labels.iter().map(|t| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), Color32::WHITE).size().x + 26.0).collect();
    let room = ui.available_width() - 6.0;
    // The most per row such that every row fits.
    let per_row = (1..=labels.len()).rev().find(|n| widths.chunks(*n).all(|c| c.iter().sum::<f32>() <= room)).unwrap_or(1);
    let mut clicked = None;
    for (r, (names, ons)) in labels.chunks(per_row).zip(on.chunks(per_row)).enumerate() {
        if r > 0 {
            ui.add_space(2.0);
        }
        if let Some(k) = button_group(ui, &format!("{id} {r}"), names, ons) {
            clicked = Some(r * per_row + k);
        }
    }
    clicked
}

/// A [`button_group`] whose choices show `labels` (icons, say) and are
/// named `names` for the screen reader and under the pointer.
pub fn button_group_named(ui: &mut Ui, id: &str, labels: &[&str], names: &[&str], on: &[bool]) -> Option<usize> {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let mut galleys: Vec<_> = labels.iter().map(|t| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), pal.text)).collect();
    let mut widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 26.0).collect();
    const INSET: f32 = 3.0;
    let seg_h = GROUP_H - 2.0 * INSET;
    let stacked = widths.len() > 1 && widths.iter().sum::<f32>() + 2.0 * INSET > ui.available_width();
    let size = if stacked {
        // As wide as the room: a choice longer than it is cut with an
        // ellipsis (its whole name stays the button's).
        let w = (ui.available_width() - 2.0 * INSET).max(24.0);
        for (g, t) in galleys.iter_mut().zip(labels) {
            if g.size().x + 12.0 > w {
                let mut job = egui::text::LayoutJob::simple_singleline((*t).to_owned(), font.clone(), pal.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 12.0);
                *g = ui.painter().layout_job(job);
            }
        }
        widths.iter_mut().for_each(|x| *x = w);
        vec2(w + 2.0 * INSET, seg_h * widths.len() as f32 + 2.0 * INSET)
    } else {
        vec2(widths.iter().sum::<f32>() + 2.0 * INSET, GROUP_H)
    };
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter().clone();
    painter.rect(rect, GROUP_R, if pal.dark { pal.well() } else { pal.header }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.min.x + INSET;
    for (k, (galley, w)) in galleys.into_iter().zip(&widths).enumerate() {
        let seg = if stacked {
            Rect::from_min_size(pos2(rect.min.x + INSET, rect.min.y + INSET + seg_h * k as f32), vec2(*w, seg_h))
        } else {
            Rect::from_min_size(pos2(x, rect.min.y + INSET), vec2(*w, seg_h))
        };
        let pressed = on.get(k).copied().unwrap_or(false);
        // Its own in the Ui it is in: the same settings shown twice (Tool
        // Settings and a layer's dialog) are two controls, each answering
        // its clicks.
        let mut r = ui.interact(seg, ui.make_persistent_id((id, k)), Sense::click());
        let name = names.get(k).copied().unwrap_or(labels[k]);
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, pressed, name));
        if name != labels[k] {
            r = r.on_hover_text(name);
        }
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
            crate::sounds::play(crate::sounds::Cue::Select);
        }
        x += w;
    }
    clicked
}


#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// The text of a field sits in its middle, top to bottom: a text field,
    /// a search field, at 1× and 2×.
    #[test]
    fn text_fields_centre_their_text() {
        use egui_kittest::kittest::NodeT;
        for ppp in [1.0, 2.0] {
            let mut h = Harness::builder().with_size(vec2(320.0, 120.0)).with_pixels_per_point(ppp).with_accessibility_check(false).build_ui_state(
                |ui, s: &mut (String, String)| {
                    theme::install(ui.ctx());
                    ui.add(TextEdit::singleline(&mut s.0).desired_width(200.0));
                    search(ui, &mut s.1, "Search", 200.0);
                },
                ("Generated World".to_owned(), "dirt".to_owned()),
            );
            h.run_steps(3);
            let mut seen = 0;
            for field in h.query_all_by_role(egui::accesskit::Role::TextInput) {
                let outer = field.rect();
                for child in field.children() {
                    if child.accesskit_node().role() != egui::accesskit::Role::TextRun {
                        continue;
                    }
                    let Some(b) = child.accesskit_node().bounding_box() else { continue };
                    let text = Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)) / ppp;
                    seen += 1;
                    let off = text.center().y - outer.center().y;
                    eprintln!("{ppp}x: field {outer:?}, text {text:?}, {off:+.2}");
                    assert!(off.abs() <= 0.5, "{ppp}x: the text {off:+.2} points off the middle of its field");
                    assert!(outer.height() <= 28.5, "{ppp}x: a field {} high, as high as a control", outer.height());
                }
            }
            assert!(seen >= 2, "{seen} texts");
        }
    }

    /// A digit typed into a number clicked to be typed in adds to it.
    #[test]
    fn typing_into_a_number_adds_to_it() {
        let mut h = Harness::builder().with_size(vec2(240.0, 80.0)).with_accessibility_check(false).build_ui_state(
            |ui, value: &mut i32| {
                drag(ui, egui::DragValue::new(value));
            },
            63,
        );
        h.run();
        h.get_by_role(egui::accesskit::Role::SpinButton).click();
        h.run();
        h.get_by_role(egui::accesskit::Role::SpinButton).type_text("5");
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(*h.state(), 635, "the digit went after the number");
    }

    /// A number with a least value typed in from scratch: nothing changes
    /// while it is typed, and what was typed is what it becomes (the size
    /// of a new map once jumped to 128 at the first digit).
    #[test]
    fn typing_a_number_waits_for_the_end() {
        let mut h = Harness::builder().with_size(vec2(240.0, 80.0)).with_accessibility_check(false).build_ui_state(
            |ui, value: &mut i32| {
                drag(ui, egui::DragValue::new(value).range(128..=65536));
            },
            2048,
        );
        h.run();
        h.get_by_role(egui::accesskit::Role::SpinButton).click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        h.get_by_role(egui::accesskit::Role::SpinButton).type_text("1");
        h.run();
        assert_eq!(*h.state(), 2048, "nothing changes while typing");
        h.get_by_role(egui::accesskit::Role::SpinButton).type_text("024");
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(*h.state(), 1024);
    }

    /// The same in a field: the caret where the number was clicked.
    #[test]
    fn typing_into_a_field_goes_where_clicked() {
        let mut h = Harness::builder().with_size(vec2(90.0, 40.0)).with_accessibility_check(false).build_ui_state(
            |ui, value: &mut f32| {
                ui.add(Field::new(value, 0.0..=1000.0).max_decimals(0).width(80.0));
            },
            24.0,
        );
        h.run();
        let rect = h.get_by_role(egui::accesskit::Role::Slider).rect();
        // After the last digit (the value sits at the right end).
        let p = egui::pos2(rect.right() - 4.0, rect.center().y);
        h.event(egui::Event::PointerMoved(p));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
        }
        h.run();
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("5");
        h.run();
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(*h.state(), 245.0);
    }
}
