//! The Layers panel: what is painted over the ground, in one list, as
//! WorldPainter's layers are — the map's ground covers, its object layers
//! and the terrain mixes saved for noise painting. Click one to paint it
//! (its settings are in Tool Settings), double-click to rename it, the eye
//! to show or hide it on the map, its menu to copy or remove it.

use crate::blocks::BlockLibrary;
use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Tool, ToolState};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

/// The kinds of layer the panel lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A ground cover, by number.
    Cover,
    /// An object layer, by number.
    Objects,
    /// A terrain mix saved for noise painting, by index.
    Mix,
}

/// What the panel keeps between frames: the layer being renamed.
#[derive(Debug, Default)]
pub struct LayersUi {
    pub rename: Option<(Kind, usize, String)>,
}

/// What a row was asked to do.
enum Action {
    Paint(Kind, usize),
    ToggleShown(Kind, usize),
    Rename(Kind, usize, String),
    Renamed(Kind, usize, String),
    Duplicate(Kind, usize),
    Remove(Kind, usize),
}

/// One layer as the list shows it.
struct Entry {
    kind: Kind,
    id: usize,
    name: String,
    color: [u8; 3],
    detail: String,
    tip: String,
    /// `None` for a layer that is not drawn on the map.
    shown: Option<bool>,
    selected: bool,
}

const ROW_H: f32 = 26.0;

/// The Layers panel. Returns what to say (objects read, or why not).
pub fn layers_panel(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, state: &mut LayersUi) -> Option<String> {
    let before = (doc.world.registry.covers.clone(), doc.world.registry.object_layers.clone());
    let mut said = None;
    let mut actions = Vec::new();

    // Ground covers.
    let covers: Vec<Entry> = doc
        .world
        .registry
        .covers
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.removed)
        .map(|(k, c)| Entry {
            kind: Kind::Cover,
            id: k + 1,
            name: c.name.clone(),
            color: c.color,
            detail: format!("{:.0}%", c.density * 100.0),
            tip: crate::panels::describe_cover(c),
            shown: Some(!c.hidden),
            selected: tools.tool == Tool::Cover && tools.cover as usize == k + 1,
        })
        .collect();
    if header(ui, Icon::Sprout, "Ground covers", "New cover", "A cover of your own: any blocks, any mix") {
        tools.cover = crate::panels::new_cover(&mut doc.world.registry);
        tools.select(Tool::Cover);
    }
    list(ui, &covers, state, &mut actions);

    // Object layers.
    ui.add_space(8.0);
    let objects: Vec<Entry> = doc
        .world
        .registry
        .object_layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.removed)
        .map(|(k, l)| {
            let n = l.objects.len();
            let names: Vec<&str> = l.objects.iter().filter_map(|o| doc.world.registry.objects.get(o.0 as usize)).map(|o| o.name.as_str()).take(6).collect();
            Entry {
                kind: Kind::Objects,
                id: k + 1,
                name: l.name.clone(),
                color: l.color,
                detail: format!("{n} object{}", if n == 1 { "" } else { "s" }),
                tip: format!("{} · one every {} blocks, {:.0}% of the spots", if names.is_empty() { "No objects yet".to_owned() } else { names.join(", ") }, l.spacing, l.density * 100.0),
                shown: Some(!l.hidden),
                selected: tools.tool == Tool::Objects && tools.object_layer as usize == k + 1,
            }
        })
        .collect();
    if header(ui, Icon::TreePine, "Object layers", "Import objects…", "Trees, rocks, ruins from .schem, .schematic, .bp, .litematic or .nbt files") {
        let files = rfd::FileDialog::new().set_title("Import objects").add_filter("Schematics", &massif_objects::EXTENSIONS).pick_files().unwrap_or_default();
        if !files.is_empty() {
            said = Some(crate::panels::import_objects(doc, tools, &files, None, blocks.catalogue.as_deref()));
        }
    }
    if objects.is_empty() {
        hint(ui, "None yet: import objects, or drop their files on the window.");
    }
    list(ui, &objects, state, &mut actions);

    // Terrain mixes.
    ui.add_space(8.0);
    let current = (tools.tool == Tool::PaintTerrain && tools.pattern.on).then(|| tools.pattern.preset_name()).flatten();
    let mixes: Vec<Entry> = tools
        .pattern
        .saved
        .iter()
        .enumerate()
        .map(|(k, p)| {
            let color = p.terrains.first().and_then(|(t, _)| doc.world.registry.terrain_by_name(t)).map_or([128; 3], |t| doc.world.registry.terrain(t).color);
            let parts: Vec<String> = p.terrains.iter().map(|(t, w)| format!("{t} {:.0}%", w * 100.0)).collect();
            Entry {
                kind: Kind::Mix,
                id: k,
                name: p.name.clone(),
                color,
                detail: format!("{} terrains", p.terrains.len()),
                tip: parts.join(", "),
                shown: None,
                selected: current.as_deref() == Some(p.name.as_str()),
            }
        })
        .collect();
    if header(ui, Icon::Blend, "Terrain mixes", "New mix", "Terrains mixed by a noise: set them up in Tool Settings, then save them") {
        tools.pattern.on = true;
        tools.select(Tool::PaintTerrain);
    }
    if mixes.is_empty() {
        hint(ui, "Mixes saved from Paint terrain ▸ Noise show here.");
    }
    list(ui, &mixes, state, &mut actions);

    ui.add_space(8.0);
    hint(ui, "Click one to paint it: its settings are in Tool Settings.");

    for action in actions {
        apply(action, tools, doc, state);
    }
    if (doc.world.registry.covers.clone(), doc.world.registry.object_layers.clone()) != before {
        doc.mark_changed();
    }
    said
}

