//! The help: topics written in `assets/help.md`, shown in a window of their
//! own with a list of the topics and a search.
//!
//! The text is a small part of Markdown: `# ` starts a topic, `## ` a
//! heading in it, `- ` a point of a list, `| … | … |` a row of a table (a
//! shortcut and what it does); any other line is a paragraph. In a line,
//! `**…**` is bold and `` `…` `` a key.

use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, RichText, Ui, vec2};
use std::sync::OnceLock;

pub struct Topic {
    pub title: String,
    pub blocks: Vec<Block>,
}

pub enum Block {
    Heading(String),
    Para(String),
    Bullet(String),
    Row(Vec<String>),
}

impl Topic {
    /// Whether the topic speaks of `query` (lower case).
    fn mentions(&self, query: &str) -> bool {
        let has = |s: &str| s.to_lowercase().contains(query);
        has(&self.title)
            || self.blocks.iter().any(|b| match b {
                Block::Heading(t) | Block::Para(t) | Block::Bullet(t) => has(t),
                Block::Row(cells) => cells.iter().any(|c| has(c)),
            })
    }
}

/// The topics, read once.
pub fn topics() -> &'static [Topic] {
    static TOPICS: OnceLock<Vec<Topic>> = OnceLock::new();
    TOPICS.get_or_init(|| parse(include_str!("../assets/help.md")))
}

fn parse(text: &str) -> Vec<Topic> {
    let mut topics: Vec<Topic> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(title) = line.strip_prefix("# ") {
            topics.push(Topic {
                title: title.to_owned(),
                blocks: Vec::new(),
            });
            continue;
        }
        let Some(topic) = topics.last_mut() else { continue };
        topic.blocks.extend(block(line));
    }
    topics
}

/// What one line of the help's Markdown is (`None`: an empty line).
fn block(line: &str) -> Option<Block> {
    Some(if let Some(h) = line.strip_prefix("## ").or_else(|| line.strip_prefix("### ")) {
        Block::Heading(h.to_owned())
    } else if let Some(b) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        Block::Bullet(b.to_owned())
    } else if line.starts_with('|') {
        Block::Row(line.trim_matches('|').split('|').map(|c| c.trim().to_owned()).collect())
    } else if line.trim().is_empty() {
        return None;
    } else {
        Block::Para(line.to_owned())
    })
}

/// Text in the help's Markdown, as blocks: release notes, say.
pub fn blocks(text: &str) -> Vec<Block> {
    text.lines().filter_map(|l| block(l.trim_end())).collect()
}

/// The index of the topic titled `title`.
pub fn topic_index(title: &str) -> usize {
    topics().iter().position(|t| t.title == title).unwrap_or(0)
}

/// Which topic is shown, and what is searched.
#[derive(Debug, Clone, Default)]
pub struct HelpState {
    pub topic: usize,
    pub query: String,
}

/// A line with its bold parts and keys, `size` points high.
fn rich(ui: &Ui, text: &str, size: f32, color: Color32) -> LayoutJob {
    let pal = Palette::of(ui);
    let mut job = LayoutJob::default();
    let (mut bold, mut key) = (false, false);
    let mut rest = text;
    while !rest.is_empty() {
        let next = [rest.find("**"), rest.find('`')].into_iter().flatten().min().unwrap_or(rest.len());
        let (part, tail) = rest.split_at(next);
        if !part.is_empty() {
            let format = if key {
                TextFormat {
                    font_id: FontId::monospace(size - 1.0),
                    color: pal.text,
                    background: pal.control(),
                    ..Default::default()
                }
            } else {
                TextFormat {
                    font_id: FontId::proportional(size),
                    color: if bold { pal.text } else { color },
                    ..Default::default()
                }
            };
            // A little air either side of a key.
            if key {
                job.append(" ", 0.0, format.clone());
            }
            job.append(part, 0.0, format.clone());
            if key {
                job.append(" ", 0.0, format);
            }
        }
        if let Some(t) = tail.strip_prefix("**") {
            bold = !bold;
            rest = t;
        } else if let Some(t) = tail.strip_prefix('`') {
            key = !key;
            rest = t;
        } else {
            rest = tail;
        }
    }
    job
}

