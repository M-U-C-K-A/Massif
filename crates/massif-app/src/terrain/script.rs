//! The Script pane beside the graph: the graph written as MassifScript,
//! coloured, its mistakes underlined as it is typed; Apply makes the graph
//! from it (one step to undo), From graph writes it again, Copy for AI
//! copies a guide to paste into any AI chat; `.mscript` files open and save.

use super::panel::TerrainPanel;
use crate::theme::{self, Icon, Palette};
use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, Stroke, Ui, vec2};
use massif_terrain::mscript::{self, Error};
use std::path::PathBuf;

/// Where the text shown came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    /// Written from the graph: written again when the graph changes, until
    /// it is edited.
    #[default]
    Graph,
    /// Typed, applied, or read from a file: kept as it is.
    Own,
}

#[derive(Default)]
pub struct ScriptPane {
    pub open: bool,
    pub text: String,
    /// The text as last written from the graph, applied or opened.
    pub base: String,
    pub origin: Origin,
    /// The panel's revision the text follows.
    pub revision: u64,
    /// What checking the text found, and the text checked.
    pub errors: Vec<Error>,
    checked: Option<String>,
    /// The `.mscript` file the text came from or went to.
    pub path: Option<PathBuf>,
    /// The project's name scripts are written under (the world's).
    pub title: String,
}

impl ScriptPane {
    /// The text was changed here and not applied.
    pub fn edited(&self) -> bool {
        self.text != self.base
    }

    /// Writes the graph as text, unless the text shown is the user's own.
    pub fn follow(&mut self, p_doc: impl FnOnce() -> massif_terrain::model::Graph, revision: u64, force: bool) {
        if !force && (self.revision == revision || self.edited() || self.origin == Origin::Own) {
            return;
        }
        let title = if self.title.is_empty() { None } else { Some(self.title.as_str()) };
        self.text = mscript::decompile(&p_doc(), title);
        self.base = self.text.clone();
        self.origin = Origin::Graph;
        self.revision = revision;
    }

    /// Checks the text when it changed since.
    pub fn check(&mut self) {
        if self.checked.as_deref() == Some(self.text.as_str()) {
            return;
        }
        let mut loader = mscript::FileLoader { dir: self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())) };
        self.errors = mscript::compile(&self.text, &mut loader).errors;
        self.checked = Some(self.text.clone());
    }

    fn has_errors(&self) -> bool {
        self.errors.iter().any(|e| !e.warning)
    }
}

/// The graph made from the pane's text, laid out where the nodes were.
pub fn apply(p: &mut TerrainPanel) -> Result<(), String> {
    let s = &p.script;
    let mut loader = mscript::FileLoader { dir: s.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())) };
    let out = mscript::compile(&s.text, &mut loader);
    let Some(graph) = out.graph else {
        let e = out.errors.into_iter().find(|e| !e.warning).map(|e| e.to_string()).unwrap_or_default();
        return Err(e);
    };
    p.replace_document(graph);
    let s = &mut p.script;
    s.base = s.text.clone();
    s.origin = Origin::Own;
    s.revision = p.revision;
    Ok(())
}

/// The guide to paste into an AI chat, with the graph open now as a script.
pub fn guide_for(p: &TerrainPanel) -> String {
    let s = &p.script;
    let current = if !s.text.trim().is_empty() && !s.has_errors() { s.text.clone() } else { mscript::decompile(&p.document(), if s.title.is_empty() { None } else { Some(&s.title) }) };
    mscript::guide(Some(&current))
}

const KEYWORDS: &[&str] = &["project", "group", "component", "expose", "output", "const", "let", "if", "else", "for", "in", "import", "true", "false"];

/// What each part of the text is, for its colour.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Plain,
    Comment,
    Keyword,
    Type,
    Literal,
    Punct,
}

