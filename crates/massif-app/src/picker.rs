//! The editors an area can show, as one page: in the popover of an area's
//! header and in an empty area. They stand in columns by what they are
//! for — the World side's views, tools and paint, then the Terrain side's
//! generator — and the line under them says what the one under the pointer
//! is for.

use crate::layout::{EditorKind, Section};
use crate::theme::{self, Palette, RichTextExt};
use egui::{Align2, FontId, Response, RichText, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui, WidgetText, pos2, vec2};
use std::collections::HashSet;

/// Width of a column of editors.
const COLUMN_W: f32 = 176.0;
/// Between two columns.
const GAP: f32 = 8.0;
/// Height of an editor's row.
const ROW_H: f32 = 30.0;
/// Height of a section's title.
const TITLE_H: f32 = 24.0;
/// Height of the line that says what an editor is for.
const ABOUT_H: f32 = 38.0;
/// Where an editor's name starts in its row, after its icon.
const TEXT_X: f32 = 34.0;

/// The width of the page in `columns` columns.
pub fn width(columns: usize) -> f32 {
    let n = columns.max(1) as f32;
    n * COLUMN_W + (n - 1.0) * GAP
}

/// How many columns fit in `room` points: four, two or one.
pub fn columns_for(room: f32) -> usize {
    if room >= width(4) {
        4
    } else if room >= width(2) {
        2
    } else {
        1
    }
}

/// The sections in `columns` columns: the four as they are, or two by two
/// (views and paint, tools and the generator), or one after the other.
fn arrange(columns: usize, developer: bool) -> Vec<Vec<Section>> {
    let mut four: Vec<Vec<Section>> = EditorKind::COLUMNS.iter().map(|c| c.to_vec()).collect();
    if developer {
        four[3].push(EditorKind::DEVELOPER);
    }
    match columns {
        4.. => four,
        2 | 3 => {
            let mut it = four.into_iter();
            let (a, b, c, d) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
            vec![[a, c].concat(), [b, d].concat()]
        }
        _ => vec![four.concat()],
    }
}

/// The page: the editors an area showing `current` can show instead (none
/// for an empty area), in `columns` columns; `shown` are those on screen,
/// Developer's is offered with developer mode. Returns the one chosen.
pub fn page(ui: &mut Ui, current: Option<EditorKind>, shown: &HashSet<EditorKind>, developer: bool, columns: usize) -> Option<EditorKind> {
    let developer = developer || current == Some(EditorKind::Developer);
    let sections = arrange(columns, developer);
    let mut picked = None;
    let mut under = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
        for column in &sections {
            ui.vertical(|ui| {
                ui.set_width(COLUMN_W);
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                for (n, (title, kinds)) in column.iter().enumerate() {
                    if n > 0 {
                        ui.add_space(10.0);
                    }
                    heading(ui, title);
                    for &kind in kinds.iter() {
                        let response = entry(ui, kind, current == Some(kind));
                        if response.hovered() || response.has_focus() {
                            under = Some(kind);
                        }
                        if response.clicked() {
                            picked = Some(kind);
                        }
                    }
                }
            });
        }
    });
    ui.add_space(8.0);
    about(ui, under.or(current), current, shown, width(sections.len()));
    picked
}

/// A section's title, muted, over its editors.
fn heading(ui: &mut Ui, title: &str) {
    let pal = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(COLUMN_W, TITLE_H), Sense::hover());
    let galley = WidgetText::from(RichText::new(title).size(11.5).medium()).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Body);
    let at = pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, pal.weak);
}

