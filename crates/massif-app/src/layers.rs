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
    /// A layer of underground pockets, by number.
    Pockets,
    /// A combined layer, by number.
    Combined,
}

/// What the panel keeps between frames: the layer being renamed.
#[derive(Debug, Default)]
pub struct LayersUi {
    pub rename: Option<(Kind, usize, String)>,
    /// The Packs shown rather than the map's layers.
    pub packs: bool,
    /// What the Packs are searched for.
    pub search: String,
    /// The packs imported.
    pub library: crate::pack_share::PackLibrary,
    /// A layer whose settings were asked for (Settings… in its menu): the
    /// window opens its dialog.
    pub open: Option<(Kind, usize)>,
}

/// What a row was asked to do.
enum Action {
    Paint(Kind, usize),
    Settings(Kind, usize),
    ToggleShown(Kind, usize),
    ToggleLock(Kind, usize),
    /// Up (`true`) or down the list of its section.
    Move(Kind, usize, bool),
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
    /// `None` for one that cannot be locked.
    locked: Option<bool>,
    selected: bool,
}

const ROW_H: f32 = 26.0;

/// The Layers panel, every layer of the map by the slot of a block it
/// paints (see `docs/layers.md`): terrain mixes, ground covers (plants,
/// Frost, Lava), tree layers, sub-canopy layers, then the tree sizes and the
/// annotations. Returns what to say (objects read, or why not).
pub fn layers_panel(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, state: &mut LayersUi) -> Option<String> {
    if let Some(k) = crate::widgets::button_group(ui, "layers or packs", &[tr!("This map"), tr!("Packs")], &[!state.packs, state.packs]) {
        state.packs = k == 1;
    }
    ui.add_space(6.0);
    if state.packs {
        return crate::packs::packs_panel(ui, tools, doc, blocks.catalogue.as_deref(), &mut state.search, &mut state.library);
    }
    let before = (doc.world.registry.covers.clone(), doc.world.registry.object_layers.clone(), doc.world.registry.pockets.clone(), doc.world.registry.combined.clone());
    let mut said = None;
    let mut actions = Vec::new();

    // Combined layers: several of the others at once.
    let reg = &doc.world.registry;
    let mut combined: Vec<(u32, Entry)> = reg
        .combined
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.removed)
        .map(|(k, c)| {
            (
                c.order,
                Entry {
                    kind: Kind::Combined,
                    id: k + 1,
                    name: c.name.clone(),
                    color: c.color,
                    detail: trn!("{n} layer", "{n} layers", n = c.parts.len()),
                    tip: crate::panels::describe_combined(reg, c),
                    shown: None,
                    locked: None,
                    selected: tools.tool == Tool::Combined && tools.combined as usize == k + 1,
                },
            )
        })
        .collect();
    combined.sort_by_key(|(order, e)| (*order, e.id));
    let combined: Vec<Entry> = combined.into_iter().map(|c| c.1).collect();
    if header(ui, Icon::Combine, tr!("Combined layers"), tr!("New combined"), tr!("Several layers painted at once: a terrain, a biome, a cover, trees, pockets")) {
        tools.combined = crate::panels::new_combined(&mut doc.world.registry);
        tools.select(Tool::Combined);
    }
    if combined.is_empty() {
        hint(ui, tr!("A forest's terrain, biome, ground cover and trees in one stroke, as WorldPainter's combined layers."));
    }
    list(ui, &combined, state, &mut actions);
    ui.add_space(8.0);

    // Terrain mixes: the ground.
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
                detail: tr!("{n} terrains", n = p.terrains.len()),
                tip: parts.join(", "),
                shown: None,
                locked: None,
                selected: current.as_deref() == Some(p.name.as_str()),
            }
        })
        .collect();
    if header(ui, Icon::Blend, tr!("Terrain mixes"), tr!("New mix"), tr!("Terrains mixed by a noise: set them up in Tool Settings, then save them")) {
        tools.pattern.on = true;
        tools.select(Tool::PaintTerrain);
    }
    if mixes.is_empty() {
        hint(ui, tr!("Mixes saved from Paint terrain ▸ Noise show here."));
    }
    list(ui, &mixes, state, &mut actions);

    // Ground covers: one a block.
    ui.add_space(8.0);
    let mut covers: Vec<(u32, Entry)> = doc
        .world
        .registry
        .covers
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.removed)
        .map(|(k, c)| {
            (
                c.order,
                Entry {
                    kind: Kind::Cover,
                    id: k + 1,
                    name: c.name.clone(),
                    color: c.color,
                    detail: format!("{:.0}%", c.density * 100.0),
                    tip: crate::panels::describe_cover(c),
                    shown: Some(!c.hidden),
                    locked: Some(c.locked),
                    selected: tools.tool == Tool::Cover && tools.cover as usize == k + 1,
                },
            )
        })
        .collect();
    covers.sort_by_key(|(order, e)| (*order, e.id));
    let covers: Vec<Entry> = covers.into_iter().map(|c| c.1).collect();
    if header(ui, Icon::Sprout, tr!("Ground covers"), tr!("New cover"), tr!("A cover of your own: any blocks, any mix")) {
        tools.cover = crate::panels::new_cover(&mut doc.world.registry);
        tools.select(Tool::Cover);
    }
    list(ui, &covers, state, &mut actions);

    // Tree and object layers, one a block, then the sub-canopy ones.
    let reg = &doc.world.registry;
    let mut trees: Vec<(u32, bool, Entry)> = reg
        .object_layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.removed)
        .map(|(k, l)| {
            let n = l.objects.len();
            let names: Vec<&str> = l.objects.iter().filter_map(|o| reg.objects.get(o.0 as usize)).map(|o| o.name.as_str()).take(6).collect();
            (
                l.order,
                l.understory,
                Entry {
                    kind: Kind::Objects,
                    id: k + 1,
                    name: l.name.clone(),
                    color: l.color,
                    detail: trn!("{n} object", "{n} objects", n = n),
                    tip: tr!("{objects} · one every {spacing} blocks, {share:.0}% of the spots", objects = if names.is_empty() { tr!("No objects yet").to_owned() } else { names.join(", ") }, spacing = l.spacing, share = l.density * 100.0),
                    shown: Some(!l.hidden),
                    locked: Some(l.locked),
                    selected: tools.tool == Tool::Objects && tools.object_layer as usize == k + 1,
                },
            )
        })
        .collect();
    trees.sort_by_key(|(order, _, e)| (*order, e.id));
    let (under, base): (Vec<_>, Vec<_>) = trees.into_iter().partition(|t| t.1);
    let (base, under): (Vec<Entry>, Vec<Entry>) = (base.into_iter().map(|t| t.2).collect(), under.into_iter().map(|t| t.2).collect());
    ui.add_space(8.0);
    if header(ui, Icon::TreePine, tr!("Trees and objects"), tr!("Import objects…"), tr!("Trees, rocks, ruins from .schem, .schematic, .bp, .litematic or .nbt files")) {
        let files = rfd::FileDialog::new().set_title(tr!("Import objects")).add_filter(tr!("Schematics"), &massif_objects::EXTENSIONS).pick_files().unwrap_or_default();
        if !files.is_empty() {
            said = Some(crate::panels::import_objects(doc, tools, &files, None, blocks.catalogue.as_deref()));
        }
    }
    if base.is_empty() {
        hint(ui, tr!("None yet: import objects, or drop their files on the window."));
    }
    list(ui, &base, state, &mut actions);
    ui.add_space(8.0);
    title(ui, Icon::Shrub, tr!("Under the trees"));
    if under.is_empty() {
        hint(ui, tr!("Bushes, hazel, holly that grow under the trees: Bushes from Packs, or any object layer made Sub-canopy in its settings."));
    }
    list(ui, &under, state, &mut actions);

    // Underground pockets, one layer a block.
    ui.add_space(8.0);
    let mut pockets: Vec<(u32, Entry)> = doc
        .world
        .registry
        .pockets
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.removed)
        .map(|(k, p)| {
            (
                p.order,
                Entry {
                    kind: Kind::Pockets,
                    id: k + 1,
                    name: p.name.clone(),
                    color: p.color,
                    detail: format!("{:.0}%", p.share * 100.0),
                    tip: tr!("{blocks} · from {low} to {high}", blocks = p.blocks.iter().map(|b| crate::panels::plant_name(&doc.world.registry.block(b.0).state)).collect::<Vec<_>>().join(", "), low = p.low, high = p.high),
                    shown: None,
                    locked: Some(p.locked),
                    selected: tools.tool == Tool::Pockets && tools.pocket_layer as usize == k + 1,
                },
            )
        })
        .collect();
    pockets.sort_by_key(|(order, e)| (*order, e.id));
    let pockets: Vec<Entry> = pockets.into_iter().map(|p| p.1).collect();
    if header(ui, Icon::Gem, tr!("Underground pockets"), tr!("New pockets"), tr!("Pockets of your own: any blocks, in the rock under the ground")) {
        let dim = &doc.world.dims[doc.dim];
        tools.pocket_layer = crate::panels::new_pockets(&mut doc.world.registry, dim);
        tools.select(Tool::Pockets);
    }
    if pockets.is_empty() {
        hint(ui, tr!("Gravel, clay, a mod's stone in the rock under the ground, as WorldPainter's underground pockets."));
    }
    list(ui, &pockets, state, &mut actions);

    // The tree sizes the object layers take where they are painted.
    ui.add_space(8.0);
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  {}", theme::glyph(Icon::ChartNoAxesColumnIncreasing), tr!("Tree sizes").to_uppercase())).size(10.5).strong().color(pal.weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let eye = if tools.show_sizes { Icon::Eye } else { Icon::EyeOff };
            if ui.add(egui::Button::new(RichText::new(theme::glyph(eye).to_string()).size(12.0)).frame(false)).on_hover_text(tr!("Show the tree sizes over the map with every tool")).clicked() {
                tools.show_sizes = !tools.show_sizes;
            }
        });
    });
    if let Some(n) = crate::panels::size_swatches(ui, 20.0, (tools.tool == Tool::TreeSize).then_some(tools.tree_size), false) {
        tools.tree_size = n;
        tools.select(Tool::TreeSize);
    }
    hint(ui, tr!("Paint sizes, then a species over them: each spot takes its objects of the size marked."));

    // Annotations: a click on a colour paints with it.
    ui.add_space(8.0);
    title(ui, Icon::PenLine, tr!("Annotations"));
    if let Some(n) = crate::panels::annotation_swatches(ui, 20.0, (tools.tool == Tool::Annotate).then_some(tools.annotation)) {
        tools.annotation = n;
        tools.select(Tool::Annotate);
    }
    hint(ui, tr!("Marks on the map, to plan it, as in WorldPainter: never exported."));

    ui.add_space(8.0);
    hint(ui, tr!("Click a layer to paint it, double-click to rename it; its menu has its settings, its place in the list, its lock. A block holds one layer of each section. Ready-made trees, plants and mixes: Packs, above."));

    let lists = [(Kind::Combined, false, combined), (Kind::Mix, false, mixes), (Kind::Cover, false, covers), (Kind::Objects, false, base), (Kind::Objects, true, under), (Kind::Pockets, false, pockets)];
    for action in actions {
        apply(action, tools, doc, state, &lists);
    }
    if (doc.world.registry.covers.clone(), doc.world.registry.object_layers.clone(), doc.world.registry.pockets.clone(), doc.world.registry.combined.clone()) != before {
        doc.mark_changed();
    }
    said
}

