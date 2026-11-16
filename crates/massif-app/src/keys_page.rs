//! Settings ▸ Keyboard: every shortcut of the keymap (`keymap`), in columns
//! by area. A shortcut clicked listens for new keys; keys another action
//! has where both listen ask first (Replace or Cancel). A preset to start
//! from, a search, a reset of one action or of all, and keymap files out
//! and in.

use crate::keymap::{self, Action, Area, Chord, Kind, Mods, Preset, Saved};
use crate::theme::{self, Icon, Palette, RichTextExt};
use egui::{Color32, CursorIcon, FontId, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

/// Where the page is: what it listens for, what it asks.
#[derive(Clone, Default)]
struct State {
    query: String,
    /// The action and the place of the key being recorded (one past its
    /// keys: a key added).
    recording: Option<(Action, usize)>,
    /// The modifiers held since recording began, with no key yet.
    mods: Mods,
    /// Keys recorded that another action has: what is asked.
    clash: Option<Clash>,
    /// What the last import or recording said.
    note: Option<String>,
}

#[derive(Clone)]
struct Clash {
    action: Action,
    slot: usize,
    chord: Chord,
    others: Vec<Action>,
}

/// The width of a column.
const COLUMN: f32 = 290.0;
const GAP: f32 = 18.0;
/// The width the page takes: three columns.
pub const WIDTH: f32 = 3.0 * COLUMN + 2.0 * GAP;
const ROW_H: f32 = 30.0;

fn id() -> Id {
    Id::unique("settings keyboard")
}

/// The page, changing `saved`.
pub fn page(ui: &mut Ui, saved: &mut Saved) {
    let pal = Palette::of(ui);
    let mut state: State = ui.data(|d| d.get_temp(id())).unwrap_or_default();
    let map = saved.keymap();
    if state.recording.is_some() {
        record(ui, saved, &map, &mut state);
    }

    // The preset, the search, files and reset, on one line.
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("Start from")).size(12.5));
        let on = Preset::ALL.map(|p| p == saved.preset);
        let names = Preset::ALL.map(Preset::name);
        if let Some(k) = crate::widgets::button_group(ui, "keymap preset", &names, &on) {
            saved.preset = Preset::ALL[k];
            state.note = Some(tr!("The {preset} keys, with your changes over them.", preset = Preset::ALL[k].name()));
        }
        ui.add_space(10.0);
        let hint = format!("{}  {}", theme::glyph(Icon::Search), tr!("Search"));
        ui.add(egui::TextEdit::singleline(&mut state.query).hint_text(hint).desired_width(220.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let reset = ui.add_enabled(!saved.changed.is_empty(), egui::Button::new(theme::with_icon(Icon::RotateCcw, tr!("Reset all"))));
            if reset.clicked() {
                saved.changed.clear();
                state.note = Some(tr!("Every shortcut is as in {preset} again.", preset = saved.preset.name()));
            }
            if ui.button(theme::with_icon(Icon::Upload, tr!("Export…"))).clicked() {
                export(saved, &mut state);
            }
            if ui.button(theme::with_icon(Icon::Download, tr!("Import…"))).clicked() {
                import(saved, &mut state);
            }
        });
    });
    ui.add_space(8.0);

    // What a recording asks, or said.
    if let Some(clash) = state.clash.clone() {
        let names: Vec<String> = clash.others.iter().map(|a| a.name()).collect();
        notice(ui, Icon::TriangleAlert, pal.danger, &tr!("{keys} is already {actions}.", keys = clash.chord.label(), actions = names.join(", ")), |ui| {
            if ui.add(theme::primary_button(ui, tr!("Replace"))).clicked() {
                for other in &clash.others {
                    let keys: Vec<Chord> = saved.keymap().keys(*other).iter().copied().filter(|c| *c != clash.chord).collect();
                    saved.set(*other, &keys);
                }
                put(saved, clash.action, clash.slot, clash.chord);
                state.clash = None;
            }
            if ui.button(tr!("Cancel")).clicked() {
                state.clash = None;
            }
        });
    } else if let Some(note) = state.note.clone() {
        notice(ui, Icon::Info, pal.weak, &note, |ui| {
            if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false)).clicked() {
                state.note = None;
            }
        });
    }

    let query = state.query.trim().to_lowercase();
    let map = saved.keymap();
    let shown = |a: &Action| query.is_empty() || a.name().to_lowercase().contains(&query) || map.keys(*a).iter().any(|c| c.label().to_lowercase().contains(&query) || c.id().to_lowercase().contains(&query));
    let all = Action::all();
    let mut columns: Vec<(Area, Vec<Action>)> = Area::ALL.iter().map(|area| (*area, all.iter().filter(|a| a.area() == *area && shown(a)).copied().collect())).filter(|(_, list): &(Area, Vec<Action>)| !list.is_empty()).collect();
    if columns.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(tr!("No shortcut for \u{201c}{query}\u{201d}.", query = state.query.trim())).color(pal.weak)));
    }
    // Three columns a row: General, Tools, Brushes; Map, 3D view, Player.
    let column = ((ui.available_width() - 2.0 * GAP) / 3.0).min(COLUMN);
    while !columns.is_empty() {
        let rest = columns.split_off(columns.len().min(3));
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (area, actions) in &columns {
                ui.vertical(|ui| {
                    ui.set_width(column);
                    ui.label(RichText::new(area.name()).size(12.0).medium().color(pal.weak));
                    ui.add_space(4.0);
                    crate::widgets::well(ui, |ui| {
                        for (n, action) in actions.iter().enumerate() {
                            action_row(ui, n == 0, *action, saved, &map, &mut state);
                        }
                    });
                });
            }
        });
        ui.add_space(14.0);
        columns = rest;
    }

    // The mouse keeps its own.
    let mouse = if keymap::MAC {
        tr!("The mouse keeps its own: the wheel sizes the brush (Shift: a block at a time), ⌘ and the wheel turn it, ⌥ or ⌃ and the wheel zoom, ⌥ with the left button does the tool's inverse.")
    } else {
        tr!("The mouse keeps its own: the wheel sizes the brush (Shift: a block at a time), Ctrl + Shift and the wheel turn it, Alt or Ctrl and the wheel zoom, Alt with the left button does the tool's inverse.")
    };
    ui.add(egui::Label::new(RichText::new(mouse).size(11.5).color(pal.weak)).wrap());
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(tr!("Shortcuts wait while a text field is typed in, but those with ⌘ (Ctrl) that typing has no use for, as Save.")).size(11.5).color(pal.weak)).wrap());
    ui.data_mut(|d| d.insert_temp(id(), state));
}