/// An editor: its icon and name, lifted under the pointer and when it is
/// the one the area shows (its icon then in the accent).
fn entry(ui: &mut Ui, kind: EditorKind, here: bool) -> Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(COLUMN_W, ROW_H), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, here, kind.label()));
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter();
    if here || hot {
        let fill = theme::mix(pal.panel, pal.text, if here { 0.08 } else { 0.05 });
        painter.rect_filled(rect, theme::TILE_RADIUS, fill);
        if here && pal.high {
            painter.rect_stroke(rect, theme::TILE_RADIUS, ui.visuals().selection.stroke, StrokeKind::Inside);
        }
    }
    let icon = if here {
        pal.accent
    } else if hot {
        pal.text
    } else {
        pal.weak
    };
    painter.text(
        pos2(rect.left() + 18.0, rect.center().y),
        Align2::CENTER_CENTER,
        theme::glyph(kind.icon()).to_string(),
        FontId::proportional(14.0),
        icon,
    );
    let name = RichText::new(kind.label()).size(13.0);
    let name = if here { name.medium() } else { name };
    let room = rect.width() - TEXT_X - 8.0;
    let galley = WidgetText::from(name).into_galley(ui, Some(TextWrapMode::Truncate), room, TextStyle::Body);
    let ink = if here || hot { pal.text } else { theme::mix(pal.text, pal.weak, 0.35) };
    painter.galley(pos2(rect.left() + TEXT_X, rect.center().y - galley.size().y / 2.0), galley, ink);
    response
}

/// Under a hairline: what `kind` is for and where it shows already — or,
/// with none, how areas move.
fn about(ui: &mut Ui, kind: Option<EditorKind>, current: Option<EditorKind>, shown: &HashSet<EditorKind>, width: f32) {
    let pal = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, ABOUT_H), Sense::hover());
    let painter = ui.painter();
    painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0, pal.border));
    let y = rect.top() + 1.0 + (ABOUT_H - 1.0) / 2.0;
    let line = |ui: &Ui, text: RichText, room: f32| WidgetText::from(text.size(12.0)).into_galley(ui, Some(TextWrapMode::Truncate), room.max(0.0), TextStyle::Body);
    let Some(kind) = kind else {
        let galley = line(ui, RichText::new("Drag an area by its title to move it, or onto another to swap them"), rect.width() - 20.0);
        painter.galley(pos2(rect.left() + 10.0, y - galley.size().y / 2.0), galley, pal.weak);
        return;
    };
    painter.text(pos2(rect.left() + 18.0, y), Align2::CENTER_CENTER, theme::glyph(kind.icon()).to_string(), FontId::proportional(14.0), pal.weak);
    // Where it shows, on the right.
    let mut right = rect.right() - 10.0;
    let status = if Some(kind) == current {
        Some("This area")
    } else if shown.contains(&kind) {
        Some("Shown in another area")
    } else {
        None
    };
    if let Some(status) = status {
        let galley = line(ui, RichText::new(status), f32::INFINITY);
        right -= galley.size().x;
        painter.galley(pos2(right, y - galley.size().y / 2.0), galley, pal.weak);
        right -= 16.0;
    }
    // Its name, then what it is for.
    let left = rect.left() + TEXT_X;
    let name = line(ui, RichText::new(kind.label()).medium(), right - left);
    let after = left + name.size().x + 8.0;
    painter.galley(pos2(left, y - name.size().y / 2.0), name, pal.text);
    let what = line(ui, RichText::new(kind.about()), right - after);
    painter.galley(pos2(after, y - what.size().y / 2.0), what, pal.weak);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every editor but the empty area and Developer's is offered once, in
    /// whatever number of columns.
    #[test]
    fn every_editor_is_offered_once() {
        for columns in [1, 2, 4] {
            let sections = arrange(columns, true);
            assert_eq!(sections.len(), columns);
            let mut offered: Vec<EditorKind> = sections.iter().flatten().flat_map(|(_, kinds)| kinds.iter().copied()).collect();
            offered.sort_by_key(|k| *k as u8);
            let before = offered.len();
            offered.dedup();
            assert_eq!(offered.len(), before, "an editor offered twice");
            assert_eq!(offered.len(), 18, "every editor but the empty area");
            assert!(!offered.contains(&EditorKind::Empty));
        }
        assert!(!arrange(4, false).iter().flatten().any(|(_, kinds)| kinds.contains(&EditorKind::Developer)));
    }

    #[test]
    fn columns_follow_the_room() {
        assert_eq!(columns_for(width(4)), 4);
        assert_eq!(columns_for(width(4) - 1.0), 2);
        assert_eq!(columns_for(width(2)), 2);
        assert_eq!(columns_for(width(2) - 1.0), 1);
    }
}