/// The help window's content. Returns whether it is to be closed.
pub fn ui(ui: &mut Ui, state: &mut HelpState) -> bool {
    let pal = Palette::of(ui);
    let topics = topics();
    let mut close = false;
    let screen = ui.ctx().content_rect();
    let size = vec2((screen.width() - 80.0).clamp(560.0, 980.0), (screen.height() - 80.0).clamp(360.0, 700.0));
    ui.set_width(size.x);
    // The title, the search and a way out.
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(Icon::BookOpen).to_string()).size(17.0).color(pal.accent));
        ui.label(RichText::new("Massif Help").size(17.0).medium());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let x = egui::Button::new(RichText::new(theme::glyph(Icon::X).to_string()).color(pal.weak)).fill(Color32::TRANSPARENT).stroke(egui::Stroke::NONE);
            if ui.add(x).on_hover_text("Close (Esc)").clicked() {
                close = true;
            }
            ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text(format!("{}  Search the help", theme::glyph(Icon::Search)))
                    .desired_width(240.0),
            );
        });
    });
    ui.add_space(10.0);
    let query = state.query.trim().to_lowercase();
    let shown: Vec<usize> = (0..topics.len()).filter(|&k| query.is_empty() || topics[k].mentions(&query)).collect();
    if !shown.is_empty() && !shown.contains(&state.topic) {
        state.topic = shown[0];
    }
    let body = size.y - 60.0;
    ui.horizontal_top(|ui| {
        // The topics.
        ui.vertical(|ui| {
            ui.set_width(190.0);
            ui.set_min_height(body);
            ui.spacing_mut().item_spacing.y = 2.0;
            for &k in &shown {
                let on = k == state.topic;
                let text = RichText::new(&topics[k].title).size(13.0).color(if on { pal.text } else { pal.weak });
                let button = egui::Button::new(if on { text.medium() } else { text })
                    .fill(if on { pal.accent_soft } else { Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(theme::PILL)
                    .min_size(vec2(190.0, 28.0));
                if ui.add(button).clicked() {
                    state.topic = k;
                }
            }
            if shown.is_empty() {
                ui.label(RichText::new("Nothing about that.").size(12.0).color(pal.weak));
            }
        });
        ui.add_space(8.0);
        let x = ui.cursor().min.x;
        ui.painter().vline(x, ui.cursor().min.y..=ui.cursor().min.y + body, egui::Stroke::new(1.0, pal.border));
        ui.add_space(20.0);
        // The topic.
        let Some(topic) = shown.contains(&state.topic).then(|| &topics[state.topic]) else { return };
        egui::ScrollArea::vertical().id_salt(("help topic", state.topic)).max_height(body).show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| topic_ui(ui, topic));
        });
    });
    close
}

/// One topic: its title, then what it says.
fn topic_ui(ui: &mut Ui, topic: &Topic) {
    ui.set_width(ui.available_width() - 12.0);
    ui.label(RichText::new(&topic.title).size(22.0).medium());
    ui.add_space(6.0);
    blocks_ui(ui, &topic.blocks);
    ui.add_space(12.0);
}

/// Blocks of the help's Markdown, one under the other.
pub fn blocks_ui(ui: &mut Ui, blocks: &[Block]) {
    let pal = Palette::of(ui);
    {
        {
            let body_color = theme::mix(pal.weak, pal.text, 0.55);
            for block in blocks {
                match block {
                    Block::Heading(t) => {
                        ui.add_space(12.0);
                        ui.label(RichText::new(t).size(15.0).medium());
                        ui.add_space(2.0);
                    }
                    Block::Para(t) => {
                        ui.add(egui::Label::new(rich(ui, t, 13.0, body_color)).wrap());
                        ui.add_space(4.0);
                    }
                    Block::Bullet(t) => {
                        ui.horizontal_top(|ui| {
                            ui.add_space(4.0);
                            ui.label(RichText::new("•").size(13.0).color(pal.accent));
                            ui.add(egui::Label::new(rich(ui, t, 13.0, body_color)).wrap());
                        });
                    }
                    Block::Row(cells) => {
                        ui.horizontal_top(|ui| {
                            let (keys, what) = (cells.first().map_or("", |s| s.as_str()), cells.get(1).map_or("", |s| s.as_str()));
                            ui.allocate_ui(vec2(250.0, 0.0), |ui| {
                                ui.set_width(250.0);
                                ui.add(egui::Label::new(rich(ui, keys, 13.0, pal.text)).wrap());
                            });
                            ui.add(egui::Label::new(rich(ui, what, 13.0, body_color)).wrap());
                        });
                        ui.add_space(3.0);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_reads() {
        let topics = topics();
        assert!(topics.len() >= 10, "{} topics", topics.len());
        assert!(topics.iter().all(|t| !t.blocks.is_empty()), "every topic says something");
        let shortcuts = &topics[topic_index("Shortcuts")];
        assert!(shortcuts.blocks.iter().filter(|b| matches!(b, Block::Row(_))).count() > 10);
        assert!(topics.iter().any(|t| t.mentions("beaches")));
    }
}
