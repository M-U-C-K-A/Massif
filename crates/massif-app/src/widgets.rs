//! Controls of our own, where egui's look like a toolkit's.
//!
//! [`Field`] is the number field of tools such as Blender or Gaea: a bar
//! filled up to its value, the value written in it. Dragged sideways it
//! changes (Shift: finely); clicked, it is typed in.

use crate::theme::{self, Palette};
use egui::emath::Numeric;
use egui::{Align2, CursorIcon, FontId, Rect, Response, Sense, SliderClamping, Stroke, StrokeKind, TextEdit, Ui, Widget, vec2};
use std::ops::RangeInclusive;

type Formatter<'a> = Box<dyn 'a + Fn(f64, RangeInclusive<usize>) -> String>;
type Parser<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;

/// A number shown as a filled bar, to drag or to type.
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

impl Widget for Field<'_> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let pal = Palette::of(ui);
        let height = ui.spacing().interact_size.y.clamp(20.0, 24.0);
        let width = self.width.unwrap_or_else(|| ui.available_width().min(ui.spacing().slider_width + 70.0)).max(56.0);
        let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let id = response.id;
        let value = self.get();

        // Typed in.
        let editing: Option<String> = ui.data(|d| d.get_temp(id));
        if let Some(mut buffer) = editing {
            let edit = ui.put(
                rect,
                TextEdit::singleline(&mut buffer)
                    .font(FontId::proportional(12.5))
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

        // Dragged: the value follows, a bar's width for the whole range.
        if response.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let dx = response.drag_delta().x as f64 * if fine { 0.1 } else { 1.0 };
            let start: f64 = if response.drag_started() {
                self.fraction(value)
            } else {
                ui.data(|d| d.get_temp(id.with("drag"))).unwrap_or_else(|| self.fraction(value))
            };
            let t = (start + dx / rect.width() as f64).clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(id.with("drag"), t));
            let v = self.at(t);
            if v != value {
                self.set(v);
                response.mark_changed();
            }
        } else if response.clicked() {
            // Clicked without dragging: typed in.
            let shown = self.format(value);
            let plain = shown.trim_end_matches(self.suffix.as_str()).trim().to_owned();
            ui.data_mut(|d| d.insert_temp(id, plain));
            ui.ctx().request_repaint();
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        // ---- drawing -------------------------------------------------------
        if ui.is_rect_visible(rect) {
            let value = self.get();
            let painter = ui.painter();
            let lit = response.hovered() || response.dragged();
            let base = pal.control();
            let back = if lit { theme::mix(base, pal.text, 0.06) } else { base };
            let radius = 5;
            painter.rect(rect, radius, back, Stroke::new(1.0, if lit { theme::mix(pal.border, pal.text, 0.12) } else { pal.border }), StrokeKind::Inside);
            let t = self.fraction(value) as f32;
            if t > 0.0 {
                let fill = Rect::from_min_max(rect.min, egui::pos2(rect.min.x + rect.width() * t, rect.max.y)).shrink(1.0);
                let tint = theme::mix(base, pal.accent, if pal.dark { 0.42 } else { 0.30 });
                painter.rect_filled(fill, radius.saturating_sub(1), if response.dragged() { theme::mix(tint, pal.accent, 0.2) } else { tint });
            }
            let font = FontId::proportional(12.0);
            let shown = self.format(value);
            let ink = pal.text;
            match &self.text {
                Some(name) => {
                    painter.text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, name, font.clone(), theme::mix(ink, back, 0.25));
                    painter.text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, shown, font, ink);
                }
                None => {
                    painter.text(rect.center(), Align2::CENTER_CENTER, shown, font, ink);
                }
            }
        }
        response
    }
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
