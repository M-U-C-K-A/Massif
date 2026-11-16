//! Edit ▸ Analyse the map: a survival check of the whole map, in a window.
//!
//! Every block is counted as the export writes it and the land measured
//! (see [`massif_core::analysis`]); the window reads that out as a survival
//! player would: what breaks survival in red, what is missing in amber, the
//! way through a game from wood to the Nether, what holds up, and the
//! numbers behind it all — relief, blocks, ores, biomes — beside the map's
//! card. A spot of trouble shows on the map in a click; the whole report
//! copies as Markdown.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets;
use egui::{Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use massif_core::World;
use massif_core::analysis::{self, Analysis, Finding, Mineral, Severity, Stage, Topic, Verdict, percent, pretty, thousands};
use massif_core::auto::Steps;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The main column and the card beside it, in points, and the room
/// between them.
const MAIN: f32 = 612.0;
const SIDE: f32 = 268.0;
const GAP: f32 = 26.0;
/// The window's height, where the screen has room for it.
const HEIGHT: f32 = 720.0;
/// How far the bodies of notes stand in: past their icon.
const INDENT: f32 = 27.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Survival,
    Relief,
    Blocks,
    Biomes,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Survival, Tab::Relief, Tab::Blocks, Tab::Biomes];

    fn label(self) -> &'static str {
        match self {
            Tab::Survival => tr!("Survival"),
            Tab::Relief => tr!("Relief"),
            Tab::Blocks => tr!("Blocks"),
            Tab::Biomes => tr!("Biomes"),
        }
    }
}

/// What the window wants done.
pub enum Choice {
    Keep,
    Close,
    /// Analyse the map again, as it is now.
    Again,
    /// Show this column on the map.
    Show(i32, i32),
    /// Say this, an error or not.
    Say(String, bool),
    /// Open Edit ▸ Ores.
    Ores,
}

/// A finished analysis, read out.
struct Done {
    analysis: Analysis,
    findings: Vec<Finding>,
    stages: Vec<Stage>,
    verdict: Verdict,
    summary: String,
}

/// The pass under way, on another thread.
struct Running {
    steps: Arc<Steps>,
    rx: Receiver<Option<(Analysis, Option<egui::ColorImage>)>>,
}

