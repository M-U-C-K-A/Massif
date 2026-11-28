//! The dialog of a problem report (Help ▸ Report a Problem…, see `report`):
//! what happened in the user's words, a performance recording to make, the
//! map to put in or not, and every file that goes in, each to look at as it
//! will be written, before the zip is saved. And the pill that says a
//! recording is on, with its Stop.

use crate::perf::Recording;
use crate::report::{self, Part};
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::i18n::tr;
use egui::{Align, RichText, Stroke, Ui};

/// The dialog's state.
#[derive(Clone, Debug)]
pub struct ReportForm {
    pub inputs: report::Inputs,
    pub include_map: bool,
    /// A map is open, to put in.
    pub has_map: bool,
    /// The files, the map aside, as they will be written.
    parts: Vec<Part>,
    /// The parts were made with this text and this recording.
    made_with: (String, usize),
    /// The file shown in full.
    pub shown: Option<String>,
    /// Opened from the dialog of a start after a crash: back to it after.
    pub after_crash: bool,
}

/// What the dialog asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Stay,
    Close,
    /// Start a recording (the dialog closes until it stops).
    Record,
    /// Leave the recording out.
    DropRecording,
    /// The short summary to the clipboard.
    Copy,
    /// Ask where, then write the zip.
    Save,
}

impl ReportForm {
    pub fn new(inputs: report::Inputs, has_map: bool) -> ReportForm {
        ReportForm { inputs, include_map: false, has_map, parts: Vec::new(), made_with: (String::from("\0"), usize::MAX), shown: None, after_crash: false }
    }

    /// The files, made again when the words or the recording changed.
    pub fn parts(&mut self, recording: Option<&Recording>) -> &[Part] {
        let key = (self.inputs.what_happened.clone(), recording.map_or(0, |r| r.frames() + 1));
        if key != self.made_with {
            self.parts = report::parts(&self.inputs, recording, None);
            self.made_with = key;
        }
        &self.parts
    }
}