/// A section's title, without a button.
fn title(ui: &mut Ui, icon: Icon, title: &str) {
    let pal = Palette::of(ui);
    ui.add(egui::Label::new(RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase())).size(10.5).strong().color(pal.weak)).truncate());
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
        // The button first, from the right; the title takes what is left,
        // cut short with … in a narrow panel or a long language.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked = ui.add(egui::Button::new(RichText::new(theme::with_icon(Icon::Plus, add)).size(11.5)).frame(false)).on_hover_text(tip).clicked();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase())).size(10.5).strong().color(pal.weak)).truncate());
            });
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
        if r.on_hover_text(if on { tr!("Hide it on the map") } else { tr!("Show it on the map") }).clicked() {
            actions.push(Action::ToggleShown(e.kind, e.id));
        }
    }
    let dot = Rect::from_center_size(rect.left_center() + vec2(34.0, 0.0), vec2(11.0, 11.0));
    painter.rect(dot, 3, theme::rgb(e.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
    // The lock, on the right: shown when locked, and to lock it when the
    // row is pointed at.
    let mut lock_w = 0.0;
    if let Some(locked) = e.locked
        && (locked || response.hovered() || ui.rect_contains_pointer(rect))
    {
        lock_w = 20.0;
        let at = Rect::from_center_size(rect.right_center() - vec2(13.0, 2.0), vec2(20.0, 20.0));
        let r = ui.put(at, egui::Button::new(RichText::new(theme::glyph(if locked { Icon::Lock } else { Icon::LockOpen }).to_string()).color(if locked { pal.text } else { pal.weak })).frame(false));
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, locked, tr!("Lock")));
        if r.on_hover_text(if locked { tr!("Locked: brushes leave its blocks as they are. Click to unlock") } else { tr!("Lock it: brushes then leave its blocks as they are") }).clicked() {
            actions.push(Action::ToggleLock(e.kind, e.id));
        }
    }
    let detail = painter.layout_no_wrap(e.detail.clone(), egui::FontId::proportional(11.0), pal.weak);
    let detail_w = detail.size().x + lock_w;
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
        if e.kind != Kind::Mix && ui.button(theme::with_icon(Icon::SlidersHorizontal, tr!("Settings…"))).clicked() {
            actions.push(Action::Settings(e.kind, e.id));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::Paintbrush, tr!("Paint with it"))).clicked() {
            actions.push(Action::Paint(e.kind, e.id));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::Pencil, tr!("Rename"))).clicked() {
            actions.push(Action::Rename(e.kind, e.id, e.name.clone()));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::Copy, tr!("Duplicate"))).clicked() {
            actions.push(Action::Duplicate(e.kind, e.id));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::ArrowUp, tr!("Move up"))).clicked() {
            actions.push(Action::Move(e.kind, e.id, true));
            ui.close();
        }
        if ui.button(theme::with_icon(Icon::ArrowDown, tr!("Move down"))).clicked() {
            actions.push(Action::Move(e.kind, e.id, false));
            ui.close();
        }
        if let Some(on) = e.shown
            && ui.button(theme::with_icon(if on { Icon::EyeOff } else { Icon::Eye }, if on { tr!("Hide on the map") } else { tr!("Show on the map") })).clicked()
        {
            actions.push(Action::ToggleShown(e.kind, e.id));
            ui.close();
        }
        if let Some(locked) = e.locked
            && ui.button(theme::with_icon(if locked { Icon::LockOpen } else { Icon::Lock }, if locked { tr!("Unlock") } else { tr!("Lock") })).clicked()
        {
            actions.push(Action::ToggleLock(e.kind, e.id));
            ui.close();
        }
        ui.separator();
        let pal = Palette::of(ui);
        let what = match e.kind {
            Kind::Cover => tr!("Nothing grows where it was painted any more"),
            Kind::Objects => tr!("Nothing stands where it was painted any more"),
            Kind::Mix => tr!("Forget this mix (the ground painted with it stays)"),
            Kind::Pockets => tr!("No pocket where it was painted any more"),
            Kind::Combined => tr!("What it painted stays"),
        };
        if ui.button(RichText::new(theme::with_icon(Icon::Trash2, tr!("Remove"))).color(pal.danger)).on_hover_text(what).clicked() {
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

fn apply(action: Action, tools: &mut ToolState, doc: &mut Document, state: &mut LayersUi, lists: &[(Kind, bool, Vec<Entry>)]) {
    let reg = &mut doc.world.registry;
    match action {
        Action::Settings(kind, n) => {
            apply(Action::Paint(kind, n), tools, doc, state, lists);
            state.open = Some((kind, n));
        }
        Action::ToggleLock(Kind::Cover, n) => {
            if let Some(c) = reg.covers.get_mut(n - 1) {
                c.locked = !c.locked;
            }
        }
        Action::ToggleLock(Kind::Objects, n) => {
            if let Some(l) = reg.object_layers.get_mut(n - 1) {
                l.locked = !l.locked;
            }
        }
        Action::ToggleLock(Kind::Pockets, n) => {
            if let Some(p) = reg.pockets.get_mut(n - 1) {
                p.locked = !p.locked;
            }
        }
        Action::ToggleLock(Kind::Mix | Kind::Combined, _) => {}
        Action::Move(kind, id, up) => {
            // Its section's list, in the order shown; it changes place with
            // its neighbour.
            let Some(ids) = lists.iter().find(|l| l.0 == kind && l.2.iter().any(|e| e.id == id)).map(|l| l.2.iter().map(|e| e.id).collect::<Vec<usize>>()) else { return };
            let Some(k) = ids.iter().position(|&i| i == id) else { return };
            let other = if up { k.checked_sub(1) } else { Some(k + 1).filter(|&o| o < ids.len()) };
            let Some(other) = other else { return };
            let mut ids = ids;
            ids.swap(k, other);
            match kind {
                Kind::Mix => tools.pattern.saved.swap(k, other),
                Kind::Pockets => {
                    for (place, &n) in ids.iter().enumerate() {
                        if let Some(p) = reg.pockets.get_mut(n - 1) {
                            p.order = place as u32 + 1;
                        }
                    }
                }
                Kind::Combined => {
                    for (place, &n) in ids.iter().enumerate() {
                        if let Some(c) = reg.combined.get_mut(n - 1) {
                            c.order = place as u32 + 1;
                        }
                    }
                }
                Kind::Cover => {
                    for (place, &n) in ids.iter().enumerate() {
                        if let Some(c) = reg.covers.get_mut(n - 1) {
                            c.order = place as u32 + 1;
                        }
                    }
                }
                Kind::Objects => {
                    for (place, &n) in ids.iter().enumerate() {
                        if let Some(l) = reg.object_layers.get_mut(n - 1) {
                            l.order = place as u32 + 1;
                        }
                    }
                }
            }
        }
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
        Action::ToggleShown(Kind::Mix | Kind::Pockets | Kind::Combined, _) => {}
        Action::Paint(Kind::Combined, n) => {
            tools.combined = n as u16;
            tools.select(Tool::Combined);
        }
        Action::Duplicate(Kind::Combined, n) => {
            if let Some(mut c) = reg.combined.get(n - 1).cloned() {
                c.name = unique_name(&tr!("{name} copy", name = c.name), reg.combined.iter().filter(|c| !c.removed).map(|c| c.name.as_str()));
                tools.combined = reg.add_combined(c);
                tools.select(Tool::Combined);
            }
        }
        Action::Remove(Kind::Combined, n) => {
            if let Some(c) = reg.combined.get_mut(n - 1) {
                c.removed = true;
            }
            if tools.combined as usize == n {
                tools.combined = reg.combined.iter().position(|c| !c.removed).map_or(0, |k| k as u16 + 1);
            }
        }
        Action::Paint(Kind::Pockets, n) => {
            tools.pocket_layer = n as u16;
            tools.select(Tool::Pockets);
        }
        Action::Duplicate(Kind::Pockets, n) => {
            if let Some(mut p) = reg.pockets.get(n - 1).cloned() {
                p.name = unique_name(&tr!("{name} copy", name = p.name), reg.pockets.iter().filter(|p| !p.removed).map(|p| p.name.as_str()));
                p.locked = false;
                tools.pocket_layer = reg.add_pocket(p);
                tools.select(Tool::Pockets);
            }
        }
        Action::Remove(Kind::Pockets, n) => {
            if let Some(p) = reg.pockets.get_mut(n - 1) {
                p.removed = true;
            }
            if tools.pocket_layer as usize == n {
                tools.pocket_layer = reg.pockets.iter().position(|p| !p.removed).map_or(0, |k| k as u16 + 1);
            }
        }
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
                Kind::Pockets => {
                    if let Some(p) = reg.pockets.get_mut(id - 1) {
                        p.name = name;
                    }
                }
                Kind::Combined => {
                    if let Some(c) = reg.combined.get_mut(id - 1) {
                        c.name = name;
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
                c.name = unique_name(&tr!("{name} copy", name = c.name), reg.covers.iter().filter(|c| !c.removed).map(|c| c.name.as_str()));
                c.hidden = false;
                tools.cover = reg.add_cover(c);
                tools.select(Tool::Cover);
            }
        }
        Action::Duplicate(Kind::Objects, n) => {
            if let Some(mut l) = reg.object_layers.get(n - 1).cloned() {
                l.name = unique_name(&tr!("{name} copy", name = l.name), reg.object_layers.iter().filter(|l| !l.removed).map(|l| l.name.as_str()));
                l.hidden = false;
                tools.object_layer = reg.add_object_layer(l);
                tools.select(Tool::Objects);
            }
        }
        Action::Duplicate(Kind::Mix, k) => {
            if let Some(mut p) = tools.pattern.saved.get(k).cloned() {
                p.name = unique_name(&tr!("{name} copy", name = p.name), tools.pattern.saved.iter().map(|p| p.name.as_str()));
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
        // WorldPainter's Frost and Lava are covers of every new map; a
        // colour of the annotations paints with it.
        h.get_by_label("Frost");
        h.get_by_label("Lava");
        h.get_by_label("Light blue").click();
        h.run_steps(2);
        assert_eq!((h.state().tools.tool, h.state().tools.annotation), (Tool::Annotate, 4));
        // A tree size paints with it.
        h.get_by_label("Large trees").click();
        h.run_steps(2);
        assert_eq!((h.state().tools.tool, h.state().tools.tree_size), (Tool::TreeSize, 4));
        // A layer locked, moved up its section, and its settings asked for.
        let lists = |s: &State| -> Vec<(Kind, bool, Vec<Entry>)> {
            let reg = &s.doc.world.registry;
            let mut covers: Vec<Entry> = reg.covers.iter().enumerate().filter(|(_, c)| !c.removed).map(|(k, c)| entry(Kind::Cover, k + 1, &c.name)).collect();
            covers.sort_by_key(|e| (reg.covers[e.id - 1].order, e.id));
            vec![(Kind::Cover, false, covers)]
        };
        let s = h.state_mut();
        apply(Action::ToggleLock(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        assert!(s.doc.world.registry.covers[flowers as usize - 1].locked);
        let l = lists(s);
        apply(Action::Move(Kind::Cover, flowers as usize, true), &mut s.tools, &mut s.doc, &mut s.ui, &l);
        let order: Vec<String> = lists(s)[0].2.iter().map(|e| e.name.clone()).take(3).collect();
        assert_eq!(order, ["Grass", "Flowers", "Tall grass"]);
        let l = lists(s);
        apply(Action::Settings(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui, &l);
        assert_eq!(s.ui.open, Some((Kind::Cover, flowers as usize)));

        // Hidden: left off the map, written in the file, nothing else.
        let s = h.state_mut();
        apply(Action::ToggleShown(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        assert!(s.doc.world.registry.covers[flowers as usize - 1].hidden);
        let json = serde_json::to_string(&s.doc.world.registry.covers).unwrap();
        assert_eq!(json.matches("\"hidden\"").count(), 1, "only a hidden cover says so");

        let s = h.state_mut();
        apply(Action::Renamed(Kind::Objects, 1, "Big oaks".into()), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        apply(Action::Duplicate(Kind::Objects, 1), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        let names: Vec<&str> = s.doc.world.registry.object_layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Big oaks", "Big oaks copy"]);
        assert_eq!(s.tools.object_layer, 2);
        apply(Action::Remove(Kind::Objects, 2), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        assert!(s.doc.world.registry.object_layer(2).is_none());
        assert_eq!(s.tools.object_layer, 1);
        apply(Action::Duplicate(Kind::Cover, flowers as usize), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        assert!(s.doc.world.registry.cover_by_name("Flowers copy").is_some());
        assert!(!s.doc.world.registry.covers.last().unwrap().hidden, "a copy is shown");
        apply(Action::Remove(Kind::Mix, 0), &mut s.tools, &mut s.doc, &mut s.ui, &[]);
        assert!(s.tools.pattern.saved.is_empty());
        h.run_steps(2);
        h.get_by_label("Big oaks");
    }

    fn entry(kind: Kind, id: usize, name: &str) -> Entry {
        Entry { kind, id, name: name.to_owned(), color: [0; 3], detail: String::new(), tip: String::new(), shown: None, locked: None, selected: false }
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
            // A locked tree layer, and a sub-canopy one under it.
            s.doc.world.registry.object_layers[0].locked = true;
            let mut hazel = s.doc.world.registry.object_layers[0].clone();
            (hazel.name, hazel.understory, hazel.locked, hazel.color) = ("Hazel".into(), true, false, [126, 150, 70]);
            s.doc.world.registry.add_object_layer(hazel);
            // Two pocket layers, one locked.
            let reg = &mut s.doc.world.registry;
            let (gravel, andesite, clay) = (reg.intern_block("minecraft:gravel", [136, 126, 118]), reg.intern_block("minecraft:andesite", [136, 136, 136]), reg.intern_block("minecraft:clay", [160, 166, 179]));
            reg.add_pocket(massif_core::pockets::PocketDef::new("Gravel and andesite", [136, 126, 118], &[(gravel, 3), (andesite, 1)], 0.2, 5.0, -40, 50));
            let mut clay = massif_core::pockets::PocketDef::new("Clay lenses", [160, 166, 179], &[(clay, 1)], 0.08, 7.0, 30, 58);
            clay.locked = true;
            s.doc.world.registry.add_pocket(clay);
            // A combined layer.
            let mut forest = massif_core::combined::CombinedDef::new("Oak forest", [74, 116, 52]);
            forest.set(massif_core::combined::Part::Cover, Some((2, 0.6)));
            forest.set(massif_core::combined::Part::Trees, Some((1, 1.0)));
            s.doc.world.registry.add_combined(forest);
            s.tools.cover = 3;
            s.tools.select(Tool::Cover);
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(320.0, 1100.0))
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