/// The window.
pub struct AnalysisView {
    /// The map's name and size in blocks, known before the analysis is.
    name: String,
    size: (i32, i32),
    done: Option<Box<Done>>,
    running: Option<Running>,
    /// The map, small, for the card.
    picture: Option<TextureHandle>,
    tab: Tab,
    /// The document's revision analysed.
    revision: u64,
    /// When something was last copied, and what: for the button's word.
    copied: Option<(f64, &'static str)>,
}

impl Drop for AnalysisView {
    fn drop(&mut self) {
        if let Some(r) = &self.running {
            r.steps.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl AnalysisView {
    /// The window, the analysis of a cold copy of the map at `revision`
    /// starting at once.
    pub fn start(ctx: &egui::Context, world: World, dim: usize, revision: u64) -> Self {
        let size = world.dims.get(dim).and_then(|d| d.bounds()).map_or((0, 0), |(lo, hi)| {
            let t = massif_core::TILE_SIZE;
            ((hi.x - lo.x + 1) * t, (hi.z - lo.z + 1) * t)
        });
        let mut view = Self {
            name: world.meta.name.clone(),
            size,
            done: None,
            running: None,
            picture: None,
            tab: Tab::Survival,
            revision,
            copied: None,
        };
        view.again(ctx, world, dim, revision);
        view
    }

    /// Analyse the map again, from a new copy of it.
    pub fn again(&mut self, ctx: &egui::Context, world: World, dim: usize, revision: u64) {
        if let Some(r) = &self.running {
            r.steps.cancel.store(true, Ordering::Relaxed);
        }
        self.name = world.meta.name.clone();
        let steps = Arc::new(Steps::default());
        let (tx, rx) = channel();
        let (s, ctx) = (steps.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("massif analysis".into())
            .spawn(move || {
                let made = analysis::analyse(&world, dim, &s).map(|a| {
                    let picture = massif_core::thumbnail::render(&world, 160)
                        .map(|img| egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()));
                    (a, picture)
                });
                let _ = tx.send(made);
                ctx.request_repaint();
            })
            .expect("a thread for the analysis");
        self.running = Some(Running { steps, rx });
        self.revision = revision;
    }

    /// The document's revision analysed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether an analysis is there to read (for tests).
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done.is_some() && self.running.is_none()
    }

    /// Show tab `k` (for tests).
    #[cfg(test)]
    pub fn set_tab(&mut self, k: usize) {
        self.tab = Tab::ALL[k];
    }

    /// The analysis read out (for tests).
    #[cfg(test)]
    pub fn findings(&self) -> &[Finding] {
        self.done.as_ref().map_or(&[], |d| &d.findings)
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.running else { return };
        match r.rx.try_recv() {
            Ok(made) => {
                self.running = None;
                if let Some((analysis, picture)) = made {
                    let findings = analysis.findings();
                    self.done = Some(Box::new(Done {
                        verdict: Verdict::of(&findings),
                        summary: Analysis::summary(&findings),
                        stages: analysis.progression(),
                        findings,
                        analysis,
                    }));
                    self.picture = picture.map(|img| ctx.load_texture("analysis map", img, TextureOptions::LINEAR));
                }
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(80)),
            Err(TryRecvError::Disconnected) => self.running = None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, revision: u64) -> Choice {
        self.poll(ui.ctx());
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        let height = (ui.ctx().content_rect().height() - 120.0).clamp(380.0, HEIGHT);
        let spacing = ui.spacing().item_spacing;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.allocate_ui_with_layout(vec2(MAIN, height), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing = spacing;
                ui.set_min_size(vec2(MAIN, height));
                ui.set_max_width(MAIN);
                self.main(ui, &pal, revision, &mut choice);
            });
            ui.add_space(GAP);
            let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
            ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, pal.border));
            ui.add_space(GAP);
            ui.allocate_ui_with_layout(vec2(SIDE, height), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing = spacing;
                ui.set_min_size(vec2(SIDE, height));
                ui.set_max_width(SIDE);
                self.side(ui, &pal, &mut choice);
            });
        });
        choice
    }

    // ---------------------------------------------------------- main --

    fn main(&mut self, ui: &mut Ui, pal: &Palette, revision: u64, choice: &mut Choice) {
        // The title, what the map is and the verdict in pills; the commands.
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(MAIN - 190.0);
                ui.label(RichText::new(tr!("Survival check")).size(19.0).bold());
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    let (w, d) = self.size;
                    ui.label(RichText::new(tr!("{thousands} × {thousands2} blocks", thousands = thousands(w as u64), thousands2 = thousands(d as u64))).size(12.5).color(pal.weak));
                    ui.label(RichText::new("·").size(12.5).color(pal.weak));
                    match &self.done {
                        Some(done) => {
                            pill(ui, pal, verdict_word(done.verdict), verdict_tone(pal, done.verdict), true);
                            for s in [Severity::Critical, Severity::Warning] {
                                let n = done.findings.iter().filter(|f| f.severity == s).count() as u64;
                                if n > 0 {
                                    pill(ui, pal, &if s == Severity::Critical { trn!("{n} problem", "{n} problems", n = n) } else { trn!("{n} warning", "{n} warnings", n = n) }, tone(pal, s), false);
                                }
                            }
                        }
                        None => {
                            pill(ui, pal, tr!("Reading"), pal.weak, true);
                        }
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if ui.add(outline_button(pal, &theme::with_icon(Icon::Check, tr!("Done")))).clicked() {
                    *choice = Choice::Close;
                }
                let busy = self.running.is_some();
                if busy {
                    ui.add_space(6.0);
                    crate::loader::loader(ui, 16.0);
                } else if icon_button(ui, pal, Icon::RotateCw, tr!("Check the map again")).clicked() {
                    *choice = Choice::Again;
                }
            });
        });
        ui.add_space(12.0);
        if self.done.is_some() && self.running.is_none() && revision != self.revision {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let amber = tone(pal, Severity::Warning);
                ui.label(RichText::new(theme::glyph(Icon::Clock).to_string()).size(12.5).color(amber));
                ui.label(RichText::new(tr!("The map has changed since this check.")).size(12.0).color(pal.weak));
                if link(ui, pal, None, tr!("Check again")).clicked() {
                    *choice = Choice::Again;
                }
            });
            ui.add_space(10.0);
        }
        let top = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, pal.border));
        ui.add_space(14.0);
        let labels: Vec<&str> = Tab::ALL.iter().map(|t| t.label()).collect();
        let on: Vec<bool> = Tab::ALL.iter().map(|&t| t == self.tab).collect();
        if let Some(k) = widgets::button_group(ui, "analysis tabs", &labels, &on) {
            self.tab = Tab::ALL[k];
        }
        ui.add_space(14.0);
        let height = ui.available_height();
        let copied = &mut self.copied;
        egui::ScrollArea::vertical()
            .id_salt(("analysis", self.tab as u8))
            .auto_shrink([false, false])
            .max_height(height)
            .show(ui, |ui| {
                ui.set_width(MAIN - 16.0);
                match (&self.done, &self.running) {
                    (Some(done), _) => match self.tab {
                        Tab::Survival => survival(ui, pal, done, choice, copied),
                        Tab::Relief => relief(ui, pal, &done.analysis, choice),
                        Tab::Blocks => blocks(ui, pal, &done.analysis),
                        Tab::Biomes => biomes(ui, pal, &done.analysis),
                    },
                    (None, Some(r)) => progress(ui, pal, &r.steps),
                    (None, None) => {
                        ui.add_space(140.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(tr!("Nothing to check")).size(14.5).medium());
                            ui.label(RichText::new(tr!("This map has no ground yet.")).size(12.5).color(pal.weak));
                        });
                    }
                }
                ui.add_space(12.0);
            });
    }

    // ---------------------------------------------------------- side --

    fn side(&mut self, ui: &mut Ui, pal: &Palette, choice: &mut Choice) {
        let height = ui.available_height();
        egui::ScrollArea::vertical().id_salt("analysis side").auto_shrink([false, false]).max_height(height).show(ui, |ui| {
            ui.set_width(SIDE - 8.0);
            // The map's picture, in place of a portrait.
            let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
            match &self.picture {
                Some(t) => {
                    // The middle of the map, square.
                    let [w, h] = t.size().map(|s| s as f32);
                    let (u, v) = (((w - h) / w).max(0.0) / 2.0, ((h - w) / h).max(0.0) / 2.0);
                    egui::Image::new(t)
                        .uv(Rect::from_min_max(pos2(u, v), pos2(1.0 - u, 1.0 - v)))
                        .corner_radius(16)
                        .paint_at(ui, r);
                }
                None => {
                    ui.painter().rect_filled(r, 16, pal.well());
                    glyph(ui.painter(), r.center(), Icon::Map, 22.0, pal.weak);
                }
            }
            ui.painter().rect_stroke(r, 16, Stroke::new(1.0, pal.border), StrokeKind::Inside);
            ui.add_space(14.0);
            ui.add(egui::Label::new(RichText::new(&self.name).size(17.0).bold()).wrap());
            ui.add_space(2.0);
            let (w, d) = self.size;
            let tiles = (w / massif_core::TILE_SIZE) as u64 * (d / massif_core::TILE_SIZE) as u64;
            ui.label(RichText::new(tr!("{thousands} × {thousands2} blocks, {value}", thousands = thousands(w as u64), thousands2 = thousands(d as u64), value = trn!("{n} tile", "{n} tiles", n = tiles))).size(12.5).color(pal.weak));
            ui.add_space(14.0);
            let now = ui.input(|i| i.time);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let done = self.done.as_ref();
                let copied = self.copied.filter(|c| now - c.0 < 1.6 && c.1 == "report").is_some();
                let label = theme::with_icon(if copied { Icon::Check } else { Icon::Copy }, if copied { tr!("Copied") } else { tr!("Copy report") });
                if ui.add_enabled(done.is_some(), outline_button(pal, &label)).on_hover_text(tr!("The whole check as Markdown, to paste where people talk about the map")).clicked()
                    && let Some(done) = done
                {
                    ui.ctx().copy_text(done.analysis.report());
                    self.copied = Some((now, "report"));
                }
                if let Some(done) = done {
                    ui.menu_button(RichText::new(theme::glyph(Icon::Ellipsis).to_string()).size(15.0).color(pal.weak), |ui| {
                        if ui.button(theme::with_icon(Icon::FileDown, tr!("Save the report…"))).clicked() {
                            ui.close();
                            *choice = save_report(&self.name, &done.analysis);
                        }
                        if done.analysis.spawn_column.is_some() && ui.button(theme::with_icon(Icon::MapPin, tr!("Show the spawn point"))).clicked() {
                            ui.close();
                            *choice = Choice::Show(done.analysis.spawn[0], done.analysis.spawn[2]);
                        }
                    });
                }
            });
            ui.add_space(18.0);
            hairline(ui, pal);
            ui.add_space(16.0);
            let Some(done) = &self.done else {
                section_label(ui, pal, tr!("Survival"));
                for name in [tr!("Ores"), tr!("Trees"), tr!("Water"), tr!("Lava"), tr!("Grass"), tr!("Spawn")] {
                    side_row(ui, pal, name, None, "—");
                }
                return;
            };
            let a = &done.analysis;
            let worst = |topics: &[Topic]| done.findings.iter().filter(|f| topics.contains(&f.topic)).map(|f| f.severity).min();
            section_label(ui, pal, tr!("Survival"));
            // The ores: kinds found of the eight, red when riches lie around.
            let kinds = Mineral::OVERWORLD.iter().filter(|&&m| a.mineral(m).ore + a.mineral(m).riches > 0).count();
            let (ores, ores_tone) = match worst(&[Topic::Riches, Topic::Ores]) {
                Some(Severity::Critical) => (tr!("Too rich").to_owned(), Severity::Critical),
                _ if kinds == 0 => (tr!("None").to_owned(), Severity::Warning),
                _ if kinds == 8 => (tr!("All eight kinds").to_owned(), Severity::Good),
                _ => (tr!("{kinds} kinds of 8", kinds), Severity::Warning),
            };
            side_row(ui, pal, tr!("Ores"), Some(tone(pal, ores_tone)), &ores);
            let wood = a.wood();
            let trees = if a.trees == 0 && wood > 0 { tr!("Wood, no trees").to_owned() } else { thousands(a.trees) };
            side_row(ui, pal, tr!("Trees"), worst(&[Topic::Wood]).map(|s| tone(pal, s)), &trees);
            let water = if a.lava_world { tr!("Lava").to_owned() } else { percent(a.wet, a.columns) };
            side_row(ui, pal, tr!("Water"), worst(&[Topic::Water, Topic::Lava]).map(|s| tone(pal, s)), &water);
            let lava = a.lava();
            let lava_word = if lava == 0 { tr!("None").to_owned() } else { tr!("{human} blocks", human = human(lava)) };
            side_row(ui, pal, tr!("Lava"), worst(&[Topic::Nether]).map(|s| tone(pal, s)), &lava_word);
            let grass = a.surface_of(|k| k == analysis::Kind::Grass);
            side_row(ui, pal, tr!("Grass"), worst(&[Topic::Food]).map(|s| tone(pal, s)), &percent(grass, a.grounded()));
            let spawn = match &a.spawn_column {
                None => tr!("Off the map"),
                Some(c) if c.void => tr!("Over the void"),
                Some(c) if a.lava_world && c.depth > 0 => tr!("In lava"),
                Some(c) if c.depth >= 2 => tr!("In water"),
                Some(_) => tr!("Safe"),
            };
            side_row(ui, pal, tr!("Spawn"), worst(&[Topic::Spawn]).map(|s| tone(pal, s)), spawn);
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(theme::glyph(Icon::Database).to_string()).size(12.0).color(pal.weak));
                ui.add(
                    egui::Label::new(
                        RichText::new(tr!("{human} blocks counted as the export writes them, in {seconds:.1} s", human = human(a.solid()), seconds = a.seconds)).size(11.5).color(pal.weak),
                    )
                    .wrap(),
                );
            });
            ui.add_space(18.0);
            hairline(ui, pal);
            ui.add_space(16.0);
            section_label(ui, pal, tr!("Map"));
            side_row(ui, pal, tr!("Chunks"), None, &thousands(a.chunks().round() as u64));
            let r = side_row(ui, pal, tr!("Surface"), None, &format!("{} → {}", a.lowest[1], a.highest[1]));
            sparkline(ui, pal, r, &a.heights);
            side_row(ui, pal, tr!("Mean"), None, &tr!("{mean_height:.0}, sea at {sea_level}", mean_height = a.mean_height(), sea_level = a.sea_level));
            side_row(ui, pal, tr!("Limits"), None, &format!("{} → {}", a.min_y, a.max_y));
            side_row(ui, pal, tr!("Walkable"), None, &percent(a.steps[0] + a.steps[1], a.columns));
            ui.add_space(18.0);
            hairline(ui, pal);
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("Surface")).size(12.5).medium().color(pal.weak));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(a.surface.len().to_string()).size(12.5).color(pal.weak));
                });
            });
            ui.add_space(6.0);
            let ground = a.grounded();
            for c in a.surface.iter().take(5) {
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                let p = ui.painter();
                p.circle_filled(pos2(rect.left() + 4.0, rect.center().y), 4.0, theme::rgb(c.color));
                p.circle_stroke(pos2(rect.left() + 4.0, rect.center().y), 4.0, Stroke::new(1.0, pal.border));
                p.text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, pretty(&c.id), FontId::proportional(12.5), pal.text);
                p.text(rect.right_center(), Align2::RIGHT_CENTER, percent(c.count, ground), FontId::proportional(12.5), pal.weak);
                response.on_hover_text(tr!("{id} · {thousands} columns", id = c.id, thousands = thousands(c.count)));
            }
        });
    }
}