fn hint(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(text).size(11.5).color(pal.weak));
}

/// A section's title, with its button to add one on the right. Returns
/// whether the button was clicked.
fn header(ui: &mut Ui, icon: Icon, title: &str, add: &str, tip: &str) -> bool {
    let pal = Palette::of(ui);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase())).size(10.5).strong().color(pal.weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = ui.add(egui::Button::new(RichText::new(theme::with_icon(Icon::Plus, add)).size(11.5)).frame(false)).on_hover_text(tip).clicked();
        });
    });
    clicked
}

fn list(ui: &mut Ui, entries: &[Entry], state: &mut LayersUi, actions: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing.y = 2.0;
    for e in entries {
        row(ui, e, state, actions);
    }
    ui.spacing_mut().item_spacing.y = 4.0;
}

/// A layer's row: its eye, its colour, its name and a detail; selected
/// when it is the one painted.
fn row(ui: &mut Ui, e: &Entry, state: &mut LayersUi, actions: &mut Vec<Action>) {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let response = response.on_hover_text(&e.tip);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, e.selected, &e.name));
    let fill = if e.selected {
        pal.accent_soft
    } else if response.hovered() {
        pal.header
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().clone();
    painter.rect(rect, theme::TILE_RADIUS, fill, if e.selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);

    // The eye, for a layer the map draws.
    let eye = Rect::from_center_size(rect.left_center() + vec2(13.0, -2.0), vec2(20.0, 20.0));
    let shown = e.shown.unwrap_or(true);
    if let Some(on) = e.shown {
        let r = ui.put(eye, egui::Button::new(RichText::new(theme::glyph(if on { Icon::Eye } else { Icon::EyeOff }).to_string()).color(if on { pal.text } else { pal.weak })).frame(false));
        if r.on_hover_text(if on { "Hide it on the map" } else { "Show it on the map" }).clicked() {
            actions.push(Action::ToggleShown(e.kind, e.id));
        }
    }
    let dot = Rect::from_center_size(rect.left_center() + vec2(34.0, 0.0), vec2(11.0, 11.0));
    painter.rect(dot, 3, theme::rgb(e.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
    let detail = painter.layout_no_wrap(e.detail.clone(), egui::FontId::proportional(11.0), pal.weak);
    let detail_w = detail.size().x;
    painter.galley(rect.right_center() - vec2(8.0 + detail_w, detail.size().y / 2.0), detail, pal.weak);
    let name_rect = Rect::from_min_max(rect.left_top() + vec2(46.0, 3.0), rect.right_bottom() - vec2(16.0 + detail_w, 3.0));

    match &mut state.rename {
        Some((kind, id, text)) if *kind == e.kind && *id == e.id => {
            let r = ui.put(name_rect, egui::TextEdit::singleline(text).desired_width(name_rect.width()));
            if r.lost_focus() {
                let name = text.trim().to_owned();
                if ui.input(|i| !i.key_pressed(egui::Key::Escape)) && !name.is_empty() {
                    actions.push(Action::Renamed(e.kind, e.id, name));
                } else {
                    state.rename = None;
                }
            } else {
                r.request_focus();
            }
        }
        _ => {
            let galley = painter.layout(e.name.clone(), egui::FontId::proportional(12.5), if shown { pal.text } else { pal.weak }, name_rect.width());
            let galley = if galley.rows.len() > 1 {
                // Too long: cut it.
                let mut job = egui::text::LayoutJob::simple_singleline(e.name.clone(), egui::FontId::proportional(12.5), if shown { pal.text } else { pal.weak });
                job.wrap = egui::text::TextWrapping::truncate_at_width(name_rect.width());
                painter.fonts_mut(|f| f.layout_job(job))
            } else {
                galley
            };
            painter.galley(egui::pos2(name_rect.left(), rect.center().y - galley.size().y / 2.0), galley, pal.text);
        }
    }

    if response.double_clicked() {
        actions.push(Action::Rename(e.kind, e.id, e.name.clone()));
    } else if response.clicked() {
        actions.push(Action::Paint(e.kind, e.id));
    }
    response.context_menu(|ui| {
        if ui.button(theme::with_icon(Icon::Paintbrush, "Paint with it")).clicked() {
            actions.push(Action::Paint(e.kind, e.id));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::Pencil, "Rename")).clicked() {
            actions.push(Action::Rename(e.kind, e.id, e.name.clone()));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::Copy, "Duplicate")).clicked() {
            actions.push(Action::Duplicate(e.kind, e.id));
            ui.close();
        }
        if let Some(on) = e.shown
            && ui.button(theme::with_icon(if on { Icon::EyeOff } else { Icon::Eye }, if on { "Hide on the map" } else { "Show on the map" })).clicked()
        {
            actions.push(Action::ToggleShown(e.kind, e.id));
            ui.close();
        }
        ui.separator();
        let pal = Palette::of(ui);
        let what = match e.kind {
            Kind::Cover => "Nothing grows where it was painted any more",
            Kind::Objects => "Nothing stands where it was painted any more",
            Kind::Mix => "Forget this mix (the ground painted with it stays)",
        };
        if ui.button(RichText::new(theme::with_icon(Icon::Trash2, "Remove")).color(pal.danger)).on_hover_text(what).clicked() {
            actions.push(Action::Remove(e.kind, e.id));
            ui.close();
        }
    });
}