/// The text coloured, its mistakes underlined.
pub fn highlight(text: &str, errors: &[Error], pal: &Palette, font: FontId) -> LayoutJob {
    let b = text.as_bytes();
    let mut kinds = vec![Kind::Plain; b.len()];
    let mut i = 0;
    let mark = |kinds: &mut Vec<Kind>, a: usize, z: usize, k: Kind| kinds[a..z.min(b.len())].iter_mut().for_each(|x| *x = k);
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            mark(&mut kinds, start, i, Kind::Comment);
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            mark(&mut kinds, start, i, Kind::Comment);
        } else if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
            mark(&mut kinds, start, i, Kind::Literal);
        } else if c.is_ascii_digit() || c == b'#' {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))) {
                i += 1;
            }
            mark(&mut kinds, start, i, Kind::Literal);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let called = matches!(b.get(j), Some(b'(' | b'@'));
            let k = if KEYWORDS.contains(&word) {
                Kind::Keyword
            } else if called && word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Kind::Type
            } else {
                Kind::Plain
            };
            mark(&mut kinds, start, i, k);
        } else {
            if c.is_ascii_punctuation() {
                mark(&mut kinds, start, start + 1, Kind::Punct);
            }
            i += text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    // Where the mistakes are (in this text, not the files it imports).
    let mut wrong = vec![0u8; b.len()];
    for e in errors.iter().filter(|e| e.span.file == 0) {
        let (a, z) = (e.span.start.min(b.len()), e.span.end.min(b.len()));
        // An empty span (a “;” missing) underlines the character before.
        let (a, z) = if z > a { (a, z) } else { (a.saturating_sub(1), a.max(1).min(b.len())) };
        let level = if e.warning { 1 } else { 2 };
        wrong[a..z].iter_mut().for_each(|w| *w = (*w).max(level));
    }
    let literal = pal.text.lerp_to_gamma(pal.accent, 0.45);
    let format = |k: Kind, w: u8| {
        let color = match k {
            Kind::Plain => pal.text,
            Kind::Comment => pal.weak,
            Kind::Keyword => pal.accent,
            Kind::Type => pal.accent.lerp_to_gamma(pal.text, 0.35),
            Kind::Literal => literal,
            Kind::Punct => pal.weak,
        };
        let mut f = TextFormat { font_id: font.clone(), color, italics: k == Kind::Comment, ..Default::default() };
        match w {
            2 => f.underline = Stroke::new(1.5, pal.danger),
            1 => f.underline = Stroke::new(1.0, pal.weak),
            _ => {}
        }
        f
    };
    let mut job = LayoutJob::default();
    let mut at = 0;
    while at < b.len() {
        let (k, w) = (kinds[at], wrong[at]);
        let mut end = at + 1;
        while end < b.len() && kinds[end] == k && wrong[end] == w {
            end += 1;
        }
        // Whole characters only.
        while end < b.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        job.append(&text[at..end], 0.0, format(k, w));
        at = end;
    }
    job
}