// ------------------------------------------------------------ tabs --

fn survival(ui: &mut Ui, pal: &Palette, done: &Done, choice: &mut Choice, copied: &mut Option<(f64, &'static str)>) {
    // The verdict, as the first message of a thread.
    let v = done.verdict;
    let tone_v = verdict_tone(pal, v);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
        let (fill, line) = tinted(pal, tone_v);
        ui.painter().circle(r.center(), 17.0, fill, Stroke::new(1.0, line));
        glyph(ui.painter(), r.center(), verdict_icon(v), 16.0, tone_v);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::i18n::tr_name(v.label())).size(14.0).medium());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{:.1} s", done.analysis.seconds)).size(11.5).color(pal.weak));
                });
            });
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(&done.summary).size(13.0).color(soft(pal))).wrap());
        });
    });
    ui.add_space(18.0);
    section_label(ui, pal, tr!("From wood to the Nether"));
    stepper(ui, pal, &done.stages);
    ui.add_space(18.0);
    for f in done.findings.iter().filter(|f| f.severity <= Severity::Warning) {
        note(ui, pal, f, choice);
        ui.add_space(10.0);
    }
    fixes(ui, pal, done, copied);
    let fine: Vec<&Finding> = done.findings.iter().filter(|f| f.severity == Severity::Good).collect();
    if !fine.is_empty() {
        ui.add_space(8.0);
        section_label(ui, pal, tr!("Holding up"));
        checklist(ui, pal, &fine, choice);
    }
    let info: Vec<&Finding> = done.findings.iter().filter(|f| f.severity == Severity::Info).collect();
    if !info.is_empty() {
        ui.add_space(16.0);
        section_label(ui, pal, tr!("Worth knowing"));
        checklist(ui, pal, &info, choice);
    }
}