/// A line over the columns: a sign, a text, its buttons on the right.
fn notice(ui: &mut Ui, icon: Icon, ink: Color32, text: &str, buttons: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    egui::Frame::new().fill(pal.well()).stroke(Stroke::new(1.0, pal.border)).corner_radius(theme::GROUP_RADIUS).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(icon).to_string()).color(ink));
            ui.label(RichText::new(text).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
        });
    });
    ui.add_space(10.0);
}

/// An action: its name, a reset when changed, its keys as pills, and a
/// place for one more.
fn action_row(ui: &mut Ui, first: bool, action: Action, saved: &mut Saved, map: &keymap::Keymap, state: &mut State) {
    let pal = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    if !first {
        ui.painter().hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
    }
    let keys = map.keys(action).to_vec();
    let changed = saved.changed.contains_key(&action.id());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 4.0;
    // From the right: room for one more key, the keys, the reset.
    let recording = state.recording;
    let listening = |slot| recording == Some((action, slot));
    if keys.len() < 2 {
        let slot = keys.len();
        let text = if listening(slot) { tr!("Press keys…").to_owned() } else { theme::glyph(Icon::Plus).to_string() };
        let r = pill(&mut child, &text, listening(slot), true);
        if r.clicked() {
            start(state, action, slot);
        }
        if !listening(slot) {
            r.on_hover_text(tr!("Add a shortcut"));
        }
    }
    for (slot, chord) in keys.iter().enumerate().rev() {
        let text = if listening(slot) { tr!("Press keys…").to_owned() } else { chord.label() };
        let r = pill(&mut child, &text, listening(slot), false);
        if r.clicked() {
            start(state, action, slot);
        }
        r.context_menu(|ui| {
            if ui.button(theme::with_icon(Icon::X, tr!("Remove"))).clicked() {
                let mut keys = keys.clone();
                keys.remove(slot);
                saved.set(action, &keys);
                ui.close();
            }
        });
    }
    if changed {
        let r = child.add(egui::Button::new(RichText::new(theme::glyph(Icon::RotateCcw).to_string()).size(12.0).color(pal.weak)).frame(false));
        if r.on_hover_text(tr!("Back to {preset}'s", preset = saved.preset.name())).clicked() {
            saved.changed.remove(&action.id());
        }
    }
    // The name takes what is left, cut short with an ellipsis.
    let left = Rect::from_min_max(rect.min, egui::pos2(child.min_rect().left() - 6.0, rect.max.y));
    let mut name_ui = ui.new_child(egui::UiBuilder::new().max_rect(left).layout(egui::Layout::left_to_right(egui::Align::Center)));
    name_ui.add(egui::Label::new(RichText::new(action.name()).size(12.5).color(if keys.is_empty() { pal.weak } else { pal.text })).truncate());
}

