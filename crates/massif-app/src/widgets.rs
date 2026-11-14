//! Controls of our own, where egui's look like a toolkit's.
//!
//! [`Field`] is the number field of tools such as Blender or Gaea: a bar
//! filled up to its value, the value written in it. Dragged sideways it
//! changes (Shift: finely); clicked, it is typed in.

use crate::theme::{self, Icon, Palette};
use egui::emath::Numeric;
use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, Response, Sense, SliderClamping, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Widget, pos2, vec2};
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

// --------------------------------------------------------------- groups --
//
// As shadcn's input and button groups: the parts of one setting joined in
// one outline, a hairline between them, what is not to be edited muted.

const GROUP_H: f32 = 28.0;
const GROUP_R: u8 = 8;

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
            Part::Text(t) => ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), pal.weak).size().x + 20.0,
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
                            .frame(egui::Frame::NONE)
                            .margin(vec2(8.0, 0.0))
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

/// Buttons joined in one outline, as shadcn's button groups; those `on`
/// are pressed. Returns the one clicked.
pub fn button_group(ui: &mut Ui, id: &str, labels: &[&str], on: &[bool]) -> Option<usize> {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let galleys: Vec<_> = labels.iter().map(|t| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), pal.text)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum(), GROUP_H), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, GROUP_R, group_fill(&pal));
    let n = labels.len();
    let mut clicked = None;
    let mut x = rect.min.x;
    for (k, (galley, w)) in galleys.into_iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.min.y), vec2(*w, GROUP_H));
        let pressed = on.get(k).copied().unwrap_or(false);
        let r = ui.interact(seg, egui::Id::unique((id, k)), Sense::click());
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, pressed, labels[k]));
        let fill = if pressed {
            theme::mix(pal.control(), pal.text, 0.12)
        } else if r.hovered() {
            theme::mix(pal.control(), pal.text, 0.06)
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            painter.rect_filled(seg, corners(k, n), fill);
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let ink = if pressed { pal.text } else { theme::mix(pal.text, pal.weak, 0.45) };
        painter.galley_with_override_text_color(seg.center() - galley.size() / 2.0, galley, ink);
        if k > 0 {
            painter.vline(x, seg.y_range(), Stroke::new(1.0, pal.border));
        }
        if r.clicked() {
            clicked = Some(k);
        }
        x += w;
    }
    painter.rect_stroke(rect, GROUP_R, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    clicked
}