fn relief(ui: &mut Ui, pal: &Palette, a: &Analysis, choice: &mut Choice) {
    section_label(ui, pal, tr!("Heights players stand at"));
    panel(ui, pal, |ui| heights_chart(ui, pal, a));
    ui.add_space(10.0);
    widgets::well(ui, |ui| {
        stat(ui, pal, tr!("Lowest"), format!("y {}", a.lowest[1]), Some((a.lowest[0], a.lowest[2])), choice);
        stat(ui, pal, tr!("Highest"), format!("y {}", a.highest[1]), Some((a.highest[0], a.highest[2])), choice);
        stat(ui, pal, tr!("Mean, median"), format!("{:.1}, {}", a.mean_height(), a.median_height()), None, choice);
        stat(ui, pal, tr!("Spread"), tr!("{height_spread:.1} blocks either way", height_spread = a.height_spread()), None, choice);
        stat(ui, pal, tr!("Above the sea"), percent(a.heights.iter().skip((a.sea_level - a.min_y + 1).max(0) as usize).sum(), a.grounded()), None, choice);
        stat(ui, pal, tr!("Build limits"), tr!("{min_y} to {max_y}", min_y = a.min_y, max_y = a.max_y), None, choice);
    });

    ui.add_space(18.0);
    section_label(ui, pal, tr!("Steps between columns"));
    panel(ui, pal, |ui| steps_chart(ui, pal, a));
    ui.add_space(10.0);
    widgets::well(ui, |ui| {
        stat(ui, pal, tr!("Walkable"), tr!("{percent} of the columns", percent = percent(a.steps[0] + a.steps[1], a.columns)), None, choice);
        stat(ui, pal, tr!("Falls that hurt"), tr!("{thousands} edges", thousands = thousands(a.hurting)), None, choice);
        stat(ui, pal, tr!("Falls that kill"), tr!("{thousands} edges", thousands = thousands(a.lethal)), None, choice);
        let worst = a.worst_drop.map_or(tr!("None").to_owned(), |w| tr!("{value} blocks", value = w.0));
        stat(ui, pal, tr!("Highest drop"), worst, a.worst_drop.map(|w| (w.1, w.2)), choice);
    });

    ui.add_space(18.0);
    section_label(ui, pal, if a.lava_world { tr!("Lava") } else { tr!("Water") });
    widgets::well(ui, |ui| {
        stat(ui, pal, tr!("Covered"), tr!("{percent} of the map", percent = percent(a.wet, a.columns)), None, choice);
        stat(ui, pal, tr!("Blocks"), thousands(a.fluid), None, choice);
        stat(ui, pal, tr!("Deepest"), tr!("{deepest} blocks", deepest = a.deepest), None, choice);
        let mean = if a.wet == 0 { 0.0 } else { a.depth_sum as f64 / a.wet as f64 };
        stat(ui, pal, tr!("Mean depth"), tr!("{mean:.1} blocks", mean), None, choice);
        stat(ui, pal, tr!("Above the sea"), tr!("{thousands} columns: lakes, rivers", thousands = thousands(a.above_sea)), None, choice);
        stat(ui, pal, tr!("Under the surface"), tr!("{thousands} blocks", thousands = thousands(a.buried_fluid)), None, choice);
    });

    ui.add_space(18.0);
    section_label(ui, pal, tr!("Shape"));
    widgets::well(ui, |ui| {
        stat(ui, pal, tr!("More than one floor"), tr!("{thousands} columns", thousands = thousands(a.layered)), None, choice);
        stat(ui, pal, tr!("Air under the surface"), tr!("{thousands} blocks", thousands = thousands(a.hollow)), None, choice);
        stat(ui, pal, tr!("At the top of the world"), tr!("{thousands} columns", thousands = thousands(a.at_ceiling)), a.spots.ceiling, choice);
        stat(ui, pal, tr!("Open to the void"), tr!("{thousands} columns", thousands = thousands(a.void)), a.spots.void, choice);
    });
}

fn blocks(ui: &mut Ui, pal: &Palette, a: &Analysis) {
    section_label(ui, pal, tr!("Ores"));
    if a.minerals.iter().all(|c| c.ore + c.riches == 0) {
        ui.label(RichText::new(tr!("Not one ore on the map: Minecraft adds none to the chunks the export writes finished.")).size(12.0).color(pal.weak));
        ui.add_space(8.0);
    }
    ores_table(ui, pal, a);
    ui.add_space(18.0);
    section_label(ui, pal, tr!("At the surface"));
    shares(ui, pal, &a.surface, a.grounded(), 12, tr!("columns"));
    ui.add_space(18.0);
    section_label(ui, pal, tr!("In the ground"));
    shares(ui, pal, &a.volume, a.solid(), 12, tr!("blocks"));
    ui.add_space(18.0);
    section_label(ui, pal, tr!("Plants"));
    if a.plants.is_empty() {
        muted(ui, pal, tr!("No ground cover grows on the map."));
    } else {
        let total = a.plants.iter().map(|c| c.count).sum();
        shares(ui, pal, &a.plants, total, 10, tr!("plants"));
    }
    ui.add_space(18.0);
    section_label(ui, pal, tr!("Objects"));
    if a.objects.is_empty() {
        muted(ui, pal, tr!("No object stands on the map: no trees, rocks or buildings."));
    } else {
        let total = a.objects.iter().map(|c| c.count).sum();
        shares(ui, pal, &a.objects, total, 10, tr!("times"));
    }
}

fn biomes(ui: &mut Ui, pal: &Palette, a: &Analysis) {
    section_label(ui, pal, tr!("Biomes at the surface, as the export gives them"));
    shares(ui, pal, &a.biomes, a.columns, usize::MAX, tr!("columns"));
}

// ---------------------------------------------------------- pieces --

/// A note: a finding that breaks survival (red) or misses something
/// (amber), in a card tinted its colour.
fn note(ui: &mut Ui, pal: &Palette, f: &Finding, choice: &mut Choice) {
    let c = tone(pal, f.severity);
    let (fill, line) = tinted(pal, c);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, line))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin { left: 16, right: 16, top: 13, bottom: 15 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new(theme::glyph(topic_icon(f.topic)).to_string()).size(16.0).color(c));
                ui.label(RichText::new(&f.title).size(13.5).medium());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match f.spot {
                    Some((x, z)) => {
                        if link(ui, pal, Some(Icon::MapPin), tr!("Show on map")).on_hover_text(format!("x {x}, z {z}")).clicked() {
                            *choice = Choice::Show(x, z);
                        }
                    }
                    None => {
                        ui.label(RichText::new(severity_word(f.severity)).size(11.5).color(pal.weak));
                    }
                });
            });
            ui.add_space(5.0);
            indented(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&f.detail).size(12.5).color(soft(pal))).wrap());
                if !f.chips.is_empty() {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for (name, color) in &f.chips {
                            chip(ui, pal, name, theme::rgb(*color));
                        }
                    });
                }
                if let Some(evidence) = &f.evidence {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(RichText::new(evidence).monospace().size(11.0).color(pal.weak)).wrap());
                }
                if f.action == Some(analysis::Action::Ores) {
                    ui.add_space(10.0);
                    if ui.add(control_button(pal, &theme::with_icon(Icon::Pickaxe, tr!("Set up the ores")))).on_hover_text(tr!("Edit ▸ Ores: where, how many, how large")).clicked() {
                        *choice = Choice::Ores;
                    }
                }
            });
        });
}

