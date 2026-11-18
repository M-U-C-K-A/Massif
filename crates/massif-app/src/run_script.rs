//! File ▸ Run Script…: a WorldPainter script, or one written for Massif's
//! own API, run on the open map. The dialog asks for the parameters its
//! descriptor names (and its arguments, unless it hides them), runs it on
//! a thread of its own over a copy of the map, shows what it prints, its
//! progress and the line of its error, and stops it when asked. What it
//! changed comes back as one step to undo.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, row, switch, well};
use egui::{RichText, Ui};
use massif_script::{Descriptor, Ending, Line, ParamKind, ParamValue, Progress, Script};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

/// A parameter's value as the dialog holds it.
enum Value {
    Text(String),
    Number(f64),
    Flag(bool),
    File(Option<PathBuf>),
}

struct Running {
    lines: Receiver<Line>,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<massif_script::Outcome>>,
    started: Instant,
}

enum State {
    Ready,
    Running(Running),
    Done { ending: Ending, elapsed: Duration, changed: bool },
}

/// The dialog.
pub struct ScriptForm {
    path: PathBuf,
    descriptor: Descriptor,
    values: Vec<Value>,
    args: String,
    console: Vec<Line>,
    state: State,
    /// The source could not be read.
    unreadable: Option<String>,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    /// Run it on a copy of the map: the app hands the copy over.
    Run,
    /// What the script made of the map, to take as one step to undo.
    Apply(Box<massif_core::World>, String),
}

impl ScriptForm {
    pub fn new(path: PathBuf) -> Self {
        let (descriptor, unreadable) = match Script::load(&path) {
            Ok(s) => (s.descriptor(), None),
            Err(e) => (Descriptor::default(), Some(e)),
        };
        let values = descriptor
            .params
            .iter()
            .map(|p| match p.parse(p.default.as_deref().unwrap_or("")) {
                ParamValue::String(s) => Value::Text(s),
                ParamValue::Integer(i) => Value::Number(i as f64),
                ParamValue::Float(f) => Value::Number(f),
                ParamValue::Boolean(b) => Value::Flag(b),
                ParamValue::File(f) => Value::File(f),
            })
            .collect();
        Self { path, descriptor, values, args: String::new(), console: Vec::new(), state: State::Ready, unreadable }
    }