/// The pane, in `ui`'s rectangle.
pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    let revision = p.revision;
    {
        let doc = p.document();
        p.script.follow(|| doc, revision, false);
    }
    p.script.check();
    let now = ui.input(|i| i.time);
    let full = ui.max_rect();
    let inner = full.shrink2(vec2(theme::PAD, 8.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut ui;
    // Header: the title, then the commands.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(t!("Script")).strong());
        ui.label(egui::RichText::new(t!("Experimental")).small().color(pal.weak)).on_hover_text(t!("MassifScript is new in 1.2: its language may still change"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let icon = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| -> bool {
                let r = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(theme::glyph(icon).to_string()).size(13.0)).frame(false).min_size(vec2(24.0, 24.0)));
                r.on_hover_text(tip).clicked()
            };
            if icon(ui, Icon::X, t!("Close the script"), true) {
                p.script.open = false;
            }
            crate::widgets::menu_button(ui, theme::glyph(Icon::FileCode).to_string(), |ui| {
                if crate::widgets::button(ui, t!("Open MassifScript…")).clicked() {
                    ui.close();
                    open_file(p, now);
                }
                if crate::widgets::button(ui, t!("Save as MassifScript…")).clicked() {
                    ui.close();
                    save_file(p, now);
                }
            });
            if icon(ui, Icon::Sparkles, t!("Copy for AI: a guide to MassifScript, every node, and this graph, to paste into any AI chat before your request"), true) {
                let text = guide_for(p);
                ui.ctx().copy_text(text);
                p.say(t!("Copied: paste it into an AI chat, then say what to make. Paste its answer here and Apply."), now);
            }
            if icon(ui, Icon::RefreshCw, t!("Graph → script: write the graph as it is now (what is typed here is replaced)"), true) {
                let doc = p.document();
                let r = p.revision;
                p.script.follow(|| doc, r, true);
                p.script.check();
            }
            let can = p.script.edited() || p.script.origin == Origin::Own;
            let apply_label = format!("{} {}", theme::glyph(Icon::Play), t!("Apply"));
            let r = ui.add_enabled(can && !p.script.has_errors() && !p.read_only(), egui::Button::new(apply_label));
            if r.on_hover_text(t!("Make the graph from the script (one step to undo)")).clicked() {
                match apply(p) {
                    Ok(()) => p.say(t!("The graph is made from the script"), now),
                    Err(e) => p.say(e, now),
                }
            }
        });
    });
    // What the text is.
    let s = &p.script;
    let errors = s.errors.iter().filter(|e| !e.warning).count();
    let warnings = s.errors.len() - errors;
    let status = if errors == 1 {
        t!("A mistake: the graph is left as it is").to_owned()
    } else if errors > 0 {
        t!("{n} mistakes: the graph is left as it is", n = errors)
    } else if s.edited() {
        t!("Edited: Apply makes the graph from it").to_owned()
    } else if s.origin == Origin::Own && s.revision != p.revision {
        t!("The graph changed since: Graph → script writes it again").to_owned()
    } else {
        t!("The graph as a script").to_owned()
    };
    let status = match warnings {
        0 => status,
        1 => format!("{status} · {}", t!("a warning")),
        n => format!("{status} · {}", t!("{n} warnings", n = n)),
    };
    ui.label(egui::RichText::new(status).small().color(if errors > 0 { pal.danger } else { pal.weak }));
    ui.add_space(4.0);
    // The mistakes, under the text.
    let list_h = (s.errors.len().min(4) as f32) * 18.0 + if s.errors.is_empty() { 0.0 } else { 8.0 };
    let text_h = (ui.available_height() - list_h).max(60.0);
    let font = FontId::monospace(12.5);
    let errs = p.script.errors.clone();
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(text.as_str(), &errs, &pal, font.clone());
        // Lines are never folded: their numbers stay beside them.
        let _ = wrap;
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::Frame::new().fill(pal.header).corner_radius(theme::TILE_RADIUS).inner_margin(6.0).show(ui, |ui| {
        egui::ScrollArea::both().id_salt("terrain script").max_height(text_h - 12.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Line numbers, in the text's font and rows.
                let lines = p.script.text.split('\n').count().max(1);
                let numbers: String = (1..=lines).map(|k| format!("{k:>3}\n")).collect();
                let mut job = LayoutJob::default();
                job.append(numbers.trim_end(), 0.0, TextFormat { font_id: font.clone(), color: pal.weak, ..Default::default() });
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let (r, _) = ui.allocate_exact_size(galley.size() + vec2(6.0, 0.0), egui::Sense::hover());
                ui.painter().galley(egui::pos2(r.left(), r.top() + 2.0), galley, pal.weak);
                ui.spacing_mut().item_spacing.x = 4.0;
                let edit = egui::TextEdit::multiline(&mut p.script.text).code_editor().desired_width(f32::INFINITY).desired_rows(20).frame(egui::Frame::NONE).margin(egui::Margin { left: 0, right: 0, top: 2, bottom: 2 }).lock_focus(true).layouter(&mut layouter);
                ui.add(edit);
            });
        });
    });
    if !errs.is_empty() {
        ui.add_space(4.0);
        for e in errs.iter().take(4) {
            let color = if e.warning { pal.weak } else { pal.danger };
            ui.add(egui::Label::new(egui::RichText::new(e.to_string()).small().color(color)).truncate());
        }
    }
}

fn open_file(p: &mut TerrainPanel, now: f64) {
    let Some(path) = rfd::FileDialog::new().set_title(t!("Open MassifScript")).add_filter("MassifScript", &["mscript"]).pick_file() else { return };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let s = &mut p.script;
            s.text = text;
            s.base = String::new();
            s.origin = Origin::Own;
            s.path = Some(path);
            s.open = true;
        }
        Err(e) => p.say(e.to_string(), now),
    }
}

fn save_file(p: &mut TerrainPanel, now: f64) {
    let name = p.script.path.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("{}.mscript", if p.script.title.is_empty() { "terrain" } else { &p.script.title }));
    let Some(path) = rfd::FileDialog::new().set_title(t!("Save as MassifScript")).set_file_name(name).add_filter("MassifScript", &["mscript"]).save_file() else { return };
    match std::fs::write(&path, &p.script.text) {
        Ok(()) => p.script.path = Some(path),
        Err(e) => p.say(e.to_string(), now),
    }
}