/// What to do about what is wrong, in one card.
fn fixes(ui: &mut Ui, pal: &Palette, done: &Done, copied: &mut Option<(f64, &'static str)>) {
    let list: Vec<&Finding> = done.findings.iter().filter(|f| f.severity <= Severity::Warning && f.fix.is_some()).collect();
    if list.is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(raised(pal))
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(theme::CARD_RADIUS)
        .inner_margin(egui::Margin { left: 16, right: 16, top: 13, bottom: 14 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new(theme::glyph(Icon::Sparkles).to_string()).size(16.0).color(pal.accent));
                ui.label(RichText::new(tr!("How to fix it")).size(13.5).medium());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(trn!("From {n} finding", "From {n} findings", n = list.len())).size(11.5).color(pal.weak));
                });
            });
            ui.add_space(6.0);
            indented(ui, |ui| {
                for f in &list {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new(&f.title).size(12.5).medium());
                        ui.label(RichText::new(format!(" — {}", f.fix.as_deref().unwrap_or_default())).size(12.5).color(soft(pal)));
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let now = ui.input(|i| i.time);
                    let done_copy = copied.filter(|c| now - c.0 < 1.6 && c.1 == "fixes").is_some();
                    let label = theme::with_icon(if done_copy { Icon::Check } else { Icon::Copy }, if done_copy { tr!("Copied") } else { tr!("Copy the fixes") });
                    if ui.add(control_button(pal, &label)).clicked() {
                        let text: String = list.iter().map(|f| format!("- {}: {}\n", f.title, f.fix.as_deref().unwrap_or_default())).collect();
                        ui.ctx().copy_text(text);
                        *copied = Some((now, "fixes"));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(tr!("{n} to fix", n = list.len())).monospace().size(11.5).color(pal.weak));
                    });
                });
            });
        });
}

/// Findings in a well, one a row, each with its mark.
fn checklist(ui: &mut Ui, pal: &Palette, items: &[&Finding], choice: &mut Choice) {
    egui::Frame::new()
        .fill(pal.well())
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(theme::GROUP_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (k, f) in items.iter().enumerate() {
                if k > 0 {
                    let y = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range().expand(14.0), y, Stroke::new(1.0, pal.border));
                }
                ui.add_space(11.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 3.0);
                    let (icon, c) = match f.severity {
                        Severity::Good => (Icon::CircleCheck, tone(pal, Severity::Good)),
                        _ => (topic_icon(f.topic), pal.weak),
                    };
                    ui.label(RichText::new(theme::glyph(icon).to_string()).size(14.5).color(c));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&f.title).size(12.5).medium());
                            if let Some((x, z)) = f.spot {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if link(ui, pal, Some(Icon::MapPin), tr!("Show")).on_hover_text(format!("x {x}, z {z}")).clicked() {
                                        *choice = Choice::Show(x, z);
                                    }
                                });
                            } else if f.action == Some(analysis::Action::Ores) {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if link(ui, pal, Some(Icon::Pickaxe), tr!("Ores…")).on_hover_text(tr!("Edit ▸ Ores: where, how many, how large")).clicked() {
                                        *choice = Choice::Ores;
                                    }
                                });
                            }
                        });
                        ui.add(egui::Label::new(RichText::new(&f.detail).size(11.5).color(pal.weak)).wrap());
                        if !f.chips.is_empty() {
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                                for (name, color) in &f.chips {
                                    chip(ui, pal, name, theme::rgb(*color));
                                }
                            });
                        }
                    });
                });
                ui.add_space(11.0);
            }
        });
}

/// The steps of a survival game, from wood to the Nether, each marked.
fn stepper(ui: &mut Ui, pal: &Palette, stages: &[Stage]) {
    let n = stages.len().max(1);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
    let step = rect.width() / n as f32;
    let centre = |k: usize| pos2(rect.left() + step * (k as f32 + 0.5), rect.top() + 15.0);
    let p = ui.painter().clone();
    for k in 0..n.saturating_sub(1) {
        let (a, b) = (centre(k), centre(k + 1));
        let through = stages[k].state == Severity::Good && stages[k + 1].state == Severity::Good;
        let color = if through { theme::mix(pal.border, tone(pal, Severity::Good), 0.55) } else { pal.border };
        p.line_segment([a + vec2(19.0, 0.0), b - vec2(19.0, 0.0)], Stroke::new(1.5, color));
    }
    for (k, s) in stages.iter().enumerate() {
        let c = centre(k);
        let color = tone(pal, s.state);
        let (fill, line) = tinted(pal, color);
        p.circle(c, 14.0, fill, Stroke::new(1.0, line));
        let icon = match s.state {
            Severity::Good => Icon::Check,
            Severity::Critical => Icon::X,
            _ => Icon::Minus,
        };
        glyph(&p, c, icon, 14.0, color);
        p.text(c + vec2(0.0, 25.0), Align2::CENTER_CENTER, crate::i18n::tr_name(s.name), FontId::proportional(12.0), if s.state == Severity::Good { pal.text } else { color });
        let hit = Rect::from_center_size(c + vec2(0.0, 10.0), vec2(step, 48.0));
        ui.interact(hit, egui::Id::unique(("analysis stage", k)), Sense::hover()).on_hover_text(&s.why);
    }
}