/// The dialog, a frame of it.
pub fn ui(ctx: &egui::Context, form: &mut ReportForm, recording: Option<&Recording>, map_note: &str) -> Choice {
    let mut choice = Choice::Stay;
    let response = crate::app::modal(ctx, "report").show(ctx, |ui| {
        ui.set_width(540.0);
        let pal = Palette::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Bug).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Report a Problem")).bold().size(15.0));
        });
        ui.add_space(2.0);
        let about = if form.after_crash {
            tr!("Massif closed without warning last time. A report says what it was doing, on what computer: save it, then send it to us on Discord. Nothing is sent from here.")
        } else {
            tr!("One .zip file with what Massif knows of the problem: save it, then send it to us on Discord. Nothing is sent from here.")
        };
        ui.add(egui::Label::new(RichText::new(about).color(pal.weak)).wrap());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.label(RichText::new(tr!("Where to send it:")).color(pal.weak));
            ui.hyperlink_to(theme::with_icon(Icon::MessagesSquare, tr!("#bug-reports on Discord")), crate::menu::DISCORD);
            ui.hyperlink_to(theme::with_icon(Icon::ExternalLink, tr!("an issue on GitHub")), crate::menu::GITHUB_ISSUES);
        });
        ui.add_space(10.0);

        ui.label(RichText::new(tr!("What happened?")).medium());
        ui.add(egui::TextEdit::multiline(&mut form.inputs.what_happened).hint_text(tr!("What you did, what you saw, what you expected")).desired_rows(3).desired_width(f32::INFINITY).char_limit(2000));
        ui.add_space(10.0);

        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Performance recording"), |ui| match recording.filter(|r| r.frames() > 0) {
                Some(r) => {
                    if ui.button(tr!("Leave out")).clicked() {
                        choice = Choice::DropRecording;
                    }
                    if ui.button(tr!("Record again")).clicked() {
                        choice = Choice::Record;
                    }
                    ui.label(RichText::new(format!("{:.0} s · {}", r.seconds(), trn!("{n} frame", "{n} frames", n = r.frames()))).size(12.0).color(pal.weak));
                }
                None => {
                    if ui.button(theme::with_icon(Icon::Circle, tr!("Start recording"))).clicked() {
                        choice = Choice::Record;
                    }
                }
            });
            ui.add(egui::Label::new(RichText::new(tr!("For slowness: start, do the slow thing, then stop. Each frame and what took its time go in the report.")).size(11.5).color(pal.weak)).wrap());
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.x_range().expand(12.0), r.top() + 3.0, Stroke::new(1.0, pal.border));
            ui.add_space(7.0);
            crate::widgets::row(ui, tr!("Include the map"), |ui| {
                ui.add_enabled_ui(form.has_map, |ui| crate::widgets::switch(ui, &mut form.include_map));
            });
            let note = if form.has_map { map_note } else { tr!("No map is open.") };
            ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
        });
        ui.add_space(10.0);

        ui.label(RichText::new(tr!("What goes in")).medium());
        ui.add_space(2.0);
        let include_map = form.include_map && form.has_map;
        let mut toggle = None;
        let shown = form.shown.clone();
        let parts = form.parts(recording);
        crate::widgets::well(ui, |ui| {
            for (k, p) in parts.iter().enumerate() {
                if k > 0 {
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range().expand(12.0), r.top() + 1.0, Stroke::new(1.0, pal.border));
                    ui.add_space(3.0);
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&p.name).size(12.5).monospace());
                        ui.label(RichText::new(tr(p.about)).size(11.0).color(pal.weak));
                    });
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let open = shown.as_deref() == Some(p.name.as_str());
                        if p.text && ui.selectable_label(open, if open { tr!("Hide") } else { tr!("Show") }).clicked() {
                            toggle = Some(p.name.clone());
                        }
                        ui.label(RichText::new(crate::panels::human(p.bytes.len() as u64)).size(11.5).color(pal.weak));
                    });
                });
                if shown.as_deref() == Some(p.name.as_str()) {
                    show_text(ui, p);
                }
            }
            if include_map {
                let r = ui.available_rect_before_wrap();
                ui.painter().hline(r.x_range().expand(12.0), r.top() + 1.0, Stroke::new(1.0, pal.border));
                ui.add_space(3.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(report::MAP_NAME).size(12.5).monospace());
                    ui.label(RichText::new(tr!("The map itself, as it is now")).size(11.0).color(pal.weak));
                });
            }
        });
        if let Some(name) = toggle {
            form.shown = if form.shown.as_deref() == Some(name.as_str()) { None } else { Some(name) };
        }
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("Your home folder is written ~ and other accounts' names are left out.")).size(11.0).color(pal.weak));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(theme::with_icon(Icon::Copy, tr!("Copy summary"))).on_hover_text(tr!("A few lines for a Discord message: the version, the computer, how it ran")).clicked() {
                choice = Choice::Copy;
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::primary_button(ui, tr!("Save report…"))).clicked() {
                    choice = Choice::Save;
                }
                if ui.button(tr!("Cancel")).clicked() {
                    choice = Choice::Close;
                }
            });
        });
    });
    if choice == Choice::Stay && response.should_close() {
        choice = Choice::Close;
    }
    choice
}

/// A file of the report as it will be written: its first lines, in a box
/// that scrolls.
fn show_text(ui: &mut Ui, p: &Part) {
    let pal = Palette::of(ui);
    const MAX: usize = 60_000;
    let text = String::from_utf8_lossy(&p.bytes[..p.bytes.len().min(MAX)]);
    egui::Frame::new().fill(pal.well()).corner_radius(theme::TILE_RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        egui::ScrollArea::both().id_salt(("report part", &p.name)).max_height(200.0).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text.as_ref()).size(11.0).monospace()).extend());
            if p.bytes.len() > MAX {
                ui.label(RichText::new(tr!("… and {size} more", size = crate::panels::human((p.bytes.len() - MAX) as u64))).size(11.0).color(pal.weak));
            }
        });
    });
    ui.add_space(4.0);
}

/// The pill that says a recording is on, over the top of the window: how
/// long, and its Stop. True when Stop is clicked.
pub fn recording_pill(ctx: &egui::Context, recording: &Recording) -> bool {
    let mut stop = false;
    egui::Area::new(egui::Id::unique("report recording")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 44.0)).show(ctx, |ui| {
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).stroke(Stroke::new(1.0, pal.border)).corner_radius(theme::PILL).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.5, pal.danger);
                let secs = recording.seconds() as u64;
                ui.label(RichText::new(tr!("Recording {time}", time = format!("{}:{:02}", secs / 60, secs % 60))).medium());
                ui.label(RichText::new(trn!("{n} frame", "{n} frames", n = recording.frames())).size(11.5).color(pal.weak));
                if ui.add(theme::primary_button(ui, theme::with_icon(Icon::Square, tr!("Stop")))).clicked() {
                    stop = true;
                }
            });
        });
    });
    // The time goes on: once a second, unless motion is reduced (the
    // frames counted still move as Massif draws).
    if !theme::reduce_motion(ctx) {
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
    stop
}