/// A name not yet taken by one of `taken`: `name`, or `name 2`, `name 3`…
pub fn unique_name<'a>(name: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    let free = |n: &str| !taken.clone().any(|t| t.eq_ignore_ascii_case(n));
    if free(name) {
        return name.to_owned();
    }
    (2..).map(|k| format!("{name} {k}")).find(|n| free(n)).expect("a free name")
}

fn apply(action: Action, tools: &mut ToolState, doc: &mut Document, state: &mut LayersUi) {
    let reg = &mut doc.world.registry;
    match action {
        Action::Paint(Kind::Cover, n) => {
            tools.cover = n as u16;
            tools.select(Tool::Cover);
        }
        Action::Paint(Kind::Objects, n) => {
            tools.object_layer = n as u16;
            tools.select(Tool::Objects);
        }
        Action::Paint(Kind::Mix, k) => {
            if let Some(p) = tools.pattern.saved.get(k).cloned() {
                tools.pattern.load(&p);
                tools.pattern.on = true;
                tools.select(Tool::PaintTerrain);
            }
        }
        Action::ToggleShown(Kind::Cover, n) => {
            if let Some(c) = reg.covers.get_mut(n - 1) {
                c.hidden = !c.hidden;
            }
        }
        Action::ToggleShown(Kind::Objects, n) => {
            if let Some(l) = reg.object_layers.get_mut(n - 1) {
                l.hidden = !l.hidden;
            }
        }
        Action::ToggleShown(Kind::Mix, _) => {}
        Action::Rename(kind, id, name) => state.rename = Some((kind, id, name)),
        Action::Renamed(kind, id, name) => {
            state.rename = None;
            match kind {
                Kind::Cover => {
                    if let Some(c) = reg.covers.get_mut(id - 1) {
                        c.name = name;
                    }
                }
                Kind::Objects => {
                    if let Some(l) = reg.object_layers.get_mut(id - 1) {
                        l.name = name;
                    }
                }
                Kind::Mix => {
                    let others: Vec<String> = tools.pattern.saved.iter().enumerate().filter(|(k, _)| *k != id).map(|(_, p)| p.name.clone()).collect();
                    let name = unique_name(&name, others.iter().map(String::as_str));
                    if let Some(p) = tools.pattern.saved.get_mut(id) {
                        p.name = name;
                    }
                }
            }
        }
        Action::Duplicate(Kind::Cover, n) => {
            if let Some(mut c) = reg.covers.get(n - 1).cloned() {
                c.name = unique_name(&format!("{} copy", c.name), reg.covers.iter().filter(|c| !c.removed).map(|c| c.name.as_str()));
                c.hidden = false;
                tools.cover = reg.add_cover(c);
                tools.select(Tool::Cover);
            }
        }
        Action::Duplicate(Kind::Objects, n) => {
            if let Some(mut l) = reg.object_layers.get(n - 1).cloned() {
                l.name = unique_name(&format!("{} copy", l.name), reg.object_layers.iter().filter(|l| !l.removed).map(|l| l.name.as_str()));
                l.hidden = false;
                tools.object_layer = reg.add_object_layer(l);
                tools.select(Tool::Objects);
            }
        }
        Action::Duplicate(Kind::Mix, k) => {
            if let Some(mut p) = tools.pattern.saved.get(k).cloned() {
                p.name = unique_name(&format!("{} copy", p.name), tools.pattern.saved.iter().map(|p| p.name.as_str()));
                tools.pattern.saved.push(p);
            }
        }
        Action::Remove(Kind::Cover, n) => {
            if let Some(c) = reg.covers.get_mut(n - 1) {
                c.removed = true;
            }
            if tools.cover as usize == n {
                tools.cover = reg.covers.iter().position(|c| !c.removed).map_or(0, |k| k as u16 + 1);
            }
        }
        Action::Remove(Kind::Objects, n) => {
            if let Some(l) = reg.object_layers.get_mut(n - 1) {
                l.removed = true;
            }
            if tools.object_layer as usize == n {
                tools.object_layer = reg.object_layers.iter().position(|l| !l.removed).map_or(0, |k| k as u16 + 1);
            }
        }
        Action::Remove(Kind::Mix, k) => {
            if k < tools.pattern.saved.len() {
                tools.pattern.saved.remove(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use massif_core::generate::{GenParams, generate};

    struct State {
        tools: ToolState,
        doc: Document,
        blocks: BlockLibrary,
        ui: LayersUi,
    }

    fn state() -> State {
        let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let mut doc = Document::new(world, None, 1);
        let reg = &mut doc.world.registry;
        reg.objects.push(massif_core::objects::ObjectDef::new("Oak", [1, 2, 1], vec!["minecraft:air".into(), "minecraft:oak_log".into()], vec![1, 1]));
        let layer = massif_core::objects::ObjectLayerDef::new("Oaks", &[0], reg);
        reg.add_object_layer(layer);
        let mut tools = ToolState::default();
        tools.pattern.save("Gravel paths");
        State { tools, doc, blocks: BlockLibrary::default(), ui: LayersUi::default() }
    }

    fn draw(ui: &mut Ui, s: &mut State) {
        theme::install(ui.ctx());
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            layers_panel(ui, &mut s.tools, &mut s.doc, &mut s.blocks, &mut s.ui);
        });
    }

    /// Every cover, object layer and saved mix is listed; a click paints
    /// with it, the eye hides it from the map (and the file keeps it), a
    /// rename, a copy and a removal do what they say.
    #[test]
    fn layers_are_listed_picked_and_edited() {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(320.0, 900.0)).build_ui_state(draw, state());
        h.run_steps(2);
        for name in ["Grass", "Flowers", "Seagrass", "Oaks", "Gravel paths"] {
            h.get_by_label(name);
        }
        h.get_by_label("Flowers").click();
        h.run_steps(2);
        let flowers = h.state().doc.world.registry.cover_by_name("Flowers").unwrap();
        assert_eq!((h.state().tools.tool, h.state().tools.cover), (Tool::Cover, flowers));
        h.get_by_label("Oaks").click();
        h.run_steps(2);
        assert_eq!((h.state().tools.tool, h.state().tools.object_layer), (Tool::Objects, 1));
        h.get_by_label("Gravel paths").click();
        h.run_steps(2);
        assert_eq!(h.state().tools.tool, Tool::PaintTerrain);
        assert!(h.state().tools.pattern.on);

        // Hidden: left off the map, written in the file, nothing else.
        let s = h.state_mut();
        apply(Action::ToggleShown(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui);
        assert!(s.doc.world.registry.covers[flowers as usize - 1].hidden);
        let json = serde_json::to_string(&s.doc.world.registry.covers).unwrap();
        assert_eq!(json.matches("\"hidden\"").count(), 1, "only a hidden cover says so");

        let s = h.state_mut();
        apply(Action::Renamed(Kind::Objects, 1, "Big oaks".into()), &mut s.tools, &mut s.doc, &mut s.ui);
        apply(Action::Duplicate(Kind::Objects, 1), &mut s.tools, &mut s.doc, &mut s.ui);
        let names: Vec<&str> = s.doc.world.registry.object_layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Big oaks", "Big oaks copy"]);
        assert_eq!(s.tools.object_layer, 2);
        apply(Action::Remove(Kind::Objects, 2), &mut s.tools, &mut s.doc, &mut s.ui);
        assert!(s.doc.world.registry.object_layer(2).is_none());
        assert_eq!(s.tools.object_layer, 1);
        apply(Action::Duplicate(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui);
        assert!(s.doc.world.registry.cover_by_name("Flowers copy").is_some());
        assert!(!s.doc.world.registry.covers.last().unwrap().hidden, "a copy is shown");
        apply(Action::Remove(Kind::Mix, 0), &mut s.tools, &mut s.doc, &mut s.ui);
        assert!(s.tools.pattern.saved.is_empty());
        h.run_steps(2);
        h.get_by_label("Big oaks");
    }

    #[test]
    fn names_are_kept_apart() {
        let taken = ["Oaks", "Oaks 2"];
        assert_eq!(unique_name("Pines", taken.iter().copied()), "Pines");
        assert_eq!(unique_name("oaks", taken.iter().copied()), "oaks 3");
    }

    /// The panel drawn offscreen in both themes, a cover hidden and one
    /// picked. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn layers_panel_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
        for dark in [true, false] {
            let mut s = state();
            s.doc.world.registry.covers[6].hidden = true;
            s.tools.cover = 3;
            s.tools.select(Tool::Cover);
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(320.0, 560.0))
                .with_pixels_per_point(ppp)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut State| {
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        draw(ui, s);
                    },
                    s,
                );
            h.run_steps(4);
            h.render().unwrap().save(dir.join(if dark { "layers-dark.png" } else { "layers-light.png" })).unwrap();
        }
    }
}