/// The columns by the height of their surface, the sea marked.
fn heights_chart(ui: &mut Ui, pal: &Palette, a: &Analysis) {
    let (Some(first), Some(last)) = (a.heights.iter().position(|&c| c > 0), a.heights.iter().rposition(|&c| c > 0)) else {
        muted(ui, pal, tr!("No surface."));
        return;
    };
    let span = last - first + 1;
    let bars = span.min(72);
    let per = span.div_ceil(bars);
    let bins: Vec<(i32, u64)> = (0..span.div_ceil(per))
        .map(|b| {
            let from = first + b * per;
            let to = (from + per).min(last + 1);
            (a.min_y + from as i32, a.heights[from..to].iter().sum())
        })
        .collect();
    let top = bins.iter().map(|b| b.1).max().unwrap_or(1).max(1);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 128.0), Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(0.0, 6.0), rect.max - vec2(0.0, 22.0));
    let p = ui.painter();
    let w = plot.width() / bins.len() as f32;
    let water = if a.lava_world { Color32::from_rgb(232, 112, 40) } else { Color32::from_rgb(64, 132, 230) };
    let land = theme::mix(pal.weak, pal.text, 0.25);
    let hover = response.hover_pos().filter(|p| plot.x_range().contains(p.x));
    let mut tip = None;
    for (k, &(y, count)) in bins.iter().enumerate() {
        let h = (count as f32 / top as f32 * plot.height()).max(if count > 0 { 1.5 } else { 0.0 });
        let x0 = plot.left() + k as f32 * w;
        let bar = Rect::from_min_max(pos2(x0 + 0.75, plot.bottom() - h), pos2(x0 + w - 0.75, plot.bottom()));
        let under = y + per as i32 - 1 < a.sea_level;
        let lit = hover.is_some_and(|p| p.x >= x0 && p.x < x0 + w);
        let fill = if under { water } else { land };
        p.rect_filled(bar, 1.5, if lit { fill } else { fill.gamma_multiply(0.75) });
        if lit {
            tip = Some((y, count));
        }
    }
    p.hline(plot.x_range(), plot.bottom() + 0.5, Stroke::new(1.0, pal.border));
    // The sea level.
    let to_x = |y: i32| plot.left() + ((y - (a.min_y + first as i32)) as f32 / (bins.len() * per) as f32) * plot.width();
    if (a.min_y + first as i32..=a.min_y + last as i32).contains(&a.sea_level) {
        let x = to_x(a.sea_level);
        let mut y = plot.top();
        while y < plot.bottom() {
            p.line_segment([pos2(x, y), pos2(x, (y + 3.0).min(plot.bottom()))], Stroke::new(1.0, water));
            y += 6.0;
        }
        p.text(pos2(x + 4.0, plot.top()), Align2::LEFT_TOP, tr!("Sea {sea_level}", sea_level = a.sea_level), FontId::proportional(11.0), water);
    }
    let label_y = rect.bottom() - 7.0;
    p.text(pos2(plot.left(), label_y), Align2::LEFT_CENTER, format!("y {}", a.min_y + first as i32), FontId::proportional(11.0), pal.weak);
    p.text(pos2(plot.right(), label_y), Align2::RIGHT_CENTER, format!("y {}", a.min_y + last as i32), FontId::proportional(11.0), pal.weak);
    if let Some((y, count)) = tip {
        let range = if per > 1 { tr!("y {y} to {value}", y, value = y + per as i32 - 1) } else { format!("y {y}") };
        response.on_hover_text(tr!("{range}: {thousands} columns, {percent}", thousands = thousands(count), percent = percent(count, a.grounded()), range));
    }
}

/// How the steps between columns share the map: one bar, its legend.
fn steps_chart(ui: &mut Ui, pal: &Palette, a: &Analysis) {
    let total = a.steps.iter().sum::<u64>().max(1);
    let amber = tone(pal, Severity::Warning);
    let colors = [
        tone(pal, Severity::Good),
        theme::mix(tone(pal, Severity::Good), amber, 0.45),
        amber,
        theme::mix(amber, pal.danger, 0.5),
        pal.danger,
    ];
    let names = [
        (tr!("Flat"), tr!("no step to a neighbour")),
        (tr!("One block"), tr!("walked, jumped")),
        (tr!("Two or three"), tr!("a climb")),
        (tr!("4 to 22"), tr!("a fall that hurts")),
        (tr!("23 or more"), tr!("a fall that kills")),
    ];
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(bar, 5, pal.border);
    let mut x = bar.left();
    let shown: Vec<usize> = (0..5).filter(|&k| a.steps[k] > 0).collect();
    for (i, &k) in shown.iter().enumerate() {
        let w = (a.steps[k] as f32 / total as f32 * bar.width()).max(2.0);
        let seg = Rect::from_min_max(pos2(x, bar.top()), pos2((x + w).min(bar.right()), bar.bottom()));
        let r = |first: bool, last: bool| egui::CornerRadius {
            nw: if first { 5 } else { 0 },
            sw: if first { 5 } else { 0 },
            ne: if last { 5 } else { 0 },
            se: if last { 5 } else { 0 },
        };
        p.rect_filled(seg, r(i == 0, i + 1 == shown.len()), colors[k]);
        x += w;
    }
    ui.add_space(12.0);
    for (k, (name, what)) in names.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        let p = ui.painter();
        p.circle_filled(pos2(rect.left() + 4.0, rect.center().y), 4.0, colors[k]);
        let galley = p.layout_no_wrap((*name).to_owned(), FontId::proportional(12.5), pal.text);
        let w = galley.size().x;
        p.galley(pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
        p.text(pos2(rect.left() + 22.0 + w, rect.center().y), Align2::LEFT_CENTER, *what, FontId::proportional(12.0), pal.weak);
        p.text(rect.right_center(), Align2::RIGHT_CENTER, percent(a.steps[k], total), FontId::proportional(12.5), pal.text);
    }
}

/// The ores and riches of every mineral, in a table.
fn ores_table(ui: &mut Ui, pal: &Palette, a: &Analysis) {
    widgets::well(ui, |ui| {
        let columns = [tr!("Ore"), tr!("Blocks"), tr!("In the open"), tr!("A chunk")];
        let right_edges = |rect: Rect| [rect.right() - 246.0, rect.right() - 164.0, rect.right() - 74.0, rect.right()];
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        for (text, x) in columns.iter().zip(right_edges(head)) {
            ui.painter().text(pos2(x, head.center().y), Align2::RIGHT_CENTER, *text, FontId::proportional(11.5), pal.weak);
        }
        for c in &a.minerals {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let p = ui.painter();
            p.hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
            let found = c.ore + c.riches > 0;
            p.circle_filled(pos2(rect.left() + 4.0, rect.center().y), 4.5, theme::rgb(c.mineral.color()));
            p.text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, crate::i18n::tr_name(c.mineral.name()), FontId::proportional(12.5), if found { pal.text } else { pal.weak });
            let per_chunk = c.ore as f64 / a.chunks().max(1.0 / 256.0);
            let values = [
                c.ore.to_string(),
                c.riches.to_string(),
                c.exposed.to_string(),
                if c.ore == 0 { "0".to_owned() } else if per_chunk < 0.1 { "< 0.1".to_owned() } else { tr!("{per_chunk:.1}", per_chunk) },
            ];
            for ((value, x), raw) in values.iter().zip(right_edges(rect)).zip([c.ore, c.riches, c.exposed, c.ore]) {
                let shown = if raw == 0 { "—".to_owned() } else if value.contains('.') || value.contains('<') { value.clone() } else { thousands(raw) };
                p.text(pos2(x, rect.center().y), Align2::RIGHT_CENTER, shown, FontId::proportional(12.5), if raw == 0 { pal.weak } else { pal.text });
            }
            response.on_hover_text(tr!("{name} — for {use_for}", name = crate::i18n::tr_name(c.mineral.name()), use_for = crate::i18n::tr_name(c.mineral.use_for())));
        }
    });
}