    pub fn name(&self) -> String {
        self.descriptor.name.clone().unwrap_or_else(|| self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }

    pub fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    /// Start the script on `world` (a copy of the map), showing dimension
    /// `dim`.
    pub fn start(&mut self, world: massif_core::World, dim: usize, world_path: Option<PathBuf>) {
        let script = match Script::load(&self.path) {
            Ok(s) => s,
            Err(e) => {
                self.console.push(Line::Error(e));
                return;
            }
        };
        // Read again: the script may have been edited since.
        self.descriptor = script.descriptor();
        let params = self
            .descriptor
            .params
            .iter()
            .zip(&self.values)
            .map(|(p, v)| {
                let value = match (p.kind, v) {
                    (ParamKind::Integer | ParamKind::Percentage, Value::Number(n)) => ParamValue::Integer(n.round() as i64),
                    (ParamKind::Float, Value::Number(n)) => ParamValue::Float(*n),
                    (ParamKind::Boolean, Value::Flag(b)) => ParamValue::Boolean(*b),
                    (ParamKind::File, Value::File(f)) => ParamValue::File(f.clone()),
                    (_, Value::Text(t)) => p.parse(t),
                    _ => p.parse(p.default.as_deref().unwrap_or("")),
                };
                (p.name.clone(), value)
            })
            .collect();
        let args: Vec<String> = if self.descriptor.hide_command_line { Vec::new() } else { self.args.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect() };
        let (tx, rx) = channel();
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let folder = script.folder();
        let options = massif_script::Options {
            args,
            params,
            base: folder,
            allow: Vec::new(),
            world: Some((world, dim)),
            world_path,
            out: Arc::new(move |line| {
                let _ = tx.send(line);
            }),
            progress: progress.clone(),
            cancel: cancel.clone(),
        };
        self.console.clear();
        let handle = std::thread::Builder::new()
            .name("script".into())
            .stack_size(16 << 20)
            .spawn(move || massif_script::run(&script, options))
            .ok();
        self.state = State::Running(Running { lines: rx, progress, cancel, handle, started: Instant::now() });
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(620.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::FileCode).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(self.name()).size(16.0).bold());
        });
        let about = self.descriptor.description.clone().unwrap_or_else(|| tr!("A script: WorldPainter's, or one written for Massif's own API.").to_owned());
        ui.add(egui::Label::new(RichText::new(about).size(12.0).color(pal.weak)).wrap());
        ui.label(RichText::new(self.path.display().to_string()).size(11.0).color(pal.weak));
        ui.add_space(10.0);
        if let Some(e) = &self.unreadable {
            ui.label(RichText::new(e).color(pal.danger));
        }

        // Its parameters and arguments, while it is not running.
        let editable = !self.running();
        if !self.descriptor.params.is_empty() {
            ui.add_enabled_ui(editable, |ui| {
                well(ui, |ui| {
                    for (p, v) in self.descriptor.params.iter().zip(self.values.iter_mut()) {
                        let label = if p.optional { tr!("{name} (optional)", name = p.label()) } else { p.label().to_owned() };
                        row(ui, &label, |ui| {
                            let r = match v {
                                Value::Text(t) => ui.add(egui::TextEdit::singleline(t).desired_width(220.0)),
                                Value::Number(n) => match p.kind {
                                    ParamKind::Percentage => ui.add(Field::new(n, 0.0..=100.0).suffix(" %").max_decimals(0).width(140.0)),
                                    ParamKind::Integer => ui.add(Field::new(n, -1e9..=1e9).max_decimals(0).width(140.0)),
                                    _ => ui.add(Field::new(n, -1e9..=1e9).max_decimals(4).width(140.0)),
                                },
                                Value::Flag(b) => switch(ui, b),
                                Value::File(f) => {
                                    let shown = f.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr!("None").to_owned());
                                    let r = ui.button(theme::with_icon(Icon::FolderOpen, &shown));
                                    if r.clicked()
                                        && let Some(path) = rfd::FileDialog::new().set_title(p.label()).pick_file()
                                    {
                                        *f = Some(path);
                                    }
                                    r
                                }
                            };
                            if let Some(d) = &p.description {
                                r.on_hover_text(d);
                            }
                        });
                    }
                });
            });
            ui.add_space(8.0);
        }
        if !self.descriptor.hide_command_line {
            ui.label(RichText::new(tr!("Arguments, one a line")).size(11.5).color(pal.weak).medium());
            ui.add_enabled(editable, egui::TextEdit::multiline(&mut self.args).desired_rows(2).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            ui.add_space(8.0);
        }

        // What it prints.
        self.receive(ui.ctx());
        ui.label(RichText::new(tr!("Output")).size(11.5).color(pal.weak).medium());
        egui::Frame::new().fill(pal.well()).stroke(egui::Stroke::new(1.0, pal.border)).corner_radius(theme::GROUP_RADIUS).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().max_height(220.0).min_scrolled_height(220.0).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                if self.console.is_empty() {
                    let hint = if self.running() { tr!("Running…") } else { tr!("What the script prints shows here.") };
                    ui.label(RichText::new(hint).size(11.5).color(pal.weak));
                }
                for line in &self.console {
                    let (text, colour) = match line {
                        Line::Out(t) => (t.as_str(), pal.text),
                        Line::Warn(t) => (t.as_str(), pal.weak),
                        Line::Error(t) => (t.as_str(), pal.danger),
                    };
                    ui.add(egui::Label::new(RichText::new(text).monospace().size(11.5).color(colour)).wrap());
                }
            });
        });
        ui.add_space(8.0);

        // How far it is, and how it ended.
        match &self.state {
            State::Running(run) => {
                let fraction = run.progress.fraction();
                let label = run.progress.label();
                let elapsed = run.started.elapsed().as_secs_f32();
                let text = if label.is_empty() { tr!("Running · {elapsed:.0} s", elapsed) } else { format!("{label} · {elapsed:.0} s") };
                ui.add(egui::ProgressBar::new(fraction).text(text).animate(!theme::reduce_motion(ui.ctx()) && fraction == 0.0));
                // The output and the progress as they come.
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
            State::Done { ending, elapsed, changed } => {
                let secs = elapsed.as_secs_f32();
                let (text, colour) = match ending {
                    Ending::Done | Ending::Exit(0) if *changed => (tr!("Done in {secs:.1} s: the map changed, one step to undo.", secs), pal.success),
                    Ending::Done | Ending::Exit(0) => (tr!("Done in {secs:.1} s: the map did not change.", secs), pal.success),
                    Ending::Exit(code) => (tr!("The script ended with code {code}; the map is as it was.", code), pal.danger),
                    Ending::Cancelled => (tr!("Stopped; the map is as it was.").to_owned(), pal.weak),
                    Ending::Failed(e) => (
                        match e.line {
                            Some(line) => tr!("Error at line {line}; the map is as it was.", line),
                            None => tr!("Error; the map is as it was.").to_owned(),
                        },
                        pal.danger,
                    ),
                };
                ui.label(RichText::new(text).size(12.0).color(colour));
            }
            State::Ready => {}
        }
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &self.state {
            State::Running(run) => {
                if ui.button(tr!("Stop")).clicked() {
                    run.cancel.store(true, Ordering::Relaxed);
                }
            }
            _ => {
                let again = matches!(self.state, State::Done { .. });
                if ui.add_enabled(self.unreadable.is_none(), theme::primary_button(ui, if again { tr!("Run again") } else { tr!("Run") })).clicked() {
                    choice = Choice::Run;
                }
                if ui.button(tr!("Close")).clicked() {
                    choice = Choice::Close;
                }
                if !self.console.is_empty() && ui.button(theme::with_icon(Icon::Copy, tr!("Copy output"))).clicked() {
                    let text: Vec<&str> = self
                        .console
                        .iter()
                        .map(|l| match l {
                            Line::Out(t) | Line::Warn(t) | Line::Error(t) => t.as_str(),
                        })
                        .collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
            }
        });
        if let Some(applied) = self.finish() {
            choice = applied;
        }
        choice
    }

    /// Take what the script printed.
    fn receive(&mut self, _ctx: &egui::Context) {
        if let State::Running(run) = &self.state {
            while let Ok(line) = run.lines.try_recv() {
                self.console.push(line);
            }
        }
    }

    /// Once the script is done: its ending, and the map to take back.
    fn finish(&mut self) -> Option<Choice> {
        let State::Running(run) = &mut self.state else { return None };
        if !run.handle.as_ref().is_some_and(|h| h.is_finished()) {
            return None;
        }
        let outcome = run.handle.take()?.join().ok()?;
        while let Ok(line) = run.lines.try_recv() {
            self.console.push(line);
        }
        if let Ending::Failed(e) = &outcome.ending {
            self.console.push(Line::Error(e.to_string()));
            if !e.stack.is_empty() {
                self.console.push(Line::Error(e.stack.clone()));
            }
        }
        let ok = matches!(outcome.ending, Ending::Done | Ending::Exit(0));
        let changed = ok && outcome.changed;
        self.state = State::Done { ending: outcome.ending, elapsed: outcome.elapsed, changed };
        match (changed, outcome.world) {
            (true, Some(world)) => Some(Choice::Apply(Box::new(world), tr!("Script {name}", name = self.name()))),
            _ => None,
        }
    }
}