/// A key as a pill; `listening` in the accent, `add` faint.
fn pill(ui: &mut Ui, text: &str, listening: bool, add: bool) -> egui::Response {
    let pal = Palette::of(ui);
    let font = FontId::proportional(11.5);
    let ink = if listening { pal.accent } else if add { pal.weak } else { pal.text };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, ink);
    let size = vec2(galley.size().x + 14.0, 20.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, listening, text));
    let fill = if listening {
        pal.accent_soft
    } else if response.hovered() {
        theme::mix(pal.well(), pal.text, 0.08)
    } else if add {
        Color32::TRANSPARENT
    } else {
        pal.panel
    };
    let stroke = if listening { pal.accent } else { pal.border };
    ui.painter().rect(rect, theme::PILL, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    // A key pressed while recording must not press the pill again.
    if response.has_focus() {
        response.surrender_focus();
    }
    response
}

fn start(state: &mut State, action: Action, slot: usize) {
    state.recording = Some((action, slot));
    state.mods = Mods::NONE;
    state.clash = None;
    state.note = None;
    crate::sounds::play(crate::sounds::Cue::Select);
}

/// Listen for the keys of the action being recorded: a key with the
/// modifiers held, or modifiers alone let go (for what is held). A click
/// anywhere stops listening.
fn record(ui: &mut Ui, saved: &mut Saved, map: &keymap::Keymap, state: &mut State) {
    let Some((action, slot)) = state.recording else { return };
    keymap::set_recording();
    ui.ctx().request_repaint();
    let (pressed, mods, clicked) = ui.input(|i| {
        let key = i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some(Chord::with(Mods::of(*modifiers), *key)),
            _ => None,
        });
        (key, Mods::of(i.modifiers), i.pointer.any_pressed())
    });
    let mut chord = pressed;
    if chord.is_none() {
        if mods.is_none() && !state.mods.is_none() {
            // Modifiers alone, let go.
            chord = Some(Chord::mods(state.mods));
        } else {
            state.mods = Mods { command: state.mods.command || mods.command, ctrl: state.mods.ctrl || mods.ctrl, alt: state.mods.alt || mods.alt, shift: state.mods.shift || mods.shift };
        }
    }
    let Some(chord) = chord else {
        if clicked {
            state.recording = None;
        }
        return;
    };
    state.recording = None;
    if !action.accepts(&chord) {
        if chord.key.is_none() && action.kind() == Kind::Press {
            // Modifiers let go before a key: keep listening.
            state.recording = Some((action, slot));
            state.mods = Mods::NONE;
        } else {
            state.note = Some(tr!("{action} takes modifiers alone, as Shift or Alt.", action = action.name()));
        }
        return;
    }
    let others = map.clashes(action, &chord);
    if others.is_empty() {
        put(saved, action, slot, chord);
        state.note = None;
    } else {
        state.clash = Some(Clash { action, slot, chord, others });
    }
}

/// `chord` as `action`'s key at `slot` (one past its keys: one more).
fn put(saved: &mut Saved, action: Action, slot: usize, chord: Chord) {
    let mut keys = saved.keymap().keys(action).to_vec();
    if keys.contains(&chord) {
        return;
    }
    if slot < keys.len() {
        keys[slot] = chord;
    } else {
        keys.push(chord);
    }
    saved.set(action, &keys);
}

fn export(saved: &Saved, state: &mut State) {
    let Some(path) = rfd::FileDialog::new().set_file_name("Massif keys.json").add_filter(tr!("Massif keymap"), &["json"]).save_file() else {
        return;
    };
    state.note = Some(match std::fs::write(&path, saved.export()) {
        Ok(()) => tr!("Saved in {path}.", path = path.display()),
        Err(e) => tr!("Could not save it: {e}.", e),
    });
}

fn import(saved: &mut Saved, state: &mut State) {
    let Some(path) = rfd::FileDialog::new().add_filter(tr!("Massif keymap"), &["json"]).pick_file() else {
        return;
    };
    state.note = Some(match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Saved::import(&t)) {
        Ok(new) => {
            *saved = new;
            tr!("The keys of {file} are in use.", file = crate::jobs::display_name(&path))
        }
        Err(e) => e,
    });
}