/// Rows of a list with their shares of `whole`: a swatch, the name, a bar.
fn shares(ui: &mut Ui, pal: &Palette, list: &[analysis::Count], whole: u64, most: usize, unit: &str) {
    widgets::well(ui, |ui| {
        let whole = whole.max(1);
        for (k, c) in list.iter().take(most).enumerate() {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
            let p = ui.painter();
            if k > 0 {
                p.hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
            }
            let swatch = Rect::from_center_size(pos2(rect.left() + 6.0, rect.center().y), vec2(12.0, 12.0));
            p.rect(swatch, 3, theme::rgb(c.color), Stroke::new(1.0, theme::mix(pal.border, Color32::BLACK, 0.2)), StrokeKind::Inside);
            p.text(pos2(rect.left() + 22.0, rect.center().y), Align2::LEFT_CENTER, pretty(&c.id), FontId::proportional(12.5), pal.text);
            let track = Rect::from_min_max(pos2(rect.left() + 232.0, rect.center().y - 2.5), pos2(rect.right() - 64.0, rect.center().y + 2.5));
            p.rect_filled(track, 2.5, pal.border);
            let share = c.count as f32 / whole as f32;
            if share > 0.0 {
                let fill = Rect::from_min_size(track.min, vec2((track.width() * share).max(3.0), track.height()));
                p.rect_filled(fill, 2.5, theme::mix(theme::rgb(c.color), pal.text, 0.12));
            }
            p.text(rect.right_center(), Align2::RIGHT_CENTER, percent(c.count, whole), FontId::proportional(12.5), pal.text);
            response.on_hover_text(tr!("{id} · {thousands} {unit}", id = c.id, thousands = thousands(c.count), unit));
        }
        if list.len() > most {
            let rest: u64 = list[most..].iter().map(|c| c.count).sum();
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
            let p = ui.painter();
            p.hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
            p.text(pos2(rect.left() + 22.0, rect.center().y), Align2::LEFT_CENTER, tr!("{most} more", most = list.len() - most), FontId::proportional(12.0), pal.weak);
            p.text(rect.right_center(), Align2::RIGHT_CENTER, percent(rest, whole), FontId::proportional(12.0), pal.weak);
        }
    });
}

/// A row of a well: its name muted on the left, its value on the right,
/// and a way to the map where there is a spot.
fn stat(ui: &mut Ui, pal: &Palette, label: &str, value: String, spot: Option<(i32, i32)>, choice: &mut Choice) {
    let first = ui.cursor().top() <= ui.max_rect().top() + 0.5;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    if !first {
        ui.painter().hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
    }
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    row.label(RichText::new(label).size(12.5).color(pal.weak));
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if let Some((x, z)) = spot
            && link(ui, pal, Some(Icon::MapPin), &format!("{x}, {z}")).on_hover_text(tr!("Show on the map")).clicked()
        {
            *choice = Choice::Show(x, z);
        }
        ui.label(RichText::new(value).size(12.5));
    });
}

/// A row of the card beside: its name, a dot of its colour, its value.
/// Returns where the row is.
fn side_row(ui: &mut Ui, pal: &Palette, label: &str, dot: Option<Color32>, value: &str) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 27.0), Sense::hover());
    let p = ui.painter();
    p.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(12.5), pal.weak);
    let mut x = rect.left() + 92.0;
    if let Some(c) = dot {
        p.circle_filled(pos2(x + 4.0, rect.center().y), 4.0, c);
        x += 15.0;
    }
    let galley = p.layout_no_wrap(value.to_owned(), FontId::proportional(12.5), pal.text);
    let w = galley.size().x;
    p.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    Rect::from_min_max(pos2(x + w, rect.top()), rect.max)
}

/// The heights of the map, as a line after a row's value.
fn sparkline(ui: &Ui, pal: &Palette, room: Rect, heights: &[u64]) {
    let (Some(first), Some(last)) = (heights.iter().position(|&c| c > 0), heights.iter().rposition(|&c| c > 0)) else { return };
    let rect = Rect::from_min_max(pos2(room.left() + 12.0, room.top() + 6.0), pos2(room.right().min(room.left() + 12.0 + 92.0), room.bottom() - 6.0));
    if rect.width() < 24.0 {
        return;
    }
    let n = 32usize;
    let span = last - first + 1;
    let bins: Vec<f32> = (0..n)
        .map(|k| {
            let (a, b) = (first + span * k / n, (first + span * (k + 1) / n).max(first + span * k / n + 1).min(last + 1));
            heights[a..b].iter().sum::<u64>() as f32 / (b - a) as f32
        })
        .collect();
    let top = bins.iter().cloned().fold(0.0, f32::max).max(1e-6);
    let points: Vec<egui::Pos2> = bins
        .iter()
        .enumerate()
        .map(|(k, &v)| pos2(rect.left() + rect.width() * k as f32 / (n - 1) as f32, rect.bottom() - (v / top).sqrt() * rect.height()))
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.5, pal.accent)));
}

fn progress(ui: &mut Ui, pal: &Palette, steps: &Steps) {
    let (done, total) = (steps.done.load(Ordering::Relaxed), steps.total.load(Ordering::Relaxed));
    ui.add_space(120.0);
    ui.vertical_centered(|ui| {
        crate::loader::loader(ui, 24.0);
        ui.add_space(16.0);
        ui.label(RichText::new(tr!("Reading every block of the map")).size(14.5).medium());
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("As the export will write them: terrains, rock, water, plants and objects.")).size(12.0).color(pal.weak));
        ui.add_space(18.0);
        let (bar, _) = ui.allocate_exact_size(vec2(300.0, 4.0), Sense::hover());
        ui.painter().rect_filled(bar, 2, pal.border);
        let f = if total == 0 { 0.0 } else { done as f32 / total as f32 };
        if f > 0.0 {
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())), 2, pal.accent);
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("{thousands} of {thousands2} tiles", thousands = thousands(done as u64), thousands2 = thousands(total as u64))).size(11.5).color(pal.weak));
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
}

fn save_report(name: &str, a: &Analysis) -> Choice {
    let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let Some(path) = rfd::FileDialog::new().set_file_name(tr!("{trim} — survival check.md", trim = file.trim())).add_filter(tr!("Markdown"), &["md"]).save_file() else {
        return Choice::Keep;
    };
    match massif_core::atomic::write(&path, a.report().as_bytes(), true) {
        Ok(()) => Choice::Say(tr!("Report saved: {path}", path = path.display()), false),
        Err(e) => Choice::Say(tr!("Could not save the report. {why}", why = massif_core::atomic::describe(&e, &path)), true),
    }
}

// -------------------------------------------------------------- look --

/// The colour of a severity.
fn tone(pal: &Palette, s: Severity) -> Color32 {
    match s {
        Severity::Critical => pal.danger,
        Severity::Warning => {
            if pal.dark {
                Color32::from_rgb(240, 164, 52)
            } else {
                Color32::from_rgb(196, 116, 8)
            }
        }
        Severity::Good => pal.success,
        Severity::Info => pal.weak,
    }
}

fn verdict_tone(pal: &Palette, v: Verdict) -> Color32 {
    tone(
        pal,
        match v {
            Verdict::Ready => Severity::Good,
            Verdict::Gaps => Severity::Warning,
            Verdict::NotReady => Severity::Critical,
        },
    )
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Ready => tr!("Ready"),
        Verdict::Gaps => tr!("Gaps"),
        Verdict::NotReady => tr!("Not ready"),
    }
}

fn verdict_icon(v: Verdict) -> Icon {
    match v {
        Verdict::Ready => Icon::ShieldCheck,
        Verdict::Gaps => Icon::TriangleAlert,
        Verdict::NotReady => Icon::OctagonAlert,
    }
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Critical => tr!("Breaks survival"),
        Severity::Warning => tr!("Missing"),
        Severity::Good => tr!("Fine"),
        Severity::Info => tr!("Worth knowing"),
    }
}

fn topic_icon(t: Topic) -> Icon {
    match t {
        Topic::Riches => Icon::Gem,
        Topic::Ores => Icon::Pickaxe,
        Topic::Bedrock => Icon::Ban,
        Topic::Unbreakable => Icon::LockKeyhole,
        Topic::Void => Icon::CircleDashed,
        Topic::Wood => Icon::TreePine,
        Topic::Stone => Icon::Mountain,
        Topic::Water => Icon::Droplets,
        Topic::Lava => Icon::Flame,
        Topic::Nether => Icon::DoorOpen,
        Topic::Food => Icon::Wheat,
        Topic::Spawn => Icon::MapPin,
        Topic::Drops => Icon::ArrowDownToLine,
        Topic::Ceiling => Icon::ArrowUpToLine,
        Topic::Caves => Icon::Layers,
        Topic::Export => Icon::Package,
    }
}

/// A card's fill and hairline, tinted `c` over the window.
fn tinted(pal: &Palette, c: Color32) -> (Color32, Color32) {
    if pal.dark {
        (theme::mix(pal.panel, c, 0.12), theme::mix(pal.panel, c, 0.40))
    } else {
        (theme::mix(Color32::WHITE, c, 0.07), theme::mix(Color32::WHITE, c, 0.32))
    }
}

/// A card raised a little from the window.
fn raised(pal: &Palette) -> Color32 {
    if pal.dark { theme::mix(pal.panel, pal.text, 0.035) } else { Color32::WHITE }
}

/// Text a little softer than the titles.
fn soft(pal: &Palette) -> Color32 {
    theme::mix(pal.text, pal.weak, 0.22)
}

fn glyph(painter: &egui::Painter, at: egui::Pos2, icon: Icon, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(theme::glyph(icon).to_string(), FontId::proportional(size), color);
    painter.galley(at - galley.size() / 2.0, galley, color);
}

/// A small label in a tinted pill, a dot of its colour before it.
fn pill(ui: &mut Ui, pal: &Palette, text: &str, c: Color32, dot: bool) -> Response {
    let ink = theme::mix(pal.text, c, if pal.dark { 0.18 } else { 0.55 });
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.0), ink);
    let lead = if dot { 15.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 18.0 + lead, 22.0), Sense::hover());
    let (fill, line) = tinted(pal, c);
    let p = ui.painter();
    p.rect(rect, 11, fill, Stroke::new(1.0, line), StrokeKind::Inside);
    if dot {
        p.circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, c);
    }
    p.galley(pos2(rect.left() + 9.0 + lead, rect.center().y - galley.size().y / 2.0), galley, ink);
    response
}

/// A chip: a dot of colour and a name, on the control fill.
fn chip(ui: &mut Ui, pal: &Palette, text: &str, c: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(11.5), pal.text);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 30.0, 22.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, 11, pal.control(), Stroke::new(1.0, pal.control_border()), StrokeKind::Inside);
    p.circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.5, c);
    p.galley(pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
}

/// A pill button outlined: the window's commands.
fn outline_button(pal: &Palette, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).size(13.0).medium())
        .fill(if pal.dark { theme::mix(pal.panel, pal.text, 0.04) } else { Color32::WHITE })
        .stroke(Stroke::new(1.0, pal.control_border()))
        .corner_radius(theme::PILL)
        .min_size(vec2(0.0, 32.0))
}

/// A pill button on the control fill.
fn control_button(pal: &Palette, text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).size(12.5).medium())
        .fill(pal.control())
        .stroke(Stroke::new(1.0, pal.control_border()))
        .corner_radius(theme::PILL)
        .min_size(vec2(0.0, 28.0))
}

/// An icon alone, frameless, its words in a tooltip.
fn icon_button(ui: &mut Ui, pal: &Palette, icon: Icon, tip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    if response.hovered() {
        ui.painter().circle_filled(rect.center(), 16.0, theme::mix(pal.panel, pal.text, 0.08));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    glyph(ui.painter(), rect.center(), icon, 17.0, if response.hovered() { pal.text } else { pal.weak });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip)
}

/// A link: words (and an icon) that do something, underlined when pointed at.
fn link(ui: &mut Ui, pal: &Palette, icon: Option<Icon>, text: &str) -> Response {
    let label = match icon {
        Some(icon) => theme::with_icon(icon, text),
        None => text.to_owned(),
    };
    let response = ui.add(egui::Button::new(RichText::new(label).size(11.5).color(pal.weak)).frame(false).min_size(vec2(0.0, 20.0)));
    if response.hovered() {
        let r = response.rect;
        ui.painter().hline(r.x_range().shrink(4.0), r.bottom() - 3.0, Stroke::new(1.0, pal.weak));
    }
    response
}

/// The name of a group of rows, muted.
fn section_label(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(12.5).medium().color(pal.weak));
    ui.add_space(6.0);
}

fn muted(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(pal.weak));
}

/// A well around a chart.
fn panel(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(pal.well())
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(theme::GROUP_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn hairline(ui: &mut Ui, pal: &Palette) {
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, pal.border));
    ui.add_space(1.0);
}

/// What `add` puts, stood in past the icon of its card.
fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_space(INDENT);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}

/// A large number in words: "4.31 billion", "553 million", "12,402".
fn human(n: u64) -> String {
    match n {
        0..1_000_000 => thousands(n),
        1_000_000..1_000_000_000 => tr!("{e6:.0} million", e6 = n as f64 / 1e6),
        _ => tr!("{e9:.2} billion", e9 = n as f64 / 1e9),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_numbers_read() {
        assert_eq!(human(12_402), "12,402");
        assert_eq!(human(553_194_731), "553 million");
        assert_eq!(human(4_310_990_571), "4.31 billion");
    }
}
